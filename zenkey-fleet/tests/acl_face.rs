//! A face principal, judged by zenoh itself (RFC 09 §4, v1.49; #529).
//!
//! The planner's unit tests judge a plan by keyexpr inclusion over the plan
//! — the planner's own reading of zenoh. This suite does not trust that
//! reading. It renders a face block with a `user` principal, pastes it into
//! a real router's config beside a `usrpwd` dictionary, and asks the
//! router: an operator on the face's transport, authenticated as the
//! granted user, calls the granted write and gets the answer; the same
//! operator calling a write it was not granted, another user, and a peer
//! with no user at all get nothing.
//!
//! The design rests on two facts read from
//! `zenoh-1.10.0/src/net/routing/interceptor/` — a transport matches every
//! subject whose properties match, and across those subjects **any allow
//! wins** — and this is the test that fails if either stops being true.
//! The face's transport is `unixsock-stream`, as on the reference adopter's
//! modem lane; the queryable sits on a loopback tcp link the face does not
//! select, as the adopter's driver does on its host bus.

mod util;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use zenkey_fleet::report::{FaceSpec, LinkInterval, PrincipalSpec, Role};

/// A radio driver's slice: every procedure `host`, so the face denies the
/// `@rpc` plane whole, and two writes — one to grant, one not to.
const MODEM: &str = r#"
[registry]
version = "1.0"
app = "acme"
convention = 1

[producer]
name = "modem"

[[subject]]
path = "{device}/tx_sdus_total"
class = "telemetry"
type = "C"
cardinality = 4
exposure = "host"

[[procedure]]
path = "introspect"
kind = "read"
reply = "RegistrySlice"
exposure = "host"

[[procedure]]
path = "config/{device}/power/set"
kind = "write"
reply = "ConfigView"
cardinality = 4
exposure = "host"

[[procedure]]
path = "config/{device}/persist"
kind = "write"
reply = "Ack"
cardinality = 4
exposure = "host"
"#;

const GRANTED: &str = "v1/h-0123456789ab/@rpc/modem/config/rf0/power/set";
const NOT_GRANTED: &str = "v1/h-0123456789ab/@rpc/modem/config/rf0/persist";
const READ: &str = "v1/h-0123456789ab/@rpc/modem/introspect";
/// A `host` counter, served as a query so the face's class deny is judged
/// by the same instrument as the plane's.
const TELEMETRY: &str = "v1/h-0123456789ab/telemetry/modem/rf0/tx_sdus_total";

/// A wait for a reply that is expected **not** to come. A net in the other
/// direction from [`util::SETTLE`]: too short and a slow router reads as a
/// deny, so the granted calls are asked first and with the long net — a
/// deny is only believed once the same session has been answered.
const SILENCE: Duration = Duration::from_millis(1500);

/// A wait for replies that are expected to come, asked after the session
/// has been answered once — the queryables are known by then.
const ANSWER: Duration = Duration::from_secs(3);

fn sorted(keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    out.sort();
    out
}

fn scratch(name: &str) -> std::path::PathBuf {
    static NTH: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "zenkey-acl-face-{}-{}-{name}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ))
}

/// The face block for one `user = "ops"` console granted the power write.
fn face_block() -> String {
    let slices = zenkey_fleet::SliceSet::from_slices(vec![
        zenkey::parse_slice(MODEM).expect("fixture slice"),
    ]);
    let plan = zenkey_fleet::plan_face(
        &slices,
        "",
        &FaceSpec {
            id: "constrained-link".into(),
            link_protocols: vec!["unixsock-stream".into()],
            interfaces: Vec::new(),
            interval: LinkInterval::None,
        },
        &[PrincipalSpec {
            user: Some("ops".into()),
            role: Role::Console,
            writes: vec!["modem/config/*/power/set".into()],
            ..Default::default()
        }],
    );
    assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
    zenkey_fleet::acl_plan_json5(&plan)
}

/// A router holding the face block, listening on a unix socket (the face)
/// and on loopback tcp (the host bus). With `dictionary`, it authenticates
/// every link by usrpwd.
async fn router(sock: &std::path::Path, dictionary: Option<&std::path::Path>) -> zenoh::Session {
    node("router", sock, dictionary).await
}

