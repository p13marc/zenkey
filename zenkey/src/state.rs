//! State (spec §4): an owner's stamped mutations, its GET answers with
//! tombstones, clocks bounded both ways, and the consumer's owner-only GET.
//!
//! | Rule | Here |
//! |---|---|
//! | S1 every put and delete stamped by the owner | [`StateWriter`], [`Minter`] |
//! | S2 GET replies carry the mutation's stamp; the owner's state queryables answer every live key plus `reply_del` within the window | the state server ([`Store`]) |
//! | S3 the tombstone window, 60 s unless configured | [`DEFAULT_WINDOW`], `ServiceConfig::tombstone_window_s` |
//! | S4 a consumer's GET: the owner's keys, target `All`, consolidation `Latest` | [`crate::consumer::Consumer::get`] |
//! | S6 current is the owner's answer; silence is no verdict | [`StateGet`] |
//! | S7 clocks: minting, catch-up, new epoch, ahead | [`Minter`], the clock guard (`ServiceConfig::clock_reference`) |
//! | a re-put is a mutation (0.21); `freshness.v1` §2.4, §2.10 | [`StateWriter`]'s refresher |
//!
//! Last-known state (S5) is an archive's: [`crate::archive`].
//!
//! **The refresher** (`freshness.v1` §2.4, #720). A state resource whose
//! contract declares `freshness.ttl_s` above 0 is confirmed at least every
//! ttl/2: a [`StateWriter`] on one of its members re-puts the member's
//! current value, unchanged, whenever that long passes without a put. The
//! re-put is a mutation like any other (core 0.21): a fresh stamp from the
//! [`Minter`], recorded for the GET answers (S2), then published. It is on
//! by default, since the contract asks it of every owner, and
//! [`StateWriter::set_refresh`] turns it off for an owner that can no longer
//! vouch for its value (§2.4). It stops when the writer drops, when the
//! service closes or drops, after a delete until the next put, and while
//! the clock guard holds writes (§2.10).

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use zenkey_model::freshness::{ClockTrust, Horizon, Observation, Reply, StampAge};
use zenoh::Wait;
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::Query;
use zenoh::sample::Sample;
use zenoh::time::{NTP64, Timestamp};

use crate::error::{Error, Result};
use crate::writer::Writer;

/// How early the refresher re-puts: at this share of the ttl/2 bound, so
/// that a timer's lateness never takes an interval past it (`freshness.v1`
/// §2.4).
pub const REFRESH_LEAD: f64 = 0.95;

/// How often a re-put the clock guard holds is tried again, or every
/// period when that is shorter (`freshness.v1` §2.10).
const HELD_RETRY: Duration = Duration::from_secs(1);

/// The longest single sleep of the refresher: a far horizon (a year, in
/// ZenSight's catalog) is waited out in steps.
const STEP: Duration = Duration::from_secs(3600);

/// The tombstone window when the deployment configures none (S3).
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

/// zenoh's default HLC maximum delta (§4.1): a router re-stamps a put dated
/// further ahead, so an owner must not stamp beyond it (§4.3, "Ahead").
pub const HLC_DELTA: Duration = Duration::from_millis(500);

/// Mints the owner's state timestamps (§4.3): the greater of
/// `Session::new_timestamp()` and the last stamp issued plus one tick, with
/// the session's zid as the id. One per service.
pub struct Minter {
    session: zenoh::Session,
    last: Mutex<Option<Timestamp>>,
    offset_ms: AtomicI64,
    ahead: AtomicBool,
    /// The guard's hold, for what reports it (`health.v1` §2.5): the offset
    /// last measured while it holds, `None` while it does not. Receivers
    /// are woken on a change of hold only; the offset is kept current.
    held: tokio::sync::watch::Sender<Option<Duration>>,
}

impl Minter {
    pub(crate) fn new(session: &zenoh::Session) -> Self {
        Self {
            session: session.clone(),
            last: Mutex::new(None),
            offset_ms: AtomicI64::new(0),
            ahead: AtomicBool::new(false),
            held: tokio::sync::watch::Sender::new(None),
        }
    }

