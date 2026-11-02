//! zenoh-modem's SX1262 under `config.v2` (#985 G4): the far-side reach
//! change of `examples/zk2/zenoh-modem/config.v2-binding.md` §15, run in
//! process on [`zenkey::config_v2`]'s owner, with the binding's bytes.
//!
//! The shape is `spec/scenarios/constrained.md` §1's: a vehicle router, the
//! owner `h-c2a95e41d7f0/lora0` its client, and one ground session attached
//! as a client of it, through a [`common::Link`] that counts what crosses
//! toward the ground. The router is the face: users by name and password,
//! the owner granted everything, the ground only what the binding's §12
//! grants (its subscriptions, the record's and the heartbeat's puts, and
//! its calls to `set/duty`, `set/radio`, `confirm`, `cancel` and `extend`,
//! whose queryables are declared toward it),
//! so `@zk` and the change event stay on the vehicle's side. The ground
//! binds statically and writes its requests' bytes by hand, as a far side
//! does: it reads no presence and no descriptor.
//!
//! The owner implements `examples/zk2/zenoh-modem/config/sx1262_config.v1`
//! under zenoh-modem's catalog (its epoch 2026-01-01), writing under the
//! 16-bit run 0x5A17 from a run store, its token count from 0x9C41. Its
//! startup values are the file's, its floor 15 s, its simulated device
//! states `apply_at` 2 s after a reach `set`, and its clock is moved to the
//! binding's instants ([`Configuration::simulate_offset`]), each step at
//! least a second inside the one the bytes count.

mod common;

#[allow(dead_code)]
#[path = "../examples/owner/config.rs"]
mod sim;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{SETTLE, eventually};
use serde_json::{Value as Json, json};
use sim::SimDevice;
use zenkey::config_v2::{self, Configuration, Interface, Startup, State, Values};
use zenkey::model::catalog::Catalog;
use zenkey::model::config::Value;
use zenkey::model::config::v2::{ChangeEvent, ParamState};
use zenkey::model::contract::{Contract, load_path};
use zenkey::model::envelope::Code;
use zenkey::model::wire::{Envelope, Request, Tag, read_reply, read_reply_err};
use zenkey::{CallMetadata, Implementation, ServiceBuilder};
use zenoh::bytes::Encoding;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};

/// lora0, device-as-service on its made-up host (the binding's §2).
const LORA0: &str = "h-c2a95e41d7f0/lora0";

/// The config interface's id.
const IFACE: &str = "sx1262_config.v1";

/// The owner's 16-bit run.
const RUN: u16 = 0x5A17;

/// Its token count's start under that run.
const COUNTER: u16 = 0x9C41;

/// 2026-11-02T12:00:00Z, the binding's first step, in seconds since the
/// Unix epoch: 305 days after the catalog's.
const NOON: i64 = 1_793_620_800;

/// The ground's reply timeout: a face's, above the contract's 1 s (link.v2
/// §2.20).
const CALL: Duration = Duration::from_secs(3);

/// §15's bytes, as lane P's `config.v2` codec wrote them: the request of
/// step 2 and its reply, step 5's request and its reply.
const SET_RADIO: &str = "4cd5c6291100774000000067a5836140";
const SET_RADIO_REPLY: &str = "495a1700000300005a1700000001000e40001685e710a032584fd0192c2f480002\
                               d0bce2082fc08ca2a0e0fd5ab355882a00";
const CONFIRM: &str = "4800005a179c4200";
const CONFIRM_REPLY: &str = "495a1700000500005a1700000002000e080002d0bce21027c08ca2a0e0fbfab8c522\
                             2a00";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn examples() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/zenoh-modem")
}

/// `sx1262_config.v1`, with its packed file and the standard schema.
fn contract() -> Arc<Contract> {
    static C: std::sync::OnceLock<Arc<Contract>> = std::sync::OnceLock::new();
    Arc::clone(C.get_or_init(|| {
        let l = load_path(&examples().join("config/sx1262_config.v1.toml"));
        Arc::new(
            l.contract
                .unwrap_or_else(|| panic!("sx1262_config.v1 does not load:\n{}", l.report)),
        )
    }))
}

