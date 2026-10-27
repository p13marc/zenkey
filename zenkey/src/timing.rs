//! `timing.v1`'s runtime (`spec/profiles/timing/v1.md`, #908, MB): what a
//! consumer's subscription to a timed resource judges, and how an owner
//! stamps one.
//!
//! Every verdict is `zenkey_model::timing`'s, never re-derived here: the
//! runtime takes each arrival's instant in the subscriber's callback and
//! its wall clock there, follows each provider's presence, reads its
//! descriptors, and drives the model's [`Tracker`] and [`LifespanReader`].
//!
//! | Text | Runtime |
//! |---|---|
//! | §2.2 the declaration | read from the contract the consumer was compiled against; invalid: nothing judged |
//! | §2.4 no runtime re-publishes | nothing here publishes; a [`crate::writer::Writer`] puts only what it is given |
//! | §2.5 per provider and key, the monotonic clock | a [`Tracker`] per provider address, member = the key; each arrival's [`Instant`] taken in the subscriber's callback |
//! | §2.5 the detection bound | a one-shot timer at [`Tracker::next_due`], re-armed at every change, never an idle tick |
//! | §2.5 the owner present, unconfirmed, absent (core §8.1, 0.37) | interface tokens on the watch session handed in ([`crate::consumer::Consumer::with_watch`]); a withdrawal leaving none read again 100 ms later, on the consumer's session, which holds no liveliness subscriber; absent only by a complete read |
//! | §2.5 ended | a member token gone by a complete read (the same re-read), or forgotten |
//! | §2.6 ε | the binding's `clock_bound_ms` ([`crate::config::Binding`]), or [`crate::consumer::Consumer::with_clock_bound`] for a tool; none: nothing discarded |
//! | §2.6 `meta.zid` | the providers' descriptors: a GET at the subscription's start, their puts after it (core §3.3), and a GET again at a stamp of an id not held |
//! | §2.6 the owner stamps | a writer of a resource declaring a lifespan stamps each put from the service's clock ([`crate::state::Minter`]), its id the session's zid, which the descriptor states as `meta.zid`; a configuration stating another `meta.zid` is refused at start |
//!
//! **Without a watch session** the owner is never seen: a known member is
//! judged from its first arrival like any other, and a member ends only by
//! being forgotten. Its arrivals and the timer still judge every deadline.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::authoring::Kind;
use zenkey_model::contract::Resource;
use zenkey_model::freshness::StampAge;
use zenkey_model::grammar::{Addr, IfaceId, ZkKey, data_key, parse};
use zenkey_model::template::Segment;
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

pub use zenkey_model::timing::{
    Declaration, Declared, EndReason, Event, LifespanReader, MemberState, Mono, OwnerState, Reason,
    Report, Sample as LifespanSample, SampleVerdict, Stamped, Tracker, declaration, declaration_of,
    detection_bound, zid_value,
};

use crate::consumer::Provider;

/// How long after a withdrawal it hears the reader reads presence again
/// (`timing.v1` §2.5, core §8.1, 0.37; the scenarios' 100 ms).
pub const REREAD: Duration = Duration::from_millis(100);

/// How long a presence read or a descriptor GET of the reader waits.
pub const READ_TIMEOUT: Duration = Duration::from_secs(1);

/// How often an idle timer wakes to forget (§2.5): a member is forgotten
/// an hour after its last arrival, so a minute late is nothing.
const IDLE: Duration = Duration::from_secs(60);

/// What a subscription to a timed resource reports beside its deliveries
/// (`timing.v1` §2.5, §2.6), each about one member of one provider. The
/// instants are this host's monotonic clock's.
#[derive(Debug, Clone)]
pub enum TimingEvent {
    /// The member became missed (§2.5): more than the deadline passed since
    /// its last arrival, or, for a known member never heard, since the
    /// instant its wait began. `due` is when the miss came to hold, `by` the
    /// latest a reader may report it (due plus the detection bound), and
    /// `owner` the owner's state as this reader observed it then.
    Missed {
        provider: Addr,
        key: String,
        at: Instant,
        due: Instant,
        by: Instant,
        owner: OwnerState,
    },
    /// Its next arrival after a miss.
    Met {
        provider: Addr,
        key: String,
        at: Instant,
    },
    /// The reader stopped judging a member it had heard or reported missed:
    /// its member tokens gone by a complete read, or forgotten.
    Ended {
        provider: Addr,
        key: String,
        at: Instant,
        reason: EndReason,
    },
    /// A member first heard while the resource's bound is held: not
    /// judged, past the owner's bound (core §2.7).
    BeyondBound {
        provider: Addr,
        key: String,
        at: Instant,
    },
    /// A sample discarded as expired (§2.6): not delivered, and no arrival.
    Expired {
        provider: Addr,
        key: String,
        arrival: Instant,
        stamp: Timestamp,
        age: StampAge,
    },
}

