//! zk2's `check conform` (#703), the reads: one service, read through a
//! session in the deployment's namespace as its consumers and callers read
//! it, against the contract revision its descriptor claims.
//!
//! What is read, in order:
//! 1. presence and every instance's descriptor (§8.1, §3.3), and from the
//!    descriptors the revision claimed and the resources exposed;
//! 2. the claimed bundle, from its holders through a store of its own, so a
//!    holder is asked even when `--contracts` holds the revision (§8.4);
//! 3. over one window, at once: a subscription on every exposed stream,
//!    state and event resource through the runtime's consumer (R1, R6); a
//!    state GET on every exposed state resource (S4's GET, which S2 says
//!    the owner answers); a call of every exposed operation the run may
//!    call — an `idempotent` one, or every one under `call_all` — with a
//!    request synthesized from the bundle; and, for a templated operation
//!    that forbids fan-out, a call over its template's wildcard (O2), whose
//!    refusal must come before any handler runs;
//! 4. at the window's end, for `freshness.v1` (#720): each member's last
//!    delivery on this host's monotonic clock, each stamping clock's offset
//!    at receipt, and this host's wall clock, which the GET replies' stamps
//!    are aged against;
//! 5. for the population budget (core §2.7, 0.24; #735): each delivery's
//!    instant on this host's monotonic clock and its stamp, per key, which
//!    a stream's and an event's members and an event's rate are counted
//!    from; whether each state GET ran to its final reply, a complete
//!    reading of a state population; and, when a templated stream or any
//!    event is exposed, whether the owner held its instance token from
//!    before the window until a lease after its end — a liveliness
//!    subscriber on its instance tokens, then a read of them, before the
//!    window opens, kept for the lease and the reply timeout after it
//!    (0.25) — which a window needs to be a complete reading of a stream's
//!    or an event's, and a rate window to be clean; and, over the same
//!    span, what this run saw of its own receive path (0.27, #753): a
//!    ticker's worst gap and its session's transports closed
//!    ([`crate::bus::stall`]), without which no window is lossless.
//!
//! **What the subscriptions see of a loss** (0.25, #740). Each one is the
//! runtime's consumer, a plain zenoh subscriber: it counts what this tool
//! fell behind on ([`Heard::lagged`]) and what passed the arrival cap, and
//! it detects no misses ([`Heard::misses_detected`] is `None`), even where
//! a resource declares miss detection. What a router dropped when it gave
//! up on this run is not in any of them: [`crate::bus::stall`] watches for
//! that. Whether a window lost no delivery is the judge's, by
//! `zenkey_model::budget::lossless`.
//!
//! Nothing is judged here: [`crate::judge::conform`] decides every case
//! from the [`ConformObservation`] this returns.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use zenkey_model::authoring::Kind;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Fanout, Resource};
use zenkey_model::freshness::{ClockMeasure, Observation};
use zenkey_model::grammar::{Addr, GRAMMAR, IfaceId};
use zenkey_model::template::{Bindings, Segment};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};

use crate::bus::contracts::BundleStore;
use crate::bus::presence::Scope;
use crate::judge::doctor::DoctorBus;
use crate::model::catalog::{ContractSet, ContractState, Observed, Revision};
use crate::model::render::Member;
use crate::model::target::Target;
use crate::report::{OperationReport, StateReport, WatchSample};

/// How many samples a resource's subscription keeps for the judge; past
/// it they are counted, and the cases say what they judged.
pub const SAMPLE_CAP: usize = 256;

/// How many deliveries' instants a resource's subscription keeps for the
/// budget (§2.7); past it, [`Heard::arrivals_capped`] says so, and a count
/// from what was kept is a lower bound still.
pub const ARRIVAL_CAP: usize = 65_536;

/// One delivery, as the budget counts it (§2.7): when it arrived, on this
/// host's monotonic clock from the subscription's declaration, and the
/// stamp it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub at: Duration,
    pub stamp: Option<crate::report::Stamp>,
}

/// What a `check conform` run waits for and may do.
#[derive(Debug, Clone, Copy)]
pub struct ConformSpec {
    /// Each read's timeout: presence, a descriptor, a retrieval, a GET, a
    /// call.
    pub timeout: Duration,
    /// How long the data resources are listened to.
    pub window: Duration,
    /// Call every exposed operation, not only the `idempotent` ones: each
    /// call is a write (`--i-know`).
    pub call_all: bool,
    /// The seed requests are synthesized with.
    pub seed: u64,
    /// Trust every admin-space answer on the operator's word (§4.2), as
    /// the doctor's `trust_admin`.
    pub trust_admin: bool,
    /// The operator's word that this tool's grants let it call the service,
    /// or that the deployment runs no access control (§5.1, 0.17). No tool
    /// can observe its grants (§11.3): without this, a present owner's
    /// silence is unobservable, never the O3 finding (O5).
    pub calls_granted: bool,
    /// The operator's word that this tool's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1). Without it, a
    /// GET reply's stamp is aged only against a clock this run measured on
    /// a live put of the same clock, and is unobservable otherwise.
    pub clocks_synced: bool,
    /// The deployment's link lease: a path gone silent withdraws the
    /// owner's token only when it runs out, so a window's owner is watched
    /// for it, and `timeout` more, after the window (core §2.7, 0.25).
    /// zenoh's default, `zenkey_model::budget::DEFAULT_LEASE`, unless the
    /// deployment lengthens it.
    pub lease: Duration,
    /// The routers' `wait_before_close`: how long a router waits on this
    /// run's full queue before it closes the transport, and with the lease
    /// the longest this run may stall and still read a window lossless
    /// (core §2.7, 0.27). zenoh's default,
    /// `zenkey_model::budget::DEFAULT_WAIT_BEFORE_CLOSE`, unless the
    /// deployment changes it.
    pub wait_before_close: Duration,
}

impl ConformSpec {
    /// The longest this run's reader may stall over a window and the lease
    /// after it (core §2.7, 0.27): `wait_before_close`, or the lease less
    /// one keep-alive interval (zenoh's default keep-alive) when shorter.
    pub fn stall_bound(&self) -> Duration {
        zenkey_model::budget::stall_bound(
            self.wait_before_close,
            self.lease,
            zenkey_model::budget::DEFAULT_KEEP_ALIVE,
        )
    }

