//! The configuration convention (RFC 05 §5.1, v1.42): the wire shapes a
//! producer's configuration plane serves and a consumer reads, and the one
//! validator every producer applies before its device sees a change.
//!
//! RFC 05 §5 has sanctioned the skeleton since v1.0 — `<topic>/set` (write),
//! `<topic>` (read), a `state` echo — and adopters each invented the rest.
//! This module is the rest, once: a typed schema a tool can render without
//! being compiled against the producer; groups as the resource an ACL sees;
//! three parameter classes and the sensitive flag; the change request with
//! its revision, idempotency key, dry run and confirm window; the read-back
//! document; and the change event. Nothing here touches a bus or a device.
//! A producer's own crate does that, and [`ConfigSchema::validate`] runs
//! first so every producer refuses the same input in the same words.
//!
//! Everything on the wire is JSON-shaped and, under the `serde` feature,
//! serializable; under `schemars` it self-describes for `describe`
//! (RFC 08 §7), so `zenctl` can encode a request against the served schema.

use std::collections::BTreeMap;
use std::fmt;

/// What values a parameter accepts (RFC 05 §5.1).
///
/// Three kinds, and no more until something acts on a fourth: every radio
/// or interface parameter met so far is a switch, a bounded integer in a
/// stated unit, or text. No float — a value that needs one is an integer in
/// a finer unit (`milli-dBm`), which also keeps equality exact.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ParamKind {
    /// `true` or `false`.
    Bool,
    /// A whole number, optionally bounded, in a stated unit.
    Integer {
        /// Smallest accepted value, inclusive.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        min: Option<i64>,
        /// Largest accepted value, inclusive.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        max: Option<i64>,
        /// What one unit is — `"bytes"`, `"packets"`, `"dBm"`, `"s"`. `None`
        /// for a dimensionless count.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        unit: Option<String>,
    },
    /// Free text.
    Text,
}

/// Whether a group may change under a running producer (RFC 05 §5.1).
///
/// The class belongs to the producer's declaration, not to a parameter's
/// name: an MTU is `contract` on a bearer whose transport was started
/// against it and `hot` on one whose transport adapts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ParamClass {
    /// Takes effect at once, and nothing above the producer needs to know.
    /// The reply to a `set` is the read-back.
    Hot,
    /// Decides whether the producer can reach the bus at all — a frequency,
    /// a network id, an APN. A `set` is accepted only as a confirmed change
    /// (`confirm_s`), answered `{token, apply_at}` *before* it is applied,
    /// because the read-back would cross the link being changed; it rolls
    /// back at the deadline unless confirmed. Never carried by desired
    /// state.
    Reach,
    /// Part of what the producer's transport was started against. Refused
    /// at runtime, with the restart named; changed in the producer's own
    /// startup configuration.
    Contract,
}

impl ParamClass {
    /// The token as the wire spells it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            ParamClass::Hot => "hot",
            ParamClass::Reach => "reach",
            ParamClass::Contract => "contract",
        }
    }
}

/// One parameter a producer declares (RFC 05 §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ParamSpec {
    /// The name a change addresses, unique within its group.
    pub name: String,
    /// What values it accepts.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub kind: ParamKind,
    /// Write-only: never read back, published, logged or evented (RFC 8341's
    /// `default-deny-all`, as data). A PIN, a key. Its [`ParamView`] carries
    /// no `value`, and a change event redacts its edit.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub sensitive: bool,
    /// One line for the operator reading the schema cold.
    pub description: String,
}

impl ParamSpec {
    /// A parameter of the given kind.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: ParamKind, description: impl Into<String>) -> Self {
        ParamSpec {
            name: name.into(),
            kind,
            sensitive: false,
            description: description.into(),
        }
    }

    /// Mark the parameter write-only.
    #[must_use]
    pub fn sensitive(mut self) -> Self {
        self.sensitive = true;
        self
    }
}

