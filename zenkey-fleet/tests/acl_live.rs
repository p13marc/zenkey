//! The generated access control, judged by zenoh itself (spec §11;
//! `spec/scenarios/security.md` §1 and §2; #612, FJ7).
//!
//! The planner's unit tests judge a plan by key-expression inclusion, the
//! planner's own reading of zenoh. These do not trust that reading. Each
//! renders the plan as `zenctl acl gen --json5` writes it, pastes the
//! fragment into a real router's config beside a usrpwd dictionary, opens a
//! client session per principal, and asks the router.
//!
//! The principals are spike S14's: the control walkthrough (a thruster and
//! its three commanders), the tcgui pilot (two backends and a frontend),
//! offline commanding (a fleet manager and an executor per vehicle), plus a
//! publisher using advanced publication and a reader with history. The
//! actors are plain zenoh sessions, so what is judged is the access control,
//! not a runtime's discipline; where the runtime's own selector is the thing
//! to judge (the fan-in GET a consumer sends), the runtime sends it.
//!
//! **Deny** (§1): the grants are allows, and every check S14 ran holds, with
//! 0.8's step added and measured here: a principal with Call on a backend's
//! operation reads that backend's tokens, and with its liveliness reads
//! removed from its grant the same read is answered complete and empty.
//! **Allow** (§2): every unauthorized action is blocked except a put on a
//! wildcard key, which R6 discards at the consumer; allow rules alone block
//! nothing. Measured beside it, and said in the plan's warning: a GET whose
//! selector is wider than every deny reaches the providers, and gets nothing
//! back (a value reply is checked against its own key), but a wildcard call
//! to an operation that allows fan-out *executes* on every provider for a
//! principal never granted it. **The generator check** (§2): without the
//! fan-in egress grant the fan-in GET gets no reply, and without the fan-in
//! reply grant a refusal never reaches the caller, while value replies still
//! do (checked against their own key, which Own includes). **#684**: a
//! principal's queryable in the admin space is refused.

mod util;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use util::zk2::{T, eventually};
use zenkey_fleet::report::{AclPermission, AclPlan, ContractSource, Enrollment};
use zenkey_fleet::{AclOptions, ContractSet, Revision, acl_plan_json5, plan_acl};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh_ext::{
    AdvancedPublisherBuilderExt, AdvancedSubscriberBuilderExt, CacheConfig, HistoryConfig,
};

/// The deployment: every principal a usrpwd user (password `<user>-pw`).
const ENROLLMENT: &str = r#"
[[principal]]
user = "thruster-l"
services = ["vehicle-01/thruster-l"]
[[principal]]
user = "teleop"
services = ["vehicle-01/teleop"]
[[principal]]
user = "autopilot"
services = ["vehicle-01/autopilot"]
[[principal]]
user = "safety"
services = ["vehicle-01/safety"]
[[principal]]
user = "tc-h1"
services = ["h1/tc"]
[[principal]]
user = "tc-h2"
services = ["h2/tc"]
[[principal]]
user = "frontend"
services = ["ops/frontend"]
[[principal]]
user = "fleet-mgr"
services = ["ground/fleet-mgr"]
[[principal]]
user = "executor-1"
services = ["vehicle-01/executor"]
[[principal]]
user = "executor-2"
services = ["vehicle-02/executor"]
[[principal]]
user = "beacon"
services = ["h1/beacon"]
[[principal]]
user = "reader"
services = ["ops/reader"]

[[service]]
address = "vehicle-01/thruster-l"
implements = ["thruster.v1"]
[service.bindings.cmd]
providers = ["vehicle-01/safety", "vehicle-01/teleop", "vehicle-01/autopilot"]

[[service]]
address = "vehicle-01/teleop"
implements = ["twist_cmd.v1"]
[[service]]
address = "vehicle-01/autopilot"
implements = ["twist_cmd.v1"]
[[service]]
address = "vehicle-01/safety"
implements = ["twist_cmd.v1"]

[[service]]
address = "h1/tc"
implements = ["tc.netif.v1", "tc.netem.v1", "tc.scenario.v1"]
[[service]]
address = "h2/tc"
implements = ["tc.netif.v1", "tc.netem.v1", "tc.scenario.v1"]
[[service]]
address = "ops/frontend"
[service.bindings.netif]
interface = "tc.netif.v1"
providers = ["*/tc"]
[service.bindings.netem]
interface = "tc.netem.v1"
providers = ["*/tc"]
[[service.calls]]
interface = "tc.netem.v1"
providers = ["*/tc"]
[[service.calls]]
interface = "tc.netif.v1"
providers = ["*/tc"]
operations = ["diagnostics"]

[[service]]
address = "ground/fleet-mgr"
implements = ["mission_plan.v1"]
[[service]]
address = "vehicle-01/executor"
[service.bindings.plan]
interface = "mission_plan.v1"
providers = ["ground/fleet-mgr"]
params = { vehicle = "self.system" }
[[service]]
address = "vehicle-02/executor"
[service.bindings.plan]
interface = "mission_plan.v1"
providers = ["ground/fleet-mgr"]
params = { vehicle = "self.system" }

