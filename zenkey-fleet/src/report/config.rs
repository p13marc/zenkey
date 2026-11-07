//! `config.v1` (#768, CC): a running service's configuration as `zenctl
//! config` reads it and changes it (`spec/profiles/config/v1.md`).
//!
//! **Two families.** [`ConfigReport`] is a reading (`config get`): the view
//! and the status as their owner answered them, each with its freshness
//! judged by `freshness.v1` at the contract's 60 s, and §5's questions
//! answered from them in the profile's three states. [`ConfigActReport`] is
//! one act (`config set|confirm|cancel|extend|persist`): the request as it
//! was sent, and the reply union as the text names it.
//!
//! **§5's answers keep their unestablished kinds apart.** A
//! [`ConfigAnswer`] is `yes` or `no`, or one of three ways of neither:
//! `not_yet` (the change is still pending, or the owner has not reached the
//! document's revision), `unobservable` (the question was put, and nothing
//! fresh answered it) and `not_asked` (its premise does not hold: the
//! descriptor does not list `config.v1`, the owner is absent, the schema
//! declares no such group). None of them is ever folded into a `no` (core
//! O5): a view that is stale, or that a GET did not draw, is no owner's
//! word.
//!
//! **The reply unions as the text names them** (§3). A `set` is answered
//! `applied` (the read-back after the apply) or `pending` (a reach group's
//! `{token, apply_at, deadline}`, before the apply); a control operation
//! `view` or `status`. A refusal is the core's envelope, its `app` detail a
//! [`ConfigError`]; a call that drew nothing is silence, attributed through
//! presence, never a failure.
//!
//! **A sensitive value is never shown** (§2.15). The request carries it
//! `"<redacted>"`, and a parameter the schema declares sensitive is shown
//! without a value or a startup value whatever a reply carried.
//!
//! **`config.v2`** (#985 G3, `spec/profiles/config/v2.md`) rides the same
//! shapes, `profile` saying which. Its one record, read through the
//! contract, stands where the view stands ("a fresh record is a fresh view
//! and status", v2 §5): [`ConfigReport::view`] is the record as the view
//! reads it, the status is not asked, and [`ConfigReport::record`] holds
//! the record itself ([`ConfigRecordRead`]). Every act answers the record
//! ([`ConfigActAnswer::Record`]), and a refusal is core §7.5's error form,
//! its `Error` detail read by the contract's layout.

use serde::Serialize;
use zenkey_model::config::{
    Class, ConfigChangeEvent, ConfigError, ConfigStatus, ConfigView, Pending, PendingReply,
};

use super::asked::{Asked, u64_is_zero};
use super::health::ClockGround;
use super::operation::{MalformedReply, SilenceView};
use super::state::{InbandStamp, Stamp};

/// Which profile a reading or an act is of (#985 G3): `config.v1`'s
/// standard interface, or a `config.v2` owner's own config interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum ConfigProfile {
    #[default]
    #[serde(rename = "config.v1")]
    V1,
    #[serde(rename = "config.v2")]
    V2,
}

impl ConfigProfile {
    /// The profile's id, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigProfile::V1 => "config.v1",
            ConfigProfile::V2 => "config.v2",
        }
    }

    /// The `skip_serializing_if` predicate: `config.v1`'s shapes are
    /// unchanged, so a report of it says nothing more.
    pub fn is_v1(&self) -> bool {
        *self == ConfigProfile::V1
    }
}

/// `config.v2`'s record as one reading read it (v2 §2.3), through its
/// contract: the interface and the revision read by, the record's key, the
/// deployment epoch its times were dated from, and the record itself as
/// the fixtures spell it (`conformance/v2/records.json`'s `state`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigRecordRead {
    /// The config interface, `<name>.v<major>`: the one whose contract
    /// uses `config.v2` (v2 §2.1).
    pub iface: String,
    /// The revision read by, its fingerprint (core §8.4).
    pub contract: String,
    /// The record's key, base-relative.
    pub key: String,
    /// The deployment epoch its times count from, seconds since the Unix
    /// epoch (core §8.6).
    pub epoch: i64,
    /// Nothing stated the epoch: 0 is assumed (no catalog in hand, no
    /// descriptor naming one).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub epoch_assumed: bool,
    /// The record read last; absent when none was. Its times are
    /// milliseconds since the Unix epoch, on the owner's clock (v2 §2.8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<zenkey_model::config::v2::State>,
    /// The in-band stamp of the put it was (core §4.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inband: Option<InbandStamp>,
    /// Its payload's size, bytes, the envelope included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
}

