//! `.zsnap` — a snapshot kept on disk (RFC 13 §4.4, v1.34; #219): the
//! writer, the reader, and — behind `decode` — the collection itself.
//!
//! The file is the `.zrec` dialect's sibling: newline-delimited JSON, line 1
//! the header, one row per key after it, payloads lossless as base64
//! `bytes`. A reader that does not know the version refuses the file rather
//! than guessing, with the same wording [`crate::ZrecReader`] uses.
//!
//! What deliberately does not exist here: a *replay* of a snapshot. A
//! snapshot is read, never published (RFC 13 §4.4); seeding a fleet from a
//! file is `--seed-state` over a version-2 `.zrec` preamble.

use std::io::{BufRead, Write};

use crate::report::{Snapshot, SnapshotRow, ZsnapHeader};
use crate::{Error, Result};

/// The current `.zsnap` format version, written into every header.
pub const ZSNAP_VERSION: u32 = 1;

fn io(e: std::io::Error) -> Error {
    Error::Io {
        path: std::path::PathBuf::new(),
        source: e,
    }
}

/// A `.zsnap` writer over any byte sink: header first, then rows. Wrap the
/// sink in a `BufWriter` — the writer emits line-at-a-time.
pub struct ZsnapWriter<W: Write> {
    out: W,
    rows: u64,
}

impl<W: Write> ZsnapWriter<W> {
    /// Write the header line and hand back a row writer.
    pub fn new(mut out: W, header: &ZsnapHeader) -> Result<Self> {
        serde_json::to_writer(&mut out, header).map_err(|e| io(e.into()))?;
        out.write_all(b"\n").map_err(io)?;
        Ok(ZsnapWriter { out, rows: 0 })
    }

    /// Write one row.
    pub fn write_row(&mut self, row: &SnapshotRow) -> Result<()> {
        serde_json::to_writer(&mut self.out, row).map_err(|e| io(e.into()))?;
        self.out.write_all(b"\n").map_err(io)?;
        self.rows += 1;
        Ok(())
    }

    /// Rows written so far.
    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Flush and hand the sink back.
    pub fn finish(mut self) -> Result<W> {
        self.out.flush().map_err(io)?;
        Ok(self.out)
    }
}

/// A `.zsnap` reader over any buffered byte source: header up front, then
/// one row per line — bounded memory, like the writer.
pub struct ZsnapReader<R: BufRead> {
    header: ZsnapHeader,
    lines: std::io::Lines<R>,
    /// 1-based number of the last line handed out (the header is line 1).
    line: u64,
}

impl<R: BufRead> ZsnapReader<R> {
    /// Parse the header line. A file without one is not a `.zsnap`; a
    /// version this reader does not speak is refused, never guessed at
    /// (RFC 13 §4.1's rule, which §4.4 inherits).
    pub fn new(source: R) -> Result<Self> {
        let mut lines = source.lines();
        let first = lines
            .next()
            .ok_or_else(|| Error::malformed(".zsnap", "empty file — no header line"))?
            .map_err(io)?;
        let header: ZsnapHeader = serde_json::from_str(&first)
            .map_err(|e| Error::malformed_with(".zsnap line 1", "is not a header", e))?;
        if header.zsnap != ZSNAP_VERSION {
            return Err(Error::malformed(
                ".zsnap",
                format!(
                    "unsupported version {} (this reader speaks {ZSNAP_VERSION})",
                    header.zsnap
                ),
            ));
        }
        Ok(ZsnapReader {
            header,
            lines,
            line: 1,
        })
    }

    pub fn header(&self) -> &ZsnapHeader {
        &self.header
    }

    /// The next row, or `Err` naming the line and the reason — a malformed
    /// row is reported, never silently skipped. `None` ends the file.
    #[allow(clippy::should_implement_trait)] // fallible, line-numbered next
    pub fn next(&mut self) -> Option<std::result::Result<SnapshotRow, String>> {
        loop {
            let line = match self.lines.next()? {
                Ok(l) => l,
                Err(e) => {
                    self.line += 1;
                    return Some(Err(format!("line {}: read: {e}", self.line)));
                }
            };
            self.line += 1;
            if line.trim().is_empty() {
                continue;
            }
            return Some(
                serde_json::from_str::<SnapshotRow>(&line)
                    .map_err(|e| format!("line {}: {e}", self.line)),
            );
        }
    }

    /// The whole file, or the first malformed line as an error — a diff over
    /// a file with a hole in it would be a diff of a document nobody wrote.
    pub fn read_all(mut self) -> Result<Snapshot> {
        let mut rows = Vec::new();
        while let Some(row) = self.next() {
            rows.push(row.map_err(|e| Error::malformed(".zsnap", e))?);
        }
        Ok(Snapshot {
            header: self.header,
            rows,
        })
    }
}