    /// The session's clock now, offset as [`Minter::simulate_offset`] says.
    pub(crate) fn now(&self) -> Timestamp {
        let t = self.session.new_timestamp();
        let off = self.offset_ms.load(Ordering::Relaxed);
        if off == 0 {
            return t;
        }
        let d = t.get_time().to_duration();
        let shifted = if off >= 0 {
            d + Duration::from_millis(off.unsigned_abs())
        } else {
            d.saturating_sub(Duration::from_millis(off.unsigned_abs()))
        };
        Timestamp::new(NTP64::from(shifted), *t.get_id())
    }

    /// The next state timestamp, or [`Error::ClockAhead`] while the clock
    /// guard has found this owner ahead of its router (§4.3).
    pub fn mint(&self) -> Result<Timestamp> {
        if self.ahead.load(Ordering::Relaxed) {
            return Err(Error::ClockAhead);
        }
        let mut t = self.now();
        let mut last = self.last.lock().expect("not poisoned");
        if let Some(l) = *last
            && t.get_time() <= l.get_time()
        {
            t = Timestamp::new(*l.get_time() + 1, *t.get_id());
        }
        *last = Some(t);
        Ok(t)
    }

    /// Catch-up (§4.3): the last stamp this owner issued for its keys, read
    /// from its own persistent record or an archive on its own side, before
    /// its first write. Nothing is then stamped at or below it.
    pub fn catch_up(&self, last: Timestamp) {
        let mut l = self.last.lock().expect("not poisoned");
        if l.is_none_or(|cur| cur.get_time() < last.get_time()) {
            *l = Some(last);
        }
    }

    /// The last stamp issued, for the owner's persistent record.
    #[must_use]
    pub fn last(&self) -> Option<Timestamp> {
        *self.last.lock().expect("not poisoned")
    }

    /// Whether the clock guard found this owner ahead (§4.3).
    #[must_use]
    pub fn is_ahead(&self) -> bool {
        self.ahead.load(Ordering::Relaxed)
    }

    /// The guard's hold as a watch: `Some(offset)` while it holds this
    /// owner's state writes, with the offset it last measured, `None` while
    /// it does not. It wakes on a change of hold only (`health.v1` §2.5).
    pub(crate) fn held(&self) -> tokio::sync::watch::Receiver<Option<Duration>> {
        self.held.subscribe()
    }

    /// The guard measured this owner `offset` ahead of its router: beyond
    /// [`HLC_DELTA`] its state writes stop (§4.3). Returns whether this
    /// detection is the first of a hold.
    fn measure(&self, offset: Duration) -> bool {
        let ahead = offset > HLC_DELTA;
        let first = if ahead {
            !self.ahead.swap(true, Ordering::Relaxed)
        } else {
            self.ahead.store(false, Ordering::Relaxed);
            false
        };
        let now = ahead.then_some(offset);
        self.held.send_if_modified(|h| {
            let changed = h.is_some() != now.is_some();
            *h = now;
            changed
        });
        first
    }

    /// Shifts this owner's clock by `ms` (negative: behind). For tests and
    /// simulation of the clock rules (spec scenarios state.md §7); a
    /// deployment never sets it.
    pub fn simulate_offset(&self, ms: i64) {
        self.offset_ms.store(ms, Ordering::Relaxed);
    }
}

/// Watches a router-stamped reference key (§4.3, "Ahead"): the owner's own
/// puts are never echoed back, so drift is measured against samples a router
/// stamped. Beyond [`HLC_DELTA`] the owner stops writing state.
pub(crate) struct ClockGuard {
    _sub: zenoh::pubsub::Subscriber<()>,
}

