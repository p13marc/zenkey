//! The zk2c envelope as the runtime's readers read it (core §4.3, §7.5;
//! #976): these suites put envelopes from a raw session, under
//! `zenoh/bytes` on a state key, so each case can hold any run and seq,
//! and show what the readers make of them. A client session stamps
//! nothing and the router stamps every put it routes (core §4.1), so each
//! sample here also carries a Zenoh timestamp later than the last: the
//! order shown is the in-band one, never the router's (observer §4.4: off
//! §4.3's table, a router's stamp is expected).

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{T, client, config, contract, eventually, eventually_some, imp, router};
use zenkey::archive::{self, Archive, ArchiveConfig, Recorded};
use zenkey::consumer::{Consumer, Delivery};
use zenkey::model::wire::{DeleteStamp, Envelope, Run, Seq, Stamp, Tag};
use zenkey::state::{Current, ValueOrder};
use zenoh::Wait;
use zenoh::bytes::Encoding;

const ORIGIN: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";
const OTHER: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-02";
const PLANS: &str = "state/plans/{vehicle}";

fn stamp(run: Run, seq: u32) -> Stamp {
    Stamp {
        run,
        seq: Seq::new(seq).expect("a seq"),
    }
}

/// A mutation's bytes: `CH · run · seq · record`.
fn enveloped(run: Run, seq: u32, record: &str) -> Vec<u8> {
    Envelope {
        form: zenkey::model::wire::Form::Mutation,
        tag: Tag::NONE,
        stamp: stamp(run, seq),
        record: record.as_bytes(),
    }
    .write()
}

async fn put(s: &zenoh::Session, key: &str, run: Run, seq: u32, record: &str) {
    s.put(key, enveloped(run, seq, record))
        .encoding(Encoding::ZENOH_BYTES)
        .await
        .unwrap();
}

/// A delete whose attachment is its envelope, `CH · run · seq`.
async fn delete(s: &zenoh::Session, key: &str, run: Run, seq: u32) {
    s.delete(key)
        .attachment(
            DeleteStamp {
                stamp: stamp(run, seq),
            }
            .write(),
        )
        .await
        .unwrap();
}

fn tool(s: &zenoh::Session) -> Consumer {
    Consumer::for_tool(
        s,
        Arc::new(contract("mission_plan.v1")),
        &["ground/fleet-mgr"],
        &Default::default(),
    )
    .unwrap()
}

/// What a subscription delivered of [`ORIGIN`]: each record and its
/// in-band stamp. The subscription is probed on [`OTHER`].
type Delivered = Arc<Mutex<Vec<Delivery>>>;

fn of_origin(got: &Delivered) -> Vec<Delivery> {
    got.lock()
        .unwrap()
        .iter()
        .filter(|d| d.sample.key_expr().as_str() == ORIGIN)
        .cloned()
        .collect()
}

fn text(d: &Delivery) -> String {
    String::from_utf8(d.sample.payload().to_bytes().into_owned()).unwrap()
}

/// The consumer delivers an envelope's record, with its run and seq; a
/// sample with no envelope as it arrived; and a refused envelope never,
/// counted undecodable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_consumer_delivers_the_record_and_counts_a_refused_envelope() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let reader = client(&ep).await;
    let got: Delivered = Arc::default();
    let sink = Arc::clone(&got);
    let consumer = tool(&reader);
    let sub = consumer
        .subscribe(PLANS, move |d| sink.lock().unwrap().push(d))
        .await
        .unwrap();
    eventually("the subscription is heard", || async {
        put(&raw, OTHER, Run::U16(1), 0, "probe").await;
        !got.lock().unwrap().is_empty()
    })
    .await;

    put(&raw, ORIGIN, Run::U16(7), 3, "a plan").await;
    // Today's JSON under `zenoh/bytes`: `{` is a reserved form. Refused,
    // never read as a payload of today's wire.
    raw.put(ORIGIN, r#"{"plan":1}"#)
        .encoding(Encoding::ZENOH_BYTES)
        .await
        .unwrap();
    // Today's wire: the predefined Encoding, read as it is.
    raw.put(ORIGIN, "plain")
        .encoding(Encoding::TEXT_PLAIN)
        .await
        .unwrap();
    eventually("both delivered", || async { of_origin(&got).len() == 2 }).await;
    let got = of_origin(&got);
    assert_eq!(text(&got[0]), "a plan", "the record, without its envelope");
    assert_eq!(got[0].inband, Some(stamp(Run::U16(7), 3)));
    assert!(got[0].stamp.is_some(), "the router's Zenoh timestamp stays");
    assert_eq!(
        got[0].mark(),
        Some(zenkey::model::seed::Mark::InBand(stamp(Run::U16(7), 3))),
        "ordered by its in-band stamp"
    );
    assert_eq!(text(&got[1]), "plain");
    assert_eq!(got[1].inband, None);
    assert_eq!(sub.undecodable(), 1, "the refused envelope, counted");
}

