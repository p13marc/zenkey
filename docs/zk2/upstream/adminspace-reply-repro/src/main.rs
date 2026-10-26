//! asr: an admin-space GET of declared resources can freeze a zenoh router
//! (zenkey #890), with zenoh alone.
//!
//! `asr trial` starts `asr router` as a child process: a zenoh router with
//! its admin space on, listening on an ephemeral loopback port. The trial
//! then opens five client sessions of it, in its own process:
//!
//! - **load** declares `--load` each of subscribers, queryables, liveliness
//!   tokens, publishers and queriers, so the admin space's reply loop has
//!   work;
//! - **churn** declares and undeclares, in a loop, a subscriber, a
//!   liveliness subscriber with history (an interest) and a liveliness
//!   token: each one asks the router's tables for the write lock
//!   (`--churn-every MS` pauses between rounds);
//! - **ask** GETs `--selector` in a loop (target All, consolidation None,
//!   3 s timeout);
//! - **hb-pub** puts a heartbeat every 10 ms, and **hb-sub** hears it.
//!
//! The router is frozen when no heartbeat has arrived for `--stall` seconds
//! (default 10). The trial then takes the router's stacks with `gdb -p`
//! (`--stacks DIR`), prints one `result` line and exits, killing the
//! router. A trial that runs `--for` seconds (default 20) without a stall
//! is `clean`.
//!
//! One trial per process pair, so a frozen router never leaks into the next
//! trial. `run-matrix.sh` runs the matrix.
//!
//! **`--acl wide|narrow`** (zenkey #894) runs the router under access
//! control, `default_permission: deny`, with usrpwd principals: `asr`, which
//! every session but the asker presents and which may do anything under
//! `asr/**`, and `tool`, the asker, which holds zk2's admin read alone:
//! `wide` is core 0.15's (`@/*/router`, `@/*/router/**`), `narrow` 0.50's
//! (`@/*/router`, the storage manager's status subtree and `@/*/*`), each a
//! `query` on ingress and a `reply` on egress. `run-acl.sh` runs that
//! matrix.

use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::{Config, Session};

fn ver() -> &'static str {
    match env!("CARGO_PKG_NAME") {
        "asr-1101" => "1.10.1",
        "asr-1101-fix" => "1.10.1+fix",
        _ => "?",
    }
}

/// `--name value` from the command line, or the default.
fn opt(args: &[String], name: &str, default: &str) -> String {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn base() -> Config {
    let mut c = Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c
}

/// A client presenting `user` (password `<user>-pw`), when given: what a
/// router under `--acl` requires of every session.
async fn client_as(endpoint: &str, user: Option<&str>) -> Session {
    let mut c = base();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    if let Some(u) = user {
        c.insert_json5(
            "transport/auth/usrpwd",
            &format!("{{ user: \"{u}\", password: \"{u}-pw\" }}"),
        )
        .unwrap();
    }
    zenoh::open(c).await.expect("a client session")
}

/// The admin read `--acl` grants the asker: core 0.15's, or 0.50's.
fn admin_read(acl: &str) -> &'static [&'static str] {
    match acl {
        "wide" => &["@/*/router", "@/*/router/**"],
        "narrow" => &[
            "@/*/router",
            "@/*/router/status/plugins/storage_manager/storages/**",
            "@/*/*",
        ],
        other => panic!("--acl {other}: wide, narrow or none"),
    }
}

/// The router's `access_control` block under `--acl`: deny by default; `asr`
/// may do anything under `asr/**`, both ways; `tool` holds the admin read.
fn access_control(acl: &str) -> String {
    let keys: Vec<String> = admin_read(acl).iter().map(|k| format!("\"{k}\"")).collect();
    let keys = keys.join(", ");
    format!(
        r#"{{
  enabled: true,
  default_permission: "deny",
  rules: [
    {{ id: "asr", permission: "allow", flows: ["ingress", "egress"],
      messages: ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply",
                 "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"],
      key_exprs: ["asr/**"] }},
    {{ id: "admin-read-in", permission: "allow", flows: ["ingress"], messages: ["query"],
      key_exprs: [{keys}] }},
    {{ id: "admin-read-out", permission: "allow", flows: ["egress"], messages: ["reply"],
      key_exprs: [{keys}] }},
  ],
  subjects: [ {{ id: "asr", usernames: ["asr"] }}, {{ id: "tool", usernames: ["tool"] }} ],
  policies: [
    {{ id: "asr", rules: ["asr"], subjects: ["asr"] }},
    {{ id: "tool", rules: ["admin-read-in", "admin-read-out"], subjects: ["tool"] }},
  ],
}}"#
    )
}

