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
//!
//! Last-known state (S5) is an archive's: [`crate::archive`].

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use zenoh::Wait;
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::Query;
use zenoh::sample::Sample;
use zenoh::time::{NTP64, Timestamp};

use crate::error::{Error, Result};
use crate::writer::Writer;

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
}

impl Minter {
    pub(crate) fn new(session: &zenoh::Session) -> Self {
        Self {
            session: session.clone(),
            last: Mutex::new(None),
            offset_ms: AtomicI64::new(0),
            ahead: AtomicBool::new(false),
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
                let ahead = mine.saturating_sub(theirs) > HLC_DELTA;
                if ahead && !minter.ahead.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        ahead_ms = mine.saturating_sub(theirs).as_millis() as u64,
                        "this owner's clock is ahead of its router beyond the HLC delta: \
                         state writes stop (spec §4.3)"
                    );
                } else if !ahead {
                    minter.ahead.store(false, Ordering::Relaxed);
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

    fn put(&self, key: &OwnedKeyExpr, bytes: Vec<u8>, encoding: Encoding, ts: Timestamp) {
        self.map.lock().expect("not poisoned").insert(
            key.clone(),
            Entry {
                value: Some((bytes, encoding)),
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
                    .timestamp(e.ts)
                    .wait(),
                None => q.reply_del(key.clone()).timestamp(e.ts).wait(),
            };
        }
    }
}

/// A writer on one state member: every put and delete stamped by the owner
/// (S1), recorded for the owner's GET answers (S2), then published.
pub struct StateWriter {
    writer: Writer,
    key: OwnedKeyExpr,
    store: Arc<Store>,
    minter: Arc<Minter>,
}

impl StateWriter {
    pub(crate) fn new(writer: Writer, store: Arc<Store>, minter: Arc<Minter>) -> Self {
        let key = writer.key_expr().clone().into_owned();
        let key = OwnedKeyExpr::from(key);
        Self {
            writer,
            key,
            store,
            minter,
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

    /// Puts a value (already in the contract's type), stamped. Returns the
    /// stamp.
    pub async fn put(&self, payload: impl Into<ZBytes>) -> Result<Timestamp> {
        let ts = self.minter.mint()?;
        let payload: ZBytes = payload.into();
        self.store.put(
            &self.key,
            payload.to_bytes().into_owned(),
            self.writer.encoding().clone(),
            ts,
        );
        self.writer.put_stamped(payload, ts).await?;
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
        let ts = self.minter.mint()?;
        self.store.delete(&self.key, ts);
        self.writer.delete_stamped(ts).await?;
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
