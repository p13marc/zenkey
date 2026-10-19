//! `link.v1`'s scenarios, run against live routers
//! (`spec/profiles/link/scenarios.md` §1–§10, text 0.4; #769, LB, LC and
//! LD): one test per section, named after it. §7 runs under both postures,
//! §8 gives the generator two revisions of `radio.v1`, and §9 and §10 run
//! hand edits of the generator's block that §2.15's subject checks refuse
//! (0.4, F-120), each with what it leaks.
//!
//! The generator is the reference's: `plan_acl` over the example of
//! `examples/zk2/link/` (the `radio.v1` contract, its enrollment and the
//! lab router's face policy), rendered as `zenctl acl gen --face-policy
//! --json5` writes it and pasted into N, a zenoh 1.10.1 router with
//! timestamping on (a router's default) and usrpwd authentication. N listens
//! on `tcp/127.0.0.1:0`, the near side, and on a `unixsock-stream` socket,
//! the face, which is what the downsampling block selects (`link.v1` §2.14).
//! Every session authenticates as a user of its own, under `deny` unless a
//! section says `allow`; §7's X presents `local`'s credentials over the
//! face, which is the point (0.3, F-118).
//!
//! **The readers subscribe to what they are granted** (0.2, F-116): under
//! `deny`, zenoh admits a subscription or a GET only when a grant includes
//! its key expression (core §11.3), so G and L subscribe to the console
//! tool's Consume selectors, read off the plan, never to `zk2/lab/*/**`.
//!
//! **The owner** is a plain session, so what is judged is the router's
//! block and not a runtime's discipline; it serves its state as core S2
//! says, one queryable over `…/radio.v1/state/**`, which is what makes
//! §2's declaration grant matter (core §8.5, 0.30; F-117). §5's owner is the
//! runtime's own (`zenkey::ServiceBuilder`), because the tombstone window it
//! keeps, `tombstone_window_s` (core S3), is the thing judged.
//!
//! What a test cannot judge, it does not claim: the batch against the lease
//! (§2.10) is a linter's, and nothing here runs a 2,400 bit/s link.

mod util;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use util::zk2::{T, eventually};
use zenkey_fleet::report::{
    AclConfigDoc, AclPermission, AclPlan, AclPolicy, AclSubject, DownsamplingDoc, Enrollment,
};
use zenkey_fleet::{AclOptions, ContractSet, acl_plan_json5, check_face, plan_acl};
use zenkey_model::freshness::{self, Last, Observation, Verdict};
use zenkey_model::link;
use zenoh::Wait;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::SampleKind;

/// Every user N knows, password `<user>-pw`.
const USERS: [&str; 6] = [
    "router",
    "radio-a",
    "radio-b",
    "local",
    "ground",
    "ground-router",
];

/// How long a reader waits for what must not arrive, once it has seen what
/// must.
const SILENCE: Duration = Duration::from_millis(1500);

fn dir() -> PathBuf {
    util::zk2::examples().join("link")
}

fn contracts() -> ContractSet {
    let (set, problems) = ContractSet::load_path(&dir().join("radio.v1.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    set
}

fn enrollment() -> Enrollment {
    toml::from_str(&std::fs::read_to_string(dir().join("radio.enrollment.toml")).unwrap()).unwrap()
}

/// The scenarios' face policy (`radio.face.toml`), each edit applied.
fn policy(edits: &[(&str, &str)]) -> String {
    let mut text = std::fs::read_to_string(dir().join("radio.face.toml")).unwrap();
    for (from, to) in edits {
        assert!(text.contains(from), "{from:?} in the face policy");
        text = text.replace(from, to);
    }
    text
}

const SOUTH: [(&str, &str); 2] = [
    (
        "attach       = \"client\"\n",
        "attach       = \"south-region\"\nregion       = \"ground\"\n",
    ),
    (
        "far          = \"ground\"",
        "far          = \"ground-router\"",
    ),
];
const BOTH_LISTED: (&str, &str) = (
    "services = [\"radio-a\"]",
    "services = [\"radio-a\", \"radio-b\"]",
);
const CLOSED: (&str, &str) = (
    "expose       = \"link\"\ninterval_s   = 1\n",
    "expose       = \"none\"\n",
);

/// The generator: the block for `policy` from `contracts`, under `posture`.
fn plan_with(policy: &str, contracts: &ContractSet, posture: AclPermission) -> AclPlan {
    let plan = plan_acl(
        &enrollment(),
        contracts,
        &AclOptions {
            default_permission: posture,
            face_policy: Some(link::parse(policy)),
            ..AclOptions::default()
        },
    )
    .expect("the radio example plans");
    assert!(plan.refusals.is_empty(), "{:#?}", plan.refusals);
    plan
}

/// The generator: the block for `policy`, under `deny`.
fn plan(policy: &str) -> AclPlan {
    plan_with(policy, &contracts(), AclPermission::Deny)
}

/// A tool checking the block N runs (§2.15): the fragment `block` wrote,
/// read back by zenoh's loader, against the face policy and the plan the
/// generator makes of it. A hand edit of `block` is what the comparison
/// finds.
fn check(
    block: &AclPlan,
    policy: &str,
    windows: Option<&BTreeMap<String, u64>>,
) -> zenkey_fleet::report::FaceCheck {
    check_against(block, &self::plan(policy), policy, windows)
}

/// [`check`], against the plan `planned`.
fn check_against(
    block: &AclPlan,
    planned: &AclPlan,
    policy: &str,
    windows: Option<&BTreeMap<String, u64>>,
) -> zenkey_fleet::report::FaceCheck {
    let block = acl_plan_json5(block);
    let c = zenoh::Config::from_json5(&format!("{{\n{block}}}")).unwrap();
    let acl: AclConfigDoc = serde_json::from_str(&c.get_json("access_control").unwrap()).unwrap();
    let ds: Vec<DownsamplingDoc> = c
        .get_json("downsampling")
        .ok()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_default();
    let gw = c
        .get_json("gateway")
        .ok()
        .and_then(|g| serde_json::from_str(&g).ok());
    let parsed = link::parse(policy);
    check_face(
        planned,
        parsed.policy.as_ref().unwrap(),
        &acl,
        &ds,
        gw.as_ref(),
        windows,
        "N.json5",
    )
}

fn scratch(name: &str) -> PathBuf {
    static NTH: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "zk-lb-{}-{}-{name}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ))
}

/// N, running `plan`'s block: its near-side and face endpoints.
struct Near {
    _router: zenoh::Session,
    tcp: String,
    face: String,
    files: Vec<PathBuf>,
}

impl Drop for Near {
    fn drop(&mut self) {
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
    }
}

async fn near(plan: &AclPlan) -> Near {
    let dict = scratch("near.txt");
    std::fs::write(
        &dict,
        USERS
            .iter()
            .map(|u| format!("{u}:{u}-pw\n"))
            .collect::<String>(),
    )
    .unwrap();
    let sock = scratch("face.sock");
    let face = format!("unixsock-stream/{}", sock.display());
    let text = format!(
        "{{\n  mode: \"router\",\n  scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }},\n  \
         listen: {{ endpoints: [\"{}\", \"{face}\"] }},\n  \
         transport: {{ auth: {{ usrpwd: {{ user: \"router\", password: \"router-pw\", dictionary_file: {:?} }} }} }},\n{}}}\n",
        util::ANY_PORT,
        dict.display().to_string(),
        acl_plan_json5(plan)
    );
    let router = zenoh::open(
        zenoh::Config::from_json5(&text)
            .unwrap_or_else(|e| panic!("N's config does not parse: {e}\n{text}")),
    )
    .await
    .expect("N runs the block");
    let tcp = util::bound(&router).await;
    Near {
        _router: router,
        tcp,
        face,
        files: vec![dict, sock],
    }
}

fn base(mode: &str) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", &format!("{mode:?}")).unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c
}

/// A client of the router at `ep`, as `user`.
async fn client(ep: &str, user: &str) -> zenoh::Session {
    let mut c = base("client");
    c.insert_json5("connect/endpoints", &format!("[\"{ep}\"]"))
        .unwrap();
    c.insert_json5(
        "transport/auth/usrpwd",
        &format!("{{ user: \"{user}\", password: \"{user}-pw\" }}"),
    )
    .unwrap();
    zenoh::open(c)
        .await
        .unwrap_or_else(|e| panic!("{user} connects to {ep}: {e}"))
}

/// F (§2): a router naming its region `ground`, attached to N over the
/// face as `ground-router`, with G a client of it.
struct Far {
    _router: zenoh::Session,
    tcp: String,
    dict: PathBuf,
}

impl Drop for Far {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.dict);
    }
}

