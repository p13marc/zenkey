//! `config.v1` (#768, CC): one service's configuration as values in hand
//! (`spec/profiles/config/v1.md`).
//!
//! **Values in hand → a reading.** [`ConfigReading`] is what §5's questions
//! are put to: the premise ([`premise_in`]: the descriptor lists
//! `config.v1`, or presence shows the service absent, or the read could not
//! tell), and the view and the status ([`DocRead`]), each judged fresh,
//! stale or unobservable by `freshness.v1` at the contract's 60 s from what
//! was observed of it ([`doc_read`]): a window's subscription, on this
//! host's monotonic clock (§2.5), and the owner's reply to an S4 GET, its
//! stamp aged only against a clock trusted to the HLC delta (§2.6). The
//! answers are [`crate::judge::config`]'s.
//!
//! **A change, before it is sent** (§2.5, "A caller that checks first").
//! [`type_values`] reads a command line's `name=value` pairs as the kinds
//! the view declares (or, with no view, as the status's values are typed,
//! or by their spelling), refusing a sensitive value given inline;
//! [`precheck`] runs the owner's own validator, `zenkey_model::config::
//! validate_set`, against the schema the view serves
//! ([`zenkey_model::config::Schema::of_view`]) and the facts it shows, so
//! that the refusal a tool prints is the one the owner would send;
//! [`consent`] says when a change can cut the link it travels on, the
//! class read or, with none, a window taken as reach (§2.4).
//!
//! **A sensitive value is never shown** (§2.15): [`shown_request`] writes
//! it `"<redacted>"`, [`redact_view`] strips a sensitive parameter's value
//! and startup value from a view whatever an owner put there, and
//! [`scrub`] takes the values a tool sent out of any text an owner sends
//! back.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use serde_json::Value as Json;
use zenkey_model::config::{
    self as cm, Admitted, Class, ConfigStatus, ConfigView, Facts, Kind, LastFacts, PendingFacts,
    Refusal, Schema, SetRequest, Value,
};
use zenkey_model::freshness::{
    self as fresh, ClockTrust, Horizon, Judged, Observation, Reply, StampAge,
};
use zenkey_model::grammar::Addr;

use crate::bus::config::Doc;
use crate::bus::health::{Replied, Stamped};
use crate::model::catalog::{Catalog, Observed};
use crate::model::health::Trust;
use crate::report::{
    ConfigDocument, ConfigFreshness, ConfigPremise, ConfigTyping, ConfigVia, FreshnessToken,
};

/// The view's and the status's horizon, their `freshness.ttl_s = 60`
/// (§2.3).
pub fn horizon() -> Horizon {
    Horizon::Within(cm::TTL)
}

/// What `addr`'s descriptors say of `config.v1` (§2.1): listed by any
/// served descriptor; else unread when an instance's descriptor did not
/// read, or descriptors were not asked; else not listed.
pub fn listing(o: &Observed, addr: &Addr) -> ConfigPremise {
    let Some(descriptors) = &o.descriptors else {
        return ConfigPremise::DescriptorUnread;
    };
    let mut unread = false;
    for ((a, _), read) in descriptors {
        if a != addr {
            continue;
        }
        match read.descriptor() {
            Some(d) if d.interfaces.iter().any(|e| e.iface == cm::IFACE) => {
                return ConfigPremise::Listed;
            }
            Some(_) => {}
            None => unread = true,
        }
    }
    if unread {
        ConfigPremise::DescriptorUnread
    } else {
        ConfigPremise::NotListed
    }
}

/// `addr`'s premise in one presence read (core §8.1): its instance token
/// seen, with what its descriptors say; else absent from a complete read,
/// possibly incomplete otherwise.
pub fn premise_in(o: &Observed, addr: &Addr) -> ConfigPremise {
    if Catalog::new(o).has_instance(addr) {
        listing(o, addr)
    } else if o.complete {
        ConfigPremise::Absent
    } else {
        ConfigPremise::Incomplete
    }
}