/// The router: admin space on, read permitted, an ephemeral loopback port;
/// under `--acl`, usrpwd from `--dict` and the access control block.
/// Prints `endpoint <locator>` and runs until stdin closes.
async fn router(args: &[String]) {
    let mut c = base();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")
        .unwrap();
    c.insert_json5("adminspace/enabled", "true").unwrap();
    c.insert_json5("adminspace/permissions/read", "true")
        .unwrap();
    let acl = opt(args, "--acl", "none");
    if acl != "none" {
        let dict = opt(args, "--dict", "");
        c.insert_json5(
            "transport/auth/usrpwd",
            &format!("{{ user: \"router\", password: \"router-pw\", dictionary_file: {dict:?} }}"),
        )
        .unwrap();
        c.insert_json5("access_control", &access_control(&acl))
            .unwrap();
    }
    let r = zenoh::open(c).await.expect("a router");
    let endpoint = r
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("a loopback listener");
    println!("endpoint {endpoint}");
    println!("zid {}", r.zid());
    // Until the trial goes away.
    let mut line = String::new();
    let _ = BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await;
    std::process::exit(0);
}

/// One gdb frame as `function (dir/file.rs:line)`: the function's last two
/// path segments, generics dropped.
fn frame(l: &str) -> String {
    let body = l.split_once(' ').map(|(_, b)| b.trim_start()).unwrap_or(l);
    let body = body.split_once(" in ").map(|(_, b)| b).unwrap_or(body);
    let func = body.split(" (").next().unwrap_or(body);
    let func = func.split('<').next().unwrap_or(func);
    let func: Vec<&str> = func.rsplit("::").take(2).collect();
    let func: Vec<&str> = func.into_iter().rev().collect();
    let at = l
        .rsplit_once(" at ")
        .map(|(_, s)| {
            let mut v: Vec<&str> = s.rsplit('/').take(2).collect();
            v.reverse();
            v.join("/")
        })
        .unwrap_or_default();
    format!("{} ({at})", func.join("::"))
}

/// `gdb -p`'s `thread apply all bt`, whole, and one line per thread: its
/// name and the frames that say what it waits on.
fn stacks(pid: u32) -> (String, String) {
    let out = std::process::Command::new("gdb")
        .args([
            "-p",
            &pid.to_string(),
            "-batch",
            "-nx",
            "-ex",
            "thread apply all bt",
        ])
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_else(|e| format!("gdb did not run: {e}\n"));
    const WANTED: [&str; 18] = [
        "new_transport_unicast",
        "send_declare",
        "send_interest",
        "closed",
        "resources_data",
        "linkstate_data",
        "route_successor",
        "subscribers_data",
        "publishers_data",
        "queryables_data",
        "queriers_data",
        "tokens_data",
        "route_send_response",
        "route_data",
        "read_contended",
        "write_contended",
        "interests::interest",
        "declare",
    ];
    let mut lines = Vec::new();
    let mut name = String::new();
    let mut frames: Vec<String> = Vec::new();
    let flush = |name: &str, frames: &mut Vec<String>, lines: &mut Vec<String>| {
        if !name.is_empty() && !frames.is_empty() {
            frames.reverse();
            frames.dedup();
            lines.push(format!("{name}: {}", frames.join(" -> ")));
        }
        frames.clear();
    };
    for l in out.lines() {
        if l.starts_with("Thread ") {
            flush(&name, &mut frames, &mut lines);
            name = l
                .split('"')
                .nth(1)
                .map(str::to_string)
                .unwrap_or_else(|| l.to_string());
        } else if l.starts_with('#') {
            if WANTED.iter().any(|w| l.contains(*w)) {
                frames.push(frame(l));
            }
        }
    }
    flush(&name, &mut frames, &mut lines);
    (out, lines.join("\n"))
}

