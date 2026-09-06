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
    /// The watchdog's evaluation cadence for the engine rules, seconds.
    #[serde(default = "default_tick")]
    pub tick_s: f64,
    pub rules: Vec<RuleConfig>,
    pub sinks: BTreeMap<String, SinkConfig>,
    #[serde(default)]
    pub render: RenderConfig,
}

fn default_tick() -> f64 {
    5.0
}

/// The `bus` section — the same knobs as a zenctl context, and resolved
/// against one ([`crate::bus`]).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BusConfig {
    /// A named explorer context to start from; `--context` overrides it.
    pub context: Option<String>,
    pub base: Option<String>,
    #[serde(default)]
    pub connect: Vec<String>,
    #[serde(default)]
    pub listen: Vec<String>,
    pub scouting: Option<bool>,
    pub timeout_s: Option<u64>,
    pub zenoh_config: Option<PathBuf>,
    #[serde(default)]
    pub registry: Vec<PathBuf>,
}

/// One rule, as written.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    pub name: String,
    /// One spelling of the closed vocabulary ([`crate::rules::VOCABULARY`]).
    pub rule: String,
    /// `info` | `warning` | `error` | `critical`; `warning` when unsaid. An
    /// `alerts` rule uses the producer's own severity when the document
    /// carries one, this otherwise.
    pub severity: Option<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    /// Names into `sinks`; at least one.
    pub sinks: Vec<String>,
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
/// delivery failure at 3am.
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
                { name: "a", rule: "silent-for v1/** 30", sinks: ["nope"] },
                { name: "b", rule: "silent-for v1/** 30", sinks: ["hook"] },
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
            vocabulary.message.contains("liveliness-gone"),
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
