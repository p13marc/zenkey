//! The access-control plans of the example deployments, pinned (spec §11;
//! #612, FJ7): `examples/zk2/acl/`'s enrollments over the walkthrough's and
//! the tcgui pilot's contracts, under both postures (§11.2); the
//! walkthrough's vehicle router with its constrained face to the ground in
//! both attachments (§8.5, U23), from its face policy
//! (`examples/zk2/acl/walkthrough.face.toml`, `link.v1`); and `link.v1`'s
//! own example, `examples/zk2/link/`, under both postures (#769, LB).
//!
//! Each plan is pinned as the JSON5 fragment `zenctl acl gen --json5` writes,
//! in `tests/fixtures/acl/`, and each fragment is read back by zenoh's own
//! config loader, so a pinned block is one `zenohd` parses. After an
//! intended change, `ACL_BLESS=1 cargo test -p zenkey-fleet --test
//! acl_plans` rewrites them; read the diff, because a bless claims every
//! changed rule is right.

use std::path::{Path, PathBuf};

use zenkey_fleet::report::{AclPermission, AclPlan, Enrollment};
use zenkey_fleet::{AclOptions, ContractSet, acl_plan_json5, plan_acl};
use zenkey_model::link;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// An example directory's contracts, with the contracts an example
/// deployment implements beside its own: the profiles' standard
/// `health.v1` (#721), and zenwatch's own `zenwatch.v1`, which lives with
/// zenwatch (#614, FL3).
fn contracts(dir: &str) -> ContractSet {
    let (mut set, problems) = ContractSet::load_path(&root().join("examples/zk2").join(dir));
    // A directory of contracts may hold a deployment file beside them (the
    // tcgui frontend's bindings), reported and skipped.
    for p in &problems {
        assert!(p.at.ends_with(".bindings.toml"), "{p:?}");
    }
    assert!(!set.is_empty());
    for beside in [
        "spec/profiles/health",
        "zenwatch/contracts/zenwatch.v1.toml",
    ] {
        let (more, problems) = ContractSet::load_path(&root().join(beside));
        assert!(problems.is_empty(), "{beside}: {problems:?}");
        assert!(!more.is_empty(), "{beside}");
        set.extend(more);
    }
    set
}

fn enrollment(name: &str) -> Enrollment {
    let path = root().join(format!("examples/zk2/acl/{name}.enrollment.toml"));
    let text = std::fs::read_to_string(&path).unwrap();
    toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn plan(
    name: &str,
    dir: &str,
    default_permission: AclPermission,
    face_policy: Option<&str>,
) -> AclPlan {
    plan_acl(
        &enrollment(name),
        &contracts(dir),
        &AclOptions {
            namespace: String::new(),
            default_permission,
            face_policy: face_policy.map(link::parse),
        },
    )
    .expect("the example plans")
}

/// The walkthrough's face policy, its far router in a south region; or the
/// same face with the ground as one client session.
fn walkthrough_face(south: bool) -> String {
    let text =
        std::fs::read_to_string(root().join("examples/zk2/acl/walkthrough.face.toml")).unwrap();
    if south {
        text
    } else {
        text.replace(
            "attach       = \"south-region\"\nregion       = \"ground\"\n",
            "attach       = \"client\"\n",
        )
    }
}

/// The fragment pinned, and read back by zenoh's own loader.
fn pinned(file: &str, plan: &AclPlan) {
    assert!(plan.refusals.is_empty(), "{file}: {:#?}", plan.refusals);
    let block = acl_plan_json5(plan);
    let config = zenoh::Config::from_json5(&format!("{{\n{block}}}"))
        .unwrap_or_else(|e| panic!("{file}: zenoh refuses the block: {e}\n{block}"));
    let acl = config.get_json("access_control").expect("access_control");
    assert!(acl.contains("\"enabled\":true"), "{file}: {acl}");
    if plan.gateway.is_some() {
        let gw = config.get_json("gateway").expect("gateway");
        assert!(gw.contains("region_names"), "{file}: {gw}");
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/acl")
        .join(file);
    if std::env::var_os("ACL_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &block).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; ACL_BLESS=1 cargo test -p zenkey-fleet --test acl_plans",
            path.display()
        )
    });
    assert!(
        want == block,
        "{file} is stale: ACL_BLESS=1 cargo test -p zenkey-fleet --test acl_plans, and read \
         the diff\n--- generated ---\n{block}"
    );
}

