//! `desired.v1` in the runtime: desired documents, from their author to
//! their target (`spec/profiles/desired/v1.md`, #840), and `desired.v2`
//! beside it (`spec/profiles/desired/v2.md`, #986's J3).
//!
//! **Both majors, by the author's revision** (`desired.v2` §2.18). An
//! author whose contract lists `desired.v2` writes each document with no
//! Zenoh timestamp, its version the put's in-band stamp
//! ([`Written::PutV2`]); its clock orders nothing, so nothing is caught up
//! at a restart, and its new instance's new run orders what it puts again;
//! its record holds no stamp; and its GETs are answered by snapshot, a
//! conditional read by the held stamp alone ([`Author::reads`]). A role
//! bound to such an author reads through [`zenkey_model::desired::v2`]'s
//! machine: each read conditional, one at a time, the watch on its
//! author's heartbeat in place of its period, a snapshot with no entry
//! judged by its run, and its report the packed `Report`
//! ([`Target::report_v2`], [`Target::watching`]). That half is in the `v2`
//! submodule, whose table lists its rules. The calls are the same three,
//! and so is the [`Handler`]: it refuses in words, which a `desired.v2`
//! report carries as indices into the report resource's two tables. What
//! follows is `desired.v1`'s, which a `desired.v2` role keeps wherever its
//! text says "v1 §n holds".
//!
//! Both halves of the profile's runtime. Every rule a fixture can check is
//! `zenkey-model`'s ([`zenkey_model::desired`]): the declaration, the
//! binding and the slice's key, the order of versions, the target's pure
//! two-phase state machine ([`model::Target`]), the report and its reading.
//! This module adds what that machine cannot do: the author's writes, its
//! diff and its record; the target's subscription, its seeds, its archive
//! read, its acts on the application's handler, and its report put.
//!
//! ```text
//! ServiceBuilder::desired_author(iface, resource, startup) ─▶ Author: the resource exposed and served;
//!        │      the previous instance's documents and deletes taken from the record, else an
//!        │      archive on its own side, and put again before the instance token (§2.11)
//!        │  put(target, doc) ─▶ put │ unchanged          delete(target) ─▶ deleted │ none
//! ServiceBuilder::desired_target(role, author, resource, report, handler, startup) ─▶ Target:
//!        │      the binding read (§2.3), the baseline report put before the tokens (§2.10)
//!        ▼ start(), after the tokens
//!   subscribe to the slice ─▶ seed: S4's GET (§2.5) ─▶ the author absent by a complete read:
//!   the archive's last-known (§2.9) ─▶ model::Target::observe ─▶ Step::Act ─▶ Handler::apply
//!   or revert, on a task of its own ─▶ model::Target::finish ─▶ the report put (§2.10)
//!   the author's instance token appears (a watch session, else a presence poll) ─▶ seed again
//! ServiceBuilder::desired_config(role, author, resource, report, startup): config.v1 as a target (§2.16)
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.2 the declaration; a target SHOULD refuse to bind a role to an invalid one | [`ServiceBuilder::desired_author`] and [`ServiceBuilder::desired_target`] read it with [`model::declaration_of`], and refuse anything but a valid one ([`Error::Contract`]) |
//! | §2.1 one document type per role: a target given a role on two or more desired resources MUST refuse to start (0.7, F-224) | [`ServiceBuilder::desired_target`] counts the author's desired resources the requirement names (every one for a requirement naming none, or a manifest's role, core §3.1); two or more are `two_documents`, refused ([`Error::NotExposed`]), and [`ServiceBuilder::start`] refuses too, nothing declared |
//! | §2.3 one author, each target parameter bound to `self.system` or `self.service`; the slice's key | [`model::binding`] over the role's configured binding: an invalid one refused, the start with it ([`Error::NotExposed`]), an optional one left unbound the kill switch |
//! | §2.4 the version is the author's stamp; a type MAY carry a revision | each sample's zenoh timestamp; the revision read by [`model::revision_of_text`] (a JSON payload, every number exact) or [`model::revision_of_value`] (CBOR, protobuf) |
//! | §2.5 subscribe, then seed, at start and each time the author's instance token appears | [`Target`]: the subscription on the slice's key, then S4's GET (target `All`, consolidation `None`), every sample and reply judged in the order it arrives on one task |
//! | §2.5 the author's instance token, never an interface token | the watch reads `zk2/<author>/@zk/instance/*` |
//! | §2.5 SHOULD seed again after a lost delivery | not detected: a zenoh subscriber does not say what it lost; [`TargetStartup::reread`] is the deployment's period, which covers it |
//! | §2.5 a GET that draws nothing is no verdict; R6 | nothing is observed from silence; a sample or reply on any key but the slice's is discarded |
//! | §2.5, §2.9, core §8.1 (0.37) the author absent: a complete read with no instance token | the seed's presence read on the target's session, which holds no liveliness subscriber; a withdrawal heard on the watch session is a reason to read, never absence; a read through a session cut off is not made ([`AuthorSeen::NotMade`]) |
//! | §2.6 the order of versions; §2.7 once per version, adopted when the same document | [`model::Target::observe`], fed each sample's stamp, kind, revision and [`model::Content::of_payload`] |
//! | §2.7 one act at a time, the latest held | the model; the held version's payload kept beside it |
//! | §2.7 refused whole: `undecodable`, `invalid`; `reach`, `sensitive` and the type's own | the runtime decodes (core §7.2, a JSON payload as strict JSON) and validates against the author's bundle (`invalid` at the first violation's pointer) before the [`Handler`], which refuses the rest |
//! | §2.7 two phases, on a task of its own | each act runs [`Handler::apply`] or [`Handler::revert`] on a spawned task: a handler that panics is `failed` (`apply`), what it applied unknown |
//! | §2.7 a restart acts again | a new service is a new [`Target`], holding nothing |
//! | §2.8 a delete reverts; never on silence | a delete or `reply_del` the model admits is [`Handler::revert`] |
//! | §2.9 last-known from an archive, reported `last_known` | [`TargetStartup::archive`]: read explicitly ([`crate::archive::last_known`]) when a seed drew nothing and a complete presence read shows the author absent |
//! | §2.10 the report put before the tokens, and after every change | the baseline put when the target is declared; again after each act, adoption, confirmation and call |
//! | §2.11 only on a content change, within an instance | [`Author::put`] compares by [`model::Content`], and puts nothing for the same document |
//! | §2.11 MUST NOT stamp at or below its last stamp | the service's [`Minter`], caught up from the record's latest stamp |
//! | §2.11 a restart from its record, else an archive on its side, never a GET of its own keys | [`AuthorStartup::record`], else [`AuthorStartup::archive`] ([`crate::archive::last_known_all`]) |
//! | §2.11 each document kept put again, each delete within the window deleted again, before a GET is answered with it, SHOULD before the instance token | [`ServiceBuilder::desired_author`], deletes then puts, in key order, before it returns; [`Pace::Spread`] puts them after, at a rate, the store answering only what is put |
//! | §2.11 a re-mint puts again what it keeps | [`crate::Service::new_epoch`] re-puts every document and re-deletes every delete within its window first |
//! | §2.12 the kill switch: an optional role left unbound | the report `{"source": "baseline"}` put before the tokens, and nothing read |
//! | §2.13 the cardinality bounds the targets | the resource's budget (core §2.7): a new target's first document past the bound is [`Error::Budget`] |
//! | §2.16 `config.v1` as a target | [`ServiceBuilder::desired_config`]: each act converges the document on the owner, its report the view's and the status's; a delete reverts every parameter the documents acted on named to its startup value, a failed revert keeping the names it did not restore; a wait on a change that never ends gives up `failed`, `apply`, 5 s past its deadline or when the service closes ([`crate::config_v1::DESIRED_WAIT_PAST_DEADLINE`]) |
//!
//! **The record is the application's** ([`Record`]), as `alarms.v1`'s is:
//! loaded once, at [`ServiceBuilder::desired_author`], and saved whole after
//! every put and every delete. It holds each document as put, by target,
//! and each delete within the tombstone window, each with its stamp, so a
//! restart catches its clock up (core §4.3). [`FileRecord`] is one record,
//! a JSON file.
//!
//! **What an archive gives a restart without a stamp** (core S1). A delete
//! is dropped, logged and named in [`Adoption::unstamped`]: it cannot be
//! dated within its window, and no unstamped sample is acted on. A document
//! is put again, its content being the author's own, and catches no clock
//! up. A record's entries carry `at_ms` beside their stamps, so a record's
//! delete with no stamp is dated still, and kept.
//!
//! **Pacing a large re-put** (§2.11's cost). [`Pace::All`], the default,
//! puts everything again before the builder returns, so before the instance
//! token. [`Pace::Spread`] puts the deletes first, then the documents in key
//! order on a task of its own at the rate given, from the declaration on:
//! the SHOULD of "before the instance token" is given up for the tail, and
//! the MUST holds, since the service's store answers a GET only with what
//! this instance has put. A target seeding meanwhile draws nothing for a key
//! not yet put, which is no verdict, and runs what it holds until the re-put
//! arrives; a document the application puts first is never put over.
//!
//! **Watching the author** (core §8.1, 0.37). With a watch session
//! ([`TargetStartup::watch`]), the target holds its liveliness subscriber on
//! the author's instance tokens there, without history unless the session
//! is in a namespace (its first seed reads what is held; core §8.1, 0.41),
//! settles it before its first seed, and
//! seeds whenever one appears or one is withdrawn; its presence reads go on
//! its own session, which holds none. While the watch session's barrier
//! does not stand, a presence read is possibly incomplete
//! ([`AuthorSeen::Incomplete`]), and the barrier is asked again before the
//! next one. Without one, it reads the author's
//! instance tokens every [`TargetStartup::poll`] (500 ms by default) and
//! seeds when the set it reads changes: an absence shorter than the period,
//! whose author comes back as the same instance, is missed, and so is any
//! version put during it, until the deployment's period re-reads
//! ([`TargetStartup::reread`]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use zenkey_model::authoring::Encoding as WireEncoding;
use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Contract, Resource};
use zenkey_model::desired as model;
use zenkey_model::desired::v2 as model2;
use zenkey_model::grammar::{Addr, IfaceId, KindToken, data_key};
use zenkey_model::schema::TypeId;
use zenkey_model::template::Bindings;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};
use zenoh::sample::{Sample, SampleKind};
use zenoh::time::Timestamp;

pub use zenkey_model::desired::v2::Major;
pub use zenkey_model::desired::{
    APPLY, Ended, INVALID, Outcome, REACH, Refusal, Report, SENSITIVE, Source, UNDECODABLE,
    Version, Via,
};
pub use zenkey_model::seed::Stamp;

pub use crate::state::Asked;

use crate::budget::Budget;
use crate::call::BoxFuture;
use crate::config_v1::DesiredEnd;
use crate::error::{Error, Result, zenoh};
use crate::implementation::{Implementation, resource_name};
use crate::service::ServiceBuilder;
use crate::state::{Minter, Stamped, StateWriter, Store};
use crate::writer::Writer;

mod v2;

pub use v2::{REVISION_LINE, REVISION_POINTER, Watching};

/// How often a target without a watch session reads its author's instance
/// tokens (the module doc), by default.
pub const PRESENCE_POLL: Duration = Duration::from_millis(500);

/// How many times a seed made when the author's instance token appears
/// retries a GET that drew nothing: a token can arrive before the route to
/// its owner's queryable does (as `archive.v1`'s alignment retries).
const SEED_RETRIES: u32 = 4;

/// A zenoh timestamp as the model orders it and a report writes it (§2.4,
/// core §4.2): its NTP64 time and its id's 128-bit value.
#[must_use]
pub fn stamp(ts: &Timestamp) -> Stamp {
    Stamp {
        time: ts.get_time().as_u64(),
        id: u128::from_le_bytes(ts.get_id().to_le_bytes()),
    }
}

/// The Zenoh timestamp a desired document's write carries: a desired
/// resource is on core §4.3's table, so its writer gives every write one,
/// which orders the author's versions (§2.4, §2.6).
fn time(s: Stamped) -> Result<Timestamp> {
    s.time.ok_or_else(|| {
        Error::Contract(
            "desired.v1: a desired resource's write carried no Zenoh timestamp, which core \
             §4.3's table gives it"
                .to_owned(),
        )
    })
}

/// Milliseconds since the Unix epoch now, on this host's clock.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// The major of the profile a revision's desired resources are of
/// (`desired.v2` §2.18): `desired.v2` where its contract lists it in
/// `uses`, else `desired.v1`.
fn major_of(c: &Contract) -> Major {
    Major::of(c).unwrap_or(Major::V1)
}

/// The profile's id, for a message.
fn profile(major: Major) -> &'static str {
    match major {
        Major::V1 => model::PROFILE,
        Major::V2 => model2::PROFILE,
    }
}

