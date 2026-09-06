//! `zenctl registry export|diff|lint|lock` (issue #50) — the registry as a
//! document, as a comparison, and as a checkable artifact. Plus
//! [`retired`](retired), which answers under `check` (#307) but reads the
//! same ledger and so lives with it, and [`consumers`](consumers) /
//! [`impact`](impact) (#224), which answer the question the registry
//! cannot on its own — *who reads this* — by joining it to the admin space.
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

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;

use crate::Bus;

/// The verdict verb's name, spelled once (#355) — the dispatcher
/// uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check retired");

/// What `registry export` emits.
pub async fn export(cli: crate::cli::RegistryExportArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::RegistryExportArgs {
        target,
        producer,
        bus: _,
    } = cli;
    let producer = producer.as_deref();
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
            let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
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
    let listen = for_secs.map(|secs| ASKING.ask(super::positive_secs("--for", secs)));
    // A verdict verb: every pre-run failure below goes through `asked`'s
    // exit 2 — an exit 1 here would read "a retired subject still speaks"
    // about a ledger nobody could walk.
    let dirs = args.registry_dirs();
    // The ledger source is the dirs alone, never the bus union: the served
    // slices are a *fact to check against* (§6.1), not a second ledger.
    let local = ASKING.ask(if dirs.is_empty() {
        Err(anyhow!(
            "check retired walks the [[deprecated]] ledger of local registry \
                 files — pass --registry <dir> (or set one on the active context)"
        ))
    } else {
        zenkey_fleet::SliceSet::from_dirs(&dirs).map_err(anyhow::Error::from)
    });
    let session = ASKING.ask(args.session().await);
    let entries: usize = local.slices().iter().map(|s| s.deprecated.len()).sum();
    if let Some(window) = listen {
        // Stated before the window opens, not after (O5).
        eprintln!(
            "{}",
            zenkey_fleet::retired_scope_note(
                entries,
                &zenkey_fleet::new_prefix(args.base()),
                window
            )
        );
    }
    let registries: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    let report = ASKING.ask(
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
pub fn lint(cli: crate::cli::RegistryLintArgs) -> Result<()> {
    let crate::cli::RegistryLintArgs { dir, ledger, out } = cli;
    let (dir, ledger) = (dir.as_path(), ledger.as_ref());
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
        // that it says exactly what the build would. Carried as the *error*,
        // not `format!`ed into a new one: `anyhow!("{e}")` renders the same
        // sentence and throws the type away, and the type is what
        // `exit::code_for` reads to tell a missing directory (no verdict,
        // exit 2) from a lint finding (a finding, exit 1) — #348.
        Err(e) => Err(e.into()),
    }
}

/// `registry lock <dir>` — regenerate the RFC 08 §3.1 compatibility lock.
/// Refuses an incompatible rewrite without `--force`; a forced break prints
/// every broken pin (loud by contract).
pub fn lock(cli: crate::cli::RegistryLockArgs) -> Result<()> {
    let crate::cli::RegistryLockArgs { dir, force, out } = cli;
    let dir = dir.as_path();
    let update = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed()
        .write_compat_lock(if force {
            zenkey_build::OnIncompatible::ForceAndReport
        } else {
            zenkey_build::OnIncompatible::Refuse
        })
        .map_err(anyhow::Error::from)?;
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

/// Whether a `registry consumers` target is already a wire key or selector
/// rather than a `<producer>/<path>` registry spelling: anything carrying a
/// wildcard or a verbatim chunk, or starting at the base or at `v1/`.
///
/// A registry path never contains `*` or `@` (`{var}` is its wildcard) and
/// never starts with `v1` — `v1` is not a producer name — so the two
/// spellings cannot collide.
fn is_raw_target(target: &str, base: &str) -> bool {
    target.contains('*')
        || target.contains('@')
        || target.starts_with("v1/")
        || (!base.is_empty() && target.starts_with(&format!("{base}/")))
}

/// Split `<producer>/<subject-path>`, or the refusal.
fn registry_form(target: &str) -> Result<(&str, &str)> {
    match target.split_once('/') {
        Some((producer, path)) if !producer.is_empty() && !path.is_empty() => Ok((producer, path)),
        _ => Err(crate::exit::unaskable!(
            "expected `<producer>/<subject-path>` (or a wire key/selector): {target:?}"
        )),
    }
}

/// `registry consumers <target>` (#224) — who declares a reader of it.
///
/// The target resolves one of two ways: a wire key or selector passes the
/// raw seam (`$*` refusal, RFC 03 §2) and is asked as spelled; a
/// `<producer>/<path>` is resolved through the slices to its family's
/// selector under the base. The slices *determine* the target here, so a
/// slice failure is exit 2 — nothing was asked of the admin space.
pub async fn consumers(cli: crate::cli::RegistryConsumersArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::RegistryConsumersArgs { target, bus: _ } = cli;
    let selector = if is_raw_target(&target, args.base()) {
        super::raw_selector(&target)?.to_string()
    } else {
        let (producer, path) = registry_form(&target)?;
        let slices = args.slice_set().await?;
        subject_selector(&slices, args.base(), producer, path)?.selector
    };
    let session = args.session().await?;
    let report = zenkey_fleet::consumers(&args.fleet(&session), &selector, args.timeout()).await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

/// `registry impact <producer>/<path>` (#224) — the blast radius of a
/// change to one declared subject. Always the registry form: the storage
/// coverage and the ledger entry are facts about a *declared* subject.
pub async fn impact(cli: crate::cli::RegistryImpactArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::RegistryImpactArgs { target, bus: _ } = cli;
    let (producer, path) = registry_form(&target)?;
    let slices = args.slice_set().await?;
    // Resolved before the session opens: an unknown subject is refused
    // without a bus round-trip, with the same sentence `consumers` uses.
    subject_selector(&slices, args.base(), producer, path)?;
    let session = args.session().await?;
    let report = zenkey_fleet::subject_impact(
        &args.fleet(&session),
        &slices,
        producer,
        path,
        args.timeout(),
    )
    .await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

/// The engine's resolution, with the refusal this tool's exit contract
/// wants: the caller named a subject nothing declares, so exit 2.
fn subject_selector(
    slices: &zenkey_fleet::SliceSet,
    base: &str,
    producer: &str,
    path: &str,
) -> Result<zenkey_fleet::SubjectTarget> {
    zenkey_fleet::subject_target(slices, base, producer, path).ok_or_else(|| {
        let mut known: Vec<&str> = slices.slices().iter().map(|s| s.name.as_str()).collect();
        known.sort_unstable();
        match slices.get(producer) {
            Some(_) => crate::exit::unaskable!(
                "producer {producer:?} declares no subject {path:?}, and its [[deprecated]] \
                 ledger does not retire one — `zenctl topic list --producer {producer}` \
                 lists what it declares"
            ),
            None => crate::exit::unaskable!(
                "no loaded slice names producer {producer:?} (known: {})",
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            ),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two target spellings cannot collide: a registry path carries no
    /// `*`/`@` and never starts at `v1`, a wire key always does one or the
    /// other.
    #[test]
    fn raw_targets_are_told_from_registry_paths() {
        assert!(is_raw_target("v1/*/state/sysinfo/health", ""));
        assert!(is_raw_target(
            "acme/v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "acme"
        ));
        assert!(is_raw_target("v1/@catalog/state/entity/**", "acme"));
        assert!(is_raw_target("**", ""));
        assert!(!is_raw_target("sysinfo/health", ""));
        assert!(!is_raw_target("sysinfo/disk/{mount}/used", "acme"));
        // A base-less bus: a key starting at `v1/` is still raw.
        assert!(is_raw_target("v1/h-3fa9c2d41b7e/state/sysinfo/health", ""));
        // Not the base: `acme-dev/...` is a producer named `acme-dev`.
        assert!(!is_raw_target("acme-dev/health", "acme"));
    }

    #[test]
    fn the_registry_form_needs_both_halves() {
        assert_eq!(
            registry_form("sysinfo/health").unwrap(),
            ("sysinfo", "health")
        );
        assert_eq!(
            registry_form("sysinfo/disk/{mount}/used").unwrap(),
            ("sysinfo", "disk/{mount}/used")
        );
        assert!(registry_form("sysinfo").is_err());
        assert!(registry_form("sysinfo/").is_err());
        assert!(registry_form("/health").is_err());
    }
}
