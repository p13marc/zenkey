//! The zk2 spray, smoked (#614, FL5): the demo traffic `just spray` and
//! `just gui-demo` run is a deployment a tool can read, and its deliberately
//! non-conforming traffic sits where the doctor sees it.
//!
//! In process, with no port named: the example is included by path and
//! brought up as a router on an ephemeral port, and a tool's two sessions
//! read it as `zenctl service list` and `zenctl doctor --deep
//! --clocks-synced` read it. `just smoke-spray` runs those two binaries
//! against the example binary itself.

use std::path::PathBuf;
use std::time::Duration;

use zenkey_fleet::report::CheckId;
use zenkey_fleet::{BundleStore, DoctorBus, DoctorSpec, PresenceScope};

/// The example, whole. Its `main` and the flags only `main` reads are dead
/// here.
#[allow(dead_code)]
#[path = "../examples/spray.rs"]
mod spray;

const NAMESPACE: &str = "spray";

/// How long a presence listing may take to show every address.
const SETTLE: Duration = Duration::from_secs(20);

async fn client(endpoint: &str, namespace: Option<&str>) -> zenoh::Session {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .expect("config");
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .expect("config");
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\""))
            .expect("config");
    }
    zenoh::open(c).await.expect("a client session")
}

/// Every owner and `host-a/sysmon` is listed with its descriptor served,
/// beside the phantom; the doctor finds the phantom's missing instance token
/// (`token-missing`) and the squatter's foreign stamp on `host-a/tc`'s state
/// (`state-stamp-foreign`, asked under `deep`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_spray_is_a_deployment_and_the_doctor_finds_its_non_conformance() {
    let args = spray::Args {
        connect: vec![],
        listen: vec!["tcp/127.0.0.1:0".into()],
        namespace: NAMESPACE.into(),
        hz: 5.0,
        keys: 0,
        seed: 42,
        contracts: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/zk2")),
    };
    let up = spray::Spray::start(&args)
        .await
        .expect("the spray comes up");
    let endpoint = up
        .endpoints
        .iter()
        .find(|e| e.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback listener")
        .clone();
    let tool = client(&endpoint, Some(NAMESPACE)).await;
    let raw = client(&endpoint, None).await;

    let mut want: Vec<String> = up.instances.keys().cloned().collect();
    want.push(spray::PHANTOM.to_owned());
    assert_eq!(want.len(), 10, "nine services and the phantom: {want:?}");
    let deadline = tokio::time::Instant::now() + SETTLE;
    loop {
        let listing =
            zenkey_fleet::service_listing(&tool, &PresenceScope::all(), Duration::from_secs(2))
                .await
                .expect("a presence read");
        let seen: Vec<&str> = listing
            .services
            .iter()
            .map(|s| s.address.as_str())
            .collect();
        if listing.complete && want.iter().all(|a| seen.contains(&a.as_str())) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never listed together: {want:?}; last read {seen:?}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let mut spec = DoctorSpec::new(Duration::from_secs(2));
    spec.deep = true;
    spec.clocks_synced = true;
    let bus = DoctorBus {
        session: tool.clone(),
        raw: raw.clone(),
        namespace: NAMESPACE.into(),
    };
    let report =
        zenkey_fleet::run_doctor(&bus, &BundleStore::new(Duration::from_secs(2)), &spec).await;
    let findings = |id: CheckId| -> Vec<String> {
        report
            .check(id)
            .unwrap_or_else(|| panic!("{id:?} reported"))
            .findings
            .iter()
            .map(|f| f.subject.clone())
            .collect()
    };
    let missing = findings(CheckId::TokenMissing);
    assert!(
        missing
            .iter()
            .any(|s| s.starts_with(&format!("{}@", spray::PHANTOM))),
        "the phantom holds an interface token and no instance token: {missing:?}"
    );
    let (owner, iface, _) = spray::SQUATTED;
    let foreign = findings(CheckId::StateStampForeign);
    assert!(
        foreign.contains(&format!("{owner} {iface}")),
        "the squatter answers {owner}'s {iface} state with another clock's stamp: {foreign:?}"
    );

    let _ = tool.close().await;
    let _ = raw.close().await;
    up.stop().await;
}
