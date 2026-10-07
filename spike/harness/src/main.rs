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
        Cmd::Router { listen, connect } => router(listen, connect).await,
        Cmd::Services { connect, mode, system, count, namespace, synthetic, contracts } => {
            let topo = Topo { mode, listen: Vec::new(), connect, namespace };
            services(&topo, &system, count, synthetic.as_deref(), &contracts).await
        }
        Cmd::Smoke { services, results, contracts } => smoke(services, &results, &contracts).await,
        Cmd::S1 { results, examples } => {
            if s1::run(&results, &examples).await? {
                Ok(())
            } else {
                bail!("S1: a case failed (see results/s1/summary.md)")
            }
        }
    }
}

async fn router(listen: Vec<String>, connect: Vec<String>) -> Result<()> {
    let s = Topo { mode: Mode::Router, listen, connect, namespace: None }.open().await?;
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

async fn services(
    topo: &Topo,
    system: &str,
    count: usize,
    synth: Option<&str>,
    files: &[PathBuf],
) -> Result<()> {
    let contracts = match synth {
        Some(s) => synthetic(s)?,
        None => load_contracts(files)?,
    };
    if contracts.is_empty() {
        bail!("no contracts");
    }
    let mut running = Vec::with_capacity(count);
    for i in 0..count {
        let session = topo.open().await?;
        let addr = Addr::new(system, &format!("svc-{i}"))?;
        let c = &contracts[i % contracts.len()];
        let s = Service::start(session, addr, instance_id(i), std::slice::from_ref(c), &Options::default())
            .await
            .with_context(|| format!("service {i} ({})", c.iface))?;
        running.push(s);
    }
    println!("ready {count}");
    tokio::signal::ctrl_c().await?;
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
