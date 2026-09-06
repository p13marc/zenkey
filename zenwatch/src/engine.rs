//! The run loop: two observers, one router, N sinks.
//!
//! The engine's `watchdog` Straw judges the eight closed conditions every
//! tick and yields a [`Transition`] per genuine change; this daemon's own
//! [`Monitor`] watches the alert plane and the liveliness roster for the two
//! kinds the daemon owns. Both feed one [`Notice`] stream, [`route`] turns a
//! notice into one [`Outgoing`] per matching rule, and every named sink gets
//! it concurrently — a slow sink widens nothing, a failed sink is counted and
//! logged, never fatal.
//!
//! **Three states, preserved.** `ok` / `firing` / `unobservable` ride from
//! the engine's `CondState` into the sink payload unchanged, and this loop
//! adds its own honesty rule on top: a [`StreamItem::Dropped`] from the
//! monitor is an `unobservable` notification on every `alerts` and
//! `liveliness-gone` rule, because a dropped event may have been the
//! resolve — and the alternative is a phone that stays quiet about an alert
//! that is still firing (RFC 13 §3 O6).
//!
//! **Transitions, not states.** An alert re-put with the same state and
//! severity is not a notification; a token that was already down going
//! down is not one either. Per-key state lives in a bounded [`Ledger`], and
//! what the bound cost rides the summary.
//!
//! The discipline chunk (#387) — `for`, dedup windows, grouping, inhibition,
//! repeat, and the daemon's own `health`/`firing/{rule_id}` publication —
//! sits between [`route`] and delivery and is not built here; [`Notice`] is
//! the seam it will hang off.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::{
    AlertState, AlertTransition, CondState, Fleet, FleetEvent, Monitor, MonitorSpec, RenderSource,
    SchemaStore, Sipper as _, SliceSet, StreamItem, Transition, WatchdogSpec, WatchdogSummary,
};
use zenoh::key_expr::KeyExpr;
use zenoh::sample::SampleKind;

use crate::config::Config;
use crate::exit::unaskable;
use crate::render::{Draft, Payload, RenderConfig};
use crate::rules::{Rule, RuleKind};
use crate::sinks::{Notification, Outgoing, Sink};

/// One thing the observers saw that might be worth telling someone.
#[derive(Debug, Clone)]
pub enum Notice {
    /// The engine judged a condition into a different state.
    Engine(Transition),
    /// An alert key moved (RFC 04 §1.2), with the document's rendering.
    Alert {
        key: String,
        /// Boxed: the transition carries the document's labels and summary,
        /// and an engine transition is a fraction of that.
        transition: Box<AlertTransition>,
        /// What this daemon last saw the key as — `None` on first sight.
        prior: Option<CondState>,
        payload: Payload,
    },
    /// An alive token went (`up: false`) or came back (RFC 04 §5).
    Liveliness {
        key: String,
        origin: String,
        producer: String,
        up: bool,
        prior: Option<CondState>,
        at: String,
    },
    /// The monitor's broadcast overflowed: `n` events this daemon never saw.
    Dropped { n: u64, at: String },
}

/// A bounded per-key memory: the last thing seen on each key, oldest
/// evicted past `cap`, with the cost counted (RFC 13 §3 O6).
#[derive(Debug)]
pub struct Ledger<V> {
    map: HashMap<String, (V, u64)>,
    cap: usize,
    seq: u64,
    evicted: u64,
}

impl<V> Ledger<V> {
    pub fn new(cap: usize) -> Ledger<V> {
        Ledger {
            map: HashMap::new(),
            cap: cap.max(1),
            seq: 0,
            evicted: 0,
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key).map(|(v, _)| v)
    }

    pub fn set(&mut self, key: String, v: V) {
        self.seq += 1;
        if !self.map.contains_key(&key) && self.map.len() >= self.cap {
            // Evict the least recently *set* key: O(n) on overflow only,
            // which at the bound is once per new key past it.
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (_, s))| *s)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
                self.evicted += 1;
            }
        }
        self.map.insert(key, (v, self.seq));
    }

    pub fn evicted(&self) -> u64 {
        self.evicted
    }
}

/// How many alert keys and alive tokens the ledgers remember.
pub const LEDGER_CAP: usize = 4096;

/// The `CondState` an alert state projects to: firing is firing, resolved
/// is the established-clean pole.
pub fn cond_of(a: AlertState) -> CondState {
    match a {
        AlertState::Firing => CondState::Firing,
        AlertState::Resolved => CondState::Ok,
    }
}

