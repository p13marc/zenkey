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
use zenkey::{Client, Implementation, Outcome, ServiceBuilder};

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
}
