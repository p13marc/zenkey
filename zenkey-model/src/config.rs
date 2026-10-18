//! `config.v1`: changing a running service's configuration
//! (`spec/profiles/config/v1.md`, #768).
//!
//! The profile's session-free half: the wire types of its standard contract
//! (§3), the validators every owner refuses with (§2.5, §2.10, §2.13,
//! §2.18), the window (§2.8), redaction (§2.15), and [`Owner`], a pure
//! state machine of one owner's configuration (§2.6 to §2.17), driven by
//! explicit calls, device results and instants, which the runtime wraps.
//!
//! ```text
//! validate_set(schema, facts, group, req)  ─▶ Admitted │ Refusal{code, field, app}      §2.5 rows 0, 1, 3–11
//! validate_control(op, facts, req)         ─▶ ()       │ Refusal                        §2.10, §2.13
//! check_desired(schema, doc)               ─▶ ()       │ DesiredRefusal{code, group, parameter}   §2.18
//! window(confirm_s, floor_s), deadline(t, w)                                            §2.8
//! log_request, device_detail                                                            §2.15
//!
//! Owner::set / confirm / cancel / extend ─▶ Step::Done{reply, events} │ Step::Act(Apply │ Undo)
//! Owner::due(now)                        ─▶ the act owed at now: the deadline's undo, or a reach apply
//! Owner::finish(now, DeviceResult)       ─▶ the act's outcome: the reply it owed, the events, or a next act
//! Owner::persist_plan / persisted        ─▶ the last change into the persisted layer
//! Owner::view / status                   ─▶ ConfigView │ ConfigStatus, its projection (§2.19)
//! Owner::converge(doc)                   ─▶ Wait │ Refused │ Set{group, request} │ Converged   §2.18
//! Owner::published(now)                  ─▶ the change events of the last hour, for the next instance §2.17
//! device_words(schema, act, words)       ─▶ the device's words as a log line may carry them    §2.15
//! ```
//!
//! There is no I/O and no clock here. The runtime receives calls, performs
//! each [`Act`] on its device, writes the persisted layer, keeps the timers
//! [`Owner::next_due`] names, and puts what [`Owner::view`],
//! [`Owner::status`] and the events say.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The profile's id, which is also its standard contract's interface id.
pub const PROFILE: &str = "config.v1";

/// The standard contract's interface id (§3).
pub const IFACE: &str = "config.v1";

/// The read-back's template (§2.3): `state/view`.
pub const VIEW: &str = "view";
/// The compact status's template (§2.19): `state/status`.
pub const STATUS: &str = "status";
/// The set operation's template (§2.5): `@op/set/{group}`.
pub const SET: &str = "set/{group}";
/// `@op/confirm` (§2.10).
pub const CONFIRM: &str = "confirm";
/// `@op/cancel` (§2.10).
pub const CANCEL: &str = "cancel";
/// `@op/extend` (§2.10).
pub const EXTEND: &str = "extend";
/// `@op/persist` (§2.13), optional.
pub const PERSIST: &str = "persist";
/// The change event's template (§2.17): `events/change/<ulid>`.
pub const CHANGE: &str = "change";

/// The view's and the status's horizon, their `freshness.ttl_s` (§2.3).
pub const TTL: Duration = Duration::from_secs(60);
/// The owner's bound between two confirmations of either (`freshness.v1`
/// §2.4).
pub const REFRESH: Duration = Duration::from_secs(30);
/// The contract's bound on a schema's groups, `set/{group}`'s cardinality
/// (§2.2).
pub const MAX_GROUPS: usize = 16;
/// How long an owner keeps an idempotency key's first answer (§2.14).
pub const KEY_HELD: Duration = Duration::from_secs(300);
/// The change event's rate, per [`RATE_PERIOD`] (§2.17).
pub const RATE: usize = 600;
/// The change event's rate period, an hour (§2.17, core §2.7).
pub const RATE_PERIOD: Duration = Duration::from_secs(3600);
/// What a log line writes for a sensitive value (§2.15).
pub const REDACTED: &str = "<redacted>";
/// A token's longest form, in characters (§2.6).
pub const MAX_TOKEN: usize = 64;
/// An idempotency key's longest form, in characters (§2.14).
pub const MAX_KEY: usize = 128;

// --- vocabulary (§4) ----------------------------------------------------------

/// A group's class (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// Takes effect at once; a `set` answers the read-back after the apply.
    Hot,
    /// Decides whether the owner reaches the bus: a window or a join is
    /// required, and the reply goes before the apply.
    Reach,
    /// What the owner's transport was started against: refused at runtime.
    Contract,
}

impl Class {
    /// The wire's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::Reach => "reach",
            Self::Contract => "contract",
        }
    }
}

/// Where a running value came from (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// The owner's built-in default.
    Default,
    /// Its startup file.
    File,
    /// Its persisted layer (§2.13).
    Overlay,
    /// A change in this instance.
    Runtime,
}

impl Source {
    /// The wire's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::File => "file",
            Self::Overlay => "overlay",
            Self::Runtime => "runtime",
        }
    }
}

/// How a change ended (§2.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Applied without a window, and permanent at once.
    Applied,
    /// A pending change kept by `confirm`.
    Confirmed,
    /// Undone: by `cancel`, at its deadline, or after its apply failed.
    RolledBack,
    /// Its undo failed: the read-back is the truth.
    Partial,
}

impl Outcome {
    /// The wire's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Confirmed => "confirmed",
            Self::RolledBack => "rolled-back",
            Self::Partial => "partial",
        }
    }
}

/// `ConfigError`'s code, the `app` detail of every operation (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    /// A contract group: set it in the startup configuration and restart.
    RestartRequired,
    /// The device refused; its words, when they may be carried, in `detail`.
    DeviceRefused,
}

impl ErrorCode {
    /// The wire's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RestartRequired => "restart-required",
            Self::DeviceRefused => "device-refused",
        }
    }
}

/// Which document a control operation's reply carries (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    /// The read-back, the default.
    #[default]
    View,
    /// The compact status (§2.19).
    Status,
}

/// What an owner did with the last desired document it acted on (§2.18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesiredOutcome {
    /// Every value it names is the running value.
    Converged,
    /// Refused whole, before anything applied.
    Refused,
    /// A `set` made to converge it was refused.
    Failed,
}

/// Why a desired document was not converged (§2.18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DesiredCode {
    /// It names a reach group.
    #[serde(rename = "reach")]
    Reach,
    /// It names a contract group.
    #[serde(rename = "contract")]
    Contract,
    /// It names a sensitive parameter.
    #[serde(rename = "sensitive")]
    Sensitive,
    /// A group or a parameter the schema does not declare.
    #[serde(rename = "not_found")]
    NotFound,
    /// A value of the wrong kind or out of bounds.
    #[serde(rename = "invalid_request")]
    InvalidRequest,
    /// A group outside the allowlist.
    #[serde(rename = "unavailable")]
    Unavailable,
    /// For `failed`: the event's rate was spent.
    #[serde(rename = "busy")]
    Busy,
    /// For `failed`: the device refused.
    #[serde(rename = "device-refused")]
    DeviceRefused,
}

impl DesiredCode {
    /// The wire's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reach => "reach",
            Self::Contract => "contract",
            Self::Sensitive => "sensitive",
            Self::NotFound => "not_found",
            Self::InvalidRequest => "invalid_request",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::DeviceRefused => "device-refused",
        }
    }
}

// --- the schema (§2.2) ----------------------------------------------------------

/// A parameter's kind, tagged by `kind` on the wire (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// A signed 64-bit integer, with optional inclusive bounds and a unit.
    Integer {
        /// The smallest value accepted, inclusive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        /// The largest value accepted, inclusive.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
        /// A word for a person: `s`, `By`, `Hz`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
    },
    /// Any string.
    Text,
}

/// Why a JSON value is not one of a kind's (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Misfit {
    /// Not a value of the kind: `1` for a bool, `2.0` for an integer, `null`.
    Kind,
    /// An integer below the declared minimum.
    Below(i64),
    /// An integer above the declared maximum.
    Above(i64),
    /// An integer beyond the signed 64-bit range.
    Range,
}

impl Kind {
    /// The kind's name on the wire.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Integer { .. } => "integer",
            Self::Text => "text",
        }
    }

    /// Reads `v` as a value of this kind, within its bounds (§2.5). An
    /// integer is a JSON number written without a fraction or an exponent.
    ///
    /// # Errors
    ///
    /// The [`Misfit`].
    pub fn check(&self, v: &Json) -> Result<Value, Misfit> {
        match (self, v) {
            (Self::Bool, Json::Bool(b)) => Ok(Value::Bool(*b)),
            (Self::Text, Json::String(s)) => Ok(Value::Text(s.clone())),
            (Self::Integer { min, max, .. }, Json::Number(n)) => {
                let Some(i) = n.as_i64() else {
                    return Err(if n.is_u64() {
                        Misfit::Range
                    } else {
                        Misfit::Kind
                    });
                };
                if let Some(m) = min
                    && i < *m
                {
                    return Err(Misfit::Below(*m));
                }
                if let Some(m) = max
                    && i > *m
                {
                    return Err(Misfit::Above(*m));
                }
                Ok(Value::Integer(i))
            }
            _ => Err(Misfit::Kind),
        }
    }
}

/// One parameter a group declares (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamDecl {
    /// Its name, unique in its group.
    pub name: String,
    /// Its kind.
    #[serde(flatten)]
    pub kind: Kind,
    /// One line for a person.
    pub description: String,
    /// Write-only (§2.15).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
}

/// One group a schema declares (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDecl {
    /// Its name, the `{group}` of `@op/set/{group}`.
    pub name: String,
    /// Its class (§2.4).
    pub class: Class,
    /// One line for a person.
    pub description: String,
    /// Whether the startup allowlist lets a `set` reach it (§2.16).
    pub writable: bool,
    /// Its parameters, in the owner's order.
    pub parameters: Vec<ParamDecl>,
}

impl GroupDecl {
    /// The parameter by name, if declared.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<&ParamDecl> {
        self.parameters.iter().find(|p| p.name == name)
    }

    fn is_sensitive(&self, name: &str) -> bool {
        self.parameter(name).is_none_or(|p| p.sensitive)
    }
}

/// An owner's schema: its groups, as the view serves them (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Schema {
    /// The groups, in the owner's order.
    pub groups: Vec<GroupDecl>,
}

