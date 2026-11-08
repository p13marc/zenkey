//! `spec/scenarios/state.md` §1–§9 (spec §4, §2.6), over core 0.73's runs
//! and seqs: every mutation carries its in-band stamp (§7.5's envelope),
//! and a Zenoh timestamp only on §4.3's table.
//!
//! Common setup: an owner `ground/fleet-mgr` with the state template
//! `plans/{vehicle}`; a consumer `vehicle-01/executor` bound with
//! `{vehicle} = self.system`. Routers R1 (ground) and R2 (vehicle) are
//! linked through a [`common::Link`] the archive scenarios cut and heal.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use common::{
    T, client, config, contract, eventually, imp, link_to, router, router_via, router_with,
};
use zenkey::archive::{self, Archive, ArchiveConfig, Recorded};
use zenkey::consumer::Consumer;
use zenkey::model::grammar::IfaceId;
use zenkey::model::seed::{Held, Mark, admits_mark};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{self, DeleteStamp, Envelope, Run, Stamp as InBand};
use zenkey::run::{FileRunStore, RunStore};
use zenkey::state::{Current, StateAnswer, StateGet, StateWriter, ValueOrder};
use zenkey::{Service, ServiceBuilder};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::{Sample, SampleKind};

/// What a union storage holds: each key's bytes and `Encoding`.
type Stored = Arc<Mutex<BTreeMap<String, (Vec<u8>, Encoding)>>>;

/// A union storage on `zk2/*/*/*/events/**` whose backend ignores `_time`,
/// like the storage manager's memory backend (spike S5).
async fn union_storage(
    s: &zenoh::Session,
) -> (
    zenoh::pubsub::Subscriber<()>,
    zenoh::query::Queryable<()>,
    Stored,
) {
    let store: Stored = Arc::default();
    let w = Arc::clone(&store);
    let sub = s
        .declare_subscriber("zk2/*/*/*/events/**")
        .callback(move |smp: Sample| {
            // As a storage keeps a sample: its bytes, envelope included,
            // and its Encoding.
            w.lock().unwrap().insert(
                smp.key_expr().as_str().to_owned(),
                (
                    smp.payload().to_bytes().into_owned(),
                    smp.encoding().clone(),
                ),
            );
        })
        .await
        .unwrap();
    let r = Arc::clone(&store);
    let q = s
        .declare_queryable("zk2/*/*/*/events/**")
        .callback(move |q| {
            for (k, (v, enc)) in r.lock().unwrap().iter() {
                let ke = OwnedKeyExpr::try_from(k.clone()).unwrap();
                if q.key_expr().intersects(&ke) {
                    let _ = q.reply(ke, v.clone()).encoding(enc.clone()).wait();
                }
            }
        })
        .await
        .unwrap();
    (sub, q, store)
}

/// §8 (core §2.6): 1,000 occurrences dated by `put_at`, 500 within the
/// hour, a second apart, the newest at T, and 500 two hours and more before
/// T; one of an unknown time; then the owner restarts under a new run and
/// publishes two dated T − 100.5 s and T − 0.5 s. Every key ends in its
/// run and seq, and its payload is an event (§7.5's form 4). Step 4 (core
/// 0.104, F-514): another session puts a mutation under `zenoh/bytes`
/// and a JSON document with no envelope on two keys of the event no
/// occurrence took. The backend ignores `_time`, so the storage holds all
/// 1,005, and the consumer's replay keeps 503 by each event's time: the
/// 502 with a time in time order across the run change, then the unknown
/// one. Neither of step 4's puts is delivered, by the subscription or the
/// replay, and the subscription counts both undecodable, `form`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_events_replay() {
    let (_r1, ep) = router(None).await;
    let storage = client(&ep).await;
    let reader = client(&ep).await;
    let journal: IfaceId = "journal.v1".parse().unwrap();
    let (_ssub, _sq, store) = union_storage(&storage).await;
    // The storage's subscriber must be known before an owner's puts.
    let subscribed = |s: zenoh::Session| async move {
        let probe = s
            .declare_publisher("zk2/lab/journal/journal.v1/events/entries/002g0009")
            .await
            .unwrap();
        eventually("the storage subscribes", || async {
            probe.matching_status().await.unwrap().matching()
        })
        .await;
    };
    let owner = client(&ep).await;
    let mut b = ServiceBuilder::new(&owner, config("lab/journal"));
    b.implement(imp("journal.v1")).unwrap();
    let events = b
        .event_writer(&journal, "events/entries", &Bindings::new())
        .unwrap();
    let svc = b.start().await.unwrap();
    subscribed(owner.clone()).await;
    // Steps 1 and 2.
    let t = SystemTime::now();
    for i in 0..500u64 {
        events
            .put_at(format!("new{i}"), t - Duration::from_secs(i))
            .await
            .unwrap();
        events
            .put_at(format!("old{i}"), t - Duration::from_secs(7200 + i))
            .await
            .unwrap();
    }
    events.put_untimed("unknown").await.unwrap();
    eventually("the storage holds 1,001", || async {
        store.lock().unwrap().len() == 1001
    })
    .await;
    // The consumer, subscribed from here on (step 4 asks it subscribed):
    // step 3's deliveries show its subscriber is routed before step 4's
    // puts.
    let mut b = ServiceBuilder::new(
        &reader,
        config("lab/auditor").bind("journal", &["lab/journal"]),
    );
    b.require("journal", journal.clone(), false);
    let auditor = b.start().await.unwrap();
    // Each refusal the subscription makes, by key and why (step 4).
    let refused: Arc<Mutex<Vec<(String, zenkey::inband::Undecodable)>>> = Arc::default();
    let r = Arc::clone(&refused);
    let consumer = auditor
        .consumer("journal", Arc::new(contract("journal.v1")))
        .unwrap()
        .with_refused(move |x: zenkey::consumer::Refused| {
            let key = x.read.sample().key_expr().as_str().to_owned();
            r.lock().unwrap().push((key, x.why));
        });
    let heard: Arc<Mutex<Vec<String>>> = Arc::default();
    let h = Arc::clone(&heard);
    let sub = consumer
        .subscribe("events/entries", move |d: zenkey::consumer::Delivery| {
            h.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&d.sample.payload().to_bytes()).into_owned());
        })
        .await
        .unwrap();
    svc.close().await.unwrap();
    // Step 3: a new instance, under a new run.
    let owner = client(&ep).await;
    let mut b = ServiceBuilder::new(&owner, config("lab/journal"));
    b.implement(imp("journal.v1")).unwrap();
    let events = b
        .event_writer(&journal, "events/entries", &Bindings::new())
        .unwrap();
    let _svc = b.start().await.unwrap();
    subscribed(owner.clone()).await;
    for (record, ago) in [("run2-a", 100_500), ("run2-b", 500)] {
        events
            .put_at(record, t - Duration::from_millis(ago))
            .await
            .unwrap();
    }
    eventually("the storage holds 1,003", || async {
        store.lock().unwrap().len() == 1003
    })
    .await;
    eventually("the subscription hears step 3's", || async {
        heard.lock().unwrap().iter().any(|r| r == "run2-b")
    })
    .await;
    // Step 4: two puts that carry no event, on occurrence chunks of a
    // 16-bit run no instance took (each instance's is a 32-bit draw).
    let stray = client(&ep).await;
    let stray_key = |seq: u32| {
        let e = wire::Event {
            tag: wire::Tag::NONE,
            stamp: InBand {
                run: Run::U16(0x7777),
                seq: wire::Seq::new(seq).unwrap(),
            },
            time: zenkey::model::collection::Time::Unknown,
            record: b"",
        };
        format!("zk2/lab/journal/journal.v1/events/entries/{}", e.chunk())
    };
    let (mutation_key, json_key) = (stray_key(1), stray_key(2));
    // Each a key the consumer reads (review-bot on #1104): an events key
    // whose last chunk is a 16-bit run's occurrence chunk, run 0x7777 and
    // its seq, so a refusal below is the payload's, never the key's.
    for (key, seq) in [(&mutation_key, 1), (&json_key, 2)] {
        assert!(
            matches!(
                zenkey::model::grammar::parse(key),
                Ok(zenkey::model::grammar::ZkKey::Data { .. })
            ),
            "{key}: a zk2 data key"
        );
        let chunk = key.rsplit('/').next().unwrap();
        assert_eq!(chunk.len(), 8, "{key}: a 16-bit run's chunk");
        assert_eq!(
            zenkey::model::chunk::occurrence(chunk),
            Some(InBand {
                run: Run::U16(0x7777),
                seq: wire::Seq::new(seq).unwrap(),
            }),
            "{key}"
        );
    }
    let mutation = Envelope::mutation(
        wire::Tag::NONE,
        InBand {
            run: Run::U16(0x7777),
            seq: wire::Seq::new(1).unwrap(),
        },
        b"{}",
    )
    .write();
    stray
        .put(&mutation_key, mutation)
        .encoding(Encoding::ZENOH_BYTES)
        .await
        .unwrap();
    stray
        .put(&json_key, b"{}".to_vec())
        .encoding(Encoding::APPLICATION_JSON)
        .await
        .unwrap();
    eventually("the storage holds 1,005", || async {
        store.lock().unwrap().len() == 1005
    })
    .await;
    // Expected 5, the subscription's half: neither delivered, both counted
    // undecodable.
    eventually("both counted undecodable", || async {
        sub.undecodable() == 2
    })
    .await;
    // Each refused for carrying no event, token `form` (core §2.6).
    let mut why = refused.lock().unwrap().clone();
    why.sort_by(|a, b| a.0.cmp(&b.0));
    let mut expected = vec![mutation_key.clone(), json_key.clone()];
    expected.sort();
    assert_eq!(
        why.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        expected,
        "step 4's two keys, each refused once"
    );
    assert!(
        why.iter()
            .all(|(_, w)| *w == zenkey::inband::Undecodable::NoEvent && w.token() == "form"),
        "{why:?}"
    );
    assert!(
        heard.lock().unwrap().iter().all(|r| r != "{}"),
        "no put that carries no event is delivered"
    );
    // Expected 1: each key ends in its run and seq, its payload an event;
    // step 4's two carry none.
    let mut runs = std::collections::HashSet::new();
    for (key, (bytes, _)) in store.lock().unwrap().iter() {
        if *key == mutation_key || *key == json_key {
            continue;
        }
        let chunk = key.rsplit('/').next().unwrap();
        assert!(matches!(chunk.len(), 8 | 12), "{key}");
        let e = wire::read_event(key, bytes)
            .expect("an event")
            .expect("read");
        assert_eq!(e.chunk(), chunk, "its stamp is its key's");
        runs.insert(e.stamp.run);
    }
    assert_eq!(runs.len(), 2, "one run per instance");

    let got = consumer
        .replay("events/entries", Duration::from_secs(3600), T)
        .await
        .unwrap();
    // Expected 2: the 503, dropped by each event's time.
    let records: Vec<String> = got
        .iter()
        .map(|d| String::from_utf8(d.sample.payload().to_bytes().into_owned()).unwrap())
        .collect();
    assert_eq!(
        records.len(),
        503,
        "the recent ones, step 2's and step 3's; neither of step 4's (Expected 5)"
    );
    assert!(records.iter().all(|r| !r.starts_with("old") && r != "{}"));
    // Expected 3: time order across the run change.
    let times: Vec<_> = got[..502]
        .iter()
        .map(|d| match d.occurred {
            Some(zenkey::model::collection::Time::Known { s, ms }) => (s, ms),
            other => panic!("a known time, not {other:?}"),
        })
        .collect();
    assert!(times.is_sorted(), "by time");
    let at = |r: &str| records.iter().position(|x| x == r).unwrap();
    assert_eq!(
        (at("new101") + 1, at("run2-a"), at("new100") - 1),
        (at("run2-a"), at("run2-a"), at("run2-a"))
    );
    assert_eq!(
        (at("new1") + 1, at("run2-b"), at("new0") - 1),
        (at("run2-b"), at("run2-b"), at("run2-b"))
    );
    // Expected 4: the unknown time last, never judged.
    assert_eq!(records[502], "unknown");
    assert_eq!(
        got[502].occurred,
        Some(zenkey::model::collection::Time::Unknown)
    );
}

