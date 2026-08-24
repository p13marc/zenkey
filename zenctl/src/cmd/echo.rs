//! `zenctl echo` — subscribe, refine, schema-decode (RFC 08 §7) with honest
//! structural fallback.
//!
//! Top-level since #307: subscribing to live traffic is not something the
//! registry declares, so it is not a verb of the `topic` noun. The rename is
//! the whole change — the stream, the row shape and the `%`-vocabulary are
//! untouched, which is what keeps `zenctl echo --format ndjson | zenctl pub
//! --from ndjson` composing.

use anyhow::Result;

use super::sample::{
    self, attachment_display, attachment_json, format_sample, hex, qos_summary, source_summary,
    type_tag,
};
use crate::Bus;
use crate::cli::EchoArgs;

/// `zenctl echo` — subscribe-first is not a style choice: RFC 04 §3.2 forbids
/// GET-then-subscribe (it drops everything published in the gap).
pub async fn run(cli: EchoArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let EchoArgs {
        selector: sel,
        fmt,
        raw,
        hex: hex_payload,
        rate,
        no_decode,
        count,
        seed,
        bus: _,
    } = &cli;
    let (fmt, raw, hex_payload, rate, no_decode, count, seed) = (
        fmt.as_deref(),
        *raw,
        *hex_payload,
        *rate,
        *no_decode,
        *count,
        *seed,
    );
    let selector = super::selector_of(sel, args)?;
    let base = args.base().to_string();

    // Slices first (a single introspect fan-in), then subscribe: the slice
    // set names each subject's payload type; the schema store fetches
    // `describe` lazily on first decode miss.
    // Slices enrich: they name each key's payload type, and without them the
    // decode ladder falls to its structural rung — which is exactly what
    // `--raw` asks for on purpose. A registry that will not answer must not
    // cost the user the stream itself (#210). `None` stays `None` into the
    // ladder, so a row's verdict reads `no registry loaded` rather than
    // claiming `no schema served` about types nobody looked up
    // (RFC 09 §5.1 O4; #246).
    let slices = if raw {
        None
    } else {
        args.slices_optional().await?
    };
    let store = zenkey_fleet::model::decode::SchemaStore::new(&base, args.timeout());

    let session = args.session().await?;
    let fleet = args.fleet(&session);
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

    // One resolution for the whole run, and it happens in `Mode::of` (#198).
    // A streaming verb's question is only ever "is a program reading this" —
    // it has rows for one and prose for the other, and no third answer.
    let ndjson = crate::render::Mode::of(args.format()).machine();
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
                    // Tagged (`"row":"dropped"`) so the pipe's other end —
                    // `pub --from ndjson`, via `parse_stream_line` —
                    // skips it as stream metadata instead of counting a
                    // malformed row. A bare `{"dropped":n}` poisoned the
                    // round trip the row dialect exists for (#235).
                    println!(
                        "{}",
                        crate::render::Row::tagged("dropped", serde_json::json!({ "dropped": n }))
                            .into_line()
                    );
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
                    // Tagged for the same reason as the dropped line above.
                    println!(
                        "{}",
                        crate::render::Row::tagged(
                            "seed",
                            serde_json::json!({ "seed_complete": coverage })
                        )
                        .into_line()
                    );
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
            let type_name = zenkey_fleet::model::decode::decode_sample(
                &fleet,
                &store,
                slices.as_ref(),
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
            // `--no-decode` never asks, so it has no verdict to misreport —
            // `sample::decode` carries that rule now, for both verbs.
            let d = sample::decode(
                &fleet,
                &store,
                slices.as_ref(),
                key,
                Some(encoding),
                &bytes,
                no_decode,
            )
            .await;
            let type_name = d.type_name;
            let v = sample::value_of(&d.rendering);
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
                row.typed = Some(v.typed);
                row.payload_bytes = Some(bytes.len());
                row.value = Some(
                    serde_json::from_str::<serde_json::Value>(&v.text)
                        .unwrap_or(serde_json::Value::String(v.text.clone())),
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
                if let Some(verdict) = &d.verdict {
                    row.verdict = Some(match verdict {
                        zenkey_fleet::Verdict::Valid => "valid".to_string(),
                        zenkey_fleet::Verdict::Invalid(errors) => {
                            row.violations = Some(errors.clone());
                            "invalid".to_string()
                        }
                        zenkey_fleet::Verdict::NotValidated(r) => format!("not-validated: {r}"),
                    });
                    row.decode_error = d.decode_error.clone();
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
                        &v.text,
                        att.as_deref(),
                        Some(&qos),
                        source.as_deref(),
                    )
                );
            } else {
                let tag = type_tag(type_name.as_deref(), v.typed);
                println!("{key}\n  {tag} {}{rate_suffix}", v.text);
                if let Some(a) = &att {
                    println!("  attachment: {a}");
                }
                for note in v.notes {
                    eprintln!("  note: {note}");
                }
                // #159: a checked failure says so; Valid and NotValidated
                // stay quiet here (the tag already carries typed/structural).
                if let Some(zenkey_fleet::Verdict::Invalid(errors)) = &d.verdict {
                    for e in errors {
                        eprintln!("  invalid: {e}");
                    }
                }
                if let Some(e) = &d.decode_error {
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

#[cfg(test)]
mod tests {
    /// The pipe round trip, meta lines included (#235's symmetry, kept): the
    /// exact tagged lines this verb prints between its sample rows read back
    /// as stream metadata — skipped, not malformed — and a sample row still
    /// reads back as a row.
    #[test]
    fn echo_meta_lines_read_back_as_meta_and_samples_as_samples() {
        use zenkey_fleet::StreamLine;

        // The two meta lines exactly as the ndjson arm above spells them.
        let dropped =
            crate::render::Row::tagged("dropped", serde_json::json!({ "dropped": 3 })).into_line();
        let seed = crate::render::Row::tagged(
            "seed",
            serde_json::json!({ "seed_complete": { "superseded": 0 } }),
        )
        .into_line();
        // A sample row as the same arm writes one.
        let mut row = zenkey_fleet::SampleRow::of_key("v1/h-3fa9c2d41b7e/state/p/health", "");
        row.value = Some(serde_json::json!({"status": "ok"}));
        let sample = row.to_line();

        let stream = [sample.as_str(), dropped.as_str(), seed.as_str()];
        let parsed: Vec<StreamLine> = stream
            .iter()
            .map(|l| zenkey_fleet::parse_stream_line(l).expect("every echo line parses"))
            .collect();
        assert!(
            matches!(&parsed[0], StreamLine::Sample(r) if r.key == "v1/h-3fa9c2d41b7e/state/p/health"),
            "the sample row is a row"
        );
        assert_eq!(
            &parsed[1..],
            [
                StreamLine::Meta("dropped".into()),
                StreamLine::Meta("seed".into())
            ],
            "meta lines are skipped as metadata, never counted malformed"
        );
    }
}
