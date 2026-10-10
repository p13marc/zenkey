//! A tool that was never compiled against an interface (#612, FJ2): it
//! discovers a service from presence, retrieves and verifies its bundle,
//! builds the contract from the bundle alone, then subscribes and calls.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{T, client, config, eventually, imp, router};
use zenkey::consumer::{Consumer, Delivery};
use zenkey::model::canonical::Fingerprint;
use zenkey::model::grammar::{IfaceId, ZkKey};
use zenkey::model::template::Bindings;
use zenkey::presence;
use zenkey::retrieval::{Retrieved, fetch_bundle};
use zenkey::{Client, Fleet, Implementation, Outcome, ServiceBuilder};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_discovers_retrieves_subscribes_and_calls() {
    let (_r1, ep) = router(None).await;
    let owner = client(&ep).await;
    let tool = client(&ep).await;

    // The owner: probe.v1, a stream and an echoing operation.
    let probe: IfaceId = "probe.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("lab/probe"));
    b.implement(imp("probe.v1")).unwrap();
    let none = Bindings::new();
    let s = b.declare_writer(&probe, "stream/s", &none).await.unwrap();
    let _op = b
        .serve(&probe, "@op/op", Some(&none), |call| async move {
            let body = call
                .payload()
                .map(|p| p.to_bytes().into_owned())
                .unwrap_or_default();
            call.reply(body)
                .await
                .map_err(|e| zenkey::OpError::internal(e.to_string()))
        })
        .await
        .unwrap();
    for res in ["@stream/xs", "state/st", "@state/xst", "events/ev"] {
        b.expose(&probe, res).unwrap();
    }
    let _svc = b.start().await.unwrap();

    // Discovery: presence, the descriptor, the bundle, the contract.
    let mut found = None;
    let deadline = tokio::time::Instant::now() + common::SETTLE;
    while found.is_none() {
        let read = presence::liveliness_read(&tool, "zk2/lab/*/@zk/**", T)
            .await
            .unwrap();
        for k in read
            .keys
            .iter()
            .filter_map(|k| zenkey::model::grammar::parse(k).ok())
        {
            if let ZkKey::Instance { addr, instance } = k {
                let d = presence::descriptor(&tool, &addr, &instance, T)
                    .await
                    .unwrap()
                    .into_descriptor()
                    .unwrap();
                let e = d.interfaces.iter().find(|e| e.iface == "probe.v1").unwrap();
                let fp = Fingerprint::parse(&e.contract).unwrap();
                if let Retrieved::Bundle(_, bytes) =
                    fetch_bundle(&tool, &probe, &fp, T).await.unwrap()
                {
                    found = Some((addr, Implementation::from_bundle(&bytes).unwrap()));
                }
            }
        }
        assert!(tokio::time::Instant::now() < deadline, "never discovered");
    }
    let (addr, imp) = found.unwrap();
    assert_eq!(
        imp.contract().iface,
        probe,
        "the contract, from the bundle alone"
    );
    let contract = imp.shared_contract();

    // Subscribe as a tool.
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let g = Arc::clone(&got);
    let consumer = Consumer::for_tool(
        &tool,
        Arc::clone(&contract),
        &["lab/*"],
        &Default::default(),
    )
    .unwrap();
    let _sub = consumer
        .subscribe("stream/s", move |d: Delivery| {
            g.lock().unwrap().push(d.provider.to_string())
        })
        .await
        .unwrap();
    eventually("a sample reached the tool", || async {
        s.put("x").await.unwrap();
        !got.lock().unwrap().is_empty()
    })
    .await;
    assert!(got.lock().unwrap().iter().all(|p| p == "lab/probe"));

    // Call as a tool.
    let c = Client::new(&tool, contract, &["lab/probe"]).unwrap();
    match c.call(&addr, "@op/op", &none, "ping").await.unwrap() {
        Outcome::Value(a) => assert_eq!(&*a.payload().to_bytes(), b"ping"),
        other => panic!("{other:?}"),
    }

    // A tool has no service: `self.*` parameters are refused.
    let params = [("x".to_owned(), "self.system".to_owned())].into();
    assert!(Consumer::for_tool(&tool, imp.shared_contract(), &["lab/*"], &params).is_err());

    // Completeness (§8.1): a GET that ran to its timeout is flagged.
    let quick = presence::liveliness_read(&tool, "zk2/lab/*/@zk/**", Duration::from_secs(5))
        .await
        .unwrap();
    assert!(quick.complete, "{quick:?}");

    // A fleet's presence carries the reads' completeness too (#671).
    let present = Fleet::new(&tool, imp.shared_contract(), &["lab/*"])
        .unwrap()
        .presence(T)
        .await
        .unwrap();
    assert_eq!(present.providers, std::slice::from_ref(&addr));
    assert!(present.complete && present.errors.is_empty(), "{present:?}");
}

