//! S12, store-and-forward commanding (#595, r3 §7, walkthrough §4.3,
//! `desired.v1`). Decides U-A, U1, U2.
//!
//! `ground/fleet-mgr` owns `state plans/{vehicle}` (one template per document
//! type, D22). `vehicle-01/executor` binds `{vehicle} = self`. The vehicle's
//! router reaches the ground router through a proxy: cutting it takes the
//! vehicle offline. A storage keeps `plans/*`, on the ground router or on the
//! vehicle router. The executor applies a plan only if its timestamp is
//! newer than the one applied (stale-command rejection), re-reads on the
//! fleet manager's presence and every 2 s (R7), and reports its observed
//! revision on its own state key.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenoh::sample::SampleKind;
use zenoh::time::{NTP64, Timestamp};
use zk2rt::config::Topo;
use zk2rt::metrics::csv_row;
use zk2rt::proxy::Proxy;

use crate::procs;
use crate::s5::{Producer, storage_with};

const PLAN: &str = "zk2/ground/fleet-mgr/desired.v1/state/plans/vehicle-01";
const PLANS: &str = "zk2/ground/fleet-mgr/desired.v1/state";
const OBSERVED: &str = "zk2/vehicle-01/executor/desired.v1/state/observed";
const MGR_TOKEN: &str = "zk2/ground/fleet-mgr/@zk/instance/*";

#[derive(Default)]
struct Applied {
    rev: Option<String>,
    ts: Option<NTP64>,
    /// Every applied revision, in order, with the time it was applied.
    log: Vec<(Instant, Option<String>)>,
    rejected: u64,
}

impl Applied {
    /// Applies `rev` (None = deleted) if it is newer than what is applied.
    fn offer(&mut self, rev: Option<String>, ts: Option<Timestamp>) {
        let t = ts.map(|t| *t.get_time());
        let newer = match (self.ts, t) {
            (None, _) => true,
            (Some(a), Some(b)) => b > a,
            (Some(_), None) => false,
        };
        if !newer {
            if t != self.ts {
                self.rejected += 1;
            }
            return;
        }
        if rev != self.rev || self.log.is_empty() {
            self.log.push((Instant::now(), rev.clone()));
        }
        self.rev = rev;
        self.ts = t;
    }
}

