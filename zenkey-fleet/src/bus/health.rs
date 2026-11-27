//! `health.v1` (#721, PF) and `health.v2` (#986's Q2), the reads: how well
//! each service says it serves (`spec/profiles/health/v1.md`, `v2.md`),
//! read as §2.11's reader reads it, through a session in the deployment's
//! namespace. Each service is read by the major its descriptor lists, and
//! `health.v2` where it lists both (v2 §2.11, §2.12); across a face, by the
//! major the deployment's word names ([`AcrossFace::major`]).
//!
//! What is read, in order:
//! 1. with a window, first: a subscription to every status and to the
//!    faults, through the runtime's consumer on the standard contract (R4:
//!    any revision of the major reads the interface). It confirms a status
//!    on this host's monotonic clock, by its puts and, since `health.v1`
//!    0.8, by the owner's heartbeats that vouch for it (`freshness.v2`
//!    §2.5; #978's F4). It measures each stamping clock on live puts (§2.6,
//!    ground 2), and records, in order, the status's puts and deletes, the
//!    faults, and each heartbeat read in step with whether its time is
//!    known: §5's clock question reads that time, since `health.v1` 0.9
//!    (#978's F6), never a put, which the guard no longer holds;
//! 2. presence and every instance's descriptor (core §8.1, §3.3): a service
//!    implements `health.v1` when its descriptor lists it, token or not
//!    (§2.7). Across a constrained face neither crosses, and the
//!    deployment's word stands for them (§2.8, R7). The window opens after
//!    this read, which was answered after the subscriptions were declared
//!    (#819 TE, F-154); across a face, after the first reading's GET, the
//!    round trip there (#819 TG, F-159);
//! 3. two readings, a grace apart (or the window apart): each two GETs, of
//!    the owners' `health.v2/state/**` and `health.v1/state/**` (v2
//!    §2.12), target `All`, consolidation `None` (core S4), each of which
//!    answers a status and its checks together (§2.4, core S2) — so a
//!    status is never paired with a check from another instant (§2.2). A
//!    `health.v2` owner's two snapshots are also read as one reading
//!    ([`zenkey_model::health::v2::Reading`]), which is one instant or not
//!    (v2 §2.2);
//! 4. the tables of each `health.v2` owner's words (v2 §2.10): from the
//!    revisions its descriptor names, held offline ([`HealthSpec::contracts`])
//!    or retrieved by fingerprint (core §8.4), never another revision's;
//!    across a face, from the revisions held, the deployment's word;
//! 5. for one service named and absent, an archive's form of its status
//!    (core §4.4), of either major: last-known, never current (S6).
//!
//! **The window under `health.v2`** hands every read heartbeat to §5's
//! clock question (v2 §2.5): a heartbeat covering `health.v2` or a group
//! of an owner heartbeat of the `host` class, whatever its section says,
//! answers by its time alone (ahead, an instant, unknown). A fault and a
//! put of the status answer nothing.
//!
//! Nothing is judged here: [`crate::judge::health`] decides every verdict
//! from the [`HealthObservation`] this returns. The payloads are decoded
//! here, into the levels and words a verdict needs, with the runtime's own
//! types (`zenkey::health::v1`) or the model's packed records
//! (`zenkey_model::health::v2`, under CH's tag and the owner's epoch); a
//! payload that does not decode, and a record core §7.6 refuses, is
//! [`Read::Undecodable`], never a level (§2.6, v2 §2.6).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zenkey::health::v1;
use zenkey::prost::Message as _;
use zenkey_model::freshness::{ClockMeasure, Observation};
use zenkey_model::grammar::{Addr, GRAMMAR, IfaceId, KindToken, ZkKey};
use zenkey_model::health::v2::{self as hv2, ClockHeard, Packed as _};
use zenkey_model::health::{self as hm, Heard, Major, Read};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::{Sample, SampleKind};

use crate::bus::presence::Scope;
use crate::model::catalog::{Catalog, Observed};

/// `archive.v1`'s interface id: its providers are the archives an absent
/// owner's last-known status is asked of.
const ARCHIVE: &str = "archive.v1";

/// How many deliveries of one service a window keeps in order; past it the
/// oldest go. Only the order of the last heartbeat of each kind and the
/// last `clock_ahead` fault is read, and repeats collapse, so this is
/// generous.
pub const SEQUENCE_CAP: usize = 64;

/// Which services a reading covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthTarget {
    /// Every service presence shows, and every one that answers.
    All,
    /// One service, `<system>/<service>`.
    One(Addr),
}

impl HealthTarget {
    /// The provider a tool's consumer binds (R1).
    fn provider(&self) -> String {
        match self {
            HealthTarget::All => "*/*".to_owned(),
            HealthTarget::One(a) => a.to_string(),
        }
    }

    fn chunks(&self) -> (String, String) {
        match self {
            HealthTarget::All => ("*".to_owned(), "*".to_owned()),
            HealthTarget::One(a) => (a.system.to_string(), a.service.to_string()),
        }
    }

    /// The selector a reading's `health.v1` GET goes out on: the owners'
    /// `health.v1/state/**`, base-relative (§2.4).
    pub fn state_selector(&self) -> String {
        self.state_selector_of(Major::V1)
    }

    /// The selector a reading's GET of `major` goes out on: the owners'
    /// `<major>/state/**`, base-relative (§2.4, v2 §2.12).
    pub fn state_selector_of(&self, major: Major) -> String {
        let (sys, svc) = self.chunks();
        [GRAMMAR, &sys, &svc, major.iface(), "state", "**"].join("/")
    }

    /// Both GETs' selectors, `health.v2`'s first (v2 §2.12).
    pub fn state_selectors(&self) -> [String; 2] {
        [
            self.state_selector_of(Major::V2),
            self.state_selector_of(Major::V1),
        ]
    }

    fn scope(&self) -> Scope {
        match self {
            HealthTarget::All => Scope::all(),
            HealthTarget::One(a) => Scope::service(a),
        }
    }
}

/// A service across a constrained face (§2.8, core §8.5): presence and
/// descriptors do not cross, and the deployment's word says which major it
/// implements (R7, v2 §2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcrossFace {
    /// Whether the face lets `state/status` cross.
    pub status_crosses: bool,
    /// The major the deployment's word names: `health.v1` or `health.v2`.
    pub major: Major,
}

