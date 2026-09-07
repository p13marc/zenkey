//! `.zrec` capture and replay (issue #39; RFC 09 §5.2 documents the
//! etiquette, this module is normative for the format).
//!
//! A `.zrec` file is newline-delimited JSON in the explorers' one row
//! dialect — the same shape `echo --format ndjson` emits and
//! [`crate::tape::ingest::parse_row`] reads back — upgraded with what a pipe does
//! not need but a capture does: a versioned header line naming what was
//! asked, a lossless `"bytes"` payload (a `"value"` is a rendering), a
//! pacing offset `"t"` on the observer's arrival clock, and drop records
//! interleaved **where the gap happened** (RFC 09 §5.1 O6, applied to a
//! file — a capture taken while behind is a partial view and says so at
//! the position of the loss).
//!
//! **Neither direction touches the disk from a runtime thread** (#332).
//! [`ZrecWriter`] and [`ZrecReader`] are plain synchronous `std::io` — a
//! capture is line-at-a-time base64 and JSON, which is CPU as well as I/O —
//! and the async ends of the module, [`ZrecSink`] and [`ZrecSource`], run
//! them on the blocking pool behind a bounded channel. The alternative,
//! `AsyncWrite`/`AsyncBufRead` bounds on the two types, was rejected: the
//! serialization would still run on a runtime worker, and the two callers
//! that write a `.zrec` **without** a runtime at all — zengui's "save the
//! retained window", this module's tests — would need a second, synchronous
//! writer to stay honest. `bus/blob/transfer.rs`'s `tokio::fs` is the model
//! for byte-shovelling; this is the model for a serialising loop.
//!
//! It matters because the thing being recorded is the thing the writer
//! stalls: a blocking `write_all` per sample on the drain's own task left the
//! monitor's bounded broadcast unattended, and `zenctl record` faithfully
//! wrote `{"dropped": n}` records it had caused itself.
//!
//! Replay is publishing. Every replayed sample rides a declared publisher
//! ([`crate::bus::write::declare_publication`], P7 — no ad-hoc puts), gets the
//! *replaying* session's HLC (re-stamped deliberately: a preserved foreign
//! HLC silently loses every RFC 04 §3.2 reconciliation), and a recorded
//! delete passes the same class-conscious retire gate as a live one
//! ([`crate::bus::write::check_retire`], RFC 04 §1.2 v1.12). The etiquette the
//! CLI enforces on top — dry-run first, header-base refusal without an
//! explicit override — is RFC 09 §5.2's.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::{Error, Result};
use zenkey::qos::QosProfile;
use zenoh::Session;
use zenoh::sample::SampleKind;

use crate::bus::monitor::{EventStream, FleetEvent, SampleView, StreamItem};
use crate::model::registry::SliceSet;
use crate::report::{ReplayReport, SampleRow, Transition, ZrecHeader};
use crate::tape::ingest::{IngestRow, parse_row};

/// The current `.zrec` format version, written into every header.
///
/// Version 2 (RFC 13 §4.1, v1.34; #218) adds the state preamble — rows
/// marked `"preamble": true` at `t: 0` ahead of the first observed row —
/// and the interleaved `{"trigger": …}` record. A version-1 file is a
/// version-2 file with neither, which is why [`ZREC_READS`] names both.
pub const ZREC_VERSION: u32 = 2;

/// The versions [`ZrecReader`] speaks: the current one and every earlier
/// one whose lines it still reads verbatim. A version outside this list is
/// refused, never guessed at (RFC 13 §4.1's unknown-version rule) — and
/// the list is what lets a version-2 reader read version 1 while a
/// version-1 reader refuses version 2, both by the rule they already had.
pub const ZREC_READS: [u32; 2] = [1, 2];

/// Why a replayer skips a preamble row unless told otherwise — one sentence,
/// spelled once, carried on every [`ReplayEvent::PreambleSkipped`].
pub const PREAMBLE_SKIP_REASON: &str = "state at capture start; re-stamping it republishes a \
     snapshot over live state (RFC 13 §4.2) — pass --seed-state to mean it";

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

/// RFC 3339 (`YYYY-MM-DDTHH:MM:SSZ`) of a Unix second count — the one
/// formatter every timestamp this engine writes goes through.
pub fn rfc3339_from_unix(secs: u64) -> String {
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
    counts: SinkCounts,
}

/// What a writer or sink has put on the file so far, by kind — and the
/// kinds are never folded (RFC 13 §4.1: a reader counts preamble rows
/// apart from observed rows, so the writer does too).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SinkCounts {
    /// Observed sample rows.
    pub samples: u64,
    /// Samples the capture missed, summed from the drop records it wrote.
    pub dropped: u64,
    /// Preamble rows (version 2).
    pub preamble: u64,
    /// Trigger records (version 2).
    pub triggers: u64,
}

impl<W: Write> ZrecWriter<W> {
    /// Write the header line and hand back a row writer.
    pub fn new(out: W, header: &ZrecHeader) -> Result<Self> {
        ZrecWriter::new_at(out, header, Instant::now())
    }

    /// [`ZrecWriter::new`] with the capture epoch injected (#217).
    ///
    /// A live capture's epoch is "now" — nothing precedes the writer. A
    /// **retained window** is the opposite: every sample was received before
    /// the writer existed, and under `new` they would all saturate to `t: 0`,
    /// erasing the pacing the ring preserved. Passing the window's own start
    /// (its oldest sample's arrival) keeps each row's `t` the offset it
    /// really had, so the file is indistinguishable from one recorded
    /// deliberately at that moment.
    pub fn new_at(mut out: W, header: &ZrecHeader, epoch: Instant) -> Result<Self> {
        serde_json::to_writer(&mut out, header).map_err(|e| Error::Io {
            path: std::path::PathBuf::new(),
            source: e.into(),
        })?;
        out.write_all(b"\n").map_err(|e| Error::Io {
            path: std::path::PathBuf::new(),
            source: e,
        })?;
        Ok(ZrecWriter {
            out,
            epoch,
            counts: SinkCounts::default(),
        })
    }

