//! `zenctl acl gen` (#612, FJ7): zk2's grant shapes (spec §11), compiled
//! from an enrollment and the contracts into a router's `access_control`
//! block, and checked, and explained. With `--face constrained`, the block
//! also guards the face to the far side (§8.5).
//!
//! Thin by design. The grants, the fan-in, the complement, the face, the
//! check and the explanation are all judgement over values in hand, so they
//! live in `zenkey_fleet::model::acl` where the other explorer can reach
//! them. What is left here is what only a CLI has: the files (the
//! enrollment, the contracts, the router config), the rendering, and the
//! exit.
//!
//! **Where bindings come from.** The enrollment, only: it names each
//! service's bindings (R1, R2) and the operations each one calls. A
//! deployment's descriptors carry the same facts (R3), but a generator run
//! before the deployment starts has none to read, and a plan that changed
//! with what happened to be alive would not be a plan. Contracts are read
//! offline, from `--contracts` (authoring files, directories of them, or a
//! `.history` root of bundles); this verb opens no session.
//!
//! ## Three modes, one plan
//!
//! * **gen** renders the plan: the table for a person, the pinned shape for
//!   a script, or, under `--json5`, the router config fragment on stdout
//!   with a comment per rule. The fragment is a foreign schema, zenoh's, so
//!   `--json5` and `--format` conflict the way `--dot` and `--format` do
//!   (#243). A refused principal exits 1: rows were refused, which the exit
//!   contract calls a finding.
//! * **`--check --against <router.json5>`** is a judgement verb. The
//!   router's running block is not observable on the bus (§11.3), so the
//!   observed side is the config *file*, read through
//!   `zenoh::Config::from_file`, zenoh's own loader: what is compared is
//!   what `zenohd` would parse. Every failure before the comparison lands on
//!   the reserved 2 through [`ASKING`]; the verdict exits through
//!   [`crate::exit::verdict`].
//! * **`--explain <principal> <key> <message>`** is pure over the plan and
//!   exits 0; a principal the plan does not carry is an `Unaskable`.

use std::path::Path;

use anyhow::{Context, Result};
use zenkey_fleet::report::{AclFace, AclPermission, FaceAttach};

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;
use crate::render::Note;

/// This verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("acl gen --check");

/// The enrollment file, parsed: a shape the engine owns
/// (`report::Enrollment`) and a file only the frontend reads.
pub(crate) fn load_enrollment(path: &Path) -> Result<zenkey_fleet::report::Enrollment> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("enrollment {}", path.display()))
        .map_err(|e| unaskable!("{e:#}"))?;
    toml::from_str(&text).map_err(|e| {
        unaskable!(
            "enrollment {}: {}",
            path.display(),
            zenkey_fleet::one_line(&e)
        )
    })
}

/// The router config's `access_control` block, as zenoh's loader parsed it,
/// and its `gateway` block, which a south-region face is checked against.
fn load_against(
    path: &Path,
) -> Result<(
    zenkey_fleet::report::AclConfigDoc,
    Option<serde_json::Value>,
)> {
    let config = zenoh::Config::from_file(path)
        .map_err(|e| unaskable!("{}: zenoh refused the config: {e}", path.display()))?;
    let json = config
        .get_json("access_control")
        .map_err(|e| unaskable!("{}: no access_control block: {e}", path.display()))?;
    let acl = serde_json::from_str(&json).map_err(|e| {
        unaskable!(
            "{}: access_control does not parse as zenoh 1.10's AclConfig: {e}",
            path.display()
        )
    })?;
    let gateway = config
        .get_json("gateway")
        .ok()
        .and_then(|g| serde_json::from_str(&g).ok());
    Ok((acl, gateway))
}

/// The plan's warnings and refusals as stderr notes, for the modes that
/// render something *other* than the plan (`--json5`, `--explain`, the
/// check), so the honesty rides every path.
fn plan_notes(plan: &zenkey_fleet::report::AclPlan) {
    for w in &plan.warnings {
        eprintln!("{}", warning_note(w).to_line());
    }
    for r in &plan.refusals {
        eprintln!("{}", refusal_note(r).to_line());
    }
}

