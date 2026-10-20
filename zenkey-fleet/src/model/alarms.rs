//! `alarms.v1` (#800, AC): one reader's alarms, built from what the bus
//! answered (`spec/profiles/alarms/v1.md` §2.6, §2.14, §2.15).
//!
//! **Values in hand → meaning.** An [`AlarmBook`] is one reader's view of
//! the alarms a selector reaches ([`AlarmScope`]). It is fed values, never
//! a session — a presence read with its descriptors, the instance tokens'
//! moves, each sample of each alarm key (a subscription's delivery or a
//! reply to S4's GET), the end of each GET, a loss, an archive's
//! last-known documents — so a `.zrec` replays through it as live traffic
//! does. Time is an argument too: an [`Instant`] for this host's monotonic
//! clock, a [`SystemTime`] for stamp ages.
//!
//! What it builds is what `zenkey_model::alarms` judges, and the verdicts
//! are that module's, never re-derived here:
//!
//! - **one key's samples** merged by the owner's stamp (core §4.2,
//!   `zenkey_model::seed::admits`), read through
//!   `zenkey_model::alarms::Track` into §2.6's transitions, each payload
//!   read by `decode`, strictly (§2.4, 0.2);
//! - **one key's reading** ([`KeyReading`]): its owner's presence and
//!   descriptor (§2.13: found by descriptor, never by token), its
//!   observations as `freshness.v1` judges them (the subscription's last
//!   delivery, the GET's reply aged against a trusted clock, dated by when
//!   its GET was asked), the latest sample, `previous_instance` (the latest
//!   put's stamp id is not a present descriptor's `meta.zid`, by value;
//!   never with no `meta.zid`), whether the owner's alarms are known whole,
//!   and whether the latest GET left the key out (§2.14 steps 4 and 5, text
//!   0.5) — for `firing`, `stale_firing` and `rederives`;
//! - **one owner's alarms** (§2.15): the latest GET as its seed, and its
//!   window — from the subscription's declaration plus the reply timeout,
//!   or, after a loss or a gap in the owner's presence, from the first
//!   complete GET that followed it; lossless and present throughout over
//!   it; its span counted to one lease and the reply timeout before now,
//!   where its presence is still watched (core §2.7, 0.25) — for
//!   `owner_set`;
//! - **the owners in scope** (§2.15, F-131): those a presence read finds,
//!   those an earlier reading of its own knew, those an archive's keys
//!   name, each with how it was known, and whether the reads that found
//!   them were whole (text 0.5, F-137) — for `any_firing`;
//! - **a reader's transitions** ([`AlarmTransition`], pinned in
//!   [`crate::report`]): §2.6's per sample, `lost` on every key at a loss
//!   (unobservable, never ok), and `judged` when a key's answer moves with
//!   no sample ([`AlarmBook::sweep`]). What a notifier routes.
//!
//! **Bounded.** At most [`MAX_KEYS`] keys are held; a key past it is
//! refused and counted, and the scope's answer can then never be *no*. A
//! key deleted, or dropped by its owner's answer, is forgotten
//! [`FORGET_AFTER`] later.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::alarms::{
    self as am, Alarm, AlarmRef, Answer, AnyFiring, Invalid, Judged, Latest, Listing, OwnerAlarms,
    Payload, Presence, Reading, Reason, ScopeRead, Seed, SetJudged, SetVerdict, Severity, Track,
    Window,
};
use zenkey_model::freshness::{ClockMeasure, Last, Observation, Reply, StampAge};
use zenkey_model::grammar::{Addr, InstanceId, ZkKey};
use zenkey_model::seed::Held;
use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};
use zenoh::time::Timestamp;

use crate::model::catalog::{Observed, zid_value};
use crate::model::health::Trust;
use crate::report::{
    AlarmAnswer, AlarmAnswerToken, AlarmChange, AlarmLatest, AlarmLoss, AlarmSeen, AlarmSetVerdict,
    AlarmTransition, AlarmVia, ClockGround, KnownBy, Stamp,
};

/// The most keys a book holds; a key past it is refused and counted.
pub const MAX_KEYS: usize = 16_384;

/// How long a key deleted, or dropped by its owner's answer, is kept
/// before it is forgotten: twice the horizon.
pub const FORGET_AFTER: Duration = Duration::from_secs(1800);

/// Every alarm of every service, base-relative.
pub const ALL: &str = "zk2/*/*/alarms.v1/state/alarms/*";

// ─── the scope ──────────────────────────────────────────────────────────────

/// The alarms a selector reaches (§2.15): a base-relative key expression,
/// in the deployment's namespace, that intersects the profile's template
/// `zk2/*/*/alarms.v1/state/alarms/*`. An owner is in it when its alarm
/// template intersects the selector; a key, when it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmScope {
    selector: String,
    ke: OwnedKeyExpr,
    alarms: String,
    system: Option<String>,
    service: Option<String>,
}

/// A chunk usable as a literal position of a narrower selector.
fn literal(c: Option<&&str>) -> Option<String> {
    c.filter(|c| !c.contains('*') && !c.is_empty())
        .map(|c| (*c).to_owned())
}

impl AlarmScope {
    /// Every alarm of every service: [`ALL`].
    pub fn all() -> AlarmScope {
        AlarmScope::parse(ALL).expect("the template is a scope")
    }

    /// The alarms of one service.
    pub fn of(addr: &Addr) -> AlarmScope {
        AlarmScope::parse(&format!(
            "zk2/{}/{}/alarms.v1/state/alarms/*",
            addr.system, addr.service
        ))
        .expect("an address's alarms are a scope")
    }

    /// A selector, refused when it is not a key expression, is not
    /// base-relative (`zk2/…`), or reaches no `alarms.v1` key.
    pub fn parse(selector: &str) -> std::result::Result<AlarmScope, String> {
        let ke = OwnedKeyExpr::try_from(selector.to_owned())
            .map_err(|e| format!("{selector:?} is not a key expression: {e}"))?;
        let chunks: Vec<&str> = selector.split('/').collect();
        if chunks.first() != Some(&"zk2") {
            return Err(format!(
                "{selector:?} is not base-relative: alarms are read through a session in the \
                 deployment's namespace, so the selector starts at zk2/ (the namespace is \
                 --namespace)"
            ));
        }
        let template = KeyExpr::try_from(ALL).expect("the template is a key expression");
        if !ke.intersects(&template) {
            return Err(format!(
                "{selector:?} reaches no alarms.v1 key \
                 (zk2/<system>/<service>/alarms.v1/state/alarms/<alert_key>)"
            ));
        }
        let system = literal(chunks.get(1));
        let service = system.as_ref().and_then(|_| literal(chunks.get(2)));
        let alert_key = (chunks.len() == 7 && chunks[3..6] == ["alarms.v1", "state", "alarms"])
            .then(|| chunks[6].to_owned());
        let alarms = format!(
            "zk2/{}/{}/alarms.v1/state/alarms/{}",
            system.as_deref().unwrap_or("*"),
            service.as_deref().unwrap_or("*"),
            alert_key.as_deref().unwrap_or("*"),
        );
        Ok(AlarmScope {
            selector: selector.to_owned(),
            ke,
            alarms,
            system,
            service,
        })
    }

    /// The selector as given.
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// What is subscribed to and GET: the selector narrowed to the
    /// profile's template, never wider than it.
    pub fn alarms_selector(&self) -> &str {
        &self.alarms
    }

    /// The instance tokens of the owners in scope:
    /// `zk2/<system>/<service>/@zk/instance/*`.
    pub fn instance_selector(&self) -> String {
        format!(
            "zk2/{}/{}/@zk/instance/*",
            self.system.as_deref().unwrap_or("*"),
            self.service.as_deref().unwrap_or("*")
        )
    }

    /// The one owner the selector names, when its system and service are
    /// both literal.
    pub fn one_owner(&self) -> Option<Addr> {
        format!("{}/{}", self.system.as_ref()?, self.service.as_ref()?)
            .parse()
            .ok()
    }

    /// The narrowest presence scope covering the owners in scope.
    pub fn presence_scope(&self) -> crate::bus::presence::Scope {
        use crate::bus::presence::Scope;
        if let Some(a) = self.one_owner() {
            return Scope::service(&a);
        }
        self.system
            .as_deref()
            .and_then(|s| Scope::system(s).ok())
            .unwrap_or_else(Scope::all)
    }

    /// Whether `addr`'s alarms are in scope.
    pub fn selects_owner(&self, addr: &Addr) -> bool {
        KeyExpr::try_from(format!(
            "zk2/{}/{}/alarms.v1/state/alarms/*",
            addr.system, addr.service
        ))
        .is_ok_and(|k| self.ke.intersects(&k))
    }

    /// Whether the key `key` is in scope.
    pub fn selects_key(&self, key: &str) -> bool {
        KeyExpr::try_from(key).is_ok_and(|k| self.ke.intersects(&k))
    }
}

// ─── the inputs ─────────────────────────────────────────────────────────────

