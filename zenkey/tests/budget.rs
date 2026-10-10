//! `spec/scenarios/budget.md` (spec §2.7): what a window can establish
//! about the deliveries it heard (§1–§3, 0.25; #740), and the owner keeping
//! its bound and its rate (§4–§7, #741).
//!
//! §1–§3 are the measurements 0.25's "lost no delivery" rests on, run as
//! the spike ran its own (`docs/zk2/spike-report.md`): in-process routers,
//! a link between them that can be cut or stalled, and each number printed
//! for the record. Appendix B of the core cites them.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{client, eventually, link_to, router, router_via_with, router_with};
use zenkey::model::contract::{Contract, load_str};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::{Implementation, ServiceBuilder, ServiceConfig};
use zenoh::qos::{CongestionControl, Reliability};
use zenoh::sample::SampleKind;

/// `budget.v1`: one templated resource of each kind §2.7 bounds, and an
/// event with a rate. The stream is reliable and `block`, so that a window
/// of it can be lossless (§2.7, 0.25); the event's retention is short
/// enough for a member to stop being live within a test.
const BUDGET: &str = r#"
[interface]
name = "budget"
major = 1

[resources."items/{item}"]
kind = "state"
type = { raw = "text/plain" }
params = { item = "string" }
cardinality = 3

[resources."load/{cpu}"]
kind = "stream"
type = { raw = "text/plain" }
params = { cpu = "string" }
cardinality = 3
reliability = "reliable"
congestion = "block"

[resources."alarms/{src}"]
kind = "event"
type = { raw = "text/plain" }
params = { src = "string" }
cardinality = 2
rate = "burst(3/h)"
retention = "2s"
"#;

/// One test of this file at a time. The sessions of one process share
/// zenoh's receive threads (two by default), and §1's stuck reader holds
/// one for seconds: beside it, §2's timings would measure this process.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn budget() -> Contract {
    let l = load_str(
        BUDGET,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        None,
    );
    l.contract
        .unwrap_or_else(|| panic!("budget.v1 does not load:\n{}", l.report))
}

fn iface() -> IfaceId {
    "budget.v1".parse().unwrap()
}

fn one(param: &str, value: &str) -> Bindings {
    [(param.to_owned(), vec![value.to_owned()])].into()
}

/// The sequence numbers a subscriber heard, in arrival order.
type Heard = Arc<Mutex<Vec<u32>>>;

fn seq(payload: &[u8]) -> u32 {
    u32::from_le_bytes(payload[..4].try_into().expect("a sequence number"))
}

/// How many of `0..sent` never arrived, and whether what arrived came in
/// order.
fn losses(heard: &[u32], sent: u32) -> (u32, bool) {
    let set: std::collections::BTreeSet<u32> = heard.iter().copied().collect();
    let lost = (0..sent).filter(|n| !set.contains(n)).count() as u32;
    (lost, heard.windows(2).all(|w| w[0] < w[1]))
}

/// A router in a process of its own, as a deployment's routers are: this
/// test binary run again as [`router_process`]. A router shares zenoh's
/// receive threads (two by default) with every session of its process, so
/// routers and a slow reader in one process measure that process, not a
/// path (`spec/scenarios/budget.md` §1). Killed when dropped.
struct RouterProcess {
    child: std::process::Child,
    endpoint: String,
}

impl Drop for RouterProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_router(connect: Option<&str>) -> RouterProcess {
    use std::io::BufRead;
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["router_process", "--exact", "--ignored", "--nocapture"])
        .env("ZK2_ROUTER_PROCESS", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(up) = connect {
        cmd.env("ZK2_ROUTER_CONNECT", up);
    }
    let mut child = cmd.spawn().expect("a router process");
    let out = child.stdout.take().unwrap();
    let mut lines = std::io::BufReader::new(out).lines();
    let endpoint = loop {
        let line = lines
            .next()
            .expect("the router process says where it listens")
            .unwrap();
        if let Some(ep) = line.strip_prefix("ZK2-LISTENING ") {
            break ep.to_owned();
        }
    };
    // Its standard output stays open; it writes nothing more.
    std::mem::forget(lines);
    RouterProcess { child, endpoint }
}

/// The router of [`spawn_router`]: not a test, so it runs only when spawned.
#[test]
#[ignore = "a router process, spawned by the tests of this file"]
fn router_process() {
    if std::env::var("ZK2_ROUTER_PROCESS").is_err() {
        return;
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let connect = std::env::var("ZK2_ROUTER_CONNECT").ok();
        let (_r, ep) = router(connect.as_deref()).await;
        println!("\nZK2-LISTENING {ep}");
        // Until the parent closes its end, or kills it.
        let _ = tokio::task::spawn_blocking(|| {
            let mut s = String::new();
            while std::io::stdin().read_line(&mut s).is_ok_and(|n| n > 0) {
                s.clear();
            }
        })
        .await;
    });
}