impl Schema {
    /// The group by name, if declared. Names compare exactly.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&GroupDecl> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// The schema a view serves, without its values: what a tool checks a
    /// change against before it sends it (§2.5).
    #[must_use]
    pub fn of_view(view: &ConfigView) -> Self {
        Self {
            groups: view
                .groups
                .iter()
                .map(|g| GroupDecl {
                    name: g.name.clone(),
                    class: g.class,
                    description: g.description.clone(),
                    writable: g.writable,
                    parameters: g
                        .parameters
                        .iter()
                        .map(|p| ParamDecl {
                            name: p.name.clone(),
                            kind: p.kind.clone(),
                            description: p.description.clone(),
                            sensitive: p.sensitive,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

// --- values and time ------------------------------------------------------------------

/// One parameter's value, untagged on the wire: the kind says which (§3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// A `bool`'s.
    Bool(bool),
    /// An `integer`'s.
    Integer(i64),
    /// A `text`'s.
    Text(String),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Integer(i) => write!(f, "{i}"),
            Self::Text(t) => write!(f, "{t:?}"),
        }
    }
}

/// An instant on the owner's clock, in milliseconds since the Unix epoch
/// (§2.8). Negative before 1970.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(i64);

impl Time {
    /// The instant `ms` milliseconds after the Unix epoch.
    #[must_use]
    pub const fn from_millis(ms: i64) -> Self {
        Self(ms)
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// The instant `secs` seconds after the epoch, to the millisecond.
    #[must_use]
    pub fn from_secs_f64(secs: f64) -> Self {
        Self((secs * 1000.0).round() as i64)
    }

    /// This instant plus `d`, saturating.
    #[must_use]
    pub fn plus(self, d: Duration) -> Self {
        Self(
            self.0
                .saturating_add(i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
        )
    }

    /// This instant plus `secs` seconds, saturating.
    #[must_use]
    pub fn plus_secs(self, secs: u64) -> Self {
        self.plus(Duration::from_secs(secs))
    }

    /// Whether `self` is less than `d` after `earlier`.
    fn within(self, earlier: Self, d: Duration) -> bool {
        i128::from(self.0) - i128::from(earlier.0)
            < i128::try_from(d.as_millis()).unwrap_or(i128::MAX)
    }

    /// The owner's spelling (§2.8): RFC 3339 in UTC,
    /// `YYYY-MM-DDTHH:MM:SS.sssZ`, always three fraction digits.
    #[must_use]
    pub fn rfc3339(self) -> String {
        let days = self.0.div_euclid(86_400_000);
        let ms = self.0.rem_euclid(86_400_000);
        let (y, m, d) = civil_from_days(days);
        let (h, rest) = (ms / 3_600_000, ms % 3_600_000);
        let (mi, rest) = (rest / 60_000, rest % 60_000);
        let (s, frac) = (rest / 1000, rest % 1000);
        format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{frac:03}Z")
    }

    /// Reads any RFC 3339 date-time (§2.8): `Z` or an offset, a fraction of
    /// any length, kept to the millisecond (truncated). `None` when it is
    /// not one.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        let num = |r: std::ops::Range<usize>| -> Option<i64> {
            let t = s.get(r)?;
            if t.is_empty() || !t.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            t.parse().ok()
        };
        if b.len() < 20
            || b[4] != b'-'
            || b[7] != b'-'
            || !matches!(b[10], b'T' | b't')
            || b[13] != b':'
            || b[16] != b':'
        {
            return None;
        }
        let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
        let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
        if !(1..=12).contains(&mo)
            || d < 1
            || d > days_in_month(y, mo)
            || h > 23
            || mi > 59
            || sec > 60
        {
            return None;
        }
        let mut i = 19;
        let mut ms = 0;
        if b.get(i) == Some(&b'.') {
            let start = i + 1;
            i = start;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            if i == start {
                return None;
            }
            let digits = &s[start..i];
            let three: String = digits.chars().chain("000".chars()).take(3).collect();
            ms = three.parse::<i64>().ok()?;
        }
        let offset_min = match b.get(i)? {
            b'Z' | b'z' if i + 1 == b.len() => 0,
            sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
                let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
                if oh > 23 || om > 59 {
                    return None;
                }
                let o = oh * 60 + om;
                if *sign == b'-' { -o } else { o }
            }
            _ => return None,
        };
        let days = days_from_civil(y, mo, d);
        let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
        Some(Self(secs * 1000 + ms))
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.rfc3339())
    }
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day count since 1970-01-01 (`civil_from_days`).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// The window of a `confirm_s`, raised to the floor: `max(confirm_s,
/// floor_s)` seconds (§2.8).
#[must_use]
pub fn window(confirm_s: u64, floor_s: u64) -> u64 {
    confirm_s.max(floor_s)
}

/// The deadline of a window counted from `received` (§2.8).
#[must_use]
pub fn deadline(received: Time, window_s: u64) -> Time {
    received.plus_secs(window_s)
}

// --- the wire types (§3) --------------------------------------------------------------

/// One parameter as the view shows it (§2.3): its declaration, and what the
/// owner runs with. A sensitive parameter never carries `value` or
/// `startup`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameter {
    /// Its name.
    pub name: String,
    /// Its kind, with an integer's bounds and unit.
    #[serde(flatten)]
    pub kind: Kind,
    /// One line for a person.
    pub description: String,
    /// Write-only (§2.15).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    /// The running value; absent when sensitive or unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Where the running value came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    /// What a restart would run with, when it differs from a known running
    /// value; never for a sensitive parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<Value>,
}

/// One group as the view shows it (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupView {
    /// Its name.
    pub name: String,
    /// Its class.
    pub class: Class,
    /// One line for a person.
    pub description: String,
    /// Whether the allowlist lets a `set` reach it (§2.16).
    pub writable: bool,
    /// Its parameters, in the owner's order.
    pub parameters: Vec<Parameter>,
}

/// The pending change, as the view and the status show it (§2.7, §2.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    /// Its token.
    pub token: String,
    /// Its groups, in the order they first joined.
    pub groups: Vec<String>,
    /// When its undo runs, on the owner's clock (§2.8).
    pub deadline: String,
    /// Whether every group of it is applied.
    pub applied: bool,
}

/// The last change made permanent in this instance (§2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastChange {
    /// Its token.
    pub token: String,
    /// Its groups.
    pub groups: Vec<String>,
}

/// Why a desired document was not converged, and where (§2.18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredRefusal {
    /// The code.
    pub code: DesiredCode,
    /// The group.
    pub group: String,
    /// The parameter, for a parameter's check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
}

/// What the owner did with the last desired document it acted on (§2.18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredReport {
    /// The document's revision.
    pub observed_revision: u64,
    /// Converged, refused or failed.
    pub outcome: DesiredOutcome,
    /// Why, for refused and failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<DesiredRefusal>,
}

/// The read-back, `state/view` (§2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigView {
    /// The owner's address, `<system>/<service>`.
    pub service: String,
    /// Its instance id.
    pub instance: String,
    /// The count of acts that moved its running values.
    pub revision: u64,
    /// Its floor, in seconds (§2.8).
    pub confirm_floor_s: u64,
    /// The pending change, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// The last change, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change: Option<LastChange>,
    /// The desired report, for an owner that converges desired state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired: Option<DesiredReport>,
    /// The groups.
    pub groups: Vec<GroupView>,
}

/// The compact status, `state/status`: the view's projection (§2.19).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigStatus {
    /// The instance id.
    pub instance: String,
    /// The revision.
    pub revision: u64,
    /// The floor, in seconds.
    pub confirm_floor_s: u64,
    /// The pending change, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// The last change, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change: Option<LastChange>,
    /// The desired report, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired: Option<DesiredReport>,
    /// The values by group and name: no sensitive or unknown value, and no
    /// group left without one.
    pub values: BTreeMap<String, BTreeMap<String, Value>>,
}

impl ConfigStatus {
    /// The status a view projects to (§2.19).
    #[must_use]
    pub fn of(view: &ConfigView) -> Self {
        let mut values = BTreeMap::new();
        for g in &view.groups {
            let shown: BTreeMap<String, Value> = g
                .parameters
                .iter()
                .filter(|p| !p.sensitive)
                .filter_map(|p| p.value.clone().map(|v| (p.name.clone(), v)))
                .collect();
            if !shown.is_empty() {
                values.insert(g.name.clone(), shown);
            }
        }
        Self {
            instance: view.instance.clone(),
            revision: view.revision,
            confirm_floor_s: view.confirm_floor_s,
            pending: view.pending.clone(),
            last_change: view.last_change.clone(),
            desired: view.desired.clone(),
            values,
        }
    }
}

/// The body of `@op/set/{group}` (§2.5). Its values stay JSON until the
/// validator reads them against the schema, so a value of the wrong kind is
/// row 11's refusal, naming its parameter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SetRequest {
    /// Values for some parameters of the group.
    pub values: BTreeMap<String, Json>,
    /// The window, in seconds, at least 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_s: Option<u64>,
    /// The pending change's token, to join it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// The revision the caller read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    /// A key that makes a retry return the first answer (§2.14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Check, and answer the would-be view, touching nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dry_run: bool,
}

/// A reach `set`'s reply, before the apply (§2.9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingReply {
    /// The change's token.
    pub token: String,
    /// When the group applies, on the owner's clock.
    pub apply_at: String,
    /// When the change rolls back unless confirmed.
    pub deadline: String,
}

/// The reply to a `set` (§2.9, §3): `{applied: ConfigView}` or `{pending:
/// PendingReply}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SetReply {
    /// The read-back after the apply, or a dry run's would-be read-back.
    Applied(Box<ConfigView>),
    /// A reach group's pending reply.
    Pending(PendingReply),
}

/// The body of `confirm`, `cancel`, `extend` and `persist` (§2.10).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ControlRequest {
    /// The change's token.
    pub token: String,
    /// For `extend` alone: the new window from now, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_s: Option<u64>,
    /// The document the reply carries, the view by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
}

/// The reply to a control operation (§2.10): `{view: ConfigView}` or
/// `{status: ConfigStatus}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlReply {
    /// The read-back after the act.
    View(Box<ConfigView>),
    /// The compact status after the act.
    Status(Box<ConfigStatus>),
}

/// One parameter a change set, in its event (§2.17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edit {
    /// Its group.
    pub group: String,
    /// Its name.
    pub parameter: String,
    /// Its value before the change; absent when unknown or redacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<Value>,
    /// The value the change set; absent when redacted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Value>,
    /// A sensitive parameter's edit, with neither value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub redacted: bool,
}

/// One change as it ended, `events/change/<ulid>` (§2.17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigChangeEvent {
    /// The owner's address.
    pub service: String,
    /// Its instance id.
    pub instance: String,
    /// The revision after the act that ended the change.
    pub revision: u64,
    /// The change's token.
    pub token: String,
    /// How it ended.
    pub outcome: Outcome,
    /// Its edits.
    pub edits: Vec<Edit>,
    /// The actor the `set` that started it claimed (core O7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The request id it claimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Where the call came from, best effort, unauthenticated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_source: Option<String>,
}

/// The `app` detail of every operation (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigError {
    /// `restart-required` or `device-refused`.
    pub code: ErrorCode,
    /// For a person: the device's own words, when they may be carried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A desired document for a `config.v1` owner (§2.18). Its values stay JSON
