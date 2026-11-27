//! Consuming through a role (spec §3.2): data reaches a component from the
//! providers its configuration binds the role to, never from a provider
//! named in code.
//!
//! - **R1:** a binding is a list of service addresses, exact
//!   (`vehicle-01/teleop`) or wildcard (`vehicle-01/*`, `*/tc`). It resolves
//!   at once: a subscription needs no presence. A service's
//!   `self.system/<service>` is spelled out when it starts (0.20); a tool,
//!   with no system of its own, is refused one.
//! - **R2:** a template parameter may be bound, to `self.system`,
//!   `self.service` or a value. Unbound parameters select every member.
//! - **R5:** a consumer MAY wait on presence for its providers. A provider
//!   is present by its interface token, or, for an interface in its
//!   tokenless set, by an instance token whose descriptor marks the
//!   interface `"token": false` (§8.1, U22); it is absent only by a
//!   complete read (observer §3.6), and a read possibly incomplete leaves
//!   it unobservable (#1015).
//! - **R6:** a sample whose key expression is not concrete is discarded and
//!   counted: zenoh delivers a put on a wildcard key with the publisher's key.
//! - **R7:** where presence cannot be observed (a constrained face),
//!   liveness is judged from the freshness of what crosses, and silence is
//!   *unobservable*, never *down*.
//!
//! **`timing.v1`** (#908): every [`Delivery`] carries its arrival, taken in
//! the subscriber's callback, its stamp and its lifespan's judgement. A
//! subscription to a timed stream judges each provider's members' deadline
//! and each sample's lifespan, as [`crate::timing`] says, and hands its
//! events to [`Consumer::subscribe_timed`]'s second callback; a sample that
//! arrives expired is discarded, counted ([`Subscription::expired`]), and
//! never delivered.
//!
//! **The zk2c envelope** (core §4.3, §7.5): every data sample is read
//! through [`crate::inband`], so a stream, state or event sample that
//! carries an envelope is delivered with its record as its payload and its
//! run and seq as [`Delivery::inband`], what orders it
//! ([`Delivery::mark`]). One whose envelope is refused is undecodable:
//! never delivered, and counted ([`Subscription::undecodable`],
//! [`StateAnswer::undecodable`]). So is a record of a packed type (core
//! §7.6) its layout refuses under the tag its CH carried
//! ([`crate::inband::admit`]): another revision's, by its tag or its
//! length. A tool that shows samples is handed each one a subscription
//! refuses, with its refusal ([`Consumer::with_refused`]), so it never
//! drops one silently (observer §7.3). A sample with no envelope, an `@stream` sample's or a foreign
//! key's, is delivered as it arrived, ordered by its Zenoh timestamp where
//! it carries one.
//!
//! **Heartbeats** (core §7.5's form 6, #978's F2): a subscription to a
//! collection, a state with a horizon above 0 whose template it binds no
//! parameter of, also hears each bound provider's heartbeat covering the
//! interface (`zenkey_model::heartbeat::key`), and folds it into a
//! `zenkey_model::freshness::View` per provider, from the members it
//! delivered. A heartbeat whose section's digest is the view's confirms
//! the members it vouches for, so [`Subscription::freshness`] ages a
//! member from its last confirmation, a put or a heartbeat
//! ([`Consumer::subscribe_confirmed`] hands each one over).
//! `freshness.v2` adopts it (#978's F3): an owner of a contract that uses
//! it sends a heartbeat per interface, and a reader's rule is v2 §2.5's.
//! After two heartbeats in a row whose digest is not the view's, the
//! subscription re-reads the provider's collection by S4's GET on its own
//! ([`Subscription::rereads`]), and the answer replaces the view, merged
//! with what it heard since it asked; a late joiner seeds the views from
//! its own GET ([`Subscription::seed`]). A re-read delivers nothing: it
//! makes the view whole, so the next heartbeat confirms.
//!
//! **Frames** (core §7.7, #981's K2): a subscription to a frame resource
//! reads each put as a frame, its body parsed with the resource's layout
//! ([`crate::inband::admit_frame`]), and keeps one
//! `zenkey_model::frame::Reader` per key, so [`Delivery::frame`] carries
//! each record's seq, time and field states: `known`, `within` its delta,
//! `absent`, or `unknown` at a gap, a rewind, a run it does not hold and
//! past a deadband, never a value nor 0 (core §7.7's MUST NOT). A body its
//! layout refuses, a mutation on a frame resource and a frame on any other
//! are undecodable, refused as any other sample.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Body, Contract, Resource};
use zenkey_model::freshness::{
    Applied, ClockMeasure, ClockTrust, Horizon, Judged, Last, Observation, SectionRead, StampAge,
    View, Views,
};
use zenkey_model::grammar::{Addr, IfaceId, KindToken, ZkKey, parse};
use zenkey_model::packed::Layout;
use zenkey_model::seed::Mark;
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::{Bindings, Segment};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::pubsub::Subscriber;
use zenoh::sample::Sample;
use zenoh::time::Timestamp;

use crate::error::{Error, Result, zenoh};
use crate::state::{ConditionalAnswer, Current, StateAnswer, StateGet};
use crate::timing::{self, LifespanSample, MemberState, OwnerState, TimingEvent};

/// One bound provider address: a position is `None` for `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    system: Option<String>,
    service: Option<String>,
}

impl Provider {
    /// Parses `<system>/<service>`, either position `*` (R1).
    pub fn parse(s: &str) -> Result<Self> {
        let bad = || {
            Error::Contract(format!(
                "binding {s:?} is not <system>/<service> (each a name or *)"
            ))
        };
        let (sys, svc) = s.split_once('/').ok_or_else(bad)?;
        if sys == crate::config::SELF_SYSTEM {
            // R1 (0.20): a service's runtime spells it out at start, so only
            // a tool, which has no system of its own, gets here.
            return Err(Error::Contract(format!(
                "binding {s:?} names the service's own system, which a tool has none of (R1)"
            )));
        }
        let pos = |p: &str| -> Result<Option<String>> {
            if p == "*" {
                Ok(None)
            } else if zenkey_model::chunk::is_plain_chunk(p) {
                Ok(Some(p.to_owned()))
            } else {
                Err(bad())
            }
        };
        Ok(Self {
            system: pos(sys)?,
            service: pos(svc)?,
        })
    }

    /// Whether `addr` is one of the services this binding names.
    #[must_use]
    pub fn matches(&self, addr: &Addr) -> bool {
        self.system
            .as_deref()
            .is_none_or(|s| s == addr.system.as_str())
            && self
                .service
                .as_deref()
                .is_none_or(|s| s == addr.service.as_str())
    }

    pub(crate) fn chunks(&self) -> (&str, &str) {
        (
            self.system.as_deref().unwrap_or("*"),
            self.service.as_deref().unwrap_or("*"),
        )
    }

    /// Whether it names one address, neither position `*`.
    pub(crate) fn is_exact(&self) -> bool {
        self.system.is_some() && self.service.is_some()
    }

    /// The binding that names `addr` alone.
    pub(crate) fn exact(addr: &Addr) -> Self {
        Self {
            system: Some(addr.system.to_string()),
            service: Some(addr.service.to_string()),
        }
    }
}

/// Whether presence can be observed for a role's providers (R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Tokens reach this consumer.
    Observable,
    /// They do not: the providers sit across a constrained face whose `@zk`
    /// traffic is denied (§8.5).
    Unavailable,
}

/// A provider's liveness as a consumer can judge it (R7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Presence is observable, and the provider serves the interface: it
    /// holds the interface's token, or, for an interface in its tokenless
    /// set, an instance token whose descriptor lists the interface
    /// `"token": false` (§8.1, #1015).
    Present,
    /// Presence is observable, and a complete read shows the provider
    /// serving none of it: absence, as only a complete read establishes it
    /// (§8.1, observer §3.6).
    Absent,
    /// Presence is unavailable, and data arrived within the window.
    Fresh,
    /// Nothing can tell, so not *down*: presence is unavailable and nothing
    /// arrived within the window (R7), or the presence read was possibly
    /// incomplete (§8.1: it ended at its timeout or with another error
    /// reply, was cut off while it ran or ran to the session's interests
    /// timeout, or a descriptor it needed did not answer) and did not show
    /// the provider (observer §3.6, "an address unobservable, never down").
    Unobservable,
}

/// One delivered sample, attributed (R3's edge, observed).
#[derive(Debug, Clone)]
pub struct Delivery {
    /// The provider whose key carried it.
    pub provider: Addr,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The template's parameters, unslugged.
    pub values: Bindings,
    pub sample: Sample,
    /// When it arrived, on this host's monotonic clock, read in the
    /// subscriber's callback (`timing.v1` §2.5): a replay's, when its reply
    /// was read.
    pub arrival: Instant,
    /// Its Zenoh timestamp, if it carries one (core §4.1): its time, where
    /// a profile reads one.
    pub stamp: Option<Timestamp>,
    /// Its envelope's run and seq, when it carried one (core §7.5): what
    /// orders it, whatever Zenoh timestamp it also carries. `sample`'s
    /// payload is then the envelope's record.
    pub inband: Option<zenkey_model::wire::Stamp>,
    /// Its envelope's CH tag, when it carried one: a packed record's
    /// type's (core §7.6), which the consumer admitted it by; 0 otherwise.
    pub tag: Option<zenkey_model::wire::Tag>,
    /// Its lifespan's judgement at its arrival (`timing.v1` §2.6): live, or
    /// unobservable or not asked with why. An expired sample is never
    /// delivered.
    pub lifespan: LifespanSample,
    /// A frame's records, as the subscription's reader of its key read
    /// them (core §7.7): `None` for a sample of any other resource, and
    /// for a delete. `sample`'s payload is then the frame's body.
    pub frame: Option<FrameRead>,
    /// An event's time (core §7.5's form 4; #980's C2b), from the
    /// deployment epoch, known or unknown: `None` for every sample that is
    /// no event.
    pub occurred: Option<zenkey_model::collection::Time>,
}

/// A frame as a subscription's reader of its key read it (core §7.7,
/// "The reader's fields"; #981's K2): its layout, which names its fields,
/// each record's seq, time and field states, and the deployment epoch its
/// record type's `time` fields count from.
#[derive(Debug, Clone)]
pub struct FrameRead {
    pub layout: Arc<zenkey_model::frame::Layout>,
    pub applied: zenkey_model::frame::Applied,
    /// The consumer's deployment epoch ([`Consumer::with_deployment_epoch`],
    /// core §8.6), which a packed record's `time` fields read under too.
    pub epoch: i64,
}

impl FrameRead {
    /// The frame as `spec/conformance/frame/apply.json` spells a reading
    /// (`zenkey_model::frame::Applied::reading`), its `time` fields under
    /// [`FrameRead::epoch`].
    ///
    /// # Errors
    /// A `time` field's instant past an `i64` of milliseconds under it.
    pub fn reading(
        &self,
    ) -> std::result::Result<serde_json::Value, zenkey_model::frame::ReadRefusal> {
        self.applied.reading(&self.layout, Some(self.epoch))
    }

    /// Field `name`'s state in record `index`: `None` for no such record
    /// or field. An `unknown` or `absent` field holds no code.
    #[must_use]
    pub fn field(&self, index: usize, name: &str) -> Option<zenkey_model::frame::FieldState> {
        let i = self.layout.fields().iter().position(|f| f.name == name)?;
        self.applied.records.get(index)?.fields.get(i).copied()
    }
}

impl Delivery {
    /// What orders it among its key's values: its in-band stamp, else its
    /// Zenoh timestamp ([`crate::state::ValueOrder::accept_mark`]).
    #[must_use]
    pub fn mark(&self) -> Option<Mark> {
        Mark::of(
            self.inband,
            self.stamp.as_ref().map(crate::inband::stamp_of),
        )
    }
}

/// A sample a subscription or a replay refused as undecodable (core §7.5,
/// §7.6, §2.6): its envelope refused, a packed record its type's layout
/// refuses, or a sample on an events key that carries no event. Never
/// delivered nor ordered, and counted ([`Subscription::undecodable`]);
/// handed to [`Consumer::with_refused`]'s callback, for a tool that shows
/// it with its refusal named (observer §7.3), and for a replay's count.
#[derive(Debug, Clone)]
pub struct Refused {
    /// The provider whose key carried it.
    pub provider: Addr,
    /// `<kind token>/<template>`.
    pub resource: String,
    /// The template's parameters, unslugged.
    pub values: Bindings,
    /// The sample as [`crate::inband::read`] read it: an envelope's record
    /// with its run, seq and tag, a refused envelope's sample as it
    /// arrived, or a sample with no envelope.
    pub read: crate::inband::Read,
    /// Why it is undecodable.
    pub why: crate::inband::Undecodable,
    /// When it arrived, on this host's monotonic clock, read in the
    /// subscriber's callback, or as the replay read its reply.
    pub arrival: Instant,
}

/// What [`Consumer::with_refused`] hands each refused sample to.
type OnRefused = Arc<dyn Fn(Refused) + Send + Sync>;

/// What a gated read hands each GET's replies to, once the GET ended,
/// before it merges any (`redundancy.v2` §2.10: a follower's fence over its
/// seed, [`crate::follow`]). It returns those it admits, in their order.
pub(crate) trait ReplyGate: Send + Sync {
    fn admit<'a>(&'a self, replies: Vec<Sample>) -> crate::call::BoxFuture<'a, Vec<Sample>>;
}

/// One GET's replies, as a state read takes them: as they arrive, or once
/// a gate admitted them.
enum Replies {
    Stream(flume::Receiver<zenoh::query::Reply>),
    Held(std::vec::IntoIter<Sample>),
}

impl Replies {
    /// The next reply's sample; an error reply makes the read incomplete.
    async fn next(&mut self, complete: &mut bool) -> Option<Sample> {
        match self {
            Replies::Stream(rx) => loop {
                let reply = rx.recv_async().await.ok()?;
                match reply.into_result() {
                    Ok(sample) => return Some(sample),
                    Err(_) => *complete = false,
                }
            },
            Replies::Held(it) => it.next(),
        }
    }
}

/// A role's consumer, compiled against the required interface's contract
/// (R4: any revision of the major is fine).
#[derive(Clone)]
pub struct Consumer {
    session: zenoh::Session,
    role: String,
    contract: Arc<Contract>,
    providers: Vec<Provider>,
    params: BTreeMap<String, String>,
    presence: Presence,
    /// A watch session of the consumer's own (core §8.1, 0.37), where its
    /// timed subscriptions follow their owners' tokens (`timing.v1` §2.5).
    watch: Option<zenoh::Session>,
    /// ε, the deployment's stated bound on how far an owner's stamps and
    /// this host's wall clock disagree (`timing.v1` §2.6). `None`: nothing
    /// is discarded.
    epsilon: Option<Duration>,
    /// The contract declaring the role (core §3.3): what
    /// [`Consumer::arbitrate`] reads its requirement from. `None` for a
    /// role a component's manifest declares, and for a tool's consumer
    /// until [`Consumer::declared_by`].
    declaring: Option<Arc<Contract>>,
    /// Where a subscription hands each sample it refuses as undecodable
    /// ([`Consumer::with_refused`]). `None`: counted alone.
    refused: Option<OnRefused>,
    /// How long a re-read of a diverged collection waits for its replies
    /// (`freshness.v2` §2.5; core §8.1: timeouts are the caller's).
    reread_timeout: Duration,
    /// How long a read of an owner's descriptor waits for its replies, to
    /// learn the catalog it names (core §8.6, §8.1).
    catalog_timeout: Duration,
    /// The deployment epoch its typed reads count packed `time` fields
    /// from, in seconds (core §7.6, §8.6): its service's catalog's, else 0.
    epoch: i64,
    /// The face its providers sit across (core §8.1, §8.5;
    /// [`Consumer::across`]). `None`: no face.
    far: Option<zenkey_model::link::FarSide>,
    /// The deployment catalogs it holds (core §8.6): what it reads an
    /// owner heartbeat's indices through. None: every group it hears is
    /// refused `unknown_catalog`, and confirms nothing.
    catalogs: Arc<Vec<zenkey_model::catalog::Catalog>>,
}

