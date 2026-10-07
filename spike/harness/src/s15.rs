//! S15, constrained devices (#617): a zenoh-pico 1.10.1 participant as a zk2
//! owner (`s15/zk2_pico.c`), driven by the spike's Rust tools against a
//! router. What works, and what a constrained conformance level must relax.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenkey_model::grammar::{ZkKey, parse};
use zenoh::bytes::Encoding;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zk2rt::config::{Mode, Topo};
use zk2rt::metrics::csv_row;

use crate::procs;

struct Row {
    case: String,
    value: String,
}

async fn call_op(c: &zenoh::Session, pre: &str, op: &str, t: Duration) -> Result<Vec<Result<zk2rt::client::Got, String>>> {
    zk2rt::client::call(c, &format!("{pre}/health.v1/@op/{op}"), (Vec::new(), Encoding::default()), t).await
}

async fn pico(bin: &Path, ep: &str, prefix: &str, svc: &str) -> Result<tokio::process::Child> {
    let mut c = tokio::process::Command::new(bin)
        .args([ep, prefix, svc])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let out = c.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let mut lines = tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(out));
    let l = tokio::time::timeout(Duration::from_secs(20), lines.next_line()).await.map_err(|_| anyhow!("pico not ready"))??.ok_or_else(|| anyhow!("pico exited"))?;
    anyhow::ensure!(l.starts_with("ready"), "pico said {l:?}");
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Ok(c)
}

