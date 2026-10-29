//! `timing.v1` (#908, MC): one tool's window over a scope's timed streams,
//! built from what the bus delivered (`spec/profiles/timing/v1.md` §2.5–§2.7).
//!
//! **Values in hand → meaning.** A [`TimingBook`] is one tool's view of the
//! timed members it held. It is fed values, never a session — a presence
//! read with the descriptors of the instances it shows, each token's move,
//! each sample with the two clock readings its subscriber's callback took
//! (an [`Instant`] on this host's monotonic clock, a [`SystemTime`] on its
//! wall clock), each re-read a withdrawal owed, each loss, and the instants
//! it judges at — so it decides nothing from a clock of its own.
//!
//! What it builds is what `zenkey_model::timing` judges, and every verdict
//! is that module's, never re-derived here:
//!
//! - **a resource's declaration** (`declaration_of`, §2.2, §2.3) and its
//!   bound (core §2.7: one for a template without parameters, the
//!   contract's `cardinality` else, none for 4294967295);
//! - **per provider address and resource, the model's [`Tracker`]** (§2.5),
//!   started at the window's start, its owner's state given as presence
//!   gives it: **present** while an interface token of the address is held
//!   (or, for an interface its descriptor marks `"token": false`, its
//!   instance token, v1.md §0), **unconfirmed** when a withdrawal heard
//!   leaves none, **absent** only when the read owed after it is complete
//!   and shows none (core §8.1, 0.37), and every member token seen
//!   (`member_seen`) or gone by a complete read (`member_gone`) on a
//!   template that declares `epoch`;
//! - **per provider address and resource, the model's [`LifespanReader`]**
//!   (§2.6), under the operator's ε (`None`: every sample `no_bound`), its
//!   owners the `meta.zid`s of the address's descriptors held;
//! - **per member**, what §2.7 asks over the window: how long it was held,
//!   each miss with the owner's state at it ([`DeadlineAccount`]), whether
//!   it ended, its arrivals for `period_window`, and its samples' lifespans.
//!
//! **One order, instants that never go back.** The bus folds every
//! delivery in the one order its queue received it, and the book takes each
//! event at the later of its own instant and the last one's: a tracker
//! judges at an instant, then applies the event, and an instant earlier
//! than one it already judged at would undo a judgement made. The bus
//! drains its queue before it hands the book a tick or a read's answer, so
//! what the clamp moves is the microseconds between a callback's reading
//! and the drain.
//!
//! **Across a constrained face** (§2.8) no presence is read: every owner is
//! unseen, no known member is waited for, and a member ends only by being
//! forgotten; §5's deadline and period read unobservable there, and the
//! lifespan is judged as anywhere, on the `meta.zid`s the deployment's word
//! gives, since a descriptor does not cross either (`@zk`, link.v1).
//!
//! **Bounded.** At most [`MAX_MEMBERS`] members are held over the book (one
//! past it is refused and counted), each keeps at most [`MAX_ARRIVALS`]
//! arrivals for its period (one past it makes its period unobservable,
//! `lost`, never a guess), at most [`MAX_MISSES`] misses listed beside
//! their counts, and the first sample of each lifespan verdict, which is
//! every sample `lifespan_expired` reads.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::authoring::Kind;
use zenkey_model::freshness::StampAge;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};
use zenkey_model::template::Segment;
use zenkey_model::timing::{
    DeadlineAccount, Declaration, EndReason, Event, LifespanReader, MemberState, Mono, OwnerState,
    PeriodReading, Report, Sample, SampleVerdict, Stamped, Tracker, Window, declaration_of,
    period_window, zid_value,
};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::time::Timestamp;

use crate::model::catalog::{Catalog, Observed, Revision};

/// The scope a timing reading reaches: a base-relative selector in the
/// deployment's namespace, or every timed resource the descriptors in scope
/// declare, read exactly as `telemetry.v1`'s reading reads its scope.
pub use crate::model::telemetry::TelemetryScope as TimingScope;

/// The most members a book holds.
pub const MAX_MEMBERS: usize = 4_096;

/// The most arrivals a member keeps for its period: an hour's window at a
/// 4 ms period.
pub const MAX_ARRIVALS: usize = 1 << 20;

/// The most misses a member lists; every one is counted.
pub const MAX_MISSES: usize = 16;

/// The resource chunks of a key start after `zk2/<system>/<service>/<iface>/<kind>`.
const RESOURCE_AT: usize = 5;

// ─── a timed resource of one owner ──────────────────────────────────────────

/// One timed resource of one owner (§0): a stream whose resolved
/// annotations hold a key of the vocabulary, valid or not.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedResource {
    pub address: Addr,
    pub iface: IfaceId,
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// Always a stream: a key on anything else is ignored (§2.3), and no
    /// question is asked of it.
    pub kind: Kind,
    pub declaration: Declaration,
    /// The resource's bound (core §2.7), `None` for no bound.
    pub bound: Option<u64>,
    /// The template declares `epoch`: the resource chunk its parameter sits
    /// at (core §2.3).
    pub epoch_at: Option<usize>,
    /// Its one member's key, for a template without parameters: a known
    /// member (§0), the provider's address being exact.
    pub known: Option<String>,
    /// Its members' key expression, base-relative: every parameter `*`, a
    /// rest parameter `**`.
    pub selector: String,
}

