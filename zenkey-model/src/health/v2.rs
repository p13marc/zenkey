//! `health.v2`: how well a service serves, in packed records
//! (`spec/profiles/health/v2.md`, text 0.1, #986's Q1).
//!
//! The profile's session-free half. It restates `health.v1` by reference,
//! so the levels, a reading's judgement, the roll-up and the agreement of
//! a status with its checks are [`super`]'s, over either major. What this
//! module adds is what the major replaces:
//!
//! ```text
//! Status · Check · Fault   ─▶ the three packed records (§3): write, put, read, each refusal core §7.6's
//! Word, Kind, Tables       ─▶ §2.10: 0 none, 1–31 the profile's, 32–62 the service's own, 63 out_of_range
//! declaration(uses, values)─▶ a contract's table of one kind: none │ a list │ not_used │ value │ disagrees
//! instance_table(named…)   ─▶ the one table among the revisions a descriptor names: none │ a list │ differ
//! write_word · read_word   ─▶ a name ⇄ its number, by the profile's table and the instance's own
//! Reading::of(replies)     ─▶ a late joiner's two snapshots (§2.4), and whether they are one instant (§2.2)
//! clock_ahead(p, heard)    ─▶ §5 "is its clock ahead?", answered by the last read heartbeat's time (§2.5)
//! ```
//!
//! **No I/O, no clock, and no file.** The standard schema is restated
//! here as a declaration ([`RecordType::layout`]), since no crate may read
//! `spec/`;
//! `tests/profiles/health_v2.rs` holds it to
//! `spec/profiles/health/health.v2.json`'s layouts, their forms' hashes and
//! their tags. The owner's puts, its fault at the guard's first hold and
//! the readers on a bus are the runtime's and the fleet's (#986's Q2).

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{Answer, Clock, ClockReason, Level, Listing, Presence, Read};
use crate::collection::{self, Confirmation, Page};
use crate::contract::Contract;
use crate::heartbeat;
use crate::packed::{self, Layout, ReadRefusal, Record, WriteRefusal};
use crate::wire::{Envelope, Run, Seq, Stamp, Tag};
use crate::words::{self, Index};

/// The profile's id, which is also its standard contract's interface id.
pub const PROFILE: &str = "health.v2";

/// The standard contract's interface id (§3).
pub const IFACE: &str = "health.v2";

/// Revision 2.0 of the standard contract (§3), `health.v2.toml`: its
/// fingerprint's hex. Its heartbeat's tag is its last hex digit, 1 (core
/// §7.5).
pub const FINGERPRINT: &str = "b447f613bea7bdc24703500280a20bf0cf20fb27cbb38ecd25f5164b0ae005b1";

/// The standard packed schema's stem (§3): `health.v2.json`.
pub const SCHEMA_STEM: &str = "health.v2";

/// The standard packed schema's artifact ids the profile publishes (§3),
/// oldest first: text 0.1's.
pub const PUBLISHED: &[&str] =
    &["sha256:d270e1487bb6b2688aa34e5cb55804a14edc61a64e5fdd88d167b21ca4eaa983"];

/// Whether `id` is one of the standard schema's published artifact ids.
#[must_use]
pub fn is_published(id: &str) -> bool {
    PUBLISHED.contains(&id)
}

/// The annotation keys' names (§4): what W105 reads for a contract that
/// lists `health.v2` (core §10). `health.v1` publishes none.
pub const VOCABULARY: &[&str] = &["reasons", "details", "codes"];

/// The collection's index of `state/checks/{check}` in a snapshot (§3):
/// the contract's `state` resources in the canonical form's order.
pub const CHECKS_COLL: u32 = 0;

/// The collection's index of `state/status` in a snapshot (§3).
pub const STATUS_COLL: u32 = 1;

/// The class of the status's collection (§2.8, `freshness.v2` §2.4): the
/// standard contract exposes nothing `link`, so an owner heartbeat that
/// covers the status is the `host` key's.
pub const STATUS_CLASS: heartbeat::Class = heartbeat::Class::Host;

/// Whether a contract lists `health.v2` in `uses`: one that may declare
/// the service's own tables (§2.10).
#[must_use]
pub fn uses_v2(c: &Contract) -> bool {
    words::uses(c, PROFILE)
}

// --- §2.10: words -------------------------------------------------------------

/// The profile's reasons (§2.10): the name at index i is the word i + 1.
pub const REASONS: &[&str] = &[
    "starting",
    "serving",
    "stopping",
    "check",
    "upstream_lost",
    "fallback",
    "backlog",
    "misconfigured",
];

/// The profile's details (§2.10): the name at index i is the word i + 1.
pub const DETAILS: &[&str] = &["ok", "unreachable", "timeout", "refused", "cannot_run"];

/// The profile's fault codes (§2.10): the name at index i is the word
/// i + 1. `clock_ahead`'s argument is the offset measured, in whole
/// seconds (§2.5).
pub const CODES: &[&str] = &[super::CLOCK_AHEAD];

/// The reason an owner that has not yet established its level gives
/// (§2.3).
pub const STARTING: &str = "starting";

/// The reason of a status raised to its worst check's level (§2.2).
pub const CHECK: &str = "check";

/// The detail of a check its owner can no longer run (§2.3).
pub const CANNOT_RUN: &str = "cannot_run";

/// The first word of a service's own table (§2.10): the name at index i
/// is the word 32 + i.
pub const OWN_FIRST: u8 = 32;

/// The most names a service's own table holds (§2.10): the words 32 to 62.
pub const OWN_MAX: usize = 31;

/// The first word of the profile's tables (§2.10): the name at index i is
/// the word 1 + i.
const PROFILE_FIRST: u8 = 1;

/// The greatest place either half of the numbers names: 31 words each, 1
/// to 31 and 32 to 62.
const HALF_MAX: u8 = 30;

/// A word (§0, §2.10): a status's `reason`, a check's `detail` or a
/// fault's `code`, a number from 0 to 63 in 6 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Word(u8);

impl Word {
    /// 0: the owner gives no word.
    pub const NONE: Word = Word(0);

    /// 63, the record's `out_of_range`: a word no table names. The words
    /// stay in the owner's log.
    pub const OUT_OF_RANGE: Word = Word(63);

    /// The record's name for [`Word::OUT_OF_RANGE`] (core §7.6's reserved
    /// code).
    pub const OUT_OF_RANGE_NAME: &'static str = words::OUT_OF_RANGE;

    /// The largest number that is a value, 62.
    pub const MAX: u8 = 62;

