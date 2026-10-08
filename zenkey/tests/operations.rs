//! `spec/scenarios/operations.md` §1–§8, core 0.7 (spec §5 O1–O7, §5.2,
//! §6; #660).
//!
//! Common setup: hosts `h1`, `h2`, `h3`, each with a service `tc`
//! implementing `tc.v1` (`tests/contracts/tc.v1.toml`):
//! `@op/interfaces/{if}/set` (exclusive, fanout forbidden),
//! `@op/diagnostics` (fanout allowed), `@op/listing` (many replies, a
//! summary), the optional `@op/offload`, gated on `capability:xdp`, and §2's
//! `@op/interfaces/{if}/reset` (fanout allowed). `tc-replicated.toml` is the
//! variant of §3 step 7, §6 step 3 and §8 step 5, and `ping.v1.toml` the
//! interface of §8 step 4.
//!
//! **What a query carries.** zenoh 1.10.1 hands a queryable neither a
//! query's target nor its consolidation, so where a scenario captures them
//! (§1), the test reads what each one does: `BestMatching` reaches one
//! `complete` queryable per router, where `All` reaches every one, and `None`
//! delivers a reply before the query completes, where `Latest` holds it.

mod common;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{T, addr, client, config, contract, eventually, imp, router, router_with};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zenkey::client::{Attribution, Fleet, Outcome};
use zenkey::codec::Json;
use zenkey::model::descriptor::Cause;
use zenkey::model::envelope::{self, Detail};
use zenkey::model::grammar::{Addr, IfaceId};
use zenkey::model::template::Bindings;
use zenkey::operation::{Call, CallMetadata, OpError, OperationServer};
use zenkey::ownership;
use zenkey::{CallInfo, Client, Implementation, Service, ServiceBuilder};
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
const OFFLOAD: &str = "@op/offload";
const RESET: &str = "@op/interfaces/{if}/reset";

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
    reset: AtomicU64,
    /// The request ids of the `set` calls executed, from the call metadata.
    ids: Mutex<Vec<String>>,
    metadata: Mutex<Vec<Option<CallMetadata>>>,
}

fn count(n: &AtomicU64) -> u64 {
    n.load(Ordering::SeqCst)
}

/// How a `tc` instance serves `reset` (§2).
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Reset {
    /// One queryable per member, `eth0` and `eth1` (`h1`).
    PerMember,
    /// One typed queryable over the template, naming `eth0` when the key
    /// names no member (`h2`).
    #[default]
    Template,
    /// One queryable over the template, refusing every call `busy` (`h3`).
    Busy,
}

/// How a `tc` instance behaves.
#[derive(Clone, Copy, Default)]
struct Behaviour {
    /// `set` holds every call past any caller's timeout.
    frozen_set: bool,
    /// `set` replies, then holds the call for [`LINGER`]: its query
    /// completes only then (§1).
    linger_set: bool,
    /// `diagnostics` holds every call past any caller's timeout.
    frozen_diagnostics: bool,
    /// `diagnostics` refuses every call `busy`.
    busy_diagnostics: bool,
    /// `listing` sends its values and returns before its summary.
    cut_listing: bool,
    reset: Reset,
}

/// How long a frozen handler holds a call: past every caller's timeout.
const FROZEN: Duration = Duration::from_secs(2);
/// How long a lingering `set` holds its call after replying.
const LINGER: Duration = Duration::from_secs(2);

/// A `tc` instance at `address` serving `set` over its template,
/// `diagnostics`, `listing` and `reset`; `offload` is not served (no `xdp`).
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
        b.serve(&tc(), SET, None, move |call: Call| {
            let sn = Arc::clone(&sn);
            async move {
                let req: SetRequest = call.request()?;
                let n = sn.set.fetch_add(1, Ordering::SeqCst);
                sn.metadata.lock().unwrap().push(call.metadata().cloned());
                if let Some(id) = call.metadata().and_then(|m| m.request_id.clone()) {
                    sn.ids.lock().unwrap().push(id);
                }
                if how.frozen_set {
                    tokio::time::sleep(FROZEN).await;
                }
                if call.values().unwrap()["if"] == ["eth9"] {
                    return Err(OpError::app(
                        "the kernel refused",
                        &json!({"kind": "kernel", "message": "no such device"}),
                    ));
                }
                call.reply_value(&SetResponse { up: req.up }).await?;
                if how.linger_set && n % 2 == 0 {
                    tokio::time::sleep(LINGER).await;
                }
                Ok(())
            }
        })
        .await
        .unwrap(),
    );

    let sn = Arc::clone(&seen);
    servers.push(
        b.serve_value(&tc(), DIAGNOSTICS, None, move |_call: Call, _req: Value| {
            let sn = Arc::clone(&sn);
            async move {
                sn.diagnostics.fetch_add(1, Ordering::SeqCst);
                if how.frozen_diagnostics {
                    tokio::time::sleep(FROZEN).await;
                }
                if how.busy_diagnostics {
                    return Err(OpError::busy("a scan is running"));
                }
                Ok(json!({"ok": true}))
            }
        })
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
                if !how.cut_listing {
                    call.summary_value(&json!({"partial": false, "scanned": 5}))
                        .await?;
                }
                Ok(())
            }
        })
        .await
        .unwrap(),
    );

    match how.reset {
        Reset::PerMember => {
            for m in ["eth0", "eth1"] {
                let sn = Arc::clone(&seen);
                servers.push(
                    b.serve(&tc(), RESET, Some(&member(m)), move |call: Call| {
                        let sn = Arc::clone(&sn);
                        async move {
                            sn.reset.fetch_add(1, Ordering::SeqCst);
                            call.reply_value(&json!({"ok": true})).await?;
                            Ok(())
                        }
                    })
                    .await
                    .unwrap(),
                );
            }
        }
        Reset::Template | Reset::Busy => {
            let sn = Arc::clone(&seen);
            let busy = how.reset == Reset::Busy;
            servers.push(
                zenkey::typed::serve_one::<Json<Value>, Json<Value>, _, _>(
                    &mut b,
                    &tc(),
                    RESET,
                    move |_req: Value, info: CallInfo| {
                        let sn = Arc::clone(&sn);
                        async move {
                            sn.reset.fetch_add(1, Ordering::SeqCst);
                            if busy {
                                return Err(OpError::busy("resetting already"));
                            }
                            // C-2: a typed handler names the member a
                            // fan-out answers for, unless its key did.
                            if info.values().is_none() {
                                info.member(&member("eth0"))?;
                            }
                            Ok(json!({"ok": true}))
                        }
                    },
                )
                .await
                .unwrap(),
            );
        }
    }
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

