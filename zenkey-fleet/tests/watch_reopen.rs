//! A watch session's reopen beside a large population (#882, core §8.1,
//! 0.44), in a binary of its own: the crowd's tokens load the router, and
//! nothing else should run beside them.
//!
//! zk2py's round 68 (F-239) measured that a watch session's transport that
//! opens again tells its subscribers without history every token they
//! match, and that the round trip after the reopen waits for them; and that
//! `zenctl watchdog --rule 'instance-gone crowd/*'` beside 10,000 tokens
//! never read ok again after a cut and a heal. These measure both in Rust.

mod util;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use util::zk2::router;
use zenkey_fleet::Sipper as _;
use zenkey_fleet::judge::condition::{Condition, WatchdogSpec, watchdog};
use zenkey_fleet::report::CondState;

/// A TCP relay to `upstream` the test can cut (every connection closed,
/// new ones refused) and heal; sessions dialling it reconnect once healed.
struct Relay {
    endpoint: String,
    cut: Arc<AtomicBool>,
    conns: Arc<Mutex<Vec<tokio::task::AbortHandle>>>,
}

impl Relay {
    async fn to(upstream: &str) -> Relay {
        let target = upstream.trim_start_matches("tcp/").to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("tcp/{}", listener.local_addr().unwrap());
        let cut = Arc::new(AtomicBool::new(false));
        let conns: Arc<Mutex<Vec<tokio::task::AbortHandle>>> = Arc::default();
        let (c, cs) = (Arc::clone(&cut), Arc::clone(&conns));
        tokio::spawn(async move {
            while let Ok((mut down, _)) = listener.accept().await {
                if c.load(Ordering::SeqCst) {
                    continue;
                }
                let Ok(mut up) = tokio::net::TcpStream::connect(&target).await else {
                    continue;
                };
                let h = tokio::spawn(async move {
                    let _ = tokio::io::copy_bidirectional(&mut down, &mut up).await;
                });
                cs.lock().unwrap().push(h.abort_handle());
            }
        });
        Relay {
            endpoint,
            cut,
            conns,
        }
    }

    fn cut(&self) {
        self.cut.store(true, Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            c.abort();
        }
    }

    fn heal(&self) {
        self.cut.store(false, Ordering::SeqCst);
    }
}