    /// The word `n`, from 0 to 62; `None` above, 63 being no value but
    /// [`Word::OUT_OF_RANGE`]'s code.
    #[must_use]
    pub const fn new(n: u8) -> Option<Word> {
        if n <= Word::MAX { Some(Word(n)) } else { None }
    }

    /// Its 6-bit code, 63 for `out_of_range`.
    #[must_use]
    pub const fn code(self) -> u8 {
        self.0
    }

    /// Whether the owner gave no word.
    #[must_use]
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// Whether it is a word no table names.
    #[must_use]
    pub const fn is_out_of_range(self) -> bool {
        self.0 == Word::OUT_OF_RANGE.0
    }

    /// Core §7.6's JSON reading: the number, or the reserved code's name.
    #[must_use]
    pub fn to_json(self) -> Value {
        if self.is_out_of_range() {
            json!(Word::OUT_OF_RANGE_NAME)
        } else {
            json!(self.0)
        }
    }

    /// [`Word::to_json`] read back; `None` for anything else.
    #[must_use]
    pub fn of_json(v: &Value) -> Option<Word> {
        match v {
            Value::String(s) if s == Word::OUT_OF_RANGE_NAME => Some(Word::OUT_OF_RANGE),
            v => Word::new(u8::try_from(v.as_u64()?).ok()?),
        }
    }
}

/// A word's kind (§0): which field carries it, and so which tables name
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A status's `reason`.
    Reason,
    /// A check's `detail`.
    Detail,
    /// A fault's `code`.
    Code,
}

impl Kind {
    /// The three kinds, in the vocabulary's order.
    pub const ALL: [Kind; 3] = [Kind::Reason, Kind::Detail, Kind::Code];

    /// The annotation key a service's own table of this kind is written
    /// under (§4).
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Kind::Reason => "health.reasons",
            Kind::Detail => "health.details",
            Kind::Code => "health.codes",
        }
    }

    /// The table's name, as the fixtures and [`VOCABULARY`] spell it.
    #[must_use]
    pub const fn table(self) -> &'static str {
        match self {
            Kind::Reason => "reasons",
            Kind::Detail => "details",
            Kind::Code => "codes",
        }
    }

    /// The profile's table of this kind, as this text has it.
    #[must_use]
    pub const fn profile(self) -> &'static [&'static str] {
        match self {
            Kind::Reason => REASONS,
            Kind::Detail => DETAILS,
            Kind::Code => CODES,
        }
    }
}

/// Whether `s` is a name (§2.10): `[a-z][a-z0-9_]*`, the whole of it, as
/// v1 §2.10's codes are.
#[must_use]
pub fn is_name(s: &str) -> bool {
    super::is_code_form(s)
}

/// A value that is a table (§2.10, §4): a list of at most 31 names, none
/// twice. `None` for any other value.
#[must_use]
pub fn names(v: &Value) -> Option<Vec<String>> {
    words::list(v, OWN_MAX, is_name, true)
}

/// Why a contract's declaration of one table is invalid (§2.10), the
/// first that applies. An invalid declaration holds no table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invalid {
    /// A resource carries the key, and the contract's `uses` does not
    /// list `health.v2`.
    NotUsed,
    /// A resource's value is not a list of at most 31 names, none twice.
    Value,
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
            Self::Disagrees => "disagrees",
        }
    }

    /// The reason in words, for a linter's message.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotUsed => "the contract's `uses` does not list health.v2",
            Self::Value => {
                "its value is not a list of at most 31 names, each [a-z][a-z0-9_]*, none twice"
            }
            Self::Disagrees => {
                "two resources carry two different lists: write the key once, on [defaults]"
            }
        }
    }
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The table of one kind a contract holds (§2.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// No resource carries the key.
    None,
    /// The one list its resources carry.
    Table(Vec<String>),
    /// An invalid declaration, which holds none.
    Invalid(Invalid),
}

impl Declared {
    /// The table it holds, none where none is declared or the declaration
    /// is invalid.
    #[must_use]
    pub fn table(&self) -> Option<&[String]> {
        match self {
            Declared::Table(t) => Some(t),
            Declared::None | Declared::Invalid(_) => None,
        }
    }
}

/// [`declaration`], with the place of the value that made it invalid: its
/// index among the values given.
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
        match names(v) {
            Some(list) => lists.push(list),
            None => return (Declared::Invalid(Invalid::Value), Some(i)),
        }
    }
    if let Some(i) = lists.iter().position(|l| *l != lists[0]) {
        return (Declared::Invalid(Invalid::Disagrees), Some(i));
    }
    (Declared::Table(lists.swap_remove(0)), None)
}

/// A contract's table of one kind (§2.10), from whether its `uses` lists
/// `health.v2` and the value of the kind's key on each resource that
/// carries it. A resource that does not carry the key takes no part.
#[must_use]
pub fn declaration<'v>(uses_v2: bool, carried: impl IntoIterator<Item = &'v Value>) -> Declared {
    declared(uses_v2, carried).0
}

/// [`declaration`] over a loaded contract's resolved annotations, where
/// the canonical form carries a key written on `[defaults]` (core §9.5).
#[must_use]
pub fn declaration_of(c: &Contract, kind: Kind) -> Declared {
    declaration(
        uses_v2(c),
        c.resources
            .iter()
            .filter_map(|r| r.annotations.get(kind.key())),
    )
}

/// The service's own tables one contract holds, or one instance has
/// (§2.10): for each kind a list, or none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tables {
    pub reasons: Option<Vec<String>>,
    pub details: Option<Vec<String>>,
    pub codes: Option<Vec<String>>,
}

impl Tables {
    /// The table of `kind`, where there is one.
    #[must_use]
    pub fn get(&self, kind: Kind) -> Option<&[String]> {
        match kind {
            Kind::Reason => self.reasons.as_deref(),
            Kind::Detail => self.details.as_deref(),
            Kind::Code => self.codes.as_deref(),
        }
    }

    fn slot(&mut self, kind: Kind) -> &mut Option<Vec<String>> {
        match kind {
            Kind::Reason => &mut self.reasons,
            Kind::Detail => &mut self.details,
            Kind::Code => &mut self.codes,
        }
    }

    /// The tables a contract holds: each kind's [`declaration_of`], an
    /// invalid one holding none.
    #[must_use]
    pub fn of(c: &Contract) -> Tables {
        let mut t = Tables::default();
        for kind in Kind::ALL {
            *t.slot(kind) = declaration_of(c, kind).table().map(<[String]>::to_vec);
        }
        t
    }