impl TimingEvent {
    /// The event's word, as the owner example prints it: `missed`, `met`,
    /// `ended`, `beyond_bound`, `expired`.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Missed { .. } => "missed",
            Self::Met { .. } => "met",
            Self::Ended { .. } => "ended",
            Self::BeyondBound { .. } => "beyond_bound",
            Self::Expired { .. } => "expired",
        }
    }

    /// The member's key.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Missed { key, .. }
            | Self::Met { key, .. }
            | Self::Ended { key, .. }
            | Self::BeyondBound { key, .. }
            | Self::Expired { key, .. } => key,
        }
    }

    /// Its provider.
    #[must_use]
    pub fn provider(&self) -> &Addr {
        match self {
            Self::Missed { provider, .. }
            | Self::Met { provider, .. }
            | Self::Ended { provider, .. }
            | Self::BeyondBound { provider, .. }
            | Self::Expired { provider, .. } => provider,
        }
    }
}

/// A stamp's id as a zid's value (core §3.3: by value).
#[must_use]
pub fn stamp_id(t: &Timestamp) -> Option<u128> {
    zid_value(&t.get_id().to_string())
}

/// What one subscription knows of its resource's timing, fixed at its
/// declaration.
pub(crate) struct Plan {
    pub(crate) kind: Kind,
    pub(crate) declaration: Declaration,
    /// The deadline judged (§2.5): none below the period.
    pub(crate) deadline: Option<Duration>,
    /// Whether each sample's lifespan is judged against owners' stamps: a
    /// lifespan declared, and ε stated.
    pub(crate) lifespan: bool,
    pub(crate) epsilon: Option<Duration>,
    /// The resource's bound (core §2.7): one for a template without
    /// parameters, the contract's `cardinality` else, none for the
    /// no-ceiling value.
    pub(crate) bound: Option<u64>,
    /// The position of the template's `epoch` parameter among the resource
    /// chunks, when it declares one (core §2.3).
    pub(crate) epoch_at: Option<usize>,
    /// The known members' keys (§0), per exact provider.
    pub(crate) known: BTreeMap<Addr, Vec<String>>,
    pub(crate) iface: IfaceId,
}

impl Plan {
    /// The plan of a subscription to `r`, of `iface`, through `providers`
    /// with parameter bindings `params` (R2), under ε `epsilon`.
    pub(crate) fn new(
        r: &Resource,
        iface: &IfaceId,
        providers: &[Provider],
        params: &BTreeMap<String, String>,
        epsilon: Option<Duration>,
    ) -> Self {
        let decl = declaration_of(r);
        let declared = decl.declared().copied();
        let deadline = declared.and_then(|d| d.judged_deadline());
        let lifespan = declared.is_some_and(|d| d.lifespan.is_some()) && epsilon.is_some();
        let segments = r.template.segments();
        let bound = if segments.iter().all(|s| matches!(s, Segment::Literal(_))) {
            Some(1)
        } else {
            r.cardinality.filter(|c| *c != u64::from(u32::MAX))
        };
        let epoch_at = r.epoch.as_ref().and_then(|e| {
            segments
                .iter()
                .position(|s| matches!(s, Segment::Param(n) if n == e))
        });
        // §0: a known member's key is one the reader can write without
        // hearing it: every parameter bound (R2), its provider exact.
        let chunks: Option<Vec<String>> = segments
            .iter()
            .map(|s| match s {
                Segment::Literal(l) => Some(l.clone()),
                Segment::Param(n) => params.get(n).map(|v| zenkey_model::slug::chunk_slug(v)),
                Segment::Rest(_) => None,
            })
            .collect();
        let mut known = BTreeMap::new();
        if let Some(chunks) = chunks {
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            for p in providers {
                let (sys, svc) = p.chunks();
                let Ok(addr) = format!("{sys}/{svc}").parse::<Addr>() else {
                    continue;
                };
                if let Ok(k) = data_key(&addr, iface, r.token, &refs) {
                    known.insert(addr, vec![k.as_str().to_owned()]);
                }
            }
        }
        Self {
            kind: r.kind,
            declaration: decl,
            deadline,
            lifespan,
            epsilon,
            bound,
            epoch_at,
            known,
            iface: iface.clone(),
        }
    }

