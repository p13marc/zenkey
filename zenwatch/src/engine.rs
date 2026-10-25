//! The run loop: four observers, one router, N sinks — on two sessions
//! (#614, FL2).
//!
//! The fleet's zk2 `watchdog` Straw judges the eight engine conditions
//! every tick — its wire selectors on the session in no namespace, its
//! contracts, presence, doctor and `alert-firing`'s alarms through the one
//! in the namespace — and yields a [`Transition`] per genuine change. The
//! fleet's live presence projection ([`zenkey_fleet::PresenceFeed`], its
//! own monitor on the namespaced session) carries `instance-gone` and
//! `liveliness-gone` on the deployment's zk2 tokens ([`crate::presence`]);
//! the daemon's own token ledger ([`crate::liveliness`], its own monitor on
//! the raw session) carries `liveliness-gone` on anything else. A monitor
//! each, because a `Dropped(n)` is one receiver's lag: a flood on one plane
//! must not make another's tokens unobservable. The fleet's live `alarms.v1`
//! readers ([`zenkey_fleet::AlarmFeed`], one per distinct `alerts`
//! selector, on the namespaced session, #800 AD) carry `alerts`
//! ([`crate::alarms`]): their deliveries drain here, their re-reads run
//! beside the drain each tick, and their sweep runs on the tick. All four
//! feed one [`Notice`] stream,
//! [`route`] turns a notice into one [`Outgoing`] per matching rule, and
//! every named sink gets it concurrently — a slow sink widens nothing, a
//! failed sink is counted and logged, never fatal.
//!
//! **Three states, preserved.** `ok` / `firing` / `unobservable` ride from
//! the engine's `CondState` into the sink payload unchanged. A presence,
//! token or alarm rule whose observer lost events, or whose read ran to its
//! timeout, says `unobservable` once, on its `observer` identity — never
//! `ok` (RFC 13 §3 O6).
//!
//! **Transitions, not states.** A token re-declared is not a notification;
//! an address that was already down going down is not one either. What
//! each rule last said lives with it, and what the discipline announced
//! lives in its bounded ledger.
//!
//! **Between [`route`] and delivery sits the [`Discipline`]** (#389): every
//! routed [`Outgoing`] is observed, and only what the tick flushes — after
//! `for`, dedup, grouping and repeat — is dispatched. The tick also
//! persists the ledger and brings what this daemon publishes about itself
//! up to date ([`crate::publish`], a zk2 service since FL3): its
//! `firing/*` documents, its counters, and its `health.v1` checks.
//!
//! **Its own stop is not news** (#614, FL3). The service comes up after
//! every observer is watching and seeded, and goes last: on a stop, the
//! final tick flushes, the observers stop, the sinks drain, and only then
//! does the service delete its `firing/*` documents and withdraw its
//! tokens. A rule over this daemon's own address (`instance-gone */*`) is
//! not excluded — two zenwatches watching each other is a deployment — and
//! by then no observer is left to see it go, so the daemon never pages
//! about its own clean stop, nor records itself gone in the state file.
//!
//! **The scheduled doctor runs beside the drain** (#390, [`crate::doctor`]):
//! an interval of hours starts zk2's `run_doctor` as a future the loop
//! selects on, so a sweep that takes seconds never stops sampling. Its
//! outcome becomes [`Notice::Doctor`]s on the same stream — routed under
//! the `doctor` block's own rule ([`RuleKind::ScheduledDoctor`]),
//! disciplined like every notice, and published after every run.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use zenkey_fleet::{
    BundleStore, CondState, ContractSet, DoctorBus, DoctorReport, LivePresence, Monitor,
    MonitorSpec, PresenceFeed, PresenceScope, PresenceTransition, Sipper as _, Transition,
    WatchdogSpec, WatchdogSummary,
};

use crate::alarms::{AlarmRules, AlertFacts, Remembered};
use crate::config::{DisciplineConfig, DoctorConfig};
use crate::discipline::{Counters as DisciplineCounters, Discipline, NoticeMeta, state as ledger};
use crate::doctor::{self, DoctorNotice};
use crate::exit::unaskable;
use crate::liveliness::{LivelinessRules, TokenFeed};
use crate::presence::PresenceRules;
use crate::publish::{
    self, DoctorCheck, HealthInputs, Observer, SelfService, SinkOutcome, ZenwatchCounters,
};
use crate::render::{Draft, Payload, RenderConfig, RenderSource};
use crate::rules::{DOCTOR_RULE, Rule, RuleKind};
use crate::sinks::{NoticeKind, Notification, Outgoing, Sink};

/// One thing the observers saw that might be worth telling someone.
#[derive(Debug, Clone)]
pub enum Notice {
    /// The engine judged a condition into a different state.
    Engine(Transition),
    /// A presence, token or alarm rule said something about one identity,
    /// its observer, or its first look (#614, FL2; #800, AD). Already the
    /// rule's: the projection, the ledger and the alarm readers fold per
    /// rule. Boxed: it carries the presence move or the alarm's change that
    /// decided it.
    Rule(Box<RuleNotice>),
    /// One thing a scheduled doctor run said (#390): the baseline, a
    /// finding new or fixed since the last run, or the run itself failing
    /// or recovering.
    Doctor(Box<DoctorNotice>),
}

/// One presence, token or alarm rule's notice: the identity, its state, and
/// the facts its rendering and the discipline need.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleNotice {
    /// The rule's name.
    pub rule: String,
    /// The identity under the rule: an address, an address and what a
    /// token names, a token's wire key, an alarm's key, `observer`,
    /// `baseline`.
    pub what: String,
    /// The notice's own kind: what a passthrough carries as is; a
    /// remembered identity's is the discipline's verdict.
    pub kind: NoticeKind,
    pub state: CondState,
    /// What the rule last said of the identity; `None` on first sight.
    pub prior: Option<CondState>,
    /// The severity, when it is not the rule's (an `info` baseline, an
    /// alarm's document).
    pub severity: Option<String>,
    pub evidence: String,
    /// Lines the message carries after the evidence (a baseline's list).
    pub lines: Vec<String>,
    /// The token's wire key, for a token rule.
    pub key: Option<String>,
    /// The address the notice is about, when it is about one.
    pub address: Option<String>,
    /// RFC 3339 wall clock.
    pub at: String,
    /// The presence move that decided it, as the engine pins it.
    pub presence: Option<PresenceTransition>,
    /// A fact that predates the watch: remembered, delivered to nobody.
    pub assume: bool,
    /// No identity to remember: delivered every time.
    pub passthrough: bool,
    /// Why no payload is rendered.
    pub reason: &'static str,
    /// An alarm's change and its document (`alerts`, #800 AD): the
    /// notification's `alert`, the payload rendered through the contract.
    pub alert: Option<Box<AlertFacts>>,
}

