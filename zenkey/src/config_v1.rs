//! `config.v1` in the runtime: changing a running service's configuration
//! (`spec/profiles/config/v1.md`, #768).
//!
//! The owner's half of the profile. Every rule a fixture can check is
//! `zenkey-model`'s ([`zenkey_model::config`]): the validators, the window,
//! redaction, and [`Owner`], a pure state machine of one owner's
//! configuration. This module wraps that machine and adds what it cannot
//! do: the five operations served, the device's acts performed, the
//! deadline timer, the persisted layer written, and the view, the status
//! and the change events put, on the standard contract, embedded.
//!
//! ```text
//! ServiceBuilder::config_v1(startup, device) ─▶ Configuration: implements config.v1,
//!        │      serves set/{group} over the template, confirm, cancel, extend, and
//!        │      persist (or lists it unavailable, cause config), puts the view and the
//!        │      status, spawns the timer
//!        ▼ start()
//!   a call ─▶ Owner::set / confirm / cancel / extend ─▶ Step
//!              Step::Act ─▶ Device::apply / undo ─▶ Owner::finish ─▶ … ─▶ Done
//!              Done ─▶ the view and the status put (on a change), the events, then the reply
//!   the timer, at Owner::next_due ─▶ Owner::due ─▶ the reach apply, or the undo
//!   a desired document ─▶ Owner::converge ─▶ one set per group, then the report
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.1 exposure; `persist` optional | every resource served; `persist` without a persisted layer is listed unavailable, cause `config`, and the core answers it before any handler (O3) |
//! | §2.3 the view and the status put before §8.2 step 4, on every change, confirmed every 30 s, never deleted | put when [`ServiceBuilder::config_v1`] declares them; after every step that changes either, before the call's reply; their [`StateWriter`]s re-put them from the contract's `freshness.ttl_s = 60`; no call deletes them |
//! | §2.5 the order of refusals | the core's first (O2's `fanout_forbidden`, O3's `unavailable`, a key that names no member, a request that does not decode), then [`Owner::set`]'s rows 1 to 14 |
//! | §2.6–§2.11 changes, tokens, joining, the window, hot and reach, the undo once | [`Owner`]; each [`Act`] performed once on the [`Device`], its result reported with [`Owner::finish`] |
//! | §2.7 one act at a time, `busy` meanwhile | the owner is unlocked while the device acts, so a call that arrives meanwhile is answered `busy` by the model, never queued |
//! | §2.8 the deadline; §2.9 `apply_at` | the timer sleeps until [`Owner::next_due`] and calls [`Owner::due`]; every call wakes it |
//! | §2.12 a restart is a rollback | a new service is a new instance: [`Owner::start`] from the startup values, nothing pending, no key held; [`crate::Service::new_epoch`] is refused (below) |
//! | §2.13 persist | [`Device::persist_ready`] first (`unavailable`, cause `config`), then [`Owner::persist_plan`] (the values the last change set, text 0.2), the write ([`Device::persist`]; a failure is `internal`), then [`Owner::persisted`]; the owner stays locked through the write |
//! | §2.14 idempotency keys | [`Owner::set`] keeps each first answer 300 s; a restart forgets them |
//! | §2.15 sensitive values | the model's view, status, replies and edits; each request logged by [`log_request`], the device's words logged by [`device_words`] |
//! | §2.16 the startup allowlist | each group's [`GroupDecl::writable`] in [`Startup::schema`]; a group outside it answered `unavailable`, cause `config`, by the handler (core §5.1, 0.28) |
//! | §2.17 one event per change; the rate | each event [`Owner`] ends a change with, on `events/change/<ulid>`; the rate kept by row 12, from [`Startup::events`] |
//! | §2.18 desired state | [`Configuration::set_desired`]: [`Owner::converge`] in the timer's task, one `set` per group, the report in the view and the status |
//!
//! **The device is the application's** ([`Device`]): it applies a group,
//! runs the undo, and writes the persisted layer, each async and fallible.
//! Its words are logged only where [`device_words`] lets them be. An act
//! runs on a task of its own, so a device that panics is a refused apply,
//! or a failed undo with nothing read back, never an owner left busy.
//!
//! **A restart is a new service.** A process that restarts builds a new
//! [`ServiceBuilder`], which mints a new instance id (core §1.5) and starts
//! the owner from its startup values: the persisted layer's, the startup
//! file's, the defaults (§2.12). Nothing of the old instance carries over
//! but what the application passes in [`Startup`]: the persisted layer,
//! and the instants of the change events of the last hour
//! ([`Configuration::published`]), which keep the event's rate. A rate
//! bounds each member of the template, `events/change` is one member per
//! service, and a re-mint does not reset it (core §2.7), so an application
//! that keeps no such record lets a restarted owner publish up to 600 more
//! change events within the hour. A re-mint within one running service
//! ([`crate::Service::new_epoch`]) is refused: it would be a restart (a new
//! instance, the revision from 0) whose device still runs the old
//! instance's values, and the view would then show values the device does
//! not hold.
//!
//! **The contract is the published bundle**, revision 1.0
//! ([`FINGERPRINT`]), embedded byte for byte from
//! `spec/profiles/.history/config.v1/` and served as it is, as `health.v1`'s
//! is ([`crate::health`]).
//!
//! **The owner's clock** is this host's system clock, to the millisecond,
//! shifted by [`Configuration::simulate_offset`] in a test. `apply_at` and
//! `deadline` are instants on it (§2.8).

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zenkey_model::config::{
    self as model, Act, ControlOp, ControlRequest, Converge, DeviceResult, Done, Owner, Refusal,
    SetRequest, Start, Step, Time,
};
use zenkey_model::descriptor::Cause;
use zenkey_model::grammar::IfaceId;
use zenkey_model::template::Bindings;