#[allow(clippy::too_many_lines)]
pub async fn run(results: &Path, bin: &Path) -> Result<()> {
    std::fs::create_dir_all(results)?;
    let mut rows = Vec::new();
    let mut push = |case: &str, value: String| {
        println!("{case} → {value}");
        rows.push(Row { case: case.into(), value });
    };
    let (_r, ep) = procs::router(&[]).await?;
    let pre = "zk2/vehicle-01/pico";
    let t0 = Instant::now();
    let _p = pico(bin, &ep, "zk2", "pico").await?;
    push("the pico participant opens a client session and brings itself up", format!("ready in {:.0} ms", t0.elapsed().as_secs_f64() * 1e3));
    let c = Topo::client(&[ep.clone()]).open().await?;
    let t = Duration::from_secs(2);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Presence, parsed by zenkey-model.
    let toks = zk2rt::client::tokens(&c, &format!("{pre}/@zk/**"), t).await?;
    let parsed: Vec<String> = toks.iter().map(|k| match parse(k) {
        Ok(ZkKey::Instance { .. }) => "instance".into(),
        Ok(ZkKey::Alive { iface, .. }) => format!("alive {iface}"),
        Ok(_) => "other".into(),
        Err(e) => format!("REFUSED {e}"),
    }).collect();
    push("its tokens, parsed by zenkey-model", parsed.join(", "));

    // The descriptor.
    let inst = toks.iter().find(|k| k.contains("/@zk/instance/")).cloned().unwrap_or_default();
    let d = zk2rt::client::descriptor(&c, &inst, t).await.map(|v| v.to_string()).unwrap_or_else(|e| format!("error {e}"));
    push("its descriptor, by GET on the instance key", d);

    // The stream: samples and their timestamps.
    let got: Arc<Mutex<Vec<Option<f64>>>> = Arc::default();
    let g = got.clone();
    let _s = c
        .declare_subscriber(format!("{pre}/health.v1/stream/heartbeat"))
        .callback(move |smp| {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            g.lock().unwrap().push(smp.timestamp().map(|ts| (ts.get_time().to_duration().as_secs_f64() - now) * 1e3));
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    {
        let g = got.lock().unwrap();
        let stamped = g.iter().filter(|x| x.is_some()).count();
        push("its heartbeat stream: samples in 2 s, how many carry a timestamp", format!("{} samples, {stamped} stamped (the router stamps puts; pico stamps nothing by itself)", g.len()));
    }

    // State: a GET (All + Latest), its timestamp, a delete, reply_del.
    let st = format!("{pre}/health.v1/state/status");
    let r = zk2rt::client::state_get(&c, &st, t).await?;
    let desc = r.iter().flatten().map(|g| format!("value {}; timestamp {}", g.value.as_ref().map_or("del".into(), |v| String::from_utf8_lossy(v).into_owned()), g.timestamp.map_or("none".into(), |ts| format!("id {} ", ts.get_id())))).collect::<Vec<_>>().join(" | ");
    push("a state GET (All + Latest)", desc);
    let rr = call_op(&c, pre, "reset", t).await?;
    push("a call to the complete @op/reset", format!("{} replies", rr.iter().flatten().count()));
    call_op(&c, pre, "toggle", t).await?;
    let r = zk2rt::client::state_get(&c, &st, t).await?;
    push("after @op/toggle deleted the state: the GET", r.iter().flatten().map(|g| if g.value.is_none() { "a delete (reply_del) with a timestamp".to_owned() } else { "a value".to_owned() }).collect::<Vec<_>>().join(", "));
    call_op(&c, pre, "toggle", t).await?;

    // Reply sizes: zenoh-pico's 4 KB fragment default.
    for n in [1024usize, 4096, 8192, 65_536, 100_000] {
        let t1 = Instant::now();
        let r = zk2rt::client::get(&c, &format!("{pre}/health.v1/@op/big/{n}"), QueryTarget::BestMatching, ConsolidationMode::None, None, Duration::from_secs(5)).await?;
        let bytes: usize = r.iter().flatten().filter_map(|g| g.value.as_ref().map(Vec::len)).sum();
        push(&format!("a {n} B reply from the pico"), format!("{bytes} B received in {:.0} ms ({} errors)", t1.elapsed().as_secs_f64() * 1e3, r.iter().filter(|x| x.is_err()).count()));
    }

    // A storage beside the pico producer: the merged GET.
    {
        let (_st, _sp) = crate::s5::storage_with(&ep, &["p=zk2/vehicle-01/pico/**"], false, 30).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        call_op(&c, pre, "toggle", t).await?; // delete
        tokio::time::sleep(Duration::from_millis(300)).await;
        call_op(&c, pre, "toggle", t).await?; // put again
        tokio::time::sleep(Duration::from_millis(300)).await;
        let r = zk2rt::client::get(&c, &st, QueryTarget::All, ConsolidationMode::None, None, t).await?;
        let replies = r.iter().flatten().map(|g| format!("{} ts {}", g.value.as_ref().map_or("del".into(), |v| String::from_utf8_lossy(v).into_owned()), g.timestamp.map_or("none".into(), |ts| ts.get_id().to_string()))).collect::<Vec<_>>().join(" | ");
        push("pico producer + a storage, unconsolidated replies to a state GET", replies);
    }

    // The namespace variant: the deployment prefix is literal in pico keys.
    {
        let _p2 = pico(bin, &ep, "dep1/zk2", "pico2").await?;
        let ns = Topo { mode: Mode::Client, listen: Vec::new(), connect: vec![ep.clone()], namespace: Some("dep1".into()), shm: None }.open().await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let r = zk2rt::client::state_get(&ns, "zk2/vehicle-01/pico2/health.v1/state/status", t).await?;
        push("a pico with the literal prefix dep1/zk2, read by a Rust session in namespace dep1", format!("{} replies, key as seen: {:?}", r.iter().flatten().count(), r.iter().flatten().map(|g| g.key.clone()).collect::<Vec<_>>()));
    }

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let mut md = format!("# S15 — constrained devices (#617)\n\nWritten by `spike s15`; the latest run (`unix_s` {unix}). A zenoh-pico 1.10.1 participant (`s15/zk2_pico.c`, client mode) against a zenoh {} router.\n\n| Case | Observed |\n|---|---|\n", zk2rt::ZENOH_VERSION);
    for r in &rows {
        csv_row(&results.join("s15.csv"), &["unix_s", "case", "observed"], &[unix.clone(), r.case.clone(), r.value.clone()])?;
        md.push_str(&format!("| {} | {} |\n", r.case, r.value.replace('|', "\\|")));
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(())
}
