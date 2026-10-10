//! zk2's doctor against live services (#612, FJ6): each check's finding and
//! its clean pole, with services brought up by the runtime's own
//! `ServiceBuilder`, and the unobservable cases the tooling guide names.
//!
//! The scenario setups are the runtime's (#610, #620): operations.md §8 for
//! split-brain, presence.md §1, §2 and §5 for tokens, descriptors and the
//! tokenless set, state.md §1, §3 and §5 for stamps, storages and an
//! archive's alignment. Where the runtime refuses to make a violation — a
//! descriptor that fails its own check, an exposed interface with no
//! token, an unstamped state reply — a raw zenoh session plays the
//! offending owner: a tool meets those on a bus the runtime does not
//! control.
//!
//! One router per case, owners and tools as its clients, nothing named by
//! port.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

mod util;
use util::zk2::{T, client, client_ns, config, eventually, example, iface};

use serde_json::{Value, json};
use zenkey::model::descriptor::Cause;
use zenkey::model::template::Bindings;
use zenkey::{Implementation, Service, ServiceBuilder};
use zenkey_fleet::report::{CheckId, CheckReport, DoctorReport, DoctorSeverity, Judgement};
use zenkey_fleet::{BundleStore, DoctorBus, DoctorSpec, run_doctor};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, load_path};
use zenoh::Wait;

/// The two presence reads' spacing in these cases: above the sub-second
/// overlap of a re-mint (§6), short enough to keep the suite quick.
const GRACE: Duration = Duration::from_millis(600);

/// A router on an ephemeral port, its admin space on or off (zenoh's
/// default is off: the deployment an explorer most often meets).
async fn router(admin: bool) -> (zenoh::Session, String) {
    router_to(admin, None).await
}

/// [`router`], linked to `upstream` when given.
async fn router_to(admin: bool, upstream: Option<&str>) -> (zenoh::Session, String) {
    let mut c = zenoh::Config::default();
    if let Some(up) = upstream {
        c.insert_json5("connect/endpoints", &format!("[\"{up}\"]"))
            .expect("config");
    }
    for (k, v) in [
        ("scouting/multicast/enabled", "false"),
        ("scouting/gossip/enabled", "false"),
        ("mode", "\"router\""),
        ("listen/endpoints", r#"["tcp/127.0.0.1:0"]"#),
        ("adminspace/enabled", if admin { "true" } else { "false" }),
    ] {
        c.insert_json5(k, v).expect("config");
    }
    let r = zenoh::open(c).await.expect("router");
    let ep = r
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .expect("the router listens on loopback");
    (r, ep)
}

/// A contract from the runtime's scenario suites (`zenkey/tests/contracts`).
fn scenario(name: &str) -> Contract {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../zenkey/tests/contracts/{name}.toml"));
    let l = load_path(&path);
    l.contract
        .unwrap_or_else(|| panic!("{name} does not load:\n{}", l.report))
}

fn load(text: &str) -> Contract {
    let l = zenkey_model::contract::load_str(text, Path::new("."), None);
    l.contract.unwrap_or_else(|| panic!("{}", l.report))
}

/// Brings up `cfg` implementing every contract, every resource exposed.
async fn bring_up(
    session: &zenoh::Session,
    cfg: zenkey::ServiceConfig,
    cs: &[Contract],
) -> Service {
    let mut b = ServiceBuilder::new(session, cfg);
    for c in cs {
        let id = c.iface.clone();
        let names: Vec<String> = c
            .resources
            .iter()
            .map(zenkey::implementation::resource_name)
            .collect();
        b.implement(Implementation::new(c.clone()))
            .expect("implement");
        for n in names {
            let _ = b.expose(&id, &n);
        }
    }
    b.start().await.expect("start")
}

fn bus(session: &zenoh::Session, raw: &zenoh::Session, namespace: &str) -> DoctorBus {
    DoctorBus {
        session: session.clone(),
        raw: raw.clone(),
        namespace: namespace.to_owned(),
    }
}

fn spec(checks: &[CheckId]) -> DoctorSpec {
    let mut s = DoctorSpec::new(T);
    s.grace = GRACE;
    if !checks.is_empty() {
        s.checks = checks.iter().copied().collect();
    }
    s
}

async fn doctor(bus: &DoctorBus, spec: &DoctorSpec) -> DoctorReport {
    run_doctor(bus, &BundleStore::new(T), spec).await
}

fn verdict(r: &DoctorReport, id: CheckId) -> &CheckReport {
    r.check(id).expect("every check is reported")
}

#[track_caller]
fn found<'r>(
    r: &'r DoctorReport,
    id: CheckId,
    subject: &str,
) -> &'r zenkey_fleet::report::DoctorFinding {
    let c = verdict(r, id);
    assert_eq!(c.verdict, Judgement::Established, "{id}: {c:#?}");
    c.findings
        .iter()
        .find(|f| f.subject == subject)
        .unwrap_or_else(|| panic!("{id}: no finding on {subject:?}: {c:#?}"))
}

#[track_caller]
fn clean(r: &DoctorReport, id: CheckId) -> String {
    match &verdict(r, id).verdict {
        Judgement::NotEstablished { reason } => reason.clone(),
        other => panic!("{id} is not clean: {other:?}\n{:#?}", verdict(r, id)),
    }
}

#[track_caller]
fn unseen(r: &DoctorReport, id: CheckId) -> String {
    match &verdict(r, id).verdict {
        Judgement::Unobservable { reason } => reason.clone(),
        other => panic!("{id} is not unobservable: {other:?}\n{:#?}", verdict(r, id)),
    }
}

