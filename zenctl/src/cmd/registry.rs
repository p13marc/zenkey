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

use anyhow::{Context as _, Result, anyhow};
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
    let crate::cli::RegistryLintArgs {
        dir,
        ledger,
        allow_drafts,
        out,
    } = cli;
    let (dir, ledger) = (dir.as_path(), ledger.as_ref());
    let mut config = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed()
        .allow_drafts(allow_drafts);
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

/// `registry infer` (#225, RFC 08 §6.1) — draft a registry from the wire,
/// marked as a draft.
///
/// Thin, the `field` shape: the window (live through the monitor, or a
/// `.zrec` read under its own base) feeds `zenkey_fleet::InferObservation`,
/// `infer` drafts the report, and what only a CLI has is here — the
/// output directory, the files, the rendering. Slices are a *hint* only
/// (`{var}` names where a declared pattern binds, a service's name), so
/// they degrade through `slices_optional`; the draft comes out without them
/// and the report says so.
///
/// The output directory is checked **before** anything is written: it must
/// be creatable and must not already hold any file this run would write —
/// all-or-nothing, and a refusal is exit 2 (nothing was drafted, so there
/// is no finding). A draft never overwrites: the file a reviewer is halfway
/// through is exactly the one an overwrite would destroy.
pub async fn infer(cli: crate::cli::RegistryInferArgs) -> Result<()> {
    use std::io::BufReader;

    use zenkey_fleet::{FleetEvent, InferObservation, StreamItem, ZrecItem, ZrecReader};

    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::RegistryInferArgs {
        from,
        for_secs,
        out,
        app,
        max_keys,
        max_paths,
        bus: _,
    } = cli;
    if max_keys == 0 || max_paths == 0 {
        return Err(crate::exit::unaskable!(
            "--max-keys and --max-paths must be at least 1 — a zero bound observes nothing"
        ));
    }
    let app = app.unwrap_or_else(|| "unknown".to_string());

    // A capture is a path that exists and ends in `.zrec`; anything else
    // is a selector. The two never collide: a selector is a key
    // expression, and a key expression that names an existing file with
    // that suffix is a coincidence this tool refuses to guess about.
    let capture = from
        .as_deref()
        .filter(|f| f.ends_with(".zrec") && std::path::Path::new(f).is_file())
        .map(std::path::PathBuf::from);

    // The QoS label the observation records: the profile the wire's axes
    // match, else the axes spelled out. A comment in the draft, never a
    // field.
    fn qos_label(s: &zenkey_fleet::SampleView) -> String {
        zenkey::QosProfile::ALL
            .iter()
            .find(|p| s.qos_matches(**p))
            .map(|p| p.name().to_string())
            .unwrap_or_else(|| {
                format!(
                    "no profile: {:?}/{:?}/{:?}{}",
                    s.priority,
                    s.congestion_control,
                    s.reliability,
                    if s.express { "/express" } else { "" }
                )
            })
    }

    // Hints are a registry the operator already has. A capture is read
    // without a session — the bus it came from may be long gone — so its
    // hints come from `--registry` dirs alone; a live window asks the bus
    // too, degrading through the one door (#210).
    let slices = match (&capture, args.registry_dirs()) {
        (Some(_), dirs) if dirs.is_empty() => None,
        (Some(_), dirs) => Some(zenkey_fleet::SliceSet::from_dirs(&dirs)?),
        (None, _) => args.slices_optional().await?,
    };
    // "Hinted" means a slice bound: an empty set names no var and no
    // service, and saying it hinted would be a false claim (O4).
    let hints = slices.as_ref().filter(|s| !s.slices().is_empty());
    let (obs, source, window_s, dropped) = match capture {
        Some(path) => {
            if for_secs.is_some() {
                return Err(crate::exit::unaskable!(
                    "--for is a live window; a capture's span is in its rows — drop one of the two"
                ));
            }
            let file =
                std::fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
            let mut reader = ZrecReader::new(BufReader::new(file))
                .with_context(|| format!("read {} as a .zrec", path.display()))?;
            let base = reader.header().base.clone();
            let mut obs = InferObservation::new(&base, max_keys, max_paths);
            let mut dropped = 0u64;
            let mut malformed = 0u64;
            while let Some(item) = reader.next() {
                match item {
                    Ok(ZrecItem::Sample { row, t_us, .. }) => {
                        let at_s = t_us.map_or(0.0, |t| t as f64 / 1e6);
                        let doc = (!row.delete)
                            .then(|| zenkey_fleet::structural_value(&row.payload))
                            .flatten();
                        obs.observe(
                            &row.key,
                            at_s,
                            row.encoding.as_deref(),
                            row.qos.as_deref(),
                            doc.as_ref(),
                        );
                    }
                    Ok(ZrecItem::Dropped(n)) => dropped += n,
                    Err(_) => malformed += 1,
                }
            }
            if malformed > 0 {
                eprintln!("infer: {malformed} malformed row(s) in the capture skipped");
            }
            (obs, path.display().to_string(), None, dropped)
        }
        None => {
            let selector = match from.as_deref() {
                Some(sel) => super::raw_selector(sel)?.to_string(),
                None => zenkey::grammar::with_base(args.base(), "v1/**"),
            };
            let secs = for_secs.unwrap_or(60.0);
            let window = super::positive_secs("--for", secs)?;
            let session = args.session().await?;
            let monitor =
                zenkey_fleet::Monitor::start(&session, zenkey_fleet::MonitorSpec::default())
                    .await?;
            let mut events = monitor.events();
            // Declared before the window opens (O4).
            let monitor = monitor.watching([selector.as_str()]).await?;
            let opened = std::time::Instant::now();
            eprintln!(
                "infer: watching {selector} for {secs}s — subscriber declared before the window opened (RFC 09 §5.1 O4)"
            );
            let mut obs = InferObservation::new(args.base(), max_keys, max_paths);
            let mut dropped = 0u64;
            let window_over = tokio::time::sleep(window);
            tokio::pin!(window_over);
            loop {
                let item = tokio::select! {
                    item = events.recv() => item,
                    () = &mut window_over => break,
                };
                match item {
                    Some(StreamItem::Event(FleetEvent::Sample(s))) => {
                        let at_s = s.received.duration_since(opened).as_secs_f64();
                        let qos = qos_label(&s);
                        let bytes = s.payload.to_bytes();
                        if bytes.len() > zenkey_fleet::OBSERVE_LIMIT {
                            obs.observe_unread(&s.key, at_s, Some(&s.encoding), Some(&qos));
                        } else {
                            let doc = (s.kind == zenoh::sample::SampleKind::Put)
                                .then(|| zenkey_fleet::structural_value(&bytes))
                                .flatten();
                            obs.observe(&s.key, at_s, Some(&s.encoding), Some(&qos), doc.as_ref());
                        }
                    }
                    Some(StreamItem::Dropped(n)) => dropped += n,
                    Some(_) => continue,
                    None => break,
                }
            }
            monitor.shutdown().await?;
            (obs, selector, Some(secs), dropped)
        }
    };

    let mut report = zenkey_fleet::infer(&obs, &source, window_s, hints);
    report.dropped = dropped;
    if report.producers.is_empty() {
        // Silence under a window is exit 2, not a draft of nothing: an
        // empty directory would read as "this fleet has no subjects".
        return Err(crate::exit::unaskable!(
            "nothing to draft: {} sample(s) on {} v1 key(s) under {source} — no draft written",
            report.samples,
            report.keys_seen
        ));
    }

    // The output directory, checked whole before a byte is written.
    let names = zenkey_fleet::draft_file_names(&report);
    if out.exists() && !out.is_dir() {
        return Err(crate::exit::unaskable!(
            "--out {} exists and is not a directory",
            out.display()
        ));
    }
    let occupied: Vec<&String> = names.iter().filter(|n| out.join(n).exists()).collect();
    if !occupied.is_empty() {
        return Err(crate::exit::unaskable!(
            "--out {} already holds {} file(s) this run would write ({}…) — a draft never overwrites; pick an empty directory",
            out.display(),
            occupied.len(),
            occupied
                .iter()
                .take(3)
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    std::fs::create_dir_all(out.join("schemas"))
        .with_context(|| format!("create {}", out.join("schemas").display()))?;
    let prov = zenkey_fleet::Provenance {
        app,
        source: source.clone(),
        span_s: report.span_s,
        at: zenkey_fleet::rfc3339_now(),
        keys: report.keys_seen,
        samples: report.samples,
        dropped: report.dropped,
    };
    let mut written = Vec::new();
    for p in &report.producers {
        let path = out.join(format!("{}.toml", p.name));
        std::fs::write(&path, zenkey_fleet::to_draft_toml(p, &prov))
            .with_context(|| format!("write {}", path.display()))?;
        written.push(path);
    }
    let types = out.join("types.toml");
    std::fs::write(
        &types,
        zenkey_fleet::to_draft_types_toml(&report.producers, &prov),
    )
    .with_context(|| format!("write {}", types.display()))?;
    written.push(types);
    for (rel, body) in zenkey_fleet::draft_schema_files(&report.producers) {
        let path = out.join(rel);
        std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        written.push(path);
    }
    eprintln!(
        "infer: wrote {} file(s) under {} — run `zenctl registry lint --allow-drafts {}` to check the draft; review it, drop `draft = true` and add `since` to promote it",
        written.len(),
        out.display(),
        out.display()
    );
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
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