/// How a reader is set up (§2.15, core §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BookSpec {
    /// The reply timeout: a window counts from its declaration plus it, and
    /// its owner is watched a lease and it past its end (core §2.7).
    pub timeout: Duration,
    /// The deployment's link lease (core §2.7, 0.25).
    pub lease: Duration,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1). Without it, a
    /// GET reply's stamp is aged only against a clock measured on a live
    /// put of the same clock (ground 2).
    pub clocks_synced: bool,
    /// Across a constrained face (§2.11), where presence does not cross and
    /// the deployment's word stands for the descriptor: whether the face
    /// lets this reader GET the alarms. `None`: no face.
    pub face: Option<bool>,
}

impl BookSpec {
    /// No face, nothing on the operator's word, zenoh's lease.
    pub fn new(timeout: Duration) -> BookSpec {
        BookSpec {
            timeout,
            lease: zenkey_model::budget::DEFAULT_LEASE,
            clocks_synced: false,
            face: None,
        }
    }

    /// How long past a window's end its owner is watched: a lease and the
    /// reply timeout (core §2.7, 0.25).
    pub fn tail(&self) -> Duration {
        self.lease + self.timeout
    }
}

/// One sample of one alarm key, as a reader received it.
#[derive(Debug, Clone)]
pub struct AlarmSample {
    /// The key, base-relative.
    pub key: String,
    /// A delete, or a `reply_del`.
    pub delete: bool,
    pub payload: Vec<u8>,
    /// The owner's stamp (core S1), when it carried one.
    pub stamp: Option<Timestamp>,
    /// Its arrival, on this host's monotonic clock.
    pub at: Instant,
    /// Its arrival, on this host's wall clock: a live put's stamp against
    /// it measures the stamping clock (`freshness.v1` §2.6, ground 2).
    pub wall: SystemTime,
    pub via: AlarmVia,
}

impl AlarmSample {
    /// A sample as the fleet's sample view carries it (a live delivery, or
    /// a `.zrec` row), its key already base-relative.
    pub fn of_view(view: &crate::bus::monitor::SampleView, key: String, via: AlarmVia) -> Self {
        AlarmSample {
            key,
            delete: view.kind == zenoh::sample::SampleKind::Delete,
            payload: view.payload.to_bytes().into_owned(),
            stamp: view.timestamp,
            at: view.received,
            wall: SystemTime::now(),
            via,
        }
    }
}

/// How one S4 GET of the scope's alarms ended (§2.15, core §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeedEnd {
    /// When it was asked: a loss or a gap before it is healed by it.
    pub asked_at: Instant,
    /// It ran to the final reply with no error reply.
    pub complete: bool,
}

/// One S4 GET of the scope's alarms, as it was read: when it was asked,
/// how it ended, and its replies, each a sample via [`AlarmVia::Get`].
#[derive(Debug, Clone)]
pub struct SeedRead {
    pub asked_at: Instant,
    pub complete: bool,
    pub replies: Vec<AlarmSample>,
    /// Replies on a non-concrete key, discarded (R6).
    pub discarded: u64,
}

/// An archive's last-known document of one alarm key (core §4.4, S6).
#[derive(Debug, Clone)]
pub struct ArchivedAlarm {
    /// The archive, `<system>/<service>`.
    pub archive: Addr,
    /// The origin key, base-relative.
    pub key: String,
    /// The value, or `None` for a tombstone.
    pub value: Option<Vec<u8>>,
    pub stamp: Option<Timestamp>,
    pub confirmed: bool,
}

// ─── the book ───────────────────────────────────────────────────────────────

/// What one descriptor of an instance says of `alarms.v1` (§2.13).
#[derive(Debug, Clone, PartialEq, Eq)]
struct InstanceSeen {
    listing: Listing,
    /// Its `meta.zid`, by value; `None` when it carries none.
    zid: Option<String>,
}

/// One owner, as this reader knows it.
#[derive(Debug, Clone, Default)]
struct OwnerEntry {
    known: BTreeSet<KnownBy>,
    /// Its instances holding an instance token now, with what each
    /// descriptor said; `None` when no descriptor of it was read.
    instances: BTreeMap<InstanceId, Option<InstanceSeen>>,
    /// The last change of its instance set this reader saw, and the first
    /// complete GET asked after it: a gap in its presence (§2.15).
    gap: Option<Break>,
}

/// A break in a window — a loss, or a gap in an owner's presence — and the
/// first complete GET asked after it, which restarts the window (§2.15,
/// 0.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Break {
    at: Instant,
    healed: Option<Instant>,
}

impl Break {
    fn heal(&mut self, seed: SeedEnd) {
        if seed.complete && self.healed.is_none() && seed.asked_at > self.at {
            self.healed = Some(seed.asked_at);
        }
    }
}

/// The latest sample of one key by the owner's stamp.
#[derive(Debug, Clone)]
struct LatestSample {
    doc: Option<std::result::Result<Alarm, Invalid>>,
    stamp: Option<Timestamp>,
    via: AlarmVia,
}

/// The last GET's reply for one key.
#[derive(Debug, Clone, Copy)]
struct GotReply {
    /// When its GET was asked, once the GET ended ([`AlarmBook::seeded`]);
    /// its arrival until then.
    asked_at: Instant,
    delete: bool,
    stamp: Option<Timestamp>,
}

/// One alarm key, as this reader follows it.
#[derive(Debug, Clone)]
struct KeyEntry {
    owner: Addr,
    alarm: AlarmRef,
    track: Track,
    merged: Held,
    latest: Option<LatestSample>,
    /// The last arrival of anything of it, by either path, admitted by the
    /// merge or not: a GET asked after it that did not answer the key left
    /// it out (§2.14 step 5).
    arrived: Instant,
    /// The subscription's last delivery of it, and whether a delete.
    sub_last: Option<(Instant, bool)>,
    got: Option<GotReply>,
    /// When its latest was dropped by its owner's answer, or it was
    /// deleted: when it may be forgotten.
    gone_at: Option<Instant>,
    /// The first question's answer, as last reported.
    reported: Option<Judged>,
}

/// What a presence read established, as a whole.
#[derive(Debug, Clone, Copy)]
struct Basis {
    complete: bool,
}

/// The latest GET folded: how it ended, how many keys of each owner it
/// answered with a value, and every key it answered, a `reply_del`
/// included. A key it did not answer, with nothing of it arriving after it
/// was asked, it left out (§2.14 step 5).
#[derive(Debug, Clone)]
struct LastSeed {
    end: SeedEnd,
    members: BTreeMap<Addr, usize>,
    answered: BTreeSet<String>,
}

/// One reader's alarms under a scope: see the module doc.
#[derive(Debug, Clone)]
pub struct AlarmBook {
    scope: AlarmScope,
    spec: BookSpec,
    /// When the subscription was declared; `None` when there is none.
    declared: Option<Instant>,
    presence: Option<Basis>,
    owners: BTreeMap<Addr, OwnerEntry>,
    keys: BTreeMap<String, KeyEntry>,
    /// The latest GET folded.
    seed: Option<LastSeed>,
    /// The GET in flight: what it has answered so far.
    pending: BTreeMap<Addr, usize>,
    pending_keys: BTreeSet<String>,
    loss: Option<Break>,
    lost: u64,
    refused: u64,
    clocks: BTreeMap<String, ClockMeasure>,
    archived: BTreeMap<String, ArchivedAlarm>,
}

impl AlarmBook {
    /// An empty book over `scope`. Across a face, the one owner the scope
    /// names is in it on the deployment's word (§2.11).
    pub fn new(scope: AlarmScope, spec: BookSpec) -> AlarmBook {
        let mut owners = BTreeMap::new();
        if spec.face.is_some()
            && let Some(addr) = scope.one_owner()
        {
            owners.insert(
                addr,
                OwnerEntry {
                    known: BTreeSet::from([KnownBy::Word]),
                    ..OwnerEntry::default()
                },
            );
        }
        AlarmBook {
            scope,
            spec,
            declared: None,
            presence: None,
            owners,
            keys: BTreeMap::new(),
            seed: None,
            pending: BTreeMap::new(),
            pending_keys: BTreeSet::new(),
            loss: None,
            lost: 0,
            refused: 0,
            clocks: BTreeMap::new(),
            archived: BTreeMap::new(),
        }
    }

    pub fn scope(&self) -> &AlarmScope {
        &self.scope
    }

    pub fn spec(&self) -> &BookSpec {
        &self.spec
    }

    /// When the subscription was declared.
    pub fn declared(&self) -> Option<Instant> {
        self.declared
    }

    /// Deliveries lost, and stalls, so far.
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// Keys refused at [`MAX_KEYS`].
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Whether the reader's latest loss — deliveries its queue refused, or
    /// a stall of its receive path — has no complete GET asked after it
    /// yet: until one heals it, every key reads unobservable (§2.14 step
    /// 4), and every window restarts from it (§2.15). A notifier says it
    /// once for its scope (#800, AD).
    pub fn loss_unhealed(&self) -> bool {
        self.loss.is_some_and(|l| l.healed.is_none())
    }

