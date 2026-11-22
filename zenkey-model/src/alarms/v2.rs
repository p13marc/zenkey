//! `alarms.v2`: the active alarms of one service, in slots
//! (`spec/profiles/alarms/v2.md`, text 0.1, #984's S1).
//!
//! The profile's session-free half. It restates `alarms.v1` by reference,
//! so the alert key, the sanitizing step, the severities, §2.6's table and
//! "is anything firing?" are [`super`]'s, over either major. What this
//! module adds is what the major replaces:
//!
//! ```text
//! Record                      ─▶ the packed `Alarm` (§2.4, §3): write, put, read; core §7.6's refusals, then key_with_rule, key_missing
//! slot_of · alarm_key · …     ─▶ §2.2, §2.12: a slot is the key's last chunk, and a reference names a collection read
//! Rule, declaration(…)        ─▶ §4: a contract's `alarms.rules`: none │ a list │ one of eight findings; lint_contract
//! instance_table(named…)      ─▶ the one table among the revisions a descriptor names: none │ a list │ differ
//! write_alarm · Record::name  ─▶ words ⇄ a rule's index, its labels' codes and its argument, or by words with the alert key
//! Slots                       ─▶ §2.2, §2.8, §2.9: the slot an identity takes, the bound's refusal, a restart's adoption
//! taken(seen, run)            ─▶ §2.6: which of two slots a reader takes for one identity, and §2.2's break
//! Words, words_taken          ─▶ §2.4: `words/{slot}`, taken only where they derive the record's key
//! Track::apply(sample)        ─▶ §2.6: one slot read by identity: rebound │ replaced │ dropped, then v1's table
//! Dating, acknowledged        ─▶ §2.5: `at`, new at a transition, a second at a time, and the acknowledgement it carries
//! Seed::of(replies)           ─▶ a late joiner's snapshot, entry by entry (§3, core §7.5)
//! firing · stale_firing       ─▶ §2.14, with `rebound`; the state is the record's, never a bit's or a count's (§2.7)
//! owner_set(p, read, since)   ─▶ §2.15: established by one complete snapshot, with an entry or with none
//! ```
//!
//! **No I/O, no clock, and no file.** The standard schema is restated here
//! as a declaration ([`layout`]), since no crate may read `spec/`;
//! `tests/profiles/alarms_v2.rs` holds it to
//! `spec/profiles/alarms/alarms.v2.json`'s layout, its form's hash and its
//! tag. The owner's puts and its record file are the runtime's, and the
//! readers on a bus the fleet's (#984's S2 and S3).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{Map, Value, json};

use super::{
    Answer, Asked, Judged, Presence, PutFacts, Reason, SetJudged, SetVerdict, Severity, State,
    Step5, Transition,
};
use crate::collection::{self, Confirmation, Page};
use crate::contract::Contract;
use crate::freshness;
use crate::grammar::{self, Addr, IfaceId, Key, KeyError, KindToken, ZkKey};
use crate::heartbeat;
use crate::packed::{self, Decimal, Fixed, Layout, ReadRefusal, WriteRefusal};
use crate::template::Template;
use crate::wire::{Envelope, Run, Seq, Stamp, Tag};
use crate::words;

/// The profile's id, which is also its standard contract's interface id.
pub const PROFILE: &str = "alarms.v2";

/// The standard contract's interface id (§3).
pub const IFACE: &str = "alarms.v2";

/// Revision 2.0 of the standard contract (§3), `alarms.v2.toml`: its
/// fingerprint's hex. Its heartbeat's tag is its last hex digit, 7 (core
/// §7.5).
pub const FINGERPRINT: &str = "2292477cadf555dd09b07499ca048add4e48c63d60a209c13eaf543988c32cb7";

/// The two standard schemas' stem (§3): `alarms.v2.json`, the packed file
/// and the JSON Schema under `schemas/`.
pub const SCHEMA_STEM: &str = "alarms.v2";

/// The standard schemas' artifact ids the profile publishes (§3), oldest
/// first: text 0.1's packed `Alarm`, then its JSON `alarms.v2.Words`.
pub const PUBLISHED: &[&str] = &[
    "sha256:fe59e748fc957322428ac633a0908d09aa8b6c66952f7a9cdd8608db715a2b2a",
    "sha256:a3515ad47772ac778ccfbad3850a086ac18c28a9975eb7c8288c3437db9b99de",
];

/// Whether `id` is one of the standard schemas' published artifact ids.
#[must_use]
pub fn is_published(id: &str) -> bool {
    PUBLISHED.contains(&id)
}

/// The annotation keys' names (§4): what W105 reads for a contract that
/// lists `alarms.v2` (core §10). `alarms.v1`'s table stays empty.
pub const VOCABULARY: &[&str] = &["rules"];

/// The one annotation key (§4): a service's own table.
pub const RULES: &str = "alarms.rules";

/// The records' template (§3): `state/alarms/{slot}`.
pub const ALARMS: &str = "alarms/{slot}";

/// The words' template (§3): `state/words/{slot}`.
pub const WORDS: &str = "words/{slot}";

const ALARMS_CHUNK: &str = "alarms";
const WORDS_CHUNK: &str = "words";

/// The collection's index of `state/alarms/{slot}` in a snapshot (§3): the
/// contract's `state` resources in the canonical form's order.
pub const ALARMS_COLL: u32 = 0;

/// The collection's index of `state/words/{slot}` in a snapshot (§3).
pub const WORDS_COLL: u32 = 1;

/// The packed `Alarm`'s tag (§3, core §7.6): never 0, the tag a type of
/// any other kind carries.
pub const TAG: u8 = 11;

/// The contract's bound on one service's firing alarms, and the number of
/// its slots (§2.2, §2.8): a descriptor may lower it.
pub const MAX_SLOTS: u16 = 256;

/// The most rules a table holds (§4): a record's `rule` has the indices 0
/// to 254.
pub const MAX_RULES: usize = 255;

/// The greatest index of a table's rule.
const MAX_INDEX: u8 = 254;

/// The most bits a rule's labels take (§4).
pub const LABELS_BITS: u32 = 24;

/// The class of the alarms' collection (§2.7, `freshness.v2` §2.4): the
/// standard contract exposes nothing `link`, so an owner heartbeat that
/// covers the alarms is the `host` key's.
pub const CLASS: heartbeat::Class = heartbeat::Class::Host;

/// Whether a contract lists `alarms.v2` in `uses`: one that may declare
/// the service's own table (§4).
#[must_use]
pub fn uses_v2(c: &Contract) -> bool {
    words::uses(c, PROFILE)
}

fn iface() -> IfaceId {
    IfaceId::new(super::NAME, 2).expect("alarms.v2 is an interface id")
}

// --- §2.2, §2.12: slots and keys ---------------------------------------------

/// A key's chunk read as a slot (§2.2): a number from 0 to 255, written in
/// decimal with no leading zero, as core §2.2 writes a `uint`. `None` for
/// any other chunk, which is on no slot.
#[must_use]
pub fn parse_slot(chunk: &str) -> Option<u8> {
    let digits = !chunk.is_empty() && chunk.bytes().all(|b| b.is_ascii_digit());
    if !digits || (chunk.len() > 1 && chunk.starts_with('0')) {
        return None;
    }
    chunk.parse().ok()
}

/// An owner and one of its slots, as a key of either template names them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotKey {
    pub system: String,
    pub service: String,
    pub slot: u8,
}

fn slot_under(key: &str, chunk: &str) -> Option<SlotKey> {
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
    let [lit, slot] = resource.as_slice() else {
        return None;
    };
    if lit != chunk {
        return None;
    }
    Some(SlotKey {
        system: addr.system.as_str().to_owned(),
        service: addr.service.as_str().to_owned(),
        slot: parse_slot(slot)?,
    })
}

/// The owner and the slot a record's key names (§2.2), `None` for a key
/// that is no `alarms.v2/state/alarms/<slot>`: a reader reads nothing from
/// such a member.
#[must_use]
pub fn slot_of(key: &str) -> Option<SlotKey> {
    slot_under(key, ALARMS_CHUNK)
}

/// The same for `alarms.v2/state/words/<slot>` (§2.4).
#[must_use]
pub fn words_slot_of(key: &str) -> Option<SlotKey> {
    slot_under(key, WORDS_CHUNK)
}

fn key_under(system: &str, service: &str, chunk: &str, slot: u8) -> Result<Key, KeyError> {
    let addr = Addr::new(system, service)?;
    grammar::data_key(
        &addr,
        &iface(),
        KindToken::State,
        &[chunk, &slot.to_string()],
    )
}

/// A slot's record's key (§2.2):
/// `zk2/<system>/<service>/alarms.v2/state/alarms/<slot>`.
///
/// # Errors
/// [`KeyError`] when the system or the service is not a name (core §1.2).
pub fn alarm_key(system: &str, service: &str, slot: u8) -> Result<Key, KeyError> {
    key_under(system, service, ALARMS_CHUNK, slot)
}

/// A slot's words' key (§2.4):
/// `zk2/<system>/<service>/alarms.v2/state/words/<slot>`.
///
/// # Errors
/// As [`alarm_key`].
pub fn words_key(system: &str, service: &str, slot: u8) -> Result<Key, KeyError> {
    key_under(system, service, WORDS_CHUNK, slot)
}

/// Where a reference's alarm is found (§2.12): the selector of the
/// collection read of its owner's records, and the snapshot key its answer
/// comes on (core §4.2). A reference names no key and never a slot: the
/// alarm is the one among those records that derives its alert key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRead {
    pub selector: String,
    pub snapshot_key: String,
}

/// [`CollectionRead`] for an owner's alarms.
///
/// # Errors
/// As [`alarm_key`].
pub fn collection_read(system: &str, service: &str) -> Result<CollectionRead, KeyError> {
    let addr = Addr::new(system, service)?;
    let template = Template::parse(ALARMS).expect("the standard template parses");
    let members = grammar::template_selector(&addr, &iface(), KindToken::State, &template);
    let selector = collection::collection_read(&members, &template)
        .expect("a template whose first chunk is a literal has a snapshot key");
    let snapshot_key =
        grammar::data_key(&addr, &iface(), KindToken::State, &[ALARMS_CHUNK])?.to_string();
    Ok(CollectionRead {
        selector,
        snapshot_key,
    })
}

// --- §2.4, §3: the record -----------------------------------------------------

/// The record's name for a `rule` or an `argument` that is none (core
/// §7.6's reserved code).
pub const NOT_AVAILABLE: &str = "not_available";

/// A record's `rule` (§2.4): the index of the alarm's rule in its owner's
/// table, or `not_available` for an alarm by words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuleIndex {
    /// A table's rule, by its index from 0 to 254.
    Table(u8),
    /// No table names the alarm: its words are on `words/<slot>`, and its
    /// record carries `key`.
    NotAvailable,
}

impl RuleIndex {
    /// The index `i`, from 0 to 254; `None` for 255, which is no index but
    /// `not_available`'s code.
    #[must_use]
    pub const fn new(i: u8) -> Option<RuleIndex> {
        if i <= MAX_INDEX {
            Some(RuleIndex::Table(i))
        } else {
            None
        }
    }

    /// A rule's place in its table as a record carries it
    /// ([`words::Index`]): a place the record has no code for is by words.
    #[must_use]
    pub const fn of(index: words::Index) -> RuleIndex {
        match index {
            words::Index::At(i) => RuleIndex::Table(i),
            words::Index::OutOfRange => RuleIndex::NotAvailable,
        }
    }

    /// Core §7.6's JSON reading: the number, or the reserved code's name.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            RuleIndex::Table(i) => json!(i),
            RuleIndex::NotAvailable => json!(NOT_AVAILABLE),
        }
    }

    /// [`RuleIndex::to_json`] read back; `None` for anything else.
    #[must_use]
    pub fn of_json(v: &Value) -> Option<RuleIndex> {
        match v {
            Value::String(s) if s == NOT_AVAILABLE => Some(RuleIndex::NotAvailable),
            v => RuleIndex::new(u8::try_from(v.as_u64()?).ok()?),
        }
    }
}

/// A record's `argument` (§2.4, §4): the rule's own number as a code, read
/// through the rule's declaration ([`Rule::reading`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Argument {
    /// A code from 0 to 1,048,574.
    Code(u32),
    /// The rule declares none, the reading has no code, or there is none.
    NotAvailable,
}

impl Argument {
    /// The largest code that is a value, 1,048,574.
    pub const MAX: u32 = 1_048_574;

    /// The code `n`, from 0 to 1,048,574; `None` above.
    #[must_use]
    pub const fn new(n: u32) -> Option<Argument> {
        if n <= Argument::MAX {
            Some(Argument::Code(n))
        } else {
            None
        }
    }

