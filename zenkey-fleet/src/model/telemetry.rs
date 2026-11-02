//! `telemetry.v1` (#819, TC): one reader's metric members, built from what
//! the bus delivered (`spec/profiles/telemetry/v1.md` §2.5–§2.11).
//!
//! **Values in hand → meaning.** A [`TelemetryBook`] is one reader's view of
//! the metric members it heard. It is fed values, never a session — a
//! presence seed, each token's move, each sample of each member (a
//! subscription's delivery or a reply to S4's GET), and each loss — so a
//! `.zrec` replays through it as live traffic does. Time is an argument too:
//! an [`Instant`] on this host's monotonic clock.
//!
//! What it builds is what `zenkey_model::telemetry` judges, and every verdict
//! is that module's, never re-derived here:
//!
//! - **a member's resource** ([`MetricResource`]): its declaration
//!   (`declaration_of`, §2.2, §2.4), how its samples are carried, and the
//!   template's `epoch` parameter, when it declares one;
//! - **each sample's reading** (`check`, §2.5, §2.6): its conformance with
//!   its reasons, the member's kind and unit as it gives them, and its value
//!   when it conforms, decoded as §2.6 reads bytes (`Datum::decode`, strict
//!   JSON or CBOR, and `Datum::attachment` for the Epoch);
//! - **the pairs** (`pair`, §2.9–§2.11): each two consecutive conforming
//!   samples of a member, in the order received for a stream, and, for
//!   state, in the order a late joiner's merge keeps them (§2.10, 0.2;
//!   `zenkey_model::seed`, one [`Held`] per key for the book's life): a
//!   GET's reply older than a put already held is no sample of any pair;
//! - **presence**, the epoch's second evidence (§2.9): the owner's instance
//!   tokens, or a member's tokens for a template that declares `epoch`, as a
//!   [`LivePresence`] folds them. Samples and token moves are folded in the
//!   **one** order they were received, so "alone throughout" is what the
//!   reader saw between two arrivals. A loss — a `Dropped(n)`, a stall of
//!   the receive path, anything the reader did not see — makes presence
//!   unobservable for every pair it spans: it never yields `same_epoch` by
//!   presence. Presence stands only on a complete seed; before one, and
//!   across a constrained face where it does not cross (§2.14), a pair has
//!   none. An in-band epoch needs none.
//!
//! **Time bases** (§2.11): a Point's `timestamp`, then, for state, the
//! owner's Zenoh timestamp in nanoseconds — a state metric resource is on
//! core §4.3's table, so it carries one — `zenkey_model::telemetry::stamp_ns`
//! of each stamp, converted alone (0.2) — then the arrival on this host's
//! monotonic clock, in nanoseconds since the book opened. No time base
//! compares the owner's clock with this host's, so no clock is trusted. A
//! state member's samples are *merged* by the in-band stamp (core §4.2,
//! §4.3); the Zenoh timestamp only times them.
//!
//! **Bounded.** At most [`MAX_MEMBERS`] members are held (a member past it
//! is refused and counted), and each keeps its last [`PAIRS_KEPT`] pairs
//! beside a tally of all of them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use zenkey_model::authoring::{Encoding, Kind};
use zenkey_model::grammar::{Addr, IfaceId, InstanceId, ZkKey};
use zenkey_model::schema::TypeId;
use zenkey_model::seed::Held;
use zenkey_model::telemetry::frames::{self, FieldReading, FieldSeries, Fold, Folded, Series};
use zenkey_model::telemetry::{
    self as tm, Checked, Conformance, Datum, Declaration, Measure, MetricKind, Pair, Payload,
    Problem, Reading, RecordTime, Source, TimeBasis,
};
use zenkey_model::template::Segment;
use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};
use zenoh::time::Timestamp;

use crate::bus::presence::Scope;
use crate::model::catalog::{Observed, Revision};
use crate::model::presence_live::{LivePresence, PresenceBasis};
use crate::report::{MemberId, ScopeReach, UnreachedResource};

/// The most members a book holds; a member past it is refused and counted.
pub const MAX_MEMBERS: usize = 4_096;

/// The pairs a member keeps for a report, the latest; every pair is tallied.
pub const PAIRS_KEPT: usize = 16;

// ─── what a sample is read as ───────────────────────────────────────────────

/// What reading one sample of a resource needs (§2.4, §2.6): its
/// declaration, and how its payload and its attachment are carried.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricType {
    /// `declared` or `invalid`: a resource that is not this profile's, or an
    /// event or an operation, has no [`MetricType`].
    pub declaration: Declaration,
    /// The payload's encoding: JSON unless CBOR.
    pub wire: Option<Encoding>,
    /// The attachment's encoding: JSON unless CBOR.
    pub attachment_wire: Option<Encoding>,
    /// A raw type: its samples are bytes, always (§2.4).
    pub raw: bool,
}

impl MetricType {
    /// A frame resource's: none of v1's (`not_this_profiles`), its series
    /// [`FrameMetric`]'s.
    pub fn frame() -> Self {
        MetricType {
            declaration: Declaration::NotThisProfiles,
            wire: None,
            attachment_wire: None,
            raw: false,
        }
    }

    /// The metric type of `r`, a resource of `c`: `None` when it is no
    /// metric resource (§2.1) — not this profile's, or an event or an
    /// operation, whose annotations are ignored (§2.2).
    pub fn of(
        c: &zenkey_model::contract::Contract,
        r: &zenkey_model::contract::Resource,
    ) -> Option<Self> {
        let zenkey_model::contract::Body::Data(d) = &r.body else {
            return None;
        };
        if !matches!(r.kind, Kind::Stream | Kind::State) {
            return None;
        }
        // A frame resource of a `telemetry.v2` contract: v1's four keys mean
        // nothing there, wherever they resolve from (v2 §2.16); its series
        // are [`FrameMetric`]'s.
        if d.frame.is_some() && tm::uses_v2(c) {
            return None;
        }
        let declaration = tm::declaration_of(c, r);
        if matches!(
            declaration,
            Declaration::NotThisProfiles | Declaration::Ignored
        ) {
            return None;
        }
        Some(MetricType {
            declaration,
            wire: d.encoding,
            attachment_wire: d.attachment_encoding,
            raw: matches!(d.type_, TypeId::Raw { .. }),
        })
    }

    /// One sample read (§2.5, §2.6): its bytes decoded as §2.6 reads them —
    /// strict JSON, or CBOR with its bignums; a raw type's bytes as they are
    /// — and its attachment as the Epoch's datum (a non-decoding one is an
    /// epoch that is not a string, 0.2), then the profile's `check`.
    pub fn check(&self, payload: &[u8], attachment: Option<&[u8]>) -> Checked {
        let value = if self.raw {
            Some(Datum::Bytes(payload.to_vec()))
        } else {
            Datum::decode(payload, self.wire)
        };
        let attachment = attachment.map(|a| Datum::attachment(a, self.attachment_wire));
        let p = match &value {
            Some(value) => Payload::Decoded {
                value,
                attachment: attachment.as_ref(),
            },
            None => Payload::Undecodable,
        };
        tm::check(&self.declaration, &p)
    }
}

/// A frame resource's series (`telemetry.v2` §2.16, #983 T2): its layout,
/// by which a frame of its key is read (core §7.7), and its
/// `telemetry.series`, declared or invalid. A frame resource with no
/// series (`ignored`, `not_this_profiles`) has none: it is no metric
/// resource.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameMetric {
    pub layout: Arc<zenkey_model::frame::Layout>,
    /// [`Series::Declared`] or [`Series::Invalid`].
    pub series: Series,
}

impl FrameMetric {
    /// `r`'s series, when `r` is a frame resource of a contract that uses
    /// `telemetry.v2` and its `telemetry.series` is declared or invalid
    /// (`zenkey_model::telemetry::frames::series_of`, its mirrors read in
    /// `c`). A frame resource of a `telemetry.v1` contract is v1's to read
    /// ([`MetricType::of`]), which reads none of its fields (v2 §1).
    pub fn of(
        c: &zenkey_model::contract::Contract,
        r: &zenkey_model::contract::Resource,
    ) -> Option<FrameMetric> {
        if !tm::uses_v2(c) || r.kind != Kind::Stream {
            return None;
        }
        let layout = zenkey_model::frame::Layout::of_resource(c, r)
            .ok()
            .flatten()?;
        let series = frames::series_of(c, r)?;
        matches!(series, Series::Declared(_) | Series::Invalid(_)).then(|| FrameMetric {
            layout: Arc::new(layout),
            series,
        })
    }

    /// Its series, when its declaration is valid.
    pub fn declared(&self) -> Option<&[FieldSeries]> {
        self.series.declared()
    }

    /// Its declaration's problems, when it is invalid.
    pub fn problems(&self) -> Option<&[frames::Problem]> {
        match &self.series {
            Series::Invalid(p) => Some(p),
            _ => None,
        }
    }
}

