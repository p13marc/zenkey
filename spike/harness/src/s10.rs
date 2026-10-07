//! S10, wildcard bindings (#593, r3 §3.4 R1, R3, R5, R6).
//!
//! A consumer binds `detections.v1` as `sources` to `vehicle-01/*`: it
//! subscribes to `zk2/vehicle-01/*/detections.v1/stream/**`, waits on the
//! interface tokens under the same pattern (R5), discards non-concrete keys
//! (R6), and serves a descriptor that records the binding (R3). Producers
//! are mock services in child processes, publishing every stream at 10 Hz
//! with (seq, stamp) in the attachment.
//!
//! Phases: fan-in 1 → 10 → 100; a producer of another interface on the same
//! system; steady state; churn (clean exit, `kill -9`, and SIGSTOP as a
//! network cut that only the lease can detect); a system-position wildcard
//! (`*/tc`); an injection on a wildcard key; and the graph drawn from
//! descriptors and tokens alone.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use zenkey_model::grammar::{ZkKey, parse};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::sample::SampleKind;
use zk2rt::config::Topo;
use zk2rt::metrics::{csv_row, proc_sample};

use crate::procs::{self, Proc};

#[derive(Default, Debug, Clone)]
struct Producer {
    joined: Option<Instant>,
    left: Option<Instant>,
    first_sample: Option<Instant>,
    samples: u64,
    last_seq: Option<u64>,
    gaps: u64,
    dups: u64,
}

#[derive(Default)]
struct Consumer {
    producers: BTreeMap<String, Producer>,
    discarded_wild: u64,
    wrong_interface: u64,
}

struct Row {
    phase: &'static str,
    measure: String,
    value: String,
    expected: String,
    pass: bool,
}