    /// How the latest GET folded into the book ended: `Some(true)` when it
    /// ran to its final reply with no error reply; `None` before any.
    pub fn seed_complete(&self) -> Option<bool> {
        self.seed.as_ref().map(|s| s.end.complete)
    }

    /// Whether a presence read has been folded, and was complete.
    pub fn presence_complete(&self) -> Option<bool> {
        self.presence.map(|b| b.complete)
    }

    /// Whether the reads that found the owners in scope were whole (§2.15,
    /// text 0.5, F-137): across a face, the deployment's word; otherwise
    /// the latest presence read ran to its final reply, and nothing was
    /// lost since — a loss may have taken an instance token's appearance.
    /// An archive's read plays no part: an owner known only by its keys,
    /// which a complete presence read does not find, is absent, and never
    /// stops a no.
    pub fn scope_read(&self) -> ScopeRead {
        if self.spec.face.is_some() || self.presence.is_some_and(|b| b.complete) {
            ScopeRead::Whole
        } else {
            ScopeRead::NotWhole
        }
    }

    /// How the reader trusts its clock, as the report spells it.
    pub fn clock_ground(&self) -> ClockGround {
        self.trust().ground()
    }

    fn trust(&self) -> Trust<'_> {
        if self.spec.clocks_synced {
            Trust::Word
        } else if self.declared.is_some() {
            Trust::Measured(&self.clocks)
        } else {
            Trust::None
        }
    }

    // ─── folding ────────────────────────────────────────────────────────

    /// The subscription was declared at `at`, before any GET (core §4.2):
    /// the window starts its count here.
    pub fn subscribed(&mut self, at: Instant) {
        self.declared = Some(at);
    }

    fn owner(&mut self, addr: &Addr) -> &mut OwnerEntry {
        self.owners.entry(addr.clone()).or_default()
    }

    /// One presence read, with its descriptors (core §8.1, §3.3): every
    /// owner it finds in scope, by instance token and descriptor, never by
    /// interface token (§2.13). An owner known before that a complete read
    /// does not find is absent from now. An instance set that moved since
    /// the book last saw it is a gap in that owner's presence.
    pub fn presence_read(&mut self, o: &Observed, at: Instant) -> Vec<AlarmTransition> {
        let first = self.presence.is_none();
        self.presence = Some(Basis {
            complete: o.complete,
        });
        let mut held: BTreeMap<Addr, BTreeMap<InstanceId, Option<InstanceSeen>>> = BTreeMap::new();
        for k in &o.tokens {
            if let ZkKey::Instance { addr, instance } = k
                && self.scope.selects_owner(addr)
            {
                let seen = o
                    .descriptors
                    .as_ref()
                    .and_then(|ds| ds.get(&(addr.clone(), instance.clone())))
                    .map(|read| match read.descriptor() {
                        Some(d) => InstanceSeen {
                            listing: d
                                .interfaces
                                .iter()
                                .find(|e| e.iface == am::IFACE)
                                .map_or(Listing::NotListed, |e| Listing::Listed { token: e.token }),
                            zid: d.meta.get("zid").and_then(|v| v.as_str()).map(zid_value),
                        },
                        None => InstanceSeen {
                            listing: Listing::Unread,
                            zid: None,
                        },
                    });
                held.entry(addr.clone())
                    .or_default()
                    .insert(instance.clone(), seen);
            }
        }
        for (addr, instances) in &held {
            let entry = self.owner(addr);
            entry.known.insert(KnownBy::Presence);
            let before: BTreeSet<&InstanceId> = entry.instances.keys().collect();
            let now: BTreeSet<&InstanceId> = instances.keys().collect();
            if !first && before != now {
                entry.gap = Some(Break { at, healed: None });
            }
            entry.instances = instances.clone();
        }
        for (addr, entry) in &mut self.owners {
            if held.contains_key(addr) {
                continue;
            }
            entry.known.remove(&KnownBy::Presence);
            entry.known.insert(KnownBy::Earlier);
            if !entry.instances.is_empty() {
                entry.instances.clear();
                entry.gap = Some(Break { at, healed: None });
            }
        }
        // A read only adds to what an owner was known by: the next read's
        // finding it again is an earlier reading too.
        for entry in self.owners.values_mut() {
            if entry.known.contains(&KnownBy::Presence) {
                entry.known.insert(KnownBy::Earlier);
            }
        }
        Vec::new()
    }

    /// One instance token's move (core §8.1): held from `at`, or gone. A
    /// move is a gap in the owner's presence; a new instance's descriptor
    /// is unread until the next presence read.
    pub fn instance(&mut self, addr: &Addr, instance: &InstanceId, held: bool, at: Instant) {
        if !self.scope.selects_owner(addr) {
            return;
        }
        let entry = self.owner(addr);
        entry.known.insert(KnownBy::Earlier);
        let changed = if held {
            entry.instances.insert(instance.clone(), None).is_none()
        } else {
            entry.instances.remove(instance).is_some()
        };
        if changed {
            entry.gap = Some(Break { at, healed: None });
        }
    }

    /// One sample of an alarm key, the next to arrive: merged by the
    /// owner's stamp (core §4.2), and, when it is the key's latest, read as
    /// §2.6 says. Returns the key's transition, when it made one.
    pub fn sample(&mut self, s: AlarmSample) -> Option<AlarmTransition> {
        if !self.scope.selects_key(&s.key) {
            return None;
        }
        let alarm = AlarmRef::from_key(&s.key)?;
        let owner: Addr = format!("{}/{}", alarm.system, alarm.service).parse().ok()?;
        if s.via == AlarmVia::Subscription
            && !s.delete
            && let Some(t) = &s.stamp
        {
            let offset = StampAge::between(t.get_time().to_system_time(), s.wall);
            self.clocks
                .entry(t.get_id().to_string())
                .and_modify(|m| m.record(offset))
                .or_insert(ClockMeasure::new(offset));
        }
        if !self.keys.contains_key(&s.key) && self.keys.len() >= MAX_KEYS {
            self.refused += 1;
            return None;
        }
        self.owner(&owner).known.insert(KnownBy::Earlier);
        if s.via == AlarmVia::Get {
            self.pending_keys.insert(s.key.clone());
            if !s.delete {
                *self.pending.entry(owner.clone()).or_default() += 1;
            }
        }
        let entry = self.keys.entry(s.key.clone()).or_insert_with(|| KeyEntry {
            owner,
            alarm,
            track: Track::default(),
            merged: Held::Nothing,
            latest: None,
            arrived: s.at,
            sub_last: None,
            got: None,
            gone_at: None,
            reported: None,
        });
        entry.arrived = entry.arrived.max(s.at);
        let incoming = s.stamp.as_ref().map(crate::bus::seed::stamp_of);
        let admitted = zenkey_model::seed::admits(entry.merged, incoming);
        if s.via == AlarmVia::Get {
            entry.got = Some(GotReply {
                asked_at: s.at,
                delete: s.delete,
                stamp: s.stamp,
            });
        }
        if !admitted {
            return None;
        }
        entry.merged = entry.merged.after(incoming);
        if s.via == AlarmVia::Subscription {
            entry.sub_last = Some((s.at, s.delete));
        }
        let doc = (!s.delete).then(|| am::decode(&s.payload));
        let t = match &doc {
            None => entry.track.apply(am::Sample::Delete),
            Some(Ok(a)) => entry.track.apply(am::Sample::Put(a)),
            Some(Err(_)) => entry.track.apply(am::Sample::Unreadable),
        };
        entry.gone_at = s.delete.then_some(s.at);
        entry.latest = Some(LatestSample {
            doc,
            stamp: s.stamp,
            via: s.via,
        });
        let key = s.key.clone();
        Some(self.transition_of(&key, change_of(t), Some(s.via), None, s.at, s.wall))
    }

    /// A GET was asked at `asked_at`: its replies follow as samples
    /// ([`AlarmBook::sample`], via [`AlarmVia::Get`]), then its end
    /// ([`AlarmBook::seeded`]).
    pub fn seeding(&mut self) {
        self.pending.clear();
        self.pending_keys.clear();
    }

    /// The GET ended: the owners' seed (§2.15). A complete one heals a loss
    /// and a gap asked before it; a key it left out of an owner whose alarms
    /// it established holds nothing ([`AlarmBook::sweep`], §2.14 step 5).
    pub fn seeded(&mut self, end: SeedEnd, now: Instant, wall: SystemTime) -> Vec<AlarmTransition> {
        let members = std::mem::take(&mut self.pending);
        let answered = std::mem::take(&mut self.pending_keys);
        if let Some(l) = &mut self.loss {
            l.heal(end);
        }
        for o in self.owners.values_mut() {
            if let Some(g) = &mut o.gap {
                g.heal(end);
            }
        }
        // A reply is dated by the instant its GET was asked, never by its
        // arrival: a loss after the ask, before the reply came, may hold a
        // put newer than the owner's answer (§2.14 step 4, core §4.2).
        for k in &answered {
            if let Some(g) = self.keys.get_mut(k).and_then(|e| e.got.as_mut()) {
                g.asked_at = end.asked_at;
            }
        }
        self.seed = Some(LastSeed {
            end,
            members,
            answered,
        });
        self.sweep(now, wall)
    }

    /// Whether the latest GET left `key` out: it ran to its final reply,
    /// did not answer the key, with a value or a `reply_del`, and nothing of
    /// the key arrived after it was asked (§2.14 step 5). With the owner's
    /// alarms established, that GET established them (§2.15): it was asked
    /// after every loss, or the window lost a delivery.
    fn left_out(&self, key: &str, e: &KeyEntry) -> bool {
        self.seed.as_ref().is_some_and(|s| {
            s.end.complete && !s.answered.contains(key) && e.arrived < s.end.asked_at
        })
    }

    /// One GET folded whole: [`AlarmBook::seeding`], each reply, then
    /// [`AlarmBook::seeded`]. Returns every transition it made.
    pub fn seed(&mut self, read: SeedRead, now: Instant, wall: SystemTime) -> Vec<AlarmTransition> {
        self.seeding();
        let mut out: Vec<AlarmTransition> = read
            .replies
            .into_iter()
            .filter_map(|r| self.sample(r))
            .collect();
        out.extend(self.seeded(
            SeedEnd {
                asked_at: read.asked_at,
                complete: read.complete,
            },
            now,
            wall,
        ));
        out
    }

    /// The reader lost `dropped` deliveries, or its receive path stalled
    /// (core §2.7, 0.27), at `at`: every key is unobservable until a GET
    /// asked after it answers it, every window restarts at the first
    /// complete GET after it, and what presence moved is not known either.
    pub fn lose(&mut self, loss: AlarmLoss, at: Instant, wall: SystemTime) -> Vec<AlarmTransition> {
        self.lost += loss.dropped.max(1);
        self.loss = Some(Break { at, healed: None });
        if let Some(b) = &mut self.presence {
            b.complete = false;
        }
        for o in self.owners.values_mut() {
            o.gap = Some(Break { at, healed: None });
        }
        let keys: Vec<String> = self.keys.keys().cloned().collect();
        keys.iter()
            .map(|k| self.transition_of(k, AlarmChange::Lost, None, Some(loss), at, wall))
            .collect()
    }

    /// An archive's last-known documents (core §4.4, S6): shown, never
    /// current, and every owner their keys name known by the archive.
    pub fn archived(&mut self, found: Vec<ArchivedAlarm>) {
        for a in found {
            if !self.scope.selects_key(&a.key) {
                continue;
            }
            let Some(r) = AlarmRef::from_key(&a.key) else {
                continue;
            };
            let Ok(owner) = format!("{}/{}", r.system, r.service).parse::<Addr>() else {
                continue;
            };
            self.owner(&owner).known.insert(KnownBy::Archive);
            let newer = self
                .archived
                .get(&a.key)
                .is_none_or(|have| a.stamp > have.stamp);
            if newer {
                self.archived.insert(a.key.clone(), a);
            }
        }
    }

    /// Every key whose first answer moved since it was last reported, with
    /// no sample: gone stale, its owner gone or back, its owner's answer
    /// dropped it. A key the GET that established its owner's alarms left
    /// out holds nothing from now (§2.14 step 5, text 0.3), so a later put
    /// of it is a raise; it is forgotten [`FORGET_AFTER`] later, as are the
    /// keys deleted.
    pub fn sweep(&mut self, now: Instant, wall: SystemTime) -> Vec<AlarmTransition> {
        let established: BTreeSet<Addr> = self
            .owners
            .keys()
            .filter(|a| self.owner_set(a, now).verdict == SetVerdict::Established)
            .cloned()
            .collect();
        let dropped: Vec<String> = self
            .keys
            .iter()
            .filter(|(k, e)| {
                e.latest.is_some() && established.contains(&e.owner) && self.left_out(k, e)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in dropped {
            let Some(e) = self.keys.get_mut(&k) else {
                continue;
            };
            e.latest = None;
            e.track = Track::default();
            e.merged = Held::Nothing;
            e.gone_at.get_or_insert(now);
        }
        self.keys.retain(|_, e| {
            e.gone_at
                .is_none_or(|g| now.saturating_duration_since(g) < FORGET_AFTER)
        });
        let keys: Vec<String> = self.keys.keys().cloned().collect();
        let mut out = Vec::new();
        for k in keys {
            let j = self.reading(&k, now, wall).map(|r| r.firing());
            let moved = self.keys.get(&k).is_some_and(|e| e.reported != j);
            if moved {
                out.push(self.transition_of(&k, AlarmChange::Judged, None, None, now, wall));
            }
        }
        out
    }

    fn transition_of(
        &mut self,
        key: &str,
        change: AlarmChange,
        via: Option<AlarmVia>,
        lost: Option<AlarmLoss>,
        now: Instant,
        wall: SystemTime,
    ) -> AlarmTransition {
        let firing = self
            .reading(key, now, wall)
            .map_or_else(unread, |r| r.firing());
        let e = self.keys.get_mut(key).expect("a key the book holds");
        e.reported = Some(firing);
        let doc = e
            .latest
            .as_ref()
            .and_then(|l| l.doc.as_ref())
            .and_then(|d| d.as_ref().ok())
            .cloned();
        AlarmTransition {
            key: key.to_owned(),
            alarm: e.alarm.clone(),
            change,
            firing: answer_of(firing),
            severity: doc.as_ref().map(|d| d.severity),
            stamp: e
                .latest
                .as_ref()
                .and_then(|l| l.stamp.as_ref())
                .map(stamp_report),
            document: if change == AlarmChange::Lost || change == AlarmChange::Judged {
                None
            } else {
                doc
            },
            via,
            lost,
        }
    }

    // ─── reading ────────────────────────────────────────────────────────

    /// `addr`'s presence (§2.14 step 1): across a face, the deployment's
    /// word; before any read, possibly incomplete; held, with what its
    /// descriptors say; else absent from a complete read, possibly
    /// incomplete otherwise.
    pub fn presence_of(&self, addr: &Addr) -> Presence {
        if let Some(reachable) = self.spec.face {
            return Presence::AcrossFace { reachable };
        }
        let Some(basis) = self.presence else {
            return Presence::Incomplete;
        };
        match self.owners.get(addr).filter(|o| !o.instances.is_empty()) {
            Some(o) => {
                let mut unread = false;
                for seen in o.instances.values() {
                    match seen {
                        Some(InstanceSeen {
                            listing: l @ Listing::Listed { .. },
                            ..
                        }) => return Presence::Present(*l),
                        Some(InstanceSeen {
                            listing: Listing::NotListed,
                            ..
                        }) => {}
                        _ => unread = true,
                    }
                }
                Presence::Present(if unread {
                    Listing::Unread
                } else {
                    Listing::NotListed
                })
            }
            None if basis.complete => Presence::Absent,
            None => Presence::Incomplete,
        }
    }

    /// The `meta.zid`s of `addr`'s present descriptors, by value: whose
    /// stamps are its own (core §3.3).
    fn zids(&self, addr: &Addr) -> BTreeSet<String> {
        self.owners
            .get(addr)
            .into_iter()
            .flat_map(|o| o.instances.values())
            .filter_map(|s| s.as_ref()?.zid.clone())
            .collect()
    }

    /// `addr`'s window (§2.15), when a subscription listens: see the module
    /// doc.
    pub fn window(&self, addr: &Addr, now: Instant) -> Option<Window> {
        let declared = self.declared?;
        let o = self.owners.get(addr);
        let gap = o.and_then(|o| o.gap);
        let mut start = declared + self.spec.timeout;
        let mut lossless = true;
        if let Some(l) = self.loss {
            match l.healed {
                Some(h) => start = start.max(h),
                None => lossless = false,
            }
        }
        let mut present = matches!(self.presence_of(addr), Presence::Present(_));
        if let Some(g) = gap {
            match g.healed {
                Some(h) => start = start.max(h),
                None => present = false,
            }
        }
        Some(Window {
            listened: now.saturating_duration_since(start + self.spec.tail()),
            lossless,
            present_throughout: present,
        })
    }

    /// `addr`'s seed: the latest GET, and the keys of `addr` it answered
    /// with a value.
    fn seed_of(&self, addr: &Addr) -> Option<Seed> {
        let s = self.seed.as_ref()?;
        Some(if s.end.complete {
            Seed::Complete {
                members: s.members.get(addr).copied().unwrap_or(0),
            }
        } else {
            Seed::Incomplete
        })
    }

    /// Whether `addr`'s alarms are known whole (§2.15).
    pub fn owner_set(&self, addr: &Addr, now: Instant) -> SetJudged {
        am::owner_set(
            self.presence_of(addr),
            self.seed_of(addr),
            self.window(addr, now),
        )
    }

    /// The reading of one key at `now` (§2.14); `None` for a key the book
    /// does not hold.
    pub fn reading(&self, key: &str, now: Instant, wall: SystemTime) -> Option<KeyReading> {
        let e = self.keys.get(key)?;
        let presence = self.presence_of(&e.owner);
        let mut observations = Vec::new();
        let healed_by = |asked: Instant| self.loss.is_none_or(|l| asked > l.at);
        if let Some(declared) = self.declared {
            observations.push(Observation::Subscribed {
                last: e.sub_last.map(|(t, delete)| {
                    let age = now.saturating_duration_since(t);
                    if delete {
                        Last::Delete(age)
                    } else {
                        Last::Put(age)
                    }
                }),
                listened: now.saturating_duration_since(declared),
                complete: self.loss.is_none(),
            });
        }
        if let Some(g) = e.got.filter(|g| healed_by(g.asked_at)) {
            let trust = self.trust();
            observations.push(Observation::Got {
                reply: Some(if g.delete {
                    Reply::Delete
                } else {
                    Reply::Put {
                        stamp_age: g
                            .stamp
                            .map(|t| StampAge::between(t.get_time().to_system_time(), wall)),
                    }
                }),
                clock: g
                    .stamp
                    .map_or(zenkey_model::freshness::ClockTrust::Untrusted, |t| {
                        trust.of(&t.get_id().to_string())
                    }),
            });
        }
        if self.archived.contains_key(key) {
            observations.push(Observation::Archived);
        }
        let latest = e.latest.as_ref().map(|l| match &l.doc {
            None => Latest::Deleted,
            Some(Ok(a)) if a.state == am::State::Firing => Latest::Firing,
            Some(Ok(_)) => Latest::Resolved,
            Some(Err(_)) => Latest::Undecodable,
        });
        let zids = self.zids(&e.owner);
        let stamp_id = e
            .latest
            .as_ref()
            .filter(|l| l.doc.is_some())
            .and_then(|l| l.stamp.as_ref())
            .map(|t| zid_value(&t.get_id().to_string()));
        let previous_instance = !zids.is_empty() && stamp_id.is_some_and(|id| !zids.contains(&id));
        let held_whole = self.owner_set(&e.owner, now).verdict == SetVerdict::Established;
        let left_out = self.left_out(key, e);
        let severity = e
            .latest
            .as_ref()
            .and_then(|l| l.doc.as_ref())
            .and_then(|d| d.as_ref().ok())
            .map(|a| a.severity);
        Some(KeyReading {
            presence,
            observations,
            latest,
            previous_instance,
            held_whole,
            left_out,
            severity,
            payload: e.latest.as_ref().map(|l| l.doc.clone()),
            alert_key: e.alarm.alert_key.clone(),
        })
    }

    /// The keys held, sorted.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(String::as_str)
    }

    /// The owners in scope, sorted: every owner this reader knows of whose
    /// alarms the scope reaches (§2.15).
    pub fn owners(&self) -> impl Iterator<Item = &Addr> {
        self.owners.keys()
    }

    /// How the reader knew of `addr`.
    pub fn known_by(&self, addr: &Addr) -> Vec<KnownBy> {
        self.owners
            .get(addr)
            .map(|o| {
                let mut k: Vec<KnownBy> = o.known.iter().copied().collect();
                // `earlier` beside `presence` says only that this read found
                // it again.
                if k.contains(&KnownBy::Presence) {
                    k.retain(|x| *x != KnownBy::Earlier);
                }
                k
            })
            .unwrap_or_default()
    }

    /// The keys of `addr` held.
    pub fn keys_of<'a>(&'a self, addr: &'a Addr) -> impl Iterator<Item = &'a str> + 'a {
        self.keys
            .iter()
            .filter(move |(_, e)| &e.owner == addr)
            .map(|(k, _)| k.as_str())
    }

    /// The latest sample of `key` as read, for a report.
    pub fn seen(&self, key: &str) -> Option<AlarmSeen> {
        let l = self.keys.get(key)?.latest.as_ref()?;
        let (latest, document, invalid) = match &l.doc {
            None => (AlarmLatest::Deleted, None, None),
            Some(Ok(a)) => (
                if a.state == am::State::Firing {
                    AlarmLatest::Firing
                } else {
                    AlarmLatest::Resolved
                },
                Some(a.clone()),
                None,
            ),
            Some(Err(e)) => (
                AlarmLatest::Undecodable,
                None,
                Some(format!("{}: {e}", e.as_str())),
            ),
        };
        Some(AlarmSeen {
            latest,
            document,
            invalid,
            stamp: l.stamp.as_ref().map(stamp_report),
            via: l.via,
        })
    }

    /// Whether `key`'s latest put is not its owner's stamp: core §4.2's
    /// foreign stamp, a finding apart (§2.14 step 5, 0.2).
    pub fn foreign_stamp(&self, key: &str, now: Instant, wall: SystemTime) -> bool {
        self.reading(key, now, wall)
            .is_some_and(|r| r.previous_instance && r.latest.is_some())
    }

    /// The archive's document of `key`, when one was read.
    pub fn last_known(&self, key: &str) -> Option<&ArchivedAlarm> {
        self.archived.get(key)
    }

    /// The archived keys whose owner holds no key here: shown as their
    /// owner's last-known alarms.
    pub fn archived_keys(&self) -> impl Iterator<Item = &str> {
        self.archived.keys().map(String::as_str)
    }

    /// §5's second question at `at_least` over the owners in scope
    /// (§2.15): each owner's set, each of its alarms' first answer with its
    /// latest document's severity, and whether the scope's reads were whole
    /// ([`AlarmBook::scope_read`]). A key refused at [`MAX_KEYS`] could be
    /// firing, so the answer is then never *no*.
    pub fn any_firing(&self, at_least: Severity, now: Instant, wall: SystemTime) -> AnyFiring {
        let rows: Vec<(SetJudged, Vec<FirstAnswer>)> = self
            .owners
            .keys()
            .map(|a| {
                let alarms = self
                    .keys_of(a)
                    .filter_map(|k| self.reading(k, now, wall))
                    .map(|r| (r.firing(), r.severity))
                    .collect();
                (self.owner_set(a, now), alarms)
            })
            .collect();
        let owners: Vec<OwnerAlarms<'_>> = rows
            .iter()
            .map(|(set, alarms)| OwnerAlarms { set: *set, alarms })
            .collect();
        let mut out = am::any_firing(at_least, &owners, self.scope_read());
        if self.refused > 0 && out.answer == Answer::No {
            out.answer = Answer::Unobservable;
            out.reason = Reason::AlarmUnobservable;
        }
        out
    }
}