impl TimedResource {
    /// The resource at `index` of `revision`, as `address` provides it;
    /// `None` when it is not this profile's or not a stream (a key on a
    /// state, an event or an operation means nothing, §2.3, and is the
    /// doctor's `timing-declaration`).
    pub fn of(address: &Addr, revision: &Revision, index: usize) -> Option<TimedResource> {
        let r = revision.contract().resources.get(index)?;
        if r.kind != Kind::Stream {
            return None;
        }
        let declaration = declaration_of(r);
        if !matches!(
            declaration,
            Declaration::Declared(_) | Declaration::Invalid(_)
        ) {
            return None;
        }
        let segments = r.template.segments();
        let literal = segments.iter().all(|s| matches!(s, Segment::Literal(_)));
        let bound = if literal {
            Some(1)
        } else {
            r.cardinality.filter(|c| *c != u64::from(u32::MAX))
        };
        let epoch_at = r.epoch.as_ref().and_then(|p| {
            segments
                .iter()
                .position(|s| matches!(s, Segment::Param(n) if n == p))
        });
        let chunks: Vec<&str> = segments
            .iter()
            .map(|s| match s {
                Segment::Literal(l) => l.as_str(),
                Segment::Param(_) => "*",
                Segment::Rest(_) => "**",
            })
            .collect();
        let selector = format!(
            "zk2/{}/{}/{}/{}/{}",
            address.system,
            address.service,
            revision.iface(),
            r.token.as_str(),
            chunks.join("/")
        );
        Some(TimedResource {
            address: address.clone(),
            iface: revision.iface().clone(),
            fingerprint: revision.fingerprint().to_string(),
            resource: zenkey::implementation::resource_name(r),
            kind: r.kind,
            declaration,
            bound,
            epoch_at,
            known: literal.then(|| selector.clone()),
            selector,
        })
    }

    /// A resource from its parts, for a reader that holds no contract (a
    /// test, a replay with the declaration in hand).
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        address: Addr,
        iface: IfaceId,
        resource: &str,
        declaration: Declaration,
        bound: Option<u64>,
        epoch_at: Option<usize>,
        selector: &str,
        known: bool,
    ) -> TimedResource {
        TimedResource {
            address,
            iface,
            fingerprint: String::new(),
            resource: resource.to_owned(),
            kind: Kind::Stream,
            declaration,
            bound,
            epoch_at,
            known: known.then(|| selector.to_owned()),
            selector: selector.to_owned(),
        }
    }

    /// The member chunk a key carries at the template's `epoch` parameter:
    /// the member its member tokens name (core §8.1).
    pub fn epoch_chunk<'k>(&self, key: &'k str) -> Option<&'k str> {
        key.split('/').nth(RESOURCE_AT + self.epoch_at?)
    }

    /// The deadline judged, when one is (§2.5): a valid declaration's, not
    /// below its period.
    pub fn deadline(&self) -> Option<Duration> {
        self.declaration.declared()?.judged_deadline()
    }

    /// The period declared, when one is valid.
    pub fn period(&self) -> Option<Duration> {
        self.declaration.declared()?.period
    }

    /// Whether `key` is one of its members.
    pub fn holds_key(&self, key: &str) -> bool {
        OwnedKeyExpr::try_from(self.selector.clone())
            .ok()
            .zip(zenoh::key_expr::KeyExpr::try_from(key).ok())
            .is_some_and(|(sel, k)| !k.is_wild() && sel.includes(&k))
    }
}

/// The timed resources a revision declares, as `address` provides it.
pub fn timed_resources(address: &Addr, revision: &Revision) -> Vec<TimedResource> {
    (0..revision.contract().resources.len())
        .filter_map(|i| TimedResource::of(address, revision, i))
        .collect()
}

// ─── the inputs ─────────────────────────────────────────────────────────────

/// One sample, as a subscriber's callback took it.
#[derive(Debug, Clone)]
pub struct TimedSample {
    /// The concrete key, base-relative.
    pub key: String,
    /// A delete: no arrival (§2.5).
    pub delete: bool,
    pub stamp: Option<Timestamp>,
    /// Its arrival, on this host's monotonic clock, read in the callback.
    pub arrival: Instant,
    /// This host's wall clock, read in the callback with `arrival` (§2.6).
    pub wall: SystemTime,
}

/// A read a withdrawal owes (core §8.1, 0.37): made again a while after it,
/// on a session holding no liveliness subscriber.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owed {
    /// The owner's interface tokens of `iface` at `addr`, none left held.
    Owner(Addr, IfaceId),
    /// The member tokens of `member` of `iface` at `addr`, none left held.
    Member(Addr, IfaceId, String),
}

impl Owed {
    /// The liveliness selector the read asks, base-relative.
    pub fn selector(&self) -> String {
        match self {
            Owed::Owner(a, i) => format!("zk2/{}/{}/@zk/alive/{i}/**", a.system, a.service),
            Owed::Member(a, i, m) => {
                format!("zk2/{}/{}/@zk/member/{i}/{m}/*", a.system, a.service)
            }
        }
    }
}

// ─── the book ───────────────────────────────────────────────────────────────

/// What one member's window showed: see the module doc.
#[derive(Debug, Clone)]
pub struct MemberWindow {
    /// The index of its resource in [`TimingBook::resources`].
    pub resource: usize,
    pub known: bool,
    /// How long it was held in the window (§2.7), so far.
    pub held: Duration,
    held_from: Option<Mono>,
    /// The tracker's account: misses by the owner's state, and whether it
    /// ended.
    pub account: DeadlineAccount,
    /// The misses reported, at most [`MAX_MISSES`], oldest first.
    pub misses: Vec<Report>,
    /// `met` reports.
    pub mets: u32,
    /// Its first end in the window, with why.
    pub ended: Option<(Mono, EndReason)>,
    /// Its arrivals in the window, for its period: at most [`MAX_ARRIVALS`].
    pub arrivals: Vec<Mono>,
    /// Arrivals past [`MAX_ARRIVALS`]: its period is then unobservable.
    pub arrivals_dropped: u64,
    /// The first sample of each lifespan verdict, in arrival order: what
    /// `lifespan_expired` reads.
    pub samples: Vec<Sample>,
    /// Every sample's lifespan, counted by verdict, then by reason.
    pub lifespans: BTreeMap<(&'static str, &'static str), u64>,
    /// Deletes received: no arrival.
    pub deletes: u64,
}