fn iface() -> Interface {
    Interface::of(&contract()).expect("sx1262_config.v1 declares its groups")
}

/// zenoh-modem's catalog, which holds `sx1262_config.v1`.
fn catalog() -> Catalog {
    let bytes = std::fs::read(examples().join("catalog.json")).unwrap();
    Catalog::parse_bytes(&bytes).unwrap()
}

/// The owner's startup (§15's setup): every value its file's, `duty` and
/// `radio` writable, the floor 15 s, `apply_at` 2 s after a reach `set`,
/// and a persisted layer, empty, which `persist` writes on the host.
fn startup() -> Startup {
    let mut file = Values::new();
    for (g, p, v) in [
        ("duty", "fraction_ppm", 10_000),
        ("duty", "period_s", 3_600),
        ("radio", "frequency_hz", 868_100_000),
        ("modulation", "spreading_factor", 7),
    ] {
        file.entry(g.to_owned())
            .or_default()
            .insert(p.to_owned(), Value::Integer(v));
    }
    Startup {
        file,
        persisted: Some(Values::new()),
        floor_s: 15,
        apply_after: Duration::from_secs(2),
        writable: Some(["duty", "radio"].map(str::to_owned).into_iter().collect()),
        ..Startup::default()
    }
}

/// A run store issuing 0x5A17, then the runs after it.
struct Runs(Mutex<u16>);

impl zenkey::run::RunStore for Runs {
    fn next(&self) -> std::io::Result<Option<u16>> {
        let mut n = self.0.lock().unwrap();
        let run = *n;
        *n = n.wrapping_add(1);
        Ok(Some(run))
    }
}

/// The vehicle router: users by name and password, the owner's grants and
/// the ground's (the binding's §12), every other act denied.
async fn face() -> (zenoh::Session, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let dict = dir.path().join("users.txt");
    std::fs::write(
        &dict,
        "router:router-pw\nowner:owner-pw\nground:ground-pw\n",
    )
    .unwrap();
    let all = "[\"put\", \"delete\", \"declare_subscriber\", \"query\", \"declare_queryable\", \
               \"reply\", \"liveliness_token\", \"declare_liveliness_subscriber\", \
               \"liveliness_query\"]";
    let everywhere = "[\"zk2/**\", \"zk2/@zk/**\", \"zk2/*/*/@zk/**\", \"zk2/*/*/*/@op/**\"]";
    let key = |x: &str| format!("\"zk2/{LORA0}/{x}\"");
    let pushed = [
        key(&format!("{IFACE}/state/config")),
        key(&format!("freshness.v2/stream/heartbeat/{IFACE}")),
    ]
    .join(", ");
    let calls = ["set/duty", "set/radio", "confirm", "cancel", "extend"]
        .map(|op| key(&format!("{IFACE}/@op/{op}")))
        .join(", ");
    let acl = format!(
        "{{ enabled: true, default_permission: \"deny\", rules: [\
         {{ id: \"everything\", permission: \"allow\", flows: [\"egress\", \"ingress\"], \
            messages: {all}, key_exprs: {everywhere} }}, \
         {{ id: \"subscribe\", permission: \"allow\", flows: [\"ingress\"], \
            messages: [\"declare_subscriber\"], key_exprs: [\"zk2/**\"] }}, \
         {{ id: \"pushed\", permission: \"allow\", flows: [\"egress\"], \
            messages: [\"put\"], key_exprs: [{pushed}] }}, \
         {{ id: \"calls\", permission: \"allow\", flows: [\"egress\", \"ingress\"], \
            messages: [\"query\", \"reply\"], key_exprs: [{calls}] }}, \
         {{ id: \"routed\", permission: \"allow\", flows: [\"egress\"], \
            messages: [\"declare_queryable\"], key_exprs: [{calls}] }}], \
         subjects: [{{ id: \"vehicle\", usernames: [\"owner\"] }}, \
            {{ id: \"ground\", usernames: [\"ground\"] }}], \
         policies: [{{ id: \"p-vehicle\", rules: [\"everything\"], subjects: [\"vehicle\"] }}, \
            {{ id: \"p-ground\", rules: [\"subscribe\", \"pushed\", \"calls\", \"routed\"], \
               subjects: [\"ground\"] }}] }}"
    );
    let usrpwd = format!(
        "{{ user: \"router\", password: \"router-pw\", dictionary_file: {:?} }}",
        dict.display().to_string()
    );
    let (r, ep) = common::router_with(
        None,
        &[("transport/auth/usrpwd", &usrpwd), ("access_control", &acl)],
    )
    .await;
    (r, ep, dir)
}