/// Waits until `selector`'s liveliness read holds exactly `n` tokens: a
/// service is up when presence says so, not when `start()` returned.
async fn tokens(tool: &zenoh::Session, selector: &str, n: usize) {
    eventually(&format!("{n} token(s) on {selector}"), || async {
        zenkey::presence::liveliness_keys(tool, selector, T)
            .await
            .is_ok_and(|k| k.len() == n)
    })
    .await;
}

/// Waits until every one of `addresses` is listed with its descriptor
/// served.
async fn settled(tool: &zenoh::Session, addresses: &[&str]) {
    let want: BTreeSet<&str> = addresses.iter().copied().collect();
    eventually("every address listed with its descriptor", || async {
        let Ok(l) =
            zenkey_fleet::service_listing(tool, &zenkey_fleet::PresenceScope::all(), T).await
        else {
            return false;
        };
        let served: BTreeSet<&str> = l
            .services
            .iter()
            .filter(|s| {
                s.instances.iter().all(|i| {
                    matches!(
                        i.descriptor,
                        zenkey_fleet::report::Asked::Asked(
                            zenkey_fleet::report::DescriptorAnswer::Served { .. }
                        )
                    )
                })
            })
            .map(|s| s.address.as_str())
            .collect();
        want.iter().all(|a| served.contains(a))
    })
    .await;
}

/// An owner the runtime would not build: a raw session holding the
/// instance token and answering `descriptor` on its key, and whatever else
/// a case adds.
struct Fake {
    _token: zenoh::liveliness::LivelinessToken,
    _descriptor: zenoh::query::Queryable<()>,
    _more: Vec<Box<dyn std::any::Any + Send>>,
}

async fn fake(session: &zenoh::Session, service: &str, instance: &str, descriptor: Value) -> Fake {
    let key = format!("zk2/{service}/@zk/instance/{instance}");
    let body = serde_json::to_vec(&descriptor).expect("JSON");
    let q = session
        .declare_queryable(key.as_str())
        .callback(move |q| {
            let _ = q.reply(q.key_expr().clone(), body.clone()).wait();
        })
        .await
        .expect("a descriptor queryable");
    let token = session
        .liveliness()
        .declare_token(key.as_str())
        .await
        .expect("an instance token");
    Fake {
        _token: token,
        _descriptor: q,
        _more: Vec::new(),
    }
}

/// A descriptor of `service@instance` listing `entries`.
fn descriptor(service: &str, instance: &str, entries: Value) -> Value {
    json!({
        "format": "zk2-descriptor/0.1",
        "service": service,
        "instance": instance,
        "interfaces": entries,
    })
}

fn entry(c: &Contract) -> Value {
    json!({
        "iface": c.iface.to_string(),
        "contract": Fingerprint::of(c).to_string(),
        "minor": c.minor.unwrap_or(0),
    })
}

// ─── split-brain: operations.md §8 ──────────────────────────────────────────

/// operations.md §8: two instances holding one interface's token past the
/// grace period are a finding; a re-mint, whose overlap is shorter, is
/// not; nor is a standby, nor two instances of an interface whose only
/// resource is replicated, nor a replica beside the instance serving the
/// exclusive resource. Nothing is fenced.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn split_brain_is_diagnosed_as_operations_8_runs_it() {
    let (_r, ep) = router(false).await;
    let tool = client(&ep).await;
    let mut s = Vec::new();
    for _ in 0..7 {
        s.push(client(&ep).await);
    }
    let on = bus(&tool, &tool, "");
    let only = spec(&[CheckId::SplitBrain]);
    let tc = scenario("tc.v1");
    let alive = "zk2/h1/tc/@zk/alive/**";

    // 1. Two instances of h1/tc, both holding tc.v1's token.
    let mut a = bring_up(&s[0], config("h1/tc"), std::slice::from_ref(&tc)).await;
    let b = bring_up(&s[1], config("h1/tc"), std::slice::from_ref(&tc)).await;
    tokens(&tool, alive, 2).await;
    let r = doctor(&on, &only).await;
    let f = found(&r, CheckId::SplitBrain, "h1/tc tc.v1");
    assert_eq!(f.severity, DoctorSeverity::Error);
    for i in [a.instance(), b.instance()] {
        assert!(f.evidence.contains(i.as_str()), "{f:?}");
    }
    assert_eq!(a.tokens_held(), [iface("tc.v1")], "nothing was fenced");
    assert_eq!(b.tokens_held(), [iface("tc.v1")], "nothing was fenced");
    b.close().await.expect("close");
    tokens(&tool, alive, 1).await;

    // 2. The remaining instance re-mints during the check.
    let check = tokio::spawn({
        let (on, only) = (on.clone(), only.clone());
        async move { doctor(&on, &only).await }
    });
    tokio::time::sleep(GRACE / 3).await;
    a.new_epoch().await.expect("a re-mint");
    let r = check.await.expect("the doctor");
    clean(&r, CheckId::SplitBrain);

    // 3. A standby: an instance token, no interface token.
    let _standby = ServiceBuilder::new(&s[2], config("h1/tc"))
        .start()
        .await
        .expect("a standby");
    tokens(&tool, "zk2/h1/tc/@zk/instance/*", 2).await;
    let r = doctor(&on, &only).await;
    clean(&r, CheckId::SplitBrain);

    // 4. Two instances of an interface whose only resource is replicated.
    let ping = scenario("ping.v1");
    let _p1 = bring_up(&s[3], config("h2/pinger"), std::slice::from_ref(&ping)).await;
    let _p2 = bring_up(&s[4], config("h2/pinger"), std::slice::from_ref(&ping)).await;
    // 5. A replica beside the instance serving the exclusive operation.
    let rep = scenario("tc-replicated");
    let _full = bring_up(&s[5], config("h3/tc"), std::slice::from_ref(&rep)).await;
    let mut replica = ServiceBuilder::new(&s[6], config("h3/tc"));
    replica
        .implement(Implementation::new(rep.clone()))
        .expect("implement");
    replica
        .unavailable(
            &iface("tc.v1"),
            "@op/interfaces/{if}/set",
            Cause::Config,
            Some("a replica"),
        )
        .expect("set is optional");
    for r in &rep.resources {
        let name = zenkey::implementation::resource_name(r);
        if name != "@op/interfaces/{if}/set" {
            let _ = replica.expose(&iface("tc.v1"), &name);
        }
    }
    let _replica = replica.start().await.expect("the replica");
    tokens(&tool, "zk2/h2/pinger/@zk/alive/**", 2).await;
    tokens(&tool, "zk2/h3/tc/@zk/alive/**", 2).await;
    let r = doctor(&on, &only).await;
    let why = clean(&r, CheckId::SplitBrain);
    assert!(why.contains("replicated serving"), "{why}");
}

