//! S13, simulation and replay by rebinding (#596, r3 §7, walkthrough §4.1).
//!
//! - **Rebinding.** One detector binary, three binding files: `input` bound
//!   to `vehicle-01/cam-front`, then `vehicle-01/replay-cam`, then
//!   `sim-1/cam-front`. Each run must receive from its bound source only.
//! - **Replay.** v1's `zenctl record` captures the camera's explicit
//!   stream; `zenctl replay` publishes it into a `replay` deployment prefix,
//!   and the same detector, in session namespace `replay`, bound to the
//!   *original* address, must receive it.
//! - **Simulated time.** A `clock.v1` provider (this spike's sketch: sim
//!   time in ns on `zk2/sim-1/clock/clock.v1/stream/now` at 100 Hz) at
//!   speeds 1, 0 (paused) and 2. A consumer evaluates `twist_cmd.v1`'s
//!   `timing.v1` deadline on the bound clock: a commander falls silent,
//!   and the miss must come after 100 ms of *simulated* time.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::json;
use zk2rt::config::{Mode, Topo};
use zk2rt::metrics::csv_row;

use crate::procs;

const CLOCK_KEY: &str = "zk2/sim-1/clock/clock.v1/stream/now";

#[derive(Deserialize)]
struct BindingFile {
    bind: BTreeMap<String, Bind>,
}

#[derive(Deserialize)]
struct Bind {
    interface: String,
    providers: Vec<String>,
}

/// The detector child: reads a binding file, binds `input`, counts what
/// arrives for `secs`, prints `stats {…}`. Its code never names a source.
pub async fn detector(connect: Vec<String>, namespace: Option<String>, bindings: PathBuf, secs: f64) -> Result<()> {
    let f: BindingFile = toml::from_str(&std::fs::read_to_string(&bindings)?)?;
    let input = f.bind.get("input").ok_or_else(|| anyhow!("no [bind.input]"))?;
    if input.interface != "camera.v1" {
        bail!("input is typed camera.v1, bound to {}", input.interface);
    }
    let s = Topo { mode: Mode::Client, listen: Vec::new(), connect, namespace, shm: None }.open().await?;
    let got: Arc<Mutex<BTreeMap<String, u64>>> = Arc::default();
    let mut subs = Vec::new();
    for p in &input.providers {
        // `@stream` is verbatim: the binding names the key, never `**`.
        let g = got.clone();
        subs.push(
            s.declare_subscriber(format!("zk2/{p}/camera.v1/@stream/image"))
                .callback(move |smp| {
                    *g.lock().unwrap().entry(smp.key_expr().as_str().to_owned()).or_default() += 1;
                })
                .await
                .map_err(|e| anyhow!("{e}"))?,
        );
    }
    println!("ready");
    let mut info = 0;
    for p in &input.providers {
        info += zk2rt::client::state_get(&s, &format!("zk2/{p}/camera.v1/state/info"), Duration::from_secs(1))
            .await?
            .into_iter()
            .flatten()
            .count();
    }
    tokio::time::sleep(Duration::from_secs_f64(secs)).await;
    let g = got.lock().unwrap().clone();
    println!("stats {}", json!({"received": g, "info_replies": info}));
    drop(subs);
    Ok(())
}

/// The clock.v1 sketch: simulated time at `speed` × wall, published at 100 Hz.
pub async fn clock(connect: Vec<String>, speed: f64) -> Result<()> {
    let s = Topo::client(&connect).open().await?;
    let p = s.declare_publisher(CLOCK_KEY).await.map_err(|e| anyhow!("{e}"))?;
    println!("ready");
    let start = Instant::now();
    let epoch: u64 = 1_000_000_000_000; // simulated time starts at 1,000 s
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    loop {
        tick.tick().await;
        let sim = epoch + (start.elapsed().as_nanos() as f64 * speed) as u64;
        p.put(sim.to_le_bytes().to_vec()).await.map_err(|e| anyhow!("{e}"))?;
    }
}

struct Row {
    case: String,
    value: String,
    pass: bool,
}

