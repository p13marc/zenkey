//! A constrained face, planned and run (spec §8.5, U23; #612, FJ7).
//!
//! The walkthrough's vehicle router holds the plan of
//! `examples/zk2/acl/walkthrough.enrollment.toml` with the ground segment as
//! its far side, in both attachments: a far router in a south region
//! (`--face constrained --attach south-region --far ground --region
//! ground`), which names its region, authenticates to the vehicle router as
//! the `ground` principal and serves a ground client of its own; and one
//! ground session as a client (`--attach client`). What is asked of the
//! running routers is what the unit tests can only read off the block:
//! zenoh accepts the near router's `gateway.south`, the data the ground is
//! granted crosses, and presence does not (the `@zk` deny on the face).
//!
//! Measured here, and the reason for the plan's `face-declarations` rule: a
//! far router learns of the vehicle's queryables by declaration, and routes
//! a query there only for one it has learnt, so the queryables over what the
//! far side may query are declared toward it. Without the rule, its GET gets
//! no reply. A client needs none: it sends every query to its router.

mod util;

use std::time::Duration;

use util::zk2::{T, eventually};
use zenkey_fleet::report::{AclFace, AclPermission, AclPlan, Enrollment, FaceAttach};
use zenkey_fleet::{AclOptions, ContractSet, acl_plan_json5, plan_acl};
use zenoh::Wait;
use zenoh::query::{ConsolidationMode, QueryTarget};

const STATUS: &str = "zk2/vehicle-01/navigation/nav.v2/state/status";
const INSTANCE: &str = "zk2/vehicle-01/navigation/@zk/instance/00000000000000a1";

fn scratch(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NTH: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "zenkey-acl-face-{}-{}-{name}",
        std::process::id(),
        NTH.fetch_add(1, Ordering::Relaxed)
    ))
}

fn base(mode: &str) -> zenoh::Config {
    let mut c = zenoh::Config::default();
    c.insert_json5("mode", &format!("{mode:?}")).unwrap();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c
}

fn usrpwd(c: &mut zenoh::Config, user: &str, dictionary: Option<&std::path::Path>) {
    let dict = dictionary.map_or(String::new(), |d| {
        format!(", dictionary_file: {:?}", d.display().to_string())
    });
    c.insert_json5(
        "transport/auth/usrpwd",
        &format!("{{ user: \"{user}\", password: \"{user}-pw\"{dict} }}"),
    )
    .unwrap();
}

