//! S1, grammar basics (#597, r3 §7). v1's verbatim `@v1` broke advanced
//! pub/sub silently (RFC 03 §1.2); this makes sure zk2's grammar has no such
//! trap. Every case is one CSV row: group, case, expected, observed, pass.
//!
//! - **roundtrip** (offline): every resource of every example contract,
//!   built with slugged values, parsed back, and resolved to its template
//!   with the same values. Control keys too.
//! - **guard** (offline, then live): `zk2/<system>/**` never matches an
//!   `@stream`, `@state`, `@op` or `@zk` key.
//! - **namespace** (live): a session namespace × each kind token, for
//!   prefixing on egress, stripping on ingress, and isolation.
//! - **adv** (live): zenoh-ext's advanced publisher and subscriber (cache,
//!   late-joiner history) on `stream`, `state` and `@stream` keys, with and
//!   without a namespace.
//! - **wildcard-put** (live): a put on a wildcard key, what subscribers see,
//!   and the cost of the consumer-side "concrete keys only" filter (R6).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use zenkey_model::authoring::{Kind, ParamType};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::grammar::{
    Addr, InstanceId, KindToken, ZkKey, alive_key, contract_key, data_key, instance_key, member_key, parse,
};
use zenkey_model::template::{Bindings, resolve};
use zenoh::key_expr::KeyExpr;
use zenoh_ext::{AdvancedPublisherBuilderExt, AdvancedSubscriberBuilderExt, CacheConfig, HistoryConfig};
use zk2rt::config::{Mode, Topo};
use zk2rt::metrics::csv_row;

use crate::procs;

struct Row {
    group: &'static str,
    case: String,
    expected: String,
    observed: String,
    pass: bool,
}

impl Row {
    fn new(group: &'static str, case: impl Into<String>, expected: impl Into<String>, observed: impl Into<String>) -> Self {
        let (expected, observed) = (expected.into(), observed.into());
        let pass = expected == observed;
        Self { group, case: case.into(), expected, observed, pass }
    }
}

const ULID: &str = "01j9zk6q6x5m2f4a8c0d3e7b9h";
const STRINGS: &[&str] = &["m0", "ETH0", "a/b", "10.0.0.1", "über", "x-y", "-"];

fn contract_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            contract_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "toml")
            && !p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".bindings.toml"))
        {
            out.push(p);
        }
    }
}

/// Bindings for member `i` of a resource, cycling through awkward values.
fn bindings(c: &Contract, ri: usize, i: usize) -> Bindings {
    let r = &c.resources[ri];
    let mut b = Bindings::new();
    for (n, ty) in &r.params {
        let v = match ty {
            ParamType::String => vec![STRINGS[i % STRINGS.len()].to_owned()],
            ParamType::Uint => vec![(i * 7).to_string()],
            ParamType::Path => vec!["if".to_owned(), STRINGS[i % STRINGS.len()].to_owned(), "in_octets".into()],
        };
        b.insert(n.clone(), v);
    }
    b
}

/// The data keys built for a contract, with the token each was built under.
fn roundtrip(c: &Contract, keys: &mut Vec<(String, Option<KindToken>)>) -> Result<(usize, usize, usize)> {
    let addr = Addr::new("h-3fa9c2d41b7e", "svc.dev-1")?;
    let (mut built, mut failed, mut shadowed) = (0, 0, 0);
    for (ri, r) in c.resources.iter().enumerate() {
        let same_token: Vec<_> = c.resources.iter().filter(|x| x.token == r.token).map(|x| x.template.clone()).collect();
        for i in 0..STRINGS.len() {
            let b = bindings(c, ri, i);
            let mut chunks = r.template.build(&b).map_err(|e| anyhow!(e))?;
            if r.kind == Kind::Event {
                chunks.push(ULID.to_owned());
            }
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            let key = data_key(&addr, &c.iface, r.token, &refs)?;
            built += 1;
            keys.push((key.as_str().to_owned(), Some(r.token)));
            let ok = match parse(key.as_str()) {
                Ok(ZkKey::Data { addr: a, iface, kind, resource }) if a == addr && iface == c.iface && kind == r.token => {
                    let res: Vec<&str> = resource.iter().map(String::as_str).collect();
                    let res = if r.kind == Kind::Event { &res[..res.len() - 1] } else { &res[..] };
                    match resolve(&same_token, res) {
                        Some((w, got)) if same_token[w] == r.template => got == b,
                        Some(_) => {
                            // A more literal template wins this key (D1): a
                            // property of the contract, not a parse failure.
                            shadowed += 1;
                            true
                        }
                        None => false,
                    }
                }
                _ => false,
            };
            if !ok {
                failed += 1;
            }
            if !r.template.has_params() {
                break;
            }
        }
    }
    let inst = InstanceId::from_u64(0x0123_4567_89ab_cdef);
    let fp = Fingerprint::of(c);
    let control = [
        instance_key(&addr, &inst)?,
        alive_key(&addr, &c.iface, &inst, &fp.hex().fp16())?,
        member_key(&addr, &c.iface, "cam0", &inst)?,
        contract_key(&c.iface, fp.hex())?,
    ];
    for k in control {
        built += 1;
        keys.push((k.as_str().to_owned(), None));
        let back = parse(k.as_str()).and_then(|z| z.to_key());
        if back.map(|b| b.as_str() != k.as_str()).unwrap_or(true) {
            failed += 1;
        }
    }
    Ok((built, failed, shadowed))
}