    /// How long after a window's end the owner's instance tokens are
    /// watched: a lease, and the reply timeout for the withdrawal's
    /// transit (core §2.7, 0.25).
    pub fn presence_tail(&self) -> Duration {
        self.lease + self.timeout
    }

    /// How much of a subscription's listening counts toward a window's
    /// span: what it listened past its declaration plus the reply timeout,
    /// when it is taken to have reached the owner (core §2.7, 0.25).
    pub fn counted(&self, listened: Duration) -> Duration {
        listened.saturating_sub(self.timeout)
    }
}

/// What one resource's subscription heard in the window.
#[derive(Debug, Clone, Default)]
pub struct Heard {
    /// The samples kept, at most [`SAMPLE_CAP`].
    pub samples: Vec<WatchSample>,
    /// Samples delivered, kept or not.
    pub received: u64,
    /// Samples this tool fell behind on, discarded on a wildcard key (R6),
    /// and on a concrete key no member of the resource names.
    pub lagged: u64,
    pub discarded: u64,
    pub unresolved: u64,
    /// The key expressions subscribed, base-relative.
    pub selectors: Vec<String>,
    /// Each member delivered in the window, as a `freshness.v1`
    /// observation at the window's end (§2.5): its last delivery's age on
    /// this host's monotonic clock, and how long the subscription listened.
    pub members: BTreeMap<String, Observation>,
    /// Per stamping clock, the offset at receipt of the delivery whose
    /// stamp came closest to this host's clock: what a GET reply's stamp is
    /// trusted against (§2.6, ground 2).
    pub clocks: BTreeMap<String, ClockMeasure>,
    /// How long the subscription listened: from its declaration to the
    /// window's end. A member it never heard is judged on this (§2.5).
    pub listened: Duration,
    /// Every put delivered, by key, at most [`ARRIVAL_CAP`] in all: what a
    /// stream's or an event's population and an event's rate are counted
    /// from (§2.7). An event's key still carries its ULID.
    pub arrivals: BTreeMap<String, Vec<Arrival>>,
    /// Puts delivered past [`ARRIVAL_CAP`], not kept.
    pub arrivals_capped: u64,
    /// The misses the subscription detected (core §2.5, §2.7), or `None`
    /// when it does not detect misses — every subscription this run makes,
    /// today: the runtime's consumer is a plain zenoh subscriber.
    pub misses_detected: Option<u64>,
}

/// Whether the owner held its instance token over the whole window and a
/// lease after it (§2.7, 0.25): a window of a stream or an event is a
/// complete reading, and a rate window clean, only then.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowPresence {
    /// The instance tokens a read just before the window found.
    pub held: BTreeSet<String>,
    /// Whether that read ran to the routers' final reply (§8.1).
    pub complete: bool,
    /// That read's session was cut off while it ran (core §8.1, 0.38).
    pub cut_off_during: bool,
    /// Those tokens a liveliness subscriber, declared before the read, saw
    /// deleted by the time it was read: the window's end and [`Self::tail`].
    pub gone: BTreeSet<String>,
    /// How long after the window's end the subscriber was read. The run
    /// waits [`ConformSpec::presence_tail`] only where a window could read
    /// clean: a resource `reliable` with `block`, and a window long enough.
    pub tail: Duration,
}

impl WindowPresence {
    /// Present throughout: a complete read found a token, and none of the
    /// tokens it found was deleted by the time the subscriber was read,
    /// at least `tail` after the window's end (0.25).
    pub fn throughout(&self, tail: Duration) -> bool {
        self.complete && !self.held.is_empty() && self.gone.is_empty() && self.tail >= tail
    }
}

/// What a call over an operation's template wildcard drew (O2).
#[derive(Debug, Clone, Default)]
pub struct FanoutSeen {
    /// The key expression called, base-relative.
    pub selector: String,
    /// Value replies: the operation ran for a call it forbids.
    pub values: usize,
    /// The envelope codes of the refusals, decoded (§5.2).
    pub codes: Vec<String>,
    /// Replies that claim an envelope encoding and do not decode.
    pub malformed: Vec<String>,
    /// The transport's error replies, a timeout among them.
    pub transport: Vec<String>,
}

/// One operation, as the run treated it.
#[derive(Debug, Clone)]
pub enum OpObserved {
    /// Not called: not `idempotent`, and the run was not told to call every
    /// operation.
    NotCalled,
    /// Called, and what came back; with the fan-out probe for a templated
    /// operation that forbids fan-out.
    Called {
        call: std::result::Result<Box<OperationReport>, String>,
        fanout: Option<std::result::Result<FanoutSeen, String>>,
    },
}

