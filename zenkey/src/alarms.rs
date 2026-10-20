//! `alarms.v1` in the runtime: a service says which of its alarms fire
//! (`spec/profiles/alarms/v1.md`, #800).
//!
//! The owner's half of the profile. The key recipe, the document, the
//! reading of transitions and every judgement are `zenkey-model`'s
//! ([`zenkey_model::alarms`]); this module puts what the text asks of an
//! owner, on the standard contract, embedded.
//!
//! ```text
//! ServiceBuilder::alarms(startup) ─▶ Alarms: implements alarms.v1, exposes alarms/{alert_key},
//!        │                           answers its GETs; adopts the previous instance's alarms
//!        │                           from the record, else an archive on its own side (§2.9)
//!        │  raise(condition) ─▶ raised │ escalated │ refreshed │ unchanged   (§2.5)
//!        │  resolve(alert_key) ─▶ the resolved put, then the delete           (§2.5)
//!        │  set_confirming(alert_key, on), set_confirming_all(on)            (§2.7)
//!        ▼ start()
//!   Service::alarms() ─ the same handle; Service::new_epoch re-puts every alarm first (§2.9)
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.1 implemented with nothing firing | [`ServiceBuilder::alarms`] exposes the template and serves its GETs; the descriptor lists `alarms.v1` with no member put |
//! | §2.2, §2.3 each alarm keyed by the recipe, sanitized when unframed | every act keyed by [`zenkey_model::alarms::derive`] over the document it puts ([`Condition::alert_key`]) |
//! | §2.3 SHOULD put framed input | an unframed document is put, and logged as a warning with the document's other problems |
//! | §2.4 the owner's document | the runtime writes every document: the 1.0 schema's members, both times on the minter's clock (core §4.3) to the millisecond, below 2^53, `observed_at_ms` at or after `timestamp` and only on a refresh |
//! | §2.5 each act as its table says | [`Alarms::raise`] decides the act from what is held: a raise, an escalation (a new `timestamp`), a refresh (`timestamp` kept, `observed_at_ms` set), or nothing for a duplicate observation; [`Alarms::resolve`] |
//! | §2.5 a new `timestamp` later than the one before it on that key | the minter's clock, or the previous plus 1, from the alarm held, or the last transition of a key no longer held, of which the last 1024 are kept |
//! | §2.5 a resolve: the resolved put, then the delete, at once | [`Alarms::resolve`]; a delete that fails leaves the alarm held resolved, not confirmed, and the next resolve of it deletes it |
//! | §2.5 no delete of a firing alarm without its resolved put | no call deletes one; adoption's retirements are §2.9's exception |
//! | §2.7 each firing alarm it evaluates confirmed within 450 s | each alarm's [`StateWriter`] re-puts its document unchanged, `observed_at_ms` included, every 427.5 s, from the contract's `freshness.ttl_s = 900` (`freshness.v1` §2.4) |
//! | §2.7 no confirmation of an alarm it no longer evaluates | [`Alarms::set_confirming`], [`Alarms::set_confirming_all`]: it goes stale; any act on it is an evaluation, and turns its confirmation back on |
//! | §2.7 a resolved alarm never confirmed | its confirmation is turned off at its resolved put |
//! | §2.8 at most 256, or the descriptor's bound; a raise past it refused, its writer told | the resource's budget (core §2.7): [`Startup::cardinality`] or [`ServiceBuilder::cardinality`]; the raise returns [`Error::Budget`], nothing put |
//! | §2.8 no firing alarm resolved or deleted to make room | nothing is evicted |
//! | §2.8 SHOULD report the refusal (`health.v1`) | the application's: the refusal names the resource, the bound and whose it is |
//! | §2.9 adoption, before the first put, from the record, else an archive on its own side, never an S4 GET | [`ServiceBuilder::alarms`], from [`Startup::record`], else [`Startup::archive`] read explicitly ([`crate::archive::last_known_all`]) |
//! | §2.9 its order: resolved, then undecodable or not re-deriving, deleted; a rule no longer evaluated, resolved; else adopted; deletes and resolves before adoptions, and past the bound none of the rest resolved or deleted (0.2) | [`Adoption`]: retirements, then resolves, then adoptions in key order; the rest past the bound in [`Adoption::over_bound`], a step that fails in [`Adoption::failed`], both kept in the record |
//! | §2.9 SHOULD keep in the record the resolved document until its delete (0.2) | [`Record::save`] after every put and every delete, the resolved document's before its delete |
//! | §2.9 each alarm kept put again from the new instance before a GET is answered with it, and before the instance token | adoption puts each one, unchanged, in [`ServiceBuilder::alarms`], before `start()` declares anything alive; [`crate::Service::new_epoch`] re-puts every alarm before its new descriptor and tokens |
//! | §2.9 no source: nothing adopted | [`Source::Nothing`]; the instance is new, as every service's is (core §1.5) |
//! | §2.9 SHOULD NOT resolve or delete at a clean close | [`crate::Service::close`] touches no alarm |
//! | §2.10 each device service puts its own | one [`ServiceBuilder::alarms`] per service, a device service's included |
//! | §2.13 SHOULD be tokenless near the presence budget | the deployment's tokenless set (`ServiceConfig::tokenless`) |
//!
//! **The record is the application's** ([`Record`]): the runtime loads it
//! once, in [`ServiceBuilder::alarms`], and saves the alarms it holds after
//! every put and every delete: a resolve's resolved document is saved
//! before its delete, so that a process that ends between the two leaves a
//! record that reads resolved (§2.9, step 1), never firing. A record that
//! cannot be saved is logged as a warning; the act on the bus stands.
//! [`FileRecord`] is one record, a JSON file.
//!
//! **An adoption past the bound.** A record holding more firing alarms than
//! the bound in force adopts up to the bound, in key order. The rest are
//! neither resolved, which would say their condition cleared, nor deleted,
//! which §2.5 forbids of a firing alarm: a reader that held one reads it as
//! the previous instance's last word (§2.14), stale within 900 s. Each is
//! named in [`Adoption::over_bound`], and kept in the record for a later
//! start.
//!
//! **A step that fails** on the bus, a delete or a put of the adoption,
//! fails for its document only: [`ServiceBuilder::alarms`] still returns,
//! the document is named in [`Adoption::failed`] and kept in the record as
//! far as its step went (its resolved document once that is put), it is
//! not held, and every other document takes its step. The record is saved
//! once the adoption ends, so the next start redoes nothing done, and
//! settles what is left. An act on such a key supersedes what was found.
//!
//! **The contract is the published bundle**, revision 1.0
//! ([`FINGERPRINT`]), embedded byte for byte from
//! `spec/profiles/.history/alarms.v1/` and served as it is, as `health.v1`'s
//! is ([`crate::health`]). [`ServiceBuilder::alarms_with`] implements
//! another revision of the major (R4), the confirmation period following
//! its `freshness.ttl_s`: a deployment's later minor, or a test's copy with
//! a short horizon.
//!
//! **The owner's clock** is [`Minter`]'s, the session's HLC that core §4.3
//! requires of a serving session, to the millisecond: the clock its stamps
//! carry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use serde::Deserialize;
use zenkey_model::alarms as model;
use zenkey_model::authoring::Encoding as WireEncoding;
use zenkey_model::contract::{Body, Resource};
use zenkey_model::grammar::{Addr, IfaceId, KindToken, data_key};
use zenkey_model::schema::TypeId;
use zenkey_model::template::Bindings;