/// #980's C2b and C2d (core §2.6): a replay's order. Its storage answers,
/// in no order, five events on occurrence chunks (§7.5's form 4) of run 7,
/// at seq 1 80 s old, seq 3 50 s old, seq 2 of an unknown time, seq 4 two
/// hours old, and run 9's seq 1 of an unknown time, and a put on a key
/// that ends in a ULID, which is no zk2 key. The two-hour-old one is
/// beyond the retention; the unknown times are never judged so, and come
/// after every known time, by seq within a run, run 7's before run 9's
/// (the order between two runs is unspecified, and the reference breaks
/// the tie by the run's value). Each delivery's in-band stamp is its key's
/// chunk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replay_orders_by_time_then_by_run_and_seq() {
    use zenkey::model::collection::Time;
    use zenkey::model::wire::{Event, Seq, Tag};
    let (_r1, ep) = router(None).await;
    let storage = client(&ep).await;
    let reader = client(&ep).await;
    let base = "zk2/lab/journal/journal.v1/events/entries";
    let now = SystemTime::now();
    let at = |ago: u64| {
        let s = (now - Duration::from_secs(ago))
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();
        Time::Known {
            s: u32::try_from(s.as_secs()).unwrap(),
            ms: u16::try_from(s.subsec_millis()).unwrap(),
        }
    };
    let stamp = |run: u16, seq: u32| InBand {
        run: Run::U16(run),
        seq: Seq::new(seq).unwrap(),
    };
    let event = |run: u16, seq: u32, time: Time, record: &'static [u8]| {
        let e = Event {
            tag: Tag::NONE,
            stamp: stamp(run, seq),
            time,
            record,
        };
        (format!("{base}/{}", e.chunk()), e.write().unwrap())
    };
    let answers: Vec<(String, Vec<u8>)> = vec![
        event(7, 3, at(50), b"seq 3"),
        event(9, 1, Time::Unknown, b"run 9"),
        event(7, 4, at(7200), b"too old"),
        event(7, 2, Time::Unknown, b"unknown"),
        (
            format!("{base}/01j9zk6q6x5m2f4a8c0d3e7b9h"),
            Envelope::mutation(Tag::NONE, stamp(1, 10), b"ulid").write(),
        ),
        event(7, 1, at(80), b"seq 1"),
    ];
    let _q = storage
        .declare_queryable(format!("{base}/**"))
        .callback(move |q| {
            for (k, v) in &answers {
                let _ = q
                    .reply(k.clone(), v.clone())
                    .encoding(Encoding::ZENOH_BYTES)
                    .wait();
            }
        })
        .await
        .unwrap();
    let consumer = Consumer::for_tool(
        &reader,
        Arc::new(contract("journal.v1")),
        &["lab/journal"],
        &BTreeMap::new(),
    )
    .unwrap();
    let mut got = Vec::new();
    for _ in 0..50 {
        got = consumer
            .replay("events/entries", Duration::from_secs(3600), T)
            .await
            .unwrap();
        if got.len() == 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let records: Vec<String> = got
        .iter()
        .map(|d| String::from_utf8(d.sample.payload().to_bytes().into_owned()).unwrap())
        .collect();
    assert_eq!(
        records,
        ["seq 1", "seq 3", "unknown", "run 9"],
        "no ULID's key"
    );
    let stamps: Vec<Option<InBand>> = got.iter().map(|d| d.inband).collect();
    assert_eq!(
        stamps,
        [
            Some(stamp(7, 1)),
            Some(stamp(7, 3)),
            Some(stamp(7, 2)),
            Some(stamp(9, 1)),
        ],
        "an event's stamp is its key's chunk"
    );
}

fn plan() -> IfaceId {
    "mission_plan.v1".parse().unwrap()
}

fn vehicle(v: &str) -> Bindings {
    [("vehicle".to_owned(), vec![v.to_owned()])].into()
}

const ORIGIN: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";

/// How an owner comes up: its run store and its clock's offset.
#[derive(Default)]
struct Up {
    window_s: Option<u64>,
    store: Option<Arc<dyn RunStore>>,
    offset_ms: i64,
}

/// The owner, serving `plans/{vehicle}`, with writers for `vehicles`.
async fn fleet_mgr(
    s: &zenoh::Session,
    window_s: Option<u64>,
    vehicles: &[&str],
) -> (Service, BTreeMap<String, StateWriter>) {
    fleet_mgr_up(
        s,
        Up {
            window_s,
            ..Up::default()
        },
        vehicles,
    )
    .await
}

/// [`fleet_mgr`], brought up as `up` says.
async fn fleet_mgr_up(
    s: &zenoh::Session,
    up: Up,
    vehicles: &[&str],
) -> (Service, BTreeMap<String, StateWriter>) {
    let mut cfg = config("ground/fleet-mgr");
    cfg.tombstone_window_s = up.window_s;
    let mut b = ServiceBuilder::new(s, cfg);
    if let Some(store) = up.store {
        b.run_store(store).unwrap();
    }
    b.minter().simulate_offset(up.offset_ms);
    b.implement(imp("mission_plan.v1")).unwrap();
    b.expose(&plan(), "state/plans/{vehicle}").unwrap();
    b.serve_state(&plan()).unwrap();
    let mut svc = b.start().await.unwrap();
    let mut w = BTreeMap::new();
    for v in vehicles {
        w.insert(
            (*v).to_owned(),
            svc.state_writer(&plan(), "state/plans/{vehicle}", &vehicle(v))
                .await
                .unwrap(),
        );
    }
    (svc, w)
}

/// The run a running service's descriptor states (core §3.3).
fn described_run(svc: &Service) -> String {
    let d: serde_json::Value = serde_json::from_slice(&svc.descriptor_bytes()).unwrap();
    d["run"]
        .as_str()
        .expect("a descriptor states its run")
        .to_owned()
}

/// A consumer of `plan`, bound to the fleet manager, with `{vehicle} =
/// self.system` when `own_slice`.
async fn executor(s: &zenoh::Session, own_slice: bool) -> (Service, Consumer) {
    let mut cfg = config("vehicle-01/executor").bind("plan", &["ground/fleet-mgr"]);
    if own_slice {
        cfg.bindings
            .get_mut("plan")
            .unwrap()
            .params
            .insert("vehicle".to_owned(), "self.system".to_owned());
    }
    let mut b = ServiceBuilder::new(s, cfg);
    b.require("plan", plan(), false);
    let svc = b.start().await.unwrap();
    let c = svc
        .consumer("plan", Arc::new(contract("mission_plan.v1")))
        .unwrap();
    (svc, c)
}

/// The owner's answer with its in-band stamps (S2): what orders each key.
async fn answer(c: &Consumer, values: Option<&Bindings>) -> StateAnswer {
    // Asked again until the owner answers (#1028): under load a GET can
    // end at its timeout before the owner's reply arrives, which is no
    // answer, and nothing the owner holds moves meanwhile.
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    loop {
        let a = c
            .get_answer("state/plans/{vehicle}", values, T)
            .await
            .unwrap();
        if !a.current.is_empty() {
            return a;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the owner did not answer"
        );
    }
}

/// Every sample a raw subscriber on `selector` hears, as it arrived: its
/// envelope, Encoding and Zenoh timestamp untouched.
type Raw = Arc<Mutex<Vec<Sample>>>;

async fn capture(s: &zenoh::Session, selector: &str) -> (zenoh::pubsub::Subscriber<()>, Raw) {
    let raw: Raw = Arc::default();
    let into = Arc::clone(&raw);
    let sub = s
        .declare_subscriber(selector.to_owned())
        .callback(move |smp: Sample| into.lock().unwrap().push(smp))
        .await
        .unwrap();
    (sub, raw)
}

/// A raw GET of `selector`, `All` + `None` as S4 asks (core 0.100): each
/// reply as it arrived, a snapshot's page as the per-key replies its
/// entries stand for ([`zenkey::inband::replies`]; #977's E3).
async fn raw_get(s: &zenoh::Session, selector: &str) -> Vec<Sample> {
    raw_pages(s, selector)
        .await
        .into_iter()
        .flat_map(|smp| match zenkey::inband::read(smp.clone()) {
            zenkey::inband::Read::Snapshot { page: Ok(p), .. } => zenkey::inband::replies(&smp, &p),
            _ => vec![smp],
        })
        .collect()
}

/// [`raw_get`] asked again until an owner answers (#1028): a GET under
/// load can end at its timeout before the owner's reply arrives, which is
/// no answer, and nothing the owner holds moves meanwhile.
async fn raw_get_answered(s: &zenoh::Session, selector: &str) -> Vec<Sample> {
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    loop {
        let got = raw_get(s, selector).await;
        if !got.is_empty() {
            return got;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no owner answered {selector}"
        );
    }
}

/// A raw GET of `selector`, `All` + `None`: each reply as it arrived, a
/// snapshot's pages whole.
async fn raw_pages(s: &zenoh::Session, selector: &str) -> Vec<Sample> {
    raw_get_under(s, selector, ConsolidationMode::None).await
}

/// A raw GET of `selector` under `consolidation`, each reply as it arrived.
async fn raw_get_under(
    s: &zenoh::Session,
    selector: &str,
    consolidation: ConsolidationMode,
) -> Vec<Sample> {
    let rx = s
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(consolidation)
        .timeout(T)
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        if let Ok(smp) = r.into_result() {
            out.push(smp);
        }
    }
    out
}

