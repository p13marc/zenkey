//! S11, control arbitration (#594, r3 §7, walkthrough §4.2).
//!
//! Two actuators (`thruster-l`, `thruster-r`) bind `twist_cmd.v1` as `cmd`
//! to `[safety, teleop, autopilot]`, in priority order. The deadline and the
//! lifespan come from the contract's `timing.v1` annotations (100 ms each).
//! The arbiter picks the highest-priority commander whose last command
//! arrived within the deadline, measured on the actuator's own monotonic
//! clock; none gives the dead-man stop. A liveliness delete drops a
//! commander at once. Commanders are child processes (`spike s11-cmd`)
//! publishing at the contract's 50 Hz with its QoS, so they can be stopped
//! (SIGSTOP: silent but alive) or killed.
//!
//! The same scenario runs twice: **P3**, each commander on its own key and
//! the actuators bound to them; and **P2**, the fallback, where commanders
//! put to each actuator's `@in` key and claim their name in the attachment.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use prost::Message as _;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::load_path;
use zenkey_model::grammar::{Addr, InstanceId, alive_key};
use zenoh::qos::{CongestionControl, Priority, Reliability};
use zenoh::sample::SampleKind;
use zk2rt::config::Topo;
use zk2rt::metrics::{csv_row, proc_sample};

use crate::procs::{self, Proc};
use crate::s9::Twist;

const SYSTEM: &str = "vehicle-01";
const ORDER: [&str; 3] = ["safety", "teleop", "autopilot"];
const ACTUATORS: [&str; 2] = ["thruster-l", "thruster-r"];

fn unix_ns() -> i128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i128).unwrap_or(0)
}

/// The commander child: publishes twist commands at `hz` until killed.
pub async fn commander(connect: Vec<String>, name: String, p2: bool, hz: f64, skew_ms: i64, contract: &Path) -> Result<()> {
    let l = load_path(contract);
    let c = l.contract.ok_or_else(|| anyhow!("{}", l.report))?;
    let s = Topo::client(&connect).open().await?;
    let keys: Vec<String> = if p2 {
        ACTUATORS.iter().map(|a| format!("zk2/{SYSTEM}/{a}/twist_cmd.v1/@in/cmd")).collect()
    } else {
        vec![format!("zk2/{SYSTEM}/{name}/twist_cmd.v1/stream/cmd")]
    };
    let mut pubs = Vec::new();
    for k in &keys {
        // The contract's QoS for `cmd`: best_effort, drop, real_time, express.
        pubs.push(
            s.declare_publisher(k.clone())
                .reliability(Reliability::BestEffort)
                .congestion_control(CongestionControl::Drop)
                .priority(Priority::RealTime)
                .express(true)
                .await
                .map_err(|e| anyhow!("{e}"))?,
        );
    }
    let addr = Addr::new(SYSTEM, &name)?;
    let inst = InstanceId::from_u64(u64::from(std::process::id()));
    let tok = alive_key(&addr, &c.iface, &inst, &Fingerprint::of(&c).hex().fp16())?;
    let _t = s.liveliness().declare_token(tok.as_str().to_owned()).await.map_err(|e| anyhow!("{e}"))?;
    println!("ready");
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / hz));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut seq = 0u64;
    loop {
        tick.tick().await;
        let stamp = (unix_ns() + i128::from(skew_ms) * 1_000_000) as u64;
        let mut att = seq.to_le_bytes().to_vec();
        att.extend_from_slice(&stamp.to_le_bytes());
        att.extend_from_slice(name.as_bytes());
        let body = Twist { stamp_ns: stamp, seq, ..Default::default() }.encode_to_vec();
        for p in &pubs {
            p.put(body.clone()).attachment(att.clone()).await.map_err(|e| anyhow!("{e}"))?;
        }
        seq += 1;
    }
}

#[derive(Debug, Clone)]
struct Switch {
    at: Instant,
    at_unix: i128,
    to: Option<String>,
    cause: &'static str,
    /// The previous active commander's last receive time, at the switch.
    prev_last_rx: Option<Instant>,
}