/// An invalid `telemetry.series`' problems (v2 §2.16) in a tool's words:
/// `<entry>: <why> (<token>)`, the annotation whole with no entry.
pub fn series_problem_words(problems: &[frames::Problem]) -> String {
    problems
        .iter()
        .map(|p| match &p.entry {
            Some(e) => format!("{e}: {} ({})", p.reason.says(), p.reason.as_str()),
            None => format!("{} ({})", p.reason.says(), p.reason.as_str()),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A reason of §2.6 in a tool's words: `telemetry.v1 §2.6 <token>: <why>`.
pub fn problem_words(p: Problem) -> String {
    format!("telemetry.v1 §2.6 {}: {}", p.as_str(), p.says())
}

/// A declaration's reasons (§2.2) in a tool's words.
pub fn invalid_words(why: &[tm::Invalid]) -> String {
    why.iter()
        .map(|w| format!("{} ({})", w.says(), w.as_str()))
        .collect::<Vec<_>>()
        .join("; ")
}

// ─── the scope ──────────────────────────────────────────────────────────────

/// The metric members a reading reaches: every metric resource the
/// descriptors in scope declare, or those a base-relative selector, in the
/// deployment's namespace, intersects.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TelemetryScope {
    selector: Option<(String, OwnedKeyExpr)>,
    system: Option<String>,
    service: Option<String>,
    iface: Option<IfaceId>,
}

/// A chunk usable as a literal position of a narrower scope.
fn literal(c: Option<&&str>) -> Option<String> {
    c.filter(|c| !c.contains('*') && !c.is_empty())
        .map(|c| (*c).to_owned())
}

impl TelemetryScope {
    /// Every metric resource of every service in the namespace.
    pub fn all() -> TelemetryScope {
        TelemetryScope::default()
    }

    /// Every metric resource of one service.
    pub fn of(addr: &Addr) -> TelemetryScope {
        TelemetryScope {
            selector: None,
            system: Some(addr.system.to_string()),
            service: Some(addr.service.to_string()),
            iface: None,
        }
    }

    /// The scope narrowed to one interface's resources (`check conform`).
    pub fn with_iface(mut self, iface: IfaceId) -> TelemetryScope {
        self.iface = Some(iface);
        self
    }

    /// A selector, refused when it is not a key expression or not
    /// base-relative (`zk2/…`).
    pub fn parse(selector: &str) -> std::result::Result<TelemetryScope, String> {
        let ke = OwnedKeyExpr::try_from(selector.to_owned())
            .map_err(|e| format!("{selector:?} is not a key expression: {e}"))?;
        let chunks: Vec<&str> = selector.split('/').collect();
        if chunks.first() != Some(&"zk2") {
            return Err(format!(
                "{selector:?} is not base-relative: metrics are read through a session in the \
                 deployment's namespace, so the selector starts at zk2/ (the namespace is \
                 --namespace)"
            ));
        }
        let system = literal(chunks.get(1));
        let service = system.as_ref().and_then(|_| literal(chunks.get(2)));
        Ok(TelemetryScope {
            selector: Some((selector.to_owned(), ke)),
            system,
            service,
            iface: None,
        })
    }

    /// The selector as given; `None` for every metric resource.
    pub fn selector(&self) -> Option<&str> {
        self.selector.as_ref().map(|(s, _)| s.as_str())
    }

    /// The one owner the scope names, when its system and service are both
    /// literal.
    pub fn one_owner(&self) -> Option<Addr> {
        format!("{}/{}", self.system.as_ref()?, self.service.as_ref()?)
            .parse()
            .ok()
    }

    /// The narrowest presence scope covering the owners in scope.
    pub fn presence_scope(&self) -> Scope {
        if let Some(a) = self.one_owner() {
            return Scope::service(&a);
        }
        self.system
            .as_deref()
            .and_then(|s| Scope::system(s).ok())
            .unwrap_or_else(Scope::all)
    }

    /// Whether `addr`'s resources may be in scope.
    pub fn selects_owner(&self, addr: &Addr) -> bool {
        self.system
            .as_deref()
            .is_none_or(|s| addr.system.as_str() == s)
            && self
                .service
                .as_deref()
                .is_none_or(|s| addr.service.as_str() == s)
    }

    /// What to subscribe to for `r`'s members in scope, when any are: its
    /// key expression, or the selector where that is narrower. `None` when
    /// the two do not intersect.
    pub fn subscription(&self, r: &MetricResource) -> Option<String> {
        self.subscription_to(&r.address, &r.iface, &r.selector)
    }

    /// [`TelemetryScope::subscription`] of any resource: its owner, its
    /// interface and its members' key expression, base-relative (a
    /// `timing.v1` reading's scope is this one, #908).
    pub fn subscription_to(
        &self,
        address: &Addr,
        iface: &IfaceId,
        selector: &str,
    ) -> Option<String> {
        if !self.selects_owner(address) || self.iface.as_ref().is_some_and(|i| i != iface) {
            return None;
        }
        let Some((given, sel)) = &self.selector else {
            return Some(selector.to_owned());
        };
        let ours = KeyExpr::try_from(selector).ok()?;
        if !sel.intersects(&ours) {
            return None;
        }
        Some(if ours.includes(sel) {
            given.clone()
        } else {
            selector.to_owned()
        })
    }

    /// Whether the key `key` is in scope.
    pub fn selects_key(&self, key: &str) -> bool {
        match &self.selector {
            None => true,
            Some((_, sel)) => KeyExpr::try_from(key).is_ok_and(|k| sel.intersects(&k)),
        }
    }

    /// Whether the selector misses a resource of an owner and an interface
    /// in scope, whose members are under `selector`: the resource is there,
    /// and out of the selector's reach (`spec/observer.md` §8.2; #950).
    /// Never with no selector, which subscribes to each resource by its own
    /// key expression.
    pub fn misses(&self, address: &Addr, iface: &IfaceId, selector: &str) -> bool {
        self.selector.is_some()
            && self.selects_owner(address)
            && self.iface.as_ref().is_none_or(|i| i == iface)
            && self.subscription_to(address, iface, selector).is_none()
    }

    /// What the selector cannot reach, over `missed` (those
    /// [`TelemetryScope::misses`] finds); `None` with no selector.
    pub fn reach(&self, missed: Vec<UnreachedResource>) -> Option<ScopeReach> {
        self.selector().map(|s| ScopeReach::of(s, missed))
    }
}

// ─── a metric resource of one owner ─────────────────────────────────────────

/// One metric resource of one owner, as a reader reads its members (§2.1):
/// where it is, what its samples are read as, and the key expression its
/// members are under.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricResource {
    pub address: Addr,
    pub iface: IfaceId,
    pub fingerprint: String,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// `Stream` or `State`.
    pub kind: Kind,
    /// Its `telemetry.v1` declaration and carriage. A frame resource of a
    /// `telemetry.v2` contract has none of v1's (`not_this_profiles`): its
    /// series are [`MetricResource::frame`]'s.
    pub ty: MetricType,
    /// A frame resource's series (`telemetry.v2` §2.16): its fields' metric
    /// series, each its own member ([`TelemetryBook::frame`]).
    pub frame: Option<Arc<FrameMetric>>,
    /// The resource chunk the template's `epoch` parameter sits at, when it
    /// declares one (core §2.2): that member's tokens are its presence
    /// (§2.9).
    epoch_at: Option<usize>,
    /// Its members' key expression, base-relative: every parameter `*`, a
    /// rest parameter `**`.
    pub selector: String,
}

impl MetricResource {
    /// The resource at `index` of `revision`, as `address` provides it;
    /// `None` when it is no metric resource ([`MetricType::of`]).
    pub fn of(address: &Addr, revision: &Revision, index: usize) -> Option<MetricResource> {
        let c = revision.contract();
        let r = c.resources.get(index)?;
        let (ty, frame) = match FrameMetric::of(c, r) {
            Some(f) => (MetricType::frame(), Some(Arc::new(f))),
            None => (MetricType::of(c, r)?, None),
        };
        let epoch_at = r.epoch.as_ref().and_then(|p| {
            r.template
                .segments()
                .iter()
                .position(|s| matches!(s, Segment::Param(n) if n == p))
        });
        let chunks: Vec<&str> = r
            .template
            .segments()
            .iter()
            .map(|s| match s {
                Segment::Literal(l) => l.as_str(),
                Segment::Param(_) => "*",
                Segment::Rest(_) => "**",
            })
            .collect();
        let selector = format!(
            "zk2/{}/{}/{}/{}/{}",
            address.system,
            address.service,
            revision.iface(),
            r.token.as_str(),
            chunks.join("/")
        );
        Some(MetricResource {
            address: address.clone(),
            iface: revision.iface().clone(),
            fingerprint: revision.fingerprint().to_string(),
            resource: zenkey::implementation::resource_name(r),
            kind: r.kind,
            ty,
            frame,
            epoch_at,
            selector,
        })
    }

    /// A resource from its parts, for a reader that holds no contract (a
    /// test, a replay with the declaration in hand).
    pub fn from_parts(
        address: Addr,
        iface: IfaceId,
        resource: &str,
        kind: Kind,
        ty: MetricType,
        epoch_at: Option<usize>,
    ) -> MetricResource {
        let (token, template) = resource.split_once('/').unwrap_or((resource, ""));
        MetricResource {
            selector: format!(
                "zk2/{}/{}/{iface}/{token}/{template}",
                address.system, address.service
            ),
            address,
            iface,
            fingerprint: String::new(),
            resource: resource.to_owned(),
            kind,
            ty,
            frame: None,
            epoch_at,
        }
    }

    /// The member a key names, for a template that declares `epoch`: its
    /// member token's id (core §8.1), the parameter's chunk as the key
    /// carries it.
    pub fn member_of(&self, key: &str) -> Option<MemberId> {
        let at = self.epoch_at?;
        let chunk = key.split('/').nth(5 + at)?;
        Some(MemberId {
            iface: self.iface.clone(),
            member: chunk.to_owned(),
        })
    }

    /// Whether the resource's declaration is valid: its `telemetry.series`
    /// for a frame resource (v2 §2.16), else its v1 declaration (§2.2).
    pub fn declared(&self) -> bool {
        match &self.frame {
            Some(f) => f.declared().is_some(),
            None => matches!(self.ty.declaration, Declaration::Declared(_)),
        }
    }

    /// Why its declaration is invalid, in a tool's words, the text that
    /// says so cited; `None` when it is valid.
    pub fn invalid_words(&self) -> Option<String> {
        match (&self.frame, &self.ty.declaration) {
            (Some(f), _) => f.problems().map(|p| {
                format!(
                    "its telemetry.series is invalid ({}): no field of it is read \
                     (telemetry.v2 §2.16)",
                    series_problem_words(p)
                )
            }),
            (None, Declaration::Invalid(why)) => Some(format!(
                "its declaration is invalid ({}): no question of its members is answered \
                 (telemetry.v1 §2.2)",
                invalid_words(why)
            )),
            _ => None,
        }
    }
}

// ─── the inputs ─────────────────────────────────────────────────────────────

/// How a sample reached this reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleVia {
    /// The reader's subscription.
    Subscription,
    /// The owner's reply to S4's GET (core §4.2).
    Get,
}