/// One GET's answer for one document.
#[derive(Debug)]
pub enum Got<'a, T> {
    /// No GET was made.
    NotMade,
    /// The GET was made and drew nothing for the document: silence.
    Silent,
    /// Its reply, judged at `at`, this host's clock as the GET ended.
    Answered {
        reply: &'a Replied<Doc<T>>,
        at: SystemTime,
    },
}

impl<T> Clone for Got<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Got<'_, T> {}

impl<'a, T> Got<'a, T> {
    /// A GET's reply for the document, judged at `at`; silence when none
    /// came.
    pub fn of(reply: Option<&'a Replied<Doc<T>>>, at: SystemTime) -> Self {
        match reply {
            Some(reply) => Got::Answered { reply, at },
            None => Got::Silent,
        }
    }
}

/// One document as a reading holds it: the latest value read (the GET's,
/// else the window's last put), how it came, its stamp, and its freshness
/// over every observation.
#[derive(Debug, Clone, PartialEq)]
pub struct DocRead<T> {
    /// `Ok` a decoded document, `Err` why the bytes are not one; `None` when
    /// nothing was read, or the last reply was a deletion.
    pub value: Option<Doc<T>>,
    pub via: Option<ConfigVia>,
    pub stamp: Option<Stamped>,
    pub fresh: Judged,
}

impl<T> DocRead<T> {
    /// The decoded document, whatever its freshness.
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref().and_then(|v| v.as_ref().ok())
    }

    /// The decoded document when it is fresh: the only one §5 concludes
    /// from.
    pub fn fresh_value(&self) -> Option<&T> {
        (self.fresh.verdict == fresh::Verdict::Fresh)
            .then(|| self.value())
            .flatten()
    }

    /// As the report spells it.
    pub fn report(&self) -> ConfigDocument<T>
    where
        T: Clone,
    {
        ConfigDocument {
            value: self.value().cloned(),
            undecodable: self.value.as_ref().and_then(|v| v.as_ref().err()).cloned(),
            via: self.via,
            stamp: self.stamp.as_ref().map(Stamped::report),
            freshness: freshness(self.fresh),
        }
    }
}

/// A `freshness.v1` verdict as the report spells it.
pub fn freshness(j: Judged) -> ConfigFreshness {
    ConfigFreshness {
        verdict: match j.verdict {
            fresh::Verdict::Fresh => FreshnessToken::Fresh,
            fresh::Verdict::Stale => FreshnessToken::Stale,
            fresh::Verdict::Unobservable => FreshnessToken::Unobservable,
            fresh::Verdict::NotAsked => FreshnessToken::NotAsked,
        },
        reason: j.reason.as_str().to_owned(),
        says: j.reason.says().to_owned(),
    }
}

/// One document from what was read: `heard`, the window's observation of it
/// and the last put it heard, then the GET's reply, which stands over the
/// window's value since it is read last (`freshness.v1` §2.7 combines the
/// two observations).
pub fn doc_read<T: Clone>(
    got: Got<'_, T>,
    heard: Option<(Observation, Option<&Replied<Doc<T>>>)>,
    trust: &Trust<'_>,
) -> DocRead<T> {
    let mut observations = Vec::new();
    let mut out = DocRead {
        value: None,
        via: None,
        stamp: None,
        fresh: fresh::judge_all(&horizon(), &[]),
    };
    if let Some((o, last)) = heard {
        observations.push(o);
        if let Some(r) = last {
            out.value = r.value.clone();
            out.via = Some(ConfigVia::Subscription);
            out.stamp = r.stamp.clone();
        }
    }
    match got {
        Got::NotMade => {}
        Got::Silent => observations.push(Observation::Got {
            reply: None,
            clock: ClockTrust::Untrusted,
        }),
        Got::Answered { reply, at } => {
            observations.push(Observation::Got {
                reply: Some(match &reply.value {
                    None => Reply::Delete,
                    Some(_) => Reply::Put {
                        stamp_age: reply.stamp.as_ref().map(|s| StampAge::between(s.time, at)),
                    },
                }),
                clock: reply
                    .stamp
                    .as_ref()
                    .map_or(ClockTrust::Untrusted, |s| trust.of(&s.clock)),
            });
            out.value = reply.value.clone();
            out.via = Some(ConfigVia::Get);
            out.stamp = reply.stamp.clone();
        }
    }
    out.fresh = fresh::judge_all(&horizon(), &observations);
    out
}