impl MemberWindow {
    fn new(resource: usize, known: bool) -> MemberWindow {
        MemberWindow {
            resource,
            known,
            held: Duration::ZERO,
            held_from: None,
            account: DeadlineAccount::default(),
            misses: Vec::new(),
            mets: 0,
            ended: None,
            arrivals: Vec::new(),
            arrivals_dropped: 0,
            samples: Vec::new(),
            lifespans: BTreeMap::new(),
            deletes: 0,
        }
    }

    /// The tracker's account with the span held, as §5's deadline reads
    /// it (§2.7).
    pub fn deadline_account(&self) -> DeadlineAccount {
        DeadlineAccount {
            held: self.held,
            ..self.account
        }
    }

    /// Its misses, by the owner's state at each.
    pub fn missed(&self) -> u32 {
        self.account.missed_present + self.account.missed_not_present
    }

    fn sample(&mut self, s: Sample) {
        *self
            .lifespans
            .entry((s.verdict.as_str(), s.reason.as_str()))
            .or_default() += 1;
        if !self.samples.iter().any(|x| x.verdict == s.verdict) {
            self.samples.push(s);
        }
    }

    fn hold(&mut self, held: Option<Mono>, at: Mono) {
        match (self.held_from, held) {
            (None, Some(from)) => self.held_from = Some(from.min(at)),
            (Some(from), None) => {
                self.held += at.saturating_since(from);
                self.held_from = None;
            }
            _ => {}
        }
    }
}

/// One provider's resource: its tracker and its lifespan reader.
#[derive(Debug, Clone)]
struct Judging {
    tracker: Option<Tracker>,
    lifespan: LifespanReader,
    /// Each held key's epoch chunk.
    epochs: BTreeMap<String, String>,
    /// The epoch chunks a member token of which was seen and not since
    /// shown gone (F-261).
    seen: BTreeSet<String>,
}

/// An owner, `(address, interface)`, as presence gives it.
#[derive(Debug, Clone, Default)]
struct OwnerFollow {
    state: OwnerState,
    /// The tokens held that make it present: interface tokens, or instance
    /// tokens for a tokenless interface.
    held: BTreeSet<String>,
    /// The interface is marked `"token": false` by a descriptor held.
    tokenless: bool,
    /// Present at every instant of the window so far.
    present_throughout: bool,
}

/// One tool's window over the timed resources in scope: see the module doc.
#[derive(Debug, Clone)]
pub struct TimingBook {
    t0: Instant,
    last: Mono,
    resources: Vec<TimedResource>,
    /// By resource index.
    judging: Vec<Judging>,
    /// Each key resolved to its resource, or to none.
    resolved: BTreeMap<String, Option<usize>>,
    /// The book reads presence (no face).
    follows: bool,
    owners: BTreeMap<(Addr, IfaceId), OwnerFollow>,
    /// Each (address, interface, member chunk)'s member tokens held.
    member_tokens: BTreeMap<(Addr, IfaceId, String), BTreeSet<String>>,
    /// Each address's `meta.zid`s, by instance.
    zids: BTreeMap<Addr, BTreeMap<InstanceId, u128>>,
    /// The deployment's word for an address's `meta.zid`s (§2.8).
    stated: BTreeMap<Addr, BTreeSet<u128>>,
    started: Option<Mono>,
    ended: Option<Mono>,
    members: BTreeMap<String, MemberWindow>,
    lost: u64,
    refused: u64,
    unresolved: u64,
    beyond_bound: BTreeMap<usize, BTreeSet<String>>,
    epsilon: Option<Duration>,
}

impl TimingBook {
    /// An empty book over `resources`, under ε `epsilon` (`None`: none
    /// stated), following presence unless `across_face`. Instants count
    /// from `t0`.
    pub fn new(
        resources: Vec<TimedResource>,
        epsilon: Option<Duration>,
        across_face: bool,
        t0: Instant,
    ) -> TimingBook {
        let judging = resources
            .iter()
            .map(|r| Judging {
                tracker: zenkey_model::timing::tracker_for(
                    r.kind,
                    &r.declaration,
                    r.bound,
                    r.known.clone(),
                )
                .map(|t| {
                    if r.epoch_at.is_some() {
                        t.with_epoch()
                    } else {
                        t
                    }
                }),
                lifespan: LifespanReader::new(r.kind, &r.declaration, epsilon, []),
                epochs: BTreeMap::new(),
                seen: BTreeSet::new(),
            })
            .collect();
        let mut owners = BTreeMap::new();
        for r in &resources {
            owners
                .entry((r.address.clone(), r.iface.clone()))
                .or_insert_with(OwnerFollow::default);
        }
        TimingBook {
            t0,
            last: Mono::ZERO,
            judging,
            resolved: BTreeMap::new(),
            follows: !across_face,
            owners,
            member_tokens: BTreeMap::new(),
            zids: BTreeMap::new(),
            stated: BTreeMap::new(),
            started: None,
            ended: None,
            members: BTreeMap::new(),
            lost: 0,
            refused: 0,
            unresolved: 0,
            beyond_bound: BTreeMap::new(),
            epsilon,
            resources,
        }
    }

    /// The timed resources, in the order given.
    pub fn resources(&self) -> &[TimedResource] {
        &self.resources
    }

    /// The ε stated.
    pub fn epsilon(&self) -> Option<Duration> {
        self.epsilon
    }

    /// Whether the book follows presence.
    pub fn follows_presence(&self) -> bool {
        self.follows
    }

    /// The window's start, once it started.
    pub fn started(&self) -> Option<Mono> {
        self.started
    }

    /// The window's end, once it ended.
    pub fn finished(&self) -> Option<Mono> {
        self.ended
    }

