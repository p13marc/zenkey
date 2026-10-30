//! `timing.v1` (#908, MC), the reads: the timed streams a scope reaches
//! (`spec/profiles/timing/v1.md`), over one window, as §2.7's tool reads
//! them, through a session in the deployment's namespace.
//!
//! **Why the fleet runs its own engine.** The runtime's
//! (`zenkey::timing`, a [`zenkey::consumer::Consumer::with_watch`]
//! subscription) is a consumer's: one role's bindings, a watch session in
//! the deployment's namespace, deadline events and nothing else. A tool's
//! window needs what a consumer never keeps — each member's span held, each
//! miss with its owner's state, its arrivals for the period, each sample's
//! lifespan, the owner present throughout — and holds its liveliness
//! subscribers on a [`crate::WatchSession`] in no namespace on the wire
//! keys (#875), which `with_watch` does not take. So the window is
//! [`crate::model::timing::TimingBook`], over the same model the runtime
//! drives (`zenkey_model::timing`'s `Tracker` and `LifespanReader`), fed
//! here by [`crate::bus::watch::TokenHeard`]s and the watch session's
//! barrier as [`crate::bus::telemetry`]'s reader is; every decision stays
//! the model's.
//!
//! What is read, in order:
//! 1. presence and every instance's descriptor (core §8.1, §3.3), and every
//!    revision they name, retrieved (§8.4): the timed resources in scope are
//!    those revisions' streams with a timing key. Across a constrained face,
//!    where neither crosses, the deployment's word stands for them: one
//!    owner, the contracts in hand, and its `meta.zid`s as the operator
//!    states them (§2.8);
//! 2. a liveliness subscription to the owners' instance, interface and
//!    member tokens on the watch session, without history, its barrier
//!    kept (#851); then a subscription to each timed resource's members on
//!    the deployment's session, each sample's arrival read **in the
//!    subscriber's callback** on this host's monotonic clock with its wall
//!    clock beside it (§2.5, §2.6), then a watch of this reader's own
//!    receive path ([`crate::bus::stall`], core §2.7). Every subscription
//!    calls back into **one** bounded queue, so samples and token moves are
//!    folded in the one order received, and what the queue refuses is
//!    counted: a loss;
//! 3. **the window's start** (§2.7, text 0.2, F-263): the barrier made, a
//!    presence read with the descriptor of every instance it shows — the
//!    first instant both stand, made again once when one did not. Those
//!    descriptor GETs are made after the subscriptions, so an owner that
//!    answered one holds them (#819 TG). The window's start is every
//!    tracker's subscription start (`Event::Start`), and the caller's
//!    `listening` runs with what it stood on ([`TimingOpening`]);
//! 4. the window: every delivery folded as it comes; a withdrawal that
//!    leaves an owner's or a member's tokens empty owes a read
//!    [`REREAD`] later on the deployment's session (core §8.1, 0.37); an
//!    instance heard declared, or a stamp of an id no descriptor held
//!    names, asks its descriptors; a loss, a stall or a move of the watch
//!    session's transport reads presence again; and a one-shot timer at
//!    the trackers' earliest due judges at it, so a miss is reported within
//!    the detection bound (§2.5). The queue is drained before a tick or a
//!    read's answer is folded, so nothing it holds is folded after an
//!    instant later than its own.
//!
//! Nothing is judged here: the book's model decides, and
//! [`crate::judge::timing`] answers §5.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::grammar::{Addr, InstanceId, ZkKey};
use zenoh::Session;
use zenoh::sample::{Sample, SampleKind};

use crate::bus::contracts::BundleStore;
use crate::bus::stall::{StallSeen, StallWatch};
use crate::model::catalog::{Catalog, ContractSet, ContractState, DescriptorRead, Observed};
use crate::model::timing::{Owed, TimedResource, TimedSample, TimingBook, TimingScope};
use crate::report::{ScopeReach, TimingOpening, UnreachedResource};

/// The queue the subscriptions call back into.
pub const QUEUE: usize = 1024;