    /// An instance's tables (§2.10): for each kind, [`instance_table`]
    /// over `named`, the tables of each revision its descriptor names that
    /// the reader holds. Never a revision the descriptor does not name: a
    /// table read under the wrong revision gives the wrong word.
    #[must_use]
    pub fn of_instance<'a>(named: impl IntoIterator<Item = &'a Tables>) -> InstanceTables {
        let named: Vec<&Tables> = named.into_iter().collect();
        let of = |kind| instance_table(named.iter().map(|t| t.get(kind)));
        InstanceTables {
            reasons: of(Kind::Reason),
            details: of(Kind::Detail),
            codes: of(Kind::Code),
        }
    }
}

/// An instance's table of one kind (§2.10): the one table among the
/// contracts it implements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instance {
    /// No named revision in hand holds one.
    None,
    /// The one list they hold.
    Table(Vec<String>),
    /// Two hold two different lists: no table. Its owner writes no word
    /// from 32 to 62, and a reader shows numbers.
    Differ,
}

impl Instance {
    /// The table, where the instance has one.
    #[must_use]
    pub fn table(&self) -> Option<&[String]> {
        match self {
            Instance::Table(t) => Some(t),
            Instance::None | Instance::Differ => None,
        }
    }
}

/// An instance's table of one kind, from the table of that kind in each
/// revision its descriptor names that the reader holds (`None`: that
/// revision holds none, or its declaration is invalid).
#[must_use]
pub fn instance_table<'a>(named: impl IntoIterator<Item = Option<&'a [String]>>) -> Instance {
    let mut held: Option<&[String]> = None;
    for table in named.into_iter().flatten() {
        match held {
            None => held = Some(table),
            Some(first) if first == table => {}
            Some(_) => return Instance::Differ,
        }
    }
    held.map_or(Instance::None, |t| Instance::Table(t.to_vec()))
}

/// An instance's three tables ([`Tables::of_instance`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceTables {
    pub reasons: Instance,
    pub details: Instance,
    pub codes: Instance,
}

impl Default for InstanceTables {
    /// No revision named or none in hand: no table, so numbers.
    fn default() -> Self {
        InstanceTables {
            reasons: Instance::None,
            details: Instance::None,
            codes: Instance::None,
        }
    }
}

impl InstanceTables {
    /// The instance's table of `kind`, none or `differ` among them.
    #[must_use]
    pub fn get(&self, kind: Kind) -> &Instance {
        match kind {
            Kind::Reason => &self.reasons,
            Kind::Detail => &self.details,
            Kind::Code => &self.codes,
        }
    }

    /// The instance's own table of `kind`, as [`write_word`] and
    /// [`read_word`] take it: none where it has none, or where two
    /// revisions differ.
    #[must_use]
    pub fn own(&self, kind: Kind) -> Option<&[String]> {
        self.get(kind).table()
    }
}

/// The number an owner writes for a word of `kind` (§2.10): 0 for no word
/// given, the profile's number for a name of the profile's table, 32 plus
/// its index for a name of its instance's `own` table, and `out_of_range`
/// for any other word, which stays in the owner's log. A name in both
/// tables is the profile's.
#[must_use]
pub fn write_word(kind: Kind, word: Option<&str>, own: Option<&[String]>) -> Word {
    let Some(word) = word else {
        return Word::NONE;
    };
    // Each table names 31 numbers, from its first: its places 0 to 30.
    let place = |index: Index, first: u8| match index {
        Index::At(i) => Word(first + i),
        Index::OutOfRange => Word::OUT_OF_RANGE,
    };
    // A name in both tables is the profile's.
    if kind.profile().contains(&word) {
        return place(words::write(kind.profile(), word, HALF_MAX), PROFILE_FIRST);
    }
    place(
        own.map_or(Index::OutOfRange, |t| words::write(t, word, HALF_MAX)),
        OWN_FIRST,
    )
}

/// A word as a reader shows it (§2.10, §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Named<'a> {
    /// 0: no word. Shown as no word.
    None,
    /// 1 to 31, a number this text's table names.
    Profile(&'static str),
    /// 32 to 62, the name at that index of the instance's own table.
    Service(&'a str),
    /// A number no table in hand names: a later text's, a later
    /// revision's, or the reader holds no table. Shown as its number.
    Number(u8),
    /// A word no table names: never a name, and never no word.
    OutOfRange,
}

/// Reads a word of `kind` (§2.10) with the instance's `own` table as the
/// reader holds it: never a table of a revision the owner's descriptor
/// does not name.
#[must_use]
pub fn read_word(kind: Kind, word: Word, own: Option<&[String]>) -> Named<'_> {
    let n = word.code();
    if word.is_none() {
        Named::None
    } else if word.is_out_of_range() {
        Named::OutOfRange
    } else if n < OWN_FIRST {
        match words::read(Some(kind.profile()), Index::At(n - PROFILE_FIRST)) {
            words::Word::Entry(name) => Named::Profile(name),
            _ => Named::Number(n),
        }
    } else {
        match words::read(own, Index::At(n - OWN_FIRST)) {
            words::Word::Entry(name) => Named::Service(name),
            _ => Named::Number(n),
        }
    }
}

/// How a table moved from one revision to the next (§2.10): a table grows
/// by appending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Growth {
    /// The same list.
    Same,
    /// The old list is a prefix of the new: every number keeps its word.
    Appended,
    /// Some number names another word, or none: a name removed,
    /// reordered, renamed or inserted.
    Moved,
}

impl Growth {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Appended => "appended",
            Self::Moved => "moved",
        }
    }
}

/// [`Growth`] of a table from `old` to `new`; a first table where there
/// was none is appended to the empty one.
#[must_use]
pub fn growth(old: &[String], new: &[String]) -> Growth {
    if old == new {
        Growth::Same
    } else if new.starts_with(old) {
        Growth::Appended
    } else {
        Growth::Moved
    }
}

/// One finding of [`lint_contract`]: the resource whose value drew it, by
/// its kind token and template, the key, and the reason's token and words
/// ([`Invalid::as_str`], [`Invalid::says`]). [`crate::words`]'s shape,
/// which `desired.v2` shares.
pub use crate::words::Lint;

/// What a linter reports of a contract that lists `health.v2` (§2.10):
/// each kind whose declaration is invalid, at the first resource whose
/// value is no table, or the first whose list differs from the one before
/// it. None for any other contract: `health.v1` defines no key, and a key
/// of a profile `uses` does not list is core E020's.
#[must_use]
pub fn lint_contract(c: &Contract) -> Vec<Lint> {
    if !uses_v2(c) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for kind in Kind::ALL {
        let carrying: Vec<_> = c
            .resources
            .iter()
            .filter(|r| r.annotations.contains_key(kind.key()))
            .collect();
        let values = carrying.iter().map(|r| &r.annotations[kind.key()]);
        if let (Declared::Invalid(reason), Some(i)) = declared(true, values) {
            out.push(Lint {
                profile: PROFILE,
                at: Lint::place(carrying[i]),
                key: Some(kind.key()),
                token: reason.as_str(),
                says: reason.says(),
            });
        }
    }
    out
}