[[service]]
address = "h1/beacon"
implements = ["beacon.v1"]
[[service]]
address = "ops/reader"
[service.bindings.fix]
interface = "beacon.v1"
providers = ["*/beacon"]
history = true
"#;

/// A state with `history` (§2.5): the case the examples do not carry.
const BEACON: &str = r#"[interface]
name = "beacon"
major = 1
minor = 0

[resources.position]
kind = "state"
type = { raw = "text/plain" }
history = true
"#;

const PRINCIPALS: [&str; 12] = [
    "thruster-l",
    "teleop",
    "autopilot",
    "safety",
    "tc-h1",
    "tc-h2",
    "frontend",
    "fleet-mgr",
    "executor-1",
    "executor-2",
    "beacon",
    "reader",
];

const CMD: &str = "zk2/vehicle-01/{}/twist_cmd.v1/stream/cmd";
const STATUS: &str = "zk2/vehicle-01/thruster-l/thruster.v1/state/status";
const POSITION: &str = "zk2/h1/beacon/beacon.v1/state/position";
const CONTRACT: &str =
    "zk2/@zk/contract/tc.netif.v1/0f66b421dc6decf2b28bf4b8f0c6006cbdbb7fe0a4d9a997e637f9105dc0ea98";
/// A router zid nobody has (#684).
const ADMIN: &str = "@/0123456789abcdef0123456789abcdef/router";

/// How long a test waits for something it expects **not** to arrive, once
/// the same session has seen what it expects to arrive.
const SILENCE: Duration = Duration::from_millis(1200);

fn cmd(commander: &str) -> String {
    CMD.replace("{}", commander)
}

fn contracts() -> ContractSet {
    let mut set = ContractSet::new();
    for dir in ["walkthrough", "tcgui"] {
        let (s, _) = ContractSet::load_path(&util::zk2::examples().join(dir));
        set.extend(s);
    }
    let beacon = zenkey_model::contract::load_str(BEACON, std::path::Path::new("."), None)
        .contract
        .expect("the beacon contract loads");
    set.insert(Revision::from_contract(beacon, ContractSource::File));
    set
}

fn plan(default_permission: AclPermission) -> AclPlan {
    let enrollment: Enrollment = toml::from_str(ENROLLMENT).expect("the enrollment parses");
    let plan = plan_acl(
        &enrollment,
        &contracts(),
        &AclOptions {
            default_permission,
            ..AclOptions::default()
        },
    )
    .expect("the deployment plans");
    assert!(plan.refusals.is_empty(), "{:#?}", plan.refusals);
    assert_eq!(plan.subjects.len(), PRINCIPALS.len());
    plan
}

/// `plan` with `rule` dropped from every policy holding it.
fn without_rule(mut plan: AclPlan, rule: &str) -> AclPlan {
    assert!(plan.rules.iter().any(|r| r.id == rule), "{rule}");
    for p in &mut plan.policies {
        p.rules.retain(|r| r != rule);
    }
    plan.policies.retain(|p| !p.rules.is_empty());
    plan.rules.retain(|r| r.id != rule);
    plan
}

/// `plan` with `key` dropped from every rule whose id starts with `prefix`.
fn without_key(mut plan: AclPlan, prefix: &str, key: &str) -> AclPlan {
    let mut n = 0;
    for r in plan.rules.iter_mut().filter(|r| r.id.starts_with(prefix)) {
        let before = r.key_exprs.len();
        r.key_exprs.retain(|k| k != key);
        n += before - r.key_exprs.len();
    }
    assert!(n > 0, "{prefix} carries {key}");
    plan
}