/// Everything one `check conform` run read, as values.
#[derive(Debug, Clone)]
pub struct ConformObservation {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    pub address: Addr,
    pub iface: IfaceId,
    pub spec: ConformSpec,
    /// A fingerprint, or a prefix of one, the caller named: the service
    /// must claim it.
    pub asked_fp: Option<String>,
    /// The service's presence and descriptors.
    pub presence: std::result::Result<Observed, String>,
    /// The claimed bundle, as its holders served it (§8.4); `None` when no
    /// revision could be told from the descriptors.
    pub served: Option<std::result::Result<ContractState, String>>,
    /// The revision the suite decodes with: the one served, else the one
    /// `--contracts` holds.
    pub revision: Option<Arc<Revision>>,
    /// Each exposed stream, state and event resource's subscription.
    pub heard: BTreeMap<String, std::result::Result<Heard, String>>,
    /// Each exposed state resource's GET (S2).
    pub gets: BTreeMap<String, std::result::Result<StateReport, String>>,
    /// Whether each state resource's GET ran to its final reply, with no
    /// error reply: the one complete reading of its population (§2.7).
    pub gets_complete: BTreeMap<String, bool>,
    /// Whether the owner held its instance token over the window (§2.7);
    /// `None` when no templated stream or event is exposed, so nothing
    /// needed it.
    pub window_presence: Option<std::result::Result<WindowPresence, String>>,
    /// What this run's reader saw of its own receive path over the window
    /// and the watch after it — its ticker's worst gap and its session's
    /// transports closed (§2.7, 0.27); `None` when no templated stream or
    /// event is exposed, as for [`Self::window_presence`].
    pub reader_stall: Option<std::result::Result<crate::bus::stall::StallSeen, String>>,
    /// Each exposed operation.
    pub calls: BTreeMap<String, OpObserved>,
    /// The routers' admin space, read un-namespaced: an owner's own stamp
    /// proves S1 only against the routers it verified (§4.2, 0.17).
    pub admin: Option<std::result::Result<crate::judge::doctor::AdminSpace, String>>,
    /// This host's clock at the window's end: the instant every member's
    /// freshness is judged at, a GET reply's stamp aged against it
    /// (`freshness.v1` §2.6). `None` when no window ran.
    pub read_at: Option<SystemTime>,
    /// The service's `health.v1` reading (#721, PF), over the same window,
    /// when a descriptor of it lists `health.v1` (§2.7); `None` otherwise.
    pub health: Option<crate::bus::health::HealthObservation>,
    /// The service's `config.v1` reads and probes (#768, CC), over the same
    /// window, when a descriptor of it lists `config.v1` (§2.1); `None`
    /// otherwise.
    pub config: Option<ConfigConform>,
    /// The service's alarms (#800, AC), one S4 GET of its
    /// `alarms.v1/state/alarms/*`, when a descriptor of it lists
    /// `alarms.v1` (§2.13); `None` otherwise.
    pub alarms: Option<std::result::Result<crate::model::alarms::SeedRead, String>>,
    /// The service's metric members of the interface (#819, TC), read over
    /// the same window by `telemetry.v1`'s reader: each sample's
    /// conformance, and each pair's continuity with presence. `None` when
    /// the presence read failed.
    pub telemetry: Option<crate::bus::telemetry::TelemetryObservation>,
}

impl ConformObservation {
    /// The revision the served descriptors claim for the interface: one
    /// fingerprint, or why there is none to test.
    pub fn claimed(&self) -> std::result::Result<Fingerprint, String> {
        let o = self
            .presence
            .as_ref()
            .map_err(|e| format!("the presence read could not be made: {e}"))?;
        let want = self.iface.to_string();
        let fps: BTreeSet<&str> = o
            .descriptors
            .iter()
            .flatten()
            .filter_map(|(_, r)| r.descriptor())
            .flat_map(|d| d.interfaces.iter())
            .filter(|e| e.iface == want)
            .map(|e| e.contract.as_str())
            .collect();
        match fps.len() {
            0 => Err(format!("no descriptor of {} lists {want}", self.address)),
            1 => {
                let fp = fps.into_iter().next().expect("one");
                Fingerprint::parse(fp).map_err(|e| format!("{fp:?} is not a fingerprint: {e}"))
            }
            _ => Err(format!(
                "{} claims {want} at {} revisions ({}): one service, one revision under test",
                self.address,
                fps.len(),
                fps.into_iter().collect::<Vec<_>>().join(", ")
            )),
        }
    }

    /// The resources of the revision the service exposes, by its
    /// descriptors' compact rule (§3.3), across its instances.
    pub fn exposed(&self) -> Vec<&Resource> {
        let (Ok(o), Some(rev)) = (&self.presence, &self.revision) else {
            return Vec::new();
        };
        let mut names: BTreeSet<String> = BTreeSet::new();
        let mut out = Vec::new();
        for d in o
            .descriptors
            .iter()
            .flatten()
            .filter_map(|(_, r)| r.descriptor())
        {
            for r in d.exposed(rev.contract()).unwrap_or_default() {
                if names.insert(zenkey::implementation::resource_name(r)) {
                    out.push(r);
                }
            }
        }
        out
    }
}

/// One `check conform` run: [`observe`], then
/// [`crate::judge::conform::judge`].
pub async fn run_conform(
    bus: &DoctorBus,
    offline: &ContractSet,
    address: Addr,
    iface: IfaceId,
    asked_fp: Option<String>,
    spec: ConformSpec,
) -> crate::report::ConformReport {
    let obs = observe(bus, offline, address, iface, asked_fp, spec).await;
    crate::judge::conform::judge(&obs)
}

/// The reads of the module doc: the service's through `bus.session`, in
/// the deployment's namespace, and the routers' admin space through
/// `bus.raw`, in none (S1, §4.2, 0.17). When a descriptor of the service
/// lists `health.v1`, its health is read over the same window, beside the
/// suite (#721, PF); when one lists `config.v1`, its configuration is read
/// and probed over it too (#768, CC).
pub async fn observe(
    bus: &DoctorBus,
    offline: &ContractSet,
    address: Addr,
    iface: IfaceId,
    asked_fp: Option<String>,
    spec: ConformSpec,
) -> ConformObservation {
    let t = spec.timeout;
    let session = &bus.session;
    let scope = Scope::service(&address);
    let (presence, admin) = tokio::join!(
        crate::bus::presence::observe(session, &scope, t),
        crate::judge::doctor::admin_space(&bus.raw, t, spec.trust_admin),
    );
    let presence = presence.map_err(|e| crate::one_line(&e));
    let listed = match &presence {
        Ok(o) => matches!(
            crate::model::health::listing(o, &address),
            zenkey_model::health::Listing::Listed { .. }
        ),
        Err(_) => false,
    };
    let health = async {
        let Ok(o) = &presence else {
            return None;
        };
        if !listed {
            return None;
        }
        let spec = crate::bus::health::HealthSpec {
            timeout: t,
            window: Some(spec.window),
            grace: crate::judge::doctor::DEFAULT_GRACE,
            clocks_synced: spec.clocks_synced,
            face: None,
        };
        let target = crate::bus::health::HealthTarget::One(address.clone());
        Some(
            crate::bus::health::observe(
                session,
                &bus.namespace,
                target,
                spec,
                Some(o.clone()),
                |_| {},
            )
            .await,
        )
    };
    let config = async {
        let Ok(o) = &presence else {
            return None;
        };
        if crate::model::config::listing(o, &address) != crate::report::ConfigPremise::Listed {
            return None;
        }
        Some(config_suite(session, &bus.namespace, &address, o, spec).await)
    };
    let alarms = async {
        let Ok(o) = &presence else {
            return None;
        };
        let book = {
            let mut b = crate::model::alarms::AlarmBook::new(
                crate::model::alarms::AlarmScope::of(&address),
                crate::model::alarms::BookSpec::new(t),
            );
            b.presence_read(o, std::time::Instant::now());
            b
        };
        if !matches!(
            book.presence_of(&address),
            zenkey_model::alarms::Presence::Present(zenkey_model::alarms::Listing::Listed { .. })
        ) {
            return None;
        }
        let scope = crate::model::alarms::AlarmScope::of(&address);
        Some(crate::bus::alarms::get(session, scope.alarms_selector(), t).await)
    };
    let telemetry = async {
        if presence.is_err() {
            return None;
        }
        let store = BundleStore::new(t);
        store.seed(offline);
        let mut tspec = crate::bus::telemetry::TelemetrySpec::new(t, spec.window);
        tspec.lease = spec.lease;
        tspec.wait_before_close = spec.wait_before_close;
        let scope = crate::model::telemetry::TelemetryScope::of(&address).with_iface(iface.clone());
        Some(
            crate::bus::telemetry::observe(
                session,
                bus.watch.as_ref(),
                &bus.namespace,
                scope,
                &store,
                offline,
                tspec,
                |_| {},
            )
            .await,
        )
    };
    let suite = suite(
        bus,
        offline,
        address.clone(),
        iface.clone(),
        asked_fp,
        spec,
        presence.clone(),
        admin,
    );
    let (mut obs, health, config, alarms, telemetry) =
        tokio::join!(suite, health, config, alarms, telemetry);
    obs.health = health;
    obs.config = config;
    obs.alarms = alarms;
    obs.telemetry = telemetry;
    obs
}

