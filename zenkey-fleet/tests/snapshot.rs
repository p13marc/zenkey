//! `take_snapshot` (RFC 13 §4.4; #219) against a real bus: the holder is
//! evidence, and each of its three kinds is produced by the fixture that
//! defines it.
//!
//! One listener session plays the fleet: a bare queryable on one origin
//! with **no** `alive` token (a storage remembering a value nobody says),
//! and a producer brought up on a second origin — queryable, then token,
//! answering its own key with its own session's stamp. The connector is the
//! observer. Ports are ephemeral (`util`), so two runs cannot collide.

use std::time::Duration;

use zenkey_fleet::report::{AnsweredBy, Asked, Holder, RegistrationWire, VerdictWire};
use zenkey_fleet::{BringUp, Fleet, SchemaStore, SnapshotSpec, declare_responder, take_snapshot};

mod util;
use util::timestamping_pair;

const STORED: &str = "v1/h-aaaaaaaaaaaa/state/demo/health";
const LIVE: &str = "v1/h-bbbbbbbbbbbb/state/demo/health";
const LIVE_ALIVE: &str = "v1/h-bbbbbbbbbbbb/state/demo/alive";

fn spec(roster: bool) -> SnapshotSpec {
    SnapshotSpec {
        selectors: vec!["v1/**".into()],
        timeout: Duration::from_secs(2),
        max_replies: 64,
        roster,
    }
}

/// The fleet: a storage-shaped responder on origin `a…` and a live producer
/// on origin `b…`, both on the stamping session. Returns the tasks driving
/// them, so a test drops them at its end.
async fn stage(fleet_session: &zenoh::Session) -> Vec<tokio::task::JoinHandle<()>> {
    let storage = declare_responder(
        fleet_session,
        STORED,
        br#"{"status":"remembered"}"#.to_vec(),
        Some("application/json"),
        false,
    )
    .await
    .expect("storage responder");
    let storage_task = tokio::spawn(async move {
        while let Some(q) = storage.next().await {
            storage.answer(q).await;
        }
    });

    let mut bring_up = BringUp::new(fleet_session);
    bring_up.serve(LIVE).await.expect("live queryable");
    let producer = bring_up.alive(LIVE_ALIVE).await.expect("alive token");
    let stamping = fleet_session.clone();
    let live_task = tokio::spawn(async move {
        // Move the *whole* producer into the task. Rust 2021's disjoint
        // capture would otherwise take only `producer.responders` and drop
        // `producer.token` — the alive token — the moment this function
        // returns, which undeclares it and turns every `live` holder below
        // into `storage_only`.
        let producer = producer;
        let responder = &producer.responders[0];
        while let Some(q) = responder.next().await {
            // The producer stamps its own reply with its own clock, so the
            // stamper *is* the replier — the fact `answered_by: stamper`
            // asserts. zenoh stamps pushes, never replies, so this is
            // explicit.
            let _ = q
                .reply(LIVE, br#"{"status":"ok"}"#.to_vec())
                .encoding("application/json")
                .timestamp(stamping.new_timestamp())
                .await;
        }
    });
    vec![storage_task, live_task]
}

/// Take snapshots until `ready` holds — routing propagation is async, and a
/// token or a queryable declared a moment ago is not yet visible to the
/// observer (the `tests/doctor.rs` retry, in the snapshot's shape).
async fn settle(
    fleet: &Fleet<'_>,
    store: &SchemaStore,
    spec: &SnapshotSpec,
    ready: impl Fn(&zenkey_fleet::Taken) -> bool,
) -> zenkey_fleet::Taken {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let taken = take_snapshot(fleet, None, store, spec)
                .await
                .expect("a snapshot");
            if ready(&taken) {
                break taken;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the fleet should become visible within 10s")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_holder_is_evidence_from_the_roster_and_the_replier() {
    let (fleet_session, observer) = timestamping_pair().await;
    let tasks = stage(&fleet_session).await;

    let fleet = Fleet::new(&observer, "");
    let store = SchemaStore::new("", Duration::from_millis(300));
    let taken = settle(&fleet, &store, &spec(true), |t| {
        t.snapshot.header.roster == Asked::Asked(1) && t.snapshot.rows.len() == 2
    })
    .await;
    let snapshot = taken.snapshot;
    assert!(taken.incomplete.is_empty());

    let h = &snapshot.header;
    assert_eq!(h.asked, 1);
    assert_eq!(h.answered, 2, "both queryables answered: {h:?}");
    assert!(
        h.collection_span_s > 0.0,
        "collected over a span, never at an instant"
    );
    assert_eq!(h.roster, Asked::Asked(1), "one origin held a token");
    assert_eq!(h.superseded, 0);
    assert_eq!(h.errors, 0);
    assert_eq!(h.elided, 0);

    let row = |key: &str| {
        snapshot
            .rows
            .iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("no row for {key}: {:?}", snapshot.rows))
    };
    let stored = row(STORED);
    assert_eq!(
        stored.holder,
        Holder::StorageOnly {
            origin: "h-aaaaaaaaaaaa".into()
        },
        "a value answered and no token was held"
    );
    assert!(stored.source_zid.is_some(), "the reply named its replier");
    assert_eq!(stored.registration, RegistrationWire::RegistryNotLoaded);
    assert_eq!(
        stored.verdict,
        VerdictWire::NotValidated {
            reason: "no_registry".into()
        }
    );

    let live = row(LIVE);
    assert_eq!(
        live.holder,
        Holder::Live {
            origin: "h-bbbbbbbbbbbb".into(),
            answered_by: AnsweredBy::Stamper,
        },
        "the token stood and the replier stamped the value: {live:?}"
    );
    assert!(live.timestamp.is_some() && live.stamper.is_some());
    assert!(!live.delete);

    for t in tasks {
        t.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_roster_every_holder_is_unattributed_and_says_why() {
    let (fleet_session, observer) = timestamping_pair().await;
    let tasks = stage(&fleet_session).await;

    let fleet = Fleet::new(&observer, "");
    let store = SchemaStore::new("", Duration::from_millis(300));
    let snapshot = settle(&fleet, &store, &spec(false), |t| t.snapshot.rows.len() == 2)
        .await
        .snapshot;

    assert_eq!(snapshot.header.roster, Asked::NotAsked);
    assert_eq!(snapshot.rows.len(), 2);
    for row in &snapshot.rows {
        assert_eq!(
            row.holder,
            Holder::Unattributed {
                reason: "roster not asked".into()
            },
            "{}",
            row.key
        );
    }
    // The file says so too: the header carries no `roster` key at all.
    let line = serde_json::to_value(&snapshot.header).unwrap();
    assert!(line.get("roster").is_none(), "{line}");

    for t in tasks {
        t.abort();
    }
}
