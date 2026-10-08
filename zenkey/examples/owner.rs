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
//! Every resource of every contract is exposed (§8.1–§8.4), and for a
//! counterpart to read:
//! - a state resource with no template parameters and a `raw` type holds
//!   the value `ok`, stamped (S1), answered on GET (S2);
//! - an operation with no template parameters is served: a `raw` request
//!   and response echo the request; any other types refuse with an `app`
//!   error envelope (O3), since this owner decodes no schema.
//!
//! Capabilities named by `capability:` gates are all held, so gated
//! resources are exposed too.

use std::io::Read;

use zenkey::model::authoring::Kind;
use zenkey::model::contract::{Body, load_path};
use zenkey::model::schema::TypeId;
use zenkey::model::template::Bindings;
use zenkey::{Implementation, OpError, ServiceBuilder, ServiceConfig};

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
    let mut states = Vec::new();
    let mut servers = Vec::new();
    let none = Bindings::new();
    for imp in imps {
        let iface = imp.iface().clone();
        let resources = imp.contract().resources.clone();
        b.implement(imp)?;
        for r in resources {
            let name = zenkey::implementation::resource_name(&r);
            let raw = |t: &TypeId| matches!(t, TypeId::Raw { .. });
            match &r.body {
                Body::Data(d)
                    if r.kind == Kind::State && !r.template.has_params() && raw(&d.type_) =>
                {
                    states.push(b.declare_state_writer(&iface, &name, &none).await?);
                }
                Body::Operation(o) if !r.template.has_params() => {
                    let echo = raw(&o.request) && raw(&o.response);
                    servers.push(
                        b.serve(&iface, &name, Some(&none), move |call| async move {
                            if echo {
                                let body = call
                                    .payload()
                                    .map(|p| p.to_bytes().into_owned())
                                    .unwrap_or_default();
                                call.reply(body)
                                    .await
                                    .map_err(|e| OpError::internal(e.to_string()))
                            } else {
                                Err(OpError::app_bytes(
                                    "this interop owner decodes no schema",
                                    Vec::new(),
                                ))
                            }
                        })
                        .await?,
                    );
                }
                _ => {
                    b.expose(&iface, &name)?;
                }
            }
        }
    }
    let svc = b.start().await?;
    for w in &states {
        w.put("ok").await?;
    }
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
