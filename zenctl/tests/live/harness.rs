//! The live suite's bus: one in-process producer per case, and a way to run
//! the real `zenctl` binary against it (#499).
//!
//! ## The producer
//!
//! A zenoh session listening on a port the OS has just handed out (the
//! `endpoint()` pattern of `zenkey-fleet/tests/util/mod.rs`, which is not
//! exported), with no scouting and no external router, brought up the way
//! RFC 04 §5 says a producer is: `introspect` (a real slice), `describe`
//! (a schema for every type the slice names), a read procedure and the
//! config double's procedures, all declared through [`BringUp`] — and only
//! then `alive`. It publishes one state key and one telemetry key, and
//! answers a GET on the state key the way a storage would.
//!
//! The session runs in **router** mode. An explorer's session is a peer
//! today and becomes a client under #501; a router accepts either, so the
//! suite does not have to change when the explorer's mode does.
//!
//! Every bus gets a **base of its own** (`live-<pid>-<n>`), so two cases —
//! or two whole runs — never see each other's keys even if their
//! sessions ever met.
//!
//! ## Waiting
//!
//! Each `zenctl` run is a new process with a new session, and a query sent
//! before that session has met the producer's declarations is silence, not
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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use zenkey_fleet::bus::producer::{BringUp, LiveProducer};
use zenoh::Session;

use crate::config_server::ConfigServer;

/// How long a case waits before calling a hang a hang — the fleet suites'
/// net (#371), and for the same reason: a net, never an assertion. Every
/// wait under it returns the moment its outcome appears.
pub const SETTLE: Duration = Duration::from_secs(20);

/// How long one `zenctl` run may take before it is killed and reported as
/// hung. Above any window a case asks for, so it only ever fires on a hang.
pub const RUN_LIMIT: Duration = Duration::from_secs(60);

/// The producer's origin.
pub const HOST: &str = "h-11fe11fe11fe";
/// The producer's name.
pub const PRODUCER: &str = "probe";

/// The served slice (RFC 08 §6): one state subject, one telemetry subject,
/// one read procedure. Every type it names is in [`schema_set`].
const SLICE: &str = r#"
[registry]
version = "1.0"
app = "live"
convention = 1

[producer]
name = "probe"
description = "zenctl's live-suite producer (#499)"

[[subject]]
path = "health"
class = "state"
type = "Health"
since = "1.0"
description = "whether the probe is well"

[[subject]]
path = "cpu"
class = "telemetry"
type = "Cpu"
since = "1.0"
description = "a load reading"

[[procedure]]
path = "ping"
kind = "read"
reply = "Pong"
idempotent = true
since = "1.0"
description = "answers pong"
"#;

/// The `describe` reply (RFC 08 §7): a schema for each type [`SLICE`] names.
fn schema_set() -> String {
    let object = |props: serde_json::Value| {
        zenkey::schema::TypeSchema::json_schema(serde_json::json!({
            "type": "object",
            "properties": props,
        }))
    };
    zenkey::schema::SchemaSet::builder("live")
        .entry(
            "Health",
            object(serde_json::json!({ "status": { "type": "string" } })),
        )
        .entry(
            "Cpu",
            object(serde_json::json!({ "value": { "type": "number" } })),
        )
        .entry(
            "Pong",
            object(serde_json::json!({ "pong": { "type": "boolean" } })),
        )
        .build()
        .to_json()
}

/// The state value the producer publishes and answers a GET with.
pub const HEALTH: &str = r#"{"status":"ok"}"#;

/// How often the telemetry key publishes. Fast enough that a two-second
/// window holds dozens of samples, which is what `rate` and `check expect`
/// are pointed at.
const TELEMETRY_PERIOD: Duration = Duration::from_millis(50);

/// What else a bus carries beyond the healthy producer.
#[derive(Debug, Default, Clone, Copy)]
pub struct Extras {
    /// A second producer that holds `alive` and answers nothing — the
    /// RFC 04 §5 violation `doctor` files as `introspect-coverage`.
    pub mute: bool,
}

