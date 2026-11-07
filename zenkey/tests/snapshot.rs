//! A collection's snapshot as the runtime's readers read it (core §7.5's
//! form 7; #977's E2, E3): `common::snapshot_owner` answers S4's GET with
//! pages written by `collection::Page::write`, as owners do, and each
//! reader takes every entry as the per-key reply it stands for, a live
//! entry a value and a tombstone a deletion, each with its stamp. Every
//! reader GETs under `None` (E3), so a snapshot of several pages arrives
//! whole; the consumer asks a collection read where it asks every member.

mod common;

use std::sync::{Arc, Mutex};

use common::snapshot_owner::{self as owner, OWN_RUN, RUN};
use common::{T, client, config, contract, eventually, eventually_some, imp, router};
use zenkey::archive::{Archive, ArchiveConfig, Recorded};
use zenkey::consumer::Consumer;
use zenkey::model::collection::{Page, Snapshot};
use zenkey::model::seed::Mark;
use zenkey::state::{Current, StateAnswer};
use zenoh::Wait;
use zenoh::bytes::Encoding;

const ADDRESS: &str = "ground/fleet-mgr";
const PLANS: &str = "state/plans/{vehicle}";

fn tool(s: &zenoh::Session) -> Consumer {
    Consumer::for_tool(
        s,
        Arc::new(contract("mission_plan.v1")),
        &[ADDRESS],
        &Default::default(),
    )
    .unwrap()
}

fn text(c: &Current) -> Option<String> {
    match c {
        Current::Value { sample, .. } => {
            Some(String::from_utf8(sample.payload().to_bytes().into_owned()).unwrap())
        }
        Current::Deleted { .. } => None,
    }
}

fn find<'a>(a: &'a StateAnswer, id: &str) -> &'a Current {
    let key = owner::member(ADDRESS, id);
    a.current
        .iter()
        .find(|c| c.key() == key)
        .unwrap_or_else(|| panic!("{key} in {a:?}"))
}

