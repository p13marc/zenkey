//! S4, contract retrieval by hash against bad holders (#600, r3 §3.10).
//!
//! The rule under test: GET `zk2/@zk/contract/<iface>/<sha256>` with
//! `BestMatching` (holders declare `complete`), verify every reply, accept
//! the first valid one; on none, retry once with `All`; then report the
//! contract unavailable.
//!
//! Holders are child processes (`spike s4-holder`) in one of four modes:
//! `ok`, `slow` (replies after 3 s), `corrupt` (one byte flipped) and
//! `small-only` (a constrained holder that cannot send more than 4 KB,
//! like zenoh-pico's default fragment limit, so it never answers a larger
//! bundle). "Nearest" is the holder on the client's own router; the others
//! sit behind a second router.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::grammar::contract_key;
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zk2rt::config::Topo;
use zk2rt::metrics::csv_row;

use crate::procs::{self, Proc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum HolderMode {
    Ok,
    Slow,
    Corrupt,
    SmallOnly,
}

/// `count` holders of one contract's bundle, until killed.
pub async fn holder(connect: Vec<String>, contract: &Path, mode: HolderMode, count: usize) -> Result<()> {
    let l = load_path(contract);
    let c = l.contract.ok_or_else(|| anyhow!("{}", l.report))?;
    let key = contract_key(&c.iface, Fingerprint::of(&c).hex())?.as_str().to_owned();
    let mut bytes = Bundle::build(&c).to_bytes();
    if mode == HolderMode::Corrupt {
        let i = bytes.len() / 2;
        bytes[i] ^= 0x01;
    }
    let mut keep = Vec::new();
    for _ in 0..count {
        let s = Topo::client(&connect).open().await?;
        let (k, b) = (key.clone(), bytes.clone());
        let q = s
            .declare_queryable(key.clone())
            .complete(true)
            .callback(move |q| {
                let (k, b) = (k.clone(), b.clone());
                match mode {
                    HolderMode::Ok | HolderMode::Corrupt => {
                        let _ = q.reply(k, b).encoding(Encoding::APPLICATION_JSON).wait();
                    }
                    HolderMode::Slow => {
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_secs(3));
                            let _ = q.reply(k, b).encoding(Encoding::APPLICATION_JSON).wait();
                        });
                    }
                    HolderMode::SmallOnly => {
                        if b.len() <= 4096 {
                            let _ = q.reply(k, b).encoding(Encoding::APPLICATION_JSON).wait();
                        }
                    }
                }
            })
            .await
            .map_err(|e| anyhow!("{e}"))?;
        keep.push((s, q));
    }
    println!("ready");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[derive(Debug, Default)]
struct Fetch {
    accepted: bool,
    bad_accepted: bool,
    attempts: u32,
    replies: usize,
    invalid: usize,
    bytes: usize,
    /// Time to the first valid reply (or to giving up).
    ms: f64,
    /// Time until every routed holder answered or timed out: what a client
    /// that waits for completion would pay.
    complete_ms: f64,
}

/// The retrieval rule, counting everything. Replies are verified as they
/// arrive and the first valid one is accepted at once; the GET is then
/// drained only to measure when it would have completed.
async fn fetch(s: &zenoh::Session, c: &Contract, timeout: Duration) -> Result<Fetch> {
    let fp = Fingerprint::of(c);
    let key = contract_key(&c.iface, fp.hex())?;
    let mut f = Fetch::default();
    let t0 = Instant::now();
    for (attempt, target) in [(1, QueryTarget::BestMatching), (2, QueryTarget::All)] {
        f.attempts = attempt;
        let replies = s
            .get(key.as_str())
            .target(target)
            .consolidation(ConsolidationMode::None)
            .timeout(timeout)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        while let Ok(r) = replies.recv_async().await {
            f.replies += 1;
            let Ok(smp) = r.result() else { continue };
            let v = smp.payload().to_bytes();
            f.bytes += v.len();
            match Bundle::verify_expecting(&v, &fp) {
                Ok(b) => {
                    if !f.accepted {
                        f.accepted = true;
                        f.ms = t0.elapsed().as_secs_f64() * 1e3;
                        // Defence in depth: what was accepted must hash right.
                        f.bad_accepted = b.fingerprint() != fp;
                    }
                }
                Err(_) => f.invalid += 1,
            }
        }
        if f.accepted {
            break;
        }
    }
    f.complete_ms = t0.elapsed().as_secs_f64() * 1e3;
    if !f.accepted {
        f.ms = f.complete_ms;
    }
    Ok(f)
}

