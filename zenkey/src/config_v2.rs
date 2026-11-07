//! `config.v2` in the runtime: a configuration declared in its contract,
//! read back in one packed record (`spec/profiles/config/v2.md`, #985 G2).
//!
//! The owner's half of the profile. Every rule a fixture can check is
//! `zenkey-model`'s ([`zenkey_model::config::v2`]): the declaration, the
//! derived types, the record, the requests, and [`v2::Owner`], a pure state
//! machine over `config.v1`'s, which stamps what it writes through a
//! [`Mint`]. This module wraps that machine as [`crate::config_v1`] wraps
//! `config.v1`'s, over the service's own config contract, and adds what it
//! cannot do: the operations served, the device's acts performed, the
//! timer, the persisted layer written, and the record and the change events
//! put, each under the stamp the model gave it.
//!
//! ```text
//! ServiceBuilder::config_v2(startup, device) ─▶ Configuration: the service's config contract,
//!        │      linted and read; set/<g> served per settable group, confirm, cancel, extend,
//!        │      and persist (or listed unavailable, cause config); the record put; the timer
//!        ▼ start()
//!   a call ─▶ Owner::set / confirm / cancel / extend / persist ─▶ Step
//!              Step::Act ─▶ Device::apply / undo ─▶ Owner::finish ─▶ … ─▶ Output
//!              Output ─▶ its puts, as stamped, then its reply, then its events
//!   the timer, at Owner::next_due ─▶ Owner::due ─▶ the reach apply, or the undo
//!   a desired document ─▶ Owner::converge ─▶ one set per group, then Owner::report_desired
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.1 one config contract; every resource but `persist` | [`ServiceBuilder::config_v2`] reads the one implemented interface whose contract lists `config.v2` in `uses`, refuses none, two, or `config.v1` beside it; `set/<g>` is served for each settable group, `persist` with a persisted layer, else listed unavailable, cause `config` |
//! | §2.2 the declaration and the lint | `zenkey_model::config::v2::lint_contract`: a finding refuses the start, nothing put; the device's runtime schema, when [`Startup::device_schema`] gives it, checked against the contract (Appendix B) |
//! | §2.3 the record put before §8.2 step 4, on every change, never re-put, never deleted | put when [`ServiceBuilder::config_v2`] declares it, and wherever an [`Output`] carries a put; the contract uses `freshness.v2`, so the heartbeat covering the config interface vouches for it |
//! | §2.5 the order of refusals | the core's first (O2's `fanout_forbidden`, O3's `unavailable`, a request §7.5 or `Set<T>`'s layout refuses), then the model's rows |
//! | §2.6 tokens | the model's, over [`Mint`]: the run and seq the service's [`Minter`]'s, shared with every state key and event; a new run before the counter passes 65,535; each run's counter start drawn at random once |
//! | §2.8 times | the owner's clock (this host's, shifted by [`Configuration::simulate_offset`]); the deployment epoch the service's catalog's, else 0 |
//! | §2.9 put first, then the reply; the as-of mark | each [`Output`] written in order by one flush, the reply sent from it in the value or the error form, as stamped |
//! | §2.14 the first answer, byte for byte | the model's: a retry gets the bytes it held |
//! | §2.15 sensitive values | the model's record, replies and events; each request logged through [`Interface::log_set`], the device's words through `config::device_words` |
//! | §2.17 the event after the reply | the [`Output`]'s order |
//! | §2.18 desired state | [`Configuration::set_desired`]: [`v2::Owner::converge`] in the timer's task, one `set` per group, then [`v2::Owner::report_desired`] |
//!
//! **The device** is `config.v1`'s [`Device`]: the acts are config.v1's,
//! and a value is an integer or a boolean. **A restart is a new service**,
//! as with `config.v1`: a new instance number, every key forgotten.
//!
//! **A re-mint is a restart** (v1 §2.12, which §2.12 keeps; #1064), as
//! [`crate::config_v1`]'s is: [`crate::Service::new_epoch`] on a running
//! service restarts its configuration in the new instance, with its device
//! running its startup values first:
//!
//! ```text
//! new_epoch ─▶ Configuration::restarting: a pending change cancelled, as `cancel` does
//!                 (its undo, its record, its event); then Owner::restart_plan, each
//!                 parameter off its startup value ─▶ Device::restore_startup, once, if any;
//!                 the owner held
//!          ─▶ the new instance's descriptor and run ─▶ Restarting::start: Owner::restart_in
//!                 (its instance number the new run, revision 0, nothing pending, no last
//!                 change, no key held, the last hour's events kept), the record put ─▶ the tokens
//! ```
//!
//! Refused, the instance running on, while an act is in flight, when a
//! parameter set at runtime starts unknown, when the device refuses to
//! return (by default it cannot), and for an owner fed through `desired.v1`
//! (`desired.v1` §2.16). A cancel made before a refusal stays made. A late
//! confirm with a token of the old instance is `not_found`: its run is the
//! old instance's. A new run within the instance
//! ([`crate::Service::new_run`], core §4.3) restarts nothing.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use zenkey_model::config::v2::{self, Answer, Converge, Mint, Output, Step};
use zenkey_model::config::{self as model, Act, DeviceResult, Restore, Source, Time};
use zenkey_model::descriptor::Cause;
use zenkey_model::grammar::IfaceId;
use zenkey_model::template::Bindings;
use zenkey_model::wire::{Run, Seq, Stamp};

pub use crate::config_v1::{Device, UndoFailed, Values};
pub use zenkey_model::config::v2::{Interface, State};
pub use zenkey_model::config::{Class, DesiredConfig, DesiredReport, Kind, Schema, Value};

use crate::call::OpError;
use crate::config_v1::{
    DESIRED_WAIT_IN_FLIGHT, DESIRED_WAIT_PAST_DEADLINE, DesiredEnd, RESTART_TRIES,
};
use crate::error::{Error, Result};
use crate::operation::{Call, OperationServer};
use crate::service::ServiceBuilder;
use crate::state::{Minter, StateWriter};
use crate::writer::EventWriter;

/// The record, as the descriptor names it (§2.3).
pub const RECORD: &str = "state/config";
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

/// A settable group's `set`, as the descriptor names it (§2.5).
#[must_use]
pub fn set(group: &str) -> String {
    format!("@op/{}{group}", v2::SET_PREFIX)
}