/// A warning as a note. What the plan could not narrow (a contract not
/// given, a complement that inclusion cannot carve) is a *coverage* claim
/// and rides the machine formats; the rest are caveats about reading the
/// plan. The cite is data on the warning and `Note::cite` wants a static,
/// so it rides the text.
pub(crate) fn warning_note(w: &zenkey_fleet::report::AclWarning) -> Note {
    use zenkey_fleet::report::AclWarningKind;
    let about = w
        .about
        .as_deref()
        .map_or(String::new(), |p| format!("{p}: "));
    let text = format!("{}{} ({})", about, w.text, w.cite);
    match w.kind {
        AclWarningKind::ContractNotGiven | AclWarningKind::ComplementPartial => {
            Note::coverage(text)
        }
        _ => Note::caveat(text),
    }
}

pub(crate) fn refusal_note(r: &zenkey_fleet::report::AclRefusal) -> Note {
    Note::caveat(format!(
        "REFUSED {}: {} ({}), left out of subjects and policies",
        r.principal, r.reason, r.cite
    ))
}

/// On the check path, a verdict verb: every failure before the comparison
/// is `asked`, the reserved 2; a refused input exiting 1 would read "the
/// router's block differs" about a block nobody read.
fn ask<T>(check: bool, r: Result<T>) -> Result<T> {
    if check { Ok(ASKING.ask(r)) } else { r }
}

pub async fn run(cli: crate::cli::AclGenArgs) -> Result<()> {
    let crate::cli::AclGenArgs {
        enrollment,
        contracts,
        default_permission,
        json5,
        check,
        against,
        explain,
        face,
        attach,
        far,
        region,
        ns,
    } = cli;

    let dep = ask(check, Deployment::resolve(&ns))?;
    let enrollment = ask(check, load_enrollment(&enrollment))?;
    let contracts = ask(check, zk2::load_contracts(&contracts))?;
    let face = face.map(|crate::cli::Face::Constrained| AclFace {
        attach: match attach.expect("clap: --face requires --attach") {
            crate::cli::Attach::Client => FaceAttach::Client,
            crate::cli::Attach::SouthRegion => FaceAttach::SouthRegion,
            crate::cli::Attach::Router => FaceAttach::Router,
        },
        far: far.expect("clap: --face requires --far"),
        region,
    });
    let opts = zenkey_fleet::AclOptions {
        namespace: enrollment
            .namespace
            .clone()
            .unwrap_or_else(|| dep.namespace().to_owned()),
        default_permission: match default_permission {
            crate::cli::DefaultPermission::Deny => AclPermission::Deny,
            crate::cli::DefaultPermission::Allow => AclPermission::Allow,
        },
        face,
    };
    let plan = ask(
        check,
        zenkey_fleet::plan_acl(&enrollment, &contracts, &opts).map_err(anyhow::Error::from),
    )?;

    if check {
        let against = against.expect("clap: --check requires --against");
        let (observed, gateway) = ASKING.ask(load_against(&against));
        let report = zenkey_fleet::check_acl(
            &plan,
            &observed,
            gateway.as_ref(),
            &against.display().to_string(),
        );
        plan_notes(&plan);
        crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
        // The library returns a verdict, the command exits with it, and a
        // refused principal is a finding on top of a clean comparison: the
        // block cannot carry a principal the plan left out.
        crate::exit::verdict(&report.judgement)?;
        if !plan.refusals.is_empty() {
            std::process::exit(crate::exit::FINDING);
        }
        return Ok(());
    }

    if let Some(words) = explain {
        let [principal, key, message]: [String; 3] = words
            .try_into()
            .map_err(|_| unaskable!("--explain takes PRINCIPAL KEY MESSAGE"))?;
        let message = zenkey_fleet::report::AclMessage::parse(&message).ok_or_else(|| {
            unaskable!(
                "--explain: {message:?} is not a zenoh 1.10 ACL message kind; one of {}",
                zenkey_fleet::report::AclMessage::ALL
                    .iter()
                    .map(|m| m.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let report = zenkey_fleet::explain_acl(&plan, &principal, &key, message)?;
        plan_notes(&plan);
        return crate::render::emit_with(
            &mut std::io::stdout(),
            &report,
            dep.format(),
            dep.color(),
        );
    }

    if json5 {
        print!("{}", zenkey_fleet::acl_plan_json5(&plan));
        plan_notes(&plan);
    } else {
        crate::render::emit_with(&mut std::io::stdout(), &plan, dep.format(), dep.color())?;
    }
    if !plan.refusals.is_empty() {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}
