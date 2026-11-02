//! State (spec §4): an owner's mutations under its in-band stamps, its GET
//! answers with tombstones, the clock guard, and the consumer's owner-only
//! GET.
//!
//! | Rule | Here |
//! |---|---|
//! | S1 every put and delete carries its in-band stamp (§7.5's envelope), and on §4.3's table a Zenoh timestamp the owner set | [`StateWriter`], [`Minter`] |
//! | S2 GET replies are the bytes put, a `reply_del`'s attachment the delete's envelope, each with the mutation's Zenoh timestamp where it had one; the owner's state queryables answer every live key plus `reply_del` within the window | the state server ([`Store`]) |
//! | S3 the tombstone window, 60 s unless configured | [`DEFAULT_WINDOW`], `ServiceConfig::tombstone_window_s` |
//! | S4 a consumer's GET: the owner's keys, target `All`, consolidation `Latest` | [`crate::consumer::Consumer::get`] |
//! | S6 current is the owner's answer; silence is no verdict | [`StateGet`] |
//! | S7 a run never reused, a seq never wrapped within it; §4.3's table's timestamps, ahead: the guard holds a state write that carries a Zenoh timestamp | [`Minter`], [`crate::run`], the clock guard (`ServiceConfig::clock_reference`) |
//! | a re-put is a mutation (0.21); `freshness.v1` §2.4, §2.10 | [`StateWriter`]'s refresher |
//! | `freshness.v2` §2.4: a collection's members vouched for by a heartbeat, its replies carrying the last confirmation (§4.3's table, row 1) | [`Store`]'s books, [`crate::heartbeat`] |
//!
//! Last-known state (S5) is an archive's: [`crate::archive`].
//!
//! **Under `freshness.v2`** (#978's F3): a state member of a contract that
//! uses it is confirmed by its owner's heartbeats ([`crate::heartbeat`]),
//! never re-put: its writer runs no refresher, [`StateWriter::set_vouched`]
//! is its bit, and its puts carry no Zenoh timestamp (core §4.3's table
//! drops row 1 for it). The [`Store`] keeps each covered interface's
//! collections as a heartbeat reads them: each collection's digest,
//! toggled at each mutation, and each live member's bit; and each member's
//! last confirmation, the later of its last mutation and the last
//! heartbeat that vouched for it, which its GET replies carry as their
//! Zenoh timestamp (S2). On a session without timestamping the clock
//! guard holds none of its writes, since they carry no Zenoh timestamp
//! (§4.3, "Ahead"; #978's F6). On a session that stamps every write, a
//! `redundancy.v1` candidate's (§8.2), it holds them all, and a held write
//! clears its member's bit until a write of it goes through (`freshness.v2`
//! §2.10, F-461).
//!
//! **The refresher** (`freshness.v1` §2.4, #720). A state resource whose
//! contract declares `freshness.ttl_s` above 0 is confirmed at least every
//! ttl/2: a [`StateWriter`] on one of its members re-puts the member's
//! current value, unchanged, whenever that long passes without a put. The
//! re-put is a mutation like any other (core 0.21): a new seq and a fresh
//! Zenoh timestamp from the [`Minter`], recorded for the GET answers (S2),
//! then published. It is on
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
use zenkey_model::seed::Mark;
use zenkey_model::wire;
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

/// What a write was stamped with (core §4.3): its in-band stamp, the run
/// and seq its envelope carries, and the Zenoh timestamp the owner set on
/// it, only on a resource §4.3's table lists (`None` elsewhere: the owner
/// sets none, §8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamped {
    pub stamp: wire::Stamp,
    pub time: Option<Timestamp>,
}

impl Stamped {
    /// A put's payload under this stamp: §7.5's mutation, `record` after
    /// it, CH's tag 0.
    #[must_use]
    pub fn seal(&self, record: &[u8]) -> Vec<u8> {
        self.seal_tagged(wire::Tag::NONE, record)
    }

    /// [`Stamped::seal`] with `tag` in CH: a packed record's type's (core
    /// §7.6), 0 for every other.
    #[must_use]
    pub fn seal_tagged(&self, tag: wire::Tag, record: &[u8]) -> Vec<u8> {
        wire::Envelope {
            form: wire::Form::Mutation,
            tag,
            stamp: self.stamp,
            record,
        }
        .write()
    }

    /// A delete's attachment under this stamp: §7.5's delete form.
    #[must_use]
    pub fn delete_attachment(&self) -> Vec<u8> {
        wire::DeleteStamp { stamp: self.stamp }.write()
    }
}

/// How many stream keys a run numbers before the owner takes a new one
/// (core §4.3's MAY): each key's seq is kept for the whole run, so the map
/// is bounded by a run change rather than by forgetting a key.
pub const STREAM_KEYS: usize = 65_536;

/// One seq past the greatest: the state and event seq takes a new run
/// before it would reach this (core §4.3, "No wrap").
const SEQ_END: u32 = wire::Seq::MAX.get() + 1;

/// The run an owner writes under now, and its counts (core §4.3).
#[derive(Default)]
struct Runs {
    /// `None` until the first write, or [`Minter::run`]'s first ask.
    run: Option<wire::Run>,
    /// The next state or event seq of the run, from 0.
    seq: u32,
    /// The next seq of each stream key in the run, from 0, wrapping.
    streams: HashMap<OwnedKeyExpr, u32>,
    /// Every run this instance wrote under, so a random draw never repeats
    /// one (§4.3).
    used: Vec<wire::Run>,
}