// ─── presence.md §1, §2, §5: a deployment that keeps the rules ──────────────

/// The tcgui pilot, as FJ4 reads it: two hosts' `tc` (host-b with
/// `tc.netem.v1` tokenless, presence.md §5) and the frontend binding three
/// roles to `*/tc` — `scenario` to an interface nobody provides.
async fn tcgui(owners: &zenoh::Session) -> Vec<Service> {
    let both = [example("tcgui/tc.netif.v1"), example("tcgui/tc.netem.v1")];
    let a = bring_up(owners, config("host-a/tc"), &both).await;
    let b = bring_up(
        owners,
        config("host-b/tc").tokenless(iface("tc.netem.v1")),
        &both,
    )
    .await;
    let mut gui = ServiceBuilder::new(
        owners,
        config("ws-01/tcgui-frontend")
            .bind("netif", &["*/tc"])
            .bind("netem", &["*/tc"])
            .bind("scenario", &["*/tc"]),
    );
    gui.require("netif", iface("tc.netif.v1"), false);
    gui.require("netem", iface("tc.netem.v1"), false);
    gui.require("scenario", iface("tc.scenario.v1"), true);
    vec![a, b, gui.start().await.expect("the frontend")]
}

const TCGUI: [&str; 3] = ["host-a/tc", "host-b/tc", "ws-01/tcgui-frontend"];

/// presence.md §1, §2 and §5 hold, so tokens, descriptors and retrieval are
/// clean — the tokenless interface included; the frontend's `scenario`
/// role selects no provider, which is the finding; and with the routers'
/// admin space off, the checks that read it are unobservable, never clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_conforming_deployment_is_clean_and_an_unreachable_admin_space_is_no_verdict() {
    let (_r, ep) = router(false).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let _services = tcgui(&owners).await;
    settled(&tool, &TCGUI).await;

    let r = doctor(&bus(&tool, &tool, ""), &spec(&[])).await;
    for id in [
        CheckId::SplitBrain,
        CheckId::ContractDrift,
        CheckId::ContractUnavailable,
        CheckId::DescriptorInvalid,
        CheckId::TokenMissing,
        CheckId::PresenceOverBudget,
        CheckId::ArchiveUnaligned,
    ] {
        clean(&r, id);
    }
    assert!(clean(&r, CheckId::ContractUnavailable).starts_with("2 revision(s)"));
    let f = found(
        &r,
        CheckId::BindingUnsatisfied,
        "ws-01/tcgui-frontend scenario",
    );
    assert_eq!(
        f.severity,
        DoctorSeverity::Info,
        "an optional manifest role, as its descriptor says (§3.3, 0.10)"
    );
    assert_eq!(verdict(&r, CheckId::BindingUnsatisfied).findings.len(), 1);
    // The admin space is off: a finding worth knowing, and no verdict on S4.
    assert_eq!(
        found(&r, CheckId::AdminUnreachable, "admin space").severity,
        DoctorSeverity::Info
    );
    assert!(unseen(&r, CheckId::StorageOnState).contains("no router answered"));
    assert!(unseen(&r, CheckId::RouterVersionSkew).contains("no router answered"));
    assert!(
        verdict(&r, CheckId::StateStampForeign)
            .verdict
            .is_not_asked()
    );
    assert!(r.unobservable.is_none());
    let scope = r.scope.presence.as_option().expect("presence was read");
    assert_eq!(scope.services, 3);
    assert!(scope.complete);
    // The optional binding's info is the run's finding under `--fail-on
    // info`; above that, the unobservable admin checks make it no verdict.
    let exit = |floor| zenkey_fleet::judgement_exit_code(&r.judgement(floor));
    assert_eq!(exit(DoctorSeverity::Info), 1);
    assert_eq!(exit(DoctorSeverity::Warning), 2);
    assert_eq!(exit(DoctorSeverity::Error), 2);

    // §8.3, against a budget this deployment is over.
    let mut tight = spec(&[CheckId::PresenceOverBudget]);
    tight.presence_budget = 3;
    let r = doctor(&bus(&tool, &tool, ""), &tight).await;
    let f = found(&r, CheckId::PresenceOverBudget, "presence domain");
    assert!(f.evidence.contains("the budget is 3"), "{f:?}");
}

// ─── what the runtime refuses to make ───────────────────────────────────────

