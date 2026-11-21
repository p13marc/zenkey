//! A consumer reads owner heartbeats (core §7.5's second layout of form
//! 6; `freshness.v2` §2.4, §2.5; #1110's H2b). No owner sends one yet, so
//! these tests write them through the model's writer, raw, on the class
//! key `freshness.v2`'s revision 1.1 declares.
//!
//! Setup: R1 a router; the owner `lab/trio` implements `alpha.v1`,
//! `bravo.v1` and `charlie.v1` (`tests/contracts/trio/`) as a client of
//! R1, given a catalog that lists them at indices 0, 1 and 2, its re-puts
//! off; one tool's consumer per interface on another client, each
//! subscribed to `state/status`, a collection of the `host` class.
//! `charlie.v1`'s `state/reach` is of the `link` class.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{client, config, contract, eventually, imp, router};
use zenkey::consumer::{Consumer, Delivery, HeartbeatRead, HeartbeatSection, OwnerGroup};
use zenkey::model::canonical::Fingerprint;
use zenkey::model::catalog::Catalog;
use zenkey::model::freshness::{Horizon, Observation, Verdict, judge};
use zenkey::model::grammar::IfaceId;
use zenkey::model::heartbeat::{
    Class, Digest, Group, Layout, Members, OwnerHeartbeat, Section, Time, owner_key,
};
use zenkey::model::template::Bindings;
use zenkey::model::wire::{Seq, Stamp};
use zenkey::state::StateWriter;
use zenkey::{Service, ServiceBuilder};

const STATUS: &str = "state/status";
const REACH: &str = "state/reach";
const IFACES: [&str; 3] = ["alpha.v1", "bravo.v1", "charlie.v1"];

fn trio(name: &str) -> String {
    format!("trio/{name}")
}

fn iface(name: &str) -> IfaceId {
    name.parse().expect("an interface")
}

fn status_key(name: &str) -> String {
    format!("zk2/lab/trio/{name}/state/status")
}

/// The catalog the deployment gives the owner and its readers: the three
/// interfaces at indices 0, 1 and 2.
fn catalog() -> Catalog {
    let mut c = Catalog::new(0).unwrap();
    for name in IFACES {
        let k = contract(&trio(name));
        c.push(k.iface.clone(), Fingerprint::of(&k)).unwrap();
    }
    c
}

/// The owner, started: each interface's status writer, re-puts off, and
/// `charlie.v1`'s `reach`.
async fn owner(
    session: &zenoh::Session,
    catalog: Option<Catalog>,
) -> (Service, Vec<StateWriter>, StateWriter) {
    let mut b = ServiceBuilder::new(session, config("lab/trio"));
    if let Some(c) = catalog {
        b.catalog(c).unwrap();
    }
    let none = Bindings::new();
    let mut statuses = Vec::new();
    for name in IFACES {
        b.implement(imp(&trio(name))).unwrap();
        let w = b
            .declare_state_writer(&iface(name), STATUS, &none)
            .await
            .unwrap();
        w.set_refresh(false);
        statuses.push(w);
    }
    let reach = b
        .declare_state_writer(&iface("charlie.v1"), REACH, &none)
        .await
        .unwrap();
    reach.set_refresh(false);
    (b.start().await.unwrap(), statuses, reach)
}

/// One reader of an interface's collection: what it was delivered and
/// each heartbeat its view read.
struct Reader {
    sub: zenkey::consumer::Subscription,
    delivered: Arc<Mutex<Vec<Stamp>>>,
    read: Arc<Mutex<Vec<HeartbeatRead>>>,
}

impl Reader {
    async fn of(
        session: &zenoh::Session,
        name: &str,
        resource: &str,
        catalog: Option<&Catalog>,
    ) -> Reader {
        let mut c = Consumer::for_tool(
            session,
            Arc::new(contract(&trio(name))),
            &["lab/trio"],
            &Default::default(),
        )
        .unwrap();
        if let Some(cat) = catalog {
            c = c.with_catalog(cat);
        }
        let delivered: Arc<Mutex<Vec<Stamp>>> = Arc::default();
        let read: Arc<Mutex<Vec<HeartbeatRead>>> = Arc::default();
        let (d, k) = (Arc::clone(&delivered), Arc::clone(&read));
        let sub = c
            .subscribe_heartbeats(
                resource,
                move |x: Delivery| d.lock().unwrap().extend(x.inband),
                move |x: HeartbeatRead| k.lock().unwrap().push(x),
            )
            .await
            .unwrap();
        Reader {
            sub,
            delivered,
            read,
        }
    }

