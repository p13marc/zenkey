//! `arbitration.v1`'s runtime (`spec/profiles/arbitration/v1.md`, #909,
//! BB): a consumer bound to several providers of one stream follows one of
//! them at a time, the first in the binding's order whose stream arrives
//! within its deadline and whose interface token it has not heard
//! withdrawn, and none, the dead-man, when no provider is left.
//!
//! Every decision is `zenkey_model::arbitration`'s [`Arbiter`]'s, never
//! re-derived here: the runtime takes each sample's instant in the
//! subscriber's callback, hands the arbiter every sample, every token its
//! watch hears, every read, every move of the watch session's transport and
//! the round trip after it, and its timer, each at its instant, and does
//! what each [`Step`](zenkey_model::arbitration::Step) says: deliver the
//! active provider's arrival, tell the application of each switch, read an
//! address again.
//!
//! ```text
//! ServiceBuilder::arbitrate(role, revision) ─ checked at start(): invalid → not started (§2.2)
//! Service::consumer(role, revision) ─▶ Consumer::arbitrate(on_event) ─▶ Arbitration
//!   data:  a subscriber per entry ─▶ R6, the key's address, timing.v1's expiry, redundancy.v1 §2.10
//!          ─▶ Arbiter::observe(at, Sample) ─▶ Delivered{source, delivery} │ counted
//!   watch: interface tokens on the watch session ─▶ Declared / Withdrawn; a move ─▶ Moved … Stood
//!   reads: 0.1 s after a withdrawal leaving none known held, on the consumer's session ─▶ Read
//!   timer: one-shot at Arbiter::due ─▶ Tick
//!   every switch ─▶ Switched{from, to, cause, at, last}, the first `start`'s, to the dead-man
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.2 an invalid declaration refused at start, nothing declared | [`crate::ServiceBuilder::start`] reads every role carrying `arbitration.policy` with [`declaration_of`], over the revision [`crate::ServiceBuilder::arbitrate`] gave (checks 1 to 5 without one): [`crate::Error::NotExposed`], `… invalid: <reason>` |
//! | §2.3 the deadline, from the revision held | the consumer's contract's, read once at [`crate::consumer::Consumer::arbitrate`] |
//! | §2.4 the binding's order | the consumer's providers in their configured order, `self.system` spelled out ([`Binding`]) |
//! | §2.5 who sent it, from the key; R6 first | [`Arbiter::identify`], over the delivery's key, base-relative |
//! | §2.5 `expired` | `timing.v1` §2.6, by [`crate::consumer::Consumer::subscribe_timed`]'s engine, its deadline left to the arbiter: on the binding's `clock_bound_ms` |
//! | §2.5 `not_incumbent` | a governed required interface (`redundancy.v1` §2.2): a sample whose stamp's id is not the incumbent's `meta.zid` (`redundancy.v1` §2.10), the incumbent the lowest instance holding the interface's token, as the watch and the reads show it, and its descriptor |
//! | §2.7 withdrawals on a watch session, never polled | a liveliness subscriber per entry on [`crate::consumer::Consumer::with_watch`]'s session, its round trip made after declaring it; without one, the deadline alone, said so ([`Arbitration::watching`]) |
//! | §2.7 read again | 0.1 s after the first withdrawal of a burst ([`crate::timing::REREAD`]), one read per address, on the consumer's session, which holds no liveliness subscriber ([`Arbitration::reads`]) |
//! | §2.8 a move | `Moved` at the watch session's transport event, or at a withdrawal heard while it holds no link, whichever comes first (text 0.2, F-268; core §8.1, 0.56); `Stood` when the round trip asked after it stands ([`crate::presence::Barrier`]), asked again until one does |
//! | §2.9 decided at every event; the timer | a one-shot timer at [`Arbiter::due`], re-armed at every step; every event's instant on this host's monotonic clock, never earlier than the last |
//! | §2.9 only the active provider's arrivals delivered | [`ArbitrationEvent::Delivered`], from the subscriber's lock, in decision order |
//! | §2.10, §2.11 every switch told, the dead-man at start | [`ArbitrationEvent::Switched`]; the first is `start`'s, to none |
//!
//! **Order.** Every event goes through one lock, and what it decides is
//! sent, under it, to one channel a task of its own empties into the
//! application's callback: a switch is always told before the delivery it
//! precedes, and the callback may call back into the handle.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::contract::Contract;
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey, parse};
use zenoh::pubsub::Subscriber;
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

pub use zenkey_model::arbitration::{
    Arbiter, Arbitrated, Binding, Cause, Counts, Declaration, Discards, Event, Invalid, Outcome,
    Policy, declaration, declaration_of,
};

