//! `alarms.v1` (#800, AC), the reads: which alarms fire under a selector
//! (`spec/profiles/alarms/v1.md`), read as §2.14 and §2.15's reader reads
//! them, through a session in the deployment's namespace.
//!
//! What is read, in order (core §4.2's late joiner):
//! 1. when the reader listens: a liveliness subscription to the instance
//!    tokens of the owners in scope (a gap in an owner's presence breaks
//!    its window, §2.15), then a subscription to the scope's alarm keys,
//!    then a watch of this reader's own receive path
//!    ([`crate::bus::stall`], core §2.7, 0.27). Both subscriptions call
//!    back into one bounded queue, and what it refuses is counted: a
//!    [`AlarmLoss`] that makes every key unobservable, never ok;
//! 2. presence and every instance's descriptor (core §8.1, §3.3): an
//!    owner implements `alarms.v1` when its descriptor lists it, token or
//!    not (§2.13). Across a constrained face neither crosses, and the
//!    deployment's word stands for them (§2.11);
//! 3. S4's GET of the scope's alarms (`bus::query::state_get`: target
//!    `All`, consolidation `Latest`), every reply merged with the live
//!    deliveries by the owner's stamp, in the [`AlarmBook`];
//! 4. when asked, every archive's last-known documents of the scope
//!    (core §4.4, an explicit read, never current, S6): what they hold is
//!    shown, and the owners their keys name are in scope (§2.15, F-131).
//!
//! Nothing is judged here, and every payload is read by
//! `zenkey_model::alarms::decode` inside the book. [`AlarmFeed`] is the
//! live reader the watchdog's `alert-firing` and a notifier hold: its
//! deliveries are folded as they come ([`AlarmFeed::next`]), and its
//! refresh — presence, the GET, the archives — is an owned
//! [`AlarmRefresh`], run beside a drain and folded after it, so a drain
//! never waits on a read. [`observe`] is the one-shot reading `zenctl
//! alarms` makes: the feed, listened to for a window when asked, then a
//! lease and the reply timeout more while its owners are watched (core
//! §2.7, 0.25).

use std::collections::BTreeSet;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::grammar::{IfaceId, ZkKey};
use zenoh::Session;
use zenoh::sample::{Sample, SampleKind};

use crate::bus::presence::Scope;
use crate::bus::stall::{StallSeen, StallWatch};
use crate::model::alarms::{AlarmBook, AlarmSample, AlarmScope, ArchivedAlarm, BookSpec, SeedRead};
use crate::model::catalog::{Catalog, Observed};
use crate::report::{AlarmLoss, AlarmTransition, AlarmVia};

/// `archive.v1`'s interface id: its providers are the archives asked.
const ARCHIVE: &str = "archive.v1";

/// The queue the subscriptions call back into: the monitor's default
/// broadcast capacity, for the same reason (bound it to what a reader can
/// drain, and count what it refuses).
pub const QUEUE: usize = 1024;

/// What a reading asks.
#[derive(Debug, Clone, Copy)]
pub struct AlarmSpec {
    /// Each read's timeout: presence, a descriptor, the GET, an archive.
    pub timeout: Duration,
    /// [`observe`]'s window: how long its subscription listens before the
    /// lease and the reply timeout it watches the owners for after (core
    /// §2.7, 0.25). `None`: no subscription, the GET alone.
    pub window: Option<Duration>,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1).
    pub clocks_synced: bool,
    /// The deployment's link lease (core §2.7, 0.25).
    pub lease: Duration,
    /// The routers' `wait_before_close`: with the lease, the longest this
    /// reader may stall and keep a window lossless (core §2.7, 0.27).
    pub wait_before_close: Duration,
    /// Across a constrained face (§2.11): whether the face lets this
    /// reader GET the alarms. `None`: no face.
    pub face: Option<bool>,
    /// Read every archive's last-known documents of the scope (S6).
    pub archives: bool,
}

impl AlarmSpec {
    /// No window, no word, zenoh's lease and `wait_before_close`, no face,
    /// archives read.
    pub fn new(timeout: Duration) -> AlarmSpec {
        AlarmSpec {
            timeout,
            window: None,
            clocks_synced: false,
            lease: zenkey_model::budget::DEFAULT_LEASE,
            wait_before_close: zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE,
            face: None,
            archives: true,
        }
    }

