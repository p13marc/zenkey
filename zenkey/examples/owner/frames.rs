//! Frames' owner drive (#981's K3, core §7.7): `linkframes.v1`, the
//! contract of `spec/scenarios/frames.md`, its frame resources written
//! through the runtime's `FrameWriter` from standard input.
//!
//! ```text
//! frame record <resource> <json> [at=<ms>]   one record, §7.6's JSON reading; measured now,
//!                                             or <ms> after the drive's origin on its monotonic clock
//! frame flush <resource>                      puts the open frame now
//! frame lose <resource>                       the next frame is lost on its way: its seqs taken, nothing put
//! frame mutation <resource>                   asks for a plain writer, which the runtime refuses
//! frame new-epoch                             Service::new_epoch: a new instance, a new run (core §4.3)
//! ```
//!
//! A resource is named by its template (`frames/link`). Each answer is one
//! JSON line: `{"frame": "record", "resource", "put": {"run", "seq",
//! "records"} | null, "open"}`, the frame the record completed and put,
//! and how many records the open frame holds after it. `frame flush`
//! answers the same shape. A refusal is `refused <line>: <why>`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use zenkey::Service;
use zenkey::frame::FrameWriter;
use zenkey::model::contract::{Contract, load_path};
use zenkey::model::grammar::IfaceId;
use zenkey::model::wire::Stamp;

/// `linkframes.v1`, as the runtime's tests keep it, and its record types.
pub const LINKFRAMES_TOML: &str = include_str!("../../tests/contracts/frames/linkframes.v1.toml");
pub const LINKFRAMES_JSON: &str = include_str!("../../tests/contracts/frames/linkframes.json");

/// `linkframes.v1`, embedded: written to a directory of its own, so its
/// packed file resolves beside it.
pub fn contract() -> Result<Contract, String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    std::fs::write(root.path().join("linkframes.v1.toml"), LINKFRAMES_TOML)
        .map_err(|e| e.to_string())?;
    std::fs::write(root.path().join("linkframes.json"), LINKFRAMES_JSON)
        .map_err(|e| e.to_string())?;
    let l = load_path(&root.path().join("linkframes.v1.toml"));
    l.contract
        .ok_or_else(|| format!("linkframes.v1, embedded:\n{}", l.report))
}

pub fn iface() -> IfaceId {
    "linkframes.v1".parse().expect("an interface")
}

/// The drive: a frame writer per frame resource, and the origin `at=`
/// counts from.
pub struct Drive {
    writers: BTreeMap<String, FrameWriter>,
    origin: Instant,
}

impl Drive {
    /// A frame writer on each of `svc`'s frame resources of `linkframes.v1`.
    pub async fn start(svc: &Service, contract: &Contract) -> Result<Drive, String> {
        let mut writers = BTreeMap::new();
        for r in &contract.resources {
            let zenkey::model::contract::Body::Data(d) = &r.body else {
                continue;
            };
            if d.frame.is_none() {
                continue;
            }
            let name = zenkey::implementation::resource_name(r);
            let w = svc
                .frame_writer(&iface(), &name)
                .await
                .map_err(|e| e.to_string())?;
            writers.insert(r.template.as_str().to_owned(), w);
        }
        Ok(Drive {
            writers,
            origin: Instant::now(),
        })
    }
}

fn put(stamp: Option<Stamp>, records: Option<u32>) -> serde_json::Value {
    match stamp {
        None => serde_json::Value::Null,
        Some(s) => serde_json::json!({
            "run": s.run.hex(), "seq": s.seq.get(), "records": records,
        }),
    }
}

/// Answers one `frame …` line (the module doc's).
pub async fn command(line: &str, svc: &mut Service, d: &Drive) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let refused = |why: String| format!("refused {}: {why}", words.join(" "));
    let writer = |name: &str| {
        d.writers
            .get(name)
            .ok_or_else(|| format!("no frame resource {name:?}: frames/link or frames/sample"))
    };
    match words.as_slice() {
        ["frame", "record", name, ..] => {
            let w = match writer(name) {
                Ok(w) => w,
                Err(why) => return refused(why),
            };
            let mut rest = line
                .splitn(4, char::is_whitespace)
                .nth(3)
                .unwrap_or("")
                .trim();
            let mut at = Instant::now();
            if let Some((json, tail)) = rest.rsplit_once(" at=") {
                match tail.trim().parse::<u64>() {
                    Ok(ms) => {
                        at = d.origin + Duration::from_millis(ms);
                        rest = json.trim();
                    }
                    Err(_) => return refused(format!("at={tail:?}: whole milliseconds")),
                }
            }
            let value: serde_json::Value = match serde_json::from_str(rest) {
                Ok(v) => v,
                Err(e) => return refused(format!("the record is not JSON: {e}")),
            };
            let before = w.open().await;
            match w.record_json_at(&value, at).await {
                Ok(stamp) => {
                    let open = w.open().await;
                    let records = stamp.map(|_| u32::try_from(before + 1 - open).unwrap_or(0));
                    serde_json::json!({
                        "frame": "record", "resource": name, "put": put(stamp, records), "open": open,
                    })
                    .to_string()
                }
                Err(e) => refused(e.to_string()),
            }
        }
        ["frame", "flush", name] => {
            let w = match writer(name) {
                Ok(w) => w,
                Err(why) => return refused(why),
            };
            let open = u32::try_from(w.open().await).unwrap_or(0);
            match w.flush().await {
                Ok(stamp) => serde_json::json!({
                    "frame": "flush", "resource": name, "put": put(stamp, Some(open)), "open": 0,
                })
                .to_string(),
                Err(e) => refused(e.to_string()),
            }
        }
        ["frame", "lose", name] => match writer(name) {
            Ok(w) => {
                w.simulate_loss();
                serde_json::json!({"frame": "lose", "resource": name}).to_string()
            }
            Err(why) => refused(why),
        },
        ["frame", "mutation", name] => {
            let resource = format!("stream/{name}");
            match svc
                .writer(
                    &iface(),
                    &resource,
                    &zenkey::model::template::Bindings::new(),
                )
                .await
            {
                Ok(_) => refused("the runtime gave a plain writer of a frame resource".to_owned()),
                Err(e) => serde_json::json!({
                    "frame": "mutation", "resource": name, "refused": e.to_string(),
                })
                .to_string(),
            }
        }
        ["frame", "new-epoch"] => match svc.new_epoch().await {
            Ok(id) => serde_json::json!({
                "frame": "new-epoch", "instance": id.to_string(), "run": svc.minter().run().hex(),
            })
            .to_string(),
            Err(e) => refused(e.to_string()),
        },
        _ => refused("frame record|flush|lose|mutation <resource>, or frame new-epoch".to_owned()),
    }
}
