//! A zk2 owner to interoperate with (#609's live half, #610): it brings up
//! one service implementing the given contracts and runs until its standard
//! input closes.
//!
//! ```text
//! cargo run -p zenkey --example owner -- <system>/<service> <contract.toml>...
//! ```
//!
//! The session is a router listening on an ephemeral loopback port, so a
//! test connects to it as a client. Two lines on standard output say what
//! to connect to and what came up:
//!
//! ```text
//! listening tcp/127.0.0.1:<port>
//! ready zk2/<system>/<service>/@zk/instance/<instance>
//! ```
//!
//! Every resource of every contract is exposed, and nothing is published:
//! presence, the descriptor and the bundles are what this owner serves
//! (spec §8.1–§8.4). Capabilities named by `capability:` gates are all
//! held, so gated resources are exposed too.

use std::io::Read;

use zenkey::model::contract::load_path;
use zenkey::{Implementation, ServiceBuilder, ServiceConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [address, files @ ..] = args.as_slice() else {
        eprintln!("usage: owner <system>/<service> <contract.toml>...");
        std::process::exit(2);
    };
    let mut config = ServiceConfig::new(address.parse()?);
    let mut imps = Vec::new();
    for f in files {
        let l = load_path(std::path::Path::new(f));
        let Some(c) = l.contract else {
            eprintln!("{f}:\n{}", l.report);
            std::process::exit(2);
        };
        for r in &c.resources {
            for cap in r.gate.iter().filter_map(|g| g.strip_prefix("capability:")) {
                config = config.capability(cap);
            }
        }
        imps.push(Implementation::new(c));
    }

    let mut z = zenoh::Config::default();
    z.insert_json5("mode", "\"router\"")?;
    z.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")?;
    z.insert_json5("scouting/multicast/enabled", "false")?;
    let session = zenoh::open(z).await?;
    let endpoint = session
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .ok_or("no loopback listener")?;
    println!("listening {endpoint}");

    let mut b = ServiceBuilder::new(&session, config);
    for imp in imps {
        let iface = imp.iface().clone();
        let names: Vec<String> = imp
            .contract()
            .resources
            .iter()
            .map(zenkey::implementation::resource_name)
            .collect();
        b.implement(imp)?;
        for n in names {
            b.expose(&iface, &n)?;
        }
    }
    let svc = b.start().await?;
    println!("ready {}", svc.instance_key()?);

    // Run until standard input closes.
    tokio::task::spawn_blocking(|| {
        let mut sink = Vec::new();
        let _ = std::io::stdin().read_to_end(&mut sink);
    })
    .await?;
    svc.close().await?;
    Ok(())
}
