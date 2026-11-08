//! A zk2 archive to interoperate with (#977's last done-when row): it
//! serves `archive.v1` through the runtime's [`zenkey::archive::Archive`],
//! recording one or more owners' state and aligning with each by its
//! snapshot (core §4.4, §4.2), and runs until its standard input closes or
//! reads `quit`.
//!
//! ```text
//! cargo run -p zenkey --example archive -- [--connect <endpoint>] [--namespace <ns>]
//!     [--system <system>] [--service <service>] [--window <s>] [--peer <system>/<service>]...
//!     [--unconfirmed-horizon <s>] [--no-watch] [--hlc] [--timeout-ms <ms>] [--poll-ms <ms>]
//!     --record <system>/<service> <selector> <contract.toml> [--record …]...
//! ```
//!
//! `archive --help` (or `-h`) prints the interface a live runner drives,
//! its flags, its standard input and every line it prints, and exits 0:
//! [`HELP`] is the long form, and this doc its summary.
//!
//! - **The session**, as `owner`'s: a router listening on an ephemeral
//!   loopback port (`listening tcp/127.0.0.1:<port>`), or with `--connect`
//!   a client of that router (`connected <endpoint>`). `--namespace` is the
//!   session's namespace, the deployment base (core §1.1).
//! - **The archive** is `--system`/`--service` (default `ex/archive`). It
//!   records each `--record`'s owner under its selector, typed by the
//!   contract given, keeps tombstones for `--window` seconds (S3), and
//!   reads each `--peer` while an owner is absent (§4.4). It hears an owner
//!   appear on a watch session of its own (core §8.1), a second client of
//!   the same router, unless `--no-watch`, which reads presence on a
//!   period instead.
//! - **Standard output**, one line per event: the session's line, `ready
//!   <instance key>`, then one `aligned …` line for the alignment it runs
//!   once up and for each `align`, and a `held`/`gone` line each time what
//!   it holds changes, read back from its own keys every `--poll-ms`.
//! - **Standard input**: `align`, `last-known <origin key>`, `quit`. A
//!   line it cannot do is answered `refused <line>: <why>`.
//!
//! Exit 0 at `quit` or the end of standard input, 1 when the archive cannot
//! start, 2 on usage errors.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use zenkey::archive::{self, AlignedFrom, Alignment, Archive, ArchiveConfig, LastKnown, Recorded};
use zenkey::model::contract::load_path;
use zenkey::model::grammar::Addr;
use zenkey::{Implementation, ServiceConfig};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// What `archive --help` prints: the interface a runner drives, whole.
pub const HELP: &str = "\
archive: a zk2 archive to interoperate with (#977). It serves archive.v1
through the runtime's zenkey::archive::Archive (core §4.4): it records the
given owners' state, aligns with each by its snapshot, answers last-known
reads on its own keys, and runs until its standard input closes or reads
quit.

usage: archive [--connect <endpoint>] [--namespace <ns>]
    [--system <system>] [--service <service>] [--window <s>]
    [--peer <system>/<service>]... [--unconfirmed-horizon <s>] [--no-watch] [--hlc]
    [--timeout-ms <ms>] [--poll-ms <ms>]
    --record <system>/<service> <selector> <contract.toml> [--record ...]...

Flags:
  --connect <endpoint>        a client of that router; default: a router listening
                              on an ephemeral loopback port, as owner's
  --namespace <ns>            the sessions' namespace, the deployment base (core §1.1);
                              default: none. Every key below is base-relative
  --system <system>           the archive's system; default ex
  --service <service>         the archive's service; default archive
  --window <s>                its tombstone window in seconds, at least its owners'
                              (S3); default the runtime's, 60
  --record <owner> <selector> <contract.toml>
                              record <owner>'s (<system>/<service>) state keys under
                              <selector>, a key expression such as
                              zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*,
                              typed by the owner's contract; at least one, repeatable
  --peer <system>/<service>   an archive on the owners' side, read while an owner is
                              absent (§4.4); repeatable
  --unconfirmed-horizon <s>   an unconfirmed key may be dropped after <s> seconds;
                              default: kept
  --no-watch                  no watch session: owners' presence is read every 500 ms,
                              which misses an outage shorter than that (core §8.1)
  --hlc                       the sessions' timestamping on (core §8.2); default off
  --timeout-ms <ms>           each alignment GET's and last-known read's timeout;
                              default 2000
  --poll-ms <ms>              how often it reads back what it holds, for the held and
                              gone lines; default 200, 0 for never