pub use zenkey_model::config::{
    Class, ConfigChangeEvent, ConfigStatus, ConfigView, DesiredConfig, DesiredReport, GroupDecl,
    Kind, ParamDecl, Restore, Schema, Source, Value, device_words, log_request,
};

use crate::call::{BoxFuture, OpError};
use crate::error::{Error, Result};
use crate::implementation::Implementation;
use crate::operation::{Call, OperationServer};
use crate::service::ServiceBuilder;
use crate::state::StateWriter;
use crate::writer::EventWriter;

/// The standard contract's bundle, revision 1.0 as published in
/// `spec/profiles/.history/config.v1/` (§3), byte for byte.
pub const BUNDLE: &[u8] = include_bytes!(
    "../../spec/profiles/.history/config.v1/727ff0090b99ad8fc09b5398804d1a14288bebf661f374ec6904351344a1a7b1.bundle.json"
);

/// Its fingerprint (§3).
pub const FINGERPRINT: &str = "727ff0090b99ad8fc09b5398804d1a14288bebf661f374ec6904351344a1a7b1";

/// The read-back, as the descriptor names it (§2.3).
pub const VIEW: &str = "state/view";
/// The compact status, as the descriptor names it (§2.19).
pub const STATUS: &str = "state/status";
/// The set operation's template (§2.5).
pub const SET: &str = "@op/set/{group}";
/// `confirm` (§2.10).
pub const CONFIRM: &str = "@op/confirm";
/// `cancel` (§2.10).
pub const CANCEL: &str = "@op/cancel";
/// `extend` (§2.10).
pub const EXTEND: &str = "@op/extend";
/// `persist`, optional (§2.13).
pub const PERSIST: &str = "@op/persist";
/// The change event (§2.17).
pub const CHANGE: &str = "events/change";

/// Why `persist` is listed unavailable without a persisted layer (§2.13).
pub const NO_LAYER: &str = "this service has no persisted layer (config.v1 §2.13)";

static IMPLEMENTATION: LazyLock<Implementation> = LazyLock::new(|| {
    Implementation::from_bundle(BUNDLE).expect("the published config.v1 bundle verifies")
});

/// The standard contract's interface id, `config.v1`.
#[must_use]
pub fn iface() -> IfaceId {
    model::IFACE.parse().expect("config.v1 is an interface id")
}

/// The standard contract, implemented from the embedded bundle: what
/// [`ServiceBuilder::config_v1`] implements, and what a reader of
/// `config.v1` is compiled against (R4).
#[must_use]
pub fn implementation() -> Implementation {
    IMPLEMENTATION.clone()
}

/// Values by group and parameter: a startup layer, the persisted layer,
/// what a device reads back.
pub type Values = BTreeMap<String, BTreeMap<String, Value>>;

