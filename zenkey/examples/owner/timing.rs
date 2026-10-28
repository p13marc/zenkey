//! `timing.v1`'s owner drive (#908, MB): `pace.v1`, the scenarios'
//! contract (`spec/profiles/timing/scenarios.md`), published at its
//! period, each member paused, resumed, added or removed from standard
//! input; its stamps offset, or left unset, for §4 and §5.
//!
//! Every member starts paused: `cmd` is published once `timing resume
//! cmd` says so, an axis once added. Each sample's payload is its sequence
//! number as text; with `timing stamp none`, the owner's wall clock less
//! 10 s, in nanoseconds since the Unix epoch, as text, put raw through the
//! session with no stamp set, as `scenarios.md` §5 has it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zenkey::Service;
use zenkey::model::contract::{Contract, load_str};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::writer::Writer;

/// `pace.v1`, as the runtime's tests keep it.
pub const PACE_TOML: &str = include_str!("../../tests/contracts/pace.v1.toml");

/// `pace.v1`'s `cmd` and its axes.
pub const CMD: &str = "stream/cmd";
pub const AXIS: &str = "stream/axis/{axis}";

/// `pace.v1`, embedded.
pub fn contract() -> Result<Contract, String> {
    let l = load_str(PACE_TOML, std::path::Path::new("."), Some("pace.v1.toml"));
    l.contract
        .ok_or_else(|| format!("pace.v1, embedded:\n{}", l.report))
}

pub fn iface() -> IfaceId {
    "pace.v1".parse().expect("an interface")
}

/// One member published at the period.
struct Member {
    writer: Arc<Writer>,
    key: String,
    on: Arc<AtomicBool>,
    published: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Member {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The drive: every member's publisher and its state.
pub struct Drive {
    session: zenoh::Session,
    period: Duration,
    /// `timing stamp none`: raw puts, no stamp set.
    raw: Arc<AtomicBool>,
    members: Mutex<BTreeMap<String, Member>>,
}

impl Drive {
    /// Starts the drive on `svc`'s `cmd`, paused.
    pub async fn start(svc: &Service, session: &zenoh::Session) -> Result<Drive, String> {
        let cmd = svc
            .writer(&iface(), CMD, &Bindings::new())
            .await
            .map_err(|e| e.to_string())?;
        let period = cmd
            .timing()
            .and_then(|t| t.period)
            .ok_or("pace.v1's cmd declares no period")?;
        let d = Drive {
            session: session.clone(),
            period,
            raw: Arc::default(),
            members: Mutex::default(),
        };
        d.add("cmd", cmd);
        Ok(d)
    }

    fn add(&self, name: &str, writer: Writer) {
        let key = writer.key_expr().as_str().to_owned();
        let writer = Arc::new(writer);
        let on = Arc::new(AtomicBool::new(name != "cmd"));
        let published = Arc::new(AtomicU64::new(0));
        let task = {
            let (w, on, n, raw, session, period, key) = (
                Arc::clone(&writer),
                Arc::clone(&on),
                Arc::clone(&published),
                Arc::clone(&self.raw),
                self.session.clone(),
                self.period,
                key.clone(),
            );
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(period);
                let mut seq = 0u64;
                loop {
                    tick.tick().await;
                    if !on.load(Ordering::Relaxed) {
                        continue;
                    }
                    let put = if raw.load(Ordering::Relaxed) {
                        // §5: no stamp set, a payload's time 10 s old.
                        let stale = SystemTime::now() - Duration::from_secs(10);
                        let ns = stale
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos();
                        session
                            .put(&key, ns.to_string())
                            .await
                            .map_err(|e| e.to_string())
                    } else {
                        w.put(seq.to_string()).await.map_err(|e| e.to_string())
                    };
                    match put {
                        Ok(()) => {
                            seq += 1;
                            n.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => eprintln!("timing: a put on {key} failed: {e}"),
                    }
                }
            })
        };
        self.members.lock().expect("not poisoned").insert(
            name.to_owned(),
            Member {
                writer,
                key,
                on,
                published,
                task,
            },
        );
    }
}

/// Answers one `timing …` line (the owner's `--help`), each answer one
/// JSON line.
pub async fn command(line: &str, svc: &mut Service, d: &Drive) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let refused = |why: String| format!("refused {}: {why}", words.join(" "));
    match words.as_slice() {
        ["timing", verb @ ("pause" | "resume"), member] => {
            let members = d.members.lock().expect("not poisoned");
            let Some(m) = members.get(*member) else {
                return refused(format!("no member {member:?}: cmd, or an axis added"));
            };
            let on = *verb == "resume";
            m.on.store(on, Ordering::Relaxed);
            serde_json::json!({"timing": verb, "member": member, "key": m.key, "on": on})
                .to_string()
        }
        ["timing", "add", axis] => {
            let name = format!("axis/{axis}");
            if d.members.lock().expect("not poisoned").contains_key(&name) {
                return refused(format!("{name} is published already"));
            }
            let values: Bindings = [("axis".to_owned(), vec![(*axis).to_owned()])].into();
            // Core §8.1: a member exists from its owner's first declaration
            // of it, its token before its first sample.
            let epoch = match svc.declare_member(&iface(), axis).await {
                Ok(e) => e,
                Err(e) => return refused(e.to_string()),
            };
            let w = match svc.writer(&iface(), AXIS, &values).await {
                Ok(w) => w,
                Err(e) => return refused(e.to_string()),
            };
            let key = w.key_expr().as_str().to_owned();
            d.add(&name, w);
            serde_json::json!({"timing": "add", "member": name, "key": key, "epoch": epoch.to_string()})
                .to_string()
        }
        ["timing", "remove", axis] => {
            let name = format!("axis/{axis}");
            let Some(m) = d.members.lock().expect("not poisoned").remove(&name) else {
                return refused(format!("no member {name:?}"));
            };
            drop(m);
            if let Err(e) = svc.remove_member(&iface(), axis).await {
                return refused(e.to_string());
            }
            serde_json::json!({"timing": "remove", "member": name}).to_string()
        }
        ["timing", "offset", ms] => match ms.parse::<i64>() {
            Ok(ms) => {
                svc.minter().simulate_offset(ms);
                serde_json::json!({"timing": "offset", "ms": ms}).to_string()
            }
            Err(e) => refused(format!("{ms:?}: {e}")),
        },
        ["timing", "stamp", mode @ ("own" | "none")] => {
            d.raw.store(*mode == "none", Ordering::Relaxed);
            serde_json::json!({"timing": "stamp", "mode": mode}).to_string()
        }
        ["timing", "published"] => {
            let members: serde_json::Map<String, serde_json::Value> = d
                .members
                .lock()
                .expect("not poisoned")
                .iter()
                .map(|(k, m)| (k.clone(), m.published.load(Ordering::Relaxed).into()))
                .collect();
            serde_json::json!({"timing": "published", "members": members}).to_string()
        }
        ["timing", "zid"] => {
            serde_json::json!({"timing": "zid", "zid": d.session.zid().to_string()}).to_string()
        }
        ["timing", "writers"] => {
            let members: serde_json::Map<String, serde_json::Value> = d
                .members
                .lock()
                .expect("not poisoned")
                .iter()
                .map(|(k, m)| (k.clone(), m.writer.is_stamped().into()))
                .collect();
            serde_json::json!({"timing": "writers", "stamped": members}).to_string()
        }
        _ => refused(
            "timing pause|resume <member>, add|remove <axis>, offset <ms>, stamp own|none, \
             published, zid or writers"
                .to_owned(),
        ),
    }
}