/// What one run of §1 saw: the sequence numbers heard, and the owner's
/// instance-token deletes the consumer saw.
struct SlowRun {
    heard: Vec<u32>,
    deletes: usize,
    took: Duration,
}

/// One run of §1: R2 → R1 each in a process of its own; an owner, a client
/// of R2 holding an instance token, puts `sent` samples of 16 KiB as fast as
/// its publisher lets it, with the QoS given; a consumer, a client of R1,
/// takes 2 ms over each sample, and `stuck` over the 100th. It runs until a
/// second passes with nothing new heard.
async fn slow_reader(
    reliability: Reliability,
    congestion: CongestionControl,
    sent: u32,
    stuck: Duration,
) -> SlowRun {
    let r1 = spawn_router(None);
    let r2 = spawn_router(Some(&r1.endpoint));
    let owner = client(&r2.endpoint).await;
    let consumer = client(&r1.endpoint).await;
    let key = "zk2/lab/budget/budget.v1/stream/load/c0";
    let t = Instant::now();
    let heard: Heard = Arc::default();
    let h = Arc::clone(&heard);
    let _sub = consumer
        .declare_subscriber(key)
        .callback(move |s| {
            // A slow reader: zenoh's receive task waits on it.
            let n = seq(&s.payload().to_bytes());
            let wait = if n == 100 {
                stuck
            } else {
                Duration::from_millis(2)
            };
            std::thread::sleep(wait);
            h.lock().unwrap().push(n);
        })
        .await
        .unwrap();
    let deletes = Arc::new(AtomicU64::new(0));
    let d = Arc::clone(&deletes);
    let _tokens = consumer
        .liveliness()
        .declare_subscriber("zk2/lab/budget/@zk/instance/*")
        .callback(move |s| {
            if s.kind() == SampleKind::Delete {
                d.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap();
    let _token = owner
        .liveliness()
        .declare_token("zk2/lab/budget/@zk/instance/1")
        .await
        .unwrap();
    let publisher = owner
        .declare_publisher(key)
        .reliability(reliability)
        .congestion_control(congestion)
        .await
        .unwrap();
    eventually("the owner's publisher matches the consumer", || async {
        publisher.matching_status().await.unwrap().matching()
    })
    .await;
    let mut payload = vec![0u8; 16 * 1024];
    for n in 0..sent {
        payload[..4].copy_from_slice(&n.to_le_bytes());
        publisher.put(payload.clone()).await.unwrap();
    }
    // A stuck reader hears nothing while it is stuck.
    tokio::time::sleep(stuck).await;
    let mut last = usize::MAX;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let now = heard.lock().unwrap().len();
        if now == last {
            break;
        }
        last = now;
    }
    // A withdrawal still on its way.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let heard = heard.lock().unwrap().clone();
    SlowRun {
        heard,
        deletes: deletes.load(Ordering::SeqCst) as usize,
        took: t.elapsed(),
    }
}

/// §1: a reliable, `block` path loses nothing to a slow reader: the queue
/// that fills waits rather than drops, back to the owner's put, and every
/// sample arrives in order. The same path at best effort and `drop` loses
/// samples, and nothing the consumer receives says so. A reader stuck
/// longer than the queue's `wait_before_close` (5 s) is the one way the
/// reliable path loses one: its router closes the transport, and the
/// consumer sees the owner's token withdrawn (Appendix B; 0.25).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_a_reliable_blocking_path_loses_nothing_to_a_slow_reader() {
    let _serial = SERIAL.lock().await;
    const SENT: u32 = 1000;
    let slow = Duration::from_millis(2);
    for (what, rel, cc, stuck) in [
        (
            "reliable+block",
            Reliability::Reliable,
            CongestionControl::Block,
            slow,
        ),
        (
            "best_effort+drop",
            Reliability::BestEffort,
            CongestionControl::Drop,
            slow,
        ),
        (
            "reliable+block, the reader stuck 7 s",
            Reliability::Reliable,
            CongestionControl::Block,
            Duration::from_secs(7),
        ),
    ] {
        let run = slow_reader(rel, cc, SENT, stuck).await;
        let (lost, ordered) = losses(&run.heard, SENT);
        eprintln!(
            "§1 {what}: {} of {SENT} heard, {lost} lost, in order: {ordered}, the owner's \
             token withdrawn {} time(s), in {:?}",
            run.heard.len(),
            run.deletes,
            run.took
        );
        assert!(ordered, "{what}: in order");
        match (rel, stuck > slow) {
            (Reliability::Reliable, false) => {
                assert_eq!(lost, 0, "{what}: a reliable, blocking path drops nothing");
                assert_eq!(run.heard.len() as u32, SENT, "{what}: nor duplicates");
                assert_eq!(run.deletes, 0, "{what}: and nothing failed");
            }
            (Reliability::Reliable, true) => {
                assert!(lost > 0, "{what}: the stuck reader's router gave up on it");
                assert!(
                    run.deletes > 0,
                    "{what}: by closing the transport, which the consumer sees"
                );
            }
            (Reliability::BestEffort, _) => {
                assert!(lost > 0, "{what}: a full queue drops a droppable sample");
                assert_eq!(run.deletes, 0, "{what}: and nothing says so");
            }
        }
    }
}

/// The owner of §2: `budget.v1` at `lab/budget` on `session`, publishing a
/// counter on `stream/load/c0` every 10 ms; returns the service, how many
/// it put so far, and the task.
async fn counting_owner(
    session: &zenoh::Session,
) -> (zenkey::Service, Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
    let mut b = ServiceBuilder::new(session, ServiceConfig::new("lab/budget".parse().unwrap()));
    b.implement(Implementation::new(budget())).unwrap();
    let w = b
        .declare_writer(&iface(), "stream/load/{cpu}", &one("cpu", "c0"))
        .await
        .unwrap();
    for r in ["state/items/{item}", "events/alarms/{src}"] {
        b.expose(&iface(), r).unwrap();
    }
    let svc = b.start().await.unwrap();
    let put = Arc::new(AtomicU64::new(0));
    let p = Arc::clone(&put);
    let task = tokio::spawn(async move {
        loop {
            let n = p.load(Ordering::SeqCst) as u32;
            if w.put(n.to_le_bytes().to_vec()).await.is_ok() {
                p.fetch_add(1, Ordering::SeqCst);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    (svc, put, task)
}

/// What the consumer of §2 watches: the owner's instance tokens, each
/// delete stamped with when it arrived, and the counter's values.
struct Watcher {
    deletes: Arc<Mutex<Vec<Instant>>>,
    heard: Heard,
    _tokens: zenoh::pubsub::Subscriber<()>,
    _data: zenoh::pubsub::Subscriber<()>,
}

async fn watcher(consumer: &zenoh::Session) -> Watcher {
    let deletes: Arc<Mutex<Vec<Instant>>> = Arc::default();
    let d = Arc::clone(&deletes);
    let tokens = consumer
        .liveliness()
        .declare_subscriber("zk2/lab/budget/@zk/instance/*")
        .callback(move |s| {
            if s.kind() == SampleKind::Delete {
                d.lock().unwrap().push(Instant::now());
            }
        })
        .await
        .unwrap();
    let heard: Heard = Arc::default();
    let h = Arc::clone(&heard);
    let data = consumer
        .declare_subscriber("zk2/lab/budget/budget.v1/stream/load/*")
        .callback(move |s| h.lock().unwrap().push(seq(&s.payload().to_bytes())))
        .await
        .unwrap();
    Watcher {
        deletes,
        heard,
        _tokens: tokens,
        _data: data,
    }
}

/// §2: a failed path withdraws the owner's instance token at a reader on
/// the far side — at once when the link between R2 and R1 closes, and one
/// lease after it when the link goes silent — and the samples the owner put
/// meanwhile never arrive. The routers run a 1 s lease, so the silent case
/// takes a second, not zenoh's default ten (the spike measured those:
/// 9,961–10,047 ms, S10; 10.0 s, S2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_failed_path_withdraws_the_owners_token() {
    let _serial = SERIAL.lock().await;
    const LEASE: Duration = Duration::from_secs(1);
    let lease = [("transport/link/tx/lease", "1000")];
    let (_r1, ep1) = router_with(None, &lease).await;
    let between = link_to(&ep1).await;
    let (_r2, ep2) = router_via_with(&between, &lease).await;
    let own = link_to(&ep1).await;
    let owner = client(&ep2).await;
    let consumer = client_via(&own, &lease).await;
    let w = watcher(&consumer).await;
    let (_svc, put, task) = counting_owner(&owner).await;
    eventually("the consumer hears the owner", || async {
        w.heard.lock().unwrap().len() > 20
    })
    .await;

    for (how, link, silent) in [
        ("R2-R1 closed", &between, false),
        ("R2-R1 silent", &between, true),
        ("the consumer's own link closed", &own, false),
        ("the consumer's own link silent", &own, true),
    ] {
        let before = w.deletes.lock().unwrap().len();
        let at = Instant::now();
        let from = put.load(Ordering::SeqCst) as u32;
        if silent {
            link.stall();
        } else {
            link.cut();
        }
        let deadline = Instant::now() + LEASE * 5;
        while w.deletes.lock().unwrap().len() == before && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let seen = w
            .deletes
            .lock()
            .unwrap()
            .get(before)
            .map(|d| d.duration_since(at));
        let to = put.load(Ordering::SeqCst) as u32;
        link.cut();
        link.heal();
        eventually("the consumer hears the owner again", || async {
            w.heard.lock().unwrap().last().is_some_and(|n| *n > to + 5)
        })
        .await;
        let heard = w.heard.lock().unwrap().clone();
        let lost = (from..to).filter(|n| !heard.contains(n)).count();
        eprintln!(
            "§2 {how}: the token's delete arrived {seen:?} after the link failed; {lost} of \
             the {} sample(s) put meanwhile never arrived",
            to - from
        );
        let seen = seen.unwrap_or_else(|| panic!("{how}: the owner's token was not withdrawn"));
        if silent {
            assert!(
                seen >= LEASE / 2 && seen <= LEASE + Duration::from_secs(2),
                "{how}: a silent link is seen failing at its lease: {seen:?}"
            );
            assert!(
                lost > 0,
                "{how}: a silent link loses what was put meanwhile"
            );
        } else {
            assert!(
                seen < LEASE,
                "{how}: a closed link is seen at once: {seen:?}"
            );
        }
    }
    task.abort();
}

/// A client of the router behind `link`, with more configuration inserted.
async fn client_via(link: &common::Link, extra: &[(&str, &str)]) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{}\"]", link.endpoint()))
        .unwrap();
    c.insert_json5(
        "connect/retry",
        "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
    )
    .unwrap();
    for (k, v) in extra {
        c.insert_json5(k, v).unwrap();
    }
    zenoh::open(c).await.expect("client")
}

/// §3: zenoh-ext's sample-miss detection reports a gap between two samples
/// of one publisher it heard, and never the samples before the first it
/// heard: an unknown publisher's first sample is taken as its start
/// (`advanced_subscriber.rs`, `handle_sample`). So a member none of whose
/// samples arrived leaves no trace there either.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_miss_detection_cannot_see_a_publisher_it_never_heard() {
    let _serial = SERIAL.lock().await;
    use zenoh_ext::{
        AdvancedPublisherBuilderExt, AdvancedSubscriberBuilderExt, MissDetectionConfig,
    };
    let (_r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (_r2, ep2) = router_via_with(&link, &[]).await;
    let owner = client(&ep2).await;
    let consumer = client(&ep1).await;
    let key = "zk2/lab/budget/budget.v1/stream/load/c0";
    let publisher = owner
        .declare_publisher(key)
        .reliability(Reliability::Reliable)
        .congestion_control(CongestionControl::Block)
        .advanced()
        .sample_miss_detection(MissDetectionConfig::default())
        .await
        .unwrap();
    let mut n: u32 = 0;
    // Five samples no subscriber hears, gone before one is declared.
    for _ in 0..5 {
        publisher.put(n.to_le_bytes().to_vec()).await.unwrap();
        n += 1;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let heard: Heard = Arc::default();
    let h = Arc::clone(&heard);
    let sub = consumer
        .declare_subscriber(key)
        .advanced()
        .callback(move |s| h.lock().unwrap().push(seq(&s.payload().to_bytes())))
        .await
        .unwrap();
    let missed = Arc::new(AtomicU64::new(0));
    let m = Arc::clone(&missed);
    let _misses = sub
        .sample_miss_listener()
        .callback(move |miss| {
            m.fetch_add(miss.nb() as u64, Ordering::SeqCst);
        })
        .await
        .unwrap();
    let put_until_heard = async |n: &mut u32| {
        let before = heard.lock().unwrap().len();
        let deadline = Instant::now() + common::SETTLE;
        while heard.lock().unwrap().len() == before {
            assert!(Instant::now() < deadline, "never heard");
            publisher.put(n.to_le_bytes().to_vec()).await.unwrap();
            *n += 1;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    put_until_heard(&mut n).await;
    let first = heard.lock().unwrap()[0];
    let before = missed.load(Ordering::SeqCst);
    eprintln!("§3 the first sample heard is #{first}; misses reported: {before}");
    assert!(first >= 5, "the first five were never heard");
    assert_eq!(before, 0, "and no miss is reported for them");

    // A gap between two samples heard is reported.
    link.cut();
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..3 {
        publisher.put(n.to_le_bytes().to_vec()).await.unwrap();
        n += 1;
    }
    link.heal();
    put_until_heard(&mut n).await;
    let after = missed.load(Ordering::SeqCst);
    eprintln!("§3 after a cut: misses reported: {after}");
    assert!(
        after >= 3,
        "a gap after a sample heard is reported: {after}"
    );
}