Alignment (§4.4): it re-reads each owner it records by a collection read (a
member read for a selector naming one member), so the owner answers by
snapshot; while an owner is absent it reads its peers. It drops a key only
on positive evidence: a snapshot's tombstone, a per-key resource's or a
peer's reply_del, or the key's absence from the owner's complete snapshot
(core §4.2's omission rule, its guard keeping a value heard after the read
was asked). A key nothing reports stays, unconfirmed. It also aligns by
itself whenever an owner or a peer appears; those print no aligned line,
and what they change shows in the held and gone lines.

Standard output, one line per event, written as it happens: a held or gone line
may come between a command and its answer, so match an answer by its first word
(aligned, last-known, refused), never by its position:
  listening tcp/127.0.0.1:<port>      a router on an ephemeral port (with --connect:
                                      connected <endpoint>, a client of that router)
  ready zk2/<system>/<service>/@zk/instance/<instance>
                                      the archive is up: its tokens, its queryable,
                                      its subscribers on every --record selector
  aligned from=<from>[,<from>...] confirmed=<n> dropped=<n> unconfirmed=<n>
                                      one alignment: once when up, then per align.
                                      <from>, one per --record in order: owner:<owner>
                                      (the owner answered), peer:<peer> (it was absent,
                                      a peer answered) or nothing (neither did: nothing
                                      confirmed, nothing dropped). confirmed: keys the
                                      read confirmed; dropped: keys it dropped on
                                      positive evidence; unconfirmed: keys held that
                                      the read left unconfirmed
  held <origin> confirmed=<true|false>[ tombstone]
                                      it holds <origin> (a base-relative zk2 key),
                                      newly or with its confirmation changed; tombstone
                                      for a deletion kept within the window
  gone <origin>                       it no longer holds <origin>
  last-known <origin> ...             the answer to last-known (below)
  refused <line>: <why>               a line it cannot do

Standard input, one command per line:
  align                     align now: aligned ..., or refused align: <error>
  last-known <origin>       read <origin>'s last-known state from this archive's own
                            keys, as a consumer reads it (S5):
                              last-known <origin> none
                              last-known <origin> run=<run>|none seq=<seq>|none
                                ts=<timestamp>|none confirmed=<true|false> tombstone
                              last-known <origin> run=<run>|none seq=<seq>|none
                                ts=<timestamp>|none confirmed=<true|false> bytes=<n>
                                value=<the record, lossy UTF-8, to the end of the line>
                            (one line each; <run> in hex, 4 digits for a 16-bit run,
                            8 for a 32-bit one, as a descriptor spells it; <seq> in
                            decimal; ts the owner's Zenoh timestamp, <time>/<zid>
                            as zenoh writes one, where the resource carries one)
  quit                      stop, exit 0 (as the end of standard input does)

Exit: 0 at quit or the end of standard input; 1 when the archive cannot start
(the error on stderr); 2 on usage errors.
";

/// One `--record`.
#[derive(Debug, Clone)]
pub struct Record {
    pub owner: String,
    pub selector: String,
    pub contract: PathBuf,
}

/// The command line.
#[derive(Debug, Clone)]
pub struct Options {
    pub connect: Option<String>,
    pub namespace: Option<String>,
    pub system: String,
    pub service: String,
    pub window_s: Option<u64>,
    pub records: Vec<Record>,
    pub peers: Vec<String>,
    pub unconfirmed_horizon_s: Option<u64>,
    pub watch: bool,
    pub hlc: bool,
    pub timeout: Duration,
    pub poll: Option<Duration>,
}