/// A client of `ep` authenticated as `user`.
async fn as_user(ep: &str, user: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5(
        "transport/auth/usrpwd",
        &format!("{{ user: \"{user}\", password: \"{user}-pw\" }}"),
    )
    .unwrap();
    zenoh::open(c).await.expect("an authenticated client")
}

/// One delivery, as a subscriber recorded it.
#[derive(Debug, Clone)]
struct Rec {
    at: Instant,
    key: String,
    liveliness: bool,
    payload: Vec<u8>,
}

type Log = Arc<Mutex<Vec<Rec>>>;

/// A recording subscriber of `zk2/<LORA0>/**`, and of its `@zk/**`
/// tokens.
struct Recorder {
    _data: zenoh::pubsub::Subscriber<()>,
    _tokens: zenoh::pubsub::Subscriber<()>,
    log: Log,
}

async fn recorder(s: &zenoh::Session) -> Recorder {
    let log: Log = Arc::default();
    let push = |log: Log, liveliness: bool| {
        move |x: Sample| {
            if x.kind() == SampleKind::Put {
                log.lock().unwrap().push(Rec {
                    at: Instant::now(),
                    key: x.key_expr().as_str().to_owned(),
                    liveliness,
                    payload: x.payload().to_bytes().into_owned(),
                });
            }
        }
    };
    let data = s
        .declare_subscriber(format!("zk2/{LORA0}/**"))
        .callback(push(Arc::clone(&log), false))
        .await
        .unwrap();
    let tokens = s
        .liveliness()
        .declare_subscriber(format!("zk2/{LORA0}/@zk/**"))
        .callback(push(Arc::clone(&log), true))
        .await
        .unwrap();
    Recorder {
        _data: data,
        _tokens: tokens,
        log,
    }
}

impl Recorder {
    fn all(&self) -> Vec<Rec> {
        self.log.lock().unwrap().clone()
    }

    /// Every put of the record: its whole payload, and the record read.
    fn records(&self) -> Vec<(Instant, Vec<u8>, State)> {
        let i = iface();
        self.all()
            .into_iter()
            .filter(|r| r.key.ends_with(&format!("/{IFACE}/state/config")))
            .map(|r| {
                let e = Envelope::parse(&r.payload).expect("a mutation");
                let s = i
                    .read_record(e.tag.get(), e.record, epoch())
                    .expect("a record");
                (r.at, r.payload, s)
            })
            .collect()
    }

    /// Every change event.
    fn events(&self) -> Vec<ChangeEvent> {
        self.all()
            .into_iter()
            .filter(|r| r.key.contains(&format!("/{IFACE}/events/change/")))
            .map(|r| {
                let e = Envelope::parse(&r.payload).expect("an event under its stamp");
                serde_json::from_slice(e.record).expect("a change event")
            })
            .collect()
    }

