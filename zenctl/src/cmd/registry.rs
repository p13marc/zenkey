//! `zenctl registry export|diff|lint|lock` (issue #50) — the registry as a
//! document, as a comparison, and as a checkable artifact. Plus
//! [`retired`](retired), which answers under `check` (#307) but reads the
//! same ledger and so lives with it.
//!
//! The four sit together because they answer the questions an operator has
//! about a registry they did not write: *what does it say* (export), *does
//! the fleet agree with the checkout* (diff), *would this pass a build*
//! (lint), and *what is pinned* (lock).
//!
//! `lint` deliberately runs the real thing — `zenkey_build::Config::lint`, the
//! same lints a consumer's `build.rs` runs, stopping where a build would. A
//! lint that reported more than the build does would be a different tool
//! wearing the same name.

use crate::cli::ExportAs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;

use crate::Bus;

/// What `registry export` emits.
pub async fn export(target: ExportAs, producer: Option<&str>, args: &Bus) -> Result<()> {
    let slices = args.slice_set().await?;
    let selected: Vec<&RegistrySlice> = slices
        .slices()
        .iter()
        .filter(|s| producer.is_none_or(|p| p == s.name))
        .collect();
    if selected.is_empty() {
        // Silence under a fan-out is exit 2, not 1 (`crate::exit`): nothing
        // answered, so there is no document and no finding either.
        return Err(crate::exit::unaskable!(
            "no slices to export{} — an empty set is not a verdict (RFC 05 §3.1); \
             try --registry <dir> or check --base",
            producer
                .map(|p| format!(" for producer {p:?}"))
                .unwrap_or_default()
        ));
    }

    match target {
        ExportAs::Toml => {
            // One document per producer, separated by the file boundary a
            // registry dir would have. A single concatenated TOML would not
            // re-parse: the format is one slice per file by construction.
            for (i, slice) in selected.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                println!("# ── {}.toml ─────────────────────────────", slice.name);
                print!("{}", zenkey::slice_to_toml(slice));
            }
        }
        ExportAs::Jsonschema => {
            let session = args.session().await?;
            let store = zenkey_fleet::model::decode::SchemaStore::new(args.base(), args.timeout());
            // Fetch here, where the session is; shape the document in
            // zenkey-build, where every other registry-in-document-out
            // exporter lives (#208).
            let mut served = Vec::new();
            let mut undescribed: Vec<String> = Vec::new();
            for slice in &selected {
                match store.set_for(&session, &slice.name).await {
                    Some(set) => served.push(set),
                    None => undescribed.push(slice.name.clone()),
                }
            }
            let types: Vec<zenkey_build::export::BundledType<'_>> = served
                .iter()
                .flat_map(|set| {
                    set.iter()
                        .map(|(name, schema)| zenkey_build::export::BundledType {
                            name,
                            document: schema.json_document(),
                            kind: schema.kind_str(),
                            hash: schema.hash().unwrap_or_default(),
                        })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&zenkey_build::export::json_schema_bundle(
                    &types,
                    &undescribed
                ))?
            );
        }
        ExportAs::Asyncapi => {
            println!(
                "{}",
                serde_json::to_string_pretty(&zenkey_build::export::asyncapi(&selected))?
            );
        }
    }
    Ok(())
}