/// How long a re-read waits unless [`Consumer::with_reread_timeout`] says
/// otherwise: a GET's timeout, the caller's choice (core §8.1).
pub const REREAD_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a read of an owner's descriptor, for the catalog it names,
/// waits unless [`Consumer::with_owner_catalog_timeout`] says otherwise: a
/// GET's timeout, the caller's choice (core §8.1).
pub const OWNER_CATALOG_TIMEOUT: Duration = Duration::from_secs(5);

/// A live subscription through a role. Dropping it undeclares it, and
/// stops its `timing.v1` engine.
pub struct Subscription {
    _subs: Vec<Subscriber<()>>,
    /// Each bound provider's heartbeat subscriber, for a collection.
    _heartbeats: Vec<Subscriber<()>>,
    /// Across a face whose medium repairs no lost frame, the subscribers
    /// declared again after silence (core §8.5), in place of the two
    /// above for those with a horizon.
    redeclaring: Option<crate::redeclare::Redeclaring>,
    /// The collection it follows by heartbeats, if any.
    followed: Option<Arc<Followed>>,
    seen: Arc<Seen>,
    /// When it was declared: how long it has listened (`freshness.v1` §2.5),
    /// and the origin of its deadlines (`timing.v1` §2.5).
    since: Instant,
    timing: Option<timing::Running>,
}

#[derive(Default)]
struct Seen {
    discarded: AtomicU64,
    unresolved: AtomicU64,
    undecodable: AtomicU64,
    last: Mutex<BTreeMap<Addr, Instant>>,
    /// Each member's last delivery, on this host's monotonic clock, and
    /// whether it was a delete (`freshness.v1` §2.5).
    members: Mutex<BTreeMap<String, (Instant, bool)>>,
    /// Per stamping clock, the delivery whose stamp came closest to this
    /// host's clock at receipt: what `freshness.v1` §2.6 measures a GET
    /// reader's clock from.
    clocks: Mutex<BTreeMap<String, ClockMeasure>>,
    /// For a collection, each provider's view as heartbeats confirm it
    /// (#978's F2), behind a sequence per owner heartbeat key (#1110).
    views: Mutex<BTreeMap<Addr, Views<Instant>>>,
    /// Heartbeats folded into a view, a group read for the interface
    /// among them, and those heard that could not be: another revision's
    /// tag, a body or a group refused, a sample on a heartbeat key that is
    /// no heartbeat.
    heartbeats: AtomicU64,
    heartbeats_unread: AtomicU64,
    /// Owner heartbeats read on a class key, whatever became of their
    /// groups (`freshness.v2` §2.5).
    owner_heartbeats: AtomicU64,
    /// The catalog each provider names, and the owner heartbeats waiting
    /// for one to be read.
    named: Mutex<Named>,
    /// Reads of its owners' descriptors made for that
    /// ([`crate::answer::owner_catalog`]).
    catalog_reads: AtomicU64,
    /// Owner heartbeats heard past [`OWNER_QUEUE`], their owner's catalog
    /// not known: each read in its turn with none.
    owner_unqueued: AtomicU64,
    /// Owner heartbeats dropped with [`OWNER_HELD`] already held.
    owner_dropped: AtomicU64,
    /// The groups read, waiting for the callback.
    outbox: Mutex<Outbox>,
    /// The providers whose collection a re-read is running for, and the
    /// re-reads that replaced a view (`freshness.v2` §2.5).
    rereading: Mutex<std::collections::BTreeSet<Addr>>,
    rereads: AtomicU64,
    last_reread: Mutex<Option<Instant>>,
    /// For a frame resource, each key's reader (core §7.7).
    frames: Mutex<BTreeMap<String, zenkey_model::frame::Reader>>,
}

/// What a heartbeat confirmed of a subscription's collection (#978's F2):
/// the provider, the members it vouched for whose digest was the view's,
/// in key order, and the heartbeat's stamp, arrival and time.
#[derive(Debug, Clone)]
pub struct Confirmation {
    pub provider: Addr,
    /// Base-relative, in key order; empty when its section confirmed none.
    pub keys: Vec<String>,
    /// The heartbeat key's run and seq.
    pub heartbeat: zenkey_model::wire::Stamp,
    /// Read in the subscriber's callback, on this host's monotonic clock.
    pub arrival: Instant,
    /// The owner's word of its clock when it wrote it, as the heartbeat
    /// carries it (core §7.5): an instant, the unknown time, or the ahead
    /// time while its clock guard held (`freshness.v2` §2.10). `None`
    /// where the view has read no time. It takes no part in what the
    /// heartbeat confirms (§2.5).
    pub owner_time: Option<zenkey_model::heartbeat::Time>,
}

/// One heartbeat a subscription's view read for its collection
/// (`freshness.v2` §2.5, §5; #990 V6): what its section did, and whether
/// a heartbeat was lost before it. Both are the reader's word: a
/// divergence says the reader's view and the owner's digest differ, no
/// finding about the owner (§5).
///
/// An owner heartbeat is read as one heartbeat per group (#1110): each
/// group read for the subscription's interface is one of these, `group`
/// naming it, `heartbeat` the class key's stamp and `lost` that key's gap.
#[derive(Debug, Clone)]
pub struct HeartbeatRead {
    pub provider: Addr,
    /// For a group of an owner heartbeat, the class its key names and the
    /// group's index in the owner's catalog; `None` for a heartbeat.
    pub group: Option<OwnerGroup>,
    /// The heartbeat key's run and seq: the interface's own key's, or the
    /// owner heartbeat key's.
    pub heartbeat: zenkey_model::wire::Stamp,
    /// Read in the subscriber's callback, on this host's monotonic clock.
    pub arrival: Instant,
    /// The owner's word of its clock when it wrote it, as the heartbeat
    /// carries it (core §7.5): an instant, the unknown time, or the ahead
    /// time while its clock guard held (`freshness.v2` §2.10). `None`
    /// where the view has read no time. It takes no part in what the
    /// heartbeat confirms (§2.5).
    pub owner_time: Option<zenkey_model::heartbeat::Time>,
    /// Its key's seqs skipped since the last heartbeat this view read:
    /// one or more were lost.
    pub lost: bool,
    /// What its section did to the collection.
    pub section: HeartbeatSection,
}

/// The group of an owner heartbeat a [`HeartbeatRead`] is (core §7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnerGroup {
    /// The class of the owner heartbeat key it rode.
    pub class: zenkey_model::heartbeat::Class,
    /// Its covered revision's index in the owner's catalog.
    pub index: u16,
}

/// How many owner heartbeats of one subscription may wait for an owner's
/// descriptor to be read (`freshness.v2` §2.5, "What it holds is
/// bounded"). Past it a put waits for no read: it is read in its turn,
/// behind those held, through the catalog known for its run when it
/// arrived, or with none, every group `unknown_catalog`, and counted
/// ([`Subscription::owner_heartbeats_unqueued`]). So it is never fresh
/// for want of a read, and never read ahead of the puts held on its key,
/// whose sequence it would move (#1146).
pub const OWNER_QUEUE: usize = 64;

/// How many owner heartbeats of one subscription may be held at all while
/// a descriptor's read runs: [`OWNER_QUEUE`] waiting for it, and past
/// them those decided without it, each holding its body until its turn.
/// One more is dropped, never read and counted
/// ([`Subscription::owner_heartbeats_dropped`]), so what a slow read
/// holds stays bounded whatever the put rate and the caller's timeout
/// (review-bot on #1169). Its key then reads a gap at its next put read:
/// the members it would have confirmed go stale, never fresh.
pub const OWNER_HELD: usize = 4 * OWNER_QUEUE;

/// How long a read of an owner's descriptor that failed stands for its
/// run before the next one is made: its owner heartbeats meanwhile read no
/// group (core §8.6, never a guess), one GET a period at most, never one
/// a put.
pub const OWNER_CATALOG_RETRY: Duration = Duration::from_secs(30);

/// What a read of one provider's descriptor said of the catalog it names
/// under a run.
struct Asked {
    run: zenkey_model::wire::Run,
    /// The catalog named; `None` where it names none, and where the read
    /// failed.
    named: Option<zenkey_model::catalog::CatalogId>,
    /// For a read that failed, when the next one may be made.
    retry: Option<Instant>,
}

/// The catalog each bound provider names in its descriptor, as last read
/// for a run, and how many owner heartbeats wait behind such a read.
#[derive(Default)]
struct Named {
    by: BTreeMap<Addr, Asked>,
    pending: usize,
}

impl Named {
    /// What `addr` names under `run`, where a read said: `Some(None)` for
    /// an owner that names no catalog, and for a read that failed until
    /// its retry is due at `now`. `None`: to be read.
    fn of(
        &self,
        addr: &Addr,
        run: zenkey_model::wire::Run,
        now: Instant,
    ) -> Option<Option<zenkey_model::catalog::CatalogId>> {
        let asked = self.by.get(addr).filter(|a| a.run == run)?;
        match asked.retry {
            Some(due) if now >= due => None,
            _ => Some(asked.named),
        }
    }
}

/// The reads of one subscription's owner heartbeats waiting to be handed
/// to its callback, in the order they were made, and whether one is being
/// handed over now: the callback runs under no lock of the subscription's,
/// one read at a time.
#[derive(Default)]
struct Outbox {
    queue: std::collections::VecDeque<HeartbeatRead>,
    delivering: bool,
}

/// What a heartbeat's section did to a followed collection
/// (`zenkey_model::freshness::SectionRead`, without `not_followed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeartbeatSection {
    /// Its digest was the view's: these members, vouched for, confirmed,
    /// base-relative in key order (none when it vouches for none).
    Confirmed(Vec<String>),
    /// Its digest or live count was not the view's: nothing confirmed.
    Diverged,
    /// Its map bit was 0: the collection not available, nothing confirmed.
    Absent,
}

/// A collection a subscription follows by heartbeats: the revision's
/// layout, and the collection's index in it.
struct Followed {
    layout: zenkey_model::heartbeat::Layout,
    index: usize,
    /// Whether it is a per-key resource (core §4.2): only then is a
    /// `reply_del` on its member an owner's answer (#977's E4).
    per_key: bool,
}

impl Followed {
    /// The collection's class: the owner heartbeat key that confirms it
    /// (`freshness.v2` §2.4).
    fn class(&self) -> zenkey_model::heartbeat::Class {
        self.layout.collections[self.index].class
    }

    /// One provider's view of the collection, in `views`, made when it is
    /// first heard of.
    fn view_in<'a>(
        &self,
        views: &'a mut BTreeMap<Addr, Views<Instant>>,
        addr: &Addr,
    ) -> &'a mut View<Instant> {
        let vs = views.entry(addr.clone()).or_default();
        if vs.view(&self.layout.iface).is_none() {
            let mut v = View::new(self.layout.clone());
            v.follow(self.index);
            vs.insert(v);
        }
        vs.view_mut(&self.layout.iface).expect("just inserted")
    }
}

impl Consumer {
    pub(crate) fn new(
        session: &zenoh::Session,
        me: Option<&Addr>,
        role: &str,
        contract: Arc<Contract>,
        providers: &[String],
        params: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let providers = providers
            .iter()
            .map(|p| Provider::parse(p))
            .collect::<Result<Vec<_>>>()?;
        let params = params
            .iter()
            .map(|(k, v)| {
                let v = match (v.as_str(), me) {
                    ("self.system", Some(me)) => me.system.to_string(),
                    ("self.service", Some(me)) => me.service.to_string(),
                    ("self.system" | "self.service", None) => {
                        return Err(Error::Contract(format!(
                            "parameter {k:?} = {v:?} needs a service of its own (R2); a tool has none"
                        )));
                    }
                    (other, _) => other.to_owned(),
                };
                Ok((k.clone(), v))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            session: session.clone(),
            role: role.to_owned(),
            contract,
            providers,
            params,
            presence: Presence::Observable,
            watch: None,
            epsilon: None,
            declaring: None,
            refused: None,
            reread_timeout: REREAD_TIMEOUT,
            catalog_timeout: OWNER_CATALOG_TIMEOUT,
            epoch: zenkey_model::catalog::EPOCH_WITHOUT_CATALOG,
            far: None,
            catalogs: Arc::default(),
        })
    }

    /// Reads across a constrained face (core §8.1, §8.5; `link.v2` §2.20,
    /// §2.21), as `far` states it:
    /// - **every GET waits at least the face's `reply_timeout_ms`**: S4's
    ///   GET and a seed ([`Consumer::get`], [`Consumer::get_answer`]), an
    ///   event's replay, a re-read of a diverged collection, and the read
    ///   of an owner's descriptor for the catalog it names, where it is
    ///   read ([`Consumer::with_owner_catalog_timeout`]). A reply later
    ///   than the query's timeout is dropped by the router (core Appendix
    ///   B);
    /// - **where the medium repairs no lost frame** (`arq = false`), each
    ///   subscription with a horizon is declared again after two horizons
    ///   of silence while the session's transport stayed up
    ///   ([`crate::redeclare`], [`Subscription::redeclared`]): a resource's
    ///   `freshness.ttl_s`, or a heartbeat's, the shortest of its
    ///   collections' ([`zenkey_model::link::redeclare_horizon`]).
    ///
    /// The session is the application's, opened with the face's transport
    /// ([`zenkey_model::link::FarSide::session_keys`]); presence across the
    /// face is [`Consumer::with_presence`]'s to say.
    #[must_use]
    pub fn across(mut self, far: &zenkey_model::link::FarSide) -> Self {
        self.reread_timeout = far.at_least(self.reread_timeout);
        self.catalog_timeout = far.at_least(self.catalog_timeout);
        self.far = Some(far.clone());
        self
    }

    /// The face [`Consumer::across`] named, if any.
    #[must_use]
    pub fn far_side(&self) -> Option<&zenkey_model::link::FarSide> {
        self.far.as_ref()
    }

    /// `timeout`, at least the face's reply timeout across a face (core
    /// §8.1).
    fn reply_wait(&self, timeout: Duration) -> Duration {
        self.far.as_ref().map_or(timeout, |f| f.at_least(timeout))
    }

    /// Where this consumer's subscriptions re-declare after silence: a face
    /// whose medium repairs no lost frame.
    async fn redeclaring(&self) -> Result<Option<crate::redeclare::Redeclaring>> {
        match &self.far {
            Some(f) if f.redeclares() => Ok(Some(
                crate::redeclare::Redeclaring::start(&self.session).await?,
            )),
            _ => Ok(None),
        }
    }

    /// How long a subscription's re-read of a diverged collection waits
    /// for its replies (`freshness.v2` §2.5): across a slow face, at least
    /// its round trip. [`REREAD_TIMEOUT`] unless set, and at least the
    /// face's reply timeout across one ([`Consumer::across`]).
    #[must_use]
    pub fn with_reread_timeout(mut self, timeout: Duration) -> Self {
        self.reread_timeout = self.reply_wait(timeout);
        self
    }