async fn far(n: &Near) -> Far {
    // A router presenting usrpwd credentials checks them on its own links
    // too, so it holds a dictionary: N's, and its client's.
    let dict = scratch("far.txt");
    std::fs::write(&dict, "router:router-pw\nground:ground-pw\n").unwrap();
    let mut c = base("router");
    c.insert_json5("region_name", "\"ground\"").unwrap();
    c.insert_json5("listen/endpoints", &format!("[\"{}\"]", util::ANY_PORT))
        .unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{}\"]", n.face))
        .unwrap();
    c.insert_json5(
        "transport/auth/usrpwd",
        &format!(
            "{{ user: \"ground-router\", password: \"ground-router-pw\", dictionary_file: {:?} }}",
            dict.display().to_string()
        ),
    )
    .unwrap();
    let router = zenoh::open(c).await.expect("F attaches to N over the face");
    let tcp = util::bound(&router).await;
    Far {
        _router: router,
        tcp,
        dict,
    }
}

/// Every delivery a reader records: its arrival, key and kind.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(Instant, String, SampleKind)>>>);

impl Seen {
    /// Puts whose key contains `needle`.
    fn puts(&self, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, k, kind)| *kind == SampleKind::Put && k.contains(needle))
            .count()
    }

    fn last(&self, needle: &str) -> Option<Instant> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, k, _)| k.contains(needle))
            .map(|(t, _, _)| *t)
            .max()
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// A reader's subscribers, on exactly the selectors its grants name (the
/// console's Consume, F-116), or on `only` of them.
struct Reader {
    seen: Seen,
    _subs: Vec<zenoh::pubsub::Subscriber<()>>,
}

async fn reader(s: &zenoh::Session, plan: &AclPlan, only: Option<&[String]>) -> Reader {
    let seen = Seen::default();
    let selectors: Vec<String> = match only {
        Some(o) => o.to_vec(),
        None => plan
            .rules
            .iter()
            .find(|r| r.id == "consume-in:tool.console")
            .expect("the console's Consume")
            .key_exprs
            .clone(),
    };
    let mut subs = Vec::new();
    for sel in selectors {
        let s2 = seen.clone();
        subs.push(
            s.declare_subscriber(sel)
                .callback(move |sample| {
                    s2.0.lock().unwrap().push((
                        Instant::now(),
                        sample.key_expr().to_string(),
                        sample.kind(),
                    ));
                })
                .await
                .unwrap(),
        );
    }
    Reader { seen, _subs: subs }
}

/// A plain owner: its instance and interface tokens, its descriptor, its
/// state queryable over `…/radio.v1/state/**` (core S2) answering
/// `peers/x`, and `reset`.
struct Owner {
    s: zenoh::Session,
    base: String,
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
    _qs: Vec<zenoh::query::Queryable<()>>,
}

async fn owner(ep: &str, name: &str) -> Owner {
    let s = client(ep, name).await;
    let base = format!("zk2/lab/{name}");
    let instance = format!("{base}/@zk/instance/00000000000000a1");
    let tokens = vec![
        s.liveliness().declare_token(&instance).await.unwrap(),
        s.liveliness()
            .declare_token(format!(
                "{base}/@zk/alive/radio.v1/00000000000000a1/0000000000000000"
            ))
            .await
            .unwrap(),
    ];
    let reply = |key: String, value: &'static str| {
        move |q: zenoh::query::Query| {
            let _ = q.reply(&key, value).wait();
        }
    };
    let qs = vec![
        s.declare_queryable(&instance)
            .complete(true)
            .callback(reply(instance.clone(), "{}"))
            .await
            .unwrap(),
        s.declare_queryable(format!("{base}/radio.v1/state/**"))
            .callback(reply(format!("{base}/radio.v1/state/peers/x"), "px"))
            .await
            .unwrap(),
        s.declare_queryable(format!("{base}/radio.v1/@op/reset"))
            .complete(true)
            .callback(reply(format!("{base}/radio.v1/@op/reset"), "reset"))
            .await
            .unwrap(),
    ];
    Owner {
        s,
        base,
        _tokens: tokens,
        _qs: qs,
    }
}

impl Owner {
    async fn put(&self, rel: &str, payload: impl Into<zenoh::bytes::ZBytes>) {
        self.s
            .put(format!("{}/radio.v1/{rel}", self.base), payload)
            .await
            .unwrap();
    }

    /// Puts `oper` until every reader has one, then clears them all: the
    /// subscriptions stand, across N and F, before the traffic starts.
    async fn settle(&self, ready: &[&Seen], clear: &[&Seen]) {
        eventually("the readers' subscriptions stand", || async {
            self.put("state/oper", "probe").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
            ready.iter().all(|s| s.puts("/state/oper") > 0)
        })
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        for s in ready.iter().chain(clear) {
            s.clear();
        }
    }
}

/// An event occurrence's key chunk: a ULID's shape (26 Crockford digits).
fn ulid(n: u64) -> String {
    format!("01J{n:023}")
}

