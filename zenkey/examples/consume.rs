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
//!
//! **`--timing`** (`timing.v1`, #908) is a reader of one timed stream, as
//! `spec/profiles/timing/scenarios.md`'s C is, for a black-box runner to
//! read (`consume --help` prints its interface):
//!
//! ```text
//! cargo run -p zenkey --example consume -- --timing [--clock-bound-ms <ms>] [--no-watch]
//!     [--param <name>=<value>]... [--for <s>] <endpoint> <system>/<service> <contract.toml> <resource>
//! ```
//!
//! It runs a service, `interop/timing-reader`, whose role `p` binds the
//! provider (R1), its parameters bound by `--param` (R2) and its ε the
//! binding's `clock_bound_ms` (`--clock-bound-ms`; none: nothing is
//! discarded, §2.6). Its sessions are clients of `endpoint` with no HLC
//! (text 0.2, F-265): one that reads, and a watch session of its own where
//! it follows the provider's tokens (§2.5, core §8.1), unless `--no-watch`.
//! It subscribes to `resource` with `Consumer::subscribe_timed`, and prints
//! one JSON line per arrival and per deadline report, each instant in
//! milliseconds since the reader's origin, an instant taken just before it
//! subscribed, on its monotonic clock. A line on standard input `state
//! <system>/<service>` is answered with what it holds of that provider. It
//! runs until standard input closes, or `--for` seconds.

use std::sync::Arc;
use std::time::Duration;

use zenkey::model::contract::load_path;
use zenkey::model::template::Bindings;
use zenkey::state::{Current, StateGet};
use zenkey::{Outcome, ServiceBuilder, ServiceConfig};

/// What `consume --help` prints: the timing reader's interface whole.
pub const HELP: &str = "\
consume: a zk2 consumer to interoperate with (#609).

usage: consume <endpoint> <system>/<service> <contract.toml> <state> <operation>
       consume --timing [--clock-bound-ms <ms>] [--no-watch] [--param <name>=<value>]...
               [--for <s>] <endpoint> <system>/<service> <contract.toml> <resource>