    /// The stamp of the one put it was delivered.
    async fn stamp(&self) -> Stamp {
        eventually("the put is delivered", || async {
            !self.delivered.lock().unwrap().is_empty()
        })
        .await;
        self.delivered.lock().unwrap()[0]
    }

    /// Each heartbeat handed over: its key's seq, whether one was lost
    /// before it, the group it was, and what its section did.
    fn got(&self) -> Vec<(u32, bool, Option<OwnerGroup>, HeartbeatSection)> {
        self.read
            .lock()
            .unwrap()
            .iter()
            .map(|h| (h.heartbeat.seq.get(), h.lost, h.group, h.section.clone()))
            .collect()
    }
}

/// A group of `class` covering `name` at `index`: its collection
/// `resource` with `digest`, vouched for, and every other map bit 0.
fn group(index: u16, name: &str, resource: &str, digest: Digest) -> Group {
    let layout = Layout::of(&contract(&trio(name)));
    let sections: Vec<Option<Section>> = layout
        .collections
        .iter()
        .map(|c| {
            (c.resource() == resource).then_some(Section {
                digest,
                members: Members::One { vouched: true },
            })
        })
        .collect();
    Group::of(index, &layout, &sections).unwrap()
}

/// An owner heartbeat of these groups, at `seq` of `run`'s sequence.
fn owner_heartbeat(groups: Vec<Group>, like: Stamp, seq: u32) -> Vec<u8> {
    OwnerHeartbeat {
        time: Time::Unknown,
        groups,
    }
    .envelope(Stamp {
        run: like.run,
        seq: Seq::new(seq).unwrap(),
    })
    .unwrap()
}

/// Puts each status once, and waits until every reader holds its own.
async fn put_statuses(statuses: &[StateWriter], readers: &[Reader]) -> Vec<Stamp> {
    for w in statuses {
        eventually("the status is matched", || async {
            w.writer().matching().await.unwrap()
        })
        .await;
        w.put("up").await.unwrap();
    }
    let mut stamps = Vec::new();
    for r in readers {
        stamps.push(r.stamp().await);
    }
    stamps
}