/// A group of parameters that change together (RFC 05 §5.1): the resource
/// a `set` addresses and an ACL grants — `config/{resource}/{group}/set`.
///
/// Coupled parameters share a group because a single-parameter intermediate
/// state can be unsafe (a frequency without its bandwidth), and because a
/// group is the unit that has a class.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConfigGroup {
    /// The group's name, a plain chunk: it is spelled in the write key.
    pub name: String,
    pub class: ParamClass,
    pub description: String,
    pub parameters: Vec<ParamSpec>,
}

impl ConfigGroup {
    /// A group with its class; parameters are pushed onto `parameters`.
    #[must_use]
    pub fn new(name: impl Into<String>, class: ParamClass, description: impl Into<String>) -> Self {
        ConfigGroup {
            name: name.into(),
            class,
            description: description.into(),
            parameters: Vec::new(),
        }
    }

    /// Builder form of pushing a parameter.
    #[must_use]
    pub fn with(mut self, spec: ParamSpec) -> Self {
        self.parameters.push(spec);
        self
    }

    /// The parameter by name, if declared.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<&ParamSpec> {
        self.parameters.iter().find(|p| p.name == name)
    }
}

/// Everything a producer declares about one configurable resource.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConfigSchema {
    pub groups: Vec<ConfigGroup>,
}

impl ConfigSchema {
    /// An empty schema; groups are pushed onto `groups`.
    #[must_use]
    pub fn new() -> Self {
        ConfigSchema::default()
    }

    /// Builder form of pushing a group.
    #[must_use]
    pub fn with(mut self, group: ConfigGroup) -> Self {
        self.groups.push(group);
        self
    }

    /// The group by name, if declared.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&ConfigGroup> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// Check a change against this schema — the one place the rules live,
    /// so every producer refuses the same things with the same words.
    ///
    /// In order: the group must exist; a `contract` group is refused at
    /// runtime; a `reach` group needs a confirm window unless the change is
    /// a dry run; every value names a declared parameter, is of its kind,
    /// and is within its bounds. The first problem found is the error.
    ///
    /// # Errors
    ///
    /// [`ConfigError`], whose [`Display`](fmt::Display) is the `message` of
    /// the `reply_err` a producer sends back, and whose
    /// [`reserved_error`](ConfigError::reserved_error) is its `error` name.
    pub fn validate(&self, group: &str, change: &ConfigChange) -> Result<(), ConfigError> {
        let g = self
            .group(group)
            .ok_or_else(|| ConfigError::UnknownGroup(group.to_string()))?;
        match g.class {
            ParamClass::Contract => return Err(ConfigError::Contract(group.to_string())),
            ParamClass::Reach if change.confirm_s.is_none() && !change.dry_run => {
                return Err(ConfigError::ReachNeedsConfirm(group.to_string()));
            }
            ParamClass::Hot | ParamClass::Reach => {}
        }
        for (name, value) in &change.values {
            let spec = g
                .parameter(name)
                .ok_or_else(|| ConfigError::UnknownParameter {
                    group: group.to_string(),
                    name: name.clone(),
                })?;
            let invalid = |reason: String| ConfigError::InvalidValue {
                name: name.clone(),
                reason,
            };
            match (&spec.kind, value) {
                (ParamKind::Bool, ParamValue::Bool(_)) | (ParamKind::Text, ParamValue::Text(_)) => {
                }
                (ParamKind::Integer { min, max, unit }, ParamValue::Integer(v)) => {
                    let unit = unit.as_deref().map_or(String::new(), |u| format!(" {u}"));
                    if let Some(min) = min
                        && v < min
                    {
                        return Err(invalid(format!(
                            "{v}{unit} is below the minimum of {min}{unit}"
                        )));
                    }
                    if let Some(max) = max
                        && v > max
                    {
                        return Err(invalid(format!(
                            "{v}{unit} is above the maximum of {max}{unit}"
                        )));
                    }
                }
                (kind, value) => {
                    return Err(invalid(format!(
                        "expected {}, got {value}",
                        kind_name(kind)
                    )));
                }
            }
        }
        Ok(())
    }
}