    /// Whether the subscription runs an engine: a stream whose declaration
    /// judges a deadline or a lifespan.
    pub(crate) fn engaged(&self) -> bool {
        self.kind == Kind::Stream && (self.deadline.is_some() || self.lifespan)
    }

    /// A sample's lifespan judged from the premises alone (§2.6 steps 1
    /// to 4): what every sample of a subscription with no engine reads.
    pub(crate) fn premise(&self) -> LifespanSample {
        LifespanReader::new(self.kind, &self.declaration, self.epsilon, []).judge(None)
    }
}

/// One provider's judging.
struct ProviderTiming {
    tracker: Option<Tracker>,
    lifespan: Option<LifespanReader>,
    /// The `meta.zid`s held of the provider's instances.
    owners: BTreeSet<u128>,
    /// The stamp ids a descriptor GET was asked for, so each is asked once.
    asked: BTreeSet<u128>,
    /// Each held member's epoch chunk, for its member token (core §8.1).
    epochs: BTreeMap<String, String>,
}

/// A subscription's engine: every provider's judging, behind one lock.
pub(crate) struct Engine {
    plan: Plan,
    origin: Instant,
    providers: BTreeMap<Addr, ProviderTiming>,
    /// The owner's state per address, as the presence follower last said,
    /// with the instant it said it.
    owners: BTreeMap<Addr, (OwnerState, Instant)>,
    events: tokio::sync::mpsc::UnboundedSender<TimingEvent>,
    expired: Arc<AtomicU64>,
    /// Woken when a due may have moved earlier.
    rearm: Arc<tokio::sync::Notify>,
    /// Descriptor GETs asked of the driver: a provider, at a stamp of an id
    /// it holds no `meta.zid` for.
    refresh: tokio::sync::mpsc::UnboundedSender<Addr>,
}

impl Engine {
    fn mono(&self, at: Instant) -> Mono {
        Mono::after(at.saturating_duration_since(self.origin))
    }

    fn instant(&self, m: Mono) -> Instant {
        self.origin + m.since_origin()
    }

    fn provider(&mut self, addr: &Addr) -> &mut ProviderTiming {
        if !self.providers.contains_key(addr) {
            let tracker = self.plan.deadline.map(|d| {
                let known = self.plan.known.get(addr).cloned().unwrap_or_default();
                let mut t = Tracker::new(d, self.plan.bound, known);
                t.observe(Mono::ZERO, Event::Start);
                if let Some((state, since)) = self.owners.get(addr) {
                    let at = Mono::after(since.saturating_duration_since(self.origin));
                    t.observe(at, Event::Owner(*state));
                }
                t
            });
            let lifespan = self.plan.lifespan.then(|| {
                LifespanReader::new(
                    self.plan.kind,
                    &self.plan.declaration,
                    self.plan.epsilon,
                    [],
                )
            });
            self.providers.insert(
                addr.clone(),
                ProviderTiming {
                    tracker,
                    lifespan,
                    owners: BTreeSet::new(),
                    asked: BTreeSet::new(),
                    epochs: BTreeMap::new(),
                },
            );
        }
        self.providers.get_mut(addr).expect("inserted above")
    }

    fn send(&self, addr: &Addr, reports: Vec<Report>) {
        for r in reports {
            let ev = match r {
                Report::Missed {
                    at,
                    member,
                    due,
                    by,
                    owner,
                } => TimingEvent::Missed {
                    provider: addr.clone(),
                    key: member,
                    at: self.instant(at),
                    due: self.instant(due),
                    by: self.instant(by),
                    owner,
                },
                Report::Met { at, member } => TimingEvent::Met {
                    provider: addr.clone(),
                    key: member,
                    at: self.instant(at),
                },
                Report::Ended { at, member, reason } => TimingEvent::Ended {
                    provider: addr.clone(),
                    key: member,
                    at: self.instant(at),
                    reason,
                },
                Report::BeyondBound { at, member } => TimingEvent::BeyondBound {
                    provider: addr.clone(),
                    key: member,
                    at: self.instant(at),
                },
            };
            let _ = self.events.send(ev);
        }
    }