struct Arbiter {
    deadline: Duration,
    lifespan_ns: i128,
    last_rx: BTreeMap<String, Instant>,
    first_stamp: BTreeMap<String, i128>,
    active: Option<String>,
    log: Vec<Switch>,
    samples: u64,
    /// Samples a sender-clock lifespan check would have rejected.
    lifespan_rejects: BTreeMap<String, u64>,
    received: BTreeMap<String, u64>,
    false_stops: u64,
}

impl Arbiter {
    fn new(deadline: Duration, lifespan: Duration) -> Self {
        Self {
            deadline,
            lifespan_ns: lifespan.as_nanos() as i128,
            last_rx: BTreeMap::new(),
            first_stamp: BTreeMap::new(),
            active: None,
            log: Vec::new(),
            samples: 0,
            lifespan_rejects: BTreeMap::new(),
            received: BTreeMap::new(),
            false_stops: 0,
        }
    }

    fn evaluate(&mut self, now: Instant, cause: &'static str) {
        let best = ORDER
            .iter()
            .find(|c| self.last_rx.get(**c).is_some_and(|t| now.saturating_duration_since(*t) <= self.deadline))
            .map(|c| (*c).to_owned());
        if best != self.active {
            if best.is_none() && self.last_rx.values().any(|t| now.saturating_duration_since(*t) <= self.deadline) {
                self.false_stops += 1;
            }
            let prev_last_rx = self.active.as_ref().and_then(|a| self.last_rx.get(a).copied());
            self.log.push(Switch { at: now, at_unix: unix_ns(), to: best.clone(), cause, prev_last_rx });
            self.active = best;
        }
    }

    fn on_sample(&mut self, from: &str, stamp: i128) {
        let now = Instant::now();
        self.samples += 1;
        *self.received.entry(from.to_owned()).or_default() += 1;
        if unix_ns() - stamp > self.lifespan_ns || stamp - unix_ns() > self.lifespan_ns {
            *self.lifespan_rejects.entry(from.to_owned()).or_default() += 1;
        }
        self.first_stamp.entry(from.to_owned()).or_insert(stamp);
        self.last_rx.insert(from.to_owned(), now);
        self.evaluate(now, "sample");
    }

    fn on_gone(&mut self, from: &str) {
        self.last_rx.remove(from);
        self.first_stamp.remove(from);
        self.evaluate(Instant::now(), "liveliness");
    }
}

type Shared = Arc<Mutex<Arbiter>>;

struct Actuator {
    arb: Shared,
    _session: zenoh::Session,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
    _live: zenoh::pubsub::Subscriber<()>,
    _tick: tokio::task::JoinHandle<()>,
}

async fn actuator(ep: &str, name: &str, p2: bool, deadline: Duration, lifespan: Duration) -> Result<Actuator> {
    let s = Topo::client(&[ep.to_owned()]).open().await?;
    let arb: Shared = Arc::new(Mutex::new(Arbiter::new(deadline, lifespan)));
    let keys: Vec<String> = if p2 {
        vec![format!("zk2/{SYSTEM}/{name}/twist_cmd.v1/@in/cmd")]
    } else {
        ORDER.iter().map(|c| format!("zk2/{SYSTEM}/{c}/twist_cmd.v1/stream/cmd")).collect()
    };
    let mut subs = Vec::new();
    for k in keys {
        let a = arb.clone();
        subs.push(
            s.declare_subscriber(k)
                .callback(move |smp| {
                    if smp.key_expr().is_wild() {
                        return;
                    }
                    let Some(att) = smp.attachment().map(|a| a.to_bytes().into_owned()) else { return };
                    if att.len() < 16 {
                        return;
                    }
                    let stamp = i128::from(u64::from_le_bytes(att[8..16].try_into().unwrap()));
                    // P3: the key says who sent it. P2: the sender claims it.
                    let from = if p2 {
                        String::from_utf8_lossy(&att[16..]).into_owned()
                    } else {
                        smp.key_expr().as_str().split('/').nth(2).unwrap_or_default().to_owned()
                    };
                    let _ = Twist::decode(&*smp.payload().to_bytes());
                    a.lock().unwrap().on_sample(&from, stamp);
                })
                .await
                .map_err(|e| anyhow!("{e}"))?,
        );
    }
    let a = arb.clone();
    let live = s
        .liveliness()
        .declare_subscriber(format!("zk2/{SYSTEM}/*/@zk/alive/twist_cmd.v1/**"))
        .callback(move |smp| {
            if smp.kind() == SampleKind::Delete {
                let who = smp.key_expr().as_str().split('/').nth(2).unwrap_or_default().to_owned();
                a.lock().unwrap().on_gone(&who);
            }
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let a = arb.clone();
    let tick = tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_millis(2));
        loop {
            t.tick().await;
            a.lock().unwrap().evaluate(Instant::now(), "deadline");
        }
    });
    Ok(Actuator { arb, _session: s, _subs: subs, _live: live, _tick: tick })
}

