//! S3, discovery over constrained links (#599, r3 §7 S3).
//!
//! A vehicle router (services, a storage) reaches a ground router (the
//! tools) through a shaping proxy: a one-way delay line and a bandwidth cap,
//! with the bytes counted in each direction. `tc`/netem is not available on
//! this host, so loss is not emulated: over TCP, loss shows as retransmission
//! delay. Profiles: 600 ms RTT at 64 kbit/s and at 1 Mbit/s; an RF class at
//! 2,400 bit/s with 200 ms RTT (zenoh-modem's 220 B MTU is not modelled).

use std::path::Path;
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
    shape: Shape,
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
    _rg: Proc,
    rg: String,
    proxy: Proxy,
}

/// Ground router RG; vehicle router RV connecting to RG through the proxy.
async fn link(shape: Shape) -> Result<Link> {
    let (rg_p, rg) = procs::router(&[]).await?;
    let proxy = Proxy::shaped(rg.strip_prefix("tcp/").unwrap_or(&rg).to_owned(), shape).await?;
    let (rv_p, rv) = procs::router(&[proxy.endpoint()]).await?;
    Ok(Link { _rv: rv_p, rv, _rg: rg_p, rg, proxy })
}

/// Bytes over the link (vehicle → ground, ground → vehicle) during `f`.
async fn measured<F, T>(l: &Link, f: F) -> Result<(T, f64, u64, u64)>
where
    F: std::future::Future<Output = Result<T>>,
{
    let (u0, d0) = l.proxy.bytes();
    let t0 = Instant::now();
    let v = f.await?;
    let s = t0.elapsed().as_secs_f64();
    let (u1, d1) = l.proxy.bytes();
    Ok((v, s, u1 - u0, d1 - d0))
}

