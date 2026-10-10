//! The config: one JSON5 file, the whole statement of what this process
//! does.
//!
//! JSON5 because an operator edits it by hand — comments and trailing commas
//! — and because it is the dialect zenoh's own config already speaks, so a
//! fleet has one config grammar, not two. `deny_unknown_fields` everywhere:
//! a misspelt `sinks` key is a rule that silently pages nobody, and the only
//! honest answer to that is a refusal at load.
//!
//! **Secrets are never inline.** A token is `{ env: "NAME" }` or
//! `{ file: "/path" }`; a bare string in a secret position fails to
//! deserialize with those words. The point is not that inline secrets are
//! worse than files — it is that a config which *can* be checked in must not
//! be *able* to carry a credential by accident. [`Secret::resolve`] reads the
//! value; [`check`] proves it readable before anything runs.
//!
//! **What zk2 took away is refused by name, never ignored** (#614, FL2):
//! `bus.registry` (v1's registry slices; zk2 reads `contracts`),
//! `discipline.inhibit` with inhibition on (removed: it read v1's catalog),
//! `group_by: ["origin"]` (v1's origin chunk; zk2 groups by `system` or
//! `address`) and the doctor block's `sample` each parse, so that [`check`]
//! can say where the question went instead of an unknown-field error that
//! says nothing.
//!
//! [`load`] parses; [`check`] refuses. Both are pure over their inputs, so
//! `check-config` and `run` cannot disagree about what a bad config is.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde::de::{self, Error as _, MapAccess, Visitor};

use crate::render::RenderConfig;
use crate::rules::{RuleKind, parse_rule};

/// The whole file.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub bus: BusConfig,
    /// Contracts known offline (spec §8.5): authoring files, directories of
    /// them, or `.history` roots, as `zenctl --contracts` reads them. A
    /// revision held here is never retrieved; every other one a descriptor
    /// names is retrieved from its holders (§8.4). `--contracts` adds to
    /// these. Relative paths are the working directory's.
    #[serde(default)]
    pub contracts: Vec<PathBuf>,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (500 ms), as the deployment keeps them
    /// (`freshness.v1` §2.6): the doctor — scheduled, or a `doctor` rule's —
    /// then ages a `health.v1` status reply's stamp against this clock.
    /// It listens to no status, so without it the `health-*` checks of
    /// every service implementing `health.v1` are unobservable. Default off.
    #[serde(default)]
    pub clocks_synced: bool,
    /// The watchdog's evaluation cadence for the engine rules, seconds.
    #[serde(default = "default_tick")]
    pub tick_s: f64,
    pub rules: Vec<RuleConfig>,
    pub sinks: BTreeMap<String, SinkConfig>,
    #[serde(default)]
    pub render: RenderConfig,
    /// The notification discipline (#389): `for`, dedup, grouping, repeat,
    /// resolved notices.
    #[serde(default)]
    pub discipline: DisciplineConfig,
    /// Where the discipline's ledger survives a restart (JSON, version 2).
    /// Unset: the ledger is in memory only, and a restart re-announces the
    /// world.
    pub state_file: Option<PathBuf>,
    /// How many notice entries the ledger keeps; the least recently changed
    /// are evicted past it, counted and reported (RFC 13 §3 O6).
    #[serde(default = "default_state_max_entries")]
    pub state_max_entries: usize,
    /// The scheduled doctor (#390): zk2's `run_doctor` over the whole
    /// deployment every few **hours**, run-over-run deltas to the named
    /// sinks, the last report published. Absent: not scheduled — nothing
    /// runs, nothing is published, and the health document says so.
    pub doctor: Option<DoctorConfig>,
}

fn default_tick() -> f64 {
    5.0
}

fn default_state_max_entries() -> usize {
    4096
}

/// The `doctor` section (#390): the deployment's conformance checks
/// (`zenctl doctor`'s stable check ids, zk2's since FL2) asked on a
/// schedule, so that a fleet which drifts three weeks after deployment — a
/// provider upgraded on four hosts and not the fifth, a role whose
/// provider went away — is noticed without anyone running the doctor by
/// hand.
///
/// **Hours, not seconds.** A doctor run is a fan-in sweep — two presence
/// reads a grace apart, a descriptor per instance, the bundles they name,
/// the admin space — not a tick. `every_s` exists for tests and demos,
/// where an interval of hours is an interval nobody sees, and is
/// documented as exactly that.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DoctorConfig {
    /// The interval, in hours. Exactly one of `every_h` and `every_s`.
    pub every_h: Option<f64>,
    /// The interval, in seconds — **for tests and demos only**: a sweep
    /// every few seconds is load on the fleet, not observation of it.
    pub every_s: Option<f64>,
    /// The checks that read the owners' data plane too —
    /// `state-stamp-foreign` and `population-over-bound`, a GET of every
    /// owner's state — real query load, opt-in (`zenctl doctor --deep`).
    #[serde(default)]
    pub deep: bool,
    /// v1's per-family sample bound. zk2's deep checks read every owner's
    /// state once and take no sample: refused by [`check`] with that said.
    pub sample: Option<usize>,
    /// Per-read timeout for the sweep; the bus timeout when unsaid.
    pub timeout_s: Option<f64>,
    /// Names into `sinks`; at least one. Every doctor notification — the
    /// baseline, each new finding, each fixed one, a run that could not
    /// happen — goes to these.
    pub sinks: Vec<String>,
    /// `info` | `warning` | `error`: findings below it are in the published
    /// report but are never a notification. `info` (everything) when unsaid.
    pub severity_floor: Option<String>,
}

/// The severity floors a doctor block may name — the doctor's own three,
/// which are the rule severities minus `critical` (a doctor finding is
/// never one).
pub const DOCTOR_FLOORS: [&str; 3] = ["info", "warning", "error"];