/// A resource's declaration (§2.2), refused unless valid: `desired.v1`'s
/// reading, and under `desired.v2` its one reason more, a horizon above 0
/// outside `freshness.v2` (`horizon_v1`).
fn declared(c: &Contract, r: &Resource) -> Result<Vec<model::TargetParam>> {
    let iface = &c.iface;
    let major = major_of(c);
    let p = profile(major);
    let declaration = match major {
        Major::V1 => model::declaration_of(r),
        Major::V2 => model2::declaration_of(c, r),
    };
    match declaration {
        model::Declaration::Declared(t) => Ok(t),
        model::Declaration::NotThisProfiles => Err(Error::Contract(format!(
            "{iface} {:?} declares no desired.target_param: it is no desired resource \
             ({p} §2.2)",
            resource_name(r)
        ))),
        model::Declaration::Invalid(why) => Err(Error::Contract(format!(
            "{iface} {:?}: its desired.target_param is invalid ({}), and no role is bound to \
             it ({p} §2.2)",
            resource_name(r),
            why.iter()
                .map(|w| w.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Whether a resource's payload is JSON text: a JSON Schema type on the
/// JSON wire (core §7.2), compared by its canonical form (§2.7).
fn json_wire(r: &Resource) -> bool {
    matches!(&r.body, Body::Data(d)
        if matches!(d.type_, TypeId::JsonSchema { .. })
            && !matches!(d.encoding, Some(WireEncoding::Cbor)))
}

// --- the author (§2.11) -----------------------------------------------------------

/// One document as a record keeps it (§2.11): its payload as put, and the
/// stamp it was put under: `None` under `desired.v2`, whose record holds no
/// stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptDocument {
    pub payload: Vec<u8>,
    pub stamp: Option<Timestamp>,
}

/// One delete as a record keeps it (§2.11): when it was made, on this
/// host's clock in milliseconds since the Unix epoch, and its stamp, `None`
/// under `desired.v2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptDelete {
    pub at_ms: u64,
    pub stamp: Option<Timestamp>,
}

/// What an author's record holds (§2.11): its documents and its deletes,
/// each by its target (the target parameters' values, joined by `/`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    pub documents: BTreeMap<String, KeptDocument>,
    pub deletes: BTreeMap<String, KeptDelete>,
    /// The major its author writes under. `Some(Major::V2)`: a
    /// `desired.v2` author's record, which holds no stamp (`desired.v2`
    /// §2.11). `None`: `desired.v1`'s, as before.
    pub major: Option<Major>,
}

impl Kept {
    /// The latest stamp kept, for the minter's catch-up (core §4.3).
    #[must_use]
    pub fn latest(&self) -> Option<Timestamp> {
        self.documents
            .values()
            .filter_map(|d| d.stamp)
            .chain(self.deletes.values().filter_map(|d| d.stamp))
            .max_by_key(|t| *t.get_time())
    }
}

/// An author's own persistent record (§2.11). The application holds it, as
/// `alarms.v1`'s owner holds its own ([`crate::alarms::Record`]): the runtime
/// loads it at start, and saves it whole after every change.
pub trait Record: Send + Sync + 'static {
    /// What the previous instance kept, as the last [`Record::save`] wrote
    /// it. `Ok(None)`: there is no record, and the archive, if any, is read
    /// instead. `Err` is logged, and read as no record.
    fn load(&self) -> BoxFuture<'_, std::result::Result<Option<Kept>, String>>;

    /// What the author keeps now, whole.
    fn save<'a>(&'a self, kept: &'a Kept) -> BoxFuture<'a, std::result::Result<(), String>>;
}

/// A [`Record`] in a JSON file: `{"profile": "desired.v1", "documents":
/// {<target>: {"stamp": "<time>/<id>", "document": <JSON>}}, "deletes":
/// {<target>: {"stamp": …, "at_ms": <n>}}}`. A document that is not JSON is
/// kept as `"hex": "<its bytes>"` instead of `"document"`. A missing file is
/// no record; it is written whole, through a temporary file renamed. A
/// `desired.v2` author's says `"profile": "desired.v2"` and holds no
/// `"stamp"` (`desired.v2` §2.11).
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

    /// The file `kept` makes.
    #[must_use]
    pub fn render(kept: &Kept) -> Value {
        let v2 = kept.major == Some(Major::V2);
        // desired.v2 §2.11: the record holds no stamp.
        let stamped = |s: &Option<Timestamp>| {
            if v2 {
                serde_json::json!({})
            } else {
                serde_json::json!({"stamp": s.map_or(Value::Null, |t| Value::String(t.to_string()))})
            }
        };
        let documents: serde_json::Map<String, Value> = kept
            .documents
            .iter()
            .map(|(t, d)| {
                let mut entry = stamped(&d.stamp);
                match serde_json::from_slice::<Value>(&d.payload) {
                    Ok(doc) => entry["document"] = doc,
                    Err(_) => {
                        entry["hex"] =
                            Value::String(d.payload.iter().map(|b| format!("{b:02x}")).collect());
                    }
                }
                (t.clone(), entry)
            })
            .collect();
        let deletes: serde_json::Map<String, Value> = kept
            .deletes
            .iter()
            .map(|(t, d)| {
                let mut entry = stamped(&d.stamp);
                entry["at_ms"] = serde_json::json!(d.at_ms);
                (t.clone(), entry)
            })
            .collect();
        let profile = profile(if v2 { Major::V2 } else { Major::V1 });
        serde_json::json!({"profile": profile, "documents": documents, "deletes": deletes})
    }

    /// What a record's file holds. Members other than `documents` and
    /// `deletes` are ignored, and either may be missing.
    ///
    /// # Errors
    ///
    /// The file is not JSON, or an entry is not of the form above.
    pub fn parse(text: &str) -> std::result::Result<Kept, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        let stamp = |t: &str, e: &Value| -> std::result::Result<Option<Timestamp>, String> {
            match &e["stamp"] {
                Value::Null => Ok(None),
                Value::String(s) => s
                    .parse::<Timestamp>()
                    .map(Some)
                    .map_err(|e| format!("{t}: the stamp {s:?}: {e:?}")),
                _ => Err(format!("{t}: the stamp is not a string")),
            }
        };
        let mut kept = Kept {
            major: (v.get("profile").and_then(Value::as_str) == Some(model2::PROFILE))
                .then_some(Major::V2),
            ..Kept::default()
        };
        if let Some(docs) = v.get("documents") {
            let docs = docs.as_object().ok_or("\"documents\" is not an object")?;
            for (t, e) in docs {
                let payload = match (e.get("document"), e.get("hex").and_then(Value::as_str)) {
                    (Some(doc), _) => serde_json::to_vec(doc).map_err(|e| format!("{t}: {e}"))?,
                    (None, Some(h)) => unhex(h).ok_or_else(|| format!("{t}: not hex"))?,
                    (None, None) => return Err(format!("{t}: no document")),
                };
                kept.documents.insert(
                    t.clone(),
                    KeptDocument {
                        payload,
                        stamp: stamp(t, e)?,
                    },
                );
            }
        }
        if let Some(dels) = v.get("deletes") {
            let dels = dels.as_object().ok_or("\"deletes\" is not an object")?;
            for (t, e) in dels {
                let at_ms = e["at_ms"]
                    .as_u64()
                    .ok_or_else(|| format!("{t}: no at_ms"))?;
                kept.deletes.insert(
                    t.clone(),
                    KeptDelete {
                        at_ms,
                        stamp: stamp(t, e)?,
                    },
                );
            }
        }
        Ok(kept)
    }
}

fn unhex(h: &str) -> Option<Vec<u8>> {
    if !h.len().is_multiple_of(2) {
        return None;
    }
    (0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(h.get(i..i + 2)?, 16).ok())
        .collect()
}

impl Record for FileRecord {
    fn load(&self) -> BoxFuture<'_, std::result::Result<Option<Kept>, String>> {
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

    fn save<'a>(&'a self, kept: &'a Kept) -> BoxFuture<'a, std::result::Result<(), String>> {
        Box::pin(async move {
            let text = serde_json::to_vec_pretty(&Self::render(kept)).map_err(|e| e.to_string())?;
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

/// How an author puts again what it keeps, at a restart (§2.11, the module
/// doc).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pace {
    /// Everything before [`ServiceBuilder::desired_author`] returns: before
    /// the instance token.
    #[default]
    All,
    /// The deletes first, then at most `per_second` documents a second, on a
    /// task of its own, from the declaration on.
    Spread { per_second: u32 },
}

/// What an author starts from (§2.11). The default takes nothing over.
#[derive(Clone, Default)]
pub struct AuthorStartup {
    /// Its own persistent record, loaded at start and kept current.
    pub record: Option<Arc<dyn Record>>,
    /// An archive on its own side (core §4.4), and the read's timeout: read
    /// explicitly when the record has nothing.
    pub archive: Option<(Addr, Duration)>,
    /// How the re-put is paced.
    pub pace: Pace,
}

impl std::fmt::Debug for AuthorStartup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorStartup")
            .field("record", &self.record.is_some())
            .field("archive", &self.archive)
            .field("pace", &self.pace)
            .finish()
    }
}

impl AuthorStartup {
    /// With `record` as its persistent record.
    #[must_use]
    pub fn with_record(mut self, record: Arc<dyn Record>) -> Self {
        self.record = Some(record);
        self
    }

    /// With the archive at `archive` on its own side, read within `timeout`.
    #[must_use]
    pub fn with_archive(mut self, archive: Addr, timeout: Duration) -> Self {
        self.archive = Some((archive, timeout));
        self
    }

    /// Paced as `pace` says.
    #[must_use]
    pub fn paced(mut self, pace: Pace) -> Self {
        self.pace = pace;
        self
    }
}

/// Where a restart's documents came from (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Found {
    /// The author's own record.
    Record,
    /// An archive on its own side.
    Archive,
    /// Neither: a new epoch (core §4.3).
    #[default]
    Nothing,
}

impl Found {
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

/// What a restart did with what it found (§2.11), by target.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Adoption {
    pub found: Found,
    /// Each document put again (or, paced, to be put again).
    pub reput: Vec<String>,
    /// Each delete made again, within its window.
    pub redeleted: Vec<String>,
    /// Each delete past its window, dropped from the record.
    pub expired: Vec<String>,
    /// Each delete an archive holds with no stamp, dropped and logged: it
    /// cannot be dated within its window, and core S1 acts on no unstamped
    /// sample.
    pub unstamped: Vec<String>,
    /// Each step that failed on the bus, and why: the document stays in the
    /// record for the next start.
    pub failed: Vec<(String, String)>,
}

/// What a write did (§2.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// Put, under this stamp: a `desired.v1` author's version, its Zenoh
    /// timestamp.
    Put(Timestamp),
    /// Nothing put: the same document is held (§2.11).
    Unchanged,
    /// Deleted, under this stamp (`desired.v1`).
    Deleted(Timestamp),
    /// Nothing deleted: no document is held for the target.
    Nothing,
    /// Put by a `desired.v2` author, under this version: the run and the
    /// seq its envelope carries (`desired.v2` §2.4). It carries no Zenoh
    /// timestamp.
    PutV2(model2::Version),
    /// Deleted by a `desired.v2` author, under this version.
    DeletedV2(model2::Version),
}

impl Written {
    /// The version a `desired.v2` author wrote under (`desired.v2` §2.4).
    #[must_use]
    pub fn version(&self) -> Option<model2::Version> {
        match self {
            Self::PutV2(v) | Self::DeletedV2(v) => Some(*v),
            _ => None,
        }
    }
}

/// One target's slice, as the author holds it.
struct Slot {
    writer: Arc<StateWriter>,
    state: SlotState,
}

enum SlotState {
    /// A document, put in this instance (or kept, waiting for a paced
    /// re-put: `pending`).
    Put {
        payload: Vec<u8>,
        content: model::Content,
        stamp: Option<Timestamp>,
        pending: bool,
    },
    /// Deleted at `at_ms`.
    Deleted {
        at_ms: u64,
        stamp: Option<Timestamp>,
    },
}

/// A service's authorship of one desired resource (§2.11), from
/// [`ServiceBuilder::desired_author`], and again from
/// [`crate::Service::desired_author`] once it runs.
///
/// A clone is the same handle. Calls are serialized: each finishes its put
/// before the next begins.
#[derive(Clone)]
pub struct Author {
    inner: Arc<AuthorInner>,
}

struct AuthorInner {
    iface: IfaceId,
    /// Which major its contract uses (`desired.v2` §2.18).
    major: Major,
    resource: Resource,
    targets: Vec<model::TargetParam>,
    session: zenoh::Session,
    addr: Addr,
    store: Arc<Store>,
    minter: Arc<Minter>,
    closed: tokio::sync::watch::Receiver<bool>,
    budget: Option<Arc<Budget>>,
    window: Duration,
    json: bool,
    record: Option<Arc<dyn Record>>,
    slots: tokio::sync::Mutex<BTreeMap<String, Slot>>,
    adoption: Mutex<Adoption>,
}

