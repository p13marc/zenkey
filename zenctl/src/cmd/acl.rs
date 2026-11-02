//! `zenctl acl gen` (#612, FJ7): zk2's grant shapes (spec §11), compiled
//! from an enrollment and the contracts into a router's `access_control`
//! block, and checked, and explained. With `--face-policy`, the block also
//! guards the router's constrained faces (§8.5, `link.v1`, #769 LB): each
//! face's plan in its far principal's policy, and the `downsampling` block
//! beside it. The old `--face/--attach/--far/--region` are refused by name.
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
use zenkey_fleet::RouterConfig;
use zenkey_fleet::report::AclPermission;

use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;
use crate::render::Note;

/// This verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("acl gen --check");

/// The face policy file, read through `link.v1`'s own reader (§2.1). The
/// linter's verdict is the planner's to refuse, with every finding named.
pub(crate) fn load_face_policy(path: &Path) -> Result<zenkey_model::link::Parsed> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("face policy {}", path.display()))
        .map_err(|e| unaskable!("{e:#}"))?;
    Ok(zenkey_model::link::parse(&text))
}

/// The posture flag, as the planner takes it.
pub(crate) fn permission(p: crate::cli::DefaultPermission) -> AclPermission {
    match p {
        crate::cli::DefaultPermission::Deny => AclPermission::Deny,
        crate::cli::DefaultPermission::Allow => AclPermission::Allow,
    }
}

/// The face flags `--face-policy` replaced (#769), refused by name: a face
/// policy states everything they did, and exposure and downsampling too.
fn refuse_old_face_flags(given: &[(&str, &Option<String>)]) -> Result<()> {
    let named: Vec<&str> = given
        .iter()
        .filter(|(_, v)| v.is_some())
        .map(|(n, _)| *n)
        .collect();
    if named.is_empty() {
        return Ok(());
    }
    Err(unaskable!(
        "{} {} replaced by --face-policy <FILE> (link.v1 §2.1, #769): a face policy names each \
         constrained face's far principal (`far`), its attachment (`attach`, `region`) and its \
         link protocols (`select`), and adds what crosses (`expose`), its rate (`interval_s`), \
         its outage, batch and link rate. examples/zk2/link/radio.face.toml is one; see `zenctl \
         acl gen --help`",
        named.join(", "),
        if named.len() == 1 { "was" } else { "were" }
    ))
}

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

/// A config file's own text as a JSON tree, once zenoh's loader accepted
/// it: the keys it *states*, which `link.v2` §2.19 reads apart from the
/// defaults zenoh fills in. The dialect is the one zenoh 1.10.1's
/// `Config::from_file` picks by the extension, read with the parser it
/// used (`zenkey_fleet::ConfigDialect`), so a file zenoh accepts reads here.
fn stated_tree(path: &Path) -> Result<serde_json::Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{}", path.display()))
        .map_err(|e| unaskable!("{e:#}"))?;
    let dialect = zenkey_fleet::ConfigDialect::of_path(path).ok_or_else(|| {
        unaskable!(
            "{}: zenoh reads .json, .json5, .yaml, .yml and .toml config files, and this is none",
            path.display()
        )
    })?;
    RouterConfig::stated_tree(&text, dialect).map_err(|e| {
        unaskable!(
            "{}: zenoh read this file, but which keys it states could not be read: {e}",
            path.display()
        )
    })
}

/// A config file zenoh's own loader accepts, and its own tree.
fn load_config(path: &Path) -> Result<(zenoh::Config, serde_json::Value)> {
    let config = zenoh::Config::from_file(path)
        .map_err(|e| unaskable!("{}: zenoh refused the config: {e}", path.display()))?;
    Ok((config, stated_tree(path)?))
}

/// The router config's `access_control`, `downsampling` and `gateway`
/// blocks, as zenoh's loader parsed them, and the transport keys it states
/// (`link.v2` §2.19).
pub(crate) fn load_against(path: &Path) -> Result<RouterConfig> {
    let (config, stated) = load_config(path)?;
    RouterConfig::read(&config, &stated).map_err(|e| unaskable!("{}: {e}", path.display()))
}

/// The transport keys a far end's config file states (`link.v2` §2.19):
/// the far router's, or the gateway client's session config.
pub(crate) fn load_peer(path: &Path) -> Result<zenkey_model::link::StatedTransport> {
    let (_, stated) = load_config(path)?;
    Ok(zenkey_model::link::StatedTransport::read(&stated))
}