/// One of §5's answers: a yes, a no, or one of the three ways of neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigAnswerToken {
    Yes,
    No,
    /// Unestablished, and still moving: the change is pending, or the owner
    /// has acted on a document below the one asked about.
    NotYet,
    /// The question was put, and nothing fresh answered it.
    Unobservable,
    /// The premise does not hold: not this profile's service, absent, or no
    /// such group or parameter.
    NotAsked,
}

impl ConfigAnswerToken {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigAnswerToken::Yes => "yes",
            ConfigAnswerToken::No => "no",
            ConfigAnswerToken::NotYet => "not_yet",
            ConfigAnswerToken::Unobservable => "unobservable",
            ConfigAnswerToken::NotAsked => "not_asked",
        }
    }
}

/// One of §5's questions answered: the answer, its reason class, and the
/// reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigAnswer {
    pub answer: ConfigAnswerToken,
    pub reason: String,
    pub says: String,
}

impl ConfigAnswer {
    pub fn new(answer: ConfigAnswerToken, reason: &str, says: impl Into<String>) -> ConfigAnswer {
        ConfigAnswer {
            answer,
            reason: reason.to_owned(),
            says: says.into(),
        }
    }
}

/// `freshness.v1`'s verdict on one document (§5's premise for every row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessToken {
    Fresh,
    Stale,
    Unobservable,
    NotAsked,
}

impl FreshnessToken {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            FreshnessToken::Fresh => "fresh",
            FreshnessToken::Stale => "stale",
            FreshnessToken::Unobservable => "unobservable",
            FreshnessToken::NotAsked => "not_asked",
        }
    }
}

/// A document's freshness: the verdict, its reason class (the fixtures'
/// token) and the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigFreshness {
    pub verdict: FreshnessToken,
    pub reason: String,
    pub says: String,
}

/// How a document reached this reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigVia {
    /// The owner's reply to an S4 GET (core §4.2).
    Get,
    /// The last put a subscription heard.
    Subscription,
}

/// One document as read: the value, or why the bytes read are not one, how
/// it came, its stamp, and its freshness.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigDocument<T> {
    /// The document; absent when nothing decodable was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<T>,
    /// Why what was read is not the document's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undecodable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<ConfigVia>,
    /// The stamp of the put it carries, the owner's clock (core S1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    pub freshness: ConfigFreshness,
}

/// What presence and the descriptors say of `config.v1` at the service:
/// every §5 row's premise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigPremise {
    /// Present, and a served descriptor lists `config.v1` (§2.1).
    Listed,
    /// Present, and no served descriptor lists it: not asked.
    NotListed,
    /// Present, and no descriptor of it read: unobservable.
    DescriptorUnread,
    /// No instance token in a complete read: not asked, presence's word.
    Absent,
    /// The presence read did not show the service and is possibly
    /// incomplete — it ended at its timeout, or its session was cut off
    /// while it ran — or it was not made: unobservable (core §8.1; 0.38,
    /// F-191, for a read not made, whose own reason the report keeps in
    /// `unobservable`).
    Incomplete,
    /// Across a constrained face: the deployment's word stands for the
    /// descriptor (§2.19, core R7).
    AcrossFace,
    /// Present, a served descriptor lists `config.v2`, and no revision it
    /// names that uses `config.v2` is in hand: the record cannot be read
    /// (v2 §2.2), unobservable.
    ContractUnread,
    /// The config contract in hand has a `config.v2` lint finding: its
    /// records are unobservable (v2 §2.2).
    ContractInvalid,
}

