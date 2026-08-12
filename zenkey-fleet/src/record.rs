//! `.zrec` capture and replay (issue #39; RFC 09 §5.2 documents the
//! etiquette, this module is normative for the format).
//!
//! A `.zrec` file is newline-delimited JSON in the explorers' one row
//! dialect — the same shape `topic echo --format ndjson` emits and
//! [`crate::ingest::parse_row`] reads back — upgraded with what a pipe does
//! not need but a capture does: a versioned header line naming what was
//! asked, a lossless `"bytes"` payload (a `"value"` is a rendering), a
//! pacing offset `"t"` on the observer's arrival clock, and drop records
//! interleaved **where the gap happened** (RFC 09 §5.1 O6, applied to a
//! file — a capture taken while behind is a partial view and says so at
//! the position of the loss).
//!
//! Replay is publishing. Every replayed sample rides a declared publisher
//! ([`crate::write::declare_publication`], P7 — no ad-hoc puts), gets the
//! *replaying* session's HLC (re-stamped deliberately: a preserved foreign
//! HLC silently loses every RFC 04 §3.2 reconciliation), and a recorded
//! delete passes the same class-conscious retire gate as a live one
//! ([`crate::write::check_retire`], RFC 04 §1.2 v1.12). The etiquette the
//! CLI enforces on top — dry-run first, header-base refusal without an
//! explicit override — is RFC 09 §5.2's.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use zenoh::Session;
use zenoh::sample::SampleKind;

use crate::ingest::{IngestRow, parse_row};
use crate::registry::SliceSet;
use crate::sub::{EventStream, FleetEvent, SampleView, StreamItem};