/// A put's in-band stamp and record, read from its raw payload.
fn sealed(s: &Sample) -> (InBand, Vec<u8>) {
    assert_eq!(
        s.encoding().to_string(),
        "zenoh/bytes",
        "an envelope's Encoding (core §7.2)"
    );
    let bytes = s.payload().to_bytes();
    let e = Envelope::parse(&bytes).expect("a mutation envelope");
    assert_eq!(e.header().form, wire::Form::Mutation);
    (e.stamp, e.record.to_vec())
}

/// A delete's in-band stamp, read from its raw attachment.
fn delete_stamp(s: &Sample) -> InBand {
    assert_eq!(s.kind(), SampleKind::Delete);
    let a = s.attachment().expect("a delete's envelope").to_bytes();
    DeleteStamp::parse(&a).expect("a delete envelope").stamp
}

/// §1: in-band stamps (S1, S2, core §4.3), and no per-put Zenoh timestamp
/// outside §4.3's table. Played twice: through R1, whose timestamping is
/// disabled, nothing of `plans/{vehicle}` carries a Zenoh timestamp,
/// which is #976's capture; through R2, which stamps, each put arrives
/// with R2's zid, its delete and its reply with none. `status`
/// (`status.v1`, which uses `freshness.v1`, `freshness.ttl_s` 60, on the
/// table) carries the owner's zid through either. Since #978's F4,
/// `health.v1`'s status uses `freshness.v2`, whose puts leave the table.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_in_band_stamps_and_no_per_put_hlc() {
    in_band_stamps(false).await;
    in_band_stamps(true).await;
}

async fn in_band_stamps(stamping: bool) {
    let through = if stamping {
        "R2, which stamps"
    } else {
        "R1, which does not"
    };
    let (r, ep) = router_with(
        None,
        &[(
            "timestamping/enabled",
            if stamping { "true" } else { "false" },
        )],
    )
    .await;
    let owner = client(&ep).await;
    let consumer = client(&ep).await;
    let third = client(&ep).await;
    let (_plans, plans) = capture(&consumer, "zk2/ground/fleet-mgr/mission_plan.v1/**").await;
    let (_status, status) = capture(&consumer, "zk2/ground/fleet-mgr/status.v1/**").await;
    let (_control, control) = capture(&consumer, "control/**").await;

    let mut b = ServiceBuilder::new(&owner, config("ground/fleet-mgr"));
    b.implement(imp("mission_plan.v1")).unwrap();
    b.implement(status_v1()).unwrap();
    let status_w = b
        .declare_state_writer(
            &"status.v1".parse().unwrap(),
            "state/status",
            &Bindings::new(),
        )
        .await
        .unwrap();
    let w = b
        .declare_state_writer(&plan(), "state/plans/{vehicle}", &vehicle("vehicle-01"))
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    let run = described_run(&svc);
    assert_ne!(
        owner.zid().to_string(),
        r.zid().to_string(),
        "the owner's session is not the router"
    );
    // Every writer matched before it puts (#1028): a put its session has
    // no matching subscription for yet is dropped at its source.
    let control_pub = third.declare_publisher("control/x").await.unwrap();
    eventually("subscribed", || async {
        w.writer().matching().await.unwrap()
            && status_w.writer().matching().await.unwrap()
            && control_pub.matching_status().await.unwrap().matching()
    })
    .await;

    // 1. v1, v2, the GET after v2, then the delete.
    let v1 = w.put("v1").await.unwrap();
    let v2 = w.put("v2").await.unwrap();
    let replies = raw_get_answered(&consumer, ORIGIN).await;
    assert_eq!(replies.len(), 1, "{through}: one reply");
    assert_eq!(
        &*replies[0].payload().to_bytes(),
        &v2.seal(b"v2")[..],
        "{through}: 2. the GET returns v2's bytes, its envelope included"
    );
    assert_eq!(
        replies[0].timestamp(),
        None,
        "{through}: 3. a reply carries no Zenoh timestamp"
    );
    let del = w.delete().await.unwrap();
    // 2. The control: an unstamped put of a third client's.
    control_pub.put("c").await.unwrap();
    // 3. v3 and v4 back to back.
    let v3 = w.put("v3").await.unwrap();
    let v4 = w.put("v4").await.unwrap();
    // 4. `status` twice, then its GET.
    let mark = status.lock().unwrap().len();
    status_w.put("one").await.unwrap();
    status_w.put("two").await.unwrap();

    eventually("every mutation arrives", || async {
        plans.lock().unwrap().len() >= 5
            && status.lock().unwrap().len() >= mark + 2
            && !control.lock().unwrap().is_empty()
    })
    .await;
    let got = plans.lock().unwrap().clone();
    assert_eq!(got.len(), 5, "{through}: {got:?}");

    // 1. Each put an envelope of the descriptor's run, the delete's
    //    attachment one too, each seq above the one before.
    let mut stamps = Vec::new();
    for (s, value) in got.iter().zip(["v1", "v2", "", "v3", "v4"]) {
        let stamp = if value.is_empty() {
            delete_stamp(s)
        } else {
            let (stamp, record) = sealed(s);
            assert_eq!(record, value.as_bytes(), "{through}: the record");
            stamp
        };
        assert_eq!(stamp.run.hex(), run, "{through}: the descriptor's run");
        stamps.push(stamp);
    }
    assert_eq!(
        stamps,
        [v1.stamp, v2.stamp, del.stamp, v3.stamp, v4.stamp],
        "{through}: the stamps the owner minted"
    );
    assert!(
        stamps.windows(2).all(|p| p[0].seq < p[1].seq),
        "{through}: each seq above the one before"
    );
    for v in [v1, v2, del, v3, v4] {
        assert_eq!(v.time, None, "the owner sets no Zenoh timestamp here");
    }

    // 3. No per-put HLC through R1; R2's zid on each put through R2.
    for s in &got {
        let ts = s.timestamp().map(|t| t.get_id().to_string());
        if stamping && s.kind() == SampleKind::Put {
            assert_eq!(ts, Some(r.zid().to_string()), "{through}: the router's zid");
        } else {
            assert_eq!(ts, None, "{through}: no Zenoh timestamp on {:?}", s.kind());
        }
    }
    let c = control.lock().unwrap()[0].clone();
    assert_eq!(
        c.timestamp().map(|t| t.get_id().to_string()),
        stamping.then(|| r.zid().to_string()),
        "{through}: the control's put"
    );

    // 4. v4's seq is v3's plus 1.
    assert_eq!(v4.stamp.seq.get(), v3.stamp.seq.get() + 1);

    // 5. `status` carries the owner's own Zenoh timestamp, increasing.
    let puts: Vec<Sample> = status.lock().unwrap()[mark..].to_vec();
    let times: Vec<_> = puts
        .iter()
        .map(|s| *s.timestamp().expect("a table resource's stamp"))
        .collect();
    assert!(
        times
            .iter()
            .all(|t| t.get_id().to_string() == owner.zid().to_string()),
        "{through}: the owner's zid"
    );
    assert!(times[1] > times[0], "{through}: the second above the first");
    for s in &puts {
        assert_eq!(sealed(s).0.run.hex(), run, "{through}: status's run");
    }
    let reply = raw_get_answered(&consumer, "zk2/ground/fleet-mgr/status.v1/state/status").await;
    assert_eq!(
        reply[0].timestamp().map(|t| t.get_id().to_string()),
        Some(owner.zid().to_string()),
        "{through}: the reply carries the mutation's stamp (S2)"
    );
    drop(w);
    svc.close().await.unwrap();
}

/// `status.v1`: one state, `freshness.ttl_s` 60, under `freshness.v1`, so
/// core §4.3's table lists it (row 1) and its puts carry the owner's Zenoh
/// timestamp. Never published.
fn status_v1() -> zenkey::Implementation {
    let text = "[interface]\nname = \"status\"\nmajor = 1\nminor = 0\nuses = [\"freshness.v1\"]\n\
                [resources.status]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
                annotations = { \"freshness.ttl_s\" = 60 }\n";
    let l = zenkey::model::contract::load_str(text, std::path::Path::new("."), None);
    zenkey::Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)))
}

/// §2: within the window, a deleted key answers `reply_del` whose
/// attachment is the delete's envelope, alone and inside the collection;
/// the live key answers the bytes the owner put.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_reply_del_carries_the_delete_envelope() {
    let (_r1, ep) = router(None).await;
    let (_mgr, w) = fleet_mgr(&client(&ep).await, None, &["a", "b"]).await;
    let tool = client(&ep).await;
    let (_ex, c) = executor(&client(&ep).await, false).await;
    let a = w["a"].put("pa").await.unwrap();
    w["b"].put("pb").await.unwrap();
    let del = w["b"].delete().await.unwrap();
    const A: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/a";
    const B: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/b";

    let b = raw_get(&tool, B).await;
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(delete_stamp(&b[0]), del.stamp, "the delete's envelope");
    assert_eq!(b[0].timestamp(), None);
    let all = raw_get(&tool, "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*").await;
    let by_key: BTreeMap<&str, &Sample> = all.iter().map(|s| (s.key_expr().as_str(), s)).collect();
    assert_eq!(by_key.len(), 2);
    assert_eq!(&*by_key[A].payload().to_bytes(), &a.seal(b"pa")[..]);
    assert_eq!(delete_stamp(by_key[B]), del.stamp);

    // The consumer reads the same: the deletion with its in-band stamp.
    let got = answer(&c, None).await;
    assert_eq!(got.inband.get(B), Some(&del.stamp));
    assert_eq!(got.inband.get(A), Some(&a.stamp));
    assert!(
        got.current
            .iter()
            .any(|c| c.key() == B && matches!(c, Current::Deleted { .. }))
    );
}

/// §3 (step 2): the consumer's GET (`All` + `None`, set explicitly by
/// `Consumer::get`) is answered by the owner alone, under its in-band
/// stamp. Step 1, the storage finding read from the routers' admin space,
/// is a tool's (`zenctl doctor`, #612).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_the_owner_is_authoritative() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let (mgr, w) = fleet_mgr(&owner, None, &["vehicle-01"]).await;
    let (_ex, c) = executor(&client(&ep).await, true).await;
    let t = w["vehicle-01"].put("mine").await.unwrap();
    let now = answer(&c, None).await;
    assert_eq!(now.current.len(), 1, "one answer: the owner's");
    assert_eq!(now.inband.get(ORIGIN), Some(&t.stamp));
    assert_eq!(t.stamp.run.hex(), described_run(&mgr));
}

/// Two sites: the ground (R1) with the owner, the vehicle (R2) with its
/// archive and the executor, linked by a cuttable link.
struct Sites {
    link: common::Link,
    ground: zenoh::Session,
    vehicle: zenoh::Session,
    /// The vehicle archive's watch session (#830): where it learns the
    /// owner came back, however short the outage was.
    vehicle_watch: zenoh::Session,
    _routers: (zenoh::Session, zenoh::Session),
}

