//! A follower of a governed interface (`redundancy.v2` §2.10, §2.17; v1
//! §2.10; #987's N2): a consumer that accepts the interface's samples only
//! from the instance serving it, its incumbent.
//!
//! The rule is `zenkey_model::redundancy`'s [`Fence`]; this module feeds it
//! from the bus and does what it asks, deciding nothing itself.
//!
//! ```text
//! Consumer::follow(resource, on_event) ─▶ Following
//!   watch session: the providers' interface tokens ─▶ Holding ─▶ the incumbent (the lowest holder)
//!   descriptors heard put, and read             ─▶ each instance's runs, major, meta.zid
//!   the resource's samples (a subscription)     ─▶ Fence::offer ─▶ accepted │ discarded │ held
//!   held, and no read running                   ─▶ one read of the address's descriptors ─▶ Fence::read_ended
//!   the incumbent changed                       ─▶ a seed: S4's GET, each reply judged, the incumbent's merged
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | v2 §2.10 the incumbent: the watch, a complete read, the holder's descriptor | a liveliness subscriber on each provider's `@zk/alive/<iface>/**` on the consumer's watch session ([`Consumer::with_watch`], refused without one), its round trip, then a read of each provider's tokens on the consumer's own session; a read again [`BURST`] after a withdrawal; each holder's descriptor read when none is held |
//! | observer §3.6 (0.38): a read never undoes a token heard since it was asked | each presence read asks the fence first ([`Fence::ask`]) and carries its [`zenkey_model::redundancy::Asked`] back, so its answer applies to no instance the watch heard declared or withdrawn while it ran: B's token heard at a takeover stands over a read again asked after A's withdrawal that came back without it (round 128's F-629) |
//! | v2 §2.10 each instance's runs, as its descriptors state them | every descriptor put heard on the providers' `@zk/instance/*`, and every one read, through [`zenkey_model::redundancy::read_descriptor`], each read by its form through the consumer's catalogs and contract (core §3.3: a compact one as its JSON twin, one not read never guessed): its run kept beside those before. Its major is the revision's its entry names: the consumer's own where the fingerprints agree, else the one major of `redundancy` its `profiles` lists (core §3.3: the `uses` of what it implements), else unknown and read by run |
//! | v2 §2.10 the rule, by run; v1 §2.10 by `meta.zid` under a `redundancy.v1` incumbent | each sample's in-band stamp and Zenoh timestamp's id to [`Fence::offer`], a state's by its key ([`Member::Key`]), any other alone |
//! | v2 §2.10 an unattributed sample held for one read of the descriptors asked after it, one read at a time | each read the fence asks: a GET of the address's `@zk/instance/*` on the consumer's session, its end handed back ([`Fence::read_ended`]) |
//! | v2 §2.10 a change of incumbent: seed again, merge only the incumbent's snapshots, the omission rule | a state's S4 GET of that provider ([`Consumer::get_answer`]'s, through a gate): each reply judged by its run (a snapshot's page by its header's), an unattributed one decided after one read of the descriptors; what is admitted merged into the view by core §4.3's consumer rule, and each member of the provider's a complete snapshot of the incumbent's lists neither live nor as a tombstone dropped, unless it was put under the snapshot's run past its asof |
//! | v2 §2.10 "A heartbeat": the heartbeats that cover the interface, read under the incumbent's runs alone | for a collection followed by heartbeats (`freshness.v2`), each heartbeat covering the interface and each owner heartbeat of its class, once admitted, offered to the fence by its envelope's stamp, held alone ([`Member::Alone`]): read into the subscription's view only when accepted, now or once a read of the descriptors attributes it; another run's, or one still unattributed when that read ends, never read, so it is neither a loss nor a divergence ([`FollowEvent::HeartbeatDiscarded`]) |
//!
//! **What it holds.** For a state, the address's word: each member's value
//! as the incumbent's samples and seeds left it ([`Following::value`]).
//! For a stream or an event, nothing: each accepted sample is handed over,
//! and a held one is handed over once decided. The subscription's views
//! ([`Following::subscription`]) hold what the incumbent's heartbeats
//! confirmed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey, parse};
use zenkey_model::redundancy::{
    self as model, Described, Discard, Election, Fence, Fenced, Governance, Holding, Major, Member,
    Outcome,
};
use zenkey_model::seed::Mark;
use zenkey_model::wire::{Run, Stamp};
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::{Sample, SampleKind};

use crate::consumer::{Consumer, Delivery, ReplyGate, Subscription};
use crate::error::{Error, Result, zenoh};
use crate::state::Current;

/// How long a read of presence or of descriptors waits, by default.
pub const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// How long a seed's GET waits, by default (core §8.1: the caller's).
pub const SEED_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a read after a withdrawal waits for the rest of its burst.
pub const BURST: Duration = Duration::from_millis(100);

/// How a follower runs.
#[derive(Debug, Clone)]
pub struct FollowStartup {
    /// Every read's timeout: presence, a descriptor, the address's
    /// descriptors.
    pub timeout: Duration,
    /// A seed's GET's timeout.
    pub seed_timeout: Duration,
    /// A test's hook (`scenarios-v2.md` §14): each descriptor put heard is
    /// handled this long after it arrived. Reads are not delayed.
    #[doc(hidden)]
    pub descriptor_delay: Duration,
    /// A test's hook (`scenarios-v2.md` §13, observer §3.6): each read
    /// again after a withdrawal is applied this long after its answer came
    /// back, as an answer that the watch's news overtook.
    #[doc(hidden)]
    pub reread_hold: Duration,
}

