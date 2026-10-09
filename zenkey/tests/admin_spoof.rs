//! Core §4.2 (0.12, F-80): who can answer the admin space. Any session can
//! declare a queryable under `@/<zid>/router`, the zid of a real router
//! included, so neither the key nor the document tells a router's answer
//! from a session's. A reply's replier id (unstable API in zenoh 1.10.1,
//! Appendix B) names the session that replied.

mod common;

use common::{client, router_with};
use zenoh::Wait;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};

async fn read(tool: &zenoh::Session) -> Vec<(String, String, Option<String>)> {
    let rx = tool
        .get("@/*/router")
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(common::T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        let replier = r.replier_id().map(|g| g.zid().to_string());
        if let Ok(s) = r.result() {
            out.push((
                s.key_expr().to_string(),
                String::from_utf8_lossy(&s.payload().to_bytes()).into_owned(),
                replier,
            ));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_can_answer_as_a_router_and_its_replier_id_says_so() {
    for admin in [false, true] {
        let extra: Vec<(&str, &str)> = if admin {
            vec![(
                "adminspace",
                r#"{ enabled: true, permissions: { read: true, write: false } }"#,
            )]
        } else {
            vec![]
        };
        let (r1, ep) = router_with(None, &extra).await;
        let (spoofer, tool) = (client(&ep).await, client(&ep).await);
        let rz = r1.zid().to_string();
        let key = format!("@/{rz}/router");
        let k = key.clone();
        let _q = spoofer
            .declare_queryable(&key)
            .callback(move |q| {
                let _ = q.reply(k.clone(), r#"{"plugins": null}"#).wait();
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let got = read(&tool).await;
        eprintln!(
            "ADMIN {admin}: router {rz}, spoofer {}\n  {got:#?}",
            spoofer.zid()
        );
        let spoofed: Vec<_> = got
            .iter()
            .filter(|(_, v, _)| v.contains("\"plugins\": null"))
            .collect();
        assert_eq!(spoofed.len(), 1, "the spoofer's answer arrives: {got:?}");
        assert_eq!(spoofed[0].0, key, "on the router's own key");
        assert_eq!(
            spoofed[0].2.as_deref(),
            Some(spoofer.zid().to_string().as_str()),
            "its replier id names the spoofer, not the router"
        );
        if admin {
            assert!(
                got.iter()
                    .any(|(_, _, by)| by.as_deref() == Some(rz.as_str())),
                "the router answers too, under its own id: {got:?}"
            );
        } else {
            assert_eq!(got.len(), 1, "admin off: the spoof is the only answer");
        }
    }
}

/// Core §4.2 (0.13, F-81): a far router is verified through a verified
/// router's own document, which lists its sessions with their `whatami`.
/// R2 links to R1; a client tool on R1 reads both routers' answers, each
/// under its own replier id; R1's document lists R2 as a `router` session
/// and a client spoofing `@/<its own zid>/router` as a `client` one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_far_router_is_listed_as_a_router_and_a_client_as_a_client() {
    let on = [(
        "adminspace",
        r#"{ enabled: true, permissions: { read: true, write: false } }"#,
    )];
    let (r1, ep) = router_with(None, &on).await;
    let (r2, _ep2) = router_with(Some(&ep), &on).await;
    let (spoofer, tool) = (client(&ep).await, client(&ep).await);
    let sz = spoofer.zid().to_string();
    let key = format!("@/{sz}/router");
    let k = key.clone();
    let _q = spoofer
        .declare_queryable(&key)
        .callback(move |q| {
            let _ = q.reply(k.clone(), r#"{"plugins": null}"#).wait();
        })
        .await
        .unwrap();
    let (z1, z2) = (r1.zid().to_string(), r2.zid().to_string());
    let mut got = Vec::new();
    for _ in 0..50 {
        got = read(&tool).await;
        if got.len() >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let by_key = |zid: &str| {
        got.iter()
            .find(|(k, _, _)| k == &format!("@/{zid}/router"))
            .unwrap_or_else(|| panic!("an answer for {zid}: {got:#?}"))
            .clone()
    };
    // Each answer under its own replier id: self-consistent, all three.
    for z in [&z1, &z2, &sz] {
        assert_eq!(by_key(z).2.as_deref(), Some(z.as_str()), "{z}");
    }
    // R1's document lists its sessions by `whatami`.
    let doc: serde_json::Value = serde_json::from_str(&by_key(&z1).1).unwrap();
    let whatami = |zid: &str| {
        doc["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["peer"] == zid)
            .map(|s| s["whatami"].as_str().unwrap().to_owned())
    };
    eprintln!("R1 sessions: {}", doc["sessions"]);
    assert_eq!(whatami(&z2).as_deref(), Some("router"), "R2 is a router");
    assert_eq!(
        whatami(&sz).as_deref(),
        Some("client"),
        "the spoofer is a client"
    );
    assert_eq!(whatami(&tool.zid().to_string()).as_deref(), Some("client"));
}