/// What the discipline needs to know about a notice that its rendering
/// does not carry: the address it is about, whether it has an identity,
/// and whether it is a fact to remember rather than tell.
pub fn meta_of(notice: &Notice) -> NoticeMeta {
    match notice {
        Notice::Engine(_) => NoticeMeta::default(),
        Notice::Rule(n) => NoticeMeta {
            address: n.address.clone(),
            passthrough: n.passthrough,
            assume: n.assume,
        },
        Notice::Doctor(d) => NoticeMeta {
            address: None,
            passthrough: d.is_passthrough(),
            assume: d.is_assumed(),
        },
    }
}

/// One notice → one [`Outgoing`] per rule it matches. Pure: the router
/// holds no state.
///
/// The notification's `id` is the notice **identity** (#389): the rule's id
/// and what the notice is about — an address, a token's key, an alarm's
/// key, `observer`, an engine condition's sorted labels, a doctor finding's
/// check and subject — stable across ticks, runs and restarts, which is
/// what dedup and the state file key on.
pub fn route(notice: &Notice, rules: &[Rule], render: &RenderConfig) -> Vec<Outgoing> {
    let mut out = Vec::new();
    for r in rules {
        match (notice, &r.kind) {
            // The scheduled doctor renders its own notices (#390): a finding
            // is a judgement over a sweep, not a payload on a key.
            (Notice::Doctor(d), RuleKind::ScheduledDoctor) => {
                out.push(doctor::outgoing(d, r, render));
            }
            (Notice::Engine(t), RuleKind::Engine(c)) if c.to_string() == t.rule => {
                let what = r
                    .labels
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(",");
                let payload = Payload::None {
                    reason: "an engine condition is judged over a window, not a payload".into(),
                };
                let rendered = crate::render::render(
                    &Draft {
                        rule: &r.name,
                        state: t.to,
                        prior: t.from,
                        severity: &r.severity,
                        evidence: &t.evidence,
                        extra: &[],
                        labels: &r.labels,
                        key: c.selector(),
                        timestamp: None,
                        payload: &payload,
                    },
                    render,
                );
                out.push(outgoing(
                    r,
                    format!("{}:{what}", r.id),
                    NoticeKind::Transition,
                    t.to,
                    t.from,
                    r.severity.clone(),
                    rendered,
                    t.at.clone(),
                    t.evidence.clone(),
                    None,
                ));
            }
            (Notice::Rule(n), _) if n.rule == r.name => {
                let severity = n.severity.clone().unwrap_or_else(|| r.severity.clone());
                let mut extra = Vec::new();
                if let Some(a) = &n.address {
                    extra.push(format!("address: {a}"));
                }
                extra.extend(n.lines.iter().cloned());
                // An alert's document is rendered through the contract that
                // declares it (alarms.v1's, strict, §2.4), its discriminating
                // labels over the rule's (#800, AD).
                let mut labels = r.labels.clone();
                let payload = match &n.alert {
                    Some(a) => {
                        labels.extend(a.labels());
                        match &a.document {
                            Some(d) => Payload::Typed(
                                serde_json::to_value(d).unwrap_or(serde_json::Value::Null),
                            ),
                            None => Payload::None {
                                reason: n.reason.into(),
                            },
                        }
                    }
                    None => Payload::None {
                        reason: n.reason.into(),
                    },
                };
                let rendered = crate::render::render(
                    &Draft {
                        rule: &r.name,
                        state: n.state,
                        prior: n.prior,
                        severity: &severity,
                        evidence: &n.evidence,
                        extra: &extra,
                        labels: &labels,
                        key: n.key.as_deref(),
                        timestamp: n.alert.as_ref().and_then(|a| a.timestamp()),
                        payload: &payload,
                    },
                    render,
                );
                let rendering = rendered.source;
                let mut o = outgoing(
                    r,
                    format!("{}:{}", r.id, n.what),
                    n.kind,
                    n.state,
                    n.prior,
                    severity,
                    rendered,
                    n.at.clone(),
                    n.evidence.clone(),
                    n.presence.clone(),
                );
                o.notification.labels = labels;
                o.notification.rendering = rendering;
                o.notification.alert = n.alert.as_ref().map(|a| a.transition.clone());
                out.push(o);
            }
            _ => {}
        }
    }
    out
}

/// A rendered notice as the [`Outgoing`] its rule's sinks get.
#[allow(clippy::too_many_arguments)]
fn outgoing(
    r: &Rule,
    id: String,
    kind: NoticeKind,
    state: CondState,
    prior: Option<CondState>,
    severity: String,
    rendered: crate::render::Rendered,
    at: String,
    evidence: String,
    presence: Option<PresenceTransition>,
) -> Outgoing {
    Outgoing {
        notification: Notification {
            id,
            rule: r.name.clone(),
            rule_kind: r.kind.head().to_string(),
            kind,
            state,
            prior,
            severity,
            title: rendered.title,
            message: rendered.message,
            labels: r.labels.clone(),
            at,
            evidence,
            rendering: RenderSource::KeyOnly,
            truncated: rendered.truncated,
            repeat: 0,
            group: None,
            presence,
            alert: None,
        },
        sinks: r.sinks.clone(),
    }
}

/// What one run watched and did.
#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    pub notices: u64,
    pub outgoing: u64,
    pub delivered: u64,
    pub failed: u64,
    /// Liveliness events the presence and token monitors dropped on this
    /// consumer (each one an observer gone unobservable until a re-read).
    pub dropped: u64,
    /// Address records the presence projection forgot at its bound.
    pub evicted: u64,
    /// Deliveries the `alerts` rules' readers lost, and their stalls (each
    /// one their observer gone unobservable until a GET heals it, #800 AD).
    pub alarms_lost: u64,
    pub watchdog: Option<WatchdogSummary>,
    /// What the discipline did (#389).
    pub discipline: DisciplineCounters,
    /// The address this daemon's own service ran at (`<system>/<service>`,
    /// resolved), when it ran one (#614, FL3).
    pub self_address: Option<String>,
    /// `firing/*` documents the runtime refused at the bound (core §2.7).
    pub firing_refused: u64,
    /// Scheduled doctor runs, attempted (#390).
    pub doctor_runs: u64,
}

/// How the daemon publishes itself (#614, FL3): its service's
/// configuration, and the `hostid.v1` minter its address resolves through —
/// the process's ([`zenkey::HostIdMinter::global`]) in a daemon, a test's
/// on a root of its own.
pub struct Publish<'a> {
    pub config: zenkey::ServiceConfig,
    pub hostid: &'a zenkey::HostIdMinter,
}

