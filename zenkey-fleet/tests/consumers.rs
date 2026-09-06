//! The consumers join over a live admin space (#224): declared readers,
//! ranked by relation, attributed on evidence — and the honesty when no
//! admin space answers at all.
//!
//! Same adminspace-config fixture as origin_attach.rs and topology.rs
//! (#122's passthrough). Ports are ephemeral (`util::endpoint`), so two
//! test runs at once cannot collide.

use std::time::Duration;

mod util;
use util::{admin_config, endpoint, peer_pair};

use zenkey_fleet::report::{AdminAnswer, Relation};

const TOKEN: &str = "v1/h-cccccccccccc/state/demo/alive";
const NARROW: &str = "v1/h-cccccccccccc/state/demo/health";
const TARGET: &str = "v1/h-cccccccccccc/state/demo/**";

/// A peer serving its admin space declares one narrow subscriber and one
/// `**` subscriber, plus its alive token; the asking peer declares its own.
/// The join ranks narrower before total, flags the wildcard, names the
/// asking session as itself, and states exactly one admin space answered.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_narrow_and_a_total_subscriber_rank_by_relation() {
    let file = admin_config();
    let endpoint = endpoint();
    let serving = zenkey_fleet::open_with_config(
        Some(&file),
        &[],
        std::slice::from_ref(&endpoint),
        Some(false),
    )
    .await
    .expect("serving session");
    let asking = zenkey_fleet::bus::session::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("asking session");

    let _token = serving
        .liveliness()
        .declare_token(TOKEN)
        .await
        .expect("declare token");
    let _narrow = serving
        .declare_subscriber(NARROW)
        .await
        .expect("declare the narrow subscriber");
    let _wide = serving
        .declare_subscriber("**")
        .await
        .expect("declare the total subscriber");
    let _own = asking
        .declare_subscriber(NARROW)
        .await
        .expect("declare the asking session's own subscriber");

    let fleet = zenkey_fleet::Fleet::new(&asking, "");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let report = loop {
        let r = zenkey_fleet::consumers(&fleet, TARGET, Duration::from_millis(500))
            .await
            .expect("a quiet admin space is a reading, not an error");
        // The asking session's own declaration reaches the serving peer's
        // admin space by interest propagation; wait for all three.
        if r.rows.len() >= 3 {
            break r;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the admin space never served the declarations: {r:?}"
        );
    };

    assert_eq!(report.target, TARGET);
    assert!(
        matches!(report.admin, AdminAnswer::Answered { answered: 1, .. }),
        "one admin space serves: {:?}",
        report.admin
    );
    assert!(
        report.asked.iter().any(|a| a == "@/*/*/subscriber/**"),
        "the sweep states what it asked: {:?}",
        report.asked
    );
    assert_eq!(report.self_zid, asking.zid().to_string());

    let serving_zid = serving.zid().to_string();
    let relations: Vec<Relation> = report.rows.iter().map(|r| r.relation).collect();
    assert!(
        relations.windows(2).all(|w| w[0] <= w[1]),
        "ranked most-specific first: {relations:?}"
    );
    let narrow: Vec<_> = report.rows.iter().filter(|r| r.keyexpr == NARROW).collect();
    assert_eq!(
        narrow.len(),
        2,
        "one per declaring session: {:?}",
        report.rows
    );
    for r in &narrow {
        assert_eq!(r.relation, Relation::Narrower);
        assert!(!r.total_wildcard);
    }
    let own = narrow
        .iter()
        .find(|r| r.zid == report.self_zid)
        .expect("the tool's own declaration appears in its own results");
    assert!(own.is_self, "and is named as such");
    let theirs = narrow
        .iter()
        .find(|r| r.zid == serving_zid)
        .expect("the serving peer's declaration, attributed to its session");
    assert!(!theirs.is_self);
    // The token attaches the origin to the serving session only when the
    // admin sources name it — if they do, the row must carry it.
    if !theirs.origins.is_empty() {
        assert_eq!(theirs.origins, ["h-cccccccccccc"]);
    }

    let total = report
        .rows
        .iter()
        .find(|r| r.keyexpr == "**")
        .expect("the `**` subscriber is a row");
    assert_eq!(total.relation, Relation::Total);
    assert!(total.total_wildcard, "shown as intersecting everything");
    assert_eq!(
        report.rows.last().map(|r| r.relation),
        Some(Relation::Total)
    );

    std::fs::remove_file(file).ok();
}

/// A plain peer pair serves no admin space: the report says *not
/// available* and carries no rows — never an empty consumer set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_admin_space_is_not_asked_never_none() {
    let (listen, connect) = peer_pair().await;
    let _sub = listen
        .declare_subscriber(NARROW)
        .await
        .expect("a subscriber nobody's admin space can report");
    let fleet = zenkey_fleet::Fleet::new(&connect, "");
    let report = zenkey_fleet::consumers(&fleet, TARGET, Duration::from_millis(300))
        .await
        .expect("silence is a reading, not an error");
    assert_eq!(report.admin, AdminAnswer::NotAvailable);
    assert!(report.rows.is_empty());
    assert_eq!(report.asked.len(), 6, "{:?}", report.asked);
    assert_eq!(report.reply_elided, 0);
}

/// A target that is not a key expression is refused before anything is
/// asked (exit 2's business), and the refusal says so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_target_is_refused_not_answered() {
    let (_listen, connect) = peer_pair().await;
    let fleet = zenkey_fleet::Fleet::new(&connect, "");
    let err = zenkey_fleet::consumers(&fleet, "v1//bad", Duration::from_millis(100))
        .await
        .expect_err("not a key expression");
    assert!(err.is_unaskable(), "{err}");
}