/// What a reading asks.
#[derive(Debug, Clone)]
pub struct HealthSpec {
    /// Each read's timeout: presence, a descriptor, a GET, an archive.
    pub timeout: Duration,
    /// The window: how long a subscription to every status and to the
    /// faults listens, from its opening, before the first reading: the
    /// subscriptions declared and presence read after them. `None`: no
    /// subscription, and §5's clock question is not asked.
    pub window: Option<Duration>,
    /// How far apart the two readings are when no window separates them.
    pub grace: Duration,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1).
    pub clocks_synced: bool,
    /// The service sits across a constrained face (§2.8).
    pub face: Option<AcrossFace>,
    /// The deployment catalogs given (`--catalog`, core §8.6): a GET's
    /// entry is dated under the epoch of the catalog its owner's descriptor
    /// names, read through these (`bus::query::EntryEpochs`; #1087). With
    /// none, an owner naming a catalog leaves its confirmations undated.
    pub catalogs: crate::model::deployment::Catalogs,
    /// The revisions held offline (`--contracts`): a `health.v2` owner's
    /// words are read by the tables of the revisions its descriptor names,
    /// these first, one not held retrieved by fingerprint (core §8.4); across
    /// a face, these alone, the deployment's word (v2 §2.10).
    pub contracts: crate::model::catalog::ContractSet,
}

/// A stamp's time, and the id of the clock that issued it (core §4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamped {
    pub time: SystemTime,
    pub clock: String,
}

impl Stamped {
    fn of(sample: &Sample) -> Option<Stamped> {
        sample.timestamp().map(|t| Stamped {
            time: t.get_time().to_system_time(),
            clock: t.get_id().to_string(),
        })
    }

    /// As the report spells a stamp, as [`crate::bus::consume::stamp`]
    /// does: RFC 3339 with nanoseconds, and the clock's id.
    pub fn report(&self) -> crate::report::Stamp {
        let since = self
            .time
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        crate::report::Stamp {
            time: format!("{:#}", zenoh::time::NTP64::from(since)),
            clock: self.clock.clone(),
        }
    }
}

/// A status as decoded (§2.6): its level as read, and the words beside it.
/// Under `health.v1` its reason is text; under `health.v2` it is a word,
/// read by the instance's tables where the row is built (v2 §2.10), and
/// its `since` whole seconds, absent where the record carries none (v2
/// §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusValue {
    pub level: Read,
    /// `health.v1`'s text; empty under `health.v2`.
    pub reason: String,
    /// `health.v1`'s nanoseconds; 0 under `health.v2`.
    pub since_ns: u64,
    /// `health.v2`'s reason, a word; `None` under `health.v1`, and for a
    /// record refused.
    pub word: Option<hv2::Word>,
    /// `health.v2`'s `since`, seconds since the Unix epoch; `None` where
    /// the record carries none, and under `health.v1`.
    pub since_s: Option<i64>,
}

impl StatusValue {
    /// A status payload: `health.v1.Status`, or [`Read::Undecodable`].
    pub fn decode(bytes: &[u8]) -> StatusValue {
        match v1::Status::decode(bytes) {
            Ok(s) => StatusValue {
                level: s.read(),
                reason: s.reason,
                since_ns: s.since_ns,
                word: None,
                since_s: None,
            },
            Err(_) => StatusValue::undecodable(),
        }
    }

    /// A `health.v2` status record under CH's `tag`, its `since` counted
    /// from the deployment epoch in force for its owner (core §7.6, §8.6):
    /// a record the layout refuses, and one whose owner's epoch cannot be
    /// had, is [`Read::Undecodable`], never a level (v2 §2.6).
    pub fn decode_v2(tag: u8, record: &[u8], epoch: Option<i64>) -> StatusValue {
        match epoch.map(|e| hv2::Status::read(tag, record, e)) {
            Some(Ok(s)) => StatusValue {
                level: hv2::level_read(Ok(s.level)),
                reason: String::new(),
                since_ns: 0,
                word: Some(s.reason),
                since_s: s.since,
            },
            _ => StatusValue::undecodable(),
        }
    }

    /// A payload that does not decode: [`Read::Undecodable`].
    pub fn undecodable() -> StatusValue {
        StatusValue {
            level: Read::Undecodable,
            reason: String::new(),
            since_ns: 0,
            word: None,
            since_s: None,
        }
    }
}

/// A check as decoded (§2.6), its detail text under `health.v1` and a word
/// under `health.v2`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckValue {
    pub level: Read,
    pub detail: String,
    /// `health.v2`'s detail, a word; `None` under `health.v1`.
    pub word: Option<hv2::Word>,
    /// `health.v2`'s `at`, seconds since the Unix epoch, where it carries one.
    pub at_s: Option<i64>,
}

impl CheckValue {
    /// A check payload: `health.v1.Check`, or [`Read::Undecodable`].
    pub fn decode(bytes: &[u8]) -> CheckValue {
        match v1::Check::decode(bytes) {
            Ok(c) => CheckValue {
                level: c.read(),
                detail: c.detail,
                word: None,
                at_s: None,
            },
            Err(_) => CheckValue::undecodable(),
        }
    }

    /// A `health.v2` check record, as [`StatusValue::decode_v2`] reads a
    /// status.
    pub fn decode_v2(tag: u8, record: &[u8], epoch: Option<i64>) -> CheckValue {
        match epoch.map(|e| hv2::Check::read(tag, record, e)) {
            Some(Ok(c)) => CheckValue {
                level: hv2::level_read(Ok(c.level)),
                detail: String::new(),
                word: Some(c.detail),
                at_s: c.at,
            },
            _ => CheckValue::undecodable(),
        }
    }

    /// A payload that does not decode: [`Read::Undecodable`].
    pub fn undecodable() -> CheckValue {
        CheckValue {
            level: Read::Undecodable,
            detail: String::new(),
            word: None,
            at_s: None,
        }
    }
}

/// A fault as decoded (§2.3, §2.10): an occurrence. Under `health.v2` its
/// code is a word and its detail a number (v2 §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaultValue {
    /// `health.v1`'s code; empty under `health.v2`.
    pub code: String,
    pub level: Read,
    pub detail: String,
    /// `health.v2`'s code, a word; `None` under `health.v1`.
    pub word: Option<hv2::Word>,
    /// `health.v2`'s `argument`.
    pub argument: Option<hv2::Argument>,
    /// `health.v2`'s `at`, seconds since the Unix epoch, where it carries one.
    pub at_s: Option<i64>,
}

impl FaultValue {
    /// A `health.v1` fault payload, or one that does not decode.
    pub fn decode(bytes: &[u8]) -> FaultValue {
        match v1::Fault::decode(bytes) {
            Ok(f) => FaultValue {
                level: f.read(),
                code: f.code,
                detail: f.detail,
                word: None,
                argument: None,
                at_s: None,
            },
            Err(_) => FaultValue::undecodable(),
        }
    }

    /// A `health.v2` fault record, as [`StatusValue::decode_v2`] reads a
    /// status.
    pub fn decode_v2(tag: u8, record: &[u8], epoch: Option<i64>) -> FaultValue {
        match epoch.map(|e| hv2::Fault::read(tag, record, e)) {
            Some(Ok(f)) => FaultValue {
                code: String::new(),
                level: hv2::level_read(Ok(f.level)),
                detail: String::new(),
                word: Some(f.code),
                argument: Some(f.argument),
                at_s: f.at,
            },
            _ => FaultValue::undecodable(),
        }
    }

