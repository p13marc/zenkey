//! `spec/scenarios/operations.md` §1–§8 (spec §5 O1–O7, §5.2, §6).
//!
//! Common setup: hosts `h1`, `h2`, `h3`, each with a service `tc`
//! implementing `tc.v1` (`tests/contracts/tc.v1.toml`):
//! `@op/interfaces/{if}/set` (exclusive, fanout forbidden),
//! `@op/diagnostics` (fanout allowed), `@op/listing` (many replies, a
//! summary) and the optional `@op/offload`, gated on `capability:xdp`.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{T, addr, client, config, contract, eventually, imp, router, router_with};
use serde::{Deserialize, Serialize};
use serde_json::json;
use zenkey::client::{Attribution, Fleet, Outcome};
use zenkey::model::descriptor::Cause;
use zenkey::model::envelope::{self, Detail};
use zenkey::model::grammar::IfaceId;
use zenkey::model::template::Bindings;
use zenkey::operation::{Call, CallMetadata, OpError, OperationServer};
use zenkey::ownership;
use zenkey::{Client, Service, ServiceBuilder};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};

fn tc() -> IfaceId {
    "tc.v1".parse().unwrap()
}

fn tc_contract() -> Arc<zenkey::model::contract::Contract> {
    Arc::new(contract("tc.v1"))
}

fn member(name: &str) -> Bindings {
    [("if".to_owned(), vec![name.to_owned()])].into()
}