    /// One sample's arrival (§2.5, §2.6), at `arrival` on this host's
    /// monotonic clock and `wall` on its wall clock, both read in the
    /// subscriber's callback. Returns its lifespan, or `None` when it is
    /// discarded as expired.
    pub(crate) fn arrive(
        &mut self,
        addr: &Addr,
        key: &str,
        chunks: &[String],
        sample: &Sample,
        arrival: Instant,
        wall: SystemTime,
    ) -> Option<LifespanSample> {
        let mono = self.mono(arrival);
        let epoch_at = self.plan.epoch_at;
        let plan_premise = self.plan.premise();
        let stamp = sample.timestamp().copied();
        let delete = sample.kind() == SampleKind::Delete;
        let mut refresh = false;
        let p = self.provider(addr);
        // §2.6: the lifespan first; an expired sample is no arrival.
        let judged = match (&mut p.lifespan, delete) {
            (Some(reader), false) => {
                let stamped = stamp.and_then(|t| {
                    let id = stamp_id(&t)?;
                    if !p.owners.contains(&id) && p.asked.insert(id) {
                        refresh = true;
                    }
                    Some(Stamped {
                        age: StampAge::between(t.get_time().to_system_time(), wall),
                        id,
                    })
                });
                reader.judge(stamped)
            }
            _ => plan_premise,
        };
        let expired = judged.verdict == SampleVerdict::Expired;
        // §2.5: a delete sample is no arrival.
        let reports = match (&mut p.tracker, delete) {
            (Some(t), false) => {
                if let Some(at) = epoch_at
                    && let Some(c) = chunks.get(at)
                {
                    p.epochs.insert(key.to_owned(), c.clone());
                }
                t.observe(
                    mono,
                    Event::Arrival {
                        member: key,
                        expired,
                    },
                )
            }
            _ => Vec::new(),
        };
        self.send(addr, reports);
        self.rearm.notify_one();
        if refresh {
            let _ = self.refresh.send(addr.clone());
        }
        if expired {
            self.expired.fetch_add(1, Ordering::Relaxed);
            if let Some(stamp) = stamp {
                let _ = self.events.send(TimingEvent::Expired {
                    provider: addr.clone(),
                    key: key.to_owned(),
                    arrival,
                    stamp,
                    age: StampAge::between(stamp.get_time().to_system_time(), wall),
                });
            }
            return None;
        }
        Some(judged)
    }

    /// Judges every provider's members at `now` (the timer).
    fn tick(&mut self, now: Instant) {
        let at = self.mono(now);
        let addrs: Vec<Addr> = self.providers.keys().cloned().collect();
        for a in addrs {
            let reports = match self.providers.get_mut(&a).and_then(|p| p.tracker.as_mut()) {
                Some(t) => t.observe(at, Event::Tick),
                None => continue,
            };
            self.send(&a, reports);
        }
    }

    /// The earliest due among every provider's members.
    fn next_due(&self) -> Option<Instant> {
        self.providers
            .values()
            .filter_map(|p| p.tracker.as_ref()?.next_due())
            .min()
            .map(|m| self.instant(m))
    }

    /// The owner of `addr` as the presence follower now observes it.
    fn owner(&mut self, addr: &Addr, state: OwnerState, at: Instant) {
        let changed = self.owners.get(addr).is_none_or(|(s, _)| *s != state);
        if !changed {
            return;
        }
        self.owners.insert(addr.clone(), (state, at));
        // A provider with a known member is judged from its owner's
        // presence before its first arrival (§2.5).
        if self.plan.known.contains_key(addr) || self.providers.contains_key(addr) {
            let mono = self.mono(at);
            let reports = match self.provider(addr).tracker.as_mut() {
                Some(t) => t.observe(mono, Event::Owner(state)),
                None => Vec::new(),
            };
            self.send(addr, reports);
            self.rearm.notify_one();
        }
    }

    /// Every member of `addr` whose epoch chunk is `member` ends (§2.5):
    /// a complete read shows no member token of it.
    fn member_gone(&mut self, addr: &Addr, member: &str, at: Instant) {
        let mono = self.mono(at);
        let Some(p) = self.providers.get_mut(addr) else {
            return;
        };
        let keys: Vec<String> = p
            .epochs
            .iter()
            .filter(|(_, c)| c.as_str() == member)
            .map(|(k, _)| k.clone())
            .collect();
        let mut reports = Vec::new();
        if let Some(t) = p.tracker.as_mut() {
            for k in &keys {
                reports.extend(t.observe(mono, Event::MemberGone(k)));
            }
        }
        for k in &keys {
            p.epochs.remove(k);
        }
        self.send(addr, reports);
    }