impl Default for FollowStartup {
    fn default() -> Self {
        Self {
            timeout: READ_TIMEOUT,
            seed_timeout: SEED_TIMEOUT,
            descriptor_delay: Duration::ZERO,
            reread_hold: Duration::ZERO,
        }
    }
}

/// What a follower tells, in the order it decided it.
#[derive(Debug, Clone)]
pub enum FollowEvent {
    /// The incumbent it follows at `provider` changed: the lowest holder
    /// of the interface's token, `None` with none seen.
    Incumbent {
        provider: Addr,
        incumbent: Option<InstanceId>,
    },
    /// A sample the rule accepted: live, or from a seed.
    Accepted {
        delivery: Box<Delivery>,
        seeded: bool,
    },
    /// A sample the rule discarded, with its reason.
    Discarded {
        delivery: Box<Delivery>,
        why: Discard,
    },
    /// A sample held unattributed, until a read of the address's
    /// descriptors asked after it ends (v2 §2.10).
    Held { delivery: Box<Delivery> },
    /// A held state member's value replaced by a newer one, or a newer one
    /// not kept beside it: within one run the higher seq is kept, the held
    /// one at an equal seq, across two the later arrival.
    Superseded { delivery: Box<Delivery> },
    /// A reply of a seed's GET the rule did not admit: a snapshot's page by
    /// its header's run, a reply per key by its own.
    SeedDiscarded {
        provider: Addr,
        key: String,
        snapshot: bool,
        run: Option<Run>,
        why: Discard,
    },
    /// A member that left the view: a complete snapshot of the incumbent's
    /// lists it neither live nor as a tombstone (core §4.2).
    Omitted { provider: Addr, key: String },
    /// A heartbeat covering the interface, or an owner heartbeat, on `key`,
    /// the rule accepted: the subscription's view read it (v2 §2.10).
    Heartbeat {
        provider: Addr,
        key: String,
        stamp: Stamp,
    },
    /// One the rule discarded, with its reason: another instance's run, or
    /// a run still unattributed when a read of the descriptors ended. The
    /// view never read it: no heartbeat of the incumbent's, neither a loss
    /// nor a divergence (v2 §2.10).
    HeartbeatDiscarded {
        provider: Addr,
        key: String,
        stamp: Stamp,
        why: Discard,
    },
    /// One held unattributed, until a read of the address's descriptors
    /// asked after it ends (v2 §2.10).
    HeartbeatHeld {
        provider: Addr,
        key: String,
        stamp: Stamp,
    },
    /// A seed made, after the incumbent at `provider` changed: whether its
    /// GET was complete, and how many snapshots of the incumbent's it
    /// merged.
    Seeded {
        provider: Addr,
        incumbent: Option<InstanceId>,
        complete: bool,
        snapshots: usize,
    },
}

/// What a follower has decided so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FollowCounts {
    /// Samples accepted, live and seeded.
    pub accepted: u64,
    /// Samples and seed replies discarded, by the rule's reason.
    pub discarded: BTreeMap<&'static str, u64>,
    /// Samples held unattributed.
    pub held: u64,
    pub superseded: u64,
    pub omitted: u64,
    /// Reads of an address's descriptors made for a held sample.
    pub reads: u64,
    /// Seeds made.
    pub seeds: u64,
    /// Heartbeats the rule accepted, which the subscription's view read.
    pub heartbeats: u64,
    /// Heartbeats the rule discarded, by its reason.
    pub heartbeats_discarded: BTreeMap<&'static str, u64>,
    /// Heartbeats held unattributed.
    pub heartbeats_held: u64,
}

/// What the fence holds of one sample: a delivery of the resource, or a
/// heartbeat covering it, its view's read to make once accepted.
enum Item {
    Data(Box<Delivery>),
    Beat(Beat),
}

/// A heartbeat the fence holds or decides (v2 §2.10).
struct Beat {
    provider: Addr,
    key: String,
    stamp: Stamp,
    /// The subscription's view reads it.
    read: Box<dyn FnOnce() + Send>,
}

/// A heartbeat as the fence reads it (v2 §2.10's table: a heartbeat's run
/// is its envelope's): its in-band stamp, and its Zenoh timestamp's id,
/// which a `redundancy.v1` incumbent's is read by.
pub(crate) fn beat_sample(s: &Sample, stamp: Stamp) -> model::Sample {
    model::Sample {
        stamp: Some(stamp),
        timestamp_id: s.timestamp().and_then(crate::timing::stamp_id),
    }
}

/// The follower's gate before its subscription's heartbeats
/// ([`crate::consumer::BeatGate`]): each offered to the fence, read into
/// the view only once accepted.
struct FollowGate {
    core: Arc<Mutex<Core>>,
}