impl DoctorConfig {
    /// The interval, when exactly one of the two spellings is given and it
    /// is a positive number. `None` is what [`check`] refuses.
    pub fn every(&self) -> Option<std::time::Duration> {
        let secs = match (self.every_h, self.every_s) {
            (Some(h), None) => h * 3600.0,
            (None, Some(s)) => s,
            _ => return None,
        };
        (secs.is_finite() && secs > 0.0).then(|| std::time::Duration::from_secs_f64(secs))
    }

    /// How the interval was spelled, for a summary line.
    pub fn every_spelled(&self) -> String {
        match (self.every_h, self.every_s) {
            (Some(h), None) => format!("{h}h"),
            (None, Some(s)) => format!("{s}s"),
            _ => "?".into(),
        }
    }

    /// The floor as a rank on [`crate::discipline::severity_rank`]'s scale;
    /// `info` when unsaid.
    pub fn floor_rank(&self) -> u8 {
        crate::discipline::severity_rank(self.severity_floor.as_deref().unwrap_or("info"))
    }
}

/// The `discipline` section (#389): everything between a notice and a
/// delivery that makes a notifier one someone keeps enabled.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DisciplineConfig {
    /// Default `for` window, seconds: a firing notice is announced only if
    /// it is still firing this long after it started. A rule's own `for_s`
    /// overrides it. `0` (or unset) announces at the next tick.
    pub for_s: Option<f64>,
    /// How long notices sharing a group key wait for company before one
    /// notification is sent for all of them. At most 60.
    #[serde(default = "default_group_window")]
    pub group_window_s: f64,
    /// The label set a group key is built from. Two are the notice's own
    /// when it is about an address: `system` (the `<system>` of
    /// `<system>/<service>`, a host under `hostid.v1`) and `address`; any
    /// other is a label a rule carries. Default: by system.
    #[serde(default = "default_group_by")]
    pub group_by: Vec<String>,
    /// Re-send a notice still firing after this many seconds, once per
    /// interval. `0`: never.
    #[serde(default)]
    pub repeat_s: f64,
    /// Inhibition **was removed** (#614, FL2): it read v1's catalog
    /// (`edge/*`, `entity/*`, `alias/*`), and no zk2 profile brings it
    /// back. The block still parses so that `enabled: false` (or no block)
    /// runs, and [`check`] refuses inhibition switched on, saying why.
    pub inhibit: Option<InhibitConfig>,
    /// Send the resolved family (`resolved`, `observable_again`) at all.
    #[serde(default = "default_true")]
    pub resolved_notice: bool,
}

fn default_group_window() -> f64 {
    2.0
}
fn default_group_by() -> Vec<String> {
    vec![GROUP_SYSTEM.into()]
}
fn default_true() -> bool {
    true
}

/// The group label that is a notice's `<system>` (#614, FL2).
pub const GROUP_SYSTEM: &str = "system";
/// The group label that is a notice's `<system>/<service>`.
pub const GROUP_ADDRESS: &str = "address";

impl Default for DisciplineConfig {
    fn default() -> Self {
        DisciplineConfig {
            for_s: None,
            group_window_s: default_group_window(),
            group_by: default_group_by(),
            repeat_s: 0.0,
            inhibit: None,
            resolved_notice: true,
        }
    }
}

/// v1's inhibition block, kept only to be refused when switched on (#614,
/// FL2). In v1 a block without `enabled` was on, so only `enabled: false`
/// is accepted.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InhibitConfig {
    pub enabled: Option<bool>,
    /// The walk bound v1 took; meaningless with inhibition off, and parsed
    /// so an `enabled: false` block that still carries it runs.
    pub depth: Option<usize>,
}

/// Why inhibition is refused: named once, for [`check`] and the docs.
pub const INHIBIT_REMOVED: &str = "inhibition was removed in the zk2 port \
     (#614): it read v1's catalog (edge/*, entity/*, alias/*) to attribute a \
     symptom to a down host, and no zk2 profile carries those documents. \
     Delete the block, or set `enabled: false`";

/// The `bus` section — the same knobs as a zenctl context, and resolved
/// against one ([`crate::bus`]).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BusConfig {
    /// A named explorer context to start from; `--context` overrides it.
    pub context: Option<String>,
    /// The deployment namespace: the resolved reads' session is opened in
    /// it (decided 2026-10-08). `base` is the same field, as `--base` is
    /// zenctl's `--namespace`; `--namespace` overrides it.
    #[serde(alias = "base")]
    pub namespace: Option<String>,
    #[serde(default)]
    pub connect: Vec<String>,
    #[serde(default)]
    pub listen: Vec<String>,
    pub scouting: Option<bool>,
    pub timeout_s: Option<u64>,
    pub zenoh_config: Option<PathBuf>,
    /// v1's registry directories. zk2 reads contracts: refused by [`check`]
    /// with the field that replaced it named.
    pub registry: Option<Vec<PathBuf>>,
}

/// One rule, as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    pub name: String,
    /// One spelling of the closed vocabulary ([`crate::rules::VOCABULARY`]).
    pub rule: String,
    /// `info` | `warning` | `error` | `critical`; `warning` when unsaid.
    pub severity: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Names into `sinks`; at least one.
    pub sinks: Vec<String>,
    /// This rule's `for` window, seconds — overrides `discipline.for_s`.
    pub for_s: Option<f64>,
}

/// The severity vocabulary a rule may declare — closed, like the rules.
pub const SEVERITIES: [&str; 4] = ["info", "warning", "error", "critical"];

/// A credential, by reference — never by value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Secret {
    /// The name of an environment variable.
    Env(String),
    /// A file whose contents (trailing newline trimmed) are the value.
    File(PathBuf),
}