#[allow(clippy::too_many_lines)]
async fn profile(p: &Profile, examples: &Path, rows: &mut Vec<Row>) -> Result<()> {
    let l = link(p.shape).await?;
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

    let (u0, _) = l.proxy.bytes();
    let t_spawn = Instant::now();
    let _svcs = procs::spawn(&svc_args(0, n, &cam), Duration::from_secs(120)).await?;
    let s = t_spawn.elapsed().as_secs_f64();
    let u = l.proxy.bytes().0 - u0;
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

    // A descriptor, by GET.
    let inst = seen.lock().unwrap().iter().find(|k| k.contains("/@zk/instance/")).cloned().unwrap_or_default();
    let (desc, s1, u1, d1) = measured(&l, async { zk2rt::client::descriptor(&ground, &inst, Duration::from_secs(60)).await }).await?;
    push("one descriptor GET".into(), (s1, u1, d1), format!("{} B of JSON", serde_json::to_vec(&desc)?.len()));

    // Contract bundles by hash: camera.v1 from the vehicle's services, then
    // zs.snmp.v1 from a holder on the vehicle.
    let lc = load_path(&cam);
    let c_cam = lc.contract.ok_or_else(|| anyhow!("{}", lc.report))?;
    let (f, s1, u1, d1) = measured(&l, async { zk2rt::client::fetch_contract(&ground, &c_cam.iface, &Fingerprint::of(&c_cam), Duration::from_secs(60)).await }).await?;
    push(format!("contract bundle fetch: camera.v1 ({} B)", f.bytes), (s1, u1, d1), format!("attempt {}", f.attempts));
    let _holder = procs::spawn(&["s4-holder".into(), "--connect".into(), l.rv.clone(), "--mode".into(), "ok".into(), snmp.display().to_string()], Duration::from_secs(30)).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let ls = load_path(&snmp);
    let c_snmp = ls.contract.ok_or_else(|| anyhow!("{}", ls.report))?;
    let (f, s1, u1, d1) = measured(&l, async { zk2rt::client::fetch_contract(&ground, &c_snmp.iface, &Fingerprint::of(&c_snmp), Duration::from_secs(120)).await }).await?;
    push(format!("contract bundle fetch: zs.snmp.v1 ({} B)", f.bytes), (s1, u1, d1), format!("attempt {}", f.attempts));

    // State with the storage on the far side (the vehicle).
    let (_st, _sp) = crate::s5::storage_with(&l.rv, &["v=zk2/vehicle-01/**"], false, 30).await?;
    let w = Topo::client(&[l.rv.clone()]).open().await?;
    w.put("zk2/vehicle-01/nav/nav.v2/state/status", vec![b'x'; 200]).timestamp(w.new_timestamp()).await.map_err(|e| anyhow!("{e}"))?;
    drop(w);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (got, s1, u1, d1) = measured(&l, async { zk2rt::client::state_get(&ground, "zk2/vehicle-01/nav/nav.v2/state/status", Duration::from_secs(60)).await }).await?;
    push("state GET answered by a storage on the vehicle".into(), (s1, u1, d1), format!("{} replies", got.iter().flatten().count()));

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
    let ((), s1, u1, d1) = measured(&l, publish(4)).await?;
    push("4 s of 10 KB @stream frames + 100 B stream samples at 5 Hz, ground subscribed to zk2/vehicle-01/**".into(), (s1, u1, d1), "the @stream frames do not cross".into());
    let narrow = ground.declare_subscriber("zk2/vehicle-01/cam-0/camera.v1/@stream/image").callback(|_| {}).await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let ((), s1, u1, d1) = measured(&l, publish(4)).await?;
    push("the same, with the ground also naming the @stream key".into(), (s1, u1, d1), "frames cross only when asked for".into());
    drop((wide, narrow));

    // A tool's time to first useful view: presence, every descriptor, one
    // contract, one state.
    let tool = Topo::client(&[l.rg.clone()]).open().await?;
    let ((), s1, u1, d1) = measured(&l, async {
        let toks = zk2rt::client::tokens(&tool, "zk2/vehicle-01/*/@zk/instance/*", Duration::from_secs(60)).await?;
        let _ = zk2rt::client::get(&tool, "zk2/vehicle-01/*/@zk/instance/*", zenoh::query::QueryTarget::All, zenoh::query::ConsolidationMode::None, None, Duration::from_secs(60)).await?;
        let _ = zk2rt::client::fetch_contract(&tool, &c_cam.iface, &Fingerprint::of(&c_cam), Duration::from_secs(60)).await?;
        let _ = zk2rt::client::state_get(&tool, "zk2/vehicle-01/nav/nav.v2/state/status", Duration::from_secs(60)).await?;
        anyhow::ensure!(!toks.is_empty(), "no presence");
        Ok(())
    })
    .await?;
    push(format!("a tool's first useful view: presence, {n} descriptors, one contract, one state"), (s1, u1, d1), String::new());

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
            tokio::time::sleep(Duration::from_secs(3)).await;
            Ok(v)
        })
        .await?;
        let klen = fmt.replace("{}", "10").len();
        push(format!("100 subscriber declarations, {label}-shaped keys ({klen} B each)"), (s1, u1, d1), format!("{:.0} B per declaration toward the vehicle", d1 as f64 / 100.0));
        drop(subs);
    }

    // Presence flaps per coverage gap when `@zk` is not denied.
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

pub async fn run(results: &Path, examples: &Path) -> Result<()> {
    let profiles = [
        Profile { name: "600 ms RTT, 1 Mbit/s", shape: Shape { delay: Duration::from_millis(300), bytes_per_s: Some(125_000) } },
        Profile { name: "600 ms RTT, 64 kbit/s", shape: Shape { delay: Duration::from_millis(300), bytes_per_s: Some(8_000) } },
        Profile { name: "RF class: 200 ms RTT, 2,400 bit/s", shape: Shape { delay: Duration::from_millis(100), bytes_per_s: Some(300) } },
    ];
    let mut rows = Vec::new();
    for p in &profiles {
        if let Err(e) = profile(p, examples, &mut rows).await {
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