impl ConfigPremise {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigPremise::Listed => "listed",
            ConfigPremise::NotListed => "not_listed",
            ConfigPremise::DescriptorUnread => "descriptor_unread",
            ConfigPremise::Absent => "absent",
            ConfigPremise::Incomplete => "incomplete",
            ConfigPremise::AcrossFace => "across_face",
            ConfigPremise::ContractUnread => "contract_unread",
            ConfigPremise::ContractInvalid => "contract_invalid",
        }
    }
}

/// The presence read the premise rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigPresenceRead {
    /// The liveliness selector, base-relative in the namespace.
    pub selector: String,
    /// Whether the read ended at the routers' final reply (core §8.1).
    pub complete: bool,
    /// The read's session was cut off while it ran (core §8.1, 0.38): zenoh
    /// answered it at the cut, so `complete` is false. Absent otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off_during: bool,
    /// The instances of the service it saw.
    pub instances: Vec<String>,
}

/// "Is this group writable?" for one group (§2.16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GroupAnswer {
    pub group: String,
    #[serde(flatten)]
    pub answer: ConfigAnswer,
}

/// "Will this value survive a restart?" for one parameter (§2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParameterAnswer {
    pub group: String,
    pub parameter: String,
    #[serde(flatten)]
    pub answer: ConfigAnswer,
}

/// "Did my change land?" and "was its undo clean?" for one change, named by
/// its token as the `set` that started it was answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChangeAnswers {
    pub token: String,
    /// The instance that answered the `set`, when the caller named it: a
    /// restart is told from it (§2.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub landed: ConfigAnswer,
    pub undo_clean: ConfigAnswer,
    /// The change's event, when one was read (§2.17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ConfigChangeEvent>,
}

/// "Has it converged to its desired document?" (§2.18).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DesiredAnswer {
    /// The document's revision asked about; absent when the question was
    /// put of the last document the owner acted on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(flatten)]
    pub answer: ConfigAnswer,
}

/// `zenctl config get` (#768, CC): one service's `config.v1`, read as §5's
/// tool reads it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// `<system>/<service>`.
    pub service: String,
    /// `config.v2` when the reading read one (#985 G3); absent for
    /// `config.v1`.
    #[serde(default, skip_serializing_if = "ConfigProfile::is_v1")]
    pub profile: ConfigProfile,
    pub premise: ConfigPremise,
    /// `config.v2`'s record, read through the contract (v2 §2.3): the
    /// interface, the revision, the epoch and the record itself. Absent
    /// for `config.v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<ConfigRecordRead>,
    /// Presence as read; absent across a face, where it does not cross.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<ConfigPresenceRead>,
    /// The window's length, seconds: how long a subscription to the view,
    /// the status and the change events listened. Absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    /// How this reader trusted its clock against the owner's, to age a GET
    /// reply's stamp (`freshness.v1` §2.6).
    pub clock: ClockGround,
    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider.
    pub asked: Vec<String>,
    /// The read-back, `state/view`; absent when only the status was asked
    /// (`--status`). For `config.v2`, the record as the view reads it: its
    /// schema the contract's, its values, pending and last change the
    /// record's, a token and the instance number in decimal, and no
    /// startup value, which the record does not carry (v2 §2.3).
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub view: Asked<ConfigDocument<ConfigView>>,
    /// The compact status, `state/status`; never asked of `config.v2`,
    /// whose one record replaces it.
    #[serde(default, skip_serializing_if = "Asked::is_not_asked")]
    pub status: Asked<ConfigDocument<ConfigStatus>>,
    /// Whether the status read is the view's projection (§2.19). One
    /// reading can pair a view and a status a put apart (§2.3), so a `no`
    /// here is informative, never a finding.
    pub projection: ConfigAnswer,
    /// "Is a change pending?"
    pub pending: ConfigAnswer,
    /// The pending change that answer rests on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_change: Option<Pending>,
    /// The change asked about (`--token`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<ChangeAnswers>,
    /// "Is this group writable?", per group of the view read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable: Vec<GroupAnswer>,
    /// "Will this value survive a restart?", per parameter of the view read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub survives_restart: Vec<ParameterAnswer>,
    /// "Has it converged to its desired document?"
    pub converged: DesiredAnswer,
    /// The change events the window heard, and those a replay drew for the
    /// change asked about, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ConfigChangeEvent>,
    /// Events read that do not decode as `ConfigChangeEvent`.
    #[serde(default, skip_serializing_if = "u64_is_zero")]
    pub events_undecodable: u64,
    /// Set when the reading could not be made at all, with why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl ConfigReport {
    /// The documents read and decoded: the view's, when asked, and the
    /// status's.
    fn decoded(&self) -> (bool, bool) {
        let of = |d: Option<(bool, bool)>| d.unwrap_or((false, false));
        let view = of(self
            .view
            .as_option()
            .map(|d| (d.value.is_some(), d.undecodable.is_some())));
        let status = of(self
            .status
            .as_option()
            .map(|d| (d.value.is_some(), d.undecodable.is_some())));
        (view.0 || status.0, view.1 || status.1)
    }

    /// `config get`'s exit code, a read's: 0 a document read and decoded,
    /// 1 the owner answered with bytes that are not its document's type (a
    /// broken owner), 2 nothing read — silence, an absent service, one whose
    /// descriptor does not list `config.v1`, or a read that could not be
    /// made. §5's answers ride the report and never move the code.
    pub fn exit_code(&self) -> i32 {
        match self.decoded() {
            (true, _) => 0,
            (false, true) => 1,
            (false, false) => 2,
        }
    }
}