// --- §3: the three records ---------------------------------------------------

/// Which of the standard schema's three types a record is (§3). The key
/// says which, never the tag: a `Status` and a `Check` have one layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordType {
    Status,
    Check,
    Fault,
}

impl RecordType {
    /// The three types, in the schema's order.
    pub const ALL: [RecordType; 3] = [RecordType::Status, RecordType::Check, RecordType::Fault];

    /// The type's name in the standard schema.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            RecordType::Status => "Status",
            RecordType::Check => "Check",
            RecordType::Fault => "Fault",
        }
    }

    /// The type's tag (§3, core §7.6): 3 for a `Status` and a `Check`,
    /// which have one layout, and 11 for a `Fault`.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            RecordType::Status | RecordType::Check => 3,
            RecordType::Fault => 11,
        }
    }

    /// The type's layout (§3): 42 bits with its time and 10 without for a
    /// `Status` and a `Check`, 58 and 26 for a `Fault`.
    #[must_use]
    pub fn layout(self) -> &'static Layout {
        static LAYOUTS: OnceLock<[Layout; 3]> = OnceLock::new();
        let all = LAYOUTS.get_or_init(|| {
            let doc = schema();
            RecordType::ALL.map(|t| {
                packed::layout_in(&doc, t.name()).expect("the standard schema's declaration reads")
            })
        });
        &all[self as usize]
    }
}

/// The standard schema's three types, without their descriptions, which
/// are no part of a layout (core §7.6):
/// `spec/profiles/health/health.v2.json`'s fields, in order.
fn schema() -> Value {
    let level = json!({"name": "level", "type": "enum",
                       "values": ["unknown", "ok", "degraded", "failed"]});
    let word = |name: &str| {
        json!({"name": name, "type": "int", "minimum": 0, "maximum": Word::MAX,
               "reserved": [Word::OUT_OF_RANGE_NAME]})
    };
    let time = |name: &str| json!({"name": name, "type": "time", "optional": true});
    json!({"types": {
        "Status": {"fields": [level, word("reason"), time("since"), packed::MARKER]},
        "Check": {"fields": [level, word("detail"), time("at"), packed::MARKER]},
        "Fault": {"fields": [
            word("code"),
            level,
            {"name": "argument", "type": "int", "minimum": 0, "maximum": Argument::MAX,
             "reserved": [Argument::NOT_AVAILABLE_NAME]},
            time("at"),
            packed::MARKER,
        ]},
    }})
}

/// The record's `level` name for no level (§2.6): code 0, v1's
/// `UNSPECIFIED`.
pub const UNKNOWN: &str = "unknown";

/// A record's level as core §7.6's reading spells it: a level's name in
/// lower case, or `unknown`.
fn level_json(l: Option<Level>) -> Value {
    json!(l.map_or(UNKNOWN, Level::as_str))
}

fn level_of_json(v: &Value) -> Option<Option<Level>> {
    Some(match v.as_str()? {
        UNKNOWN => None,
        "ok" => Some(Level::Ok),
        "degraded" => Some(Level::Degraded),
        "failed" => Some(Level::Failed),
        _ => return None,
    })
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

/// The time an owner writes as a record's `since` or `at` (§2.3), in whole
/// seconds since the Unix epoch, for its clock reading `unix_ms` under the
/// deployment epoch `epoch` in seconds: `None` where that instant has no
/// code, before the epoch or 2^32 seconds or more after it, and the owner
/// leaves the field absent. A device with no real-time clock at boot
/// reads 1970, which a catalog's later epoch does not code.
#[must_use]
pub fn time_at(unix_ms: i128, epoch: i64) -> Option<i64> {
    packed::time_code(unix_ms, Some(epoch), false).ok()?;
    i64::try_from(unix_ms.div_euclid(1000)).ok()
}

/// What a reader reads a record's level as (§2.6): the level of a record
/// at one, unknown for the record's `unknown` (v1's `UNSPECIFIED`,
/// [`Read::Unspecified`]), and unknown too for a record core §7.6's
/// reading refuses ([`Read::Undecodable`]), never `ok` and never a level
/// read from its bits.
#[must_use]
pub fn level_read(record: Result<Option<Level>, ReadRefusal>) -> Read {
    match record {
        Ok(Some(l)) => Read::Level(l),
        Ok(None) => Read::Unspecified,
        Err(_) => Read::Undecodable,
    }
}

/// The shape the three records share: a value in core §7.6's JSON
/// reading, written and read through its type's [`Layout`].
pub trait Packed: Sized {
    /// Which of the schema's types it is.
    const TYPE: RecordType;

    /// The record's value in core §7.6's JSON reading: a word a number or
    /// `out_of_range`, a time in milliseconds since the Unix epoch, an
    /// absent optional field no member.
    fn to_json(&self) -> Value;

    /// A value [`Layout::read`] gave for the type; `None` for any other.
    fn of_json(v: &Value) -> Option<Self>;

    /// The type's tag, which a put's CH carries.
    #[must_use]
    fn tag() -> u8 {
        Self::TYPE.tag()
    }

    /// The record's bytes, its times counted from the deployment epoch
    /// `epoch` in seconds (0 where the owner is given no catalog).
    ///
    /// # Errors
    /// [`WriteRefusal::Range`] for a time the epoch does not code: an
    /// owner leaves such a time absent ([`time_at`]).
    fn write(&self, epoch: i64) -> Result<Record, WriteRefusal> {
        Self::TYPE.layout().write(&self.to_json(), Some(epoch))
    }

    /// A put's payload under the owner's own `stamp`: CH (form 0, the
    /// stamp's width bit, the type's tag), the run, the seq, the record.
    ///
    /// # Errors
    /// As [`Packed::write`].
    fn put(&self, stamp: Stamp, epoch: i64) -> Result<Vec<u8>, WriteRefusal> {
        let record = self.write(epoch)?;
        let tag = Tag::new(Self::tag()).expect("a tag of four bits");
        Ok(Envelope::mutation(tag, stamp, &record.bytes).write())
    }

    /// Reads a record under the tag its envelope's CH carries, or a
    /// snapshot's page's, by core §7.6's reading: a later text's fields
    /// behind the marker are ignored.
    ///
    /// # Errors
    /// Core §7.6's refusal: `tag`, `length` or `pad`. No field of these
    /// types has an unused code, and each layout ends at its marker, so
    /// none is refused `code` or `extension`. A reader reads a refused
    /// status or check as unknown ([`level_read`]).
    fn read(tag: u8, bytes: &[u8], epoch: i64) -> Result<Self, ReadRefusal> {
        let value = Self::TYPE.layout().read(tag, bytes, Some(epoch))?;
        // Every value the layout reads is one the type declares.
        Self::of_json(&value).ok_or(ReadRefusal::Code)
    }
}

/// `state/status`'s record (§3): the service's level, why, and when the
/// level last changed. 6 bytes, or 2 with no time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Status {
    /// `None`: `unknown`, no level (§2.6).
    pub level: Option<Level>,
    /// A word of the reasons' tables.
    pub reason: Word,
    /// When the level last changed, on the owner's clock, in whole
    /// seconds since the Unix epoch; absent while that clock reads no
    /// instant the epoch codes (§2.3). A reader shows it, and ages
    /// nothing by it.
    pub since: Option<i64>,
}

