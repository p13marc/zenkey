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
//! call value <record>                             (or: call refused <code> <message> / call malformed / call silent)
//! ```
//!
//! Resources are named `<kind token>/<template>` (`state/status`, `@op/echo`)
//! and must have no template parameters. A raw request is sent as the
//! bytes `ping`. A value's line is its record, lossy UTF-8, after the
//! value form's CH and as-of mark; one the form's parse refuses is `call
//! malformed <refusal>` (core §5.1). Exit 0 when every line was printed, 2
//! on usage or setup errors.
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
//!
//! **`--follow`** (`redundancy.v2` §2.10, §2.17, #987's N2) is a follower
//! of a governed interface, as `spec/profiles/redundancy/scenarios-v2.md`'s
//! K is, for a black-box runner to read (`consume --help` prints its
//! interface):
//!
//! ```text
//! cargo run -p zenkey --example consume -- --follow [--descriptor-delay-ms <ms>] [--for <s>]
//!     <endpoint> <system>/<service> <contract.toml> <resource>
//! ```
//!
//! It runs a service, `interop/follower`, whose role `p` binds the provider
//! (R1), on two client sessions of `endpoint` with no HLC: one that reads,
//! and a watch session of its own where it follows the provider's interface
//! tokens. It follows `resource` with `Consumer::follow_with`, and prints
//! one JSON line per thing the follower tells: its incumbent, each sample
//! accepted, discarded (with the rule's reason), held or superseded, each
//! seed and each reply of a seed's discarded, and each member a complete
//! snapshot of the incumbent's dropped. `--descriptor-delay-ms` delays its
//! handling of each descriptor put it hears (scenarios-v2.md §14's hook);
//! its reads are not delayed. Lines on standard input: `values`, `counts`,
//! `incumbent <system>/<service>`.

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
       consume --follow [--descriptor-delay-ms <ms>] [--for <s>]
               <endpoint> <system>/<service> <contract.toml> <resource>

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
  subscribed), a float exact to the nanosecond below 2^53 ns (about 104 days):
    {\"timing\": \"subscribed\", \"resource\": <name>, \"since_ms\": <ms>, \"zid\": <zid>,
      \"watch\": <zid>|null, \"clock_bound_ms\": <ms>|null}
    {\"timing\": \"arrival\", \"provider\": <sys/svc>, \"key\": <key>, \"at_ms\": <ms>,
      \"wall_ns\": <ns since the Unix epoch>, \"stamp\": {\"time\": <NTP64>, \"id\": <zid>,
      \"age_ms\": <wall at arrival less the stamp>}|null,
      \"lifespan\": {\"verdict\": <verdict>, \"reason\": <reason>}, \"delivered\": <bool>}
      (delivered false: expired, discarded and no arrival; §2.6). wall_ns is the
      wall clock read in the subscriber's callback at the arrival, the one
      age_ms is taken against, for an expired arrival too (text 0.3, F-276)
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

--follow (redundancy.v2 §2.10, §2.17; spec/profiles/redundancy/v2.md): a follower of a
  governed interface's resource through a role bound to <system>/<service> (R1),
  as scenarios-v2.md's K: it accepts a sample only under its incumbent's runs
  (by meta.zid under a redundancy.v1 incumbent), holds an unattributed one for
  one read of the address's descriptors, and for a state seeds again by core S4
  each time its incumbent changes, merging only the incumbent's snapshots. The
  contract's uses must list a major of redundancy.
  --descriptor-delay-ms <ms> each descriptor put it hears handled <ms> after it
                            arrived (scenarios-v2.md §14's hook); its reads are not
  --for <s>                 stop after <s> seconds (default: when stdin closes)
  Its sessions are clients of <endpoint>, with no HLC: one reads, one watches.

  Standard output, one JSON line each, every at_ms since the follower's origin
  (just before it followed), on its monotonic clock; <run> is 4 or 8 lowercase
  hex digits as a descriptor writes it, null for none; <record> lossy UTF-8:
    {\"follow\": \"started\", \"resource\": <name>, \"zid\": <zid>, \"watch\": <zid>}
    {\"follow\": \"incumbent\", \"provider\", \"incumbent\": <id>|null, \"at_ms\"}
    {\"follow\": \"accepted\"|\"discarded\"|\"held\"|\"superseded\", \"provider\", \"key\",
      \"record\", \"delete\": <bool>, \"run\", \"seq\": <n>|null, \"at_ms\",
      \"seeded\": <bool> (accepted), \"reason\": <reason> (discarded)}
    {\"follow\": \"seed_discarded\", \"provider\", \"key\", \"snapshot\": <bool>, \"run\",
      \"reason\", \"at_ms\"}
    {\"follow\": \"omitted\", \"provider\", \"key\", \"at_ms\"}
    {\"follow\": \"seeded\", \"provider\", \"incumbent\": <id>|null, \"complete\": <bool>,
      \"snapshots\": <n>, \"at_ms\"}
    <reason>: no_incumbent, no_run, not_incumbent, unattributed, no_stamp, as
      redundancy.v2's fences.json writes them

  Standard input:
    values                    {\"follow\": \"values\", \"values\": {<key>: {\"record\", \"run\", \"seq\"}}}
    counts                    {\"follow\": \"counts\", \"accepted\", \"discarded\": {<reason>: <n>},
                              \"held\", \"superseded\", \"omitted\", \"reads\", \"seeds\"}
    incumbent <system>/<service>  {\"follow\": \"incumbent?\", \"provider\", \"incumbent\": <id>|null}
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
    if args.first().map(String::as_str) == Some("--follow") {
        return follow::run(&args[1..]).await;
    }
    run(&args, |l| println!("{l}")).await
}

/// The default mode: each line handed to `say` (`main` prints them), so a
/// test runs the example in process.
pub async fn run(
    args: &[String],
    mut say: impl FnMut(String),
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let [endpoint, provider, file, state, op] = args else {
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
    say(format!("present {addr}"));

    say(match consumer.get(state, None, t).await? {
        StateGet::Answered(v) => match &v[0] {
            Current::Value { key, sample } => format!(
                "state {key} {} {}",
                sample
                    .timestamp()
                    .map_or_else(|| "-".to_owned(), ToString::to_string),
                String::from_utf8_lossy(&sample.payload().to_bytes())
            ),
            Current::Deleted { key, .. } => format!("state deleted {key}"),
        },
        StateGet::Silent => "state silent".to_owned(),
    });

    say(call_line(
        &client.call(&addr, op, &Bindings::new(), "ping").await?,
    ));
    svc.close().await?;
    Ok(())
}

/// The call's line. A value prints its record alone, after the value
/// form's CH and as-of mark (core §5.1, "Reading a value"; zk2py round
/// 101's F-479), and one the form's parse refuses, or under another
/// `Encoding`, is malformed, its refusal named (`encoding` for the
/// latter): an answer all the same.
pub fn call_line(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Value(a) => match a.record() {
            Ok(record) => format!("call value {}", String::from_utf8_lossy(&record)),
            Err(refusal) => format!("call malformed {}", refusal.token()),
        },
        Outcome::Refused(e) => format!("call refused {} {}", e.code, e.message),
        Outcome::NoAnswer(_) => "call silent".to_owned(),
    }
}

/// `--timing`: a reader of one timed stream (`timing.v1`, #908).
pub mod timing {
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

    /// One report's line, as `--help` writes it, its instants from
    /// `origin`. An expired arrival's `wall_ns` is the wall clock the
    /// engine read at its arrival, its age's (text 0.3, F-276).
    pub fn event_line(origin: Instant, ev: &TimingEvent) -> serde_json::Value {
        match ev {
            TimingEvent::Missed {
                provider,
                key,
                at,
                due,
                by,
                owner,
            } => serde_json::json!({
                "timing": "missed", "provider": provider.to_string(), "key": key,
                "at_ms": ms(origin, *at), "due_ms": ms(origin, *due), "by_ms": ms(origin, *by),
                "owner": owner.as_str(),
            }),
            TimingEvent::Met { provider, key, at }
            | TimingEvent::BeyondBound { provider, key, at } => serde_json::json!({
                "timing": ev.as_str(), "provider": provider.to_string(), "key": key,
                "at_ms": ms(origin, *at),
            }),
            TimingEvent::Ended {
                provider,
                key,
                at,
                reason,
            } => serde_json::json!({
                "timing": "ended", "provider": provider.to_string(), "key": key,
                "at_ms": ms(origin, *at), "reason": reason.as_str(),
            }),
            TimingEvent::Expired {
                provider,
                key,
                arrival,
                wall,
                stamp,
                age,
            } => serde_json::json!({
                "timing": "arrival", "provider": provider.to_string(), "key": key,
                "at_ms": ms(origin, *arrival),
                "wall_ns": wall.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos().to_string(),
                "stamp": {"time": stamp.get_time().as_u64().to_string(), "id": stamp.get_id().to_string(),
                          "age_ms": age.nanos() as f64 / 1e6},
                "lifespan": {"verdict": "expired", "reason": "expired"},
                "delivered": false,
            }),
        }
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
                move |ev| println!("{}", event_line(origin, &ev)),
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

/// `--follow`: a follower of a governed interface (`redundancy.v2` §2.10,
/// #987's N2).
pub mod follow {
    use std::io::BufRead;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use zenkey::consumer::Delivery;
    use zenkey::follow::{FollowEvent, FollowStartup};
    use zenkey::model::contract::load_path;
    use zenkey::model::grammar::Addr;
    use zenkey::{ServiceBuilder, ServiceConfig};

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    struct Opts {
        delay_ms: u64,
        for_s: Option<f64>,
        endpoint: String,
        provider: String,
        file: String,
        resource: String,
    }

    fn parse(args: &[String]) -> Result<Opts, String> {
        let usage = || {
            "usage: consume --follow [--descriptor-delay-ms <ms>] [--for <s>] \
             <endpoint> <system>/<service> <contract.toml> <resource>"
                .to_owned()
        };
        let mut o = Opts {
            delay_ms: 0,
            for_s: None,
            endpoint: String::new(),
            provider: String::new(),
            file: String::new(),
            resource: String::new(),
        };
        let mut rest = args;
        loop {
            match rest {
                [f, v, tail @ ..] if f == "--descriptor-delay-ms" => {
                    o.delay_ms = v.parse().map_err(|e| format!("{f} {v:?}: {e}"))?;
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

    /// A delivery's members of its line: its provider, key, record, run
    /// and seq.
    fn delivery(d: &Delivery) -> serde_json::Value {
        serde_json::json!({
            "provider": d.provider.to_string(),
            "key": d.sample.key_expr().as_str(),
            "record": String::from_utf8_lossy(&d.sample.payload().to_bytes()),
            "delete": d.sample.kind() == zenoh::sample::SampleKind::Delete,
            "run": d.inband.map(|s| s.run.hex()),
            "seq": d.inband.map(|s| s.seq.get()),
        })
    }

    /// One event's line, as `--help` writes it, its instant from `origin`.
    pub fn event_line(origin: Instant, at: Instant, e: &FollowEvent) -> serde_json::Value {
        let at_ms = ms(origin, at);
        let with = |kind: &str, d: &Delivery| {
            let mut v = delivery(d);
            v["follow"] = kind.into();
            v["at_ms"] = at_ms.into();
            v
        };
        match e {
            FollowEvent::Incumbent {
                provider,
                incumbent,
            } => serde_json::json!({
                "follow": "incumbent", "provider": provider.to_string(),
                "incumbent": incumbent.as_ref().map(|i| i.as_str().to_owned()), "at_ms": at_ms,
            }),
            FollowEvent::Accepted { delivery, seeded } => {
                let mut v = with("accepted", delivery);
                v["seeded"] = (*seeded).into();
                v
            }
            FollowEvent::Discarded { delivery, why } => {
                let mut v = with("discarded", delivery);
                v["reason"] = why.as_str().into();
                v
            }
            FollowEvent::Held { delivery } => with("held", delivery),
            FollowEvent::Superseded { delivery } => with("superseded", delivery),
            FollowEvent::SeedDiscarded {
                provider,
                key,
                snapshot,
                run,
                why,
            } => serde_json::json!({
                "follow": "seed_discarded", "provider": provider.to_string(), "key": key,
                "snapshot": snapshot, "run": run.map(|r| r.hex()), "reason": why.as_str(),
                "at_ms": at_ms,
            }),
            FollowEvent::Omitted { provider, key } => serde_json::json!({
                "follow": "omitted", "provider": provider.to_string(), "key": key, "at_ms": at_ms,
            }),
            FollowEvent::Seeded {
                provider,
                incumbent,
                complete,
                snapshots,
            } => serde_json::json!({
                "follow": "seeded", "provider": provider.to_string(),
                "incumbent": incumbent.as_ref().map(|i| i.as_str().to_owned()),
                "complete": complete, "snapshots": snapshots, "at_ms": at_ms,
            }),
        }
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
        let watch = session(&o.endpoint).await?;
        let cfg = ServiceConfig::new("interop/follower".parse()?).bind("p", &[&o.provider]);
        let mut b = ServiceBuilder::new(&main, cfg);
        b.require("p", contract.iface.clone(), false);
        let svc = b.start().await?;
        let consumer = svc
            .consumer("p", Arc::clone(&contract))?
            .with_watch(watch.clone());
        let origin = Instant::now();
        let st = FollowStartup {
            descriptor_delay: Duration::from_millis(o.delay_ms),
            ..FollowStartup::default()
        };
        let following = match consumer
            .follow_with(&o.resource, st, move |e| {
                println!("{}", event_line(origin, Instant::now(), &e));
            })
            .await
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("not followed: {e}");
                std::process::exit(2);
            }
        };
        println!(
            "{}",
            serde_json::json!({
                "follow": "started",
                "resource": o.resource,
                "zid": main.zid().to_string(),
                "watch": watch.zid().to_string(),
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
                    Ok(line) => println!("{}", answer(&line, &following)),
                    Err(_) => break,
                },
            }
        }
        drop(following);
        svc.close().await?;
        watch.close().await?;
        main.close().await?;
        Ok(())
    }

    /// A line of standard input's answer, as `--help` writes it.
    fn answer(line: &str, f: &zenkey::follow::Following) -> String {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["values"] => {
                let values: serde_json::Map<String, serde_json::Value> = f
                    .values()
                    .iter()
                    .map(|(k, d)| {
                        (
                            k.clone(),
                            serde_json::json!({
                                "record": String::from_utf8_lossy(&d.sample.payload().to_bytes()),
                                "run": d.inband.map(|s| s.run.hex()),
                                "seq": d.inband.map(|s| s.seq.get()),
                            }),
                        )
                    })
                    .collect();
                serde_json::json!({"follow": "values", "values": values}).to_string()
            }
            ["counts"] => {
                let c = f.counts();
                serde_json::json!({
                    "follow": "counts", "accepted": c.accepted, "discarded": c.discarded,
                    "held": c.held, "superseded": c.superseded, "omitted": c.omitted,
                    "reads": c.reads, "seeds": c.seeds,
                })
                .to_string()
            }
            ["incumbent", p] => match p.parse::<Addr>() {
                Ok(a) => serde_json::json!({
                    "follow": "incumbent?",
                    "provider": a.to_string(),
                    "incumbent": f.incumbent(&a).map(|i| i.as_str().to_owned()),
                })
                .to_string(),
                Err(e) => format!("refused {line}: {e}"),
            },
            _ => format!("refused {line}: values | counts | incumbent <system>/<service>"),
        }
    }
}
