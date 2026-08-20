//! `zenctl registry export|diff|lint` (issue #50) — the registry as a
//! document, as a comparison, and as a checkable artifact.
//!
//! The three sit together because they answer the three questions an operator
//! has about a registry they did not write: *what does it say* (export),
//! *does the fleet agree with the checkout* (diff), and *would this pass a
//! build* (lint).
//!
//! `lint` deliberately runs the real thing — `zenkey_build::Config::lint`, the
//! same lints a consumer's `build.rs` runs, stopping where a build would. A
//! lint that reported more than the build does would be a different tool
//! wearing the same name.

use crate::cli::ExportAs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;

use crate::{BusArgs, output};

/// What `registry export` emits.
pub async fn export(target: ExportAs, producer: Option<&str>, args: &BusArgs) -> Result<()> {
    let slices = args.slice_set().await?;
    let selected: Vec<&RegistrySlice> = slices
        .slices()
        .iter()
        .filter(|s| producer.is_none_or(|p| p == s.name))
        .collect();
    if selected.is_empty() {
        return Err(anyhow!(
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
            let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
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
                            kind: schema.kind().as_str(),
                            hash: schema.hash(),
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
pub async fn diff(args: &BusArgs) -> Result<()> {
    let dirs = args.registry_dirs();
    if dirs.is_empty() {
        return Err(anyhow!(
            "registry diff compares local files against the bus — pass --registry <dir> \
             (or set one on the active context)"
        ));
    }
    let local = zenkey_fleet::SliceSet::from_dirs(&dirs)?;
    let session = args.session().await?;
    let served = zenkey_fleet::SliceSet::from_bus(&session, args.base(), args.timeout()).await?;
    let report = served.diff(&local);
    output::registry_diff(&report, args.format)
}

/// `registry lint <dir>` — the consumer's build lints, without the build.
pub fn lint(dir: &Path, ledger: Option<&PathBuf>) -> Result<()> {
    let mut config = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed();
    if let Some(l) = ledger {
        config = config.ledger(l);
    }
    match config.lint() {
        Ok(()) => {
            println!("{}: registry lints pass (RFC 08 §5).", dir.display());
            Ok(())
        }
        // The build's own wording, verbatim — the value of this command is
        // that it says exactly what the build would.
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// `registry lock <dir>` — regenerate the RFC 08 §3.1 compatibility lock.
/// Refuses an incompatible rewrite without `--force`; a forced break prints
/// every broken pin (loud by contract).
pub fn lock(dir: &Path, force: bool) -> Result<()> {
    let update = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed()
        .write_compat_lock(force)
        .map_err(|e| anyhow!("{e}"))?;
    println!(
        "{}: {} ({} pinned entr{} added, {} released)",
        update.path.display(),
        if update.created { "created" } else { "updated" },
        update.added,
        if update.added == 1 { "y" } else { "ies" },
        update.retired,
    );
    for broken in &update.forced {
        eprintln!("FORCED BREAK (RFC 08 §3.1):\n{broken}");
    }
    Ok(())
}