pub use zenkey_model::alarms::{Alarm, AlarmRef, Invalid, Severity, State};

use crate::budget::Budget;
use crate::call::BoxFuture;
use crate::error::{Error, Result};
use crate::implementation::Implementation;
use crate::service::ServiceBuilder;
use crate::state::{Minter, StateWriter, Store};
use crate::writer::Writer;

/// The standard contract's bundle, revision 1.0 as published in
/// `spec/profiles/.history/alarms.v1/` (§3), byte for byte.
pub const BUNDLE: &[u8] = include_bytes!(
    "../../spec/profiles/.history/alarms.v1/95579151aa231d2ba1224eae828083bdfc7f4f9403a80c831c1915decca9e8b6.bundle.json"
);

/// Its fingerprint (§3).
pub const FINGERPRINT: &str = "95579151aa231d2ba1224eae828083bdfc7f4f9403a80c831c1915decca9e8b6";

/// The alarms' template, as the descriptor names it.
pub const ALARMS: &str = "state/alarms/{alert_key}";

/// The template's parameter.
const PARAM: &str = "alert_key";

/// How many keys no longer held this owner remembers the last transition
/// of (§2.5, "as far as the owner knows of one").
const PAST: usize = 1024;

static IMPLEMENTATION: LazyLock<Implementation> = LazyLock::new(|| {
    Implementation::from_bundle(BUNDLE).expect("the published alarms.v1 bundle verifies")
});

/// The standard contract's interface id, `alarms.v1`.
#[must_use]
pub fn iface() -> IfaceId {
    model::IFACE.parse().expect("alarms.v1 is an interface id")
}

/// The standard contract, implemented from the embedded bundle: what
/// [`ServiceBuilder::alarms`] implements, and what a reader of `alarms.v1`
/// is compiled against (R4).
#[must_use]
pub fn implementation() -> Implementation {
    IMPLEMENTATION.clone()
}

/// What an owner found: the condition an alarm is raised for, with what it
/// says of it (§2.4). Its alert key is its rule's and its discriminating
/// labels' (§2.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Condition {
    /// The stable name of what raised it.
    pub rule: String,
    /// Every discriminating label is identity; a host-scoped one (`host`,
    /// `host.*`) is not hashed, and its change is a refresh (§2.2, §2.5).
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    pub severity: Severity,
    /// One line for a person; a reading that changes belongs here.
    pub summary: String,
    /// Free text naming what produced it, not hashed.
    #[serde(default)]
    pub category: Option<String>,
}

impl Condition {
    /// A condition of `rule` at `severity`, with no label.
    #[must_use]
    pub fn new(rule: impl Into<String>, severity: Severity, summary: impl Into<String>) -> Self {
        Self {
            rule: rule.into(),
            labels: BTreeMap::new(),
            severity,
            summary: summary.into(),
            category: None,
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

    /// Its alert key, by the reader's recipe (§2.2, §2.3).
    #[must_use]
    pub fn alert_key(&self) -> String {
        model::derive(
            &self.rule,
            self.labels.iter().map(|(n, v)| (n.as_str(), v.as_str())),
        )
        .key
    }

    /// Whether `doc` says what this condition says, its state and times
    /// aside.
    fn says(&self, doc: &Alarm) -> bool {
        self.rule == doc.rule
            && self.labels == doc.labels
            && self.severity == doc.severity
            && self.summary == doc.summary
            && self.category == doc.category
    }
}

/// The act an owner's call made (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Act {
    /// Put firing with a new `timestamp`: nothing was held, or a resolve
    /// whose delete had failed.
    Raised,
    /// Put firing at another severity, with a new `timestamp`.
    Escalated,
    /// Put firing at the same severity, its content changed: the
    /// `timestamp` kept, `observed_at_ms` set.
    Refreshed,
    /// Nothing put: the same condition observed again (`scenarios.md` §1).
    Unchanged,
    /// Put resolved with a new `timestamp`, then deleted.
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
            Self::Unchanged => "unchanged",
            Self::Resolved => "resolved",
        }
    }
}

/// What a call did: the alarm's key, the act, and the document now on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    pub alert_key: String,
    pub act: Act,
    pub document: Alarm,
}

/// One alarm as a record keeps it: the alert key it was put on, and its
/// document's payload as put.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub alert_key: String,
    pub payload: Vec<u8>,
}

/// The owner's own persistent record of the alarms it holds (§2.9). The
/// application holds it, as `config.v1`'s device holds its persisted layer
/// ([`crate::config_v1::Device`]): the runtime loads it at start, and saves
/// the alarms held after every change, whole.
pub trait Record: Send + Sync + 'static {
    /// What the previous instance held, by the key each was put on, as the
    /// last [`Record::save`] wrote it, or as a test wrote it. `Ok(None)`:
    /// there is no record, and the archive, if any, is read instead. `Err`
    /// is logged, and read as no record.
    fn load(&self) -> BoxFuture<'_, std::result::Result<Option<Vec<Kept>>, String>>;

    /// The alarms held now, whole, after a put or a delete.
    fn save<'a>(&'a self, held: &'a [Kept]) -> BoxFuture<'a, std::result::Result<(), String>>;
}