    /// Core §7.6's JSON reading: the number, or the reserved code's name.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Argument::Code(n) => json!(n),
            Argument::NotAvailable => json!(NOT_AVAILABLE),
        }
    }

    /// [`Argument::to_json`] read back; `None` for anything else.
    #[must_use]
    pub fn of_json(v: &Value) -> Option<Argument> {
        match v {
            Value::String(s) if s == NOT_AVAILABLE => Some(Argument::NotAvailable),
            v => Argument::new(u32::try_from(v.as_u64()?).ok()?),
        }
    }
}

/// The standard schema's one packed type, without its descriptions, which
/// are no part of a layout (core §7.6):
/// `spec/profiles/alarms/alarms.v2.json`'s fields, in order.
fn schema() -> Value {
    json!({"types": {"Alarm": {"fields": [
        {"name": "state", "type": "enum", "values": ["firing", "resolved"]},
        {"name": "severity", "type": "enum", "values": ["info", "warning", "critical"],
         "escape": true},
        {"name": "rule", "type": "int", "minimum": 0, "maximum": MAX_INDEX,
         "reserved": [NOT_AVAILABLE]},
        {"name": "at", "type": "time", "optional": true},
        {"name": "labels", "type": "int", "minimum": 0, "maximum": (1u32 << LABELS_BITS) - 1,
         "optional": true},
        {"name": "argument", "type": "int", "minimum": 0, "maximum": Argument::MAX,
         "reserved": [NOT_AVAILABLE]},
        {"name": "key", "type": "counter", "width": 64, "optional": true},
        packed::MARKER,
    ]}}})
}

/// The packed type's name in the standard schema (§3).
pub const TYPE: &str = "Alarm";

/// The `Alarm`'s layout (§3): 35 bits with no optional field, 67 with its
/// time, and its tag 11.
#[must_use]
pub fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        packed::layout_in(&schema(), TYPE).expect("the standard schema's declaration reads")
    })
}

/// The time an owner writes as a record's `at` (§2.4), in whole seconds
/// since the Unix epoch, for its clock reading `unix_ms` under the
/// deployment epoch `epoch` in seconds: `None` where that instant has no
/// code, before the epoch or 2^32 seconds or more after it, and the owner
/// writes none. A device with no real-time clock at boot reads 1970, which
/// a catalog's later epoch does not code.
#[must_use]
pub fn time_at(unix_ms: i128, epoch: i64) -> Option<i64> {
    packed::time_code(unix_ms, Some(epoch), false).ok()?;
    i64::try_from(unix_ms.div_euclid(1000)).ok()
}

/// Why a writer refuses a record (§2.4): core §7.6's refusal of the value,
/// then this text's, for a value the core writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// Core §7.6's: `missing`, `unknown`, `value` or `range`.
    Core(WriteRefusal),
    /// `key` given under a table's rule: a reader reads it undecodable.
    KeyWithRule,
    /// No `key` under `not_available`: the same.
    KeyMissing,
    /// `labels` given, or an `argument` other than `not_available`, under
    /// `not_available`: an alarm by words has both in its words.
    ByWordsFields,
}

impl Refused {
    /// The fixture's token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Core(r) => r.token(),
            Self::KeyWithRule => "key_with_rule",
            Self::KeyMissing => "key_missing",
            Self::ByWordsFields => "by_words_fields",
        }
    }
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(r) => r.fmt(f),
            Self::KeyWithRule => f.write_str("key_with_rule: `key` beside a table's rule"),
            Self::KeyMissing => f.write_str("key_missing: no `key` under a rule not_available"),
            Self::ByWordsFields => {
                f.write_str("by_words_fields: `labels` or an `argument` under a rule not_available")
            }
        }
    }
}

impl std::error::Error for Refused {}

impl From<WriteRefusal> for Refused {
    fn from(r: WriteRefusal) -> Self {
        Refused::Core(r)
    }
}

/// Why a reader reads a record as undecodable (§2.4), the first that
/// applies: it is then never read at a severity or in a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undecodable {
    /// Core §7.6's reading refuses it: by its `tag`, its `length` or its
    /// `pad`. No field has an unused code, and the layout ends at its
    /// marker, so none is refused `code` or `extension`.
    Core(ReadRefusal),
    /// `key` is present and `rule` is a table's.
    KeyWithRule,
    /// `key` is absent and `rule` is `not_available`.
    KeyMissing,
}

impl Undecodable {
    /// The fixture's token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            Self::Core(r) => r.token(),
            Self::KeyWithRule => "key_with_rule",
            Self::KeyMissing => "key_missing",
        }
    }
}

impl std::fmt::Display for Undecodable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(r) => r.fmt(f),
            Self::KeyWithRule => f.write_str("key_with_rule: `key` beside a table's rule"),
            Self::KeyMissing => f.write_str("key_missing: no `key` under a rule not_available"),
        }
    }
}

impl std::error::Error for Undecodable {}

impl From<ReadRefusal> for Undecodable {
    fn from(r: ReadRefusal) -> Self {
        Undecodable::Core(r)
    }
}

/// One alarm's record, the packed `Alarm` on `alarms/<slot>` (§2.4, §3).
/// The slot is not in it: it is the key's last chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Record {
    pub state: State,
    /// `None`: the record's escape, a severity a later text names, read as
    /// no severity known.
    pub severity: Option<Severity>,
    pub rule: RuleIndex,
    /// When the alarm's latest transition happened, on the owner's clock,
    /// in whole seconds since the Unix epoch; absent while that clock reads
    /// no instant the epoch codes (§2.5). It is the acknowledgement
    /// identity ([`acknowledged`]).
    pub at: Option<i64>,
    /// The codes of the rule's labels as one integer below 2^24 (§4):
    /// present exactly where the rule declares a label.
    pub labels: Option<u32>,
    pub argument: Argument,
    /// The alert key's 64 bits: present exactly for an alarm by words.
    pub key: Option<u64>,
}

/// A record's time as core §7.6's reading spells it, milliseconds since
/// the Unix epoch, from whole seconds since it.
fn time_json(seconds: i64) -> Value {
    json!(i128::from(seconds) * 1000)
}

fn time_of_json(v: &Value) -> Option<i64> {
    let ms = match v {
        Value::Number(n) => n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from))?,
        _ => return None,
    };
    i64::try_from(ms.div_euclid(1000)).ok()
}

impl Record {
    /// The record of an alarm in `state` at `severity`, with the fields
    /// that name it ([`write_alarm`]) and the `at` of its latest
    /// transition ([`Dating`]).
    #[must_use]
    pub fn new(state: State, severity: Severity, fields: Fields, at: Option<i64>) -> Record {
        Record {
            state,
            severity: Some(severity),
            rule: fields.rule,
            at,
            labels: fields.labels,
            argument: fields.argument,
            key: fields.key,
        }
    }

    /// Whether the record is an alarm's by words: its `rule` is
    /// `not_available`.
    #[must_use]
    pub const fn is_by_words(&self) -> bool {
        matches!(self.rule, RuleIndex::NotAvailable)
    }

    /// The record's value in core §7.6's JSON reading: a severity its
    /// name, or null for the escape; `rule` and `argument` a number or
    /// `not_available`; `at` in milliseconds since the Unix epoch; `key`
    /// an integer; an absent optional field no member.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("state".into(), json!(self.state.as_str()));
        m.insert(
            "severity".into(),
            self.severity.map_or(Value::Null, |s| json!(s.as_str())),
        );
        m.insert("rule".into(), self.rule.to_json());
        if let Some(t) = self.at {
            m.insert("at".into(), time_json(t));
        }
        if let Some(l) = self.labels {
            m.insert("labels".into(), json!(l));
        }
        m.insert("argument".into(), self.argument.to_json());
        if let Some(k) = self.key {
            m.insert("key".into(), json!(k));
        }
        Value::Object(m)
    }

    /// A value [`Layout::read`] gave for the type; `None` for any other.
    #[must_use]
    pub fn of_json(v: &Value) -> Option<Record> {
        let state = match v.get("state")?.as_str()? {
            "firing" => State::Firing,
            "resolved" => State::Resolved,
            _ => return None,
        };
        let severity = match v.get("severity")? {
            Value::Null => None,
            s => Some(Severity::parse(s.as_str()?)?),
        };
        let optional = |name: &str| v.get(name);
        Some(Record {
            state,
            severity,
            rule: RuleIndex::of_json(v.get("rule")?)?,
            at: match optional("at") {
                None => None,
                Some(t) => Some(time_of_json(t)?),
            },
            labels: match optional("labels") {
                None => None,
                Some(l) => Some(u32::try_from(l.as_u64()?).ok()?),
            },
            argument: Argument::of_json(v.get("argument")?)?,
            key: match optional("key") {
                None => None,
                Some(k) => Some(k.as_u64()?),
            },
        })
    }

    /// What a reader refuses of a record that core §7.6 reads (§2.4): a
    /// `key` beside a table's rule, or none under `not_available`.
    ///
    /// # Errors
    /// [`Undecodable::KeyWithRule`] or [`Undecodable::KeyMissing`].
    pub fn check(&self) -> Result<(), Undecodable> {
        match (self.rule, self.key) {
            (RuleIndex::Table(_), Some(_)) => Err(Undecodable::KeyWithRule),
            (RuleIndex::NotAvailable, None) => Err(Undecodable::KeyMissing),
            _ => Ok(()),
        }
    }

    /// The record's bytes, its `at` counted from the deployment epoch
    /// `epoch` in seconds (0 where the owner is given no catalog).
    ///
    /// # Errors
    /// [`Refused`]: core §7.6's `range` for an `at` the epoch does not
    /// code (an owner writes none, [`time_at`]), for labels of 2^24 or
    /// more; then what no owner writes, since a reader of this text reads
    /// it undecodable or ignores it.
    pub fn write(&self, epoch: i64) -> Result<packed::Record, Refused> {
        write_value(&self.to_json(), epoch)
    }

    /// A put's payload under the owner's own `stamp`: CH (form 0, the
    /// stamp's width bit, the type's tag), the run, the seq, the record.
    ///
    /// # Errors
    /// As [`Record::write`].
    pub fn put(&self, stamp: Stamp, epoch: i64) -> Result<Vec<u8>, Refused> {
        let record = self.write(epoch)?;
        let tag = Tag::new(TAG).expect("a tag of four bits");
        Ok(Envelope::mutation(tag, stamp, &record.bytes).write())
    }

    /// Reads a record under the tag its envelope's CH carries, or its
    /// snapshot page's (§2.4): core §7.6's reading, a later text's fields
    /// behind the marker ignored, then the record's `key` against its
    /// `rule`. `labels` and `argument` under `not_available` are kept as
    /// read, and a reader ignores them.
    ///
    /// # Errors
    /// [`Undecodable`], the first that applies.
    pub fn read(tag: u8, bytes: &[u8], epoch: i64) -> Result<Record, Undecodable> {
        let value = layout().read(tag, bytes, Some(epoch))?;
        // Every value the layout reads is one the type declares.
        let record = Record::of_json(&value).ok_or(ReadRefusal::Code)?;
        record.check()?;
        Ok(record)
    }
}

/// Writes a record from its value in core §7.6's JSON reading
/// ([`Record::to_json`]), under the deployment epoch `epoch` in seconds.
///
/// # Errors
/// [`Refused`]: core §7.6's refusal of the value first, then
/// `key_with_rule`, `key_missing` and `by_words_fields`.
pub fn write_value(value: &Value, epoch: i64) -> Result<packed::Record, Refused> {
    let record = layout().write(value, Some(epoch))?;
    // The core wrote it, so it reads back as a record.
    let r = Record::of_json(value).ok_or(WriteRefusal::Value)?;
    match r.check() {
        Err(Undecodable::KeyWithRule) => return Err(Refused::KeyWithRule),
        Err(_) => return Err(Refused::KeyMissing),
        Ok(()) => {}
    }
    if r.is_by_words() && (r.labels.is_some() || r.argument != Argument::NotAvailable) {
        return Err(Refused::ByWordsFields);
    }
    Ok(record)
}

// --- §2.5: `at`, and the acknowledgement it carries ---------------------------

/// An owner's act on one alarm (§2.5's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    Raise,
    Escalate,
    Refresh,
    Resolve,
}

impl Act {
    /// Whether the act is a transition, which takes a new `at`: a raise,
    /// an escalation or a resolve. A refresh keeps the one its record
    /// holds.
    #[must_use]
    pub const fn is_transition(self) -> bool {
        !matches!(self, Act::Refresh)
    }
}

/// The `at` of one identity's transitions (§2.5): the greatest `at` its
/// owner knows it wrote for the identity, so that each transition's is
/// later than the one before it, whatever the clock does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Dating {
    greatest: Option<i64>,
}

