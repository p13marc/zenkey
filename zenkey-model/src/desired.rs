//! `desired.v1`: desired documents, from their author to their target
//! (`spec/profiles/desired/v1.md`, #840).
//!
//! The profile's session-free half: the declaration read from a resolved
//! resource (§2.2), a requirement's binding and its slice's key (§2.3), a
//! stamp as a report writes it (§2.4), the standard `Report` with its
//! reading and its consistency (§2.10), a target's state machine (§2.6 to
//! §2.10), pure and two-phase like [`crate::config::Owner`], and §5's four
//! questions as judgements in three states (§2.17).
//!
//! ```text
//! declaration(kind, template, annotations) ─▶ declared [targets] │ invalid [reasons] │ not this profile's   §2.2
//! binding(slice, requirement, own address) ─▶ bound {key} │ unbound (the kill switch) │ invalid [problems] §2.3
//! parse_stamp(text) / Stamp's Display       ─▶ <time>/<id>, as zenoh writes a timestamp                    §2.4
//! decode(bytes), Report::from_json(v)       ─▶ Report │ Unreadable{json, schema, stamp, revision_range}    §2.10
//! Report::check()                           ─▶ the findings: keyless, key_form, observed_outcome, …        §2.10
//! Content::of_payload(bytes, json)          ─▶ a put's document, JCS-equal for JSON, else byte-equal       §2.7
//! Target::observe(sample)                   ─▶ act │ revert │ adopted │ held │ unstamped │ same_version │ older │ confirmed  §2.6–§2.9
//! Target::finish(ended)                     ─▶ the report updated; done, or the held version adopted or acted on  §2.7, §2.10
//! Target::call()                            ─▶ source call                                                 §2.7
//! converged(target, author)                 ─▶ §5 "has the target converged on its author's current document?"
//! refused(target) / last_known(target) / in_force(target) ─▶ §5's other three questions              §2.17
//! ```
//!
//! There is no I/O and no clock here. A target's runtime subscribes, seeds
//! and reads an archive (§2.5, §2.9), hands each sample to
//! [`Target::observe`] as it arrives, performs each [`Act`] on its own
//! application (decode, check, apply or revert), tells
//! [`Target::finish`] how it ended, and puts [`Target::report`] whenever it
//! changes (§2.10). An author's writes, its diff and its record (§2.11) are
//! the runtime's.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::authoring::Kind;
use crate::contract::Resource;
use crate::freshness;
use crate::grammar::{self, Addr, IfaceId, Key, KindToken, ZkKey};
use crate::seed::Stamp;
use crate::template::{Bindings, Template};

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "desired.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "desired";

/// `desired.target_param` (§4).
pub const TARGET_PARAM: &str = "desired.target_param";

/// The published vocabulary (§4): the keys under `desired.`, which W105
/// reads in place of core Appendix D's interim row (core 0.37). The interim
/// `target` is retired, so W105 warns of it.
pub const VOCABULARY: &[&str] = &["target_param"];

/// The standard schema's stem (§3): `desired.v1.json`.
pub const SCHEMA_STEM: &str = "desired.v1";
/// The report's type name (§2.10, §3).
pub const REPORT: &str = "desired.v1.Report";
/// A version's type name (§2.4, §3).
pub const VERSION: &str = "desired.v1.Version";
/// A refusal's type name (§2.10, §3).
pub const REFUSAL: &str = "desired.v1.Refusal";

/// The standard schema's artifact ids the profile publishes (§3), oldest
/// first. A type of one of these artifacts is the profile's.
pub const PUBLISHED: &[&str] =
    &["sha256:0b249e71ab1b20cad122aecda2ef02c62e97a6e7db13a47e33ef0cf4f89856cd"];

/// Whether `id` is one of the standard schema's published artifact ids.
#[must_use]
pub fn is_published(id: &str) -> bool {
    PUBLISHED.contains(&id)
}

/// R2's spelling of the target's own system (core §3.2).
pub const SELF_SYSTEM: &str = "self.system";
/// R2's spelling of the target's own service (core §3.2).
pub const SELF_SERVICE: &str = "self.service";

/// The longest refusal code, in characters (§3).
pub const MAX_CODE: usize = 64;

/// The profile's refusal codes (§2.7, §2.10): the document does not decode
/// as its type.
pub const UNDECODABLE: &str = "undecodable";
/// It names what the target needs to reach the bus.
pub const REACH: &str = "reach";
/// It carries a sensitive value: a secret, a credential, a key
/// (`config.v1`'s word).
pub const SENSITIVE: &str = "sensitive";
/// Its type, or the target's own validation of it, refuses it.
pub const INVALID: &str = "invalid";
/// An act that could not finish.
pub const APPLY: &str = "apply";

// --- §2.2: the declaration -----------------------------------------------------

/// Which identity of the target fills a target parameter (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binds {
    /// The one target parameter: `self.system` or `self.service`, as the
    /// deployment chooses.
    Either,
    /// The first of two: `self.system`.
    System,
    /// The second of two: `self.service`.
    Service,
}

impl Binds {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Either => "either",
            Self::System => SELF_SYSTEM,
            Self::Service => SELF_SERVICE,
        }
    }

    /// Whether a parameter binding spelled `bound` (core R2) is this one.
    #[must_use]
    pub fn admits(self, bound: &str) -> bool {
        match self {
            Self::Either => bound == SELF_SYSTEM || bound == SELF_SERVICE,
            Self::System => bound == SELF_SYSTEM,
            Self::Service => bound == SELF_SERVICE,
        }
    }
}

/// One target parameter (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetParam {
    pub param: String,
    pub binds: Binds,
}

/// Why a declaration is invalid (§2.2), in the order checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invalid {
    /// The resource is not state.
    NotState,
    /// Not a string, nor an array of one or two strings.
    Value,
    /// A name twice.
    Repeated,
    /// A name that is no parameter of the template.
    NotAParam,
    /// A name of a rest parameter.
    RestParam,
    /// A template parameter the value does not name.
    ParamNotTarget,
}