fn kind_name(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::Bool => "a boolean",
        ParamKind::Integer { .. } => "an integer",
        ParamKind::Text => "text",
    }
}

/// One parameter's value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ParamValue {
    Bool(bool),
    Integer(i64),
    Text(String),
}

impl fmt::Display for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParamValue::Bool(b) => write!(f, "{b}"),
            ParamValue::Integer(i) => write!(f, "{i}"),
            ParamValue::Text(t) => write!(f, "{t:?}"),
        }
    }
}

impl ParamValue {
    /// Read a value as a person typed it, against the kind the schema
    /// declares: `true`/`false` for a bool, a decimal integer, anything at
    /// all for text.
    ///
    /// The schema decides, which is the point of serving one: `"123"` is
    /// text for a text parameter, and a tool never guesses a kind from the
    /// spelling. Bounds are not checked here — that is
    /// [`ConfigSchema::validate`]'s job, so the refusal is the producer's
    /// wording either way.
    ///
    /// # Errors
    ///
    /// The reason, as a line for the person who typed it.
    pub fn parse_as(kind: &ParamKind, text: &str) -> Result<Self, String> {
        match kind {
            ParamKind::Bool => match text {
                "true" => Ok(ParamValue::Bool(true)),
                "false" => Ok(ParamValue::Bool(false)),
                other => Err(format!("expected true or false, got {other:?}")),
            },
            ParamKind::Integer { .. } => text
                .parse::<i64>()
                .map(ParamValue::Integer)
                .map_err(|_| format!("expected an integer, got {text:?}")),
            ParamKind::Text => Ok(ParamValue::Text(text.to_string())),
        }
    }
}

/// Where a running value came from (RFC 05 §5.1): the producer's default,
/// its startup file, a persisted overlay, or a runtime `set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ValueSource {
    Default,
    File,
    Overlay,
    Runtime,
}

impl ValueSource {
    /// The token as the wire spells it.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            ValueSource::Default => "default",
            ValueSource::File => "file",
            ValueSource::Overlay => "overlay",
            ValueSource::Runtime => "runtime",
        }
    }
}

/// A change request: the body of `config/{resource}/{group}/set`.
///
/// `values` are the group's parameters to change — a subset is a change of
/// those alone. `expected_revision` refuses a change written against a
/// document that has since moved; `idempotency_key` lets a retried request
/// return the first answer instead of applying twice, because a lost reply
/// is otherwise a doubled write. `dry_run` validates and reports without
/// touching the device. `confirm_s` arms a rollback (`reach` requires it).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConfigChange {
    pub values: BTreeMap<String, ParamValue>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub expected_revision: Option<u64>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub idempotency_key: Option<String>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub dry_run: bool,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confirm_s: Option<u64>,
}

impl ConfigChange {
    /// A change of the given values, with no revision guard, key, dry run
    /// or confirm window.
    #[must_use]
    pub fn of(values: impl IntoIterator<Item = (impl Into<String>, ParamValue)>) -> Self {
        ConfigChange {
            values: values.into_iter().map(|(k, v)| (k.into(), v)).collect(),
            ..ConfigChange::default()
        }
    }
}

/// The body of `confirm`, `cancel`, `extend` and `persist` (RFC 05 §5.1):
/// the change's token, and for `extend` the new window counted from now.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ControlRequest {
    pub token: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub confirm_s: Option<u64>,
}

impl ControlRequest {
    /// A request naming a change by its token.
    #[must_use]
    pub fn of(token: impl Into<String>) -> Self {
        ControlRequest {
            token: token.into(),
            confirm_s: None,
        }
    }

    /// The same, with `extend`'s new window.
    #[must_use]
    pub fn extended_by(mut self, confirm_s: u64) -> Self {
        self.confirm_s = Some(confirm_s);
        self
    }
}