--timing (timing.v1, spec/profiles/timing/v1.md): a reader of one timed stream
  through a role bound to <system>/<service> (R1), as scenarios.md's C.
  --clock-bound-ms <ms>     ε, the binding's clock_bound_ms (§2.6); absent: no bound,
                            nothing discarded, every lifespan unobservable (no_bound)
  --no-watch                no watch session: the owner is never seen, a known member
                            judged from its first arrival, a member ended only forgotten
  --param <name>=<value>    a template parameter bound (R2): a member known (§0)
  --for <s>                 stop after <s> seconds (default: when stdin closes)
  Its sessions are clients of <endpoint>, with no HLC (text 0.2, F-265).

  Standard output, one JSON line each; every *_ms an instant in milliseconds
  since the reader's origin, on its monotonic clock (origin: just before it
  subscribed), exact to the nanosecond:
    {\"timing\": \"subscribed\", \"resource\": <name>, \"since_ms\": <ms>, \"zid\": <zid>,
      \"watch\": <zid>|null, \"clock_bound_ms\": <ms>|null}
    {\"timing\": \"arrival\", \"provider\": <sys/svc>, \"key\": <key>, \"at_ms\": <ms>,
      \"wall_ns\": <ns since the Unix epoch>, \"stamp\": {\"time\": <NTP64>, \"id\": <zid>,
      \"age_ms\": <wall at arrival less the stamp>}|null,
      \"lifespan\": {\"verdict\": <verdict>, \"reason\": <reason>}, \"delivered\": <bool>}
      (delivered false: expired, discarded and no arrival; §2.6)
    {\"timing\": \"missed\", \"provider\", \"key\", \"at_ms\", \"due_ms\", \"by_ms\",
      \"owner\": \"present\"|\"unconfirmed\"|\"absent\"|\"unseen\"}
    {\"timing\": \"met\", \"provider\", \"key\", \"at_ms\"}
    {\"timing\": \"ended\", \"provider\", \"key\", \"at_ms\", \"reason\": \"gone\"|\"forgotten\"}
    {\"timing\": \"beyond_bound\", \"provider\", \"key\", \"at_ms\"}
  A delivered arrival's line is printed from the subscriber's callback, and
  every other from the reports' own task, in the order judged: two lines of
  the two paths may print out of order, never with their instants changed.

  Standard input:
    state <system>/<service>  {\"timing\": \"state\", \"provider\", \"owner\": <state>,
                              \"owner_zids\": [<zid>...], \"expired\": <n>}
";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--timing") {
        return timing::run(&args[1..]).await;
    }
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

/// `--timing`: a reader of one timed stream (`timing.v1`, #908).
mod timing {
    use std::collections::BTreeMap;
    use std::io::BufRead;
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use zenkey::model::contract::load_path;
    use zenkey::model::freshness::StampAge;
    use zenkey::model::grammar::Addr;
    use zenkey::timing::TimingEvent;
    use zenkey::{ServiceBuilder, ServiceConfig};
    use zenoh::time::Timestamp;

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    struct Opts {
        clock_bound_ms: Option<u64>,
        watch: bool,
        params: BTreeMap<String, String>,
        for_s: Option<f64>,
        endpoint: String,
        provider: String,
        file: String,
        resource: String,
    }

    fn parse(args: &[String]) -> Result<Opts, String> {
        let usage = || {
            "usage: consume --timing [--clock-bound-ms <ms>] [--no-watch] \
             [--param <name>=<value>]... [--for <s>] \
             <endpoint> <system>/<service> <contract.toml> <resource>"
                .to_owned()
        };
        let mut o = Opts {
            clock_bound_ms: None,
            watch: true,
            params: BTreeMap::new(),
            for_s: None,
            endpoint: String::new(),
            provider: String::new(),
            file: String::new(),
            resource: String::new(),
        };
        let mut rest = args;
        loop {
            match rest {
                [f, v, tail @ ..] if f == "--clock-bound-ms" => {
                    o.clock_bound_ms = Some(v.parse().map_err(|e| format!("{f} {v:?}: {e}"))?);
                    rest = tail;
                }
                [f, tail @ ..] if f == "--no-watch" => {
                    o.watch = false;
                    rest = tail;
                }
                [f, v, tail @ ..] if f == "--param" => {
                    let (k, val) = v.split_once('=').ok_or_else(usage)?;
                    o.params.insert(k.to_owned(), val.to_owned());
                    rest = tail;
                }
                [f, v, tail @ ..] if f == "--for" => {
                    o.for_s = Some(v.parse().map_err(|e| format!("{f} {v:?}: {e}"))?);
                    rest = tail;
                }
                [f, ..] if f.starts_with("--") => return Err(usage()),
                _ => break,
            }
        }
        let [endpoint, provider, file, resource] = rest else {
            return Err(usage());
        };
        o.endpoint = endpoint.clone();
        o.provider = provider.clone();
        o.file = file.clone();
        o.resource = resource.clone();
        Ok(o)
    }

    fn ms(origin: Instant, at: Instant) -> f64 {
        at.saturating_duration_since(origin).as_nanos() as f64 / 1e6
    }

    fn stamp_json(t: &Timestamp, wall: SystemTime) -> serde_json::Value {
        let age = StampAge::between(t.get_time().to_system_time(), wall);
        serde_json::json!({
            "time": t.get_time().as_u64().to_string(),
            "id": t.get_id().to_string(),
            "age_ms": age.nanos() as f64 / 1e6,
        })
    }

    async fn session(endpoint: &str) -> Result<zenoh::Session, BoxError> {
        let mut z = zenoh::Config::default();
        z.insert_json5("mode", "\"client\"")?;
        z.insert_json5("connect/endpoints", &serde_json::to_string(&[endpoint])?)?;
        z.insert_json5("scouting/multicast/enabled", "false")?;
        zenoh::open(z).await
    }

    pub async fn run(args: &[String]) -> Result<(), BoxError> {
        let o = match parse(args) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        };
        let l = load_path(std::path::Path::new(&o.file));
        let Some(contract) = l.contract else {
            eprintln!("{}:\n{}", o.file, l.report);
            std::process::exit(2);
        };
        let contract = Arc::new(contract);
        let main = session(&o.endpoint).await?;
        let watch = if o.watch {
            Some(session(&o.endpoint).await?)
        } else {
            None
        };
        let mut cfg =
            ServiceConfig::new("interop/timing-reader".parse()?).bind("p", &[&o.provider]);
        if let Some(b) = cfg.bindings.get_mut("p") {
            b.params = o.params.clone();
            b.clock_bound_ms = o.clock_bound_ms;
        }
        let mut b = ServiceBuilder::new(&main, cfg);
        b.require("p", contract.iface.clone(), false);
        let svc = b.start().await?;
        let mut consumer = svc.consumer("p", Arc::clone(&contract))?;
        if let Some(w) = &watch {
            consumer = consumer.with_watch(w.clone());
        }
        let origin = Instant::now();
        let sub = consumer
            .subscribe_timed(
                &o.resource,
                move |d| {
                    let wall = SystemTime::now();
                    let line = serde_json::json!({
                        "timing": "arrival",
                        "provider": d.provider.to_string(),
                        "key": d.sample.key_expr().as_str(),
                        "at_ms": ms(origin, d.arrival),
                        "wall_ns": wall.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().to_string(),
                        "stamp": d.stamp.as_ref().map(|t| stamp_json(t, wall)),
                        "lifespan": {"verdict": d.lifespan.verdict.as_str(), "reason": d.lifespan.reason.as_str()},
                        "delivered": true,
                    });
                    println!("{line}");
                },
                move |ev| {
                    let line = match &ev {
                        TimingEvent::Missed { provider, key, at, due, by, owner } => serde_json::json!({
                            "timing": "missed", "provider": provider.to_string(), "key": key,
                            "at_ms": ms(origin, *at), "due_ms": ms(origin, *due), "by_ms": ms(origin, *by),
                            "owner": owner.as_str(),
                        }),
                        TimingEvent::Met { provider, key, at } | TimingEvent::BeyondBound { provider, key, at } => serde_json::json!({
                            "timing": ev.as_str(), "provider": provider.to_string(), "key": key,
                            "at_ms": ms(origin, *at),
                        }),
                        TimingEvent::Ended { provider, key, at, reason } => serde_json::json!({
                            "timing": "ended", "provider": provider.to_string(), "key": key,
                            "at_ms": ms(origin, *at), "reason": reason.as_str(),
                        }),
                        TimingEvent::Expired { provider, key, arrival, stamp, age } => serde_json::json!({
                            "timing": "arrival", "provider": provider.to_string(), "key": key,
                            "at_ms": ms(origin, *arrival),
                            "wall_ns": null,
                            "stamp": {"time": stamp.get_time().as_u64().to_string(), "id": stamp.get_id().to_string(),
                                      "age_ms": age.nanos() as f64 / 1e6},
                            "lifespan": {"verdict": "expired", "reason": "expired"},
                            "delivered": false,
                        }),
                    };
                    println!("{line}");
                },
            )
            .await?;
        println!(
            "{}",
            serde_json::json!({
                "timing": "subscribed",
                "resource": o.resource,
                "since_ms": ms(origin, sub.since()),
                "zid": main.zid().to_string(),
                "watch": watch.as_ref().map(|w| w.zid().to_string()),
                "clock_bound_ms": o.clock_bound_ms,
            })
        );
        let (tx, rx) = flume::unbounded::<String>();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = o
            .for_s
            .map(|s| tokio::time::Instant::now() + Duration::from_secs_f64(s));
        loop {
            let stop = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                () = stop => break,
                line = rx.recv_async() => match line {
                    Ok(line) => {
                        let words: Vec<&str> = line.split_whitespace().collect();
                        match words.as_slice() {
                            ["state", p] => match p.parse::<Addr>() {
                                Ok(a) => println!("{}", serde_json::json!({
                                    "timing": "state",
                                    "provider": a.to_string(),
                                    "owner": sub.owner_state(&a).as_str(),
                                    "owner_zids": sub.owner_zids(&a).iter().map(|z| format!("{z:x}")).collect::<Vec<_>>(),
                                    "expired": sub.expired(),
                                })),
                                Err(e) => println!("refused {line}: {e}"),
                            },
                            _ => println!("refused {line}: state <system>/<service>"),
                        }
                    }
                    Err(_) => break,
                },
            }
        }
        drop(sub);
        svc.close().await?;
        if let Some(w) = watch {
            w.close().await?;
        }
        main.close().await?;
        Ok(())
    }
}