/// The value replies to a GET.
async fn values(s: &zenoh::Session, selector: &str) -> usize {
    let rx = s
        .get(selector)
        .target(QueryTarget::All)
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

fn face_plan(attach: FaceAttach) -> AclPlan {
    let examples = util::zk2::examples();
    let enrollment: Enrollment = toml::from_str(
        &std::fs::read_to_string(examples.join("acl/walkthrough.enrollment.toml")).unwrap(),
    )
    .unwrap();
    let (contracts, _) = ContractSet::load_path(&examples.join("walkthrough"));
    let plan = plan_acl(
        &enrollment,
        &contracts,
        &AclOptions {
            default_permission: AclPermission::Deny,
            face: Some(AclFace {
                attach,
                far: "ground".into(),
                region: (attach == FaceAttach::SouthRegion).then(|| "ground".to_owned()),
            }),
            ..AclOptions::default()
        },
    )
    .unwrap();
    assert!(plan.refusals.is_empty(), "{:#?}", plan.refusals);
    plan
}

/// What crosses to the ground.
#[derive(Debug, PartialEq, Eq)]
struct Across {
    /// Navigation's status, a state the ground's ops tool may GET.
    status: usize,
    /// Navigation's tokens, read by liveliness.
    tokens: usize,
    /// Navigation's descriptor, a GET under `@zk`.
    descriptor: usize,
}

/// The vehicle router holding `plan`, navigation a client of it, and the
/// ground on its far side as `attach` says; what the ground then sees.
async fn across(plan: &AclPlan, attach: FaceAttach) -> Across {
    let dict = scratch("near.txt");
    std::fs::write(
        &dict,
        "router:router-pw\nnavigation:navigation-pw\nground:ground-pw\n",
    )
    .unwrap();
    let text = format!(
        "{{\n  mode: \"router\",\n  scouting: {{ multicast: {{ enabled: false }}, gossip: {{ enabled: false }} }},\n  \
         listen: {{ endpoints: [\"{}\"] }},\n  \
         transport: {{ auth: {{ usrpwd: {{ user: \"router\", password: \"router-pw\", dictionary_file: {:?} }} }} }},\n{}}}\n",
        util::ANY_PORT,
        dict.display().to_string(),
        acl_plan_json5(plan)
    );
    let near = zenoh::open(
        zenoh::Config::from_json5(&text)
            .unwrap_or_else(|e| panic!("the near router's config does not parse: {e}\n{text}")),
    )
    .await
    .expect("the near router accepts the plan, gateway.south included");
    let near_ep = util::bound(&near).await;

    let mut c = base("client");
    c.insert_json5("connect/endpoints", &format!("[\"{near_ep}\"]"))
        .unwrap();
    usrpwd(&mut c, "navigation", None);
    let nav = zenoh::open(c).await.expect("navigation connects");
    let _token = nav.liveliness().declare_token(INSTANCE).await.unwrap();
    let _status = nav
        .declare_queryable(STATUS)
        .callback(|q| {
            let _ = q.reply(STATUS, "ok").wait();
        })
        .await
        .unwrap();

    let far_dict = scratch("far.txt");
    let (_far, ground) = match attach {
        FaceAttach::SouthRegion => {
            // The ground router names its region and is the `ground`
            // principal to the vehicle router. A router presenting usrpwd
            // credentials checks them on its own links too, so it holds a
            // dictionary: the vehicle router's, and its client's.
            std::fs::write(&far_dict, "router:router-pw\nops:ops-pw\n").unwrap();
            let mut c = base("router");
            c.insert_json5("region_name", "\"ground\"").unwrap();
            c.insert_json5("listen/endpoints", &format!("[\"{}\"]", util::ANY_PORT))
                .unwrap();
            c.insert_json5("connect/endpoints", &format!("[\"{near_ep}\"]"))
                .unwrap();
            usrpwd(&mut c, "ground", Some(&far_dict));
            let far = zenoh::open(c).await.expect("the ground router");
            let far_ep = util::bound(&far).await;
            let mut c = base("client");
            c.insert_json5("connect/endpoints", &format!("[\"{far_ep}\"]"))
                .unwrap();
            usrpwd(&mut c, "ops", None);
            (Some(far), zenoh::open(c).await.expect("a ground client"))
        }
        _ => {
            let mut c = base("client");
            c.insert_json5("connect/endpoints", &format!("[\"{near_ep}\"]"))
                .unwrap();
            usrpwd(&mut c, "ground", None);
            (None, zenoh::open(c).await.expect("the ground session"))
        }
    };

    // The control, on the vehicle side: the near router holds the token.
    eventually("the vehicle router holds navigation's token", || async {
        zk2::presence::liveliness_read(&near, "zk2/vehicle-01/navigation/@zk/**", T)
            .await
            .unwrap()
            .keys
            .len()
            == 1
    })
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut status = 0;
    while status == 0 && tokio::time::Instant::now() < deadline {
        status = values(&ground, STATUS).await;
    }
    let read = zk2::presence::liveliness_read(&ground, "zk2/vehicle-01/navigation/@zk/**", T)
        .await
        .unwrap();
    assert!(read.complete, "a refused read is complete (§8.1): {read:?}");
    let descriptor = values(&ground, INSTANCE).await;
    let _ = std::fs::remove_file(&dict);
    let _ = std::fs::remove_file(&far_dict);
    Across {
        status,
        tokens: read.keys.len(),
        descriptor,
    }
}

const CROSSES: Across = Across {
    status: 1,
    tokens: 0,
    descriptor: 0,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_far_router_south_gets_its_data_and_no_presence() {
    let plan = face_plan(FaceAttach::SouthRegion);
    assert!(plan.gateway.is_some());
    assert_eq!(across(&plan, FaceAttach::SouthRegion).await, CROSSES);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_far_session_as_a_client_gets_its_data_and_no_presence() {
    let plan = face_plan(FaceAttach::Client);
    assert!(plan.gateway.is_none());
    assert!(
        !plan
            .rules
            .iter()
            .any(|r| r.id.starts_with("face-declarations:"))
    );
    assert_eq!(across(&plan, FaceAttach::Client).await, CROSSES);
}

/// The control for `face-declarations`: without it, the far router never
/// learns of navigation's queryable, and its GET gets no reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_the_declarations_a_far_router_routes_no_query() {
    let mut plan = face_plan(FaceAttach::SouthRegion);
    let id = "face-declarations:ground";
    assert!(plan.rules.iter().any(|r| r.id == id));
    for p in &mut plan.policies {
        p.rules.retain(|r| r != id);
    }
    plan.rules.retain(|r| r.id != id);
    assert_eq!(
        across(&plan, FaceAttach::SouthRegion).await,
        Across {
            status: 0,
            tokens: 0,
            descriptor: 0
        }
    );
}