    /// How long a subscription's read of an owner's descriptor waits for
    /// its replies, to learn the catalog the owner names under a run (core
    /// §8.6; `freshness.v2` §2.5): across a slow hop, at least the
    /// descriptor's round trip, which took 5.7 s over RF (#1146).
    /// [`OWNER_CATALOG_TIMEOUT`] unless set, and at least the face's reply
    /// timeout across one ([`Consumer::across`]). The owner heartbeats
    /// heard meanwhile wait for it, at most [`OWNER_QUEUE`] of them, and
    /// a read that fails names no catalog for [`OWNER_CATALOG_RETRY`].
    #[must_use]
    pub fn with_owner_catalog_timeout(mut self, timeout: Duration) -> Self {
        self.catalog_timeout = self.reply_wait(timeout);
        self
    }

    /// Reads packed records' `time` fields under `epoch`, the deployment
    /// epoch in seconds (core §7.6, §8.6). A service's consumer reads under
    /// its catalog's ([`crate::ServiceBuilder::catalog`]), the deployment's
    /// word for every provider it binds.
    #[must_use]
    pub fn with_deployment_epoch(mut self, epoch: i64) -> Self {
        self.epoch = epoch;
        self
    }

    /// Reads under `catalog`'s epoch ([`Consumer::with_deployment_epoch`]),
    /// and an owner heartbeat's indices through it
    /// ([`Consumer::with_catalogs`]): a tool's, given the deployment's
    /// catalog.
    #[must_use]
    pub fn with_catalog(self, catalog: &zenkey_model::catalog::Catalog) -> Self {
        self.with_deployment_epoch(catalog.epoch())
            .with_catalogs([catalog.clone()])
    }

    /// Holds `catalogs` (core §8.6), in place of any held: an owner
    /// heartbeat's group is read through the one its owner's descriptor
    /// names among them, or a successor of it (core §7.5; `freshness.v2`
    /// §2.5). Where no descriptor reaches this consumer, as across a
    /// constrained face, the last one given is the deployment's word; with
    /// presence unavailable ([`Consumer::with_presence`]) no descriptor is
    /// read, and it is the word at once. A
    /// subscription made outside a tokio runtime reads no descriptor, so
    /// it reads no group: an owner that names no catalog writes no index,
    /// and its own catalog would be a guess (core §8.6). With none given,
    /// a subscription declares no owner heartbeat key at all: it could
    /// read no group. The epoch stays
    /// [`Consumer::with_deployment_epoch`]'s.
    #[must_use]
    pub fn with_catalogs(
        mut self,
        catalogs: impl IntoIterator<Item = zenkey_model::catalog::Catalog>,
    ) -> Self {
        self.catalogs = Arc::new(catalogs.into_iter().collect());
        self
    }

    /// The catalogs it reads an owner heartbeat's indices through.
    #[must_use]
    pub fn catalogs(&self) -> &[zenkey_model::catalog::Catalog] {
        &self.catalogs
    }