/// The face's node in `mode` — `peer` is the reference adopter's zenohd.
async fn node(
    mode: &str,
    sock: &std::path::Path,
    dictionary: Option<&std::path::Path>,
) -> zenoh::Session {
    let auth = match dictionary {
        Some(d) => format!(
            "transport: {{ auth: {{ usrpwd: {{ user: \"router\", password: \"router-pw\", \
             dictionary_file: {:?} }} }} }},",
            d.display().to_string()
        ),
        None => String::new(),
    };
    let text = format!(
        "{{\n  mode: {mode:?},\n  scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }},\n  \
         listen: {{ endpoints: [\"unixsock-stream/{}\", \"{}\"] }},\n  {auth}\n{}}}\n",
        sock.display(),
        util::ANY_PORT,
        face_block()
    );
    let config = zenoh::Config::from_json5(&text)
        .unwrap_or_else(|e| panic!("the router config does not parse: {e}\n{text}"));
    zenoh::open(config).await.expect("router")
}

/// A client on `endpoint`, presenting `user` when given.
async fn client(endpoint: &str, user: Option<&str>) -> zenoh::Result<zenoh::Session> {
    session("client", endpoint, user).await
}

/// A session of `mode` on `endpoint`, presenting `user` when given.
async fn session(mode: &str, endpoint: &str, user: Option<&str>) -> zenoh::Result<zenoh::Session> {
    let mut cfg = zenoh::Config::default();
    cfg.insert_json5("mode", &format!("{mode:?}")).unwrap();
    cfg.insert_json5("scouting/gossip/enabled", "false")
        .unwrap();
    cfg.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    cfg.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    if let Some(u) = user {
        cfg.insert_json5(
            "transport/auth/usrpwd",
            &format!("{{ user: \"{u}\", password: \"{u}-pw\" }}"),
        )
        .unwrap();
    }
    zenoh::open(cfg).await
}

/// The driver's side: one queryable per key, each answering on its own key
/// whatever the query spelled — so a wildcard's replies are concrete, as a
/// served procedure's are.
async fn serve(session: &zenoh::Session) -> Vec<zenoh::query::Queryable<()>> {
    let mut out = Vec::new();
    for key in [GRANTED, NOT_GRANTED, READ, TELEMETRY] {
        out.push(
            session
                .declare_queryable(key)
                .callback(move |q| {
                    tokio::spawn(async move {
                        let _ = q.reply(key, "ok").await;
                    });
                })
                .await
                .expect("queryable"),
        );
    }
    out
}

/// The keys that answered a GET on `selector` within `wait`.
async fn answered(session: &zenoh::Session, selector: &str, wait: Duration) -> Vec<String> {
    let replies = session
        .get(selector)
        .timeout(wait)
        .await
        .expect("get is sent");
    let mut keys = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        if let Ok(sample) = reply.result() {
            keys.push(sample.key_expr().to_string());
        }
    }
    keys.sort();
    keys
}