    /// A payload that does not decode.
    pub fn undecodable() -> FaultValue {
        FaultValue {
            code: String::new(),
            level: Read::Undecodable,
            detail: String::new(),
            word: None,
            argument: None,
            at_s: None,
        }
    }

    /// Whether it is a `clock_ahead` fault, of either major (§2.5, v2
    /// §2.10): the profile's code 1 under `health.v2`.
    pub fn is_clock_ahead(&self) -> bool {
        match self.word {
            Some(w) => {
                hv2::read_word(hv2::Kind::Code, w, None) == hv2::Named::Profile(hm::CLOCK_AHEAD)
            }
            None => self.code == hm::CLOCK_AHEAD,
        }
    }
}

/// One state member as a GET answered it: a value, or `None` for a
/// `reply_del` (core S2, S3), with its stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replied<T> {
    pub value: Option<T>,
    pub stamp: Option<Stamped>,
}

/// What one GET of `<major>/state/**` answered for one service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceGot {
    /// The status; `None` when no reply named it.
    pub status: Option<Replied<StatusValue>>,
    /// Each check answered, by the `{check}` chunk of its key.
    pub checks: BTreeMap<String, Replied<CheckValue>>,
    /// Under `health.v2`, the owner's replies read as one reading of its two
    /// snapshots (v2 §2.2, §2.4): whether it is one instant, and whether it
    /// shows a status better than a check. `None` under `health.v1`.
    pub reading_v2: Option<hv2::Reading>,
}

/// One reading's GETs, every service's answer by address, under each major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthGet {
    /// The `health.v1` GET's selector, base-relative.
    pub selector: String,
    /// This host's clock as the GETs ended: the instant their replies'
    /// stamps are aged at (`freshness.v1` §2.6).
    pub read_at: SystemTime,
    /// Whether both GETs ended without an error reply (a timeout among
    /// them).
    pub complete: bool,
    /// What the `health.v1` GET answered.
    pub services: BTreeMap<Addr, ServiceGot>,
    /// What the `health.v2` GET answered (v2 §2.12).
    pub v2: BTreeMap<Addr, ServiceGot>,
}

impl HealthGet {
    /// What the GET of `major` answered.
    pub fn services_of(&self, major: Major) -> &BTreeMap<Addr, ServiceGot> {
        match major {
            Major::V1 => &self.services,
            Major::V2 => &self.v2,
        }
    }

    /// Every address either GET answered for.
    pub fn addresses(&self) -> impl Iterator<Item = &Addr> {
        self.services.keys().chain(self.v2.keys())
    }

    /// The major a service whose descriptor is not in hand is read by: the
    /// one whose GET answered for it, `health.v2` first, else `health.v1`.
    pub fn answered_major(&self, addr: &Addr) -> Option<Major> {
        if self.v2.contains_key(addr) {
            Some(Major::V2)
        } else if self.services.contains_key(addr) {
            Some(Major::V1)
        } else {
            None
        }
    }
}

/// What the window's subscriptions heard of one service, under one major.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceHeard {
    /// The last status put heard, with its stamp.
    pub last: Option<Replied<StatusValue>>,
    /// Under `health.v1`: status puts and deletes, faults and heartbeats
    /// read in step, in arrival order, consecutive repeats collapsed, at
    /// most [`SEQUENCE_CAP`]: what §5's clock question reads.
    pub sequence: Vec<Heard>,
    /// Under `health.v2`: every read heartbeat by its time, status puts
    /// and deletes and faults, in arrival order, repeats collapsed, at most
    /// [`SEQUENCE_CAP`]: what v2 §2.5's clock question reads, the last read
    /// heartbeat answering.
    pub clock: Vec<ClockHeard>,
    /// Faults heard.
    pub faults: u64,
    pub last_fault: Option<FaultValue>,
    /// Under `health.v2`, the last status record and the last fault record
    /// as they arrived (CH's tag, the record), read under the owner's epoch
    /// once presence says which ([`Window::close`]).
    pub(crate) raw_status: Option<(u8, Vec<u8>, Option<Stamped>)>,
    pub(crate) raw_fault: Option<(u8, Vec<u8>)>,
}

impl ServiceHeard {
    fn push(&mut self, h: Heard) {
        if self.sequence.last() == Some(&h) {
            return;
        }
        if self.sequence.len() >= SEQUENCE_CAP {
            self.sequence.remove(0);
        }
        self.sequence.push(h);
    }

    fn push_clock(&mut self, h: ClockHeard) {
        if self.clock.last() == Some(&h) {
            return;
        }
        if self.clock.len() >= SEQUENCE_CAP {
            self.clock.remove(0);
        }
        self.clock.push(h);
    }
}

/// What the window heard, read at its end.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthWindow {
    /// The key expressions subscribed, base-relative.
    pub selectors: Vec<String>,
    /// How long the subscriptions listened, to the second reading.
    pub listened: Duration,
    /// By address, every service heard under `health.v1`.
    pub heard: BTreeMap<Addr, ServiceHeard>,
    /// The `health.v1` status member's observation at the second reading
    /// (§2.5), for every service the run read, heard or not.
    pub status: BTreeMap<Addr, Observation>,
    /// By address, every service heard under `health.v2`.
    pub heard_v2: BTreeMap<Addr, ServiceHeard>,
    /// The `health.v2` status member's observation at the second reading,
    /// for every service the run read.
    pub status_v2: BTreeMap<Addr, Observation>,
    /// Per stamping clock, what the window measured of it (§2.6, ground 2).
    pub clocks: BTreeMap<String, ClockMeasure>,
}

impl HealthWindow {
    /// What the window heard under `major`.
    pub fn heard_of(&self, major: Major) -> &BTreeMap<Addr, ServiceHeard> {
        match major {
            Major::V1 => &self.heard,
            Major::V2 => &self.heard_v2,
        }
    }

    /// The status observations under `major`.
    pub fn status_of(&self, major: Major) -> &BTreeMap<Addr, Observation> {
        match major {
            Major::V1 => &self.status,
            Major::V2 => &self.status_v2,
        }
    }
}

/// An archive's status for an absent owner (core §4.4, S6), of the major
/// whose status key it recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedStatus {
    pub archive: Addr,
    pub major: Major,
    pub value: Option<StatusValue>,
    pub stamp: Option<Stamped>,
    pub confirmed: bool,
}

/// Everything one health reading read, as values.
#[derive(Debug, Clone)]
pub struct HealthObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub target: HealthTarget,
    pub spec: HealthSpec,
    /// Presence and descriptors; `None` across a face, where neither
    /// crosses (§2.8).
    pub presence: Option<Result<Observed, String>>,
    /// The two readings' GETs, the first first.
    pub first: Result<HealthGet, String>,
    pub second: Result<HealthGet, String>,
    /// The window, when one ran.
    pub window: Option<Result<HealthWindow, String>>,
    /// For one service named and absent: the first archive presence shows
    /// that holds its status, or none; `None` when not asked.
    pub archived: Option<Result<Option<ArchivedStatus>, String>>,
    /// Each `health.v2` owner's tables (v2 §2.10), read from the revisions
    /// its descriptor names that this reader holds or retrieved; across a
    /// face, from the revisions held. An owner with none here has none, and
    /// its words from 32 to 62 are shown as numbers.
    pub tables: BTreeMap<Addr, hv2::InstanceTables>,
}