/// One alarm's first answer and its latest document's severity, as
/// `any_firing` takes them.
type FirstAnswer = (Judged, Option<Severity>);

/// One key's reading at an instant (§2.14), owned: what
/// `zenkey_model::alarms`'s judges take.
#[derive(Debug, Clone)]
pub struct KeyReading {
    pub presence: Presence,
    pub observations: Vec<Observation>,
    pub latest: Option<Latest>,
    pub previous_instance: bool,
    pub held_whole: bool,
    /// The latest GET left the key out (§2.14 step 5): with `held_whole`,
    /// the sample held is none, and a loss before that GET is healed for it
    /// (step 4, text 0.5, F-136).
    pub left_out: bool,
    /// The latest document's severity, when one reads.
    pub severity: Option<Severity>,
    /// The latest sample's document: `None` for no sample, `Some(None)` for
    /// a delete.
    payload: Option<Option<std::result::Result<Alarm, Invalid>>>,
    alert_key: String,
}

impl KeyReading {
    fn as_model(&self) -> Reading<'_> {
        Reading {
            presence: self.presence,
            observations: &self.observations,
            latest: self.latest,
            previous_instance: self.previous_instance,
            held_whole: self.held_whole,
            left_out: self.left_out,
        }
    }

    /// "Is this alarm firing?" (§2.14).
    pub fn firing(&self) -> Judged {
        am::firing(&self.as_model())
    }

    /// "Is this firing alarm past its horizon, its owner present?" (§2.14).
    pub fn stale(&self) -> Judged {
        am::stale_firing(&self.as_model())
    }

    /// "Does this key re-derive from its document?" (§2.2, §2.3), of its
    /// latest sample; not asked with none, which carries no document.
    pub fn rederives(&self) -> Judged {
        match &self.payload {
            None | Some(None) => am::rederives(&self.alert_key, Payload::Deleted),
            Some(Some(Ok(a))) => am::rederives(&self.alert_key, Payload::Alarm(a)),
            Some(Some(Err(_))) => am::rederives(&self.alert_key, Payload::Unreadable),
        }
    }

    /// The latest document's problems (§2.4), as the fixtures' tokens.
    pub fn problems(&self) -> Vec<String> {
        match &self.payload {
            Some(Some(Ok(a))) => a.check().iter().map(|p| p.as_str().to_owned()).collect(),
            _ => Vec::new(),
        }
    }
}