fn scratch(name: &str) -> std::path::PathBuf {
    static NTH: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "zenkey-acl-live-{}-{}-{name}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A router holding the plan's block and a usrpwd dictionary of every
/// principal. Returns the router and its endpoint.
async fn router(plan: &AclPlan) -> (zenoh::Session, String, std::path::PathBuf) {
    let dict = scratch("usrpwd.txt");
    let mut users = String::from("router:router-pw\n");
    for p in PRINCIPALS {
        users.push_str(&format!("{p}:{p}-pw\n"));
    }
    std::fs::write(&dict, users).unwrap();
    let text = format!(
        "{{\n  mode: \"router\",\n  scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }},\n  \
         listen: {{ endpoints: [\"{}\"] }},\n  \
         transport: {{ auth: {{ usrpwd: {{ user: \"router\", password: \"router-pw\", dictionary_file: {:?} }} }} }},\n{}}}\n",
        util::ANY_PORT,
        dict.display().to_string(),
        acl_plan_json5(plan)
    );
    let config = zenoh::Config::from_json5(&text)
        .unwrap_or_else(|e| panic!("the router config does not parse: {e}\n{text}"));
    let r = zenoh::open(config).await.expect("a router with the plan");
    let ep = util::bound(&r).await;
    (r, ep, dict)
}

/// A client of `endpoint` presenting `user`, when given.
async fn client(endpoint: &str, user: Option<&str>) -> zenoh::Result<zenoh::Session> {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    // An advanced publisher sequences by timestamp (zenoh-ext 1.10.1).
    c.insert_json5("timestamping/enabled", "true").unwrap();
    if let Some(u) = user {
        c.insert_json5(
            "transport/auth/usrpwd",
            &format!("{{ user: \"{u}\", password: \"{u}-pw\" }}"),
        )
        .unwrap();
    }
    zenoh::open(c).await
}

type Got = Arc<Mutex<Vec<String>>>;

async fn subscribe(s: &zenoh::Session, key: &str) -> (zenoh::pubsub::Subscriber<()>, Got) {
    let got: Got = Arc::default();
    let g = Arc::clone(&got);
    let sub = s
        .declare_subscriber(key.to_owned())
        .callback(move |smp| g.lock().unwrap().push(smp.key_expr().as_str().to_owned()))
        .await
        .expect("a subscriber");
    (sub, got)
}

fn seen(got: &Got) -> Vec<String> {
    got.lock().unwrap().clone()
}

/// The replies to a GET: (value keys, error payloads).
async fn get(
    s: &zenoh::Session,
    selector: &str,
    target: QueryTarget,
    wait: Duration,
) -> (Vec<String>, Vec<String>) {
    let rx = s
        .get(selector)
        .target(target)
        .consolidation(ConsolidationMode::None)
        .timeout(wait)
        .await
        .expect("a get is sent");
    let (mut values, mut errors) = (Vec::new(), Vec::new());
    while let Ok(reply) = rx.recv_async().await {
        match reply.result() {
            Ok(sample) => values.push(sample.key_expr().as_str().to_owned()),
            Err(e) => errors.push(String::from_utf8_lossy(&e.payload().to_bytes()).into_owned()),
        }
    }
    values.sort();
    (values, errors)
}

/// A queryable answering on `key` with `payload`, on its own key.
async fn serve(
    s: &zenoh::Session,
    key: &str,
    payload: &'static str,
) -> zenoh::query::Queryable<()> {
    let k = key.to_owned();
    s.declare_queryable(key.to_owned())
        .callback(move |q| {
            let _ = q.reply(k.clone(), payload).wait();
        })
        .await
        .expect("a queryable")
}

/// A `complete` operation over a template: a concrete call is answered on
/// its key, a wildcard one refused `fanout_forbidden` (O2), as a server
/// does.
async fn operation(s: &zenoh::Session, template: &str) -> zenoh::query::Queryable<()> {
    s.declare_queryable(template.to_owned())
        .complete(true)
        .callback(|q| {
            if q.key_expr().is_wild() {
                let _ = q
                    .reply_err(r#"{"error":"fanout_forbidden"}"#)
                    .encoding(Encoding::APPLICATION_JSON)
                    .wait();
            } else {
                let k = q.key_expr().clone().into_owned();
                let _ = q.reply(k, "done").wait();
            }
        })
        .await
        .expect("an operation")
}

/// Every principal's session, by user.
struct Bus {
    _router: zenoh::Session,
    endpoint: String,
    dict: std::path::PathBuf,
    s: std::collections::BTreeMap<&'static str, zenoh::Session>,
}

impl Bus {
    async fn up(plan: &AclPlan) -> Bus {
        let (router, endpoint, dict) = router(plan).await;
        let mut s = std::collections::BTreeMap::new();
        for p in PRINCIPALS {
            s.insert(
                p,
                client(&endpoint, Some(p))
                    .await
                    .unwrap_or_else(|e| panic!("{p} connects: {e}")),
            );
        }
        Bus {
            _router: router,
            endpoint,
            dict,
            s,
        }
    }

    fn of(&self, p: &str) -> &zenoh::Session {
        &self.s[p]
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.dict);
    }
}

/// The tcgui side's servers: each backend's state and netem operation, h1's
/// tokens and a contract bundle.
async fn tcgui(bus: &Bus) -> Servers {
    let mut q = Vec::new();
    let diagnostics = Arc::new(AtomicU64::new(0));
    for (h, who) in [("h1", "tc-h1"), ("h2", "tc-h2")] {
        q.push(
            serve(
                bus.of(who),
                &format!("zk2/{h}/tc/tc.netif.v1/state/namespaces"),
                "ns",
            )
            .await,
        );
        q.push(
            operation(
                bus.of(who),
                &format!("zk2/{h}/tc/tc.netem.v1/@op/config/*/*/set"),
            )
            .await,
        );
        // `diagnostics` allows fan-out: a wildcard call executes (O2).
        let (n, key) = (
            Arc::clone(&diagnostics),
            format!("zk2/{h}/tc/tc.netif.v1/@op/diagnostics"),
        );
        let k = key.clone();
        q.push(
            bus.of(who)
                .declare_queryable(key)
                .complete(true)
                .callback(move |query| {
                    n.fetch_add(1, Ordering::SeqCst);
                    let _ = query.reply(k.clone(), "diag").wait();
                })
                .await
                .expect("an operation"),
        );
    }
    q.push(serve(bus.of("tc-h1"), CONTRACT, "{}").await);
    let mut t = Vec::new();
    for key in [
        "zk2/h1/tc/@zk/instance/00000000000000a1",
        "zk2/h1/tc/@zk/alive/tc.netem.v1/00000000000000a1/0123456789abcdef",
    ] {
        t.push(
            bus.of("tc-h1")
                .liveliness()
                .declare_token(key)
                .await
                .expect("a token"),
        );
    }
    Servers {
        _queryables: q,
        _tokens: t,
        diagnostics,
    }
}

/// The tcgui backends' servers, held while a test runs.
struct Servers {
    _queryables: Vec<zenoh::query::Queryable<()>>,
    _tokens: Vec<zenoh::liveliness::LivelinessToken>,
    /// How many times a backend's `diagnostics` handler ran.
    diagnostics: Arc<AtomicU64>,
}

/// The frontend's fan-in GET: the runtime's own selector for
/// `state/namespaces` across `*/tc` (R1), a value per backend.
async fn fan_in(bus: &Bus) -> usize {
    let netif = contracts()
        .of_iface(&util::zk2::iface("tc.netif.v1"))
        .next()
        .expect("tc.netif.v1")
        .shared_contract();
    let consumer = zk2::consumer::Consumer::for_tool(
        bus.of("frontend"),
        netif,
        &["*/tc"],
        &Default::default(),
    )
    .expect("a consumer");
    match consumer
        .get("state/namespaces", None, T)
        .await
        .expect("a get")
    {
        zk2::state::StateGet::Answered(v) => v.len(),
        zk2::state::StateGet::Silent => 0,
    }
}

/// What the frontend's wildcard call to a fan-out-forbidden operation gets:
/// (values, server refusals).
async fn wildcard_call(bus: &Bus) -> (usize, usize) {
    let (values, errors) = get(
        bus.of("frontend"),
        "zk2/*/tc/tc.netem.v1/@op/config/default/eth0/set",
        QueryTarget::All,
        T,
    )
    .await;
    let refused = errors
        .iter()
        .filter(|e| e.contains("fanout_forbidden"))
        .count();
    (values.len(), refused)
}

/// The deny posture, security.md §1: S14's thirteen checks, 0.8's presence
/// step, `@adv`, and the admin space.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_deny_the_grants_are_exactly_what_is_allowed() {
    let plan = plan(AclPermission::Deny);
    let bus = Bus::up(&plan).await;

    // 1. A commander's put on its own command key reaches the actuator.
    let (_s1, from_teleop) = subscribe(bus.of("thruster-l"), &cmd("teleop")).await;
    let (_s2, from_autopilot) = subscribe(bus.of("thruster-l"), &cmd("autopilot")).await;
    eventually("the thruster receives teleop's own command", || async {
        bus.of("teleop").put(cmd("teleop"), "t").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        !seen(&from_teleop).is_empty()
    })
    .await;
    // 2. A put on another principal's key is blocked …
    bus.of("teleop")
        .put(cmd("autopilot"), "impersonated")
        .await
        .unwrap();
    // 3. … and so is one on a wildcard key over it.
    bus.of("teleop")
        .put("zk2/vehicle-01/*/twist_cmd.v1/stream/cmd", "wild")
        .await
        .unwrap();
    // 4. A subscription the bindings do not name is blocked.
    let (_s3, teleop_reads_status) = subscribe(bus.of("teleop"), STATUS).await;
    let (_s4, thruster_reads_status) = subscribe(bus.of("thruster-l"), STATUS).await;
    eventually(
        "the thruster's own status reaches its own subscriber",
        || async {
            bus.of("thruster-l").put(STATUS, "ok").await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            !seen(&thruster_reads_status).is_empty()
        },
    )
    .await;
    tokio::time::sleep(SILENCE).await;
    assert!(
        seen(&from_autopilot).is_empty(),
        "impersonation: {:?}",
        seen(&from_autopilot)
    );
    assert!(
        seen(&teleop_reads_status).is_empty(),
        "an unbound subscription"
    );

    // 5. The thruster sees its commanders' presence (0.8), one read per
    //    provider it binds; 6. the frontend, bound to none of them, gets a
    //    complete and empty read.
    let mut tokens = Vec::new();
    for c in ["teleop", "autopilot", "safety"] {
        tokens.push(
            bus.of(c)
                .liveliness()
                .declare_token(format!(
                    "zk2/vehicle-01/{c}/@zk/instance/00000000000000{:02x}",
                    c.len()
                ))
                .await
                .unwrap(),
        );
    }
    for c in ["teleop", "autopilot", "safety"] {
        let sel = format!("zk2/vehicle-01/{c}/@zk/**");
        eventually(&format!("the thruster reads {c}'s token"), || async {
            let r = zk2::presence::liveliness_read(bus.of("thruster-l"), &sel, T)
                .await
                .unwrap();
            r.complete && r.keys.len() == 1
        })
        .await;
    }
    let refused = zk2::presence::liveliness_read(bus.of("frontend"), "zk2/vehicle-01/*/@zk/**", T)
        .await
        .unwrap();
    assert!(refused.complete && refused.errors.is_empty(), "{refused:?}");
    assert!(refused.keys.is_empty(), "{refused:?}");

    // 7. The fan-in GET over every backend: one reply per backend, through
    //    the runtime's own selector.
    let servers = tcgui(&bus).await;
    eventually("the fan-in GET gets both backends", || async {
        fan_in(&bus).await == 2
    })
    .await;
    // A wildcard call to a fan-out-allowed operation: the frontend, granted
    // it, gets an answer per backend; teleop, not granted it, is refused at
    // the router, and nothing executes for it.
    let (granted, _) = get(
        bus.of("frontend"),
        "zk2/*/tc/tc.netif.v1/@op/diagnostics",
        QueryTarget::All,
        T,
    )
    .await;
    assert_eq!(granted.len(), 2, "{granted:?}");
    let before = servers.diagnostics.load(Ordering::SeqCst);
    let (answers, _) = get(
        bus.of("teleop"),
        "zk2/*/tc/tc.netif.v1/@op/diagnostics",
        QueryTarget::All,
        T,
    )
    .await;
    assert!(answers.is_empty());
    assert_eq!(servers.diagnostics.load(Ordering::SeqCst), before);
    // 8. A concrete call: one reply.
    eventually("the frontend calls h1's config set", || async {
        get(
            bus.of("frontend"),
            "zk2/h1/tc/tc.netem.v1/@op/config/default/eth0/set",
            QueryTarget::BestMatching,
            T,
        )
        .await
        .0
        .len()
            == 1
    })
    .await;
    // 9. A wildcard call to a fan-out-forbidden operation: two server
    //    refusals, no execution (O2), each refusal let back by fan-in-reply.
    assert_eq!(wildcard_call(&bus).await, (0, 2));
    // 10. A backend's put on the other's state never reaches the frontend.
    let (_s5, frontend_h2) = subscribe(bus.of("frontend"), "zk2/h2/tc/tc.netif.v1/state/**").await;
    let (_s6, frontend_h1) = subscribe(bus.of("frontend"), "zk2/h1/tc/tc.netif.v1/state/**").await;
    eventually("the frontend receives h1's own state", || async {
        bus.of("tc-h1")
            .put("zk2/h1/tc/tc.netif.v1/state/namespaces", "ns")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        !seen(&frontend_h1).is_empty()
    })
    .await;
    bus.of("tc-h1")
        .put("zk2/h2/tc/tc.netif.v1/state/namespaces", "impersonated")
        .await
        .unwrap();
    // … and a queryable declared under another principal's prefix serves
    //    nobody.
    let _fake = serve(bus.of("tc-h1"), "zk2/h2/tc/tc.netif.v1/state/fake", "fake").await;
    // 11. A contract bundle: any principal fetches it.
    assert_eq!(
        get(bus.of("frontend"), CONTRACT, QueryTarget::BestMatching, T)
            .await
            .0
            .len(),
        1
    );
    // 12. R2: vehicle-01's executor reads its own plan, and not vehicle-02's.
    let mut plans = Vec::new();
    for v in ["vehicle-01", "vehicle-02"] {
        plans.push(
            serve(
                bus.of("fleet-mgr"),
                &format!("zk2/ground/fleet-mgr/mission_plan.v1/state/plans/{v}"),
                "plan",
            )
            .await,
        );
    }
    let own = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";
    eventually("executor-1 reads its plan", || async {
        get(bus.of("executor-1"), own, QueryTarget::All, T)
            .await
            .0
            .len()
            == 1
    })
    .await;
    assert!(
        get(
            bus.of("executor-1"),
            "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-02",
            QueryTarget::All,
            T
        )
        .await
        .0
        .is_empty()
    );
    assert!(
        get(
            bus.of("frontend"),
            "zk2/h2/tc/tc.netif.v1/state/fake",
            QueryTarget::All,
            T
        )
        .await
        .0
        .is_empty(),
        "a queryable under another principal's prefix"
    );
    tokio::time::sleep(SILENCE).await;
    assert!(
        seen(&frontend_h2).is_empty(),
        "h1 wrote h2's key: {:?}",
        seen(&frontend_h2)
    );
    // 13. A client without credentials is refused at the link.
    assert!(client(&bus.endpoint, None).await.is_err());

    // 0.8: a principal with Call on h1's operation reads h1's instance and
    // interface tokens, complete.
    let h1 = zk2::presence::liveliness_read(bus.of("frontend"), "zk2/h1/tc/@zk/**", T)
        .await
        .unwrap();
    assert!(h1.complete, "{h1:?}");
    assert_eq!(h1.keys.len(), 2, "{h1:?}");

    // §2.5: an advanced publisher declares its cache and token under Own,
    // and a late reader with history gets the value from the cache.
    let publisher = bus
        .of("beacon")
        .declare_publisher(POSITION)
        .cache(CacheConfig::default().max_samples(1))
        .publisher_detection()
        .await
        .expect("an advanced publisher");
    publisher.put("fix-1").await.unwrap();
    assert_eq!(history(&bus).await, ["fix-1"]);

    // #684: under deny no grant reaches the admin space; a queryable there
    // answers nobody.
    let _admin = serve(bus.of("teleop"), ADMIN, "fake").await;
    tokio::time::sleep(SILENCE).await;
    assert!(
        get(&bus._router, ADMIN, QueryTarget::All, T)
            .await
            .0
            .is_empty()
    );
    drop(tokens);
}