/// How long after a withdrawal it hears the reader reads presence again
/// (core §8.1, 0.37; the scenarios' 100 ms).
pub const REREAD: Duration = Duration::from_millis(100);

/// How long the window's opening waits before its second presence read
/// when the first did not stand (§2.7, F-263).
pub const RETRY: Duration = Duration::from_secs(1);

/// How many times a descriptor GET is asked for an address at a stamp of
/// an id it holds no `meta.zid` for, and how far apart.
const ASK_LIMIT: u32 = 5;
const ASK_EVERY: Duration = Duration::from_secs(1);

/// What a reading asks.
#[derive(Debug, Clone)]
pub struct TimingSpec {
    /// Each read's timeout: presence, a descriptor, a retrieval.
    pub timeout: Duration,
    /// How long the window runs, from its start.
    pub window: Duration,
    /// The deployment's link lease (core §2.7): with `wait_before_close`,
    /// the longest this reader may stall before it counts a loss.
    pub lease: Duration,
    /// The routers' `wait_before_close` (core §2.7, 0.27).
    pub wait_before_close: Duration,
    /// ε, the deployment's stated clock bound (§2.6); `None`: none stated,
    /// and every lifespan unobservable (`no_bound`), never the HLC's delta.
    pub clock_bound: Option<Duration>,
    /// Across a constrained face (§2.8), on the deployment's word: no
    /// presence is read, the timed resources are the contracts' in hand of
    /// the one owner the scope names, and the deadline and the period are
    /// unobservable.
    pub across_face: bool,
    /// Across a face, the owner's `meta.zid`s the deployment states: no
    /// descriptor crosses (§2.8, text 0.3).
    pub owner_zids: Vec<u128>,
    /// Across a face, how long after the subscriptions the window starts:
    /// a margin for them to cross, never proof.
    pub settle: Duration,
}

impl TimingSpec {
    /// A window, zenoh's lease and `wait_before_close`, no bound, no face.
    pub fn new(timeout: Duration, window: Duration) -> TimingSpec {
        TimingSpec {
            timeout,
            window,
            lease: zenkey_model::budget::DEFAULT_LEASE,
            wait_before_close: zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE,
            clock_bound: None,
            across_face: false,
            owner_zids: Vec::new(),
            settle: crate::bus::telemetry::DEFAULT_SETTLE,
        }
    }

    /// The longest this reader may stall (core §2.7, 0.27).
    pub fn stall_bound(&self) -> Duration {
        zenkey_model::budget::stall_bound(
            self.wait_before_close,
            self.lease,
            zenkey_model::budget::DEFAULT_KEEP_ALIVE,
        )
    }
}

/// One window over a scope's timed members, as values: what
/// [`crate::judge::timing::judge`] decides from.
#[derive(Debug, Clone)]
pub struct TimingObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub spec: TimingSpec,
    pub scope: TimingScope,
    pub book: TimingBook,
    /// The selectors put to the bus, base-relative.
    pub asked: Vec<String>,
    /// What the window opened on; `None` when it never opened.
    pub opening: Option<TimingOpening>,
    /// Samples on a wildcard key (R6), discarded in the callback.
    pub discarded: u64,
    /// What in scope could not be read, one line each.
    pub skipped: Vec<String>,
    /// What the selector cannot reach (`spec/observer.md` §8.2; #950):
    /// `None` with no selector.
    pub reach: Option<ScopeReach>,
    /// Set when the reading could not be made at all, with why.
    pub unobservable: Option<String>,
}

/// The timed resources in scope: every revision the descriptors in scope
/// name and the store holds, or, across a face, every contract in hand of
/// the one owner the scope names, those the selector reaches. A revision
/// not in hand is skipped, with why.
pub fn timed_resources_in_scope(
    scope: &TimingScope,
    catalog: Option<&Catalog>,
    store: &BundleStore,
    offline: &ContractSet,
    skipped: &mut Vec<String>,
) -> Vec<TimedResource> {
    timed_resources_of_owners(scope, catalog, store, offline, skipped)
        .into_iter()
        .filter(|r| {
            scope
                .subscription_to(&r.address, &r.iface, &r.selector)
                .is_some()
        })
        .collect()
}