/// A key no reading holds: nothing of it was read.
fn unread() -> Judged {
    Judged {
        answer: Answer::Unobservable,
        reason: Reason::NotRead,
    }
}

/// §2.6's transition, as the report spells it.
fn change_of(t: am::Transition) -> AlarmChange {
    match t {
        am::Transition::Raised => AlarmChange::Raised,
        am::Transition::Escalated => AlarmChange::Escalated,
        am::Transition::Refreshed => AlarmChange::Refreshed,
        am::Transition::Confirmed => AlarmChange::Confirmed,
        am::Transition::Resolved => AlarmChange::Resolved,
        am::Transition::Retired => AlarmChange::Retired,
        am::Transition::Unreadable => AlarmChange::Unreadable,
    }
}

/// A §5 answer as the report spells it.
pub fn answer_of(j: Judged) -> AlarmAnswer {
    AlarmAnswer {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: j.reason.says().to_owned(),
    }
}

/// The answer's token.
pub fn token(a: Answer) -> AlarmAnswerToken {
    match a {
        Answer::Yes => AlarmAnswerToken::Yes,
        Answer::No => AlarmAnswerToken::No,
        Answer::Unobservable => AlarmAnswerToken::Unobservable,
        Answer::NotAsked => AlarmAnswerToken::NotAsked,
    }
}