async fn trial(args: &[String]) {
    let selector = opt(args, "--selector", "@/*/router/subscriber/**");
    let load: usize = opt(args, "--load", "500").parse().unwrap();
    // `concrete`: `asr/load/<i>`; `wild`: `asr/load/<i>/**`.
    let shape = opt(args, "--shape", "concrete");
    // `all`, `sub`, `lsub`, `token`, `connect` or `none`.
    let churn = opt(args, "--churn", "all");
    let run_for = Duration::from_secs_f64(opt(args, "--for", "20").parse().unwrap());
    let stall = Duration::from_secs_f64(opt(args, "--stall", "10").parse().unwrap());
    // A pause between churn rounds, ms (0: flat out).
    let churn_every = Duration::from_millis(opt(args, "--churn-every", "0").parse().unwrap());
    let stacks_dir = opt(args, "--stacks", "");
    let label = opt(args, "--trial", "0");
    // `none`, `wide` or `narrow`; under either of the last two every session
    // presents a principal: the asker `tool`, every other one `asr`.
    let acl = opt(args, "--acl", "none");
    let (me, asker_user) = match acl.as_str() {
        "none" => (None, None),
        a => {
            let _ = admin_read(a);
            (Some("asr"), Some("tool"))
        }
    };
    let dict = std::env::temp_dir().join(format!("asr-usrpwd-{}.txt", std::process::id()));
    if me.is_some() {
        std::fs::write(&dict, "router:router-pw\nasr:asr-pw\ntool:tool-pw\n").unwrap();
    }

    let exe = std::env::current_exe().unwrap();
    let mut child = tokio::process::Command::new(exe)
        .arg("router")
        .args(["--acl", &acl, "--dict", &dict.display().to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("the router process");
    let pid = child.id().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap()).lines();
    let endpoint = loop {
        let line = out
            .next_line()
            .await
            .unwrap()
            .expect("the router's endpoint");
        if let Some(e) = line.strip_prefix("endpoint ") {
            break e.to_string();
        }
    };

    let loader = client_as(&endpoint, me).await;
    let mut held = Vec::new();
    for i in 0..load {
        let key = match shape.as_str() {
            "wild" => format!("asr/load/{i}/**"),
            _ => format!("asr/load/{i}"),
        };
        held.push(Box::new(loader.declare_subscriber(&key).await.unwrap()) as Box<dyn Send>);
        held.push(Box::new(
            loader
                .declare_queryable(&key)
                .callback(|_q| {})
                .await
                .unwrap(),
        ));
        held.push(Box::new(
            loader
                .liveliness()
                .declare_token(format!("asr/load/{i}/token"))
                .await
                .unwrap(),
        ));
        held.push(Box::new(
            loader
                .declare_publisher(format!("asr/load/{i}/pub"))
                .await
                .unwrap(),
        ));
        held.push(Box::new(
            loader
                .declare_querier(format!("asr/load/{i}/querier"))
                .await
                .unwrap(),
        ));
    }

    // The heartbeat, heard before anything else runs.
    let hb_pub = client_as(&endpoint, me).await;
    let hb_sub = client_as(&endpoint, me).await;
    let epoch = Instant::now();
    let last_hb = Arc::new(AtomicU64::new(0));
    let heard = Arc::new(AtomicBool::new(false));
    let _hb = {
        let last_hb = Arc::clone(&last_hb);
        let heard = Arc::clone(&heard);
        hb_sub
            .declare_subscriber("asr/hb")
            .callback(move |_s| {
                last_hb.store(epoch.elapsed().as_millis() as u64, Ordering::Relaxed);
                heard.store(true, Ordering::Relaxed);
            })
            .await
            .unwrap()
    };
    tokio::spawn(async move {
        let mut n = 0u64;
        loop {
            n += 1;
            let _ = hb_pub.put("asr/hb", n.to_le_bytes().to_vec()).await;
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !heard.load(Ordering::Relaxed) {
        assert!(Instant::now() < deadline, "no heartbeat through the router");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Every session the trial churns or asks with is open before the GETs
    // start: a session that connects is a writer of the tables too (a new
    // face, `Gateway::new_transport_unicast`), and the `connect` churn
    // measures that path on its own.
    let asker = client_as(&endpoint, asker_user).await;
    let churner = client_as(&endpoint, me).await;
    // Settle the load's declarations, then one GET alone: what the admin
    // space answers this selector with, before any churn.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let get = |s: Session, sel: String| async move {
        let replies = s
            .get(&sel)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(Duration::from_secs(3))
            .await
            .unwrap();
        let (mut ok, mut err) = (0u64, 0u64);
        while let Ok(r) = replies.recv_async().await {
            if r.result().is_ok() {
                ok += 1
            } else {
                err += 1
            }
        }
        (ok, err)
    };
    let (first_ok, first_err) = get(asker.clone(), selector.clone()).await;

    let start = epoch.elapsed().as_millis() as i64;
    // Asks that ended, and churn rounds the client issued: a declaration is
    // sent and not waited for, so `churns` goes on counting after a freeze
    // until the client's queue fills.
    let asks = Arc::new(AtomicU64::new(0));
    let churns = Arc::new(AtomicU64::new(0));
    {
        let asks = Arc::clone(&asks);
        let (asker, selector) = (asker.clone(), selector.clone());
        tokio::spawn(async move {
            loop {
                get(asker.clone(), selector.clone()).await;
                asks.fetch_add(1, Ordering::Relaxed);
            }
        });
    }
    if churn != "none" {
        let churns = Arc::clone(&churns);
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let mut n = 0u64;
            loop {
                n += 1;
                let key = format!("asr/churn/{n}");
                let every = churn == "all";
                if every || churn == "sub" {
                    let s = churner.declare_subscriber(&key).await.unwrap();
                    let _ = s.undeclare().await;
                }
                if every || churn == "lsub" {
                    let s = churner
                        .liveliness()
                        .declare_subscriber(format!("{key}/**"))
                        .history(true)
                        .callback(|_s| {})
                        .await
                        .unwrap();
                    let _ = s.undeclare().await;
                }
                if every || churn == "token" {
                    let t = churner.liveliness().declare_token(&key).await.unwrap();
                    let _ = t.undeclare().await;
                }
                if churn == "connect" {
                    // A session that comes and goes: a face added and
                    // removed under the write lock.
                    let mut c = base();
                    c.insert_json5("mode", "\"client\"").unwrap();
                    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
                        .unwrap();
                    if let Some(u) = me {
                        c.insert_json5(
                            "transport/auth/usrpwd",
                            &format!("{{ user: \"{u}\", password: \"{u}-pw\" }}"),
                        )
                        .unwrap();
                    }
                    // A frozen router accepts no session: the churn stops.
                    let Ok(s) = zenoh::open(c).await else { break };
                    let _ = tokio::time::timeout(Duration::from_secs(2), s.close()).await;
                }
                churns.fetch_add(1, Ordering::Relaxed);
                if !churn_every.is_zero() {
                    tokio::time::sleep(churn_every).await;
                }
            }
        });
    }

    let mut outcome = "clean";
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let now = epoch.elapsed().as_millis() as i64;
        let last = last_hb.load(Ordering::Relaxed) as i64;
        if now - last >= stall.as_millis() as i64 {
            outcome = "hung";
            break;
        }
        if now - start >= run_for.as_millis() as i64 {
            break;
        }
    }
    // When the router froze: its last heartbeat, from the start of the
    // churn and the asks (negative: before the first of them ended).
    let t_ms = if outcome == "hung" {
        last_hb.load(Ordering::Relaxed) as i64 - start
    } else {
        epoch.elapsed().as_millis() as i64 - start
    };
    // The counters as they stood at the stall, read before the stacks.
    let (asks, churns) = (asks.load(Ordering::Relaxed), churns.load(Ordering::Relaxed));
    let mut summary = String::new();
    if outcome == "hung" {
        let (full, short) = stacks(pid);
        if !stacks_dir.is_empty() {
            let dir = std::path::Path::new(&stacks_dir);
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(dir.join(format!("trial-{label}.full.txt")), &full);
            let _ = std::fs::write(dir.join(format!("trial-{label}.txt")), &short);
        }
        // The replying thread holds a read and waits for another; the
        // writer waits for the first to go. Each named by its frames.
        let replier = short
            .lines()
            .find(|l| {
                l.contains("adminspace::")
                    && l.contains("route_send_response")
                    && l.contains("read_contended")
            })
            .and_then(|l| l.split(": ").nth(1))
            .and_then(|f| f.split(" -> ").next())
            .map(|f| f.split(' ').next().unwrap_or(f).to_string())
            .unwrap_or_else(|| "none".into());
        // The writer: the frame that asked for the write lock.
        let writer = short
            .lines()
            .find(|l| l.contains("write_contended"))
            .and_then(|l| l.split(": ").nth(1))
            .and_then(|f| f.rsplit(" -> ").nth(1))
            .map(|f| f.split(' ').next().unwrap_or(f).to_string())
            .unwrap_or_else(|| "none".into());
        summary = format!(" replier={replier} writer={writer}");
    }
    println!(
        "result ver={} trial={label} selector={selector} load={load} shape={shape} churn={} \
         every_ms={} acl={acl} outcome={outcome} t_ms={t_ms} asks={asks} churns={churns} \
         first_replies={first_ok} first_errors={first_err}{summary}",
        ver(),
        opt(args, "--churn", "all"),
        churn_every.as_millis(),
    );
    let _ = std::fs::remove_file(&dict);
    let _ = child.start_kill();
    std::process::exit(0);
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("router") => router(&args).await,
        Some("trial") => trial(&args).await,
        _ => {
            eprintln!(
                "usage: asr trial [--selector SEL] [--load N] [--shape concrete|wild] \
                 [--churn all|sub|lsub|token|connect|none] [--churn-every MS] [--for SECS] \
                 [--stall SECS] [--acl none|wide|narrow] \
                 [--stacks DIR] [--trial LABEL]"
            );
            std::process::exit(2);
        }
    }
}
