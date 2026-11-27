//! `alarms.v2` in the runtime: a service's active alarms in slots, one
//! packed record each (`spec/profiles/alarms/v2.md`, text 0.1, #984's S2).
//!
//! The owner's half of the profile. The record, the table, the slot
//! choice and every reading are `zenkey-model`'s
//! ([`zenkey_model::alarms::v2`]); this module puts what the text asks of
//! an owner, on the standard contract, embedded.
//!
//! ```text
//! ServiceBuilder::alarms_v2(startup) ─▶ Alarms: implements alarms.v2, exposes alarms/{slot} and
//!        │                              words/{slot}, answers their GETs; adopts the previous
//!        │                              instance's alarms from its record, by slot (§2.9)
//!        │  raise(condition) ─▶ raised │ escalated │ refreshed │ deferred │ unchanged   (§2.5)
//!        │  resolve(alert_key) ─▶ the resolved record on its slot, and no delete       (§2.5)
//!        │  set_confirming(alert_key, on), set_confirming_all(on)                     (§2.7)
//!        ▼ start()
//!   Service::alarms_v2() ─ the same handle; Service::new_epoch re-puts every firing record
//!   first, and the new instance holds no resolved one (§2.9)
//! ```
//!
//! | Rule (`v2.md`) | Here |
//! |---|---|
//! | §2.1 one major per instance | [`crate::ServiceBuilder::alarms_v2`] is refused beside `alarms.v1`, and `alarms.v1` beside it, through either major's call or [`crate::ServiceBuilder::implement`] |
//! | §2.2 which slot, one identity one slot, never a firing one | [`model::Slots`]: an identity's own slot, firing or resolved, else the lowest that does not fire |
//! | §2.4 the record, by the table or by words | [`model::write_alarm`] over the instance's table ([`Alarms::table`]): the rule's index, its labels' codes and its argument, or `not_available` and the alert key, the words on `words/<slot>` first |
//! | §2.5 the acts, `at` | [`Alarms::raise`] decides the act from what the slot holds; `at` new at each transition by [`model::Dating`], kept at a refresh, absent while the clock has no code |
//! | §2.5 "what stays" | an identity's `rule`, `labels` and `key` are fixed when it takes its slot, and held for as long as the instance holds the slot: only the argument and the severity move |
//! | §2.5 a resolve is one put, no delete | [`Alarms::resolve`] puts the resolved record; no call deletes a slot's key |
//! | §2.5 SHOULD NOT refresh more than once in 450 s | a refresh within half the horizon of the last one put is [`Act::Deferred`], nothing put; the next act carries the latest reading, and a raise once the window passed puts it |
//! | §2.7 confirmation by the heartbeat | the contract uses `freshness.v2`: the heartbeat covering `alarms.v2` vouches for each firing record by its bit every 427.5 s; a resolved record's bit is cleared before its put |
//! | §2.7 no confirmation of an alarm no longer evaluated | [`Alarms::set_confirming`]; any act on the alarm is an evaluation, and sets its bit again |
//! | §2.8 the bound, a refusal, nothing for room | 256 slots, or [`Startup::cardinality`]; a raise with every slot firing is [`Error::Budget`], nothing put, nothing resolved |
//! | §2.8 SHOULD report the refusal | the application's: the refusal names the resource, the bound and whose it is |
//! | §2.9 adoption by slot, by name, before anything is alive | [`crate::ServiceBuilder::alarms_v2`], from [`Startup::record`]: [`model::Slots::restart`]'s steps, each alarm written again by this instance's table where it names its words, else by words; an `alarms.v1` record's firing documents onto the slots §2.2 gives them, `at` the `timestamp`'s second |
//! | §2.9 SHOULD keep, by slot, each record, its words and its alert key | [`OwnerRecord::save`] after every act, a resolved record included; [`FileRecord`] is one, a JSON file |
//! | §2.9 a clean close leaves them | [`crate::Service::close`] touches no alarm |
//!
//! **The table is the instance's** (§4): the one `alarms.rules` among the
//! contracts the service implements ([`model::instance_table`]), read again
//! at each [`crate::ServiceBuilder::implement`]. An alarm takes the table
//! in force when its identity takes a slot, so implement the contract that
//! holds the table before [`crate::ServiceBuilder::alarms_v2`]: adoption
//! writes by the table implemented at that call. A service with no table
//! writes every alarm by words, and asks a newcomer for nothing more.
//!
//! **The record is the application's** ([`OwnerRecord`]): loaded once, in
//! [`crate::ServiceBuilder::alarms_v2`], and saved whole after every act.
//! A record that cannot be saved is logged as a warning; the act on the
//! bus stands. No archive is read: v2.md §2.9's second source is not built
//! here.
//!
//! **The contract is the published bundle**, revision 2.0 ([`FINGERPRINT`]),
//! embedded byte for byte from `spec/profiles/.history/alarms.v2/` through
//! `bundles/`. [`crate::ServiceBuilder::alarms_v2_with`] implements another
//! revision of the major, its confirmation period and its refresh window
//! following its `freshness.ttl_s`: a test's copy with a short horizon.
//!
//! **The owner's clock** is [`Minter`]'s, the session's `new_timestamp()`,
//! the wall clock on a session without timestamping (core §4.3, §8.2);
//! a record's `at` counts whole seconds of it from the deployment epoch
//! ([`crate::ServiceBuilder::deployment_epoch`], core §7.6).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::Deserialize;
use zenkey_model::alarms::v2 as model;
use zenkey_model::alarms::{self as common, Severity, State};
use zenkey_model::contract::{Body, Resource};
use zenkey_model::grammar::{Addr, IfaceId, KindToken};
use zenkey_model::packed::Decimal;
use zenkey_model::schema::TypeId;
use zenkey_model::template::Bindings;

pub use zenkey_model::alarms::v2::{
    Argument, FINGERPRINT, Fields, Record, Rule, RuleIndex, Words, alarm_key, words_key,
};

use crate::budget::{Breach, Budget, Refusal};
use crate::call::BoxFuture;
use crate::error::{Error, Result};
use crate::implementation::Implementation;
use crate::service::ServiceBuilder;
use crate::state::{Minter, StateWriter, Store};
use crate::writer::{PackedType, Writer};

/// The standard contract's bundle, revision 2.0 as published in
/// `spec/profiles/.history/alarms.v2/` (§3), byte for byte, read through
/// `bundles/` so that the crate's package carries the bytes (#929).
pub const BUNDLE: &[u8] = include_bytes!(
    "../../bundles/alarms.v2/2292477cadf555dd09b07499ca048add4e48c63d60a209c13eaf543988c32cb7.bundle.json"
);

/// The records' template, as the descriptor names it (§3).
pub const ALARMS: &str = "state/alarms/{slot}";

/// The words' template, as the descriptor names it (§3).
pub const WORDS: &str = "state/words/{slot}";

/// The templates' parameter.
const PARAM: &str = "slot";

/// How many identities no longer held this owner remembers the greatest
/// `at` of (§2.5, "the greatest `at` the owner knows it wrote").
const PAST: usize = 1024;

static IMPLEMENTATION: LazyLock<Implementation> = LazyLock::new(|| {
    Implementation::from_bundle(BUNDLE).expect("the published alarms.v2 bundle verifies")
});

/// The standard contract's interface id, `alarms.v2`.
#[must_use]
pub fn iface() -> IfaceId {
    model::IFACE.parse().expect("alarms.v2 is an interface id")
}

