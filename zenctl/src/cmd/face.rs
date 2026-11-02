//! `zenctl check face <ROUTER-CONFIG> --face-policy <FILE>` (#769, LB): a
//! router's configuration against the constrained faces it guards, exit-coded
//! (`link.v1` §2.15); for a `link.v2` policy, its transport too, the far
//! end's (`--peer-config`) and the catalog the far side holds
//! (`--catalog`, link.v2 §2.19; #988, L2).
//!
//! A judgement, so it hangs under `check` (#307), beside `check schema`, the
//! other one that opens no session. What a router runs is not on the bus
//! (core §11.3, `link.v1` §2.14): the block judged is the config **file**,
//! read through `zenoh::Config::from_file`, so what is judged is what
//! `zenohd` would parse. The face policy, the enrollment and the contracts
//! compile the plan `acl gen --face-policy` writes, and the block is checked
//! against both §2.15's rules and that plan.
//!
//! Every judgement is `zenkey_fleet::check_face`'s; what is here is what only
//! a CLI has: the files, the `--window` spellings, the rendering and the
//! exit. Every failure before the verdict is the reserved 2 ([`ASKING`]): a
//! 1 there would claim the router guards its faces wrongly when nobody read
//! its block.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::bus::Deployment;
use crate::cmd::{acl, zk2};
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check face");

/// `--window ADDRESS=SECS`, each a listed owner's configured tombstone
/// window (`link.v1` §2.11). None at all: windows are not asked.
fn windows(given: &[String]) -> Result<Option<BTreeMap<String, u64>>> {
    if given.is_empty() {
        return Ok(None);
    }
    let mut out = BTreeMap::new();
    for w in given {
        let (addr, secs) = w
            .split_once('=')
            .ok_or_else(|| unaskable!("--window {w:?}: write ADDRESS=SECS, as lab/radio-a=3600"))?;
        let parsed: zenkey_model::grammar::Addr = addr
            .parse()
            .map_err(|e| unaskable!("--window {w:?}: {addr:?} is not <system>/<service>: {e}"))?;
        let secs: u64 = secs
            .parse()
            .map_err(|_| unaskable!("--window {w:?}: {secs:?} is not a whole number of seconds"))?;
        if out.insert(parsed.to_string(), secs).is_some() {
            return Err(unaskable!("--window names {addr} twice"));
        }
    }
    Ok(Some(out))
}

/// Every `--window` names a listed owner of state behind a face: one the
/// plan has no floor for is a question nobody asked (`link.v1` §2.11, "not
/// this profile's"), and passing over it would let a misspelt address
/// leave its owner on the core's 60 s and the check clean.
fn known_windows(
    given: &BTreeMap<String, u64>,
    plan: &zenkey_fleet::report::AclPlan,
) -> Result<()> {
    let floors: std::collections::BTreeSet<&str> = plan
        .faces
        .iter()
        .flat_map(|f| &f.windows)
        .map(|w| w.service.as_str())
        .collect();
    let unknown: Vec<&str> = given
        .keys()
        .map(String::as_str)
        .filter(|a| !floors.contains(a))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(unaskable!(
        "--window names {}, which owns no state behind a face of this policy (link.v1 §2.11); the \
         listed owners of state are: {}",
        unknown.join(", "),
        if floors.is_empty() {
            "none".to_owned()
        } else {
            floors.into_iter().collect::<Vec<_>>().join(", ")
        }
    ))
}

/// The id of the deployment catalog in `path` (`link.v2` §2.19): the
/// catalog read as core §8.6 reads one, and its id §8.6's. A file §8.6
/// refuses holds no id to compare, so the question is not asked: refused,
/// naming the refusal.
fn catalog_id(path: &std::path::Path) -> Result<String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| unaskable!("--catalog {}: {e}", path.display()))?;
    zenkey_model::link::catalog_id(&text).map_err(|r| {
        unaskable!(
            "--catalog {}: core §8.6 refuses it as a catalog: {r} (link.v2 §2.19)",
            path.display()
        )
    })
}

/// `check face`: exit 0 every face guarded as planned, 1 a finding or a
/// difference from the plan, 2 no verdict.
pub async fn run(cli: crate::cli::CheckFaceArgs) -> Result<()> {
    let crate::cli::CheckFaceArgs {
        router,
        face_policy,
        enrollment,
        contracts,
        default_permission,
        window,
        peer_config,
        ns,
    } = cli;
    let dep = ASKING.ask(Deployment::resolve(&ns));
    let windows = ASKING.ask(windows(&window));
    let enrollment = ASKING.ask(acl::load_enrollment(&enrollment));
    // The far side's catalog, `--catalog` beside `--contracts`: one at most.
    let catalog = ASKING.ask(match contracts.catalog.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        several => Err(unaskable!(
            "{} catalogs given: the far side holds one --catalog (link.v2 §2.19)",
            several.len()
        )),
    });
    let contracts = ASKING.ask(zk2::load_contracts(&contracts));
    let parsed = ASKING.ask(acl::load_face_policy(&face_policy));
    let opts = zenkey_fleet::AclOptions {
        namespace: enrollment
            .namespace
            .clone()
            .unwrap_or_else(|| dep.namespace().to_owned()),
        default_permission: acl::permission(default_permission),
        face_policy: Some(parsed.clone()),
        catalog: None,
    };
    let plan = ASKING
        .ask(zenkey_fleet::plan_acl(&enrollment, &contracts, &opts).map_err(anyhow::Error::from));
    let policy = ASKING.ask(
        parsed
            .policy
            .as_ref()
            .ok_or_else(|| unaskable!("--face-policy: the planner kept no policy to judge")),
    );
    if let Some(given) = &windows {
        ASKING.ask(known_windows(given, &plan));
    }
    // link.v2 §2.19's two asks: a link.v1 policy states no transport and no
    // catalog, so neither question has a premise there.
    let v2 = policy.profile == zenkey_model::link::Profile::V2;
    for (flag, given) in [
        ("--peer-config", peer_config.is_some()),
        ("--catalog", catalog.is_some()),
    ] {
        if given && !v2 {
            ASKING.ask(Err::<(), anyhow::Error>(unaskable!(
                "{flag} needs a link.v2 face policy (`profile = \"link.v2\"`, link.v2 §2.19): \
                 a link.v1 policy states no transport and no catalog"
            )));
        }
    }
    let peer = peer_config.as_ref().map(|p| ASKING.ask(acl::load_peer(p)));
    let held = catalog.as_ref().map(|p| ASKING.ask(catalog_id(p)));
    let observed = ASKING.ask(acl::load_against(&router));
    let report = zenkey_fleet::check_face(
        &plan,
        policy,
        &observed,
        &zenkey_fleet::FaceAsk {
            windows: windows.as_ref(),
            peer: peer.as_ref(),
            catalog: held.as_deref(),
        },
        &router.display().to_string(),
    );
    acl::plan_notes(&plan);
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    crate::exit::verdict(&report.judgement)
}