    /// The `meta.zid`s of `addr`'s instances, as its descriptors state them
    /// (§2.6, core §3.3).
    fn set_owners(&mut self, addr: &Addr, zids: BTreeSet<u128>) {
        let p = self.provider(addr);
        if zids.is_subset(&p.owners) {
            return;
        }
        p.owners.extend(zids);
        let owners = p.owners.clone();
        if let Some(r) = p.lifespan.as_mut() {
            r.set_owners(owners);
        }
    }

    /// The owner's state of `addr`, as the reader last observed it.
    pub(crate) fn owner_state(&self, addr: &Addr) -> OwnerState {
        self.owners
            .get(addr)
            .map_or(OwnerState::Unseen, |(s, _)| *s)
    }

    /// Where a held member stands, for `addr`'s tracker.
    pub(crate) fn member_state(&self, addr: &Addr, key: &str) -> Option<MemberState> {
        self.providers.get(addr)?.tracker.as_ref()?.state(key)
    }
}

/// What the subscription keeps of its engine.
pub(crate) struct Running {
    pub(crate) engine: Arc<Mutex<Engine>>,
    pub(crate) expired: Arc<AtomicU64>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _watch: Vec<zenoh::pubsub::Subscriber<()>>,
    _descriptors: Vec<zenoh::pubsub::Subscriber<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// A presence move heard on the watch session.
struct Heard {
    key: String,
    put: bool,
    at: Instant,
}

/// The engine's sessions and selectors.
pub(crate) struct Sources {
    /// The consumer's session: where it reads presence and descriptors. It
    /// holds no liveliness subscriber (core §8.1, 0.37).
    pub(crate) session: zenoh::Session,
    /// The watch session handed in, where the liveliness subscribers go.
    pub(crate) watch: Option<zenoh::Session>,
    pub(crate) providers: Vec<Provider>,
}

/// A subscription's engine, made before its data subscribers are
/// declared, so the subscription's start is its origin (§2.5); its sources
/// are started after them ([`Prepared::run`]).
pub(crate) struct Prepared {
    pub(crate) engine: Arc<Mutex<Engine>>,
    expired: Arc<AtomicU64>,
    rearm: Arc<tokio::sync::Notify>,
    rx: tokio::sync::mpsc::UnboundedReceiver<TimingEvent>,
    refresh_rx: tokio::sync::mpsc::UnboundedReceiver<Addr>,
}

/// The engine of a subscription declared at `origin`, or `None` for a
/// subscription with nothing to judge ([`Plan::engaged`]).
pub(crate) fn prepare(plan: Plan, origin: Instant) -> Option<Prepared> {
    if !plan.engaged() {
        return None;
    }
    let (events, rx) = tokio::sync::mpsc::unbounded_channel::<TimingEvent>();
    let (refresh, refresh_rx) = tokio::sync::mpsc::unbounded_channel::<Addr>();
    let expired = Arc::new(AtomicU64::new(0));
    let rearm = Arc::new(tokio::sync::Notify::new());
    let engine = Arc::new(Mutex::new(Engine {
        plan,
        origin,
        providers: BTreeMap::new(),
        owners: BTreeMap::new(),
        events,
        expired: Arc::clone(&expired),
        rearm: Arc::clone(&rearm),
        refresh,
    }));
    Some(Prepared {
        engine,
        expired,
        rearm,
        rx,
        refresh_rx,
    })
}

impl Prepared {
    /// Starts its sources, once the subscription's data subscribers are
    /// declared: the dispatcher that hands `on_event` every event in
    /// order; the descriptors read and followed (with a lifespan judged);
    /// the providers' presence followed (with a deadline and a watch
    /// session), seeded by a read after the watch session's round trip
    /// (core §8.1, 0.37); and the timer.
    pub(crate) async fn run<T>(self, sources: Sources, on_event: T) -> Running
    where
        T: Fn(TimingEvent) + Send + Sync + 'static,
    {
        let Prepared {
            engine,
            expired,
            rearm,
            mut rx,
            mut refresh_rx,
        } = self;
        let (lifespan, deadline, epoch, iface) = {
            let e = engine.lock().expect("not poisoned");
            (
                e.plan.lifespan,
                e.plan.deadline.is_some(),
                e.plan.epoch_at.is_some(),
                e.plan.iface.clone(),
            )
        };
        let mut tasks = Vec::new();
        // The dispatcher: every event, in the order the engine made it.
        tasks.push(tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                on_event(ev);
            }
        }));
        // §2.6: the providers' `meta.zid`s, from their descriptors: their
        // puts (core §3.3: an instance's first is put before any token, so
        // before its first sample), and a GET of those already up.
        let mut descriptors = Vec::new();
        if lifespan {
            for p in &sources.providers {
                let (sys, svc) = p.chunks();
                let sel = format!("zk2/{sys}/{svc}/@zk/instance/*");
                let e = Arc::clone(&engine);
                let sub = sources
                    .session
                    .declare_subscriber(sel.clone())
                    .callback(move |s: Sample| {
                        if let Some((addr, zid)) = descriptor_zid(&s) {
                            e.lock()
                                .expect("not poisoned")
                                .set_owners(&addr, [zid].into());
                        }
                    })
                    .await;
                match sub {
                    Ok(s) => descriptors.push(s),
                    Err(e) => tracing::warn!(
                        "timing.v1: the descriptors' subscription failed, so a stamp is attributed by GET alone: {e}"
                    ),
                }
                for (addr, zids) in read_descriptors(&sources.session, &sel).await {
                    engine.lock().expect("not poisoned").set_owners(&addr, zids);
                }
            }
            let (e, session) = (Arc::clone(&engine), sources.session.clone());
            tasks.push(tokio::spawn(async move {
                while let Some(addr) = refresh_rx.recv().await {
                    let sel = format!("zk2/{}/{}/@zk/instance/*", addr.system, addr.service);
                    for (a, zids) in read_descriptors(&session, &sel).await {
                        e.lock().expect("not poisoned").set_owners(&a, zids);
                    }
                }
            }));
        }
        // §2.5: the owner's presence and its member tokens, on the watch
        // session handed in.
        let mut watch_subs = Vec::new();
        if deadline && let Some(w) = &sources.watch {
            let (heard_tx, heard_rx) = tokio::sync::mpsc::unbounded_channel::<Heard>();
            let mut scopes = Vec::new();
            for p in &sources.providers {
                let (sys, svc) = p.chunks();
                scopes.push(format!("zk2/{sys}/{svc}/@zk/alive/{iface}/**"));
                if epoch {
                    scopes.push(format!("zk2/{sys}/{svc}/@zk/member/{iface}/**"));
                }
            }
            for sel in &scopes {
                let tx = heard_tx.clone();
                // Without history outside a namespace (core §8.1, 0.41): the
                // seed below reads what is held.
                let sub = crate::presence::declare_subscriber(w, sel.clone())
                    .history(crate::presence::subscriber_needs_history(w))
                    .callback(move |s: Sample| {
                        let _ = tx.send(Heard {
                            key: s.key_expr().as_str().to_owned(),
                            put: s.kind() == SampleKind::Put,
                            at: Instant::now(),
                        });
                    })
                    .await;
                match sub {
                    Ok(s) => watch_subs.push(s),
                    Err(e) => tracing::warn!(
                        "timing.v1: the watch on {sel} failed, so its owner is never seen: {e}"
                    ),
                }
            }
            let barrier = match scopes.first() {
                Some(scope) => match crate::presence::Barrier::new(w, scope, READ_TIMEOUT).await {
                    Ok(b) => Some(b),
                    Err(e) => {
                        tracing::warn!(
                            "timing.v1: the watch session's barrier could not be kept: {e}"
                        );
                        None
                    }
                },
                None => None,
            };
            let mut follower = Follower {
                engine: Arc::clone(&engine),
                session: sources.session.clone(),
                iface: iface.clone(),
                providers: sources.providers.clone(),
                barrier,
                alive: BTreeMap::new(),
                members: BTreeMap::new(),
            };
            // Seeded before the subscription is handed back: the owner's
            // presence as a read after the round trip shows it.
            follower.seed().await;
            tasks.push(tokio::spawn(follower.run(heard_rx)));
        }
        // §2.5: the timer, a one-shot at the earliest due, re-armed whenever
        // a due may have moved earlier.
        if deadline {
            let e = Arc::clone(&engine);
            tasks.push(tokio::spawn(async move {
                loop {
                    let due = e.lock().expect("not poisoned").next_due();
                    let wake = match due {
                        // A miss holds at any instant after its due.
                        Some(d) => d + Duration::from_micros(1),
                        None => Instant::now() + IDLE,
                    };
                    tokio::select! {
                        () = tokio::time::sleep_until(wake.into()) => {
                            e.lock().expect("not poisoned").tick(Instant::now());
                        }
                        () = rearm.notified() => {}
                    }
                }
            }));
        }
        Running {
            engine,
            expired,
            tasks,
            _watch: watch_subs,
            _descriptors: descriptors,
        }
    }
}

