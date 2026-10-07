//! S14, the ownership ACL on a live router (#616, r3 §3.13). Decides U14,
//! U21 and the generator rules `zenctl acl gen` ports (#612).
//!
//! Principals authenticate with usrpwd; one router enforces ACL. The grant
//! shapes of P3 compile to rules:
//! - **own** `sys/svc`: ingress put, delete, queryable, reply, token on the
//!   service's keys; egress query and subscriber interest on them. `**`
//!   never crosses a verbatim chunk, so `@stream`, `@state`, `@op` and `@zk`
//!   are spelled out.
//! - **consume** keys: ingress subscriber, query, liveliness subscriber and
//!   query; egress samples, replies and tokens.
//! - **call** operation keys: ingress query; egress reply.
//!
//! Three postures: default deny with these allows; default allow with the
//! same allows (U21: are allows evaluated?); default allow with each grant
//! compiled into denies of its complement (D13).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zk2rt::config::Topo;
use zk2rt::metrics::csv_row;

use crate::procs::{self, Proc};

struct Principal {
    name: &'static str,
    own: &'static [&'static str],
    consume: &'static [&'static str],
    call: &'static [&'static str],
    /// Resources of its own that nobody consumes (for D13's complement).
    private: &'static [&'static str],
}

const PRINCIPALS: &[Principal] = &[
    Principal {
        name: "thruster-l",
        own: &["vehicle-01/thruster-l"],
        consume: &[
            "zk2/vehicle-01/teleop/twist_cmd.v1/stream/cmd",
            "zk2/vehicle-01/autopilot/twist_cmd.v1/stream/cmd",
            "zk2/vehicle-01/safety/twist_cmd.v1/stream/cmd",
            "zk2/vehicle-01/*/@zk/**",
        ],
        call: &[],
        private: &["zk2/vehicle-01/thruster-l/thruster.v1/state/status"],
    },
    Principal { name: "teleop", own: &["vehicle-01/teleop"], consume: &[], call: &[], private: &[] },
    Principal { name: "autopilot", own: &["vehicle-01/autopilot"], consume: &[], call: &[], private: &[] },
    Principal { name: "safety", own: &["vehicle-01/safety"], consume: &[], call: &[], private: &[] },
    Principal { name: "tc-h1", own: &["h1/tc"], consume: &[], call: &[], private: &[] },
    Principal { name: "tc-h2", own: &["h2/tc"], consume: &[], call: &[], private: &[] },
    Principal {
        name: "frontend",
        own: &["ops/frontend"],
        consume: &["zk2/*/tc/tc.netif.v1/state/**", "zk2/*/tc/@zk/**"],
        call: &["zk2/*/tc/tc.netem.v1/@op/**"],
        private: &[],
    },
    Principal { name: "fleet-mgr", own: &["ground/fleet-mgr"], consume: &[], call: &[], private: &[] },
    Principal {
        name: "executor",
        own: &["vehicle-01/executor"],
        consume: &["zk2/ground/fleet-mgr/desired.v1/state/plans/vehicle-01", "zk2/ground/fleet-mgr/@zk/**"],
        call: &[],
        private: &[],
    },
];

fn own_keys(p: &str) -> Vec<String> {
    vec![
        format!("zk2/{p}/**"),
        format!("zk2/{p}/*/@stream/**"),
        format!("zk2/{p}/*/@state/**"),
        format!("zk2/{p}/*/@op/**"),
        format!("zk2/{p}/@zk/**"),
    ]
}

fn rule(id: String, messages: &[&str], flow: &str, permission: &str, keys: Vec<String>) -> Value {
    json!({"id": id, "messages": messages, "flows": [flow], "permission": permission, "key_exprs": keys})
}