    /// The book's half of it.
    pub fn book(&self) -> BookSpec {
        BookSpec {
            timeout: self.timeout,
            lease: self.lease,
            clocks_synced: self.clocks_synced,
            face: self.face,
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

/// What a subscription called back with.
enum Delivery {
    Sample(AlarmSample),
    Instance { key: ZkKey, held: bool, at: Instant },
}

/// The bounded queue's sending half: a refusal is counted, never waited
/// on (a callback must not block zenoh's receive path).
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

/// What the reads of the last refresh found, as a report says it.
#[derive(Debug, Clone, Default)]
pub struct ReadLog {
    /// The presence read: its selector, whether complete, and how many
    /// services in scope it found holding an instance token, and listing
    /// `alarms.v1`; or why it could not be made. `None` across a face.
    pub presence: Option<Result<PresenceSeen, String>>,
    /// The GET: whether it ran to its final reply, or why it could not be
    /// made.
    pub seed: Option<Result<bool, String>>,
    /// The archives asked, or why they could not be.
    pub archives: Option<Result<Vec<String>, String>>,
    /// Replies and deliveries on a non-concrete key, discarded (R6).
    pub discarded: u64,
}

/// One presence read, summed up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceSeen {
    pub selector: String,
    pub complete: bool,
    pub services: usize,
    pub listing: usize,
}

/// A refresh's reads, owned: presence, S4's GET, the archives. Run it
/// beside a drain ([`AlarmRefresh::run`]), and fold what it read
/// ([`AlarmFeed::fold`]).
#[derive(Debug, Clone)]
pub struct AlarmRefresh {
    session: Session,
    scope: AlarmScope,
    spec: AlarmSpec,
}

/// What a refresh read, as values.
#[derive(Debug, Clone)]
pub struct Refreshed {
    /// The presence read, and when it was taken. `None` across a face.
    pub presence: Option<(Instant, Result<Observed, String>)>,
    pub seed: Result<SeedRead, String>,
    pub archived: Option<Result<(Vec<String>, Vec<ArchivedAlarm>), String>>,
}

impl AlarmRefresh {
    /// Presence, then the GET, then the archives.
    pub async fn run(self) -> Refreshed {
        let t = self.spec.timeout;
        let presence = match self.spec.face {
            Some(_) => None,
            None => Some((
                Instant::now(),
                crate::bus::presence::observe(&self.session, &self.scope.presence_scope(), t)
                    .await
                    .map_err(|e| crate::one_line(&e)),
            )),
        };
        let seed = get(&self.session, self.scope.alarms_selector(), t).await;
        let archived = if self.spec.archives && self.spec.face.is_none() {
            Some(archived(&self.session, &self.scope, t).await)
        } else {
            None
        };
        Refreshed {
            presence,
            seed,
            archived,
        }
    }
}

/// S4's GET of `selector` (core §4.2): target `All`, consolidation
/// `Latest`, through the crate's one state GET. A reply on a wildcard key
/// is discarded (R6); an error reply, a timeout's included, makes it
/// incomplete.
pub async fn get(session: &Session, selector: &str, timeout: Duration) -> Result<SeedRead, String> {
    let asked_at = Instant::now();
    let replies =
        crate::bus::query::state_get(session, selector, &crate::bus::query::GetOpts::new(timeout))
            .await
            .map_err(|e| format!("GET {selector}: {}", crate::one_line(&e)))?;
    let mut out = SeedRead {
        asked_at,
        complete: true,
        replies: Vec::new(),
        discarded: 0,
    };
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            out.complete = false;
            continue;
        };
        if sample.key_expr().as_str().contains('*') {
            out.discarded += 1;
            continue;
        }
        out.replies.push(sample_of(sample, AlarmVia::Get));
    }
    Ok(out)
}

fn sample_of(sample: &Sample, via: AlarmVia) -> AlarmSample {
    AlarmSample {
        key: sample.key_expr().as_str().to_owned(),
        delete: sample.kind() == SampleKind::Delete,
        payload: sample.payload().to_bytes().into_owned(),
        stamp: sample.timestamp().copied(),
        at: Instant::now(),
        wall: SystemTime::now(),
        via,
    }
}

