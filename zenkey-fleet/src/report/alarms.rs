//! `alarms.v1` (#800, AC): which alarms fire, as a tool reads them
//! (`spec/profiles/alarms/v1.md`).
//!
//! **The profile's own vocabulary, then the judgement shape.** §5's four
//! questions each answer in the tokens the profile's fixtures state
//! (`conclusions.json`: `yes`, `no`, `unobservable`, `not_asked`, each with
//! a reason class), so a script reads `firing` where the text says firing:
//! - "is this alarm firing?" — [`AlarmRow::firing`] (§2.14), yes the
//!   finding;
//! - "is anything firing at or above *S* under this selector?" —
//!   [`AlarmsReport::any_firing`] (§2.15), over the owners in scope, each
//!   counted by its verdict ([`AlarmOwnerRow`]);
//! - "does this key re-derive from its document?" — [`AlarmRow::rederives`]
//!   (§2.2, §2.3), no the finding and `sanitized` a SHOULD broken;
//! - "is this firing alarm past its horizon, its owner present?" —
//!   [`AlarmRow::stale`] (§2.7), yes the finding, about the owner.
//!
//! [`AlarmsReport::judgement`] is the second question's answer as the one
//! [`Judgement`] a frontend exits through.
//!
//! **What was read is shown as read.** An alarm's latest document is the
//! payload's, with its stamp and how it reached this reader
//! ([`AlarmSeen`]); an archive's document for an absent owner is
//! [`LastKnownAlarm`]: last-known, never current, never part of a verdict
//! (core S6).
//!
//! **`alarms.v2`** (#984's S3, `spec/profiles/alarms/v2.md`): a row of an
//! owner whose descriptor lists `alarms.v2` says so ([`AlarmRow::profile`]),
//! names its slot ([`AlarmRow::slot`]) and its words ([`AlarmWords`]: a
//! table's, a slot's `words/<slot>`, its alert key alone, or its numbers
//! for an unnamed alarm, whose [`AlarmRow::alarm`] is then absent: it
//! matches no reference, §2.14). Its record is shown as the profile's JSON
//! reading ([`AlarmSeen::record`]), its `at` to the second, an absent one
//! absent. "Do this slot's words derive its record's key?" is
//! [`AlarmRow::rederives`]'s place under v2 (§5's third question), and "does
//! this owner hold each identity on one slot?" [`AlarmOwnerRow::one_slot_each`].
//! Every member v2 adds is omitted on an `alarms.v1` row, so v1's shape
//! stands.
//!
//! **A reader's transitions** ([`AlarmTransition`]): one key's change, as
//! a notifier routes it — §2.6's transition of a sample, a loss, or the
//! verdict moving with no sample (stale, its owner gone) — each carrying
//! the first question's answer after it, its three states kept apart. A
//! loss makes every key unobservable, never ok. Computed by
//! [`crate::model::alarms`], pinned here.

use std::collections::BTreeMap;

use serde::Serialize;
use zenkey_model::alarms::{Alarm, AlarmRef, Severity};

use super::health::ClockGround;
use super::judgement::Judgement;
use super::state::Stamp;

/// A question of §5, answered in the three states: the fixtures' answer
/// tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmAnswerToken {
    Yes,
    No,
    Unobservable,
    NotAsked,
}

impl AlarmAnswerToken {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmAnswerToken::Yes => "yes",
            AlarmAnswerToken::No => "no",
            AlarmAnswerToken::Unobservable => "unobservable",
            AlarmAnswerToken::NotAsked => "not_asked",
        }
    }
}

/// One of §5's questions answered for one key or one scope: the answer,
/// its reason class (the fixtures' token), and the reason in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmAnswer {
    pub answer: AlarmAnswerToken,
    pub reason: String,
    pub says: String,
}

/// How a sample reached this reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmVia {
    /// The reader's subscription (core §4.2).
    Subscription,
    /// The owner's reply to S4's GET.
    Get,
}

/// What the latest sample of a key was, as §2.14 step 5 reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmLatest {
    Firing,
    Resolved,
    /// A delete, or a `reply_del`: read as resolved by a reader that held
    /// it firing (§2.6).
    Deleted,
    /// A put that does not read (§2.4).
    Undecodable,
}

