//! `zenctl acl gen` (#392): RFC 09 §3's grant matrix, generated from an
//! enrollment file — and checked, and explained.
//!
//! Thin by design. The matrix, the four facts and the fifth, the registry
//! narrowing, the check and the explanation are all judgement over values
//! in hand, so they live in `zenkey_fleet::model::acl` where the other
//! explorer can reach them. What is left here is what only a CLI has: the
//! files (the enrollment, the router config), the registry source, the
//! rendering, and the exit.
//!
//! ## Three modes, one plan
//!
//! * **gen** renders the plan — the table for a person, the pinned shape for
//!   a script — or, under `--json5`, the router's `access_control` block on
//!   stdout with a comment per rule. The block is a foreign schema, zenoh's,
//!   so `--json5` and `--format` conflict the way `--dot` and `--format` do
//!   (#243): the render seam decides nothing here, it is the flag that does.
//!   A refused principal exits 1: rows were refused, which the exit contract
//!   calls a finding.
//! * **`--check --against <router.json5>`** is a judgement verb. The router's
//!   *running* block is not observable: zenoh 1.10's admin space registers
//!   no GET handler under `config/**` — it subscribes to that subtree for
//!   runtime edits and serves the root document, `plugins/**` and the
//!   declared-entity subtrees only (`zenoh-1.10.0/src/net/runtime/
//!   adminspace.rs`, `add_handler!`). So the observed side is the config
//!   *file*, read through `zenoh::Config::from_file` — zenoh's own loader,
//!   so what is compared is what `zenohd` would parse and refuse. Every
//!   failure before the comparison lands on the reserved 2 through
//!   [`ASKING`]; the verdict exits through [`crate::exit::verdict`].
//! * **`--explain <principal> <key> <message>`** is pure over the plan and
//!   exits 0; a principal the plan does not carry is an `Unaskable`.
//!
//! ## Where the registry comes in
//!
//! Slices only *enrich* the plan — they narrow the planes and the write set
//! — so they ride [`crate::Bus::slices_optional`], and they are asked only
//! when a registry source is named (`--registry`, or the context's dirs):
//! a generator run on a laptop with no bus should not wait out a fleet
//! sweep to say "not asked". The plan states which it was (RFC 13 §3 O4).

use std::path::Path;

use anyhow::{Context, Result};

use crate::Bus;
use crate::exit::unaskable;
use crate::render::Note;

/// This verb's name, spelled once (#355).
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("acl gen --check");

/// The enrollment file, parsed — a shape the engine owns
/// (`report::Enrollment`) and a file only the frontend reads.
fn load_enrollment(path: &Path) -> Result<zenkey_fleet::report::Enrollment> {
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

/// The router config's `access_control` block, as zenoh's loader parsed it.
fn load_against(path: &Path) -> Result<zenkey_fleet::report::AclConfigDoc> {
    let config = zenoh::Config::from_file(path)
        .map_err(|e| unaskable!("{}: zenoh refused the config: {e}", path.display()))?;
    let json = config
        .get_json("access_control")
        .map_err(|e| unaskable!("{}: no access_control block: {e}", path.display()))?;
    serde_json::from_str(&json).map_err(|e| {
        unaskable!(
            "{}: access_control does not parse as zenoh 1.10's AclConfig: {e}",
            path.display()
        )
    })
}

/// The plan's warnings and refusals as stderr notes — for the modes that
/// render something *other* than the plan (`--json5`, `--explain`), so the
/// honesty rides every path.
fn plan_notes(plan: &zenkey_fleet::report::AclPlan) {
    for w in &plan.warnings {
        eprintln!("{}", warning_note(w).to_line());
    }
    for r in &plan.refusals {
        eprintln!("{}", refusal_note(r).to_line());
    }
}

/// A warning as a note. A `write_set_not_narrowed` is a *coverage* claim —
/// what was not asked (RFC 13 §3 O4) — and rides the machine formats; the
/// rest are caveats about how to read the plan. The cite is data on the
/// warning and `Note::cite` wants a static, so it rides the text.
pub(crate) fn warning_note(w: &zenkey_fleet::report::AclWarning) -> Note {
    use zenkey_fleet::report::AclWarningKind;
    let who = w
        .principal
        .as_deref()
        .map_or(String::new(), |p| format!("{p}: "));
    let text = format!("{}{} ({})", who, w.text, w.cite);
    match w.kind {
        AclWarningKind::WriteSetNotNarrowed | AclWarningKind::PlaneNotDeclared => {
            Note::coverage(text)
        }
        _ => Note::caveat(text),
    }
}

pub(crate) fn refusal_note(r: &zenkey_fleet::report::AclRefusal) -> Note {
    Note::caveat(format!(
        "REFUSED {}: {} ({}) — left out of subjects and policies",
        r.principal, r.reason, r.cite
    ))
}

pub async fn run(cli: crate::cli::AclGenArgs) -> Result<()> {
    let crate::cli::AclGenArgs {
        enrollment,
        json5,
        check,
        against,
        explain,
        allow_zid_subjects,
        bus,
    } = cli;

    // A verdict verb on the check path: everything before the comparison
    // is `asked` — a resolution failure exiting 1 would read "the router's
    // block differs" about a block nobody read.
    let bus = if check {
        ASKING.ask(Bus::resolve(&bus))
    } else {
        Bus::resolve(&bus)?
    };
    let enrollment = if check {
        ASKING.ask(load_enrollment(&enrollment))
    } else {
        load_enrollment(&enrollment)?
    };
    let base = enrollment
        .base
        .clone()
        .unwrap_or_else(|| bus.base().to_string());

    // Slices enrich; asked only when a source is named (module doc).
    let slices = if bus.registry_dirs().is_empty() {
        None
    } else if check {
        ASKING.ask(bus.slices_optional().await)
    } else {
        bus.slices_optional().await?
    };
    let plan = zenkey_fleet::plan_acl(
        &enrollment,
        &base,
        slices.as_ref(),
        zenkey_fleet::AclOptions { allow_zid_subjects },
    );

    if check {
        let against = against.expect("clap: --check requires --against");
        let observed = ASKING.ask(load_against(&against));
        let report = zenkey_fleet::check_acl(&plan, &observed, &against.display().to_string());
        plan_notes(&plan);
        crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())?;
        // The library returns a verdict, the command exits with it — and a
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
            bus.format(),
            bus.color(),
        );
    }

    if json5 {
        print!("{}", zenkey_fleet::acl_plan_json5(&plan));
        plan_notes(&plan);
    } else {
        crate::render::emit_with(&mut std::io::stdout(), &plan, bus.format(), bus.color())?;
    }
    if !plan.refusals.is_empty() {
        std::process::exit(crate::exit::FINDING);
    }
    Ok(())
}
