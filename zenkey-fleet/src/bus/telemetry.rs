//! `telemetry.v1` (#819, TC), the reads: the metric members a scope reaches
//! (`spec/profiles/telemetry/v1.md`), read as §2.9–§2.11's reader reads
//! them, through a session in the deployment's namespace.
//!
//! What is read, in order:
//! 1. presence and every instance's descriptor (core §8.1, §3.3), and every
//!    revision they name, retrieved (§8.4): the metric resources in scope
//!    are those revisions' (§2.1). Across a constrained face, where neither
//!    crosses, the deployment's word stands for them: one owner, and the
//!    contracts in hand (§2.14);
//! 2. when the reader listens: a liveliness subscription to the owners'
//!    instance and member tokens, on a **watch session** of its own and
//!    settled there (#830, core §8.1, 0.37: no presence read is made beside
//!    a liveliness subscriber, and the settle orders the subscription
//!    before the seed on the other session), then a subscription to each
//!    metric resource's members, then a watch of this reader's own receive
//!    path
//!    ([`crate::bus::stall`], core §2.7). Every subscription calls back into
//!    **one** bounded queue, so samples and token moves are folded in the one
//!    order they were received, and what the queue refuses is counted: a
//!    loss, which leaves every pair across it without presence (§2.9);
//! 3. a liveliness read of the owners' tokens, the seed presence stands on
//!    ([`TelemetryBook::seed`]), read again after every loss;
//! 4. **the window opens** (zk2py round 47's F-154): the caller's
//!    `listening` runs with what it opened on ([`Opening`]), and
//!    `spec.window` counts from that moment. The subscriptions are declared
//!    and, after the seed, each owner instance in scope answered its
//!    descriptor GET (#819 TG): the seed proves only that the router holds
//!    them, and an owner's put is dropped at its source until its session
//!    does. So a sample put from then on is the window's. Across a face,
//!    where no presence is read, step 5 comes first: S4's GET, answered by
//!    the owner after the subscriptions, is the round trip (round 49's
//!    F-159). With no state metric resource in scope, nothing can answer:
//!    the window opens after `spec.settle`, a margin, never proof;
//! 5. S4's GET of each state metric resource (core §4.2), its replies merged
//!    with the live puts by the owner's stamp in the book (§2.10, 0.2);
//! 6. the window, every delivery folded as it comes.
//!
//! Nothing is judged here: every sample is read and paired by
//! `zenkey_model::telemetry` inside the book ([`crate::model::telemetry`]),
//! and judged by [`crate::judge::telemetry`].

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey_model::authoring::Kind;
use zenoh::Session;
use zenoh::sample::{Sample, SampleKind};

use crate::bus::contracts::BundleStore;
use crate::bus::stall::{StallSeen, StallWatch};
use crate::model::catalog::{Catalog, ContractSet, ContractState, Observed};
use crate::model::lens::Lens;
use crate::model::telemetry::{
    MAX_MEMBERS, MetricResource, MetricSample, SampleVia, TelemetryBook, TelemetryScope,
};

/// The queue the subscriptions call back into: the monitor's default
/// broadcast capacity, as alarms' reader's.
pub const QUEUE: usize = 1024;

/// What a reading asks.
#[derive(Debug, Clone, Copy)]
pub struct TelemetrySpec {
    /// Each read's timeout: presence, a descriptor, a retrieval, a GET.
    pub timeout: Duration,
    /// How long the window runs, from its opening: the subscriptions
    /// declared and presence seeded (step 4 of the module doc).
    pub window: Duration,
    /// The deployment's link lease (core §2.7): with `wait_before_close`,
    /// the longest this reader may stall before it counts a loss.
    pub lease: Duration,
    /// The routers' `wait_before_close` (core §2.7, 0.27).
    pub wait_before_close: Duration,
    /// Across a constrained face (§2.14), on the deployment's word: no
    /// presence is read, and the metric resources are the contracts' in
    /// hand, of the one owner the scope names.
    pub across_face: bool,
    /// Across a face with no state metric resource in scope to GET, how
    /// long the window waits after the subscriptions are declared before it
    /// opens: a margin for the declarations to cross, never proof that they
    /// did ([`Opening::Margin`], zk2py round 49's F-159).
    pub settle: Duration,
}

/// [`TelemetrySpec::settle`]'s default: five times what a loopback face
/// was measured to need (#819 TG: 24 of 24 samples put on the line lost
/// with none, 0 of 24 at 50 ms).
pub const DEFAULT_SETTLE: Duration = Duration::from_millis(250);