/// Why `persist` is listed unavailable without a persisted layer (§2.13).
pub const NO_LAYER: &str = "this service has no persisted layer (config.v2 §2.13)";

/// What an owner starts from (§2.3, §2.8, §2.13, §2.16): its startup
/// layers, its device's facts and the allowlist. The schema is the
/// contract's. Every instance starts from these, never from a runtime
/// change of an earlier one (§2.12).
#[derive(Debug, Clone, Default)]
pub struct Startup {
    /// The owner's defaults, `source: default`.
    pub defaults: Values,
    /// Its startup file's values, `source: file`, over the defaults.
    pub file: Values,
    /// The persisted layer's values, `source: overlay`, over both; `None`
    /// when the owner has no persisted layer, which lists `persist`
    /// unavailable (§2.13). A parameter in no layer starts unknown.
    pub persisted: Option<Values>,
    /// The floor, from 1 to 65,535 s (§2.8).
    pub floor_s: u16,
    /// How long after receiving a reach `set` its device applies it: the
    /// instant it states, rounded up to `apply_at` (§2.8, §2.9).
    pub apply_after: Duration,
    /// The startup allowlist (§2.16): the settable groups a `set` may
    /// reach. `None`: every settable group.
    pub writable: Option<BTreeSet<String>>,
    /// When the service's change events were published within the last
    /// hour, by an earlier instance ([`Configuration::published`]): the
    /// record that keeps the event's rate across a restart (§2.17).
    pub events: Vec<SystemTime>,
    /// The device's runtime schema, when the application reads one from
    /// it: checked against the contract's at start, and a difference in a
    /// group, a class, a parameter, a kind, a bound or a sensitive mark
    /// refuses the start, since the record would state bounds the device
    /// does not hold (Appendix B). Descriptions, units and `writable` are
    /// not compared.
    pub device_schema: Option<Schema>,
}

impl Startup {
    fn layers(&self) -> BTreeMap<String, BTreeMap<String, (Value, Source)>> {
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
        startup
    }
}

/// Where the device's schema and the contract's differ, if they do
/// (Appendix B): the first difference, for a person.
fn schema_differs(contract: &Schema, device: &Schema) -> Option<String> {
    let names = |s: &Schema| s.groups.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
    if names(contract) != names(device) {
        return Some(format!(
            "the groups: the contract's {:?}, the device's {:?}",
            names(contract),
            names(device)
        ));
    }
    for (c, d) in contract.groups.iter().zip(&device.groups) {
        if c.class != d.class {
            return Some(format!(
                "{}'s class: the contract's {}, the device's {}",
                c.name,
                c.class.as_str(),
                d.class.as_str()
            ));
        }
        let params = |g: &model::GroupDecl| {
            g.parameters
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
        };
        if params(c) != params(d) {
            return Some(format!(
                "{}'s parameters: the contract's {:?}, the device's {:?}",
                c.name,
                params(c),
                params(d)
            ));
        }
        for (pc, pd) in c.parameters.iter().zip(&d.parameters) {
            let bounds = |k: &Kind| match k {
                Kind::Integer { min, max, .. } => Some((*min, *max)),
                _ => None,
            };
            let same_kind = std::mem::discriminant(&pc.kind) == std::mem::discriminant(&pd.kind)
                && bounds(&pc.kind) == bounds(&pd.kind);
            if !same_kind || pc.sensitive != pd.sensitive {
                return Some(format!(
                    "{}.{}: the contract's {:?}{}, the device's {:?}{}",
                    c.name,
                    pc.name,
                    pc.kind,
                    if pc.sensitive { ", sensitive" } else { "" },
                    pd.kind,
                    if pd.sensitive { ", sensitive" } else { "" },
                ));
            }
        }
    }
    None
}

/// Each run's token counter start (§2.6): drawn at random once per run,
/// or the start a test hook set ([`Configuration::simulate_counter_start`]).
#[derive(Debug, Default)]
struct Counter {
    run: Option<Run>,
    start: u16,
    forced: Option<u16>,
}

/// The model's [`Mint`] over the service's [`Minter`]: the run and every
/// seq are the instance's, shared with every state key and event the
/// service writes (core §4.3).
struct RunMint<'a> {
    minter: &'a Minter,
    counter: &'a mut Counter,
}

impl Mint for RunMint<'_> {
    fn run(&self) -> Run {
        self.minter.run()
    }

    /// The minter's as-of mark's seq: 0 when none was minted under the run,
    /// which the model's mark reads as `None` would be read.
    fn last_seq(&self) -> Option<Seq> {
        Some(self.minter.mark().seq)
    }

    fn stamp(&mut self) -> Stamp {
        self.minter.mint_seq()
    }

    fn next_run(&mut self) {
        self.minter.new_run();
    }

    fn counter_start(&mut self) -> u16 {
        let run = self.minter.run();
        if self.counter.run != Some(run) {
            self.counter.run = Some(run);
            self.counter.start = self.counter.forced.take().unwrap_or_else(rand::random);
        }
        self.counter.start
    }
}

/// A configuration readied to restart in a new instance (v1 §2.12, which
/// §2.12 keeps), from [`Configuration::restarting`]: nothing pending, its
/// device at its startup values, its owner held so that no call lands
/// before [`Restarting::start`]. Dropped unstarted (the re-mint failed after
/// it), the old instance runs on, and says so.
pub(crate) struct Restarting<'a> {
    inner: &'a Inner,
    core: Option<tokio::sync::MutexGuard<'a, Core>>,
}

impl Restarting<'_> {
    /// The configuration restarts in the new instance (§2.12), whose run
    /// the service took already: its instance number that run, from its
    /// startup values and persisted layer, revision 0, nothing pending, no
    /// last change, no idempotency key held, the last hour's change events
    /// still counted (§2.17), and the next token's counter drawn afresh
    /// (§2.6); a document given by [`Configuration::set_desired`] acted on
    /// again, as a restart's is. Its record is put now, under that run,
    /// before the new instance's tokens (§2.3).
    pub(crate) async fn start(mut self) {
        let Some(mut c) = self.core.take() else {
            return;
        };
        let (owner, out) = c.with(&self.inner.minter, |o, m| o.restart_in(m));
        let instance = owner.instance();
        c.owner = owner;
        c.acted = None;
        c.named.clear();
        c.take(out, None);
        self.inner.instance.store(instance, Ordering::Relaxed);
        drop(c);
        tracing::info!(
            service = %self.inner.service, instance,
            "config.v2: restarted in a new instance, from its startup values (config.v1 §2.12)"
        );
        self.inner.flush().await;
        self.inner.wake.notify_one();
    }
}

