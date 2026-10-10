//! The run loop: three observers, one router, N sinks — on two sessions
//! (#614, FL2).
//!
//! The fleet's zk2 `watchdog` Straw judges the seven engine conditions
//! every tick — its wire selectors on the session in no namespace, its
//! contracts, presence and doctor through the one in the namespace — and
//! yields a [`Transition`] per genuine change. The fleet's live presence
//! projection ([`zenkey_fleet::PresenceFeed`], its own monitor on the
//! namespaced session) carries `instance-gone` and `liveliness-gone` on the
//! deployment's zk2 tokens ([`crate::presence`]); the daemon's own token
//! ledger ([`crate::liveliness`], its own monitor on the raw session)
//! carries `liveliness-gone` on anything else. A monitor each, because a
//! `Dropped(n)` is one receiver's lag: a flood on one plane must not make
//! another's tokens unobservable. All three feed one [`Notice`] stream,
//! [`route`] turns a notice into one [`Outgoing`] per matching rule, and
//! every named sink gets it concurrently — a slow sink widens nothing, a
//! failed sink is counted and logged, never fatal.
//!
//! **Three states, preserved.** `ok` / `firing` / `unobservable` ride from
//! the engine's `CondState` into the sink payload unchanged. A presence or
//! token rule whose observer lost events, or whose read ran to its timeout,
//! says `unobservable` once, on its `observer` identity — never `ok`
//! (RFC 13 §3 O6).
//!
//! **Transitions, not states.** A token re-declared is not a notification;
//! an address that was already down going down is not one either. What
//! each rule last said lives with it, and what the discipline announced
//! lives in its bounded ledger.
//!
//! **Between [`route`] and delivery sits the [`Discipline`]** (#389): every
//! routed [`Outgoing`] is observed, and only what the tick flushes — after
//! `for`, dedup, grouping and repeat — is dispatched. The tick also
//! persists the ledger and refreshes what this daemon publishes about
//! itself ([`crate::publish`], v1's write side until FL3).
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

use crate::config::{DisciplineConfig, DoctorConfig};
use crate::discipline::{Counters as DisciplineCounters, Discipline, NoticeMeta, state as ledger};
use crate::doctor::{self, DoctorNotice};
use crate::exit::unaskable;
use crate::liveliness::{LivelinessRules, TokenFeed};
use crate::presence::PresenceRules;
use crate::publish::{self, DoctorStatus, FiringRule, HealthStatus, SelfProducer, ZenwatchHealth};
use crate::render::{Draft, Payload, RenderConfig, RenderSource};
use crate::rules::{DOCTOR_RULE, Rule, RuleKind};
use crate::sinks::{NoticeKind, Notification, Outgoing, Sink};

/// One thing the observers saw that might be worth telling someone.
#[derive(Debug, Clone)]
pub enum Notice {
    /// The engine judged a condition into a different state.
    Engine(Transition),
    /// A presence or token rule said something about one identity, its
    /// observer, or its first look (#614, FL2). Already the rule's: the
    /// projection and the ledger fold per rule. Boxed: it carries the
    /// presence move that decided it.
    Rule(Box<RuleNotice>),
    /// One thing a scheduled doctor run said (#390): the baseline, a
    /// finding new or fixed since the last run, or the run itself failing
    /// or recovering.
    Doctor(Box<DoctorNotice>),
}

/// One presence or token rule's notice: the identity, its state, and the
/// facts its rendering and the discipline need.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleNotice {
    /// The rule's name.
    pub rule: String,
    /// The identity under the rule: an address, an address and what a
    /// token names, a token's wire key, `observer`, `baseline`.
    pub what: String,
    /// The notice's own kind: what a passthrough carries as is; a
    /// remembered identity's is the discipline's verdict.
    pub kind: NoticeKind,
    pub state: CondState,
    /// What the rule last said of the identity; `None` on first sight.
    pub prior: Option<CondState>,
    /// The severity, when it is not the rule's (an `info` baseline).
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
/// and what the notice is about — an address, a token's key, `observer`, an
/// engine condition's sorted labels, a doctor finding's check and subject —
/// stable across ticks, runs and restarts, which is what dedup and the
/// state file key on.
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
                let payload = Payload::None {
                    reason: n.reason.into(),
                };
                let rendered = crate::render::render(
                    &Draft {
                        rule: &r.name,
                        state: n.state,
                        prior: n.prior,
                        severity: &severity,
                        evidence: &n.evidence,
                        extra: &extra,
                        labels: &r.labels,
                        key: n.key.as_deref(),
                        timestamp: None,
                        payload: &payload,
                    },
                    render,
                );
                out.push(outgoing(
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
                ));
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
    pub watchdog: Option<WatchdogSummary>,
    /// What the discipline did (#389).
    pub discipline: DisciplineCounters,
    /// The origin this daemon published under, when it did.
    pub self_origin: Option<String>,
    /// Scheduled doctor runs, attempted (#390).
    pub doctor_runs: u64,
}