impl crate::consumer::BeatGate for FollowGate {
    fn beat(
        &self,
        provider: Addr,
        key: String,
        sample: model::Sample,
        read: Box<dyn FnOnce() + Send>,
    ) {
        let Some(stamp) = sample.stamp else {
            return;
        };
        let mut c = lock(&self.core);
        let beat = Beat {
            provider: provider.clone(),
            key: key.clone(),
            stamp,
            read,
        };
        let out = c
            .fence
            .offer(&provider, sample, Member::Alone, Item::Beat(beat));
        if out.held {
            c.counts.heartbeats_held += 1;
            c.emit(FollowEvent::HeartbeatHeld {
                provider: provider.clone(),
                key,
                stamp,
            });
        }
        c.outcome(&provider, out);
    }
}

fn lock(core: &Mutex<Core>) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A seed of one address: running, and asked again meanwhile.
#[derive(Debug, Default)]
struct Seeding {
    again: bool,
}

/// The follower's state, behind one lock.
struct Core {
    fence: Fence<Item>,
    iface: IfaceId,
    /// The consumer's revision's fingerprint, as a descriptor's entry
    /// spells it, and the major it uses.
    own_fp: String,
    own_major: Option<Major>,
    /// Whether the resource is a state: then each member is held by its
    /// key, and the view kept.
    state: bool,
    view: BTreeMap<String, Delivery>,
    counts: FollowCounts,
    events: tokio::sync::mpsc::UnboundedSender<FollowEvent>,
    reads: tokio::sync::mpsc::UnboundedSender<Addr>,
    describe: tokio::sync::mpsc::UnboundedSender<(Addr, InstanceId)>,
    seeds: tokio::sync::mpsc::UnboundedSender<Addr>,
    rereads: tokio::sync::mpsc::UnboundedSender<Addr>,
    asked: BTreeSet<(Addr, InstanceId)>,
    seeding: BTreeMap<Addr, Seeding>,
}

impl Core {
    fn emit(&self, e: FollowEvent) {
        let _ = self.events.send(e);
    }

    /// What one descriptor of the interface says to the fence (v2 §2.10).
    fn described(&self, v: &Value) -> Described {
        let iface = self.iface.to_string();
        let read = model::read_descriptor(v, &iface, Governance::Governed(Election::Claim));
        let zid = v
            .get("meta")
            .and_then(|m| m.get("zid"))
            .and_then(Value::as_str)
            .and_then(crate::timing::zid_value);
        Described {
            major: self.major_of(v, &iface),
            run: read.run,
            zid,
        }
    }

    /// The major of the revision a descriptor's entry names (v2 §2.17):
    /// the consumer's own where the fingerprints agree, else the one major
    /// of `redundancy` the descriptor's `profiles` lists, else unknown.
    fn major_of(&self, v: &Value, iface: &str) -> Option<Major> {
        let fp = v
            .get("interfaces")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|e| e.get("iface").and_then(Value::as_str) == Some(iface))
            .and_then(|e| e.get("contract"))
            .and_then(Value::as_str);
        if fp == Some(self.own_fp.as_str()) {
            return self.own_major;
        }
        let listed: BTreeSet<Major> = v
            .get("profiles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str()?.parse::<IfaceId>().ok())
            .filter(|p| p.name() == model::NAME)
            .filter_map(|p| Major::from_number(p.major()))
            .collect();
        match (listed.len(), listed.first()) {
            (1, Some(m)) => Some(*m),
            _ => None,
        }
    }

    /// Does what one call to the fence decided.
    fn outcome(&mut self, addr: &Addr, out: Outcome<Item>) {
        for item in out.accepted {
            match item {
                Item::Data(d) => self.accept(*d, false),
                Item::Beat(b) => {
                    self.counts.heartbeats += 1;
                    (b.read)();
                    self.emit(FollowEvent::Heartbeat {
                        provider: b.provider,
                        key: b.key,
                        stamp: b.stamp,
                    });
                }
            }
        }
        for (item, why) in out.discarded {
            match item {
                Item::Data(d) => {
                    *self.counts.discarded.entry(why.as_str()).or_default() += 1;
                    self.emit(FollowEvent::Discarded { delivery: d, why });
                }
                Item::Beat(b) => {
                    *self
                        .counts
                        .heartbeats_discarded
                        .entry(why.as_str())
                        .or_default() += 1;
                    self.emit(FollowEvent::HeartbeatDiscarded {
                        provider: b.provider,
                        key: b.key,
                        stamp: b.stamp,
                        why,
                    });
                }
            }
        }
        for item in out.superseded {
            // A heartbeat is held alone, never superseded.
            if let Item::Data(d) = item {
                self.counts.superseded += 1;
                self.emit(FollowEvent::Superseded { delivery: d });
            }
        }
        if out.asks_read {
            let _ = self.reads.send(addr.clone());
        }
    }

    fn accept(&mut self, d: Delivery, seeded: bool) {
        self.counts.accepted += 1;
        if self.state {
            self.hold_value(&d);
        }
        self.emit(FollowEvent::Accepted {
            delivery: Box::new(d),
            seeded,
        });
    }

