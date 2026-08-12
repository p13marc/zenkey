//! The origin→session join (#131): a liveliness token declared on the bus
//! attaches its origin to the session the admin space names — from
//! evidence only, never a guess.
//!
//! Same adminspace-config fixture as topology.rs (#122's passthrough).
//! Ports 7528-7529 (disjoint from every other test binary).

use std::time::Duration;

fn admin_config() -> std::path::PathBuf {
    let path = std::env::temp_dir().join("zenkey-fleet-origin-attach-admin.json5");
    std::fs::write(&path, r#"{ adminspace: { enabled: true } }"#).unwrap();
    path
}

const TOKEN: &str = "v1/h-cccccccccccc/state/demo/alive";

/// A peer serving its admin space and holding a liveliness token: the join
/// yields exactly that origin, reported by that peer — and a token-shaped
/// key that is not an alive leaf never becomes an attachment.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declared_token_attaches_its_origin() {
    let file = admin_config();
    let serving = zenkey_fleet::open_with_config(
        Some(&file),
        &[],
        &["tcp/127.0.0.1:7528".to_string()],
        Some(false),
    )
    .await
    .expect("serving session");
    let asking = zenkey_fleet::session::open(&["tcp/127.0.0.1:7528".to_string()], &[], false)
        .await
        .expect("asking session");

    let _token = serving
        .liveliness()
        .declare_token(TOKEN)
        .await
        .expect("declare token");
    // A token that parses but is not an alive leaf: never an attachment.
    let _other = serving
        .liveliness()
        .declare_token("v1/h-cccccccccccc/state/demo/lease")
        .await
        .expect("declare non-alive token");

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let attachments = loop {
        let a = zenkey_fleet::origin_attachments(&asking, "", Duration::from_millis(500))
            .await
            .expect("a quiet admin space is a reading, not an error");
        if !a.is_empty() {
            break a;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the admin space never served the token"
        );
    };

    assert_eq!(
        attachments.len(),
        1,
        "only the alive leaf attaches: {attachments:?}"
    );
    let a = &attachments[0];
    assert_eq!(a.origin, "h-cccccccccccc");
    assert_eq!(a.token_key, TOKEN);
    let serving_zid = serving.zid().to_string();
    assert_eq!(
        a.reporter_zid, serving_zid,
        "the peer's own admin space is the reporter"
    );
    // The sources may or may not name the declaring session in a
    // self-reporting peer — but if they name one, it must be the truth.
    if let Some(z) = &a.session_zid {
        assert_eq!(z, &serving_zid, "an attachment is evidence, never a guess");
    }

    std::fs::remove_file(file).ok();
}
