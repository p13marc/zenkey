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
//!    instance and member tokens, then a subscription to each metric
//!    resource's members, then a watch of this reader's own receive path
//!    ([`crate::bus::stall`], core §2.7). Every subscription calls back into
//!    **one** bounded queue, so samples and token moves are folded in the one
//!    order they were received, and what the queue refuses is counted: a
//!    loss, which leaves every pair across it without presence (§2.9);
//! 3. a liveliness read of the owners' tokens, the seed presence stands on
//!    ([`TelemetryBook::seed`]), read again after every loss;
//! 4. S4's GET of each state metric resource (core §4.2), its replies merged
//!    with the live puts by the owner's stamp in the book (§2.10, 0.2);
//! 5. the window, every delivery folded as it comes.
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
    /// How long the subscriptions listen.
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
}

impl TelemetrySpec {
    /// A window, zenoh's lease and `wait_before_close`, no face.
    pub fn new(timeout: Duration, window: Duration) -> TelemetrySpec {
        TelemetrySpec {
            timeout,
            window,
            lease: zenkey_model::budget::DEFAULT_LEASE,
            wait_before_close: zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE,
            across_face: false,
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
    /// How long the subscriptions listened.
    pub listened: Duration,
    /// Samples on a key that resolved to no metric member, or a wildcard
    /// key (R6).
    pub unresolved: u64,
    /// What in scope could not be read, one line each.
    pub skipped: Vec<String>,
    /// Set when the reading could not be made at all, with why.
    pub unobservable: Option<String>,
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
pub async fn run_telemetry(
    session: &Session,
    namespace: &str,
    scope: TelemetryScope,
    store: &BundleStore,
    offline: &ContractSet,
    spec: TelemetrySpec,
) -> crate::report::TelemetryReport {
    let obs = observe(session, namespace, scope, store, offline, spec).await;
    crate::judge::telemetry::judge(&obs)
}

/// The reader of the module doc, over `spec.window`.
pub async fn observe(
    session: &Session,
    namespace: &str,
    scope: TelemetryScope,
    store: &BundleStore,
    offline: &ContractSet,
    spec: TelemetrySpec,
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
    let mut reader = Reader {
        session: session.clone(),
        spec,
        presence_scope,
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
    let listening = Instant::now();
    if !spec.across_face {
        let [instances, _alive, members] = reader.presence_scope.token_selectors();
        for sel in [instances, members] {
            let q = queue.clone();
            let declared = crate::bus::teardown::declared(
                "telemetry's presence tokens",
                &sel,
                session
                    .liveliness()
                    .declare_subscriber(sel.clone())
                    .callback(move |s: Sample| {
                        q.send(Delivery::Token {
                            key: s.key_expr().as_str().to_owned(),
                            held: s.kind() == SampleKind::Put,
                        });
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
            obs.asked.push(sel);
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
                    if s.key_expr().as_str().contains('*') {
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
    // 3. The seed presence stands on.
    if !spec.across_face {
        reader.reseed(&mut obs.book, &mut rx).await;
    }
    // 4. S4's GET of each state metric resource, after what the queue holds.
    for m in obs.resources.iter().filter(|m| m.kind == Kind::State) {
        let Some(sel) = scope.subscription(m) else {
            continue;
        };
        let replies = state_replies(session, &sel, t).await;
        reader.drain(&mut obs.book, &mut rx);
        match replies {
            Ok(rs) => {
                for s in rs {
                    reader.fold_sample(&mut obs.book, s);
                }
            }
            Err(e) => obs.skipped.push(e),
        }
        obs.asked.push(sel);
    }
    // 5. The window.
    let deadline = tokio::time::Instant::from_std(listening + spec.window);
    loop {
        if reader.owed_seed {
            reader.reseed(&mut obs.book, &mut rx).await;
        }
        tokio::select! {
            () = tokio::time::sleep_until(deadline) => break,
            d = rx.recv() => match d {
                Some(d) => reader.fold(&mut obs.book, d),
                None => break,
            },
        }
    }
    reader.drain(&mut obs.book, &mut rx);
    reader.losses(&mut obs.book);
    obs.listened = listening.elapsed();
    drop(held);
    obs.seed = reader.seed.take();
    obs.unresolved = reader.unresolved + reader.discarded.load(Ordering::Relaxed);
    let mut seen = BTreeSet::new();
    obs.asked.retain(|s| seen.insert(s.clone()));
    obs
}

/// S4's GET of `selector` (core §4.2): target `All`, consolidation
/// `Latest`; a reply on a wildcard key is discarded (R6).
async fn state_replies(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<Vec<MetricSample>, String> {
    let replies =
        crate::bus::query::state_get(session, selector, &crate::bus::query::GetOpts::new(timeout))
            .await
            .map_err(|e| format!("GET {selector}: {}", crate::one_line(&e)))?;
    let mut out = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            continue;
        };
        if sample.key_expr().as_str().contains('*') {
            continue;
        }
        out.push(sample_of(sample, SampleVia::Get));
    }
    Ok(out)
}

/// The reader's state beside the book: what resolves keys, and what it
/// watches of itself.
struct Reader<'a> {
    session: Session,
    spec: TelemetrySpec,
    presence_scope: crate::bus::presence::Scope,
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

    /// A liveliness read of the owners' tokens, then what the queue
    /// delivered meanwhile: its token moves folded after the seed, in order
    /// ([`TelemetryBook::seed`]), its samples folded before it, with no
    /// presence. A seed that lost deliveries itself is read again, once.
    async fn reseed(
        &mut self,
        book: &mut TelemetryBook,
        rx: &mut tokio::sync::mpsc::Receiver<Delivery>,
    ) {
        self.owed_seed = false;
        for _ in 0..2 {
            let read = crate::bus::presence::read_tokens(
                &self.session,
                &self.presence_scope,
                self.spec.timeout,
            )
            .await;
            let observed: Observed = match read {
                Ok(o) => o,
                Err(e) => {
                    self.seed = Some(Err(crate::one_line(&e)));
                    return;
                }
            };
            let mut then = Vec::new();
            while let Ok(d) = rx.try_recv() {
                match d {
                    Delivery::Token { key, held } => then.push((key, held)),
                    Delivery::Sample(s) => self.fold_sample(book, s),
                }
            }
            let refused = self.refused.swap(0, Ordering::Relaxed);
            if refused > 0 {
                book.lose(refused);
                continue;
            }
            book.seed(&observed, then.iter().map(|(k, h)| (k.as_str(), *h)));
            let c = Catalog::new(&observed);
            self.seed = Some(Ok(SeedSeen {
                selector: observed.selector.clone(),
                complete: observed.complete,
                services: c.addresses().filter(|a| c.has_instance(a)).count(),
            }));
            return;
        }
        self.owed_seed = true;
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
}