/// The far side's catalog, `--catalog` beside a `link.v2` --face-policy
/// (link.v2 §2.18): one at most, read as core §8.6 reads one, a file it
/// refuses refused with its token named.
fn far_catalog(paths: &[std::path::PathBuf]) -> Result<Option<zenkey_model::catalog::Catalog>> {
    match paths {
        [] => Ok(None),
        [one] => {
            let text = std::fs::read_to_string(one)
                .map_err(|e| unaskable!("--catalog {}: {e}", one.display()))?;
            zenkey_model::catalog::Catalog::parse(&text)
                .map(Some)
                .map_err(|r| {
                    unaskable!(
                        "--catalog {}: core §8.6 refuses it as a catalog: {r} (link.v2 §2.18)",
                        one.display()
                    )
                })
        }
        several => Err(unaskable!(
            "{} catalogs given: the far side holds one --catalog (link.v2 §2.18)",
            several.len()
        )),
    }
}

/// The plan's warnings and refusals as stderr notes, for the modes that
/// render something *other* than the plan (`--json5`, `--explain`, the
/// check), so the honesty rides every path.
pub(crate) fn plan_notes(plan: &zenkey_fleet::report::AclPlan) {
    for w in &plan.warnings {
        eprintln!("{}", warning_note(w).to_line());
    }
    for f in plan.faces.iter().flat_map(|f| &f.findings) {
        eprintln!("{}", face_finding_note(f).to_line());
    }
    for r in &plan.refusals {
        eprintln!("{}", refusal_note(r).to_line());
    }
}

/// A face plan's warning (`link.v1` §2.3) as a note: what does not cross
/// that a contract exposed, or crosses without a verdict on liveness, is a
/// coverage claim.
pub(crate) fn face_finding_note(f: &zenkey_fleet::report::LinkFinding) -> Note {
    Note::coverage(format!(
        "face {}: {} {}: {}",
        f.face.as_deref().unwrap_or("-"),
        f.code,
        f.at,
        zenkey_fleet::link_says(f.code)
    ))
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
        face_policy,
        far_config,
        face,
        attach,
        far,
        region,
        ns,
    } = cli;

    ask(
        check,
        refuse_old_face_flags(&[
            ("--face", &face),
            ("--attach", &attach),
            ("--far", &far),
            ("--region", &region),
        ]),
    )?;
    let dep = ask(check, Deployment::resolve(&ns))?;
    let enrollment = ask(check, load_enrollment(&enrollment))?;
    let catalogs = contracts.catalog.clone();
    let contracts = ask(check, zk2::load_contracts(&contracts))?;
    let face_policy = match &face_policy {
        None => None,
        Some(p) => Some(ask(check, load_face_policy(p))?),
    };
    // The far side's catalog is a link.v2 face's (link.v2 §2.18): asked
    // before the catalog is read, so a catalog given beside no link.v2
    // policy is refused for that, whatever its file holds.
    let v2 = face_policy
        .as_ref()
        .and_then(|p| p.policy.as_ref())
        .is_some_and(|p| p.profile == zenkey_model::link::Profile::V2);
    if !catalogs.is_empty() && !v2 {
        ask(
            check,
            Err::<(), _>(unaskable!(
                "--catalog needs a link.v2 --face-policy (`profile = \"link.v2\"`, link.v2 \
                 §2.18): the catalog is what a face's far side decodes with"
            )),
        )?;
    }
    let catalog = ask(check, far_catalog(&catalogs))?;
    let opts = zenkey_fleet::AclOptions {
        namespace: enrollment
            .namespace
            .clone()
            .unwrap_or_else(|| dep.namespace().to_owned()),
        default_permission: permission(default_permission),
        face_policy,
        catalog,
    };
    let plan = ask(
        check,
        zenkey_fleet::plan_acl(&enrollment, &contracts, &opts).map_err(anyhow::Error::from),
    )?;

    if check {
        let against = against.expect("clap: --check requires --against");
        let observed = ASKING.ask(load_against(&against));
        let report = zenkey_fleet::check_acl(&plan, &observed, &against.display().to_string());
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

    if far_config {
        let far = zenkey_fleet::acl_far_json5(&plan).ok_or_else(|| {
            unaskable!(
                "--far-config: the face policy states no transport; a link.v2 policy does \
                 (`profile = \"link.v2\"` and a `[transport]` table, link.v2 §2.1)"
            )
        })?;
        print!("{far}");
        plan_notes(&plan);
    } else if json5 {
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