/// One owner heartbeat on the `host` key confirms three interfaces'
/// members, each reader handed the group of its own interface with the
/// class key's stamp (§2.5, "An owner heartbeat"). One lost on the key is
/// lost for every collection the key covers, until one arrives in
/// sequence, and says nothing of the `link` key's collection. A
/// divergence in one group takes nothing from the groups beside it, and
/// neither does a group whose index the catalog does not hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_owner_heartbeat_confirms_three_interfaces() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let cat = catalog();
    let (_svc, statuses, reach) = owner(&os, Some(cat.clone())).await;
    let mut readers = Vec::new();
    for name in IFACES {
        readers.push(Reader::of(&rs, name, STATUS, Some(&cat)).await);
    }
    // charlie.v1's collection of the `link` class, which the `host` key
    // never confirms.
    let link = Reader::of(&rs, "charlie.v1", REACH, Some(&cat)).await;
    let stamps = put_statuses(&statuses, &readers).await;
    eventually("reach is matched", || async {
        reach.writer().matching().await.unwrap()
    })
    .await;
    reach.put("near").await.unwrap();
    link.stamp().await;

    let host = owner_key("lab", "trio", Class::Host);
    let keys: Vec<String> = IFACES.iter().map(|n| status_key(n)).collect();
    let held = |i: usize| Digest::of([(keys[i].as_str(), stamps[i])]);
    let whole = |digests: [Digest; 3]| {
        (0..3)
            .map(|i| group(i as u16, IFACES[i], STATUS, digests[i]))
            .collect::<Vec<_>>()
    };
    let at = |index: u16| {
        Some(OwnerGroup {
            class: Class::Host,
            index,
        })
    };
    let confirmed = |i: usize| HeartbeatSection::Confirmed(vec![keys[i].clone()]);

    // Seq 0: three groups in step. Each reader is handed its own.
    os.put(
        &host,
        owner_heartbeat(whole([held(0), held(1), held(2)]), stamps[0], 0),
    )
    .await
    .unwrap();
    for (i, r) in readers.iter().enumerate() {
        eventually("its group is handed over", || async { r.got().len() == 1 }).await;
        assert_eq!(r.got(), [(0, false, at(i as u16), confirmed(i))]);
        assert!(
            r.sub.confirmed(&keys[i]).is_some(),
            "{} confirmed",
            IFACES[i]
        );
        assert_eq!((r.sub.heartbeats(), r.sub.owner_heartbeats()), (1, 1));
        assert!(!r.sub.lost(&keys[i]));
    }

    // Seq 2: seq 1 was lost on the key, with every group it held.
    os.put(
        &host,
        owner_heartbeat(whole([held(0), held(1), held(2)]), stamps[0], 2),
    )
    .await
    .unwrap();
    for (i, r) in readers.iter().enumerate() {
        eventually("the one after the gap is handed over", || async {
            r.got().len() == 2
        })
        .await;
        assert_eq!(r.got()[1], (2, true, at(i as u16), confirmed(i)));
        assert!(r.sub.lost(&keys[i]), "{}: a lost delivery", IFACES[i]);
        // Read as a lost delivery (v1 §2.5 rule 3): the observation is
        // incomplete, so a member past its horizon is unobservable, never
        // stale.
        assert!(
            matches!(
                r.sub.freshness(&keys[i]),
                Observation::Subscribed {
                    complete: false,
                    ..
                }
            ),
            "{}: incomplete",
            IFACES[i]
        );
    }
    // Two keys are two sequences: nothing was heard on the `link` key.
    assert!(!link.sub.lost("zk2/lab/trio/charlie.v1/state/reach"));
    assert_eq!(link.sub.owner_heartbeats(), 0);

    // Seq 3, in sequence: nothing is lost any more. bravo.v1's group
    // carries a digest its reader does not hold: it alone diverges.
    let missed = Digest::of([(
        keys[1].as_str(),
        Stamp {
            seq: Seq::new(stamps[1].seq.get() + 1).unwrap(),
            ..stamps[1]
        },
    )]);
    os.put(
        &host,
        owner_heartbeat(whole([held(0), missed, held(2)]), stamps[0], 3),
    )
    .await
    .unwrap();
    for (i, r) in readers.iter().enumerate() {
        eventually("seq 3 is handed over", || async { r.got().len() == 3 }).await;
        let section = if i == 1 {
            HeartbeatSection::Diverged
        } else {
            confirmed(i)
        };
        assert_eq!(r.got()[2], (3, false, at(i as u16), section));
        assert!(!r.sub.lost(&keys[i]));
    }

    // Seq 4: alpha.v1's group beside one at an index the catalog does not
    // hold. The known one is read all the same; the other confirms
    // nothing, and no reader is handed it.
    let unread: Vec<u64> = readers.iter().map(|r| r.sub.heartbeats_unread()).collect();
    os.put(
        &host,
        owner_heartbeat(
            vec![
                group(0, "alpha.v1", STATUS, held(0)),
                group(900, "bravo.v1", STATUS, held(1)),
            ],
            stamps[0],
            4,
        ),
    )
    .await
    .unwrap();
    eventually("alpha.v1's group is handed over", || async {
        readers[0].got().len() == 4
    })
    .await;
    assert_eq!(readers[0].got()[3], (4, false, at(0), confirmed(0)));
    for (i, r) in readers.iter().enumerate() {
        eventually("the owner heartbeat is read", || async {
            r.sub.owner_heartbeats() == 4
        })
        .await;
        assert_eq!(
            r.sub.heartbeats_unread(),
            unread[i] + 1,
            "{}: the group at index 900 was not read (unknown_index)",
            IFACES[i]
        );
    }
    assert_eq!(readers[1].got().len(), 3, "bravo.v1 was handed no group");
    assert_eq!(readers[2].got().len(), 3, "charlie.v1 had none in it");
}