/// The latest sample of one key by the owner's in-band stamp (core §4.3),
/// as read (§2.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmSeen {
    pub latest: AlarmLatest,
    /// The document, when the put reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<Alarm>,
    /// Why the put does not read (§2.4): `json`, `schema` or `time_range`,
    /// then the words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    pub via: AlarmVia,
    /// `alarms.v2`: the record in the profile's JSON reading (§3), where it
    /// reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<serde_json::Value>,
    /// `alarms.v2`: the record's `at`, whole seconds since the Unix epoch
    /// (§2.4); absent where the record carries none, never the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_s: Option<i64>,
}

/// An archive's document for an alarm of an absent owner (core §4.4, S6):
/// last-known, never current, and never part of a verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LastKnownAlarm {
    /// The archive, `<system>/<service>`.
    pub archive: String,
    /// The archived document; absent for a tombstone, or a value that does
    /// not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<Alarm>,
    /// The archived sample was a tombstone (`reply_del`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    /// Whether the archive's alignment confirmed it (core §4.4).
    pub confirmed: bool,
    /// `alarms.v2`: the archived record, in the profile's JSON reading
    /// (§3), where it reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<serde_json::Value>,
}

/// One alarm key, as read, with §5's three per-key questions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmRow {
    /// The key, base-relative in the namespace.
    pub key: String,
    /// Its owner's system and service, and its alert key (§2.12): its
    /// reference. Absent for an unnamed `alarms.v2` alarm, which matches no
    /// reference (`alarms.v2` §2.14).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm: Option<AlarmRef>,
    /// `alarms.v2` for a row read by that text (§2.1); absent for an
    /// `alarms.v1` row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<&'static str>,
    /// `alarms.v2`: the slot the alarm is on (§2.2), where it is now, never
    /// its name. Absent for an alarm no slot holds any more: `rebound`,
    /// `replaced` or `dropped` ([`AlarmRow::displaced`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u8>,
    /// `alarms.v2`: how the alarm is named (§5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<AlarmWords>,
    /// `alarms.v2`: what became of the alarm when another identity's record
    /// took its slot (§2.6): `rebound`, `replaced` or `dropped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displaced: Option<&'static str>,
    /// "Is this alarm firing?" (§2.14).
    pub firing: AlarmAnswer,
    /// "Is this firing alarm past its horizon, its owner present?" (§2.7,
    /// §2.14).
    pub stale: AlarmAnswer,
    /// "Does this key re-derive from its document?" (§2.2, §2.3); under
    /// `alarms.v2`, "do this slot's words derive its record's key?" (v2
    /// §2.4, §5).
    pub rederives: AlarmAnswer,
    /// The latest document's severity, when one reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<AlarmSeen>,
    /// The latest document's problems (§2.4): an owner's errors, then its
    /// warnings, as the fixtures' tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// The latest put's stamp is not its owner's (core §4.2's foreign
    /// stamp, a finding apart from the verdict, §2.14 step 5): its id is
    /// none of the present descriptors' `meta.zid`, by value.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub foreign_stamp: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_known: Option<LastKnownAlarm>,
}

/// How an `alarms.v2` alarm is named (#984's S3; `alarms.v2` §2.4, §4,
/// §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmNamedBy {
    /// A rule of its owner's table, read from the revision its descriptor
    /// names (or the deployment's word for it).
    Table,
    /// Its slot's words, taken because they derive the record's key.
    Words,
    /// By words, its words not taken: shown by its alert key alone.
    Key,
    /// Unnamed: a table's rule the reader cannot name, shown by its
    /// numbers, as in `rule 1, labels 2`.
    Numbers,
}

impl AlarmNamedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmNamedBy::Table => "table",
            AlarmNamedBy::Words => "words",
            AlarmNamedBy::Key => "key",
            AlarmNamedBy::Numbers => "numbers",
        }
    }
}