impl ClockGuard {
    pub(crate) async fn start(
        session: &zenoh::Session,
        key: &str,
        minter: Arc<Minter>,
    ) -> Result<Self> {
        let own = session.zid().to_string();
        let sub = session
            .declare_subscriber(key)
            .callback(move |s: Sample| {
                let Some(router) = s.timestamp() else { return };
                if router.get_id().to_string() == own {
                    return;
                }
                let mine = minter.now().get_time().to_duration();
                let theirs = router.get_time().to_duration();
                let offset = mine.saturating_sub(theirs);
                let was = minter.is_ahead();
                if minter.measure(offset) {
                    tracing::warn!(
                        ahead_ms = offset.as_millis() as u64,
                        "this owner's clock is ahead of its router beyond the HLC delta: \
                         state writes stop (spec §4.3)"
                    );
                } else if was && !minter.is_ahead() {
                    tracing::info!(
                        ahead_ms = offset.as_millis() as u64,
                        "this owner's clock is back within the HLC delta of its router: \
                         state writes resume (spec §4.3)"
                    );
                }
            })
            .await
            .map_err(crate::error::zenoh)?;
        Ok(Self { _sub: sub })
    }
}

#[derive(Debug, Clone)]
struct Entry {
    value: Option<(Vec<u8>, Encoding)>,
    /// The value's attachment (§2.3), answered with it (S2).
    attachment: Option<Vec<u8>>,
    ts: Timestamp,
    at: Instant,
}

/// The owner's state, as its queryables answer it (S2, S3).
pub struct Store {
    map: Mutex<HashMap<OwnedKeyExpr, Entry>>,
    window: Duration,
}

impl Store {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            window,
        }
    }

    fn put(
        &self,
        key: &OwnedKeyExpr,
        bytes: Vec<u8>,
        encoding: Encoding,
        attachment: Option<Vec<u8>>,
        ts: Timestamp,
    ) {
        self.map.lock().expect("not poisoned").insert(
            key.clone(),
            Entry {
                value: Some((bytes, encoding)),
                attachment,
                ts,
                at: Instant::now(),
            },
        );
    }

    fn delete(&self, key: &OwnedKeyExpr, ts: Timestamp) {
        self.map.lock().expect("not poisoned").insert(
            key.clone(),
            Entry {
                value: None,
                attachment: None,
                ts,
                at: Instant::now(),
            },
        );
    }

    /// Answers a query: every matching live key with its value and stamp,
    /// and a `reply_del` with the deletion's stamp for each matching key
    /// deleted within the window (S2, S3). Older tombstones are forgotten.
    pub(crate) fn answer(&self, q: &Query) {
        let mut map = self.map.lock().expect("not poisoned");
        map.retain(|_, e| e.value.is_some() || e.at.elapsed() <= self.window);
        for (key, e) in map.iter() {
            if !q.key_expr().intersects(key) {
                continue;
            }
            let _ = match &e.value {
                Some((bytes, enc)) => q
                    .reply(key.clone(), bytes.clone())
                    .encoding(enc.clone())
                    .attachment(e.attachment.clone())
                    .timestamp(e.ts)
                    .wait(),
                None => q.reply_del(key.clone()).timestamp(e.ts).wait(),
            };
        }
    }
}

/// What a member's writer last put, for its refresher.
struct Held {
    /// The payload and attachment of the last put, kept only when the
    /// resource has a horizon above 0; `None` after a delete.
    value: Option<(Vec<u8>, Option<Vec<u8>>)>,
    /// When the last put went out (a change or a re-put), or when a re-put
    /// the clock guard held was last tried.
    at: tokio::time::Instant,
}

/// One member's writer state, shared with its refresher. Puts take `held`
/// for their whole mint-record-publish, so a re-put never overtakes a
/// change with an older value.
struct Member {
    held: tokio::sync::Mutex<Held>,
    /// [`StateWriter::set_refresh`].
    refresh: AtomicBool,
    /// A put, a delete, or the refresh switched: the refresher looks again.
    wake: tokio::sync::Notify,
}

/// Everything the refresher of one member needs.
struct Refresher {
    member: Arc<Member>,
    writer: Arc<Writer>,
    store: Arc<Store>,
    minter: Arc<Minter>,
    key: OwnedKeyExpr,
    period: Duration,
    closed: tokio::sync::watch::Receiver<bool>,
}