/// The standard contract, implemented from the embedded bundle: what
/// [`crate::ServiceBuilder::alarms_v2`] implements, and what a reader of
/// `alarms.v2` is compiled against.
#[must_use]
pub fn implementation() -> Implementation {
    IMPLEMENTATION.clone()
}

/// What an owner found (§2.4): the condition an alarm is raised for, in
/// words, and its reading. Its identity is its rule's and its
/// discriminating labels' alert key (§2.2); the table maps the rest onto
/// numbers, or the alarm goes by words.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Condition {
    /// The stable name of what raised it: a table's rule's, or any other.
    pub rule: String,
    /// Every discriminating label is identity; a host-scoped one (`host`,
    /// `host.*`) is not, and is written only in an alarm's words (§2.4).
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    pub severity: Severity,
    /// One line for a person. A table's rule writes its own template
    /// (§4); an alarm by words carries this one.
    pub summary: String,
    /// What produced it: a table's rule has its own (§4).
    #[serde(default)]
    pub category: Option<String>,
    /// The rule's own number, the reading that changes while the
    /// condition holds, as a decimal (§2.4, §4): written as the rule's
    /// `argument` declares it, `not_available` where it declares none. Read
    /// from JSON as a string or a number.
    #[serde(default, deserialize_with = "reading_text")]
    pub reading: Option<String>,
}

/// A reading as JSON gives it: a decimal's string, or a number's text.
fn reading_text<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    match Option::<serde_json::Value>::deserialize(d)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Number(n)) => Ok(Some(n.to_string())),
        Some(other) => Err(serde::de::Error::custom(format!(
            "a reading is a decimal, a string or a number: {other}"
        ))),
    }
}

impl Condition {
    /// A condition of `rule` at `severity`, with no label and no reading.
    #[must_use]
    pub fn new(rule: impl Into<String>, severity: Severity, summary: impl Into<String>) -> Self {
        Self {
            rule: rule.into(),
            labels: BTreeMap::new(),
            severity,
            summary: summary.into(),
            category: None,
            reading: None,
        }
    }

    /// With the label `name` = `value`.
    #[must_use]
    pub fn label(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(name.into(), value.into());
        self
    }

    /// With a category.
    #[must_use]
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// With the reading `reading`, a decimal such as `85.5`.
    #[must_use]
    pub fn reading(mut self, reading: impl Into<String>) -> Self {
        self.reading = Some(reading.into());
        self
    }

    /// Its alert key, by v1's recipe through its sanitizing step (§2.2,
    /// v1 §2.3), as 16 hexadecimal digits.
    #[must_use]
    pub fn alert_key(&self) -> String {
        common::key_hex(self.identity())
    }

    fn identity(&self) -> u64 {
        common::derive(
            &self.rule,
            self.labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
        )
        .integer()
    }

    /// Its words, as an alarm by words puts them (§2.4) and the owner's
    /// record keeps them (§2.9).
    #[must_use]
    pub fn words(&self) -> Words {
        Words {
            rule: self.rule.clone(),
            labels: self.labels.clone(),
            summary: self.summary.clone(),
            category: self.category.clone(),
        }
    }

    /// The reading as a decimal, `None` for none.
    fn decimal(&self) -> Result<Option<Decimal>> {
        self.reading
            .as_deref()
            .map(|r| {
                Decimal::parse_input(r).ok_or_else(|| {
                    Error::Payload(format!(
                        "alarms.v2: the reading {r:?} of {} is no decimal (§4)",
                        self.rule
                    ))
                })
            })
            .transpose()
    }
}

impl From<crate::alarms::Condition> for Condition {
    /// An `alarms.v1` condition, with no reading.
    fn from(c: crate::alarms::Condition) -> Self {
        Self {
            rule: c.rule,
            labels: c.labels,
            severity: c.severity,
            summary: c.summary,
            category: c.category,
            reading: None,
        }
    }
}

/// The act an owner's call made (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    /// Put firing with a new `at`: no slot held it, or its record was
    /// resolved.
    Raised,
    /// Put firing at another severity, with a new `at`.
    Escalated,
    /// Put firing at the same severity with another argument, `at` kept;
    /// or, by words, its words alone put again.
    Refreshed,
    /// A refresh within the window of the last one put (§2.5's SHOULD
    /// NOT): nothing put. The reading is kept, and the next act puts it.
    Deferred,
    /// Nothing put: the same condition observed again (`scenarios-v2.md`
    /// §1).
    Unchanged,
    /// Put resolved with a new `at`, on its slot, and no delete.
    Resolved,
}

impl Act {
    /// A word for it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raised => "raised",
            Self::Escalated => "escalated",
            Self::Refreshed => "refreshed",
            Self::Deferred => "deferred",
            Self::Unchanged => "unchanged",
            Self::Resolved => "resolved",
        }
    }
}

/// What a call did: the alarm's alert key, its slot, the act, and the
/// record now on the slot, with its words for an alarm by words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub alert_key: String,
    pub slot: u8,
    pub act: Act,
    pub record: Record,
    /// The words on `words/<slot>`, for an alarm by words.
    pub words: Option<Words>,
}

/// One slot as the owner holds it: its alarm's alert key, the record put,
/// the alarm's words and its latest reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub slot: u8,
    pub alert_key: String,
    pub record: Record,
    pub words: Words,
    pub reading: Option<String>,
}

/// One slot as an owner's record keeps it (§2.9): the record as put, its
/// alarm's words and its alert key, a resolved one included, and the
/// reading its argument was written from. `record` and `words` are JSON as
/// [`Record::to_json`] and [`Words::to_json`] write them, so that an entry
/// that does not read stays one (§2.9, step 2).
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub slot: u8,
    pub record: serde_json::Value,
    pub words: serde_json::Value,
    /// 16 hexadecimal digits.
    pub alert_key: String,
    pub reading: Option<String>,
}

/// What an owner's record held at a restart.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    /// An `alarms.v2` owner's slots.
    V2(Vec<Kept>),
    /// An `alarms.v1` owner's documents, by alert key: a service that
    /// moves adopts each firing one onto a slot (§2.9, "From
    /// `alarms.v1`").
    V1(Vec<crate::alarms::Kept>),
}

/// The owner's own persistent record of the slots it holds (§2.9). The
/// application holds it, as [`crate::alarms::Record`] for `alarms.v1`: the
/// runtime loads it at start, and saves the slots held after every act,
/// whole.
pub trait OwnerRecord: Send + Sync + 'static {
    /// What the previous instance held, as the last [`OwnerRecord::save`]
    /// wrote it, or as a test or an `alarms.v1` owner wrote it.
    /// `Ok(None)`: there is no record. `Err` is logged, and read as none.
    fn load(&self) -> BoxFuture<'_, std::result::Result<Option<Found>, String>>;

    /// The slots held now, whole, after an act.
    fn save<'a>(&'a self, held: &'a [Kept]) -> BoxFuture<'a, std::result::Result<(), String>>;
}

/// An [`OwnerRecord`] in a JSON file: `{"profile": "alarms.v2", "slots":
/// {"<slot>": {"record": …, "words": …, "alert_key": "<16 hex>",
/// "reading"?: "<decimal>"}, …}}`. A file an `alarms.v1` owner's
/// [`crate::alarms::FileRecord`] wrote (`"profile": "alarms.v1"`) reads as
/// its documents, so a service that moves keeps its `--alarms-record`, and
/// the first save rewrites it as this one. A missing file is no record.
/// It is written whole, through a temporary file beside it, renamed.
#[derive(Debug, Clone)]
pub struct FileRecord {
    path: PathBuf,
}

