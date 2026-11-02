//! Frames written (core §7.7, "The owner's frames"; #981's K3).
//!
//! A frame resource's owner hands each record it measures to a
//! [`FrameWriter`], which gathers them into frames with
//! `zenkey_model::frame::Framer`, the session-free half, and puts each
//! frame on the key:
//!
//! - **Its seqs.** A frame takes one stream seq of its key per record, its
//!   envelope carrying record 0's (core §4.3, [`crate::state::Minter::mint_stream_n`]),
//!   so a reader reads a gap in the seqs as records lost.
//! - **Full and delta records**, the deadband and the times are the
//!   framer's: record 0 of a run's first frame full, a full record every
//!   `keyframe_every`, a delta record including what changed. A frame never
//!   sent gives its seqs back, and the next is written against the last one
//!   sent.
//! - **Record 0's time** is the owner's wall clock when it was measured,
//!   from the deployment epoch (core §7.6, §8.6), and unknown while the
//!   clock guard holds. A later record's is its offset on the monotonic
//!   clock, or scheduled within `jitter_ms`.
//! - **Never a mutation.** A plain writer of a frame resource is refused
//!   ([`crate::writer::Writer`]), so no mutation reaches its key.
//! - **No Zenoh timestamp**: a frame resource is on no row of core §4.3's
//!   table (E050 refuses a lifespan), unless its session stamps every put
//!   (§8.2).
//!
//! - **Counters** are taken as counted, unwrapped, and written modulo
//!   their field's width (core §7.6). On a contract that uses
//!   `telemetry.v2`, a frame resource's series are metered first
//!   ([`crate::telemetry`]'s second table, #983 T3): a counter field below
//!   its last count within the run is refused, [`Error::Telemetry`], and
//!   the record joins no frame. A counter that restarts is written under
//!   a new run (`telemetry.v2` §2.19): flush the open frame
//!   ([`FrameWriter::flush`]), mint a new instance, which takes one
//!   ([`crate::Service::new_epoch`], core §1.5), then record the restarted
//!   count.
//!
//! A frame is put when it holds `records` records, or when a record lies
//! past its span; [`FrameWriter::flush`] puts the open frame earlier.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::Value;
use zenkey_model::frame::{Batch, Framer, Layout, Measured, Slot, Time};
use zenkey_model::packed::{FieldKind, top_code};
use zenkey_model::telemetry::frames::FieldSeries;
use zenkey_model::timing::Mono;
use zenkey_model::wire::{Run, Stamp};
use zenoh::key_expr::OwnedKeyExpr;

use crate::error::{Error, Result};
use crate::state::Minter;
use crate::telemetry::{Counts, FrameMeter};
use crate::writer::Writer;

/// A frame writer's framer, and its meter where its resource's series are
/// declared (`telemetry.v2`): one lock, so a record is checked, framed and
/// put in one order.
struct Framing {
    framer: Framer,
    meter: Option<FrameMeter>,
}

impl Framing {
    /// The open frame went to a put under `run`.
    fn sent(&mut self, run: Run) {
        if let Some(m) = &mut self.meter {
            m.sent(run);
        }
    }
}

/// One frame resource's writer (core §7.7): its records gathered into
/// frames, each put under its record 0's seq.
///
/// Made by its service ([`crate::ServiceBuilder::declare_frame_writer`],
/// [`crate::Service::frame_writer`]), on the resource's one key. Its puts
/// keep the contract's QoS, the resource's budget, and on a governed
/// interface its gate, as a [`Writer`]'s do: a frame while the instance
/// stands by is refused, [`Error::Standby`], and dropped.
pub struct FrameWriter {
    writer: Writer,
    key: OwnedKeyExpr,
    layout: Arc<Layout>,
    framing: tokio::sync::Mutex<Framing>,
    minter: Arc<Minter>,
    /// The deployment epoch its `time` fields and record 0's time count
    /// from, in seconds (core §7.6, §8.6).
    epoch: i64,
    /// The origin of its monotonic instants.
    origin: Instant,
    /// The next frame is lost on its way: a test's hook.
    lose: AtomicBool,
}

impl FrameWriter {
    pub(crate) fn new(
        writer: Writer,
        layout: Layout,
        minter: &Arc<Minter>,
        epoch: i64,
        series: Option<Arc<[FieldSeries]>>,
    ) -> Self {
        let key = OwnedKeyExpr::from(writer.key_expr().clone().into_owned());
        Self {
            writer,
            key,
            layout: Arc::new(layout.clone()),
            framing: tokio::sync::Mutex::new(Framing {
                framer: Framer::new(layout),
                meter: series.map(FrameMeter::new),
            }),
            minter: Arc::clone(minter),
            epoch,
            origin: Instant::now(),
            lose: AtomicBool::new(false),
        }
    }