    /// How long the window ran.
    pub fn window(&self) -> Duration {
        match (self.started, self.ended) {
            (Some(s), Some(e)) => e.saturating_since(s),
            _ => Duration::ZERO,
        }
    }

    /// Every member held, by key.
    pub fn members(&self) -> impl Iterator<Item = (&String, &MemberWindow)> {
        self.members.iter()
    }

    /// Deliveries lost and stalls, so far.
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// Members refused at [`MAX_MEMBERS`].
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Samples on a key no timed resource holds, or a wildcard key (R6).
    pub fn unresolved(&self) -> u64 {
        self.unresolved
    }

    /// The members of resource `i` reported past its bound.
    pub fn beyond_bound(&self, i: usize) -> impl Iterator<Item = &String> {
        self.beyond_bound.get(&i).into_iter().flatten()
    }

    /// The owner's state of `iface` at `addr`, as presence last gave it.
    pub fn owner(&self, addr: &Addr, iface: &IfaceId) -> OwnerState {
        self.owners
            .get(&(addr.clone(), iface.clone()))
            .map_or(OwnerState::Unseen, |o| o.state)
    }

    /// Whether the owner of `iface` at `addr` was present (§0) at every
    /// instant of the window: present at its start, and at no instant after
    /// it unconfirmed, absent or unseen.
    pub fn owner_present_throughout(&self, addr: &Addr, iface: &IfaceId) -> bool {
        self.started.is_some()
            && self
                .owners
                .get(&(addr.clone(), iface.clone()))
                .is_some_and(|o| o.present_throughout)
    }

    /// Whether a descriptor held marks `iface` at `addr` `"token": false`:
    /// its owner's token is then its instance token (v1.md §0).
    pub fn tokenless(&self, addr: &Addr, iface: &IfaceId) -> bool {
        self.owners
            .get(&(addr.clone(), iface.clone()))
            .is_some_and(|o| o.tokenless)
    }

    /// The `meta.zid`s held of `addr`'s instances.
    pub fn owner_zids(&self, addr: &Addr) -> BTreeSet<u128> {
        self.zids
            .get(addr)
            .into_iter()
            .flat_map(|z| z.values().copied())
            .chain(self.stated.get(addr).into_iter().flatten().copied())
            .collect()
    }

    /// The instant `at` on the book's clock, never before the last event's.
    fn mono(&mut self, at: Instant) -> Mono {
        let m = Mono::after(at.saturating_duration_since(self.t0));
        self.last = self.last.max(m);
        self.last
    }

    /// An instant on the book's clock as an [`Instant`].
    pub fn instant(&self, m: Mono) -> Instant {
        self.t0 + m.since_origin()
    }

    /// Milliseconds from the window's start to `m`.
    pub fn ms_since_start(&self, m: Mono) -> f64 {
        let s = self.started.unwrap_or(Mono::ZERO);
        let d = i128::try_from(m.since_origin().as_nanos()).unwrap_or(i128::MAX)
            - i128::try_from(s.since_origin().as_nanos()).unwrap_or(i128::MAX);
        d as f64 / 1e6
    }

    fn resource_of(&mut self, key: &str) -> Option<usize> {
        if let Some(r) = self.resolved.get(key) {
            return *r;
        }
        let r = self.resources.iter().position(|r| r.holds_key(key));
        if self.resolved.len() < 2 * MAX_MEMBERS {
            self.resolved.insert(key.to_owned(), r);
        }
        r
    }

    /// Hands `event` to resource `i`'s tracker at `at`, then counts what it
    /// reported and moves each member's held span.
    fn observe(&mut self, i: usize, at: Mono, event: Event<'_>) {
        let Some(t) = self.judging[i].tracker.as_mut() else {
            return;
        };
        let reports = t.observe(at, event);
        let in_window = self.started.is_some() && self.ended.is_none();
        for r in reports {
            let key = r.member().to_owned();
            if let Report::BeyondBound { .. } = r {
                if in_window {
                    self.beyond_bound.entry(i).or_default().insert(key);
                }
                continue;
            }
            if !in_window {
                continue;
            }
            let Some(m) = self.member_entry(&key, i) else {
                continue;
            };
            m.account.count(&r);
            match &r {
                Report::Missed { .. } if m.misses.len() < MAX_MISSES => m.misses.push(r.clone()),
                Report::Met { .. } => m.mets += 1,
                Report::Ended { at, reason, .. } if m.ended.is_none() => {
                    m.ended = Some((*at, *reason));
                }
                _ => {}
            }
        }
        if in_window {
            self.restate_held(i, at);
        }
    }

    /// Each member of resource `i`'s held span, from its tracker's state.
    fn restate_held(&mut self, i: usize, at: Mono) {
        let Some(t) = self.judging[i].tracker.as_ref() else {
            return;
        };
        let mut states: Vec<(String, Option<MemberState>)> =
            t.held().map(|k| (k.to_owned(), t.state(k))).collect();
        for (k, m) in &self.members {
            if m.resource == i && !states.iter().any(|(s, _)| s == k) {
                states.push((k.clone(), None));
            }
        }
        for (key, state) in states {
            let held = match state {
                Some(MemberState::Waiting { since }) => Some(since),
                Some(MemberState::Heard { last }) => Some(last),
                Some(MemberState::Missed { .. }) => Some(at),
                Some(MemberState::Idle { .. }) | None => None,
            };
            if held.is_none() && !self.members.contains_key(&key) {
                continue;
            }
            if let Some(m) = self.member_entry(&key, i) {
                m.hold(held, at);
            }
        }
    }

