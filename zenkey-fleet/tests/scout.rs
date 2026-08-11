//! Scouting end-to-end (#116) — `#[ignore]`d, deliberately.
//!
//! A scout hears only what UDP multicast reaches, and CI runners routinely
//! filter or lack multicast — a failure here would say nothing about the
//! code. Run locally with `cargo test -p zenkey-fleet --test scout -- --ignored`
//! on a segment where multicast works. The verb's correctness otherwise
//! rests on the pure unit tests (matcher folding, dedup, empty verdict,
//! config bits) in `src/scout.rs`, `src/session.rs`, and zenctl.

use std::time::Duration;

use zenoh::config::WhatAmIMatcher;

/// A peer session with multicast on should be heard by a scout on the same
/// segment within a generous deadline.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "multicast is unreliable/absent on CI runners; run locally with --ignored"]
async fn a_scout_hears_a_multicasting_peer() {
    let _peer = zenkey_fleet::session::open(&[], &[], true).await.unwrap();
    let stream = zenkey_fleet::scout(WhatAmIMatcher::empty().router().peer().client(), &[], &[])
        .await
        .unwrap();
    let hello = tokio::time::timeout(Duration::from_secs(10), stream.recv())
        .await
        .expect("no Hello within 10s — is multicast working on this segment?")
        .expect("scout stopped before a Hello arrived");
    assert!(!hello.zid.is_empty());
    stream.stop();
}
