//! Why a session failed to open is a distinction with consequences (#196).
//!
//! A caller holding `--registry` dirs can answer without a bus, but only for
//! one of the two reasons: a transport that will not come up leaves what is on
//! disk perfectly readable, while a config file the user *named* and that does
//! not parse is their own error — answering anyway would hide it.

use std::path::Path;

use zenkey_fleet::{OpenFailure, open_reporting};

#[tokio::test(flavor = "multi_thread")]
async fn a_named_config_file_that_cannot_be_read_is_the_callers_error() {
    let missing = Path::new("/nonexistent-zenkey-session-test.json5");
    match open_reporting(Some(missing), &[], &[], None).await {
        Err(OpenFailure::Config(e)) => {
            let text = format!("{e:#}");
            assert!(
                text.contains("nonexistent-zenkey-session-test"),
                "the error names the file the user asked for: {text}"
            );
        }
        Err(OpenFailure::Transport(e)) => {
            panic!("a missing config file is not a transport failure: {e:#}")
        }
        Ok(_) => panic!("a missing config file must not open a session"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transport_that_will_not_come_up_is_reported_as_such() {
    // Hold a port for the duration, so the listener below cannot have it.
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a scratch port");
    let port = held.local_addr().expect("addr").port();

    let endpoint = format!("tcp/127.0.0.1:{port}");
    match open_reporting(None, &[], std::slice::from_ref(&endpoint), Some(false)).await {
        Err(OpenFailure::Transport(_)) => {}
        Err(OpenFailure::Config(e)) => {
            panic!("a taken port is not a config error: {e:#}")
        }
        Ok(_) => panic!("zenoh opened a session on a port this test holds ({endpoint})"),
    }
    drop(held);
}

#[path = "util/mod.rs"]
mod util;

/// #501: the explorer session is a **client** of whatever it connects to —
/// no listener of its own, and the node it dialled sees a client, not a peer
/// it could route through — and every act an explorer performs still works
/// over that one link: subscribe, get, liveliness in both directions.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_explorer_holds_no_listener_and_still_sees_everything() {
    use zenoh::config::WhatAmI;

    let serving = zenkey_fleet::open(&[], &[util::ANY_PORT.to_string()], false)
        .await
        .expect("a listening peer");
    let endpoint = util::bound(&serving).await;
    let asking = zenkey_fleet::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("the explorer session");

    assert!(
        asking.info().locators().await.is_empty(),
        "an explorer listens nowhere — a peer would hold tcp/[::]:<port>"
    );
    let kinds: Vec<WhatAmI> = serving
        .info()
        .transports()
        .await
        .map(|t| t.whatami())
        .collect();
    assert_eq!(
        kinds,
        [WhatAmI::Client],
        "the node it dialled sees a client"
    );

    // Liveliness, both ways: the explorer sees the fleet's token, and a
    // token the explorer declares (`zenctl serve`, zengui's own) reaches it.
    let _alive = serving
        .liveliness()
        .declare_token("demo/alive")
        .await
        .unwrap();
    let _mine = asking
        .liveliness()
        .declare_token("explorer/alive")
        .await
        .unwrap();
    let seen = tokio::time::timeout(util::SETTLE, async {
        loop {
            let replies = asking.liveliness().get("demo/**").await.unwrap();
            if replies.recv_async().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(seen.is_ok(), "the fleet's token reaches the client");
    let seen = tokio::time::timeout(util::SETTLE, async {
        loop {
            let replies = serving.liveliness().get("explorer/**").await.unwrap();
            if replies.recv_async().await.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(seen.is_ok(), "the client's token reaches the fleet");

    // Get: a queryable on the fleet answers the client.
    let _q = serving
        .declare_queryable("demo/state")
        .callback(|q| {
            tokio::spawn(async move {
                q.reply("demo/state", "ok").await.ok();
            });
        })
        .await
        .unwrap();
    let answered = tokio::time::timeout(util::SETTLE, async {
        loop {
            let replies = asking.get("demo/state").await.unwrap();
            if let Ok(reply) = replies.recv_async().await
                && let Ok(sample) = reply.into_result()
            {
                return sample.payload().try_to_string().unwrap().into_owned();
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the queryable answers the client");
    assert_eq!(answered, "ok");

    // Subscribe: the client hears a put once its interest has propagated.
    let sub = asking.declare_subscriber("demo/tick").await.unwrap();
    let heard = tokio::time::timeout(util::SETTLE, async {
        loop {
            serving.put("demo/tick", "1").await.unwrap();
            if let Ok(Ok(_)) =
                tokio::time::timeout(std::time::Duration::from_millis(100), sub.recv_async()).await
            {
                break;
            }
        }
    })
    .await;
    assert!(heard.is_ok(), "the client's subscriber hears the fleet");
}

/// #503: an endpoint nothing answers fails `open` — a **transport** failure,
/// promptly — where a peer used to open onto nothing and read as an empty
/// bus. Port 1 on loopback: privileged, so nothing a test run holds.
#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_router_fails_open_as_transport() {
    let started = std::time::Instant::now();
    match open_reporting(None, &["tcp/127.0.0.1:1".to_string()], &[], None).await {
        Err(OpenFailure::Transport(e)) => {
            let text = zenkey_fleet::one_line(&e);
            assert!(
                text.contains("tcp/127.0.0.1:1"),
                "names the endpoint: {text}"
            );
        }
        Err(OpenFailure::Config(e)) => panic!("a dead router is not a config error: {e:#}"),
        Ok(_) => panic!("a client opened against an endpoint nothing answers"),
    }
    assert!(
        started.elapsed() < zenkey_fleet::OPEN_TIMEOUT,
        "it fails at once, not at the deadline"
    );
}

/// #503: a malformed endpoint is the caller's error — `Config`, so a frontend
/// refuses it (exit 2) instead of degrading past it as an unreachable bus.
#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_endpoint_is_a_config_failure() {
    match open_reporting(None, &["127.0.0.1:7461".to_string()], &[], None).await {
        Err(OpenFailure::Config(e)) => {
            assert!(e.is_unaskable(), "{e}");
            assert!(e.to_string().contains("127.0.0.1:7461"), "{e}");
        }
        Err(OpenFailure::Transport(e)) => panic!("a typo is not the transport: {e:#}"),
        Ok(_) => panic!("a session opened on an endpoint that does not parse"),
    }
}
