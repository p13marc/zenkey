//! `arbitration.v1`'s drives (#909, BB): an actuator (`--arbitrate`) and a
//! commander (`--commander`), on the scenarios' contracts
//! (`spec/profiles/arbitration/scenarios.md`), embedded as
//! `zenkey/tests/contracts/` keeps them: `rudder.v1`, the actuator's, and
//! `steer.v1`, the commanders'.
//!
//! **The actuator** implements `rudder.v1`, its role `cmd` bound to the
//! entries given, in their order, and arbitrated over the revision of
//! `steer.v1` it holds: the embedded one, or a `steer.v1` contract file
//! given. It prints each switch as the scenarios' conventions write it,
//! one JSON line, from its start's on, and puts every command it delivers
//! on `angle`, its payload unchanged.
//!
//! **The commander** implements `steer.v1` (or the file given), and puts
//! `cmd` at its period, `<service> <n>` with `n` from 0, from a lead after
//! its tokens (500 ms by default, text 0.2, F-269), while it is not held.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey::Service;
use zenkey::arbitration::{Arbitration, ArbitrationEvent, Switched};
use zenkey::model::contract::{Contract, load_str};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::state::StateWriter;

/// The scenarios' contracts, as the runtime's tests keep them.
pub const RUDDER_TOML: &str = include_str!("../../tests/contracts/rudder.v1.toml");
pub const STEER_TOML: &str = include_str!("../../tests/contracts/steer.v1.toml");

/// `rudder.v1`'s arbitrated role, and `steer.v1`'s stream.
pub const ROLE: &str = "cmd";
pub const CMD: &str = "stream/cmd";

fn load(text: &str, name: &str) -> Result<Contract, String> {
    let l = load_str(text, std::path::Path::new("."), Some(name));
    l.contract
        .ok_or_else(|| format!("{name}, embedded:\n{}", l.report))
}

/// `rudder.v1`, embedded.
pub fn rudder() -> Result<Contract, String> {
    load(RUDDER_TOML, "rudder.v1.toml")
}

/// `steer.v1`, embedded.
pub fn steer() -> Result<Contract, String> {
    load(STEER_TOML, "steer.v1.toml")
}

pub fn rudder_iface() -> IfaceId {
    "rudder.v1".parse().expect("an interface")
}

pub fn steer_iface() -> IfaceId {
    "steer.v1".parse().expect("an interface")
}

/// Milliseconds from `origin` to `at`, a float exact to the nanosecond.
fn ms(origin: Instant, at: Instant) -> f64 {
    at.saturating_duration_since(origin).as_nanos() as f64 / 1e6
}

/// A switch as the scenarios' conventions print it.
#[must_use]
pub fn switch_line(s: &Switched, origin: Instant) -> String {
    serde_json::json!({
        "switch": {
            "from": s.from.as_ref().map(ToString::to_string),
            "to": s.to.as_ref().map(ToString::to_string),
            "cause": s.cause.as_str(),
        },
        "at_ms": ms(origin, s.at),
        "last_ms": s.from.as_ref().and(s.last).map(|l| ms(origin, l)),
    })
    .to_string()
}

/// The actuator: its arbitration.
pub struct Actuator {
    arb: Arbitration,
}