/// What a `set` read before it was sent (§2.5, "A caller that checks
/// first").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigChecked {
    /// The document read: `view`, `status`, or `nothing` (`--no-precheck`).
    pub read: ConfigCheckedRead,
    /// How the values were typed: by the schema the view serves, by the
    /// status's values, or by their spelling.
    pub typed_by: ConfigTyping,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// The group's class, when the view declares the group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<Class>,
}

/// The document a `set` read first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigCheckedRead {
    View,
    Status,
    /// `config.v2`'s record, through its contract (v2 §2.2, §2.3).
    Record,
    Nothing,
}

/// How a `set`'s values were typed from the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTyping {
    /// Each value as the kind the view declares for its parameter.
    Schema,
    /// Each value as the type of the status's value for its parameter.
    Status,
    /// `true` and `false` a bool, a decimal integer an integer, else text.
    Spelling,
}

/// A refusal, as the core's envelope carries it (§2.5): its code, the
/// message for a person, `unavailable`'s cause, and `app`'s detail, decoded
/// as its profile's type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigRefusal {
    pub code: String,
    /// For a person, never parsed (core §5.2). Absent when an error form
    /// carried none (core §7.5), as a `config.v2` owner refuses a change
    /// that carries a sensitive value (v2 §2.5, §2.15); the bare wire's
    /// envelope always carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The `app` detail, decoded as its profile's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ConfigAppError>,
    /// Why an `app` detail is not its profile's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_undecodable: Option<String>,
    /// The error form's as-of mark (core §5.2); absent on the bare wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<InbandStamp>,
}

/// A refusal's `app` detail, as its profile names it. Each spells its own
/// codes, so the two are never mapped onto each other.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ConfigAppError {
    /// `config.v1`'s `ConfigError` (§2.5): `{"code": "device-refused"}`.
    V1(ConfigError),
    /// `config.v2`'s `Error` record (v2 §2.5, §4), read by the contract's
    /// layout: its code the packed enum's own name, `{"code":
    /// "device_refused"}`, or `null` for the escape, a code this revision
    /// does not name.
    V2 { code: Option<String> },
}