/// A collection read (`…/state/plans/**`, the consumer's GET where it
/// asks every member; #977's E3): the collection's snapshot on its
/// snapshot key, three pages of one entry each, read whole under `None`. A
/// live entry is a value with its record and stamp, its own run where it
/// has one; a tombstone a deletion with its stamp. A member read of one
/// key: its own snapshot, its entry's id empty; a page whose body is
/// refused is undecodable, by its refusal's token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_consumer_reads_a_collection_read_as_values_and_deletions() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let reader = client(&ep).await;
    let _owner = owner::start(&raw, ADDRESS).await;
    let consumer = tool(&reader);
    assert_eq!(
        consumer.get_selectors(PLANS, None).unwrap()[0].as_str(),
        format!("{}/**", owner::plans(ADDRESS)),
        "the collection read"
    );
    let answer = eventually_some("the owner answers", || async {
        let a = consumer.get_answer(PLANS, None, T).await.unwrap();
        (a.current.len() == 3).then_some(a)
    })
    .await;
    assert!(answer.complete, "{answer:?}");
    assert_eq!(text(find(&answer, "vehicle-01")).as_deref(), Some("plan 1"));
    assert_eq!(text(find(&answer, "vehicle-02")).as_deref(), Some("plan 2"));
    assert!(matches!(
        find(&answer, "vehicle-03"),
        Current::Deleted { .. }
    ));
    for id in ["vehicle-01", "vehicle-02", "vehicle-03"] {
        assert_eq!(
            answer.mark(find(&answer, id)),
            Some(Mark::InBand(owner::stamp(id))),
            "{id}: its entry's stamp, its own run or the page's"
        );
    }
    assert_eq!(owner::stamp("vehicle-02").run, OWN_RUN);
    assert!(answer.complete, "{answer:?}");
    assert_eq!(answer.snapshots.len(), 1, "{:?}", answer.snapshots);
    let h = &answer.snapshots[0];
    assert!(h.complete(), "{h:?}");
    assert_eq!(h.key, owner::plans(ADDRESS));
    assert_eq!((h.run, h.asof, h.coll), (RUN, owner::asof(), 0));
    assert_eq!(h.pages.iter().copied().collect::<Vec<_>>(), [0, 1, 2]);
    assert!(
        answer.omitted.iter().all(Vec::is_empty),
        "every member is listed"
    );
    // A page whose body is refused, on the one member read of its key.
    let values: zenkey::model::template::Bindings =
        [("vehicle".to_owned(), vec!["broken".to_owned()])].into();
    let broken = consumer.get_answer(PLANS, Some(&values), T).await.unwrap();
    assert_eq!(
        broken
            .undecodable
            .get(&owner::member(ADDRESS, "broken"))
            .map(|u| u.token()),
        Some("short"),
        "a page whose body is refused"
    );
    // One member, every parameter bound: one snapshot on its key.
    let values: zenkey::model::template::Bindings =
        [("vehicle".to_owned(), vec!["vehicle-01".to_owned()])].into();
    let one = consumer.get_answer(PLANS, Some(&values), T).await.unwrap();
    assert_eq!(one.current.len(), 1, "{one:?}");
    assert_eq!(one.snapshots[0].key, owner::member(ADDRESS, "vehicle-01"));
    // A member the owner holds none at: a snapshot of no entry, complete,
    // which is no silence, though no key answered.
    let values: zenkey::model::template::Bindings =
        [("vehicle".to_owned(), vec!["vehicle-09".to_owned()])].into();
    let none = consumer.get_answer(PLANS, Some(&values), T).await.unwrap();
    assert!(none.current.is_empty(), "{none:?}");
    assert_eq!(none.snapshots.len(), 1);
    assert!(none.snapshots[0].complete());
    // The typed GET carries the same.
    let typed = zenkey::typed::get_answer::<zenkey::codec::Raw>(&consumer, PLANS, None, T)
        .await
        .unwrap();
    assert_eq!(typed.current.len(), 3);
    assert_eq!(typed.snapshots.len(), 1);
}

/// A snapshot's pages share one key (core §4.2's "Why `None`"): under
/// `None` both arrive, and the consumer holds the snapshot complete at its
/// last page, its entry standing; under `Latest`, the control, a GET
/// delivers one of the two.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paged_snapshot_is_read_whole_under_none() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let reader = client(&ep).await;
    let key = owner::member(ADDRESS, "vehicle-07");
    // Two pages on one member's key: the entry on page 0, page 1 empty.
    let mut s: Snapshot = owner::member_snapshot("vehicle-01");
    let entry = s.live.pop().expect("one entry");
    let pages = [
        Page {
            tag: s.tag,
            run: s.run,
            asof: s.asof,
            time: s.time,
            index: 0,
            more: true,
            coll: 0,
            live: vec![entry],
            tombs: Vec::new(),
        },
        Page {
            tag: s.tag,
            run: s.run,
            asof: s.asof,
            time: s.time,
            index: 1,
            more: false,
            coll: 0,
            live: Vec::new(),
            tombs: Vec::new(),
        },
    ];
    let bytes: Vec<Vec<u8>> = pages.iter().map(|p| p.write().unwrap()).collect();
    let k = key.clone();
    let _q = raw
        .declare_queryable(format!("zk2/{ADDRESS}/mission_plan.v1/state/**"))
        .callback(move |q| {
            for b in &bytes {
                q.reply(k.clone(), b.clone())
                    .encoding(Encoding::ZENOH_BYTES)
                    .wait()
                    .unwrap();
            }
        })
        .await
        .unwrap();
    let consumer = tool(&reader);
    let answer = eventually_some("the owner answers", || async {
        let a = consumer.get_answer(PLANS, None, T).await.unwrap();
        (!a.snapshots.is_empty()).then_some(a)
    })
    .await;
    assert_eq!(answer.snapshots.len(), 1, "{answer:?}");
    let h = &answer.snapshots[0];
    assert_eq!(h.pages.iter().copied().collect::<Vec<_>>(), [0, 1], "{h:?}");
    assert!(h.complete(), "both pages, its last one's more bit 0");
    assert!(answer.complete);
    assert_eq!(answer.current.len(), 1, "page 0's entry");
    // The control: `Latest` keeps one reply on the key.
    let rx = reader
        .get(format!("{}/**", owner::plans(ADDRESS)))
        .consolidation(zenoh::query::ConsolidationMode::Latest)
        .timeout(T)
        .await
        .unwrap();
    let mut n = 0;
    while let Ok(r) = rx.recv_async().await {
        n += usize::from(r.result().is_ok());
    }
    assert_eq!(n, 1, "one page of the two");
}