impl Invalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotState => "not_state",
            Self::Value => "value",
            Self::Repeated => "repeated",
            Self::NotAParam => "not_a_param",
            Self::RestParam => "rest_param",
            Self::ParamNotTarget => "param_not_target",
        }
    }

    /// What it says, for a linter's line.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotState => "a desired document is state of its author",
            Self::Value => "desired.target_param is a string, or an array of one or two strings",
            Self::Repeated => "a target parameter is named twice",
            Self::NotAParam => "a name is no parameter of the template",
            Self::RestParam => "a rest parameter cannot be a target parameter",
            Self::ParamNotTarget => {
                "every template parameter must be a target parameter: one document per target"
            }
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.says())
    }
}

/// A resource's declaration (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// No `desired.target_param`.
    NotThisProfiles,
    /// The reasons, in the order checked.
    Invalid(Vec<Invalid>),
    /// The target parameters, in the value's order.
    Declared(Vec<TargetParam>),
}

impl Declaration {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotThisProfiles => "not_this_profiles",
            Self::Invalid(_) => "invalid",
            Self::Declared(_) => "declared",
        }
    }

    /// The target parameters of a valid declaration.
    #[must_use]
    pub fn targets(&self) -> Option<&[TargetParam]> {
        match self {
            Self::Declared(t) => Some(t),
            _ => None,
        }
    }
}

/// Reads the declaration of a resource of `kind` and `template` from its
/// resolved annotations (§2.2, core §9.3). Each check runs alone, and a
/// check that needs a value an earlier one found wrong is skipped. A
/// horizon, whatever its value, makes no declaration invalid: its re-puts
/// are the same document, which a target adopts (§2.7).
#[must_use]
pub fn declaration(
    kind: Kind,
    template: &Template,
    annotations: &BTreeMap<String, Value>,
) -> Declaration {
    let Some(v) = annotations.get(TARGET_PARAM) else {
        return Declaration::NotThisProfiles;
    };
    let params: Vec<(&str, bool)> = template.params().collect();
    let mut why = Vec::new();
    if kind != Kind::State {
        why.push(Invalid::NotState);
    }
    let names: Option<Vec<&str>> = match v {
        Value::String(s) => Some(vec![s.as_str()]),
        Value::Array(a) if (1..=2).contains(&a.len()) => {
            a.iter().map(Value::as_str).collect::<Option<Vec<_>>>()
        }
        _ => None,
    };
    match &names {
        None => why.push(Invalid::Value),
        Some(names) => {
            if names.len() == 2 && names[0] == names[1] {
                why.push(Invalid::Repeated);
            }
            if names.iter().any(|n| !params.iter().any(|(p, _)| p == n)) {
                why.push(Invalid::NotAParam);
            }
            if names
                .iter()
                .any(|n| params.iter().any(|(p, rest)| *rest && p == n))
            {
                why.push(Invalid::RestParam);
            }
            if params.iter().any(|(p, _)| !names.contains(p)) {
                why.push(Invalid::ParamNotTarget);
            }
        }
    }
    match names {
        Some(names) if why.is_empty() => Declaration::Declared(match names.as_slice() {
            [one] => vec![TargetParam {
                param: (*one).to_owned(),
                binds: Binds::Either,
            }],
            [first, second] => vec![
                TargetParam {
                    param: (*first).to_owned(),
                    binds: Binds::System,
                },
                TargetParam {
                    param: (*second).to_owned(),
                    binds: Binds::Service,
                },
            ],
            // One or two names, by the match above.
            _ => return Declaration::Invalid(vec![Invalid::Value]),
        }),
        _ => Declaration::Invalid(why),
    }
}

/// [`declaration`] of a resource of a contract.
#[must_use]
pub fn declaration_of(r: &Resource) -> Declaration {
    declaration(r.kind, &r.template, &r.annotations)
}

// --- §2.3: the binding ---------------------------------------------------------

/// A target's requirement on a desired resource, as its descriptor lists it
/// (core §3.3).
#[derive(Debug, Clone, Copy)]
pub struct Requirement<'a> {
    /// The provider addresses, `<system>/<service>`, a chunk possibly `*`.
    pub bindings: &'a [String],
    /// The parameter bindings, as configured (core R2, R3).
    pub params: &'a BTreeMap<String, String>,
    /// Whether the role is optional (core §3.1).
    pub optional: bool,
}

/// The desired resource a requirement binds (§2.3): its interface, kind
/// token, template and target parameters.
#[derive(Debug, Clone, Copy)]
pub struct Slice<'a> {
    pub iface: &'a IfaceId,
    pub token: KindToken,
    pub template: &'a Template,
    pub targets: &'a [TargetParam],
}

/// Why a binding is invalid (§2.3), in the order checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingProblem {
    /// No binding, and the role is required.
    RequiredUnbound,
    /// More than one binding, or one with `*`, or one that is no address.
    NotOneAuthor,
    /// A target parameter `params` does not bind.
    TargetUnbound,
    /// A target parameter bound to anything but `self.system` or
    /// `self.service`.
    NotSelf,
    /// Of two target parameters, one bound to the other's self.
    WrongSelf,
}

impl BindingProblem {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RequiredUnbound => "required_unbound",
            Self::NotOneAuthor => "not_one_author",
            Self::TargetUnbound => "target_unbound",
            Self::NotSelf => "not_self",
            Self::WrongSelf => "wrong_self",
        }
    }
}

/// A requirement's binding (§2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// No binding, an optional role: the kill switch (§2.12).
    Unbound,
    /// The problems, in the order checked.
    Invalid(Vec<BindingProblem>),
    /// The target's slice: its one document's key.
    Bound { key: Key },
}

impl Binding {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unbound => "unbound",
            Self::Invalid(_) => "invalid",
            Self::Bound { .. } => "bound",
        }
    }
}

/// The one author a binding names: an exact address, or `None`.
fn one_author(bindings: &[String]) -> Option<Addr> {
    let [one] = bindings else {
        return None;
    };
    if one.split('/').any(|c| c == "*") {
        return None;
    }
    one.parse().ok()
}