impl HealthObservation {
    /// The selectors put to the bus, base-relative: what was read, and no
    /// wider (tooling guide O5).
    pub fn asked(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(Ok(p)) = &self.presence {
            out.push(p.selector.clone());
        }
        out.extend(self.target.state_selectors());
        if let Some(Ok(w)) = &self.window {
            out.extend(w.selectors.iter().cloned());
        }
        let mut seen = BTreeSet::new();
        out.retain(|s| seen.insert(s.clone()));
        out
    }
}

/// The member key of `addr`'s status: `zk2/<addr>/health.v1/state/status`.
pub fn status_key(addr: &Addr) -> String {
    status_key_of(addr, Major::V1)
}

/// The member key of `addr`'s status under `major`.
pub fn status_key_of(addr: &Addr, major: Major) -> String {
    [
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        major.iface(),
        "state",
        hm::STATUS,
    ]
    .join("/")
}

/// `major`'s interface id.
fn iface(major: Major) -> IfaceId {
    IfaceId::from_str(major.iface()).expect("a health major is an interface id")
}

/// A health reading's key, parsed: its owner and its resource chunks,
/// when it is a state key of `major` of the kind `kind`.
fn health_key_of(key: &str, kind: KindToken, major: Major) -> Option<(Addr, Vec<String>)> {
    match zenkey_model::grammar::parse(key) {
        Ok(ZkKey::Data {
            addr,
            iface: got,
            kind: k,
            resource,
        }) if got == iface(major) && k == kind => Some((addr, resource)),
        _ => None,
    }
}

/// One health reading: [`observe`], then [`crate::judge::health::judge`].
pub async fn run_health(
    session: &Session,
    namespace: &str,
    target: HealthTarget,
    spec: HealthSpec,
    listening: impl FnOnce(HealthOpening) + Send,
) -> crate::report::HealthReport {
    let obs = observe(session, namespace, target, spec, None, listening).await;
    crate::judge::health::judge(&obs)
}

/// What a health window opened on (#819 TG): the round trip made after the
/// subscription that reaches the owners, or why none proved it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthOpening {
    /// With no face, every instance the presence read found answered its
    /// descriptor GET, made after the subscription. Across a face, where no
    /// presence is read, an owner replied to the first reading's GET and its
    /// final reply came.
    Answered,
    /// Nothing proved the owners hold the subscription, with why: a status
    /// or a fault put at once may be missed.
    Unanswered(String),
}

/// [`HealthOpening`] from what was read before the window opened: with no
/// face, the presence read (`fresh` when made after the subscription, not
/// the caller's); across a face, the first reading's GET.
fn opening_of(
    face: bool,
    fresh: bool,
    presence: Option<&Result<Observed, String>>,
    first: Option<&Result<HealthGet, String>>,
) -> HealthOpening {
    use crate::model::catalog::DescriptorRead;
    let no = |why: String| HealthOpening::Unanswered(why);
    if face {
        return match first {
            Some(Ok(g)) if !g.complete => no(
                "across the face the first reading's GET ran to its timeout, or drew an error \
                 reply"
                    .to_owned(),
            ),
            Some(Ok(g)) if g.services.is_empty() => no(
                "across the face the first reading's GET drew no reply, so nothing shows it \
                 reached the owner"
                    .to_owned(),
            ),
            Some(Ok(_)) => HealthOpening::Answered,
            Some(Err(e)) => no(format!(
                "across the face the first reading's GET could not be made: {e}"
            )),
            None => no("across the face no GET was made".to_owned()),
        };
    }
    match presence {
        Some(Ok(_)) if !fresh => no("presence was read before the subscription".to_owned()),
        Some(Ok(o)) => {
            let instances = o.instances();
            if instances.is_empty() {
                return no(
                    "presence found no instance in scope to answer a GET after the subscription"
                        .to_owned(),
                );
            }
            let answered = instances
                .iter()
                .filter(|k| {
                    matches!(
                        o.descriptors.as_ref().and_then(|d| d.get(*k)),
                        Some(DescriptorRead::Served(_) | DescriptorRead::Invalid(_))
                    )
                })
                .count();
            if answered == instances.len() {
                HealthOpening::Answered
            } else {
                no(format!(
                    "{} of the {} instance(s) presence found answered no descriptor GET after the \
                     subscription",
                    instances.len() - answered,
                    instances.len()
                ))
            }
        }
        Some(Err(e)) => no(format!("the presence read could not be made: {e}")),
        None => no("no presence was read".to_owned()),
    }
}

/// The two readings, the window and the archive, as the module doc says.
/// `presence`, when given, is the read a caller already made (`check
/// conform`), and is not made again. `listening` runs once the window
/// opens, its subscriptions declared and presence read after them (zk2py
/// round 47's F-154, #819 TE) or, across a face, the first reading's GET
/// answered after them (#819 TG, F-159): the far side holds them then, and
/// a status or a fault put from then on is heard. The window counts from that moment; with
/// no window, or one whose subscriptions could not be declared, `listening`
/// never runs.
pub async fn observe(
    session: &Session,
    namespace: &str,
    target: HealthTarget,
    spec: HealthSpec,
    presence: Option<Observed>,
    listening: impl FnOnce(HealthOpening) + Send,
) -> HealthObservation {
    let t = spec.timeout;
    let fresh = presence.is_none();
    let window = match spec.window {
        Some(_) => {
            Some(Window::open(session, &target, t, spec.face.is_some(), &spec.catalogs).await)
        }
        None => None,
    };
    let presence = match (spec.face, presence) {
        (Some(_), _) => None,
        (None, Some(p)) => Some(Ok(p)),
        (None, None) => Some(
            crate::bus::presence::observe(session, &target.scope(), t)
                .await
                .map_err(|e| crate::one_line(&e)),
        ),
    };
    // An owner's entries are dated under the catalog its descriptor
    // names, read through the catalogs given, 0 where it names none; with
    // no descriptor in hand, as across a face, the catalogs' own (the
    // deployment's word), else 0. Undated where it names one none given is
    // or succeeds (#1087).
    let epochs = crate::bus::query::EntryEpochs::of_presence(
        &spec.catalogs,
        presence.as_ref().and_then(|p| p.as_ref().ok()),
    );
    let epochs = &epochs;
    // Across a face no presence is read, so the first reading's GET,
    // answered by the owner after the subscription, is the round trip the
    // window opens on (#819 TG, as zk2py round 49's F-159 found of
    // `telemetry`); without a face, the presence read above was.
    let across = if spec.face.is_some() {
        Some(get(session, &target, t, epochs).await)
    } else {
        None
    };
    if matches!(window, Some(Ok(_))) {
        listening(opening_of(
            spec.face.is_some(),
            fresh,
            presence.as_ref(),
            across.as_ref(),
        ));
    }
    let opened = Instant::now();
    let first = match across {
        Some(first) => first,
        None => get(session, &target, t, epochs).await,
    };
    let wait = match spec.window {
        Some(w) => (opened + w).saturating_duration_since(Instant::now()),
        None => spec.grace,
    };
    tokio::time::sleep(wait).await;
    let second = get(session, &target, t, epochs).await;
    let window = window.map(|w| {
        w.map(|w| {
            let mut addrs: BTreeSet<Addr> = BTreeSet::new();
            if let HealthTarget::One(a) = &target {
                addrs.insert(a.clone());
            }
            if let Some(Ok(p)) = &presence {
                addrs.extend(Catalog::new(p).addresses().cloned());
            }
            for g in [&first, &second].into_iter().flatten() {
                addrs.extend(g.addresses().cloned());
            }
            w.close(addrs, epochs)
        })
    });
    let archived = match (&target, &presence) {
        (HealthTarget::One(a), Some(Ok(p))) if p.complete && !Catalog::new(p).has_instance(a) => {
            Some(archived(session, a, t, epochs).await)
        }
        _ => None,
    };
    let tables = tables(
        session,
        &target,
        &spec,
        presence.as_ref().and_then(|p| p.as_ref().ok()),
    )
    .await;
    HealthObservation {
        namespace: namespace.to_owned(),
        target,
        spec,
        presence,
        first,
        second,
        window,
        archived,
        tables,
    }
}

