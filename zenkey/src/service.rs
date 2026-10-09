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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

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

use crate::config::ServiceConfig;
use crate::consumer::Consumer;
use crate::descriptor;
use crate::error::{Error, Result, zenoh};
use crate::hostid::{HostIdError, HostIdMinter, Minted};
use crate::implementation::{Implementation, missing_capability, resource_name};
use crate::qos;
use crate::state::{ClockGuard, DEFAULT_WINDOW, Minter, StateWriter, Store};
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
        Self {
            session: session.clone(),
            config,
            addr,
            minted,
            host,
            instance: mint(),
            impls: Vec::new(),
            roles: Vec::new(),
            ops: Arc::default(),
            minter: Arc::new(Minter::new(session)),
            store: Arc::new(Store::new(window)),
            state_ifaces: BTreeSet::new(),
            closed: tokio::sync::watch::Sender::new(false),
        }
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
        let r = find(&self.impls, iface)?.imp.resource(resource)?.clone();
        if !matches!(r.token, KindToken::State | KindToken::ExplicitState) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} is not state"
            )));
        }
        self.expose(iface, resource)?;
        self.state_ifaces.insert(iface.clone());
        let w = Writer::declare(&self.session, key.into_keyexpr(), &r, values).await?;
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
        self.impls.push(ImplState {
            imp,
            exposed: BTreeSet::new(),
            unavailable: BTreeMap::new(),
            cardinality: BTreeMap::new(),
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
    /// it (D007).
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
        s.cardinality.insert(resource.to_owned(), n);
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
        let r = find(&self.impls, iface)?.imp.resource(resource)?.clone();
        self.expose(iface, resource)?;
        Writer::declare(&self.session, key.into_keyexpr(), &r, values).await
    }

    /// An [`EventWriter`] for an event's member, and exposes the resource.
    pub fn event_writer(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: &Bindings,
    ) -> Result<EventWriter> {
        let key = self.key(iface, resource, values)?;
        let r = find(&self.impls, iface)?.imp.resource(resource)?.clone();
        self.expose(iface, resource)?;
        EventWriter::new(&self.session, key.to_string(), &r, values)
    }

    /// Brings the service up in the order of §8.2. On any refusal, nothing
    /// alive is declared.
    pub async fn start(self) -> Result<Service> {
        // Before step 1 (hostid.v1 §2.7): a service whose minted system
        // could not be had declared nothing, and starts nothing (§2.6).
        let addr = self.address()?.clone();
        // 2. Exposure.
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
        let mut svc = Service {
            alive: BTreeMap::new(),
            instance_token: None,
            members: BTreeMap::new(),
            descriptor_q: None,
            contract_qs: Vec::new(),
            ops,
            state_qs: BTreeMap::new(),
            clock: None,
            descriptor: Arc::new(RwLock::new(Arc::from(Vec::new()))),
            session: self.session,
            config: self.config,
            addr,
            minted: self.minted,
            host: self.host,
            instance: self.instance,
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
        if let Some(key) = svc.config.clock_reference.clone() {
            svc.clock = Some(ClockGuard::start(&svc.session, &key, Arc::clone(&svc.minter)).await?);
        }
        // 3. The descriptor, then the contracts.
        let (q, current) = svc.serve_descriptor(&svc.instance.clone()).await?;
        svc.descriptor = current;
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
    // Tokens first: fields drop in declaration order.
    alive: BTreeMap<IfaceId, LivelinessToken>,
    instance_token: Option<LivelinessToken>,
    members: BTreeMap<(IfaceId, String), (InstanceId, LivelinessToken)>,
    descriptor_q: Option<Queryable<()>>,
    contract_qs: Vec<Queryable<()>>,
    ops: crate::operation::Ops,
    state_qs: BTreeMap<IfaceId, Vec<Queryable<()>>>,
    clock: Option<ClockGuard>,
    descriptor: Arc<RwLock<Arc<[u8]>>>,
    session: zenoh::Session,
    config: ServiceConfig,
    /// Resolved once at start, and kept across every re-mint (`hostid.v1`
    /// §2.7).
    addr: Addr,
    minted: Option<Minted>,
    host: Option<String>,
    instance: InstanceId,
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
        Arc::clone(&self.descriptor.read().expect("not poisoned"))
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
        Writer::declare(&self.session, key.into_keyexpr(), &r, values).await
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
        let w = Writer::declare(&self.session, key.into_keyexpr(), &r, values).await?;
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

    /// Declares `iface`'s state queryables, once: one over `state/**` and
    /// one over `@state/**` where the contract has such resources (S2).
    async fn ensure_state_server(&mut self, iface: &IfaceId) -> Result<()> {
        if self.state_qs.contains_key(iface) {
            return Ok(());
        }
        let s = find(&self.impls, iface)?;
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
            qs.push(
                self.session
                    .declare_queryable(ke)
                    .callback(move |q| store.answer(&q))
                    .await
                    .map_err(zenoh)?,
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
        EventWriter::new(&self.session, key.to_string(), &r, values)
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

    /// The interfaces for which this instance holds an interface token now.
    #[must_use]
    pub fn tokens_held(&self) -> Vec<IfaceId> {
        self.alive.keys().cloned().collect()
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
        self.publish_descriptor()
    }

    /// Mints a new instance id (§1.5) **make-before-break** (§8.1): the new
    /// descriptor, instance token and interface tokens are declared first,
    /// then the old ones are undeclared. A watcher sees one or two live
    /// instance tokens, never none.
    ///
    /// The spec's SHOULD of a new session, so that timestamps' id changes
    /// too (§4.3), is the caller's: the runtime never owns the session.
    pub async fn new_epoch(&mut self) -> Result<InstanceId> {
        let next = mint();
        let (q, current) = self.serve_descriptor(&next).await?;
        let token = self.token(instance_key(&self.addr, &next)?).await?;
        let alive = self.declare_alive(&next).await?;
        // Break: the old tokens, then the old descriptor queryable.
        let old_alive = std::mem::replace(&mut self.alive, alive);
        let old_token = self.instance_token.replace(token);
        let old_q = self.descriptor_q.replace(q);
        self.descriptor = current;
        self.instance = next.clone();
        for (_, t) in old_alive {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(t) = old_token {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(q) = old_q {
            q.undeclare().await.map_err(zenoh)?;
        }
        Ok(next)
    }

    /// Declares the member token of `value` (§8.1), for the interface's
    /// template that declares `epoch`. Returns the member's epoch.
    pub async fn declare_member(&mut self, iface: &IfaceId, value: &str) -> Result<InstanceId> {
        self.member_template(iface)?;
        let member = chunk_slug(value);
        let epoch = mint();
        let key = member_key(&self.addr, iface, &member, &epoch)?;
        let token = self.token(key).await?;
        if let Some((_, old)) = self
            .members
            .insert((iface.clone(), member), (epoch.clone(), token))
        {
            old.undeclare().await.map_err(zenoh)?;
        }
        Ok(epoch)
    }

    /// Cycles a member's token when its continuity breaks (§8.1): the new
    /// epoch's token first, then the old one. Other members are untouched.
    pub async fn cycle_member(&mut self, iface: &IfaceId, value: &str) -> Result<InstanceId> {
        self.declare_member(iface, value).await
    }

    /// Withdraws a member's token.
    pub async fn remove_member(&mut self, iface: &IfaceId, value: &str) -> Result<()> {
        if let Some((_, t)) = self.members.remove(&(iface.clone(), chunk_slug(value))) {
            t.undeclare().await.map_err(zenoh)?;
        }
        Ok(())
    }

    /// Stops its state writers' re-puts (`freshness.v1` §2.4), then
    /// undeclares the tokens, then the queryables, and waits for each.
    pub async fn close(mut self) -> Result<()> {
        self.closed.send_replace(true);
        for (_, t) in std::mem::take(&mut self.alive) {
            t.undeclare().await.map_err(zenoh)?;
        }
        for (_, (_, t)) in std::mem::take(&mut self.members) {
            t.undeclare().await.map_err(zenoh)?;
        }
        if let Some(t) = self.instance_token.take() {
            t.undeclare().await.map_err(zenoh)?;
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

    fn encode_descriptor(&self, instance: &InstanceId) -> Result<Vec<u8>> {
        let zid = self.session.zid().to_string();
        let id = descriptor::Identity {
            addr: &self.addr,
            minted: self.minted.is_some(),
            host: self.host.as_deref(),
            zid: &zid,
        };
        let d = descriptor::build(&id, instance, &self.impls, &self.roles, &self.config);
        descriptor::encode_checked(&d, &self.impls)
    }

    /// Declares the descriptor's queryable on `instance`'s key and puts the
    /// descriptor there (§3.3).
    async fn serve_descriptor(
        &self,
        instance: &InstanceId,
    ) -> Result<(Queryable<()>, Arc<RwLock<Arc<[u8]>>>)> {
        let bytes: Arc<[u8]> = self.encode_descriptor(instance)?.into();
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
        let bytes: Arc<[u8]> = self.encode_descriptor(&self.instance)?.into();
        *self.descriptor.write().expect("not poisoned") = Arc::clone(&bytes);
        let key = instance_key(&self.addr, &self.instance)?.into_keyexpr();
        self.session
            .put(key, bytes.to_vec())
            .encoding(Encoding::APPLICATION_JSON)
            .timestamp(self.session.new_timestamp())
            .wait()
            .map_err(zenoh)
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
}

impl Service {
    pub(crate) fn session(&self) -> &zenoh::Session {
        &self.session
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