/// A raw GET, as a caller that breaks the rules would make it: every reply.
async fn raw_get(
    s: &zenoh::Session,
    sel: &str,
    payload: &[u8],
    target: QueryTarget,
    consolidation: ConsolidationMode,
) -> Vec<Reply> {
    raw_get_with(s, sel, payload, target, consolidation, None).await
}

async fn raw_get_with(
    s: &zenoh::Session,
    sel: &str,
    payload: &[u8],
    target: QueryTarget,
    consolidation: ConsolidationMode,
    attachment: Option<&[u8]>,
) -> Vec<Reply> {
    let rx = s
        .get(sel)
        .payload(payload.to_vec())
        .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
        .attachment(attachment.map(<[u8]>::to_vec))
        .target(target)
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

/// A fan-out, raw: `All` + `None` (O2).
async fn fan(s: &zenoh::Session, sel: &str, payload: &[u8]) -> Vec<Reply> {
    raw_get(s, sel, payload, QueryTarget::All, ConsolidationMode::None).await
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

fn values(replies: &[Reply]) -> usize {
    replies.iter().filter(|r| r.result().is_ok()).count()
}

/// §1: 200 concrete calls execute 200 times with one serving instance, and
/// each returns on its first reply, before its query completes (`None`).
/// A second instance on the same router takes none of them (`BestMatching`
/// reaches one `complete` queryable per router). One behind another router
/// (a split-brain) runs each call on each side: 400 executions, both
/// replies seen with `None`, one per key under `Latest`. That is O1's
/// documented limit, which §8 diagnoses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_at_most_once_while_one_instance_serves() {
    const N: usize = 200;
    let (_r1, ep1) = router(None).await;
    let (_r2, ep2) = router(Some(&ep1)).await;
    let (sa, sb, sc) = (client(&ep1).await, client(&ep2).await, client(&ep1).await);
    let lingering = Behaviour {
        linger_set: true,
        ..Behaviour::default()
    };
    let (_a, _as, a) = tc_service(&sa, "h1/tc", lingering).await;
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
    let ran = |seen: &Seen, prefix: &str| {
        seen.ids
            .lock()
            .unwrap()
            .iter()
            .filter(|id| id.starts_with(prefix))
            .count()
    };

    // 1. One serving instance: one execution per call, and the caller
    //    returns on the first reply. Every other `set` holds its query open
    //    for LINGER after replying; under `Latest` the call would wait it out.
    for i in 0..N {
        let started = Instant::now();
        assert!(matches!(call(format!("one-{i}")).await, Outcome::Value(_)));
        assert!(
            started.elapsed() < LINGER,
            "call {i} returned on its first reply, before its query completed"
        );
    }
    assert_eq!(ran(&a, "one-"), N);

    // `BestMatching`: a second instance on the same router takes none of
    // the calls the first one runs; `All` would run each on both.
    let sd = client(&ep1).await;
    let (d_svc, d_servers, d) = tc_service(&sd, "h1/tc", Behaviour::default()).await;
    wait_present(&sc, "h1/tc", 2).await;
    for i in 0..N {
        assert!(matches!(call(format!("same-{i}")).await, Outcome::Value(_)));
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        ran(&a, "same-") + ran(&d, "same-"),
        N,
        "one execution per call on one router"
    );
    drop(d_servers);
    d_svc.close().await.unwrap();
    wait_present(&sc, "h1/tc", 1).await;

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

    // Both replies of one call with `None`; one per key under `Latest`.
    let key = "zk2/h1/tc/tc.v1/@op/interfaces/eth0/set";
    let req = br#"{"up": true}"#;
    let none = raw_get(
        &sc,
        key,
        req,
        QueryTarget::BestMatching,
        ConsolidationMode::None,
    )
    .await;
    assert_eq!(values(&none), 2, "one reply per side");
    let latest = raw_get(
        &sc,
        key,
        req,
        QueryTarget::BestMatching,
        ConsolidationMode::Latest,
    )
    .await;
    assert_eq!(
        values(&latest),
        1,
        "one per key: the second execution vanishes"
    );
}

/// §2: non-concrete calls to `set` are refused by every server they reach,
/// with `fanout_forbidden` and no execution; `diagnostics` on `zk2/*/tc/…`
/// gives one reply per host. A fan-out over `reset`'s template: `h1`'s
/// per-member queryables answer for both members, `h2`'s template-wide
/// typed server for the member it names, on that member's key (C-2), and
/// `h3`'s `busy` is reported unattributed; presence shows `h3` holding the
/// token and sending no value.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_fan_out() {
    let (_r1, ep) = router(None).await;
    let caller_s = client(&ep).await;
    let mut hosts = Vec::new();
    for (h, reset) in [
        ("h1", Reset::PerMember),
        ("h2", Reset::Template),
        ("h3", Reset::Busy),
    ] {
        let s = client(&ep).await;
        let how = Behaviour {
            reset,
            ..Behaviour::default()
        };
        let (svc, servers, seen) = tc_service(&s, &format!("{h}/tc"), how).await;
        hosts.push((s, svc, servers, seen));
    }
    wait_present(&caller_s, "*/tc", 3).await;
    let seen = |i: usize| Arc::clone(&hosts[i].3);
    let req = br#"{"up": true}"#;

    // 1. `set` forbids a fan-out.
    let one = fan(&caller_s, "zk2/h1/tc/tc.v1/@op/interfaces/*/set", req).await;
    assert_eq!(refusal_codes(&one), ["fanout_forbidden"]);
    assert_eq!(values(&one), 0, "no value");
    let every = fan(&caller_s, "zk2/*/tc/tc.v1/@op/interfaces/*/set", req).await;
    assert_eq!(refusal_codes(&every), ["fanout_forbidden"; 3]);
    assert_eq!(values(&every), 0, "no value");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!((0..3).all(|i| count(&seen(i).set) == 0), "0 executions");
    // The caller's side of O2: a fleet refuses to fan `set` out at all.
    let fleet = Fleet::new(&caller_s, tc_contract(), &["*/tc"]).unwrap();
    assert!(
        fleet
            .call_value(SET, &Bindings::new(), &json!({"up": true}))
            .await
            .is_err()
    );

    // 2. `diagnostics` allows it: one reply per host, attributed by key.
    let replies = fleet
        .call_value(DIAGNOSTICS, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    let by: BTreeSet<String> = replies
        .repliers
        .iter()
        .map(|r| {
            assert_eq!(r.values.len(), 1);
            assert_eq!(r.values[0].value::<Value>().unwrap()["ok"], true);
            r.addr.to_string()
        })
        .collect();
    assert_eq!(by, ["h1/tc", "h2/tc", "h3/tc"].map(String::from).into());
    assert!(replies.refusals.is_empty() && replies.discarded == 0);
    assert!((0..3).all(|i| count(&seen(i).diagnostics) == 1));

    // 3. `reset` over its template: `zk2/*/tc/tc.v1/@op/interfaces/*/reset`.
    assert_eq!(
        fleet.selectors(RESET, &Bindings::new()).unwrap()[0].as_str(),
        "zk2/*/tc/tc.v1/@op/interfaces/*/reset"
    );
    let replies = fleet
        .call_value(RESET, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    let keys: BTreeSet<&str> = replies.repliers.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "zk2/h1/tc/tc.v1/@op/interfaces/eth0/reset",
            "zk2/h1/tc/tc.v1/@op/interfaces/eth1/reset",
            "zk2/h2/tc/tc.v1/@op/interfaces/eth0/reset",
        ]
        .into(),
        "h2 replies on the key of the member its server named"
    );
    assert!(replies.repliers.iter().all(|r| r.values.len() == 1));
    let codes: Vec<&str> = replies.refusals.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(
        codes,
        ["busy"],
        "a refusal, unattributed: it carries no key"
    );
    assert!(replies.malformed.is_empty() && replies.discarded == 0);
    // Who sent no value is read from presence: refused or silent, alike.
    let answered: BTreeSet<Addr> = replies.repliers.iter().map(|r| r.addr.clone()).collect();
    let present = fleet.present(T).await.unwrap();
    let no_value: Vec<&Addr> = present.iter().filter(|a| !answered.contains(*a)).collect();
    assert_eq!(no_value, [&addr("h3/tc")]);

    // A key that binds every parameter names the member by itself: the
    // template-wide server replies on it without naming one.
    let replies = fleet
        .call_value(RESET, &member("eth1"), &json!({}))
        .await
        .unwrap();
    let keys: BTreeSet<&str> = replies.repliers.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "zk2/h1/tc/tc.v1/@op/interfaces/eth1/reset",
            "zk2/h2/tc/tc.v1/@op/interfaces/eth1/reset",
        ]
        .into()
    );
    assert_eq!(replies.refusals.len(), 1);
    // A fan-out whose parameter chunk is not a canonical slug names no
    // member (§5.1): `invalid_request` from each template-wide server,
    // before any handler.
    let malformed = fan(&caller_s, "zk2/*/tc/tc.v1/@op/interfaces/ETH0/reset", b"{}").await;
    assert_eq!(refusal_codes(&malformed), ["invalid_request"; 2]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        [0, 1, 2].map(|i| count(&seen(i).reset)),
        [3, 2, 2],
        "h1: eth0, eth1, eth1; h2: two; h3: two refused"
    );
}