/// The allow rules of every principal (the P3 grants).
fn grants() -> (Vec<Value>, Vec<Value>) {
    let mut rules = Vec::new();
    let mut policies = Vec::new();
    for p in PRINCIPALS {
        let own: Vec<String> = p.own.iter().flat_map(|o| own_keys(o)).collect();
        let mut ids = vec![format!("{}-own-in", p.name), format!("{}-own-out", p.name), "contracts-in".into(), "contracts-out".into()];
        rules.push(rule(format!("{}-own-in", p.name), &["put", "delete", "declare_queryable", "reply", "liveliness_token"], "ingress", "allow", own.clone()));
        // Egress to a provider is checked against the query's or the
        // subscription's own key expression, by inclusion: a consumer's
        // wildcard selector (`zk2/*/tc/...`) is not included in
        // `zk2/h1/tc/**`, so every consumer selector that intersects this
        // provider's keys is granted here too.
        let mut out = own.clone();
        for q in PRINCIPALS.iter().filter(|q| q.name != p.name) {
            for sel in q.consume.iter().chain(q.call.iter()) {
                let Ok(sk) = zenoh::key_expr::KeyExpr::try_from(*sel) else { continue };
                if own.iter().any(|o| zenoh::key_expr::KeyExpr::try_from(o.as_str()).is_ok_and(|ok| ok.intersects(&sk))) {
                    out.push((*sel).to_owned());
                }
            }
        }
        out.sort();
        out.dedup();
        // A reply to a wildcard query (a refusal included) is checked
        // against the query's key: the same selectors, for replies.
        let wild: Vec<String> = out.iter().filter(|k| !own.contains(k)).cloned().collect();
        if !wild.is_empty() {
            rules.push(rule(format!("{}-replies-in", p.name), &["reply"], "ingress", "allow", wild));
            ids.push(format!("{}-replies-in", p.name));
        }
        rules.push(rule(format!("{}-own-out", p.name), &["query", "declare_subscriber"], "egress", "allow", out));
        if !p.consume.is_empty() {
            let keys: Vec<String> = p.consume.iter().map(|k| (*k).to_owned()).collect();
            rules.push(rule(format!("{}-consume-in", p.name), &["declare_subscriber", "query", "declare_liveliness_subscriber", "liveliness_query"], "ingress", "allow", keys.clone()));
            rules.push(rule(format!("{}-consume-out", p.name), &["put", "delete", "reply", "liveliness_token"], "egress", "allow", keys));
            ids.extend([format!("{}-consume-in", p.name), format!("{}-consume-out", p.name)]);
        }
        if !p.call.is_empty() {
            let keys: Vec<String> = p.call.iter().map(|k| (*k).to_owned()).collect();
            rules.push(rule(format!("{}-call-in", p.name), &["query"], "ingress", "allow", keys.clone()));
            rules.push(rule(format!("{}-call-out", p.name), &["reply"], "egress", "allow", keys));
            ids.extend([format!("{}-call-in", p.name), format!("{}-call-out", p.name)]);
        }
        policies.push(json!({"id": format!("{}-policy", p.name), "rules": ids, "subjects": [p.name]}));
    }
    // Contract bundles: anyone may hold or fetch one; the hash is the check.
    rules.push(rule("contracts-in".into(), &["declare_queryable", "reply", "query"], "ingress", "allow", vec!["zk2/@zk/contract/**".into()]));
    rules.push(rule("contracts-out".into(), &["query", "reply"], "egress", "allow", vec!["zk2/@zk/contract/**".into()]));
    (rules, policies)
}