/// `ValueOrder` orders an envelope's values by seq within a run and accepts
/// a value of another run, whichever the router's timestamps say.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn value_order_orders_within_a_run_and_accepts_across_runs() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let reader = client(&ep).await;
    let got: Delivered = Arc::default();
    let sink = Arc::clone(&got);
    let consumer = tool(&reader);
    let _sub = consumer
        .subscribe(PLANS, move |d| sink.lock().unwrap().push(d))
        .await
        .unwrap();
    eventually("the subscription is heard", || async {
        put(&raw, OTHER, Run::U16(1), 0, "probe").await;
        !got.lock().unwrap().is_empty()
    })
    .await;

    let sent = [
        (Run::U16(1), 1, "a"),
        (Run::U16(1), 3, "c"),
        // Older within the run: refused, though the router stamped it later.
        (Run::U16(1), 2, "b"),
        // The same seq: the same value, not a new one.
        (Run::U16(1), 3, "c again"),
        // Another run: accepted, as another id is (§4.3).
        (Run::U32(0xdead_beef), 0, "d"),
        (Run::U32(0xdead_beef), 1, "e"),
        // The old run again: accepted too, §4.3's residual risk.
        (Run::U16(1), 9, "f"),
    ];
    for (run, seq, v) in sent {
        put(&raw, ORIGIN, run, seq, v).await;
    }
    eventually("every value delivered", || async {
        of_origin(&got).len() == sent.len()
    })
    .await;
    let mut order = ValueOrder::new();
    let accepted: Vec<String> = of_origin(&got)
        .iter()
        .filter(|d| order.accept_mark(ORIGIN, d.mark().expect("a mark")))
        .map(text)
        .collect();
    assert_eq!(accepted, ["a", "c", "d", "e", "f"]);
}

/// S4's GET: an enveloped reply's record and stamp, a `reply_del` whose
/// attachment is a delete's envelope, and a refused envelope left out,
/// undecodable.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_state_get_reads_enveloped_replies() {
    let (_r, ep) = router(None).await;
    let owner = client(&ep).await;
    let reader = client(&ep).await;
    let vehicle = |n: u32| format!("zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-0{n}");
    let _q = owner
        .declare_queryable("zk2/ground/fleet-mgr/mission_plan.v1/state/**")
        .callback(move |q| {
            q.reply(vehicle(1), enveloped(Run::U16(4), 10, "kept"))
                .encoding(Encoding::ZENOH_BYTES)
                .wait()
                .unwrap();
            q.reply_del(vehicle(2))
                .attachment(
                    DeleteStamp {
                        stamp: stamp(Run::U16(4), 11),
                    }
                    .write(),
                )
                .wait()
                .unwrap();
            q.reply(vehicle(3), vec![0xe0, 0, 0])
                .encoding(Encoding::ZENOH_BYTES)
                .wait()
                .unwrap();
        })
        .await
        .unwrap();
    let consumer = tool(&reader);
    let answer = eventually_some("the owner answers", || async {
        let a = consumer.get_answer(PLANS, None, T).await.unwrap();
        (a.current.len() + a.undecodable.len() == 3).then_some(a)
    })
    .await;
    let value = answer
        .current
        .iter()
        .find(|c| c.key() == vehicle(1))
        .unwrap();
    let Current::Value { sample, .. } = value else {
        panic!("{value:?}")
    };
    assert_eq!(&*sample.payload().to_bytes(), b"kept");
    assert_eq!(
        answer.mark(value),
        Some(zenkey::model::seed::Mark::InBand(stamp(Run::U16(4), 10)))
    );
    let deleted = answer
        .current
        .iter()
        .find(|c| c.key() == vehicle(2))
        .unwrap();
    assert!(matches!(deleted, Current::Deleted { .. }), "{deleted:?}");
    assert_eq!(
        answer.inband.get(&vehicle(2)),
        Some(&stamp(Run::U16(4), 11)),
        "a reply_del's stamp, from its attachment"
    );
    assert_eq!(
        answer.undecodable.get(&vehicle(3)).map(|r| r.token()),
        Some("form"),
        "form 7, a snapshot, is reserved"
    );
}