use crate::consumer::{Delivery, Provider};
use crate::error::{Error, Result, zenoh};
use crate::timing::{self, Mono, READ_TIMEOUT, REREAD};

/// What an [`Arbitration`] tells its application, in decision order.
#[derive(Debug, Clone)]
pub enum ArbitrationEvent {
    /// An arrival of the active provider (§2.9): `source` is the address
    /// its key names (§2.5), never anything its payload says.
    Delivered {
        source: Addr,
        delivery: Box<Delivery>,
    },
    /// A switch (§2.10).
    Switched(Switched),
}

/// A switch (§2.10): `None` is no provider, the dead-man (§2.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    pub from: Option<Addr>,
    pub to: Option<Addr>,
    pub cause: Cause,
    /// The instant of the event that made it, on this host's monotonic
    /// clock.
    pub at: Instant,
    /// `from`'s last arrival, `None` when `from` is.
    pub last: Option<Instant>,
}

/// Locks a role's core. Zenoh's callbacks lock it, so a panic on another
/// path must not turn every later sample's callback into a panic, as
/// [`crate::timing`]'s engine lock is taken.
fn lock(core: &Mutex<Core>) -> MutexGuard<'_, Core> {
    core.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A presence event as the incumbents of a governed interface follow it
/// (`redundancy.v1` §2.10), kept, as the arbiter keeps its own, through a
/// move (§2.8).
#[derive(Debug, Clone)]
enum Held {
    Declared(Addr, InstanceId),
    Withdrawn(Addr, InstanceId),
    Read(Addr, bool, BTreeSet<InstanceId>),
}

/// The incumbent of each provider of a governed required interface
/// (`redundancy.v1` §2.10): the lowest instance holding the interface's
/// token, and the `meta.zid` its descriptor states.
#[derive(Debug, Default)]
struct Incumbents {
    holders: BTreeMap<Addr, BTreeSet<InstanceId>>,
    zids: BTreeMap<(Addr, InstanceId), u128>,
    kept: Vec<Held>,
}

impl Incumbents {
    fn apply(&mut self, h: Held) {
        match h {
            Held::Declared(a, i) => {
                self.holders.entry(a).or_default().insert(i);
            }
            Held::Withdrawn(a, i) => {
                self.holders.entry(a).or_default().remove(&i);
            }
            Held::Read(a, complete, held) => {
                let known = self.holders.entry(a).or_default();
                if complete {
                    *known = held;
                } else {
                    known.extend(held);
                }
            }
        }
    }

    /// Whether `stamp` is the incumbent's of `addr` (§2.10): its id the
    /// lowest holder's `meta.zid`, by value. Nothing is accepted with no
    /// incumbent seen, or no `meta.zid` held of it, or no stamp.
    fn accepts(&self, addr: &Addr, stamp: Option<&Timestamp>) -> bool {
        let Some(lowest) = self.holders.get(addr).and_then(|h| h.iter().next()) else {
            return false;
        };
        let Some(zid) = self.zids.get(&(addr.clone(), lowest.clone())) else {
            return false;
        };
        stamp.and_then(timing::stamp_id) == Some(*zid)
    }

    /// The holders whose `meta.zid` is not held.
    fn undescribed(&self) -> Vec<(Addr, InstanceId)> {
        self.holders
            .iter()
            .flat_map(|(a, is)| is.iter().map(move |i| (a.clone(), i.clone())))
            .filter(|k| !self.zids.contains_key(k))
            .collect()
    }
}

/// A role's arbitration, behind one lock.
struct Core {
    arbiter: Arbiter,
    iface: IfaceId,
    origin: Instant,
    /// The instant of the last event: none later may precede it.
    last: Instant,
    events: tokio::sync::mpsc::UnboundedSender<ArbitrationEvent>,
    /// Woken when the due instant may have moved.
    rearm: Arc<tokio::sync::Notify>,
    /// Addresses to read again (§2.7).
    reread: tokio::sync::mpsc::UnboundedSender<Addr>,
    /// A governed interface's incumbents (`redundancy.v1` §2.10).
    incumbents: Option<Incumbents>,
    /// Descriptors to GET: a holder whose `meta.zid` is not held.
    describe: tokio::sync::mpsc::UnboundedSender<(Addr, InstanceId)>,
    asked: BTreeSet<(Addr, InstanceId)>,
}

impl Core {
    fn mono(&self, at: Instant) -> Mono {
        Mono::after(at.saturating_duration_since(self.origin))
    }

    fn instant(&self, m: Mono) -> Instant {
        self.origin + m.since_origin()
    }

    /// The event's instant, never before the last one's: callbacks on
    /// several threads take their instants before they take the lock.
    fn settle(&mut self, at: Instant) -> Instant {
        let at = at.max(self.last);
        self.last = at;
        at
    }