/// An `alarms.v2` alarm's words as a reader has them (§5): the rule's name,
/// its labels, its summary and its category, or its numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmWords {
    pub by: AlarmNamedBy,
    /// The rule's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// The rule's index in its owner's table: a table's rule, named or not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u8>,
    /// Each label's name and its text; host-scoped ones too, for an alarm
    /// by words.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// The record's `labels`, as one integer: an unnamed alarm's numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The argument's reading, in exact decimal (§4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
    /// Why it is shown by its numbers (`no_table`, `rule`, `labels`), or by
    /// its alert key alone (`no_words`, `unreadable`, `mismatch`; §2.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// How a reader knew of an owner in scope (§2.15, F-131).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownBy {
    /// This reading's presence read found it, by instance token and
    /// descriptor (§2.13).
    Presence,
    /// An earlier reading of this reader's own: a presence read, a token,
    /// or its alarms heard.
    Earlier,
    /// An archive's keys name it (core S6).
    Archive,
    /// The deployment's word, across a constrained face where presence
    /// does not cross (§2.11).
    Word,
}

/// Whether an owner's alarms are known whole (§2.15): the fixtures' set
/// verdict tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmSetVerdict {
    Established,
    Unobservable,
    NotAsked,
}

impl AlarmSetVerdict {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmSetVerdict::Established => "established",
            AlarmSetVerdict::Unobservable => "unobservable",
            AlarmSetVerdict::NotAsked => "not_asked",
        }
    }
}

/// One owner in scope (§2.15): how it was known, and whether its alarms
/// are known whole.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlarmOwnerRow {
    /// `<system>/<service>`.
    pub address: String,
    /// How the reader knew of it, every way that applies.
    pub known_by: Vec<KnownBy>,
    pub verdict: AlarmSetVerdict,
    pub reason: String,
    pub says: String,
    /// Its alarm keys read.
    pub alarms: usize,
    /// Its subscription window's span as §2.15 counts it, seconds: from its
    /// start (the declaration plus the reply timeout, or the first complete
    /// GET after a loss or a gap in presence) to one lease and the reply
    /// timeout before now. Absent when no subscription listened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    /// `alarms.v2` for an owner read by that text (§2.1); absent for an
    /// `alarms.v1` owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<&'static str>,
    /// `alarms.v2`: "does this owner hold each identity on one slot?"
    /// (§2.2, §5), no the finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_slot_each: Option<AlarmAnswer>,
}

/// "Is anything firing at or above *S* under this selector?" (§2.15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmScopeAnswer {
    /// *S*.
    pub at_least: Severity,
    pub answer: AlarmAnswerToken,
    pub reason: String,
    pub says: String,
    /// The alarms firing at or above *S*.
    pub firing: usize,
    /// The owners in scope, by their verdict.
    pub established: usize,
    pub unobservable: usize,
    pub not_asked: usize,
}

/// Where the owners were looked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmsPresence {
    /// The liveliness selector, base-relative in the namespace.
    pub selector: String,
    /// Whether the read ended at the routers' final reply (core §8.1).
    pub complete: bool,
    /// The read's session was cut off while it ran (core §8.1, 0.38): zenoh
    /// answered it at the cut, so `complete` is false. Absent otherwise.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_off_during: bool,
    /// Services holding an instance token in it, in scope.
    pub services: usize,
    /// Of which, a descriptor lists `alarms.v1` (§2.13).
    pub listing: usize,
}

/// The deployment's word for an owner across a constrained face (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AlarmsFace {
    /// Whether the face lets this reader GET the alarms (`link.v1` §2.9).
    pub reachable: bool,
}