/// A snapshot's page heard on a subscription is no owner's answer (core
/// §7.5: only a GET's reply carries one): never delivered, and counted
/// undecodable, as the subscription counts a refused envelope.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_heard_on_a_subscription_is_undecodable() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let reader = client(&ep).await;
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = Arc::clone(&got);
    let consumer = tool(&reader);
    let sub = consumer
        .subscribe(PLANS, move |d| {
            sink.lock().unwrap().push(d.sample.key_expr().to_string());
        })
        .await
        .unwrap();
    let page = owner::member_snapshot("vehicle-01").paginate(1024).unwrap()[0]
        .write()
        .unwrap();
    let key = owner::member(ADDRESS, "vehicle-01");
    eventually("the page is counted", || async {
        raw.put(&key, page.clone())
            .encoding(Encoding::ZENOH_BYTES)
            .await
            .unwrap();
        sub.undecodable() > 0
    })
    .await;
    assert!(got.lock().unwrap().is_empty(), "never delivered");
    let read = zenkey::inband::read(
        zenoh::sample::SampleBuilder::put(
            zenoh::key_expr::KeyExpr::try_from(key.clone()).unwrap(),
            page.clone(),
        )
        .encoding(Encoding::ZENOH_BYTES)
        .into(),
    );
    assert_eq!(read.undecodable().map(|u| u.token()), Some("snapshot"));
    assert_eq!(read.stamp(), None, "a page orders no value");
    assert_eq!(read.page().map(|p| p.coll), Some(0));
}

/// An archive aligns from the owner's snapshot (#977's E2, E3): a collection
/// read (`…/state/plans/**`, consolidation `None`) brings the three pages,
/// and each entry is recorded as the per-key reply it stands for — a live
/// entry the bytes its mutation put, which `last_known` reads back as its
/// record and stamp, a tombstone a deletion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_aligns_from_a_paged_snapshot() {
    let (_r, ep) = router(None).await;
    let raw = client(&ep).await;
    let s = client(&ep).await;
    let reader = client(&ep).await;
    let _owner = owner::start(&raw, ADDRESS).await;
    let _token = raw
        .liveliness()
        .declare_token(format!("zk2/{ADDRESS}/@zk/instance/00000000000000aa"))
        .await
        .unwrap();
    let archive = Archive::start(
        &s,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![Recorded {
                owner: common::addr(ADDRESS),
                selector: format!("{}/**", owner::plans(ADDRESS)),
                implementation: imp("mission_plan.v1"),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
            watch: None,
        },
    )
    .await
    .unwrap();
    let a = common::addr("vehicle-01/archive");
    for (id, record) in [("vehicle-01", "plan 1"), ("vehicle-02", "plan 2")] {
        let origin = owner::member(ADDRESS, id);
        let lk = eventually_some("the entry is recorded", || async {
            archive.align(T).await.unwrap();
            zenkey::archive::last_known(&reader, &a, &origin, T)
                .await
                .unwrap()
        })
        .await;
        assert_eq!(lk.value.as_deref(), Some(record.as_bytes()), "{id}");
        assert_eq!(lk.inband, Some(owner::stamp(id)), "{id}");
        assert_eq!(archive.confirmed(&origin), Some(true), "{id}");
    }
    let tomb = owner::member(ADDRESS, "vehicle-03");
    let lk = zenkey::archive::last_known(&reader, &a, &tomb, T)
        .await
        .unwrap()
        .expect("the tombstone is held");
    assert_eq!(lk.value, None, "a deletion, not a value");
    assert_eq!(lk.inband, Some(owner::stamp("vehicle-03")));
    assert!(
        zenkey::archive::last_known(&reader, &a, &owner::plans(ADDRESS), T)
            .await
            .unwrap()
            .is_none(),
        "the page's own key is no member, and nothing is recorded there"
    );
}