/// until [`check_desired`] reads them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DesiredConfig {
    /// The author's revision.
    pub revision: u64,
    /// Values by group and parameter.
    pub values: BTreeMap<String, BTreeMap<String, Json>>,
}

// --- refusals ---------------------------------------------------------------------------

/// The core §5.2 code of a refusal this profile makes. `fanout_forbidden`
/// and `internal` are the runtime's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    /// `invalid_request`.
    InvalidRequest,
    /// `not_found`.
    NotFound,
    /// `unavailable`, always with cause `config` here.
    Unavailable,
    /// `forbidden`.
    Forbidden,
    /// `busy`.
    Busy,
    /// `app`, with a [`ConfigError`].
    App,
}

impl Code {
    /// The envelope's code.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::Forbidden => "forbidden",
            Self::Busy => "busy",
            Self::App => "app",
        }
    }
}

/// What a refusal is about, as the fixtures name it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Field {
    /// The key's `{group}`.
    Group,
    /// A member of the request, as a JSON pointer (RFC 6901).
    Member(String),
}

impl Field {
    /// `"group"`, or the pointer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Group => "group",
            Self::Member(p) => p,
        }
    }

    fn member(name: &str) -> Self {
        Self::Member(format!("/{name}"))
    }

    fn value(name: &str) -> Self {
        Self::Member(format!(
            "/values/{}",
            name.replace('~', "~0").replace('/', "~1")
        ))
    }
}

/// A refusal, before or after an act: the envelope's code, its `app`
/// detail, what it is about, and a message for a person that never carries
/// a sensitive value (§2.15).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The code.
    pub code: Code,
    /// The `ConfigError`, with [`Code::App`] alone.
    pub app: Option<ConfigError>,
    /// What the refusal is about: `None` for the owner's state or the
    /// operation.
    pub field: Option<Field>,
    /// For a person, never parsed.
    pub message: String,
}

impl Refusal {
    fn new(code: Code, field: Option<Field>, message: impl Into<String>) -> Self {
        Self {
            code,
            app: None,
            field,
            message: message.into(),
        }
    }

    fn app(code: ErrorCode, detail: Option<String>, message: impl Into<String>) -> Self {
        Self {
            code: Code::App,
            app: Some(ConfigError { code, detail }),
            field: Some(Field::Group),
            message: message.into(),
        }
    }

    fn busy(message: impl Into<String>) -> Self {
        Self::new(Code::Busy, None, message)
    }

    /// The envelope's cause: `config` for `unavailable`, else none.
    #[must_use]
    pub fn cause(&self) -> Option<&'static str> {
        (self.code == Code::Unavailable).then_some("config")
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for Refusal {}

// --- the validators (§2.5, §2.10, §2.13, §2.18) ----------------------------------------

/// What a validator needs of the owner's state.
#[derive(Debug, Clone, Copy, Default)]
pub struct Facts<'a> {
    /// The revision.
    pub revision: u64,
    /// The pending change, if any.
    pub pending: Option<PendingFacts<'a>>,
    /// The last change, if any.
    pub last_change: Option<LastFacts<'a>>,
    /// Whether the owner has a persisted layer.
    pub persist: bool,
    /// Whether an act is in flight.
    pub in_flight: bool,
}

/// The pending change, as a validator reads it.
#[derive(Debug, Clone, Copy)]
pub struct PendingFacts<'a> {
    /// Its token.
    pub token: &'a str,
    /// Whether every group of it is applied.
    pub applied: bool,
    /// Whether its deadline has passed with its undo not yet run.
    pub deadline_passed: bool,
}

/// The last change, as `persist` reads it.
#[derive(Debug, Clone, Copy)]
pub struct LastFacts<'a> {
    /// Its token.
    pub token: &'a str,
    /// Whether it set a sensitive parameter.
    pub set_sensitive: bool,
}

/// A `set` that passed §2.5's rows 0, 1 and 3 to 11.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// The group's class.
    pub class: Class,
    /// The values, read against their kinds.
    pub values: BTreeMap<String, Value>,
    /// Whether it joins the pending change.
    pub joins: bool,
    /// Whether it is a dry run.
    pub dry_run: bool,
}

/// A control operation (§2.10, §2.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlOp {
    /// `confirm`.
    Confirm,
    /// `cancel`.
    Cancel,
    /// `extend`.
    Extend,
    /// `persist`.
    Persist,
}

impl ControlOp {
    /// The operation's template.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => CONFIRM,
            Self::Cancel => CANCEL,
            Self::Extend => EXTEND,
            Self::Persist => PERSIST,
        }
    }
}

fn too_long(s: &str, max: usize) -> bool {
    s.is_empty() || s.chars().count() > max
}

fn check_set_shape(req: &SetRequest) -> Result<(), Refusal> {
    if req.confirm_s == Some(0) {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("confirm_s")),
            "confirm_s is a window of at least 1 s",
        ));
    }
    if let Some(t) = &req.token
        && too_long(t, MAX_TOKEN)
    {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("token")),
            format!("a token is 1 to {MAX_TOKEN} characters"),
        ));
    }
    if let Some(k) = &req.idempotency_key
        && too_long(k, MAX_KEY)
    {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("idempotency_key")),
            format!("an idempotency key is 1 to {MAX_KEY} characters"),
        ));
    }
    Ok(())
}

fn check_control_shape(req: &ControlRequest) -> Result<(), Refusal> {
    if too_long(&req.token, MAX_TOKEN) {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("token")),
            format!("a token is 1 to {MAX_TOKEN} characters"),
        ));
    }
    if req.confirm_s == Some(0) {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("confirm_s")),
            "confirm_s is a window of at least 1 s",
        ));
    }
    Ok(())
}

fn in_flight() -> Refusal {
    Refusal::busy("an act is in flight on this service's device; call again once it ends")
}

fn misfit_message(p: &ParamDecl, why: Misfit, v: &Json) -> String {
    let unit = match &p.kind {
        Kind::Integer { unit: Some(u), .. } => format!(" {u}"),
        _ => String::new(),
    };
    let shown = if p.sensitive {
        String::new()
    } else {
        format!(", got {v}")
    };
    match why {
        Misfit::Kind => format!(
            "{}: expected a value of kind {}{shown}",
            p.name,
            p.kind.as_str()
        ),
        Misfit::Below(m) => format!("{}: below the minimum of {m}{unit}{shown}", p.name),
        Misfit::Above(m) => format!("{}: above the maximum of {m}{unit}{shown}", p.name),
        Misfit::Range => format!("{}: beyond the signed 64-bit range{shown}", p.name),
    }
}

/// §2.5's rows 3 to 11: what a `set` is refused for by the schema and the
/// state, before the rate, a reach join's deadline and the device.
fn check_rows(
    schema: &Schema,
    facts: &Facts<'_>,
    group: &str,
    req: &SetRequest,
) -> Result<Admitted, Refusal> {
    let Some(g) = schema.group(group) else {
        return Err(Refusal::new(
            Code::NotFound,
            Some(Field::Group),
            format!("no group {group:?}: the view names every group this service accepts"),
        ));
    };
    if !g.writable {
        return Err(Refusal::new(
            Code::Unavailable,
            Some(Field::Group),
            format!("group {group:?} is outside this service's startup allowlist"),
        ));
    }
    if let Some(p) = &facts.pending
        && (req.token.as_deref() != Some(p.token) || p.deadline_passed)
    {
        return Err(Refusal::busy(format!(
            "a change is pending on this service (token {}); confirm, cancel or extend it, or carry its token",
            p.token
        )));
    }
    if req.token.is_some() && facts.pending.is_none() {
        return Err(Refusal::new(
            Code::NotFound,
            Some(Field::member("token")),
            "no change is pending: the token names nothing",
        ));
    }
    if let Some(e) = req.expected_revision
        && e != facts.revision
    {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("expected_revision")),
            format!(
                "the change expects revision {e} and the view is at {}; read it back and decide again",
                facts.revision
            ),
        ));
    }
    if g.class == Class::Contract {
        return Err(Refusal::app(
            ErrorCode::RestartRequired,
            None,
            format!(
                "group {group:?} is part of what this service was started against; set it in its startup \
                 configuration and restart"
            ),
        ));
    }
    if req.token.is_some() && req.confirm_s.is_some() {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("confirm_s")),
            "a change joining a pending one takes its window; drop confirm_s (extend moves the deadline)",
        ));
    }
    if g.class == Class::Reach && req.confirm_s.is_none() && req.token.is_none() && !req.dry_run {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("confirm_s")),
            format!(
                "group {group:?} can cut the link this reply travels on; send it with a window (confirm_s) and \
                 confirm over the new link"
            ),
        ));
    }
    let mut values = BTreeMap::new();
    for (name, v) in &req.values {
        let Some(p) = g.parameter(name) else {
            return Err(Refusal::new(
                Code::NotFound,
                Some(Field::value(name)),
                format!("group {group:?} declares no parameter {name:?}"),
            ));
        };
        match p.kind.check(v) {
            Ok(value) => {
                values.insert(name.clone(), value);
            }
            Err(why) => {
                return Err(Refusal::new(
                    Code::InvalidRequest,
                    Some(Field::value(name)),
                    misfit_message(p, why, v),
                ));
            }
        }
    }
    Ok(Admitted {
        class: g.class,
        values,
        joins: req.token.is_some(),
        dry_run: req.dry_run,
    })
}

/// Checks a `set` as §2.5 orders it: the request's own type (row 0), an act
/// in flight (row 1), then rows 3 to 11. Row 2, an idempotency key the owner
/// holds, needs the keys, and rows 12 to 14 the rate, the deadline and the
/// device: [`Owner::set`] adds them. A tool runs this against a view before
/// it sends (§2.5, "A caller that checks first").
///
/// # Errors
///
/// The first [`Refusal`] that applies.
pub fn validate_set(
    schema: &Schema,
    facts: &Facts<'_>,
    group: &str,
    req: &SetRequest,
) -> Result<Admitted, Refusal> {
    check_set_shape(req)?;
    if facts.in_flight {
        return Err(in_flight());
    }
    check_rows(schema, facts, group, req)
}

