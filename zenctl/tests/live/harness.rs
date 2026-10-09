//! The live suite's bus: one in-process publisher per case, and a way to run
//! the real `zenctl` binary against it (#499).
//!
//! ## The publisher
//!
//! A zenoh session listening on loopback port 0 and reading back the port it
//! was given (the `ANY_PORT`/`bound` pattern of `zenkey-fleet/tests/util/mod.rs`,
//! which is not exported; #527), with no scouting and no external router. It
//! publishes one telemetry-shaped key on a period and one state-shaped key
//! on a heartbeat, and answers a GET on the state key the way a storage
//! would. Its keys are **foreign** — no zk2 service owns them — which is
//! what the raw verbs pinned here (`get`, `rate`, `pub`, `watchdog`) must
//! handle as well as a zk2 key; the zk2 cases are `tests/live_zk2.rs`.
//! v1's producer — an `introspect` slice and a `describe` behind `BringUp` —
//! left with the v1 registry (#612, FJ9).
//!
//! The session runs in **router** mode: an explorer's session is a client
//! (#501), and a router accepts it.
//!
//! Every bus gets a **base of its own** (`live-<pid>-<n>`), so two cases —
//! or two whole runs — never see each other's keys even if their
//! sessions ever met.
//!
//! ## Waiting
//!
//! Each `zenctl` run is a new process with a new session, and a query sent
//! before that session has met the publisher's declarations is silence, not
//! an answer. So a case that expects an answer asks [`Bus::until`]: rerun
//! until the outcome the case is about appears, within [`SETTLE`] — which
//! returns the instant it does, so a generous net costs a passing run
//! nothing. A case that expects *silence* runs once, after a sibling run
//! has already proved the bus answers. Nothing here sleeps a fixed time and
//! hopes.

#![allow(dead_code)]

use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenoh::Session;

/// How long a case waits before calling a hang a hang — the fleet suites'
/// net (#371), and for the same reason: a net, never an assertion. Every
/// wait under it returns the moment its outcome appears.
pub const SETTLE: Duration = Duration::from_secs(20);

/// How long one `zenctl` run may take before it is killed and reported as
/// hung. Above any window a case asks for, so it only ever fires on a hang.
pub const RUN_LIMIT: Duration = Duration::from_secs(60);

/// The base-relative keys the publisher writes.
pub const CPU: &str = "plant/line-1/cpu";
pub const HEALTH_KEY: &str = "plant/line-1/health";

/// The state value the publisher publishes and answers a GET with.
pub const HEALTH: &str = r#"{"status":"ok"}"#;

/// How often the telemetry key publishes. Fast enough that a two-second
/// window holds dozens of samples, which is what `rate` and `watchdog` are
/// pointed at.
const TELEMETRY_PERIOD: Duration = Duration::from_millis(50);

/// One case's bus.
pub struct Bus {
    pub endpoint: String,
    pub base: String,
    session: Session,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    home: PathBuf,
}