/// The current `.zrec` format version, written into every header.
pub const ZREC_VERSION: u32 = 1;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// RFC 3339 UTC "now", seconds precision — the header's provenance stamp.
/// A hand-rolled civil-date conversion (Hinnant's days algorithm) beats a
/// clock crate this crate needs for nothing else.
pub fn rfc3339_now() -> String {
    rfc3339_from_unix(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

fn rfc3339_from_unix(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil from days since 1970-01-01 (era-based, valid far past 2100).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// The first line of a `.zrec` file: what was asked, under which base, and
/// when (RFC 09 §5.1 O4 — a capture names its question). The `base` is the
/// operator's *stated* deployment base at capture time; recorded keys are
/// full wire keys and are never re-derived from it (O3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZrecHeader {
    /// Format version ([`ZREC_VERSION`]).
    pub zrec: u32,
    /// The full wire selectors the capture watched. A wildcard selector
    /// never crosses an `@`-chunk, so a `**` capture excludes the verbatim
    /// planes by construction (O5) — the reader states that rather than
    /// letting the file claim "everything".
    pub selectors: Vec<String>,
    /// The deployment base the operator resolved at capture time
    /// (may be empty: the base-less bus-root deployment).
    pub base: String,
    /// Capture start, RFC 3339 wall clock — provenance, not a pacing clock
    /// (pacing rides each row's `t`).
    pub captured_at: String,
}

/// A `.zrec` writer over any byte sink: header first, then rows as they
/// arrive, drop records in place. Wrap the sink in a `BufWriter` — the
/// writer emits line-at-a-time and never buffers samples itself, so a
/// capture streams in bounded memory.
pub struct ZrecWriter<W: Write> {
    out: W,
    /// The capture epoch on this observer's monotonic clock: every row's
    /// `t` is an offset from here. Stamped at construction, so a sample
    /// received before the writer existed saturates to 0 rather than
    /// underflowing.
    epoch: Instant,
    samples: u64,
    dropped: u64,
}

impl<W: Write> ZrecWriter<W> {
    /// Write the header line and hand back a row writer.
    pub fn new(mut out: W, header: &ZrecHeader) -> Result<Self> {
        serde_json::to_writer(&mut out, header).context("write .zrec header")?;
        out.write_all(b"\n").context("write .zrec header")?;
        Ok(ZrecWriter {
            out,
            epoch: Instant::now(),
            samples: 0,
            dropped: 0,
        })
    }

    /// Write one observed sample as a row.
    ///
    /// The payload rides lossless (`"bytes"`), the pacing offset is the
    /// observer's arrival clock (`"t"`, µs since the capture epoch), and
    /// the publisher's HLC — when one rode the sample — is carried
    /// informatively (`"timestamp"`): replay re-stamps (RFC 09 §5.2). QoS
    /// is stored as a profile *name* only when the wire's actual axes match
    /// one (RFC 04 §3); axes matching no profile are not approximated.
    pub fn write_sample(&mut self, view: &SampleView) -> Result<()> {
        let t_us = u64::try_from(
            view.received
                .saturating_duration_since(self.epoch)
                .as_micros(),
        )
        .unwrap_or(u64::MAX);
        let mut obj = serde_json::json!({
            "key": view.key,
            "t": t_us,
        });
        if view.kind == SampleKind::Delete {
            obj["delete"] = true.into();
        } else {
            obj["bytes"] = b64(&view.payload.to_bytes()).into();
        }
        if !view.encoding.is_empty() {
            obj["encoding"] = view.encoding.clone().into();
        }
        if let Some(t) = view.timestamp {
            obj["timestamp"] = t.to_string().into();
        }
        if let Some(profile) = zenkey::qos::QosProfile::ALL
            .into_iter()
            .find(|p| view.qos_matches(*p))
        {
            obj["qos"] = profile.name().into();
        }
        if let Some(a) = &view.attachment {
            obj["attachment_b64"] = b64(&a.to_bytes()).into();
        }
        serde_json::to_writer(&mut self.out, &obj).context("write .zrec row")?;
        self.out.write_all(b"\n").context("write .zrec row")?;
        self.samples += 1;
        Ok(())
    }

    /// Write a drop record where the gap happened (O6 on a file).
    pub fn write_dropped(&mut self, n: u64) -> Result<()> {
        serde_json::to_writer(&mut self.out, &serde_json::json!({ "dropped": n }))
            .context("write .zrec drop record")?;
        self.out
            .write_all(b"\n")
            .context("write .zrec drop record")?;
        self.dropped += n;
        Ok(())
    }

    /// Samples and drops written so far — the progress line's numbers.
    pub fn counts(&self) -> (u64, u64) {
        (self.samples, self.dropped)
    }

    /// Flush and hand the sink back.
    pub fn finish(mut self) -> Result<W> {
        self.out.flush().context("flush .zrec")?;
        Ok(self.out)
    }
}

/// Bounds on a capture. Unset bounds mean "until the caller stops the
/// loop" (Ctrl-C is the caller's `select!`, not this module's business —
/// [`record()`](record) is cancel-safe between lines).
#[derive(Debug, Clone, Copy, Default)]
pub struct RecordBounds {
    /// Stop after this many samples (drop records do not count).
    pub max_samples: Option<u64>,
    /// Stop after this long, measured from entering [`record()`](record).
    pub max_duration: Option<Duration>,
}

/// Drain a monitor's event stream into a `.zrec` writer until a bound is
/// hit or the stream ends. Samples and interleaved drops are recorded;
/// liveliness and tick events are not part of the format. `on_progress` is
/// called after every written line with (samples, dropped) — throttle in
/// the callback, not here.
///
/// Cancel-safe: dropping the future mid-`recv` loses nothing already
/// written (each line lands whole); call [`ZrecWriter::finish`] afterwards
/// to flush.
pub async fn record<W: Write>(
    events: &mut EventStream,
    writer: &mut ZrecWriter<W>,
    bounds: RecordBounds,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    let deadline = bounds.max_duration.map(|d| Instant::now() + d);
    loop {
        let (samples, _) = writer.counts();
        if bounds.max_samples.is_some_and(|max| samples >= max) {
            return Ok(());
        }
        let item = match deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Ok(());
                }
                match tokio::time::timeout(left, events.recv()).await {
                    Ok(item) => item,
                    Err(_) => return Ok(()),
                }
            }
            None => events.recv().await,
        };
        match item {
            Some(StreamItem::Event(FleetEvent::Sample(view))) => {
                writer.write_sample(&view)?;
            }
            Some(StreamItem::Dropped(n)) => {
                writer.write_dropped(n)?;
            }
            Some(_) => continue,
            None => return Ok(()),
        }
        let (samples, dropped) = writer.counts();
        on_progress(samples, dropped);
    }
}