/// One notice → one [`Outgoing`] per rule it matches. Pure: the router
/// holds no state but the id counter.
pub fn route(
    notice: &Notice,
    rules: &[Rule],
    render: &RenderConfig,
    seq: &mut u64,
) -> Vec<Outgoing> {
    let keyexpr = match notice {
        Notice::Alert { key, .. } | Notice::Liveliness { key, .. } => {
            KeyExpr::try_from(key.as_str()).ok()
        }
        _ => None,
    };
    let mut out = Vec::new();
    for r in rules {
        let no_payload = |reason: &str| Payload::None {
            reason: reason.to_string(),
        };
        let covers = keyexpr.as_ref().is_some_and(|k| r.covers(k));
        let (state, prior, severity, evidence, labels, key, timestamp, payload, at, rendering) =
            match (notice, &r.kind) {
                (Notice::Engine(t), RuleKind::Engine(c)) if c.to_string() == t.rule => (
                    t.to,
                    t.from,
                    r.severity.clone(),
                    t.evidence.clone(),
                    r.labels.clone(),
                    c.selector().map(str::to_string),
                    None,
                    no_payload("an engine condition is judged over a window, not a payload"),
                    t.at.clone(),
                    RenderSource::KeyOnly,
                ),
                (
                    Notice::Alert {
                        key,
                        transition: a,
                        prior,
                        payload,
                    },
                    RuleKind::Alerts { .. },
                ) if covers => {
                    let mut labels = r.labels.clone();
                    labels.extend(a.labels.iter().map(|(k, v)| (k.clone(), v.clone())));
                    let mut evidence = format!(
                        "alert {} {}",
                        a.alert_ref,
                        match a.state {
                            AlertState::Firing => "firing",
                            AlertState::Resolved => "resolved (tombstone)",
                        }
                    );
                    if let Some(rule) = &a.rule {
                        evidence.push_str(&format!(" rule={rule}"));
                    }
                    if let Some(s) = &a.summary {
                        evidence.push_str(&format!(" — {s}"));
                    }
                    (
                        cond_of(a.state),
                        *prior,
                        a.severity.clone().unwrap_or_else(|| r.severity.clone()),
                        evidence,
                        labels,
                        Some(key.clone()),
                        a.timestamp.clone(),
                        payload.clone(),
                        a.at.clone(),
                        a.rendering,
                    )
                }
                (
                    Notice::Liveliness {
                        key,
                        origin,
                        producer,
                        up,
                        prior,
                        at,
                    },
                    RuleKind::LivelinessGone { .. },
                ) if covers => (
                    if *up {
                        CondState::Ok
                    } else {
                        CondState::Firing
                    },
                    *prior,
                    r.severity.clone(),
                    format!(
                        "alive token for {origin}/{producer} {} (RFC 04 §5)",
                        if *up { "is back" } else { "is gone" }
                    ),
                    r.labels.clone(),
                    Some(key.clone()),
                    None,
                    no_payload("a liveliness token carries no payload"),
                    at.clone(),
                    RenderSource::KeyOnly,
                ),
                (
                    Notice::Dropped { n, at },
                    RuleKind::Alerts { .. } | RuleKind::LivelinessGone { .. },
                ) => (
                    CondState::Unobservable,
                    None,
                    r.severity.clone(),
                    format!(
                        "the observer dropped {n} event(s): a firing or a resolve inside that \
                         span was not seen — unobservable, not ok (RFC 13 §3 O6)"
                    ),
                    r.labels.clone(),
                    None,
                    None,
                    no_payload("nothing was observed"),
                    at.clone(),
                    RenderSource::KeyOnly,
                ),
                _ => continue,
            };
        let rendered = crate::render::render(
            &Draft {
                rule: &r.name,
                state,
                prior,
                severity: &severity,
                evidence: &evidence,
                labels: &labels,
                key: key.as_deref(),
                timestamp: timestamp.as_deref(),
                payload: &payload,
            },
            render,
        );
        *seq += 1;
        out.push(Outgoing {
            notification: Notification {
                id: format!("{}-{seq}", r.id),
                rule: r.name.clone(),
                kind: r.kind.head().to_string(),
                state,
                prior,
                severity,
                title: rendered.title,
                message: rendered.message,
                labels,
                at,
                evidence,
                rendering: match payload {
                    Payload::None { .. } => rendering,
                    _ => rendered.source,
                },
                truncated: rendered.truncated,
            },
            sinks: r.sinks.clone(),
        });
    }
    out
}