#[test]
fn tcgui_under_deny() {
    pinned(
        "tcgui.deny.json5",
        &plan("tcgui", "tcgui", AclPermission::Deny, None),
    );
}

#[test]
fn tcgui_under_allow() {
    pinned(
        "tcgui.allow.json5",
        &plan("tcgui", "tcgui", AclPermission::Allow, None),
    );
}

#[test]
fn walkthrough_under_deny() {
    pinned(
        "walkthrough.deny.json5",
        &plan("walkthrough", "walkthrough", AclPermission::Deny, None),
    );
}

#[test]
fn walkthrough_under_allow() {
    pinned(
        "walkthrough.allow.json5",
        &plan("walkthrough", "walkthrough", AclPermission::Allow, None),
    );
}

/// The ground as one client session of the vehicle router (§8.5), from the
/// walkthrough's face policy: its policy carries the face's denies, and no
/// presence, history or contract grant. None of the walkthrough's contracts
/// uses `link.v1`, so every push across the face is denied, wholesale per
/// kind token, and the ground's bindings cross as GETs and calls, by grant
/// (link.v1 §2.7, §2.9, Appendix B).
#[test]
fn walkthrough_with_the_ground_as_a_client() {
    let plan = plan(
        "walkthrough",
        "walkthrough",
        AclPermission::Deny,
        Some(&walkthrough_face(false)),
    );
    assert!(plan.downsampling.is_empty(), "nothing is exposed");
    pinned("walkthrough.face-client.json5", &plan);
}

/// The ground's router in a south region of the vehicle router (U23), under
/// deny: the face's denies, the queryables the far router may query
/// declared toward it, and the near router's `gateway.south`.
#[test]
fn walkthrough_with_the_ground_router_south_under_deny() {
    pinned(
        "walkthrough.face-south-deny.json5",
        &plan(
            "walkthrough",
            "walkthrough",
            AclPermission::Deny,
            Some(&walkthrough_face(true)),
        ),
    );
}

/// The same, under the allow posture zenoh-modem's faces run: the face's
/// denies, the complement's, and the near router's `gateway.south`.
#[test]
fn walkthrough_with_the_ground_router_south() {
    pinned(
        "walkthrough.face-south.json5",
        &plan(
            "walkthrough",
            "walkthrough",
            AclPermission::Allow,
            Some(&walkthrough_face(true)),
        ),
    );
}

/// `link.v1`'s example (`examples/zk2/link/`): the lab router with its radio
/// face to the ground, as a client, under either posture.
fn radio_plan(default_permission: AclPermission) -> AclPlan {
    let dir = root().join("examples/zk2/link");
    let (set, problems) = ContractSet::load_path(&dir.join("radio.v1.toml"));
    assert!(problems.is_empty(), "{problems:?}");
    let enrollment: Enrollment =
        toml::from_str(&std::fs::read_to_string(dir.join("radio.enrollment.toml")).unwrap())
            .unwrap();
    let policy = std::fs::read_to_string(dir.join("radio.face.toml")).unwrap();
    plan_acl(
        &enrollment,
        &set,
        &AclOptions {
            namespace: String::new(),
            default_permission,
            face_policy: Some(link::parse(&policy)),
        },
    )
    .expect("the radio example plans")
}

/// Under deny, the scenarios' posture: the far principal's policy carries
/// `zk`, `adv`, `admin` and the exposure denies, carved around what
/// crosses, and the downsampling block has one rule per crossing stream of
/// `lab/radio-a` at 1 Hz (scenarios §1, expected 1).
#[test]
fn radio_under_deny() {
    let plan = radio_plan(AclPermission::Deny);
    let ids: Vec<&str> = plan.faces[0].deny.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "zk",
            "adv",
            "admin",
            "stream:radio.v1",
            "@stream:radio.v1",
            "state:radio.v1",
            "@state",
            "events:radio.v1",
        ]
    );
    assert_eq!(plan.downsampling[0].rules.len(), 2);
    pinned("radio.face.deny.json5", &plan);
}