impl ConfigAppError {
    /// The detail's type, as its profile names it.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::V1(_) => "ConfigError",
            Self::V2 { .. } => "Error",
        }
    }

    /// Its code for a person: the profile's own name, or for `config.v2`'s
    /// escape what it is.
    #[must_use]
    pub fn code(&self) -> &str {
        match self {
            Self::V1(e) => e.code.as_str(),
            Self::V2 { code: Some(c) } => c,
            Self::V2 { code: None } => "the escape: a code this revision does not name",
        }
    }

    /// `config.v1`'s detail text; `config.v2`'s `Error` carries none.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::V1(e) => e.detail.as_deref(),
            Self::V2 { .. } => None,
        }
    }
}

/// What an act got back. Tagged `answer`; each case is distinct (core O5).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ConfigActAnswer {
    /// A `set`'s `{applied: ConfigView}`: the read-back after the apply, or
    /// a dry run's would-be read-back (§2.9).
    Applied { view: Box<ConfigView> },
    /// A reach `set`'s `{pending: {token, apply_at, deadline}}`, answered
    /// before the apply; the instants are the owner's clock (§2.8, §2.9).
    Pending { pending: PendingReply },
    /// A control operation's `{view: ConfigView}` (§2.10).
    View { view: Box<ConfigView> },
    /// A control operation's `{status: ConfigStatus}`, asked with `answer =
    /// "status"` (§2.10, §2.19).
    Status { status: Box<ConfigStatus> },
    /// `config.v2`'s every reply (v2 §2.9): the record as the act left it,
    /// in the value form, as the view reads it (`view`) and as it is
    /// (`record`, the fixtures' spelling); a dry run's with `would_be`
    /// set, never the owner's state. `size` is the reply's payload, bytes,
    /// and `mark` its as-of mark (core §5.1).
    Record {
        view: Box<ConfigView>,
        record: Box<zenkey_model::config::v2::State>,
        size: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mark: Option<InbandStamp>,
    },
    /// The core's envelope: an answer, and the owner's refusal.
    Refused { refusal: ConfigRefusal },
    /// A `reply_err` claiming an envelope encoding that does not decode.
    Malformed { malformed: MalformedReply },
    /// A value reply that is not the operation's reply type.
    Undecodable { reason: String, size: usize },
    /// No value and no envelope: silence, never a verdict (core O5). A `set`
    /// that drew nothing started nothing a tool can name (§5).
    Silent { silence: SilenceView },
}

/// `zenctl config set|confirm|cancel|extend|persist` (#768, CC): one act on
/// one service's configuration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigActReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// `<system>/<service>`.
    pub service: String,
    /// `config.v2` for an act on a `config.v2` owner (#985 G3); absent for
    /// `config.v1`.
    #[serde(default, skip_serializing_if = "ConfigProfile::is_v1")]
    pub profile: ConfigProfile,
    /// `set/<group>`, `confirm`, `cancel`, `extend` or `persist`.
    pub operation: String,
    /// The concrete key the call went out on, base-relative.
    pub key: String,
    /// The request as sent, every sensitive value `"<redacted>"` (§2.15).
    pub request: serde_json::Value,
    /// `config.v2`'s request payload, bytes: `CH · record` (core §7.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_bytes: Option<usize>,
    /// The call metadata claimed (core O7): never authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// What a `set` read and checked before it was sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<ConfigChecked>,
    /// Sent across a constrained face: presence does not cross, so a
    /// silence is unattributed (core R7).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub across_face: bool,
    /// How long the call waited for its reply, seconds.
    pub timeout_s: f64,
    /// What came back. Tagged `answer`.
    #[serde(flatten)]
    pub answer: ConfigActAnswer,
}