/// `zenctl alarms` (#800, AC): which alarms fire under a selector, read as
/// the profile's reader reads them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlarmsReport {
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The selector asked, base-relative.
    pub selector: String,
    /// Presence as read; absent across a face, where it does not cross.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<AlarmsPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<AlarmsFace>,
    /// How long the subscription listened, seconds. Absent when none did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    pub clock: ClockGround,
    /// The selectors put to the bus, base-relative in the namespace: what
    /// was read, and no wider (tooling guide O5).
    pub asked: Vec<String>,
    /// §2.15's question at the floor.
    pub any_firing: AlarmScopeAnswer,
    /// The owners in scope, each counted by its verdict.
    pub owners: Vec<AlarmOwnerRow>,
    pub alarms: Vec<AlarmRow>,
    /// Deliveries this reader lost, and how often its receive path stalled
    /// (core §2.7): each makes every key unobservable until a GET answers
    /// it.
    #[serde(default, skip_serializing_if = "super::asked::u64_is_zero")]
    pub lost: u64,
    /// Set when something in scope could not be read — a presence read or a
    /// GET that could not be made, or an archive whose last-known could know
    /// an absent owner — with why. The run is then no verdict unless an
    /// alarm fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl AlarmsReport {
    /// The run as one [`Judgement`]: §2.15's answer at the floor. An alarm
    /// firing at or above it is the finding (exit 1), whatever else is
    /// known; *no* (exit 0) only when every owner asked is established and
    /// none of its alarms that may be at or above it is unobservable;
    /// unobservable and not asked are no verdict (exit 2), with why.
    pub fn judgement(&self) -> Judgement {
        let a = &self.any_firing;
        match a.answer {
            AlarmAnswerToken::Yes => Judgement::Established,
            _ if self.unobservable.is_some() => Judgement::Unobservable {
                reason: self.unobservable.clone().unwrap_or_default(),
            },
            AlarmAnswerToken::No => Judgement::NotEstablished {
                reason: format!("{} ({})", a.says, a.reason),
            },
            AlarmAnswerToken::Unobservable => Judgement::Unobservable {
                reason: format!("{} ({})", a.says, a.reason),
            },
            AlarmAnswerToken::NotAsked => Judgement::NotAsked,
        }
    }
}

/// What a key's change was (§2.6), or the reader's own: a loss, or its
/// verdict moving with no sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmChange {
    Raised,
    Escalated,
    Refreshed,
    Confirmed,
    Resolved,
    Retired,
    /// A put that does not read: the alarm's state is not known.
    Unreadable,
    /// The reader lost deliveries, or its receive path stalled: the key is
    /// unobservable until a GET answers it (§2.14 step 4).
    Lost,
    /// The first question's answer moved with no sample of the key: it went
    /// stale, its owner went or came back, its owner's answer dropped it.
    Judged,
}

impl AlarmChange {
    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            AlarmChange::Raised => "raised",
            AlarmChange::Escalated => "escalated",
            AlarmChange::Refreshed => "refreshed",
            AlarmChange::Confirmed => "confirmed",
            AlarmChange::Resolved => "resolved",
            AlarmChange::Retired => "retired",
            AlarmChange::Unreadable => "unreadable",
            AlarmChange::Lost => "lost",
            AlarmChange::Judged => "judged",
        }
    }
}

/// What a reader lost (core §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AlarmLoss {
    /// Deliveries its queue refused.
    pub dropped: u64,
    /// Its receive path stalled past the bound, or a transport of its
    /// session closed: what a router held for it went (0.27). Or its watch
    /// session's transport moved, and its instance tokens' subscriber may
    /// not have heard what moved meanwhile (core §8.1, 0.42, F-229).
    pub stalled: bool,
}