/// A descriptor put's address and `meta.zid`.
fn descriptor_zid(s: &Sample) -> Option<(Addr, u128)> {
    if s.kind() != SampleKind::Put {
        return None;
    }
    let Ok(ZkKey::Instance { addr, .. }) = parse(s.key_expr().as_str()) else {
        return None;
    };
    let v: serde_json::Value = serde_json::from_slice(&s.payload().to_bytes()).ok()?;
    let zid = zid_value(v.get("meta")?.get("zid")?.as_str()?)?;
    Some((addr, zid))
}

/// GETs the descriptors under `selector`: each instance's address and its
/// `meta.zid`. An instance whose descriptor states none attributes nothing
/// (core §3.3).
async fn read_descriptors(session: &zenoh::Session, selector: &str) -> Vec<(Addr, BTreeSet<u128>)> {
    let rx = match session
        .get(selector)
        .consolidation(zenoh::query::ConsolidationMode::None)
        .timeout(READ_TIMEOUT)
        .with(flume::unbounded::<zenoh::query::Reply>())
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            tracing::warn!("timing.v1: the descriptors' GET was refused: {e}");
            return Vec::new();
        }
    };
    let mut out: BTreeMap<Addr, BTreeSet<u128>> = BTreeMap::new();
    while let Ok(reply) = rx.recv_async().await {
        let Ok(s) = reply.into_result() else { continue };
        if let Some((addr, zid)) = descriptor_zid(&s) {
            out.entry(addr).or_default().insert(zid);
        }
    }
    out.into_iter().collect()
}