/// A reader given no catalog could read no group (core §8.6), so it
/// declares no owner heartbeat key: the puts a reader with the catalog
/// reads never reach it, nothing is confirmed, and the member ages from
/// its put and goes stale. Never fresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_reader_with_no_catalog_declares_no_owner_heartbeat_key() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let cat = catalog();
    let (_svc, statuses, _reach) = owner(&os, Some(cat.clone())).await;
    let reader = Reader::of(&rs, "alpha.v1", STATUS, None).await;
    let control = Reader::of(&rs, "alpha.v1", STATUS, Some(&cat)).await;
    let stamps = put_statuses(&statuses[..1], std::slice::from_ref(&reader)).await;
    let key = status_key("alpha.v1");
    let host = owner_key("lab", "trio", Class::Host);
    let held = Digest::of([(key.as_str(), stamps[0])]);
    for seq in 0..2 {
        os.put(
            &host,
            owner_heartbeat(vec![group(0, "alpha.v1", STATUS, held)], stamps[0], seq),
        )
        .await
        .unwrap();
        eventually("the reader with the catalog reads it", || async {
            control.sub.owner_heartbeats() == u64::from(seq) + 1
        })
        .await;
    }
    let h = Horizon::Within(Duration::from_secs(2));
    eventually("the member goes stale, 2 s after its put", || async {
        judge(&h, &reader.sub.freshness(&key)).verdict == Verdict::Stale
    })
    .await;
    assert!(reader.got().is_empty(), "no group is handed over");
    assert_eq!(
        (
            reader.sub.owner_heartbeats(),
            reader.sub.heartbeats(),
            reader.sub.heartbeats_unread(),
            reader.sub.owner_catalog_reads()
        ),
        (0, 0, 0, 0),
        "it heard none, and asked no descriptor"
    );
    assert!(reader.sub.confirmed(&key).is_none());
    assert!(control.sub.confirmed(&key).is_some(), "the control did");
}

/// A tool's consumer of `alpha.v1` at `lab/trio`, given the catalog, with
/// no owner behind the address: the test answers its descriptor reads.
fn bare_consumer(session: &zenoh::Session) -> Consumer {
    Consumer::for_tool(
        session,
        Arc::new(contract(&trio("alpha.v1"))),
        &["lab/trio"],
        &Default::default(),
    )
    .unwrap()
    .with_catalog(&catalog())
}

/// A hand-written owner heartbeat of `alpha.v1`'s group, at `seq` of run 7.
fn bare_heartbeat(seq: u32) -> Vec<u8> {
    let like = Stamp {
        run: zenkey::model::wire::Run::U16(7),
        seq: Seq::new(0).unwrap(),
    };
    owner_heartbeat(vec![group(0, "alpha.v1", STATUS, Digest::EMPTY)], like, seq)
}

/// A descriptor read that fails names no catalog for its run, and stands
/// for `OWNER_CATALOG_RETRY`: the owner heartbeats that follow read no
/// group, and none of them asks again. One GET, never one a put.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_descriptor_read_is_not_asked_again_at_each_owner_heartbeat() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    // An owner whose descriptor read answers an error.
    let asked = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let n = Arc::clone(&asked);
    let _descriptor = os
        .declare_queryable("zk2/lab/trio/@zk/instance/*")
        .callback(move |q| {
            n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = zenoh::Wait::wait(q.reply_err("no descriptor here"));
        })
        .await
        .unwrap();
    let sub = bare_consumer(&rs)
        .subscribe_heartbeats(STATUS, |_: Delivery| {}, |_: HeartbeatRead| {})
        .await
        .unwrap();
    let host = owner_key("lab", "trio", Class::Host);
    let publisher = os.declare_publisher(host).await.unwrap();
    eventually("the class key is matched", || async {
        publisher.matching_status().await.unwrap().matching()
    })
    .await;
    for seq in 0..5 {
        publisher.put(bare_heartbeat(seq)).await.unwrap();
        eventually("the owner heartbeat is read on its key", || async {
            sub.owner_heartbeats() == u64::from(seq) + 1
        })
        .await;
    }
    assert_eq!(sub.owner_catalog_reads(), 1, "one read for the run");
    assert_eq!(asked.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(
        (sub.heartbeats(), sub.heartbeats_unread()),
        (0, 5),
        "no catalog named: every group unknown_catalog, never fresh"
    );
}

