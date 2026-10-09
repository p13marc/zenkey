//! `zenctl echo` — every sample on a wire selector, decoded where a
//! contract reaches it (#612, FJ8b; the tooling guide's O1 and O2).
//!
//! RAW: the subscription is on a session in no namespace, so every key on
//! the wire is shown, whoever's it is. A key in the deployment's namespace
//! that parses as zk2 is resolved through a lens ([`zenkey_fleet::Lens`]):
//! the presence read and the descriptors a second session, **in** the
//! namespace, made of it, and the revisions those descriptors name,
//! retrieved from their holders or held by `--contracts`. The payload is
//! decoded as its declared type (spec §7.2), a JSON Schema value validated
//! against it (§7.3), and where the ladder stops the rung is said — on the
//! row, in every format. A foreign key renders structurally, never refused.
//!
//! The lens is a [`zenkey_fleet::LensFeed`]: a key that falls short of a
//! provider or a revision nudges a re-read in the background, so a service
//! that starts after `echo` resolves too, and the drain never waits on one.
//!
//! The row dialect is the one `pub --from ndjson` and `.zrec` read back
//! (#235, FJ8a's `qos_axes`), with the key's `identity` beside it; the
//! tagged `dropped` lines are stream metadata, skipped by the reader.

use anyhow::Result;
use zenkey_fleet::model::render::Member;
use zenkey_fleet::report::Conformance;
use zenkey_fleet::{Lens, LensFeed};

use super::sample::{Line, Payload, SampleLine, format_sample, hex, qos_summary, source_summary};
use crate::bus::Deployment;
use crate::cli::EchoArgs;
use crate::cmd::zk2;