/// Under allow: the same face, beside the complement's denies.
#[test]
fn radio_under_allow() {
    pinned("radio.face.allow.json5", &radio_plan(AclPermission::Allow));
}

/// What `check face` finds in `plan`'s block, read back by zenoh's loader,
/// against the face policy `policy` it was compiled from: §2.15's findings,
/// and the comparison's.
fn self_check(plan: &AclPlan, policy: &str) -> (Vec<link::Finding>, usize) {
    let block = acl_plan_json5(plan);
    let c = zenoh::Config::from_json5(&format!("{{\n{block}}}")).unwrap();
    let acl = serde_json::from_str(&c.get_json("access_control").unwrap()).unwrap();
    let ds: Vec<zenkey_fleet::report::DownsamplingDoc> = c
        .get_json("downsampling")
        .ok()
        .and_then(|d| serde_json::from_str(&d).ok())
        .unwrap_or_default();
    let gw: Option<serde_json::Value> = c
        .get_json("gateway")
        .ok()
        .and_then(|g| serde_json::from_str(&g).ok());
    let parsed = link::parse(policy);
    let report = zenkey_fleet::check_face(
        plan,
        parsed.policy.as_ref().unwrap(),
        &acl,
        &ds,
        gw.as_ref(),
        None,
        "self",
    );
    (report.findings, report.comparison.findings.len())
}

/// Every face plan the generator writes passes every check of `link.v1`
/// §2.15 in both postures, the subject checks of 0.4 among them (F-120):
/// the walkthrough's face in both attachments, and the radio's.
#[test]
fn every_face_block_passes_its_own_checks() {
    for posture in [AclPermission::Deny, AclPermission::Allow] {
        for south in [false, true] {
            let policy = walkthrough_face(south);
            let plan = plan("walkthrough", "walkthrough", posture, Some(&policy));
            assert_eq!(
                self_check(&plan, &policy),
                (Vec::new(), 0),
                "walkthrough, south {south}, {posture:?}"
            );
        }
        let policy =
            std::fs::read_to_string(root().join("examples/zk2/link/radio.face.toml")).unwrap();
        assert_eq!(
            self_check(&radio_plan(posture), &policy),
            (Vec::new(), 0),
            "radio, {posture:?}"
        );
    }
}

/// `config.v1`'s acts, each its own grant (config.v1 §2.21; #768, CC): the
/// standard contract alone, as `--contracts spec/profiles/config` gives it.
fn config_plan(default_permission: AclPermission) -> AclPlan {
    let (set, problems) = ContractSet::load_path(&root().join("spec/profiles/config"));
    assert!(problems.is_empty(), "{problems:?}");
    plan_acl(
        &enrollment("config"),
        &set,
        &AclOptions {
            namespace: String::new(),
            default_permission,
            face_policy: None,
        },
    )
    .expect("the config enrollment plans")
}

/// Under deny, the far side's operators hold rf0's `set/duty`,
/// `set/radio`, `confirm`, `cancel` and `extend` and the GET of its view
/// and status, one key each, and neither `persist` nor anything of sat0;
/// the local operator holds every act of both.
#[test]
fn config_acts_under_deny() {
    let plan = config_plan(AclPermission::Deny);
    let keys = |principal: &str| -> Vec<String> {
        plan.rules
            .iter()
            .filter(|r| r.id.contains(principal) && r.id.contains("call"))
            .flat_map(|r| r.key_exprs.clone())
            .collect()
    };
    let ops = keys("ops");
    for want in [
        "zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/set/duty",
        "zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/set/radio",
        "zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/confirm",
        "zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/cancel",
        "zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/extend",
    ] {
        assert!(ops.iter().any(|k| k == want), "{want}: {ops:#?}");
    }
    assert!(
        !ops.iter()
            .any(|k| k.contains("persist") || k.contains("sat0") || k.ends_with("set/*")),
        "{ops:#?}"
    );
    let admin = keys("admin");
    assert!(
        admin
            .iter()
            .any(|k| k == "zk2/h-3fa9c2d41b7e/sat0/config.v1/@op/persist"),
        "{admin:#?}"
    );
    pinned("config.deny.json5", &plan);
}