/// The `encodings.v1` service of §3: each operation refuses as its request
/// says: `none` is `app` without a detail, `detail` and `misfit` the given
/// errors, `silent` returns without replying, anything else
/// `invalid_request`.
async fn encodings_service(s: &zenoh::Session) -> (Service, Vec<OperationServer>) {
    let enc: IfaceId = "encodings.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(s, config("h1/enc"));
    b.implement(imp("encodings.v1")).unwrap();
    let jpeg = vec![0xff, 0xd8, 0xff, 0xe0];
    let tc_error = json!({"kind": "kernel", "message": "no such device"});
    let app = |d: &Value| OpError::app("refused", d);
    let bytes = |b: &[u8]| OpError::app_bytes("refused", b.to_vec());
    let mut servers = Vec::new();
    for (op, detail, misfit) in [
        ("json", app(&tc_error), bytes(&[1])),
        ("cbor", app(&tc_error), bytes(&[1])),
        ("proto", bytes(&[0x08, 0x01]), bytes(&[1])),
        ("raw", bytes(&jpeg), app(&tc_error)),
        ("proto-error", bytes(&[0x08, 0x01]), app(&tc_error)),
        ("cbor-error", app(&json!({"kind": "busy"})), bytes(&[1])),
        ("json-error", app(&tc_error), bytes(&[1])),
        ("raw-error", bytes(&jpeg), app(&json!("not base64!"))),
    ] {
        servers.push(
            b.serve(&enc, &format!("@op/{op}"), None, move |call: Call| {
                let (detail, misfit) = (detail.clone(), misfit.clone());
                async move {
                    let body = call
                        .payload()
                        .map(|p| p.to_bytes().into_owned())
                        .unwrap_or_default();
                    match body.as_slice() {
                        b"silent" => Ok(()),
                        b"none" => Err(OpError::app_without_detail("refused")),
                        b"detail" => Err(detail),
                        b"misfit" => Err(misfit),
                        _ => Err(OpError::invalid_request("no")),
                    }
                }
            })
            .await
            .unwrap(),
        );
    }
    (b.start().await.unwrap(), servers)
}