/// Checks a control operation as §2.10 and §2.13 order it.
///
/// # Errors
///
/// The first [`Refusal`] that applies.
pub fn validate_control(
    op: ControlOp,
    facts: &Facts<'_>,
    req: &ControlRequest,
) -> Result<(), Refusal> {
    if op == ControlOp::Persist && !facts.persist {
        return Err(Refusal::new(
            Code::Unavailable,
            None,
            "this service has no persisted layer: persist is unavailable here",
        ));
    }
    check_control_shape(req)?;
    if facts.in_flight {
        return Err(in_flight());
    }
    let window_wrong = match op {
        ControlOp::Extend => req.confirm_s.is_none(),
        _ => req.confirm_s.is_some(),
    };
    if window_wrong {
        return Err(Refusal::new(
            Code::InvalidRequest,
            Some(Field::member("confirm_s")),
            if op == ControlOp::Extend {
                "extend carries confirm_s, the new window from now".to_owned()
            } else {
                format!("{} carries no confirm_s", op.as_str())
            },
        ));
    }
    if op == ControlOp::Persist {
        if let Some(p) = &facts.pending
            && p.token == req.token
        {
            return Err(Refusal::busy(format!(
                "change {} is still pending; confirm it first",
                p.token
            )));
        }
        let Some(last) = facts.last_change.filter(|l| l.token == req.token) else {
            return Err(Refusal::new(
                Code::NotFound,
                Some(Field::member("token")),
                "persist takes the last change's token, and nothing older",
            ));
        };
        if last.set_sensitive {
            return Err(Refusal::new(
                Code::Forbidden,
                Some(Field::member("token")),
                "the last change set a sensitive value, which is never written to the persisted layer",
            ));
        }
        return Ok(());
    }
    let Some(p) = facts.pending.filter(|p| p.token == req.token) else {
        return Err(Refusal::new(
            Code::NotFound,
            Some(Field::member("token")),
            "no pending change by this token",
        ));
    };
    if p.deadline_passed {
        return Err(Refusal::busy(
            "the change's deadline has passed and its undo is running; it can no longer be kept",
        ));
    }
    if op == ControlOp::Confirm && !p.applied {
        return Err(Refusal::busy(
            "the change is not applied yet; confirm it once the view shows it applied",
        ));
    }
    Ok(())
}

/// Checks a desired document (§2.18): `Ok` when the owner may converge it,
/// else the first problem, in the profile's order.
///
/// # Errors
///
/// The [`DesiredRefusal`].
pub fn check_desired(schema: &Schema, doc: &DesiredConfig) -> Result<(), DesiredRefusal> {
    let refuse = |code, group: &str, parameter: Option<&str>| DesiredRefusal {
        code,
        group: group.to_owned(),
        parameter: parameter.map(str::to_owned),
    };
    for (gname, params) in &doc.values {
        let Some(g) = schema.group(gname) else {
            return Err(refuse(DesiredCode::NotFound, gname, None));
        };
        match g.class {
            Class::Contract => return Err(refuse(DesiredCode::Contract, gname, None)),
            Class::Reach => return Err(refuse(DesiredCode::Reach, gname, None)),
            Class::Hot => {}
        }
        if !g.writable {
            return Err(refuse(DesiredCode::Unavailable, gname, None));
        }
        for (pname, v) in params {
            let Some(p) = g.parameter(pname) else {
                return Err(refuse(DesiredCode::NotFound, gname, Some(pname)));
            };
            if p.sensitive {
                return Err(refuse(DesiredCode::Sensitive, gname, Some(pname)));
            }
            if p.kind.check(v).is_err() {
                return Err(refuse(DesiredCode::InvalidRequest, gname, Some(pname)));
            }
        }
    }
    Ok(())
}

// --- redaction (§2.15) -------------------------------------------------------------------

/// A `set` request as a log line may carry it (§2.15): every value whose
/// parameter is sensitive, or that the owner cannot tell is not (an unknown
/// group or parameter), written [`REDACTED`].
#[must_use]
pub fn log_request(schema: &Schema, group: &str, req: &SetRequest) -> Json {
    let g = schema.group(group);
    let mut out = serde_json::to_value(req).unwrap_or(Json::Null);
    if let Some(values) = out.get_mut("values").and_then(Json::as_object_mut) {
        for (name, v) in values.iter_mut() {
            if g.is_none_or(|g| g.is_sensitive(name)) {
                *v = Json::String(REDACTED.to_owned());
            }
        }
    }
    out
}

/// Whether a `set` carries a value whose parameter is sensitive, or one the
/// owner cannot tell is not.
#[must_use]
pub fn carries_sensitive(schema: &Schema, group: &str, req: &SetRequest) -> bool {
    let g = schema.group(group);
    req.values
        .keys()
        .any(|n| g.is_none_or(|g| g.is_sensitive(n)))
}

/// The `detail` of a `device-refused` envelope (§2.15): the device's
/// words, left out when the change carries a sensitive value, since they
/// may repeat it.
#[must_use]
pub fn device_detail(
    schema: &Schema,
    group: &str,
    req: &SetRequest,
    words: Option<&str>,
) -> Option<String> {
    if carries_sensitive(schema, group, req) {
        None
    } else {
        words.map(str::to_owned)
    }
}

// --- the owner (§2.6 to §2.17) -----------------------------------------------------------

/// The call metadata a call claimed (core O7): never authentication.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Call {
    /// The claimed actor.
    pub actor: Option<String>,
    /// The claimed request id.
    pub request_id: Option<String>,
}

/// What an owner starts from: its address, schema, startup layers and
/// device facts.
#[derive(Debug, Clone)]
pub struct Start {
    /// The owner's address, `<system>/<service>`.
    pub service: String,
    /// The instance id; tokens are minted `<instance>-<n>`.
    pub instance: String,
    /// The schema.
    pub schema: Schema,
    /// The startup file's and the defaults' values, `{group: {parameter:
    /// (value, source)}}`, each source `file` or `default`. A parameter
    /// absent here starts unknown.
    pub startup: BTreeMap<String, BTreeMap<String, (Value, Source)>>,
    /// The persisted layer, `{group: {parameter: value}}`.
    pub overlay: BTreeMap<String, BTreeMap<String, Value>>,
    /// The floor, in seconds, at least 1 (§2.8).
    pub floor_s: u64,
    /// How long after receiving a reach `set` the device applies it: the
    /// `apply_at` it states (§2.9). Less than the floor, so that a new
    /// reach change applies before its deadline.
    pub apply_after: Duration,
    /// Whether the owner has a persisted layer (§2.13).
    pub persist: bool,
    /// The change events already published, for the rate (§2.17).
    pub events: Vec<Time>,
}

/// Why [`Owner::start`] refused a [`Start`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError(pub String);

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StartError {}

/// A device act the runtime performs, then reports with
/// [`Owner::finish`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Apply these values of one group.
    Apply {
        /// The group.
        group: String,
        /// The values, by parameter.
        values: BTreeMap<String, Value>,
    },
    /// The undo, once: restore each parameter to its value before the
    /// change (§2.11).
    Undo {
        /// What to restore.
        restore: Vec<Restore>,
    },
}

/// One parameter an undo restores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    /// Its group.
    pub group: String,
    /// Its name.
    pub parameter: String,
    /// Its value before the change.
    pub value: Value,
}

/// What the device said of an act. An apply reported `Failed` reads as
/// refused, and an undo reported `Refused` as failed with nothing read
/// back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceResult {
    /// Done.
    Ok,
    /// An apply refused, in the device's words.
    Refused(String),
    /// An undo failed, in the device's words, with what the owner read
    /// back: `{group: {parameter: value}}`, a parameter not in it unknown.
    Failed {
        /// The device's words.
        words: String,
        /// What was read back.
        readback: BTreeMap<String, BTreeMap<String, Value>>,
    },
}

impl Act {
    /// Whether the act sets a value whose parameter is sensitive, or one
    /// the owner cannot tell is not (an unknown group or parameter): an
    /// apply of such a value, or an undo restoring one (§2.15).
    #[must_use]
    pub fn carries_sensitive(&self, schema: &Schema) -> bool {
        let sensitive = |g: &str, p: &str| schema.group(g).is_none_or(|g| g.is_sensitive(p));
        match self {
            Self::Apply { group, values } => values.keys().any(|p| sensitive(group, p)),
            Self::Undo { restore } => restore.iter().any(|r| sensitive(&r.group, &r.parameter)),
        }
    }
}

/// The device's words about `act` as a log line may carry them (§2.15):
/// left out when the act sets a sensitive value, since they may repeat it.
#[must_use]
pub fn device_words<'w>(schema: &Schema, act: &Act, words: &'w str) -> Option<&'w str> {
    (!act.carries_sensitive(schema)).then_some(words)
}

/// What converging a desired document asks of the owner next (§2.18),
/// from [`Owner::converge`].
#[derive(Debug, Clone, PartialEq)]
pub enum Converge {
    /// A change is pending, or an act is in flight: wait until it ends.
    Wait,
    /// Refused whole, before anything applied: the report to carry.
    Refused(DesiredReport),
    /// One `set` without a window, on the first group by bytewise name
    /// whose values differ from the document: its own change, `applied`.
    /// A refusal of it is the report [`desired_failed`] makes.
    Set {
        /// The group.
        group: String,
        /// The request: the values that differ, and nothing else.
        request: SetRequest,
    },
    /// Every value the document names is the running value.
    Converged(DesiredReport),
}

/// The report of a document whose converging `set` on `group` was refused
/// (§2.18): `failed`, with that refusal's code (`busy` for the rate,
/// `device-refused`) and group.
#[must_use]
pub fn desired_failed(revision: u64, group: &str, refusal: &Refusal) -> DesiredReport {
    let code = match refusal.code {
        Code::Busy => DesiredCode::Busy,
        Code::App => DesiredCode::DeviceRefused,
        Code::InvalidRequest => DesiredCode::InvalidRequest,
        Code::NotFound => DesiredCode::NotFound,
        // A set never answers `forbidden`; and `unavailable` is the
        // allowlist's, which check_desired refused first.
        Code::Unavailable | Code::Forbidden => DesiredCode::Unavailable,
    };
    DesiredReport {
        observed_revision: revision,
        outcome: DesiredOutcome::Failed,
        refusal: Some(DesiredRefusal {
            code,
            group: group.to_owned(),
            parameter: None,
        }),
    }
}

/// A reply an owner sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// To a `set`.
    Set(SetReply),
    /// To a control operation.
    Control(ControlReply),
}

impl Reply {
    /// The reply's JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Set(r) => serde_json::to_value(r),
            Self::Control(r) => serde_json::to_value(r),
        }
        .unwrap_or(Json::Null)
    }
}

/// An answered step: the reply owed to the call, if any, and the change
/// events to publish, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Done {
    /// The reply, or the refusal, owed to the call; `None` when no call is
    /// answered (a deadline, a reach apply).
    pub reply: Option<Result<Reply, Refusal>>,
    /// The events to publish.
    pub events: Vec<ConfigChangeEvent>,
}

/// What the owner does next: answer, or have the runtime act on the device
/// and report back with [`Owner::finish`]. A call answered with an act
/// gets its reply from the act's finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Answered now.
    Done(Done),
    /// An act to perform first.
    Act(Act),
}

/// A `persist` the owner admitted: the entries the runtime writes, then
/// commits with [`Owner::persisted`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistPlan {
    /// The last change's token.
    pub token: String,
    /// The values to write, `{group: {parameter: value}}`.
    pub entries: BTreeMap<String, BTreeMap<String, Value>>,
    answer: Answer,
}

type Key = (String, String);

#[derive(Debug, Clone)]
struct Slot {
    value: Option<Value>,
    source: Option<Source>,
}