/// D13: under default allow, each grant compiled into denies of its
/// complement: writes and serving on every other principal's keys, and
/// reads of every resource not granted.
fn complement() -> (Vec<Value>, Vec<Value>) {
    let mut rules = Vec::new();
    let mut policies = Vec::new();
    for p in PRINCIPALS {
        let others: Vec<String> = PRINCIPALS.iter().filter(|o| o.name != p.name).flat_map(|o| o.own.iter().flat_map(|k| own_keys(k))).collect();
        let mut ids = Vec::new();
        rules.push(rule(format!("{}-deny-write", p.name), &["put", "delete", "declare_queryable", "reply", "liveliness_token"], "ingress", "deny", others));
        ids.push(format!("{}-deny-write", p.name));
        // Reads not granted: other principals' private resources, and the
        // presence of systems it does not consume.
        let mut read_deny: Vec<String> = PRINCIPALS.iter().filter(|o| o.name != p.name).flat_map(|o| o.private.iter().map(|k| (*k).to_owned())).collect();
        if !p.consume.iter().any(|k| k.contains("vehicle-01/*/@zk")) {
            read_deny.push("zk2/vehicle-01/*/@zk/**".into());
        }
        if !p.consume.iter().any(|k| k.contains("fleet-mgr/desired.v1/state/plans")) {
            read_deny.push("zk2/ground/fleet-mgr/desired.v1/state/**".into());
        } else {
            read_deny.push("zk2/ground/fleet-mgr/desired.v1/state/plans/vehicle-02".into());
        }
        rules.push(rule(format!("{}-deny-read", p.name), &["declare_subscriber", "query", "declare_liveliness_subscriber", "liveliness_query"], "ingress", "deny", read_deny));
        ids.push(format!("{}-deny-read", p.name));
        policies.push(json!({"id": format!("{}-policy", p.name), "rules": ids, "subjects": [p.name]}));
    }
    (rules, policies)
}

fn acl(default: &str, rules: Vec<Value>, policies: Vec<Value>) -> Value {
    let subjects: Vec<Value> = PRINCIPALS.iter().map(|p| json!({"id": p.name, "usernames": [p.name]})).collect();
    json!({"enabled": true, "default_permission": default, "rules": rules, "subjects": subjects, "policies": policies})
}

async fn open_as(ep: &str, user: Option<&str>) -> Result<zenoh::Session> {
    let mut c = Topo::client(&[ep.to_owned()]).config()?;
    if let Some(u) = user {
        c.insert_json5("transport/auth/usrpwd/user", &format!("\"{u}\"")).map_err(|e| anyhow!("{e}"))?;
        c.insert_json5("transport/auth/usrpwd/password", &format!("\"pw-{u}\"")).map_err(|e| anyhow!("{e}"))?;
    }
    zenoh::open(c).await.map_err(|e| anyhow!("open as {user:?}: {e}"))
}

type Got = Arc<Mutex<Vec<String>>>;

async fn sub(s: &zenoh::Session, key: &str) -> Result<(zenoh::pubsub::Subscriber<()>, Got)> {
    let got: Got = Arc::default();
    let g = got.clone();
    let sb = s.declare_subscriber(key.to_owned()).callback(move |smp| g.lock().unwrap().push(smp.key_expr().as_str().to_owned())).await.map_err(|e| anyhow!("{e}"))?;
    Ok((sb, got))
}

struct Row {
    posture: &'static str,
    check: &'static str,
    expected: String,
    observed: String,
}