/// Mints an owner's stamps (core §4.3). One per service.
///
/// - **The in-band stamp**: the run, from the service's [`crate::run::RunStore`]
///   or drawn at random, and the seq, one per run for every state mutation
///   and event occurrence and one per stream key. Before the state and
///   event seq would pass 16,777,215, or past [`STREAM_KEYS`] stream keys,
///   it takes a new run and puts the descriptor stating it, under its own
///   lock, before it hands out seq 0 of the new run.
/// - **The table's Zenoh timestamps**: the greater of
///   `Session::new_timestamp()` and the last issued plus one tick, the
///   session's zid as the id, on the resources §4.3's table lists. On a
///   session without timestamping, as an owner's runs (§8.2), that is the
///   wall clock with the zid.
/// - **The clock guard** ("Ahead", #978's F6): while it holds, a state
///   write that carries a Zenoh timestamp is refused, [`Error::ClockAhead`]:
///   one on §4.3's table (`timed`), or any on a session that stamps. Every
///   other state write goes on, a `freshness.v2` member's among them on a
///   session without timestamping, since a router has no time of the
///   owner's to re-stamp on it.
///
/// **Session posture** (core §8.2): a session with timestamping enabled
/// stamps every put and delete it is not given a timestamp for, so on one
/// every write is given the minter's, and its GET answers carry it (S2).
pub struct Minter {
    session: zenoh::Session,
    last: Mutex<Option<Timestamp>>,
    offset_ms: AtomicI64,
    ahead: AtomicBool,
    /// The guard's hold, for what reports it (`health.v1` §2.5): the offset
    /// last measured while it holds, `None` while it does not. Receivers
    /// are woken on a change of hold only; the offset is kept current.
    held: tokio::sync::watch::Sender<Option<Duration>>,
    /// Whether the session's timestamping is enabled (§8.2's posture).
    session_hlc: bool,
    /// The run and its seqs. Held while a run changes, the descriptor's
    /// put included, so nothing is written under a new run before it.
    /// Lock order: this, then the descriptor's ([`crate::service`]).
    runs: Mutex<Runs>,
    store: Mutex<Option<Arc<dyn crate::run::RunStore>>>,
    /// The descriptor, once the service started: put again at each run
    /// change within the instance (§3.3).
    served: Mutex<Option<std::sync::Weak<dyn RunListener>>>,
    /// The interfaces whose contracts use `freshness.v2`, by id: their
    /// states with a horizon leave §4.3's table (row 1), their heartbeats
    /// confirming them instead.
    covered: Mutex<std::collections::BTreeSet<String>>,
}

/// What hears a run change within an instance: the descriptor, which states
/// the run and is put again before anything is written under the new one
/// (core §3.3). Called under the minter's lock.
pub(crate) trait RunListener: Send + Sync {
    fn run_changed(&self, run: wire::Run);
}

/// Whether `session` runs with timestamping enabled, as zenoh reads its
/// configuration: `timestamping/enabled` for the session's mode, a
/// router's on by default and a peer's and a client's off (core §4.1).
pub(crate) fn timestamping(session: &zenoh::Session) -> bool {
    let config = session.config();
    let mode = config
        .get("mode")
        .ok()
        .and_then(|v| serde_json::from_str::<Option<String>>(&v).ok())
        .flatten()
        .unwrap_or_else(|| "peer".to_owned());
    let value = config
        .get("timestamping/enabled")
        .ok()
        .and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok())
        .unwrap_or(serde_json::Value::Null);
    let stated = match &value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Object(m) => m.get(&mode).and_then(serde_json::Value::as_bool),
        _ => None,
    };
    stated.unwrap_or(mode == "router")
}

impl Minter {
    pub(crate) fn new(session: &zenoh::Session) -> Self {
        Self {
            session: session.clone(),
            last: Mutex::new(None),
            offset_ms: AtomicI64::new(0),
            ahead: AtomicBool::new(false),
            held: tokio::sync::watch::Sender::new(None),
            session_hlc: timestamping(session),
            runs: Mutex::new(Runs::default()),
            store: Mutex::new(None),
            served: Mutex::new(None),
            covered: Mutex::new(std::collections::BTreeSet::new()),
        }
    }

    /// `iface`'s contract uses `freshness.v2` (#978's F3): its states with
    /// a horizon carry no Zenoh timestamp on their puts (core §4.3's
    /// table, row 1).
    pub(crate) fn cover(&self, iface: &zenkey_model::grammar::IfaceId) {
        self.covered
            .lock()
            .expect("not poisoned")
            .insert(iface.to_string());
    }

    /// Whether `key`'s interface's contract uses `freshness.v2`.
    pub(crate) fn covers(&self, key: &str) -> bool {
        key.split('/')
            .nth(3)
            .is_some_and(|i| self.covered.lock().expect("not poisoned").contains(i))
    }

    /// Whether the session's timestamping is enabled (core §8.2): then every
    /// write is given a Zenoh timestamp, the minter's.
    #[must_use]
    pub fn session_timestamping(&self) -> bool {
        self.session_hlc
    }

    /// Keeps `store` as the counter its 16-bit runs come from. Refused once
    /// a run is taken: the store must come before the first write.
    pub(crate) fn set_store(&self, store: Arc<dyn crate::run::RunStore>) -> Result<()> {
        if self.runs.lock().expect("not poisoned").run.is_some() {
            return Err(Error::Contract(
                "the run store comes before the first write: a run is taken already (core §4.3)"
                    .to_owned(),
            ));
        }
        *self.store.lock().expect("not poisoned") = Some(store);
        Ok(())
    }

    /// Whether a run store is configured.
    pub(crate) fn has_store(&self) -> bool {
        self.store.lock().expect("not poisoned").is_some()
    }

    /// Hears every run change from now on, the descriptor's put.
    pub(crate) fn listen(&self, l: std::sync::Weak<dyn RunListener>) {
        *self.served.lock().expect("not poisoned") = Some(l);
    }

    /// The run written under now, taken at this first ask if no write took
    /// it yet.
    pub fn run(&self) -> wire::Run {
        let mut r = self.runs.lock().expect("not poisoned");
        self.ensure(&mut r)
    }

    /// The as-of mark a reply carries now (core §5.1, "The as-of mark"):
    /// the run written under, and the last state or event seq minted under
    /// it, 0 when none was. A reply mints no seq, so two replies with no
    /// write between them carry one mark.
    pub fn mark(&self) -> wire::Stamp {
        let mut r = self.runs.lock().expect("not poisoned");
        let run = self.ensure(&mut r);
        let last = r.seq.saturating_sub(1).min(wire::Seq::MAX.get());
        wire::Stamp {
            run,
            seq: wire::Seq::new(last).expect("at most 2^24 - 1"),
        }
    }