    /// The deployment epoch its typed reads count `time` fields from.
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.epoch
    }

    /// A consumer for a tool, which has no service of its own: `contract`
    /// is typically built from a retrieved bundle
    /// ([`crate::Implementation::from_bundle`]), `providers` are service
    /// addresses, exact or wildcard (R1), and `params` binds template
    /// parameters to values (R2; `self.*` needs a service and is refused).
    /// It appears in no graph: a component that should is a service.
    pub fn for_tool(
        session: &zenoh::Session,
        contract: Arc<Contract>,
        providers: &[&str],
        params: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let providers: Vec<String> = providers.iter().map(|p| (*p).to_owned()).collect();
        Self::new(session, None, "tool", contract, &providers, params)
    }

    /// Marks presence unobservable for this role's providers (R7): set by a
    /// deployment whose providers sit across a constrained face. Its
    /// subscriptions then read no descriptor for an owner heartbeat's
    /// catalog, since none crosses (core §8.5, §8.6): each group is read
    /// at once through the last catalog given, the deployment's word
    /// ([`Consumer::with_catalogs`]; `freshness.v2` §2.5).
    #[must_use]
    pub fn with_presence(mut self, presence: Presence) -> Self {
        self.presence = presence;
        self
    }

    /// Follows its timed subscriptions' owners on `watch`, a session of the
    /// consumer's own that holds their liveliness subscribers (`timing.v1`
    /// §2.5, core §8.1, 0.37): a known member is then waited for from its
    /// owner's presence, and a member ends when its member tokens are gone.
    /// The consumer's own session goes on making every read. Without one, a
    /// member is judged from its first arrival, and ends only forgotten.
    #[must_use]
    pub fn with_watch(mut self, watch: zenoh::Session) -> Self {
        self.watch = Some(watch);
        self
    }

    /// ε (`timing.v1` §2.6): the deployment's stated bound on how far an
    /// owner's stamps and this host's wall clock disagree, under which a
    /// sample older than its lifespan is discarded. A service's consumer
    /// takes it from its binding's `clock_bound_ms`; `None` states none, and
    /// nothing is discarded. Never the HLC's delta.
    #[must_use]
    pub fn with_clock_bound(mut self, epsilon: Option<Duration>) -> Self {
        self.epsilon = epsilon;
        self
    }

    /// Names the contract declaring the role and the role's name, for a
    /// tool's consumer ([`Consumer::for_tool`]), whose role a contract it
    /// holds declares: what [`Consumer::arbitrate`] reads the role's
    /// requirement from. A service's consumer has its own already.
    #[must_use]
    pub fn declared_by(mut self, declaring: Arc<Contract>, role: &str) -> Self {
        self.declaring = Some(declaring);
        role.clone_into(&mut self.role);
        self
    }

    /// Hands `on_refused` each sample this consumer's subscriptions refuse
    /// as undecodable ([`Refused`]), beside its count
    /// ([`Subscription::undecodable`]), from the subscriber's callback, and
    /// each occurrence [`Consumer::replay`] skips as undecodable, from the
    /// replay's task: a replay's count (core §2.6, "It counts it
    /// undecodable"). A tool that shows samples shows such a one, its
    /// refusal named, and never drops it silently (observer §7.3). Without
    /// it a subscription's sample is counted alone, and a replay's logged.
    #[must_use]
    pub fn with_refused(mut self, on_refused: impl Fn(Refused) + Send + Sync + 'static) -> Self {
        self.refused = Some(Arc::new(on_refused));
        self
    }

    pub(crate) fn with_declaring(mut self, declaring: Option<Arc<Contract>>) -> Self {
        self.declaring = declaring;
        self
    }

    /// Follows one provider at a time (`arbitration.v1`, #909,
    /// [`crate::arbitration`]): the role's declaration read from the
    /// contract declaring it and this consumer's, the revision held (§2.2,
    /// §2.3), refused unless arbitrated; its one stream subscribed through
    /// every bound provider, ranked in the binding's order (§2.4); its
    /// withdrawals heard on [`Consumer::with_watch`]'s session, and read
    /// again on this one (§2.7), unless presence is unavailable (R7,
    /// §2.12), when the deadline alone judges; each sample's lifespan on
    /// [`Consumer::with_clock_bound`]'s ε (§2.5). `on_event` is handed, in
    /// decision order and from a task of its own, the `start` switch to
    /// the dead-man, each later switch, and each arrival of the active
    /// provider, nothing else. It returns once the watch's round trip
    /// stood, or ran to its timeout.
    pub async fn arbitrate<C>(&self, on_event: C) -> Result<crate::arbitration::Arbitration>
    where
        C: Fn(crate::arbitration::ArbitrationEvent) + Send + Sync + 'static,
    {
        let watch = match self.presence {
            Presence::Observable => self.watch.clone(),
            Presence::Unavailable => None,
        };
        let mut inputs = crate::arbitration::Inputs {
            session: &self.session,
            watch,
            role: &self.role,
            declaring: self.declaring.as_deref(),
            required: &self.contract,
            providers: &self.providers,
            params: &self.params,
            epsilon: self.epsilon,
            selectors: Vec::new(),
        };
        let declared = crate::arbitration::arbitrated(&inputs)?;
        inputs.selectors = self.selectors(&declared.resource())?;
        crate::arbitration::start(inputs, on_event).await
    }

    /// The ε it judges lifespans under, if one is stated.
    #[must_use]
    pub fn clock_bound(&self) -> Option<Duration> {
        self.epsilon
    }

    #[must_use]
    pub fn role(&self) -> &str {
        &self.role
    }

    #[must_use]
    pub fn interface(&self) -> &IfaceId {
        &self.contract.iface
    }

    /// The bound providers (R1).
    #[must_use]
    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    /// The session it reads through: zenoh stays reachable.
    #[must_use]
    pub fn session(&self) -> &zenoh::Session {
        &self.session
    }

    /// The role's parameter bindings (R2), resolved.
    #[must_use]
    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    /// The revision of the required interface it is compiled against (R4).
    #[must_use]
    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    /// Its watch session, if it holds one ([`Consumer::with_watch`]).
    pub(crate) fn watch_session(&self) -> Option<&zenoh::Session> {
        self.watch.as_ref()
    }

    /// The same consumer, bound to `addr` alone: a follower's seed of one
    /// provider (`redundancy.v2` §2.10).
    pub(crate) fn bound_to(&self, addr: &Addr) -> Self {
        let mut c = self.clone();
        c.providers = vec![Provider::exact(addr)];
        c
    }

    /// The lifespan a delivery not judged by a timed subscription carries:
    /// a seed's value (`timing.v1` §2.6's premise for `r`).
    pub(crate) fn premise(&self, r: &Resource) -> LifespanSample {
        timing::Plan::new(
            r,
            &self.contract.iface,
            &self.providers,
            &self.params,
            self.epsilon,
        )
        .premise()
    }

    pub(crate) fn resource(&self, name: &str) -> Result<&Resource> {
        name.split_once('/')
            .and_then(|(t, tpl)| self.contract.resource(t.parse().ok()?, tpl))
            .ok_or_else(|| Error::NoResource {
                iface: self.contract.iface.clone(),
                resource: name.to_owned(),
            })
    }

    /// The key expressions a subscription on `resource` uses: one per
    /// bound provider, with R2's parameter bindings applied and every other
    /// parameter a wildcard. An event's selector ends in `*`, its
    /// occurrence chunk. A
    /// state GET's are [`Consumer::get_selectors`].
    pub fn selectors(&self, resource: &str) -> Result<Vec<OwnedKeyExpr>> {
        let r = self.resource(resource)?;
        let iface = self.contract.iface.to_string();
        let mut tail: Vec<String> = r
            .template
            .segments()
            .iter()
            .map(|seg| match seg {
                Segment::Literal(l) => l.clone(),
                Segment::Param(n) => self
                    .params
                    .get(n)
                    .map_or_else(|| "*".to_owned(), |v| chunk_slug(v)),
                Segment::Rest(n) => self
                    .params
                    .get(n)
                    .map_or_else(|| "**".to_owned(), |v| chunk_slug(v)),
            })
            .collect();
        if r.kind == Kind::Event {
            tail.push("*".to_owned());
        }
        self.providers
            .iter()
            .map(|p| {
                let (sys, svc) = p.chunks();
                let ke = format!("zk2/{sys}/{svc}/{iface}/{}/{}", r.token, tail.join("/"));
                OwnedKeyExpr::try_from(ke).map_err(zenoh)
            })
            .collect()
    }

    /// Subscribes to `resource` across the bound providers. It resolves at
    /// once, without presence (R1, R5). Samples on a non-concrete key are
    /// discarded and counted (R6). A sample on a concrete key that does not
    /// resolve to a member of the resource through a bound provider is
    /// dropped and counted apart ([`Subscription::unresolved`]); every other
    /// one is delivered attributed, but one that arrives expired
    /// (`timing.v1` §2.6), which is counted ([`Subscription::expired`]).
    ///
    /// [`Consumer::subscribe_timed`], its timing events dropped.
    pub async fn subscribe<C>(&self, resource: &str, callback: C) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
    {
        self.subscribe_timed(resource, callback, |_| {}).await
    }

    /// [`Consumer::subscribe`], handing `on_timing` what the subscription
    /// judges of a timed stream (`timing.v1`, [`crate::timing`]): each
    /// provider's members missed, met, ended, or past its bound (§2.5), and
    /// each sample discarded as expired (§2.6), in the order judged, from a
    /// task of its own. A resource that is not a timed stream has none.
    ///
    /// It returns once its first reads stand: the providers' descriptors
    /// (with a lifespan) and their presence after the watch session's round
    /// trip (with a deadline), each provider's at once. On a session cut
    /// off from every router that is about one [`timing::READ_TIMEOUT`] per
    /// kind of read, beside the round trip's own wait. Samples are
    /// delivered from the data subscribers' declaration on.
    pub async fn subscribe_timed<C, T>(
        &self,
        resource: &str,
        callback: C,
        on_timing: T,
    ) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
        T: Fn(TimingEvent) + Send + Sync + 'static,
    {
        self.subscribe_inner(resource, callback, on_timing, |_| {})
            .await
    }

    /// [`Consumer::subscribe`], handing `on_confirm` what each heartbeat
    /// confirmed of the collection subscribed (#978's F2): a resource that
    /// is no collection, or one whose template parameters the consumer
    /// binds, hears no heartbeat.
    pub async fn subscribe_confirmed<C, F>(
        &self,
        resource: &str,
        callback: C,
        on_confirm: F,
    ) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
        F: Fn(Confirmation) + Send + Sync + 'static,
    {
        self.subscribe_inner(
            resource,
            callback,
            |_| {},
            move |h: HeartbeatRead| {
                if let HeartbeatSection::Confirmed(keys) = h.section {
                    on_confirm(Confirmation {
                        provider: h.provider,
                        keys,
                        heartbeat: h.heartbeat,
                        arrival: h.arrival,
                        owner_time: h.owner_time,
                    });
                }
            },
        )
        .await
    }

    /// [`Consumer::subscribe`], handing `on_heartbeat` every heartbeat its
    /// view read for the collection subscribed (#990 V6): confirmed,
    /// diverged or absent, and whether one was lost before it
    /// ([`HeartbeatRead`]). [`Consumer::subscribe_confirmed`] hands over
    /// only the confirmations.
    pub async fn subscribe_heartbeats<C, F>(
        &self,
        resource: &str,
        callback: C,
        on_heartbeat: F,
    ) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
        F: Fn(HeartbeatRead) + Send + Sync + 'static,
    {
        self.subscribe_inner(resource, callback, |_| {}, on_heartbeat)
            .await
    }

    /// The collection `r` is, followed by heartbeats: a state with a
    /// horizon above 0 (core §7.5's collections) none of whose template
    /// parameters this consumer binds, so it hears every member of each
    /// bound provider.
    fn followed(&self, r: &Resource) -> Option<Followed> {
        let bound = r
            .template
            .params()
            .any(|(name, _)| self.params.contains_key(name));
        if bound {
            return None;
        }
        let layout = zenkey_model::heartbeat::Layout::of(&self.contract);
        let index = layout.index_of(&format!("{}/{}", r.token, r.template))?;
        Some(Followed {
            layout,
            index,
            per_key: zenkey_model::collection::per_key(&self.contract, r),
        })
    }

    async fn subscribe_inner<C, T, F>(
        &self,
        resource: &str,
        callback: C,
        on_timing: T,
        on_confirm: F,
    ) -> Result<Subscription>
    where
        C: Fn(Delivery) + Send + Sync + 'static,
        T: Fn(TimingEvent) + Send + Sync + 'static,
        F: Fn(HeartbeatRead) + Send + Sync + 'static,
    {
        let r = self.resource(resource)?.clone();
        let name = format!("{}/{}", r.token, r.template);
        let layout = self.packed_layout(&r)?;
        let frame_layout = self.frame_layout(&r)?;
        // A frame's `time` fields count from the deployment epoch, as a
        // packed record's do (core §7.6, §8.6).
        let epoch = self.epoch;
        let seen = Arc::new(Seen::default());
        let plan = timing::Plan::new(
            &r,
            &self.contract.iface,
            &self.providers,
            &self.params,
            self.epsilon,
        );
        let premise = plan.premise();
        let since = Instant::now();
        let prepared = timing::prepare(plan, since);
        let engine = prepared.as_ref().map(|p| Arc::clone(&p.engine));
        let callback = Arc::new(callback);
        let followed = self.followed(&r).map(Arc::new);
        // Core §8.5: across a face whose medium repairs no lost frame, a
        // subscription with a horizon is declared again after silence.
        let redeclaring = self.redeclaring().await?;
        let horizon = redeclaring
            .as_ref()
            .and(zenkey_model::link::redeclare_horizon(&self.contract, &r));
        let heartbeats = match &followed {
            Some(f) => {
                let on_confirm = Arc::new(on_confirm);
                let mut subs = self
                    .heartbeat_subscribers(f, &seen, &on_confirm, redeclaring.as_ref())
                    .await?;
                subs.extend(
                    self.owner_heartbeat_subscribers(f, &seen, &on_confirm, redeclaring.as_ref())
                        .await?,
                );
                subs
            }
            None => Vec::new(),
        };
        let mut subs = Vec::new();
        for ke in self.selectors(resource)? {
            let (seen, callback, r, name, engine, layout, followed, frame_layout) = (
                Arc::clone(&seen),
                Arc::clone(&callback),
                r.clone(),
                name.clone(),
                engine.clone(),
                layout.clone(),
                followed.clone(),
                frame_layout.clone(),
            );
            let iface = self.contract.iface.clone();
            let providers = self.providers.clone();
            let on_refused = self.refused.clone();
            let on_sample: crate::redeclare::Callback = Arc::new(move |sample: Sample| {
                // timing.v1 §2.5, §2.6: its arrival, on both clocks,
                // read here, before anything else.
                let arrival = Instant::now();
                let wall = SystemTime::now();
                let key = sample.key_expr().as_str();
                if key.contains('*') {
                    seen.discarded.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                let Ok(ZkKey::Data {
                    addr,
                    iface: got,
                    resource: mut chunks,
                    ..
                }) = parse(key)
                else {
                    seen.unresolved.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                if got != iface || !providers.iter().any(|p| p.matches(&addr)) {
                    seen.unresolved.fetch_add(1, Ordering::Relaxed);
                    return;
                }
                if r.kind == Kind::Event {
                    chunks.pop();
                }
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                let Some(values) = r.template.matches(&refs) else {
                    seen.unresolved.fetch_add(1, Ordering::Relaxed);
                    return;
                };
                // Core §7.5, §7.6: a refused envelope, or a packed
                // record its layout refuses, is undecodable, never
                // delivered, never an arrival; so is a sample on an
                // events key that carries no event, a delete among them
                // (§2.6).
                let occurred = crate::inband::event_time(&sample);
                let no_event = r.kind == Kind::Event && occurred.is_none();
                let read = crate::inband::read(sample);
                let (inband, tag) = (read.stamp(), read.tag());
                // Core §7.7: a frame resource's put is a frame, read
                // with its layout by the key's reader.
                let admitted = match &frame_layout {
                    Some(fl) => crate::inband::admit_frame(&read, fl).map(|f| {
                        f.zip(inband).map(|(frame, stamp)| {
                            let key = read.sample().key_expr().as_str().to_owned();
                            let applied = seen
                                .frames
                                .lock()
                                .expect("not poisoned")
                                .entry(key)
                                .or_default()
                                .apply(fl, stamp, &frame);
                            FrameRead {
                                layout: Arc::clone(fl),
                                applied,
                                epoch,
                            }
                        })
                    }),
                    None if r.kind == Kind::Event => {
                        crate::inband::admit_event(&read, layout.as_deref(), no_event)
                            .map(|()| None)
                    }
                    None => crate::inband::admit(&read, layout.as_deref()).map(|()| None),
                };
                let frame = match admitted {
                    Ok(frame) => frame,
                    Err(why) => {
                        seen.undecodable.fetch_add(1, Ordering::Relaxed);
                        // A frame its layout refused is never applied:
                        // its key's reader notes it, so the next frame
                        // follows a gap that counts and names it.
                        if let (Some(_), crate::inband::Read::Framed { stamp, .. }) =
                            (&frame_layout, &read)
                        {
                            let key = read.sample().key_expr().as_str().to_owned();
                            seen.frames
                                .lock()
                                .expect("not poisoned")
                                .entry(key)
                                .or_default()
                                .refuse(*stamp);
                        }
                        tracing::debug!(
                            key = %read.sample().key_expr(),
                            reason = %why.reason(),
                            "an undecodable sample"
                        );
                        if let Some(f) = &on_refused {
                            f(Refused {
                                provider: addr,
                                resource: name.clone(),
                                values,
                                read,
                                why,
                                arrival,
                            });
                        }
                        return;
                    }
                };
                let sample = read.into_sample();
                let key = sample.key_expr().as_str();
                let lifespan = match &engine {
                    Some(e) => {
                        match timing::lock(e).arrive(&addr, key, &chunks, &sample, arrival, wall) {
                            Some(l) => l,
                            // Expired: discarded, counted, no arrival.
                            None => return,
                        }
                    }
                    None => premise,
                };
                let now = arrival;
                seen.last
                    .lock()
                    .expect("not poisoned")
                    .insert(addr.clone(), now);
                let deleted = sample.kind() == zenoh::sample::SampleKind::Delete;
                seen.members
                    .lock()
                    .expect("not poisoned")
                    .insert(key.to_owned(), (now, deleted));
                if let Some(f) = &followed {
                    let mut views = seen.views.lock().expect("not poisoned");
                    let view = f.view_in(&mut views, &addr);
                    if deleted {
                        view.delete(key, inband, now);
                    } else {
                        view.put(key, inband, now);
                    }
                }
                if let Some(t) = sample.timestamp() {
                    let offset =
                        StampAge::between(t.get_time().to_system_time(), SystemTime::now());
                    seen.clocks
                        .lock()
                        .expect("not poisoned")
                        .entry(t.get_id().to_string())
                        .and_modify(|m| m.record(offset))
                        .or_insert(ClockMeasure::new(offset));
                }
                let stamp = sample.timestamp().copied();
                callback(Delivery {
                    provider: addr,
                    resource: name.clone(),
                    values,
                    sample,
                    arrival,
                    stamp,
                    inband,
                    tag,
                    lifespan,
                    frame,
                    occurred,
                });
            });
            match (&redeclaring, horizon) {
                (Some(rd), Some(h)) => {
                    rd.declare(ke, h, on_sample).await?;
                }
                _ => subs.push(
                    self.session
                        .declare_subscriber(ke)
                        .callback(move |sample: Sample| on_sample(sample))
                        .await
                        .map_err(zenoh)?,
                ),
            }
        }
        let timing = match prepared {
            Some(p) => Some(
                p.run(
                    timing::Sources {
                        session: self.session.clone(),
                        watch: self.watch.clone(),
                        providers: self.providers.clone(),
                    },
                    on_timing,
                )
                .await,
            ),
            None => None,
        };
        Ok(Subscription {
            _subs: subs,
            _heartbeats: heartbeats,
            redeclaring,
            followed,
            seen,
            since,
            timing,
        })
    }

    /// One heartbeat subscriber per bound provider, on the key of its
    /// heartbeat covering this interface: each heartbeat folded into the
    /// provider's view, what it did to the collection handed to
    /// `on_confirm`.
    ///
    /// Through `redeclaring` across a face (core §8.5), each is declared
    /// again after two of the heartbeat's horizons of silence, where the
    /// contract uses `freshness.v2`, and is not among those returned.
    async fn heartbeat_subscribers<F>(
        &self,
        f: &Arc<Followed>,
        seen: &Arc<Seen>,
        on_confirm: &Arc<F>,
        redeclaring: Option<&crate::redeclare::Redeclaring>,
    ) -> Result<Vec<Subscriber<()>>>
    where
        F: Fn(HeartbeatRead) + Send + Sync + 'static,
    {
        let horizon = redeclaring.and(zenkey_model::link::redeclare_heartbeat_horizon(
            &self.contract,
        ));
        let rt = tokio::runtime::Handle::try_current().ok();
        let mut out = Vec::new();
        for p in &self.providers {
            let (sys, svc) = p.chunks();
            let ke = OwnedKeyExpr::try_from(zenkey_model::heartbeat::key(
                sys,
                svc,
                &self.contract.iface,
            ))
            .map_err(zenoh)?;
            let (f, seen, on_confirm, providers) = (
                Arc::clone(f),
                Arc::clone(seen),
                Arc::clone(on_confirm),
                self.providers.clone(),
            );
            let iface = self.contract.iface.clone();
            let (session, timeout, rt) = (self.session.clone(), self.reread_timeout, rt.clone());
            let on_sample: crate::redeclare::Callback = Arc::new(move |sample: Sample| {
                let arrival = Instant::now();
                let unread = || seen.heartbeats_unread.fetch_add(1, Ordering::Relaxed);
                let Some((addr, covered)) =
                    zenkey_model::heartbeat::covered(sample.key_expr().as_str())
                else {
                    unread();
                    return;
                };
                if covered != iface || !providers.iter().any(|p| p.matches(&addr)) {
                    unread();
                    return;
                }
                // Core §7.5: another form on a heartbeat's key is refused
                // `form`, before its tag or its body is read.
                let (sample, stamp, tag) = match crate::inband::admit_heartbeat(
                    crate::inband::read(sample),
                ) {
                    Ok(heartbeat) => heartbeat,
                    Err(why) => {
                        tracing::debug!(provider = %addr, covered = %covered, refused = why.token(), "a sample on a heartbeat's key is no heartbeat");
                        unread();
                        return;
                    }
                };
                let body = sample.payload().to_bytes();
                let (applied, owner_time, due) = {
                    let mut views = seen.views.lock().expect("not poisoned");
                    let view = f.view_in(&mut views, &addr);
                    let applied = view.apply(tag, stamp, &body, arrival);
                    (
                        applied,
                        view.owner_time(),
                        view.reread_due().contains(&f.index),
                    )
                };
                if due {
                    reread_once(rt.as_ref(), &session, &f, &seen, &addr, timeout);
                }
                let (lost, sections) = match applied {
                    Applied::Read { gap, sections } => (gap, sections),
                    Applied::Older => return,
                    Applied::Revision | Applied::Refused(_) => {
                        unread();
                        return;
                    }
                };
                seen.heartbeats.fetch_add(1, Ordering::Relaxed);
                let section = match sections.get(f.index) {
                    Some(SectionRead::Confirmed(keys)) => HeartbeatSection::Confirmed(keys.clone()),
                    Some(SectionRead::Diverged) => HeartbeatSection::Diverged,
                    Some(SectionRead::Absent) => HeartbeatSection::Absent,
                    // `OtherClass` is a group's reading alone, never a heartbeat's.
                    Some(SectionRead::NotFollowed | SectionRead::OtherClass) | None => return,
                };
                on_confirm(HeartbeatRead {
                    provider: addr,
                    group: None,
                    heartbeat: stamp,
                    arrival,
                    owner_time,
                    lost,
                    section,
                });
            });
            match (redeclaring, horizon) {
                (Some(rd), Some(h)) => {
                    rd.declare(ke, h, on_sample).await?;
                }
                _ => out.push(
                    self.session
                        .declare_subscriber(ke)
                        .callback(move |sample: Sample| on_sample(sample))
                        .await
                        .map_err(zenoh)?,
                ),
            }
        }
        Ok(out)
    }

    /// One subscriber per bound provider on its owner heartbeat key of the
    /// followed collection's class (`freshness.v2` §2.5, "Both forms"): a
    /// provider confirms an interface by its heartbeat or by its owner
    /// heartbeats, and a subscriber cannot tell which before it hears one.
    /// Each owner heartbeat is read through the catalog its owner names
    /// ([`crate::answer::owner_catalog`], read once a run on a task of the
    /// subscription's within [`Consumer::with_owner_catalog_timeout`]'s
    /// timeout, the owner heartbeats heard meanwhile kept in order, at
    /// most [`OWNER_QUEUE`] of them waiting for it and those past it read
    /// in their turn), and each group read for this interface handed to
    /// `on_confirm` as one heartbeat, under no lock of the subscription's.
    ///
    /// **With presence unavailable** ([`Consumer::with_presence`]) no
    /// descriptor crosses to it (core §8.5): each owner heartbeat is read
    /// at once through the last catalog it holds, the deployment's word
    /// (§8.6, R7), and no descriptor is read (#1146).
    ///
    /// **A consumer that holds no catalog declares none.** It could read no
    /// group (core §8.6), so the key would tell it nothing it can use: a
    /// member of an interface its owner confirms by owner heartbeats alone
    /// ages from its last confirmation and goes stale either way, the safe
    /// side (`freshness.v2` §2.4, "Readers first"). Observer §7.5 gives a
    /// tool given no catalog no such scope for the same reason.
    ///
    /// **Outside a tokio runtime** nothing reads a descriptor, so no
    /// catalog is named and no group is read: never a guess.
    ///
    /// Through `redeclaring` across a face (core §8.5), each is declared
    /// again after two of the key's horizons of silence. The horizon is
    /// this consumer's word on it: the shortest among the collections of
    /// the class in the one revision it holds
    /// ([`zenkey_model::link::redeclare_owner_heartbeat_horizon`]). The
    /// key's own is the shortest among every interface its groups cover,
    /// which this consumer cannot see, and which is never longer: so its
    /// re-declaration is never early, and may come later than two of the
    /// key's own horizons.
    async fn owner_heartbeat_subscribers<F>(
        &self,
        f: &Arc<Followed>,
        seen: &Arc<Seen>,
        on_confirm: &Arc<F>,
        redeclaring: Option<&crate::redeclare::Redeclaring>,
    ) -> Result<Vec<Subscriber<()>>>
    where
        F: Fn(HeartbeatRead) + Send + Sync + 'static,
    {
        if self.catalogs.is_empty() {
            return Ok(Vec::new());
        }
        let class = f.class();
        let horizon = redeclaring.and(zenkey_model::link::redeclare_owner_heartbeat_horizon(
            &self.contract,
            class,
        ));
        let rt = tokio::runtime::Handle::try_current().ok();
        // Core §8.5, §8.6: where presence is unavailable no descriptor
        // crosses, and the last catalog held is the deployment's word.
        let word = (self.presence == Presence::Unavailable)
            .then(|| self.catalogs.last().map(zenkey_model::catalog::Catalog::id));
        let reader = Arc::new(OwnerReader {
            f: Arc::clone(f),
            seen: Arc::clone(seen),
            on_read: Arc::clone(on_confirm),
            held: Arc::clone(&self.catalogs),
            session: self.session.clone(),
            reread_timeout: self.reread_timeout,
            catalog_timeout: self.catalog_timeout,
            rt: rt.clone(),
        });
        // The owner heartbeats that wait for their owner's descriptor, read
        // by one task, so they are applied in the order they arrived.
        let waiting = rt.as_ref().map(|rt| {
            let (tx, rx) = flume::unbounded::<OwnerBeat>();
            rt.spawn(Arc::clone(&reader).wait(rx));
            tx
        });
        let mut out = Vec::new();
        for p in &self.providers {
            let (sys, svc) = p.chunks();
            let ke = OwnedKeyExpr::try_from(zenkey_model::heartbeat::owner_key(sys, svc, class))
                .map_err(zenoh)?;
            let (reader, waiting, providers) =
                (Arc::clone(&reader), waiting.clone(), self.providers.clone());
            let on_sample: crate::redeclare::Callback = Arc::new(move |sample: Sample| {
                let arrival = Instant::now();
                let unread = || {
                    reader
                        .seen
                        .heartbeats_unread
                        .fetch_add(1, Ordering::Relaxed)
                };
                // Core §7.5: the key says which layout a put carries.
                let key = sample.key_expr().as_str().to_owned();
                let Some((addr, on)) = zenkey_model::heartbeat::owner_class(&key) else {
                    unread();
                    return;
                };
                if on != class || !providers.iter().any(|p| p.matches(&addr)) {
                    unread();
                    return;
                }
                // Core §7.5: another form on an owner heartbeat's key is
                // refused `form`, before its tag or its body is read, and
                // before its owner's catalog is asked for.
                let (sample, stamp, tag) = match crate::inband::admit_heartbeat(
                    crate::inband::read(sample),
                ) {
                    Ok(heartbeat) => heartbeat,
                    Err(why) => {
                        tracing::debug!(provider = %addr, class = %class, refused = why.token(), "a sample on an owner heartbeat's key is no owner heartbeat");
                        unread();
                        return;
                    }
                };
                let mut beat = OwnerBeat {
                    addr,
                    class,
                    key,
                    stamp,
                    tag,
                    body: sample.payload().to_bytes().into_owned(),
                    arrival,
                    decided: None,
                };
                {
                    let mut named = reader.seen.named.lock().expect("not poisoned");
                    let known = word.or_else(|| named.of(&beat.addr, stamp.run, arrival));
                    match (known, named.pending, &waiting) {
                        // Its owner's catalog is known and none waits: read
                        // here, under the lock that keeps the order.
                        (Some(through), 0, _) => reader.apply(&beat, through),
                        // Others wait: it waits its turn behind them, so no
                        // put on its key is read ahead of one held
                        // (`freshness.v2` §2.5). Past the bound it waits
                        // for no read of its own: it is read through the
                        // catalog known now, or with none, and counted.
                        (_, n, Some(_)) if n >= OWNER_HELD => {
                            reader.seen.owner_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                        (known, n, Some(tx)) => {
                            beat.decided = match known {
                                None if n >= OWNER_QUEUE => {
                                    reader.seen.owner_unqueued.fetch_add(1, Ordering::Relaxed);
                                    Some(None)
                                }
                                known => known,
                            };
                            named.pending += 1;
                            if tx.send(beat).is_err() {
                                named.pending -= 1;
                            }
                        }
                        // No runtime reads a descriptor: no catalog is named,
                        // and no group is read (core §8.6, never a guess).
                        (through, _, None) => reader.apply(&beat, through.flatten()),
                    }
                }
                reader.deliver();
            });
            match (redeclaring, horizon) {
                (Some(rd), Some(h)) => {
                    rd.declare(ke, h, on_sample).await?;
                }
                _ => out.push(
                    self.session
                        .declare_subscriber(ke)
                        .callback(move |sample: Sample| on_sample(sample))
                        .await
                        .map_err(zenoh)?,
                ),
            }
        }
        Ok(out)
    }

    /// Replays an event's occurrences within `retention` (§2.6): a GET on
    /// every bound provider's selector, bounded by `_time`, then filtered by
    /// each occurrence's time on this side, because a storage backend may
    /// ignore `_time` (spike S5: the memory backend does).
    ///
    /// - **Its time** (core §2.6; #980's C2b, C2c) is its event's (§7.5's
    ///   form 4), counted from this consumer's deployment epoch. An unknown
    ///   time is never judged within the retention or beyond it: the
    ///   storage's `_time` bound alone admitted it.
    /// - **A sample that carries no event** (core §2.6), a mutation, a
    ///   heartbeat, a payload with no envelope or a `reply_del`, is never
    ///   replayed, nor one whose envelope or record is refused: each is
    ///   handed to [`Consumer::with_refused`]'s callback, counted as
    ///   [`crate::inband::admit_event`] counts it.
    /// - **The order**: the occurrences with a time by their times, then
    ///   each of one time by its run's value and its seq; those with none
    ///   after them, by run value, then the seq. So one run's occurrences
    ///   keep their seq's order wherever a time does not decide, and two
    ///   runs at one time a fixed order, which says nothing of which ran
    ///   first: two replays of one storage deliver alike.
    pub async fn replay(
        &self,
        resource: &str,
        retention: Duration,
        timeout: Duration,
    ) -> Result<Vec<Delivery>> {
        let r = self.resource(resource)?.clone();
        if r.kind != Kind::Event {
            return Err(Error::Contract(format!("{resource:?} is not an event")));
        }
        let name = format!("{}/{}", r.token, r.template);
        let layout = self.packed_layout(&r)?;
        let since = std::time::SystemTime::now() - retention;
        // An event has no lifespan (timing.v1 §2.3): one premise for every
        // delivery.
        let premise =
            timing::Plan::new(&r, &self.contract.iface, &[], &self.params, self.epsilon).premise();
        let mut out: Vec<(
            Option<std::time::SystemTime>,
            Option<zenkey_model::wire::Stamp>,
            Delivery,
        )> = Vec::new();
        let timeout = self.reply_wait(timeout);
        for ke in self.selectors(resource)? {
            let rx = self
                .session
                .get(format!("{ke}?_time=[now(-{}s)..]", retention.as_secs()))
                .target(zenoh::query::QueryTarget::All)
                .consolidation(zenoh::query::ConsolidationMode::None)
                .timeout(timeout)
                .with(flume::unbounded::<zenoh::query::Reply>())
                .await
                .map_err(zenoh)?;
            while let Ok(reply) = rx.recv_async().await {
                let Ok(sample) = reply.into_result() else {
                    continue;
                };
                let key = sample.key_expr().as_str();
                if key.contains('*') {
                    continue;
                }
                let Ok(ZkKey::Data {
                    addr,
                    resource: mut chunks,
                    ..
                }) = parse(key)
                else {
                    continue;
                };
                // The occurrence chunk, which the grammar checked; the
                // event's time, `None` where it is unknown.
                if chunks.pop().is_none() {
                    continue;
                }
                let at = crate::inband::event_time(&sample)
                    .and_then(|t| t.unix_ms(self.epoch))
                    .and_then(|ms| u64::try_from(ms).ok())
                    .map(|ms| std::time::UNIX_EPOCH + Duration::from_millis(ms));
                if at.is_some_and(|at| at < since)
                    || !self.providers.iter().any(|p| p.matches(&addr))
                {
                    continue;
                }
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                if let Some(values) = r.template.matches(&refs) {
                    let occurred = crate::inband::event_time(&sample);
                    // Core §2.6: a sample that carries no event, a
                    // mutation, a heartbeat, a sample with no envelope or a
                    // `reply_del`, is never replayed.
                    let read = crate::inband::read(sample);
                    if let Err(why) =
                        crate::inband::admit_event(&read, layout.as_deref(), occurred.is_none())
                    {
                        tracing::warn!(key = %read.sample().key_expr(), reason = %why.reason(), "an undecodable occurrence is not replayed");
                        if let Some(f) = &self.refused {
                            f(Refused {
                                provider: addr,
                                resource: name.clone(),
                                values,
                                read,
                                why,
                                arrival: Instant::now(),
                            });
                        }
                        continue;
                    }
                    let (inband, tag) = (read.stamp(), read.tag());
                    let sample = read.into_sample();
                    let stamp = sample.timestamp().copied();
                    out.push((
                        at,
                        inband,
                        Delivery {
                            provider: addr,
                            resource: name.clone(),
                            values,
                            sample,
                            arrival: Instant::now(),
                            stamp,
                            inband,
                            tag,
                            lifespan: premise,
                            frame: None,
                            occurred,
                        },
                    ));
                }
            }
        }
        Ok(replay_order(out))
    }

    /// GETs current state from its owners (S4): every bound provider's keys
    /// for `resource`, one member when `values` binds its parameters (R2's
    /// bindings apply either way), with target `All` and consolidation
    /// `None` set explicitly, its replies merged per key. Silence is
    /// [`StateGet::Silent`], never a verdict (S6, O5); a complete snapshot
    /// of no entry is the owner's word, [`StateGet::Answered`] with none.
    pub async fn get(
        &self,
        resource: &str,
        values: Option<&Bindings>,
        timeout: Duration,
    ) -> Result<StateGet> {
        self.get_answer(resource, values, timeout)
            .await
            .map(StateGet::from)
    }

    /// The selectors of S4's GET on `resource` (core §4.2; #977's E3): one
    /// per bound provider, `values` and R2's bindings applied. Where they
    /// bind none of its template's parameters, the collection read, its
    /// snapshot key followed by `/**`, so that the owner answers with its
    /// collection's snapshot; otherwise [`Consumer::selectors`]' member
    /// read, or the one member `values` names.
    pub fn get_selectors(
        &self,
        resource: &str,
        values: Option<&Bindings>,
    ) -> Result<Vec<OwnedKeyExpr>> {
        let r = self.resource(resource)?.clone();
        let Some(v) = values else {
            return self
                .selectors(resource)?
                .into_iter()
                .map(
                    |ke| match crate::answer::collection_read(ke.as_str(), &r.template) {
                        Some(whole) => OwnedKeyExpr::try_from(whole).map_err(zenoh),
                        None => Ok(ke),
                    },
                )
                .collect();
        };
        let mut merged: Bindings = self
            .params
            .iter()
            .map(|(k, v)| (k.clone(), vec![v.clone()]))
            .collect();
        merged.extend(v.clone());
        let chunks = r
            .template
            .build(&merged)
            .map_err(|e| Error::Contract(format!("{resource:?}: {e}")))?;
        self.providers
            .iter()
            .map(|p| {
                let (sys, svc) = p.chunks();
                let ke = format!(
                    "zk2/{sys}/{svc}/{}/{}/{}",
                    self.contract.iface,
                    r.token,
                    chunks.join("/")
                );
                OwnedKeyExpr::try_from(ke).map_err(zenoh)
            })
            .collect()
    }

    /// [`Consumer::get`], with whether every GET ran to its final reply
    /// ([`StateAnswer::complete`]): a reader that counts members needs to
    /// know (§2.7, 0.24). An error reply, the timeout's included, is kept
    /// out of the answer and makes it incomplete.
    ///
    /// **Snapshots** (core §4.2, §7.5; #977's E3): the owner answers a
    /// collection read with its collection's snapshot, in pages, and a
    /// member read with each member's. Each entry is the per-key reply it
    /// stands for, a live entry a value and a tombstone a deletion, each
    /// with its stamp, merged per key with every other reply by §4.3's
    /// consumer rule, and a live entry's confirmation is kept
    /// ([`StateAnswer::confirmations`]). Each page of the resource's
    /// collection is held toward its snapshot ([`StateAnswer::snapshots`]);
    /// an entry or a per-key reply on a key that is no member of the
    /// resource is not this resource's. A complete snapshot applies the
    /// omission rule over what the answer holds then
    /// ([`StateAnswer::omitted`]), once the owner's descriptor names this
    /// consumer's revision (`answer::runs_revision`, F-497). A snapshot held
    /// incomplete drops nothing and makes the answer incomplete; the
    /// entries it brought stand.
    pub async fn get_answer(
        &self,
        resource: &str,
        values: Option<&Bindings>,
        timeout: Duration,
    ) -> Result<StateAnswer> {
        self.read_state(resource, values, &BTreeMap::new(), None, timeout)
            .await
            .map(|c| c.answer)
    }

    /// [`Consumer::get_answer`] of every member, its replies handed to
    /// `gate` per GET, once each GET ended, and only those it admits merged
    /// (`redundancy.v2` §2.10: a follower's seed merges its incumbent's
    /// snapshots alone, [`crate::follow`]).
    pub(crate) async fn get_gated(
        &self,
        resource: &str,
        gate: &dyn ReplyGate,
        timeout: Duration,
    ) -> Result<StateAnswer> {
        self.read_state(resource, None, &BTreeMap::new(), Some(gate), timeout)
            .await
            .map(|c| c.answer)
    }

    /// A conditional read (core 0.110 §4.2, "A conditional read"; #986's
    /// J1): [`Consumer::get_answer`], where each GET on one concrete key
    /// that `held` names carries that key's held stamp (§7.5) as its
    /// payload, under `zenoh/bytes`. An owner whose member is unchanged
    /// answers with the held stamp alone, 6 or 8 bytes, in place of the
    /// member's snapshot.
    ///
    /// `held` maps a member's key, as this consumer's session hears it, to
    /// the in-band stamp of the mutation the caller holds for it: a
    /// previous answer's [`StateAnswer::inband`] is that map, and so is a
    /// subscription's `Delivery::inband` by key. **Never any other
    /// stamp** (§4.2's MUST NOT): not a snapshot's asof, not another key's,
    /// nothing for a value with no envelope
    /// (`zenkey_model::seed::held_stamp`). A key `held` does not name, and
    /// a selector with a wildcard, are read with no payload, so a
    /// collection read is never conditional.
    ///
    /// What comes back ([`ConditionalAnswer`]):
    ///
    /// - **A held stamp that answers the GET** (the one it carried, on its
    ///   one concrete key) puts the key in
    ///   [`ConditionalAnswer::unchanged`]. It merges nothing: the caller
    ///   keeps what it holds, and `answer.current` holds no value for the
    ///   key unless another reply of this GET brought one, which stands.
    /// - **A snapshot in answer** is accepted and merged as any member
    ///   read's: the owner sends one when the member changed, and an owner
    ///   built before core 0.110 sends one always.
    /// - **Any other held stamp** establishes nothing: counted in
    ///   `answer.undecodable`, refused as `held`
    ///   ([`zenkey_model::wire::Refusal::Held`]). [`Consumer::get_answer`]
    ///   counts every held stamp so, since its GETs carry none.
    /// - **The omission rule** reads the mutation each GET named as held by
    ///   the caller: heard before the GET, and heard again once its
    ///   unchanged answer arrives. A complete snapshot that lists no entry
    ///   for it drops it as it drops any member, in `answer.omitted`; an
    ///   unchanged answer that arrives after that finds nothing held, and
    ///   the key is in [`ConditionalAnswer::read_again`].
    ///
    /// A held stamp carries no time and no confirmation: a reader that ages
    /// a member from its GET (`freshness.v2` §2.6) asks with
    /// [`Consumer::get_answer`].
    pub async fn get_conditional(
        &self,
        resource: &str,
        values: Option<&Bindings>,
        held: &BTreeMap<String, zenkey_model::wire::Stamp>,
        timeout: Duration,
    ) -> Result<ConditionalAnswer> {
        self.read_state(resource, values, held, None, timeout).await
    }

    /// [`Consumer::get_answer`] and [`Consumer::get_conditional`]: S4's
    /// GETs, each carrying the held stamp `held` names for its one concrete
    /// key, and their replies merged; with a `gate`, each GET's replies
    /// handed to it once the GET ended, and those it admits merged.
    async fn read_state(
        &self,
        resource: &str,
        values: Option<&Bindings>,
        held: &BTreeMap<String, zenkey_model::wire::Stamp>,
        gate: Option<&dyn ReplyGate>,
        timeout: Duration,
    ) -> Result<ConditionalAnswer> {
        use zenkey_model::collection::{self, Heard, Page};
        use zenkey_model::seed::{self, Asked, HeldRead, Unchanged};
        let r = self.resource(resource)?.clone();
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!("{resource:?} is not state")));
        }
        let selectors = self.get_selectors(resource, values)?;
        let layout = self.packed_layout(&r)?;
        let mut out: Vec<Option<Current>> = Vec::new();
        let mut at: BTreeMap<String, usize> = BTreeMap::new();
        let mut inband = BTreeMap::new();
        let mut tags = BTreeMap::new();
        let mut undecodable = BTreeMap::new();
        let mut confirmations = BTreeMap::new();
        let mut snapshots: Vec<Heard> = Vec::new();
        let mut pages: Vec<Vec<Page>> = Vec::new();
        let mut omitted: Vec<Vec<String>> = Vec::new();
        let mut complete = true;
        let timeout = self.reply_wait(timeout);
        // The owner's revision under each run a `reply_del` named.
        let mut owners = BTreeMap::new();
        // The resource's own collection, as this consumer's revision reads
        // a page's index: what its snapshots are held toward.
        let ours = |key: &str, page: &Page| {
            collection::collection_at(&self.contract, key, page.coll)
                .is_some_and(|c| c.token == r.token && c.template == r.template)
        };
        // A conditional read's books (core 0.110 §4.2): the mutation each
        // GET named, which the caller holds from before it; the keys whose
        // owner answered unchanged; those among them whose mutation counts
        // as heard again; and those the answer found nothing held for.
        let mut named: BTreeMap<String, zenkey_model::wire::Stamp> = BTreeMap::new();
        let mut unchanged = std::collections::BTreeSet::new();
        let mut heard_again = std::collections::BTreeSet::new();
        let mut read_again = std::collections::BTreeSet::new();
        for ke in selectors {
            // One concrete key alone carries a held stamp: a collection has
            // no single stamp.
            let carried = held
                .get(ke.as_str())
                .copied()
                .filter(|_| seed::is_concrete(ke.as_str()));
            let asked_key = ke.as_str().to_owned();
            let asked = Asked {
                key: &asked_key,
                held: carried,
            };
            let mut get = self
                .session
                .get(ke)
                .target(zenoh::query::QueryTarget::All)
                .consolidation(zenoh::query::ConsolidationMode::None)
                .timeout(timeout);
            if let Some(stamp) = carried {
                named.insert(asked_key.clone(), stamp);
                get = get
                    .payload(zenkey_model::wire::HeldStamp::new(stamp).write())
                    .encoding(zenoh::bytes::Encoding::ZENOH_BYTES);
            }
            let rx = get
                .with(flume::unbounded::<zenoh::query::Reply>())
                .await
                .map_err(zenoh)?;
            let mut replies = match gate {
                None => Replies::Stream(rx),
                Some(g) => {
                    let mut all = Vec::new();
                    while let Ok(reply) = rx.recv_async().await {
                        match reply.into_result() {
                            Ok(sample) => all.push(sample),
                            Err(_) => complete = false,
                        }
                    }
                    Replies::Held(g.admit(all).await.into_iter())
                }
            };
            while let Some(sample) = replies.next(&mut complete).await {
                let key = sample.key_expr().as_str().to_owned();
                if key.contains('*') {
                    continue;
                }
                // A held stamp (core §7.5): the owner's answer only as a
                // reply to the GET that carried the same one, on its one
                // concrete key. Any other establishes nothing, `held`.
                // Either way it merges nothing.
                if let Some(h) = crate::inband::held(&sample) {
                    if asked.read(true, &key, h.stamp) == HeldRead::Refused {
                        undecodable.insert(
                            key,
                            crate::inband::Undecodable::Envelope(
                                zenkey_model::wire::Refusal::Held(h.stamp),
                            ),
                        );
                        continue;
                    }
                    // What the reader holds for the member by now: a value
                    // another reply of this GET brought, else the mutation
                    // it named, unless a snapshot's omission dropped it.
                    let now = match at.get(&key).and_then(|&i| out[i].as_ref()) {
                        Some(c) => match StateAnswer::mark_in(&inband, c) {
                            Some(seed::Mark::InBand(s)) => seed::Held::InBand(s),
                            Some(seed::Mark::Zenoh(z)) => seed::Held::Stamped(z),
                            None => seed::Held::Unstamped,
                        },
                        None => named
                            .get(&key)
                            .map_or(seed::Held::Nothing, |s| seed::Held::InBand(*s)),
                    };
                    match seed::unchanged(now, h.stamp) {
                        Unchanged::HeardAgain => {
                            heard_again.insert(key.clone());
                        }
                        Unchanged::Stands => {}
                        Unchanged::ReadAgain => {
                            read_again.insert(key.clone());
                        }
                    }
                    unchanged.insert(key);
                    continue;
                }
                // A snapshot's page: each entry read as the per-key reply
                // it stands for, and the page held toward its snapshot.
                let mut completed = None;
                let mut entries = BTreeMap::new();
                let from_page = crate::inband::is_snapshot(&sample);
                let reads = match crate::inband::read(sample) {
                    crate::inband::Read::Snapshot { sample, page } => match page {
                        // A page whose body is refused lost its entries: the
                        // answer may miss a member (review-bot on #1078).
                        Err(why) => {
                            complete = false;
                            undecodable
                                .insert(key, crate::inband::Undecodable::Snapshot(Some(why)));
                            continue;
                        }
                        Ok(page) => {
                            if ours(&key, &page) {
                                let (i, done) = collection::hear(&mut snapshots, &key, &page);
                                if i == pages.len() {
                                    pages.push(Vec::new());
                                    omitted.push(Vec::new());
                                }
                                if !pages[i].iter().any(|p| p.index == page.index) {
                                    pages[i].push((*page).clone());
                                }
                                completed = done.then_some(i);
                            }
                            for m in page.members(&key) {
                                entries.insert(
                                    m.key,
                                    crate::state::Confirmed {
                                        confirmation: m.confirmation,
                                        time: page.time,
                                    },
                                );
                            }
                            // freshness.v2 §2.6: an entry's confirmation
                            // is its stamp, a GET reader ages it from it.
                            crate::inband::replies_confirmed(&sample, &page, self.epoch)
                                .into_iter()
                                .map(crate::inband::read)
                                .collect()
                        }
                    },
                    read => vec![read],
                };
                for read in reads {
                    let key = read.sample().key_expr().as_str().to_owned();
                    // A collection read reaches the snapshot key's other
                    // collections too: only this resource's members count.
                    if !collection::is_member(&r, &key) {
                        continue;
                    }
                    if let Err(why) = crate::inband::admit(&read, layout.as_deref()) {
                        undecodable.insert(key, why);
                        continue;
                    }
                    // A `reply_del` where the owner answers by snapshot is
                    // no owner's answer (core §4.2): counted undecodable,
                    // never merged as a deletion. Which resources are per
                    // key is read from the owner's revision under the
                    // deletion's run, never from this consumer's alone
                    // (0.102): an owner on an earlier revision answers its
                    // member per key. A snapshot's tombstone is no
                    // `reply_del` here.
                    if !from_page
                        && read.sample().kind() == zenoh::sample::SampleKind::Delete
                        && let Some(stamp) = read.stamp()
                        && crate::answer::stray_reply_del(
                            &self.session,
                            &key,
                            stamp.run,
                            &self.contract,
                            timeout,
                            &mut owners,
                        )
                        .await
                    {
                        undecodable.insert(
                            key,
                            crate::inband::Undecodable::Envelope(collection::STRAY_REPLY_DEL),
                        );
                        continue;
                    }
                    // One value per key: a key answered twice (a page
                    // heard twice, two instances) keeps what §4.3's
                    // consumer rule accepts.
                    let mark = read.mark();
                    if let Some(held) = at.get(&key).and_then(|&i| out[i].as_ref()) {
                        let held = StateAnswer::mark_in(&inband, held);
                        let newer = match (held, mark) {
                            (_, None) => held.is_none(),
                            (held, Some(m)) => zenkey_model::seed::consumer_accepts(held, m),
                        };
                        if !newer {
                            continue;
                        }
                    }
                    match read.stamp() {
                        Some(stamp) => inband.insert(key.clone(), stamp),
                        None => inband.remove(&key),
                    };
                    match read.tag() {
                        Some(tag) => tags.insert(key.clone(), tag),
                        None => tags.remove(&key),
                    };
                    let sample = read.into_sample();
                    let current = match sample.kind() {
                        zenoh::sample::SampleKind::Delete => {
                            confirmations.remove(&key);
                            Current::Deleted {
                                key: key.clone(),
                                timestamp: sample.timestamp().copied(),
                            }
                        }
                        zenoh::sample::SampleKind::Put => {
                            match entries.get(&key) {
                                Some(c) => confirmations.insert(key.clone(), *c),
                                None => confirmations.remove(&key),
                            };
                            Current::Value {
                                key: key.clone(),
                                sample: Box::new(sample),
                            }
                        }
                    };
                    match at.get(&key) {
                        Some(&i) => out[i] = Some(current),
                        None => {
                            at.insert(key, out.len());
                            out.push(Some(current));
                        }
                    }
                }
                // The omission rule over a snapshot this page completed:
                // every reply of this GET came after it was asked, so only a
                // value of the snapshot's run at or below its asof drops.
                let Some(i) = completed else { continue };
                // And over the mutation a conditional read named, which
                // its caller holds from before the GET: heard after it only
                // once its unchanged answer came (core 0.110 §4.2).
                let held: Vec<(String, Option<zenkey_model::seed::Mark>, bool)> = at
                    .iter()
                    .filter_map(|(k, &j)| {
                        let c = out[j].as_ref()?;
                        Some((k.clone(), StateAnswer::mark_in(&inband, c), true))
                    })
                    .chain(
                        named
                            .iter()
                            .filter(|(k, _)| !at.contains_key(*k))
                            .map(|(k, s)| {
                                let mark = Some(seed::Mark::InBand(*s));
                                (k.clone(), mark, heard_again.contains(k))
                            }),
                    )
                    .collect();
                let drop: Vec<String> = collection::omit(
                    &self.contract,
                    &snapshots[i].key,
                    &pages[i],
                    held.iter()
                        .map(|(k, m, heard_after)| zenkey_model::seed::HeldMember {
                            key: k,
                            mark: *m,
                            heard_after: *heard_after,
                        }),
                )
                .into_iter()
                .map(str::to_owned)
                .collect();
                if drop.is_empty()
                    || !crate::answer::runs_revision(
                        &self.session,
                        &snapshots[i].key,
                        snapshots[i].run,
                        &self.contract,
                        timeout,
                    )
                    .await
                {
                    continue;
                }
                for k in &drop {
                    if let Some(j) = at.remove(k) {
                        out[j] = None;
                    }
                    inband.remove(k);
                    tags.remove(k);
                    confirmations.remove(k);
                    named.remove(k);
                }
                omitted[i].extend(drop);
            }
        }
        // A snapshot held incomplete may lack members: a page lost, or a
        // GET that reached its timeout.
        let complete = complete && snapshots.iter().all(Heard::complete);
        Ok(ConditionalAnswer {
            answer: StateAnswer {
                current: out.into_iter().flatten().collect(),
                complete,
                inband,
                tags,
                undecodable,
                snapshots,
                confirmations,
                omitted,
            },
            unchanged,
            read_again,
        })
    }

    /// The frame layout of `r`, when it is a frame resource (core §7.7):
    /// what reads each frame ([`crate::inband::admit_frame`]). A frame
    /// resource whose layout does not read is refused, [`Error::Contract`].
    fn frame_layout(&self, r: &Resource) -> Result<Option<Arc<zenkey_model::frame::Layout>>> {
        zenkey_model::frame::Layout::of_resource(&self.contract, r)
            .map(|l| l.map(Arc::new))
            .map_err(|e| Error::Contract(format!("{}/{}: {e}", r.token, r.template)))
    }

    /// The layout of `r`'s type, when it is packed (core §7.6): what admits
    /// each record read ([`crate::inband::admit`]). A packed type whose
    /// layout does not read is refused, [`Error::Contract`].
    fn packed_layout(&self, r: &Resource) -> Result<Option<Arc<Layout>>> {
        let Body::Data(d) = &r.body else {
            return Ok(None);
        };
        self.contract
            .packed_layout(&d.type_)
            .map(|l| l.map(Arc::new))
            .map_err(|e| Error::Contract(format!("{}/{}: {e}", r.token, r.template)))
    }

    /// The bound providers serving this interface now (§8.1): each holding
    /// its token, or, for an interface in its tokenless set (U22), an
    /// instance token whose descriptor lists it `"token": false` (#1015).
    /// Sorted. Whether the reads were complete is
    /// [`Consumer::presence`]'s to say.
    pub async fn present(&self, timeout: Duration) -> Result<Vec<Addr>> {
        Ok(self.presence(timeout).await?.providers)
    }

    /// [`Consumer::present`], with whether the reads were complete (§8.1):
    /// a provider missing from a possibly incomplete read is not absent
    /// (observer §3.6).
    pub async fn presence(&self, timeout: Duration) -> Result<crate::Present> {
        crate::presence::serving(
            &self.session,
            &self.providers,
            &self.contract.iface,
            timeout,
            None,
        )
        .await
    }

    /// Waits until a bound provider serves this interface, or `timeout`
    /// passes (R5). Returns the first one a read shows, in address order.
    ///
    /// It reads presence again and again ([`Consumer::presence`]), backing
    /// off from 20 ms to 250 ms, and declares no liveliness subscriber: a
    /// session that reads presence holds none (core §8.1, 0.37; #830),
    /// and this consumer's session goes on reading it ([`Consumer::present`],
    /// [`Consumer::liveness`]). A tokenless provider is found by its
    /// instance token and descriptor (#1015). A read possibly incomplete
    /// that shows no provider, and a read through a session cut off from
    /// every other zenoh node, which is not made, leave the wait going on.
    /// No read waits past `timeout`, so a round it cuts short still returns
    /// a provider its earlier reads found.
    pub async fn wait_for_provider(&self, timeout: Duration) -> Result<Option<Addr>> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut pause = Duration::from_millis(20);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            // Each read waits at most 1 s and none past the deadline, so a
            // round the deadline cuts short still returns what it found.
            let read = crate::presence::serving(
                &self.session,
                &self.providers,
                &self.contract.iface,
                Duration::from_secs(1),
                Some(deadline),
            );
            match read.await {
                Ok(found) => {
                    if let Some(first) = found.providers.into_iter().next() {
                        return Ok(Some(first));
                    }
                }
                Err(Error::CutOff { .. }) => {}
                Err(e) => return Err(e),
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            tokio::time::sleep(pause.min(left)).await;
            pause = (pause * 2).min(Duration::from_millis(250));
        }
    }

    /// The horizon `resource` declares (`freshness.v1` §2.1–§2.3), as the
    /// contract this consumer was compiled against states it.
    pub fn horizon(&self, resource: &str) -> Result<Horizon> {
        Ok(zenkey_model::freshness::horizon_of(
            self.resource(resource)?,
        ))
    }

    /// Whether one member of `resource` is fresh (`freshness.v1` §5), from
    /// the observations made of it: [`Subscription::freshness`], and
    /// [`crate::state::Current::freshness`] for a GET. Combined as §2.7
    /// says.
    pub fn freshness(&self, resource: &str, observations: &[Observation]) -> Result<Judged> {
        Ok(zenkey_model::freshness::judge_all(
            &self.horizon(resource)?,
            observations,
        ))
    }

    /// A provider's liveness (R7): from its presence when presence is
    /// observable, from the freshness of `sub`'s deliveries when not.
    /// `window` is the horizon of what crosses (`freshness.v1` §2.9).
    ///
    /// Presence is read at `provider`'s own address, as
    /// [`Consumer::presence`] reads a binding: its interface token, else,
    /// tokenless, its instance tokens and descriptors (#1015). It is
    /// [`Liveness::Absent`] only when that read is complete (§8.1, observer
    /// §3.6); a possibly incomplete read that does not show it is
    /// [`Liveness::Unobservable`]. A read through a session cut off is not
    /// made: [`Error::CutOff`].
    pub async fn liveness(
        &self,
        sub: &Subscription,
        provider: &Addr,
        window: Duration,
        timeout: Duration,
    ) -> Result<Liveness> {
        Ok(match self.presence {
            Presence::Observable => {
                let read = crate::presence::serving(
                    &self.session,
                    &[Provider::exact(provider)],
                    &self.contract.iface,
                    timeout,
                    None,
                )
                .await?;
                if read.providers.contains(provider) {
                    Liveness::Present
                } else if read.complete {
                    Liveness::Absent
                } else {
                    Liveness::Unobservable
                }
            }
            Presence::Unavailable => match sub.last_seen(provider) {
                Some(t) if t.elapsed() <= window => Liveness::Fresh,
                _ => Liveness::Unobservable,
            },
        })
    }
}

