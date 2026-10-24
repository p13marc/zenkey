//! lgr: a liveliness GET beside liveliness subscribers, zenoh only (zenkey #830).
//!
//! One binary, built twice (`v1101/` against zenoh 1.10.1, `vmain/` against
//! zenoh's main branch). Subcommands:
//!
//! - `trial`: one fresh process making one measurement. Without `--connect`
//!   it opens its own in-process router, token holder and reader (the shape
//!   zenkey's test suite measured); with `--connect EP` it is only the reader
//!   of a shared router and holder.
//! - `drive`: runs `--trials` trials, `--jobs` at once, each a child process,
//!   and appends every result line to `--out`.
//! - `router`, `holder`: the shared router and token holder for `--connect`.
//! - `summary FILE...`: the table over result lines.
//!
//! Shapes (where the liveliness subscribers are):
//! - A: history subscribers on the reading session;
//! - B: non-history subscribers on the reading session;
//! - C: history subscribers on a second session of the same process;
//! - D: no subscribers.
//!
//! Every GET uses an unbounded flume handler (zenoh#2678), so a full channel
//! is never the cause; subscribers are callbacks.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use zenoh::query::Reply;
use zenoh::sample::SampleKind;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const BROAD: &str = "zk2/*/*/@zk/**";
const SYSTEM: &str = "h1";

fn version() -> &'static str {
    match env!("CARGO_PKG_NAME") {
        "lgr-1101" => "1.10.1",
        "lgr-1101-fix" => "1.10.1+fix",
        _ => "main",
    }
}

// ---------------------------------------------------------------- args

#[derive(Clone, Debug)]
struct Args(BTreeMap<String, String>, Vec<String>);

impl Args {
    fn parse(v: &[String]) -> Args {
        let mut m = BTreeMap::new();
        let mut free = Vec::new();
        let mut i = 0;
        while i < v.len() {
            if let Some(k) = v[i].strip_prefix("--") {
                if i + 1 < v.len() && !v[i + 1].starts_with("--") {
                    m.insert(k.to_owned(), v[i + 1].clone());
                    i += 2;
                } else {
                    m.insert(k.to_owned(), "true".to_owned());
                    i += 1;
                }
            } else {
                free.push(v[i].clone());
                i += 1;
            }
        }
        Args(m, free)
    }
    fn get(&self, k: &str, d: &str) -> String {
        self.0.get(k).cloned().unwrap_or_else(|| d.to_owned())
    }
    fn opt(&self, k: &str) -> Option<String> {
        self.0.get(k).cloned()
    }
    fn num(&self, k: &str, d: u64) -> u64 {
        self.0.get(k).map(|s| s.parse().expect(k)).unwrap_or(d)
    }
    fn flag(&self, k: &str) -> bool {
        self.0.get(k).is_some_and(|s| s == "true")
    }
}

// ---------------------------------------------------------------- sessions

fn base_config() -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false").unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c
}

async fn open_router(listen: &str) -> Res<(zenoh::Session, String)> {
    let mut c = base_config();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", &format!("[\"{listen}\"]")).unwrap();
    let r = zenoh::open(c).await?;
    let ep = r
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .ok_or("no loopback tcp listener")?;
    Ok((r, ep))
}

async fn open_client(ep: &str) -> Res<zenoh::Session> {
    let mut c = base_config();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]")).unwrap();
    Ok(zenoh::open(c).await?)
}

// ---------------------------------------------------------------- tokens

fn hex16(v: u64) -> String {
    format!("{v:016x}")
}

fn seed() -> u64 {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    t ^ ((std::process::id() as u64) << 32)
}

/// zk2's token shapes for `services` services under `zk2/h1/`: an instance
/// token, two interface tokens, and for every third service two member
/// tokens. Returns (all keys, the instance key of each service).
fn token_keys(services: usize, salt: u64) -> (Vec<String>, Vec<String>) {
    let fp = "0123456789abcdef";
    let mut all = Vec::new();
    let mut inst = Vec::new();
    for s in 0..services {
        let svc = format!("svc{s}");
        let id = hex16(salt.wrapping_mul(6364136223846793005).wrapping_add(s as u64 + 1));
        let base = format!("zk2/{SYSTEM}/{svc}/@zk");
        let i = format!("{base}/instance/{id}");
        inst.push(i.clone());
        all.push(i);
        if s % 3 == 2 {
            all.push(format!("{base}/alive/config.v1/{id}/{fp}"));
            all.push(format!("{base}/member/config.v1/m0/{id}"));
            all.push(format!("{base}/member/config.v1/m1/{id}"));
        } else {
            all.push(format!("{base}/alive/ping.v1/{id}/{fp}"));
            all.push(format!("{base}/alive/health.v1/{id}/{fp}"));
        }
    }
    (all, inst)
}

