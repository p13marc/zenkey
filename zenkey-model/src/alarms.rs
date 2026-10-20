//! `alarms.v1`: the active alarms of one service
//! (`spec/profiles/alarms/v1.md`, #800).
//!
//! The profile's session-free half: the key recipe, byte for byte
//! ZenSight's (§2.2), with its framing and the sanitizing step a reader
//! re-derives through (§2.3); the document and its checks (§2.4); the
//! reading of one key's samples into transitions (§2.6); a reference to an
//! alarm from another interface (§2.12); and §5's four questions as pure
//! judgements in three states (§2.14, §2.15), built on
//! [`freshness::judge_all`] at the contract's 900 s.
//!
//! ```text
//! alert_key(rule, labels)   ─▶ the key │ Unframed: the strict recipe, framed input only      §2.2
//! sanitize(rule, labels)    ─▶ the discriminating input, with §2.3's substitutions         §2.3
//! derive(rule, labels)      ─▶ Derived{key, input, framed}: the reader's, never refusing   §2.3
//! Alarm::from_json(v)       ─▶ Alarm │ Invalid{schema, time_range}; check(a) ─▶ problems   §2.4
//! Track::apply(sample)      ─▶ raised │ escalated │ refreshed │ confirmed │ resolved │ retired │ unreadable  §2.6
//! AlarmRef::key / from_key  ─▶ zk2/<system>/<service>/alarms.v1/state/alarms/<alert_key>  §2.12
//! firing(reading)           ─▶ §5 "is this alarm firing?": yes │ no │ unobservable │ not_asked, + reason
//! owner_set(p, seed, win)   ─▶ established │ unobservable │ not_asked: an owner's alarms known whole  §2.15
//! any_firing(s, owners)     ─▶ §5 "is anything firing at or above s?"
//! rederives(key, payload)   ─▶ §5 "does this key re-derive from its document?"
//! stale_firing(reading)     ─▶ §5 "is a firing alarm past its horizon, its owner present?"
//! ```
//!
//! There is no I/O and no clock here. A reader merges what it heard of a
//! key by the owner's stamp (core §4.2), measures what its observations
//! hold, and hands them over as values. The owner's puts, its confirmations
//! and its adoption at a restart (§2.5, §2.7, §2.9) are the runtime's.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::freshness::{self, Horizon, Observation};
use crate::grammar::{self, Addr, IfaceId, Key, KeyError, KindToken, ZkKey};
use crate::slug::{chunk_slug, chunk_unslug};

/// The profile's id, which is also its standard contract's interface id.
pub const PROFILE: &str = "alarms.v1";

/// The standard contract's interface id (§3).
pub const IFACE: &str = "alarms.v1";

/// The profile's name, as an annotation key's prefix would carry it.
pub const NAME: &str = "alarms";

/// The published vocabulary (§4): empty. Core 0.34 replaces the interim
/// `severity_default` with it, so W105 warns of every `alarms.<key>`.
pub const VOCABULARY: &[&str] = &[];

/// The one template (§3): `state/alarms/{alert_key}`.
pub const ALARMS: &str = "alarms/{alert_key}";

/// The template's literal chunk.
const ALARMS_CHUNK: &str = "alarms";

/// The contract's horizon, `freshness.ttl_s = 900` (§2.7).
pub const TTL: Duration = Duration::from_secs(900);

/// The owner's bound between two confirmations of a firing alarm, ttl/2
/// (`freshness.v1` §2.4).
pub const REFRESH: Duration = Duration::from_secs(450);

/// The contract's bound on one service's alarms (§2.8).
pub const MAX_ALARMS: u64 = 256;

/// The host-scoped label named for the host itself (§2.2).
pub const HOST: &str = "host";

/// The prefix of every other host-scoped label name (§2.2).
pub const HOST_SCOPED_PREFIX: &str = "host.";

/// The rule an empty one is sanitized to (§2.3).
pub const UNNAMED: &str = "unnamed";

/// The largest time a document carries, 2^53−1 ms (§2.4).
pub const MAX_TIME_MS: u64 = (1 << 53) - 1;

/// The horizon, as `freshness.v1` reads it.
#[must_use]
pub fn horizon() -> Horizon {
    Horizon::Within(TTL)
}

// --- §2.2, §2.3: the key ---------------------------------------------------------

/// Whether a label is host-scoped (§2.2): `host`, or a name starting with
/// `host.`. The comparison is exact: no trimming, no case folding.
#[must_use]
pub fn is_host_scoped(name: &str) -> bool {
    name == HOST || name.starts_with(HOST_SCOPED_PREFIX)
}

/// Why the strict recipe refuses its input (§2.3): the framing would stop
/// being injective. Host-scoped labels are removed first, so none of them
/// is ever the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unframed {
    /// The rule is empty, or holds a newline.
    Rule,
    /// A discriminating label's name is empty, or holds a newline or `=`.
    LabelName(String),
    /// A discriminating label's value holds a newline.
    LabelValue { name: String },
}

impl Unframed {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::LabelName(_) => "label_name",
            Self::LabelValue { .. } => "label_value",
        }
    }
}

impl fmt::Display for Unframed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rule => f.write_str("the rule is empty or holds a newline"),
            Self::LabelName(n) => write!(f, "label name {n:?} is empty or holds a newline or '='"),
            Self::LabelValue { name } => write!(f, "label {name:?}'s value holds a newline"),
        }
    }
}

/// FNV-1a, 64 bits: offset basis `0xcbf29ce484222325`, prime
/// `0x100000001b3` (§2.2).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The input: the rule, then `\n<name>=<value>` per label, sorted by name,
/// then by value, in byte order (§2.2). `str`'s order is byte order.
fn frame(rule: &str, labels: &mut [(&str, &str)]) -> String {
    labels.sort_unstable();
    let mut input = String::from(rule);
    for (name, value) in labels.iter() {
        input.push('\n');
        input.push_str(name);
        input.push('=');
        input.push_str(value);
    }
    input
}

fn render(input: &str) -> String {
    format!("{:016x}", fnv1a_64(input.as_bytes()))
}

/// The strict recipe (§2.2): host-scoped labels removed, then the input
/// framed and hashed, or refused when the rule or a discriminating label
/// breaks the framing (§2.3). `labels` are every label of the document.
///
/// ```
/// use zenkey_model::alarms::alert_key;
/// let key = alert_key("link_down", [("port", "eth0"), ("host", "h-3fa9c2d41b7e"), ("peer", "r2")]);
/// assert_eq!(key.as_deref(), Ok("a659f813308ad1da"));
/// ```
///
/// # Errors
///
/// [`Unframed`], for the first part that breaks the framing: the rule,
/// then the labels in the order given.
pub fn alert_key<'a>(
    rule: &str,
    labels: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<String, Unframed> {
    if rule.is_empty() || rule.contains('\n') {
        return Err(Unframed::Rule);
    }
    let mut kept = Vec::new();
    for (name, value) in labels {
        if is_host_scoped(name) {
            continue;
        }
        if name.is_empty() || name.contains(['\n', '=']) {
            return Err(Unframed::LabelName(name.to_owned()));
        }
        if value.contains('\n') {
            return Err(Unframed::LabelValue {
                name: name.to_owned(),
            });
        }
        kept.push((name, value));
    }
    Ok(render(&frame(rule, &mut kept)))
}

