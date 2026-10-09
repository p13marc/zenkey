//! `.zsnap` — a snapshot kept on disk (RFC 13 §4.4, v1.34; #219): the
//! writer, the reader, and the collection itself — zk2's since #612
//! (FJ8b): the owners' current state, one S4 GET per selector, each row
//! resolved through the raw observers' lens. Version 2; a version-1 file
//! is v1's, and the `v1` branch reads it.
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

/// The current `.zsnap` format version, written into every header: 2 since
/// #612 (FJ8b), zk2's rows.
pub const ZSNAP_VERSION: u32 = 2;

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
        // The version first, so a file of another version is refused by its
        // number rather than by the first field this version lacks.
        let version = serde_json::from_str::<serde_json::Value>(&first)
            .ok()
            .and_then(|v| v.get("zsnap").and_then(serde_json::Value::as_u64));
        if let Some(v) = version
            && v != u64::from(ZSNAP_VERSION)
        {
            let hint = if v == 1 {
                " — a version-1 file is v1's registry snapshot, which the `v1` branch reads"
            } else {
                ""
            };
            return Err(Error::malformed(
                ".zsnap",
                format!("unsupported version {v} (this reader speaks {ZSNAP_VERSION}){hint}"),
            ));
        }
        let header: ZsnapHeader = serde_json::from_str(&first)
            .map_err(|e| Error::malformed_with(".zsnap line 1", "is not a header", e))?;
        if header.zsnap != ZSNAP_VERSION {
            let hint = if header.zsnap == 1 {
                " — a version-1 file is v1's registry snapshot, which the `v1` branch reads"
            } else {
                ""
            };
            return Err(Error::malformed(
                ".zsnap",
                format!(
                    "unsupported version {} (this reader speaks {ZSNAP_VERSION}){hint}",
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

/// What to collect.
#[derive(Debug, Clone)]
pub struct SnapshotSpec {
    /// Full wire selectors, one state GET each: already the state
    /// projection of what the operator asked ([`crate::state_projection`]).
    pub selectors: Vec<String>,
    /// Reply-wait per GET.
    pub timeout: std::time::Duration,
    /// Replies kept per GET (#339); what the bound cost rides the header.
    pub max_replies: usize,
}

/// What [`take_snapshot`] hands back: the snapshot, and the selectors whose
/// GET could not be issued at all (asked, never put — O5).
#[derive(Debug, Clone)]
pub struct Taken {
    pub snapshot: Snapshot,
    pub incomplete: Vec<String>,
}

/// One S4 GET's replies (spec §4.2): target `All`, consolidation `Latest`,
/// a reply on a non-concrete key discarded by rule (R6).
struct StateReplies {
    values: Vec<(
        crate::bus::monitor::SampleView,
        crate::model::snapshot::Replier,
    )>,
    errors: u64,
    elided: u64,
    discarded: u64,
}

async fn state_get(
    session: &zenoh::Session,
    selector: &str,
    timeout: std::time::Duration,
    max_replies: usize,
) -> Result<StateReplies> {
    use zenoh::query::{ConsolidationMode, QueryTarget};
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::Latest)
        .timeout(timeout)
        .await
        .map_err(|e| Error::bus("snapshot", selector, e))?;
    let mut out = StateReplies {
        values: Vec::new(),
        errors: 0,
        elided: 0,
        discarded: 0,
    };
    while let Ok(reply) = replies.recv_async().await {
        let replier = reply.replier_id().map(|e| e.zid());
        match reply.result() {
            Ok(sample) if sample.key_expr().is_wild() => out.discarded += 1,
            Ok(sample) => {
                if out.values.len() >= max_replies {
                    out.elided += 1;
                    continue;
                }
                out.values
                    .push((crate::bus::monitor::SampleView::of(sample), replier));
            }
            Err(_) => out.errors += 1,
        }
    }
    Ok(out)
}

/// Collect a snapshot of the owners' current state (RFC 13 §4.4; spec §4.2
/// S4).
///
/// One state GET per selector — target `All`, consolidation `Latest`, R6's
/// discard — all at once, on `session` (in no namespace: the selectors are
/// wire keys and rows keep them whole). The replies fold per key across
/// selectors ([`crate::model::snapshot::fold_latest`]), and each kept value
/// becomes a row through `lens` ([`crate::model::snapshot::row_of`]): its
/// identity, its conformance to its declared type, whose clock stamped it,
/// and who holds it.
///
/// `collection_span_s` is measured from before the first GET to after the
/// last row. A fan-in GET is collected *over* a span, and every rendering
/// states it; the presence read behind `lens` was the caller's, before.
pub async fn take_snapshot(
    session: &zenoh::Session,
    lens: &crate::model::lens::Lens<'_>,
    spec: &SnapshotSpec,
) -> Result<Taken> {
    use crate::model::snapshot::{fold_latest, row_of};

    let started = std::time::Instant::now();
    let collected_at = crate::tape::record::rfc3339_now();

    let gets = futures_util::future::join_all(spec.selectors.iter().map(|selector| async move {
        (
            selector.clone(),
            state_get(session, selector, spec.timeout, spec.max_replies).await,
        )
    }))
    .await;

    let mut values = Vec::new();
    let (mut errors, mut elided, mut discarded) = (0u64, 0u64, 0u64);
    let mut incomplete = Vec::new();
    for (selector, replies) in gets {
        match replies {
            Ok(r) => {
                errors += r.errors;
                elided += r.elided;
                discarded += r.discarded;
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
    let rows: Vec<SnapshotRow> = kept
        .into_values()
        .map(|(view, replier)| row_of(lens, &view, replier))
        .collect();

    let header = ZsnapHeader {
        zsnap: ZSNAP_VERSION,
        selectors: spec.selectors.clone(),
        base: lens.namespace().to_owned(),
        collected_at,
        collection_span_s: started.elapsed().as_secs_f64(),
        asked: spec.selectors.len() as u64,
        answered,
        elided,
        errors,
        discarded,
        superseded,
        presence: lens.scope().presence,
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
        no_instance: 0,
        unattributed: 0,
        nonconforming: 0,
        incomplete,
    };
    for row in &snapshot.rows {
        match row.holder {
            Holder::Live { .. } => report.live += 1,
            Holder::NoInstance { .. } => report.no_instance += 1,
            Holder::Unattributed { .. } => report.unattributed += 1,
        }
        if row.conformance.is_violation() {
            report.nonconforming += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Conformance, Holder, KeyGroup, KeyIdentity};

    fn header() -> ZsnapHeader {
        ZsnapHeader {
            zsnap: ZSNAP_VERSION,
            selectors: vec!["zk2/*/*/*/state/**".into()],
            base: String::new(),
            collected_at: "2026-10-09T00:00:00Z".into(),
            collection_span_s: 0.75,
            asked: 1,
            answered: 3,
            elided: 0,
            errors: 1,
            discarded: 0,
            superseded: 1,
            presence: None,
        }
    }

    fn row(key: &str) -> SnapshotRow {
        SnapshotRow {
            key: key.into(),
            identity: KeyIdentity {
                group: KeyGroup::Resource {
                    address: "lab/m".into(),
                    iface: "m.v1".into(),
                    token: "state".into(),
                    resource: None,
                },
                values: Default::default(),
                unresolved: Some(crate::report::Unresolved::PresenceNotRead),
            },
            delete: false,
            bytes: Some("e30=".into()),
            encoding: Some("application/json".into()),
            timestamp: Some("7f00...".into()),
            stamper: None,
            source_zid: Some("ab12".into()),
            conformance: Conformance::NotChecked {
                reason: "presence was not read".into(),
            },
            holder: Holder::NoInstance {
                address: "lab/m".into(),
            },
        }
    }

    /// Writer → reader is the identity on the whole document.
    #[test]
    fn a_snapshot_round_trips_through_the_file() {
        let rows = vec![row("zk2/lab/m/m.v1/state/a"), row("zk2/lab/m/m.v1/state/b")];
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
    /// by the version's number; v1's files are named as v1's; and a row is
    /// not a header.
    #[test]
    fn the_header_is_a_contract() {
        let future = r#"{"zsnap":99,"selectors":[],"base":"","collected_at":"x","collection_span_s":0,"asked":0,"answered":0}"#;
        let err = ZsnapReader::new(future.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("unsupported version 99"), "{err}");

        let v1 = r#"{"zsnap":1,"selectors":["v1/**"],"base":"","collected_at":"x","collection_span_s":0,"asked":1,"answered":0}"#;
        let err = ZsnapReader::new(v1.as_bytes()).err().unwrap().to_string();
        assert!(err.contains("unsupported version 1"), "{err}");
        assert!(err.contains("`v1` branch"), "{err}");

        let not_a_header = r#"{"key":"zk2/x","delete":false}"#;
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
            serde_json::to_string(&row("zk2/x")).unwrap()
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
    fn the_report_counts_holders_and_nonconforming_rows() {
        let mut live = row("zk2/lab/m/m.v1/state/a");
        live.holder = Holder::Live {
            address: "lab/m".into(),
            answered_by: crate::report::AnsweredBy::Owner,
        };
        live.conformance = Conformance::Invalid {
            violations: vec!["/x: expected integer".into()],
        };
        let snapshot = Snapshot {
            header: header(),
            rows: vec![live, row("zk2/lab/m/m.v1/state/b")],
        };
        let r = report_of(&snapshot, Some("a.zsnap".into()), vec![]);
        assert_eq!((r.live, r.no_instance, r.unattributed), (1, 1, 0));
        assert_eq!(r.nonconforming, 1);
    }
}