    /// A run never used by this instance: the store's next counter, else a
    /// random 32-bit draw (§4.3).
    fn draw(&self, used: &[wire::Run]) -> wire::Run {
        let store = self.store.lock().expect("not poisoned").clone();
        if let Some(s) = store {
            match s.next() {
                Ok(Some(n)) if !used.contains(&wire::Run::U16(n)) => return wire::Run::U16(n),
                Ok(Some(n)) => tracing::warn!(
                    run = n,
                    "the run store gave a run this instance used: a 32-bit run is drawn (core §4.3)"
                ),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    "the run store could not be written, so this owner draws a 32-bit run \
                     (core §4.3): {e}"
                ),
            }
        }
        loop {
            let r = wire::Run::U32(rand::random());
            if !used.contains(&r) {
                return r;
            }
        }
    }

    fn ensure(&self, r: &mut Runs) -> wire::Run {
        if let Some(run) = r.run {
            return run;
        }
        let run = self.draw(&r.used);
        r.used.push(run);
        r.run = Some(run);
        run
    }

    /// A new run within the instance (§4.3): drawn, the descriptor put
    /// stating it, then every seq from 0. Under the runs lock.
    fn switch(&self, r: &mut Runs) {
        let run = self.draw(&r.used);
        r.used.push(run);
        let listener = self
            .served
            .lock()
            .expect("not poisoned")
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        if let Some(l) = listener {
            l.run_changed(run);
        }
        r.run = Some(run);
        r.seq = 0;
        r.streams.clear();
        tracing::info!(run = %run, "a new run (core §4.3)");
    }

    /// A new instance's run (§1.5, §4.3), drawn but not yet written under:
    /// its descriptor states it, and [`Minter::begin`] makes it current.
    pub(crate) fn next_run(&self) -> wire::Run {
        let mut r = self.runs.lock().expect("not poisoned");
        let run = self.draw(&r.used);
        r.used.push(run);
        run
    }

    /// Writes under `run` from now on, every seq from 0: a new instance's,
    /// once its descriptor states it.
    pub(crate) fn begin(&self, run: wire::Run) {
        let mut r = self.runs.lock().expect("not poisoned");
        r.run = Some(run);
        r.seq = 0;
        r.streams.clear();
    }

    /// The next state or event seq, a new run taken first when it would
    /// pass 16,777,215.
    fn next_seq(&self, r: &mut Runs) -> wire::Stamp {
        self.ensure(r);
        if r.seq >= SEQ_END {
            self.switch(r);
        }
        let seq = wire::Seq::new(r.seq).expect("below 2^24");
        r.seq += 1;
        wire::Stamp {
            run: r.run.expect("ensured"),
            seq,
        }
    }

    fn timed(&self, timed: bool) -> Option<Timestamp> {
        (timed || self.session_hlc).then(|| self.time())
    }

    /// A state mutation's stamps (S1): the next seq of the run, and a Zenoh
    /// timestamp when `timed` (§4.3's table) or the session stamps. Refused,
    /// [`Error::ClockAhead`], while the clock guard holds, only when it
    /// would carry that timestamp (§4.3, "Ahead"): an untimed write goes
    /// on.
    pub fn mint_state(&self, timed: bool) -> Result<Stamped> {
        if (timed || self.session_hlc) && self.ahead.load(Ordering::Relaxed) {
            return Err(Error::ClockAhead);
        }
        let mut r = self.runs.lock().expect("not poisoned");
        let stamp = self.next_seq(&mut r);
        Ok(Stamped {
            stamp,
            time: self.timed(timed),
        })
    }

    /// An event occurrence's stamps: the next seq of the run, shared with
    /// state's (§4.3). Never held by the clock guard, which stops timed state.
    pub fn mint_event(&self, timed: bool) -> Stamped {
        let mut r = self.runs.lock().expect("not poisoned");
        let stamp = self.next_seq(&mut r);
        Stamped {
            stamp,
            time: self.timed(timed),
        }
    }

    /// A stream sample's stamps: the next seq of `key` in the run, from 0
    /// at its first sample and wrapping modulo 2^24, never restarted within
    /// the run (§4.3), whoever declares its publisher again. A key past
    /// [`STREAM_KEYS`] in the run takes a new run first.
    pub fn mint_stream(&self, key: &OwnedKeyExpr, timed: bool) -> Stamped {
        self.mint_stream_n(key, 1, timed)
    }

    /// A frame's stamps (core §4.3, §7.7): `n` seqs of `key` in the run, the
    /// first its record 0's, so the key's next sample takes the seq after
    /// its last record's. Otherwise as [`Minter::mint_stream`].
    pub fn mint_stream_n(&self, key: &OwnedKeyExpr, n: u32, timed: bool) -> Stamped {
        let mut r = self.runs.lock().expect("not poisoned");
        self.ensure(&mut r);
        if !r.streams.contains_key(key) && r.streams.len() >= STREAM_KEYS {
            self.switch(&mut r);
        }
        let run = r.run.expect("ensured");
        let next = r.streams.entry(key.clone()).or_insert(0);
        let seq = wire::Seq::new(*next).expect("kept below 2^24");
        *next = (*next + n % SEQ_END) % SEQ_END;
        Stamped {
            stamp: wire::Stamp { run, seq },
            time: self.timed(timed),
        }
    }

    /// Gives back `stamp`, a seq of `key` minted for a put that was never
    /// sent, so the next put on the key takes it again and its readers see
    /// no gap in its seqs (core §4.3, `freshness.v2` §2.5): no sample ever
    /// carried it, so nothing restarts. Only while `stamp`'s run is the
    /// current one and nothing was minted on `key` since; otherwise nothing
    /// changes, and the gap stands. Returns whether it was given back.
    pub(crate) fn unmint_stream(&self, key: &OwnedKeyExpr, stamp: wire::Stamp) -> bool {
        self.unmint_stream_n(key, stamp, 1)
    }

    /// [`Minter::unmint_stream`] of a frame's `n` seqs from `stamp` on.
    pub(crate) fn unmint_stream_n(&self, key: &OwnedKeyExpr, stamp: wire::Stamp, n: u32) -> bool {
        let mut r = self.runs.lock().expect("not poisoned");
        if r.run != Some(stamp.run) {
            return false;
        }
        match r.streams.get_mut(key) {
            Some(next) if *next == (stamp.seq.get() + n % SEQ_END) % SEQ_END => {
                *next = stamp.seq.get();
                true
            }
            _ => false,
        }
    }

    /// Sets the next state and event seq: for tests of the run change
    /// before a wrap (spec scenarios state.md §7 step 3); a deployment
    /// never sets it. Values above 2^24 are taken as 2^24, which takes a
    /// new run at the next write.
    pub fn simulate_seq(&self, next: u32) {
        let mut r = self.runs.lock().expect("not poisoned");
        self.ensure(&mut r);
        r.seq = next.min(SEQ_END);
    }

    /// The next state and event seq of the run, with no Zenoh timestamp
    /// and never held by the clock guard: for a profile whose model mints
    /// its own stamps, its writes then putting them as minted
    /// ([`StateWriter::put_sealed`], [`crate::writer::EventWriter`]'s), each
    /// deciding its timestamp and its guard there (`config.v2`, #985 G2).
    pub(crate) fn mint_seq(&self) -> wire::Stamp {
        let mut r = self.runs.lock().expect("not poisoned");
        self.next_seq(&mut r)
    }

    /// Takes a new run within the instance now (core §4.3's MAY): the
    /// descriptor put stating it once the service runs, then every seq from
    /// 0. `config.v2`'s owner takes one before its token counter passes
    /// 65,535 (§2.6).
    pub(crate) fn new_run(&self) {
        let mut r = self.runs.lock().expect("not poisoned");
        self.ensure(&mut r);
        self.switch(&mut r);
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

    /// The next Zenoh timestamp of §4.3's table: the greater of the clock
    /// now and the last issued plus one tick. Never held by the guard.
    pub(crate) fn time(&self) -> Timestamp {
        let mut t = self.now();
        let mut last = self.last.lock().expect("not poisoned");
        if let Some(l) = *last
            && t.get_time() <= l.get_time()
        {
            t = Timestamp::new(*l.get_time() + 1, *t.get_id());
        }
        *last = Some(t);
        t
    }

    /// The next Zenoh timestamp of §4.3's table, or [`Error::ClockAhead`]
    /// while the clock guard has found this owner ahead of its router
    /// (§4.3). A state write takes its stamps from [`Minter::mint_state`].
    pub fn mint(&self) -> Result<Timestamp> {
        if self.ahead.load(Ordering::Relaxed) {
            return Err(Error::ClockAhead);
        }
        Ok(self.time())
    }

    /// Catch-up (§4.3): the last Zenoh timestamp this owner issued for its
    /// keys, read from its own persistent record or an archive on its own
    /// side, before its first write. Nothing of §4.3's table is then
    /// stamped at or below it.
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
    /// owner's timed state writes, with the offset it last measured, `None`
    /// while it does not. It wakes on a change of hold only (`health.v1`
    /// §2.5).
    pub(crate) fn held(&self) -> tokio::sync::watch::Receiver<Option<Duration>> {
        self.held.subscribe()
    }

    /// The guard measured this owner `offset` ahead of its router: beyond
    /// [`HLC_DELTA`] its timed state writes stop (§4.3). Returns whether
    /// this detection is the first of a hold.
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
/// stamped. Beyond [`HLC_DELTA`] the owner stops the state writes that
/// carry a Zenoh timestamp.
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
                         its timed state writes stop (spec §4.3)"
                    );
                } else if was && !minter.is_ahead() {
                    tracing::info!(
                        ahead_ms = offset.as_millis() as u64,
                        "this owner's clock is back within the HLC delta of its router: \
                         its timed state writes resume (spec §4.3)"
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
    /// The bytes put, envelope included, and their `Encoding`.
    value: Option<(Vec<u8>, Encoding)>,
    /// The value's attachment (§2.3), answered with it (S2).
    attachment: Option<Vec<u8>>,
    /// The mutation's stamps: a `reply_del`'s attachment is its in-band
    /// stamp, and every reply carries its Zenoh timestamp where it had one
    /// (S2).
    stamped: Stamped,
    at: Instant,
    /// A `freshness.v2` member's last confirmation (v2 §2.4): the later of
    /// its last mutation and the last heartbeat that vouched for it, which
    /// its replies carry in place of the mutation's Zenoh timestamp (S2,
    /// core §4.3's table). `None` for any other.
    confirmed: Option<Timestamp>,
}

impl Entry {
    /// The Zenoh timestamp its replies carry (S2).
    fn reply_time(&self) -> Option<Timestamp> {
        self.confirmed.or(self.stamped.time)
    }
}

/// One covered interface's collections, as its heartbeats read them
/// (`freshness.v2` §2.4, core §7.5).
struct Book {
    layout: Arc<zenkey_model::heartbeat::Layout>,
    collections: Vec<Collection>,
}

/// One collection: its digest over its live members, each with its bit.
#[derive(Default)]
struct Collection {
    digest: zenkey_model::heartbeat::Digest,
    /// Base-relative key → (its stamp, whether it is vouched for).
    live: BTreeMap<String, (wire::Stamp, bool)>,
}

/// What a heartbeat vouched for: each member's key and the stamp it held,
/// so a confirmation never lands on a value put after it was read.
pub(crate) type Vouched = Vec<(String, wire::Stamp)>;

/// How many distinct selectors a [`Store`] counts the GETs of; a GET on a
/// selector past them is counted apart, as reaching no key.
const ASKED_SELECTORS: usize = 1024;

/// The owner's state, as its queryables answer it (S2, S3).
pub struct Store {
    map: Mutex<HashMap<OwnedKeyExpr, Entry>>,
    window: Duration,
    /// The GETs answered, by their selector, at most [`ASKED_SELECTORS`]
    /// of them: what [`Store::asked_reaching`] counts (`desired.v1`'s
    /// scenarios §8, text 0.4).
    asked: Mutex<HashMap<OwnedKeyExpr, u64>>,
    /// Each interface whose contract uses `freshness.v2`, by id: its
    /// collections as its heartbeats read them. Lock order: `map`, then
    /// this.
    books: Mutex<BTreeMap<String, Book>>,
}

impl Store {
    pub(crate) fn new(window: Duration) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            window,
            asked: Mutex::new(HashMap::new()),
            books: Mutex::new(BTreeMap::new()),
        }
    }

    /// Keeps `layout`'s interface's collections from now on: its contract
    /// uses `freshness.v2` (v2 §2.4). Before any of its members is put.
    pub(crate) fn cover(&self, layout: Arc<zenkey_model::heartbeat::Layout>) {
        let n = layout.collections.len();
        self.books.lock().expect("not poisoned").insert(
            layout.iface.to_string(),
            Book {
                layout,
                collections: (0..n).map(|_| Collection::default()).collect(),
            },
        );
    }

    /// Whether `key` is a member of a collection a heartbeat covers.
    pub(crate) fn covers(&self, key: &str) -> bool {
        let books = self.books.lock().expect("not poisoned");
        Self::collection_in(&books, key).is_some()
    }

    fn collection_in<'b>(books: &'b BTreeMap<String, Book>, key: &str) -> Option<(&'b str, usize)> {
        let iface = key.split('/').nth(3)?;
        let (id, b) = books.get_key_value(iface)?;
        Some((id.as_str(), b.layout.collection_of(key)?))
    }

    /// A member's digest, in or out, and its bit: `old` the stamp of its
    /// live value before (`None` when it held none), `new` the live
    /// value's now (`None` after a delete).
    fn toggle(&self, key: &str, old: Option<wire::Stamp>, new: Option<(wire::Stamp, bool)>) {
        let mut books = self.books.lock().expect("not poisoned");
        let Some((iface, i)) = Self::collection_in(&books, key).map(|(a, i)| (a.to_owned(), i))
        else {
            return;
        };
        let c = &mut books.get_mut(&iface).expect("found").collections[i];
        if let Some(o) = old {
            c.digest.toggle(key, o);
        }
        match new {
            Some((stamp, vouched)) => {
                c.digest.toggle(key, stamp);
                c.live.insert(key.to_owned(), (stamp, vouched));
            }
            None => {
                c.live.remove(key);
            }
        }
    }

    /// Sets a covered member's bit (`freshness.v2` §2.4): it vouches for
    /// its value, or no longer does.
    pub(crate) fn vouch(&self, key: &str, on: bool) {
        let mut books = self.books.lock().expect("not poisoned");
        let Some((iface, i)) = Self::collection_in(&books, key).map(|(a, i)| (a.to_owned(), i))
        else {
            return;
        };
        if let Some(m) = books.get_mut(&iface).expect("found").collections[i]
            .live
            .get_mut(key)
        {
            m.1 = on;
        }
    }

    /// `iface`'s heartbeat body now (core §7.5): a section for each
    /// collection `exposed` marks, its digest and its members' bits in key
    /// order. Returns it with what it vouched for, or `None` for an
    /// interface it does not cover.
    pub(crate) fn heartbeat(
        &self,
        iface: &str,
        exposed: &[bool],
        time: zenkey_model::heartbeat::Time,
    ) -> Option<(zenkey_model::heartbeat::Heartbeat, Vouched)> {
        use zenkey_model::heartbeat::{Heartbeat, Members, Section};
        let books = self.books.lock().expect("not poisoned");
        let b = books.get(iface)?;
        let mut vouched = Vec::new();
        let sections = b
            .layout
            .collections
            .iter()
            .zip(&b.collections)
            .enumerate()
            .map(|(i, (layout, c))| {
                if !exposed.get(i).copied().unwrap_or(false) {
                    return None;
                }
                let bits: Vec<bool> = c.live.values().map(|(_, v)| *v).collect();
                for ((k, (stamp, _)), on) in c.live.iter().zip(&bits) {
                    if *on {
                        vouched.push((k.clone(), *stamp));
                    }
                }
                let members = if layout.is_templated() {
                    Members::templated(
                        bits.len() as u64,
                        bits.iter()
                            .enumerate()
                            .filter(|(_, on)| **on)
                            .map(|(r, _)| r as u64),
                    )
                } else {
                    Members::One {
                        vouched: bits.first().copied().unwrap_or(false),
                    }
                };
                Some(Section {
                    digest: c.digest,
                    members,
                })
            })
            .collect();
        Some((Heartbeat { time, sections }, vouched))
    }

    /// A heartbeat put at `at` vouched for `vouched`: each member still
    /// holding the stamp it was vouched for at is confirmed then, and its
    /// replies carry `at` (`freshness.v2` §2.4, S2).
    pub(crate) fn confirm(&self, vouched: &Vouched, at: Timestamp) {
        let mut map = self.map.lock().expect("not poisoned");
        for (key, stamp) in vouched {
            let Ok(k) = OwnedKeyExpr::try_from(key.as_str()) else {
                continue;
            };
            if let Some(e) = map.get_mut(&k)
                && e.value.is_some()
                && e.stamped.stamp == *stamp
                && e.confirmed.is_none_or(|c| c < at)
            {
                e.confirmed = Some(at);
            }
        }
    }

    /// How many GETs this store answered whose selector reaches `key`,
    /// whether or not it held a value for it: a lower bound once more than
    /// [`ASKED_SELECTORS`] selectors were asked.
    pub(crate) fn asked_reaching(&self, key: &zenoh::key_expr::keyexpr) -> u64 {
        self.asked
            .lock()
            .expect("not poisoned")
            .iter()
            .filter(|(s, _)| s.intersects(key))
            .map(|(_, n)| n)
            .sum()
    }

    /// Records a put. A covered member's (`freshness.v2`) is vouched for
    /// or not as `vouched` says, and confirmed at `confirmed`, the
    /// mutation's time (v2 §2.4).
    #[allow(clippy::too_many_arguments)]
    fn put(
        &self,
        key: &OwnedKeyExpr,
        bytes: Vec<u8>,
        encoding: Encoding,
        attachment: Option<Vec<u8>>,
        stamped: Stamped,
        vouched: bool,
        confirmed: Option<Timestamp>,
    ) {
        let mut map = self.map.lock().expect("not poisoned");
        let old = map.insert(
            key.clone(),
            Entry {
                value: Some((bytes, encoding)),
                attachment,
                stamped,
                at: Instant::now(),
                confirmed,
            },
        );
        let old = old.and_then(|e| e.value.is_some().then_some(e.stamped.stamp));
        self.toggle(key.as_str(), old, Some((stamped.stamp, vouched)));
    }

    /// A delete recorded `age` ago: its tombstone is answered for what is
    /// left of the window (S3), as a delete made then would be. A covered
    /// member's (`freshness.v2`) `reply_del` carries `confirmed`, the
    /// timestamp minted at the delete, its last confirmation (S2, core
    /// §4.3's "Minting").
    fn delete_aged(
        &self,
        key: &OwnedKeyExpr,
        stamped: Stamped,
        age: Duration,
        confirmed: Option<Timestamp>,
    ) {
        let now = Instant::now();
        let mut map = self.map.lock().expect("not poisoned");
        let old = map.insert(
            key.clone(),
            Entry {
                value: None,
                attachment: None,
                stamped,
                at: now.checked_sub(age).unwrap_or(now),
                confirmed,
            },
        );
        let old = old.and_then(|e| e.value.is_some().then_some(e.stamped.stamp));
        self.toggle(key.as_str(), old, None);
    }

    /// Forgets every key under `prefix`, values and tombstones: a governed
    /// interface's state once its instance steps down (`redundancy.v1`
    /// §2.9), so that a later takeover answers only what it puts again
    /// (§2.6).
    pub(crate) fn forget_prefix(&self, prefix: &str) {
        let mut map = self.map.lock().expect("not poisoned");
        let gone: Vec<(String, Option<wire::Stamp>)> = map
            .iter()
            .filter(|(k, _)| k.as_str().starts_with(prefix))
            .map(|(k, e)| {
                (
                    k.as_str().to_owned(),
                    e.value.is_some().then_some(e.stamped.stamp),
                )
            })
            .collect();
        map.retain(|k, _| !k.as_str().starts_with(prefix));
        for (k, old) in gone {
            self.toggle(&k, old, None);
        }
    }

    /// Answers a query: every matching live key with the bytes it put, its
    /// envelope included, and a `reply_del` whose attachment is the
    /// deletion's envelope for each matching key deleted within the window,
    /// each with the mutation's Zenoh timestamp where it carried one, a
    /// covered member's last confirmation in its place (S2, S3). Older
    /// tombstones are forgotten.
    pub(crate) fn answer(&self, q: &Query) {
        {
            let mut asked = self.asked.lock().expect("not poisoned");
            let selector: &zenoh::key_expr::keyexpr = q.key_expr();
            if let Some(n) = asked.get_mut(selector) {
                *n += 1;
            } else if asked.len() < ASKED_SELECTORS {
                asked.insert(selector.to_owned(), 1);
            }
        }
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
                    .timestamp(e.reply_time())
                    .wait(),
                None => q
                    .reply_del(key.clone())
                    .attachment(e.stamped.delete_attachment())
                    .timestamp(e.reply_time())
                    .wait(),
            };
        }
    }
}