/// presence.md §2's refusals, met on a bus the runtime does not control:
/// an instance exposing `tc.netif.v1` with no interface token (§8.1), and
/// one whose descriptor lists a resource its contract does not declare
/// `unavailable` (D005, §3.3) — each judged against the contract a real
/// owner serves.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_token_and_an_invalid_descriptor_are_findings() {
    let (_r, ep) = router(false).await;
    let (owners, tool, raw) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let netif = example("tcgui/tc.netif.v1");
    let _real = bring_up(&owners, config("host-a/tc"), std::slice::from_ref(&netif)).await;
    let tokenless = "00000000000000f1";
    let _no_token = fake(
        &raw,
        "h2/fake",
        tokenless,
        descriptor("h2/fake", tokenless, json!([entry(&netif)])),
    )
    .await;
    let bad = "00000000000000f2";
    let mut listed = entry(&netif);
    listed["unavailable"] = json!([{"resource": "state/nope", "cause": "config"}]);
    let _bad = fake(
        &raw,
        "h3/bad",
        bad,
        descriptor("h3/bad", bad, json!([listed])),
    )
    .await;
    settled(&tool, &["host-a/tc", "h2/fake", "h3/bad"]).await;

    let r = doctor(
        &bus(&tool, &tool, ""),
        &spec(&[CheckId::TokenMissing, CheckId::DescriptorInvalid]),
    )
    .await;
    let f = found(
        &r,
        CheckId::TokenMissing,
        &format!("h2/fake@{tokenless} tc.netif.v1"),
    );
    assert!(f.evidence.contains("holds no interface token"), "{f:?}");
    assert!(
        !verdict(&r, CheckId::TokenMissing)
            .findings
            .iter()
            .any(|f| f.subject.starts_with("host-a/tc")),
        "the real owner holds its token"
    );
    let f = found(&r, CheckId::DescriptorInvalid, &format!("h3/bad@{bad}"));
    assert!(f.evidence.starts_with("D005"), "{f:?}");
    assert_eq!(verdict(&r, CheckId::DescriptorInvalid).findings.len(), 1);
}

/// A bundle that cannot be retrieved (§8.4): a descriptor naming a revision
/// no holder serves is `contract-unavailable`'s finding, and the checks
/// that needed that contract leave its instance unjudged, saying why —
/// never clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bundle_nobody_serves_is_a_finding_and_its_dependents_are_unobservable() {
    let (_r, ep) = router(false).await;
    let (tool, raw) = (client(&ep).await, client(&ep).await);
    let ghost = "00000000000000f3";
    let fp = format!("sha256:{}", "0".repeat(64));
    let _ghost = fake(
        &raw,
        "h4/ghost",
        ghost,
        descriptor(
            "h4/ghost",
            ghost,
            json!([{"iface": "ghost.v1", "contract": fp, "minor": 0}]),
        ),
    )
    .await;
    settled(&tool, &["h4/ghost"]).await;

    let r = doctor(&bus(&tool, &tool, ""), &spec(&[])).await;
    let f = found(&r, CheckId::ContractUnavailable, &format!("ghost.v1 {fp}"));
    assert!(f.evidence.contains("no holder served"), "{f:?}");
    assert!(unseen(&r, CheckId::DescriptorInvalid).contains("syntax only"));
    assert!(unseen(&r, CheckId::TokenMissing).contains("needs its contract"));
    let scope = r.scope.presence.as_option().expect("presence was read");
    assert_eq!((scope.revisions, scope.held), (1, 0));
}

// ─── contract-drift: §9.8 ───────────────────────────────────────────────────

/// `probe.v1` at minor `minor`, with an optional operation when `extra`.
fn probe(minor: u32, extra: bool) -> Contract {
    let op = |name: &str, more: &str| {
        format!(
            "[resources.{name}]\nkind = \"operation\"\n{more}\
             request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n"
        )
    };
    let mut t = format!("[interface]\nname = \"probe\"\nmajor = 1\nminor = {minor}\n");
    t += &op("read", "idempotent = true\n");
    if extra {
        t += &op("extra", "optional = true\n");
    }
    load(&t)
}

/// Two providers of one interface at two revisions, through the contract
/// CI's classifier in the order their minors give: an optional operation
/// added is compatible, and the same change read the other way is the
/// finding.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn providers_at_two_revisions_are_classified() {
    let (_r, ep) = router(false).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let drift = spec(&[CheckId::ContractDrift]);
    {
        let _old = bring_up(&owners, config("p1/probe"), &[probe(0, false)]).await;
        let _new = bring_up(&owners, config("p2/probe"), &[probe(1, true)]).await;
        settled(&tool, &["p1/probe", "p2/probe"]).await;
        let r = doctor(&bus(&tool, &tool, ""), &drift).await;
        assert!(clean(&r, CheckId::ContractDrift).contains("compatible"));
    }
    tokens(&tool, "zk2/*/*/@zk/instance/*", 0).await;
    let _removed_later = bring_up(&owners, config("p3/probe"), &[probe(0, true)]).await;
    let _later = bring_up(&owners, config("p4/probe"), &[probe(1, false)]).await;
    settled(&tool, &["p3/probe", "p4/probe"]).await;
    let r = doctor(&bus(&tool, &tool, ""), &drift).await;
    let c = verdict(&r, CheckId::ContractDrift);
    assert_eq!(c.verdict, Judgement::Established, "{c:#?}");
    assert!(c.findings[0].evidence.contains("ordered by the minors"));
    assert!(c.findings[0].evidence.contains("p3/probe@"));
}

// ─── the admin space: state.md §3, in a namespace ───────────────────────────