/// One parameter as the read-back shows it: its declaration, and what the
/// producer is running with. A sensitive parameter carries no `value`; a
/// parameter the producer cannot read back carries none either — absence
/// is *unknown*, never a guessed value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ParamView {
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub spec: ParamSpec,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub value: Option<ParamValue>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub source: Option<ValueSource>,
    /// The startup file's value, when it differs from the running one.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub startup: Option<ParamValue>,
}

impl ParamView {
    /// A view of a parameter with no value read back.
    #[must_use]
    pub fn of(spec: ParamSpec) -> Self {
        ParamView {
            spec,
            value: None,
            source: None,
            startup: None,
        }
    }
}

/// One group as the read-back shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct GroupView {
    pub name: String,
    pub class: ParamClass,
    pub description: String,
    pub parameters: Vec<ParamView>,
}

/// A change applied and not yet confirmed (RFC 05 §5.1): the token the
/// control procedures take, the deadline the producer will roll back at,
/// spelled as the producer spells time, and the groups it touched.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct PendingChange {
    pub token: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub deadline: Option<String>,
    pub groups: Vec<String>,
}

impl PendingChange {
    /// A pending change by its token, over the groups it touched.
    #[must_use]
    pub fn new(
        token: impl Into<String>,
        groups: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        PendingChange {
            token: token.into(),
            deadline: None,
            groups: groups.into_iter().map(Into::into).collect(),
        }
    }

    /// With the deadline the producer will roll back at, as it spells time.
    #[must_use]
    pub fn until(mut self, deadline: impl Into<String>) -> Self {
        self.deadline = Some(deadline.into());
        self
    }
}

/// The read-back (RFC 05 §5.1): served by `config/{resource}`, echoed as
/// `state/<producer>/config/{resource}` on `transition` QoS, and the reply to
/// every hot `set`. Schema and values travel in one document, so a tool
/// renders a form without knowing the producer, and one `revision` guards
/// the next change.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConfigView {
    pub resource: String,
    pub revision: u64,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub pending: Option<PendingChange>,
    pub groups: Vec<GroupView>,
}

impl ConfigView {
    /// A view of a schema with nothing read back yet, at revision 0.
    #[must_use]
    pub fn of(resource: impl Into<String>, schema: &ConfigSchema) -> Self {
        ConfigView {
            resource: resource.into(),
            revision: 0,
            pending: None,
            groups: schema
                .groups
                .iter()
                .map(|g| GroupView {
                    name: g.name.clone(),
                    class: g.class,
                    description: g.description.clone(),
                    parameters: g.parameters.iter().cloned().map(ParamView::of).collect(),
                })
                .collect(),
        }
    }

    /// The group by name, if the document carries it.
    #[must_use]
    pub fn group(&self, name: &str) -> Option<&GroupView> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// The declaration the document carries, without its values — what a
    /// tool validates a change against before sending it, so the refusal it
    /// prints is the one the producer would have sent (the same
    /// [`ConfigSchema::validate`], the same words).
    #[must_use]
    pub fn schema(&self) -> ConfigSchema {
        ConfigSchema {
            groups: self
                .groups
                .iter()
                .map(|g| ConfigGroup {
                    name: g.name.clone(),
                    class: g.class,
                    description: g.description.clone(),
                    parameters: g.parameters.iter().map(|p| p.spec.clone()).collect(),
                })
                .collect(),
        }
    }
}

/// How a change ended (RFC 05 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum ChangeOutcome {
    /// Applied, and not awaiting confirmation.
    Applied,
    /// A confirmed change made permanent.
    Confirmed,
    /// Undone at its deadline, or cancelled.
    RolledBack,
    /// The undo failed partway: the read-back is the truth, and the producer
    /// never escalates past this on its own.
    Partial,
}

/// One edit in a change event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct Edit {
    pub parameter: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub old: Option<ParamValue>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub new: Option<ParamValue>,
    /// The parameter is sensitive: `old` and `new` are absent by rule, not
    /// by ignorance.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "std::ops::Not::not")
    )]
    pub redacted: bool,
}