/// What to collect (`decode`).
#[cfg(feature = "decode")]
#[derive(Debug, Clone)]
pub struct SnapshotSpec {
    /// Full wire selectors, one fan-in GET each.
    pub selectors: Vec<String>,
    /// Reply-wait per GET, and the roster ask's bound.
    pub timeout: std::time::Duration,
    /// Replies kept per GET (#339); what the bound cost rides the header.
    pub max_replies: usize,
    /// Whether to ask the liveliness roster. Off, every holder is
    /// `unattributed { reason: "roster not asked" }` — cheaper, and honest.
    pub roster: bool,
}

/// What [`take_snapshot`] hands back: the snapshot, and the selectors whose
/// GET could not be issued at all (asked, never put — RFC 09 §5.1 O5).
#[cfg(feature = "decode")]
#[derive(Debug, Clone)]
pub struct Taken {
    pub snapshot: Snapshot,
    pub incomplete: Vec<String>,
}

/// Collect a snapshot (RFC 13 §4.4).
///
/// The watchdog's discipline first: the schema store is pre-warmed for
/// every producer the registry names and then sealed, so no decode inside
/// the collection reaches for the bus ([`crate::prewarm`],
/// [`crate::SchemaStore::seal`]). Then the roster ask and one
/// [`snapshot_get`](crate::bus::query::snapshot_get) per selector run
/// **together** — one [`crate::GetOpts`] across the GETs, so `elided`
/// sums. The replies fold per key last-writer-wins
/// ([`crate::model::snapshot::fold_latest`]), and each kept value becomes a
/// row: registration from [`crate::KeyFacts`], verdict from
/// [`crate::decode_sample`], holder from the roster.
///
/// `collection_span_s` is measured from before the first ask to after the
/// last row — the roster ask included. That is the honest number: a fan-in
/// GET is collected *over* a span, and every rendering states it.
#[cfg(feature = "decode")]
pub async fn take_snapshot(
    fleet: &crate::Fleet<'_>,
    slices: Option<&crate::SliceSet>,
    store: &crate::SchemaStore,
    spec: &SnapshotSpec,
) -> Result<Taken> {
    use crate::model::snapshot::{fold_latest, holder_of, registration_of, stamper_of, verdict_of};
    use crate::report::{Asked, VerdictWire};
    use zenoh::sample::SampleKind;

    let started = std::time::Instant::now();
    let collected_at = crate::tape::record::rfc3339_now();

    crate::model::decode::prewarm(fleet, store, slices).await;
    let _sealed = store.seal();

    let opts = crate::GetOpts::new(spec.timeout).max_replies(spec.max_replies);
    let gets = futures_util::future::join_all(spec.selectors.iter().map(|selector| {
        let opts = &opts;
        async move {
            (
                selector.clone(),
                crate::bus::query::snapshot_get(fleet.session(), selector, opts).await,
            )
        }
    }));
    let roster = async {
        if spec.roster {
            crate::bus::roster::roster(fleet, spec.timeout)
                .await
                .map(Some)
        } else {
            Ok(None)
        }
    };
    let (replies, roster) = tokio::join!(gets, roster);
    let roster = roster?;

    let mut values = Vec::new();
    let mut errors = 0u64;
    let mut incomplete = Vec::new();
    for (selector, replies) in replies {
        match replies {
            Ok(r) => {
                errors += r.errors;
                values.extend(r.values);
            }
            Err(e) => {
                tracing::warn!(selector, error = %e, "snapshot GET could not be issued");
                incomplete.push(selector);
            }
        }
    }
    let answered = values.len() as u64;
    let (kept, superseded) = fold_latest(values);

    let base = fleet.base();
    let mut rows = Vec::with_capacity(kept.len());
    for (key, (view, replier)) in kept {
        let mut facts = crate::KeyFacts::project(base, &key);
        if let Some(slices) = slices {
            facts.resolve(slices);
        }
        let delete = view.kind == SampleKind::Delete;
        let (bytes, verdict) = if delete {
            (
                None,
                VerdictWire::NotValidated {
                    reason: "tombstone".into(),
                },
            )
        } else {
            let payload = view.payload.to_bytes();
            let encoding = (!view.encoding.is_empty()).then_some(view.encoding.as_str());
            let decoded =
                crate::decode_sample(fleet, store, slices, &key, encoding, &payload).await;
            (
                Some(crate::tape::ingest::b64(&payload)),
                verdict_of(&decoded.verdict),
            )
        };
        let holder = holder_of(base, &key, &view, replier, roster.as_ref());
        rows.push(SnapshotRow {
            key,
            delete,
            bytes,
            encoding: (!view.encoding.is_empty()).then(|| view.encoding.clone()),
            timestamp: view.timestamp.map(|t| t.to_string()),
            stamper: view.stamped_by.as_ref().map(stamper_of),
            source: view.source.map(|s| format!("{}:{}#{}", s.zid, s.eid, s.sn)),
            source_zid: replier.map(|z| z.to_string()),
            registration: registration_of(&facts),
            verdict,
            holder,
        });
    }

    let header = ZsnapHeader {
        zsnap: ZSNAP_VERSION,
        selectors: spec.selectors.clone(),
        base: base.to_string(),
        collected_at,
        collection_span_s: started.elapsed().as_secs_f64(),
        asked: spec.selectors.len() as u64,
        answered,
        elided: opts.elided(),
        errors,
        superseded,
        roster: match &roster {
            Some(r) => Asked::Asked(r.len()),
            None => Asked::NotAsked,
        },
    };
    Ok(Taken {
        snapshot: Snapshot { header, rows },
        incomplete,
    })
}