impl Dating {
    /// An identity whose `at` the owner knows of from before: an adopted
    /// alarm's (§2.9), `None` where it carried none.
    #[must_use]
    pub const fn known(at: Option<i64>) -> Dating {
        Dating { greatest: at }
    }

    /// The greatest `at` written for the identity, in whole seconds since
    /// the Unix epoch.
    #[must_use]
    pub const fn greatest(&self) -> Option<i64> {
        self.greatest
    }

    /// A transition's new `at` (§2.5), the owner's clock reading
    /// `clock_ms` in milliseconds since the Unix epoch under the
    /// deployment epoch `epoch` in seconds: the clock in whole seconds,
    /// or one second after the greatest `at` written where the clock is
    /// not later than it. `None`, and no `at` is written, while the clock
    /// reads no instant the epoch codes, and where the new `at` would have
    /// no code.
    pub fn transition(&mut self, clock_ms: i128, epoch: i64) -> Option<i64> {
        let clock = time_at(clock_ms, epoch)?;
        let new = match self.greatest {
            Some(g) if clock <= g => g.checked_add(1)?,
            _ => clock,
        };
        time_at(i128::from(new) * 1000, epoch)?;
        self.greatest = Some(new);
        Some(new)
    }

    /// The `at` the record of `act` carries: a new one at a transition
    /// ([`Dating::transition`]), and `held`, the one its record holds, at
    /// a refresh.
    pub fn at(&mut self, act: Act, held: Option<i64>, clock_ms: i128, epoch: i64) -> Option<i64> {
        if act.is_transition() {
            self.transition(clock_ms, epoch)
        } else {
            held
        }
    }
}

/// Whether an acknowledgement applies to an alarm (§2.5): while the
/// alarm's `at`, in whole seconds since the Unix epoch, is at most the
/// acknowledged instant, in milliseconds since it. Never with no `at` on
/// either side: the alarm is then told again. An acknowledgement made
/// against `alarms.v1`'s `timestamp` is compared as it is, so it still
/// applies to the alarm adopted ([`adopted_at`]).
#[must_use]
pub fn acknowledged(acked_at_ms: Option<i128>, at: Option<i64>) -> bool {
    match (acked_at_ms, at) {
        (Some(acked), Some(at)) => i128::from(at) * 1000 <= acked,
        _ => false,
    }
}

/// The `at` of an alarm adopted from `alarms.v1` (§2.9): its document's
/// `timestamp` in whole seconds, `None` where that instant has no code
/// under the deployment epoch.
#[must_use]
pub fn adopted_at(timestamp_ms: u64, epoch: i64) -> Option<i64> {
    time_at(i128::from(timestamp_ms), epoch)
}

// --- §4: the table --------------------------------------------------------------

/// How a rule's `argument` reads (§4): `minimum` + code × `resolution`,
/// two decimal strings as core §7.6 writes them, the resolution above 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArgumentDecl {
    minimum: String,
    resolution: String,
}

impl ArgumentDecl {
    /// A declaration of two decimal strings, `None` where either is none
    /// or the resolution is not above 0.
    #[must_use]
    pub fn new(minimum: &str, resolution: &str) -> Option<ArgumentDecl> {
        Decimal::parse(minimum)?;
        if Decimal::parse(resolution)? <= Decimal::from(0) {
            return None;
        }
        Some(ArgumentDecl {
            minimum: minimum.to_owned(),
            resolution: resolution.to_owned(),
        })
    }

    /// The declaration's `minimum`, as written.
    #[must_use]
    pub fn minimum(&self) -> &str {
        &self.minimum
    }

    /// The declaration's `resolution`, as written.
    #[must_use]
    pub fn resolution(&self) -> &str {
        &self.resolution
    }

    fn fixed(&self) -> Fixed {
        Fixed::new(
            &self.minimum,
            &self.resolution,
            u128::from(Argument::MAX) + 1,
        )
    }

    fn of_json(v: &Value) -> Option<ArgumentDecl> {
        let o = v.as_object()?;
        if o.len() != 2 {
            return None;
        }
        ArgumentDecl::new(o.get("minimum")?.as_str()?, o.get("resolution")?.as_str()?)
    }
}

/// A label's values (§4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LabelValues {
    /// `values`: one or more strings, a value's code its index.
    List(Vec<String>),
    /// `minimum` and `maximum`: the integers from one to the other, a
    /// value's code the integer less `minimum`.
    Range { minimum: i128, maximum: i128 },
}

/// One discriminating label of a rule (§4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    pub name: String,
    pub values: LabelValues,
}

impl Label {
    /// How many values the label has.
    #[must_use]
    pub fn count(&self) -> u128 {
        match &self.values {
            LabelValues::List(l) => l.len() as u128,
            LabelValues::Range { minimum, maximum } => {
                u128::try_from(maximum - minimum).map_or(0, |d| d + 1)
            }
        }
    }

    /// The bits the label's code takes (§4): ⌈log₂ n⌉ for n values, so
    /// none for a label of one value.
    #[must_use]
    pub fn width(&self) -> u32 {
        match self.count() {
            0 | 1 => 0,
            n => u128::BITS - (n - 1).leading_zeros(),
        }
    }

    /// The code of the value `text` (§4): its index in `values`, or its
    /// integer less `minimum`, the integer written in decimal with `-` for
    /// a negative one and no leading zero. `None` for a text that is no
    /// value of the label.
    #[must_use]
    pub fn code(&self, text: &str) -> Option<u64> {
        match &self.values {
            LabelValues::List(l) => l.iter().position(|v| v == text).map(|i| i as u64),
            LabelValues::Range { minimum, maximum } => {
                let n: i128 = text.parse().ok()?;
                if n.to_string() != text || n < *minimum || n > *maximum {
                    return None;
                }
                u64::try_from(n - minimum).ok()
            }
        }
    }

    /// The text of the value at `code` (§4), `None` for a code the label
    /// does not have.
    #[must_use]
    pub fn text(&self, code: u64) -> Option<String> {
        if u128::from(code) >= self.count() {
            return None;
        }
        match &self.values {
            LabelValues::List(l) => l.get(usize::try_from(code).ok()?).cloned(),
            LabelValues::Range { minimum, .. } => Some((minimum + i128::from(code)).to_string()),
        }
    }

    fn of_json(v: &Value) -> Option<Label> {
        let o = v.as_object()?;
        let name = o.get("name")?.as_str()?.to_owned();
        let values = match (o.get("values"), o.get("minimum"), o.get("maximum")) {
            (Some(list), None, None) if o.len() == 2 => {
                let list = words::list(list, usize::MAX, |_| true, false)?;
                if list.is_empty() {
                    return None;
                }
                LabelValues::List(list)
            }
            (None, Some(minimum), Some(maximum)) if o.len() == 3 => {
                let minimum = packed::json_integer(minimum).ok()?;
                let maximum = packed::json_integer(maximum).ok()?;
                if minimum > maximum {
                    return None;
                }
                LabelValues::Range { minimum, maximum }
            }
            _ => return None,
        };
        Some(Label { name, values })
    }

    fn to_json(&self) -> Value {
        match &self.values {
            LabelValues::List(l) => json!({"name": self.name, "values": l}),
            LabelValues::Range { minimum, maximum } => {
                // An integer a contract holds is within −2^63 to 2^64 − 1.
                let n = |x: i128| {
                    i64::try_from(x)
                        .map(Value::from)
                        .or_else(|_| u64::try_from(x).map(Value::from))
                        .unwrap_or(Value::Null)
                };
                json!({"name": self.name, "minimum": n(*minimum), "maximum": n(*maximum)})
            }
        }
    }
}

/// One rule of a service's table (§4): the rule at index i is the one a
/// record's `rule` i names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    /// The rule's name: v1 §2.4's `rule`, and the recipe's first input.
    pub name: String,
    /// A template: the alarm's one line for a person ([`Rule::summary`]).
    pub summary: String,
    /// v1 §2.4's `category`, the same for every alarm of the rule.
    pub category: Option<String>,
    pub argument: Option<ArgumentDecl>,
    /// The rule's discriminating labels, in the order their codes are
    /// written. Empty for a rule that declares none.
    pub labels: Vec<Label>,
}

impl AsRef<str> for Rule {
    /// The rule's name: what [`words::write`] finds a rule by.
    fn as_ref(&self) -> &str {
        &self.name
    }
}

/// One piece of a summary's template (§4).
enum Piece<'a> {
    Text(&'a str),
    /// A placeholder, by its name.
    Placeholder(&'a str),
    /// A `{` with no `}` after it, or a `}` outside a placeholder.
    Stray,
}

/// A summary's template in pieces: a `{` opens a placeholder, which ends
/// at the next `}`.
fn pieces(summary: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = summary;
    while !rest.is_empty() {
        match rest.find(['{', '}']) {
            None => {
                out.push(Piece::Text(rest));
                break;
            }
            Some(i) => {
                if i > 0 {
                    out.push(Piece::Text(&rest[..i]));
                }
                if rest[i..].starts_with('}') {
                    out.push(Piece::Stray);
                    rest = &rest[i + 1..];
                } else if let Some(end) = rest[i + 1..].find('}') {
                    out.push(Piece::Placeholder(&rest[i + 1..i + 1 + end]));
                    rest = &rest[i + end + 2..];
                } else {
                    out.push(Piece::Stray);
                    break;
                }
            }
        }
    }
    out
}

impl Rule {
    /// The bits the rule's labels take: their widths' sum.
    #[must_use]
    pub fn labels_width(&self) -> u32 {
        self.labels.iter().map(Label::width).sum()
    }

    /// The record's `labels` for an alarm whose labels are `labels`, each
    /// a name and its text (§2.4, §4): `Some(None)` for a rule that
    /// declares none, `Some(Some(codes))` for one that does, the first
    /// label in the most significant bits. `None` where the alarm is not
    /// the table's: its discriminating labels, host-scoped ones left out,
    /// are not exactly the rule's, or a text has no code.
    #[must_use]
    pub fn encode<'a>(
        &self,
        labels: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Option<Option<u32>> {
        let given: Vec<(&str, &str)> = labels
            .into_iter()
            .filter(|(n, _)| !super::is_host_scoped(n))
            .collect();
        if given.len() != self.labels.len() || self.labels_width() > LABELS_BITS {
            return None;
        }
        let mut codes: u64 = 0;
        for label in &self.labels {
            let mut of_name = given.iter().filter(|(n, _)| *n == label.name);
            let (_, text) = of_name.next()?;
            if of_name.next().is_some() {
                return None;
            }
            codes = (codes << label.width()) | label.code(text)?;
        }
        if self.labels.is_empty() {
            return Some(None);
        }
        Some(Some(u32::try_from(codes).ok()?))
    }

    /// The labels a record's `labels` names, each its name and its text,
    /// in the rule's order (§2.4). `None` where they name none: `labels`
    /// present where the rule declares no label, absent where it declares
    /// one, or holding a code a label does not have, a value at or above
    /// 2^w among them.
    #[must_use]
    pub fn decode(&self, labels: Option<u32>) -> Option<Vec<(String, String)>> {
        let Some(codes) = labels else {
            return self.labels.is_empty().then(Vec::new);
        };
        let width = self.labels_width();
        if self.labels.is_empty() || width > LABELS_BITS || u64::from(codes) >> width != 0 {
            return None;
        }
        let mut left = width;
        let mut out = Vec::with_capacity(self.labels.len());
        for label in &self.labels {
            let w = label.width();
            left -= w;
            let code = (u64::from(codes) >> left) & ((1u64 << w) - 1);
            out.push((label.name.clone(), label.text(code)?));
        }
        Some(out)
    }

    /// The code an owner writes for a reading (§4): round((reading −
    /// `minimum`) / `resolution`), a tie to the even code, as core §7.6
    /// writes a `fixed`. `not_available` for a reading whose code is
    /// outside 0 to 1,048,574, for none, and under a rule that declares no
    /// argument. The alarm still fires.
    #[must_use]
    pub fn argument(&self, reading: Option<&Decimal>) -> Argument {
        let (Some(decl), Some(reading)) = (&self.argument, reading) else {
            return Argument::NotAvailable;
        };
        decl.fixed()
            .code(reading)
            .ok()
            .and_then(|c| Argument::new(u32::try_from(c).ok()?))
            .unwrap_or(Argument::NotAvailable)
    }

    /// What a record's `argument` reads as (§4): `minimum` + code ×
    /// `resolution`, exactly, shown in its shortest form by its `Display`.
    /// `None` for `not_available`, and under a rule that declares no
    /// argument, where a reader ignores any code.
    #[must_use]
    pub fn reading(&self, argument: Argument) -> Option<Decimal> {
        let (Some(decl), Argument::Code(code)) = (&self.argument, argument) else {
            return None;
        };
        decl.fixed().value(u64::from(code)).ok()
    }

    /// The alarm's summary (§4): the rule's template with `{argument}`
    /// replaced by the argument's reading, or by `?` where there is none,
    /// and `{<label>}` by that label's text.
    #[must_use]
    pub fn summary(&self, labels: &[(String, String)], argument: Argument) -> String {
        let mut out = String::new();
        for piece in pieces(&self.summary) {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Stray => {}
                Piece::Placeholder("argument") if self.argument.is_some() => {
                    match self.reading(argument) {
                        Some(r) => out.push_str(&r.to_string()),
                        None => out.push('?'),
                    }
                }
                Piece::Placeholder(name) => {
                    if let Some((_, text)) = labels.iter().find(|(n, _)| n == name) {
                        out.push_str(text);
                    }
                }
            }
        }
        out
    }

    fn of_json(v: &Value) -> Option<Rule> {
        const MEMBERS: [&str; 5] = ["name", "summary", "category", "argument", "labels"];
        let o = v.as_object()?;
        if o.keys().any(|k| !MEMBERS.contains(&k.as_str())) {
            return None;
        }
        Some(Rule {
            name: o.get("name")?.as_str()?.to_owned(),
            summary: o.get("summary")?.as_str()?.to_owned(),
            category: match o.get("category") {
                None => None,
                Some(c) => Some(c.as_str()?.to_owned()),
            },
            argument: match o.get("argument") {
                None => None,
                Some(a) => Some(ArgumentDecl::of_json(a)?),
            },
            labels: match o.get("labels") {
                None => Vec::new(),
                Some(l) => {
                    let l = l.as_array().filter(|l| !l.is_empty())?;
                    l.iter().map(Label::of_json).collect::<Option<_>>()?
                }
            },
        })
    }

    /// The rule as its contract writes it (§4).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), json!(self.name));
        m.insert("summary".into(), json!(self.summary));
        if let Some(c) = &self.category {
            m.insert("category".into(), json!(c));
        }
        if let Some(a) = &self.argument {
            m.insert(
                "argument".into(),
                json!({"minimum": a.minimum, "resolution": a.resolution}),
            );
        }
        if !self.labels.is_empty() {
            m.insert(
                "labels".into(),
                Value::Array(self.labels.iter().map(Label::to_json).collect()),
            );
        }
        Value::Object(m)
    }
}

