//! `node_info` asks about one node, and only that node (issue #96).
//!
//! Before this, one call re-ran the whole fleet liveliness sweep *and* a
//! fleet-wide `introspect` fan-in, then filtered the result to the origin the
//! caller named — so the zengui node dashboard (#61) paid for a full fleet
//! introspect on every card click. The fix is structural (origin-scoped
//! selectors through the typed builders), so the test is structural too: a
//! queryable that **records the key expression it was asked on**, and an
//! assertion that no `*`-origin selector appears.
//!
//! Self-contained like `querier.rs`: two in-process peers, explicit endpoints,
//! no scouting, no external router. Ports 7501-7502 (disjoint from every other
//! test binary).

use std::sync::{Arc, Mutex};
use std::time::Duration;

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

const ORIGIN: &str = "h-aaaaaaaaaaaa";
const OTHER: &str = "h-bbbbbbbbbbbb";

fn slice_toml(producer: &str) -> String {
    format!(
        r#"
[registry]
version = "1.0"
app = "t"
convention = 1

[producer]
name = "{producer}"
description = "fixture"

[[subject]]
path = "health"
class = "state"
type = "Health"
since = "1.0"
description = "d"
"#
    )
}

/// Every keyexpr the fixture was asked on, in arrival order.
type Asked = Arc<Mutex<Vec<String>>>;

/// A producer on each of two origins, both recording what they were asked.
/// The handles are returned because a dropped queryable undeclares itself.
async fn declare_introspect(
    session: &zenoh::Session,
    asked: &Asked,
) -> Vec<zenoh::query::Queryable<()>> {
    let mut handles = Vec::new();
    for origin in [ORIGIN, OTHER] {
        let key = format!("v1/{origin}/@rpc/sysinfo/introspect");
        let asked = Arc::clone(asked);
        let reply_key = key.clone();
        handles.push(
            session
                .declare_queryable(&key)
                .callback(move |query| {
                    asked
                        .lock()
                        .expect("asked lock")
                        .push(query.key_expr().to_string());
                    let q = query.clone();
                    let reply_key = reply_key.clone();
                    tokio::spawn(async move {
                        q.reply(reply_key, slice_toml("sysinfo")).await.unwrap();
                    });
                })
                .await
                .expect("queryable"),
        );
    }
    handles
}

/// The acceptance: one origin asked, origin-scoped GETs only — and the other
/// origin's producer is never even reached.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn node_info_asks_only_the_named_origin() {
    let (a, b) = peer_pair(7501).await;
    let asked: Asked = Arc::default();
    let _queryables = declare_introspect(&a, &asked).await;

    // Routing propagation is async; retry bounded until the fixture answers.
    let info = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = zenkey_fleet::node_info(&b, "", ORIGIN, Duration::from_secs(2), false)
                .await
                .expect("node_info");
            if !info.producers.is_empty() {
                break info;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the fixture should answer within 5s");

    assert_eq!(info.origin, ORIGIN);
    assert_eq!(
        info.producers
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        vec!["sysinfo"],
        "only the named origin's producer is reported"
    );
    assert_eq!(info.producers[0].subjects, 1, "the slice was read");

    let asked = asked.lock().expect("asked lock").clone();
    assert!(!asked.is_empty(), "the fixture was queried at all");
    for key in &asked {
        assert!(
            key.starts_with(&format!("v1/{ORIGIN}/")),
            "node_info must not sweep past the origin it was asked about — saw {key}"
        );
    }
    assert!(
        !asked.iter().any(|k| k.contains(&format!("v1/{OTHER}/"))),
        "the other origin's producer must never be asked: {asked:?}"
    );
}

/// A hostname in the origin position is the RFC 06 §6 bridge bug, and it fails
/// loudly here rather than being string-glued into a selector.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hostname_is_refused_before_any_get() {
    let (_a, b) = peer_pair(7502).await;
    let err = zenkey_fleet::node_info(&b, "", "toolbx", Duration::from_millis(200), false)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("RFC 06 §6"), "{err}");
}