/// The presence follower (§2.5, core §8.1, 0.37): the owner present while
/// an interface token of its address is held, unconfirmed when a
/// withdrawal leaves none, absent when the read 100 ms later is complete
/// and shows none; a member token's withdrawal read again the same way.
struct Follower {
    engine: Arc<Mutex<Engine>>,
    session: zenoh::Session,
    iface: IfaceId,
    providers: Vec<Provider>,
    barrier: Option<crate::presence::Barrier>,
    /// Each address's interface tokens held, as heard and read.
    alive: BTreeMap<Addr, BTreeSet<String>>,
    /// Each (address, member chunk)'s member tokens held.
    members: BTreeMap<(Addr, String), BTreeSet<String>>,
}

/// A read owed after a withdrawal.
enum Owed {
    Owner(Addr),
    Member(Addr, String),
}

impl Follower {
    async fn run(mut self, mut heard: tokio::sync::mpsc::UnboundedReceiver<Heard>) {
        let (owe_tx, mut owe_rx) = tokio::sync::mpsc::unbounded_channel::<Owed>();
        let mut moves_seen = self
            .barrier
            .as_ref()
            .map_or(0, crate::presence::Barrier::moves);
        let barrier = self.barrier.clone();
        loop {
            let moved = async {
                match &barrier {
                    Some(b) => b.moved_since(moves_seen).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                h = heard.recv() => match h {
                    Some(h) => self.heard(h, &owe_tx),
                    None => break,
                },
                o = owe_rx.recv() => if let Some(o) = o {
                    self.read_owed(o, &owe_tx).await;
                },
                // The watch session's transport moved (core §8.1, 0.41): what
                // it told may be behind, so presence is read again.
                () = moved => {
                    moves_seen = barrier.as_ref().map_or(moves_seen, crate::presence::Barrier::moves);
                    self.seed().await;
                }
            }
        }
    }

    fn engine(&self) -> std::sync::MutexGuard<'_, Engine> {
        self.engine.lock().expect("not poisoned")
    }

