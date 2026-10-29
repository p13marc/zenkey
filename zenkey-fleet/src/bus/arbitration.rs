//! `arbitration.v1` (#909, BC), the reads: each consumer's arbitrated role
//! a scope reaches (`spec/profiles/arbitration/v1.md` §2.14), and the
//! tool's own view of it over a short listen, through a session in the
//! deployment's namespace.
//!
//! **Why the fleet runs its own view.** The runtime's
//! (`zenkey::consumer::Consumer::arbitrate`) is a consumer's: it takes a
//! watch session in the deployment's namespace and tells an application.
//! A tool holds its liveliness subscribers on a [`crate::WatchSession`] in
//! no namespace on the wire keys (#875), which that does not take, and
//! reports a view. So the view is [`crate::model::arbitration::ViewBook`],
//! the model's `Arbiter` over the tool's own events, fed here as the
//! runtime feeds its own: every decision stays the model's.
//!
//! What is read, in order:
//! 1. presence and every instance's descriptor (core §8.1, §3.3) in scope:
//!    the consumer named, or every one; and the revisions they name,
//!    retrieved (§8.4) or held offline. Each descriptor's `requires`
//!    entries are the subjects ([`crate::model::arbitration::subjects`]);
//! 2. presence and descriptors of every binding entry's services the first
//!    read did not cover, and the revisions they name: what the tool holds
//!    of the required interface is every revision of it in hand, offline or
//!    retrieved from the bound providers' descriptors (§2.14). With
//!    `--role` and no consumer's descriptor listing it, each contract in
//!    hand declaring the role is read with no descriptor (F-272);
//! 3. for each role whose declaration is arbitrated and whose binding is
//!    not empty, **the view**, all roles at once, each over `listen`:
//!    - one liveliness subscriber per entry on the watch session, on
//!      `zk2/<entry>/@zk/alive/<iface>/**` (`*` kept, §2.7), its round trip
//!      made after declaring them; without a watch session, the deadline
//!      alone, said so;
//!    - one subscriber per entry on the deployment's session, each sample's
//!      instant read **in its callback**, a sample two entries' selectors
//!      cover handed over once, by the first (§2.4), as the runtime does;
//!    - a governed interface's incumbents read once after the round trip
//!      (§2.5, outcome 6), and the descriptor of each holder whose
//!      `meta.zid` is not held;
//!    - every callback into **one** bounded queue, so samples and token
//!      moves are folded in the one order received, what the queue refuses
//!      a loss, and a watch of this reader's own receive path
//!      ([`crate::bus::stall`], core §2.7) a loss too (§2.14's `lost`);
//!    - a withdrawal leaving an address none known held owes one read
//!      [`REREAD`] later, one per burst, on the deployment's session (§2.7);
//!      a move of the watch session's transport, or a withdrawal heard
//!      while it holds no link (F-268), keeps what the watch hears until
//!      the round trip asked [`AFTER_OPEN`] after it stands (§2.8; core
//!      §8.1, 0.56), asked again until one does; and a one-shot timer at
//!      the active provider's due instant (§2.9).
//!
//! Nothing is judged here: [`crate::judge::arbitration`] answers §5.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey_model::arbitration::{Arbitrated, Binding};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Contract;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId};
use zenoh::Session;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::sample::{Sample, SampleKind};

use crate::bus::contracts::BundleStore;
use crate::bus::presence::Scope;
use crate::bus::stall::{StallSeen, StallWatch};
use crate::model::arbitration::{
    Declaring, RoleRead, RoleSubject, ViewBook, held_by_address, in_scope, offline_subject,
    read_role, served, subjects,
};
use crate::model::catalog::{Catalog, ContractSet, ContractState, Contracts};
use crate::report::ArbitrationReport;

/// The queue a view's subscriptions call back into.
pub const QUEUE: usize = 1024;

/// How long after the first withdrawal of a burst a view reads the
/// addresses it left none known held (§2.7; the reference's 0.1 s).
pub const REREAD: Duration = Duration::from_millis(100);