/// Reads a target's requirement on a desired resource (§2.3): the kill
/// switch, a binding's problems, or the target's slice, built from the
/// author's address and the target's own address `own`, each target
/// parameter's value the target's system or service as its binding says,
/// slugged (core §1.4).
#[must_use]
pub fn binding(slice: &Slice<'_>, req: &Requirement<'_>, own: &Addr) -> Binding {
    if req.bindings.is_empty() {
        return if req.optional {
            Binding::Unbound
        } else {
            Binding::Invalid(vec![BindingProblem::RequiredUnbound])
        };
    }
    let mut why = Vec::new();
    let author = one_author(req.bindings);
    if author.is_none() {
        why.push(BindingProblem::NotOneAuthor);
    }
    let bound = |t: &TargetParam| req.params.get(&t.param).map(String::as_str);
    if slice.targets.iter().any(|t| bound(t).is_none()) {
        why.push(BindingProblem::TargetUnbound);
    }
    if slice
        .targets
        .iter()
        .filter_map(bound)
        .any(|b| b != SELF_SYSTEM && b != SELF_SERVICE)
    {
        why.push(BindingProblem::NotSelf);
    }
    if slice.targets.iter().any(|t| {
        bound(t).is_some_and(|b| (b == SELF_SYSTEM || b == SELF_SERVICE) && !t.binds.admits(b))
    }) {
        why.push(BindingProblem::WrongSelf);
    }
    let Some(author) = author.filter(|_| why.is_empty()) else {
        return Binding::Invalid(why);
    };
    let mut values = Bindings::new();
    for t in slice.targets {
        let v = if bound(t) == Some(SELF_SYSTEM) {
            own.system.as_str()
        } else {
            own.service.as_str()
        };
        values.insert(t.param.clone(), vec![v.to_owned()]);
    }
    let built = slice.template.build(&values).ok().and_then(|chunks| {
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        grammar::data_key(&author, slice.iface, slice.token, &refs).ok()
    });
    match built {
        Some(key) => Binding::Bound { key },
        // Every target parameter bound, every chunk a slug: only a template
        // that is no desired one's reaches here.
        None => Binding::Invalid(vec![BindingProblem::TargetUnbound]),
    }
}

// --- §2.4: stamps ----------------------------------------------------------------

/// Reads a stamp as a report writes it (§2.4): the NTP64 time in decimal,
/// `0` or without a leading zero, at most 2^64−1; `/`; the id in lowercase
/// hexadecimal without a leading zero, at most 32 digits. `None` for any
/// other spelling. [`Stamp`]'s `Display` writes this form.
#[must_use]
pub fn parse_stamp(s: &str) -> Option<Stamp> {
    let (time, id) = s.split_once('/')?;
    let digits = !time.is_empty() && time.bytes().all(|b| b.is_ascii_digit());
    if !digits || (time.len() > 1 && time.starts_with('0')) {
        return None;
    }
    Stamp::parse(time.parse().ok()?, id)
}

/// A stamp written as a string (§2.4).
mod stamp_text {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Stamp, parse_stamp};

    pub fn serialize<S: Serializer>(s: &Stamp, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_str(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Stamp, D::Error> {
        let s = String::deserialize(de)?;
        parse_stamp(&s).ok_or_else(|| serde::de::Error::custom(format!("{s:?} is no stamp")))
    }
}

// --- §2.10: the report -----------------------------------------------------------

/// What is in force (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The target's own configuration, with no document.
    Baseline,
    /// A document the target acted on.
    Desired,
    /// A change the target's own operation made since.
    Call,
}

impl Source {
    /// The wire's and the fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Desired => "desired",
            Self::Call => "call",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Baseline, Self::Desired, Self::Call]
            .into_iter()
            .find(|v| v.as_str() == s)
    }
}

/// What the last act did (§2.7, §2.8, §2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Applied whole.
    Converged,
    /// Nothing applied; what was in force kept.
    Refused,
    /// Applied in part.
    Failed,
    /// A delete acted on, the baseline back in force.
    Reverted,
}

impl Outcome {
    /// The wire's and the fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Converged => "converged",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Reverted => "reverted",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Converged, Self::Refused, Self::Failed, Self::Reverted]
            .into_iter()
            .find(|v| v.as_str() == s)
    }
}

/// One version of a document (§2.4): `desired.v1.Version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version {
    #[serde(with = "stamp_text")]
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
}

/// Why a document was refused, or failed to apply (§2.10):
/// `desired.v1.Refusal`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Refusal {
    /// The profile's code ([`UNDECODABLE`], [`REACH`], [`SENSITIVE`],
    /// [`INVALID`], [`APPLY`]) or the type's own, 1 to 64 characters.
    pub code: String,
    /// Where in the document, a JSON Pointer (RFC 6901).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// A line for a person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Refusal {
    /// A refusal with a code alone.
    #[must_use]
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            pointer: None,
            detail: None,
        }
    }
}

/// A target's report of one role (§2.10): `desired.v1.Report`. Its members
/// serialize as the schema names them, an absent one left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The slice's key, absent only for an unbound role (§2.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_force: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
    /// `true` when `observed` was read from an archive (§2.9).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub last_known: bool,
}

/// Why a report does not read (§2.10): the first refusal only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// Bytes that are not strict JSON.
    Json(String),
    /// Outside `desired.v1.Report`.
    Schema(String),
    /// A stamp not in zenoh's form (§2.4).
    Stamp(String),
    /// A revision above 2^64−1.
    RevisionRange(&'static str),
}

impl Unreadable {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Json(_) => "json",
            Self::Schema(_) => "schema",
            Self::Stamp(_) => "stamp",
            Self::RevisionRange(_) => "revision_range",
        }
    }
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "not strict JSON: {e}"),
            Self::Schema(e) => write!(f, "not a desired.v1.Report: {e}"),
            Self::Stamp(s) => write!(f, "{s:?} is not a stamp as zenoh writes one"),
            Self::RevisionRange(m) => write!(f, "/{m}/revision is above 2^64-1"),
        }
    }
}

/// A report that reads, but breaks §2.10's consistency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Finding {
    Keyless,
    KeyForm,
    ObservedOutcome,
    Refusal,
    Source,
    InForce,
    RevisionOnRevert,
    LastKnown,
}

impl Finding {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keyless => "keyless",
            Self::KeyForm => "key_form",
            Self::ObservedOutcome => "observed_outcome",
            Self::Refusal => "refusal",
            Self::Source => "source",
            Self::InForce => "in_force",
            Self::RevisionOnRevert => "revision_on_revert",
            Self::LastKnown => "last_known",
        }
    }
}

/// A revision as the schema reads it: an integer from 0, an integral float
/// that integer; whether it fits 64 bits is checked after the stamps.
enum RevisionRead {
    InRange(u64),
    TooLarge,
}

