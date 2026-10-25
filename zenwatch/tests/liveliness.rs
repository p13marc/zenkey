//! The presence rules against live services (#614, FL2): an in-process
//! router, owners brought up and stopped with the runtime's own
//! `ServiceBuilder` in a deployment namespace, and the daemon on its two
//! sessions — one in the namespace, one in none.
//!
//! An owner stops: `instance-gone */tc` fires for its address, carrying the
//! projection's status move, and `liveliness-gone` on its interface token
//! (a zk2 presence selector, riding the same projection) fires beside it.
//! The owner restarts — a new instance, the same address — and both
//! resolve. Beside them, `liveliness-gone` on a foreign token (another
//! application's, on the shared bus) keeps the daemon's own ledger: gone is
//! one firing, back is one resolve. Everything up when the daemon joins is
//! the baseline, never news.

mod util;

use std::time::Duration;

use util::engine::{self, Capture, Options};
use util::zk2;
use zenkey_fleet::{AddressStatus, CondState, PresenceChange, PresenceSource};
use zenwatch::sinks::{NoticeKind, Notification, Outgoing};

const NS: &str = "site";
const FOREIGN: &str = "demo/held/p1/alive";

fn find<'o>(got: &'o [Outgoing], id: &str, kind: NoticeKind) -> Option<&'o Notification> {
    got.iter()
        .map(|o| &o.notification)
        .find(|n| n.id == id && n.kind == kind)
}