/// A [`Record`] in a JSON file: `{"profile": "alarms.v1", "alarms": {<alert
/// key>: <document>, …}}`, each document a JSON value as it was put. A
/// missing file is no record. It is written whole, through a temporary file
/// beside it, renamed.
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
    ///
    /// # Errors
    ///
    /// A payload that is not JSON, named by its key: the file is not
    /// written, so a document never turns undecodable there, to be deleted
    /// at the next start.
    pub fn render(held: &[Kept]) -> std::result::Result<serde_json::Value, String> {
        let mut alarms = serde_json::Map::new();
        for k in held {
            let doc = serde_json::from_slice(&k.payload)
                .map_err(|e| format!("the document of {} is not JSON: {e}", k.alert_key))?;
            alarms.insert(k.alert_key.clone(), doc);
        }
        Ok(serde_json::json!({"profile": model::PROFILE, "alarms": alarms}))
    }

    /// What a record's file holds, by key: each document re-encoded as
    /// JSON. Members other than `alarms` are ignored.
    ///
    /// # Errors
    ///
    /// The file is not JSON, its `alarms` is not an object, or a document
    /// does not encode again.
    pub fn parse(text: &str) -> std::result::Result<Vec<Kept>, String> {
        let v: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        let alarms = v
            .get("alarms")
            .and_then(serde_json::Value::as_object)
            .ok_or("no \"alarms\" object")?;
        alarms
            .iter()
            .map(|(k, doc)| {
                Ok(Kept {
                    alert_key: k.clone(),
                    payload: serde_json::to_vec(doc)
                        .map_err(|e| format!("the document of {k}: {e}"))?,
                })
            })
            .collect()
    }
}

impl Record for FileRecord {
    fn load(&self) -> BoxFuture<'_, std::result::Result<Option<Vec<Kept>>, String>> {
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
            let text =
                serde_json::to_vec_pretty(&Self::render(held)?).map_err(|e| e.to_string())?;
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

/// Which rules, and targets, an owner still evaluates (§2.5, §2.9 step
/// 3): asked of each firing document a restart finds.
pub type Evaluates = Arc<dyn Fn(&Alarm) -> bool + Send + Sync>;

/// What an owner starts from (§2.8, §2.9): where its previous instance's
/// alarms are, what it still evaluates, and its bound. The default adopts
/// nothing, evaluates every rule, and keeps the contract's bound.
#[derive(Clone, Default)]
pub struct Startup {
    /// Its own persistent record, loaded at start and kept current.
    pub record: Option<Arc<dyn Record>>,
    /// An archive on its own side (core §4.4), and the read's timeout:
    /// read explicitly when the record has nothing.
    pub archive: Option<(Addr, Duration)>,
    /// The documents whose condition it still evaluates; `None`: every one.
    pub evaluates: Option<Evaluates>,
    /// The bound its descriptor lowers the contract's 256 to (core §3.3),
    /// set before adoption.
    pub cardinality: Option<u64>,
    /// A test's injected failures: the bus operations, counted from 1, that
    /// fail as zenoh would.
    #[cfg(test)]
    pub(crate) fail_ops: std::collections::BTreeSet<usize>,
}

impl std::fmt::Debug for Startup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Startup")
            .field("record", &self.record.is_some())
            .field("archive", &self.archive)
            .field("evaluates", &self.evaluates.is_some())
            .field("cardinality", &self.cardinality)
            .finish()
    }
}

impl Startup {
    /// With `record` as its persistent record.
    #[must_use]
    pub fn with_record(mut self, record: Arc<dyn Record>) -> Self {
        self.record = Some(record);
        self
    }

    /// With the archive at `archive` on its own side, read within
    /// `timeout`.
    #[must_use]
    pub fn with_archive(mut self, archive: Addr, timeout: Duration) -> Self {
        self.archive = Some((archive, timeout));
        self
    }

    /// Evaluating exactly the rules named.
    #[must_use]
    pub fn evaluating<S: Into<String>>(mut self, rules: impl IntoIterator<Item = S>) -> Self {
        let rules: std::collections::BTreeSet<String> = rules.into_iter().map(Into::into).collect();
        self.evaluates = Some(Arc::new(move |a: &Alarm| rules.contains(&a.rule)));
        self
    }

    /// Evaluating the documents `f` accepts.
    #[must_use]
    pub fn evaluating_with(mut self, f: impl Fn(&Alarm) -> bool + Send + Sync + 'static) -> Self {
        self.evaluates = Some(Arc::new(f));
        self
    }

    /// With the bound lowered to `n`.
    #[must_use]
    pub fn with_cardinality(mut self, n: u64) -> Self {
        self.cardinality = Some(n);
        self
    }
}

/// Where a restart's alarms came from (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// The owner's own record.
    Record,
    /// An archive on its own side, last-known.
    Archive,
    /// Neither: nothing adopted.
    #[default]
    Nothing,
}

impl Source {
    /// A word for it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Record => "record",
            Self::Archive => "archive",
            Self::Nothing => "nothing",
        }
    }
}

/// Why a restart deleted a key it found (§2.9, steps 1 and 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retired {
    /// A resolved document: its delete was lost.
    Resolved,
    /// A payload that does not read (§2.4).
    Undecodable,
    /// A document its key does not re-derive from (§2.2, §2.3): a phantom.
    Phantom,
}

impl Retired {
    /// A word for it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolved => "resolved",
            Self::Undecodable => "undecodable",
            Self::Phantom => "phantom",
        }
    }
}

/// What a restart did with what it found (§2.9), by alert key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Adoption {
    pub source: Source,
    /// Steps 1 and 2: deleted.
    pub retired: Vec<(String, Retired)>,
    /// Step 3: firing under a rule no longer evaluated, resolved.
    pub resolved: Vec<String>,
    /// Step 4: adopted, `timestamp` kept, put again.
    pub adopted: Vec<String>,
    /// Firing and evaluated, past the bound: neither adopted, resolved nor
    /// deleted (the module doc), and kept in the record for a later start.
    pub over_bound: Vec<String>,
    /// The documents whose step failed on the bus, and why. Each is kept in
    /// the record as far as its step went, for the next start (its resolved
    /// document once that is put), and none is held; every other document
    /// took its step.
    pub failed: Vec<(String, String)>,
}

/// A service's `alarms.v1` (§2.5–§2.9), from [`ServiceBuilder::alarms`],
/// and again from [`crate::Service::alarms`] once it runs.
///
/// A clone is the same handle. Calls are serialized: each finishes its puts
/// before the next begins. While the clock guard holds state writes (core
/// §4.3), a call that puts returns [`Error::ClockAhead`] and changes
/// nothing; a confirmation it holds goes out once the guard releases
/// (`freshness.v1` §2.10).
#[derive(Clone)]
pub struct Alarms {
    inner: Arc<Inner>,
}

struct Inner {
    iface: IfaceId,
    resource: Resource,
    session: zenoh::Session,
    addr: Addr,
    store: Arc<Store>,
    minter: Arc<Minter>,
    closed: tokio::sync::watch::Receiver<bool>,
    budget: Option<Arc<Budget>>,
    record: Option<Arc<dyn Record>>,
    held: tokio::sync::Mutex<Held>,
    adoption: Mutex<Adoption>,
    /// A test's injected failures: the operations counted so far, and those
    /// that fail.
    #[cfg(test)]
    faults: Mutex<(usize, std::collections::BTreeSet<usize>)>,
}