    /// Hands the arbiter `event` at `at`, and does what it decides: each
    /// switch told, each read again asked, the timer re-armed. Returns
    /// the sample's outcome, if it is one.
    fn observe(&mut self, at: Instant, event: Event<'_>) -> Option<Outcome> {
        let at = self.settle(at);
        let step = self.arbiter.observe(self.mono(at), event);
        for s in step.switches {
            let last = s.last.map(|m| self.instant(m));
            let _ = self.events.send(ArbitrationEvent::Switched(Switched {
                from: s.from,
                to: s.to,
                cause: s.cause,
                at,
                last,
            }));
        }
        for a in step.reread {
            let _ = self.reread.send(a);
        }
        self.rearm.notify_one();
        step.outcome
    }

    /// A presence event, to the arbiter and to the incumbents, each kept
    /// through a move (§2.8): the arbiter keeps its own.
    fn presence(&mut self, at: Instant, event: Event<'_>, held: Option<Held>) {
        let moving = self.arbiter.moving();
        let stood = matches!(event, Event::Stood);
        self.observe(at, event);
        let Some(inc) = self.incumbents.as_mut() else {
            return;
        };
        if let Some(h) = held {
            if moving {
                inc.kept.push(h);
            } else {
                inc.apply(h);
            }
        }
        if stood && moving {
            for h in std::mem::take(&mut inc.kept) {
                inc.apply(h);
            }
        }
        for k in inc.undescribed() {
            if self.asked.insert(k.clone()) {
                let _ = self.describe.send(k);
            }
        }
    }

    /// An interface token heard, declared or withdrawn: its provider's
    /// incumbents' event, for a bound address of the required interface.
    fn token(&self, key: &str, put: bool) -> Option<Held> {
        let Ok(ZkKey::Alive {
            addr,
            iface,
            instance,
            ..
        }) = parse(key)
        else {
            return None;
        };
        if iface != self.iface || self.arbiter.binding().rank(&addr).is_none() {
            return None;
        }
        Some(if put {
            Held::Declared(addr, instance)
        } else {
            Held::Withdrawn(addr, instance)
        })
    }
}

/// A role's tasks, aborted when dropped: with its [`Arbitration`], or with
/// a start that failed half-way, so that none outlives it holding the core.
#[derive(Default)]
struct Tasks(Vec<tokio::task::JoinHandle<()>>);

impl Tasks {
    fn push(&mut self, t: tokio::task::JoinHandle<()>) {
        self.0.push(t);
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

/// One role's arbitration, running: its subscribers, its watch, its timer.
/// Dropping it undeclares them and stops its tasks.
pub struct Arbitration {
    core: Arc<Mutex<Core>>,
    deadline: Duration,
    watch: Option<zenoh::Session>,
    reads: Arc<AtomicU64>,
    not_made: Arc<AtomicU64>,
    _tasks: Tasks,
    _subs: Vec<Subscriber<()>>,
    _watch_subs: Vec<Subscriber<()>>,
    _lifespan: Option<timing::Running>,
}

impl std::fmt::Debug for Arbitration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Arbitration")
            .field("active", &self.active())
            .field("deadline", &self.deadline)
            .field("watching", &self.watching())
            .finish_non_exhaustive()
    }
}

impl Arbitration {
    /// The active provider, `None` in the dead-man (§2.9, §2.11).
    #[must_use]
    pub fn active(&self) -> Option<Addr> {
        lock(&self.core).arbiter.active().cloned()
    }

    /// Whether the role holds the dead-man (§2.11).
    #[must_use]
    pub fn dead_man(&self) -> bool {
        lock(&self.core).arbiter.dead_man()
    }

    /// The deadline every provider is judged by (§2.3).
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// The instant the arbitration started: its `start` switch's, the
    /// origin of every instant it reports.
    #[must_use]
    pub fn since(&self) -> Instant {
        lock(&self.core).origin
    }

    /// The providers known, in rank order (§2.4).
    #[must_use]
    pub fn providers(&self) -> Vec<Addr> {
        lock(&self.core).arbiter.providers().cloned().collect()
    }

    /// The counts of `provider`'s samples (§2.5).
    #[must_use]
    pub fn counts(&self, provider: &Addr) -> Counts {
        lock(&self.core).arbiter.counts(provider)
    }

    /// Every provider known, in rank order, with its counts (§2.5).
    #[must_use]
    pub fn all_counts(&self) -> Vec<(Addr, Counts)> {
        let core = lock(&self.core);
        core.arbiter
            .providers()
            .map(|a| (a.clone(), core.arbiter.counts(a)))
            .collect()
    }

    /// The counts of the samples with no provider (§2.5).
    #[must_use]
    pub fn discards(&self) -> Discards {
        lock(&self.core).arbiter.discards()
    }