/// An admin-space document played by a raw session: zenoh's storage
/// manager is a plugin an in-process router cannot load, and a mesh of one
/// router has one version.
async fn admin_doc(raw: &zenoh::Session, key: &str, doc: Value) -> zenoh::query::Queryable<()> {
    let body = serde_json::to_vec(&doc).expect("JSON");
    let at = key.to_owned();
    raw.declare_queryable(key)
        .callback(move |q| {
            let _ = q.reply(at.as_str(), body.clone()).wait();
        })
        .await
        .expect("an admin document")
}

/// state.md §3 step 1: a storage covering the owner's keys is a finding,
/// read from the routers' storage admin space in no namespace while the
/// deployment is read in its own; a storage on another namespace's keys is
/// not this deployment's. Routers at two versions are a skew.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_storage_on_owners_state_and_a_version_skew_are_findings() {
    let (_r, ep) = router(true).await;
    let owners = client_ns(&ep, Some("acme")).await;
    let tool = client_ns(&ep, Some("acme")).await;
    let raw = client(&ep).await;
    let _services = tcgui(&owners).await;
    settled(&tool, &TCGUI).await;
    let on = bus(&tool, &raw, "acme");
    let admin = spec(&[
        CheckId::StorageOnState,
        CheckId::AdminUnreachable,
        CheckId::RouterVersionSkew,
    ]);

    let r = doctor(&on, &admin).await;
    assert!(clean(&r, CheckId::AdminUnreachable).contains("1 router(s)"));
    assert!(clean(&r, CheckId::StorageOnState).starts_with("0 storage(s) on 1 router(s)"));
    clean(&r, CheckId::RouterVersionSkew);

    let storages = "@/f00d/router/status/plugins/storage_manager/storages";
    let _mine = admin_doc(
        &raw,
        &format!("{storages}/mine"),
        json!({"key_expr": "acme/zk2/**", "volume": "memory"}),
    )
    .await;
    let _theirs = admin_doc(
        &raw,
        &format!("{storages}/theirs"),
        json!({"key_expr": "other/zk2/**", "volume": "memory"}),
    )
    .await;
    let _old = admin_doc(
        &raw,
        "@/f00d/router",
        json!({"zid": "f00d", "version": "0.0.0-old"}),
    )
    .await;
    eventually("the storage documents answer", || async {
        zenkey_fleet::storages(&raw, T)
            .await
            .is_ok_and(|s| s.len() == 2)
    })
    .await;
    // A raw session plays `f00d`'s documents, which is what F-80 says any
    // session can do: untrusted, they decide nothing (§4.2, 0.12).
    let r = doctor(&on, &admin).await;
    assert!(
        verdict(&r, CheckId::StorageOnState)
            .verdict
            .is_unobservable()
    );
    assert!(unseen(&r, CheckId::StorageOnState).contains("f00d"));
    // Trusted on the operator's word (grants deny `@/**` queryables to
    // every principal, §11.1), they are read as a router's.
    let mut admin = admin;
    admin.trust_admin = true;
    let r = doctor(&on, &admin).await;
    let f = found(&r, CheckId::StorageOnState, "mine@f00d");
    assert!(f.evidence.contains("acme/zk2/*/*/*/state/**"), "{f:?}");
    assert_eq!(verdict(&r, CheckId::StorageOnState).findings.len(), 1);
    let f = found(&r, CheckId::RouterVersionSkew, "mesh");
    assert!(f.evidence.contains("0.0.0-old"), "{f:?}");
}

// ─── archive-unaligned: state.md §5 ─────────────────────────────────────────

/// state.md §5.2: an archive whose alignment read came back empty keeps
/// its keys and serves them `confirmed: false`, which is the finding; once
/// it recorded them from the owner, they are confirmed and the check is
/// clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_archive_serving_unconfirmed_keys_is_unaligned() {
    use zenkey::archive::{Archive, ArchiveConfig, Recorded};
    let (_r, ep) = router(false).await;
    let (owners, tool, archiving) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let netif = example("tcgui/tc.netif.v1");
    let origin = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";
    let archive = Archive::start(
        &archiving,
        ArchiveConfig {
            service: config("ground/archive"),
            records: vec![Recorded {
                owner: "host-a/tc".parse().expect("an address"),
                selector: "zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".to_owned(),
                implementation: Implementation::new(netif.clone()),
            }],
            peers: Vec::new(),
            unconfirmed_horizon: None,
        },
    )
    .await
    .expect("an archive");
    let mut b = ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(Implementation::new(netif.clone()))
        .expect("implement");
    for r in &netif.resources {
        let _ = b.expose(
            &iface("tc.netif.v1"),
            &zenkey::implementation::resource_name(r),
        );
    }
    b.serve_state(&iface("tc.netif.v1")).expect("serve state");
    let mut owner = b.start().await.expect("the owner");
    let member: Bindings = [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into();
    let writer = owner
        .state_writer(
            &iface("tc.netif.v1"),
            "state/interfaces/{ns}/{iface}",
            &member,
        )
        .await
        .expect("a state writer");
    writer
        .put_value(&json!({"name": "eth0", "is_up": true}))
        .await
        .expect("a put");
    eventually("the archive records it, confirmed", || async {
        archive.confirmed(origin) == Some(true)
    })
    .await;
    settled(&tool, &["host-a/tc", "ground/archive"]).await;
    let only = spec(&[CheckId::ArchiveUnaligned]);
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    assert!(clean(&r, CheckId::ArchiveUnaligned).contains("1 value(s), every one confirmed"));

    // Every alignment read refused: nothing confirmed, nothing dropped.
    archive.refuse_next_reads(100);
    let a = archive.align(T).await.expect("an alignment");
    assert_eq!(a.unconfirmed, 1);
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    let f = found(&r, CheckId::ArchiveUnaligned, "ground/archive");
    assert!(f.evidence.contains("1 of 1"), "{f:?}");
    assert!(f.evidence.contains("eth0"), "{f:?}");
    drop(writer);
}

