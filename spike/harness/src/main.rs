//! The zk2 spike harness (#591): routers, mock services, synthetic load and
//! the measuring clients. Every group writes CSV rows under
//! `spike/results/<group>/` and a markdown summary beside them.
//!
//! - `spike router`: one router session, until killed.
//! - `spike services`: N mock services from contract files, or synthetic
//!   ones (`--synthetic M:K`: M interfaces of K state resources each).
//! - `spike smoke`: the harness's own check. Two routers, N services behind
//!   the second, and a client on the first that waits for presence, reads a
//!   descriptor, fetches a contract by hash, does a state GET and a call,
//!   and writes a report row.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Contract, load_path, load_str};
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, KindToken, ZkKey, data_key, parse};
use zenkey_model::template::Bindings;
use zk2rt::client;
use zk2rt::config::{Mode, Topo, free_port};
use zk2rt::metrics::{Counting, csv_row, proc_sample};
use zk2rt::mock::Mocker;
use zk2rt::service::{Options, Service};

mod procs;
mod s1;
mod s10;
mod s11;
mod s12;
mod s13;
mod s14;
mod s15;
mod s2;
mod s3;
mod s4;
mod s5;
mod s6;
mod s7;
mod s9;
mod storage_router;

use procs::{Proc, spawn};

#[global_allocator]
static ALLOC: Counting = Counting;