/// A tool's subscription counts what it drops apart (#671; the tooling
/// guide's O6): R6's discards (a wildcard key) and samples whose concrete
/// key resolves to no member (`tracks/X` is not a canonical slug, §1.4).
/// Neither is delivered; only `tracks/t1` is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_subscription_counts_unresolved_samples_apart() {
    let (_r1, ep) = router(None).await;
    let (writer, tool) = (client(&ep).await, client(&ep).await);
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let g = Arc::clone(&got);
    let consumer = Consumer::for_tool(
        &tool,
        imp("nav.v2").shared_contract(),
        &["p1/nav"],
        &Default::default(),
    )
    .unwrap();
    let sub = consumer
        .subscribe("state/tracks/{track}", move |d: Delivery| {
            g.lock().unwrap().push(d.sample.key_expr().to_string())
        })
        .await
        .unwrap();
    let probe = writer
        .declare_publisher("zk2/p1/nav/nav.v2/state/tracks/t1")
        .await
        .unwrap();
    eventually("the tool's subscriber is matched", || async {
        probe.matching_status().await.unwrap().matching()
    })
    .await;
    for key in [
        "zk2/p1/nav/nav.v2/state/tracks/t1",
        "zk2/p1/nav/nav.v2/state/tracks/X",
        "zk2/p1/nav/nav.v2/state/tracks/*",
    ] {
        writer.put(key, "v").await.unwrap();
    }
    eventually("each sample is accounted for", || async {
        sub.discarded() == 1 && sub.unresolved() == 1 && got.lock().unwrap().len() == 1
    })
    .await;
    assert_eq!(*got.lock().unwrap(), ["zk2/p1/nav/nav.v2/state/tracks/t1"]);
}

/// A tool's state GET says whether it ran to its end (core §2.7, 0.24;
/// #735): an owner that answers is a complete reading, and a queryable that
/// holds the query past the timeout leaves it incomplete — the timeout
/// arrives as an error reply (Appendix B), kept out of the answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_state_get_says_whether_it_ran_to_its_end() {
    let (_r1, ep) = router(None).await;
    let (owner, mute, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let probe: IfaceId = "probe.v1".parse().unwrap();
    let mut b = ServiceBuilder::new(&owner, config("lab/probe"));
    b.implement(imp("probe.v1")).unwrap();
    let none = Bindings::new();
    let st = b
        .declare_state_writer(&probe, "state/st", &none)
        .await
        .unwrap();
    st.put("v").await.unwrap();
    for res in [
        "stream/s",
        "@stream/xs",
        "@state/xst",
        "events/ev",
        "@op/op",
    ] {
        b.expose(&probe, res).unwrap();
    }
    let _svc = b.start().await.unwrap();
    let consumer = Consumer::for_tool(
        &tool,
        imp("probe.v1").shared_contract(),
        &["lab/probe"],
        &Default::default(),
    )
    .unwrap();
    eventually("the owner answers, to the end", || async {
        let a = consumer.get_answer("state/st", None, T).await.unwrap();
        a.complete && a.current.len() == 1
    })
    .await;
    // A queryable that never replies: each query is held, then dropped
    // only after the GET's timeout.
    let held: Arc<Mutex<Vec<zenoh::query::Query>>> = Arc::default();
    let h = Arc::clone(&held);
    let _q = mute
        .declare_queryable("zk2/lab/mute/probe.v1/state/**")
        .callback(move |q| h.lock().unwrap().push(q))
        .await
        .unwrap();
    let silent = Consumer::for_tool(
        &tool,
        imp("probe.v1").shared_contract(),
        &["lab/mute"],
        &Default::default(),
    )
    .unwrap();
    eventually("a held GET ends at its timeout, incomplete", || async {
        let a = silent
            .get_answer("state/st", None, Duration::from_millis(300))
            .await
            .unwrap();
        !a.complete && a.current.is_empty()
    })
    .await;
    assert!(
        !held.lock().unwrap().is_empty(),
        "the queryable was reached"
    );
}