impl Author {
    /// Authors `resource` of `iface` on `b` (§2.11), then takes over the
    /// previous instance's documents and deletes.
    pub(crate) async fn declare(
        b: &mut ServiceBuilder,
        iface: &IfaceId,
        resource: &str,
        startup: AuthorStartup,
    ) -> Result<Self> {
        let addr = b.address()?.clone();
        let imp = &b
            .impls()
            .iter()
            .find(|s| s.imp.iface() == iface)
            .ok_or_else(|| Error::NotImplemented(iface.clone()))?
            .imp;
        let r = imp.resource(resource)?.clone();
        let major = major_of(imp.contract());
        let targets = declared(imp.contract(), &r)?;
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is not state ({} §2.2)",
                profile(major)
            )));
        }
        b.expose(iface, resource)?;
        b.serve_state(iface)?;
        let budget = b.budget_of(iface, resource);
        let window = b
            .config()
            .tombstone_window_s
            .map_or(crate::state::DEFAULT_WINDOW, Duration::from_secs);
        let (session, store, minter, closed) = b.state_parts();
        let a = Self {
            inner: Arc::new(AuthorInner {
                iface: iface.clone(),
                major,
                json: json_wire(&r),
                resource: r,
                targets,
                session,
                addr,
                store,
                minter,
                closed,
                budget,
                window,
                record: startup.record.clone(),
                slots: tokio::sync::Mutex::new(BTreeMap::new()),
                adoption: Mutex::new(Adoption::default()),
            }),
        };
        a.adopt(&startup).await?;
        b.desired_authors.push(a.clone());
        Ok(a)
    }

    /// The interface whose resource this author writes.
    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.inner.iface
    }

    /// The resource, as the descriptor names it.
    #[must_use]
    pub fn resource(&self) -> String {
        resource_name(&self.inner.resource)
    }

    /// Which major of the profile its contract uses (`desired.v2` §2.18).
    /// Under `desired.v2` a document's version is its in-band stamp: its
    /// writes carry no Zenoh timestamp, its clock orders nothing, and its
    /// GETs are answered by snapshot (`desired.v2` §2.4).
    #[must_use]
    pub fn major(&self) -> Major {
        self.inner.major
    }

    /// The Zenoh timestamp a write's version is under `desired.v1`; none
    /// under `desired.v2`, whose version is the write's in-band stamp.
    fn zenoh_time(&self, s: Stamped) -> Result<Option<Timestamp>> {
        match self.inner.major {
            Major::V1 => time(s).map(Some),
            Major::V2 => Ok(None),
        }
    }

    /// What the restart did (§2.11).
    #[must_use]
    pub fn adoption(&self) -> Adoption {
        self.inner.adoption.lock().expect("not poisoned").clone()
    }

    /// The tombstone window its deletes are answered in (core S3).
    #[must_use]
    pub fn window(&self) -> Duration {
        self.inner.window
    }

    /// The target parameters' values `target` names: two values joined by
    /// `/` for two target parameters (the system, then the service), one for
    /// one (§2.2).
    fn values(&self, target: &str) -> Result<Bindings> {
        let parts: Vec<&str> = target.split('/').collect();
        if parts.len() != self.inner.targets.len() {
            return Err(Error::Contract(format!(
                "{target:?} names no target of {}: {} value(s), joined by /, one per target \
                 parameter (desired.v1 §2.2)",
                self.resource(),
                self.inner.targets.len()
            )));
        }
        Ok(self
            .inner
            .targets
            .iter()
            .zip(parts)
            .map(|(t, v)| (t.param.clone(), vec![v.to_owned()]))
            .collect())
    }

    /// The key of `target`'s document (§2.3): the slice its binding gives.
    pub fn key(&self, target: &str) -> Result<String> {
        let values = self.values(target)?;
        let i = &self.inner;
        let chunks = i
            .resource
            .template
            .build(&values)
            .map_err(|e| Error::Contract(format!("{}: {e}", self.resource())))?;
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        Ok(data_key(&i.addr, &i.iface, i.resource.token, &refs)?.to_string())
    }

    /// How many GETs the service's state queryables answered whose
    /// selector reaches `target`'s document key (§2.3), whether or not a
    /// document was held: what `scenarios.md` §8 observes of an unbound
    /// target, which makes none (text 0.4, F-197). A lower bound once the
    /// service was asked on more than 1024 distinct selectors.
    pub fn gets(&self, target: &str) -> Result<u64> {
        let key = zenoh::key_expr::OwnedKeyExpr::try_from(self.key(target)?).map_err(zenoh)?;
        Ok(self.inner.store.asked_reaching(&key))
    }

    /// The GETs of `target`'s one document key the service answered, oldest
    /// first: for each, the held stamp it carried, or none, and whether the
    /// author answered with that stamp alone (`desired.v2` §2.5, a
    /// conditional re-read; `scenarios-v2.md`, "What a section observes of
    /// a read"). The latest 1,024 over every `desired.v2` desired resource
    /// the service authors: a GET of any other resource is not kept, so a
    /// `desired.v1` author's are none.
    pub fn reads(&self, target: &str) -> Result<Vec<Asked>> {
        let key = zenoh::key_expr::OwnedKeyExpr::try_from(self.key(target)?).map_err(zenoh)?;
        Ok(self.inner.store.reads_of(&key))
    }

    /// The documents held now, by target, each as put.
    pub async fn documents(&self) -> BTreeMap<String, Vec<u8>> {
        let slots = self.inner.slots.lock().await;
        slots
            .iter()
            .filter_map(|(t, s)| match &s.state {
                SlotState::Put { payload, .. } => Some((t.clone(), payload.clone())),
                SlotState::Deleted { .. } => None,
            })
            .collect()
    }

    /// Puts `target`'s document (§2.11), unless it is the document already
    /// held for it in this instance ([`Written::Unchanged`], nothing put):
    /// JSON compared by its canonical form, every number exact, any other
    /// payload byte for byte (§2.7). A new target past the resource's bound
    /// is [`Error::Budget`], nothing put (§2.13).
    pub async fn put(&self, target: &str, payload: impl Into<Vec<u8>>) -> Result<Written> {
        let payload = payload.into();
        let content = model::Content::of_payload(&payload, self.inner.json);
        let mut slots = self.inner.slots.lock().await;
        if let Some(Slot {
            state:
                SlotState::Put {
                    content: held,
                    pending: false,
                    ..
                },
            ..
        }) = slots.get(target)
            && *held == content
        {
            return Ok(Written::Unchanged);
        }
        let writer = match slots.get(target) {
            Some(s) => Arc::clone(&s.writer),
            None => Arc::new(self.writer(target).await?),
        };
        let put = writer.put(payload.clone()).await?;
        let stamp = self.zenoh_time(put)?;
        slots.insert(
            target.to_owned(),
            Slot {
                writer,
                state: SlotState::Put {
                    payload,
                    content,
                    stamp,
                    pending: false,
                },
            },
        );
        self.save(&slots).await;
        Ok(stamp.map_or(Written::PutV2(put.stamp), Written::Put))
    }

    /// [`Author::put`] of a value of the resource's JSON Schema type, encoded
    /// as the contract says.
    pub async fn put_value<T: serde::Serialize>(&self, target: &str, doc: &T) -> Result<Written> {
        let Body::Data(d) = &self.inner.resource.body else {
            return Err(Error::Contract(format!("{} is no state", self.resource())));
        };
        if !matches!(d.type_, TypeId::JsonSchema { .. }) {
            return Err(Error::Contract(format!(
                "{} is not of a JSON Schema type: put its bytes",
                self.resource()
            )));
        }
        let bytes = crate::writer::encode_value(doc, d.encoding)?;
        self.put(target, bytes).await
    }

    /// Deletes `target`'s document, on the author's own decision (§2.8,
    /// §2.11): [`Written::Nothing`] when none is held. Answered as a
    /// `reply_del` within the tombstone window (core S3), and kept in the
    /// record that long.
    pub async fn delete(&self, target: &str) -> Result<Written> {
        let mut slots = self.inner.slots.lock().await;
        let Some(slot) = slots.get_mut(target) else {
            return Ok(Written::Nothing);
        };
        if matches!(slot.state, SlotState::Deleted { .. }) {
            return Ok(Written::Nothing);
        }
        let deleted = slot.writer.delete().await?;
        let stamp = self.zenoh_time(deleted)?;
        slot.state = SlotState::Deleted {
            at_ms: now_ms(),
            stamp,
        };
        self.save(&slots).await;
        Ok(stamp.map_or(Written::DeletedV2(deleted.stamp), Written::Deleted))
    }

    /// Deletes `target`'s key whether or not this instance holds a document
    /// there: a delete it puts on the bus under its run, kept in the record
    /// as any delete is. [`Author::delete`] puts nothing where no document
    /// is held. This heals `desired.v2` §2.8's residual, a target cut off
    /// across both a delete past the window and its author's restart, which
    /// keeps the deleted document in force until it hears a delete under
    /// the new run (`scenarios-v2.md` §15 step 2).
    pub async fn delete_unheld(&self, target: &str) -> Result<Written> {
        let mut slots = self.inner.slots.lock().await;
        let writer = match slots.get(target) {
            Some(s) => Arc::clone(&s.writer),
            None => Arc::new(self.writer(target).await?),
        };
        let deleted = writer.delete().await?;
        let stamp = self.zenoh_time(deleted)?;
        slots.insert(
            target.to_owned(),
            Slot {
                writer,
                state: SlotState::Deleted {
                    at_ms: now_ms(),
                    stamp,
                },
            },
        );
        self.save(&slots).await;
        Ok(stamp.map_or(Written::DeletedV2(deleted.stamp), Written::Deleted))
    }

    /// A writer on `target`'s slice, as the service's own state writers are
    /// made: stamped, answered by its state queryables, within the
    /// resource's budget (core S1, S2, §2.7).
    async fn writer(&self, target: &str) -> Result<StateWriter> {
        let i = &self.inner;
        let values = self.values(target)?;
        let key = self.key(target)?;
        let ke = zenoh::key_expr::OwnedKeyExpr::try_from(key).map_err(zenoh)?;
        let w = Writer::declare(&i.session, &i.minter, ke, &i.resource, &values)
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

    /// What the record holds now: every document, and every delete within
    /// the window.
    fn kept(&self, slots: &BTreeMap<String, Slot>) -> Kept {
        let now = now_ms();
        let window = u64::try_from(self.inner.window.as_millis()).unwrap_or(u64::MAX);
        let mut kept = Kept {
            major: (self.inner.major == Major::V2).then_some(Major::V2),
            ..Kept::default()
        };
        for (t, s) in slots {
            match &s.state {
                SlotState::Put { payload, stamp, .. } => {
                    kept.documents.insert(
                        t.clone(),
                        KeptDocument {
                            payload: payload.clone(),
                            stamp: *stamp,
                        },
                    );
                }
                SlotState::Deleted { at_ms, stamp } if now.saturating_sub(*at_ms) < window => {
                    kept.deletes.insert(
                        t.clone(),
                        KeptDelete {
                            at_ms: *at_ms,
                            stamp: *stamp,
                        },
                    );
                }
                SlotState::Deleted { .. } => {}
            }
        }
        kept
    }

    async fn save(&self, slots: &BTreeMap<String, Slot>) {
        let Some(r) = &self.inner.record else {
            return;
        };
        if let Err(e) = r.save(&self.kept(slots)).await {
            tracing::warn!(
                "desired: the author's record was not saved, so a restart may not put again \
                 what this instance holds (§2.11): {e}"
            );
        }
    }

    /// Where the previous instance's documents are (§2.11): the record,
    /// else the archive, else nothing; and the targets of the archive's
    /// deletes that carry no stamp, dropped.
    async fn load(&self, startup: &AuthorStartup) -> (Found, Kept, Vec<String>) {
        let mut unstamped = Vec::new();
        if let Some(r) = &startup.record {
            match r.load().await {
                Ok(Some(k)) => return (Found::Record, k, unstamped),
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    "desired: the author's record could not be read, so it is read as none \
                     (§2.11): {e}"
                ),
            }
        }
        if let Some((archive, timeout)) = &startup.archive {
            let pattern = self.pattern();
            match crate::archive::last_known_all(&self.inner.session, archive, &pattern, *timeout)
                .await
            {
                Ok(found) => {
                    let mut kept = Kept::default();
                    for lk in found {
                        let Some(target) = self.target_of(&lk.origin) else {
                            continue;
                        };
                        match lk.value {
                            Some(payload) => {
                                kept.documents.insert(
                                    target,
                                    KeptDocument {
                                        payload,
                                        stamp: lk.timestamp,
                                    },
                                );
                            }
                            None => {
                                // A delete with no stamp cannot be dated
                                // within its window, and core S1 acts on no
                                // unstamped sample: dropped, and said.
                                let Some(t) = lk.timestamp else {
                                    tracing::warn!(
                                        target,
                                        "desired: the archive holds a delete with no stamp, \
                                         which is not made again: it cannot be dated within \
                                         its window (§2.11), and an unstamped sample is never \
                                         acted on (core S1)"
                                    );
                                    unstamped.push(target);
                                    continue;
                                };
                                let at_ms = t
                                    .get_time()
                                    .to_system_time()
                                    .duration_since(UNIX_EPOCH)
                                    .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
                                    .unwrap_or(0);
                                kept.deletes.insert(
                                    target,
                                    KeptDelete {
                                        at_ms,
                                        stamp: Some(t),
                                    },
                                );
                            }
                        }
                    }
                    if !(kept.documents.is_empty() && kept.deletes.is_empty()) {
                        return (Found::Archive, kept, unstamped);
                    }
                }
                Err(e) => tracing::warn!(
                    "desired: the archive at {archive} could not be read, so nothing is taken \
                     from it (§2.11): {e}"
                ),
            }
        }
        (Found::Nothing, Kept::default(), unstamped)
    }

    /// Every member of the resource, as one key expression.
    fn pattern(&self) -> String {
        let i = &self.inner;
        let tail: Vec<String> = i
            .resource
            .template
            .segments()
            .iter()
            .map(|s| match s {
                zenkey_model::template::Segment::Literal(l) => l.clone(),
                zenkey_model::template::Segment::Param(_) => "*".to_owned(),
                zenkey_model::template::Segment::Rest(_) => "**".to_owned(),
            })
            .collect();
        format!(
            "zk2/{}/{}/{}/{}/{}",
            i.addr.system,
            i.addr.service,
            i.iface,
            i.resource.token,
            tail.join("/")
        )
    }

    /// The target a key of the resource is the document of.
    fn target_of(&self, key: &str) -> Option<String> {
        let zenkey_model::grammar::ZkKey::Data { resource, .. } =
            zenkey_model::grammar::parse(key).ok()?
        else {
            return None;
        };
        let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
        let values = self.inner.resource.template.matches(&refs)?;
        let parts: Option<Vec<String>> = self
            .inner
            .targets
            .iter()
            .map(|t| values.get(&t.param)?.first().cloned())
            .collect();
        Some(parts?.join("/"))
    }

    /// §2.11: each delete within its window made again, then each document
    /// put again, unchanged, in key order, before anything is alive (or
    /// paced, [`Pace::Spread`]); the clock caught up first.
    async fn adopt(&self, startup: &AuthorStartup) -> Result<()> {
        let (found, kept, unstamped) = self.load(startup).await;
        // desired.v1 §2.11: never a stamp at or below its last. Under
        // desired.v2 no rule orders an author's stamps by its clock, and
        // core §4.3's catch-up plays no part: its new run orders them.
        if self.inner.major == Major::V1
            && let Some(last) = kept.latest()
        {
            self.inner.minter.catch_up(last);
        }
        let mut report = Adoption {
            found,
            unstamped,
            ..Adoption::default()
        };
        let now = now_ms();
        let window = u64::try_from(self.inner.window.as_millis()).unwrap_or(u64::MAX);
        let mut slots = self.inner.slots.lock().await;
        for (target, d) in &kept.deletes {
            let age = now.saturating_sub(d.at_ms);
            if age >= window {
                report.expired.push(target.clone());
                continue;
            }
            let done = async {
                let w = Arc::new(self.writer(target).await?);
                let stamp = self.zenoh_time(w.delete_aged(Duration::from_millis(age)).await?)?;
                Ok::<_, Error>((w, stamp))
            };
            match done.await {
                Ok((writer, stamp)) => {
                    slots.insert(
                        target.clone(),
                        Slot {
                            writer,
                            state: SlotState::Deleted {
                                at_ms: d.at_ms,
                                stamp,
                            },
                        },
                    );
                    report.redeleted.push(target.clone());
                }
                Err(e) => {
                    tracing::warn!(target, "desired: a kept delete was not made again: {e}");
                    report.failed.push((target.clone(), e.to_string()));
                }
            }
        }
        let mut paced = Vec::new();
        for (target, d) in &kept.documents {
            let content = model::Content::of_payload(&d.payload, self.inner.json);
            let writer = match self.writer(target).await {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    report.failed.push((target.clone(), e.to_string()));
                    continue;
                }
            };
            if let Pace::Spread { .. } = startup.pace {
                slots.insert(
                    target.clone(),
                    Slot {
                        writer,
                        state: SlotState::Put {
                            payload: d.payload.clone(),
                            content,
                            stamp: d.stamp,
                            pending: true,
                        },
                    },
                );
                paced.push(target.clone());
                report.reput.push(target.clone());
                continue;
            }
            match writer
                .put(d.payload.clone())
                .await
                .and_then(|s| self.zenoh_time(s))
            {
                Ok(stamp) => {
                    slots.insert(
                        target.clone(),
                        Slot {
                            writer,
                            state: SlotState::Put {
                                payload: d.payload.clone(),
                                content,
                                stamp,
                                pending: false,
                            },
                        },
                    );
                    report.reput.push(target.clone());
                }
                Err(e) => {
                    tracing::warn!(target, "desired: a kept document was not put again: {e}");
                    report.failed.push((target.clone(), e.to_string()));
                }
            }
        }
        if found != Found::Nothing {
            tracing::info!(
                found = found.as_str(),
                reput = report.reput.len(),
                redeleted = report.redeleted.len(),
                expired = report.expired.len(),
                failed = report.failed.len(),
                "desired: the previous instance's documents, put again (§2.11)"
            );
            // A failed step keeps its document in the record for the next
            // start: saved with what is held, and what failed as found.
            let mut k = self.kept(&slots);
            for (t, _) in &report.failed {
                if let Some(d) = kept.documents.get(t) {
                    k.documents.entry(t.clone()).or_insert_with(|| d.clone());
                } else if let Some(d) = kept.deletes.get(t) {
                    k.deletes.entry(t.clone()).or_insert_with(|| d.clone());
                }
            }
            if let Some(r) = &self.inner.record
                && let Err(e) = r.save(&k).await
            {
                tracing::warn!("desired: the author's record was not saved (§2.11): {e}");
            }
        }
        drop(slots);
        *self.inner.adoption.lock().expect("not poisoned") = report;
        if let Pace::Spread { per_second } = startup.pace
            && !paced.is_empty()
        {
            let me = self.clone();
            let period = Duration::from_secs(1) / per_second.max(1);
            tokio::spawn(async move { me.reput_paced(paced, period).await });
        }
        Ok(())
    }

    /// The paced tail of the re-put ([`Pace::Spread`]): each document still
    /// pending put again, one each `period`, until the service closes. A
    /// document written since is skipped.
    async fn reput_paced(self, targets: Vec<String>, period: Duration) {
        for target in targets {
            if *self.inner.closed.borrow() {
                return;
            }
            {
                let mut slots = self.inner.slots.lock().await;
                let Some(slot) = slots.get_mut(&target) else {
                    continue;
                };
                let SlotState::Put {
                    payload,
                    stamp,
                    pending,
                    ..
                } = &mut slot.state
                else {
                    continue;
                };
                if !*pending {
                    continue;
                }
                match slot
                    .writer
                    .put(payload.clone())
                    .await
                    .and_then(|s| self.zenoh_time(s))
                {
                    Ok(s) => {
                        *stamp = s;
                        *pending = false;
                    }
                    Err(e) => {
                        tracing::warn!(target, "desired: a kept document was not put again: {e}");
                    }
                }
                self.save(&slots).await;
            }
            tokio::time::sleep(period).await;
        }
    }

    /// §2.11, a re-mint: every document held put again, unchanged, under a
    /// fresh stamp of the same session, and every delete within its window
    /// made again, before the new instance's descriptor and tokens.
    pub(crate) async fn reput_all(&self) -> Result<()> {
        let mut slots = self.inner.slots.lock().await;
        let now = now_ms();
        let window = u64::try_from(self.inner.window.as_millis()).unwrap_or(u64::MAX);
        for slot in slots.values_mut() {
            match &mut slot.state {
                SlotState::Put {
                    payload,
                    stamp,
                    pending,
                    ..
                } => {
                    *stamp = self.zenoh_time(slot.writer.put(payload.clone()).await?)?;
                    *pending = false;
                }
                SlotState::Deleted { at_ms, stamp } if now.saturating_sub(*at_ms) < window => {
                    let age = Duration::from_millis(now.saturating_sub(*at_ms));
                    *stamp = self.zenoh_time(slot.writer.delete_aged(age).await?)?;
                }
                SlotState::Deleted { .. } => {}
            }
        }
        self.save(&slots).await;
        Ok(())
    }
}