/// The change event (RFC 05 §5.1, after RFC 6470's `netconf-config-change`):
/// `events/<producer>/config_change/{event_id}`. Attribution is what the
/// caller *claimed* — `actor` and `request_id` from its selector, the source
/// the transport reported — and none of it authenticates (RFC 06 §5.5 names
/// the actor; the ACL is the authority).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub struct ConfigChangeEvent {
    pub resource: String,
    pub revision: u64,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub token: Option<String>,
    pub outcome: ChangeOutcome,
    pub edits: Vec<Edit>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub actor: Option<String>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub request_id: Option<String>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub claimed_source: Option<String>,
}

/// Why a change was refused (RFC 05 §5.1).
///
/// `#[non_exhaustive]`: matched from outside this crate, and a producer will
/// eventually need a reason nobody has thought of.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// The schema declares no group by this name.
    UnknownGroup(String),
    /// The group declares no parameter by this name.
    UnknownParameter { group: String, name: String },
    /// The value is of the wrong kind, or out of bounds.
    InvalidValue { name: String, reason: String },
    /// The group is `contract`: changed in the producer's startup
    /// configuration, applied by a restart.
    Contract(String),
    /// The group is `reach` and the change carries no `confirm_s`: a change
    /// that can cut the link is accepted only with a rollback armed.
    ReachNeedsConfirm(String),
    /// The change was written against a revision that has since moved.
    StaleRevision { expected: u64, current: u64 },
    /// Another change is pending on this resource, and this one carries
    /// neither its token nor its idempotency key.
    Busy { token: String },
    /// The device refused, in its own words — carried verbatim, because it
    /// is the only place the device says what is actually wrong.
    Device(String),
}

impl ConfigError {
    /// The RFC 05 §3 reserved error name this refusal rides, where one fits.
    /// `None` for [`Contract`](ConfigError::Contract) and
    /// [`Device`](ConfigError::Device), which are the producer's own names
    /// (`error/<producer>/restart-required`, `error/<producer>/device-refused`
    /// in the reference adopter) and are declared as `[[error]]` entries.
    #[must_use]
    pub const fn reserved_error(&self) -> Option<&'static str> {
        match self {
            ConfigError::UnknownGroup(_) | ConfigError::UnknownParameter { .. } => {
                Some("error/not-found")
            }
            ConfigError::InvalidValue { .. }
            | ConfigError::ReachNeedsConfirm(_)
            | ConfigError::StaleRevision { .. } => Some("error/invalid-args"),
            ConfigError::Busy { .. } => Some("error/busy"),
            ConfigError::Contract(_) | ConfigError::Device(_) => None,
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::UnknownGroup(g) => {
                write!(
                    f,
                    "unknown group {g:?}; the schema names every group it accepts"
                )
            }
            ConfigError::UnknownParameter { group, name } => {
                write!(f, "group {group:?} declares no parameter {name:?}")
            }
            ConfigError::InvalidValue { name, reason } => write!(f, "{name}: {reason}"),
            ConfigError::Contract(g) => write!(
                f,
                "group {g:?} is part of what this producer was started against and cannot change \
                 under a running transport; set it in the startup configuration and restart \
                 (RFC 05 §5.1)"
            ),
            ConfigError::ReachNeedsConfirm(g) => write!(
                f,
                "group {g:?} can cut the link this reply would travel on; send it with a \
                 confirm window (`confirm_s`), and confirm over the new link (RFC 05 §5.1)"
            ),
            ConfigError::StaleRevision { expected, current } => write!(
                f,
                "the change expects revision {expected} and the document is at {current}; read \
                 it back and decide again"
            ),
            ConfigError::Busy { token } => write!(
                f,
                "a change is pending on this resource (token {token}); confirm, cancel or extend \
                 it, or carry its token"
            ),
            ConfigError::Device(text) => write!(f, "the device refused: {text}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// A value that must never be read back, logged or serialized in the clear
/// (RFC 05 §5.1's *sensitive*): its `Debug` and `Display` are `<redacted>`,
/// and under the `serde` feature it serializes as the literal string
/// `"<redacted>"` while deserializing as the wrapped type — so a PIN can be
/// received and can never leave.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct Sensitive<T>(T);

impl<T> Sensitive<T> {
    /// Wrap a value.
    pub const fn new(value: T) -> Self {
        Sensitive(value)
    }

    /// The value, for the code that applies it — the one place it is read.
    pub const fn expose(&self) -> &T {
        &self.0
    }

    /// Unwrap, consuming.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl<T> fmt::Display for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(feature = "serde")]
mod serde_impls {
    use super::Sensitive;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    impl<T> Serialize for Sensitive<T> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str("<redacted>")
        }
    }

    impl<'de, T: Deserialize<'de>> Deserialize<'de> for Sensitive<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            T::deserialize(d).map(Sensitive)
        }
    }
}