/// A plan's record: the test contract's type is `text/plain`.
fn plan(revision: u64) -> Vec<u8> {
    format!("plan {revision}").into_bytes()
}

/// A mutation whose record is `record`'s bytes.
fn sealed(run: Run, seq: u32, record: &[u8]) -> Vec<u8> {
    Envelope {
        form: zenkey::model::wire::Form::Mutation,
        tag: Tag::NONE,
        stamp: stamp(run, seq),
        record,
    }
    .write()
}

type Plans = zenkey::codec::Raw;

/// The typed GET keeps what orders each key (core §4.3's consumer rule, a
/// late joiner's seed replies included): `typed::get_answer` holds each
/// key's in-band stamp beside its decoded record, a `reply_del`'s from its
/// attachment, and the keys whose envelope was refused; `typed::get` is
/// the same answer's keys alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_typed_get_keeps_each_keys_in_band_stamp() {
    use zenkey::typed::{self, Current as Typed, StateGet};
    let (_r, ep) = router(None).await;
    let owner = client(&ep).await;
    let reader = client(&ep).await;
    let vehicle = |n: u32| format!("zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-0{n}");
    let _q = owner
        .declare_queryable("zk2/ground/fleet-mgr/mission_plan.v1/state/**")
        .callback(move |q| {
            q.reply(vehicle(1), sealed(Run::U32(0xdead_beef), 10, &plan(7)))
                .encoding(Encoding::ZENOH_BYTES)
                .wait()
                .unwrap();
            q.reply_del(vehicle(2))
                .attachment(
                    DeleteStamp {
                        stamp: stamp(Run::U32(0xdead_beef), 11),
                    }
                    .write(),
                )
                .wait()
                .unwrap();
            q.reply(vehicle(3), vec![0xe0, 0, 0])
                .encoding(Encoding::ZENOH_BYTES)
                .wait()
                .unwrap();
        })
        .await
        .unwrap();
    let consumer = tool(&reader);
    let answer = eventually_some("the owner answers", || async {
        let a = typed::get_answer::<Plans>(&consumer, PLANS, None, T)
            .await
            .unwrap();
        (a.current.len() + a.undecodable.len() == 3).then_some(a)
    })
    .await;
    assert!(answer.complete);
    let value = answer
        .current
        .iter()
        .find(|c| c.key() == vehicle(1))
        .unwrap();
    let Typed::Value { value: decoded, .. } = value else {
        panic!("{value:?}")
    };
    assert_eq!(decoded.as_ref().unwrap(), &plan(7), "the record, decoded");
    assert_eq!(
        answer.mark(value),
        Some(zenkey::model::seed::Mark::InBand(stamp(
            Run::U32(0xdead_beef),
            10
        ))),
        "ordered by its run and seq, never by the router's stamp"
    );
    let deleted = answer
        .current
        .iter()
        .find(|c| c.key() == vehicle(2))
        .unwrap();
    assert!(matches!(deleted, Typed::Deleted { .. }), "{deleted:?}");
    assert_eq!(
        answer.mark(deleted),
        Some(zenkey::model::seed::Mark::InBand(stamp(
            Run::U32(0xdead_beef),
            11
        )))
    );
    assert_eq!(
        answer.undecodable.get(&vehicle(3)).map(|r| r.token()),
        Some("form")
    );
    // A consumer orders the seed's replies by them (§4.2, §4.3).
    let mut order = ValueOrder::new();
    assert!(order.accept_mark(&vehicle(1), answer.mark(value).unwrap()));
    assert!(!order.accept_mark(
        &vehicle(1),
        zenkey::model::seed::Mark::InBand(stamp(Run::U32(0xdead_beef), 9))
    ));
    match typed::get::<Plans>(&consumer, PLANS, None, T)
        .await
        .unwrap()
    {
        StateGet::Answered(v) => assert_eq!(v.len(), 2, "{v:?}"),
        StateGet::Silent => panic!("the owner answered"),
    }
}

