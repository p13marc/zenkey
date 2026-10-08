//! Shared fixtures for the scenario suites (`spec/scenarios/`): in-process
//! routers and clients on ephemeral ports, and the test contracts.
//!
//! The scenarios' convention (`spec/scenarios/README.md`): R1 and R2 are
//! routers, linked R2 → R1; owners, consumers and callers are client
//! sessions. Nothing here names a port: a router listens on port 0 and the
//! suite reads back what it bound.

// Each suite includes the whole module and uses the part it needs.
#![allow(dead_code)]

use std::path::Path;
use std::time::Duration;

use zenkey::model::contract::{Contract, load_path};
use zenkey::model::grammar::Addr;
use zenkey::{Implementation, ServiceConfig};

/// The scenarios' reply timeout (`spec/scenarios/README.md`: 1 s unless
/// stated).
pub const T: Duration = Duration::from_secs(1);

/// How long a suite waits before calling a hang a hang. A net, not an
/// assertion: every use returns the moment its event arrives, and these
/// suites run beside every other test binary.
pub const SETTLE: Duration = Duration::from_secs(20);

fn base_config() -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c
}

async fn bound(s: &zenoh::Session) -> String {
    s.info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback tcp listener")
}

/// A router listening on an ephemeral port, connected to `upstream` if
/// given. Returns the router and the endpoint it listens on.
pub async fn router(upstream: Option<&str>) -> (zenoh::Session, String) {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")
        .unwrap();
    if let Some(up) = upstream {
        c.insert_json5("connect/endpoints", &format!("[\"{up}\"]"))
            .unwrap();
    }
    let r = zenoh::open(c).await.expect("router");
    let ep = bound(&r).await;
    (r, ep)
}

/// A client of the router at `endpoint`, in `namespace` if given.
pub async fn client_ns(endpoint: &str, namespace: Option<&str>) -> zenoh::Session {
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\"")).unwrap();
    }
    zenoh::open(c).await.expect("client")
}

pub async fn client(endpoint: &str) -> zenoh::Session {
    client_ns(endpoint, None).await
}

/// A test contract from `tests/contracts/`.
pub fn contract(name: &str) -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contracts")
        .join(format!("{name}.toml"));
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

/// A contract from `examples/zk2/`, by its path without `.toml`.
pub fn example(path: &str) -> Contract {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/zk2")
        .join(format!("{path}.toml"));
    let l = load_path(&full);
    l.contract
        .unwrap_or_else(|| panic!("{path} does not load:\n{}", l.report))
}

pub fn imp(name: &str) -> Implementation {
    Implementation::new(contract(name))
}

pub fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

pub fn config(address: &str) -> ServiceConfig {
    ServiceConfig::new(addr(address))
}

/// Polls `f` until it holds, failing after [`SETTLE`].
pub async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + SETTLE;
    while !f().await {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A router like [`router`], with more configuration inserted, each a
/// (key, JSON5 value) pair: access control, for one.
pub async fn router_with(
    upstream: Option<&str>,
    extra: &[(&str, &str)],
) -> (zenoh::Session, String) {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")
        .unwrap();
    if let Some(up) = upstream {
        c.insert_json5("connect/endpoints", &format!("[\"{up}\"]"))
            .unwrap();
    }
    for (k, v) in extra {
        c.insert_json5(k, v).unwrap();
    }
    let r = zenoh::open(c).await.expect("router");
    let ep = bound(&r).await;
    (r, ep)
}