/// What a member's writer last put, for its refresher.
struct Held {
    /// The record and attachment of the last put, kept only when the
    /// resource has a horizon above 0; `None` after a delete. A re-put
    /// seals the record under a new stamp (§4.2, "A re-put is a mutation").
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

/// A governed interface's state member forgets its value when its instance
/// steps down (`redundancy.v1` §2.9): nothing is re-put, and a later
/// takeover starts from what the application puts (§2.6).
impl crate::gate::Slot for Member {
    fn expose(&self) -> crate::call::BoxFuture<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }

    fn unexpose(&self) -> crate::call::BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.held.lock().await.value = None;
            Ok(())
        })
    }
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

    /// One re-put, if it is still due, and only while its governed
    /// interface serves (`redundancy.v1` §2.4).
    async fn once(&self) {
        let Ok(admitted) = self.writer.serving().await else {
            return;
        };
        let mut h = self.member.held.lock().await;
        if *self.closed.borrow()
            || !self.member.refresh.load(Ordering::Relaxed)
            || h.at.elapsed() < self.period
        {
            return;
        }
        let Some((record, attachment)) = h.value.clone() else {
            return;
        };
        match self.minter.mint_state(self.writer.is_stamped()) {
            Ok(stamped) => {
                let bytes = stamped.seal_tagged(self.writer.tag(), &record);
                self.store.put(
                    &self.key,
                    bytes.clone(),
                    self.writer.encoding().clone(),
                    attachment.clone(),
                    stamped,
                    true,
                    None,
                );
                h.at = tokio::time::Instant::now();
                if let Err(e) = self
                    .writer
                    .put_stamped(&admitted, bytes.into(), attachment.map(Into::into), stamped)
                    .await
                {
                    tracing::warn!(key = %self.key, "freshness.v1: a re-put failed: {e}");
                }
            }
            Err(_) => {
                // §4.3, freshness.v1 §2.10: the clock guard holds a re-put,
                // which carries a Zenoh timestamp, so the member goes stale
                // by design. Try again shortly.
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
/// put (the module doc's refresher). Dropping it stops that. Under
/// `freshness.v2` it re-puts nothing: its owner's heartbeat vouches for
/// the member while [`StateWriter::set_vouched`] leaves its bit set.
///
/// Made by its service, it keeps the resource's budget (core §2.7, #741):
/// a put of a key without a value while the live members are at the bound
/// is refused, [`Error::Budget`], nothing stored or published; a put to a
/// live key adds no member, and a delete frees one.
///
/// On a metric resource it holds its puts to `telemetry.v1` as
/// [`Writer`] does ([`crate::telemetry`]), [`Error::Telemetry`] before
/// anything is minted, stored or published. A re-put is the value last
/// put, unchanged, its epoch the one it was put in; a delete forgets no
/// last value (the module doc's).
pub struct StateWriter {
    writer: Arc<Writer>,
    key: OwnedKeyExpr,
    store: Arc<Store>,
    minter: Arc<Minter>,
    member: Arc<Member>,
    /// The refresher's period: [`REFRESH_LEAD`] of ttl/2.
    period: Option<Duration>,
    /// [`REFRESH_LEAD`] of ttl/2 whoever confirms the member: the
    /// refresher, or under `freshness.v2` its owner's heartbeat.
    confirm: Option<Duration>,
    refresher: Option<tokio::task::JoinHandle<()>>,
    /// Whether it keeps its last value: a horizon above 0, so a re-put
    /// ([`StateWriter::reput`]) can put it again.
    keep: bool,
    /// Whether a heartbeat vouches for the member (`freshness.v2` §2.4):
    /// its contract uses the profile, and the resource is a collection.
    covered: bool,
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
        if let Some(g) = writer.gate() {
            g.hold(Arc::downgrade(&member) as std::sync::Weak<dyn crate::gate::Slot>);
        }
        // freshness.v2 §2.4: a member a heartbeat covers is never re-put.
        let covered = store.covers(key.as_str());
        let keep = horizon.refresh_bound().is_some();
        let confirm = horizon
            .refresh_bound()
            .map(|b| b.mul_f64(REFRESH_LEAD))
            .filter(|p| !p.is_zero());
        let period = confirm.filter(|_| !covered);
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
            confirm,
            refresher,
            keep,
            covered,
        }
    }

    /// A write's stamps (S1), from [`Minter::mint_state`]. Where the clock
    /// guard holds the write of a covered member, its bit is cleared until
    /// a write of it goes through (`freshness.v2` §2.10, F-461): only a
    /// session that stamps every write, a `redundancy.v1` candidate's (core
    /// §8.2), has such a write held, and the heartbeat that goes on must
    /// not vouch for the value the owner asked to replace or delete. The
    /// next put that goes through sets the bit as
    /// [`StateWriter::set_vouched`] left it, and so does that call itself.
    fn mint(&self) -> Result<Stamped> {
        let minted = self.minter.mint_state(self.writer.is_stamped());
        if self.covered && matches!(minted, Err(Error::ClockAhead)) {
            self.store.vouch(self.key.as_str(), false);
        }
        minted
    }

    /// The Zenoh timestamp a covered member's replies carry from a
    /// mutation stamped so (`freshness.v2` §2.4, S2): the mutation's own
    /// where it set one, else the owner's clock now. `None` for a member no
    /// heartbeat covers, whose replies carry the mutation's.
    fn confirmed_at(&self, stamped: &Stamped) -> Option<Timestamp> {
        self.covered
            .then(|| stamped.time.unwrap_or_else(|| self.minter.time()))
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

    /// The longest a member goes unconfirmed while it is vouched for
    /// (`freshness.v1` and `freshness.v2` §2.4): [`REFRESH_LEAD`] of ttl/2.
    /// Under `freshness.v1` it is the refresher's period. Under
    /// `freshness.v2` the member's heartbeat beats at most that far apart,
    /// at its interface's shortest horizon. `None` for a member with no
    /// horizon above 0, or one under `freshness.v1` with no runtime to
    /// re-put it.
    #[must_use]
    pub fn confirm_period(&self) -> Option<Duration> {
        if self.covered {
            self.confirm
        } else {
            self.refresh_period()
        }
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
    /// re-put at once. Under `freshness.v2` it is the member's bit:
    /// [`StateWriter::set_vouched`].
    pub fn set_refresh(&self, on: bool) {
        self.set_vouched(on);
    }

    /// Vouches for the member's value, or stops (`freshness.v1` and
    /// `freshness.v2` §2.4, "only a value it holds current"). Under
    /// `freshness.v2` it is the member's bit in its owner's heartbeat: set,
    /// each heartbeat confirms the value; clear, none does, and the member
    /// goes stale from its last confirmation. Under `freshness.v1` it
    /// turns the re-puts on and off ([`StateWriter::set_refresh`]): one
    /// switch, which the refresher reads alone, the store's bit being kept
    /// only for a member a heartbeat covers. On by default, and kept
    /// across puts. A write of a covered member that the clock guard holds
    /// clears its bit, not this switch, until a write of it goes through
    /// (`freshness.v2` §2.10).
    pub fn set_vouched(&self, on: bool) {
        self.member.refresh.store(on, Ordering::Relaxed);
        self.store.vouch(self.key.as_str(), on);
        self.member.wake.notify_one();
    }

    /// The switch [`StateWriter::set_vouched`] set: under `freshness.v2`,
    /// whether the member's bit is set in its owner's heartbeats; under
    /// `freshness.v1`, whether its re-puts are on (the refresher reads this
    /// switch alone, [`StateWriter::is_refreshing`]). On a resource with no
    /// horizon above 0 it vouches for nothing either way.
    #[must_use]
    pub fn is_vouched(&self) -> bool {
        self.member.refresh.load(Ordering::Relaxed)
    }

    /// Whether a heartbeat confirms the member (`freshness.v2` §2.4): its
    /// contract uses the profile, and its resource is a collection.
    #[must_use]
    pub fn is_covered(&self) -> bool {
        self.covered
    }

    /// Puts a value (already in the contract's type), its record sealed in
    /// §7.5's envelope under the owner's stamps (S1). Returns them.
    pub async fn put(&self, payload: impl Into<ZBytes>) -> Result<Stamped> {
        self.put_with(payload, None::<ZBytes>).await
    }

    /// [`StateWriter::put`] with an attachment, encoded per the contract's
    /// `attachment` type and `attachment_encoding` (§2.3, §7.2; #698). The
    /// owner's GET answers carry it with the value (S2).
    pub async fn put_with(
        &self,
        payload: impl Into<ZBytes>,
        attachment: Option<impl Into<ZBytes>>,
    ) -> Result<Stamped> {
        // redundancy.v1 §2.4: a standby writes nothing of the interface.
        let admitted = self.writer.serving().await?;
        let mut held = self.member.held.lock().await;
        // telemetry.v1 (#819): a metric sample is checked before anything
        // is minted, stored or published, its epoch written in.
        let (payload, attachment, pending) = self
            .writer
            .meter(payload.into(), attachment.map(Into::into))
            .await?;
        // Core §7.6 (#979): a packed record its layout refuses is refused
        // before anything is minted, stored or published.
        self.writer.admit_record(&payload.to_bytes())?;
        let stamped = self.mint()?;
        // §2.7 (#741): a new live member at the bound is refused, nothing
        // stored or published. After the mint, so a stopped clock never
        // leaves a member counted live without a value.
        self.writer.admit()?;
        let record = payload.to_bytes().into_owned();
        let bytes = stamped.seal_tagged(self.writer.tag(), &record);
        let kept = attachment.as_ref().map(|a| a.to_bytes().into_owned());
        if self.keep {
            held.value = Some((record, kept.clone()));
        }
        held.at = tokio::time::Instant::now();
        self.store.put(
            &self.key,
            bytes.clone(),
            self.writer.encoding().clone(),
            kept,
            stamped,
            self.is_vouched(),
            self.confirmed_at(&stamped),
        );
        let sent = self
            .writer
            .put_stamped(&admitted, bytes.into(), attachment, stamped)
            .await;
        drop(held);
        self.member.wake.notify_one();
        sent?;
        if let Some(p) = pending {
            p.commit();
        }
        Ok(stamped)
    }

    /// Puts `payload`, a put's payload already sealed in §7.5's envelope
    /// under `stamp` by a profile's model, which minted the seq through
    /// [`Minter`] (`config.v2`, #985 G2: the owner writes what its model's
    /// output says, byte for byte). Otherwise as [`StateWriter::put`]: the
    /// record checked by its layout, the budget, the store, and a Zenoh
    /// timestamp only where the writer stamps, which the clock guard holds.
    pub(crate) async fn put_sealed(&self, stamp: wire::Stamp, payload: Vec<u8>) -> Result<Stamped> {
        let admitted = self.writer.serving().await?;
        let mut held = self.member.held.lock().await;
        let record = wire::Envelope::parse(&payload)
            .map_err(|r| Error::Payload(format!("not a put's envelope: {}", r.reason())))?
            .record
            .to_vec();
        self.writer.admit_record(&record)?;
        let timed = self.writer.is_stamped();
        if timed && self.minter.is_ahead() {
            if self.covered {
                self.store.vouch(self.key.as_str(), false);
            }
            return Err(Error::ClockAhead);
        }
        let stamped = Stamped {
            stamp,
            time: timed.then(|| self.minter.time()),
        };
        self.writer.admit()?;
        if self.keep {
            held.value = Some((record, None));
        }
        held.at = tokio::time::Instant::now();
        self.store.put(
            &self.key,
            payload.clone(),
            self.writer.encoding().clone(),
            None,
            stamped,
            self.is_vouched(),
            self.confirmed_at(&stamped),
        );
        let sent = self
            .writer
            .put_stamped(&admitted, payload.into(), None, stamped)
            .await;
        drop(held);
        self.member.wake.notify_one();
        sent?;
        Ok(stamped)
    }

    /// The resource's `telemetry.v1` declaration, when it is a metric
    /// resource whose puts this writer checks ([`crate::telemetry`]).
    #[must_use]
    pub fn telemetry(&self) -> Option<&zenkey_model::telemetry::Declared> {
        self.writer.telemetry()
    }

    /// The tag CH carries on its records: its packed type's (core §7.6),
    /// else 0.
    #[must_use]
    pub fn tag(&self) -> wire::Tag {
        self.writer.tag()
    }

    /// The deployment epoch its packed records' `time` fields count from,
    /// in seconds (core §7.6, §8.6).
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.writer.deployment_epoch()
    }

    /// Re-puts the member's current value now, unchanged, under a fresh
    /// stamp: a confirmation out of turn (`alarms.v1` §2.9's re-mint). It
    /// takes the value under the member's lock, so it never puts an older
    /// value over a newer one. `None`, nothing put, when the writer keeps no
    /// value: after a delete, or on a resource with no horizon above 0.
    pub(crate) async fn reput(&self) -> Result<Option<Stamped>> {
        let admitted = self.writer.serving().await?;
        let mut held = self.member.held.lock().await;
        let Some((record, attachment)) = held.value.clone() else {
            return Ok(None);
        };
        let stamped = self.mint()?;
        let bytes = stamped.seal_tagged(self.writer.tag(), &record);
        self.store.put(
            &self.key,
            bytes.clone(),
            self.writer.encoding().clone(),
            attachment.clone(),
            stamped,
            self.is_vouched(),
            self.confirmed_at(&stamped),
        );
        held.at = tokio::time::Instant::now();
        let sent = self
            .writer
            .put_stamped(&admitted, bytes.into(), attachment.map(Into::into), stamped)
            .await;
        drop(held);
        self.member.wake.notify_one();
        sent?;
        Ok(Some(stamped))
    }

    /// Puts a value of a JSON Schema type, encoded as the contract says.
    pub async fn put_value<T: Serialize>(&self, value: &T) -> Result<Stamped> {
        let bytes = self.writer.encode(value)?;
        self.put(bytes).await
    }

    /// Deletes the member, its attachment §7.5's delete envelope under the
    /// owner's stamps (S1); GETs within the window answer it with
    /// `reply_del` carrying them (S2, S3).
    pub async fn delete(&self) -> Result<Stamped> {
        self.delete_aged(Duration::ZERO).await
    }

    /// [`StateWriter::delete`] of a delete first made `age` ago, made again
    /// by a new instance (`desired.v1` §2.11): fresh stamps, and its
    /// tombstone answered for what is left of the window (S3), not a window
    /// more.
    pub(crate) async fn delete_aged(&self, age: Duration) -> Result<Stamped> {
        let admitted = self.writer.serving().await?;
        let mut held = self.member.held.lock().await;
        let stamped = self.mint()?;
        // A deleted member is not re-put (`freshness.v1` §2.4).
        held.value = None;
        held.at = tokio::time::Instant::now();
        self.store
            .delete_aged(&self.key, stamped, age, self.confirmed_at(&stamped));
        // §2.7: a deleted key is not live, so it frees a member.
        self.writer.release();
        let sent = self.writer.delete_stamped(&admitted, stamped).await;
        drop(held);
        self.member.wake.notify_one();
        sent?;
        Ok(stamped)
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

    /// The id of the clock that stamped it: on a resource core §4.3's table
    /// lists, the owner's session's zid when S1 holds; elsewhere a
    /// router's, the reader's own, or none. What a reader's clock trust is
    /// against (`freshness.v1` §2.6).
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
    /// Every key answered, its value or its deletion. Empty is silence. A
    /// value that carried a zk2c envelope (core §7.5) holds its record as
    /// its payload.
    pub current: Vec<Current>,
    /// Whether every GET ran to its final reply with no error reply: a GET
    /// that reached its timeout ends with one (Appendix B), and so may miss
    /// a member. A refused GET is complete and empty (§8.1, 0.8).
    pub complete: bool,
    /// The in-band stamp of each key answered under a zk2c envelope, a
    /// value's or a deletion's: what orders it ([`StateAnswer::mark`]).
    pub inband: BTreeMap<String, zenkey_model::wire::Stamp>,
    /// The CH tag of each value answered under a zk2c envelope: a packed
    /// record's type's (core §7.6), which the consumer admitted it by.
    pub tags: BTreeMap<String, zenkey_model::wire::Tag>,
    /// Each key answered with an envelope refused, or a packed record its
    /// layout refused (core §7.5, §7.6): undecodable, with the refusal,
    /// and left out of [`StateAnswer::current`].
    pub undecodable: BTreeMap<String, crate::inband::Undecodable>,
}

impl StateAnswer {
    /// What orders `current` among its key's values: its in-band stamp,
    /// else its Zenoh timestamp ([`ValueOrder::accept_mark`]).
    #[must_use]
    pub fn mark(&self, current: &Current) -> Option<Mark> {
        Mark::of(
            self.inband.get(current.key()).copied(),
            current.timestamp().as_ref().map(crate::inband::stamp_of),
        )
    }
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

/// Orders one owner's values (core §4.3, the consumer rule), per key, by
/// their in-band stamp: by seq within one run, accepting a value under a
/// run other than the one the key's held value carries, and ordering by
/// seq within that run from then on. A consumer hands it every value from
/// its first on, a late joiner's seed replies included (§4.2), so an old
/// run's value that arrives after the new run's is accepted too, and the
/// key steps back: §4.3's residual risk.
///
/// A value with no envelope, one that broke S1, is ordered by its Zenoh
/// timestamp: by time within one timestamp id, a value under another id
/// accepted as under another run, and so is one beside a value held under
/// an in-band stamp (`zenkey_model::seed::consumer_accepts`). Either way
/// the mark is the value's own ([`crate::consumer::Delivery::mark`],
/// [`StateAnswer::mark`]): an enveloped value's Zenoh timestamp is its
/// time where a profile reads one, never its order.
#[derive(Debug, Default)]
pub struct ValueOrder {
    last: BTreeMap<String, Mark>,
}

impl ValueOrder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a value of `key` marked `mark` is accepted over what was
    /// applied: a later seq within the held value's run (a later time
    /// within its timestamp id, for a value with no envelope), or a value
    /// under another run, another id or the other wire. Records it if so.
    pub fn accept_mark(&mut self, key: &str, mark: Mark) -> bool {
        let newer = zenkey_model::seed::consumer_accepts(self.last.get(key).copied(), mark);
        if newer {
            self.last.insert(key.to_owned(), mark);
        }
        newer
    }
}