/// What an owner starts from (§2.2, §2.3, §2.8, §2.13): its schema, its
/// startup layers and its device's facts. Every instance starts from
/// these, never from a runtime change of an earlier one (§2.12).
#[derive(Debug, Clone, Default)]
pub struct Startup {
    /// The groups and their parameters, each group's class and whether the
    /// startup allowlist lets a `set` reach it ([`GroupDecl::writable`],
    /// §2.16). At most 16 groups.
    pub schema: Schema,
    /// The owner's defaults, `source: default`.
    pub defaults: Values,
    /// Its startup file's values, `source: file`, over the defaults.
    pub file: Values,
    /// The persisted layer's values, `source: overlay`, over both; `None`
    /// when the owner has no persisted layer, which lists `persist`
    /// unavailable (§2.13). A parameter in no layer starts unknown.
    pub persisted: Option<Values>,
    /// The floor, in seconds, at least 1 (§2.8).
    pub floor_s: u64,
    /// How long after receiving a reach `set` its device applies it: the
    /// `apply_at` it states (§2.9), below the floor.
    pub apply_after: Duration,
    /// When the service's change events were published within the last
    /// hour, by an earlier instance: the record that keeps the event's rate
    /// across a restart (§2.17, core §2.7), from
    /// [`Configuration::published`].
    pub events: Vec<SystemTime>,
}

impl Startup {
    fn start(&self, service: String, instance: String) -> Result<Owner> {
        let mut startup: BTreeMap<String, BTreeMap<String, (Value, Source)>> = BTreeMap::new();
        for (layer, source) in [
            (&self.defaults, Source::Default),
            (&self.file, Source::File),
        ] {
            for (g, ps) in layer {
                for (p, v) in ps {
                    startup
                        .entry(g.clone())
                        .or_default()
                        .insert(p.clone(), (v.clone(), source));
                }
            }
        }
        Owner::start(Start {
            service,
            instance,
            schema: self.schema.clone(),
            startup,
            overlay: self.persisted.clone().unwrap_or_default(),
            floor_s: self.floor_s,
            apply_after: self.apply_after,
            persist: self.persisted.is_some(),
            events: self.events.iter().map(|t| time_of(*t)).collect(),
        })
        .map_err(|e| Error::Contract(format!("config.v1: {e}")))
    }
}

/// A failed undo (§2.11): the device's words, and what the owner reads back
/// from it, `{group: {parameter: value}}`, a parameter not in it unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoFailed {
    /// The device's words. Logged only where [`device_words`] lets them be.
    pub words: String,
    /// What the device holds now, as far as it can tell.
    pub readback: Values,
}

/// The device a `config.v1` owner configures: the application's
/// (§0, "Device"). The runtime calls it for one act at a time, and never
/// while it holds another act in flight (§2.7).
///
/// The futures are boxed ([`BoxFuture`]) so that a device is a trait
/// object: write `Box::pin(async move { … })`. A method must not call back
/// into the [`Configuration`], which waits for it.
pub trait Device: Send + Sync + 'static {
    /// Applies `values` to `group` (§2.9), whole or not at all: `Err` is
    /// the device's refusal, in its words, and leaves nothing applied. A
    /// hot group's refusal is the call's `device-refused`; a reach group's,
    /// at its `apply_at`, ends the change by its undo.
    fn apply<'a>(
        &'a self,
        group: &'a str,
        values: &'a BTreeMap<String, Value>,
    ) -> BoxFuture<'a, std::result::Result<(), String>>;

    /// The undo, once (§2.11): restores each parameter to its value before
    /// the change. Never retried, whatever it returns. `Err` ends the
    /// change `partial`, and what it reads back is the truth the view then
    /// shows.
    fn undo<'a>(
        &'a self,
        restore: &'a [Restore],
    ) -> BoxFuture<'a, std::result::Result<(), UndoFailed>>;

    /// Whether the persisted layer can be written now (§2.13): `Err`, with
    /// why, answers `persist` `unavailable` with cause `config`, first of
    /// its refusals. By default it can.
    fn persist_ready(&self) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Writes the last change's values into the persisted layer (§2.13),
    /// never a sensitive one: the model refuses that change first. `Err`
    /// answers `internal` (a full disk, say), and nothing reads `overlay`.
    /// Called only when [`Startup::persisted`] is `Some`.
    fn persist<'a>(
        &'a self,
        entries: &'a Values,
    ) -> BoxFuture<'a, std::result::Result<(), String>> {
        let _ = entries;
        Box::pin(async { Err("this device has no persisted layer".to_owned()) })
    }
}

/// The instant `t` on the owner's clock (§2.8).
fn time_of(t: SystemTime) -> Time {
    let ms = t
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_else(|e| -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX));
    Time::from_millis(ms)
}