async fn detect(ep: &str, ns: Option<&str>, file: &Path, secs: f64) -> Result<serde_json::Value> {
    let mut args = vec!["s13-detector".to_owned(), "--connect".into(), ep.to_owned(), "--bindings".into(), file.display().to_string()];
    args.extend(["--secs".into(), secs.to_string()]);
    if let Some(n) = ns {
        args.extend(["--namespace".into(), n.to_owned()]);
    }
    let mut p = procs::spawn(&args, Duration::from_secs(20)).await?;
    let line = p.line(Duration::from_secs_f64(secs + 10.0)).await.ok_or_else(|| anyhow!("detector: no stats"))?;
    Ok(serde_json::from_str(line.strip_prefix("stats ").ok_or_else(|| anyhow!("said {line:?}"))?)?)
}

fn binding_file(dir: &Path, name: &str, provider: &str) -> Result<PathBuf> {
    let p = dir.join(format!("{name}.bindings.toml"));
    std::fs::write(&p, format!("# The detector's bindings: configuration, never code (R1).\n[bind.input]\ninterface = \"camera.v1\"\nproviders = [\"{provider}\"]\n"))?;
    Ok(p)
}

#[allow(clippy::too_many_lines)]
pub async fn run(results: &Path, examples: &Path, zc: &Path) -> Result<bool> {
    std::fs::create_dir_all(results)?;
    let cam = examples.join("walkthrough/camera.v1.toml");
    let twist = examples.join("walkthrough/twist_cmd.v1.toml");
    let (_router, ep) = procs::router(&[]).await?;
    let mut rows = Vec::new();
    let src = |system: &str, name: &str| {
        vec![
            "services".to_owned(), "--connect".into(), ep.clone(), "--system".into(), system.to_owned(),
            "--service-name".into(), name.to_owned(), "--count".into(), "1".into(), "--stream-hz".into(), "10".into(),
            cam.display().to_string(),
        ]
    };
    let real = procs::spawn(&src("vehicle-01", "cam-front"), Duration::from_secs(20)).await?;
    let _replay_cam = procs::spawn(&src("vehicle-01", "replay-cam"), Duration::from_secs(20)).await?;
    let _sim = procs::spawn(&src("sim-1", "cam-front"), Duration::from_secs(20)).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Rebinding: three binding files, one binary.
    for (name, provider) in [("real", "vehicle-01/cam-front"), ("replay-cam", "vehicle-01/replay-cam"), ("sim", "sim-1/cam-front")] {
        let f = binding_file(results, name, provider)?;
        let st = detect(&ep, None, &f, 2.0).await?;
        let rx = st["received"].as_object().cloned().unwrap_or_default();
        let want = format!("zk2/{provider}/camera.v1/@stream/image");
        let only_bound = rx.keys().all(|k| *k == want);
        let n = rx.get(&want).and_then(serde_json::Value::as_u64).unwrap_or(0);
        let info = st["info_replies"].as_u64().unwrap_or(0);
        rows.push(Row {
            case: format!("rebind input -> {provider} ({name}.bindings.toml): frames from it / other sources / info state"),
            value: format!("{n} / {} / {info}", rx.len() - usize::from(rx.contains_key(&want))),
            pass: n > 10 && only_bound && info == 1,
        });
    }

    // Replay: zenctl record, then replay into the `replay` prefix.
    let version = std::process::Command::new(zc).arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).unwrap_or_default();
    rows.push(Row { case: "v1 zenctl used for record and replay".into(), value: version.clone(), pass: !version.is_empty() });
    let zrec = results.join("cam-front.zrec");
    let _ = std::fs::remove_file(&zrec);
    let rec = tokio::process::Command::new(zc)
        .args(["record", "zk2/vehicle-01/cam-front/camera.v1/@stream/image", "-o"])
        .arg(&zrec)
        .args(["--for", "3", "-c", &ep])
        .output()
        .await?;
    let rec_ok = rec.status.success() && zrec.exists();
    rows.push(Row {
        case: "zenctl record zk2/vehicle-01/cam-front/camera.v1/@stream/image --for 3".into(),
        value: if rec_ok {
            format!("ok, {} bytes", std::fs::metadata(&zrec)?.len())
        } else {
            format!("exit {:?}: {}", rec.status.code(), String::from_utf8_lossy(&rec.stderr).lines().last().unwrap_or(""))
        },
        pass: rec_ok,
    });
    drop(real);
    tokio::time::sleep(Duration::from_millis(500)).await;
    if rec_ok {
        let f = binding_file(results, "replayed", "vehicle-01/cam-front")?;
        let mut args = vec!["s13-detector".to_owned(), "--connect".into(), ep.clone(), "--bindings".into(), f.display().to_string()];
        args.extend(["--secs".into(), "6".into(), "--namespace".into(), "replay".into()]);
        let mut det = procs::spawn(&args, Duration::from_secs(20)).await?;
        let rep = tokio::process::Command::new(zc)
            .arg("replay")
            .arg(&zrec)
            .args(["--base", "replay", "--force-base", "-c", &ep])
            .output()
            .await?;
        let line = det.line(Duration::from_secs(20)).await.ok_or_else(|| anyhow!("detector: no stats"))?;
        let st: serde_json::Value = serde_json::from_str(line.strip_prefix("stats ").unwrap_or("{}"))?;
        let rx = st["received"].as_object().cloned().unwrap_or_default();
        let n: u64 = rx.values().filter_map(serde_json::Value::as_u64).sum();
        // v1 zenctl re-prefixes v1 keys only (`--base`), and refuses a
        // namespaced --zenoh-config by design (RFC 09 §5), so this row
        // records the block: zero frames reach the `replay` namespace.
        rows.push(Row {
            case: "v1 zenctl replay --base replay: frames reaching a detector in namespace `replay` (expected 0: --base re-prefixes v1 keys only)".into(),
            value: format!(
                "replay exit {:?}; frames received {n}; keys {:?}{}",
                rep.status.code(),
                rx.keys().collect::<Vec<_>>(),
                if rep.status.success() { String::new() } else { format!("; {}", String::from_utf8_lossy(&rep.stderr).lines().last().unwrap_or("")) }
            ),
            pass: rep.status.success() && n == 0,
        });
    }

    // The concept, with a namespace-aware replayer: capture the bound key,
    // then republish it through a session in namespace `replay`, keys
    // unchanged; the namespace prefixes them on egress.
    {
        let live = procs::spawn(&src("vehicle-01", "cam-front"), Duration::from_secs(20)).await?;
        let cap = Topo::client(&[ep.clone()]).open().await?;
        type Row3 = (Duration, Vec<u8>, zenoh::bytes::Encoding, Option<Vec<u8>>);
        let rec: Arc<Mutex<Vec<Row3>>> = Arc::default();
        let (r2, t0) = (rec.clone(), Instant::now());
        let sub = cap
            .declare_subscriber("zk2/vehicle-01/cam-front/camera.v1/@stream/image")
            .callback(move |smp| {
                r2.lock().unwrap().push((
                    t0.elapsed(),
                    smp.payload().to_bytes().into_owned(),
                    smp.encoding().clone(),
                    smp.attachment().map(|a| a.to_bytes().into_owned()),
                ));
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        tokio::time::sleep(Duration::from_secs(3)).await;
        drop(sub);
        drop(live);
        let rows3 = rec.lock().unwrap().clone();
        let f = binding_file(results, "replayed", "vehicle-01/cam-front")?;
        let mut args = vec!["s13-detector".to_owned(), "--connect".into(), ep.clone(), "--bindings".into(), f.display().to_string()];
        args.extend(["--secs".into(), "5".into(), "--namespace".into(), "replay".into()]);
        let mut det = procs::spawn(&args, Duration::from_secs(20)).await?;
        let out = Topo { mode: Mode::Client, listen: Vec::new(), connect: vec![ep.clone()], namespace: Some("replay".into()), shm: None }.open().await?;
        let start = Instant::now();
        let base = rows3.first().map_or(Duration::ZERO, |r| r.0);
        for (at, payload, enc, att) in &rows3 {
            let due = start + at.saturating_sub(base);
            tokio::time::sleep_until(due.into()).await;
            let mut b = out.put("zk2/vehicle-01/cam-front/camera.v1/@stream/image", payload.clone()).encoding(enc.clone());
            if let Some(a) = att {
                b = b.attachment(a.clone());
            }
            b.await.map_err(|e| anyhow!("{e}"))?;
        }
        let line = det.line(Duration::from_secs(20)).await.ok_or_else(|| anyhow!("detector: no stats"))?;
        let st: serde_json::Value = serde_json::from_str(line.strip_prefix("stats ").unwrap_or("{}"))?;
        let rx = st["received"].as_object().cloned().unwrap_or_default();
        let n: u64 = rx.values().filter_map(serde_json::Value::as_u64).sum();
        rows.push(Row {
            case: "namespace-aware replay (capture, then republish through a session in namespace `replay`): detector in `replay`, bound to the original address".into(),
            value: format!("{} captured; {n} received; keys as the detector sees them {:?}", rows3.len(), rx.keys().collect::<Vec<_>>()),
            pass: n + 2 >= rows3.len() as u64 && rows3.len() > 10 && rx.keys().all(|k| k == "zk2/vehicle-01/cam-front/camera.v1/@stream/image"),
        });
    }

    // Simulated time: the deadline runs on the bound clock.
    let l = zenkey_model::contract::load_path(&twist);
    let c = l.contract.ok_or_else(|| anyhow!("{}", l.report))?;
    let deadline_ms = c.resources[0].annotations.get("timing.deadline_ms").and_then(serde_json::Value::as_u64).unwrap_or(100);
    for speed in [1.0, 0.0, 2.0] {
        let clockp = procs::spawn(&["s13-clock".into(), "--connect".into(), ep.clone(), "--speed".into(), speed.to_string()], Duration::from_secs(20)).await?;
        let cmd = procs::spawn(
            &["s11-cmd".into(), "--connect".into(), ep.clone(), "--name".into(), "autopilot".into(), "--contract".into(), twist.display().to_string()],
            Duration::from_secs(20),
        )
        .await?;
        let s = Topo::client(&[ep.clone()]).open().await?;
        let sim_now = Arc::new(Mutex::new(0u64));
        let last_cmd_sim = Arc::new(Mutex::new(None::<u64>));
        let last_cmd_wall = Arc::new(Mutex::new(None::<Instant>));
        let sn = sim_now.clone();
        let _clk = s
            .declare_subscriber(CLOCK_KEY)
            .callback(move |smp| {
                if let Ok(b) = <[u8; 8]>::try_from(&*smp.payload().to_bytes()) {
                    *sn.lock().unwrap() = u64::from_le_bytes(b);
                }
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let (sn, lc, lw) = (sim_now.clone(), last_cmd_sim.clone(), last_cmd_wall.clone());
        let _cmd = s
            .declare_subscriber("zk2/vehicle-01/autopilot/twist_cmd.v1/stream/cmd")
            .callback(move |_| {
                let now = *sn.lock().unwrap();
                *lc.lock().unwrap() = Some(now);
                *lw.lock().unwrap() = Some(Instant::now());
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        std::process::Command::new("kill").args(["-STOP".to_owned(), cmd.pid.to_string()]).status()?;
        let t0 = Instant::now();
        let limit = Duration::from_secs(3);
        let missed = loop {
            let now = *sim_now.lock().unwrap();
            let last = *last_cmd_sim.lock().unwrap();
            if let Some(last) = last {
                if now.saturating_sub(last) > deadline_ms * 1_000_000 {
                    let from = last_cmd_wall.lock().unwrap().unwrap_or(t0);
                    break Some(from.elapsed());
                }
            }
            if t0.elapsed() > limit {
                break None;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        };
        let (value, pass) = match (speed, missed) {
            (s, None) if s == 0.0 => ("no miss in 3 s of wall time (clock paused)".to_owned(), true),
            (_, None) => ("no miss in 3 s".to_owned(), false),
            (s, Some(d)) => {
                let wall = d.as_secs_f64() * 1e3;
                let expect = deadline_ms as f64 / s;
                // Within one clock tick (10 ms of wall) plus scheduling.
                (format!("miss {wall:.0} ms of wall time after the last command (expected about {expect:.0}, within a 10 ms clock tick)"), s > 0.0 && (wall - expect).abs() < 30.0)
            }
        };
        rows.push(Row { case: format!("clock.v1 at speed {speed}: commander silent, {deadline_ms} ms deadline on the bound clock"), value, pass });
        drop((clockp, cmd));
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s13.csv");
    let mut md = format!("# S13 — simulation and replay (#596)\n\nWritten by `spike s13`; the latest run (`unix_s` {unix}), zenoh {}. Code changes needed between cases: **none** (one detector binary; bindings and the clock are configuration).\n\n| Case | Value | Pass |\n|---|---|---|\n", zk2rt::ZENOH_VERSION);
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "case", "value", "pass"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.case.clone(), r.value.clone(), r.pass.to_string()])?;
        md.push_str(&format!("| {} | {} | {} |\n", r.case, r.value.replace('|', "\\|"), if r.pass { "yes" } else { "**no**" }));
        println!("{:5} {} → {}", if r.pass { "ok" } else { "FAIL" }, r.case, r.value);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(rows.iter().all(|r| r.pass))
}