    /// Whether `provider` is withdrawn (§2.7).
    #[must_use]
    pub fn withdrawn(&self, provider: &Addr) -> bool {
        lock(&self.core).arbiter.withdrawn(provider)
    }

    /// Whether a move of the watch session is pending (§2.8): what the
    /// watch hears is kept until the round trip after it stands.
    #[must_use]
    pub fn moving(&self) -> bool {
        lock(&self.core).arbiter.moving()
    }

    /// Whether withdrawals are heard: a watch session was handed in, and
    /// presence is observable (§2.7, §2.12). Without, every provider is
    /// judged by its deadline alone.
    #[must_use]
    pub fn watching(&self) -> bool {
        self.watch.is_some()
    }

    /// The reads again made (§2.7), each one read of one address.
    #[must_use]
    pub fn reads(&self) -> u64 {
        self.reads.load(Ordering::Relaxed)
    }

    /// The reads again not made: refused, through a session cut off from
    /// every other zenoh node (core §8.1, 0.37).
    #[must_use]
    pub fn reads_not_made(&self) -> u64 {
        self.not_made.load(Ordering::Relaxed)
    }

    /// The round trips the watch session has asked
    /// ([`crate::presence::liveliness_gets`]'s `settles`): the one after
    /// its subscribers' declaration, and one after each move (§2.8; core
    /// §8.1, 0.44). Zero without a watch session.
    #[must_use]
    pub fn round_trips(&self) -> u64 {
        self.watch
            .as_ref()
            .map_or(0, |w| crate::presence::liveliness_gets(w).settles)
    }

    /// The watch session, when there is one.
    #[must_use]
    pub fn watch_session(&self) -> Option<&zenoh::Session> {
        self.watch.as_ref()
    }
}

/// What [`crate::consumer::Consumer::arbitrate`] hands over.
pub(crate) struct Inputs<'a> {
    pub(crate) session: &'a zenoh::Session,
    pub(crate) watch: Option<zenoh::Session>,
    pub(crate) role: &'a str,
    /// The contract declaring the role, `None` for a manifest's.
    pub(crate) declaring: Option<&'a Contract>,
    /// The revision of the required interface held (§2.3).
    pub(crate) required: &'a Contract,
    pub(crate) providers: &'a [Provider],
    pub(crate) params: &'a BTreeMap<String, String>,
    pub(crate) epsilon: Option<Duration>,
    pub(crate) selectors: Vec<zenoh::key_expr::OwnedKeyExpr>,
}

/// The declaration of `role`, arbitrated, or why it is not (§2.2).
pub(crate) fn arbitrated(i: &Inputs<'_>) -> Result<Arbitrated> {
    let d = match i.declaring {
        Some(c) => declaration_of(c, i.role, Some(i.required), i.params)
            .ok_or_else(|| Error::Contract(format!("{} declares no role {:?}", c.iface, i.role)))?,
        // A manifest's role carries no annotation (core §3.3).
        None => Declaration::NotArbitrated,
    };
    match d {
        Declaration::Arbitrated(a) => Ok(a),
        Declaration::NotArbitrated => Err(Error::Contract(format!(
            "role {:?} carries no arbitration.policy: it is not arbitrated, and receives from \
             every bound provider merged (core §3.2; arbitration.v1 §2.2)",
            i.role
        ))),
        Declaration::Unread => Err(Error::Contract(format!(
            "role {:?}: no revision of {} held (arbitration.v1 §2.2)",
            i.role, i.required.iface
        ))),
        Declaration::Invalid(why) => Err(Error::Contract(format!(
            "role {:?} ({}): arbitration.v1 {}",
            i.role,
            i.required.iface,
            invalid_words(&why)
        ))),
    }
}

/// `invalid: <reason>[, <reason>…] (<what the first says>; §2.2)`, the
/// words a refusal ends with.
pub(crate) fn invalid_words(why: &[Invalid]) -> String {
    let tokens: Vec<&str> = why.iter().map(|w| w.as_str()).collect();
    let says = why.first().map_or("", |w| w.says());
    format!("invalid: {} ({says}; §2.2)", tokens.join(", "))
}

