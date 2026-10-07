//! S2, presence at fleet scale (#598, r3 §7 S2, §3.10). Decides U5, U11,
//! U18 and part of U20.
//!
//! Token holders are child processes (`spike s2-tokens`) on router R2; an
//! observer client sits on router R1; R2 reaches R1 through a proxy that
//! counts the bytes between the routers (the declaration traffic) and can
//! cut or blackhole the link. Layouts:
//! - `a`: an instance token + interface tokens under `@zk/alive/` (r3);
//! - `b`: interface tokens under the instance token (instance-first);
//! - `c`: instance tokens only;
//! - `members`: one parent service per host with member tokens (D9b).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenoh::sample::SampleKind;
use zk2rt::config::{Mode, Topo};
use zk2rt::metrics::{csv_row, proc_sample};
use zk2rt::proxy::Proxy;

use crate::procs::{self, Proc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Layout {
    A,
    B,
    C,
    Members,
}

fn hex16(v: u64) -> String {
    format!("{v:016x}")
}

/// The token keys of service `i` (instance id `id`).
fn keys(layout: Layout, i: usize, id: u64, interfaces: usize, members: usize) -> Vec<String> {
    let (sys, svc) = (format!("h{}", i / 6), format!("s{}", i % 6));
    let inst = format!("zk2/{sys}/{svc}/@zk/instance/{}", hex16(id));
    let fp = "0123456789abcdef";
    match layout {
        Layout::A => std::iter::once(inst.clone())
            .chain((0..interfaces).map(|j| format!("zk2/{sys}/{svc}/@zk/alive/if{j}.v1/{}/{fp}", hex16(id))))
            .collect(),
        Layout::B => std::iter::once(inst.clone()).chain((0..interfaces).map(|j| format!("{inst}/if{j}.v1/{fp}"))).collect(),
        Layout::C => vec![inst],
        Layout::Members => std::iter::once(inst)
            .chain((0..members).map(|m| format!("zk2/{sys}/{svc}/@zk/member/if0.v1/m{m}/{}", hex16(id))))
            .collect(),
    }
}

/// The token-holder child: `services` services over `sessions` sessions.
/// With `churn_hz`, re-mints one service at that rate (new tokens first,
/// then the old ones undeclared: D9a), optionally putting a descriptor.
#[allow(clippy::too_many_arguments)]
pub async fn tokens(connect: Vec<String>, layout: Layout, services: usize, first: usize, interfaces: usize, members: usize, sessions: usize, churn_hz: f64, descriptor: bool) -> Result<()> {
    let topo = Topo { mode: Mode::Client, listen: Vec::new(), connect, namespace: None, shm: None };
    let mut ss = Vec::new();
    for _ in 0..sessions.max(1) {
        ss.push(topo.open().await?);
    }
    let mut held: Vec<Vec<zenoh::liveliness::LivelinessToken>> = Vec::new();
    let mut n = 0;
    for i in first..first + services {
        let s = &ss[i % ss.len()];
        let mut toks = Vec::new();
        for k in keys(layout, i, i as u64, interfaces, members) {
            toks.push(s.liveliness().declare_token(k).await.map_err(|e| anyhow!("{e}"))?);
            n += 1;
        }
        held.push(toks);
    }
    println!("ready {n}");
    if churn_hz > 0.0 {
        let desc = vec![b'x'; 300];
        let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / churn_hz));
        let mut epoch: u64 = 1 << 32;
        let mut k = 0usize;
        loop {
            tick.tick().await;
            let idx = k % held.len();
            let i = first + idx;
            let s = &ss[i % ss.len()];
            epoch += 1;
            let mut fresh = Vec::new();
            for key in keys(layout, i, epoch, interfaces, members) {
                fresh.push(s.liveliness().declare_token(key).await.map_err(|e| anyhow!("{e}"))?);
            }
            if descriptor {
                let ik = format!("zk2/h{}/s{}/@zk/instance/{}", i / 6, i % 6, hex16(epoch));
                let _ = s.put(ik, desc.clone()).await;
            }
            held[idx] = fresh; // the old tokens drop after the new ones exist
            k += 1;
        }
    }
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[derive(Default)]
struct Seen {
    live: HashSet<String>,
    puts: u64,
    deletes: u64,
}