impl Actuator {
    /// Arbitrates `svc`'s role `cmd` over `steer`, its withdrawals heard on
    /// `watch` when given; every switch printed through `out`, every
    /// delivery put on `angle`.
    pub async fn start(
        svc: &Service,
        steer: Arc<Contract>,
        watch: Option<zenoh::Session>,
        angle: Option<Arc<StateWriter>>,
        out: flume::Sender<String>,
    ) -> Result<Self, String> {
        let mut c = svc.consumer(ROLE, steer).map_err(|e| e.to_string())?;
        if let Some(w) = watch {
            c = c.with_watch(w);
        }
        // The origin is the arbitration's start, its `start` switch's
        // instant: the first event it hands over.
        let origin: Arc<std::sync::OnceLock<Instant>> = Arc::default();
        let o = Arc::clone(&origin);
        // The puts on `angle`, in delivery order: one task takes them from a
        // channel, so the last value put is the last command delivered.
        let puts = angle.map(|w| {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<zenoh::bytes::ZBytes>();
            tokio::spawn(async move {
                let mut rx = rx;
                while let Some(payload) = rx.recv().await {
                    if let Err(e) = w.put(payload).await {
                        eprintln!("arbitration: a put on angle failed: {e}");
                    }
                }
            });
            tx
        });
        let arb = c
            .arbitrate(move |ev| match ev {
                ArbitrationEvent::Switched(s) => {
                    let origin = *o.get_or_init(|| s.at);
                    let _ = out.send(switch_line(&s, origin));
                }
                ArbitrationEvent::Delivered { delivery, .. } => {
                    if let Some(tx) = &puts {
                        let _ = tx.send(delivery.sample.payload().clone());
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())?;
        Ok(Self { arb })
    }

    fn counts(&self) -> serde_json::Value {
        let providers: serde_json::Map<String, serde_json::Value> = self
            .arb
            .all_counts()
            .into_iter()
            .map(|(a, c)| {
                (
                    a.to_string(),
                    serde_json::json!({
                        "delivered": c.delivered,
                        "not_active": c.not_active,
                        "deleted": c.deleted,
                        "expired": c.expired,
                        "not_incumbent": c.not_incumbent,
                    }),
                )
            })
            .collect();
        let d = self.arb.discards();
        serde_json::json!({"counts": {
            "providers": providers,
            "wild": d.wild,
            "unresolved": d.unresolved,
            "unbound": d.unbound,
        }})
    }
}

/// Answers one `arbitrate …` line (the owner's `--help`), one JSON line.
pub fn command(line: &str, a: &Actuator) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.as_slice() {
        ["arbitrate", "counts"] => a.counts().to_string(),
        ["arbitrate", "state"] => {
            let withdrawn: Vec<String> = a
                .arb
                .providers()
                .into_iter()
                .filter(|p| a.arb.withdrawn(p))
                .map(|p| p.to_string())
                .collect();
            serde_json::json!({
                "arbitrate": "state",
                "active": a.arb.active().map(|p| p.to_string()),
                "providers": a.arb.providers().iter().map(ToString::to_string).collect::<Vec<_>>(),
                "withdrawn": withdrawn,
                "moving": a.arb.moving(),
                "watching": a.arb.watching(),
                "deadline_ms": a.arb.deadline().as_millis() as u64,
            })
            .to_string()
        }
        ["arbitrate", "reads"] => serde_json::json!({
            "arbitrate": "reads",
            "reads": a.arb.reads(),
            "not_made": a.arb.reads_not_made(),
            "round_trips": a.arb.round_trips(),
        })
        .to_string(),
        _ => format!(
            "refused {}: arbitrate counts, state or reads",
            words.join(" ")
        ),
    }
}

/// The commander: its putting loop.
pub struct Commander {
    session: zenoh::Session,
    on: Arc<AtomicBool>,
    raw: Arc<AtomicBool>,
    sent: Arc<AtomicU64>,
    refused: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Commander {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Commander {
    /// Puts `svc`'s `steer.v1` `cmd` at its period from `lead` on.
    pub async fn start(
        svc: &Service,
        session: &zenoh::Session,
        lead: Duration,
    ) -> Result<Self, String> {
        let w = svc
            .writer(&steer_iface(), CMD, &Bindings::new())
            .await
            .map_err(|e| e.to_string())?;
        let period = w
            .timing()
            .and_then(|t| t.period)
            .ok_or("steer.v1's cmd declares no period")?;
        let key = w.key_expr().as_str().to_owned();
        let name = svc.address().service.to_string();
        let (on, raw) = (
            Arc::new(AtomicBool::new(true)),
            Arc::new(AtomicBool::new(false)),
        );
        let (sent, refused) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let task = {
            let (on, raw, sent, refused, session) = (
                Arc::clone(&on),
                Arc::clone(&raw),
                Arc::clone(&sent),
                Arc::clone(&refused),
                session.clone(),
            );
            tokio::spawn(async move {
                tokio::time::sleep(lead).await;
                let mut tick = tokio::time::interval(period);
                let mut n = 0u64;
                loop {
                    tick.tick().await;
                    if !on.load(Ordering::Relaxed) {
                        continue;
                    }
                    let put = if raw.load(Ordering::Relaxed) {
                        // Round the writer: a standby's raw samples
                        // (scenarios §11 step 1), `standby <n>`, stamped by
                        // the session's HLC (text 0.3, F-276).
                        session
                            .put(&key, format!("standby {n}"))
                            .await
                            .map_err(|e| e.to_string())
                    } else {
                        w.put(format!("{name} {n}"))
                            .await
                            .map_err(|e| e.to_string())
                    };
                    match put {
                        Ok(()) => {
                            n += 1;
                            sent.fetch_add(1, Ordering::Relaxed);
                        }
                        // A standby's writer refuses (redundancy.v1 §2.4).
                        Err(_) => {
                            refused.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
        };
        Ok(Self {
            session: session.clone(),
            on,
            raw,
            sent,
            refused,
            task,
        })
    }
}

/// Answers one `commander …` line (the owner's `--help`), one JSON line.
pub async fn commander_command(line: &str, svc: &mut Service, c: &Commander) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let refused = |why: String| format!("refused {}: {why}", words.join(" "));
    match words.as_slice() {
        ["commander", verb @ ("hold" | "resume")] => {
            c.on.store(*verb == "resume", Ordering::Relaxed);
            serde_json::json!({"commander": verb, "on": *verb == "resume"}).to_string()
        }
        ["commander", "raw", mode @ ("on" | "off")] => {
            c.raw.store(*mode == "on", Ordering::Relaxed);
            serde_json::json!({"commander": "raw", "raw": *mode == "on"}).to_string()
        }
        ["commander", "sent"] => serde_json::json!({
            "commander": "sent",
            "sent": c.sent.load(Ordering::Relaxed),
            "refused": c.refused.load(Ordering::Relaxed),
        })
        .to_string(),
        ["commander", "new-epoch"] => match svc.new_epoch().await {
            Ok(i) => {
                serde_json::json!({"commander": "new-epoch", "instance": i.to_string()}).to_string()
            }
            Err(e) => refused(e.to_string()),
        },
        ["commander", "zid"] => {
            serde_json::json!({"commander": "zid", "zid": c.session.zid().to_string()}).to_string()
        }
        _ => refused("commander hold|resume, raw on|off, sent, new-epoch or zid".to_owned()),
    }
}
