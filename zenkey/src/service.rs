//! A service's life (spec §1.5, §3.3, §8.1–§8.2): declare resources, come
//! up in the order that makes alive ⇒ callable, keep the descriptor
//! current, re-mint the instance id make-before-break, and hold member
//! tokens.
//!
//! ```text
//! ServiceBuilder::new ─ the address resolved: a minted system, once per run
//!        │              (hostid.v1 §2.7); `self.system/<service>` providers
//!        │              spelled out (R1, 0.20). A failure declares nothing.
//!        │
//!        ├─ implement / require / declare_* / expose / unavailable
//!        │
//!        ▼ start()                                          (§8.2)
//!   1. resources: already declared on the builder
//!   2. every required resource exposed; every optional one exposed,
//!      unavailable, or implied absent by a missing capability
//!   3. the descriptor (checked, put, served) and one contract queryable
//!      per implemented interface
//!   4. the instance token, then the interface tokens
//!        │
//!        ▼
//!     Service ─ set_capabilities / new_epoch / declare_member / cycle_member
//! ```
//!
//! A service implementing `health.v1` through [`ServiceBuilder::health`]
//! has its status put before step 3, as `health.v1` §2.3 makes a MUST
//! ([`crate::health`]); one implementing `config.v1` through
//! [`ServiceBuilder::config_v1`] has its view and status put when it
//! declares them, as `config.v1` §2.3 makes a MUST ([`crate::config_v1`]);
//! one implementing `alarms.v1` through [`ServiceBuilder::alarms`] adopts
//! its previous instance's alarms there, putting each again before
//! anything is alive (`alarms.v1` §2.9, [`crate::alarms`]); an author of
//! `desired.v1` documents through [`ServiceBuilder::desired_author`] puts
//! again there what its record keeps (`desired.v1` §2.11), and a target
//! through [`ServiceBuilder::desired_target`] puts its baseline report
//! there, then reads from step 4 on, after the tokens ([`crate::desired`]).
//! One electing its governed interfaces through
//! [`ServiceBuilder::redundancy`] claims at steps 3 and 4, each governed
//! interface marked standby and holding no token, then elects; it takes
//! over in steps 1 to 4's order and steps down in their reverse
//! ([`crate::redundancy`]).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use zenkey_model::descriptor::Cause;
use zenkey_model::grammar::{
    Addr, IfaceId, InstanceId, Key, KindToken, alive_key, contract_key, data_key, instance_key,
    member_key,
};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::{Bindings, Segment};
use zenoh::Wait;
use zenoh::bytes::Encoding;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::liveliness::LivelinessToken;
use zenoh::pubsub::Publisher;
use zenoh::query::{Query, Queryable};

use crate::budget::Budget;
use crate::config::ServiceConfig;
use crate::consumer::Consumer;
use crate::descriptor;
use crate::error::{Error, Result, zenoh};
use crate::gate::{Gate, QSlot};
use crate::hostid::{HostIdError, HostIdMinter, Minted};
use crate::implementation::{Implementation, missing_capability, resource_name};
use crate::qos;
use crate::state::{ClockGuard, DEFAULT_WINDOW, Minter, StateWriter, Store};
use crate::telemetry::{Epochs, Meter, Metric};
use crate::writer::{EventWriter, Writer};

/// One implemented interface, and what this instance does with it.
#[derive(Debug, Clone)]
pub(crate) struct ImplState {
    pub(crate) imp: Implementation,
    /// Resources declared (or exposed for later), by descriptor name.
    pub(crate) exposed: BTreeSet<String>,
    /// Optional resources absent here, by descriptor name (§3.3).
    pub(crate) unavailable: BTreeMap<String, (Cause, Option<String>)>,
    /// Lowered template bounds, keyed `<kind token>/<template>` (§3.3).
    pub(crate) cardinality: BTreeMap<String, u64>,
    /// Each stream, state and event resource's budget (§2.7, #741), keyed
    /// as `cardinality`: shared by every writer of the resource.
    pub(crate) budgets: BTreeMap<String, Arc<Budget>>,
    /// Each metric resource's declaration (`telemetry.v1`, #819), keyed as
    /// `cardinality`: what every writer of it checks.
    pub(crate) metrics: BTreeMap<String, Arc<Metric>>,
    /// A governed interface's gate (`redundancy.v1` §2.4, #841): every
    /// resource of it declared only while the instance serves it.
    pub(crate) gate: Option<Arc<Gate>>,
}

impl ImplState {
    /// The budget every writer of `r` keeps (§2.7).
    fn budget(&self, r: &zenkey_model::contract::Resource) -> Option<Arc<Budget>> {
        self.budgets.get(&resource_name(r)).cloned()
    }

    /// The `telemetry.v1` meter of a writer of `r` on the member `values`
    /// binds, for a metric resource.
    fn meter(
        &self,
        r: &zenkey_model::contract::Resource,
        values: &Bindings,
        epochs: &Arc<Epochs>,
    ) -> Option<Meter> {
        let m = self.metrics.get(&resource_name(r))?;
        Some(Meter::new(
            Arc::clone(m),
            Arc::clone(epochs),
            self.imp.iface(),
            values,
        ))
    }
}

/// A role through which data reaches this component (§3.1). Its bindings
/// come from the configuration (R1).
#[derive(Debug, Clone)]
pub(crate) struct Role {
    pub(crate) role: String,
    pub(crate) interface: IfaceId,
    /// The contract that declares it, or `None` for the component's
    /// manifest (§3.3).
    pub(crate) declared_by: Option<IfaceId>,
    pub(crate) optional: bool,
}

/// A service being declared. Nothing is visible on the bus as alive until
/// [`ServiceBuilder::start`].
pub struct ServiceBuilder {
    session: zenoh::Session,
    /// Its `self.system` providers spelled out (R1, 0.20) once the address
    /// is resolved.
    config: ServiceConfig,
    /// The address, resolved once when the builder is made (`hostid.v1`
    /// §2.7), or why it could not be: then nothing is declared, and
    /// [`ServiceBuilder::start`] returns why.
    addr: std::result::Result<Addr, Arc<HostIdError>>,
    /// How the system was minted, for a minted address (§2.8).
    minted: Option<Minted>,
    /// The host name a minted system states as `meta.host` (§2.13).
    host: Option<String>,
    instance: InstanceId,
    /// The epochs its metric writers write (`telemetry.v1` §2.9), the
    /// instance id first.
    epochs: Arc<Epochs>,
    impls: Vec<ImplState>,
    roles: Vec<Role>,
    /// Shared with its operation servers (O3, #621).
    ops: Arc<crate::operation::Availability>,
    minter: Arc<Minter>,
    store: Arc<Store>,
    /// Interfaces whose state this service answers GETs for (S2).
    state_ifaces: BTreeSet<IfaceId>,
    /// Set when the service closes, and dropped with it: its state
    /// writers' refreshers stop on either (`freshness.v1` §2.4).
    closed: tokio::sync::watch::Sender<bool>,
    /// `health.v1`, once [`ServiceBuilder::health`] implemented it.
    pub(crate) health: Option<crate::health::Health>,
    /// `config.v1`, once [`ServiceBuilder::config_v1`] implemented it.
    pub(crate) config_v1: Option<crate::config_v1::Configuration>,
    /// `alarms.v1`, once [`ServiceBuilder::alarms`] implemented it.
    pub(crate) alarms: Option<crate::alarms::Alarms>,
    /// `desired.v1`'s desired resources this service authors.
    pub(crate) desired_authors: Vec<crate::desired::Author>,
    /// `desired.v1`'s roles this service holds as a target.
    pub(crate) desired_targets: Vec<crate::desired::Target>,
    /// Why a role was refused at its declaration (`desired.v1` §2.1, text
    /// 0.7, F-224): [`ServiceBuilder::start`] then refuses to start, as
    /// §8.2 step 2 refuses an invalid declaration, so a service never
    /// starts without the role it was given.
    pub(crate) refused: Vec<String>,
    /// `redundancy.v1`, once [`ServiceBuilder::redundancy`] configured it.
    redundancy: Option<crate::redundancy::Redundancy>,
}

/// Why a profile's owner the runtime implements is refused on a governed
/// interface (`redundancy.v1` §2.4, #841): it writes its keys before the
/// instance is elected, at its start or its re-mint, and outside the gate a
/// standby's writes go through, so a standby would write them.
fn not_elected(what: &str, iface: &IfaceId) -> String {
    format!(
        "{what} on {iface}, which redundancy.v1 governs: it writes the interface's keys before \
         any election and outside its gate, so a standby would write them; not at this version \
         (redundancy.v1 §2.4)"
    )
}

fn mint() -> InstanceId {
    InstanceId::from_u64(rand::random())
}