#[derive(Default)]
struct Held {
    alarms: BTreeMap<String, Entry>,
    /// The last transition's `timestamp` of keys no longer held, at most
    /// [`PAST`] of them.
    past: BTreeMap<String, u64>,
    /// Documents a restart found and could not settle, or adopt within the
    /// bound: not held, kept in the record for a later start, until an act
    /// on their key supersedes them.
    unsettled: BTreeMap<String, Vec<u8>>,
}

impl Held {
    fn forget(&mut self, key: &str, timestamp: u64) {
        self.past.insert(key.to_owned(), timestamp);
        if self.past.len() > PAST
            && let Some(oldest) = self
                .past
                .iter()
                .min_by_key(|(_, t)| **t)
                .map(|(k, _)| k.clone())
        {
            self.past.remove(&oldest);
        }
    }

    /// What the record holds: the alarms held, and the documents unsettled.
    fn kept(&self) -> Vec<Kept> {
        let mut all: BTreeMap<&String, &Vec<u8>> = self.unsettled.iter().collect();
        all.extend(self.alarms.iter().map(|(k, e)| (k, &e.payload)));
        all.into_iter()
            .map(|(k, p)| Kept {
                alert_key: k.clone(),
                payload: p.clone(),
            })
            .collect()
    }
}

/// One alarm held: its document, the payload put, and its writer.
struct Entry {
    doc: Alarm,
    payload: Vec<u8>,
    writer: Arc<StateWriter>,
}

/// A stamp's time in milliseconds since the Unix epoch, below 2^53 (§2.4).
fn ms(t: &zenoh::time::Timestamp) -> u64 {
    let since = t
        .get_time()
        .to_system_time()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(since.as_millis())
        .unwrap_or(u64::MAX)
        .min(model::MAX_TIME_MS)
}

impl Alarms {
    /// Implements `alarms.v1` on `b` from `imp`, then adopts the previous
    /// instance's alarms (§2.9).
    pub(crate) async fn declare(
        b: &mut ServiceBuilder,
        imp: Implementation,
        startup: Startup,
    ) -> Result<Self> {
        let addr = b.address()?.clone();
        if imp.iface() != &iface() {
            return Err(Error::Contract(format!(
                "{} is not alarms.v1: ServiceBuilder::alarms_with implements a revision of \
                 alarms.v1",
                imp.iface()
            )));
        }
        let resource = imp.resource(ALARMS)?.clone();
        let json = match &resource.body {
            Body::Data(d) => {
                matches!(d.type_, TypeId::JsonSchema { .. })
                    && !matches!(d.encoding, Some(WireEncoding::Cbor))
            }
            Body::Operation(_) => false,
        };
        if resource.token != KindToken::State || !json {
            return Err(Error::Contract(format!(
                "alarms.v1 {ALARMS:?} is a JSON state in every revision of the major (§3)"
            )));
        }
        if zenkey_model::freshness::horizon_of(&resource)
            .refresh_bound()
            .is_none_or(|b| b.is_zero())
        {
            return Err(Error::Contract(format!(
                "alarms.v1 {ALARMS:?} declares no freshness.ttl_s above 0, so no alarm of it \
                 would be confirmed (§2.7)"
            )));
        }
        let iface = imp.iface().clone();
        b.implement(imp)?;
        b.expose(&iface, ALARMS)?;
        b.serve_state(&iface)?;
        if let Some(n) = startup.cardinality {
            b.cardinality(&iface, ALARMS, n)?;
        }
        let budget = b.budget_of(&iface, ALARMS);
        let (session, store, minter, closed) = b.state_parts();
        let a = Self {
            inner: Arc::new(Inner {
                iface,
                resource,
                session,
                addr,
                store,
                minter,
                closed,
                budget,
                record: startup.record.clone(),
                held: tokio::sync::Mutex::new(Held::default()),
                adoption: Mutex::new(Adoption::default()),
                #[cfg(test)]
                faults: Mutex::new((0, startup.fail_ops.clone())),
            }),
        };
        a.adopt(&startup).await?;
        b.alarms = Some(a.clone());
        Ok(a)
    }

