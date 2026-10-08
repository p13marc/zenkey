//! S3, discovery over constrained links (#599, r3 §7 S3).
//!
//! A vehicle router (services, a storage) reaches a ground router (the
//! tools) through a shaping proxy: a one-way delay line and a bandwidth cap
//! that behaves like a serial line (`zk2rt::proxy`), with the bytes counted
//! in each direction. Windowed measures run until the link is quiet. `tc`/netem is not available on
//! this host, so loss is not emulated: over TCP, loss shows as retransmission
//! delay. Profiles: 600 ms RTT at 64 kbit/s and at 1 Mbit/s; an RF class at
//! 2,400 bit/s with 200 ms RTT (zenoh-modem's 220 B MTU is not modelled);
//! and the same RF class with `@zk` denied on the link face (R7): both
//! routers carry an ACL on their TCP face, the proxied link, while their
//! local clients reach them by unix socket.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::load_path;
use zenoh::sample::SampleKind;
use zk2rt::config::Topo;
use zk2rt::metrics::csv_row;
use zk2rt::proxy::{Proxy, Shape};

use crate::procs::{self, Proc};

struct Profile {
    name: &'static str,
    /// For the config file's name.
    tag: &'static str,
    /// `transport/link/tx/batch_size` on both routers (zenoh's default is
    /// 65,535 B).
    batch: Option<u16>,
    shape: Shape,
    /// The timeout for a GET, scaled to the link.
    timeout: Duration,
    /// `@zk` crosses the link. Off: both routers deny `@zk` on the link
    /// face, and consumers bind statically (R7).
    presence: bool,
    /// No ground router: the ground's sessions are zenoh clients of the
    /// vehicle router across the link, so only the declarations they show
    /// interest in cross it.
    ground_client: bool,
}

struct Row {
    profile: &'static str,
    measure: String,
    seconds: f64,
    bytes_up: u64,
    bytes_down: u64,
    note: String,
}

struct Link {
    _rv: Proc,
    rv: String,
    /// None when the ground's sessions are clients of RV across the link.
    _rg: Option<Proc>,
    /// Where the ground's sessions connect: RG, or the link itself.
    rg: String,
    proxy: Proxy,
    /// The proxy's client side is the ground (`ground_client`), not RV.
    ground_dials: bool,
}

impl Link {
    /// Bytes over the link so far: (vehicle → ground, ground → vehicle).
    fn bytes(&self) -> (u64, u64) {
        let (to_upstream, to_client) = self.proxy.bytes();
        if self.ground_dials { (to_client, to_upstream) } else { (to_upstream, to_client) }
    }
}

/// Ground router RG; vehicle router RV connecting to RG through the proxy.
/// Router config overrides (the profile's batch size, R7's ACL) go to both
/// routers, from `link-<tag>.json` in `dir`. With `@zk` denied, the ACL's
/// subject is the TCP face (the link), and local clients reach both routers
/// by unix socket.
async fn link(p: &Profile, dir: &Path) -> Result<Link> {
    let tcp = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let target = tcp.strip_prefix("tcp/").unwrap_or(&tcp).to_owned();
    let dir = std::fs::canonicalize(dir)?;
    let mut cfg = serde_json::Map::new();
    if let Some(b) = p.batch {
        cfg.insert("transport/link/tx/batch_size".into(), b.into());
    }
    if !p.presence {
        cfg.insert("access_control".into(), deny_zk_acl());
    }
    let extra = if cfg.is_empty() {
        None
    } else {
        let f = dir.join(format!("link-{}.json", p.tag));
        std::fs::write(&f, serde_json::to_string_pretty(&cfg)?)?;
        Some(f)
    };
    if p.presence {
        let rg_p = procs::router_with(std::slice::from_ref(&tcp), &[], extra.as_deref()).await?;
        let proxy = Proxy::shaped(target, p.shape).await?;
        let rv = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
        let rv_p = procs::router_with(std::slice::from_ref(&rv), &[proxy.endpoint()], extra.as_deref()).await?;
        return Ok(Link { _rv: rv_p, rv, _rg: Some(rg_p), rg: tcp, proxy, ground_dials: false });
    }
    let sock = |name: &str| -> String {
        let f: PathBuf = dir.join(format!("{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&f);
        format!("unixsock-stream/{}", f.display())
    };
    let (rg, rv) = (sock("rg"), sock("rv"));
    if p.ground_client {
        let rv_p = procs::router_with(&[tcp, rv.clone()], &[], extra.as_deref()).await?;
        let proxy = Proxy::shaped(target, p.shape).await?;
        let ep = proxy.endpoint();
        return Ok(Link { _rv: rv_p, rv, _rg: None, rg: ep, proxy, ground_dials: true });
    }
    let rg_p = procs::router_with(&[tcp, rg.clone()], &[], extra.as_deref()).await?;
    let proxy = Proxy::shaped(target, p.shape).await?;
    let rv_p = procs::router_with(std::slice::from_ref(&rv), &[proxy.endpoint()], extra.as_deref()).await?;
    Ok(Link { _rv: rv_p, rv, _rg: Some(rg_p), rg, proxy, ground_dials: false })
}

/// R7's posture as router config: every message on a `@zk` key, either way,
/// denied on TCP faces (here, only the link).
fn deny_zk_acl() -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "default_permission": "allow",
        "rules": [{
            "id": "zk-off-the-link",
            "messages": ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable", "liveliness_token", "liveliness_query", "declare_liveliness_subscriber"],
            "flows": ["egress", "ingress"],
            "permission": "deny",
            "key_exprs": ["zk2/**/@zk/**"],
        }],
        "subjects": [{ "id": "link", "link_protocols": ["tcp"] }],
        "policies": [{ "id": "r7", "rules": ["zk-off-the-link"], "subjects": ["link"] }],
    })
}