/// The sanitizing step (§2.3): host-scoped labels removed, then a newline
/// becomes `_` in the rule and in each value, a newline or `=` becomes `_`
/// in each name, an empty rule becomes `unnamed` and an empty name `_`. It
/// changes nothing in framed input, so a reader can always apply it.
#[must_use]
pub fn sanitize<'a>(
    rule: &str,
    labels: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> (String, Vec<(String, String)>) {
    let rule = rule.replace('\n', "_");
    let rule = if rule.is_empty() {
        UNNAMED.to_owned()
    } else {
        rule
    };
    let labels = labels
        .into_iter()
        .filter(|(n, _)| !is_host_scoped(n))
        .map(|(n, v)| {
            let n = n.replace(['\n', '='], "_");
            (
                if n.is_empty() { "_".to_owned() } else { n },
                v.replace('\n', "_"),
            )
        })
        .collect();
    (rule, labels)
}

/// What a reader derives from a document (§2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    /// The alert key: 16 lowercase hex digits.
    pub key: String,
    /// The input hashed: the framed, sanitized rule and discriminating
    /// labels.
    pub input: String,
    /// Whether the strict recipe accepts the document as it is: `false`
    /// when the key went through the sanitizing step.
    pub framed: bool,
}

/// The reader's recipe (§2.3): [`sanitize`], then frame and hash. It never
/// refuses, and on framed input it is [`alert_key`].
#[must_use]
pub fn derive<'a>(rule: &str, labels: impl IntoIterator<Item = (&'a str, &'a str)>) -> Derived {
    let labels: Vec<(&str, &str)> = labels.into_iter().collect();
    let framed = alert_key(rule, labels.iter().copied()).is_ok();
    let (rule, sanitized) = sanitize(rule, labels);
    let mut pairs: Vec<(&str, &str)> = sanitized
        .iter()
        .map(|(n, v)| (n.as_str(), v.as_str()))
        .collect();
    let input = frame(&rule, &mut pairs);
    Derived {
        key: render(&input),
        input,
        framed,
    }
}

// --- §2.4: the document ----------------------------------------------------------

/// A severity (§2.4), ordered: `Info` < `Warning` < `Critical`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    /// The wire token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }

    /// The severity a wire token names, or `None`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "info" => Some(Self::Info),
            "warning" => Some(Self::Warning),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An alarm's state, in its document (§2.4, §2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Firing,
    Resolved,
}

impl State {
    /// The wire token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
        }
    }
}

/// One alarm's document, `alarms.v1.Alarm` (§2.4, §3). Members the 1.0
/// schema does not name are ignored when read, as a later minor may add
/// some.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alarm {
    /// Milliseconds since the Unix epoch, on the owner's clock, of the
    /// latest transition (§2.5).
    pub timestamp: u64,
    /// The instant of the latest refresh, on a refresh only (§2.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    pub rule: String,
    pub severity: Severity,
    pub state: State,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

/// Why a payload is no document a reader can read (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// Not strict JSON (text 0.2, F-126): not UTF-8, not JSON, a duplicate
    /// member, a number that is not finite, or a string with no UTF-8 form
    /// (a lone surrogate escape), with why. Only [`decode`] reports it.
    Json(String),
    /// Outside the 1.0 schema, with where and why.
    Schema(String),
    /// A time outside 0 to 2^53−1 ms, naming the member.
    TimeRange(&'static str),
}

impl Invalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Json(_) => "json",
            Self::Schema(_) => "schema",
            Self::TimeRange(_) => "time_range",
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(why) => write!(f, "not strict JSON: {why}"),
            Self::Schema(why) => write!(f, "outside alarms.v1.Alarm: {why}"),
            Self::TimeRange(m) => write!(f, "{m} is outside 0 to 2^53-1 ms"),
        }
    }
}

/// An integral number as the 1.0 schema reads one (`1.0` included, JSON
/// Schema 2020-12): `Ok(Some(n))` within 0 to 2^53−1, `Ok(None)` for an
/// integer outside it, `Err` for a value that is no integer.
fn time(v: &Value) -> Result<Option<u64>, ()> {
    let Value::Number(n) = v else { return Err(()) };
    if let Some(u) = n.as_u64() {
        return Ok((u <= MAX_TIME_MS).then_some(u));
    }
    if n.is_i64() {
        return Ok(None);
    }
    let f = n.as_f64().ok_or(())?;
    if !f.is_finite() || f.fract() != 0.0 {
        return Err(());
    }
    // `-0.0 >= 0.0` holds, and is the integer 0.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    Ok((f >= 0.0 && f <= MAX_TIME_MS as f64).then_some(f as u64))
}

impl Alarm {
    /// Reads a decoded payload (§2.4): the 1.0 schema's members, types and
    /// enums, then the times' range.
    ///
    /// # Errors
    ///
    /// [`Invalid`]: the schema's first refusal, then a time out of range.
    pub fn from_json(v: &Value) -> Result<Self, Invalid> {
        let schema = |why: String| Invalid::Schema(why);
        let Value::Object(o) = v else {
            return Err(schema("not an object".into()));
        };
        for m in ["timestamp", "rule", "severity", "state", "summary"] {
            if !o.contains_key(m) {
                return Err(schema(format!("/{m}: missing")));
            }
        }
        let string = |m: &str| -> Result<Option<String>, Invalid> {
            match o.get(m) {
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(Invalid::Schema(format!("/{m}: not a string"))),
            }
        };
        let integer = |m: &'static str| -> Result<Option<Option<u64>>, Invalid> {
            match o.get(m) {
                None => Ok(None),
                Some(t) => time(t)
                    .map(Some)
                    .map_err(|()| Invalid::Schema(format!("/{m}: not an integer"))),
            }
        };
        // Present by the loop above; refused, never a panic, if it is not.
        let missing = |m: &str| Invalid::Schema(format!("/{m}: missing"));
        let timestamp = integer("timestamp")?.ok_or_else(|| missing("timestamp"))?;
        let observed_at_ms = integer("observed_at_ms")?;
        let rule = string("rule")?.ok_or_else(|| missing("rule"))?;
        let summary = string("summary")?.ok_or_else(|| missing("summary"))?;
        let category = string("category")?;
        let severity = match o.get("severity") {
            Some(Value::String(s)) => Severity::parse(s),
            _ => None,
        }
        .ok_or_else(|| schema("/severity: not info, warning or critical".into()))?;
        let state = match o.get("state").and_then(Value::as_str) {
            Some("firing") => State::Firing,
            Some("resolved") => State::Resolved,
            _ => return Err(schema("/state: not firing or resolved".into())),
        };
        let mut labels = BTreeMap::new();
        match o.get("labels") {
            None => {}
            Some(Value::Object(l)) => {
                for (n, v) in l {
                    let Value::String(v) = v else {
                        return Err(schema(format!("/labels/{n}: not a string")));
                    };
                    labels.insert(n.clone(), v.clone());
                }
            }
            Some(_) => return Err(schema("/labels: not an object".into())),
        }
        let timestamp = timestamp.ok_or(Invalid::TimeRange("timestamp"))?;
        let observed_at_ms = match observed_at_ms {
            None => None,
            Some(t) => Some(t.ok_or(Invalid::TimeRange("observed_at_ms"))?),
        };
        Ok(Self {
            timestamp,
            observed_at_ms,
            rule,
            severity,
            state,
            summary,
            category,
            labels,
        })
    }

    /// The labels, as the recipe takes them.
    pub fn label_pairs(&self) -> impl Iterator<Item = (&str, &str)> {
        self.labels.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// What a reader derives from this document (§2.3).
    #[must_use]
    pub fn derive(&self) -> Derived {
        derive(&self.rule, self.label_pairs())
    }

    /// The alert key of this document (§2.2, §2.3).
    #[must_use]
    pub fn alert_key(&self) -> String {
        self.derive().key
    }

    /// The document's problems (§2.4), in this order: the errors, then the
    /// warnings.
    #[must_use]
    pub fn check(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        if let Some(o) = self.observed_at_ms {
            if o < self.timestamp {
                out.push(Problem::ObservedBeforeTimestamp);
            }
            if self.state == State::Resolved {
                out.push(Problem::ObservedOnResolved);
            }
        }
        if let Err(u) = alert_key(&self.rule, self.label_pairs()) {
            out.push(Problem::Unframed(u));
        }
        for n in self.labels.keys() {
            if !is_label_name(n) {
                out.push(Problem::LabelName(n.clone()));
            }
        }
        out
    }
}