impl ConfigActReport {
    /// The act's exit code (`crate::exit`'s acts): 0 the owner answered with
    /// its reply type, 1 a refusal, a malformed envelope or a reply that is
    /// not the type (the act failed), 2 silence — unestablished, never a
    /// failure (core O5).
    pub fn exit_code(&self) -> i32 {
        match &self.answer {
            ConfigActAnswer::Applied { .. }
            | ConfigActAnswer::Pending { .. }
            | ConfigActAnswer::View { .. }
            | ConfigActAnswer::Status { .. }
            | ConfigActAnswer::Record { .. } => 0,
            ConfigActAnswer::Refused { .. }
            | ConfigActAnswer::Malformed { .. }
            | ConfigActAnswer::Undecodable { .. } => 1,
            ConfigActAnswer::Silent { .. } => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::PresenceAttribution;
    use serde_json::json;

    fn answer(a: ConfigAnswerToken, reason: &str) -> ConfigAnswer {
        ConfigAnswer::new(a, reason, "s")
    }

    fn fresh() -> ConfigFreshness {
        ConfigFreshness {
            verdict: FreshnessToken::Fresh,
            reason: "within_horizon".into(),
            says: "s".into(),
        }
    }

    fn status() -> ConfigStatus {
        serde_json::from_value(json!({
            "instance": "i1", "revision": 1, "confirm_floor_s": 10,
            "last_change": {"token": "i1-1", "groups": ["duty"]},
            "values": {"duty": {"fraction_ppm": 20000}}
        }))
        .unwrap()
    }

    fn report() -> ConfigReport {
        ConfigReport {
            namespace: "acme".into(),
            service: "lab/rf0".into(),
            profile: ConfigProfile::V1,
            premise: ConfigPremise::Listed,
            record: None,
            presence: Some(ConfigPresenceRead {
                selector: "zk2/lab/rf0/@zk/**".into(),
                complete: true,
                cut_off_during: false,
                instances: vec!["i1".into()],
            }),
            window_s: None,
            clock: ClockGround::DeploymentWord,
            asked: vec!["zk2/lab/rf0/config.v1/state/status".into()],
            view: Asked::NotAsked,
            status: Asked::Asked(ConfigDocument {
                value: Some(status()),
                undecodable: None,
                via: Some(ConfigVia::Get),
                stamp: Some(Stamp {
                    time: "2026-10-18T00:00:00.000000000Z".into(),
                    clock: "ab12".into(),
                }),
                freshness: fresh(),
            }),
            projection: answer(ConfigAnswerToken::NotAsked, "view_not_asked"),
            pending: answer(ConfigAnswerToken::No, "none_pending"),
            pending_change: None,
            change: None,
            writable: vec![],
            survives_restart: vec![],
            converged: DesiredAnswer {
                revision: None,
                answer: answer(ConfigAnswerToken::NotAsked, "no_desired"),
            },
            events: vec![],
            events_undecodable: 0,
            unobservable: None,
        }
    }

    /// The reading's wire shape: §5's tokens on every answer, the not-yet
    /// pole of its own, a group's and a parameter's answer flattened beside
    /// their names, the view absent when only the status was asked, and
    /// every optional field absent when empty.
    #[test]
    fn the_config_report_pins_its_shape() {
        let mut r = report();
        r.change = Some(ChangeAnswers {
            token: "i1-2".into(),
            instance: None,
            landed: answer(ConfigAnswerToken::NotYet, "still_pending"),
            undo_clean: answer(ConfigAnswerToken::NotAsked, "not_ended"),
            event: None,
        });
        r.writable = vec![GroupAnswer {
            group: "queue".into(),
            answer: answer(ConfigAnswerToken::No, "outside_allowlist"),
        }];
        r.survives_restart = vec![ParameterAnswer {
            group: "duty".into(),
            parameter: "fraction_ppm".into(),
            answer: answer(ConfigAnswerToken::No, "startup_differs"),
        }];
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("view").is_none(), "--status asks no view");
        assert!(
            v.get("profile").is_none() && v.get("record").is_none(),
            "config.v1's shape says nothing of config.v2"
        );
        assert!(v.get("window_s").is_none() && v.get("events").is_none());
        assert_eq!(v["premise"], "listed");
        assert_eq!(v["clock"], "deployment_word");
        assert_eq!(
            v["status"]["freshness"],
            json!({"verdict": "fresh", "reason": "within_horizon", "says": "s"})
        );
        assert_eq!(v["status"]["via"], "get");
        assert_eq!(v["status"]["value"]["revision"], 1);
        assert_eq!(
            v["change"],
            json!({"token": "i1-2",
                   "landed": {"answer": "not_yet", "reason": "still_pending", "says": "s"},
                   "undo_clean": {"answer": "not_asked", "reason": "not_ended", "says": "s"}})
        );
        assert_eq!(
            v["writable"],
            json!([{"group": "queue", "answer": "no", "reason": "outside_allowlist",
                    "says": "s"}])
        );
        assert_eq!(
            v["survives_restart"][0],
            json!({"group": "duty", "parameter": "fraction_ppm", "answer": "no",
                   "reason": "startup_differs", "says": "s"})
        );
        assert_eq!(
            v["converged"],
            json!({"answer": "not_asked", "reason": "no_desired", "says": "s"})
        );
        assert_eq!(r.exit_code(), 0);
    }

    /// A read's exit: a document decoded is the 0, bytes that are not one
    /// the 1, nothing read the 2.
    #[test]
    fn a_reading_exits_on_what_it_read() {
        let mut r = report();
        assert_eq!(r.exit_code(), 0);
        let Asked::Asked(status) = &mut r.status else {
            unreachable!()
        };
        status.value = None;
        status.undecodable = Some("missing field `instance`".into());
        assert_eq!(r.exit_code(), 1);
        let Asked::Asked(status) = &mut r.status else {
            unreachable!()
        };
        status.undecodable = None;
        assert_eq!(r.exit_code(), 2);
        let v = serde_json::to_value(&r).unwrap();
        assert!(v["status"].get("value").is_none());
        assert!(v["status"].get("undecodable").is_none());
    }

    fn act(answer: ConfigActAnswer) -> ConfigActReport {
        ConfigActReport {
            namespace: String::new(),
            service: "lab/rf0".into(),
            profile: ConfigProfile::V1,
            operation: "set/radio".into(),
            key: "zk2/lab/rf0/config.v1/@op/set/radio".into(),
            request: json!({"values": {"frequency_hz": 869525000}, "confirm_s": 120}),
            request_bytes: None,
            actor: Some("ops".into()),
            request_id: Some("r-1".into()),
            checked: Some(ConfigChecked {
                read: ConfigCheckedRead::View,
                typed_by: ConfigTyping::Schema,
                instance: Some("i1".into()),
                revision: Some(0),
                class: Some(Class::Reach),
            }),
            across_face: false,
            timeout_s: 2.0,
            answer,
        }
    }

    /// An act's wire shape: the union's branch as `answer`, the text's own
    /// names; a refusal's `ConfigError` decoded; silence attributed; and the
    /// exit an act keeps — 0 a reply of its type, 1 a refusal, 2 silence.
    #[test]
    fn the_config_act_report_pins_its_shape() {
        let r = act(ConfigActAnswer::Pending {
            pending: PendingReply {
                token: "i1-1".into(),
                apply_at: "2026-10-18T00:00:01.000Z".into(),
                deadline: "2026-10-18T00:02:00.000Z".into(),
            },
        });
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            json!({
                "namespace": "", "service": "lab/rf0", "operation": "set/radio",
                "key": "zk2/lab/rf0/config.v1/@op/set/radio",
                "request": {"values": {"frequency_hz": 869525000}, "confirm_s": 120},
                "actor": "ops", "request_id": "r-1",
                "checked": {"read": "view", "typed_by": "schema", "instance": "i1",
                            "revision": 0, "class": "reach"},
                "timeout_s": 2.0,
                "answer": "pending",
                "pending": {"token": "i1-1", "apply_at": "2026-10-18T00:00:01.000Z",
                            "deadline": "2026-10-18T00:02:00.000Z"},
            })
        );
        assert_eq!(r.exit_code(), 0);
        let refused = act(ConfigActAnswer::Refused {
            refusal: ConfigRefusal {
                code: "app".into(),
                message: Some(
                    "group \"modulation\" is part of what this service was started against".into(),
                ),
                cause: None,
                error: Some(ConfigAppError::V1(ConfigError {
                    code: zenkey_model::config::ErrorCode::RestartRequired,
                    detail: None,
                })),
                detail_undecodable: None,
                mark: None,
            },
        });
        let v = serde_json::to_value(&refused).unwrap();
        assert_eq!(v["answer"], "refused");
        assert_eq!(v["refusal"]["error"], json!({"code": "restart-required"}));
        assert_eq!(refused.exit_code(), 1);
        let silent = act(ConfigActAnswer::Silent {
            silence: SilenceView {
                attempts: 1,
                transport: None,
                presence: PresenceAttribution::Unobservable,
            },
        });
        let v = serde_json::to_value(&silent).unwrap();
        assert_eq!(
            v["silence"],
            json!({"attempts": 1, "presence": "unobservable"})
        );
        assert_eq!(silent.exit_code(), 2);
        assert!(v.get("across_face").is_none());
    }