impl Subscription {
    /// How many times one of its subscribers was declared again after two
    /// horizons of silence across a face (core §8.5, [`Consumer::across`]).
    #[must_use]
    pub fn redeclared(&self) -> u64 {
        self.redeclaring
            .as_ref()
            .map_or(0, crate::redeclare::Redeclaring::redeclared)
    }

    /// How many times declaring one of its subscribers again failed, each
    /// retried at the next check and warned of once a horizon (#1047).
    #[must_use]
    pub fn redeclare_failed(&self) -> u64 {
        self.redeclaring
            .as_ref()
            .map_or(0, crate::redeclare::Redeclaring::failed)
    }

    /// Samples discarded as expired (`timing.v1` §2.6): older than their
    /// lifespan on a stated ε, never delivered, and no arrival.
    #[must_use]
    pub fn expired(&self) -> u64 {
        self.timing
            .as_ref()
            .map_or(0, |t| t.expired.load(Ordering::Relaxed))
    }

    /// The owner of `provider` as this subscription's presence follower last
    /// observed it (`timing.v1` §0): unseen without a watch session.
    #[must_use]
    pub fn owner_state(&self, provider: &Addr) -> OwnerState {
        self.timing.as_ref().map_or(OwnerState::Unseen, |t| {
            timing::lock(&t.engine).owner_state(provider)
        })
    }