/// The suite's reads, after presence and the admin space.
#[allow(clippy::too_many_arguments)]
async fn suite(
    bus: &DoctorBus,
    offline: &ContractSet,
    address: Addr,
    iface: IfaceId,
    asked_fp: Option<String>,
    spec: ConformSpec,
    presence: std::result::Result<Observed, String>,
    admin: std::result::Result<crate::judge::doctor::AdminSpace, String>,
) -> ConformObservation {
    let t = spec.timeout;
    let session = &bus.session;
    let mut obs = ConformObservation {
        namespace: bus.namespace.clone(),
        address,
        iface,
        spec,
        asked_fp,
        presence,
        served: None,
        revision: None,
        heard: BTreeMap::new(),
        gets: BTreeMap::new(),
        gets_complete: BTreeMap::new(),
        window_presence: None,
        reader_stall: None,
        calls: BTreeMap::new(),
        admin: Some(admin),
        read_at: None,
        health: None,
        config: None,
        alarms: None,
        telemetry: None,
    };
    let Ok(fp) = obs.claimed() else {
        return obs;
    };
    if obs
        .asked_fp
        .as_deref()
        .is_some_and(|p| !fp.hex().to_string().starts_with(p))
    {
        return obs;
    }
    // A store of its own: the holders are asked whatever --contracts holds.
    let served = BundleStore::new(t)
        .fetch(session, &obs.iface, &fp)
        .await
        .map_err(|e| crate::one_line(&e));
    obs.revision = match &served {
        Ok(ContractState::Held(r)) => Some(Arc::clone(r)),
        _ => offline.get(&obs.iface, &fp).cloned(),
    };
    obs.served = Some(served);
    let Some(rev) = obs.revision.clone() else {
        return obs;
    };
    let target = match Target::parse(&obs.address.to_string()) {
        Ok(t) => t,
        Err(_) => return obs,
    };
    let exposed: Vec<Resource> = obs.exposed().into_iter().cloned().collect();

    // §2.7: a window of a stream or an event is complete, and a rate window
    // clean, only with the owner present throughout. The subscriber first,
    // so a token the read finds and that goes later is seen going.
    let windowed = exposed
        .iter()
        .any(|r| (r.kind == Kind::Stream && r.template.has_params()) || r.kind == Kind::Event);
    let presence_watch = if windowed {
        Some(watch_instances(bus.watch.as_ref(), session, &obs.address, t).await)
    } else {
        None
    };
    // 0.27: and only with this run's own receive path never stalled long
    // enough for a router to give up on it — its ticker and its session's
    // transport events, watched as long as the owner is.
    let stall_watch = if windowed {
        Some(crate::bus::stall::StallWatch::start(session).await)
    } else {
        None
    };
    // 0.25: the watch outlasts the window by a lease and the reply timeout
    // where a window could read clean: a resource reliable with `block`
    // whose span or period the window covers. Elsewhere nothing waits on it.
    let tail = if windowed && clean_reachable(&exposed, &spec) {
        spec.presence_tail()
    } else {
        Duration::ZERO
    };

    // The subscriptions first, so what the GETs and the calls stir up is
    // heard too.
    let deadline = tokio::time::Instant::now() + spec.window;
    // The window's end on this host's wall clock: every watch observes its
    // members then, and the GET replies are aged then (freshness.v1 §2.6).
    obs.read_at = Some(SystemTime::now() + spec.window);
    let mut watches = Vec::new();
    for r in exposed.iter().filter(|r| r.kind != Kind::Operation) {
        let name = zenkey::implementation::resource_name(r);
        match crate::bus::consume::watch(session, &rev, &target, r, &Bindings::new()).await {
            Ok(w) => watches.push((name, w)),
            Err(e) => {
                obs.heard.insert(name, Err(crate::one_line(&e)));
            }
        }
    }
    let listen =
        futures_util::future::join_all(watches.into_iter().map(|(name, mut w)| async move {
            let mut heard = Heard {
                selectors: w.selectors().to_vec(),
                ..Heard::default()
            };
            let mut kept = 0usize;
            while let Ok(Some(sample)) = tokio::time::timeout_at(deadline, w.next()).await {
                heard.received += 1;
                // §2.7: a put's instant, on this host's clock, and its stamp.
                if matches!(sample.event, crate::report::WatchEvent::Put { .. }) {
                    if kept < ARRIVAL_CAP {
                        kept += 1;
                        heard
                            .arrivals
                            .entry(sample.key.clone())
                            .or_default()
                            .push(Arrival {
                                at: w.subscription().listened(),
                                stamp: sample.timestamp.clone(),
                            });
                    } else {
                        heard.arrivals_capped += 1;
                    }
                }
                if heard.samples.len() < SAMPLE_CAP {
                    heard.samples.push(sample);
                }
            }
            heard.lagged = w.lagged();
            heard.discarded = w.discarded();
            heard.unresolved = w.unresolved();
            // freshness.v1 §2.5: each member as the window's end sees it.
            let sub = w.subscription();
            heard.members = sub
                .members()
                .into_iter()
                .map(|k| {
                    let o = sub.freshness(&k);
                    (k, o)
                })
                .collect();
            heard.clocks = sub.clock_offsets();
            heard.listened = sub.listened();
            (name, heard)
        }));
    let asks = async {
        let mut gets = BTreeMap::new();
        let mut complete = BTreeMap::new();
        for r in exposed.iter().filter(|r| r.kind == Kind::State) {
            let read = crate::bus::consume::StateRead {
                revision: &rev,
                owner: &obs.address,
                resource: r,
                values: &Bindings::new(),
                timeout: t,
            };
            let name = zenkey::implementation::resource_name(r);
            let got = crate::bus::consume::get_state_answer(session, read)
                .await
                .map(|(report, done)| {
                    complete.insert(name.clone(), done);
                    report
                })
                .map_err(|e| crate::one_line(&e));
            gets.insert(name, got);
        }
        let mut calls = BTreeMap::new();
        for r in exposed.iter().filter(|r| r.kind == Kind::Operation) {
            let name = zenkey::implementation::resource_name(r);
            calls.insert(
                name,
                call(session, &rev, &target, &obs.address, r, spec).await,
            );
        }
        (gets, complete, calls)
    };
    let (heard, (gets, complete, calls)) = tokio::join!(listen, asks);
    obs.heard.extend(heard.into_iter().map(|(n, h)| (n, Ok(h))));
    obs.gets = gets;
    obs.gets_complete = complete;
    obs.calls = calls;
    if presence_watch.as_ref().is_some_and(Result::is_ok) && !tail.is_zero() {
        tokio::time::sleep_until(deadline + tail).await;
    }
    let read_at = tokio::time::Instant::now();
    obs.window_presence = presence_watch.map(|w| {
        w.map(|(mut p, gone, _subscriber)| {
            let gone = gone.lock().expect("not poisoned");
            p.gone = p.held.intersection(&gone).cloned().collect();
            p.tail = read_at.saturating_duration_since(deadline);
            p
        })
    });
    obs.reader_stall = stall_watch.map(|w| w.map(|w| w.read()));
    obs
}

