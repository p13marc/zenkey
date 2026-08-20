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