fn guard_offline(keys: &[(String, Option<KindToken>)]) -> Vec<Row> {
    let sel = KeyExpr::try_from("zk2/h-3fa9c2d41b7e/**").expect("valid");
    let all = KeyExpr::try_from("zk2/**").expect("valid");
    let (mut viol_sys, mut viol_all, mut plain, mut verbatim) = (0, 0, 0, 0);
    for (k, tok) in keys {
        let ke = KeyExpr::try_from(k.as_str()).expect("built keys are valid");
        let expect = matches!(tok, Some(KindToken::Stream | KindToken::State | KindToken::Events));
        if expect {
            plain += 1;
        } else {
            verbatim += 1;
        }
        if sel.intersects(&ke) != expect {
            viol_sys += 1;
        }
        if all.intersects(&ke) != expect {
            viol_all += 1;
        }
    }
    vec![
        Row::new(
            "guard",
            format!("zk2/<system>/** over {plain} plain and {verbatim} verbatim/control keys"),
            "0 violations",
            format!("{viol_sys} violations"),
        ),
        Row::new("guard", "zk2/** over the same keys", "0 violations", format!("{viol_all} violations")),
    ]
}

async fn drain(sub: &zenoh::pubsub::Subscriber<zenoh::handlers::FifoChannelHandler<zenoh::sample::Sample>>) -> Vec<String> {
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut out = Vec::new();
    while let Ok(Some(s)) = sub.try_recv() {
        out.push(s.key_expr().as_str().to_owned());
    }
    out.sort();
    out
}

fn show(v: &[String]) -> String {
    if v.is_empty() { "none".to_owned() } else { v.join(" ") }
}

async fn get_keys(s: &zenoh::Session, sel: &str) -> Result<Vec<String>> {
    let got = zk2rt::client::get(
        s,
        sel,
        zenoh::query::QueryTarget::All,
        zenoh::query::ConsolidationMode::None,
        None,
        Duration::from_secs(1),
    )
    .await?;
    let mut v: Vec<String> = got.into_iter().flatten().map(|g| g.key).collect();
    v.sort();
    Ok(v)
}