/// Every archive presence shows (by token or by descriptor), each asked
/// for its last-known documents of the scope's alarms (core §4.4, S5).
async fn archived(
    session: &Session,
    scope: &AlarmScope,
    timeout: Duration,
) -> Result<(Vec<String>, Vec<ArchivedAlarm>), String> {
    let all = crate::bus::presence::observe(session, &Scope::all(), timeout)
        .await
        .map_err(|e| crate::one_line(&e))?;
    let archive = IfaceId::from_str(ARCHIVE).expect("archive.v1 is an interface id");
    let mut asked = Vec::new();
    let mut out = Vec::new();
    for a in Catalog::new(&all).providers_of(&archive) {
        asked.push(a.to_string());
        let got = zenkey::archive::last_known_all(session, &a, scope.alarms_selector(), timeout)
            .await
            .map_err(|e| format!("archive {a}: {e}"))?;
        out.extend(got.into_iter().map(|lk| ArchivedAlarm {
            archive: a.clone(),
            key: lk.origin,
            value: lk.value,
            stamp: lk.timestamp,
            confirmed: lk.confirmed,
        }));
    }
    Ok((asked, out))
}

/// One reader of a scope's alarms, live: see the module doc.
pub struct AlarmFeed {
    book: AlarmBook,
    session: Session,
    spec: AlarmSpec,
    rx: Option<tokio::sync::mpsc::Receiver<Delivery>>,
    refused: Arc<AtomicU64>,
    _tokens: Option<zenoh::pubsub::Subscriber<()>>,
    _data: Option<zenoh::pubsub::Subscriber<()>>,
    discarded: Arc<AtomicU64>,
    stall: Option<Result<StallWatch, String>>,
    stall_seen: StallSeen,
    restart_stall: bool,
    log: ReadLog,
}

impl std::fmt::Debug for AlarmFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlarmFeed")
            .field("scope", self.book.scope())
            .finish_non_exhaustive()
    }
}

impl AlarmFeed {
    /// Opens a reader over `scope`: when `listen`, its subscriptions and
    /// its receive path's watch first, then one refresh, folded. Returns
    /// the feed and the transitions the refresh made: each alarm found,
    /// raised or resolved as a late reader reads it (§2.6).
    pub async fn open(
        session: &Session,
        scope: AlarmScope,
        spec: AlarmSpec,
        listen: bool,
    ) -> crate::Result<(AlarmFeed, Vec<AlarmTransition>)> {
        let mut feed = AlarmFeed {
            book: AlarmBook::new(scope, spec.book()),
            session: session.clone(),
            spec,
            rx: None,
            refused: Arc::default(),
            _tokens: None,
            _data: None,
            discarded: Arc::default(),
            stall: None,
            stall_seen: StallSeen::default(),
            restart_stall: false,
            log: ReadLog::default(),
        };
        if listen {
            feed.listen().await?;
        }
        let refreshed = feed.refresh_plan().run().await;
        let out = feed.fold(refreshed);
        Ok((feed, out))
    }