/// Everything a run needs, with the sessions already open — the shape the
/// bus tests drive directly.
pub struct Engine<'a> {
    /// The two sessions: in the deployment's namespace, and in none; and
    /// the deployment's watch session, in no namespace on its wire keys,
    /// when a rule follows presence live (#830, #875).
    pub bus: &'a DoctorBus,
    /// The raw watch session the foreign-token ledger's monitor holds
    /// (#830), when a `liveliness-gone` rule names a foreign token: the bus
    /// root's, whose keys are the wire's.
    pub raw_watch: Option<&'a zenkey_fleet::WatchSession>,
    /// The contract store, seeded with `contracts`: a revision held
    /// offline is never retrieved (spec §8.5).
    pub store: &'a BundleStore,
    /// The contracts known offline (`contracts`, `--contracts`).
    pub contracts: &'a ContractSet,
    pub timeout: Duration,
    pub tick: Duration,
    pub rules: &'a [Rule],
    pub sinks: Arc<Vec<Sink>>,
    pub render: &'a RenderConfig,
    /// One tick, then stop.
    pub once: bool,
    /// Fired once every observer is watching and seeded — the test seam
    /// that lets a bus test act *after* the observer is watching (O4: a
    /// sample the observer was not yet declared for is not a sample it
    /// missed).
    pub ready: Option<tokio::sync::oneshot::Sender<()>>,
    /// The notification discipline (#389).
    pub discipline: &'a DisciplineConfig,
    /// Where the ledger survives a restart; `None` keeps it in memory.
    pub state_file: Option<PathBuf>,
    pub state_max_entries: usize,
    /// The daemon's own zk2 service (#614, FL3) — a real daemon runs one
    /// unless told `publish: false`; a bus test that is not about
    /// self-publication does not.
    pub publish: Option<Publish<'a>>,
    /// The scheduled doctor (#390); `None` is not scheduled — nothing runs,
    /// and the service lists `state/doctor` unavailable.
    pub doctor: Option<&'a DoctorConfig>,
    /// The operator's word on the clocks (`freshness.v1` §2.6), for the
    /// doctor's `health-*` checks, scheduled or a rule's.
    pub clocks_synced: bool,
}

/// Delivery counters shared with the spawned deliveries, and each sink's
/// last delivery — what the `sinks` health check reads (#614, FL3).
#[derive(Default)]
struct Counters {
    delivered: AtomicU64,
    failed: AtomicU64,
    last: std::sync::Mutex<BTreeMap<String, SinkOutcome>>,
}

impl Counters {
    fn record(&self, sink: &str, error: Option<String>) {
        match error {
            None => self.delivered.fetch_add(1, Ordering::Relaxed),
            Some(_) => self.failed.fetch_add(1, Ordering::Relaxed),
        };
        self.last.lock().expect("not poisoned").insert(
            sink.to_string(),
            SinkOutcome {
                at: zenkey_fleet::rfc3339_now(),
                error,
            },
        );
    }

    /// Every configured sink, with its last delivery.
    fn by_sink(&self, sinks: &[Sink]) -> BTreeMap<String, Option<SinkOutcome>> {
        let last = self.last.lock().expect("not poisoned");
        sinks
            .iter()
            .map(|s| (s.name.clone(), last.get(&s.name).cloned()))
            .collect()
    }
}

/// Fan one outgoing to its sinks, each on its own task, each bounded by its
/// sink's timeout. Nothing here can fail the run.
fn dispatch(
    tasks: &mut tokio::task::JoinSet<()>,
    sinks: &Arc<Vec<Sink>>,
    counters: &Arc<Counters>,
    outgoing: Outgoing,
) {
    let outgoing = Arc::new(outgoing);
    for (i, sink) in sinks.iter().enumerate() {
        if !outgoing.sinks.iter().any(|n| n == &sink.name) {
            continue;
        }
        let (sinks, counters, outgoing) = (
            Arc::clone(sinks),
            Arc::clone(counters),
            Arc::clone(&outgoing),
        );
        tasks.spawn(async move {
            let sink = &sinks[i];
            let n = &outgoing.notification;
            match sink.deliver(&outgoing).await {
                Ok(d) => {
                    counters.record(&sink.name, None);
                    tracing::info!(sink = %sink.name, kind = sink.kind_name(), rule = %n.rule,
                        state = crate::render::state_word(n.state), id = %n.id, "delivered: {}", d.detail);
                }
                Err(e) => {
                    counters.record(&sink.name, Some(e.to_string()));
                    tracing::warn!(sink = %sink.name, kind = sink.kind_name(), rule = %n.rule,
                        state = crate::render::state_word(n.state), id = %n.id, "delivery failed: {e}");
                }
            }
        });
    }
}