/// The set verdict's token.
pub fn set_token(v: SetVerdict) -> AlarmSetVerdict {
    match v {
        SetVerdict::Established => AlarmSetVerdict::Established,
        SetVerdict::Unobservable => AlarmSetVerdict::Unobservable,
        SetVerdict::NotAsked => AlarmSetVerdict::NotAsked,
    }
}

/// A stamp as the report spells it: RFC 3339 with nanoseconds, and the
/// clock's id.
pub fn stamp_report(t: &Timestamp) -> Stamp {
    Stamp {
        time: format!("{:#}", t.get_time()),
        clock: t.get_id().to_string(),
    }
}

#[cfg(test)]
mod tests {
    //! The book folded from values: §2.6's transitions read through the
    //! merge, §2.14's first question at each of its steps the book can
    //! reach, §2.15's window after a loss and a gap, the scope with an
    //! owner known only before, and a loss making every key unobservable.

    use super::*;
    use std::collections::BTreeMap;
    use zenoh::time::{NTP64, TimestampId};

    const A: &str = "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da";
    const ZID: &str = "a1";

    fn wall() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn stamp(age_s: u64, id: &str) -> Timestamp {
        let t = wall() - Duration::from_secs(age_s);
        Timestamp::new(
            NTP64::from(t.duration_since(SystemTime::UNIX_EPOCH).unwrap()),
            id.parse::<TimestampId>().unwrap(),
        )
    }