/// Whether a window of this run could read clean (§2.7, 0.25): some
/// windowed resource is `reliable` with `block`, so its window can be
/// lossless, and the window counts at least its liveness span (a
/// templated stream or event) or its rate's period (an event).
fn clean_reachable(exposed: &[Resource], spec: &ConformSpec) -> bool {
    use zenkey_model::authoring::{Congestion, Reliability};
    let counted = spec.counted(spec.window);
    exposed.iter().any(|r| {
        let Body::Data(d) = &r.body else {
            return false;
        };
        if d.reliability != Reliability::Reliable || d.congestion != Congestion::Block {
            return false;
        }
        let span = r
            .template
            .has_params()
            .then(|| zenkey_model::budget::liveness(r.kind, d.retention_s.map(Duration::from_secs)))
            .flatten();
        let period = d.rate.map(|rate| rate.limit().1);
        [span, period]
            .into_iter()
            .flatten()
            .any(|need| counted >= need)
    })
}

/// What [`watch_instances`] hands back: the read's tokens, the deletes seen
/// so far, and the subscriber that keeps seeing them until dropped.
type InstanceWatch = (
    WindowPresence,
    Arc<std::sync::Mutex<BTreeSet<String>>>,
    zenoh::pubsub::Subscriber<()>,
);

/// A callback liveliness subscriber on `addr`'s instance tokens (§8.1: a
/// subscriber beside a GET is callback-driven), on `watch`, a session of
/// its own, and settled there; then a read of them on `session`, which
/// holds no liveliness subscriber (#830, core §8.1, 0.37): the tokens held
/// when the window opens, and every one deleted from then on.
async fn watch_instances(
    watch: Option<&Session>,
    session: &Session,
    addr: &Addr,
    timeout: Duration,
) -> std::result::Result<InstanceWatch, String> {
    let selector = format!("{GRAMMAR}/{}/{}/@zk/instance/*", addr.system, addr.service);
    let Some(watch) = watch.filter(|w| w.zid() != session.zid()) else {
        return Err(format!(
            "watching `{selector}` needs a watch session of its own: no presence read is made \
             beside a liveliness subscriber (#830, core §8.1)"
        ));
    };
    let gone: Arc<std::sync::Mutex<BTreeSet<String>>> = Arc::default();
    let g = Arc::clone(&gone);
    let subscriber = zenkey::presence::declare_subscriber(watch, &selector)
        .callback(move |sample| {
            if sample.kind() == zenoh::sample::SampleKind::Delete {
                g.lock()
                    .expect("not poisoned")
                    .insert(sample.key_expr().as_str().to_owned());
            }
        })
        .await
        .map_err(|e| format!("a liveliness subscriber on `{selector}`: {e}"))?;
    // The routers have read the subscriber before the read below, on the
    // other session, is answered (#830).
    if !zenkey::presence::settle(watch, &selector, timeout)
        .await
        .unwrap_or(false)
    {
        return Err(format!(
            "the liveliness subscriber on `{selector}` did not settle within the timeout"
        ));
    }
    let read = zenkey::presence::liveliness_read(session, &selector, timeout)
        .await
        .map_err(|e| format!("the read of `{selector}`: {e}"))?;
    Ok((
        WindowPresence {
            complete: read.complete,
            cut_off_during: read.cut_off_during,
            held: read.keys.into_iter().collect(),
            gone: BTreeSet::new(),
            tail: Duration::ZERO,
        },
        gone,
        subscriber,
    ))
}