// --- the target (§2.5–§2.10) ------------------------------------------------------

/// Where a target puts its report (§2.10): a state resource of its own
/// contract typed `desired.v1.Report`, and the member.
#[derive(Debug, Clone)]
pub struct ReportAt {
    pub iface: IfaceId,
    /// As the descriptor names it: `state/desired/task`.
    pub resource: String,
    /// The member's template parameters, if it has any.
    pub values: Bindings,
}

impl ReportAt {
    /// The member of `resource` of `iface` with no template parameters.
    #[must_use]
    pub fn new(iface: IfaceId, resource: impl Into<String>) -> Self {
        Self {
            iface,
            resource: resource.into(),
            values: Bindings::new(),
        }
    }
}

/// What a target starts from (§2.5, §2.9).
#[derive(Debug, Clone)]
pub struct TargetStartup {
    /// An archive the target reads its slice's last-known from, and the
    /// read's timeout, when a seed draws nothing and a complete presence
    /// read shows the author absent (§2.9). `None`: no last-known.
    pub archive: Option<(Addr, Duration)>,
    /// A watch session of its own (core §8.1, 0.37): where the target holds
    /// the liveliness subscriber on its author's instance tokens. `None`:
    /// it reads them every [`TargetStartup::poll`] instead (the module doc).
    pub watch: Option<zenoh::Session>,
    /// The deployment's period of re-reads (§2.5): a seed every period.
    /// `None`: none.
    pub reread: Option<Duration>,
    /// The timeout of every GET and presence read (core §8.1: the
    /// caller's), 1 s by default: the read of its author's descriptor for
    /// the catalog it names among them (`desired.v2` §2.5; #1146).
    pub timeout: Duration,
    /// How often presence is read without a watch session.
    pub poll: Duration,
    /// Reads its author's revision by the deployment's word alone, the
    /// author's [`Implementation`] given, and never by its descriptor:
    /// what a target built before `desired.v2` does (`desired.v2` §2.18,
    /// "Targets first"), which `scenarios-v2.md` §16's runs A and B play.
    /// Such a target is not `desired.v2`'s. `false` by default.
    pub word_alone: bool,
    /// The deployment's word that the target is across a constrained face
    /// from its author (core §8.1): no presence crosses, so no descriptor
    /// reaches it, and a read that draws none takes the deployment's word
    /// (`desired.v2` §2.18). Anywhere else descriptors reach the target,
    /// whatever a read drew, and a read that draws none finds none stating
    /// a run, so its versions wait (text 0.3, F-627). Set by
    /// [`TargetStartup::across`], and by a service on a first hop. `false`
    /// by default.
    pub across_face: bool,
}

impl Default for TargetStartup {
    fn default() -> Self {
        Self {
            archive: None,
            watch: None,
            reread: None,
            timeout: Duration::from_secs(1),
            poll: PRESENCE_POLL,
            word_alone: false,
            across_face: false,
        }
    }
}

impl TargetStartup {
    /// With the archive at `archive`, read within `timeout`.
    #[must_use]
    pub fn with_archive(mut self, archive: Addr, timeout: Duration) -> Self {
        self.archive = Some((archive, timeout));
        self
    }

    /// With `watch` as its watch session.
    #[must_use]
    pub fn with_watch(mut self, watch: zenoh::Session) -> Self {
        self.watch = Some(watch);
        self
    }

    /// With a re-read every `period`.
    #[must_use]
    pub fn with_reread(mut self, period: Duration) -> Self {
        self.reread = Some(period);
        self
    }

    /// Reading its author's revision by the deployment's word alone
    /// ([`TargetStartup::word_alone`]): a target built before
    /// `desired.v2`, played.
    #[must_use]
    pub fn by_word_alone(mut self) -> Self {
        self.word_alone = true;
        self
    }

    /// Across a constrained face (core §8.1, `link.v2` §2.20): every GET and
    /// presence read waits at least the face's `reply_timeout_ms`, and so
    /// does the archive's read, and no descriptor reaches the target
    /// ([`TargetStartup::across_face`]). Over RF, a reply came back 3.3–7.4 s after
    /// its GET, where the default [`TargetStartup::timeout`] is 1 s.
    #[must_use]
    pub fn across(mut self, far: &zenkey_model::link::FarSide) -> Self {
        self.across_face = true;
        self.timeout = far.at_least(self.timeout);
        if let Some((_, timeout)) = &mut self.archive {
            *timeout = far.at_least(*timeout);
        }
        self
    }
}

/// A put's document as an act hands it to the [`Handler`] (§2.7): decoded
/// as its type, and checked against its schema.
#[derive(Debug, Clone)]
pub struct Document {
    /// The slice's key.
    pub key: String,
    /// Its version under `desired.v1` (§2.4): its author's Zenoh timestamp,
    /// and its own revision. Under `desired.v2` a version has no Zenoh
    /// timestamp: `stamp` is zero here, `revision` the document's own, and
    /// [`Document::inband`] the version.
    pub version: Version,
    /// Its version under `desired.v2` (`desired.v2` §2.4): the run and the
    /// seq its put carried. `None` for a role bound to a `desired.v1`
    /// author.
    pub inband: Option<model2::Version>,
    /// Read from an archive (§2.9).
    pub last_known: bool,
    /// Its payload, as put.
    pub payload: Vec<u8>,
    /// Its zenoh `Encoding`, as zenoh spells it.
    pub encoding: String,
    /// Its value, for a JSON Schema type (read from JSON or CBOR) or a
    /// protobuf message (protobuf's JSON mapping); `None` for a raw type.
    pub value: Option<Value>,
}

impl Document {
    /// Its value as `T`.
    ///
    /// # Errors
    ///
    /// No value, or one that is not a `T`.
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> std::result::Result<T, String> {
        let v = self
            .value
            .as_ref()
            .ok_or("the document has no decoded value")?;
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())
    }
}

/// What a target's acts are made on: the application's (§2.7). The runtime
/// calls it for one act at a time, on a task of its own.
///
/// The futures are boxed ([`BoxFuture`]): write `Box::pin(async move { … })`.
pub trait Handler: Send + Sync + 'static {
    /// Checks `doc` and applies it, whole or not at all
    /// ([`Ended::Refused`], keeping what was in force: `reach`, `sensitive`,
    /// `invalid` or a code of the type's), or ends partway
    /// ([`Ended::Failed`], code `apply`). The runtime has refused a document
    /// that does not decode (`undecodable`) or that its schema refuses
    /// (`invalid`) before this is called.
    fn apply<'a>(&'a self, doc: &'a Document) -> BoxFuture<'a, Ended>;

    /// Applies the target's baseline: the document `version` deleted
    /// (§2.8). Never refused; one that could not finish is
    /// [`Ended::Failed`].
    fn revert<'a>(&'a self, version: &'a Version) -> BoxFuture<'a, Ended>;

    /// [`Handler::revert`] for a role bound to a `desired.v2` author
    /// (`desired.v2` §2.8): `version` is the delete's, or the tombstone's,
    /// and `None` for a snapshot with no entry under the held version's
    /// run, which names none. By default it is [`Handler::revert`] of a
    /// version with a zero stamp: a baseline rarely reads which delete
    /// brought it back.
    fn revert_v2<'a>(&'a self, version: Option<&'a model2::Version>) -> BoxFuture<'a, Ended> {
        let _ = version;
        self.revert(&v2::NO_STAMP)
    }
}

/// What a seed read of the author's presence (core §8.1, 0.37).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorSeen {
    /// Not read: the GET drew an answer, or no archive is configured.
    NotRead,
    /// An instance token of the author's.
    Present,
    /// A complete read with none: absent.
    Absent,
    /// None, in a read possibly incomplete: unobservable. A read cut off
    /// while it ran ([`crate::presence::PresenceRead::cut_off_during`],
    /// core §8.1, 0.38) is one: what it shows absent establishes nothing.
    /// So is one that ran to its session's interests timeout, or one made
    /// while the watch session's barrier did not stand (0.41).
    Incomplete,
    /// Not made, and why: its session is cut off from every other zenoh
    /// node, or holds a liveliness subscriber. Unobservable.
    NotMade(String),
}

/// What a target read of its author's revision (`desired.v2` §2.18): the
/// one its author's descriptor names for a version's run, retrieved by
/// its fingerprint (core §8.4) where it is not the one the target was
/// given, or the deployment's word where no descriptor reaches the target.
/// Read at the start and each time the author's instance token appears,
/// and for a version under a run read for the first time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorRevision {
    /// Not read: the kill switch, or a target that reads the deployment's
    /// word alone ([`TargetStartup::word_alone`]).
    NotRead,
    /// No descriptor of the author's reached the target, as across a
    /// constrained face: the revision the target was given, the
    /// deployment's word, stands.
    Word,
    /// The author's descriptor names the revision the target was given.
    Given,
    /// It names another revision of the author's interface, whose
    /// fingerprint this is, of the role's major: its versions are ordered
    /// as the role's are.
    Another(String),
    /// It names a revision of the other major, whose fingerprint this is,
    /// which the role's report cannot report. While it does, the role is
    /// stopped: the target acts on none of that author's versions under
    /// it, and its report stays as it was (§2.18).
    Stopped { fingerprint: String, major: Major },
    /// Not read: the GET of its descriptors failed, or a revision it
    /// names could not be retrieved. No word, so the deployment's stands.
    Unread,
}

/// What the last seed found (§2.5, §2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seeded {
    /// Replies the author's GET drew for the slice.
    pub answered: usize,
    /// What the presence read found.
    pub author: AuthorSeen,
    /// Whether the archive was read, and answered.
    pub archive: Option<bool>,
    /// What the target read of its author's revision (`desired.v2`
    /// §2.18): the revisions its author's descriptors name now, a stopped
    /// one first.
    pub revision: AuthorRevision,
}

/// How many times the read at an author's instance token is made again
/// while a run held before it is stated by none (`desired.v2` §2.18, text
/// 0.3, F-628).
const SEED_RETRIES_AT_TOKEN: u64 = 4;

/// Why a seed is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// At start.
    Start,
    /// The author's instance token appeared, or the poll's set changed.
    Appeared,
    /// A withdrawal heard on the watch session: read again (core §8.1).
    Withdrawn,
    /// The deployment's period.
    Reread,
    /// `desired.v2`'s watch: its author's heartbeat's section moved, or it
    /// expects none (`desired.v2` §2.5).
    Watch,
}

impl Why {
    /// Whether the seed reads every descriptor of the author's again
    /// first (§2.18): at the start, and when its instance tokens moved.
    fn reads_revisions(self) -> bool {
        matches!(self, Self::Start | Self::Appeared | Self::Withdrawn)
    }
}

/// One sample or reply of the slice, as received: a document under a zk2c
/// envelope (core §7.5) as its record ([`crate::inband`]). Its stamp is
/// its Zenoh timestamp, which orders an author's versions (§2.6) whatever
/// envelope it carries.
#[derive(Debug, Clone)]
struct Incoming {
    via: Via,
    put: bool,
    stamp: Option<Timestamp>,
    /// Its in-band stamp (core §4.3), when its envelope gave one: a
    /// `desired.v2` author's version (`desired.v2` §2.4).
    inband: Option<zenkey_model::wire::Stamp>,
    /// Heard on the slice's subscription, never read: what a `desired.v2`
    /// watch moves the digest it expects at.
    live: bool,
    payload: Vec<u8>,
    encoding: String,
    /// A put whose envelope was refused: undecodable (§2.7), its bytes
    /// never read as a document.
    refused: Option<crate::inband::Undecodable>,
    /// CH's tag, when it came in an envelope: a packed document is read by
    /// it (core §7.6).
    tag: Option<zenkey_model::wire::Tag>,
}