/// Starts one role's arbitration (§2.4 to §2.11).
pub(crate) async fn start<C>(i: Inputs<'_>, on_event: C) -> Result<Arbitration>
where
    C: Fn(ArbitrationEvent) + Send + Sync + 'static,
{
    let declared = arbitrated(&i)?;
    if let Some(w) = &i.watch
        && w.zid() == i.session.zid()
    {
        return Err(Error::Contract(
            "arbitration.v1 §2.7: the watch session is the consumer's own, which makes every \
             read again (core §8.1, 0.37)"
                .to_owned(),
        ));
    }
    let entries: Vec<String> = i
        .providers
        .iter()
        .map(|p| {
            let (s, v) = p.chunks();
            format!("{s}/{v}")
        })
        .collect();
    let binding = Binding::parse(&entries).map_err(|e| Error::Contract(e.to_string()))?;
    let resource = i
        .required
        .resources
        .iter()
        .find(|r| r.token == declared.token && r.template == declared.template)
        .cloned()
        .ok_or_else(|| {
            Error::Contract(format!(
                "{} has no {}",
                i.required.iface,
                declared.resource()
            ))
        })?;
    let governed = zenkey_model::redundancy::is_governed(i.required);
    let (events, mut rx) = tokio::sync::mpsc::unbounded_channel::<ArbitrationEvent>();
    let (reread_tx, reread_rx) = tokio::sync::mpsc::unbounded_channel::<Addr>();
    let (describe_tx, describe_rx) = tokio::sync::mpsc::unbounded_channel::<(Addr, InstanceId)>();
    let rearm = Arc::new(tokio::sync::Notify::new());
    let mut tasks = Tasks::default();
    // The dispatcher: every event, in the order decided.
    tasks.push(tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            on_event(ev);
        }
    }));
    let origin = Instant::now();
    let mut arbiter = Arbiter::new(&declared, binding, i.params.clone());
    let deadline = arbiter.deadline();
    // §2.11: the dead-man at start, told first.
    for s in arbiter.start(Mono::ZERO).switches {
        let _ = events.send(ArbitrationEvent::Switched(Switched {
            from: s.from,
            to: s.to,
            cause: s.cause,
            at: origin,
            last: None,
        }));
    }
    let core = Arc::new(Mutex::new(Core {
        arbiter,
        iface: i.required.iface.clone(),
        origin,
        last: origin,
        events,
        rearm: Arc::clone(&rearm),
        reread: reread_tx,
        incumbents: governed.then(Incumbents::default),
        describe: describe_tx,
        asked: BTreeSet::new(),
    }));
    // §2.5's `expired`: timing.v1's lifespan engine, its deadline the
    // arbiter's own.
    let plan = timing::Plan::new(
        &resource,
        &i.required.iface,
        i.providers,
        i.params,
        i.epsilon,
    )
    .without_deadline();
    let premise = plan.premise();
    let prepared = timing::prepare(plan, origin);
    let engine = prepared.as_ref().map(|p| Arc::clone(&p.engine));
    let name = declared.resource();
    let template = declared.template.clone();
    let mut subs = Vec::new();
    // One subscriber per entry: a sample whose key two entries' selectors
    // cover (an address matched twice, a wildcard over an exact entry, or
    // a wild key) reaches each of them, and is the arbiter's once, from
    // the first (§2.4: an address matched twice is one provider).
    let selectors = Arc::new(i.selectors);
    for (at, ke) in selectors.iter().enumerate() {
        let (core, engine, name, template, all) = (
            Arc::clone(&core),
            engine.clone(),
            name.clone(),
            template.clone(),
            Arc::clone(&selectors),
        );
        let sub = i
            .session
            .declare_subscriber(ke.clone())
            .callback(move |sample: Sample| {
                // timing.v1 §2.5: its arrival, read here, before anything.
                let arrival = Instant::now();
                let wall = SystemTime::now();
                if all
                    .iter()
                    .position(|k| k.intersects(sample.key_expr()))
                    .is_some_and(|first| first != at)
                {
                    return;
                }
                let mut c = lock(&core);
                let key = sample.key_expr().as_str().to_owned();
                let addr = match c.arbiter.identify(&key) {
                    Ok(a) => a,
                    // §2.5's outcomes 1 to 3: counted, no provider.
                    Err(_) => {
                        c.observe(
                            arrival,
                            Event::Sample(zenkey_model::arbitration::Sample::new(&key)),
                        );
                        return;
                    }
                };
                let delete = sample.kind() == SampleKind::Delete;
                let chunks: Vec<String> = match parse(&key) {
                    Ok(ZkKey::Data { resource, .. }) => resource,
                    _ => Vec::new(),
                };
                let lifespan = match (&engine, delete) {
                    (Some(e), false) => {
                        timing::lock(e).arrive(&addr, &key, &chunks, &sample, arrival, wall)
                    }
                    _ => Some(premise),
                };
                let not_incumbent = !delete
                    && c.incumbents
                        .as_ref()
                        .is_some_and(|inc| !inc.accepts(&addr, sample.timestamp()));
                let outcome = c.observe(
                    arrival,
                    Event::Sample(zenkey_model::arbitration::Sample {
                        key: &key,
                        delete,
                        expired: lifespan.is_none(),
                        not_incumbent,
                    }),
                );
                if outcome == Some(Outcome::Delivered) {
                    let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                    let values = template.matches(&refs).unwrap_or_default();
                    let stamp = sample.timestamp().copied();
                    let _ = c.events.send(ArbitrationEvent::Delivered {
                        source: addr.clone(),
                        delivery: Box::new(Delivery {
                            provider: addr,
                            resource: name.clone(),
                            values,
                            sample,
                            arrival,
                            stamp,
                            lifespan: lifespan.unwrap_or(premise),
                        }),
                    });
                }
            })
            .await
            .map_err(zenoh)?;
        subs.push(sub);
    }
    let lifespan = match prepared {
        Some(p) => Some(
            p.run(
                timing::Sources {
                    session: i.session.clone(),
                    watch: None,
                    providers: i.providers.to_vec(),
                },
                |_| {},
            )
            .await,
        ),
        None => None,
    };
    // A governed interface's descriptors: each holder's `meta.zid`, from
    // its descriptor's puts (core §3.3: before its tokens) and a GET.
    if governed {
        for p in i.providers {
            let (sys, svc) = p.chunks();
            let c = Arc::clone(&core);
            let sub = i
                .session
                .declare_subscriber(format!("zk2/{sys}/{svc}/@zk/instance/*"))
                .callback(move |s: Sample| {
                    if let Some((addr, instance, zid)) = described(&s)
                        && let Some(inc) = lock(&c).incumbents.as_mut()
                    {
                        inc.zids.insert((addr, instance), zid);
                    }
                })
                .await
                .map_err(zenoh)?;
            subs.push(sub);
        }
        tasks.push(tokio::spawn(describe(
            i.session.clone(),
            Arc::clone(&core),
            describe_rx,
        )));
    }
    // §2.7: the watch, its round trip made after declaring it.
    let reads = Arc::new(AtomicU64::new(0));
    let not_made = Arc::new(AtomicU64::new(0));
    let iface = i.required.iface.clone();
    let mut watch_subs = Vec::new();
    let mut barrier = None;
    let (heard_tx, heard_rx) = tokio::sync::mpsc::unbounded_channel::<Heard>();
    if let Some(w) = &i.watch {
        let mut first = None;
        for p in i.providers {
            let (sys, svc) = p.chunks();
            let scope = format!("zk2/{sys}/{svc}/@zk/alive/{iface}/**");
            let tx = heard_tx.clone();
            let sub = crate::presence::declare_subscriber(w, scope.clone())
                .history(crate::presence::subscriber_needs_history(w))
                .callback(move |s: Sample| {
                    let _ = tx.send(Heard {
                        key: s.key_expr().as_str().to_owned(),
                        put: s.kind() == SampleKind::Put,
                        at: Instant::now(),
                    });
                })
                .await
                .map_err(zenoh)?;
            watch_subs.push(sub);
            first.get_or_insert(scope);
        }
        if let Some(scope) = first {
            let b = crate::presence::Barrier::new(w, &scope, READ_TIMEOUT).await?;
            if !b.establish().await {
                tracing::warn!(
                    "arbitration.v1: the watch session's round trip did not stand at start: a \
                     withdrawal heard before it is still applied (§2.8: at start there is no move)"
                );
            }
            barrier = Some(b);
        }
    } else {
        tracing::info!(
            role = i.role,
            "arbitration.v1: no watch session, so no withdrawal is heard and every provider is \
             judged by its deadline alone (§2.7, §2.12)"
        );
    }
    drop(heard_tx);
    // redundancy.v1 §2.10: a governed interface's incumbents, read once
    // after the round trip; a read the arbiter is told of too, never a
    // read again.
    if governed {
        let seeds = AtomicU64::new(0);
        for p in i.providers {
            let (sys, svc) = p.chunks();
            let sel = format!("zk2/{sys}/{svc}/@zk/alive/{iface}/**");
            read_into_core(i.session, &core, &iface, &sel, &seeds, &not_made, false).await;
        }
    }
    let follower = Follower {
        core: Arc::clone(&core),
        session: i.session.clone(),
        watch: i.watch.clone(),
        iface: iface.clone(),
        barrier,
        reads: Arc::clone(&reads),
        not_made: Arc::clone(&not_made),
        awaiting_event: None,
    };
    tasks.push(tokio::spawn(follower.run(heard_rx, reread_rx)));
    // §2.9: the timer, one-shot at the active provider's due instant.
    let c = Arc::clone(&core);
    tasks.push(tokio::spawn(async move {
        loop {
            let due = {
                let core = lock(&c);
                core.arbiter.due().map(|d| core.instant(d))
            };
            match due {
                // A miss holds at any instant after its due.
                Some(d) => tokio::select! {
                    () = tokio::time::sleep_until((d + Duration::from_micros(1)).into()) => {
                        lock(&c).observe(Instant::now(), Event::Tick);
                    }
                    () = rearm.notified() => {}
                },
                None => rearm.notified().await,
            }
        }
    }));
    Ok(Arbitration {
        core,
        deadline,
        watch: i.watch,
        reads,
        not_made,
        _tasks: tasks,
        _subs: subs,
        _watch_subs: watch_subs,
        _lifespan: lifespan,
    })
}