/// Unix seconds now, as the discipline's clock.
fn now_s() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// A liveliness monitor's spec: no data plane, no tree ticks (the feeds
/// read tokens only, and a tick on the broadcast is a slot a token needs).
fn liveliness_spec(liveliness: Vec<String>) -> MonitorSpec {
    MonitorSpec {
        liveliness,
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
}

/// Run until `stop` resolves (or, with `once`, until one tick has been
/// evaluated). Returns the summary; delivery failures are in it, never an
/// `Err`.
pub async fn run_on(e: Engine<'_>, stop: impl Future<Output = ()>) -> Result<RunSummary> {
    let mut summary = RunSummary::default();
    let started_at = zenkey_fleet::rfc3339_now();
    let namespace = e.bus.namespace.as_str();

    // The rules the router and the discipline see: the configured ones,
    // plus the `doctor` block as a rule of its own (#390), so a doctor
    // finding is deduplicated, repeated, restored and published under a
    // rule like every notice.
    let rules_all: Vec<Rule> = e
        .rules
        .iter()
        .cloned()
        .chain(e.doctor.map(Rule::scheduled_doctor))
        .collect();
    let rule_ids: BTreeMap<String, String> = rules_all
        .iter()
        .map(|r| (r.name.clone(), r.id.clone()))
        .collect();

    // The ledger first: a state file this process cannot read — malformed,
    // version 1, another namespace's — is a refusal before anything opens
    // (silently starting fresh is how a resolved notice re-pages).
    let mut discipline = Discipline::new(e.discipline, &rules_all, e.render, e.state_max_entries);
    if let Some(path) = &e.state_file {
        match ledger::load(path, namespace) {
            Ok(Some(saved)) => {
                let evicted = saved.evicted_total;
                let n = discipline.restore(saved);
                tracing::info!(path = %path.display(), entries = n, evicted_total = evicted,
                    "state file loaded");
            }
            Ok(None) => tracing::info!(path = %path.display(), "no state file yet — fresh ledger"),
            Err(err) => return Err(unaskable!("{err}")),
        }
    }

    // The daemon's own address, minted before anything is declared
    // (hostid.v1 §2.7): a host with no id refuses here, with every way out
    // named (§2.6), and nothing has opened. Once per process: the service
    // brought up below gets the same system.
    if let Some(p) = &e.publish
        && let Err(err) = p.config.resolve_with(p.hostid)
    {
        return Err(unaskable!("{}", publish::unminted(&err)));
    }

    // One notice into the discipline: routed under every rule it matches.
    let observe = |discipline: &mut Discipline, summary: &mut RunSummary, n: Notice| {
        summary.notices += 1;
        let meta = meta_of(&n);
        let now = now_s();
        for o in route(&n, &rules_all, e.render) {
            discipline.observe(o, &meta, now);
        }
    };

    // The engine's seven, on the fleet's zk2 watchdog.
    let engine_rules: Vec<_> = e
        .rules
        .iter()
        .filter_map(|r| match &r.kind {
            RuleKind::Engine(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let spec = WatchdogSpec {
        rules: engine_rules.clone(),
        tick: e.tick,
        ticks: e.once.then_some(1),
        timeout: e.timeout,
        clocks_synced: e.clocks_synced,
    };
    let mut watchdog = (!engine_rules.is_empty())
        .then(|| zenkey_fleet::watchdog(e.bus, e.store, e.contracts, &spec).pin());
    let mut watchdog_done = watchdog.is_none();

    // Presence, live (#614): one projection of the whole deployment, its
    // own monitor on the deployment's watch session, seeded before `ready`.
    let mut presence_rules = PresenceRules::new(e.rules, namespace);
    let mut presence_monitor: Option<Monitor> = None;
    let mut presence_feed: Option<PresenceFeed> = None;
    if !presence_rules.is_empty() {
        let scope = PresenceScope::all();
        // Its liveliness subscribers on the deployment's watch session, in
        // no namespace on the wire keys (#875), its reads on the namespaced
        // session (#830, core §8.1, 0.37).
        let watch = e.bus.watch.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "the presence rules need a watch session for the namespace: no presence read \
                 is made beside a liveliness subscriber (#830)"
            )
        })?;
        let monitor =
            Monitor::start_watch(watch, liveliness_spec(Vec::new()).with_presence(&scope)).await?;
        let mut live = LivePresence::new(scope);
        presence_rules.track(&mut live, SystemTime::now());
        let (feed, _seeded) = PresenceFeed::open(&e.bus.session, &monitor, live, e.timeout).await?;
        tracing::info!(seed = ?feed.last_seed(), basis = ?feed.presence().basis(),
            "presence: the first look");
        let announced: BTreeMap<String, Vec<String>> = presence_rules
            .names()
            .map(|n| (n.to_string(), discipline.announced_under(n)))
            .collect();
        let at = zenkey_fleet::rfc3339_now();
        for n in presence_rules.baseline(feed.presence(), &announced, &rule_ids, &at) {
            observe(&mut discipline, &mut summary, Notice::Rule(Box::new(n)));
        }
        presence_feed = Some(feed);
        presence_monitor = Some(monitor);
    }

    // Foreign tokens: the daemon's own ledger, its own monitor on the raw
    // session, seeded before `ready`.
    let mut token_rules = LivelinessRules::new(e.rules, namespace);
    let mut token_monitor: Option<Monitor> = None;
    let mut token_feed: Option<TokenFeed> = None;
    if !token_rules.is_empty() {
        let selectors = token_rules.selectors();
        // Its monitor on the raw watch session, its reads on the raw
        // session (#830, core §8.1, 0.37).
        let watch = e.raw_watch.ok_or_else(|| {
            anyhow::anyhow!(
                "the foreign-token rules need a raw watch session: no liveliness read is made \
                 beside a liveliness subscriber (#830)"
            )
        })?;
        let monitor = Monitor::start_watch(watch, liveliness_spec(selectors.clone())).await?;
        let feed = TokenFeed::open(&e.bus.raw, &monitor, selectors, e.timeout).await?;
        let announced: BTreeMap<String, Vec<String>> = e
            .rules
            .iter()
            .map(|r| (r.name.clone(), discipline.announced_under(&r.name)))
            .collect();
        let at = zenkey_fleet::rfc3339_now();
        for n in token_rules.baseline(feed.held(), feed.basis(), &announced, &rule_ids, &at) {
            observe(&mut discipline, &mut summary, Notice::Rule(Box::new(n)));
        }
        token_feed = Some(feed);
        token_monitor = Some(monitor);
    }
    // Alarms, live (#800, AD): one reader per distinct `alerts` selector,
    // on the namespaced session, subscribed and seeded before `ready`; its
    // first look told as `crate::alarms` decides, reconciled against what
    // the ledger remembers announced.
    let mut alarm_rules = AlarmRules::new(e.rules);
    if !alarm_rules.is_empty() {
        let initial = alarm_rules
            .open(
                &e.bus.session,
                e.bus.watch.as_ref(),
                e.timeout,
                e.clocks_synced,
            )
            .await;
        let mut remembered: BTreeMap<String, Vec<Remembered>> = BTreeMap::new();
        for entry in discipline.announced() {
            remembered
                .entry(entry.rule.clone())
                .or_default()
                .push(Remembered {
                    id: entry.id.clone(),
                    state: entry.state,
                    severity: entry.severity.clone(),
                });
        }
        let sight = alarm_rules.sight();
        tracing::info!(rules = ?alarm_rules.names().collect::<Vec<_>>(), ?sight,
            "alarms: the first look");
        let at = zenkey_fleet::rfc3339_now();
        for n in alarm_rules.baseline(&initial, &remembered, &at) {
            observe(&mut discipline, &mut summary, Notice::Rule(Box::new(n)));
        }
    }
    type AlarmRefreshRun =
        Pin<Box<dyn Future<Output = Vec<(usize, zenkey_fleet::AlarmRefreshed)>> + Send>>;
    let mut alarm_refresh: Option<AlarmRefreshRun> = None;
    let dropped_now = |p: &Option<Monitor>, t: &Option<Monitor>| {
        p.as_ref().map_or(0, |m| m.core().dropped()) + t.as_ref().map_or(0, |m| m.core().dropped())
    };
    // What each observer's basis lets its rules conclude, for the
    // `presence` and `tokens` checks: `None` when no rule reads it, and a
    // feed gone is a monitor that stopped.
    let had_presence = !presence_rules.is_empty();
    let had_tokens = !token_rules.is_empty();
    let observers = |p: &Option<PresenceFeed>, t: &Option<TokenFeed>| {
        (
            had_presence.then(|| {
                p.as_ref()
                    .map_or(Observer::Stopped, |f| f.presence().basis().into())
            }),
            had_tokens.then(|| t.as_ref().map_or(Observer::Stopped, |f| f.basis().into())),
        )
    };
    let doctor_every = e.doctor.map(DoctorConfig::every_spelled);

    let counters = Arc::new(Counters::default());
    // The last persist error, which the `state-file` check reads: a record,
    // so the final tick's write is not a dead store.
    struct Pulse {
        last_persist_error: Option<String>,
    }
    let mut pulse = Pulse {
        last_persist_error: None,
    };

    // The daemon as a zk2 service (#614, FL3), on the namespaced session:
    // after the watches, before `ready`, so a test that sees its instance
    // token can already read it; its status, checks and counters put
    // before its tokens (health.v1 §2.3, core §8.2).
    macro_rules! counters_now {
        ($firing_refused:expr) => {{
            let announced = |s: CondState| discipline.announced().filter(|x| x.state == s).count();
            ZenwatchCounters {
                started_at: started_at.clone(),
                rules: e.rules.len(),
                sinks: e.sinks.len(),
                firing: announced(CondState::Firing),
                unobservable: announced(CondState::Unobservable),
                delivered: counters.delivered.load(Ordering::Relaxed),
                deliveries_failed: counters.failed.load(Ordering::Relaxed),
                dropped: summary.dropped,
                forgotten: presence_feed
                    .as_ref()
                    .map_or(0, |f| f.presence().forgotten()),
                state_entries: discipline.len(),
                firing_refused: $firing_refused,
                doctor_runs: summary.doctor_runs,
                discipline: discipline.counters(),
            }
        }};
    }
    let mut publisher: Option<SelfService> = match e.publish {
        None => None,
        Some(p) => {
            let (presence, tokens) = observers(&presence_feed, &token_feed);
            let by_sink = counters.by_sink(&e.sinks);
            let checks = publish::checks(&HealthInputs {
                sinks: &by_sink,
                presence,
                tokens,
                alarms: alarm_rules.sight(),
                doctor: doctor_every.as_deref().map(|every| DoctorCheck {
                    status: publish::DoctorStatus::Pending,
                    every,
                    last: None,
                }),
                state_file: e.state_file.as_ref().map(|_| None),
                firing_refused: 0,
            });
            let plan = publish::Plan {
                rules: e.rules.len(),
                sinks: e.sinks.len(),
                doctor: e.doctor.is_some(),
            };
            summary.dropped = dropped_now(&presence_monitor, &token_monitor);
            let initial = counters_now!(0);
            match SelfService::start(&e.bus.session, p.config, p.hostid, &plan, &checks, &initial)
                .await
            {
                Ok(s) => {
                    summary.self_address = Some(s.address().to_string());
                    Some(s)
                }
                Err(publish::StartError::Unminted(err)) => {
                    return Err(unaskable!("{}", publish::unminted(&err)));
                }
                Err(err) => return Err(err.into()),
            }
        }
    };
    if let Some(ready) = e.ready {
        let _ = ready.send(());
    }

    // The scheduled doctor (#390): the first run one interval after start
    // — the fleet just stood up is the fleet CI already checked — or at
    // once under `--once`, which is a smoke run. A run in flight is a
    // future the loop selects on beside the drain; a tick that lands while
    // one runs waits for it (`Delay`), never stacks a second sweep.
    let mut doctor_schedule = e
        .doctor
        .map(|cfg| doctor::Schedule::new(cfg, e.timeout, e.clocks_synced));
    let mut doctor_tick = doctor_schedule.as_ref().map(|s| {
        let start = if e.once {
            tokio::time::Instant::now()
        } else {
            tokio::time::Instant::now() + s.every()
        };
        let mut i = tokio::time::interval_at(start, s.every());
        i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        i
    });
    type DoctorRun<'f> =
        Pin<Box<dyn Future<Output = std::result::Result<DoctorReport, String>> + Send + 'f>>;
    let mut doctoring: Option<DoctorRun<'_>> = None;
    let mut disc_tick = tokio::time::interval(e.tick);
    disc_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    disc_tick.tick().await;
    let once_deadline = tokio::time::sleep(e.tick + Duration::from_millis(250));
    tokio::pin!(once_deadline);
    let mut stop = std::pin::pin!(stop);

    let mut tasks = tokio::task::JoinSet::new();

    // One tick of the discipline: flush, dispatch, persist, publish.
    macro_rules! discipline_tick {
        ($force:expr) => {{
            let now = now_s();
            let flush = discipline.tick(now, $force);
            for o in flush.outgoing {
                summary.outgoing += 1;
                dispatch(&mut tasks, &e.sinks, &counters, o);
            }
            if let Some(path) = &e.state_file
                && discipline.dirty()
            {
                let snapshot = discipline.snapshot(now, namespace);
                match ledger::save(path, &snapshot) {
                    Ok(()) => pulse.last_persist_error = None,
                    Err(err) => {
                        tracing::warn!("{err}");
                        pulse.last_persist_error = Some(err.to_string());
                    }
                }
            }
            summary.dropped = dropped_now(&presence_monitor, &token_monitor);
            if let Some(p) = publisher.as_mut() {
                // `firing/*` first, so the `firing` check reads what the
                // bound refuses after this tick's puts.
                let docs = publish::firing_docs(&discipline);
                if let Err(err) = p.publish_firing(&docs).await {
                    tracing::warn!("firing/* publish failed: {err}");
                }
                let c = counters_now!(p.firing_refused());
                if let Err(err) = p.publish_counters(&c).await {
                    tracing::warn!("counters publish failed: {err}");
                }
                let (presence, tokens) = observers(&presence_feed, &token_feed);
                let by_sink = counters.by_sink(&e.sinks);
                let checks = publish::checks(&HealthInputs {
                    sinks: &by_sink,
                    presence,
                    tokens,
                    alarms: alarm_rules.sight(),
                    doctor: doctor_schedule.as_ref().zip(doctor_every.as_deref()).map(
                        |(s, every)| DoctorCheck {
                            status: s.status(),
                            every,
                            last: s.document(),
                        },
                    ),
                    state_file: e
                        .state_file
                        .as_ref()
                        .map(|_| pulse.last_persist_error.as_deref()),
                    firing_refused: p.firing_refused_now(),
                });
                if let Err(err) = p.set_checks(&checks).await {
                    tracing::warn!("health.v1 checks not put: {err}");
                }
            }
        }};
    }

    loop {
        let notices: Vec<Notice> = tokio::select! {
            biased;
            () = &mut stop => break,
            // Under `--once` a doctor run in flight is the run this smoke
            // run exists for: the deadline waits for it.
            () = &mut once_deadline, if e.once && doctoring.is_none() => break,
            t = async { watchdog.as_mut().expect("guarded").sip().await }, if !watchdog_done => {
                match t {
                    Some(t) => vec![Notice::Engine(t)],
                    None => {
                        watchdog_done = true;
                        if e.once && doctoring.is_none() { break }
                        Vec::new()
                    }
                }
            }
            _ = async { doctor_tick.as_mut().expect("guarded").tick().await },
                if doctor_tick.is_some() && doctoring.is_none() =>
            {
                let spec = doctor_schedule.as_ref().expect("guarded").spec();
                tracing::info!(deep = spec.deep, "doctor: scheduled run starting (#390)");
                doctoring = Some(Box::pin(async move {
                    Ok(zenkey_fleet::run_doctor(e.bus, e.store, &spec).await)
                }));
                Vec::new()
            }
            outcome = async { doctoring.as_mut().expect("guarded").await }, if doctoring.is_some() => {
                doctoring = None;
                summary.doctor_runs += 1;
                let schedule = doctor_schedule.as_mut().expect("guarded");
                match &outcome {
                    Ok(r) => tracing::info!(findings = r.findings().count(),
                        unobservable = r.unobservable.is_some(), "doctor: run complete"),
                    Err(err) => tracing::warn!("doctor: the run could not happen — unobservable, not clean: {err}"),
                }
                let announced = discipline.announced_under(DOCTOR_RULE);
                let notices: Vec<Notice> = schedule
                    .observe(outcome, now_s(), &announced)
                    .into_iter()
                    .map(|n| Notice::Doctor(Box::new(n)))
                    .collect();
                if let (Some(p), Some(doc)) = (publisher.as_ref(), schedule.document())
                    && let Err(err) = p.publish_doctor(doc).await
                {
                    tracing::warn!("doctor publish failed: {err}");
                }
                for n in notices {
                    observe(&mut discipline, &mut summary, n);
                }
                if e.once && watchdog_done { break }
                Vec::new()
            }
            _ = disc_tick.tick() => {
                // The alarm readers' sweep first (#800, AD) — what moved with
                // no sample, stale or an owner gone — so it flushes in this
                // tick; then their re-read starts beside the drain, one at a
                // time.
                let at = zenkey_fleet::rfc3339_now();
                for n in alarm_rules.sweep(&at) {
                    observe(&mut discipline, &mut summary, Notice::Rule(Box::new(n)));
                }
                if alarm_refresh.is_none() && alarm_rules.listening() {
                    alarm_refresh = Some(Box::pin(crate::alarms::refresh(alarm_rules.plans())));
                }
                discipline_tick!(false);
                Vec::new()
            }
            moved = async { presence_feed.as_mut().expect("guarded").next().await },
                if presence_feed.is_some() =>
            {
                match moved {
                    Some(moved) => {
                        let at = zenkey_fleet::rfc3339_now();
                        let presence = presence_feed.as_ref().expect("guarded").presence();
                        presence_rules
                            .fold(&moved, presence, &at)
                            .into_iter()
                            .map(|n| Notice::Rule(Box::new(n)))
                            .collect()
                    }
                    None => {
                        tracing::warn!("the presence monitor stopped: presence rules see nothing more");
                        presence_feed = None;
                        Vec::new()
                    }
                }
            }
            moved = async { token_feed.as_mut().expect("guarded").next().await },
                if token_feed.is_some() =>
            {
                match moved {
                    Some(moved) => {
                        let at = zenkey_fleet::rfc3339_now();
                        let basis = token_feed.as_ref().expect("guarded").basis();
                        token_rules
                            .fold(&moved, basis, &at)
                            .into_iter()
                            .map(|n| Notice::Rule(Box::new(n)))
                            .collect()
                    }
                    None => {
                        tracing::warn!("the token monitor stopped: liveliness rules see nothing more");
                        token_feed = None;
                        Vec::new()
                    }
                }
            }
            // An alarm reader's delivery folds into its book as it arrives
            // (cancel-safe: `AlarmFeed::next`), and becomes this rule's
            // notices (#800, AD).
            got = alarm_rules.next(), if alarm_rules.listening() => {
                let at = zenkey_fleet::rfc3339_now();
                alarm_rules
                    .delivered(got, &at)
                    .into_iter()
                    .map(|n| Notice::Rule(Box::new(n)))
                    .collect()
            }
            refreshed = async {
                match alarm_refresh.as_mut() {
                    Some(r) => r.await,
                    None => std::future::pending().await,
                }
            }, if alarm_refresh.is_some() =>
            {
                alarm_refresh = None;
                let at = zenkey_fleet::rfc3339_now();
                alarm_rules
                    .folded(refreshed, &at)
                    .into_iter()
                    .map(|n| Notice::Rule(Box::new(n)))
                    .collect()
            }
            Some(_) = tasks.join_next(), if !tasks.is_empty() => Vec::new(),
        };
        for n in notices {
            observe(&mut discipline, &mut summary, n);
        }
    }

    // A stop is a stop, but what is already due goes out: the last tick,
    // every open group flushed, the ledger saved, the service's documents
    // and checks brought up to date.
    discipline_tick!(true);

    // Teardown, acknowledged, in the order the module doc gives: the rules
    // and sinks stop before the service withdraws its tokens (#614, FL3).
    // A completed watchdog hands back its summary; an interrupted one is
    // dropped the way `zenctl watchdog` drops it — it runs until told to
    // stop, and awaiting it would be waiting forever.
    if watchdog_done && let Some(run) = watchdog.take() {
        summary.watchdog = Some(run.await?);
    }
    drop(watchdog);
    summary.dropped = dropped_now(&presence_monitor, &token_monitor);
    summary.evicted = presence_feed
        .as_ref()
        .map_or(0, |f| f.presence().forgotten());
    drop(presence_feed);
    drop(token_feed);
    summary.alarms_lost = alarm_rules.lost();
    drop(alarm_refresh);
    drop(alarm_rules);
    for monitor in [presence_monitor, token_monitor].into_iter().flatten() {
        monitor.shutdown().await?;
    }
    // In-flight deliveries get one sink-timeout's grace, then are abandoned
    // — a stop is a stop.
    let grace = e
        .sinks
        .iter()
        .map(|s| s.timeout)
        .max()
        .unwrap_or(Duration::from_secs(5));
    let _ = tokio::time::timeout(grace, async { while tasks.join_next().await.is_some() {} }).await;
    tasks.abort_all();
    // Last, the service: nothing is left to see it go.
    if let Some(p) = publisher.take() {
        summary.firing_refused = p.firing_refused();
        p.shutdown().await?;
    }
    summary.delivered = counters.delivered.load(Ordering::Relaxed);
    summary.failed = counters.failed.load(Ordering::Relaxed);
    summary.discipline = discipline.counters();
    Ok(summary)
}

/// The signal a daemon stops on: SIGINT or SIGTERM.
async fn stop_signal() {
    #[cfg(unix)]
    {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = async {
                match term.as_mut() {
                    Some(t) => { t.recv().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Which watch sessions `rules` need (#830, core §8.1, 0.37): one in the
/// namespace when a rule follows the deployment's presence live — the
/// presence rules, `alerts`, the engine's `alert-firing` and
/// `instance-gone` — and one in none when a `liveliness-gone` rule names a
/// foreign token.
pub fn watches(rules: &[Rule], namespace: &str) -> crate::bus::Watches {
    let engine = rules.iter().any(|r| match &r.kind {
        RuleKind::Engine(c) => matches!(
            c,
            zenkey_fleet::Condition::InstanceGone { .. }
                | zenkey_fleet::Condition::AlertFiring { .. }
        ),
        _ => false,
    });
    crate::bus::Watches {
        namespaced: engine
            || !PresenceRules::new(rules, namespace).is_empty()
            || !AlarmRules::new(rules).is_empty(),
        raw: !LivelinessRules::new(rules, namespace).is_empty(),
    }
}

/// `zenwatch run`: load and refuse (the file, the deployment flags, the
/// contracts, the state file), build the sinks (secrets resolved now), open
/// the two sessions, and run until a signal.
pub async fn run(args: crate::cli::RunArgs) -> Result<()> {
    let loaded = crate::load_checked(&args.config.config, &args.deployment)?;
    let cfg = &loaded.cfg;
    let rules = cfg
        .rules
        .iter()
        .map(Rule::from_config)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| unaskable!("{e}"))?;
    let sinks =
        Arc::new(crate::sinks::build_all(cfg, args.dry_run).map_err(|e| unaskable!("{e}"))?);
    let watches = watches(&rules, &loaded.bus.namespace);
    let crate::bus::Sessions { bus, raw_watch } = loaded.bus.sessions(watches).await?;
    let store = BundleStore::new(loaded.bus.timeout);
    store.seed(&loaded.contracts);
    let tick = Duration::from_secs_f64(cfg.tick_s);
    eprintln!(
        "zenwatch: {} rule(s), {} sink(s), tick {}s, {}{}{}{}{}{} — three states, \
         ok/firing/unobservable, one notification per sink per genuine change (RFC 13 §3)",
        rules.len(),
        sinks.len(),
        cfg.tick_s,
        if bus.namespace.is_empty() {
            "the bus-root deployment".to_string()
        } else {
            format!("namespace {:?}", bus.namespace)
        },
        match loaded.publish.as_ref().and_then(|p| p.resolve().ok()) {
            Some(r) => format!(
                "; published as {} (health.v1, zenwatch.v1){}",
                r.address,
                if r.minted.as_ref().is_some_and(zenkey::Minted::is_ephemeral) {
                    ", its system EPHEMERAL (hostid.v1 §2.6)"
                } else {
                    ""
                }
            ),
            None => "; no service of its own (publish: false)".to_string(),
        },
        match loaded.contracts.len() {
            0 => "; contracts: from the bus only".to_string(),
            n => format!("; {n} contract revision(s) held offline"),
        },
        match &cfg.doctor {
            Some(d) => format!(
                "; doctor every {} → {}",
                d.every_spelled(),
                d.sinks.join(", ")
            ),
            None => "; doctor not scheduled".to_string(),
        },
        match &cfg.state_file {
            Some(p) => format!("; state file {}", p.display()),
            None => "; no state file: a restart re-announces what is firing".to_string(),
        },
        if args.dry_run {
            "; dry-run: printing, sending nothing"
        } else {
            ""
        },
    );
    let summary = run_on(
        Engine {
            bus: &bus,
            raw_watch: raw_watch.as_ref(),
            store: &store,
            contracts: &loaded.contracts,
            timeout: loaded.bus.timeout,
            tick,
            rules: &rules,
            sinks,
            render: &cfg.render,
            once: args.once,
            ready: None,
            discipline: &cfg.discipline,
            state_file: cfg.state_file.clone(),
            state_max_entries: cfg.state_max_entries,
            // Resolved by `load_checked`, before the sessions opened, through
            // the process's minter, which mints once per run (hostid.v1 §2.7).
            publish: loaded.publish.clone().map(|config| Publish {
                config,
                hostid: zenkey::HostIdMinter::global(),
            }),
            doctor: cfg.doctor.as_ref(),
            clocks_synced: cfg.clocks_synced,
        },
        stop_signal(),
    )
    .await?;
    eprintln!(
        "zenwatch: stopped — {} notice(s), {} notification(s): {} delivered, {} failed{}{}{}{}",
        summary.notices,
        summary.outgoing,
        summary.delivered,
        summary.failed,
        if summary.dropped > 0 {
            format!(
                ", {} liveliness event(s) dropped by the observers (O6)",
                summary.dropped
            )
        } else {
            String::new()
        },
        if summary.alarms_lost > 0 {
            format!(
                ", {} alarm delivery(ies) lost or stall(s) by the alarm readers (O6)",
                summary.alarms_lost
            )
        } else {
            String::new()
        },
        if summary.evicted > 0 {
            format!(
                ", {} address record(s) forgotten at the bound",
                summary.evicted
            )
        } else {
            String::new()
        },
        if cfg.doctor.is_some() {
            format!(", {} doctor run(s)", summary.doctor_runs)
        } else {
            String::new()
        },
    );
    if let Some(at) = &summary.self_address {
        eprintln!(
            "zenwatch: its service at {at} withdrawn, its firing documents deleted{}",
            match summary.firing_refused {
                0 => String::new(),
                n => format!(
                    "; {n} refused at the bound of {}",
                    publish::FIRING_CARDINALITY
                ),
            }
        );
    }
    let d = summary.discipline;
    eprintln!(
        "zenwatch: discipline — {} deduplicated, {} grouped, {} repeat(s), {} cancelled in \
         `for`, {} baseline, {} entr{} evicted",
        d.deduped,
        d.grouped,
        d.repeats,
        d.cancelled,
        d.baseline,
        d.evicted,
        if d.evicted == 1 { "y" } else { "ies" },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RuleConfig;
    use zenkey_fleet::{AddressStatus, PresenceChange, PresenceSource};

    /// The watch sessions a run opens (#830): one in the namespace for the
    /// rules that follow the deployment's presence live, one in none for a
    /// foreign token's ledger, and none that no rule needs.
    #[test]
    fn a_run_opens_the_watch_sessions_its_rules_need() {
        let w = |texts: &[&str]| {
            let rules: Vec<Rule> = texts.iter().map(|t| rule("r", t, &[])).collect();
            let w = watches(&rules, "site");
            (w.namespaced, w.raw)
        };
        assert_eq!(w(&["silent-for site/zk2/**/stream/** 10"]), (false, false));
        assert_eq!(w(&["instance-gone */tc"]), (true, false));
        assert_eq!(
            w(&["liveliness-gone site/zk2/*/tc/@zk/alive/tc.v1/**"]),
            (true, false)
        );
        assert_eq!(w(&["alerts zk2/*/*/alarms.v1/**"]), (true, false));
        assert_eq!(w(&["alert-firing zk2/*/*/alarms.v1/**"]), (true, false));
        assert_eq!(w(&["liveliness-gone demo/held/*/alive"]), (false, true));
        assert_eq!(
            w(&["instance-gone */tc", "liveliness-gone demo/held/*/alive"]),
            (true, true)
        );
    }

    fn rule(name: &str, text: &str, labels: &[(&str, &str)]) -> Rule {
        Rule::from_config(&RuleConfig {
            name: name.into(),
            rule: text.into(),
            severity: Some("error".into()),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            sinks: vec!["ops".into()],
            for_s: None,
        })
        .unwrap()
    }

    /// An engine transition routes to the rule spelling its condition, its
    /// three states whole — `unobservable` stays `unobservable` — under the
    /// identity of the rule's sorted labels; no other rule takes it.
    #[test]
    fn an_engine_transition_routes_to_its_condition_three_states_whole() {
        let rules = vec![
            rule(
                "quiet",
                "silent-for site/zk2/** 30",
                &[("team", "infra"), ("env", "prod")],
            ),
            rule("tc-gone", "instance-gone */tc", &[]),
        ];
        let t = Transition {
            rule: "silent-for site/zk2/** 30".into(),
            from: Some(CondState::Ok),
            to: CondState::Unobservable,
            at: "2026-10-10T00:00:00Z".into(),
            evidence: "no sample seen, but the observer dropped".into(),
        };
        let out = route(&Notice::Engine(t), &rules, &RenderConfig::default());
        assert_eq!(out.len(), 1);
        let n = &out[0].notification;
        assert_eq!(n.id, "quiet:env=prod,team=infra");
        assert_eq!(n.rule_kind, "silent-for");
        assert_eq!(n.kind, NoticeKind::Transition);
        assert_eq!(
            (n.state, n.prior),
            (CondState::Unobservable, Some(CondState::Ok))
        );
        assert_eq!(n.title, "error quiet unobservable");
        assert!(n.message.contains("key: site/zk2/**"), "{}", n.message);
        assert_eq!(n.rendering, RenderSource::KeyOnly);
        assert!(n.presence.is_none());
        assert_eq!(
            meta_of(&Notice::Engine(Transition {
                rule: String::new(),
                from: None,
                to: CondState::Ok,
                at: String::new(),
                evidence: String::new(),
            }))
            .address,
            None
        );
    }

    /// A presence rule's notice routes to its rule alone, under
    /// `<rule_id>:<identity>`, carrying the move that decided it, its
    /// address as a line and as the discipline's grouping fact; a baseline's
    /// own severity wins over the rule's.
    #[test]
    fn a_rule_notice_routes_to_its_rule_with_its_move() {
        let rules = vec![
            rule("tc-gone", "instance-gone */tc", &[]),
            rule("other", "instance-gone */gui", &[]),
        ];
        let moved = PresenceTransition {
            at: SystemTime::UNIX_EPOCH,
            source: PresenceSource::Seed,
            address: "host-a/tc".parse().unwrap(),
            change: PresenceChange::Status {
                from: AddressStatus::Unobservable,
                to: AddressStatus::Down,
            },
        };
        let n = RuleNotice {
            rule: "tc-gone".into(),
            what: "host-a/tc".into(),
            kind: NoticeKind::Presence,
            state: CondState::Firing,
            prior: Some(CondState::Ok),
            severity: None,
            evidence: "no instance token of host-a/tc".into(),
            lines: vec![],
            key: None,
            address: Some("host-a/tc".into()),
            at: "t".into(),
            presence: Some(moved.clone()),
            assume: false,
            passthrough: false,
            reason: "a presence token carries no payload",
            alert: None,
        };
        let notice = Notice::Rule(Box::new(n.clone()));
        let out = route(&notice, &rules, &RenderConfig::default());
        assert_eq!(out.len(), 1, "its rule alone");
        let o = &out[0].notification;
        assert_eq!(o.id, "tc-gone:host-a/tc");
        assert_eq!(o.rule_kind, "instance-gone");
        assert_eq!(o.presence.as_ref(), Some(&moved));
        assert!(o.message.contains("address: host-a/tc"), "{}", o.message);
        let meta = meta_of(&notice);
        assert_eq!(meta.address.as_deref(), Some("host-a/tc"));
        assert!(!meta.assume && !meta.passthrough);

        let baseline = RuleNotice {
            what: "baseline".into(),
            kind: NoticeKind::Baseline,
            severity: Some("info".into()),
            passthrough: true,
            presence: None,
            ..n
        };
        let out = route(
            &Notice::Rule(Box::new(baseline)),
            &rules,
            &RenderConfig::default(),
        );
        assert_eq!(out[0].notification.severity, "info");
        assert_eq!(out[0].notification.kind, NoticeKind::Baseline);
        assert_eq!(out[0].notification.id, "tc-gone:baseline");
    }

    /// An alert notice (#800, AD) routes to its rule alone, under
    /// `<rule_id>:<alarm key>`, at its document's severity, its document's
    /// discriminating labels over the rule's, the document rendered through
    /// the contract (`schema`) with the owner's stamp, and the fleet's
    /// transition carried whole.
    #[test]
    fn an_alert_notice_carries_its_document_and_transition() {
        use zenkey_fleet::report::{AlarmAnswer, AlarmAnswerToken, AlarmChange, AlarmVia, Stamp};
        let rules = vec![
            rule(
                "fleet",
                "alerts zk2/*/*/alarms.v1/state/alarms/*",
                &[("team", "infra")],
            ),
            rule("other", "alerts zk2/far/**", &[]),
        ];
        let key = "zk2/h1/netlink/alarms.v1/state/alarms/a659f813308ad1da";
        let document = zenkey_model::alarms::Alarm {
            timestamp: 1_760_950_000_000,
            observed_at_ms: None,
            rule: "link_down".into(),
            severity: zenkey_model::alarms::Severity::Critical,
            state: zenkey_model::alarms::State::Firing,
            summary: "link to r2 down on eth0".into(),
            category: None,
            labels: BTreeMap::from([
                ("peer".into(), "r2".into()),
                ("host".into(), "h-3fa9c2d41b7e".into()),
            ]),
        };
        let transition = zenkey_fleet::AlarmTransition {
            key: key.into(),
            alarm: zenkey_model::alarms::AlarmRef::from_key(key).unwrap(),
            change: AlarmChange::Raised,
            firing: AlarmAnswer {
                answer: AlarmAnswerToken::Yes,
                reason: "firing".into(),
                says: "its owner says it fires".into(),
            },
            severity: Some(zenkey_model::alarms::Severity::Critical),
            document: Some(document.clone()),
            stamp: Some(Stamp {
                time: "2026-10-20T00:00:00.000000000Z".into(),
                clock: "a1".into(),
            }),
            via: Some(AlarmVia::Subscription),
            lost: None,
        };
        let n = RuleNotice {
            rule: "fleet".into(),
            what: key.into(),
            kind: NoticeKind::Alert,
            state: CondState::Firing,
            prior: None,
            severity: Some("critical".into()),
            evidence: "alarm a659f813308ad1da of h1/netlink raised at critical".into(),
            lines: vec![],
            key: Some(key.into()),
            address: Some("h1/netlink".into()),
            at: "t".into(),
            presence: None,
            assume: false,
            passthrough: false,
            reason: "a delete carries no document",
            alert: Some(Box::new(AlertFacts {
                transition: transition.clone(),
                document: Some(document),
            })),
        };
        let out = route(&Notice::Rule(Box::new(n)), &rules, &RenderConfig::default());
        assert_eq!(out.len(), 1, "its rule alone");
        let o = &out[0].notification;
        assert_eq!(o.id, format!("fleet:{key}"));
        assert_eq!(
            (o.rule_kind.as_str(), o.kind),
            ("alerts", NoticeKind::Alert)
        );
        assert_eq!(o.severity, "critical");
        assert_eq!(o.title, "critical fleet firing");
        assert_eq!(
            o.labels,
            BTreeMap::from([
                ("peer".to_string(), "r2".to_string()),
                ("team".to_string(), "infra".to_string()),
            ])
        );
        assert_eq!(o.rendering, RenderSource::Schema);
        assert_eq!(o.alert.as_ref(), Some(&transition));
        assert!(o.message.contains(&format!("key: {key}")), "{}", o.message);
        assert!(
            o.message
                .contains("timestamp: 2026-10-20T00:00:00.000000000Z"),
            "{}",
            o.message
        );
        assert!(
            o.message.contains("payload: {") && o.message.contains("\"rule\":\"link_down\""),
            "{}",
            o.message
        );
        assert!(o.presence.is_none());
    }
}