    /// `config.v2`'s act (#985 G3): `profile` said, the request's bytes,
    /// and every reply the record, as the view reads it and as it is, with
    /// its size and as-of mark; its exit a reply's 0.
    #[test]
    fn a_config_v2_act_pins_its_record() {
        let state: zenkey_model::config::v2::State = serde_json::from_value(json!({
            "instance": 4660, "revision": 1, "floor_s": 10, "would_be": false,
            "pending": null, "last": {"token": 305438721u64, "groups": ["duty"]},
            "desired": null, "groups": {}
        }))
        .unwrap();
        let view: ConfigView = serde_json::from_value(json!({
            "service": "lab/rf0", "instance": "4660", "revision": 1, "confirm_floor_s": 10,
            "last_change": {"token": "305438721", "groups": ["duty"]}, "groups": []
        }))
        .unwrap();
        let mut r = act(ConfigActAnswer::Record {
            view: Box::new(view),
            record: Box::new(state),
            size: 36,
            mark: Some(InbandStamp {
                run: "1234".into(),
                seq: 41,
            }),
        });
        r.profile = ConfigProfile::V2;
        r.request_bytes = Some(9);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["profile"], "config.v2");
        assert_eq!(v["request_bytes"], 9);
        assert_eq!(v["answer"], "record");
        assert_eq!(v["record"]["last"]["token"], 305438721u64);
        assert_eq!(v["view"]["last_change"]["token"], "305438721");
        assert_eq!(
            (v["size"].clone(), v["mark"].clone()),
            (json!(36), json!({"run": "1234", "seq": 41}))
        );
        assert_eq!(r.exit_code(), 0);
    }

    /// `config.v2`'s refusal (zk2py round 102, F-485): its `Error` code as
    /// the packed enum names it, never `config.v1`'s spelling, the escape
    /// `null`, and a message the error form did not carry absent, never
    /// `""` (v2 §2.5, core §7.5).
    #[test]
    fn a_config_v2_refusal_pins_its_own_code_and_no_message() {
        let refused = |code: Option<&str>| {
            let mut r = act(ConfigActAnswer::Refused {
                refusal: ConfigRefusal {
                    code: "app".into(),
                    message: None,
                    cause: None,
                    error: Some(ConfigAppError::V2 {
                        code: code.map(str::to_owned),
                    }),
                    detail_undecodable: None,
                    mark: Some(InbandStamp {
                        run: "1234".into(),
                        seq: 7,
                    }),
                },
            });
            r.profile = ConfigProfile::V2;
            serde_json::to_value(&r).unwrap()
        };
        assert_eq!(
            refused(Some("device_refused"))["refusal"],
            json!({"code": "app", "error": {"code": "device_refused"},
                   "mark": {"run": "1234", "seq": 7}})
        );
        assert_eq!(refused(None)["refusal"]["error"], json!({"code": null}));
        let escape = ConfigAppError::V2 { code: None };
        assert_eq!(escape.type_name(), "Error");
        assert!(escape.code().starts_with("the escape"));
    }
}