    fn line(&mut self, line: &str) -> Result<()> {
        self.out.write_all(line.as_bytes()).map_err(|e| Error::Io {
            path: std::path::PathBuf::new(),
            source: e,
        })?;
        self.out.write_all(b"\n").map_err(|e| Error::Io {
            path: std::path::PathBuf::new(),
            source: e,
        })
    }

    /// The row a sample becomes, minus its pacing: the wire facts, the
    /// lossless payload unless it is a tombstone, the attachment.
    fn row_of(view: &SampleView) -> SampleRow {
        let mut row = SampleRow {
            key: view.key.clone(),
            ..SampleRow::default()
        }
        .with_wire(view);
        // A tombstone has no payload to store: `delete` is the whole fact
        // (RFC 04 §1.2), and an empty `bytes` would read as an empty put.
        if view.kind != SampleKind::Delete {
            row = row.with_payload_bytes(&view.payload.to_bytes());
        }
        if let Some(a) = &view.attachment {
            row.attachment_b64 = Some(crate::tape::ingest::b64(&a.to_bytes()));
        }
        row
    }

    /// Write one observed sample as a row.
    ///
    /// The payload rides lossless (`"bytes"`), the pacing offset is the
    /// observer's arrival clock (`"t"`, µs since the capture epoch), and
    /// the publisher's HLC — when one rode the sample — is carried
    /// informatively (`"timestamp"`): replay re-stamps (RFC 09 §5.2). QoS
    /// is stored as a profile *name* only when the wire's actual axes match
    /// one (RFC 04 §3); axes matching no profile are not approximated —
    /// a rule [`SampleRow::with_wire`] now enforces for every writer of the
    /// dialect rather than for this one alone (#235).
    ///
    /// A capture carries **no** `origin`/`subject`: those are the observer's
    /// reading of the key under a base it chose, and a file that outlives
    /// the session must not freeze one deployment's interpretation into
    /// somebody else's replay (RFC 09 §5.2 — keys are recorded whole and
    /// never re-derived from the header's base).
    pub fn write_sample(&mut self, view: &SampleView) -> Result<()> {
        let t_us = u64::try_from(
            view.received
                .saturating_duration_since(self.epoch)
                .as_micros(),
        )
        .unwrap_or(u64::MAX);
        let mut row = ZrecWriter::<W>::row_of(view);
        row.t = Some(t_us);
        self.line(&row.to_line())?;
        self.counts.samples += 1;
        Ok(())
    }

    /// Write one **preamble** row (version 2, RFC 13 §4.1; #218): a value
    /// fetched at trigger time, written ahead of the first observed row.
    ///
    /// `t` is 0 — the row precedes the window, and a preamble is state, not
    /// pacing — and `"preamble": true` marks it so a reader counts it apart
    /// from observed rows (O6 applied to rows). The `timestamp` is the HLC
    /// the fetched value carried, kept as provenance: it says when the
    /// value was published, which is the one thing a pre-roll of `state`
    /// deltas cannot say for itself.
    pub fn write_preamble(&mut self, view: &SampleView) -> Result<()> {
        let mut row = ZrecWriter::<W>::row_of(view);
        row.t = Some(0);
        row.preamble = Some(true);
        self.line(&row.to_line())?;
        self.counts.preamble += 1;
        Ok(())
    }

    /// Write a drop record where the gap happened (O6 on a file).
    pub fn write_dropped(&mut self, n: u64) -> Result<()> {
        let line =
            serde_json::to_string(&serde_json::json!({ "dropped": n })).map_err(|e| Error::Io {
                path: std::path::PathBuf::new(),
                source: e.into(),
            })?;
        self.line(&line)?;
        self.counts.dropped += n;
        Ok(())
    }

    /// Write the trigger record where the transition was observed (version
    /// 2, RFC 13 §4.1): `{"trigger": {rule, from, to, at, evidence}}` — no
    /// `key`, like a drop record — so a reader can say what fired and where
    /// in the file it did.
    pub fn write_trigger(&mut self, transition: &Transition) -> Result<()> {
        let line =
            serde_json::to_string(&serde_json::json!({ "trigger": transition })).map_err(|e| {
                Error::Io {
                    path: std::path::PathBuf::new(),
                    source: e.into(),
                }
            })?;
        self.line(&line)?;
        self.counts.triggers += 1;
        Ok(())
    }

    /// What has been written so far, by kind — the progress line's numbers.
    pub fn counts(&self) -> SinkCounts {
        self.counts
    }

    /// Flush and hand the sink back.
    pub fn finish(mut self) -> Result<W> {
        self.out.flush().map_err(|e| Error::Io {
            path: std::path::PathBuf::new(),
            source: e,
        })?;
        Ok(self.out)
    }
}

/// How many lines a [`ZrecSink`] queues ahead of its writer.
///
/// Four times the monitor's default broadcast capacity (1024), on purpose:
/// a burst the *bus* side can hold is a burst the disk side can hold too, so
/// a momentary write stall spends the queue instead of manufacturing drops
/// the bus never had. Past that the queue backpressures the drain — which is
/// where a genuinely-too-slow disk belongs, surfacing as `Dropped(n)` like
/// any other observer that could not keep up (RFC 13 §3 O6). Bounded, not
/// unbounded, because a capture promises to stream in bounded memory.
const SINK_QUEUE: usize = 4096;

/// One line on its way to the disk. Serialization happens on the writer's
/// thread, so what crosses the channel is the sample itself — a pointer
/// move, not a copy.
enum ZrecLine {
    Sample(Arc<SampleView>),
    Dropped(u64),
    Preamble(Arc<SampleView>),
    Trigger(Box<Transition>),
}

