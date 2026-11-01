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

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Body, Contract, Resource};
use zenkey_model::freshness::{
    Applied, ClockMeasure, ClockTrust, Horizon, Judged, Last, Observation, SectionRead, StampAge,
    View,
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
use crate::state::{Current, StateAnswer, StateGet};
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

/// A sample a subscription refused as undecodable (core §7.5, §7.6): its
/// envelope refused, or a packed record its type's layout refuses. Never
/// delivered nor ordered, and counted ([`Subscription::undecodable`]);
/// handed to [`Consumer::with_refused`]'s callback, for a tool that shows
/// it with its refusal named (observer §7.3).
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
    /// subscriber's callback.
    pub arrival: Instant,
}

/// What [`Consumer::with_refused`] hands each refused sample to.
type OnRefused = Arc<dyn Fn(Refused) + Send + Sync>;

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
    /// The deployment epoch its typed reads count packed `time` fields
    /// from, in seconds (core §7.6, §8.6): its service's catalog's, else 0.
    epoch: i64,
}

/// How long a re-read waits unless [`Consumer::with_reread_timeout`] says
/// otherwise: a GET's timeout, the caller's choice (core §8.1).
pub const REREAD_TIMEOUT: Duration = Duration::from_secs(5);

/// A live subscription through a role. Dropping it undeclares it, and
/// stops its `timing.v1` engine.
pub struct Subscription {
    _subs: Vec<Subscriber<()>>,
    /// Each bound provider's heartbeat subscriber, for a collection.
    _heartbeats: Vec<Subscriber<()>>,
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
    /// (#978's F2).
    views: Mutex<BTreeMap<Addr, View<Instant>>>,
    /// Heartbeats folded into a view, and heartbeats heard that could not
    /// be: another revision's tag, a body refused, a sample on the
    /// heartbeat key that is no heartbeat.
    heartbeats: AtomicU64,
    heartbeats_unread: AtomicU64,
    /// The providers whose collection a re-read is running for, and the
    /// re-reads that replaced a view (`freshness.v2` §2.5).
    rereading: Mutex<std::collections::BTreeSet<Addr>>,
    rereads: AtomicU64,
    last_reread: Mutex<Option<Instant>>,
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
    /// The owner's clock when it wrote it (`None`: unknown).
    pub owner_time: Option<zenkey_model::heartbeat::Time>,
}