impl Refresher {
    /// Re-puts the member whenever `period` passes without a put
    /// (`freshness.v1` §2.4), until the service closes.
    async fn run(mut self) {
        loop {
            if *self.closed.borrow() {
                return;
            }
            let due = {
                let h = self.member.held.lock().await;
                (self.member.refresh.load(Ordering::Relaxed) && h.value.is_some()).then_some(h.at)
            };
            let period = self.period;
            let sleep = async move {
                let Some(at) = due else {
                    return std::future::pending::<()>().await;
                };
                loop {
                    let elapsed = at.elapsed();
                    if elapsed >= period {
                        return;
                    }
                    tokio::time::sleep((period - elapsed).min(STEP)).await;
                }
            };
            tokio::select! {
                () = sleep => {}
                () = self.member.wake.notified() => continue,
                changed = self.closed.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue;
                }
            }
            self.once().await;
        }
    }

    /// One re-put, if it is still due.
    async fn once(&self) {
        let mut h = self.member.held.lock().await;
        if *self.closed.borrow()
            || !self.member.refresh.load(Ordering::Relaxed)
            || h.at.elapsed() < self.period
        {
            return;
        }
        let Some((bytes, attachment)) = h.value.clone() else {
            return;
        };
        match self.minter.mint() {
            Ok(ts) => {
                self.store.put(
                    &self.key,
                    bytes.clone(),
                    self.writer.encoding().clone(),
                    attachment.clone(),
                    ts,
                );
                h.at = tokio::time::Instant::now();
                if let Err(e) = self
                    .writer
                    .put_stamped(bytes.into(), attachment.map(Into::into), ts)
                    .await
                {
                    tracing::warn!(key = %self.key, "freshness.v1: a re-put failed: {e}");
                }
            }
            Err(_) => {
                // §4.3, freshness.v1 §2.10: the clock guard holds writes, so
                // the member goes stale by design. Try again shortly.
                let retry = HELD_RETRY.min(self.period);
                let now = tokio::time::Instant::now();
                h.at = now.checked_sub(self.period - retry).unwrap_or(now);
            }
        }
    }
}

/// A writer on one state member: every put and delete stamped by the owner
/// (S1), recorded for the owner's GET answers (S2), then published.
///
/// On a resource whose contract declares `freshness.ttl_s` above 0, it
/// re-puts the member's value unchanged whenever ttl/2 passes without a
/// put (the module doc's refresher). Dropping it stops that.
pub struct StateWriter {
    writer: Arc<Writer>,
    key: OwnedKeyExpr,
    store: Arc<Store>,
    minter: Arc<Minter>,
    member: Arc<Member>,
    /// The refresher's period: [`REFRESH_LEAD`] of ttl/2.
    period: Option<Duration>,
    refresher: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for StateWriter {
    fn drop(&mut self) {
        if let Some(t) = self.refresher.take() {
            t.abort();
        }
    }
}

impl StateWriter {
    pub(crate) fn new(
        writer: Writer,
        store: Arc<Store>,
        minter: Arc<Minter>,
        horizon: &Horizon,
        closed: tokio::sync::watch::Receiver<bool>,
    ) -> Self {
        let key = writer.key_expr().clone().into_owned();
        let key = OwnedKeyExpr::from(key);
        let writer = Arc::new(writer);
        let member = Arc::new(Member {
            held: tokio::sync::Mutex::new(Held {
                value: None,
                at: tokio::time::Instant::now(),
            }),
            refresh: AtomicBool::new(true),
            wake: tokio::sync::Notify::new(),
        });
        let period = horizon
            .refresh_bound()
            .map(|b| b.mul_f64(REFRESH_LEAD))
            .filter(|p| !p.is_zero());
        let refresher = period.and_then(|period| {
            let Ok(rt) = tokio::runtime::Handle::try_current() else {
                tracing::warn!(
                    key = %key,
                    "freshness.v1: no tokio runtime here, so this member is not re-put"
                );
                return None;
            };
            Some(
                rt.spawn(
                    Refresher {
                        member: Arc::clone(&member),
                        writer: Arc::clone(&writer),
                        store: Arc::clone(&store),
                        minter: Arc::clone(&minter),
                        key: key.clone(),
                        period,
                        closed,
                    }
                    .run(),
                ),
            )
        });
        Self {
            writer,
            key,
            store,
            minter,
            member,
            period,
            refresher,
        }
    }