/// One reading of one service, as values: what §5's questions are put to.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigReading {
    pub premise: ConfigPremise,
    /// The view; `None` when only the status was asked.
    pub view: Option<DocRead<ConfigView>>,
    pub status: DocRead<ConfigStatus>,
    /// The change events read: the window's, and a replay's.
    pub events: Vec<cm::ConfigChangeEvent>,
}

/// The fresh documents of a reading, the later-stamped first: §5 answers
/// from "a fresh view or status", and between two that disagree the later
/// put is the owner's latest word (§2.3: the owner puts the view, then the
/// status).
pub enum Fresh<'r> {
    View(&'r ConfigView),
    Status(&'r ConfigStatus),
}

impl Fresh<'_> {
    /// Which document, as a person reads it.
    pub fn name(&self) -> &'static str {
        match self {
            Fresh::View(_) => "view",
            Fresh::Status(_) => "status",
        }
    }

    pub fn instance(&self) -> &str {
        match self {
            Fresh::View(v) => &v.instance,
            Fresh::Status(s) => &s.instance,
        }
    }

    pub fn pending(&self) -> Option<&cm::Pending> {
        match self {
            Fresh::View(v) => v.pending.as_ref(),
            Fresh::Status(s) => s.pending.as_ref(),
        }
    }

    pub fn last_change(&self) -> Option<&cm::LastChange> {
        match self {
            Fresh::View(v) => v.last_change.as_ref(),
            Fresh::Status(s) => s.last_change.as_ref(),
        }
    }

    pub fn desired(&self) -> Option<&cm::DesiredReport> {
        match self {
            Fresh::View(v) => v.desired.as_ref(),
            Fresh::Status(s) => s.desired.as_ref(),
        }
    }
}

impl ConfigReading {
    /// The fresh view, when the view was asked and read fresh.
    pub fn fresh_view(&self) -> Option<&ConfigView> {
        self.view.as_ref().and_then(DocRead::fresh_value)
    }

    /// The fresh documents, the later-stamped first; the view first when
    /// neither stamp orders them.
    pub fn fresh(&self) -> Vec<Fresh<'_>> {
        let view = self
            .view
            .as_ref()
            .and_then(|d| d.fresh_value().map(|v| (d.stamp.as_ref(), Fresh::View(v))));
        let status = self
            .status
            .fresh_value()
            .map(|s| (self.status.stamp.as_ref(), Fresh::Status(s)));
        match (view, status) {
            (Some((vs, v)), Some((ss, s))) => match (vs, ss) {
                (Some(a), Some(b)) if b.time > a.time => vec![s, v],
                _ => vec![v, s],
            },
            (Some((_, v)), None) => vec![v],
            (None, Some((_, s))) => vec![s],
            (None, None) => Vec::new(),
        }
    }
}

/// Whether `status` is `view`'s projection (§2.19): the status the model
/// projects from the view, member for member.
pub fn projection_holds(view: &ConfigView, status: &ConfigStatus) -> bool {
    ConfigStatus::of(view) == *status
}

/// Whether one reading's status is its view's projection: `None` when the
/// two were not both read and decoded.
pub fn projection_of(docs: &crate::bus::config::ServiceDocs) -> Option<bool> {
    let view = docs.view.as_ref()?.value.as_ref()?.as_ref().ok()?;
    let status = docs.status.as_ref()?.value.as_ref()?.as_ref().ok()?;
    Some(projection_holds(view, status))
}