/// The sentence a bare-string secret fails with.
pub const INLINE_SECRET: &str =
    "secrets are never inline — write { env: \"NAME\" } or { file: \"/path\" }";

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Secret;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("{ env: \"NAME\" } or { file: \"/path\" }")
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Secret, E> {
                Err(E::custom(INLINE_SECRET))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Secret, A::Error> {
                let mut out: Option<Secret> = None;
                while let Some(key) = m.next_key::<String>()? {
                    let next = match key.as_str() {
                        "env" => Secret::Env(m.next_value()?),
                        "file" => Secret::File(m.next_value()?),
                        other => return Err(A::Error::unknown_field(other, &["env", "file"])),
                    };
                    if out.is_some() {
                        return Err(A::Error::custom(
                            "a secret is one of `env` or `file`, not both",
                        ));
                    }
                    out = Some(next);
                }
                out.ok_or_else(|| A::Error::custom("a secret needs `env` or `file`"))
            }
        }
        d.deserialize_any(V)
    }
}

/// Why a secret could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("environment variable {0} is not set")]
    EnvUnset(String),
    #[error("environment variable {0} is not valid UTF-8")]
    EnvNotUtf8(String),
    #[error("cannot read secret file {0}: {1}")]
    File(PathBuf, String),
}

impl Secret {
    /// Read the value now. An env var must be set; a file must read, and its
    /// trailing newline is not part of the secret.
    pub fn resolve(&self) -> Result<String, SecretError> {
        match self {
            Secret::Env(name) => match std::env::var(name) {
                Ok(v) => Ok(v),
                Err(std::env::VarError::NotPresent) => Err(SecretError::EnvUnset(name.clone())),
                Err(std::env::VarError::NotUnicode(_)) => {
                    Err(SecretError::EnvNotUtf8(name.clone()))
                }
            },
            Secret::File(path) => std::fs::read_to_string(path)
                .map(|s| s.trim_end_matches(['\n', '\r']).to_string())
                .map_err(|e| SecretError::File(path.clone(), e.to_string())),
        }
    }

    /// How the secret is spelled, for a problem message — never its value.
    pub fn describe(&self) -> String {
        match self {
            Secret::Env(n) => format!("env {n}"),
            Secret::File(p) => format!("file {}", p.display()),
        }
    }
}

/// A webhook header value: a plain string, or a secret by reference. The
/// plain form is for `Content-Type` and its kind; a credential-shaped header
/// name with a plain value is refused by [`check`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum HeaderValue {
    Plain(String),
    Secret(Secret),
}

impl HeaderValue {
    pub fn resolve(&self) -> Result<String, SecretError> {
        match self {
            HeaderValue::Plain(s) => Ok(s.clone()),
            HeaderValue::Secret(s) => s.resolve(),
        }
    }
}

/// SMTP transport security.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SmtpTls {
    /// Implicit TLS from the first byte (port 465).
    Tls,
    /// Plain connect, then STARTTLS (port 587) — the common relay shape.
    Starttls,
    /// No TLS at all. Named `dangerous` so the config says what it does.
    Dangerous,
}

/// One sink. `kind` selects the implementation; everything else is that
/// kind's own vocabulary, unknown fields refused.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum SinkConfig {
    /// `POST {url}/{topic}` with the title, a priority mapped from severity,
    /// and tags — a phone that buzzes.
    Ntfy {
        url: String,
        topic: String,
        token: Option<Secret>,
        /// Severity word → ntfy priority 1–5. Keys: the rule severities,
        /// plus `resolved` (a notification whose state is `ok`) and
        /// `unobservable`. Unlisted keys take [`crate::sinks::ntfy::DEFAULT_PRIORITY`].
        #[serde(default)]
        priority: BTreeMap<String, u8>,
        #[serde(default)]
        tags: Vec<String>,
        timeout_s: Option<f64>,
    },
    /// The [`crate::sinks::Outgoing`] as a JSON body; 2xx is delivered.
    Webhook {
        url: String,
        #[serde(default = "default_method")]
        method: String,
        #[serde(default)]
        headers: BTreeMap<String, HeaderValue>,
        timeout_s: Option<f64>,
    },
    /// A program, with the [`crate::sinks::Outgoing`] JSON on stdin. The one
    /// sink that runs code — deliberately last in every list.
    Exec {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        timeout_s: Option<f64>,
    },
    /// Mail through a relay.
    Smtp {
        host: String,
        #[serde(default = "default_smtp_port")]
        port: u16,
        #[serde(default = "default_smtp_tls")]
        tls: SmtpTls,
        username: Option<String>,
        password: Option<Secret>,
        from: String,
        to: Vec<String>,
        #[serde(default = "default_subject_prefix")]
        subject_prefix: String,
        timeout_s: Option<f64>,
    },
}

fn default_method() -> String {
    "POST".into()
}
fn default_smtp_port() -> u16 {
    587
}
fn default_smtp_tls() -> SmtpTls {
    SmtpTls::Starttls
}
fn default_subject_prefix() -> String {
    "[zenwatch]".into()
}

impl SinkConfig {
    /// The `kind` word, as the config spells it.
    pub fn kind(&self) -> &'static str {
        match self {
            SinkConfig::Ntfy { .. } => "ntfy",
            SinkConfig::Webhook { .. } => "webhook",
            SinkConfig::Exec { .. } => "exec",
            SinkConfig::Smtp { .. } => "smtp",
        }
    }

    /// The per-delivery bound: the sink's own `timeout_s`, else 10 s (30 s
    /// for `exec`, which may page a human on the way).
    pub fn timeout(&self) -> std::time::Duration {
        let (given, default) = match self {
            SinkConfig::Ntfy { timeout_s, .. }
            | SinkConfig::Webhook { timeout_s, .. }
            | SinkConfig::Smtp { timeout_s, .. } => (*timeout_s, 10.0),
            SinkConfig::Exec { timeout_s, .. } => (*timeout_s, 30.0),
        };
        std::time::Duration::from_secs_f64(given.unwrap_or(default))
    }
}