    /// The subscriptions, in core §4.2's order, and the watch of this
    /// reader's receive path.
    async fn listen(&mut self) -> crate::Result<()> {
        let (tx, rx) = tokio::sync::mpsc::channel(QUEUE);
        let queue = Queue {
            tx,
            refused: Arc::clone(&self.refused),
        };
        let scope = self.book.scope().clone();
        let q = queue.clone();
        let tokens = crate::bus::teardown::declared(
            "alarms' instance tokens",
            scope.instance_selector(),
            self.session
                .liveliness()
                .declare_subscriber(scope.instance_selector())
                .callback(move |s: Sample| {
                    if let Ok(key @ ZkKey::Instance { .. }) =
                        zenkey_model::grammar::parse(s.key_expr().as_str())
                    {
                        q.send(Delivery::Instance {
                            key,
                            held: s.kind() == SampleKind::Put,
                            at: Instant::now(),
                        });
                    }
                }),
        )
        .await?;
        let q = queue;
        let discarded = Arc::clone(&self.discarded);
        let data = crate::bus::teardown::declared(
            "alarms",
            scope.alarms_selector(),
            self.session
                .declare_subscriber(scope.alarms_selector().to_owned())
                .callback(move |s: Sample| {
                    if s.key_expr().as_str().contains('*') {
                        discarded.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    q.send(Delivery::Sample(sample_of(&s, AlarmVia::Subscription)));
                }),
        )
        .await?;
        self.book.subscribed(Instant::now());
        self.stall = Some(StallWatch::start(&self.session).await);
        self._tokens = Some(tokens);
        self._data = Some(data);
        self.rx = Some(rx);
        Ok(())
    }

    /// The book: every judgement reads it.
    pub fn book(&self) -> &AlarmBook {
        &self.book
    }

    /// What the last refresh's reads found.
    pub fn log(&self) -> &ReadLog {
        &self.log
    }

    /// The spec it reads with.
    pub fn spec(&self) -> &AlarmSpec {
        &self.spec
    }

    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider (tooling guide O5).
    pub fn asked(&self) -> Vec<String> {
        let scope = self.book.scope();
        let mut out = Vec::new();
        if self.spec.face.is_none() {
            out.push(scope.presence_scope().selector());
        }
        out.push(scope.alarms_selector().to_owned());
        if self.rx.is_some() {
            out.push(scope.instance_selector());
        }
        let mut seen = BTreeSet::new();
        out.retain(|s| seen.insert(s.clone()));
        out
    }

    /// A refresh's reads, owned, to run beside a drain.
    pub fn refresh_plan(&self) -> AlarmRefresh {
        AlarmRefresh {
            session: self.session.clone(),
            scope: self.book.scope().clone(),
            spec: self.spec,
        }
    }

    /// One refresh, run and folded: when nothing drains beside it.
    pub async fn refresh(&mut self) -> Vec<AlarmTransition> {
        self.restart_watch().await;
        let r = self.refresh_plan().run().await;
        self.fold(r)
    }

    /// What a refresh read, folded: presence, the archives, then the GET's
    /// replies and its end; then the receive path's watch and every key's
    /// verdict. Returns every transition it made.
    pub fn fold(&mut self, r: Refreshed) -> Vec<AlarmTransition> {
        let (now, wall) = (Instant::now(), SystemTime::now());
        let mut out = Vec::new();
        match r.presence {
            None => self.log.presence = None,
            Some((at, Ok(o))) => {
                out.extend(self.book.presence_read(&o, at));
                let scope = self.book.scope();
                let catalog = Catalog::new(&o);
                let present: Vec<_> = catalog
                    .addresses()
                    .filter(|a| scope.selects_owner(a) && catalog.has_instance(a))
                    .collect();
                let listing = present
                    .iter()
                    .filter(|a| {
                        matches!(
                            self.book.presence_of(a),
                            zenkey_model::alarms::Presence::Present(
                                zenkey_model::alarms::Listing::Listed { .. }
                            )
                        )
                    })
                    .count();
                self.log.presence = Some(Ok(PresenceSeen {
                    selector: o.selector.clone(),
                    complete: o.complete,
                    services: present.len(),
                    listing,
                }));
            }
            Some((_, Err(e))) => self.log.presence = Some(Err(e)),
        }
        match r.archived {
            None => {}
            Some(Ok((asked, found))) => {
                self.book.archived(found);
                self.log.archives = Some(Ok(asked));
            }
            Some(Err(e)) => self.log.archives = Some(Err(e)),
        }
        // A receive path that could not be watched may have lost anything
        // before this GET: a loss, which the GET heals for the keys it
        // answers and from which every window restarts (core §2.7, 0.27).
        if matches!(self.stall, Some(Err(_))) {
            out.extend(self.book.lose(
                AlarmLoss {
                    dropped: 0,
                    stalled: true,
                },
                now,
                wall,
            ));
        }
        match r.seed {
            Ok(read) => {
                self.log.seed = Some(Ok(read.complete));
                self.log.discarded += read.discarded;
                out.extend(self.book.seed(read, now, wall));
            }
            Err(e) => self.log.seed = Some(Err(e)),
        }
        out.extend(self.check_stall(now, wall));
        out.extend(self.book.sweep(now, wall));
        out
    }

    /// The next delivery, folded: a loss first, when the queue refused
    /// some since, then the delivery. `None` once the subscriptions are
    /// gone, or when the feed does not listen.
    pub async fn next(&mut self) -> Option<Vec<AlarmTransition>> {
        self.restart_watch().await;
        let d = self.rx.as_mut()?.recv().await?;
        let (now, wall) = (Instant::now(), SystemTime::now());
        let mut out = Vec::new();
        let refused = self.refused.swap(0, Ordering::Relaxed);
        if refused > 0 {
            out.extend(self.book.lose(
                AlarmLoss {
                    dropped: refused,
                    stalled: false,
                },
                now,
                wall,
            ));
        }
        match d {
            Delivery::Sample(s) => out.extend(self.book.sample(s)),
            Delivery::Instance { key, held, at } => {
                if let ZkKey::Instance { addr, instance } = key {
                    self.book.instance(&addr, &instance, held, at);
                }
            }
        }
        out.extend(self.check_stall(now, wall));
        Some(out)
    }

    /// Every key's verdict at this instant: what moved with no sample.
    pub fn sweep(&mut self) -> Vec<AlarmTransition> {
        let (now, wall) = (Instant::now(), SystemTime::now());
        let mut out = self.check_stall(now, wall);
        out.extend(self.book.sweep(now, wall));
        out
    }

    /// A watch restarted after it saw a stall, so that the next one is seen
    /// too.
    async fn restart_watch(&mut self) {
        if self.restart_stall {
            // Cleared only once the new watch stands: a drain that drops
            // this future mid-start retries it on its next call.
            let watch = StallWatch::start(&self.session).await;
            self.stall = Some(watch);
            self.stall_seen = StallSeen::default();
            self.restart_stall = false;
        }
    }

    /// The receive path, read: a stall past the bound, or a transport of
    /// this session closed, is a loss (core §2.7, 0.27). A path that could
    /// not be watched is a loss at every refresh ([`AlarmFeed::fold`]).
    fn check_stall(&mut self, now: Instant, wall: SystemTime) -> Vec<AlarmTransition> {
        let stalled = match &self.stall {
            None | Some(Err(_)) => false,
            Some(Ok(w)) => {
                let seen = w.read();
                let bound = self.spec.stall_bound();
                let fresh = (seen.gap > bound && self.stall_seen.gap <= bound)
                    || seen.transports_closed > self.stall_seen.transports_closed;
                self.stall_seen = seen;
                if fresh {
                    self.restart_stall = true;
                }
                fresh
            }
        };
        if stalled {
            self.book.lose(
                AlarmLoss {
                    dropped: 0,
                    stalled: true,
                },
                now,
                wall,
            )
        } else {
            Vec::new()
        }
    }

    /// Deliveries discarded on a non-concrete key (R6), so far.
    pub fn discarded(&self) -> u64 {
        self.discarded.load(Ordering::Relaxed) + self.log.discarded
    }
}

/// One reading of a scope's alarms, as values: the book, what its reads
/// found, and when it was judged.
#[derive(Debug, Clone)]
pub struct AlarmObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub spec: AlarmSpec,
    /// The selector asked.
    pub selector: String,
    /// The book, as the reading left it; an `Err` when the reader could not
    /// be opened.
    pub book: Result<AlarmBook, String>,
    pub log: ReadLog,
    pub asked: Vec<String>,
    /// How long the subscription listened, when one did.
    pub listened: Option<Duration>,
    pub at: Instant,
    pub wall: SystemTime,
}