    /// The member's window, made when it is first held; `None` past
    /// [`MAX_MEMBERS`], counted.
    fn member_entry(&mut self, key: &str, i: usize) -> Option<&mut MemberWindow> {
        if !self.members.contains_key(key) {
            if self.members.len() >= MAX_MEMBERS {
                self.refused += 1;
                return None;
            }
            let known = self.resources[i].known.as_deref() == Some(key);
            self.members
                .insert(key.to_owned(), MemberWindow::new(i, known));
        }
        self.members.get_mut(key)
    }

    /// Gives every tracker of `(addr, iface)` the owner's state.
    fn tell_owner(&mut self, addr: &Addr, iface: &IfaceId, at: Mono) {
        let state = self.owner(addr, iface);
        if self.started.is_some()
            && self.ended.is_none()
            && state != OwnerState::Present
            && let Some(o) = self.owners.get_mut(&(addr.clone(), iface.clone()))
        {
            o.present_throughout = false;
        }
        let of: Vec<usize> = (0..self.resources.len())
            .filter(|i| self.resources[*i].address == *addr && self.resources[*i].iface == *iface)
            .collect();
        for i in of {
            self.observe(i, at, Event::Owner(state));
        }
    }

    fn set_owner(&mut self, addr: &Addr, iface: &IfaceId, state: OwnerState, at: Mono) {
        let Some(o) = self.owners.get_mut(&(addr.clone(), iface.clone())) else {
            return;
        };
        o.state = state;
        self.tell_owner(addr, iface, at);
    }

    /// Every resource of `(addr, iface)` declaring `epoch`: `chunk`'s member
    /// token seen.
    fn member_seen(&mut self, addr: &Addr, iface: &IfaceId, chunk: &str, at: Mono) {
        for i in 0..self.resources.len() {
            let r = &self.resources[i];
            if r.address != *addr || r.iface != *iface || r.epoch_at.is_none() {
                continue;
            }
            let j = &mut self.judging[i];
            j.seen.insert(chunk.to_owned());
            let mut keys: Vec<String> = j
                .epochs
                .iter()
                .filter(|(_, c)| c.as_str() == chunk)
                .map(|(k, _)| k.clone())
                .collect();
            if let Some(k) = &r.known
                && r.epoch_chunk(k) == Some(chunk)
                && !keys.contains(k)
            {
                keys.push(k.clone());
            }
            for k in keys {
                self.observe(i, at, Event::MemberSeen(&k));
            }
        }
    }

    /// Every resource of `(addr, iface)` declaring `epoch`: `chunk`'s member
    /// tokens gone by a complete read.
    fn member_gone(&mut self, addr: &Addr, iface: &IfaceId, chunk: &str, at: Mono) {
        for i in 0..self.resources.len() {
            let r = &self.resources[i];
            if r.address != *addr || r.iface != *iface || r.epoch_at.is_none() {
                continue;
            }
            let j = &mut self.judging[i];
            if !j.seen.remove(chunk) {
                continue;
            }
            let mut keys: Vec<String> = j
                .epochs
                .iter()
                .filter(|(_, c)| c.as_str() == chunk)
                .map(|(k, _)| k.clone())
                .collect();
            if let Some(k) = &r.known
                && r.epoch_chunk(k) == Some(chunk)
                && !keys.contains(k)
            {
                keys.push(k.clone());
            }
            for k in keys {
                self.observe(i, at, Event::MemberGone(&k));
            }
        }
    }

    /// The `meta.zid`s of the instances `catalog`'s descriptors name, and
    /// which interfaces they mark tokenless: held, and handed to every
    /// lifespan reader of their addresses.
    pub fn descriptors(&mut self, catalog: &Catalog) {
        for d in catalog.descriptors() {
            let (Ok(addr), Ok(instance)) = (d.service.parse::<Addr>(), d.instance.parse()) else {
                continue;
            };
            if let Some(z) = d
                .meta
                .get("zid")
                .and_then(|v| v.as_str())
                .and_then(zid_value)
            {
                self.zids
                    .entry(addr.clone())
                    .or_default()
                    .insert(instance, z);
            }
            for e in &d.interfaces {
                if let Ok(iface) = e.iface.parse::<IfaceId>()
                    && let Some(o) = self.owners.get_mut(&(addr.clone(), iface))
                {
                    o.tokenless |= !e.token;
                }
            }
        }
        self.restate_zids();
    }

    /// The deployment's word for `addr`'s `meta.zid`s, where no descriptor
    /// can be read (across a face, §2.8).
    pub fn stated_zids(&mut self, addr: &Addr, zids: impl IntoIterator<Item = u128>) {
        self.stated.entry(addr.clone()).or_default().extend(zids);
        self.restate_zids();
    }

    fn restate_zids(&mut self) {
        for i in 0..self.resources.len() {
            let zids = self.owner_zids(&self.resources[i].address);
            self.judging[i].lifespan.set_owners(zids);
        }
    }

