//! The zk2 suites' fixtures (#612, FJ3), in the style of the runtime's own
//! (`zenkey/tests/common/mod.rs`): in-process routers and clients on
//! ephemeral ports, the example contracts, and a poll that fails after
//! [`super::SETTLE`].
//!
//! The scenarios' convention: routers are routers, and owners, consumers
//! and tools are client sessions of them. Nothing here names a port.

use std::path::Path;
use std::time::Duration;

use zenkey_model::contract::{Contract, load_path};
use zenkey_model::grammar::{Addr, IfaceId};

/// The scenarios' reply timeout (`spec/scenarios/README.md`: 1 s unless
/// stated).
pub const T: Duration = Duration::from_secs(1);

fn base_config() -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .expect("config");
    c.insert_json5("scouting/gossip/enabled", "false")
        .expect("config");
    c
}

/// A router listening on an ephemeral port, connected to `upstream` if
/// given. Returns the router and the endpoint it listens on.
pub async fn router(upstream: Option<&str>) -> (zenoh::Session, String) {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").expect("config");
    c.insert_json5("listen/endpoints", &format!("[\"{}\"]", super::ANY_PORT))
        .expect("config");
    if let Some(up) = upstream {
        c.insert_json5("connect/endpoints", &format!("[\"{up}\"]"))
            .expect("config");
    }
    let r = zenoh::open(c).await.expect("router");
    let ep = super::bound(&r).await;
    (r, ep)
}

/// A client of the router at `endpoint`, in `namespace` if given.
pub async fn client_ns(endpoint: &str, namespace: Option<&str>) -> zenoh::Session {
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").expect("config");
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .expect("config");
    if let Some(ns) = namespace {
        c.insert_json5("namespace", &format!("\"{ns}\""))
            .expect("config");
    }
    zenoh::open(c).await.expect("client")
}

pub async fn client(endpoint: &str) -> zenoh::Session {
    client_ns(endpoint, None).await
}

/// The repository's `examples/zk2/` directory.
pub fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
}

/// The repository's `spec/profiles/` directory, which holds the profiles'
/// standard contracts as `<name>/<name>.v<major>.toml` (`health.v1`, #721).
pub fn profiles() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles")
}

/// A contract from `examples/zk2/`, by its path without `.toml`, or a
/// profile's standard contract from `spec/profiles/` when the path starts
/// with `profiles/` (`profiles/health/health.v1`).
pub fn example(path: &str) -> Contract {
    let full = match path.strip_prefix("profiles/") {
        Some(p) => profiles().join(format!("{p}.toml")),
        None => examples().join(format!("{path}.toml")),
    };
    let l = load_path(&full);
    l.contract
        .unwrap_or_else(|| panic!("{path} does not load:\n{}", l.report))
}

pub fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

pub fn iface(s: &str) -> IfaceId {
    s.parse().expect("an interface")
}

pub fn config(address: &str) -> zenkey::ServiceConfig {
    zenkey::ServiceConfig::new(addr(address))
}

/// Polls `f` until it holds, failing after [`super::SETTLE`].
pub async fn eventually<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + super::SETTLE;
    while !f().await {
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