impl Drop for Restarting<'_> {
    fn drop(&mut self) {
        if self.core.is_some() {
            tracing::warn!(
                service = %self.inner.service, instance = self.inner.instance(),
                "config.v2: the re-mint failed after the device returned to its startup values; \
                 the record may name values it no longer runs: close this service and start \
                 another"
            );
        }
    }
}

/// One write an [`Output`] owes the bus, in its order.
enum Write {
    /// The record, as stamped.
    Put(v2::Put),
    /// A call's answer: a value form or an error form, as written.
    Reply(Call, v2::Reply),
    /// A change event, under its stamp.
    Event(v2::Published),
}

/// The owner and what it owes the bus. Every step is taken under its lock;
/// a device act is performed with it released.
struct Core {
    owner: v2::Owner,
    counter: Counter,
    /// Written in order by whoever flushes next.
    queue: VecDeque<Write>,
    /// The desired document held, the latest given (§2.18).
    desired: Option<DesiredConfig>,
    /// The revision of the last document acted on.
    acted: Option<u64>,
    /// The parameters the documents fed through `desired.v1` named since
    /// the last revert, refused ones aside: what a delete reverts
    /// (`desired.v1` §2.16).
    named: BTreeSet<(String, String)>,
}

impl Core {
    /// Queues `out`'s writes in its order: the puts, the reply to `call`
    /// when there is one, then the events. Returns the reply, for a caller
    /// that reads it (a desired document's `set`).
    fn take(&mut self, out: Output, call: Option<&Call>) -> Option<v2::Reply> {
        let Output {
            puts,
            reply,
            events,
        } = out;
        self.queue.extend(puts.into_iter().map(Write::Put));
        if let (Some(r), Some(c)) = (&reply, call) {
            self.queue.push_back(Write::Reply(c.clone(), r.clone()));
        }
        self.queue.extend(events.into_iter().map(Write::Event));
        reply
    }

    /// Runs `f` on the owner with the service's mint.
    fn with<T>(
        &mut self,
        minter: &Minter,
        f: impl FnOnce(&mut v2::Owner, &mut dyn Mint) -> T,
    ) -> T {
        let Core { owner, counter, .. } = self;
        let mut mint = RunMint { minter, counter };
        f(owner, &mut mint)
    }
}

/// A service's `config.v2` (§2), from [`ServiceBuilder::config_v2`], and
/// again from [`crate::Service::config_v2`] once it runs.
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
    /// The owner's address, `<system>/<service>`.
    service: String,
    /// The instance number (§2.3), moved at a re-mint
    /// ([`Restarting::start`]).
    instance: AtomicU32,
    /// Fed through `desired.v1` ([`crate::ServiceBuilder::desired_config`]):
    /// its re-mint is refused (`desired.v1` §2.16).
    fed: AtomicBool,
    interface: Interface,
    /// config.v1's schema of the groups, the allowlist's `writable` set:
    /// what the device's words are redacted by.
    schema: Schema,
    /// Each settable group, served at `set/<group>`.
    settable: Vec<String>,
    device: Arc<dyn Device>,
    minter: Arc<Minter>,
    core: tokio::sync::Mutex<Core>,
    /// Held while a flush writes, so the writes go in the model's order.
    flushing: tokio::sync::Mutex<()>,
    record: StateWriter,
    change: EventWriter,
    /// Every step, every desired document and every clock shift wakes the
    /// timer.
    wake: tokio::sync::Notify,
    offset_ms: AtomicI64,
    /// The operations' servers, until the service closes or drops.
    servers: std::sync::Mutex<Vec<OperationServer>>,
    /// The service's close: a document fed through `desired.v1` stops
    /// waiting at it.
    closed: tokio::sync::watch::Receiver<bool>,
}