impl Incoming {
    fn of(s: &Sample, via: Via) -> Self {
        let read = crate::inband::read(s.clone());
        let (refused, tag) = (read.undecodable(), read.tag());
        let inband = match &read {
            crate::inband::Read::Enveloped { stamp, .. } => Some(*stamp),
            _ => None,
        };
        let s = read.sample();
        let put = s.kind() == SampleKind::Put;
        Self {
            via,
            put,
            stamp: s.timestamp().copied(),
            inband,
            live: false,
            payload: if put {
                s.payload().to_bytes().into_owned()
            } else {
                Vec::new()
            },
            encoding: s.encoding().to_string(),
            refused,
            tag,
        }
    }
}

enum Event {
    Sample(Incoming),
    Seed(Why),
    Ended(Ended),
    Called,
    /// `desired.v2`: a read of the slice ended, with what it drew.
    Read(v2::Read),
    /// `desired.v2`: an act ended, its refusal indexed.
    EndedV2(model2::Ended),
    /// `desired.v2`: a heartbeat of the author's.
    Beat(v2::Heard),
}

/// One role a target holds on a desired resource (§2.5–§2.10), from
/// [`ServiceBuilder::desired_target`] or [`ServiceBuilder::desired_config`],
/// and again from [`crate::Service::desired_target`] once it runs.
///
/// A clone is the same handle. It reads from [`crate::ServiceBuilder::start`]
/// on, until the service closes or drops.
#[derive(Clone)]
pub struct Target {
    inner: Arc<TargetInner>,
}

struct TargetInner {
    role: String,
    /// Which major the author's revision uses (`desired.v2` §2.18): the
    /// one the target was given, the deployment's word.
    major: Major,
    /// The author's interface, and the fingerprint of the revision of it
    /// the target was given.
    author_iface: IfaceId,
    given: String,
    /// What the author's descriptors named of its revision (§2.18).
    revisions: tokio::sync::Mutex<Revisions>,
    /// How many times an instance token of the author's appeared, as its
    /// watch or its poll heard it (§2.18: a token follows its descriptor,
    /// core §8.2).
    appearances: Arc<AtomicU64>,
    /// `desired.v2`'s half, for a role bound to an author of it.
    v2: Option<v2::V2>,
    /// The slice's key; `None` for the kill switch.
    key: Option<String>,
    author: Option<Addr>,
    bundle: Option<Bundle>,
    /// The resource's type reference in the bundle.
    ty: Option<Value>,
    json: bool,
    session: zenoh::Session,
    report: Option<StateWriter>,
    handler: Arc<dyn Handler>,
    startup: TargetStartup,
    core: tokio::sync::Mutex<Core>,
    tx: tokio::sync::mpsc::UnboundedSender<Event>,
    rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Event>>>,
    reread: tokio::sync::watch::Sender<Option<Duration>>,
    seeding: AtomicBool,
    again: AtomicBool,
    seeds: AtomicU64,
    acts: AtomicU64,
    last_seed: Mutex<Option<Seeded>>,
    /// The watch session's barrier, once its subscriber is declared (core
    /// §8.1, 0.41; #851): a presence read made while it does not stand is
    /// possibly incomplete.
    barrier: Mutex<Option<crate::presence::Barrier>>,
}

struct Core {
    model: model::Target,
    /// The running act's document, and the held one's.
    running: Option<Incoming>,
    waiting: Option<Incoming>,
}

/// What a target read of its author's revisions (`desired.v2` §2.18).
#[derive(Default)]
struct Revisions {
    /// What the author's descriptor names under each run read: from the
    /// last read of every descriptor, and for each version under a run
    /// that read did not hold.
    runs: BTreeMap<String, Named>,
    /// What the last read of every descriptor named, a run stated or not.
    sweep: Vec<Named>,
    /// The major of each revision retrieved, by fingerprint: `None` where
    /// it could not be had.
    majors: BTreeMap<String, Option<Major>>,
    /// The runs whose stop was logged.
    said: std::collections::BTreeSet<String>,
    /// The runs none of whose author's descriptors stated them when a
    /// version under them was read, where descriptors reach the target:
    /// their versions held until a read after the author's instance token
    /// appears (§2.18, "None states the run yet", text 0.3, F-621).
    pending: BTreeMap<String, Pending>,
    /// The runs a read after that token found stated by none: the
    /// deployment's word, kept across reads.
    by_word: std::collections::BTreeSet<String>,
}

/// A run's versions held while no descriptor states it (§2.18).
struct Pending {
    /// [`TargetInner::appearances`] when the read that found none was
    /// asked: a token appearing after it is one the next read follows.
    since: u64,
    /// Its versions, in the order they arrived, judged once it is read.
    held: Vec<Incoming>,
}

/// What a descriptor read named of the author's revision.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Named {
    /// No descriptor reached the target: the deployment's word.
    Word,
    /// A revision, by its fingerprint, and its major where it was had.
    Revision {
        fingerprint: String,
        major: Option<Major>,
    },
    /// No word: a read that failed.
    Unread,
}

/// The major of the report a report resource puts (§2.18): `desired.v2`'s
/// for a report resource of this major (its annotation, or the packed
/// `Report`), `desired.v1`'s for a `desired.v1.Report`; `None` for any
/// other, which §2.10's readings refuse.
fn report_major(b: &ServiceBuilder, at: &ReportAt) -> Result<Option<Major>> {
    let imp = &b
        .impls()
        .iter()
        .find(|s| s.imp.iface() == &at.iface)
        .ok_or_else(|| Error::NotImplemented(at.iface.clone()))?
        .imp;
    let r = imp.resource(&at.resource)?;
    if model2::report_declaration_of(imp.contract(), r)
        != model2::ReportDeclaration::NotThisProfiles
        || matches!(&r.body, Body::Data(d) if model2::is_report(&d.type_))
    {
        return Ok(Some(Major::V2));
    }
    Ok(match &r.body {
        Body::Data(d) if matches!(&d.type_, TypeId::JsonSchema { name, .. } if name == model::REPORT) => {
            Some(Major::V1)
        }
        _ => None,
    })
}

impl Target {
    /// Declares the role `role` on `b`, bound to `author`'s `resource`
    /// (§2.3), reporting at `report` (§2.10), acting on `handler`.
    pub(crate) async fn declare(
        b: &mut ServiceBuilder,
        role: &str,
        author: &Implementation,
        resource: &str,
        report: Option<ReportAt>,
        handler: Arc<dyn Handler>,
        mut startup: TargetStartup,
    ) -> Result<Self> {
        // A service on a first hop reaches its router only across a
        // constrained face (core §8.1): no presence crosses to it.
        startup.across_face |= b.config().first_hop;
        let own = b.address()?.clone();
        let (interface, optional) = b.role_of(role)?;
        let contract: &Contract = author.contract();
        if interface != contract.iface {
            return Err(Error::Contract(format!(
                "role {role:?} requires {interface}, not {}",
                contract.iface
            )));
        }
        let r = author.resource(resource)?.clone();
        // desired.v2 §2.18: a target reads which major its author's
        // revision uses, and orders its versions by that major's rule.
        let major = major_of(contract);
        let targets = declared(contract, &r)?;
        let binding = b.config().bindings.get(role).cloned().unwrap_or_default();
        let slice = model::Slice {
            iface: &contract.iface,
            token: r.token,
            template: &r.template,
            targets: &targets,
        };
        // One role per document type (§2.1, text 0.6, F-218): the
        // requirement names one desired resource of the author's
        // declaration; two or more refuse the role and the start, nothing
        // declared (text 0.7, F-224). A requirement naming none names every
        // resource (core §3.1), and so does a manifest's role.
        let names = b.role_resources(role)?;
        let documents = contract
            .resources
            .iter()
            .filter(|x| {
                names
                    .as_ref()
                    .is_none_or(|n| n.iter().any(|t| t == x.template.as_str()))
            })
            .filter(|x| model::declaration_of(x) != model::Declaration::NotThisProfiles)
            .count();
        let req = model::Requirement {
            bindings: &binding.providers,
            params: &binding.params,
            optional,
            documents,
        };
        let key = match model::binding(&slice, &req, &own) {
            model::Binding::Bound { key } => Some(key.to_string()),
            model::Binding::Unbound => None,
            model::Binding::Invalid(why) => {
                // Refused at the start, as core §8.2 step 2 refuses an
                // invalid declaration: `start()` refuses too, so a service
                // never starts without the role (§2.1, text 0.7, F-224).
                let why = format!(
                    "role {role:?}: its binding is invalid ({}) ({} §2.3)",
                    why.iter()
                        .map(|w| w.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    profile(major)
                );
                b.refused.push(why.clone());
                return Err(Error::NotExposed(why));
            }
        };
        // desired.v2 §2.18: one major's report alone cannot report a role
        // bound to an author of the other. That binding is invalid, so a
        // target given such an author's revision refuses to start, as
        // `two_documents` is refused, naming the role; nothing declared.
        if let Some(at) = &report
            && let Some(theirs) = report_major(b, at)?
            && theirs != major
        {
            let why = format!(
                "role {role:?}: its author's revision of {} uses {}, and its report, {} {:?}, \
                 is a {} report, which cannot report it (desired.v2 §2.18)",
                contract.iface,
                profile(major),
                at.iface,
                at.resource,
                profile(theirs)
            );
            b.refused.push(why.clone());
            return Err(Error::NotExposed(why));
        }
        let author_addr = key
            .as_ref()
            .and_then(|_| binding.providers.first()?.parse::<Addr>().ok());
        if let Some(w) = &startup.watch
            && w.zid() == b.session().zid()
        {
            return Err(Error::ReadBesideSubscriber {
                selector: format!("the instance tokens of role {role:?}'s author"),
                zid: w.zid().to_string(),
            });
        }
        let bundle = Bundle::verify(author.bundle_bytes()).ok();
        let ty = bundle.as_ref().and_then(|bd| {
            zenkey_model::decode::type_of(bd, &r.token.to_string(), r.template.as_str(), "type")
                .cloned()
        });
        // desired.v2 §2.5: a period below the read's timeout is refused at
        // the start, as an invalid binding is: such a target would always
        // hold a read outstanding.
        if major == Major::V2
            && key.is_some()
            && let Some(period) = startup.reread
            && period < startup.timeout
        {
            let why = period_below_timeout(role, period, startup.timeout);
            b.refused.push(why.clone());
            return Err(Error::NotExposed(why));
        }
        // desired.v2 §2.10: the report resource's declaration, and its two
        // tables, read before anything is declared.
        let tables = match (&report, major) {
            (Some(at), Major::V2) => Some(v2::tables_of(b, role, at)?),
            (None, Major::V2) => Some(model2::Tables::default()),
            (_, Major::V1) => None,
        };
        // §2.10: the first report before the tokens, the baseline's.
        let report = match report {
            Some(at) => {
                let w = b
                    .declare_state_writer(&at.iface, &at.resource, &at.values)
                    .await?;
                Some(w)
            }
            None => None,
        };
        let m = model::Target::new(key.clone());
        if let Some(w) = &report {
            match major {
                Major::V1 => w.put_value(&m.report().to_json()).await?,
                Major::V2 => w.put(v2::record(&model2::Report::baseline())?).await?,
            };
        }
        let v2 = tables.map(|t| v2::V2::new(b, contract, &r, key.as_deref(), t));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let t = Self {
            inner: Arc::new(TargetInner {
                role: role.to_owned(),
                major,
                author_iface: contract.iface.clone(),
                given: Fingerprint::of(contract).to_string(),
                revisions: tokio::sync::Mutex::new(Revisions::default()),
                appearances: Arc::default(),
                v2,
                key,
                author: author_addr,
                json: json_wire(&r),
                bundle,
                ty,
                session: b.session().clone(),
                report,
                handler,
                reread: tokio::sync::watch::Sender::new(startup.reread),
                startup,
                core: tokio::sync::Mutex::new(Core {
                    model: m,
                    running: None,
                    waiting: None,
                }),
                tx,
                rx: Mutex::new(Some(rx)),
                seeding: AtomicBool::new(false),
                again: AtomicBool::new(false),
                seeds: AtomicU64::new(0),
                acts: AtomicU64::new(0),
                last_seed: Mutex::new(None),
                barrier: Mutex::new(None),
            }),
        };
        b.desired_targets.push(t.clone());
        Ok(t)
    }

    /// The role.
    #[must_use]
    pub fn role(&self) -> &str {
        &self.inner.role
    }

    /// The slice's key (§2.3); `None` for the kill switch (§2.12).
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        self.inner.key.as_deref()
    }

    /// The author the role is bound to.
    #[must_use]
    pub fn author(&self) -> Option<&Addr> {
        self.inner.author.as_ref()
    }

    /// Which major the role's author uses (`desired.v2` §2.18): what
    /// orders its versions, how its slice is read, and which report the
    /// target puts.
    #[must_use]
    pub fn major(&self) -> Major {
        self.inner.major
    }

    /// The report as the target holds it now (§2.10), for a role bound to
    /// a `desired.v1` author. For one bound to a `desired.v2` author it
    /// stays the baseline's: that role's is [`Target::report_v2`].
    pub async fn report(&self) -> Report {
        self.inner.core.lock().await.model.report().clone()
    }

    /// The report of a role bound to a `desired.v2` author, as the target
    /// holds it now (`desired.v2` §2.10): what its packed `Report` says.
    /// `None` for a role bound to a `desired.v1` author.
    pub async fn report_v2(&self) -> Option<model2::Report> {
        let v2 = self.inner.v2.as_ref()?;
        Some(v2.core.lock().await.model.report())
    }

    /// The stamp the target's next conditional read carries (`desired.v2`
    /// §2.5): that of the latest version it admitted, `None` where it
    /// holds none, and for a role bound to a `desired.v1` author.
    pub async fn held(&self) -> Option<model2::Version> {
        let v2 = self.inner.v2.as_ref()?;
        v2.core.lock().await.model.held()
    }

