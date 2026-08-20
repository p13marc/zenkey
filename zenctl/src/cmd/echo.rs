//! `topic echo` v2 — subscribe, refine, schema-decode (RFC 08 §7) with
//! honest structural fallback.

use anyhow::Result;

use super::sample::{
    attachment_display, attachment_json, format_sample, hex, qos_summary, source_summary, type_tag,
};
use crate::{BusArgs, output};

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
        // Borrowed, not cloned: `sample` is an `Arc<SampleView>` that
        // outlives every use below (`docs/zero-copy.md`). `to_bytes()` is a
        // `Cow` and is already the right form — it is not a copy.
        let key = sample.key.as_str();
        let bytes = sample.payload.to_bytes();
        let encoding = sample.encoding.as_str();
        let timestamp = sample.timestamp.map(|t| t.to_string());
        // The attachment is a wire fact, rendered once (structural → hex,
        // size-tagged, never schema-decoded) and shown wherever the sample is.
        let att = sample.attachment.as_ref().map(attachment_display);
        // The wire's actual QoS axes (#120) — always stamped, defaults
        // included; the source only when the publisher attaches it.
        let qos = qos_summary(
            sample.priority,
            sample.congestion_control,
            sample.reliability,
            sample.express,
        );
        let source = sample.source.as_ref().map(source_summary);
        let rate_suffix = if rate {
            let (_, _, hz) = monitor.core().with_stats(|s| s.totals());
            format!("  @ {hz:.1}/s")
        } else {
            String::new()
        };

        if sample.kind == zenoh::sample::SampleKind::Delete {
            // A tombstone is not an empty put (#115): render the retirement,
            // decode nothing — there is nothing to decode.
            if ndjson {
                // No `value` and no `payload_bytes`: a tombstone has no
                // payload, and "0 bytes" would read as an empty put, which
                // is the one thing RFC 04 §1.2 says it is not.
                let mut row = zenkey_fleet::SampleRow::of_key(key, &base).with_wire(&sample);
                row.qos_axes = Some(qos.clone());
                if let Some(a) = &sample.attachment {
                    row.attachment = Some(attachment_json(a));
                    row.attachment_bytes = Some(a.len());
                }
                println!("{}", row.to_line());
            } else {
                println!(
                    "{key}\n  <tombstone — authoritative retirement (RFC 04 §1.2), \
                     not an empty value>{rate_suffix}"
                );
                if let Some(a) = &att {
                    println!("  attachment: {a}");
                }
            }
        } else if raw {
            println!("{key}\n  {}{rate_suffix}", hex(&bytes));
            if let Some(a) = &sample.attachment {
                println!("  attachment: {}", hex(&a.to_bytes()));
            }
        } else if hex_payload {
            // --hex: the decode pipeline still names the type, the payload
            // shows as bytes.
            let type_name = zenkey_fleet::decode::decode_sample(
                &store,
                &session,
                &slices,
                &base,
                key,
                Some(encoding),
                &bytes,
            )
            .await
            .type_name;
            // The tag names the registered type; the payload is shown as
            // bytes at the user's request, which is not a failed decode —
            // so no `?`.
            let tag = type_tag(type_name.as_deref(), true);
            println!("{key}\n  {tag} {}{rate_suffix}", hex(&bytes));
            if let Some(a) = &sample.attachment {
                println!("  attachment: {}", hex(&a.to_bytes()));
            }
        } else {
            // `--no-decode` never asks, so it has no verdict to misreport.
            let (type_name, rendering, verdict, decode_error) = if no_decode {
                (
                    None,
                    zenkey_fleet::decode::Rendering::Structural(zenkey_fleet::decode::structural(
                        &bytes,
                    )),
                    None,
                    None,
                )
            } else {
                let d = zenkey_fleet::decode::decode_sample(
                    &store,
                    &session,
                    &slices,
                    &base,
                    key,
                    Some(encoding),
                    &bytes,
                )
                .await;
                (d.type_name, d.rendering, Some(d.verdict), d.decode_error)
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
                let mut row = zenkey_fleet::SampleRow::of_key(key, &base).with_wire(&sample);
                // The wire axes are a fact worth carrying, but not under
                // `qos`: that key is resolved as an RFC 04 §3 profile *name*
                // by the reader on the other end of this pipe (#235).
                row.qos_axes = Some(qos.clone());
                // `--no-decode` never asks, so it has nothing to report —
                // absent, not null-when-unknown (RFC 09 §5.1 O4). The same
                // rule the verdict below has always followed.
                row.type_name = type_name.clone();
                row.typed = Some(typed);
                row.payload_bytes = Some(bytes.len());
                row.value = Some(
                    serde_json::from_str::<serde_json::Value>(&value)
                        .unwrap_or(serde_json::Value::String(value.clone())),
                );
                // Present only when the publisher attached SourceInfo —
                // absent, never null-when-unknown (#120).
                row.source = sample.source.as_ref().map(source_summary);
                // Present only when the wire carried one — absent, never
                // null-when-unknown (#117).
                if let Some(a) = &sample.attachment {
                    row.attachment = Some(attachment_json(a));
                    row.attachment_bytes = Some(a.len());
                }
                // #159: present only when the pipeline was asked (--no-decode
                // never asks) — and then always, so "valid" and "not checked"
                // cannot be confused by their shared absence.
                if let Some(v) = &verdict {
                    row.verdict = Some(match v {
                        zenkey_fleet::Verdict::Valid => "valid".to_string(),
                        zenkey_fleet::Verdict::Invalid(errors) => {
                            row.violations = Some(errors.clone());
                            "invalid".to_string()
                        }
                        zenkey_fleet::Verdict::NotValidated(r) => format!("not-validated: {r}"),
                    });
                    row.decode_error = decode_error.clone();
                }
                println!("{}", row.to_line());
            } else if let Some(fmt) = fmt {
                println!(
                    "{}",
                    format_sample(
                        fmt,
                        seen,
                        key,
                        &base,
                        type_name.as_deref(),
                        encoding,
                        bytes.len(),
                        timestamp.as_deref(),
                        &value,
                        att.as_deref(),
                        Some(&qos),
                        source.as_deref(),
                    )
                );
            } else {
                let tag = type_tag(type_name.as_deref(), typed);
                println!("{key}\n  {tag} {value}{rate_suffix}");
                if let Some(a) = &att {
                    println!("  attachment: {a}");
                }
                for note in notes {
                    eprintln!("  note: {note}");
                }
                // #159: a checked failure says so; Valid and NotValidated
                // stay quiet here (the tag already carries typed/structural).
                if let Some(zenkey_fleet::Verdict::Invalid(errors)) = &verdict {
                    for e in errors {
                        eprintln!("  invalid: {e}");
                    }
                }
                if let Some(e) = &decode_error {
                    eprintln!("  undecodable: {e}");
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