    /// The view by core §4.3's consumer rule: a value older than the one
    /// held within its run is not taken.
    fn hold_value(&mut self, d: &Delivery) {
        let key = d.sample.key_expr().as_str().to_owned();
        let held = self.view.get(&key).and_then(Delivery::mark);
        if let (Some(_), Some(m)) = (held, d.mark())
            && !zenkey_model::seed::consumer_accepts(held, m)
        {
            return;
        }
        if d.sample.kind() == SampleKind::Delete {
            self.view.remove(&key);
        } else {
            self.view.insert(key, d.clone());
        }
    }

    /// A presence event, and what a change of incumbent asks: a seed, and
    /// each holder's descriptor not held.
    fn presence(&mut self, h: Holding) {
        let addr = match &h {
            Holding::Declared(a, _) | Holding::Withdrawn(a, _) | Holding::Read(a, ..) => a.clone(),
        };
        let withdrawn = matches!(h, Holding::Withdrawn(..));
        let before = self.fence.incumbent(&addr).cloned();
        self.fence.hear(h, false);
        let after = self.fence.incumbent(&addr).cloned();
        if before != after {
            self.emit(FollowEvent::Incumbent {
                provider: addr.clone(),
                incumbent: after.clone(),
            });
            if after.is_some() {
                let _ = self.seeds.send(addr.clone());
            }
        }
        if withdrawn {
            let _ = self.rereads.send(addr);
        }
        for k in self.fence.undescribed() {
            if self.asked.insert(k.clone()) {
                let _ = self.describe.send(k);
            }
        }
    }
}

/// A follower's tasks, aborted when dropped.
#[derive(Default)]
struct Tasks(Vec<tokio::task::JoinHandle<()>>);

impl Drop for Tasks {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

/// A live follower ([`Consumer::follow`]). Dropping it stops it.
pub struct Following {
    core: Arc<Mutex<Core>>,
    subscription: Subscription,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
    _tasks: Tasks,
}

impl std::fmt::Debug for Following {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Following")
            .field("counts", &self.counts())
            .finish_non_exhaustive()
    }
}

impl Following {
    /// The incumbent it follows at `provider`, `None` with none seen.
    #[must_use]
    pub fn incumbent(&self, provider: &Addr) -> Option<InstanceId> {
        lock(&self.core).fence.incumbent(provider).cloned()
    }

    /// A state member's value as the follower holds it: the address's word
    /// (v2 §2.10). `None` for none, and for a resource that is no state.
    #[must_use]
    pub fn value(&self, key: &str) -> Option<Delivery> {
        lock(&self.core).view.get(key).cloned()
    }

    /// Every state member it holds, by key.
    #[must_use]
    pub fn values(&self) -> BTreeMap<String, Delivery> {
        lock(&self.core).view.clone()
    }

    /// What it decided so far.
    #[must_use]
    pub fn counts(&self) -> FollowCounts {
        lock(&self.core).counts.clone()
    }

    /// The subscription it follows through.
    #[must_use]
    pub fn subscription(&self) -> &Subscription {
        &self.subscription
    }
}

/// A seed's gate (v2 §2.10): each reply judged by the fence, an
/// unattributed one after one read of the address's descriptors asked once
/// the GET ended.
struct SeedGate {
    core: Arc<Mutex<Core>>,
    addr: Addr,
    session: zenoh::Session,
    held: crate::presence::Held,
    timeout: Duration,
}

/// A reply as the fence reads it: a snapshot's page by its header's run
/// (its asof in the seq's place), any other by its own stamp. `None` for
/// one the read refuses or reads otherwise: a held stamp, a page whose
/// body is refused, a refused envelope.
fn reply_sample(s: &Sample) -> Option<model::Sample> {
    if crate::inband::held(s).is_some() {
        return None;
    }
    let timestamp_id = s.timestamp().and_then(crate::timing::stamp_id);
    let stamp = match crate::inband::read(s.clone()) {
        crate::inband::Read::Snapshot { page: Ok(page), .. } => Some(Stamp {
            run: page.run,
            seq: page.asof,
        }),
        crate::inband::Read::Snapshot { page: Err(_), .. }
        | crate::inband::Read::Refused { .. } => return None,
        r => r.stamp(),
    };
    Some(model::Sample {
        stamp,
        timestamp_id,
    })
}

impl SeedGate {
    fn discard(&self, s: &Sample, fs: &model::Sample, why: Discard) {
        let mut c = lock(&self.core);
        *c.counts.discarded.entry(why.as_str()).or_default() += 1;
        c.emit(FollowEvent::SeedDiscarded {
            provider: self.addr.clone(),
            key: s.key_expr().as_str().to_owned(),
            snapshot: crate::inband::is_snapshot(s),
            run: fs.run(),
            why,
        });
    }
}