struct Executor {
    applied: Arc<Mutex<Applied>>,
    s: zenoh::Session,
    _observed: zenoh::query::Queryable<()>,
    _sub: zenoh::pubsub::Subscriber<()>,
    _live: zenoh::pubsub::Subscriber<()>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Executor {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

async fn reread(s: &zenoh::Session, a: &Arc<Mutex<Applied>>) {
    if let Ok(got) = zk2rt::client::state_get(s, PLAN, Duration::from_secs(1)).await {
        for g in got.into_iter().flatten() {
            let rev = g.value.map(|v| String::from_utf8_lossy(&v).into_owned());
            a.lock().unwrap().offer(rev, g.timestamp);
        }
    }
}

async fn executor(vehicle_ep: &str) -> Result<Executor> {
    let s = Topo::client(&[vehicle_ep.to_owned()]).open().await?;
    let applied: Arc<Mutex<Applied>> = Arc::default();
    let a = applied.clone();
    let sub = s
        .declare_subscriber(PLAN)
        .callback(move |smp| {
            let rev = (smp.kind() == SampleKind::Put).then(|| String::from_utf8_lossy(&smp.payload().to_bytes()).into_owned());
            a.lock().unwrap().offer(rev, smp.timestamp().copied());
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    // R5: the fleet manager coming (back) into view triggers a re-read.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let live = s
        .liveliness()
        .declare_subscriber(MGR_TOKEN)
        .history(true)
        .callback(move |smp| {
            if smp.kind() == SampleKind::Put {
                let _ = tx.send(());
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let mut tasks = Vec::new();
    let (s2, a2) = (s.clone(), applied.clone());
    tasks.push(tokio::spawn(async move {
        while rx.recv().await.is_some() {
            reread(&s2, &a2).await;
        }
    }));
    // R7: a static fallback, every 2 s.
    let (s3, a3) = (s.clone(), applied.clone());
    tasks.push(tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(2));
        loop {
            t.tick().await;
            reread(&s3, &a3).await;
        }
    }));
    // The observed revision, reported on the executor's own key.
    let (s4, a4) = (s.clone(), applied.clone());
    tasks.push(tokio::spawn(async move {
        let mut last = None::<Option<String>>;
        let mut t = tokio::time::interval(Duration::from_millis(100));
        loop {
            t.tick().await;
            let cur = a4.lock().unwrap().rev.clone();
            if last.as_ref() != Some(&cur) {
                let v = cur.clone().unwrap_or_else(|| "none".into());
                let _ = s4.put(OBSERVED, v).timestamp(s4.new_timestamp()).await;
                last = Some(cur);
            }
        }
    }));
    // The executor owns its observed-revision state, so it answers GETs on it.
    let a5 = applied.clone();
    let observed = s
        .declare_queryable(OBSERVED)
        .callback(move |q| {
            use zenoh::Wait;
            let v = a5.lock().unwrap().rev.clone().unwrap_or_else(|| "none".into());
            let _ = q.reply(OBSERVED, v).wait();
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    reread(&s, &applied).await;
    Ok(Executor { applied, s, _observed: observed, _sub: sub, _live: live, tasks })
}

struct Row {
    variant: &'static str,
    phase: String,
    truth: String,
    applied: String,
    wrong: bool,
    note: String,
}

fn show(v: &Option<String>) -> String {
    v.clone().unwrap_or_else(|| "(deleted)".into())
}

/// Waits until the executor applies `truth`, up to `limit`; the time taken.
async fn converge(e: &Executor, truth: &Option<String>, limit: Duration) -> Option<Duration> {
    let t0 = Instant::now();
    loop {
        if e.applied.lock().unwrap().rev == *truth {
            return Some(t0.elapsed());
        }
        if t0.elapsed() > limit {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The ground's view of the executor's observed revision.
async fn observed(ground: &zenoh::Session) -> String {
    zk2rt::client::state_get(ground, OBSERVED, Duration::from_secs(1))
        .await
        .ok()
        .and_then(|g| g.into_iter().flatten().find_map(|g| g.value))
        .map_or("?".into(), |v| String::from_utf8_lossy(&v).into_owned())
}

/// Revisions applied after `since` that are older than a revision applied
/// before them: stale commands that got through.
fn stale_applied(e: &Executor) -> usize {
    let a = e.applied.lock().unwrap();
    let num = |r: &Option<String>| r.as_ref().and_then(|s| s.strip_prefix("rev").and_then(|n| n.parse::<u32>().ok()));
    let mut max = 0;
    let mut stale = 0;
    for (_, r) in &a.log {
        if let Some(n) = num(r) {
            if n < max {
                stale += 1;
            }
            max = max.max(n);
        }
    }
    stale
}

#[allow(clippy::too_many_lines)]
async fn variant(storage_on_vehicle: bool, rows: &mut Vec<Row>) -> Result<()> {
    let tag: &'static str = if storage_on_vehicle { "storage on the vehicle" } else { "storage on the ground" };
    let (_g, g_ep) = procs::router(&[]).await?;
    let link = Proxy::start(g_ep.strip_prefix("tcp/").unwrap_or(&g_ep).to_owned(), Duration::ZERO).await?;
    let (_v, v_ep) = procs::router(&[link.endpoint()]).await?;
    let (_stor, _sp) = storage_with(if storage_on_vehicle { &v_ep } else { &g_ep }, &["plans=zk2/ground/fleet-mgr/desired.v1/state/**"], false, 30).await?;
    let ground = Topo::client(&[g_ep.clone()]).open().await?;
    let mgr = Producer::new(&g_ep, PLANS, 0, true, Duration::from_secs(60)).await?;
    let _mgr_tok = mgr.s.liveliness().declare_token("zk2/ground/fleet-mgr/@zk/instance/00000000000000f1").await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut truth = Some("rev1".to_owned());
    mgr.put(PLAN, "rev1").await?;
    let mut ex = executor(&v_ep).await?;
    let limit = Duration::from_secs(15);
    let check = |rows: &mut Vec<Row>, phase: String, truth: &Option<String>, ex: &Executor, took: Option<Duration>, extra: String| {
        let applied = ex.applied.lock().unwrap().rev.clone();
        let note = match took {
            Some(d) => format!("converged in {:.0} ms{extra}", d.as_secs_f64() * 1e3),
            None => format!("did not converge in {} s{extra}", limit.as_secs()),
        };
        rows.push(Row { variant: tag, phase, truth: show(truth), applied: show(&applied), wrong: applied != *truth, note });
    };
    let took = converge(&ex, &truth, limit).await;
    check(rows, "online: rev1".into(), &truth, &ex, took, String::new());

    // Offline: one change, many changes, a delete.
    let many: Vec<String> = (3..=12).map(|i| format!("rev{i}")).collect();
    for (phase, writes) in [("offline, plan changed once", vec!["rev2".to_owned()]), ("offline, plan changed 10 times", many), ("offline, plan deleted", vec!["DELETE".to_owned()])] {
        link.cut().await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        for w in &writes {
            if w == "DELETE" {
                mgr.delete(PLAN).await?;
                truth = None;
            } else {
                mgr.put(PLAN, w).await?;
                truth = Some(w.clone());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let while_off = show(&ex.applied.lock().unwrap().rev);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let t_heal = Instant::now();
        link.heal();
        let took = converge(&ex, &truth, limit).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let obs = observed(&ground).await;
        check(rows, format!("{phase}, then the link heals"), &truth, &ex, took, format!("; applied while offline: {while_off}; ground sees observed = {obs} {:.1} s after the heal", t_heal.elapsed().as_secs_f64()));
    }
    // Back online.
    mgr.put(PLAN, "rev13").await?;
    truth = Some("rev13".into());
    let took = converge(&ex, &truth, limit).await;
    check(rows, "online again: rev13".into(), &truth, &ex, took, String::new());

    // The executor restarts.
    drop(ex);
    ex = executor(&v_ep).await?;
    let took = converge(&ex, &truth, limit).await;
    check(rows, "executor restart".into(), &truth, &ex, took, String::new());

    // Clock skew: the fleet manager restarts 5 s behind, naive and with U2.
    for rule in [false, true] {
        let m2 = Producer::new(&g_ep, PLANS, -5000, true, Duration::from_secs(60)).await?;
        if rule {
            m2.catch_up(PLAN).await?;
        }
        let rev = if rule { "rev15" } else { "rev14" };
        m2.put(PLAN, rev).await?;
        truth = Some(rev.into());
        let took = converge(&ex, &truth, Duration::from_secs(6)).await;
        let rejected = ex.applied.lock().unwrap().rejected;
        let label = if rule { "fleet manager restarts 5 s behind, with U2's catch-up" } else { "fleet manager restarts 5 s behind, naive" };
        check(rows, label.into(), &truth, &ex, took, format!("; stale-command rejections so far: {rejected}"));
        // Leave the newest revision in place for the next case.
        drop(m2);
    }
    // Clock ahead: the router re-stamps the live sample, not the GET reply,
    // so a later write by a right clock within the skew looks older.
    {
        let m3 = Producer::new(&g_ep, PLANS, 2000, true, Duration::from_secs(60)).await?;
        m3.put(PLAN, "rev16").await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        // The executor's periodic GET lands in the window: rev16 comes back
        // with the producer's +2 s stamp, not the router's re-stamp.
        reread(&ex.s, &ex.applied).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(m3);
        mgr.put(PLAN, "rev17").await?; // the fleet manager's clock is right
        truth = Some("rev17".into());
        let took = converge(&ex, &truth, Duration::from_secs(6)).await;
        let rejected = ex.applied.lock().unwrap().rejected;
        check(rows, "a fleet manager 2 s ahead writes rev16 (a GET reads it back at +2 s); 0.5 s later the right-clocked one writes rev17".into(), &truth, &ex, took, format!("; stale-command rejections so far: {rejected}"));
        tokio::time::sleep(Duration::from_secs(2)).await;
        mgr.put(PLAN, "rev18").await?;
        truth = Some("rev18".into());
        let took = converge(&ex, &truth, Duration::from_secs(6)).await;
        check(rows, "the next write once the 2 s skew has passed (rev18)".into(), &truth, &ex, took, String::new());
    }
    rows.push(Row { variant: tag, phase: "stale revisions applied over the whole run".into(), truth: "0".into(), applied: stale_applied(&ex).to_string(), wrong: stale_applied(&ex) > 0, note: String::new() });
    Ok(())
}

pub async fn run(results: &Path, storage_label: &str) -> Result<bool> {
    let mut rows = Vec::new();
    variant(false, &mut rows).await?;
    variant(true, &mut rows).await?;
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s12.csv");
    let wrong = rows.iter().filter(|r| r.wrong).count();
    let mut md = format!(
        "# S12 — store-and-forward (#595)\n\nWritten by `spike s12`; the latest run (`unix_s` {unix}), zenoh {} with the {storage_label} storage manager 1.10.1. **{wrong} wrong answers** in {} rows.\n\n| Storage | Phase | Truth | Applied | Wrong | Note |\n|---|---|---|---|---|---|\n",
        zk2rt::ZENOH_VERSION,
        rows.len()
    );
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "variant", "phase", "truth", "applied", "wrong", "note"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.variant.into(), r.phase.clone(), r.truth.clone(), r.applied.clone(), r.wrong.to_string(), r.note.clone()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", r.variant, r.phase, r.truth, r.applied, if r.wrong { "**WRONG**" } else { "" }, r.note));
        println!("{:6} [{}] {} → truth {} applied {} {}", if r.wrong { "WRONG" } else { "ok" }, r.variant, r.phase, r.truth, r.applied, r.note);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(true)
}
