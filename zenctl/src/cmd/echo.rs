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
//! The scope is the raw observers' one ([`zk2::wire_scope`]): a typed
//! selector, or the deployment's zk2 data, and with `--explicit` its
//! explicit kinds by name (#924), each watched on the same monitor.
//!
//! The row dialect is the one `pub --from ndjson` and `.zrec` read back
//! (#235, FJ8a's `qos_axes`), with the key's `identity` beside it; the
//! tagged `dropped` lines are stream metadata, skipped by the reader.

use anyhow::Result;
use zenkey_fleet::model::render::Member;
use zenkey_fleet::report::Conformance;
use zenkey_fleet::{Lens, LensFeed};

use super::sample::{
    Line, Payload, SampleLine, TOMBSTONE, format_sample, hex, qos_summary, source_summary,
};
use crate::bus::Deployment;
use crate::cli::EchoArgs;
use crate::cmd::zk2;

/// `zenctl echo [SELECTOR]` — subscribe first, and resolve beside it.
pub async fn run(cli: EchoArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let catalogs = zk2::load_catalogs(&cli.contracts)?;
    let EchoArgs {
        selector,
        explicit: crate::cli::ExplicitArgs { explicit },
        face: across,
        fmt,
        raw,
        hex: hex_payload,
        rate,
        no_decode,
        count,
        from,
        contracts: _,
        ns: _,
    } = cli;
    if let Some(file) = from {
        let shape = match &fmt {
            _ if raw => Shape::Raw,
            Some(f) => Shape::Fmt(f.as_str()),
            None => Shape::Table { hex: hex_payload },
        };
        let catalog = catalogs
            .one()
            .map_err(|e| crate::exit::unaskable!("echo --from --catalog: {e}"))?
            .cloned();
        return from_file(
            &file,
            &dep,
            contracts,
            catalog,
            !across.across_face.is_empty(),
            raw || no_decode,
            count,
            shape,
        );
    }
    let namespace = dep.namespace().to_owned();
    let face = zk2::FaceScope::of(&across, &contracts, &namespace, zk2::Reads::Samples)?;
    let dep = match &face {
        Some(f) => f.deployment(dep),
        None => dep,
    };
    let selectors = zk2::wire_scope(
        selector.as_slice(),
        explicit,
        &namespace,
        zk2::Reads::Samples,
        face.as_ref(),
    )?;

    // The data plane in no namespace: every key as the wire carries it.
    let session = dep.link().session().await?;
    let store = zk2::store(&dep, &contracts);
    // The deployment in its namespace, for the lens — unless nothing is to
    // be decoded, which asks nothing of it, or the scope is across a face,
    // where no descriptor crosses and --contracts resolves every key
    // (observer §7.5).
    let feed = if raw || no_decode || face.is_some() {
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
    for s in &selectors {
        match &face {
            Some(face) => face.watch(&monitor, s).await?,
            None => monitor.watch(s).await?,
        };
    }

    if let Some(face) = &face {
        eprintln!("{}", face.note(&selectors));
    }
    let ndjson = crate::render::Mode::of(dep.format()).machine();
    // What the scope cannot reach is said in every format: on standard
    // error, beside an NDJSON stream of rows that has no place for it
    // (observer §8.2, text 0.3, F-292).
    eprintln!(
        "echoing {}{} (ctrl-c to stop)",
        selectors.join(" + "),
        zk2::scope_note(&selectors)
    );
    let mut seen = 0usize;
    let mut dropped_total = 0u64;
    // One reader per frame key (core §7.7), for the window's life: a frame
    // lost before another turns the fields it omits unknown.
    let mut frames = zenkey_fleet::model::frame::Readers::new();
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
        let lens = Lens::new(&namespace, catalog.as_deref(), &store)
            .offline(&contracts)
            .catalogs(&catalogs);
        let line = line_of(&sample, &lens, raw || no_decode, &mut frames);
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
        } else {
            let shape = match &fmt {
                _ if raw => Shape::Raw,
                Some(fmt) => Shape::Fmt(fmt),
                None => Shape::Table { hex: hex_payload },
            };
            let printed = human(&line, &sample, shape, seen, &namespace, &rate_suffix);
            for l in printed.out {
                println!("{l}");
            }
            for l in printed.err {
                eprintln!("{l}");
            }
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

/// `echo --from FILE` (the tooling guide's §5, version 4): a capture
/// printed through the offline lens, no session opened. A version-4 header
/// stands in presence's place ([`zenkey_fleet::model::recorded`]), its
/// catalog, or `catalog` when that is its or a successor of it, dating its
/// times; `contracts` holds what it does not carry, and with `across` a key
/// it lists no revision for reads through them as across a face.
#[allow(clippy::too_many_arguments)]
fn from_file(
    file: &std::path::Path,
    dep: &Deployment,
    contracts: zenkey_fleet::ContractSet,
    catalog: Option<zenkey_fleet::DeploymentCatalog>,
    across: bool,
    structural_only: bool,
    count: usize,
    shape: Shape<'_>,
) -> Result<()> {
    use zenkey_fleet::tape::record::{ZrecItem, ZrecReader};
    let opened = std::fs::File::open(file)
        .map_err(|e| crate::exit::unaskable!("--from {}: {e}", file.display()))?;
    let mut reader = ZrecReader::new(std::io::BufReader::new(opened))
        .map_err(|e| crate::exit::unaskable!("--from {}: {e}", file.display()))?;
    let header = reader.header().clone();
    let (catalogs, recorded_word) =
        zenkey_fleet::tape::record::reader_catalogs(&header, catalog.as_ref())
            .map_err(|e| crate::exit::unaskable!("--from {}: {e}", file.display()))?;
    if recorded_word && let Some(c) = &header.catalog {
        eprintln!(
            "reading {} under its header's catalog {} (epoch {}), the recorder's word: give \
             --catalog to read it through one of your own",
            file.display(),
            c.id,
            c.epoch
        );
    }
    let recorded = zenkey_fleet::model::recorded::Recorded::of(&header, contracts.clone());
    for (fp, b) in recorded.bundles() {
        if let zenkey_fleet::model::recorded::Bundled::Refused(tag) = b {
            eprintln!("bundle {fp} refused ({tag}, core §9.6): its keys stop at rung 5");
        }
    }
    if header.zrec >= 4 && header.bundles.is_none() && contracts.is_empty() {
        eprintln!(
            "{} carries no bundle (record --no-bundles): give --contracts to decode it (O4)",
            file.display()
        );
    }
    let namespace = header.base.clone();
    let mut lens = zenkey_fleet::Lens::new(&namespace, None, &recorded)
        .recorded(&recorded)
        .catalogs(&catalogs);
    if across {
        lens = lens.offline(&contracts);
    }
    let ndjson = crate::render::Mode::of(dep.format()).machine();
    let mut frames = zenkey_fleet::model::frame::Readers::new();
    let mut seen = 0usize;
    while let Some(item) = reader.next() {
        let (row, t_us, timestamp) = match item {
            Ok(ZrecItem::Sample {
                row,
                t_us,
                timestamp,
                ..
            }) => (row, t_us, timestamp),
            Ok(ZrecItem::Preamble { row, timestamp }) => (row, Some(0), timestamp),
            Ok(ZrecItem::Dropped(n)) => {
                if ndjson {
                    println!(
                        "{}",
                        crate::render::Row::tagged("dropped", serde_json::json!({ "dropped": n }))
                            .into_line()
                    );
                } else {
                    eprintln!("-- the capture dropped {n} sample(s) here --");
                }
                continue;
            }
            Ok(ZrecItem::Trigger(_)) => continue,
            Err(e) => {
                eprintln!("--from {}: {e}", file.display());
                continue;
            }
        };
        seen += 1;
        let sample = view_of(&row);
        let line = line_of(&sample, &lens, structural_only, &mut frames);
        if ndjson {
            let mut out = zenkey_fleet::SampleRow::of_key(&row.key);
            out.t = t_us;
            out.timestamp = timestamp;
            out.delete = row.delete;
            out.encoding.clone_from(&row.encoding);
            line.fill(&mut out, row.attachment.as_deref());
            println!("{}", out.to_line());
        } else {
            let printed = human(&line, &sample, shape_ref(&shape), seen, &namespace, "");
            for l in printed.out {
                println!("{l}");
            }
            for l in printed.err {
                eprintln!("{l}");
            }
        }
        if count > 0 && seen >= count {
            break;
        }
    }
    Ok(())
}

/// A shape borrowed again, for each row.
fn shape_ref<'a>(s: &'a Shape<'a>) -> Shape<'a> {
    match s {
        Shape::Raw => Shape::Raw,
        Shape::Fmt(f) => Shape::Fmt(f),
        Shape::Table { hex } => Shape::Table { hex: *hex },
    }
}

/// A recorded row as the sample it was: its key, bytes, encoding,
/// attachment and the QoS axes it rode, received now.
fn view_of(row: &zenkey_fleet::tape::ingest::IngestRow) -> zenkey_fleet::SampleView {
    let qos = row.qos_axes.unwrap_or(zenkey_fleet::WireQos::DEFAULT);
    zenkey_fleet::SampleView {
        key: row.key.clone(),
        payload: zenoh::bytes::ZBytes::from(row.payload.clone()),
        encoding: row.encoding.clone().unwrap_or_default(),
        kind: if row.delete {
            zenoh::sample::SampleKind::Delete
        } else {
            zenoh::sample::SampleKind::Put
        },
        timestamp: None,
        stamped_by: None,
        attachment: row
            .attachment
            .as_ref()
            .map(|a| zenoh::bytes::ZBytes::from(a.clone())),
        priority: qos.priority,
        congestion_control: qos.congestion,
        reliability: qos.reliability,
        express: qos.express,
        source: None,
        received: std::time::Instant::now(),
    }
}

/// One sample through the lens.
fn line_of(
    sample: &zenkey_fleet::SampleView,
    lens: &Lens<'_>,
    structural_only: bool,
    frames: &mut zenkey_fleet::model::frame::Readers,
) -> Line {
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
        Some(frames),
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

/// How a human reads the stream: the key and the bytes in hex (`--raw`),
/// one line from the `%` vocabulary (`--fmt`), or the table form, its
/// rendering or the type tag and the hex (`--hex`).
enum Shape<'a> {
    Raw,
    Fmt(&'a str),
    Table { hex: bool },
}

/// One sample as a human form prints it: stdout's lines, then stderr's.
#[derive(Debug, Default)]
struct Printed {
    out: Vec<String>,
    err: Vec<String>,
}

/// One sample in a human form, pure so a snapshot pins it. A deletion
/// reads as one in every shape, in the words `get` prints for a tombstone
/// reply (#765, #766): `--fmt`'s `%v` is those words and `%l` is 0, never
/// an empty value. `n` is `--fmt`'s `%n`; `rate_suffix` is `--rate`'s.
fn human(
    line: &Line,
    sample: &zenkey_fleet::SampleView,
    shape: Shape<'_>,
    n: usize,
    namespace: &str,
    rate_suffix: &str,
) -> Printed {
    let mut p = Printed::default();
    match shape {
        Shape::Raw => {
            let delete = sample.kind == zenoh::sample::SampleKind::Delete;
            p.out.push(sample.key.clone());
            p.out.push(if delete {
                format!("  {TOMBSTONE}{rate_suffix}")
            } else {
                format!("  {}{rate_suffix}", hex(&line.bytes))
            });
            if let Some(a) = &sample.attachment {
                p.out.push(format!("  attachment: {}", hex(&a.to_bytes())));
            }
        }
        Shape::Fmt(fmt) => p.out.push(formatted(line, fmt, n, sample, namespace)),
        Shape::Table { hex: hex_payload } => {
            p.out.push(sample.key.clone());
            let Some((rendering, conformance)) = &line.checked else {
                p.out.push(format!("  {TOMBSTONE}{rate_suffix}"));
                return p;
            };
            p.out.push(if hex_payload {
                format!("  {} {}{rate_suffix}", line.tag(), hex(&line.bytes))
            } else {
                format!("  {}{rate_suffix}", crate::render::payload_text(rendering))
            });
            // An event: its time beside its key's stamp (observer §7.3).
            if let Some(o) = &line.occurred {
                p.out.push(format!("  at {o}"));
            }
            if let Some(a) = &line.attachment {
                p.out
                    .push(format!("  attachment: {}", crate::render::payload_text(a)));
            }
            // A revision CH's tag chose, across a face: the tool's reading,
            // never the deployment's word (observer §7.5).
            if let (Some(c), Some(r)) = (&line.identity.chosen, &rendering.resource) {
                p.out.push(format!(
                    "  revision {} by tag (step {}, observer §7.5)",
                    r.fingerprint, c.step
                ));
            }
            if let Conformance::Invalid { violations } = conformance {
                p.err
                    .extend(violations.iter().map(|v| format!("  invalid: {v}")));
            }
        }
    }
    p
}

/// One `--fmt` line.
fn formatted(
    line: &Line,
    fmt: &str,
    n: usize,
    sample: &zenkey_fleet::SampleView,
    namespace: &str,
) -> String {
    let value = if sample.kind == zenoh::sample::SampleKind::Delete {
        TOMBSTONE.to_owned()
    } else {
        line.value_text()
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    use snapbox::{assert_data_eq, str};
    use zenoh::sample::SampleKind;

    const ETH0: &str = "prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";

    /// A sample as the monitor hands it over, its payload empty and its
    /// stamp the one `get`'s tombstone reply carries: a deletion or, for
    /// contrast, an empty put, under its type's predefined Encoding (core
    /// §7.2), since a put under `zenoh/bytes` carries a zk2c envelope
    /// (§7.5).
    fn sample(kind: SampleKind) -> zenkey_fleet::SampleView {
        zenkey_fleet::SampleView {
            key: ETH0.into(),
            payload: zenoh::bytes::ZBytes::new(),
            encoding: match kind {
                SampleKind::Put => "application/json".into(),
                SampleKind::Delete => "zenoh/bytes".into(),
            },
            kind,
            timestamp: Some(zenoh::time::Timestamp::new(
                zenoh::time::NTP64(1 << 32),
                "a1b2".parse().expect("an id"),
            )),
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::Data,
            congestion_control: zenoh::qos::CongestionControl::Drop,
            reliability: zenoh::qos::Reliability::Reliable,
            express: false,
            source: None,
            received: std::time::Instant::now(),
        }
    }

    /// One sample's stdout in each human shape: the table form, `--hex`,
    /// `--raw` (which resolves nothing) and `--fmt`, with the format
    /// `get`'s own snapshot uses.
    fn shown(s: &zenkey_fleet::SampleView, lens: &Lens<'_>) -> String {
        let mut out = String::new();
        for (name, shape) in [
            ("table", Shape::Table { hex: false }),
            ("--hex", Shape::Table { hex: true }),
            ("--raw", Shape::Raw),
            ("--fmt", Shape::Fmt("%n %k <%t> %v (%l B) %T")),
        ] {
            let line = line_of(
                s,
                lens,
                matches!(shape, Shape::Raw),
                &mut Default::default(),
            );
            out.push_str(&format!("# {name}\n"));
            for l in human(&line, s, shape, 1, "prod", "").out {
                out.push_str(&l);
                out.push('\n');
            }
        }
        out
    }

    /// #766: a deletion reads as one in every human shape, `--fmt` and
    /// `--raw` included, in the words `get` prints for a tombstone reply:
    /// the `--fmt` line is the one `get`'s snapshot pins, `%v` the words
    /// and `%l` 0. An empty put, drawn below it, is a value.
    #[test]
    fn a_deletion_renders_as_one_in_every_human_form() {
        let contracts = zenkey_fleet::ContractSet::new();
        let lens = Lens::new("prod", None, &contracts);
        assert_data_eq!(
            shown(&sample(SampleKind::Delete), &lens),
            str![[r#"
# table
prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0
  <tombstone — a deletion, not an empty value>
# --hex
prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0
  <tombstone — a deletion, not an empty value>
# --raw
prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0
  <tombstone — a deletion, not an empty value>
# --fmt
1 prod/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0 <> <tombstone — a deletion, not an empty value> (0 B) 4294967296/a1b2

"#]]
        );
        let put = shown(&sample(SampleKind::Put), &lens);
        assert!(put.contains("\n  <0 bytes>  (structural: "), "{put}");
        assert!(!put.contains(TOMBSTONE), "an empty put is a value: {put}");
    }

    /// The envelope first (core §7.5, observer §7.3; #976's A2): `echo` of a
    /// state sample under `zenoh/bytes` renders its zk2c envelope's record
    /// as the contract's type, in every human shape and in the row, `--raw`
    /// sniffing the record; a refused envelope is undecodable, its refusal
    /// named, never sniffed.
    #[test]
    fn an_enveloped_sample_renders_its_record() {
        use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};
        let (contracts, problems) = zenkey_fleet::ContractSet::load_path(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../examples/zk2/tcgui/tc.netif.v1.toml"),
        );
        assert!(problems.is_empty(), "{problems:?}");
        let lens = Lens::new("prod", None, &contracts).offline(&contracts);
        let record = br#"{"namespaces":["default"]}"#;
        let envelope = Envelope {
            form: zenkey_model::wire::Form::Mutation,
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(7),
                seq: Seq::new(3).expect("a seq"),
            },
            record,
        }
        .write();
        let mut s = sample(SampleKind::Put);
        s.key = "prod/zk2/host-a/tc/tc.netif.v1/state/namespaces".into();
        s.encoding = "zenoh/bytes".into();
        s.payload = zenoh::bytes::ZBytes::from(envelope.clone());
        let shown = shown(&s, &lens);
        assert!(
            shown.contains(r#"{"namespaces":["default"]}"#),
            "the record, never the envelope's bytes: {shown}"
        );
        assert!(shown.contains("json:Namespaces"), "{shown}");
        let line = line_of(&s, &lens, false, &mut Default::default());
        let row = row(&line, &s);
        assert_eq!(
            row.value,
            Some(serde_json::json!({"namespaces": ["default"]}))
        );
        assert_eq!(row.verdict.as_deref(), Some("valid"));
        // Today's bytes under zenoh/bytes: an envelope refused.
        s.payload = zenoh::bytes::ZBytes::from(record.to_vec());
        let shown = shown_refused(&s, &lens);
        assert!(shown.contains("zk2c envelope refused: "), "{shown}");
        assert!(
            !shown.contains(r#"["default"]"#),
            "a refused envelope is never sniffed: {shown}"
        );
    }

    /// An event (core §7.5's form 4, observer §7.3; #980's C2c): the table
    /// form shows its time beside its key's stamp, `at t, run r, seq s`,
    /// counted from the deployment epoch where no epoch is given; its
    /// record is the payload.
    #[test]
    fn an_event_shows_its_time_beside_its_stamp() {
        use zenkey_model::collection::Time;
        use zenkey_model::wire::{Event, Run, Seq, Stamp, Tag};
        let contracts = zenkey_fleet::ContractSet::new();
        let lens = Lens::new("prod", None, &contracts);
        let event = |time| Event {
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(5),
                seq: Seq::new(9).expect("a seq"),
            },
            time,
            record: b"up",
        };
        let mut s = sample(SampleKind::Put);
        s.key = "prod/zk2/lab/journal/journal.v1/events/entries/002g0009".into();
        s.encoding = "zenoh/bytes".into();
        s.payload = zenoh::bytes::ZBytes::from(
            event(Time::Known { s: 3600, ms: 250 })
                .write()
                .expect("bytes"),
        );
        let line = line_of(&s, &lens, false, &mut Default::default());
        let out = human(&line, &s, Shape::Table { hex: false }, 1, "prod", "").out;
        assert_eq!(
            out.last().map(String::as_str),
            Some("  at 3600.250 s from the deployment epoch, run 0005, seq 9"),
            "{out:?}"
        );
        assert!(out[1].contains("up"), "the record: {out:?}");
        s.payload = zenoh::bytes::ZBytes::from(event(Time::Unknown).write().expect("bytes"));
        let line = line_of(&s, &lens, false, &mut Default::default());
        let out = human(&line, &s, Shape::Table { hex: false }, 1, "prod", "").out;
        assert_eq!(
            out.last().map(String::as_str),
            Some("  at time unknown, run 0005, seq 9")
        );
    }

    /// Frames (core §7.7, observer §7.6; #981's K2): `echo` reads a frame
    /// key's frames through one reader for the window's life, so a lost
    /// frame turns the fields its successor omits unknown; a scheduled
    /// record reads scheduled with its jitter; `--raw` shows a frame's
    /// envelope and size, never its body sniffed.
    #[test]
    fn a_frame_keys_frames_read_in_order() {
        use serde_json::json;
        use zenkey_model::frame::{Frame, Layout};
        use zenkey_model::wire::{Run, Seq, Stamp};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/frames/linkframes.v1.toml");
        let (contracts, problems) = zenkey_fleet::ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        let lens = Lens::new("prod", None, &contracts).offline(&contracts);
        let c = zenkey_model::contract::load_path(&path).contract.unwrap();
        let layout = |t: &str| {
            let r = c
                .resources
                .iter()
                .find(|r| r.template.as_str() == t)
                .unwrap();
            Layout::of_resource(&c, r).unwrap().unwrap()
        };
        let frame = |template: &str, seq: u32, v: serde_json::Value| {
            let l = layout(template);
            let put = Frame::from_json(&v, &l, None)
                .unwrap()
                .envelope(
                    &l,
                    Stamp {
                        run: Run::U16(7),
                        seq: Seq::new(seq).unwrap(),
                    },
                )
                .unwrap();
            let mut s = sample(SampleKind::Put);
            s.key = format!("prod/zk2/lab/modem/linkframes.v1/stream/{template}");
            s.encoding = "zenoh/bytes".into();
            s.payload = zenoh::bytes::ZBytes::from(put);
            s
        };
        let table = |line: &Line, s: &zenkey_fleet::SampleView| {
            human(line, s, Shape::Table { hex: false }, 1, "prod", "")
                .out
                .join("\n")
        };
        let mut frames = zenkey_fleet::model::frame::Readers::new();
        let key = frame(
            "frames/link",
            10,
            json!({"time": {"s": 100, "ms": 0}, "records": [{"full": true, "values":
                {"dropped": 7, "queue_depth": 12, "duty_available": 1}}]}),
        );
        let first = line_of(&key, &lens, false, &mut frames);
        let shown = table(&first, &key);
        assert!(shown.contains("zk2c frame of packed:Link"), "{shown}");
        assert!(
            shown.contains("run 0007, seq 10, the first frame read"),
            "{shown}"
        );
        assert!(shown.contains("duty_wait_ms absent"), "{shown}");
        // 11 lost: 12's omitted fields unknown, never their last value.
        let delta = frame(
            "frames/link",
            12,
            json!({"time": {"s": 220, "ms": 0}, "records": [{"full": false, "values": {"dropped": 9}}]}),
        );
        let line = line_of(&delta, &lens, false, &mut frames);
        let shown = table(&line, &delta);
        assert!(shown.contains("seq 12, 1 record lost before it"), "{shown}");
        assert!(
            shown.contains("queue_depth unknown (records lost)"),
            "{shown}"
        );
        assert!(!shown.contains("queue_depth 12"), "{shown}");
        let row = row(&line, &delta);
        assert_eq!(row.value.as_ref().unwrap()["follows"], "gap");
        assert_eq!(row.verdict.as_deref(), Some("valid"));
        // The same frame alone, as `get` reads a reply: a reader's first.
        let alone = line_of(&delta, &lens, false, &mut Default::default());
        assert!(
            table(&alone, &delta)
                .contains("queue_depth unknown (not included since the first frame read)")
        );
        // Three records: the second scheduled, the third measured late.
        let s3 = frame(
            "frames/sample",
            30,
            json!({"time": {"s": 7200, "ms": 0}, "records": [
                {"full": true, "values": {"temperature": "21.5", "rssi": -85, "link_up": true}},
                {"full": false, "at": "scheduled", "values": {"temperature": "22"}},
                {"full": false, "at": 20750, "values": {"rssi": -90}}]}),
        );
        let shown = table(&line_of(&s3, &lens, false, &mut frames), &s3);
        assert!(
            shown.contains("#31 t=7210.000 scheduled ±500 ms"),
            "{shown}"
        );
        assert!(shown.contains("#32 t=7220.750 measured"), "{shown}");
        // `--raw`: the envelope and the size, the body never sniffed.
        let raw = line_of(&delta, &lens, true, &mut frames);
        assert!(
            human(&raw, &delta, Shape::Table { hex: false }, 1, "prod", "")
                .out
                .join("\n")
                .contains("<zk2c frame, run 0007, seq 12, "),
        );
    }

    fn shown_refused(s: &zenkey_fleet::SampleView, lens: &Lens<'_>) -> String {
        let mut out = shown(s, lens);
        let line = line_of(s, lens, false, &mut Default::default());
        out.push_str(&row(&line, s).to_line());
        out
    }

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
            chosen: None,
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
