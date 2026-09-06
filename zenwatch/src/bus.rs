//! The bus ladders — zenctl's (`zenctl/src/resolve.rs`), with the config
//! file where zenctl has flags.
//!
//! Every knob resolves **config-file `bus.*` > env > active context >
//! default**, as pure functions over scalars and a
//! [`StoredContext`], so no test has to touch `~/.config`. [`resolve`] is the
//! one impure edge: it reads the explorer config once, and a `--context`
//! (or `bus.context`) that names nothing is refused there, before anything
//! is opened (#209's lesson, kept).
//!
//! What zenwatch does **not** have: an `OutputArgs`. It renders nothing to a
//! terminal that a format flag would apply to — its output is the sinks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use zenkey_explorer_config::StoredContext;

use crate::config::BusConfig;
use crate::exit::unaskable;

/// The default reply/watch window, in seconds — the same as zenctl's.
pub const DEFAULT_TIMEOUT_S: u64 = 5;

/// Everything `zenkey_fleet::open_reporting` needs, climbed once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    pub file: Option<PathBuf>,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    pub scouting: Option<bool>,
}

/// Everything the engine needs from the bus section, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusSettings {
    /// The deployment base — empty is the bus-root deployment (RFC v1.6).
    pub base: String,
    pub transport: Transport,
    pub timeout: Duration,
    /// Registry directories (offline slices), if any.
    pub registry: Vec<PathBuf>,
}

/// The deployment base: config > `ZENWATCH_BASE` > context > empty.
///
/// An explicitly empty base is a deployment — the bus root — and must not
/// fall through to the context.
pub fn base(cfg: Option<&str>, env: Option<&str>, stored: Option<&StoredContext>) -> String {
    cfg.or(env)
        .map(str::to_string)
        .unwrap_or_else(|| stored.and_then(|c| c.base.clone()).unwrap_or_default())
}

/// Endpoints: a non-empty config list **replaces** the context's, never
/// merges with it — a notifier that quietly joins a mesh the config did not
/// name is the failure scouting is off by default to avoid.
pub fn endpoints(cfg: &[String], stored: Option<&[String]>) -> Vec<String> {
    if cfg.is_empty() {
        stored.map(<[String]>::to_vec).unwrap_or_default()
    } else {
        cfg.to_vec()
    }
}

/// Scouting: three states, not two — an unstated config value leaves a
/// `zenoh_config` file's own choice alone (#122).
pub fn scouting(cfg: Option<bool>, stored: Option<&StoredContext>) -> Option<bool> {
    cfg.or_else(|| stored.and_then(|c| c.scouting))
}

/// The reply/watch window: config > context > [`DEFAULT_TIMEOUT_S`]. No env
/// rung, as in zenctl: a timeout is per-question, not per-shell.
pub fn timeout(cfg: Option<u64>, stored: Option<&StoredContext>) -> Duration {
    Duration::from_secs(
        cfg.or_else(|| stored.and_then(|c| c.timeout))
            .unwrap_or(DEFAULT_TIMEOUT_S),
    )
}

/// Registry directories: config replaces context, on the endpoints rule.
pub fn registry_dirs(cfg: &[PathBuf], stored: Option<&StoredContext>) -> Vec<PathBuf> {
    if cfg.is_empty() {
        stored.map(|c| c.registry.clone()).unwrap_or_default()
    } else {
        cfg.to_vec()
    }
}

/// The zenoh JSON5 config file: config > `ZENWATCH_ZENOH_CONFIG` > context.
pub fn zenoh_config(
    cfg: Option<&Path>,
    env: Option<&Path>,
    stored: Option<&StoredContext>,
) -> Option<PathBuf> {
    cfg.or(env)
        .map(Path::to_path_buf)
        .or_else(|| stored.and_then(|c| c.zenoh_config.clone()))
}

/// The pure half: every ladder over the config section and a
/// caller-supplied context. `env_base`/`env_zenoh_config` are passed in so
/// the environment is an argument, not a hidden rung.
pub fn resolve_with(
    cfg: &BusConfig,
    env_base: Option<&str>,
    env_zenoh_config: Option<&Path>,
    stored: Option<&StoredContext>,
) -> BusSettings {
    BusSettings {
        base: base(cfg.base.as_deref(), env_base, stored),
        transport: Transport {
            file: zenoh_config(cfg.zenoh_config.as_deref(), env_zenoh_config, stored),
            connect: endpoints(&cfg.connect, stored.map(|c| c.connect.as_slice())),
            listen: endpoints(&cfg.listen, stored.map(|c| c.listen.as_slice())),
            scouting: scouting(cfg.scouting, stored),
        },
        timeout: timeout(cfg.timeout_s, stored),
        registry: registry_dirs(&cfg.registry, stored),
    }
}

/// The impure edge: read the explorer config once, resolve the context —
/// `--context` flag > `bus.context` > `ZENKEY_EXPLORER_CONTEXT` > the
/// file's `current` — and climb every ladder. A named context that does not
/// exist is refused (exit 2) here, before anything opens.
pub fn resolve(cfg: &BusConfig, context_flag: Option<&str>) -> Result<BusSettings> {
    let name = context_flag.or(cfg.context.as_deref());
    let stored = zenkey_explorer_config::active(name).map_err(|e| unaskable!("{e}"))?;
    let env_base = std::env::var("ZENWATCH_BASE").ok();
    let env_zc = std::env::var_os("ZENWATCH_ZENOH_CONFIG").map(PathBuf::from);
    Ok(resolve_with(
        cfg,
        env_base.as_deref(),
        env_zc.as_deref(),
        stored.as_ref(),
    ))
}

