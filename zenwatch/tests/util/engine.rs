//! The daemon's engine, run in the background against a test bus with a
//! capturing sink (#614, FL2): what every live suite drives.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_fleet::{BundleStore, ContractSet, DoctorBus};
use zenwatch::config::{DisciplineConfig, DoctorConfig, RuleConfig};
use zenwatch::engine::{Engine, Publish, RunSummary, run_on};
use zenwatch::render::RenderConfig;
use zenwatch::rules::Rule;
use zenwatch::sinks::{Outgoing, Sink};

/// What a capturing sink saw.
pub type Capture = Arc<Mutex<Vec<Outgoing>>>;

/// The daemon's own service in a test (#614, FL3): its configuration, and
/// the `hostid.v1` minter its address resolves through, on a root of the
/// test's own.
pub struct TestPublish {
    pub config: zenkey::ServiceConfig,
    pub hostid: zenkey::HostIdMinter,
    /// Holds the minter's root for as long as the run.
    pub _root: Option<tempfile::TempDir>,
}

impl TestPublish {
    /// At a literal address: nothing is minted.
    pub fn at(address: &str) -> TestPublish {
        let root = tempfile::tempdir().expect("a root");
        TestPublish {
            config: zenkey::ServiceConfig::at(address.parse().expect("an address")),
            hostid: zenkey::HostIdMinter::new(zenkey::HostIdSource::at(root.path()).read_only()),
            _root: Some(root),
        }
    }

    /// As the config's `publish` block says, minted (if it asks) on a host
    /// with no id (`hostid.v1` §2.6, its scenarios' §4 root): no machine-id
    /// file, and a shared directory the service cannot write, through the
    /// runtime's seam so a run as root refuses too.
    pub fn on_a_host_without_an_id(settings: &zenwatch::config::PublishSettings) -> TestPublish {
        let root = tempfile::tempdir().expect("a root");
        std::fs::create_dir_all(root.path().join("var/lib/zk2")).expect("the shared directory");
        let source = zenkey::HostIdSource::at(root.path())
            .with_fault(zenkey::hostid::Step::CreateTemp, libc::EACCES);
        TestPublish {
            config: zenwatch::publish::service_config(settings).expect("a service config"),
            hostid: zenkey::HostIdMinter::new(source),
            _root: Some(root),
        }
    }
}

/// How a test's engine runs; [`Options::default`] is a half-second tick,
/// no grouping, no state file, no self-publication, no doctor, no word on
/// the clocks, and one capturing sink, `ops`.
pub struct Options {
    pub tick: Duration,
    pub timeout: Duration,
    pub state_file: Option<PathBuf>,
    pub publish: Option<TestPublish>,
    pub doctor: Option<DoctorConfig>,
    pub discipline: DisciplineConfig,
    /// Sinks beside the capturing `ops`.
    pub extra_sinks: Vec<Sink>,
    /// The operator's word on the clocks (`clocks_synced`): every session
    /// of a test runs on this host.
    pub clocks_synced: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            tick: Duration::from_millis(500),
            timeout: Duration::from_secs(1),
            state_file: None,
            publish: None,
            doctor: None,
            extra_sinks: Vec::new(),
            clocks_synced: false,
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
    let mut sinks = vec![Sink::capturing("ops", Capture::default())];
    sinks.extend(opts.extra_sinks);
    run_on(
        Engine {
            bus: &bus,
            store: &store,
            contracts: &contracts,
            timeout: opts.timeout,
            tick: opts.tick,
            rules: &rules,
            sinks: Arc::new(sinks),
            render: &render,
            once: true,
            ready: None,
            discipline: &opts.discipline,
            state_file: opts.state_file.clone(),
            state_max_entries: 4096,
            publish: opts.publish.as_ref().map(|p| Publish {
                config: p.config.clone(),
                hostid: &p.hostid,
            }),
            doctor: None,
            clocks_synced: opts.clocks_synced,
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
        let mut sinks = vec![Sink::capturing("ops", ops)];
        sinks.extend(opts.extra_sinks);
        run_on(
            Engine {
                bus: &bus,
                store: &store,
                contracts: &contracts,
                timeout: opts.timeout,
                tick: opts.tick,
                rules: &rules,
                sinks: Arc::new(sinks),
                render: &render,
                once: false,
                ready: Some(ready_tx),
                discipline: &opts.discipline,
                state_file: opts.state_file.clone(),
                state_max_entries: 4096,
                publish: opts.publish.as_ref().map(|p| Publish {
                    config: p.config.clone(),
                    hostid: &p.hostid,
                }),
                doctor: opts.doctor.as_ref(),
                clocks_synced: opts.clocks_synced,
            },
            async move {
                let _ = stop_rx.await;
            },
        )
        .await
    });
    let ready = tokio::time::timeout(super::SETTLE, ready_rx)
        .await
        .expect("the engine came up");
    if ready.is_err() {
        // The run ended before it was ready: say why.
        match task.await {
            Ok(Err(e)) => panic!("the engine refused to come up: {e:#}"),
            other => panic!("the engine ended before it was ready: {other:?}"),
        }
    }
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