const USAGE: &str = "usage: archive [--connect <endpoint>] [--namespace <ns>] \
     [--system <system>] [--service <service>] [--window <s>] \
     [--peer <system>/<service>]... [--unconfirmed-horizon <s>] [--no-watch] [--hlc] \
     [--timeout-ms <ms>] [--poll-ms <ms>] \
     --record <system>/<service> <selector> <contract.toml> [--record ...]...";

impl Options {
    /// The command line, or the usage line and why.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let bad = |why: &str| format!("{USAGE}\n{why}");
        let mut o = Options {
            connect: None,
            namespace: None,
            system: "ex".to_owned(),
            service: "archive".to_owned(),
            window_s: None,
            records: Vec::new(),
            peers: Vec::new(),
            unconfirmed_horizon_s: None,
            watch: true,
            hlc: false,
            timeout: Duration::from_millis(2000),
            poll: Some(Duration::from_millis(200)),
        };
        let mut it = args.iter();
        let value = |flag: &str, it: &mut std::slice::Iter<'_, String>| {
            it.next()
                .cloned()
                .ok_or_else(|| bad(&format!("{flag} needs a value")))
        };
        let number = |flag: &str, v: String| {
            v.parse::<u64>()
                .map_err(|_| bad(&format!("{flag}: {v:?} is not a whole number")))
        };
        while let Some(a) = it.next() {
            match a.as_str() {
                "--connect" => o.connect = Some(value(a, &mut it)?),
                "--namespace" => o.namespace = Some(value(a, &mut it)?),
                "--system" => o.system = value(a, &mut it)?,
                "--service" => o.service = value(a, &mut it)?,
                "--window" => o.window_s = Some(number(a, value(a, &mut it)?)?),
                "--peer" => o.peers.push(value(a, &mut it)?),
                "--unconfirmed-horizon" => {
                    o.unconfirmed_horizon_s = Some(number(a, value(a, &mut it)?)?);
                }
                "--no-watch" => o.watch = false,
                "--hlc" => o.hlc = true,
                "--timeout-ms" => {
                    o.timeout = Duration::from_millis(number(a, value(a, &mut it)?)?);
                }
                "--poll-ms" => {
                    let ms = number(a, value(a, &mut it)?)?;
                    o.poll = (ms > 0).then(|| Duration::from_millis(ms));
                }
                "--record" => {
                    let owner = value(a, &mut it)?;
                    let selector = value(a, &mut it)?;
                    let contract = PathBuf::from(value(a, &mut it)?);
                    o.records.push(Record {
                        owner,
                        selector,
                        contract,
                    });
                }
                other => return Err(bad(&format!("unknown argument {other:?}"))),
            }
        }
        if o.records.is_empty() {
            return Err(bad("at least one --record"));
        }
        let address = format!("{}/{}", o.system, o.service);
        address
            .parse::<Addr>()
            .map_err(|e| bad(&format!("--system/--service {address:?}: {e}")))?;
        for r in &o.records {
            let owner: Addr = r
                .owner
                .parse()
                .map_err(|e| bad(&format!("--record owner {:?}: {e}", r.owner)))?;
            let under = format!("zk2/{}/{}/", owner.system, owner.service);
            if !r.selector.starts_with(&under) {
                return Err(bad(&format!(
                    "--record selector {:?} is not under {under}",
                    r.selector
                )));
            }
        }
        for p in &o.peers {
            p.parse::<Addr>()
                .map_err(|e| bad(&format!("--peer {p:?}: {e}")))?;
        }
        Ok(o)
    }
}

/// How a line goes out: `main` prints it, a test collects it.
pub type Say = Arc<dyn Fn(String) + Send + Sync>;

/// `alignment` as its `aligned` line.
#[must_use]
pub fn aligned_line(a: &Alignment) -> String {
    let from: Vec<String> = a
        .from
        .iter()
        .map(|f| match f {
            AlignedFrom::Owner(o) => format!("owner:{o}"),
            AlignedFrom::Peer(p) => format!("peer:{p}"),
            AlignedFrom::Nothing => "nothing".to_owned(),
        })
        .collect();
    format!(
        "aligned from={} confirmed={} dropped={} unconfirmed={}",
        from.join(","),
        a.confirmed,
        a.dropped,
        a.unconfirmed
    )
}