/// A member value per template parameter: what a concrete call to a
/// templated operation names. The owner may hold no such member; its
/// refusal (`not_found`) is an answer all the same.
fn member(r: &Resource) -> Bindings {
    r.template
        .params()
        .map(|(name, _)| (name.to_owned(), vec!["conform".to_owned()]))
        .map(|(name, mut v)| {
            if r.params.get(&name) == Some(&zenkey_model::authoring::ParamType::Uint) {
                v = vec!["0".to_owned()];
            }
            (name, v)
        })
        .collect()
}

/// One operation: called when the run may (an `idempotent` one, or every
/// one under `call_all`), with a request synthesized from the bundle; and
/// for a templated one that forbids fan-out, called over its wildcard too.
async fn call(
    session: &Session,
    rev: &Revision,
    target: &Target,
    addr: &Addr,
    r: &Resource,
    spec: ConformSpec,
) -> OpObserved {
    let Body::Operation(op) = &r.body else {
        return OpObserved::NotCalled;
    };
    if !(op.idempotent || spec.call_all) {
        return OpObserved::NotCalled;
    }
    let name = zenkey::implementation::resource_name(r);
    let request = crate::tape::synth::Synth::new(spec.seed)
        .sample(rev, r, Member::Request, 0)
        .map(|s| s.bytes);
    let call = match &request {
        Err(why) => Err(format!("no request could be synthesized: {why}")),
        Ok(bytes) => match crate::model::target::plan_call(rev, target.clone(), &name, member(r)) {
            Err(e) => Err(crate::one_line(&e)),
            Ok(plan) => crate::bus::operation::call(
                session,
                crate::bus::operation::OperationCall {
                    revision: rev,
                    plan: &plan,
                    request: bytes.clone(),
                    timeout: spec.timeout,
                    retries: 0,
                },
            )
            .await
            .map(Box::new)
            .map_err(|e| crate::one_line(&e)),
        },
    };
    let fanout = (r.template.has_params() && op.fanout != Fanout::Allowed).then(|| {
        let bytes = request.clone().unwrap_or_default();
        (rev.iface().clone(), bytes)
    });
    let fanout = match fanout {
        Some((iface, bytes)) => {
            Some(fanout_probe(session, addr, &iface, r, bytes, spec.timeout).await)
        }
        None => None,
    };
    OpObserved::Called { call, fanout }
}

/// A call over the template's wildcard (O2): target `All`, consolidation
/// `None`, every reply kept apart — a value (the operation ran), an
/// envelope (decoded, §5.2), the transport's own error.
async fn fanout_probe(
    session: &Session,
    addr: &Addr,
    iface: &IfaceId,
    r: &Resource,
    request: Vec<u8>,
    timeout: Duration,
) -> std::result::Result<FanoutSeen, String> {
    let mut chunks: Vec<String> = vec![
        GRAMMAR.to_owned(),
        addr.system.to_string(),
        addr.service.to_string(),
        iface.to_string(),
        "@op".to_owned(),
    ];
    for seg in r.template.segments() {
        chunks.push(match seg {
            Segment::Literal(l) => l.clone(),
            Segment::Param(_) | Segment::Rest(_) => "*".to_owned(),
        });
    }
    let selector = chunks.join("/");
    let replies = session
        .get(&selector)
        .payload(request)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut seen = FanoutSeen {
        selector,
        ..FanoutSeen::default()
    };
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(_) => seen.values += 1,
            Err(e) => {
                let encoding = e.encoding().to_string();
                let bytes = e.payload().to_bytes();
                match zenkey_model::envelope::decode(&encoding, &bytes) {
                    Ok(env) => seen.codes.push(env.code),
                    Err(zenkey_model::envelope::EnvelopeError::Encoding(_)) => seen
                        .transport
                        .push(format!("{encoding}: {}", String::from_utf8_lossy(&bytes))),
                    Err(err) => seen.malformed.push(format!("{encoding}: {err}")),
                }
            }
        }
    }
    Ok(seen)
}

// ─── config.v1 (#768, CC) ───────────────────────────────────────────────────

/// The token a control probe carries: one no owner mints (the reference
/// mints `<instance>-<n>`, §2.6), so it names nothing.
pub const PROBE_TOKEN: &str = "zk2-conform-probe";

/// The revision a refusal probe expects: one no owner reaches in an
/// instance (an act at a time, 600 changes an hour, §2.17), so even an
/// owner that skipped the row under test refuses it at row 7.
pub const PROBE_REVISION: u64 = 1 << 40;

/// What the text says a probe is answered (§2.5, §2.9, §2.10, §2.13).
#[derive(Debug, Clone)]
pub enum ProbeExpect {
    /// A refusal: one of these codes, each with the `unavailable` cause it
    /// carries, and the rule that names it.
    Refused {
        codes: &'static [(&'static str, Option<&'static str>)],
        rule: &'static str,
    },
    /// A call over the template's wildcard: `fanout_forbidden`, before any
    /// handler runs (core O2).
    Fanout,
    /// Nothing: the probe was not made.
    Nothing,
    /// A dry run of `parameter`'s running `value` in `group`: `applied`,
    /// the would-be view against `view`, the one read first.
    DryRun {
        view: Box<zenkey_model::config::ConfigView>,
        group: String,
        parameter: String,
        value: zenkey_model::config::Value,
    },
}

/// What a probe drew.
#[derive(Debug, Clone)]
pub enum ProbeOutcome {
    /// Not made, and why: the reason is the case's.
    NotMade(String),
    /// A call through the runtime's client, and its reply.
    Call(std::result::Result<Box<crate::report::ConfigActReport>, String>),
    /// A call over the wildcard, every reply kept apart.
    Fanout(std::result::Result<FanoutSeen, String>),
}

/// One probe of `config.v1`, the case it answers and its subject.
#[derive(Debug, Clone)]
pub struct ConfigProbe {
    pub case: crate::report::CaseId,
    /// `@op/<operation>`, as the case's subject.
    pub subject: String,
    pub expect: ProbeExpect,
    pub outcome: ProbeOutcome,
}

/// What `check conform` read of a service's `config.v1` (#768, CC): a GET
/// as the window opens, the reading over the window (its subscription and
/// the GET at its end), and the probes, made while the window listens.
#[derive(Debug, Clone)]
pub struct ConfigConform {
    pub first: std::result::Result<crate::bus::config::ConfigGet, String>,
    pub reading: crate::bus::config::ConfigObservation,
    pub probes: Vec<ConfigProbe>,
}