impl Configuration {
    /// Implements `config.v2` on `b` over its config contract (§2.1): the
    /// record put, the operations served, the timer spawned.
    #[allow(clippy::too_many_lines)]
    pub(crate) async fn declare(
        b: &mut ServiceBuilder,
        startup: Startup,
        device: Arc<dyn Device>,
    ) -> Result<Self> {
        let rt = tokio::runtime::Handle::try_current()
            .map_err(|e| Error::Contract(format!("config.v2 needs a tokio runtime: {e}")))?;
        let addr = b.address()?.clone();
        // §2.1: one config contract, implemented already.
        let configs: Vec<_> = b
            .impls()
            .iter()
            .filter(|s| {
                s.imp
                    .contract()
                    .uses
                    .iter()
                    .any(|u| u.to_string() == v2::PROFILE)
            })
            .map(|s| s.imp.clone())
            .collect();
        let imp = match configs.as_slice() {
            [one] => one.clone(),
            [] => {
                return Err(Error::Contract(
                    "config.v2: implement the service's config contract first, one that lists \
                     config.v2 in uses (config.v2 §2.1)"
                        .to_owned(),
                ));
            }
            many => {
                return Err(Error::Contract(format!(
                    "config.v2: an instance implements at most one config contract, and this \
                     one implements {} (config.v2 §2.1)",
                    many.iter()
                        .map(|i| i.iface().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        };
        let iface = imp.iface().clone();
        // §2.2: a contract with a finding is refused, never read.
        let findings = v2::lint_contract(imp.contract());
        if !findings.is_empty() {
            return Err(Error::Contract(format!(
                "config.v2: {iface}'s contract does not lint: {} (config.v2 §2.2)",
                findings
                    .iter()
                    .map(|f| format!("{} {}", f.at, f.reason.token()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        let interface = Interface::of(imp.contract())
            .map_err(|e| Error::Contract(format!("config.v2: {iface}: {e}")))?;
        let settable: BTreeSet<String> = interface
            .groups
            .iter()
            .filter(|g| g.settable())
            .map(|g| g.name.clone())
            .collect();
        let writable = startup.writable.clone().unwrap_or_else(|| settable.clone());
        if let Some(g) = writable.iter().find(|g| !settable.contains(*g)) {
            return Err(Error::Contract(format!(
                "config.v2: the allowlist names {g:?}, which is no settable group of {iface} \
                 (config.v2 §2.16)"
            )));
        }
        let schema = interface.schema(&writable);
        if let Some(d) = &startup.device_schema
            && let Some(why) = schema_differs(&schema, d)
        {
            return Err(Error::Contract(format!(
                "config.v2: the device's schema is not {iface}'s contract's: {why} (config.v2 \
                 Appendix B)"
            )));
        }
        let settable: Vec<String> = interface
            .groups
            .iter()
            .filter(|g| g.settable() && interface.derived.set(&g.name).is_some())
            .map(|g| g.name.clone())
            .collect();
        let none = Bindings::new();
        let record = b.declare_state_writer(&iface, RECORD, &none).await?;
        let change = b.event_writer(&iface, CHANGE, &none)?;
        let minter = Arc::clone(b.minter());
        let start = v2::Start {
            service: addr.to_string(),
            startup: startup.layers(),
            overlay: startup.persisted.clone().unwrap_or_default(),
            floor_s: startup.floor_s,
            apply_after: startup.apply_after,
            persist: startup.persisted.is_some(),
            writable,
            events: startup
                .events
                .iter()
                .map(|t| crate::config_v1::time_of(*t))
                .collect(),
            epoch: b.deployment_epoch(),
        };
        let mut counter = Counter::default();
        let (owner, out) = {
            let mut mint = RunMint {
                minter: &minter,
                counter: &mut counter,
            };
            v2::Owner::start(interface.clone(), start, &mut mint)
                .map_err(|e| Error::Contract(format!("config.v2: {e}")))?
        };
        let instance = owner.instance();
        let (_, _, _, closed) = b.state_parts();
        let (_, _, _, closing) = b.state_parts();
        let inner = Arc::new(Inner {
            iface: iface.clone(),
            service: addr.to_string(),
            instance: AtomicU32::new(instance),
            fed: AtomicBool::new(false),
            interface,
            schema,
            settable,
            device,
            minter,
            core: tokio::sync::Mutex::new(Core {
                owner,
                counter,
                queue: VecDeque::new(),
                desired: None,
                acted: None,
                named: BTreeSet::new(),
            }),
            flushing: tokio::sync::Mutex::new(()),
            record,
            change,
            wake: tokio::sync::Notify::new(),
            offset_ms: AtomicI64::new(0),
            servers: std::sync::Mutex::new(Vec::new()),
            closed: closing,
        });
        // §2.3: the record before §8.2 step 4, put here, before the
        // descriptor lists the config interface and before any token.
        inner.core.lock().await.take(out, None);
        inner.flush().await;

        let mut servers: Vec<OperationServer> = Vec::new();
        for group in &inner.settable {
            let i = Arc::clone(&inner);
            let g = group.clone();
            servers.push(
                b.serve(&iface, &set(group), Some(&none), move |call| {
                    let i = Arc::clone(&i);
                    let g = g.clone();
                    async move { i.on_set(&g, call).await }
                })
                .await?,
            );
        }
        for (resource, op) in [
            (CONFIRM, model::ControlOp::Confirm),
            (CANCEL, model::ControlOp::Cancel),
            (EXTEND, model::ControlOp::Extend),
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
        rt.spawn(Arc::clone(&inner).run(closed));
        let cfg = Self { inner };
        b.config_v2 = Some(cfg.clone());
        Ok(cfg)
    }

    /// Undeclares the operations' servers, and waits for it: the service
    /// closes, after its tokens. A restarted instance then never shares a
    /// key with this one.
    pub(crate) async fn shutdown(&self) -> Result<()> {
        let servers = std::mem::take(&mut *self.inner.servers.lock().expect("not poisoned"));
        for s in servers {
            s.undeclare().await?;
        }
        Ok(())
    }

    /// The config interface.
    #[must_use]
    pub fn iface(&self) -> &IfaceId {
        &self.inner.iface
    }

    /// Its declaration and derived types, read from the contract (§2.2).
    #[must_use]
    pub fn interface(&self) -> &Interface {
        &self.inner.interface
    }

    /// config.v1's schema of the groups the owner runs under, each group's
    /// `writable` the allowlist's (§2.16).
    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.inner.schema
    }

    /// The instance number (§2.3): the run the instance first wrote under.
    #[must_use]
    pub fn instance(&self) -> u32 {
        self.inner.instance()
    }

    /// Fed through `desired.v1` from now on
    /// ([`crate::ServiceBuilder::desired_config`]).
    pub(crate) fn fed_by_desired(&self) {
        self.inner.fed.store(true, Ordering::Relaxed);
    }

    /// Readies a re-mint (v1 §2.12, "A re-mint is a restart", which §2.12
    /// keeps; [`crate::Service::new_epoch`]), as
    /// [`crate::config_v1::Configuration`]'s does: a pending change
    /// cancelled as `cancel` does, its record and its event written; the
    /// device returned to its startup values where a runtime change left a
    /// parameter elsewhere ([`Device::restore_startup`]); then the owner
    /// held, so that no call lands until the new instance starts it
    /// ([`Restarting::start`]).
    ///
    /// # Errors
    ///
    /// [`Error::Contract`], the instance running on (a cancel made stays
    /// made): fed through `desired.v1`; an act in flight (§2.7); a
    /// parameter set at runtime that starts unknown; the device's refusal
    /// to return; a change pending again at every try.
    pub(crate) async fn restarting(&self) -> Result<Restarting<'_>> {
        let i = &*self.inner;
        if i.fed.load(Ordering::Relaxed) {
            return Err(Error::Contract(
                "config.v2: a re-mint is a restart (config.v1 §2.12, which config.v2 keeps), \
                 which an owner fed through desired.v1 refuses while it runs (desired.v1 \
                 §2.16): close this service and start another"
                    .to_owned(),
            ));
        }
        // A call can start another change while the cancel acts, so it is
        // tried a few times.
        for _ in 0..RESTART_TRIES {
            let mut c = i.core.lock().await;
            if c.owner.in_flight() {
                return Err(Error::Contract(
                    "config.v2: an act is in flight (config.v1 §2.7), so the re-mint is \
                     refused: try again once it ends"
                        .to_owned(),
                ));
            }
            let Some(token) = c.owner.state().pending.map(|p| p.token) else {
                let plan = c.owner.restart_plan().map_err(|(g, p)| {
                    Error::Contract(format!(
                        "config.v1 §2.12: {g}.{p} runs a runtime value and starts unknown, so \
                         nothing returns the device to its startup values: close this service \
                         and start another"
                    ))
                })?;
                if !plan.is_empty() {
                    i.restore(&plan).await?;
                }
                return Ok(Restarting {
                    inner: i,
                    core: Some(c),
                });
            };
            tracing::info!(
                service = %i.service, instance = i.instance(),
                token,
                "config.v2: the pending change is cancelled, for a re-mint (config.v1 §2.12)"
            );
            let control = v2::Control {
                token,
                confirm_s: None,
            };
            let payload = i.interface.encode_control(&control).map_err(|e| {
                Error::Contract(format!(
                    "config.v2: the cancel of {token} does not encode: {}",
                    e.token()
                ))
            })?;
            let now = i.now();
            let step = c.with(&i.minter, |o, m| {
                o.cancel(now, &payload, &model::Call::default(), m)
            });
            drop(c);
            let reply = i.drive(step, None).await;
            i.flush().await;
            i.wake.notify_one();
            if let Some(v2::Reply {
                answer: Answer::Error { code, .. },
                ..
            }) = reply
            {
                return Err(Error::Contract(format!(
                    "config.v2: the pending change was not cancelled for a re-mint ({})",
                    code.as_str()
                )));
            }
        }
        Err(Error::Contract(
            "config.v2: a change was pending again at every try, so the re-mint is refused"
                .to_owned(),
        ))
    }

    /// The record as the owner holds it now (§2.3). Between an act's start
    /// and its end it may run ahead of the record last put, which is what a
    /// reader sees.
    pub async fn state(&self) -> State {
        self.inner.core.lock().await.owner.state()
    }

    /// The desired report the record carries (§2.18), if any.
    pub async fn desired(&self) -> Option<DesiredReport> {
        self.inner
            .core
            .lock()
            .await
            .owner
            .inner()
            .desired()
            .cloned()
    }

    /// Gives the owner a desired document to converge (§2.18), as
    /// [`crate::config_v1::Configuration::set_desired`] does: refused
    /// whole, or converged through one `set` without a window per group
    /// whose values differ, waiting while a change is pending; its outcome
    /// is the record's `desired` fields. A revision already acted on is not
    /// acted on again in this instance.
    pub async fn set_desired(&self, doc: DesiredConfig) {
        self.inner.core.lock().await.desired = Some(doc);
        self.inner.wake.notify_one();
    }

    /// The desired document this instance holds, the latest given: the
    /// binding is the deployment's, so an application hands it to the next
    /// instance.
    pub async fn desired_document(&self) -> Option<DesiredConfig> {
        self.inner.core.lock().await.desired.clone()
    }

    /// Converges `doc` now, as `desired.v1` feeds it (§2.18, `desired.v1`
    /// §2.16), as [`crate::config_v1::Configuration`]'s does: whatever its
    /// revision; waiting while a change is pending, up to
    /// [`DESIRED_WAIT_PAST_DEADLINE`] past its deadline (or
    /// [`DESIRED_WAIT_IN_FLIGHT`] on an act in flight with none pending), and
    /// no longer than the service runs; through one `set` without a window
    /// per group whose values differ. Its report is the record's, and
    /// returned.
    pub(crate) async fn converge_desired(&self, doc: DesiredConfig) -> DesiredEnd {
        let end = self.inner.converge_doc(&doc, true).await;
        let refused = matches!(
            &end,
            DesiredEnd::Report(r) if r.outcome == model::DesiredOutcome::Refused
        );
        if !refused {
            let mut c = self.inner.core.lock().await;
            for (g, ps) in &doc.values {
                for p in ps.keys() {
                    c.named.insert((g.clone(), p.clone()));
                }
            }
        }
        self.inner.wake.notify_one();
        end
    }

    /// A delete fed through `desired.v1` (`desired.v1` §2.16): each
    /// parameter the documents named since the last revert back at its
    /// startup value, through one `set` without a window per group that
    /// differs, and no `desired` fields in the record from then on; as
    /// [`crate::config_v1::Configuration`]'s, what was not restored staying
    /// named.
    pub(crate) async fn revert_desired(&self) -> std::result::Result<(), DesiredEnd> {
        let doc = {
            let c = self.inner.core.lock().await;
            c.owner
                .inner()
                .baseline_document(c.named.iter().map(|(g, p)| (g.as_str(), p.as_str())))
        };
        let end = self.inner.converge_doc(&doc, false).await;
        let failed = {
            let mut c = self.inner.core.lock().await;
            let failed = match &end {
                DesiredEnd::Report(DesiredReport {
                    refusal: Some(r), ..
                }) => {
                    let from = r.group.clone();
                    c.named.retain(|(g, _)| *g >= from);
                    true
                }
                DesiredEnd::Report(_) => {
                    c.named.clear();
                    false
                }
                DesiredEnd::GaveUp(_) => true,
            };
            let out = c.with(&self.inner.minter, |o, m| o.report_desired(None, m));
            c.take(out, None);
            failed
        };
        self.inner.flush().await;
        self.inner.wake.notify_one();
        if failed { Err(end) } else { Ok(()) }
    }

    /// When the change events of the last hour were published, on the
    /// owner's clock: the record the next instance starts from
    /// ([`Startup::events`]).
    pub async fn published(&self) -> Vec<SystemTime> {
        let now = self.inner.now();
        let c = self.inner.core.lock().await;
        c.owner
            .published(now)
            .into_iter()
            .map(crate::config_v1::system_of)
            .collect()
    }

    /// The owner's clock now (§2.8).
    #[must_use]
    pub fn now(&self) -> SystemTime {
        crate::config_v1::system_of(self.inner.now())
    }

    /// Shifts the owner's clock by `ms` from this host's (negative:
    /// behind), and runs what is owed at the new instant. For tests and
    /// simulation; a deployment never sets it.
    pub fn simulate_offset(&self, ms: i64) {
        self.inner.offset_ms.store(ms, Ordering::Relaxed);
        self.inner.wake.notify_one();
    }

    /// Starts the next run's token counter at `start` rather than a random
    /// draw (scenarios-v2.md §7 step 5). When the run written under now has
    /// minted a token, its counter is drawn already, so the owner takes a
    /// new run (core §4.3's MAY), whose counter starts there. For tests; a
    /// deployment never sets it.
    pub async fn simulate_counter_start(&self, start: u16) {
        let mut c = self.inner.core.lock().await;
        c.counter.forced = Some(start);
        if c.counter.run == Some(self.inner.minter.run()) {
            self.inner.minter.new_run();
        }
    }

    /// The record's writer: zenoh stays reachable.
    #[must_use]
    pub fn record_writer(&self) -> &StateWriter {
        &self.inner.record
    }
}

impl Inner {
    /// The instance number it runs under now.
    fn instance(&self) -> u32 {
        self.instance.load(Ordering::Relaxed)
    }

    /// [`Device::restore_startup`], on a task of its own, as an act is
    /// ([`Inner::perform`]): its refusal, or its panic, refuses the re-mint.
    async fn restore(&self, plan: &[Restore]) -> Result<()> {
        tracing::info!(
            service = %self.service, instance = self.instance(),
            parameters = plan.len(),
            "config.v2: the device returns to its startup values, for a re-mint (config.v1 §2.12)"
        );
        let device = Arc::clone(&self.device);
        let owned = plan.to_vec();
        let joined = tokio::spawn(async move { device.restore_startup(&owned).await }).await;
        let words = match joined {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(words)) => words,
            Err(e) => format!("the device's act did not complete: {e}"),
        };
        let act = Act::Undo {
            restore: plan.to_vec(),
        };
        let words = model::device_words(&self.schema, &act, &words).unwrap_or(model::REDACTED);
        tracing::warn!(
            service = %self.service, instance = self.instance(),
            words = %words,
            "config.v2: the device did not return to its startup values, so the re-mint is \
             refused (config.v1 §2.12)"
        );
        Err(Error::Contract(format!(
            "config.v1 §2.12: the device did not return to its startup values ({words}), so \
             the re-mint is refused"
        )))
    }

    fn now(&self) -> Time {
        let t = crate::config_v1::time_of(SystemTime::now());
        Time::from_millis(
            t.millis()
                .saturating_add(self.offset_ms.load(Ordering::Relaxed)),
        )
    }

    /// Performs one act on the device, on a task of its own, logging it and
    /// the device's words where §2.15 lets them be.
    async fn perform(&self, act: &Act) -> DeviceResult {
        match act {
            Act::Apply { group, values } => tracing::info!(
                service = %self.service, instance = self.instance(),
                group = %group,
                parameters = values.len(),
                "config.v2: apply"
            ),
            Act::Undo { restore } => tracing::info!(
                service = %self.service, instance = self.instance(),
                parameters = restore.len(),
                "config.v2: undo"
            ),
        }
        let result = crate::config_v1::perform(&self.device, act).await;
        if let DeviceResult::Refused(words) | DeviceResult::Failed { words, .. } = &result {
            let words = model::device_words(&self.schema, act, words).unwrap_or(model::REDACTED);
            tracing::warn!(
                service = %self.service, instance = self.instance(),
                words = %words,
                "config.v2: the device {} (config.v1 §2.9, §2.11)",
                if matches!(act, Act::Apply { .. }) {
                    "refused an apply"
                } else {
                    "failed an undo, once: never retried"
                }
            );
        }
        result
    }

    /// Takes a step to its end: each act performed, its result reported,
    /// until the owner answers. The answer's writes are queued, its reply to
    /// `call`; the reply is returned too.
    async fn drive(&self, step: Step, call: Option<&Call>) -> Option<v2::Reply> {
        let mut step = step;
        loop {
            match step {
                Step::Done(out) => return self.core.lock().await.take(out, call),
                Step::Act(act) => {
                    let result = self.perform(&act).await;
                    let mut c = self.core.lock().await;
                    let now = self.now();
                    step = c.with(&self.minter, |o, m| o.finish(now, result, m));
                }
            }
        }
    }

    /// Writes what the steps queued, in order: each put as stamped, each
    /// reply as written, each event under its stamp (§2.3, §2.9, §2.17).
    async fn flush(&self) {
        let _order = self.flushing.lock().await;
        loop {
            let Some(w) = self.core.lock().await.queue.pop_front() else {
                break;
            };
            match w {
                Write::Put(p) => {
                    if let Err(e) = self.record.put_sealed(p.stamp, p.payload).await {
                        tracing::warn!(service = %self.service, instance = self.instance(), "config.v2: the record was not put: {e}");
                    }
                }
                Write::Reply(call, r) => {
                    if let Answer::Error { code, cause, .. } = &r.answer {
                        tracing::info!(
                            service = %self.service, instance = self.instance(),
                            code = code.as_str(),
                            cause = cause.map(|c| c.as_str()).unwrap_or("-"),
                            "config.v2: refused"
                        );
                    }
                    let sent = match r.answer {
                        Answer::Value { .. } => call.reply_form(r.payload).await,
                        Answer::Error { .. } => call.refuse_form(r.payload).await,
                    };
                    if let Err(e) = sent {
                        tracing::warn!(service = %self.service, instance = self.instance(), "config.v2: a reply was not sent: {e}");
                    }
                }
                Write::Event(p) => {
                    let ev = &p.event;
                    tracing::info!(
                        service = %ev.service, instance = ev.instance,
                        token = ev.token,
                        outcome = ev.outcome.as_str(),
                        revision = ev.revision,
                        "config.v2: a change ended"
                    );
                    let sent = match serde_json::to_vec(ev) {
                        Ok(bytes) => self.change.put_sealed(&bytes, p.stamp).await.map(|_| ()),
                        Err(e) => Err(Error::Payload(e.to_string())),
                    };
                    if let Err(e) = sent {
                        tracing::error!(service = %ev.service, instance = ev.instance, token = ev.token, "config.v2: a change event was not published: {e}");
                    }
                }
            }
        }
    }

    /// A call's request as the model reads it, CH and the record: the
    /// query's payload, in the request form, since the runtime refused any
    /// other before the handler ran (core §5.1, "The request").
    fn payload(call: &Call) -> Vec<u8> {
        call.payload()
            .map(|p| p.to_bytes().into_owned())
            .unwrap_or_default()
    }

    /// `@op/set/<group>` (§2.5).
    async fn on_set(&self, group: &str, call: Call) -> std::result::Result<(), OpError> {
        let payload = Self::payload(&call);
        let who = crate::config_v1::claimed(&call);
        let logged = match self.interface.decode_set(group, &payload) {
            Some(Ok((_, fields))) => self.interface.log_set(group, &fields).to_string(),
            _ => "-".to_owned(),
        };
        tracing::info!(
            service = %self.service, instance = self.instance(),
            group = %group,
            request = %logged,
            actor = who.actor.as_deref().unwrap_or("-"),
            request_id = who.request_id.as_deref().unwrap_or("-"),
            "config.v2: set"
        );
        let step = {
            let mut c = self.core.lock().await;
            let now = self.now();
            c.with(&self.minter, |o, m| o.set(now, group, &payload, &who, m))
        };
        self.drive(step, Some(&call)).await;
        self.flush().await;
        self.wake.notify_one();
        Ok(())
    }

    /// `confirm`, `cancel` and `extend` (§2.10).
    async fn on_control(
        &self,
        op: model::ControlOp,
        call: Call,
    ) -> std::result::Result<(), OpError> {
        let payload = Self::payload(&call);
        let who = crate::config_v1::claimed(&call);
        tracing::info!(
            service = %self.service, instance = self.instance(),
            token = self.interface.decode_control(&payload).map(|c| c.token).unwrap_or_default(),
            actor = who.actor.as_deref().unwrap_or("-"),
            request_id = who.request_id.as_deref().unwrap_or("-"),
            "config.v2: {}",
            op.as_str()
        );
        let step = {
            let mut c = self.core.lock().await;
            let now = self.now();
            c.with(&self.minter, |o, m| match op {
                model::ControlOp::Confirm => o.confirm(now, &payload, &who, m),
                model::ControlOp::Cancel => o.cancel(now, &payload, &who, m),
                model::ControlOp::Extend => o.extend(now, &payload, &who, m),
                model::ControlOp::Persist => unreachable!("persist is on_persist's"),
            })
        };
        self.drive(step, Some(&call)).await;
        self.flush().await;
        self.wake.notify_one();
        Ok(())
    }

    /// `persist` (§2.13): served only with a persisted layer. The plan, the
    /// write, then the commit, the owner locked throughout.
    async fn on_persist(&self, call: Call) -> std::result::Result<(), OpError> {
        // §2.13 row 1: a layer that cannot be written now, as no layer is,
        // first of the handler's refusals.
        if let Err(why) = self.device.persist_ready() {
            return Err(OpError::unavailable(
                Cause::Config,
                format!("the persisted layer cannot be written now: {why} (config.v2 §2.13)"),
            ));
        }
        let payload = Self::payload(&call);
        tracing::info!(
            service = %self.service, instance = self.instance(),
            token = self.interface.decode_control(&payload).map(|c| c.token).unwrap_or_default(),
            "config.v2: persist"
        );
        {
            let mut c = self.core.lock().await;
            let now = self.now();
            match c.with(&self.minter, |o, m| o.persist(now, &payload, m)) {
                Err(out) => {
                    c.take(out, Some(&call));
                }
                Ok(plan) => {
                    if let Err(why) = self.device.persist(&plan.entries).await {
                        tracing::warn!(
                            service = %self.service, instance = self.instance(),
                            "config.v2: the persisted layer was not written: {why}"
                        );
                        return Err(OpError::internal(format!(
                            "the persisted layer was not written: {why} (config.v2 §2.13)"
                        )));
                    }
                    let out = c.with(&self.minter, |o, m| o.persisted(plan, m));
                    c.take(out, Some(&call));
                }
            }
        }
        self.flush().await;
        self.wake.notify_one();
        Ok(())
    }

    /// Every act owed now (§2.9, §2.11), each to its end.
    async fn run_due(&self) {
        loop {
            let step = {
                let mut c = self.core.lock().await;
                let now = self.now();
                c.with(&self.minter, |o, m| o.due(now, m))
            };
            let Some(step) = step else { break };
            self.drive(step, None).await;
            self.flush().await;
        }
    }

    /// A desired document's `set` of `group`, its reply returned: `None`
    /// when the request does not encode, which no group of the contract's
    /// lets happen.
    async fn desired_set(
        &self,
        revision: u64,
        group: &str,
        request: &v2::Set,
    ) -> Option<v2::Reply> {
        let payload = match self.interface.encode_set(group, request) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    service = %self.service, instance = self.instance(),
                    revision,
                    "config.v2: a desired document's set of {group} does not encode: {}",
                    e.token()
                );
                return None;
            }
        };
        let step = {
            let mut c = self.core.lock().await;
            let now = self.now();
            c.with(&self.minter, |o, m| {
                o.set(now, group, &payload, &model::Call::default(), m)
            })
        };
        self.drive(step, None).await
    }

    /// The report of a desired document whose `set` of `group` was refused.
    fn failed(revision: u64, group: &str, code: zenkey_model::envelope::Code) -> DesiredReport {
        let refusal = model::Refusal {
            code: code_of(code),
            app: None,
            field: None,
            message: String::new(),
        };
        model::desired_failed(revision, group, &refusal)
    }

    /// Converges `doc` to its end (§2.18) for `desired.v1` (§2.16),
    /// waiting while a change is pending or an act is in flight, as
    /// `config.v1`'s owner does, and returns its report. With `report`, the
    /// report is the record's; without, the record's `desired` is left as it
    /// is (a revert's own document).
    async fn converge_doc(&self, doc: &DesiredConfig, report: bool) -> DesiredEnd {
        let mut closed = self.closed.clone();
        let mut in_flight_since: Option<tokio::time::Instant> = None;
        loop {
            let (group, request) = {
                let mut c = self.core.lock().await;
                match c.owner.converge(doc) {
                    Converge::Wait => {
                        let why = match c.owner.inner().deadline() {
                            Some(deadline) => {
                                in_flight_since = None;
                                let past = self.now().millis() - deadline.millis();
                                let grace = i64::try_from(DESIRED_WAIT_PAST_DEADLINE.as_millis())
                                    .unwrap_or(i64::MAX);
                                (past > grace).then(|| {
                                    format!(
                                        "the pending change did not end within {} s of its \
                                         deadline, so the document was not converged",
                                        DESIRED_WAIT_PAST_DEADLINE.as_secs()
                                    )
                                })
                            }
                            None => {
                                let since =
                                    *in_flight_since.get_or_insert_with(tokio::time::Instant::now);
                                (since.elapsed() > DESIRED_WAIT_IN_FLIGHT).then(|| {
                                    format!(
                                        "an act in flight did not end within {} s, so the \
                                         document was not converged",
                                        DESIRED_WAIT_IN_FLIGHT.as_secs()
                                    )
                                })
                            }
                        };
                        drop(c);
                        let why = why.or_else(|| {
                            (*closed.borrow())
                                .then(|| "the service closed while the document waited".to_owned())
                        });
                        if let Some(why) = why {
                            tracing::warn!(
                                service = %self.service, instance = self.instance(),
                                revision = doc.revision,
                                "config.v2: a desired document gave up waiting (desired.v1): {why}"
                            );
                            return DesiredEnd::GaveUp(why);
                        }
                        tokio::select! {
                            () = tokio::time::sleep(Duration::from_millis(25)) => {}
                            changed = closed.changed() => {
                                if changed.is_err() {
                                    return DesiredEnd::GaveUp(
                                        "the service dropped while the document waited".to_owned(),
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    Converge::Refused(r) | Converge::Converged(r) => {
                        tracing::info!(
                            service = %self.service, instance = self.instance(),
                            revision = doc.revision,
                            report = %serde_json::to_string(&r).unwrap_or_default(),
                            "config.v2: a desired document acted on (desired.v1)"
                        );
                        if report {
                            let out =
                                c.with(&self.minter, |o, m| o.report_desired(Some(r.clone()), m));
                            c.take(out, None);
                        }
                        drop(c);
                        self.flush().await;
                        return DesiredEnd::Report(r);
                    }
                    Converge::Set { group, request } => (group, request),
                }
            };
            let reply = self.desired_set(doc.revision, &group, &request).await;
            let code = match &reply {
                Some(v2::Reply {
                    answer: Answer::Error { code, .. },
                    ..
                }) => Some(*code),
                Some(_) => None,
                None => Some(zenkey_model::envelope::Code::InvalidRequest),
            };
            if let Some(code) = code {
                let r = Self::failed(doc.revision, &group, code);
                tracing::info!(
                    service = %self.service, instance = self.instance(),
                    revision = doc.revision,
                    report = %serde_json::to_string(&r).unwrap_or_default(),
                    "config.v2: a desired document failed (desired.v1)"
                );
                if report {
                    let mut c = self.core.lock().await;
                    let out = c.with(&self.minter, |o, m| o.report_desired(Some(r.clone()), m));
                    c.take(out, None);
                }
                self.flush().await;
                return DesiredEnd::Report(r);
            }
            self.flush().await;
        }
    }

    /// Converges the desired document held, as far as it can now (§2.18).
    async fn converge(&self) {
        loop {
            let (revision, group, request) = {
                let mut c = self.core.lock().await;
                let Some(doc) = c.desired.clone() else { return };
                if c.acted == Some(doc.revision) {
                    return;
                }
                match c.owner.converge(&doc) {
                    Converge::Wait => return,
                    Converge::Refused(report) | Converge::Converged(report) => {
                        tracing::info!(
                            service = %self.service, instance = self.instance(),
                            revision = doc.revision,
                            report = %serde_json::to_string(&report).unwrap_or_default(),
                            "config.v2: a desired document acted on"
                        );
                        let out = c.with(&self.minter, |o, m| o.report_desired(Some(report), m));
                        c.acted = Some(doc.revision);
                        c.take(out, None);
                        drop(c);
                        self.flush().await;
                        return;
                    }
                    Converge::Set { group, request } => (doc.revision, group, request),
                }
            };
            let reply = self.desired_set(revision, &group, &request).await;
            let code = match &reply {
                Some(v2::Reply {
                    answer: Answer::Error { code, .. },
                    ..
                }) => Some(*code),
                Some(_) => None,
                None => Some(zenkey_model::envelope::Code::InvalidRequest),
            };
            if let Some(code) = code {
                let report = Self::failed(revision, &group, code);
                tracing::info!(
                    service = %self.service, instance = self.instance(),
                    revision,
                    report = %serde_json::to_string(&report).unwrap_or_default(),
                    "config.v2: a desired document failed"
                );
                let mut c = self.core.lock().await;
                let out = c.with(&self.minter, |o, m| o.report_desired(Some(report), m));
                c.acted = Some(revision);
                c.take(out, None);
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

/// config.v1's code of an error form's (§2.5): what a desired document's
/// failure is reported by.
fn code_of(code: zenkey_model::envelope::Code) -> model::Code {
    use zenkey_model::envelope::Code as E;
    match code {
        E::InvalidRequest => model::Code::InvalidRequest,
        E::NotFound => model::Code::NotFound,
        E::Unavailable => model::Code::Unavailable,
        E::Forbidden => model::Code::Forbidden,
        E::Busy | E::FanoutForbidden | E::Internal => model::Code::Busy,
        E::App => model::Code::App,
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Schema, schema_differs};
    use zenkey_model::config::{Class, GroupDecl, ParamDecl};

    fn schema(max: i64, class: Class) -> Schema {
        Schema {
            groups: vec![GroupDecl {
                name: "radio".into(),
                class,
                description: "what the contract says".into(),
                writable: true,
                parameters: vec![ParamDecl {
                    name: "frequency_hz".into(),
                    kind: Kind::Integer {
                        min: Some(150_000_000),
                        max: Some(max),
                        unit: Some("Hz".into()),
                    },
                    description: String::new(),
                    sensitive: false,
                }],
            }],
        }
    }

    /// Appendix B: a bound or a class differs; a description, a unit or
    /// the allowlist does not.
    #[test]
    fn the_devices_schema_is_compared_by_its_wire() {
        let contract = schema(960_000_000, Class::Reach);
        let mut same = contract.clone();
        same.groups[0].description = "the device's words".into();
        same.groups[0].writable = false;
        if let Kind::Integer { unit, .. } = &mut same.groups[0].parameters[0].kind {
            *unit = None;
        }
        assert_eq!(schema_differs(&contract, &same), None);
        assert!(schema_differs(&contract, &schema(928_000_000, Class::Reach)).is_some());
        assert!(schema_differs(&contract, &schema(960_000_000, Class::Hot)).is_some());
    }

    /// An application shares the handle across tasks.
    #[test]
    fn the_handle_is_shared_across_tasks() {
        fn shared<T: Clone + Send + Sync + 'static>() {}
        shared::<super::Configuration>();
    }
}