/// §3: a success is a value reply on the concrete key; a request that does
/// not decode, and a key whose parameter chunk is not a canonical slug, are
/// `invalid_request`; an optional operation unavailable for a missing
/// capability is `unavailable` with `cause = capability`; a handler that
/// returns without replying leaves `internal`; `app` comes without a detail,
/// with a value inline, or with a raw type's bytes as base64 text, and a
/// detail that does not fit goes out as `internal`. The envelope's encoding
/// follows §5.2: the error type, else the response type, decides it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_replies_and_errors() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (_svc, _servers, seen) = tc_service(&owner, "h1/tc", Behaviour::default()).await;
    wait_present(&tool, "h1/tc", 1).await;
    let c = caller(&tool);
    let h1 = addr("h1/tc");

    // 1. A value reply on the concrete key.
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

    // 2. A request that does not decode as the request type (O-13).
    let invalid = c
        .call_value(&h1, SET, &member("eth0"), &json!({"up": "yes"}))
        .await
        .unwrap();
    assert_eq!(invalid.refusal().unwrap().code, "invalid_request");

    // 3. A key that names no member (O-4): invalid, not not_found.
    let upper = raw_get(
        &tool,
        "zk2/h1/tc/tc.v1/@op/interfaces/ETH0/set",
        br#"{"up": true}"#,
        QueryTarget::BestMatching,
        ConsolidationMode::None,
    )
    .await;
    assert_eq!(refusal_codes(&upper), ["invalid_request"]);

    // 4. An optional operation gated on a capability not held.
    let gated = c
        .call_value(&h1, OFFLOAD, &Bindings::new(), &SetRequest { up: true })
        .await
        .unwrap();
    let env = gated.refusal().expect("an envelope");
    assert_eq!(
        (env.code.as_str(), env.cause.as_deref()),
        ("unavailable", Some("capability"))
    );

    // 6. `app`, with the declared error type's value inline.
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
        count(&seen.set),
        2,
        "neither the invalid requests nor the gated call reached a handler"
    );

    // The envelope on the wire, for each kind of type (§5.2), and steps 5
    // and 6 across them.
    let (_enc_svc, _enc_servers) = encodings_service(&owner).await;
    eventually("h1/enc is present", || async {
        !zenkey::presence::tokens(&tool, "zk2/h1/enc/@zk/alive/**", T)
            .await
            .unwrap()
            .is_empty()
    })
    .await;
    const PB: &str = "application/protobuf;zk2.core.v1.Error";
    const JS: &str = "application/json";
    const CB: &str = "application/cbor";
    let tc_error = json!({"kind": "kernel", "message": "no such device"});
    let base64 = Detail::Value(json!("/9j/4A=="));
    for (op, payload, wire, code, detail) in [
        // The encoding follows the error type, else the response type.
        ("json", "x", JS, "invalid_request", None),
        ("cbor", "x", CB, "invalid_request", None),
        ("proto", "x", PB, "invalid_request", None),
        ("raw", "x", JS, "invalid_request", None),
        // 5. A handler that returns without replying.
        ("json", "silent", JS, "internal", None),
        ("raw", "silent", JS, "internal", None),
        // 6. No `error` type: `app` and no detail; a detail is `internal`.
        ("json", "none", JS, "app", None),
        ("proto", "none", PB, "app", None),
        ("raw", "none", JS, "app", None),
        ("json", "detail", JS, "internal", None),
        ("proto", "detail", PB, "internal", None),
        ("raw", "detail", JS, "internal", None),
        // A JSON Schema `error` type: the value inline, or none.
        (
            "json-error",
            "detail",
            JS,
            "app",
            Some(Detail::Value(tc_error.clone())),
        ),
        ("json-error", "none", JS, "app", None),
        ("json-error", "misfit", JS, "internal", None),
        (
            "cbor-error",
            "detail",
            CB,
            "app",
            Some(Detail::Value(json!({"kind": "busy"}))),
        ),
        ("cbor-error", "misfit", CB, "internal", None),
        // A protobuf `error` type: the encoded message, or none.
        (
            "proto-error",
            "detail",
            PB,
            "app",
            Some(Detail::Bytes(vec![0x08, 0x01])),
        ),
        ("proto-error", "none", PB, "app", None),
        ("proto-error", "misfit", PB, "internal", None),
        // A raw `error` type: a JSON envelope, the bytes as base64 text.
        ("raw-error", "detail", JS, "app", Some(base64.clone())),
        ("raw-error", "none", JS, "app", None),
        ("raw-error", "misfit", JS, "internal", None),
    ] {
        let replies = raw_get(
            &tool,
            &format!("zk2/h1/enc/encodings.v1/@op/{op}"),
            payload.as_bytes(),
            QueryTarget::BestMatching,
            ConsolidationMode::None,
        )
        .await;
        let errs: Vec<_> = replies.iter().filter_map(|r| r.result().err()).collect();
        assert_eq!(errs.len(), 1, "{op} {payload}: one refusal");
        assert_eq!(errs[0].encoding().to_string(), wire, "{op} {payload}");
        let env = envelope::decode(wire, &errs[0].payload().to_bytes()).unwrap();
        assert_eq!(
            (env.code.as_str(), env.detail.as_ref()),
            (code, detail.as_ref()),
            "{op} {payload}"
        );
    }
    // A caller holding the contract reads the raw detail back as bytes.
    assert_eq!(base64.raw_bytes(), Some(vec![0xff, 0xd8, 0xff, 0xe0]));
}