/// A descriptor put's address, instance and `meta.zid`.
fn described(s: &Sample) -> Option<(Addr, InstanceId, u128)> {
    if s.kind() != SampleKind::Put {
        return None;
    }
    let Ok(ZkKey::Instance { addr, instance }) = parse(s.key_expr().as_str()) else {
        return None;
    };
    let v: serde_json::Value = serde_json::from_slice(&s.payload().to_bytes()).ok()?;
    let zid = timing::zid_value(v.get("meta")?.get("zid")?.as_str()?)?;
    Some((addr, instance, zid))
}

/// GETs the descriptor of each holder whose `meta.zid` is not held
/// (`redundancy.v1` §2.10): a holder heard before its descriptor's put
/// reached this session.
async fn describe(
    session: zenoh::Session,
    core: Arc<Mutex<Core>>,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<(Addr, InstanceId)>,
) {
    while let Some((addr, instance)) = rx.recv().await {
        let found = crate::presence::descriptor(&session, &addr, &instance, READ_TIMEOUT).await;
        let zid = match found {
            Ok(crate::presence::Found::Descriptor(d, _)) => d
                .meta
                .get("zid")
                .and_then(|z| z.as_str())
                .and_then(timing::zid_value),
            _ => None,
        };
        let mut c = lock(&core);
        match zid {
            Some(z) => {
                if let Some(inc) = c.incumbents.as_mut() {
                    inc.zids.insert((addr, instance), z);
                }
            }
            // Asked again at the holder's next sight.
            None => {
                c.asked.remove(&(addr, instance));
            }
        }
    }
}