/// Each `health.v2` owner's tables (v2 §2.10): with no face, for each
/// service whose descriptors list `health.v2`, the tables of the revisions
/// they name, read from those held offline or retrieved by fingerprint
/// (core §8.4), never another revision's; across a face, where no
/// descriptor crosses, the revisions held, the deployment's word. The
/// standard contract writes none, so its revision is not retrieved.
async fn tables(
    session: &Session,
    target: &HealthTarget,
    spec: &HealthSpec,
    presence: Option<&Observed>,
) -> BTreeMap<Addr, hv2::InstanceTables> {
    let mut out = BTreeMap::new();
    match (spec.face, presence) {
        (Some(face), _) => {
            if let (HealthTarget::One(a), Major::V2) = (target, face.major) {
                let held: Vec<hv2::Tables> = spec
                    .contracts
                    .iter()
                    .map(|r| hv2::Tables::of(r.contract()))
                    .collect();
                out.insert(a.clone(), hv2::Tables::of_instance(&held));
            }
        }
        (None, Some(o)) => {
            let catalog = Catalog::new(o);
            let mut named: BTreeMap<
                Addr,
                BTreeSet<(IfaceId, zenkey_model::canonical::Fingerprint)>,
            > = BTreeMap::new();
            for a in catalog.addresses() {
                if crate::model::health::major_of(o, a) != Some(Major::V2) {
                    continue;
                }
                let revisions = named.entry(a.clone()).or_default();
                for ((at, _), read) in o.descriptors.iter().flatten() {
                    let Some(d) = read.descriptor().filter(|_| at == a) else {
                        continue;
                    };
                    for e in &d.interfaces {
                        let (Ok(i), Ok(f)) = (
                            IfaceId::from_str(&e.iface),
                            zenkey_model::canonical::Fingerprint::parse(&e.contract),
                        ) else {
                            continue;
                        };
                        if e.iface == hv2::IFACE && f.hex().to_string() == hv2::FINGERPRINT {
                            continue;
                        }
                        revisions.insert((i, f));
                    }
                }
            }
            let store = crate::bus::contracts::BundleStore::new(spec.timeout);
            store.seed(&spec.contracts);
            let wanted: Vec<_> = named
                .values()
                .flatten()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let got = store.fetch_all(session, &wanted).await;
            let held: BTreeMap<_, hv2::Tables> = wanted
                .into_iter()
                .zip(got)
                .filter_map(|(w, g)| {
                    let g = g.ok()?;
                    let r = g.revision()?;
                    Some((w, hv2::Tables::of(r.contract())))
                })
                .collect();
            for (a, revisions) in named {
                let tables: Vec<&hv2::Tables> =
                    revisions.iter().filter_map(|w| held.get(w)).collect();
                out.insert(a, hv2::Tables::of_instance(tables));
            }
        }
        (None, None) => {}
    }
    out
}

/// One reading's GETs (core S4), `health.v2`'s and `health.v1`'s at once
/// (v2 §2.12), each as [`get_of`] reads it.
pub async fn get(
    session: &Session,
    target: &HealthTarget,
    timeout: Duration,
    epochs: &crate::bus::query::EntryEpochs,
) -> Result<HealthGet, String> {
    let [v2, v1] = target.state_selectors();
    let (v2, v1) = tokio::join!(
        get_of(session, &v2, timeout, epochs, Major::V2),
        get_of(session, &v1, timeout, epochs, Major::V1)
    );
    let (v2, v1) = (v2?, v1?);
    Ok(HealthGet {
        selector: v1.selector,
        read_at: SystemTime::now(),
        complete: v1.complete && v2.complete,
        services: v1.services,
        v2: v2.services,
    })
}