/// The typed last-known read carries the archive's in-band stamp beside
/// its decoded record (S5), what orders it, never a time (observer §8.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_typed_last_known_carries_the_in_band_stamp() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let s = client(&ep).await;
    let reader = client(&ep).await;
    let _arch = Archive::start(
        &s,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![recorded()],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .unwrap();
    let consumer = tool(&reader);
    let values: zenkey::model::template::Bindings =
        [("vehicle".to_owned(), vec!["vehicle-01".to_owned()])].into();
    let lk = eventually_some("the plan is recorded", || async {
        raw.put(ORIGIN, sealed(Run::U16(3), 9, &plan(4)))
            .encoding(Encoding::ZENOH_BYTES)
            .await
            .unwrap();
        zenkey::typed::last_known::<Plans>(
            &consumer,
            &common::addr("vehicle-01/archive"),
            &common::addr("ground/fleet-mgr"),
            PLANS,
            &values,
            T,
        )
        .await
        .unwrap()
    })
    .await;
    assert_eq!(lk.origin, ORIGIN);
    assert_eq!(lk.inband, Some(stamp(Run::U16(3), 9)));
    assert_eq!(
        lk.mark(),
        Some(zenkey::model::seed::Mark::InBand(stamp(Run::U16(3), 9)))
    );
    assert_eq!(lk.value.unwrap().unwrap(), plan(4), "the record");
}

fn recorded() -> Recorded {
    Recorded {
        owner: common::addr("ground/fleet-mgr"),
        selector: "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*".to_owned(),
        implementation: imp("mission_plan.v1"),
    }
}

/// What the archive answers for [`ORIGIN`]: the record, or `None` for a
/// tombstone.
async fn last_known(s: &zenoh::Session) -> Option<Option<String>> {
    archive::last_known(s, &common::addr("vehicle-01/archive"), ORIGIN, T)
        .await
        .unwrap()
        .map(|lk| lk.value.map(|v| String::from_utf8(v).unwrap()))
}

/// Puts `marker` on [`OTHER`] under a run of its own and waits until the
/// archive holds it: what was put on [`ORIGIN`] before it, from the same
/// session, was heard first.
async fn heard(raw: &zenoh::Session, reader: &zenoh::Session, run: u32, marker: &str) {
    let a = common::addr("vehicle-01/archive");
    eventually("the marker is recorded", || async {
        put(raw, OTHER, Run::U32(run), 0, marker).await;
        archive::last_known(reader, &a, OTHER, T)
            .await
            .unwrap()
            .and_then(|lk| lk.value)
            .is_some_and(|v| v == marker.as_bytes())
    })
    .await;
}