impl ReplyGate for SeedGate {
    fn admit<'a>(&'a self, replies: Vec<Sample>) -> crate::call::BoxFuture<'a, Vec<Sample>> {
        Box::pin(async move {
            let mut admitted = Vec::new();
            let mut pending = Vec::new();
            for s in replies {
                let Some(fs) = reply_sample(&s) else {
                    admitted.push(s);
                    continue;
                };
                let judged = lock(&self.core).fence.judge(&self.addr, &fs, false);
                match judged {
                    Fenced::Accepted => admitted.push(s),
                    Fenced::Discarded(why) => self.discard(&s, &fs, why),
                    Fenced::Pending => pending.push((s, fs)),
                }
            }
            if pending.is_empty() {
                return admitted;
            }
            // v2 §2.10: one read of the address's descriptors, asked after
            // these replies arrived; each descriptor learnt (deciding a live
            // sample it states the run of), then these decided, an
            // unattributed one discarded.
            let read = read_descriptors(&self.session, &self.addr, &self.held, self.timeout).await;
            let mut c = lock(&self.core);
            for (i, v) in read {
                let d = c.described(&v);
                let out = c.fence.describe(&self.addr, i, d);
                c.outcome(&self.addr, out);
            }
            let decided: Vec<(Sample, model::Sample, Fenced)> = pending
                .into_iter()
                .map(|(s, fs)| {
                    let f = c.fence.judge(&self.addr, &fs, true);
                    (s, fs, f)
                })
                .collect();
            drop(c);
            for (s, fs, f) in decided {
                match f {
                    Fenced::Accepted => admitted.push(s),
                    Fenced::Discarded(why) => self.discard(&s, &fs, why),
                    Fenced::Pending => self.discard(&s, &fs, Discard::Unattributed),
                }
            }
            admitted
        })
    }
}

/// Every descriptor `addr` answers on `session` (v2 §2.10's read of the
/// address's descriptors): each instance's, as JSON, read by its form
/// through what `held` holds, a compact one's revisions retrieved where it
/// lacks them (core §3.3, [`crate::presence::read_held`]). One not read is
/// left out, never guessed.
pub(crate) async fn read_descriptors(
    session: &zenoh::Session,
    addr: &Addr,
    held: &crate::presence::Held,
    timeout: Duration,
) -> Vec<(InstanceId, Value)> {
    let selector = format!("zk2/{}/{}/@zk/instance/*", addr.system, addr.service);
    let Ok(rx) = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .with(flume::unbounded::<zenoh::query::Reply>())
        .await
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Ok(reply) = rx.recv_async().await {
        let Ok(s) = reply.into_result() else { continue };
        if s.kind() != SampleKind::Put {
            continue;
        }
        let Ok(ZkKey::Instance { addr: a, instance }) = parse(s.key_expr().as_str()) else {
            continue;
        };
        if a != *addr {
            continue;
        }
        let served = crate::presence::Served::of(&s);
        let found = crate::presence::read_held(session, served, &a, &instance, held, timeout).await;
        if let Some(v) = found.json() {
            out.push((instance, v));
        }
    }
    out
}

/// A descriptor put's address, instance and JSON, read by its form through
/// what `held` holds, here and now ([`crate::presence::heard`]): a compact
/// one naming a revision `held` lacks is not read here, and the holder's
/// descriptor GET, which retrieves it, reads it.
fn descriptor_of(s: &Sample, held: &crate::presence::Held) -> Option<(Addr, InstanceId, Value)> {
    let (addr, instance, found) = crate::presence::heard(s, held)?;
    Some((addr, instance, found.json()?))
}

/// The holders of `iface`'s token a read's keys show, by address.
fn holders_by_addr(keys: &[String], iface: &IfaceId) -> BTreeMap<Addr, BTreeSet<InstanceId>> {
    let mut out: BTreeMap<Addr, BTreeSet<InstanceId>> = BTreeMap::new();
    for k in keys {
        if let Ok(ZkKey::Alive {
            addr,
            iface: i,
            instance,
            ..
        }) = parse(k)
            && i == *iface
        {
            out.entry(addr).or_default().insert(instance);
        }
    }
    out
}

/// A read of `selector`'s interface tokens on `session`, which holds no
/// liveliness subscriber (core §8.1), handed to the fence `hold` after its
/// answer (a test's hook): each address it shows, and `exact`'s with none
/// held too. The fence is asked first, so the read applies to no token the
/// watch hears while it runs (observer §3.6).
async fn read_holders(
    session: &zenoh::Session,
    core: &Mutex<Core>,
    selector: &str,
    exact: Option<&Addr>,
    timeout: Duration,
    hold: Duration,
) {
    let asked = lock(core).fence.ask();
    let Ok(read) = crate::presence::liveliness_read(session, selector, timeout).await else {
        return;
    };
    if !hold.is_zero() {
        tokio::time::sleep(hold).await;
    }
    let iface = lock(core).iface.clone();
    let mut by = holders_by_addr(&read.keys, &iface);
    if let Some(a) = exact {
        by.entry(a.clone()).or_default();
    }
    let mut c = lock(core);
    for (a, held) in by {
        c.presence(Holding::Read(a, asked, read.complete, held));
    }
}