async fn declare_tokens(s: &zenoh::Session, keys: &[String]) -> Res<Vec<zenoh::liveliness::LivelinessToken>> {
    let mut v = Vec::new();
    for k in keys {
        v.push(s.liveliness().declare_token(k.clone()).await?);
    }
    Ok(v)
}

// ---------------------------------------------------------------- reads

struct Read {
    keys: BTreeSet<String>,
    errors: Vec<String>,
    ms: f64,
}

impl Read {
    fn complete(&self) -> bool {
        self.errors.is_empty()
    }
}

async fn lget(s: &zenoh::Session, sel: &str, timeout: Duration) -> Res<Read> {
    let t0 = Instant::now();
    let rx = s
        .liveliness()
        .get(sel)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await?;
    let mut keys = BTreeSet::new();
    let mut errors = Vec::new();
    while let Ok(reply) = rx.recv_async().await {
        match reply.result() {
            Ok(sample) => {
                keys.insert(sample.key_expr().as_str().to_owned());
            }
            Err(e) => errors.push(String::from_utf8_lossy(&e.payload().to_bytes()).into_owned()),
        }
    }
    Ok(Read { keys, errors, ms: t0.elapsed().as_secs_f64() * 1e3 })
}

/// `c:<found>/<held>` complete, `i:<found>/<held>` incomplete (an error reply).
fn verdict(r: &Read, held: &[String]) -> (String, Vec<String>) {
    let missing: Vec<String> = held.iter().filter(|k| !r.keys.contains(*k)).cloned().collect();
    let found = held.len() - missing.len();
    let tag = if r.complete() { "c" } else { "i" };
    (format!("{tag}:{found}/{}", held.len()), missing)
}

fn class(r: &Read, held: &[String]) -> &'static str {
    if !r.complete() {
        "incomplete"
    } else if held.iter().all(|k| r.keys.contains(k)) {
        "complete"
    } else {
        "MISSED"
    }
}

// ---------------------------------------------------------------- subscribers

fn sub_selectors(which: &str) -> Vec<String> {
    which
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|w| match w {
            "instance" => "zk2/*/*/@zk/instance/*".to_owned(),
            "alive" => "zk2/*/*/@zk/alive/**".to_owned(),
            "member" => "zk2/*/*/@zk/member/**".to_owned(),
            "broad" => BROAD.to_owned(),
            other => other.to_owned(),
        })
        .collect()
}

async fn declare_subs(
    s: &zenoh::Session,
    sels: &[String],
    history: bool,
    seen: Arc<Mutex<BTreeSet<String>>>,
    puts: Arc<AtomicUsize>,
) -> Res<Vec<zenoh::pubsub::Subscriber<()>>> {
    let mut v = Vec::new();
    for sel in sels {
        let seen = seen.clone();
        let puts = puts.clone();
        v.push(
            s.liveliness()
                .declare_subscriber(sel.clone())
                .history(history)
                .callback(move |smp| {
                    let k = smp.key_expr().as_str().to_owned();
                    if smp.kind() == SampleKind::Put {
                        puts.fetch_add(1, Ordering::SeqCst);
                        seen.lock().unwrap().insert(k);
                    } else {
                        seen.lock().unwrap().remove(&k);
                    }
                })
                .await?,
        );
    }
    Ok(v)
}

// ---------------------------------------------------------------- trial