/// What a late reader with history receives from the beacon's cache.
async fn history(bus: &Bus) -> Vec<String> {
    let got: Arc<Mutex<Vec<String>>> = Arc::default();
    let g = Arc::clone(&got);
    let _sub = bus
        .of("reader")
        .declare_subscriber("zk2/*/beacon/beacon.v1/state/position")
        .history(HistoryConfig::default().detect_late_publishers())
        .callback(move |s| {
            g.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&s.payload().to_bytes()).into_owned())
        })
        .await
        .expect("an advanced subscriber");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while got.lock().unwrap().is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let out = got.lock().unwrap().clone();
    out
}

/// 0.8, the step security.md §1 added after S14: with the frontend's
/// liveliness reads removed from its grant, the same read of h1's tokens is
/// answered complete and empty, which is why the grant holds them (§11.1).
/// And without the reader's history grant, a late reader gets nothing from
/// the cache.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_its_presence_grant_a_read_is_complete_and_empty() {
    let plan = without_rule(
        without_rule(plan(AclPermission::Deny), "presence-in:ops/frontend"),
        "history-in:ops/reader",
    );
    let bus = Bus::up(&plan).await;
    let _servers = tcgui(&bus).await;
    // The control: the router holds h1's tokens, and the frontend's calls
    // still go through.
    eventually("the router holds h1's tokens", || async {
        zk2::presence::liveliness_read(&bus._router, "zk2/h1/tc/@zk/**", T)
            .await
            .unwrap()
            .keys
            .len()
            == 2
    })
    .await;
    eventually("the frontend still calls h1", || async {
        get(
            bus.of("frontend"),
            "zk2/h1/tc/tc.netem.v1/@op/config/default/eth0/set",
            QueryTarget::BestMatching,
            T,
        )
        .await
        .0
        .len()
            == 1
    })
    .await;
    let read = zk2::presence::liveliness_read(bus.of("frontend"), "zk2/h1/tc/@zk/**", T)
        .await
        .unwrap();
    assert!(
        read.complete && read.errors.is_empty(),
        "answered, not timed out: {read:?}"
    );
    assert!(
        read.keys.is_empty(),
        "refused, and indistinguishable from absence: {read:?}"
    );

    let publisher = bus
        .of("beacon")
        .declare_publisher(POSITION)
        .cache(CacheConfig::default().max_samples(1))
        .publisher_detection()
        .await
        .expect("an advanced publisher");
    publisher.put("fix-1").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        history(&bus).await.is_empty(),
        "no history grant, no history"
    );
}

