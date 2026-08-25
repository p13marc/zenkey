//! The declared-query chokepoint (#37), proven against real zenoh — the
//! RFC 05 §2.1 checklist must hold on the *declared* path exactly as
//! `fleet_get` pins it on the one-shot path: target `All` (a `complete`
//! queryable must not collapse the fleet), consolidation `None`, and
//! attribution by each reply's own key.
//!
//! Self-contained like `lazy.rs`: two in-process peers, explicit endpoints,
//! no scouting, no external router.
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

use zenkey_fleet::{Answer, declare_repeating};

mod util;
use util::peer_pair;

const HOST_A: &str = "v1/h-aaaaaaaaaaaa/@rpc/sysinfo/introspect";
const HOST_B: &str = "v1/h-bbbbbbbbbbbb/@rpc/sysinfo/introspect";
const SELECTOR: &str = "v1/*/@rpc/sysinfo/introspect";

/// Two queryables, one declared `complete` — the exact configuration that
/// makes `BestMatching` collapse a fleet to one reply. The declared path
/// must still hear both, each attributed by its own reply key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_complete_queryable_does_not_collapse_the_declared_fleet() {
    let (a, b) = peer_pair().await;

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

    let repeating = declare_repeating(
        &zenkey_fleet::Fleet::new(&b, ""),
        SELECTOR,
        Duration::from_secs(5),
    )
    .await
    .expect("declare");
    // Routing propagation is async; retry bounded until both peers answer.
    let answers = tokio::time::timeout(util::SETTLE, async {
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
    let (a, b) = peer_pair().await;

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

    let repeating = declare_repeating(
        &zenkey_fleet::Fleet::new(&b, ""),
        SELECTOR,
        Duration::from_secs(5),
    )
    .await
    .expect("declare");
    assert!(
        !repeating.key().contains('?'),
        "the declared keyexpr must never carry parameters"
    );

    // Routing propagation is async; the first answered fetch is the start
    // of the assertion, bounded like every wait in this suite.
    let first = tokio::time::timeout(util::SETTLE, async {
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

/// #339: the fan-in is bounded, and the bound says what it cost.
///
/// One queryable answering many times is the cheap stand-in for the case the
/// issue names — a `**` sweep against a router with a large storage — and it
/// exercises the same drain. Under a bound of three, three answers come back
/// and `GetOpts::elided` names the rest: the memory is bounded, the count is
/// exact, and the rendering has what it needs to say "a sample, not all of
/// them" (RFC 13 §3 O6).
///
/// Unbounded, this loop kept every reply and its refcounted payload, and
/// nothing anywhere said how many that was.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reply_bound_keeps_what_it_says_and_counts_the_rest() {
    const REPLIES: usize = 40;
    const KEEP: usize = 3;

    let (a, b) = peer_pair().await;
    let _many = a
        .declare_queryable(HOST_A)
        .callback(|query| {
            let q = query.clone();
            tokio::spawn(async move {
                for _ in 0..REPLIES {
                    if q.reply(HOST_A, "one-of-many").await.is_err() {
                        return;
                    }
                }
            });
        })
        .await
        .expect("queryable");

    let fleet = zenkey_fleet::Fleet::new(&b, "");
    // Unbounded first, to learn how many replies actually landed — the point
    // is the *relationship* between the two runs, not a hard-coded 40 that a
    // dropped reply would turn into a flake.
    let all = zenkey_fleet::GetOpts::new(Duration::from_secs(5)).max_replies(usize::MAX);
    let total = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let answers = zenkey_fleet::fleet_get(&fleet, SELECTOR, &all)
                .await
                .expect("get");
            if answers.len() > KEEP {
                return answers.len();
            }
        }
    })
    .await
    .expect("the queryable answered");
    assert_eq!(all.elided(), 0, "an unbounded read hides nothing");

    let bounded = zenkey_fleet::GetOpts::new(Duration::from_secs(5)).max_replies(KEEP);
    let answers = zenkey_fleet::fleet_get(&fleet, SELECTOR, &bounded)
        .await
        .expect("get");
    assert_eq!(answers.len(), KEEP, "it kept exactly what it said it would");
    assert_eq!(
        answers.len() as u64 + bounded.elided(),
        total as u64,
        "every reply is in hand or in the ledger: kept {}, elided {}",
        answers.len(),
        bounded.elided()
    );
    assert!(matches!(answers[0].answer, Answer::Value(_)));
}