/// A command line's value for one parameter: written on it, or read from a
/// file or standard input (`name=@FILE`, `name=@-`), which is how a
/// sensitive value is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Given {
    Inline(String),
    Read(String),
}

impl Given {
    fn text(&self) -> &str {
        match self {
            Given::Inline(t) | Given::Read(t) => t,
        }
    }
}

/// One `name=value` of a `set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub name: String,
    pub given: Given,
}

/// A `set`'s values, typed.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedValues {
    pub values: BTreeMap<String, Json>,
    pub typed_by: ConfigTyping,
    /// The values given from a file or standard input, by parameter: never
    /// echoed, and taken out of anything the owner sends back.
    pub read: BTreeSet<String>,
}

/// `text` as a value of `kind` where it spells one; otherwise as text, so
/// that the owner's validator refuses it in its own words (§2.5, row 11).
fn as_kind(kind: &Kind, text: &str) -> Json {
    match kind {
        Kind::Bool => match text {
            "true" => Json::Bool(true),
            "false" => Json::Bool(false),
            _ => Json::String(text.to_owned()),
        },
        Kind::Integer { .. } => text
            .parse::<i64>()
            .map_or_else(|_| Json::String(text.to_owned()), Json::from),
        Kind::Text => Json::String(text.to_owned()),
    }
}

/// `text` by its spelling: `true` and `false` a bool, a decimal integer an
/// integer, anything else text.
fn by_spelling(text: &str) -> Json {
    match text {
        "true" => Json::Bool(true),
        "false" => Json::Bool(false),
        _ => text
            .parse::<i64>()
            .map_or_else(|_| Json::String(text.to_owned()), Json::from),
    }
}

/// A `set`'s values typed for `group`: as the kinds the view declares (a
/// parameter it does not declare by its spelling, for the validator to
/// refuse by name); with no view, as the status's values are typed; with
/// neither, by their spelling.
///
/// # Errors
///
/// A parameter given twice, or a sensitive one given on the command line:
/// a sensitive value is given from a file or standard input, so that it
/// never sits in a shell's history or a process listing (§2.15).
pub fn type_values(
    view: Option<&ConfigView>,
    status: Option<&ConfigStatus>,
    group: &str,
    given: &[Assignment],
) -> Result<TypedValues, String> {
    let declared = view.and_then(|v| v.groups.iter().find(|g| g.name == group));
    let shown = status.and_then(|s| s.values.get(group));
    let typed_by = if view.is_some() {
        ConfigTyping::Schema
    } else if status.is_some() {
        ConfigTyping::Status
    } else {
        ConfigTyping::Spelling
    };
    let mut out = TypedValues {
        values: BTreeMap::new(),
        typed_by,
        read: BTreeSet::new(),
    };
    for a in given {
        if out.values.contains_key(&a.name) {
            return Err(format!("{} is given twice", a.name));
        }
        let param = declared.and_then(|g| g.parameters.iter().find(|p| p.name == a.name));
        if let (Some(p), Given::Inline(_)) = (param, &a.given)
            && p.sensitive
        {
            return Err(format!(
                "{group}.{} is sensitive, write-only (config.v1 §2.15): give it from a file or \
                 standard input, {}=@FILE or {}=@-, so it never sits on a command line",
                a.name, a.name, a.name
            ));
        }
        let text = a.given.text();
        let value = match (view, param, shown.and_then(|s| s.get(&a.name))) {
            (Some(_), Some(p), _) => as_kind(&p.kind, text),
            (Some(_), None, _) => by_spelling(text),
            (None, _, Some(Value::Bool(_))) => as_kind(&Kind::Bool, text),
            (None, _, Some(Value::Integer(_))) => as_kind(
                &Kind::Integer {
                    min: None,
                    max: None,
                    unit: None,
                },
                text,
            ),
            (None, _, Some(Value::Text(_))) => Json::String(text.to_owned()),
            (None, _, None) => by_spelling(text),
        };
        if matches!(a.given, Given::Read(_)) {
            out.read.insert(a.name.clone());
        }
        out.values.insert(a.name.clone(), value);
    }
    Ok(out)
}