/// The allow posture, security.md §2: each grant compiled into denies of
/// its complement. Every unauthorized action is blocked except a put on a
/// wildcard key; measured beside it, a GET wider than every deny reads what
/// it selects, as the plan's warning says.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_allow_the_complement_is_denied_but_a_wildcard_is_in_no_deny() {
    let plan = plan(AclPermission::Allow);
    assert!(
        plan.rules
            .iter()
            .all(|r| r.permission == AclPermission::Deny)
    );
    let bus = Bus::up(&plan).await;

    let (_s1, from_teleop) = subscribe(bus.of("thruster-l"), &cmd("teleop")).await;
    let (_s2, from_autopilot) = subscribe(bus.of("thruster-l"), &cmd("autopilot")).await;
    eventually("the thruster receives teleop's own command", || async {
        bus.of("teleop").put(cmd("teleop"), "t").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        !seen(&from_teleop).is_empty()
    })
    .await;
    bus.of("teleop")
        .put(cmd("autopilot"), "impersonated")
        .await
        .unwrap();
    let (_s3, teleop_reads_status) = subscribe(bus.of("teleop"), STATUS).await;
    let (_s4, teleop_firehose) = subscribe(bus.of("teleop"), "zk2/**").await;
    let (_s5, thruster_reads_status) = subscribe(bus.of("thruster-l"), STATUS).await;
    eventually(
        "the thruster's own status reaches its own subscriber",
        || async {
            bus.of("thruster-l").put(STATUS, "ok").await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
            !seen(&thruster_reads_status).is_empty()
        },
    )
    .await;
    tokio::time::sleep(SILENCE).await;
    let impersonated: Vec<String> = seen(&from_autopilot);
    assert!(impersonated.is_empty(), "impersonation: {impersonated:?}");
    assert!(
        seen(&teleop_reads_status).is_empty(),
        "an unbound subscription"
    );
    assert!(
        !seen(&teleop_firehose).iter().any(|k| k == STATUS),
        "a put is checked against its own key on egress: `zk2/**` does not escape it"
    );
    // The exception: a put on a wildcard key is in no deny (§11.3), and
    // reaches the subscription; R6 discards it at the consumer.
    bus.of("teleop")
        .put("zk2/vehicle-01/*/twist_cmd.v1/stream/cmd", "wild")
        .await
        .unwrap();
    eventually(
        "the wildcard put reaches autopilot's subscription",
        || async { seen(&from_autopilot).iter().any(|k| k.contains('*')) },
    )
    .await;

    let mut tokens = Vec::new();
    for c in ["teleop", "autopilot", "safety"] {
        tokens.push(
            bus.of(c)
                .liveliness()
                .declare_token(format!(
                    "zk2/vehicle-01/{c}/@zk/instance/00000000000000{:02x}",
                    c.len()
                ))
                .await
                .unwrap(),
        );
    }
    eventually("the thruster reads its commanders' tokens", || async {
        zk2::presence::liveliness_read(bus.of("thruster-l"), "zk2/vehicle-01/*/@zk/**", T)
            .await
            .unwrap()
            .keys
            .len()
            == 3
    })
    .await;
    // The frontend's selector is wider than every deny, so the read crosses
    // the ingress check, and each token is denied on egress by its own key.
    let read = zk2::presence::liveliness_read(bus.of("frontend"), "zk2/vehicle-01/*/@zk/**", T)
        .await
        .unwrap();
    assert!(read.keys.is_empty(), "{read:?}");

    let servers = tcgui(&bus).await;
    eventually("the fan-in GET gets both backends", || async {
        fan_in(&bus).await == 2
    })
    .await;
    assert_eq!(wildcard_call(&bus).await, (0, 2));
    assert_eq!(
        get(bus.of("frontend"), CONTRACT, QueryTarget::BestMatching, T)
            .await
            .0
            .len(),
        1
    );
    let (_s6, frontend_h2) = subscribe(bus.of("frontend"), "zk2/h2/tc/tc.netif.v1/state/**").await;
    let (_s7, frontend_h1) = subscribe(bus.of("frontend"), "zk2/h1/tc/tc.netif.v1/state/**").await;
    eventually("the frontend receives h1's own state", || async {
        bus.of("tc-h1")
            .put("zk2/h1/tc/tc.netif.v1/state/namespaces", "ns")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        !seen(&frontend_h1).is_empty()
    })
    .await;
    bus.of("tc-h1")
        .put("zk2/h2/tc/tc.netif.v1/state/namespaces", "impersonated")
        .await
        .unwrap();

    let mut plans = Vec::new();
    for v in ["vehicle-01", "vehicle-02"] {
        plans.push(
            serve(
                bus.of("fleet-mgr"),
                &format!("zk2/ground/fleet-mgr/mission_plan.v1/state/plans/{v}"),
                "plan",
            )
            .await,
        );
    }
    eventually("executor-1 reads its plan", || async {
        get(
            bus.of("executor-1"),
            "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01",
            QueryTarget::All,
            T,
        )
        .await
        .0
        .len()
            == 1
    })
    .await;
    assert!(
        get(
            bus.of("executor-1"),
            "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-02",
            QueryTarget::All,
            T
        )
        .await
        .0
        .is_empty(),
        "the other vehicle's slice, denied by name"
    );
    // Measured: a GET whose selector is wider than every deny passes the
    // ingress check and reaches the backends, but a value reply is checked
    // against its own key on the way back, so the complement denies it.
    let (leak, _) = get(
        bus.of("teleop"),
        "zk2/*/tc/tc.netif.v1/state/**",
        QueryTarget::All,
        T,
    )
    .await;
    assert!(
        leak.is_empty(),
        "a value reply is denied by its own key: {leak:?}"
    );
    // Measured: the same holds for a call's answers, and not for the call.
    // A wildcard call to an operation that allows fan-out is in no deny: it
    // executes on every backend for a principal the plan never granted, and
    // only its answers are denied (O2 refuses fan-out to the others).
    let before = servers.diagnostics.load(Ordering::SeqCst);
    let (answers, _) = get(
        bus.of("teleop"),
        "zk2/*/tc/tc.netif.v1/@op/diagnostics",
        QueryTarget::All,
        T,
    )
    .await;
    assert!(answers.is_empty(), "{answers:?}");
    assert_eq!(
        servers.diagnostics.load(Ordering::SeqCst) - before,
        2,
        "the call executed on both backends"
    );
    // The control: the frontend, granted the call, gets both answers.
    let (granted, _) = get(
        bus.of("frontend"),
        "zk2/*/tc/tc.netif.v1/@op/diagnostics",
        QueryTarget::All,
        T,
    )
    .await;
    assert_eq!(granted.len(), 2, "{granted:?}");
    tokio::time::sleep(SILENCE).await;
    assert!(
        seen(&frontend_h2).is_empty(),
        "h1 wrote h2's key: {:?}",
        seen(&frontend_h2)
    );
    assert!(client(&bus.endpoint, None).await.is_err());
    drop(tokens);
}

