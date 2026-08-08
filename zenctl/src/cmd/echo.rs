//! `topic echo` v2 — subscribe, refine, schema-decode (RFC 08 §7) with
//! honest structural fallback.

use anyhow::Result;

use crate::{BusArgs, output};

/// One decoded/rendered sample line for echo.
#[allow(clippy::too_many_arguments)]
fn format_sample(
    fmt: &str,
    n: usize,
    wire_key: &str,
    base: &str,
    type_name: Option<&str>,
    encoding: &str,
    payload_len: usize,
    timestamp: Option<&str>,
    value: &str,
) -> String {
    let parsed = zenkey::grammar::parse_full(base, wire_key);
    let mut out = String::with_capacity(fmt.len() + value.len());
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
            continue;
        }
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('{') => {
                // %{a.b.c}: a decoded payload field by dot-path. Unresolvable
                // paths render as an empty field, honestly (the line shape
                // stays stable for cut/awk).
                let mut path = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    path.push(c);
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(value) {
                    let mut cur = &v;
                    let mut ok = true;
                    for seg in path.split('.') {
                        match cur.get(seg) {
                            Some(next) => cur = next,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        match cur {
                            serde_json::Value::String(s) => out.push_str(s),
                            other => out.push_str(&other.to_string()),
                        }
                    }
                }
            }
            Some('k') => out.push_str(wire_key),
            Some('K') => {
                out.push_str(zenkey::grammar::strip_base(base, wire_key).unwrap_or(wire_key))
            }
            Some('o') => {
                if let Some(p) = &parsed {
                    out.push_str(p.origin.chunk());
                }
            }
            Some('c') => {
                if let Some(p) = &parsed {
                    out.push_str(match &p.class {
                        zenkey::grammar::ClassOrPlane::Class(c) => c.chunk(),
                        zenkey::grammar::ClassOrPlane::Plane(pl) => pl.chunk(),
                    });
                }
            }
            Some('p') => {
                if let Some(name) = parsed.as_ref().and_then(|p| p.producer.as_ref()) {
                    out.push_str(name.name());
                }
            }
            Some('s') => {
                if let Some(p) = &parsed {
                    out.push_str(&p.subject.join("/"));
                }
            }
            Some('t') => out.push_str(type_name.unwrap_or("unregistered")),
            Some('v') => out.push_str(value),
            Some('e') => out.push_str(encoding),
            Some('l') => {
                use std::fmt::Write as _;
                let _ = write!(out, "{payload_len}");
            }
            Some('n') => {
                use std::fmt::Write as _;
                let _ = write!(out, "{n}");
            }
            Some('T') => out.push_str(timestamp.unwrap_or("-")),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `topic echo` — subscribe-first is not a style choice: RFC 04 §3.2 forbids
/// GET-then-subscribe (it drops everything published in the gap).
#[allow(clippy::too_many_arguments)]
pub async fn run(
    selector: Option<&str>,
    origin: Option<&str>,
    class: Option<&str>,
    producer: Option<&str>,
    fmt: Option<&str>,
    raw: bool,
    hex_payload: bool,
    rate: bool,
    no_decode: bool,
    count: usize,
    seed: bool,
    args: &BusArgs,
) -> Result<()> {
    let selector = match selector {
        Some(s) => s.to_string(),
        None => super::compose_selector(args, origin, class, producer)?,
    };
    let base = args.base().to_string();

    // Slices first (a single introspect fan-in), then subscribe: the slice
    // set names each subject's payload type; the schema store fetches
    // `describe` lazily on first decode miss.
    let slices = if raw {
        zenkey_fleet::SliceSet::default()
    } else {
        args.slice_set().await?
    };
    let store = zenkey_fleet::decode::SchemaStore::new(&base, args.timeout());

    let session = args.session().await?;
    // Through the Monitor (issue #48): the same bounded broadcast the GUI
    // uses, so a bus that outruns this terminal surfaces as an explicit
    // dropped count instead of invisible loss (RFC 09 §5.1 O6). This is also
    // the §6.3 promise kept: the CLI validates the engine's path.
    // --seed makes the watch a seeded one (issue #92): the monitor declares
    // the subscriber first, then pulls both seed paths through one LWW merge;
    // the boundary event separates seeded state from live traffic.
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    if seed {
        monitor
            .watch_seeded(
                &selector,
                zenkey_fleet::SeedPolicy {
                    timeout: args.timeout(),
                    ..Default::default()
                },
            )
            .await?;
    } else {
        monitor.watch(&selector).await?;
    }

    let ndjson = matches!(args.format.resolved(), output::Format::Ndjson);
    if !ndjson {
        eprintln!(
            "echoing {selector}{} (ctrl-c to stop)",
            if seed {
                " (seeding current state…)"
            } else {
                ""
            }
        );
    }
    let mut seen = 0usize;
    let mut dropped_total = 0u64;
    while let Some(item) = events.recv().await {
        let sample = match item {
            zenkey_fleet::StreamItem::Dropped(n) => {
                dropped_total += n;
                if ndjson {
                    println!("{}", serde_json::json!({ "dropped": n }));
                } else {
                    eprintln!("-- dropped {n} sample(s): the bus outran us --");
                }
                continue;
            }
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::WatchSeeded {
                coverage,
                ..
            }) => {
                // The boundary, rendered per O4: which paths ran and what
                // each yielded — zeros are observations, not verdicts.
                if ndjson {
                    println!("{}", serde_json::json!({ "seed_complete": coverage }));
                } else {
                    let path = |n: Option<usize>, what: &str| match n {
                        Some(n) => format!("{n} {what}"),
                        None => format!("{what} path off"),
                    };
                    eprintln!(
                        "-- seed complete: {} · {} · {} superseded — live from here --",
                        path(coverage.history_replies, "from caches"),
                        path(coverage.storage_replies, "from storage"),
                        coverage.superseded,
                    );
                }
                continue;
            }
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) => s,
            zenkey_fleet::StreamItem::Event(_) => continue,
        };
        seen += 1;
        let key = sample.key.clone();
        let bytes = sample.payload.to_bytes();
        let encoding = sample.encoding.clone();
        let timestamp = sample.timestamp.map(|t| t.to_string());
        let rate_suffix = if rate {
            let (_, _, hz) = monitor.core().with_stats(|s| s.totals());
            format!("  @ {hz:.1}/s")
        } else {
            String::new()
        };

        if raw {
            println!("{key}\n  {}{rate_suffix}", hex(&bytes));
        } else if hex_payload {
            // --hex: the decode pipeline still names the type, the payload
            // shows as bytes.
            let (type_name, _) = zenkey_fleet::decode::decode_sample(
                &store,
                &session,
                &slices,
                &base,
                &key,
                Some(&encoding),
                &bytes,
            )
            .await;
            let tag = type_name
                .map(|t| format!("<{t}>"))
                .unwrap_or_else(|| "<unregistered>".to_string());
            println!("{key}\n  {tag} {}{rate_suffix}", hex(&bytes));
        } else {
            let (type_name, rendering) = if no_decode {
                (
                    None,
                    zenkey_fleet::decode::Rendering::Structural(zenkey_fleet::decode::structural(
                        &bytes,
                    )),
                )
            } else {
                zenkey_fleet::decode::decode_sample(
                    &store,
                    &session,
                    &slices,
                    &base,
                    &key,
                    Some(&encoding),
                    &bytes,
                )
                .await
            };
            let (value, typed, notes) = match &rendering {
                zenkey_fleet::decode::Rendering::Typed(d) => (
                    serde_json::to_string(&d.value).unwrap_or_default(),
                    true,
                    d.notes.clone(),
                ),
                zenkey_fleet::decode::Rendering::Structural(text) => {
                    (text.clone(), false, Vec::new())
                }
            };
            if ndjson {
                let parsed = zenkey::grammar::parse_full(&base, &key);
                let obj = serde_json::json!({
                    "key": key,
                    "origin": parsed.as_ref().map(|p| p.origin.chunk().to_string()),
                    "subject": parsed.as_ref().map(|p| p.subject.join("/")),
                    "type": type_name,
                    "typed": typed,
                    "encoding": encoding,
                    "timestamp": timestamp,
                    "value": serde_json::from_str::<serde_json::Value>(&value)
                        .unwrap_or(serde_json::Value::String(value.clone())),
                });
                println!("{obj}");
            } else if let Some(fmt) = fmt {
                println!(
                    "{}",
                    format_sample(
                        fmt,
                        seen,
                        &key,
                        &base,
                        type_name.as_deref(),
                        &encoding,
                        bytes.len(),
                        timestamp.as_deref(),
                        &value,
                    )
                );
            } else {
                let tag = match (&type_name, typed) {
                    (Some(t), true) => format!("<{t}>"),
                    (Some(t), false) => format!("<{t}?>"),
                    (None, _) => "<unregistered>".to_string(),
                };
                println!("{key}\n  {tag} {value}{rate_suffix}");
                for note in notes {
                    eprintln!("  note: {note}");
                }
            }
        }

        if count > 0 && seen >= count {
            break;
        }
    }
    if !ndjson {
        eprintln!(
            "{seen} sample(s) shown, {dropped_total} dropped{}",
            if dropped_total > 0 {
                " (the terminal could not keep up; the counts are the honest record)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_sample_extracts_payload_fields() {
        let line = format_sample(
            "%{iface.name} up=%{iface.up} missing=[%{no.such}]",
            1,
            "k",
            "",
            None,
            "application/json",
            2,
            None,
            r#"{"iface":{"name":"eth0","up":true}}"#,
        );
        assert_eq!(line, "eth0 up=true missing=[]");
    }

    #[test]
    fn format_sample_expands_fields() {
        let line = format_sample(
            "%n %o %c/%p %s <%t> %v (%l B, %e)",
            3,
            "tcgui/v1/h-3fa9c2d41b7e/state/tc/iface/eth0/state",
            "tcgui",
            Some("NetworkInterface"),
            "application/json",
            12,
            None,
            r#"{"up":true}"#,
        );
        assert_eq!(
            line,
            r#"3 h-3fa9c2d41b7e state/tc iface/eth0/state <NetworkInterface> {"up":true} (12 B, application/json)"#
        );
        // Escapes and literals.
        assert_eq!(
            format_sample(
                "%%|%K\\t.",
                1,
                "b/v1/h-3fa9c2d41b7e/state/tc/x",
                "b",
                None,
                "e",
                0,
                None,
                "v"
            ),
            "%|v1/h-3fa9c2d41b7e/state/tc/x\t."
        );
    }
}