fn revision(v: &Value) -> Option<RevisionRead> {
    let Value::Number(n) = v else {
        return None;
    };
    if let Some(u) = n.as_u64() {
        return Some(RevisionRead::InRange(u));
    }
    if n.as_i64().is_some() {
        return None; // negative
    }
    let f = n.as_f64()?;
    if !(f.is_finite() && f >= 0.0 && f.fract() == 0.0) {
        return None;
    }
    // 2^64, exactly a double.
    if f >= 2.0_f64.powi(64) {
        return Some(RevisionRead::TooLarge);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = f as u64;
    Some(RevisionRead::InRange(n))
}

/// A version's members, read by the schema's types.
struct RawVersion<'a> {
    stamp: &'a str,
    revision: Option<RevisionRead>,
}

fn raw_version<'a>(v: &'a Value, member: &str) -> Result<RawVersion<'a>, Unreadable> {
    let schema = |why: &str| Unreadable::Schema(format!("/{member}{why}"));
    let Value::Object(o) = v else {
        return Err(schema(": not an object"));
    };
    let stamp = match o.get("stamp") {
        Some(Value::String(s)) if s.chars().count() >= 3 => s.as_str(),
        Some(Value::String(_)) => return Err(schema("/stamp: shorter than 3 characters")),
        Some(_) => return Err(schema("/stamp: not a string")),
        None => return Err(schema("/stamp: missing")),
    };
    let revision = match o.get("revision") {
        None => None,
        Some(r) => Some(revision(r).ok_or_else(|| schema("/revision: not an integer from 0"))?),
    };
    Ok(RawVersion { stamp, revision })
}

fn optional_string(
    o: &serde_json::Map<String, Value>,
    m: &str,
    at: &str,
) -> Result<Option<String>, Unreadable> {
    match o.get(m) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(Unreadable::Schema(format!("{at}/{m}: not a string"))),
    }
}

impl Report {
    /// A target's first report of a role (§2.10): its slice's key, or none
    /// for an unbound role, and the baseline in force.
    #[must_use]
    pub fn baseline(key: Option<String>) -> Self {
        Self {
            key,
            source: Source::Baseline,
            in_force: None,
            observed: None,
            outcome: None,
            refusal: None,
            last_known: false,
        }
    }

    /// Reads a decoded payload (§2.10): the schema's members, types and
    /// enums, an integral number read as an integer and a member the schema
    /// does not name ignored; then each stamp's form; then each revision's
    /// range.
    ///
    /// # Errors
    ///
    /// [`Unreadable`]: the first refusal, in that order.
    pub fn from_json(v: &Value) -> Result<Self, Unreadable> {
        let schema = |why: String| Unreadable::Schema(why);
        let Value::Object(o) = v else {
            return Err(schema("not an object".into()));
        };
        let source = match o.get("source") {
            Some(Value::String(s)) => Source::parse(s),
            _ => None,
        }
        .ok_or_else(|| schema("/source: not baseline, desired or call".into()))?;
        let key = optional_string(o, "key", "")?;
        let in_force = o
            .get("in_force")
            .map(|v| raw_version(v, "in_force"))
            .transpose()?;
        let observed = o
            .get("observed")
            .map(|v| raw_version(v, "observed"))
            .transpose()?;
        let outcome = match o.get("outcome") {
            None => None,
            Some(Value::String(s)) => Some(Outcome::parse(s).ok_or_else(|| {
                schema("/outcome: not converged, refused, failed or reverted".into())
            })?),
            Some(_) => return Err(schema("/outcome: not a string".into())),
        };
        let refusal = match o.get("refusal") {
            None => None,
            Some(Value::Object(r)) => {
                let code = match r.get("code") {
                    Some(Value::String(c)) if (1..=MAX_CODE).contains(&c.chars().count()) => {
                        c.clone()
                    }
                    Some(Value::String(_)) => {
                        return Err(schema("/refusal/code: not 1 to 64 characters".into()));
                    }
                    Some(_) => return Err(schema("/refusal/code: not a string".into())),
                    None => return Err(schema("/refusal/code: missing".into())),
                };
                Some(Refusal {
                    code,
                    pointer: optional_string(r, "pointer", "/refusal")?,
                    detail: optional_string(r, "detail", "/refusal")?,
                })
            }
            Some(_) => return Err(schema("/refusal: not an object".into())),
        };
        let last_known = match o.get("last_known") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(schema("/last_known: not a boolean".into())),
        };
        let stamp = |r: &RawVersion<'_>| {
            parse_stamp(r.stamp).ok_or_else(|| Unreadable::Stamp(r.stamp.to_owned()))
        };
        let in_force_stamp = in_force.as_ref().map(stamp).transpose()?;
        let observed_stamp = observed.as_ref().map(stamp).transpose()?;
        let rev = |r: &RawVersion<'_>, m: &'static str| match &r.revision {
            None => Ok(None),
            Some(RevisionRead::InRange(n)) => Ok(Some(*n)),
            Some(RevisionRead::TooLarge) => Err(Unreadable::RevisionRange(m)),
        };
        let in_force = match (&in_force, in_force_stamp) {
            (Some(r), Some(stamp)) => Some(Version {
                stamp,
                revision: rev(r, "in_force")?,
            }),
            _ => None,
        };
        let observed = match (&observed, observed_stamp) {
            (Some(r), Some(stamp)) => Some(Version {
                stamp,
                revision: rev(r, "observed")?,
            }),
            _ => None,
        };
        Ok(Self {
            key,
            source,
            in_force,
            observed,
            outcome,
            refusal,
            last_known,
        })
    }

    /// The report's findings (§2.10), every one that applies, in the order
    /// checked. A report with one is malformed, and answers nothing
    /// (§2.17).
    #[must_use]
    pub fn check(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        if self.key.is_none() && (self.source != Source::Baseline || self.observed.is_some()) {
            out.push(Finding::Keyless);
        }
        if let Some(k) = &self.key
            && !matches!(
                grammar::parse(k),
                Ok(ZkKey::Data {
                    kind: KindToken::State | KindToken::ExplicitState,
                    ..
                })
            )
        {
            out.push(Finding::KeyForm);
        }
        if self.observed.is_some() != self.outcome.is_some() {
            out.push(Finding::ObservedOutcome);
        }
        let refusing = matches!(self.outcome, Some(Outcome::Refused | Outcome::Failed));
        if self.refusal.is_some() != refusing {
            out.push(Finding::Refusal);
        }
        if (self.source == Source::Desired && self.observed.is_none())
            || (self.outcome == Some(Outcome::Converged) && self.source == Source::Baseline)
            || (self.outcome == Some(Outcome::Reverted) && self.source == Source::Desired)
        {
            out.push(Finding::Source);
        }
        let misplaced = self.in_force.is_some()
            && (self.source != Source::Desired || self.outcome == Some(Outcome::Failed));
        let not_observed = self.outcome == Some(Outcome::Converged)
            && self.source == Source::Desired
            && self.in_force != self.observed;
        if misplaced || not_observed {
            out.push(Finding::InForce);
        }
        if self.outcome == Some(Outcome::Reverted)
            && self.observed.is_some_and(|o| o.revision.is_some())
        {
            out.push(Finding::RevisionOnRevert);
        }
        if self.last_known && self.observed.is_none() {
            out.push(Finding::LastKnown);
        }
        out
    }

    /// The report as JSON, as a target puts it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// Reads a report's bytes (§2.10): strict JSON first, as `alarms.v1` §2.4