// ─── state-stamp-foreign: state.md §1 (deep) ────────────────────────────────

/// state.md §1 under `deep`: the runtime's owner answers its state with its
/// own session's stamp (S1, S2), which is clean against the router this run
/// verified (§4.2, 0.17); an owner answering a state GET unstamped is the
/// finding.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_state_reply_not_stamped_by_its_owner_is_a_finding() {
    let (_r, ep) = router(true).await;
    let (owners, tool, raw) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let netif = example("tcgui/tc.netif.v1");
    let mut b = ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(Implementation::new(netif.clone()))
        .expect("implement");
    for r in &netif.resources {
        let _ = b.expose(
            &iface("tc.netif.v1"),
            &zenkey::implementation::resource_name(r),
        );
    }
    b.serve_state(&iface("tc.netif.v1")).expect("serve state");
    let mut owner = b.start().await.expect("the owner");
    let member: Bindings = [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into();
    let writer = owner
        .state_writer(
            &iface("tc.netif.v1"),
            "state/interfaces/{ns}/{iface}",
            &member,
        )
        .await
        .expect("a state writer");
    writer
        .put_value(&json!({"name": "eth0", "is_up": true}))
        .await
        .expect("a put");

    // The offender: a descriptor naming its session, and a state queryable
    // that answers unstamped.
    let instance = "00000000000000f5";
    let mut d = descriptor("h5/fake", instance, json!([entry(&netif)]));
    d["meta"] = json!({"zid": raw.zid().to_string()});
    let mut offender = fake(&raw, "h5/fake", instance, d).await;
    let q = raw
        .declare_queryable("zk2/h5/fake/tc.netif.v1/state/**")
        .callback(|q| {
            let _ = q
                .reply(
                    "zk2/h5/fake/tc.netif.v1/state/interfaces/default/eth0",
                    "{}",
                )
                .wait();
        })
        .await
        .expect("a state queryable");
    offender._more.push(Box::new(q));
    settled(&tool, &["host-a/tc", "h5/fake"]).await;

    let mut deep = spec(&[CheckId::StateStampForeign]);
    deep.deep = true;
    let r = doctor(&bus(&tool, &tool, ""), &deep).await;
    let f = found(&r, CheckId::StateStampForeign, "h5/fake tc.netif.v1");
    assert!(f.evidence.contains("unstamped"), "{f:?}");
    let c = verdict(&r, CheckId::StateStampForeign);
    assert!(
        !c.findings
            .iter()
            .any(|f| f.subject.starts_with("host-a/tc"))
            && !c
                .unjudged
                .iter()
                .any(|u| u.subject.starts_with("host-a/tc")),
        "the owner's own stamp is clean: {c:#?}"
    );
    // Without `deep`, the data plane is not asked.
    let r = doctor(&bus(&tool, &tool, ""), &spec(&[CheckId::StateStampForeign])).await;
    assert!(
        verdict(&r, CheckId::StateStampForeign)
            .verdict
            .is_not_asked()
    );
    drop(writer);
}

// ─── the empty scope ────────────────────────────────────────────────────────

/// A doctor pointed at the wrong namespace: no zk2 token visible, so every
/// check that reads presence is unobservable and the run is no verdict
/// (exit 2), however clean the rest reads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_empty_namespace_is_no_verdict() {
    let (_r, ep) = router(false).await;
    let owners = client(&ep).await;
    let elsewhere = client_ns(&ep, Some("nobody")).await;
    let tool = client(&ep).await;
    let _services = tcgui(&owners).await;
    settled(&tool, &TCGUI).await;

    let r = doctor(&bus(&elsewhere, &tool, "nobody"), &spec(&[])).await;
    let why = r.unobservable.clone().expect("the empty scope");
    assert!(why.contains("no zk2 token visible to this reader"), "{why}");
    assert!(why.contains("\"nobody\""), "{why}");
    for c in &r.checks {
        if c.check.reads_presence() && !c.verdict.is_not_asked() {
            assert!(c.verdict.is_unobservable(), "{c:?}");
        }
    }
    assert_eq!(
        zenkey_fleet::judgement_exit_code(&r.judgement(DoctorSeverity::Warning)),
        2
    );
}

// ─── binding-unsatisfied: bindings.md §1, R5 ────────────────────────────────

/// bindings.md §1 (R5): a role bound to a provider present now is clean;
/// once the provider is gone — no token, no descriptor visible to this
/// reader — the same role is the finding, graded by what the descriptor
/// says of its need (a role the component's own manifest declares carries
/// none, so a warning).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_bound_role_is_clean_while_its_provider_is_present() {
    let (_r, ep) = router(false).await;
    let (owners, consumers, tool) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let provider = bring_up(
        &owners,
        config("host-a/tc"),
        &[example("tcgui/tc.netif.v1")],
    )
    .await;
    let mut gui = ServiceBuilder::new(
        &consumers,
        config("ws-01/gui").bind("netif", &["host-a/tc"]),
    );
    gui.require("netif", iface("tc.netif.v1"), false);
    let _gui = gui.start().await.expect("the consumer");
    settled(&tool, &["host-a/tc", "ws-01/gui"]).await;
    let only = spec(&[CheckId::BindingUnsatisfied]);

    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    assert!(clean(&r, CheckId::BindingUnsatisfied).starts_with("1 bound role"));

    provider.close().await.expect("the provider closes");
    tokens(&tool, "zk2/host-a/tc/@zk/instance/*", 0).await;
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    let f = found(&r, CheckId::BindingUnsatisfied, "ws-01/gui netif");
    assert_eq!(
        f.severity,
        DoctorSeverity::Error,
        "a required manifest role (§3.3, 0.10)"
    );
    assert!(f.evidence.contains("host-a/tc"), "{f:?}");
}