    /// What the target's watch of its author's heartbeat holds
    /// (`desired.v2` §2.5): the section it expects, the heartbeats it
    /// read, the reads it asked for, and whether it reads none, so that
    /// the deployment's period stands. `None` for a role bound to a
    /// `desired.v1` author, and for the kill switch.
    pub async fn watching(&self) -> Option<Watching> {
        self.inner.key.as_ref()?;
        Some(self.inner.v2.as_ref()?.watching().await)
    }

    /// The GETs of its slice a role bound to a `desired.v2` author made
    /// so far, each a conditional read where it held a version.
    #[must_use]
    pub fn reads(&self) -> u64 {
        self.inner
            .v2
            .as_ref()
            .map_or(0, |v| v.gets.load(Ordering::Relaxed))
    }

    /// The seeds made so far.
    #[must_use]
    pub fn seeds(&self) -> u64 {
        self.inner.seeds.load(Ordering::Relaxed)
    }

    /// The acts started so far (§2.7).
    #[must_use]
    pub fn acts(&self) -> u64 {
        self.inner.acts.load(Ordering::Relaxed)
    }

    /// What the last seed found, once one has run.
    #[must_use]
    pub fn last_seed(&self) -> Option<Seeded> {
        self.inner.last_seed.lock().expect("not poisoned").clone()
    }

    /// Sets the deployment's period of re-reads (§2.5); `None`: none.
    ///
    /// # Errors
    /// For a role bound to a `desired.v2` author, a period below its
    /// read's timeout is refused, as the same period is at the start
    /// (`desired.v2` §2.5: such a target would always hold a read
    /// outstanding): [`Error::NotExposed`], in the start's words. The
    /// period the role had stands.
    pub fn set_reread(&self, period: Option<Duration>) -> Result<()> {
        if self.inner.major == Major::V2
            && self.inner.key.is_some()
            && let Some(p) = period
            && p < self.inner.startup.timeout
        {
            return Err(Error::NotExposed(period_below_timeout(
                &self.inner.role,
                p,
                self.inner.startup.timeout,
            )));
        }
        self.inner.reread.send_replace(period);
        Ok(())
    }

    /// The target's own operation changed what is in force (§2.7): its
    /// report's `source` becomes `call`, and is put.
    pub fn called(&self) {
        let _ = self.inner.tx.send(Event::Called);
    }

    /// Starts reading (§2.5), once: from [`crate::ServiceBuilder::start`],
    /// after the tokens, until `closed` says the service closed or drops.
    pub(crate) fn start(&self, closed: tokio::sync::watch::Receiver<bool>) {
        let Some(rx) = self.inner.rx.lock().expect("not poisoned").take() else {
            return;
        };
        if self.inner.key.is_none() {
            // §2.12: the kill switch reads and acts on nothing.
            return;
        }
        tokio::spawn(Arc::clone(&self.inner).run(rx, closed));
    }
}

/// Why a `desired.v2` role's period of re-reads is refused (§2.5): at the
/// start, and when one is set on a running target.
fn period_below_timeout(role: &str, period: Duration, timeout: Duration) -> String {
    format!(
        "role {role:?}: its period of re-reads, {period:?}, is below its read's timeout, \
         {timeout:?} (desired.v2 §2.5)"
    )
}

impl TargetInner {
    fn author_selector(&self) -> Option<String> {
        let a = self.author.as_ref()?;
        Some(format!("zk2/{}/{}/@zk/instance/*", a.system, a.service))
    }

    /// The major of the revision of the author's interface whose
    /// fingerprint is `fp` (§2.18): the role's for the one the target was
    /// given; else that revision, retrieved once by its fingerprint (core
    /// §8.4); `None` where it cannot be had.
    async fn major_named(&self, rv: &mut Revisions, fp: &str) -> Option<Major> {
        if fp == self.given {
            return Some(self.major);
        }
        if let Some(m) = rv.majors.get(fp) {
            return *m;
        }
        let m =
            crate::answer::retrieved(&self.session, &self.author_iface, fp, self.startup.timeout)
                .await
                .map(|c| major_of(&c));
        if m.is_none() {
            tracing::warn!(role = %self.role, fingerprint = fp, "desired: the revision the author's descriptor names was not retrieved, so the deployment's word stands for it (desired.v2 §2.18, core §8.4)");
        }
        rv.majors.insert(fp.to_owned(), m);
        m
    }

    /// Every descriptor the author serves now, read again (§2.18): at the
    /// start's seed, and at each seed after its instance token appears or
    /// is withdrawn. What each names under the run it states replaces what
    /// was held, so a stop lasts while a descriptor names it.
    ///
    /// A run held because no descriptor stated it (§2.18, "None states
    /// the run yet") is released once this read states it, or once it is
    /// a read after the author's instance token appeared that states it
    /// not, which leaves the deployment's word: its versions are judged
    /// again, in the order they arrived.
    async fn read_revisions(&self) {
        if self.startup.word_alone {
            return;
        }
        let Some(author) = &self.author else { return };
        let seen = self.appearances.load(Ordering::Acquire);
        // The read at a token MAY be made again, a few times and soon, as
        // v1 §2.5's seed is (text 0.3, F-628): a token can reach the target
        // before the route to its author's queryable does. Made again while
        // a run held before that token is stated by none, four times at
        // most, 100 ms more apart each time.
        let mut attempt = 0u64;
        let read = loop {
            let read = crate::answer::owner_revisions(
                &self.session,
                author,
                &self.author_iface,
                self.startup.timeout,
            )
            .await;
            let stated = |run: &String| match &read {
                crate::answer::OwnerRevisions::Named(named) => {
                    named.iter().any(|(r, _)| r.as_ref() == Some(run))
                }
                _ => false,
            };
            let unstated = self
                .revisions
                .lock()
                .await
                .pending
                .iter()
                .any(|(run, p)| seen > p.since && !stated(run));
            if !unstated || attempt == SEED_RETRIES_AT_TOKEN {
                break read;
            }
            attempt += 1;
            tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
        };
        let mut rv = self.revisions.lock().await;
        let mut runs = BTreeMap::new();
        let mut sweep = Vec::new();
        match read {
            crate::answer::OwnerRevisions::Named(named) => {
                for (run, fingerprint) in named {
                    let major = self.major_named(&mut rv, &fingerprint).await;
                    let n = Named::Revision { fingerprint, major };
                    if let Some(run) = run {
                        runs.insert(run, n.clone());
                    }
                    sweep.push(n);
                }
            }
            crate::answer::OwnerRevisions::Unreached => sweep.push(Named::Word),
            crate::answer::OwnerRevisions::Unread => sweep.push(Named::Unread),
        }
        let mut release = Vec::new();
        for (run, p) in std::mem::take(&mut rv.pending) {
            if runs.contains_key(&run) {
                release.extend(p.held);
            } else if seen > p.since {
                tracing::debug!(role = %self.role, %run, "desired: no descriptor of the author's states the run after its instance token appeared: the deployment's word stands for it (desired.v2 §2.18)");
                rv.by_word.insert(run);
                release.extend(p.held);
            } else {
                rv.pending.insert(run, p);
            }
        }
        for run in &rv.by_word {
            runs.entry(run.clone()).or_insert(Named::Unread);
        }
        rv.runs = runs;
        rv.sweep = sweep;
        drop(rv);
        for i in release {
            let _ = self.tx.send(Event::Sample(i));
        }
    }

    /// Whether a version under `run` may be judged (§2.18): not while the
    /// author's descriptor stating that run names a revision of the other
    /// major, which the role cannot report. A run the last read did not
    /// hold is read once ([`crate::answer::run_revision`]); a version
    /// with no run, no descriptor reaching the target, and a read that
    /// failed take the deployment's word.
    ///
    /// **None states the run yet** (text 0.3, F-621, F-627): where
    /// descriptors reach the target, anywhere but across a face
    /// ([`TargetStartup::across_face`]), a read that finds none stating the
    /// run, no descriptor at all among them, holds that run's versions,
    /// `held` with them, until [`TargetInner::read_revisions`] releases
    /// them. A run first seen while its instance's token stood is read so
    /// at once, and waits for the next appearance. A version read from an
    /// archive is never held, and takes the deployment's word where the
    /// run is not known.
    async fn admits(&self, run: Option<zenkey_model::wire::Run>, held: Option<&Incoming>) -> bool {
        if self.startup.word_alone {
            return true;
        }
        let (Some(run), Some(key)) = (run, &self.key) else {
            return true;
        };
        let at = run.to_string();
        // Last-known from an archive is never held (§2.18, §2.9): with its
        // author absent no descriptor will state its run, so the
        // deployment's word judges it. Its author's return is an
        // appearance, and its next run is read first.
        let archived = held.is_some_and(|i| i.via == Via::Archive);
        let mut rv = self.revisions.lock().await;
        if let Some(p) = rv.pending.get_mut(&at) {
            if archived {
                return true;
            }
            p.held.extend(held.cloned());
            return false;
        }
        let named = match rv.runs.get(&at) {
            Some(n) => n.clone(),
            None if archived => Named::Word,
            None => {
                let since = self.appearances.load(Ordering::Acquire);
                let read = crate::answer::run_revision(
                    &self.session,
                    key,
                    run,
                    &self.author_iface,
                    self.startup.timeout,
                )
                .await;
                use crate::answer::{OwnerRevision, RunRevision};
                // Descriptors answered and none states the run; or none
                // answered where descriptors reach the target, which is the
                // deployment's word, never what a read drew: anywhere but
                // across a face (F-627), an author not started yet included.
                let hold = match &read {
                    RunRevision::NotStated => true,
                    RunRevision::Read(OwnerRevision::Unreached) => !self.startup.across_face,
                    RunRevision::Read(_) => false,
                };
                if hold {
                    tracing::debug!(role = %self.role, run = %at, "desired: no descriptor of the author's states the run yet: its versions are held until a read after its instance token appears (desired.v2 §2.18)");
                    rv.pending.insert(
                        at,
                        Pending {
                            since,
                            held: held.into_iter().cloned().collect(),
                        },
                    );
                    return false;
                }
                let n = match read {
                    RunRevision::Read(OwnerRevision::Named(fingerprint)) => {
                        let major = self.major_named(&mut rv, &fingerprint).await;
                        Named::Revision { fingerprint, major }
                    }
                    RunRevision::Read(OwnerRevision::Unreached) => Named::Word,
                    RunRevision::Read(OwnerRevision::Unread) | RunRevision::NotStated => {
                        Named::Unread
                    }
                };
                rv.runs.insert(at.clone(), n.clone());
                n
            }
        };
        match named {
            Named::Revision {
                fingerprint,
                major: Some(m),
            } if m != self.major => {
                if rv.said.insert(at.clone()) {
                    tracing::warn!(role = %self.role, run = %at, fingerprint, "desired: the author's descriptor names a revision of {} that uses {}, which this role's report cannot report: the role is stopped, and acts on none of its versions under that run (desired.v2 §2.18)", self.author_iface, profile(m));
                }
                false
            }
            _ => true,
        }
    }

    /// What the target holds of its author's revision now (§2.18), for
    /// [`Seeded::revision`]: a stopped revision first, then one not
    /// retrieved, another, the one given, a read that failed, and no
    /// descriptor reaching it.
    async fn revision_now(&self) -> AuthorRevision {
        if self.startup.word_alone {
            return AuthorRevision::NotRead;
        }
        let rv = self.revisions.lock().await;
        let all: Vec<&Named> = rv.sweep.iter().chain(rv.runs.values()).collect();
        let revision = |f: &dyn Fn(&str, Option<Major>) -> bool| {
            all.iter().find_map(|n| match n {
                Named::Revision { fingerprint, major } if f(fingerprint, *major) => {
                    Some((fingerprint.clone(), *major))
                }
                _ => None,
            })
        };
        if let Some((fingerprint, Some(major))) =
            revision(&|_, m| m.is_some_and(|m| m != self.major))
        {
            return AuthorRevision::Stopped { fingerprint, major };
        }
        if revision(&|fp, m| fp != self.given && m.is_none()).is_some() {
            return AuthorRevision::Unread;
        }
        if let Some((fingerprint, _)) = revision(&|fp, _| fp != self.given) {
            return AuthorRevision::Another(fingerprint);
        }
        if revision(&|_, _| true).is_some() {
            return AuthorRevision::Given;
        }
        if all.contains(&&Named::Unread) {
            return AuthorRevision::Unread;
        }
        if all.contains(&&Named::Word) {
            return AuthorRevision::Word;
        }
        AuthorRevision::NotRead
    }

