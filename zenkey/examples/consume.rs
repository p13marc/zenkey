//! A zk2 consumer to interoperate with (#609's live half): it reads one
//! provider through a role, as a Rust component would, and prints what it
//! found.
//!
//! ```text
//! cargo run -p zenkey --example consume -- <endpoint> <system>/<service> <contract.toml> <state resource> <operation resource>
//! ```
//!
//! It connects as a client to `endpoint`, binds the role `p` to the given
//! provider, waits for its interface token (R5), then prints, one line each:
//!
//! ```text
//! present <system>/<service>
//! state <key> <stamp> <payload, lossy UTF-8>     (or: state deleted <key> / state silent)
//! call value <payload>                            (or: call refused <code> <message> / call malformed / call silent)
//! ```
//!
//! Resources are named `<kind token>/<template>` (`state/status`, `@op/echo`)
//! and must have no template parameters. A raw request is sent as the
//! bytes `ping`. Exit 0 when every line was printed, 2 on usage or setup
//! errors.

use std::sync::Arc;
use std::time::Duration;

use zenkey::model::contract::load_path;
use zenkey::model::template::Bindings;
use zenkey::state::{Current, StateGet};
use zenkey::{Outcome, ServiceBuilder, ServiceConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [endpoint, provider, file, state, op] = args.as_slice() else {
        eprintln!(
            "usage: consume <endpoint> <system>/<service> <contract.toml> <state> <operation>"
        );
        std::process::exit(2);
    };
    let l = load_path(std::path::Path::new(file));
    let Some(contract) = l.contract else {
        eprintln!("{file}:\n{}", l.report);
        std::process::exit(2);
    };
    let contract = Arc::new(contract);

    let mut z = zenoh::Config::default();
    z.insert_json5("mode", "\"client\"")?;
    z.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))?;
    z.insert_json5("scouting/multicast/enabled", "false")?;
    let session = zenoh::open(z).await?;

    let mut b = ServiceBuilder::new(
        &session,
        ServiceConfig::new("interop/consumer".parse()?).bind("p", &[provider]),
    );
    b.require("p", contract.iface.clone(), false);
    let svc = b.start().await?;
    let consumer = svc.consumer("p", Arc::clone(&contract))?;
    let client = svc.client("p", Arc::clone(&contract))?;
    let t = Duration::from_secs(1);

    let Some(addr) = consumer.wait_for_provider(Duration::from_secs(20)).await? else {
        eprintln!("no provider appeared");
        std::process::exit(2);
    };
    println!("present {addr}");

    match consumer.get(state, None, t).await? {
        StateGet::Answered(v) => match &v[0] {
            Current::Value { key, sample } => println!(
                "state {key} {} {}",
                sample
                    .timestamp()
                    .map_or_else(|| "-".to_owned(), ToString::to_string),
                String::from_utf8_lossy(&sample.payload().to_bytes())
            ),
            Current::Deleted { key, .. } => println!("state deleted {key}"),
        },
        StateGet::Silent => println!("state silent"),
    }

    match client.call(&addr, op, &Bindings::new(), "ping").await? {
        Outcome::Value(a) => println!(
            "call value {}",
            String::from_utf8_lossy(&a.payload().to_bytes())
        ),
        Outcome::Refused(e) => println!("call refused {} {}", e.code, e.message),
        Outcome::Malformed(_) => println!("call malformed"),
        Outcome::NoAnswer(_) => println!("call silent"),
    }
    svc.close().await?;
    Ok(())
}
