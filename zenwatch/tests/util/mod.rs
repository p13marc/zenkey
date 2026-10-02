//! Shared fixtures for the bus tests: **ephemeral ports**, and the two or
//! three session shapes every suite opens.
//!
//! Every suite used to hard-code a port and a header comment claiming it was
//! "disjoint from every other test binary" — a range carved up by hand from
//! 7461 to 7551, forty-odd of them. It had already stopped being true
//! (`expect.rs` and `doctor_listen.rs` both claimed 7535-7536), and it could
//! never be true of the case that actually bites: **two `cargo test` runs at
//! once**, on one machine, in two checkouts or two shells. Then the ports
//! collide with themselves and suites fail for a reason that has nothing to
//! do with the code under test.
//!
//! `session_open.rs` already had the technique: ask the OS for a free port
//! (`TcpListener::bind("127.0.0.1:0")`), take its number, and let it go.
//! Nothing here names a port, so nothing here can collide with a port.

// Each suite includes the whole module and uses the part it needs.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How long a bus test waits before calling a hang a hang (#371).
///
/// **This is a net, not an assertion.** Every use of it is wrapped around a
/// `recv()` that returns the instant its event arrives, so a generous value
/// costs a passing run nothing and costs a hanging run only the wait. What a
/// tight value costs is a *false* failure: these suites run in one
/// `cargo test --workspace` job alongside every other test binary, and under
/// that load a settle that normally takes milliseconds can take seconds.
///
/// Two of them at five seconds were the whole of #371 — read as "the seed
/// task outlived its monitor" and "presence did not meet", neither of which
/// was true. A timeout that is *the property under test* still has to be
/// tight; none of these are.
pub const SETTLE: Duration = Duration::from_secs(20);

/// The endpoint a test listener binds: loopback, the port chosen by the OS
/// at bind time. Read back what it got with [`bound`].
pub const ANY_PORT: &str = "tcp/127.0.0.1:0";

/// Where a listener opened on [`ANY_PORT`] actually listens.
///
/// The listener binds port 0 itself and this reads back what zenoh bound, so
/// no port is ever learned from one socket and then handed to another (#527).
/// The old helper did exactly that — bind, read the number, drop, let zenoh
/// bind next — and in that window any socket on the box could be given the
/// port, a concurrent run's *outgoing* connection included: one run in
/// eighteen of six concurrent live suites failed with EADDRINUSE.
pub async fn bound(session: &zenoh::Session) -> String {
    session
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback tcp listener")
}

/// Two in-process peers on a fresh port: one listening, one connected to it.
/// No scouting, no external router — the fixture nearly every suite opens.
pub async fn peer_pair() -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::bus::session::open(&[], &[ANY_PORT.to_string()], false)
        .await
        .expect("listener session");
    let endpoint = bound(&listen).await;
    let connect = zenkey_fleet::bus::session::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

/// The same pair, with the listener **stamping what it publishes** — a fleet
/// with `timestamping.enabled`, which the seed, stamper and `@adv` cache
/// fixtures need (an AdvancedPublisher's sequencing is timestamp-based).
pub async fn timestamping_pair() -> (zenoh::Session, zenoh::Session) {
    let mut cfg = zenoh::Config::default();
    cfg.insert_json5("scouting/multicast/enabled", "false").ok();
    cfg.insert_json5("timestamping/enabled", "true").ok();
    cfg.insert_json5("listen/endpoints", &format!("[\"{ANY_PORT}\"]"))
        .ok();
    let listen = zenoh::open(cfg).await.expect("timestamping session");
    let endpoint = bound(&listen).await;
    let connect = zenkey_fleet::bus::session::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

/// A config file enabling the admin space, for the `open_with_config`
/// passthrough the topology fixtures dogfood (#122).
///
/// Uniquely named for the same reason the ports are ephemeral: two runs at
/// once must not write — or delete — each other's fixture.
pub fn admin_config() -> std::path::PathBuf {
    static NTH: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "zenwatch-admin-{}-{}.json5",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, r#"{ adminspace: { enabled: true } }"#).expect("write admin config");
    path
}