struct Observer {
    seen: Arc<Mutex<Seen>>,
    s: zenoh::Session,
    _sub: zenoh::pubsub::Subscriber<()>,
}

async fn observer(ep: &str) -> Result<Observer> {
    let s = Topo::client(&[ep.to_owned()]).open().await?;
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let s2 = seen.clone();
    let sub = s
        .liveliness()
        .declare_subscriber("zk2/*/*/@zk/**")
        .history(true)
        .callback(move |smp| {
            let mut g = s2.lock().unwrap();
            let k = smp.key_expr().as_str().to_owned();
            match smp.kind() {
                SampleKind::Put => {
                    g.puts += 1;
                    g.live.insert(k);
                }
                SampleKind::Delete => {
                    g.deletes += 1;
                    g.live.remove(&k);
                }
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    Ok(Observer { seen, s, _sub: sub })
}

async fn wait_count(o: &Observer, n: usize, limit: Duration) -> Option<Duration> {
    let t0 = Instant::now();
    loop {
        if o.seen.lock().unwrap().live.len() >= n {
            return Some(t0.elapsed());
        }
        if t0.elapsed() > limit {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn holders(ep: &str, layout: Layout, services: usize, first: usize, interfaces: usize, members: usize, sessions: usize, churn: f64, descriptor: bool) -> Result<Proc> {
    let mut a = vec!["s2-tokens".to_owned(), "--connect".into(), ep.to_owned(), "--layout".into(), format!("{layout:?}").to_lowercase()];
    for (k, v) in [("--services", services), ("--first", first), ("--interfaces", interfaces), ("--members", members), ("--sessions", sessions)] {
        a.extend([k.into(), v.to_string()]);
    }
    a.extend(["--churn-hz".into(), churn.to_string()]);
    if descriptor {
        a.push("--descriptor".into());
    }
    procs::spawn(&a, Duration::from_secs(300)).await
}

struct Topology {
    r1: Proc,
    r2: Proc,
    e1: String,
    e2: String,
    link: Proxy,
}

async fn topology() -> Result<Topology> {
    let (r1, e1) = procs::router(&[]).await?;
    let link = Proxy::start(e1.strip_prefix("tcp/").unwrap_or(&e1).to_owned(), Duration::ZERO).await?;
    let (r2, e2) = procs::router(&[link.endpoint()]).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(Topology { r1, r2, e1, e2, link })
}

struct Row {
    group: &'static str,
    case: String,
    value: String,
}

/// Appends a row to `s2.csv` at once, so a late failure loses nothing.
fn record(results: &Path, r: &Row) -> Result<()> {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    csv_row(&results.join("s2.csv"), &["unix_s", "zenoh", "group", "case", "value"], &[unix, zk2rt::ZENOH_VERSION.into(), r.group.into(), r.case.clone(), r.value.clone()])?;
    println!("{} → {}", r.case, r.value);
    Ok(())
}

fn rss(p: &Proc) -> u64 {
    proc_sample(p.pid).map(|s| s.rss_kib).unwrap_or(0)
}

/// The default-handler liveliness GET, in a child process: zenoh#2678's
/// deadlock blocks the thread inside the `.await`, so only a process can be
/// abandoned. Prints `count <n>`.
pub async fn get_child(connect: Vec<String>, subscribe_first: usize) -> Result<()> {
    let s = Topo::client(&connect).open().await?;
    // A tool that also watches presence: its session holds every token
    // locally before it queries.
    let seen = Arc::new(Mutex::new(0usize));
    let s2 = seen.clone();
    let _sub = if subscribe_first > 0 {
        let sub = s
            .liveliness()
            .declare_subscriber("zk2/*/*/@zk/**")
            .history(true)
            .callback(move |_| *s2.lock().unwrap() += 1)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let t0 = Instant::now();
        while *seen.lock().unwrap() < subscribe_first && t0.elapsed() < Duration::from_secs(30) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Some(sub)
    } else {
        None
    };
    println!("ready");
    let r = s.liveliness().get("zk2/*/*/@zk/**").timeout(Duration::from_secs(10)).await.map_err(|e| anyhow!("{e}"))?;
    let mut k = 0usize;
    while r.recv_async().await.is_ok() {
        k += 1;
    }
    println!("count {k}");
    Ok(())
}

/// Liveliness GETs over every token: zenoh's default 256-slot handler (in a
/// child), and a callback (no bound).
async fn big_gets(o: &Observer, ep: &str, n: usize) -> Result<(String, String)> {
    let mut fifo = String::new();
    for (label, sub) in [("fresh session", 0), ("session already watching presence", n)] {
        let t0 = Instant::now();
        let mut child = procs::spawn(&["s2-get".into(), "--connect".into(), ep.to_owned(), "--subscribe-first".into(), sub.to_string()], Duration::from_secs(60)).await?;
        let r = match child.line(Duration::from_secs(20)).await {
            Some(l) => format!("{}/{n} in {:.0} ms", l.strip_prefix("count ").unwrap_or(&l), t0.elapsed().as_secs_f64() * 1e3),
            None => "**hung** (no completion in 20 s; zenoh#2678)".to_owned(),
        };
        fifo.push_str(&format!("{label}: {r}; "));
        drop(child);
    }
    let t0 = Instant::now();
    let count = Arc::new(Mutex::new(0usize));
    let c2 = count.clone();
    o
        .s
        .liveliness()
        .get("zk2/*/*/@zk/**")
        .timeout(Duration::from_secs(10))
        .callback(move |_| *c2.lock().unwrap() += 1)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    // A callback get has no completion signal here: wait until the count is stable.
    let mut last = 0;
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let c = *count.lock().unwrap();
        if c == last && c > 0 || t0.elapsed() > Duration::from_secs(15) {
            break;
        }
        last = c;
    }
    let cb = format!("{}/{n} (callback)", *count.lock().unwrap());
    Ok((fifo, cb))
}

#[allow(clippy::too_many_lines)]
pub async fn run(results: &Path, quick: bool) -> Result<()> {
    std::fs::create_dir_all(results)?;
    let mut rows = Vec::new();
    let interfaces = 5;
    let counts: &[usize] = if quick { &[100, 1000] } else { &[100, 1_000, 10_000, 50_000] };
    // A. Scale × layout.
    for &n in counts {
        for layout in [Layout::A, Layout::B, Layout::C] {
            let t = topology().await?;
            let o = observer(&t.e1).await?;
            let services = match layout {
                Layout::C => n,
                _ => n / (1 + interfaces),
            };
            let total = match layout {
                Layout::C => services,
                _ => services * (1 + interfaces),
            };
            let sessions = services.min(200);
            let (r1a, r2a, oa) = (rss(&t.r1), rss(&t.r2), proc_sample(std::process::id())?.rss_kib);
            let (up0, down0) = t.link.bytes();
            let t0 = Instant::now();
            let h = holders(&t.e2, layout, services, 0, interfaces, 0, sessions, 0.0, false).await?;
            let declared = t0.elapsed();
            let disc = wait_count(&o, total, Duration::from_secs(120)).await;
            let (up1, down1) = t.link.bytes();
            let (r1b, r2b, ob) = (rss(&t.r1), rss(&t.r2), proc_sample(std::process::id())?.rss_kib);
            let (fifo, cb) = if n >= 100 { big_gets(&o, &t.e1, total).await? } else { ("-".into(), "-".into()) };
            rows.push(Row {
                group: "scale",
                case: format!("{total} tokens, layout {layout:?} ({services} services, {sessions} sessions)"),
                value: format!(
                    "declared+ready {:.1} s; observer saw all {}; R1 +{} KiB, R2 +{} KiB, observer +{} KiB; R2->R1 {} KiB, R1->R2 {} KiB; liveliness GET default handler: {fifo}; {cb}",
                    declared.as_secs_f64(),
                    disc.map_or("**never**".to_owned(), |d| format!("in {:.1} s", d.as_secs_f64())),
                    r1b.saturating_sub(r1a),
                    r2b.saturating_sub(r2a),
                    ob.saturating_sub(oa),
                    (up1 - up0) / 1024,
                    (down1 - down0) / 1024,
                ),
            });
            record(results, rows.last().unwrap())?;
            drop((h, o, t));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    if quick {
        return write(results, &rows);
    }

    // B. The ZenSight shape and U18: device-as-service vs member tokens.
    for (label, layout, services, members) in [
        ("ZenSight: 1,000 hosts x 6 sensors x (1 instance + 5 interfaces)", Layout::A, 6_000, 0),
        ("U18 device-as-service: 5,000 SNMP devices x (1 instance + 2 interfaces)", Layout::A, 5_000, 0),
        ("U18 member tokens: one poller with 5,000 device members (D9b)", Layout::Members, 1, 5_000),
        ("member tokens at 512 containers x 20 hosts (D9b)", Layout::Members, 20, 512),
    ] {
        let ifs = if label.starts_with("U18 device") { 2 } else { interfaces };
        let t = topology().await?;
        let o = observer(&t.e1).await?;
        let total = match layout {
            Layout::Members => services * (1 + members),
            _ => services * (1 + ifs),
        };
        let (r1a, r2a) = (rss(&t.r1), rss(&t.r2));
        let (up0, _) = t.link.bytes();
        let h = holders(&t.e2, layout, services, 0, ifs, members, services.min(200), 0.0, false).await?;
        let disc = wait_count(&o, total, Duration::from_secs(120)).await;
        let (up1, _) = t.link.bytes();
        rows.push(Row {
            group: "shapes",
            case: format!("{label}: {total} tokens"),
            value: format!(
                "observer saw all {}; R1 +{} KiB, R2 +{} KiB; R2->R1 {} KiB",
                disc.map_or("**never**".to_owned(), |d| format!("in {:.1} s", d.as_secs_f64())),
                rss(&t.r1).saturating_sub(r1a),
                rss(&t.r2).saturating_sub(r2a),
                (up1 - up0) / 1024
            ),
        });
        record(results, rows.last().unwrap())?;
        drop((h, o, t));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // C. Disruptions at 10k tokens, layout a.
    {
        let services = 10_000 / (1 + interfaces);
        let total = services * (1 + interfaces);
        // A router restart: R2 dies and comes back on the same endpoint.
        let (r1, e1) = procs::router(&[]).await?;
        let listen2 = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
        let r2 = procs::spawn(&["router".into(), "--listen".into(), listen2.clone(), "--connect".into(), e1.clone()], Duration::from_secs(30)).await?;
        let o = observer(&e1).await?;
        let h = holders(&listen2, Layout::A, services, 0, interfaces, 0, 200, 0.0, false).await?;
        wait_count(&o, total, Duration::from_secs(60)).await;
        let d0 = o.seen.lock().unwrap().deletes;
        let t0 = Instant::now();
        drop(r2);
        let gone = loop {
            if o.seen.lock().unwrap().live.is_empty() || t0.elapsed() > Duration::from_secs(30) {
                break t0.elapsed();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let r2 = procs::spawn(&["router".into(), "--listen".into(), listen2.clone(), "--connect".into(), e1.clone()], Duration::from_secs(30)).await?;
        let back = wait_count(&o, total, Duration::from_secs(60)).await;
        rows.push(Row {
            group: "disruption",
            case: format!("router R2 restart under {total} tokens"),
            value: format!(
                "all gone from the observer {:.1} s after the kill ({} deletes); all back {} after R2 restarted",
                gone.as_secs_f64(),
                o.seen.lock().unwrap().deletes - d0,
                back.map_or("**never in 60 s**".to_owned(), |d| format!("in {:.1} s", d.as_secs_f64()))
            ),
        });
        record(results, rows.last().unwrap())?;
        drop((h, r2, r1, o));
    }
    {
        // A client moving to another router: its link to R2 is blackholed
        // (only the lease can tell); it also knows R1.
        let t = topology().await?;
        let o = observer(&t.e1).await?;
        let up = Proxy::start(t.e2.strip_prefix("tcp/").unwrap_or(&t.e2).to_owned(), Duration::ZERO).await?;
        let services = 200;
        let total = services * (1 + interfaces);
        let mut a = vec!["s2-tokens".to_owned(), "--connect".into(), up.endpoint(), "--connect".into(), t.e1.clone(), "--layout".into(), "a".into()];
        a.extend(["--services".into(), services.to_string(), "--interfaces".into(), interfaces.to_string(), "--sessions".into(), "20".into()]);
        let h = procs::spawn(&a, Duration::from_secs(60)).await?;
        wait_count(&o, total, Duration::from_secs(60)).await;
        let (p0, d0) = { let g = o.seen.lock().unwrap(); (g.puts, g.deletes) };
        up.blackhole();
        let t0 = Instant::now();
        let mut min_live = total;
        let mut timeline = Vec::new();
        while t0.elapsed() < Duration::from_secs(30) {
            let l = o.seen.lock().unwrap().live.len();
            min_live = min_live.min(l);
            if timeline.last().map(|(_, x)| *x) != Some(l) {
                timeline.push((t0.elapsed().as_secs_f64(), l));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (p1, d1, end) = { let g = o.seen.lock().unwrap(); (g.puts, g.deletes, g.live.len()) };
        rows.push(Row {
            group: "disruption",
            case: format!("{total} tokens' client loses R2 silently (blackhole) and moves to R1"),
            value: format!(
                "lowest live count {min_live}; {} deletes and {} puts seen; {end}/{total} live 30 s later; timeline (s, live): {:?}",
                d1 - d0,
                p1 - p0,
                timeline.iter().take(8).map(|(t, l)| format!("{t:.1}:{l}")).collect::<Vec<_>>()
            ),
        });
        record(results, rows.last().unwrap())?;
        drop((h, o, t, up));
    }
    // D. Churn: one service re-minted per second, with and without a
    // descriptor put, over 10k tokens; and the re-mint overlap (D9a).
    for descriptor in [false, true] {
        let t = topology().await?;
        let o = observer(&t.e1).await?;
        let services = 10_000 / (1 + interfaces);
        let total = services * (1 + interfaces);
        let _bg = holders(&t.e2, Layout::A, services, 0, interfaces, 0, 200, 0.0, false).await?;
        wait_count(&o, total, Duration::from_secs(60)).await;
        let watch = "zk2/h2000/s0/@zk/instance/";
        let (up0, _) = t.link.bytes();
        let cpu0 = proc_sample(t.r1.pid)?.cpu_ticks + proc_sample(t.r2.pid)?.cpu_ticks;
        let churn = holders(&t.e2, Layout::A, 1, 12_000, interfaces, 0, 1, 1.0, descriptor).await?;
        let t0 = Instant::now();
        let mut min_inst = usize::MAX;
        let mut max_inst = 0;
        while t0.elapsed() < Duration::from_secs(10) {
            let n = o.seen.lock().unwrap().live.iter().filter(|k| k.starts_with(watch)).count();
            if t0.elapsed() > Duration::from_secs(1) {
                min_inst = min_inst.min(n);
                max_inst = max_inst.max(n);
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (up1, _) = t.link.bytes();
        let cpu1 = proc_sample(t.r1.pid)?.cpu_ticks + proc_sample(t.r2.pid)?.cpu_ticks;
        rows.push(Row {
            group: "churn",
            case: format!("1 re-mint/s for 10 s over {total} tokens, descriptor put on change: {descriptor}"),
            value: format!(
                "R2->R1 {:.1} KiB/s; routers' CPU {:.1}% of a core; the re-minted service's live instance tokens stayed within [{min_inst}, {max_inst}] (0 = a gap)",
                (up1 - up0) as f64 / 1024.0 / 10.0,
                (cpu1 - cpu0) as f64 * 10.0 / 10_000.0 * 100.0
            ),
        });
        record(results, rows.last().unwrap())?;
        drop((churn, o, t));
    }
    write(results, &rows)
}

fn write(results: &Path, rows: &[Row]) -> Result<()> {
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s2.csv");
    let mut md = format!("# S2 — presence at fleet scale (#598)\n\nWritten by `spike s2`; the latest run (`unix_s` {unix}), zenoh {}. Token holders on R2, the observer on R1; R2 reaches R1 through a byte-counting proxy.\n\n| Group | Case | Value |\n|---|---|---|\n", zk2rt::ZENOH_VERSION);
    let _ = &csv;
    for r in rows {
        md.push_str(&format!("| {} | {} | {} |\n", r.group, r.case, r.value.replace('|', "\\|")));
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(())
}