    /// `alarms.v1`.
    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.inner.iface
    }

    /// The most alarms this owner holds: the contract's 256, or the bound
    /// its descriptor lowers it to (§2.8).
    #[must_use]
    pub fn max_alarms(&self) -> u64 {
        self.inner
            .budget
            .as_ref()
            .and_then(|b| b.bound().0.limit())
            .unwrap_or(model::MAX_ALARMS)
    }

    /// What the restart did (§2.9).
    #[must_use]
    pub fn adoption(&self) -> Adoption {
        self.inner.adoption.lock().expect("not poisoned").clone()
    }

    /// The alarms held now, by alert key: firing, or resolved while their
    /// delete is due.
    pub async fn held(&self) -> BTreeMap<String, Alarm> {
        let st = self.inner.held.lock().await;
        st.alarms
            .iter()
            .map(|(k, e)| (k.clone(), e.doc.clone()))
            .collect()
    }

    /// The alarm held on `alert_key`.
    pub async fn get(&self, alert_key: &str) -> Option<Alarm> {
        let st = self.inner.held.lock().await;
        st.alarms.get(alert_key).map(|e| e.doc.clone())
    }

    /// The condition holds, as `c` says (§2.5): the act it makes on its
    /// alarm, from what is held.
    /// - nothing held: a **raise**, firing, with a new `timestamp`;
    /// - held firing at another severity: an **escalation**, with a new
    ///   `timestamp`;
    /// - held firing at that severity, its content changed (the summary,
    ///   the category, a host-scoped label): a **refresh**, the `timestamp`
    ///   kept and `observed_at_ms` the instant now;
    /// - held firing as `c` says: **unchanged**, nothing put.
    ///
    /// Each is an evaluation, so the alarm's confirmation is turned back on
    /// (§2.7). A raise past the bound is refused, [`Error::Budget`],
    /// nothing put and nothing evicted (§2.8). On an alarm held resolved,
    /// its delete having failed ([`Alarms::resolve`]), it is a re-raise: a
    /// new `timestamp`, confirmed again, and no delete owed any more, since
    /// the key holds a firing alarm again.
    pub async fn raise(&self, c: Condition) -> Result<Done> {
        let key = c.alert_key();
        let mut st = self.inner.held.lock().await;
        let now = self.now_ms();
        let (act, doc) = match st.alarms.get(&key) {
            None => (
                Act::Raised,
                self.firing(&c, new_after(now, st.past.get(&key))),
            ),
            Some(e) if e.doc.state == State::Resolved => (
                Act::Raised,
                self.firing(&c, new_after(now, Some(&e.doc.timestamp))),
            ),
            Some(e) if c.severity != e.doc.severity => (
                Act::Escalated,
                self.firing(&c, new_after(now, Some(&e.doc.timestamp))),
            ),
            Some(e) if c.says(&e.doc) => {
                e.writer.set_refresh(true);
                return Ok(Done {
                    alert_key: key,
                    act: Act::Unchanged,
                    document: e.doc.clone(),
                });
            }
            Some(e) => {
                let mut doc = self.firing(&c, e.doc.timestamp);
                doc.observed_at_ms = Some(now.max(e.doc.timestamp));
                (Act::Refreshed, doc)
            }
        };
        warn_problems(&key, &doc);
        let fresh = if st.alarms.contains_key(&key) {
            None
        } else {
            Some(Arc::new(self.writer(&key).await?))
        };
        let payload = match &fresh {
            Some(w) => w.writer().encode(&doc)?,
            None => st.alarms[&key].writer.writer().encode(&doc)?,
        };
        match fresh {
            Some(writer) => {
                writer.put(payload.clone()).await?;
                st.past.remove(&key);
                st.unsettled.remove(&key);
                st.alarms.insert(
                    key.clone(),
                    Entry {
                        doc: doc.clone(),
                        payload,
                        writer,
                    },
                );
            }
            None => {
                let e = st.alarms.get_mut(&key).expect("held");
                e.writer.put(payload.clone()).await?;
                e.writer.set_refresh(true);
                e.doc = doc.clone();
                e.payload = payload;
                st.unsettled.remove(&key);
            }
        }
        self.save(&st).await;
        Ok(Done {
            alert_key: key,
            act,
            document: doc,
        })
    }

    /// The condition of the alarm on `alert_key` no longer holds, or the
    /// owner will evaluate it no more while it runs (§2.5): its resolved
    /// document is put, with a new `timestamp`, then its key is deleted at
    /// once (core S3). `None` when no alarm is held there. A delete that
    /// fails leaves the alarm held resolved, unconfirmed and saved so in the
    /// record, until a resolve deletes it, or a raise re-raises it.
    pub async fn resolve(&self, alert_key: &str) -> Result<Option<Done>> {
        let mut st = self.inner.held.lock().await;
        let now = self.now_ms();
        let Some(e) = st.alarms.get_mut(alert_key) else {
            return Ok(None);
        };
        if e.doc.state == State::Firing {
            let doc = Alarm {
                timestamp: new_after(now, Some(&e.doc.timestamp)),
                observed_at_ms: None,
                state: State::Resolved,
                ..e.doc.clone()
            };
            let payload = e.writer.writer().encode(&doc)?;
            self.fault()?;
            e.writer.put(payload.clone()).await?;
            // §2.7: a resolved alarm is never confirmed.
            e.writer.set_refresh(false);
            e.doc = doc;
            e.payload = payload;
            self.save(&st).await;
        }
        let e = st.alarms.get(alert_key).expect("held");
        self.fault()?;
        e.writer.delete().await?;
        let e = st.alarms.remove(alert_key).expect("held");
        st.forget(alert_key, e.doc.timestamp);
        self.save(&st).await;
        Ok(Some(Done {
            alert_key: alert_key.to_owned(),
            act: Act::Resolved,
            document: e.doc,
        }))
    }

    /// Turns the confirmation of the alarm on `alert_key` off, or back on
    /// (§2.7): an owner that has stopped evaluating its condition, its
    /// evaluator stuck or its source gone, lets it go stale rather than
    /// confirm what it no longer sees. Turned back on, it is re-put at once
    /// if its interval has run out. `false`, nothing changed, when no firing
    /// alarm is held there: none at all, or one held resolved, which is
    /// never confirmed.
    pub async fn set_confirming(&self, alert_key: &str, on: bool) -> bool {
        let st = self.inner.held.lock().await;
        match st.alarms.get(alert_key) {
            Some(e) if e.doc.state == State::Firing => {
                e.writer.set_refresh(on);
                true
            }
            _ => false,
        }
    }

    /// [`Alarms::set_confirming`] for every firing alarm held: an evaluator
    /// that is stuck as a whole.
    pub async fn set_confirming_all(&self, on: bool) {
        let st = self.inner.held.lock().await;
        for e in st.alarms.values() {
            if e.doc.state == State::Firing {
                e.writer.set_refresh(on);
            }
        }
    }

    /// Whether the alarm on `alert_key` is being confirmed; `None` when no
    /// alarm is held there.
    pub async fn is_confirming(&self, alert_key: &str) -> Option<bool> {
        let st = self.inner.held.lock().await;
        st.alarms.get(alert_key).map(|e| e.writer.is_refreshing())
    }

    /// The writer of the alarm on `alert_key`: zenoh stays reachable.
    pub async fn with_writer<T>(
        &self,
        alert_key: &str,
        f: impl FnOnce(&StateWriter) -> T,
    ) -> Option<T> {
        let st = self.inner.held.lock().await;
        st.alarms.get(alert_key).map(|e| f(&e.writer))
    }

    /// §2.9, a re-mint: every alarm held put again, unchanged, under a
    /// fresh stamp, before the new instance's descriptor and tokens.
    ///
    /// The writers are taken under the lock, and each re-puts its own
    /// current value under its member's lock ([`StateWriter`]), so a raise
    /// or a resolve meanwhile is neither blocked nor overwritten: a resolved
    /// alarm deleted since is not put again. A failure stops the loop and is
    /// returned, and [`crate::Service::new_epoch`] then mints nothing: the
    /// alarms already re-put carry a fresh stamp of the same session, a
    /// confirmation out of turn, and a retry puts every one again.
    pub(crate) async fn reput_all(&self) -> Result<()> {
        let writers: Vec<Arc<StateWriter>> = {
            let st = self.inner.held.lock().await;
            st.alarms.values().map(|e| Arc::clone(&e.writer)).collect()
        };
        for w in writers {
            w.reput().await?;
        }
        Ok(())
    }

    /// A test's injected failure of the next bus operation; nothing outside
    /// tests.
    #[allow(clippy::unnecessary_wraps, clippy::unused_self)]
    fn fault(&self) -> Result<()> {
        #[cfg(test)]
        {
            let mut f = self.inner.faults.lock().expect("not poisoned");
            f.0 += 1;
            if f.1.contains(&f.0) {
                return Err(Error::Zenoh(format!(
                    "injected failure of operation {}",
                    f.0
                )));
            }
        }
        Ok(())
    }

    fn now_ms(&self) -> u64 {
        ms(&self.inner.minter.now())
    }

    fn firing(&self, c: &Condition, timestamp: u64) -> Alarm {
        Alarm {
            timestamp,
            observed_at_ms: None,
            rule: c.rule.clone(),
            severity: c.severity,
            state: State::Firing,
            summary: c.summary.clone(),
            category: c.category.clone(),
            labels: c.labels.clone(),
        }
    }

    /// A writer on the member `alarms/<alert_key>`, as the service's own
    /// state writers are made: stamped, answered by its state queryables,
    /// within the resource's budget (S1, S2, core §2.7).
    async fn writer(&self, alert_key: &str) -> Result<StateWriter> {
        let i = &self.inner;
        let values: Bindings = [(PARAM.to_owned(), vec![alert_key.to_owned()])].into();
        let chunks = i
            .resource
            .template
            .build(&values)
            .map_err(|e| Error::Contract(format!("alarms.v1 {ALARMS:?}: {e}")))?;
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let key = data_key(&i.addr, &i.iface, i.resource.token, &refs)?;
        let w = Writer::declare(&i.session, key.into_keyexpr(), &i.resource, &values)
            .await?
            .bounded(i.budget.clone());
        Ok(StateWriter::new(
            w,
            Arc::clone(&i.store),
            Arc::clone(&i.minter),
            &zenkey_model::freshness::horizon_of(&i.resource),
            i.closed.clone(),
        ))
    }

    /// Saves the alarms held into the record, if any; a failure is logged.
    async fn save(&self, st: &Held) {
        let Some(r) = &self.inner.record else {
            return;
        };
        if let Err(e) = r.save(&st.kept()).await {
            tracing::warn!(
                "alarms.v1: the record was not saved, so a restart may not adopt what this \
                 instance holds (§2.9): {e}"
            );
        }
    }

    /// Where the previous instance's alarms are (§2.9): the record, else
    /// the archive, else nothing.
    async fn load(&self, startup: &Startup) -> (Source, Vec<Kept>) {
        if let Some(r) = &startup.record {
            match r.load().await {
                Ok(Some(found)) => return (Source::Record, found),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    "alarms.v1: the record could not be read, so it is read as none (§2.9): {e}"
                ),
            }
        }
        if let Some((archive, timeout)) = &startup.archive {
            let a = &self.inner.addr;
            let pattern = format!(
                "zk2/{}/{}/{}/{}/alarms/*",
                a.system, a.service, self.inner.iface, self.inner.resource.token
            );
            match crate::archive::last_known_all(&self.inner.session, archive, &pattern, *timeout)
                .await
            {
                Ok(found) => {
                    let kept: Vec<Kept> = found
                        .into_iter()
                        .filter_map(|lk| {
                            Some(Kept {
                                alert_key: AlarmRef::from_key(&lk.origin)?.alert_key,
                                payload: lk.value?,
                            })
                        })
                        .collect();
                    if !kept.is_empty() {
                        return (Source::Archive, kept);
                    }
                }
                Err(e) => tracing::warn!(
                    "alarms.v1: the archive at {archive} could not be read, so nothing is \
                     adopted from it (§2.9): {e}"
                ),
            }
        }
        (Source::Nothing, Vec::new())
    }

    /// §2.9: each document found, by its order of rules, before any other
    /// put of this instance. A step that fails on the bus fails for its
    /// document only: it is named in [`Adoption::failed`], kept in the record
    /// as far as its step went, and every other document takes its step.
    async fn adopt(&self, startup: &Startup) -> Result<()> {
        let (source, found) = self.load(startup).await;
        let mut seen = std::collections::BTreeSet::new();
        let mut retire = Vec::new();
        let mut resolve = Vec::new();
        let mut adopt = Vec::new();
        for k in found {
            if !seen.insert(k.alert_key.clone()) {
                continue;
            }
            match model::decode(&k.payload) {
                Err(_) => retire.push((k, Retired::Undecodable, None)),
                Ok(d) if d.state == State::Resolved => {
                    let t = d.timestamp;
                    retire.push((k, Retired::Resolved, Some(t)));
                }
                Ok(d) if d.alert_key() != k.alert_key => {
                    retire.push((k, Retired::Phantom, None));
                }
                Ok(d) if !startup.evaluates.as_ref().is_none_or(|f| f(&d)) => {
                    resolve.push((k, d));
                }
                Ok(d) => adopt.push((k, d)),
            }
        }
        let mut report = Adoption {
            source,
            ..Adoption::default()
        };
        let mut st = self.inner.held.lock().await;
        // Steps 1 and 2: deleted, which reads as resolved to whoever held
        // one firing (§2.6).
        for (k, why, t) in retire {
            let done = async {
                let w = self.writer(&k.alert_key).await?;
                self.fault()?;
                w.delete().await
            };
            match done.await {
                Ok(_) => {
                    if let Some(t) = t {
                        st.forget(&k.alert_key, t);
                    }
                    report.retired.push((k.alert_key, why));
                }
                Err(e) => unsettled(&mut st, &mut report, k, &e),
            }
        }
        // Step 3: resolved, as §2.5 says: put resolved, then deleted.
        for (k, doc) in resolve {
            let resolved = Alarm {
                timestamp: new_after(self.now_ms(), Some(&doc.timestamp)),
                observed_at_ms: None,
                state: State::Resolved,
                ..doc
            };
            let mut kept = k.clone();
            let done = async {
                let w = self.writer(&k.alert_key).await?;
                let payload = w.writer().encode(&resolved)?;
                self.fault()?;
                w.put(payload.clone()).await?;
                // The resolved document is the record's from here on.
                kept.payload = payload;
                w.set_refresh(false);
                self.fault()?;
                w.delete().await
            };
            match done.await {
                Ok(_) => {
                    st.forget(&k.alert_key, resolved.timestamp);
                    report.resolved.push(k.alert_key);
                }
                Err(e) => unsettled(&mut st, &mut report, kept, &e),
            }
        }
        // Step 4: adopted, its timestamp kept, put again unchanged.
        for (k, doc) in adopt {
            let put = async {
                let writer = self.writer(&k.alert_key).await?;
                self.fault()?;
                writer.put(k.payload.clone()).await?;
                Ok::<_, Error>(writer)
            };
            match put.await {
                Ok(writer) => {
                    st.alarms.insert(
                        k.alert_key.clone(),
                        Entry {
                            doc,
                            payload: k.payload,
                            writer: Arc::new(writer),
                        },
                    );
                    report.adopted.push(k.alert_key);
                }
                Err(Error::Budget(r)) => {
                    tracing::warn!(
                        alert_key = %k.alert_key,
                        "alarms.v1: a firing alarm of the previous instance is past the bound, \
                         so it is neither adopted, resolved nor deleted, and stays in the record \
                         (§2.8, §2.9): {r}"
                    );
                    report.over_bound.push(k.alert_key.clone());
                    st.unsettled.insert(k.alert_key, k.payload);
                }
                Err(e) => unsettled(&mut st, &mut report, k, &e),
            }
        }
        if source != Source::Nothing {
            tracing::info!(
                source = source.as_str(),
                adopted = report.adopted.len(),
                retired = report.retired.len(),
                resolved = report.resolved.len(),
                failed = report.failed.len(),
                "alarms.v1: the previous instance's alarms, taken over (§2.9)"
            );
            self.save(&st).await;
        }
        drop(st);
        *self.inner.adoption.lock().expect("not poisoned") = report;
        Ok(())
    }
}