/// Whether `address`'s descriptors list `config.v1`'s `persist` unavailable
/// with cause `config` (§2.13, core §3.3).
fn persist_unavailable(presence: &Observed, address: &Addr) -> bool {
    presence
        .descriptors
        .iter()
        .flatten()
        .filter(|((a, _), _)| a == address)
        .filter_map(|(_, r)| r.descriptor())
        .flat_map(|d| d.interfaces.iter())
        .filter(|e| e.iface == zenkey_model::config::IFACE)
        .flat_map(|e| e.unavailable.iter())
        .any(|u| {
            u.resource == zenkey::config_v1::PERSIST
                && u.cause == zenkey_model::descriptor::Cause::Config
        })
}

/// One probe through the runtime's client, with the call metadata a tool
/// claims (core O7).
async fn probe_call(
    session: &Session,
    namespace: &str,
    address: &Addr,
    op: crate::bus::config::ConfigOp,
    body: serde_json::Value,
    timeout: Duration,
) -> ProbeOutcome {
    let call = crate::bus::config::ConfigCall {
        namespace,
        address,
        op,
        request: body.clone(),
        shown: body,
        secrets: Vec::new(),
        sensitive: BTreeSet::new(),
        metadata: Some(zenkey::CallMetadata {
            actor: Some("zenctl check conform".to_owned()),
            request_id: Some(zenkey::writer::ulid(SystemTime::now())),
        }),
        timeout,
        across_face: false,
        checked: None,
    };
    ProbeOutcome::Call(
        crate::bus::config::call(session, call)
            .await
            .map(Box::new)
            .map_err(|e| crate::one_line(&e)),
    )
}

/// The probes of `config.v1` the text makes checkable without changing
/// anything, against the view read first. Each refusal probe is a dry run
/// against [`PROBE_REVISION`]; a probe a broken owner could act on (a
/// control operation while a change is pending, `persist` while a last
/// change exists) is made only under `call_all` (`--i-know`).
async fn config_probes(
    session: &Session,
    namespace: &str,
    address: &Addr,
    presence: &Observed,
    view: Option<&zenkey_model::config::ConfigView>,
    spec: ConformSpec,
) -> Vec<ConfigProbe> {
    use crate::bus::config::ConfigOp;
    use crate::report::CaseId;
    let t = spec.timeout;
    let refusal_body = serde_json::json!({
        "values": {}, "dry_run": true, "expected_revision": PROBE_REVISION,
    });
    let mut out = Vec::new();
    let probe = |case, subject: String, expect, outcome| ConfigProbe {
        case,
        subject,
        expect,
        outcome,
    };
    const NOT_FOUND: &[(&str, Option<&str>)] = &[("not_found", None)];
    const UNAVAILABLE: &[(&str, Option<&str>)] = &[("unavailable", Some("config"))];
    const NO_LAST: &[(&str, Option<&str>)] =
        &[("not_found", None), ("unavailable", Some("config"))];
    let controls = [
        (ConfigOp::Confirm, "@op/confirm"),
        (ConfigOp::Cancel, "@op/cancel"),
        (ConfigOp::Extend, "@op/extend"),
    ];
    match view {
        None => {
            let why = "no view was read as the window opened: the groups and the state a probe \
                       needs are the view's";
            for subject in [
                "@op/set/{group}: a group the schema does not declare",
                "@op/set/{group}: a group outside the allowlist",
                "@op/confirm",
                "@op/cancel",
                "@op/extend",
                "@op/persist",
            ] {
                out.push(probe(
                    CaseId::ConfigRefusal,
                    subject.to_owned(),
                    ProbeExpect::Nothing,
                    ProbeOutcome::NotMade(why.to_owned()),
                ));
            }
            out.push(probe(
                CaseId::ConfigDryRun,
                "@op/set/{group}".to_owned(),
                ProbeExpect::Nothing,
                ProbeOutcome::NotMade(why.to_owned()),
            ));
        }
        Some(v) => {
            let unknown = (0..)
                .map(|i| match i {
                    0 => "zk2-conform-none".to_owned(),
                    i => format!("zk2-conform-none-{i}"),
                })
                .find(|n| !v.groups.iter().any(|g| &g.name == n))
                .expect("a name the view does not declare");
            out.push(probe(
                CaseId::ConfigRefusal,
                format!("@op/set/{unknown}"),
                ProbeExpect::Refused {
                    codes: NOT_FOUND,
                    rule: "a set on a group the schema does not declare is refused not_found \
                           (config.v1 §2.5, row 3)",
                },
                probe_call(
                    session,
                    namespace,
                    address,
                    ConfigOp::Set {
                        group: unknown.clone(),
                    },
                    refusal_body.clone(),
                    t,
                )
                .await,
            ));
            let shut = v.groups.iter().find(|g| !g.writable);
            out.push(probe(
                CaseId::ConfigRefusal,
                match shut {
                    Some(g) => format!("@op/set/{}", g.name),
                    None => "@op/set/{group}: a group outside the allowlist".to_owned(),
                },
                ProbeExpect::Refused {
                    codes: UNAVAILABLE,
                    rule: "a set on a group outside the startup allowlist is refused \
                           unavailable, cause config, before its values are read (config.v1 \
                           §2.5, row 4; §2.16)",
                },
                match shut {
                    Some(g) => {
                        probe_call(
                            session,
                            namespace,
                            address,
                            ConfigOp::Set {
                                group: g.name.clone(),
                            },
                            refusal_body.clone(),
                            t,
                        )
                        .await
                    }
                    None => ProbeOutcome::NotMade(
                        "every group the view declares is writable: no allowlist to probe".into(),
                    ),
                },
            ));
            let pending = v.pending.is_some();
            for (op, subject) in controls {
                let mut body = serde_json::json!({"token": PROBE_TOKEN});
                if op == ConfigOp::Extend {
                    body["confirm_s"] = serde_json::json!(1);
                }
                let outcome = if pending && !spec.call_all {
                    ProbeOutcome::NotMade(
                        "a change is pending, and an owner that skipped its token check would act \
                         on it: a write, made only under --i-know"
                            .into(),
                    )
                } else {
                    probe_call(session, namespace, address, op, body, t).await
                };
                out.push(probe(
                    CaseId::ConfigRefusal,
                    subject.to_owned(),
                    ProbeExpect::Refused {
                        codes: NOT_FOUND,
                        rule: "a control operation whose token names no pending change is \
                               refused not_found (config.v1 §2.10)",
                    },
                    outcome,
                ));
            }
            let body = serde_json::json!({"token": PROBE_TOKEN});
            let (expect, outcome) = if persist_unavailable(presence, address) {
                (
                    ProbeExpect::Refused {
                        codes: UNAVAILABLE,
                        rule: "persist, listed unavailable with cause config, is answered \
                               unavailable with that cause before any handler runs (config.v1 \
                               §2.13, core O3)",
                    },
                    probe_call(session, namespace, address, ConfigOp::Persist, body, t).await,
                )
            } else {
                let expect = ProbeExpect::Refused {
                    codes: NO_LAST,
                    rule: "persist with a token that is not the last change's is refused \
                           not_found, or unavailable, cause config, by a persisted layer that \
                           cannot be written now (config.v1 §2.13)",
                };
                if v.last_change.is_some() && !spec.call_all {
                    (
                        expect,
                        ProbeOutcome::NotMade(
                            "a last change exists, and an owner that skipped its token check \
                             would persist it: a write, made only under --i-know"
                                .into(),
                        ),
                    )
                } else {
                    (
                        expect,
                        probe_call(session, namespace, address, ConfigOp::Persist, body, t).await,
                    )
                }
            };
            out.push(probe(
                CaseId::ConfigRefusal,
                "@op/persist".to_owned(),
                expect,
                outcome,
            ));
            let candidate = v
                .groups
                .iter()
                .filter(|g| g.writable && g.class == zenkey_model::config::Class::Hot)
                .find_map(|g| {
                    g.parameters
                        .iter()
                        .filter(|p| !p.sensitive)
                        .find_map(|p| p.value.clone().map(|val| (g, p, val)))
                });
            match (pending, candidate) {
                (true, _) => out.push(probe(
                    CaseId::ConfigDryRun,
                    "@op/set/{group}".to_owned(),
                    ProbeExpect::Nothing,
                    ProbeOutcome::NotMade(
                        "a change is pending: a dry run is busy then (config.v1 §2.5, row 5)"
                            .into(),
                    ),
                )),
                (false, None) => out.push(probe(
                    CaseId::ConfigDryRun,
                    "@op/set/{group}".to_owned(),
                    ProbeExpect::Nothing,
                    ProbeOutcome::NotMade("no hot, writable group shows a value to run dry".into()),
                )),
                (false, Some((g, p, value))) => {
                    let body = serde_json::json!({
                        "values": {p.name.clone(): value.clone()}, "dry_run": true,
                    });
                    let outcome = probe_call(
                        session,
                        namespace,
                        address,
                        ConfigOp::Set {
                            group: g.name.clone(),
                        },
                        body,
                        t,
                    )
                    .await;
                    out.push(probe(
                        CaseId::ConfigDryRun,
                        format!("@op/set/{}", g.name),
                        ProbeExpect::DryRun {
                            view: Box::new(v.clone()),
                            group: g.name.clone(),
                            parameter: p.name.clone(),
                            value,
                        },
                        outcome,
                    ));
                }
            }
        }
    }
    let set = zenkey::config_v1::implementation()
        .contract()
        .resources
        .iter()
        .find(|r| zenkey::implementation::resource_name(r) == zenkey::config_v1::SET)
        .cloned()
        .expect("config.v1 declares set/{group}");
    let bytes = serde_json::to_vec(&refusal_body).expect("a JSON body");
    out.push(probe(
        CaseId::ConfigRefusal,
        "@op/set/*".to_owned(),
        ProbeExpect::Fanout,
        ProbeOutcome::Fanout(
            fanout_probe(
                session,
                address,
                &crate::bus::config::iface(),
                &set,
                bytes,
                t,
            )
            .await,
        ),
    ));
    out
}