/// Reads a payload's bytes as a document (§2.4, text 0.2): strict JSON
/// first, as core §3.3's D000 reads a descriptor (UTF-8, RFC 8259, no
/// duplicate member at any depth, every number finite, every string with a
/// UTF-8 form), then [`Alarm::from_json`]. So two readers never read two
/// severities, or derive two keys, from one put (F-126).
///
/// # Errors
///
/// [`Invalid::Json`] for bytes that are not strict JSON, else
/// [`Alarm::from_json`]'s.
pub fn decode(bytes: &[u8]) -> Result<Alarm, Invalid> {
    let text = std::str::from_utf8(bytes).map_err(|e| Invalid::Json(format!("not UTF-8: {e}")))?;
    let v = crate::strict::parse_json(text).map_err(|e| Invalid::Json(e.to_string()))?;
    Alarm::from_json(&v)
}

/// The recommended form of a label name (§2.4): one or more
/// `[a-z][a-z0-9_]*` segments joined by `.`.
#[must_use]
pub fn is_label_name(name: &str) -> bool {
    name.split('.').all(|seg| {
        let mut cs = seg.bytes();
        cs.next().is_some_and(|c| c.is_ascii_lowercase())
            && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    })
}

/// Whether a problem breaks a MUST (an error) or a SHOULD (a warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gravity {
    Error,
    Warning,
}

/// A problem of a readable document (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// `observed_at_ms` earlier than `timestamp`: an error.
    ObservedBeforeTimestamp,
    /// `observed_at_ms` on a resolved document: an error.
    ObservedOnResolved,
    /// The rule or a discriminating label breaks the framing, so the key
    /// is derived through the sanitizing step (§2.3): a warning.
    Unframed(Unframed),
    /// A label name outside the recommended form: a warning.
    LabelName(String),
}

impl Problem {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ObservedBeforeTimestamp => "observed_before_timestamp",
            Self::ObservedOnResolved => "observed_on_resolved",
            Self::Unframed(_) => "unframed",
            Self::LabelName(_) => "label_name_form",
        }
    }

    /// An error or a warning.
    #[must_use]
    pub fn gravity(&self) -> Gravity {
        match self {
            Self::ObservedBeforeTimestamp | Self::ObservedOnResolved => Gravity::Error,
            Self::Unframed(_) | Self::LabelName(_) => Gravity::Warning,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObservedBeforeTimestamp => f.write_str("observed_at_ms is before timestamp"),
            Self::ObservedOnResolved => f.write_str("a resolved document carries observed_at_ms"),
            Self::Unframed(u) => {
                write!(f, "{u}, so the key is derived through the sanitizing step")
            }
            Self::LabelName(n) => write!(f, "label name {n:?} is not lowercase dotted snake_case"),
        }
    }
}

// --- §2.6: transitions ----------------------------------------------------------

/// One sample of one alarm key, in the owner's stamp order (core §4.2).
#[derive(Debug, Clone, Copy)]
pub enum Sample<'a> {
    /// A put whose payload reads as a document.
    Put(&'a Alarm),
    /// A put whose payload does not (§2.4).
    Unreadable,
    /// A delete, or a `reply_del`.
    Delete,
}

/// What one sample means for the alarm (§2.6), read from the document's
/// `state`, never from the sample's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transition {
    /// Firing, from nothing held, from resolved, or with a new timestamp at
    /// the same severity.
    Raised,
    /// Firing at another severity than the firing one held, either way.
    Escalated,
    /// The same transition, its content changed (a summary, a host-scoped
    /// label, `observed_at_ms`).
    Refreshed,
    /// The document held, put again unchanged (`freshness.v1` §2.4).
    Confirmed,
    /// Resolved: a resolved document, or a delete of a firing one.
    Resolved,
    /// A delete of a key whose alarm was resolved, or not known.
    Retired,
    /// A put that does not read: the alarm's state is not known.
    Unreadable,
}

impl Transition {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raised => "raised",
            Self::Escalated => "escalated",
            Self::Refreshed => "refreshed",
            Self::Confirmed => "confirmed",
            Self::Resolved => "resolved",
            Self::Retired => "retired",
            Self::Unreadable => "unreadable",
        }
    }
}

impl fmt::Display for Transition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The transition `sample` makes from the document `held` (§2.6).
#[must_use]
pub fn transition(held: Option<&Alarm>, sample: Sample<'_>) -> Transition {
    use Transition as T;
    let same = |h: &Alarm, n: &Alarm| {
        if n == h { T::Confirmed } else { T::Refreshed }
    };
    match (held, sample) {
        (_, Sample::Unreadable) => T::Unreadable,
        (Some(h), Sample::Delete) if h.state == State::Firing => T::Resolved,
        (_, Sample::Delete) => T::Retired,
        (None, Sample::Put(n)) => match n.state {
            State::Firing => T::Raised,
            State::Resolved => T::Resolved,
        },
        (Some(h), Sample::Put(n)) => match (h.state, n.state) {
            (State::Firing, State::Firing) if n.severity != h.severity => T::Escalated,
            (State::Resolved, State::Firing) => T::Raised,
            (State::Firing, State::Resolved) => T::Resolved,
            (State::Firing, State::Firing) if n.timestamp != h.timestamp => T::Raised,
            (State::Resolved, State::Resolved) if n.timestamp != h.timestamp => T::Resolved,
            _ => same(h, n),
        },
    }
}