/// The archive refuses an outdated put within a run, a put older than a
/// delete it holds included, and records a value of another run; it answers
/// with the owner's envelope, which `last_known` reads as its record.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_archive_refuses_an_outdated_put_only_within_a_run() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let s = client(&ep).await;
    let reader = client(&ep).await;
    let _arch = Archive::start(
        &s,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![recorded()],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .unwrap();
    eventually("v5 is recorded", || async {
        put(&raw, ORIGIN, Run::U16(1), 5, "v5").await;
        last_known(&reader).await == Some(Some("v5".to_owned()))
    })
    .await;

    // Older within the run, though the router stamped it later: refused.
    put(&raw, ORIGIN, Run::U16(1), 3, "v3").await;
    heard(&raw, &reader, 1, "one").await;
    assert_eq!(last_known(&reader).await, Some(Some("v5".to_owned())));

    // Another run: nothing orders it, and it is recorded.
    put(&raw, ORIGIN, Run::U16(2), 1, "w1").await;
    eventually("w1 is recorded", || async {
        last_known(&reader).await == Some(Some("w1".to_owned()))
    })
    .await;

    // A delete of the run, then a put older than it: the delete stands.
    delete(&raw, ORIGIN, Run::U16(2), 4).await;
    eventually("the delete is recorded", || async {
        last_known(&reader).await == Some(None)
    })
    .await;
    put(&raw, ORIGIN, Run::U16(2), 3, "w3").await;
    heard(&raw, &reader, 2, "two").await;
    assert_eq!(
        last_known(&reader).await,
        Some(None),
        "a put older than a held delete resurrects nothing"
    );

    // The tombstone's stamp is answered as it arrived: the reply_del's
    // attachment.
    let rx = reader
        .get("zk2/vehicle-01/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01")
        .timeout(T)
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), rx.recv_async())
        .await
        .unwrap()
        .unwrap();
    let tomb = reply.into_result().unwrap();
    assert_eq!(tomb.kind(), zenoh::sample::SampleKind::Delete);
    assert_eq!(
        zenkey::inband::stamp(&tomb),
        Some(stamp(Run::U16(2), 4)),
        "the archive answers the owner's delete stamp"
    );
}

/// Observer §7.3 reads `zenoh/bytes` by its spelling. A put that sets no
/// Encoding carries zenoh's default, so the default must spell the same,
/// or a reader would miss every envelope put that way.
#[test]
fn zenohs_default_encoding_is_spelled_as_the_envelopes() {
    assert_eq!(
        Encoding::default().to_string(),
        zenkey::model::wire::ENCODING
    );
    assert_eq!(
        Encoding::ZENOH_BYTES.to_string(),
        zenkey::model::wire::ENCODING
    );
}

/// A refused envelope is no arrival (core §7.5, observer §7.3): it does not
/// refresh a consumer's freshness of its member (`freshness.v1` §2.5), and
/// a member that only ever sent one is not one the subscription heard.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_envelope_does_not_refresh_a_consumers_freshness() {
    use zenkey::model::freshness::{Last, Observation};
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let reader = client(&ep).await;
    let got: Delivered = Arc::default();
    let sink = Arc::clone(&got);
    let consumer = tool(&reader);
    let sub = consumer
        .subscribe(PLANS, move |d| sink.lock().unwrap().push(d))
        .await
        .unwrap();
    eventually("the subscription is heard", || async {
        put(&raw, OTHER, Run::U16(1), 0, "probe").await;
        !got.lock().unwrap().is_empty()
    })
    .await;
    put(&raw, ORIGIN, Run::U16(7), 3, "a plan").await;
    eventually("the plan delivered", || async {
        of_origin(&got).len() == 1
    })
    .await;
    let age = |o: Observation| match o {
        Observation::Subscribed {
            last: Some(Last::Put(d)),
            ..
        } => d,
        other => panic!("a put heard: {other:?}"),
    };
    tokio::time::sleep(Duration::from_millis(400)).await;
    // An envelope refused (`{` is a reserved form), on the member and on a
    // member never heard.
    const NEVER: &str = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-03";
    for key in [ORIGIN, NEVER] {
        raw.put(key, r#"{"plan":2}"#)
            .encoding(Encoding::ZENOH_BYTES)
            .await
            .unwrap();
    }
    eventually("both refused", || async { sub.undecodable() == 2 }).await;
    assert!(
        age(sub.freshness(ORIGIN)) >= Duration::from_millis(400),
        "the refused envelope did not refresh it"
    );
    assert!(
        !sub.members().iter().any(|m| m == NEVER),
        "a member only refused is not heard"
    );
    assert!(matches!(
        sub.freshness(NEVER),
        Observation::Subscribed { last: None, .. }
    ));
    assert_eq!(of_origin(&got).len(), 1, "nothing more delivered");
}