/// Reads `selector`'s interface tokens on `session`, which holds no
/// liveliness subscriber (core §8.1, 0.37), and hands the arbiter a read
/// of each bound address the selector covers, counted in `made`; with
/// `exact`, of the selector's one address, none held included. A read
/// refused, through a session cut off, is not made, and counted so.
async fn read_into_core(
    session: &zenoh::Session,
    core: &Arc<Mutex<Core>>,
    iface: &IfaceId,
    selector: &str,
    made: &AtomicU64,
    not_made: &AtomicU64,
    exact: bool,
) {
    let read = match crate::presence::liveliness_read(session, selector, READ_TIMEOUT).await {
        Ok(r) => r,
        Err(e) => {
            not_made.fetch_add(1, Ordering::Relaxed);
            tracing::debug!("arbitration.v1: a read of {selector} was not made: {e}");
            return;
        }
    };
    made.fetch_add(1, Ordering::Relaxed);
    let now = Instant::now();
    let mut by: BTreeMap<Addr, BTreeSet<InstanceId>> = BTreeMap::new();
    for k in &read.keys {
        if let Ok(ZkKey::Alive {
            addr,
            iface: i,
            instance,
            ..
        }) = parse(k)
            && i == *iface
        {
            by.entry(addr).or_default().insert(instance);
        }
    }
    let mut c = lock(core);
    // An exact address's read says what it holds, none included.
    if exact && let Some(a) = address_of(selector) {
        by.entry(a).or_default();
    }
    for (address, held) in by {
        c.presence(
            now,
            Event::Read {
                address: address.clone(),
                complete: read.complete,
                held: held.clone(),
            },
            Some(Held::Read(address, read.complete, held)),
        );
    }
}

/// The address of `zk2/<system>/<service>/@zk/…`, when both are plain.
fn address_of(selector: &str) -> Option<Addr> {
    let mut parts = selector.split('/');
    let (_, sys, svc) = (parts.next()?, parts.next()?, parts.next()?);
    format!("{sys}/{svc}").parse().ok()
}

/// A token the watch heard.
struct Heard {
    key: String,
    put: bool,
    at: Instant,
}

/// The watch's follower (§2.7, §2.8): every token heard, to the arbiter;
/// each read again, 0.1 s after the first withdrawal of a burst; each move
/// and the round trip after it.
struct Follower {
    core: Arc<Mutex<Core>>,
    session: zenoh::Session,
    watch: Option<zenoh::Session>,
    iface: IfaceId,
    barrier: Option<crate::presence::Barrier>,
    reads: Arc<AtomicU64>,
    not_made: Arc<AtomicU64>,
    /// A move begun at a withdrawal whose transport event has not come
    /// yet (F-268).
    awaiting_event: Option<()>,
}