/// Core §4.2 (0.12, F-80): any session can answer the admin space, a real
/// router's own key included, and the doctor trusts only a reply whose
/// replier id is the router its key names. With the router's admin space
/// off, the spoof is the only answer, and S4 stays unobservable, naming the
/// spoofer; with it on, the router answers too, and the spoof still keeps
/// the verdict from clean.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_admin_answer_a_router_did_not_send_is_never_trusted() {
    for admin_on in [false, true] {
        let (r, ep) = router(admin_on).await;
        let owners = client(&ep).await;
        let tool = client(&ep).await;
        let spoofer = client(&ep).await;
        let _services = tcgui(&owners).await;
        settled(&tool, &TCGUI).await;
        let rz = r.zid().to_string();
        let _doc = admin_doc(
            &spoofer,
            &format!("@/{rz}/router"),
            json!({"plugins": null}),
        )
        .await;
        // Wait for the spoofer's own answer, by its replier id: with the
        // admin space on, the router answers first, and any answer would do.
        let sz = spoofer.zid().to_string();
        eventually("the spoofed document answers", || async {
            zenkey_fleet::admin_get(&tool, &format!("@/{rz}/router"), T)
                .await
                .is_ok_and(|e| e.iter().any(|a| a.replier.as_deref() == Some(sz.as_str())))
        })
        .await;
        let r = doctor(
            &bus(&tool, &tool, ""),
            &spec(&[CheckId::StorageOnState, CheckId::AdminUnreachable]),
        )
        .await;
        let s4 = verdict(&r, CheckId::StorageOnState);
        assert!(s4.verdict.is_unobservable(), "admin {admin_on}: {s4:#?}");
        let why = unseen(&r, CheckId::StorageOnState);
        assert!(
            why.contains(&spoofer.zid().to_string()),
            "admin {admin_on}: names the spoofer: {why}"
        );
        if admin_on {
            assert!(clean(&r, CheckId::AdminUnreachable).contains("1 router(s)"));
        }
    }
}

/// Core §4.2 (0.13, F-81): a far router is verified through a verified
/// router's document, which lists it as a `router` session. With R2 linked
/// to R1 and a client tool on R1, both routers' answers count, and S4 is
/// clean over both; a client answering `@/<its own zid>/router` under its
/// own replier id is listed by R1 as a `client`, and is never trusted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_far_router_is_verified_through_the_router_that_lists_it() {
    let (r1, ep) = router(true).await;
    let (r2, _) = router_to(true, Some(&ep)).await;
    let (owners, tool) = (client(&ep).await, client(&ep).await);
    let _services = tcgui(&owners).await;
    settled(&tool, &TCGUI).await;
    let asked = spec(&[CheckId::StorageOnState, CheckId::AdminUnreachable]);
    eventually("both routers answer", || async {
        zenkey_fleet::admin_get(&tool, "@/*/router", T)
            .await
            .is_ok_and(|e| e.len() == 2)
    })
    .await;
    let r = doctor(&bus(&tool, &tool, ""), &asked).await;
    assert!(
        clean(&r, CheckId::AdminUnreachable).contains("2 router(s)"),
        "{r:#?}"
    );
    assert!(clean(&r, CheckId::StorageOnState).contains("on 2 router(s)"));

    let spoofer = client(&ep).await;
    let sz = spoofer.zid().to_string();
    let _doc = admin_doc(
        &spoofer,
        &format!("@/{sz}/router"),
        json!({"plugins": null}),
    )
    .await;
    eventually("the spoofer answers too", || async {
        zenkey_fleet::admin_get(&tool, "@/*/router", T)
            .await
            .is_ok_and(|e| e.len() == 3)
    })
    .await;
    let r = doctor(&bus(&tool, &tool, ""), &asked).await;
    let why = unseen(&r, CheckId::StorageOnState);
    assert!(
        why.contains(&sz) && why.contains("no verified router lists it as a router"),
        "{why}"
    );
    drop((r1, r2));
}

// ── hostid.v1 §2.12 (#721, PF) ─────────────────────────────────────────────

/// `spec/profiles/hostid/scenarios.md`'s machine ids (each a case of its
/// `conformance/vectors.json`).
const M1: &str = "b642b4217b34b1e8d3bd915fc65c4452";
const M2: &str = "0123456789abcdef0123456789abcdef";

/// A root standing in for `/`, its `etc/machine-id` holding `id` and a
/// newline, and `var/lib` in it (the scenarios' conventions).
fn root(id: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NTH: AtomicU64 = AtomicU64::new(0);
    let r = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "hostid-s6-{}-{}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(r.join("etc")).expect("etc");
    std::fs::create_dir_all(r.join("var/lib")).expect("var/lib");
    std::fs::write(r.join("etc/machine-id"), format!("{id}\n")).expect("machine-id");
    r
}