/// reads a document (UTF-8, RFC 8259, nothing after the value but
/// whitespace, no duplicate member at any depth, every number finite), then
/// [`Report::from_json`].
///
/// # Errors
///
/// [`Unreadable::Json`] for bytes that are not strict JSON, else
/// [`Report::from_json`]'s.
pub fn decode(bytes: &[u8]) -> Result<Report, Unreadable> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| Unreadable::Json(format!("not UTF-8: {e}")))?;
    let v = crate::strict::parse_json(text).map_err(|e| Unreadable::Json(e.to_string()))?;
    Report::from_json(&v)
}

// --- §2.6–§2.10: a target ------------------------------------------------------

/// Which path a sample came by (§2.5, §2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Via {
    /// The author's own answer, or its put.
    Owner,
    /// An archive's last-known (core S5, S6).
    Archive,
}

/// A sample's kind: a put, or a delete or a `reply_del`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutation {
    Put,
    Delete,
}

/// A put's document, as a target compares two (§2.7): the SHA-256 of its
/// JCS bytes (RFC 8785) for a payload encoded as JSON that reads as strict
/// JSON, else of its bytes. Equal digests are the same document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Content([u8; 32]);

impl Content {
    fn digest(bytes: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        let mut d = [0u8; 32];
        d.copy_from_slice(&Sha256::digest(bytes));
        Self(d)
    }

    /// A decoded JSON document, compared by its JCS bytes.
    #[must_use]
    pub fn of_json(v: &Value) -> Self {
        Self::digest(&crate::schema::jcs(v))
    }

    /// A payload compared byte for byte: CBOR, protobuf, raw.
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self::digest(bytes)
    }

    /// A payload as a target compares it (§2.7): one encoded as JSON
    /// (`json`) by its JCS bytes when it reads as strict JSON, and every
    /// other payload, or JSON that does not read, byte for byte.
    #[must_use]
    pub fn of_payload(bytes: &[u8], json: bool) -> Self {
        let read = json
            .then(|| std::str::from_utf8(bytes).ok())
            .flatten()
            .and_then(|t| crate::strict::parse_json(t).ok());
        match read {
            Some(v) => Self::of_json(&v),
            None => Self::of_bytes(bytes),
        }
    }
}

/// One sample or reply of a target's slice, as its runtime received it
/// (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub via: Via,
    pub mutation: Mutation,
    /// The author's stamp, `None` when the sample carries none.
    pub stamp: Option<Stamp>,
    /// The document type's own revision, when it carries one (§2.4). A
    /// delete has none.
    pub revision: Option<u64>,
    /// A put's document ([`Content`]); `None` for a delete, and for a put
    /// whose runtime did not compare it, which is then never the same
    /// document as another.
    pub content: Option<Content>,
}

/// What an act does (§2.7, §2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActKind {
    /// Decode, check and apply the document.
    Apply,
    /// Apply the baseline.
    Revert,
}

/// An act the target's runtime performs, then reports by
/// [`Target::finish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Act {
    pub kind: ActKind,
    pub stamp: Stamp,
    pub revision: Option<u64>,
    pub via: Via,
    pub content: Option<Content>,
}

impl Act {
    fn of(s: &Sample, stamp: Stamp) -> Self {
        Self {
            kind: match s.mutation {
                Mutation::Put => ActKind::Apply,
                Mutation::Delete => ActKind::Revert,
            },
            stamp,
            revision: match s.mutation {
                Mutation::Put => s.revision,
                Mutation::Delete => None,
            },
            via: s.via,
            content: match s.mutation {
                Mutation::Put => s.content,
                Mutation::Delete => None,
            },
        }
    }

    /// Whether `next` carries the same document as this act (§2.7): two
    /// puts whose documents compare equal, or two deletes.
    #[must_use]
    pub fn same_document(&self, next: &Act) -> bool {
        match (self.kind, next.kind) {
            (ActKind::Revert, ActKind::Revert) => true,
            (ActKind::Apply, ActKind::Apply) => {
                self.content.is_some() && self.content == next.content
            }
            _ => false,
        }
    }

    /// The act's version, as the report writes it.
    #[must_use]
    pub fn version(&self) -> Version {
        Version {
            stamp: self.stamp,
            revision: self.revision,
        }
    }
}

/// How an act ended (§2.7, §2.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// Applied whole: a put converged, a delete reverted.
    Done,
    /// Refused whole, nothing applied. A revert is never refused: a
    /// refusal of one is read as [`Ended::Failed`].
    Refused(Refusal),
    /// Applied in part.
    Failed(Refusal),
}

/// What a step of a target does (§2.6–§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Start this act now.
    Act(Act),
    /// Admitted, and the same document as the last version acted on or
    /// adopted: no act, and the report moved (§2.7).
    Adopted,
    /// Admitted while an act runs: held, replacing any version held before.
    Held,
    /// No stamp: never acted on (§2.6 step 1).
    Unstamped,
    /// The version held, received again (§2.6 step 3).
    SameVersion,
    /// An earlier time of the held version's id (§2.6 step 3).
    Older,
    /// The author's own sample of a version held from an archive (§2.9).
    /// When no act runs, the report changed: its `last_known` went.
    Confirmed,
    /// An act ended, and nothing is held.
    Done,
}