/// `config.v1`'s reads and probes for `check conform` (#768, CC): a GET as
/// the window opens, then the reading over the window and the probes, at
/// once.
async fn config_suite(
    session: &Session,
    namespace: &str,
    address: &Addr,
    presence: &Observed,
    spec: ConformSpec,
) -> ConfigConform {
    let t = spec.timeout;
    let selector = crate::bus::config::ConfigTarget::One(address.clone()).state_selector(false);
    let first = crate::bus::config::get(session, &selector, t).await;
    let view = first
        .as_ref()
        .ok()
        .and_then(|g| g.services.get(address))
        .and_then(|d| d.view.as_ref())
        .and_then(|r| r.value.clone())
        .and_then(|v| v.ok());
    let cspec = crate::bus::config::ConfigSpec {
        window: Some(spec.window),
        clocks_synced: spec.clocks_synced,
        ..crate::bus::config::ConfigSpec::new(t)
    };
    let reading = crate::bus::config::observe(
        session,
        namespace,
        address.clone(),
        cspec,
        Some(presence.clone()),
    );
    let probes = config_probes(session, namespace, address, presence, view.as_ref(), spec);
    let (reading, probes) = tokio::join!(reading, probes);
    ConfigConform {
        first,
        reading,
        probes,
    }
}

#[cfg(test)]
mod watch_tests {
    /// A window's owner is watched on a watch session of its own (#830):
    /// with none, or the session the run reads through, the watch is
    /// refused before anything is declared, and says why.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_window_watch_needs_a_watch_session_of_its_own() {
        let mut c = zenoh::Config::default();
        c.insert_json5("mode", "\"peer\"").unwrap();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("listen/endpoints", "[]").unwrap();
        let s = zenoh::open(c).await.expect("a session");
        let addr: zenkey_model::grammar::Addr = "h1/tc".parse().unwrap();
        for watch in [None, Some(&s)] {
            let refused = super::watch_instances(watch, &s, &addr, std::time::Duration::ZERO)
                .await
                .map(|_| ())
                .expect_err("refused");
            assert!(refused.contains("watch session"), "{refused}");
        }
        assert!(!zenkey::presence::watching(&s), "nothing declared");
        s.close().await.unwrap();
    }
}
