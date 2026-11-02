//! Frames' owner drives: `linkframes.v1` (#981's K3, core §7.7), the
//! contract of `spec/scenarios/frames.md`, and `meterframe.v1` (#983's T3,
//! `telemetry.v2`), the contract of `spec/profiles/telemetry/scenarios-v2.md`,
//! their frame resources written through the runtime's `FrameWriter` from
//! standard input.
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
//! `telemetry frame …` takes the same verbs on `meterframe.v1`, its records
//! metered (`telemetry.v2` §2.19: a counter field's count given unwrapped,
//! refused below its last within the run, written modulo its width), and
//! one more:
//!
//! ```text
//! telemetry frame raw <resource> <json>      a frame of one full record, from the session, unchecked,
//!                                             under the writer's run at its key's next seq
//! ```
//!
//! A resource is named by its template (`frames/link`). Each answer is one
//! JSON line: `{"frame": "record", "resource", "put": {"run", "seq",
//! "records"} | null, "open"}`, the frame the record completed and put,
//! and how many records the open frame holds after it. `frame flush`
//! answers the same shape, and `raw` too, with no `open`. A refusal is
//! `refused <line>: <why>`, a metered one's `<why>` as `telemetry put`'s:
//! `telemetry.v2 decreased field=<f> before=<n> after=<n>: <words>` or
//! `telemetry.v2 code field=<f> code=<n>: <words>`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use zenkey::Service;
use zenkey::frame::FrameWriter;
use zenkey::model::contract::{Contract, load_path};
use zenkey::model::frame::{Frame, Record, Time};
use zenkey::model::grammar::IfaceId;
use zenkey::model::wire::Stamp;
use zenoh::key_expr::OwnedKeyExpr;

/// `linkframes.v1`, as the runtime's tests keep it, and its record types.
pub const LINKFRAMES_TOML: &str = include_str!("../../tests/contracts/frames/linkframes.v1.toml");
pub const LINKFRAMES_JSON: &str = include_str!("../../tests/contracts/frames/linkframes.json");

/// `meterframe.v1`, `telemetry.v2`'s scenarios' contract, as the spec keeps
/// it, with its packed file and the JSON Schema file beside it.
pub const METERFRAME_TOML: &str =
    include_str!("../../../spec/profiles/telemetry/conformance/contracts/meterframe.v1.toml");
pub const METERFRAME_JSON: &str =
    include_str!("../../../spec/profiles/telemetry/conformance/contracts/meterframe.json");
pub const METER_JSON: &str =
    include_str!("../../../spec/profiles/telemetry/conformance/contracts/meter.json");

/// A contract embedded: its files written to a directory of their own, so
/// its schema files resolve beside it.
fn embedded(name: &str, files: &[(&str, &str)]) -> Result<Contract, String> {
    let root = tempfile::tempdir().map_err(|e| e.to_string())?;
    for (file, text) in files {
        std::fs::write(root.path().join(file), text).map_err(|e| e.to_string())?;
    }
    let l = load_path(&root.path().join(files[0].0));
    l.contract
        .ok_or_else(|| format!("{name}, embedded:\n{}", l.report))
}

/// `linkframes.v1`, embedded.
pub fn contract() -> Result<Contract, String> {
    embedded(
        "linkframes.v1",
        &[
            ("linkframes.v1.toml", LINKFRAMES_TOML),
            ("linkframes.json", LINKFRAMES_JSON),
        ],
    )
}

pub fn iface() -> IfaceId {
    "linkframes.v1".parse().expect("an interface")
}

/// `meterframe.v1`, embedded.
pub fn meterframe() -> Result<Contract, String> {
    embedded(
        "meterframe.v1",
        &[
            ("meterframe.v1.toml", METERFRAME_TOML),
            ("meterframe.json", METERFRAME_JSON),
            ("meter.json", METER_JSON),
        ],
    )
}

pub fn meterframe_iface() -> IfaceId {
    "meterframe.v1".parse().expect("an interface")
}

/// The drive: an interface, a frame writer per frame resource of it, and
/// the origin `at=` counts from.
pub struct Drive {
    iface: IfaceId,
    writers: BTreeMap<String, FrameWriter>,
    origin: Instant,
}