/// One key's change, as a reader saw it: what a notifier routes (#800,
/// zenwatch's `alerts`). `firing` is the first question's answer after it,
/// in its three states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlarmTransition {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alarm: Option<AlarmRef>,
    pub change: AlarmChange,
    /// "Is this alarm firing?" after the change (§2.14).
    pub firing: AlarmAnswer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    /// The document the sample carried, when it reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<Alarm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<AlarmVia>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Option<AlarmLoss>,
    /// `alarms.v2` for an alarm read by that text; absent for `alarms.v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<&'static str>,
    /// `alarms.v2`: the slot (§2.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u8>,
    /// `alarms.v2`: how the alarm is named (§5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<AlarmWords>,
    /// `alarms.v2`: the record, in the profile's JSON reading (§3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<serde_json::Value>,
    /// `alarms.v2`: the held alarm another identity's record displaced
    /// from its slot (§2.6): `rebound`, a resolve to report (§2.16), or
    /// `replaced` or `dropped`, nothing to report by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displaced: Option<&'static str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;
    use std::collections::BTreeMap;
    use zenkey_model::alarms::State;

    fn answer(answer: AlarmAnswerToken, reason: &str) -> AlarmAnswer {
        AlarmAnswer {
            answer,
            reason: reason.into(),
            says: "s".into(),
        }
    }

    fn doc() -> Alarm {
        Alarm {
            timestamp: 1_760_950_000_000,
            observed_at_ms: None,
            rule: "link_down".into(),
            severity: Severity::Warning,
            state: State::Firing,
            summary: "link to r2 down on eth0".into(),
            category: None,
            labels: BTreeMap::from([("peer".into(), "r2".into()), ("port".into(), "eth0".into())]),
        }
    }

    fn a_ref() -> AlarmRef {
        AlarmRef {
            system: "lab".into(),
            service: "svc".into(),
            alert_key: "a659f813308ad1da".into(),
        }
    }

    fn scope(a: AlarmAnswerToken, reason: &str) -> AlarmScopeAnswer {
        AlarmScopeAnswer {
            at_least: Severity::Warning,
            answer: a,
            reason: reason.into(),
            says: "s".into(),
            firing: usize::from(a == AlarmAnswerToken::Yes),
            established: 1,
            unobservable: 0,
            not_asked: 0,
        }
    }

    fn report(any: AlarmScopeAnswer) -> AlarmsReport {
        AlarmsReport {
            namespace: "acme".into(),
            selector: "zk2/lab/*/alarms.v1/state/alarms/*".into(),
            presence: Some(AlarmsPresence {
                selector: "zk2/lab/*/@zk/**".into(),
                complete: true,
                cut_off_during: false,
                services: 1,
                listing: 1,
            }),
            face: None,
            window_s: None,
            clock: ClockGround::DeploymentWord,
            asked: vec!["zk2/lab/*/alarms.v1/state/alarms/*".into()],
            any_firing: any,
            owners: vec![],
            alarms: vec![],
            lost: 0,
            unobservable: None,
        }
    }

    /// The report's wire shape: §5's answers in the fixtures' tokens, the
    /// document as the profile writes it, a reference in three members
    /// (§2.12), the archive's document apart and last-known, and every
    /// optional field absent when empty.
    #[test]
    fn the_alarms_report_pins_its_shape() {
        let mut r = report(scope(AlarmAnswerToken::Yes, "any_firing"));
        r.window_s = Some(31.0);
        r.lost = 2;
        r.owners = vec![AlarmOwnerRow {
            address: "lab/svc".into(),
            known_by: vec![KnownBy::Presence, KnownBy::Archive],
            verdict: AlarmSetVerdict::Established,
            reason: "answered".into(),
            says: "s".into(),
            alarms: 1,
            window_s: Some(12.5),
            profile: None,
            one_slot_each: None,
        }];
        r.alarms = vec![AlarmRow {
            key: "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da".into(),
            alarm: Some(a_ref()),
            profile: None,
            slot: None,
            words: None,
            displaced: None,
            firing: answer(AlarmAnswerToken::Yes, "firing"),
            stale: answer(AlarmAnswerToken::No, "fresh"),
            rederives: answer(AlarmAnswerToken::Yes, "strict"),
            severity: Some(Severity::Warning),
            latest: Some(AlarmSeen {
                latest: AlarmLatest::Firing,
                document: Some(doc()),
                invalid: None,
                stamp: Some(Stamp {
                    time: "2026-10-20T00:00:00.000000000Z".into(),
                    clock: "ab12".into(),
                }),
                via: AlarmVia::Get,
                record: None,
                at_s: None,
            }),
            problems: vec!["label_name_form".into()],
            foreign_stamp: true,
            last_known: Some(LastKnownAlarm {
                archive: "lab/archive".into(),
                document: None,
                deleted: true,
                stamp: None,
                confirmed: false,
                record: None,
            }),
        }];
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["clock"], "deployment_word");
        assert_eq!(v["lost"], 2);
        assert_eq!(
            v["any_firing"],
            json!({"at_least": "warning", "answer": "yes", "reason": "any_firing", "says": "s",
                   "firing": 1, "established": 1, "unobservable": 0, "not_asked": 0})
        );
        assert_eq!(
            v["owners"][0],
            json!({"address": "lab/svc", "known_by": ["presence", "archive"],
                   "verdict": "established", "reason": "answered", "says": "s", "alarms": 1,
                   "window_s": 12.5})
        );
        assert_eq!(
            v["alarms"][0],
            json!({
                "key": "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da",
                "alarm": {"system": "lab", "service": "svc", "alert_key": "a659f813308ad1da"},
                "firing": {"answer": "yes", "reason": "firing", "says": "s"},
                "stale": {"answer": "no", "reason": "fresh", "says": "s"},
                "rederives": {"answer": "yes", "reason": "strict", "says": "s"},
                "severity": "warning",
                "latest": {
                    "latest": "firing",
                    "document": {"timestamp": 1_760_950_000_000_u64, "rule": "link_down",
                                 "severity": "warning", "state": "firing",
                                 "summary": "link to r2 down on eth0",
                                 "labels": {"peer": "r2", "port": "eth0"}},
                    "stamp": {"time": "2026-10-20T00:00:00.000000000Z", "clock": "ab12"},
                    "via": "get",
                },
                "problems": ["label_name_form"],
                "foreign_stamp": true,
                "last_known": {"archive": "lab/archive", "deleted": true, "confirmed": false},
            })
        );
        let empty = AlarmsReport {
            presence: None,
            face: Some(AlarmsFace { reachable: false }),
            ..report(scope(AlarmAnswerToken::Unobservable, "owner_unobservable"))
        };
        let v = serde_json::to_value(&empty).unwrap();
        assert_eq!(v["face"], json!({"reachable": false}));
        for absent in ["presence", "window_s", "lost", "unobservable"] {
            assert!(v.get(absent).is_none(), "{absent}: {v}");
        }
    }

    /// An `alarms.v2` row's wire shape (#984's S3): its profile, its slot,
    /// its words, its record in the profile's JSON reading and its `at`; an
    /// unnamed one has no reference, and its numbers say why. Its owner row
    /// says its profile and "one slot each".
    #[test]
    fn an_alarms_v2_row_pins_its_shape() {
        let words = AlarmWords {
            by: AlarmNamedBy::Table,
            rule: Some("link_down".into()),
            index: Some(1),
            labels: BTreeMap::from([("peer".into(), "r2".into()), ("port".into(), "eth0".into())]),
            codes: None,
            summary: Some("Link to r2 on eth0 is down".into()),
            category: None,
            argument: None,
            reason: None,
        };
        let row = AlarmRow {
            key: "zk2/lab/svc/alarms.v2/state/alarms/0".into(),
            alarm: Some(a_ref()),
            profile: Some("alarms.v2"),
            slot: Some(0),
            words: Some(words),
            displaced: None,
            firing: answer(AlarmAnswerToken::Yes, "firing"),
            stale: answer(AlarmAnswerToken::No, "fresh"),
            rederives: answer(AlarmAnswerToken::NotAsked, "table"),
            severity: Some(Severity::Warning),
            latest: Some(AlarmSeen {
                latest: AlarmLatest::Firing,
                document: None,
                invalid: None,
                stamp: None,
                via: AlarmVia::Get,
                record: Some(json!({"state": "firing", "severity": "warning", "rule": 1,
                                    "at": 1_794_062_759_000_i64, "labels": 2,
                                    "argument": "not_available"})),
                at_s: Some(1_794_062_759),
            }),
            problems: vec![],
            foreign_stamp: false,
            last_known: None,
        };
        assert_eq!(
            serde_json::to_value(&row).unwrap(),
            json!({
                "key": "zk2/lab/svc/alarms.v2/state/alarms/0",
                "alarm": {"system": "lab", "service": "svc", "alert_key": "a659f813308ad1da"},
                "profile": "alarms.v2",
                "slot": 0,
                "words": {"by": "table", "rule": "link_down", "index": 1,
                          "labels": {"peer": "r2", "port": "eth0"},
                          "summary": "Link to r2 on eth0 is down"},
                "firing": {"answer": "yes", "reason": "firing", "says": "s"},
                "stale": {"answer": "no", "reason": "fresh", "says": "s"},
                "rederives": {"answer": "not_asked", "reason": "table", "says": "s"},
                "severity": "warning",
                "latest": {
                    "latest": "firing",
                    "via": "get",
                    "record": {"state": "firing", "severity": "warning", "rule": 1,
                               "at": 1_794_062_759_000_i64, "labels": 2,
                               "argument": "not_available"},
                    "at_s": 1_794_062_759,
                },
            })
        );
        let unnamed = AlarmRow {
            alarm: None,
            words: Some(AlarmWords {
                by: AlarmNamedBy::Numbers,
                rule: None,
                index: Some(1),
                labels: BTreeMap::new(),
                codes: Some(2),
                summary: None,
                category: None,
                argument: None,
                reason: Some("no_table".into()),
            }),
            ..row
        };
        let v = serde_json::to_value(&unnamed).unwrap();
        assert!(v.get("alarm").is_none(), "no reference: {v}");
        assert_eq!(
            v["words"],
            json!({"by": "numbers", "index": 1, "codes": 2, "reason": "no_table"})
        );
        let owner = AlarmOwnerRow {
            address: "lab/svc".into(),
            known_by: vec![KnownBy::Presence],
            verdict: AlarmSetVerdict::Established,
            reason: "snapshot".into(),
            says: "s".into(),
            alarms: 2,
            window_s: None,
            profile: Some("alarms.v2"),
            one_slot_each: Some(answer(AlarmAnswerToken::Yes, "one_slot_each")),
        };
        assert_eq!(
            serde_json::to_value(&owner).unwrap(),
            json!({"address": "lab/svc", "known_by": ["presence"], "verdict": "established",
                   "reason": "snapshot", "says": "s", "alarms": 2, "profile": "alarms.v2",
                   "one_slot_each": {"answer": "yes", "reason": "one_slot_each", "says": "s"}})
        );
    }

    /// The run's judgement is §2.15's answer: yes the 1 whatever else, no
    /// the 0, and unobservable, not asked or an unreadable scope the 2.
    #[test]
    fn the_judgement_is_the_scopes_answer() {
        let exit = |r: &AlarmsReport| judgement_exit_code(&r.judgement());
        assert_eq!(exit(&report(scope(AlarmAnswerToken::Yes, "any_firing"))), 1);
        assert_eq!(exit(&report(scope(AlarmAnswerToken::No, "none_firing"))), 0);
        assert_eq!(
            exit(&report(scope(
                AlarmAnswerToken::Unobservable,
                "owner_unobservable"
            ))),
            2
        );
        assert_eq!(
            exit(&report(scope(AlarmAnswerToken::NotAsked, "no_owner"))),
            2
        );
        let mut whole = report(scope(AlarmAnswerToken::No, "none_firing"));
        whole.unobservable = Some("the presence read could not be made".into());
        assert_eq!(exit(&whole), 2, "no answer stands on a scope not read");
        whole.any_firing = scope(AlarmAnswerToken::Yes, "any_firing");
        assert_eq!(exit(&whole), 1, "a firing alarm stands whatever else");
    }

    /// A transition's wire shape: the change, the first question's answer
    /// after it, and a loss counted with no document.
    #[test]
    fn a_transition_pins_its_shape() {
        let t = AlarmTransition {
            key: "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da".into(),
            alarm: Some(a_ref()),
            change: AlarmChange::Lost,
            firing: answer(AlarmAnswerToken::Unobservable, "incomplete"),
            severity: Some(Severity::Critical),
            document: None,
            stamp: None,
            via: None,
            lost: Some(AlarmLoss {
                dropped: 3,
                stalled: false,
            }),
            profile: None,
            slot: None,
            words: None,
            record: None,
            displaced: None,
        };
        assert_eq!(
            serde_json::to_value(&t).unwrap(),
            json!({
                "key": "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da",
                "alarm": {"system": "lab", "service": "svc", "alert_key": "a659f813308ad1da"},
                "change": "lost",
                "firing": {"answer": "unobservable", "reason": "incomplete", "says": "s"},
                "severity": "critical",
                "lost": {"dropped": 3, "stalled": false},
            })
        );
        for c in [
            AlarmChange::Raised,
            AlarmChange::Escalated,
            AlarmChange::Refreshed,
            AlarmChange::Confirmed,
            AlarmChange::Resolved,
            AlarmChange::Retired,
            AlarmChange::Unreadable,
            AlarmChange::Lost,
            AlarmChange::Judged,
        ] {
            assert_eq!(serde_json::to_value(c).unwrap(), json!(c.as_str()));
        }
    }
}