async fn sites() -> Sites {
    let (r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (r2, ep2) = router_via(&link).await;
    Sites {
        ground: client(&ep1).await,
        vehicle: client(&ep2).await,
        vehicle_watch: client(&ep2).await,
        link,
        _routers: (r1, r2),
    }
}

fn recorded(selector: &str) -> Recorded {
    Recorded {
        owner: common::addr("ground/fleet-mgr"),
        selector: selector.to_owned(),
        implementation: imp("mission_plan.v1"),
    }
}

/// The vehicle's archive on `s`, its owners' appearances heard on `watch`
/// (#830): a session of its own, since it reads their presence on `s`.
async fn vehicle_archive(s: &zenoh::Session, watch: &zenoh::Session, peers: &[&str]) -> Archive {
    Archive::start(
        s,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![recorded(ORIGIN)],
            peers: peers.iter().map(|p| common::addr(p)).collect(),
            unconfirmed_horizon: None,
            watch: Some(watch.clone()),
        },
    )
    .await
    .unwrap()
}

async fn wait_owner_seen(s: &zenoh::Session, present: bool) {
    eventually(
        if present {
            "the owner is visible"
        } else {
            "the owner is gone"
        },
        || async {
            zenkey::presence::liveliness_keys(s, "zk2/ground/fleet-mgr/@zk/instance/*", T)
                .await
                .unwrap()
                .is_empty()
                != present
        },
    )
    .await;
}

/// §4: with the link cut, the owner is silent; the archive gives v3's
/// bytes as the owner put them, its envelope included, with its type
/// identity, as last-known.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_last_known_from_an_archive() {
    let x = sites().await;
    let (_mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
    let arch = vehicle_archive(&x.vehicle, &x.vehicle_watch, &[]).await;
    let (_ex, c) = executor(&x.vehicle, true).await;
    wait_owner_seen(&x.vehicle, true).await;
    eventually("the archive records", || async {
        w["vehicle-01"].put("v2").await.unwrap();
        arch.confirmed(ORIGIN) == Some(true)
    })
    .await;
    // v3, once, and the archive holding exactly it before the cut.
    let t3 = w["vehicle-01"].put("v3").await.unwrap();
    let archived = archive::archive_key(&common::addr("vehicle-01/archive"), ORIGIN);
    eventually("the archive recorded v3", || async {
        raw_get(&x.vehicle, &archived)
            .await
            .first()
            .is_some_and(|s| *s.payload().to_bytes() == t3.seal(b"v3")[..])
    })
    .await;
    x.link.cut();
    wait_owner_seen(&x.vehicle, false).await;
    assert!(matches!(
        c.get("state/plans/{vehicle}", None, T).await.unwrap(),
        StateGet::Silent
    ));
    // The archive's reply: v3's bytes verbatim, envelope included (§4.4).
    let raw = raw_get(&x.vehicle, &archived).await;
    assert_eq!(raw.len(), 1);
    assert_eq!(sealed(&raw[0]), (t3.stamp, b"v3".to_vec()));
    let lk = archive::last_known(&x.vehicle, &common::addr("vehicle-01/archive"), ORIGIN, T)
        .await
        .unwrap()
        .expect("the archive answers");
    assert_eq!(lk.value.as_deref(), Some(&b"v3"[..]), "its record");
    assert_eq!(lk.inband, Some(t3.stamp), "the owner's in-band stamp");
    assert_ne!(
        lk.timestamp.map(|t| t.get_id().to_string()),
        Some(x.ground.zid().to_string()),
        "plain state carries no Zenoh timestamp of its owner's (a router's at most)"
    );
    assert_eq!(lk.identity["iface"], "mission_plan.v1");
    assert_eq!(
        lk.identity["contract"],
        zenkey::Implementation::new(contract("mission_plan.v1"))
            .fingerprint()
            .to_string()
    );
    assert_eq!(lk.identity["type"]["kind"], "raw");
    // The same, over a pattern (#671): every origin the archive holds that
    // the pattern selects, here the one it recorded.
    let all = archive::last_known_all(
        &x.vehicle,
        &common::addr("vehicle-01/archive"),
        "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*",
        T,
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].origin, ORIGIN);
    assert_eq!(all[0].value.as_deref(), Some(&b"v3"[..]));
}

/// §5: alignment after reconnect drops a key only on a `reply_del`, keeps
/// it unconfirmed on an empty reply set, and aligns from an owner-side
/// archive while the owner is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_alignment_after_reconnect() {
    let lk = |s: &zenoh::Session| {
        let s = s.clone();
        async move {
            archive::last_known(&s, &common::addr("vehicle-01/archive"), ORIGIN, T)
                .await
                .unwrap()
        }
    };
    // 1. Deleted during the cut, within W: dropped after the heal.
    {
        let x = sites().await;
        let (_mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
        let arch = vehicle_archive(&x.vehicle, &x.vehicle_watch, &[]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        x.link.heal();
        eventually("dropped on the owner's reply_del", || async {
            lk(&x.vehicle).await.is_some_and(|l| l.value.is_none())
        })
        .await;
    }
    // 2. The same, but the archive's read is refused once: nothing dropped,
    //    served unconfirmed.
    {
        let x = sites().await;
        let (_mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
        let arch = vehicle_archive(&x.vehicle, &x.vehicle_watch, &[]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        // The access control refuses the archive's reads, its retries included.
        arch.refuse_next_reads(u32::MAX);
        x.link.heal();
        eventually("kept, unconfirmed", || async {
            arch.confirmed(ORIGIN) == Some(false)
        })
        .await;
        let l = lk(&x.vehicle).await.expect("still served");
        assert_eq!(l.value.as_deref(), Some(&b"p"[..]));
        assert!(!l.confirmed);
    }
    // 3. The owner is gone; an archive on its side recorded the delete.
    {
        let x = sites().await;
        let (mgr, w) = fleet_mgr(&x.ground, None, &["vehicle-01"]).await;
        let _ground_arch = Archive::start(
            &x.ground,
            ArchiveConfig {
                service: config("ground/archive"),
                records: vec![recorded(
                    "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*",
                )],
                peers: vec![],
                unconfirmed_horizon: None,
                watch: None,
            },
        )
        .await
        .unwrap();
        let arch = vehicle_archive(&x.vehicle, &x.vehicle_watch, &["ground/archive"]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        drop(w);
        mgr.close().await.unwrap();
        x.link.heal();
        eventually("dropped on the owner-side archive's reply_del", || async {
            lk(&x.vehicle).await.is_some_and(|l| l.value.is_none())
        })
        .await;
    }
}

/// §6 step 1 (core 0.100, #977's E3): a delete during an outage reaches the
/// archive after the heal whatever the window. Within it, the owner's
/// snapshot answers the plan's key with its tombstone, and the archive holds
/// the deletion; beyond it, with a snapshot of no entry, positive evidence
/// by the omission rule, and the archive drops the key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_a_window_shorter_than_the_outage() {
    for (window, tombstone) in [(5u64, true), (1u64, false)] {
        let x = sites().await;
        let (_mgr, w) = fleet_mgr(&x.ground, Some(window), &["vehicle-01"]).await;
        let arch = vehicle_archive(&x.vehicle, &x.vehicle_watch, &[]).await;
        wait_owner_seen(&x.vehicle, true).await;
        eventually("recorded", || async {
            w["vehicle-01"].put("p").await.unwrap();
            arch.confirmed(ORIGIN) == Some(true)
        })
        .await;
        x.link.cut();
        wait_owner_seen(&x.vehicle, false).await;
        w["vehicle-01"].delete().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        x.link.heal();
        wait_owner_seen(&x.vehicle, true).await;
        let lk = || async {
            archive::last_known(&x.vehicle, &common::addr("vehicle-01/archive"), ORIGIN, T)
                .await
                .unwrap()
        };
        if tombstone {
            eventually("W covers the outage: the snapshot's tombstone", || async {
                lk().await.is_some_and(|l| l.value.is_none())
            })
            .await;
        } else {
            eventually(
                "beyond W: dropped on the owner's snapshot of no entry",
                || async { lk().await.is_none() },
            )
            .await;
        }
    }
}

/// A session with a pinned zid, as an owner that must catch up uses.
/// One put's arrival at a raw subscriber on `ORIGIN`, the first sample
/// after `from`: its stamp and its first byte, CH.
async fn heard(raw: &Raw, from: usize, what: &str) -> (InBand, u8) {
    eventually(what, || async { raw.lock().unwrap().len() > from }).await;
    let s = raw.lock().unwrap()[from].clone();
    let ch = s.payload().to_bytes()[0];
    (sealed(&s).0, ch)
}

/// §7 step 1, a record: rev 10 under run n; the owner restarts with its
/// clock 5 s behind and reads its record; rev 11 comes under run n + 1,
/// which the record holds before rev 11 is put and the new descriptor
/// states; the consumer accepts it as another run's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_a_run_record_persists_across_a_restart() {
    let (_r1, ep) = router(None).await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fleet-mgr.run");
    let store = || -> Arc<dyn RunStore> { Arc::new(FileRunStore::new(&path)) };
    let mut order = ValueOrder::new();

    let s = client(&ep).await;
    let (mgr, w) = fleet_mgr_up(
        &s,
        Up {
            store: Some(store()),
            ..Up::default()
        },
        &["vehicle-01"],
    )
    .await;
    let rev10 = w["vehicle-01"].put("rev10").await.unwrap();
    let n = match rev10.stamp.run {
        Run::U16(n) => n,
        other => panic!("a counter's run, not {other:?}"),
    };
    assert_eq!(described_run(&mgr), rev10.stamp.run.hex());
    assert_eq!(std::fs::read(&path).unwrap(), n.to_be_bytes());
    assert!(order.accept_mark(ORIGIN, Mark::InBand(rev10.stamp)));
    drop(w);
    mgr.close().await.unwrap();
    s.close().await.unwrap();

    // The restart: 5 s behind, its record read.
    let s = client(&ep).await;
    let (mgr, w) = fleet_mgr_up(
        &s,
        Up {
            store: Some(store()),
            offset_ms: -5000,
            ..Up::default()
        },
        &["vehicle-01"],
    )
    .await;
    // Durable before anything is written under it, and stated.
    assert_eq!(std::fs::read(&path).unwrap(), (n + 1).to_be_bytes());
    assert_eq!(described_run(&mgr), Run::U16(n + 1).hex());
    let rev11 = w["vehicle-01"].put("rev11").await.unwrap();
    assert_eq!(rev11.stamp.run, Run::U16(n + 1), "run n + 1");
    assert!(
        order.accept_mark(ORIGIN, Mark::InBand(rev11.stamp)),
        "the consumer accepts rev 11, another run's, whatever the clock"
    );
    let rev12 = w["vehicle-01"].put("rev12").await.unwrap();
    assert!(order.accept_mark(ORIGIN, Mark::InBand(rev12.stamp)));
    assert!(
        !order.accept_mark(ORIGIN, Mark::InBand(rev11.stamp)),
        "by seq within run n + 1 from then on"
    );
    drop(w);
    mgr.close().await.unwrap();
}

/// §7 step 2, no record: each start draws a 32-bit run, its envelopes'
/// CH `0x10` and the descriptor's `run` 8 hex digits, the two starts' runs
/// apart, and the consumer accepts rev 11 whatever the clock. Likewise a
/// record at 0xFFFF, spent, left as it is, and one that cannot be read,
/// never reset.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_no_record_takes_a_random_32_bit_run() {
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let (_sub, raw) = capture(&tool, ORIGIN).await;
    let dir = tempfile::tempdir().unwrap();
    let spent = dir.path().join("spent.run");
    std::fs::write(&spent, [0xff, 0xff]).unwrap();
    let unreadable = dir.path().join("unreadable.run");
    std::fs::write(&unreadable, b"abc").unwrap();

    let mut order = ValueOrder::new();
    let mut runs = Vec::new();
    for (i, (record, offset_ms)) in [
        (None, 0),
        (None, -5000),
        (Some(&spent), 0),
        (Some(&unreadable), 0),
    ]
    .into_iter()
    .enumerate()
    {
        let s = client(&ep).await;
        let (mgr, w) = fleet_mgr_up(
            &s,
            Up {
                store: record.map(|p| Arc::new(FileRunStore::new(p)) as Arc<dyn RunStore>),
                offset_ms,
                ..Up::default()
            },
            &["vehicle-01"],
        )
        .await;
        eventually("subscribed", || async {
            w["vehicle-01"].writer().matching().await.unwrap()
        })
        .await;
        let from = raw.lock().unwrap().len();
        let rev = w["vehicle-01"].put(format!("rev{}", 10 + i)).await.unwrap();
        let (stamp, ch) = heard(&raw, from, "the put arrives").await;
        assert_eq!(stamp, rev.stamp);
        assert!(matches!(stamp.run, Run::U32(_)), "a 32-bit run: {stamp:?}");
        assert_eq!(ch, 0x10, "CH: a mutation, a 32-bit run, tag 0");
        let described = described_run(&mgr);
        assert_eq!(described.len(), 8, "8 hex digits: {described}");
        assert_eq!(described, stamp.run.hex());
        assert!(
            order.accept_mark(ORIGIN, Mark::InBand(stamp)),
            "start {i}: the consumer accepts it, whatever the clock"
        );
        runs.push(stamp.run);
        drop(w);
        mgr.close().await.unwrap();
        s.close().await.unwrap();
    }
    assert_ne!(runs[0], runs[1], "two starts, two runs");
    assert_eq!(
        std::fs::read(&spent).unwrap(),
        [0xff, 0xff],
        "left as it is"
    );
    assert_eq!(std::fs::read(&unreadable).unwrap(), b"abc", "never reset");
}

/// §7 step 3, before the wrap: the next seq set to 2^24 − 2, three writes,
/// the third under a new run at seq 0, the descriptor stating it put
/// before it; the consumer accepts the third as another run's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_the_run_changes_before_the_seq_wraps() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let tool = client(&ep).await;
    let (_desc, descriptors) = capture(&tool, "zk2/ground/fleet-mgr/@zk/instance/*").await;
    let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
    let w = &w["vehicle-01"];
    let n = mgr.minter().run();
    mgr.minter().simulate_seq(16_777_214);
    let mut order = ValueOrder::new();
    let first = w.put("a").await.unwrap();
    let second = w.put("b").await.unwrap();
    assert_eq!(
        (first.stamp.run, first.stamp.seq.get()),
        (n, 16_777_214),
        "the first at 2^24 − 2 of run n"
    );
    assert_eq!(
        (second.stamp.run, second.stamp.seq.get()),
        (n, 16_777_215),
        "the second at 2^24 − 1"
    );
    let third = w.put("c").await.unwrap();
    assert_ne!(third.stamp.run, n, "a new run");
    assert_eq!(third.stamp.seq.get(), 0, "from seq 0");
    // The owner's order: its descriptor states the new run once the third
    // is put, since the run changes, and the descriptor is put, before the
    // third's stamp is handed out.
    assert_eq!(described_run(&mgr), third.stamp.run.hex());
    eventually("the descriptor stating the new run arrives", || async {
        descriptors.lock().unwrap().iter().any(|d| {
            serde_json::from_slice::<serde_json::Value>(&d.payload().to_bytes())
                .is_ok_and(|v| v["run"] == third.stamp.run.hex())
        })
    })
    .await;
    for (v, accepted) in [(first, true), (second, true), (third, true)] {
        assert_eq!(
            order.accept_mark(ORIGIN, Mark::InBand(v.stamp)),
            accepted,
            "{v:?}"
        );
    }
    assert_eq!(mgr.minter().run(), third.stamp.run);
}