fn row(phase: &'static str, measure: impl Into<String>, value: impl ToString, expected: impl Into<String>, pass: bool) -> Row {
    Row { phase, measure: measure.into(), value: value.to_string(), expected: expected.into(), pass }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn quantiles(mut v: Vec<f64>) -> (f64, f64, f64) {
    if v.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    (q(0.0), q(0.5), q(1.0))
}

async fn group(ep: &str, system: &str, first: usize, count: usize, extra: &[&str], contract: &Path) -> Result<Proc> {
    let mut args: Vec<String> = vec![
        "services".into(),
        "--connect".into(),
        ep.into(),
        "--system".into(),
        system.into(),
        "--count".into(),
        count.to_string(),
        "--first".into(),
        first.to_string(),
    ];
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args.push(contract.display().to_string());
    procs::spawn(&args, Duration::from_secs(120)).await
}

fn signal(p: &Proc, sig: &str) -> Result<()> {
    let ok = std::process::Command::new("kill").args([format!("-{sig}"), p.pid.to_string()]).status()?.success();
    if ok { Ok(()) } else { Err(anyhow!("kill -{sig} {} failed", p.pid)) }
}

fn names(prefix: &str, first: usize, count: usize) -> Vec<String> {
    (first..first + count).map(|i| format!("{prefix}{i}")).collect()
}

/// Waits until every named producer has left (a liveliness delete), up to
/// `limit`; returns each one's leave latency from `t0`.
async fn wait_left(st: &Arc<Mutex<Consumer>>, who: &[String], t0: Instant, limit: Duration) -> Vec<f64> {
    loop {
        let done: Vec<f64> = {
            let g = st.lock().unwrap();
            who.iter().filter_map(|n| g.producers.get(n).and_then(|p| p.left)).map(|l| ms(l.saturating_duration_since(t0))).collect()
        };
        if done.len() == who.len() || t0.elapsed() > limit {
            return done;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[allow(clippy::too_many_lines)]
pub async fn run(results: &Path, examples: &Path) -> Result<bool> {
    let det = examples.join("walkthrough/detections.v1.toml");
    let nav = examples.join("walkthrough/nav.v2.toml");
    let tc = examples.join("tcgui/tc.netif.v1.toml");
    let mut rows = Vec::new();
    let (_router, ep) = procs::router(&[]).await?;
    let c = Topo::client(&[ep.clone()]).open().await?;
    let st: Arc<Mutex<Consumer>> = Arc::default();

    // R5: the interface tokens under the binding's pattern, with history.
    let st1 = st.clone();
    let _live = c
        .liveliness()
        .declare_subscriber("zk2/vehicle-01/*/@zk/alive/detections.v1/**")
        .history(true)
        .callback(move |s| {
            let Ok(ZkKey::Alive { addr, .. }) = parse(s.key_expr().as_str()) else { return };
            let mut g = st1.lock().unwrap();
            let p = g.producers.entry(addr.service.as_str().to_owned()).or_default();
            match s.kind() {
                SampleKind::Put => {
                    p.joined.get_or_insert_with(Instant::now);
                    p.left = None;
                }
                SampleKind::Delete => p.left = Some(Instant::now()),
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    // The binding's data: R6 discards non-concrete keys.
    let st2 = st.clone();
    let _data = c
        .declare_subscriber("zk2/vehicle-01/*/detections.v1/stream/**")
        .callback(move |s| {
            let ke = s.key_expr();
            let mut g = st2.lock().unwrap();
            if ke.is_wild() {
                g.discarded_wild += 1;
                return;
            }
            let Ok(ZkKey::Data { addr, iface, .. }) = parse(ke.as_str()) else { return };
            if iface.to_string() != "detections.v1" {
                g.wrong_interface += 1;
                return;
            }
            let seq = s
                .attachment()
                .and_then(|a| a.to_bytes().get(0..8).map(|b| u64::from_le_bytes(b.try_into().unwrap())));
            let p = g.producers.entry(addr.service.as_str().to_owned()).or_default();
            p.first_sample.get_or_insert_with(Instant::now);
            p.samples += 1;
            if let Some(seq) = seq {
                match p.last_seq {
                    Some(last) if seq <= last => p.dups += 1,
                    Some(last) => p.gaps += seq - last - 1,
                    None => {}
                }
                p.last_seq = Some(seq.max(p.last_seq.unwrap_or(0)));
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    // R3: the consumer's own presence and descriptor, binding recorded.
    let me = "zk2/vehicle-01/detector-sink/@zk/instance/00000000000000c0";
    let descriptor = json!({
        "service": "vehicle-01/detector-sink",
        "instance": "00000000000000c0",
        "interfaces": [],
        "requires": [{"role": "sources", "interface": "detections.v1", "cardinality": "many", "bindings": ["vehicle-01/*"]}],
        "meta": {"build": "zk2-spike"},
    });
    let desc_bytes = serde_json::to_vec(&descriptor)?;
    let db = desc_bytes.clone();
    let _dq = c
        .declare_queryable(me)
        .complete(true)
        .callback(move |q| {
            let _ = q.reply(me, db.clone()).encoding(Encoding::APPLICATION_JSON).wait();
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let _tok = c.liveliness().declare_token(me).await.map_err(|e| anyhow!("{e}"))?;
    let rss0 = proc_sample(std::process::id())?.rss_kib;

    // R5: the consumer starts with no provider and waits for the first.
    let t_wait = Instant::now();
    let mut groups: Vec<Proc> = Vec::new();
    let mut total = 0;
    for (first, n) in [(0, 1), (1, 9), (10, 90)] {
        let t0 = Instant::now();
        groups.push(group(&ep, "vehicle-01", first, n, &["--stream-hz", "10"], &det).await?);
        total += n;
        let want = names("svc-", 0, total);
        loop {
            let ok = {
                let g = st.lock().unwrap();
                want.iter().all(|w| g.producers.get(w).is_some_and(|p| p.joined.is_some() && p.first_sample.is_some()))
            };
            if ok || t0.elapsed() > Duration::from_secs(30) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if first == 0 {
            let g = st.lock().unwrap();
            let p = &g.producers["svc-0"];
            rows.push(row("start-up", "R5: wait for the first bound provider: token seen, then its first sample (ms after spawn)", format!("{:.0} / {:.0}", ms(p.joined.unwrap() - t_wait), ms(p.first_sample.unwrap() - t_wait)), "both seen", true));
        }
        let g = st.lock().unwrap();
        let seen: Vec<&Producer> = want.iter().filter_map(|w| g.producers.get(w)).collect();
        let joined = seen.iter().filter(|p| p.joined.is_some()).count();
        let sampling = seen.iter().filter(|p| p.first_sample.is_some()).count();
        let lat: Vec<f64> = seen
            .iter()
            .filter_map(|p| Some(ms(p.first_sample?.saturating_duration_since(p.joined?))))
            .collect();
        let (mn, md, mx) = quantiles(lat);
        rows.push(row("fan-in", format!("{total} producers: tokens seen / producers delivering"), format!("{joined} / {sampling}"), format!("{total} / {total}"), joined == total && sampling == total));
        rows.push(row("fan-in", format!("{total} producers: join (token) to first sample, ms min/median/max"), format!("{mn:.1} / {md:.1} / {mx:.1}"), "measured", true));
    }
    let rss1 = proc_sample(std::process::id())?.rss_kib;
    rows.push(row("fan-in", "consumer process RSS growth for 100 producers, KiB", rss1.saturating_sub(rss0), "measured", true));

    // Another interface on the same system: never picked up.
    let _other = group(&ep, "vehicle-01", 0, 1, &["--service-name", "nav", "--stream-hz", "10"], &nav).await?;

    // Steady state: every producer's sequence, gaps and duplicates.
    {
        let mut g = st.lock().unwrap();
        for p in g.producers.values_mut() {
            p.samples = 0;
            p.gaps = 0;
            p.dups = 0;
        }
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    {
        let g = st.lock().unwrap();
        let ps: Vec<Producer> = names("svc-", 0, 100).iter().filter_map(|n| g.producers.get(n).cloned()).collect();
        let samples: u64 = ps.iter().map(|p| p.samples).sum();
        let gaps: u64 = ps.iter().map(|p| p.gaps).sum();
        let dups: u64 = ps.iter().map(|p| p.dups).sum();
        rows.push(row("steady", "100 producers x 10 Hz for 5 s: samples received", samples, "about 5,000", samples >= 4_500));
        rows.push(row("steady", "sequence gaps (missed samples)", gaps, "0", gaps == 0));
        rows.push(row("steady", "duplicates", dups, "0", dups == 0));
        rows.push(row("wrong-interface", "samples of nav.v2 on vehicle-01 delivered to the detections.v1 binding", g.wrong_interface, "0", g.wrong_interface == 0));
    }

    // Churn: clean exit, kill -9, and SIGSTOP (a cut only the lease sees).
    let d = group(&ep, "vehicle-01", 100, 5, &["--stream-hz", "10"], &det).await?;
    let e = group(&ep, "vehicle-01", 105, 5, &["--stream-hz", "10"], &det).await?;
    let f = group(&ep, "vehicle-01", 110, 5, &["--stream-hz", "10"], &det).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    for (label, p, who, sig, limit) in [
        ("clean exit (SIGINT)", &d, names("svc-", 100, 5), "INT", 15),
        ("kill -9", &e, names("svc-", 105, 5), "KILL", 15),
        ("network cut (SIGSTOP; only the lease can tell)", &f, names("svc-", 110, 5), "STOP", 30),
    ] {
        let t0 = Instant::now();
        signal(p, sig)?;
        let lat = wait_left(&st, &who, t0, Duration::from_secs(limit)).await;
        let n = lat.len();
        let (mn, md, mx) = quantiles(lat);
        rows.push(row("churn", format!("{label}: leave detected for {n}/5; ms min/median/max"), format!("{mn:.0} / {md:.0} / {mx:.0}"), "5/5 detected", n == 5));
    }
    // The cut heals: SIGCONT, and does the producer come back?
    let t0 = Instant::now();
    signal(&f, "CONT")?;
    let back = loop {
        let n = {
            let g = st.lock().unwrap();
            names("svc-", 110, 5).iter().filter(|w| g.producers.get(*w).is_some_and(|p| p.left.is_none())).count()
        };
        if n == 5 || t0.elapsed() > Duration::from_secs(20) {
            break n;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    rows.push(row("churn", "after SIGCONT: producers back within 20 s (ms)", format!("{back}/5 in {:.0}", ms(t0.elapsed())), "measured", true));
    // Fan-in survived the churn: the 100 long-lived producers still deliver.
    {
        let before: BTreeMap<String, u64> = {
            let g = st.lock().unwrap();
            names("svc-", 0, 100).iter().map(|n| (n.clone(), g.producers.get(n).map_or(0, |p| p.samples))).collect()
        };
        tokio::time::sleep(Duration::from_secs(2)).await;
        let g = st.lock().unwrap();
        let still = before.iter().filter(|(n, b)| g.producers.get(*n).is_some_and(|p| p.samples > **b)).count();
        rows.push(row("churn", "the 100 long-lived producers still delivering after the churn", still, "100", still == 100));
    }
    drop((d, e, f));

    // A system-position wildcard: `*/tc`, on 2 and then 5 hosts.
    let mut hosts = Vec::new();
    for (k, upto) in [(0, 2), (2, 5)] {
        for h in k..upto {
            hosts.push(group(&ep, &format!("h{h}"), 0, 1, &["--service-name", "tc"], &tc).await?);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let got = zk2rt::client::state_get(&c, "zk2/*/tc/tc.netif.v1/state/namespaces", Duration::from_secs(2)).await?;
        let systems: BTreeSet<String> = got
            .into_iter()
            .flatten()
            .filter_map(|g| match parse(&g.key) {
                Ok(ZkKey::Data { addr, .. }) => Some(addr.system.as_str().to_owned()),
                _ => None,
            })
            .collect();
        let toks = zk2rt::client::tokens(&c, "zk2/*/tc/@zk/alive/tc.netif.v1/**", Duration::from_secs(1)).await?.len();
        rows.push(row("system-wildcard", format!("*/tc on {upto} hosts: systems answering a state GET / interface tokens"), format!("{} / {toks}", systems.len()), format!("{upto} / {upto}"), systems.len() == upto && toks == upto));
    }
    drop(hosts);

    // Injection: an over-granted principal puts on a wildcard key.
    let x = Topo::client(&[ep.clone()]).open().await?;
    let before = st.lock().unwrap().discarded_wild;
    for _ in 0..10 {
        x.put("zk2/vehicle-01/*/detections.v1/stream/objects", "injected").await.map_err(|e| anyhow!("{e}"))?;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    let discarded = st.lock().unwrap().discarded_wild - before;
    rows.push(row("injection", "10 puts on zk2/vehicle-01/*/detections.v1/stream/objects: discarded by the R6 filter", discarded, "10", discarded == 10));

    // R3: the graph from descriptors and tokens alone, against the data.
    let o = Topo::client(&[ep.clone()]).open().await?;
    let descs = zk2rt::client::get(&o, "zk2/*/*/@zk/instance/*", zenoh::query::QueryTarget::All, zenoh::query::ConsolidationMode::None, None, Duration::from_secs(3)).await?;
    let mut edges = BTreeSet::new();
    let mut bound = 0;
    for g in descs.into_iter().flatten() {
        let Some(v) = g.value else { continue };
        let d: Value = serde_json::from_slice(&v)?;
        for r in d["requires"].as_array().into_iter().flatten() {
            for b in r["bindings"].as_array().into_iter().flatten() {
                bound += 1;
                let (pat, iface) = (b.as_str().unwrap_or_default(), r["interface"].as_str().unwrap_or_default());
                let toks = zk2rt::client::tokens(&o, &format!("zk2/{pat}/@zk/alive/{iface}/**"), Duration::from_secs(2)).await?;
                for t in toks {
                    if let Ok(ZkKey::Alive { addr, .. }) = parse(&t) {
                        edges.insert((addr.to_string(), d["service"].as_str().unwrap_or_default().to_owned(), r["role"].as_str().unwrap_or_default().to_owned()));
                    }
                }
            }
        }
    }
    let delivering: BTreeSet<String> = {
        let before: BTreeMap<String, u64> = st.lock().unwrap().producers.iter().map(|(n, p)| (n.clone(), p.samples)).collect();
        tokio::time::sleep(Duration::from_secs(1)).await;
        let g = st.lock().unwrap();
        g.producers.iter().filter(|(n, p)| p.left.is_none() && p.samples > before.get(*n).copied().unwrap_or(0)).map(|(n, _)| format!("vehicle-01/{n}")).collect()
    };
    let drawn: BTreeSet<String> = edges.iter().map(|(p, _, _)| p.clone()).collect();
    rows.push(row("graph", "edges drawn from descriptors + interface tokens = producers actually delivering", format!("{} drawn, {} delivering, {} differ", drawn.len(), delivering.len(), drawn.symmetric_difference(&delivering).count()), "0 differ", drawn == delivering));
    rows.push(row("graph", "descriptor bytes with one wildcard binding (requires + bindings)", desc_bytes.len(), "measured", true));
    rows.push(row("graph", "binding records found in all descriptors", bound, "1", bound == 1));

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s10.csv");
    let mut md = format!("# S10 — wildcard bindings (#593)\n\nWritten by `spike s10`; the latest run (`unix_s` {unix}), zenoh {}.\n\n| Phase | Measure | Value | Expected | Pass |\n|---|---|---|---|---|\n", zk2rt::ZENOH_VERSION);
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "phase", "measure", "value", "expected", "pass"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.phase.into(), r.measure.clone(), r.value.clone(), r.expected.clone(), r.pass.to_string()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} |\n", r.phase, r.measure, r.value, r.expected, if r.pass { "yes" } else { "**no**" }));
        println!("{:5} {:16} {} → {}", if r.pass { "ok" } else { "FAIL" }, r.phase, r.measure, r.value);
    }
    std::fs::write(results.join("summary.md"), md)?;
    drop(groups);
    Ok(rows.iter().all(|r| r.pass))
}