/// One reading's GET of `major` (core S4): target `All`, consolidation `None`, a
/// snapshot's page read as its entries, each entry's confirmation dated
/// under its owner's epoch (`epochs`), every reply merged per key by its
/// stamp, never the last to arrive, and grouped by the owner its key
/// names. A reply on a key that is not a state key of `major`, or on a
/// wildcard key, is not a member and is left out (R6). Under `health.v2` a
/// record is read under CH's tag and its owner's epoch, and each owner's
/// pages are read again as one reading of its two snapshots
/// ([`ServiceGot::reading_v2`], v2 §2.2). The returned GET holds its answer
/// in `services`, whichever the major.
pub async fn get_of(
    session: &Session,
    selector: &str,
    timeout: Duration,
    epochs: &crate::bus::query::EntryEpochs,
    major: Major,
) -> Result<HealthGet, String> {
    let replies = session
        .get(selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut out = HealthGet {
        selector: selector.to_owned(),
        read_at: SystemTime::now(),
        complete: true,
        services: BTreeMap::new(),
        v2: BTreeMap::new(),
    };
    // A snapshot's page is its entries, a live entry's confirmation its
    // stamp under its owner's epoch (core §4.2, `freshness.v2` §2.6;
    // #977's E3), every reply merged per key by its stamp (S4).
    let mut samples = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            out.complete = false;
            continue;
        };
        samples.push(sample.clone());
    }
    // A stray `reply_del` of the status or a check, where its owner's
    // revision under the deletion's run answers it by snapshot, is no
    // owner's answer: undecodable, `form` (core §4.2, 0.102). An owner on
    // revision 1.0 confirms its status by `freshness.v1`, so it answers per
    // key, and its `reply_del` is a deletion.
    // Under `health.v2`, each owner's pages as they came, read as one
    // reading of its two snapshots (v2 §2.2).
    if major == Major::V2 {
        let mut pages: BTreeMap<Addr, Vec<(String, Vec<u8>)>> = BTreeMap::new();
        for s in &samples {
            let key = s.key_expr().as_str();
            if s.kind() != SampleKind::Put {
                continue;
            }
            if let Some((addr, _)) = health_key_of(key, KindToken::State, major) {
                pages
                    .entry(addr)
                    .or_default()
                    .push((key.to_owned(), s.payload().to_bytes().into_owned()));
            }
        }
        for (addr, pages) in pages {
            let epoch = epochs
                .of(&status_key_of(&addr, major))
                .unwrap_or(zenkey_model::catalog::EPOCH_WITHOUT_CATALOG);
            let reading =
                hv2::Reading::of(pages.iter().map(|(k, p)| (k.as_str(), p.as_slice())), epoch);
            out.services.entry(addr).or_default().reading_v2 = Some(reading);
        }
    }
    let health = match major {
        Major::V1 => zenkey::health::implementation(),
        Major::V2 => zenkey::health::v2::implementation(),
    }
    .shared_contract();
    let held = [&*health];
    let mut revisions = crate::bus::query::OwnerRevisions::new(session, timeout, &held, None);
    let (entries, strays) =
        crate::bus::query::merged_without_strays(samples, &mut revisions, |s| {
            crate::bus::query::reply_entries_aged(s, epochs)
        })
        .await;
    let samples: Vec<Sample> = entries.into_iter().map(|e| e.sample).collect();
    for sample in &strays {
        let Some((addr, resource)) =
            health_key_of(sample.key_expr().as_str(), KindToken::State, major)
        else {
            continue;
        };
        let got = out.services.entry(addr).or_default();
        match resource.as_slice() {
            [s] if s == hm::STATUS => {
                got.status = Some(Replied {
                    value: Some(StatusValue::undecodable()),
                    stamp: None,
                });
            }
            [c, name] if c == "checks" => {
                got.checks.insert(
                    name.clone(),
                    Replied {
                        value: Some(CheckValue::undecodable()),
                        stamp: None,
                    },
                );
            }
            _ => {}
        }
    }
    for sample in &samples {
        let key = sample.key_expr().as_str();
        if key.contains('*') {
            continue;
        }
        let Some((addr, resource)) = health_key_of(key, KindToken::State, major) else {
            continue;
        };
        let stamp = Stamped::of(sample);
        let epoch = epochs.of(key);
        // The envelope first (core §7.5): a payload is its record, a refused
        // envelope undecodable. A `health.v2` record is read under CH's tag
        // (core §7.6); one with no envelope has none, and is refused.
        let record = (sample.kind() == SampleKind::Put).then(|| match major {
            Major::V1 => crate::model::inband::record_of(sample)
                .ok()
                .map(|b| (0, b.into_owned())),
            Major::V2 => packed_record(sample),
        });
        let got = out.services.entry(addr).or_default();
        match resource.as_slice() {
            [s] if s == hm::STATUS => {
                got.status = Some(Replied {
                    value: record.map(|r| match (major, r) {
                        (_, None) => StatusValue::undecodable(),
                        (Major::V1, Some((_, b))) => StatusValue::decode(&b),
                        (Major::V2, Some((tag, b))) => StatusValue::decode_v2(tag, &b, epoch),
                    }),
                    stamp,
                });
            }
            [c, name] if c == "checks" => {
                got.checks.insert(
                    name.clone(),
                    Replied {
                        value: record.map(|r| match (major, r) {
                            (_, None) => CheckValue::undecodable(),
                            (Major::V1, Some((_, b))) => CheckValue::decode(&b),
                            (Major::V2, Some((tag, b))) => CheckValue::decode_v2(tag, &b, epoch),
                        }),
                        stamp,
                    },
                );
            }
            _ => {}
        }
    }
    out.read_at = SystemTime::now();
    Ok(out)
}

/// A put's packed record and CH's tag (core §7.5, §7.6): `None` for a put
/// with no envelope, a refused one, and any other form.
fn packed_record(sample: &Sample) -> Option<(u8, Vec<u8>)> {
    let bytes = sample.payload().to_bytes();
    let encoding = sample.encoding().to_string();
    match crate::model::inband::payload(sample.key_expr().as_str(), Some(&encoding), &bytes) {
        crate::model::inband::Payload::Record { record, tag, .. } => {
            Some((tag.get(), record.to_vec()))
        }
        _ => None,
    }
}

/// The window's subscriptions, while they listen: each major's status and
/// faults.
struct Window {
    status: zenkey::consumer::Subscription,
    faults: zenkey::consumer::Subscription,
    status_v2: zenkey::consumer::Subscription,
    faults_v2: zenkey::consumer::Subscription,
    heard: Arc<Mutex<BTreeMap<Addr, ServiceHeard>>>,
    heard_v2: Arc<Mutex<BTreeMap<Addr, ServiceHeard>>>,
    selectors: Vec<String>,
}