type Trip = std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

/// How long after a move of the watch session the round trip that ends it
/// is asked (text 0.2; core §8.1, 0.56): zenoh 1.10.1 tells a transport's
/// open to its listener before its router re-declares the session's
/// subscribers' interests on it, and a round trip asked in between finds no
/// face, and stands at once, complete and empty, before the reopen's
/// telling (measured: 2 of 30 asked at the event; 0 of 30 asked 100 ms
/// after, `tests/presence.rs`).
const AFTER_OPEN: Duration = Duration::from_millis(100);

impl Follower {
    async fn run(
        mut self,
        mut heard: tokio::sync::mpsc::UnboundedReceiver<Heard>,
        mut reread: tokio::sync::mpsc::UnboundedReceiver<Addr>,
    ) {
        let barrier = self.barrier.clone();
        let mut moves_seen = barrier.as_ref().map_or(0, crate::presence::Barrier::moves);
        let mut trip: Option<Trip> = None;
        let mut burst: BTreeSet<Addr> = BTreeSet::new();
        let mut burst_due: Option<std::pin::Pin<Box<tokio::time::Sleep>>> = None;
        let mut heard_open = true;
        loop {
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
                h = heard.recv(), if heard_open => match h {
                    Some(h) => self.heard(h).await,
                    None => heard_open = false,
                },
                a = reread.recv() => match a {
                    Some(a) => {
                        burst.insert(a);
                        if burst_due.is_none() {
                            burst_due = Some(Box::pin(tokio::time::sleep(REREAD)));
                        }
                    }
                    None => break,
                },
                () = read_due => {
                    burst_due = None;
                    for a in std::mem::take(&mut burst) {
                        let sel = format!("zk2/{}/{}/@zk/alive/{}/**", a.system, a.service, self.iface);
                        read_into_core(&self.session, &self.core, &self.iface, &sel, &self.reads, &self.not_made, true).await;
                    }
                }
                // §2.8: the watch session's transport closed or opened.
                () = moved => {
                    moves_seen = barrier.as_ref().map_or(moves_seen, crate::presence::Barrier::moves);
                    self.awaiting_event = None;
                    lock(&self.core).presence(Instant::now(), Event::Moved, None);
                    // Not at the event's instant: zenoh 1.10.1 tells an
                    // open before its router declares the subscribers'
                    // interests again, and a round trip asked in between
                    // stands with nothing told (core §8.1, 0.56).
                    trip = Some(self.ask_trip(AFTER_OPEN));
                }
                stood = tripped => {
                    trip = None;
                    let established = barrier.as_ref().is_some_and(crate::presence::Barrier::established);
                    // A move begun at a withdrawal (F-268) stands only by a
                    // round trip asked after its transport event.
                    if stood && established && self.awaiting_event.is_none() {
                        // What the watch heard before the round trip's
                        // final reply is the move's: the reopen's telling
                        // comes first (core §8.1, 0.44), and waits in the
                        // channel, so it is taken before the move ends.
                        while let Ok(h) = heard.try_recv() {
                            self.heard(h).await;
                        }
                        lock(&self.core).presence(Instant::now(), Event::Stood, None);
                    } else if !self.cut_off().await {
                        // Not through a session cut off: asked again until
                        // one stands, or a later move asks its own.
                        trip = Some(self.ask_trip(READ_TIMEOUT / 2));
                    }
                    // Cut off: the reopen is a move, which asks its own.
                }
            }
        }
    }

    /// A token heard, to the arbiter and the incumbents: a withdrawal heard
    /// with no link held beginning a move first.
    async fn heard(&mut self, h: Heard) {
        self.move_begins(&h).await;
        let held = lock(&self.core).token(&h.key, h.put);
        let ev = if h.put {
            Event::Declared(&h.key)
        } else {
            Event::Withdrawn(&h.key)
        };
        lock(&self.core).presence(h.at, ev, held);
    }

    /// A withdrawal heard while the watch session holds no link begins a
    /// move (§2.8, text 0.2, F-268; core §8.1, 0.56): zenoh 1.10.1 tells a
    /// close's withdrawals before the close's transport event. Its `Moved`
    /// is handed over at once, at the withdrawal's instant, before the
    /// withdrawal itself; and until the transport event comes, no round
    /// trip stands for it: one asked before the event would find the
    /// barrier standing still.
    async fn move_begins(&mut self, h: &Heard) {
        if h.put || !self.cut_off().await {
            return;
        }
        self.awaiting_event = Some(());
        if !lock(&self.core).arbiter.moving() {
            lock(&self.core).presence(h.at, Event::Moved, None);
        }
    }

    async fn cut_off(&self) -> bool {
        match &self.watch {
            Some(w) => crate::presence::cut_off(w).await,
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