fn system_of(t: Time) -> SystemTime {
    match u64::try_from(t.millis()) {
        Ok(ms) => UNIX_EPOCH + Duration::from_millis(ms),
        Err(_) => UNIX_EPOCH - Duration::from_millis(t.millis().unsigned_abs()),
    }
}

/// A model refusal as the core's envelope (§2.5): `unavailable` with cause
/// `config`, `app` with the `ConfigError`, every other code bare.
fn envelope(r: Refusal) -> OpError {
    match r.code {
        model::Code::InvalidRequest => OpError::invalid_request(r.message),
        model::Code::NotFound => OpError::not_found(r.message),
        model::Code::Unavailable => OpError::unavailable(Cause::Config, r.message),
        model::Code::Forbidden => OpError::forbidden(r.message),
        model::Code::Busy => OpError::busy(r.message),
        model::Code::App => match &r.app {
            Some(detail) => OpError::app(r.message, detail),
            None => OpError::app_without_detail(r.message),
        },
    }
}

/// The call metadata a call claims (core O7), as the model takes it.
fn claimed(call: &Call) -> model::Call {
    let m = call.metadata();
    model::Call {
        actor: m.and_then(|m| m.actor.clone()),
        request_id: m.and_then(|m| m.request_id.clone()),
    }
}

/// What a step left to publish, in order: the view after it, and the events
/// it ended changes with.
struct Out {
    view: ConfigView,
    events: Vec<ConfigChangeEvent>,
}

/// The owner and what it owes the bus. Every step is taken under its lock;
/// a device act is performed with it released.
struct Core {
    owner: Owner,
    /// Published in order by whoever flushes next.
    queue: VecDeque<Out>,
    /// The desired document held, the latest given (§2.18).
    desired: Option<DesiredConfig>,
    /// The revision of the last document acted on.
    acted: Option<u64>,
}

impl Core {
    /// Queues the view as it stands, and `events`, for the next flush.
    fn enqueue(&mut self, events: &[ConfigChangeEvent]) {
        let view = self.owner.view();
        self.queue.push_back(Out {
            view,
            events: events.to_vec(),
        });
    }

    /// Queues what a step answered: a step that acts changes nothing a
    /// reader may see until its act ends.
    fn after(&mut self, step: &Step) {
        if let Step::Done(d) = step {
            self.enqueue(&d.events);
        }
    }
}

/// A service's `config.v1` (§2), from [`ServiceBuilder::config_v1`], and
/// again from [`crate::Service::config_v1`] once it runs.
///
/// A clone is the same handle. Its operations are served, and its timer
/// runs, until the service closes or drops; the handle stays readable
/// after.
#[derive(Clone)]
pub struct Configuration {
    inner: Arc<Inner>,
}

struct Inner {
    iface: IfaceId,
    /// The owner's address, `<system>/<service>`, as the view names it.
    service: String,
    instance: String,
    schema: Schema,
    device: Arc<dyn Device>,
    core: tokio::sync::Mutex<Core>,
    /// The view last put; held while a flush puts, so puts go in order.
    published: tokio::sync::Mutex<Option<ConfigView>>,
    view: StateWriter,
    status: StateWriter,
    change: EventWriter,
    /// Every step, every desired document and every clock shift wakes the
    /// timer.
    wake: tokio::sync::Notify,
    offset_ms: AtomicI64,
    /// The five operations' servers, until the service closes
    /// ([`Configuration::shutdown`]) or drops (the timer, as it ends). Their
    /// handlers hold this `Inner`, so taking them out is what frees it.
    servers: std::sync::Mutex<Vec<OperationServer>>,
}