/// What one run watched and did.
#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    pub notices: u64,
    pub outgoing: u64,
    pub delivered: u64,
    pub failed: u64,
    /// Events the monitor's broadcast dropped on this consumer.
    pub dropped: u64,
    /// Keys the two ledgers retired at their bound.
    pub evicted: u64,
    pub watchdog: Option<WatchdogSummary>,
}

/// Everything a run needs, with the session already open — the shape the
/// bus tests drive directly.
pub struct Engine<'a> {
    pub fleet: Fleet<'a>,
    pub slices: Option<&'a SliceSet>,
    pub timeout: Duration,
    pub tick: Duration,
    pub rules: &'a [Rule],
    pub sinks: Arc<Vec<Sink>>,
    pub render: &'a RenderConfig,
    /// One tick, then stop.
    pub once: bool,
    /// Fired once the monitor is subscribed — the test seam that lets a
    /// bus test publish *after* the observer is watching (O4: a sample the
    /// observer was not yet declared for is not a sample it missed).
    pub ready: Option<tokio::sync::oneshot::Sender<()>>,
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

/// One liveliness event against the token ledger: a token first seen up is
/// the baseline (the monitor replays the roster on join — that is not
/// "everyone came back"); up→down and down→up are the transitions.
fn liveliness(
    tokens: &mut Ledger<bool>,
    base: &str,
    key: String,
    up: bool,
    rules: &[Rule],
) -> Option<Notice> {
    let Ok(keyexpr) = KeyExpr::try_from(key.as_str()) else {
        return None;
    };
    if !rules
        .iter()
        .any(|r| matches!(r.kind, RuleKind::LivelinessGone { .. }) && r.covers(&keyexpr))
    {
        return None;
    }
    let (origin, producer) = zenkey_fleet::token_identity(base, &key)?;
    let prior = tokens.get(&key).copied();
    tokens.set(key.clone(), up);
    match (prior, up) {
        // Baseline, or no change: not a transition.
        (None, true) | (Some(true), true) | (Some(false), false) => None,
        _ => Some(Notice::Liveliness {
            key,
            origin,
            producer,
            up,
            prior: prior.map(|was_up| {
                if was_up {
                    CondState::Ok
                } else {
                    CondState::Firing
                }
            }),
            at: zenkey_fleet::rfc3339_now(),
        }),
    }
}