/// What a late joiner received of one key, in arrival order: each value
/// and its mark, whichever path brought it.
type Arrivals = Arc<Mutex<Vec<(String, Mark)>>>;

fn arrived(into: &Arrivals, sample: &Sample, mark: Option<Mark>) {
    into.lock().unwrap().push((
        String::from_utf8_lossy(&sample.payload().to_bytes()).into_owned(),
        mark.expect("an owner stamps every mutation in band (S1)"),
    ));
}

/// The late joiner's seed: S4's GET, each value with its in-band mark.
fn seeded(into: &Arrivals, got: &StateAnswer) {
    for cur in &got.current {
        if let Current::Value { sample, .. } = cur {
            arrived(into, sample, got.mark(cur));
        }
    }
}

/// What a consumer holds of [`ORIGIN`] after `arrivals`: §4.3's consumer
/// rule over each in order, from the first ([`ValueOrder`]).
fn consumer_holds(arrivals: &[(String, Mark)]) -> &str {
    let mut order = ValueOrder::new();
    let mut held = "";
    for (v, m) in arrivals {
        if order.accept_mark(ORIGIN, *m) {
            held = v.as_str();
        }
    }
    held
}

/// What a tool holds of [`ORIGIN`] after `arrivals`: §4.2's merge by each
/// value's in-band stamp, over each in order (observer §4.1): by seq within
/// one run, the later arrival across two.
fn tool_holds(arrivals: &[(String, Mark)]) -> Vec<&str> {
    let mut held = Held::Nothing;
    let mut kept = Vec::new();
    for (v, m) in arrivals {
        if admits_mark(held, Some(*m)) {
            held = held.after_mark(Some(*m));
            kept.push(v.as_str());
        }
    }
    kept
}

/// §7 step 4's late joiners across a restart (core §4.2, §4.3). A consumer
/// and a tool each seeded rev 10, in §10's order. The owner restarts, a
/// new instance with no record, and writes rev 11, then rev 12, under its
/// new run. The consumer orders by S7 from its first value on, its seed's
/// reply included: it accepts rev 11, another run's, then rev 12. The tool
/// keeps the later arrival across two runs: rev 11, then rev 12 (the
/// fleet's seeded subscriptions do so on the bus,
/// `zenkey-fleet/tests/seeding.rs`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_late_joiners_across_runs() {
    let (_r1, ep) = router(None).await;
    let (_ex, c) = executor(&client(&ep).await, true).await;
    let s = client(&ep).await;
    let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
    let rev10 = w["vehicle-01"].put("rev10").await.unwrap();

    // The late joiner: the subscription first, then S4's GET (core §4.2).
    let got: Arrivals = Arc::default();
    let live = Arc::clone(&got);
    let _sub = c
        .subscribe("state/plans/{vehicle}", move |d| {
            arrived(&live, &d.sample, d.mark());
        })
        .await
        .unwrap();
    seeded(&got, &answer(&c, None).await);
    // The seed's reply, and the subscription's copy of rev 10 when its put
    // reached the router after the subscription did: one mutation, one
    // stamp, whichever path brought it.
    let seeds = got.lock().unwrap().len();
    assert!(seeds >= 1, "the seed's reply");
    assert!(
        got.lock()
            .unwrap()
            .iter()
            .all(|(v, m)| v == "rev10" && *m == Mark::InBand(rev10.stamp)),
        "rev 10 alone, with its stamp"
    );

    // The restart: a new instance, no record, its clock 5 s behind.
    drop(w);
    mgr.close().await.unwrap();
    s.close().await.unwrap();
    let s = client(&ep).await;
    let (mgr, w) = fleet_mgr_up(
        &s,
        Up {
            offset_ms: -5000,
            ..Up::default()
        },
        &["vehicle-01"],
    )
    .await;
    // The new instance's writer matched before it puts (#1028): a put its
    // session has no matching subscription for yet is dropped at source.
    eventually(
        "the new instance's writer matches the subscription",
        || async { w["vehicle-01"].writer().matching().await.unwrap() },
    )
    .await;
    let rev11 = w["vehicle-01"].put("rev11").await.unwrap();
    let rev12 = w["vehicle-01"].put("rev12").await.unwrap();
    assert_ne!(rev11.stamp.run, rev10.stamp.run, "a new run");
    assert_eq!(rev12.stamp.run, rev11.stamp.run, "one run");
    eventually("rev 11 and rev 12 arrive", || async {
        got.lock().unwrap().len() == seeds + 2
    })
    .await;
    let arrivals = got.lock().unwrap().clone();
    assert_eq!(
        arrivals[seeds..]
            .iter()
            .map(|(v, _)| v.as_str())
            .collect::<Vec<_>>(),
        ["rev11", "rev12"]
    );

    // The consumer: §4.3's consumer rule over every arrival, the seed's first.
    let mut order = ValueOrder::new();
    let consumer: Vec<&str> = arrivals
        .iter()
        .filter(|(_, m)| order.accept_mark(ORIGIN, *m))
        .map(|(v, _)| v.as_str())
        .collect();
    assert_eq!(
        consumer,
        ["rev10", "rev11", "rev12"],
        "another run's value is accepted, and ordered by seq within it"
    );
    // The tool: §4.2's merge, the later arrival across two runs.
    assert_eq!(
        tool_holds(&arrivals),
        ["rev10", "rev11", "rev12"],
        "the merge keeps the later arrival across two runs"
    );
    drop(w);
    mgr.close().await.unwrap();
}