/// A table as its contract writes it: [`Rule::to_json`], in order.
#[must_use]
pub fn table_json(table: &[Rule]) -> Value {
    Value::Array(table.iter().map(Rule::to_json).collect())
}

/// A value of §4's shape: a list of at most 255 rules, each with the
/// members the text names and no other. `None` for any other value. A
/// list of the shape may still draw a finding ([`declaration`]).
#[must_use]
pub fn rules(v: &Value) -> Option<Vec<Rule>> {
    let list = v.as_array()?;
    if list.len() > MAX_RULES {
        return None;
    }
    list.iter().map(Rule::of_json).collect()
}

/// Why a contract's declaration of its table is invalid (§4), the first
/// that applies. An invalid declaration holds no table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invalid {
    /// A resource carries the key, and the contract's `uses` does not list
    /// `alarms.v2`.
    NotUsed,
    /// A resource's value is not a list of at most 255 rules of §4's
    /// shape.
    Value,
    /// A rule's name is empty, holds a newline, or is another rule's.
    RuleName,
    /// A label's name is empty, is `argument`, holds a newline, `=`, `{`
    /// or `}`, or is another label's of its rule; or one of its values
    /// holds a newline, or comes twice.
    Label,
    /// A rule's labels take more than 24 bits.
    LabelsWidth,
    /// A label's name is host-scoped (v1 §2.2).
    HostLabel,
    /// A summary holds a `{` with no `}` after it, a `}` outside a
    /// placeholder, or a placeholder whose name is neither a label of its
    /// rule nor `argument` under a rule that declares one.
    Placeholder,
    /// Two resources carry two different lists.
    Disagrees,
}

impl Invalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotUsed => "not_used",
            Self::Value => "value",
            Self::RuleName => "rule_name",
            Self::Label => "label",
            Self::LabelsWidth => "labels_width",
            Self::HostLabel => "host_label",
            Self::Placeholder => "placeholder",
            Self::Disagrees => "disagrees",
        }
    }

    /// The reason in words, for a linter's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotUsed => "the contract's `uses` does not list alarms.v2",
            Self::Value => {
                "its value is not a list of at most 255 rules: name, summary, and optionally \
                 category, argument { minimum, resolution } and labels"
            }
            Self::RuleName => "a rule's name is empty, holds a newline, or is another rule's",
            Self::Label => {
                "a label's name is empty, `argument`, holds a newline, `=`, `{` or `}`, or comes \
                 twice in its rule; or a value holds a newline or comes twice"
            }
            Self::LabelsWidth => "a rule's labels take more than 24 bits",
            Self::HostLabel => "a label's name is host-scoped: the key's system names the host",
            Self::Placeholder => {
                "a summary holds an unclosed `{`, a stray `}`, or a placeholder that is neither \
                 a label of its rule nor its `argument`"
            }
            Self::Disagrees => {
                "two resources carry two different lists: write the key once, on [defaults]"
            }
        }
    }

    /// Whether a list of §4's shape draws this finding. `not_used`,
    /// `value` and `disagrees` are no list's alone.
    fn found_in(self, table: &[Rule]) -> bool {
        let labels = || table.iter().flat_map(|r| r.labels.iter());
        match self {
            Self::NotUsed | Self::Value | Self::Disagrees => false,
            Self::RuleName => table.iter().enumerate().any(|(i, r)| {
                r.name.is_empty()
                    || r.name.contains('\n')
                    || table[..i].iter().any(|before| before.name == r.name)
            }),
            Self::Label => table.iter().any(|r| {
                r.labels.iter().enumerate().any(|(i, l)| {
                    l.name.is_empty()
                        || l.name == "argument"
                        || l.name.contains(['\n', '=', '{', '}'])
                        || r.labels[..i].iter().any(|before| before.name == l.name)
                        || match &l.values {
                            LabelValues::List(values) => values
                                .iter()
                                .enumerate()
                                .any(|(j, v)| v.contains('\n') || values[..j].contains(v)),
                            LabelValues::Range { .. } => false,
                        }
                })
            }),
            Self::LabelsWidth => table.iter().any(|r| r.labels_width() > LABELS_BITS),
            Self::HostLabel => labels().any(|l| super::is_host_scoped(&l.name)),
            Self::Placeholder => table.iter().any(|r| {
                pieces(&r.summary).iter().any(|p| match p {
                    Piece::Text(_) => false,
                    Piece::Stray => true,
                    Piece::Placeholder(name) => {
                        !(r.labels.iter().any(|l| l.name == *name)
                            || (*name == "argument" && r.argument.is_some()))
                    }
                })
            }),
        }
    }
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The findings a list of §4's shape can draw, in §4's order.
const OF_A_LIST: [Invalid; 5] = [
    Invalid::RuleName,
    Invalid::Label,
    Invalid::LabelsWidth,
    Invalid::HostLabel,
    Invalid::Placeholder,
];

/// The table a contract holds (§4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// No resource carries the key.
    None,
    /// The one list its resources carry.
    Table(Vec<Rule>),
    /// An invalid declaration, which holds none.
    Invalid(Invalid),
}

impl Declared {
    /// The table it holds, none where none is declared or the declaration
    /// is invalid.
    #[must_use]
    pub fn table(&self) -> Option<&[Rule]> {
        match self {
            Declared::Table(t) => Some(t),
            Declared::None | Declared::Invalid(_) => None,
        }
    }
}

/// [`declaration`], with the place of the value that made it invalid: its
/// index among the values given. Each finding is looked for over the whole
/// of every list carried before the next.
fn declared<'v>(
    uses_v2: bool,
    carried: impl IntoIterator<Item = &'v Value>,
) -> (Declared, Option<usize>) {
    let carried: Vec<&Value> = carried.into_iter().collect();
    if carried.is_empty() {
        return (Declared::None, None);
    }
    if !uses_v2 {
        return (Declared::Invalid(Invalid::NotUsed), Some(0));
    }
    let mut lists = Vec::with_capacity(carried.len());
    for (i, v) in carried.iter().enumerate() {
        match rules(v) {
            Some(list) => lists.push(list),
            None => return (Declared::Invalid(Invalid::Value), Some(i)),
        }
    }
    for finding in OF_A_LIST {
        if let Some(i) = lists.iter().position(|l| finding.found_in(l)) {
            return (Declared::Invalid(finding), Some(i));
        }
    }
    if let Some(i) = lists.iter().position(|l| *l != lists[0]) {
        return (Declared::Invalid(Invalid::Disagrees), Some(i));
    }
    (Declared::Table(lists.swap_remove(0)), None)
}

/// A contract's table (§4), from whether its `uses` lists `alarms.v2` and
/// the value of `alarms.rules` on each resource that carries it. A
/// resource that does not carry the key takes no part.
#[must_use]
pub fn declaration<'v>(uses_v2: bool, carried: impl IntoIterator<Item = &'v Value>) -> Declared {
    declared(uses_v2, carried).0
}

/// [`declaration`] over a loaded contract's resolved annotations, where
/// the canonical form carries a key written on `[defaults]` (core §9.5).
#[must_use]
pub fn declaration_of(c: &Contract) -> Declared {
    declaration(
        uses_v2(c),
        c.resources.iter().filter_map(|r| r.annotations.get(RULES)),
    )
}

/// The table a contract holds, an invalid declaration holding none.
#[must_use]
pub fn table_of(c: &Contract) -> Option<Vec<Rule>> {
    declaration_of(c).table().map(<[Rule]>::to_vec)
}

/// An instance's table (§4): the one table among the contracts it
/// implements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instance {
    /// No named revision in hand holds one: its owner writes every alarm
    /// by words.
    None,
    /// The one list they hold.
    Table(Vec<Rule>),
    /// Two hold two different lists: no table, and its owner writes every
    /// alarm by words.
    Differ,
}

impl Instance {
    /// The table, where the instance has one.
    #[must_use]
    pub fn table(&self) -> Option<&[Rule]> {
        match self {
            Instance::Table(t) => Some(t),
            Instance::None | Instance::Differ => None,
        }
    }
}

/// An instance's table, from the table of each revision its descriptor
/// names that the reader holds (`None`: that revision holds none, or its
/// declaration is invalid). Never a revision the descriptor does not
/// name: a table read under the wrong revision gives a wrong name over a
/// right state and severity.
#[must_use]
pub fn instance_table<'a>(named: impl IntoIterator<Item = Option<&'a [Rule]>>) -> Instance {
    let mut held: Option<&[Rule]> = None;
    for table in named.into_iter().flatten() {
        match held {
            None => held = Some(table),
            Some(first) if first == table => {}
            Some(_) => return Instance::Differ,
        }
    }
    held.map_or(Instance::None, |t| Instance::Table(t.to_vec()))
}

/// How a table moved from one revision to the next (§4): a table grows by
/// appending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Growth {
    /// The same list.
    Same,
    /// Every rule of the old list is at its index in the new, with its
    /// name and its labels: every index and every code keeps its words.
    /// Rules may follow, and a summary, a category or an argument may
    /// differ.
    Kept,
    /// Some index names another rule, other labels, or none.
    Moved,
}

impl Growth {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Kept => "kept",
            Self::Moved => "moved",
        }
    }
}

/// [`Growth`] of a table from `old` to `new`.
#[must_use]
pub fn growth(old: &[Rule], new: &[Rule]) -> Growth {
    if old == new {
        Growth::Same
    } else if old.len() <= new.len()
        && old
            .iter()
            .zip(new)
            .all(|(o, n)| o.name == n.name && o.labels == n.labels)
    {
        Growth::Kept
    } else {
        Growth::Moved
    }
}

/// One finding of [`lint_contract`]: [`crate::words`]'s shape, which
/// `desired.v2` and `health.v2` share.
pub use crate::words::Lint;

/// What a linter reports of a contract that lists `alarms.v2` (§4): its
/// table's finding, at the first resource whose list draws it, or the
/// first whose list differs from the one before it. None for any other
/// contract: `alarms.v1` defines no key, and a key of a profile `uses`
/// does not list is core E020's.
#[must_use]
pub fn lint_contract(c: &Contract) -> Vec<Lint> {
    if !uses_v2(c) {
        return Vec::new();
    }
    let carrying: Vec<_> = c
        .resources
        .iter()
        .filter(|r| r.annotations.contains_key(RULES))
        .collect();
    let values = carrying.iter().map(|r| &r.annotations[RULES]);
    match declared(true, values) {
        (Declared::Invalid(reason), Some(i)) => vec![Lint {
            profile: PROFILE,
            at: Lint::place(carrying[i]),
            key: Some(RULES),
            token: reason.as_str(),
            says: reason.says(),
        }],
        _ => Vec::new(),
    }
}