/// A document whose step failed on the bus: kept for the record, as far as
/// its step went, and reported (§2.9).
fn unsettled(st: &mut Held, report: &mut Adoption, k: Kept, e: &Error) {
    tracing::warn!(
        alert_key = %k.alert_key,
        "alarms.v1: a document of the previous instance could not take its step, so it stays \
         in the record for the next start (§2.9): {e}"
    );
    report.failed.push((k.alert_key.clone(), e.to_string()));
    st.unsettled.insert(k.alert_key, k.payload);
}

/// A new `timestamp` (§2.5): the clock's `now`, or the previous plus 1
/// where the clock has not passed it.
fn new_after(now: u64, previous: Option<&u64>) -> u64 {
    match previous {
        Some(p) if now <= *p => (p + 1).min(model::MAX_TIME_MS),
        _ => now,
    }
}

/// §2.3's SHOULD, and §2.4's forms: what the runtime puts anyway, said.
fn warn_problems(key: &str, doc: &Alarm) {
    for p in doc.check() {
        tracing::warn!(alert_key = %key, "alarms.v1: {p} (§2.3, §2.4)");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BUNDLE, Condition, FINGERPRINT, FileRecord, Kept, Severity, State, implementation, model,
        new_after,
    };

    /// The embedded bundle is the published revision, and its resource
    /// carries the horizon the confirmations come from and the bound.
    #[test]
    fn the_contract_drives_the_confirmation_and_the_bound() {
        let imp = implementation();
        assert_eq!(imp.fingerprint().hex().to_string(), FINGERPRINT);
        assert_eq!(&**imp.bundle_bytes(), BUNDLE, "served byte for byte");
        let r = imp.resource(super::ALARMS).unwrap();
        assert_eq!(
            zenkey_model::freshness::horizon_of(r).refresh_bound(),
            Some(std::time::Duration::from_secs(450))
        );
        assert_eq!(r.cardinality, Some(256));
    }

    /// §2.2: a condition's key is the recipe's, host-scoped labels aside.
    #[test]
    fn a_conditions_key_is_the_recipes() {
        let c = Condition::new("link_down", Severity::Warning, "down")
            .label("port", "eth0")
            .label("peer", "r2")
            .label("host.id", "x");
        assert_eq!(c.alert_key(), "a659f813308ad1da");
    }

    /// §2.5: a new timestamp passes the previous one.
    #[test]
    fn a_new_timestamp_is_later() {
        assert_eq!(new_after(10, None), 10);
        assert_eq!(new_after(10, Some(&5)), 10);
        assert_eq!(new_after(10, Some(&10)), 11);
        assert_eq!(new_after(10, Some(&20)), 21);
    }

    /// The file record reads back what it renders.
    #[test]
    fn the_file_record_round_trips() {
        let held = [Kept {
            alert_key: "a659f813308ad1da".into(),
            payload: br#"{"rule":"link_down","severity":"warning","state":"firing","summary":"s","timestamp":1}"#.to_vec(),
        }];
        let text = FileRecord::render(&held).unwrap().to_string();
        let back = FileRecord::parse(&text).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].alert_key, held[0].alert_key);
        let a: serde_json::Value = serde_json::from_slice(&back[0].payload).unwrap();
        let b: serde_json::Value = serde_json::from_slice(&held[0].payload).unwrap();
        assert_eq!(a, b);
        assert!(FileRecord::parse("{}").is_err());
        let corrupt = [Kept {
            alert_key: "k".into(),
            payload: b"not json".to_vec(),
        }];
        assert!(
            FileRecord::render(&corrupt).is_err(),
            "a corrupt document is never written as another"
        );
    }

    /// An application shares the handle across tasks, and spawns its calls.
    #[test]
    fn the_handle_is_shared_across_tasks() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        fn spawnable<F: std::future::Future + Send>(_: F) {}
        shared::<super::Alarms>();
        let _ = |a: super::Alarms| {
            spawnable(async move {
                let _ = a.raise(Condition::new("r", Severity::Info, "s")).await;
                let _ = a.resolve("k").await;
                let _ = a.set_confirming("k", false).await;
                let _ = a.held().await;
            });
        };
    }

    /// A [`super::Record`] in memory, whose saves a test reads.
    #[derive(Default)]
    struct Memory {
        found: std::sync::Mutex<Option<Vec<Kept>>>,
        saves: std::sync::Mutex<Vec<Vec<Kept>>>,
    }

    impl Memory {
        fn with(found: Vec<Kept>) -> std::sync::Arc<Self> {
            std::sync::Arc::new(Self {
                found: std::sync::Mutex::new(Some(found)),
                saves: std::sync::Mutex::default(),
            })
        }

        /// The last save, by key, each document as JSON.
        fn last(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
            self.saves
                .lock()
                .unwrap()
                .last()
                .expect("a save")
                .iter()
                .map(|k| {
                    (
                        k.alert_key.clone(),
                        serde_json::from_slice(&k.payload).unwrap(),
                    )
                })
                .collect()
        }
    }

    impl super::Record for Memory {
        fn load(
            &self,
        ) -> crate::call::BoxFuture<'_, std::result::Result<Option<Vec<Kept>>, String>> {
            let found = self.found.lock().unwrap().clone();
            Box::pin(async move { Ok(found) })
        }

        fn save<'a>(
            &'a self,
            held: &'a [Kept],
        ) -> crate::call::BoxFuture<'a, std::result::Result<(), String>> {
            self.saves.lock().unwrap().push(held.to_vec());
            Box::pin(async { Ok(()) })
        }
    }

    /// A session of its own: puts reach no one, and succeed.
    async fn alone() -> zenoh::Session {
        let mut c = zenoh::Config::default();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("listen/endpoints", "[]").unwrap();
        zenoh::open(c).await.unwrap()
    }

    fn kept(alert_key: &str, doc: &serde_json::Value) -> Kept {
        Kept {
            alert_key: alert_key.to_owned(),
            payload: serde_json::to_vec(doc).unwrap(),
        }
    }

    fn firing(rule: &str, labels: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"timestamp": 1_760_950_000_000_u64, "rule": rule, "severity": "warning",
            "state": "firing", "summary": "s", "labels": labels})
    }

    async fn builder(
        s: &zenoh::Session,
        startup: super::Startup,
    ) -> (crate::ServiceBuilder, super::Alarms) {
        let mut b =
            crate::ServiceBuilder::new(s, crate::ServiceConfig::new("lab/svc".parse().unwrap()));
        let a = b.alarms(startup).await.unwrap();
        (b, a)
    }

    /// §2.9: a step that fails on the bus fails for its document only. It is
    /// reported, kept in the record as far as its step went, and not held;
    /// every other document takes its step; and the next start settles what
    /// is left, redoing nothing done.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_adoption_step_loses_nothing() {
        let s = alone().await;
        let a_doc = firing(
            "link_down",
            serde_json::json!({"peer": "r2", "port": "eth0"}),
        );
        let b_doc = firing("disk_full", serde_json::json!({"mount": "/var"}));
        let x_doc = firing("fan_gone", serde_json::json!({}));
        let mut r_doc = firing("fan_failed", serde_json::json!({}));
        r_doc["state"] = "resolved".into();
        let key = |d: &serde_json::Value| model::Alarm::from_json(d).unwrap().alert_key();
        let (a, b, x, r) = (key(&a_doc), key(&b_doc), key(&x_doc), key(&r_doc));
        let p = "0000000000000001".to_owned();
        let record = Memory::with(vec![
            kept(&r, &r_doc),
            kept(&p, &a_doc),
            kept(&x, &x_doc),
            kept(&a, &a_doc),
            kept(&b, &b_doc),
        ]);
        // Operations: 1 R's delete, 2 P's delete, 3 X's resolved put, 4 X's
        // delete, 5 A's put, 6 B's put. 2, 4 and 6 fail.
        let mut startup = super::Startup::default()
            .with_record(record.clone())
            .evaluating(["link_down", "disk_full"]);
        startup.fail_ops = [2, 4, 6].into();
        let (_b1, alarms) = builder(&s, startup).await;
        let report = alarms.adoption();
        assert_eq!(report.retired, [(r.clone(), super::Retired::Resolved)]);
        assert!(report.resolved.is_empty());
        assert_eq!(report.adopted, std::slice::from_ref(&a));
        let failed: Vec<&str> = report.failed.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(failed, [p.as_str(), x.as_str(), b.as_str()]);
        assert!(
            report.failed[0]
                .1
                .contains("injected failure of operation 2")
        );
        assert_eq!(alarms.held().await.keys().collect::<Vec<_>>(), [&a]);
        let saved = record.last();
        assert_eq!(saved.len(), 4, "A held, P, X and B kept: {saved:?}");
        assert_eq!(saved[&a], a_doc);
        assert_eq!(saved[&b], b_doc, "as found");
        assert_eq!(saved[&p], a_doc, "as found");
        assert_eq!(saved[&x]["state"], "resolved", "as far as its step went");
        assert!(!saved.contains_key(&r), "R's delete went out");

        // B raised by this instance supersedes what was found.
        alarms
            .raise(
                super::Condition::new("disk_full", Severity::Critical, "full")
                    .label("mount", "/var"),
            )
            .await
            .unwrap();
        assert_eq!(record.last()[&b]["severity"], "critical");

        // The next start, on what was saved, with nothing failing.
        let next = Memory::with(record.saves.lock().unwrap().last().unwrap().clone());
        let startup = super::Startup::default()
            .with_record(next.clone())
            .evaluating(["link_down", "disk_full"]);
        let (_b2, again) = builder(&s, startup).await;
        let report = again.adoption();
        let mut retired = report.retired.clone();
        retired.sort_by(|l, r| l.0.cmp(&r.0));
        let mut want = vec![
            (p.clone(), super::Retired::Phantom),
            (x.clone(), super::Retired::Resolved),
        ];
        want.sort_by(|l, r| l.0.cmp(&r.0));
        assert_eq!(
            retired, want,
            "X resolved already: deleted, not resolved again"
        );
        let mut adopted = report.adopted.clone();
        adopted.sort();
        let mut want = vec![a.clone(), b.clone()];
        want.sort();
        assert_eq!(adopted, want);
        assert!(report.failed.is_empty() && report.resolved.is_empty());
        assert_eq!(next.last().len(), 2);
    }

    /// §2.5: a resolve whose delete fails leaves the alarm held resolved,
    /// unconfirmed and recorded so; a raise then re-raises it, confirmed
    /// again, with no delete owed; and a resolve then deletes it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_delete_leaves_the_alarm_resolved_until_a_raise_or_a_resolve() {
        let s = alone().await;
        let record = std::sync::Arc::new(Memory::default());
        // No source: adoption makes no operation. 1 the resolved put, 2 its
        // delete, which fails.
        let mut startup = super::Startup::default().with_record(record.clone());
        startup.fail_ops = [2].into();
        let (_b, alarms) = builder(&s, startup).await;
        let a = || {
            super::Condition::new("link_down", Severity::Warning, "down")
                .label("peer", "r2")
                .label("port", "eth0")
        };
        let raised = alarms.raise(a()).await.unwrap();
        let key = raised.alert_key.clone();
        assert!(alarms.resolve(&key).await.is_err());
        let held = alarms.get(&key).await.expect("still held");
        assert_eq!(held.state, State::Resolved);
        assert_eq!(alarms.is_confirming(&key).await, Some(false));
        assert!(!alarms.set_confirming(&key, true).await, "never confirmed");
        assert_eq!(record.last()[&key]["state"], "resolved");

        let again = alarms.raise(a()).await.unwrap();
        assert_eq!(again.act, super::Act::Raised);
        assert!(again.document.timestamp > held.timestamp);
        assert_eq!(alarms.is_confirming(&key).await, Some(true));
        assert_eq!(record.last()[&key]["state"], "firing");

        let done = alarms.resolve(&key).await.unwrap().expect("held");
        assert_eq!(done.act, super::Act::Resolved);
        assert!(alarms.held().await.is_empty());
        assert!(record.last().is_empty());
    }
}