/// `@hostid.v1/sysinfo` on `root`, in a session of its own: a pure
/// consumer, its `meta.host` `host`, its `meta.zid` its session's unless
/// `zid` is false; implementing `also` in its tokenless set when given.
async fn sysinfo(
    ep: &str,
    root: &std::path::Path,
    host: &str,
    zid: bool,
    also: Option<&Contract>,
) -> (zenoh::Session, Service) {
    let s = client(ep).await;
    let name = zenkey::model::grammar::Name::new("service", "sysinfo").expect("a name");
    let mut cfg = zenkey::ServiceConfig::minted(name);
    cfg.meta.insert("host".into(), json!(host));
    if !zid {
        // What the deployment states in `meta` wins: a null zid is no zid
        // stated.
        cfg.meta.insert("zid".into(), Value::Null);
    }
    if let Some(c) = also {
        cfg = cfg.tokenless(c.iface.clone());
    }
    let minter = zenkey::hostid::HostIdMinter::new(zenkey::hostid::HostIdSource::at(root));
    let mut b = ServiceBuilder::with_hostid(&s, cfg, &minter);
    if let Some(c) = also {
        b.implement(Implementation::new(c.clone()))
            .expect("implement");
        let _ = b.expose(&c.iface, "stream/cpu");
    }
    let svc = b.start().await.expect("it starts");
    (s, svc)
}

/// `spec/profiles/hostid/scenarios.md` §6 (#721, PF): what a tool — the
/// doctor's `hostid-duplicate` — concludes. Two roots holding one machine
/// id, as two hosts booted from one image, each running `sysinfo` in a
/// session of its own: two instances of one minted address with different
/// `meta.zid` in both reads, the finding, its cause undecided and never
/// named. The control (B holding M2) is two systems and no finding; B
/// stating no `meta.zid` leaves the address undecided; a literal system in
/// the minted shape is not minted, and not this profile's; and, owners of
/// an interface whose contract lists `hostid.v1` in `uses`, neither
/// instance is counted, so the address is undecided too. The scenario's
/// `sysinfo-x.v1` is spelled `sysinfo_x.v1`: an interface name's segments
/// are `[a-z][a-z0-9_]*` (E001).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostid_s6_what_a_tool_concludes() {
    const S1: &str = "h-bbd1aa1db10b/sysinfo";
    let only = spec(&[CheckId::HostidDuplicate]);
    let (a_root, b_root) = (root(M1), root(M1));

    // Step 1: one machine id, two hosts.
    let (_r1, ep) = router(false).await;
    let tool = client(&ep).await;
    let a = sysinfo(&ep, &a_root, "host-a", true, None).await;
    let b = sysinfo(&ep, &b_root, "host-b", true, None).await;
    assert_eq!(a.1.address().to_string(), S1);
    assert_eq!(b.1.address().to_string(), S1);
    tokens(&tool, "zk2/h-bbd1aa1db10b/sysinfo/@zk/**", 2).await;
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    let f = found(&r, CheckId::HostidDuplicate, S1);
    assert_eq!(f.severity, DoctorSeverity::Warning);
    assert!(f.evidence.contains("cause is undecided"), "{f:?}");
    assert!(
        f.evidence.contains(&a.1.instance().to_string())
            && f.evidence.contains(&b.1.instance().to_string()),
        "{f:?}"
    );
    assert!(
        f.evidence.contains("\"host-a\"") && f.evidence.contains("\"host-b\""),
        "meta.host shown as evidence: {f:?}"
    );
    drop((a, b));

    // Step 2, the control, and step 4: B holds M2, and a third service has
    // the literal address h-504c6767c349/logger.
    let (_r2, ep) = router(false).await;
    let tool = client(&ep).await;
    let a = sysinfo(&ep, &a_root, "host-a", true, None).await;
    let b = sysinfo(&ep, &root(M2), "host-b", true, None).await;
    assert_eq!(b.1.address().to_string(), "h-3f6d94515669/sysinfo");
    let logger_s = client(&ep).await;
    let logger = bring_up(&logger_s, config("h-504c6767c349/logger"), &[]).await;
    tokens(&tool, "zk2/*/*/@zk/**", 3).await;
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    let why = clean(&r, CheckId::HostidDuplicate);
    assert!(why.starts_with("2 address(es) on minted systems"), "{why}");
    assert!(
        why.contains("not this profile's"),
        "the literal logger: {why}"
    );
    drop((a, b, logger, logger_s));

    // Step 3: B states no meta.zid.
    let (_r3, ep) = router(false).await;
    let tool = client(&ep).await;
    let a = sysinfo(&ep, &a_root, "host-a", true, None).await;
    let b = sysinfo(&ep, &b_root, "host-b", false, None).await;
    tokens(&tool, "zk2/h-bbd1aa1db10b/sysinfo/@zk/**", 2).await;
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    let why = unseen(&r, CheckId::HostidDuplicate);
    assert!(why.contains("states no meta.zid"), "{why}");
    drop((a, b));

    // Step 5: each also owns an interface whose contract lists hostid.v1 in
    // `uses`, in its tokenless set.
    let x = load(
        "[interface]\nname = \"sysinfo_x\"\nmajor = 1\nminor = 0\nuses = [\"hostid.v1\"]\n\
         [resources.cpu]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n",
    );
    let (_r5, ep) = router(false).await;
    let tool = client(&ep).await;
    let a = sysinfo(&ep, &a_root, "host-a", true, Some(&x)).await;
    let b = sysinfo(&ep, &b_root, "host-b", true, Some(&x)).await;
    tokens(&tool, "zk2/h-bbd1aa1db10b/sysinfo/@zk/**", 2).await;
    let r = doctor(&bus(&tool, &tool, ""), &only).await;
    let why = unseen(&r, CheckId::HostidDuplicate);
    assert!(why.contains("lists hostid.v1 in uses"), "{why}");
    drop((a, b));
    for r in [a_root, b_root] {
        let _ = std::fs::remove_dir_all(r);
    }
}