    /// The member's key expression.
    #[must_use]
    pub fn key_expr(&self) -> &OwnedKeyExpr {
        &self.key
    }

    /// The underlying writer (QoS, `Encoding`, the zenoh publisher).
    #[must_use]
    pub fn writer(&self) -> &Writer {
        &self.writer
    }

    /// How often the member is re-put without a change (`freshness.v1`
    /// §2.4): [`REFRESH_LEAD`] of ttl/2, or `None` when the resource
    /// declares no horizon above 0.
    #[must_use]
    pub fn refresh_period(&self) -> Option<Duration> {
        self.period.filter(|_| self.refresher.is_some())
    }

    /// Whether the member is re-put now: it has a horizon above 0, and
    /// [`StateWriter::set_refresh`] has not turned it off.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.refresh_period().is_some() && self.member.refresh.load(Ordering::Relaxed)
    }

    /// Turns the re-puts off, or back on (`freshness.v1` §2.4, "only a value
    /// it holds current"). An owner that can no longer vouch for the value,
    /// because the source it reports went away, turns them off and lets the
    /// member go stale, rather than confirming a value it does not know to
    /// be current. Turned back on, a member whose interval has run out is
    /// re-put at once.
    pub fn set_refresh(&self, on: bool) {
        self.member.refresh.store(on, Ordering::Relaxed);
        self.member.wake.notify_one();
    }

    /// Puts a value (already in the contract's type), stamped. Returns the
    /// stamp.
    pub async fn put(&self, payload: impl Into<ZBytes>) -> Result<Timestamp> {
        self.put_with(payload, None::<ZBytes>).await
    }

    /// [`StateWriter::put`] with an attachment, encoded per the contract's
    /// `attachment` type and `attachment_encoding` (§2.3, §7.2; #698). The
    /// owner's GET answers carry it with the value (S2).
    pub async fn put_with(
        &self,
        payload: impl Into<ZBytes>,
        attachment: Option<impl Into<ZBytes>>,
    ) -> Result<Timestamp> {
        let mut held = self.member.held.lock().await;
        let ts = self.minter.mint()?;
        let payload: ZBytes = payload.into();
        let attachment: Option<ZBytes> = attachment.map(Into::into);
        let bytes = payload.to_bytes().into_owned();
        let kept = attachment.as_ref().map(|a| a.to_bytes().into_owned());
        if self.period.is_some() {
            held.value = Some((bytes.clone(), kept.clone()));
        }
        held.at = tokio::time::Instant::now();
        self.store
            .put(&self.key, bytes, self.writer.encoding().clone(), kept, ts);
        let sent = self.writer.put_stamped(payload, attachment, ts).await;
        drop(held);
        self.member.wake.notify_one();
        sent?;
        Ok(ts)
    }

    /// Puts a value of a JSON Schema type, encoded as the contract says.
    pub async fn put_value<T: Serialize>(&self, value: &T) -> Result<Timestamp> {
        let bytes = self.writer.encode(value)?;
        self.put(bytes).await
    }

    /// Deletes the member, stamped; GETs within the window answer it with
    /// `reply_del` (S3).
    pub async fn delete(&self) -> Result<Timestamp> {
        let mut held = self.member.held.lock().await;
        let ts = self.minter.mint()?;
        // A deleted member is not re-put (`freshness.v1` §2.4).
        held.value = None;
        held.at = tokio::time::Instant::now();
        self.store.delete(&self.key, ts);
        let sent = self.writer.delete_stamped(ts).await;
        drop(held);
        self.member.wake.notify_one();
        sent?;
        Ok(ts)
    }
}