/// §3 step 7: beside a replica. In `tc-replicated`, `set` is optional and
/// `diagnostics` and `listing` replicated; a second instance of `h1/tc`
/// exposes the replicated ones only and lists `set` unavailable. Every
/// call to `set` executes on the first instance, and none is answered
/// `unavailable`: the replica declares no queryable on `set` (O-10).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_7_beside_a_replica() {
    const N: u64 = 200;
    let (_r1, ep) = router(None).await;
    let (sa, sb, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (full, _fs, ran) = replicated_tc(&sa, "h1/tc", true).await;
    let (replica, _rs, _) = replicated_tc(&sb, "h1/tc", false).await;
    assert!(full.unavailable_queryables().is_empty());
    assert!(
        replica.unavailable_queryables().is_empty(),
        "a replica intercepts no call to an exclusive operation"
    );
    wait_present(&tool, "h1/tc", 2).await;
    let c = Client::new(&tool, Arc::new(contract("tc-replicated")), &["h1/tc"]).unwrap();
    let h1 = addr("h1/tc");
    for i in 0..N {
        let out = c
            .call_value(&h1, SET, &member("eth0"), &SetRequest { up: true })
            .await
            .unwrap();
        assert!(matches!(out, Outcome::Value(_)), "call {i}: {out:?}");
    }
    assert_eq!(count(&ran.set), N, "200 executions on the first instance");
    // Every `complete` queryable on the key: the first instance's alone.
    let all = raw_get(
        &tool,
        "zk2/h1/tc/tc.v1/@op/interfaces/eth0/set",
        br#"{"up": true}"#,
        QueryTarget::All,
        ConsolidationMode::None,
    )
    .await;
    assert_eq!((values(&all), refusal_codes(&all).len()), (1, 0));
}