impl Configuration {
    /// Implements `config.v1` on `b` (§2.1): the contract, the view and the
    /// status put, the operations served, the timer spawned.
    pub(crate) async fn declare(
        b: &mut ServiceBuilder,
        startup: Startup,
        device: Arc<dyn Device>,
    ) -> Result<Self> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|e| Error::Contract(format!("config.v1 needs a tokio runtime: {e}")))?;
        let addr = b.address()?.clone();
        let instance = b.instance().to_string();
        let owner = startup.start(addr.to_string(), instance.clone())?;
        let imp = implementation();
        let iface = imp.iface().clone();
        b.implement(imp)?;
        let none = Bindings::new();
        let view = b.declare_state_writer(&iface, VIEW, &none).await?;
        let status = b.declare_state_writer(&iface, STATUS, &none).await?;
        let change = b.event_writer(&iface, CHANGE, &none)?;
        let inner = Arc::new(Inner {
            iface: iface.clone(),
            service: addr.to_string(),
            instance,
            schema: startup.schema.clone(),
            device,
            core: tokio::sync::Mutex::new(Core {
                owner,
                queue: VecDeque::new(),
                desired: None,
                acted: None,
            }),
            published: tokio::sync::Mutex::new(None),
            view,
            status,
            change,
            wake: tokio::sync::Notify::new(),
            offset_ms: AtomicI64::new(0),
            servers: std::sync::Mutex::new(Vec::new()),
        });
        // §2.3: the view and the status before §8.2 step 4, put here, before
        // the descriptor lists config.v1 and before any token.
        inner.core.lock().await.enqueue(&[]);
        inner.flush().await;

        let mut servers: Vec<OperationServer> = Vec::new();
        let i = Arc::clone(&inner);
        servers.push(
            b.serve(&iface, SET, None, move |call| {
                let i = Arc::clone(&i);
                async move { i.on_set(call).await }
            })
            .await?,
        );
        for (resource, op) in [
            (CONFIRM, ControlOp::Confirm),
            (CANCEL, ControlOp::Cancel),
            (EXTEND, ControlOp::Extend),
        ] {
            let i = Arc::clone(&inner);
            servers.push(
                b.serve(&iface, resource, Some(&none), move |call| {
                    let i = Arc::clone(&i);
                    async move { i.on_control(op, call).await }
                })
                .await?,
            );
        }
        if startup.persisted.is_some() {
            let i = Arc::clone(&inner);
            servers.push(
                b.serve(&iface, PERSIST, Some(&none), move |call| {
                    let i = Arc::clone(&i);
                    async move { i.on_persist(call).await }
                })
                .await?,
            );
        } else {
            b.unavailable(&iface, PERSIST, Cause::Config, Some(NO_LAYER))?;
        }

        *inner.servers.lock().expect("not poisoned") = servers;
        // The timer: until the service closes or drops, when the servers go
        // with it.
        let (_, _, _, closed) = b.state_parts();
        rt.spawn(Arc::clone(&inner).run(closed));
        let cfg = Self { inner };
        b.config_v1 = Some(cfg.clone());
        Ok(cfg)
    }

    /// Undeclares the operations' servers, and waits for it: the service
    /// closes ([`crate::Service::close`]), after its tokens. A restarted
    /// instance then never shares a key with this one.
    pub(crate) async fn shutdown(&self) -> Result<()> {
        let servers = std::mem::take(&mut *self.inner.servers.lock().expect("not poisoned"));
        for s in servers {
            s.undeclare().await?;
        }
        Ok(())
    }

    /// `config.v1`.
    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.inner.iface
    }

    /// The instance id the configuration runs in (§2.3), the service's.
    #[must_use]
    pub fn instance(&self) -> &str {
        &self.inner.instance
    }

    /// The schema served (§2.2).
    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.inner.schema
    }

    /// The read-back as the owner holds it now (§2.3). Between an act's
    /// start and its end it may run ahead of the view last put, which is
    /// what a reader sees.
    pub async fn view(&self) -> ConfigView {
        self.inner.core.lock().await.owner.view()
    }

    /// The compact status as the owner holds it now (§2.19).
    pub async fn status(&self) -> ConfigStatus {
        self.inner.core.lock().await.owner.status()
    }

    /// The desired report the view and the status carry (§2.18), if any.
    pub async fn desired(&self) -> Option<DesiredReport> {
        self.inner.core.lock().await.owner.desired().cloned()
    }

    /// Gives the owner a desired document to converge (§2.18), the one a
    /// deployment binds it to: the application reads it through its own
    /// role. The owner refuses it whole, or converges it through one `set`
    /// without a window per group whose values differ, waiting while a
    /// change is pending; the outcome is the `desired` report of the view
    /// and the status. A document whose revision the owner already acted
    /// on is not acted on again.
    pub async fn set_desired(&self, doc: DesiredConfig) {
        self.inner.core.lock().await.desired = Some(doc);
        self.inner.wake.notify_one();
    }

    /// When the change events of the last hour were published, on the
    /// owner's clock: the record the next instance starts from
    /// ([`Startup::events`]), so the event's rate holds across a restart
    /// (§2.17, core §2.7).
    pub async fn published(&self) -> Vec<SystemTime> {
        let now = self.inner.now();
        let c = self.inner.core.lock().await;
        c.owner.published(now).into_iter().map(system_of).collect()
    }

    /// The owner's clock now (§2.8).
    #[must_use]
    pub fn now(&self) -> SystemTime {
        system_of(self.inner.now())
    }

    /// Shifts the owner's clock by `ms` from this host's (negative:
    /// behind), and runs what is owed at the new instant. For tests and
    /// simulation, as [`crate::state::Minter::simulate_offset`] is; a
    /// deployment never sets it. State stamps are not shifted.
    pub fn simulate_offset(&self, ms: i64) {
        self.inner.offset_ms.store(ms, Ordering::Relaxed);
        self.inner.wake.notify_one();
    }

    /// The view's writer: zenoh stays reachable.
    #[must_use]
    pub fn view_writer(&self) -> &StateWriter {
        &self.inner.view
    }

    /// The status's writer: zenoh stays reachable.
    #[must_use]
    pub fn status_writer(&self) -> &StateWriter {
        &self.inner.status
    }
}