    /// Where the member `key` of `provider` stands against its deadline
    /// (`timing.v1` §2.5), if it is held.
    #[must_use]
    pub fn deadline_state(&self, provider: &Addr, key: &str) -> Option<MemberState> {
        timing::lock(&self.timing.as_ref()?.engine).member_state(provider, key)
    }

    /// The `meta.zid`s this subscription holds of `provider`'s instances,
    /// read from their descriptors (`timing.v1` §2.6): empty for a
    /// subscription that judges no lifespan.
    #[must_use]
    pub fn owner_zids(&self, provider: &Addr) -> Vec<u128> {
        self.timing.as_ref().map_or_else(Vec::new, |t| {
            timing::lock(&t.engine)
                .owner_zids(provider)
                .into_iter()
                .collect()
        })
    }

    /// The instant this subscription was declared: its deadlines' origin
    /// (`timing.v1` §2.5).
    #[must_use]
    pub fn since(&self) -> Instant {
        self.since
    }

    /// Samples discarded because their key was not concrete (R6).
    #[must_use]
    pub fn discarded(&self) -> u64 {
        self.seen.discarded.load(Ordering::Relaxed)
    }

    /// Samples dropped as undecodable: a stream, state or event sample
    /// under `zenoh/bytes` whose zk2c envelope was refused (core §7.5),
    /// never read as a payload with no envelope, nor delivered, and a
    /// packed record its type's layout refuses (§7.6). Each is handed to
    /// [`Consumer::with_refused`]'s callback, when it has one.
    #[must_use]
    pub fn undecodable(&self) -> u64 {
        self.seen.undecodable.load(Ordering::Relaxed)
    }