/// One key's document as a reader follows it (§2.6): what it holds, and
/// the transition each sample makes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Track {
    /// The document held: `None` before any, after a delete, and after a
    /// put that did not read.
    pub held: Option<Alarm>,
}

impl Track {
    /// Reads one sample, the next in the owner's stamp order.
    pub fn apply(&mut self, sample: Sample<'_>) -> Transition {
        let t = transition(self.held.as_ref(), sample);
        self.held = match sample {
            Sample::Put(a) => Some(a.clone()),
            Sample::Unreadable | Sample::Delete => None,
        };
        t
    }
}

// --- §2.12: references ----------------------------------------------------------

/// One alarm, named from another interface (§2.12): its owner's system and
/// service, and its alert key, never packed into one string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AlarmRef {
    pub system: String,
    pub service: String,
    pub alert_key: String,
}

impl AlarmRef {
    /// The alarm's key: `zk2/<system>/<service>/alarms.v1/state/alarms/<alert_key>`,
    /// the alert key slugged as any parameter value is (core §1.4).
    ///
    /// # Errors
    ///
    /// [`KeyError`] when the system or the service is not a name (core §1.2).
    pub fn key(&self) -> Result<Key, KeyError> {
        let addr = Addr::new(&self.system, &self.service)?;
        let iface = IfaceId::new(NAME, 1)?;
        grammar::data_key(
            &addr,
            &iface,
            KindToken::State,
            &[ALARMS_CHUNK, &chunk_slug(&self.alert_key)],
        )
    }

    /// The alarm a key names, or `None` for a key that is not one of this
    /// major's alarms.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        let ZkKey::Data {
            addr,
            iface,
            kind: KindToken::State,
            resource,
        } = grammar::parse(key).ok()?
        else {
            return None;
        };
        if iface.to_string() != IFACE {
            return None;
        }
        let [lit, chunk] = resource.as_slice() else {
            return None;
        };
        if lit != ALARMS_CHUNK {
            return None;
        }
        Some(Self {
            system: addr.system.as_str().to_owned(),
            service: addr.service.as_str().to_owned(),
            alert_key: chunk_unslug(chunk)?,
        })
    }
}

// --- §5: what a tool may conclude -----------------------------------------------

/// What the descriptor of a present owner says (§2.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listing {
    /// Its `interfaces` list `alarms.v1`, with its `"token"`, which plays
    /// no part in a verdict.
    Listed { token: bool },
    /// Its `interfaces` do not list `alarms.v1`.
    NotListed,
    /// The descriptor could not be read.
    Unread,
}

/// An owner's presence, as the reader saw it (core §8.1, §8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Its instance token was seen, with what its descriptor says.
    Present(Listing),
    /// A complete read held no instance token of it.
    Absent,
    /// Its token was not seen, in a read possibly incomplete or refused.
    Incomplete,
    /// Across a constrained face, where presence does not cross, and the
    /// deployment's word says the owner implements `alarms.v1` (§2.11):
    /// whether the face lets the reader reach its alarms.
    AcrossFace { reachable: bool },
}

/// The latest sample a reader holds of one key, by the owner's stamp
/// (core §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Latest {
    /// A put of a firing document.
    Firing,
    /// A put of a resolved document.
    Resolved,
    /// A delete, or a `reply_del`.
    Deleted,
    /// A put that does not read (§2.4).
    Undecodable,
}

/// One reading of one alarm key (§2.14).
#[derive(Debug, Clone, Copy)]
pub struct Reading<'a> {
    pub presence: Presence,
    /// The observations of the key, judged as `freshness.v1` judges them,
    /// at [`TTL`]. A GET among them is the reader's seed, made after any
    /// loss its subscription had (core §4.2): one that answered this key,
    /// with a value or a `reply_del`, heals the loss for it; one that drew
    /// nothing for it does not (text 0.2, F-128).
    pub observations: &'a [Observation],
    /// The latest sample read, or `None` when none was.
    pub latest: Option<Latest>,
    /// Whether that latest put's stamp's id is not the current
    /// descriptor's `meta.zid`, compared by value (core §4.3, §3.3): an
    /// earlier instance stamped it, or another session did, which is also
    /// core §4.2's foreign-stamp finding. `false` when the descriptor
    /// carries no `meta.zid`: a stamp is then unattributable, never foreign
    /// (core §3.3). A re-mint that keeps its session keeps its zid, and is
    /// not told apart (text 0.2, F-129).
    pub previous_instance: bool,
    /// Whether the owner's alarms are established in this reading
    /// ([`owner_set`], §2.15): a key they do not hold then reads not firing.
    pub held_whole: bool,
}

/// A question of §5, answered in the three states: a pole of each
/// established kind, and the two unestablished kinds apart (core O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    No,
    /// The question was put, and the reading could not answer it.
    Unobservable,
    /// Nobody put the question, or its premise does not hold.
    NotAsked,
}