/// Core §7.5, "Another form on a heartbeat's key": a put of another form
/// on a class key or on a heartbeat's key is refused before its tag or
/// its body is read. A mutation and a frame whose bytes after the seq are
/// an owner heartbeat's body, under tag 0, and the same bytes with no
/// envelope, are each counted unread: none is read as an owner heartbeat,
/// none asks for the owner's catalog, and nothing is confirmed. The same
/// on the interface's heartbeat key, under its revision's tag. The owner
/// heartbeat itself, put after them, is read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_form_on_a_heartbeats_key_is_refused_and_never_read() {
    use zenkey::model::heartbeat::Heartbeat;
    use zenoh::bytes::Encoding;
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let asked = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let n = Arc::clone(&asked);
    let _descriptor = os
        .declare_queryable("zk2/lab/trio/@zk/instance/*")
        .callback(move |q| {
            n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let _ = zenoh::Wait::wait(q.reply_err("no descriptor here"));
        })
        .await
        .unwrap();
    let read: Arc<Mutex<Vec<HeartbeatRead>>> = Arc::default();
    let seen = Arc::clone(&read);
    let sub = bare_consumer(&rs)
        .subscribe_heartbeats(
            STATUS,
            |_: Delivery| {},
            move |h: HeartbeatRead| seen.lock().unwrap().push(h),
        )
        .await
        .unwrap();
    let class_key = owner_key("lab", "trio", Class::Host);
    let own_key = zenkey::model::heartbeat::key("lab", "trio", &iface("alpha.v1"));
    let on_class = os.declare_publisher(class_key).await.unwrap();
    let on_own = os.declare_publisher(own_key).await.unwrap();
    eventually("both heartbeat keys are matched", || async {
        on_class.matching_status().await.unwrap().matching()
            && on_own.matching_status().await.unwrap().matching()
    })
    .await;

    // CH's form bits, the top three: 0 a mutation, 5 a frame, 6 a
    // heartbeat. The run's width and the tag stay as written.
    let as_form = |payload: &[u8], form: u8| -> Vec<u8> {
        let mut bytes = payload.to_vec();
        bytes[0] = (bytes[0] & 0x1f) | (form << 5);
        bytes
    };
    let mut unread = 0;
    // On the class key: an owner heartbeat's bytes under tag 0, as a
    // mutation, as a frame, and with no envelope.
    let owner_beat = bare_heartbeat(0);
    assert_eq!(owner_beat[0], 0xc0, "form 6, a 16-bit run, tag 0");
    for (what, payload, encoding) in [
        ("a mutation", as_form(&owner_beat, 0), Encoding::ZENOH_BYTES),
        ("a frame", as_form(&owner_beat, 5), Encoding::ZENOH_BYTES),
        (
            "no envelope",
            owner_beat.clone(),
            Encoding::APPLICATION_OCTET_STREAM,
        ),
    ] {
        on_class.put(payload).encoding(encoding).await.unwrap();
        unread += 1;
        eventually(what, || async { sub.heartbeats_unread() == unread }).await;
    }
    // On the interface's heartbeat key: a heartbeat's bytes under its
    // revision's tag, the same three ways.
    let layout = Layout::of(&contract(&trio("alpha.v1")));
    let beat = Heartbeat {
        time: Time::Unknown,
        sections: vec![None; layout.collections.len()],
    }
    .envelope(
        &layout,
        Stamp {
            run: zenkey::model::wire::Run::U16(7),
            seq: Seq::new(0).unwrap(),
        },
    )
    .unwrap();
    for (what, payload, encoding) in [
        ("a mutation", as_form(&beat, 0), Encoding::ZENOH_BYTES),
        ("a frame", as_form(&beat, 5), Encoding::ZENOH_BYTES),
        (
            "no envelope",
            beat.clone(),
            Encoding::APPLICATION_OCTET_STREAM,
        ),
    ] {
        on_own.put(payload).encoding(encoding).await.unwrap();
        unread += 1;
        eventually(what, || async { sub.heartbeats_unread() == unread }).await;
    }
    assert_eq!(unread, 6);
    assert_eq!(
        (sub.owner_heartbeats(), sub.heartbeats()),
        (0, 0),
        "none read as an owner heartbeat or as a heartbeat"
    );
    assert_eq!(
        (
            sub.owner_catalog_reads(),
            asked.load(std::sync::atomic::Ordering::Relaxed)
        ),
        (0, 0),
        "refused before its owner's catalog is asked for"
    );
    assert!(read.lock().unwrap().is_empty(), "nothing told the reader");

    // The reader is live: the owner heartbeat itself is read on its key.
    on_class.put(owner_beat).await.unwrap();
    eventually("the owner heartbeat is read on its key", || async {
        sub.owner_heartbeats() == 1
    })
    .await;
    // And the heartbeat itself on its own: folded into the view.
    on_own.put(beat).await.unwrap();
    eventually("the heartbeat is read on its key", || async {
        sub.heartbeats() == 1
    })
    .await;
}