/// §7 step 5, a restart inside a late joiner's seed (core §4.2, §4.3).
/// The owner writes rev 10. A second session holds a queryable on the
/// plans that is not `complete`, and keeps each query it takes, so the
/// consumer's S4 GET stays open after the owner answered it. While it is
/// open, the owner restarts under a new run and writes rev 11, which the
/// subscription hears. Then the holder lets the query go, and the GET
/// ends. Its answer of rev 10, read when the GET ends, comes after rev 11's
/// live sample (core §4.3's residual risk "Inside a seed"). The consumer
/// accepts it, another
/// run's, and a tool keeps it, the later arrival: both hold rev 10 until
/// rev 12.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_late_joiners_across_runs_inside_the_seed() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use zenoh::query::Query;
    let (_r1, ep) = router(None).await;
    let (_ex, c) = executor(&client(&ep).await, true).await;
    let s = client(&ep).await;
    let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
    let rev10 = w["vehicle-01"].put("rev10").await.unwrap();

    // The holder: a queryable on the plans that is not `complete`. It
    // answers no query it takes and keeps each one: dropping it is its
    // final reply, which ends the GET.
    let holder = client(&ep).await;
    let held: Arc<Mutex<Vec<Query>>> = Arc::default();
    let into = Arc::clone(&held);
    let _hold = holder
        .declare_queryable("zk2/ground/fleet-mgr/mission_plan.v1/state/plans/**")
        .complete(false)
        .callback(move |q| into.lock().unwrap().push(q))
        .await
        .unwrap();
    // The router routes a GET to it: a probe's GET reaches it.
    let probe = client(&ep).await;
    eventually("the holder takes a probe's query", || async {
        let _ = probe
            .get(ORIGIN)
            .target(QueryTarget::All)
            .timeout(T)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut queries = held.lock().unwrap();
        let took = !queries.is_empty();
        queries.clear();
        took
    })
    .await;

    // The late joiner: the subscription first, then S4's GET (core §4.2).
    let got: Arrivals = Arc::default();
    let live = Arc::clone(&got);
    let _sub = c
        .subscribe("state/plans/{vehicle}", move |d| {
            arrived(&live, &d.sample, d.mark());
        })
        .await
        .unwrap();
    let open = AtomicBool::new(true);
    let seed = async {
        let answer = c
            .get_answer("state/plans/{vehicle}", None, common::SETTLE)
            .await
            .unwrap();
        open.store(false, Ordering::SeqCst);
        answer
    };
    let restart = async {
        eventually("the holder holds the GET open", || async {
            !held.lock().unwrap().is_empty()
        })
        .await;
        // The owner answered it before it restarts. The router sent the
        // GET to the owner when it sent it to the holder, so a later GET
        // that only the owner's `complete` queryable takes is answered
        // after it, on the same link.
        let rx = probe
            .get(ORIGIN)
            .target(QueryTarget::BestMatching)
            .consolidation(ConsolidationMode::None)
            .timeout(common::SETTLE)
            .await
            .unwrap();
        let first = rx.recv_async().await.expect("a reply to the probe");
        assert!(first.result().is_ok(), "the owner answers the probe");
        drop(rx);

        // The restart, inside the seed: a new instance, a new run.
        drop(w);
        mgr.close().await.unwrap();
        s.close().await.unwrap();
        let s = client(&ep).await;
        let (mgr, w) = fleet_mgr(&s, None, &["vehicle-01"]).await;
        let rev11 = w["vehicle-01"].put("rev11").await.unwrap();
        eventually("rev 11 arrives live, inside the seed", || async {
            got.lock().unwrap().iter().any(|(v, _)| v == "rev11")
        })
        .await;
        assert!(open.load(Ordering::SeqCst), "the GET is still open");
        // The holder lets its queries go: the GET ends.
        held.lock().unwrap().clear();
        (s, mgr, w, rev11)
    };
    let (answer, (_s, mgr, w, rev11)) = tokio::join!(seed, restart);
    assert_ne!(rev11.stamp.run, rev10.stamp.run, "a new run");

    // The consumer hands the seed's replies over at the GET's end, after
    // what the subscription heard during it.
    seeded(&got, &answer);
    let arrivals = got.lock().unwrap().clone();
    assert_eq!(
        arrivals.last(),
        Some(&("rev10".to_owned(), Mark::InBand(rev10.stamp))),
        "the owner's reply of rev 10 arrives last"
    );
    assert!(
        arrivals
            .iter()
            .any(|(v, m)| v == "rev11" && *m == Mark::InBand(rev11.stamp)),
        "rev 11 arrived live before it"
    );
    assert_eq!(
        consumer_holds(&arrivals),
        "rev10",
        "the consumer accepts the reply of the old run, which arrived last"
    );
    assert_eq!(
        tool_holds(&arrivals).last(),
        Some(&"rev10"),
        "the tool keeps the later arrival too"
    );

    // Until the key's next value: rev 12, under the new run.
    let rev12 = w["vehicle-01"].put("rev12").await.unwrap();
    eventually("rev 12 arrives", || async {
        got.lock()
            .unwrap()
            .iter()
            .any(|(_, m)| *m == Mark::InBand(rev12.stamp))
    })
    .await;
    let arrivals = got.lock().unwrap().clone();
    assert_eq!(consumer_holds(&arrivals), "rev12", "the consumer");
    assert_eq!(tool_holds(&arrivals).last(), Some(&"rev12"), "the tool");
    drop(w);
    mgr.close().await.unwrap();
}

/// §7 step 6, ahead: an owner 2 s ahead of its router, which stamps a
/// heartbeat, detects the drift and stops the state writes that carry a
/// Zenoh timestamp (core §4.3, "Ahead"; #978's F6): `status.v1`'s status,
/// a `freshness.v1` state with a horizon on §4.3's table, is refused, put
/// and delete; `plans/{vehicle}`, on no row, goes on, and so does an event,
/// its time unknown (core §2.6, #980's C2c).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_an_owner_ahead_stops_its_timed_state_writes() {
    let (_r1, ep) = router(None).await;
    let hb = client(&ep).await;
    let s = client(&ep).await;
    let mut cfg = config("ground/fleet-mgr");
    cfg.clock_reference = Some("clock/heartbeat".to_owned());
    let mut b = ServiceBuilder::new(&s, cfg);
    b.implement(imp("mission_plan.v1")).unwrap();
    b.implement(status_v1()).unwrap();
    b.implement(imp("journal.v1")).unwrap();
    let events = b
        .event_writer(
            &"journal.v1".parse().unwrap(),
            "events/entries",
            &Bindings::new(),
        )
        .unwrap();
    let heard = s
        .declare_subscriber("zk2/ground/fleet-mgr/journal.v1/events/entries/*")
        .with(flume::unbounded())
        .await
        .unwrap();
    b.minter().simulate_offset(2000);
    let w = b
        .declare_state_writer(&plan(), "state/plans/{vehicle}", &vehicle("vehicle-01"))
        .await
        .unwrap();
    let status = b
        .declare_state_writer(
            &"status.v1".parse().unwrap(),
            "state/status",
            &Bindings::new(),
        )
        .await
        .unwrap();
    assert!(status.writer().is_stamped(), "on §4.3's table");
    assert!(!w.writer().is_stamped(), "on no row");
    let mgr = b.start().await.unwrap();
    eventually("the drift is detected", || async {
        hb.put("clock/heartbeat", "tick").await.unwrap();
        mgr.minter().is_ahead()
    })
    .await;
    assert!(
        matches!(status.put("late").await, Err(zenkey::Error::ClockAhead)),
        "a state write that carries a Zenoh timestamp stops"
    );
    assert!(matches!(
        status.delete().await,
        Err(zenkey::Error::ClockAhead)
    ));
    let put = w
        .put("on time")
        .await
        .expect("an untimed state write goes on");
    assert!(put.time.is_none(), "it carries no Zenoh timestamp");
    w.delete().await.expect("and so does its delete");
    let key = events.put("ahead").await.expect("an event goes on");
    let sample = tokio::time::timeout(T, heard.recv_async())
        .await
        .expect("the event is heard")
        .unwrap();
    assert_eq!(sample.key_expr().as_str(), key);
    assert_eq!(
        zenkey::inband::event_time(&sample),
        Some(zenkey::model::collection::Time::Unknown),
        "its time unknown while the guard holds"
    );
    assert!(mgr.minter().is_ahead(), "the guard still holds");
}

/// §9: an archive that holds v1 (seq a of run n) and its delete (seq b of
/// run n) refuses a late put of the same run at an older seq, answering
/// the tombstone; and records a late put of another run, answering it:
/// nothing orders two runs (core §4.3's residual risk, as an archive
/// meets it).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_an_archives_backend_refuses_an_outdated_put() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let (_mgr, w) = fleet_mgr(&owner, None, &["vehicle-01"]).await;
    let arch_s = client(&ep).await;
    let arch_watch = client(&ep).await;
    let arch = vehicle_archive(&arch_s, &arch_watch, &[]).await;
    let w = &w["vehicle-01"];
    eventually("recorded v1", || async {
        w.put("v1").await.unwrap();
        arch.confirmed(ORIGIN) == Some(true)
    })
    .await;
    let del = w.delete().await.unwrap();
    let last_known = || async {
        archive::last_known(&arch_s, &common::addr("vehicle-01/archive"), ORIGIN, T)
            .await
            .unwrap()
    };
    eventually("recorded the delete", || async {
        last_known().await.is_some_and(|l| l.value.is_none())
    })
    .await;
    let late = |stamp: InBand, record: &'static [u8]| {
        let owner = owner.clone();
        async move {
            let bytes = Envelope {
                form: wire::Form::Mutation,
                tag: wire::Tag::NONE,
                stamp,
                record,
            }
            .write();
            owner
                .put(ORIGIN, bytes)
                .encoding(Encoding::ZENOH_BYTES)
                .await
                .unwrap();
        }
    };
    // 1. An older put of the delete's run, v0 at seq 0, reaches it late.
    assert!(del.stamp.seq.get() > 0);
    late(
        InBand {
            run: del.stamp.run,
            seq: wire::Seq::new(0).unwrap(),
        },
        b"v0",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let l = last_known().await.unwrap();
    assert!(l.value.is_none(), "a tombstone, never the older value");
    // 2. A put of another run, v9 under run m, reaches it late: recorded.
    let m = match del.stamp.run {
        Run::U32(v) => Run::U32(v ^ 1),
        Run::U16(v) => Run::U16(v ^ 1),
    };
    late(
        InBand {
            run: m,
            seq: wire::Seq::new(0).unwrap(),
        },
        b"v9",
    )
    .await;
    eventually("the other run's put is recorded and answered", || async {
        last_known()
            .await
            .is_some_and(|l| l.value.as_deref() == Some(&b"v9"[..]))
    })
    .await;
}