    async fn run(
        self: Arc<Self>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<Event>,
        mut closed: tokio::sync::watch::Receiver<bool>,
    ) {
        let Some(key) = self.key.clone() else { return };
        // §2.5: subscribe first, then seed.
        let tx = self.tx.clone();
        let slice = key.clone();
        let sub = self
            .session
            .declare_subscriber(key.clone())
            .callback(move |s: Sample| {
                // R6: a sample on any key but the slice's is discarded.
                if s.key_expr().as_str() == slice {
                    let mut heard = Incoming::of(&s, Via::Owner);
                    heard.live = true;
                    let _ = tx.send(Event::Sample(heard));
                }
            })
            .await;
        let _sub = match sub {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(role = %self.role, "desired: the slice's subscription failed, so nothing is read: {e}");
                return;
            }
        };
        let selector = self.author_selector();
        // The author's presence: a watch session's subscriber, or a poll.
        let mut _watch = None;
        let mut poll = None;
        match (&self.startup.watch, &selector) {
            (Some(w), Some(sel)) => {
                let tx = self.tx.clone();
                let appearances = Arc::clone(&self.appearances);
                // Without history unless the watch session is in a
                // namespace (core §8.1, 0.41; #851): the seed below reads
                // what is held, and the round trip after a history would
                // wait for all of it. A token held before the subscriber and
                // withdrawn after the round trip is heard, except through a
                // namespace, where zenoh 1.10.1 drops that withdrawal.
                let sub = crate::presence::declare_subscriber(w, sel.clone())
                    .history(crate::presence::subscriber_needs_history(w))
                    .callback(move |s: Sample| {
                        let why = if s.kind() == SampleKind::Put {
                            appearances.fetch_add(1, Ordering::AcqRel);
                            Why::Appeared
                        } else {
                            Why::Withdrawn
                        };
                        let _ = tx.send(Event::Seed(why));
                    })
                    .await;
                match sub {
                    Ok(s) => _watch = Some(s),
                    Err(e) => {
                        tracing::warn!(role = %self.role, "desired: the watch on the author failed: {e}")
                    }
                }
                // Core §8.1: one round trip on the watch session before the
                // seed on the other, kept and asked again before a read
                // while it does not stand.
                match crate::presence::Barrier::new(w, sel, self.startup.timeout).await {
                    Ok(b) => {
                        if !b.establish().await {
                            tracing::warn!(role = %self.role, "desired: the watch session's barrier was not established: presence reads are possibly incomplete until it is");
                        }
                        *self.barrier.lock().expect("not poisoned") = Some(b);
                    }
                    Err(e) => {
                        tracing::warn!(role = %self.role, "desired: the watch session's barrier could not be kept: {e}")
                    }
                }
            }
            (None, Some(sel)) => {
                poll = Some(tokio::spawn(Arc::clone(&self).poll(sel.clone())));
            }
            _ => {}
        }
        // desired.v2 §2.5: the author's heartbeat, where its desired
        // resource declares a horizon, in both forms' keys.
        let _beats = self.watch_heartbeats().await;
        let _ = self.tx.send(Event::Seed(Why::Start));
        let mut reread = self.reread.subscribe();
        let mut next = (*reread.borrow()).map(|p| tokio::time::Instant::now() + p);
        let barrier = self.barrier.lock().expect("not poisoned").clone();
        let mut moves_seen = barrier.as_ref().map_or(0, crate::presence::Barrier::moves);
        loop {
            let tick = async move {
                match next {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending::<()>().await,
                }
            };
            let moved = async {
                match &barrier {
                    Some(b) => b.moved_since(moves_seen).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                ev = rx.recv() => match ev {
                    Some(ev) => self.handle(ev).await,
                    None => break,
                },
                // The watch session's transport moved (core §8.1, 0.41;
                // #851): a subscriber without history hears no withdrawal
                // of what it was never told, and the author may have come
                // or gone meanwhile, so the seed reads it, as after an
                // appearance.
                () = moved => {
                    moves_seen = barrier.as_ref().map_or(moves_seen, crate::presence::Barrier::moves);
                    self.seed_soon(Why::Appeared);
                }
                () = tick => {
                    // desired.v2 §2.5: the watch in place of the period,
                    // where the author's heartbeat reaches the target.
                    if self.period_stands().await {
                        self.seed_soon(Why::Reread);
                    }
                    next = (*reread.borrow()).map(|p| tokio::time::Instant::now() + p);
                }
                changed = reread.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    next = (*reread.borrow()).map(|p| tokio::time::Instant::now() + p);
                }
                changed = closed.changed() => {
                    if changed.is_err() || *closed.borrow() {
                        break;
                    }
                }
            }
        }
        if let Some(p) = poll {
            p.abort();
        }
    }

    /// Reads the author's instance tokens every poll period, and asks a seed
    /// whenever the set read changes (the module doc). A read not made, or
    /// one possibly incomplete, changes nothing.
    async fn poll(self: Arc<Self>, selector: String) {
        let mut shown: Option<Vec<String>> = None;
        loop {
            // The first read at once: the set the start's seed was made
            // against.
            if let Ok(read) =
                crate::presence::liveliness_read(&self.session, &selector, self.startup.timeout)
                    .await
                && (read.complete || !read.keys.is_empty())
            {
                if shown.as_ref().is_some_and(|s| *s != read.keys) {
                    // A token the last read did not hold appeared (§2.18).
                    if shown
                        .as_ref()
                        .is_some_and(|s| read.keys.iter().any(|k| !s.contains(k)))
                    {
                        self.appearances.fetch_add(1, Ordering::AcqRel);
                    }
                    self.seed_soon(Why::Appeared);
                }
                shown = Some(read.keys);
            }
            tokio::time::sleep(self.startup.poll).await;
        }
    }

    /// Asks a seed: one runs at a time, and one asked meanwhile runs once
    /// after it.
    fn seed_soon(self: &Arc<Self>, why: Why) {
        if self.seeding.swap(true, Ordering::AcqRel) {
            self.again.store(true, Ordering::Release);
            return;
        }
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut why = why;
            loop {
                me.seed(why).await;
                if !me.again.swap(false, Ordering::AcqRel) {
                    break;
                }
                why = Why::Appeared;
            }
            me.seeding.store(false, Ordering::Release);
            // A request that raced the flag's release.
            if me.again.swap(false, Ordering::AcqRel) {
                me.seed_soon(Why::Appeared);
            }
        });
    }

    /// S4's GET of the slice (target `All`, consolidation `None`; #977's
    /// E3): every reply on its key, in the order it arrived, and whether
    /// it ran to its final reply. A desired resource is a per-key resource
    /// (core §4.2), answered with a reply per key; two on the key, two
    /// instances', are judged in turn, as §2.6 orders an author's versions.
    /// A snapshot's entry on the key stands for its reply (core §4.2, "What
    /// a profile reads").
    async fn get_owner(&self, key: &str) -> (Vec<Incoming>, bool) {
        let rx = match self
            .session
            .get(key)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::None)
            .timeout(self.startup.timeout)
            .with(flume::unbounded::<Reply>())
            .await
        {
            Ok(rx) => rx,
            Err(e) => {
                tracing::warn!(role = %self.role, "desired.v1: the seed's GET was refused: {e}");
                return (Vec::new(), false);
            }
        };
        let mut out = Vec::new();
        let mut complete = true;
        while let Ok(r) = rx.recv_async().await {
            match r.into_result() {
                // R6: a reply on any key but the slice's is discarded.
                Ok(s) if s.key_expr().as_str() == key => match crate::inband::read(s.clone()) {
                    crate::inband::Read::Snapshot {
                        sample,
                        page: Ok(page),
                    } => out.extend(
                        crate::inband::replies(&sample, &page)
                            .iter()
                            .filter(|m| m.key_expr().as_str() == key)
                            .map(|m| Incoming::of(m, Via::Owner)),
                    ),
                    _ => out.push(Incoming::of(&s, Via::Owner)),
                },
                Ok(_) => {}
                Err(_) => complete = false,
            }
        }
        (out, complete)
    }

    /// The author's presence, by one read on the target's session (core
    /// §8.1, 0.37): absent only when the read is complete and shows no
    /// instance token. One cut off while it ran is not complete (0.38), so
    /// it is [`AuthorSeen::Incomplete`], and no archive is read on it; a
    /// token it returned was seen, so present.
    async fn author_presence(&self) -> AuthorSeen {
        let Some(sel) = self.author_selector() else {
            return AuthorSeen::NotRead;
        };
        // A watch whose barrier does not stand may miss the author's next
        // move, so the read cannot stand either (core §8.1, 0.41).
        let barrier = self.barrier.lock().expect("not poisoned").clone();
        let settled = match barrier {
            Some(b) => b.establish().await,
            None => true,
        };
        match crate::presence::liveliness_read(&self.session, &sel, self.startup.timeout).await {
            Ok(r) if !r.keys.is_empty() => AuthorSeen::Present,
            Ok(r) if r.complete && settled => AuthorSeen::Absent,
            Ok(_) => AuthorSeen::Incomplete,
            Err(e) => AuthorSeen::NotMade(e.to_string()),
        }
    }

    /// One seed (§2.5, §2.9): S4's GET, retried while the author is present
    /// and it draws nothing after an appearance; when it draws nothing, the
    /// author's presence, and with the author absent by a complete read, the
    /// archive's last-known.
    async fn seed(self: &Arc<Self>, why: Why) {
        if self.v2.is_some() {
            return self.seed_v2(why).await;
        }
        let Some(key) = self.key.clone() else { return };
        self.seeds.fetch_add(1, Ordering::Relaxed);
        if why.reads_revisions() {
            self.read_revisions().await;
        }
        let mut attempt = 0;
        let (got, author) = loop {
            let (got, _complete) = self.get_owner(&key).await;
            if !got.is_empty() {
                break (got, AuthorSeen::NotRead);
            }
            // Nothing for the key: no verdict (core O5). Read presence when
            // it decides something: the archive, or a retry.
            let retry = matches!(why, Why::Appeared) && attempt < SEED_RETRIES;
            if self.startup.archive.is_none() && !retry {
                break (got, AuthorSeen::NotRead);
            }
            let author = self.author_presence().await;
            if author == AuthorSeen::Present && retry {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                continue;
            }
            break (got, author);
        };
        let answered = got.len();
        for i in got {
            let _ = self.tx.send(Event::Sample(i));
        }
        let mut archive = None;
        if answered == 0
            && author == AuthorSeen::Absent
            && let Some((at, timeout)) = &self.startup.archive
        {
            match crate::archive::last_known(&self.session, at, &key, *timeout).await {
                Ok(Some(lk)) => {
                    archive = Some(true);
                    let put = lk.value.is_some();
                    let _ = self.tx.send(Event::Sample(Incoming {
                        via: Via::Archive,
                        put,
                        stamp: lk.timestamp,
                        inband: lk.inband,
                        live: false,
                        payload: lk.value.unwrap_or_default(),
                        encoding: lk.encoding.map(|e| e.to_string()).unwrap_or_default(),
                        refused: None,
                        tag: lk.tag,
                    }));
                }
                Ok(None) => archive = Some(false),
                Err(e) => {
                    archive = Some(false);
                    tracing::warn!(role = %self.role, "desired.v1: the archive at {at} was not read (§2.9): {e}");
                }
            }
        }
        if let AuthorSeen::NotMade(why) = &author {
            tracing::info!(role = %self.role, "desired.v1: the author's presence was not read, so no absence is concluded and no archive read (core §8.1): {why}");
        }
        let revision = self.revision_now().await;
        *self.last_seed.lock().expect("not poisoned") = Some(Seeded {
            answered,
            author,
            archive,
            revision,
        });
    }

    /// The model's sample of one received.
    fn sample_of(&self, i: &Incoming) -> model::Sample {
        let (revision, content) = if i.put && i.refused.is_some() {
            // Never read as a document: its bytes alone.
            (None, Some(model::Content::of_payload(&i.payload, false)))
        } else if i.put {
            let revision = if self.json {
                model::revision_of_text(&i.payload)
            } else {
                self.decoded(i)
                    .ok()
                    .flatten()
                    .as_ref()
                    .and_then(model::revision_of_value)
            };
            (
                revision,
                Some(model::Content::of_payload(&i.payload, self.json)),
            )
        } else {
            (None, None)
        };
        model::Sample {
            via: i.via,
            mutation: if i.put {
                model::Mutation::Put
            } else {
                model::Mutation::Delete
            },
            stamp: i.stamp.as_ref().map(stamp),
            revision,
            content,
        }
    }

    /// A put's value as its type decodes it (core §7.2): `Ok(None)` for a
    /// raw type, `Err` with why for bytes that do not decode.
    fn decoded(&self, i: &Incoming) -> std::result::Result<Option<Value>, String> {
        if let Some(why) = i.refused {
            return Err(why.reason());
        }
        let (Some(bundle), Some(ty)) = (&self.bundle, &self.ty) else {
            return Ok(None);
        };
        if self.json {
            return model::strict_json(&i.payload)
                .map(Some)
                .ok_or_else(|| "not strict JSON (desired.v1 §2.7, §2.10's reading)".to_owned());
        }
        let enc = (!i.encoding.is_empty()).then_some(i.encoding.as_str());
        // A packed document by the tag its envelope carried, refused by its
        // layout as a consumer's admission refuses it (core §7.6).
        let cx = zenkey_model::decode::Context {
            tag: i.tag,
            epoch: None,
        };
        match zenkey_model::decode::decode_in(bundle, ty, enc, &i.payload, cx) {
            zenkey_model::decode::Rendered::Value(v) => Ok(Some(v)),
            zenkey_model::decode::Rendered::Opaque { .. } => Ok(None),
            zenkey_model::decode::Rendered::Undecodable { reason, .. } => Err(reason),
        }
    }

    async fn put_report(&self, report: &Report) {
        if let Some(w) = &self.report
            && let Err(e) = w.put_value(&report.to_json()).await
        {
            tracing::warn!(role = %self.role, "desired.v1: the report was not put (§2.10): {e}");
        }
    }

    async fn handle(self: &Arc<Self>, ev: Event) {
        if self.v2.is_some() {
            return self.handle_v2(ev).await;
        }
        match ev {
            // desired.v2's alone.
            Event::Read(_) | Event::EndedV2(_) | Event::Beat(_) => {}
            Event::Seed(why) => self.seed_soon(why),
            Event::Called => {
                let report = {
                    let mut c = self.core.lock().await;
                    c.model.call();
                    c.model.report().clone()
                };
                self.put_report(&report).await;
            }
            Event::Sample(i) => {
                // §2.18: never a version under a run whose revision the
                // role's report cannot report.
                if !self.admits(i.inband.map(|s| s.run), Some(&i)).await {
                    return;
                }
                let sample = self.sample_of(&i);
                let mut c = self.core.lock().await;
                let step = c.model.observe(sample);
                match step {
                    model::Step::Act(act) => {
                        c.running = Some(i);
                        let doc = c.running.clone();
                        drop(c);
                        self.act(act, doc);
                    }
                    model::Step::Held => c.waiting = Some(i),
                    model::Step::Adopted => {
                        let report = c.model.report().clone();
                        drop(c);
                        self.put_report(&report).await;
                    }
                    model::Step::Confirmed => {
                        // §2.9: put now unless an act runs, whose end puts it.
                        if c.model.running().is_none() {
                            let report = c.model.report().clone();
                            drop(c);
                            self.put_report(&report).await;
                        }
                    }
                    model::Step::Unstamped => {
                        tracing::warn!(role = %self.role, "desired.v1: an unstamped sample of the slice is never acted on (§2.6; its author broke core S1)");
                    }
                    model::Step::SameVersion | model::Step::Older | model::Step::Done => {}
                }
            }
            Event::Ended(ended) => {
                let mut c = self.core.lock().await;
                let step = c.model.finish(ended);
                c.running = None;
                let next = match step {
                    model::Step::Act(act) => {
                        c.running = c.waiting.take();
                        Some((act, c.running.clone()))
                    }
                    _ => {
                        c.waiting = None;
                        None
                    }
                };
                let report = c.model.report().clone();
                drop(c);
                self.put_report(&report).await;
                if let Some((act, doc)) = next {
                    self.act(act, doc);
                }
            }
        }
    }

    /// The revision a put's document carries (§2.4), when its type carries
    /// one.
    fn revision_of(&self, i: &Incoming) -> Option<u64> {
        if self.json {
            model::revision_of_text(&i.payload)
        } else {
            self.decoded(i)
                .ok()
                .flatten()
                .as_ref()
                .and_then(model::revision_of_value)
        }
    }

    /// Starts one act on a task of its own (§2.7); its end comes back as an
    /// event.
    fn act(self: &Arc<Self>, act: model::Act, doc: Option<Incoming>) {
        self.acts.fetch_add(1, Ordering::Relaxed);
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let ended = me.perform(act, doc).await;
            let _ = me.tx.send(Event::Ended(ended));
        });
    }

    async fn perform(&self, act: model::Act, doc: Option<Incoming>) -> Ended {
        let version = act.version();
        let handler = Arc::clone(&self.handler);
        let lost = |e: tokio::task::JoinError| {
            let mut r = Refusal::new(APPLY);
            r.detail = Some(format!(
                "the handler's act did not complete, so what it applied is unknown: {e}"
            ));
            Ended::Failed(r)
        };
        match act.kind {
            model::ActKind::Revert => {
                tracing::info!(role = %self.role, stamp = %version.stamp, "desired.v1: revert (§2.8)");
                tokio::spawn(async move { handler.revert(&version).await })
                    .await
                    .unwrap_or_else(lost)
            }
            model::ActKind::Apply => {
                let Some(i) = doc else {
                    return Ended::Failed(Refusal::new(APPLY));
                };
                tracing::info!(role = %self.role, stamp = %version.stamp, revision = ?version.revision, "desired.v1: act (§2.7)");
                let value = match self.decoded(&i) {
                    Ok(v) => v,
                    Err(why) => {
                        let mut r = Refusal::new(UNDECODABLE);
                        r.detail = Some(why);
                        return Ended::Refused(r);
                    }
                };
                // Its type refuses it: §2.7's `invalid`, at the first
                // violation.
                if let (Some(v), Some(bundle), Some(ty)) = (&value, &self.bundle, &self.ty)
                    && ty["kind"] == "jsonschema"
                    && let Err(violations) = zenkey_model::validate::validate(bundle, ty, v)
                {
                    let first = &violations[0];
                    let mut r = Refusal::new(INVALID);
                    r.pointer = Some(first.at.clone());
                    r.detail = Some(first.why.clone());
                    return Ended::Refused(r);
                }
                let doc = Document {
                    key: self.key.clone().unwrap_or_default(),
                    version,
                    inband: None,
                    last_known: act.via == Via::Archive,
                    payload: i.payload,
                    encoding: i.encoding,
                    value,
                };
                tokio::spawn(async move { handler.apply(&doc).await })
                    .await
                    .unwrap_or_else(lost)
            }
        }
    }
}