impl Window {
    /// Subscribes to the target's status and faults under each major
    /// through the runtime's consumer, at once and without presence (R1),
    /// then seeds the status's views by S4's GET, as a late joiner does
    /// (`freshness.v2` §2.5; #978's F4): a status put before the window is
    /// in the view, so the owner's next heartbeat confirms it. A GET that
    /// fails seeds nothing, and the subscription re-reads after two
    /// heartbeats out of step.
    ///
    /// The subscription hears the owner's heartbeat and its owner
    /// heartbeats (`freshness.v2` §2.5, "Both forms"), a group read through
    /// `catalogs`, the ones the tool was given (core §8.6): with none, no
    /// group is read, and an owner that sends owner heartbeats alone has
    /// its status unconfirmed. The catalog its owner names is read from
    /// its descriptor within `timeout`, as a re-read is (#1146); `across`
    /// a face none is read, and the last catalog given is the deployment's
    /// word.
    async fn open(
        session: &Session,
        target: &HealthTarget,
        timeout: Duration,
        across: bool,
        catalogs: &crate::model::deployment::Catalogs,
    ) -> Result<Window, String> {
        let provider = target.provider();
        let consumer_of = |imp: zenkey::Implementation| {
            zenkey::consumer::Consumer::for_tool(
                session,
                imp.shared_contract(),
                &[provider.as_str()],
                &BTreeMap::new(),
            )
            .map(|c| crate::bus::consume::tool_consumer(c, timeout, across, catalogs))
            .map_err(|e| e.to_string())
        };
        let consumer = consumer_of(zenkey::health::implementation())?;
        let consumer_v2 = consumer_of(zenkey::health::v2::implementation())?;
        let mut selectors = Vec::new();
        for c in [&consumer_v2, &consumer] {
            for r in [zenkey::health::STATUS, zenkey::health::FAULTS] {
                selectors.extend(
                    c.selectors(r)
                        .map_err(|e| e.to_string())?
                        .iter()
                        .map(|k| k.as_str().to_owned()),
                );
            }
        }
        let heard: Arc<Mutex<BTreeMap<Addr, ServiceHeard>>> = Arc::default();
        let h = Arc::clone(&heard);
        let confirmed = Arc::clone(&heard);
        let status = consumer
            .subscribe_confirmed(
                zenkey::health::STATUS,
                move |d: zenkey::consumer::Delivery| {
                    let mut all = h.lock().expect("not poisoned");
                    let s = all.entry(d.provider).or_default();
                    if d.sample.kind() == SampleKind::Delete {
                        s.push(Heard::StatusDeleted);
                        return;
                    }
                    s.push(Heard::StatusPut);
                    s.last = Some(Replied {
                        value: Some(StatusValue::decode(&d.sample.payload().to_bytes())),
                        stamp: Stamped::of(&d.sample),
                    });
                },
                // A heartbeat read in step, its bit set or clear: §5's clock
                // question reads its time, the ahead time while the owner's
                // guard holds (`freshness.v2` §2.10, core §7.5), so one whose
                // time is an instant after a `clock_ahead` fault says the
                // guard released. Under `health.v1` a time that is no
                // instant, ahead or unknown, answers nothing alone (v1 §2.5):
                // both are `HeartbeatHeld`, as the unknown time was.
                move |c: zenkey::consumer::Confirmation| {
                    let time = c
                        .owner_time
                        .unwrap_or(zenkey::model::heartbeat::Time::Unknown);
                    let mut all = confirmed.lock().expect("not poisoned");
                    all.entry(c.provider)
                        .or_default()
                        .push(Heard::heartbeat(time));
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let h = Arc::clone(&heard);
        let faults = consumer
            .subscribe(
                zenkey::health::FAULTS,
                move |d: zenkey::consumer::Delivery| {
                    let fault = FaultValue::decode(d.sample.payload().to_bytes().as_ref());
                    let mut all = h.lock().expect("not poisoned");
                    let s = all.entry(d.provider).or_default();
                    s.faults += 1;
                    s.push(if fault.code == hm::CLOCK_AHEAD {
                        Heard::ClockAhead
                    } else {
                        Heard::Fault
                    });
                    s.last_fault = Some(fault);
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        // `health.v2` (v2 §2.5): every read heartbeat answers the clock
        // question by its time, whatever its section did (confirmed,
        // diverged or absent), a heartbeat covering `health.v2` and a group
        // of an owner heartbeat of the `host` class alike. A heartbeat of
        // another revision, refused or older is no read one, and the
        // subscription hands it to no one. A record is kept as it came, and
        // read under its owner's epoch once presence says which.
        let heard_v2: Arc<Mutex<BTreeMap<Addr, ServiceHeard>>> = Arc::default();
        let h = Arc::clone(&heard_v2);
        let read = Arc::clone(&heard_v2);
        let status_v2 = consumer_v2
            .subscribe_heartbeats(
                zenkey::health::STATUS,
                move |d: zenkey::consumer::Delivery| {
                    let mut all = h.lock().expect("not poisoned");
                    let s = all.entry(d.provider).or_default();
                    if d.sample.kind() == SampleKind::Delete {
                        s.push_clock(ClockHeard::StatusDeleted);
                        return;
                    }
                    s.push_clock(ClockHeard::StatusPut);
                    s.raw_status = Some((
                        d.tag.map_or(0, |t| t.get()),
                        d.sample.payload().to_bytes().into_owned(),
                        Stamped::of(&d.sample),
                    ));
                },
                move |r: zenkey::consumer::HeartbeatRead| {
                    let time = r
                        .owner_time
                        .unwrap_or(zenkey::model::heartbeat::Time::Unknown);
                    let mut all = read.lock().expect("not poisoned");
                    all.entry(r.provider)
                        .or_default()
                        .push_clock(ClockHeard::Heartbeat(time));
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let h = Arc::clone(&heard_v2);
        let faults_v2 = consumer_v2
            .subscribe(
                zenkey::health::FAULTS,
                move |d: zenkey::consumer::Delivery| {
                    let tag = d.tag.map_or(0, |t| t.get());
                    let bytes = d.sample.payload().to_bytes().into_owned();
                    // Its code is a word whatever the epoch: the record's
                    // time plays no part in whether it is `clock_ahead`.
                    let clock_ahead = FaultValue::decode_v2(tag, &bytes, Some(0)).is_clock_ahead();
                    let mut all = h.lock().expect("not poisoned");
                    let s = all.entry(d.provider).or_default();
                    s.faults += 1;
                    s.push_clock(if clock_ahead {
                        ClockHeard::ClockAhead
                    } else {
                        ClockHeard::Fault
                    });
                    s.raw_fault = Some((tag, bytes));
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        let asked = Instant::now();
        let (one, two) = tokio::join!(
            consumer.get_answer(zenkey::health::STATUS, None, timeout),
            consumer_v2.get_answer(zenkey::health::STATUS, None, timeout)
        );
        if let Ok(answer) = one {
            status.seed(&answer, asked);
        }
        if let Ok(answer) = two {
            status_v2.seed(&answer, asked);
        }
        Ok(Window {
            status,
            faults,
            status_v2,
            faults_v2,
            heard,
            heard_v2,
            selectors,
        })
    }

    /// What the window heard, read now: every address's status as a
    /// `freshness.v1` observation (§2.5) under each major, each `health.v2`
    /// record read under its owner's epoch (`epochs`), and each clock's
    /// measurement across every subscription, faults included: a stamp
    /// ahead of this host's clock beyond the delta proves the clocks
    /// disagree (§2.6).
    fn close(self, addrs: BTreeSet<Addr>, epochs: &crate::bus::query::EntryEpochs) -> HealthWindow {
        let heard = self.heard.lock().expect("not poisoned").clone();
        let mut heard_v2 = self.heard_v2.lock().expect("not poisoned").clone();
        for (a, h) in &mut heard_v2 {
            let epoch = epochs.of(&status_key_of(a, Major::V2));
            if let Some((tag, bytes, stamp)) = h.raw_status.take() {
                h.last = Some(Replied {
                    value: Some(StatusValue::decode_v2(tag, &bytes, epoch)),
                    stamp,
                });
            }
            if let Some((tag, bytes)) = h.raw_fault.take() {
                h.last_fault = Some(FaultValue::decode_v2(tag, &bytes, epoch));
            }
        }
        let mut clocks = self.status.clock_offsets();
        for sub in [&self.faults, &self.status_v2, &self.faults_v2] {
            for (clock, m) in sub.clock_offsets() {
                clocks
                    .entry(clock)
                    .and_modify(|all| *all = all.merge(m))
                    .or_insert(m);
            }
        }
        let observe = |sub: &zenkey::consumer::Subscription,
                       heard: &BTreeMap<Addr, ServiceHeard>,
                       major: Major| {
            addrs
                .iter()
                .cloned()
                .chain(heard.keys().cloned())
                .map(|a| {
                    let o = sub.freshness(&status_key_of(&a, major));
                    (a, o)
                })
                .collect()
        };
        HealthWindow {
            selectors: self.selectors,
            listened: self.status.listened(),
            status: observe(&self.status, &heard, Major::V1),
            status_v2: observe(&self.status_v2, &heard_v2, Major::V2),
            heard,
            heard_v2,
            clocks,
        }
    }
}

/// An absent owner's status, from the first archive presence shows that
/// holds it (core §4.4, S5): presence read whole, its `archive.v1`
/// providers by token or by descriptor, each asked in turn, for the
/// `health.v2` status key first and then `health.v1`'s. A `health.v2`
/// record is read under CH's tag and the owner's epoch (`epochs`).
async fn archived(
    session: &Session,
    owner: &Addr,
    timeout: Duration,
    epochs: &crate::bus::query::EntryEpochs,
) -> Result<Option<ArchivedStatus>, String> {
    let all = crate::bus::presence::observe(session, &Scope::all(), timeout)
        .await
        .map_err(|e| crate::one_line(&e))?;
    let archive = IfaceId::from_str(ARCHIVE).expect("archive.v1 is an interface id");
    for a in &Catalog::new(&all).providers_of(&archive) {
        for major in [Major::V2, Major::V1] {
            let origin = status_key_of(owner, major);
            let got = zenkey::archive::last_known(session, a, &origin, timeout)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(lk) = got {
                let tag = lk.tag.map_or(0, |t| t.get());
                return Ok(Some(ArchivedStatus {
                    archive: a.clone(),
                    major,
                    value: lk.value.as_deref().map(|v| match major {
                        Major::V1 => StatusValue::decode(v),
                        Major::V2 => StatusValue::decode_v2(tag, v, epochs.of(&origin)),
                    }),
                    stamp: lk.timestamp.map(|t| Stamped {
                        time: t.get_time().to_system_time(),
                        clock: t.get_id().to_string(),
                    }),
                    confirmed: lk.confirmed,
                }));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #819 TG (review of PR #835): a health window opens on proof only.
    /// Across a face, the first GET proves the owner holds the subscription
    /// when the owner replied and its final came, never when it ran to its
    /// timeout, drew no reply or could not be made. With no face, every
    /// instance presence found must have answered its descriptor GET, made
    /// after the subscription.
    #[test]
    fn a_window_opens_on_proof_only() {
        use crate::model::catalog::DescriptorRead;
        let a: Addr = "lab/nav".parse().unwrap();
        let got = |complete: bool, replied: bool| {
            Ok(HealthGet {
                v2: BTreeMap::new(),
                selector: "zk2/lab/nav/health.v1/state/**".into(),
                read_at: SystemTime::now(),
                complete,
                services: if replied {
                    BTreeMap::from([(a.clone(), ServiceGot::default())])
                } else {
                    BTreeMap::new()
                },
            })
        };
        let says = |o: HealthOpening| match o {
            HealthOpening::Answered => "answered".to_owned(),
            HealthOpening::Unanswered(why) => why,
        };
        assert_eq!(
            opening_of(true, true, None, Some(&got(true, true))),
            HealthOpening::Answered
        );
        assert!(says(opening_of(true, true, None, Some(&got(false, true)))).contains("timeout"));
        assert!(says(opening_of(true, true, None, Some(&got(true, false)))).contains("no reply"));
        assert!(
            says(opening_of(true, true, None, Some(&Err("closed".into()))))
                .contains("could not be made: closed")
        );
        let instances = ["0000000000000001", "0000000000000002"];
        let observed = |reads: &[DescriptorRead]| {
            let keys: Vec<String> = instances
                .iter()
                .map(|i| format!("zk2/lab/nav/@zk/instance/{i}"))
                .collect();
            let mut o = Observed::from_keys("zk2/lab/nav/@zk/**", &keys, true);
            o.descriptors = Some(
                instances
                    .iter()
                    .zip(reads)
                    .map(|(i, r)| ((a.clone(), i.parse().unwrap()), r.clone()))
                    .collect(),
            );
            Ok(o)
        };
        let answered = DescriptorRead::Invalid("a reply all the same".into());
        let both = observed(&[answered.clone(), answered.clone()]);
        assert_eq!(
            opening_of(false, true, Some(&both), None),
            HealthOpening::Answered
        );
        assert!(
            says(opening_of(false, false, Some(&both), None)).contains("before the subscription")
        );
        let one = observed(&[answered, DescriptorRead::Silent]);
        assert!(says(opening_of(false, true, Some(&one), None)).contains("1 of the 2 instance(s)"));
        let none = Ok(Observed::from_keys("zk2/lab/nav/@zk/**", &[], true));
        assert!(says(opening_of(false, true, Some(&none), None)).contains("no instance"));
    }

    #[test]
    fn a_target_spells_its_selectors() {
        assert_eq!(
            HealthTarget::All.state_selector(),
            "zk2/*/*/health.v1/state/**"
        );
        let a: Addr = "lab/svc".parse().unwrap();
        assert_eq!(
            HealthTarget::One(a.clone()).state_selector(),
            "zk2/lab/svc/health.v1/state/**"
        );
        assert_eq!(status_key(&a), "zk2/lab/svc/health.v1/state/status");
        assert_eq!(
            health_key_of(
                "zk2/lab/svc/health.v1/state/checks/disk",
                KindToken::State,
                Major::V1
            ),
            Some((a, vec!["checks".to_owned(), "disk".to_owned()]))
        );
        assert_eq!(
            health_key_of(
                "zk2/lab/svc/nav.v2/state/status",
                KindToken::State,
                Major::V1
            ),
            None
        );
    }

    #[test]
    fn a_payload_that_does_not_decode_is_no_level() {
        let s = v1::Status {
            level: 3,
            reason: "down".into(),
            since_ns: 7,
        };
        assert_eq!(
            StatusValue::decode(&s.encode_to_vec()),
            StatusValue {
                word: None,
                since_s: None,
                level: Read::Level(hm::Level::Failed),
                reason: "down".into(),
                since_ns: 7
            }
        );
        assert_eq!(StatusValue::decode(&[0xff, 0xff]).level, Read::Undecodable);
        let mut h = ServiceHeard::default();
        for x in [
            Heard::Heartbeat,
            Heard::Heartbeat,
            Heard::ClockAhead,
            Heard::Heartbeat,
        ] {
            h.push(x);
        }
        assert_eq!(
            h.sequence,
            [Heard::Heartbeat, Heard::ClockAhead, Heard::Heartbeat],
            "repeats collapse, the order stays"
        );
    }
}