    /// Samples dropped because their concrete key did not resolve: it is
    /// not a zk2 data key, names another interface or an unbound provider,
    /// or fits none of the resource's template positions (#671). These are
    /// a mismatch between the bus and the contract, not R6's rule, and a
    /// tool reports them apart from [`Subscription::discarded`] (the
    /// tooling guide's O6).
    #[must_use]
    pub fn unresolved(&self) -> u64 {
        self.seen.unresolved.load(Ordering::Relaxed)
    }

    /// When `provider` last delivered.
    #[must_use]
    pub fn last_seen(&self, provider: &Addr) -> Option<Instant> {
        self.seen
            .last
            .lock()
            .expect("not poisoned")
            .get(provider)
            .copied()
    }

    /// The members delivered so far, by key.
    #[must_use]
    pub fn members(&self) -> Vec<String> {
        self.seen
            .members
            .lock()
            .expect("not poisoned")
            .keys()
            .cloned()
            .collect()
    }

    /// How long it has listened: since it was declared.
    #[must_use]
    pub fn listened(&self) -> Duration {
        self.since.elapsed()
    }

    /// The member `key` as a `freshness.v1` observation now (§2.5): its last
    /// confirmation's age on this host's monotonic clock, never against its
    /// stamp, and how long the subscription has listened. The callback
    /// receives every delivery, so the observation counts as complete.
    ///
    /// A confirmation is its last delivery, or, for a collection, the last
    /// heartbeat of its provider that vouched for it with the view's digest
    /// (#978's F2): the later stands. A gap in that heartbeat key's seqs
    /// is a heartbeat lost, and leaves the observation incomplete until
    /// one arrives in sequence.
    #[must_use]
    pub fn freshness(&self, key: &str) -> Observation {
        let now = Instant::now();
        let delivered = self
            .seen
            .members
            .lock()
            .expect("not poisoned")
            .get(key)
            .map(|(at, deleted)| {
                if *deleted {
                    Last::Delete(now.saturating_duration_since(*at))
                } else {
                    Last::Put(now.saturating_duration_since(*at))
                }
            });
        let heartbeat = self.view_of(key, |v| {
            v.confirmed(key).map(|c| now.saturating_duration_since(c))
        });
        Observation::Subscribed {
            last: zenkey_model::freshness::confirmed_last(delivered, heartbeat),
            listened: self.listened(),
            complete: !self.lost(key),
        }
    }

    /// The provider whose key `key` is.
    fn provider_of(key: &str) -> Option<Addr> {
        zenkey_model::heartbeat::base_relative(key).and_then(|k| match parse(k) {
            Ok(ZkKey::Data { addr, .. }) => Some(addr),
            _ => None,
        })
    }

    /// `f` over the view of the provider whose key `key` is, or its
    /// defaults with none.
    fn view_of<R: Default>(&self, key: &str, f: impl FnOnce(&View<Instant>) -> R) -> R {
        let views = self.seen.views.lock().expect("not poisoned");
        let iface = self.followed.as_ref().map(|f| &f.layout.iface);
        Self::provider_of(key)
            .zip(iface)
            .and_then(|(a, i)| views.get(&a)?.view(i))
            .map(f)
            .unwrap_or_default()
    }

    /// Whether `key`'s collection is read as a lost delivery
    /// (`freshness.v2` §2.5): the last heartbeat on its provider's
    /// heartbeat key followed a gap in that key's seqs, or the last owner
    /// heartbeat on the provider's key of the collection's class did, and
    /// a group on that key named the interface within the run. Until one
    /// arrives in sequence on that key, or one of another run does.
    #[must_use]
    pub fn lost(&self, key: &str) -> bool {
        let Some(f) = &self.followed else {
            return false;
        };
        let views = self.seen.views.lock().expect("not poisoned");
        Self::provider_of(key)
            .and_then(|a| views.get(&a))
            .is_some_and(|vs| vs.is_lost(&f.layout.iface, f.index))
    }

    /// The class of the collection this subscription follows, the one whose
    /// owner heartbeat key it declares when its consumer holds a catalog
    /// (`freshness.v2` §2.5): `None` for a subscription that follows no
    /// collection, which no heartbeat of either form confirms. A consumer
    /// that holds no catalog declares no class key, so a tool given none
    /// can say what it did not listen to (`freshness.v2` §5).
    #[must_use]
    pub fn followed_class(&self) -> Option<zenkey_model::heartbeat::Class> {
        self.followed.as_ref().map(|f| f.class())
    }

    /// Owner heartbeats read on a bound provider's key of the followed
    /// collection's class, whatever became of their groups
    /// (`freshness.v2` §2.5; #1110). A group read for this interface
    /// counts among [`Subscription::heartbeats`].
    #[must_use]
    pub fn owner_heartbeats(&self) -> u64 {
        self.seen.owner_heartbeats.load(Ordering::Relaxed)
    }

    /// Reads of a provider's descriptor made to learn the catalog it
    /// names (core §8.6): one a run, and after one that failed no other
    /// for [`OWNER_CATALOG_RETRY`].
    #[must_use]
    pub fn owner_catalog_reads(&self) -> u64 {
        self.seen.catalog_reads.load(Ordering::Relaxed)
    }

    /// Owner heartbeats heard past [`OWNER_QUEUE`], while that many waited
    /// for a descriptor's read and their own owner's catalog was not
    /// known: each is read in its turn, behind those held, with no
    /// catalog, every group `unknown_catalog`, and confirms nothing.
    /// Counted as each is heard.
    #[must_use]
    pub fn owner_heartbeats_unqueued(&self) -> u64 {
        self.seen.owner_unqueued.load(Ordering::Relaxed)
    }

    /// Owner heartbeats dropped unread, [`OWNER_HELD`] being held behind a
    /// descriptor's read. Each leaves a gap its key reads at the next put
    /// read, so the members it would have confirmed go stale.
    #[must_use]
    pub fn owner_heartbeats_dropped(&self) -> u64 {
        self.seen.owner_dropped.load(Ordering::Relaxed)
    }

    /// The last heartbeat of `key`'s provider that vouched for it, with
    /// the view's digest (#978's F2).
    #[must_use]
    pub fn confirmed(&self, key: &str) -> Option<Instant> {
        self.view_of(key, |v| v.confirmed(key))
    }

    /// Heartbeats folded into a provider's view (#978's F2).
    #[must_use]
    pub fn heartbeats(&self) -> u64 {
        self.seen.heartbeats.load(Ordering::Relaxed)
    }

    /// Heartbeats heard that could not be folded: another revision's tag,
    /// a body refused, or a sample on the heartbeat key that is none.
    #[must_use]
    pub fn heartbeats_unread(&self) -> u64 {
        self.seen.heartbeats_unread.load(Ordering::Relaxed)
    }

    /// Whether `provider`'s collection is due a read: two heartbeats in a
    /// row whose digest was not the view's (#978's design §1.5). Nothing
    /// reads it again yet.
    #[must_use]
    pub fn reread_due(&self, provider: &Addr) -> bool {
        self.seen
            .views
            .lock()
            .expect("not poisoned")
            .get(provider)
            .is_some_and(|vs| vs.iter().any(|v| !v.reread_due().is_empty()))
    }

    /// Per stamping clock (a stamp's id), what this subscription measured
    /// of it, receipt minus stamp, over its lifetime, which is its reading
    /// (`freshness.v1` §2.6, ground 2, 0.2).
    #[must_use]
    pub fn clock_offsets(&self) -> BTreeMap<String, ClockMeasure> {
        self.seen.clocks.lock().expect("not poisoned").clone()
    }

    /// Whether this host's clock is trusted to `delta` against `clock`, a
    /// stamp's id (`freshness.v1` §2.6, ground 2, 0.2): a delivery stamped by
    /// it arrived within `delta` of this host's clock, and none arrived
    /// stamped further ahead than `delta`.
    #[must_use]
    pub fn clock_trust(&self, clock: &str, delta: Duration) -> ClockTrust {
        self.seen
            .clocks
            .lock()
            .expect("not poisoned")
            .get(clock)
            .map_or(ClockTrust::Untrusted, |m| m.trust(delta))
    }
}

impl Subscription {
    /// The re-reads that replaced a provider's view of the collection it
    /// follows (`freshness.v2` §2.5): each after two heartbeats in a row
    /// whose digest was not the view's.
    #[must_use]
    pub fn rereads(&self) -> u64 {
        self.seen.rereads.load(Ordering::Relaxed)
    }

    /// When the last re-read replaced a view, on this host's monotonic
    /// clock.
    #[must_use]
    pub fn last_reread(&self) -> Option<Instant> {
        *self.seen.last_reread.lock().expect("not poisoned")
    }

    /// Seeds the views of the collection it follows from a late joiner's
    /// own S4 GET, asked at `asked` (core §4.2, `freshness.v2` §2.5): each
    /// provider's answered members replace its view, merged with what the
    /// subscription heard since `asked`, so the next heartbeat whose
    /// digest the owner's is confirms them. Nothing for a subscription
    /// that follows no collection.
    pub fn seed(&self, answer: &crate::state::StateAnswer, asked: Instant) {
        let Some(f) = &self.followed else {
            return;
        };
        let now = Instant::now();
        let mut by: BTreeMap<Addr, Vec<(String, Option<zenkey_model::wire::Stamp>)>> =
            BTreeMap::new();
        for c in &answer.current {
            let crate::state::Current::Value { key, .. } = c else {
                continue;
            };
            let Some(k) = zenkey_model::heartbeat::base_relative(key) else {
                continue;
            };
            if f.layout.collection_of(k) != Some(f.index) {
                continue;
            }
            if let Ok(ZkKey::Data { addr, .. }) = parse(k) {
                by.entry(addr)
                    .or_default()
                    .push((k.to_owned(), answer.inband.get(key).copied()));
            }
        }
        let mut views = self.seen.views.lock().expect("not poisoned");
        for (addr, live) in by {
            let view = f.view_in(&mut views, &addr);
            view.reread(
                f.index,
                live.iter().map(|(k, s)| (k.as_str(), *s)),
                asked,
                now,
            );
            // freshness.v2 §2.6 (text 0.7): an entry's confirmation, aged
            // from when the owner built its snapshot.
            for (k, _) in &live {
                if let Some(at) = answer
                    .confirmations
                    .get(k)
                    .and_then(|c| confirmed_at(c, now))
                {
                    view.confirm(k, at);
                }
            }
        }
    }
}

/// When a snapshot's entry says the owner last vouched for its member, on
/// this host's monotonic clock: `arrived`, the page's arrival, less how long
/// before building the snapshot the owner vouched, both on its clock
/// (`freshness.v2` §2.6). `None` for confirmation 0 or a time unknown.
fn confirmed_at(c: &crate::state::Confirmed, arrived: Instant) -> Option<Instant> {
    let before = c.before_snapshot()?;
    Some(arrived.checked_sub(before).unwrap_or(arrived))
}

