//! The access-control plans of the example deployments, pinned (spec §11;
//! #612, FJ7): `examples/zk2/acl/`'s enrollments over the walkthrough's and
//! the tcgui pilot's contracts, under both postures (§11.2), and the
//! walkthrough's vehicle router with its constrained face to the ground in
//! both attachments (§8.5, U23).
//!
//! Each plan is pinned as the JSON5 fragment `zenctl acl gen --json5` writes,
//! in `tests/fixtures/acl/`, and each fragment is read back by zenoh's own
//! config loader, so a pinned block is one `zenohd` parses. After an
//! intended change, `ACL_BLESS=1 cargo test -p zenkey-fleet --test
//! acl_plans` rewrites them; read the diff, because a bless claims every
//! changed rule is right.

use std::path::{Path, PathBuf};

use zenkey_fleet::report::{AclFace, AclPermission, AclPlan, Enrollment, FaceAttach};
use zenkey_fleet::{AclOptions, ContractSet, acl_plan_json5, plan_acl};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn contracts(dir: &str) -> ContractSet {
    let (set, problems) = ContractSet::load_path(&root().join("examples/zk2").join(dir));
    // A directory of contracts may hold a deployment file beside them (the
    // tcgui frontend's bindings), reported and skipped.
    for p in &problems {
        assert!(p.at.ends_with(".bindings.toml"), "{p:?}");
    }
    assert!(!set.is_empty());
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
    face: Option<AclFace>,
) -> AclPlan {
    plan_acl(
        &enrollment(name),
        &contracts(dir),
        &AclOptions {
            namespace: String::new(),
            default_permission,
            face,
        },
    )
    .expect("the example plans")
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

/// The ground as one client session of the vehicle router (§8.5): its
/// policy carries the face's denies, and no presence or contract grant.
#[test]
fn walkthrough_with_the_ground_as_a_client() {
    pinned(
        "walkthrough.face-client.json5",
        &plan(
            "walkthrough",
            "walkthrough",
            AclPermission::Deny,
            Some(AclFace {
                attach: FaceAttach::Client,
                far: "ground".into(),
                region: None,
            }),
        ),
    );
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
            Some(AclFace {
                attach: FaceAttach::SouthRegion,
                far: "ground".into(),
                region: Some("ground".into()),
            }),
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
            Some(AclFace {
                attach: FaceAttach::SouthRegion,
                far: "ground".into(),
                region: Some("ground".into()),
            }),
        ),
    );
}
