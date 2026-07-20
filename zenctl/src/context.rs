//! Named connection contexts (issue #12) — the nats-CLI model.
//!
//! `~/.config/zenctl/config.toml` holds named contexts (base, endpoints,
//! registry dirs, scouting, timeout) and a `current` pointer, so `--base` and
//! `-c` stop being required on every invocation. Flags always override
//! context values; `ZENCTL_CONFIG_DIR` overrides the directory (tests,
//! multi-config setups).
//!
//! ```toml
//! current = "lab"
//!
//! [context.lab]
//! base = "zensight"
//! connect = ["tcp/127.0.0.1:7447"]
//! # listen = [], scouting = false, timeout = 5, registry = ["/abs/registry"]
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// One named context's stored settings. All optional: a context only pins
/// what it pins; flags fill the rest.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoredContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connect: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listen: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scouting: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

/// The whole config file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(
        default,
        rename = "context",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub contexts: BTreeMap<String, StoredContext>,
}

pub fn config_path() -> PathBuf {
    let dir = std::env::var_os("ZENCTL_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::config_dir().map(|d| d.join("zenctl")))
        .unwrap_or_else(|| PathBuf::from(".zenctl"));
    dir.join("config.toml")
}

pub fn load() -> Result<ConfigFile> {
    let path = config_path();
    let Ok(src) = std::fs::read_to_string(&path) else {
        return Ok(ConfigFile::default());
    };
    toml::from_str(&src).with_context(|| format!("bad config file {}", path.display()))
}

pub fn save(config: &ConfigFile) -> Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let rendered = toml::to_string_pretty(config).context("config serializes")?;
    std::fs::write(&path, rendered).with_context(|| format!("cannot write {}", path.display()))
}

/// The context the current invocation should use: `--context`/`ZENCTL_CONTEXT`
/// by name, else the file's `current` pointer, else nothing.
pub fn active(explicit: Option<&str>) -> Result<Option<StoredContext>> {
    let config = load()?;
    let name = explicit
        .map(str::to_string)
        .or_else(|| std::env::var("ZENCTL_CONTEXT").ok())
        .or(config.current.clone());
    let Some(name) = name else { return Ok(None) };
    match config.contexts.get(&name) {
        Some(c) => Ok(Some(c.clone())),
        None if explicit.is_some() || std::env::var("ZENCTL_CONTEXT").is_ok() => {
            bail!(
                "context {name:?} not found in {} — `zenctl context list`",
                config_path().display()
            )
        }
        // A dangling `current` pointer is a stale file, not a hard error.
        None => Ok(None),
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
