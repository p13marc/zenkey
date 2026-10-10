//! A zk2 owner to interoperate with (#609's live half, #610): it brings up
//! one service implementing the given contracts and runs until its standard
//! input closes.
//!
//! ```text
//! cargo run -p zenkey --example owner -- [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral] <address> <contract.toml>...
//! ```
//!
//! The address is `<system>/<service>`, or `@hostid.v1/<service>` for a
//! system minted from the machine id (`spec/profiles/hostid/v1.md` §2.3,
//! #719). The system is minted before the session opens (§2.7), and a host
//! with no id stops the owner with nothing declared (§2.6): the error names
//! every path, and the exit is 1. `--hostid-root <dir>` reads the inputs
//! under `<dir>` instead of `/`, its links resolved in it, as a scenario's
//! root (`spec/profiles/hostid/scenarios.md`); `--hostid-ephemeral` is
//! `hostid = { ephemeral = true }`.
//!
//! By default the session is a router listening on an ephemeral loopback
//! port, so a test connects to it as a client. Two lines on standard output
//! say what to connect to and what came up:
//!
//! ```text
//! listening tcp/127.0.0.1:<port>
//! ready zk2/<system>/<service>/@zk/instance/<instance>
//! ```
//!
//! With `--connect <endpoint>`, the session is instead a client of the
//! router at `<endpoint>`, with its HLC on (§4.3), and the first line is
//! `connected <endpoint>`. That is the owner the black-box scenarios watch
//! through a separate router (#660): `state.md §1` tells the owner's stamp
//! from the router's only when the two sessions differ (§4.2, "Observing
//! S1"), and `presence.md §2` sees a refusal only through a router that
//! outlives the owner.
//!
//! Every resource of every contract is exposed (§8.1–§8.4), and for a
//! counterpart to read:
//! - a state resource with no template parameters and a `raw` type holds
//!   the value `ok`, stamped (S1), answered on GET (S2), and put before the
//!   tokens, so a GET made the moment they appear finds it (§8.2, "State
//!   values", F-68);
//! - every operation is served, so everything exposed is callable (§8.2,
//!   "Exposed"; #670): a `raw` request and response echo the request; any
//!   other types refuse with `app` and no detail (O3, §5.2), since this
//!   owner decodes no schema, and an operation with no `error` type has no
//!   detail to send (F-65). An operation with template parameters is served
//!   over the whole template (§5.1): a call whose key binds every parameter
//!   answers for that member, and an echo to a fan-out that leaves one
//!   unbound names no member, so it is refused `internal`, as §5.1 says of
//!   a server that cannot name one.
//!
//! Capabilities named by `capability:` gates are all held, so gated
//! resources are exposed too.

use std::future::Future;
use std::io::Read;
use std::path::PathBuf;

use zenkey::hostid::{HostIdMinter, HostIdSource};
use zenkey::model::authoring::Kind;
use zenkey::model::contract::{Body, load_path};
use zenkey::model::schema::TypeId;
use zenkey::model::template::Bindings;
use zenkey::{Address, Implementation, OpError, ServiceBuilder, ServiceConfig};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// The command line.
pub struct Options {
    /// `--connect <endpoint>`: a client of that router, not a router.
    pub connect: Option<String>,
    /// `--hostid-root <dir>`: the directory standing in for `/` when the
    /// address is minted (hostid.v1); `/` otherwise.
    pub hostid_root: Option<PathBuf>,
    /// `--hostid-ephemeral`: `hostid.ephemeral` (hostid.v1 §2.6).
    pub hostid_ephemeral: bool,
    pub address: String,
    pub contracts: Vec<PathBuf>,
}

impl Options {
    /// Reads `[--connect <endpoint>] [--hostid-root <dir>]
    /// [--hostid-ephemeral] <address> <contract.toml>...`, in any order
    /// before the address.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let usage = || {
            "usage: owner [--connect <endpoint>] [--hostid-root <dir>] [--hostid-ephemeral] \
             <system>/<service>|@hostid.v1/<service> <contract.toml>..."
                .to_owned()
        };
        let (mut connect, mut hostid_root, mut hostid_ephemeral) = (None, None, false);
        let mut rest = args;
        loop {
            match rest {
                [flag, value, tail @ ..] if flag == "--connect" => {
                    connect = Some(value.clone());
                    rest = tail;
                }
                [flag, value, tail @ ..] if flag == "--hostid-root" => {
                    hostid_root = Some(PathBuf::from(value));
                    rest = tail;
                }
                [flag, tail @ ..] if flag == "--hostid-ephemeral" => {
                    hostid_ephemeral = true;
                    rest = tail;
                }
                [flag, ..] if flag.starts_with("--") => return Err(usage()),
                _ => break,
            }
        }
        let [address, files @ ..] = rest else {
            return Err(usage());
        };
        Ok(Self {
            connect,
            hostid_root,
            hostid_ephemeral,
            address: address.clone(),
            contracts: files.iter().map(PathBuf::from).collect(),
        })
    }
}