// --- §2.4, §4: an alarm's words, and a record's fields ---------------------------

/// The fields of a record that name its alarm (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fields {
    pub rule: RuleIndex,
    pub labels: Option<u32>,
    pub argument: Argument,
    pub key: Option<u64>,
}

impl Fields {
    /// Core §7.6's JSON reading of these fields, as [`Record::to_json`]
    /// has them.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("rule".into(), self.rule.to_json());
        if let Some(l) = self.labels {
            m.insert("labels".into(), json!(l));
        }
        m.insert("argument".into(), self.argument.to_json());
        if let Some(k) = self.key {
            m.insert("key".into(), json!(k));
        }
        Value::Object(m)
    }
}

/// The fields an owner writes for an alarm (§2.4, §4), from its rule's
/// name, its labels as text (host-scoped ones included), its reading and
/// its instance's `table`, none where it has none or two revisions differ
/// ([`Instance::table`]).
///
/// **By the table** where the table names the alarm: it holds the rule's
/// name at an index a record codes, the alarm's discriminating labels are
/// exactly the labels that rule declares, and each one's text has a code.
/// Host-scoped labels are not written. **By words** for any other alarm:
/// `rule` and `argument` `not_available`, no `labels`, and `key` the alert
/// key, v1's recipe through its sanitizing step.
#[must_use]
pub fn write_alarm<'a>(
    table: Option<&[Rule]>,
    rule: &str,
    labels: impl IntoIterator<Item = (&'a str, &'a str)>,
    reading: Option<&Decimal>,
) -> Fields {
    let labels: Vec<(&str, &str)> = labels.into_iter().collect();
    let by_table = table.and_then(|t| {
        let RuleIndex::Table(i) = RuleIndex::of(words::write(t, rule, MAX_INDEX)) else {
            return None;
        };
        let r = &t[usize::from(i)];
        Some(Fields {
            rule: RuleIndex::Table(i),
            labels: r.encode(labels.iter().copied())?,
            argument: r.argument(reading),
            key: None,
        })
    });
    by_table.unwrap_or_else(|| Fields {
        rule: RuleIndex::NotAvailable,
        labels: None,
        argument: Argument::NotAvailable,
        key: Some(super::derive(rule, labels).integer()),
    })
}

/// Why a reader cannot turn a record's numbers into words (§2.4), the
/// first that applies. The alarm's state, its severity and its time are
/// read all the same. Its identity is not: the reader has no alert key
/// for it, and shows its numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Unnamed {
    /// The reader holds no table of the alarm's owner (§4).
    NoTable,
    /// The table holds no rule at the record's index.
    Rule,
    /// `labels` is present where the rule declares no label, absent where
    /// it declares one, or holds a code a label does not have.
    Labels,
}

impl Unnamed {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoTable => "no_table",
            Self::Rule => "rule",
            Self::Labels => "labels",
        }
    }
}

/// A table's alarm, named (§2.4, §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named<'t> {
    /// The rule's index, and the rule.
    pub index: u8,
    pub rule: &'t Rule,
    /// Each label's name and its text, in the rule's order.
    pub labels: Vec<(String, String)>,
    /// The alert key v1's recipe derives from the rule's name and those
    /// labels.
    pub alert_key: u64,
    /// The rule's template with each placeholder replaced.
    pub summary: String,
    /// The argument's reading, where the rule declares one and the record
    /// carries a code.
    pub argument: Option<Decimal>,
}

/// What a reader reads of a record's numbers, with the table it holds of
/// the record's owner (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Naming<'t> {
    /// A table's rule, in words.
    Named(Named<'t>),
    /// A table's rule the reader cannot name. Unnamed is not undecodable.
    Unnamed(Unnamed),
    /// An alarm by words: the record carries its alert key, whatever
    /// table is held, and its words are on `words/<slot>`.
    ByWords { alert_key: u64 },
}

impl Naming<'_> {
    /// The alert key the reader has for the alarm: a named alarm's, `None`
    /// for an unnamed one.
    #[must_use]
    pub fn alert_key(&self) -> Option<u64> {
        match self {
            Naming::Named(n) => Some(n.alert_key),
            Naming::ByWords { alert_key } => Some(*alert_key),
            Naming::Unnamed(_) => None,
        }
    }
}

impl Record {
    /// Reads the record's numbers into words (§2.4) with the instance's
    /// `table` as the reader holds it, none where it holds none or two
    /// revisions differ ([`Instance::table`]): never a table of a revision
    /// the owner's descriptor does not name.
    ///
    /// The record is one [`Record::read`] gave, so under `not_available`
    /// it carries its key.
    #[must_use]
    pub fn name<'t>(&self, table: Option<&'t [Rule]>) -> Naming<'t> {
        let index = match (self.rule, self.key) {
            (RuleIndex::NotAvailable, Some(alert_key)) => return Naming::ByWords { alert_key },
            // No index, and no key: a record no reader reads.
            (RuleIndex::NotAvailable, None) => return Naming::Unnamed(Unnamed::Rule),
            (RuleIndex::Table(i), _) => i,
        };
        let Some(table) = table else {
            return Naming::Unnamed(Unnamed::NoTable);
        };
        let Some(rule) = table.get(usize::from(index)) else {
            return Naming::Unnamed(Unnamed::Rule);
        };
        let Some(labels) = rule.decode(self.labels) else {
            return Naming::Unnamed(Unnamed::Labels);
        };
        let alert_key = super::derive(
            &rule.name,
            labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
        )
        .integer();
        Naming::Named(Named {
            index,
            rule,
            summary: rule.summary(&labels, self.argument),
            argument: rule.reading(self.argument),
            labels,
            alert_key,
        })
    }
}

// --- §2.2, §2.8, §2.9: which slot -------------------------------------------------

/// What a raise does (§2.2, §2.5, §2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Raise {
    /// The record is put firing on this slot: the one its instance holds a
    /// record of the identity on, else the lowest on which it holds no
    /// firing record.
    Put(u8),
    /// A duplicate observation of a firing alarm: no act.
    Firing,
    /// Every slot below the bound fires: the raise is refused, nothing is
    /// put, and no alarm is resolved or put over for room.
    Refused,
}

/// What a resolve does (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Resolve {
    /// The record is put resolved on its slot, where it stays: no delete.
    Put(u8),
    /// The identity does not fire: no act.
    NotFiring,
}

/// One entry of the record an owner's previous instance left (§2.9), as
/// its new instance reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left<I> {
    pub id: I,
    pub state: State,
    /// Whether the entry reads.
    pub reads: bool,
    /// Whether its words derive its alert key.
    pub derives: bool,
    /// Whether the new instance still evaluates its rule.
    pub evaluated: bool,
}

/// What a new instance does with one slot of that record (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Adoption {
    /// At or above the new bound: neither put nor resolved.
    Left,
    /// Resolved, or it does not read, or its words do not derive its key:
    /// nothing is put, and the slot is free.
    Free,
    /// Firing, under a rule the owner no longer evaluates: put resolved,
    /// on its slot.
    Resolved,
    /// Firing otherwise: put again on its slot, its `at` kept, under the
    /// new run.
    Adopted,
}

impl Adoption {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Free => "free",
            Self::Resolved => "resolved",
            Self::Adopted => "adopted",
        }
    }
}

/// The records one instance holds on its slots (§2.2), by identity: an
/// alert key, or anything that stands for one. An instance holds a record
/// on a slot from its put of it, until it puts another identity's there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slots<I> {
    bound: u16,
    held: BTreeMap<u8, (I, State)>,
}

impl<I: Clone + Eq> Slots<I> {
    /// An instance that holds nothing, under `bound`: 256, or the
    /// cardinality its descriptor states (§2.8).
    #[must_use]
    pub fn new(bound: u16) -> Self {
        Slots {
            bound: bound.min(MAX_SLOTS),
            held: BTreeMap::new(),
        }
    }

    /// The instance's bound.
    #[must_use]
    pub fn bound(&self) -> u16 {
        self.bound
    }

    /// The records held, by slot.
    pub fn held(&self) -> impl Iterator<Item = (u8, &I, State)> {
        self.held
            .iter()
            .map(|(slot, (id, state))| (*slot, id, *state))
    }

    /// The slot the instance holds a record of `id` on, firing or
    /// resolved.
    #[must_use]
    pub fn slot_of(&self, id: &I) -> Option<(u8, State)> {
        self.held
            .iter()
            .find(|(_, (held, _))| held == id)
            .map(|(slot, (_, state))| (*slot, *state))
    }

    /// The lowest slot below the bound on which the instance holds no
    /// firing record: one it holds nothing on, or one whose record is
    /// resolved.
    fn lowest_free(&self) -> Option<u8> {
        (0..self.bound)
            .filter_map(|s| u8::try_from(s).ok())
            .find(|s| !matches!(self.held.get(s), Some((_, State::Firing))))
    }

    /// The condition of `id` is found (§2.2): the slot its record is put
    /// firing on, and the instance holds it there.
    pub fn raise(&mut self, id: &I) -> Raise {
        let slot = match self.slot_of(id) {
            Some((_, State::Firing)) => return Raise::Firing,
            Some((slot, State::Resolved)) => slot,
            None => match self.lowest_free() {
                Some(slot) => slot,
                None => return Raise::Refused,
            },
        };
        self.held.insert(slot, (id.clone(), State::Firing));
        Raise::Put(slot)
    }

    /// The condition of `id` cleared (§2.5): its record is put resolved on
    /// its slot, which is free for another identity at once.
    pub fn resolve(&mut self, id: &I) -> Resolve {
        match self.slot_of(id) {
            Some((slot, State::Firing)) => {
                self.held.insert(slot, (id.clone(), State::Resolved));
                Resolve::Put(slot)
            }
            _ => Resolve::NotFiring,
        }
    }

    /// A new instance under `bound`, with the record its previous instance
    /// left, by slot (§2.9); empty with no source. For each slot, what the
    /// new instance does, and it holds what it put and nothing else.
    ///
    /// An identity the record holds firing on two slots breaks §2.2: the
    /// lowest is adopted, and nothing is put on the other.
    #[must_use]
    pub fn restart(
        left: impl IntoIterator<Item = (u8, Left<I>)>,
        bound: u16,
    ) -> (Self, BTreeMap<u8, Adoption>) {
        let mut slots = Slots::new(bound);
        let left: BTreeMap<u8, Left<I>> = left.into_iter().collect();
        let mut did = BTreeMap::new();
        for (slot, entry) in left {
            let adoption = if u16::from(slot) >= slots.bound {
                Adoption::Left
            } else if entry.state == State::Resolved
                || !entry.reads
                || !entry.derives
                // One identity, one slot: the lowest was taken.
                || slots.slot_of(&entry.id).is_some()
            {
                Adoption::Free
            } else if !entry.evaluated {
                slots.held.insert(slot, (entry.id, State::Resolved));
                Adoption::Resolved
            } else {
                slots.held.insert(slot, (entry.id, State::Firing));
                Adoption::Adopted
            };
            did.insert(slot, adoption);
        }
        (slots, did)
    }
}

/// One slot's record as a reader holds it of one owner (§2.6): the stamp
/// of the put that carried it, and the alert key the reader has for it,
/// `None` for an unnamed alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Seen {
    pub slot: u8,
    pub stamp: Stamp,
    pub alert_key: Option<u64>,
}

/// Which slot a reader takes for each identity (§2.6, "one identity on two
/// slots").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Taken {
    /// For each alert key, the slot the reader takes it on: the one under
    /// the run its owner's present descriptor states, the later by seq of
    /// two under it, and `None` where none is under it, each then an
    /// earlier instance's last word.
    pub takes: BTreeMap<u64, Option<u8>>,
    /// The alert keys two slots of the present run name: the break of
    /// §2.2 a tool may report (§5).
    pub double: Vec<u64>,
}

/// [`Taken`] over what a reader holds of one owner, `present` being the
/// run that owner's present descriptor states (core §3.3). Unnamed alarms
/// are not compared: two slots with no alert key are two alarms.
#[must_use]
pub fn taken(seen: &[Seen], present: Run) -> Taken {
    let mut out = Taken::default();
    let mut best: BTreeMap<u64, Seq> = BTreeMap::new();
    for s in seen {
        let Some(key) = s.alert_key else { continue };
        let take = out.takes.entry(key).or_insert(None);
        if s.stamp.run != present {
            continue;
        }
        match best.get(&key) {
            Some(seq) => {
                if !out.double.contains(&key) {
                    out.double.push(key);
                }
                if s.stamp.seq.get() > seq.get() {
                    best.insert(key, s.stamp.seq);
                    *take = Some(s.slot);
                }
            }
            None => {
                best.insert(key, s.stamp.seq);
                *take = Some(s.slot);
            }
        }
    }
    out.double.sort_unstable();
    out
}