async fn trial(a: &Args) -> Res<String> {
    let shape = a.get("shape", "A");
    let order = a.get("order", "tokens-first");
    let wait = a.get("wait", "0");
    let services = a.num("services", 3) as usize;
    let timeout = Duration::from_millis(a.num("timeout-ms", 1000));
    let sels = sub_selectors(&a.get("subs", "instance,alive,member"));
    let seq = a.get("seq", "0");
    let shared = a.opt("connect");
    let salt = a.num("salt", 0);

    // The router and holder: in process, or the shared ones.
    let mut _router = None;
    let ep = match &shared {
        Some(ep) => ep.clone(),
        None => {
            let (r, ep) = open_router("tcp/127.0.0.1:0").await?;
            _router = Some(r);
            ep
        }
    };
    // The trial holds its own tokens, on a session of its own, unless it
    // reads the shared holder's (a shared router, tokens first).
    let own_holder = shared.is_none() || order == "subs-first";
    let (held, inst) = token_keys(services, if own_holder { seed() } else { salt });

    let reader = open_client(&ep).await?;
    let second = if shape == "C" { Some(open_client(&ep).await?) } else { None };
    let holder = if own_holder { Some(open_client(&ep).await?) } else { None };

    let seen: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let puts = Arc::new(AtomicUsize::new(0));
    let mut _tokens = Vec::new();
    let mut _subs = Vec::new();

    let declare_holder = |h: Option<zenoh::Session>, held: Vec<String>| async move {
        match h {
            Some(h) => declare_tokens(&h, &held).await.map(|t| (Some(h), t)),
            None => Ok((None, Vec::new())),
        }
    };

    let mut holder_keep = None;
    if order == "tokens-first" {
        let (h, t) = declare_holder(holder.clone(), held.clone()).await?;
        holder_keep = h;
        _tokens = t;
        if own_holder {
            // The tokens are at the router before the reader subscribes: a
            // narrow read from the holder's own session is a round trip.
            for i in &inst {
                let _ = lget(holder_keep.as_ref().unwrap(), i, timeout).await?;
            }
        }
    }

    let t_sub = Instant::now();
    match shape.as_str() {
        "A" => _subs = declare_subs(&reader, &sels, true, seen.clone(), puts.clone()).await?,
        "B" => _subs = declare_subs(&reader, &sels, false, seen.clone(), puts.clone()).await?,
        "C" => _subs = declare_subs(second.as_ref().unwrap(), &sels, true, seen.clone(), puts.clone()).await?,
        "D" => {}
        s => return Err(format!("unknown shape {s}").into()),
    }
    let t_declared = t_sub.elapsed();

    if order == "subs-first" {
        // A round trip from the subscribing session through the router, so
        // the router holds the subscribers' interests before any token is
        // declared (a non-history subscriber sees only what comes after).
        let barrier = if shape == "C" { second.as_ref().unwrap() } else { &reader };
        let _ = lget(barrier, "zk2/barrier/none/@zk/instance/*", timeout).await?;
        let (h, t) = declare_holder(holder.clone(), held.clone()).await?;
        holder_keep = h;
        _tokens = t;
        // As in tokens-first: a round trip from the holder, so every token is
        // at the router before the wait starts.
        for i in &inst {
            let _ = lget(holder_keep.as_ref().unwrap(), i, timeout).await?;
        }
    }
    drop(holder);

    // The wait between declaring the subscribers and the GET.
    let t_wait = Instant::now();
    let mut narrow_before = String::from("-");
    match wait.as_str() {
        "samples" => {
            let want: BTreeSet<String> = held
                .iter()
                .filter(|k| sels.iter().any(|s| ke_intersects(s, k)))
                .cloned()
                .collect();
            let lim = Instant::now() + Duration::from_secs(10);
            while !want.is_subset(&seen.lock().unwrap()) {
                if Instant::now() > lim {
                    return Ok(format!("result seq={seq} ver={} error=samples-timeout", version()));
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        "narrow" => {
            // As zenkey's test did: read each instance narrowly until found.
            let lim = Instant::now() + Duration::from_secs(10);
            let mut tries = 0;
            for (i, ik) in inst.iter().enumerate() {
                let sel = format!("zk2/{SYSTEM}/svc{i}/@zk/instance/*");
                loop {
                    tries += 1;
                    let r = lget(&reader, &sel, timeout).await?;
                    if r.keys.contains(ik) {
                        break;
                    }
                    if Instant::now() > lim {
                        return Ok(format!("result seq={seq} ver={} error=narrow-timeout", version()));
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
            narrow_before = tries.to_string();
        }
        ms => {
            let ms: u64 = ms.trim_end_matches("ms").parse()?;
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
        }
    }
    let waited = t_sub.elapsed();
    let _ = t_wait;
    let seen_at_get = seen.lock().unwrap().len();
    let puts_at_get = puts.load(Ordering::SeqCst);

    // The measured read, then a second broad read and a narrow one per service.
    let first = lget(&reader, BROAD, timeout).await?;
    let second_r = lget(&reader, BROAD, timeout).await?;
    let mut narrow_found = 0;
    for (i, ik) in inst.iter().enumerate() {
        let r = lget(&reader, &format!("zk2/{SYSTEM}/svc{i}/@zk/instance/*"), timeout).await?;
        if r.complete() && r.keys.contains(ik) {
            narrow_found += 1;
        }
    }
    // And a broad read from a fresh session in the same process.
    let fresh = open_client(&ep).await?;
    let fresh_r = lget(&fresh, BROAD, timeout).await?;
    drop(fresh);

    let (v1, missing) = verdict(&first, &held);
    let (v2, missing2) = verdict(&second_r, &held);
    let (vf, _) = verdict(&fresh_r, &held);
    let short = |m: &[String]| {
        if m.is_empty() {
            "-".to_owned()
        } else {
            m.iter()
                .map(|k| {
                    let p: Vec<&str> = k.split('/').collect();
                    format!("{}:{}", p[2], p[4])
                })
                .collect::<Vec<_>>()
                .join(",")
        }
    };
    // Put samples the subscribers received in all: more than one per held
    // token means a token reached them twice.
    let puts_end = puts.load(Ordering::SeqCst);
    let line = format!(
        "result seq={seq} ver={} mode={} load={} order={order} shape={shape} wait={wait} subs={} held={} class={} first={v1} first_ms={:.1} first_errs={} missing={} second={v2} second_missing={} narrow={narrow_found}/{} fresh={vf} declared_ms={:.1} waited_ms={:.1} seen_at_get={seen_at_get} puts_at_get={puts_at_get} puts_end={puts_end} narrow_tries={narrow_before} extra={}",
        version(),
        if shared.is_some() { "shared" } else { "inproc" },
        a.get("load", "burn0"),
        sels.len(),
        held.len(),
        class(&first, &held),
        first.ms,
        first.errors.len(),
        short(&missing),
        short(&missing2),
        inst.len(),
        t_declared.as_secs_f64() * 1e3,
        waited.as_secs_f64() * 1e3,
        first.keys.len().saturating_sub(held.len() - missing.len()),
    );
    drop(_subs);
    drop(_tokens);
    drop(holder_keep);
    drop(second);
    Ok(line)
}

/// Key expression intersection, for the `samples` wait.
fn ke_intersects(sel: &str, key: &str) -> bool {
    match (zenoh::key_expr::KeyExpr::try_from(sel), zenoh::key_expr::KeyExpr::try_from(key)) {
        (Ok(a), Ok(b)) => a.intersects(&b),
        _ => false,
    }
}

// ---------------------------------------------------------------- shared router/holder

async fn wait_stdin_eof() {
    let mut l = String::new();
    let mut r = BufReader::new(tokio::io::stdin());
    loop {
        l.clear();
        match r.read_line(&mut l).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}

async fn router_cmd(a: &Args) -> Res<()> {
    let (_r, ep) = open_router(&a.get("listen", "tcp/127.0.0.1:0")).await?;
    println!("ready {ep}");
    wait_stdin_eof().await;
    Ok(())
}

async fn holder_cmd(a: &Args) -> Res<()> {
    let s = open_client(&a.get("connect", "")).await?;
    let (held, _) = token_keys(a.num("services", 3) as usize, a.num("salt", 0));
    let _t = declare_tokens(&s, &held).await?;
    println!("ready {}", held.len());
    wait_stdin_eof().await;
    Ok(())
}

// ---------------------------------------------------------------- driver

struct Child {
    child: tokio::process::Child,
    stdout: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
}

async fn spawn_ready(args: &[String]) -> Res<(Child, String)> {
    let mut c = tokio::process::Command::new(std::env::current_exe()?)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = BufReader::new(c.stdout.take().unwrap()).lines();
    let line = tokio::time::timeout(Duration::from_secs(30), stdout.next_line())
        .await??
        .ok_or("no ready line")?;
    Ok((Child { child: c, stdout }, line))
}

async fn drive(a: &Args) -> Res<()> {
    let trials = a.num("trials", 120);
    let jobs = a.num("jobs", 12) as usize;
    let out = a.get("out", "results.txt");
    let trace_dir = a.opt("trace-dir");
    let mut a = a.clone();
    a.0.insert("load".into(), format!("burn{}", a.num("burn", 0)));
    let a = &a;
    let pass: Vec<String> = ["shape", "order", "wait", "services", "timeout-ms", "subs", "load"]
        .iter()
        .filter_map(|k| a.opt(k).map(|v| vec![format!("--{k}"), v]))
        .flatten()
        .collect();

    // Shared mode: one router and one holder for every trial.
    let mut shared = Vec::new();
    let mut connect = Vec::new();
    if a.flag("shared") {
        let (r, line) = spawn_ready(&["router".into()]).await?;
        let ep = line.strip_prefix("ready ").ok_or("router")?.to_owned();
        let salt = seed().to_string();
        let (h, _) = spawn_ready(&["holder".into(), "--connect".into(), ep.clone(), "--services".into(), a.get("services", "3"), "--salt".into(), salt.clone()]).await?;
        shared.push(r);
        shared.push(h);
        connect = vec!["--connect".to_owned(), ep, "--salt".to_owned(), salt];
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // Optional CPU load beside the trials: `--burn N` spinning threads.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let burners: Vec<_> = (0..a.num("burn", 0))
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x: u64 = 1;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..10_000 {
                        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    }
                    std::hint::black_box(x);
                }
            })
        })
        .collect();

    let next = Arc::new(AtomicUsize::new(0));
    let file = Arc::new(tokio::sync::Mutex::new(
        tokio::fs::OpenOptions::new().create(true).append(true).open(&out).await?,
    ));
    let counts: Arc<Mutex<BTreeMap<String, u64>>> = Arc::default();
    let t0 = Instant::now();
    let mut workers = Vec::new();
    for _ in 0..jobs {
        let next = next.clone();
        let file = file.clone();
        let counts = counts.clone();
        let mut args: Vec<String> = vec!["trial".into()];
        args.extend(pass.iter().cloned());
        args.extend(connect.iter().cloned());
        let trace_dir = trace_dir.clone();
        workers.push(tokio::spawn(async move {
            loop {
                let n = next.fetch_add(1, Ordering::SeqCst) as u64;
                if n >= trials {
                    return;
                }
                let mut args = args.clone();
                args.extend(["--seq".into(), n.to_string()]);
                let mut cmd = tokio::process::Command::new(std::env::current_exe().unwrap());
                cmd.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).kill_on_drop(true);
                // With --trace-dir, the child's whole output (zenoh logs to
                // stdout) is kept in a file for a miss.
                cmd.stderr(std::process::Stdio::null());
                let (line, text) = match tokio::time::timeout(Duration::from_secs(60), cmd.output()).await {
                    Ok(Ok(o)) => {
                        let text = String::from_utf8_lossy(&o.stdout).into_owned();
                        let line = text
                            .lines()
                            .find(|l| l.starts_with("result "))
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("result seq={n} ver={} error=no-result status={}", version(), o.status));
                        (line, text)
                    }
                    Ok(Err(e)) => (format!("result seq={n} ver={} error=spawn:{e}", version()), String::new()),
                    Err(_) => (format!("result seq={n} ver={} error=hung", version()), String::new()),
                };
                let cls = field(&line, "class").unwrap_or("error").to_owned();
                if let Some(d) = &trace_dir {
                    if cls == "MISSED" || cls == "error" {
                        let _ = std::fs::write(format!("{d}/trial-{}-{n}.log", std::process::id()), &text);
                    }
                }
                *counts.lock().unwrap().entry(cls).or_default() += 1;
                let mut f = file.lock().await;
                f.write_all(format!("{line}\n").as_bytes()).await.unwrap();
                f.flush().await.unwrap();
            }
        }));
    }
    let load0 = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    for w in workers {
        w.await?;
    }
    let load1 = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    stop.store(true, Ordering::Relaxed);
    for b in burners {
        let _ = b.join();
    }
    for mut c in shared {
        drop(c.child.stdin.take());
        let _ = tokio::time::timeout(Duration::from_secs(5), c.child.wait()).await;
        let _ = &mut c.stdout;
    }
    let msg = format!(
        "# drive ver={} {} trials={trials} jobs={jobs} burn={} secs={:.0} loadavg_start={} loadavg_end={} counts={:?}",
        version(),
        pass.join(" "),
        a.num("burn", 0),
        t0.elapsed().as_secs_f64(),
        load0.split_whitespace().next().unwrap_or("?"),
        load1.split_whitespace().next().unwrap_or("?"),
        counts.lock().unwrap()
    );
    let mut f = file.lock().await;
    f.write_all(format!("{msg}\n").as_bytes()).await?;
    f.flush().await?;
    println!("{msg}");
    Ok(())
}

fn field<'a>(line: &'a str, k: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|f| f.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
}