impl Step {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Act(Act {
                kind: ActKind::Apply,
                ..
            }) => "act",
            Self::Act(Act {
                kind: ActKind::Revert,
                ..
            }) => "revert",
            Self::Adopted => "adopted",
            Self::Held => "held",
            Self::Unstamped => "unstamped",
            Self::SameVersion => "same_version",
            Self::Older => "older",
            Self::Confirmed => "confirmed",
            Self::Done => "done",
        }
    }
}

/// Whether `incoming` is admitted over `held` (§2.6, core §4.3): by time
/// within one id, the first value under another id accepted. `Err` says why
/// not.
fn admits(held: &Stamp, incoming: &Stamp) -> Result<(), Step> {
    if held.id != incoming.id {
        return Ok(());
    }
    match incoming.time.cmp(&held.time) {
        std::cmp::Ordering::Greater => Ok(()),
        std::cmp::Ordering::Equal => Err(Step::SameVersion),
        std::cmp::Ordering::Less => Err(Step::Older),
    }
}

/// One instance of a target's role (§2.6–§2.10): the version it last acted
/// on or adopted, the one it holds while an act runs, and its report. A
/// restart is a new `Target`, holding nothing (§2.7).
#[derive(Debug, Clone)]
pub struct Target {
    acted: Option<Act>,
    running: bool,
    waiting: Option<Act>,
    report: Report,
}

impl Target {
    /// A role bound to the slice `key`, or unbound (`None`, §2.12), at its
    /// start: nothing held, its report the baseline's.
    #[must_use]
    pub fn new(key: Option<String>) -> Self {
        Self {
            acted: None,
            running: false,
            waiting: None,
            report: Report::baseline(key),
        }
    }

    /// The report, as the target puts it (§2.10).
    #[must_use]
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// The act that runs, if one does.
    #[must_use]
    pub fn running(&self) -> Option<&Act> {
        self.acted.as_ref().filter(|_| self.running)
    }

    /// Judges a sample (§2.6) against the version held waiting, else the
    /// last acted on or adopted: an act to start, adopted, held while one
    /// runs, or ignored with why. After [`Step::Adopted`] the report
    /// changed.
    pub fn observe(&mut self, s: Sample) -> Step {
        let Some(stamp) = s.stamp else {
            return Step::Unstamped;
        };
        let waiting = self.waiting.is_some();
        let reference = if waiting {
            self.waiting.as_mut()
        } else {
            self.acted.as_mut()
        };
        if let Some(r) = reference {
            match admits(&r.stamp, &stamp) {
                Ok(()) => {}
                Err(Step::SameVersion) if s.via == Via::Owner && r.via == Via::Archive => {
                    r.via = Via::Owner;
                    if !waiting && !self.running {
                        self.report.last_known = false;
                    }
                    return Step::Confirmed;
                }
                Err(why) => return why,
            }
        }
        let act = Act::of(&s, stamp);
        if self.running {
            self.waiting = Some(act);
            return Step::Held;
        }
        self.start(act)
    }

    fn start(&mut self, act: Act) -> Step {
        if let Some(prev) = self.acted
            && prev.same_document(&act)
        {
            let r = &mut self.report;
            if r.in_force == Some(prev.version()) {
                r.in_force = Some(act.version());
            }
            r.observed = Some(act.version());
            r.last_known = act.via == Via::Archive;
            self.acted = Some(act);
            return Step::Adopted;
        }
        self.acted = Some(act);
        self.running = true;
        Step::Act(act)
    }

    /// Ends the running act (§2.7, §2.10): the report updated, then the
    /// held version, if one is held, adopted or its act started, else
    /// [`Step::Done`]. With no act running, nothing changes, and it is
    /// [`Step::Done`].
    pub fn finish(&mut self, ended: Ended) -> Step {
        let Some(act) = self.acted.filter(|_| self.running) else {
            return Step::Done;
        };
        self.running = false;
        let r = &mut self.report;
        r.observed = Some(act.version());
        r.refusal = None;
        r.last_known = act.via == Via::Archive;
        match (act.kind, ended) {
            (ActKind::Apply, Ended::Done) => {
                r.source = Source::Desired;
                r.in_force = Some(act.version());
                r.outcome = Some(Outcome::Converged);
            }
            (ActKind::Revert, Ended::Done) => {
                r.source = Source::Baseline;
                r.in_force = None;
                r.outcome = Some(Outcome::Reverted);
            }
            (ActKind::Apply, Ended::Refused(why)) => {
                r.outcome = Some(Outcome::Refused);
                r.refusal = Some(why);
            }
            (kind, Ended::Refused(why) | Ended::Failed(why)) => {
                r.source = match kind {
                    ActKind::Apply => Source::Desired,
                    ActKind::Revert => Source::Baseline,
                };
                r.in_force = None;
                r.outcome = Some(Outcome::Failed);
                r.refusal = Some(why);
            }
        }
        match self.waiting.take() {
            Some(next) => self.start(next),
            None => Step::Done,
        }
    }

    /// The target's own operation changed what is in force (§2.7): the
    /// report's `source` is `call`, and no version is wholly in force. The
    /// last act's report stands, and no version is acted on again.
    pub fn call(&mut self) {
        self.report.source = Source::Call;
        self.report.in_force = None;
    }
}

// --- §2.17, §5: what a tool may conclude ---------------------------------------

/// A participant's presence, as the tool saw it (core §8.1, §8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seen {
    /// Its instance token was seen.
    Present,
    /// A complete read held no instance token of it.
    Absent,
    /// Not seen, in a read possibly incomplete or refused.
    Incomplete,
    /// Across a constrained face, where presence does not cross and the
    /// deployment's word stands for the descriptor.
    AcrossFace,
}

/// What the target's descriptor says of the role (core §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Listing {
    /// Its `requires` do not list the role.
    NotListed,
    /// They list it, bound (`bound`) or with `"bindings": []`.
    Listed { bound: bool },
}

/// The target's report of the role's slice, as the tool read it (§2.10).
#[derive(Debug, Clone, Copy)]
pub enum ReportRead<'a> {
    /// None read.
    NotRead,
    /// Bytes that do not read ([`decode`] refused them).
    Unreadable,
    /// A report that reads; malformed when [`Report::check`] finds anything.
    Read(&'a Report),
}