impl Inner {
    fn now(&self) -> Time {
        let t = time_of(SystemTime::now());
        Time::from_millis(
            t.millis()
                .saturating_add(self.offset_ms.load(Ordering::Relaxed)),
        )
    }

    /// Performs one act on the device, on a task of its own, and says what
    /// the device said.
    async fn perform(&self, act: &Act) -> DeviceResult {
        match act {
            Act::Apply { group, values } => tracing::info!(
                service = %self.service, instance = %self.instance,
                group = %group,
                parameters = values.len(),
                "config.v1: apply"
            ),
            Act::Undo { restore } => tracing::info!(
                service = %self.service, instance = %self.instance,
                parameters = restore.len(),
                "config.v1: undo"
            ),
        }
        let device = Arc::clone(&self.device);
        let owned = act.clone();
        let joined = tokio::spawn(async move {
            match &owned {
                Act::Apply { group, values } => device
                    .apply(group, values)
                    .await
                    .map_err(DeviceResult::Refused),
                Act::Undo { restore } => {
                    device
                        .undo(restore)
                        .await
                        .map_err(|f| DeviceResult::Failed {
                            words: f.words,
                            readback: f.readback,
                        })
                }
            }
        })
        .await;
        let result = match joined {
            Ok(Ok(())) => DeviceResult::Ok,
            Ok(Err(r)) => r,
            Err(e) => {
                let words = format!("the device's act did not complete: {e}");
                match act {
                    Act::Apply { .. } => DeviceResult::Refused(words),
                    Act::Undo { .. } => DeviceResult::Failed {
                        words,
                        readback: Values::new(),
                    },
                }
            }
        };
        if let DeviceResult::Refused(words) | DeviceResult::Failed { words, .. } = &result {
            let words = device_words(&self.schema, act, words).unwrap_or(model::REDACTED);
            tracing::warn!(
                service = %self.service, instance = %self.instance,
                words = %words,
                "config.v1: the device {} (config.v1 §2.9, §2.11)",
                if matches!(act, Act::Apply { .. }) {
                    "refused an apply"
                } else {
                    "failed an undo, once: never retried"
                }
            );
        }
        result
    }

    /// Takes `step` to its end: each act performed, its result reported,
    /// until the owner answers.
    async fn drive(&self, mut step: Step) -> Done {
        loop {
            match step {
                Step::Done(done) => return done,
                Step::Act(act) => {
                    let result = self.perform(&act).await;
                    let mut c = self.core.lock().await;
                    step = c.owner.finish(self.now(), result);
                    c.after(&step);
                }
            }
        }
    }

    /// Puts what the steps queued, in order: the view and the status when
    /// the view changed (§2.3), then the events (§2.17).
    async fn flush(&self) {
        let mut last = self.published.lock().await;
        loop {
            let Some(out) = self.core.lock().await.queue.pop_front() else {
                break;
            };
            if last.as_ref() != Some(&out.view) {
                let status = ConfigStatus::of(&out.view);
                if let Err(e) = self.view.put_value(&out.view).await {
                    tracing::warn!(service = %self.service, instance = %self.instance, "config.v1: the view was not put: {e}");
                }
                if let Err(e) = self.status.put_value(&status).await {
                    tracing::warn!(service = %self.service, instance = %self.instance, "config.v1: the status was not put: {e}");
                }
                *last = Some(out.view);
            }
            for ev in &out.events {
                tracing::info!(
                    service = %ev.service, instance = %ev.instance,
                    token = %ev.token,
                    outcome = ev.outcome.as_str(),
                    revision = ev.revision,
                    "config.v1: a change ended"
                );
                let sent = match serde_json::to_vec(ev) {
                    Ok(bytes) => self.change.put(bytes).await.map(|_| ()),
                    Err(e) => Err(Error::Payload(e.to_string())),
                };
                if let Err(e) = sent {
                    // §2.17: never dropped by the owner's own rate, which
                    // row 12 keeps; this is the bus refusing it.
                    tracing::error!(service = %ev.service, instance = %ev.instance, token = %ev.token, "config.v1: a change event was not published: {e}");
                }
            }
        }
    }