/// One sample of one metric member, as a reader received it.
#[derive(Debug, Clone)]
pub struct MetricSample {
    /// The concrete key, base-relative.
    pub key: String,
    /// A delete, or a `reply_del`: no value of any pair.
    pub delete: bool,
    pub payload: Vec<u8>,
    pub attachment: Option<Vec<u8>>,
    /// Its Zenoh timestamp, when it carried one: on a state metric
    /// resource, core §4.3's table, the owner's (S1), §2.11's time base. Its
    /// order is its in-band stamp ([`TelemetryBook::sample_enveloped`]);
    /// this orders only a sample with no envelope.
    pub stamp: Option<Timestamp>,
    /// Its arrival, on this host's monotonic clock.
    pub at: Instant,
    pub via: SampleVia,
    /// CH's tag, when its envelope is a frame's (core §7.5's form 5): the
    /// payload is then the frame's body, read by its resource's layout
    /// ([`TelemetryBook::frame`]). `None` for every other sample.
    pub frame: Option<zenkey_model::wire::Tag>,
}

// ─── presence, as one sample saw it ─────────────────────────────────────────

/// What the reader's presence view was when one sample arrived (§2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    /// A complete seed stood, and nothing was lost since.
    whole: bool,
    /// Losses folded so far: a pair across one has no presence.
    losses: u64,
    /// The moves of the tokens that count, folded so far.
    moves: u64,
    /// The only instance (or member epoch) held, when exactly one was.
    only: Option<String>,
}

/// How a pair's presence was had (§2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairPresence {
    /// What the reader saw over the pair: one instance alone throughout, or
    /// the only one at each arrival.
    Observed {
        alone_throughout: bool,
        earlier: Option<String>,
        later: Option<String>,
    },
    /// A loss between the two arrivals: presence establishes nothing.
    SpansLoss,
    /// No presence at either arrival: none read (across a face), no
    /// complete seed yet, or the basis lost.
    None,
}

impl PairPresence {
    /// The wire token.
    pub fn as_str(&self) -> &'static str {
        match self {
            PairPresence::Observed { .. } => "observed",
            PairPresence::SpansLoss => "spans_loss",
            PairPresence::None => "none",
        }
    }
}

// ─── one member ─────────────────────────────────────────────────────────────

/// A member's last conforming sample, held to pair with the next.
#[derive(Debug, Clone)]
struct Last {
    reading: Reading,
    stamp_ns: Option<i128>,
    arrival_ns: i128,
    seen: Option<Seen>,
}

/// One pair, as this reader judged it (§2.10, §2.11).
#[derive(Debug, Clone, PartialEq)]
pub struct PairSeen {
    pub earlier: Measure,
    pub later: Measure,
    /// The elapsed time the rate was timed over, in nanoseconds, and which
    /// base it was taken from: `timestamp`, `stamp` or `arrival` (§2.11),
    /// and for a frame's series `record` (its records' times) or `arrival`
    /// (v2 §2.21).
    pub elapsed_ns: i128,
    pub base: &'static str,
    /// A frame series' time basis (v2 §2.21): `measured`, `scheduled` with
    /// its bound, or `arrival`, named whether or not the pair has a rate.
    /// `None` for a v1 pair.
    pub basis: Option<TimeBasis>,
    pub pair: Pair,
    /// A frame series' pair consults none (v2 §2.19): always `None`.
    pub presence: PairPresence,
}

/// How many pairs answered each way (§2.10), and how their epoch was had.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PairTally {
    pub same_epoch: u64,
    pub reset: u64,
    pub decreased: u64,
    /// Incomparable (§2.10, 2), or a narrow counter's change outside its
    /// serial window (v2 §2.20), which `wrap_or_decrease` counts apart.
    pub unobservable: u64,
    /// Of `unobservable`: a wrap or a decrease, which a narrow counter's
    /// two values cannot tell apart (v2 §2.20).
    pub wrap_or_decrease: u64,
    /// Neither sample a counter or a histogram (§2.10, 1).
    pub not_asked: u64,
    pub in_band: u64,
    pub by_presence: u64,
    pub assumed: u64,
}

impl PairTally {
    /// Every pair tallied.
    pub fn total(&self) -> u64 {
        self.same_epoch + self.reset + self.decreased + self.unobservable + self.not_asked
    }

    /// Pairs whose epoch was asked: of two counters or histograms,
    /// comparable.
    pub fn asked(&self) -> u64 {
        self.in_band + self.by_presence + self.assumed
    }

    pub fn add(&mut self, p: &Pair) {
        match p.continuity {
            tm::Continuity::SameEpoch => self.same_epoch += 1,
            tm::Continuity::Reset => self.reset += 1,
            tm::Continuity::Decreased => self.decreased += 1,
            tm::Continuity::Unobservable => self.unobservable += 1,
            tm::Continuity::NotAsked => self.not_asked += 1,
        }
        if p.unobservable() == Some("wrap_or_decrease") {
            self.wrap_or_decrease += 1;
        }
        match p.basis {
            Some(tm::Basis::Established(tm::Via::InBand)) => self.in_band += 1,
            Some(tm::Basis::Established(tm::Via::Presence)) => self.by_presence += 1,
            Some(tm::Basis::Assumed) => self.assumed += 1,
            None => {}
        }
    }
}

/// A frame series' last sample, held to pair with the next (v2 §2.18):
/// its value, its record's time and its frame's arrival.
#[derive(Debug, Clone)]
struct FrameLast {
    measure: Measure,
    time: RecordTime,
    arrival_ns: i128,
}

impl FrameLast {
    /// As a pair reads it (v2 §2.21): its epoch the fold's, never asked here.
    fn sampled(&self) -> tm::Sampled<'_> {
        tm::Sampled {
            value: &self.measure,
            epoch: None,
            timestamp_ms: None,
            stamp_ns: None,
            arrival_ns: self.arrival_ns,
            record: Some(self.time),
        }
    }
}

/// One metric member, as this reader read it: a v1 member (one key), or
/// one series of a frame (its key and its field, v2 §2.17).
#[derive(Debug, Clone)]
pub struct MemberBook {
    pub metric: Arc<MetricResource>,
    /// The series this member is, for a frame resource: its field, kind,
    /// unit and mirror (v2 §2.16). `None` for a v1 member, and for a frame
    /// key whose `telemetry.series` is invalid, read as one member whose
    /// every frame is unobservable.
    pub field: Option<FieldSeries>,
    /// A frame series' records that were no sample (v2 §2.18), by the
    /// reading's token: `held`, `absent`, `owner_error`, `beyond_range`,
    /// `unnamed`, `unknown`. Empty for a v1 member.
    pub no_sample: BTreeMap<&'static str, u64>,
    /// A frame series' latest sample as core §7.6's JSON reading, a `fixed`
    /// gauge's exact decimal string included; `None` for a v1 member.
    pub latest_json: Option<serde_json::Value>,
    /// The member's kind and unit as its latest sample gave them (§2.5):
    /// the declared ones, else the latest conforming Point's; `None` when
    /// unobservable or unknown — never dimensionless.
    pub kind: Option<(MetricKind, Source)>,
    pub unit: Option<(String, Source)>,
    /// Every value received, a delete not counted.
    pub samples: u64,
    pub conforming: u64,
    pub does_not: u64,
    /// Unobservable (§2.6): the declaration invalid, or the sample
    /// undecodable.
    pub unobservable: u64,
    /// The reason class of the last unobservable sample.
    pub unobservable_why: Option<tm::Unasked>,
    /// State samples the late joiner's merge did not keep (§2.10, 0.2):
    /// no sample of any pair.
    pub not_kept: u64,
    pub deletes: u64,
    /// Each reason of §2.6, by how many samples gave it.
    pub problems: BTreeMap<Problem, u64>,
    /// The reasons of the latest sample that did not conform.
    pub last_does_not: Option<Vec<Problem>>,
    /// The latest conforming value.
    pub latest: Option<Reading>,
    pub tally: PairTally,
    /// The latest pairs, at most [`PAIRS_KEPT`].
    pub pairs: VecDeque<PairSeen>,
    /// The latest rate derived (§2.11).
    pub rate: Option<tm::Rate>,
    merged: Held,
    last: Option<Last>,
    frame_last: Option<FrameLast>,
}