#[derive(Debug, Clone)]
struct EditRec {
    old: Option<Value>,
    old_source: Option<Source>,
    new: Value,
    applied: bool,
    sensitive: bool,
}

#[derive(Debug, Clone)]
struct Wait {
    group: String,
    values: BTreeMap<String, Value>,
    apply_at: Time,
}

#[derive(Debug, Clone)]
struct Change {
    token: String,
    groups: Vec<String>,
    deadline: Time,
    call: Call,
    edits: BTreeMap<Key, EditRec>,
    waiting: Vec<Wait>,
}

#[derive(Debug, Clone)]
struct Last {
    token: String,
    groups: Vec<String>,
    set: Vec<Key>,
    sensitive: bool,
}

#[derive(Debug, Clone)]
struct Held {
    key: String,
    group: String,
    request: SetRequest,
    answer: Result<Reply, Refusal>,
    at: Time,
}

#[derive(Debug, Clone, Copy)]
enum UndoWhy {
    Deadline,
    Cancel(Answer),
    Failed,
}

#[derive(Debug, Clone)]
enum Flight {
    Set {
        group: String,
        request: SetRequest,
        call: Call,
        values: BTreeMap<String, Value>,
        received: Time,
    },
    Apply(Wait),
    Undo {
        why: UndoWhy,
        plan: Vec<Key>,
    },
}

/// One owner's configuration, as a pure state machine (§2.6 to §2.17).
///
/// Every method takes the instant it acts at. A call is answered by a
/// [`Step`]: [`Step::Done`] now, or [`Step::Act`], an act the runtime
/// performs on the device before it calls [`Owner::finish`] with the
/// result. While an act is in flight, every call is `busy` (§2.7). The
/// runtime calls [`Owner::due`] at [`Owner::next_due`], for the deadline's
/// undo and each reach apply.
#[derive(Debug, Clone)]
pub struct Owner {
    service: String,
    instance: String,
    schema: Schema,
    floor_s: u64,
    apply_after: Duration,
    persist: bool,
    file: BTreeMap<Key, (Value, Source)>,
    overlay: BTreeMap<Key, Value>,
    slots: BTreeMap<Key, Slot>,
    revision: u64,
    minted: u64,
    pending: Option<Change>,
    last: Option<Last>,
    keys: Vec<Held>,
    events: Vec<Time>,
    flight: Option<Flight>,
    desired: Option<DesiredReport>,
}

fn key(g: &str, p: &str) -> Key {
    (g.to_owned(), p.to_owned())
}

fn flat<T: Clone>(m: &BTreeMap<String, BTreeMap<String, T>>) -> BTreeMap<Key, T> {
    m.iter()
        .flat_map(|(g, ps)| ps.iter().map(move |(p, v)| (key(g, p), v.clone())))
        .collect()
}

impl Owner {
    /// Starts an owner from its startup values and its persisted layer,
    /// never from a runtime change (§2.12): revision 0, nothing pending.
    ///
    /// # Errors
    ///
    /// A [`StartError`] for a floor of 0, an `apply_after` not below the
    /// floor, more than 16 groups, a name declared twice, bounds that cross,
    /// a startup source other than `file` or `default`, or a layer naming a
    /// parameter the schema does not declare, or a sensitive one in the
    /// persisted layer.
    pub fn start(s: Start) -> Result<Self, StartError> {
        let fail = |m: String| Err(StartError(m));
        if s.floor_s == 0 {
            return fail("the floor is at least 1 s".into());
        }
        if s.apply_after >= Duration::from_secs(s.floor_s) {
            return fail("apply_after must be below the floor, so that a reach change applies before its deadline".into());
        }
        if s.schema.groups.len() > MAX_GROUPS {
            return fail(format!(
                "{} groups, above the contract's {MAX_GROUPS}",
                s.schema.groups.len()
            ));
        }
        for (i, g) in s.schema.groups.iter().enumerate() {
            if g.name.is_empty() || s.schema.groups[..i].iter().any(|h| h.name == g.name) {
                return fail(format!("group {:?} is empty or declared twice", g.name));
            }
            for (j, p) in g.parameters.iter().enumerate() {
                if p.name.is_empty() || g.parameters[..j].iter().any(|q| q.name == p.name) {
                    return fail(format!(
                        "parameter {:?} of {:?} is empty or declared twice",
                        p.name, g.name
                    ));
                }
                if let Kind::Integer {
                    min: Some(a),
                    max: Some(b),
                    ..
                } = p.kind
                    && a > b
                {
                    return fail(format!("{}.{}: min above max", g.name, p.name));
                }
            }
        }
        let file = flat(&s.startup);
        let overlay = flat(&s.overlay);
        for ((g, p), (v, src)) in &file {
            let Some(d) = s.schema.group(g).and_then(|gd| gd.parameter(p)) else {
                return fail(format!(
                    "the startup layer names {g}.{p}, which the schema does not declare"
                ));
            };
            if !matches!(src, Source::File | Source::Default) {
                return fail(format!(
                    "{g}.{p}: a startup value comes from the file or a default"
                ));
            }
            if d.kind
                .check(&serde_json::to_value(v).unwrap_or(Json::Null))
                .is_err()
            {
                return fail(format!(
                    "{g}.{p}: its startup value is not of its kind or bounds"
                ));
            }
        }
        for ((g, p), v) in &overlay {
            match s.schema.group(g).and_then(|gd| gd.parameter(p)) {
                None => {
                    return fail(format!(
                        "the persisted layer names {g}.{p}, which the schema does not declare"
                    ));
                }
                Some(d) if d.sensitive => {
                    return fail(format!("{g}.{p} is sensitive, never persisted"));
                }
                Some(d)
                    if d.kind
                        .check(&serde_json::to_value(v).unwrap_or(Json::Null))
                        .is_err() =>
                {
                    return fail(format!(
                        "{g}.{p}: its persisted value is not of its kind or bounds"
                    ));
                }
                Some(_) => {}
            }
        }
        let mut slots = BTreeMap::new();
        for g in &s.schema.groups {
            for p in &g.parameters {
                let k = key(&g.name, &p.name);
                let slot = match (overlay.get(&k), file.get(&k)) {
                    (Some(v), _) => Slot {
                        value: Some(v.clone()),
                        source: Some(Source::Overlay),
                    },
                    (None, Some((v, src))) => Slot {
                        value: Some(v.clone()),
                        source: Some(*src),
                    },
                    (None, None) => Slot {
                        value: None,
                        source: None,
                    },
                };
                slots.insert(k, slot);
            }
        }
        Ok(Self {
            service: s.service,
            instance: s.instance,
            schema: s.schema,
            floor_s: s.floor_s,
            apply_after: s.apply_after,
            persist: s.persist,
            file,
            overlay,
            slots,
            revision: 0,
            minted: 0,
            pending: None,
            last: None,
            keys: Vec::new(),
            events: s.events,
            flight: None,
            desired: None,
        })
    }

    /// The owner after a restart, as a new instance (§2.12): its startup
    /// values and persisted layer, revision 0, nothing pending, no last
    /// change, no key held. The persisted layer and the published events
    /// survive it.
    #[must_use]
    pub fn restart(&self, instance: impl Into<String>) -> Self {
        let unflat = |m: &BTreeMap<Key, (Value, Source)>| {
            let mut out: BTreeMap<String, BTreeMap<String, (Value, Source)>> = BTreeMap::new();
            for ((g, p), v) in m {
                out.entry(g.clone())
                    .or_default()
                    .insert(p.clone(), v.clone());
            }
            out
        };
        let mut overlay: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        for ((g, p), v) in &self.overlay {
            overlay
                .entry(g.clone())
                .or_default()
                .insert(p.clone(), v.clone());
        }
        Self::start(Start {
            service: self.service.clone(),
            instance: instance.into(),
            schema: self.schema.clone(),
            startup: unflat(&self.file),
            overlay,
            floor_s: self.floor_s,
            apply_after: self.apply_after,
            persist: self.persist,
            events: self.events.clone(),
        })
        .expect("a start that succeeded once succeeds again")
    }

    /// The schema.
    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The instance id.
    #[must_use]
    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// The revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether an act is in flight.
    #[must_use]
    pub fn in_flight(&self) -> bool {
        self.flight.is_some()
    }

    /// The next instant [`Owner::due`] has something to do: the earliest
    /// reach `apply_at`, or the deadline.
    #[must_use]
    pub fn next_due(&self) -> Option<Time> {
        let ch = self.pending.as_ref()?;
        let apply = ch.waiting.iter().map(|w| w.apply_at).min();
        Some(apply.map_or(ch.deadline, |a| a.min(ch.deadline)))
    }

    /// The pending change's deadline, if a change is pending.
    #[must_use]
    pub fn deadline(&self) -> Option<Time> {
        self.pending.as_ref().map(|c| c.deadline)
    }

    /// Sets the desired report the view and the status carry (§2.18).
    pub fn report_desired(&mut self, report: Option<DesiredReport>) {
        self.desired = report;
    }

    /// The desired report the view and the status carry (§2.18).
    #[must_use]
    pub fn desired(&self) -> Option<&DesiredReport> {
        self.desired.as_ref()
    }

    /// The instants of the change events published within the last hour
    /// at `now`, oldest first (§2.17): what the next instance starts from
    /// ([`Start::events`]), so that the event's rate, which a re-mint does
    /// not reset (core §2.7), holds across a restart.
    #[must_use]
    pub fn published(&self, now: Time) -> Vec<Time> {
        let mut out: Vec<Time> = self
            .events
            .iter()
            .copied()
            .filter(|e| now.within(*e, RATE_PERIOD))
            .collect();
        out.sort();
        out
    }

    /// What converging `doc` asks next (§2.18): wait while a change is
    /// pending or an act is in flight; the document refused whole
    /// ([`check_desired`]); one `set` without a window on the first group,
    /// in the bytewise order of the group names, whose values differ from
    /// the running ones; or converged. Called again after each `set`, it
    /// moves to the next group, since the one applied no longer differs.
    #[must_use]
    pub fn converge(&self, doc: &DesiredConfig) -> Converge {
        if self.pending.is_some() || self.flight.is_some() {
            return Converge::Wait;
        }
        if let Err(refusal) = check_desired(&self.schema, doc) {
            return Converge::Refused(DesiredReport {
                observed_revision: doc.revision,
                outcome: DesiredOutcome::Refused,
                refusal: Some(refusal),
            });
        }
        for (g, params) in &doc.values {
            let Some(decl) = self.schema.group(g) else {
                continue;
            };
            let differ: BTreeMap<String, Json> = params
                .iter()
                .filter(|(p, v)| {
                    let wanted = decl.parameter(p).and_then(|d| d.kind.check(v).ok());
                    let running = self.slots.get(&key(g, p)).and_then(|s| s.value.as_ref());
                    wanted.as_ref() != running
                })
                .map(|(p, v)| (p.clone(), v.clone()))
                .collect();
            if !differ.is_empty() {
                return Converge::Set {
                    group: g.clone(),
                    request: SetRequest {
                        values: differ,
                        ..SetRequest::default()
                    },
                };
            }
        }
        Converge::Converged(DesiredReport {
            observed_revision: doc.revision,
            outcome: DesiredOutcome::Converged,
            refusal: None,
        })
    }

