//! The bus ladders — zenctl's (`zenctl/src/resolve.rs`), with the config
//! file where zenctl has flags — and the **two sessions** a zk2 tool runs
//! on (decided 2026-10-08; #614, FL2).
//!
//! Every knob resolves **flag > config-file `bus.*` > env > active
//! context > default**, as pure functions over scalars and a
//! [`StoredContext`], so no test has to touch `~/.config`. [`resolve`] is
//! the one impure edge: it
//! reads the explorer config once, and a `--context` (or `bus.context`)
//! that names nothing is refused there, before anything is opened (#209's
//! lesson, kept).
//!
//! **Two sessions.** [`BusSettings::sessions`] opens one **in** the
//! deployment's namespace, for what reads the deployment as its own
//! consumers do — presence, descriptors, contracts, the doctor, state — and
//! one in **no** namespace, for the raw rules over wire selectors and for
//! the routers' admin space, as zenctl's raw observers keep them. The
//! namespace climbs the base's ladder, because it is the same fact: the
//! deployment base *is* the session namespace its services run in.
//!
//! What zenwatch does **not** have: an `OutputArgs`. It renders nothing to a
//! terminal that a format flag would apply to — its output is the sinks.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use zenkey_explorer_config::StoredContext;
use zenkey_fleet::DoctorBus;

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
    /// The deployment namespace — empty is the bus-root deployment, and
    /// sets none.
    pub namespace: String,
    pub transport: Transport,
    pub timeout: Duration,
}

/// The deployment namespace: `--namespace` > config > `ZENWATCH_BASE` >
/// the context's `base` > empty.
///
/// An explicitly empty namespace is a deployment — the bus root — and must
/// not fall through to the context.
pub fn namespace(
    flag: Option<&str>,
    cfg: Option<&str>,
    env: Option<&str>,
    stored: Option<&StoredContext>,
) -> String {
    flag.or(cfg)
        .or(env)
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

/// The environment rungs, passed in so the environment is an argument, not
/// a hidden rung.
#[derive(Debug, Clone, Copy, Default)]
pub struct Env<'e> {
    pub base: Option<&'e str>,
    pub zenoh_config: Option<&'e Path>,
}

/// The pure half: every ladder over the flag, the config section and a
/// caller-supplied context. A context's `registry` is not read: zk2 reads
/// contracts (`contracts`, `--contracts`), as zenctl does.
pub fn resolve_with(
    cfg: &BusConfig,
    namespace_flag: Option<&str>,
    env: Env<'_>,
    stored: Option<&StoredContext>,
) -> BusSettings {
    BusSettings {
        namespace: namespace(namespace_flag, cfg.namespace.as_deref(), env.base, stored),
        transport: Transport {
            file: zenoh_config(cfg.zenoh_config.as_deref(), env.zenoh_config, stored),
            connect: endpoints(&cfg.connect, stored.map(|c| c.connect.as_slice())),
            listen: endpoints(&cfg.listen, stored.map(|c| c.listen.as_slice())),
            scouting: scouting(cfg.scouting, stored),
        },
        timeout: timeout(cfg.timeout_s, stored),
    }
}

/// The impure edge: read the explorer config once, resolve the context —
/// `--context` flag > `bus.context` > `ZENKEY_EXPLORER_CONTEXT` > the
/// file's `current` — and climb every ladder. A named context that does not
/// exist is refused (exit 2) here, before anything opens, and so is a
/// namespace the session would refuse.
pub fn resolve(
    cfg: &BusConfig,
    context_flag: Option<&str>,
    namespace_flag: Option<&str>,
) -> Result<BusSettings> {
    let name = context_flag.or(cfg.context.as_deref());
    let stored = zenkey_explorer_config::active(name).map_err(|e| unaskable!("{e}"))?;
    let env_base = std::env::var("ZENWATCH_BASE").ok();
    let env_zc = std::env::var_os("ZENWATCH_ZENOH_CONFIG").map(PathBuf::from);
    let settings = resolve_with(
        cfg,
        namespace_flag,
        Env {
            base: env_base.as_deref(),
            zenoh_config: env_zc.as_deref(),
        },
        stored.as_ref(),
    );
    crate::config::check_namespace(&settings.namespace).map_err(|e| unaskable!("{e}"))?;
    Ok(settings)
}

/// Which watch sessions a run's rules need (#830, core §8.1, 0.37): a
/// session holds a live reader's liveliness subscribers, or makes presence
/// reads, never both, since a liveliness GET beside a liveliness subscriber
/// can complete without a token that is live. Both run in **no** namespace
/// (#875), their subscribers without history.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Watches {
    /// For the namespace, on its wire keys: the presence projection's
    /// monitor (`instance-gone`, `liveliness-gone` on the deployment's own
    /// zk2 tokens), and the owners' instance tokens of `alerts` and
    /// `alert-firing`.
    pub namespaced: bool,
    /// For the bus root, on the wire keys as they are: the foreign-token
    /// ledger's monitor (`liveliness-gone` on any other token).
    pub raw: bool,
}