/// What the sink's async half can see of a writer that lives on the
/// blocking pool.
#[derive(Debug, Default)]
struct SinkState {
    samples: AtomicU64,
    dropped: AtomicU64,
    preamble: AtomicU64,
    triggers: AtomicU64,
    /// The writer's error, kept where the async half can name it: a `send`
    /// that fails says only "the writer is gone", and the reason is what the
    /// operator needs.
    failure: std::sync::Mutex<Option<String>>,
}

/// A [`ZrecWriter`] running on the blocking pool behind a bounded channel
/// (#332) — the async end of a capture.
///
/// Every byte of `.zrec` I/O, and every base64 and JSON encode that precedes
/// it, happens on a blocking thread. The runtime side of a capture does
/// nothing but move `Arc`s into a queue, so the drain stays available to the
/// monitor's bounded broadcast and the drop records in the file mean what
/// they say: samples the *bus* outran the observer with, not samples the
/// observer's own writer stalled it out of.
pub struct ZrecSink {
    tx: tokio::sync::mpsc::Sender<ZrecLine>,
    state: Arc<SinkState>,
    writer: tokio::task::JoinHandle<Result<SinkCounts>>,
}

impl ZrecSink {
    /// Write `header` and hand back the sink. Awaits the header's own write,
    /// so a sink that comes back is a file with a valid first line on it —
    /// the failure an operator must hear about before a capture starts
    /// "running".
    ///
    /// The capture epoch is stamped **here** rather than on the blocking
    /// thread: every row's `t` is an offset from it, and it must not shift by
    /// however long the pool took to pick the task up.
    pub async fn spawn<W: Write + Send + 'static>(out: W, header: &ZrecHeader) -> Result<ZrecSink> {
        ZrecSink::spawn_at(out, header, Instant::now()).await
    }

    /// [`ZrecSink::spawn`] with the capture epoch injected — the streaming
    /// twin of [`ZrecWriter::new_at`], and the seam a test uses to write
    /// deterministic offsets.
    pub async fn spawn_at<W: Write + Send + 'static>(
        out: W,
        header: &ZrecHeader,
        epoch: Instant,
    ) -> Result<ZrecSink> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(SINK_QUEUE);
        let (ready, opened) = tokio::sync::oneshot::channel();
        let state = Arc::new(SinkState::default());
        let header = header.clone();
        let task_state = Arc::clone(&state);
        let writer = tokio::task::spawn_blocking(move || {
            let mut writer = match ZrecWriter::new_at(out, &header, epoch) {
                Ok(w) => {
                    let _ = ready.send(None);
                    w
                }
                Err(e) => {
                    let _ = ready.send(Some(crate::one_line(&e)));
                    return Err(e);
                }
            };
            while let Some(line) = rx.blocking_recv() {
                let wrote = match line {
                    ZrecLine::Sample(view) => writer.write_sample(&view),
                    ZrecLine::Dropped(n) => writer.write_dropped(n),
                    ZrecLine::Preamble(view) => writer.write_preamble(&view),
                    ZrecLine::Trigger(t) => writer.write_trigger(&t),
                };
                if let Err(e) = wrote {
                    *task_state.failure.lock().expect("sink failure lock") =
                        Some(crate::one_line(&e));
                    return Err(e);
                }
            }
            let counts = writer.counts();
            writer.finish().map(|_| counts)
        });
        match opened.await {
            Ok(None) => Ok(ZrecSink { tx, state, writer }),
            // The writer names the file in its own message; this is the
            // open failing, which is I/O against a path the caller gave.
            Ok(Some(reason)) => Err(Error::Io {
                path: std::path::PathBuf::new(),
                source: std::io::Error::other(reason),
            }),
            Err(_) => Err(Error::Internal(
                "the .zrec writer stopped before it opened".into(),
            )),
        }
    }

    /// Queue one sample. Awaits only the queue's capacity — never the disk.
    pub async fn write_sample(&self, view: Arc<SampleView>) -> Result<()> {
        self.send(ZrecLine::Sample(view)).await?;
        self.state.samples.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Queue a drop record at the position the gap happened (O6 on a file).
    pub async fn write_dropped(&self, n: u64) -> Result<()> {
        self.send(ZrecLine::Dropped(n)).await?;
        self.state.dropped.fetch_add(n, Ordering::Relaxed);
        Ok(())
    }

    /// Queue one preamble row ([`ZrecWriter::write_preamble`]).
    pub async fn write_preamble(&self, view: Arc<SampleView>) -> Result<()> {
        self.send(ZrecLine::Preamble(view)).await?;
        self.state.preamble.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Queue the trigger record at this position
    /// ([`ZrecWriter::write_trigger`]).
    pub async fn write_trigger(&self, transition: Transition) -> Result<()> {
        self.send(ZrecLine::Trigger(Box::new(transition))).await?;
        self.state.triggers.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn send(&self, line: ZrecLine) -> Result<()> {
        if self.tx.send(line).await.is_ok() {
            return Ok(());
        }
        // The writer is gone, which only happens because it failed: report
        // *its* error rather than the channel's shadow of it.
        let failure = self
            .state
            .failure
            .lock()
            .expect("sink failure lock")
            .clone();
        Err(Error::Internal(
            failure.unwrap_or_else(|| "the .zrec writer stopped".to_string()),
        ))
    }

    /// What has been **accepted** so far, by kind — the progress line's
    /// numbers.
    ///
    /// Accepted, not yet written: the queue is what stands between the two,
    /// and [`finish`](Self::finish) drains it, so the final counts are the
    /// file's. A progress line that waited for the disk would be reporting
    /// the disk, not the capture.
    pub fn counts(&self) -> SinkCounts {
        SinkCounts {
            samples: self.state.samples.load(Ordering::Relaxed),
            dropped: self.state.dropped.load(Ordering::Relaxed),
            preamble: self.state.preamble.load(Ordering::Relaxed),
            triggers: self.state.triggers.load(Ordering::Relaxed),
        }
    }

    /// Close the queue, wait for the writer to drain it, flush, and report
    /// what reached the file, by kind.
    ///
    /// This is where a write error surfaces if the capture did not already
    /// trip over it. The counts come from the writer rather than the queue,
    /// so a report built on them is a report about the file.
    pub async fn finish(self) -> Result<SinkCounts> {
        let ZrecSink { tx, state, writer } = self;
        drop(tx);
        drop(state);
        writer
            .await
            .map_err(|e| Error::Internal(format!("the .zrec writer panicked: {e}")))?
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

/// Drain a monitor's event stream into a `.zrec` [`ZrecSink`] until a bound
/// is hit or the stream ends. Samples and interleaved drops are recorded;
/// liveliness and tick events are not part of the format. `on_progress` is
/// called after every queued line with (samples, dropped) — throttle in
/// the callback, not here.
///
/// **This loop never touches the disk** (#332): it moves `Arc`s into the
/// sink's bounded queue and goes straight back to the stream, so the
/// monitor's broadcast stays attended and a `{"dropped": n}` in the file
/// means the bus outran the observer — not that the observer's own writer
/// stalled its drain. A disk that is slower than the bus *on average* still
/// backpressures through the queue and still drops, honestly.
///
/// Cancel-safe: dropping the future mid-`recv` loses nothing already
/// queued (each line lands whole, in order); call
/// [`ZrecSink::finish`] afterwards to drain and flush.
pub async fn record(
    events: &mut EventStream,
    sink: &ZrecSink,
    bounds: RecordBounds,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<()> {
    let deadline = bounds.max_duration.map(|d| Instant::now() + d);

    loop {
        let samples = sink.counts().samples;
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
                sink.write_sample(view).await?;
            }
            Some(StreamItem::Dropped(n)) => {
                sink.write_dropped(n).await?;
            }
            Some(_) => continue,
            None => return Ok(()),
        }
        let counts = sink.counts();
        on_progress(counts.samples, counts.dropped);
    }
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
        /// The publishing entity as the row spelled it (`zid:eid#sn`),
        /// when `SourceInfo` rode the captured sample. Lifted for the
        /// timeline (#216) so a replayed window classifies its stampers
        /// exactly as the live one did; usually absent, RFC 09 §5.1 O7's
        /// practical note.
        source: Option<String>,
    },
    /// Samples the capture itself missed at this position (O6).
    Dropped(u64),
    /// A version-2 preamble row (RFC 13 §4.1; #218): state fetched at
    /// trigger time, ahead of the first observed row. `timestamp` is the
    /// HLC the fetched value carried — provenance, not pacing; the row's
    /// `t` is 0 by construction and is not repeated here. A replayer
    /// publishes these only when told to (§4.2); a pane replay seeds its
    /// fold from them.
    Preamble {
        row: IngestRow,
        timestamp: Option<String>,
    },
    /// The transition that fired a triggered capture, at the position it
    /// was observed (version 2).
    Trigger(Box<Transition>),
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
    /// ndjson pipes replay through `zenctl pub --from ndjson`, which needs
    /// no base contract because the operator is the pacing.
    pub fn new(source: R) -> Result<Self> {
        let mut lines = source.lines();
        let first = lines
            .next()
            .ok_or_else(|| Error::malformed(".zrec", "empty file — no header line"))?
            .map_err(|e| Error::Io {
                path: std::path::PathBuf::new(),
                source: e,
            })?;
        let header: ZrecHeader = serde_json::from_str(&first)
            .map_err(|e| Error::malformed_with(".zrec line 1", "is not a header", e))?;
        if !ZREC_READS.contains(&header.zrec) {
            return Err(Error::malformed(
                ".zrec",
                format!(
                    "unsupported version {} (this reader speaks {ZREC_VERSION} and reads {})",
                    header.zrec,
                    ZREC_READS
                        .iter()
                        .filter(|v| **v != ZREC_VERSION)
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
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
    /// ([`crate::tape::ingest`]'s rule). `None` ends the file.
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
            // A drop record is `{"dropped": n}` and a trigger record
            // `{"trigger": {..}}` — no key, not rows (RFC 13 §4.1).
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line)
                && v.get("key").is_none()
            {
                if let Some(n) = v.get("dropped").and_then(serde_json::Value::as_u64) {
                    return Some(Ok(ZrecItem::Dropped(n)));
                }
                if let Some(t) = v.get("trigger") {
                    return Some(
                        serde_json::from_value::<Transition>(t.clone())
                            .map(|t| ZrecItem::Trigger(Box::new(t)))
                            .map_err(|e| format!("line {}: trigger record: {e}", self.line)),
                    );
                }
            }
            return Some(match parse_row(&line) {
                Ok(row) => {
                    let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
                    let timestamp = v
                        .get("timestamp")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string);
                    if v.get("preamble").and_then(serde_json::Value::as_bool) == Some(true) {
                        Ok(ZrecItem::Preamble { row, timestamp })
                    } else {
                        Ok(ZrecItem::Sample {
                            row,
                            t_us: v.get("t").and_then(serde_json::Value::as_u64),
                            timestamp,
                            source: v
                                .get("source")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_string),
                        })
                    }
                }
                Err(e) => Err(format!("line {}: {e}", self.line)),
            });
        }
    }
}

/// A [`ZrecReader`] running on the blocking pool behind a bounded channel
/// (#332) — the async end of a replay, and the mirror of [`ZrecSink`].
///
/// [`replay`] interleaves `sleep().await`s and network puts with its reads,
/// so a blocking `BufRead` in that loop stalls the runtime on every line —
/// on a cold page cache or a network filesystem, for as long as the read
/// takes, mid-pacing. Here the file is read ahead on a blocking thread and
/// the loop awaits parsed items; the queue is bounded, so a replay that
/// pauses for pacing does not read the whole capture into memory.
pub struct ZrecSource {
    header: ZrecHeader,
    rx: tokio::sync::mpsc::Receiver<std::result::Result<ZrecItem, String>>,
}

impl ZrecSource {
    /// Parse the header, then read the rest ahead on the blocking pool.
    ///
    /// The header is awaited — a file that is not a `.zrec` is a refusal
    /// before anything is scheduled, exactly as it was when the reader was
    /// constructed inline.
    pub async fn spawn<R: BufRead + Send + 'static>(source: R) -> Result<ZrecSource> {
        let (tx, rx) = tokio::sync::mpsc::channel(SINK_QUEUE);
        let (ready, opened) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let mut reader = match ZrecReader::new(source) {
                Ok(r) => r,
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            if ready.send(Ok(reader.header().clone())).is_err() {
                return;
            }
            // A receiver that went away ends the read: a dropped replay must
            // not leave a thread reading a file nobody will look at.
            while let Some(item) = reader.next() {
                if tx.blocking_send(item).is_err() {
                    return;
                }
            }
        });
        match opened.await {
            Ok(header) => Ok(ZrecSource {
                header: header?,
                rx,
            }),
            Err(_) => Err(Error::Internal(
                "the .zrec reader stopped before it opened".into(),
            )),
        }
    }

    pub fn header(&self) -> &ZrecHeader {
        &self.header
    }

    /// The next item, or `Err` naming the line and the reason — a malformed
    /// row is counted by the caller, never silently skipped. `None` ends the
    /// file.
    pub async fn next(&mut self) -> Option<std::result::Result<ZrecItem, String>> {
        self.rx.recv().await
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

/// What one replay is.
///
/// `default_qos` is a [`QosProfile`] and not a profile *name*: the closed
/// enum is RFC 04 §3's vocabulary, and a caller that hands over a string has
/// only deferred the moment it is checked — this used to surface a bad
/// `--qos` as a per-row "malformed" event partway through a replay, rather
/// than as a refusal before anything published. A name recorded *in the
/// capture* is still a string, because a file can carry anything; that check
/// stays where it belongs, per row.
pub struct ReplaySpec<'a> {
    pub target: ReplayTarget<'a>,
    /// Pacing scale: 2.0 replays twice as fast as captured.
    pub speed: f64,
    /// Replay recorded deletes that fall off the state class — the same
    /// operator price as `zenctl retire` (RFC 04 §1.2, v1.12).
    pub i_know: bool,
    /// The profile a row that recorded none is published under.
    pub default_qos: QosProfile,
    /// Publish the version-2 preamble rows too (`--seed-state`). Off, they
    /// are skipped and counted, with the reason stated per row: re-stamped
    /// state-at-capture-start republishes a snapshot over the live fleet
    /// with no pacing between the rows (RFC 13 §4.2).
    pub seed_state: bool,
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
    /// A version-2 preamble row this replay did **not** publish, and why
    /// ([`PREAMBLE_SKIP_REASON`]) — the default without `seed_state`.
    PreambleSkipped { key: &'a str, reason: &'static str },
    /// The transition that fired the capture, at its position in the file.
    /// Never published: a marker for the operator, not a row.
    Trigger(&'a Transition),
}

/// The publishers one [`replay`] has declared, and the promise that every way
/// out of it undeclares them (#327).
///
/// The declarations used to live in a bare `HashMap`, so each of the loop's
/// `?`s returned with the whole set still declared on the bus — contradicting
/// this module's own "undeclared at the end" and the crate idiom stated at
/// [`crate::bus::query::RepeatingQuery::undeclare`]: teardown is explicit and
/// awaited, never left to `Drop`.
///
/// [`close`](Self::close) is that teardown, modelled on
/// [`crate::Monitor::shutdown`]: **every** publisher is undeclared even when
/// one fails, and the failures are reported together — a replay half torn down
/// is worse than one torn down noisily.
///
/// `Drop` is the cancellation fallback, and the one path that cannot be
/// awaited: a dropped `replay` future hands the remaining publishers to a task
/// that undeclares them properly, rather than leaving zenoh to reclaim them
/// behind everyone's back. Nothing reaches it on the normal paths — `close`
/// leaves the set empty.
#[derive(Default)]
struct Publications(HashMap<String, crate::bus::write::Publication>);

impl std::ops::Deref for Publications {
    type Target = HashMap<String, crate::bus::write::Publication>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Publications {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Publications {
    /// Undeclare every publisher, acknowledged, joining what failed.
    async fn close(mut self) -> Result<()> {
        crate::bus::teardown::drain_undeclare(self.0.drain().collect(), |p| {
            crate::bus::write::Publication::undeclare(p)
        })
        .await
    }
}

impl Drop for Publications {
    fn drop(&mut self) {
        if self.0.is_empty() {
            return;
        }
        // No runtime means nothing can be awaited at all; zenoh's own
        // drop-undeclare is then the only teardown there is.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let declared: Vec<(String, crate::bus::write::Publication)> = self.0.drain().collect();
        runtime.spawn(async move {
            for (key, publication) in declared {
                if let Err(e) = publication.undeclare().await {
                    tracing::warn!(key = %key, "undeclare after a cancelled replay: {e}");
                }
            }
        });
    }
}

/// Replay a `.zrec` onto a bus — or list what doing so would publish.
///
/// Pacing follows each row's `t` divided by `speed` (must be positive);
/// a dry run lists instantly, because a preview that takes the capture's
/// duration is a preview nobody runs. Delete rows pass
/// [`crate::bus::write::check_retire`] under the **header's** base — the keys
/// were captured under it, and classifying them under anything else would
/// re-derive what O3 says must not be re-derived; `i_know` is the operator
/// saying the off-state cleanup is meant. Publishers are declared once per
/// distinct key and undeclared on **every** way out — a failed row tears the
/// set down before it reports, and a cancelled replay hands the remainder to
/// a drop guard that undeclares them properly (#327).
pub async fn replay(
    reader: &mut ZrecSource,
    spec: ReplaySpec<'_>,
    mut on_event: impl FnMut(ReplayEvent<'_>),
) -> Result<ReplayReport> {
    let ReplaySpec {
        target,
        speed,
        i_know,
        default_qos,
        seed_state,
    } = spec;
    if !(speed.is_finite() && speed > 0.0) {
        return Err(Error::unaskable(
            "--speed",
            format!("must be a positive number (got {speed})"),
        ));
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
        preamble_skipped: 0,
        preamble_seeded: 0,
        triggers: 0,
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
    let mut publications = Publications::default();
    let mut prev_t: Option<u64> = None;
    // The one fatal error a row can raise, held rather than thrown: the
    // publishers are undeclared first, and only then does it go back to the
    // caller (#327).
    let mut fatal: Option<Error> = None;
    while let Some(item) = reader.next().await {
        let (row, t_us, seeding) = match item {
            Ok(ZrecItem::Sample { row, t_us, .. }) => (row, t_us, false),
            Ok(ZrecItem::Dropped(n)) => {
                report.capture_dropped += n;
                on_event(ReplayEvent::CaptureDropped(n));
                continue;
            }
            Ok(ZrecItem::Trigger(t)) => {
                report.triggers += 1;
                on_event(ReplayEvent::Trigger(&t));
                continue;
            }
            // A preamble row is state at capture start (RFC 13 §4.1). Without
            // the opt-in it is skipped and *said* — a snapshot republished
            // over a live fleet is the sharpest form of §4.2's hazard. With
            // it, the row is a sample at `t: 0`: same retire gate, same
            // publisher, no pacing (there is none to keep).
            Ok(ZrecItem::Preamble { row, .. }) => {
                if !seed_state {
                    report.preamble_skipped += 1;
                    on_event(ReplayEvent::PreambleSkipped {
                        key: &row.key,
                        reason: PREAMBLE_SKIP_REASON,
                    });
                    continue;
                }
                (row, Some(0), true)
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
            && let Err(e) = crate::bus::write::check_retire(&base, &row.key, slices, i_know)
        {
            let reason = e.to_string();
            on_event(ReplayEvent::Refused {
                key: row.key.clone(),
                reason: reason.clone(),
            });
            record_err(&mut report, format!("{}: {reason}", row.key), true);
            continue;
        }
        // A seeded preamble row is counted as what it is, never as an
        // observed row (O6 applied to rows); the dry-run listing still names
        // it as the put it would be.
        let count_put = |report: &mut ReplayReport, delete: bool| match (seeding, delete) {
            (true, _) => report.preamble_seeded += 1,
            (false, true) => report.tombstones += 1,
            (false, false) => report.published += 1,
        };
        match &target {
            ReplayTarget::DryRun => {
                if row.delete {
                    on_event(ReplayEvent::WouldRetire { key: &row.key });
                } else {
                    on_event(ReplayEvent::WouldPut {
                        key: &row.key,
                        bytes: row.payload.len(),
                        encoding: row.encoding.as_deref(),
                    });
                }
                count_put(&mut report, row.delete);
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
                        // A row that recorded a profile name is judged
                        // against the closed vocabulary — a name the capture
                        // carries can be anything. A row that recorded none
                        // falls to the spec's profile, which is already
                        // typed and so cannot fail here.
                        let qos = match &row.qos {
                            None => default_qos,
                            Some(name) => match zenkey::qos::QosProfile::from_name(name) {
                                Some(qos) => qos,
                                None => {
                                    let reason = format!("unknown QoS profile {name:?}");
                                    on_event(ReplayEvent::Malformed {
                                        reason: reason.clone(),
                                    });
                                    record_err(&mut report, reason, false);
                                    continue;
                                }
                            },
                        };
                        let publication = match crate::bus::write::declare_publication(
                            session,
                            &row.key,
                            qos,
                            row.encoding.as_deref(),
                        )
                        .await
                        {
                            Ok(p) => p,
                            Err(e) => {
                                fatal = Some(e);
                                break;
                            }
                        };
                        e.insert(publication)
                    }
                };
                let delete = row.delete;
                let sent = if delete {
                    publication.retire().await
                } else {
                    publication.send(row.payload, row.attachment).await
                };
                match sent {
                    Ok(()) => count_put(&mut report, delete),
                    Err(e) => {
                        fatal = Some(e);
                        break;
                    }
                }
            }
        }
    }
    // Teardown first, on every path out — the row error is the one reported,
    // but a failure to undeclare is never skipped for it.
    let closed = publications.close().await;
    if let Some(e) = fatal {
        return Err(e);
    }
    closed?;
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
            preamble: None,
            pre_roll: None,
        }
    }

    /// A replay source over an in-memory capture. `Cursor<Vec<u8>>` because
    /// the read happens on the blocking pool and so must own its bytes.
    async fn source_of(body: &str) -> ZrecSource {
        ZrecSource::spawn(std::io::Cursor::new(body.as_bytes().to_vec()))
            .await
            .expect("a .zrec header")
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

        // The version after this one is refused, never guessed at (RFC 13
        // §4.1's unknown-version rule) — and the refusal says what this
        // reader does speak.
        let future = r#"{"zrec":3,"selectors":[],"base":"","captured_at":"x"}"#;
        let err = ZrecReader::new(future.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("version 3"), "{err}");
        assert!(err.contains("speaks 2 and reads 1"), "{err}");

        let not_zrec = r#"{"key":"v1/x","value":1}"#;
        let err = ZrecReader::new(not_zrec.as_bytes())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("header"), "{err}");
    }

    /// A version-1 file — no `preamble`, no `pre_roll`, plain rows and drop
    /// records — reads under the version-2 reader exactly as it did (RFC 13
    /// §4.1: a version-2 reader MUST read version 1), and the header comes
    /// back with the two blocks absent rather than defaulted.
    #[test]
    fn a_version_one_body_reads_under_the_version_two_reader() {
        let body = concat!(
            r#"{"zrec":1,"selectors":["v1/**"],"base":"","captured_at":"2026-08-12T00:00:00Z"}"#,
            "\n",
            r#"{"key":"v1/h/state/p/a","t":0,"bytes":"AQ=="}"#,
            "\n",
            r#"{"dropped":2}"#,
            "\n",
            r#"{"key":"v1/h/state/p/a","t":1000,"delete":true}"#,
            "\n",
        );
        let mut reader = ZrecReader::new(body.as_bytes()).unwrap();
        assert_eq!(reader.header().zrec, 1);
        assert_eq!(reader.header().preamble, None);
        assert_eq!(reader.header().pre_roll, None);
        assert!(matches!(
            reader.next(),
            Some(Ok(ZrecItem::Sample { t_us: Some(0), .. }))
        ));
        assert!(matches!(reader.next(), Some(Ok(ZrecItem::Dropped(2)))));
        assert!(matches!(
            reader.next(),
            Some(Ok(ZrecItem::Sample {
                t_us: Some(1000),
                ..
            }))
        ));
        assert!(reader.next().is_none());
    }

    /// The version-2 lines read back as what they are: a preamble row is
    /// `Preamble` (at `t: 0`, keeping its HLC as provenance), a trigger
    /// record is the same `Transition` that was written — and a version-2
    /// header round-trips both blocks.
    #[test]
    fn version_two_lines_read_back_by_kind() {
        use crate::report::{CondState, PreRollInfo, PreambleInfo, PreambleSemantics};
        let epoch = Instant::now();
        let header = ZrecHeader {
            preamble: Some(PreambleInfo {
                count: 1,
                collected_over_s: 0.25,
                selectors: vec!["v1/*/state/**".into()],
                semantics: PreambleSemantics::AbsentFromWindow,
                incomplete: 0,
                failed: vec!["v1/*/telemetry/**".into()],
            }),
            pre_roll: Some(PreRollInfo {
                asked_s: 30.0,
                covered_s: 12.5,
                watched: vec!["v1/**".into()],
                evicted: 0,
                expired: 40,
            }),
            ..header()
        };
        let stamp = zenoh::time::Timestamp::new(
            zenoh::time::NTP64::from(Duration::from_secs(1_700_000_000)),
            zenoh::time::TimestampId::try_from([7u8; 16]).unwrap(),
        );
        let fetched = crate::bus::monitor::SampleView {
            key: "v1/h-0123456789ab/state/p/config".into(),
            payload: zenoh::bytes::ZBytes::from(vec![9u8]),
            encoding: String::new(),
            kind: SampleKind::Put,
            timestamp: Some(stamp),
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            // Received "after" the epoch: a preamble row still writes t: 0.
            received: epoch + Duration::from_secs(5),
        };
        let fired = Transition {
            rule: "silent-for v1/h-0123456789ab/state/p/health 0.7".into(),
            from: Some(CondState::Ok),
            to: CondState::Firing,
            at: "2026-09-06T00:00:00Z".into(),
            evidence: "no sample for 0.7s, on a drop-free observer".into(),
        };

        let mut sink = Vec::new();
        let mut w = ZrecWriter::new_at(&mut sink, &header, epoch).unwrap();
        w.write_preamble(&fetched).unwrap();
        w.write_trigger(&fired).unwrap();
        assert_eq!(
            w.counts(),
            SinkCounts {
                samples: 0,
                dropped: 0,
                preamble: 1,
                triggers: 1
            },
            "the kinds are counted apart"
        );
        let _ = w.finish().unwrap();
        let text = String::from_utf8(sink.clone()).unwrap();
        assert!(text.contains(r#""preamble":true"#), "{text}");
        assert!(text.contains(r#""t":0"#), "{text}");
        assert!(text.contains(r#"{"trigger":{"#), "{text}");

        let mut reader = ZrecReader::new(sink.as_slice()).unwrap();
        assert_eq!(reader.header(), &header);
        match reader.next() {
            Some(Ok(ZrecItem::Preamble { row, timestamp })) => {
                assert_eq!(row.key, fetched.key);
                assert_eq!(row.payload, vec![9u8]);
                assert_eq!(timestamp.as_deref(), Some(stamp.to_string().as_str()));
            }
            other => panic!("expected a preamble row, got {other:?}"),
        }
        match reader.next() {
            Some(Ok(ZrecItem::Trigger(t))) => assert_eq!(*t, fired),
            other => panic!("expected the trigger record, got {other:?}"),
        }
        assert!(reader.next().is_none());
    }

    /// A version-2 capture as the CLI replays it: without `--seed-state`
    /// the preamble row is skipped and *said* (RFC 13 §4.2's hazard), the
    /// trigger is a marker and never a put, and the observed row still
    /// counts; with it, the preamble row is a would-be put counted as
    /// seeded, apart from the observed rows.
    #[tokio::test]
    async fn a_dry_run_skips_the_preamble_by_default_and_seeds_it_on_request() {
        let body = format!(
            "{}\n{}\n{}\n{}\n",
            serde_json::to_string(&header()).unwrap(),
            r#"{"key":"v1/h-0123456789ab/state/p/config","t":0,"preamble":true,"bytes":"CQ=="}"#,
            r#"{"key":"v1/h-0123456789ab/state/p/health","t":250000,"bytes":"eyJvayI6dHJ1ZX0="}"#,
            r#"{"trigger":{"rule":"silent-for k 0.7","from":"ok","to":"firing","at":"x","evidence":"e"}}"#,
        );
        let spec = |seed_state| ReplaySpec {
            target: ReplayTarget::DryRun,
            speed: 1.0,
            i_know: false,
            default_qos: QosProfile::Refreshed,
            seed_state,
        };

        let mut reader = source_of(&body).await;
        let mut skipped = Vec::new();
        let mut triggers = 0;
        let report = replay(&mut reader, spec(false), |ev| match ev {
            ReplayEvent::PreambleSkipped { key, reason } => {
                skipped.push((key.to_string(), reason));
            }
            ReplayEvent::Trigger(t) => {
                assert_eq!(t.to, crate::report::CondState::Firing);
                triggers += 1;
            }
            _ => {}
        })
        .await
        .unwrap();
        assert_eq!(report.preamble_skipped, 1);
        assert_eq!(report.preamble_seeded, 0);
        assert_eq!(report.published, 1);
        assert_eq!(report.triggers, 1);
        assert_eq!(triggers, 1);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].0, "v1/h-0123456789ab/state/p/config");
        assert!(skipped[0].1.contains("--seed-state"), "{}", skipped[0].1);
        assert!(skipped[0].1.contains("RFC 13 §4.2"), "{}", skipped[0].1);

        let mut reader = source_of(&body).await;
        let mut would_put = 0;
        let report = replay(&mut reader, spec(true), |ev| {
            if matches!(ev, ReplayEvent::WouldPut { .. }) {
                would_put += 1;
            }
        })
        .await
        .unwrap();
        assert_eq!(report.preamble_seeded, 1);
        assert_eq!(report.preamble_skipped, 0);
        assert_eq!(report.published, 1, "the observed row, not the seed");
        assert_eq!(would_put, 2, "both rows are listed as puts");
    }

    /// A retained window written through `new_at` keeps its real pacing
    /// (#217): rows received *before* the writer existed carry their true
    /// offsets from the injected epoch instead of saturating to `t: 0`.
    #[test]
    fn an_injected_epoch_preserves_a_window_written_after_the_fact() {
        let epoch = Instant::now();
        let view = |t_ms: u64| crate::bus::monitor::SampleView {
            key: "v1/h-0123456789ab/state/p/a".into(),
            payload: zenoh::bytes::ZBytes::from(vec![1u8]),
            encoding: String::new(),
            kind: SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: epoch + Duration::from_millis(t_ms),
        };
        let mut sink = Vec::new();
        let mut w = ZrecWriter::new_at(&mut sink, &header(), epoch).unwrap();
        w.write_sample(&view(0)).unwrap();
        w.write_sample(&view(1500)).unwrap();
        let _ = w.finish().unwrap();

        let mut reader = ZrecReader::new(sink.as_slice()).unwrap();
        let t_of = |item| match item {
            Some(Ok(ZrecItem::Sample { t_us, .. })) => t_us,
            other => panic!("expected a sample, got {other:?}"),
        };
        assert_eq!(t_of(reader.next()), Some(0));
        assert_eq!(
            t_of(reader.next()),
            Some(1_500_000),
            "the offset the ring preserved, not a saturated zero"
        );
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
        let mut reader = source_of(&body).await;
        let mut would = Vec::new();
        let report = replay(
            &mut reader,
            ReplaySpec {
                target: ReplayTarget::DryRun,
                speed: 1.0,
                i_know: false,
                default_qos: QosProfile::Refreshed,
                seed_state: false,
            },
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
        let mut reader = source_of(&body).await;
        let report = replay(
            &mut reader,
            ReplaySpec {
                target: ReplayTarget::DryRun,
                speed: 1.0,
                i_know: false,
                default_qos: QosProfile::Refreshed,
                seed_state: false,
            },
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
            let mut reader = source_of(&body).await;
            let err = replay(
                &mut reader,
                ReplaySpec {
                    target: ReplayTarget::DryRun,
                    speed: bad,
                    i_know: false,
                    default_qos: QosProfile::Refreshed,
                    seed_state: false,
                },
                |_| {},
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(err.contains("speed"), "{err}");
        }
    }

    /// A row the bus refuses is still reported — the teardown that now runs
    /// first does not swallow it (#327). The undeclared publishers left behind
    /// by the old `?` were invisible from the outside, which is why the drain
    /// itself is pinned in `bus::teardown`; what is observable here is that
    /// the failing row's own error is what comes back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_row_the_bus_refuses_tears_down_and_still_reports_itself() {
        let session = crate::bus::session::open(&[], &[], false)
            .await
            .expect("a standalone peer");
        let good = SampleRow {
            key: "v1/h-aaaaaaaaaaaa/state/demo/health".into(),
            ..SampleRow::default()
        }
        .with_payload_bytes(b"{}");
        // An empty chunk is not a key expression, so `declare_publication`
        // refuses it — the fatal row this replay dies on, after one good
        // publisher is already declared.
        let bad = SampleRow {
            key: "v1//nowhere".into(),
            ..SampleRow::default()
        }
        .with_payload_bytes(b"{}");
        let body = format!(
            "{}\n{}\n{}\n",
            serde_json::to_string(&header()).unwrap(),
            good.to_line(),
            bad.to_line(),
        );

        let mut reader = source_of(&body).await;
        let err = replay(
            &mut reader,
            ReplaySpec {
                target: ReplayTarget::Bus {
                    session: &session,
                    slices: None,
                },
                speed: 1000.0,
                i_know: false,
                default_qos: QosProfile::Transition,
                seed_state: false,
            },
            |_| {},
        )
        .await
        .expect_err("the bus refused the second row")
        .to_string();
        assert!(err.contains("nowhere"), "{err}");

        session.close().await.expect("close the session");
    }
}