    /// The frame layout: its record type's fields and its `frame` table.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The resource's key.
    #[must_use]
    pub fn key_expr(&self) -> &zenoh::key_expr::KeyExpr<'static> {
        self.writer.key_expr()
    }

    /// The publisher its frames go out on: its QoS, its matching status.
    #[must_use]
    pub fn writer(&self) -> &Writer {
        &self.writer
    }

    /// The deployment epoch its `time` fields count from, in seconds: its
    /// service's catalog's, else 0 (core §7.6, §8.6).
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.epoch
    }

    /// How many records the open frame holds.
    pub async fn open(&self) -> usize {
        self.framing.lock().await.framer.open()
    }

    /// Whether its records are metered (`telemetry.v2`, [`crate::telemetry`]):
    /// its contract uses the profile, and its resource's series are
    /// declared.
    pub async fn is_metered(&self) -> bool {
        self.framing.lock().await.meter.is_some()
    }

    /// Records a §7.6 record of the resource's type, measured now. Returns
    /// the stamp of the frame it completed and put, if it completed one.
    ///
    /// A record's bytes carry each counter field modulo its width: a
    /// metered counter of width below 64 is read as its last count plus
    /// the code's increase from it ([`crate::telemetry`], "The count").
    ///
    /// # Errors
    /// [`Error::Payload`] for a record its type refuses, or a frame its
    /// layout does not write; [`Error::Telemetry`] for a record its series
    /// refuse; and a put's errors, the frame then dropped.
    pub async fn record(&self, record: &[u8]) -> Result<Option<Stamp>> {
        self.record_at(record, Instant::now()).await
    }

    /// [`FrameWriter::record`], the record measured at `at` on the
    /// monotonic clock: an owner that measured it earlier, or a test of
    /// its times.
    ///
    /// # Errors
    /// [`FrameWriter::record`]'s, and [`Error::Payload`] for a record
    /// measured before the last one.
    pub async fn record_at(&self, record: &[u8], at: Instant) -> Result<Option<Stamp>> {
        let slots = self
            .layout
            .slots_of(record)
            .map_err(|r| self.refused(r.token()))?;
        self.take(slots, Counts::Codes, at).await
    }

    /// A §7.6 record's bytes with core §7.6's JSON reading of the same
    /// record, measured now: each counter field's count taken unwrapped
    /// from the reading, every other field from the bytes. A generated
    /// typed writer's ([`crate::typed::FrameWriter`]).
    ///
    /// # Errors
    /// [`FrameWriter::record`]'s, and [`Error::Payload`] for a reading
    /// whose count is not the code its bytes carry.
    pub(crate) async fn record_counted(
        &self,
        record: &[u8],
        reading: &Value,
    ) -> Result<Option<Stamp>> {
        let mut slots = self
            .layout
            .slots_of(record)
            .map_err(|r| self.refused(r.token()))?;
        for (f, slot) in self.layout.fields().iter().zip(&mut slots) {
            let Slot::Code(code) = *slot else { continue };
            if !matches!(f.kind, FieldKind::Counter) {
                continue;
            }
            match reading.get(&f.name).and_then(Value::as_u64) {
                Some(count) if count & top_code(f.width) == code => *slot = Slot::Code(count),
                _ => {
                    return Err(Error::Payload(format!(
                        "{}: the JSON reading's count for {} does not reduce to the record's \
                         code {code}",
                        self.key, f.name
                    )));
                }
            }
        }
        self.take(slots, Counts::Unwrapped, Instant::now()).await
    }

    /// Records a record in §7.6's JSON reading, a field's name to its
    /// value, measured now: an optional field left out is absent, and a
    /// counter field's value its count, unwrapped, which the writer
    /// reduces modulo its width.
    ///
    /// # Errors
    /// [`FrameWriter::record`]'s.
    pub async fn record_json(&self, value: &Value) -> Result<Option<Stamp>> {
        self.record_json_at(value, Instant::now()).await
    }

    /// [`FrameWriter::record_json`], measured at `at`.
    ///
    /// # Errors
    /// [`FrameWriter::record_at`]'s.
    pub async fn record_json_at(&self, value: &Value, at: Instant) -> Result<Option<Stamp>> {
        let mut slots = self
            .layout
            .slots_of_json(value, Some(self.epoch))
            .map_err(|r| self.refused(r.token()))?;
        // Each counter's count as given: `slots_of_json` reduced it.
        for (f, slot) in self.layout.fields().iter().zip(&mut slots) {
            if matches!(f.kind, FieldKind::Counter)
                && let (Slot::Code(_), Some(v)) = (*slot, value.get(&f.name))
            {
                let count = f
                    .code(v, Some(self.epoch))
                    .map_err(|r| self.refused(r.token()))?;
                *slot = Slot::Code(count);
            }
        }
        self.take(slots, Counts::Unwrapped, at).await
    }

    /// Records a full record's slots, each field absent or its code, a
    /// counter field's code its count, unwrapped, which the writer reduces
    /// modulo its width; measured at `at`.
    ///
    /// # Errors
    /// [`FrameWriter::record_at`]'s.
    pub async fn record_slots_at(&self, slots: Vec<Slot>, at: Instant) -> Result<Option<Stamp>> {
        self.take(slots, Counts::Unwrapped, at).await
    }

    /// One record, its counters given as `counts` says: metered, every
    /// counter reduced to its width, then framed, and the frame it
    /// completes put.
    async fn take(
        &self,
        mut slots: Vec<Slot>,
        counts: Counts,
        at: Instant,
    ) -> Result<Option<Stamp>> {
        let (mono, wall) = (self.mono(at), self.wall(at));
        let fields = self.layout.fields();
        let mut f = self.framing.lock().await;
        let checked = match &f.meter {
            Some(m) => Some(
                m.check(fields, &slots, counts, &|| self.minter.run())
                    .map_err(|reason| Error::Telemetry {
                        member: self.key.to_string(),
                        reason: Box::new(reason),
                    })?,
            ),
            None => None,
        };
        for (field, slot) in fields.iter().zip(&mut slots) {
            if let (FieldKind::Counter, Slot::Code(c)) = (&field.kind, &mut *slot) {
                *c &= top_code(field.width);
            }
        }
        let measured = Measured {
            slots,
            at: mono,
            wall,
        };
        let batch = f
            .framer
            .push(measured)
            .map_err(|r| self.refused(r.token()))?;
        let take = |f: &mut Framing, checked| {
            if let (Some(m), Some(c)) = (&mut f.meter, checked) {
                m.take(c);
            }
        };
        let Some(b) = batch else {
            take(&mut f, checked);
            return Ok(None);
        };
        // The record completed the frame, or lies past the open one's span
        // and opens the next: its counts are its frame's.
        if f.framer.open() == 0 {
            take(&mut f, checked);
            self.put(&mut f, &b).await.map(Some)
        } else {
            let put = self.put(&mut f, &b).await;
            take(&mut f, checked);
            put.map(Some)
        }
    }

    /// Puts the open frame now, if it holds a record: an owner that sends
    /// what it measured so far, or stops, or whose counter restarts, before
    /// [`crate::Service::new_epoch`] (the module doc's counters).
    ///
    /// # Errors
    /// A put's, the frame then dropped.
    pub async fn flush(&self) -> Result<Option<Stamp>> {
        let mut f = self.framing.lock().await;
        match f.framer.flush() {
            Some(b) => self.put(&mut f, &b).await.map(Some),
            None => Ok(None),
        }
    }

    /// Loses the next frame on its way: it takes its seqs, and nothing is
    /// put, as a put a network dropped. For tests of a reader's gap
    /// (spec scenarios `frames.md`); a deployment never calls it.
    pub fn simulate_loss(&self) {
        self.lose.store(true, Ordering::Relaxed);
    }

    /// `at` on the writer's monotonic clock.
    fn mono(&self, at: Instant) -> Mono {
        Mono::after(at.saturating_duration_since(self.origin))
    }

    /// The owner's wall clock at `at`, from the deployment epoch: unknown
    /// while the clock guard holds (core §4.3, "Ahead").
    fn wall(&self, at: Instant) -> Option<Time> {
        if self.minter.is_ahead() {
            return None;
        }
        let now = Instant::now();
        let wall = self
            .minter
            .now()
            .get_time()
            .to_system_time()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        let ms = i128::try_from(wall.as_millis()).unwrap_or(i128::MAX);
        let ms = if at <= now {
            ms - i128::try_from(now.duration_since(at).as_millis()).unwrap_or(0)
        } else {
            ms + i128::try_from(at.duration_since(now).as_millis()).unwrap_or(0)
        };
        Time::from_unix_ms(ms, self.epoch)
    }

    /// Puts one frame under the framer's lock: its seqs minted, its body
    /// written under their run, then the put. A frame never sent gives its
    /// seqs back, and the framer keeps nothing of it; the meter takes its
    /// counts as of the run it went to, sent or not.
    async fn put(&self, f: &mut Framing, batch: &Batch) -> Result<Stamp> {
        let admitted = match self.writer.serving().await {
            Ok(a) => a,
            Err(e) => {
                f.sent(self.minter.run());
                return Err(e);
            }
        };
        if let Err(e) = self.writer.admit() {
            f.sent(self.minter.run());
            return Err(e);
        }
        let n = batch.len();
        let stamped = self
            .minter
            .mint_stream_n(&self.key, n, self.writer.is_stamped());
        f.sent(stamped.stamp.run);
        let framer = &mut f.framer;
        let encoded = framer.encode(batch, stamped.stamp.run);
        let payload = match encoded.frame.envelope(&self.layout, stamped.stamp) {
            Ok(p) => p,
            Err(r) => {
                self.minter.unmint_stream_n(&self.key, stamped.stamp, n);
                return Err(self.refused(r.token()));
            }
        };
        if self.lose.swap(false, Ordering::Relaxed) {
            framer.sent(encoded);
            return Ok(stamped.stamp);
        }
        match self
            .writer
            .put_stamped(&admitted, payload.into(), None, stamped)
            .await
        {
            Ok(()) => {
                framer.sent(encoded);
                Ok(stamped.stamp)
            }
            Err(e) => {
                self.minter.unmint_stream_n(&self.key, stamped.stamp, n);
                Err(e)
            }
        }
    }

    fn refused(&self, token: &str) -> Error {
        Error::Payload(format!(
            "{}: a frame's record refused, {token}: nothing published (core §7.7)",
            self.key
        ))
    }
}