#[derive(Parser)]
#[command(name = "spike", about = "The zk2 spike harness (#591)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// One router, until killed. Prints `ready <zid>` once open.
    Router {
        #[arg(long)]
        listen: Vec<String>,
        #[arg(long)]
        connect: Vec<String>,
        /// A JSON object of config overrides, `{ "path/in/config": value }`
        /// (an ACL, a usrpwd dictionary).
        #[arg(long)]
        extra_config: Option<PathBuf>,
    },
    /// N mock services, until killed. Prints `ready <n>` once all are up.
    Services {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long, default_value = "client")]
        mode: Mode,
        #[arg(long, default_value = "spike")]
        system: String,
        #[arg(long, default_value_t = 1)]
        count: usize,
        /// A session namespace (the deployment prefix), for the namespace
        /// variant of every topology.
        #[arg(long)]
        namespace: Option<String>,
        /// Index of the first service (`svc-<first>`), so several service
        /// processes can share one system.
        #[arg(long, default_value_t = 0)]
        first: usize,
        /// A fixed service name (with `--count 1`), e.g. `tc` on each host.
        #[arg(long)]
        service_name: Option<String>,
        /// Publish every stream resource at this rate, with (seq, stamp) in
        /// the attachment.
        #[arg(long)]
        stream_hz: Option<f64>,
        /// `M:K`: M synthetic interfaces of K state resources each.
        #[arg(long)]
        synthetic: Option<String>,
        /// Contract files; service i implements contract i mod len.
        contracts: Vec<PathBuf>,
    },
    /// S1, grammar basics (#597): round trips, the key-algebra guard,
    /// namespaces, advanced pub/sub, wildcard puts.
    S1 {
        #[arg(long, default_value = "results/s1")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
    },
    /// S10, wildcard bindings (#593): fan-in, churn, a system wildcard,
    /// injection, and the graph from descriptors.
    S10 {
        #[arg(long, default_value = "results/s10")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
    },
    /// S11, control arbitration (#594): P3 bindings against the P2 sink.
    S11 {
        #[arg(long, default_value = "results/s11")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
    },
    /// S11's commander child (spawned by `spike s11`).
    #[command(hide = true)]
    S11Cmd {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long)]
        name: String,
        #[arg(long)]
        p2: bool,
        #[arg(long, default_value_t = 50.0)]
        hz: f64,
        #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
        skew_ms: i64,
        #[arg(long)]
        contract: PathBuf,
    },
    /// S13, simulation and replay by rebinding (#596).
    S13 {
        #[arg(long, default_value = "results/s13")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
        /// A v1 zenctl, for record and replay.
        #[arg(long, env = "ZENCTL", default_value = "zenctl")]
        zenctl: PathBuf,
    },
    /// S13's detector child.
    #[command(hide = true)]
    S13Detector {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long)]
        bindings: PathBuf,
        #[arg(long, default_value_t = 2.0)]
        secs: f64,
    },
    /// S13's clock.v1 sketch.
    #[command(hide = true)]
    S13Clock {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long, default_value_t = 1.0)]
        speed: f64,
    },
    /// S14, the ownership ACL on a live router (#616).
    S14 {
        #[arg(long, default_value = "results/s14")]
        results: PathBuf,
    },
    /// S15, a zenoh-pico participant as a zk2 owner (#617).
    S15 {
        #[arg(long, default_value = "results/s15")]
        results: PathBuf,
        /// The built `zk2_pico` binary.
        #[arg(long, default_value = "s15/build/zk2_pico")]
        pico: PathBuf,
    },
    /// S2, presence at fleet scale (#598).
    S2 {
        #[arg(long, default_value = "results/s2")]
        results: PathBuf,
        #[arg(long)]
        quick: bool,
    },
    /// S2's default-handler liveliness GET child.
    #[command(hide = true)]
    S2Get {
        #[arg(long)]
        connect: Vec<String>,
        /// Watch presence first, until this many tokens are known.
        #[arg(long, default_value_t = 0)]
        subscribe_first: usize,
    },
    /// S2's token-holder child.
    #[command(hide = true)]
    S2Tokens {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long, value_enum)]
        layout: s2::Layout,
        #[arg(long)]
        services: usize,
        #[arg(long, default_value_t = 0)]
        first: usize,
        #[arg(long, default_value_t = 5)]
        interfaces: usize,
        #[arg(long, default_value_t = 0)]
        members: usize,
        #[arg(long, default_value_t = 1)]
        sessions: usize,
        #[arg(long, default_value_t = 0.0)]
        churn_hz: f64,
        #[arg(long)]
        descriptor: bool,
    },
    /// S3, discovery over constrained links (#599).
    S3 {
        #[arg(long, default_value = "results/s3")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
    },
    /// S4, contract retrieval by hash against bad holders (#600).
    S4 {
        #[arg(long, default_value = "results/s4")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
    },
    /// S4's holder child.
    #[command(hide = true)]
    S4Holder {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long, value_enum)]
        mode: s4::HolderMode,
        #[arg(long, default_value_t = 1)]
        count: usize,
        contract: PathBuf,
    },
    /// S6, operations and ownership (#602).
    S6 {
        #[arg(long, default_value = "results/s6")]
        results: PathBuf,
    },
    /// S6's server child.
    #[command(hide = true)]
    S6Server {
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long)]
        key: String,
        #[arg(long, value_enum, default_value = "ok")]
        mode: s6::ServerMode,
        #[arg(long)]
        fanout_forbidden: bool,
        #[arg(long, default_value_t = 1)]
        replies: u32,
        #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
        complete: bool,
        #[arg(long)]
        instance_token: Option<String>,
        #[arg(long)]
        alive_token: Option<String>,
    },
    /// S5, state correctness with producer and storage (#601).
    S5 {
        #[arg(long, default_value = "results/s5")]
        results: PathBuf,
    },
    /// S12, store-and-forward commanding (#595).
    S12 {
        #[arg(long, default_value = "results/s12")]
        results: PathBuf,
        /// Which storage manager build this binary links (stock or patched).
        #[arg(long, default_value = "stock")]
        storage: String,
    },
    /// A router with the storage manager linked in, memory storages.
    StorageRouter {
        #[arg(long)]
        listen: Vec<String>,
        #[arg(long)]
        connect: Vec<String>,
        /// `name=keyexpr`, repeatable.
        #[arg(long)]
        storage: Vec<String>,
        #[arg(long, default_value_t = 30)]
        gc_period: u64,
        #[arg(long, default_value_t = 86400)]
        gc_lifespan: u64,
        #[arg(long)]
        replication: bool,
    },
    /// S7, bundle stability and classifier feasibility (#603).
    S7 {
        #[arg(long, default_value = "results/s7")]
        results: PathBuf,
        #[arg(long, default_value = "../examples/zk2")]
        examples: PathBuf,
        #[arg(long, default_value = "s7/matrix")]
        matrix: PathBuf,
        /// The `buf` binary (1.73.0 was used).
        #[arg(long, env = "BUF", default_value = "buf")]
        buf: PathBuf,
    },
    /// S9, the typed layer's cost over raw zenoh (#592).
    S9 {
        #[arg(long, default_value = "results/s9")]
        results: PathBuf,
        /// About a second per case, for checking the harness.
        #[arg(long)]
        quick: bool,
        /// Only the cases whose name contains this.
        #[arg(long)]
        only: Option<String>,
        /// Repetitions of the whole matrix, interleaved.
        #[arg(long, default_value_t = 1)]
        repeat: usize,
    },
    /// The bundle size of each contract file.
    #[command(hide = true)]
    BundleSizes { contracts: Vec<PathBuf> },
    /// How much zenoh-shm locks for a pool and one allocation.
    #[command(hide = true)]
    ShmProbe { pool: usize, alloc: usize },
    /// S9's receiver child (spawned by `spike s9`).
    #[command(hide = true)]
    S9Recv {
        #[arg(long, default_value = "client")]
        mode: Mode,
        #[arg(long)]
        listen: Vec<String>,
        #[arg(long)]
        connect: Vec<String>,
        #[arg(long, action = clap::ArgAction::Set)]
        shm: bool,
        #[arg(long)]
        key: String,
        #[arg(long, value_enum)]
        kind: s9::Kind,
        #[arg(long)]
        count: usize,
    },
    /// The smoke check: 2 routers, N services, one client; one report row.
    Smoke {
        #[arg(long, default_value_t = 10)]
        services: usize,
        #[arg(long, default_value = "results/smoke")]
        results: PathBuf,
        contracts: Vec<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Router { listen, connect, extra_config } => router(listen, connect, extra_config).await,
        Cmd::Services { connect, mode, system, count, namespace, first, service_name, stream_hz, synthetic, contracts } => {
            let topo = Topo { mode, listen: Vec::new(), connect, namespace, shm: None };
            let opts = ServicesOpts { first, service_name, stream_hz };
            services(&topo, &system, count, synthetic.as_deref(), &contracts, &opts).await
        }
        Cmd::Smoke { services, results, contracts } => smoke(services, &results, &contracts).await,
        Cmd::S9 { results, quick, only, repeat } => s9::run(&results, quick, only.as_deref(), repeat).await,
        Cmd::ShmProbe { pool, alloc } => s9::shm_probe(pool, alloc),
        Cmd::BundleSizes { contracts } => {
            for c in load_contracts(&contracts)? {
                println!("{:>8} {}", zenkey_model::bundle::Bundle::build(&c).to_bytes().len(), c.iface);
            }
            Ok(())
        }
        Cmd::S9Recv { mode, listen, connect, shm, key, kind, count } => {
            s9::recv(mode, listen, connect, shm, key, kind, count).await
        }
        Cmd::S10 { results, examples } => {
            if s10::run(&results, &examples).await? {
                Ok(())
            } else {
                bail!("S10: a case failed (see results/s10/summary.md)")
            }
        }
        Cmd::S11 { results, examples } => {
            if s11::run(&results, &examples).await? {
                Ok(())
            } else {
                bail!("S11: a case failed (see results/s11/summary.md)")
            }
        }
        Cmd::S11Cmd { connect, name, p2, hz, skew_ms, contract } => s11::commander(connect, name, p2, hz, skew_ms, &contract).await,
        Cmd::S13 { results, examples, zenctl } => {
            if s13::run(&results, &examples, &zenctl).await? {
                Ok(())
            } else {
                bail!("S13: a case failed (see results/s13/summary.md)")
            }
        }
        Cmd::S13Detector { connect, namespace, bindings, secs } => s13::detector(connect, namespace, bindings, secs).await,
        Cmd::S13Clock { connect, speed } => s13::clock(connect, speed).await,
        Cmd::S2 { results, quick } => s2::run(&results, quick).await,
        Cmd::S14 { results } => s14::run(&results).await.map(|_| ()),
        Cmd::S3 { results, examples } => s3::run(&results, &examples).await,
        Cmd::S15 { results, pico } => s15::run(&results, &pico).await,
        Cmd::S2Get { connect, subscribe_first } => s2::get_child(connect, subscribe_first).await,
        Cmd::S2Tokens { connect, layout, services, first, interfaces, members, sessions, churn_hz, descriptor } => {
            s2::tokens(connect, layout, services, first, interfaces, members, sessions, churn_hz, descriptor).await
        }
        Cmd::S4 { results, examples } => {
            if s4::run(&results, &examples).await? {
                Ok(())
            } else {
                bail!("S4: a case failed (see results/s4/summary.md)")
            }
        }
        Cmd::S4Holder { connect, mode, count, contract } => s4::holder(connect, &contract, mode, count).await,
        Cmd::S7 { results, examples, matrix, buf } => s7::run(&results, &examples, &matrix, &buf).await,
        Cmd::S6 { results } => {
            if s6::run(&results).await? {
                Ok(())
            } else {
                bail!("S6: a case failed (see results/s6/summary.md)")
            }
        }
        Cmd::S6Server { connect, key, mode, fanout_forbidden, replies, complete, instance_token, alive_token } => {
            s6::server(connect, key, mode, fanout_forbidden, replies, complete, instance_token, alive_token).await
        }
        Cmd::S5 { results } => s5::run(&results).await.map(|_| ()),
        Cmd::S12 { results, storage } => s12::run(&results, &storage).await.map(|_| ()),
        Cmd::StorageRouter { listen, connect, storage, gc_period, gc_lifespan, replication } => {
            storage_router::run(listen, connect, storage, gc_period, gc_lifespan, replication).await
        }
        Cmd::S1 { results, examples } => {
            if s1::run(&results, &examples).await? {
                Ok(())
            } else {
                bail!("S1: a case failed (see results/s1/summary.md)")
            }
        }
    }
}