/// `zenctl echo [SELECTOR]` — subscribe first, and resolve beside it.
pub async fn run(cli: EchoArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let EchoArgs {
        selector,
        fmt,
        raw,
        hex: hex_payload,
        rate,
        no_decode,
        count,
        contracts: _,
        ns: _,
    } = cli;
    let namespace = dep.namespace().to_owned();
    let selector = zk2::wire_selector(selector.as_deref(), &namespace)?;

    // The data plane in no namespace: every key as the wire carries it.
    let session = dep.link().session().await?;
    let store = zk2::store(&dep, &contracts);
    // The deployment in its namespace, for the lens — unless nothing is to
    // be decoded, which asks nothing of it.
    let feed = if raw || no_decode {
        None
    } else {
        let ns_session = dep.session().await?;
        let (feed, error) = LensFeed::open(&ns_session, &store, dep.timeout()).await;
        if let Some(e) = error {
            eprintln!(
                "{}",
                zk2::lens_unread_note(&zenkey_fleet::one_line(&e)).to_line()
            );
        }
        Some(feed)
    };

    // Through the Monitor (#48): the same bounded broadcast every observer
    // uses, so a bus that outruns this terminal surfaces as an explicit
    // dropped count instead of invisible loss (O6).
    let monitor =
        zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default()).await?;
    let mut events = monitor.events();
    monitor.watch(&selector).await?;

    let ndjson = crate::render::Mode::of(dep.format()).machine();
    if !ndjson {
        eprintln!(
            "echoing {selector}{} (ctrl-c to stop)",
            excluded_note(&selector)
        );
    }
    let mut seen = 0usize;
    let mut dropped_total = 0u64;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let item = tokio::select! {
            item = events.recv() => item,
            _ = &mut ctrl_c => break,
        };
        let Some(item) = item else { break };
        let sample = match item {
            zenkey_fleet::StreamItem::Dropped(n) => {
                dropped_total += n;
                if ndjson {
                    // Tagged (`"row":"dropped"`) so `pub --from ndjson`
                    // skips it as stream metadata (#235).
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
            zenkey_fleet::StreamItem::Event(zenkey_fleet::FleetEvent::Sample(s)) => s,
            zenkey_fleet::StreamItem::Event(_) => continue,
        };
        seen += 1;
        let catalog = feed.as_ref().and_then(LensFeed::catalog);
        let lens = Lens::new(&namespace, catalog.as_deref(), &store).offline(&contracts);
        let line = line_of(&sample, &lens, raw || no_decode);
        if let Some(f) = &feed {
            f.nudge(line.identity.unresolved.as_ref());
        }
        let rate_suffix = if rate {
            let (_, _, hz) = monitor.core().with_stats(|s| s.totals());
            format!("  @ {hz:.1}/s")
        } else {
            String::new()
        };
        if ndjson {
            println!("{}", row(&line, &sample).to_line());
        } else if raw {
            println!("{}\n  {}{rate_suffix}", sample.key, hex(&line.bytes));
            if let Some(a) = &sample.attachment {
                println!("  attachment: {}", hex(&a.to_bytes()));
            }
        } else if let Some(fmt) = &fmt {
            println!("{}", formatted(&line, fmt, seen, &sample, &namespace));
        } else {
            print(&line, &sample, hex_payload, &rate_suffix);
        }
        if count > 0 && seen >= count {
            break;
        }
    }
    monitor.stop();
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

/// What a `**` selector cannot carry, said up front (O5).
fn excluded_note(selector: &str) -> String {
    let excluded = zenkey_fleet::zrec_excluded(&[selector.to_owned()]);
    if excluded.is_empty() {
        String::new()
    } else {
        format!(
            " — {} excluded: no selector names them, and `*`/`**` never match one",
            excluded.join(", ")
        )
    }
}

/// One sample through the lens.
fn line_of(sample: &zenkey_fleet::SampleView, lens: &Lens<'_>, structural_only: bool) -> Line {
    let attachment = sample.attachment.as_ref().map(|a| a.to_bytes());
    Line::of(
        Payload {
            key: &sample.key,
            encoding: (!sample.encoding.is_empty()).then_some(sample.encoding.as_str()),
            bytes: sample.payload.to_bytes().into_owned(),
            attachment: attachment.as_deref(),
            delete: sample.kind == zenoh::sample::SampleKind::Delete,
        },
        Member::Type,
        lens,
        structural_only,
    )
}

/// The ndjson row: the dialect `pub --from ndjson` reads back, with the
/// key's identity and the payload's conformance beside it.
fn row(line: &Line, sample: &zenkey_fleet::SampleView) -> zenkey_fleet::SampleRow {
    // `with_wire` sets the axes as they rode, `qos_axes` (#235).
    let mut row = zenkey_fleet::SampleRow::of_key(&sample.key).with_wire(sample);
    row.source = sample.source.as_ref().map(source_summary);
    let attachment = sample.attachment.as_ref().map(|a| a.to_bytes());
    line.fill(&mut row, attachment.as_deref());
    row
}

/// One `--fmt` line.
fn formatted(
    line: &Line,
    fmt: &str,
    n: usize,
    sample: &zenkey_fleet::SampleView,
    namespace: &str,
) -> String {
    let value = line.value_text();
    let qos = qos_summary(
        sample.priority,
        sample.congestion_control,
        sample.reliability,
        sample.express,
    );
    let timestamp = sample.timestamp.map(|t| t.to_string());
    let source = sample.source.as_ref().map(source_summary);
    let attachment = line
        .attachment
        .as_ref()
        .map(crate::render::payload_text)
        .or_else(|| {
            sample
                .attachment
                .as_ref()
                .map(|a| super::sample::attachment_display(&a.to_bytes()))
        });
    let declared = line.declared();
    format_sample(
        fmt,
        &SampleLine {
            n,
            wire_key: &sample.key,
            relative: zenkey_fleet::strip_namespace(namespace, &sample.key),
            identity: &line.identity,
            type_name: declared.as_deref(),
            encoding: &sample.encoding,
            payload_len: line.bytes.len(),
            timestamp: timestamp.as_deref(),
            value: &value,
            attachment: attachment.as_deref(),
            qos: Some(&qos),
            source: source.as_deref(),
        },
    )
}

/// The table form: the wire key, then the rendering with the rung it
/// stopped at, and what failed against the declared type on stderr.
fn print(line: &Line, sample: &zenkey_fleet::SampleView, hex_payload: bool, rate_suffix: &str) {
    let Some((rendering, conformance)) = &line.checked else {
        println!(
            "{}\n  <tombstone — a deletion, not an empty value>{rate_suffix}",
            sample.key
        );
        return;
    };
    if hex_payload {
        println!(
            "{}\n  {} {}{rate_suffix}",
            sample.key,
            line.tag(),
            hex(&line.bytes)
        );
    } else {
        println!(
            "{}\n  {}{rate_suffix}",
            sample.key,
            crate::render::payload_text(rendering)
        );
    }
    if let Some(a) = &line.attachment {
        println!("  attachment: {}", crate::render::payload_text(a));
    }
    if let Conformance::Invalid { violations } = conformance {
        for v in violations {
            eprintln!("  invalid: {v}");
        }
    }
}

#[cfg(test)]
mod tests {
    /// The pipe round trip, meta lines included (#235's symmetry, kept): the
    /// tagged `dropped` line this verb prints between its sample rows reads
    /// back as stream metadata — skipped, not malformed — and a sample row
    /// carrying its zk2 identity still reads back as a row.
    #[test]
    fn echo_meta_lines_read_back_as_meta_and_samples_as_samples() {
        use zenkey_fleet::StreamLine;
        use zenkey_fleet::report::{KeyGroup, KeyIdentity};

        let dropped =
            crate::render::Row::tagged("dropped", serde_json::json!({ "dropped": 3 })).into_line();
        let mut row =
            zenkey_fleet::SampleRow::of_key("prod/zk2/host-a/tc/tc.netif.v1/state/namespaces");
        row.value = Some(serde_json::json!(["default"]));
        row.identity = Some(KeyIdentity {
            group: KeyGroup::Resource {
                address: "host-a/tc".into(),
                iface: "tc.netif.v1".into(),
                token: "state".into(),
                resource: Some("state/namespaces".into()),
            },
            values: Default::default(),
            unresolved: None,
        });
        let sample = row.to_line();

        let parsed: Vec<StreamLine> = [sample.as_str(), dropped.as_str()]
            .iter()
            .map(|l| zenkey_fleet::parse_stream_line(l).expect("every echo line parses"))
            .collect();
        assert!(
            matches!(&parsed[0], StreamLine::Sample(r) if r.key == "prod/zk2/host-a/tc/tc.netif.v1/state/namespaces"),
            "the sample row is a row"
        );
        assert_eq!(parsed[1], StreamLine::Meta("dropped".into()));
    }
}