impl Status {
    /// What an owner that has not yet established its level puts (§2.3):
    /// `unknown`, with the reason `starting`.
    #[must_use]
    pub fn starting(since: Option<i64>) -> Status {
        Status {
            level: None,
            reason: write_word(Kind::Reason, Some(STARTING), None),
            since,
        }
    }
}

impl Packed for Status {
    const TYPE: RecordType = RecordType::Status;

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("level".into(), level_json(self.level));
        m.insert("reason".into(), self.reason.to_json());
        if let Some(t) = self.since {
            m.insert("since".into(), time_json(t));
        }
        Value::Object(m)
    }

    fn of_json(v: &Value) -> Option<Self> {
        Some(Status {
            level: level_of_json(v.get("level")?)?,
            reason: Word::of_json(v.get("reason")?)?,
            since: match v.get("since") {
                None => None,
                Some(t) => Some(time_of_json(t)?),
            },
        })
    }
}

/// `state/checks/{check}`'s record (§3): one named check's level, what it
/// found, and when. 6 bytes, or 2 with no time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Check {
    /// `None`: `unknown`, a check its owner cannot run, which bounds
    /// nothing (§2.2, §2.6).
    pub level: Option<Level>,
    /// A word of the details' tables.
    pub detail: Word,
    /// When the check found the value put, on the owner's clock, in whole
    /// seconds since the Unix epoch. Informative: a reader never ages a
    /// check by it (§2.3).
    pub at: Option<i64>,
}

impl Packed for Check {
    const TYPE: RecordType = RecordType::Check;

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("level".into(), level_json(self.level));
        m.insert("detail".into(), self.detail.to_json());
        if let Some(t) = self.at {
            m.insert("at".into(), time_json(t));
        }
        Value::Object(m)
    }

    fn of_json(v: &Value) -> Option<Self> {
        Some(Check {
            level: level_of_json(v.get("level")?)?,
            detail: Word::of_json(v.get("detail")?)?,
            at: match v.get("at") {
                None => None,
                Some(t) => Some(time_of_json(t)?),
            },
        })
    }
}

/// A fault's `argument` (§2.3): the code's own number, where the code's
/// table says what it counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Argument {
    /// A number from 0 to 65,534.
    Number(u16),
    /// The code takes none, or none was measured: the record's
    /// `not_available`, all 16 bits set.
    NotAvailable,
}

impl Argument {
    /// The largest number that is a value, 65,534.
    pub const MAX: u16 = 65_534;

    /// The record's name for [`Argument::NotAvailable`].
    pub const NOT_AVAILABLE_NAME: &'static str = "not_available";

    /// The argument `n`, from 0 to 65,534; `None` for 65,535, which is no
    /// value but `not_available`'s code.
    #[must_use]
    pub const fn new(n: u16) -> Option<Argument> {
        if n <= Argument::MAX {
            Some(Argument::Number(n))
        } else {
            None
        }
    }

    /// A `clock_ahead` fault's argument (§2.5): the offset its owner's
    /// guard measured, in whole seconds, rounded up; an offset of 65,534 s
    /// or more is written 65,534; `not_available` where it measured none.
    #[must_use]
    pub fn clock_ahead(offset: Option<Duration>) -> Argument {
        let Some(offset) = offset else {
            return Argument::NotAvailable;
        };
        let up = offset
            .as_secs()
            .saturating_add(u64::from(offset.subsec_nanos() > 0));
        Argument::Number(u16::try_from(up).map_or(Argument::MAX, |s| s.min(Argument::MAX)))
    }

    /// Core §7.6's JSON reading: the number, or the reserved code's name.
    #[must_use]
    pub fn to_json(self) -> Value {
        match self {
            Argument::Number(n) => json!(n),
            Argument::NotAvailable => json!(Argument::NOT_AVAILABLE_NAME),
        }
    }

    /// [`Argument::to_json`] read back; `None` for anything else.
    #[must_use]
    pub fn of_json(v: &Value) -> Option<Argument> {
        match v {
            Value::String(s) if s == Argument::NOT_AVAILABLE_NAME => Some(Argument::NotAvailable),
            v => Argument::new(u16::try_from(v.as_u64()?).ok()?),
        }
    }
}

/// `stream/faults`'s record (§3): one occurrence a level would lose. 8
/// bytes, or 4 with no time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fault {
    /// A word of the codes' tables: 1 is `clock_ahead`.
    pub code: Word,
    /// The occurrence's severity; `None`: `unknown`, an occurrence of
    /// unknown severity (§2.6).
    pub level: Option<Level>,
    pub argument: Argument,
    /// When it happened, on the owner's clock, in whole seconds since the
    /// Unix epoch. A reader ages nothing by it (§2.5).
    pub at: Option<i64>,
}

impl Fault {
    /// The one fault an owner publishes when its clock guard begins to
    /// hold (§2.5): the code `clock_ahead`, the level `failed`, and the
    /// offset it measured ([`Argument::clock_ahead`]).
    #[must_use]
    pub fn clock_ahead(offset: Option<Duration>, at: Option<i64>) -> Fault {
        Fault {
            code: write_word(Kind::Code, Some(super::CLOCK_AHEAD), None),
            level: Some(Level::Failed),
            argument: Argument::clock_ahead(offset),
            at,
        }
    }

    /// Whether its code is the profile's `clock_ahead`, whatever its level
    /// and its argument.
    #[must_use]
    pub fn is_clock_ahead(&self) -> bool {
        read_word(Kind::Code, self.code, None) == Named::Profile(super::CLOCK_AHEAD)
    }
}