/// The granted call, retried until the queryable's declaration has reached
/// the router — the one wait whose end is an answer rather than a silence.
async fn granted_answers(session: &zenoh::Session) -> bool {
    let deadline = tokio::time::Instant::now() + util::SETTLE;
    while tokio::time::Instant::now() < deadline {
        if answered(session, GRANTED, Duration::from_secs(2)).await == [GRANTED] {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_granted_user_calls_the_granted_write_and_nothing_else_crosses() {
    let sock = scratch("face.sock");
    let dict = scratch("usrpwd.txt");
    std::fs::write(
        &dict,
        "router:router-pw\ndriver:driver-pw\nops:ops-pw\nintruder:intruder-pw\n",
    )
    .unwrap();
    let r = router(&sock, Some(&dict)).await;
    let host_bus = util::bound(&r).await;
    let face = format!("unixsock-stream/{}", sock.display());

    let driver = client(&host_bus, Some("driver")).await.expect("driver");
    let _queryables = serve(&driver).await;
    assert!(
        granted_answers(&driver).await,
        "the host bus is not the face"
    );
    assert_eq!(answered(&driver, TELEMETRY, ANSWER).await, [TELEMETRY]);

    let ops = client(&face, Some("ops")).await.expect("ops on the face");
    assert!(
        granted_answers(&ops).await,
        "ops, authenticated on the face, calls the granted write — its subject's allow wins \
         over the face subject's deny-rpc"
    );
    assert!(
        answered(&ops, NOT_GRANTED, SILENCE).await.is_empty(),
        "a write ops was not granted stays denied: its subject repeats the carve"
    );
    assert!(
        answered(&ops, TELEMETRY, SILENCE).await.is_empty(),
        "a `host` counter stays home for ops too: its subject repeats every face deny, \
         because an allow beside them would win"
    );
    assert_eq!(
        answered(&ops, READ, ANSWER).await,
        [READ],
        "a console on the face reads the @rpc plane — the console's write shape, not a \
         blanket deny"
    );
    // RFC 09 §3 fact 6, pinned: a query broader than the carve is included
    // by no deny of ops's subject and crosses; the refusal of a broadcast
    // write is the server's (RFC 05 §2.1, #472), which this test double
    // does not implement.
    assert_eq!(
        answered(&ops, "v1/*/@rpc/**", ANSWER).await,
        sorted(&[GRANTED, NOT_GRANTED, READ]),
        "a wildcard query is not covered by narrower denies (fact 6)"
    );

    let intruder = client(&face, Some("intruder"))
        .await
        .expect("another user on the face");
    for key in [GRANTED, NOT_GRANTED, READ, TELEMETRY, "v1/*/@rpc/**"] {
        assert!(
            answered(&intruder, key, SILENCE).await.is_empty(),
            "another user matches only the face's subject: {key} stays home"
        );
    }

    // A dictionary refuses a link without credentials outright: on a usrpwd
    // face there is no anonymous session for the ACL to judge.
    assert!(
        client(&face, None).await.is_err(),
        "a router holding a dictionary refuses an unauthenticated link"
    );

    drop((ops, intruder, driver, r));
    let _ = std::fs::remove_file(&dict);
    let _ = std::fs::remove_file(&sock);
}

/// The same block on a router with no dictionary: a peer has no user, so
/// only the face's subject matches its link, and the whole plane stays
/// home — the user's subject widens nothing for anyone else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_a_user_only_the_face_subject_matches() {
    let sock = scratch("face.sock");
    let r = router(&sock, None).await;
    let host_bus = util::bound(&r).await;
    let face = format!("unixsock-stream/{}", sock.display());

    let driver = client(&host_bus, None).await.expect("driver");
    let _queryables = serve(&driver).await;
    // The host bus is not the face: the call answers there, which is what
    // makes the silences below a deny rather than a missing queryable.
    let local = client(&host_bus, None).await.expect("a local caller");
    assert!(
        granted_answers(&local).await,
        "the host bus is not the face"
    );

    let anonymous = client(&face, None).await.expect("a peer on the face");
    for key in [GRANTED, NOT_GRANTED, READ, TELEMETRY, "v1/*/@rpc/**"] {
        assert!(
            answered(&anonymous, key, SILENCE).await.is_empty(),
            "no user: {key} stays home"
        );
    }

    drop((anonymous, local, driver, r));
    let _ = std::fs::remove_file(&sock);
}

/// The adopter's shape: its zenohd is a peer, the driver its client, and
/// the far side a peer too — and between two peers a call crosses only if
/// the queryable's *declaration* crossed first: `declare_queryable` egress
/// toward the face, which the face denies on the plane and the user's
/// subject carves with the call. (Against a router node the declaration
/// does not matter — a peer sends its queries to its router — which is why
/// this case is not the first test's with another mode.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_on_the_face_learns_the_granted_queryable_and_calls_it() {
    let sock = scratch("face.sock");
    let dict = scratch("usrpwd.txt");
    std::fs::write(
        &dict,
        "router:router-pw\ndriver:driver-pw\nops:ops-pw\nintruder:intruder-pw\n",
    )
    .unwrap();
    let r = node("peer", &sock, Some(&dict)).await;
    let host_bus = util::bound(&r).await;
    let face = format!("unixsock-stream/{}", sock.display());
    let driver = client(&host_bus, Some("driver")).await.expect("driver");
    let _queryables = serve(&driver).await;
    assert!(
        granted_answers(&driver).await,
        "the host bus is not the face"
    );

    let ops = session("peer", &face, Some("ops"))
        .await
        .expect("ops, a peer");
    assert!(
        granted_answers(&ops).await,
        "the granted call crosses to a peer"
    );
    assert!(answered(&ops, NOT_GRANTED, SILENCE).await.is_empty());
    let intruder = session("peer", &face, Some("intruder"))
        .await
        .expect("another peer");
    assert!(answered(&intruder, GRANTED, SILENCE).await.is_empty());

    drop((ops, intruder, driver, r));
    let _ = std::fs::remove_file(&dict);
    let _ = std::fs::remove_file(&sock);
}