    /// One presence read, its descriptors read with it (or `None`): every
    /// owner present where it shows the owner's token, absent where it is
    /// complete and shows none, and every member token it shows seen. A
    /// read not complete shows presence, never absence. `settled` is whether
    /// the watch session's round trip stood before it (#851).
    pub fn seed(&mut self, read: &Observed, settled: bool, at: Instant) {
        if !self.follows {
            return;
        }
        let at = self.mono(at);
        if read.descriptors.is_some() {
            self.descriptors(&Catalog::new(read));
        }
        let whole = read.complete && settled;
        let mut found: BTreeMap<(Addr, IfaceId), BTreeSet<String>> = BTreeMap::new();
        let mut members: Vec<(Addr, IfaceId, String, String)> = Vec::new();
        let instances: Vec<(Addr, String)> = read
            .tokens
            .iter()
            .filter_map(|t| match t {
                ZkKey::Instance { addr, instance } => {
                    Some((addr.clone(), format!("instance/{instance}")))
                }
                _ => None,
            })
            .collect();
        for t in &read.tokens {
            match t {
                ZkKey::Alive { addr, iface, .. } => {
                    if let Ok(k) = t.to_key() {
                        found
                            .entry((addr.clone(), iface.clone()))
                            .or_default()
                            .insert(k.as_str().to_owned());
                    }
                }
                ZkKey::Member {
                    addr,
                    iface,
                    member,
                    ..
                } => {
                    if let Ok(k) = t.to_key() {
                        members.push((
                            addr.clone(),
                            iface.clone(),
                            member.clone(),
                            k.as_str().to_owned(),
                        ));
                    }
                }
                _ => {}
            }
        }
        let owners: Vec<((Addr, IfaceId), bool)> = self
            .owners
            .iter()
            .map(|(k, o)| (k.clone(), o.tokenless))
            .collect();
        for ((addr, iface), tokenless) in owners {
            let shown: BTreeSet<String> = if tokenless {
                instances
                    .iter()
                    .filter(|(a, _)| *a == addr)
                    .map(|(_, k)| k.clone())
                    .collect()
            } else {
                found
                    .remove(&(addr.clone(), iface.clone()))
                    .unwrap_or_default()
            };
            let present = !shown.is_empty();
            if !present && !whole {
                continue;
            }
            if let Some(o) = self.owners.get_mut(&(addr.clone(), iface.clone())) {
                o.held = shown;
            }
            let state = if present {
                OwnerState::Present
            } else {
                OwnerState::Absent
            };
            self.set_owner(&addr, &iface, state, at);
        }
        let mut shown_members: BTreeSet<(Addr, IfaceId, String)> = BTreeSet::new();
        for (addr, iface, member, key) in members {
            self.member_tokens
                .entry((addr.clone(), iface.clone(), member.clone()))
                .or_default()
                .insert(key);
            shown_members.insert((addr.clone(), iface.clone(), member.clone()));
            self.member_seen(&addr, &iface, &member, at);
        }
        if whole {
            // A member token held before and shown by none in a complete
            // read: gone (core §8.1).
            let gone: Vec<(Addr, IfaceId, String)> = self
                .member_tokens
                .keys()
                .filter(|k| !shown_members.contains(*k))
                .cloned()
                .collect();
            for (a, i, m) in gone {
                self.member_tokens
                    .remove(&(a.clone(), i.clone(), m.clone()));
                self.member_gone(&a, &i, &m, at);
            }
        }
    }

    /// One token's move, as the watch session heard it, base-relative.
    /// Returns the read it owes, when a withdrawal left nothing held.
    pub fn token(&mut self, key: &str, held: bool, at: Instant) -> Option<Owed> {
        if !self.follows {
            return None;
        }
        let at = self.mono(at);
        match zenkey_model::grammar::parse(key) {
            Ok(ZkKey::Alive { addr, iface, .. }) => {
                let o = self.owners.get_mut(&(addr.clone(), iface.clone()))?;
                if o.tokenless {
                    return None;
                }
                self.owner_token(addr, iface, key, held, at)
            }
            Ok(ZkKey::Instance { addr, instance }) => {
                let tokenless: Vec<IfaceId> = self
                    .owners
                    .iter()
                    .filter(|((a, _), o)| *a == addr && o.tokenless)
                    .map(|((_, i), _)| i.clone())
                    .collect();
                let mut owed = None;
                for iface in tokenless {
                    owed = owed.or(self.owner_token(
                        addr.clone(),
                        iface,
                        &format!("instance/{instance}"),
                        held,
                        at,
                    ));
                }
                // A gone instance's zid is kept: a sample of its in transit
                // is still its own (§2.6).
                let _ = instance;
                owed
            }
            Ok(ZkKey::Member {
                addr,
                iface,
                member,
                ..
            }) => {
                if !self.owners.contains_key(&(addr.clone(), iface.clone())) {
                    return None;
                }
                // Heard declared or withdrawn, a token of it was held
                // (F-261).
                self.member_seen(&addr, &iface, &member, at);
                let k = (addr.clone(), iface.clone(), member.clone());
                let tokens = self.member_tokens.entry(k).or_default();
                if held {
                    tokens.insert(key.to_owned());
                    None
                } else {
                    tokens.remove(key);
                    tokens
                        .is_empty()
                        .then_some(Owed::Member(addr, iface, member))
                }
            }
            _ => None,
        }
    }

    fn owner_token(
        &mut self,
        addr: Addr,
        iface: IfaceId,
        key: &str,
        held: bool,
        at: Mono,
    ) -> Option<Owed> {
        let o = self.owners.get_mut(&(addr.clone(), iface.clone()))?;
        if held {
            o.held.insert(key.to_owned());
            self.set_owner(&addr, &iface, OwnerState::Present, at);
            return None;
        }
        o.held.remove(key);
        if !o.held.is_empty() {
            return None;
        }
        if o.state == OwnerState::Present {
            self.set_owner(&addr, &iface, OwnerState::Unconfirmed, at);
        }
        Some(Owed::Owner(addr, iface))
    }

    /// The read a withdrawal owed (core §8.1, 0.37), its tokens base-relative:
    /// a token shown is presence, complete and empty (its watch session's
    /// round trip standing) is absence, and anything else is owed again.
    pub fn owed_read(
        &mut self,
        owed: &Owed,
        keys: &[String],
        complete: bool,
        at: Instant,
    ) -> Option<Owed> {
        let at = self.mono(at);
        match owed {
            Owed::Owner(addr, iface) => {
                let o = self.owners.get_mut(&(addr.clone(), iface.clone()))?;
                if !o.held.is_empty() {
                    return None;
                }
                if !keys.is_empty() {
                    o.held = keys.iter().cloned().collect();
                    self.set_owner(addr, iface, OwnerState::Present, at);
                    None
                } else if complete {
                    self.set_owner(addr, iface, OwnerState::Absent, at);
                    None
                } else {
                    Some(owed.clone())
                }
            }
            Owed::Member(addr, iface, member) => {
                let k = (addr.clone(), iface.clone(), member.clone());
                if self.member_tokens.get(&k).is_some_and(|t| !t.is_empty()) {
                    return None;
                }
                if !keys.is_empty() {
                    self.member_tokens.insert(k, keys.iter().cloned().collect());
                    None
                } else if complete {
                    self.member_tokens.remove(&k);
                    self.member_gone(addr, iface, member, at);
                    None
                } else {
                    Some(owed.clone())
                }
            }
        }
    }