/// #698 (§2.3, S2): a state value and an event occurrence carry the
/// attachment their contract declares. The subscriber receives it with the
/// sample, and the owner's GET answers carry it with the value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn state_and_events_carry_their_attachments() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let notes: IfaceId = "notes.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("h1/notes"));
    b.implement(imp("notes.v1")).unwrap();
    let none = Bindings::new();
    let last = b
        .declare_state_writer(&notes, "state/last", &none)
        .await
        .unwrap();
    let posted = b.event_writer(&notes, "events/posted", &none).unwrap();
    let _svc = b.start().await.unwrap();

    // Each sample's payload, and its attachment if any.
    type Seen = Vec<(String, Option<Vec<u8>>)>;
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let s = Arc::clone(&seen);
    let _sub = tool
        .declare_subscriber("zk2/h1/notes/notes.v1/**")
        .callback(move |sample: Sample| {
            s.lock().unwrap().push((
                String::from_utf8_lossy(&common::record(&sample)).into_owned(),
                sample.attachment().map(|a| a.to_bytes().into_owned()),
            ));
        })
        .await
        .unwrap();
    let probe = owner
        .declare_publisher("zk2/h1/notes/notes.v1/state/last")
        .await
        .unwrap();
    eventually("the tool's subscriber is matched", || async {
        probe.matching_status().await.unwrap().matching()
    })
    .await;
    drop(probe);

    last.put_with("hello", Some("by-alice")).await.unwrap();
    posted.put_with("posted", Some("by-bob")).await.unwrap();
    eventually("both samples arrive", || async {
        seen.lock().unwrap().len() == 2
    })
    .await;
    let got = seen.lock().unwrap().clone();
    assert!(
        got.contains(&("hello".to_owned(), Some(b"by-alice".to_vec()))),
        "{got:?}"
    );
    assert!(
        got.contains(&("posted".to_owned(), Some(b"by-bob".to_vec()))),
        "{got:?}"
    );

    // S2 (core 0.100, #977's E3): the owner answers its member read by
    // snapshot, whose entry is the record alone: a snapshot carries no
    // attachment (core §7.5), so the GET's reader gets the value without it.
    let replies = raw_get(&tool, "zk2/h1/notes/notes.v1/state/last").await;
    assert_eq!(replies.len(), 1, "{replies:?}");
    assert_eq!(common::record(&replies[0]), b"hello");
    assert_eq!(replies[0].attachment(), None, "a snapshot's entry has none");
}

/// Core §4.4 (0.16): an archive is never tokenless, because consumers and
/// tools find it by its interface token (S6); one configured so is refused
/// at §8.2 step 2 (0.17).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_is_never_tokenless() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let refused = Archive::start(
        &s,
        ArchiveConfig {
            service: config("vehicle-01/archive").tokenless("archive.v1".parse().unwrap()),
            records: vec![recorded(ORIGIN)],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await;
    let Err(e) = refused else {
        panic!("a tokenless archive starts")
    };
    assert!(e.to_string().contains("never tokenless"), "{e}");
}

/// An archive reads its owners' presence on its own session, so its watch
/// session must be another (#830, core §8.1, 0.37): its own session as one
/// is refused at the start, nothing declared.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_refuses_its_own_session_as_its_watch() {
    let (_r1, ep) = router(None).await;
    let s = client(&ep).await;
    let refused = Archive::start(
        &s,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![recorded(ORIGIN)],
            peers: vec![],
            unconfirmed_horizon: None,
            watch: Some(s.clone()),
        },
    )
    .await
    .err()
    .expect("refused");
    assert!(
        matches!(refused, zenkey::Error::ReadBesideSubscriber { .. }),
        "{refused}"
    );
    assert!(!zenkey::presence::watching(&s));
}

/// Core §2.5 (#976): an owner's advanced publisher numbers its samples, so
/// a `history` resource declares on a session without timestamping (zenoh-ext
/// refuses a cache whose samples carry neither a number nor a timestamp),
/// and a late subscriber gets its history once, each sample under its
/// envelope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_history_resource_declares_without_timestamping() {
    use zenoh_ext::{AdvancedSubscriberBuilderExt, HistoryConfig};
    let (_r1, ep) = router_with(None, &[("timestamping/enabled", "false")]).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;
    let ticker: IfaceId = "ticker.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("lab/ticker"));
    b.implement(imp("ticker.v1")).unwrap();
    let w = b
        .declare_writer(&ticker, "stream/tick", &Bindings::new())
        .await
        .expect("declared on a session without timestamping");
    assert!(!b.minter().session_timestamping());
    let _svc = b.start().await.unwrap();
    for v in ["t0", "t1", "t2"] {
        w.put(v).await.unwrap();
    }
    let got: Raw = Arc::default();
    let into = Arc::clone(&got);
    let _late = tool
        .declare_subscriber("zk2/lab/ticker/ticker.v1/stream/tick")
        .history(HistoryConfig::default().detect_late_publishers())
        .callback(move |s: Sample| into.lock().unwrap().push(s))
        .await
        .unwrap();
    eventually("the history arrives", || async {
        got.lock().unwrap().len() >= 3
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let got = got.lock().unwrap().clone();
    let read: Vec<(Vec<u8>, u32)> = got
        .iter()
        .map(|s| {
            let (stamp, record) = sealed(s);
            (record, stamp.seq.get())
        })
        .collect();
    assert_eq!(
        read,
        [
            (b"t0".to_vec(), 0),
            (b"t1".to_vec(), 1),
            (b"t2".to_vec(), 2)
        ],
        "each sample once, in order, under the key's seqs"
    );
    assert!(
        got.iter().all(|s| s.timestamp().is_none()),
        "no Zenoh timestamp"
    );
}

/// Core §8.2's posture, as the runtime reads it from the session's
/// configuration: a client's and a peer's timestamping off by default, a
/// router's on, and a stated value for the session's mode wins. A session
/// that stamps has every write given the owner's Zenoh timestamp, so its
/// GET answers carry the one its put did (S2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_session_posture_is_read_from_its_config() {
    let (r, ep) = router(None).await;
    let open = |extra: &'static [(&'static str, &'static str)]| {
        let ep = ep.clone();
        async move {
            let mut c = zenoh::Config::default();
            c.insert_json5("mode", "\"client\"").unwrap();
            c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
                .unwrap();
            c.insert_json5("scouting/multicast/enabled", "false")
                .unwrap();
            for (k, v) in extra {
                c.insert_json5(k, v).unwrap();
            }
            zenoh::open(c).await.unwrap()
        }
    };
    let stamps = |s: &zenoh::Session| {
        ServiceBuilder::new(s, config("ground/fleet-mgr"))
            .minter()
            .session_timestamping()
    };
    assert!(!stamps(&open(&[]).await), "a client's default: off");
    assert!(stamps(&r), "a router's default: on");
    let on = open(&[("timestamping/enabled", "true")]).await;
    assert!(stamps(&on), "stated");
    let by_mode = open(&[(
        "timestamping/enabled",
        "{ router: false, peer: false, client: true }",
    )])
    .await;
    assert!(stamps(&by_mode), "stated for its mode");

    // A stamping session: the put carries the owner's timestamp. Its GET is
    // answered by snapshot, which carries none (core 0.100, #977's E3): S2's
    // stamp rules bind a per-key resource's replies alone.
    let (mgr, w) = fleet_mgr(&on, None, &["vehicle-01"]).await;
    let tool = client(&ep).await;
    let put = w["vehicle-01"].put("v1").await.unwrap();
    let t = put.time.expect("every write given the owner's timestamp");
    assert_eq!(t.get_id().to_string(), on.zid().to_string());
    let reply = raw_get(&tool, ORIGIN).await;
    assert_eq!(reply.len(), 1);
    assert_eq!(reply[0].timestamp(), None, "S2: a snapshot's entry");
    assert_eq!(common::inband(&reply[0]), Some(put.stamp));
    drop(w);
    mgr.close().await.unwrap();
}

// --- §14: a collection's snapshot, its pages (#977's E3) --------------------

const PLANS: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans";

/// The owner of §14: 32 plans of 110 bytes each under one run, its pages
/// cut at the default 1,024 bytes.
async fn fleet_of_32(
    s: &zenoh::Session,
    window_s: Option<u64>,
) -> (Service, BTreeMap<String, StateWriter>) {
    let mut names: Vec<String> = (0..31).map(|i| format!("v{i:02}")).collect();
    names.push("vehicle-01".to_owned());
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (svc, w) = fleet_mgr(s, window_s, &refs).await;
    for (v, writer) in &w {
        let record = format!("{v:-<110}");
        writer.put(record.into_bytes()).await.unwrap();
    }
    (svc, w)
}

/// Every page a GET's replies hold, read, with the key it rode.
fn pages_of(replies: &[Sample]) -> Vec<(String, zenkey::model::collection::Page)> {
    replies
        .iter()
        .filter_map(|s| match zenkey::inband::read(s.clone()) {
            zenkey::inband::Read::Snapshot { page: Ok(p), .. } => {
                Some((s.key_expr().as_str().to_owned(), *p))
            }
            _ => None,
        })
        .collect()
}

/// §14 steps 1, 2 and 4: a collection read is answered with one snapshot
/// on the snapshot key, in pages of at most 1,024 bytes whose more bits
/// chain them, one run, asof and time; a member read with each member's
/// own snapshot, its one entry's id empty; a concrete key at no member with
/// one page of no entry; and under `Latest`, the control, one page of the
/// four arrives. The consumer's GET (`plans/**`, `None`) holds the snapshot
/// complete and every plan.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s14_a_collection_read_pages_and_member_reads() {
    let (_r1, ep) = router(None).await;
    let (mgr, _w) = fleet_of_32(&client(&ep).await, None).await;
    let tool = client(&ep).await;

    // Step 1: the collection read.
    let whole = raw_pages(&tool, &format!("{PLANS}/**")).await;
    let pages = pages_of(&whole);
    assert_eq!(pages.len(), 4, "{} replies", whole.len());
    let mut indices: Vec<u32> = pages.iter().map(|(_, p)| p.index).collect();
    indices.sort_unstable();
    assert_eq!(indices, [0, 1, 2, 3]);
    let run = mgr.minter().run();
    for (key, p) in &pages {
        assert_eq!(key, PLANS, "a snapshot rides its snapshot key");
        assert!(p.size() <= 1024, "page {}: {} bytes", p.index, p.size());
        assert_eq!(p.more, p.index != 3, "page {}: more", p.index);
        assert_eq!(
            (p.run, p.asof, p.time, p.coll),
            (run, pages[0].1.asof, pages[0].1.time, 0)
        );
        for e in &p.live {
            assert_eq!(
                e.confirmation,
                zenkey::model::collection::Confirmation::None
            );
        }
    }
    let ps: Vec<_> = pages.iter().map(|(_, p)| p.clone()).collect();
    let snap = zenkey::model::collection::Snapshot::complete(&ps).expect("complete");
    assert_eq!(snap.live.len(), 32);
    assert!(snap.tombs.is_empty());

    // The consumer's GET: the collection read, held complete.
    let (_ex, c) = executor(&client(&ep).await, false).await;
    let got = eventually_answer(&c, |a| a.current.len() == 32).await;
    assert!(got.complete, "{:?}", got.snapshots);
    assert_eq!(got.snapshots.len(), 1);
    assert_eq!(got.snapshots[0].key, PLANS);
    assert!(got.snapshots[0].complete());
    assert_eq!(got.snapshots[0].pages.len(), 4);

    // Step 2: member reads.
    let one = raw_pages(&tool, &format!("{PLANS}/v07")).await;
    let one = pages_of(&one);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].0, format!("{PLANS}/v07"));
    assert_eq!(one[0].1.live.len(), 1);
    assert_eq!(one[0].1.live[0].id, "", "a member's own snapshot");
    let each = pages_of(&raw_pages(&tool, &format!("{PLANS}/*")).await);
    assert_eq!(each.len(), 32, "a snapshot per member");
    assert!(each.iter().all(|(k, p)| k != PLANS && p.live.len() == 1));
    let none = pages_of(&raw_pages(&tool, &format!("{PLANS}/v99")).await);
    assert_eq!(none.len(), 1, "a concrete key at no member");
    assert!(none[0].1.live.is_empty() && none[0].1.tombs.is_empty());
    assert!(!none[0].1.more);

    // Step 4, the control: `Latest` keeps one reply per key, and the four
    // pages share theirs. The one state GET under `Latest` left (#977's
    // E4): the scenario's control, which shows what `None` prevents (core
    // §4.2, "Why `None`"); no reader GETs so.
    let latest = raw_get_under(&tool, &format!("{PLANS}/**"), ConsolidationMode::Latest).await;
    assert_eq!(pages_of(&latest).len(), 1, "one page of the four");
}