impl FileRecord {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Its file.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The record's file as `held` makes it.
    #[must_use]
    pub fn render(held: &[Kept]) -> serde_json::Value {
        let mut slots = serde_json::Map::new();
        for k in held {
            let mut e = serde_json::Map::new();
            e.insert("record".into(), k.record.clone());
            e.insert("words".into(), k.words.clone());
            e.insert("alert_key".into(), k.alert_key.clone().into());
            if let Some(r) = &k.reading {
                e.insert("reading".into(), r.clone().into());
            }
            slots.insert(k.slot.to_string(), serde_json::Value::Object(e));
        }
        serde_json::json!({"profile": model::PROFILE, "slots": slots})
    }

    /// What a record's file holds: an `alarms.v2` record's slots, or an
    /// `alarms.v1` record's documents.
    ///
    /// # Errors
    ///
    /// The file is not JSON, holds neither `slots` nor `alarms` as an
    /// object, or a slot's name is no slot, or an entry is no object with
    /// `record`, `words` and `alert_key`.
    pub fn parse(text: &str) -> std::result::Result<Found, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        if v.get("profile").and_then(serde_json::Value::as_str) == Some(common::PROFILE)
            || (v.get("slots").is_none() && v.get("alarms").is_some())
        {
            return crate::alarms::FileRecord::parse(text).map(Found::V1);
        }
        let slots = v
            .get("slots")
            .and_then(serde_json::Value::as_object)
            .ok_or("no \"slots\" object")?;
        slots
            .iter()
            .map(|(name, e)| {
                let slot = model::parse_slot(name).ok_or_else(|| format!("{name:?} is no slot"))?;
                let member = |m: &str| {
                    e.get(m)
                        .cloned()
                        .ok_or_else(|| format!("slot {slot}: no {m:?}"))
                };
                Ok(Kept {
                    slot,
                    record: member("record")?,
                    words: member("words")?,
                    alert_key: member("alert_key")?
                        .as_str()
                        .ok_or_else(|| format!("slot {slot}: \"alert_key\" is no string"))?
                        .to_owned(),
                    reading: e
                        .get("reading")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect::<std::result::Result<_, String>>()
            .map(Found::V2)
    }
}

impl OwnerRecord for FileRecord {
    fn load(&self) -> BoxFuture<'_, std::result::Result<Option<Found>, String>> {
        Box::pin(async move {
            match tokio::fs::read_to_string(&self.path).await {
                Ok(text) => Self::parse(&text)
                    .map(Some)
                    .map_err(|e| format!("{}: {e}", self.path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(format!("{}: {e}", self.path.display())),
            }
        })
    }

    fn save<'a>(&'a self, held: &'a [Kept]) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let text = serde_json::to_vec_pretty(&Self::render(held)).map_err(|e| e.to_string())?;
            let mut tmp = self.path.clone().into_os_string();
            tmp.push(".tmp");
            let tmp = PathBuf::from(tmp);
            tokio::fs::write(&tmp, text)
                .await
                .map_err(|e| format!("{}: {e}", tmp.display()))?;
            tokio::fs::rename(&tmp, &self.path)
                .await
                .map_err(|e| format!("{}: {e}", self.path.display()))
        })
    }
}

/// Which rules an owner still evaluates (§2.5, §2.9 step 3): asked of
/// each firing alarm a restart finds, by its words.
pub type Evaluates = Arc<dyn Fn(&Words) -> bool + Send + Sync>;

/// What an owner starts from (§2.8, §2.9): its previous instance's record,
/// what it still evaluates, and its bound. The default adopts nothing,
/// evaluates every rule, and keeps the contract's 256 slots.
#[derive(Clone, Default)]
pub struct Startup {
    /// Its own persistent record, loaded at start and kept current.
    pub record: Option<Arc<dyn OwnerRecord>>,
    /// The alarms whose condition it still evaluates; `None`: every one.
    pub evaluates: Option<Evaluates>,
    /// The bound its descriptor lowers the contract's 256 to (core §3.3,
    /// §2.8), set before adoption.
    pub cardinality: Option<u64>,
}

impl std::fmt::Debug for Startup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Startup")
            .field("record", &self.record.is_some())
            .field("evaluates", &self.evaluates.is_some())
            .field("cardinality", &self.cardinality)
            .finish()
    }
}

impl Startup {
    /// With `record` as its persistent record.
    #[must_use]
    pub fn with_record(mut self, record: Arc<dyn OwnerRecord>) -> Self {
        self.record = Some(record);
        self
    }

    /// Evaluating exactly the rules named.
    #[must_use]
    pub fn evaluating<S: Into<String>>(mut self, rules: impl IntoIterator<Item = S>) -> Self {
        let rules: std::collections::BTreeSet<String> = rules.into_iter().map(Into::into).collect();
        self.evaluates = Some(Arc::new(move |w: &Words| rules.contains(&w.rule)));
        self
    }

    /// Evaluating the alarms `f` accepts.
    #[must_use]
    pub fn evaluating_with(mut self, f: impl Fn(&Words) -> bool + Send + Sync + 'static) -> Self {
        self.evaluates = Some(Arc::new(f));
        self
    }

    /// With the bound lowered to `n` slots.
    #[must_use]
    pub fn with_cardinality(mut self, n: u64) -> Self {
        self.cardinality = Some(n);
        self
    }
}

/// Where a restart's alarms came from (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// The owner's own `alarms.v2` record.
    Record,
    /// An `alarms.v1` owner's record: the service moved (§2.9).
    RecordV1,
    /// None: nothing adopted.
    #[default]
    Nothing,
}

impl Source {
    /// A word for it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::RecordV1 => "record_v1",
            Self::Nothing => "nothing",
        }
    }
}

/// What a restart did with one entry of the record (§2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Took {
    /// The slot it was on, or, from `alarms.v1`, the one §2.2 gave it;
    /// `None` for a document put on none.
    pub slot: Option<u8>,
    /// Its alert key, as the record holds it.
    pub alert_key: String,
    /// The model's step: `left` past the bound, `free`, `resolved` or
    /// `adopted` ([`model::Adoption`]).
    pub step: model::Adoption,
}

/// What a restart did with what it found (§2.9).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Adoption {
    pub source: Source,
    /// Each entry, in the record's order: by slot, or by alert key from
    /// `alarms.v1`.
    pub took: Vec<Took>,
    /// The entries whose put failed on the bus, and why: kept in the
    /// record for the next start, and not held.
    pub failed: Vec<(String, String)>,
}

impl Adoption {
    /// The slots of the entries that took `step`.
    #[must_use]
    pub fn slots(&self, step: model::Adoption) -> Vec<u8> {
        self.took
            .iter()
            .filter(|t| t.step == step)
            .filter_map(|t| t.slot)
            .collect()
    }
}

/// A service's `alarms.v2` (§2.2–§2.9), from
/// [`crate::ServiceBuilder::alarms_v2`], and again from
/// [`crate::Service::alarms_v2`] once it runs.
///
/// A clone is the same handle. Calls are serialized: each finishes its puts
/// before the next begins. A record carries no Zenoh timestamp of the
/// owner's (`freshness.v2` §2.4), so the clock guard holds none of its
/// puts, except on a session that stamps every put (core §8.2), where a
/// call that puts returns [`Error::ClockAhead`] while it holds.
#[derive(Clone)]
pub struct Alarms {
    inner: Arc<Inner>,
}