/// §5's fourth question, "does this owner hold each identity on one
/// slot?" (§2.2): unobservable while its alarms are not established
/// ([`owner_set`]), else no where two slots of its present run name one
/// alert key ([`Taken::double`]), a finding about the owner.
#[must_use]
pub fn one_slot_each(set: SetVerdict, taken: &Taken) -> Answer {
    match set {
        SetVerdict::Established if taken.double.is_empty() => Answer::Yes,
        SetVerdict::Established => Answer::No,
        SetVerdict::Unobservable | SetVerdict::NotAsked => Answer::Unobservable,
    }
}

// --- §2.4: words on `words/<slot>` -------------------------------------------------

/// The words of an alarm no table names, `alarms.v2.Words` on
/// `words/<slot>` (§2.4): v1's document without `timestamp`,
/// `observed_at_ms`, `severity` and `state`. Members the schema does not
/// name are ignored when read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Words {
    pub rule: String,
    /// Every label, host-scoped ones included.
    pub labels: BTreeMap<String, String>,
    pub summary: String,
    pub category: Option<String>,
}

/// Why a payload is no words a reader can read (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordsInvalid {
    /// Not strict JSON, as v1 §2.4 reads a payload, with why.
    Json(String),
    /// Outside the `Words` schema, with where and why.
    Schema(String),
}

impl std::fmt::Display for WordsInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(why) => write!(f, "not strict JSON: {why}"),
            Self::Schema(why) => write!(f, "outside alarms.v2.Words: {why}"),
        }
    }
}

impl Words {
    /// Reads a decoded payload: the schema's members and their types.
    ///
    /// # Errors
    /// [`WordsInvalid::Schema`].
    pub fn from_json(v: &Value) -> Result<Words, WordsInvalid> {
        let schema = |why: String| WordsInvalid::Schema(why);
        let Value::Object(o) = v else {
            return Err(schema("not an object".into()));
        };
        let string = |m: &str, required: bool| -> Result<Option<String>, WordsInvalid> {
            match o.get(m) {
                None if required => Err(WordsInvalid::Schema(format!("/{m}: missing"))),
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(WordsInvalid::Schema(format!("/{m}: not a string"))),
            }
        };
        let rule = string("rule", true)?.unwrap_or_default();
        let summary = string("summary", true)?.unwrap_or_default();
        let category = string("category", false)?;
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
        Ok(Words {
            rule,
            labels,
            summary,
            category,
        })
    }

    /// Reads a payload's bytes (§2.4): strict JSON first, as v1 §2.4 reads
    /// a payload ([`super::decode`]), then [`Words::from_json`].
    ///
    /// # Errors
    /// [`WordsInvalid`].
    pub fn decode(bytes: &[u8]) -> Result<Words, WordsInvalid> {
        let text = std::str::from_utf8(bytes)
            .map_err(|e| WordsInvalid::Json(format!("not UTF-8: {e}")))?;
        let v = crate::strict::parse_json(text).map_err(|e| WordsInvalid::Json(e.to_string()))?;
        Words::from_json(&v)
    }

    /// The document, as an owner puts it.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("rule".into(), json!(self.rule));
        if !self.labels.is_empty() {
            m.insert("labels".into(), json!(self.labels));
        }
        m.insert("summary".into(), json!(self.summary));
        if let Some(c) = &self.category {
            m.insert("category".into(), json!(c));
        }
        Value::Object(m)
    }

    /// What a reader derives from the words (v1 §2.3): the alert key,
    /// through the sanitizing step, and whether the strict recipe gives
    /// it.
    #[must_use]
    pub fn derive(&self) -> super::Derived {
        super::derive(
            &self.rule,
            self.labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
        )
    }
}

/// How a reader takes a slot's words as its alarm's (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum How {
    /// v1's recipe gives the record's `key` from the words as they are.
    Strict,
    /// Only its sanitizing step does, which a tool MAY report.
    Sanitized,
}

impl How {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Sanitized => "sanitized",
        }
    }
}

/// Why a reader takes no words for a slot's alarm (§2.4), the first that
/// applies. An alarm by words is then shown by its alert key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotTaken {
    /// It holds no record on the slot.
    NoRecord,
    /// The record does not read.
    Undecodable,
    /// The record names a table's rule: the slot's words are an earlier
    /// alarm's.
    Table,
    /// It read no words.
    NoWords,
    /// The words do not read.
    Unreadable,
    /// The recipe gives another key: the words are another alarm's, or
    /// none's.
    Mismatch,
}

impl NotTaken {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.reason().as_str()
    }

    /// The reason §5's third question gives.
    #[must_use]
    pub fn reason(self) -> Reason {
        match self {
            Self::NoRecord => Reason::NoRecord,
            Self::Undecodable => Reason::Undecodable,
            Self::Table => Reason::Table,
            Self::NoWords => Reason::NoWords,
            Self::Unreadable => Reason::Unreadable,
            Self::Mismatch => Reason::Mismatch,
        }
    }
}

/// Whether a reader takes a slot's words as the words of the slot's alarm
/// (§2.4), from what it holds on the slot: its record (`None` for none,
/// `Err` for one that does not read) and its words (the same). It takes
/// them while the record reads with a `rule` of `not_available`, the words
/// read, and v1's recipe over them, through the sanitizing step, gives the
/// record's `key`.
///
/// # Errors
/// [`NotTaken`], the first that applies.
pub fn words_taken<E, F>(
    record: Option<&Result<Record, E>>,
    words: Option<&Result<Words, F>>,
) -> Result<How, NotTaken> {
    let record = match record {
        None => return Err(NotTaken::NoRecord),
        Some(Err(_)) => return Err(NotTaken::Undecodable),
        Some(Ok(r)) => r,
    };
    let key = match (record.rule, record.key) {
        (RuleIndex::NotAvailable, Some(key)) => key,
        (RuleIndex::NotAvailable, None) => return Err(NotTaken::Undecodable),
        (RuleIndex::Table(_), _) => return Err(NotTaken::Table),
    };
    let words = match words {
        None => return Err(NotTaken::NoWords),
        Some(Err(_)) => return Err(NotTaken::Unreadable),
        Some(Ok(w)) => w,
    };
    let derived = words.derive();
    if derived.integer() != key {
        Err(NotTaken::Mismatch)
    } else if derived.framed {
        Ok(How::Strict)
    } else {
        Ok(How::Sanitized)
    }
}

/// §5's third question, "do this slot's words derive its record's key?"
/// (§2.4), over [`words_taken`]: yes, `strict` or `sanitized`; no,
/// `mismatch`, a finding about the owner only while it lasts, since words
/// are put before their record; not asked for `table`, `no_words` and
/// `no_record`; unobservable for `unreadable` and `undecodable`.
#[must_use]
pub fn words_derive(taken: Result<How, NotTaken>) -> Judged {
    match taken {
        Ok(How::Strict) => Judged::new(Answer::Yes, Reason::Strict),
        Ok(How::Sanitized) => Judged::new(Answer::Yes, Reason::Sanitized),
        Err(why) => Judged::new(
            match why {
                NotTaken::Mismatch => Answer::No,
                NotTaken::Table | NotTaken::NoWords | NotTaken::NoRecord => Answer::NotAsked,
                NotTaken::Unreadable | NotTaken::Undecodable => Answer::Unobservable,
            },
            why.reason(),
        ),
    }
}

// --- §2.6: transitions, by identity ---------------------------------------------

/// The record a reader holds on a slot (§2.6): with the run of the put
/// that carried it, its envelope's or its snapshot entry's, and the alert
/// key the reader has for it ([`Naming::alert_key`]), `None` for an
/// unnamed alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Held {
    pub record: Record,
    pub run: Run,
    pub alert_key: Option<u64>,
}

impl Held {
    /// Whether two records of one slot have one identity (§2.6): each has
    /// an alert key, and the two are equal; or either is unnamed, and
    /// their `rule` and their `labels` are equal, an absent `labels` equal
    /// only to an absent one.
    #[must_use]
    pub fn same_identity(&self, other: &Held) -> bool {
        match (self.alert_key, other.alert_key) {
            (Some(a), Some(b)) => a == b,
            _ => self.record.rule == other.record.rule && self.record.labels == other.record.labels,
        }
    }

    fn facts(&self) -> PutFacts<Option<Severity>, Option<i64>> {
        PutFacts {
            state: self.record.state,
            severity: self.record.severity,
            at: self.record.at,
        }
    }
}

/// One sample of one slot's key, in the owner's order (core §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sample {
    /// A put whose record reads.
    Put(Held),
    /// A put that does not read (§2.4).
    Unreadable,
    /// A delete, or a tombstone. No rule here has an owner send one.
    Delete,
}

/// What becomes of a held record of another identity when a put takes its
/// slot (§2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Displaced {
    /// It fired, under the put's run. An owner never takes a slot whose
    /// record fires, so the held alarm resolved, and the reader lost that
    /// put: it reads resolved.
    Rebound,
    /// It was resolved, under the put's run: nothing to read.
    Replaced,
    /// It was under another run: an earlier run's last word, of which
    /// nothing is known.
    Dropped,
}

impl Displaced {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rebound => "rebound",
            Self::Replaced => "replaced",
            Self::Dropped => "dropped",
        }
    }
}

/// A change a notifier reports (§2.16), where every answer of §2.14
/// follows the slot's records alone: its owner present and listed,
/// nothing lost, each firing record fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Change {
    /// The alarm's answer moved to yes, or its severity moved while yes.
    Firing,
    /// It moved from yes to no: a resolved record, a delete or `rebound`.
    Resolved,
}

impl Change {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Firing => "firing",
            Self::Resolved => "resolved",
        }
    }
}

/// One [`Change`], of the alarm with this alert key; `None` for an
/// unnamed alarm, which a tool reports by its owner, its slot and its
/// `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Report {
    pub change: Change,
    pub alert_key: Option<u64>,
}

/// What a reader reads of one sample of a slot (§2.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// A held record of another identity, and what became of it. `None`
    /// where nothing was held, or the same identity was.
    pub displaced: Option<(Displaced, Held)>,
    /// The sample's own reading, by v1's table.
    pub reads: Transition,
    /// What a notifier reports after the sample (§2.16): a `rebound`
    /// alarm resolved first, then the sample's own change. A record
    /// `replaced` or `dropped`, and an unreadable put, report nothing of
    /// the alarm held before.
    pub reports: Vec<Report>,
}

/// One slot's record as a reader follows it (§2.6): what it holds, and
/// what each sample reads as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Track {
    /// The record held: `None` before any, after a delete, and after a put
    /// that did not read.
    pub held: Option<Held>,
}

impl Track {
    /// Reads one sample, the next in the owner's order, against the
    /// record held on the slot.
    pub fn apply(&mut self, sample: Sample) -> Applied {
        let held = self.held.take();
        let was_firing = |h: &Held| h.record.state == State::Firing;
        let mut reports = Vec::new();
        let new = match sample {
            Sample::Unreadable => {
                return Applied {
                    displaced: None,
                    reads: Transition::Unreadable,
                    reports,
                };
            }
            Sample::Delete => {
                let reads = super::delete_reads(held.map(|h| h.record.state));
                if let Some(h) = held.filter(was_firing) {
                    reports.push(Report {
                        change: Change::Resolved,
                        alert_key: h.alert_key,
                    });
                }
                return Applied {
                    displaced: None,
                    reads,
                    reports,
                };
            }
            Sample::Put(new) => new,
        };
        self.held = Some(new);
        let (same, displaced) = match held {
            Some(h) if h.same_identity(&new) => (Some(h), None),
            Some(h) => {
                let what = if h.run != new.run {
                    Displaced::Dropped
                } else if was_firing(&h) {
                    Displaced::Rebound
                } else {
                    Displaced::Replaced
                };
                (None, Some((what, h)))
            }
            None => (None, None),
        };
        if let Some((Displaced::Rebound, h)) = displaced {
            reports.push(Report {
                change: Change::Resolved,
                alert_key: h.alert_key,
            });
        }
        let reads = super::put_reads(
            same.as_ref().map(Held::facts),
            new.facts(),
            same.is_some_and(|h| h.record == new.record),
        );
        let fired = same.as_ref().is_some_and(was_firing);
        let change = match reads {
            // A raise of an alarm already firing at its severity, whose
            // `at` moved with no resolve heard, is not told again.
            Transition::Raised if !fired => Some(Change::Firing),
            Transition::Escalated => Some(Change::Firing),
            Transition::Resolved if fired => Some(Change::Resolved),
            _ => None,
        };
        reports.extend(change.map(|change| Report {
            change,
            alert_key: new.alert_key,
        }));
        Applied {
            displaced,
            reads,
            reports,
        }
    }
}