const SET: &str = "@op/interfaces/{if}/set";
const DIAGNOSTICS: &str = "@op/diagnostics";
const LISTING: &str = "@op/listing";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetRequest {
    up: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct SetResponse {
    up: bool,
}

/// What one `tc` instance executed: every call that reached a handler.
#[derive(Default)]
struct Seen {
    set: AtomicU64,
    diagnostics: AtomicU64,
    listing: AtomicU64,
    /// The request ids of the `set` calls executed, from the call metadata.
    ids: Mutex<Vec<String>>,
    metadata: Mutex<Vec<Option<CallMetadata>>>,
}

/// How a `tc` instance behaves.
#[derive(Clone, Copy, Default)]
struct Behaviour {
    /// Its first `set` and first `diagnostics` hold the call past any
    /// caller's timeout before replying.
    freeze_first: bool,
    /// `diagnostics` never replies in time.
    frozen_diagnostics: bool,
}

/// A `tc` instance at `address` serving `set` over its template,
/// `diagnostics` and `listing`; `offload` is not served (no `xdp`).
async fn tc_service(
    s: &zenoh::Session,
    address: &str,
    how: Behaviour,
) -> (Service, Vec<OperationServer>, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let mut b = ServiceBuilder::new(s, config(address));
    b.implement(imp("tc.v1")).unwrap();
    let mut servers = Vec::new();

    let sn = Arc::clone(&seen);
    servers.push(
        b.serve_value(&tc(), SET, None, move |call: Call, req: SetRequest| {
            let sn = Arc::clone(&sn);
            async move {
                let n = sn.set.fetch_add(1, Ordering::SeqCst);
                sn.metadata.lock().unwrap().push(call.metadata().cloned());
                if let Some(id) = call.metadata().and_then(|m| m.request_id.clone()) {
                    sn.ids.lock().unwrap().push(id);
                }
                if how.freeze_first && n == 0 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                if call.values().unwrap()["if"] == ["eth9"] {
                    return Err(OpError::app(
                        "the kernel refused",
                        &json!({"kind": "kernel", "message": "no such device"}),
                    ));
                }
                Ok(SetResponse { up: req.up })
            }
        })
        .await
        .unwrap(),
    );

    let sn = Arc::clone(&seen);
    servers.push(
        b.serve_value(
            &tc(),
            DIAGNOSTICS,
            None,
            move |_call: Call, _req: serde_json::Value| {
                let sn = Arc::clone(&sn);
                async move {
                    let n = sn.diagnostics.fetch_add(1, Ordering::SeqCst);
                    if how.frozen_diagnostics || (how.freeze_first && n == 0) {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                    Ok(json!({"ok": true}))
                }
            },
        )
        .await
        .unwrap(),
    );

    let sn = Arc::clone(&seen);
    servers.push(
        b.serve(&tc(), LISTING, None, move |call: Call| {
            let sn = Arc::clone(&sn);
            async move {
                sn.listing.fetch_add(1, Ordering::SeqCst);
                for i in 0..5 {
                    call.reply_value(&json!({"name": format!("eth{i}")}))
                        .await?;
                }
                call.summary_value(&json!({"partial": false, "scanned": 5}))
                    .await?;
                Ok(())
            }
        })
        .await
        .unwrap(),
    );
    (b.start().await.unwrap(), servers, seen)
}

/// A caller's client of every `tc`.
fn caller(s: &zenoh::Session) -> Client {
    Client::new(s, tc_contract(), &["*/tc"]).unwrap()
}

/// Waits until the caller sees `n` `tc.v1` tokens of `selection`: a tool
/// acts on presence, never on an owner's `start()` returning.
async fn wait_present(s: &zenoh::Session, selection: &str, n: usize) {
    let fleet = Fleet::new(s, tc_contract(), &[selection]).unwrap();
    eventually("the tc instances are present", || async {
        zenkey::presence::tokens(s, &format!("zk2/{selection}/@zk/alive/tc.v1/**"), T)
            .await
            .unwrap()
            .len()
            == n
            && !fleet.present(T).await.unwrap().is_empty()
    })
    .await;
}

/// A raw GET, as a caller that breaks O2 would make it: every reply.
async fn raw_get(
    s: &zenoh::Session,
    sel: &str,
    payload: &[u8],
    consolidation: ConsolidationMode,
) -> Vec<Reply> {
    let rx = s
        .get(sel)
        .payload(payload.to_vec())
        .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
        .target(QueryTarget::All)
        .consolidation(consolidation)
        .timeout(T)
        .with(flume::unbounded::<Reply>())
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Ok(r) = rx.recv_async().await {
        out.push(r);
    }
    out
}

/// The codes of the envelopes among `replies`, by their `Encoding` alone.
fn refusal_codes(replies: &[Reply]) -> Vec<String> {
    replies
        .iter()
        .filter_map(|r| r.result().err())
        .filter_map(|e| {
            envelope::decode(&e.encoding().to_string(), &e.payload().to_bytes())
                .ok()
                .map(|env| env.code)
        })
        .collect()
}

/// §1: 200 concrete `BestMatching` calls execute 200 times with one
/// serving instance; with a second instance of the service behind another
/// router (a split-brain), each call runs on each side, 400 executions.
/// That is O1's documented limit, which §8 diagnoses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_at_most_once_while_one_instance_serves() {
    const N: usize = 200;
    let (_r1, ep1) = router(None).await;
    let (_r2, ep2) = router(Some(&ep1)).await;
    let (sa, sb, sc) = (client(&ep1).await, client(&ep2).await, client(&ep1).await);
    let (_a, _as, a) = tc_service(&sa, "h1/tc", Behaviour::default()).await;
    wait_present(&sc, "h1/tc", 1).await;
    let c = caller(&sc);
    let h1 = addr("h1/tc");
    let call = |id: String| {
        let c = c.clone().with_metadata(CallMetadata::new("s1", &id));
        let h1 = h1.clone();
        async move {
            c.call_value(&h1, SET, &member("eth0"), &SetRequest { up: true })
                .await
                .unwrap()
        }
    };

    // 1. One serving instance: one execution per call.
    for i in 0..N {
        assert!(matches!(call(format!("one-{i}")).await, Outcome::Value(_)));
    }
    let ran = |seen: &Seen, prefix: &str| {
        seen.ids
            .lock()
            .unwrap()
            .iter()
            .filter(|id| id.starts_with(prefix))
            .count()
    };
    assert_eq!(ran(&a, "one-"), N);

    // 2. A second instance of h1/tc behind R2. Re-issue a probe until it
    //    executes there, then measure.
    let (_b, _bs, b) = tc_service(&sb, "h1/tc", Behaviour::default()).await;
    let mut probe = 0;
    eventually("the second instance takes calls", || {
        probe += 1;
        let c = call(format!("probe-{probe}"));
        let b = Arc::clone(&b);
        async move {
            c.await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            ran(&b, "probe-") > 0
        }
    })
    .await;
    for i in 0..N {
        assert!(matches!(call(format!("two-{i}")).await, Outcome::Value(_)));
    }
    eventually("every call ran on each side", || async {
        ran(&a, "two-") == N && ran(&b, "two-") == N
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        (ran(&a, "two-"), ran(&b, "two-")),
        (N, N),
        "400 executions, one per side, and no more"
    );
}

/// §2: non-concrete calls to `set` are refused by every server they reach,
/// with `fanout_forbidden` and no execution; `diagnostics` on `zk2/*/tc/…`
/// with `All` + `None` gives one reply per host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_fan_out_refusal() {
    let (_r1, ep) = router(None).await;
    let caller_s = client(&ep).await;
    let mut hosts = Vec::new();
    for h in ["h1", "h2", "h3"] {
        let s = client(&ep).await;
        let (svc, servers, seen) = tc_service(&s, &format!("{h}/tc"), Behaviour::default()).await;
        hosts.push((s, svc, servers, seen));
    }
    wait_present(&caller_s, "*/tc", 3).await;
    let req = br#"{"up": true}"#;

    let one = raw_get(
        &caller_s,
        "zk2/h1/tc/tc.v1/@op/interfaces/*/set",
        req,
        ConsolidationMode::None,
    )
    .await;
    assert_eq!(refusal_codes(&one), ["fanout_forbidden"]);
    assert!(one.iter().all(|r| r.result().is_err()), "no value");
    let every = raw_get(
        &caller_s,
        "zk2/*/tc/tc.v1/@op/interfaces/*/set",
        req,
        ConsolidationMode::None,
    )
    .await;
    assert_eq!(refusal_codes(&every), ["fanout_forbidden"; 3]);
    assert!(every.iter().all(|r| r.result().is_err()), "no value");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        hosts.iter().all(|h| h.3.set.load(Ordering::SeqCst) == 0),
        "0 executions"
    );

    // The caller's side of O2: a fleet refuses to fan `set` out at all.
    let fleet = Fleet::new(&caller_s, tc_contract(), &["*/tc"]).unwrap();
    assert!(
        fleet
            .call_value(SET, &Bindings::new(), &json!({"up": true}))
            .await
            .is_err()
    );

    // `diagnostics` allows it: one reply per host, attributed by its key.
    let replies = fleet
        .call_value(DIAGNOSTICS, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    let mut by: Vec<String> = replies
        .repliers
        .iter()
        .map(|r| {
            assert_eq!(r.values.len(), 1);
            assert_eq!(
                r.values[0].value::<serde_json::Value>().unwrap()["ok"],
                true
            );
            r.addr.to_string()
        })
        .collect();
    by.sort();
    assert_eq!(by, ["h1/tc", "h2/tc", "h3/tc"]);
    assert!(replies.refusals.is_empty() && replies.discarded == 0);
    assert!(
        hosts
            .iter()
            .all(|h| h.3.diagnostics.load(Ordering::SeqCst) == 1)
    );
}

/// §3: a success is a value reply on the concrete key; an invalid request
/// a `reply_err` with `invalid_request`; an optional operation unavailable
/// for a missing capability, `unavailable` with `cause = capability`. The
/// envelope's encoding follows §5.2: the error type, else the response
/// type, decides it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_replies_and_errors() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (_svc, _servers, seen) = tc_service(&owner, "h1/tc", Behaviour::default()).await;
    wait_present(&tool, "h1/tc", 1).await;
    let c = caller(&tool);
    let h1 = addr("h1/tc");

    let ok = c
        .call_value(&h1, SET, &member("eth0"), &SetRequest { up: true })
        .await
        .unwrap();
    let Outcome::Value(answer) = ok else {
        panic!("a value reply: {ok:?}")
    };
    assert_eq!(answer.key(), "zk2/h1/tc/tc.v1/@op/interfaces/eth0/set");
    assert_eq!(answer.replier, h1);
    assert_eq!(
        answer.value::<SetResponse>().unwrap(),
        SetResponse { up: true }
    );
    assert_eq!(answer.sample.encoding().to_string(), "application/json");

    let invalid = c
        .call_value(&h1, SET, &member("eth0"), &json!({"up": "yes"}))
        .await
        .unwrap();
    assert_eq!(invalid.refusal().unwrap().code, "invalid_request");

    let gated = c
        .call_value(
            &h1,
            "@op/offload",
            &Bindings::new(),
            &SetRequest { up: true },
        )
        .await
        .unwrap();
    let env = gated.refusal().expect("an envelope");
    assert_eq!(
        (env.code.as_str(), env.cause.as_deref()),
        ("unavailable", Some("capability"))
    );

    // `app`, with the declared error type's value inline.
    let app = c
        .call_value(&h1, SET, &member("eth9"), &SetRequest { up: true })
        .await
        .unwrap();
    let env = app.refusal().unwrap();
    assert_eq!(env.code, "app");
    assert_eq!(
        env.detail,
        Some(Detail::Value(
            json!({"kind": "kernel", "message": "no such device"})
        ))
    );
    assert_eq!(
        seen.set.load(Ordering::SeqCst),
        2,
        "neither the invalid request nor the gated call reached a handler"
    );

    // The encoding, on the wire, for each kind of type (§5.2).
    let enc: IfaceId = "encodings.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("h1/enc"));
    b.implement(imp("encodings.v1")).unwrap();
    let mut servers = Vec::new();
    for (op, error) in [
        ("json", OpError::invalid_request("no")),
        ("cbor", OpError::invalid_request("no")),
        ("proto", OpError::invalid_request("no")),
        ("raw", OpError::invalid_request("no")),
        ("proto-error", OpError::app_bytes("no", vec![0x08, 0x01])),
        ("cbor-error", OpError::app("no", &json!({"kind": "busy"}))),
    ] {
        servers.push(
            b.serve(&enc, &format!("@op/{op}"), None, move |_call: Call| {
                let e = error.clone();
                async move { Err(e) }
            })
            .await
            .unwrap(),
        );
    }
    let _enc_svc = b.start().await.unwrap();
    eventually("h1/enc is present", || async {
        !zenkey::presence::tokens(&tool, "zk2/h1/enc/@zk/alive/**", T)
            .await
            .unwrap()
            .is_empty()
    })
    .await;
    for (op, wire, code) in [
        ("json", "application/json", "invalid_request"),
        ("cbor", "application/cbor", "invalid_request"),
        (
            "proto",
            "application/protobuf;zk2.core.v1.Error",
            "invalid_request",
        ),
        ("raw", "application/json", "invalid_request"),
        (
            "proto-error",
            "application/protobuf;zk2.core.v1.Error",
            "app",
        ),
        ("cbor-error", "application/cbor", "app"),
    ] {
        let replies = raw_get(
            &tool,
            &format!("zk2/h1/enc/encodings.v1/@op/{op}"),
            b"x",
            ConsolidationMode::None,
        )
        .await;
        let errs: Vec<_> = replies.iter().filter_map(|r| r.result().err()).collect();
        assert_eq!(errs.len(), 1, "{op}: one refusal");
        assert_eq!(errs[0].encoding().to_string(), wire, "{op}");
        let env = envelope::decode(wire, &errs[0].payload().to_bytes()).unwrap();
        assert_eq!(env.code, code, "{op}");
        if op == "proto-error" {
            assert_eq!(env.detail, Some(Detail::Bytes(vec![0x08, 0x01])));
        }
    }
}