/// `origin`'s `last-known` line, from what the archive answered.
#[must_use]
pub fn last_known_line(origin: &str, lk: Option<&LastKnown>) -> String {
    let Some(lk) = lk else {
        return format!("last-known {origin} none");
    };
    let (run, seq) = lk
        .inband
        .map_or(("none".to_owned(), "none".to_owned()), |s| {
            (s.run.to_string(), s.seq.get().to_string())
        });
    let ts = lk.timestamp.map_or("none".to_owned(), |t| t.to_string());
    let head = format!(
        "last-known {origin} run={run} seq={seq} ts={ts} confirmed={}",
        lk.confirmed
    );
    match &lk.value {
        None => format!("{head} tombstone"),
        Some(v) => format!(
            "{head} bytes={} value={}",
            v.len(),
            String::from_utf8_lossy(v)
        ),
    }
}

fn session_config(o: &Options, endpoint: Option<&str>) -> Result<zenoh::Config, BoxError> {
    let mut z = zenoh::Config::default();
    z.insert_json5("scouting/multicast/enabled", "false")?;
    if let Some(ns) = &o.namespace {
        z.insert_json5("namespace", &serde_json::to_string(ns)?)?;
    }
    if o.hlc {
        z.insert_json5("timestamping/enabled", "true")?;
    }
    match endpoint {
        None => {
            z.insert_json5("mode", "\"router\"")?;
            z.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")?;
        }
        Some(ep) => {
            z.insert_json5("mode", "\"client\"")?;
            z.insert_json5("connect/endpoints", &serde_json::to_string(&[ep])?)?;
            z.insert_json5(
                "connect/retry",
                "{ period_init_ms: 100, period_max_ms: 500, period_increase_factor: 1.5 }",
            )?;
        }
    }
    Ok(z)
}

