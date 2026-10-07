//! S6, operations and ownership (#602, r3 §3.7 O1–O7, §3.8).
//!
//! Servers are child processes (`spike s6-server`) that print one line per
//! execution (`exec <request-id>`), so the parent counts executions exactly.
//! The cases: split-brain on an exclusive operation; replicated serving
//! (nearest wins, a crash, a hang); a templated `complete` queryable;
//! fan-in over exact-key queryables; the server-side fan-out refusal (O2);
//! many replies under each consolidation; a partition, and a slow
//! disappearance (the lease); the standby of a claim protocol; and call
//! metadata as a request attachment (O7).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::json;
use zenkey_model::grammar::{ZkKey, parse};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zk2rt::config::Topo;
use zk2rt::metrics::csv_row;

use crate::procs::{self, Proc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ServerMode {
    Ok,
    Hang,
}

/// One operation server, until killed. Prints `exec <request-id>` for every
/// execution and `refused` for every fan-out refusal.
#[allow(clippy::too_many_arguments)]
pub async fn server(
    connect: Vec<String>,
    key: String,
    mode: ServerMode,
    fanout_forbidden: bool,
    replies: u32,
    complete: bool,
    instance_token: Option<String>,
    alive_token: Option<String>,
) -> Result<()> {
    let s = Topo::client(&connect).open().await?;
    let q = s
        .declare_queryable(key)
        .complete(complete)
        .callback(move |q| {
            if fanout_forbidden && q.key_expr().is_wild() {
                // O2: a non-concrete call to a fan-out-forbidden operation.
                let body = json!({"error": "fanout_forbidden", "message": "this operation takes concrete keys only"});
                let _ = q.reply_err(serde_json::to_vec(&body).unwrap()).encoding(Encoding::APPLICATION_JSON).wait();
                println!("refused");
                return;
            }
            let att = q.attachment().map(|a| a.to_bytes().into_owned()).unwrap_or_default();
            let rid = String::from_utf8_lossy(&att).into_owned();
            println!("exec {rid}");
            match mode {
                ServerMode::Ok => {
                    let k = q.key_expr().clone().into_owned();
                    for i in 0..replies {
                        let _ = q.reply(k.clone(), format!("{i}")).attachment(att.clone()).wait();
                    }
                }
                ServerMode::Hang => {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_secs(60));
                        drop(q);
                    });
                }
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let mut toks = Vec::new();
    for t in [instance_token, alive_token].into_iter().flatten() {
        toks.push(s.liveliness().declare_token(t).await.map_err(|e| anyhow!("{e}"))?);
    }
    println!("ready");
    tokio::signal::ctrl_c().await?;
    drop((q, toks));
    Ok(())
}

struct Srv {
    p: Proc,
    name: String,
}

#[allow(clippy::too_many_arguments)]
async fn srv(ep: &str, name: &str, key: &str, mode: ServerMode, fanout_forbidden: bool, replies: u32, tokens: (Option<&str>, Option<&str>)) -> Result<Srv> {
    let mut args = vec!["s6-server".to_owned(), "--connect".into(), ep.to_owned(), "--key".into(), key.to_owned()];
    args.extend(["--mode".into(), format!("{mode:?}").to_lowercase(), "--replies".into(), replies.to_string()]);
    if fanout_forbidden {
        args.push("--fanout-forbidden".into());
    }
    if let Some(t) = tokens.0 {
        args.extend(["--instance-token".into(), t.to_owned()]);
    }
    if let Some(t) = tokens.1 {
        args.extend(["--alive-token".into(), t.to_owned()]);
    }
    Ok(Srv { p: procs::spawn(&args, Duration::from_secs(20)).await?, name: name.to_owned() })
}

/// Every `exec` and `refused` line a server printed so far.
async fn drain(s: &mut Srv) -> (Vec<String>, usize) {
    let (mut execs, mut refused) = (Vec::new(), 0);
    while let Some(l) = s.p.line(Duration::from_millis(30)).await {
        if let Some(r) = l.strip_prefix("exec ") {
            execs.push(r.to_owned());
        } else if l == "refused" {
            refused += 1;
        }
    }
    (execs, refused)
}

/// One call: the replies (Ok payloads) and error replies.
async fn call(c: &zenoh::Session, key: &str, rid: &str, target: QueryTarget, cons: ConsolidationMode, timeout: Duration) -> Result<(usize, Vec<String>, f64)> {
    let t0 = Instant::now();
    let replies = c
        .get(key)
        .target(target)
        .consolidation(cons)
        .attachment(rid.as_bytes().to_vec())
        .timeout(timeout)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let (mut ok, mut errs, mut first) = (0, Vec::new(), None);
    while let Ok(r) = replies.recv_async().await {
        match r.result() {
            Ok(_) => {
                ok += 1;
                first.get_or_insert_with(|| t0.elapsed());
            }
            Err(e) => errs.push(String::from_utf8_lossy(&e.payload().to_bytes()).into_owned()),
        }
    }
    let ms = first.unwrap_or_else(|| t0.elapsed()).as_secs_f64() * 1e3;
    Ok((ok, errs, ms))
}

struct Row {
    group: &'static str,
    case: String,
    value: String,
    pass: bool,
}

/// The descriptor-free split-brain check: more than one alive instance of a
/// service exposing one interface.
async fn split_brain(c: &zenoh::Session) -> Result<Vec<String>> {
    let toks = zk2rt::client::tokens(c, "zk2/*/*/@zk/alive/**", Duration::from_secs(1)).await?;
    let mut by: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for t in toks {
        if let Ok(ZkKey::Alive { addr, iface, instance, .. }) = parse(&t) {
            by.entry((addr.to_string(), iface.to_string())).or_default().insert(instance.as_str().to_owned());
        }
    }
    Ok(by.into_iter().filter(|(_, v)| v.len() > 1).map(|((a, i), v)| format!("{a} {i}: {} instances", v.len())).collect())
}

#[allow(clippy::too_many_lines)]
pub async fn run(results: &Path) -> Result<bool> {
    let mut rows = Vec::new();
    let (_r1, e1) = procs::router(&[]).await?;
    let (_r2, e2) = procs::router(std::slice::from_ref(&e1)).await?;
    let c = Topo::client(&[e1.clone()]).open().await?;
    let t = Duration::from_secs(1);
    let settle = || tokio::time::sleep(Duration::from_millis(400));
    let inst = |a: &str, id: &str| format!("zk2/v/{a}/@zk/instance/{id}");
    let alive = |a: &str, i: &str, id: &str| format!("zk2/v/{a}/@zk/alive/{i}/{id}/0000000000000000");

    // 1. Split-brain: two instances serve one exclusive operation.
    let op = "zk2/v/nav/nav.v2/@op/reset";
    for (label, e_b) in [("both on the client's router", e1.clone()), ("the second behind router 2", e2.clone())] {
        let mut a = srv(&e1, "A", op, ServerMode::Ok, false, 1, (Some(&inst("nav", "00000000000000a1")), Some(&alive("nav", "nav.v2", "00000000000000a1")))).await?;
        let mut b = srv(&e_b, "B", op, ServerMode::Ok, false, 1, (Some(&inst("nav", "00000000000000b1")), Some(&alive("nav", "nav.v2", "00000000000000b1")))).await?;
        settle().await;
        let mut replies = 0;
        for i in 0..200 {
            replies += call(&c, op, &format!("sb-{i}"), QueryTarget::BestMatching, ConsolidationMode::None, t).await?.0;
        }
        let (ea, _) = drain(&mut a).await;
        let (eb, _) = drain(&mut b).await;
        let (sa, sb): (BTreeSet<_>, BTreeSet<_>) = (ea.iter().cloned().collect(), eb.iter().cloned().collect());
        let dups = sa.intersection(&sb).count();
        rows.push(Row {
            group: "split-brain",
            case: format!("200 concrete BestMatching calls, {label}: executions A / B, duplicated calls, replies"),
            value: format!("{} / {} / {dups} / {replies}", ea.len(), eb.len()),
            pass: true,
        });
        let findings = split_brain(&c).await?;
        rows.push(Row {
            group: "split-brain",
            case: format!("{label}: the token check finds"),
            value: if findings.is_empty() { "nothing".into() } else { findings.join("; ") },
            pass: !findings.is_empty(),
        });
        drop((a, b));
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // 2. Replicated serving: three replicas of an idempotent operation.
    let rop = "zk2/v/geo/geo.v1/@op/elevation";
    {
        let mut reps = vec![
            srv(&e1, "r1 (near)", rop, ServerMode::Ok, false, 1, (None, None)).await?,
            srv(&e2, "r2", rop, ServerMode::Ok, false, 1, (None, None)).await?,
            srv(&e2, "r3", rop, ServerMode::Ok, false, 1, (None, None)).await?,
        ];
        settle().await;
        for i in 0..100 {
            call(&c, rop, &format!("rep-{i}"), QueryTarget::BestMatching, ConsolidationMode::None, t).await?;
        }
        let mut counts = Vec::new();
        for r in &mut reps {
            let n = drain(r).await.0.len();
            counts.push(format!("{} {n}", r.name));
        }
        rows.push(Row { group: "replicated", case: "100 calls over 3 replicas: executions per replica".into(), value: counts.join(", "), pass: true });
        // The nearest crashes: calls every 10 ms until one succeeds.
        let t0 = Instant::now();
        let near = reps.remove(0);
        drop(near);
        let (mut failed, mut ok_at) = (0, None);
        for i in 0..300 {
            let (n, _, _) = call(&c, rop, &format!("fo-{i}"), QueryTarget::BestMatching, ConsolidationMode::None, t).await?;
            if n > 0 {
                ok_at = Some(t0.elapsed());
                break;
            }
            failed += 1;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        rows.push(Row {
            group: "replicated",
            case: "the nearest replica crashes (kill -9): failed calls, then the first success at".into(),
            value: format!("{failed} failed; first success {:.1} ms after the kill", ok_at.map_or(f64::NAN, |d| d.as_secs_f64() * 1e3)),
            pass: ok_at.is_some(),
        });
        drop(reps);
    }
    {
        // The nearest replica hangs.
        let _hang = srv(&e1, "hang (near)", rop, ServerMode::Hang, false, 1, (None, None)).await?;
        let _good = srv(&e2, "r2", rop, ServerMode::Ok, false, 1, (None, None)).await?;
        settle().await;
        let mut first = Vec::new();
        let mut answered = 0;
        for i in 0..10 {
            let (n, _, ms) = call(&c, rop, &format!("hang-{i}"), QueryTarget::BestMatching, ConsolidationMode::None, t).await?;
            if n > 0 {
                answered += 1;
                first.push(ms);
            }
        }
        rows.push(Row {
            group: "replicated",
            case: "the nearest replica hangs: calls answered (by the far replica), median ms to the first reply".into(),
            value: format!("{answered}/10, {:.1} ms", if first.is_empty() { f64::NAN } else { first.sort_by(|a, b| a.partial_cmp(b).unwrap()); first[first.len() / 2] }),
            pass: true,
        });
    }

    // 3. A templated complete queryable, and fan-in over exact keys.
    {
        let mut hosts = Vec::new();
        for h in 1..=3 {
            let name = format!("h{h}");
            let tpl = format!("zk2/{name}/tc/tc.netif.v1/@op/interfaces/*/*/set");
            let diag = format!("zk2/{name}/tc/tc.netif.v1/@op/diagnostics");
            hosts.push((srv(&e1, &name, &tpl, ServerMode::Ok, true, 1, (None, None)).await?, srv(&e1, &name, &diag, ServerMode::Ok, false, 1, (None, None)).await?));
        }
        settle().await;
        let (n, errs, _) = call(&c, "zk2/h2/tc/tc.netif.v1/@op/interfaces/default/eth0/set", "tpl-1", QueryTarget::BestMatching, ConsolidationMode::None, t).await?;
        let mut execs = Vec::new();
        for (s, _) in &mut hosts {
            execs.push(drain(s).await.0.len());
        }
        rows.push(Row {
            group: "templated",
            case: "a concrete call to h2's templated `interfaces/*/*/set`: replies, executions h1/h2/h3".into(),
            value: format!("{n} ({} errors); {execs:?}", errs.len()),
            pass: n == 1 && execs == vec![0, 1, 0],
        });
        let (n, _, _) = call(&c, "zk2/*/tc/tc.netif.v1/@op/diagnostics", "fanin-1", QueryTarget::All, ConsolidationMode::None, t).await?;
        let mut dexec = Vec::new();
        for (_, d) in &mut hosts {
            dexec.push(drain(d).await.0.len());
        }
        rows.push(Row {
            group: "fan-in",
            case: "fan-in `zk2/*/tc/tc.netif.v1/@op/diagnostics`, target All, over exact-key complete queryables: replies, executions".into(),
            value: format!("{n}; {dexec:?}"),
            pass: n == 3 && dexec == vec![1, 1, 1],
        });
        // 4. Fan-out refusal (O2): non-concrete calls to a forbidden operation.
        for (label, key) in [
            ("one host, wildcard interface", "zk2/h1/tc/tc.netif.v1/@op/interfaces/*/eth0/set"),
            ("every host", "zk2/*/tc/tc.netif.v1/@op/interfaces/*/*/set"),
        ] {
            let (n, errs, _) = call(&c, key, "fo-x", QueryTarget::All, ConsolidationMode::None, t).await?;
            let mut ex = 0;
            let mut refused = 0;
            for (s, _) in &mut hosts {
                let (e, r) = drain(s).await;
                ex += e.len();
                refused += r;
            }
            let all_ff = !errs.is_empty() && errs.iter().all(|e| e.contains("fanout_forbidden"));
            rows.push(Row {
                group: "fan-out",
                case: format!("a non-concrete call ({label}) to a fanout = forbidden operation: values, refusals, executions"),
                value: format!("{n} values; {} refusals ({refused} at the servers), all `fanout_forbidden`: {all_ff}; {ex} executions", errs.len()),
                pass: n == 0 && ex == 0 && all_ff,
            });
        }
    }

    // 5. Many replies, under each consolidation.
    {
        let mop = "zk2/v/parallax/zs.parallax.v1/@op/catalogue";
        let _a = srv(&e1, "m1", mop, ServerMode::Ok, false, 5, (None, None)).await?;
        let _b = srv(&e2, "m2", mop, ServerMode::Ok, false, 5, (None, None)).await?;
        settle().await;
        let mut vals = Vec::new();
        for (label, cons) in [("None", ConsolidationMode::None), ("Monotonic", ConsolidationMode::Monotonic), ("Latest", ConsolidationMode::Latest), ("Auto", ConsolidationMode::Auto)] {
            let (n, _, _) = call(&c, mop, "many", QueryTarget::All, cons, t).await?;
            vals.push(format!("{label} {n}"));
        }
        let none_ok = vals[0] == "None 10";
        rows.push(Row { group: "many-replies", case: "replies = many (2 servers x 5 replies on one key), target All: replies kept per consolidation".into(), value: vals.join(", "), pass: none_ok });
    }

    // 6. A partition and rejoin; a slow disappearance (the lease).
    {
        let pop = "zk2/v/nav/nav.v2/@op/set_origin";
        let s1 = srv(&e1, "p1", pop, ServerMode::Ok, false, 1, (None, None)).await?;
        settle().await;
        std::process::Command::new("kill").args(["-STOP".to_owned(), s1.p.pid.to_string()]).status()?;
        let t0 = Instant::now();
        let mut outcomes = Vec::new();
        // Calls during the partition: each waits out its timeout until the
        // lease drops the session; then the call fails at once.
        loop {
            let tc = Instant::now();
            let (n, _, _) = call(&c, pop, "part", QueryTarget::BestMatching, ConsolidationMode::None, t).await?;
            outcomes.push((t0.elapsed().as_secs_f64(), n, tc.elapsed().as_secs_f64() * 1e3));
            if t0.elapsed() > Duration::from_secs(14) {
                break;
            }
        }
        // The timeline, for the report: (s since SIGSTOP, replies, call ms).
        let mut tl = String::from("t_s,replies,call_ms\n");
        for o in &outcomes {
            tl.push_str(&format!("{:.3},{},{:.1}\n", o.0, o.1, o.2));
        }
        std::fs::create_dir_all(results)?;
        std::fs::write(results.join("partition-timeline.csv"), tl)?;
        let timeouts = outcomes.iter().filter(|o| o.1 == 0 && o.2 > 900.0).count();
        let fast = outcomes.iter().filter(|o| o.1 == 0 && o.2 < 100.0).count();
        let switch_at = outcomes.iter().find(|o| o.2 < 100.0).map_or(f64::NAN, |o| o.0);
        rows.push(Row {
            group: "partition",
            case: "server frozen (SIGSTOP): calls that waited out the 1 s timeout, calls that failed at once, and when (s) failing fast began".into(),
            value: format!("{timeouts} timeouts, {fast} fast failures; fast from {switch_at:.1} s (the 10 s lease)"),
            pass: timeouts > 0 && fast > 0,
        });
        std::process::Command::new("kill").args(["-CONT".to_owned(), s1.p.pid.to_string()]).status()?;
        let t0 = Instant::now();
        let mut back = None;
        for _ in 0..200 {
            if call(&c, pop, "rejoin", QueryTarget::BestMatching, ConsolidationMode::None, t).await?.0 > 0 {
                back = Some(t0.elapsed());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        rows.push(Row {
            group: "partition",
            case: "after SIGCONT: the first successful call at".into(),
            value: format!("{:.0} ms", back.map_or(f64::NAN, |d| d.as_secs_f64() * 1e3)),
            pass: back.is_some(),
        });
        drop(s1);
    }

    // 7. A claim protocol's standby: an instance token, no interface token,
    // no queryable.
    {
        let sop = "zk2/v/catalog/zs.catalog.v1/@op/claim";
        let mut active = srv(&e1, "active", sop, ServerMode::Ok, false, 1, (Some(&inst("catalog", "00000000000000c1")), Some(&alive("catalog", "zs.catalog.v1", "00000000000000c1")))).await?;
        let st = Topo::client(&[e2.clone()]).open().await?;
        let _standby = st.liveliness().declare_token(inst("catalog", "00000000000000c2")).await.map_err(|e| anyhow!("{e}"))?;
        settle().await;
        for i in 0..20 {
            call(&c, sop, &format!("claim-{i}"), QueryTarget::BestMatching, ConsolidationMode::None, t).await?;
        }
        let ex = drain(&mut active).await.0.len();
        let findings = split_brain(&c).await?;
        let instances = zk2rt::client::tokens(&c, "zk2/v/catalog/@zk/instance/*", t).await?.len();
        rows.push(Row {
            group: "standby",
            case: "active + standby (instance token only): instances seen, split-brain findings, executions of 20 calls by the active".into(),
            value: format!("{instances}; {}; {ex}", if findings.is_empty() { "none".to_owned() } else { findings.join("; ") }),
            pass: instances == 2 && findings.is_empty() && ex == 20,
        });
    }

    // 8. Call metadata (O7): the request attachment reaches the server and
    // comes back on the reply.
    {
        let mop = "zk2/v/modem/modem.v3/@op/action";
        let mut s = srv(&e1, "meta", mop, ServerMode::Ok, false, 1, (None, None)).await?;
        settle().await;
        let meta = r#"{"actor":"operator@ground","request_id":"01j9zk6q6x5m2f4a8c0d3e7b9h"}"#;
        let replies = c.get(mop).attachment(meta.as_bytes().to_vec()).timeout(t).await.map_err(|e| anyhow!("{e}"))?;
        let mut echoed = false;
        while let Ok(r) = replies.recv_async().await {
            if let Ok(smp) = r.result() {
                echoed = smp.attachment().is_some_and(|a| a.to_bytes() == meta.as_bytes());
            }
        }
        let seen = drain(&mut s).await.0;
        rows.push(Row {
            group: "metadata",
            case: "call metadata as a request attachment: seen by the server, echoed on the reply".into(),
            value: format!("{}, {echoed}", seen.first().is_some_and(|x| x == meta)),
            pass: seen.first().is_some_and(|x| x == meta) && echoed,
        });
    }

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s6.csv");
    let mut md = format!("# S6 — operations and ownership (#602)\n\nWritten by `spike s6`; the latest run (`unix_s` {unix}), zenoh {}. Calls time out after 1 s.\n\n| Group | Case | Value | Pass |\n|---|---|---|---|\n", zk2rt::ZENOH_VERSION);
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "group", "case", "value", "pass"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.group.into(), r.case.clone(), r.value.clone(), r.pass.to_string()])?;
        md.push_str(&format!("| {} | {} | {} | {} |\n", r.group, r.case, r.value, if r.pass { "yes" } else { "**no**" }));
        println!("{:5} {:12} {} → {}", if r.pass { "ok" } else { "FAIL" }, r.group, r.case, r.value);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(rows.iter().all(|r| r.pass))
}