/// §4: a caller retries an idempotent operation after a timeout, and never
/// retries a non-idempotent one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_retries() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let how = Behaviour {
        freeze_first: true,
        ..Behaviour::default()
    };
    let (_svc, _servers, seen) = tc_service(&owner, "h1/tc", how).await;
    wait_present(&tool, "h1/tc", 1).await;
    let c = caller(&tool)
        .with_timeout(Duration::from_millis(400))
        .with_retries(3);
    let h1 = addr("h1/tc");

    // `diagnostics` is idempotent: the first call times out, the retry
    // answers.
    let out = c
        .call_value(&h1, DIAGNOSTICS, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Value(_)), "{out:?}");
    assert_eq!(seen.diagnostics.load(Ordering::SeqCst), 2);

    // `set` is not: one call, whatever the client allows.
    let out = c
        .call_value(&h1, SET, &member("eth0"), &SetRequest { up: true })
        .await
        .unwrap();
    let silence = out.silence().expect("no answer");
    assert_eq!(silence.attempts, 1);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(seen.set.load(Ordering::SeqCst), 1, "never retried");
}

/// §5: a call the access control refuses, and one to a frozen server, both
/// return no reply. The tool says "no answer" and attributes it through
/// presence; it never says "no such operation".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_silence() {
    let acl = r#"{
        enabled: true,
        default_permission: "allow",
        rules: [{
            id: "no-set",
            messages: ["query"],
            flows: ["ingress"],
            permission: "deny",
            key_exprs: ["zk2/h1/tc/tc.v1/@op/interfaces/*/set"],
        }],
        subjects: [{ id: "anyone" }],
        policies: [{ rules: ["no-set"], subjects: ["anyone"] }],
    }"#;
    let (_r1, ep) = router_with(None, &[("access_control", acl)]).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let how = Behaviour {
        frozen_diagnostics: true,
        ..Behaviour::default()
    };
    let (_svc, _servers, seen) = tc_service(&owner, "h1/tc", how).await;
    wait_present(&tool, "h1/tc", 1).await;
    let c = caller(&tool).with_timeout(Duration::from_millis(500));
    let h1 = addr("h1/tc");

    let refused = c
        .call_value(&h1, SET, &member("eth0"), &SetRequest { up: true })
        .await
        .unwrap();
    let frozen = c
        .call_value(&h1, DIAGNOSTICS, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    for out in [&refused, &frozen] {
        let Outcome::NoAnswer(s) = out else {
            panic!("no answer, not a verdict: {out:?}")
        };
        assert_eq!(s.presence, Attribution::Present, "the service is up");
    }
    assert_eq!(seen.set.load(Ordering::SeqCst), 0, "the router refused it");

    // Presence tells silence apart from absence.
    let ghost = Client::new(&tool, tc_contract(), &["*/tc"])
        .unwrap()
        .with_timeout(Duration::from_millis(300));
    let out = ghost
        .call_value(&addr("h9/tc"), DIAGNOSTICS, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    assert_eq!(out.silence().unwrap().presence, Attribution::Absent);
}

/// §6: two servers each reply 5 values and a summary to one call. With
/// consolidation `None` the caller gets 10 values and 2 summaries; with
/// `Latest` or `Auto`, values are lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_many_replies() {
    let (_r1, ep) = router(None).await;
    let (s1, s2, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (_a, _as, a) = tc_service(&s1, "h1/tc", Behaviour::default()).await;
    let (_b, _bs, b) = tc_service(&s2, "h2/tc", Behaviour::default()).await;
    wait_present(&tool, "*/tc", 2).await;

    let fleet = Fleet::new(&tool, tc_contract(), &["*/tc"]).unwrap();
    let replies = fleet
        .call_value(LISTING, &Bindings::new(), &json!({"limit": 5}))
        .await
        .unwrap();
    assert_eq!(replies.values().count(), 10);
    assert_eq!(replies.repliers.len(), 2);
    for r in &replies.repliers {
        assert_eq!(r.values.len(), 5, "{}", r.addr);
        let summary = r.summary().expect("exactly one summary");
        assert_eq!(
            summary.value::<serde_json::Value>().unwrap(),
            json!({"partial": false, "scanned": 5})
        );
    }
    assert!(replies.partial().is_empty());
    assert_eq!(
        (
            a.listing.load(Ordering::SeqCst),
            b.listing.load(Ordering::SeqCst)
        ),
        (1, 1)
    );

    // The reason `None` is required.
    for mode in [ConsolidationMode::Latest, ConsolidationMode::Auto] {
        let kept = raw_get(&tool, "zk2/*/tc/tc.v1/@op/listing", b"{}", mode)
            .await
            .iter()
            .filter(|r| r.result().is_ok())
            .count();
        assert!(kept < 10, "{mode:?} kept {kept} of 12 replies");
    }
}

/// §7: a request carries `{actor, request_id}`; the server sees both and
/// may record them. Nothing authenticates them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_call_metadata() {
    let (_r1, ep) = router(None).await;
    let (owner, ws, other) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (_svc, _servers, seen) = tc_service(&owner, "h1/tc", Behaviour::default()).await;
    wait_present(&ws, "h1/tc", 1).await;
    let claim = CallMetadata::new("operator@ws-01", "r-42");
    let h1 = addr("h1/tc");
    for s in [&ws, &other] {
        let out = caller(s)
            .with_metadata(claim.clone())
            .call_value(&h1, SET, &member("eth0"), &SetRequest { up: false })
            .await
            .unwrap();
        assert!(matches!(out, Outcome::Value(_)));
    }
    // Without the attachment, the call is the same call.
    let out = caller(&ws)
        .call_value(&h1, SET, &member("eth0"), &SetRequest { up: false })
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Value(_)));
    assert_eq!(
        *seen.metadata.lock().unwrap(),
        [Some(claim.clone()), Some(claim), None],
        "two sessions claim one actor, and the server cannot tell them apart"
    );
}