/// Run until `stop` resolves (or, with `once`, until one tick has been
/// evaluated). Returns the summary; delivery failures are in it, never an
/// `Err`.
pub async fn run_on(e: Engine<'_>, stop: impl Future<Output = ()>) -> Result<RunSummary> {
    let (session, base) = (e.fleet.session(), e.fleet.base());
    let mut summary = RunSummary::default();

    let engine_rules: Vec<_> = e
        .rules
        .iter()
        .filter_map(|r| match &r.kind {
            RuleKind::Engine(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let mut alert_selectors: Vec<String> = Vec::new();
    let mut liveliness_selectors: Vec<String> = Vec::new();
    for r in e.rules {
        let (list, sel) = match &r.kind {
            RuleKind::Alerts { selector } => (&mut alert_selectors, selector),
            RuleKind::LivelinessGone { selector } => (&mut liveliness_selectors, selector),
            RuleKind::Engine(_) => continue,
        };
        if !list.contains(sel) {
            list.push(sel.clone());
        }
    }

    // The schema store is warmed before anything is watched and sealed for
    // the run (#337): a decode on the drain loop must never become a GET.
    // The per-tick sweep below re-warms beside the drain.
    let store = SchemaStore::new(base, e.timeout);
    if e.slices.is_some() {
        zenkey_fleet::prewarm(&e.fleet, &store, e.slices).await;
    }
    let _sealed = store.seal();

    let spec = WatchdogSpec {
        rules: engine_rules.clone(),
        tick: e.tick,
        ticks: e.once.then_some(1),
        timeout: e.timeout,
    };
    let mut watchdog = (!engine_rules.is_empty())
        .then(|| zenkey_fleet::watchdog(&e.fleet, e.slices, &store, &spec).pin());
    let mut watchdog_done = watchdog.is_none();

    let monitor = Monitor::start(
        session,
        MonitorSpec {
            selectors: alert_selectors,
            liveliness: liveliness_selectors,
            ..Default::default()
        },
    )
    .await?;
    let mut events = monitor.events();

    let mut alerts: Ledger<(AlertState, Option<String>)> = Ledger::new(LEDGER_CAP);
    let mut tokens: Ledger<bool> = Ledger::new(LEDGER_CAP);
    // The roster baseline, by an explicit ask (RFC 05 §4: the seed is the
    // state itself). The monitor's history replay also delivers every token
    // already up — but it can land on the broadcast before this receiver
    // subscribed and be lost, and a token whose baseline was lost would
    // report its retirement with no prior. A token the ask and the replay
    // both name is set to the same value twice; a token neither names is
    // the baseline the first time it is seen. If the ask fails, that is
    // logged, not a verdict: nothing here says the roster is empty.
    for selector in &e
        .rules
        .iter()
        .filter_map(|r| match &r.kind {
            RuleKind::LivelinessGone { selector } => Some(selector.clone()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
    {
        match session.liveliness().get(selector).timeout(e.timeout).await {
            Ok(replies) => {
                while let Ok(reply) = replies.recv_async().await {
                    if let Ok(sample) = reply.result() {
                        tokens.set(sample.key_expr().as_str().to_string(), true);
                    }
                }
            }
            Err(err) => tracing::warn!(
                selector,
                "roster ask failed ({err}); tokens seen later are the baseline"
            ),
        }
    }
    if let Some(ready) = e.ready {
        let _ = ready.send(());
    }

    let mut sweep = tokio::time::interval(e.tick);
    sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    sweep.tick().await; // the first tick is immediate, and the prewarm was
    let mut sweeping: Option<Pin<Box<dyn Future<Output = usize> + Send + '_>>> = None;
    let once_deadline = tokio::time::sleep(e.tick + Duration::from_millis(250));
    tokio::pin!(once_deadline);
    let mut stop = std::pin::pin!(stop);

    let counters = Arc::new(Counters::default());
    let mut tasks = tokio::task::JoinSet::new();
    let mut seq = 0u64;

    loop {
        let notice: Option<Notice> = tokio::select! {
            biased;
            () = &mut stop => break,
            () = &mut once_deadline, if e.once => break,
            t = async { watchdog.as_mut().expect("guarded").sip().await }, if !watchdog_done => {
                match t {
                    Some(t) => Some(Notice::Engine(t)),
                    None => {
                        watchdog_done = true;
                        if e.once { break }
                        None
                    }
                }
            }
            _ = sweep.tick(), if sweeping.is_none() && e.slices.is_some() => {
                sweeping = Some(Box::pin(zenkey_fleet::prewarm(&e.fleet, &store, e.slices)));
                None
            }
            _ = async { sweeping.as_mut().expect("guarded").await }, if sweeping.is_some() => {
                sweeping = None;
                None
            }
            Some(_) = tasks.join_next(), if !tasks.is_empty() => None,
            item = events.recv() => match item {
                None => break,
                Some(StreamItem::Dropped(n)) => {
                    summary.dropped += n;
                    tracing::warn!(dropped = n, "the monitor's broadcast overflowed — unobservable, not ok");
                    Some(Notice::Dropped { n, at: zenkey_fleet::rfc3339_now() })
                }
                Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                    let Ok(key) = KeyExpr::try_from(s.key.as_str()) else { continue };
                    if !e.rules.iter().any(|r| matches!(r.kind, RuleKind::Alerts { .. }) && r.covers(&key)) {
                        continue;
                    }
                    let at = zenkey_fleet::rfc3339_now();
                    let stamped = s.timestamp.map(|t| t.to_string());
                    let (decoded, payload) = match s.kind {
                        SampleKind::Delete => (None, Payload::None { reason: "a tombstone carries no payload".into() }),
                        SampleKind::Put => {
                            let bytes = s.payload.to_bytes();
                            let d = zenkey_fleet::decode_sample(&e.fleet, &store, e.slices, &s.key, Some(&s.encoding), &bytes).await;
                            match d.rendering {
                                zenkey_fleet::Rendering::Typed(p) => (Some((RenderSource::Schema, p.value.clone())), Payload::Typed(p.value)),
                                zenkey_fleet::Rendering::Structural(text) => {
                                    // `structural` emits JSON text when the bytes
                                    // parsed as JSON or CBOR; that reading still
                                    // carries the document's fields.
                                    let value = serde_json::from_str::<serde_json::Value>(&text).ok();
                                    let what = d.type_name.clone().unwrap_or_else(|| s.key.clone());
                                    (value.map(|v| (RenderSource::Structural, v)), Payload::Structural { text, what })
                                }
                            }
                        }
                    };
                    let Some(t) = zenkey_fleet::alert_transition(
                        base, &s.key, s.kind, decoded.as_ref().map(|(src, v)| (*src, v)), stamped.as_deref(), &at,
                    ) else { continue };
                    let now = (t.state, t.severity.clone());
                    let prior = alerts.get(&s.key).cloned();
                    if prior.as_ref() == Some(&now) {
                        // Transitions, not states: a re-put of the same
                        // alert at the same severity is the refresh RFC 04
                        // §1.2 asks publishers for, not a new firing.
                        continue;
                    }
                    alerts.set(s.key.clone(), now);
                    Some(Notice::Alert { key: s.key.clone(), transition: Box::new(t), prior: prior.map(|(a, _)| cond_of(a)), payload })
                }
                Some(StreamItem::Event(FleetEvent::NodeUp(key))) => liveliness(&mut tokens, base, key, true, e.rules),
                Some(StreamItem::Event(FleetEvent::NodeDown(key))) => liveliness(&mut tokens, base, key, false, e.rules),
                Some(StreamItem::Event(_)) => None,
            },
        };
        if let Some(n) = notice {
            summary.notices += 1;
            for o in route(&n, e.rules, e.render, &mut seq) {
                summary.outgoing += 1;
                dispatch(&mut tasks, &e.sinks, &counters, o);
            }
        }
    }

    // Teardown, acknowledged: a completed watchdog hands back its summary;
    // an interrupted one is dropped the way `zenctl watchdog` drops it — it
    // runs until told to stop, and awaiting it would be waiting forever.
    if watchdog_done && let Some(run) = watchdog.take() {
        summary.watchdog = Some(run.await?);
    }
    drop(watchdog);
    monitor.shutdown().await?;
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
    summary.evicted = alerts.evicted() + tokens.evicted();
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

/// `zenwatch run`: load and refuse, build the sinks (secrets resolved now),
/// resolve the bus, open the session, and run until a signal.
pub async fn run(args: crate::cli::RunArgs) -> Result<()> {
    let cfg: Config = crate::load_checked(&args.config.config)?;
    let rules = cfg
        .rules
        .iter()
        .map(Rule::from_config)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| unaskable!("{e}"))?;
    let sinks =
        Arc::new(crate::sinks::build_all(&cfg, args.dry_run).map_err(|e| unaskable!("{e}"))?);
    let bus = crate::bus::resolve(&cfg.bus, args.context.as_deref())?;
    let session = bus.session().await?;
    let slices = bus.slices(&session).await?;
    let tick = Duration::from_secs_f64(cfg.tick_s);
    eprintln!(
        "zenwatch: {} rule(s), {} sink(s), tick {}s{}{} — three states, ok/firing/unobservable, \
         one notification per sink per genuine change (RFC 13 §3)",
        rules.len(),
        sinks.len(),
        cfg.tick_s,
        if args.dry_run {
            "; dry-run: printing, sending nothing"
        } else {
            ""
        },
        if slices.is_none() {
            "; no registry: payloads render structurally"
        } else {
            ""
        },
    );
    let fleet = Fleet::new(&session, &bus.base);
    let summary = run_on(
        Engine {
            fleet,
            slices: slices.as_ref(),
            timeout: bus.timeout,
            tick,
            rules: &rules,
            sinks,
            render: &cfg.render,
            once: args.once,
            ready: None,
        },
        stop_signal(),
    )
    .await?;
    eprintln!(
        "zenwatch: stopped — {} notice(s), {} notification(s): {} delivered, {} failed{}{}",
        summary.notices,
        summary.outgoing,
        summary.delivered,
        summary.failed,
        if summary.dropped > 0 {
            format!(
                ", {} event(s) dropped by the observer (O6)",
                summary.dropped
            )
        } else {
            String::new()
        },
        if summary.evicted > 0 {
            format!(", {} key(s) retired at the ledger bound", summary.evicted)
        } else {
            String::new()
        },
    );
    Ok(())
}