/// A collection a subscription follows by heartbeats: the revision's
/// layout, and the collection's index in it.
struct Followed {
    layout: zenkey_model::heartbeat::Layout,
    index: usize,
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
            epoch: zenkey_model::catalog::EPOCH_WITHOUT_CATALOG,
        })
    }

    /// How long a subscription's re-read of a diverged collection waits
    /// for its replies (`freshness.v2` §2.5): across a slow face, at least
    /// its round trip. [`REREAD_TIMEOUT`] unless set.
    #[must_use]
    pub fn with_reread_timeout(mut self, timeout: Duration) -> Self {
        self.reread_timeout = timeout;
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

    /// Reads under `catalog`'s epoch ([`Consumer::with_deployment_epoch`]): a tool's,
    /// given the deployment's catalog.
    #[must_use]
    pub fn with_catalog(self, catalog: &zenkey_model::catalog::Catalog) -> Self {
        self.with_deployment_epoch(catalog.epoch())
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
    /// deployment whose providers sit across a constrained face.
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
    /// ([`Subscription::undecodable`]), from the subscriber's callback. A
    /// tool that shows samples shows such a one, its refusal named, and
    /// never drops it silently (observer §7.3). Without it the sample is
    /// counted alone.
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

    pub(crate) fn resource(&self, name: &str) -> Result<&Resource> {
        name.split_once('/')
            .and_then(|(t, tpl)| self.contract.resource(t.parse().ok()?, tpl))
            .ok_or_else(|| Error::NoResource {
                iface: self.contract.iface.clone(),
                resource: name.to_owned(),
            })
    }

    /// The key expressions a subscription or GET on `resource` uses: one per
    /// bound provider, with R2's parameter bindings applied and every other
    /// parameter a wildcard. An event's selector ends in `*`, the ULID.
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
        self.subscribe_inner(resource, callback, |_| {}, on_confirm)
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
        Some(Followed { layout, index })
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
        F: Fn(Confirmation) + Send + Sync + 'static,
    {
        let r = self.resource(resource)?.clone();
        let name = format!("{}/{}", r.token, r.template);
        let layout = self.packed_layout(&r)?;
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
        let heartbeats = match &followed {
            Some(f) => self.heartbeat_subscribers(f, &seen, on_confirm).await?,
            None => Vec::new(),
        };
        let mut subs = Vec::new();
        for ke in self.selectors(resource)? {
            let (seen, callback, r, name, engine, layout, followed) = (
                Arc::clone(&seen),
                Arc::clone(&callback),
                r.clone(),
                name.clone(),
                engine.clone(),
                layout.clone(),
                followed.clone(),
            );
            let iface = self.contract.iface.clone();
            let providers = self.providers.clone();
            let on_refused = self.refused.clone();
            subs.push(
                self.session
                    .declare_subscriber(ke)
                    .callback(move |sample: Sample| {
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
                        // delivered, never an arrival.
                        let read = crate::inband::read(sample);
                        let (inband, tag) = (read.stamp(), read.tag());
                        if let Err(why) = crate::inband::admit(&read, layout.as_deref()) {
                            seen.undecodable.fetch_add(1, Ordering::Relaxed);
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
                        let sample = read.into_sample();
                        let key = sample.key_expr().as_str();
                        let lifespan = match &engine {
                            Some(e) => match timing::lock(e)
                                .arrive(&addr, key, &chunks, &sample, arrival, wall)
                            {
                                Some(l) => l,
                                // Expired: discarded, counted, no arrival.
                                None => return,
                            },
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
                            let view = views.entry(addr.clone()).or_insert_with(|| {
                                let mut v = View::new(f.layout.clone());
                                v.follow(f.index);
                                v
                            });
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
                        });
                    })
                    .await
                    .map_err(zenoh)?,
            );
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
            followed,
            seen,
            since,
            timing,
        })
    }

    /// One heartbeat subscriber per bound provider, on the key of its
    /// heartbeat covering this interface: each heartbeat folded into the
    /// provider's view, what it confirmed handed to `on_confirm`.
    async fn heartbeat_subscribers<F>(
        &self,
        f: &Arc<Followed>,
        seen: &Arc<Seen>,
        on_confirm: F,
    ) -> Result<Vec<Subscriber<()>>>
    where
        F: Fn(Confirmation) + Send + Sync + 'static,
    {
        let on_confirm = Arc::new(on_confirm);
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
                Arc::clone(&on_confirm),
                self.providers.clone(),
            );
            let iface = self.contract.iface.clone();
            let (session, timeout, rt) = (self.session.clone(), self.reread_timeout, rt.clone());
            out.push(
                self.session
                    .declare_subscriber(ke)
                    .callback(move |sample: Sample| {
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
                        let crate::inband::Read::Heartbeat { sample, stamp, tag } =
                            crate::inband::read(sample)
                        else {
                            unread();
                            return;
                        };
                        let body = sample.payload().to_bytes();
                        let (applied, owner_time, due) = {
                            let mut views = seen.views.lock().expect("not poisoned");
                            let view = views.entry(addr.clone()).or_insert_with(|| {
                                let mut v = View::new(f.layout.clone());
                                v.follow(f.index);
                                v
                            });
                            let applied = view.apply(tag, stamp, &body, arrival);
                            (
                                applied,
                                view.owner_time(),
                                view.reread_due().contains(&f.index),
                            )
                        };
                        // freshness.v2 §2.5: two divergences in a row, a
                        // re-read of the provider's collection, one at a
                        // time.
                        if due
                            && let Some(rt) = &rt
                            && seen
                                .rereading
                                .lock()
                                .expect("not poisoned")
                                .insert(addr.clone())
                        {
                            rt.spawn(reread(
                                session.clone(),
                                Arc::clone(&f),
                                Arc::clone(&seen),
                                addr.clone(),
                                timeout,
                            ));
                        }
                        let sections = match applied {
                            Applied::Read { sections, .. } => sections,
                            Applied::Older => return,
                            Applied::Revision | Applied::Refused(_) => {
                                unread();
                                return;
                            }
                        };
                        seen.heartbeats.fetch_add(1, Ordering::Relaxed);
                        if let Some(SectionRead::Confirmed(keys)) = sections.get(f.index) {
                            on_confirm(Confirmation {
                                provider: addr,
                                keys: keys.clone(),
                                heartbeat: stamp,
                                arrival,
                                owner_time,
                            });
                        }
                    })
                    .await
                    .map_err(zenoh)?,
            );
        }
        Ok(out)
    }

    /// Replays an event's occurrences within `retention` (§2.6): a GET on
    /// every bound provider's selector, bounded by `_time`, then filtered by
    /// each occurrence's ULID time on this side, because a storage backend
    /// may ignore `_time` (spike S5: the memory backend does).
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
        let mut out = Vec::new();
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
                let Some(at) = chunks.pop().as_deref().and_then(crate::writer::ulid_time) else {
                    continue;
                };
                if at < since || !self.providers.iter().any(|p| p.matches(&addr)) {
                    continue;
                }
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                if let Some(values) = r.template.matches(&refs) {
                    let read = crate::inband::read(sample);
                    if let Err(why) = crate::inband::admit(&read, layout.as_deref()) {
                        tracing::warn!(key = %read.sample().key_expr(), reason = %why.reason(), "an undecodable occurrence is not replayed");
                        continue;
                    }
                    let (inband, tag) = (read.stamp(), read.tag());
                    let sample = read.into_sample();
                    let stamp = sample.timestamp().copied();
                    out.push(Delivery {
                        provider: addr,
                        resource: name.clone(),
                        values,
                        sample,
                        arrival: Instant::now(),
                        stamp,
                        inband,
                        tag,
                        lifespan: premise,
                    });
                }
            }
        }
        Ok(out)
    }

    /// GETs current state from its owners (S4): every bound provider's keys
    /// for `resource`, one member when `values` binds its parameters (R2's
    /// bindings apply either way), with target `All` and consolidation
    /// `Latest` set explicitly. Silence is [`StateGet::Silent`], never a
    /// verdict (S6, O5).
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

    /// [`Consumer::get`], with whether every GET ran to its final reply
    /// ([`StateAnswer::complete`]): a reader that counts members needs to
    /// know (§2.7, 0.24). An error reply, the timeout's included, is kept
    /// out of the answer and makes it incomplete.
    pub async fn get_answer(
        &self,
        resource: &str,
        values: Option<&Bindings>,
        timeout: Duration,
    ) -> Result<StateAnswer> {
        let r = self.resource(resource)?.clone();
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!("{resource:?} is not state")));
        }
        let selectors = match values {
            None => self.selectors(resource)?,
            Some(v) => {
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
                    .collect::<Result<Vec<_>>>()?
            }
        };
        let layout = self.packed_layout(&r)?;
        let mut out = Vec::new();
        let mut inband = BTreeMap::new();
        let mut tags = BTreeMap::new();
        let mut undecodable = BTreeMap::new();
        let mut complete = true;
        for ke in selectors {
            let rx = self
                .session
                .get(ke)
                .target(zenoh::query::QueryTarget::All)
                .consolidation(zenoh::query::ConsolidationMode::Latest)
                .timeout(timeout)
                .with(flume::unbounded::<zenoh::query::Reply>())
                .await
                .map_err(zenoh)?;
            while let Ok(reply) = rx.recv_async().await {
                let Ok(sample) = reply.into_result() else {
                    complete = false;
                    continue;
                };
                let key = sample.key_expr().as_str().to_owned();
                if key.contains('*') {
                    continue;
                }
                let read = crate::inband::read(sample);
                if let Err(why) = crate::inband::admit(&read, layout.as_deref()) {
                    undecodable.insert(key, why);
                    continue;
                }
                if let Some(stamp) = read.stamp() {
                    inband.insert(key.clone(), stamp);
                }
                if let Some(tag) = read.tag() {
                    tags.insert(key.clone(), tag);
                }
                let sample = read.into_sample();
                out.push(match sample.kind() {
                    zenoh::sample::SampleKind::Delete => Current::Deleted {
                        key,
                        timestamp: sample.timestamp().copied(),
                    },
                    zenoh::sample::SampleKind::Put => Current::Value {
                        key,
                        sample: Box::new(sample),
                    },
                });
            }
        }
        Ok(StateAnswer {
            current: out,
            complete,
            inband,
            tags,
            undecodable,
        })
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
        let (heartbeat, gap) = self.view_of(key, |v| {
            (
                v.confirmed(key).map(|c| now.saturating_duration_since(c)),
                v.gap(),
            )
        });
        Observation::Subscribed {
            last: zenkey_model::freshness::confirmed_last(delivered, heartbeat),
            listened: self.listened(),
            complete: !gap,
        }
    }

    /// `f` over the view of the provider whose key `key` is, or its
    /// defaults with none.
    fn view_of<R: Default>(&self, key: &str, f: impl FnOnce(&View<Instant>) -> R) -> R {
        let addr = zenkey_model::heartbeat::base_relative(key).and_then(|k| match parse(k) {
            Ok(ZkKey::Data { addr, .. }) => Some(addr),
            _ => None,
        });
        let views = self.seen.views.lock().expect("not poisoned");
        addr.and_then(|a| views.get(&a)).map(f).unwrap_or_default()
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
            .is_some_and(|v| !v.reread_due().is_empty())
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
            let view = views.entry(addr).or_insert_with(|| {
                let mut v = View::new(f.layout.clone());
                v.follow(f.index);
                v
            });
            view.reread(
                f.index,
                live.iter().map(|(k, s)| (k.as_str(), *s)),
                asked,
                now,
            );
        }
    }
}