    /// Reads every provider's interface tokens: the owner present where it
    /// holds one, absent where a complete read shows none.
    async fn seed(&mut self) {
        let settled = match &self.barrier {
            Some(b) => b.establish().await,
            None => true,
        };
        for p in self.providers.clone() {
            let (sys, svc) = p.chunks();
            let sel = format!("zk2/{sys}/{svc}/@zk/alive/{}/**", self.iface);
            let read =
                match crate::presence::liveliness_read(&self.session, &sel, READ_TIMEOUT).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            "timing.v1: presence was not read ({e}): the owner's state stands"
                        );
                        continue;
                    }
                };
            let now = Instant::now();
            let mut found: BTreeMap<Addr, BTreeSet<String>> = BTreeMap::new();
            for k in &read.keys {
                if let Ok(ZkKey::Alive { addr, .. }) = parse(k) {
                    found.entry(addr).or_default().insert(k.clone());
                }
            }
            // An address held before and shown by none in a complete read is
            // absent; one the read shows is present.
            let before: Vec<Addr> = self
                .alive
                .keys()
                .filter(|a| p.matches(a))
                .cloned()
                .collect();
            for a in before {
                if !found.contains_key(&a) && read.complete && settled {
                    self.alive.remove(&a);
                    self.engine().owner(&a, OwnerState::Absent, now);
                }
            }
            if read.complete && settled {
                let known: Vec<Addr> = self.engine().plan.known.keys().cloned().collect();
                for a in known {
                    if p.matches(&a) && !found.contains_key(&a) {
                        self.engine().owner(&a, OwnerState::Absent, now);
                    }
                }
            }
            for (a, keys) in found {
                self.alive.insert(a.clone(), keys);
                self.engine().owner(&a, OwnerState::Present, now);
            }
        }
    }

    fn heard(&mut self, h: Heard, owe: &tokio::sync::mpsc::UnboundedSender<Owed>) {
        match parse(&h.key) {
            Ok(ZkKey::Alive { addr, iface, .. }) if iface == self.iface => {
                let held = self.alive.entry(addr.clone()).or_default();
                if h.put {
                    held.insert(h.key);
                    self.engine().owner(&addr, OwnerState::Present, h.at);
                } else {
                    held.remove(&h.key);
                    if held.is_empty() {
                        self.engine().owner(&addr, OwnerState::Unconfirmed, h.at);
                        owe_later(owe, Owed::Owner(addr));
                    }
                }
            }
            Ok(ZkKey::Member {
                addr,
                iface,
                member,
                ..
            }) if iface == self.iface => {
                let held = self
                    .members
                    .entry((addr.clone(), member.clone()))
                    .or_default();
                if h.put {
                    held.insert(h.key);
                } else {
                    held.remove(&h.key);
                    if held.is_empty() {
                        owe_later(owe, Owed::Member(addr, member));
                    }
                }
            }
            _ => {}
        }
    }

    /// The read a withdrawal owed (core §8.1, 0.37): complete and empty is
    /// absence; a token shown is presence; anything else owes it again.
    async fn read_owed(&mut self, o: Owed, owe: &tokio::sync::mpsc::UnboundedSender<Owed>) {
        let settled = match &self.barrier {
            Some(b) => b.establish().await,
            None => true,
        };
        let sel = match &o {
            Owed::Owner(a) => format!("zk2/{}/{}/@zk/alive/{}/**", a.system, a.service, self.iface),
            Owed::Member(a, m) => {
                format!(
                    "zk2/{}/{}/@zk/member/{}/{m}/*",
                    a.system, a.service, self.iface
                )
            }
        };
        // A withdrawal's re-read may find the token back already: then the
        // held set says so, and nothing is owed.
        let still_none = match &o {
            Owed::Owner(a) => self.alive.get(a).is_none_or(BTreeSet::is_empty),
            Owed::Member(a, m) => self
                .members
                .get(&(a.clone(), m.clone()))
                .is_none_or(BTreeSet::is_empty),
        };
        if !still_none {
            return;
        }
        let read = crate::presence::liveliness_read(&self.session, &sel, READ_TIMEOUT).await;
        let now = Instant::now();
        match (read, o) {
            (Ok(r), Owed::Owner(a)) if !r.keys.is_empty() => {
                self.alive.insert(a.clone(), r.keys.into_iter().collect());
                self.engine().owner(&a, OwnerState::Present, now);
            }
            (Ok(r), Owed::Owner(a)) if r.complete && settled => {
                self.engine().owner(&a, OwnerState::Absent, now);
            }
            (Ok(r), Owed::Member(a, m)) if !r.keys.is_empty() => {
                self.members.insert((a, m), r.keys.into_iter().collect());
            }
            (Ok(r), Owed::Member(a, m)) if r.complete && settled => {
                self.engine().member_gone(&a, &m, now);
            }
            (_, o) => owe_later(owe, o),
        }
    }
}

/// Owes `o` a read [`REREAD`] from now.
fn owe_later(owe: &tokio::sync::mpsc::UnboundedSender<Owed>, o: Owed) {
    let owe = owe.clone();
    tokio::spawn(async move {
        tokio::time::sleep(REREAD).await;
        let _ = owe.send(o);
    });
}