impl MemberBook {
    fn new(metric: Arc<MetricResource>) -> MemberBook {
        MemberBook {
            metric,
            field: None,
            no_sample: BTreeMap::new(),
            latest_json: None,
            kind: None,
            unit: None,
            samples: 0,
            conforming: 0,
            does_not: 0,
            unobservable: 0,
            unobservable_why: None,
            not_kept: 0,
            deletes: 0,
            problems: BTreeMap::new(),
            last_does_not: None,
            latest: None,
            tally: PairTally::default(),
            pairs: VecDeque::new(),
            rate: None,
            merged: Held::Nothing,
            last: None,
            frame_last: None,
        }
    }

    /// One series of a frame, its kind and unit its entry's (v2 §2.17); or,
    /// with no series, the frame key whose declaration is invalid.
    fn of_series(metric: Arc<MetricResource>, field: Option<FieldSeries>) -> MemberBook {
        let mut m = MemberBook::new(metric);
        if let Some(f) = &field {
            m.kind = Some((f.kind, Source::Declared));
            m.unit = f.unit.clone().map(|u| (u, Source::Declared));
        }
        m.field = field;
        m
    }
}

// ─── the book ───────────────────────────────────────────────────────────────

/// One reader's metric members: see the module doc.
#[derive(Debug, Clone)]
pub struct TelemetryBook {
    t0: Instant,
    /// `None`: the reader reads no presence (across a face, §2.14).
    presence: Option<LivePresence>,
    losses: u64,
    lost: u64,
    instance_moves: BTreeMap<Addr, u64>,
    member_moves: BTreeMap<(Addr, MemberId), u64>,
    held_instances: BTreeMap<Addr, BTreeSet<InstanceId>>,
    held_members: BTreeMap<(Addr, MemberId), Vec<InstanceId>>,
    members: BTreeMap<String, MemberBook>,
    /// Each frame key's series, by its key and its field's index (v2
    /// §2.17); the frame key whose declaration is invalid by its key alone.
    series: BTreeMap<(String, Option<usize>), MemberBook>,
    /// Each frame key's fold: its reader, the epoch it reads samples
    /// under, each counter series' last sample (v2 §2.18–§2.21).
    folds: BTreeMap<String, Fold>,
    refused: u64,
}

impl TelemetryBook {
    /// An empty book: presence read over `scope`, or none at all when
    /// `scope` is `None` (across a face). Arrivals count from `t0`.
    pub fn new(scope: Option<Scope>, t0: Instant) -> TelemetryBook {
        TelemetryBook {
            t0,
            presence: scope.map(LivePresence::new),
            losses: 0,
            lost: 0,
            instance_moves: BTreeMap::new(),
            member_moves: BTreeMap::new(),
            held_instances: BTreeMap::new(),
            held_members: BTreeMap::new(),
            members: BTreeMap::new(),
            series: BTreeMap::new(),
            folds: BTreeMap::new(),
            refused: 0,
        }
    }

    /// Whether this reader reads presence at all.
    pub fn reads_presence(&self) -> bool {
        self.presence.is_some()
    }

    /// What presence stands on now; `None` when none is read.
    pub fn presence_basis(&self) -> Option<PresenceBasis> {
        self.presence.as_ref().map(LivePresence::basis)
    }

    /// The presence projection, when one is read.
    pub fn presence(&self) -> Option<&LivePresence> {
        self.presence.as_ref()
    }

    /// Deliveries lost and stalls, so far.
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// Members refused at [`MAX_MEMBERS`].
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Every member, by key, a frame key's series after one another in
    /// their fields' order (v2 §2.17).
    pub fn members(&self) -> impl Iterator<Item = (&String, &MemberBook)> {
        let mut all: Vec<(&String, Option<usize>, &MemberBook)> = self
            .members
            .iter()
            .map(|(k, m)| (k, None, m))
            .chain(self.series.iter().map(|((k, i), m)| (k, *i, m)))
            .collect();
        all.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        all.into_iter().map(|(k, _, m)| (k, m))
    }

    /// One member.
    pub fn member(&self, key: &str) -> Option<&MemberBook> {
        self.members.get(key)
    }

    /// One series of a frame key: its key and its field's name (v2 §2.17).
    pub fn series_member(&self, key: &str, field: &str) -> Option<&MemberBook> {
        self.series
            .range((key.to_owned(), None)..)
            .take_while(|((k, _), _)| k == key)
            .map(|(_, m)| m)
            .find(|m| m.field.as_ref().is_some_and(|f| f.field == field))
    }

    /// Members held, a frame's series each one.
    fn held(&self) -> usize {
        self.members.len() + self.series.len()
    }