/// What a capture did — the shared report shape both frontends render.
#[derive(Debug, Clone, Serialize)]
pub struct RecordReport {
    /// The header as written: a capture names its question (O4).
    pub header: ZrecHeader,
    /// Where the capture went, when it went to a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// Samples written.
    pub samples: u64,
    /// Samples the capture missed while behind — stored in the file as
    /// interleaved drop records *and* totalled here (O6).
    pub dropped: u64,
    /// Wall-clock capture length.
    pub duration_ms: u64,
}

/// One `.zrec` line after the header.
#[derive(Debug, Clone)]
pub enum ZrecItem {
    /// A publishable row, its pacing offset (absent on a hand-piped ndjson
    /// row — replay treats that as "no delay"), and the capture-time
    /// publisher HLC, informative only.
    Sample {
        row: IngestRow,
        t_us: Option<u64>,
        timestamp: Option<String>,
    },
    /// Samples the capture itself missed at this position (O6).
    Dropped(u64),
}

/// A `.zrec` reader over any buffered byte source: header up front, then
/// one item per line — bounded memory, like the writer.
pub struct ZrecReader<R: BufRead> {
    header: ZrecHeader,
    lines: std::io::Lines<R>,
    /// 1-based number of the last line handed out (the header is line 1).
    line: u64,
}

impl<R: BufRead> ZrecReader<R> {
    /// Parse the header line. A file without one is not a `.zrec` — plain
    /// ndjson pipes replay through `topic pub --from ndjson`, which needs
    /// no base contract because the operator is the pacing.
    pub fn new(source: R) -> Result<Self> {
        let mut lines = source.lines();
        let first = lines
            .next()
            .ok_or_else(|| anyhow!("empty file — not a .zrec (no header line)"))?
            .context("read .zrec header")?;
        let header: ZrecHeader = serde_json::from_str(&first)
            .map_err(|e| anyhow!("line 1 is not a .zrec header: {e}"))?;
        if header.zrec != ZREC_VERSION {
            bail!(
                "unsupported .zrec version {} (this reader speaks {})",
                header.zrec,
                ZREC_VERSION
            );
        }
        Ok(ZrecReader {
            header,
            lines,
            line: 1,
        })
    }

    pub fn header(&self) -> &ZrecHeader {
        &self.header
    }

    /// The next item, or `Err` naming the line and the reason — a malformed
    /// row is counted by the caller, never silently skipped
    /// ([`crate::ingest`]'s rule). `None` ends the file.
    #[allow(clippy::should_implement_trait)] // fallible, line-numbered next
    pub fn next(&mut self) -> Option<std::result::Result<ZrecItem, String>> {
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
            // A drop record is `{"dropped": n}` — no key, not a row.
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line)
                && v.get("key").is_none()
                && let Some(n) = v.get("dropped").and_then(serde_json::Value::as_u64)
            {
                return Some(Ok(ZrecItem::Dropped(n)));
            }
            return Some(match parse_row(&line) {
                Ok(row) => {
                    let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
                    Ok(ZrecItem::Sample {
                        row,
                        t_us: v.get("t").and_then(serde_json::Value::as_u64),
                        timestamp: v
                            .get("timestamp")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    })
                }
                Err(e) => Err(format!("line {}: {e}", self.line)),
            });
        }
    }
}