impl Packed for Fault {
    const TYPE: RecordType = RecordType::Fault;

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("code".into(), self.code.to_json());
        m.insert("level".into(), level_json(self.level));
        m.insert("argument".into(), self.argument.to_json());
        if let Some(t) = self.at {
            m.insert("at".into(), time_json(t));
        }
        Value::Object(m)
    }

    fn of_json(v: &Value) -> Option<Self> {
        Some(Fault {
            code: Word::of_json(v.get("code")?)?,
            level: level_of_json(v.get("level")?)?,
            argument: Argument::of_json(v.get("argument")?)?,
            at: match v.get("at") {
                None => None,
                Some(t) => Some(time_of_json(t)?),
            },
        })
    }
}

// --- §2.2, §2.4: a late joiner's two snapshots -------------------------------

/// Which of the standard contract's two collections a snapshot is of
/// (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Collection {
    /// `state/status`: one entry of the empty id, on the status's own
    /// key, collection 1.
    Status,
    /// `state/checks/{check}`: one entry per check, its id the check's
    /// name, on `…/state/checks`, collection 0.
    Checks,
}

impl Collection {
    /// The collection's index in a snapshot ([`STATUS_COLL`],
    /// [`CHECKS_COLL`]).
    #[must_use]
    pub const fn coll(self) -> u32 {
        match self {
            Collection::Status => STATUS_COLL,
            Collection::Checks => CHECKS_COLL,
        }
    }

    /// The key a reply's snapshot of this collection is on, below the
    /// owner's `health.v2/`: the status's own key, and the checks'
    /// template's fixed chunks (core §4.2).
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Collection::Status => "health.v2/state/status",
            Collection::Checks => "health.v2/state/checks",
        }
    }

    /// The collection a page read on `key` is of, where its `coll` is
    /// that collection's: `None` for a key that is neither's, and for an
    /// index the standard contract does not give that key.
    #[must_use]
    pub fn of(key: &str, coll: u32) -> Option<Collection> {
        [Collection::Status, Collection::Checks]
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

/// One live entry of a snapshot, read (§2.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<R> {
    /// The check's name; empty for the status.
    pub id: String,
    /// Its put's stamp: the entry's own run where it states one, else the
    /// snapshot's.
    pub stamp: Stamp,
    pub confirmation: Confirmation,
    /// When the owner last vouched for it (`freshness.v2` §2.4): the
    /// snapshot's time for confirmation 1, its `conf` for 2, none for 0
    /// and for a time that is unknown. A GET reader ages the status from
    /// it.
    pub confirmed_at: Option<collection::Time>,
    /// Its record, read under the page's tag, or core §7.6's refusal.
    pub record: Result<R, ReadRefusal>,
}

/// What the status's snapshot holds of the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusEntry {
    /// No entry: the owner holds no status, which §2.3 forbids.
    None,
    /// A tombstone: the owner deleted it within its window.
    Tombstone(Stamp),
    Live(Entry<Status>),
}

/// The status's snapshot, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub heard: Heard,
    pub entry: StatusEntry,
}

/// The checks' snapshot, read: every current check, and each one retired
/// within the window, in the order read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksSnapshot {
    pub heard: Heard,
    pub live: Vec<Entry<Check>>,
    /// Each tombstone: the check's name and its delete's stamp.
    pub retired: Vec<(String, Stamp)>,
}

/// A late joiner's reading of one owner's answer to a GET over its
/// `health.v2/state/**` (§2.4, core §4.2): the status's snapshot and the
/// checks', each none where no page of it was read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reading {
    pub status: Option<StatusSnapshot>,
    pub checks: Option<ChecksSnapshot>,
    /// A page of a second snapshot of one collection was read, under
    /// another header than its first: left out, and the reading is then
    /// never one instant.
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

fn entry_of<R: Packed>(e: &collection::Entry, page: &Page, epoch: i64) -> Entry<R> {
    let confirmed_at = match e.confirmation {
        Confirmation::None => None,
        Confirmation::AtTime => Some(page.time),
        Confirmation::At(t) => Some(t),
    }
    .filter(|t| *t != collection::Time::Unknown);
    Entry {
        id: e.id.clone(),
        stamp: stamp_of(e.run, e.seq, page.run),
        confirmation: e.confirmation,
        confirmed_at,
        record: R::read(page.tag.get(), &e.record, epoch),
    }
}

impl Reading {
    /// Reads a GET's replies, each its key and its payload, in the order
    /// they arrived, under the deployment epoch `epoch` in seconds. A
    /// reply that is no page core §7.5 accepts, and a page that is of
    /// neither collection of the standard contract ([`Collection::of`]),
    /// takes no part.
    #[must_use]
    pub fn of<'a>(replies: impl IntoIterator<Item = (&'a str, &'a [u8])>, epoch: i64) -> Reading {
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
        let header = |p: &Pages| Heard {
            complete: p.heard.complete(),
            tag: p.heard.tag,
            run: p.heard.run,
            asof: p.heard.asof,
            time: p.heard.time,
        };
        let [status, checks] = held;
        let status = status.map(|p| {
            let live = p.pages.iter().find_map(|page| {
                page.live
                    .iter()
                    .find(|e| e.id.is_empty())
                    .map(|e| (e, page))
            });
            let tomb = p
                .pages
                .iter()
                .find_map(|page| page.tombs.iter().find(|t| t.id.is_empty()));
            let entry = match (live, tomb) {
                (Some((e, page)), _) => StatusEntry::Live(entry_of(e, page, epoch)),
                (None, Some(t)) => StatusEntry::Tombstone(stamp_of(t.run, t.seq, p.heard.run)),
                (None, None) => StatusEntry::None,
            };
            StatusSnapshot {
                heard: header(&p),
                entry,
            }
        });
        let checks = checks.map(|p| ChecksSnapshot {
            heard: header(&p),
            live: p
                .pages
                .iter()
                .flat_map(|page| page.live.iter().map(move |e| entry_of(e, page, epoch)))
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
        });
        Reading {
            status,
            checks,
            mixed,
        }
    }

    /// §2.2's **one instant**: both snapshots read and complete, under one
    /// run, in width and value, and one asof, and no live entry and no
    /// tombstone of that run with a seq above that asof. An entry that
    /// states another run is an earlier run's, and breaks nothing. Each
    /// snapshot then shows its collection as of that asof exactly, so the
    /// two were current together.
    #[must_use]
    pub fn is_one_instant(&self) -> bool {
        let (Some(s), Some(c)) = (&self.status, &self.checks) else {
            return false;
        };
        if self.mixed
            || !s.heard.complete
            || !c.heard.complete
            || (s.heard.run, s.heard.asof) != (c.heard.run, c.heard.asof)
        {
            return false;
        }
        let (run, asof) = (s.heard.run, s.heard.asof);
        let within = |stamp: &Stamp| stamp.run != run || stamp.seq.get() <= asof.get();
        let status = match &s.entry {
            StatusEntry::None => true,
            StatusEntry::Tombstone(stamp) => within(stamp),
            StatusEntry::Live(e) => within(&e.stamp),
        };
        status
            && c.live.iter().all(|e| within(&e.stamp))
            && c.retired.iter().all(|(_, stamp)| within(stamp))
    }

    /// The level of the status read, as [`super::judge_status`] takes it:
    /// `None` where no live entry was read.
    #[must_use]
    pub fn status_level(&self) -> Option<Read> {
        match &self.status.as_ref()?.entry {
            StatusEntry::Live(e) => Some(level_read(e.record.map(|s| s.level))),
            StatusEntry::None | StatusEntry::Tombstone(_) => None,
        }
    }

    /// The levels of the checks the owner answered, in the order read, as
    /// [`super::judge_status`] takes them: `None` where the checks'
    /// snapshot was not read whole, the checks then not known.
    #[must_use]
    pub fn check_levels(&self) -> Option<Vec<Read>> {
        let c = self.checks.as_ref().filter(|c| c.heard.complete)?;
        Some(
            c.live
                .iter()
                .map(|e| level_read(e.record.map(|k| k.level)))
                .collect(),
        )
    }

    /// The break of §2.2 a tool may report from this reading alone:
    /// `None` unless the reading is one instant and the status's record
    /// reads at a level; then whether a live check's record reads at a
    /// worse level. A check at `unknown`, and one whose record is refused,
    /// bound nothing. From any other reading a tool needs two readings a
    /// grace apart, as v1 §2.2 says.
    #[must_use]
    pub fn status_better_than_a_check(&self) -> Option<bool> {
        if !self.is_one_instant() {
            return None;
        }
        let status = self.status_level()?.level()?;
        Some(!super::consistent(status, &self.check_levels()?))
    }
}