/// The report `zenctl snapshot` renders, counted off the rows.
pub fn report_of(
    snapshot: &Snapshot,
    out: Option<String>,
    incomplete: Vec<String>,
) -> crate::report::SnapshotReport {
    use crate::report::Holder;
    let mut report = crate::report::SnapshotReport {
        header: snapshot.header.clone(),
        out,
        live: 0,
        storage_only: 0,
        unattributed: 0,
        incomplete,
    };
    for row in &snapshot.rows {
        match row.holder {
            Holder::Live { .. } => report.live += 1,
            Holder::StorageOnly { .. } => report.storage_only += 1,
            Holder::Unattributed { .. } => report.unattributed += 1,
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Asked, Holder, RegistrationWire, VerdictWire};

    fn header() -> ZsnapHeader {
        ZsnapHeader {
            zsnap: ZSNAP_VERSION,
            selectors: vec!["v1/**".into()],
            base: String::new(),
            collected_at: "2026-09-06T00:00:00Z".into(),
            collection_span_s: 0.75,
            asked: 1,
            answered: 3,
            elided: 0,
            errors: 1,
            superseded: 1,
            roster: Asked::Asked(1),
        }
    }

    fn row(key: &str) -> SnapshotRow {
        SnapshotRow {
            key: key.into(),
            delete: false,
            bytes: Some("e30=".into()),
            encoding: Some("application/json".into()),
            timestamp: Some("7f00...".into()),
            stamper: None,
            source: None,
            source_zid: Some("ab12".into()),
            registration: RegistrationWire::RegistryNotLoaded,
            verdict: VerdictWire::NotValidated {
                reason: "no_registry".into(),
            },
            holder: Holder::StorageOnly {
                origin: "h-aaaaaaaaaaaa".into(),
            },
        }
    }

    /// Writer → reader is the identity on the whole document.
    #[test]
    fn a_snapshot_round_trips_through_the_file() {
        let rows = vec![
            row("v1/h-aaaaaaaaaaaa/state/p/a"),
            row("v1/h-aaaaaaaaaaaa/state/p/b"),
        ];
        let mut sink = Vec::new();
        let mut w = ZsnapWriter::new(&mut sink, &header()).unwrap();
        for r in &rows {
            w.write_row(r).unwrap();
        }
        assert_eq!(w.rows(), 2);
        w.finish().unwrap();

        let snapshot = ZsnapReader::new(sink.as_slice())
            .unwrap()
            .read_all()
            .unwrap();
        assert_eq!(
            snapshot,
            Snapshot {
                header: header(),
                rows
            }
        );
    }

    /// A versioned reader refuses what it cannot speak rather than guessing,
    /// in the `.zrec` reader's words; and a row is not a header.
    #[test]
    fn the_header_is_a_contract() {
        let future = r#"{"zsnap":99,"selectors":[],"base":"","collected_at":"x","collection_span_s":0,"asked":0,"answered":0}"#;
        let err = ZsnapReader::new(future.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("unsupported version 99"), "{err}");

        let not_a_header = r#"{"key":"v1/x","delete":false}"#;
        let err = ZsnapReader::new(not_a_header.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("is not a header"), "{err}");

        let err = ZsnapReader::new("".as_bytes()).err().unwrap().to_string();
        assert!(err.contains("no header"), "{err}");
    }

    /// A malformed row names its line and stops the read — a diff over a
    /// document with a hole is not a diff of anything.
    #[test]
    fn a_malformed_row_is_reported_by_line() {
        let body = format!(
            "{}\n{}\nnot json\n",
            serde_json::to_string(&header()).unwrap(),
            serde_json::to_string(&row("v1/x")).unwrap()
        );
        let err = ZsnapReader::new(body.as_bytes())
            .unwrap()
            .read_all()
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("line 3"), "{err}");
    }

    #[test]
    fn the_report_counts_holders() {
        let mut live = row("v1/h-aaaaaaaaaaaa/state/p/a");
        live.holder = Holder::Live {
            origin: "h-aaaaaaaaaaaa".into(),
            answered_by: crate::report::AnsweredBy::Stamper,
        };
        let snapshot = Snapshot {
            header: header(),
            rows: vec![live, row("v1/h-aaaaaaaaaaaa/state/p/b")],
        };
        let r = report_of(&snapshot, Some("a.zsnap".into()), vec![]);
        assert_eq!((r.live, r.storage_only, r.unattributed), (1, 1, 0));
    }
}