/// Where a replay's writes go.
pub enum ReplayTarget<'a> {
    /// No session at all: list what would be published, publish nothing.
    /// The zero-puts guarantee is structural — there is nothing to put on.
    DryRun,
    /// Real puts through declared publishers on this session.
    Bus {
        session: &'a Session,
        /// Registry slices for the retire gate's `ttl_s` awareness; `None`
        /// classifies from the grammar alone.
        slices: Option<&'a SliceSet>,
    },
}

/// Replay events, surfaced as they happen so a frontend can render them —
/// the report at the end carries the counts.
#[derive(Debug, Clone)]
pub enum ReplayEvent<'a> {
    /// Dry run: this row would publish.
    WouldPut {
        key: &'a str,
        bytes: usize,
        encoding: Option<&'a str>,
    },
    /// Dry run: this row would tombstone.
    WouldRetire { key: &'a str },
    /// A row that could not be parsed — counted, never skipped.
    Malformed { reason: String },
    /// A delete row the retire gate refused (RFC 04 §1.2 v1.12).
    Refused { key: String, reason: String },
    /// The capture itself missed this many samples here (O6): the replay
    /// is a partial view of a partial view, and both halves are counted.
    CaptureDropped(u64),
}

/// What a replay did — the shared report shape both frontends render.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayReport {
    /// The capture header, echoed: a replay names what it replayed.
    pub header: ZrecHeader,
    pub dry_run: bool,
    pub speed: f64,
    /// Rows published (dry run: rows that would have been).
    pub published: u64,
    /// Tombstones sent (dry run: would have been).
    pub tombstones: u64,
    /// Rows that could not be parsed — counted, never skipped.
    pub malformed: u64,
    /// Delete rows the retire gate refused.
    pub refused: u64,
    /// Samples the *capture* missed (summed from the file's drop records):
    /// this replay is a partial view and says so (O6).
    pub capture_dropped: u64,
    /// The first few malformed/refused reasons, for the human render.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub first_errors: Vec<String>,
}