impl Answer {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
        }
    }
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an answer is what it is: the fixture's reason class (§2.14, §2.15,
/// §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// Is it firing, yes: a fresh firing document.
    Firing,
    /// Is it firing, no: a resolved document.
    Resolved,
    /// Is it firing, no: a delete or a `reply_del`, read as resolved.
    Deleted,
    /// Unobservable, or the stale question's yes: not confirmed within
    /// 900 s.
    Stale,
    /// The stale question's no: confirmed within 900 s.
    Fresh,
    /// The stale question, not asked: the latest is not firing.
    NotFiring,
    /// Not asked: absent, presence's word.
    Absent,
    /// Not asked: the descriptor does not list `alarms.v1`.
    NotListed,
    /// Unobservable: its token not seen, in a read possibly incomplete.
    PresenceIncomplete,
    /// Unobservable: the descriptor could not be read.
    NoDescriptor,
    /// Unobservable: across a face that does not let the reader reach the
    /// owner's alarms (§2.11).
    FaceClosed,
    /// Unobservable: across a face, nothing of the key arrived.
    NothingCrossed,
    /// Unobservable: across a face, where presence does not cross.
    AcrossFace,
    /// Not asked: only an archive was read, last-known (core S6).
    LastKnown,
    /// Not asked: the latest put is an earlier instance's last word, which
    /// the present instance has neither put nor answered (§2.14).
    PreviousInstance,
    /// Unobservable: a subscription lost deliveries, and no GET answered
    /// since.
    Incomplete,
    /// Unobservable: nothing of the key was read.
    NotRead,
    /// Is it firing, no: the owner's alarms, established, do not hold it.
    NotHeld,
    /// Unobservable: the latest put does not read (§2.4).
    Undecodable,
    /// An owner's alarms established: a complete GET that it answered.
    Answered,
    /// An owner's alarms established: a complete, empty GET, and a lossless
    /// window longer than the horizon, the owner present throughout.
    Window,
    /// An owner's alarms unobservable: no GET was made.
    NoSeed,
    /// An owner's alarms unobservable: the GET ended at its timeout, or
    /// with an error reply.
    SeedIncomplete,
    /// An owner's alarms unobservable: an empty GET and no window (core O5).
    NoMember,
    /// An owner's alarms unobservable: the window lost a delivery.
    WindowLost,
    /// An owner's alarms unobservable: the window was no longer than the
    /// horizon.
    WindowShort,
    /// An owner's alarms unobservable: the owner was not present
    /// throughout the window.
    OwnerGap,
    /// Anything firing, yes.
    AnyFiring,
    /// Anything firing, no.
    NoneFiring,
    /// Anything firing, unobservable: an owner's alarms are.
    OwnerUnobservable,
    /// Anything firing, unobservable: an alarm at or above the severity,
    /// or of a severity not known, is.
    AlarmUnobservable,
    /// Anything firing, not asked: no owner's alarms were established or
    /// unobservable.
    NoOwner,
    /// Re-derives, yes: the strict recipe gives the key.
    Strict,
    /// Re-derives, yes: the sanitizing step gives the key (§2.3).
    Sanitized,
    /// Re-derives, no.
    Mismatch,
    /// Re-derives, not asked: a delete carries no document.
    NoDocument,
    /// Unobservable: the key's freshness, `freshness.v1`'s reason.
    Freshness(freshness::Reason),
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
            Self::Deleted => "deleted",
            Self::Stale => "stale",
            Self::Fresh => "fresh",
            Self::NotFiring => "not_firing",
            Self::Absent => "absent",
            Self::NotListed => "not_listed",
            Self::PresenceIncomplete => "presence_incomplete",
            Self::NoDescriptor => "no_descriptor",
            Self::FaceClosed => "face_closed",
            Self::NothingCrossed => "nothing_crossed",
            Self::AcrossFace => "across_face",
            Self::LastKnown => "last_known",
            Self::PreviousInstance => "previous_instance",
            Self::Incomplete => "incomplete",
            Self::NotRead => "not_read",
            Self::NotHeld => "not_held",
            Self::Undecodable => "undecodable",
            Self::Answered => "answered",
            Self::Window => "window",
            Self::NoSeed => "no_seed",
            Self::SeedIncomplete => "seed_incomplete",
            Self::NoMember => "no_member",
            Self::WindowLost => "window_lost",
            Self::WindowShort => "window_short",
            Self::OwnerGap => "owner_gap",
            Self::AnyFiring => "any_firing",
            Self::NoneFiring => "none_firing",
            Self::OwnerUnobservable => "owner_unobservable",
            Self::AlarmUnobservable => "alarm_unobservable",
            Self::NoOwner => "no_owner",
            Self::Strict => "strict",
            Self::Sanitized => "sanitized",
            Self::Mismatch => "mismatch",
            Self::NoDocument => "no_document",
            Self::Freshness(r) => r.as_str(),
        }
    }

    /// The reason in words, for a tool's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::Firing => "its owner says it fires, and confirmed it within 900 s",
            Self::Resolved => "its owner put it resolved",
            Self::Deleted => "its owner deleted it, which reads as resolved",
            Self::Stale => "its owner has not confirmed it within 900 s",
            Self::Fresh => "its owner confirmed it within 900 s",
            Self::NotFiring => "it is not firing",
            Self::Absent => "its owner is absent: presence's word, and its alarms are last-known",
            Self::NotListed => "its owner's descriptor does not list alarms.v1",
            Self::PresenceIncomplete => {
                "its owner's token was not seen, and the presence read may be incomplete or refused"
            }
            Self::NoDescriptor => "its owner's descriptor could not be read",
            Self::FaceClosed => "the constrained face does not let this reader reach its alarms",
            Self::NothingCrossed => "nothing of it has crossed the constrained face",
            Self::AcrossFace => {
                "its owner is across a constrained face, where presence does not cross"
            }
            Self::LastKnown => "only an archive's last-known word was read, never current",
            Self::PreviousInstance => {
                "the latest put is an earlier instance's, which the present one has not made its own"
            }
            Self::Incomplete => "deliveries were lost, and no GET answered since",
            Self::NotRead => "nothing of it was read",
            Self::NotHeld => "its owner's alarms, known whole, do not hold it",
            Self::Undecodable => "its latest put does not read as an alarms.v1 document",
            Self::Answered => "its owner answered a complete GET",
            Self::Window => {
                "a complete GET drew nothing, and a lossless window longer than 900 s heard no alarm"
            }
            Self::NoSeed => "no GET of its alarms was made",
            Self::SeedIncomplete => "the GET of its alarms ended at its timeout, or with an error",
            Self::NoMember => "the GET drew nothing, which is no verdict (core O5)",
            Self::WindowLost => "the window lost a delivery",
            Self::WindowShort => "the window was no longer than 900 s",
            Self::OwnerGap => "its owner was not present throughout the window",
            Self::AnyFiring => "an alarm at or above the severity fires",
            Self::NoneFiring => "no alarm at or above the severity fires",
            Self::OwnerUnobservable => "an owner's alarms are not known whole",
            Self::AlarmUnobservable => {
                "an alarm that may be at or above the severity is unobservable"
            }
            Self::NoOwner => "no owner in scope was asked",
            Self::Strict => "the key is the document's, by the strict recipe",
            Self::Sanitized => "the key is the document's, through the sanitizing step",
            Self::Mismatch => "the key is not the one its document derives",
            Self::NoDocument => "a delete carries no document",
            Self::Freshness(r) => r.says(),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An answer and its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged {
    pub answer: Answer,
    pub reason: Reason,
}

impl Judged {
    const fn new(answer: Answer, reason: Reason) -> Self {
        Self { answer, reason }
    }
}

/// §2.14's premises: presence, then the descriptor. `None` lets the
/// reading go on.
fn premises(p: Presence) -> Option<Judged> {
    use Answer::{NotAsked, Unobservable};
    match p {
        Presence::Absent => Some(Judged::new(NotAsked, Reason::Absent)),
        Presence::Incomplete => Some(Judged::new(Unobservable, Reason::PresenceIncomplete)),
        Presence::AcrossFace { reachable: false } => {
            Some(Judged::new(Unobservable, Reason::FaceClosed))
        }
        Presence::Present(Listing::NotListed) => Some(Judged::new(NotAsked, Reason::NotListed)),
        Presence::Present(Listing::Unread) => Some(Judged::new(Unobservable, Reason::NoDescriptor)),
        Presence::AcrossFace { reachable: true } | Presence::Present(Listing::Listed { .. }) => {
            None
        }
    }
}

/// A subscription lost deliveries, and no GET answered since (§2.14).
fn lost(observations: &[Observation]) -> bool {
    let lossy = observations.iter().any(|o| {
        matches!(
            o,
            Observation::Subscribed {
                complete: false,
                ..
            }
        )
    });
    let answered = observations
        .iter()
        .any(|o| matches!(o, Observation::Got { reply: Some(_), .. }));
    lossy && !answered
}