async fn namespace(ep: &str) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    let mk = |ns: Option<&str>| Topo {
        mode: Mode::Client,
        listen: Vec::new(),
        connect: vec![ep.to_owned()],
        namespace: ns.map(str::to_owned),
    };
    let p = mk(Some("dep1")).open().await?;
    let c_ns = mk(Some("dep1")).open().await?;
    let c_raw = mk(None).open().await?;
    let c_other = mk(Some("dep2")).open().await?;

    for tok in ["stream", "@stream", "state", "@state"] {
        let key = format!("zk2/ns/svc/i.v1/{tok}/x");
        let s_ns = c_ns.declare_subscriber(key.clone()).await.map_err(|e| anyhow!("{e}"))?;
        let s_raw = c_raw.declare_subscriber(format!("dep1/{key}")).await.map_err(|e| anyhow!("{e}"))?;
        let s_raw_wild = c_raw.declare_subscriber("dep1/zk2/**").await.map_err(|e| anyhow!("{e}"))?;
        let s_other = c_other.declare_subscriber("zk2/**").await.map_err(|e| anyhow!("{e}"))?;
        let s_other_exact = c_other.declare_subscriber(key.clone()).await.map_err(|e| anyhow!("{e}"))?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        p.put(key.clone(), "v").await.map_err(|e| anyhow!("{e}"))?;
        let plain = !tok.starts_with('@');
        rows.push(Row::new("namespace", format!("{tok}: same-namespace subscriber sees the key stripped"), key.clone(), show(&drain(&s_ns).await)));
        rows.push(Row::new("namespace", format!("{tok}: un-namespaced subscriber sees the key prefixed"), format!("dep1/{key}"), show(&drain(&s_raw).await)));
        rows.push(Row::new(
            "namespace",
            format!("{tok}: un-namespaced dep1/zk2/** (guard under a prefix)"),
            if plain { format!("dep1/{key}") } else { "none".into() },
            show(&drain(&s_raw_wild).await),
        ));
        let mut other = drain(&s_other).await;
        other.extend(drain(&s_other_exact).await);
        rows.push(Row::new("namespace", format!("{tok}: another namespace sees nothing"), "none", show(&other)));
    }

    // @op: a complete queryable under the namespace.
    let op = "zk2/ns/svc/i.v1/@op/f";
    let _q = p
        .declare_queryable(op)
        .complete(true)
        .callback(move |q| {
            use zenoh::Wait;
            let _ = q.reply(op, "r").wait();
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    rows.push(Row::new("namespace", "@op: same-namespace call, reply key stripped", op, show(&get_keys(&c_ns, op).await?)));
    rows.push(Row::new(
        "namespace",
        "@op: un-namespaced call on the prefixed key",
        format!("dep1/{op}"),
        show(&get_keys(&c_raw, &format!("dep1/{op}")).await?),
    ));
    rows.push(Row::new("namespace", "@op: another namespace gets no reply", "none", show(&get_keys(&c_other, op).await?)));

    // @zk: liveliness tokens and the descriptor queryable.
    let tok = "zk2/ns/svc/@zk/instance/0123456789abcdef";
    let _t = p.liveliness().declare_token(tok).await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t = Duration::from_secs(1);
    rows.push(Row::new(
        "namespace",
        "@zk: same-namespace liveliness GET zk2/*/*/@zk/instance/*",
        tok,
        show(&zk2rt::client::tokens(&c_ns, "zk2/*/*/@zk/instance/*", t).await?),
    ));
    rows.push(Row::new(
        "namespace",
        "@zk: un-namespaced liveliness GET dep1/zk2/*/*/@zk/instance/*",
        format!("dep1/{tok}"),
        show(&zk2rt::client::tokens(&c_raw, "dep1/zk2/*/*/@zk/instance/*", t).await?),
    ));
    rows.push(Row::new(
        "namespace",
        "@zk: another namespace sees no token",
        "none",
        show(&zk2rt::client::tokens(&c_other, "zk2/*/*/@zk/instance/*", t).await?),
    ));
    Ok(rows)
}

async fn guard_live(ep: &str) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    let p = Topo::client(&[ep.to_owned()]).open().await?;
    let c = Topo::client(&[ep.to_owned()]).open().await?;
    let sub = c.declare_subscriber("zk2/g/**").await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    for tok in ["stream", "state", "events", "@stream", "@state"] {
        let key = if tok == "events" { format!("zk2/g/svc/i.v1/events/x/{ULID}") } else { format!("zk2/g/svc/i.v1/{tok}/x") };
        p.put(key, "v").await.map_err(|e| anyhow!("{e}"))?;
    }
    rows.push(Row::new(
        "guard",
        "live: subscriber zk2/g/** over puts on stream, state, events, @stream, @state",
        format!("zk2/g/svc/i.v1/events/x/{ULID} zk2/g/svc/i.v1/state/x zk2/g/svc/i.v1/stream/x"),
        show(&drain(&sub).await),
    ));
    use zenoh::Wait;
    let _qo = p
        .declare_queryable("zk2/g/svc/i.v1/@op/f")
        .complete(true)
        .callback(|q| {
            let _ = q.reply("zk2/g/svc/i.v1/@op/f", "r").wait();
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let _qs = p
        .declare_queryable("zk2/g/svc/i.v1/state/**")
        .callback(|q| {
            let _ = q.reply("zk2/g/svc/i.v1/state/x", "v").wait();
        })
        .await
        .map_err(|e| anyhow!("{e}"))?;
    let _t = p.liveliness().declare_token("zk2/g/svc/@zk/instance/0123456789abcdef").await.map_err(|e| anyhow!("{e}"))?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    rows.push(Row::new("guard", "live: GET zk2/g/** reaches state, never @op", "zk2/g/svc/i.v1/state/x", show(&get_keys(&c, "zk2/g/**").await?)));
    let t = Duration::from_secs(1);
    rows.push(Row::new("guard", "live: liveliness GET zk2/g/** sees no @zk token", "none", show(&zk2rt::client::tokens(&c, "zk2/g/**", t).await?)));
    rows.push(Row::new(
        "guard",
        "live: liveliness GET zk2/g/*/@zk/** names it",
        "zk2/g/svc/@zk/instance/0123456789abcdef",
        show(&zk2rt::client::tokens(&c, "zk2/g/*/@zk/**", t).await?),
    ));
    Ok(rows)
}

async fn adv(ep: &str) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for ns in [None, Some("dep1")] {
        let label = ns.map_or("no namespace".to_owned(), |n| format!("namespace {n}"));
        let mk = || Topo { mode: Mode::Client, listen: Vec::new(), connect: vec![ep.to_owned()], namespace: ns.map(str::to_owned) };
        for tok in ["stream", "state", "@stream"] {
            // (a) History from a publisher already present: the subscriber's
            // initial query on `<key>/@adv/**`. No token key is parsed.
            let p = mk().open().await?;
            let c = mk().open().await?;
            let key = format!("zk2/adv/svc/i.v1/{tok}/a");
            let publ = p
                .declare_publisher(key.clone())
                .cache(CacheConfig::default().max_samples(10))
                .publisher_detection()
                .await
                .map_err(|e| anyhow!("adv publisher on {key}: {e}"))?;
            for i in 0..5 {
                publ.put(format!("v{i}")).await.map_err(|e| anyhow!("{e}"))?;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            let sub = c
                .declare_subscriber(key.clone())
                .history(HistoryConfig::default().detect_late_publishers())
                .await
                .map_err(|e| anyhow!("adv subscriber on {key}: {e}"))?;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let mut n = 0;
            while let Ok(Some(_)) = sub.try_recv() {
                n += 1;
            }
            let adv_tokens = zk2rt::client::tokens(&c, &format!("{key}/@adv/**"), Duration::from_secs(1)).await?;
            let refused = adv_tokens.iter().all(|t| parse(t).is_err());
            rows.push(Row::new(
                "adv",
                format!("{tok}, {label}: (a) history from a publisher already present (initial query, no key parsing)"),
                "5",
                n.to_string(),
            ));
            rows.push(Row::new(
                "adv",
                format!("{tok}, {label}: @adv tokens seen, and refused by the zk2 parser"),
                "1 token, refused",
                format!("{} token{}, {}", adv_tokens.len(), if adv_tokens.len() == 1 { "" } else { "s" }, if refused { "refused" } else { "ACCEPTED" }),
            ));
            drop((sub, publ));

            // (b) A late-joining publisher: the subscriber is first, and the
            // publisher's puts stay session-local, so samples arrive only
            // if the subscriber parses the publisher's @adv liveliness token
            // (`${remaining:**}/@adv/…`) and queries its cache. `**` cannot
            // cross a verbatim chunk, so r3.2 predicts 0 under `@stream`.
            let key = format!("zk2/adv/svc/i.v1/{tok}/b");
            let sub = c
                .declare_subscriber(key.clone())
                .history(HistoryConfig::default().detect_late_publishers())
                .await
                .map_err(|e| anyhow!("adv subscriber on {key}: {e}"))?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let publ = p
                .declare_publisher(key.clone())
                .cache(CacheConfig::default().max_samples(10))
                .publisher_detection()
                .allowed_destination(zenoh::sample::Locality::SessionLocal)
                .await
                .map_err(|e| anyhow!("adv publisher on {key}: {e}"))?;
            for i in 0..5 {
                publ.put(format!("v{i}")).await.map_err(|e| anyhow!("{e}"))?;
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let mut n = 0;
            while let Ok(Some(_)) = sub.try_recv() {
                n += 1;
            }
            let expect = if tok == "@stream" { "0" } else { "5" };
            rows.push(Row::new(
                "adv",
                format!("{tok}, {label}: (b) late-joining publisher, detected by parsing its @adv token"),
                expect,
                n.to_string(),
            ));
        }
    }
    Ok(rows)
}

async fn wildcard_put(ep: &str) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    let p = Topo::client(&[ep.to_owned()]).open().await?;
    let c = Topo::client(&[ep.to_owned()]).open().await?;
    for tok in ["stream", "@stream"] {
        let exact = c.declare_subscriber(format!("zk2/w/svc/i.v1/{tok}/x")).await.map_err(|e| anyhow!("{e}"))?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let wild = format!("zk2/w/svc/i.v1/{tok}/*");
        p.put(wild.clone(), "injected").await.map_err(|e| anyhow!("{e}"))?;
        rows.push(Row::new(
            "wildcard-put",
            format!("{tok}: put on {wild} reaches a concrete subscriber, which sees the wildcard key"),
            wild.clone(),
            show(&drain(&exact).await),
        ));
    }
    // The R6 filter: one `is_wild` check per sample.
    let keys: Vec<KeyExpr<'static>> = (0..1000)
        .map(|i| KeyExpr::try_from(format!("zk2/h-3fa9c2d41b7e/sysinfo/zs.sysinfo.v1/stream/cpu/{i}/usage_pct")).unwrap())
        .collect();
    let n = 10_000_000usize;
    let start = Instant::now();
    let mut wild = 0usize;
    for i in 0..n {
        if std::hint::black_box(&keys[i % keys.len()]).is_wild() {
            wild += 1;
        }
    }
    let ns = start.elapsed().as_nanos() as f64 / n as f64;
    rows.push(Row::new(
        "wildcard-put",
        format!("R6 filter cost: is_wild() on a 66-byte concrete key, {n} checks"),
        "0 wild",
        format!("{wild} wild; {ns:.2} ns/check"),
    ));
    // The observed column carries a measurement; pass is on the count only.
    if let Some(r) = rows.last_mut() {
        r.pass = wild == 0;
    }
    Ok(rows)
}

/// Runs S1 and writes `results/s1/s1.csv` and `summary.md`. Ok(true) when
/// every case passed.
pub async fn run(results: &Path, examples: &Path) -> Result<bool> {
    let mut rows = Vec::new();
    let mut files = Vec::new();
    contract_files(examples, &mut files);
    files.sort();
    let mut keys = Vec::new();
    let (mut tb, mut tf, mut ts) = (0, 0, 0);
    for f in &files {
        let l = load_path(f);
        let c = l.contract.ok_or_else(|| anyhow!("{}: {}", f.display(), l.report))?;
        let (b, fl, sh) = roundtrip(&c, &mut keys)?;
        tb += b;
        tf += fl;
        ts += sh;
    }
    rows.push(Row::new(
        "roundtrip",
        format!("{} contracts: build, parse, resolve, unslug (values incl. ETH0, a/b, 10.0.0.1, über, -)", files.len()),
        "0 failures",
        format!("0 failures"),
    ));
    if let Some(r) = rows.last_mut() {
        r.observed = format!("{tf} failures of {tb} keys; {ts} shadowed by a more literal template (D1)");
        r.pass = tf == 0;
    }
    rows.extend(guard_offline(&keys));

    let (_router, ep) = procs::router(&[]).await?;
    rows.extend(guard_live(&ep).await?);
    rows.extend(namespace(&ep).await?);
    rows.extend(adv(&ep).await?);
    rows.extend(wildcard_put(&ep).await?);

    let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs().to_string();
    let csv = results.join("s1.csv");
    let header = ["unix_s", "zenoh", "group", "case", "expected", "observed", "pass"];
    for r in &rows {
        csv_row(
            &csv,
            &header,
            &[unix.clone(), zk2rt::ZENOH_VERSION.into(), r.group.into(), r.case.clone(), r.expected.clone(), r.observed.clone(), r.pass.to_string()],
        )?;
    }
    let mut md = format!("# S1 — grammar basics (#597)\n\nWritten by `spike s1`; the latest run (`unix_s` {unix}), zenoh {}.\n\n| Group | Case | Expected | Observed | Pass |\n|---|---|---|---|---|\n", zk2rt::ZENOH_VERSION);
    for r in &rows {
        md.push_str(&format!("| {} | {} | {} | {} | {} |\n", r.group, r.case.replace('|', "\\|"), r.expected, r.observed, if r.pass { "yes" } else { "**no**" }));
        println!("{:5} {:12} {} → {}", if r.pass { "ok" } else { "FAIL" }, r.group, r.case, r.observed);
    }
    std::fs::write(results.join("summary.md"), md)?;
    Ok(rows.iter().all(|r| r.pass))
}
