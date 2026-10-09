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
use zenkey_fleet::report::{Conformance, KeyIdentity, Rendered};
use zenkey_fleet::{Lens, LensFeed};

use super::sample::{
    SampleLine, Zk2Positions, attachment_json, format_sample, hex, qos_summary, source_summary,
};
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
        let line = Line::of(&sample, &lens, raw || no_decode);
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
            println!("{}", line.row(&sample, &namespace).to_line());
        } else if raw {
            println!("{}\n  {}{rate_suffix}", sample.key, hex(&line.bytes));
            if let Some(a) = &sample.attachment {
                println!("  attachment: {}", hex(&a.to_bytes()));
            }
        } else if let Some(fmt) = &fmt {
            println!("{}", line.formatted(fmt, seen, &sample, &namespace));
        } else {
            line.print(&sample, hex_payload, &rate_suffix);
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

/// One sample through the lens, once, for whichever medium prints it.
struct Line {
    identity: KeyIdentity,
    bytes: Vec<u8>,
    /// `None` on a deletion, which carries nothing to decode (#115).
    checked: Option<(zenkey_fleet::report::PayloadRendering, Conformance)>,
    attachment: Option<zenkey_fleet::report::PayloadRendering>,
}

impl Line {
    fn of(sample: &zenkey_fleet::SampleView, lens: &Lens<'_>, structural_only: bool) -> Line {
        let bytes = sample.payload.to_bytes().into_owned();
        let encoding = (!sample.encoding.is_empty()).then_some(sample.encoding.as_str());
        let delete = sample.kind == zenoh::sample::SampleKind::Delete;
        if structural_only {
            let identity = lens.identity(&sample.key);
            return Line {
                identity,
                checked: (!delete).then(|| {
                    (
                        zenkey_fleet::report::PayloadRendering {
                            key: sample.key.clone(),
                            size: bytes.len(),
                            resource: None,
                            rendered: Rendered::Structural {
                                why: zenkey_fleet::report::Unresolved::DecodeNotAsked,
                                value: zenkey_fleet::structural_value(&bytes),
                                text: zenkey_fleet::structural(&bytes),
                            },
                        },
                        Conformance::NotChecked {
                            reason: zenkey_fleet::report::Unresolved::DecodeNotAsked.words(),
                        },
                    )
                }),
                attachment: None,
                bytes,
            };
        }
        if delete {
            return Line {
                identity: lens.identity(&sample.key),
                checked: None,
                attachment: None,
                bytes,
            };
        }
        let c = lens.check(&sample.key, Member::Type, encoding, &bytes);
        let attachment = sample
            .attachment
            .as_ref()
            .map(|a| lens.render(&sample.key, Member::Attachment, None, &a.to_bytes()));
        Line {
            identity: c.identity,
            checked: Some((c.rendering, c.conformance)),
            attachment,
            bytes,
        }
    }

    /// The declared type, when the ladder reached one.
    fn declared(&self) -> Option<String> {
        match &self.checked.as_ref()?.0.rendered {
            Rendered::Value { declared, .. } | Rendered::Undecodable { declared, .. } => {
                Some(declared.clone())
            }
            Rendered::Opaque { media_type } => Some(media_type.clone()),
            Rendered::Structural { .. } => None,
        }
    }

    /// The payload as a JSON value: decoded, or the structural document,
    /// or its text.
    fn value(&self) -> Option<serde_json::Value> {
        let (r, _) = self.checked.as_ref()?;
        Some(match &r.rendered {
            Rendered::Value { value, .. } => value.clone(),
            Rendered::Structural { value: Some(v), .. } => v.clone(),
            Rendered::Structural { text, .. } => serde_json::Value::String(text.clone()),
            Rendered::Opaque { media_type } => {
                serde_json::Value::String(format!("<{media_type}, {} B>", r.size))
            }
            Rendered::Undecodable { .. } => {
                serde_json::Value::String(zenkey_fleet::structural(&self.bytes))
            }
        })
    }

    /// The ndjson row: the dialect `pub --from ndjson` reads back, with the
    /// key's identity and the payload's conformance beside it.
    fn row(&self, sample: &zenkey_fleet::SampleView, namespace: &str) -> zenkey_fleet::SampleRow {
        let mut row = zenkey_fleet::SampleRow::of_key(&sample.key, namespace).with_wire(sample);
        // The wire axes ride `qos_axes`, never `qos` (a profile name, #235).
        row.qos_axes = Some(qos_summary(
            sample.priority,
            sample.congestion_control,
            sample.reliability,
            sample.express,
        ));
        row.identity = Some(self.identity.clone());
        row.source = sample.source.as_ref().map(source_summary);
        if let Some(a) = &sample.attachment {
            row.attachment = Some(match self.attachment.as_ref().map(|r| &r.rendered) {
                Some(Rendered::Value { value, .. }) => value.clone(),
                _ => attachment_json(a),
            });
            row.attachment_bytes = Some(a.len());
        }
        let Some((rendering, conformance)) = &self.checked else {
            // A tombstone: no value, no byte count — "0 bytes" would read
            // as an empty put, which RFC 04 §1.2 says it is not.
            return row;
        };
        row.type_name = self.declared();
        row.typed = Some(matches!(rendering.rendered, Rendered::Value { .. }));
        row.payload_bytes = Some(self.bytes.len());
        row.value = self.value();
        row.verdict = Some(conformance.token());
        match conformance {
            Conformance::Invalid { violations } => row.violations = Some(violations.clone()),
            Conformance::Undecodable { reason, .. } => row.decode_error = Some(reason.clone()),
            Conformance::Valid | Conformance::NotChecked { .. } => {}
        }
        row
    }

    /// One `--fmt` line.
    fn formatted(
        &self,
        fmt: &str,
        n: usize,
        sample: &zenkey_fleet::SampleView,
        namespace: &str,
    ) -> String {
        let value = match self.value() {
            Some(serde_json::Value::String(s)) => s,
            Some(v) => v.to_string(),
            None => String::new(),
        };
        let qos = qos_summary(
            sample.priority,
            sample.congestion_control,
            sample.reliability,
            sample.express,
        );
        let timestamp = sample.timestamp.map(|t| t.to_string());
        let source = sample.source.as_ref().map(source_summary);
        let attachment = self
            .attachment
            .as_ref()
            .map(crate::render::payload_text)
            .or_else(|| {
                sample
                    .attachment
                    .as_ref()
                    .map(super::sample::attachment_display)
            });
        let declared = self.declared();
        let relative = zenkey::grammar::strip_base(namespace, &sample.key);
        format_sample(
            fmt,
            &SampleLine {
                n,
                wire_key: &sample.key,
                base: namespace,
                type_name: declared.as_deref(),
                encoding: &sample.encoding,
                payload_len: self.bytes.len(),
                timestamp: timestamp.as_deref(),
                value: &value,
                attachment: attachment.as_deref(),
                qos: Some(&qos),
                source: source.as_deref(),
                zk2: Some(Zk2Positions {
                    relative,
                    identity: &self.identity,
                }),
            },
        )
    }

    /// The table form: the wire key, then the rendering with the rung it
    /// stopped at, and what failed against the declared type on stderr.
    fn print(&self, sample: &zenkey_fleet::SampleView, hex_payload: bool, rate_suffix: &str) {
        let Some((rendering, conformance)) = &self.checked else {
            println!(
                "{}\n  <tombstone — a deletion, not an empty value>{rate_suffix}",
                sample.key
            );
            return;
        };
        if hex_payload {
            let tag = self
                .declared()
                .map(|d| format!("<{d}>"))
                .unwrap_or_else(|| format!("<{}>", identity_words(&self.identity)));
            println!("{}\n  {tag} {}{rate_suffix}", sample.key, hex(&self.bytes));
        } else {
            println!(
                "{}\n  {}{rate_suffix}",
                sample.key,
                crate::render::payload_text(rendering)
            );
        }
        if let Some(a) = &self.attachment {
            println!("  attachment: {}", crate::render::payload_text(a));
        }
        match conformance {
            Conformance::Invalid { violations } => {
                for v in violations {
                    eprintln!("  invalid: {v}");
                }
            }
            Conformance::Valid
            | Conformance::Undecodable { .. }
            | Conformance::NotChecked { .. } => {}
        }
    }
}

/// An identity in a few words, for a tag where no type was reached.
fn identity_words(id: &KeyIdentity) -> String {
    match &id.unresolved {
        Some(why) => why.words(),
        None => id.group.label(),
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
        let mut row = zenkey_fleet::SampleRow::of_key(
            "prod/zk2/host-a/tc/tc.netif.v1/state/namespaces",
            "prod",
        );
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