    /// A presence seed (one liveliness read of the scope), then the token
    /// moves the subscription delivered while it was read, in order: what
    /// [`LivePresence::seed_then`] folds. A complete seed restores presence.
    /// Every move delivered during the seed is a move of its instance's
    /// address, or of its member, whatever the seed's net change: a token
    /// that came and went while the seed was read is never "alone
    /// throughout" across it. A put of a token held before the seed is no
    /// move: a reconnection can deliver one again, an owner's session
    /// declaring its tokens anew (#872). The watch session's subscribers
    /// have no history (#875), so none comes from a history.
    pub fn seed<'k>(&mut self, seed: &Observed, then: impl IntoIterator<Item = (&'k str, bool)>) {
        let Some(p) = &mut self.presence else {
            return;
        };
        let then: Vec<(&str, bool)> = then.into_iter().collect();
        let (instances, members) = (self.held_instances.clone(), self.held_members.clone());
        p.seed_then(seed, then.iter().copied(), std::time::SystemTime::now());
        self.restate();
        for (key, held) in then {
            match zenkey_model::grammar::parse(key) {
                Ok(ZkKey::Instance { addr, instance }) => {
                    if held && instances.get(&addr).is_some_and(|i| i.contains(&instance)) {
                        continue;
                    }
                    *self.instance_moves.entry(addr).or_default() += 1;
                }
                Ok(ZkKey::Member {
                    addr,
                    iface,
                    member,
                    epoch,
                }) => {
                    let m = (addr, MemberId { iface, member });
                    if held && members.get(&m).is_some_and(|e| e.contains(&epoch)) {
                        continue;
                    }
                    *self.member_moves.entry(m).or_default() += 1;
                }
                _ => {}
            }
        }
    }

    /// One token's move, as the subscription delivered it.
    pub fn token(&mut self, key: &str, held: bool) {
        let Some(p) = &mut self.presence else {
            return;
        };
        p.token(key, held, std::time::SystemTime::now());
        self.restate();
    }

    /// A loss: `n` deliveries the reader did not receive (`n` is 0 for a
    /// stall of its receive path, which may have lost anything). Every pair
    /// across it has no presence, and presence stands again only on a
    /// complete seed after it.
    pub fn lose(&mut self, n: u64) {
        self.losses += 1;
        self.lost += n.max(1);
        if let Some(p) = &mut self.presence {
            p.dropped(n.max(1), std::time::SystemTime::now());
        }
    }

    /// The tokens that count, compared with what was held: each set that
    /// moved is one move more.
    fn restate(&mut self) {
        let Some(p) = &self.presence else {
            return;
        };
        let mut instances: BTreeMap<Addr, BTreeSet<InstanceId>> = BTreeMap::new();
        let mut members: BTreeMap<(Addr, MemberId), Vec<InstanceId>> = BTreeMap::new();
        for (addr, a) in p.addresses() {
            let held: BTreeSet<InstanceId> = a.instance_tokens().cloned().collect();
            if !held.is_empty() {
                instances.insert(addr.clone(), held);
            }
            for (m, epochs) in &a.members {
                if !epochs.is_empty() {
                    members.insert((addr.clone(), m.clone()), epochs.clone());
                }
            }
        }
        let addrs: BTreeSet<Addr> = instances
            .keys()
            .chain(self.held_instances.keys())
            .cloned()
            .collect();
        for a in addrs {
            if instances.get(&a) != self.held_instances.get(&a) {
                *self.instance_moves.entry(a).or_default() += 1;
            }
        }
        let ms: BTreeSet<(Addr, MemberId)> = members
            .keys()
            .chain(self.held_members.keys())
            .cloned()
            .collect();
        for m in ms {
            if members.get(&m) != self.held_members.get(&m) {
                *self.member_moves.entry(m).or_default() += 1;
            }
        }
        self.held_instances = instances;
        self.held_members = members;
    }

    /// What presence was at this instant for a member of `metric` on `key`.
    fn seen(&self, metric: &MetricResource, key: &str) -> Option<Seen> {
        let p = self.presence.as_ref()?;
        let whole = p.basis() == PresenceBasis::Complete;
        let (moves, held): (u64, Vec<String>) = match metric.member_of(key) {
            Some(m) => {
                let k = (metric.address.clone(), m);
                (
                    self.member_moves.get(&k).copied().unwrap_or(0),
                    self.held_members
                        .get(&k)
                        .map(|e| e.iter().map(ToString::to_string).collect())
                        .unwrap_or_default(),
                )
            }
            None => (
                self.instance_moves
                    .get(&metric.address)
                    .copied()
                    .unwrap_or(0),
                self.held_instances
                    .get(&metric.address)
                    .map(|s| s.iter().map(ToString::to_string).collect())
                    .unwrap_or_default(),
            ),
        };
        Some(Seen {
            whole,
            losses: self.losses,
            moves,
            only: match held.as_slice() {
                [one] if whole => Some(one.clone()),
                _ => None,
            },
        })
    }

    fn arrival_ns(&self, at: Instant) -> i128 {
        i128::try_from(at.saturating_duration_since(self.t0).as_nanos()).unwrap_or(i128::MAX)
    }

    /// One sample with no envelope of a member of `metric`, the next
    /// received: read (§2.6), merged by its Zenoh timestamp for state
    /// (§2.10; core §4.2), and paired with the member's last conforming
    /// sample. Returns the pair it made, when it made one. An enveloped
    /// sample goes through [`TelemetryBook::sample_enveloped`], which merges
    /// it in band.
    pub fn sample(&mut self, metric: &Arc<MetricResource>, s: MetricSample) -> Option<PairSeen> {
        self.sample_enveloped(metric, s, None)
    }

    /// [`TelemetryBook::sample`] for a sample whose zk2c envelope says
    /// `envelope` (core §7.5, [`crate::model::inband`]): `s.payload` is
    /// then its record, and a state member is merged by its run and seq
    /// (`zenkey_model::seed::admits_mark`), its time still the owner's
    /// Zenoh stamp. A refused envelope is a sample that could not be read
    /// (undecodable), never merged.
    pub fn sample_enveloped(
        &mut self,
        metric: &Arc<MetricResource>,
        s: MetricSample,
        envelope: crate::model::inband::Envelope,
    ) -> Option<PairSeen> {
        if metric.frame.is_some() {
            self.frame(metric, s, envelope);
            return None;
        }
        if !self.members.contains_key(&s.key) && self.held() >= MAX_MEMBERS {
            self.refused += 1;
            return None;
        }
        let seen = self.seen(metric, &s.key);
        let arrival_ns = self.arrival_ns(s.at);
        let entry = self
            .members
            .entry(s.key.clone())
            .or_insert_with(|| MemberBook::new(Arc::clone(metric)));
        let state = metric.kind == Kind::State;
        let stamp_ns = s
            .stamp
            .as_ref()
            .filter(|_| state)
            .map(|t| tm::stamp_ns(t.get_time().as_u64()));
        if let Some(Err(_)) = envelope {
            entry.samples += 1;
            entry.unobservable += 1;
            entry.unobservable_why = Some(tm::Unasked::Undecodable);
            return None;
        }
        if state {
            let incoming = crate::model::inband::mark_of(envelope, s.stamp.as_ref());
            if !zenkey_model::seed::admits_mark(entry.merged, incoming) {
                entry.not_kept += 1;
                return None;
            }
            entry.merged = entry.merged.after_mark(incoming);
        }
        if s.delete {
            entry.deletes += 1;
            return None;
        }
        entry.samples += 1;
        let checked = metric.ty.check(&s.payload, s.attachment.as_deref());
        if checked.kind.is_some() {
            entry.kind = checked.kind;
        }
        if checked.unit.is_some() || checked.conformance == Conformance::Conforms {
            entry.unit = checked.unit.clone();
        }
        match &checked.conformance {
            Conformance::Conforms => entry.conforming += 1,
            Conformance::DoesNot(ps) => {
                entry.does_not += 1;
                for p in ps {
                    *entry.problems.entry(*p).or_default() += 1;
                }
                entry.last_does_not = Some(ps.clone());
                return None;
            }
            Conformance::Unobservable(why) | Conformance::NotAsked(why) => {
                entry.unobservable += 1;
                entry.unobservable_why = Some(*why);
                return None;
            }
        }
        let reading = checked.reading?;
        entry.latest = Some(reading.clone());
        let now = Last {
            reading,
            stamp_ns,
            arrival_ns,
            seen,
        };
        let prev = entry.last.replace(now)?;
        let now = entry.last.as_ref().expect("just held");
        let presence = match (&prev.seen, &now.seen) {
            (Some(a), Some(b)) if a.losses != b.losses => PairPresence::SpansLoss,
            (Some(a), Some(b)) if a.whole && b.whole => PairPresence::Observed {
                alone_throughout: a.moves == b.moves && a.only.is_some() && a.only == b.only,
                earlier: a.only.clone(),
                later: b.only.clone(),
            },
            _ => PairPresence::None,
        };
        let held = match &presence {
            PairPresence::Observed {
                alone_throughout,
                earlier,
                later,
            } => Some((*alone_throughout, earlier.clone(), later.clone())),
            _ => None,
        };
        let p = held.as_ref().map(|(alone, e, l)| tm::Presence {
            alone_throughout: *alone,
            earlier: e.as_deref(),
            later: l.as_deref(),
        });
        let unit = entry.unit.as_ref().map(|(u, _)| u.as_str());
        let pair = tm::pair(&sampled(&prev), &sampled(now), unit, p.as_ref());
        let (elapsed_ns, base) = elapsed(&prev, now);
        entry.tally.add(&pair);
        if let Ok(r) = &pair.rate {
            entry.rate = Some(r.clone());
        }
        let seen = PairSeen {
            earlier: prev.reading.value.clone(),
            later: now.reading.value.clone(),
            elapsed_ns,
            base,
            basis: None,
            pair,
            presence,
        };
        if entry.pairs.len() == PAIRS_KEPT {
            entry.pairs.pop_front();
        }
        entry.pairs.push_back(seen.clone());
        Some(seen)
    }

    /// `f` on each of a frame key's series members.
    fn each_series(&mut self, key: &str, indices: &[Option<usize>], f: impl Fn(&mut MemberBook)) {
        for i in indices {
            if let Some(m) = self.series.get_mut(&(key.to_owned(), *i)) {
                f(m);
            }
        }
    }

    /// Every series member of a frame key, made when the book holds none:
    /// one per series field, or the key alone when its declaration is
    /// invalid. `false` when making them would pass [`MAX_MEMBERS`].
    fn frame_members(
        &mut self,
        metric: &Arc<MetricResource>,
        key: &str,
        slots: &[(Option<usize>, Option<&FieldSeries>)],
    ) -> bool {
        let new = slots
            .iter()
            .filter(|(i, _)| !self.series.contains_key(&(key.to_owned(), *i)))
            .count();
        if new > 0 && self.held() + new > MAX_MEMBERS {
            self.refused += 1;
            return false;
        }
        for (i, f) in slots {
            self.series
                .entry((key.to_owned(), *i))
                .or_insert_with(|| MemberBook::of_series(Arc::clone(metric), f.cloned()));
        }
        true
    }

    /// One sample of a frame resource's key (`telemetry.v2` §2.18–§2.21,
    /// #983 T2), the next received: its body read by the resource's layout
    /// under CH's tag (core §7.7), then folded by the key's
    /// [`Fold`], whose reader gives each record's field states and whose
    /// epoch is the run. Each series member takes what its record says:
    /// an included value is a sample, which conforms (§2.18: core §7.7's
    /// reader checked its code), every other reading no sample, counted by
    /// its token; and a counter's sample after another of its series is a
    /// pair, judged by `zenkey_model::telemetry::pair` with no presence
    /// (§2.19), its time the records' (§2.21).
    ///
    /// A frame that cannot be read is unobservable for every series of its
    /// key: a body its layout refuses (noted by the fold's reader, so its
    /// records read as lost at the next frame), a mutation or a put with
    /// no envelope on a frame resource, a refused envelope. Under an invalid
    /// `telemetry.series` no field is read (§2.16): every frame is one
    /// unobservable sample of the key, `declaration_invalid`.
    pub fn frame(
        &mut self,
        metric: &Arc<MetricResource>,
        s: MetricSample,
        envelope: crate::model::inband::Envelope,
    ) -> FrameSeen {
        let Some(fm) = metric.frame.clone() else {
            return FrameSeen::default();
        };
        let key = s.key.clone();
        let series = fm.declared();
        let slots: Vec<(Option<usize>, Option<&FieldSeries>)> = match series {
            Some(d) => d.iter().map(|f| (Some(f.index), Some(f))).collect(),
            None => vec![(None, None)],
        };
        if !self.frame_members(metric, &key, &slots) {
            return FrameSeen::default();
        }
        let indices: Vec<Option<usize>> = slots.iter().map(|(i, _)| *i).collect();
        if s.delete {
            self.each_series(&key, &indices, |m| m.deletes += 1);
            return FrameSeen::default();
        }
        let unread = |book: &mut TelemetryBook, why: tm::Unasked| {
            book.each_series(&key, &indices, |m| {
                m.samples += 1;
                m.unobservable += 1;
                m.unobservable_why = Some(why);
            });
            FrameSeen {
                folded: None,
                pairs: Vec::new(),
                unread: Some(why),
            }
        };
        let Some(series) = series else {
            return unread(self, tm::Unasked::DeclarationInvalid);
        };
        let (Some(Ok(stamp)), Some(tag)) = (envelope, s.frame) else {
            return unread(self, tm::Unasked::Undecodable);
        };
        let frame = match zenkey_model::frame::Frame::parse(tag, &s.payload, &fm.layout) {
            Ok(f) => f,
            Err(_) => {
                self.folds.entry(key.clone()).or_default().refuse(stamp);
                return unread(self, tm::Unasked::Undecodable);
            }
        };
        let arrival_ns = self.arrival_ns(s.at);
        if !self.folds.contains_key(&key) && self.folds.len() >= MAX_MEMBERS {
            self.refused += 1;
            return FrameSeen::default();
        }
        let folded = self
            .folds
            .entry(key.clone())
            .or_default()
            .apply(&fm.layout, series, stamp, &frame, arrival_ns);
        let mut pairs = Vec::new();
        for (at, rec) in folded.records.iter().enumerate() {
            for read in &rec.fields {
                let Some(f) = series.get(read.series) else {
                    continue;
                };
                let Some(m) = self.series.get_mut(&(key.clone(), Some(f.index))) else {
                    continue;
                };
                let FieldReading::Sample(v) = &read.reading else {
                    *m.no_sample.entry(read.reading.as_str()).or_default() += 1;
                    continue;
                };
                m.samples += 1;
                m.conforming += 1;
                m.latest = Some(Reading {
                    value: v.measure.clone(),
                    timestamp_ms: None,
                    epoch: Some(stamp.run.hex()),
                });
                m.latest_json = Some(v.json.clone());
                let now = FrameLast {
                    measure: v.measure.clone(),
                    time: rec.time,
                    arrival_ns,
                };
                let before = m.frame_last.replace(now.clone());
                let (Some((_, pair)), Some(before)) = (&read.pair, before) else {
                    continue;
                };
                let (elapsed_ns, basis) = tm::elapsed(&before.sampled(), &now.sampled());
                m.tally.add(pair);
                if let Ok(r) = &pair.rate {
                    m.rate = Some(r.clone());
                }
                let seen = PairSeen {
                    earlier: before.measure,
                    later: now.measure,
                    elapsed_ns,
                    base: match basis {
                        Some(TimeBasis::Arrival) | None => "arrival",
                        Some(_) => "record",
                    },
                    basis,
                    pair: pair.clone(),
                    presence: PairPresence::None,
                };
                if m.pairs.len() == PAIRS_KEPT {
                    m.pairs.pop_front();
                }
                m.pairs.push_back(seen.clone());
                pairs.push(FramePair {
                    record: at,
                    series: read.series,
                    seen,
                });
            }
        }
        FrameSeen {
            folded: Some(folded),
            pairs,
            unread: None,
        }
    }
}

