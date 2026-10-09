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