#[allow(clippy::too_many_lines)]
async fn posture(name: &'static str, acl: Value, dir: &Path, rows: &mut Vec<Row>, expect: &dyn Fn(&str) -> &'static str) -> Result<()> {
    let users: String = PRINCIPALS.iter().map(|p| format!("{0}:pw-{0}\n", p.name)).collect();
    let dict = dir.join("users.txt");
    std::fs::write(&dict, users)?;
    let extra = json!({ "access_control": acl, "transport/auth/usrpwd/dictionary_file": dict.display().to_string() });
    let cfg = dir.join(format!("router-{name}.json"));
    std::fs::write(&cfg, serde_json::to_string_pretty(&extra)?)?;
    let ep = format!("tcp/127.0.0.1:{}", zk2rt::config::free_port()?);
    let _r: Proc = procs::spawn(&["router".into(), "--listen".into(), ep.clone(), "--extra-config".into(), cfg.display().to_string()], Duration::from_secs(30)).await?;
    let mut ss = std::collections::BTreeMap::new();
    for p in PRINCIPALS {
        ss.insert(p.name, open_as(&ep, Some(p.name)).await?);
    }
    let settle = || tokio::time::sleep(Duration::from_millis(400));
    let mut push = |check: &'static str, observed: String| rows.push(Row { posture: name, check, expected: expect(check).to_owned(), observed });
    let yn = |b: bool| if b { "yes".to_owned() } else { "no".to_owned() };

    // The thruster's subscriptions (its consume grants), and presence.
    let (_s1, from_teleop) = sub(&ss["thruster-l"], "zk2/vehicle-01/teleop/twist_cmd.v1/stream/cmd").await?;
    let (_s2, from_autopilot) = sub(&ss["thruster-l"], "zk2/vehicle-01/autopilot/twist_cmd.v1/stream/cmd").await?;
    let _toks: Vec<_> = {
        let mut v = Vec::new();
        for n in ["teleop", "autopilot", "safety", "thruster-l"] {
            v.push(ss[n].liveliness().declare_token(format!("zk2/vehicle-01/{n}/@zk/instance/00000000000000{:02x}", n.len())).await.map_err(|e| anyhow!("{e}"))?);
        }
        v
    };
    let (_s3, teleop_reads_thruster) = sub(&ss["teleop"], "zk2/vehicle-01/thruster-l/thruster.v1/state/status").await?;
    let (_s4, frontend_h2) = sub(&ss["frontend"], "zk2/h2/tc/tc.netif.v1/state/**").await?;
    settle().await;

    ss["teleop"].put("zk2/vehicle-01/teleop/twist_cmd.v1/stream/cmd", "t").await.map_err(|e| anyhow!("{e}"))?;
    settle().await;
    push("teleop puts on its own cmd: the thruster receives it", yn(!from_teleop.lock().unwrap().is_empty()));
    ss["teleop"].put("zk2/vehicle-01/autopilot/twist_cmd.v1/stream/cmd", "impersonated").await.map_err(|e| anyhow!("{e}"))?;
    settle().await;
    push("teleop puts on autopilot's cmd key: the thruster receives it", yn(!from_autopilot.lock().unwrap().is_empty()));
    from_autopilot.lock().unwrap().clear();
    ss["teleop"].put("zk2/vehicle-01/*/twist_cmd.v1/stream/cmd", "wild").await.map_err(|e| anyhow!("{e}"))?;
    settle().await;
    let wild = from_autopilot.lock().unwrap().iter().any(|k| k.contains('*'));
    push("teleop puts on zk2/vehicle-01/*/twist_cmd.v1/stream/cmd: it reaches autopilot's subscription (wildcard key; R6 drops it)", yn(wild));
    ss["thruster-l"].put("zk2/vehicle-01/thruster-l/thruster.v1/state/status", "ok").await.map_err(|e| anyhow!("{e}"))?;
    settle().await;
    push("teleop subscribes to the thruster's status (not granted): it receives it", yn(!teleop_reads_thruster.lock().unwrap().is_empty()));
    let t = Duration::from_secs(1);
    let seen = zk2rt::client::tokens(&ss["thruster-l"], "zk2/vehicle-01/*/@zk/**", t).await?;
    let distinct: std::collections::BTreeSet<&String> = seen.iter().collect();
    push("the thruster sees the commanders' presence (4 distinct tokens)", distinct.len().to_string());
    let seen = zk2rt::client::tokens(&ss["frontend"], "zk2/vehicle-01/*/@zk/**", t).await?.len();
    push("the frontend sees the vehicle's presence (not granted)", seen.to_string());

    // tcgui: two backends, a frontend.
    let mut qs = Vec::new();
    for (h, who) in [("h1", "tc-h1"), ("h2", "tc-h2")] {
        let st = format!("zk2/{h}/tc/tc.netif.v1/state/namespaces");
        let st2 = st.clone();
        qs.push(ss[who].declare_queryable(st.clone()).callback(move |q| { let _ = q.reply(st2.clone(), "ns").wait(); }).await.map_err(|e| anyhow!("{e}"))?);
        let op = format!("zk2/{h}/tc/tc.netem.v1/@op/config/*/set");
        qs.push(
            ss[who]
                .declare_queryable(op)
                .complete(true)
                .callback(move |q| {
                    if q.key_expr().is_wild() {
                        let _ = q.reply_err(r#"{"error":"fanout_forbidden"}"#).encoding(Encoding::APPLICATION_JSON).wait();
                    } else {
                        let k = q.key_expr().clone().into_owned();
                        let _ = q.reply(k, "done").wait();
                    }
                })
                .await
                .map_err(|e| anyhow!("{e}"))?,
        );
    }
    let ck = "zk2/@zk/contract/tc.netif.v1/0f66b421dc6decf2b28bf4b8f0c6006cbdbb7fe0a4d9a997e637f9105dc0ea98";
    qs.push(ss["tc-h1"].declare_queryable(ck).complete(true).callback(move |q| { let _ = q.reply(ck, "{}").wait(); }).await.map_err(|e| anyhow!("{e}"))?);
    settle().await;
    let got = zk2rt::client::state_get(&ss["frontend"], "zk2/*/tc/tc.netif.v1/state/**", t).await?;
    push("the frontend GETs zk2/*/tc/tc.netif.v1/state/** (2 backends)", got.iter().flatten().count().to_string());
    let got = zk2rt::client::call(&ss["frontend"], "zk2/h1/tc/tc.netem.v1/@op/config/eth0/set", (Vec::new(), Encoding::default()), t).await?;
    push("the frontend calls h1's concrete config/eth0/set", got.iter().flatten().count().to_string());
    let got = zk2rt::client::get(&ss["frontend"], "zk2/*/tc/tc.netem.v1/@op/config/*/set", zenoh::query::QueryTarget::All, zenoh::query::ConsolidationMode::None, None, t).await?;
    let refused = got.iter().filter(|r| r.as_ref().is_err_and(|e| e.contains("fanout_forbidden"))).count();
    let values = got.iter().flatten().count();
    push("the frontend's wildcard call (fan-out forbidden): values / server refusals", format!("{values} / {refused}"));
    ss["tc-h1"].put("zk2/h2/tc/tc.netif.v1/state/namespaces", "impersonated").await.map_err(|e| anyhow!("{e}"))?;
    settle().await;
    push("backend h1 puts on h2's state: the frontend's subscription receives it", yn(!frontend_h2.lock().unwrap().is_empty()));
    let got = zk2rt::client::get(&ss["frontend"], ck, zenoh::query::QueryTarget::BestMatching, zenoh::query::ConsolidationMode::None, None, t).await?;
    push("the frontend fetches a contract bundle from h1", got.iter().flatten().count().to_string());

    // Offline commanding.
    for v in ["vehicle-01", "vehicle-02"] {
        let k = format!("zk2/ground/fleet-mgr/desired.v1/state/plans/{v}");
        let k2 = k.clone();
        qs.push(ss["fleet-mgr"].declare_queryable(k.clone()).callback(move |q| { let _ = q.reply(k2.clone(), "plan").wait(); }).await.map_err(|e| anyhow!("{e}"))?);
    }
    settle().await;
    let a = zk2rt::client::state_get(&ss["executor"], "zk2/ground/fleet-mgr/desired.v1/state/plans/vehicle-01", t).await?.iter().flatten().count();
    let b = zk2rt::client::state_get(&ss["executor"], "zk2/ground/fleet-mgr/desired.v1/state/plans/vehicle-02", t).await?.iter().flatten().count();
    push("vehicle-01's executor GETs its plan / vehicle-02's plan", format!("{a} / {b}"));

    // An unauthenticated client.
    let anon = open_as(&ep, None).await;
    push("a client with no credentials connects", yn(anon.is_ok()));
    Ok(())
}

pub async fn run(results: &Path) -> Result<bool> {
    std::fs::create_dir_all(results)?;
    let mut rows = Vec::new();
    let (g_rules, g_pol) = grants();
    let deny: &dyn Fn(&str) -> &'static str = &|c: &str| match c {
        c if c.starts_with("teleop puts on its own") => "yes",
        c if c.starts_with("teleop puts on autopilot's") => "no",
        c if c.starts_with("teleop puts on zk2/vehicle-01/*") => "no",
        c if c.starts_with("teleop subscribes") => "no",
        c if c.starts_with("the thruster sees") => "4",
        c if c.starts_with("the frontend sees the vehicle") => "0",
        c if c.starts_with("the frontend GETs") => "2",
        c if c.starts_with("the frontend calls") => "1",
        c if c.starts_with("the frontend's wildcard") => "0 / 2",
        c if c.starts_with("backend h1 puts") => "no",
        c if c.starts_with("the frontend fetches") => "1",
        c if c.starts_with("vehicle-01's executor") => "1 / 0",
        c if c.starts_with("a client with no credentials") => "no",
        _ => "?",
    };
    posture("deny + allows", acl("deny", g_rules.clone(), g_pol.clone()), results, &mut rows, deny).await?;
    // U21: zenoh's source says allow rules are not evaluated under default
    // allow, so every unauthorized action is expected to get through.
    let allow_naive: &dyn Fn(&str) -> &'static str = &|c: &str| match c {
        c if c.starts_with("teleop puts on autopilot's") => "yes (allows are not evaluated)",
        c if c.starts_with("teleop subscribes") => "yes (allows are not evaluated)",
        c if c.starts_with("backend h1 puts") => "yes (allows are not evaluated)",
        c if c.starts_with("vehicle-01's executor") => "1 / 1 (allows are not evaluated)",
        c if c.starts_with("the frontend sees the vehicle") => "4 (allows are not evaluated)",
        c if c.starts_with("teleop puts on zk2/vehicle-01/*") => "yes (allows are not evaluated)",
        c => deny(c),
    };
    posture("allow + the same allows (U21)", acl("allow", g_rules, g_pol), results, &mut rows, allow_naive).await?;
    let (c_rules, c_pol) = complement();
    let comp: &dyn Fn(&str) -> &'static str = &|c: &str| match c {
        c if c.starts_with("teleop puts on zk2/vehicle-01/*") => "yes (a wildcard is not included in any deny; R6 drops it)",
        c => deny(c),
    };
    posture("allow + denies of the complement (D13)", acl("allow", c_rules, c_pol), results, &mut rows, comp).await?;

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s14.csv");
    let mut md = format!("# S14 — the ownership ACL (#616)\n\nWritten by `spike s14`; the latest run (`unix_s` {unix}), zenoh {}. usrpwd subjects; the generated router configs are beside this file.\n\n| Posture | Check | Expected | Observed | Match |\n|---|---|---|---|---|\n", zk2rt::ZENOH_VERSION);
    let mut all = true;
    for r in &rows {
        let ok = r.expected.starts_with(&r.observed) || r.expected == r.observed;
        all &= ok;
        csv_row(&csv, &["unix_s", "zenoh", "posture", "check", "expected", "observed", "match"], &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.posture.into(), r.check.into(), r.expected.clone(), r.observed.clone(), ok.to_string()])?;
        md.push_str(&format!("| {} | {} | {} | {} | {} |\n", r.posture, r.check, r.expected, r.observed, if ok { "yes" } else { "**no**" }));
        println!("{:5} [{}] {} → expected {} observed {}", if ok { "ok" } else { "DIFF" }, r.posture, r.check, r.expected, r.observed);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(all)
}