/// [`timed_resources_in_scope`] before the selector: every timed resource
/// of the owners and interfaces in scope, so that what the selector misses
/// can be named (`spec/observer.md` §8.2; #950).
pub fn timed_resources_of_owners(
    scope: &TimingScope,
    catalog: Option<&Catalog>,
    store: &BundleStore,
    offline: &ContractSet,
    skipped: &mut Vec<String>,
) -> Vec<TimedResource> {
    let mut out = Vec::new();
    let mut push = |r: TimedResource| {
        if scope
            .subscription_to(&r.address, &r.iface, &r.selector)
            .is_some()
            || scope.misses(&r.address, &r.iface, &r.selector)
        {
            out.push(r);
        }
    };
    match catalog {
        Some(c) => {
            for addr in c.addresses().filter(|a| scope.selects_owner(a)) {
                for iface in c.provided(addr) {
                    for fp in c.revisions_of(addr, &iface) {
                        match crate::model::catalog::Contracts::state(store, &iface, &fp) {
                            Some(ContractState::Held(rev)) => {
                                for r in crate::model::timing::timed_resources(addr, &rev) {
                                    push(r);
                                }
                            }
                            Some(ContractState::Unavailable { .. }) => skipped
                                .push(format!("{addr} {iface} {fp}: no holder served its bundle")),
                            Some(ContractState::Unreadable { reason }) => {
                                skipped.push(format!("{addr} {iface} {fp}: {reason}"));
                            }
                            None => skipped.push(format!("{addr} {iface} {fp}: not retrieved")),
                        }
                    }
                }
            }
        }
        None => {
            if let Some(addr) = scope.one_owner() {
                for rev in offline.iter() {
                    for r in crate::model::timing::timed_resources(&addr, rev) {
                        push(r);
                    }
                }
            }
        }
    }
    out
}

/// A timed resource the selector misses, as a report names it.
fn unreached(r: &TimedResource) -> UnreachedResource {
    UnreachedResource {
        address: r.address.to_string(),
        iface: r.iface.to_string(),
        resource: r.resource.clone(),
        selector: r.selector.clone(),
    }
}

/// One reading: [`observe`], then [`crate::judge::timing::judge`].
#[allow(clippy::too_many_arguments)]
pub async fn run_timing(
    session: &Session,
    watch: Option<&crate::WatchSession>,
    namespace: &str,
    scope: TimingScope,
    store: &BundleStore,
    offline: &ContractSet,
    spec: TimingSpec,
    listening: impl FnOnce(&TimingOpening) + Send,
) -> crate::report::TimingReport {
    let obs = observe(
        session, watch, namespace, scope, store, offline, spec, listening,
    )
    .await;
    crate::judge::timing::judge(&obs)
}

/// What a subscription called back with.
enum Delivery {
    Sample(TimedSample),
    Token { key: String, held: bool },
}

/// The bounded queue's sending half: a refusal is counted, never waited on.
#[derive(Clone)]
struct Queue {
    tx: tokio::sync::mpsc::Sender<Delivery>,
    refused: Arc<AtomicU64>,
}