    /// Waits until `f` holds of the log.
    async fn until(&self, what: &str, f: impl Fn(&Recorder) -> bool) {
        let deadline = Instant::now() + SETTLE;
        while !f(self) {
            assert!(Instant::now() < deadline, "never: {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

/// The catalog's epoch, in seconds.
fn epoch() -> i64 {
    catalog().epoch()
}

/// Moves the owner's clock so that it reads `at` seconds since the Unix
/// epoch, and `into` milliseconds of it, now.
fn clock(cfg: &Configuration, at: i64, into: i64) {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    cfg.simulate_offset(at * 1000 + into - now);
}

/// What the ground's call came back with: a value's payload, or an error's.
#[derive(Debug)]
enum Answer {
    Value(Vec<u8>),
    Error(Vec<u8>),
}

/// The ground's call of `op` with `request`'s bytes, as a far side writes
/// it: the request form under `zenoh/bytes`, the call metadata, one reply.
async fn call(ground: &zenoh::Session, op: &str, request: &[u8], rid: &str) -> Answer {
    let rx = ground
        .get(format!("zk2/{LORA0}/{IFACE}/@op/{op}"))
        .payload(request.to_vec())
        .encoding(Encoding::ZENOH_BYTES)
        .attachment(CallMetadata::new("ground", rid).to_bytes())
        .target(QueryTarget::BestMatching)
        .consolidation(ConsolidationMode::None)
        .timeout(CALL)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let reply = rx.recv_async().await.expect("an answer across the face");
    match reply.into_result() {
        Ok(s) => Answer::Value(s.payload().to_bytes().into_owned()),
        Err(e) => Answer::Error(e.payload().to_bytes().into_owned()),
    }
}

/// A value's payload, and the record it carries.
fn value(a: &Answer) -> (Vec<u8>, State) {
    let Answer::Value(bytes) = a else {
        panic!("a value, not {a:?}")
    };
    let e = read_reply(bytes).expect("the value form");
    let s = iface()
        .read_record(e.tag.get(), e.record, epoch())
        .expect("the record");
    (bytes.clone(), s)
}

/// A request of `layout`'s type: its JSON reading written, under CH.
fn request(layout: &zenkey::model::packed::Layout, fields: &Json) -> Vec<u8> {
    let record = layout.write(fields, Some(epoch())).unwrap().bytes;
    Request::new(Tag::new(layout.tag()).unwrap(), &record).write()
}

/// The record's `group.param`, its running value.
fn param(s: &State, group: &str, name: &str) -> Option<Value> {
    match s.groups.get(group).and_then(|g| g.params.get(name)) {
        Some(ParamState::Plain { value, .. }) => value.clone(),
        other => panic!("{group}.{name}: {other:?}"),
    }
}

/// Seconds since the Unix epoch of a record's time, in milliseconds.
fn secs(ms: i64) -> i64 {
    assert_eq!(ms % 1000, 0, "a whole second");
    ms / 1000
}

/// §15, step by step: the far-side reach change in one frame each way,
/// and the binding's bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_sx1262_reach_round_trip_across_a_face() {
    let (_r, ep, _dir) = face().await;
    let link = common::link_to(&ep).await;
    let ground = as_user(link.endpoint(), "ground").await;
    let vehicle = as_user(&ep, "owner").await;
    let g = recorder(&ground).await;
    let v = recorder(&vehicle).await;
    let owner = as_user(&ep, "owner").await;
    let i = iface();
    let duty = i.derived.set("duty").unwrap().1;
    let radio = i.derived.set("radio").unwrap().1;
    let control = i.derived.control();

    // The requests, as the model writes them: step 2's and step 5's are
    // the binding's bytes.
    let set_duty = request(duty, &json!({"fraction_ppm": 9000, "dry_run": false}));
    let set_radio = request(
        radio,
        &json!({
            "frequency_hz": 869_525_000,
            "confirm_s": 120,
            "expected_revision": 1,
            "idempotency_key": 0x3D2C_1B0A,
            "dry_run": false,
        }),
    );
    let pending_token = (u64::from(RUN) << 16) | u64::from(COUNTER + 1);
    let confirm = request(control, &json!({"token": pending_token}));
    assert_eq!(hex(&set_radio), SET_RADIO, "step 2's request");
    assert_eq!(hex(&confirm), CONFIRM, "step 5's request");

    // 0. The owner starts: the record, revision 0.
    let startup = startup();
    let device = Arc::new(SimDevice::new_v2(&startup));
    let mut b = ServiceBuilder::new(&owner, common::config(LORA0));
    b.run_store(Arc::new(Runs(Mutex::new(RUN)))).unwrap();
    b.catalog(catalog()).unwrap();
    b.implement(Implementation::new((*contract()).clone()))
        .unwrap();
    let cfg = b
        .config_v2(
            startup.clone(),
            Arc::clone(&device) as Arc<dyn config_v2::Device>,
        )
        .await
        .unwrap();
    cfg.simulate_counter_start(COUNTER).await;
    clock(&cfg, NOON, 0);
    let svc = b.start().await.unwrap();
    g.until("the record at start, across the face", |g| {
        !g.records().is_empty()
    })
    .await;
    let (_, start, s0) = g.records().remove(0);
    eprintln!(
        "§15 step 0: the record put, {} B: {}",
        start.len(),
        hex(&start)
    );
    assert_eq!(
        (s0.instance, s0.revision, s0.floor_s),
        (u32::from(RUN), 0, 15)
    );
    assert_eq!((s0.pending.as_ref(), s0.last.as_ref()), (None, None));
    let querier = ground
        .declare_querier(format!("zk2/{LORA0}/{IFACE}/@op/set/radio"))
        .await
        .unwrap();
    eventually("the owner's set/radio is routed to the ground", || async {
        querier.matching_status().await.unwrap().matching()
    })
    .await;
    drop(querier);

    // 1. 12:00:00Z: set/duty, applied at once.
    link.capture_down();
    let r1 = call(&ground, "set/duty", &set_duty, "g1").await;
    let down1 = link.captured_down().len();
    let (reply1, s1) = value(&r1);
    eprintln!(
        "§15 step 1: set/duty {} B, its reply {} B ({down1} B toward the ground): {}",
        set_duty.len(),
        reply1.len(),
        hex(&reply1)
    );
    assert_eq!(s1.revision, 1);
    assert_eq!(
        s1.last.as_ref().map(|l| l.token),
        Some((u64::from(RUN) << 16) | u64::from(COUNTER))
    );
    assert_eq!(
        param(&s1, "duty", "fraction_ppm"),
        Some(Value::Integer(9000))
    );
    v.until("the applied event, on the vehicle's side", |v| {
        v.events().len() == 1
    })
    .await;

    // 2. 12:01:00Z: set/radio, under a 120 s window. The owner reads its
    // clock as the second before, so apply_at and the deadline round up to
    // 12:01:02Z and 12:03:00Z.
    clock(&cfg, NOON + 59, 300);
    link.capture_down();
    let r2 = call(&ground, "set/radio", &set_radio, "g2").await;
    let replied = Instant::now();
    let down2 = link.captured_down().len();
    let (reply2, s2) = value(&r2);
    eprintln!(
        "§15 step 2: set/radio {} B, its reply {} B ({down2} B toward the ground): {}",
        set_radio.len(),
        reply2.len(),
        hex(&reply2)
    );
    let p2 = s2.pending.clone().expect("the change pending");
    assert_eq!(p2.token, pending_token);
    assert_eq!(p2.groups, ["radio"]);
    assert_eq!(p2.apply_at.map(secs), Some(NOON + 62), "apply_at 12:01:02Z");
    assert_eq!(secs(p2.deadline), NOON + 180, "the deadline 12:03:00Z");
    assert_eq!(hex(&reply2), SET_RADIO_REPLY, "step 2's reply");
    assert_eq!(
        device.holds("radio", "frequency_hz"),
        Some(Value::Integer(868_100_000)),
        "the reply left before the apply"
    );
    g.until("the pending record, pushed", |g| {
        g.records().iter().any(|r| r.2.pending.is_some())
    })
    .await;
    let pushed2 = g
        .records()
        .into_iter()
        .find(|r| r.2.pending.is_some())
        .unwrap();
    let put2 = Envelope::parse(&pushed2.1).unwrap();
    let mark2 = read_reply(&reply2).unwrap();
    assert_eq!(put2.stamp, mark2.stamp, "the reply carries the put's seq");
    assert_eq!(put2.record, mark2.record, "and its bytes");

    // 3. Before apply_at: confirm is busy, under step 2's mark.
    let r3 = call(&ground, "confirm", &confirm, "g3").await;
    let Answer::Error(busy) = &r3 else {
        panic!("busy, not {r3:?}")
    };
    let e3 = read_reply_err(busy).expect("the error form");
    eprintln!(
        "§15 step 3: confirm {} B, busy {} B: {}",
        confirm.len(),
        busy.len(),
        hex(busy)
    );
    assert_eq!(e3.code, Code::Busy);
    assert_eq!(e3.stamp, mark2.stamp, "not applied yet");

    // 4. 12:01:02Z: the apply, then the record's push, still pending, no
    // apply_at. The ground holds it from its subscription, with no GET.
    g.until("the applied record, pushed", |g| {
        g.records()
            .iter()
            .any(|r| r.2.pending.as_ref().is_some_and(|p| p.applied))
    })
    .await;
    let (at4, push4, s4) = g
        .records()
        .into_iter()
        .find(|r| r.2.pending.as_ref().is_some_and(|p| p.applied))
        .unwrap();
    let put4 = Envelope::parse(&push4).unwrap();
    eprintln!(
        "§15 step 4: the push at the apply, {} B: {}",
        push4.len(),
        hex(&push4)
    );
    assert!(at4 > replied, "pushed after the reply");
    assert_eq!(put4.stamp.run, put2.stamp.run);
    assert_eq!(put4.stamp.seq.get(), put2.stamp.seq.get() + 1);
    assert_eq!(s4.pending.as_ref().map(|p| p.apply_at), Some(None));
    assert_eq!(
        param(&s4, "radio", "frequency_hz"),
        Some(Value::Integer(869_525_000))
    );
    let applies = device.with(|d| d.applies.clone());
    assert_eq!(applies.len(), 2, "duty at step 1, radio at step 4");
    assert!(applies[1].0 > replied, "the apply after the reply");

    // 5. 12:01:32Z: confirm, kept.
    clock(&cfg, NOON + 92, 0);
    link.capture_down();
    let r5 = call(&ground, "confirm", &confirm, "g5").await;
    let down5 = link.captured_down().len();
    let (reply5, s5) = value(&r5);
    eprintln!(
        "§15 step 5: confirm {} B, its reply {} B ({down5} B toward the ground): {}",
        confirm.len(),
        reply5.len(),
        hex(&reply5)
    );
    assert_eq!((s5.revision, s5.pending.as_ref()), (2, None));
    assert_eq!(s5.last.as_ref().map(|l| l.token), Some(pending_token));
    assert_eq!(hex(&reply5), CONFIRM_REPLY, "step 5's reply");
    v.until("the confirmed event, on the vehicle's side", |v| {
        v.events().len() == 2
    })
    .await;
    assert_eq!(
        v.events().iter().map(|e| e.outcome).collect::<Vec<_>>(),
        [
            zenkey::model::config::Outcome::Applied,
            zenkey::model::config::Outcome::Confirmed
        ]
    );

    // A retry of step 2 within 300 s: step 2's answer, byte for byte.
    let again = call(&ground, "set/radio", &set_radio, "g2").await;
    assert_eq!(value(&again).0, reply2, "the first answer again (§2.14)");

    // Every request at most 16 B, every reply and push at most 64 B.
    for r in [&set_duty, &set_radio, &confirm] {
        assert!(r.len() <= 16, "{} B", r.len());
    }
    for r in [&reply1, &reply2, busy, &reply5] {
        assert!(r.len() <= 64, "{} B", r.len());
    }
    let records = g.records();
    assert!(records.iter().all(|r| r.1.len() <= 64));
    eprintln!(
        "§15: the record's puts across the face: {:?} B",
        records.iter().map(|r| r.1.len()).collect::<Vec<_>>()
    );

    // The face: the ground heard the record and the heartbeat, and no
    // token, no descriptor and no change event.
    let heard = g.all();
    let heartbeat = format!("zk2/{LORA0}/freshness.v2/stream/heartbeat/{IFACE}");
    let record = format!("zk2/{LORA0}/{IFACE}/state/config");
    assert!(
        heard
            .iter()
            .all(|r| !r.liveliness && (r.key == record || r.key == heartbeat)),
        "only the record and its heartbeat cross: {:?}",
        heard.iter().map(|r| &r.key).collect::<Vec<_>>()
    );
    assert!(
        v.all().iter().any(|r| r.liveliness),
        "the vehicle's side hears the tokens"
    );
    svc.close().await.unwrap();
}