/// Brings the owner up, says each output line through `say`, and runs until
/// `until` resolves.
pub async fn run(
    opts: Options,
    say: impl Fn(String),
    until: impl Future<Output = ()>,
) -> Result<(), BoxError> {
    let mut address: Address = opts.address.parse()?;
    if let Address::HostId { ephemeral, .. } = &mut address {
        *ephemeral = opts.hostid_ephemeral;
    }
    let mut config = ServiceConfig::at(address);
    let mut imps = Vec::new();
    for f in &opts.contracts {
        let l = load_path(f);
        let Some(c) = l.contract else {
            return Err(format!("{}:\n{}", f.display(), l.report).into());
        };
        for r in &c.resources {
            for cap in r.gate.iter().filter_map(|g| g.strip_prefix("capability:")) {
                config = config.capability(cap);
            }
        }
        imps.push(Implementation::new(c));
    }

    // hostid.v1 §2.7: the system is minted before the session opens, and a
    // host without an id stops here, with nothing declared (§2.6).
    let under_root;
    let minter = match &opts.hostid_root {
        Some(root) => {
            under_root = HostIdMinter::new(HostIdSource::at(root));
            &under_root
        }
        None => HostIdMinter::global(),
    };
    config.resolve_with(minter)?;

    let mut z = zenoh::Config::default();
    z.insert_json5("scouting/multicast/enabled", "false")?;
    let session = match &opts.connect {
        None => {
            z.insert_json5("mode", "\"router\"")?;
            z.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")?;
            let session = zenoh::open(z).await?;
            let endpoint = session
                .info()
                .locators()
                .await
                .into_iter()
                .map(|l| l.to_string())
                .find(|l| l.starts_with("tcp/127.0.0.1:"))
                .ok_or("no loopback listener")?;
            say(format!("listening {endpoint}"));
            session
        }
        Some(endpoint) => {
            z.insert_json5("mode", "\"client\"")?;
            z.insert_json5("connect/endpoints", &serde_json::to_string(&[endpoint])?)?;
            // §4.3: a session that serves state enables its HLC.
            z.insert_json5("timestamping/enabled", "true")?;
            let session = zenoh::open(z).await?;
            say(format!("connected {endpoint}"));
            session
        }
    };

    let mut b = ServiceBuilder::with_hostid(&session, config, minter);
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
                Body::Operation(o) => {
                    let echo = raw(&o.request) && raw(&o.response);
                    // Over the whole template when it has parameters (§5.1),
                    // else on the operation's one key.
                    let values = (!r.template.has_params()).then_some(&none);
                    servers.push(
                        b.serve(&iface, &name, values, move |call| async move {
                            if !echo {
                                // §5.2: `app` from any operation, and no detail
                                // where there is no `error` type to carry one.
                                return Err(OpError::app_without_detail(
                                    "this interop owner decodes no schema",
                                ));
                            }
                            let body = call
                                .payload()
                                .map(|p| p.to_bytes().into_owned())
                                .unwrap_or_default();
                            call.reply(body)
                                .await
                                .map_err(|e| OpError::internal(e.to_string()))
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
    // §8.2 "State values" (F-68): the value is put before the tokens, so a
    // GET made the moment they appear finds it.
    for w in &states {
        w.put("ok").await?;
    }
    let svc = b.start().await?;
    say(format!("ready {}", svc.instance_key()?));

    until.await;
    svc.close().await?;
    drop(servers);
    session.close().await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    // The runtime's logs go to stderr (warnings unless `RUST_LOG` says
    // otherwise), so a runner driving this owner as a black box sees what
    // the runtime says, such as hostid.v1's ephemeral warning (§2.6, 0.3).
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match Options::parse(&args) {
        Ok(o) => o,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    // Run until standard input closes.
    let stdin_closed = async {
        let _ = tokio::task::spawn_blocking(|| {
            let mut sink = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut sink);
        })
        .await;
    };
    if let Err(e) = run(opts, |line| println!("{line}"), stdin_closed).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
