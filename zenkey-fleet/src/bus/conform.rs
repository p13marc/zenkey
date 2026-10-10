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
//!    from, and whether each state GET ran to its final reply, the one
//!    complete reading of a state population.
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
/// suite (#721, PF).
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
            crate::bus::health::observe(session, &bus.namespace, target, spec, Some(o.clone()))
                .await,
        )
    };
    let suite = suite(
        bus,
        offline,
        address.clone(),
        iface,
        asked_fp,
        spec,
        presence.clone(),
        admin,
    );
    let (mut obs, health) = tokio::join!(suite, health);
    obs.health = health;
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
        calls: BTreeMap::new(),
        admin: Some(admin),
        read_at: None,
        health: None,
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
    obs
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