async fn router(listen: Vec<String>, connect: Vec<String>, extra: Option<PathBuf>) -> Result<()> {
    let topo = Topo { mode: Mode::Router, listen, connect, namespace: None, shm: None };
    let mut c = topo.config()?;
    if let Some(p) = extra {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&p)?)?;
        for (k, val) in v.as_object().ok_or_else(|| anyhow!("--extra-config is a JSON object"))? {
            c.insert_json5(k, &val.to_string()).map_err(|e| anyhow!("config {k}: {e}"))?;
        }
    }
    let s = zenoh::open(c).await.map_err(|e| anyhow!("open router: {e}"))?;
    println!("ready {}", s.zid());
    tokio::signal::ctrl_c().await?;
    Ok(())
}

/// Loads contract files; a file with any error stops the run.
fn load_contracts(paths: &[PathBuf]) -> Result<Vec<Arc<Contract>>> {
    paths
        .iter()
        .map(|p| {
            let l = load_path(p);
            l.contract.map(Arc::new).ok_or_else(|| anyhow!("{}:\n{}", p.display(), l.report))
        })
        .collect()
}

/// M interfaces `load.i<m>.v1`, each with K state resources of a raw type.
fn synthetic(spec: &str) -> Result<Vec<Arc<Contract>>> {
    let (m, k) = spec.split_once(':').ok_or_else(|| anyhow!("--synthetic is M:K"))?;
    let (m, k): (usize, usize) = (m.parse()?, k.parse()?);
    (0..m)
        .map(|i| {
            let mut src = format!("[interface]\nname = \"load.i{i}\"\nmajor = 1\nminor = 0\n");
            for j in 0..k {
                src.push_str(&format!("[resources.r{j}]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n"));
            }
            let l = load_str(&src, Path::new("."), None);
            l.contract.map(Arc::new).ok_or_else(|| anyhow!("synthetic {i}: {}", l.report))
        })
        .collect()
}