/// The request as a report shows it (§2.15): a value whose parameter the
/// view declares sensitive, or one read from a file or standard input that
/// no view says is not, written `"<redacted>"`.
pub fn shown_request(
    req: &SetRequest,
    view: Option<&ConfigView>,
    group: &str,
    read: &BTreeSet<String>,
) -> Json {
    let declared = view.and_then(|v| v.groups.iter().find(|g| g.name == group));
    let mut out = serde_json::to_value(req).unwrap_or(Json::Null);
    if let Some(values) = out.get_mut("values").and_then(Json::as_object_mut) {
        for (name, v) in values.iter_mut() {
            let param = declared.and_then(|g| g.parameters.iter().find(|p| &p.name == name));
            let hide = match param {
                Some(p) => p.sensitive,
                None => read.contains(name),
            };
            if hide {
                *v = Json::String(cm::REDACTED.to_owned());
            }
        }
    }
    out
}

/// The facts a view shows a validator (§2.5): its revision, its pending
/// change and its last change. An act in flight, a deadline passed and a
/// persisted layer are the owner's to know, and are taken as not.
fn facts(view: &ConfigView) -> Facts<'_> {
    Facts {
        revision: view.revision,
        pending: view.pending.as_ref().map(|p| PendingFacts {
            token: &p.token,
            applied: p.applied,
            deadline_passed: false,
        }),
        last_change: view.last_change.as_ref().map(|l| LastFacts {
            token: &l.token,
            set_sensitive: false,
        }),
        persist: true,
        in_flight: false,
    }
}

/// The owner's own checks of a `set`, run against a view before it is sent
/// (§2.5, "A caller that checks first"): the request's type, then rows 3 to
/// 11, against the schema the view serves and the facts it shows. The
/// owner's word is still the reply.
///
/// # Errors
///
/// The refusal the owner would send.
pub fn precheck(view: &ConfigView, group: &str, req: &SetRequest) -> Result<Admitted, Refusal> {
    cm::validate_set(&Schema::of_view(view), &facts(view), group, req)
}

/// Why a `set` needs its operator's consent, in a sentence, or `None` (§2.4):
/// a reach group can cut the link the reply travels on; and a group whose
/// class could not be read, changed with a window or joining a change that
/// has one, is taken as reach — asking once too often costs a keystroke,
/// and not asking can cost the link. A dry run touches nothing, and needs
/// none. `no_class` says why the class could not be read.
pub fn consent(
    view: Option<&ConfigView>,
    address: &str,
    group: &str,
    req: &SetRequest,
    no_class: &str,
) -> Option<String> {
    if req.dry_run {
        return None;
    }
    let class = view
        .and_then(|v| v.groups.iter().find(|g| g.name == group))
        .map(|g| g.class);
    match class {
        Some(Class::Reach) => Some(format!(
            "group {group:?} is reach: applying it can cut the link to {address} that this \
             change's reply travels on (config.v1 §2.4)"
        )),
        Some(_) => None,
        None if req.confirm_s.is_some() || req.token.is_some() => Some(format!(
            "the class of group {group:?} could not be read ({no_class}), and a change with a \
             window, or joining one, is how a reach group is changed: it may cut the link to \
             {address} (config.v1 §2.4)"
        )),
        None => None,
    }
}

/// The parameters a view declares sensitive, by group and name.
pub fn sensitive_params(view: Option<&ConfigView>) -> BTreeSet<(String, String)> {
    view.into_iter()
        .flat_map(|v| v.groups.iter())
        .flat_map(|g| {
            g.parameters
                .iter()
                .filter(|p| p.sensitive)
                .map(|p| (g.name.clone(), p.name.clone()))
        })
        .collect()
}

