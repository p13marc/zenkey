//! Named connection contexts — zenctl's verbs over the shared store.
//!
//! The store itself (format, paths, `load`/`save`/`active`) moved to
//! `zenkey_fleet::context_store` (issue #35) so zengui resolves the same
//! contexts. Reads fall back to the legacy `~/.config/zenctl/` location;
//! writes go to the neutral `~/.config/zenkey-explorer/` — see the store's
//! module docs for the migration policy.

use anyhow::{Context as _, Result, anyhow, bail};

pub use zenkey_fleet::context_store::{StoredContext, active, load, save};

/// `zenctl context <verb>` — the six verbs, dispatched.
///
/// Lived in the match arm until #209, where it was the only arm that built a
/// `StoredContext` field by field: the flags-to-record mapping is a fact about
/// contexts, and it belongs beside the code that reads the record back.
pub fn dispatch(cmd: crate::cli::ContextCmd) -> Result<()> {
    use crate::cli::ContextCmd;
    match cmd {
        ContextCmd::Create {
            name,
            base,
            connect,
            listen,
            registry,
            scouting,
            timeout,
            zenoh_config,
            select,
            out,
        } => create(
            &name,
            StoredContext {
                base,
                connect,
                listen,
                registry,
                // An unset flag means "leave it alone", here as everywhere
                // else in this tool — so `false` is `None`, not `Some(false)`.
                scouting: scouting.then_some(true),
                timeout,
                zenoh_config,
            },
            select,
            out,
        ),
        ContextCmd::List { out } => list(out),
        ContextCmd::Edit { out } => edit(out),
        ContextCmd::Show { name, out } => show(name.as_deref(), out),
        ContextCmd::Select { name, out } => select(&name, out),
        ContextCmd::Rm { name, out } => remove(&name, out),
    }
}

/// One report out, however the user asked for it.
fn emit<R: crate::render::Render>(r: &R, out: crate::cli::OutputArgs) -> Result<()> {
    crate::render::emit_with(&mut std::io::stdout(), r, out.format, out.color)
}

/// `context edit` — open the whole config file in `$VISUAL`/`$EDITOR`, then
/// validate: a file that no longer parses is reported (with its path) and
/// kept — the user's edit is never discarded, but a broken config must not
/// fail silently at the next command.
pub fn edit(out: crate::cli::OutputArgs) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .map_err(|_| anyhow!("neither $VISUAL nor $EDITOR is set"))?;
    // Ensure the file exists so a first-run edit opens something real.
    let config = load()?;
    save(&config)?;
    let path = zenkey_fleet::context_store::config_path();
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("running {editor:?}"))?;
    if !status.success() {
        bail!("{editor} exited with {status} — config left as it is");
    }
    match load() {
        Ok(config) => emit(&list_report(&config)?, out),
        Err(e) => {
            eprintln!("{} no longer parses: {e}", path.display());
            eprintln!("the file is kept as you wrote it — fix it and re-run");
            std::process::exit(1);
        }
    }
}

/// `zenctl context <verb>` handlers.
///
/// On an existing name this **updates** — and says so — which means it must
/// not clobber the fields the invocation did not mention. `create lab
/// --connect tcp/…` used to replace the whole entry, so it silently dropped
/// the base, registry dirs and timeout an earlier `create` had set (the CLI
/// half of issue #194). An unset flag means "leave it alone" here exactly as
/// it does everywhere else in this tool; clearing a field is `context edit`.
pub fn create(
    name: &str,
    flags: StoredContext,
    select: bool,
    out: crate::cli::OutputArgs,
) -> Result<()> {
    let mut config = load()?;
    let existed = config.contexts.contains_key(name);
    zenkey_fleet::context_store::upsert(&mut config, name, |c| {
        if flags.base.is_some() {
            c.base = flags.base;
        }
        if !flags.connect.is_empty() {
            c.connect = flags.connect;
        }
        if !flags.listen.is_empty() {
            c.listen = flags.listen;
        }
        if !flags.registry.is_empty() {
            c.registry = flags.registry;
        }
        // `--scouting` is a presence flag, so it arrives as `Some(true)` or
        // nothing — it has never been able to store an explicit `false`.
        if flags.scouting.is_some() {
            c.scouting = flags.scouting;
        }
        if flags.timeout.is_some() {
            c.timeout = flags.timeout;
        }
        if flags.zenoh_config.is_some() {
            c.zenoh_config = flags.zenoh_config;
        }
    });
    if select || config.current.is_none() {
        config.current = Some(name.to_string());
    }
    save(&config)?;
    emit(
        &crate::render::ContextAction {
            action: if existed { "updated" } else { "created" },
            name: name.to_string(),
            current: config.current.as_deref() == Some(name),
        },
        out,
    )
}

/// The config file as `context list` reports it.
fn list_report(
    config: &zenkey_fleet::context_store::ConfigFile,
) -> Result<crate::render::ContextList> {
    Ok(crate::render::ContextList {
        path: zenkey_fleet::context_store::config_path()
            .display()
            .to_string(),
        contexts: config
            .contexts
            .iter()
            .map(|(name, c)| crate::render::ContextRow {
                name: name.clone(),
                current: config.current.as_deref() == Some(name.as_str()),
                base: c.base.clone(),
                connect: c.connect.clone(),
            })
            .collect(),
    })
}