/// Behind a descriptor read that is slow to answer, at most `OWNER_QUEUE`
/// owner heartbeats wait: each one past it is read at once with no
/// catalog, counted, and confirms nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_queue_behind_a_slow_descriptor_read_is_bounded() {
    use zenkey::consumer::OWNER_QUEUE;
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    // An owner whose descriptor read is held open, never answered.
    let held: Arc<Mutex<Vec<zenoh::query::Query>>> = Arc::default();
    let h = Arc::clone(&held);
    let _descriptor = os
        .declare_queryable("zk2/lab/trio/@zk/instance/*")
        .callback(move |q| h.lock().unwrap().push(q))
        .await
        .unwrap();
    let sub = bare_consumer(&rs)
        .with_reread_timeout(Duration::from_secs(30))
        .subscribe_heartbeats(STATUS, |_: Delivery| {}, |_: HeartbeatRead| {})
        .await
        .unwrap();
    let host = owner_key("lab", "trio", Class::Host);
    let publisher = os.declare_publisher(host).await.unwrap();
    eventually("the class key is matched", || async {
        publisher.matching_status().await.unwrap().matching()
    })
    .await;
    let past = 6;
    for seq in 0..(OWNER_QUEUE as u32 + past) {
        publisher.put(bare_heartbeat(seq)).await.unwrap();
    }
    eventually("those past the bound are read with no catalog", || async {
        sub.owner_heartbeats_unqueued() == u64::from(past)
    })
    .await;
    assert_eq!(sub.owner_catalog_reads(), 1, "one read, still open");
    assert_eq!(held.lock().unwrap().len(), 1);
    assert_eq!(sub.heartbeats(), 0, "none confirmed anything");
    assert_eq!(
        sub.owner_heartbeats(),
        u64::from(past),
        "the rest wait, in order"
    );
}