/// One case's bus.
pub struct Bus {
    pub endpoint: String,
    pub base: String,
    /// The config double the producer serves (RFC 05 §5.1, #500).
    pub config: ConfigServer,
    session: Session,
    _live: LiveProducer,
    _mute: Option<zenoh::liveliness::LivelinessToken>,
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

/// A loopback endpoint on a port the OS has just told us is free — released
/// the instant its number is known, because zenoh binds next.
fn endpoint() -> String {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = held.local_addr().expect("local addr").port();
    drop(held);
    format!("tcp/127.0.0.1:{port}")
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
    /// The healthy producer and nothing else.
    pub async fn up() -> Bus {
        Bus::with(Extras::default()).await
    }

    pub async fn with(extras: Extras) -> Bus {
        let endpoint = endpoint();
        let base = unique_base();

        let mut cfg = zenoh::Config::default();
        cfg.insert_json5("mode", "\"router\"").expect("mode");
        cfg.insert_json5("scouting/multicast/enabled", "false")
            .expect("scouting");
        cfg.insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
            .expect("listen");
        let session = zenoh::open(cfg).await.expect("the producer's session");

        let key = |rel: &str| zenkey::grammar::with_base(&base, rel);
        let rpc = key(&format!("v1/{HOST}/@rpc/{PRODUCER}"));
        let introspect = format!("{rpc}/introspect");
        let describe = format!("{rpc}/describe");
        let ping = format!("{rpc}/ping");

        // RFC 04 §5: every queryable first…
        let config = ConfigServer::fixture(PRODUCER);
        let mut up = BringUp::new(&session);
        up.serve(&introspect).await.expect("introspect");
        up.serve(&describe).await.expect("describe");
        up.serve(&ping).await.expect("ping");
        config.declare(&mut up, &rpc).await.expect("config");

        // The state key's last value, answered as a storage would.
        let health = key(&format!("v1/{HOST}/state/{PRODUCER}/health"));
        let storage = session
            .declare_queryable(&health)
            .await
            .expect("the state storage");

        // …and `alive` last.
        let mut live = up
            .alive(&key(&format!("v1/{HOST}/state/{PRODUCER}/alive")))
            .await
            .expect("alive");

        let mut tasks = Vec::new();
        let slice_type = zenkey::SliceFormat::Toml.media_type();
        let schemas = schema_set();
        for r in std::mem::take(&mut live.responders) {
            if config.owns(r.key()) {
                tasks.push(config.spawn(r));
                continue;
            }
            let (payload, encoding) = match r.key() {
                k if k == introspect => (SLICE.to_string(), slice_type),
                k if k == describe => (schemas.clone(), "application/json"),
                _ => (r#"{"pong":true}"#.to_string(), "application/json"),
            };
            tasks.push(tokio::spawn(async move {
                while let Some(q) = r.next().await {
                    let _ = r
                        .reply(&q, payload.clone().into_bytes(), Some(encoding))
                        .await;
                }
            }));
        }

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
            .declare_publisher(key(&format!("v1/{HOST}/telemetry/{PRODUCER}/cpu")))
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

        let mute = if extras.mute {
            Some(
                session
                    .liveliness()
                    .declare_token(key(&format!("v1/{HOST}/state/mute/alive")))
                    .await
                    .expect("the mute producer's token"),
            )
        } else {
            None
        };

        let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("live-home")
            .join(&base);
        std::fs::create_dir_all(&home).expect("a config root of its own");

        Bus {
            endpoint,
            base,
            config,
            session,
            _live: live,
            _mute: mute,
            tasks,
            home,
        }
    }

    /// A base-relative key as the wire spells it under this bus's base.
    pub fn key(&self, rel: &str) -> String {
        zenkey::grammar::with_base(&self.base, rel)
    }

    /// A subscriber on the producer's session — what receives a `pub` or a
    /// `retire`.
    pub async fn subscribe(
        &self,
        key: &str,
    ) -> zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>> {
        self.session
            .declare_subscriber(key.to_string())
            .await
            .expect("a test subscriber")
    }

    /// Run `zenctl <args> -c <endpoint> --base <base>` once.
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
        argv.extend([
            "-c".into(),
            self.endpoint.clone(),
            "--base".into(),
            self.base.clone(),
        ]);
        let home = self.home.clone();
        tokio::task::spawn_blocking(move || run(&argv, &home, input))
            .await
            .expect("the zenctl runner")
    }

    /// A queryable on the producer's session that answers every query on
    /// `key` and counts them — the witness a refused fleet call is asserted
    /// against: if the count did not move, nothing reached the producer.
    pub async fn counting_responder(&mut self, key: &str) -> Arc<AtomicUsize> {
        let queryable = self
            .session
            .declare_queryable(key.to_string())
            .await
            .expect("a counting responder");
        let heard = Arc::new(AtomicUsize::new(0));
        let count = heard.clone();
        let reply_key = key.to_string();
        self.tasks.push(tokio::spawn(async move {
            while let Ok(q) = queryable.recv_async().await {
                count.fetch_add(1, Ordering::SeqCst);
                let _ = q
                    .reply(reply_key.clone(), r#"{"done":true}"#)
                    .encoding("application/json")
                    .await;
            }
        }));
        heard
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