// --- §2.5, §5: is its clock ahead? ---------------------------------------------

/// One delivery a reader heard of one service, as §5's last question
/// reads it under this major (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClockHeard {
    /// A read heartbeat (`freshness.v2` §2.5) covering `health.v2`, or a
    /// read owner heartbeat of the status's class, `host`
    /// ([`STATUS_CLASS`]), with the time it carries. Its sections play no
    /// part: a cleared bit, a digest that is not the view's and a map bit
    /// of 0 change nothing.
    Heartbeat(heartbeat::Time),
    /// A heartbeat or an owner heartbeat that was not read: another
    /// revision's, a refused one or an older one. It counts for nothing,
    /// whatever its time.
    HeartbeatUnread,
    /// A put of `state/status`, which the guard does not hold: nothing of
    /// the clock.
    StatusPut,
    /// A delete of `state/status`, which §2.3 forbids: nothing of the
    /// clock.
    StatusDeleted,
    /// A `faults` sample whose code is `clock_ahead`: an occurrence,
    /// published once, which says that a hold began and cannot say that
    /// it lasts. It answers nothing.
    ClockAhead,
    /// A `faults` sample with any other code.
    Fault,
}

/// §5's last question from the time of the last heartbeat a reader read
/// (§2.5), `None` where it read none: ahead is *yes*, an instant *no*
/// (the owner's guard did not hold when it wrote it), unknown
/// unobservable (its clock reads no instant, which says nothing of
/// ahead), and none read unobservable. Not asked as the first question's:
/// absent, or a descriptor that lists neither major. In every other case
/// the heartbeats answer: a descriptor not read and a face closed to the
/// status change nothing.
///
/// A service whose descriptor lists `health.v1` and not `health.v2` is
/// asked [`super::clock_ahead`], where the ahead time answers nothing
/// alone ([`super::Major::of`]).
#[must_use]
pub fn clock_of(presence: Presence, last_read: Option<heartbeat::Time>) -> Clock {
    let c = |answer, reason| Clock { answer, reason };
    match presence {
        Presence::Absent => return c(Answer::NotAsked, ClockReason::Absent),
        Presence::Present(Listing::NotListed) => {
            return c(Answer::NotAsked, ClockReason::NotListed);
        }
        _ => {}
    }
    match last_read {
        Some(heartbeat::Time::Ahead) => c(Answer::Yes, ClockReason::Ahead),
        Some(heartbeat::Time::At { .. }) => c(Answer::No, ClockReason::Confirmed),
        Some(heartbeat::Time::Unknown) => c(Answer::Unobservable, ClockReason::TimeUnknown),
        None => c(Answer::Unobservable, ClockReason::NothingHeard),
    }
}