/// Starts a re-read of `addr`'s collection after two divergences in a row
/// (`freshness.v2` §2.5), unless one is running for it: one at a time.
/// Nothing without a runtime to run it on.
fn reread_once(
    rt: Option<&tokio::runtime::Handle>,
    session: &zenoh::Session,
    f: &Arc<Followed>,
    seen: &Arc<Seen>,
    addr: &Addr,
    timeout: Duration,
) {
    if let Some(rt) = rt
        && seen
            .rereading
            .lock()
            .expect("not poisoned")
            .insert(addr.clone())
    {
        rt.spawn(reread(
            session.clone(),
            Arc::clone(f),
            Arc::clone(seen),
            addr.clone(),
            timeout,
        ));
    }
}

/// One owner heartbeat as its subscriber heard it (core §7.5): the class
/// key's stamp, CH's tag and the body, with its arrival.
struct OwnerBeat {
    addr: Addr,
    class: zenkey_model::heartbeat::Class,
    /// The key as heard, a namespace in front or none: where its owner's
    /// descriptors are asked.
    key: String,
    stamp: zenkey_model::wire::Stamp,
    tag: zenkey_model::wire::Tag,
    body: Vec<u8>,
    arrival: Instant,
    /// The catalog it is read through, where its arrival decided it: the
    /// one known for its run then, or none, past [`OWNER_QUEUE`]. `None`:
    /// its turn reads its owner's descriptor where none is known by then.
    decided: Option<Option<zenkey_model::catalog::CatalogId>>,
}

/// What reads a subscription's owner heartbeats (`freshness.v2` §2.5):
/// the views they confirm, the catalogs held, and the callback each group
/// read for the interface is handed to.
struct OwnerReader<F> {
    f: Arc<Followed>,
    seen: Arc<Seen>,
    on_read: Arc<F>,
    held: Arc<Vec<zenkey_model::catalog::Catalog>>,
    session: zenoh::Session,
    /// A re-read's timeout, and a descriptor read's (core §8.1).
    reread_timeout: Duration,
    catalog_timeout: Duration,
    rt: Option<tokio::runtime::Handle>,
}

impl<F> OwnerReader<F>
where
    F: Fn(HeartbeatRead) + Send + Sync + 'static,
{
    /// Reads the owner heartbeats that waited, in order of arrival: each
    /// through the catalog its arrival decided, else the one its owner's
    /// descriptor names under its run, read once a run within the
    /// consumer's [`Consumer::with_owner_catalog_timeout`] (core §8.6).
    /// - **No descriptor reaches the reader**, as across a constrained
    ///   face: the deployment's word, the last catalog this consumer was
    ///   given.
    /// - **A read that failed** names none for its run until
    ///   [`OWNER_CATALOG_RETRY`] has passed, when the next owner heartbeat
    ///   asks again: never a guess, and never a GET a put.
    async fn wait(self: Arc<Self>, rx: flume::Receiver<OwnerBeat>) {
        while let Ok(beat) = rx.recv_async().await {
            let known = beat.decided.or_else(|| {
                self.seen.named.lock().expect("not poisoned").of(
                    &beat.addr,
                    beat.stamp.run,
                    Instant::now(),
                )
            });
            let read = match known {
                Some(_) => None,
                None => {
                    self.seen.catalog_reads.fetch_add(1, Ordering::Relaxed);
                    Some(
                        crate::answer::owner_catalog(
                            &self.session,
                            &beat.key,
                            beat.stamp.run,
                            self.catalog_timeout,
                        )
                        .await,
                    )
                }
            };
            {
                let mut named = self.seen.named.lock().expect("not poisoned");
                let through = match read {
                    None => known.flatten(),
                    Some(said) => {
                        let (through, retry) = match said {
                            crate::answer::OwnerCatalog::Named(c) => (c, None),
                            crate::answer::OwnerCatalog::Unreached => (
                                self.held.last().map(zenkey_model::catalog::Catalog::id),
                                None,
                            ),
                            crate::answer::OwnerCatalog::Unread => {
                                (None, Some(Instant::now() + OWNER_CATALOG_RETRY))
                            }
                        };
                        named.by.insert(
                            beat.addr.clone(),
                            Asked {
                                run: beat.stamp.run,
                                named: through,
                                retry,
                            },
                        );
                        through
                    }
                };
                self.apply(&beat, through);
                named.pending = named.pending.saturating_sub(1);
            }
            self.deliver();
        }
    }

    /// Hands the reads that wait to the callback, in the order they were
    /// made, under no lock of the subscription's: one caller at a time
    /// hands over, and a read made meanwhile is taken by it. So a callback
    /// may call back into its subscription, and a slow one stalls no
    /// owner heartbeat's reading.
    fn deliver(&self) {
        loop {
            let next = {
                let mut out = self.seen.outbox.lock().expect("not poisoned");
                if out.delivering {
                    return;
                }
                match out.queue.pop_front() {
                    Some(read) => {
                        out.delivering = true;
                        read
                    }
                    None => return,
                }
            };
            (self.on_read)(next);
            self.seen.outbox.lock().expect("not poisoned").delivering = false;
        }
    }

    /// One owner heartbeat read into its provider's views, through the
    /// catalog `named` (`None`: its owner names none, or none could be
    /// read, so every group is refused `unknown_catalog`). Each group read
    /// for the subscription's interface is queued for the callback as one
    /// heartbeat ([`OwnerReader::deliver`]). The caller holds the lock
    /// that keeps owner heartbeats in order.
    fn apply(&self, beat: &OwnerBeat, named: Option<zenkey_model::catalog::CatalogId>) {
        use zenkey_model::freshness::{Catalogs, GroupRead, OwnerApplied};
        let (f, seen) = (&self.f, &self.seen);
        let iface = &f.layout.iface;
        let unread = || seen.heartbeats_unread.fetch_add(1, Ordering::Relaxed);
        let (applied, owner_time, due) = {
            let mut views = seen.views.lock().expect("not poisoned");
            f.view_in(&mut views, &beat.addr);
            let vs = views.get_mut(&beat.addr).expect("just made");
            let applied = vs.apply_owner(
                beat.class,
                beat.tag,
                beat.stamp,
                &beat.body,
                Catalogs {
                    held: &self.held,
                    named,
                },
                beat.arrival,
            );
            let view = vs.view(iface);
            (
                applied,
                view.and_then(View::owner_time),
                view.is_some_and(|v| v.reread_due().contains(&f.index)),
            )
        };
        if due {
            reread_once(
                self.rt.as_ref(),
                &self.session,
                f,
                seen,
                &beat.addr,
                self.reread_timeout,
            );
        }
        let (lost, groups) = match applied {
            OwnerApplied::Read { gap, groups } => (gap, groups),
            OwnerApplied::Older => return,
            OwnerApplied::Refused(_) => {
                unread();
                return;
            }
        };
        seen.owner_heartbeats.fetch_add(1, Ordering::Relaxed);
        for g in groups {
            let sections = match (&g.iface, g.read) {
                // Its index was not read: it may have been this
                // interface's, and confirmed nothing.
                (None, _) => {
                    unread();
                    continue;
                }
                (Some(i), _) if i != iface => continue,
                (_, GroupRead::NotFollowed) => continue,
                (_, GroupRead::Revision | GroupRead::Refused(_)) => {
                    unread();
                    continue;
                }
                (_, GroupRead::Read(sections)) => sections,
            };
            seen.heartbeats.fetch_add(1, Ordering::Relaxed);
            let section = match sections.get(f.index) {
                Some(SectionRead::Confirmed(keys)) => HeartbeatSection::Confirmed(keys.clone()),
                Some(SectionRead::Diverged) => HeartbeatSection::Diverged,
                Some(SectionRead::Absent) => HeartbeatSection::Absent,
                // The key is the followed collection's class's, so its
                // section is never `OtherClass`.
                Some(SectionRead::NotFollowed | SectionRead::OtherClass) | None => continue,
            };
            seen.outbox
                .lock()
                .expect("not poisoned")
                .queue
                .push_back(HeartbeatRead {
                    provider: beat.addr.clone(),
                    group: Some(OwnerGroup {
                        class: beat.class,
                        index: g.index,
                    }),
                    heartbeat: beat.stamp,
                    arrival: beat.arrival,
                    owner_time,
                    lost,
                    section,
                });
        }
    }
}

/// The selector of `addr`'s members of the collection `f` follows: its
/// collection read, its snapshot key followed by `/**` (core §4.2,
/// `freshness.v2` §2.5), where it has one; else its members' template
/// selector ([`zenkey_model::grammar::template_selector`]). Base-relative,
/// so the consumer's session adds its namespace on egress and its replies
/// come back base-relative, as [`zenkey_model::heartbeat::base_relative`]
/// reads them either way.
fn collection_selector(f: &Followed, addr: &Addr) -> Option<OwnedKeyExpr> {
    let c = f.layout.collections.get(f.index)?;
    let members =
        zenkey_model::grammar::template_selector(addr, &f.layout.iface, c.token, &c.template);
    let whole = crate::answer::collection_read(&members, &c.template).unwrap_or(members);
    OwnedKeyExpr::try_from(whole).ok()
}

/// A re-read of `addr`'s collection after two divergences in a row
/// (`freshness.v2` §2.5): S4's GET on the collection. Only a complete
/// answer replaces the view, merged with what the subscription heard since
/// it asked (core §4.2), since a member an answer leaves out leaves the
/// view. An answer is possibly incomplete, as
/// [`crate::presence::liveliness_read`] reads one, when the GET got an
/// error reply (zenoh's at its timeout among them), ran to its timeout or
/// to the session's interests timeout, or ended with its session cut off;
/// such an answer replaces nothing, and the collection stays unconfirmed:
/// the safe side. The next two divergences ask again.
async fn reread(
    session: zenoh::Session,
    f: Arc<Followed>,
    seen: Arc<Seen>,
    addr: Addr,
    timeout: Duration,
) {
    let done = || {
        seen.rereading.lock().expect("not poisoned").remove(&addr);
    };
    let Some(ke) = collection_selector(&f, &addr) else {
        done();
        return;
    };
    let asked = Instant::now();
    let ends_at = timeout.min(crate::presence::interests_timeout(&session));
    let rx = match session
        .get(ke)
        .target(zenoh::query::QueryTarget::All)
        .consolidation(zenoh::query::ConsolidationMode::None)
        .timeout(timeout)
        .with(flume::unbounded::<zenoh::query::Reply>())
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            tracing::debug!(provider = %addr, "freshness.v2: a re-read was not asked: {e}");
            done();
            return;
        }
    };
    // One value per key, the newer by §4.3's consumer rule: a key answered
    // twice (two instances) keeps the one it accepts.
    let mut held: BTreeMap<String, (Option<Mark>, bool, Option<crate::state::Confirmed>)> =
        BTreeMap::new();
    let mut complete = true;
    let mut snapshots: Vec<zenkey_model::collection::Heard> = Vec::new();
    let ours = |k: &str| {
        zenkey_model::heartbeat::base_relative(k)
            .is_some_and(|b| f.layout.collection_of(b) == Some(f.index))
    };
    while let Ok(reply) = rx.recv_async().await {
        let Ok(sample) = reply.into_result() else {
            complete = false;
            continue;
        };
        // A snapshot's page: its entries, each the per-key reply it stands
        // for; a page whose body is refused leaves the answer incomplete.
        let mut entries = BTreeMap::new();
        // A `reply_del` of a collection its owner answers by snapshot is no
        // owner's answer (core S2, S3; #977's E4), and replaces nothing.
        if sample.kind() == zenoh::sample::SampleKind::Delete && !f.per_key {
            continue;
        }
        let reads = match crate::inband::read(sample) {
            crate::inband::Read::Snapshot { sample, page } => match page {
                Err(_) => {
                    complete = false;
                    continue;
                }
                Ok(page) => {
                    let key = sample.key_expr().as_str().to_owned();
                    zenkey_model::collection::hear(&mut snapshots, &key, &page);
                    for m in page.members(&key) {
                        entries.insert(
                            m.key,
                            crate::state::Confirmed {
                                confirmation: m.confirmation,
                                time: page.time,
                            },
                        );
                    }
                    crate::inband::replies(&sample, &page)
                        .into_iter()
                        .map(crate::inband::read)
                        .collect()
                }
            },
            read => vec![read],
        };
        for read in reads {
            let key = read.sample().key_expr().as_str().to_owned();
            // The collection read reaches its snapshot key's other
            // collections too.
            if !ours(&key) {
                continue;
            }
            let mark = read.mark();
            let newer = match (held.get(&key).and_then(|h| h.0), mark) {
                (_, None) => !held.contains_key(&key),
                (h, Some(m)) => zenkey_model::seed::consumer_accepts(h, m),
            };
            if newer {
                let put = read.sample().kind() == zenoh::sample::SampleKind::Put;
                held.insert(key.clone(), (mark, put, entries.get(&key).copied()));
            }
        }
    }
    let arrived = Instant::now();
    let live: Vec<(String, Option<zenkey_model::wire::Stamp>)> = held
        .iter()
        .filter(|(_, (_, put, _))| *put)
        .map(|(k, (m, _, _))| {
            let stamp = match m {
                Some(Mark::InBand(s)) => Some(*s),
                _ => None,
            };
            (k.clone(), stamp)
        })
        .collect();
    let complete = complete
        && snapshots
            .iter()
            .all(zenkey_model::collection::Heard::complete)
        && asked.elapsed() < ends_at
        && !crate::presence::cut_off(&session).await;
    if complete {
        let mut views = seen.views.lock().expect("not poisoned");
        if let Some(v) = views
            .get_mut(&addr)
            .and_then(|vs| vs.view_mut(&f.layout.iface))
        {
            // A member the complete snapshot left out leaves the view (core
            // §4.2's omission rule, freshness.v2 §2.5); what the
            // subscription heard since `asked` stands where newer.
            v.reread(
                f.index,
                live.iter().map(|(k, s)| (k.as_str(), *s)),
                asked,
                Instant::now(),
            );
            for (k, (_, _, c)) in &held {
                if let Some(at) = c.as_ref().and_then(|c| confirmed_at(c, arrived)) {
                    v.confirm(k, at);
                }
            }
        }
        drop(views);
        *seen.last_reread.lock().expect("not poisoned") = Some(Instant::now());
        seen.rereads.fetch_add(1, Ordering::Relaxed);
    }
    done();
}

/// Replay's order ([`Consumer::replay`], core 0.101 §2.6): the occurrences
/// with a time by it, then each of one time by its run's value (a 16-bit
/// run before a 32-bit one) and its seq; those whose time is unknown after
/// them, by run value and seq. An occurrence with no in-band stamp ranks
/// after every run. Arrival breaks every other tie: a total order, so the
/// sort is stable and well defined.
fn replay_order(
    items: Vec<(
        Option<std::time::SystemTime>,
        Option<zenkey_model::wire::Stamp>,
        Delivery,
    )>,
) -> Vec<Delivery> {
    // Nothing orders two runs (core §4.3). Between two runs at one time,
    // or among unknown times, the tie is broken by the run's value: a
    // fixed rule, so two replays of one storage deliver alike, where the
    // order the runs were first heard in follows the replies' arrival
    // (review-bot on #1093). It says nothing about which run came first.
    let mut keyed: Vec<_> = items
        .into_iter()
        .enumerate()
        .map(|(i, (at, stamp, d))| {
            let rank: (u8, u32) = match stamp.map(|s| s.run) {
                Some(zenkey_model::wire::Run::U16(r)) => (0, u32::from(r)),
                Some(zenkey_model::wire::Run::U32(r)) => (1, r),
                None => (2, 0),
            };
            let seq = stamp.map_or(0, |s| s.seq.get());
            ((at.is_none(), at, rank, seq, i), d)
        })
        .collect();
    keyed.sort_by_key(|a| a.0);
    keyed.into_iter().map(|(_, d)| d).collect()
}