struct Inner {
    iface: IfaceId,
    alarms: Resource,
    words: Resource,
    session: zenoh::Session,
    addr: Addr,
    store: Arc<Store>,
    minter: Arc<Minter>,
    closed: tokio::sync::watch::Receiver<bool>,
    budget: Option<Arc<Budget>>,
    words_budget: Option<Arc<Budget>>,
    packed: PackedType,
    epoch: i64,
    /// Half the records' horizon: §2.5's refresh window, 450 s at the
    /// standard contract.
    window: Duration,
    record: Option<Arc<dyn OwnerRecord>>,
    /// The instance's table (§4), as the contracts implemented so far
    /// give it.
    table: Mutex<Option<Vec<Rule>>>,
    held: tokio::sync::Mutex<Held>,
    adoption: Mutex<Adoption>,
}

/// One slot's alarm, as held.
struct Entry {
    id: u64,
    /// The record as put.
    record: Record,
    /// The rule it was written by, for a table's alarm: its argument's
    /// declaration (§4).
    rule: Option<Rule>,
    /// The alarm's words as the owner has them, and its latest reading.
    words: Words,
    reading: Option<String>,
    /// The words last put on `words/<slot>` for it, an alarm by words.
    words_put: Option<Words>,
    dating: model::Dating,
    /// When its last refresh was put (§2.5's window).
    refreshed: Option<Instant>,
}

impl Entry {
    fn slot_view(&self, slot: u8) -> Slot {
        Slot {
            slot,
            alert_key: common::key_hex(self.id),
            record: self.record,
            words: self.words.clone(),
            reading: self.reading.clone(),
        }
    }

    fn kept(&self, slot: u8) -> Kept {
        Kept {
            slot,
            record: self.record.to_json(),
            words: self.words.to_json(),
            alert_key: common::key_hex(self.id),
            reading: self.reading.clone(),
        }
    }

    /// The fields a record of it carries for `argument`.
    fn fields(&self, argument: Argument) -> Fields {
        Fields {
            rule: self.record.rule,
            labels: self.record.labels,
            argument: if self.record.is_by_words() {
                Argument::NotAvailable
            } else {
                argument
            },
            key: self.record.key,
        }
    }

    /// The argument its latest reading writes, by its rule.
    fn argument(&self) -> Argument {
        let reading = self.reading.as_deref().and_then(Decimal::parse_input);
        self.rule
            .as_ref()
            .map_or(Argument::NotAvailable, |r| r.argument(reading.as_ref()))
    }
}

struct Held {
    slots: model::Slots<u64>,
    entries: BTreeMap<u8, Entry>,
    /// The records' writers, by slot, kept while the instance runs.
    writers: BTreeMap<u8, Arc<StateWriter>>,
    words_writers: BTreeMap<u8, Arc<StateWriter>>,
    /// The greatest `at` of identities no longer held, at most [`PAST`].
    past: BTreeMap<u64, model::Dating>,
    /// Entries a restart found and could not put, or left past the bound:
    /// not held, kept in the record for a later start.
    unsettled: BTreeMap<u8, Kept>,
}

impl Held {
    fn forget(&mut self, id: u64, dating: model::Dating) {
        self.past.insert(id, dating);
        if self.past.len() > PAST
            && let Some(oldest) = self
                .past
                .iter()
                .min_by_key(|(_, d)| d.greatest())
                .map(|(k, _)| *k)
        {
            self.past.remove(&oldest);
        }
    }

    fn kept(&self) -> Vec<Kept> {
        let mut all: BTreeMap<u8, Kept> = self.unsettled.clone();
        all.extend(self.entries.iter().map(|(s, e)| (*s, e.kept(*s))));
        all.into_values().collect()
    }
}

/// An entry of the record that reads (§2.9): its record, its words and
/// its alert key.
type Read = (Record, Words, u64);

/// A stamp's time in milliseconds since the Unix epoch.
fn ms(t: &zenoh::time::Timestamp) -> i128 {
    let since = t
        .get_time()
        .to_system_time()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i128::try_from(since.as_millis()).unwrap_or(i128::MAX)
}

fn payload_error(e: impl std::fmt::Display) -> Error {
    Error::Payload(format!("alarms.v2: {e} (§2.4)"))
}

impl Alarms {
    /// Implements `alarms.v2` on `b` from `imp`, then adopts the previous
    /// instance's alarms (§2.9).
    pub(crate) async fn declare(
        b: &mut ServiceBuilder,
        imp: Implementation,
        startup: Startup,
    ) -> Result<Self> {
        let addr = b.address()?.clone();
        if imp.iface() != &iface() {
            return Err(Error::Contract(format!(
                "{} is not alarms.v2: ServiceBuilder::alarms_v2_with implements a revision of \
                 alarms.v2",
                imp.iface()
            )));
        }
        let alarms = imp.resource(ALARMS)?.clone();
        let words = imp.resource(WORDS)?.clone();
        let epoch = b.deployment_epoch();
        let packed = PackedType::of(imp.contract(), &alarms, epoch)?;
        let json = match &words.body {
            Body::Data(d) => matches!(d.type_, TypeId::JsonSchema { .. }),
            Body::Operation(_) => false,
        };
        let (Some(packed), KindToken::State, KindToken::State, true) =
            (packed, alarms.token, words.token, json)
        else {
            return Err(Error::Contract(format!(
                "alarms.v2 {ALARMS:?} is a packed state and {WORDS:?} a JSON state in every \
                 revision of the major (§3)"
            )));
        };
        let Some(window) = zenkey_model::freshness::horizon_of(&alarms)
            .refresh_bound()
            .filter(|b| !b.is_zero())
        else {
            return Err(Error::Contract(format!(
                "alarms.v2 {ALARMS:?} declares no freshness.ttl_s above 0, so no alarm of it \
                 would be confirmed (§2.7)"
            )));
        };
        let iface = imp.iface().clone();
        b.implement(imp)?;
        b.expose(&iface, ALARMS)?;
        b.expose(&iface, WORDS)?;
        b.serve_state(&iface)?;
        if let Some(n) = startup.cardinality {
            b.cardinality(&iface, ALARMS, n)?;
        }
        let budget = b.budget_of(&iface, ALARMS);
        let words_budget = b.budget_of(&iface, WORDS);
        let bound = budget
            .as_ref()
            .and_then(|b| b.bound().0.limit())
            .unwrap_or(u64::from(model::MAX_SLOTS));
        let bound = u16::try_from(bound).unwrap_or(model::MAX_SLOTS);
        let (session, store, minter, closed) = b.state_parts();
        let a = Self {
            inner: Arc::new(Inner {
                iface,
                alarms,
                words,
                session,
                addr,
                store,
                minter,
                closed,
                budget,
                words_budget,
                packed,
                epoch,
                window,
                record: startup.record.clone(),
                table: Mutex::new(b.alarms_table()),
                held: tokio::sync::Mutex::new(Held {
                    slots: model::Slots::new(bound),
                    entries: BTreeMap::new(),
                    writers: BTreeMap::new(),
                    words_writers: BTreeMap::new(),
                    past: BTreeMap::new(),
                    unsettled: BTreeMap::new(),
                }),
                adoption: Mutex::new(Adoption::default()),
            }),
        };
        a.adopt(&startup).await?;
        b.alarms_v2 = Some(a.clone());
        Ok(a)
    }