fn instance_id(i: usize) -> InstanceId {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mix = (nanos as u64) ^ (u64::from(std::process::id()) << 32) ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    InstanceId::from_u64(mix)
}

/// How a `spike services` process names and drives its services.
struct ServicesOpts {
    first: usize,
    service_name: Option<String>,
    stream_hz: Option<f64>,
}

async fn services(
    topo: &Topo,
    system: &str,
    count: usize,
    synth: Option<&str>,
    files: &[PathBuf],
    opts: &ServicesOpts,
) -> Result<()> {
    let contracts = match synth {
        Some(s) => synthetic(s)?,
        None => load_contracts(files)?,
    };
    if contracts.is_empty() {
        bail!("no contracts");
    }
    let mut running = Vec::with_capacity(count);
    for i in opts.first..opts.first + count {
        let session = topo.open().await?;
        let name = opts.service_name.clone().unwrap_or_else(|| format!("svc-{i}"));
        let addr = Addr::new(system, &name)?;
        let c = &contracts[i % contracts.len()];
        let s = Service::start(session, addr, instance_id(i), std::slice::from_ref(c), &Options::default())
            .await
            .with_context(|| format!("service {i} ({})", c.iface))?;
        running.push(s);
    }
    println!("ready {count}");
    let running = Arc::new(running);
    let mut ticker = None;
    if let Some(hz) = opts.stream_hz {
        let r = running.clone();
        ticker = Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / hz));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut seq = 0u64;
            loop {
                tick.tick().await;
                for s in r.iter() {
                    let _ = s.tick_streams(seq).await;
                }
                seq += 1;
            }
        }));
    }
    tokio::signal::ctrl_c().await?;
    // A clean exit: stop publishing, then undeclare everything (tokens
    // first, by the Service's field order) before the process ends.
    if let Some(t) = ticker {
        t.abort();
        let _ = t.await;
    }
    drop(running);
    Ok(())
}

