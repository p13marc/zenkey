//! Named connection contexts — zenctl's verbs over the shared store.
//!
//! The store itself (format, paths, `load`/`save`/`active`) moved to
//! `zenkey_fleet::context_store` (issue #35) so zengui resolves the same
//! contexts. Reads fall back to the legacy `~/.config/zenctl/` location;
//! writes go to the neutral `~/.config/zenkey-explorer/` — see the store's
//! module docs for the migration policy.

use anyhow::{Context as _, Result, anyhow, bail};

pub use zenkey_fleet::context_store::{StoredContext, active, load, save};

/// `context edit` — open the whole config file in `$VISUAL`/`$EDITOR`, then
/// validate: a file that no longer parses is reported (with its path) and
/// kept — the user's edit is never discarded, but a broken config must not
/// fail silently at the next command.
pub fn edit() -> Result<()> {
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
        Ok(config) => {
            println!(
                "ok: {} context(s){}",
                config.contexts.len(),
                config
                    .current
                    .as_deref()
                    .map(|c| format!(", {c:?} selected"))
                    .unwrap_or_default()
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("{} no longer parses: {e}", path.display());
            eprintln!("the file is kept as you wrote it — fix it and re-run");
            std::process::exit(1);
        }
    }
}

/// `zenctl context <verb>` handlers.
pub fn create(name: &str, stored: StoredContext, select: bool) -> Result<()> {
    let mut config = load()?;
    let existed = config.contexts.insert(name.to_string(), stored).is_some();
    if select || config.current.is_none() {
        config.current = Some(name.to_string());
    }
    save(&config)?;
    println!(
        "{} context {name:?}{}",
        if existed { "updated" } else { "created" },
        if config.current.as_deref() == Some(name) {
            " (selected)"
        } else {
            ""
        }
    );
    Ok(())
}

pub fn list() -> Result<()> {
    let config = load()?;
    if config.contexts.is_empty() {
        println!(
            "no contexts. create one:\n  zenctl context create lab --base zensight -c tcp/127.0.0.1:7447"
        );
        return Ok(());
    }
    for (name, c) in &config.contexts {
        let marker = if config.current.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            " "
        };
        // Three distinct renderings: a stored empty base (`""`, a legal
        // observer target) must not read like an unset one (`-`).
        let base = match c.base.as_deref() {
            Some("") => "\"\"",
            Some(b) => b,
            None => "-",
        };
        println!("{marker} {name}  base={base}  connect={:?}", c.connect);
    }
    Ok(())
}

pub fn show(name: Option<&str>) -> Result<()> {
    let config = load()?;
    let name = name
        .map(str::to_string)
        .or(config.current.clone())
        .ok_or_else(|| anyhow!("no context selected"))?;
    let c = config
        .contexts
        .get(&name)
        .ok_or_else(|| anyhow!("context {name:?} not found"))?;
    print!("{}", toml::to_string_pretty(c).context("serializes")?);
    Ok(())
}

pub fn select(name: &str) -> Result<()> {
    let mut config = load()?;
    if !config.contexts.contains_key(name) {
        bail!("context {name:?} not found — `zenctl context list`");
    }
    config.current = Some(name.to_string());
    save(&config)?;
    println!("selected context {name:?}");
    Ok(())
}

pub fn remove(name: &str) -> Result<()> {
    let mut config = load()?;
    if config.contexts.remove(name).is_none() {
        bail!("context {name:?} not found");
    }
    if config.current.as_deref() == Some(name) {
        config.current = None;
    }
    save(&config)?;
    println!("removed context {name:?}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize the whole config lifecycle against a scratch config dir.
    /// One test (not several) because ZENCTL_CONFIG_DIR is process-global.
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
        create("lab", stored.clone(), false).unwrap();
        // First context auto-selects.
        assert_eq!(active(None).unwrap(), Some(stored.clone()));

        // Explicit name beats the pointer; unknown explicit name errors.
        create("prod", StoredContext::default(), false).unwrap();
        assert_eq!(active(Some("lab")).unwrap(), Some(stored));
        assert!(active(Some("nope")).is_err());

        select("prod").unwrap();
        assert_eq!(active(None).unwrap(), Some(StoredContext::default()));

        remove("prod").unwrap();
        // Dangling current pointer degrades to none, not an error.
        assert!(active(None).unwrap().is_none());

        // An explicitly empty base (`--base ""`, the observer identity)
        // survives the TOML round trip as Some("") — distinct from None.
        let empty = StoredContext {
            base: Some(String::new()),
            ..Default::default()
        };
        create("bare", empty.clone(), true).unwrap();
        assert_eq!(active(None).unwrap(), Some(empty));

        unsafe { std::env::remove_var("ZENCTL_CONFIG_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }
}