/// One key of an owner's answer: current state (S6).
#[derive(Debug, Clone)]
pub enum Current {
    /// The key's value, as the owner holds it now.
    Value { key: String, sample: Box<Sample> },
    /// Deleted within the window, with the deletion's stamp.
    Deleted {
        key: String,
        timestamp: Option<Timestamp>,
    },
}

impl Current {
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Value { key, .. } | Self::Deleted { key, .. } => key,
        }
    }

    #[must_use]
    pub fn timestamp(&self) -> Option<Timestamp> {
        match self {
            Self::Value { sample, .. } => sample.timestamp().copied(),
            Self::Deleted { timestamp, .. } => *timestamp,
        }
    }

    /// The id of the clock that stamped it, the owner's session's zid when
    /// S1 holds: what a reader's clock trust is against (`freshness.v1`
    /// §2.6).
    #[must_use]
    pub fn clock(&self) -> Option<String> {
        self.timestamp().map(|t| t.get_id().to_string())
    }

    /// This reply as a `freshness.v1` observation (§2.6): its stamp aged
    /// against this host's clock now, under the reader's `clock` trust
    /// against [`Current::clock`]. Judge it with
    /// [`zenkey_model::freshness::judge`].
    #[must_use]
    pub fn freshness(&self, clock: ClockTrust) -> Observation {
        self.freshness_at(SystemTime::now(), clock)
    }

    /// [`Current::freshness`] at the instant `now`.
    #[must_use]
    pub fn freshness_at(&self, now: SystemTime, clock: ClockTrust) -> Observation {
        let reply = match self {
            Self::Value { sample, .. } => Reply::Put {
                stamp_age: sample
                    .timestamp()
                    .map(|t| StampAge::between(t.get_time().to_system_time(), now)),
            },
            Self::Deleted { .. } => Reply::Delete,
        };
        Observation::Got {
            reply: Some(reply),
            clock,
        }
    }
}

/// What a state GET to the owner found (S4, S6).
#[derive(Debug, Clone)]
pub enum StateGet {
    /// The owner answered: current state, key by key.
    Answered(Vec<Current>),
    /// No reply within the timeout. Not a verdict about the key (O5): the
    /// owner may be gone, or the reply may not have crossed. A consumer may
    /// now read an archive, whose answer is last-known (S6).
    Silent,
}

/// A state GET's answer with how it ended (§2.7, 0.24; #735): what
/// [`crate::consumer::Consumer::get_answer`] returns, for a reader that must
/// tell a complete reading from one that may miss a member.
#[derive(Debug, Clone, Default)]
pub struct StateAnswer {
    /// Every key answered, its value or its deletion. Empty is silence.
    pub current: Vec<Current>,
    /// Whether every GET ran to its final reply with no error reply: a GET
    /// that reached its timeout ends with one (Appendix B), and so may miss
    /// a member. A refused GET is complete and empty (§8.1, 0.8).
    pub complete: bool,
}

impl From<StateAnswer> for StateGet {
    fn from(a: StateAnswer) -> Self {
        if a.current.is_empty() {
            StateGet::Silent
        } else {
            StateGet::Answered(a.current)
        }
    }
}

/// Orders one owner's values (§4.3, the consumer rule): by time within one
/// timestamp id, accepting the first value under a new id and restarting
/// the ordering there. Per key.
#[derive(Debug, Default)]
pub struct ValueOrder {
    last: BTreeMap<String, Timestamp>,
}

impl ValueOrder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a value of `key` stamped `ts` is newer than what was applied:
    /// later within the same id, or the first under a new id. Records it if
    /// so.
    pub fn accept(&mut self, key: &str, ts: &Timestamp) -> bool {
        let newer = match self.last.get(key) {
            Some(last) if last.get_id() == ts.get_id() => ts.get_time() > last.get_time(),
            _ => true,
        };
        if newer {
            self.last.insert(key.to_owned(), *ts);
        }
        newer
    }
}