const SMOKE_HEADER: &[&str] = &[
    "unix_s",
    "zenoh",
    "routers",
    "services",
    "instance_tokens",
    "alive_tokens",
    // Time until a liveliness GET from the client sees every instance
    // token. The services were all up first, so this is presence *read*
    // latency through two routers, not cold discovery (that is S2, #598).
    "presence_ms",
    "descriptor_bytes",
    "contract",
    "contract_bytes",
    "contract_attempts",
    "contract_fetch_ms",
    "state_key",
    "state_get_ms",
    "state_has_timestamp",
    "state_wildcard_replies",
    "op_key",
    "call_ms",
    "client_allocs",
    "client_alloc_bytes",
    "router_a_rss_kib",
    "router_b_rss_kib",
    "services_rss_kib",
    "ok",
];

#[allow(clippy::too_many_lines)]
async fn smoke(n: usize, results: &Path, files: &[PathBuf]) -> Result<()> {
    let contracts = load_contracts(files)?;
    if contracts.is_empty() {
        bail!("smoke needs contract files");
    }
    let t = Duration::from_secs(5);
    let (pa, pb) = (free_port()?, free_port()?);
    let (ea, eb) = (format!("tcp/127.0.0.1:{pa}"), format!("tcp/127.0.0.1:{pb}"));
    let ready = Duration::from_secs(30);
    let ra = spawn(&["router".into(), "--listen".into(), ea.clone()], ready).await?;
    let rb = spawn(&["router".into(), "--listen".into(), eb.clone(), "--connect".into(), ea.clone()], ready).await?;
    let mut args = vec![
        "services".to_owned(),
        "--connect".into(),
        eb.clone(),
        "--system".into(),
        "smoke".into(),
        "--count".into(),
        n.to_string(),
    ];
    args.extend(files.iter().map(|p| p.display().to_string()));
    let sv = spawn(&args, Duration::from_secs(120)).await?;
    let session = Topo::client(std::slice::from_ref(&ea)).open().await?;
    // Allocations of this client from here on: presence, descriptor,
    // contract fetch and verification, the GETs and the call.
    let (a0, b0) = zk2rt::metrics::allocations();

    // Presence: every instance token, through two routers.
    let start = Instant::now();
    let instances = loop {
        let k = client::tokens(&session, "zk2/smoke/*/@zk/instance/*", t).await?;
        if k.len() >= n || start.elapsed() > Duration::from_secs(20) {
            break k;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let presence_ms = start.elapsed().as_secs_f64() * 1e3;
    let alive = client::tokens(&session, "zk2/smoke/*/@zk/alive/**", t).await?;
    for k in instances.iter().chain(&alive) {
        parse(k).map_err(|e| anyhow!("token {k} does not parse: {e}"))?;
    }

    // The descriptor of svc-0, then its contract by hash.
    let inst0 = instances
        .iter()
        .find(|k| k.starts_with("zk2/smoke/svc-0/"))
        .ok_or_else(|| anyhow!("svc-0 never appeared"))?;
    let desc = client::descriptor(&session, inst0, t).await?;
    let descriptor_bytes = serde_json::to_vec(&desc)?.len();
    let d0 = &desc["interfaces"][0];
    let iface: IfaceId = d0["iface"].as_str().unwrap_or_default().parse()?;
    let fp = Fingerprint::parse(d0["contract"].as_str().unwrap_or_default())?;
    let fetched = client::fetch_contract(&session, &iface, &fp, t).await?;

    // A state GET and a call, on the first service whose contract has one.
    let mut state_key = String::new();
    let mut op: Option<(String, Arc<Contract>, usize)> = None;
    for k in &alive {
        let Ok(ZkKey::Alive { addr, iface, .. }) = parse(k) else { continue };
        let Some(c) = contracts.iter().find(|c| c.iface == iface) else { continue };
        for (ri, r) in c.resources.iter().enumerate() {
            if r.template.has_params() {
                continue;
            }
            let chunks = r.template.build(&Bindings::new()).map_err(|e| anyhow!(e))?;
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            let key = data_key(&addr, &iface, r.token, &refs)?.as_str().to_owned();
            if r.token == KindToken::State && state_key.is_empty() {
                state_key = key;
            } else if r.token == KindToken::Op && op.is_none() {
                op = Some((key, c.clone(), ri));
            }
        }
    }
    let (mut state_get_ms, mut state_ts) = (f64::NAN, false);
    if !state_key.is_empty() {
        let s = Instant::now();
        let got = client::state_get(&session, &state_key, t).await?;
        state_get_ms = s.elapsed().as_secs_f64() * 1e3;
        state_ts = matches!(got.as_slice(), [Ok(g)] if g.timestamp.is_some() && g.value.is_some());
    }
    let wildcard = client::state_get(&session, "zk2/smoke/*/*/state/**", t).await?;
    let (mut op_key, mut call_ms, mut call_ok) = (String::new(), f64::NAN, false);
    if let Some((key, c, ri)) = op {
        let Body::Operation(o) = &c.resources[ri].body else { unreachable!() };
        let req = Mocker::new(&c).payload(&o.request, o.encoding)?;
        let s = Instant::now();
        let got = client::call(&session, &key, (req.bytes, req.encoding), t).await?;
        call_ms = s.elapsed().as_secs_f64() * 1e3;
        call_ok = matches!(got.as_slice(), [Ok(g)] if g.value.is_some());
        op_key = key;
    }

    let (a1, b1) = zk2rt::metrics::allocations();
    let rss = |p: &Proc| proc_sample(p.pid).map(|s| s.rss_kib).unwrap_or(0);
    let ok = instances.len() == n
        && alive.len() >= n
        && fetched.attempts == 1
        && !state_key.is_empty()
        && state_ts
        && !op_key.is_empty()
        && call_ok;
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let row = vec![
        unix.to_string(),
        zk2rt::ZENOH_VERSION.to_owned(),
        "2".into(),
        n.to_string(),
        instances.len().to_string(),
        alive.len().to_string(),
        format!("{presence_ms:.1}"),
        descriptor_bytes.to_string(),
        format!("{iface} {fp}"),
        fetched.bytes.to_string(),
        fetched.attempts.to_string(),
        format!("{:.2}", fetched.elapsed.as_secs_f64() * 1e3),
        state_key.clone(),
        format!("{state_get_ms:.2}"),
        state_ts.to_string(),
        wildcard.iter().filter(|r| r.is_ok()).count().to_string(),
        op_key.clone(),
        format!("{call_ms:.2}"),
        (a1 - a0).to_string(),
        (b1 - b0).to_string(),
        rss(&ra).to_string(),
        rss(&rb).to_string(),
        rss(&sv).to_string(),
        ok.to_string(),
    ];
    let csv = results.join("smoke.csv");
    csv_row(&csv, SMOKE_HEADER, &row)?;
    write_summary(&csv, &results.join("summary.md"))?;
    for (h, v) in SMOKE_HEADER.iter().zip(&row) {
        println!("{h:>22}  {v}");
    }
    drop((ra, rb, sv));
    if !ok {
        bail!("smoke failed");
    }
    Ok(())
}

/// The markdown summary: the CSV as a table, newest row last.
fn write_summary(csv: &Path, md: &Path) -> Result<()> {
    let text = std::fs::read_to_string(csv)?;
    let mut lines = text.lines();
    let head = lines.next().unwrap_or_default();
    let cols = head.split(',').count();
    let mut out = String::from("# spike-smoke\n\nWritten by `spike smoke` (#591); one row per run.\n\n");
    out.push_str(&format!("| {} |\n|{}\n", head.replace(',', " | "), "---|".repeat(cols)));
    for l in lines {
        out.push_str(&format!("| {} |\n", l.replace(',', " | ")));
    }
    std::fs::write(md, out)?;
    Ok(())
}
