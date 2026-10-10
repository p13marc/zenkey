//! The daemon's engine, run in the background against a test bus with a
//! capturing sink (#614, FL2): what every live suite drives.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_fleet::{BundleStore, ContractSet, DoctorBus};
use zenwatch::config::{DisciplineConfig, DoctorConfig, RuleConfig};
use zenwatch::engine::{Engine, RunSummary, run_on};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::{Outgoing, Sink};

/// What a capturing sink saw.
pub type Capture = Arc<Mutex<Vec<Outgoing>>>;

/// How a test's engine runs; [`Options::default`] is a half-second tick,
/// no grouping, no state file, no self-publication, no doctor.
pub struct Options {
    pub tick: Duration,
    pub timeout: Duration,
    pub state_file: Option<PathBuf>,
    pub publish: bool,
    pub doctor: Option<DoctorConfig>,
    pub discipline: DisciplineConfig,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            tick: Duration::from_millis(500),
            timeout: Duration::from_secs(1),
            state_file: None,
            publish: false,
            doctor: None,
            // No group window, and a group label no notice carries: a test
            // about a rule sees each notification on its own, whatever
            // system its address names.
            discipline: DisciplineConfig {
                group_window_s: 0.0,
                group_by: vec!["ungrouped".into()],
                ..DisciplineConfig::default()
            },
        }
    }
}

/// A running engine: stop it to get its summary.
pub struct Running {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<anyhow::Result<RunSummary>>,
}

impl Running {
    /// Stop, and the summary of a clean run.
    pub async fn stop(self) -> RunSummary {
        self.stopped().await.expect("a clean run")
    }

    /// Stop, and whatever the run returned.
    pub async fn stopped(self) -> anyhow::Result<RunSummary> {
        let _ = self.stop.send(());
        tokio::time::timeout(super::SETTLE, self.task)
            .await
            .expect("the engine stopped")
            .expect("no panic")
    }
}

/// A rule as a config spells it, sinking to `ops`.
pub fn rule(name: &str, text: &str, severity: &str) -> RuleConfig {
    RuleConfig {
        name: name.into(),
        rule: text.into(),
        severity: Some(severity.into()),
        labels: Default::default(),
        sinks: vec!["ops".into()],
        for_s: None,
    }
}

/// The engine's run, without waiting for `ready`: what a test that expects
/// a refusal before anything opens drives.
pub async fn run_once_refused(
    bus: DoctorBus,
    rules: Vec<RuleConfig>,
    opts: Options,
) -> anyhow::Result<RunSummary> {
    let rules: Vec<Rule> = rules
        .iter()
        .map(|r| Rule::from_config(r).expect("a rule"))
        .collect();
    let contracts = ContractSet::new();
    let store = BundleStore::new(opts.timeout);
    let render = RenderConfig::default();
    run_on(
        Engine {
            bus: &bus,
            store: &store,
            contracts: &contracts,
            timeout: opts.timeout,
            tick: opts.tick,
            rules: &rules,
            sinks: Arc::new(vec![Sink::capturing("ops", Capture::default())]),
            render: &render,
            once: true,
            ready: None,
            discipline: &opts.discipline,
            state_file: opts.state_file.clone(),
            state_max_entries: 4096,
            publish: false,
            doctor: None,
            clocks_synced: false,
        },
        std::future::pending::<()>(),
    )
    .await
}

/// Start an engine on `bus` with `rules` and one capturing sink, `ops`,
/// and wait until it is watching and seeded (the `ready` seam).
pub async fn start(bus: DoctorBus, rules: Vec<RuleConfig>, ops: Capture, opts: Options) -> Running {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let rules: Vec<Rule> = rules
            .iter()
            .map(|r| Rule::from_config(r).expect("a rule"))
            .collect();
        let contracts = ContractSet::new();
        let store = BundleStore::new(opts.timeout);
        let render = RenderConfig::default();
        run_on(
            Engine {
                bus: &bus,
                store: &store,
                contracts: &contracts,
                timeout: opts.timeout,
                tick: opts.tick,
                rules: &rules,
                sinks: Arc::new(vec![Sink::capturing("ops", ops)]),
                render: &render,
                once: false,
                ready: Some(ready_tx),
                discipline: &opts.discipline,
                state_file: opts.state_file.clone(),
                state_max_entries: 4096,
                publish: opts.publish,
                doctor: opts.doctor.as_ref(),
                clocks_synced: false,
            },
            async move {
                let _ = stop_rx.await;
            },
        )
        .await
    });
    tokio::time::timeout(super::SETTLE, ready_rx)
        .await
        .expect("the engine came up")
        .expect("ready");
    Running {
        stop: stop_tx,
        task,
    }
}

/// Until `pred` holds over what the sink captured, failing after
/// [`super::SETTLE`] with every kind, id and state seen.
pub async fn until(
    capture: &Capture,
    what: &str,
    pred: impl Fn(&[Outgoing]) -> bool,
) -> Vec<Outgoing> {
    let deadline = tokio::time::Instant::now() + super::SETTLE;
    loop {
        let snapshot = capture.lock().unwrap().clone();
        if pred(&snapshot) {
            return snapshot;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "waited {:?} for {what}; have {:?}",
            super::SETTLE,
            snapshot
                .iter()
                .map(|o| format!(
                    "{:?} {} {:?}",
                    o.notification.kind, o.notification.id, o.notification.state
                ))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