    /// A loss: `n` deliveries the reader did not receive (`n` 0 for a
    /// stall of its receive path, or a move of its watch session's
    /// transport, which may have lost anything). Every owner held present
    /// is unconfirmed until the next read (core §8.1, 0.42).
    pub fn lose(&mut self, n: u64, at: Instant) {
        self.lost += n.max(1);
        let at = self.mono(at);
        let present: Vec<(Addr, IfaceId)> = self
            .owners
            .iter()
            .filter(|(_, o)| o.state == OwnerState::Present)
            .map(|(k, _)| k.clone())
            .collect();
        for (a, i) in present {
            self.set_owner(&a, &i, OwnerState::Unconfirmed, at);
        }
    }

    /// The window opens: every tracker's subscription start (§2.7: the
    /// window's start is its subscription's start for §2.5).
    pub fn start(&mut self, at: Instant) {
        if self.started.is_some() {
            return;
        }
        let at = self.mono(at);
        self.started = Some(at);
        for o in self.owners.values_mut() {
            o.present_throughout = o.state == OwnerState::Present;
        }
        for i in 0..self.resources.len() {
            // A known member is asked about whatever arrives (§2.7): its
            // row stands from the start.
            if let Some(k) = self.resources[i].known.clone() {
                let _ = self.member_entry(&k, i);
            }
            self.observe(i, at, Event::Start);
        }
    }

    /// One sample, at the instants its callback read. Outside the window
    /// it is nothing.
    pub fn sample(&mut self, s: &TimedSample) {
        let at = self.mono(s.arrival);
        if self.started.is_none() || self.ended.is_some() {
            return;
        }
        let Some(i) = self.resource_of(&s.key) else {
            self.unresolved += 1;
            return;
        };
        let r = &self.resources[i];
        let chunk = r.epoch_chunk(&s.key).map(str::to_owned);
        let has_tracker = self.judging[i].tracker.is_some();
        // A member is held when its tracker holds it; with no tracker,
        // every member heard is.
        if !has_tracker && self.member_entry(&s.key, i).is_none() {
            return;
        }
        if s.delete {
            if let Some(m) = self.members.get_mut(&s.key) {
                m.deletes += 1;
            }
            return;
        }
        // §2.6: the lifespan first; an expired sample is no arrival.
        let stamped = s.stamp.and_then(|t| {
            Some(Stamped {
                age: StampAge::between(t.get_time().to_system_time(), s.wall),
                id: zenkey::timing::stamp_id(&t)?,
            })
        });
        let judged = self.judging[i].lifespan.judge(stamped);
        let expired = judged.verdict == SampleVerdict::Expired;
        if has_tracker {
            if let Some(c) = chunk {
                self.judging[i].epochs.insert(s.key.clone(), c.clone());
                // A member token of it seen before its first arrival
                // (F-261): its tracker learns it here.
                if self.judging[i].seen.contains(&c) {
                    self.observe(i, at, Event::MemberSeen(&s.key));
                }
            }
            self.observe(
                i,
                at,
                Event::Arrival {
                    member: &s.key,
                    expired,
                },
            );
        }
        let held = !has_tracker
            || self.judging[i]
                .tracker
                .as_ref()
                .is_some_and(|t| t.state(&s.key).is_some());
        if !held {
            return;
        }
        let Some(m) = self.member_entry(&s.key, i) else {
            return;
        };
        m.sample(judged);
        if !expired {
            if m.arrivals.len() < MAX_ARRIVALS {
                m.arrivals.push(at);
            } else {
                m.arrivals_dropped += 1;
            }
        }
    }

    /// An instant the book judges at: its timer.
    pub fn tick(&mut self, at: Instant) {
        let at = self.mono(at);
        for i in 0..self.resources.len() {
            self.observe(i, at, Event::Tick);
        }
    }

    /// The earliest instant a miss is due among every tracker's members.
    pub fn next_due(&self) -> Option<Instant> {
        self.judging
            .iter()
            .filter_map(|j| j.tracker.as_ref()?.next_due())
            .min()
            .map(|m| self.instant(m))
    }

    /// The window closes at `at`: every tracker judged there, and every
    /// held span closed.
    pub fn finish(&mut self, at: Instant) {
        if self.ended.is_some() {
            return;
        }
        self.tick(at);
        let at = self.last;
        for m in self.members.values_mut() {
            m.hold(None, at);
        }
        self.ended = Some(at);
    }

    /// One member's period over the window (§2.7), by `period_window`;
    /// `None` when its resource declares none.
    pub fn period(&self, key: &str) -> Option<PeriodReading> {
        let m = self.members.get(key)?;
        let period = self.resources[m.resource].period()?;
        let (from, to) = (self.started?, self.ended?);
        Some(period_window(
            period,
            from,
            to,
            m.known,
            m.ended.map(|(at, _)| at),
            &m.arrivals,
        ))
    }

