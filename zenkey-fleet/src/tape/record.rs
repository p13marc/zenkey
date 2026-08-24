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

use anyhow::{Context, Result, anyhow, bail};
use zenkey::qos::QosProfile;
use zenoh::Session;
use zenoh::sample::SampleKind;

use crate::bus::monitor::{EventStream, FleetEvent, SampleView, StreamItem};
use crate::model::registry::SliceSet;
use crate::report::{ReplayReport, SampleRow, ZrecHeader};
use crate::tape::ingest::{IngestRow, parse_row};

/// The current `.zrec` format version, written into every header.
pub const ZREC_VERSION: u32 = 1;

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
        serde_json::to_writer(&mut out, header).context("write .zrec header")?;
        out.write_all(b"\n").context("write .zrec header")?;
        Ok(ZrecWriter {
            out,
            epoch,
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
        let mut row = SampleRow {
            key: view.key.clone(),
            t: Some(t_us),
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
        self.out
            .write_all(row.to_line().as_bytes())
            .context("write .zrec row")?;
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
}

/// What the sink's async half can see of a writer that lives on the
/// blocking pool.
#[derive(Debug, Default)]
struct SinkState {
    samples: AtomicU64,
    dropped: AtomicU64,
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
    writer: tokio::task::JoinHandle<Result<(u64, u64)>>,
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
                    let _ = ready.send(Some(format!("{e:#}")));
                    return Err(e);
                }
            };
            while let Some(line) = rx.blocking_recv() {
                let wrote = match line {
                    ZrecLine::Sample(view) => writer.write_sample(&view),
                    ZrecLine::Dropped(n) => writer.write_dropped(n),
                };
                if let Err(e) = wrote {
                    *task_state.failure.lock().expect("sink failure lock") = Some(format!("{e:#}"));
                    return Err(e);
                }
            }
            let counts = writer.counts();
            writer.finish().map(|_| counts)
        });
        match opened.await {
            Ok(None) => Ok(ZrecSink { tx, state, writer }),
            Ok(Some(reason)) => Err(anyhow!(reason)),
            Err(_) => Err(anyhow!("the .zrec writer stopped before it opened")),
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
        Err(anyhow!(
            failure.unwrap_or_else(|| "the .zrec writer stopped".to_string())
        ))
    }

    /// Samples and drops **accepted** so far — the progress line's numbers.
    ///
    /// Accepted, not yet written: the queue is what stands between the two,
    /// and [`finish`](Self::finish) drains it, so the final counts are the
    /// file's. A progress line that waited for the disk would be reporting
    /// the disk, not the capture.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.state.samples.load(Ordering::Relaxed),
            self.state.dropped.load(Ordering::Relaxed),
        )
    }

    /// Close the queue, wait for the writer to drain it, flush, and report
    /// what reached the file: (samples, dropped).
    ///
    /// This is where a write error surfaces if the capture did not already
    /// trip over it. The counts come from the writer rather than the queue,
    /// so a report built on them is a report about the file.
    pub async fn finish(self) -> Result<(u64, u64)> {
        let ZrecSink { tx, state, writer } = self;
        drop(tx);
        drop(state);
        writer.await.context("the .zrec writer panicked")?
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
        let (samples, _) = sink.counts();
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
        let (samples, dropped) = sink.counts();
        on_progress(samples, dropped);
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
    /// ndjson pipes replay through `zenctl pub --from ndjson`, which needs
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
            Err(_) => Err(anyhow!("the .zrec reader stopped before it opened")),
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
    } = spec;
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
    let mut publications = Publications::default();
    let mut prev_t: Option<u64> = None;
    // The one fatal error a row can raise, held rather than thrown: the
    // publishers are undeclared first, and only then does it go back to the
    // caller (#327).
    let mut fatal: Option<anyhow::Error> = None;
    while let Some(item) = reader.next().await {
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
                match (sent, delete) {
                    (Ok(()), true) => report.tombstones += 1,
                    (Ok(()), false) => report.published += 1,
                    (Err(e), _) => {
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
