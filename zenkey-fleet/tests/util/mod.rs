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

/// A loopback endpoint on a port the OS has just told us is free.
///
/// The listener is released the instant its number is known, because zenoh
/// binds next. That window is what makes this "a free port" rather than "a
/// reserved one" — the OS does not hand the same ephemeral port out twice in
/// a row, which is all this needs.
pub fn endpoint() -> String {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = held.local_addr().expect("local addr").port();
    drop(held);
    format!("tcp/127.0.0.1:{port}")
}

/// Two in-process peers on a fresh port: one listening, one connected to it.
/// No scouting, no external router — the fixture nearly every suite opens.
pub async fn peer_pair() -> (zenoh::Session, zenoh::Session) {
    let endpoint = endpoint();
    let listen = zenkey_fleet::bus::session::open(&[], std::slice::from_ref(&endpoint), false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::bus::session::open(std::slice::from_ref(&endpoint), &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

/// The same pair, with the listener **stamping what it publishes** — a fleet
/// with `timestamping.enabled`, which the seed, stamper and `@adv` cache
/// fixtures need (an AdvancedPublisher's sequencing is timestamp-based).
pub async fn timestamping_pair() -> (zenoh::Session, zenoh::Session) {
    let endpoint = endpoint();
    let mut cfg = zenoh::Config::default();
    cfg.insert_json5("scouting/multicast/enabled", "false").ok();
    cfg.insert_json5("timestamping/enabled", "true").ok();
    cfg.insert_json5("listen/endpoints", &format!("[\"{endpoint}\"]"))
        .ok();
    let listen = zenoh::open(cfg).await.expect("timestamping session");
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
        "zenkey-fleet-admin-{}-{}.json5",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, r#"{ adminspace: { enabled: true } }"#).expect("write admin config");
    path
}