#[cfg(feature = "schemars")]
impl<T> schemars::JsonSchema for Sensitive<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Sensitive".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "a write-only value: accepted on the way in, `<redacted>` on the way out"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> ConfigSchema {
        ConfigSchema::new()
            .with(
                ConfigGroup::new("queue", ParamClass::Hot, "the kernel queue")
                    .with(ParamSpec::new(
                        "tx_queue_len",
                        ParamKind::Integer {
                            min: Some(1),
                            max: Some(10_000),
                            unit: Some("packets".into()),
                        },
                        "packets queued in front of the modem",
                    ))
                    .with(ParamSpec::new("fq", ParamKind::Bool, "fair queueing")),
            )
            .with(
                ConfigGroup::new("link", ParamClass::Reach, "how the node reaches the bus").with(
                    ParamSpec::new("apn", ParamKind::Text, "the access point name"),
                ),
            )
            .with(
                ConfigGroup::new("sim", ParamClass::Hot, "the subscriber module")
                    .with(ParamSpec::new("pin", ParamKind::Text, "the PIN").sensitive()),
            )
            .with(
                ConfigGroup::new(
                    "transport",
                    ParamClass::Contract,
                    "what Zenoh was started against",
                )
                .with(ParamSpec::new(
                    "mtu",
                    ParamKind::Integer {
                        min: None,
                        max: None,
                        unit: Some("bytes".into()),
                    },
                    "the SDU size",
                )),
            )
    }

    fn change(name: &str, value: ParamValue) -> ConfigChange {
        ConfigChange::of([(name, value)])
    }

    #[test]
    fn a_hot_change_of_the_right_kind_in_range_validates() {
        let s = schema();
        assert_eq!(
            s.validate("queue", &change("tx_queue_len", ParamValue::Integer(100))),
            Ok(())
        );
        assert_eq!(
            s.validate("queue", &change("fq", ParamValue::Bool(true))),
            Ok(())
        );
        assert_eq!(
            s.validate("queue", &ConfigChange::default()),
            Ok(()),
            "an empty change changes nothing"
        );
    }

    #[test]
    fn the_refusals_name_what_the_caller_must_do() {
        let s = schema();
        let err = s.validate("nope", &ConfigChange::default()).unwrap_err();
        assert_eq!(err, ConfigError::UnknownGroup("nope".into()));
        assert_eq!(err.reserved_error(), Some("error/not-found"));

        let err = s
            .validate("queue", &change("txqueue", ParamValue::Integer(1)))
            .unwrap_err();
        assert!(matches!(err, ConfigError::UnknownParameter { .. }));

        let err = s
            .validate("queue", &change("tx_queue_len", ParamValue::Integer(0)))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "tx_queue_len: 0 packets is below the minimum of 1 packets"
        );
        assert_eq!(err.reserved_error(), Some("error/invalid-args"));
        let err = s
            .validate(
                "queue",
                &change("tx_queue_len", ParamValue::Integer(20_000)),
            )
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("above the maximum of 10000 packets")
        );
        let err = s
            .validate("queue", &change("fq", ParamValue::Integer(1)))
            .unwrap_err();
        assert_eq!(err.to_string(), "fq: expected a boolean, got 1");

        let err = s
            .validate("transport", &change("mtu", ParamValue::Integer(220)))
            .unwrap_err();
        assert_eq!(err, ConfigError::Contract("transport".into()));
        assert_eq!(err.reserved_error(), None, "the producer's own name");
        assert!(
            err.to_string()
                .contains("startup configuration and restart")
        );
    }

    #[test]
    fn a_reach_change_needs_a_confirm_window_unless_it_is_a_dry_run() {
        let s = schema();
        let apn = change("apn", ParamValue::Text("iot.example".into()));
        let err = s.validate("link", &apn).unwrap_err();
        assert_eq!(err, ConfigError::ReachNeedsConfirm("link".into()));
        let mut confirmed = apn.clone();
        confirmed.confirm_s = Some(120);
        assert_eq!(s.validate("link", &confirmed), Ok(()));
        let mut dry = apn;
        dry.dry_run = true;
        assert_eq!(s.validate("link", &dry), Ok(()));
    }

    #[test]
    fn a_view_of_a_schema_carries_no_values_and_a_sensitive_parameter_never_will() {
        let view = ConfigView::of("wwan0", &schema());
        assert_eq!(view.revision, 0);
        let pin = &view.group("sim").unwrap().parameters[0];
        assert!(pin.spec.sensitive);
        assert!(pin.value.is_none());
    }

    #[test]
    fn a_sensitive_value_never_prints() {
        let pin = Sensitive::new(String::from("1234"));
        assert_eq!(format!("{pin:?}"), "<redacted>");
        assert_eq!(pin.to_string(), "<redacted>");
        assert_eq!(pin.expose(), "1234");
    }

    #[test]
    fn a_value_is_read_against_the_declared_kind_and_a_view_gives_its_schema_back() {
        let int = ParamKind::Integer {
            min: Some(0),
            max: Some(30),
            unit: Some("dBm".into()),
        };
        assert_eq!(ParamValue::parse_as(&int, "7"), Ok(ParamValue::Integer(7)));
        assert_eq!(
            ParamValue::parse_as(&ParamKind::Text, "7"),
            Ok(ParamValue::Text("7".into()))
        );
        assert_eq!(
            ParamValue::parse_as(&ParamKind::Bool, "yes"),
            Err("expected true or false, got \"yes\"".into())
        );
        assert!(ParamValue::parse_as(&int, "seven").is_err());

        let schema = ConfigSchema::new().with(
            ConfigGroup::new("radio", ParamClass::Reach, "the carrier").with(ParamSpec::new(
                "tx_power",
                int,
                "transmit power",
            )),
        );
        let view = ConfigView::of("rf0", &schema);
        assert_eq!(view.schema(), schema, "a view carries its schema whole");
        let extend = ControlRequest::of("chg-1").extended_by(30);
        assert_eq!(extend.confirm_s, Some(30));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_wire_shapes_round_trip_and_a_sensitive_value_serializes_redacted() {
        let view = ConfigView::of("wwan0", &schema());
        let json = serde_json::to_string(&view).unwrap();
        let back: ConfigView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, view);
        assert!(json.contains("\"class\":\"reach\""));
        assert!(json.contains("\"kind\":\"integer\""));
        let change = ConfigChange::of([("tx_queue_len", ParamValue::Integer(100))]);
        let json = serde_json::to_string(&change).unwrap();
        assert_eq!(
            json, r#"{"values":{"tx_queue_len":100}}"#,
            "absent guards are absent"
        );
        let pin = Sensitive::new(String::from("1234"));
        assert_eq!(serde_json::to_string(&pin).unwrap(), "\"<redacted>\"");
        let back: Sensitive<String> = serde_json::from_str("\"1234\"").unwrap();
        assert_eq!(back.expose(), "1234");
    }
}