/// What [`TelemetryBook::frame`] read of one frame.
#[derive(Debug, Clone, Default)]
pub struct FrameSeen {
    /// The frame, folded: each record's seq and time, and each series'
    /// reading (v2 §2.18). `None` when it was not read.
    pub folded: Option<Folded>,
    /// The pairs it closed, in record order.
    pub pairs: Vec<FramePair>,
    /// Why the frame was not read: undecodable, or its declaration invalid.
    pub unread: Option<tm::Unasked>,
}

/// A pair a frame closed (v2 §2.18): the record of the frame its later
/// sample is in, the series' index among the resource's declared series,
/// and the pair as the book judged it.
#[derive(Debug, Clone)]
pub struct FramePair {
    pub record: usize,
    pub series: usize,
    pub seen: PairSeen,
}

/// A held sample as a pair reads it (§2.10, §2.11).
fn sampled(l: &Last) -> tm::Sampled<'_> {
    tm::Sampled {
        value: &l.reading.value,
        epoch: l.reading.epoch.as_deref(),
        timestamp_ms: l.reading.timestamp_ms,
        stamp_ns: l.stamp_ns,
        arrival_ns: l.arrival_ns,
        record: None,
    }
}

/// The elapsed time §2.11 times a pair over, and its base, as the model
/// takes it: the Points' timestamps, then the owner's Zenoh timestamps (core
/// §4.3's table: a time, where the in-band stamp is none), then the
/// arrivals.
fn elapsed(a: &Last, b: &Last) -> (i128, &'static str) {
    if let (Some(x), Some(y)) = (a.reading.timestamp_ms, b.reading.timestamp_ms) {
        ((i128::from(y) - i128::from(x)) * 1_000_000, "timestamp")
    } else if let (Some(x), Some(y)) = (a.stamp_ns, b.stamp_ns) {
        (y - x, "stamp")
    } else {
        (b.arrival_ns - a.arrival_ns, "arrival")
    }
}

#[cfg(test)]
mod tests {
    //! `spec/profiles/telemetry/conformance/continuity.json` through the
    //! book: each case's two samples put on the wire as an owner would put
    //! them, and the presence it names made of token moves folded in the
    //! one order a reader receives them — the book must read the fixture's
    //! answers.

    use super::*;
    use serde_json::{Value, json};
    use std::time::Duration;
    use zenkey_model::telemetry::{Basis, Continuity, Declared};
    use zenoh::time::{NTP64, TimestampId};

    const SVC: &str = "lab/meter";

    fn addr() -> Addr {
        SVC.parse().unwrap()
    }

    fn iface() -> IfaceId {
        "meter.v1".parse().unwrap()
    }

    /// The resource a case's samples are read as: a Point (undeclared, so
    /// each sample says its kind) unless the two are counters without a
    /// timestamp, which are a bare declared counter whose epoch rides the
    /// Epoch attachment; state when both carry the owner's stamp; a template
    /// that declares `epoch` when the presence the case names is a member's
    /// epochs.
    fn resource(case: &Value) -> (Arc<MetricResource>, bool) {
        let (e, l) = (&case["earlier"], &case["later"]);
        let timestamps = !e["timestamp_ms"].is_null() && !l["timestamp_ms"].is_null();
        let counters = e["kind"] == "counter" && l["kind"] == "counter";
        let point = timestamps || !counters;
        let stamped = |s: &Value| !s["stamp_ns"].is_null() || !s["stamp_ntp64"].is_null();
        let state = stamped(e) && stamped(l);
        let member = case["presence"]["earlier"]
            .as_str()
            .is_some_and(|s| s.len() == 16);
        let declaration = Declaration::Declared(Declared {
            carrier: if point {
                tm::Carrier::Point
            } else {
                tm::Carrier::Bare
            },
            kind: (!point).then_some(MetricKind::Counter),
            unit: (!point)
                .then(|| case["unit"].as_str().map(str::to_owned))
                .flatten(),
            buckets: None,
            semantic: None,
            epoch: Some(if point {
                tm::EpochIn::Point
            } else {
                tm::EpochIn::Attachment
            }),
        });
        let ty = MetricType {
            declaration,
            wire: None,
            attachment_wire: None,
            raw: false,
        };
        let (name, kind, epoch_at) = match (state, member) {
            (true, _) => ("state/total", Kind::State, None),
            (false, true) => ("stream/links/{link}", Kind::Stream, Some(1)),
            (false, false) => ("stream/frames", Kind::Stream, None),
        };
        (
            Arc::new(MetricResource::from_parts(
                addr(),
                iface(),
                name,
                kind,
                ty,
                epoch_at,
            )),
            point,
        )
    }

    /// A sample's value as the wire carries it.
    fn wire_value(kind: &str, v: &Value) -> Value {
        match kind {
            "histogram" => json!({
                "buckets": v["buckets"], "counts": v["counts"],
                "count": v["count"], "sum": v["sum"],
            }),
            _ => v.clone(),
        }
    }

    /// One case sample's payload and attachment, as an owner puts it.
    fn payload(case: &Value, which: &str, point: bool) -> (Vec<u8>, Option<Vec<u8>>) {
        let s = &case[which];
        let kind = s["kind"].as_str().unwrap();
        let value = wire_value(kind, &s["value"]);
        if point {
            let mut p = json!({
                // A timestamp the case leaves out is one no answer reads:
                // its kinds are not both cumulative, or not comparable.
                "timestamp": s["timestamp_ms"].as_i64().unwrap_or(1_700_000_000_000),
                "value": {"type": kind, "value": value},
            });
            if let Some(u) = case["unit"].as_str() {
                p["unit"] = json!(u);
            }
            if let Some(e) = s["epoch"].as_str() {
                p["epoch"] = json!(e);
            }
            (serde_json::to_vec(&p).unwrap(), None)
        } else {
            (
                serde_json::to_vec(&value).unwrap(),
                s["epoch"]
                    .as_str()
                    .map(|e| serde_json::to_vec(&json!(e)).unwrap()),
            )
        }
    }

    /// A stamp whose nanoseconds, as `stamp_ns` reads them, are the case's.
    fn stamp(s: &Value) -> Option<Timestamp> {
        let id = TimestampId::try_from([1u8; 16]).unwrap();
        if let Some(n) = s["stamp_ntp64"].as_u64() {
            return Some(Timestamp::new(NTP64(n), id));
        }
        let ns = s["stamp_ns"].as_i64()?;
        // The NTP64 value whose rounded-up fraction gives exactly `ns`.
        let (secs, sub) = (ns / 1_000_000_000, ns % 1_000_000_000);
        let frac = (u128::try_from(sub).unwrap() << 32) / 1_000_000_000;
        let mut n = (u64::try_from(secs).unwrap() << 32) | u64::try_from(frac).unwrap();
        while tm::stamp_ns(n) < i128::from(ns) {
            n += 1;
        }
        assert_eq!(tm::stamp_ns(n), i128::from(ns), "a stamp of {ns} ns");
        Some(Timestamp::new(NTP64(n), id))
    }

    fn instance_key(i: &str) -> String {
        format!("zk2/{SVC}/@zk/instance/{i}")
    }

    fn member_key(epoch: &str) -> String {
        format!("zk2/{SVC}/@zk/member/meter.v1/l1/{epoch}")
    }

    /// An instance id the fixture spells as a letter.
    fn id(s: &str) -> String {
        match s {
            "a" => "00000000000000aa".to_owned(),
            "b" => "00000000000000bb".to_owned(),
            other => other.to_owned(),
        }
    }

