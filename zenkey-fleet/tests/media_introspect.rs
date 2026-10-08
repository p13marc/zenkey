//! `[[media]]` reaches the introspect slice (#77; RFC 08 §2/§6, v1.16):
//! a producer declaring media streams is discoverable **off the bus** — no
//! `--registry`, no compiled-in knowledge — which is what unblocks a media
//! viewer that can enumerate streams before subscribing to one.
//!
//! Ports are ephemeral (`util::peer_pair`), so two test runs at once
//! cannot collide.

use std::time::Duration;

mod util;
use util::peer_pair;

const ORIGIN: &str = "h-aaaaaaaaaaaa";

/// The parallax fixture's shape, served the way a real producer serves it:
/// the registry TOML verbatim (RFC 08 §6 — introspect is `include_str!` of
/// the same source the key constants compile from).
const SLICE: &str = r#"
[registry]
version = "1.3"
app = "spray"
convention = 1

[producer]
name = "parallax"
description = "demo media producer"

[[media]]
path = "{stream}/preview/jpeg"
encoding = "image/jpeg"
attachment = "FrameMeta"
cardinality = 16
since = "1.0"

[[media]]
path = "{stream}/video/{codec}/{tier}"
encoding = "video/*"
attachment = "FrameMeta"
cardinality = 128
since = "1.3"
"#;

/// A media-declaring producer's streams are read from the bus
/// alone — and a fleet slice read the same way carries the declarations for
/// any consumer (the zengui detail pane).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_media_streams_are_discoverable_off_the_bus() {
    let (server, client) = peer_pair().await;

    let key = format!("v1/{ORIGIN}/@rpc/parallax/introspect");
    let queryable = server
        .declare_queryable(key.clone())
        .await
        .expect("declare introspect");
    let reply_key = key.clone();
    tokio::spawn(async move {
        while let Ok(query) = queryable.recv_async().await {
            let _ = query
                .reply(reply_key.clone(), SLICE.as_bytes().to_vec())
                .await;
        }
    });

    // Routability first: silence is not a slice (RFC 05 §3.1).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let answers = zenkey_fleet::fleet_get(
            &zenkey_fleet::Fleet::new(&client, ""),
            &key,
            &zenkey_fleet::GetOpts::new(Duration::from_millis(500)),
        )
        .await
        .expect("probe");
        if !answers.is_empty() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "introspect never became routable"
        );
    }

    // The fleet's own introspect sweep, as any consumer reads it (`node
    // info`, which joined it per origin, left with v1's `node` noun, #612).
    let served = zenkey_fleet::fleet_registry(
        &zenkey_fleet::Fleet::new(&client, ""),
        Duration::from_secs(5),
    )
    .await
    .expect("the introspect sweep");
    let (_, producer) = served
        .iter()
        .find(|(name, _)| name == "parallax")
        .expect("parallax introspected");
    assert_eq!(producer.media.len(), 2, "both declared streams surface");
    assert_eq!(producer.media[0].path, "{stream}/preview/jpeg");
    assert_eq!(producer.media[0].encoding.as_encoding_str(), "image/jpeg");
    assert_eq!(producer.media[1].encoding.as_encoding_str(), "video/*");

    // And the raw slice parse carries the full declarations (v1.16's
    // MediaDecl), optional fields included.
    let slice = zenkey::slice::parse_slice(SLICE).expect("slice parses");
    assert_eq!(slice.media.len(), 2);
    assert_eq!(slice.media[0].attachment.as_deref(), Some("FrameMeta"));
    assert_eq!(slice.media[0].cardinality, Some(16));
}