/// Says how a service's system was minted, at every start (`hostid.v1`
/// §2.6): an ephemeral one as a warning, with every path tried.
fn log_minted(addr: &Addr, m: &Minted) {
    let trail = crate::hostid::Trail(m.trail());
    if m.is_ephemeral() {
        tracing::warn!(
            address = %addr,
            inputs = %trail,
            "hostid.v1: this system is EPHEMERAL: no input gave an id and the shared file \
             was not created, so it lives for this run only and another start mints another \
             (hostid.v1 §2.6)"
        );
    } else {
        tracing::info!(
            address = %addr,
            from = m.source().unwrap_or("?"),
            inputs = %trail,
            "hostid.v1: the system is minted from this host's id"
        );
    }
}

/// Finds the state of `iface`.
fn find<'s>(impls: &'s [ImplState], iface: &IfaceId) -> Result<&'s ImplState> {
    impls
        .iter()
        .find(|s| s.imp.iface() == iface)
        .ok_or_else(|| Error::NotImplemented(iface.clone()))
}

fn find_mut<'s>(impls: &'s mut [ImplState], iface: &IfaceId) -> Result<&'s mut ImplState> {
    impls
        .iter_mut()
        .find(|s| s.imp.iface() == iface)
        .ok_or_else(|| Error::NotImplemented(iface.clone()))
}

impl ServiceBuilder {
    /// Starts declaring the service `config.address` on `session`, with a
    /// freshly minted instance id (§1.5).
    ///
    /// A minted address (`@hostid.v1/<service>`) is resolved here, through
    /// the process's minter ([`HostIdMinter::global`]): at most once per
    /// run (`hostid.v1` §2.7), and before anything is declared. When it
    /// cannot be, every method that needs the address returns why, and so
    /// does [`ServiceBuilder::start`]: the service declares nothing (§2.6).
    /// [`ServiceConfig::resolve`] mints the same system before the session
    /// opens, where the caller can.
    #[must_use]
    pub fn new(session: &zenoh::Session, config: ServiceConfig) -> Self {
        Self::with_hostid(session, config, HostIdMinter::global())
    }

    /// As [`ServiceBuilder::new`], minting through `hostid` instead of the
    /// process's minter: a test's, on a root of its own.
    #[must_use]
    pub fn with_hostid(
        session: &zenoh::Session,
        mut config: ServiceConfig,
        hostid: &HostIdMinter,
    ) -> Self {
        let window = config
            .tombstone_window_s
            .map_or(DEFAULT_WINDOW, std::time::Duration::from_secs);
        let (addr, minted) = match config.resolve_with(hostid) {
            Ok(r) => (Ok(r.address), r.minted),
            Err(e) => (Err(Arc::new(e)), None),
        };
        if let Ok(a) = &addr {
            // R1 (0.20): `self.system/<service>`, resolved once, as the
            // address is.
            for b in config.bindings.values_mut() {
                *b = b.resolved(&a.system);
            }
        }
        if let (Ok(a), Some(m)) = (&addr, &minted) {
            log_minted(a, m);
        }
        let host = minted.as_ref().and_then(|_| crate::hostid::hostname());
        let instance = mint();
        Self {
            session: session.clone(),
            config,
            addr,
            minted,
            host,
            epochs: Epochs::new(instance.clone()),
            instance,
            impls: Vec::new(),
            roles: Vec::new(),
            ops: Arc::default(),
            minter: Arc::new(Minter::new(session)),
            store: Arc::new(Store::new(window)),
            state_ifaces: BTreeSet::new(),
            closed: tokio::sync::watch::Sender::new(false),
            health: None,
            config_v1: None,
            alarms: None,
            desired_authors: Vec::new(),
            desired_targets: Vec::new(),
            refused: Vec::new(),
            redundancy: None,
        }
    }

    /// Takes part in `redundancy.v1`'s election for every governed
    /// interface this service implements, now or later
    /// ([`crate::redundancy`], #841): an interface whose contract lists
    /// `redundancy.v1` in `uses`. [`ServiceBuilder::start`] then claims
    /// (its descriptor marks each governed interface standby, its instance
    /// token), and elects on `startup`'s sessions: the instance serves an
    /// interface only once elected, and stands by otherwise, declaring
    /// none of its resources and writing none of its keys. The handle
    /// returned says each interface's role.
    ///
    /// Refused when it is configured already, when `startup`'s reader is
    /// this service's session or a watch session, or its watch is the
    /// service's session or the reader (§2.5 step 1, §2.7; text 0.2,
    /// F-204). A governed interface without it is refused at start.
    pub fn redundancy(
        &mut self,
        startup: crate::redundancy::RedundancyStartup,
    ) -> Result<crate::redundancy::Redundancy> {
        if self.redundancy.is_some() {
            return Err(Error::Contract(
                "redundancy.v1 is configured twice on this service".to_owned(),
            ));
        }
        let r = crate::redundancy::Redundancy::new(&self.session, startup)?;
        self.redundancy = Some(r.clone());
        Ok(r)
    }

    /// A test's hook: the instance id this service comes up with, in place
    /// of the one minted (core §1.5 mints one at every start, which a
    /// deployment never chooses). `redundancy.v1`'s scenarios order their
    /// instances by it (`spec/profiles/redundancy/scenarios.md`).
    #[doc(hidden)]
    pub fn set_instance_id(&mut self, id: InstanceId) -> &mut Self {
        self.epochs.set_instance(id.clone());
        self.instance = id;
        self
    }

    /// A governed interface's gate, when `iface` is governed and
    /// implemented.
    pub(crate) fn gate_of(&self, iface: &IfaceId) -> Option<Arc<Gate>> {
        find(&self.impls, iface).ok().and_then(|s| s.gate.clone())
    }

    /// Refuses a raw zenoh declaration on a governed interface, which the
    /// runtime could not undeclare while it stands by (`redundancy.v1`
    /// §2.4).
    fn not_governed(&self, iface: &IfaceId, what: &str) -> Result<()> {
        if self.gate_of(iface).is_some() {
            return Err(Error::Contract(format!(
                "{iface} is governed by redundancy.v1: declare its {what} through the runtime's \
                 writers and servers, which a standby holds undeclared (redundancy.v1 §2.4)"
            )));
        }
        Ok(())
    }

    /// Implements `health.v1`, its standard contract embedded
    /// ([`crate::health`]): the status's writer is declared, the checks'
    /// template exposed and the faults' writer declared, and the handle
    /// returned puts them as the profile says. [`ServiceBuilder::start`]
    /// puts the status before step 3, `UNSPECIFIED` with reason
    /// `"starting"` unless [`crate::health::Health::set_status`] put one
    /// first (§2.3), and with a clock reference configured, reports a clock
    /// ahead as `clock_ahead` faults (§2.5). Refused when the service
    /// implements `health.v1` already.
    pub async fn health(&mut self) -> Result<crate::health::Health> {
        let iface = crate::health::iface();
        if self.health.is_some() || self.implements(&iface) {
            return Err(Error::Duplicate(iface));
        }
        crate::health::Health::declare(self).await
    }

    /// Implements `config.v1`, its standard contract embedded
    /// ([`crate::config_v1`]): the owner starts from `startup`, never from
    /// a runtime change of an earlier instance (§2.12), and configures
    /// `device`. The view and the status are put here, so before step 4
    /// (§2.3); `set/{group}` is served over the template, `confirm`,
    /// `cancel` and `extend` on their keys, and `persist` with a persisted
    /// layer, else listed unavailable with cause `config` (§2.13). The
    /// deadline timer runs until the service closes. Refused when the
    /// service implements `config.v1` already, or when `startup` is not one
    /// an owner can start from (a floor of 0, more than 16 groups, a layer
    /// naming an undeclared parameter, …).
    pub async fn config_v1(
        &mut self,
        startup: crate::config_v1::Startup,
        device: Arc<dyn crate::config_v1::Device>,
    ) -> Result<crate::config_v1::Configuration> {
        let iface = crate::config_v1::iface();
        if self.config_v1.is_some() || self.implements(&iface) {
            return Err(Error::Duplicate(iface));
        }
        crate::config_v1::Configuration::declare(self, startup, device).await
    }

    /// Implements `alarms.v1`, its standard contract embedded
    /// ([`crate::alarms`]): the template `state/alarms/{alert_key}` is
    /// exposed and its GETs served, `startup`'s bound applied, and the
    /// previous instance's alarms adopted from `startup`'s record, else its
    /// archive, each put again before this returns, so before any GET is
    /// answered and before the instance token (§2.9). The handle returned
    /// raises, refreshes and resolves the alarms; each firing one is
    /// confirmed every 427.5 s (§2.7). Refused when the service implements
    /// `alarms.v1` already.
    pub async fn alarms(
        &mut self,
        startup: crate::alarms::Startup,
    ) -> Result<crate::alarms::Alarms> {
        self.alarms_with(crate::alarms::implementation(), startup)
            .await
    }