/// GETs the consumer's answer until `done` holds of it.
async fn eventually_answer(c: &Consumer, done: impl Fn(&StateAnswer) -> bool) -> StateAnswer {
    let mut last = None;
    for _ in 0..50 {
        let a = c
            .get_answer("state/plans/{vehicle}", None, T)
            .await
            .unwrap();
        if done(&a) {
            return a;
        }
        last = Some(a);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("never: {last:?}");
}

/// §2 and §14 steps 5 and 6 at the consumer: a delete within the window is
/// a tombstone in the snapshot, a deletion with its stamp; beyond it the
/// collection's snapshot omits the member, and a member read of its key
/// draws one page of no entry, the owner's word that it has no value
/// (`StateGet::Answered` with no key, never `Silent`). Once every plan is
/// deleted and the window passes, the collection's snapshot is one page of
/// no entry, complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s14_a_delete_within_the_window_and_beyond_it() {
    let (_r1, ep) = router(None).await;
    let (_mgr, w) = fleet_mgr(&client(&ep).await, Some(1), &["a", "b"]).await;
    let (_ex, c) = executor(&client(&ep).await, false).await;
    w["a"].put("pa").await.unwrap();
    w["b"].put("pb").await.unwrap();
    let del = w["b"].delete().await.unwrap();
    let b = format!("{PLANS}/b");
    let within = eventually_answer(&c, |a| a.current.len() == 2).await;
    assert!(
        within
            .current
            .iter()
            .any(|x| x.key() == b && matches!(x, Current::Deleted { .. }))
    );
    assert_eq!(
        within.inband.get(&b),
        Some(&del.stamp),
        "the tombstone's stamp"
    );
    // Beyond the window: the tombstone is gone, and the snapshot omits b.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let beyond = c
        .get_answer("state/plans/{vehicle}", None, T)
        .await
        .unwrap();
    assert!(beyond.complete);
    assert_eq!(beyond.current.len(), 1, "{beyond:?}");
    assert_eq!(beyond.current[0].key(), format!("{PLANS}/a"));
    let at_b = c
        .get_answer("state/plans/{vehicle}", Some(&vehicle("b")), T)
        .await
        .unwrap();
    assert!(at_b.current.is_empty());
    assert_eq!(at_b.snapshots.len(), 1);
    assert!(at_b.snapshots[0].complete());
    assert!(
        matches!(StateGet::from(at_b), StateGet::Answered(v) if v.is_empty()),
        "a snapshot of no entry is the owner's word"
    );
    // §14 step 6: an empty collection.
    w["a"].delete().await.unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let empty = c
        .get_answer("state/plans/{vehicle}", None, T)
        .await
        .unwrap();
    assert!(empty.current.is_empty());
    assert_eq!(empty.snapshots.len(), 1);
    assert!(empty.snapshots[0].complete(), "one page of no entry");
    assert!(matches!(StateGet::from(empty), StateGet::Answered(v) if v.is_empty()));
}

/// `health.v1` revision 1.0, as published in
/// `spec/profiles/.history/health.v1/`: its status confirmed by
/// `freshness.v1`, so on core §4.3's table, answered per key.
fn health_v1_0() -> zenkey::Implementation {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../spec/profiles/.history/health.v1/\
         e9dbcdcb2fc325ca3121fdcacf7badd0091f62d97ab4c7d7a5f022dd8d6671df.bundle.json",
    );
    zenkey::Implementation::from_bundle(&std::fs::read(path).unwrap()).unwrap()
}

/// §15 step 2 (core §4.2, 0.102): whether a member is per key is read from
/// the owner's revision. The owner serves `status` (ttl 60) under
/// `health.v1` 1.0, which uses `freshness.v1` and which its descriptor
/// names; T holds 1.1, which uses `freshness.v2` instead and answers the
/// status by snapshot. The owner deletes `status`, and T's GET within W
/// merges its `reply_del` as a deletion: by 1.1 alone it would be a stray.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s15_an_owner_on_an_earlier_revision_answers_per_key() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let ts = client(&ep).await;
    let earlier = health_v1_0();
    let later = zenkey::health::implementation();
    let status = "zk2/lab/svc/health.v1/state/status";
    // The two revisions read the status apart.
    assert!(
        !zenkey::model::collection::stray_reply_del(earlier.contract(), status),
        "by 1.0, a per-key resource"
    );
    assert!(
        zenkey::model::collection::stray_reply_del(later.contract(), status),
        "by 1.1, a member answered by snapshot"
    );
    let mut b = ServiceBuilder::new(&os, config("lab/svc"));
    b.implement(earlier.clone()).unwrap();
    b.expose(&zenkey::health::iface(), zenkey::health::CHECKS)
        .unwrap()
        .expose(&zenkey::health::iface(), zenkey::health::FAULTS)
        .unwrap();
    let w = b
        .declare_state_writer(&zenkey::health::iface(), "state/status", &Bindings::new())
        .await
        .unwrap();
    let svc = b.start().await.unwrap();
    let d: serde_json::Value = serde_json::from_slice(&svc.descriptor_bytes()).unwrap();
    let named = d["interfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["iface"] == "health.v1")
        .map(|e| e["contract"].clone());
    assert_eq!(
        named,
        Some(serde_json::json!(earlier.fingerprint().to_string())),
        "the descriptor names 1.0"
    );
    w.put("serving").await.unwrap();
    let del = w.delete().await.unwrap();
    // T, a consumer on 1.1, GETs the interface's state within W.
    let t =
        Consumer::for_tool(&ts, later.shared_contract(), &["lab/svc"], &BTreeMap::new()).unwrap();
    let a = eventually_answered(&t, status).await;
    assert!(
        !a.undecodable.contains_key(status),
        "no stray: the owner's revision answers it per key: {a:?}"
    );
    assert!(
        a.current
            .iter()
            .any(|c| c.key() == status && matches!(c, Current::Deleted { .. })),
        "merged as a deletion: {a:?}"
    );
    assert_eq!(a.inband.get(status), Some(&del.stamp), "under its stamp");
    svc.close().await.unwrap();
}

/// T's GET of the status, once the owner answers it.
async fn eventually_answered(t: &Consumer, status: &str) -> StateAnswer {
    let mut last = None;
    for _ in 0..50 {
        let a = t.get_answer("state/status", None, T).await.unwrap();
        if a.current.iter().any(|c| c.key() == status) || a.undecodable.contains_key(status) {
            return a;
        }
        last = Some(a);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the owner never answered the status: {last:?}")
}

/// §10's boundary, at the owner: a snapshot's asof is the last seq the
/// owner minted, each live entry its member's last put, and a put after the
/// answer carries a seq above asof, which the omission rule keeps.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_a_snapshots_asof_is_the_last_seq_minted() {
    let (_r1, ep) = router(None).await;
    let (_mgr, w) = fleet_mgr(&client(&ep).await, None, &["a", "b", "d"]).await;
    let tool = client(&ep).await;
    w["a"].put("v1").await.unwrap();
    w["b"].put("v1").await.unwrap();
    let del = w["b"].delete().await.unwrap();
    let d = w["d"].put("v1").await.unwrap();
    let pages = pages_of(&raw_pages(&tool, &format!("{PLANS}/**")).await);
    assert_eq!(pages.len(), 1);
    let p = &pages[0].1;
    assert_eq!(
        p.asof, d.stamp.seq,
        "nothing in flight: the last seq minted"
    );
    assert_eq!(p.tombs.len(), 1);
    assert_eq!(p.tombs[0].seq, del.stamp.seq);
    let next = w["a"].put("v2").await.unwrap();
    assert!(next.stamp.seq > p.asof, "a later put is above asof");
    let ps = vec![p.clone()];
    let held = [zenkey::model::seed::HeldMember {
        key: "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/c",
        mark: Some(Mark::InBand(next.stamp)),
        heard_after: true,
    }];
    let c = contract("mission_plan.v1");
    assert!(
        zenkey::model::collection::omit(&c, PLANS, &ps, held).is_empty(),
        "a value above asof is newer than the snapshot"
    );
}

/// §5 at an archive: absence from the owner's complete snapshot is positive
/// evidence (§4.4). The archive records `a` and `b` under the owner's first
/// instance; a second instance, under another run, puts `a` alone. Its
/// alignment's collection read omits `b`, which the archive heard before it
/// asked, so `b` is dropped, where before #977's E3 it stayed unconfirmed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_an_archive_drops_what_the_owners_complete_snapshot_omits() {
    let (_r1, ep) = router(None).await;
    let owner_s = client(&ep).await;
    let s = client(&ep).await;
    let reader = client(&ep).await;
    let (mgr, w) = fleet_mgr(&owner_s, None, &["a", "b"]).await;
    w["a"].put("pa").await.unwrap();
    w["b"].put("pb").await.unwrap();
    let archive = Archive::start(
        &s,
        ArchiveConfig {
            service: config("ground/archive"),
            records: vec![Recorded {
                owner: common::addr("ground/fleet-mgr"),
                selector: format!("{PLANS}/*"),
                implementation: imp("mission_plan.v1"),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .unwrap();
    let a = common::addr("ground/archive");
    let b = format!("{PLANS}/b");
    eventually("the archive records b", || async {
        archive.align(T).await.unwrap();
        archive::last_known(&reader, &a, &b, T)
            .await
            .unwrap()
            .is_some()
    })
    .await;
    drop(w);
    mgr.close().await.unwrap();
    let (_mgr2, w2) = fleet_mgr(&owner_s, None, &["a"]).await;
    w2["a"].put("pa2").await.unwrap();
    eventually("b is dropped on the owner's word", || async {
        archive.align(T).await.unwrap();
        archive::last_known(&reader, &a, &b, T)
            .await
            .unwrap()
            .is_none()
    })
    .await;
    let lk = archive::last_known(&reader, &a, &format!("{PLANS}/a"), T)
        .await
        .unwrap()
        .expect("a stands");
    assert_eq!(lk.value.as_deref(), Some(&b"pa2"[..]));
}