/// Two instances answer per key (core §4.2, "Where two instances answer";
/// review-bot on #1078): under `None` both replies on a key arrive, and the
/// consumer keeps one value per key by §4.3's consumer rule, whichever
/// arrives first. A stamped reply is kept over an unstamped one; within one
/// run a `reply_del` with a higher seq replaces the put, and a put with a
/// lower seq never replaces the delete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_instances_per_key_replies_keep_one_value_per_key() {
    use zenkey::model::wire::{DeleteStamp, Envelope, Seq, Stamp, Tag};
    let (_r, ep) = router(None).await;
    let first = client(&ep).await;
    let second = client(&ep).await;
    let reader = client(&ep).await;
    let x = owner::member(ADDRESS, "x");
    let y = owner::member(ADDRESS, "y");
    let at = |seq: u32| Stamp {
        run: RUN,
        seq: Seq::new(seq).unwrap(),
    };
    let (x1, y1) = (x.clone(), y.clone());
    let _q1 = first
        .declare_queryable(format!("zk2/{ADDRESS}/mission_plan.v1/state/**"))
        .callback(move |q| {
            // `x`, a put at seq 4; `y`, a stamped put.
            q.reply(
                x1.clone(),
                Envelope::mutation(Tag::NONE, at(4), b"x4").write(),
            )
            .encoding(Encoding::ZENOH_BYTES)
            .wait()
            .unwrap();
            q.reply(
                y1.clone(),
                Envelope::mutation(Tag::NONE, at(2), b"y2").write(),
            )
            .encoding(Encoding::ZENOH_BYTES)
            .wait()
            .unwrap();
        })
        .await
        .unwrap();
    let (x2, y2) = (x.clone(), y.clone());
    let _q2 = second
        .declare_queryable(format!("zk2/{ADDRESS}/mission_plan.v1/state/**"))
        .callback(move |q| {
            // `x`, deleted at seq 6; `y`, a plain reply with neither stamp.
            q.reply_del(x2.clone())
                .attachment(DeleteStamp { stamp: at(6) }.write())
                .wait()
                .unwrap();
            q.reply(y2.clone(), b"plain".to_vec())
                .encoding(Encoding::TEXT_PLAIN)
                .wait()
                .unwrap();
        })
        .await
        .unwrap();
    let consumer = tool(&reader);
    // Both queryables routed: the second's delete is heard.
    eventually("both answer", || async {
        let a = consumer.get_answer(PLANS, None, T).await.unwrap();
        a.inband.get(&x) == Some(&at(6))
    })
    .await;
    for _ in 0..5 {
        let a = consumer.get_answer(PLANS, None, T).await.unwrap();
        let held_x = a.current.iter().find(|c| c.key() == x).unwrap();
        assert!(matches!(held_x, Current::Deleted { .. }), "{a:?}");
        assert_eq!(a.inband.get(&x), Some(&at(6)));
        assert_eq!(
            text(a.current.iter().find(|c| c.key() == y).unwrap()).as_deref(),
            Some("y2")
        );
        assert_eq!(a.inband.get(&y), Some(&at(2)), "the stamped reply is kept");
    }
}