/// The callback runs under no lock of its subscription's: one that calls
/// back into the subscription reads it, and sees the owner heartbeat it
/// is handed already applied.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_callback_may_call_back_into_its_subscription() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let cat = catalog();
    let (_svc, statuses, _reach) = owner(&os, Some(cat.clone())).await;
    let key = status_key("alpha.v1");
    let cell: Arc<std::sync::OnceLock<zenkey::consumer::Subscription>> = Arc::default();
    let delivered: Arc<Mutex<Vec<Stamp>>> = Arc::default();
    // What the callback read of its subscription: the owner heartbeats
    // read, the heartbeats applied, whether lost, whether confirmed.
    type Seen = Vec<(u64, u64, bool, bool)>;
    let seen: Arc<Mutex<Seen>> = Arc::default();
    let (c, d, s, k) = (
        Arc::clone(&cell),
        Arc::clone(&delivered),
        Arc::clone(&seen),
        key.clone(),
    );
    let sub = Consumer::for_tool(
        &rs,
        Arc::new(contract(&trio("alpha.v1"))),
        &["lab/trio"],
        &Default::default(),
    )
    .unwrap()
    .with_catalog(&cat)
    .subscribe_heartbeats(
        STATUS,
        move |x: Delivery| d.lock().unwrap().extend(x.inband),
        move |_: HeartbeatRead| {
            let sub = c.get().expect("set before the first owner heartbeat");
            s.lock().unwrap().push((
                sub.owner_heartbeats(),
                sub.heartbeats(),
                sub.lost(&k),
                sub.confirmed(&k).is_some(),
            ));
        },
    )
    .await
    .unwrap();
    assert!(cell.set(sub).is_ok());
    eventually("the status is matched", || async {
        statuses[0].writer().matching().await.unwrap()
    })
    .await;
    statuses[0].put("up").await.unwrap();
    eventually("the put is delivered", || async {
        !delivered.lock().unwrap().is_empty()
    })
    .await;
    let stamp = delivered.lock().unwrap()[0];
    let held = Digest::of([(key.as_str(), stamp)]);
    let host = owner_key("lab", "trio", Class::Host);
    for seq in 0..2 {
        os.put(
            &host,
            owner_heartbeat(vec![group(0, "alpha.v1", STATUS, held)], stamp, seq),
        )
        .await
        .unwrap();
        eventually("the callback read its subscription", || async {
            seen.lock().unwrap().len() == seq as usize + 1
        })
        .await;
    }
    assert_eq!(
        *seen.lock().unwrap(),
        [(1, 1, false, true), (2, 2, false, true)],
        "each handed over after it was applied, in order"
    );
}

/// An owner that names no catalog writes no index (core §8.6): a reader
/// that holds one refuses every group all the same, never guessing that
/// its own catalog is the owner's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_index_from_an_owner_that_names_no_catalog_is_refused() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let cat = catalog();
    let (_svc, statuses, _reach) = owner(&os, None).await;
    let reader = Reader::of(&rs, "alpha.v1", STATUS, Some(&cat)).await;
    let stamps = put_statuses(&statuses[..1], std::slice::from_ref(&reader)).await;
    let key = status_key("alpha.v1");
    let held = Digest::of([(key.as_str(), stamps[0])]);
    os.put(
        owner_key("lab", "trio", Class::Host),
        owner_heartbeat(vec![group(0, "alpha.v1", STATUS, held)], stamps[0], 0),
    )
    .await
    .unwrap();
    eventually("the owner heartbeat is read on its key", || async {
        reader.sub.owner_heartbeats() == 1
    })
    .await;
    assert!(reader.got().is_empty());
    assert_eq!(
        (reader.sub.heartbeats(), reader.sub.heartbeats_unread()),
        (0, 1)
    );
    assert!(reader.sub.confirmed(&key).is_none());
}

/// A reader that holds a successor of the owner's catalog reads through
/// it (core §8.6, "Through a successor"), and a group on the `link` key
/// confirms the `link` collection alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_successor_catalog_reads_the_link_keys_group() {
    let (_r1, ep) = router(None).await;
    let os = client(&ep).await;
    let rs = client(&ep).await;
    let cat = catalog();
    let mut next = cat.successor();
    let extra = contract("beacon.v1");
    next.push(extra.iface.clone(), Fingerprint::of(&extra))
        .unwrap();
    let (_svc, _statuses, reach) = owner(&os, Some(cat)).await;
    let reader = Reader::of(&rs, "charlie.v1", REACH, Some(&next)).await;
    eventually("reach is matched", || async {
        reach.writer().matching().await.unwrap()
    })
    .await;
    reach.put("near").await.unwrap();
    let stamp = reader.stamp().await;
    let key = "zk2/lab/trio/charlie.v1/state/reach";
    os.put(
        owner_key("lab", "trio", Class::Link),
        owner_heartbeat(
            vec![group(2, "charlie.v1", REACH, Digest::of([(key, stamp)]))],
            stamp,
            0,
        ),
    )
    .await
    .unwrap();
    eventually("its group is handed over", || async {
        reader.got().len() == 1
    })
    .await;
    assert_eq!(
        reader.got(),
        [(
            0,
            false,
            Some(OwnerGroup {
                class: Class::Link,
                index: 2
            }),
            HeartbeatSection::Confirmed(vec![key.to_owned()])
        )]
    );
    assert!(reader.sub.confirmed(key).is_some());
}