async fn seen(ops: &Capture, id: &str, kind: NoticeKind) -> Notification {
    let got = engine::until(ops, &format!("{kind:?} {id}"), |s| {
        find(s, id, kind).is_some()
    })
    .await;
    find(&got, id, kind).expect("seen").clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_that_stops_fires_instance_gone_and_its_restart_resolves_it() {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let tool = zk2::client_ns(&ep, Some(NS)).await;
    let other_app = zk2::client(&ep).await;
    let ping = zk2::scenario("ping.v1");

    // Up before the daemon joins: an owner, and a foreign token.
    let svc = zk2::bring_up(&owners, zk2::config("h1/tc"), std::slice::from_ref(&ping)).await;
    let token = other_app
        .liveliness()
        .declare_token(FOREIGN)
        .await
        .expect("a foreign token");
    zk2::tokens(&tool, "zk2/h1/tc/@zk/instance/*", 1).await;
    zk2::tokens(&other_app, FOREIGN, 1).await;

    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![
            engine::rule("tc-gone", "instance-gone */tc", "error"),
            engine::rule(
                "ping-gone",
                &format!("liveliness-gone {NS}/zk2/*/tc/@zk/alive/ping.v1/**"),
                "warning",
            ),
            engine::rule("held-gone", "liveliness-gone demo/held/*/alive", "warning"),
        ],
        ops.clone(),
        Options::default(),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(
        ops.lock().unwrap().is_empty(),
        "everything up at the first look is the baseline: {:?}",
        ops.lock().unwrap()
    );

    // 1. The owner stops: its address is gone, and so is its interface.
    svc.close().await.expect("close");
    let n = seen(&ops, "tc-gone:h1/tc", NoticeKind::Presence).await;
    assert_eq!(n.state, CondState::Firing);
    assert_eq!(n.prior, Some(CondState::Ok), "the baseline was recorded");
    assert_eq!(n.rule_kind, "instance-gone");
    assert_eq!(n.severity, "error");
    assert_eq!(n.title, "error tc-gone firing");
    assert!(n.message.contains("address: h1/tc"), "{}", n.message);
    assert!(
        n.evidence.contains("no instance token of h1/tc"),
        "{}",
        n.evidence
    );
    // Heard withdrawn, it was unobservable; the read the withdrawal owed
    // said down, and that read is the deciding move (core §8.1, 0.37).
    let moved = n.presence.as_ref().expect("the deciding move");
    assert_eq!(moved.address.to_string(), "h1/tc");
    assert_eq!(moved.source, PresenceSource::Seed);
    assert_eq!(
        moved.change,
        PresenceChange::Status {
            from: AddressStatus::Unobservable,
            to: AddressStatus::Down
        }
    );
    let wire = serde_json::to_value(&n).expect("serializes");
    assert_eq!(wire["presence"]["change"]["kind"], "status");
    assert_eq!(wire["presence"]["change"]["to"], "down");
    let n = seen(&ops, "ping-gone:h1/tc/ping.v1", NoticeKind::Liveliness).await;
    assert_eq!(n.state, CondState::Firing);
    assert_eq!(n.rule_kind, "liveliness-gone");
    assert!(
        matches!(
            n.presence.as_ref().map(|t| &t.change),
            Some(PresenceChange::InterfaceGone { .. })
        ),
        "{:?}",
        n.presence
    );

    // 2. It restarts — a new instance at the same address: both resolve.
    let svc = zk2::bring_up(&owners, zk2::config("h1/tc"), std::slice::from_ref(&ping)).await;
    let n = seen(&ops, "tc-gone:h1/tc", NoticeKind::Resolved).await;
    assert_eq!(n.state, CondState::Ok);
    assert_eq!(n.prior, Some(CondState::Firing));
    seen(&ops, "ping-gone:h1/tc/ping.v1", NoticeKind::Resolved).await;

    // 3. The foreign token goes, and comes back.
    token.undeclare().await.expect("undeclare");
    let id = format!("held-gone:{FOREIGN}");
    let n = seen(&ops, &id, NoticeKind::Liveliness).await;
    assert_eq!(n.state, CondState::Firing);
    assert_eq!(n.prior, Some(CondState::Ok));
    assert!(
        n.message.contains(&format!("key: {FOREIGN}")),
        "{}",
        n.message
    );
    assert!(n.presence.is_none(), "not the projection's");
    let token = other_app
        .liveliness()
        .declare_token(FOREIGN)
        .await
        .expect("a foreign token");
    let n = seen(&ops, &id, NoticeKind::Resolved).await;
    assert_eq!(n.prior, Some(CondState::Firing));

    let summary = running.stop().await;
    assert_eq!(summary.outgoing, 6, "{:?}", ops.lock().unwrap());
    assert_eq!(summary.delivered, 6);
    assert_eq!(summary.dropped, 0);
    assert!(
        !ops.lock()
            .unwrap()
            .iter()
            .any(|o| o.notification.state == CondState::Unobservable),
        "nothing was lost, so nothing was unobservable"
    );
    svc.close().await.expect("close");
    token.undeclare().await.expect("undeclare");
}

/// An exact address absent at the first look, under a complete read, is
/// down: remembered rather than paged one by one, and said once in a
/// `baseline` notification at the rule's severity. Its first appearance is
/// then a `resolved`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_address_down_at_start_is_one_baseline_and_its_arrival_resolves_it() {
    let (_router, ep) = zk2::router().await;
    let owners = zk2::client_ns(&ep, Some(NS)).await;
    let ops = Capture::default();
    let running = engine::start(
        zk2::daemon_bus(&ep, NS).await,
        vec![engine::rule("h9-gone", "instance-gone h9/tc", "error")],
        ops.clone(),
        Options::default(),
    )
    .await;
    let n = seen(&ops, "h9-gone:baseline", NoticeKind::Baseline).await;
    assert_eq!(n.state, CondState::Firing);
    assert_eq!(n.severity, "error");
    assert!(n.message.contains("- gone: h9/tc"), "{}", n.message);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        ops.lock().unwrap().len(),
        1,
        "the address itself is remembered, not paged: {:?}",
        ops.lock().unwrap()
    );

    let svc = zk2::bring_up(&owners, zk2::config("h9/tc"), &[zk2::scenario("ping.v1")]).await;
    let n = seen(&ops, "h9-gone:h9/tc", NoticeKind::Resolved).await;
    assert_eq!(n.prior, Some(CondState::Firing));
    let summary = running.stop().await;
    assert_eq!(summary.discipline.baseline, 1, "the assumed address");
    svc.close().await.expect("close");
}

/// The token ledger reads beside a monitor of its own (#830, core §8.1,
/// 0.37): `TokenFeed::open` refuses the monitor's own session, or any
/// session holding a liveliness subscriber, before it reads; on the daemon's
/// raw session it reads, and a token heard withdrawn is not gone until a
/// read says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_token_ledger_reads_beside_a_monitor_of_its_own() {
    use zenwatch::liveliness::{TokenBasis, TokenFeed};
    let (_router, ep) = zk2::router().await;
    let (raw, watch) = (zk2::client(&ep).await, zk2::client(&ep).await);
    let other_app = zk2::client(&ep).await;
    let token = other_app
        .liveliness()
        .declare_token(FOREIGN)
        .await
        .expect("a foreign token");
    zk2::tokens(&other_app, FOREIGN, 1).await;
    let spec = zenkey_fleet::MonitorSpec {
        liveliness: vec![FOREIGN.to_owned()],
        stats_tick: Duration::from_secs(3600),
        ..zenkey_fleet::MonitorSpec::default()
    };
    let monitor = zenkey_fleet::Monitor::start(&watch, spec)
        .await
        .expect("a monitor");
    let refused = TokenFeed::open(&watch, &monitor, vec![FOREIGN.to_owned()], zk2::T)
        .await
        .expect_err("refused");
    assert!(refused.not_asked().is_some(), "{refused}");

    let mut feed = TokenFeed::open(&raw, &monitor, vec![FOREIGN.to_owned()], zk2::T)
        .await
        .expect("the raw session reads");
    assert_eq!(feed.basis(), TokenBasis::Complete);
    assert!(feed.held().contains(FOREIGN));

    token.undeclare().await.expect("undeclare");
    let mut moves = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !moves
        .iter()
        .any(|m: &zenwatch::liveliness::TokenMove| !m.held)
    {
        let got = tokio::time::timeout_at(deadline, feed.next())
            .await
            .expect("the read in time")
            .expect("the monitor runs");
        moves.extend(got);
    }
    let gone = moves.iter().find(|m| !m.held).expect("gone");
    assert_eq!(
        gone.source,
        PresenceSource::Seed,
        "a read said so: {moves:?}"
    );
    assert!(feed.withdrawn().is_empty());
    monitor.shutdown().await.expect("shutdown");
}