impl TelemetrySpec {
    /// A window, zenoh's lease and `wait_before_close`, no face.
    pub fn new(timeout: Duration, window: Duration) -> TelemetrySpec {
        TelemetrySpec {
            timeout,
            window,
            lease: zenkey_model::budget::DEFAULT_LEASE,
            wait_before_close: zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE,
            across_face: false,
            settle: DEFAULT_SETTLE,
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

/// One presence seed, summed up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedSeen {
    pub selector: String,
    pub complete: bool,
    /// Services in scope holding an instance token.
    pub services: usize,
}

/// One reading of a scope's metric members, as values: what
/// [`crate::judge::telemetry::judge`] decides from.
#[derive(Debug, Clone)]
pub struct TelemetryObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub spec: TelemetrySpec,
    pub scope: TelemetryScope,
    /// The metric resources in scope.
    pub resources: Vec<Arc<MetricResource>>,
    /// The last presence seed, or why it could not be read; `None` across a
    /// face.
    pub seed: Option<Result<SeedSeen, String>>,
    pub book: TelemetryBook,
    /// The selectors put to the bus, base-relative.
    pub asked: Vec<String>,
    /// How long the window ran, from its opening.
    pub listened: Duration,
    /// Samples on a key that resolved to no metric member, or a wildcard
    /// key (R6).
    pub unresolved: u64,
    /// What in scope could not be read, one line each.
    pub skipped: Vec<String>,
    /// Set when the reading could not be made at all, with why.
    pub unobservable: Option<String>,
    /// What the window opened on; `None` when it never opened.
    pub opening: Option<Opening>,
}

/// What a subscription called back with.
enum Delivery {
    Sample(MetricSample),
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

fn sample_of(s: &Sample, via: SampleVia) -> MetricSample {
    MetricSample {
        key: s.key_expr().as_str().to_owned(),
        delete: s.kind() == SampleKind::Delete,
        payload: s.payload().to_bytes().into_owned(),
        attachment: s.attachment().map(|a| a.to_bytes().into_owned()),
        stamp: s.timestamp().copied(),
        at: Instant::now(),
        via,
    }
}

/// The metric resources in scope: every revision the descriptors in scope
/// name and the store holds, or, across a face, every contract in hand of
/// the one owner the scope names. A revision not in hand is skipped, with
/// why.
pub fn metric_resources(
    scope: &TelemetryScope,
    catalog: Option<&Catalog>,
    store: &BundleStore,
    offline: &ContractSet,
    skipped: &mut Vec<String>,
) -> Vec<Arc<MetricResource>> {
    let mut out = Vec::new();
    let mut push = |m: MetricResource| {
        if scope.subscription(&m).is_some() {
            out.push(Arc::new(m));
        }
    };
    match catalog {
        Some(c) => {
            for addr in c.addresses().filter(|a| scope.selects_owner(a)) {
                for iface in c.provided(addr) {
                    for fp in c.revisions_of(addr, &iface) {
                        match crate::model::catalog::Contracts::state(store, &iface, &fp) {
                            Some(ContractState::Held(rev)) => {
                                for i in 0..rev.contract().resources.len() {
                                    if let Some(m) = MetricResource::of(addr, &rev, i) {
                                        push(m);
                                    }
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
                    for i in 0..rev.contract().resources.len() {
                        if let Some(m) = MetricResource::of(&addr, rev, i) {
                            push(m);
                        }
                    }
                }
            }
        }
    }
    out
}

/// One reading: [`observe`], then [`crate::judge::telemetry::judge`].
#[allow(clippy::too_many_arguments)]
pub async fn run_telemetry(
    session: &Session,
    watch: Option<&Session>,
    namespace: &str,
    scope: TelemetryScope,
    store: &BundleStore,
    offline: &ContractSet,
    spec: TelemetrySpec,
    listening: impl FnOnce(Opening) + Send,
) -> crate::report::TelemetryReport {
    let obs = observe(
        session, watch, namespace, scope, store, offline, spec, listening,
    )
    .await;
    crate::judge::telemetry::judge(&obs)
}

/// The reader of the module doc, over `spec.window`. `listening` runs once,
/// when the window opens (step 4), and never when the reading ends before
/// it: nothing in scope, or a read or a subscription that could not be
/// made. Its presence subscription is held on `watch`, a session of its
/// own (#830); with none, or `session` itself, the reading is
/// unobservable, saying why, unless it runs across a face, where no
/// presence is read.
#[allow(clippy::too_many_arguments)]
pub async fn observe(
    session: &Session,
    watch: Option<&Session>,
    namespace: &str,
    scope: TelemetryScope,
    store: &BundleStore,
    offline: &ContractSet,
    spec: TelemetrySpec,
    listening: impl FnOnce(Opening) + Send,
) -> TelemetryObservation {
    let t = spec.timeout;
    let presence_scope = scope.presence_scope();
    let mut obs = TelemetryObservation {
        namespace: namespace.to_owned(),
        spec,
        scope: scope.clone(),
        resources: Vec::new(),
        seed: None,
        book: TelemetryBook::new(
            (!spec.across_face).then(|| presence_scope.clone()),
            Instant::now(),
        ),
        asked: Vec::new(),
        listened: Duration::ZERO,
        unresolved: 0,
        skipped: Vec::new(),
        unobservable: None,
        opening: None,
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
    obs.resources = metric_resources(&scope, catalog.as_ref(), store, offline, &mut skipped);
    obs.skipped = skipped;
    if obs.resources.is_empty() {
        return obs;
    }
    let presence_scope_selectors = presence_scope.token_selectors();
    let mut reader = Reader {
        seeder: Seeder::Bus {
            session: session.clone(),
            scope: presence_scope,
            timeout: t,
        },
        spec,
        catalog,
        store,
        offline,
        by_resource: obs
            .resources
            .iter()
            .map(|m| {
                (
                    (m.address.clone(), m.fingerprint.clone(), m.resource.clone()),
                    Arc::clone(m),
                )
            })
            .collect(),
        resolved: BTreeMap::new(),
        refused: Arc::default(),
        discarded: Arc::default(),
        unresolved: 0,
        stall: None,
        stall_seen: StallSeen::default(),
        owed_seed: false,
        seed: None,
        seeded_instances: None,
    };
    let subscriptions: BTreeSet<String> = obs
        .resources
        .iter()
        .filter_map(|m| scope.subscription(m))
        .collect();
    // 2. Listen, in one queue.
    let (tx, mut rx) = tokio::sync::mpsc::channel(QUEUE);
    let queue = Queue {
        tx,
        refused: Arc::clone(&reader.refused),
    };
    let mut held = Vec::new();
    if !spec.across_face {
        // On a watch session of its own (#830, core §8.1, 0.37): the seed
        // below reads presence on `session`, which holds no liveliness
        // subscriber.
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
        let [instances, _alive, members] = presence_scope_selectors;
        for sel in [instances, members] {
            let q = queue.clone();
            let declared = crate::bus::teardown::declared(
                "telemetry's presence tokens",
                &sel,
                zenkey::presence::declare_subscriber(&watch, sel.clone()).callback(
                    move |s: Sample| {
                        q.send(Delivery::Token {
                            key: s.key_expr().as_str().to_owned(),
                            held: s.kind() == SampleKind::Put,
                        });
                    },
                ),
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
            obs.asked.push(sel);
        }
        // Its routers have read the subscriptions before the seed on the
        // other session is answered (#830).
        let first = obs.asked.last().cloned().unwrap_or_default();
        if !zenkey::presence::settle(&watch, &first, t)
            .await
            .unwrap_or(false)
        {
            tracing::warn!(
                selector = %first,
                "telemetry's presence tokens did not settle: a withdrawal right after the seed \
                 may not be told"
            );
        }
    }
    for sel in &subscriptions {
        let q = queue.clone();
        let discarded = Arc::clone(&reader.discarded);
        let declared = crate::bus::teardown::declared(
            "telemetry's metric members",
            sel,
            session
                .declare_subscriber(sel.clone())
                .callback(move |s: Sample| {
                    if s.key_expr().is_wild() {
                        discarded.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    q.send(Delivery::Sample(sample_of(&s, SampleVia::Subscription)));
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
    reader.stall = Some(StallWatch::start(session).await);
    // 3. The seed presence stands on, bounded by its reads' own timeouts,
    // then the round trips the window opens on (F-154, F-159): ones that
    // reach every owner in scope after the subscriptions, made at once, so
    // they cost one timeout.
    let opening = if spec.across_face {
        // Across a face there is no seed, so S4's GETs come first: the
        // owner's answer, made after the subscriptions, is the round trip
        // (zk2py round 49's F-159). With no state metric resource in scope
        // there is none to make.
        let trips = state_gets(session, &scope, &mut obs, &mut reader, &mut rx, t, None).await;
        // The scope names one owner across a face, and one reply from its
        // session that ran to its final proves that every declaration made
        // before the GET reached that session: one answered is enough.
        if trips.answered > 0 {
            Opening::Answered(trips)
        } else {
            tokio::time::sleep(spec.settle).await;
            Opening::Margin {
                settle: spec.settle,
                trips,
            }
        }
    } else {
        reader.reseed(&mut obs.book, &mut rx, None).await;
        // The seed is answered by the router from its own table: it proves
        // the router holds the subscriptions, never that an owner's session
        // does, and zenoh drops a put at its source until the owner's
        // session knows a matching subscriber (#819 TG's gate: 11 of 400
        // samples put on the line lost under load, each with its writer
        // unmatched). Each owner instance's descriptor GET is answered by
        // that session after the declarations reach it, over the same link.
        // A seed that did not stand names no instance worth asking: the
        // first read's may be gone, so none is asked.
        match to_ask(reader.seeded_instances.as_ref(), &obs.resources) {
            Err(nothing) => nothing,
            Ok(wanted) => {
                let trips = owners_answer(session, wanted, t).await;
                reader.drain(&mut obs.book, &mut rx);
                if trips.unanswered.is_empty() {
                    Opening::Answered(trips)
                } else {
                    Opening::Unanswered(trips)
                }
            }
        }
    };
    obs.opening = Some(opening.clone());
    // 4. The window opens (F-154), and every read below keeps to its end: a
    // state GET, a seed owed by a loss.
    listening(opening);
    let opened = Instant::now();
    let deadline = tokio::time::Instant::from_std(opened + spec.window);
    // 5. S4's GET of each state metric resource, after what the queue holds
    // (made already across a face).
    if !spec.across_face {
        state_gets(
            session,
            &scope,
            &mut obs,
            &mut reader,
            &mut rx,
            t,
            Some(deadline),
        )
        .await;
    }
    // 6. The window.
    reader.window(&mut obs.book, &mut rx, deadline).await;
    obs.listened = opened.elapsed();
    drop(held);
    obs.seed = reader.seed.take();
    obs.unresolved = reader.unresolved + reader.discarded.load(Ordering::Relaxed);
    let mut seen = BTreeSet::new();
    obs.asked.retain(|s| seen.insert(s.clone()));
    obs
}

/// What a window opened on (zk2py round 47's F-154, round 49's F-159, #819
/// TG): the round trips made after the subscriptions were declared that
/// reach each owner in scope, whose answers come back after the
/// declarations reached the owner's session over the same links, so it
/// holds them and a put of its is not dropped at its source. A sample put
/// once the window opens is inside it when it opened [`Opening::Answered`];
/// every other opening says what was not proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// The proof holds. With no face, every owner instance in scope the
    /// presence seed found answered its descriptor GET, made after the seed.
    /// Across a face, where the scope names one owner and no presence is
    /// read, the owner replied to at least one S4 GET of a state metric
    /// resource in scope, and its final reply came.
    Answered(RoundTrips),
    /// With no face, some owner instance in scope answered no descriptor
    /// GET: silent within the timeout, or its GET failed. Those may not
    /// hold the subscriptions yet, and a sample one puts at once may be
    /// dropped before it leaves.
    Unanswered(RoundTrips),
    /// With no face, nothing was asked, so nothing was proved: the seed
    /// found no instance of an owner in scope (`seeded`), or it did not
    /// stand, so which instances are in scope is unknown. An owner that
    /// starts later learns the subscriptions as it starts.
    NoOwner { seeded: bool },
    /// Across a face, no S4 GET answered: none in scope (a stream has no
    /// queryable, and presence and descriptors do not cross, so nothing
    /// else could), or each drew no reply, was silent or failed. The window
    /// opened after [`TelemetrySpec::settle`], a margin and never proof.
    Margin { settle: Duration, trips: RoundTrips },
}

/// The round trips a window's opening made, counted: how many were asked,
/// how many an owner answered, how many ran to their final reply with no
/// reply, how many were silent within their timeout and how many failed,
/// each of the last three named.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoundTrips {
    pub asked: usize,
    /// An owner replied, and the final reply came.
    pub answered: usize,
    /// The final reply came with no reply: nothing shows an owner was
    /// reached (a GET no queryable matches ends so at once).
    pub empty: usize,
    pub silent: usize,
    pub errored: usize,
    /// Each round trip that did not answer, in words: the selector or the
    /// owner instance, and why.
    pub unanswered: Vec<String>,
}

/// The owner instances a window asks after its seed (#819 TG): those the
/// seed that stood found, of an owner of a metric resource in scope.
/// Nothing to ask is an [`Opening::NoOwner`]: a seed that did not stand
/// (whose first read's instances may be gone, so none is asked), or one
/// that found none.
fn to_ask(
    seeded: Option<
        &BTreeSet<(
            zenkey_model::grammar::Addr,
            zenkey_model::grammar::InstanceId,
        )>,
    >,
    resources: &[Arc<MetricResource>],
) -> Result<
    Vec<(
        zenkey_model::grammar::Addr,
        zenkey_model::grammar::InstanceId,
    )>,
    Opening,
> {
    let Some(seeded) = seeded else {
        return Err(Opening::NoOwner { seeded: false });
    };
    let owners: BTreeSet<&zenkey_model::grammar::Addr> =
        resources.iter().map(|m| &m.address).collect();
    let wanted: Vec<_> = seeded
        .iter()
        .filter(|(a, _)| owners.contains(a))
        .cloned()
        .collect();
    if wanted.is_empty() {
        return Err(Opening::NoOwner { seeded: true });
    }
    Ok(wanted)
}

/// Each of `wanted`'s descriptor GETs (core §3.3), made at once
/// ([`crate::bus::presence::describe_each`]: up to
/// [`crate::bus::presence::DESCRIBE_CONCURRENCY`] in flight, so one
/// timeout up to that many instances), counted: a descriptor served or one
/// that fails its check answered, a silent GET did not, and a GET that
/// could not be made, or an instance the reads never returned, failed.
async fn owners_answer(
    session: &Session,
    wanted: Vec<(
        zenkey_model::grammar::Addr,
        zenkey_model::grammar::InstanceId,
    )>,
    t: Duration,
) -> RoundTrips {
    use crate::model::catalog::DescriptorRead;
    let mut trips = RoundTrips {
        asked: wanted.len(),
        ..RoundTrips::default()
    };
    let reads = crate::bus::presence::describe_each(session, wanted.clone(), t).await;
    for (addr, instance) in &wanted {
        match reads.get(&(addr.clone(), instance.clone())) {
            Some(DescriptorRead::Served(_) | DescriptorRead::Invalid(_)) => trips.answered += 1,
            Some(DescriptorRead::Silent) => {
                trips.silent += 1;
                trips.unanswered.push(format!(
                    "{addr} {instance}: silent within {}s",
                    t.as_secs_f64()
                ));
            }
            Some(DescriptorRead::Failed(why)) => {
                trips.errored += 1;
                trips.unanswered.push(format!("{addr} {instance}: {why}"));
            }
            None => {
                trips.errored += 1;
                trips.unanswered.push(format!(
                    "{addr} {instance}: its descriptor GET never returned"
                ));
            }
        }
    }
    trips
}

/// S4's GET of each state metric resource in scope, all at once, then
/// what the queue holds, then each GET's replies, folded by the owner's
/// stamp; bounded by `deadline` when given, else by each GET's own
/// timeout. The round trips counted: a GET answered when an owner replied
/// and its final reply came; one whose final came with no reply proves
/// nothing (`empty`); one that ran to its timeout (or the window's end)
/// was silent; and one that could not be made or drew an error reply
/// failed, named in the count and in `skipped`.
async fn state_gets(
    session: &Session,
    scope: &TelemetryScope,
    obs: &mut TelemetryObservation,
    reader: &mut Reader<'_>,
    rx: &mut tokio::sync::mpsc::Receiver<Delivery>,
    t: Duration,
    deadline: Option<tokio::time::Instant>,
) -> RoundTrips {
    let selectors: Vec<String> = obs
        .resources
        .iter()
        .filter(|m| m.kind == Kind::State)
        .filter_map(|m| scope.subscription(m))
        .collect();
    let gets = selectors.iter().map(|sel| async move {
        match deadline {
            Some(d) => tokio::time::timeout_at(d, state_replies(session, sel, t))
                .await
                .unwrap_or_else(|_| Ok((Vec::new(), GetEnd::WindowEnded))),
            None => state_replies(session, sel, t).await,
        }
    });
    let ended = futures_util::future::join_all(gets).await;
    reader.drain(&mut obs.book, rx);
    let mut trips = RoundTrips {
        asked: selectors.len(),
        ..RoundTrips::default()
    };
    for (sel, end) in selectors.into_iter().zip(ended) {
        match end {
            Ok((rs, end)) => {
                for s in rs {
                    reader.fold_sample(&mut obs.book, s);
                }
                match end {
                    GetEnd::Final => trips.answered += 1,
                    GetEnd::Empty => {
                        trips.empty += 1;
                        trips.unanswered.push(format!(
                            "GET {sel}: its final reply came with no reply, so nothing shows it \
                             reached an owner"
                        ));
                    }
                    GetEnd::Timeout => {
                        trips.silent += 1;
                        trips.unanswered.push(format!(
                            "GET {sel}: no final reply within {}s",
                            t.as_secs_f64()
                        ));
                    }
                    GetEnd::WindowEnded => {
                        trips.silent += 1;
                        let why = format!(
                            "GET {sel}: the window ended before its replies did, so it seeded \
                             nothing"
                        );
                        trips.unanswered.push(why.clone());
                        obs.skipped.push(why);
                    }
                    GetEnd::Erred(e) => {
                        trips.errored += 1;
                        let why = format!("GET {sel}: an error reply, {e}");
                        trips.unanswered.push(why.clone());
                        obs.skipped.push(why);
                    }
                }
            }
            Err(e) => {
                trips.errored += 1;
                trips.unanswered.push(e.clone());
                obs.skipped.push(e);
            }
        }
        obs.asked.push(sel);
    }
    trips
}

/// How a GET ended.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GetEnd {
    /// Its final reply came, after at least one reply: an owner answered.
    Final,
    /// Its final reply came with no reply before it. A GET no queryable
    /// matches ends so at once, so nothing shows it reached an owner.
    Empty,
    /// zenoh ended it at its timeout.
    Timeout,
    /// The window ended first.
    WindowEnded,
    /// An error reply other than the timeout's, in words.
    Erred(String),
}

/// S4's GET of `selector` (core §4.2): target `All`, consolidation
/// `Latest`; a reply on a wildcard key is discarded (R6). With the samples,
/// how the GET ended: its final reply, zenoh's `Timeout`, or another error
/// reply.
async fn state_replies(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<(Vec<MetricSample>, GetEnd), String> {
    let replies =
        crate::bus::query::state_get(session, selector, &crate::bus::query::GetOpts::new(timeout))
            .await
            .map_err(|e| format!("GET {selector}: {}", crate::one_line(&e)))?;
    let mut out = Vec::new();
    let mut end = GetEnd::Final;
    let mut replied = false;
    while let Ok(reply) = replies.recv_async().await {
        let sample = match reply.result() {
            Ok(sample) => {
                replied = true;
                sample
            }
            Err(e) => {
                let said = String::from_utf8_lossy(&e.payload().to_bytes()).into_owned();
                if said == "Timeout" {
                    end = GetEnd::Timeout;
                } else if end == GetEnd::Final {
                    end = GetEnd::Erred(said);
                }
                continue;
            }
        };
        if sample.key_expr().is_wild() {
            continue;
        }
        out.push(sample_of(sample, SampleVia::Get));
    }
    if end == GetEnd::Final && !replied {
        end = GetEnd::Empty;
    }
    Ok((out, end))
}

/// The reader's state beside the book: what resolves keys, and what it
/// watches of itself.
struct Reader<'a> {
    seeder: Seeder,
    spec: TelemetrySpec,
    catalog: Option<Catalog>,
    store: &'a BundleStore,
    offline: &'a ContractSet,
    by_resource: BTreeMap<(zenkey_model::grammar::Addr, String, String), Arc<MetricResource>>,
    /// Each key resolved, to its member's resource or to none.
    resolved: BTreeMap<String, Option<Arc<MetricResource>>>,
    refused: Arc<AtomicU64>,
    discarded: Arc<AtomicU64>,
    unresolved: u64,
    stall: Option<std::result::Result<StallWatch, String>>,
    stall_seen: StallSeen,
    owed_seed: bool,
    seed: Option<Result<SeedSeen, String>>,
    /// The owner instances the last seed that stood found.
    seeded_instances: Option<
        BTreeSet<(
            zenkey_model::grammar::Addr,
            zenkey_model::grammar::InstanceId,
        )>,
    >,
}

impl Reader<'_> {
    /// The metric resource `key` is a member of, through the lens: the
    /// namespace's presence and the revisions in hand, or, across a face,
    /// the contracts in hand alone.
    fn metric_of(&mut self, key: &str) -> Option<Arc<MetricResource>> {
        if let Some(m) = self.resolved.get(key) {
            return m.clone();
        }
        let lens = Lens::new("", self.catalog.as_ref(), self.store).offline(self.offline);
        let m = lens.resolve(key).resolved.and_then(|r| {
            let m = MetricResource::of(&r.address, &r.revision, r.index())?;
            self.by_resource
                .get(&(m.address.clone(), m.fingerprint.clone(), m.resource.clone()))
                .cloned()
        });
        if self.resolved.len() < 2 * MAX_MEMBERS {
            self.resolved.insert(key.to_owned(), m.clone());
        }
        m
    }

    fn fold_sample(&mut self, book: &mut TelemetryBook, s: MetricSample) {
        match self.metric_of(&s.key) {
            Some(m) => {
                book.sample(&m, s);
            }
            None => self.unresolved += 1,
        }
    }

    /// One delivery, folded: a loss first, when the queue refused some
    /// since, then the delivery.
    fn fold(&mut self, book: &mut TelemetryBook, d: Delivery) {
        self.losses(book);
        match d {
            Delivery::Sample(s) => self.fold_sample(book, s),
            Delivery::Token { key, held } => book.token(&key, held),
        }
    }

    /// Everything the queue holds now, folded.
    fn drain(&mut self, book: &mut TelemetryBook, rx: &mut tokio::sync::mpsc::Receiver<Delivery>) {
        while let Ok(d) = rx.try_recv() {
            self.fold(book, d);
        }
    }

    /// The deliveries the queue refused, and a stall of the receive path,
    /// as losses: each owes a seed.
    fn losses(&mut self, book: &mut TelemetryBook) {
        let refused = self.refused.swap(0, Ordering::Relaxed);
        if refused > 0 {
            book.lose(refused);
            self.owed_seed = true;
        }
        let stalled = match &self.stall {
            None | Some(Err(_)) => false,
            Some(Ok(w)) => {
                let seen = w.read();
                let bound = self.spec.stall_bound();
                let fresh = (seen.gap > bound && self.stall_seen.gap <= bound)
                    || seen.transports_closed > self.stall_seen.transports_closed;
                self.stall_seen = seen;
                fresh
            }
        };
        if stalled {
            book.lose(0);
            self.owed_seed = true;
        }
    }

    /// The window: every delivery folded as it comes, and a seed owed by a
    /// loss read again, until `deadline`, which a seed's read keeps to too
    /// ([`Reader::reseed`]): a window ends at its end, however many losses
    /// it had. What the queue holds then, and its losses, are folded last.
    async fn window(
        &mut self,
        book: &mut TelemetryBook,
        rx: &mut tokio::sync::mpsc::Receiver<Delivery>,
        deadline: tokio::time::Instant,
    ) {
        loop {
            if self.owed_seed {
                self.reseed(book, rx, Some(deadline)).await;
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                d = rx.recv() => match d {
                    Some(d) => self.fold(book, d),
                    None => break,
                },
            }
        }
        self.drain(book, rx);
        self.losses(book);
    }

    /// A liveliness read of the owners' tokens, then what the queue
    /// delivered meanwhile, as [`TelemetryBook::seed`] folds it: the token
    /// moves after the seed, in order, and the samples before it, with no
    /// presence. A loss the queue counted during the read is folded before
    /// those samples, so every pair across it has none. A read that lost
    /// deliveries itself is made again, once, and the token moves drained
    /// meanwhile are kept for the seed that stands, never dropped: a move is
    /// never lost, and a move during a seed is a move (the book's). A seed
    /// that cannot stand — a read that fails, or loses twice, or that
    /// `deadline` cuts short — says why in [`Reader::seed`], folds the moves
    /// it drained as live ones, and leaves presence where the loss left it,
    /// unobservable; one that lost twice is owed again. With no `deadline`
    /// (the first seed, before the window opens), each read is bounded by
    /// its own timeout alone.
    async fn reseed(
        &mut self,
        book: &mut TelemetryBook,
        rx: &mut tokio::sync::mpsc::Receiver<Delivery>,
        deadline: Option<tokio::time::Instant>,
    ) {
        self.owed_seed = false;
        if !book.reads_presence() {
            return;
        }
        let mut then: Vec<(String, bool)> = Vec::new();
        let failed = 'read: {
            for _ in 0..2 {
                let read = match deadline {
                    Some(d) => tokio::time::timeout_at(d, self.seeder.read()).await,
                    None => Ok(self.seeder.read().await),
                };
                let observed: Observed = match read {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => break 'read crate::one_line(&e),
                    Err(_) => {
                        break 'read "the window ended before the presence seed was read"
                            .to_owned();
                    }
                };
                let mut samples = Vec::new();
                while let Ok(d) = rx.try_recv() {
                    match d {
                        Delivery::Token { key, held } => then.push((key, held)),
                        Delivery::Sample(s) => samples.push(s),
                    }
                }
                let refused = self.refused.swap(0, Ordering::Relaxed);
                if refused > 0 {
                    book.lose(refused);
                }
                for s in samples {
                    self.fold_sample(book, s);
                }
                if refused > 0 {
                    continue;
                }
                book.seed(&observed, then.iter().map(|(k, h)| (k.as_str(), *h)));
                self.seeded_instances = Some(observed.instances());
                let c = Catalog::new(&observed);
                self.seed = Some(Ok(SeedSeen {
                    selector: observed.selector.clone(),
                    complete: observed.complete,
                    services: c.addresses().filter(|a| c.has_instance(a)).count(),
                }));
                return;
            }
            self.owed_seed = true;
            "the read lost deliveries itself, twice".to_owned()
        };
        for (k, h) in &then {
            book.token(k, *h);
        }
        self.seed = Some(Err(failed));
    }
}

/// Where a seed's liveliness read is made: the bus, or, in a test, a stand-in.
enum Seeder {
    Bus {
        session: Session,
        scope: crate::bus::presence::Scope,
        timeout: Duration,
    },
    #[cfg(test)]
    Fake(std::sync::Arc<dyn Fn() -> FakeRead + Send + Sync>),
}

#[cfg(test)]
type FakeRead =
    std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<Observed>> + Send>>;

impl Seeder {
    async fn read(&self) -> crate::Result<Observed> {
        match self {
            Seeder::Bus {
                session,
                scope,
                timeout,
            } => crate::bus::presence::read_tokens(session, scope, *timeout).await,
            #[cfg(test)]
            Seeder::Fake(f) => f().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scope narrows what is subscribed to: a selector inside a resource's
    /// members is asked as given, one wider is the resource's own, and one
    /// that misses it asks nothing.
    #[test]
    fn a_scope_asks_no_wider_than_it_must() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts");
        let (set, _) = ContractSet::load_path(&dir.join("meter.v1.toml"));
        let rev = set.iter().next().expect("meter.v1 loads");
        let addr: zenkey_model::grammar::Addr = "lab/meter".parse().unwrap();
        let ms: Vec<MetricResource> = (0..rev.contract().resources.len())
            .filter_map(|i| MetricResource::of(&addr, rev, i))
            .collect();
        let names: Vec<&str> = ms.iter().map(|m| m.resource.as_str()).collect();
        assert_eq!(
            names.len(),
            6,
            "every meter.v1 resource is a metric: {names:?}"
        );
        let drops = ms.iter().find(|m| m.resource == "stream/drops").unwrap();
        assert_eq!(drops.selector, "zk2/lab/meter/meter.v1/stream/drops");
        let all = TelemetryScope::all();
        assert_eq!(
            all.subscription(drops).as_deref(),
            Some(drops.selector.as_str())
        );
        let wide = TelemetryScope::parse("zk2/lab/**").unwrap();
        assert_eq!(
            wide.subscription(drops).as_deref(),
            Some(drops.selector.as_str())
        );
        let other = TelemetryScope::parse("zk2/lab/other/**").unwrap();
        assert_eq!(other.subscription(drops), None);
        assert!(TelemetryScope::parse("lab/**").is_err(), "base-relative");
    }

    const I1: &str = "00000000000000a1";
    const I2: &str = "00000000000000a2";

    fn instance(i: &str) -> String {
        format!("zk2/lab/meter/@zk/instance/{i}")
    }

    fn observed(instances: &[&str]) -> Observed {
        let keys: Vec<String> = instances.iter().map(|i| instance(i)).collect();
        Observed::from_keys("zk2/*/*/@zk/**", &keys, true)
    }

    /// A reader with no session: its seeds read through `seeder`.
    fn reader<'a>(
        store: &'a BundleStore,
        offline: &'a ContractSet,
        seeder: Seeder,
        refused: Arc<AtomicU64>,
    ) -> Reader<'a> {
        Reader {
            seeder,
            spec: TelemetrySpec::new(Duration::from_millis(100), Duration::from_secs(1)),
            catalog: None,
            store,
            offline,
            by_resource: BTreeMap::new(),
            resolved: BTreeMap::new(),
            refused,
            discarded: Arc::default(),
            unresolved: 0,
            stall: None,
            stall_seen: StallSeen::default(),
            owed_seed: false,
            seed: None,
            seeded_instances: None,
        }
    }

    fn held(book: &TelemetryBook) -> Vec<String> {
        let addr: zenkey_model::grammar::Addr = "lab/meter".parse().unwrap();
        book.presence()
            .and_then(|p| p.address(&addr))
            .map(|a| a.instance_tokens().map(ToString::to_string).collect())
            .unwrap_or_default()
    }

    /// Review of PR #827: a seed read again because its first read lost
    /// deliveries keeps the token moves that read drained. Here O re-mints
    /// (I2 declared, I1 withdrawn) during the first read, which the queue
    /// overflowed on, and the second read is answered as stale as the first:
    /// the moves still land, so presence holds I2 alone, never I1.
    #[tokio::test]
    async fn a_token_move_during_a_retried_seed_is_kept() {
        let (store, offline) = (BundleStore::new(Duration::ZERO), ContractSet::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let refused: Arc<AtomicU64> = Arc::default();
        let calls = Arc::new(AtomicU64::new(0));
        let seeder = {
            let (tx, refused, calls) = (tx.clone(), Arc::clone(&refused), Arc::clone(&calls));
            Seeder::Fake(Arc::new(move || {
                let (tx, refused, calls) = (tx.clone(), Arc::clone(&refused), Arc::clone(&calls));
                Box::pin(async move {
                    if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                        for (i, up) in [(I2, true), (I1, false)] {
                            tx.send(Delivery::Token {
                                key: instance(i),
                                held: up,
                            })
                            .await
                            .unwrap();
                        }
                        refused.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(observed(&[I1]))
                })
            }))
        };
        let mut r = reader(&store, &offline, seeder, Arc::clone(&refused));
        let mut book = TelemetryBook::new(Some(crate::bus::presence::Scope::all()), Instant::now());
        book.seed(&observed(&[I1]), std::iter::empty::<(&str, bool)>());
        book.lose(1);
        let far = tokio::time::Instant::now() + Duration::from_secs(30);
        r.reseed(&mut book, &mut rx, Some(far)).await;
        assert_eq!(calls.load(Ordering::Relaxed), 2, "read again after a loss");
        assert!(matches!(r.seed, Some(Ok(_))), "{:?}", r.seed);
        assert_eq!(held(&book), [I2], "the re-mint was not lost");
        assert_eq!(book.lost(), 2);
        drop(tx);
    }

    /// Review of PR #827: a window ends at its deadline however many losses
    /// it has. Every seed's read here takes a second and loses deliveries,
    /// so each loss owes another seed; the window of 600 ms still ends at
    /// 600 ms, and the seed it cut short says so.
    #[tokio::test]
    async fn a_window_with_repeated_losses_ends_at_its_deadline() {
        let (store, offline) = (BundleStore::new(Duration::ZERO), ContractSet::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Delivery>(16);
        let refused: Arc<AtomicU64> = Arc::default();
        let seeder = {
            let refused = Arc::clone(&refused);
            Seeder::Fake(Arc::new(move || {
                let refused = Arc::clone(&refused);
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    refused.fetch_add(1, Ordering::Relaxed);
                    Ok(observed(&[]))
                })
            }))
        };
        let mut r = reader(&store, &offline, seeder, Arc::clone(&refused));
        let mut book = TelemetryBook::new(Some(crate::bus::presence::Scope::all()), Instant::now());
        r.owed_seed = true;
        let started = tokio::time::Instant::now();
        r.window(&mut book, &mut rx, started + Duration::from_millis(600))
            .await;
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(900),
            "the window overran: {took:?}"
        );
        assert!(
            matches!(&r.seed, Some(Err(why)) if why.contains("window ended")),
            "{:?}",
            r.seed
        );
        assert!(
            book.presence_basis() != Some(crate::model::presence_live::PresenceBasis::Complete),
            "a seed cut short restores nothing"
        );
        drop(tx);
    }

    /// A peer of nobody: what answers its GETs is what it holds itself.
    async fn alone() -> zenoh::Session {
        let mut c = zenoh::Config::default();
        c.insert_json5("mode", "\"peer\"").unwrap();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("scouting/gossip/enabled", "false").unwrap();
        c.insert_json5("listen/endpoints", "[]").unwrap();
        zenoh::open(c).await.expect("a peer of nobody")
    }

    /// meter.v1's metric resources at lab/meter, in an observation.
    fn meter_obs() -> (TelemetryObservation, TelemetryScope) {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts");
        let (set, _) = ContractSet::load_path(&dir.join("meter.v1.toml"));
        let rev = set.iter().next().expect("meter.v1 loads");
        let addr: zenkey_model::grammar::Addr = "lab/meter".parse().unwrap();
        let scope = TelemetryScope::of(&addr);
        let resources = (0..rev.contract().resources.len())
            .filter_map(|i| MetricResource::of(&addr, rev, i))
            .map(Arc::new)
            .collect();
        let obs = TelemetryObservation {
            namespace: String::new(),
            spec: TelemetrySpec::new(Duration::from_millis(300), Duration::from_secs(1)),
            scope: scope.clone(),
            resources,
            seed: None,
            book: TelemetryBook::new(None, Instant::now()),
            asked: vec![],
            listened: Duration::ZERO,
            unresolved: 0,
            skipped: vec![],
            unobservable: None,
            opening: None,
        };
        (obs, scope)
    }

    /// Review of PR #835 (#819 TG): the state GETs a window opens on across
    /// a face are counted for what they are. A GET whose final came with no
    /// reply proves nothing (`empty`); one that drew an error reply failed,
    /// and is named in the count and in `skipped`; one the owner answered
    /// is the proof.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_opening_counts_its_state_gets_and_names_each_unanswered() {
        use zenoh::Wait as _;
        const TOTAL: &str = "zk2/lab/meter/meter.v1/state/total";
        let s = alone().await;
        let (store, offline) = (BundleStore::new(Duration::ZERO), ContractSet::new());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Delivery>(16);
        let seeder = Seeder::Fake(Arc::new(|| Box::pin(async { Ok(observed(&[])) })));
        let mut r = reader(&store, &offline, seeder, Arc::default());
        let t = Duration::from_millis(300);
        let (mut obs, scope) = meter_obs();
        let trips = state_gets(&s, &scope, &mut obs, &mut r, &mut rx, t, None).await;
        assert_eq!(
            (trips.asked, trips.answered, trips.empty),
            (1, 0, 1),
            "{trips:?}"
        );
        assert!(trips.unanswered[0].contains("no reply"), "{trips:?}");

        let q = s
            .declare_queryable(TOTAL)
            .callback(|q| {
                let _ = q.reply_err("refused").wait();
            })
            .await
            .unwrap();
        let (mut obs, _) = meter_obs();
        let trips = state_gets(&s, &scope, &mut obs, &mut r, &mut rx, t, None).await;
        assert_eq!((trips.answered, trips.errored), (0, 1), "{trips:?}");
        assert!(
            trips.unanswered[0].contains(&format!("GET {TOTAL}: an error reply, refused")),
            "{trips:?}"
        );
        assert_eq!(obs.skipped, trips.unanswered, "named in skipped too");
        drop(q);

        let q = s
            .declare_queryable(TOTAL)
            .callback(|q| {
                let _ = q.reply(TOTAL, "12").wait();
            })
            .await
            .unwrap();
        let (mut obs, _) = meter_obs();
        let trips = state_gets(&s, &scope, &mut obs, &mut r, &mut rx, t, None).await;
        assert_eq!(
            (trips.asked, trips.answered, trips.unanswered.len()),
            (1, 1, 0),
            "{trips:?}"
        );
        drop(q);
        drop(tx);
    }

    /// Review of PR #835 (#819 TG): with no face, a window asks the owner
    /// instances its seed found, of an owner in scope. A seed that did not
    /// stand asks none, never the first read's, and nothing to ask is no
    /// owner, never "every owner answered". An instance that answers no
    /// descriptor GET is counted silent, and named.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nothing_to_ask_is_no_owner_and_a_silent_owner_is_counted() {
        let (obs, _) = meter_obs();
        let addr: zenkey_model::grammar::Addr = "lab/meter".parse().unwrap();
        let other: zenkey_model::grammar::Addr = "lab/other".parse().unwrap();
        assert_eq!(
            to_ask(None, &obs.resources),
            Err(Opening::NoOwner { seeded: false })
        );
        let elsewhere = BTreeSet::from([(other, I1.parse().unwrap())]);
        assert_eq!(
            to_ask(Some(&elsewhere), &obs.resources),
            Err(Opening::NoOwner { seeded: true })
        );
        let here = BTreeSet::from([(addr.clone(), I1.parse().unwrap())]);
        let wanted = to_ask(Some(&here), &obs.resources).expect("one to ask");
        assert_eq!(wanted.len(), 1);
        let s = alone().await;
        let trips = owners_answer(&s, wanted, Duration::from_millis(300)).await;
        assert_eq!(
            (trips.asked, trips.answered, trips.silent),
            (1, 0, 1),
            "{trips:?}"
        );
        assert!(
            trips.unanswered[0].starts_with(&format!("lab/meter {I1}: silent")),
            "{trips:?}"
        );
    }
}