    /// The case's presence, made of what a reader receives: a complete seed,
    /// the token moves, and a loss, around the two samples.
    #[test]
    fn continuity_cases_read_through_the_book() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/telemetry/conformance/continuity.json");
        let doc: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for case in doc["cases"].as_array().unwrap() {
            let note = case["note"].as_str().unwrap();
            let (metric, point) = resource(case);
            let presence = &case["presence"];
            let t0 = Instant::now();
            let scope = (!presence.is_null()).then(Scope::all);
            let mut book = TelemetryBook::new(scope, t0);
            let token = if metric.epoch_at.is_some() {
                member_key
            } else {
                instance_key
            };
            let mut seed_keys = Vec::new();
            let (mut before, mut between) = (Vec::<(String, bool)>::new(), Vec::new());
            let mut loss = false;
            if let Some(p) = presence.as_object() {
                let alone = p["alone_throughout"].as_bool().unwrap();
                let (e, l) = (p["earlier"].as_str(), p["later"].as_str());
                match (alone, e, l) {
                    // One alone throughout: seeded, no move.
                    (true, Some(a), _) => seed_keys.push(token(&id(a))),
                    // The same at both arrivals, a loss between.
                    (false, Some(a), Some(b)) if a == b => {
                        seed_keys.push(token(&id(a)));
                        loss = true;
                    }
                    // Another alone at each arrival.
                    (false, Some(a), Some(b)) => {
                        seed_keys.push(token(&id(a)));
                        between.push((token(&id(b)), true));
                        between.push((token(&id(a)), false));
                    }
                    // Two at the earlier arrival, one at the later.
                    (false, None, Some(b)) => {
                        seed_keys.push(token(&id("a")));
                        before.push((token(&id(b)), true));
                        between.push((token(&id("a")), false));
                    }
                    other => panic!("{note}: no presence made for {other:?}"),
                }
                let seed = Observed::from_keys("zk2/*/*/@zk/**", &seed_keys, true);
                book.seed(&seed, std::iter::empty::<(&str, bool)>());
                for (k, held) in &before {
                    book.token(k, *held);
                }
            }
            let at = |s: &Value| {
                let ns = s["arrival_ns"].as_u64().unwrap_or(0);
                t0 + Duration::from_nanos(ns)
            };
            let sample = |which: &str| {
                let (p, a) = payload(case, which, point);
                MetricSample {
                    key: if metric.epoch_at.is_some() {
                        format!("zk2/{SVC}/meter.v1/stream/links/l1")
                    } else {
                        format!("zk2/{SVC}/meter.v1/{}", metric.resource)
                    },
                    delete: false,
                    payload: p,
                    attachment: a,
                    stamp: stamp(&case[which]),
                    at: at(&case[which]),
                    via: SampleVia::Subscription,
                    frame: None,
                }
            };
            assert!(book.sample(&metric, sample("earlier")).is_none(), "{note}");
            for (k, held) in &between {
                book.token(k, *held);
            }
            if loss {
                book.lose(1);
                let seed = Observed::from_keys("zk2/*/*/@zk/**", &seed_keys, true);
                book.seed(&seed, std::iter::empty::<(&str, bool)>());
            }
            let seen = book
                .sample(&metric, sample("later"))
                .unwrap_or_else(|| panic!("{note}: a pair"));
            let p = &seen.pair;
            let (epoch, via) = match p.basis {
                None => (None, None),
                Some(Basis::Assumed) => (Some("assumed"), None),
                Some(Basis::Established(v)) => (Some("established"), Some(v.as_str())),
            };
            let got = json!({
                "continuity": p.continuity.as_str(),
                "epoch": epoch,
                "via": via,
                "rate": p.rate.as_ref().ok().map(|r| r.per_s),
                "rate_unit": p.rate.as_ref().ok().and_then(|r| r.unit.clone()),
                "no_rate": p.rate.as_ref().err().map(|n| n.as_str()),
            });
            assert_eq!(got, case["expect"], "{note}");
            if p.continuity == Continuity::Decreased {
                assert_eq!(book.members().next().unwrap().1.tally.decreased, 1);
            }
        }
    }

    fn counter_metric(kind: Kind) -> Arc<MetricResource> {
        let declaration = Declaration::Declared(Declared {
            carrier: tm::Carrier::Bare,
            kind: Some(MetricKind::Counter),
            unit: Some("By".into()),
            buckets: None,
            semantic: None,
            epoch: None,
        });
        let name = if kind == Kind::State {
            "state/total"
        } else {
            "stream/drops"
        };
        Arc::new(MetricResource::from_parts(
            addr(),
            iface(),
            name,
            kind,
            MetricType {
                declaration,
                wire: None,
                attachment_wire: None,
                raw: false,
            },
            None,
        ))
    }

    fn bare(
        key: &str,
        n: u64,
        at: Instant,
        stamp: Option<Timestamp>,
        via: SampleVia,
    ) -> MetricSample {
        MetricSample {
            key: key.to_owned(),
            delete: false,
            payload: n.to_string().into_bytes(),
            attachment: None,
            stamp,
            at,
            via,
            frame: None,
        }
    }

    /// Scenario §9 (0.2): a state counter's pairs follow the late joiner's
    /// merge, so a GET's reply older than a put already held is no sample of
    /// any pair; paired in the order received it would read `decreased`.
    #[test]
    fn a_state_counters_pairs_follow_the_merge() {
        let t0 = Instant::now();
        let mut book = TelemetryBook::new(Some(Scope::all()), t0);
        book.seed(
            &Observed::from_keys("s", &[instance_key("00000000000000aa")], true),
            std::iter::empty::<(&str, bool)>(),
        );
        let m = counter_metric(Kind::State);
        let key = format!("zk2/{SVC}/meter.v1/state/total");
        let id = TimestampId::try_from([1u8; 16]).unwrap();
        let ts = |s: u64| Some(Timestamp::new(NTP64(s << 32), id));
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        assert!(
            book.sample(&m, bare(&key, 12, at(10), ts(2), SampleVia::Subscription))
                .is_none()
        );
        assert!(
            book.sample(&m, bare(&key, 10, at(20), ts(1), SampleVia::Get))
                .is_none()
        );
        let p = book
            .sample(&m, bare(&key, 15, at(220), ts(3), SampleVia::Subscription))
            .expect("12 → 15");
        assert_eq!(p.pair.continuity, Continuity::SameEpoch);
        assert_eq!(p.base, "stamp");
        assert_eq!(p.pair.rate.as_ref().unwrap().per_s, 3.0);
        let member = book.member(&key).unwrap();
        assert_eq!(
            (member.samples, member.not_kept, member.tally.decreased),
            (2, 1, 0)
        );
    }

    /// A loss between two arrivals leaves the pair no presence, even with
    /// one instance held at both: never `same_epoch` by presence. A stream
    /// before any complete seed has none either.
    #[test]
    fn a_loss_or_no_seed_leaves_presence_out() {
        let t0 = Instant::now();
        let m = counter_metric(Kind::Stream);
        let key = format!("zk2/{SVC}/meter.v1/stream/drops");
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut book = TelemetryBook::new(Some(Scope::all()), t0);
        book.sample(&m, bare(&key, 5, at(0), None, SampleVia::Subscription));
        let p = book
            .sample(&m, bare(&key, 3, at(500), None, SampleVia::Subscription))
            .unwrap();
        assert_eq!(p.presence, PairPresence::None, "unseeded");
        assert_eq!(p.pair.continuity, Continuity::Reset);
        let seed = Observed::from_keys("s", &[instance_key("00000000000000aa")], true);
        book.seed(&seed, std::iter::empty::<(&str, bool)>());
        book.sample(&m, bare(&key, 4, at(600), None, SampleVia::Subscription));
        book.lose(2);
        book.seed(&seed, std::iter::empty::<(&str, bool)>());
        let p = book
            .sample(&m, bare(&key, 1, at(900), None, SampleVia::Subscription))
            .unwrap();
        assert_eq!(p.presence, PairPresence::SpansLoss);
        assert_eq!(p.pair.basis, Some(Basis::Assumed));
        assert_eq!(p.pair.continuity, Continuity::Reset, "never decreased");
        let p = book
            .sample(&m, bare(&key, 0, at(1200), None, SampleVia::Subscription))
            .unwrap();
        assert_eq!(
            p.pair.continuity,
            Continuity::Decreased,
            "by presence again"
        );
        assert_eq!(book.lost(), 2);
    }

    /// Review of PR #827: a token that came and went while a seed was read
    /// is a move, though the seed's net change is none, so a pair across it
    /// is never "alone throughout".
    #[test]
    fn a_move_during_a_seed_is_a_move() {
        let t0 = Instant::now();
        let m = counter_metric(Kind::Stream);
        let key = format!("zk2/{SVC}/meter.v1/stream/drops");
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut book = TelemetryBook::new(Some(Scope::all()), t0);
        let seed = Observed::from_keys("s", &[instance_key("00000000000000aa")], true);
        book.seed(&seed, std::iter::empty::<(&str, bool)>());
        book.sample(&m, bare(&key, 5, at(0), None, SampleVia::Subscription));
        let transient = instance_key("00000000000000bb");
        book.seed(
            &seed,
            [(transient.as_str(), true), (transient.as_str(), false)],
        );
        let p = book
            .sample(&m, bare(&key, 3, at(500), None, SampleVia::Subscription))
            .unwrap();
        assert!(
            matches!(
                p.presence,
                PairPresence::Observed {
                    alone_throughout: false,
                    ..
                }
            ),
            "{:?}",
            p.presence
        );
        assert_eq!(
            p.pair.continuity,
            Continuity::Reset,
            "assumed, never decreased"
        );
    }
    /// #872: a put of a token held before a seed, delivered while it was
    /// read — an owner's session declaring its tokens anew at a
    /// reconnection — is no move, so a pair across it is still the one
    /// instance's alone throughout, and a decrease within it is found.
    #[test]
    fn a_put_of_a_token_already_held_during_a_seed_is_no_move() {
        let t0 = Instant::now();
        let m = counter_metric(Kind::Stream);
        let key = format!("zk2/{SVC}/meter.v1/stream/drops");
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut book = TelemetryBook::new(Some(Scope::all()), t0);
        let held = instance_key("00000000000000aa");
        let seed = Observed::from_keys("s", std::slice::from_ref(&held), true);
        book.seed(&seed, std::iter::empty::<(&str, bool)>());
        book.sample(&m, bare(&key, 5, at(0), None, SampleVia::Subscription));
        book.seed(&seed, [(held.as_str(), true)]);
        let p = book
            .sample(&m, bare(&key, 3, at(500), None, SampleVia::Subscription))
            .unwrap();
        assert!(
            matches!(
                p.presence,
                PairPresence::Observed {
                    alone_throughout: true,
                    ..
                }
            ),
            "{:?}",
            p.presence
        );
        assert_eq!(p.pair.continuity, Continuity::Decreased, "by presence");
    }

    /// `spec/profiles/telemetry/conformance/v2/continuity.json` through
    /// the book (#983 T2): each case's frames, their bodies under their
    /// layout's tag and their run and seq, folded by `TelemetryBook::frame`
    /// over `meterframe.v1`'s frame resources, the scenarios' contract;
    /// every pair the book reads is the fixture's, in order, and each
    /// series member keeps it.
    #[test]
    fn v2_continuity_cases_read_through_the_book() {
        use zenkey_model::wire::{Run, Seq, Stamp};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles");
        let doc: Value = serde_json::from_slice(
            &std::fs::read(root.join("telemetry/conformance/v2/continuity.json")).unwrap(),
        )
        .unwrap();
        let (set, problems) = crate::model::catalog::ContractSet::load_path(
            &root.join("telemetry/conformance/contracts/meterframe.v1.toml"),
        );
        assert!(problems.is_empty(), "{problems:?}");
        let rev = set.iter().next().expect("meterframe.v1");
        let addr: Addr = "lab/meterframe".parse().unwrap();
        let metric = |name: &str| {
            let template = format!("frames/{name}");
            let i = rev
                .contract()
                .resources
                .iter()
                .position(|r| r.template.as_str() == template)
                .expect("a frame resource");
            Arc::new(MetricResource::of(&addr, rev, i).expect("a metric resource"))
        };
        let unhex = |h: &str| -> Vec<u8> {
            (0..h.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
                .collect()
        };
        for case in doc["cases"].as_array().unwrap() {
            let note = case["note"].as_str().unwrap();
            let m = metric(case["layout"].as_str().unwrap());
            let f = m.frame.clone().expect("its series");
            let series = f.declared().expect("declared");
            let key = format!("zk2/lab/meterframe/meterframe.v1/{}", m.resource);
            let t0 = Instant::now();
            let mut book = TelemetryBook::new(None, t0);
            let mut got = Vec::new();
            for frame in case["frames"].as_array().unwrap() {
                let stamp = Stamp {
                    run: Run::from_hex(frame["run"].as_str().unwrap()).unwrap(),
                    seq: Seq::new(u32::try_from(frame["seq"].as_u64().unwrap()).unwrap()).unwrap(),
                };
                let arrival = frame["arrival_ns"].as_u64().unwrap_or(0);
                let seen = book.frame(
                    &m,
                    MetricSample {
                        key: key.clone(),
                        delete: false,
                        payload: unhex(frame["hex"].as_str().unwrap()),
                        attachment: None,
                        stamp: None,
                        at: t0 + Duration::from_nanos(arrival),
                        via: SampleVia::Subscription,
                        frame: Some(f.layout.tag()),
                    },
                    Some(Ok(stamp)),
                );
                assert!(seen.unread.is_none(), "{note}: {seen:?}");
                for p in seen.pairs {
                    let pair = &p.seen.pair;
                    let (epoch, via) = match pair.basis {
                        None => (None, None),
                        Some(Basis::Assumed) => (Some("assumed"), None),
                        Some(Basis::Established(v)) => (Some("established"), Some(v.as_str())),
                    };
                    let rate = pair.rate.as_ref().ok();
                    got.push(json!({
                        "field": series[p.series].field,
                        "continuity": pair.continuity.as_str(),
                        "why": pair.unobservable(),
                        "epoch": epoch,
                        "via": via,
                        "rate": rate.map(|r| r.per_s),
                        "rate_unit": rate.and_then(|r| r.unit.clone()),
                        "elapsed_ns": rate.map(|r| i64::try_from(r.elapsed_ns).unwrap()),
                        "time_basis": rate.and_then(|r| r.basis).map(|b| b.as_str()),
                        "jitter_ms": rate.and_then(|r| match r.basis {
                            Some(TimeBasis::Scheduled { jitter_ms }) => Some(jitter_ms),
                            _ => None,
                        }),
                        "no_rate": pair.rate.as_ref().err().map(|n| n.as_str()),
                    }));
                    assert_eq!(p.seen.presence, PairPresence::None, "{note}: in band");
                    if let Ok(r) = &pair.rate {
                        assert_eq!(p.seen.elapsed_ns, r.elapsed_ns, "{note}");
                        assert_eq!(p.seen.basis, r.basis, "{note}");
                    }
                }
            }
            let want: Vec<Value> = case["expect"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| {
                    let mut e = e.clone();
                    let o = e.as_object_mut().unwrap();
                    o.remove("earlier");
                    o.remove("later");
                    e
                })
                .collect();
            assert_eq!(got, want, "{note}");
            for s in series {
                let member = book
                    .series_member(&key, &s.field)
                    .expect("a member a series");
                assert_eq!(member.kind, Some((s.kind, Source::Declared)), "{note}");
                let fixture = want
                    .iter()
                    .filter(|e| e["field"] == s.field.as_str())
                    .count();
                assert_eq!(member.tally.total(), fixture as u64, "{note}: {}", s.field);
            }
        }
    }

    /// A frame its layout refuses, a mutation on a frame resource and a
    /// frame under an invalid `telemetry.series` are unobservable, never a
    /// sample (#983 T2; telemetry.v2 §2.16, §2.18): the first for every
    /// series of its key, which reads its records as lost at the next
    /// frame; the last as one member of the key, `declaration_invalid`.
    #[test]
    fn an_unreadable_frame_is_no_sample() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles");
        let (set, _) = crate::model::catalog::ContractSet::load_path(
            &root.join("telemetry/conformance/contracts/meterframe.v1.toml"),
        );
        let rev = set.iter().next().expect("meterframe.v1");
        let addr: Addr = "lab/meterframe".parse().unwrap();
        let i = rev
            .contract()
            .resources
            .iter()
            .position(|r| r.template.as_str() == "frames/link")
            .unwrap();
        let m = Arc::new(MetricResource::of(&addr, rev, i).unwrap());
        let key = "zk2/lab/meterframe/meterframe.v1/stream/frames/link".to_owned();
        let stamp = zenkey_model::wire::Stamp {
            run: zenkey_model::wire::Run::U16(1),
            seq: zenkey_model::wire::Seq::new(0).unwrap(),
        };
        let t0 = Instant::now();
        let sample = |payload: Vec<u8>, frame| MetricSample {
            key: key.clone(),
            delete: false,
            payload,
            attachment: None,
            stamp: None,
            at: t0,
            via: SampleVia::Subscription,
            frame,
        };
        let mut book = TelemetryBook::new(None, t0);
        let tag = m.frame.as_ref().unwrap().layout.tag();
        let seen = book.frame(&m, sample(vec![0xff], Some(tag)), Some(Ok(stamp)));
        assert_eq!(seen.unread, Some(tm::Unasked::Undecodable));
        let seen = book.frame(&m, sample(b"7".to_vec(), None), Some(Ok(stamp)));
        assert_eq!(seen.unread, Some(tm::Unasked::Undecodable), "a mutation");
        let dropped = book.series_member(&key, "dropped").unwrap();
        assert_eq!((dropped.samples, dropped.unobservable), (2, 2));
        assert_eq!(book.members().count(), 4, "one member a series");
        assert_eq!(
            book.member(&key).map(|m| m.samples),
            None,
            "no v1 member of a frame key"
        );

        // An invalid telemetry.series: one member, every frame unobservable.
        let mut c = rev.contract().clone();
        let r = c
            .resources
            .iter_mut()
            .find(|r| r.template.as_str() == "frames/link")
            .unwrap();
        r.annotations
            .insert(tm::SERIES.into(), json!({"nope": {"kind": "counter"}}));
        let r = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == "frames/link")
            .unwrap();
        let f = FrameMetric::of(&c, r).expect("a frame resource with series");
        assert!(f.declared().is_none());
        let mut bad = (*m).clone();
        bad.frame = Some(Arc::new(f));
        let bad = Arc::new(bad);
        assert!(!bad.declared());
        assert!(bad.invalid_words().unwrap().contains("series_field"));
        let mut book = TelemetryBook::new(None, t0);
        let seen = book.frame(&bad, sample(vec![0], Some(tag)), Some(Ok(stamp)));
        assert_eq!(seen.unread, Some(tm::Unasked::DeclarationInvalid));
        let (_, only) = book.members().next().unwrap();
        assert!(only.field.is_none());
        assert_eq!(only.unobservable, 1);
    }
}