/// Runs the archive, answering each line of `commands` through `say`,
/// until `until` resolves or a `quit` line.
pub async fn run_with(
    opts: Options,
    say: Say,
    commands: flume::Receiver<String>,
    until: impl Future<Output = ()>,
) -> Result<(), BoxError> {
    let (session, endpoint) = match &opts.connect {
        None => {
            let session = zenoh::open(session_config(&opts, None)?).await?;
            let endpoint = session
                .info()
                .locators()
                .await
                .into_iter()
                .map(|l| l.to_string())
                .find(|l| l.starts_with("tcp/127.0.0.1:"))
                .ok_or("no loopback listener")?;
            say(format!("listening {endpoint}"));
            (session, endpoint)
        }
        Some(ep) => {
            let session = zenoh::open(session_config(&opts, Some(ep))?).await?;
            say(format!("connected {ep}"));
            (session, ep.clone())
        }
    };
    // Core §8.1: the archive hears its owners appear on a session of its
    // own, a client of the same router, since its session reads presence.
    let watch = if opts.watch {
        Some(zenoh::open(session_config(&opts, Some(&endpoint))?).await?)
    } else {
        None
    };
    let me: Addr = format!("{}/{}", opts.system, opts.service).parse()?;
    let mut service = ServiceConfig::new(me.clone());
    service.tombstone_window_s = opts.window_s;
    let mut records = Vec::new();
    for r in &opts.records {
        let l = load_path(&r.contract);
        let contract = l
            .contract
            .ok_or_else(|| format!("{} does not load:\n{}", r.contract.display(), l.report))?;
        records.push(Recorded {
            owner: r.owner.parse()?,
            selector: r.selector.clone(),
            implementation: Implementation::new(contract),
        });
    }
    let selectors: Vec<String> = records.iter().map(|r| r.selector.clone()).collect();
    let archive = Archive::start(
        &session,
        ArchiveConfig {
            service,
            records,
            peers: opts
                .peers
                .iter()
                .map(|p| p.parse())
                .collect::<Result<_, _>>()?,
            unconfirmed_horizon: opts.unconfirmed_horizon_s.map(Duration::from_secs),
            watch,
        },
    )
    .await?;
    say(format!("ready {}", archive.service().instance_key()?));
    match archive.align(opts.timeout).await {
        Ok(a) => say(aligned_line(&a)),
        Err(e) => say(format!("refused align: {e}")),
    }
    // What it holds, read back from its own keys as a consumer would (S5):
    // a line each time an origin comes, goes, or changes confirmation.
    let held = opts.poll.map(|period| {
        let (session, me, say, timeout) =
            (session.clone(), me.clone(), Arc::clone(&say), opts.timeout);
        tokio::spawn(async move {
            let mut shown: BTreeMap<String, (bool, bool)> = BTreeMap::new();
            loop {
                let mut now = BTreeMap::new();
                let mut read = true;
                for sel in &selectors {
                    match archive::last_known_all(&session, &me, sel, timeout).await {
                        Ok(all) => {
                            for lk in all {
                                now.insert(lk.origin.clone(), (lk.confirmed, lk.value.is_none()));
                            }
                        }
                        Err(_) => read = false,
                    }
                }
                if read {
                    for (origin, &(confirmed, tombstone)) in &now {
                        if shown.get(origin) != Some(&(confirmed, tombstone)) {
                            say(format!(
                                "held {origin} confirmed={confirmed}{}",
                                if tombstone { " tombstone" } else { "" }
                            ));
                        }
                    }
                    for origin in shown.keys().filter(|o| !now.contains_key(*o)) {
                        say(format!("gone {origin}"));
                    }
                    shown = now;
                }
                tokio::time::sleep(period).await;
            }
        })
    });
    tokio::pin!(until);
    let mut ended = false;
    loop {
        let line = if ended {
            match commands.try_recv() {
                Ok(l) => l,
                Err(_) => break,
            }
        } else {
            tokio::select! {
                l = commands.recv_async() => match l {
                    Ok(l) => l,
                    Err(_) => break,
                },
                () = &mut until => {
                    ended = true;
                    continue;
                }
            }
        };
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            [] => {}
            ["quit"] => break,
            ["align"] => match archive.align(opts.timeout).await {
                Ok(a) => say(aligned_line(&a)),
                Err(e) => say(format!("refused align: {e}")),
            },
            ["last-known", origin] => {
                match archive::last_known(&session, &me, origin, opts.timeout).await {
                    Ok(lk) => say(last_known_line(origin, lk.as_ref())),
                    Err(e) => say(format!("refused {line}: {e}")),
                }
            }
            _ => say(format!(
                "refused {line}: usage: align | last-known <origin> | quit"
            )),
        }
    }
    if let Some(task) = held {
        task.abort();
    }
    drop(archive);
    session.close().await?;
    Ok(())
}

/// Each line `open`'s reader gives, as commands, and a future that
/// resolves once it ends, as `owner`'s: the reader runs on a thread of its
/// own from the call on, so a line sent before the archive is up is
/// answered once it is.
pub fn command_lines<R, F>(open: F) -> (flume::Receiver<String>, impl Future<Output = ()> + Send)
where
    R: std::io::BufRead,
    F: FnOnce() -> R + Send + 'static,
{
    let (lines, commands) = flume::unbounded::<String>();
    let (ended, end) = flume::bounded::<()>(1);
    std::thread::spawn(move || {
        for line in open().lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
        let _ = ended.send(());
    });
    (commands, async move {
        let _ = end.recv_async().await;
    })
}

#[tokio::main]
async fn main() {
    // The runtime's logs go to stderr, so a runner sees what it says.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return;
    }
    let opts = match Options::parse(&args) {
        Ok(o) => o,
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    };
    let (commands, stdin_closed) = command_lines(|| std::io::stdin().lock());
    let say: Say = Arc::new(|line: String| println!("{line}"));
    if let Err(e) = run_with(opts, say, commands, stdin_closed).await {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