/// How many tokens: `WF_CROWD`, else `default`.
fn crowd_size(default: usize) -> usize {
    std::env::var("WF_CROWD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// The crowd's `i`th instance token, base-relative: address `crowd/c<i>`.
fn crowd_key(i: usize) -> String {
    format!("zk2/crowd/c{i}/@zk/instance/{:016x}", i + 1)
}

/// `n` instance tokens in `ns` on four sessions of the router at `ep`,
/// held by the router once this returns.
async fn crowd(
    ep: &str,
    n: usize,
) -> (Vec<zenoh::Session>, Vec<zenoh::liveliness::LivelinessToken>) {
    let endpoints = [ep.to_owned()];
    let mut holders = Vec::new();
    for _ in 0..4 {
        holders.push(
            zenkey_fleet::open_in_namespace("ns", None, &endpoints, &[], Some(false))
                .await
                .expect("a session in ns"),
        );
    }
    let started = Instant::now();
    let mut tokens = Vec::with_capacity(n);
    for i in 0..n {
        tokens.push(
            holders[i % holders.len()]
                .liveliness()
                .declare_token(crowd_key(i))
                .await
                .expect("a token"),
        );
    }
    let tool = zenkey_fleet::open_in_namespace("ns", None, &endpoints, &[], Some(false))
        .await
        .expect("a session in ns");
    let deadline = Instant::now() + Duration::from_secs(300);
    for last in n.saturating_sub(holders.len())..n {
        let key = crowd_key(last);
        while !zenkey_fleet::liveliness_read(&tool, &key, Duration::from_secs(5))
            .await
            .is_ok_and(|r| r.complete && r.keys == [key.clone()])
        {
            assert!(Instant::now() < deadline, "the router never held {key}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    eprintln!("WF: the crowd of {n} held after {:?}", started.elapsed());
    tool.close().await.ok();
    (holders, tokens)
}

/// Waits until the barrier's session has had two transport events since
/// `seen`, a close and an open, returning when it had.
async fn reopened(barrier: &zenkey::presence::Barrier, seen: u64, deadline: Duration) -> Instant {
    let until = Instant::now() + deadline;
    while barrier.moves() < seen + 2 {
        assert!(Instant::now() < until, "the session never reopened");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    Instant::now()
}

/// F-239 in Rust: what a subscriber without history on a watch session in
/// no namespace is told at a reopen, and what the round trip after it
/// costs, beside the crowd in `ns`. Ignored: a measurement (`WF_CROWD`,
/// 10,000 by default), printed with `--nocapture`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement (#882): WF_CROWD tokens, --nocapture"]
async fn wf_the_reopen_tells_every_token_and_the_round_trip_waits_measured() {
    let n = crowd_size(10_000);
    let (_router, ep) = router(None).await;
    let (_holders, _tokens) = crowd(&ep, n).await;
    let relay = Relay::to(&ep).await;
    let watch = zenkey_fleet::open_watch("ns", None, &[relay.endpoint.clone()], &[], Some(false))
        .await
        .expect("a watch session");
    let heard = Arc::new(AtomicU64::new(0));
    let last = Arc::new(Mutex::new(Instant::now()));
    let (h, l) = (Arc::clone(&heard), Arc::clone(&last));
    let _sub = watch
        .subscribe("the crowd", "zk2/*/*/@zk/instance/*", move |t| {
            if t.held {
                h.fetch_add(1, Ordering::SeqCst);
                *l.lock().unwrap() = Instant::now();
            }
        })
        .await
        .expect("a subscriber");
    let barrier = watch
        .barrier("zk2/*/*/@zk/instance/*", Duration::from_secs(120))
        .await
        .expect("a barrier");
    let t = Instant::now();
    let stood = barrier.establish().await;
    eprintln!(
        "WF: n={n}: the round trip at the start took {:?}, standing {stood}, {} told",
        t.elapsed(),
        heard.load(Ordering::SeqCst)
    );
    for run in 0..2 {
        let seen = barrier.moves();
        relay.cut();
        tokio::time::sleep(Duration::from_secs(3)).await;
        heard.store(0, Ordering::SeqCst);
        relay.heal();
        let healed = Instant::now();
        let open = reopened(&barrier, seen, Duration::from_secs(30)).await;
        let t = Instant::now();
        let stood = barrier.establish().await;
        let took = t.elapsed();
        let told_by_final = heard.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let told = heard.load(Ordering::SeqCst);
        let last_told = last.lock().unwrap().duration_since(open);
        eprintln!(
            "WF: n={n} run {run}: reopened {:?} after the heal; the round trip after it took \
             {took:?}, standing {stood}; told {told_by_final} by its end, {told} in all, the \
             last {last_told:?} after the reopen",
            open.duration_since(healed)
        );
    }
}

/// Which of the reader's sessions cross the relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    Watch,
    Deployment,
    All,
}

fn link() -> Link {
    match std::env::var("WF_LINK").as_deref() {
        Ok("watch") => Link::Watch,
        Ok("deployment") => Link::Deployment,
        _ => Link::All,
    }
}

/// #882 in Rust: the fleet's watchdog with `instance-gone crowd/*`, as
/// `zenctl watchdog --namespace ns --timeout 30` runs it (`--every` 5 s),
/// beside the crowd; the relay cut for 3 s and healed. Prints how long
/// after the heal the rule reads ok again. `WF_CROWD`, `WF_LINK`
/// (`watch`, `deployment`, `all`), `WF_CAP` seconds (120).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement (#882): WF_CROWD, WF_LINK, --nocapture"]
async fn wf_the_watchdog_after_a_reopen_beside_the_crowd_measured() {
    let n = crowd_size(10_000);
    let link = link();
    let cap = Duration::from_secs(
        std::env::var("WF_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120),
    );
    let (_router, ep) = router(None).await;
    let (_holders, _tokens) = crowd(&ep, n).await;
    let relay = Relay::to(&ep).await;
    let via = |crosses: bool| {
        if crosses {
            vec![relay.endpoint.clone()]
        } else {
            vec![ep.clone()]
        }
    };
    let (dep_ep, watch_ep) = match link {
        Link::Watch => (via(false), via(true)),
        Link::Deployment => (via(true), via(false)),
        Link::All => (via(true), via(true)),
    };
    let session = zenkey_fleet::open_in_namespace("ns", None, &dep_ep, &[], Some(false))
        .await
        .expect("the deployment's session");
    let raw = zenkey_fleet::open_reporting(None, &dep_ep, &[], Some(false))
        .await
        .expect("a raw session");
    let watch = zenkey_fleet::open_watch("ns", None, &watch_ep, &[], Some(false))
        .await
        .expect("a watch session");
    let bus = zenkey_fleet::DoctorBus {
        session,
        raw,
        namespace: "ns".to_owned(),
        watch: Some(watch),
    };
    let spec = WatchdogSpec {
        rules: vec![Condition::parse("instance-gone crowd/*").expect("rule")],
        tick: Duration::from_secs(5),
        ticks: None,
        timeout: Duration::from_secs(30),
        clocks_synced: false,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = Instant::now();
    let run = tokio::spawn(async move {
        let store = zenkey_fleet::BundleStore::new(spec.timeout);
        let contracts = zenkey_fleet::ContractSet::new();
        let mut run = watchdog(&bus, &store, &contracts, &spec).pin();
        while let Some(t) = run.sip().await {
            if tx.send((Instant::now(), t)).is_err() {
                break;
            }
        }
    });
    let ok = until(
        &mut rx,
        started,
        CondState::Ok,
        Instant::now() + Duration::from_secs(300),
    )
    .await
    .expect("ok at the start");
    eprintln!(
        "WF: n={n} link={link:?}: ok {:?} after it started",
        ok.duration_since(started)
    );
    for cut in 0..2 {
        relay.cut();
        let down = until(
            &mut rx,
            started,
            CondState::Unobservable,
            Instant::now() + Duration::from_secs(30),
        )
        .await;
        eprintln!(
            "WF: n={n} link={link:?} cut {cut}: unobservable {:?} after the cut",
            down.map(|d| d.elapsed())
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        relay.heal();
        let healed = Instant::now();
        let ok = until(&mut rx, started, CondState::Ok, healed + cap).await;
        eprintln!(
            "WF: n={n} link={link:?} cut {cut}: ok {:?} after the heal (cap {cap:?})",
            ok.map(|o| o.duration_since(healed))
        );
        if ok.is_none() {
            break;
        }
    }
    run.abort();
}

type Transitions =
    tokio::sync::mpsc::UnboundedReceiver<(Instant, zenkey_fleet::report::Transition)>;

/// The next transition to `want` before `deadline`, printing each one.
async fn until(
    rx: &mut Transitions,
    started: Instant,
    want: CondState,
    deadline: Instant,
) -> Option<Instant> {
    loop {
        match tokio::time::timeout_at(deadline.into(), rx.recv()).await {
            Ok(Some((at, t))) => {
                eprintln!(
                    "WF:   {:?} {:?} -> {:?}: {}",
                    at.duration_since(started),
                    t.from,
                    t.to,
                    t.evidence
                );
                if t.to == want {
                    return Some(at);
                }
            }
            _ => return None,
        }
    }
}

/// The mechanism (#882): a [`zenkey_fleet::PresenceFeed`] after a reopen
/// beside the crowd, its `next()` polled alone, or in a `select!` beside a
/// branch that wins every `WF_COMPETE` ms (the watchdog's raw monitor
/// ticks every 250 ms). Prints how long after the heal its seed is
/// complete again, and how many reads it made. `WF_CROWD`, `WF_LINK`,
/// `WF_COMPETE` (0: none), `WF_CAP`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement (#882): WF_CROWD, WF_LINK, WF_COMPETE, --nocapture"]
async fn wf_the_feed_after_a_reopen_beside_the_crowd_measured() {
    use zenkey_fleet::{
        LivePresence, Monitor, MonitorSpec, PresenceFeed, PresenceScope, SeedOutcome,
    };
    let n = crowd_size(10_000);
    let link = link();
    let compete: u64 = std::env::var("WF_COMPETE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let cap = Duration::from_secs(
        std::env::var("WF_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120),
    );
    let (_router, ep) = router(None).await;
    let (_holders, _tokens) = crowd(&ep, n).await;
    let relay = Relay::to(&ep).await;
    let via = |crosses: bool| {
        if crosses {
            vec![relay.endpoint.clone()]
        } else {
            vec![ep.clone()]
        }
    };
    let (dep_ep, watch_ep) = match link {
        Link::Watch => (via(false), via(true)),
        Link::Deployment => (via(true), via(false)),
        Link::All => (via(true), via(true)),
    };
    let session = zenkey_fleet::open_in_namespace("ns", None, &dep_ep, &[], Some(false))
        .await
        .expect("the deployment's session");
    let watch = zenkey_fleet::open_watch("ns", None, &watch_ep, &[], Some(false))
        .await
        .expect("a watch session");
    let scope = PresenceScope::system("crowd").expect("a scope");
    let spec = MonitorSpec {
        stats_tick: Duration::from_secs(3600),
        ..MonitorSpec::default()
    }
    .with_presence(&scope);
    let monitor = Monitor::start_watch(&watch, spec).await.expect("a monitor");
    for _ in 0..2 {
        let t = Instant::now();
        let observed = zenkey_fleet::read_tokens(&session, &scope, Duration::from_secs(30))
            .await
            .expect("a read");
        let read = t.elapsed();
        let t = Instant::now();
        let mut live = LivePresence::new(scope.clone());
        let moved = live.seed(&observed, std::time::SystemTime::now());
        eprintln!(
            "WF: n={n}: a read of the crowd took {read:?} ({} tokens, complete {}), its fold {:?} \
             ({} moves)",
            observed.tokens.len(),
            observed.complete,
            t.elapsed(),
            moved.len()
        );
    }
    let started = Instant::now();
    let (mut feed, _) = PresenceFeed::open(
        &session,
        &monitor,
        LivePresence::new(scope),
        Duration::from_secs(30),
    )
    .await
    .expect("the feed opens");
    eprintln!(
        "WF: n={n} link={link:?} compete={compete}ms: the first seed {:?} in {:?}",
        feed.last_seed(),
        started.elapsed()
    );
    let barrier = monitor.barrier().expect("a barrier").clone();
    let mut ticker = tokio::time::interval(Duration::from_millis(compete.max(1)));
    for cut in 0..2 {
        relay.cut();
        tokio::time::sleep(Duration::from_secs(3)).await;
        // Folds what the cut moved.
        while tokio::time::timeout(Duration::from_millis(10), feed.next())
            .await
            .is_ok()
        {}
        let reads = feed.reads();
        let gets = zenkey::presence::liveliness_gets(&session);
        relay.heal();
        let healed = Instant::now();
        let complete = tokio::time::timeout(cap, async {
            loop {
                if compete == 0 {
                    let _ = feed.next().await;
                } else {
                    tokio::select! {
                        _ = feed.next() => {}
                        _ = ticker.tick() => {}
                    }
                }
                if *feed.last_seed() == SeedOutcome::Complete && barrier.established() {
                    return healed.elapsed();
                }
            }
        })
        .await
        .ok();
        let gets_after = zenkey::presence::liveliness_gets(&session);
        eprintln!(
            "WF: n={n} link={link:?} compete={compete}ms cut {cut}: complete {complete:?} after \
             the heal (cap {cap:?}); {} read(s) folded, {} liveliness GET(s) asked on the \
             deployment's session; last seed {:?}",
            feed.reads() - reads,
            gets_after.reads - gets.reads,
            feed.last_seed()
        );
        if complete.is_none() {
            break;
        }
    }
    drop(feed);
    monitor.shutdown().await.ok();
}

/// #882 as the round ran it: `$WF_ZENCTL watchdog --namespace ns --rule
/// 'instance-gone crowd/*' --timeout 30`, every session through the relay,
/// beside the crowd; the relay cut for 3 s and healed. Prints its
/// transitions, how long after the heal it reads ok, and how many of
/// zenoh's `Unknown interest` and `Didn't receive DeclareFinal` lines its
/// stderr carried. `WF_ZENCTL` (a built binary), `WF_CROWD`, `WF_CAP`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "a measurement (#882): WF_ZENCTL, WF_CROWD, --nocapture"]
async fn wf_zenctl_watchdog_after_a_reopen_beside_the_crowd_measured() {
    use tokio::io::AsyncBufReadExt;
    let zenctl = std::env::var("WF_ZENCTL").expect("WF_ZENCTL names a zenctl binary");
    let n = crowd_size(10_000);
    let cap = Duration::from_secs(
        std::env::var("WF_CAP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120),
    );
    let (_router, ep) = router(None).await;
    let (_holders, _tokens) = crowd(&ep, n).await;
    let relay = Relay::to(&ep).await;
    let mut child = tokio::process::Command::new(&zenctl)
        .args([
            "watchdog",
            "--namespace",
            "ns",
            "--rule",
            "instance-gone crowd/*",
            "--timeout",
            "30",
            "-c",
            &relay.endpoint,
        ])
        .env("RUST_LOG", "warn")
        .env(
            "ZENKEY_EXPLORER_CONFIG_DIR",
            std::env::temp_dir().join("wf-zenctl-home"),
        )
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("zenctl runs");
    let started = Instant::now();
    let unknown = Arc::new(AtomicU64::new(0));
    let no_final = Arc::new(AtomicU64::new(0));
    {
        let (u, f) = (Arc::clone(&unknown), Arc::clone(&no_final));
        let mut err = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = err.next_line().await {
                if line.contains("Unknown interest") {
                    u.fetch_add(1, Ordering::Relaxed);
                } else if line.contains("Didn't receive DeclareFinal") {
                    f.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut out = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::spawn(async move {
        while let Ok(Some(line)) = out.next_line().await {
            let v: serde_json::Value = serde_json::from_str(&line).expect("a json line");
            let to = match v["to"].as_str() {
                Some("ok") => CondState::Ok,
                Some("firing") => CondState::Firing,
                _ => CondState::Unobservable,
            };
            let t = zenkey_fleet::report::Transition {
                rule: v["rule"].as_str().unwrap_or_default().to_owned(),
                from: None,
                to,
                at: String::new(),
                evidence: v["evidence"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(120)
                    .collect(),
            };
            if tx.send((Instant::now(), t)).is_err() {
                break;
            }
        }
    });
    let ok = until(
        &mut rx,
        started,
        CondState::Ok,
        Instant::now() + Duration::from_secs(300),
    )
    .await
    .expect("ok at the start");
    eprintln!(
        "WF: zenctl n={n}: ok {:?} after it started",
        ok.duration_since(started)
    );
    relay.cut();
    let down = until(
        &mut rx,
        started,
        CondState::Unobservable,
        Instant::now() + Duration::from_secs(30),
    )
    .await;
    eprintln!(
        "WF: zenctl n={n}: unobservable {:?} after the cut",
        down.map(|d| d.elapsed())
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    relay.heal();
    let healed = Instant::now();
    let ok = until(&mut rx, started, CondState::Ok, healed + cap).await;
    eprintln!(
        "WF: zenctl n={n}: ok {:?} after the heal (cap {cap:?}); stderr: {} `Unknown interest`, \
         {} `Didn't receive DeclareFinal`",
        ok.map(|o| o.duration_since(healed)),
        unknown.load(Ordering::Relaxed),
        no_final.load(Ordering::Relaxed)
    );
    child.kill().await.ok();
}