// --- §3, core §7.5: a late joiner's snapshot -------------------------------------

/// Which of the standard contract's two collections a snapshot is of
/// (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Collection {
    /// `state/alarms/{slot}`: one entry per slot that has a value, its id
    /// the slot, on `…/state/alarms`, collection 0.
    Alarms,
    /// `state/words/{slot}`: the same on `…/state/words`, collection 1.
    /// Never exposed across a face (§2.11).
    Words,
}

impl Collection {
    /// The collection's index in a snapshot ([`ALARMS_COLL`],
    /// [`WORDS_COLL`]).
    #[must_use]
    pub const fn coll(self) -> u32 {
        match self {
            Collection::Alarms => ALARMS_COLL,
            Collection::Words => WORDS_COLL,
        }
    }

    /// The key a reply's snapshot of this collection is on, below the
    /// owner: the template's fixed chunks (core §4.2).
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Collection::Alarms => "alarms.v2/state/alarms",
            Collection::Words => "alarms.v2/state/words",
        }
    }

    /// The collection a page read on `key` is of, where its `coll` is that
    /// collection's: `None` for a key that is neither's, and for an index
    /// the standard contract does not give that key.
    #[must_use]
    pub fn of(key: &str, coll: u32) -> Option<Collection> {
        [Collection::Alarms, Collection::Words]
            .into_iter()
            .find(|c| {
                key.strip_suffix(c.suffix())
                    .is_some_and(|rest| rest.is_empty() || rest.ends_with('/'))
                    && c.coll() == coll
            })
    }
}

/// One snapshot as a late joiner holds it (core §7.5): its header, and
/// whether it holds every page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard {
    /// A page of each index up to one whose more bit is 0.
    pub complete: bool,
    /// CH's tag, under which each entry's record is read.
    pub tag: Tag,
    pub run: Run,
    pub asof: Seq,
    /// The snapshot's time, on the owner's clock.
    pub time: collection::Time,
}

/// One live entry of a snapshot, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<R> {
    /// The entry's id, and the slot it names: `None` for an id that is no
    /// slot, from which a reader reads nothing (§2.2).
    pub id: String,
    pub slot: Option<u8>,
    /// Its put's stamp: the entry's own run where it states one, else the
    /// snapshot's. A record is held with this run (§2.6).
    pub stamp: Stamp,
    pub confirmation: Confirmation,
    /// When the owner last vouched for it (`freshness.v2` §2.4): the
    /// snapshot's time for confirmation 1, its `conf` for 2, none for 0
    /// and for a time that is unknown. A GET reader ages a firing entry
    /// from it (§2.15).
    pub confirmed_at: Option<collection::Time>,
    /// Its record, read under the page's tag, or why it does not read; for
    /// the words, the document.
    pub record: R,
}

/// One collection's snapshot, read: every live entry in the order read,
/// and each tombstone within the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot<R> {
    pub heard: Heard,
    pub live: Vec<Entry<R>>,
    /// Each tombstone: its id and its delete's stamp.
    pub retired: Vec<(String, Stamp)>,
}

/// A late joiner's reading of one owner's answer to a collection read of
/// its `alarms.v2/state/**` (§2.15, core §4.2): the alarms' snapshot and
/// the words', each none where no page of it was read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seed {
    pub alarms: Option<Snapshot<Result<Record, Undecodable>>>,
    pub words: Option<Snapshot<Result<Words, WordsInvalid>>>,
    /// A page of a second snapshot of one collection was read, under
    /// another header than its first: left out, and the alarms are then
    /// not read whole ([`Seed::snapshot`]).
    pub mixed: bool,
}

/// One collection's pages as they arrive: the first snapshot heard, and
/// its pages in the order read.
struct Pages {
    heard: collection::Heard,
    pages: Vec<Page>,
}

fn stamp_of(run: Option<Run>, seq: Seq, header: Run) -> Stamp {
    Stamp {
        run: run.unwrap_or(header),
        seq,
    }
}

fn snapshot_of<R>(p: &Pages, read: impl Fn(&Page, &[u8]) -> R) -> Snapshot<R> {
    let entry = |e: &collection::Entry, page: &Page| {
        let confirmed_at = match e.confirmation {
            Confirmation::None => None,
            Confirmation::AtTime => Some(page.time),
            Confirmation::At(t) => Some(t),
        }
        .filter(|t| *t != collection::Time::Unknown);
        Entry {
            id: e.id.clone(),
            slot: parse_slot(&e.id),
            stamp: stamp_of(e.run, e.seq, page.run),
            confirmation: e.confirmation,
            confirmed_at,
            record: read(page, &e.record),
        }
    };
    Snapshot {
        heard: Heard {
            complete: p.heard.complete(),
            tag: p.heard.tag,
            run: p.heard.run,
            asof: p.heard.asof,
            time: p.heard.time,
        },
        live: p
            .pages
            .iter()
            .flat_map(|page| page.live.iter().map(move |e| (e, page)))
            .map(|(e, page)| entry(e, page))
            .collect(),
        retired: p
            .pages
            .iter()
            .flat_map(|page| {
                page.tombs
                    .iter()
                    .map(|t| (t.id.clone(), stamp_of(t.run, t.seq, page.run)))
            })
            .collect(),
    }
}

impl Seed {
    /// Reads a GET's replies, each its key and its payload, in the order
    /// they arrived, under the deployment epoch `epoch` in seconds. A
    /// reply that is no page core §7.5 accepts, and a page that is of
    /// neither collection of the standard contract ([`Collection::of`]),
    /// takes no part.
    #[must_use]
    pub fn of<'a>(replies: impl IntoIterator<Item = (&'a str, &'a [u8])>, epoch: i64) -> Seed {
        let mut held: [Option<Pages>; 2] = [None, None];
        let mut mixed = false;
        for (key, payload) in replies {
            let Ok(page) = Page::parse(payload) else {
                continue;
            };
            let Some(c) = Collection::of(key, page.coll) else {
                continue;
            };
            let slot = &mut held[c as usize];
            match slot {
                Some(p) if p.heard.holds(key, &page) => {
                    if p.heard.add(&page) {
                        p.pages.push(page);
                    }
                }
                Some(_) => mixed = true,
                None => {
                    let mut heard = collection::Heard::of(key, &page);
                    heard.add(&page);
                    *slot = Some(Pages {
                        heard,
                        pages: vec![page],
                    });
                }
            }
        }
        let [alarms, words] = held;
        Seed {
            alarms: alarms
                .map(|p| snapshot_of(&p, |page, bytes| Record::read(page.tag.get(), bytes, epoch))),
            words: words.map(|p| snapshot_of(&p, |_, bytes| Words::decode(bytes))),
            mixed,
        }
    }

    /// The alarms' snapshot as §2.15 judges it ([`owner_set`]), `stated`
    /// being the runs the owner's present descriptors state (core §3.3),
    /// empty where none states one and across a face, where the
    /// deployment's word stands for the descriptor and no run is
    /// compared. `None` where no page of it was read. A reading that mixed
    /// two snapshots is not complete.
    #[must_use]
    pub fn snapshot(&self, stated: &[Run]) -> Option<SnapshotRead> {
        let alarms = self.alarms.as_ref()?;
        Some(SnapshotRead {
            complete: alarms.heard.complete && !self.mixed,
            run: SnapshotRun::of(alarms.heard.run, stated),
        })
    }
}

// --- §2.14, §2.15, §5: what a tool may conclude -----------------------------------

/// The latest sample a reader holds of one alarm, as §2.6 reads it
/// (§2.14's step 5). The state is the record's, never the sample's kind, a
/// heartbeat's bit or a count (§2.6, §2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Latest {
    /// A firing record.
    Firing,
    /// A resolved record: answered at step 5, whatever its age.
    Resolved,
    /// Its record was replaced on its slot by another identity's, under
    /// its run, while it fired ([`Displaced::Rebound`]).
    Rebound,
    /// A delete, or a tombstone.
    Deleted,
    /// A put that does not read (§2.4).
    Undecodable,
}

impl Latest {
    /// The latest sample a held record is, or one that does not read.
    #[must_use]
    pub fn of<E>(record: &Result<Record, E>) -> Latest {
        match record {
            Ok(r) if r.state == State::Firing => Latest::Firing,
            Ok(_) => Latest::Resolved,
            Err(_) => Latest::Undecodable,
        }
    }
}

/// One reading of one alarm (§2.14): an alarm the reader holds on a slot,
/// or one asked by reference, which is the one whose record derives that
/// alert key. An unnamed alarm is answered as any other, by its slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    /// [`Presence::Present`] with [`super::Listing::Listed`] for a
    /// descriptor whose `interfaces` holds `alarms.v2`
    /// ([`super::Major::of`]), `NotListed` for one that holds neither
    /// major. One that holds `alarms.v1` alone is [`super::firing`]'s.
    pub presence: Presence,
    /// `freshness.v2`'s verdict over the slot's observations at 900 s
    /// ([`super::TTL`]); `None` where nothing was observed.
    pub freshness: Option<freshness::Judged>,
    /// The latest sample; `None` where the reader holds nothing of the
    /// alarm: never read, or its record `replaced` or `dropped`.
    pub latest: Option<Latest>,
    /// The reader's subscription lost a delivery, and since then no GET
    /// has answered this slot.
    pub lost: bool,
    /// The latest put is attributed to an earlier instance: a snapshot's
    /// entry under a run no present descriptor states.
    pub previous_instance: bool,
    /// The owner's alarms are established in this reading ([`owner_set`]).
    pub held_whole: bool,
    /// The read that established them listed the alarm's slot neither
    /// live nor as a tombstone, and nothing of it arrived after that read
    /// was asked. With [`Reading::held_whole`] it makes any held sample
    /// none, and heals [`Reading::lost`].
    pub left_out: bool,
}

impl Reading {
    fn asked(&self) -> Asked {
        Asked {
            presence: self.presence,
            freshness: self
                .freshness
                .unwrap_or_else(|| freshness::judge_all(&super::horizon(), &[])),
            lost: self.lost,
            latest: self.latest.map(|l| match l {
                Latest::Firing => Step5::Firing,
                Latest::Undecodable => Step5::Undecodable,
                Latest::Resolved => Step5::Not(Reason::Resolved),
                Latest::Rebound => Step5::Not(Reason::Rebound),
                Latest::Deleted => Step5::Not(Reason::Deleted),
            }),
            previous_instance: self.previous_instance,
            held_whole: self.held_whole,
            left_out: self.left_out,
        }
    }
}

/// §5's first question, "is this alarm firing?", by v1 §2.14's steps over
/// this major's records (§2.14): presence, the descriptor, an archive
/// alone, a loss, the latest sample, `rebound` among them, then its
/// freshness at 900 s. Yes is the finding.
#[must_use]
pub fn firing(r: &Reading) -> Judged {
    super::firing_asked(&r.asked())
}

/// §5's last question, "is this firing alarm past its horizon, its owner
/// present?", over the reading [`firing`] takes, with `rebound` beside a
/// resolved record: not asked, `not_firing`.
#[must_use]
pub fn stale_firing(r: &Reading) -> Judged {
    super::stale_firing_asked(&r.asked())
}

/// A snapshot's run against the runs the owner's present descriptors
/// state (§2.15, core §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotRun {
    /// A present descriptor states it.
    Stated,
    /// One states a run, and none states this one: an earlier instance
    /// built the snapshot.
    Other,
    /// No present descriptor states a run, or the owner is across a face:
    /// nothing to compare.
    Unstated,
}

impl SnapshotRun {
    /// `run` against the runs `stated`, empty where none is.
    #[must_use]
    pub fn of(run: Run, stated: &[Run]) -> SnapshotRun {
        if stated.is_empty() {
            SnapshotRun::Unstated
        } else if stated.contains(&run) {
            SnapshotRun::Stated
        } else {
            SnapshotRun::Other
        }
    }
}

/// The alarms' snapshot a collection read drew (§2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotRead {
    /// Whether the reader holds every page of it (core §7.5), with any
    /// number of entries or none.
    pub complete: bool,
    pub run: SnapshotRun,
}