    fn doc(severity: &str, state: &str, timestamp: u64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "timestamp": timestamp, "rule": "link_down", "severity": severity,
            "state": state, "summary": "link to r2 down on eth0",
            "labels": {"peer": "r2", "port": "eth0"},
        }))
        .unwrap()
    }

    fn put(at: Instant, age_s: u64, payload: Vec<u8>, via: AlarmVia) -> AlarmSample {
        AlarmSample {
            key: A.into(),
            delete: false,
            payload,
            stamp: Some(stamp(age_s, ZID)),
            at,
            wall: wall(),
            via,
        }
    }

    fn presence(listed: bool, zid: Option<&str>) -> Observed {
        let keys = vec!["zk2/lab/svc/@zk/instance/000000000000000a".to_owned()];
        let mut o = Observed::from_keys("zk2/lab/*/@zk/**", &keys, true);
        let mut ds = BTreeMap::new();
        let interfaces = if listed {
            serde_json::json!([{"iface": "alarms.v1",
                "contract": format!("sha256:{}", zenkey::alarms::FINGERPRINT),
                "minor": 0, "token": false}])
        } else {
            serde_json::json!([])
        };
        let mut meta = serde_json::Map::new();
        if let Some(z) = zid {
            meta.insert("zid".into(), z.into());
        }
        let d: zenkey_model::descriptor::Descriptor = serde_json::from_value(serde_json::json!({
            "format": "zk2-descriptor/0.1", "service": "lab/svc",
            "instance": "000000000000000a", "interfaces": interfaces, "meta": meta,
        }))
        .unwrap();
        ds.insert(
            (
                "lab/svc".parse().unwrap(),
                "000000000000000a".parse().unwrap(),
            ),
            crate::model::catalog::DescriptorRead::Served(Box::new(d)),
        );
        o.descriptors = Some(ds);
        o
    }

    fn book(synced: bool) -> AlarmBook {
        let mut spec = BookSpec::new(Duration::from_secs(1));
        spec.clocks_synced = synced;
        AlarmBook::new(
            AlarmScope::parse("zk2/lab/*/alarms.v1/state/alarms/*").unwrap(),
            spec,
        )
    }

    #[test]
    fn a_scope_narrows_to_the_template_and_refuses_what_is_not_one() {
        let s = AlarmScope::parse("zk2/lab/**").unwrap();
        assert_eq!(s.alarms_selector(), "zk2/lab/*/alarms.v1/state/alarms/*");
        assert_eq!(s.instance_selector(), "zk2/lab/*/@zk/instance/*");
        assert!(s.selects_owner(&"lab/a".parse().unwrap()));
        assert!(!s.selects_owner(&"far/a".parse().unwrap()));
        let one = AlarmScope::parse(A).unwrap();
        assert_eq!(one.alarms_selector(), A);
        assert_eq!(one.one_owner(), Some("lab/svc".parse().unwrap()));
        assert_eq!(
            AlarmScope::all().presence_scope().selector(),
            "zk2/*/*/@zk/**"
        );
        assert!(
            AlarmScope::parse("acme/zk2/**").is_err(),
            "not base-relative"
        );
        assert!(
            AlarmScope::parse("zk2/lab/svc/health.v1/**").is_err(),
            "no alarm"
        );
        assert!(AlarmScope::parse("zk2/lab/ svc").is_err());
    }

    /// §2.6 through the merge: a raise, its GET reply under the same stamp
    /// kept once, an escalation, and a stale reply never regressing it.
    #[test]
    fn samples_merge_by_stamp_and_read_as_transitions() {
        let mut b = book(true);
        let t0 = Instant::now();
        b.subscribed(t0);
        b.presence_read(&presence(true, Some(ZID)), t0);
        let t = b
            .sample(put(
                t0,
                10,
                doc("warning", "firing", 1),
                AlarmVia::Subscription,
            ))
            .unwrap();
        assert_eq!(t.change, AlarmChange::Raised);
        assert_eq!(t.alarm.alert_key, "a659f813308ad1da");
        assert!(
            b.sample(put(t0, 10, doc("warning", "firing", 1), AlarmVia::Get))
                .is_none(),
            "one mutation both paths carried is kept once"
        );
        let t = b
            .sample(put(
                t0,
                5,
                doc("critical", "firing", 2),
                AlarmVia::Subscription,
            ))
            .unwrap();
        assert_eq!(t.change, AlarmChange::Escalated);
        assert!(
            b.sample(put(t0, 8, doc("warning", "firing", 1), AlarmVia::Get))
                .is_none(),
            "an older reply never regresses the key"
        );
        let r = b.reading(A, t0, wall()).unwrap();
        assert_eq!(r.severity, Some(Severity::Critical));
        assert_eq!(r.firing().reason, Reason::Firing);
        assert_eq!(r.rederives().reason, Reason::Strict);
    }

    /// §2.14: firing from a fresh document of the present instance; an
    /// earlier instance's put not asked; no `meta.zid`, unattributable and
    /// read on; absent once a complete read misses the owner.
    #[test]
    fn the_first_question_at_its_steps() {
        let t0 = Instant::now();
        let mut b = book(true);
        b.presence_read(&presence(true, Some(ZID)), t0);
        b.seeding();
        b.sample(put(t0, 10, doc("warning", "firing", 1), AlarmVia::Get));
        b.seeded(
            SeedEnd {
                asked_at: t0,
                complete: true,
            },
            t0,
            wall(),
        );
        assert_eq!(
            b.owner_set(&"lab/svc".parse().unwrap(), t0).reason,
            Reason::Answered
        );
        let j = b.reading(A, t0, wall()).unwrap().firing();
        assert_eq!((j.answer, j.reason), (Answer::Yes, Reason::Firing));

        b.presence_read(&presence(true, Some("b2")), t0);
        let j = b.reading(A, t0, wall()).unwrap().firing();
        assert_eq!(j.reason, Reason::PreviousInstance, "{j:?}");
        assert!(b.foreign_stamp(A, t0, wall()));

        b.presence_read(&presence(true, None), t0);
        assert_eq!(
            b.reading(A, t0, wall()).unwrap().firing().reason,
            Reason::Firing,
            "no meta.zid: unattributable, never foreign"
        );

        let gone = Observed::from_keys("zk2/lab/*/@zk/**", &[], true);
        b.presence_read(&gone, t0);
        let j = b.reading(A, t0, wall()).unwrap().firing();
        assert_eq!((j.answer, j.reason), (Answer::NotAsked, Reason::Absent));
        assert_eq!(b.known_by(&"lab/svc".parse().unwrap()), [KnownBy::Earlier]);
    }

    /// The clock: without the deployment's word and with no live put of
    /// the owner's clock, a GET reply's age is unobservable.
    #[test]
    fn a_reply_is_aged_only_against_a_trusted_clock() {
        let t0 = Instant::now();
        let mut b = book(false);
        b.presence_read(&presence(true, Some(ZID)), t0);
        b.seeding();
        b.sample(put(t0, 10, doc("warning", "firing", 1), AlarmVia::Get));
        b.seeded(
            SeedEnd {
                asked_at: t0,
                complete: true,
            },
            t0,
            wall(),
        );
        let j = b.reading(A, t0, wall()).unwrap().firing();
        assert_eq!(j.answer, Answer::Unobservable);
        assert_eq!(j.reason.as_str(), "clock_untrusted");
        assert_eq!(b.clock_ground(), ClockGround::None);
    }

    /// §2.15's window: a loss restarts it at the first complete GET after
    /// it, and until one comes every key and the owner are unobservable; a
    /// `Dropped(n)` reports every key `lost`, never ok.
    #[test]
    fn a_loss_makes_every_key_unobservable_until_a_get_heals_it() {
        let t0 = Instant::now();
        let mut b = book(true);
        b.subscribed(t0);
        b.presence_read(&presence(true, Some(ZID)), t0);
        b.seeding();
        b.seeded(
            SeedEnd {
                asked_at: t0,
                complete: true,
            },
            t0,
            wall(),
        );
        b.sample(put(
            t0,
            1,
            doc("warning", "firing", 1),
            AlarmVia::Subscription,
        ));
        let owner: Addr = "lab/svc".parse().unwrap();
        let later = t0 + Duration::from_secs(1000);
        assert_eq!(b.owner_set(&owner, later).reason, Reason::Window);
        let lost = b.lose(
            AlarmLoss {
                dropped: 3,
                stalled: false,
            },
            later,
            wall(),
        );
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].change, AlarmChange::Lost);
        assert_eq!(lost[0].firing.answer, AlarmAnswerToken::Unobservable);
        assert_eq!(b.owner_set(&owner, later).reason, Reason::WindowLost);
        assert!(b.loss_unhealed(), "no GET asked after it yet");
        // A GET after it heals it, and the window counts from that GET.
        let heal = later + Duration::from_secs(1);
        b.seeding();
        b.sample(put(heal, 0, doc("warning", "firing", 1), AlarmVia::Get));
        b.seeded(
            SeedEnd {
                asked_at: heal,
                complete: true,
            },
            heal,
            wall(),
        );
        assert_eq!(b.owner_set(&owner, heal).reason, Reason::Answered);
        assert!(!b.loss_unhealed(), "the complete GET after it heals it");
        assert_eq!(b.seed_complete(), Some(true));
        let w = b.window(&owner, heal + Duration::from_secs(20)).unwrap();
        assert!(w.lossless && w.present_throughout, "{w:?}");
        assert_eq!(
            w.listened,
            Duration::from_secs(9),
            "20 s less a lease and a timeout"
        );
        assert_eq!(b.lost(), 3);
    }

    /// F-131: an owner known only by an earlier reading of its own, which a
    /// complete read does not find, is in scope and not asked; the scope is
    /// then not *no*.
    #[test]
    fn an_owner_known_before_stays_in_scope_not_asked() {
        let t0 = Instant::now();
        let mut b = book(true);
        b.presence_read(&presence(true, Some(ZID)), t0);
        b.seeding();
        b.sample(put(t0, 10, doc("warning", "firing", 1), AlarmVia::Get));
        b.seeded(
            SeedEnd {
                asked_at: t0,
                complete: true,
            },
            t0,
            wall(),
        );
        let a = b.any_firing(Severity::Warning, t0, wall());
        assert_eq!((a.answer, a.firing), (Answer::Yes, 1));
        b.presence_read(&Observed::from_keys("zk2/lab/*/@zk/**", &[], true), t0);
        let a = b.any_firing(Severity::Warning, t0, wall());
        assert_eq!((a.answer, a.reason), (Answer::NotAsked, Reason::NoOwner));
        assert_eq!(a.not_asked, 1);
        let moved = b.sweep(t0, wall());
        assert_eq!(moved.len(), 1, "absent is a verdict moved with no sample");
        assert_eq!(moved[0].change, AlarmChange::Judged);
        assert_eq!(moved[0].firing.reason, "absent");
    }

    /// Core S2: an owner that answered a complete GET holds no key it left
    /// out, unless something newer showed it.
    #[test]
    fn a_key_its_owner_no_longer_answers_is_not_held() {
        let t0 = Instant::now();
        let mut b = book(true);
        b.presence_read(&presence(true, Some(ZID)), t0);
        b.seeding();
        b.sample(put(t0, 10, doc("warning", "firing", 1), AlarmVia::Get));
        b.seeded(
            SeedEnd {
                asked_at: t0,
                complete: true,
            },
            t0,
            wall(),
        );
        let t1 = t0 + Duration::from_secs(5);
        b.seeding();
        let mut other = put(t1, 1, doc("info", "firing", 3), AlarmVia::Get);
        other.key = "zk2/lab/svc/alarms.v1/state/alarms/0000000000000001".into();
        b.sample(other);
        let moved = b.seeded(
            SeedEnd {
                asked_at: t1,
                complete: true,
            },
            t1,
            wall(),
        );
        let j = b.reading(A, t1, wall()).unwrap().firing();
        assert_eq!((j.answer, j.reason), (Answer::No, Reason::NotHeld));
        assert!(
            moved
                .iter()
                .any(|t| t.key == A && t.firing.reason == "not_held")
        );
        let mismatch = b
            .reading(
                "zk2/lab/svc/alarms.v1/state/alarms/0000000000000001",
                t1,
                wall(),
            )
            .unwrap()
            .rederives();
        assert_eq!(mismatch.reason, Reason::Mismatch);
    }

    const B: &str = "zk2/lab/svc/alarms.v1/state/alarms/0000000000000001";

    /// A GET of the scope's alarms, asked at `asked_at`, answering `replies`.
    fn get(
        b: &mut AlarmBook,
        asked_at: Instant,
        replies: Vec<AlarmSample>,
    ) -> Vec<AlarmTransition> {
        b.seed(
            SeedRead {
                asked_at,
                complete: true,
                replies,
                discarded: 0,
            },
            asked_at,
            wall(),
        )
    }

    /// B, an alarm the owner still holds, as its GET answers it.
    fn b_reply(at: Instant) -> AlarmSample {
        let mut b = put(at, 1, doc("info", "firing", 3), AlarmVia::Get);
        b.key = B.into();
        b
    }

    /// A subscriber that held A firing, then lost deliveries — A's resolve
    /// and delete among them, past the tombstone window by its re-seed.
    fn lost_a_resolve() -> (AlarmBook, Instant, Instant) {
        let t0 = Instant::now();
        let mut b = book(true);
        b.subscribed(t0);
        b.presence_read(&presence(true, Some(ZID)), t0);
        get(&mut b, t0, vec![]);
        b.sample(put(
            t0 + Duration::from_secs(1),
            1,
            doc("warning", "firing", 1),
            AlarmVia::Subscription,
        ));
        let loss = t0 + Duration::from_secs(100);
        b.lose(
            AlarmLoss {
                dropped: 2,
                stalled: false,
            },
            loss,
            wall(),
        );
        let j = b.reading(A, loss, wall()).unwrap().firing();
        assert_eq!(
            (j.answer, j.reason),
            (Answer::Unobservable, Reason::Incomplete)
        );
        (b, t0, loss)
    }

    /// F-136 (text 0.5): the complete GET that establishes the owner's
    /// alarms, asked after the loss, leaves A out, and nothing of A arrives
    /// after it: A holds nothing, and the loss is healed for it — not held,
    /// reported as the verdict moving. Its stale question is not asked.
    #[test]
    fn the_establishing_get_after_a_loss_heals_a_key_it_left_out() {
        let (mut b, _, loss) = lost_a_resolve();
        let heal = loss + Duration::from_secs(1);
        // A refresh: presence read again, then the GET.
        b.presence_read(&presence(true, Some(ZID)), heal);
        let moved = get(&mut b, heal, vec![b_reply(heal)]);
        assert_eq!(
            b.owner_set(&"lab/svc".parse().unwrap(), heal).reason,
            Reason::Answered
        );
        let r = b.reading(A, heal, wall()).unwrap();
        assert!(r.held_whole && r.left_out, "{r:?}");
        let j = r.firing();
        assert_eq!((j.answer, j.reason), (Answer::No, Reason::NotHeld));
        assert_eq!(r.stale().reason, Reason::NotFiring);
        assert!(
            b.seen(A).is_none(),
            "the sample held is none (§2.14 step 5)"
        );
        assert!(
            moved.iter().any(|t| t.key == A
                && t.change == AlarmChange::Judged
                && t.firing.reason == "not_held"),
            "{moved:?}"
        );
        assert_eq!(
            b.any_firing(Severity::Info, heal, wall()).answer,
            Answer::Yes,
            "B fires"
        );
        assert_eq!(
            b.any_firing(Severity::Warning, heal, wall()).reason,
            Reason::NoneFiring,
            "A no longer stops a no"
        );
        // A put of A after that GET is a raise, read as any other.
        let again = heal + Duration::from_secs(2);
        let t = b
            .sample(put(
                again,
                0,
                doc("warning", "firing", 9),
                AlarmVia::Subscription,
            ))
            .unwrap();
        assert_eq!(t.change, AlarmChange::Raised);
    }

    /// F-136's edge: a delivery of A received at the very instant the
    /// establishing GET is asked counts as after it (every instant is the
    /// process's monotonic clock; a subscription's taken at receipt). A is
    /// not left out, and the loss stands for it: `incomplete`, never a
    /// wrong `not_held`.
    #[test]
    fn an_arrival_at_the_gets_ask_is_after_it() {
        let (mut b, _, loss) = lost_a_resolve();
        let heal = loss + Duration::from_secs(1);
        b.presence_read(&presence(true, Some(ZID)), heal);
        b.sample(put(
            heal,
            0,
            doc("warning", "firing", 9),
            AlarmVia::Subscription,
        ));
        get(&mut b, heal, vec![b_reply(heal)]);
        let r = b.reading(A, heal, wall()).unwrap();
        assert!(r.held_whole && !r.left_out, "{r:?}");
        let j = r.firing();
        assert_eq!(
            (j.answer, j.reason),
            (Answer::Unobservable, Reason::Incomplete)
        );
        assert!(b.seen(A).is_some(), "nothing dropped: A was heard");
    }

    /// F-136, the other way: a complete GET after the loss that does not
    /// establish the owner's alarms (it answers nothing, and no window is
    /// longer than 900 s yet) heals nothing for A (core O5); once its window
    /// passes 900 s, lossless, it establishes them, and A is not held.
    #[test]
    fn an_empty_get_heals_a_left_out_key_only_once_its_window_establishes() {
        let (mut b, _, loss) = lost_a_resolve();
        let owner: Addr = "lab/svc".parse().unwrap();
        let reseed = loss + Duration::from_secs(1);
        b.presence_read(&presence(true, Some(ZID)), reseed);
        get(&mut b, reseed, vec![]);
        let soon = reseed + Duration::from_secs(60);
        assert_eq!(b.owner_set(&owner, soon).reason, Reason::WindowShort);
        let j = b.reading(A, soon, wall()).unwrap().firing();
        assert_eq!(
            (j.answer, j.reason),
            (Answer::Unobservable, Reason::Incomplete)
        );
        assert_eq!(
            b.any_firing(Severity::Warning, soon, wall()).reason,
            Reason::OwnerUnobservable
        );
        let later = reseed + Duration::from_secs(1000);
        assert_eq!(b.owner_set(&owner, later).reason, Reason::Window);
        let j = b.reading(A, later, wall()).unwrap().firing();
        assert_eq!((j.answer, j.reason), (Answer::No, Reason::NotHeld));
        let moved = b.sweep(later, wall());
        assert!(moved.iter().any(|t| t.key == A), "{moved:?}");
        assert!(b.seen(A).is_none());
        assert_eq!(
            b.any_firing(Severity::Info, later, wall()).reason,
            Reason::NoneFiring
        );
    }

    /// F-136, the third way: a GET asked before the loss, though its reply
    /// arrives and its end is folded after it, establishes nothing (the
    /// window lost a delivery) and heals no key — not even one it answered,
    /// a reply being dated by its GET's ask, never by its arrival.
    #[test]
    fn a_get_asked_before_the_loss_heals_nothing() {
        let t0 = Instant::now();
        let mut b = book(true);
        b.subscribed(t0);
        b.presence_read(&presence(true, Some(ZID)), t0);
        get(&mut b, t0, vec![]);
        b.sample(put(
            t0,
            1,
            doc("warning", "firing", 1),
            AlarmVia::Subscription,
        ));
        let asked = t0 + Duration::from_secs(10);
        let loss = asked + Duration::from_millis(5);
        b.seeding();
        b.lose(
            AlarmLoss {
                dropped: 1,
                stalled: false,
            },
            loss,
            wall(),
        );
        let late = loss + Duration::from_millis(5);
        b.sample(b_reply(late));
        b.seeded(
            SeedEnd {
                asked_at: asked,
                complete: true,
            },
            late,
            wall(),
        );
        assert_eq!(
            b.owner_set(&"lab/svc".parse().unwrap(), late).reason,
            Reason::WindowLost
        );
        for k in [A, B] {
            let j = b.reading(k, late, wall()).unwrap().firing();
            assert_eq!(
                (j.answer, j.reason),
                (Answer::Unobservable, Reason::Incomplete),
                "{k}"
            );
        }
        assert!(b.seen(A).is_some(), "nothing dropped: nothing established");
    }

    /// F-137 (text 0.5): a presence read possibly incomplete may have missed
    /// an owner, so "is anything firing?" is never no over it, nor not
    /// asked; a complete read makes it no again, and a loss since — which
    /// may have taken an instance token's appearance — unobservable again.
    #[test]
    fn a_scope_whose_presence_read_was_not_whole_is_never_no() {
        let t0 = Instant::now();
        let mut b = book(true);
        let mut partial = presence(true, Some(ZID));
        partial.complete = false;
        b.presence_read(&partial, t0);
        get(&mut b, t0, vec![b_reply(t0)]);
        assert_eq!(
            b.owner_set(&"lab/svc".parse().unwrap(), t0).verdict,
            SetVerdict::Established,
            "the owner it found is known whole"
        );
        assert_eq!(b.scope_read(), ScopeRead::NotWhole);
        let a = b.any_firing(Severity::Warning, t0, wall());
        assert_eq!(
            (a.answer, a.reason, a.established),
            (Answer::Unobservable, Reason::ScopeIncomplete, 1)
        );
        assert_eq!(
            b.any_firing(Severity::Info, t0, wall()).answer,
            Answer::Yes,
            "a yes whatever else is unknown"
        );
        b.presence_read(&presence(true, Some(ZID)), t0);
        assert_eq!(b.scope_read(), ScopeRead::Whole);
        assert_eq!(
            b.any_firing(Severity::Warning, t0, wall()).reason,
            Reason::NoneFiring
        );
        b.lose(
            AlarmLoss {
                dropped: 1,
                stalled: false,
            },
            t0,
            wall(),
        );
        assert_eq!(b.scope_read(), ScopeRead::NotWhole);
        let empty = book(true);
        assert_eq!(
            empty.any_firing(Severity::Info, t0, wall()).reason,
            Reason::ScopeIncomplete,
            "no presence read made: never not asked"
        );
    }

    /// A tokenless owner (§2.13), found by its descriptor, is asked; a
    /// service whose descriptor does not list alarms.v1 is in scope, not
    /// asked.
    #[test]
    fn found_by_descriptor_never_by_token() {
        let t0 = Instant::now();
        let owner: Addr = "lab/svc".parse().unwrap();
        let mut b = book(true);
        b.presence_read(&presence(true, Some(ZID)), t0);
        assert_eq!(
            b.presence_of(&owner),
            Presence::Present(Listing::Listed { token: false })
        );
        let mut b = book(true);
        b.presence_read(&presence(false, Some(ZID)), t0);
        assert_eq!(b.owner_set(&owner, t0).reason, Reason::NotListed);
        assert_eq!(
            b.any_firing(Severity::Info, t0, wall()).reason,
            Reason::NoOwner
        );
    }

    /// Across a face (§2.11): the owner named is in scope on the
    /// deployment's word; a face closed to the alarms makes every question
    /// unobservable, never "nothing fires"; one that lets a GET through
    /// establishes the owner by its answer.
    #[test]
    fn across_a_face_the_word_stands_for_presence() {
        let t0 = Instant::now();
        let owner: Addr = "lab/svc".parse().unwrap();
        let mut spec = BookSpec::new(Duration::from_secs(1));
        spec.clocks_synced = true;
        spec.face = Some(false);
        let closed = AlarmBook::new(AlarmScope::of(&owner), spec);
        assert_eq!(closed.known_by(&owner), [KnownBy::Word]);
        let a = closed.any_firing(Severity::Info, t0, wall());
        assert_eq!(
            (a.answer, a.reason),
            (Answer::Unobservable, Reason::OwnerUnobservable)
        );
        assert_eq!(closed.owner_set(&owner, t0).reason, Reason::FaceClosed);
        spec.face = Some(true);
        let mut open = AlarmBook::new(AlarmScope::of(&owner), spec);
        open.seed(
            SeedRead {
                asked_at: t0,
                complete: true,
                replies: vec![put(t0, 10, doc("warning", "firing", 1), AlarmVia::Get)],
                discarded: 0,
            },
            t0,
            wall(),
        );
        assert_eq!(open.owner_set(&owner, t0).reason, Reason::Answered);
        assert_eq!(
            open.any_firing(Severity::Warning, t0, wall()).answer,
            Answer::Yes
        );
    }
}