/// The merge of a seed's answer into the view (v2 §2.10): each value the
/// incumbent's replies brought, by core §4.3's consumer rule, then the
/// omission rule over each complete snapshot of the incumbent's.
fn merge_seed(
    c: &mut Core,
    consumer: &Consumer,
    resource: &zenkey_model::contract::Resource,
    name: &str,
    addr: &Addr,
    answer: &crate::state::StateAnswer,
) {
    let premise = consumer.premise(resource);
    let now = Instant::now();
    let mut listed = BTreeSet::new();
    for cur in &answer.current {
        let key = cur.key().to_owned();
        listed.insert(key.clone());
        let mark = answer.inband.get(&key).copied().map(Mark::InBand);
        match cur {
            Current::Value { sample, .. } => {
                let values = match parse(&key) {
                    Ok(ZkKey::Data {
                        resource: chunks, ..
                    }) => {
                        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                        resource.template.matches(&refs).unwrap_or_default()
                    }
                    _ => Default::default(),
                };
                c.accept(
                    Delivery {
                        provider: addr.clone(),
                        resource: name.to_owned(),
                        values,
                        sample: (**sample).clone(),
                        arrival: now,
                        stamp: sample.timestamp().copied(),
                        inband: answer.inband.get(&key).copied(),
                        tag: answer.tags.get(&key).copied(),
                        lifespan: premise,
                        frame: None,
                        occurred: None,
                    },
                    true,
                );
            }
            Current::Deleted { .. } => {
                let held = c.view.get(&key).and_then(Delivery::mark);
                if held.is_some()
                    && mark.is_none_or(|m| zenkey_model::seed::consumer_accepts(held, m))
                {
                    c.view.remove(&key);
                }
            }
        }
    }
    for snap in answer.snapshots.iter().filter(|h| h.complete()) {
        let dropped: Vec<String> = c
            .view
            .iter()
            .filter(|(k, d)| {
                d.provider == *addr
                    && zenkey_model::collection::is_member(resource, k)
                    && !listed.contains(*k)
                    && !matches!(d.mark(), Some(Mark::InBand(s))
                        if s.run == snap.run && s.seq > snap.asof)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in dropped {
            c.view.remove(&k);
            c.counts.omitted += 1;
            c.emit(FollowEvent::Omitted {
                provider: addr.clone(),
                key: k,
            });
        }
    }
}

/// One address's seeds, one at a time: each after the incumbent changed,
/// another made once it ends when it changed again meanwhile.
async fn seed_address(
    core: Arc<Mutex<Core>>,
    consumer: Consumer,
    resource: String,
    addr: Addr,
    st: FollowStartup,
) {
    let Ok(r) = consumer.resource(&resource).cloned() else {
        return;
    };
    loop {
        let incumbent = lock(&core).fence.incumbent(&addr).cloned();
        if let Some(inc) = &incumbent {
            // Its descriptor first, where none is held, so that its
            // snapshot is attributed by the runs it states.
            let held = !lock(&core)
                .fence
                .undescribed()
                .contains(&(addr.clone(), inc.clone()));
            if !held
                && let Some(v) = crate::presence::descriptor_held(
                    consumer.session(),
                    &addr,
                    inc,
                    &consumer.held(),
                    st.timeout,
                )
                .await
                .ok()
                .and_then(|f| f.json())
            {
                let mut c = lock(&core);
                let d = c.described(&v);
                let out = c.fence.describe(&addr, inc.clone(), d);
                c.outcome(&addr, out);
            }
            let gate = SeedGate {
                core: Arc::clone(&core),
                addr: addr.clone(),
                session: consumer.session().clone(),
                held: consumer.held(),
                timeout: st.timeout,
            };
            let bound = consumer.bound_to(&addr);
            match bound.get_gated(&resource, &gate, st.seed_timeout).await {
                Ok(answer) => {
                    let mut c = lock(&core);
                    c.counts.seeds += 1;
                    merge_seed(&mut c, &consumer, &r, &resource, &addr, &answer);
                    c.emit(FollowEvent::Seeded {
                        provider: addr.clone(),
                        incumbent: incumbent.clone(),
                        complete: answer.complete,
                        snapshots: answer.snapshots.len(),
                    });
                }
                Err(e) => tracing::warn!(provider = %addr, "a follower's seed was not made: {e}"),
            }
        }
        let mut c = lock(&core);
        let again = c
            .seeding
            .get_mut(&addr)
            .is_some_and(|s| std::mem::take(&mut s.again));
        if !again {
            c.seeding.remove(&addr);
            return;
        }
    }
}

impl Consumer {
    /// Follows the incumbent of each bound provider's governed interface
    /// (`redundancy.v2` §2.10, §2.17; v1 §2.10; [`crate::follow`]),
    /// subscribing to `resource` and handing `on_event` what the rule
    /// decides of each sample, in order, from a task of its own: accepted
    /// only under one of the incumbent's runs (by its `meta.zid`, under a
    /// `redundancy.v1` incumbent), an unattributed one held for one read of
    /// the descriptors, and, for a state, the provider read again by S4
    /// each time its incumbent changes, only the incumbent's snapshots
    /// merged.
    ///
    /// Refused without a watch session ([`Consumer::with_watch`]), and for
    /// an interface whose revision here neither major of `redundancy`
    /// governs. It returns once the first reads are made.
    pub async fn follow<C>(&self, resource: &str, on_event: C) -> Result<Following>
    where
        C: Fn(FollowEvent) + Send + Sync + 'static,
    {
        self.follow_with(resource, FollowStartup::default(), on_event)
            .await
    }

    /// [`Consumer::follow`], run as `st` says.
    pub async fn follow_with<C>(
        &self,
        resource: &str,
        st: FollowStartup,
        on_event: C,
    ) -> Result<Following>
    where
        C: Fn(FollowEvent) + Send + Sync + 'static,
    {
        let own_major = Major::of(&self.contract().uses);
        if own_major.is_none() {
            return Err(Error::Contract(format!(
                "{}: no major of redundancy governs it, so there is no incumbent to follow \
                 (redundancy.v2 §2.10)",
                self.interface()
            )));
        }
        let Some(watch) = self.watch_session().cloned() else {
            return Err(Error::Contract(
                "redundancy.v2 §2.10: a follower learns its incumbent by a watch on its \
                 providers' tokens, on a session of its own: Consumer::with_watch"
                    .to_owned(),
            ));
        };
        let r = self.resource(resource)?.clone();
        let state = matches!(
            r.token,
            zenkey_model::grammar::KindToken::State
                | zenkey_model::grammar::KindToken::ExplicitState
        );
        let iface = self.interface().clone();
        let (events, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
        let (reads, mut reads_rx) = tokio::sync::mpsc::unbounded_channel::<Addr>();
        let (describe, mut describe_rx) =
            tokio::sync::mpsc::unbounded_channel::<(Addr, InstanceId)>();
        let (seeds, mut seeds_rx) = tokio::sync::mpsc::unbounded_channel::<Addr>();
        let (rereads, mut rereads_rx) = tokio::sync::mpsc::unbounded_channel::<Addr>();
        let core = Arc::new(Mutex::new(Core {
            fence: Fence::default(),
            iface: iface.clone(),
            own_fp: zenkey_model::canonical::Fingerprint::of(self.contract()).to_string(),
            own_major,
            state,
            view: BTreeMap::new(),
            counts: FollowCounts::default(),
            events,
            reads,
            describe,
            seeds,
            rereads,
            asked: BTreeSet::new(),
            seeding: BTreeMap::new(),
        }));
        let mut tasks = Tasks::default();
        // The dispatcher: every event, in the order decided.
        tasks.0.push(tokio::spawn(async move {
            while let Some(e) = events_rx.recv().await {
                on_event(e);
            }
        }));
        // The data: each sample to the fence; and each heartbeat covering
        // the resource, through the gate (v2 §2.10, "A heartbeat").
        let c = Arc::clone(&core);
        let gate: Arc<dyn crate::consumer::BeatGate> = Arc::new(FollowGate {
            core: Arc::clone(&core),
        });
        let subscription = self
            .subscribe_gated(
                resource,
                move |d: Delivery| {
                    let s = model::Sample {
                        stamp: d.inband,
                        timestamp_id: d.stamp.as_ref().and_then(crate::timing::stamp_id),
                    };
                    let mut core = lock(&c);
                    let member = if core.state {
                        Member::Key(d.sample.key_expr().as_str().to_owned())
                    } else {
                        Member::Alone
                    };
                    let addr = d.provider.clone();
                    let copy = d.clone();
                    let out = core.fence.offer(&addr, s, member, Item::Data(Box::new(d)));
                    if out.held {
                        core.counts.held += 1;
                        core.emit(FollowEvent::Held {
                            delivery: Box::new(copy),
                        });
                    }
                    core.outcome(&addr, out);
                },
                |_| {},
                gate,
            )
            .await?;
        let rt = tokio::runtime::Handle::current();
        let mut subs = Vec::new();
        // Each instance's runs, from its descriptors' puts (core §3.3: put
        // again before it writes under a new run).
        for p in self.providers() {
            let (sys, svc) = p.chunks();
            let (c, rt, delay) = (Arc::clone(&core), rt.clone(), st.descriptor_delay);
            let held = self.held();
            let sub = self
                .session()
                .declare_subscriber(format!("zk2/{sys}/{svc}/@zk/instance/*"))
                .callback(move |s: Sample| {
                    let Some((addr, instance, v)) = descriptor_of(&s, &held) else {
                        return;
                    };
                    let apply = {
                        let c = Arc::clone(&c);
                        move || {
                            let mut core = lock(&c);
                            let d = core.described(&v);
                            let out = core.fence.describe(&addr, instance, d);
                            core.outcome(&addr, out);
                        }
                    };
                    if delay.is_zero() {
                        apply();
                    } else {
                        rt.spawn(async move {
                            tokio::time::sleep(delay).await;
                            apply();
                        });
                    }
                })
                .await
                .map_err(zenoh)?;
            subs.push(sub);
        }
        // The watch (v1 §2.10, v2 §2.10): the providers' interface tokens.
        let mut first = None;
        for p in self.providers() {
            let (sys, svc) = p.chunks();
            let scope = format!("zk2/{sys}/{svc}/@zk/alive/{iface}/**");
            let c = Arc::clone(&core);
            let sub = crate::presence::declare_subscriber(&watch, scope.clone())
                .history(crate::presence::subscriber_needs_history(&watch))
                .callback(move |s: Sample| {
                    let Ok(ZkKey::Alive {
                        addr,
                        iface: i,
                        instance,
                        ..
                    }) = parse(s.key_expr().as_str())
                    else {
                        return;
                    };
                    let mut core = lock(&c);
                    if i != core.iface {
                        return;
                    }
                    core.presence(match s.kind() {
                        SampleKind::Put => Holding::Declared(addr, instance),
                        SampleKind::Delete => Holding::Withdrawn(addr, instance),
                    });
                })
                .await
                .map_err(zenoh)?;
            subs.push(sub);
            first.get_or_insert(scope);
        }
        if let Some(scope) = &first
            && !crate::presence::settle(&watch, scope, st.timeout)
                .await
                .unwrap_or(false)
        {
            tracing::warn!(
                "a follower's watch round trip did not come back: a withdrawal right after its \
                 first read may not be heard (core §8.1)"
            );
        }
        // Each holder's descriptor, where none is held, read by its form
        // through the consumer's catalogs and contract (core §3.3).
        let (c, session, timeout) = (Arc::clone(&core), self.session().clone(), st.timeout);
        let held = self.held();
        tasks.0.push(tokio::spawn(async move {
            while let Some((addr, instance)) = describe_rx.recv().await {
                let found =
                    crate::presence::descriptor_held(&session, &addr, &instance, &held, timeout)
                        .await;
                let v = found.ok().and_then(|f| f.json());
                let mut core = lock(&c);
                match v {
                    Some(v) => {
                        let d = core.described(&v);
                        let out = core.fence.describe(&addr, instance, d);
                        core.outcome(&addr, out);
                    }
                    // Asked again at the holder's next sight.
                    None => {
                        core.asked.remove(&(addr, instance));
                    }
                }
            }
        }));
        // The reads the fence asks for a held sample, one at a time.
        let (c, session, held) = (Arc::clone(&core), self.session().clone(), self.held());
        tasks.0.push(tokio::spawn(async move {
            while let Some(addr) = reads_rx.recv().await {
                let read = read_descriptors(&session, &addr, &held, timeout).await;
                let mut core = lock(&c);
                core.counts.reads += 1;
                let descriptors: Vec<(InstanceId, Described)> = read
                    .iter()
                    .map(|(i, v)| (i.clone(), core.described(v)))
                    .collect();
                let out = core.fence.read_ended(&addr, descriptors);
                core.outcome(&addr, out);
            }
        }));
        // A read again after a withdrawal.
        let (c, session, iface_c, hold) = (
            Arc::clone(&core),
            self.session().clone(),
            iface.clone(),
            st.reread_hold,
        );
        tasks.0.push(tokio::spawn(async move {
            while let Some(addr) = rereads_rx.recv().await {
                tokio::time::sleep(BURST).await;
                let sel = format!(
                    "zk2/{}/{}/@zk/alive/{iface_c}/**",
                    addr.system, addr.service
                );
                read_holders(&session, &c, &sel, Some(&addr), timeout, hold).await;
            }
        }));
        // The seeds, one at a time per address.
        let (c, consumer, name, st_c) = (
            Arc::clone(&core),
            self.clone(),
            resource.to_owned(),
            st.clone(),
        );
        let state_seeds = state;
        tasks.0.push(tokio::spawn(async move {
            let mut running = Vec::new();
            while let Some(addr) = seeds_rx.recv().await {
                if !state_seeds {
                    continue;
                }
                {
                    let mut core = lock(&c);
                    if let Some(s) = core.seeding.get_mut(&addr) {
                        s.again = true;
                        continue;
                    }
                    core.seeding.insert(addr.clone(), Seeding::default());
                }
                running.push(tokio::spawn(seed_address(
                    Arc::clone(&c),
                    consumer.clone(),
                    name.clone(),
                    addr,
                    st_c.clone(),
                )));
                running.retain(|t: &tokio::task::JoinHandle<()>| !t.is_finished());
            }
            for t in running {
                t.abort();
            }
        }));
        // The first reads, after the watch's round trip.
        for p in self.providers() {
            let (sys, svc) = p.chunks();
            let sel = format!("zk2/{sys}/{svc}/@zk/alive/{iface}/**");
            let exact = (sys != "*" && svc != "*")
                .then(|| format!("{sys}/{svc}").parse::<Addr>().ok())
                .flatten();
            read_holders(
                self.session(),
                &core,
                &sel,
                exact.as_ref(),
                st.timeout,
                Duration::ZERO,
            )
            .await;
        }
        Ok(Following {
            core,
            subscription,
            _subs: subs,
            _tasks: tasks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> InstanceId {
        InstanceId::from_u64(n)
    }

    /// A read's keys give each address's holders of the interface alone.
    #[test]
    fn a_read_shows_the_holders_of_the_interface_by_address() {
        let iface: IfaceId = "ledger.v1".parse().unwrap();
        let keys = vec![
            format!(
                "zk2/lab/ledger/@zk/alive/ledger.v1/{}/0123456789abcdef",
                id(0xa)
            ),
            format!(
                "zk2/lab/ledger/@zk/alive/other.v1/{}/0123456789abcdef",
                id(0xb)
            ),
            format!("zk2/lab/ledger/@zk/instance/{}", id(0xc)),
            format!(
                "zk2/lab/ledger2/@zk/alive/ledger.v1/{}/0123456789abcdef",
                id(0xd)
            ),
        ];
        let by = holders_by_addr(&keys, &iface);
        let a: Addr = "lab/ledger".parse().unwrap();
        let b: Addr = "lab/ledger2".parse().unwrap();
        assert_eq!(by.get(&a), Some(&BTreeSet::from([id(0xa)])));
        assert_eq!(by.get(&b), Some(&BTreeSet::from([id(0xd)])));
        assert_eq!(by.len(), 2);
    }
}