/// What the reading of a key established before its freshness was asked,
/// or its freshness. Shared by [`firing`] and [`stale_firing`].
enum Read {
    Done(Judged),
    /// The latest is a readable firing document, or a put that does not
    /// read, with the key's freshness.
    Judge {
        firing: bool,
        freshness: freshness::Judged,
    },
}

fn read(r: &Reading<'_>, not_firing: impl Fn(Reason) -> Judged) -> Read {
    use Answer::{NotAsked, Unobservable};
    let f = freshness::judge_all(&horizon(), r.observations);
    if f.verdict == freshness::Verdict::NotAsked && f.reason == freshness::Reason::LastKnown {
        return Read::Done(Judged::new(NotAsked, Reason::LastKnown));
    }
    if lost(r.observations) {
        return Read::Done(Judged::new(Unobservable, Reason::Incomplete));
    }
    match r.latest {
        None if r.held_whole => Read::Done(not_firing(Reason::NotHeld)),
        None => Read::Done(Judged::new(
            Unobservable,
            if matches!(r.presence, Presence::AcrossFace { .. }) {
                Reason::NothingCrossed
            } else {
                Reason::NotRead
            },
        )),
        Some(Latest::Deleted) => Read::Done(not_firing(Reason::Deleted)),
        Some(Latest::Resolved) => Read::Done(not_firing(Reason::Resolved)),
        Some(Latest::Firing | Latest::Undecodable) if r.previous_instance => {
            Read::Done(Judged::new(NotAsked, Reason::PreviousInstance))
        }
        Some(l @ (Latest::Firing | Latest::Undecodable)) => Read::Judge {
            firing: l == Latest::Firing,
            freshness: f,
        },
    }
}

/// §5's first question, "is this alarm firing?", by §2.14's order:
/// presence, the descriptor, an archive alone, a loss, the latest sample,
/// then its freshness at 900 s. Yes is the finding.
#[must_use]
pub fn firing(r: &Reading<'_>) -> Judged {
    use Answer::{No, Unobservable, Yes};
    if let Some(j) = premises(r.presence) {
        return j;
    }
    let (firing, f) = match read(r, |reason| Judged::new(No, reason)) {
        Read::Done(j) => return j,
        Read::Judge { firing, freshness } => (firing, freshness),
    };
    match f.verdict {
        freshness::Verdict::Fresh if firing => Judged::new(Yes, Reason::Firing),
        freshness::Verdict::Fresh => Judged::new(Unobservable, Reason::Undecodable),
        // Across a face nothing says the key exists until something
        // crosses (`freshness.v1` §2.5, "a member it knows").
        freshness::Verdict::Stale
            if f.reason == freshness::Reason::NoDelivery
                && matches!(r.presence, Presence::AcrossFace { .. }) =>
        {
            Judged::new(Unobservable, Reason::NothingCrossed)
        }
        freshness::Verdict::Stale => Judged::new(Unobservable, Reason::Stale),
        freshness::Verdict::Unobservable | freshness::Verdict::NotAsked => {
            Judged::new(Unobservable, Reason::Freshness(f.reason))
        }
    }
}

/// §5's last question, "is this firing alarm past its horizon while its
/// owner is present?", over the reading [`firing`] takes. Yes is the
/// finding, about the owner (§2.7).
#[must_use]
pub fn stale_firing(r: &Reading<'_>) -> Judged {
    use Answer::{No, NotAsked, Unobservable, Yes};
    if let Some(j) = premises(r.presence) {
        return j;
    }
    if matches!(r.presence, Presence::AcrossFace { .. }) {
        return Judged::new(Unobservable, Reason::AcrossFace);
    }
    let f = match read(r, |_| Judged::new(NotAsked, Reason::NotFiring)) {
        Read::Done(j) => return j,
        Read::Judge { firing: false, .. } => {
            return Judged::new(Unobservable, Reason::Undecodable);
        }
        Read::Judge { freshness, .. } => freshness,
    };
    match f.verdict {
        freshness::Verdict::Stale => Judged::new(Yes, Reason::Stale),
        freshness::Verdict::Fresh => Judged::new(No, Reason::Fresh),
        freshness::Verdict::Unobservable | freshness::Verdict::NotAsked => {
            Judged::new(Unobservable, Reason::Freshness(f.reason))
        }
    }
}

/// How a reader's S4 GET of an owner's alarms ended (§2.15, core §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seed {
    /// At the routers' final reply, with no error reply, with this many
    /// keys answered with a value. A `reply_del` is not one.
    Complete { members: usize },
    /// At its timeout, or with an error reply.
    Incomplete,
}

/// A reader's subscription window on an owner's alarms, declared before
/// its GET (core §4.2), as core §2.7 establishes one. After a loss, the
/// window starts again at the first complete GET that followed it, the
/// reader's re-seed (text 0.2, F-128): a loss before that GET is healed by
/// it, one after it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// How long it has listened, from its declaration plus its reply
    /// timeout (core §2.7), or since the re-seed that followed its latest
    /// loss.
    pub listened: Duration,
    /// It lost no delivery over the window, established as core §2.7 says
    /// (0.25, 0.27).
    pub lossless: bool,
    /// The owner held its instance token from the window's start until one
    /// lease after its end (core §2.7). Not asked across a face, where
    /// presence does not cross.
    pub present_throughout: bool,
}

/// Whether an owner's alarms are known whole (§2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetVerdict {
    Established,
    Unobservable,
    NotAsked,
}

impl SetVerdict {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Established => "established",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
        }
    }
}

/// A [`SetVerdict`] and its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetJudged {
    pub verdict: SetVerdict,
    pub reason: Reason,
}

/// Whether an owner's alarms are known whole, by §2.15's order (text 0.2,
/// F-130): §2.14's steps 1 and 2, with their reasons; the seed; a window
/// that lost a delivery, then a gap in presence, which is not asked across
/// a face; a seed the owner answered; then, for an empty seed, the face and
/// the window.
#[must_use]
pub fn owner_set(presence: Presence, seed: Option<Seed>, window: Option<Window>) -> SetJudged {
    let j = |verdict, reason| SetJudged { verdict, reason };
    use SetVerdict::{Established, NotAsked, Unobservable};
    if let Some(p) = premises(presence) {
        return j(
            match p.answer {
                Answer::NotAsked => NotAsked,
                _ => Unobservable,
            },
            p.reason,
        );
    }
    let members = match seed {
        None => return j(Unobservable, Reason::NoSeed),
        Some(Seed::Incomplete) => return j(Unobservable, Reason::SeedIncomplete),
        Some(Seed::Complete { members }) => members,
    };
    // Across a face presence does not cross, so whether the owner was
    // present throughout is not asked there (text 0.2, F-130).
    let across = matches!(presence, Presence::AcrossFace { .. });
    if let Some(w) = window {
        if !w.lossless {
            return j(Unobservable, Reason::WindowLost);
        }
        if !w.present_throughout && !across {
            return j(Unobservable, Reason::OwnerGap);
        }
    }
    if members > 0 {
        return j(Established, Reason::Answered);
    }
    if across {
        return j(Unobservable, Reason::AcrossFace);
    }
    match window {
        None => j(Unobservable, Reason::NoMember),
        Some(w) if w.listened <= TTL => j(Unobservable, Reason::WindowShort),
        Some(_) => j(Established, Reason::Window),
    }
}