/// What the tool read of the target (§2.17 steps 1 to 4).
#[derive(Debug, Clone, Copy)]
pub struct TargetReading<'a> {
    pub presence: Seen,
    /// Its descriptor, `None` when not read. Ignored across a face.
    pub descriptor: Option<Listing>,
    pub report: ReportRead<'a>,
    /// The report's freshness by `freshness.v1`, [`freshness::Verdict::NotAsked`]
    /// when its resource declares no horizon.
    pub freshness: freshness::Verdict,
}

/// The author's latest version of the slice, by S4's GET (§2.17 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Current {
    /// No GET made.
    NotRead,
    /// A GET that ended at its timeout or with an error reply.
    Incomplete,
    /// A GET that drew nothing for the key.
    Silent,
    /// Its latest put.
    Put(Stamp),
    /// Its latest delete, or `reply_del`.
    Delete(Stamp),
}

/// What the tool read of the author (§2.17 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorReading {
    pub presence: Seen,
    pub current: Current,
}

/// A question of §5, answered in its states: the two poles, not yet, and
/// the two unestablished kinds apart (core O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    No,
    /// The target has not acted on the author's current version yet.
    NotYet,
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
            Self::NotYet => "not_yet",
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

/// Why an answer is what it is: the fixture's reason (§2.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    TargetAbsent,
    PresenceIncomplete,
    NoDescriptor,
    NotATarget,
    Unbound,
    NoReport,
    NothingCrossed,
    ReportInvalid,
    ReportStale,
    ReportUnobservable,
    NotActed,
    Converged,
    Refused,
    Failed,
    Reverted,
    LastKnown,
    Current,
    Desired,
    Partial,
    Baseline,
    Call,
    AuthorAbsent,
    AuthorPresenceIncomplete,
    AuthorNotRead,
    AuthorReadIncomplete,
    AuthorSilent,
    Behind,
    Ahead,
    OtherEpoch,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TargetAbsent => "target_absent",
            Self::PresenceIncomplete => "presence_incomplete",
            Self::NoDescriptor => "no_descriptor",
            Self::NotATarget => "not_a_target",
            Self::Unbound => "unbound",
            Self::NoReport => "no_report",
            Self::NothingCrossed => "nothing_crossed",
            Self::ReportInvalid => "report_invalid",
            Self::ReportStale => "report_stale",
            Self::ReportUnobservable => "report_unobservable",
            Self::NotActed => "not_acted",
            Self::Converged => "converged",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Reverted => "reverted",
            Self::LastKnown => "last_known",
            Self::Current => "current",
            Self::Desired => "desired",
            Self::Partial => "partial",
            Self::Baseline => "baseline",
            Self::Call => "call",
            Self::AuthorAbsent => "author_absent",
            Self::AuthorPresenceIncomplete => "author_presence_incomplete",
            Self::AuthorNotRead => "author_not_read",
            Self::AuthorReadIncomplete => "author_read_incomplete",
            Self::AuthorSilent => "author_silent",
            Self::Behind => "behind",
            Self::Ahead => "ahead",
            Self::OtherEpoch => "other_epoch",
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

/// §2.17 steps 1 to 4, shared by every question: the report to answer
/// from, or the answer. `unbound` is the answer for an unbound role.
fn premises<'a>(t: &TargetReading<'a>, unbound: Judged) -> Result<&'a Report, Judged> {
    use Answer::{NotAsked, Unobservable};
    match t.presence {
        Seen::Absent => return Err(Judged::new(NotAsked, Reason::TargetAbsent)),
        Seen::Incomplete => return Err(Judged::new(Unobservable, Reason::PresenceIncomplete)),
        Seen::AcrossFace => {}
        Seen::Present => match t.descriptor {
            None => return Err(Judged::new(Unobservable, Reason::NoDescriptor)),
            Some(Listing::NotListed) => return Err(Judged::new(NotAsked, Reason::NotATarget)),
            Some(Listing::Listed { bound: false }) => return Err(unbound),
            Some(Listing::Listed { bound: true }) => {}
        },
    }
    let report = match t.report {
        ReportRead::NotRead => {
            return Err(Judged::new(
                Unobservable,
                if t.presence == Seen::AcrossFace {
                    Reason::NothingCrossed
                } else {
                    Reason::NoReport
                },
            ));
        }
        ReportRead::Unreadable => return Err(Judged::new(Unobservable, Reason::ReportInvalid)),
        ReportRead::Read(r) if !r.check().is_empty() => {
            return Err(Judged::new(Unobservable, Reason::ReportInvalid));
        }
        ReportRead::Read(r) => r,
    };
    match t.freshness {
        freshness::Verdict::Stale => Err(Judged::new(Unobservable, Reason::ReportStale)),
        freshness::Verdict::Unobservable => {
            Err(Judged::new(Unobservable, Reason::ReportUnobservable))
        }
        freshness::Verdict::Fresh | freshness::Verdict::NotAsked => Ok(report),
    }
}

const UNBOUND: Judged = Judged::new(Answer::NotAsked, Reason::Unbound);

fn outcome_reason(o: Outcome) -> Reason {
    match o {
        Outcome::Converged => Reason::Converged,
        Outcome::Refused => Reason::Refused,
        Outcome::Failed => Reason::Failed,
        Outcome::Reverted => Reason::Reverted,
    }
}

/// §5's second question, "is the target's last act refused or failed?"
/// (§2.17). Yes is the finding.
#[must_use]
pub fn refused(t: &TargetReading<'_>) -> Judged {
    let r = match premises(t, UNBOUND) {
        Ok(r) => r,
        Err(j) => return j,
    };
    match r.outcome {
        None => Judged::new(Answer::NotAsked, Reason::NotActed),
        Some(o @ (Outcome::Refused | Outcome::Failed)) => {
            Judged::new(Answer::Yes, outcome_reason(o))
        }
        Some(o) => Judged::new(Answer::No, outcome_reason(o)),
    }
}

/// §5's third question, "is what the target acted on last-known?" (§2.9,
/// §2.17). Informative.
#[must_use]
pub fn last_known(t: &TargetReading<'_>) -> Judged {
    let r = match premises(t, UNBOUND) {
        Ok(r) => r,
        Err(j) => return j,
    };
    match (&r.observed, r.last_known) {
        (None, _) => Judged::new(Answer::NotAsked, Reason::NotActed),
        (Some(_), true) => Judged::new(Answer::Yes, Reason::LastKnown),
        (Some(_), false) => Judged::new(Answer::No, Reason::Current),
    }
}