/// How a reader's latest collection read of an owner's
/// `alarms.v2/state/alarms/**` ended (§2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Read {
    /// At its final reply, with no error reply, with the snapshot it drew:
    /// `None` where it drew no page, which is silence and never "nothing
    /// fires".
    Complete(Option<SnapshotRead>),
    /// At its timeout, or with an error reply.
    Incomplete,
}

/// What a reader's subscription saw since its read was asked (§2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Since {
    /// It lost no delivery.
    pub lossless: bool,
    /// No gap in the owner's presence: its instance token not withdrawn,
    /// and no new instance's appearing. Not asked across a face, where
    /// presence does not cross.
    pub present_throughout: bool,
}

/// Whether an owner's alarms are known whole (§2.15), by the first of its
/// order that answers: v1 §2.14's steps 1 and 2, with their reasons; the
/// read (`no_seed`, `seed_incomplete`); since the read, for a reader that
/// subscribes (`window_lost`, then `owner_gap`, never asked across a
/// face); the snapshot (`snapshot_incomplete`, then `snapshot_run`, never
/// asked across a face nor where no run is stated); otherwise
/// established, `snapshot`. `since` is `None` for a reader that does not
/// subscribe.
///
/// One complete snapshot establishes them, with an entry or with none: a
/// snapshot with no entry is the owner's word that nothing fires. It does
/// not lapse.
#[must_use]
pub fn owner_set(presence: Presence, read: Option<Read>, since: Option<Since>) -> SetJudged {
    let j = |verdict, reason| SetJudged { verdict, reason };
    use SetVerdict::{Established, NotAsked, Unobservable};
    if let Some(p) = super::premises(presence) {
        return j(
            match p.answer {
                Answer::NotAsked => NotAsked,
                _ => Unobservable,
            },
            p.reason,
        );
    }
    let snapshot = match read {
        None => return j(Unobservable, Reason::NoSeed),
        Some(Read::Incomplete) => return j(Unobservable, Reason::SeedIncomplete),
        Some(Read::Complete(snapshot)) => snapshot,
    };
    let across = matches!(presence, Presence::AcrossFace { .. });
    if let Some(s) = since {
        if !s.lossless {
            return j(Unobservable, Reason::WindowLost);
        }
        if !s.present_throughout && !across {
            return j(Unobservable, Reason::OwnerGap);
        }
    }
    match snapshot {
        None
        | Some(SnapshotRead {
            complete: false, ..
        }) => j(Unobservable, Reason::SnapshotIncomplete),
        Some(SnapshotRead {
            run: SnapshotRun::Other,
            ..
        }) if !across => j(Unobservable, Reason::SnapshotRun),
        Some(_) => j(Established, Reason::Snapshot),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<Rule> {
        rules(&json!([
            {"name": "hot", "category": "sensor_health", "summary": "Temperature {argument} C",
             "argument": {"minimum": "-40", "resolution": "0.1"}},
            {"name": "link_down", "summary": "Link to {peer} on {port} is down",
             "labels": [{"name": "peer", "values": ["r1", "r2", "r3"]},
                        {"name": "port", "values": ["eth0", "eth1"]}]},
            {"name": "sensor_lost", "summary": "Sensor {sensor} lost",
             "labels": [{"name": "sensor", "minimum": 0, "maximum": 63}]},
        ]))
        .expect("the text's table")
    }

    fn stamp(run: u16, seq: u32) -> Stamp {
        Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        }
    }

    #[test]
    fn the_layout_is_nine_bytes_for_the_usual_record_and_its_tag_is_eleven() {
        let l = layout();
        assert_eq!((l.tag(), l.min_bits(), l.max_bits()), (TAG, 35, 155));
        let epoch = 1_767_225_600;
        let r = Record {
            state: State::Firing,
            severity: Some(Severity::Critical),
            rule: RuleIndex::Table(3),
            at: Some(1_794_062_759),
            labels: None,
            argument: Argument::NotAvailable,
            key: None,
        };
        let written = r.write(epoch).unwrap();
        assert_eq!(
            written.bytes,
            [0x40, 0x70, 0x19, 0x98, 0x0a, 0x77, 0xff, 0xff, 0x80]
        );
        assert_eq!(Record::read(TAG, &written.bytes, epoch), Ok(r));
        let put = r.put(stamp(3, 5), epoch).unwrap();
        assert_eq!(put[0], 0x0b, "form 0, a 16-bit run, tag 11");
        assert_eq!(put.len(), 15);
        // A JSON payload's tag is 0: refused before a bit is read.
        assert_eq!(
            Record::read(0, &written.bytes, epoch),
            Err(Undecodable::Core(ReadRefusal::Tag))
        );
        // No `at`: 5 bytes.
        let bare = Record { at: None, ..r };
        assert_eq!(bare.write(epoch).unwrap().bytes.len(), 5);
    }

    #[test]
    fn a_key_goes_with_not_available_and_with_nothing_else() {
        let epoch = 0;
        let by_words = Record {
            state: State::Firing,
            severity: Some(Severity::Warning),
            rule: RuleIndex::NotAvailable,
            at: None,
            labels: None,
            argument: Argument::NotAvailable,
            key: Some(0xa659_f813_308a_d1da),
        };
        assert_eq!(by_words.write(epoch).unwrap().bytes.len(), 13);
        let no_key = Record {
            key: None,
            ..by_words
        };
        assert_eq!(no_key.write(epoch), Err(Refused::KeyMissing));
        let both = Record {
            rule: RuleIndex::Table(1),
            ..by_words
        };
        assert_eq!(both.write(epoch), Err(Refused::KeyWithRule));
        let labelled = Record {
            labels: Some(2),
            ..by_words
        };
        assert_eq!(labelled.write(epoch), Err(Refused::ByWordsFields));
        assert_eq!(RuleIndex::new(255), None);
        assert_eq!(Argument::new(1_048_575), None);
    }

    #[test]
    fn an_alarm_is_written_by_its_table_or_by_words_and_read_back() {
        let t = table();
        let f = write_alarm(
            Some(&t),
            "link_down",
            [("port", "eth0"), ("peer", "r2"), ("host", "h-3fa9c2d41b7e")],
            None,
        );
        assert_eq!(
            (f.rule, f.labels, f.key),
            (RuleIndex::Table(1), Some(2), None)
        );
        let record = Record::new(State::Firing, Severity::Warning, f, None);
        let Naming::Named(n) = record.name(Some(&t)) else {
            panic!("named")
        };
        assert_eq!(super::super::key_hex(n.alert_key), "a659f813308ad1da");
        assert_eq!(n.summary, "Link to r2 on eth0 is down");
        // A value no table lists: by words, with v1's alert key.
        let w = write_alarm(
            Some(&t),
            "link_down",
            [("peer", "r4"), ("port", "eth0")],
            None,
        );
        assert_eq!(w.rule, RuleIndex::NotAvailable);
        assert!(w.key.is_some() && w.labels.is_none());
        // No table in hand: numbers, never another revision's name.
        assert_eq!(record.name(None), Naming::Unnamed(Unnamed::NoTable));
        // The argument: a tie to the even code, and `?` for none.
        let hot = &t[0];
        let reading = |s: &str| hot.argument(Decimal::parse_input(s).as_ref());
        assert_eq!(reading("21.55"), Argument::Code(616));
        assert_eq!(reading("-40.2"), Argument::NotAvailable);
        assert_eq!(hot.summary(&[], Argument::Code(615)), "Temperature 21.5 C");
        assert_eq!(hot.summary(&[], Argument::NotAvailable), "Temperature ? C");
    }

    #[test]
    fn a_tables_findings_come_in_the_texts_order() {
        let t = table_json(&table());
        assert_eq!(declaration(true, [&t]), Declared::Table(table()));
        assert_eq!(
            declaration(false, [&t]),
            Declared::Invalid(Invalid::NotUsed)
        );
        let twice = json!([{"name": "a", "summary": "{x}"}, {"name": "a", "summary": "s"}]);
        assert_eq!(
            declaration(true, [&twice]),
            Declared::Invalid(Invalid::RuleName),
            "before the placeholder"
        );
        let other = json!([{"name": "a", "summary": "s"}]);
        assert_eq!(
            declaration(true, [&t, &other]),
            Declared::Invalid(Invalid::Disagrees)
        );
        assert_eq!(
            declaration(true, [&t, &twice]),
            Declared::Invalid(Invalid::RuleName),
            "a finding of any list stands before `disagrees`"
        );
        assert_eq!(growth(&table()[..2], &table()), Growth::Kept);
        assert_eq!(growth(&table(), &table()[..2]), Growth::Moved);
    }

    #[test]
    fn an_identity_takes_its_own_slot_else_the_lowest_that_does_not_fire() {
        let mut s: Slots<&str> = Slots::new(2);
        assert_eq!(s.raise(&"a"), Raise::Put(0));
        assert_eq!(s.raise(&"a"), Raise::Firing);
        assert_eq!(s.raise(&"b"), Raise::Put(1));
        assert_eq!(s.raise(&"c"), Raise::Refused, "never over a firing record");
        assert_eq!(s.resolve(&"a"), Resolve::Put(0));
        assert_eq!(s.resolve(&"a"), Resolve::NotFiring);
        assert_eq!(s.raise(&"a"), Raise::Put(0), "a re-raise keeps its slot");
        assert_eq!(s.resolve(&"b"), Resolve::Put(1));
        assert_eq!(s.raise(&"c"), Raise::Put(1), "a resolved slot retaken");
        assert_eq!(s.slot_of(&"b"), None);
    }

    #[test]
    fn at_moves_a_second_at_a_time_and_an_acknowledgement_follows_it() {
        let epoch = 1_767_225_600;
        let mut d = Dating::default();
        assert_eq!(d.transition(1_794_062_759_100, epoch), Some(1_794_062_759));
        assert_eq!(d.transition(1_794_062_759_200, epoch), Some(1_794_062_760));
        assert_eq!(d.transition(5000, epoch), None, "1970 has no code");
        assert_eq!(d.at(Act::Refresh, None, 1_794_062_800_000, epoch), None);
        assert_eq!(d.transition(1_794_062_759_300, epoch), Some(1_794_062_761));
        assert!(acknowledged(Some(1_794_062_759_123), Some(1_794_062_759)));
        assert!(!acknowledged(Some(1_794_062_759_000), Some(1_794_062_760)));
        assert!(!acknowledged(Some(1_794_062_759_000), None));
        assert!(!acknowledged(None, None));
    }

    #[test]
    fn another_identity_on_a_firing_slot_is_a_resolve_the_reader_lost() {
        let record = |rule, state| Record {
            state,
            severity: Some(Severity::Warning),
            rule: RuleIndex::Table(rule),
            at: Some(1),
            labels: None,
            argument: Argument::NotAvailable,
            key: None,
        };
        let held = |rule, state, run, key| Held {
            record: record(rule, state),
            run: Run::U16(run),
            alert_key: Some(key),
        };
        let mut t = Track::default();
        assert_eq!(
            t.apply(Sample::Put(held(1, State::Firing, 3, 10))).reads,
            Transition::Raised
        );
        let a = t.apply(Sample::Put(held(2, State::Firing, 3, 20)));
        assert_eq!(a.displaced.map(|d| d.0), Some(Displaced::Rebound));
        assert_eq!(
            a.reports.iter().map(|r| r.change).collect::<Vec<_>>(),
            [Change::Resolved, Change::Firing]
        );
        let b = t.apply(Sample::Put(held(1, State::Firing, 4, 10)));
        assert_eq!(b.displaced.map(|d| d.0), Some(Displaced::Dropped));
        assert_eq!(
            b.reports.len(),
            1,
            "an earlier run's last word resolves nothing"
        );
        assert_eq!(t.apply(Sample::Delete).reads, Transition::Resolved);
        assert_eq!(t.held, None);
    }

    #[test]
    fn one_complete_snapshot_establishes_an_owners_alarms_with_no_entry() {
        let listed = Presence::Present(super::super::Listing::Listed { token: false });
        let whole = Read::Complete(Some(SnapshotRead {
            complete: true,
            run: SnapshotRun::Stated,
        }));
        let set = owner_set(listed, Some(whole), None);
        assert_eq!(
            (set.verdict, set.reason),
            (SetVerdict::Established, Reason::Snapshot)
        );
        assert_eq!(
            owner_set(listed, Some(Read::Complete(None)), None).reason,
            Reason::SnapshotIncomplete,
            "no page is silence"
        );
        let r = Reading {
            presence: listed,
            freshness: None,
            latest: Some(Latest::Rebound),
            lost: false,
            previous_instance: false,
            held_whole: true,
            left_out: false,
        };
        assert_eq!(firing(&r), Judged::new(Answer::No, Reason::Rebound));
        assert_eq!(
            stale_firing(&r),
            Judged::new(Answer::NotAsked, Reason::NotFiring)
        );
    }
}