// --- config.v1 as a target (§2.16) ----------------------------------------------

/// `config.v1`'s owner as a target (§2.16), or `config.v2`'s, which its
/// text feeds so too (`config.v2` §2.18): a put converges the document on
/// the owner, a delete reverts what the documents set.
pub(crate) struct ConfigHandler {
    pub(crate) cfg: Configured,
}

/// The owner a [`ConfigHandler`] acts on.
pub(crate) enum Configured {
    V1(crate::config_v1::Configuration),
    V2(crate::config_v2::Configuration),
}

impl Configured {
    async fn converge_desired(&self, doc: crate::config_v1::DesiredConfig) -> DesiredEnd {
        match self {
            Self::V1(c) => c.converge_desired(doc).await,
            Self::V2(c) => c.converge_desired(doc).await,
        }
    }

    async fn revert_desired(&self) -> std::result::Result<(), DesiredEnd> {
        match self {
            Self::V1(c) => c.revert_desired().await,
            Self::V2(c) => c.revert_desired().await,
        }
    }
}

/// An RFC 6901 escape of one name.
fn escape(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

/// A `config.v1` refusal as the profile's (§2.16): its code, and the
/// pointer `/values/<group>`, then `/<parameter>`.
fn refusal_of(r: &zenkey_model::config::DesiredRefusal) -> Refusal {
    let code = serde_json::to_value(r.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| APPLY.to_owned());
    let mut pointer = format!("/values/{}", escape(&r.group));
    if let Some(p) = &r.parameter {
        pointer.push('/');
        pointer.push_str(&escape(p));
    }
    Refusal {
        code,
        pointer: Some(pointer),
        detail: None,
    }
}

/// A wait given up (`config.v1`'s pending change never ended, or the
/// service closed): `failed`, code `apply`, since what converged before it
/// stays applied (§2.7).
fn gave_up(why: String) -> Ended {
    Ended::Failed(Refusal {
        code: APPLY.to_owned(),
        pointer: None,
        detail: Some(why),
    })
}

/// How a document's convergence ended, in this profile's words (§2.16).
fn ended(end: DesiredEnd) -> Ended {
    use zenkey_model::config::DesiredOutcome;
    match end {
        DesiredEnd::Report(r) => match (r.outcome, &r.refusal) {
            (DesiredOutcome::Converged, _) => Ended::Done,
            (DesiredOutcome::Refused, Some(f)) => Ended::Refused(refusal_of(f)),
            (DesiredOutcome::Failed, Some(f)) => Ended::Failed(refusal_of(f)),
            (_, None) => Ended::Failed(Refusal::new(APPLY)),
        },
        DesiredEnd::GaveUp(why) => gave_up(why),
    }
}

impl Handler for ConfigHandler {
    fn apply<'a>(&'a self, doc: &'a Document) -> BoxFuture<'a, Ended> {
        Box::pin(async move {
            let d: crate::config_v1::DesiredConfig = match doc.decode() {
                Ok(d) => d,
                Err(why) => {
                    let mut r = Refusal::new(UNDECODABLE);
                    r.detail = Some(why);
                    return Ended::Refused(r);
                }
            };
            ended(self.cfg.converge_desired(d).await)
        })
    }

    fn revert<'a>(&'a self, _version: &'a Version) -> BoxFuture<'a, Ended> {
        Box::pin(async move {
            match self.cfg.revert_desired().await {
                Ok(()) => Ended::Done,
                // A revert is never refused (§2.8): its refusing `set` is a
                // failure, the groups before it restored.
                Err(DesiredEnd::Report(r)) => Ended::Failed(
                    r.refusal
                        .as_ref()
                        .map_or_else(|| Refusal::new(APPLY), refusal_of),
                ),
                Err(DesiredEnd::GaveUp(why)) => gave_up(why),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FileRecord, Kept, KeptDelete, KeptDocument, escape};

    /// Review of #1156: a state GET is kept for [`super::Author::reads`]
    /// only where it reads a member of a `desired.v2` desired resource.
    /// One on another state of the same service keeps nothing, with a held
    /// stamp or none, answered whole or by that stamp alone; one on the
    /// author's slice is kept, with the stamp it carried and how it was
    /// answered.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_get_is_kept_only_for_a_desired_v2_resource() {
        use crate::state::Asked;
        use std::time::Duration;
        use zenkey_model::wire;
        use zenoh::query::{ConsolidationMode, QueryTarget};

        // A session of its own: its GETs reach its own queryables.
        let mut c = zenoh::Config::default();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("listen/endpoints", "[]").unwrap();
        let s = zenoh::open(c).await.unwrap();
        let imp = |name: &str| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/contracts")
                .join(name);
            let l = zenkey_model::contract::load_path(&path);
            crate::Implementation::new(l.contract.unwrap_or_else(|| panic!("{}", l.report)))
        };
        let (tasking, sysinfo) = (imp("tasking.v2.toml"), imp("sysinfo.v1.toml"));
        let (ti, si) = (tasking.iface().clone(), sysinfo.iface().clone());
        let mut b = crate::ServiceBuilder::new(
            &s,
            crate::ServiceConfig::new("lab/author".parse().unwrap()),
        );
        b.implement(tasking).unwrap();
        b.implement(sysinfo).unwrap();
        let none = zenkey_model::template::Bindings::new();
        let _cpu = b.declare_writer(&si, "stream/cpu", &none).await.unwrap();
        let uptime = b
            .declare_state_writer(
                &si,
                "state/uptime",
                &zenkey_model::template::Bindings::new(),
            )
            .await
            .unwrap();
        let author = b
            .desired_author(
                &ti,
                "state/tasks/{system}/{service}",
                super::AuthorStartup::default(),
            )
            .await
            .unwrap();
        assert_eq!(author.major(), super::Major::V2);
        let svc = b.start().await.unwrap();
        let up = uptime.put(b"42".to_vec()).await.unwrap().stamp;
        let v1 = author
            .put(
                "lab/worker",
                br#"{"revision": 1, "mode": "run", "rate_hz": 10}"#.to_vec(),
            )
            .await
            .unwrap()
            .version()
            .unwrap();
        // One GET of one concrete key, with a held stamp or none: how many
        // replies it drew.
        let get = |key: String, held: Option<wire::Stamp>| {
            let s = s.clone();
            async move {
                let mut get = s
                    .get(key)
                    .target(QueryTarget::All)
                    .consolidation(ConsolidationMode::None)
                    .timeout(Duration::from_secs(2));
                if let Some(stamp) = held {
                    get = get
                        .payload(wire::HeldStamp::new(stamp).write())
                        .encoding(zenoh::bytes::Encoding::ZENOH_BYTES);
                }
                let rx = get.await.unwrap();
                let mut replies = 0;
                while let Ok(r) = rx.recv_async().await {
                    assert!(r.result().is_ok(), "an error reply");
                    replies += 1;
                }
                replies
            }
        };
        let slice = author.key("lab/worker").unwrap();
        let other = "zk2/lab/author/sysinfo.v1/state/uptime".to_owned();
        let store = svc.store();
        // The other state: answered whole, and by its held stamp alone, and
        // by a snapshot under another stamp; nothing is kept.
        for held in [None, Some(up), Some(v1)] {
            assert_eq!(get(other.clone(), held).await, 1, "{held:?}");
        }
        assert_eq!(store.reads_kept(), 0, "no GET of sysinfo.v1 is kept");
        let other_key = zenoh::key_expr::OwnedKeyExpr::try_from(other).unwrap();
        assert!(store.reads_of(&other_key).is_empty());
        assert!(author.reads("lab/worker").unwrap().is_empty());
        // The slice: each GET kept, with what it carried and how it was
        // answered.
        for held in [None, Some(v1), Some(up)] {
            assert_eq!(get(slice.clone(), held).await, 1, "{held:?}");
        }
        assert_eq!(
            author.reads("lab/worker").unwrap(),
            [
                Asked {
                    held: None,
                    unchanged: false
                },
                Asked {
                    held: Some(v1),
                    unchanged: true
                },
                Asked {
                    held: Some(up),
                    unchanged: false
                },
            ]
        );
        assert_eq!(store.reads_kept(), 3);
        // A wildcard over the slice's resource is no read of one key.
        assert_eq!(
            get("zk2/lab/author/tasking.v2/state/tasks/**".to_owned(), None).await,
            1
        );
        assert_eq!(store.reads_kept(), 3);
        svc.close().await.unwrap();
    }

    /// The file record reads back what it renders, a binary document as
    /// hex.
    #[test]
    fn the_file_record_round_trips() {
        let ts: zenoh::time::Timestamp = "7386690599959157260/33".parse().unwrap();
        let mut kept = Kept::default();
        kept.documents.insert(
            "lab/worker".into(),
            KeptDocument {
                payload: br#"{"mode":"run","rate_hz":10,"revision":1}"#.to_vec(),
                stamp: Some(ts),
            },
        );
        kept.documents.insert(
            "lab/raw".into(),
            KeptDocument {
                payload: vec![0, 159, 255],
                stamp: None,
            },
        );
        kept.deletes.insert(
            "lab/spare".into(),
            KeptDelete {
                at_ms: 1_760_000_000_000,
                stamp: Some(ts),
            },
        );
        let text = FileRecord::render(&kept).to_string();
        let back = FileRecord::parse(&text).unwrap();
        assert_eq!(back.deletes, kept.deletes);
        assert_eq!(back.documents["lab/raw"], kept.documents["lab/raw"]);
        let a: serde_json::Value =
            serde_json::from_slice(&back.documents["lab/worker"].payload).unwrap();
        let b: serde_json::Value =
            serde_json::from_slice(&kept.documents["lab/worker"].payload).unwrap();
        assert_eq!(a, b);
        assert_eq!(back.latest(), Some(ts));
        assert!(FileRecord::parse("[]").is_ok_and(|k| k == Kept::default()));
        assert!(FileRecord::parse(r#"{"documents": {"x": {}}}"#).is_err());
    }

    /// `desired.v2` §2.11: an author's record holds no stamp, and says its
    /// major, which it reads back.
    #[test]
    fn a_desired_v2_record_holds_no_stamp() {
        let ts: zenoh::time::Timestamp = "7386690599959157260/33".parse().unwrap();
        let mut kept = Kept {
            major: Some(super::Major::V2),
            ..Kept::default()
        };
        kept.documents.insert(
            "lab/worker".into(),
            KeptDocument {
                payload: br#"{"mode":"run"}"#.to_vec(),
                // A stamp a v1 record carried over is not written either.
                stamp: Some(ts),
            },
        );
        kept.deletes.insert(
            "lab/spare".into(),
            KeptDelete {
                at_ms: 1_760_000_000_000,
                stamp: None,
            },
        );
        let file = FileRecord::render(&kept);
        assert_eq!(
            file,
            serde_json::json!({
                "profile": "desired.v2",
                "documents": {"lab/worker": {"document": {"mode": "run"}}},
                "deletes": {"lab/spare": {"at_ms": 1_760_000_000_000_u64}},
            })
        );
        let back = FileRecord::parse(&file.to_string()).unwrap();
        assert_eq!(back.major, Some(super::Major::V2));
        assert_eq!(back.latest(), None);
        assert_eq!(back.deletes, kept.deletes);
    }

    #[test]
    fn a_pointer_escapes_its_names() {
        assert_eq!(escape("a/b~c"), "a~1b~0c");
    }

    /// Across a face (core §8.1): the target's GETs and its archive's read
    /// wait at least the face's reply timeout, and a longer one stands.
    #[test]
    fn a_target_across_a_face_waits_its_reply_timeout() {
        use std::time::Duration;
        use zenkey_model::link::{Binding, FarSide};
        let far = FarSide {
            face: "rf".into(),
            reply_timeout_ms: 15_000,
            arq: false,
            transport: Binding::Zenoh1_10_1.defaults(),
        };
        let archive: zenkey_model::grammar::Addr = "ground/archive".parse().unwrap();
        let t = super::TargetStartup::default()
            .with_archive(archive.clone(), Duration::from_secs(60))
            .across(&far);
        assert_eq!(
            t.timeout,
            Duration::from_secs(15),
            "1 s by default, 15 s across RF"
        );
        assert_eq!(
            t.archive,
            Some((archive, Duration::from_secs(60))),
            "a longer one stands"
        );
    }

    /// An application shares the handles across tasks.
    #[test]
    fn the_handles_are_shared_across_tasks() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        shared::<super::Author>();
        shared::<super::Target>();
    }
}