impl Queue {
    fn send(&self, d: Delivery) {
        if let Err(tokio::sync::mpsc::error::TrySendError::Full(_)) = self.tx.try_send(d) {
            self.refused.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// What a read made beside the window answered.
enum Answered {
    /// The read a withdrawal owed.
    Owed {
        owed: Owed,
        keys: Vec<String>,
        complete: bool,
        at: Instant,
    },
    /// Descriptors, of the instances named.
    Described(Observed),
    /// Presence read again after a loss, with its descriptors.
    Reseed {
        read: Result<Observed, String>,
        settled: bool,
        at: Instant,
    },
}

/// The window's reader of the module doc, over `spec.window`. `listening`
/// runs once, when the window starts (step 3), and never when the reading
/// ends before it.
#[allow(clippy::too_many_arguments)]
pub async fn observe(
    session: &Session,
    watch: Option<&crate::WatchSession>,
    namespace: &str,
    scope: TimingScope,
    store: &BundleStore,
    offline: &ContractSet,
    spec: TimingSpec,
    listening: impl FnOnce(&TimingOpening) + Send,
) -> TimingObservation {
    let t = spec.timeout;
    let presence_scope = scope.presence_scope();
    let t0 = Instant::now();
    let mut obs = TimingObservation {
        namespace: namespace.to_owned(),
        spec: spec.clone(),
        scope: scope.clone(),
        book: TimingBook::new(Vec::new(), spec.clock_bound, spec.across_face, t0),
        asked: Vec::new(),
        opening: None,
        discarded: 0,
        skipped: Vec::new(),
        reach: None,
        unobservable: None,
    };
    if spec.across_face && scope.one_owner().is_none() {
        obs.unobservable = Some(
            "across a face the deployment's word stands for one owner: the selector names its \
             system and service (zk2/<system>/<service>/…)"
                .to_owned(),
        );
        return obs;
    }
    // 1. What is in scope.
    let catalog = if spec.across_face {
        None
    } else {
        obs.asked.push(presence_scope.selector());
        match crate::bus::presence::observe(session, &presence_scope, t).await {
            Ok(o) => {
                let c = Catalog::new(&o);
                let _ = store.fetch_all(session, &c.wanted()).await;
                Some(c)
            }
            Err(e) => {
                obs.unobservable = Some(format!(
                    "the presence read could not be made: {}",
                    crate::one_line(&e)
                ));
                return obs;
            }
        }
    };
    let mut skipped = Vec::new();
    let (resources, missed): (Vec<TimedResource>, Vec<TimedResource>) =
        timed_resources_of_owners(&scope, catalog.as_ref(), store, offline, &mut skipped)
            .into_iter()
            .partition(|r| {
                scope
                    .subscription_to(&r.address, &r.iface, &r.selector)
                    .is_some()
            });
    obs.skipped = skipped;
    // What the selector misses is said even when it reaches nothing
    // (`spec/observer.md` §8.2; zk2py round 82's F-287, #950).
    obs.reach = scope.reach(missed.iter().map(unreached).collect());
    if resources.is_empty() {
        return obs;
    }
    let subscriptions: BTreeSet<String> = resources
        .iter()
        .filter_map(|r| scope.subscription_to(&r.address, &r.iface, &r.selector))
        .collect();
    let addresses: BTreeSet<Addr> = resources.iter().map(|r| r.address.clone()).collect();
    obs.book = TimingBook::new(resources, spec.clock_bound, spec.across_face, t0);
    if spec.across_face
        && let Some(a) = scope.one_owner()
    {
        obs.book.stated_zids(&a, spec.owner_zids.iter().copied());
    }
    // 2. Listen, in one queue.
    let refused: Arc<AtomicU64> = Arc::default();
    let discarded: Arc<AtomicU64> = Arc::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(QUEUE);
    let queue = Queue {
        tx,
        refused: Arc::clone(&refused),
    };
    let mut held = Vec::new();
    let mut barrier = None;
    if !spec.across_face {
        let watch = match watch {
            Some(w) if w.zid() != session.zid() => w.clone(),
            _ => {
                obs.unobservable = Some(
                    "the presence subscription needs a watch session of its own: no presence \
                     read is made beside a liveliness subscriber (#830, core §8.1)"
                        .to_owned(),
                );
                return obs;
            }
        };
        for sel in presence_scope.token_selectors() {
            let q = queue.clone();
            let declared = watch
                .subscribe("timing's presence tokens", &sel, move |h| {
                    q.send(Delivery::Token {
                        key: h.key,
                        held: h.held,
                    });
                })
                .await;
            match declared {
                Ok(sub) => held.push(sub),
                Err(e) => {
                    obs.unobservable = Some(format!(
                        "the subscription to {sel} could not be declared: {}",
                        crate::one_line(&e)
                    ));
                    return obs;
                }
            }
            obs.asked.push(sel);
        }
        let first = obs.asked.last().cloned().unwrap_or_default();
        match watch.barrier(&first, t).await {
            Ok(b) => barrier = Some(b),
            Err(e) => {
                obs.unobservable = Some(format!(
                    "the presence subscriptions' barrier could not be kept: {}",
                    crate::one_line(&e)
                ));
                return obs;
            }
        }
    }
    for sel in &subscriptions {
        let q = queue.clone();
        let wild = Arc::clone(&discarded);
        let declared = crate::bus::teardown::declared(
            "timing's members",
            sel,
            session
                .declare_subscriber(sel.clone())
                .callback(move |s: Sample| {
                    // §2.5, §2.6: the arrival's two clocks, read here.
                    let arrival = Instant::now();
                    let wall = SystemTime::now();
                    if s.key_expr().is_wild() {
                        wild.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    q.send(Delivery::Sample(TimedSample {
                        key: s.key_expr().as_str().to_owned(),
                        delete: s.kind() == SampleKind::Delete,
                        stamp: s.timestamp().copied(),
                        arrival,
                        wall,
                    }));
                }),
        )
        .await;
        match declared {
            Ok(sub) => held.push(sub),
            Err(e) => {
                obs.unobservable = Some(format!(
                    "the subscription to {sel} could not be declared: {}",
                    crate::one_line(&e)
                ));
                return obs;
            }
        }
        obs.asked.push(sel.clone());
    }
    drop(queue);
    let stall = StallWatch::start(session).await;
    let mut reader = Reader {
        session: session.clone(),
        presence_scope: presence_scope.clone(),
        timeout: t,
        barrier: barrier.clone(),
        moves_seen: barrier.as_ref().map_or(0, zenkey::presence::Barrier::moves),
        refused,
        stall,
        stall_seen: StallSeen::default(),
        stall_bound: spec.stall_bound(),
        instances: BTreeMap::new(),
        asked_ids: BTreeMap::new(),
        pending: BTreeSet::new(),
        reseeding: false,
        answers: tokio::sync::mpsc::unbounded_channel(),
    };
    // 3. The window's start: presence read after the round trip, with every
    // instance's descriptor, made again once when it did not stand.
    let opening = if spec.across_face {
        tokio::time::sleep(spec.settle).await;
        TimingOpening {
            presence: "across_face".into(),
            selector: String::new(),
            instances: 0,
            descriptors: 0,
            unanswered: vec![],
            stood: false,
        }
    } else {
        let mut opening = None;
        for attempt in 0..2 {
            if attempt > 0 {
                // A silent descriptor or an unsettled read is asked again a
                // pause later, as the telemetry reader's seed is, never at
                // once (review-bot on PR #917). The queue is folded while it
                // waits, so a fast stream fills nothing.
                let until = tokio::time::Instant::now() + RETRY;
                loop {
                    tokio::select! {
                        () = tokio::time::sleep_until(until) => break,
                        d = rx.recv() => match d {
                            Some(d) => reader.fold(&mut obs.book, d),
                            None => break,
                        },
                    }
                }
            }
            let settled = reader.establish().await;
            let read = crate::bus::presence::observe(session, &presence_scope, t).await;
            let at = Instant::now();
            reader.drain(&mut obs.book, &mut rx);
            let o = match read {
                Ok(o) => {
                    reader.note_instances(&o);
                    obs.book.seed(&o, settled, at);
                    opening_of(&o, settled, &addresses)
                }
                Err(e) => TimingOpening {
                    presence: "failed".into(),
                    selector: presence_scope.selector(),
                    instances: 0,
                    descriptors: 0,
                    unanswered: vec![crate::one_line(&e)],
                    stood: false,
                },
            };
            let stood = o.stood;
            opening = Some(o);
            if stood {
                break;
            }
        }
        opening.expect("one attempt at least")
    };
    reader.drain(&mut obs.book, &mut rx);
    obs.opening = Some(opening.clone());
    // The window starts.
    let opened = Instant::now();
    obs.book.start(opened);
    listening(&opening);
    let deadline = tokio::time::Instant::from_std(opened + spec.window);
    // 4. The window.
    reader.window(&mut obs.book, &mut rx, deadline).await;
    reader.drain(&mut obs.book, &mut rx);
    obs.book.finish(Instant::now());
    drop(held);
    obs.discarded = discarded.load(Ordering::Relaxed);
    let mut seen = BTreeSet::new();
    obs.asked.retain(|s| seen.insert(s.clone()));
    obs
}

/// What a presence read with its descriptors stood on (§2.7).
fn opening_of(o: &Observed, settled: bool, addresses: &BTreeSet<Addr>) -> TimingOpening {
    let presence = if o.cut_off_during {
        "cut_off"
    } else if !settled {
        "unsettled"
    } else if o.complete {
        "complete"
    } else {
        "incomplete"
    };
    let wanted: Vec<(Addr, InstanceId)> = o
        .instances()
        .into_iter()
        .filter(|(a, _)| addresses.contains(a))
        .collect();
    let mut answered = 0;
    let mut unanswered = Vec::new();
    for (a, i) in &wanted {
        match o
            .descriptors
            .as_ref()
            .and_then(|d| d.get(&(a.clone(), i.clone())))
        {
            Some(DescriptorRead::Served(_) | DescriptorRead::Invalid(_)) => answered += 1,
            Some(DescriptorRead::Silent) => unanswered.push(format!("{a} {i}: silent")),
            Some(DescriptorRead::Failed(why)) => unanswered.push(format!("{a} {i}: {why}")),
            None => unanswered.push(format!("{a} {i}: its descriptor was not read")),
        }
    }
    TimingOpening {
        presence: presence.into(),
        selector: o.selector.clone(),
        instances: wanted.len(),
        descriptors: answered,
        stood: presence == "complete" && unanswered.is_empty(),
        unanswered,
    }
}

/// The reader's state beside the book: its sessions, what it watches of
/// itself, and the reads it has in flight.
struct Reader {
    session: Session,
    presence_scope: crate::bus::presence::Scope,
    timeout: Duration,
    barrier: Option<zenkey::presence::Barrier>,
    moves_seen: u64,
    refused: Arc<AtomicU64>,
    stall: std::result::Result<StallWatch, String>,
    stall_seen: StallSeen,
    stall_bound: Duration,
    /// Each address's instances, as reads showed and tokens told.
    instances: BTreeMap<Addr, BTreeSet<InstanceId>>,
    /// Descriptor GETs asked at a stamp of an unknown id, per address: how
    /// many, and when last.
    asked_ids: BTreeMap<Addr, (u32, Instant)>,
    /// The withdrawals' reads in flight.
    pending: BTreeSet<Owed>,
    reseeding: bool,
    answers: (
        tokio::sync::mpsc::UnboundedSender<Answered>,
        tokio::sync::mpsc::UnboundedReceiver<Answered>,
    ),
}

impl Reader {
    /// The watch session's barrier, made unless it stands; `true` with no
    /// barrier (across a face).
    async fn establish(&mut self) -> bool {
        match self.barrier.clone() {
            Some(b) => {
                self.moves_seen = b.moves();
                b.establish().await
            }
            None => true,
        }
    }

    fn note_instances(&mut self, o: &Observed) {
        for (a, i) in o.instances() {
            self.instances.entry(a).or_default().insert(i);
        }
    }

    /// Everything the queue holds now, folded, its losses first.
    fn drain(&mut self, book: &mut TimingBook, rx: &mut tokio::sync::mpsc::Receiver<Delivery>) {
        while let Ok(d) = rx.try_recv() {
            self.fold(book, d);
        }
        self.losses(book);
    }

    fn fold(&mut self, book: &mut TimingBook, d: Delivery) {
        self.losses(book);
        match d {
            Delivery::Sample(s) => {
                let unknown = s
                    .stamp
                    .and_then(|t| zenkey::timing::stamp_id(&t))
                    .zip(zenkey_model::grammar::parse(&s.key).ok())
                    .and_then(|(id, k)| match k {
                        ZkKey::Data { addr, .. } => {
                            (!book.owner_zids(&addr).contains(&id)).then_some(addr)
                        }
                        _ => None,
                    });
                book.sample(&s);
                if let Some(addr) = unknown
                    && book.follows_presence()
                {
                    let ask = self.asked_ids.entry(addr.clone()).or_insert((0, s.arrival));
                    if ask.0 == 0
                        || (ask.0 < ASK_LIMIT
                            && s.arrival.saturating_duration_since(ask.1) >= ASK_EVERY)
                    {
                        *ask = (ask.0 + 1, s.arrival);
                        let wanted: Vec<(Addr, InstanceId)> = self
                            .instances
                            .get(&addr)
                            .into_iter()
                            .flatten()
                            .map(|i| (addr.clone(), i.clone()))
                            .collect();
                        self.describe(wanted);
                    }
                }
            }
            Delivery::Token { key, held } => {
                let at = Instant::now();
                if held
                    && let Ok(ZkKey::Instance { addr, instance }) =
                        zenkey_model::grammar::parse(&key)
                {
                    self.instances
                        .entry(addr.clone())
                        .or_default()
                        .insert(instance.clone());
                    self.describe(vec![(addr, instance)]);
                }
                if let Some(owed) = book.token(&key, held, at) {
                    self.owe(owed);
                }
            }
        }
    }

    /// Asks the descriptors of `wanted`, beside the window.
    fn describe(&self, wanted: Vec<(Addr, InstanceId)>) {
        if wanted.is_empty() {
            return;
        }
        let (session, t, tx) = (self.session.clone(), self.timeout, self.answers.0.clone());
        tokio::spawn(async move {
            let reads = crate::bus::presence::describe_each(&session, wanted.clone(), t).await;
            let mut o = Observed::default();
            for (a, i) in wanted {
                o.tokens.push(ZkKey::Instance {
                    addr: a.clone(),
                    instance: i.clone(),
                });
                if let Some(r) = reads.get(&(a.clone(), i.clone())) {
                    o.with_descriptor(&a, &i, r.clone());
                }
            }
            let _ = tx.send(Answered::Described(o));
        });
    }

    /// The read `owed`, [`REREAD`] from now, unless one is in flight.
    fn owe(&mut self, owed: Owed) {
        if !self.pending.insert(owed.clone()) {
            return;
        }
        let (session, t, tx, barrier) = (
            self.session.clone(),
            self.timeout,
            self.answers.0.clone(),
            self.barrier.clone(),
        );
        tokio::spawn(async move {
            tokio::time::sleep(REREAD).await;
            let settled = match &barrier {
                Some(b) => b.establish().await,
                None => true,
            };
            let read = crate::bus::presence::liveliness_read(&session, &owed.selector(), t).await;
            let at = Instant::now();
            let (keys, complete) = match read {
                Ok(r) => (r.keys, r.complete && settled),
                Err(_) => (Vec::new(), false),
            };
            let _ = tx.send(Answered::Owed {
                owed,
                keys,
                complete,
                at,
            });
        });
    }

    /// Presence read again, with its descriptors, unless a read is in
    /// flight.
    fn reseed(&mut self) {
        if self.reseeding || self.barrier.is_none() {
            return;
        }
        self.reseeding = true;
        let (session, scope, t, tx, barrier) = (
            self.session.clone(),
            self.presence_scope.clone(),
            self.timeout,
            self.answers.0.clone(),
            self.barrier.clone(),
        );
        tokio::spawn(async move {
            let settled = match &barrier {
                Some(b) => b.establish().await,
                None => true,
            };
            let read = crate::bus::presence::observe(&session, &scope, t)
                .await
                .map_err(|e| crate::one_line(&e));
            let _ = tx.send(Answered::Reseed {
                read,
                settled,
                at: Instant::now(),
            });
        });
    }

    fn answered(&mut self, book: &mut TimingBook, a: Answered) {
        match a {
            Answered::Owed {
                owed,
                keys,
                complete,
                at,
            } => {
                self.pending.remove(&owed);
                if let Some(again) = book.owed_read(&owed, &keys, complete, at) {
                    self.owe(again);
                }
            }
            Answered::Described(o) => {
                book.descriptors(&Catalog::new(&o));
            }
            Answered::Reseed { read, settled, at } => {
                self.reseeding = false;
                match read {
                    Ok(o) => {
                        self.note_instances(&o);
                        book.seed(&o, settled, at);
                        if !(o.complete && settled) {
                            self.reseed();
                        }
                    }
                    Err(_) => self.reseed(),
                }
            }
        }
    }

    /// The deliveries the queue refused, and a stall of the receive path,
    /// as losses: each reads presence again.
    fn losses(&mut self, book: &mut TimingBook) {
        let refused = self.refused.swap(0, Ordering::Relaxed);
        if refused > 0 {
            book.lose(refused, Instant::now());
            self.reseed();
        }
        let stalled = match &self.stall {
            Err(_) => false,
            Ok(w) => {
                let seen = w.read();
                let fresh = (seen.gap > self.stall_bound
                    && self.stall_seen.gap <= self.stall_bound)
                    || seen.transports_closed > self.stall_seen.transports_closed;
                self.stall_seen = seen;
                fresh
            }
        };
        if stalled {
            book.lose(0, Instant::now());
            self.reseed();
        }
    }

    /// The window: every delivery folded as it comes, each owed read and
    /// descriptor answer folded after what the queue holds, and the
    /// trackers judged at their earliest due, until `deadline`.
    async fn window(
        &mut self,
        book: &mut TimingBook,
        rx: &mut tokio::sync::mpsc::Receiver<Delivery>,
        deadline: tokio::time::Instant,
    ) {
        let mut open = true;
        loop {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            // A one-shot timer just past the earliest due: a miss holds
            // only after it (§2.5).
            let due = book
                .next_due()
                .map(|d| tokio::time::Instant::from_std(d + Duration::from_micros(50)));
            let (barrier, seen) = (self.barrier.clone(), self.moves_seen);
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                () = until(due) => {
                    self.drain(book, rx);
                    book.tick(Instant::now());
                }
                () = watch_moved(barrier.as_ref(), seen) => {
                    self.drain(book, rx);
                    if let Some(b) = &self.barrier {
                        self.moves_seen = b.moves();
                    }
                    // Core §8.1, 0.42: what the subscribers told may be
                    // behind; presence is read again.
                    book.lose(0, Instant::now());
                    self.reseed();
                }
                a = self.answers.1.recv() => {
                    self.drain(book, rx);
                    if let Some(a) = a {
                        self.answered(book, a);
                    }
                }
                d = recv(rx, open) => match d {
                    Some(d) => self.fold(book, d),
                    None => open = false,
                },
            }
        }
    }
}

/// The queue's next delivery, or never once it is closed.
async fn recv(rx: &mut tokio::sync::mpsc::Receiver<Delivery>, open: bool) -> Option<Delivery> {
    if open {
        rx.recv().await
    } else {
        std::future::pending().await
    }
}

/// Resolves once the watch session has had a transport event since the
/// reader last answered `seen`; never without a barrier.
async fn watch_moved(barrier: Option<&zenkey::presence::Barrier>, seen: u64) {
    match barrier {
        Some(b) => b.moved_since(seen).await,
        None => std::future::pending().await,
    }
}

/// Resolves at `at`; never with none.
async fn until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