/// Replay a `.zrec` onto a bus — or list what doing so would publish.
///
/// Pacing follows each row's `t` divided by `speed` (must be positive);
/// a dry run lists instantly, because a preview that takes the capture's
/// duration is a preview nobody runs. Delete rows pass
/// [`crate::write::check_retire`] under the **header's** base — the keys
/// were captured under it, and classifying them under anything else would
/// re-derive what O3 says must not be re-derived; `i_know` is the operator
/// saying the off-state cleanup is meant. Publishers are declared once per
/// distinct key and undeclared at the end.
pub async fn replay<R: BufRead>(
    reader: &mut ZrecReader<R>,
    target: ReplayTarget<'_>,
    speed: f64,
    i_know: bool,
    default_qos: &str,
    mut on_event: impl FnMut(ReplayEvent<'_>),
) -> Result<ReplayReport> {
    if !(speed.is_finite() && speed > 0.0) {
        bail!("--speed must be a positive number (got {speed})");
    }
    let base = reader.header().base.clone();
    let mut report = ReplayReport {
        header: reader.header().clone(),
        dry_run: matches!(target, ReplayTarget::DryRun),
        speed,
        published: 0,
        tombstones: 0,
        malformed: 0,
        refused: 0,
        capture_dropped: 0,
        first_errors: Vec::new(),
    };
    let record_err = |report: &mut ReplayReport, reason: String, refused: bool| {
        if refused {
            report.refused += 1;
        } else {
            report.malformed += 1;
        }
        if report.first_errors.len() < 3 {
            report.first_errors.push(reason);
        }
    };
    let mut publications: HashMap<String, crate::write::Publication> = HashMap::new();
    let mut prev_t: Option<u64> = None;
    while let Some(item) = reader.next() {
        let (row, t_us) = match item {
            Ok(ZrecItem::Sample { row, t_us, .. }) => (row, t_us),
            Ok(ZrecItem::Dropped(n)) => {
                report.capture_dropped += n;
                on_event(ReplayEvent::CaptureDropped(n));
                continue;
            }
            Err(reason) => {
                on_event(ReplayEvent::Malformed {
                    reason: reason.clone(),
                });
                record_err(&mut report, reason, false);
                continue;
            }
        };
        let slices = match &target {
            ReplayTarget::Bus { slices, .. } => *slices,
            ReplayTarget::DryRun => None,
        };
        if row.delete
            && let Err(e) = crate::write::check_retire(&base, &row.key, slices, i_know)
        {
            let reason = e.to_string();
            on_event(ReplayEvent::Refused {
                key: row.key.clone(),
                reason: reason.clone(),
            });
            record_err(&mut report, format!("{}: {reason}", row.key), true);
            continue;
        }
        match &target {
            ReplayTarget::DryRun => {
                if row.delete {
                    on_event(ReplayEvent::WouldRetire { key: &row.key });
                    report.tombstones += 1;
                } else {
                    on_event(ReplayEvent::WouldPut {
                        key: &row.key,
                        bytes: row.payload.len(),
                        encoding: row.encoding.as_deref(),
                    });
                    report.published += 1;
                }
            }
            ReplayTarget::Bus { session, .. } => {
                // Original pacing, scaled — the observer's arrival clock is
                // the only clock a capture has for "when" (RFC 09 §5.2).
                if let (Some(prev), Some(t)) = (prev_t, t_us)
                    && t > prev
                {
                    let delay = Duration::from_micros(t - prev).div_f64(speed);
                    tokio::time::sleep(delay).await;
                }
                if t_us.is_some() {
                    prev_t = t_us;
                }
                let publication = match publications.entry(row.key.clone()) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let qos_name = row.qos.as_deref().unwrap_or(default_qos);
                        let Some(qos) = zenkey::qos::QosProfile::from_name(qos_name) else {
                            let reason = format!("unknown QoS profile {qos_name:?}");
                            on_event(ReplayEvent::Malformed {
                                reason: reason.clone(),
                            });
                            record_err(&mut report, reason, false);
                            continue;
                        };
                        let publication = crate::write::declare_publication(
                            session,
                            &row.key,
                            qos,
                            row.encoding.as_deref(),
                        )
                        .await?;
                        e.insert(publication)
                    }
                };
                if row.delete {
                    publication.retire().await?;
                    report.tombstones += 1;
                } else {
                    publication.send(row.payload, row.attachment).await?;
                    report.published += 1;
                }
            }
        }
    }
    for (_, publication) in publications.drain() {
        publication.undeclare().await?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The provenance stamp is a real RFC 3339 instant, leap-era safe.
    #[test]
    fn the_wall_clock_formats_correctly() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_unix(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339_from_unix(1_786_492_800), "2026-08-12T00:00:00Z");
        assert!(!rfc3339_now().is_empty());
    }

    fn header() -> ZrecHeader {
        ZrecHeader {
            zrec: ZREC_VERSION,
            selectors: vec!["v1/**".into()],
            base: String::new(),
            captured_at: "2026-08-12T00:00:00Z".into(),
        }
    }

    /// The header round-trips, and a versioned reader refuses what it
    /// cannot speak rather than guessing.
    #[test]
    fn the_header_is_a_contract() {
        let mut sink = Vec::new();
        let writer = ZrecWriter::new(&mut sink, &header()).unwrap();
        let _ = writer.finish().unwrap();
        let reader = ZrecReader::new(sink.as_slice()).unwrap();
        assert_eq!(reader.header(), &header());

        let future = r#"{"zrec":99,"selectors":[],"base":"","captured_at":"x"}"#;
        let err = ZrecReader::new(future.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("version 99"), "{err}");

        let not_zrec = r#"{"key":"v1/x","value":1}"#;
        let err = ZrecReader::new(not_zrec.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("header"), "{err}");
    }

    /// Drop records read back as drops, at their position (O6): the gap is
    /// part of the record, not a footnote.
    #[test]
    fn drops_are_interleaved_facts() {
        let body = format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::to_string(&header()).unwrap(),
            r#"{"key":"v1/h/state/p/a","t":0,"bytes":"AQ=="}"#,
            r#"{"dropped":7}"#,
            r#"{"key":"v1/h/state/p/a","t":1000,"bytes":"Ag=="}"#,
        );
        let mut reader = ZrecReader::new(body.as_bytes()).unwrap();
        assert!(matches!(reader.next(), Some(Ok(ZrecItem::Sample { .. }))));
        assert!(matches!(reader.next(), Some(Ok(ZrecItem::Dropped(7)))));
        assert!(matches!(
            reader.next(),
            Some(Ok(ZrecItem::Sample {
                t_us: Some(1000),
                ..
            }))
        ));
        assert!(reader.next().is_none());
    }

    /// A malformed line is an error naming its line number — counted by
    /// the caller, never a skip.
    #[test]
    fn malformed_lines_are_named_not_skipped() {
        let body = format!(
            "{}\nnot json\n{}\n",
            serde_json::to_string(&header()).unwrap(),
            r#"{"key":"v1/h/state/p/a","t":0,"bytes":"AQ=="}"#,
        );
        let mut reader = ZrecReader::new(body.as_bytes()).unwrap();
        let err = match reader.next() {
            Some(Err(e)) => e,
            other => panic!("expected a named error, got {other:?}"),
        };
        assert!(err.starts_with("line 2:"), "{err}");
        assert!(matches!(reader.next(), Some(Ok(ZrecItem::Sample { .. }))));
    }

    /// A dry run performs zero puts by construction — there is no session —
    /// and still counts and classifies every row.
    #[tokio::test]
    async fn a_dry_run_lists_and_publishes_nothing() {
        let body = format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::to_string(&header()).unwrap(),
            r#"{"key":"v1/h-0123456789ab/state/p/health","t":0,"bytes":"eyJvayI6dHJ1ZX0=","encoding":"application/json"}"#,
            r#"{"dropped":3}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/health","t":500000,"delete":true}"#,
        );
        let mut reader = ZrecReader::new(body.as_bytes()).unwrap();
        let mut would = Vec::new();
        let report = replay(
            &mut reader,
            ReplayTarget::DryRun,
            1.0,
            false,
            "refreshed",
            |ev| {
                would.push(format!("{ev:?}"));
            },
        )
        .await
        .unwrap();
        assert!(report.dry_run);
        assert_eq!(report.published, 1);
        assert_eq!(report.tombstones, 1); // state-shaped: licensed without force
        assert_eq!(report.capture_dropped, 3);
        assert_eq!(report.malformed, 0);
        assert_eq!(would.len(), 3, "{would:?}");
    }

    /// A recorded delete off the state class keeps its price on replay
    /// (RFC 04 §1.2 v1.12): refused without `i_know`, counted.
    #[tokio::test]
    async fn replayed_tombstones_pass_the_retire_gate() {
        let body = format!(
            "{}\n{}\n",
            serde_json::to_string(&header()).unwrap(),
            r#"{"key":"v1/h-0123456789ab/telemetry/p/temp","t":0,"delete":true}"#,
        );
        let mut reader = ZrecReader::new(body.as_bytes()).unwrap();
        let report = replay(
            &mut reader,
            ReplayTarget::DryRun,
            1.0,
            false,
            "refreshed",
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(report.refused, 1);
        assert_eq!(report.tombstones, 0);
        assert!(
            report.first_errors[0].contains("telemetry"),
            "{:?}",
            report.first_errors
        );
    }

    /// Speed is a positive finite scale, stated rather than clamped.
    #[tokio::test]
    async fn speed_must_be_positive() {
        let body = serde_json::to_string(&header()).unwrap() + "\n";
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut reader = ZrecReader::new(body.as_bytes()).unwrap();
            let err = replay(
                &mut reader,
                ReplayTarget::DryRun,
                bad,
                false,
                "refreshed",
                |_| {},
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(err.contains("speed"), "{err}");
        }
    }
}