/// One thing [`check`] refuses: where in the file, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProblem {
    /// A path into the document — `rules[2].rule`, `sinks.ops.token`.
    pub path: String,
    pub message: String,
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Why a file did not become a [`Config`].
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("cannot read {0}: {1}")]
    Read(PathBuf, std::io::Error),
    #[error("{0}: {1}")]
    Parse(PathBuf, json5::Error),
}

/// Read and parse. Parsing is the first refusal — an unknown field, an
/// inline secret, a wrong type — and the error names the position.
pub fn load(path: &Path) -> Result<Config, LoadError> {
    let text = std::fs::read_to_string(path).map_err(|e| LoadError::Read(path.to_path_buf(), e))?;
    parse(&text).map_err(|e| LoadError::Parse(path.to_path_buf(), e))
}

/// Parse from text — the same deserialization `load` runs, for tests.
pub fn parse(text: &str) -> Result<Config, json5::Error> {
    json5::from_str(text)
}

/// Everything `run` would refuse that the parser cannot: each returned
/// problem is one reason for exit 2. Empty means the config is runnable.
///
/// Secrets are **resolved here** (and their values discarded): an unset env
/// var or an unreadable file is a config problem at check time, not a
/// delivery failure at 3am. The contracts and the state file are read by
/// [`crate::load_checked`], which knows the flags.
pub fn check(cfg: &Config) -> Vec<ConfigProblem> {
    let mut out = Vec::new();
    let mut problem = |path: String, message: String| out.push(ConfigProblem { path, message });

    if !cfg.tick_s.is_finite() || cfg.tick_s <= 0.0 {
        problem(
            "tick_s".into(),
            format!("must be a positive number of seconds, got {}", cfg.tick_s),
        );
    }
    if cfg.render.max_message_bytes == 0 || cfg.render.max_structural_bytes == 0 {
        problem("render".into(), "the byte bounds must be positive".into());
    }
    if cfg.rules.is_empty() {
        problem("rules".into(), "no rules — nothing to watch".into());
    }
    if cfg.sinks.is_empty() {
        problem("sinks".into(), "no sinks — nowhere to notify".into());
    }
    if cfg.bus.registry.is_some() {
        problem(
            "bus.registry".into(),
            "v1's registry slices are gone: zk2 reads contracts — `contracts: [<PATH>]` \
             (an authoring file, a directory of them, or a .history root) or `--contracts`; \
             a revision held offline is never retrieved, every other one comes from the bus \
             (spec §8.4)"
                .into(),
        );
    }
    if let Some(ns) = &cfg.bus.namespace
        && let Err(e) = check_namespace(ns)
    {
        problem("bus.namespace".into(), e);
    }
    check_discipline(&mut problem, cfg);
    check_doctor(&mut problem, cfg);

    // Rules: parse, closed severities, every sink named exists, names and
    // ids unique, and no two rules spelling one engine condition — the
    // engine reports a transition once per condition, and two rules on it
    // could not be told apart by their `rule` string.
    let mut names: BTreeMap<&str, usize> = BTreeMap::new();
    let mut ids: BTreeMap<String, usize> = BTreeMap::new();
    let mut conditions: BTreeMap<String, usize> = BTreeMap::new();
    for (i, r) in cfg.rules.iter().enumerate() {
        let at = |field: &str| format!("rules[{i}].{field}");
        if r.name.trim().is_empty() {
            problem(at("name"), "a rule needs a name".into());
        }
        if let Some(prev) = names.insert(&r.name, i) {
            problem(at("name"), format!("duplicates rules[{prev}].name"));
        }
        let id = crate::rules::rule_id(&r.name);
        if let Some(prev) = ids.insert(id.clone(), i) {
            problem(
                at("name"),
                format!("slugs to {id:?}, the same id as rules[{prev}]"),
            );
        }
        if cfg.doctor.is_some() && id == crate::rules::DOCTOR_RULE {
            problem(
                at("name"),
                format!(
                    "slugs to {id:?}, the id the scheduled doctor publishes and notifies \
                     under (`doctor` block present) — name the rule something else"
                ),
            );
        }
        match parse_rule(&r.rule) {
            Err(e) => problem(at("rule"), e.to_string()),
            Ok(RuleKind::Engine(c)) => {
                let spelled = c.to_string();
                if let Some(prev) = conditions.insert(spelled.clone(), i) {
                    problem(
                        at("rule"),
                        format!(
                            "duplicates rules[{prev}]'s condition {spelled:?} — the engine \
                             reports one transition per condition; give the second rule a \
                             different condition or merge the sinks"
                        ),
                    );
                }
            }
            Ok(_) => {}
        }
        if let Some(sev) = &r.severity
            && !SEVERITIES.contains(&sev.as_str())
        {
            problem(
                at("severity"),
                format!(
                    "{sev:?} is not a severity — one of {}",
                    SEVERITIES.join(", ")
                ),
            );
        }
        if r.sinks.is_empty() {
            problem(at("sinks"), "a rule needs at least one sink".into());
        }
        if let Some(f) = r.for_s {
            if !(f.is_finite() && f >= 0.0) {
                problem(
                    at("for_s"),
                    "must be a non-negative number of seconds".into(),
                );
            } else if cfg.discipline.repeat_s > 0.0 && f >= cfg.discipline.repeat_s {
                problem(
                    at("for_s"),
                    format!(
                        "{f}s is not below discipline.repeat_s ({}s) — a repeat that fires \
                         before the first announcement is not a repeat",
                        cfg.discipline.repeat_s
                    ),
                );
            }
        }
        for (j, s) in r.sinks.iter().enumerate() {
            if !cfg.sinks.contains_key(s) {
                problem(
                    format!("rules[{i}].sinks[{j}]"),
                    format!(
                        "{s:?} is not in `sinks` (which has: {})",
                        cfg.sinks.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                );
            }
        }
    }

    // Sinks: shape, bounds, and every secret readable now.
    for (name, sink) in &cfg.sinks {
        let at = |field: &str| format!("sinks.{name}.{field}");
        if let Some(t) = sink_timeout_s(sink)
            && !(t.is_finite() && t > 0.0)
        {
            problem(
                at("timeout_s"),
                "must be a positive number of seconds".into(),
            );
        }
        match sink {
            SinkConfig::Ntfy {
                url,
                topic,
                token,
                priority,
                ..
            } => {
                check_url(&mut problem, &at("url"), url);
                if topic.is_empty() || topic.contains('/') {
                    problem(at("topic"), "a topic is one path segment".into());
                }
                for (k, v) in priority {
                    if !(1..=5).contains(v) {
                        problem(
                            format!("sinks.{name}.priority.{k}"),
                            format!("ntfy priorities are 1–5, got {v}"),
                        );
                    }
                }
                check_secret(&mut problem, &at("token"), token.as_ref());
            }
            SinkConfig::Webhook {
                url,
                method,
                headers,
                ..
            } => {
                check_url(&mut problem, &at("url"), url);
                if reqwest::Method::from_bytes(method.as_bytes()).is_err()
                    || !method.bytes().all(|b| b.is_ascii_uppercase())
                {
                    problem(at("method"), format!("{method:?} is not an HTTP method"));
                }
                for (h, v) in headers {
                    let hat = format!("sinks.{name}.headers.{h}");
                    if reqwest::header::HeaderName::from_bytes(h.as_bytes()).is_err() {
                        problem(hat.clone(), "not a header name".into());
                    }
                    match v {
                        HeaderValue::Plain(_) if credential_shaped(h) => problem(
                            hat,
                            "a credential-shaped header takes a secret ({ env: … } or \
                             { file: … }), never an inline value"
                                .into(),
                        ),
                        HeaderValue::Plain(p) => {
                            if reqwest::header::HeaderValue::from_str(p).is_err() {
                                problem(hat, "not a header value".into());
                            }
                        }
                        HeaderValue::Secret(s) => check_secret(&mut problem, &hat, Some(s)),
                    }
                }
            }
            SinkConfig::Exec { program, .. } => {
                if program.is_empty() {
                    problem(at("program"), "a program to run".into());
                }
            }
            SinkConfig::Smtp {
                host,
                port,
                username,
                password,
                from,
                to,
                ..
            } => {
                if host.is_empty() {
                    problem(at("host"), "a relay host".into());
                }
                if *port == 0 {
                    problem(at("port"), "a port".into());
                }
                if username.is_some() != password.is_some() {
                    problem(
                        at("username"),
                        "username and password come together, or not at all".into(),
                    );
                }
                check_secret(&mut problem, &at("password"), password.as_ref());
                if from.parse::<lettre::message::Mailbox>().is_err() {
                    problem(at("from"), format!("{from:?} is not a mailbox"));
                }
                if to.is_empty() {
                    problem(at("to"), "at least one recipient".into());
                }
                for (j, t) in to.iter().enumerate() {
                    if t.parse::<lettre::message::Mailbox>().is_err() {
                        problem(
                            format!("sinks.{name}.to[{j}]"),
                            format!("{t:?} is not a mailbox"),
                        );
                    }
                }
            }
        }
    }
    out
}

/// A deployment namespace, refused the way the session would refuse it: a
/// concrete key-expression prefix, no wildcard, no empty chunk. Empty is the
/// bus-root deployment.
pub fn check_namespace(ns: &str) -> Result<(), String> {
    if ns.is_empty() {
        return Ok(());
    }
    zenkey_fleet::check_namespace(ns).map_err(|e| e.to_string())
}

/// The `discipline` bounds, and the state file's directory writable now
/// rather than at the first persist (#389).
fn check_discipline(problem: &mut impl FnMut(String, String), cfg: &Config) {
    let d = &cfg.discipline;
    let non_negative = |v: f64| v.is_finite() && v >= 0.0;
    if let Some(f) = d.for_s {
        if !non_negative(f) {
            problem(
                "discipline.for_s".into(),
                "must be a non-negative number of seconds".into(),
            );
        } else if d.repeat_s > 0.0 && f >= d.repeat_s {
            problem(
                "discipline.for_s".into(),
                format!(
                    "{f}s is not below repeat_s ({}s) — a repeat that fires before the first \
                     announcement is not a repeat",
                    d.repeat_s
                ),
            );
        }
    }
    if !non_negative(d.repeat_s) {
        problem(
            "discipline.repeat_s".into(),
            "must be a non-negative number of seconds (0 = never)".into(),
        );
    }
    if !non_negative(d.group_window_s) || d.group_window_s > 60.0 {
        problem(
            "discipline.group_window_s".into(),
            format!(
                "must be 0–60 seconds, got {} — a group that waits longer than a minute is a \
                 notification that arrives late, not a quieter one",
                d.group_window_s
            ),
        );
    }
    if d.group_by.is_empty() {
        problem(
            "discipline.group_by".into(),
            format!("at least one label to group by (`{GROUP_SYSTEM}` is the default)"),
        );
    }
    for (i, label) in d.group_by.iter().enumerate() {
        if label == "origin" {
            problem(
                format!("discipline.group_by[{i}]"),
                format!(
                    "\"origin\" was v1's origin chunk, which zk2 keys do not carry: group by \
                     `{GROUP_SYSTEM}` (the <system> of <system>/<service>, a host under \
                     hostid.v1) or `{GROUP_ADDRESS}` (<system>/<service>)"
                ),
            );
        }
    }
    if let Some(inhibit) = &d.inhibit
        && inhibit.enabled != Some(false)
    {
        problem("discipline.inhibit".into(), INHIBIT_REMOVED.into());
    }
    if cfg.state_max_entries == 0 {
        problem(
            "state_max_entries".into(),
            "must be positive — a ledger of zero entries re-pages the world every tick".into(),
        );
    }
    if let Some(path) = &cfg.state_file
        && let Err(e) = crate::discipline::state::check_writable(path)
    {
        problem("state_file".into(), e);
    }
}

/// The `doctor` block (#390): exactly one interval spelling, positive;
/// every sink named exists; the floor is one of the three; the bounds are
/// numbers; v1's `sample` named for what it was.
fn check_doctor(problem: &mut impl FnMut(String, String), cfg: &Config) {
    let Some(d) = &cfg.doctor else {
        return;
    };
    match (d.every_h, d.every_s) {
        (None, None) => problem(
            "doctor.every_h".into(),
            "an interval — `every_h` (hours; `every_s` is for tests and demos)".into(),
        ),
        (Some(_), Some(_)) => problem(
            "doctor.every_s".into(),
            "one of `every_h` or `every_s`, not both".into(),
        ),
        (Some(h), None) if !(h.is_finite() && h > 0.0) => problem(
            "doctor.every_h".into(),
            format!("must be a positive number of hours, got {h}"),
        ),
        (None, Some(s)) if !(s.is_finite() && s > 0.0) => problem(
            "doctor.every_s".into(),
            format!("must be a positive number of seconds, got {s}"),
        ),
        _ => {}
    }
    if let Some(t) = d.timeout_s
        && !(t.is_finite() && t > 0.0)
    {
        problem(
            "doctor.timeout_s".into(),
            "must be a positive number of seconds".into(),
        );
    }
    if d.sample.is_some() {
        problem(
            "doctor.sample".into(),
            "v1's per-family state sample: zk2's deep checks (`state-stamp-foreign`, \
             `population-over-bound`) read every owner's state once and take no sample — \
             remove it"
                .into(),
        );
    }
    if let Some(f) = &d.severity_floor
        && !DOCTOR_FLOORS.contains(&f.as_str())
    {
        problem(
            "doctor.severity_floor".into(),
            format!(
                "{f:?} is not a doctor severity — one of {}",
                DOCTOR_FLOORS.join(", ")
            ),
        );
    }
    if d.sinks.is_empty() {
        problem(
            "doctor.sinks".into(),
            "at least one sink — a doctor that reports to nobody is `zenctl doctor` \
             nobody ran"
                .into(),
        );
    }
    for (j, s) in d.sinks.iter().enumerate() {
        if !cfg.sinks.contains_key(s) {
            problem(
                format!("doctor.sinks[{j}]"),
                format!(
                    "{s:?} is not in `sinks` (which has: {})",
                    cfg.sinks.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
            );
        }
    }
}

fn sink_timeout_s(sink: &SinkConfig) -> Option<f64> {
    match sink {
        SinkConfig::Ntfy { timeout_s, .. }
        | SinkConfig::Webhook { timeout_s, .. }
        | SinkConfig::Exec { timeout_s, .. }
        | SinkConfig::Smtp { timeout_s, .. } => *timeout_s,
    }
}

fn check_url(problem: &mut impl FnMut(String, String), at: &str, url: &str) {
    match reqwest::Url::parse(url) {
        Ok(u) if u.scheme() == "http" || u.scheme() == "https" => {}
        Ok(u) => problem(
            at.to_string(),
            format!("scheme {:?} is not http(s)", u.scheme()),
        ),
        Err(e) => problem(at.to_string(), format!("{url:?} is not a URL: {e}")),
    }
}

fn check_secret(problem: &mut impl FnMut(String, String), at: &str, secret: Option<&Secret>) {
    if let Some(s) = secret
        && let Err(e) = s.resolve()
    {
        problem(at.to_string(), e.to_string());
    }
}

/// A header name that carries a credential more often than not.
fn credential_shaped(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "authorization",
        "key",
        "token",
        "secret",
        "password",
        "cookie",
    ]
    .iter()
    .any(|w| n.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
        rules: [{ name: "drops", rule: "dropped", sinks: ["hook"] }],
        sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
    }"#;

    #[test]
    fn a_minimal_config_parses_with_defaults_and_checks_clean() {
        let cfg = parse(MINIMAL).unwrap();
        assert_eq!(cfg.tick_s, 5.0);
        assert_eq!(cfg.bus, BusConfig::default());
        assert!(cfg.contracts.is_empty() && !cfg.clocks_synced);
        assert_eq!(cfg.render.max_message_bytes, 2048);
        assert!(check(&cfg).is_empty(), "{:?}", check(&cfg));
    }

    /// The parse-time refusals: an unknown field anywhere, and a bare string
    /// where a secret goes — with the sentence that says why.
    #[test]
    fn unknown_fields_and_inline_secrets_fail_to_parse() {
        let e = parse(r#"{ rules: [], sinks: {}, tick: 5 }"#)
            .unwrap_err()
            .to_string();
        assert!(e.contains("unknown field `tick`"), "{e}");
        let e = parse(
            r#"{ rules: [], sinks: { ops: { kind: "ntfy", url: "https://n", topic: "t", token: "abc" } } }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("secrets are never inline"), "{e}");
        let e = parse(
            r#"{ rules: [], sinks: { ops: { kind: "ntfy", url: "https://n", topic: "t", token: { env: "A", file: "/b" } } } }"#,
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("not both"), "{e}");
    }

    /// The check-time refusals, each naming its path: a missing sink, a
    /// rule outside the vocabulary, a duplicate condition, a bad tick, an
    /// unset env var, a credential-shaped plain header.
    #[test]
    fn check_names_every_refusal_by_path() {
        // SAFETY: test-local name nobody else reads; set once, unconditionally.
        unsafe { std::env::remove_var("ZENWATCH_TEST_UNSET") };
        let cfg = parse(
            r#"{
              tick_s: 0,
              rules: [
                { name: "a", rule: "silent-for prod/zk2/** 30", sinks: ["nope"] },
                { name: "b", rule: "silent-for prod/zk2/** 30", sinks: ["hook"] },
                { name: "c", rule: "if x then y", severity: "loud", sinks: [] },
              ],
              sinks: {
                hook: { kind: "webhook", url: "ftp://x", method: "post",
                        headers: { "X-Api-Key": "inline", "X-Tok": { env: "ZENWATCH_TEST_UNSET" } } },
              },
            }"#,
        )
        .unwrap();
        let problems = check(&cfg);
        let paths: Vec<&str> = problems.iter().map(|p| p.path.as_str()).collect();
        for expected in [
            "tick_s",
            "rules[0].sinks[0]",
            "rules[1].rule",
            "rules[2].rule",
            "rules[2].severity",
            "rules[2].sinks",
            "sinks.hook.url",
            "sinks.hook.method",
            "sinks.hook.headers.X-Api-Key",
            "sinks.hook.headers.X-Tok",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {paths:?}");
        }
        let vocabulary = problems.iter().find(|p| p.path == "rules[2].rule").unwrap();
        assert!(
            vocabulary.message.contains("liveliness-gone")
                && vocabulary.message.contains("instance-gone"),
            "{vocabulary}"
        );
        let dup = problems.iter().find(|p| p.path == "rules[1].rule").unwrap();
        assert!(dup.message.contains("duplicates rules[0]"), "{dup}");
        let unset = problems
            .iter()
            .find(|p| p.path == "sinks.hook.headers.X-Tok")
            .unwrap();
        assert!(unset.message.contains("not set"), "{unset}");
    }

    /// What the zk2 port took away is refused by name (#614, FL2), each by
    /// its path and with where its question went: v1's `origin-down` (no
    /// silent alias), the dark `alerts`, inhibition switched on (a block
    /// without `enabled` was on in v1), grouping by v1's origin, v1's
    /// registry dirs and the doctor's sample. Inhibition off, or no block,
    /// still runs.
    #[test]
    fn what_zk2_took_away_is_refused_by_name() {
        let cfg = parse(
            r#"{
              bus: { registry: ["/etc/zenwatch/registry"] },
              rules: [
                { name: "hosts", rule: "origin-down h-aaaaaaaaaaaa", sinks: ["hook"] },
                { name: "fleet-alerts", rule: "alerts v1/*/state/*/alert/*", sinks: ["hook"] },
              ],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              discipline: { inhibit: { depth: 4 }, group_by: ["origin", "team"] },
              doctor: { every_h: 6, sample: 200, sinks: ["hook"] },
            }"#,
        )
        .unwrap();
        let problems = check(&cfg);
        let message = |path: &str| {
            problems
                .iter()
                .find(|p| p.path == path)
                .unwrap_or_else(|| panic!("no problem at {path}: {problems:?}"))
                .message
                .clone()
        };
        assert!(message("rules[0].rule").contains("instance-gone"));
        assert!(message("rules[1].rule").contains("#613"));
        let inhibit = message("discipline.inhibit");
        assert!(
            inhibit.contains("removed") && inhibit.contains("catalog"),
            "{inhibit}"
        );
        assert!(message("discipline.group_by[0]").contains("system"));
        assert!(
            !problems.iter().any(|p| p.path == "discipline.group_by[1]"),
            "a rule's own label groups as before"
        );
        assert!(message("bus.registry").contains("contracts"));
        assert!(message("doctor.sample").contains("remove it"));

        for off in [
            "discipline: { inhibit: { enabled: false } }",
            "discipline: { inhibit: { enabled: false, depth: 4 } }",
            "discipline: {}",
        ] {
            let cfg = parse(&format!(
                r#"{{ {off}, rules: [{{ name: "d", rule: "dropped", sinks: ["hook"] }}],
                   sinks: {{ hook: {{ kind: "webhook", url: "https://example.org/h" }} }} }}"#
            ))
            .unwrap();
            assert!(check(&cfg).is_empty(), "{off}: {:?}", check(&cfg));
        }
        let on = parse(
            r#"{ discipline: { inhibit: { enabled: true } },
                 rules: [{ name: "d", rule: "dropped", sinks: ["hook"] }],
                 sinks: { hook: { kind: "webhook", url: "https://example.org/h" } } }"#,
        )
        .unwrap();
        assert_eq!(
            check(&on).into_iter().map(|p| p.path).collect::<Vec<_>>(),
            ["discipline.inhibit"]
        );
    }

    /// The bus section's zk2 field: `namespace`, which `base` spells too
    /// (as zenctl's `--base` is its `--namespace`), refused when it is not a
    /// concrete prefix.
    #[test]
    fn the_namespace_is_spelled_two_ways_and_must_be_concrete() {
        let with = |field: &str, ns: &str| {
            parse(&format!(
                r#"{{ bus: {{ {field}: "{ns}" }}, rules: [{{ name: "d", rule: "dropped", sinks: ["h"] }}],
                   sinks: {{ h: {{ kind: "webhook", url: "https://example.org/h" }} }} }}"#
            ))
            .unwrap()
        };
        assert_eq!(
            with("namespace", "site/prod").bus.namespace.as_deref(),
            Some("site/prod")
        );
        assert_eq!(with("base", "prod").bus.namespace.as_deref(), Some("prod"));
        assert!(check(&with("namespace", "site/prod")).is_empty());
        for bad in ["prod/*", "a//b"] {
            let problems = check(&with("namespace", bad));
            assert!(
                problems.iter().any(|p| p.path == "bus.namespace"),
                "{bad}: {problems:?}"
            );
        }
    }

    /// The discipline's own refusals (#389), each by path: a `for` at or
    /// past the repeat, a group window past a minute, a state file whose
    /// directory does not exist.
    #[test]
    fn the_discipline_bounds_are_refused_by_path() {
        let cfg = parse(
            r#"{
              rules: [{ name: "drops", rule: "dropped", sinks: ["hook"], for_s: 30 }],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              discipline: { for_s: 60, repeat_s: 30, group_window_s: 61 },
              state_file: "/nonexistent-zenwatch-dir/state.json",
              state_max_entries: 0,
            }"#,
        )
        .unwrap();
        let problems = check(&cfg);
        let paths: Vec<&str> = problems.iter().map(|p| p.path.as_str()).collect();
        for expected in [
            "rules[0].for_s",
            "discipline.for_s",
            "discipline.group_window_s",
            "state_file",
            "state_max_entries",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {paths:?}");
        }
        let state = problems.iter().find(|p| p.path == "state_file").unwrap();
        assert!(
            state.message.contains("/nonexistent-zenwatch-dir"),
            "names the directory: {state}"
        );
        // The defaults check clean and are what the README says.
        let d = DisciplineConfig::default();
        assert_eq!(d.group_by, vec!["system".to_string()]);
        assert_eq!(d.inhibit, None);
        assert!(d.resolved_notice && d.repeat_s == 0.0 && d.for_s.is_none());
    }

    /// The doctor block's refusals (#390), each by path: no interval, both
    /// spellings, a non-positive one, an unknown sink, a floor outside the
    /// three, a rule that would collide with the doctor's own id — and the
    /// two honest spellings both parse to an interval.
    #[test]
    fn the_doctor_block_is_refused_by_path() {
        let cfg = parse(
            r#"{
              rules: [{ name: "doctor", rule: "doctor split-brain", sinks: ["hook"] }],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              doctor: { sinks: ["pager"], severity_floor: "critical", timeout_s: 0 },
            }"#,
        )
        .unwrap();
        let problems = check(&cfg);
        let paths: Vec<&str> = problems.iter().map(|p| p.path.as_str()).collect();
        for expected in [
            "doctor.every_h",
            "doctor.sinks[0]",
            "doctor.severity_floor",
            "doctor.timeout_s",
            "rules[0].name",
        ] {
            assert!(paths.contains(&expected), "missing {expected} in {paths:?}");
        }
        let both = parse(
            r#"{
              rules: [{ name: "drops", rule: "dropped", sinks: ["hook"] }],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              doctor: { every_h: 6, every_s: 5, sinks: ["hook"] },
            }"#,
        )
        .unwrap();
        assert!(check(&both).iter().any(|p| p.path == "doctor.every_s"));
        assert_eq!(both.doctor.as_ref().unwrap().every(), None);
        let zero = parse(
            r#"{
              rules: [{ name: "drops", rule: "dropped", sinks: ["hook"] }],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              doctor: { every_h: 0, sinks: ["hook"] },
            }"#,
        )
        .unwrap();
        assert!(check(&zero).iter().any(|p| p.path == "doctor.every_h"));

        let hours = parse(
            r#"{
              clocks_synced: true,
              rules: [{ name: "drops", rule: "dropped", sinks: ["hook"] }],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              doctor: { every_h: 6, deep: true, sinks: ["hook"], severity_floor: "warning" },
            }"#,
        )
        .unwrap();
        assert!(check(&hours).is_empty(), "{:?}", check(&hours));
        assert!(hours.clocks_synced);
        let d = hours.doctor.as_ref().unwrap();
        assert!(d.deep);
        assert_eq!(d.every(), Some(std::time::Duration::from_secs(6 * 3600)));
        assert_eq!(d.every_spelled(), "6h");
        assert_eq!(d.floor_rank(), severity_rank_of("warning"));
        let seconds = parse(
            r#"{
              rules: [{ name: "drops", rule: "dropped", sinks: ["hook"] }],
              sinks: { hook: { kind: "webhook", url: "https://example.org/h" } },
              doctor: { every_s: 2.5, sinks: ["hook"] },
            }"#,
        )
        .unwrap();
        assert!(check(&seconds).is_empty());
        assert_eq!(
            seconds.doctor.as_ref().unwrap().every(),
            Some(std::time::Duration::from_secs_f64(2.5))
        );
        // Not scheduled: absent, and nothing to refuse.
        assert!(parse(MINIMAL).unwrap().doctor.is_none());
    }

    fn severity_rank_of(s: &str) -> u8 {
        crate::discipline::severity_rank(s)
    }

    /// A file secret reads with its newline trimmed; an unreadable one is a
    /// problem that names the file.
    #[test]
    fn a_file_secret_reads_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pw");
        std::fs::write(&path, "hunter2\n").unwrap();
        assert_eq!(Secret::File(path.clone()).resolve().unwrap(), "hunter2");
        let missing = Secret::File(dir.path().join("nope")).resolve().unwrap_err();
        assert!(matches!(missing, SecretError::File(..)), "{missing}");
    }
}