    /// The reply a call is owed, or its refusal.
    async fn answer(&self, call: &Call, done: Done) -> std::result::Result<(), OpError> {
        match done.reply {
            Some(Ok(reply)) => call
                .reply_value(&reply.to_json())
                .await
                .map_err(OpError::from),
            Some(Err(r)) => {
                tracing::info!(
                    service = %self.service, instance = %self.instance,
                    code = r.code.as_str(),
                    why = %r.message,
                    "config.v1: refused"
                );
                Err(envelope(r))
            }
            None => Err(OpError::internal("config.v1: the owner answered nothing")),
        }
    }

    /// `@op/set/{group}` (§2.5).
    async fn on_set(&self, call: Call) -> std::result::Result<(), OpError> {
        let group = call
            .values()
            .and_then(|v| v.get("group"))
            .and_then(|v| v.first())
            .cloned()
            .ok_or_else(|| OpError::internal("config.v1: a set names no group"))?;
        let req: SetRequest = call.request()?;
        let who = claimed(&call);
        tracing::info!(
            service = %self.service, instance = %self.instance,
            group = %group,
            request = %log_request(&self.schema, &group, &req),
            actor = who.actor.as_deref().unwrap_or("-"),
            request_id = who.request_id.as_deref().unwrap_or("-"),
            "config.v1: set"
        );
        let step = {
            let mut c = self.core.lock().await;
            let s = c.owner.set(self.now(), &group, &req, &who);
            c.after(&s);
            s
        };
        let done = self.drive(step).await;
        self.flush().await;
        self.wake.notify_one();
        self.answer(&call, done).await
    }

    /// `confirm`, `cancel` and `extend` (§2.10).
    async fn on_control(&self, op: ControlOp, call: Call) -> std::result::Result<(), OpError> {
        let req: ControlRequest = call.request()?;
        let who = claimed(&call);
        tracing::info!(
            service = %self.service, instance = %self.instance,
            token = %req.token,
            actor = who.actor.as_deref().unwrap_or("-"),
            request_id = who.request_id.as_deref().unwrap_or("-"),
            "config.v1: {}",
            op.as_str()
        );
        let step = {
            let mut c = self.core.lock().await;
            let now = self.now();
            let s = match op {
                ControlOp::Confirm => c.owner.confirm(now, &req, &who),
                ControlOp::Cancel => c.owner.cancel(now, &req, &who),
                ControlOp::Extend => c.owner.extend(now, &req, &who),
                ControlOp::Persist => unreachable!("persist is on_persist's"),
            };
            c.after(&s);
            s
        };
        let done = self.drive(step).await;
        self.flush().await;
        self.wake.notify_one();
        self.answer(&call, done).await
    }

    /// `persist` (§2.13): served only with a persisted layer.
    async fn on_persist(&self, call: Call) -> std::result::Result<(), OpError> {
        // §2.13 row 1: a layer that cannot be written now, as no layer is,
        // first of the handler's refusals.
        if let Err(why) = self.device.persist_ready() {
            return Err(OpError::unavailable(
                Cause::Config,
                format!("the persisted layer cannot be written now: {why} (config.v1 §2.13)"),
            ));
        }
        let req: ControlRequest = call.request()?;
        tracing::info!(
            service = %self.service, instance = %self.instance,
            token = %req.token,
            "config.v1: persist"
        );
        let done = {
            // Locked through the write: no step lands between the plan and
            // its commit.
            let mut c = self.core.lock().await;
            match c.owner.persist_plan(self.now(), &req) {
                Err(r) => Done {
                    reply: Some(Err(r)),
                    events: Vec::new(),
                },
                Ok(plan) => {
                    if let Err(why) = self.device.persist(&plan.entries).await {
                        tracing::warn!(
                            service = %self.service, instance = %self.instance,
                            "config.v1: the persisted layer was not written: {why}"
                        );
                        return Err(OpError::internal(format!(
                            "the persisted layer was not written: {why} (config.v1 §2.13)"
                        )));
                    }
                    let done = c.owner.persisted(plan);
                    c.enqueue(&done.events);
                    done
                }
            }
        };
        self.flush().await;
        self.wake.notify_one();
        self.answer(&call, done).await
    }