    /// What the validators read of the owner's state at `now`.
    #[must_use]
    pub fn facts(&self, now: Time) -> Facts<'_> {
        Facts {
            revision: self.revision,
            pending: self.pending.as_ref().map(|c| PendingFacts {
                token: &c.token,
                applied: c.waiting.is_empty(),
                deadline_passed: now >= c.deadline,
            }),
            last_change: self.last.as_ref().map(|l| LastFacts {
                token: &l.token,
                set_sensitive: l.sensitive,
            }),
            persist: self.persist,
            in_flight: self.flight.is_some(),
        }
    }

    fn startup_value(&self, k: &Key) -> Option<(Value, Source)> {
        self.overlay
            .get(k)
            .map(|v| (v.clone(), Source::Overlay))
            .or_else(|| self.file.get(k).cloned())
    }

    fn render(&self, slots: &BTreeMap<Key, Slot>) -> ConfigView {
        let groups = self
            .schema
            .groups
            .iter()
            .map(|g| GroupView {
                name: g.name.clone(),
                class: g.class,
                description: g.description.clone(),
                writable: g.writable,
                parameters: g
                    .parameters
                    .iter()
                    .map(|d| {
                        let k = key(&g.name, &d.name);
                        let slot = slots.get(&k);
                        let value = slot.and_then(|s| s.value.clone());
                        let startup = self.startup_value(&k).map(|(v, _)| v);
                        Parameter {
                            name: d.name.clone(),
                            kind: d.kind.clone(),
                            description: d.description.clone(),
                            sensitive: d.sensitive,
                            value: if d.sensitive { None } else { value.clone() },
                            source: slot.and_then(|s| s.source),
                            startup: match (d.sensitive, value, startup) {
                                (false, Some(v), Some(s)) if v != s => Some(s),
                                _ => None,
                            },
                        }
                    })
                    .collect(),
            })
            .collect();
        ConfigView {
            service: self.service.clone(),
            instance: self.instance.clone(),
            revision: self.revision,
            confirm_floor_s: self.floor_s,
            pending: self.pending.as_ref().map(|c| Pending {
                token: c.token.clone(),
                groups: c.groups.clone(),
                deadline: c.deadline.rfc3339(),
                applied: c.waiting.is_empty(),
            }),
            last_change: self.last.as_ref().map(|l| LastChange {
                token: l.token.clone(),
                groups: l.groups.clone(),
            }),
            desired: self.desired.clone(),
            groups,
        }
    }

    /// The read-back (§2.3).
    #[must_use]
    pub fn view(&self) -> ConfigView {
        self.render(&self.slots)
    }

    /// The compact status, the view's projection (§2.19).
    #[must_use]
    pub fn status(&self) -> ConfigStatus {
        ConfigStatus::of(&self.view())
    }

    fn control_reply(&self, answer: Answer) -> Reply {
        let view = self.view();
        Reply::Control(match answer {
            Answer::View => ControlReply::View(Box::new(view)),
            Answer::Status => ControlReply::Status(Box::new(ConfigStatus::of(&view))),
        })
    }

    fn mint(&mut self) -> String {
        self.minted += 1;
        format!("{}-{}", self.instance, self.minted)
    }

    fn rate_spent(&self, now: Time) -> bool {
        self.events
            .iter()
            .filter(|e| now.within(**e, RATE_PERIOD))
            .count()
            >= RATE
    }

    fn edits(&self, groups: &[String], edits: &BTreeMap<Key, EditRec>) -> Vec<Edit> {
        groups
            .iter()
            .flat_map(|g| {
                edits
                    .iter()
                    .filter(move |((eg, _), _)| eg == g)
                    .map(|((eg, ep), r)| {
                        if r.sensitive {
                            Edit {
                                group: eg.clone(),
                                parameter: ep.clone(),
                                old: None,
                                new: None,
                                redacted: true,
                            }
                        } else {
                            Edit {
                                group: eg.clone(),
                                parameter: ep.clone(),
                                old: r.old.clone(),
                                new: Some(r.new.clone()),
                                redacted: false,
                            }
                        }
                    })
            })
            .collect()
    }

    fn event(
        &mut self,
        now: Time,
        token: &str,
        outcome: Outcome,
        edits: Vec<Edit>,
        call: &Call,
    ) -> ConfigChangeEvent {
        self.events.retain(|e| now.within(*e, RATE_PERIOD));
        self.events.push(now);
        ConfigChangeEvent {
            service: self.service.clone(),
            instance: self.instance.clone(),
            revision: self.revision,
            token: token.to_owned(),
            outcome,
            edits,
            actor: call.actor.clone(),
            request_id: call.request_id.clone(),
            claimed_source: None,
        }
    }

    fn is_sensitive(&self, k: &Key) -> bool {
        self.schema
            .group(&k.0)
            .is_some_and(|g| g.is_sensitive(&k.1))
    }

    /// Records the values a change sets, keeping each parameter's value
    /// before the change from its first set.
    fn record(
        &self,
        edits: &mut BTreeMap<Key, EditRec>,
        group: &str,
        values: &BTreeMap<String, Value>,
        applied: bool,
    ) {
        for (p, v) in values {
            let k = key(group, p);
            let sensitive = self.is_sensitive(&k);
            match edits.get_mut(&k) {
                Some(r) => {
                    r.new = v.clone();
                    r.applied |= applied;
                }
                None => {
                    let slot = self.slots.get(&k);
                    edits.insert(
                        k,
                        EditRec {
                            old: slot.and_then(|s| s.value.clone()),
                            old_source: slot.and_then(|s| s.source),
                            new: v.clone(),
                            applied,
                            sensitive,
                        },
                    );
                }
            }
        }
    }

    fn write(&mut self, group: &str, values: &BTreeMap<String, Value>) {
        for (p, v) in values {
            self.slots.insert(
                key(group, p),
                Slot {
                    value: Some(v.clone()),
                    source: Some(Source::Runtime),
                },
            );
        }
        self.revision += 1;
    }

    fn forget_keys(&mut self, now: Time) {
        self.keys.retain(|h| now.within(h.at, KEY_HELD));
    }

    fn hold(&mut self, now: Time, group: &str, req: &SetRequest, answer: &Result<Reply, Refusal>) {
        if let Some(k) = &req.idempotency_key
            && !self.keys.iter().any(|h| &h.key == k)
        {
            self.keys.push(Held {
                key: k.clone(),
                group: group.to_owned(),
                request: req.clone(),
                answer: answer.clone(),
                at: now,
            });
        }
    }

    /// A `set` (§2.5, §2.9): answered now, or an [`Act::Apply`] for a hot
    /// group, whose reply comes with [`Owner::finish`].
    pub fn set(&mut self, now: Time, group: &str, req: &SetRequest, call: &Call) -> Step {
        let refuse = |r: Refusal| {
            Step::Done(Done {
                reply: Some(Err(r)),
                events: Vec::new(),
            })
        };
        if let Err(r) = check_set_shape(req) {
            return refuse(r);
        }
        if self.flight.is_some() {
            return refuse(in_flight());
        }
        self.forget_keys(now);
        if let Some(k) = &req.idempotency_key
            && let Some(h) = self.keys.iter().find(|h| &h.key == k)
        {
            return if h.group == group && h.request == *req {
                Step::Done(Done {
                    reply: Some(h.answer.clone()),
                    events: Vec::new(),
                })
            } else {
                refuse(Refusal::new(
                    Code::InvalidRequest,
                    Some(Field::member("idempotency_key")),
                    "this idempotency key was used for another request within 300 s",
                ))
            };
        }
        let step = self.admit(now, group, req, call);
        if let Step::Done(Done {
            reply: Some(answer),
            ..
        }) = &step
        {
            self.hold(now, group, req, answer);
        }
        step
    }

    fn admit(&mut self, now: Time, group: &str, req: &SetRequest, call: &Call) -> Step {
        let refuse = |r: Refusal| {
            Step::Done(Done {
                reply: Some(Err(r)),
                events: Vec::new(),
            })
        };
        let facts = self.facts(now);
        let a = match check_rows(&self.schema, &facts, group, req) {
            Ok(a) => a,
            Err(r) => return refuse(r),
        };
        if a.dry_run {
            let mut slots = self.slots.clone();
            for (p, v) in &a.values {
                slots.insert(
                    key(group, p),
                    Slot {
                        value: Some(v.clone()),
                        source: Some(Source::Runtime),
                    },
                );
            }
            return Step::Done(Done {
                reply: Some(Ok(Reply::Set(SetReply::Applied(Box::new(
                    self.render(&slots),
                ))))),
                events: Vec::new(),
            });
        }
        if !a.joins && self.rate_spent(now) {
            return refuse(Refusal::busy(format!(
                "{RATE} change events within the last hour: the event's rate is spent; call again later"
            )));
        }
        match a.class {
            Class::Hot => {
                self.flight = Some(Flight::Set {
                    group: group.to_owned(),
                    request: req.clone(),
                    call: call.clone(),
                    values: a.values.clone(),
                    received: now,
                });
                Step::Act(Act::Apply {
                    group: group.to_owned(),
                    values: a.values,
                })
            }
            Class::Reach => {
                let apply_at = now.plus(self.apply_after);
                if a.joins {
                    let deadline = self.pending.as_ref().map_or(now, |c| c.deadline);
                    if apply_at >= deadline {
                        return refuse(Refusal::busy(
                            "this group could not apply before the pending change's deadline; extend the change first",
                        ));
                    }
                    let mut ch = self.pending.take().expect("a join has a pending change");
                    self.record(&mut ch.edits, group, &a.values, false);
                    if !ch.groups.iter().any(|g| g == group) {
                        ch.groups.push(group.to_owned());
                    }
                    ch.waiting.push(Wait {
                        group: group.to_owned(),
                        values: a.values,
                        apply_at,
                    });
                    let reply = PendingReply {
                        token: ch.token.clone(),
                        apply_at: apply_at.rfc3339(),
                        deadline: ch.deadline.rfc3339(),
                    };
                    self.pending = Some(ch);
                    Step::Done(Done {
                        reply: Some(Ok(Reply::Set(SetReply::Pending(reply)))),
                        events: Vec::new(),
                    })
                } else {
                    let w = window(req.confirm_s.expect("row 10: a window"), self.floor_s);
                    let token = self.mint();
                    let deadline = deadline(now, w);
                    let mut edits = BTreeMap::new();
                    self.record(&mut edits, group, &a.values, false);
                    self.pending = Some(Change {
                        token: token.clone(),
                        groups: vec![group.to_owned()],
                        deadline,
                        call: call.clone(),
                        edits,
                        waiting: vec![Wait {
                            group: group.to_owned(),
                            values: a.values,
                            apply_at,
                        }],
                    });
                    Step::Done(Done {
                        reply: Some(Ok(Reply::Set(SetReply::Pending(PendingReply {
                            token,
                            apply_at: apply_at.rfc3339(),
                            deadline: deadline.rfc3339(),
                        })))),
                        events: Vec::new(),
                    })
                }
            }
            Class::Contract => unreachable!("row 8 refuses a contract group"),
        }
    }

    /// `confirm` (§2.10): always answered now.
    pub fn confirm(&mut self, now: Time, req: &ControlRequest, _call: &Call) -> Step {
        if let Err(r) = validate_control(ControlOp::Confirm, &self.facts(now), req) {
            return Step::Done(Done {
                reply: Some(Err(r)),
                events: Vec::new(),
            });
        }
        let ch = self.pending.take().expect("validated: a change is pending");
        let edits = self.edits(&ch.groups, &ch.edits);
        let ev = self.event(now, &ch.token, Outcome::Confirmed, edits, &ch.call);
        self.last = Some(Last {
            token: ch.token.clone(),
            groups: ch.groups.clone(),
            set: ch.edits.keys().cloned().collect(),
            sensitive: ch.edits.values().any(|r| r.sensitive),
        });
        Step::Done(Done {
            reply: Some(Ok(self.control_reply(req.answer.unwrap_or_default()))),
            events: vec![ev],
        })
    }

    /// `extend` (§2.10): always answered now; the deadline moves to `now`
    /// plus the window, earlier or later.
    pub fn extend(&mut self, now: Time, req: &ControlRequest, _call: &Call) -> Step {
        if let Err(r) = validate_control(ControlOp::Extend, &self.facts(now), req) {
            return Step::Done(Done {
                reply: Some(Err(r)),
                events: Vec::new(),
            });
        }
        let w = window(req.confirm_s.expect("validated: a window"), self.floor_s);
        if let Some(ch) = self.pending.as_mut() {
            ch.deadline = deadline(now, w);
        }
        Step::Done(Done {
            reply: Some(Ok(self.control_reply(req.answer.unwrap_or_default()))),
            events: Vec::new(),
        })
    }

    /// `cancel` (§2.10): the undo now, an [`Act::Undo`] when something of
    /// the change is applied, else answered now.
    pub fn cancel(&mut self, now: Time, req: &ControlRequest, _call: &Call) -> Step {
        if let Err(r) = validate_control(ControlOp::Cancel, &self.facts(now), req) {
            return Step::Done(Done {
                reply: Some(Err(r)),
                events: Vec::new(),
            });
        }
        self.end_by_undo(now, UndoWhy::Cancel(req.answer.unwrap_or_default()))
    }

    /// Admits a `persist` (§2.13): the entries to write. Nothing changes
    /// until [`Owner::persisted`]; a write that fails is the runtime's
    /// `internal`.
    ///
    /// # Errors
    ///
    /// The [`Refusal`].
    pub fn persist_plan(&self, now: Time, req: &ControlRequest) -> Result<PersistPlan, Refusal> {
        validate_control(ControlOp::Persist, &self.facts(now), req)?;
        let last = self.last.as_ref().expect("validated: a last change");
        let mut entries: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
        for k in &last.set {
            if let Some(v) = self.slots.get(k).and_then(|s| s.value.clone()) {
                entries
                    .entry(k.0.clone())
                    .or_default()
                    .insert(k.1.clone(), v);
            }
        }
        Ok(PersistPlan {
            token: last.token.clone(),
            entries,
            answer: req.answer.unwrap_or_default(),
        })
    }

    /// Commits a `persist` whose entries the runtime wrote: they read
    /// `source: overlay`. No revision, no event (§2.13).
    pub fn persisted(&mut self, plan: PersistPlan) -> Done {
        for (g, ps) in &plan.entries {
            for (p, v) in ps {
                let k = key(g, p);
                self.overlay.insert(k.clone(), v.clone());
                if let Some(s) = self.slots.get_mut(&k)
                    && s.value.as_ref() == Some(v)
                {
                    s.source = Some(Source::Overlay);
                }
            }
        }
        Done {
            reply: Some(Ok(self.control_reply(plan.answer))),
            events: Vec::new(),
        }
    }

    /// The act owed at `now`, if any (§2.9, §2.11): the deadline's undo,
    /// or else the earliest reach apply whose `apply_at` has come. A change
    /// with nothing applied ends at its deadline without an act. `None`
    /// while an act is in flight, or when nothing is owed.
    pub fn due(&mut self, now: Time) -> Option<Step> {
        if self.flight.is_some() {
            return None;
        }
        if now >= self.pending.as_ref()?.deadline {
            return Some(self.end_by_undo(now, UndoWhy::Deadline));
        }
        let ch = self.pending.as_mut()?;
        let i = ch
            .waiting
            .iter()
            .enumerate()
            .filter(|(_, w)| w.apply_at <= now)
            .min_by_key(|(i, w)| (w.apply_at, *i))
            .map(|(i, _)| i)?;
        let w = ch.waiting.remove(i);
        let act = Act::Apply {
            group: w.group.clone(),
            values: w.values.clone(),
        };
        self.flight = Some(Flight::Apply(w));
        Some(Step::Act(act))
    }

    /// Ends the pending change by its undo: an act over its applied groups,
    /// or, with none applied, `rolled-back` at once (§2.11).
    fn end_by_undo(&mut self, now: Time, why: UndoWhy) -> Step {
        let Some(ch) = self.pending.as_mut() else {
            return Step::Done(Done::default());
        };
        ch.waiting.clear();
        let plan: Vec<Key> = ch
            .edits
            .iter()
            .filter(|(_, r)| r.applied)
            .map(|(k, _)| k.clone())
            .collect();
        if plan.is_empty() {
            let ch = self.pending.take().expect("checked above");
            let edits = self.edits(&ch.groups, &ch.edits);
            let ev = self.event(now, &ch.token, Outcome::RolledBack, edits, &ch.call);
            let reply = match why {
                UndoWhy::Cancel(answer) => Some(Ok(self.control_reply(answer))),
                _ => None,
            };
            return Step::Done(Done {
                reply,
                events: vec![ev],
            });
        }
        let ch = self.pending.as_ref().expect("checked above");
        let restore = plan
            .iter()
            .filter_map(|k| {
                let r = &ch.edits[k];
                let value = r
                    .old
                    .clone()
                    .or_else(|| self.startup_value(k).map(|(v, _)| v))?;
                Some(Restore {
                    group: k.0.clone(),
                    parameter: k.1.clone(),
                    value,
                })
            })
            .collect();
        self.flight = Some(Flight::Undo { why, plan });
        Step::Act(Act::Undo { restore })
    }

    /// Reports the device's result for the act in flight, and answers what
    /// it owed: a `set`'s reply, a `cancel`'s, the events of a change it
    /// ended, or a next act (the undo after a reach apply that failed).
    pub fn finish(&mut self, now: Time, result: DeviceResult) -> Step {
        let Some(flight) = self.flight.take() else {
            return Step::Done(Done::default());
        };
        match flight {
            Flight::Set {
                group,
                request,
                call,
                values,
                received,
            } => {
                let done = match result {
                    DeviceResult::Ok => {
                        self.applied_hot(now, &group, &request, &call, &values, received)
                    }
                    DeviceResult::Refused(words) | DeviceResult::Failed { words, .. } => {
                        let detail = device_detail(&self.schema, &group, &request, Some(&words));
                        Done {
                            reply: Some(Err(Refusal::app(
                                ErrorCode::DeviceRefused,
                                detail,
                                format!("the device refused group {group:?}"),
                            ))),
                            events: Vec::new(),
                        }
                    }
                };
                if let Some(answer) = &done.reply {
                    self.hold(now, &group, &request, answer);
                }
                Step::Done(done)
            }
            Flight::Apply(w) => match result {
                DeviceResult::Ok => {
                    self.write(&w.group, &w.values);
                    if let Some(ch) = self.pending.as_mut() {
                        for p in w.values.keys() {
                            if let Some(r) = ch.edits.get_mut(&key(&w.group, p)) {
                                r.applied = true;
                            }
                        }
                    }
                    Step::Done(Done::default())
                }
                DeviceResult::Refused(_) | DeviceResult::Failed { .. } => {
                    self.end_by_undo(now, UndoWhy::Failed)
                }
            },
            Flight::Undo { why, plan } => {
                let ch = self
                    .pending
                    .take()
                    .expect("an undo ends the pending change");
                let outcome = match &result {
                    DeviceResult::Ok => {
                        for k in &plan {
                            let r = &ch.edits[k];
                            let (value, source) = match &r.old {
                                Some(v) => (Some(v.clone()), r.old_source),
                                None => match self.startup_value(k) {
                                    Some((v, s)) => (Some(v), Some(s)),
                                    None => (None, None),
                                },
                            };
                            self.slots.insert(k.clone(), Slot { value, source });
                        }
                        Outcome::RolledBack
                    }
                    DeviceResult::Refused(_) | DeviceResult::Failed { .. } => {
                        let empty = BTreeMap::new();
                        let readback = match &result {
                            DeviceResult::Failed { readback, .. } => readback,
                            _ => &empty,
                        };
                        for k in &plan {
                            let r = &ch.edits[k];
                            let read = readback.get(&k.0).and_then(|ps| ps.get(&k.1)).cloned();
                            let slot = match read {
                                Some(v) => {
                                    let source = if Some(&v) == r.old.as_ref() {
                                        r.old_source
                                    } else {
                                        Some(Source::Runtime)
                                    };
                                    Slot {
                                        value: Some(v),
                                        source,
                                    }
                                }
                                None => Slot {
                                    value: None,
                                    source: None,
                                },
                            };
                            self.slots.insert(k.clone(), slot);
                        }
                        Outcome::Partial
                    }
                };
                self.revision += 1;
                let edits = self.edits(&ch.groups, &ch.edits);
                let ev = self.event(now, &ch.token, outcome, edits, &ch.call);
                let reply = match why {
                    UndoWhy::Cancel(answer) => Some(Ok(self.control_reply(answer))),
                    UndoWhy::Deadline | UndoWhy::Failed => None,
                };
                Step::Done(Done {
                    reply,
                    events: vec![ev],
                })
            }
        }
    }

    fn applied_hot(
        &mut self,
        now: Time,
        group: &str,
        request: &SetRequest,
        call: &Call,
        values: &BTreeMap<String, Value>,
        received: Time,
    ) -> Done {
        let mut events = Vec::new();
        if let Some(mut ch) = self.pending.take() {
            self.record(&mut ch.edits, group, values, true);
            if !ch.groups.iter().any(|g| g == group) {
                ch.groups.push(group.to_owned());
            }
            self.pending = Some(ch);
            self.write(group, values);
        } else {
            let mut edits = BTreeMap::new();
            self.record(&mut edits, group, values, true);
            self.write(group, values);
            let token = self.mint();
            let groups = vec![group.to_owned()];
            if let Some(cs) = request.confirm_s {
                self.pending = Some(Change {
                    token,
                    groups,
                    deadline: deadline(received, window(cs, self.floor_s)),
                    call: call.clone(),
                    edits,
                    waiting: Vec::new(),
                });
            } else {
                let list = self.edits(&groups, &edits);
                events.push(self.event(now, &token, Outcome::Applied, list, call));
                self.last = Some(Last {
                    token,
                    groups,
                    set: edits.keys().cloned().collect(),
                    sensitive: edits.values().any(|r| r.sensitive),
                });
            }
        }
        Done {
            reply: Some(Ok(Reply::Set(SetReply::Applied(Box::new(self.view()))))),
            events,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_spells_and_reads_rfc3339() {
        assert_eq!(Time::from_millis(0).rfc3339(), "1970-01-01T00:00:00.000Z");
        assert_eq!(Time::from_millis(-1).rfc3339(), "1969-12-31T23:59:59.999Z");
        let t = Time::parse("2028-02-29T12:34:56.789Z").unwrap();
        assert_eq!(t.rfc3339(), "2028-02-29T12:34:56.789Z");
        assert_eq!(
            Time::parse("2026-10-18T14:00:00+02:00"),
            Time::parse("2026-10-18T12:00:00Z")
        );
        assert_eq!(
            Time::parse("2026-10-18t12:00:00.1234z").map(Time::millis),
            Time::parse("2026-10-18T12:00:00.123Z").map(Time::millis)
        );
        for bad in [
            "2027-02-29T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-10-18 12:00:00Z",
            "2026-10-18T12:00:00",
            "2026-10-18T12:00:00.Z",
            "2026-10-18T24:00:00Z",
            "",
        ] {
            assert_eq!(Time::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_kind_reads_json_as_the_profile_does() {
        let int = Kind::Integer {
            min: Some(0),
            max: Some(10),
            unit: None,
        };
        assert_eq!(int.check(&serde_json::json!(10)), Ok(Value::Integer(10)));
        assert_eq!(int.check(&serde_json::json!(11)), Err(Misfit::Above(10)));
        assert_eq!(int.check(&serde_json::json!(2.0)), Err(Misfit::Kind));
        assert_eq!(int.check(&serde_json::json!(u64::MAX)), Err(Misfit::Range));
        assert_eq!(Kind::Bool.check(&serde_json::json!(1)), Err(Misfit::Kind));
        assert_eq!(Kind::Text.check(&Json::Null), Err(Misfit::Kind));
    }

    #[test]
    fn the_wire_shapes_are_the_contracts() {
        let reply = SetReply::Pending(PendingReply {
            token: "i1-1".into(),
            apply_at: "1970-01-01T00:00:01.000Z".into(),
            deadline: "1970-01-01T00:02:00.000Z".into(),
        });
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            serde_json::json!({"pending": {"token": "i1-1", "apply_at": "1970-01-01T00:00:01.000Z", "deadline": "1970-01-01T00:02:00.000Z"}})
        );
        let p = Parameter {
            name: "mtu".into(),
            kind: Kind::Integer {
                min: Some(68),
                max: None,
                unit: Some("By".into()),
            },
            description: String::new(),
            sensitive: false,
            value: Some(Value::Integer(1500)),
            source: Some(Source::File),
            startup: None,
        };
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(
            j,
            serde_json::json!({"name": "mtu", "kind": "integer", "min": 68, "unit": "By", "description": "", "value": 1500, "source": "file"})
        );
        assert_eq!(serde_json::from_value::<Parameter>(j).unwrap(), p);
        let decl: ParamDecl = serde_json::from_value(serde_json::json!({"name": "pin", "kind": "text", "description": "", "sensitive": true, "value": "x"})).unwrap();
        assert!(decl.sensitive);
        assert_eq!(decl.kind, Kind::Text);
    }

    /// scenarios.md's `rf`, reduced: `duty` and `sim` hot, `radio` reach.
    fn rf() -> Owner {
        let int = |min, max| Kind::Integer {
            min: Some(min),
            max: Some(max),
            unit: None,
        };
        let group = |name: &str, class, p: ParamDecl| GroupDecl {
            name: name.into(),
            class,
            description: String::new(),
            writable: true,
            parameters: vec![p],
        };
        let param = |name: &str, kind, sensitive| ParamDecl {
            name: name.into(),
            kind,
            description: String::new(),
            sensitive,
        };
        let schema = Schema {
            groups: vec![
                group(
                    "duty",
                    Class::Hot,
                    param("fraction_ppm", int(0, 1_000_000), false),
                ),
                group(
                    "radio",
                    Class::Reach,
                    param("frequency_hz", int(0, 1_000_000_000), false),
                ),
                group("sim", Class::Hot, param("pin", Kind::Text, true)),
                group("tx", Class::Hot, param("power", int(0, 30), false)),
            ],
        };
        let startup = [
            ("duty", "fraction_ppm", Value::Integer(10_000)),
            ("radio", "frequency_hz", Value::Integer(868_100_000)),
            ("tx", "power", Value::Integer(14)),
        ]
        .into_iter()
        .map(|(g, p, v)| {
            (
                g.to_owned(),
                BTreeMap::from([(p.to_owned(), (v, Source::File))]),
            )
        })
        .collect();
        Owner::start(Start {
            service: "lab/rf0".into(),
            instance: "i1".into(),
            schema,
            startup,
            overlay: BTreeMap::new(),
            floor_s: 10,
            apply_after: Duration::from_secs(1),
            persist: true,
            events: vec![Time::from_millis(0)],
        })
        .unwrap()
    }

    fn doc(revision: u64, values: Json) -> DesiredConfig {
        serde_json::from_value(serde_json::json!({"revision": revision, "values": values})).unwrap()
    }

    /// §2.18 "Converging": one set per differing group, bytewise; a group
    /// already at its values is skipped; a pending change is waited out;
    /// a reach group refuses the document whole, applying nothing.
    #[test]
    fn converging_a_desired_document() {
        let mut o = rf();
        let t = Time::from_millis(10_000);
        let d = doc(
            1,
            serde_json::json!({"tx": {"power": 20}, "duty": {"fraction_ppm": 10000}}),
        );
        let Converge::Set { group, request } = o.converge(&d) else {
            panic!("a set")
        };
        assert_eq!(group, "tx", "duty already runs its value");
        assert_eq!(
            request.values,
            BTreeMap::from([("power".into(), 20.into())])
        );
        assert_eq!((request.confirm_s, request.token.as_ref()), (None, None));
        let Step::Act(_) = o.set(t, &group, &request, &Call::default()) else {
            panic!("a hot apply")
        };
        let Step::Done(done) = o.finish(t, DeviceResult::Ok) else {
            panic!("done")
        };
        assert_eq!(done.events[0].outcome, Outcome::Applied);
        let Converge::Converged(r) = o.converge(&d) else {
            panic!("converged")
        };
        assert_eq!(
            (r.observed_revision, r.outcome),
            (1, DesiredOutcome::Converged)
        );

        // A pending change is waited out.
        let reach = SetRequest {
            values: BTreeMap::from([("frequency_hz".into(), 869_525_000.into())]),
            confirm_s: Some(60),
            ..SetRequest::default()
        };
        let _ = o.set(t, "radio", &reach, &Call::default());
        assert_eq!(
            o.converge(&doc(2, serde_json::json!({"tx": {"power": 1}}))),
            Converge::Wait
        );

        // Refused whole: the reach group, though tx is legal on its own.
        let mut o = rf();
        let d = doc(
            3,
            serde_json::json!({"tx": {"power": 1}, "radio": {"frequency_hz": 1}}),
        );
        let Converge::Refused(r) = o.converge(&d) else {
            panic!("refused")
        };
        assert_eq!(r.outcome, DesiredOutcome::Refused);
        assert_eq!(
            r.refusal,
            Some(DesiredRefusal {
                code: DesiredCode::Reach,
                group: "radio".into(),
                parameter: None
            })
        );
        o.report_desired(Some(r.clone()));
        assert_eq!(o.desired(), Some(&r));
        assert_eq!(o.view().desired, Some(r));
    }

    /// §2.18: a converging set refused is `failed`, with its code and group.
    #[test]
    fn a_converging_set_refused_is_failed() {
        let busy = Refusal::busy("the rate");
        let r = desired_failed(4, "tx", &busy);
        assert_eq!(
            (r.observed_revision, r.outcome),
            (4, DesiredOutcome::Failed)
        );
        assert_eq!(r.refusal.as_ref().map(|r| r.code), Some(DesiredCode::Busy));
        let refused = Refusal::app(ErrorCode::DeviceRefused, None, "no");
        let r = desired_failed(4, "tx", &refused);
        assert_eq!(
            r.refusal,
            Some(DesiredRefusal {
                code: DesiredCode::DeviceRefused,
                group: "tx".into(),
                parameter: None
            })
        );
    }

    /// §2.17, core §2.7: the events of the last hour carry to the next
    /// instance, and the restart keeps them.
    #[test]
    fn the_published_events_carry_to_the_next_instance() {
        let mut o = rf();
        let t = Time::from_millis(3_000_000);
        let req = SetRequest {
            values: BTreeMap::from([("power".into(), 3.into())]),
            ..SetRequest::default()
        };
        let _ = o.set(t, "tx", &req, &Call::default());
        let _ = o.finish(t, DeviceResult::Ok);
        assert_eq!(o.published(t), vec![Time::from_millis(0), t]);
        let later = Time::from_millis(3_600_000);
        assert_eq!(o.published(later), vec![t], "an hour on, the first is out");
        assert_eq!(o.restart("i2").published(later), vec![t]);
    }

    /// §2.15: the device's words about an act that sets a sensitive value,
    /// or restores one, are not logged.
    #[test]
    fn device_words_leave_a_sensitive_act_out() {
        let o = rf();
        let apply = |g: &str, p: &str, v: Value| Act::Apply {
            group: g.into(),
            values: BTreeMap::from([(p.into(), v)]),
        };
        let schema = o.schema();
        assert_eq!(
            device_words(schema, &apply("tx", "power", Value::Integer(1)), "no"),
            Some("no")
        );
        assert_eq!(
            device_words(
                schema,
                &apply("sim", "pin", Value::Text("1234".into())),
                "1234?"
            ),
            None
        );
        assert_eq!(
            device_words(schema, &apply("nope", "x", Value::Bool(true)), "x"),
            None,
            "unknown: it cannot tell"
        );
        let undo = Act::Undo {
            restore: vec![Restore {
                group: "sim".into(),
                parameter: "pin".into(),
                value: Value::Text("0000".into()),
            }],
        };
        assert!(undo.carries_sensitive(schema));
        assert_eq!(device_words(schema, &undo, "0000"), None);
    }
}