/// The selector of `addr`'s members of the collection `f` follows
/// ([`zenkey_model::grammar::template_selector`]): base-relative, so the
/// consumer's session adds its namespace on egress and its replies come
/// back base-relative, as [`zenkey_model::heartbeat::base_relative`] reads
/// them either way.
fn collection_selector(f: &Followed, addr: &Addr) -> Option<OwnedKeyExpr> {
    let c = f.layout.collections.get(f.index)?;
    OwnedKeyExpr::try_from(zenkey_model::grammar::template_selector(
        addr,
        &f.layout.iface,
        c.token,
        &c.template,
    ))
    .ok()
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
        .consolidation(zenoh::query::ConsolidationMode::Latest)
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
    let mut live = Vec::new();
    let mut complete = true;
    while let Ok(reply) = rx.recv_async().await {
        let Ok(sample) = reply.into_result() else {
            complete = false;
            continue;
        };
        if sample.kind() != zenoh::sample::SampleKind::Put {
            continue;
        }
        let key = sample.key_expr().as_str().to_owned();
        let read = crate::inband::read(sample);
        live.push((key, read.stamp()));
    }
    let complete =
        complete && asked.elapsed() < ends_at && !crate::presence::cut_off(&session).await;
    if complete {
        let mut views = seen.views.lock().expect("not poisoned");
        if let Some(v) = views.get_mut(&addr) {
            v.reread(
                f.index,
                live.iter().map(|(k, s)| (k.as_str(), *s)),
                asked,
                Instant::now(),
            );
        }
        drop(views);
        *seen.last_reread.lock().expect("not poisoned") = Some(Instant::now());
        seen.rereads.fetch_add(1, Ordering::Relaxed);
    }
    done();
}