/// §8: two instances holding one interface token past the grace period are
/// a finding; a re-mint, whose overlap is shorter, is not; nor is a standby
/// holding its instance token only. The runtime fences nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_split_brain_diagnosis() {
    const GRACE: Duration = Duration::from_millis(600);
    let alive = "zk2/*/*/@zk/alive/**";
    let (_r1, ep) = router(None).await;
    let (sa, sb, sc, tool) = (
        client(&ep).await,
        client(&ep).await,
        client(&ep).await,
        client(&ep).await,
    );
    let (mut a, _as, _a_seen) = tc_service(&sa, "h1/tc", Behaviour::default()).await;

    // 1. A second instance of h1/tc, both holding tc.v1's token.
    let (b, _bs, _b_seen) = tc_service(&sb, "h1/tc", Behaviour::default()).await;
    wait_present(&tool, "h1/tc", 2).await;
    let found = ownership::split_brain(&tool, alive, GRACE, T)
        .await
        .unwrap()
        .findings;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].service, addr("h1/tc"));
    assert_eq!(found[0].iface, tc());
    let mut both = vec![a.instance().clone(), b.instance().clone()];
    both.sort();
    assert_eq!(found[0].instances, both);
    // Nothing was fenced: both still hold the token, and both still serve.
    assert_eq!(a.tokens_held(), [tc()]);
    assert_eq!(b.tokens_held(), [tc()]);
    b.close().await.unwrap();
    wait_present(&tool, "h1/tc", 1).await;

    // 2. The remaining instance re-mints during the check.
    let check = tokio::spawn({
        let tool = tool.clone();
        async move { ownership::split_brain(&tool, alive, GRACE, T).await }
    });
    tokio::time::sleep(GRACE / 3).await;
    let next = a.new_epoch().await.unwrap();
    assert!(
        check.await.unwrap().unwrap().is_clear(),
        "a re-mint is no finding"
    );
    let holders = zenkey::presence::tokens(&tool, "zk2/h1/tc/@zk/alive/**", T)
        .await
        .unwrap();
    assert!(matches!(
        holders.as_slice(),
        [zenkey::model::grammar::ZkKey::Alive { instance, .. }] if *instance == next
    ));

    // 3. A standby: an instance token, no interface token, no queryable.
    let standby = ServiceBuilder::new(&sc, config("h1/tc"))
        .start()
        .await
        .unwrap();
    assert!(standby.tokens_held().is_empty());
    assert!(standby.unavailable_queryables().is_empty());
    eventually("the standby is present", || async {
        zenkey::presence::tokens(&tool, "zk2/h1/tc/@zk/instance/*", T)
            .await
            .unwrap()
            .len()
            == 2
    })
    .await;
    assert!(
        ownership::split_brain(&tool, alive, GRACE, T)
            .await
            .unwrap()
            .is_clear(),
        "a standby is no finding"
    );
}