impl Drop for Bus {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        // The base names this run's pid, so a home left behind is never
        // reused — only accumulated.
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// A base no other bus — in this run or a concurrent one — has used.
fn unique_base() -> String {
    static NTH: AtomicU64 = AtomicU64::new(0);
    format!(
        "live-{}-{}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    )
}

impl Bus {
    /// The publisher, publishing.
    pub async fn up() -> Bus {
        let base = unique_base();

        let mut cfg = zenoh::Config::default();
        cfg.insert_json5("mode", "\"router\"").expect("mode");
        cfg.insert_json5("scouting/multicast/enabled", "false")
            .expect("scouting");
        // Port 0, read back after the bind: a port learned from one socket and
        // handed to zenoh after it is released can be taken in between — by
        // a concurrent run's outgoing connection, as often as not (#527).
        cfg.insert_json5("listen/endpoints", r#"["tcp/127.0.0.1:0"]"#)
            .expect("listen");
        let session = zenoh::open(cfg).await.expect("the publisher's session");
        let endpoint = session
            .info()
            .locators()
            .await
            .into_iter()
            .map(|l| l.to_string())
            .find(|l| l.starts_with("tcp/127.0.0.1:"))
            .expect("the publisher listens on loopback");
        let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("live-home")
            .join(&base);
        std::fs::create_dir_all(&home).expect("a config root of its own");

        let key = |rel: &str| format!("{base}/{rel}");
        let health = key(HEALTH_KEY);
        let storage = session
            .declare_queryable(&health)
            .await
            .expect("the state storage");

        let mut tasks = Vec::new();
        let reply_key = health.clone();
        tasks.push(tokio::spawn(async move {
            while let Ok(q) = storage.recv_async().await {
                let _ = q
                    .reply(reply_key.clone(), HEALTH)
                    .encoding("application/json")
                    .await;
            }
        }));

        // The data planes: state on change (here, a heartbeat), telemetry
        // on a period.
        let state_pub = session.declare_publisher(health).await.expect("state");
        let cpu = session
            .declare_publisher(key(CPU))
            .await
            .expect("telemetry");
        tasks.push(tokio::spawn(async move {
            let mut tick = tokio::time::interval(TELEMETRY_PERIOD);
            let mut n = 0u64;
            loop {
                tick.tick().await;
                let _ = cpu
                    .put(format!(r#"{{"value":{}}}"#, n % 100))
                    .encoding("application/json")
                    .await;
                if n.is_multiple_of(10) {
                    let _ = state_pub.put(HEALTH).encoding("application/json").await;
                }
                n = n.wrapping_add(1);
            }
        }));

        Bus {
            endpoint,
            base,
            session,
            tasks,
            home,
        }
    }

    /// A base-relative key as the wire spells it under this bus's base.
    pub fn key(&self, rel: &str) -> String {
        format!("{}/{rel}", self.base)
    }

    /// A subscriber on the producer's session — what receives a `pub`.
    pub async fn subscribe(
        &self,
        key: &str,
    ) -> zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>> {
        self.session
            .declare_subscriber(key.to_string())
            .await
            .expect("a test subscriber")
    }

    /// Run `zenctl <args> -c <endpoint>` once. A verb that reads a
    /// deployment is given `--namespace` by the case, through [`Bus::ns`]:
    /// `pub` takes none since FJ9.
    pub async fn zenctl(&self, args: &[&str]) -> Run {
        self.zenctl_fed(args, None).await
    }

    /// [`zenctl`](Self::zenctl), with `input` piped to its stdin — the
    /// `pub --from ndjson` shape. A pipe is no more a terminal than a closed
    /// stdin is.
    pub async fn zenctl_with_stdin(&self, args: &[&str], input: &str) -> Run {
        self.zenctl_fed(args, Some(input.to_string())).await
    }

    async fn zenctl_fed(&self, args: &[&str], input: Option<String>) -> Run {
        let mut argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        argv.extend(["-c".into(), self.endpoint.clone()]);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run(&argv, &home, input))
            .await
            .expect("the zenctl runner")
    }

    /// `args` with `--namespace <base>` after them, for a verb that resolves
    /// keys against the deployment.
    pub fn ns<'a>(&'a self, args: &[&'a str]) -> Vec<&'a str> {
        let mut v = args.to_vec();
        v.extend(["--namespace", self.base.as_str()]);
        v
    }

    /// Rerun until `done` holds, within [`SETTLE`]; the last run either way,
    /// for the caller to assert on.
    pub async fn until(&self, args: &[&str], done: impl Fn(&Run) -> bool) -> Run {
        let deadline = Instant::now() + SETTLE;
        loop {
            let r = self.zenctl(args).await;
            if done(&r) || Instant::now() >= deadline {
                return r;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// One finished `zenctl` run.
#[derive(Debug)]
pub struct Run {
    pub argv: Vec<String>,
    /// The exit code; `-1` for a run killed at [`RUN_LIMIT`] or by a signal.
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// stdout as one JSON document (`--format json`).
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not one JSON document ({e}):\n{self}"))
    }

    /// stdout as JSON lines (`--format ndjson`).
    pub fn ndjson(&self) -> Vec<serde_json::Value> {
        self.stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str(l)
                    .unwrap_or_else(|e| panic!("not a JSON line ({e}): {l:?}\n{self}"))
            })
            .collect()
    }

    /// Rows of one kind from an ndjson stream (`"row": <kind>`).
    pub fn rows(&self, kind: &str) -> Vec<serde_json::Value> {
        self.ndjson()
            .into_iter()
            .filter(|r| r["row"] == kind)
            .collect()
    }
}

impl std::fmt::Display for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "$ zenctl {}\nexit {}\n--- stdout\n{}--- stderr\n{}",
            self.argv.join(" "),
            self.code,
            self.stdout,
            self.stderr
        )
    }
}

/// The run itself, blocking: hermetic environment, stdin closed or a pipe
/// (so never a terminal — the non-interactive posture a script has), both
/// output pipes drained on threads of their own so a chatty run cannot fill
/// one and stall.
fn run(argv: &[String], home: &std::path::Path, input: Option<String>) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zenctl"))
        .args(argv)
        // The variables that would redirect a run (see `tests/cli.rs`'s
        // hermeticity note): a context, a config file, a format, a base.
        .env_remove("ZENCTL_CONTEXT")
        .env_remove("ZENKEY_EXPLORER_CONTEXT")
        .env_remove("ZENCTL_ZENOH_CONFIG")
        .env_remove("ZENCTL_FORMAT")
        .env_remove("ZENCTL_BASE")
        .env("ZENKEY_EXPLORER_CONFIG_DIR", home)
        .env("NO_COLOR", "1")
        .env("RUST_LOG", "off")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zenctl");
    if let Some(input) = input {
        // Written whole and closed, so the run sees an EOF and finishes.
        let mut stdin = child.stdin.take().expect("stdin");
        std::thread::spawn(move || {
            use std::io::Write as _;
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let drain = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = pipe.read_to_string(&mut s);
            s
        })
    };
    let out = drain(Box::new(child.stdout.take().expect("stdout")));
    let err = drain(Box::new(child.stderr.take().expect("stderr")));
    let deadline = Instant::now() + RUN_LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait on zenctl") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = err.join().expect("stderr drain");
    if status.is_none() {
        stderr.push_str(&format!("\n[killed: still running after {RUN_LIMIT:?}]\n"));
    }
    Run {
        argv: argv.to_vec(),
        code: status.and_then(|s| s.code()).unwrap_or(-1),
        stdout: out.join().expect("stdout drain"),
        stderr,
    }
}