impl Drive {
    /// A frame writer on each of `svc`'s frame resources of `contract`.
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
                .frame_writer(&contract.iface, &name)
                .await
                .map_err(|e| e.to_string())?;
            writers.insert(r.template.as_str().to_owned(), w);
        }
        Ok(Drive {
            iface: contract.iface.clone(),
            writers,
            origin: Instant::now(),
        })
    }

    fn writer(&self, name: &str) -> Result<&FrameWriter, String> {
        self.writers.get(name).ok_or_else(|| {
            let names: Vec<&str> = self.writers.keys().map(String::as_str).collect();
            format!("no frame resource {name:?}: {}", names.join(" or "))
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
    answer(line, &words, 1, svc, d, None).await
}

/// Answers one `telemetry frame …` line (the module doc's), `raw` from
/// `session`.
pub async fn telemetry_command(
    line: &str,
    svc: &mut Service,
    d: &Drive,
    session: &zenoh::Session,
) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    answer(line, &words, 2, svc, d, Some(session)).await
}

/// A verb's answer: `words[lead..]` is `frame`'s verb and its arguments,
/// the line's text after them the record. `raw` only with a session.
async fn answer(
    line: &str,
    words: &[&str],
    lead: usize,
    svc: &mut Service,
    d: &Drive,
    session: Option<&zenoh::Session>,
) -> String {
    let refused = |why: String| format!("refused {}: {why}", words.join(" "));
    let refusal = |e: zenkey::Error| refused(super::telemetry_refusal(&e));
    // The record's JSON and its `at=`: the line after the verb and resource.
    let record = || -> Result<(serde_json::Value, Instant), String> {
        let mut rest = line
            .splitn(lead + 3, char::is_whitespace)
            .nth(lead + 2)
            .unwrap_or("")
            .trim();
        let mut at = Instant::now();
        if let Some((json, tail)) = rest.rsplit_once(" at=") {
            let ms = tail
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("at={tail:?}: whole milliseconds"))?;
            at = d.origin + Duration::from_millis(ms);
            rest = json.trim();
        }
        let value =
            serde_json::from_str(rest).map_err(|e| format!("the record is not JSON: {e}"))?;
        Ok((value, at))
    };
    match &words[lead - 1..] {
        ["frame", "record", name, ..] => {
            let w = match d.writer(name) {
                Ok(w) => w,
                Err(why) => return refused(why),
            };
            let (value, at) = match record() {
                Ok(x) => x,
                Err(why) => return refused(why),
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
                Err(e) => refusal(e),
            }
        }
        ["frame", "raw", name, ..] if session.is_some() => {
            let w = match d.writer(name) {
                Ok(w) => w,
                Err(why) => return refused(why),
            };
            let (value, _) = match record() {
                Ok(x) => x,
                Err(why) => return refused(why),
            };
            match raw(w, &value, svc, session.expect("matched")).await {
                Ok(stamp) => serde_json::json!({
                    "frame": "raw", "resource": name, "put": put(Some(stamp), Some(1)),
                })
                .to_string(),
                Err(why) => refused(why),
            }
        }
        ["frame", "flush", name] => {
            let w = match d.writer(name) {
                Ok(w) => w,
                Err(why) => return refused(why),
            };
            let open = u32::try_from(w.open().await).unwrap_or(0);
            match w.flush().await {
                Ok(stamp) => serde_json::json!({
                    "frame": "flush", "resource": name, "put": put(stamp, Some(open)), "open": 0,
                })
                .to_string(),
                Err(e) => refusal(e),
            }
        }
        ["frame", "lose", name] => match d.writer(name) {
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
                    &d.iface,
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
        _ if session.is_some() => refused(
            "telemetry frame record|flush|lose|raw|mutation <resource>, or telemetry frame \
             new-epoch"
                .to_owned(),
        ),
        _ => refused("frame record|flush|lose|mutation <resource>, or frame new-epoch".to_owned()),
    }
}

/// A *raw* frame (`scenarios-v2.md`'s conventions): one full record, its
/// time this host's clock now from the writer's deployment epoch, put from
/// `session` under the writer's run at its key's next seq, which the
/// minter hands out, so the writer's next frame follows it. Nothing of the
/// writer's checks: the record's counts are reduced to their widths and
/// no more.
async fn raw(
    w: &FrameWriter,
    value: &serde_json::Value,
    svc: &Service,
    session: &zenoh::Session,
) -> Result<Stamp, String> {
    let layout = w.layout();
    let slots = layout
        .slots_of_json(value, Some(w.deployment_epoch()))
        .map_err(|r| format!("the record is refused: {}", r.token()))?;
    let frame = Frame {
        time: Time::from_unix_ms(
            i128::from(zenkey::telemetry::now_ms()),
            w.deployment_epoch(),
        ),
        records: vec![Record {
            full: true,
            at: None,
            slots,
        }],
    };
    let key = OwnedKeyExpr::from(w.key_expr().clone().into_owned());
    let stamp = svc.minter().mint_stream_n(&key, 1, false).stamp;
    let payload = frame
        .envelope(layout, stamp)
        .map_err(|r| format!("the frame is refused: {}", r.token()))?;
    session
        .put(w.key_expr().clone(), payload)
        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
        .await
        .map_err(|e| e.to_string())?;
    Ok(stamp)
}