    /// The window §5 reads one member under (§2.7).
    pub fn window_of(&self, key: &str) -> Option<Window> {
        let m = self.members.get(key)?;
        let r = &self.resources[m.resource];
        Some(Window {
            across_face: !self.follows,
            lost: self.lost > 0 || m.arrivals_dropped > 0,
            owner_present: self.owner_present_throughout(&r.address, &r.iface),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_model::timing::{
        Answer, Declared, Reason, deadline_met, lifespan_expired, period_kept,
    };

    const ADDR: &str = "lab/pace";
    const CMD: &str = "zk2/lab/pace/pace.v1/stream/cmd";

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn pace_cmd() -> TimedResource {
        TimedResource::from_parts(
            ADDR.parse().unwrap(),
            "pace.v1".parse().unwrap(),
            "stream/cmd",
            Declaration::Declared(Declared {
                period: Some(ms(100)),
                deadline: Some(ms(500)),
                lifespan: Some(ms(300)),
            }),
            Some(1),
            None,
            CMD,
            true,
        )
    }

    fn alive(i: &str) -> String {
        format!("zk2/lab/pace/@zk/alive/pace.v1/{i}/0123456789abcdef")
    }

    fn present(book: &mut TimingBook, t0: Instant) {
        let read = Observed::from_keys("zk2/lab/pace/@zk/**", &[alive("00000000000000a1")], true);
        book.seed(&read, true, t0);
    }

    fn put(key: &str, at: Instant) -> TimedSample {
        TimedSample {
            key: key.to_owned(),
            delete: false,
            stamp: None,
            arrival: at,
            wall: SystemTime::now(),
        }
    }

    /// §6 step 1 in values: a known member published at its period, its
    /// owner present throughout: met, kept; the lifespan unobservable with
    /// no ε stated (`no_bound`).
    #[test]
    fn a_member_at_its_period_is_met_and_kept() {
        let t0 = Instant::now();
        let mut book = TimingBook::new(vec![pace_cmd()], None, false, t0);
        present(&mut book, t0);
        book.start(t0);
        for n in 1..50 {
            book.sample(&put(CMD, t0 + ms(n * 100)));
        }
        book.finish(t0 + ms(5000));
        let m = book.members().next().unwrap().1.clone();
        let r = &book.resources()[0];
        let w = book.window_of(CMD).unwrap();
        assert!(w.owner_present && !w.lost && !w.across_face, "{w:?}");
        let d = deadline_met(r.kind, &r.declaration, &w, &m.deadline_account());
        assert_eq!((d.answer, d.reason), (Answer::Yes, Reason::Met));
        let p = period_kept(
            r.kind,
            &r.declaration,
            &w,
            book.period(CMD).unwrap().verdict,
        );
        assert_eq!((p.answer, p.reason), (Answer::Yes, Reason::Kept));
        let l = lifespan_expired(r.kind, &r.declaration, &m.samples);
        assert_eq!(
            (l.answer, l.reason),
            (Answer::Unobservable, Reason::NoBound)
        );
    }

    /// §6 step 2 in values: a second's pause: missed with the owner
    /// present, and the period not kept.
    #[test]
    fn a_pause_is_missed_with_the_owner_present() {
        let t0 = Instant::now();
        let mut book = TimingBook::new(vec![pace_cmd()], None, false, t0);
        present(&mut book, t0);
        book.start(t0);
        for n in 1..50 {
            if !(20..30).contains(&n) {
                book.sample(&put(CMD, t0 + ms(n * 100)));
            }
            book.tick(t0 + ms(n * 100 + 50));
        }
        book.finish(t0 + ms(5000));
        let (_, m) = book.members().next().unwrap();
        assert_eq!(m.account.missed_present, 1, "{:?}", m.misses);
        assert_eq!(m.mets, 1);
        let Report::Missed { due, .. } = &m.misses[0] else {
            panic!("{:?}", m.misses)
        };
        assert_eq!(
            book.ms_since_start(*due),
            2400.0,
            "due 500 ms after the last arrival"
        );
        let p = book.period(CMD).unwrap();
        assert_eq!(p.arrivals, 39);
    }

    /// A withdrawal heard leaves the owner unconfirmed and owes a read; a
    /// complete read showing none is absence, and the owner was not present
    /// throughout.
    #[test]
    fn a_withdrawal_owes_a_read_and_absence_waits_for_it() {
        let t0 = Instant::now();
        let mut book = TimingBook::new(vec![pace_cmd()], None, false, t0);
        present(&mut book, t0);
        book.start(t0);
        let addr: Addr = ADDR.parse().unwrap();
        let iface: IfaceId = "pace.v1".parse().unwrap();
        assert!(book.owner_present_throughout(&addr, &iface));
        let owed = book.token(&alive("00000000000000a1"), false, t0 + ms(100));
        assert_eq!(owed, Some(Owed::Owner(addr.clone(), iface.clone())));
        assert_eq!(book.owner(&addr, &iface), OwnerState::Unconfirmed);
        assert_eq!(
            book.owed_read(owed.as_ref().unwrap(), &[], false, t0 + ms(200)),
            owed,
            "a read not complete is owed again"
        );
        assert_eq!(
            book.owed_read(owed.as_ref().unwrap(), &[], true, t0 + ms(300)),
            None
        );
        assert_eq!(book.owner(&addr, &iface), OwnerState::Absent);
        assert!(!book.owner_present_throughout(&addr, &iface));
        assert_eq!(
            owed.unwrap().selector(),
            "zk2/lab/pace/@zk/alive/pace.v1/**"
        );
    }

    /// Instants never go back: an event earlier than the last is taken at
    /// the last's.
    #[test]
    fn an_instant_earlier_than_the_last_is_taken_at_the_last() {
        let t0 = Instant::now();
        let mut book = TimingBook::new(vec![pace_cmd()], None, false, t0);
        book.start(t0 + ms(10));
        book.tick(t0 + ms(50));
        book.sample(&put(CMD, t0 + ms(20)));
        let (_, m) = book.members().next().unwrap();
        assert_eq!(m.arrivals, [Mono::after(ms(50))]);
    }
}
