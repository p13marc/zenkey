//! Named connection contexts — the store shared by every explorer (issue #35).
//!
//! Born in zenctl (the nats-CLI model, issue #12) and moved into the engine so
//! zenctl and zengui resolve the same named contexts from the same file. One
//! fleet, two explorers, one config.
//!
//! ```toml
//! current = "lab"
//!
//! [context.lab]
//! base = "zensight"
//! connect = ["tcp/127.0.0.1:7447"]
//! # listen = [], scouting = false, timeout = 5, registry = ["/abs/registry"]
//! ```
//!
//! **Path policy.** The explorer-neutral home is
//! `~/.config/zenkey-explorer/config.toml`. Reads fall back to the legacy
//! `~/.config/zenctl/config.toml` when the neutral file does not exist, so an
//! existing zenctl setup keeps working; writes always go to the neutral path
//! (a one-way migration — the legacy file is left untouched, never deleted).
//! `ZENKEY_EXPLORER_CONFIG_DIR` (or the legacy `ZENCTL_CONFIG_DIR`) overrides
//! the directory outright (tests, multi-config setups) — an override names
//! *the* directory: no fallback chain applies.
//!
//! **Everything here is pure and fallible.** No process-global caches, no
//! `exit()`: a GUI must render a bad config as a banner and keep its window;
//! zenctl turns the `Err` into its own exit code at its own edge.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
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
    /// Path to a zenoh JSON5 config file (#122) — the passthrough that makes
    /// a secured bus (TLS/QUIC/usrpwd) reachable. The explorer's own knobs
    /// apply on top: flag > env > context > file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zenoh_config: Option<PathBuf>,
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

/// The path writes go to (and reads prefer).
pub fn config_path() -> PathBuf {
    if let Some(dir) = override_dir() {
        return dir.join("config.toml");
    }
    neutral_dir().join("config.toml")
}

/// The legacy read-fallback path, when no override is in force.
fn legacy_path() -> Option<PathBuf> {
    if override_dir().is_some() {
        return None;
    }
    dirs::config_dir().map(|d| d.join("zenctl").join("config.toml"))
}

fn override_dir() -> Option<PathBuf> {
    std::env::var_os("ZENKEY_EXPLORER_CONFIG_DIR")
        .or_else(|| std::env::var_os("ZENCTL_CONFIG_DIR"))
        .map(PathBuf::from)
}

fn neutral_dir() -> PathBuf {
    dirs::config_dir()
        .map(|d| d.join("zenkey-explorer"))
        .unwrap_or_else(|| PathBuf::from(".zenkey-explorer"))
}

/// Load the config: the neutral path, else the legacy zenctl path, else empty.
/// A *malformed* file is an error at whichever path supplied it — silently
/// treating a broken config as absent would make edits mysteriously vanish.
pub fn load() -> Result<ConfigFile> {
    for path in [Some(config_path()), legacy_path()].into_iter().flatten() {
        match std::fs::read_to_string(&path) {
            Ok(src) => {
                return toml::from_str(&src)
                    .with_context(|| format!("bad config file {}", path.display()));
            }
            Err(_) => continue,
        }
    }
    Ok(ConfigFile::default())
}

/// Where a context's cached slices live (issue #54).
///
/// Keyed by context name so two deployments cannot complete each other's
/// producers, and `"default"` for an invocation with no named context —
/// which is a real configuration (flags only), not an absence.
///
/// The cache is a *convenience*, never a source of truth: it feeds shell
/// completion and nothing else reads it without saying so. That is why it
/// sits under the cache dir, where an OS is free to delete it.
pub fn cache_dir(context: Option<&str>) -> PathBuf {
    let root = override_dir()
        .map(|d| d.join("cache"))
        .or_else(|| dirs::cache_dir().map(|d| d.join("zenkey-explorer")))
        .unwrap_or_else(|| PathBuf::from(".zenkey-explorer-cache"));
    root.join(context.unwrap_or("default")).join("slices")
}

/// The name of the context this invocation resolves to — the cache key. The
/// same precedence [`active`] uses, minus the lookup, so a completion can find
/// the cache without loading (or failing on) the config file.
pub fn active_name(explicit: Option<&str>) -> Option<String> {
    if let Some(name) = explicit {
        return Some(name.to_string());
    }
    if let Ok(name) =
        std::env::var("ZENKEY_EXPLORER_CONTEXT").or_else(|_| std::env::var("ZENCTL_CONTEXT"))
    {
        return Some(name);
    }
    load().ok().and_then(|c| c.current)
}

/// Save to the neutral path (creating the directory), never to the legacy one.
pub fn save(config: &ConfigFile) -> Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    let rendered = toml::to_string_pretty(config).context("config serializes")?;
    std::fs::write(&path, rendered).with_context(|| format!("cannot write {}", path.display()))
}

/// The context the current invocation should use: `explicit` by name, else
/// `ZENKEY_EXPLORER_CONTEXT`/`ZENCTL_CONTEXT`, else the file's `current`
/// pointer, else nothing.
///
/// An explicitly named context that does not exist is an error (the user
/// asked for something specific); a dangling `current` pointer is a stale
/// file, not a hard error.
pub fn active(explicit: Option<&str>) -> Result<Option<StoredContext>> {
    let config = load()?;
    let env_named = std::env::var("ZENKEY_EXPLORER_CONTEXT")
        .or_else(|_| std::env::var("ZENCTL_CONTEXT"))
        .ok();
    let was_named = explicit.is_some() || env_named.is_some();
    let name = explicit
        .map(str::to_string)
        .or(env_named)
        .or(config.current.clone());
    let Some(name) = name else { return Ok(None) };
    match config.contexts.get(&name) {
        Some(c) => Ok(Some(c.clone())),
        None if was_named => {
            bail!(
                "context {name:?} not found in {} — `zenctl context list`",
                config_path().display()
            )
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize a config through the store and read it back — the format is
    /// the contract both explorers share.
    #[test]
    fn config_round_trips_through_toml() {
        let mut cfg = ConfigFile {
            current: Some("lab".into()),
            ..Default::default()
        };
        cfg.contexts.insert(
            "lab".into(),
            StoredContext {
                base: Some("zensight".into()),
                connect: vec!["tcp/127.0.0.1:7447".into()],
                listen: vec![],
                registry: vec![PathBuf::from("/tmp/reg")],
                scouting: Some(false),
                timeout: Some(5),
                zenoh_config: None,
            },
        );
        let rendered = toml::to_string_pretty(&cfg).unwrap();
        let back: ConfigFile = toml::from_str(&rendered).unwrap();
        assert_eq!(back.current.as_deref(), Some("lab"));
        assert_eq!(back.contexts["lab"], cfg.contexts["lab"]);
    }

    /// The legacy zenctl file format parses unchanged — the migration is a
    /// path change, not a format change.
    #[test]
    fn legacy_zenctl_files_parse_unchanged() {
        let legacy = r#"
current = "lab"

[context.lab]
base = "zensight"
connect = ["tcp/127.0.0.1:7447"]
"#;
        let cfg: ConfigFile = toml::from_str(legacy).unwrap();
        assert_eq!(cfg.contexts["lab"].base.as_deref(), Some("zensight"));
    }

    /// An unknown field is a spelling mistake surfaced, not silently dropped.
    #[test]
    fn unknown_context_fields_are_rejected() {
        let bad = r#"
[context.lab]
bse = "typo"
"#;
        assert!(toml::from_str::<ConfigFile>(bad).is_err());
    }
}