/// How long after a move of the watch session the round trip that ends it
/// is asked (§2.8; core §8.1, 0.56): zenoh 1.10.1 tells an open before it
/// declares the watch's interests again, and a round trip asked in between
/// stands with nothing told.
pub const AFTER_OPEN: Duration = Duration::from_millis(100);

/// How long a view listens by default: several deadlines of the
/// scenarios' 300 ms.
pub const DEFAULT_LISTEN: Duration = Duration::from_secs(2);

/// What a reading asks.
#[derive(Debug, Clone)]
pub struct ArbitrationSpec {
    /// Each read's timeout: presence, a descriptor, a retrieval, a round
    /// trip.
    pub timeout: Duration,
    /// How long each view listens; zero: no view (the selection not
    /// asked, `not_listened`).
    pub listen: Duration,
    /// Only this role; `None`: every role carrying a policy, or whose
    /// declaring contract is not in hand.
    pub role: Option<String>,
    /// The deployment's link lease (core §2.7): with `wait_before_close`,
    /// the longest this reader may stall before it counts a loss.
    pub lease: Duration,
    /// The routers' `wait_before_close` (core §2.7, 0.27).
    pub wait_before_close: Duration,
}

impl ArbitrationSpec {
    /// Every role, [`DEFAULT_LISTEN`], zenoh's lease and
    /// `wait_before_close`.
    pub fn new(timeout: Duration) -> ArbitrationSpec {
        ArbitrationSpec {
            timeout,
            listen: DEFAULT_LISTEN,
            role: None,
            lease: zenkey_model::budget::DEFAULT_LEASE,
            wait_before_close: zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE,
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

/// What a view listened with, beside its book.
#[derive(Debug, Clone)]
pub struct Listened {
    /// When it stopped listening: the instant the view is read at.
    pub end: Instant,
    /// Whether a watch session followed the providers' interface tokens.
    pub watching: bool,
    /// The reads again it made (§2.7).
    pub reads: u64,
    /// The round trips it asked on the watch session (§2.8).
    pub round_trips: u64,
    /// The selectors it put to the bus.
    pub asked: Vec<String>,
    /// What it could not do, one line each.
    pub notes: Vec<String>,
}

/// One role, as read.
#[derive(Debug, Clone)]
pub struct RoleObserved {
    pub subject: RoleSubject,
    pub read: RoleRead,
    /// The tool's view, when it listened.
    pub view: Option<(ViewBook, Listened)>,
}

/// One reading of a scope's arbitrated roles, as values: what
/// [`crate::judge::arbitration::judge`] decides from.
#[derive(Debug, Clone)]
pub struct ArbitrationObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub target: Option<Addr>,
    pub role: Option<String>,
    pub listen: Duration,
    /// The selectors put to the bus, base-relative.
    pub asked: Vec<String>,
    pub roles: Vec<RoleObserved>,
    /// What in scope could not be read, one line each.
    pub skipped: Vec<String>,
    /// Set when the reading could not be made at all, with why.
    pub unobservable: Option<String>,
}

/// One reading: [`observe`], then [`crate::judge::arbitration::judge`].
#[allow(clippy::too_many_arguments)]
pub async fn run_arbitration(
    session: &Session,
    watch: Option<&crate::WatchSession>,
    namespace: &str,
    target: Option<Addr>,
    store: &BundleStore,
    offline: &ContractSet,
    spec: ArbitrationSpec,
) -> ArbitrationReport {
    let obs = observe(session, watch, namespace, target, store, offline, spec).await;
    crate::judge::arbitration::judge(&obs)
}

/// A revision in the store, or why not.
fn state_of(
    store: &BundleStore,
    iface: &IfaceId,
    fp: &Fingerprint,
) -> Result<Arc<crate::model::catalog::Revision>, String> {
    match store.state(iface, fp) {
        Some(ContractState::Held(r)) => Ok(r),
        Some(ContractState::Unavailable { .. }) => {
            Err(format!("{iface} {fp}: no holder served its bundle"))
        }
        Some(ContractState::Unreadable { reason }) => Err(format!("{iface} {fp}: {reason}")),
        None => Err(format!("{iface} {fp}: not retrieved")),
    }
}

/// The reads of the module doc, as values.
#[allow(clippy::too_many_arguments)]
pub async fn observe(
    session: &Session,
    watch: Option<&crate::WatchSession>,
    namespace: &str,
    target: Option<Addr>,
    store: &BundleStore,
    offline: &ContractSet,
    spec: ArbitrationSpec,
) -> ArbitrationObservation {
    let t = spec.timeout;
    let mut obs = ArbitrationObservation {
        namespace: namespace.to_owned(),
        target: target.clone(),
        role: spec.role.clone(),
        listen: spec.listen,
        asked: Vec::new(),
        roles: Vec::new(),
        skipped: Vec::new(),
        unobservable: None,
    };
    // 1. The consumers in scope.
    let scope = target.as_ref().map_or_else(Scope::all, Scope::service);
    obs.asked.push(scope.selector());
    let first = match crate::bus::presence::observe(session, &scope, t).await {
        Ok(o) => o,
        Err(e) => {
            obs.unobservable = Some(format!(
                "the presence read could not be made: {}",
                crate::one_line(&e)
            ));
            return obs;
        }
    };
    let _ = store
        .fetch_all(session, &Catalog::new(&first).wanted())
        .await;
    if !first.complete {
        obs.skipped.push(format!(
            "the presence read of {} is possibly incomplete: a consumer it did not show is not \
             read",
            first.selector
        ));
    }
    let found = subjects(served(&first), spec.role.as_deref());
    // 2. The bound providers' descriptors, where the first read did not
    // reach them.
    let mut scopes: BTreeSet<String> = BTreeSet::new();
    for s in &found {
        for e in s.bindings.iter().flatten() {
            match Scope::entry(e) {
                Ok(sc) if !scope.covers(&sc) => {
                    scopes.insert(e.clone());
                }
                Ok(_) => {}
                Err(why) => obs.skipped.push(format!(
                    "{} {}: entry {e:?} is not read: {}",
                    s.consumer
                        .as_ref()
                        .map_or_else(String::new, ToString::to_string),
                    s.role,
                    crate::one_line(&why)
                )),
            }
        }
    }
    for e in &scopes {
        let Ok(sc) = Scope::entry(e) else { continue };
        obs.asked.push(sc.selector());
        match crate::bus::presence::observe(session, &sc, t).await {
            Ok(o) => {
                let _ = store.fetch_all(session, &Catalog::new(&o).wanted()).await;
            }
            Err(why) => obs.skipped.push(format!(
                "the providers of {e}: their presence read could not be made: {}",
                crate::one_line(&why)
            )),
        }
    }
    // The roles, read.
    let required_of = |iface: &IfaceId| -> Vec<Arc<crate::model::catalog::Revision>> {
        let mut held = store.held_of(iface);
        held.extend(offline.of_iface(iface).cloned());
        held
    };
    let read = |s: RoleSubject, declaring: Result<Arc<crate::model::catalog::Revision>, String>| {
        let required = required_of(&s.required);
        let pairs: Vec<(Fingerprint, &Contract)> = required
            .iter()
            .map(|r| (r.fingerprint().clone(), r.contract()))
            .collect();
        let rr = match &declaring {
            Ok(r) => read_role(&s, Declaring::Held(r.contract()), &pairs),
            Err(why) => read_role(&s, Declaring::Missing(why), &pairs),
        };
        RoleObserved {
            subject: s,
            read: rr,
            view: None,
        }
    };
    for s in found {
        let declaring = match (&s.declared_by, &s.declaring) {
            (None, _) => Err("a manifest's role".to_owned()),
            (Some(by), Some(fp)) => state_of(store, by, fp),
            (Some(by), None) => Err(format!(
                "the descriptor lists no revision of {by}, which declares the role"
            )),
        };
        let r = read(s, declaring);
        if spec.role.is_some() || in_scope(&r.read) {
            obs.roles.push(r);
        }
    }
    // F-272: a role named, no consumer's descriptor listing it — each
    // contract in hand that declares it, read with no descriptor.
    if let Some(role) = &spec.role
        && obs.roles.is_empty()
    {
        for rev in offline.iter() {
            if let Some(s) = offline_subject(rev.contract(), rev.fingerprint(), role) {
                let r = read(s, Ok(Arc::clone(rev)));
                obs.roles.push(r);
            }
        }
    }
    // 3. The views, all at once.
    if spec.listen > Duration::ZERO {
        let sp = &spec;
        let views = obs.roles.iter().enumerate().filter_map(|(i, r)| {
            let declared = r.read.arbitrated.as_ref()?;
            let binding = r.read.reading.binding.as_ref()?;
            if binding.is_empty() {
                return None;
            }
            let required = required_of(&r.subject.required)
                .into_iter()
                .find(|c| r.read.revisions.contains(c.fingerprint()))?;
            Some(async move {
                (
                    i,
                    listen(
                        session,
                        watch,
                        &r.subject,
                        declared,
                        binding.clone(),
                        required.shared_contract(),
                        r.read.governed,
                        sp,
                    )
                    .await,
                )
            })
        });
        let done = futures_util::future::join_all(views).await;
        for (i, v) in done {
            match v {
                Ok((book, l)) => {
                    obs.asked.extend(l.asked.iter().cloned());
                    obs.roles[i].view = Some((book, l));
                }
                Err(why) => obs.roles[i]
                    .read
                    .notes
                    .push(format!("the tool's view did not listen: {why}")),
            }
        }
    }
    let mut seen = BTreeSet::new();
    obs.asked.retain(|s| seen.insert(s.clone()));
    obs
}

/// What a subscription called back with, at the instant its callback read.
enum Delivery {
    Sample {
        key: String,
        delete: bool,
        stamp: Option<u128>,
        at: Instant,
    },
    Token {
        key: String,
        held: bool,
        at: Instant,
    },
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

/// What a read made beside the view answered.
enum Answered {
    /// A read again of one address (§2.7), or a governed interface's read
    /// at start: its address, completeness and holders.
    Read {
        address: Addr,
        complete: bool,
        held: BTreeSet<InstanceId>,
        at: Instant,
    },
    /// A read again made, whatever it showed.
    Made,
    /// A holder's `meta.zid`, or none read.
    Described(Addr, InstanceId, Option<u128>),
}

/// One role's view over `spec.listen` (the module doc's step 3).
#[allow(clippy::too_many_arguments)]
async fn listen(
    session: &Session,
    watch: Option<&crate::WatchSession>,
    s: &RoleSubject,
    declared: &Arbitrated,
    binding: Binding,
    required: Arc<Contract>,
    governed: bool,
    spec: &ArbitrationSpec,
) -> Result<(ViewBook, Listened), String> {
    let t = spec.timeout;
    let iface = declared.iface.clone();
    let entries: Vec<String> = binding.entries().iter().map(ToString::to_string).collect();
    let refs: Vec<&str> = entries.iter().map(String::as_str).collect();
    let selectors: Vec<OwnedKeyExpr> =
        zenkey::consumer::Consumer::for_tool(session, required, &refs, &s.params)
            .and_then(|c| c.selectors(&declared.resource()))
            .map_err(|e| e.to_string())?;
    if let Some(w) = watch
        && w.zid() == session.zid()
    {
        return Err(
            "the watch session is the reader's own, which makes every read again (core §8.1, \
             0.37)"
                .to_owned(),
        );
    }
    let mut l = Listened {
        end: Instant::now(),
        watching: watch.is_some(),
        reads: 0,
        round_trips: 0,
        asked: Vec::new(),
        notes: Vec::new(),
    };
    let refused: Arc<AtomicU64> = Arc::default();
    let (tx, mut rx) = tokio::sync::mpsc::channel(QUEUE);
    let queue = Queue {
        tx,
        refused: Arc::clone(&refused),
    };
    let mut held = Vec::new();
    let origin = Instant::now();
    let mut book = ViewBook::new(declared, binding, s.params.clone(), governed, origin);
    // §2.7: the watch, then its round trip.
    let mut barrier = None;
    if let Some(w) = watch {
        let mut first = None;
        for e in &entries {
            let sel = format!("zk2/{e}/@zk/alive/{iface}/**");
            let q = queue.clone();
            let sub = w
                .subscribe("arbitration's watch", &sel, move |h| {
                    q.send(Delivery::Token {
                        key: h.key,
                        held: h.held,
                        at: Instant::now(),
                    });
                })
                .await
                .map_err(|e| crate::one_line(&e))?;
            held.push(sub);
            l.asked.push(sel.clone());
            first.get_or_insert(sel);
        }
        if let Some(sel) = first {
            let b = w.barrier(&sel, t).await.map_err(|e| crate::one_line(&e))?;
            l.round_trips += 1;
            if !b.establish().await {
                l.notes.push(
                    "the watch session's round trip did not stand at the start: a withdrawal \
                     heard before it is still applied (§2.8: at start there is no move)"
                        .to_owned(),
                );
            }
            barrier = Some(b);
        }
    } else {
        l.notes.push(
            "no watch session: no withdrawal is heard, and every provider is judged by its \
             deadline alone (§2.7, §2.12)"
                .to_owned(),
        );
    }
    // §2.5, §2.9: a subscriber per entry, each sample's instant read in its
    // callback, a sample two selectors cover the first's (§2.4).
    let all = Arc::new(selectors);
    for (at, ke) in all.iter().enumerate() {
        let (q, all) = (queue.clone(), Arc::clone(&all));
        let sub = crate::bus::teardown::declared(
            "arbitration's stream",
            ke,
            session
                .declare_subscriber(ke.clone())
                .callback(move |sample: Sample| {
                    let now = Instant::now();
                    if all
                        .iter()
                        .position(|k| k.intersects(sample.key_expr()))
                        .is_some_and(|first| first != at)
                    {
                        return;
                    }
                    q.send(Delivery::Sample {
                        key: sample.key_expr().as_str().to_owned(),
                        delete: sample.kind() == SampleKind::Delete,
                        stamp: sample.timestamp().and_then(zenkey::timing::stamp_id),
                        at: now,
                    });
                }),
        )
        .await
        .map_err(|e| crate::one_line(&e))?;
        held.push(sub);
        l.asked.push(ke.to_string());
    }
    drop(queue);
    let stall = StallWatch::start(session).await;
    if let Err(why) = &stall {
        l.notes.push(format!(
            "its own receive path is not watched ({why}): a stall would not be known lost"
        ));
    }
    let (answers_tx, mut answers) = tokio::sync::mpsc::unbounded_channel::<Answered>();
    let mut v = Follow {
        session: session.clone(),
        watch: watch.cloned(),
        iface,
        timeout: t,
        barrier: barrier.clone(),
        refused,
        stall,
        stall_seen: StallSeen::default(),
        stall_bound: spec.stall_bound(),
        answers: answers_tx,
        asked_zids: BTreeSet::new(),
        burst: BTreeSet::new(),
        awaiting_event: None,
    };
    // §2.5's outcome 6: a governed interface's incumbents, read once after
    // the round trip, with their descriptors.
    if governed {
        for e in &entries {
            let Ok(sc) = Scope::entry(e) else { continue };
            match crate::bus::presence::observe(session, &sc, t).await {
                Ok(o) => {
                    let now = Instant::now();
                    for (address, ids) in held_by_address(o.tokens.iter(), &v.iface, None) {
                        book.read(now, address, o.complete, ids);
                    }
                    for (a, i, d) in served(&o) {
                        if let Some(z) = d
                            .meta
                            .get("zid")
                            .and_then(|z| z.as_str())
                            .and_then(zenkey_model::timing::zid_value)
                        {
                            book.described(a.clone(), i.clone(), z);
                        }
                    }
                }
                Err(why) => l.notes.push(format!(
                    "the incumbents of {e} were not read at the start: {}",
                    crate::one_line(&why)
                )),
            }
        }
        v.describe(&book);
    }
    // The window.
    let end = tokio::time::Instant::from_std(origin + spec.listen);
    let mut moves_seen = barrier.as_ref().map_or(0, zenkey::presence::Barrier::moves);
    let mut trip: Option<Trip> = None;
    let mut burst_due: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
    let mut open = true;
    loop {
        if tokio::time::Instant::now() >= end {
            break;
        }
        let due = book
            .due()
            .map(|d| tokio::time::Instant::from_std(d + Duration::from_micros(50)));
        let moved = async {
            match &barrier {
                Some(b) => b.moved_since(moves_seen).await,
                None => std::future::pending::<()>().await,
            }
        };
        let tripped = async {
            match trip.as_mut() {
                Some(t) => t.await,
                None => std::future::pending::<bool>().await,
            }
        };
        let read_due = async {
            match burst_due.as_mut() {
                Some(s) => s.await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            () = tokio::time::sleep_until(end) => break,
            () = until(due) => {
                v.drain(&mut book, &mut rx, &mut burst_due).await;
                book.tick(Instant::now());
            }
            () = moved => {
                v.drain(&mut book, &mut rx, &mut burst_due).await;
                moves_seen = barrier.as_ref().map_or(moves_seen, zenkey::presence::Barrier::moves);
                v.awaiting_event = None;
                book.moved(Instant::now());
                trip = Some(v.ask_trip(AFTER_OPEN));
                l.round_trips += 1;
            }
            stood = tripped => {
                trip = None;
                let established = barrier
                    .as_ref()
                    .is_some_and(zenkey::presence::Barrier::established);
                if stood && established && !v.awaiting() {
                    // What the watch heard before the round trip's reply is
                    // the move's: taken before it ends (§2.8; core §8.1,
                    // 0.44, 0.56).
                    v.drain(&mut book, &mut rx, &mut burst_due).await;
                    let again = book.stood(Instant::now());
                    v.owe(again, &mut burst_due);
                    v.describe(&book);
                } else if !v.cut_off().await {
                    trip = Some(v.ask_trip(t / 2));
                    l.round_trips += 1;
                }
            }
            () = read_due => {
                burst_due = None;
                for a in std::mem::take(&mut v.burst) {
                    v.read_again(a);
                }
            }
            a = answers.recv() => {
                v.drain(&mut book, &mut rx, &mut burst_due).await;
                match a {
                    Some(Answered::Read { address, complete, held: ids, at }) => {
                        let again = book.read(at, address, complete, ids);
                        v.owe(again, &mut burst_due);
                        v.describe(&book);
                    }
                    Some(Answered::Made) => l.reads += 1,
                    Some(Answered::Described(a, i, z)) => match z {
                        Some(z) => book.described(a, i, z),
                        None => {
                            v.asked_zids.remove(&(a, i));
                        }
                    },
                    None => {}
                }
            }
            d = recv(&mut rx, open) => match d {
                Some(d) => v.fold(&mut book, d, &mut burst_due).await,
                None => open = false,
            },
        }
    }
    v.drain(&mut book, &mut rx, &mut burst_due).await;
    l.end = Instant::now();
    v.losses(&mut book);
    drop(held);
    Ok((book, l))
}

type Trip = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

/// A view's follower beside its book: its sessions, what it watches of
/// itself, and the reads it has in flight.
struct Follow {
    session: Session,
    watch: Option<crate::WatchSession>,
    iface: IfaceId,
    timeout: Duration,
    barrier: Option<zenkey::presence::Barrier>,
    refused: Arc<AtomicU64>,
    stall: std::result::Result<StallWatch, String>,
    stall_seen: StallSeen,
    stall_bound: Duration,
    answers: tokio::sync::mpsc::UnboundedSender<Answered>,
    /// Holders whose descriptor was asked for.
    asked_zids: BTreeSet<(Addr, InstanceId)>,
    /// Addresses owed a read again, in the burst being gathered.
    burst: BTreeSet<Addr>,
    /// A move begun at a withdrawal whose transport event has not come
    /// yet (F-268): the barrier's count of transport events when it began.
    awaiting_event: Option<u64>,
}

impl Follow {
    /// Everything the queue holds now, folded, its losses first.
    async fn drain(
        &mut self,
        book: &mut ViewBook,
        rx: &mut tokio::sync::mpsc::Receiver<Delivery>,
        burst_due: &mut Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
    ) {
        while let Ok(d) = rx.try_recv() {
            self.fold(book, d, burst_due).await;
        }
        self.losses(book);
    }

    async fn fold(
        &mut self,
        book: &mut ViewBook,
        d: Delivery,
        burst_due: &mut Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
    ) {
        self.losses(book);
        match d {
            Delivery::Sample {
                key,
                delete,
                stamp,
                at,
            } => {
                book.sample(at, &key, delete, stamp);
            }
            Delivery::Token { key, held, at } => {
                // F-268: a withdrawal heard while the watch session holds
                // no link begins a move, before it is applied.
                if !held && !book.moving() && self.cut_off().await {
                    self.awaiting_event = Some(
                        self.barrier
                            .as_ref()
                            .map_or(0, zenkey::presence::Barrier::moves),
                    );
                    book.moved(at);
                }
                let again = book.token(at, &key, held);
                self.owe(again, burst_due);
                self.describe(book);
            }
        }
    }

    /// Addresses owed a read again: one read each, [`REREAD`] after the
    /// first withdrawal of the burst (§2.7).
    fn owe(
        &mut self,
        addrs: Vec<Addr>,
        burst_due: &mut Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
    ) {
        if addrs.is_empty() {
            return;
        }
        self.burst.extend(addrs);
        if burst_due.is_none() {
            *burst_due = Some(Box::pin(tokio::time::sleep(REREAD)));
        }
    }

    /// One read again of `a`, on the deployment's session, beside the view.
    fn read_again(&self, a: Addr) {
        let (session, t, tx, iface) = (
            self.session.clone(),
            self.timeout,
            self.answers.clone(),
            self.iface.clone(),
        );
        tokio::spawn(async move {
            let sel = format!("zk2/{}/{}/@zk/alive/{iface}/**", a.system, a.service);
            match crate::bus::presence::liveliness_read(&session, &sel, t).await {
                Ok(r) => {
                    let at = Instant::now();
                    let tokens: Vec<zenkey_model::grammar::ZkKey> = r
                        .keys
                        .iter()
                        .filter_map(|k| zenkey_model::grammar::parse(k).ok())
                        .collect();
                    let _ = tx.send(Answered::Made);
                    for (address, held) in held_by_address(tokens.iter(), &iface, Some(&a)) {
                        let _ = tx.send(Answered::Read {
                            address,
                            complete: r.complete,
                            held,
                            at,
                        });
                    }
                }
                // Not made, through a session cut off: the view is judged by
                // the deadline meanwhile, and a later withdrawal owes one.
                Err(e) => {
                    tracing::debug!("arbitration.v1: a read again of {sel} was not made: {e}")
                }
            }
        });
    }

    /// The descriptor of each holder whose `meta.zid` is not held, once.
    fn describe(&mut self, book: &ViewBook) {
        let wanted: Vec<(Addr, InstanceId)> = book
            .undescribed()
            .into_iter()
            .filter(|k| self.asked_zids.insert(k.clone()))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let (session, t, tx) = (self.session.clone(), self.timeout, self.answers.clone());
        tokio::spawn(async move {
            let reads = crate::bus::presence::describe_each(&session, wanted, t).await;
            for ((a, i), r) in reads {
                let zid = r
                    .descriptor()
                    .and_then(|d| d.meta.get("zid"))
                    .and_then(|z| z.as_str())
                    .and_then(zenkey_model::timing::zid_value);
                let _ = tx.send(Answered::Described(a, i, zid));
            }
        });
    }

    /// The deliveries the queue refused, and a stall of the receive path,
    /// as losses (§2.14).
    fn losses(&mut self, book: &mut ViewBook) {
        if self.refused.swap(0, Ordering::Relaxed) > 0 {
            book.lose(Instant::now());
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
            book.lose(Instant::now());
        }
    }

    /// Whether a move begun at a withdrawal still waits for its transport
    /// event: none counted since it began.
    fn awaiting(&self) -> bool {
        match (self.awaiting_event, &self.barrier) {
            (Some(at), Some(b)) => b.moves() == at,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    async fn cut_off(&self) -> bool {
        match &self.watch {
            Some(w) => w.cut_off().await.is_some(),
            None => false,
        }
    }

    /// The round trip after a move, `after` from now.
    fn ask_trip(&self, after: Duration) -> Trip {
        let b = self.barrier.clone();
        Box::pin(async move {
            tokio::time::sleep(after).await;
            match b {
                Some(b) => b.establish().await,
                None => false,
            }
        })
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

/// Resolves at `at`; never with none.
async fn until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