    /// [`ServiceBuilder::alarms`] on `imp`, another revision of `alarms.v1`
    /// (R4): its `state/alarms/{alert_key}` must be a JSON state, and its
    /// `freshness.ttl_s` sets the confirmation period, half of it with the
    /// refresher's lead (`freshness.v1` §2.4). A later minor, or a test's
    /// copy with a short horizon.
    pub async fn alarms_with(
        &mut self,
        imp: Implementation,
        startup: crate::alarms::Startup,
    ) -> Result<crate::alarms::Alarms> {
        let iface = crate::alarms::iface();
        if self.alarms.is_some() || self.implements(&iface) {
            return Err(Error::Duplicate(iface));
        }
        if zenkey_model::redundancy::is_governed(imp.contract()) {
            return Err(Error::Contract(not_elected("alarms.v1's owner", &iface)));
        }
        crate::alarms::Alarms::declare(self, imp, startup).await
    }

    /// Authors the desired resource `resource` (`state/<template>`) of
    /// `iface`, which this service implements ([`crate::desired`],
    /// `desired.v1` §2.11): the resource exposed and its GETs served, then
    /// the previous instance's documents and deletes taken over from
    /// `startup`'s record, else its archive, and put again before this
    /// returns ([`crate::desired::Pace::All`]), so before the instance token.
    /// The handle returned puts a document only on a content change, and
    /// deletes. Refused for a resource whose `desired.target_param` is not
    /// a valid declaration (§2.2).
    pub async fn desired_author(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        startup: crate::desired::AuthorStartup,
    ) -> Result<crate::desired::Author> {
        if self
            .desired_authors
            .iter()
            .any(|a| a.iface() == iface && a.resource() == resource)
        {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is authored twice (desired.v1 §2.11)"
            )));
        }
        if self.gate_of(iface).is_some() {
            return Err(Error::Contract(not_elected("a desired.v1 author", iface)));
        }
        crate::desired::Author::declare(self, iface, resource, startup).await
    }

    /// Holds the role `role` as a target of `author`'s desired resource
    /// `resource` (`state/<template>`; [`crate::desired`], `desired.v1`
    /// §2.5–§2.10): `author` is the required interface's implementation the
    /// target is compiled against (R4), its bundle what documents are
    /// decoded and checked by. The role's configured binding gives the
    /// slice's key (§2.3); an optional role left unbound is the kill switch
    /// (§2.12). The report is put here, at `report`, before the tokens; the
    /// target reads from [`ServiceBuilder::start`] on, after the tokens, and
    /// acts on `handler`. Refused for a role this service does not hold, an
    /// invalid declaration or binding, or a watch session that is this
    /// service's own.
    pub async fn desired_target(
        &mut self,
        role: &str,
        author: &Implementation,
        resource: &str,
        report: crate::desired::ReportAt,
        handler: Arc<dyn crate::desired::Handler>,
        startup: crate::desired::TargetStartup,
    ) -> Result<crate::desired::Target> {
        self.no_target_yet(role)?;
        if self.gate_of(&report.iface).is_some() {
            return Err(Error::Contract(not_elected(
                "a desired.v1 target's report",
                &report.iface,
            )));
        }
        crate::desired::Target::declare(
            self,
            role,
            author,
            resource,
            Some(report),
            handler,
            startup,
        )
        .await
    }

    /// `config.v1`'s owner as a target (`desired.v1` §2.16): the role
    /// `role`, as [`ServiceBuilder::desired_target`] holds it, each act made
    /// on the `config.v1` this builder implements
    /// ([`ServiceBuilder::config_v1`], called first). A put is converged
    /// once in the instance, whatever its revision, and reported in the view
    /// and the status; a delete reverts each parameter the documents it
    /// acted on named to its startup value, and the view and the status
    /// carry no `desired` from then on (`config.v1` §2.18, text 0.5). Its
    /// `desired.v1.Report` is put at `report`, if given: `config.v1`'s
    /// standard contract carries none (§2.16).
    pub async fn desired_config(
        &mut self,
        role: &str,
        author: &Implementation,
        resource: &str,
        report: Option<crate::desired::ReportAt>,
        startup: crate::desired::TargetStartup,
    ) -> Result<crate::desired::Target> {
        self.no_target_yet(role)?;
        if let Some(r) = &report
            && self.gate_of(&r.iface).is_some()
        {
            return Err(Error::Contract(not_elected(
                "a desired.v1 target's report",
                &r.iface,
            )));
        }
        let Some(cfg) = self.config_v1.clone() else {
            return Err(Error::Contract(
                "desired.v1 §2.16: implement config.v1 (ServiceBuilder::config_v1) first"
                    .to_owned(),
            ));
        };
        let handler = Arc::new(crate::desired::ConfigHandler { cfg });
        crate::desired::Target::declare(self, role, author, resource, report, handler, startup)
            .await
    }

    fn no_target_yet(&self, role: &str) -> Result<()> {
        if self.desired_targets.iter().any(|t| t.role() == role) {
            return Err(Error::Contract(format!(
                "role {role:?} is held as a desired.v1 target twice"
            )));
        }
        Ok(())
    }

    /// The service's state stamp minter (§4.3): catch-up before the first
    /// write goes through it.
    #[must_use]
    pub fn minter(&self) -> &Arc<Minter> {
        &self.minter
    }

    /// A [`StateWriter`] on a state member: stamped puts and deletes (S1),
    /// answered by the service's state queryables (S2, S3), which `start`
    /// declares for this interface. Exposes the resource. Where the
    /// contract declares `freshness.ttl_s` above 0, the writer re-puts the
    /// member at least every ttl/2 (`freshness.v1` §2.4), from its first put
    /// on, before `start` included, until it drops or the service closes.
    pub async fn declare_state_writer(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<StateWriter> {
        let key = self.key(iface, resource, values)?;
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?.clone();
        let budget = s.budget(&r);
        let meter = s.meter(&r, values, &self.epochs);
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is not state"
            )));
        }
        self.expose(iface, resource)?;
        self.state_ifaces.insert(iface.clone());
        let gate = self.gate_of(iface);
        let w = Writer::declare_gated(&self.session, key.into_keyexpr(), &r, values, gate)
            .await?
            .bounded(budget)
            .metered(meter);
        Ok(StateWriter::new(
            w,
            Arc::clone(&self.store),
            Arc::clone(&self.minter),
            &zenkey_model::freshness::horizon_of(&r),
            self.closed.subscribe(),
        ))
    }

    /// Answers state GETs for `iface` (S2) although its writers come later,
    /// through [`Service::state_writer`].
    pub fn serve_state(&mut self, iface: &IfaceId) -> Result<&mut Self> {
        find(&self.impls, iface)?;
        self.state_ifaces.insert(iface.clone());
        Ok(self)
    }

    /// The instance id this service will come up with.
    #[must_use]
    pub fn instance(&self) -> &InstanceId {
        &self.instance
    }

    /// The address this service comes up at, resolved (`hostid.v1` §2.7),
    /// or why it has none (§2.6, or a configuration error, §2.3).
    pub fn address(&self) -> Result<&Addr> {
        self.addr.as_ref().map_err(|e| Error::HostId(Arc::clone(e)))
    }

    /// How its system was minted, for a minted address (`hostid.v1`).
    #[must_use]
    pub fn minted(&self) -> Option<&Minted> {
        self.minted.as_ref()
    }

    /// Implements an interface. Its contract's requirements become roles of
    /// this service, `declared_by` it (§3.3).
    pub fn implement(&mut self, imp: Implementation) -> Result<&mut Self> {
        if self.impls.iter().any(|s| s.imp.iface() == imp.iface()) {
            return Err(Error::Duplicate(imp.iface().clone()));
        }
        for (role, req) in &imp.contract().requires {
            self.roles.push(Role {
                role: role.clone(),
                interface: req.interface.clone(),
                declared_by: Some(imp.iface().clone()),
                optional: req.optional,
            });
        }
        let budgets = imp
            .contract()
            .resources
            .iter()
            .filter_map(|r| Some((resource_name(r), Arc::new(Budget::new(imp.iface(), r)?))))
            .collect();
        let metrics = imp
            .contract()
            .resources
            .iter()
            .filter_map(|r| Some((resource_name(r), Arc::new(Metric::of(imp.contract(), r)?))))
            .collect();
        // redundancy.v1 §2.2: a contract listing the profile in `uses`
        // governs its interface, whose resources are born standing by.
        let gate = zenkey_model::redundancy::is_governed(imp.contract())
            .then(|| Gate::new(imp.iface().clone()));
        self.impls.push(ImplState {
            imp,
            exposed: BTreeSet::new(),
            unavailable: BTreeMap::new(),
            cardinality: BTreeMap::new(),
            budgets,
            metrics,
            gate,
        });
        Ok(self)
    }

    /// Whether this service implements `iface` already.
    #[must_use]
    pub fn implements(&self, iface: &IfaceId) -> bool {
        self.impls.iter().any(|s| s.imp.iface() == iface)
    }

    /// Whether an optional resource is absent here (§3.3): gated on a
    /// capability not held, or listed [`ServiceBuilder::unavailable`]. A
    /// required resource is never absent. Generated servers declare what is
    /// not (#611).
    pub fn is_absent(&self, iface: &IfaceId, resource: &str) -> Result<bool> {
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        Ok(r.optional
            && (missing_capability(r, &self.config.capabilities).is_some()
                || s.unavailable.contains_key(resource)))
    }

    /// Declares a role from the component's manifest (§3.1), not from a
    /// contract. Its bindings come from the configuration.
    pub fn require(&mut self, role: &str, interface: IfaceId, optional: bool) -> &mut Self {
        self.roles.push(Role {
            role: role.to_owned(),
            interface,
            declared_by: None,
            optional,
        });
        self
    }

    /// Marks an optional resource absent here, with its cause (§3.3). A
    /// required resource cannot be unavailable (D005).
    pub fn unavailable(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        cause: Cause,
        reason: Option<&str>,
    ) -> Result<&mut Self> {
        let s = find_mut(&mut self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        if !r.optional {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is required, so it cannot be unavailable"
            )));
        }
        s.unavailable
            .insert(resource.to_owned(), (cause, reason.map(str::to_owned)));
        Ok(self)
    }

    /// Lowers a template's bound for this instance (§3.3). It cannot raise
    /// it (D007), and it cannot lower it below the members its writers have
    /// made live already (§2.7, #741): that is [`Error::Budget`], the bound
    /// unchanged.
    pub fn cardinality(&mut self, iface: &IfaceId, resource: &str, n: u64) -> Result<&mut Self> {
        let s = find_mut(&mut self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        match r.cardinality {
            Some(ceiling) if n <= ceiling => {}
            _ => {
                return Err(Error::Contract(format!(
                    "{iface} {resource:?}: {n} is not a lowering of the contract's bound {:?}",
                    r.cardinality
                )));
            }
        }
        if let Some(b) = s.budget(r) {
            b.lower(n, std::time::Instant::now())?;
        }
        s.cardinality.insert(resource.to_owned(), n);
        if let Some(h) = &self.health
            && h.iface() == iface
            && resource == crate::health::CHECKS
        {
            h.lower_checks(n);
        }
        Ok(self)
    }

    /// Records that this instance serves `resource`, without declaring
    /// anything: for a template whose members appear later, or a resource
    /// declared on the session directly.
    pub fn expose(&mut self, iface: &IfaceId, resource: &str) -> Result<&mut Self> {
        let held = self.config.capabilities.clone();
        let s = find_mut(&mut self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        if let Some(cap) = missing_capability(r, &held) {
            return Err(Error::Gated {
                iface: iface.clone(),
                resource: resource.to_owned(),
                capability: cap.to_owned(),
            });
        }
        s.exposed.insert(resource_name(r));
        Ok(self)
    }

    /// The concrete key of a resource member: `values` binds every template
    /// parameter, raw (they are slugged here, §1.4).
    pub fn key(&self, iface: &IfaceId, resource: &str, values: &Bindings) -> Result<Key> {
        resource_key(self.address()?, &self.impls, iface, resource, values)
    }

    /// Every member of a resource's template, as one key expression: each
    /// parameter becomes `*`, a rest parameter `**`.
    pub fn pattern(&self, iface: &IfaceId, resource: &str) -> Result<OwnedKeyExpr> {
        let addr = self.address()?;
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        let mut chunks = vec![
            "zk2".to_owned(),
            addr.system.to_string(),
            addr.service.to_string(),
            iface.to_string(),
            r.token.to_string(),
        ];
        chunks.extend(r.template.segments().iter().map(|seg| match seg {
            Segment::Literal(l) => l.clone(),
            Segment::Param(_) => "*".to_owned(),
            Segment::Rest(_) => "**".to_owned(),
        }));
        OwnedKeyExpr::try_from(chunks.join("/")).map_err(zenoh)
    }

    /// Declares a publisher on a resource member, with the contract's QoS
    /// (§2.4), and exposes the resource. Streams, state and events only.
    ///
    /// A raw zenoh publisher: the runtime does not see its puts, so keeping
    /// the resource's budget (§2.7) with it is the caller's. The writers
    /// ([`ServiceBuilder::declare_writer`], [`ServiceBuilder::declare_state_writer`],
    /// [`ServiceBuilder::event_writer`]) keep it.
    pub async fn declare_publisher(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<Publisher<'static>> {
        let key = self.key(iface, resource, values)?;
        let r = find(&self.impls, iface)?.imp.resource(resource)?.clone();
        let zenkey_model::contract::Body::Data(d) = &r.body else {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is an operation: it is served by a queryable"
            )));
        };
        self.not_governed(iface, "publishers")?;
        self.expose(iface, resource)?;
        self.session
            .declare_publisher(key.into_keyexpr())
            .reliability(qos::reliability(d.reliability))
            .congestion_control(qos::congestion(d.congestion))
            .priority(qos::priority(d.priority))
            .express(d.express)
            .await
            .map_err(zenoh)
    }

    /// Declares a queryable on a resource member, or on every member when
    /// `values` is `None`, and exposes the resource. An operation's
    /// queryable is `complete` (O1); state and events answer GETs. Streams
    /// take no queryable.
    pub async fn declare_queryable<C>(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: Option<&Bindings>,
        callback: C,
    ) -> Result<Queryable<()>>
    where
        C: Fn(Query) + Send + Sync + 'static,
    {
        let r = find(&self.impls, iface)?.imp.resource(resource)?.clone();
        if matches!(r.token, KindToken::Stream | KindToken::ExplicitStream) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is a stream: it takes no queryable"
            )));
        }
        let ke = match values {
            Some(v) => self.key(iface, resource, v)?.into_keyexpr(),
            None => self.pattern(iface, resource)?,
        };
        self.not_governed(iface, "queryables")?;
        self.expose(iface, resource)?;
        self.session
            .declare_queryable(ke)
            .complete(r.token == KindToken::Op)
            .callback(callback)
            .await
            .map_err(zenoh)
    }

    /// Declares a [`Writer`] on a stream or state member, with the
    /// contract's QoS and `Encoding`, and exposes the resource.
    pub async fn declare_writer(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<Writer> {
        let key = self.key(iface, resource, values)?;
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?.clone();
        let budget = s.budget(&r);
        let meter = s.meter(&r, values, &self.epochs);
        let gate = s.gate.clone();
        self.expose(iface, resource)?;
        Ok(
            Writer::declare_gated(&self.session, key.into_keyexpr(), &r, values, gate)
                .await?
                .bounded(budget)
                .metered(meter),
        )
    }

    /// An [`EventWriter`] for an event's member, and exposes the resource.
    pub fn event_writer(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<EventWriter> {
        let key = self.key(iface, resource, values)?;
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?.clone();
        let budget = s.budget(&r);
        let gate = s.gate.clone();
        self.expose(iface, resource)?;
        Ok(
            EventWriter::new(&self.session, key.to_string(), &r, values)?
                .bounded(budget)
                .gated(gate),
        )
    }

    /// Brings the service up in the order of §8.2. On any refusal, nothing
    /// alive is declared.
    pub async fn start(self) -> Result<Service> {
        // Before step 1 (hostid.v1 §2.7): a service whose minted system
        // could not be had declared nothing, and starts nothing (§2.6).
        let addr = self.address()?.clone();
        // 2. A role refused at its declaration refuses the start, nothing
        // declared (desired.v1 §2.1, text 0.7, F-224).
        if let Some(why) = self.refused.first() {
            return Err(Error::NotExposed(why.clone()));
        }
        // Exposure.
        check_exposure(&self.impls, &self.config)?;
        // §4.4, §8.2 step 2 (0.17): an archive is never tokenless. The set
        // is the deployment's configuration, so naming the archive in it is
        // refused whether or not this owner implements it.
        if self.config.tokenless.contains(&crate::archive::iface()) {
            return Err(Error::NotExposed(
                "archive.v1 is in the tokenless set, and an archive is never tokenless: \
                 consumers and tools find it by its interface token (§4.4)"
                    .to_owned(),
            ));
        }
        // redundancy.v1 §2.2, §2.14 (#841): every governed interface's
        // declaration valid, and taking part in an election; under
        // `external`, a lock to ask, and with `refuse_without_lock` the lock
        // held (§2.11). Nothing is declared before.
        let governed = crate::redundancy::elections(
            &self.impls,
            &self.config.tokenless,
            self.redundancy.is_some(),
        )?;
        if let Some(r) = &self.redundancy {
            let external = governed
                .iter()
                .any(|(_, e, ..)| *e == crate::redundancy::Election::External);
            if external && r.startup().lock.is_none() {
                return Err(Error::NotExposed(
                    "redundancy.v1 §2.11: an interface's election is external, and \
                     RedundancyStartup gives no lock to ask"
                        .to_owned(),
                ));
            }
            if external && !r.may_start().await {
                return Err(Error::NotExposed(
                    "redundancy.v1 §2.11: the deployment's lock is not held, and this instance \
                     refuses to start without it"
                        .to_owned(),
                ));
            }
        }
        for r in &self.roles {
            let bound = self
                .config
                .bindings
                .get(&r.role)
                .is_some_and(|b| !b.providers.is_empty());
            if !r.optional && !bound {
                return Err(Error::NotExposed(format!(
                    "role {:?} ({}) is required and the configuration binds it to nothing (R1)",
                    r.role, r.interface
                )));
            }
        }
        // O3: `unavailable` answered for the optional operations not served.
        let ops = crate::operation::Ops::start(&self).await?;
        let served = Arc::new(Served {
            session: self.session.clone(),
            inputs: Mutex::new(Inputs {
                addr: addr.clone(),
                minted: self.minted.is_some(),
                host: self.host.clone(),
                instance: self.instance.clone(),
                impls: self.impls.clone(),
                roles: self.roles.clone(),
                config: self.config.clone(),
            }),
            current: Mutex::new(Arc::new(RwLock::new(Arc::from(Vec::new())))),
            putting: Mutex::new(()),
        });
        let mut svc = Service {
            _guard: self.redundancy.clone().map(RedundancyGuard),
            alive: BTreeMap::new(),
            instance_token: None,
            members: BTreeMap::new(),
            descriptor_q: None,
            contract_qs: Vec::new(),
            ops,
            state_qs: BTreeMap::new(),
            clock: None,
            health: self.health,
            config_v1: self.config_v1,
            alarms: self.alarms,
            desired_authors: self.desired_authors,
            desired_targets: self.desired_targets,
            redundancy: self.redundancy,
            served,
            session: self.session,
            config: self.config,
            addr,
            minted: self.minted,
            instance: self.instance,
            epochs: self.epochs,
            impls: self.impls,
            roles: self.roles,
            minter: self.minter,
            store: self.store,
            closed: self.closed,
        };
        // 1. (continued) The state queryables, with the other resources.
        for iface in &self.state_ifaces {
            svc.ensure_state_server(iface).await?;
        }
        // health.v1 §2.3: the status before step 4, and before the
        // descriptor lists health.v1, so a GET made at either finds it.
        if let Some(h) = &svc.health {
            h.bring_up().await?;
        }
        if let Some(key) = svc.config.clock_reference.clone() {
            svc.clock = Some(ClockGuard::start(&svc.session, &key, Arc::clone(&svc.minter)).await?);
            // health.v1 §2.5: a clock ahead is reported as a fault.
            if let Some(h) = &svc.health {
                h.watch_clock(svc.closed.subscribe());
            }
        }
        // 3. The descriptor, then the contracts.
        let (q, current) = svc.serve_descriptor(&svc.instance.clone()).await?;
        svc.served.set_current(current);
        svc.descriptor_q = Some(q);
        for s in &svc.impls {
            let key = contract_key(s.imp.iface(), s.imp.fingerprint().hex())?;
            let bytes = Arc::clone(s.imp.bundle_bytes());
            let reply_key = key.clone().into_keyexpr();
            let q = svc
                .session
                .declare_queryable(key.into_keyexpr())
                .complete(true)
                .callback(move |q| {
                    let _ = q
                        .reply(reply_key.clone(), bytes.to_vec())
                        .encoding(Encoding::APPLICATION_JSON)
                        .wait();
                })
                .await
                .map_err(zenoh)?;
            svc.contract_qs.push(q);
        }
        // 4. The instance token, then the interface tokens.
        let instance = svc.instance.clone();
        svc.instance_token = Some(svc.token(instance_key(&svc.addr, &instance)?).await?);
        svc.alive = svc.declare_alive(&instance).await?;
        // redundancy.v1 §2.5 (#841): the claim is made (the descriptor
        // marks each governed interface standby, the instance token); its
        // watch and round trips, then the election.
        if let Some(r) = svc.redundancy.clone() {
            r.start(&svc, governed).await?;
        }
        // desired.v1 §2.5: each target reads from here on, after the tokens,
        // its baseline report put before them (§2.10).
        for t in &svc.desired_targets {
            t.start(svc.closed.subscribe());
        }
        Ok(svc)
    }
}

/// Validates §8.2 step 2 for every implemented interface.
fn check_exposure(impls: &[ImplState], config: &ServiceConfig) -> Result<()> {
    let mut problems = Vec::new();
    for s in impls {
        for r in &s.imp.contract().resources {
            let name = resource_name(r);
            // A declared resource whose capability is lost later is simply
            // absent, implied by the capability (§3.3); `expose` refuses to
            // declare one whose capability is not held in the first place.
            if s.exposed.contains(&name) {
                continue;
            }
            if !r.optional {
                problems.push(format!(
                    "{} {name:?} is required and not exposed",
                    s.imp.iface()
                ));
            } else if missing_capability(r, &config.capabilities).is_none()
                && !s.unavailable.contains_key(&name)
            {
                problems.push(format!(
                    "{} {name:?} is optional and neither exposed nor listed unavailable",
                    s.imp.iface()
                ));
            }
        }
        for name in s.unavailable.keys() {
            if s.exposed.contains(name) {
                problems.push(format!(
                    "{} {name:?} is both exposed and listed unavailable",
                    s.imp.iface()
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(Error::NotExposed(problems.join("; ")))
    }
}

/// Whether an interface exposes at least one resource now (§8.1): the
/// condition for holding its interface token.
pub(crate) fn exposes_any(s: &ImplState, held: &BTreeSet<String>) -> bool {
    s.imp
        .contract()
        .resources
        .iter()
        .any(|r| s.exposed.contains(&resource_name(r)) && missing_capability(r, held).is_none())
}

fn resource_key(
    addr: &Addr,
    impls: &[ImplState],
    iface: &IfaceId,
    resource: &str,
    values: &Bindings,
) -> Result<Key> {
    let s = find(impls, iface)?;
    let r = s.imp.resource(resource)?;
    let chunks = r
        .template
        .build(values)
        .map_err(|e| Error::Contract(format!("{iface} {resource:?}: {e}")))?;
    let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
    Ok(data_key(addr, iface, r.token, &refs)?)
}

/// A running service. Dropping it undeclares its tokens first, then its
/// queryables; [`Service::close`] does the same and waits for it.
pub struct Service {
    // Tokens first: fields drop in declaration order. The elections' guard
    // before them: a governed interface's token goes first.
    _guard: Option<RedundancyGuard>,
    alive: BTreeMap<IfaceId, LivelinessToken>,
    instance_token: Option<LivelinessToken>,
    members: BTreeMap<(IfaceId, String), (InstanceId, LivelinessToken)>,
    descriptor_q: Option<Queryable<()>>,
    contract_qs: Vec<Queryable<()>>,
    ops: crate::operation::Ops,
    state_qs: BTreeMap<IfaceId, Vec<Arc<QSlot>>>,
    clock: Option<ClockGuard>,
    /// `health.v1`, when the builder implemented it (`health.v1` §2.3).
    health: Option<crate::health::Health>,
    /// `config.v1`, when the builder implemented it.
    config_v1: Option<crate::config_v1::Configuration>,
    /// `alarms.v1`, when the builder implemented it.
    alarms: Option<crate::alarms::Alarms>,
    /// `desired.v1`'s authored resources and target roles.
    desired_authors: Vec<crate::desired::Author>,
    desired_targets: Vec<crate::desired::Target>,
    /// `redundancy.v1`, when the builder configured it.
    redundancy: Option<crate::redundancy::Redundancy>,
    /// The descriptor as served, shared with `redundancy.v1`'s elections.
    served: Arc<Served>,
    session: zenoh::Session,
    config: ServiceConfig,
    /// Resolved once at start, and kept across every re-mint (`hostid.v1`
    /// §2.7).
    addr: Addr,
    minted: Option<Minted>,
    instance: InstanceId,
    /// The epochs its metric writers write (`telemetry.v1` §2.9).
    epochs: Arc<Epochs>,
    impls: Vec<ImplState>,
    roles: Vec<Role>,
    minter: Arc<Minter>,
    store: Arc<Store>,
    /// Set by [`Service::close`], dropped with the service: either stops
    /// its state writers' refreshers (`freshness.v1` §2.4).
    closed: tokio::sync::watch::Sender<bool>,
}

impl Service {
    /// The address, resolved: a minted system spelled out.
    #[must_use]
    pub fn address(&self) -> &Addr {
        &self.addr
    }

    /// How its system was minted, for a minted address (`hostid.v1`); `None`
    /// for a literal one.
    #[must_use]
    pub fn minted(&self) -> Option<&Minted> {
        self.minted.as_ref()
    }

    /// The current instance id (§1.5).
    #[must_use]
    pub fn instance(&self) -> &InstanceId {
        &self.instance
    }

    /// The instance key: the instance token's and the descriptor's.
    pub fn instance_key(&self) -> Result<Key> {
        Ok(instance_key(&self.addr, &self.instance)?)
    }

    /// The descriptor currently served, as JSON bytes.
    #[must_use]
    pub fn descriptor_bytes(&self) -> Arc<[u8]> {
        self.served.bytes()
    }

    /// Its `redundancy.v1` elections, when [`ServiceBuilder::redundancy`]
    /// configured them: the handle the builder returned.
    #[must_use]
    pub fn redundancy(&self) -> Option<&crate::redundancy::Redundancy> {
        self.redundancy.as_ref()
    }

    /// A governed interface's gate, when `iface` is governed.
    pub(crate) fn gate_of(&self, iface: &IfaceId) -> Option<Arc<Gate>> {
        find(&self.impls, iface).ok().and_then(|s| s.gate.clone())
    }

    /// The concrete key of a resource member (see [`ServiceBuilder::key`]).
    pub fn key(&self, iface: &IfaceId, resource: &str, values: &Bindings) -> Result<Key> {
        resource_key(&self.addr, &self.impls, iface, resource, values)
    }

    /// A [`Writer`] on a member of an exposed resource: for templates whose
    /// members appear while the service runs. Exposure is fixed at start, so
    /// an unexposed resource is refused.
    pub async fn writer(
        &self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<Writer> {
        let r = self.exposed(iface, resource)?;
        let key = self.key(iface, resource, values)?;
        let s = find(&self.impls, iface)?;
        let (budget, meter) = (s.budget(&r), s.meter(&r, values, &self.epochs));
        Ok(Writer::declare_gated(
            &self.session,
            key.into_keyexpr(),
            &r,
            values,
            s.gate.clone(),
        )
        .await?
        .bounded(budget)
        .metered(meter))
    }

    /// A [`StateWriter`] on a member of an exposed state resource, for
    /// templates whose members appear while the service runs. The
    /// interface's state queryables are declared on first use. It re-puts
    /// the member as [`ServiceBuilder::declare_state_writer`]'s does.
    pub async fn state_writer(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<StateWriter> {
        let r = self.exposed(iface, resource)?;
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is not state"
            )));
        }
        self.ensure_state_server(iface).await?;
        let key = self.key(iface, resource, values)?;
        let s = find(&self.impls, iface)?;
        let (budget, meter) = (s.budget(&r), s.meter(&r, values, &self.epochs));
        let w = Writer::declare_gated(
            &self.session,
            key.into_keyexpr(),
            &r,
            values,
            s.gate.clone(),
        )
        .await?
        .bounded(budget)
        .metered(meter);
        Ok(StateWriter::new(
            w,
            Arc::clone(&self.store),
            Arc::clone(&self.minter),
            &zenkey_model::freshness::horizon_of(&r),
            self.closed.subscribe(),
        ))
    }

    /// The service's state stamp minter (§4.3).
    #[must_use]
    pub fn minter(&self) -> &Arc<Minter> {
        &self.minter
    }

    /// Its `health.v1`, when [`ServiceBuilder::health`] implemented it: the
    /// handle the builder returned.
    #[must_use]
    pub fn health(&self) -> Option<&crate::health::Health> {
        self.health.as_ref()
    }

    /// Its `config.v1`, when [`ServiceBuilder::config_v1`] implemented it:
    /// the handle the builder returned.
    #[must_use]
    pub fn config_v1(&self) -> Option<&crate::config_v1::Configuration> {
        self.config_v1.as_ref()
    }

    /// Its `alarms.v1`, when [`ServiceBuilder::alarms`] implemented it: the
    /// handle the builder returned.
    #[must_use]
    pub fn alarms(&self) -> Option<&crate::alarms::Alarms> {
        self.alarms.as_ref()
    }

    /// Its authorship of `iface`'s desired resource `resource`, when
    /// [`ServiceBuilder::desired_author`] declared it.
    #[must_use]
    pub fn desired_author(
        &self,
        iface: &IfaceId,
        resource: &str,
    ) -> Option<&crate::desired::Author> {
        self.desired_authors
            .iter()
            .find(|a| a.iface() == iface && a.resource() == resource)
    }

    /// Its `desired.v1` role `role`, when [`ServiceBuilder::desired_target`]
    /// or [`ServiceBuilder::desired_config`] declared it.
    #[must_use]
    pub fn desired_target(&self, role: &str) -> Option<&crate::desired::Target> {
        self.desired_targets.iter().find(|t| t.role() == role)
    }

    /// Declares `iface`'s state queryables, once: one over `state/**` and
    /// one over `@state/**` where the contract has such resources (S2).
    async fn ensure_state_server(&mut self, iface: &IfaceId) -> Result<()> {
        if self.state_qs.contains_key(iface) {
            return Ok(());
        }
        let s = find(&self.impls, iface)?;
        let gate = s.gate.clone();
        let mut qs = Vec::new();
        for token in [KindToken::State, KindToken::ExplicitState] {
            if !s.imp.contract().resources.iter().any(|r| r.token == token) {
                continue;
            }
            let ke = OwnedKeyExpr::try_from(format!(
                "zk2/{}/{}/{iface}/{token}/**",
                self.addr.system, self.addr.service
            ))
            .map_err(zenoh)?;
            let store = Arc::clone(&self.store);
            // redundancy.v1 §2.4: a governed interface's, only while it
            // serves.
            qs.push(
                QSlot::declare(
                    &self.session,
                    ke,
                    false,
                    Arc::new(move |q| store.answer(&q)),
                    gate.as_ref(),
                )
                .await?,
            );
        }
        self.state_qs.insert(iface.clone(), qs);
        Ok(())
    }

    /// An [`EventWriter`] on a member of an exposed event.
    pub fn event_writer(
        &self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<EventWriter> {
        let r = self.exposed(iface, resource)?;
        let key = self.key(iface, resource, values)?;
        let s = find(&self.impls, iface)?;
        Ok(
            EventWriter::new(&self.session, key.to_string(), &r, values)?
                .bounded(s.budget(&r))
                .gated(s.gate.clone()),
        )
    }

    /// The members of `resource` its writers hold live now, and its bound
    /// in force with whose it is (core §2.7, #741); `None` for an
    /// operation.
    pub fn budget(
        &self,
        iface: &IfaceId,
        resource: &str,
    ) -> Result<Option<(u64, zenkey_model::budget::Bound, crate::budget::Source)>> {
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        Ok(s.budget(r).map(|b| {
            let (bound, source) = b.bound();
            (b.live(std::time::Instant::now()), bound, source)
        }))
    }

    /// The consumer of `role`, compiled against `contract`, the required
    /// interface's (R4: any revision of its major). Its providers and
    /// parameter bindings are the configuration's (R1, R2).
    pub fn consumer(
        &self,
        role: &str,
        contract: std::sync::Arc<zenkey_model::contract::Contract>,
    ) -> Result<Consumer> {
        let r = self
            .roles
            .iter()
            .find(|r| r.role == role)
            .ok_or_else(|| Error::Contract(format!("this service has no role {role:?}")))?;
        if r.interface != contract.iface {
            return Err(Error::Contract(format!(
                "role {role:?} requires {}, not {}",
                r.interface, contract.iface
            )));
        }
        let b = self.config.bindings.get(role).cloned().unwrap_or_default();
        Consumer::new(
            &self.session,
            Some(&self.addr),
            role,
            contract,
            &b.providers,
            &b.params,
        )
    }

    fn exposed(&self, iface: &IfaceId, resource: &str) -> Result<zenkey_model::contract::Resource> {
        let s = find(&self.impls, iface)?;
        let r = s.imp.resource(resource)?;
        if !s.exposed.contains(&resource_name(r)) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} was not exposed before start (§8.2)"
            )));
        }
        Ok(r.clone())
    }

    /// The interfaces for which this instance holds an interface token now:
    /// a governed one's only while it serves (`redundancy.v1` §2.4).
    #[must_use]
    pub fn tokens_held(&self) -> Vec<IfaceId> {
        let mut held: Vec<IfaceId> = self.alive.keys().cloned().collect();
        if let Some(r) = &self.redundancy {
            held.extend(r.tokens_held());
            held.sort();
        }
        held
    }

    /// Replaces the held capabilities (§3.3). The descriptor is put again,
    /// and interface tokens follow exposure (§8.1). Refused, with nothing
    /// changed, if a resource the new set ungates is neither exposed nor
    /// listed unavailable.
    pub async fn set_capabilities(&mut self, capabilities: BTreeSet<String>) -> Result<()> {
        let mut next = self.config.clone();
        next.capabilities = capabilities;
        check_exposure(&self.impls, &next)?;
        self.config = next;
        self.ops
            .availability
            .refresh(&self.impls, &self.config.capabilities);
        self.sync_served();
        self.publish_descriptor()?;
        let instance = self.instance.clone();
        let want: BTreeSet<IfaceId> = self.wanted_tokens();
        self.alive.retain(|iface, _| want.contains(iface));
        for s in &self.impls {
            let iface = s.imp.iface();
            if want.contains(iface) && !self.alive.contains_key(iface) {
                let key = alive_key(
                    &self.addr,
                    iface,
                    &instance,
                    &s.imp.fingerprint().hex().fp16(),
                )?;
                let t = self.token(key).await?;
                self.alive.insert(iface.clone(), t);
            }
        }
        Ok(())
    }

    /// Lists an optional resource as unavailable, or clears it (`None`),
    /// and puts the descriptor again.
    pub fn set_unavailable(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        entry: Option<(Cause, Option<&str>)>,
    ) -> Result<()> {
        let s = find_mut(&mut self.impls, iface)?;
        if !s.imp.resource(resource)?.optional {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is required, so it cannot be unavailable"
            )));
        }
        match entry {
            Some((cause, reason)) => {
                s.unavailable
                    .insert(resource.to_owned(), (cause, reason.map(str::to_owned)));
            }
            None => {
                s.unavailable.remove(resource);
            }
        }
        self.ops
            .availability
            .refresh(&self.impls, &self.config.capabilities);
        self.sync_served();
        self.publish_descriptor()
    }

    /// Mints a new instance id (§1.5) **make-before-break** (§8.1): the new
    /// descriptor, instance token and interface tokens are declared first,
    /// then the old ones are undeclared. A watcher sees one or two live
    /// instance tokens, never none.
    ///
    /// The spec's SHOULD of a new session, so that timestamps' id changes
    /// too (§4.3), is the caller's: the runtime never owns the session.
    ///
    /// Refused for a service implementing `config.v1` through
    /// [`ServiceBuilder::config_v1`]: a new instance is a restart of its
    /// configuration (`config.v1` §2.12), which the runtime makes as a new
    /// service, from the startup values ([`crate::config_v1`]).
    ///
    /// A service implementing `alarms.v1` through [`ServiceBuilder::alarms`]
    /// puts every alarm it holds again first, before the new descriptor and
    /// tokens (`alarms.v1` §2.9), and an author of `desired.v1` documents
    /// every document it keeps, and every delete within its window
    /// (`desired.v1` §2.11).
    pub async fn new_epoch(&mut self) -> Result<InstanceId> {
        if self.config_v1.is_some() {
            return Err(Error::Contract(
                "config.v1: a new instance is a restart of the configuration, from its startup \
                 values (config.v1 §2.12): close this service and start another"
                    .to_owned(),
            ));
        }
        // alarms.v1 §2.9: each alarm kept put again from the new instance,
        // before its new instance token.
        if let Some(a) = &self.alarms {
            a.reput_all().await?;
        }
        // desired.v1 §2.11: so is each document an author keeps, and each
        // delete within its window. A target keeps its acts and its report:
        // the process that applied them runs on.
        for a in &self.desired_authors {
            a.reput_all().await?;
        }
        let next = mint();
        // redundancy.v1 §2.9 (text 0.2, F-209): every elector told of the
        // new id before its tokens, a standby's included, and held, so that
        // no takeover or step-down runs through the re-mint.
        let remints = match &self.redundancy {
            Some(r) => r.remint(&next).await,
            None => Vec::new(),
        };
        let (q, current) = self.serve_descriptor(&next).await?;
        let token = self.token(instance_key(&self.addr, &next)?).await?;
        let alive = self.declare_alive(&next).await?;
        // A serving governed interface's token, make-before-break too.
        let mut governed = Vec::with_capacity(remints.len());
        for r in &remints {
            governed.push(r.make(&next).await?);
        }
        // Break: the old tokens, then the old descriptor queryable.
        let old_alive = std::mem::replace(&mut self.alive, alive);
        let old_token = self.instance_token.replace(token);
        let old_q = self.descriptor_q.replace(q);
        self.served.set_current(current);
        self.instance = next.clone();
        self.sync_served();
        // telemetry.v1 §2.9: every metric writer's epoch moves with it.
        self.epochs.set_instance(next.clone());
        let old_governed: Vec<LivelinessToken> = remints
            .iter()
            .zip(governed)
            .filter_map(|(r, t)| r.swap(t))
            .collect();
        for (_, t) in old_alive {
            t.undeclare().await.map_err(zenoh)?;
        }
        for t in old_governed {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(t) = old_token {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(q) = old_q {
            q.undeclare().await.map_err(zenoh)?;
        }
        drop(remints);
        Ok(next)
    }

    /// Declares the member token of `value` (§8.1), for the interface's
    /// template that declares `epoch`. Returns the member's epoch.
    ///
    /// A member's first token takes the epoch a metric writer already wrote
    /// on it, if one did (`telemetry.v1` §2.9: a member exists from its first
    /// value, core §8.1); a token declared again is cycled, with a new
    /// epoch, which every metric writer of the member writes from then on.
    ///
    /// On a governed interface, only while the instance serves it
    /// (`redundancy.v1` §2.12): [`Error::Standby`] otherwise, and a
    /// step-down withdraws every member token, which the application
    /// declares again as each member appears after a later takeover (§2.6
    /// step 4).
    pub async fn declare_member(&mut self, iface: &IfaceId, value: &str) -> Result<InstanceId> {
        self.member_template(iface)?;
        let gate = self.gate_of(iface);
        if let Some(g) = &gate {
            g.admit()?;
        }
        let member = chunk_slug(value);
        let held = match &gate {
            Some(g) => g.holds_member(&member),
            None => self.members.contains_key(&(iface.clone(), member.clone())),
        };
        let epoch = if held {
            self.epochs.cycled(iface, &member)
        } else {
            self.epochs.first_token(iface, &member)
        };
        let key = member_key(&self.addr, iface, &member, &epoch)?;
        let token = self.token(key).await?;
        let old = match &gate {
            Some(g) => g.hold_member(member, epoch.clone(), token)?,
            None => self
                .members
                .insert((iface.clone(), member), (epoch.clone(), token)),
        };
        if let Some((_, old)) = old {
            old.undeclare().await.map_err(zenoh)?;
        }
        Ok(epoch)
    }

    /// Whether this instance holds `value`'s member token of `iface` now
    /// (§8.1). On a governed interface, only while it serves: a step-down
    /// withdraws every member token (`redundancy.v1` §2.12), so an
    /// application that declares its members at each takeover asks this
    /// first, since [`Service::declare_member`] of a member held cycles it,
    /// a new epoch (#841, RD).
    #[must_use]
    pub fn holds_member(&self, iface: &IfaceId, value: &str) -> bool {
        let member = chunk_slug(value);
        match self.gate_of(iface) {
            Some(g) => g.holds_member(&member),
            None => self.members.contains_key(&(iface.clone(), member)),
        }
    }

    /// Cycles a member's token when its continuity breaks (§8.1): the new
    /// epoch's token first, then the old one. Other members are untouched.
    pub async fn cycle_member(&mut self, iface: &IfaceId, value: &str) -> Result<InstanceId> {
        self.declare_member(iface, value).await
    }

    /// Withdraws a member's token.
    pub async fn remove_member(&mut self, iface: &IfaceId, value: &str) -> Result<()> {
        let member = chunk_slug(value);
        let held = match self.gate_of(iface) {
            Some(g) => g.take_member(&member),
            None => self.members.remove(&(iface.clone(), member.clone())),
        };
        if let Some((_, t)) = held {
            self.epochs.removed(iface, &member);
            t.undeclare().await.map_err(zenoh)?;
        }
        Ok(())
    }

    /// Stops its state writers' re-puts (`freshness.v1` §2.4), then
    /// undeclares the tokens, then the queryables, and waits for each.
    pub async fn close(mut self) -> Result<()> {
        self.closed.send_replace(true);
        // redundancy.v1 (#841): every election stopped, its writes, then
        // each governed interface's tokens, before the rest.
        if let Some(r) = &self.redundancy {
            r.shutdown().await?;
        }
        for (_, t) in std::mem::take(&mut self.alive) {
            t.undeclare().await.map_err(zenoh)?;
        }
        for (_, (_, t)) in std::mem::take(&mut self.members) {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(t) = self.instance_token.take() {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(r) = &self.redundancy {
            r.unexpose_all().await?;
        }
        // config.v1: its operations go with the tokens, so a restarted
        // instance never shares a key with this one (§2.12).
        if let Some(c) = &self.config_v1 {
            c.shutdown().await?;
        }
        if let Some(q) = self.descriptor_q.take() {
            q.undeclare().await.map_err(zenoh)?;
        }
        for q in std::mem::take(&mut self.contract_qs) {
            q.undeclare().await.map_err(zenoh)?;
        }
        Ok(())
    }

    fn member_template(&self, iface: &IfaceId) -> Result<()> {
        let s = find(&self.impls, iface)?;
        if s.imp.contract().resources.iter().any(|r| r.epoch.is_some()) {
            Ok(())
        } else {
            Err(Error::Contract(format!(
                "{iface} has no template that declares `epoch`, so it has no members"
            )))
        }
    }

    fn wanted_tokens(&self) -> BTreeSet<IfaceId> {
        self.impls
            .iter()
            .filter(|s| !self.config.tokenless.contains(s.imp.iface()))
            // redundancy.v1 §2.6: a governed interface's token is its
            // election's, declared last at a takeover.
            .filter(|s| s.gate.is_none())
            .filter(|s| exposes_any(s, &self.config.capabilities))
            .map(|s| s.imp.iface().clone())
            .collect()
    }

    async fn declare_alive(
        &self,
        instance: &InstanceId,
    ) -> Result<BTreeMap<IfaceId, LivelinessToken>> {
        let want = self.wanted_tokens();
        let mut out = BTreeMap::new();
        for s in self.impls.iter().filter(|s| want.contains(s.imp.iface())) {
            let key = alive_key(
                &self.addr,
                s.imp.iface(),
                instance,
                &s.imp.fingerprint().hex().fp16(),
            )?;
            out.insert(s.imp.iface().clone(), self.token(key).await?);
        }
        Ok(out)
    }

    async fn token(&self, key: Key) -> Result<LivelinessToken> {
        self.session
            .liveliness()
            .declare_token(key.into_keyexpr())
            .await
            .map_err(zenoh)
    }

    /// Copies what the descriptor is made of into [`Served`], after a
    /// change of it.
    fn sync_served(&self) {
        let mut i = self.served.inputs.lock().expect("not poisoned");
        i.instance = self.instance.clone();
        i.impls = self.impls.clone();
        i.roles = self.roles.clone();
        i.config = self.config.clone();
    }

    /// Declares the descriptor's queryable on `instance`'s key and puts the
    /// descriptor there (§3.3).
    async fn serve_descriptor(
        &self,
        instance: &InstanceId,
    ) -> Result<(Queryable<()>, Arc<RwLock<Arc<[u8]>>>)> {
        let bytes: Arc<[u8]> = self.served.encode(instance)?.into();
        let key = instance_key(&self.addr, instance)?.into_keyexpr();
        let current = Arc::new(RwLock::new(Arc::clone(&bytes)));
        let shared = Arc::clone(&current);
        let reply_key = key.clone();
        let q = self
            .session
            .declare_queryable(key.clone())
            .complete(true)
            .callback(move |q| {
                let now = Arc::clone(&shared.read().expect("not poisoned"));
                let _ = q
                    .reply(reply_key.clone(), now.to_vec())
                    .encoding(Encoding::APPLICATION_JSON)
                    .wait();
            })
            .await
            .map_err(zenoh)?;
        self.session
            .put(key, bytes.to_vec())
            .encoding(Encoding::APPLICATION_JSON)
            .timestamp(self.session.new_timestamp())
            .await
            .map_err(zenoh)?;
        Ok((q, current))
    }

    /// Re-encodes the descriptor, swaps it into the queryable and puts it
    /// (§3.3: put on every change).
    fn publish_descriptor(&mut self) -> Result<()> {
        self.served.publish()
    }
}

/// What the descriptor is made of (§3.3), as [`Served`] keeps it.
struct Inputs {
    addr: Addr,
    minted: bool,
    host: Option<String>,
    instance: InstanceId,
    impls: Vec<ImplState>,
    roles: Vec<Role>,
    config: ServiceConfig,
}

/// The descriptor as served (§3.3), and what it is made of: the service's,
/// shared with `redundancy.v1`'s elections, which put it again at a
/// takeover and a step-down, each governed interface's mark read from its
/// gate (§2.6 step 3, §2.9 step 3).
pub(crate) struct Served {
    session: zenoh::Session,
    inputs: Mutex<Inputs>,
    /// What the current descriptor queryable answers.
    current: Mutex<Arc<RwLock<Arc<[u8]>>>>,
    /// One encode-and-put at a time, so that puts go out in order.
    putting: Mutex<()>,
}

impl Served {
    /// The descriptor of `instance`, encoded and checked.
    fn encode(&self, instance: &InstanceId) -> Result<Vec<u8>> {
        let i = self.inputs.lock().expect("not poisoned");
        let zid = self.session.zid().to_string();
        let id = descriptor::Identity {
            addr: &i.addr,
            minted: i.minted,
            host: i.host.as_deref(),
            zid: &zid,
        };
        let d = descriptor::build(&id, instance, &i.impls, &i.roles, &i.config);
        descriptor::encode_checked(&d, &i.impls)
    }

    /// The bytes the current queryable answers.
    fn bytes(&self) -> Arc<[u8]> {
        let current = self.current.lock().expect("not poisoned");
        Arc::clone(&current.read().expect("not poisoned"))
    }

    fn set_current(&self, current: Arc<RwLock<Arc<[u8]>>>) {
        *self.current.lock().expect("not poisoned") = current;
    }

    /// Re-encodes the descriptor, swaps it into the queryable and puts it
    /// (§3.3: put on every change).
    pub(crate) fn publish(&self) -> Result<()> {
        let _putting = self.putting.lock().expect("not poisoned");
        let (instance, addr) = {
            let i = self.inputs.lock().expect("not poisoned");
            (i.instance.clone(), i.addr.clone())
        };
        let bytes: Arc<[u8]> = self.encode(&instance)?.into();
        {
            let current = self.current.lock().expect("not poisoned");
            *current.write().expect("not poisoned") = Arc::clone(&bytes);
        }
        let key = instance_key(&addr, &instance)?.into_keyexpr();
        self.session
            .put(key, bytes.to_vec())
            .encoding(Encoding::APPLICATION_JSON)
            .timestamp(self.session.new_timestamp())
            .wait()
            .map_err(zenoh)
    }
}

/// Stops a service's elections and drops their tokens first when the
/// service drops without [`Service::close`].
struct RedundancyGuard(crate::redundancy::Redundancy);

impl Drop for RedundancyGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// What the operation layer (`operation.rs`, `client.rs`, #621) reads.
impl ServiceBuilder {
    pub(crate) fn session(&self) -> &zenoh::Session {
        &self.session
    }

    pub(crate) fn config(&self) -> &ServiceConfig {
        &self.config
    }

    pub(crate) fn impls(&self) -> &[ImplState] {
        &self.impls
    }

    pub(crate) fn availability(&self) -> &Arc<crate::operation::Availability> {
        &self.ops
    }

    /// The templates the requirement of `role` names (core §3.1):
    /// `Ok(None)` for every resource, the default, which a requirement
    /// naming none names, and so does a role the component's manifest
    /// declares: its descriptor names no declaring contract (core §3.3),
    /// so every resource of its interface is what a tool reads it to
    /// name, and the runtime reads it so too (desired.v1 §2.1, text 0.7).
    /// A lookup that fails, a role this service does not hold or a
    /// declaring contract without the requirement, is an error, never
    /// every resource.
    pub(crate) fn role_resources(&self, role: &str) -> Result<Option<Vec<String>>> {
        let r = self
            .roles
            .iter()
            .find(|r| r.role == role)
            .ok_or_else(|| Error::Contract(format!("this service has no role {role:?}")))?;
        let Some(by) = r.declared_by.as_ref() else {
            return Ok(None);
        };
        let req = find(&self.impls, by)?
            .imp
            .contract()
            .requires
            .get(role)
            .ok_or_else(|| Error::Contract(format!("{by} declares no role {role:?}")))?;
        Ok(req.resources.clone())
    }

    /// A role this service holds (§3.1): its interface, and whether it is
    /// optional.
    pub(crate) fn role_of(&self, role: &str) -> Result<(IfaceId, bool)> {
        self.roles
            .iter()
            .find(|r| r.role == role)
            .map(|r| (r.interface.clone(), r.optional))
            .ok_or_else(|| Error::Contract(format!("this service has no role {role:?}")))
    }

    /// The budget every writer of `resource` keeps (core §2.7): the
    /// runtime's own writers made outside the builder (`alarms.v1`'s).
    pub(crate) fn budget_of(
        &self,
        iface: &IfaceId,
        resource: &str,
    ) -> Option<Arc<crate::budget::Budget>> {
        let s = find(&self.impls, iface).ok()?;
        s.budget(s.imp.resource(resource).ok()?)
    }

    /// What a state writer made outside the builder is made of
    /// (`health.v1`'s checks): the session, the store its GETs are answered
    /// from, the stamp minter, and the service's close.
    pub(crate) fn state_parts(
        &self,
    ) -> (
        zenoh::Session,
        Arc<Store>,
        Arc<Minter>,
        tokio::sync::watch::Receiver<bool>,
    ) {
        (
            self.session.clone(),
            Arc::clone(&self.store),
            Arc::clone(&self.minter),
            self.closed.subscribe(),
        )
    }
}

impl Service {
    pub(crate) fn session(&self) -> &zenoh::Session {
        &self.session
    }

    pub(crate) fn served(&self) -> Arc<Served> {
        Arc::clone(&self.served)
    }

    pub(crate) fn store(&self) -> Arc<Store> {
        Arc::clone(&self.store)
    }

    pub(crate) fn config(&self) -> &ServiceConfig {
        &self.config
    }

    pub(crate) fn impls(&self) -> &[ImplState] {
        &self.impls
    }

    pub(crate) fn roles(&self) -> &[Role] {
        &self.roles
    }

    pub(crate) fn ops(&self) -> &crate::operation::Ops {
        &self.ops
    }
}
