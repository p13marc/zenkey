//! The discipline against a real bus (#389; zk2 since #614, FL2): a restart
//! with the same state file does not re-page a token already announced
//! gone — its resolve after the restart still arrives, which proves the
//! memory — and a state file the daemon cannot honestly read (version 1,
//! another namespace's) is a refusal before anything is watched. Ports are
//! ephemeral; one router, the daemon and a foreign application as its
//! clients. (Inhibition and its test left with v1's catalog.)

mod util;

use std::time::Duration;

use util::engine::{self, Capture, Options};
use util::zk2;
use zenkey_fleet::CondState;
use zenwatch::sinks::NoticeKind;

const KEY: &str = "demo/held/p1/alive";

fn rules() -> Vec<zenwatch::config::RuleConfig> {
    vec![engine::rule(
        "held-gone",
        "liveliness-gone demo/held/*/alive",
        "error",
    )]
}

/// Run with a token that goes, a state file, stop; start again with the
/// same file while the token is still gone: nothing — the first look finds
/// it absent under a complete read and remembers it as announced. The
/// token coming back is one `resolved`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_with_the_same_state_file_does_not_re_page() {
    let (_router, ep) = zk2::router().await;
    let app = zk2::client(&ep).await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("zenwatch-state.json");
    let token = app.liveliness().declare_token(KEY).await.unwrap();
    zk2::tokens(&app, KEY, 1).await;

    let first = Capture::default();
    let run1 = engine::start(
        zk2::daemon_bus(&ep, "").await,
        rules(),
        first.clone(),
        Options {
            state_file: Some(state.clone()),
            ..Options::default()
        },
    )
    .await;
    token.undeclare().await.unwrap();
    let got = engine::until(&first, "the first firing", |s| !s.is_empty()).await;
    assert_eq!(got[0].notification.kind, NoticeKind::Liveliness);
    assert_eq!(got[0].notification.state, CondState::Firing);
    let summary = run1.stop().await;
    assert_eq!(summary.outgoing, 1);
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert_eq!(saved["namespace"], "");
    assert_eq!(saved["entries"].as_array().unwrap().len(), 1, "{saved}");
    assert_eq!(saved["entries"][0]["id"], format!("held-gone:{KEY}"));
    assert_eq!(saved["entries"][0]["state"], "firing");

    // The second life: still gone, and already announced before the
    // restart — remembered, not re-paged.
    let second = Capture::default();
    let run2 = engine::start(
        zk2::daemon_bus(&ep, "").await,
        rules(),
        second.clone(),
        Options {
            state_file: Some(state.clone()),
            ..Options::default()
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        second.lock().unwrap().is_empty(),
        "a token announced gone before the restart is nothing: {:?}",
        second.lock().unwrap()
    );
    // …and its return is one resolved, which proves the memory.
    let token = app.liveliness().declare_token(KEY).await.unwrap();
    let got = engine::until(&second, "the resolve after the restart", |s| !s.is_empty()).await;
    assert_eq!(got[0].notification.kind, NoticeKind::Resolved);
    assert_eq!(got[0].notification.state, CondState::Ok);
    assert_eq!(got[0].notification.prior, Some(CondState::Firing));
    let summary = run2.stop().await;
    assert_eq!(
        summary.discipline.baseline, 1,
        "the remembered token, assumed"
    );
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(
        saved["entries"].as_array().unwrap().len(),
        0,
        "resolved: nothing to remember"
    );
    token.undeclare().await.unwrap();
}

/// A state file the daemon cannot honestly read is refused before anything
/// is watched (exit 2): one v1 zenwatch wrote, never migrated, and one
/// written in another namespace, whose addresses are another deployment's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_v1_or_foreign_namespace_state_file_is_refused() {
    let (_router, ep) = zk2::router().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"version":1,"saved_at":"x","entries":[],"evicted_total":0}"#,
    )
    .unwrap();
    let e = engine::run_once_refused(
        zk2::daemon_bus(&ep, "prod").await,
        rules(),
        Options {
            state_file: Some(state.clone()),
            ..Options::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(zenwatch::exit::code_for(&e), zenwatch::exit::REFUSED);
    assert!(e.to_string().contains("version 1"), "{e}");

    std::fs::write(
        &state,
        r#"{"version":2,"saved_at":"x","namespace":"staging","entries":[],"evicted_total":0}"#,
    )
    .unwrap();
    let e = engine::run_once_refused(
        zk2::daemon_bus(&ep, "prod").await,
        rules(),
        Options {
            state_file: Some(state),
            ..Options::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(zenwatch::exit::code_for(&e), zenwatch::exit::REFUSED);
    assert!(e.to_string().contains("\"staging\""), "{e}");
}