/// One owner in a scope, as §5's second question reads it.
#[derive(Debug, Clone, Copy)]
pub struct OwnerAlarms<'a> {
    /// Whether its alarms are known whole ([`owner_set`]).
    pub set: SetJudged,
    /// Each alarm read of it: [`firing`]'s answer, and the severity of its
    /// latest document, `None` when none reads.
    pub alarms: &'a [(Judged, Option<Severity>)],
}

/// §5's second question over a scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnyFiring {
    pub answer: Answer,
    pub reason: Reason,
    /// The alarms firing at or above the severity.
    pub firing: usize,
    /// The owners, by their set's verdict.
    pub established: usize,
    pub unobservable: usize,
    pub not_asked: usize,
}

/// §5's second question, "is anything firing at or above `at_least` under
/// this selector?", over the owners in scope (§2.15): yes on any alarm
/// firing at or above it, whatever else is known; no only when every owner
/// asked is established and none of its alarms that may be at or above it
/// is unobservable. Yes is the finding.
#[must_use]
pub fn any_firing(at_least: Severity, owners: &[OwnerAlarms<'_>]) -> AnyFiring {
    let mut out = AnyFiring {
        answer: Answer::No,
        reason: Reason::NoneFiring,
        firing: 0,
        established: 0,
        unobservable: 0,
        not_asked: 0,
    };
    let mut alarm_doubt = false;
    for o in owners {
        match o.set.verdict {
            SetVerdict::Established => out.established += 1,
            SetVerdict::Unobservable => out.unobservable += 1,
            SetVerdict::NotAsked => out.not_asked += 1,
        }
        for (j, sev) in o.alarms {
            let high = sev.is_none_or(|s| s >= at_least);
            match j.answer {
                Answer::Yes if high && sev.is_some() => out.firing += 1,
                Answer::Yes | Answer::Unobservable
                    if high && o.set.verdict == SetVerdict::Established =>
                {
                    alarm_doubt = true;
                }
                _ => {}
            }
        }
    }
    (out.answer, out.reason) = if out.firing > 0 {
        (Answer::Yes, Reason::AnyFiring)
    } else if out.unobservable > 0 {
        (Answer::Unobservable, Reason::OwnerUnobservable)
    } else if alarm_doubt {
        (Answer::Unobservable, Reason::AlarmUnobservable)
    } else if out.established == 0 {
        (Answer::NotAsked, Reason::NoOwner)
    } else {
        (Answer::No, Reason::NoneFiring)
    };
    out
}

/// The payload a key holds, as §5's third question reads it.
#[derive(Debug, Clone, Copy)]
pub enum Payload<'a> {
    /// A document.
    Alarm(&'a Alarm),
    /// A put that does not read (§2.4).
    Unreadable,
    /// A delete: no document.
    Deleted,
}

/// §5's third question, "does this key re-derive from its document?"
/// (§2.2, §2.3), for the alert key the key carries (the template's value,
/// unslugged). No is the finding; a yes through the sanitizing step is the
/// owner's break of §2.3's SHOULD, which a tool MAY report.
#[must_use]
pub fn rederives(alert_key: &str, payload: Payload<'_>) -> Judged {
    match payload {
        Payload::Deleted => Judged::new(Answer::NotAsked, Reason::NoDocument),
        Payload::Unreadable => Judged::new(Answer::Unobservable, Reason::Undecodable),
        Payload::Alarm(a) => {
            let d = a.derive();
            if d.key != alert_key {
                Judged::new(Answer::No, Reason::Mismatch)
            } else if d.framed {
                Judged::new(Answer::Yes, Reason::Strict)
            } else {
                Judged::new(Answer::Yes, Reason::Sanitized)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_test_vector() {
        let labels = [("peer", "r2"), ("port", "eth0"), ("host", "h-3fa9c2d41b7e")];
        assert_eq!(
            alert_key("link_down", labels).as_deref(),
            Ok("a659f813308ad1da")
        );
        let d = derive("link_down", labels);
        assert_eq!(d.input, "link_down\npeer=r2\nport=eth0");
        assert_eq!(d.input.len(), 27);
        assert!(d.framed);
        assert_eq!(d.key, "a659f813308ad1da");
    }

    #[test]
    fn host_scoped_labels_drop_out_first() {
        assert!(is_host_scoped("host"));
        assert!(is_host_scoped("host.id"));
        assert!(is_host_scoped("host."));
        assert!(!is_host_scoped("hostname"));
        assert!(!is_host_scoped("Host"));
        let with = alert_key("r", [("port", "1"), ("host.id", "a\nb"), ("host.=", "")]);
        assert_eq!(with, alert_key("r", [("port", "1")]));
    }

    #[test]
    fn sanitizing_is_the_identity_on_framed_input() {
        let labels = [("b", "2"), ("a", "1"), ("host", "h")];
        let (rule, s) = sanitize("r", labels);
        assert_eq!(rule, "r");
        assert_eq!(s, vec![("b".into(), "2".into()), ("a".into(), "1".into())]);
        assert_eq!(
            derive("r", labels).key,
            alert_key("r", labels).expect("framed")
        );
    }

    #[test]
    fn unframed_input_is_sanitized_not_refused() {
        assert_eq!(
            alert_key("link\ndown", [("port", "22")]),
            Err(Unframed::Rule)
        );
        let d = derive("link\ndown", [("port", "22")]);
        assert!(!d.framed);
        assert_eq!(Ok(d.key), alert_key("link_down", [("port", "22")]));
        assert_eq!(derive("", []).input, "unnamed");
        assert_eq!(derive("\n", []).input, "_");
        assert_eq!(derive("x", [("", "1")]).input, "x\n_=1");
        // Two names that sanitize to one sort by value.
        assert_eq!(
            derive("x", [("a=b", "2"), ("a_b", "1")]).input,
            "x\na_b=1\na_b=2"
        );
    }

    fn doc(v: Value) -> Alarm {
        Alarm::from_json(&v).expect("a document")
    }

    fn firing_doc() -> Value {
        json!({"timestamp": 1000, "rule": "link_down", "severity": "warning",
               "state": "firing", "summary": "s", "labels": {"peer": "r2"}})
    }

    #[test]
    fn documents_read_as_the_schema_says() {
        let a = doc(firing_doc());
        assert_eq!(a.severity, Severity::Warning);
        assert!(a.check().is_empty());
        // An integral float is an integer (JSON Schema 2020-12).
        let mut v = firing_doc();
        v["timestamp"] = json!(1000.0);
        assert_eq!(doc(v).timestamp, 1000);
        // Unknown members are ignored.
        let mut v = firing_doc();
        v["later"] = json!({"x": 1});
        assert_eq!(doc(v), a);
        for (m, bad) in [
            ("severity", json!("major")),
            ("state", json!("acked")),
            ("timestamp", json!(1.5)),
            ("labels", json!({"a": 1})),
            ("observed_at_ms", Value::Null),
        ] {
            let mut v = firing_doc();
            v[m] = bad;
            assert_eq!(
                Alarm::from_json(&v).map_err(|e| e.as_str()),
                Err("schema"),
                "{m}"
            );
        }
        let mut v = firing_doc();
        v["timestamp"] = json!(-1);
        assert_eq!(Alarm::from_json(&v), Err(Invalid::TimeRange("timestamp")));
        // Serialized back, the document is the schema's.
        assert_eq!(serde_json::to_value(&a).unwrap(), firing_doc());
    }

    /// F-126 (text 0.2): a payload is strict JSON before it is a document.
    #[test]
    fn a_payload_is_strict_json_first() {
        let ok = br#"{"timestamp": 1000, "rule": "r", "severity": "info", "state": "firing", "summary": "s"}"#;
        assert_eq!(decode(ok).expect("reads").rule, "r");
        for (bad, why) in [
            (
                &br#"{"timestamp": 1, "rule": "r", "severity": "info", "severity": "critical", "state": "firing", "summary": "s"}"#[..],
                "a duplicate member",
            ),
            (
                br#"{"timestamp": 1, "rule": "r", "severity": "info", "state": "firing", "summary": "s", "labels": {"a": "1", "a": "2"}}"#,
                "a duplicate label",
            ),
            (
                br#"{"timestamp": NaN, "rule": "r", "severity": "info", "state": "firing", "summary": "s"}"#,
                "NaN",
            ),
            (
                br#"{"timestamp": 1, "rule": "\ud800", "severity": "info", "state": "firing", "summary": "s"}"#,
                "a lone surrogate",
            ),
            (b"\xff", "not UTF-8"),
            (
                br#"{"timestamp": 1, "rule": "r", "severity": "info", "state": "firing", "summary": "s"} x"#,
                "trailing text",
            ),
        ] {
            assert_eq!(decode(bad).map_err(|e| e.as_str()), Err("json"), "{why}");
        }
        let schema = br#"{"timestamp": 1, "rule": "r", "severity": "major", "state": "firing", "summary": "s"}"#;
        assert_eq!(decode(schema).map_err(|e| e.as_str()), Err("schema"));
    }

    #[test]
    fn a_documents_problems() {
        let mut v = firing_doc();
        v["observed_at_ms"] = json!(999);
        v["state"] = json!("resolved");
        v["labels"] = json!({"Peer": "r2", "a=b": "c"});
        let p: Vec<&str> = doc(v).check().iter().map(Problem::as_str).collect();
        assert_eq!(
            p,
            [
                "observed_before_timestamp",
                "observed_on_resolved",
                "unframed",
                "label_name_form",
                "label_name_form"
            ]
        );
        assert!(is_label_name("host.boot_id"));
        assert!(is_label_name("if_index"));
        assert!(!is_label_name("host."));
        assert!(!is_label_name("_x"));
        assert!(!is_label_name(""));
    }

    #[test]
    fn transitions_take_the_state_from_the_document() {
        let raise = doc(firing_doc());
        let mut escalate = raise.clone();
        escalate.severity = Severity::Critical;
        escalate.timestamp = 2000;
        let mut refresh = escalate.clone();
        refresh.summary = "t".into();
        refresh.observed_at_ms = Some(3000);
        let mut resolve = refresh.clone();
        resolve.state = State::Resolved;
        resolve.timestamp = 4000;
        resolve.observed_at_ms = None;
        let mut t = Track::default();
        let seen: Vec<Transition> = [
            Sample::Put(&raise),
            Sample::Put(&escalate),
            Sample::Put(&refresh),
            Sample::Put(&refresh),
            Sample::Put(&resolve),
            Sample::Delete,
            Sample::Put(&raise),
            Sample::Delete,
        ]
        .into_iter()
        .map(|s| t.apply(s))
        .collect();
        use Transition as T;
        assert_eq!(
            seen,
            [
                T::Raised,
                T::Escalated,
                T::Refreshed,
                T::Confirmed,
                T::Resolved,
                T::Retired,
                T::Raised,
                T::Resolved
            ]
        );
    }

    #[test]
    fn a_ref_is_the_alarms_key() {
        let r = AlarmRef {
            system: "h-3fa9c2d41b7e".into(),
            service: "snmp.router01".into(),
            alert_key: "a659f813308ad1da".into(),
        };
        let k = r.key().expect("a key");
        assert_eq!(
            k.as_str(),
            "zk2/h-3fa9c2d41b7e/snmp.router01/alarms.v1/state/alarms/a659f813308ad1da"
        );
        assert_eq!(AlarmRef::from_key(k.as_str()), Some(r));
        assert_eq!(AlarmRef::from_key("zk2/a/b/health.v1/state/status"), None);
    }

    fn sub(age: u64) -> Observation {
        Observation::Subscribed {
            last: Some(freshness::Last::Put(Duration::from_secs(age))),
            listened: Duration::from_secs(2000),
            complete: true,
        }
    }

    #[test]
    fn firing_and_stale() {
        let present = Presence::Present(Listing::Listed { token: true });
        let fresh = [sub(10)];
        let stale = [sub(1000)];
        let r = |obs, latest| Reading {
            presence: present,
            observations: obs,
            latest: Some(latest),
            previous_instance: false,
            held_whole: false,
        };
        assert_eq!(firing(&r(&fresh, Latest::Firing)).answer, Answer::Yes);
        assert_eq!(firing(&r(&stale, Latest::Firing)).reason, Reason::Stale);
        assert_eq!(stale_firing(&r(&stale, Latest::Firing)).answer, Answer::Yes);
        assert_eq!(firing(&r(&stale, Latest::Resolved)).answer, Answer::No);
        assert_eq!(
            firing(&Reading {
                presence: Presence::Absent,
                ..r(&fresh, Latest::Firing)
            })
            .reason,
            Reason::Absent
        );
    }

    #[test]
    fn a_scope_answers_no_only_when_known_whole() {
        let est = owner_set(
            Presence::Present(Listing::Listed { token: false }),
            Some(Seed::Complete { members: 0 }),
            Some(Window {
                listened: Duration::from_secs(901),
                lossless: true,
                present_throughout: true,
            }),
        );
        assert_eq!(est.verdict, SetVerdict::Established);
        let warn = [(
            Judged::new(Answer::Yes, Reason::Firing),
            Some(Severity::Warning),
        )];
        let a = any_firing(
            Severity::Critical,
            &[OwnerAlarms {
                set: est,
                alarms: &warn,
            }],
        );
        assert_eq!((a.answer, a.firing), (Answer::No, 0));
        let a = any_firing(
            Severity::Warning,
            &[OwnerAlarms {
                set: est,
                alarms: &warn,
            }],
        );
        assert_eq!((a.answer, a.firing), (Answer::Yes, 1));
        assert_eq!(any_firing(Severity::Info, &[]).answer, Answer::NotAsked);
    }
}