/// The active instance answers `unavailable` for an optional operation it
/// does not serve, and the cause follows the capabilities and the
/// `unavailable` list as they change (O3, §3.3). A handler that returns
/// without its reply leaves `internal`, never silence.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unavailable_follows_exposure() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let mut b = ServiceBuilder::new(&owner, config("h1/tc").capability("xdp"));
    b.implement(imp("tc.v1")).unwrap();
    let ran = Arc::new(AtomicU64::new(0));
    let mut servers = Vec::new();
    for op in [SET, DIAGNOSTICS, LISTING, "@op/offload"] {
        let r = Arc::clone(&ran);
        servers.push(
            b.serve(&tc(), op, None, move |call: Call| {
                let r = Arc::clone(&r);
                async move {
                    // Only `offload` replies; the others return without.
                    if op == "@op/offload" {
                        r.fetch_add(1, Ordering::SeqCst);
                        call.reply_value(&json!({"up": true})).await?;
                    }
                    Ok(())
                }
            })
            .await
            .unwrap(),
        );
    }
    let mut svc = b.start().await.unwrap();
    assert!(svc.unavailable_queryables().is_empty(), "offload is served");
    wait_present(&tool, "h1/tc", 1).await;
    let c = caller(&tool);
    let h1 = addr("h1/tc");
    let offload = || async {
        c.call_value(&h1, "@op/offload", &Bindings::new(), &json!({"up": true}))
            .await
            .unwrap()
    };
    assert!(matches!(offload().await, Outcome::Value(_)));

    svc.set_capabilities(Default::default()).await.unwrap();
    let env = offload().await.refusal().cloned().expect("refused");
    assert_eq!(env.cause.as_deref(), Some("capability"));

    svc.set_capabilities(["xdp".to_owned()].into())
        .await
        .unwrap();
    svc.set_unavailable(
        &tc(),
        "@op/offload",
        Some((Cause::Config, Some("disabled"))),
    )
    .unwrap();
    let env = offload().await.refusal().cloned().expect("refused");
    assert_eq!(
        (env.cause.as_deref(), env.message.as_str()),
        (Some("config"), "disabled")
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1, "only the first call ran");

    // No reply from the handler: `internal`, not silence.
    let out = c
        .call_value(&h1, DIAGNOSTICS, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    assert_eq!(out.refusal().map(|e| e.code.as_str()), Some("internal"));
    let listing = Fleet::new(&tool, tc_contract(), &["h1/tc"])
        .unwrap()
        .call_value(LISTING, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    assert!(listing.repliers.is_empty());
    assert_eq!(listing.refusals.len(), 1);
    assert_eq!(listing.refusals[0].code, "internal", "no summary");
}