/// A watch session reaching `endpoint` through a TCP proxy the test can
/// stall (both ways, closing nothing), whose runtime ends a liveliness GET
/// its router has not finished at 300 ms (`routing/interests/timeout`): its
/// round trip cannot stand while the proxy is stalled (#851).
async fn stallable_watch(
    endpoint: &str,
) -> (
    zenoh::Session,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let target = endpoint.trim_start_matches("tcp/").to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxied = format!("tcp/{}", listener.local_addr().unwrap());
    let stalled = Arc::new(AtomicBool::new(false));
    let st = Arc::clone(&stalled);
    tokio::spawn(async move {
        while let Ok((down, _)) = listener.accept().await {
            let Ok(up) = tokio::net::TcpStream::connect(&target).await else {
                continue;
            };
            let (dr, dw) = down.into_split();
            let (ur, uw) = up.into_split();
            for (mut from, mut to) in [(dr, uw), (ur, dw)] {
                let st = Arc::clone(&st);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        while st.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        let n = match from.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        while st.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(5)).await;
                        }
                        if to.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        }
    });
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", "\"client\"".to_owned()),
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("connect/endpoints", format!("[\"{proxied}\"]")),
        ("routing/interests/timeout", "300".to_owned()),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    (zenoh::open(c).await.expect("a watch session"), stalled)
}

/// The token ledger while its monitor's round trip does not stand (#851,
/// core §8.1, 0.41): its read is possibly incomplete, saying the barrier is
/// not established, so the rule's observer is unobservable, never ok; it
/// reads again, making the round trip first, and once one stands its basis
/// is complete.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_token_ledger_is_incomplete_while_its_round_trip_does_not_stand() {
    use std::sync::atomic::Ordering;
    use zenwatch::liveliness::{TokenBasis, TokenFeed};
    let (_router, ep) = zk2::router().await;
    let raw = zk2::client(&ep).await;
    let other_app = zk2::client(&ep).await;
    let _token = other_app
        .liveliness()
        .declare_token(FOREIGN)
        .await
        .expect("a foreign token");
    zk2::tokens(&raw, FOREIGN, 1).await;
    let (watch, stalled) = stallable_watch(&ep).await;
    let spec = zenkey_fleet::MonitorSpec {
        liveliness: vec![FOREIGN.to_owned()],
        stats_tick: Duration::from_secs(3600),
        ..zenkey_fleet::MonitorSpec::default()
    };
    stalled.store(true, Ordering::SeqCst);
    let monitor = zenkey_fleet::Monitor::start(&watch, spec)
        .await
        .expect("a monitor");
    assert_eq!(monitor.settled(), Some(false));
    let mut feed = TokenFeed::open(&raw, &monitor, vec![FOREIGN.to_owned()], zk2::T)
        .await
        .expect("the raw session reads");
    assert_eq!(
        feed.basis(),
        TokenBasis::Incomplete {
            cut_off_during: false,
            unsettled: true
        }
    );
    assert!(feed.held().contains(FOREIGN), "what the read showed");
    assert!(
        matches!(
            zenwatch::publish::Observer::from(feed.basis()),
            zenwatch::publish::Observer::Incomplete { .. }
        ),
        "the observer is possibly incomplete, never complete"
    );

    stalled.store(false, Ordering::SeqCst);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while feed.basis() != TokenBasis::Complete {
        tokio::time::timeout_at(deadline, feed.next())
            .await
            .expect("a complete read in time")
            .expect("the monitor runs");
    }
    assert_eq!(monitor.settled(), Some(true));
    assert!(feed.held().contains(FOREIGN));
    monitor.shutdown().await.expect("shutdown");
}