/// One reading: [`observe`], then [`crate::judge::alarms::judge`], at
/// `at_least`.
pub async fn run_alarms(
    session: &Session,
    namespace: &str,
    scope: AlarmScope,
    spec: AlarmSpec,
    at_least: zenkey_model::alarms::Severity,
) -> crate::report::AlarmsReport {
    let obs = observe(session, namespace, scope, spec).await;
    crate::judge::alarms::judge(&obs, at_least)
}

/// The one-shot reading of the module doc: the feed opened (listening when
/// `spec.window` is set), drained over the window and the lease and reply
/// timeout after it, its owners watched throughout.
pub async fn observe(
    session: &Session,
    namespace: &str,
    scope: AlarmScope,
    spec: AlarmSpec,
) -> AlarmObservation {
    let selector = scope.selector().to_owned();
    let started = Instant::now();
    let opened = AlarmFeed::open(session, scope, spec, spec.window.is_some()).await;
    let mut feed = match opened {
        Ok((feed, _)) => feed,
        Err(e) => {
            return AlarmObservation {
                namespace: namespace.to_owned(),
                spec,
                selector,
                book: Err(crate::one_line(&e)),
                log: ReadLog::default(),
                asked: Vec::new(),
                listened: None,
                at: Instant::now(),
                wall: SystemTime::now(),
            };
        }
    };
    let mut listened = None;
    if let Some(w) = spec.window {
        let end = started + w + spec.book().tail();
        let deadline = tokio::time::Instant::from_std(end);
        loop {
            tokio::select! {
                () = tokio::time::sleep_until(deadline) => break,
                d = feed.next() => if d.is_none() { break },
            }
        }
        listened = feed.book().declared().map(|d| d.elapsed());
        feed.sweep();
    }
    AlarmObservation {
        namespace: namespace.to_owned(),
        spec,
        selector,
        asked: feed.asked(),
        log: feed.log().clone(),
        book: Ok(feed.book().clone()),
        listened,
        at: Instant::now(),
        wall: SystemTime::now(),
    }
}