/// Strips a sensitive parameter's value and startup value from `view`:
/// one the view itself declares sensitive, or one in `known` (§2.15). An
/// owner MUST NOT carry them; a tool never shows one either way.
pub fn redact_view(view: &mut ConfigView, known: &BTreeSet<(String, String)>) {
    for g in &mut view.groups {
        for p in &mut g.parameters {
            if p.sensitive || known.contains(&(g.name.clone(), p.name.clone())) {
                p.value = None;
                p.startup = None;
            }
        }
    }
}

/// Strips a sensitive parameter's value from a status (§2.15).
pub fn redact_status(status: &mut ConfigStatus, known: &BTreeSet<(String, String)>) {
    for (g, p) in known {
        if let Some(values) = status.values.get_mut(g) {
            values.remove(p);
        }
    }
    status.values.retain(|_, v| !v.is_empty());
}

/// `text` with every value in `secrets` written `"<redacted>"`: what an
/// owner sends back never carries a sensitive value a tool sent (§2.15). A
/// value shorter than 3 characters is left alone, since replacing it would
/// mangle the words around it and hide nothing.
pub fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_owned();
    for s in secrets.iter().filter(|s| s.chars().count() >= 3) {
        out = out.replace(s.as_str(), cm::REDACTED);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use zenkey_model::freshness::Last;

    fn view(pending: bool) -> ConfigView {
        let mut v = json!({
            "service": "lab/rf0", "instance": "i1", "revision": 1, "confirm_floor_s": 10,
            "last_change": {"token": "i1-1", "groups": ["duty"]},
            "groups": [
                {"name": "duty", "class": "hot", "description": "d", "writable": true,
                 "parameters": [{"name": "fraction_ppm", "kind": "integer", "min": 0,
                                 "max": 1000000, "description": "p", "value": 20000,
                                 "source": "runtime", "startup": 10000}]},
                {"name": "radio", "class": "reach", "description": "d", "writable": true,
                 "parameters": [{"name": "frequency_hz", "kind": "integer",
                                 "description": "p", "value": 868100000, "source": "file"}]},
                {"name": "sim", "class": "hot", "description": "d", "writable": true,
                 "parameters": [{"name": "pin", "kind": "text", "description": "p",
                                 "sensitive": true, "source": "file"}]},
                {"name": "queue", "class": "hot", "description": "d", "writable": false,
                 "parameters": [{"name": "on", "kind": "bool", "description": "p",
                                 "value": true, "source": "default"}]}
            ]
        });
        if pending {
            v["pending"] = json!({"token": "i1-2", "groups": ["radio"],
                                  "deadline": "2026-10-18T00:02:00.000Z", "applied": true});
        }
        serde_json::from_value(v).unwrap()
    }

    fn inline(name: &str, v: &str) -> Assignment {
        Assignment {
            name: name.into(),
            given: Given::Inline(v.into()),
        }
    }

    /// Values typed by the schema, the status and the spelling, in that
    /// order of preference; a value its kind cannot spell stays text for
    /// the owner's validator to refuse by name; a sensitive value given
    /// inline is refused before anything is read further.
    #[test]
    fn a_value_is_typed_by_what_was_read() {
        let v = view(false);
        let t = type_values(
            Some(&v),
            None,
            "duty",
            &[inline("fraction_ppm", "20000"), inline("other", "true")],
        )
        .unwrap();
        assert_eq!(t.typed_by, ConfigTyping::Schema);
        assert_eq!(t.values["fraction_ppm"], json!(20000));
        assert_eq!(
            t.values["other"],
            json!(true),
            "undeclared: by its spelling"
        );
        let t = type_values(Some(&v), None, "duty", &[inline("fraction_ppm", "lots")]).unwrap();
        assert_eq!(t.values["fraction_ppm"], json!("lots"));
        let req = SetRequest {
            values: t.values,
            ..SetRequest::default()
        };
        let refused = precheck(&v, "duty", &req).unwrap_err();
        assert_eq!(refused.code, cm::Code::InvalidRequest);
        assert!(refused.message.contains("fraction_ppm"), "{refused}");
        let err = type_values(Some(&v), None, "sim", &[inline("pin", "1234")]).unwrap_err();
        assert!(err.contains("pin=@FILE") && !err.contains("1234"), "{err}");
        let t = type_values(
            Some(&v),
            None,
            "sim",
            &[Assignment {
                name: "pin".into(),
                given: Given::Read("1234".into()),
            }],
        )
        .unwrap();
        assert_eq!(t.read, BTreeSet::from(["pin".to_owned()]));
        let s = ConfigStatus::of(&v);
        let t = type_values(None, Some(&s), "duty", &[inline("fraction_ppm", "7")]).unwrap();
        assert_eq!(
            (t.typed_by, &t.values["fraction_ppm"]),
            (ConfigTyping::Status, &json!(7))
        );
        let t = type_values(None, None, "x", &[inline("a", "7"), inline("b", "eu")]).unwrap();
        assert_eq!(t.typed_by, ConfigTyping::Spelling);
        assert_eq!((&t.values["a"], &t.values["b"]), (&json!(7), &json!("eu")));
        assert!(type_values(None, None, "x", &[inline("a", "1"), inline("a", "2")]).is_err());
    }

    /// The request shown never carries a sensitive value, nor one read from
    /// a file that no view says is not; `scrub` takes a sent value out of
    /// an owner's words.
    #[test]
    fn a_sensitive_value_is_never_shown() {
        let v = view(false);
        let req = SetRequest {
            values: [("pin".to_owned(), json!("1234"))].into(),
            ..SetRequest::default()
        };
        let shown = shown_request(&req, Some(&v), "sim", &BTreeSet::new());
        assert_eq!(shown["values"]["pin"], "<redacted>");
        let shown = shown_request(&req, None, "sim", &BTreeSet::from(["pin".to_owned()]));
        assert_eq!(shown["values"]["pin"], "<redacted>");
        let shown = shown_request(&req, None, "sim", &BTreeSet::new());
        assert_eq!(
            shown["values"]["pin"], "1234",
            "inline, no schema: as typed"
        );
        assert_eq!(
            scrub("pin 1234 refused", &["1234".into()]),
            "pin <redacted> refused"
        );
        let mut leaky = view(false);
        leaky.groups[2].parameters[0].value = Some(Value::Text("1234".into()));
        redact_view(&mut leaky, &BTreeSet::new());
        assert_eq!(leaky.groups[2].parameters[0].value, None);
        let mut s = ConfigStatus::of(&view(false));
        s.values.insert(
            "sim".into(),
            [("pin".into(), Value::Text("1234".into()))].into(),
        );
        redact_status(&mut s, &sensitive_params(Some(&view(false))));
        assert!(!s.values.contains_key("sim"));
    }

    /// The precheck is the owner's validator over the view's facts: a
    /// pending change makes another writer's `set` busy, a contract or an
    /// unwritable group is refused, and a reach `set` needs its window.
    #[test]
    fn the_precheck_refuses_as_the_owner_would() {
        let busy = precheck(
            &view(true),
            "duty",
            &SetRequest {
                values: [("fraction_ppm".to_owned(), json!(1))].into(),
                ..SetRequest::default()
            },
        )
        .unwrap_err();
        assert_eq!(busy.code, cm::Code::Busy);
        assert!(busy.message.contains("i1-2"));
        let queue = precheck(&view(false), "queue", &SetRequest::default()).unwrap_err();
        assert_eq!(
            (queue.code, queue.cause()),
            (cm::Code::Unavailable, Some("config"))
        );
        let reach = precheck(&view(false), "radio", &SetRequest::default()).unwrap_err();
        assert_eq!(reach.code, cm::Code::InvalidRequest);
        assert!(
            precheck(
                &view(false),
                "radio",
                &SetRequest {
                    confirm_s: Some(60),
                    ..SetRequest::default()
                }
            )
            .is_ok()
        );
    }

    /// Consent: a reach group asks it, a hot group does not, a dry run never
    /// does, and with no class read a window is taken as reach (§2.4).
    #[test]
    fn a_change_that_can_cut_the_link_asks_first() {
        let v = view(false);
        let windowed = SetRequest {
            confirm_s: Some(60),
            ..SetRequest::default()
        };
        assert!(consent(Some(&v), "lab/rf0", "radio", &windowed, "").is_some());
        assert!(consent(Some(&v), "lab/rf0", "duty", &windowed, "").is_none());
        let dry = SetRequest {
            dry_run: true,
            ..windowed.clone()
        };
        assert!(consent(Some(&v), "lab/rf0", "radio", &dry, "").is_none());
        let unknown = consent(None, "lab/rf0", "duty", &windowed, "--no-precheck").unwrap();
        assert!(unknown.contains("--no-precheck"), "{unknown}");
        assert!(consent(None, "lab/rf0", "duty", &SetRequest::default(), "").is_none());
    }

    fn stamped(age: u64) -> Option<Stamped> {
        Some(Stamped {
            time: SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 - age),
            clock: "ab12".into(),
        })
    }

    fn at() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
    }

    /// A document's freshness from the GET alone needs a trusted clock; a
    /// window's put confirms it on this host's clock; the GET's value
    /// stands over the window's; a deletion holds no value.
    #[test]
    fn a_documents_freshness_comes_from_what_observed_it() {
        let reply = Replied {
            value: Some(Ok(view(false))),
            stamp: stamped(10),
        };
        let got = Got::Answered {
            reply: &reply,
            at: at(),
        };
        let untrusted = doc_read(got, None, &Trust::None);
        assert_eq!(untrusted.fresh.verdict, fresh::Verdict::Unobservable);
        assert!(untrusted.value().is_some() && untrusted.fresh_value().is_none());
        let word = doc_read(got, None, &Trust::Word);
        assert_eq!(word.fresh.verdict, fresh::Verdict::Fresh);
        assert_eq!(word.via, Some(ConfigVia::Get));
        let stale = Replied {
            value: Some(Ok(view(false))),
            stamp: stamped(120),
        };
        let r = doc_read(
            Got::Answered {
                reply: &stale,
                at: at(),
            },
            None,
            &Trust::Word,
        );
        assert_eq!(r.fresh.verdict, fresh::Verdict::Stale);
        let heard = Replied {
            value: Some(Ok(view(true))),
            stamp: stamped(5),
        };
        let window = doc_read(
            Got::NotMade,
            Some((
                Observation::Subscribed {
                    last: Some(Last::Put(Duration::from_secs(5))),
                    listened: Duration::from_secs(31),
                    complete: true,
                },
                Some(&heard),
            )),
            &Trust::None,
        );
        assert_eq!(window.fresh.verdict, fresh::Verdict::Fresh);
        assert_eq!(window.via, Some(ConfigVia::Subscription));
        assert!(window.fresh_value().unwrap().pending.is_some());
        let gone = Replied::<Doc<ConfigView>> {
            value: None,
            stamp: stamped(1),
        };
        let r = doc_read(
            Got::Answered {
                reply: &gone,
                at: at(),
            },
            None,
            &Trust::Word,
        );
        assert_eq!(
            (r.value(), r.fresh.verdict),
            (None, fresh::Verdict::NotAsked)
        );
        let silent = doc_read::<ConfigView>(Got::Silent, None, &Trust::Word);
        assert_eq!(silent.fresh.reason, fresh::Reason::Silent);
    }

    /// The projection is the model's, member for member.
    #[test]
    fn the_projection_is_the_models() {
        let v = view(true);
        let mut s = ConfigStatus::of(&v);
        assert!(projection_holds(&v, &s));
        s.revision += 1;
        assert!(!projection_holds(&v, &s));
    }
}