/// An instance of `tc-replicated` at `address`: the replicated
/// `diagnostics` and `listing`, and, when `exclusive`, `set` too, else `set`
/// listed unavailable (`config`).
async fn replicated_tc(
    s: &zenoh::Session,
    address: &str,
    exclusive: bool,
) -> (Service, Vec<OperationServer>, Arc<Seen>) {
    let seen = Arc::new(Seen::default());
    let mut b = ServiceBuilder::new(s, config(address));
    b.implement(Implementation::new(contract("tc-replicated")))
        .unwrap();
    let mut servers = Vec::new();
    if exclusive {
        let sn = Arc::clone(&seen);
        servers.push(
            b.serve_value(&tc(), SET, None, move |_call: Call, req: SetRequest| {
                sn.set.fetch_add(1, Ordering::SeqCst);
                async move { Ok(SetResponse { up: req.up }) }
            })
            .await
            .unwrap(),
        );
    } else {
        b.unavailable(&tc(), SET, Cause::Config, Some("a replica"))
            .unwrap();
    }
    let sn = Arc::clone(&seen);
    servers.push(
        b.serve_value(&tc(), DIAGNOSTICS, None, move |_call: Call, _req: Value| {
            sn.diagnostics.fetch_add(1, Ordering::SeqCst);
            async move { Ok(json!({"ok": true})) }
        })
        .await
        .unwrap(),
    );
    let sn = Arc::clone(&seen);
    servers.push(
        b.serve(&tc(), LISTING, None, move |call: Call| {
            sn.listing.fetch_add(1, Ordering::SeqCst);
            async move {
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

/// §4: with retries configured, an idempotent operation whose server is
/// frozen is called again after each timeout and ends silent; one that
/// answers `busy` is called once, since an envelope is an answer; a
/// non-idempotent one is called once. Without retries configured, the
/// idempotent one is called once: retrying is opt-in (O-7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_retries() {
    let (_r1, ep) = router(None).await;
    let (frozen_s, busy_s, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let frozen = Behaviour {
        frozen_set: true,
        frozen_diagnostics: true,
        ..Behaviour::default()
    };
    let busy = Behaviour {
        busy_diagnostics: true,
        ..Behaviour::default()
    };
    let (_f, _fs, f) = tc_service(&frozen_s, "h1/tc", frozen).await;
    let (_b, _bs, b) = tc_service(&busy_s, "h2/tc", busy).await;
    wait_present(&tool, "*/tc", 2).await;
    let timeout = Duration::from_millis(300);
    let retrying = caller(&tool).with_timeout(timeout).with_retries(2);
    let (h1, h2) = (addr("h1/tc"), addr("h2/tc"));
    let diagnostics = |c: Client, at: Addr| async move {
        c.call_value(&at, DIAGNOSTICS, &Bindings::new(), &json!({}))
            .await
            .unwrap()
    };

    // Idempotent and frozen: three attempts, then silence.
    let out = diagnostics(retrying.clone(), h1.clone()).await;
    let silence = out.silence().expect("no answer");
    assert_eq!(silence.attempts, 3);
    assert_eq!(silence.presence, Attribution::Present);
    assert_eq!(count(&f.diagnostics), 3, "called again after each timeout");

    // Idempotent and `busy`: an answer, called once.
    let out = diagnostics(retrying.clone(), h2).await;
    assert_eq!(out.refusal().map(|e| e.code.as_str()), Some("busy"));
    assert_eq!(count(&b.diagnostics), 1);

    // Not idempotent: called once, whatever the client allows.
    let out = retrying
        .call_value(&h1, SET, &member("eth0"), &SetRequest { up: true })
        .await
        .unwrap();
    assert_eq!(out.silence().expect("no answer").attempts, 1);

    // No retries configured: called once.
    let once = caller(&tool).with_timeout(timeout);
    let out = diagnostics(once, h1).await;
    assert_eq!(out.silence().expect("no answer").attempts, 1);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(count(&f.set), 1, "never retried");
    assert_eq!(count(&f.diagnostics), 4, "opt-in: no retry without one");
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
    assert_eq!(count(&seen.set), 0, "the router refused it");

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

/// §6: two servers each reply 5 values and a summary to one call: with
/// consolidation `None`, 10 values and 2 summaries, where `Latest` or
/// `Auto` lose values. A server whose handler returns before its summary
/// leaves its 5 values, then `internal`, and a replier reported possibly
/// partial.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_many_replies() {
    let (_r1, ep) = router(None).await;
    let (s1, s2, s3, tool) = (
        client(&ep).await,
        client(&ep).await,
        client(&ep).await,
        client(&ep).await,
    );
    let (_a, _as, a) = tc_service(&s1, "h1/tc", Behaviour::default()).await;
    let (_b, _bs, b) = tc_service(&s2, "h2/tc", Behaviour::default()).await;
    let cut = Behaviour {
        cut_listing: true,
        ..Behaviour::default()
    };
    let (_c, _cs, _) = tc_service(&s3, "h3/tc", cut).await;
    wait_present(&tool, "*/tc", 3).await;

    // 1. Two complete repliers.
    let fleet = Fleet::new(&tool, tc_contract(), &["h1/tc", "h2/tc"]).unwrap();
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
            summary.value::<Value>().unwrap(),
            json!({"partial": false, "scanned": 5})
        );
    }
    assert!(replies.partial().is_empty());
    assert_eq!((count(&a.listing), count(&b.listing)), (1, 1));
    // The reason `None` is required: 15 values and 2 summaries reach it.
    let all = |mode| {
        raw_get(
            &tool,
            "zk2/*/tc/tc.v1/@op/listing",
            b"{}",
            QueryTarget::All,
            mode,
        )
    };
    assert_eq!(values(&all(ConsolidationMode::None).await), 17);
    for mode in [ConsolidationMode::Latest, ConsolidationMode::Auto] {
        let kept = values(&all(mode).await);
        assert!(kept < 10, "{mode:?} kept {kept} of 17 replies");
    }

    // 2. A replier cut off before its summary.
    let replies = caller(&tool)
        .call_many(&addr("h3/tc"), LISTING, &Bindings::new(), &b"{}"[..])
        .await
        .unwrap();
    assert_eq!(replies.repliers.len(), 1);
    assert_eq!(replies.repliers[0].values.len(), 5, "the 5 values");
    assert!(replies.repliers[0].summaries.is_empty());
    let codes: Vec<&str> = replies.refusals.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, ["internal"], "then internal, never silence");
    assert_eq!(replies.partial().len(), 1, "possibly partial");
}

/// §6 step 3: a replicated operation's two replicas behind two routers,
/// each replying 5 values and its summary to one concrete call: 10 values
/// and 2 summaries on one key, a replier the caller reports possibly
/// partial, since two instances on one key cannot be told apart (O-3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_3_replicas_on_one_key() {
    let (_r1, ep1) = router(None).await;
    let (_r2, ep2) = router(Some(&ep1)).await;
    let (sa, sb, tool) = (client(&ep1).await, client(&ep2).await, client(&ep1).await);
    let (_a, _as, a) = replicated_tc(&sa, "h1/tc", false).await;
    let (_b, _bs, b) = replicated_tc(&sb, "h1/tc", false).await;
    wait_present(&tool, "h1/tc", 2).await;
    let c = Client::new(&tool, Arc::new(contract("tc-replicated")), &["h1/tc"]).unwrap();
    let h1 = addr("h1/tc");
    // Until the route to the replica behind R2 has crossed.
    let deadline = Instant::now() + common::SETTLE;
    let replies = loop {
        let r = c
            .call_many(&h1, LISTING, &Bindings::new(), &b"{}"[..])
            .await
            .unwrap();
        if r.repliers.first().is_some_and(|x| x.summaries.len() == 2) {
            break r;
        }
        assert!(Instant::now() < deadline, "never: both replicas answer");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(replies.repliers.len(), 1, "one key");
    let r = &replies.repliers[0];
    assert_eq!(r.key, "zk2/h1/tc/tc.v1/@op/listing");
    assert_eq!((r.values.len(), r.summaries.len()), (10, 2));
    assert_eq!(
        replies.partial().len(),
        1,
        "two summaries: possibly partial"
    );
    assert!(
        count(&a.listing) >= 1 && count(&b.listing) >= 1,
        "one per router"
    );
}

/// §7: `{actor, request_id}` is metadata with both; `{"actor": "a",
/// "extra": 1}` an actor and no request id; `{"actor": 3}`, `[]` and bytes
/// that are not JSON are no metadata. Every call is served; nothing
/// authenticates any of them (O-6).
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
    // The other attachments, raw.
    for attachment in [
        &br#"{"actor": "a", "extra": 1}"#[..],
        br#"{"actor": 3}"#,
        br#"{"actor": null}"#,
        b"[]",
        b"not json",
    ] {
        let replies = raw_get_with(
            &ws,
            "zk2/h1/tc/tc.v1/@op/interfaces/eth0/set",
            br#"{"up": false}"#,
            QueryTarget::BestMatching,
            ConsolidationMode::None,
            Some(attachment),
        )
        .await;
        assert_eq!(values(&replies), 1, "served all the same");
    }
    let actor_only = CallMetadata {
        actor: Some("a".into()),
        request_id: None,
    };
    assert_eq!(
        *seen.metadata.lock().unwrap(),
        [
            Some(claim.clone()),
            Some(claim),
            None,
            Some(actor_only),
            None,
            None,
            None,
            None
        ],
        "two sessions claim one actor, and the server cannot tell them apart"
    );
}

/// §8: two instances holding one interface token past the grace period are
/// a finding; a re-mint, whose overlap is shorter, is not; nor is a
/// standby holding its instance token only. Nor are two instances of an
/// interface whose only resources are replicated operations, nor a replica
/// beside the instance serving the exclusive ones: the check decides those
/// from the contract and the holders' descriptors (O-12). The runtime
/// fences nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_split_brain_diagnosis() {
    const GRACE: Duration = Duration::from_millis(600);
    let alive = "zk2/*/*/@zk/alive/**";
    let (_r1, ep) = router(None).await;
    let tool = client(&ep).await;
    let mut sessions = Vec::new();
    for _ in 0..7 {
        sessions.push(client(&ep).await);
    }
    let (mut a, _as, _a_seen) = tc_service(&sessions[0], "h1/tc", Behaviour::default()).await;

    // 1. A second instance of h1/tc, both holding tc.v1's token.
    let (b, _bs, _b_seen) = tc_service(&sessions[1], "h1/tc", Behaviour::default()).await;
    wait_present(&tool, "h1/tc", 2).await;
    let found = ownership::split_brain(&tool, alive, GRACE, T)
        .await
        .unwrap();
    assert_eq!(found.findings.len(), 1, "{found:?}");
    assert!(found.undecided.is_empty(), "{found:?}");
    assert_eq!(found.findings[0].service, addr("h1/tc"));
    assert_eq!(found.findings[0].iface, tc());
    let mut both = vec![a.instance().clone(), b.instance().clone()];
    both.sort();
    assert_eq!(found.findings[0].instances, both);
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
    let standby = ServiceBuilder::new(&sessions[2], config("h1/tc"))
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

    // 4. Two instances of an interface whose only resource is replicated.
    let ping: IfaceId = "ping.v1".parse().unwrap();
    let mut pingers = Vec::new();
    for s in &sessions[3..5] {
        let mut p = ServiceBuilder::new(s, config("h2/pinger"));
        p.implement(imp("ping.v1")).unwrap();
        let server = p
            .serve(
                &ping,
                "@op/ping",
                Some(&Bindings::new()),
                |call: Call| async move {
                    call.reply("pong").await?;
                    Ok(())
                },
            )
            .await
            .unwrap();
        pingers.push((p.start().await.unwrap(), server));
    }
    // 5. A replica beside the instance serving the exclusive operation.
    let (_full, _fs, _) = replicated_tc(&sessions[5], "h3/tc", true).await;
    let (_replica, _rs, _) = replicated_tc(&sessions[6], "h3/tc", false).await;
    eventually("the replicas are present", || async {
        zenkey::presence::tokens(&tool, "zk2/h2/pinger/@zk/alive/**", T)
            .await
            .unwrap()
            .len()
            == 2
            && zenkey::presence::tokens(&tool, "zk2/h3/tc/@zk/alive/**", T)
                .await
                .unwrap()
                .len()
                == 2
    })
    .await;
    let token_only = ownership::candidates(
        &zenkey::presence::tokens(&tool, alive, T).await.unwrap(),
        &zenkey::presence::tokens(&tool, alive, T).await.unwrap(),
    );
    assert_eq!(token_only.len(), 2, "the tokens alone would report both");
    let found = ownership::split_brain(&tool, alive, GRACE, T)
        .await
        .unwrap();
    assert!(
        found.is_clear(),
        "replicated serving is no finding: {found:?}"
    );
}