// ---------------------------------------------------------------- summary

fn summary(files: &[String]) -> Res<()> {
    #[derive(Default)]
    struct Cell {
        n: u64,
        incomplete: u64,
        complete: u64,
        missed: u64,
        error: u64,
        second_recovered: u64,
        narrow_found: u64,
        fresh_found: u64,
        missed_all: u64,
    }
    let mut cells: BTreeMap<String, Cell> = BTreeMap::new();
    for f in files {
        for line in std::fs::read_to_string(f)?.lines().filter(|l| l.starts_with("result ")) {
            let key = format!(
                "{} | {} | {} | {} | {} | {}",
                field(line, "ver").unwrap_or("?"),
                field(line, "mode").unwrap_or("?"),
                field(line, "load").unwrap_or("burn0"),
                field(line, "order").unwrap_or("?"),
                field(line, "shape").unwrap_or("?"),
                field(line, "wait").unwrap_or("?"),
            );
            let c = cells.entry(key).or_default();
            c.n += 1;
            match field(line, "class") {
                Some("incomplete") => c.incomplete += 1,
                Some("complete") => c.complete += 1,
                Some("MISSED") => {
                    c.missed += 1;
                    if field(line, "second").is_some_and(|s| s.starts_with("c:") && {
                        let (a, b) = s[2..].split_once('/').unwrap();
                        a == b
                    }) {
                        c.second_recovered += 1;
                    }
                    if let Some(n) = field(line, "narrow") {
                        let (a, b) = n.split_once('/').unwrap();
                        if a == b {
                            c.narrow_found += 1;
                        }
                    }
                    if let Some(s) = field(line, "fresh") {
                        let (a, b) = s[2..].split_once('/').unwrap();
                        if a == b {
                            c.fresh_found += 1;
                        }
                    }
                    if field(line, "first").is_some_and(|s| s.starts_with("c:0/")) {
                        c.missed_all += 1;
                    }
                }
                _ => c.error += 1,
            }
        }
    }
    println!("| zenoh | router | load | order | shape | wait | trials | incomplete | complete, all found | **complete, missed** | missed all | 2nd GET found all | narrow found all | fresh session found all | errors |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (k, c) in &cells {
        let miss = |x: u64| if c.missed == 0 { "-".to_owned() } else { format!("{x}/{}", c.missed) };
        println!(
            "| {k} | {} | {} | {} | **{}** | {} | {} | {} | {} | {} |",
            c.n,
            c.incomplete,
            c.complete,
            c.missed,
            miss(c.missed_all),
            miss(c.second_recovered),
            miss(c.narrow_found),
            miss(c.fresh_found),
            c.error
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- main

fn main() -> Res<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!("usage: lgr trial|drive|router|holder|summary ...");
        std::process::exit(2);
    };
    let a = Args::parse(&argv[1..]);
    if cmd == "summary" {
        return summary(&a.1);
    }
    if std::env::var("RUST_LOG").is_ok() {
        zenoh::init_log_from_env_or("error");
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build()?;
    rt.block_on(async move {
        match cmd.as_str() {
            "trial" => {
                let line = trial(&a).await?;
                println!("{line}");
                Ok(())
            }
            "drive" => drive(&a).await,
            "router" => router_cmd(&a).await,
            "holder" => holder_cmd(&a).await,
            other => Err(format!("unknown command {other}").into()),
        }
    })
}