/// Everything a run needs, with the sessions already open — the shape the
/// bus tests drive directly.
pub struct Engine<'a> {
    /// The two sessions: in the deployment's namespace, and in none.
    pub bus: &'a DoctorBus,
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
    /// Publish `health`, `firing/*` and the token — a real daemon does;
    /// a bus test that is not about self-publication does not.
    pub publish: bool,
    /// The scheduled doctor (#390); `None` is not scheduled — nothing runs,
    /// nothing is published, and the health document says so.
    pub doctor: Option<&'a DoctorConfig>,
    /// The operator's word on the clocks (`freshness.v1` §2.6), for the
    /// doctor's `health-*` checks, scheduled or a rule's.
    pub clocks_synced: bool,
}

/// Delivery counters shared with the spawned deliveries.
#[derive(Default)]
struct Counters {
    delivered: AtomicU64,
    failed: AtomicU64,
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
                    counters.delivered.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(sink = %sink.name, kind = sink.kind_name(), rule = %n.rule,
                        state = crate::render::state_word(n.state), id = %n.id, "delivered: {}", d.detail);
                }
                Err(e) => {
                    counters.failed.fetch_add(1, Ordering::Relaxed);
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
    // own monitor on the namespaced session, seeded before `ready`.
    let mut presence_rules = PresenceRules::new(e.rules, namespace);
    let mut presence_monitor: Option<Monitor> = None;
    let mut presence_feed: Option<PresenceFeed> = None;
    if !presence_rules.is_empty() {
        let scope = PresenceScope::all();
        let monitor = Monitor::start(
            &e.bus.session,
            liveliness_spec(Vec::new()).with_presence("", &scope),
        )
        .await?;
        let mut live = LivePresence::new(scope);
        presence_rules.track(&mut live, SystemTime::now());
        let (feed, _seeded) =
            PresenceFeed::open(&e.bus.session, monitor.events(), live, e.timeout).await;
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
        let monitor = Monitor::start(&e.bus.raw, liveliness_spec(selectors.clone())).await?;
        let feed = TokenFeed::open(&e.bus.raw, monitor.events(), selectors, e.timeout).await;
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
    let dropped_now = |p: &Option<Monitor>, t: &Option<Monitor>| {
        p.as_ref().map_or(0, |m| m.core().dropped()) + t.as_ref().map_or(0, |m| m.core().dropped())
    };

    // The daemon as a producer — v1's write side until FL3 (#614): its v1
    // keys under the deployment's base, on the raw session, as v1 zenwatch
    // published them. After the watches, before `ready`, so a test that
    // sees the token can already call the queryables.
    let mut publisher: Option<SelfProducer> = if e.publish {
        let fleet = zenkey_fleet_v1::Fleet::new(&e.bus.raw, namespace);
        let p = SelfProducer::bring_up(&fleet).await?;
        summary.self_origin = Some(p.origin());
        Some(p)
    } else {
        None
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

    let counters = Arc::new(Counters::default());
    let mut tasks = tokio::task::JoinSet::new();
    // The health cadence and what the last document said, as one record
    // so the final tick's writes are reads for the next daemon, not
    // dead stores.
    struct Pulse {
        last_health: Option<tokio::time::Instant>,
        last_persist_error: Option<String>,
        failed_at_last_health: u64,
    }
    let mut pulse = Pulse {
        last_health: None,
        last_persist_error: None,
        failed_at_last_health: 0,
    };

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
                let docs: BTreeMap<String, FiringRule> = publish::firing_docs(&discipline);
                if let Err(err) = p.publish_firing(&docs).await {
                    tracing::warn!("firing/* publish failed: {err}");
                }
                let due = pulse
                    .last_health
                    .is_none_or(|t| t.elapsed() >= publish::HEALTH_PERIOD);
                if due {
                    let failed = counters.failed.load(Ordering::Relaxed);
                    let c = discipline.counters();
                    let health = ZenwatchHealth {
                        status: if pulse.last_persist_error.is_some()
                            || failed > pulse.failed_at_last_health
                        {
                            HealthStatus::Degraded
                        } else {
                            HealthStatus::Ok
                        },
                        host_id: p.origin(),
                        started_at: started_at.clone(),
                        rules: e.rules.len(),
                        sinks: e.sinks.len(),
                        firing: discipline
                            .announced()
                            .filter(|x| x.state == CondState::Firing)
                            .count(),
                        unobservable: discipline
                            .announced()
                            .filter(|x| x.state == CondState::Unobservable)
                            .count(),
                        notifications_sent: c.sent,
                        deliveries_failed: failed,
                        // Inhibition was removed (#614, FL2); the field is
                        // v1's served shape until FL3 redraws it.
                        inhibited: 0,
                        dropped_total: summary.dropped,
                        state_entries: discipline.len(),
                        state_evicted: c.evicted,
                        firing_refused: p.refused(),
                        last_persist_error: pulse.last_persist_error.clone(),
                        doctor: doctor_schedule
                            .as_ref()
                            .map_or(DoctorStatus::NotScheduled, |s| s.status()),
                        doctor_last: doctor_schedule
                            .as_ref()
                            .and_then(|s| s.document())
                            .map(|d| d.ran_at.clone()),
                        doctor_next: doctor_schedule
                            .as_ref()
                            .and_then(|s| s.document())
                            .map(|d| d.next_at.clone()),
                    };
                    if let Err(err) = p.publish_health(&health).await {
                        tracing::warn!("health publish failed: {err}");
                    }
                    pulse.failed_at_last_health = failed;
                    pulse.last_health = Some(tokio::time::Instant::now());
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
            Some(_) = tasks.join_next(), if !tasks.is_empty() => Vec::new(),
        };
        for n in notices {
            observe(&mut discipline, &mut summary, n);
        }
    }

    // A stop is a stop, but what is already due goes out: the last tick,
    // every open group flushed, the ledger saved.
    discipline_tick!(true);

    // Teardown, acknowledged: a completed watchdog hands back its summary;
    // an interrupted one is dropped the way `zenctl watchdog` drops it — it
    // runs until told to stop, and awaiting it would be waiting forever.
    if watchdog_done && let Some(run) = watchdog.take() {
        summary.watchdog = Some(run.await?);
    }
    drop(watchdog);
    if let Some(p) = publisher.take() {
        p.shutdown().await?;
    }
    summary.dropped = dropped_now(&presence_monitor, &token_monitor);
    summary.evicted = presence_feed
        .as_ref()
        .map_or(0, |f| f.presence().forgotten());
    drop(presence_feed);
    drop(token_feed);
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
    let bus = loaded.bus.sessions().await?;
    let store = BundleStore::new(loaded.bus.timeout);
    store.seed(&loaded.contracts);
    let tick = Duration::from_secs_f64(cfg.tick_s);
    eprintln!(
        "zenwatch: {} rule(s), {} sink(s), tick {}s, {}{}{}{}{} — three states, \
         ok/firing/unobservable, one notification per sink per genuine change (RFC 13 §3)",
        rules.len(),
        sinks.len(),
        cfg.tick_s,
        if bus.namespace.is_empty() {
            "the bus-root deployment".to_string()
        } else {
            format!("namespace {:?}", bus.namespace)
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
            publish: true,
            doctor: cfg.doctor.as_ref(),
            clocks_synced: cfg.clocks_synced,
        },
        stop_signal(),
    )
    .await?;
    eprintln!(
        "zenwatch: stopped — {} notice(s), {} notification(s): {} delivered, {} failed{}{}{}",
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
}