/// The active instance answers `unavailable` for an optional operation it
/// does not serve, and the cause follows the capabilities and the
/// `unavailable` list as they change (O3, §3.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unavailable_follows_exposure() {
    let (_r1, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let mut b = ServiceBuilder::new(&owner, config("h1/tc").capability("xdp"));
    b.implement(imp("tc.v1")).unwrap();
    let ran = Arc::new(AtomicU64::new(0));
    let mut servers = Vec::new();
    for op in [SET, DIAGNOSTICS, LISTING, OFFLOAD, RESET] {
        let r = Arc::clone(&ran);
        servers.push(
            b.serve(&tc(), op, None, move |call: Call| {
                let r = Arc::clone(&r);
                async move {
                    // Only `offload` and `reset` reply; the others return
                    // without. `reset` names no member, so it cannot.
                    if op == OFFLOAD {
                        r.fetch_add(1, Ordering::SeqCst);
                    }
                    if op == OFFLOAD || op == RESET {
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
        c.call_value(&h1, OFFLOAD, &Bindings::new(), &json!({"up": true}))
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
    svc.set_unavailable(&tc(), OFFLOAD, Some((Cause::Config, Some("disabled"))))
        .unwrap();
    let env = offload().await.refusal().cloned().expect("refused");
    assert_eq!(
        (env.cause.as_deref(), env.message.as_str()),
        (Some("config"), "disabled")
    );
    assert_eq!(ran.load(Ordering::SeqCst), 1, "only the first call ran");

    // No reply from the handler: `internal`, not silence (O-5).
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
    // A fan-out over a template whose handler names no member: no key to
    // reply on, so `internal` (§5.1, "Over a template").
    let reset = Fleet::new(&tool, tc_contract(), &["h1/tc"])
        .unwrap()
        .call_value(RESET, &Bindings::new(), &json!({}))
        .await
        .unwrap();
    assert!(reset.repliers.is_empty());
    assert_eq!(reset.refusals.len(), 1);
    assert_eq!(reset.refusals[0].code, "internal");
    assert!(
        reset.refusals[0].message.contains("names no member"),
        "{}",
        reset.refusals[0].message
    );
}