/// U21, security.md §2: under allow, allow rules are not evaluated, so the
/// deny posture's grants under `default_permission: allow` block nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_allow_the_allow_rules_alone_block_nothing() {
    let mut plan = plan(AclPermission::Deny);
    plan.default_permission = AclPermission::Allow;
    let bus = Bus::up(&plan).await;
    let (_s, from_autopilot) = subscribe(bus.of("thruster-l"), &cmd("autopilot")).await;
    let (_t, teleop_reads_status) = subscribe(bus.of("teleop"), STATUS).await;
    eventually("teleop's impersonation reaches the thruster", || async {
        bus.of("teleop")
            .put(cmd("autopilot"), "impersonated")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        !seen(&from_autopilot).is_empty()
    })
    .await;
    eventually("teleop reads the thruster's status", || async {
        bus.of("thruster-l").put(STATUS, "ok").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        !seen(&teleop_reads_status).is_empty()
    })
    .await;
}

/// The generator check, security.md §2: without the frontend's wildcard
/// selector in a provider's egress grant, the fan-in GET gets no reply from
/// that provider; with it restored and the matching ingress reply grant
/// removed, a refusal never reaches the caller.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_generator_check_egress_and_reply() {
    let selector = "zk2/*/tc/tc.netif.v1/state/**";
    {
        let plan = without_key(plan(AclPermission::Deny), "fan-in:", selector);
        let bus = Bus::up(&plan).await;
        let _servers = tcgui(&bus).await;
        // The control: the same backends answer a concrete GET, and the
        // frontend's call goes through.
        eventually("a concrete GET reaches h1", || async {
            get(
                bus.of("frontend"),
                "zk2/h1/tc/tc.netif.v1/state/namespaces",
                QueryTarget::All,
                T,
            )
            .await
            .0
            .len()
                == 1
        })
        .await;
        assert_eq!(fan_in(&bus).await, 0, "the fan-in GET gets 0 replies");
    }
    {
        let base = plan(AclPermission::Deny);
        let bus = Bus::up(&base).await;
        let _servers = tcgui(&bus).await;
        eventually("the fan-in GET gets both backends", || async {
            fan_in(&bus).await == 2
        })
        .await;
        assert_eq!(wildcard_call(&bus).await, (0, 2));
    }
    {
        let plan = without_rule(
            without_rule(plan(AclPermission::Deny), "fan-in-reply:h1/tc"),
            "fan-in-reply:h2/tc",
        );
        let bus = Bus::up(&plan).await;
        let _servers = tcgui(&bus).await;
        eventually("a concrete call reaches h1", || async {
            get(
                bus.of("frontend"),
                "zk2/h1/tc/tc.netem.v1/@op/config/default/eth0/set",
                QueryTarget::BestMatching,
                T,
            )
            .await
            .0
            .len()
                == 1
        })
        .await;
        assert_eq!(
            wildcard_call(&bus).await,
            (0, 0),
            "a refusal never reaches the caller"
        );
        // Measured: a value reply carries its own key and is checked against
        // it, which Own includes; an error reply carries none and is checked
        // against the query's. The reply grant is for the refusals.
        assert_eq!(
            fan_in(&bus).await,
            2,
            "a value reply is checked by its own key"
        );
    }
}

/// #684 (F-80): under allow, every principal's queryable in the admin space
/// is denied. The control, without the rule: a plain client answers a
/// read of `@/<zid>/router`, which is how a tool's check turns from
/// unobservable into a false clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn under_allow_no_principal_answers_the_admin_space() {
    for (plan, answered) in [
        (plan(AclPermission::Allow), 0),
        (
            without_rule(plan(AclPermission::Allow), "deny-admin-space"),
            1,
        ),
    ] {
        let bus = Bus::up(&plan).await;
        let _admin = serve(bus.of("teleop"), ADMIN, "fake").await;
        let _control = serve(bus.of("teleop"), &cmd("teleop"), "own").await;
        // The tool is the router's own session: no principal's grants decide
        // its query, so what answers is what declared.
        eventually("teleop's own queryable is reachable", || async {
            get(&bus._router, &cmd("teleop"), QueryTarget::All, T)
                .await
                .0
                .len()
                == 1
        })
        .await;
        let replies: BTreeSet<String> = get(&bus._router, ADMIN, QueryTarget::All, T)
            .await
            .0
            .into_iter()
            .collect();
        assert_eq!(replies.len(), answered, "{replies:?}");
    }
}