    /// Every act owed now (§2.9, §2.11), each to its end.
    async fn run_due(&self) {
        loop {
            let step = {
                let mut c = self.core.lock().await;
                let s = c.owner.due(self.now());
                if let Some(s) = &s {
                    c.after(s);
                }
                s
            };
            let Some(step) = step else { break };
            self.drive(step).await;
            self.flush().await;
        }
    }

    /// Converges the desired document held, as far as it can now (§2.18).
    async fn converge(&self) {
        loop {
            let (revision, group, step) = {
                let mut c = self.core.lock().await;
                let Some(doc) = c.desired.clone() else { return };
                if c.acted == Some(doc.revision) {
                    return;
                }
                match c.owner.converge(&doc) {
                    Converge::Wait => return,
                    Converge::Refused(report) | Converge::Converged(report) => {
                        tracing::info!(
                            service = %self.service, instance = %self.instance,
                            revision = doc.revision,
                            report = %serde_json::to_string(&report).unwrap_or_default(),
                            "config.v1: a desired document acted on"
                        );
                        c.owner.report_desired(Some(report));
                        c.acted = Some(doc.revision);
                        c.enqueue(&[]);
                        drop(c);
                        self.flush().await;
                        return;
                    }
                    Converge::Set { group, request } => {
                        let s = c
                            .owner
                            .set(self.now(), &group, &request, &model::Call::default());
                        c.after(&s);
                        (doc.revision, group, s)
                    }
                }
            };
            let done = self.drive(step).await;
            if let Some(Err(refusal)) = &done.reply {
                let report = model::desired_failed(revision, &group, refusal);
                tracing::info!(
                    service = %self.service, instance = %self.instance,
                    revision,
                    report = %serde_json::to_string(&report).unwrap_or_default(),
                    "config.v1: a desired document failed"
                );
                let mut c = self.core.lock().await;
                c.owner.report_desired(Some(report));
                c.acted = Some(revision);
                c.enqueue(&[]);
            }
            self.flush().await;
        }
    }

    /// The timer (§2.8, §2.9, §2.11) and the desired document (§2.18),
    /// until the service closes or drops; the servers go with it.
    async fn run(self: Arc<Self>, closed: tokio::sync::watch::Receiver<bool>) {
        self.timer(closed).await;
        drop(std::mem::take(
            &mut *self.servers.lock().expect("not poisoned"),
        ));
    }

    async fn timer(&self, mut closed: tokio::sync::watch::Receiver<bool>) {
        loop {
            if *closed.borrow() {
                return;
            }
            let next = {
                let c = self.core.lock().await;
                (!c.owner.in_flight()).then(|| c.owner.next_due()).flatten()
            };
            let now = self.now();
            let sleep = async move {
                match next {
                    Some(t) => {
                        let ms =
                            u64::try_from(t.millis().saturating_sub(now.millis())).unwrap_or(0);
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                changed = closed.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue;
                }
                () = self.wake.notified() => {}
                () = sleep => {}
            }
            self.run_due().await;
            self.converge().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FINGERPRINT, implementation};

    /// The embedded bundle is the published revision, and names every
    /// resource the runtime serves.
    #[test]
    fn the_embedded_contract_is_the_published_one() {
        let imp = implementation();
        assert_eq!(imp.fingerprint().hex().to_string(), FINGERPRINT);
        assert_eq!(&**imp.bundle_bytes(), super::BUNDLE, "served byte for byte");
        for r in [
            super::VIEW,
            super::STATUS,
            super::SET,
            super::CONFIRM,
            super::CANCEL,
            super::EXTEND,
            super::PERSIST,
            super::CHANGE,
        ] {
            assert!(imp.resource(r).is_ok(), "{r}");
        }
        assert!(imp.resource(super::PERSIST).unwrap().optional);
        assert_eq!(
            zenkey_model::freshness::horizon_of(imp.resource(super::VIEW).unwrap()).refresh_bound(),
            Some(std::time::Duration::from_secs(30))
        );
    }

    /// An application shares the handle across tasks, and a device is a
    /// trait object.
    #[test]
    fn the_handle_is_shared_across_tasks() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        fn object(_: &dyn super::Device) {}
        shared::<super::Configuration>();
        let _ = object;
    }
}