pub fn list(out: crate::cli::OutputArgs) -> Result<()> {
    // The empty case is a note on the report, not an early return: an empty
    // set is a state and it says what to type next, and a `return` here is
    // exactly what used to make `--format ndjson` print nothing at all (#199).
    emit(&list_report(&load()?)?, out)
}

pub fn show(name: Option<&str>, out: crate::cli::OutputArgs) -> Result<()> {
    let config = load()?;
    let name = name
        .map(str::to_string)
        .or(config.current.clone())
        .ok_or_else(|| anyhow!("no context selected"))?;
    let c = config
        .contexts
        .get(&name)
        .ok_or_else(|| anyhow!("context {name:?} not found"))?;
    emit(
        &crate::render::ContextShow {
            current: config.current.as_deref() == Some(name.as_str()),
            name,
            context: c.clone(),
        },
        out,
    )
}

pub fn select(name: &str, out: crate::cli::OutputArgs) -> Result<()> {
    let mut config = load()?;
    if !config.contexts.contains_key(name) {
        bail!("context {name:?} not found — `zenctl context list`");
    }
    config.current = Some(name.to_string());
    save(&config)?;
    emit(
        &crate::render::ContextAction {
            action: "selected",
            name: name.to_string(),
            current: true,
        },
        out,
    )
}

pub fn remove(name: &str, out: crate::cli::OutputArgs) -> Result<()> {
    let mut config = load()?;
    if config.contexts.remove(name).is_none() {
        bail!("context {name:?} not found");
    }
    if config.current.as_deref() == Some(name) {
        config.current = None;
    }
    save(&config)?;
    emit(
        &crate::render::ContextAction {
            action: "removed",
            name: name.to_string(),
            current: false,
        },
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize the whole config lifecycle against a scratch config dir.
    /// One test (not several) because ZENCTL_CONFIG_DIR is process-global.
    /// The verbs write a report now, and a test has no opinion about how it
    /// is rendered — table to a captured stdout is fine, and `Never` keeps
    /// escapes out of a comparison nobody makes.
    fn quiet() -> crate::cli::OutputArgs {
        crate::cli::OutputArgs {
            format: crate::render::Format::Table,
            color: crate::render::ColorChoice::Never,
        }
    }

    #[test]
    fn config_lifecycle_round_trip() {
        let dir = std::env::temp_dir().join(format!("zenctl-ctx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // SAFETY: single-threaded test process section; the var is read
        // through config_path() only.
        unsafe { std::env::set_var("ZENCTL_CONFIG_DIR", &dir) };

        assert!(load().unwrap().contexts.is_empty());
        assert!(active(None).unwrap().is_none());

        let stored = StoredContext {
            base: Some("zensight".into()),
            connect: vec!["tcp/127.0.0.1:7447".into()],
            ..Default::default()
        };
        create("lab", stored.clone(), false, quiet()).unwrap();
        // First context auto-selects.
        assert_eq!(active(None).unwrap(), Some(stored.clone()));

        // Explicit name beats the pointer; unknown explicit name errors.
        create("prod", StoredContext::default(), false, quiet()).unwrap();
        assert_eq!(active(Some("lab")).unwrap(), Some(stored));
        assert!(active(Some("nope")).is_err());

        select("prod", quiet()).unwrap();
        assert_eq!(active(None).unwrap(), Some(StoredContext::default()));

        remove("prod", quiet()).unwrap();
        // Dangling current pointer degrades to none, not an error.
        assert!(active(None).unwrap().is_none());

        // An explicitly empty base (`--base ""`, the observer identity)
        // survives the TOML round trip as Some("") — distinct from None.
        let empty = StoredContext {
            base: Some(String::new()),
            ..Default::default()
        };
        create("bare", empty.clone(), true, quiet()).unwrap();
        assert_eq!(active(None).unwrap(), Some(empty));

        // `create` on an existing name updates, and an update must not drop
        // what the invocation did not mention (#194). This is the CLI half of
        // the same hazard the GUI's connect pane had.
        let full = StoredContext {
            base: Some("acme".into()),
            connect: vec!["tcp/127.0.0.1:7447".into()],
            registry: vec!["/abs/registry".into()],
            timeout: Some(10),
            ..Default::default()
        };
        create("keep", full, false, quiet()).unwrap();
        // …now the same shape a `--connect`-only invocation produces.
        create(
            "keep",
            StoredContext {
                connect: vec!["tcp/10.0.0.1:7447".into()],
                ..Default::default()
            },
            false,
            quiet(),
        )
        .unwrap();
        let kept = active(Some("keep")).unwrap().expect("keep");
        assert_eq!(
            kept.connect,
            ["tcp/10.0.0.1:7447"],
            "the flag that was given wins"
        );
        assert_eq!(
            kept.registry,
            [std::path::PathBuf::from("/abs/registry")],
            "a flag that was not given must not clear the field"
        );
        assert_eq!(kept.timeout, Some(10), "nor the timeout");
        assert_eq!(kept.base.as_deref(), Some("acme"), "nor the base");

        unsafe { std::env::remove_var("ZENCTL_CONFIG_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