impl BusSettings {
    /// Open the session, saying which half failed: a config file the user
    /// named is theirs to fix (a refusal, 2); a transport that would not
    /// come up is the world being unavailable (1). The fork is
    /// `zenkey_fleet::OpenFailure`'s (#196); this is where it is spent.
    pub async fn session(&self) -> Result<zenoh::Session> {
        let t = &self.transport;
        match zenkey_fleet::open_reporting(t.file.as_deref(), &t.connect, &t.listen, t.scouting)
            .await
        {
            Ok(s) => Ok(s),
            Err(zenkey_fleet::OpenFailure::Config(e)) => {
                Err(unaskable!("{}", zenkey_fleet::one_line(&e)))
            }
            Err(other) => Err(other.into_error().into()),
        }
    }

    /// Registry slices when they are available, `None` when they are not —
    /// zenctl's `slices_optional` (#210), for a tool slices only *enrich*:
    /// with none, an alert document still renders structurally and
    /// `qos-mismatch`/`invalid-payload` say what they could not judge.
    ///
    /// Two failure sources, kept apart: a directory the config **named**
    /// that does not read is a refusal (never degraded past — a typo must not
    /// become a silent structural rendering); a fleet that did not answer is
    /// a degradation, announced once.
    pub async fn slices(&self, session: &zenoh::Session) -> Result<Option<zenkey_fleet::SliceSet>> {
        let fleet = zenkey_fleet::Fleet::new(session, &self.base);
        if self.registry.is_empty() {
            return match zenkey_fleet::SliceSet::from_bus(&fleet, self.timeout).await {
                Ok(set) => {
                    if set.slices().is_empty() {
                        tracing::warn!(
                            base = %self.base,
                            "no introspect slices — an empty set is not a verdict (RFC 05 §3.1); \
                             payloads render structurally"
                        );
                    }
                    Ok(Some(set))
                }
                Err(e) => {
                    tracing::warn!(
                        "registry sweep failed ({}); payloads render structurally",
                        zenkey_fleet::one_line(&e)
                    );
                    Ok(None)
                }
            };
        }
        // Named dirs union with the bus, served winning per producer (RFC 08
        // §6.1); a dir that will not read is the user's error.
        let out = zenkey_fleet::SliceSet::from_union(&fleet, &self.registry, self.timeout)
            .await
            .map_err(|e| unaskable!("{}", zenkey_fleet::one_line(&e)))?;
        for d in &out.disagreements {
            tracing::warn!(
                producer = %d.producer,
                "registry disagreement: bus serves v{}, dirs carry v{} (served wins)",
                d.bus_version,
                d.dirs_version
            );
        }
        Ok(Some(out.set))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> StoredContext {
        StoredContext {
            base: Some("prod".into()),
            connect: vec!["tcp/router:7447".into()],
            listen: vec![],
            registry: vec![PathBuf::from("/ctx/registry")],
            scouting: Some(false),
            zenoh_config: Some(PathBuf::from("/ctx/zenoh.json5")),
            timeout: Some(30),
        }
    }

    /// The four scalar ladders, each rung reachable: config > env > context
    /// > default, with an empty base staying empty.
    #[test]
    fn the_ladders_climb_config_env_context_default() {
        let c = ctx();
        assert_eq!(base(Some(""), Some("env"), Some(&c)), "");
        assert_eq!(base(None, Some("env"), Some(&c)), "env");
        assert_eq!(base(None, None, Some(&c)), "prod");
        assert_eq!(base(None, None, None), "");
        assert_eq!(
            endpoints(&["tcp/localhost:7447".to_string()], Some(&c.connect)),
            vec!["tcp/localhost:7447".to_string()],
            "replace, never merge"
        );
        assert_eq!(endpoints(&[], Some(&c.connect)), c.connect);
        assert_eq!(scouting(None, Some(&c)), Some(false));
        assert_eq!(scouting(Some(true), Some(&c)), Some(true));
        assert_eq!(scouting(None, None), None, "unstated stays unstated");
        assert_eq!(timeout(Some(1), Some(&c)), Duration::from_secs(1));
        assert_eq!(timeout(None, Some(&c)), Duration::from_secs(30));
        assert_eq!(timeout(None, None), Duration::from_secs(DEFAULT_TIMEOUT_S));
        assert_eq!(registry_dirs(&[], Some(&c)), c.registry);
        assert_eq!(
            zenoh_config(None, Some(Path::new("/env.json5")), Some(&c)),
            Some(PathBuf::from("/env.json5"))
        );
    }

    /// The whole climb, with nothing given: the base-less bus root, five
    /// seconds, no dirs — a configuration, not a failure.
    #[test]
    fn nothing_given_falls_to_the_defaults() {
        let s = resolve_with(&BusConfig::default(), None, None, None);
        assert_eq!(s.base, "");
        assert_eq!(s.timeout, Duration::from_secs(5));
        assert!(s.registry.is_empty());
        assert_eq!(
            s.transport,
            Transport {
                file: None,
                connect: vec![],
                listen: vec![],
                scouting: None
            }
        );
    }
}