/// [`clock_of`] from what one reader heard of one service, in arrival
/// order: the last read heartbeat answers, and nothing else does. A
/// `clock_ahead` fault, a put of the status and the faults' silence say
/// nothing of the question.
#[must_use]
pub fn clock_ahead(presence: Presence, heard: &[ClockHeard]) -> Clock {
    let last = heard.iter().rev().find_map(|h| match h {
        ClockHeard::Heartbeat(t) => Some(*t),
        _ => None,
    });
    clock_of(presence, last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn the_schemas_layouts_are_six_six_and_eight_bytes() {
        for (t, tag, min, max) in [
            (RecordType::Status, 3, 10, 42),
            (RecordType::Check, 3, 10, 42),
            (RecordType::Fault, 11, 26, 58),
        ] {
            let l = t.layout();
            assert_eq!(
                (l.tag(), l.min_bits(), l.max_bits()),
                (tag, min, max),
                "{t:?}"
            );
            assert_eq!(t.tag(), tag);
        }
        assert_eq!(
            RecordType::Status.layout().form(),
            RecordType::Check.layout().form(),
            "one layout, so one tag: the key says which"
        );
    }

    #[test]
    fn a_status_writes_reads_and_puts_as_the_text_says() {
        let epoch = 1_767_225_600;
        let s = Status {
            level: Some(Level::Ok),
            reason: write_word(Kind::Reason, Some("serving"), None),
            since: Some(1_794_062_759),
        };
        let r = s.write(epoch).unwrap();
        assert_eq!(r.bytes, [0x42, 0x80, 0xcc, 0xc0, 0x53, 0x80]);
        assert_eq!(Status::read(3, &r.bytes, epoch), Ok(s));
        let stamp = Stamp {
            run: Run::U16(3),
            seq: Seq::new(5).unwrap(),
        };
        let put = s.put(stamp, epoch).unwrap();
        assert_eq!(put[0], 0x03, "form 0, a 16-bit run, tag 3");
        assert_eq!(put.len(), 12);
        // An owner at start, on a clock the epoch does not code: 2 bytes.
        let start = Status::starting(time_at(0, epoch));
        assert_eq!(start.since, None);
        assert_eq!(start.write(epoch).unwrap().bytes, [0x01, 0x00]);
        // A time the epoch does not code is refused: the owner leaves it
        // absent.
        let before = Status {
            since: Some(epoch - 1),
            ..s
        };
        assert_eq!(before.write(epoch).unwrap_err(), WriteRefusal::Range);
        assert_eq!(time_at(1_794_062_759_123, epoch), Some(1_794_062_759));
        assert_eq!(time_at(i128::from(epoch) * 1000 - 1, epoch), None);
    }

    #[test]
    fn a_refused_record_and_unknown_are_never_ok() {
        let epoch = 0;
        let ok = [0x42, 0x80, 0xcc, 0xc0, 0x53, 0x80];
        let level =
            |tag, bytes: &[u8]| level_read(Status::read(tag, bytes, epoch).map(|s| s.level));
        assert_eq!(level(3, &ok), Read::Level(Level::Ok));
        let mut pad = ok;
        pad[5] |= 1;
        assert_eq!(Status::read(3, &pad, epoch), Err(ReadRefusal::Pad));
        assert_eq!(
            level(3, &pad),
            Read::Undecodable,
            "never the ok its bits read"
        );
        assert_eq!(level(11, &ok), Read::Undecodable, "a fault's tag");
        assert_eq!(level(3, &ok[..5]), Read::Undecodable);
        assert_eq!(level(3, &[0x01, 0x00]), Read::Unspecified, "unknown");
        assert_eq!(Read::Unspecified.level(), None);
    }

    #[test]
    fn a_clock_ahead_fault_carries_its_offset_rounded_up_and_capped() {
        let a = |ms| Argument::clock_ahead(Some(Duration::from_millis(ms)));
        assert_eq!(a(2000), Argument::Number(2));
        assert_eq!(a(2001), Argument::Number(3));
        assert_eq!(a(1), Argument::Number(1));
        assert_eq!(a(65_534_001), Argument::Number(65_534));
        assert_eq!(Argument::clock_ahead(None), Argument::NotAvailable);
        assert_eq!(Argument::new(65_535), None);
        let f = Fault::clock_ahead(Some(Duration::from_secs(2)), Some(1_794_062_759));
        assert!(f.is_clock_ahead());
        assert_eq!(
            f.write(1_767_225_600).unwrap().bytes,
            [0x07, 0x00, 0x02, 0x80, 0xcc, 0xc0, 0x53, 0x80]
        );
        assert_eq!(Fault::tag(), 11);
    }

    #[test]
    fn a_word_is_the_profiles_number_then_the_instances_own_then_out_of_range() {
        let own = table(&["starting", "tc_netns_gone"]);
        let w = |word| write_word(Kind::Reason, word, Some(&own));
        assert_eq!(w(None), Word::NONE);
        assert_eq!(w(Some("starting")).code(), 1, "the profile's number");
        assert_eq!(w(Some("tc_netns_gone")).code(), 33);
        assert_eq!(w(Some("timeout")), Word::OUT_OF_RANGE, "a detail's name");
        assert_eq!(
            write_word(Kind::Reason, Some("tc_netns_gone"), None),
            Word::OUT_OF_RANGE
        );
        let r = |n| read_word(Kind::Reason, Word::new(n).unwrap(), Some(&own));
        assert_eq!(r(0), Named::None);
        assert_eq!(r(8), Named::Profile("misconfigured"));
        assert_eq!(r(9), Named::Number(9), "a later text's");
        assert_eq!(r(33), Named::Service("tc_netns_gone"));
        assert_eq!(r(34), Named::Number(34), "a later revision's");
        assert_eq!(
            read_word(Kind::Reason, Word::OUT_OF_RANGE, Some(&own)),
            Named::OutOfRange
        );
        assert_eq!(Word::new(63), None);
        // Every profile table fits its half, 1 to 31.
        for kind in Kind::ALL {
            assert!(kind.profile().len() <= 31, "{kind:?}");
            for (i, name) in kind.profile().iter().enumerate() {
                assert!(is_name(name));
                assert_eq!(
                    usize::from(write_word(kind, Some(name), None).code()),
                    i + 1
                );
            }
        }
    }

    #[test]
    fn an_instance_has_the_one_table_its_named_revisions_hold() {
        let a = table(&["tc_a", "tc_b"]);
        let b = table(&["tc_a"]);
        assert_eq!(instance_table([None, None]), Instance::None);
        assert_eq!(
            instance_table([None, Some(&a[..]), Some(&a[..])]),
            Instance::Table(a.clone())
        );
        assert_eq!(
            instance_table([Some(&a[..]), Some(&b[..])]),
            Instance::Differ
        );
        assert_eq!(Instance::Differ.table(), None, "no word from 32 to 62");
        assert_eq!(growth(&b, &a), Growth::Appended);
        assert_eq!(growth(&a, &b), Growth::Moved);
    }

    #[test]
    fn the_last_read_heartbeat_answers_the_clock_question_and_no_fault_does() {
        use heartbeat::Time;
        let p = Presence::Present(Listing::Listed { token: true });
        let at = Time::At { secs: 1, ms: 0 };
        let a = |heard: &[ClockHeard]| {
            let c = clock_ahead(p, heard);
            (c.answer, c.reason)
        };
        assert_eq!(
            a(&[ClockHeard::Heartbeat(Time::Ahead)]),
            (Answer::Yes, ClockReason::Ahead)
        );
        assert_eq!(
            a(&[ClockHeard::ClockAhead]),
            (Answer::Unobservable, ClockReason::NothingHeard),
            "a fault is an occurrence, and answers nothing"
        );
        assert_eq!(
            a(&[ClockHeard::Heartbeat(at), ClockHeard::ClockAhead]),
            (Answer::No, ClockReason::Confirmed)
        );
        assert_eq!(
            a(&[
                ClockHeard::Heartbeat(Time::Ahead),
                ClockHeard::Heartbeat(Time::Unknown)
            ]),
            (Answer::Unobservable, ClockReason::TimeUnknown)
        );
        assert_eq!(
            a(&[
                ClockHeard::Heartbeat(Time::Ahead),
                ClockHeard::HeartbeatUnread,
                ClockHeard::StatusPut
            ]),
            (Answer::Yes, ClockReason::Ahead)
        );
        assert_eq!(
            clock_of(Presence::Absent, Some(Time::Ahead)).answer,
            Answer::NotAsked
        );
        assert_eq!(
            clock_of(Presence::Incomplete, Some(Time::Ahead)).answer,
            Answer::Yes,
            "a heartbeat read on its key is its owner's word"
        );
    }
}