/// The daemon's sessions: the fleet's [`DoctorBus`] — in the namespace, in
/// none, and the deployment's watch session when a rule needs one — and the
/// raw watch session (#830, #875).
pub struct Sessions {
    pub bus: DoctorBus,
    pub raw_watch: Option<zenkey_fleet::WatchSession>,
}

impl BusSettings {
    /// Open the sessions, saying which half failed: a config file the user
    /// named is theirs to fix (a refusal, 2); a transport that would not
    /// come up is the world being unavailable (1). The fork is
    /// `zenkey_fleet::OpenFailure`'s (#196); this is where it is spent. The
    /// raw session opens first, so a dead transport is said once.
    ///
    /// The two the daemon reads through always open; each watch session
    /// only when `watches` says a rule follows presence live through it
    /// (#830): one more session, with its router-side declarations, per
    /// kind of live reader, and none that no rule needs. Each watch session
    /// is in no namespace, on the wire keys (#875): the deployment's with
    /// its namespace in front, the raw one's as they are.
    pub async fn sessions(&self, watches: Watches) -> Result<Sessions> {
        let t = &self.transport;
        let raw = spend(
            zenkey_fleet::open_reporting(t.file.as_deref(), &t.connect, &t.listen, t.scouting)
                .await,
        )?;
        let namespaced = || async {
            spend(
                zenkey_fleet::open_in_namespace(
                    &self.namespace,
                    t.file.as_deref(),
                    &t.connect,
                    &t.listen,
                    t.scouting,
                )
                .await,
            )
        };
        let session = namespaced().await?;
        let watch_for = |namespace: &'_ str| {
            let namespace = namespace.to_owned();
            async move {
                spend(
                    zenkey_fleet::open_watch(
                        &namespace,
                        t.file.as_deref(),
                        &t.connect,
                        &t.listen,
                        t.scouting,
                    )
                    .await,
                )
            }
        };
        let watch = match watches.namespaced {
            true => Some(watch_for(&self.namespace).await?),
            false => None,
        };
        let raw_watch = match watches.raw {
            true => Some(watch_for("").await?),
            false => None,
        };
        Ok(Sessions {
            bus: DoctorBus {
                session,
                raw,
                namespace: self.namespace.clone(),
                watch,
            },
            raw_watch,
        })
    }
}

/// An open's outcome, with the half that failed turned into an exit code.
fn spend<T>(opened: std::result::Result<T, zenkey_fleet::OpenFailure>) -> Result<T> {
    match opened {
        Ok(s) => Ok(s),
        Err(zenkey_fleet::OpenFailure::Config(e)) => {
            Err(unaskable!("{}", zenkey_fleet::one_line(&e)))
        }
        Err(other) => Err(other.into_error().into()),
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

    /// The scalar ladders, each rung reachable: flag > config > env >
    /// context > default, with an empty namespace staying empty.
    #[test]
    fn the_ladders_climb_flag_config_env_context_default() {
        let c = ctx();
        assert_eq!(
            namespace(Some("flag"), Some("cfg"), Some("env"), Some(&c)),
            "flag"
        );
        assert_eq!(namespace(None, Some(""), Some("env"), Some(&c)), "");
        assert_eq!(namespace(None, None, Some("env"), Some(&c)), "env");
        assert_eq!(namespace(None, None, None, Some(&c)), "prod");
        assert_eq!(namespace(None, None, None, None), "");
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
        assert_eq!(
            zenoh_config(None, Some(Path::new("/env.json5")), Some(&c)),
            Some(PathBuf::from("/env.json5"))
        );
    }

    /// The whole climb, with nothing given: the bus-root deployment, five
    /// seconds — a configuration, not a failure. A context's `registry` is
    /// not read.
    #[test]
    fn nothing_given_falls_to_the_defaults() {
        let s = resolve_with(&BusConfig::default(), None, Env::default(), None);
        assert_eq!(s.namespace, "");
        assert_eq!(s.timeout, Duration::from_secs(5));
        assert_eq!(
            s.transport,
            Transport {
                file: None,
                connect: vec![],
                listen: vec![],
                scouting: None
            }
        );
        let from_ctx = resolve_with(&BusConfig::default(), None, Env::default(), Some(&ctx()));
        assert_eq!(from_ctx.namespace, "prod", "the context's base");
    }
}