/// Waits until the link is quiet (at most 64 B either way over `quiet`,
/// which lets keep-alives through); returns the seconds to the last burst.
async fn settle(l: &Link, quiet: Duration, limit: Duration) -> f64 {
    let t0 = Instant::now();
    let mut window = std::collections::VecDeque::new();
    loop {
        let (u, d) = l.bytes();
        let now = Instant::now();
        window.push_back((now, u + d));
        while window.front().is_some_and(|(t, _)| now.duration_since(*t) > quiet) {
            window.pop_front();
        }
        let span = window.front().map_or(Duration::ZERO, |(t, _)| now.duration_since(*t));
        let moved = window.back().map_or(0, |b| b.1) - window.front().map_or(0, |f| f.1);
        if (span + Duration::from_millis(100) >= quiet && moved <= 64) || t0.elapsed() >= limit {
            return (t0.elapsed().saturating_sub(quiet)).as_secs_f64();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Bytes over the link (vehicle → ground, ground → vehicle) during `f`.
async fn measured<F, T>(l: &Link, f: F) -> Result<(T, f64, u64, u64)>
where
    F: std::future::Future<Output = Result<T>>,
{
    let (u0, d0) = l.bytes();
    let t0 = Instant::now();
    let v = f.await?;
    let s = t0.elapsed().as_secs_f64();
    let (u1, d1) = l.bytes();
    Ok((v, s, u1 - u0, d1 - d0))
}

#[allow(clippy::too_many_lines)]
async fn profile(p: &Profile, examples: &Path, results: &Path, rows: &mut Vec<Row>) -> Result<()> {
    let l = link(p, results).await?;
    let ground = Topo::client(&[l.rg.clone()]).open().await?;
    // The vehicle: 50 mock services (camera.v1, 300 tokens with their
    // interface tokens counted once each), a storage, a holder of zs.snmp.v1.
    let cam = examples.join("walkthrough/camera.v1.toml");
    let snmp = examples.join("zensight/zs.snmp.v1.toml");
    let n = 50;
    let svc_args = |first: usize, count: usize, contract: &Path| -> Vec<String> {
        vec![
            "services".into(), "--connect".into(), l.rv.clone(), "--system".into(), "vehicle-01".into(),
            "--count".into(), count.to_string(), "--first".into(), first.to_string(), contract.display().to_string(),
        ]
    };
    let mut push = |measure: String, (s, u, d): (f64, u64, u64), note: String| rows.push(Row { profile: p.name, measure, seconds: s, bytes_up: u, bytes_down: d, note });

    let (u0, _) = l.bytes();
    let t_spawn = Instant::now();
    let _svcs = procs::spawn(&svc_args(0, n, &cam), Duration::from_secs(120)).await?;
    let s = t_spawn.elapsed().as_secs_f64();
    let u = l.bytes().0 - u0;
    // Presence: wait until the ground sees every instance token.
    let seen = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
    let (s2, events) = (seen.clone(), Arc::new(Mutex::new((0u64, 0u64))));
    let ev = events.clone();
    let _live = ground
        .liveliness()
        .declare_subscriber("zk2/vehicle-01/*/@zk/**")
        .history(true)
        .callback(move |smp| {
            let mut g = s2.lock().unwrap();
            let mut e = ev.lock().unwrap();
            match smp.kind() {
                SampleKind::Put => {
                    e.0 += 1;
                    g.insert(smp.key_expr().as_str().to_owned());
                }
                SampleKind::Delete => {
                    e.1 += 1;
                    g.remove(smp.key_expr().as_str());
                }
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let want = n * 2; // an instance token and one interface token each
    let wait_all = |limit: Duration| {
        let seen = seen.clone();
        async move {
            let t0 = Instant::now();
            while seen.lock().unwrap().len() < want && t0.elapsed() < limit {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Ok::<usize, anyhow::Error>(seen.lock().unwrap().len())
        }
    };
    if p.presence {
    let (got, s1, u1, d1) = measured(&l, wait_all(Duration::from_secs(300))).await?;
    push(format!("first view of {n} services' presence ({want} tokens; spawn {s:.1} s, {u} B up during spawn)"), (s1, u1, d1), format!("{got}/{want} seen"));

    // Presence replay after a reconnect.
    l.proxy.cut().await;
    let t0 = Instant::now();
    while !seen.lock().unwrap().is_empty() && t0.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    l.proxy.heal();
    let (got, s1, u1, d1) = measured(&l, wait_all(Duration::from_secs(300))).await?;
    push("presence replay after a reconnect (cut, then heal)".into(), (s1, u1, d1), format!("{got}/{want} seen again; router reconnect included"));
    } else {
        // The services declared their tokens as usual; the ACL keeps them,
        // and every other `@zk` declaration, on the vehicle.
        let (u0, d0) = l.bytes();
        let s1 = settle(&l, Duration::from_secs(3), p.timeout).await;
        let (u1, d1) = l.bytes();
        push(format!("bring-up across the link: {n} services' data declarations (spawn {s:.1} s, {u} B up during spawn)"), (s1, u1 - u0, d1 - d0), "until the link is quiet (≤ 64 B in 3 s)".into());
        let toks = zk2rt::client::tokens(&ground, "zk2/vehicle-01/*/@zk/**", Duration::from_secs(10)).await?;
        push("presence across the link".into(), (f64::NAN, 0, 0), format!("{} tokens visible from the ground (the ACL denies `@zk`), {} seen by the history subscriber", toks.len(), seen.lock().unwrap().len()));
        let r = zk2rt::client::get(&ground, "zk2/vehicle-01/*/@zk/instance/*", zenoh::query::QueryTarget::All, zenoh::query::ConsolidationMode::None, None, Duration::from_secs(10)).await?;
        push("a descriptor GET across the link".into(), (f64::NAN, 0, 0), format!("{} replies: the ground binds from configuration and its own copy of the contract", r.iter().flatten().count()));
    }

    let lc = load_path(&cam);
    let c_cam = lc.contract.ok_or_else(|| anyhow!("{}", lc.report))?;
    if p.presence {
        // A descriptor, by GET (on svc-0's instance key, found by a
        // liveliness GET if presence did not converge).
        let inst = match seen.lock().unwrap().iter().find(|k| k.contains("/@zk/instance/")).cloned() {
            Some(k) => k,
            None => zk2rt::client::tokens(&ground, "zk2/vehicle-01/svc-0/@zk/instance/*", Duration::from_secs(120)).await?.into_iter().next().unwrap_or_default(),
        };
        if inst.is_empty() {
            push("one descriptor GET".into(), (f64::NAN, 0, 0), "skipped: no instance token visible on the ground".into());
        } else {
            let (desc, s1, u1, d1) = measured(&l, async { zk2rt::client::descriptor(&ground, &inst, Duration::from_secs(120)).await }).await?;
            push("one descriptor GET".into(), (s1, u1, d1), format!("{} B of JSON", serde_json::to_vec(&desc)?.len()));
        }

        // Contract bundles by hash: camera.v1 from the vehicle's services,
        // then zs.snmp.v1 from a holder on the vehicle.
        let (f, s1, u1, d1) = measured(&l, async { zk2rt::client::fetch_contract(&ground, &c_cam.iface, &Fingerprint::of(&c_cam), Duration::from_secs(60)).await }).await?;
        push(format!("contract bundle fetch: camera.v1 ({} B)", f.bytes), (s1, u1, d1), format!("attempt {}", f.attempts));
        let _holder = procs::spawn(&["s4-holder".into(), "--connect".into(), l.rv.clone(), "--mode".into(), "ok".into(), snmp.display().to_string()], Duration::from_secs(30)).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let ls = load_path(&snmp);
        let c_snmp = ls.contract.ok_or_else(|| anyhow!("{}", ls.report))?;
        match measured(&l, async { zk2rt::client::fetch_contract(&ground, &c_snmp.iface, &Fingerprint::of(&c_snmp), p.timeout).await }).await {
            Ok((f, s1, u1, d1)) => push(format!("contract bundle fetch: zs.snmp.v1 ({} B)", f.bytes), (s1, u1, d1), format!("attempt {}", f.attempts)),
            Err(e) => push("contract bundle fetch: zs.snmp.v1 (82,391 B)".into(), (f64::NAN, 0, 0), format!("failed within 2 x {} s: {e}", p.timeout.as_secs())),
        }
    } else {
        push("descriptors and contract bundles".into(), (f64::NAN, 0, 0), "not fetched: under R7 the ground holds the bundles it binds".into());
    }

    // State with the storage on the far side (the vehicle).
    let st_listen = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let _st = procs::spawn(&["storage-router".into(), "--listen".into(), st_listen, "--connect".into(), l.rv.clone(), "--storage".into(), "v=zk2/vehicle-01/**".into()], Duration::from_secs(30)).await?;
    let w = Topo::client(&[l.rv.clone()]).open().await?;
    w.put("zk2/vehicle-01/nav/nav.v2/state/status", vec![b'x'; 200]).timestamp(w.new_timestamp()).await.map_err(|e| anyhow!("{e}"))?;
    drop(w);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (got, s1, u1, d1) = measured(&l, async { zk2rt::client::state_get(&ground, "zk2/vehicle-01/nav/nav.v2/state/status", Duration::from_secs(60)).await }).await?;
    push("state GET answered by a storage on the vehicle".into(), (s1, u1, d1), format!("{} replies", got.iter().flatten().count()));
    if !p.presence {
        // A static binding to an owner: svc-0's own state, no presence.
        let (got, s1, u1, d1) = measured(&l, async { zk2rt::client::state_get(&ground, "zk2/vehicle-01/svc-0/camera.v1/state/info", Duration::from_secs(60)).await }).await?;
        push("state GET answered by the owner, bound statically (svc-0 camera.v1/state/info)".into(), (s1, u1, d1), format!("{} replies", got.iter().flatten().count()));
    }

    // `@stream` stays off `zk2/vehicle-01/**`.
    let pubs = Topo::client(&[l.rv.clone()]).open().await?;
    let wide = ground.declare_subscriber("zk2/vehicle-01/**").callback(|_| {}).await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let publish = |secs: u64| {
        let pubs = pubs.clone();
        async move {
            for _ in 0..secs * 5 {
                pubs.put("zk2/vehicle-01/cam-0/camera.v1/@stream/image", vec![0u8; 10_000]).await.map_err(|e| anyhow!("{e}"))?;
                pubs.put("zk2/vehicle-01/cam-0/camera.v1/stream/status", vec![0u8; 100]).await.map_err(|e| anyhow!("{e}"))?;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Ok::<(), anyhow::Error>(())
        }
    };
    let ((), s1, u1, d1) = measured(&l, async {
        publish(4).await?;
        settle(&l, Duration::from_secs(3), p.timeout).await;
        Ok(())
    })
    .await?;
    push("4 s of 10 KB @stream frames + 100 B stream samples at 5 Hz, ground subscribed to zk2/vehicle-01/**, until the link is quiet".into(), (s1, u1, d1), "the @stream frames do not cross".into());
    let narrow = ground.declare_subscriber("zk2/vehicle-01/cam-0/camera.v1/@stream/image").callback(|_| {}).await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let ((), s1, u1, d1) = measured(&l, async {
        publish(4).await?;
        settle(&l, Duration::from_secs(3), p.timeout).await;
        Ok(())
    })
    .await?;
    push("the same, with the ground also naming the @stream key".into(), (s1, u1, d1), "frames cross only when asked for".into());
    drop((wide, narrow));

    // A tool's time to first useful view: presence, every descriptor, one
    // contract, one state.
    let tool = Topo::client(&[l.rg.clone()]).open().await?;
    let mut total = (0.0, 0, 0);
    let mut steps = Vec::new();
    if !p.presence {
        let (st, s1, u1, d1) = measured(&l, async { zk2rt::client::state_get(&tool, "zk2/vehicle-01/nav/nav.v2/state/status", Duration::from_secs(120)).await }).await?;
        push("a tool's first useful view, bound statically: one state".into(), (s1, u1, d1), format!("{} replies; presence, descriptors and contracts are local under R7", st.iter().flatten().count()));
    } else {
    let (toks, s1, u1, d1) = measured(&l, async { zk2rt::client::tokens(&tool, "zk2/vehicle-01/*/@zk/instance/*", p.timeout).await }).await?;
    steps.push(format!("presence {s1:.1} s / {u1} B ({} tokens)", toks.len()));
    total = (total.0 + s1, total.1 + u1, total.2 + d1);
    let (descs, s1, u1, d1) = measured(&l, async { zk2rt::client::get(&tool, "zk2/vehicle-01/*/@zk/instance/*", zenoh::query::QueryTarget::All, zenoh::query::ConsolidationMode::None, None, Duration::from_secs(120)).await }).await?;
    steps.push(format!("descriptors {s1:.1} s / {u1} B ({} replies)", descs.iter().flatten().count()));
    total = (total.0 + s1, total.1 + u1, total.2 + d1);
    let (f, s1, u1, d1) = measured(&l, async { zk2rt::client::fetch_contract(&tool, &c_cam.iface, &Fingerprint::of(&c_cam), Duration::from_secs(120)).await }).await?;
    steps.push(format!("contract {s1:.1} s / {u1} B (attempt {})", f.attempts));
    total = (total.0 + s1, total.1 + u1, total.2 + d1);
    let (st, s1, u1, d1) = measured(&l, async { zk2rt::client::state_get(&tool, "zk2/vehicle-01/nav/nav.v2/state/status", Duration::from_secs(120)).await }).await?;
    steps.push(format!("state {s1:.1} s / {u1} B ({} replies)", st.iter().flatten().count()));
    total = (total.0 + s1, total.1 + u1, total.2 + d1);
    push(format!("a tool's first useful view: presence, {n} descriptors, one contract, one state"), total, steps.join("; "));
    }

    // Declaration cost: 100 subscriber declarations from the ground with
    // zk2 keys and with v1-shaped keys.
    for (label, fmt) in [
        ("zk2", "zk2/h-3fa9c2d41b7e/sysinfo/zs.sysinfo.v1/stream/cpu/{}/usage_pct"),
        ("v1", "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/{}/usage_pct"),
    ] {
        let s = Topo::client(&[l.rg.clone()]).open().await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let (subs, s1, u1, d1) = measured(&l, async {
            let mut v = Vec::new();
            for i in 0..100 {
                v.push(s.declare_subscriber(fmt.replace("{}", &i.to_string())).callback(|_| {}).await.map_err(|e| anyhow!("{e}"))?);
            }
            settle(&l, Duration::from_secs(3), p.timeout).await;
            Ok(v)
        })
        .await?;
        let klen = fmt.replace("{}", "10").len();
        push(format!("100 subscriber declarations, {label}-shaped keys ({klen} B each)"), (s1, u1, d1), format!("{:.0} B per declaration toward the vehicle", d1 as f64 / 100.0));
        drop(subs);
    }

    // A coverage gap: with `@zk` denied, what re-crosses is the data
    // declarations; otherwise every token also flaps.
    if !p.presence {
        let ((), s1, u1, d1) = measured(&l, async {
            for _ in 0..3 {
                l.proxy.cut().await;
                tokio::time::sleep(Duration::from_secs(2)).await;
                l.proxy.heal();
                settle(&l, Duration::from_secs(3), p.timeout).await;
            }
            Ok(())
        })
        .await?;
        push("3 coverage gaps (2 s each), `@zk` denied".into(), (s1, u1, d1), format!("{} B per gap, until the link is quiet again; {} tokens seen by the ground", (u1 + d1) / 3, seen.lock().unwrap().len()));
        return Ok(());
    }
    let (p0, d0) = *events.lock().unwrap();
    let ((), s1, u1, d1) = measured(&l, async {
        for _ in 0..3 {
            l.proxy.cut().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            l.proxy.heal();
            let t0 = Instant::now();
            while seen.lock().unwrap().len() < want && t0.elapsed() < Duration::from_secs(120) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        Ok(())
    })
    .await?;
    let (p1, dd1) = *events.lock().unwrap();
    push("3 coverage gaps (2 s each) with presence crossing the link".into(), (s1, u1, d1), format!("{} deletes and {} puts seen by the ground: every token flaps each gap", dd1 - d0, p1 - p0));
    Ok(())
}

pub async fn run(results: &Path, examples: &Path, only: Option<&str>) -> Result<()> {
    std::fs::create_dir_all(results)?;
    let rf = Shape { delay: Duration::from_millis(100), bytes_per_s: Some(300) };
    let profiles = [
        Profile { name: "600 ms RTT, 1 Mbit/s", tag: "1mbit", batch: None, shape: Shape { delay: Duration::from_millis(300), bytes_per_s: Some(125_000) }, timeout: Duration::from_secs(120), presence: true, ground_client: false },
        Profile { name: "600 ms RTT, 64 kbit/s", tag: "64kbit", batch: None, shape: Shape { delay: Duration::from_millis(300), bytes_per_s: Some(8_000) }, timeout: Duration::from_secs(120), presence: true, ground_client: false },
        Profile { name: "RF class: 200 ms RTT, 2,400 bit/s, zenoh defaults", tag: "rf", batch: None, shape: rf, timeout: Duration::from_secs(600), presence: true, ground_client: false },
        Profile { name: "RF class, 1 KB batches", tag: "rf-batch", batch: Some(1024), shape: rf, timeout: Duration::from_secs(600), presence: true, ground_client: false },
        Profile { name: "RF class, 1 KB batches, @zk denied (R7)", tag: "rf-batch-r7", batch: Some(1024), shape: rf, timeout: Duration::from_secs(600), presence: false, ground_client: false },
        Profile { name: "RF class, 1 KB batches, @zk denied, ground as a client (R7)", tag: "rf-batch-r7-client", batch: Some(1024), shape: rf, timeout: Duration::from_secs(600), presence: false, ground_client: true },
    ];
    let mut rows = Vec::new();
    for p in &profiles {
        if only.is_some_and(|o| !p.name.contains(o)) {
            continue;
        }
        if let Err(e) = profile(p, examples, results, &mut rows).await {
            rows.push(Row { profile: p.name, measure: "profile aborted".into(), seconds: f64::NAN, bytes_up: 0, bytes_down: 0, note: e.to_string() });
        }
        println!("profile {} done", p.name);
    }
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s3.csv");
    let mut md = format!("# S3 — constrained links (#599)\n\nWritten by `spike s3`; the latest run (`unix_s` {unix}), zenoh {}. A shaping proxy between the vehicle and ground routers (delay line + bandwidth cap; no loss emulation). Bytes are TCP payload over the link.\n\n| Profile | Measure | s | B vehicle→ground | B ground→vehicle | Note |\n|---|---|---|---|---|---|\n", zk2rt::ZENOH_VERSION);
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "profile", "measure", "seconds", "bytes_up", "bytes_down", "note"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.profile.into(), r.measure.clone(), format!("{:.2}", r.seconds), r.bytes_up.to_string(), r.bytes_down.to_string(), r.note.clone()])?;
        md.push_str(&format!("| {} | {} | {:.2} | {} | {} | {} |\n", r.profile, r.measure, r.seconds, r.bytes_up, r.bytes_down, r.note));
        println!("[{}] {} → {:.2} s, up {} B, down {} B {}", r.profile, r.measure, r.seconds, r.bytes_up, r.bytes_down, r.note);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(())
}

/// Two routers joined by TCP, each serving a local client by unix socket,
/// with `acl` on both: what crosses.
#[allow(clippy::too_many_arguments)]
pub async fn acl_probe(acl: Option<&Path>, dir: &Path, bps: Option<u64>, live_sub: bool, client_link: bool, south: bool, side: &str, zk: usize, data: usize) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let dir = std::fs::canonicalize(dir)?;
    let tcp = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let sock = |name: &str| -> String {
        let p = dir.join(format!("{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&p);
        format!("unixsock-stream/{}", p.display())
    };
    let (rg, rv) = (sock("rg"), sock("rv"));
    let shape = Shape { delay: Duration::from_millis(if bps.is_some() { 100 } else { 0 }), bytes_per_s: bps };
    let (_r1, proxy, _r2, rg) = if client_link {
        let r = procs::router_with(&[tcp.clone(), rv.clone()], &[], acl.filter(|_| side != "rg")).await?;
        let proxy = Proxy::shaped(tcp.strip_prefix("tcp/").unwrap_or(&tcp).to_owned(), shape).await?;
        let ep = proxy.endpoint();
        (r, proxy, None, ep)
    } else {
        // U23: the ground router names its region; the vehicle router keeps
        // `auto`'s rule (clients and peers south) and adds the ground router
        // as a second south region.
        let with = |base: Option<&Path>, extra: serde_json::Value, name: &str| -> Result<Option<PathBuf>> {
            if !south {
                return Ok(base.map(Path::to_path_buf));
            }
            let mut v: serde_json::Value = match base {
                Some(p) => serde_json::from_str(&std::fs::read_to_string(p)?)?,
                None => serde_json::json!({}),
            };
            for (k, x) in extra.as_object().expect("object") {
                v[k] = x.clone();
            }
            let f = dir.join(format!("{name}-{}.json", std::process::id()));
            std::fs::write(&f, serde_json::to_string_pretty(&v)?)?;
            Ok(Some(f))
        };
        let rg_cfg = with(acl.filter(|_| side != "rv"), serde_json::json!({"region_name": "ground"}), "rg")?;
        let rv_cfg = with(
            acl.filter(|_| side != "rg"),
            serde_json::json!({"gateway": {"south": [
                {"filters": [{"modes": ["peer", "client"]}]},
                {"filters": [{"region_names": ["ground"]}]}
            ]}}),
            "rv",
        )?;
        let r1 = procs::router_with(&[tcp.clone(), rg.clone()], &[], rg_cfg.as_deref()).await?;
        let proxy = Proxy::shaped(tcp.strip_prefix("tcp/").unwrap_or(&tcp).to_owned(), shape).await?;
        let r2 = procs::router_with(std::slice::from_ref(&rv), &[proxy.endpoint()], rv_cfg.as_deref()).await?;
        (r1, proxy, Some(r2), rg)
    };
    let owner = Topo::client(std::slice::from_ref(&rv)).open().await?;
    let _q = owner
        .declare_queryable("zk2/v/s/x.v1/state/a")
        .callback(|q| {
            let k = q.key_expr().clone();
            tokio::spawn(async move {
                let _ = q.reply(k, "a").await;
            });
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let _t1 = owner.liveliness().declare_token("zk2/v/s/@zk/instance/1").await.map_err(|e| anyhow!("{e}"))?;
    let _t2 = owner.liveliness().declare_token("zk2/v/s/x.v1/alive").await.map_err(|e| anyhow!("{e}"))?;
    let mut extra_t = Vec::new();
    for i in 0..zk {
        extra_t.push(owner.liveliness().declare_token(format!("zk2/v/svc-{i}/@zk/instance/{i:016x}")).await.map_err(|e| anyhow!("{e}"))?);
    }
    let mut extra_q = Vec::new();
    for i in 0..data {
        extra_q.push(owner.declare_queryable(format!("zk2/v/svc-{i}/camera.v1/state/info")).callback(|_| {}).await.map_err(|e| anyhow!("{e}"))?);
    }
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let g = got.clone();
    let reader = Topo::client(std::slice::from_ref(&rg)).open().await?;
    let _ls = if live_sub {
        Some(reader.liveliness().declare_subscriber("zk2/v/*/@zk/**").history(true).callback(|_| {}).await.map_err(|e| anyhow!("{e}"))?)
    } else {
        None
    };
    let _s = reader.declare_subscriber("zk2/v/**").callback(move |s| g.lock().unwrap().push(s.key_expr().to_string())).await.map_err(|e| anyhow!("{e}"))?;
    // Quiet: at most 64 B either way over 3 s (keep-alives pass).
    let t0 = Instant::now();
    let mut window = std::collections::VecDeque::new();
    loop {
        let (u, d) = proxy.bytes();
        window.push_back((Instant::now(), u + d));
        while window.front().is_some_and(|(t, _)| t.elapsed() > Duration::from_secs(3)) {
            window.pop_front();
        }
        let full = window.front().is_some_and(|(t, _)| t.elapsed() >= Duration::from_millis(2900));
        let moved = window.back().map_or(0, |b| b.1) - window.front().map_or(0, |f| f.1);
        if (full && moved <= 64) || t0.elapsed() > Duration::from_secs(400) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!("settled after {:.1} s", t0.elapsed().as_secs_f64());
    owner.put("zk2/v/s/x.v1/stream/b", "b").await.map_err(|e| anyhow!("{e}"))?;
    owner.put("zk2/v/s/@zk/descriptor", "d").await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let t = Duration::from_secs(2);
    let r = zk2rt::client::get(&reader, "zk2/v/s/x.v1/state/a", zenoh::query::QueryTarget::All, zenoh::query::ConsolidationMode::None, None, t).await?;
    println!("state GET: {} replies", r.iter().flatten().count());
    println!("tokens zk2/v/**: {:?}", zk2rt::client::tokens(&reader, "zk2/v/**", t).await?);
    println!("tokens zk2/v/s/@zk/**: {:?}", zk2rt::client::tokens(&reader, "zk2/v/s/@zk/**", t).await?);
    println!("subscriber zk2/v/** saw: {:?}", got.lock().unwrap());
    // The proxy's client side is the vehicle router, or the reader when it
    // is a client across the link.
    let (to_up, to_client) = proxy.bytes();
    let (v2g, g2v) = if client_link { (to_client, to_up) } else { (to_up, to_client) };
    println!("bytes: {v2g} vehicle→ground, {g2v} ground→vehicle");
    if bps.is_some() {
        let tap = if client_link { proxy.tap_down.lock().unwrap() } else { proxy.tap.lock().unwrap() };
        let count = |needle: &[u8]| tap.windows(needle.len()).filter(|w| *w == needle).count();
        println!("in the vehicle→ground bytes: {} × \"@zk/instance\", {} × \"camera.v1/state/info\", {} × \"svc-1\"", count(b"@zk/instance"), count(b"camera.v1/state/info"), count(b"svc-1"));
    }
    Ok(())
}

/// N tokens from one owner on the vehicle router, seen by a history
/// subscriber on the ground, over a link capped at `bps`: the first view,
/// then the replay after a cut. `extra` (router config overrides) applies to
/// both routers.
pub async fn rf_probe(n: usize, bps: u64, extra: Option<&Path>) -> Result<()> {
    let tcp = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let _rg = procs::router_with(std::slice::from_ref(&tcp), &[], extra).await?;
    let proxy = Proxy::shaped(tcp.strip_prefix("tcp/").unwrap_or(&tcp).to_owned(), Shape { delay: Duration::from_millis(100), bytes_per_s: Some(bps) }).await?;
    let rv = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let _rv = procs::router_with(std::slice::from_ref(&rv), &[proxy.endpoint()], extra).await?;
    let seen = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
    let s2 = seen.clone();
    let ground = Topo::client(std::slice::from_ref(&tcp)).open().await?;
    let _sub = ground
        .liveliness()
        .declare_subscriber("zk2/v/*/@zk/**")
        .history(true)
        .callback(move |smp| {
            let mut g = s2.lock().unwrap();
            match smp.kind() {
                SampleKind::Put => g.insert(smp.key_expr().as_str().to_owned()),
                SampleKind::Delete => g.remove(smp.key_expr().as_str()),
            };
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let owner = Topo::client(std::slice::from_ref(&rv)).open().await?;
    let mut toks = Vec::new();
    for i in 0..n / 2 {
        toks.push(owner.liveliness().declare_token(format!("zk2/v/svc-{i}/@zk/instance/{i:016x}")).await.map_err(|e| anyhow!("{e}"))?);
        toks.push(owner.liveliness().declare_token(format!("zk2/v/svc-{i}/@zk/alive/camera.v1/{i:016x}/0123456789abcdef")).await.map_err(|e| anyhow!("{e}"))?);
    }
    let wait = |label: &'static str| {
        let (seen, proxy) = (seen.clone(), &proxy);
        async move {
            let (b0, c0, t0) = (proxy.bytes(), proxy.accepted(), Instant::now());
            while seen.lock().unwrap().len() < n && t0.elapsed() < Duration::from_secs(300) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let b1 = proxy.bytes();
            println!("{label}: {}/{n} in {:.1} s; {} B vehicle→ground, {} B ground→vehicle; {} connections accepted", seen.lock().unwrap().len(), t0.elapsed().as_secs_f64(), b1.0 - b0.0, b1.1 - b0.1, proxy.accepted() - c0);
        }
    };
    wait("first view").await;
    proxy.cut().await;
    let t0 = Instant::now();
    while !seen.lock().unwrap().is_empty() && t0.elapsed() < Duration::from_secs(30) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    proxy.heal();
    wait("replay after a cut").await;
    Ok(())
}