async fn holders(ep: &str, contract: &Path, mode: HolderMode, count: usize) -> Result<Proc> {
    let args = vec![
        "s4-holder".to_owned(),
        "--connect".into(),
        ep.to_owned(),
        "--mode".into(),
        format!("{mode:?}").to_lowercase().replace("smallonly", "small-only"),
        "--count".into(),
        count.to_string(),
        contract.display().to_string(),
    ];
    procs::spawn(&args, Duration::from_secs(120)).await
}

struct Row {
    case: String,
    f: Fetch,
    expect: &'static str,
    pass: bool,
}

#[allow(clippy::too_many_lines)]
pub async fn run(results: &Path, examples: &Path) -> Result<bool> {
    let small_p = examples.join("walkthrough/camera.v1.toml");
    let large_p = examples.join("zensight/zs.sysinfo.v1.toml");
    let load = |p: &Path| -> Result<Contract> {
        let l = load_path(p);
        l.contract.ok_or_else(|| anyhow!("{}", l.report))
    };
    let (small, large) = (load(&small_p)?, load(&large_p)?);
    let t = Duration::from_secs(1);
    let mut rows = Vec::new();
    let (_r1, e1) = procs::router(&[]).await?;
    let (_r2, e2) = procs::router(std::slice::from_ref(&e1)).await?;
    let (_r3, e3) = procs::router(std::slice::from_ref(&e2)).await?;
    let client = Topo::client(&[e1.clone()]).open().await?;
    let settle = || tokio::time::sleep(Duration::from_millis(500));
    let ok = |f: &Fetch| f.accepted && !f.bad_accepted;

    // One holder, then 200, on the client's router.
    {
        let h = holders(&e1, &small_p, HolderMode::Ok, 1).await?;
        settle().await;
        let f = fetch(&client, &small, t).await?;
        let p = ok(&f) && f.attempts == 1;
        rows.push(Row { case: "one holder (camera.v1, 1.7 KB)".into(), f, expect: "accepted on attempt 1", pass: p });
        drop(h);
    }
    {
        let h = holders(&e1, &small_p, HolderMode::Ok, 200).await?;
        settle().await;
        let f = fetch(&client, &small, t).await?;
        let p = ok(&f) && f.attempts == 1 && f.replies == 1;
        rows.push(Row { case: "200 equal holders, BestMatching".into(), f, expect: "accepted, exactly 1 reply", pass: p });
        let key = contract_key(&small.iface, Fingerprint::of(&small).hex())?;
        let t0 = Instant::now();
        let all = zk2rt::client::get(&client, key.as_str(), QueryTarget::All, ConsolidationMode::None, None, t).await?;
        let n = all.iter().flatten().count();
        let bytes: usize = all.iter().flatten().filter_map(|g| g.value.as_ref().map(Vec::len)).sum();
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        let f = Fetch { accepted: n > 0, attempts: 1, replies: n, bytes, ms, complete_ms: ms, ..Default::default() };
        rows.push(Row { case: "200 equal holders, target All (for comparison)".into(), f, expect: "200 replies", pass: n == 200 });
        drop(h);
    }

    // Bad nearest holders, good ones behind the second router.
    let far = holders(&e2, &small_p, HolderMode::Ok, 3).await?;
    for (label, mode) in [("a slow nearest holder (3 s)", HolderMode::Slow), ("a corrupt nearest holder", HolderMode::Corrupt)] {
        let near = holders(&e1, &small_p, mode, 1).await?;
        settle().await;
        let f = fetch(&client, &small, t).await?;
        let p = ok(&f);
        rows.push(Row { case: format!("{label}; 3 good behind router 2"), f, expect: "accepted (by the retry if the nearest is chosen); bad never accepted", pass: p });
        drop(near);
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    {
        let near = holders(&e1, &small_p, HolderMode::Ok, 1).await?;
        settle().await;
        let warm = fetch(&client, &small, t).await?;
        std::process::Command::new("kill").args(["-STOP".to_owned(), near.pid.to_string()]).status()?;
        let f = fetch(&client, &small, t).await?;
        let p = ok(&f);
        rows.push(Row {
            case: format!("an unreachable nearest holder (SIGSTOP after convergence; warm fetch {:.1} ms); 3 good behind router 2", warm.ms),
            f,
            expect: "accepted by the retry",
            pass: p,
        });
        std::process::Command::new("kill").args(["-CONT".to_owned(), near.pid.to_string()]).status()?;
        drop(near);
    }
    drop(far);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Every holder corrupt: unavailable, never a bad accept.
    {
        let h = holders(&e1, &small_p, HolderMode::Corrupt, 3).await?;
        settle().await;
        let f = fetch(&client, &small, t).await?;
        let p = !f.accepted && !f.bad_accepted && f.attempts == 2;
        rows.push(Row { case: "every holder corrupt (3)".into(), f, expect: "unavailable after the retry; never accepted", pass: p });
        drop(h);
    }

    // Behind three routers.
    {
        let h = holders(&e3, &small_p, HolderMode::Ok, 1).await?;
        settle().await;
        let f = fetch(&client, &small, t).await?;
        let p = ok(&f) && f.attempts == 1;
        rows.push(Row { case: "the holder three routers away".into(), f, expect: "accepted on attempt 1", pass: p });
        drop(h);
    }

    // A constrained holder that cannot send the 84 KB bundle, nearest; a
    // router-side holder (a gateway) serves it.
    {
        let near = holders(&e1, &large_p, HolderMode::SmallOnly, 1).await?;
        let gw = holders(&e2, &large_p, HolderMode::Ok, 1).await?;
        settle().await;
        let f = fetch(&client, &large, t).await?;
        let p = ok(&f);
        rows.push(Row {
            case: format!("a constrained nearest holder (4 KB limit) for zs.sysinfo.v1 ({} B); a gateway holder behind router 2", Bundle::build(&large).to_bytes().len()),
            f,
            expect: "accepted from the gateway",
            pass: p,
        });
        drop((near, gw));
    }

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s4.csv");
    let header = ["unix_s", "zenoh", "case", "accepted", "bad_accepted", "attempts", "replies", "invalid", "bytes", "accept_ms", "complete_ms", "expected", "pass"];
    let mut md = format!(
        "# S4 — contract retrieval by hash (#600)\n\nWritten by `spike s4`; the latest run (`unix_s` {unix}), zenoh {}. Per-attempt timeout 1 s. `accept` is the time to the first valid reply; `complete` is when the GET completed (all routed holders answered or timed out).\n\n| Case | Accepted | Attempts | Replies (invalid) | Bytes | accept ms | complete ms | Pass |\n|---|---|---|---|---|---|---|---|\n",
        zk2rt::ZENOH_VERSION
    );
    for r in &rows {
        let f = &r.f;
        csv_row(&csv, &header, &[
            unix.clone(), zk2rt::ZENOH_VERSION.into(), r.case.clone(), f.accepted.to_string(), f.bad_accepted.to_string(),
            f.attempts.to_string(), f.replies.to_string(), f.invalid.to_string(), f.bytes.to_string(), format!("{:.1}", f.ms), format!("{:.1}", f.complete_ms),
            r.expect.into(), r.pass.to_string(),
        ])?;
        md.push_str(&format!("| {} | {} | {} | {} ({}) | {} | {:.1} | {:.1} | {} |\n", r.case, f.accepted, f.attempts, f.replies, f.invalid, f.bytes, f.ms, f.complete_ms, if r.pass { "yes" } else { "**no**" }));
        println!("{:5} {} → accepted {} attempts {} replies {} invalid {} bytes {} accept {:.1} ms complete {:.1} ms", if r.pass { "ok" } else { "FAIL" }, r.case, f.accepted, f.attempts, f.replies, f.invalid, f.bytes, f.ms, f.complete_ms);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(rows.iter().all(|r| r.pass))
}