/// The owner's traffic (Conventions): for 5 s, the two streams at 20 Hz,
/// the two `@stream`s at 20 Hz with 1 KB payloads, `oper` 10 times, half a
/// second apart, `peers/x` once, and `fault` and `change` 5 times each in
/// the first half second.
async fn traffic(o: &Owner) {
    let kb = vec![0u8; 1024];
    for i in 0..100u64 {
        o.put("stream/air/dropped_total", i.to_string()).await;
        o.put("stream/air/frames_total", i.to_string()).await;
        o.put("@stream/frames", kb.clone()).await;
        o.put("@stream/raw", kb.clone()).await;
        if i % 10 == 0 {
            o.put("state/oper", format!("v{i}")).await;
        }
        if i == 0 {
            o.put("state/peers/x", "px").await;
        }
        if i < 10 {
            let ev = if i % 2 == 0 { "fault" } else { "change" };
            o.put(&format!("events/{ev}/{}", ulid(i)), "e").await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The value replies to a GET, or a call (`BestMatching`, core O1).
async fn values(s: &zenoh::Session, selector: &str, call: bool) -> usize {
    let rx = s
        .get(selector)
        .target(if call {
            QueryTarget::BestMatching
        } else {
            QueryTarget::All
        })
        .consolidation(ConsolidationMode::None)
        .timeout(T)
        .await
        .expect("a get is sent");
    let mut n = 0;
    while let Ok(r) = rx.recv_async().await {
        n += usize::from(r.result().is_ok());
    }
    n
}

/// A GET or a call, retried for a while: `true` once one is answered.
async fn answered(s: &zenoh::Session, selector: &str, call: bool, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if values(s, selector, call).await > 0 {
            return true;
        }
    }
    false
}

const PEERS_X: &str = "zk2/lab/radio-a/radio.v1/state/peers/x";
const RESET: &str = "zk2/lab/radio-a/radio.v1/@op/reset";
const INSTANCE: &str = "zk2/lab/radio-a/@zk/instance/00000000000000a1";

/// What a far reader sees of §1's traffic, and of the requests.
#[derive(Debug, PartialEq, Eq)]
struct Far1 {
    dropped: usize,
    frames: usize,
    frames_total: usize,
    raw: usize,
    oper: usize,
    fault: usize,
    change: usize,
    get: bool,
    call: bool,
    tokens: usize,
    descriptor: usize,
}

/// §1's steps 2 and 3 with `g` across the face and `l` near: the traffic,
/// the GET and the call, the tokens and the descriptor.
async fn one(o: &Owner, g: &zenoh::Session, plan: &AclPlan, l: Option<&zenoh::Session>) -> Far1 {
    let gr = reader(g, plan, None).await;
    let lr = match l {
        Some(l) => Some(reader(l, plan, None).await),
        None => None,
    };
    let mut ready = vec![&gr.seen];
    ready.extend(lr.as_ref().map(|r| &r.seen));
    o.settle(&ready, &[]).await;
    traffic(o).await;
    tokio::time::sleep(SILENCE).await;
    if let Some(lr) = &lr {
        assert_near(&lr.seen);
    }
    far1(g, &gr.seen, Duration::from_secs(5)).await
}

/// L receives every sample, put and occurrence of the owner's traffic.
fn assert_near(l: &Seen) {
    let r = "radio-a/radio.v1/";
    for res in [
        "stream/air/dropped_total",
        "stream/air/frames_total",
        "@stream/frames",
        "@stream/raw",
    ] {
        let n = l.puts(&format!("{r}{res}"));
        assert!(n >= 80, "L receives about 100 of {res}: {n}");
    }
    assert_eq!(l.puts(&format!("{r}state/oper")), 10);
    assert_eq!(l.puts(&format!("{r}events/fault/")), 5);
    assert_eq!(l.puts(&format!("{r}events/change/")), 5);
}

/// What the reader `s` saw of the owner's traffic (`seen`), and what it may
/// ask: the GET and the call, each retried for `within`; the tokens; the
/// descriptor.
async fn far1(s: &zenoh::Session, seen: &Seen, within: Duration) -> Far1 {
    let r = "radio-a/radio.v1/";
    let read = zenkey::presence::liveliness_read(s, "zk2/lab/*/@zk/**", T)
        .await
        .unwrap();
    assert!(read.complete, "a refused read is complete (§8.1): {read:?}");
    Far1 {
        dropped: seen.puts(&format!("{r}stream/air/dropped_total")),
        frames: seen.puts(&format!("{r}@stream/frames")),
        frames_total: seen.puts(&format!("{r}stream/air/frames_total")),
        raw: seen.puts(&format!("{r}@stream/raw")),
        oper: seen.puts(&format!("{r}state/oper")),
        fault: seen.puts(&format!("{r}events/fault/")),
        change: seen.puts(&format!("{r}events/change/")),
        get: answered(s, PEERS_X, false, within).await,
        call: answered(s, RESET, true, within).await,
        tokens: read.keys.len(),
        descriptor: values(s, INSTANCE, false).await,
    }
}

/// §1's expected 2–4 for the far reader: one sample per second of each
/// downsampled stream, the first at once; nothing of what stays home; the
/// state and the exposed event whole; the requests answered; no token and
/// no descriptor.
fn assert_far(g: &Far1) {
    assert!((4..=6).contains(&g.dropped), "dropped_total 4 to 6: {g:?}");
    assert!((4..=6).contains(&g.frames), "frames 4 to 6: {g:?}");
    assert_eq!(
        (g.frames_total, g.raw, g.change),
        (0, 0, 0),
        "what stays home: {g:?}"
    );
    assert_eq!((g.oper, g.fault), (10, 5), "state and event whole: {g:?}");
    assert!(g.get && g.call, "requests cross by grant: {g:?}");
    assert_eq!((g.tokens, g.descriptor), (0, 0), "no @zk: {g:?}");
}

/// §1 A face compiled and enforced, the far side a client (§2.4–§2.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s1_a_face_compiled_and_enforced_the_far_side_a_client() {
    let text = policy(&[]);
    let plan = plan(&text);
    // 1. The block holds the plan: the far principal's policy carries every
    //    deny, and the downsampling block one rule per crossing stream of
    //    lab/radio-a at 1 Hz.
    let ground = plan.policies.iter().find(|p| p.id == "ground").unwrap();
    for d in &plan.faces[0].deny {
        assert!(
            ground.rules.contains(&format!("face:radio:{}", d.id)),
            "{}",
            d.id
        );
    }
    let keys: Vec<&str> = plan.downsampling[0]
        .rules
        .iter()
        .map(|r| r.key_expr.as_str())
        .collect();
    assert_eq!(
        keys,
        [
            "zk2/lab/radio-a/radio.v1/@stream/frames",
            "zk2/lab/radio-a/radio.v1/stream/air/dropped_total",
        ]
    );
    assert!(plan.downsampling[0].rules.iter().all(|r| r.freq == 1.0));

    let n = near(&plan).await;
    let o = owner(&n.tcp, "radio-a").await;
    let g = client(&n.face, "ground").await;
    let l = client(&n.tcp, "local").await;
    let far = one(&o, &g, &plan, Some(&l)).await;
    assert_far(&far);
    // 3. L's requests are answered too; 4. L sees the owner's tokens and
    //    its descriptor.
    assert!(answered(&l, PEERS_X, false, Duration::from_secs(5)).await);
    assert!(answered(&l, RESET, true, Duration::from_secs(5)).await);
    eventually(
        "L sees the owner's instance and interface tokens",
        || async {
            zenkey::presence::liveliness_read(&l, "zk2/lab/*/@zk/**", T)
                .await
                .unwrap()
                .keys
                .len()
                == 2
        },
    )
    .await;
    assert_eq!(values(&l, INSTANCE, false).await, 1);
    // 5. A tool checks the block: no finding.
    let c = check(&plan, &text, None);
    assert!(c.findings.is_empty(), "{:?}", c.findings);
    assert!(
        c.comparison.findings.is_empty(),
        "{:?}",
        c.comparison.findings
    );
    assert_eq!(c.judgement.conclusive(), Some(false));
}

/// GET and call across a south region, under `plan`: whether each is
/// answered.
async fn south_requests(plan: &AclPlan) -> (bool, bool) {
    let n = near(plan).await;
    let _o = owner(&n.tcp, "radio-a").await;
    let f = far(&n).await;
    let g = client(&f.tcp, "ground").await;
    let within = Duration::from_secs(4);
    (
        answered(&g, PEERS_X, false, within).await,
        answered(&g, RESET, true, within).await,
    )
}

/// §2 A far router in a south region (§2.4, §2.5, §2.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s2_a_far_router_in_a_south_region() {
    let text = policy(&SOUTH);
    let plan = plan(&text);
    let gw = plan.gateway.as_ref().expect("the plan's gateway");
    assert_eq!(gw.south[0].filters[0].modes, ["peer", "client"]);
    assert_eq!(gw.south[1].filters[0].region_names, ["ground"]);
    // The declaration grant covers the owner's state queryable as it
    // declares it, and reset's (core §8.5, 0.30; F-117).
    let decl = "face-declarations:ground-router";
    let d = plan.rules.iter().find(|r| r.id == decl).expect(decl);
    for k in [
        "zk2/lab/*/radio.v1/state/**",
        "zk2/lab/*/radio.v1/@op/reset",
    ] {
        assert!(d.key_exprs.iter().any(|x| x == k), "{k}: {:?}", d.key_exprs);
    }

    // The controls: no declaration grant (step 2); one over G's own GET and
    // call selectors only, not the owner's `state/**` (step 3).
    let mut none = plan.clone();
    none.rules.retain(|r| r.id != decl);
    for p in &mut none.policies {
        p.rules.retain(|r| r != decl);
    }
    let mut selectors = plan.clone();
    for r in &mut selectors.rules {
        if r.id == decl {
            r.key_exprs.retain(|k| !k.ends_with("/state/**"));
        }
    }

    let full = async {
        let n = near(&plan).await;
        let o = owner(&n.tcp, "radio-a").await;
        let f = far(&n).await;
        let g = client(&f.tcp, "ground").await;
        one(&o, &g, &plan, None).await
    };
    let far1 = full.await;
    let without = south_requests(&none).await;
    let narrow = south_requests(&selectors).await;
    // 1. G receives what §1's G receives; its GET and call are answered.
    assert_far(&far1);
    // 2. Without the grant, neither is: F learns of no queryable.
    assert_eq!(without, (false, false), "no declaration grant");
    // 3. Over G's selectors only, the call is answered and the GET is not.
    assert_eq!(
        narrow,
        (false, true),
        "the GET's selector does not include state/**"
    );
    // 4. Router to router is refused, and no block is written.
    let err = plan_acl(
        &enrollment(),
        &contracts(),
        &AclOptions {
            face_policy: Some(link::parse(&policy(&[(
                "attach       = \"client\"",
                "attach       = \"router\"",
            )]))),
            ..AclOptions::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("router_attach"), "{err}");
}

/// Each owner's `air/dropped_total` at 20 Hz for 5 s, after the settle;
/// what G receives from each.
async fn two_owners(plan: &AclPlan) -> (usize, usize) {
    let n = near(plan).await;
    let a = owner(&n.tcp, "radio-a").await;
    let b = owner(&n.tcp, "radio-b").await;
    let g = client(&n.face, "ground").await;
    let gr = reader(&g, plan, None).await;
    eventually("G hears both owners' oper", || async {
        a.put("state/oper", "probe").await;
        b.put("state/oper", "probe").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        gr.seen.puts("radio-a/radio.v1/state/oper") > 0
            && gr.seen.puts("radio-b/radio.v1/state/oper") > 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    gr.seen.clear();
    for i in 0..100u64 {
        a.put("stream/air/dropped_total", i.to_string()).await;
        b.put("stream/air/dropped_total", i.to_string()).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(SILENCE).await;
    (
        gr.seen.puts("radio-a/radio.v1/stream/air/dropped_total"),
        gr.seen.puts("radio-b/radio.v1/stream/air/dropped_total"),
    )
}

/// §3 One rule per key, and a service not listed (§2.8).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s3_one_rule_per_key_and_a_service_not_listed() {
    let both_text = policy(&[BOTH_LISTED]);
    let both = plan(&both_text);
    let keys: Vec<&str> = both.downsampling[0]
        .rules
        .iter()
        .map(|r| r.key_expr.as_str())
        .collect();
    assert!(keys.contains(&"zk2/lab/radio-a/radio.v1/stream/air/dropped_total"));
    assert!(keys.contains(&"zk2/lab/radio-b/radio.v1/stream/air/dropped_total"));
    // 2. The hand-written block: one wildcard rule for the two.
    let mut hand = both.clone();
    hand.downsampling[0].rules = vec![link::Rule {
        key_expr: "zk2/lab/*/radio.v1/stream/air/dropped_total".into(),
        freq: 1.0,
    }];
    let one_listed = plan(&policy(&[]));

    let ((a1, b1), (a2, b2), (a3, b3)) = (
        two_owners(&both).await,
        two_owners(&hand).await,
        two_owners(&one_listed).await,
    );
    // 1. A rule per key, a timer per rule.
    assert!((4..=6).contains(&a1) && (4..=6).contains(&b1), "{a1} {b1}");
    // 2. One timer for every key the rule covers.
    assert!((4..=6).contains(&(a2 + b2)), "{a2} + {b2}");
    let c = check(&hand, &both_text, None);
    assert!(
        c.findings
            .iter()
            .any(|f| f.code == link::Code::WildcardDownsampling),
        "{:?}",
        c.findings
    );
    assert_eq!(c.judgement.conclusive(), Some(true));
    // 3. A service not listed is in no rule, and crosses at its owner's rate.
    assert!((4..=6).contains(&a3), "radio-a {a3}");
    assert!(b3 >= 80, "radio-b about 100: {b3}");
}

/// `fault` and `change` 5 times each within one second; what G and L
/// receive of each.
async fn events(plan: &AclPlan) -> ((usize, usize), (usize, usize)) {
    let n = near(plan).await;
    let o = owner(&n.tcp, "radio-a").await;
    let g = client(&n.face, "ground").await;
    let l = client(&n.tcp, "local").await;
    let gr = reader(&g, plan, None).await;
    let lr = reader(&l, plan, None).await;
    // G's GET crosses whatever the face exposes: its session stands. The
    // settle waits on L, which hears `oper` whatever the face says.
    assert!(answered(&g, PEERS_X, false, Duration::from_secs(5)).await);
    o.settle(&[&lr.seen], &[&gr.seen]).await;
    for i in 0..10u64 {
        let ev = if i % 2 == 0 { "fault" } else { "change" };
        o.put(&format!("events/{ev}/{}", ulid(i)), "e").await;
        tokio::time::sleep(Duration::from_millis(90)).await;
    }
    tokio::time::sleep(SILENCE).await;
    let count = |s: &Seen| {
        (
            s.puts("radio-a/radio.v1/events/fault/"),
            s.puts("radio-a/radio.v1/events/change/"),
        )
    };
    (count(&gr.seen), count(&lr.seen))
}

/// §4 An event that must not cross (§2.7, §2.9).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s4_an_event_that_must_not_cross() {
    let generated = plan(&policy(&[]));
    // 2. The sketch's spelling of the events deny, draft 0's keys.
    let mut sketch = generated.clone();
    let id = "face:radio:events:radio.v1";
    let rule = sketch
        .rules
        .iter_mut()
        .find(|r| r.id == id)
        .expect("the plan's events deny");
    assert_eq!(rule.key_exprs, ["zk2/*/*/radio.v1/events/change/**"]);
    rule.key_exprs = vec!["zk2/*/*/radio.v1/stream/change/**".into()];
    // 3. A face that exposes none.
    let closed = plan(&policy(&[CLOSED]));
    assert_eq!(
        closed.faces[0]
            .deny
            .iter()
            .find(|d| d.id == "events")
            .map(|d| d.key_exprs.clone()),
        Some(vec!["zk2/*/*/*/events/**".to_owned()])
    );

    let ((g1, l1), (g2, _), (g3, _)) = (
        events(&generated).await,
        events(&sketch).await,
        events(&closed).await,
    );
    // 1. Every fault on its own key, none dropped by the interval; no
    //    change. L receives all ten.
    assert_eq!(g1, (5, 0), "G");
    assert_eq!(l1, (5, 5), "L");
    // 2. The sketch's deny covers no event key.
    assert_eq!(g2, (5, 5), "G under the sketch's spelling");
    // 3. No occurrence at all.
    assert_eq!(g3, (0, 0), "G across a face that exposes none");
}

/// A late joiner's merge (core §4.2): each member and its stamp.
#[derive(Default)]
struct Merged(BTreeMap<String, Option<zenoh::time::Timestamp>>);

impl Merged {
    fn apply(&mut self, key: &str, kind: SampleKind, stamp: Option<zenoh::time::Timestamp>) {
        let newer = match (self.0.get(key).copied().flatten(), stamp) {
            (Some(held), Some(t)) => t > held,
            _ => true,
        };
        if !newer {
            return;
        }
        match kind {
            SampleKind::Put => {
                self.0.insert(key.to_owned(), stamp);
            }
            SampleKind::Delete => {
                self.0.remove(key);
            }
        }
    }

    /// Seeds as core §4.2 says: subscribed already, GET each granted
    /// selector over the owner's state, merge by stamp.
    async fn seed(&mut self, g: &zenoh::Session, selectors: &[String]) {
        for sel in selectors {
            let rx = g
                .get(sel.as_str())
                .target(QueryTarget::All)
                .consolidation(ConsolidationMode::None)
                .timeout(T)
                .await
                .unwrap();
            while let Ok(r) = rx.recv_async().await {
                if let Ok(s) = r.result() {
                    self.apply(s.key_expr().as_ref(), s.kind(), s.timestamp().copied());
                }
            }
        }
    }
}

/// §5's steps 1 to 3 with the owner's window at `window_s`: what G's merge
/// holds after seeding, and after seeding again past the outage.
async fn outage(plan: &AclPlan, window_s: u64) -> (usize, usize) {
    let n = near(plan).await;
    let contract = contracts().iter().next().unwrap().contract().clone();
    let iface: zenkey_model::grammar::IfaceId = "radio.v1".parse().unwrap();
    let owner_session = client(&n.tcp, "radio-a").await;
    let mut cfg = util::zk2::config("lab/radio-a");
    cfg.tombstone_window_s = Some(window_s);
    let mut b = zenkey::ServiceBuilder::new(&owner_session, cfg);
    b.implement(zenkey::Implementation::new(contract.clone()))
        .unwrap();
    for r in &contract.resources {
        b.expose(
            &iface,
            &format!("{}/{}", r.token.as_str(), r.template.as_str()),
        )
        .unwrap();
    }
    b.serve_state(&iface).unwrap();
    let mut svc = b.start().await.expect("the owner starts under the block");
    let oper = svc
        .state_writer(&iface, "state/oper", &Default::default())
        .await
        .unwrap();
    let peer: zenkey_model::template::Bindings = [("peer".to_owned(), vec!["x".to_owned()])].into();
    let peers = svc
        .state_writer(&iface, "state/peers/{peer}", &peer)
        .await
        .unwrap();
    oper.put("on").await.unwrap();
    peers.put("px").await.unwrap();

    // G's granted selectors over lab/radio-a's state (F-116).
    let selectors = vec![
        "zk2/lab/radio-a/radio.v1/state/oper".to_owned(),
        "zk2/lab/radio-a/radio.v1/state/peers/*".to_owned(),
    ];
    let mut merged = Merged::default();
    let g = client(&n.face, "ground").await;
    let gr = reader(&g, plan, Some(&selectors)).await;
    merged.seed(&g, &selectors).await;
    let before = merged.0.len();
    // 2. The face goes down for 4 s; meanwhile the owner deletes both.
    drop(gr);
    g.close().await.unwrap();
    oper.delete().await.unwrap();
    peers.delete().await.unwrap();
    tokio::time::sleep(Duration::from_secs(4)).await;
    // 3. The face comes back, and G seeds again.
    let g = client(&n.face, "ground").await;
    let _gr = reader(&g, plan, Some(&selectors)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    merged.seed(&g, &selectors).await;
    (before, merged.0.len())
}

/// §5 A state window raised to the outage (§2.11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s5_a_state_window_raised_to_the_outage() {
    let text = policy(&[]);
    let plan = plan(&text);
    // The plan's floor is the face's outage, and the owner runs with it.
    let floor = plan.faces[0].windows[0].clone();
    assert_eq!((floor.service.as_str(), floor.min_s), ("lab/radio-a", 6));
    // One after the other: two runtime owners of `lab/radio-a` in one
    // process, each on a bus of its own, left a GET unanswered when run at
    // once (observed; not this profile's).
    let raised = outage(&plan, floor.min_s).await;
    let control = outage(&plan, 2).await;
    // 1–3. G held both, and its merge removes both by the owner's
    //      `reply_del`s, within the window.
    assert_eq!(raised, (2, 0), "a window of the outage keeps the deletes");
    // 4. A window below the outage loses them: both stay standing.
    assert_eq!(control, (2, 2), "a lost delete (core §4.2)");
    // The linter, given the windows, reports the control's.
    let low = BTreeMap::from([("lab/radio-a".to_owned(), 2)]);
    let c = check(&plan, &text, Some(&low));
    assert_eq!(
        c.findings,
        [link::Finding::new(
            link::Code::WindowUnderOutage,
            Some("radio"),
            "lab/radio-a"
        )]
    );
    let ok = BTreeMap::from([("lab/radio-a".to_owned(), floor.min_s)]);
    assert!(check(&plan, &text, Some(&ok)).findings.is_empty());
}

/// A tool across the face judging a provider's liveness (`freshness.v1`
/// §2.9, §5) over the plan's evidence: live while a member of it is fresh
/// by G's receive clock; never down.
fn liveness(plan: &AclPlan, seen: &Seen, since: Instant) -> &'static str {
    let evidence = &plan.faces[0]
        .liveness
        .iter()
        .find(|l| l.service == "lab/radio-a")
        .unwrap()
        .evidence;
    let contract = contracts().iter().next().unwrap().contract().clone();
    let fresh = evidence.iter().any(|name| {
        let rel = name.strip_prefix("radio.v1/").unwrap();
        let r = contract
            .resources
            .iter()
            .find(|r| format!("{}/{}", r.token.as_str(), r.template.as_str()) == rel)
            .unwrap();
        let h = freshness::horizon(r.kind, &r.annotations);
        let o = Observation::Subscribed {
            last: seen
                .last(&format!("radio-a/radio.v1/{rel}"))
                .map(|t| Last::Put(t.elapsed())),
            listened: since.elapsed(),
            complete: true,
        };
        freshness::judge(&h, &o).verdict == Verdict::Fresh
    });
    if fresh { "live" } else { "unobservable" }
}

/// §6's steps on `plan`: the verdict at 5 s, and 6 s after the owner
/// closes; and how many deliveries G had.
async fn judged(plan: &AclPlan) -> (&'static str, &'static str, usize) {
    let n = near(plan).await;
    let o = owner(&n.tcp, "radio-a").await;
    let g = client(&n.face, "ground").await;
    let evidence = vec![
        "zk2/lab/radio-a/radio.v1/state/oper".to_owned(),
        "zk2/lab/radio-a/radio.v1/stream/air/dropped_total".to_owned(),
    ];
    let gr = reader(&g, plan, Some(&evidence)).await;
    assert!(answered(&g, PEERS_X, false, Duration::from_secs(5)).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let since = Instant::now();
    let mut at_5 = "";
    for i in 0..120u64 {
        o.put("stream/air/dropped_total", i.to_string()).await;
        if i % 40 == 0 {
            o.put("state/oper", "up").await;
        }
        if i == 100 {
            at_5 = liveness(plan, &gr.seen, since);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // 2. The owner closes its service: nothing of it crosses any more.
    //    Across the face that is all a close changes (its tokens never
    //    crossed, §2.6), so the owner stops publishing and is left open.
    let delivered = gr.seen.len();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let after = liveness(plan, &gr.seen, since);
    drop(o);
    (at_5, after, delivered)
}

/// §6 Liveness across the face (§2.12).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s6_liveness_across_the_face() {
    let open = plan(&policy(&[]));
    assert_eq!(
        open.faces[0].liveness[0].evidence,
        ["radio.v1/state/oper", "radio.v1/stream/air/dropped_total"]
    );
    let closed = plan(&policy(&[CLOSED]));
    assert!(closed.faces[0].liveness[0].evidence.is_empty());
    let (live, after, _) = judged(&open).await;
    let (from_start, _, nothing) = judged(&closed).await;
    // 1. Both fresh at G: live.
    assert_eq!(live, "live");
    // 2. Both stale: unobservable, never down (core R7).
    assert_eq!(after, "unobservable");
    // 3. No evidence: unobservable from the start, whatever the owner
    //    publishes, and nothing of it reaches G.
    assert_eq!(from_start, "unobservable");
    assert_eq!(nothing, 0, "G receives nothing of the owner");
}

/// The console's Consume selectors, as the `deny` block grants them
/// (F-116): what every reader subscribes to, under either posture.
fn selectors() -> Vec<String> {
    plan(&policy(&[]))
        .rules
        .into_iter()
        .find(|r| r.id == "consume-in:tool.console")
        .expect("the console's Consume")
        .key_exprs
}

/// How many of five puts `s` makes on `lab/radio-a`'s `air/dropped_total`,
/// a write only `lab/radio-a` may make (core §11.1, Own), reach L's reader
/// `l`, the owner silent.
async fn forged(s: &zenoh::Session, l: &Seen) -> usize {
    tokio::time::sleep(Duration::from_millis(300)).await;
    l.clear();
    for i in 0..5 {
        s.put(
            "zk2/lab/radio-a/radio.v1/stream/air/dropped_total",
            format!("forged {i}"),
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    l.puts("radio-a/radio.v1/stream/air/dropped_total")
}

/// §7's steps on `plan`: L near, and a session over the face for each of
/// `users`, `true` for one that must hear the owner before the traffic
/// starts. What each one over the face receives of the owner's traffic and
/// may ask, and how many of its forged puts reach L.
async fn on_face(plan: &AclPlan, users: &[(&str, bool)]) -> Vec<(Far1, usize)> {
    let sel = selectors();
    let n = near(plan).await;
    let o = owner(&n.tcp, "radio-a").await;
    let l = client(&n.tcp, "local").await;
    let lr = reader(&l, plan, Some(&sel)).await;
    let mut sessions = Vec::new();
    let mut readers = Vec::new();
    for (user, _) in users {
        let s = client(&n.face, user).await;
        readers.push(reader(&s, plan, Some(&sel)).await);
        sessions.push(s);
    }
    let mut ready = vec![&lr.seen];
    let mut quiet = Vec::new();
    for ((_, hears), r) in users.iter().zip(&readers) {
        if *hears {
            ready.push(&r.seen);
        } else {
            quiet.push(&r.seen);
        }
    }
    o.settle(&ready, &quiet).await;
    traffic(&o).await;
    tokio::time::sleep(SILENCE).await;
    assert_near(&lr.seen);
    let mut out = Vec::new();
    for (s, r) in sessions.iter().zip(&readers) {
        let seen = far1(s, &r.seen, Duration::from_secs(3)).await;
        out.push((seen, forged(s, &lr.seen).await));
    }
    out
}

/// What a session over the face gets when nothing guards the face from it:
/// every push, the tokens, its requests answered (F-118's measurement).
/// The downsampling still applies: it selects by link protocol.
fn assert_unguarded(x: &Far1) {
    assert!(
        x.frames_total >= 80 && x.raw >= 80,
        "what stays home: {x:?}"
    );
    assert!(
        (4..=6).contains(&x.dropped) && (4..=6).contains(&x.frames),
        "downsampled all the same: {x:?}"
    );
    assert_eq!((x.oper, x.fault, x.change), (10, 5, 5), "{x:?}");
    assert!(x.get && x.call, "{x:?}");
    assert_eq!(
        x.tokens, 2,
        "the owner's instance and interface tokens: {x:?}"
    );
}

/// §7 A near principal's credentials on the face (§2.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s7_a_near_principals_credentials_on_the_face() {
    let text = policy(&[]);
    let deny = plan(&text);
    let allow = plan_with(&text, &contracts(), AclPermission::Allow);
    // 1. Every subject but the far principal's is narrowed off the face;
    //    under allow, the face has a subject of its own, in ground's policy.
    for p in [&deny, &allow] {
        for s in &p.subjects {
            match s.id.as_str() {
                "ground" => assert!(s.link_protocols.is_empty(), "{s:?}"),
                "face:radio" => assert_eq!(s.link_protocols, ["unixsock-stream"]),
                _ => assert_eq!(
                    s.link_protocols,
                    [
                        "tcp", "udp", "tls", "quic", "serial", "unixpipe", "vsock", "ws"
                    ],
                    "{}",
                    s.id
                ),
            }
        }
    }
    assert!(!deny.subjects.iter().any(|s| s.id == "face:radio"));
    let ground = allow.policies.iter().find(|p| p.id == "ground").unwrap();
    assert_eq!(ground.subjects, ["ground", "face:radio"]);

    // The controls. 2: 0.2's block, no subject narrowed. 4: under allow,
    // the subjects narrowed and no face subject. 5: under allow, the face's
    // subject with the plan's denies alone, in a policy of its own.
    let mut old = deny.clone();
    for s in &mut old.subjects {
        s.link_protocols.clear();
    }
    let mut open = allow.clone();
    open.subjects.retain(|s| s.id != "face:radio");
    for p in &mut open.policies {
        p.subjects.retain(|s| s != "face:radio");
    }
    let mut split = open.clone();
    split.subjects.push(
        allow
            .subjects
            .iter()
            .find(|s| s.id == "face:radio")
            .unwrap()
            .clone(),
    );
    split.policies.push(AclPolicy {
        id: "face:radio".into(),
        rules: ground
            .rules
            .iter()
            .filter(|r| r.starts_with("face:radio:"))
            .cloned()
            .collect(),
        subjects: vec!["face:radio".into()],
    });

    let under_deny = on_face(&deny, &[("ground", true), ("local", false)]).await;
    let under_old = on_face(&old, &[("local", true)]).await;
    let under_allow = on_face(&allow, &[("ground", true), ("local", true)]).await;
    let under_open = on_face(&open, &[("local", true)]).await;
    let under_split = on_face(&split, &[("ground", true)]).await;

    // 1. Under deny, `local` over the face matches no subject: nothing at
    //    all, its requests unanswered. G as §1.
    assert_far(&under_deny[0].0);
    let nothing = Far1 {
        dropped: 0,
        frames: 0,
        frames_total: 0,
        raw: 0,
        oper: 0,
        fault: 0,
        change: 0,
        get: false,
        call: false,
        tokens: 0,
        descriptor: 0,
    };
    assert_eq!(under_deny[1], (nothing, 0), "local over the face, deny");
    // 2. 0.2's block: local's subject matches over the face, and its
    //    grants carry everything there (F-118).
    assert_unguarded(&under_old[0].0);
    // 3. Under allow, `local` over the face matches the face's subject, in
    //    ground's policy: what G gets, never more. Neither may write
    //    radio-a's keys.
    assert_far(&under_allow[0].0);
    assert_far(&under_allow[1].0);
    assert_eq!((under_allow[0].1, under_allow[1].1), (0, 0), "no forgery");
    // 4. Narrowed with no face subject, under allow: no subject, the
    //    default, everything, its writes included.
    assert_unguarded(&under_open[0].0);
    assert_eq!(under_open[0].1, 5, "a session matching no subject writes");
    // 5. A face subject with the plan's denies alone undoes ground's own:
    //    any allow wins across subjects, so its writes reach L.
    assert_far(&under_split[0].0);
    assert_eq!(under_split[0].1, 5, "ground's deny-write undone");

    // 6. A tool checks each block (§2.15).
    let clean = check(&deny, &text, None);
    assert!(clean.findings.is_empty(), "{:?}", clean.findings);
    assert!(clean.comparison.findings.is_empty());
    let c = check(&old, &text, None);
    let near: Vec<(link::Code, &str)> =
        c.findings.iter().map(|f| (f.code, f.at.as_str())).collect();
    assert_eq!(
        near,
        [
            (link::Code::NearSubjectOnFace, "ground-router"),
            (link::Code::NearSubjectOnFace, "local"),
            (link::Code::NearSubjectOnFace, "radio-a"),
            (link::Code::NearSubjectOnFace, "radio-b"),
        ]
    );
    let clean = check_against(&allow, &allow, &text, None);
    assert!(clean.findings.is_empty(), "{:?}", clean.findings);
    assert!(clean.comparison.findings.is_empty());
    let c = check_against(&open, &allow, &text, None);
    assert_eq!(
        c.findings,
        [link::Finding::new(
            link::Code::FaceSubjectMissing,
            Some("radio"),
            "unixsock-stream"
        )]
    );
    // The face subject in a policy of its own: misplaced, since 0.4 reads
    // which policies list a subject (F-120); the comparison names the
    // policy too.
    let c = check_against(&split, &allow, &text, None);
    assert_eq!(
        c.findings,
        [link::Finding::new(
            link::Code::FaceSubjectMisplaced,
            Some("radio"),
            "face:radio"
        )]
    );
    assert!(!c.comparison.findings.is_empty());
}

/// `radio.v1` minor 1: minor 0 with `air/frames_total` marked `link`, an
/// annotation changed, which core §9.8 classes a review.
fn minor_1() -> ContractSet {
    let text = std::fs::read_to_string(dir().join("radio.v1.toml")).unwrap();
    let from = "[resources.\"air/frames_total\"]                # stays home\n\
                kind = \"stream\"\n\
                type = { raw = \"text/plain\" }\n";
    assert!(text.contains(from) && text.contains("minor = 0"));
    let text = text.replace("minor = 0", "minor = 1").replace(
        from,
        "[resources.\"air/frames_total\"]\n\
         kind        = \"stream\"\n\
         type        = { raw = \"text/plain\" }\n\
         annotations = { \"link.exposure\" = \"link\" }\n",
    );
    let path = scratch("radio.v1.toml");
    std::fs::write(&path, text).unwrap();
    let (set, problems) = ContractSet::load_path(&path);
    let _ = std::fs::remove_file(&path);
    assert!(problems.is_empty(), "{problems:?}");
    set
}

/// Both owners' `air/frames_total` and `air/dropped_total` at 20 Hz for
/// 5 s, after the settle: what G receives of each owner's two streams, and
/// what L receives of each owner's `air/frames_total`.
async fn two_minors(plan: &AclPlan) -> ([usize; 2], [usize; 2], [usize; 2]) {
    let sel = selectors();
    let n = near(plan).await;
    let a = owner(&n.tcp, "radio-a").await;
    let b = owner(&n.tcp, "radio-b").await;
    let g = client(&n.face, "ground").await;
    let l = client(&n.tcp, "local").await;
    let gr = reader(&g, plan, Some(&sel)).await;
    let lr = reader(&l, plan, Some(&sel)).await;
    eventually("G and L hear both owners' oper", || async {
        a.put("state/oper", "probe").await;
        b.put("state/oper", "probe").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        [&gr.seen, &lr.seen].iter().all(|s| {
            s.puts("radio-a/radio.v1/state/oper") > 0 && s.puts("radio-b/radio.v1/state/oper") > 0
        })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    gr.seen.clear();
    lr.seen.clear();
    for i in 0..100u64 {
        for o in [&a, &b] {
            o.put("stream/air/frames_total", i.to_string()).await;
            o.put("stream/air/dropped_total", i.to_string()).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(SILENCE).await;
    let of = |s: &Seen, stream: &str| {
        ["radio-a", "radio-b"].map(|o| s.puts(&format!("{o}/radio.v1/stream/air/{stream}")))
    };
    (
        of(&gr.seen, "frames_total"),
        of(&gr.seen, "dropped_total"),
        of(&lr.seen, "frames_total"),
    )
}

/// §8 Two minors that disagree on a stream's exposure (§2.3, §2.7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s8_two_minors_that_disagree_on_a_streams_exposure() {
    let text = policy(&[BOTH_LISTED]);
    let mut both = contracts();
    both.extend(minor_1());
    let together = plan_with(&text, &both, AclPermission::Deny);
    let newer = plan_with(&text, &minor_1(), AclPermission::Deny);
    let older = plan(&text);
    // 1. From both revisions: the warning, and minor 0's denies and rules.
    assert_eq!(together.contracts.len(), 2, "{:?}", together.contracts);
    assert_eq!(
        together.faces[0].findings,
        [link::Finding::new(
            link::Code::ExposureDisagrees,
            Some("radio"),
            "radio.v1/stream/air/frames_total"
        )]
    );
    assert_eq!(together.faces[0].deny, older.faces[0].deny);
    assert_eq!(together.downsampling, older.downsampling);
    // 2. The control, minor 1 alone: frames_total crosses for both owners,
    //    and radio-b's, `host` in the minor it runs, with it.
    assert!(newer.faces[0].findings.is_empty());
    for o in ["radio-a", "radio-b"] {
        let k = format!("zk2/lab/{o}/radio.v1/stream/air/frames_total");
        assert!(
            newer.downsampling[0].rules.iter().any(|r| r.key_expr == k),
            "{k}"
        );
    }

    let (g1, d1, l1) = two_minors(&together).await;
    let (g2, d2, _) = two_minors(&newer).await;
    // 1. Neither owner's frames_total crosses; dropped_total does, at 1 Hz;
    //    L receives every sample.
    assert_eq!(g1, [0, 0], "frames_total from neither owner");
    assert!(d1.iter().all(|n| (4..=6).contains(n)), "{d1:?}");
    assert!(l1.iter().all(|n| *n >= 80), "{l1:?}");
    // 2. Minor 1 alone lets both owners' cross (F-119).
    assert!(g2.iter().all(|n| (4..=6).contains(n)), "{g2:?}");
    assert!(d2.iter().all(|n| (4..=6).contains(n)), "{d2:?}");
}

/// `plan` with the subject `id` edited: a hand edit of the block N runs.
fn with_subject(plan: &AclPlan, id: &str, edit: impl Fn(&mut AclSubject)) -> AclPlan {
    let mut p = plan.clone();
    edit(
        p.subjects
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("a subject {id}")),
    );
    p
}

/// `plan` with the subject `id` listed by the policies `to`, and by no
/// other.
fn listed_in(plan: &AclPlan, id: &str, to: &[&str]) -> AclPlan {
    let mut p = plan.clone();
    for pol in &mut p.policies {
        pol.subjects.retain(|s| s != id);
        if to.contains(&pol.id.as_str()) {
            pol.subjects.push(id.to_owned());
        }
    }
    p
}

/// The face's own subject, as the generator writes it under `allow`
/// (§2.4).
fn face_subject() -> AclSubject {
    AclSubject {
        id: "face:radio".into(),
        cert_common_names: Vec::new(),
        usernames: Vec::new(),
        link_protocols: vec!["unixsock-stream".into()],
        runs: Vec::new(),
    }
}

/// How many of five puts a session with each of `users`' credentials over
/// N's TCP endpoint, the near side, makes on `lab/radio-a`'s
/// `air/dropped_total` reach L, the owner silent.
async fn near_forged(plan: &AclPlan, users: &[&str]) -> Vec<usize> {
    let sel = selectors();
    let n = near(plan).await;
    let o = owner(&n.tcp, "radio-a").await;
    let l = client(&n.tcp, "local").await;
    let lr = reader(&l, plan, Some(&sel)).await;
    o.settle(&[&lr.seen], &[]).await;
    let mut out = Vec::new();
    for user in users {
        let s = client(&n.tcp, user).await;
        out.push(forged(&s, &lr.seen).await);
    }
    out
}

/// What a session with `radio-a`'s credentials over N's TCP endpoint,
/// subscribed to `lab/radio-b`'s `air/dropped_total` alone, a read none of
/// `radio-a`'s grants names (core §11.1), receives of 20 samples
/// `lab/radio-b` publishes there; L receives all 20.
async fn near_read(plan: &AclPlan) -> usize {
    let key = "zk2/lab/radio-b/radio.v1/stream/air/dropped_total";
    let only = [key.to_owned()];
    let n = near(plan).await;
    let b = client(&n.tcp, "radio-b").await;
    let a = client(&n.tcp, "radio-a").await;
    let ar = reader(&a, plan, Some(&only)).await;
    let l = client(&n.tcp, "local").await;
    let lr = reader(&l, plan, Some(&only)).await;
    eventually("L hears radio-b", || async {
        b.put(key, "probe").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        lr.seen.puts(key) > 0
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    ar.seen.clear();
    lr.seen.clear();
    for i in 0..20u64 {
        b.put(key, i.to_string()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(lr.seen.puts(key), 20, "L receives every sample");
    ar.seen.puts(key)
}

/// §9 Who a subject binds (§2.4, §2.15).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s9_who_a_subject_binds() {
    let text = policy(&[]);
    let deny = plan(&text);
    let allow = plan_with(&text, &contracts(), AclPermission::Allow);
    // 1. F-120 (a): local's subject bound to ground and local, in local's
    //    policy. 2, control: the same, ground's subject narrowed to its
    //    face (§2.4's MAY under deny).
    let both = with_subject(&deny, "local", |s| {
        s.usernames = vec!["ground".into(), "local".into()];
        s.link_protocols.clear();
    });
    let both_apart = with_subject(&both, "ground", |s| {
        s.link_protocols = vec!["unixsock-stream".into()];
    });
    // 3. F-120 (c): under allow, ground narrowed to its face. 4: under
    //    allow, radio-b narrowed off tcp too.
    let far_narrowed = with_subject(&allow, "ground", |s| {
        s.link_protocols = vec!["unixsock-stream".into()];
    });
    let near_narrowed = with_subject(&allow, "radio-b", |s| {
        s.link_protocols.retain(|p| p != "tcp");
    });

    let under_both = on_face(&both, &[("ground", true), ("local", true)]).await;
    let under_apart = on_face(&both_apart, &[("ground", true), ("local", true)]).await;
    let far_tcp = near_forged(&far_narrowed, &["ground"]).await;
    let near_tcp = near_forged(&near_narrowed, &["radio-b"]).await;
    let control = near_forged(&allow, &["ground", "radio-b"]).await;

    // 1. X matches the subject bound to both, and gets local's grants
    //    across the face. G keeps §1's share: zenoh splits a subject into
    //    one combination per value, and equal combinations are one subject,
    //    so ground's combination of it is ground's own subject, whose deny
    //    is evaluated first. Neither's puts reach L.
    assert_far(&under_both[0].0);
    assert_unguarded(&under_both[1].0);
    assert_eq!((under_both[0].1, under_both[1].1), (0, 0), "no forgery");
    // 2. With ground's subject narrowed, the two combinations differ: G
    //    matches both, any allow wins, and it gets local's share too.
    assert_unguarded(&under_apart[0].0);
    assert_unguarded(&under_apart[1].0);
    assert_eq!((under_apart[0].1, under_apart[1].1), (0, 0), "no forgery");
    // 3. ground over tcp matches no subject: allowed everything, its puts
    //    on radio-a's key among it. 4: radio-b over tcp, the same. Under
    //    the generator's block, neither's put reaches L.
    assert_eq!(far_tcp, [5], "ground over tcp, narrowed to its face");
    assert_eq!(near_tcp, [5], "radio-b over tcp, narrowed off it");
    assert_eq!(control, [0, 0], "the generator's block");

    // 5. An empty list is refused when the configuration loads, so no
    //    router runs one (zenoh-config's NEVec).
    let loads = |subject: &str| {
        zenoh::Config::from_json5(&format!(
            "{{ access_control: {{ enabled: true, default_permission: \"deny\", rules: [{{ id: \
             \"r\", permission: \"allow\", flows: [\"ingress\"], messages: [\"put\"], \
             key_exprs: [\"a\"] }}], subjects: [{subject}], policies: [{{ rules: [\"r\"], \
             subjects: [\"s\"] }}] }} }}"
        ))
        .is_ok()
    };
    assert!(
        loads("{ id: \"s\", usernames: [\"local\"] }"),
        "the control loads"
    );
    assert!(!loads(
        "{ id: \"s\", usernames: [\"local\"], link_protocols: [] }"
    ));
    assert!(!loads("{ id: \"s\", usernames: [] }"));

    // 6. A tool checks each block (§2.15).
    let found = |c: zenkey_fleet::report::FaceCheck| -> Vec<(link::Code, Option<String>, String)> {
        c.findings
            .into_iter()
            .map(|f| (f.code, f.face, f.at))
            .collect()
    };
    let radio = || Some("radio".to_owned());
    for b in [&both, &both_apart] {
        assert_eq!(
            found(check(b, &text, None)),
            [
                (link::Code::ZkNotDenied, radio(), "@zk".to_owned()),
                (link::Code::NearSubjectOnFace, radio(), "local".to_owned()),
            ]
        );
    }
    assert_eq!(
        found(check_against(&far_narrowed, &allow, &text, None)),
        [(link::Code::NarrowedTooFar, None, "ground".to_owned())]
    );
    assert_eq!(
        found(check_against(&near_narrowed, &allow, &text, None)),
        [(link::Code::NarrowedTooFar, None, "radio-b".to_owned())]
    );
}

/// §10 The face's own subject (§2.4, §2.15).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn s10_the_faces_own_subject() {
    let text = policy(&[]);
    let deny = plan(&text);
    let allow = plan_with(&text, &contracts(), AclPermission::Allow);
    // 1. F-120 (b): under allow, the face's subject names tcp too; and
    //    under deny, a subject selected by both, in ground's policy.
    let beyond = with_subject(&allow, "face:radio", |s| {
        s.link_protocols.push("tcp".into());
    });
    let mut beyond_deny = deny.clone();
    let mut wide = face_subject();
    wide.link_protocols.push("tcp".into());
    beyond_deny.subjects.push(wide);
    let beyond_deny = listed_in(&beyond_deny, "face:radio", &["ground"]);
    // 2. Under deny, a subject selected by the face's link protocol alone,
    //    in local's policy. 3: under allow, the face's subject moved there.
    let mut stray = deny.clone();
    stray.subjects.push(face_subject());
    let stray = listed_in(&stray, "face:radio", &["local"]);
    let moved = listed_in(&allow, "face:radio", &["local"]);
    // 4. Under allow, the face's subject in no policy.
    let unlisted = listed_in(&allow, "face:radio", &[]);
    // 5. Under deny, the face's subject in a policy of its own, holding the
    //    plan's denies alone (§7's step 5, under deny).
    let mut split = deny.clone();
    split.subjects.push(face_subject());
    let ground = deny.policies.iter().find(|p| p.id == "ground").unwrap();
    split.policies.push(AclPolicy {
        id: "face:radio".into(),
        rules: ground
            .rules
            .iter()
            .filter(|r| r.starts_with("face:radio:"))
            .cloned()
            .collect(),
        subjects: vec!["face:radio".into()],
    });

    let read_beyond = near_read(&beyond).await;
    let read_beyond_deny = near_read(&beyond_deny).await;
    let read_control = near_read(&allow).await;
    let read_control_deny = near_read(&deny).await;
    let under_stray = on_face(&stray, &[("ground", true), ("local", true)]).await;
    let under_moved = on_face(&moved, &[("ground", true), ("local", true)]).await;
    let under_unlisted = on_face(&unlisted, &[("ground", true), ("local", true)]).await;
    let under_split = on_face(&split, &[("ground", true), ("local", false)]).await;

    // 1. radio-a over tcp matches the face's subject, and reads radio-b's
    //    stream with ground's grants: every sample, under either posture,
    //    where the generator's blocks let it read none.
    assert_eq!(read_beyond, 20, "radio-a reads radio-b's stream, allow");
    assert_eq!(read_beyond_deny, 20, "radio-a reads radio-b's stream, deny");
    assert_eq!(
        (read_control, read_control_deny),
        (0, 0),
        "the generator's blocks"
    );
    // 2, 3. Every session over the face matches the stray subject, ground's
    //    among them, and gets local's grants there; any allow wins. Their
    //    puts reach L under neither: local may not write radio-a's keys.
    for (under, posture) in [(&under_stray, "deny"), (&under_moved, "allow")] {
        assert_unguarded(&under[0].0);
        assert_unguarded(&under[1].0);
        assert_eq!((under[0].1, under[1].1), (0, 0), "no forgery, {posture}");
    }
    // 4. A subject that no policy lists gets the default: under allow,
    //    everything, the puts on radio-a's key among it, for G as for X.
    assert_unguarded(&under_unlisted[0].0);
    assert_unguarded(&under_unlisted[1].0);
    assert_eq!((under_unlisted[0].1, under_unlisted[1].1), (5, 5));
    // 5. Under deny, a policy that only denies grants nothing: G keeps
    //    §1's share, X gets nothing, and neither's puts reach L.
    assert_far(&under_split[0].0);
    assert_eq!(
        under_split[1],
        (
            Far1 {
                dropped: 0,
                frames: 0,
                frames_total: 0,
                raw: 0,
                oper: 0,
                fault: 0,
                change: 0,
                get: false,
                call: false,
                tokens: 0,
                descriptor: 0,
            },
            0
        ),
        "X over the face: nothing at all"
    );
    assert_eq!(under_split[0].1, 0, "no forgery");

    // 6. A tool checks each block (§2.15).
    let found = |c: zenkey_fleet::report::FaceCheck| -> Vec<(link::Code, String)> {
        c.findings.into_iter().map(|f| (f.code, f.at)).collect()
    };
    let at = |code, at: &str| (code, at.to_owned());
    let misplaced = at(link::Code::FaceSubjectMisplaced, "face:radio");
    let zk = at(link::Code::ZkNotDenied, "@zk");
    assert_eq!(
        found(check_against(&beyond, &allow, &text, None)),
        [at(link::Code::FaceSubjectBeyondFace, "face:radio")]
    );
    assert_eq!(
        found(check(&beyond_deny, &text, None)),
        [at(link::Code::FaceSubjectBeyondFace, "face:radio")]
    );
    assert_eq!(
        found(check(&stray, &text, None)),
        [zk.clone(), misplaced.clone()]
    );
    for b in [&moved, &unlisted] {
        assert_eq!(
            found(check_against(b, &allow, &text, None)),
            [zk.clone(), misplaced.clone()]
        );
    }
    // Reported though harmless: §2.15 reads which policies list a subject,
    // not what their rules say.
    assert_eq!(found(check(&split, &text, None)), [misplaced]);
}
