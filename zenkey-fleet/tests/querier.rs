//! The declared-query chokepoint (#37), proven against real zenoh — the
//! RFC 05 §2.1 checklist must hold on the *declared* path exactly as
//! `fleet_get` pins it on the one-shot path: target `All` (a `complete`
//! queryable must not collapse the fleet), consolidation `None`, and
//! attribution by each reply's own key.
//!
//! Self-contained like `lazy.rs`: two in-process peers, explicit endpoints,
//! no scouting, no external router. Ports 7481-7483 (disjoint from every
//! other test binary).

use std::time::Duration;

use zenkey_fleet::{Answer, declare_repeating};

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const HOST_A: &str = "v1/h-aaaaaaaaaaaa/@rpc/sysinfo/introspect";
const HOST_B: &str = "v1/h-bbbbbbbbbbbb/@rpc/sysinfo/introspect";
const SELECTOR: &str = "v1/*/@rpc/sysinfo/introspect";

/// Two queryables, one declared `complete` — the exact configuration that
/// makes `BestMatching` collapse a fleet to one reply. The declared path
/// must still hear both, each attributed by its own reply key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_complete_queryable_does_not_collapse_the_declared_fleet() {
    let (a, b) = peer_pair(7481).await;

    let _qa = a
        .declare_queryable(HOST_A)
        .complete(true)
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply(HOST_A, "from-a").await.unwrap();
            });
        })
        .await
        .expect("queryable a");
    let _qb = a
        .declare_queryable(HOST_B)
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                q.reply(HOST_B, "from-b").await.unwrap();
            });
        })
        .await
        .expect("queryable b");

    let repeating = declare_repeating(&b, "", SELECTOR, Duration::from_secs(5))
        .await
        .expect("declare");
    // Routing propagation is async; retry bounded until both peers answer.
    let answers = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let answers = repeating.fetch().await.expect("fetch");
            if answers.len() >= 2 {
                break answers;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both queryables should answer within 5s");
    let mut origins: Vec<&str> = answers.iter().map(|a| a.origin.as_str()).collect();
    origins.sort_unstable();
    assert_eq!(
        origins,
        vec!["h-aaaaaaaaaaaa", "h-bbbbbbbbbbbb"],
        "target All + reply-key attribution must survive a complete queryable"
    );
    repeating.undeclare().await.expect("undeclare");
}

/// Parameters ride per get — the declared keyexpr stays parameter-free, and
/// each `fetch_with` delivers its own parameters to the queryable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parameters_ride_per_get_not_in_the_declared_key() {
    let (a, b) = peer_pair(7482).await;

    let _q = a
        .declare_queryable(HOST_A)
        .callback(|query| {
            let params = query.parameters().to_string();
            let q = query.clone();
            tokio::spawn(async move {
                q.reply(HOST_A, params).await.unwrap();
            });
        })
        .await
        .expect("queryable");

    let repeating = declare_repeating(&b, "", SELECTOR, Duration::from_secs(5))
        .await
        .expect("declare");
    assert!(
        !repeating.key().contains('?'),
        "the declared keyexpr must never carry parameters"
    );

    // Routing propagation is async; the first answered fetch is the start
    // of the assertion, bounded like every wait in this suite.
    let first = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let answers = repeating
                .fetch_with("round=1", None)
                .await
                .expect("fetch 1");
            if !answers.is_empty() {
                break answers;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("a queryable should answer within 5s");
    let second = repeating
        .fetch_with("round=2", None)
        .await
        .expect("fetch 2");
    for (answers, expected) in [(&first, "round=1"), (&second, "round=2")] {
        assert_eq!(answers.len(), 1);
        let Answer::Value(bytes) = &answers[0].answer else {
            panic!("expected a value reply");
        };
        assert_eq!(
            String::from_utf8_lossy(&bytes.to_bytes()),
            expected,
            "each get must carry its own parameters"
        );
    }
    repeating.undeclare().await.expect("undeclare");
}