/// `registry diff` — local dirs against the live bus, per producer.
pub async fn diff(args: &Bus) -> Result<()> {
    let dirs = args.registry_dirs();
    if dirs.is_empty() {
        // One half of the comparison is missing, so the question cannot be
        // put at all: exit 2, not the 1 that would read as "they differ".
        return Err(crate::exit::unaskable!(
            "registry diff compares local files against the bus — pass --registry <dir> \
             (or set one on the active context)"
        ));
    }
    let local = zenkey_fleet::SliceSet::from_dirs(&dirs)?;
    let session = args.session().await?;
    let served = zenkey_fleet::SliceSet::from_bus(&args.fleet(&session), args.timeout()).await?;
    let report = served.diff(&local);
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

/// `zenctl check retired` (issue #226) — the deprecation burn-down.
///
/// It lives here, beside the registry it reads, and answers under `check`
/// because it is an exit-coded assertion (#307): one contract, one family.
///
/// Thin for `check cutover`'s reason (#206): the per-entry ladder, the wire
/// bucketing and the worst-of verdict are judgement over bus traffic and live
/// in `zenkey_fleet::judge::retired`. What is left here is what only a CLI has: the
/// session, the rendering, and the exit code.
pub async fn retired(for_secs: Option<f64>, args: &Bus) -> Result<()> {
    // Seconds off the flag, a `Duration` from here in.
    let listen = for_secs
        .map(|secs| crate::exit::asked("check retired", super::positive_secs("--for", secs)));
    // A verdict verb: every pre-run failure below goes through `asked`'s
    // exit 2 — an exit 1 here would read "a retired subject still speaks"
    // about a ledger nobody could walk.
    let dirs = args.registry_dirs();
    // The ledger source is the dirs alone, never the bus union: the served
    // slices are a *fact to check against* (§6.1), not a second ledger.
    let local = crate::exit::asked(
        "check retired",
        if dirs.is_empty() {
            Err(anyhow!(
                "check retired walks the [[deprecated]] ledger of local registry \
                 files — pass --registry <dir> (or set one on the active context)"
            ))
        } else {
            zenkey_fleet::SliceSet::from_dirs(&dirs)
        },
    );
    let session = crate::exit::asked("check retired", args.session().await);
    let entries: usize = local.slices().iter().map(|s| s.deprecated.len()).sum();
    if let Some(window) = listen {
        // Stated before the window opens, not after (O5).
        eprintln!(
            "{}",
            zenkey_fleet::judge::retired::scope_note(
                entries,
                &zenkey_fleet::new_prefix(args.base()),
                window
            )
        );
    }
    let registries: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    let report = crate::exit::asked(
        "check retired",
        zenkey_fleet::run_retired(
            &args.fleet(&session),
            &local,
            registries,
            listen,
            args.timeout(),
        )
        .await,
    );
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // The exit discipline shared with `check cutover` and `check expect`: a
    // library returns a verdict, a command exits with it, through the one
    // projection in `crate::exit` (0 = pass, 1 = a retired subject still
    // speaks, 2 = unproven — silence is not a pass).
    crate::exit::verdict(&report.verdict.to_judgement())
}

/// `registry lint <dir>` — the consumer's build lints, without the build.
pub fn lint(dir: &Path, ledger: Option<&PathBuf>, out: crate::cli::OutputArgs) -> Result<()> {
    let mut config = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed();
    if let Some(l) = ledger {
        config = config.ledger(l);
    }
    match config.lint() {
        Ok(warnings) => crate::render::emit_with(
            &mut std::io::stdout(),
            &crate::render::LintReport {
                dir: dir.display().to_string(),
                warnings: warnings.iter().map(ToString::to_string).collect(),
            },
            out.format,
            out.color,
        ),
        // The build's own wording, verbatim — the value of this command is
        // that it says exactly what the build would.
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// `registry lock <dir>` — regenerate the RFC 08 §3.1 compatibility lock.
/// Refuses an incompatible rewrite without `--force`; a forced break prints
/// every broken pin (loud by contract).
pub fn lock(dir: &Path, force: bool, out: crate::cli::OutputArgs) -> Result<()> {
    let update = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed()
        .write_compat_lock(if force {
            zenkey_build::OnIncompatible::ForceAndReport
        } else {
            zenkey_build::OnIncompatible::Refuse
        })
        .map_err(|e| anyhow!("{e}"))?;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::LockReport {
            path: update.path.display().to_string(),
            created: update.created,
            added: update.added,
            retired: update.retired,
            forced: update.forced.iter().map(ToString::to_string).collect(),
        },
        out.format,
        out.color,
    )
}
