//! The admin space's declaration records, asked only on opt-in (#890).
//!
//! zenoh 1.10.1 answers `@/<zid>/<whatami>/{subscriber,publisher,queryable,
//! querier,token}/**` while it holds its routing tables' read lock, and a
//! declaration from another client between two replies deadlocks the
//! router. So the window asks them only with `--admin-declared`. The bus
//! here is a router whose own admin space is off, and a spy session's
//! queryable on `@/**`: every admin GET the window sends reaches the spy,
//! and nothing answers it, so no test here can freeze anything.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Zengui, drive};
use crate::message::{Message, PaneMsg};
use crate::view::admin::AdminMsg;

/// A router on an ephemeral loopback port, its admin space off (zenoh's
/// default), and the endpoint it listens on.
async fn router() -> (zenoh::Session, String) {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", "\"router\""),
        ("listen/endpoints", "[\"tcp/127.0.0.1:0\"]"),
        ("scouting/multicast/enabled", "false"),
        ("scouting/gossip/enabled", "false"),
        ("adminspace/enabled", "false"),
    ] {
        c.insert_json5(k, v).unwrap();
    }
    let router = zenoh::open(c).await.expect("a router");
    let endpoint = router
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback listener");
    (router, endpoint)
}

async fn client(endpoint: &str) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", "\"client\"".to_owned()),
        ("scouting/multicast/enabled", "false".to_owned()),
        ("scouting/gossip/enabled", "false".to_owned()),
        ("connect/endpoints", format!("[\"{endpoint}\"]")),
    ] {
        c.insert_json5(k, &v).unwrap();
    }
    zenoh::open(c).await.expect("a client")
}

/// Every selector a GET under `@/` was asked with, as the spy heard it.
type Heard = Arc<Mutex<Vec<String>>>;

/// The spy: a queryable on `@/**` that records each query's key expression
/// and answers nothing, heard by the router before the window boots (a
/// probe GET reaches it).
async fn spy(endpoint: &str) -> (zenoh::Session, zenoh::query::Queryable<()>, Heard) {
    let session = client(endpoint).await;
    let heard: Heard = Arc::default();
    let queryable = {
        let heard = Arc::clone(&heard);
        session
            .declare_queryable("@/**")
            .callback(move |q| heard.lock().unwrap().push(q.key_expr().to_string()))
            .await
            .expect("a queryable on @/**")
    };
    let probe = client(endpoint).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !heard.lock().unwrap().iter().any(|k| k == "@/spy/probe") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the spy's queryable never reached the router"
        );
        if let Ok(replies) = probe
            .get("@/spy/probe")
            .timeout(Duration::from_millis(500))
            .await
        {
            while replies.recv_async().await.is_ok() {}
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = probe.close().await;
    heard.lock().unwrap().clear();
    (session, queryable, heard)
}

/// The window booted against `endpoint` as a launch would, with or without
/// `--admin-declared`, its boot tasks run to quiescence.
async fn booted_against(endpoint: &str, admin_declared: bool) -> Zengui {
    let mut settings = crate::app::test_settings();
    settings.connect = vec![endpoint.to_string()];
    settings.timeout_secs = 1;
    settings.admin_declared = admin_declared;
    let (mut app, boot) = Zengui::with_prefs(settings, crate::prefs::Prefs::default(), None);
    tokio::time::timeout(Duration::from_secs(30), drive(&mut app, boot))
        .await
        .expect("the boot settles");
    assert!(app.dep.raw.is_some(), "the boot opened its sessions");
    app
}

/// Press the admin pane's sweep and run it to quiescence.
async fn sweep(app: &mut Zengui) {
    let task = app.update(Message::Pane(PaneMsg::Admin(AdminMsg::Run)));
    tokio::time::timeout(Duration::from_secs(30), drive(app, task))
        .await
        .expect("the sweep settles");
}

/// By default the window asks the admin space nothing a zenoh 1.10.1 router
/// answers under its tables' read lock — not at boot, not with the admin
/// sweep — while the sweep's own reads (routers, storages, topology) still
/// go out, which is what shows the spy hears the window at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn by_default_the_window_never_asks_the_declaration_records() {
    let (_router, endpoint) = router().await;
    let (_spy, _queryable, heard) = spy(&endpoint).await;

    let mut app = booted_against(&endpoint, false).await;
    let at_boot = heard.lock().unwrap().clone();
    assert!(
        at_boot
            .iter()
            .all(|s| !zenkey_fleet::reaches_tables_lock(s)),
        "the boot asked {at_boot:?}"
    );
    assert!(
        app.dep.admin_declared.is_none(),
        "nothing asked, nothing held"
    );

    sweep(&mut app).await;
    let all = heard.lock().unwrap().clone();
    for read in [
        zenkey_fleet::bus::admin::ROUTERS,
        zenkey_fleet::bus::admin::STORAGES,
        zenkey_fleet::bus::admin::TOPOLOGY,
    ] {
        assert!(
            all.iter().any(|s| s == read),
            "the sweep asks {read}: {all:?}"
        );
    }
    assert!(
        all.iter().all(|s| !zenkey_fleet::reaches_tables_lock(s)),
        "the sweep asked {all:?}"
    );
}

/// With `--admin-declared` the boot asks the declared-entities sweep, every
/// one of its selectors, and the admin sweep asks it again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_the_flag_the_boot_asks_the_declared_sweep() {
    let (_router, endpoint) = router().await;
    let (_spy, _queryable, heard) = spy(&endpoint).await;
    let want = zenkey_fleet::declared_entity_selectors();
    let count = |heard: &Heard, s: &str| heard.lock().unwrap().iter().filter(|h| *h == s).count();

    let mut app = booted_against(&endpoint, true).await;
    for s in &want {
        assert_eq!(count(&heard, s), 1, "the boot asks {s} once");
    }
    sweep(&mut app).await;
    for s in &want {
        assert_eq!(count(&heard, s), 2, "the sweep asks {s} again");
    }
}