struct Row {
    mode: &'static str,
    phase: String,
    value: String,
    pass: bool,
}

async fn wait_active(acts: &[Actuator], want: Option<&str>, limit: Duration) -> bool {
    let t0 = Instant::now();
    loop {
        if acts.iter().all(|a| a.arb.lock().unwrap().active.as_deref() == want) {
            return true;
        }
        if t0.elapsed() > limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// The last switch of each actuator to `want`, after `since`.
fn switches(acts: &[Actuator], want: Option<&str>, since: Instant) -> Vec<Switch> {
    acts.iter()
        .filter_map(|a| a.arb.lock().unwrap().log.iter().rev().find(|s| s.at >= since && s.to.as_deref() == want).cloned())
        .collect()
}

fn signal(p: &Proc, sig: &str) -> Result<()> {
    std::process::Command::new("kill").args([format!("-{sig}"), p.pid.to_string()]).status()?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn scenario(ep: &str, p2: bool, contract: &Path, deadline: Duration, lifespan: Duration, rows: &mut Vec<Row>) -> Result<()> {
    let mode: &'static str = if p2 { "P2" } else { "P3" };
    let mut acts = Vec::new();
    for a in ACTUATORS {
        acts.push(actuator(ep, a, p2, deadline, lifespan).await?);
    }
    let cmd = |name: &str, skew: i64| {
        let mut v = vec!["s11-cmd".to_owned(), "--connect".into(), ep.to_owned(), "--name".into(), name.to_owned()];
        v.extend(["--skew-ms".into(), skew.to_string(), "--contract".into(), contract.display().to_string()]);
        if p2 {
            v.push("--p2".into());
        }
        v
    };
    let push = |rows: &mut Vec<Row>, phase: String, value: String, pass: bool| rows.push(Row { mode, phase, value, pass });
    let ms = |d: Duration| d.as_secs_f64() * 1e3;

    // 1. Autopilot alone.
    let autopilot = procs::spawn(&cmd("autopilot", 0), Duration::from_secs(20)).await?;
    let ok = wait_active(&acts, Some("autopilot"), Duration::from_secs(5)).await;
    push(rows, "autopilot alone: both actuators follow it".into(), ok.to_string(), ok);

    // 2. Teleop takes over: from teleop's first send to each switch.
    let t = Instant::now();
    let mut teleop = procs::spawn(&cmd("teleop", 0), Duration::from_secs(20)).await?;
    let ok = wait_active(&acts, Some("teleop"), Duration::from_secs(5)).await;
    let sw = switches(&acts, Some("teleop"), t);
    let lat: Vec<String> = acts
        .iter()
        .zip(&sw)
        .map(|(a, s)| {
            let first = a.arb.lock().unwrap().first_stamp.get("teleop").copied().unwrap_or(0);
            format!("{:.2}", (s.at_unix - first) as f64 / 1e6)
        })
        .collect();
    let spread = if sw.len() == 2 { ms(if sw[0].at > sw[1].at { sw[0].at - sw[1].at } else { sw[1].at - sw[0].at }) } else { f64::NAN };
    push(rows, "teleop takes over: first teleop send to switch, ms (l, r)".into(), lat.join(", "), ok);
    push(rows, "teleop takes over: the two actuators switch within, ms".into(), format!("{spread:.2}"), ok);

    // 3. Teleop falls silent (SIGSTOP: alive, but no commands).
    let t = Instant::now();
    signal(&teleop, "STOP")?;
    let ok = wait_active(&acts, Some("autopilot"), Duration::from_secs(2)).await;
    let sw = switches(&acts, Some("autopilot"), t);
    push(rows, "teleop silent: last teleop command to autopilot resuming, ms (l, r)".into(), sw.iter().map(|s| format!("{:.1}", s.prev_last_rx.map_or(f64::NAN, |l| ms(s.at - l)))).collect::<Vec<_>>().join(", "), ok);
    push(rows, "teleop silent: detected by".into(), sw.iter().map(|s| s.cause).collect::<Vec<_>>().join(", "), sw.iter().all(|s| s.cause == "deadline"));

    // 4. Teleop back.
    signal(&teleop, "CONT")?;
    let ok = wait_active(&acts, Some("teleop"), Duration::from_secs(2)).await;
    push(rows, "teleop resumes: both actuators back on teleop".into(), ok.to_string(), ok);

    // 5. Teleop crashes (kill -9).
    let t = Instant::now();
    signal(&teleop, "KILL")?;
    let ok = wait_active(&acts, Some("autopilot"), Duration::from_secs(2)).await;
    let sw = switches(&acts, Some("autopilot"), t);
    push(rows, "teleop crash (kill -9): signal to autopilot, ms (l, r)".into(), sw.iter().map(|s| format!("{:.1}", ms(s.at - t))).collect::<Vec<_>>().join(", "), ok);
    push(rows, "teleop crash: detected by".into(), sw.iter().map(|s| s.cause).collect::<Vec<_>>().join(", "), ok);
    teleop = procs::spawn(&cmd("teleop", 0), Duration::from_secs(20)).await?;
    let _ = wait_active(&acts, Some("teleop"), Duration::from_secs(2)).await;
    signal(&teleop, "STOP")?;
    let _ = wait_active(&acts, Some("autopilot"), Duration::from_secs(2)).await;

    // 6. Every commander silent: the dead-man stop.
    let t = Instant::now();
    signal(&autopilot, "STOP")?;
    let ok = wait_active(&acts, None, Duration::from_secs(2)).await;
    let sw = switches(&acts, None, t);
    push(rows, "all silent: last command to dead-man stop, ms (l, r)".into(), sw.iter().map(|s| format!("{:.1}", s.prev_last_rx.map_or(f64::NAN, |l| ms(s.at - l)))).collect::<Vec<_>>().join(", "), ok);

    // 7. Safety preempts.
    signal(&autopilot, "CONT")?;
    let _ = wait_active(&acts, Some("autopilot"), Duration::from_secs(2)).await;
    let t = Instant::now();
    let safety = procs::spawn(&cmd("safety", 0), Duration::from_secs(20)).await?;
    let ok = wait_active(&acts, Some("safety"), Duration::from_secs(5)).await;
    push(rows, "safety preempts autopilot: spawn to switch, ms".into(), format!("{:.1}", ms(t.elapsed())), ok);

    // Steady state with three commanders: CPU in the actuator process.
    signal(&teleop, "CONT")?;
    let pid = std::process::id();
    let c0 = proc_sample(pid)?.cpu_ticks;
    let n0: u64 = acts.iter().map(|a| a.arb.lock().unwrap().samples).sum();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let c1 = proc_sample(pid)?.cpu_ticks;
    let n1: u64 = acts.iter().map(|a| a.arb.lock().unwrap().samples).sum();
    let pct = (c1 - c0) as f64 * 10.0 / 3000.0 * 100.0;
    push(rows, format!("steady, 3 commanders x 50 Hz x 2 actuators: {} samples in 3 s; CPU of the process holding both actuators (2 ms evaluation tick each), % of one core", n1 - n0), format!("{pct:.1}"), n1 - n0 > 800);
    let stops: u64 = acts.iter().map(|a| a.arb.lock().unwrap().false_stops).sum();
    push(rows, "false stops over the whole scenario".into(), stops.to_string(), stops == 0);
    drop((autopilot, teleop, safety));
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 8. Clock skew: a sender-clock lifespan check, against the
    // receive-clock deadline the arbiter uses.
    if !p2 {
        for (who, skew) in [("autopilot", -250i64), ("teleop", 250)] {
            let (rx0, rej0) = {
                let a = acts[0].arb.lock().unwrap();
                (a.received.get(who).copied().unwrap_or(0), a.lifespan_rejects.get(who).copied().unwrap_or(0))
            };
            let p = procs::spawn(&cmd(who, skew), Duration::from_secs(20)).await?;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let a = acts[0].arb.lock().unwrap();
            let rx = a.received.get(who).copied().unwrap_or(0) - rx0;
            let rej = a.lifespan_rejects.get(who).copied().unwrap_or(0) - rej0;
            drop(a);
            let followed = acts.iter().all(|a| a.arb.lock().unwrap().active.as_deref() == Some(who));
            push(rows, format!("skew {skew:+} ms on {who}: a 100 ms sender-clock lifespan check rejects (of the skewed samples)"), format!("{rej}/{rx}"), rej == rx && rx > 0);
            push(rows, format!("skew {skew:+} ms on {who}: the receive-clock deadline still follows it"), followed.to_string(), followed);
            drop(p);
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    Ok(())
}

pub async fn run(results: &Path, examples: &Path) -> Result<bool> {
    let contract = examples.join("walkthrough/twist_cmd.v1.toml");
    let l = load_path(&contract);
    let c = l.contract.ok_or_else(|| anyhow!("{}", l.report))?;
    let cmd = c.resources.iter().find(|r| r.template.as_str() == "cmd").ok_or_else(|| anyhow!("no cmd"))?;
    let ann = |k: &str| cmd.annotations.get(k).and_then(serde_json::Value::as_u64).ok_or_else(|| anyhow!("no {k}"));
    let deadline = Duration::from_millis(ann("timing.deadline_ms")?);
    let lifespan = Duration::from_millis(ann("timing.lifespan_ms")?);
    let (_router, ep) = procs::router(&[]).await?;
    let mut rows = Vec::new();
    scenario(&ep, false, &contract, deadline, lifespan, &mut rows).await?;
    scenario(&ep, true, &contract, deadline, lifespan, &mut rows).await?;
    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s11.csv");
    let mut md = format!(
        "# S11 — control arbitration (#594)\n\nWritten by `spike s11`; the latest run (`unix_s` {unix}), zenoh {}. Deadline {} ms and lifespan {} ms from `twist_cmd.v1`'s `timing.v1` annotations.\n\n| Mode | Phase | Value | Pass |\n|---|---|---|---|\n",
        zk2rt::ZENOH_VERSION,
        deadline.as_millis(),
        lifespan.as_millis()
    );
    for r in &rows {
        csv_row(&csv, &["unix_s", "zenoh", "mode", "phase", "value", "pass"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.mode.into(), r.phase.clone(), r.value.clone(), r.pass.to_string()])?;
        md.push_str(&format!("| {} | {} | {} | {} |\n", r.mode, r.phase, r.value, if r.pass { "yes" } else { "**no**" }));
        println!("{:5} {} {} → {}", if r.pass { "ok" } else { "FAIL" }, r.mode, r.phase, r.value);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(rows.iter().all(|r| r.pass))
}