/// §5's fourth question, "is a desired document in force?" (§2.10,
/// §2.12, §2.17). An unbound role answers no: the kill switch. Informative.
#[must_use]
pub fn in_force(t: &TargetReading<'_>) -> Judged {
    let r = match premises(t, Judged::new(Answer::No, Reason::Unbound)) {
        Ok(r) => r,
        Err(j) => return j,
    };
    match r.source {
        Source::Desired if r.in_force.is_some() => Judged::new(Answer::Yes, Reason::Desired),
        Source::Desired => Judged::new(Answer::Yes, Reason::Partial),
        Source::Baseline => Judged::new(Answer::No, Reason::Baseline),
        Source::Call => Judged::new(Answer::No, Reason::Call),
    }
}

/// §5's first question, "has the target converged on its author's current
/// document?" (§2.17): the premises, then the author, then the comparison
/// of the report's `observed` with the author's version by value. No is the
/// finding.
#[must_use]
pub fn converged(t: &TargetReading<'_>, a: &AuthorReading) -> Judged {
    use Answer::{No, NotAsked, NotYet, Unobservable, Yes};
    let r = match premises(t, UNBOUND) {
        Ok(r) => r,
        Err(j) => return j,
    };
    match a.presence {
        Seen::Absent => return Judged::new(NotAsked, Reason::AuthorAbsent),
        Seen::Incomplete => return Judged::new(Unobservable, Reason::AuthorPresenceIncomplete),
        Seen::Present | Seen::AcrossFace => {}
    }
    let current = match a.current {
        Current::NotRead => return Judged::new(Unobservable, Reason::AuthorNotRead),
        Current::Incomplete => return Judged::new(Unobservable, Reason::AuthorReadIncomplete),
        Current::Silent => return Judged::new(Unobservable, Reason::AuthorSilent),
        Current::Put(s) | Current::Delete(s) => s,
    };
    let (Some(observed), Some(outcome)) = (&r.observed, r.outcome) else {
        return Judged::new(NotYet, Reason::NotActed);
    };
    let o = observed.stamp;
    if o == current {
        return match outcome {
            Outcome::Converged | Outcome::Reverted => Judged::new(Yes, outcome_reason(outcome)),
            Outcome::Refused | Outcome::Failed => Judged::new(No, outcome_reason(outcome)),
        };
    }
    if o.id != current.id {
        return Judged::new(NotYet, Reason::OtherEpoch);
    }
    if o.time < current.time {
        Judged::new(NotYet, Reason::Behind)
    } else {
        Judged::new(Unobservable, Reason::Ahead)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(time: u64, id: &str) -> Stamp {
        Stamp::parse(time, id).expect("a stamp")
    }

    fn put(time: u64, id: &str) -> Sample {
        Sample {
            via: Via::Owner,
            mutation: Mutation::Put,
            stamp: Some(st(time, id)),
            revision: Some(time),
            content: Some(Content::of_json(&serde_json::json!({"revision": time}))),
        }
    }

    #[test]
    fn stamps_read_as_zenoh_writes_them() {
        assert_eq!(
            parse_stamp("7386690599959157260/33"),
            Some(st(7_386_690_599_959_157_260, "33"))
        );
        assert_eq!(st(7, "1bc0").to_string(), "7/1bc0");
        assert_eq!(parse_stamp("0/1"), Some(st(0, "1")));
        assert_eq!(
            parse_stamp(&format!("{}/1", u64::MAX)),
            Some(st(u64::MAX, "1"))
        );
        for bad in [
            "",
            "1",
            "/1",
            "1/",
            "01/1",
            "1/01",
            "1/A",
            "+1/1",
            "1/0",
            "18446744073709551616/1",
            "1/1/1",
            " 1/1",
        ] {
            assert_eq!(parse_stamp(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_report_serializes_as_the_schema_names_it() {
        let mut t = Target::new(Some("zk2/a/b/i.v1/state/x".into()));
        assert_eq!(
            t.report().to_json(),
            serde_json::json!({"key": "zk2/a/b/i.v1/state/x", "source": "baseline"})
        );
        assert!(matches!(t.observe(put(100, "a1")), Step::Act(_)));
        t.finish(Ended::Done);
        let v = t.report().to_json();
        assert_eq!(
            v,
            serde_json::json!({
                "key": "zk2/a/b/i.v1/state/x",
                "source": "desired",
                "in_force": {"stamp": "100/a1", "revision": 100},
                "observed": {"stamp": "100/a1", "revision": 100},
                "outcome": "converged",
            })
        );
        let back: Report = serde_json::from_value(v.clone()).expect("serde reads it back");
        assert_eq!(&back, t.report());
        assert_eq!(Report::from_json(&v).as_ref(), Ok(t.report()));
        assert!(t.report().check().is_empty());
    }

    #[test]
    fn every_report_the_machine_makes_is_consistent() {
        let mut t = Target::new(Some("zk2/a/b/i.v1/state/x".into()));
        let mut seen = Vec::new();
        let steps = vec![
            put(100, "a1"),
            put(200, "a1"),
            Sample {
                mutation: Mutation::Delete,
                revision: None,
                ..put(300, "a1")
            },
            Sample {
                via: Via::Archive,
                ..put(400, "b2")
            },
        ];
        for (i, s) in steps.into_iter().enumerate() {
            assert!(
                matches!(t.observe(s), Step::Act(_)),
                "a new document is acted on"
            );
            // The same document again, under a later stamp, while the act runs.
            let again = Sample {
                stamp: s.stamp.map(|x| Stamp {
                    time: x.time + 1,
                    ..x
                }),
                ..s
            };
            assert_eq!(t.observe(again), Step::Held);
            let ended = match i % 3 {
                0 => Ended::Done,
                1 => Ended::Refused(Refusal::new(REACH)),
                _ => Ended::Failed(Refusal::new(APPLY)),
            };
            assert_eq!(
                t.finish(ended),
                Step::Adopted,
                "the held re-put is the same document"
            );
            seen.push(t.report().clone());
            t.call();
            seen.push(t.report().clone());
        }
        for r in &seen {
            assert!(r.check().is_empty(), "{r:?}: {:?}", r.check());
        }
    }
}