    /// `alarms.v2`.
    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.inner.iface
    }

    /// The most alarms this owner holds firing: its slots, the contract's
    /// 256 or the bound its descriptor lowers it to (§2.8).
    pub async fn max_alarms(&self) -> u16 {
        self.inner.held.lock().await.slots.bound()
    }

    /// The instance's table (§4): the one `alarms.rules` among the
    /// contracts the service implements, `None` where none holds one or
    /// two hold two different lists.
    #[must_use]
    pub fn table(&self) -> Option<Vec<Rule>> {
        self.inner.table.lock().expect("not poisoned").clone()
    }

    /// §4: the instance's table is `table`, read again from the contracts
    /// the service implements so far.
    pub(crate) fn retable(&self, table: Option<Vec<Rule>>) {
        *self.inner.table.lock().expect("not poisoned") = table;
    }

    /// What the restart did (§2.9).
    #[must_use]
    pub fn adoption(&self) -> Adoption {
        self.inner.adoption.lock().expect("not poisoned").clone()
    }

    /// The slots held now: firing, or resolved until another identity
    /// takes the slot.
    pub async fn held(&self) -> BTreeMap<u8, Slot> {
        let st = self.inner.held.lock().await;
        st.entries
            .iter()
            .map(|(s, e)| (*s, e.slot_view(*s)))
            .collect()
    }

    /// The slot that holds the alarm of `alert_key`.
    pub async fn get(&self, alert_key: &str) -> Option<Slot> {
        let id = common::key_integer(alert_key)?;
        let st = self.inner.held.lock().await;
        let (slot, _) = st.slots.slot_of(&id)?;
        st.entries.get(&slot).map(|e| e.slot_view(slot))
    }

    /// The condition holds, as `c` says (§2.5): the act it makes on its
    /// alarm, from what its slot holds.
    /// - no slot holds it, or its record is resolved: a **raise**, firing,
    ///   with a new `at`, on the slot §2.2 gives it;
    /// - held firing at another severity: an **escalation**, a new `at`;
    /// - held firing at that severity with another argument, or, by words,
    ///   other words: a **refresh**, `at` kept; **deferred** when the last
    ///   refresh put was within the window (§2.5's SHOULD NOT);
    /// - otherwise **unchanged**, nothing put.
    ///
    /// Each is an evaluation, so the alarm's confirmation is turned back
    /// on (§2.7). A raise with every slot firing is refused,
    /// [`Error::Budget`], nothing put and nothing resolved (§2.8). A
    /// reading that is no decimal is [`Error::Payload`].
    pub async fn raise(&self, c: Condition) -> Result<Done> {
        let reading = c.decimal()?;
        let id = c.identity();
        let words = c.words();
        let mut st = self.inner.held.lock().await;
        let clock = self.now_ms();
        let epoch = self.inner.epoch;
        match st.slots.slot_of(&id) {
            Some((slot, State::Firing)) => {
                let e = st.entries.get(&slot).expect("a slot held");
                let writer = Arc::clone(&st.writers[&slot]);
                let argument = e
                    .rule
                    .as_ref()
                    .map_or(Argument::NotAvailable, |r| r.argument(reading.as_ref()));
                let escalate = Some(c.severity) != e.record.severity;
                let moved = !e.record.is_by_words() && argument != e.record.argument;
                let reworded = e.record.is_by_words() && e.words_put.as_ref() != Some(&words);
                let act = if escalate {
                    Act::Escalated
                } else if moved || reworded {
                    if e.refreshed.is_some_and(|t| t.elapsed() < self.inner.window) {
                        Act::Deferred
                    } else {
                        Act::Refreshed
                    }
                } else {
                    Act::Unchanged
                };
                let mut dating = e.dating;
                let record = match act {
                    Act::Escalated => Record::new(
                        State::Firing,
                        c.severity,
                        e.fields(argument),
                        dating.transition(clock, epoch),
                    ),
                    Act::Refreshed if moved => Record {
                        argument,
                        ..e.record
                    },
                    _ => e.record,
                };
                let by_words = e.record.is_by_words();
                let put_words = by_words
                    && e.words_put.as_ref() != Some(&words)
                    && matches!(act, Act::Escalated | Act::Refreshed);
                let put_record = matches!(act, Act::Escalated) || (act == Act::Refreshed && moved);
                if put_words {
                    self.put_words(&mut st, slot, &words).await?;
                }
                if put_record {
                    let bytes = record.write(epoch).map_err(payload_error)?;
                    writer.put(bytes.bytes).await?;
                }
                writer.set_vouched(true);
                let e = st.entries.get_mut(&slot).expect("a slot held");
                if put_words {
                    e.words_put = Some(words.clone());
                }
                if act == Act::Refreshed {
                    e.refreshed = Some(Instant::now());
                }
                e.record = record;
                e.dating = dating;
                e.words = words;
                e.reading = c.reading.clone();
                let done = Done {
                    alert_key: common::key_hex(id),
                    slot,
                    act,
                    record,
                    words: e.words_put.clone(),
                };
                self.save(&st).await;
                Ok(done)
            }
            held => {
                let before = st.slots.clone();
                let slot = match st.slots.raise(&id) {
                    model::Raise::Put(slot) => slot,
                    model::Raise::Firing => unreachable!("not held firing"),
                    model::Raise::Refused => return Err(self.refused(&st, &c)),
                };
                let raised = self
                    .take(
                        &mut st,
                        slot,
                        id,
                        &c,
                        reading.as_ref(),
                        held.is_some(),
                        clock,
                    )
                    .await;
                match raised {
                    Ok(done) => {
                        self.save(&st).await;
                        Ok(done)
                    }
                    Err(e) => {
                        st.slots = before;
                        Err(e)
                    }
                }
            }
        }
    }

    /// A raise of `id` on `slot`: its own resolved record there when
    /// `own`, else a slot it takes, from nothing or from another identity's
    /// resolved record.
    #[allow(clippy::too_many_arguments)]
    async fn take(
        &self,
        st: &mut Held,
        slot: u8,
        id: u64,
        c: &Condition,
        reading: Option<&Decimal>,
        own: bool,
        clock: i128,
    ) -> Result<Done> {
        let epoch = self.inner.epoch;
        let words = c.words();
        // §2.5 "what stays": an identity's fields are fixed on its slot; a
        // slot taken is written by the table in force now.
        let (fields, rule, mut dating, words_put, refreshed) = match st.entries.get(&slot) {
            Some(e) if own => {
                let argument = e
                    .rule
                    .as_ref()
                    .map_or(Argument::NotAvailable, |r| r.argument(reading));
                (
                    e.fields(argument),
                    e.rule.clone(),
                    e.dating,
                    e.words_put.clone(),
                    e.refreshed,
                )
            }
            _ => {
                let table = self.table();
                let fields = model::write_alarm(
                    table.as_deref(),
                    &c.rule,
                    c.labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
                    reading,
                );
                let rule = match (fields.rule, &table) {
                    (RuleIndex::Table(i), Some(t)) => t.get(usize::from(i)).cloned(),
                    _ => None,
                };
                let dating = st.past.get(&id).copied().unwrap_or_default();
                (fields, rule, dating, None, None)
            }
        };
        let record = Record::new(
            State::Firing,
            c.severity,
            fields,
            dating.transition(clock, epoch),
        );
        let bytes = record.write(epoch).map_err(payload_error)?;
        let by_words = record.is_by_words();
        let put_words = by_words && words_put.as_ref() != Some(&words);
        if put_words {
            self.put_words(st, slot, &words).await?;
        }
        let writer = self.writer_of(st, slot).await?;
        writer.put(bytes.bytes).await?;
        writer.set_vouched(true);
        if let Some(old) = st.entries.remove(&slot)
            && old.id != id
        {
            st.forget(old.id, old.dating);
        }
        st.past.remove(&id);
        st.unsettled.remove(&slot);
        let words_put = if put_words {
            Some(words.clone())
        } else {
            words_put.filter(|_| by_words)
        };
        st.entries.insert(
            slot,
            Entry {
                id,
                record,
                rule,
                words,
                reading: c.reading.clone(),
                words_put: words_put.clone(),
                dating,
                refreshed,
            },
        );
        Ok(Done {
            alert_key: common::key_hex(id),
            slot,
            act: Act::Raised,
            record,
            words: words_put,
        })
    }

    /// The condition of the alarm of `alert_key` no longer holds, or the
    /// owner will evaluate it no more while it runs (§2.5): its resolved
    /// record is put on its slot, with a new `at`, its severity the one it
    /// last fired at and its argument the latest reading, and no delete
    /// follows. The slot is free for another identity at once, and keeps
    /// the record until one takes it. `None` when no firing alarm has that
    /// alert key.
    pub async fn resolve(&self, alert_key: &str) -> Result<Option<Done>> {
        let Some(id) = common::key_integer(alert_key) else {
            return Ok(None);
        };
        let mut st = self.inner.held.lock().await;
        let clock = self.now_ms();
        let Some((slot, State::Firing)) = st.slots.slot_of(&id) else {
            return Ok(None);
        };
        let writer = Arc::clone(&st.writers[&slot]);
        let e = st.entries.get(&slot).expect("a slot held");
        let mut dating = e.dating;
        let record = Record {
            state: State::Resolved,
            at: dating.transition(clock, self.inner.epoch),
            argument: if e.record.is_by_words() {
                Argument::NotAvailable
            } else {
                e.argument()
            },
            ..e.record
        };
        let bytes = record.write(self.inner.epoch).map_err(payload_error)?;
        // §2.7: a resolved record is never confirmed, so its bit is clear
        // before its put. A put that fails leaves the firing alarm's bit as
        // it was.
        let vouched = writer.is_vouched();
        writer.set_vouched(false);
        if let Err(err) = writer.put(bytes.bytes).await {
            writer.set_vouched(vouched);
            return Err(err);
        }
        st.slots.resolve(&id);
        let e = st.entries.get_mut(&slot).expect("a slot held");
        e.record = record;
        e.dating = dating;
        let done = Done {
            alert_key: alert_key.to_owned(),
            slot,
            act: Act::Resolved,
            record,
            words: e.words_put.clone(),
        };
        self.save(&st).await;
        Ok(Some(done))
    }

    /// Turns the confirmation of the firing alarm of `alert_key` off, or
    /// back on (§2.7): its bit in the heartbeat covering `alarms.v2`. An
    /// owner that has stopped evaluating its condition lets it go stale
    /// rather than confirm what it no longer sees. `false`, nothing
    /// changed, when no firing alarm has that alert key.
    pub async fn set_confirming(&self, alert_key: &str, on: bool) -> bool {
        let Some(id) = common::key_integer(alert_key) else {
            return false;
        };
        let st = self.inner.held.lock().await;
        match st.slots.slot_of(&id) {
            Some((slot, State::Firing)) => {
                st.writers[&slot].set_vouched(on);
                true
            }
            _ => false,
        }
    }

    /// [`Alarms::set_confirming`] for every firing alarm held.
    pub async fn set_confirming_all(&self, on: bool) {
        let st = self.inner.held.lock().await;
        for (slot, _, state) in st.slots.held() {
            if state == State::Firing {
                st.writers[&slot].set_vouched(on);
            }
        }
    }

    /// Whether the alarm of `alert_key` is confirmed by its owner's
    /// heartbeat: firing, and [`Alarms::set_confirming`] has not turned
    /// that off; `None` when no slot holds it.
    pub async fn is_confirming(&self, alert_key: &str) -> Option<bool> {
        let id = common::key_integer(alert_key)?;
        let st = self.inner.held.lock().await;
        let (slot, state) = st.slots.slot_of(&id)?;
        Some(state == State::Firing && st.writers[&slot].is_vouched())
    }

    /// The record's writer of the slot `slot`, once a record was put on
    /// it: zenoh stays reachable.
    pub async fn with_writer<T>(&self, slot: u8, f: impl FnOnce(&StateWriter) -> T) -> Option<T> {
        let st = self.inner.held.lock().await;
        st.writers.get(&slot).map(|w| f(w))
    }

    /// §2.9, a re-mint: the new instance holds what it keeps. Each firing
    /// record is put again on its slot, unchanged, `at` kept, under a fresh
    /// stamp, before the new instance's tokens; each resolved record is
    /// one the new instance does not hold, so it is put again nowhere, and
    /// its slot is free: the owner's answers leave it out from here on. A
    /// failure stops the loop and is returned, and
    /// [`crate::Service::new_epoch`] then mints nothing.
    pub(crate) async fn new_instance(&self) -> Result<()> {
        let mut st = self.inner.held.lock().await;
        let resolved: Vec<u8> = st
            .slots
            .held()
            .filter(|(_, _, s)| *s == State::Resolved)
            .map(|(slot, _, _)| slot)
            .collect();
        for slot in resolved {
            self.free(&mut st, slot);
        }
        let firing: Vec<Arc<StateWriter>> = st
            .slots
            .held()
            .map(|(slot, _, _)| Arc::clone(&st.writers[&slot]))
            .collect();
        for w in firing {
            w.reput().await?;
        }
        self.save(&st).await;
        Ok(())
    }

    /// The instance holds nothing on `slot` any more: its record and its
    /// words are left out of the owner's answers, and the member is no
    /// longer live (core §2.7). Nothing is put or deleted.
    fn free(&self, st: &mut Held, slot: u8) {
        st.slots.forget(slot);
        if let Some(e) = st.entries.remove(&slot) {
            st.forget(e.id, e.dating);
        }
        for w in [st.writers.remove(&slot), st.words_writers.remove(&slot)]
            .into_iter()
            .flatten()
        {
            self.inner.store.forget(w.key_expr().as_str());
            w.writer().release();
        }
    }

    fn now_ms(&self) -> i128 {
        ms(&self.inner.minter.now())
    }

    /// The refusal of a raise with every slot firing (§2.8, core §2.7).
    fn refused(&self, st: &Held, c: &Condition) -> Error {
        let bound = u64::from(st.slots.bound());
        let source = self
            .inner
            .budget
            .as_ref()
            .map_or(crate::budget::Source::Contract, |b| b.bound().1);
        Error::Budget(Box::new(Refusal {
            iface: self.inner.iface.clone(),
            resource: ALARMS.to_owned(),
            member: format!("the alarm {} ({})", c.alert_key(), c.rule),
            breach: Breach::Bound {
                bound,
                source,
                live: bound,
            },
        }))
    }

    /// The record's writer of `slot`, declared at its first put.
    async fn writer_of(&self, st: &mut Held, slot: u8) -> Result<Arc<StateWriter>> {
        if let Some(w) = st.writers.get(&slot) {
            return Ok(Arc::clone(w));
        }
        let i = &self.inner;
        let key = alarm_key(i.addr.system.as_str(), i.addr.service.as_str(), slot)?;
        let w = Arc::new(
            self.writer(
                &i.alarms,
                key,
                slot,
                i.budget.clone(),
                Some(i.packed.clone()),
            )
            .await?,
        );
        st.writers.insert(slot, Arc::clone(&w));
        Ok(w)
    }

    /// Puts `words` on `words/<slot>` (§2.4), before the slot's record.
    async fn put_words(&self, st: &mut Held, slot: u8, words: &Words) -> Result<()> {
        let w = match st.words_writers.get(&slot) {
            Some(w) => Arc::clone(w),
            None => {
                let i = &self.inner;
                let key = words_key(i.addr.system.as_str(), i.addr.service.as_str(), slot)?;
                let w = Arc::new(
                    self.writer(&i.words, key, slot, i.words_budget.clone(), None)
                        .await?,
                );
                st.words_writers.insert(slot, Arc::clone(&w));
                w
            }
        };
        // §2.3: words that break the framing are put, sanitized by every
        // reader, and said.
        if let Err(p) = common::alert_key(
            &words.rule,
            words.labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
        ) {
            tracing::warn!(
                alert_key = %common::key_hex(words.derive().integer()),
                "alarms.v2: these words break the framing, so a reader sanitizes them: {p} (§2.3)"
            );
        }
        let payload = w.writer().encode(&words.to_json())?;
        w.put(payload).await?;
        Ok(())
    }

    /// A writer on the member `key` of `resource`, as the service's own
    /// state writers are made: stamped, answered by its state queryables,
    /// within the resource's budget (core §2.7).
    async fn writer(
        &self,
        resource: &Resource,
        key: zenkey_model::grammar::Key,
        slot: u8,
        budget: Option<Arc<Budget>>,
        packed: Option<PackedType>,
    ) -> Result<StateWriter> {
        let i = &self.inner;
        let values: Bindings = [(PARAM.to_owned(), vec![slot.to_string()])].into();
        let w = Writer::declare(&i.session, &i.minter, key.into_keyexpr(), resource, &values)
            .await?
            .bounded(budget)
            .packed(packed);
        Ok(StateWriter::new(
            w,
            Arc::clone(&i.store),
            Arc::clone(&i.minter),
            &zenkey_model::freshness::horizon_of(resource),
            i.closed.clone(),
        ))
    }

    /// Saves the slots held into the record, if any; a failure is logged.
    async fn save(&self, st: &Held) {
        let Some(r) = &self.inner.record else {
            return;
        };
        if let Err(e) = r.save(&st.kept()).await {
            tracing::warn!(
                "alarms.v2: the record was not saved, so a restart may not adopt what this \
                 instance holds (§2.9): {e}"
            );
        }
    }

    /// The previous instance's record (§2.9), or nothing.
    async fn load(&self, startup: &Startup) -> Option<Found> {
        let r = startup.record.as_ref()?;
        match r.load().await {
            Ok(found) => found,
            Err(e) => {
                tracing::warn!(
                    "alarms.v2: the record could not be read, so it is read as none (§2.9): {e}"
                );
                None
            }
        }
    }

    /// §2.9: the record's slots, or an `alarms.v1` record's documents,
    /// taken over before any other put of this instance.
    async fn adopt(&self, startup: &Startup) -> Result<()> {
        let evaluates = |w: &Words| startup.evaluates.as_ref().is_none_or(|f| f(w));
        let report = match self.load(startup).await {
            None => Adoption::default(),
            Some(Found::V2(kept)) => self.adopt_slots(kept, &evaluates).await,
            Some(Found::V1(docs)) => self.adopt_v1(docs, &evaluates).await,
        };
        if report.source != Source::Nothing {
            tracing::info!(
                source = report.source.as_str(),
                adopted = report.slots(model::Adoption::Adopted).len(),
                resolved = report.slots(model::Adoption::Resolved).len(),
                failed = report.failed.len(),
                "alarms.v2: the previous instance's alarms, taken over (§2.9)"
            );
            let st = self.inner.held.lock().await;
            self.save(&st).await;
        }
        *self.inner.adoption.lock().expect("not poisoned") = report;
        Ok(())
    }

    /// §2.9's steps over an `alarms.v2` record, by slot.
    async fn adopt_slots(
        &self,
        kept: Vec<Kept>,
        evaluates: &(dyn Fn(&Words) -> bool + Sync),
    ) -> Adoption {
        let mut report = Adoption {
            source: Source::Record,
            ..Adoption::default()
        };
        let mut found: BTreeMap<u8, (Kept, Option<Read>)> = BTreeMap::new();
        for k in kept {
            let read = (|| {
                let record = Record::of_json(&k.record)?;
                record.check().ok()?;
                let words = Words::from_json(&k.words).ok()?;
                let id = common::key_integer(&k.alert_key)?;
                Some((record, words, id))
            })();
            found.insert(k.slot, (k, read));
        }
        let left = found.iter().map(|(slot, (_, read))| {
            let entry = match read {
                Some((record, words, id)) => model::Left {
                    id: *id,
                    state: record.state,
                    reads: true,
                    derives: words.derive().integer() == *id && record.key.is_none_or(|k| k == *id),
                    evaluated: evaluates(words),
                },
                None => model::Left {
                    id: u64::MAX,
                    state: State::Resolved,
                    reads: false,
                    derives: false,
                    evaluated: false,
                },
            };
            (*slot, entry)
        });
        let mut st = self.inner.held.lock().await;
        let (slots, did) = model::Slots::restart(left, st.slots.bound());
        st.slots = slots;
        for (slot, step) in did {
            let (k, read) = found.remove(&slot).expect("each slot found");
            report.took.push(Took {
                slot: Some(slot),
                alert_key: k.alert_key.clone(),
                step,
            });
            let put = match (step, read) {
                (model::Adoption::Left, _) => {
                    st.unsettled.insert(slot, k);
                    continue;
                }
                (model::Adoption::Free, _) | (_, None) => continue,
                (step, Some((record, words, id))) => {
                    self.settle(&mut st, slot, step, id, &record, words, k.reading.clone())
                        .await
                }
            };
            if let Err(e) = put {
                self.unsettled(&mut st, &mut report, slot, k, &e);
            }
        }
        drop(st);
        report
    }

    /// §2.9, "From `alarms.v1`": each firing document onto the slot §2.2
    /// gives it, in the record's order, `at` its `timestamp`'s second.
    async fn adopt_v1(
        &self,
        docs: Vec<crate::alarms::Kept>,
        evaluates: &(dyn Fn(&Words) -> bool + Sync),
    ) -> Adoption {
        let mut report = Adoption {
            source: Source::RecordV1,
            ..Adoption::default()
        };
        let mut st = self.inner.held.lock().await;
        for k in docs {
            let doc = common::decode(&k.payload)
                .ok()
                .filter(|d| d.state == State::Firing && d.alert_key() == k.alert_key);
            let Some(doc) = doc else {
                report.took.push(Took {
                    slot: None,
                    alert_key: k.alert_key,
                    step: model::Adoption::Free,
                });
                continue;
            };
            let words = Words {
                rule: doc.rule.clone(),
                labels: doc.labels.clone(),
                summary: doc.summary.clone(),
                category: doc.category.clone(),
            };
            let id = words.derive().integer();
            let step = if evaluates(&words) {
                model::Adoption::Adopted
            } else {
                model::Adoption::Resolved
            };
            let slot = match st.slots.raise(&id) {
                model::Raise::Put(slot) => slot,
                model::Raise::Firing => {
                    // Two documents of one identity: the first is held.
                    report.took.push(Took {
                        slot: None,
                        alert_key: k.alert_key,
                        step: model::Adoption::Free,
                    });
                    continue;
                }
                model::Raise::Refused => {
                    tracing::warn!(
                        alert_key = %k.alert_key,
                        "alarms.v2: a firing alarm of the alarms.v1 instance is past the bound, \
                         so it is neither adopted nor resolved, and the record no longer holds \
                         it (§2.8, §2.9)"
                    );
                    report.took.push(Took {
                        slot: None,
                        alert_key: k.alert_key,
                        step: model::Adoption::Left,
                    });
                    continue;
                }
            };
            if step == model::Adoption::Resolved {
                st.slots.resolve(&id);
            }
            report.took.push(Took {
                slot: Some(slot),
                alert_key: k.alert_key.clone(),
                step,
            });
            let record = Record::new(
                State::Firing,
                doc.severity,
                Fields {
                    rule: RuleIndex::NotAvailable,
                    labels: None,
                    argument: Argument::NotAvailable,
                    key: Some(id),
                },
                model::adopted_at(doc.timestamp, self.inner.epoch),
            );
            if let Err(e) = self
                .settle(&mut st, slot, step, id, &record, words, None)
                .await
            {
                st.slots.forget(slot);
                tracing::warn!(
                    alert_key = %k.alert_key,
                    "alarms.v2: an alarms.v1 document could not take its step, and the record \
                     no longer holds it (§2.9): {e}"
                );
                report.failed.push((k.alert_key, e.to_string()));
            }
        }
        drop(st);
        report
    }

    /// One adopted slot's put (§2.9, steps 3 and 4): the alarm written
    /// again by this instance's table where it names its words, else by
    /// words (by name, never by index), its severity kept; firing with its
    /// `at` kept, or resolved with a new one.
    #[allow(clippy::too_many_arguments)]
    async fn settle(
        &self,
        st: &mut Held,
        slot: u8,
        step: model::Adoption,
        id: u64,
        found: &Record,
        words: Words,
        reading: Option<String>,
    ) -> Result<()> {
        let epoch = self.inner.epoch;
        let table = self.table();
        let decimal = reading.as_deref().and_then(Decimal::parse_input);
        let fields = model::write_alarm(
            table.as_deref(),
            &words.rule,
            words.labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
            decimal.as_ref(),
        );
        let rule = match (fields.rule, &table) {
            (RuleIndex::Table(i), Some(t)) => t.get(usize::from(i)).cloned(),
            _ => None,
        };
        let mut dating = model::Dating::known(found.at);
        let resolved = step == model::Adoption::Resolved;
        let record = Record {
            state: if resolved {
                State::Resolved
            } else {
                State::Firing
            },
            severity: found.severity,
            rule: fields.rule,
            at: if resolved {
                dating.transition(self.now_ms(), epoch)
            } else {
                found.at
            },
            labels: fields.labels,
            argument: fields.argument,
            key: fields.key,
        };
        let bytes = record.write(epoch).map_err(payload_error)?;
        let by_words = record.is_by_words();
        if by_words {
            self.put_words(st, slot, &words).await?;
        }
        let writer = self.writer_of(st, slot).await?;
        if resolved {
            writer.set_vouched(false);
        }
        writer.put(bytes.bytes).await?;
        st.entries.insert(
            slot,
            Entry {
                id,
                record,
                rule,
                words_put: by_words.then(|| words.clone()),
                words,
                reading,
                dating,
                refreshed: None,
            },
        );
        Ok(())
    }

    /// An entry whose put failed: kept for the record, not held, and
    /// reported (§2.9).
    fn unsettled(&self, st: &mut Held, report: &mut Adoption, slot: u8, k: Kept, e: &Error) {
        tracing::warn!(
            slot,
            alert_key = %k.alert_key,
            "alarms.v2: a slot of the previous instance could not take its step, so it stays \
             in the record for the next start (§2.9): {e}"
        );
        st.slots.forget(slot);
        st.entries.remove(&slot);
        report.failed.push((k.alert_key.clone(), e.to_string()));
        st.unsettled.insert(slot, k);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BUNDLE, Condition, FINGERPRINT, FileRecord, Found, Kept, Severity, implementation,
    };

    /// The embedded bundle is the published revision 2.0, its records
    /// packed under tag 11, its alarms' horizon 900 s and its bound 256.
    #[test]
    fn the_contract_is_the_published_bundle() {
        let imp = implementation();
        assert_eq!(imp.fingerprint().hex().to_string(), FINGERPRINT);
        assert_eq!(&**imp.bundle_bytes(), BUNDLE, "served byte for byte");
        assert!(zenkey_model::freshness::uses_v2(imp.contract()));
        let r = imp.resource(super::ALARMS).unwrap();
        assert_eq!(
            zenkey_model::freshness::horizon_of(r).refresh_bound(),
            Some(std::time::Duration::from_secs(450))
        );
        assert_eq!(r.cardinality, Some(256));
        let packed = super::PackedType::of(imp.contract(), r, 0)
            .unwrap()
            .expect("a packed type");
        assert_eq!(packed.tag().get(), zenkey_model::alarms::v2::TAG);
        assert!(imp.resource(super::WORDS).is_ok());
    }

    /// §2.2: a condition's alert key is v1's recipe's, host-scoped labels
    /// aside.
    #[test]
    fn a_conditions_key_is_v1s() {
        let c = Condition::new("link_down", Severity::Warning, "down")
            .label("port", "eth0")
            .label("peer", "r2")
            .label("host.id", "x");
        assert_eq!(c.alert_key(), "a659f813308ad1da");
    }

    /// The file record reads back what it renders, and reads an
    /// `alarms.v1` record's documents.
    #[test]
    fn the_file_record_round_trips() {
        let held = [Kept {
            slot: 3,
            record: serde_json::json!({"state": "firing"}),
            words: serde_json::json!({"rule": "r", "summary": "s"}),
            alert_key: "a659f813308ad1da".into(),
            reading: Some("85.5".into()),
        }];
        let text = FileRecord::render(&held).to_string();
        assert_eq!(FileRecord::parse(&text), Ok(Found::V2(held.to_vec())));
        let v1 = r#"{"profile": "alarms.v1", "alarms": {"a659f813308ad1da": {"rule": "x"}}}"#;
        let Ok(Found::V1(docs)) = FileRecord::parse(v1) else {
            panic!("an alarms.v1 record")
        };
        assert_eq!(docs[0].alert_key, "a659f813308ad1da");
        assert!(FileRecord::parse("{}").is_err());
        assert!(FileRecord::parse(r#"{"slots": {"01": {}}}"#).is_err());
    }

    /// A condition reads from JSON, its reading a string or a number.
    #[test]
    fn a_condition_reads_from_json() {
        let c: Condition = serde_json::from_str(
            r#"{"rule": "hot", "severity": "warning", "summary": "s", "reading": 85.5}"#,
        )
        .unwrap();
        assert_eq!(c.reading.as_deref(), Some("85.5"));
        let c: Condition = serde_json::from_str(
            r#"{"rule": "hot", "severity": "warning", "summary": "s", "reading": "-40.2"}"#,
        )
        .unwrap();
        assert_eq!(c.reading.as_deref(), Some("-40.2"));
        let c: Condition =
            serde_json::from_str(r#"{"rule": "r", "severity": "info", "summary": "s"}"#).unwrap();
        assert_eq!(c.reading, None);
        assert!(
            serde_json::from_str::<Condition>(
                r#"{"rule": "r", "severity": "info", "summary": "s", "reading": [1]}"#
            )
            .is_err()
        );
    }

    /// An application shares the handle across tasks, and spawns its calls.
    #[test]
    fn the_handle_is_shared_across_tasks() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        fn spawnable<F: std::future::Future + Send>(_: F) {}
        shared::<super::Alarms>();
        // The builder's call is spawned too: an application starts its
        // service from a task.
        let _ = |mut b: crate::ServiceBuilder| {
            spawnable(async move {
                let _ = b.alarms_v2(super::Startup::default()).await;
            });
        };
        let _ = |a: super::Alarms| {
            spawnable(async move {
                let _ = a.raise(Condition::new("r", Severity::Info, "s")).await;
                let _ = a.resolve("k").await;
                let _ = a.set_confirming("k", false).await;
                let _ = a.held().await;
            });
        };
    }
}