/// Under allow, the grants compile to denies of their complement: the
/// operators' persist and everything of sat0 denied by name, and the
/// members of `set/{group}` no grant names left callable, which the plan
/// says (§11.3: deny works by inclusion).
#[test]
fn config_acts_under_allow() {
    let plan = config_plan(AclPermission::Allow);
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.kind == zenkey_fleet::report::AclWarningKind::ComplementPartial),
        "{:#?}",
        plan.warnings
    );
    pinned("config.allow.json5", &plan);
}

/// #875: a tool's watch session is in no namespace, and spells its
/// selectors and its barrier key on the wire, the namespace in front. Every
/// grant `acl gen` writes is a full key under the namespace (§1.6, §11.1),
/// so each of them is inside the presence grant of the walkthrough's tool,
/// zenwatch: the presence monitor's three token selectors, the alarm and
/// telemetry readers' instance and member tokens, and the round trip's
/// `…/@zk/settle`, for the bus root, whose plan is pinned
/// (`walkthrough.deny.json5`), and under a namespace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watch_sessions_wire_keys_are_inside_a_tools_presence_grant() {
    use zenkey_fleet::report::{AclFlow, AclGrantKind, AclMessage};
    use zenoh::key_expr::KeyExpr;
    let mut c = zenoh::Config::default();
    for (k, v) in [
        ("mode", r#""peer""#),
        ("listen/endpoints", "[]"),
        ("scouting/multicast/enabled", "false"),
        ("scouting/gossip/enabled", "false"),
    ] {
        c.insert_json5(k, v).expect("a fixed key");
    }
    let session = zenoh::open(c).await.expect("a session in no namespace");
    let scope = zenkey_fleet::PresenceScope::all();
    let selectors: Vec<String> = scope
        .token_selectors()
        .into_iter()
        .chain([zenkey_fleet::AlarmScope::all().instance_selector()])
        .collect();
    for namespace in ["", "fleet-a"] {
        let plan = plan_acl(
            &enrollment("walkthrough"),
            &contracts("walkthrough"),
            &AclOptions {
                namespace: namespace.to_owned(),
                default_permission: AclPermission::Deny,
                face_policy: None,
            },
        )
        .expect("the walkthrough's plan");
        let grants: Vec<&zenkey_fleet::report::AclRule> = plan
            .rules
            .iter()
            .filter(|r| {
                r.holder.as_deref() == Some("tool.zenwatch")
                    && r.grant == AclGrantKind::Presence
                    && r.flows.contains(&AclFlow::Ingress)
            })
            .collect();
        assert!(
            !grants.is_empty(),
            "{namespace:?}: zenwatch's presence grant"
        );
        let granted = |key: &str, message: AclMessage| {
            let wanted = KeyExpr::try_from(key).expect("a key expression");
            grants.iter().any(|r| {
                r.messages.contains(&message)
                    && r.key_exprs
                        .iter()
                        .any(|k| KeyExpr::try_from(k.as_str()).is_ok_and(|g| g.includes(&wanted)))
            })
        };
        let watch = zenkey_fleet::WatchSession::new(&session, namespace).expect("a watch session");
        for sel in &selectors {
            let wire = watch.wire(sel);
            assert!(
                granted(&wire, AclMessage::DeclareLivelinessSubscriber),
                "{namespace:?}: subscribing to {wire} is outside {grants:#?}"
            );
            let barrier = watch
                .barrier(sel, std::time::Duration::from_secs(1))
                .await
                .expect("a barrier");
            assert!(
                granted(barrier.key(), AclMessage::LivelinessQuery),
                "{namespace:?}: the round trip on {} is outside {grants:#?}",
                barrier.key()
            );
        }
    }
    session.close().await.expect("close");
}
