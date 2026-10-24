//! Self-publication (#389; zk2 since #614, FL3): the daemon is a zk2
//! service, so `zengui` and `zenctl` see the notifier without SSH and
//! something else can watch the watcher — another zenwatch included.
//!
//! **The service.** One instance of `zenwatch` (by default at
//! `@hostid.v1/zenwatch`, the system minted from the host's machine id,
//! failing closed on a host with none: `hostid.v1` §2.6), brought up on the
//! session in the deployment's namespace through the runtime's
//! [`ServiceBuilder`] in spec §8.2's order: the resources declared, the
//! exposure checked, the descriptor and both bundles served, then the
//! instance token and the interface tokens. Two interfaces:
//!
//! - **`health.v1`**, through [`zenkey::health`]: the status says whether
//!   zenwatch can do its job, declared `OK` and raised by the runtime to
//!   its worst check (§2.2). The checks are the parts of the job, at most
//!   seven of the profile's 64 ([`checks`]): `sinks` (the last delivery of
//!   each), `presence`, `tokens` and `alarms` (what the observers' basis
//!   lets the rules conclude: a monitor's `Dropped(n)`, an alarm reader's
//!   lost delivery or stall, is `DEGRADED` until a complete re-read; #800
//!   AD for `alarms`), `doctor` (a scheduled run that could not happen),
//!   `state-file` (a ledger that did not save) and `firing` (rules the
//!   bound refused). The runtime re-puts the status every 28.5 s.
//! - **`zenwatch.v1`** (`contracts/zenwatch.v1.toml`), through the
//!   generated [`zenwatch_v1::Server`]:
//!   - `state/firing/{rule_id}` — one [`FiringRule`] per rule with
//!     something announced, the most severe (then oldest) notice under it
//!     named and the rest counted; put on change, deleted when the rule
//!     goes quiet. The runtime keeps the contract's bound of 256 (core
//!     §2.7): a rule past it is refused with `Error::Budget`, nothing
//!     published, and counted in [`ZenwatchCounters::firing_refused`] —
//!     never silent;
//!   - `state/doctor` — [`ZenwatchDoctor`], after every scheduled doctor
//!     run (#390, [`crate::doctor`]): the outcome, the counts, and the
//!     report exactly as `zenctl doctor --format json` prints it. Optional,
//!     gated `config:doctor`: unavailable (cause `config`) in the
//!     descriptor when no `doctor` block is configured;
//!   - `state/counters` — [`ZenwatchCounters`], what the daemon's bounds
//!     cost (RFC 13 §3 O6), put on change at most once per tick. These are
//!     not health.v1 checks: a count is not a level, and health.v1 carries
//!     no application field (its Appendix A).
//!
//!   `firing` and `counters` declare a horizon, so the runtime's state
//!   writers re-put them every ttl/2 and this module has no refresh code;
//!   `doctor` declares none and is never re-put.
//!
//! **Shutdown** ([`SelfService::shutdown`]) deletes every `firing/*`
//! document, then closes the service: the tokens go, then the queryables.
//! The doctor's last report is not deleted (an archive keeps it as
//! last-known), and the health status is left as `health.v1` §2.3 asks. The
//! engine stops its observers and drains its sinks first, so the daemon
//! never pages about its own clean stop.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zenkey::health::{Health, Level};
use zenkey::model::descriptor::Cause;
use zenkey::model::grammar::{Addr, Name};
use zenkey::typed::StateWriter;
use zenkey::{Address, HostIdMinter, Service, ServiceBuilder, ServiceConfig};
use zenkey_fleet::report::{DoctorDelta, DoctorReport};

use crate::config::PublishSettings;
use crate::discipline::{Counters as DisciplineCounters, Discipline, severity_rank};
use crate::render::state_word;
use crate::zk2::zenwatch_v1;

/// The service's name: the default address is `@hostid.v1/zenwatch`.
pub const SERVICE: &str = "zenwatch";

/// The contract's `cardinality` on `firing/{rule_id}`: the most `firing/*`
/// documents this service holds at once (core §2.7), kept by the runtime.
pub const FIRING_CARDINALITY: u64 = 256;

/// `ok` or `failed`: whether a scheduled doctor run happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorOutcome {
    Ok,
    Failed,
}

/// Where the scheduled doctor stands, for its `health.v1` check (#390).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorStatus {
    /// Scheduled; the first run has not happened yet.
    Pending,
    /// The last run succeeded.
    Ok,
    /// The last run could not happen; the report before it is retained.
    Failed,
}

/// `state/firing/{rule_id}`: what one rule currently has announced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FiringRule {
    pub rule: String,
    /// The notice identity named here: the most severe, then the oldest.
    pub id: String,
    /// `firing` or `unobservable`.
    pub state: String,
    pub severity: String,
    /// RFC 3339: when the named notice's condition began.
    pub since: String,
    pub labels: BTreeMap<String, String>,
    /// How many notice identities are announced under this rule, this
    /// one included.
    pub count: usize,
}

/// `state/doctor`: the scheduled doctor's last run (#390,
/// [`crate::doctor`]). Published after every run, successful or not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZenwatchDoctor {
    /// RFC 3339: when this run happened (or was attempted).
    pub ran_at: String,
    /// RFC 3339: when the next one is due. In the past: the schedule lapsed.
    pub next_at: String,
    /// The interval, seconds.
    pub every_s: f64,
    pub outcome: DoctorOutcome,
    /// Why a `failed` run could not happen.
    pub error: Option<String>,
    /// Findings in `report` — after a failed run, the retained report's.
    pub findings: usize,
    /// Findings new since the previous run (`0` on the baseline).
    pub new: usize,
    /// Findings gone since the previous run.
    pub fixed: usize,
    /// The `DoctorReport`, exactly as `zenctl doctor --format json` prints
    /// it: each check's verdict in the judgement shape, what was not asked
    /// absent (RFC 13 §1). After a failed run, the last report that
    /// succeeded, retained; `null` when none has yet. Carried as the value
    /// it serialized to; its schema is the fleet's (`report::doctor`).
    #[serde(default)]
    #[schemars(with = "Option<DoctorReport>")]
    pub report: serde_json::Value,
    /// RFC 3339: the run that produced `report` — `ran_at` on a successful
    /// run, older after a failed one, absent when there is no report.
    pub report_at: Option<String>,
    /// The `DoctorDelta` against the previous run; absent on the baseline
    /// and after a failed run.
    #[schemars(with = "Option<DoctorDelta>")]
    pub delta: Option<serde_json::Value>,
}

/// `state/counters`: what the daemon did, and what its bounds cost, since
/// it started (RFC 13 §3 O6: every bound reports its cost).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZenwatchCounters {
    /// RFC 3339: when this run started; every count is since then.
    pub started_at: String,
    pub rules: usize,
    pub sinks: usize,
    /// Notice identities currently announced firing.
    pub firing: usize,
    /// Notice identities currently announced unobservable.
    pub unobservable: usize,
    /// Deliveries that went out, one per sink a notification reached.
    pub delivered: u64,
    /// Deliveries a sink failed.
    pub deliveries_failed: u64,
    /// Liveliness events the presence and token observers dropped.
    pub dropped: u64,
    /// Address records the presence projection forgot at its bound.
    pub forgotten: u64,
    /// Entries the discipline's ledger holds now.
    pub state_entries: usize,
    /// `firing/{rule_id}` documents the runtime refused at the bound
    /// (core §2.7): once each time a rule's document became refused.
    pub firing_refused: u64,
    /// Scheduled doctor runs attempted (#390).
    pub doctor_runs: u64,
    /// What the discipline did (#389).
    pub discipline: DisciplineCounters,
}

/// The `firing/{rule_id}` documents the discipline's ledger implies.
pub fn firing_docs(discipline: &Discipline) -> BTreeMap<String, FiringRule> {
    let mut out: BTreeMap<String, FiringRule> = BTreeMap::new();
    for e in discipline.announced() {
        let since = zenkey_fleet::rfc3339_from_unix(e.since.max(0.0) as u64);
        let doc = out.entry(e.rule_id.clone()).or_insert_with(|| FiringRule {
            rule: e.rule.clone(),
            id: e.id.clone(),
            state: state_word(e.state).into(),
            severity: e.severity.clone(),
            since: since.clone(),
            labels: e.labels.clone(),
            count: 0,
        });
        doc.count += 1;
        let more_severe = severity_rank(&e.severity) > severity_rank(&doc.severity);
        let older_at_same =
            severity_rank(&e.severity) == severity_rank(&doc.severity) && since < doc.since;
        if more_severe || older_at_same {
            doc.id = e.id.clone();
            doc.state = state_word(e.state).into();
            doc.severity = e.severity.clone();
            doc.since = since;
            doc.labels = e.labels.clone();
        }
    }
    out
}

// ── The address ───────────────────────────────────────────────────────────

/// The service's configuration from the config's `publish` block (#614,
/// FL3): `@hostid.v1/zenwatch` unless `address` names another, the
/// ephemeral rung allowed only by `ephemeral: true` (`hostid.v1` §2.6).
/// Nothing is minted here.
pub fn service_config(s: &PublishSettings) -> Result<ServiceConfig, String> {
    let address = match &s.address {
        None => Address::HostId {
            service: Name::new("service", SERVICE).expect("a literal service name"),
            ephemeral: false,
        },
        Some(a) => a
            .parse::<Address>()
            .map_err(|e| format!("{a:?} is not an address: {e}"))?,
    };
    let address = match (address, s.ephemeral) {
        (Address::HostId { service, .. }, ephemeral) => Address::HostId { service, ephemeral },
        (Address::Literal(a), true) => {
            return Err(format!(
                "`ephemeral: true` asks hostid.v1 for an ephemeral system, and the address {a} \
                 names its system: write `@hostid.v1/{}`, or drop `ephemeral` (hostid.v1 §2.3)",
                a.service
            ));
        }
        (literal, false) => literal,
    };
    Ok(ServiceConfig::at(address))
}

/// Why the daemon's own address could not be had, with every way out:
/// `hostid.v1`'s refusal (§2.6) names each path it tried.
pub fn unminted(e: &zenkey::HostIdError) -> String {
    format!(
        "publish: {e}. zenwatch publishes itself at `@hostid.v1/{SERVICE}` unless told \
         otherwise: name its system with `publish: {{ address: \"<system>/{SERVICE}\" }}`, \
         allow an ephemeral one with `publish: {{ ephemeral: true }}`, or run no service with \
         `publish: false`"
    )
}

// ── health.v1 ─────────────────────────────────────────────────────────────

/// The checks' names (`health.v1` §2.3: `{check}`, slugged in the key).
pub mod check {
    pub const SINKS: &str = "sinks";
    pub const PRESENCE: &str = "presence";
    pub const TOKENS: &str = "tokens";
    pub const ALARMS: &str = "alarms";
    pub const DOCTOR: &str = "doctor";
    pub const STATE_FILE: &str = "state-file";
    pub const FIRING: &str = "firing";
}

/// One part of the daemon's job, as a `health.v1` check (§2.2, §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    /// `None`: `UNSPECIFIED`, a check that cannot run yet (§2.6), which
    /// bounds nothing.
    pub level: Option<Level>,
    pub detail: String,
}

/// A sink's last delivery, as the engine's deliveries record it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkOutcome {
    /// RFC 3339.
    pub at: String,
    /// Why it failed; `None` when it went out.
    pub error: Option<String>,
}

/// What an observer's basis lets its rules conclude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observer {
    /// Not read yet.
    Unseeded,
    /// The last read ended at its timeout, or its session was cut off while
    /// it ran (core §8.1, 0.38): what it did not show may be held.
    Incomplete {
        /// The read's session was cut off while it ran.
        cut_off_during: bool,
    },
    /// A complete read stands, and nothing was lost since.
    Complete,
    /// Events lost (the monitor's `Dropped(n)`) since the last complete read.
    Lost { dropped: u64 },
    /// The last read was not made: the daemon's session, or its monitor's,
    /// reaches no other zenoh node (core §8.1, 0.37).
    CutOff,
    /// The monitor stopped: its rules see nothing more.
    Stopped,
}

impl From<zenkey_fleet::PresenceBasis> for Observer {
    fn from(b: zenkey_fleet::PresenceBasis) -> Self {
        use zenkey_fleet::PresenceBasis as B;
        match b {
            B::Unseeded => Observer::Unseeded,
            B::Incomplete { cut_off_during } => Observer::Incomplete { cut_off_during },
            B::Complete => Observer::Complete,
            B::Lost { dropped } => Observer::Lost { dropped },
            B::CutOff => Observer::CutOff,
        }
    }
}

impl From<crate::liveliness::TokenBasis> for Observer {
    fn from(b: crate::liveliness::TokenBasis) -> Self {
        use crate::liveliness::TokenBasis as B;
        match b {
            B::Unseeded => Observer::Unseeded,
            B::Incomplete { cut_off_during } => Observer::Incomplete { cut_off_during },
            B::Complete => Observer::Complete,
            B::Lost { dropped } => Observer::Lost { dropped },
            B::CutOff => Observer::CutOff,
        }
    }
}

/// The scheduled doctor, for its check.
#[derive(Debug, Clone, Copy)]
pub struct DoctorCheck<'a> {
    pub status: DoctorStatus,
    /// How the interval was spelled.
    pub every: &'a str,
    /// The last run's document, once a run happened.
    pub last: Option<&'a ZenwatchDoctor>,
}

/// Everything the checks are computed from, at one tick.
#[derive(Debug, Clone, Copy)]
pub struct HealthInputs<'a> {
    /// Every configured sink, with its last delivery (`None`: none yet).
    pub sinks: &'a BTreeMap<String, Option<SinkOutcome>>,
    /// The presence projection; `None` when no rule reads it.
    pub presence: Option<Observer>,
    /// The foreign-token ledger; `None` when no rule reads it.
    pub tokens: Option<Observer>,
    /// The `alerts` rules' alarm readers, the worst of them (#800, AD);
    /// `None` when no rule reads alarms.
    pub alarms: Option<Observer>,
    /// The scheduled doctor; `None` when no `doctor` block is configured.
    pub doctor: Option<DoctorCheck<'a>>,
    /// The state file: `None` when none is configured, `Some(None)` when
    /// the last save went through, `Some(Some(why))` when it failed.
    pub state_file: Option<Option<&'a str>>,
    /// Rules with something announced whose `firing/*` document the bound
    /// refuses right now.
    pub firing_refused: usize,
}

/// A detail's error text, bounded: a check's detail is for a person.
fn short(s: &str) -> String {
    const MAX: usize = 200;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut end = MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn observer_check(name: &'static str, o: Observer, what: &str) -> Check {
    let (level, detail) = match o {
        Observer::Complete => (
            Some(Level::Ok),
            format!("a complete read of {what} stands, and nothing was lost since"),
        ),
        Observer::Unseeded => (None, format!("{what} not read yet")),
        Observer::Incomplete { cut_off_during } => (
            Some(Level::Degraded),
            format!(
                "the last read of {what} {}: what it did not show may be held, so nothing is \
                 judged gone (core §8.1)",
                zenkey_fleet::model::catalog::incomplete_reason(
                    false,
                    cut_off_during,
                    "ended at its timeout"
                )
                .unwrap_or_default()
            ),
        ),
        Observer::Lost { dropped } => (
            Some(Level::Degraded),
            format!(
                "{dropped} event(s) of {what} lost since the last complete read: every \
                 identity its rules watch is unobservable until a complete re-read (RFC 13 §3 O6)"
            ),
        ),
        Observer::CutOff => (
            Some(Level::Degraded),
            format!(
                "this daemon's session reaches no router and no peer, so no read of {what} is \
                 made: every identity its rules watch is unobservable until a complete read \
                 (core §8.1)"
            ),
        ),
        Observer::Stopped => (
            Some(Level::Degraded),
            format!("the monitor of {what} stopped: its rules see nothing more"),
        ),
    };
    Check {
        name,
        level,
        detail,
    }
}

/// The checks one tick's state calls for (`health.v1` §2.3), in a stable
/// order; a part of the job the configuration does not have is no check.
///
/// | Check | `OK` | `DEGRADED` | `FAILED` | `UNSPECIFIED` |
/// |---|---|---|---|---|
/// | `sinks` | no sink's last delivery failed | some sinks' last delivery failed | every sink's last delivery failed: no notification can go out | — |
/// | `presence`, `tokens` | a complete read, nothing lost | a read that timed out, events dropped, or the monitor stopped | — | not read yet |
/// | `doctor` | scheduled and not run yet, or the last run happened | the last run could not happen | — | — |
/// | `state-file` | the last save went through | the last save failed | — | — |
/// | `firing` | every rule with something announced is published | the bound refuses some | — | — |
pub fn checks(i: &HealthInputs<'_>) -> Vec<Check> {
    let mut out = Vec::new();

    let failing: Vec<String> = i
        .sinks
        .iter()
        .filter_map(|(name, o)| {
            let o = o.as_ref()?;
            o.error
                .as_ref()
                .map(|e| format!("{name} (at {}: {})", o.at, short(e)))
        })
        .collect();
    let (level, detail) = if failing.is_empty() {
        let tried = i.sinks.values().any(Option::is_some);
        (
            Level::Ok,
            if tried {
                "no sink's last delivery failed".to_string()
            } else {
                "no delivery yet".to_string()
            },
        )
    } else if failing.len() == i.sinks.len() {
        (
            Level::Failed,
            format!(
                "every sink's last delivery failed, so no notification can go out: {}",
                failing.join("; ")
            ),
        )
    } else {
        (
            Level::Degraded,
            format!(
                "the last delivery failed on {} of {} sink(s): {}",
                failing.len(),
                i.sinks.len(),
                failing.join("; ")
            ),
        )
    };
    out.push(Check {
        name: check::SINKS,
        level: Some(level),
        detail,
    });

    if let Some(o) = i.presence {
        out.push(observer_check(
            check::PRESENCE,
            o,
            "the deployment's presence",
        ));
    }
    if let Some(o) = i.tokens {
        out.push(observer_check(check::TOKENS, o, "the watched tokens"));
    }
    if let Some(o) = i.alarms {
        out.push(observer_check(
            check::ALARMS,
            o,
            "the alarms the `alerts` rules read",
        ));
    }

    if let Some(d) = i.doctor {
        let (level, detail) = match (d.status, d.last) {
            (DoctorStatus::Ok, Some(doc)) => (
                Level::Ok,
                format!(
                    "the last run, at {}, found {} finding(s); the next is due at {}",
                    doc.ran_at, doc.findings, doc.next_at
                ),
            ),
            (DoctorStatus::Failed, Some(doc)) => (
                Level::Degraded,
                format!(
                    "the last run, at {}, could not happen: {}; {}",
                    doc.ran_at,
                    short(doc.error.as_deref().unwrap_or("no reason given")),
                    match &doc.report_at {
                        Some(at) => format!("the report of {at} is retained"),
                        None => "no report has been had yet".to_string(),
                    }
                ),
            ),
            _ => (
                Level::Ok,
                format!("scheduled every {}; no run yet", d.every),
            ),
        };
        out.push(Check {
            name: check::DOCTOR,
            level: Some(level),
            detail,
        });
    }

    if let Some(saved) = i.state_file {
        out.push(match saved {
            None => Check {
                name: check::STATE_FILE,
                level: Some(Level::Ok),
                detail: "the ledger's last save went through".into(),
            },
            Some(why) => Check {
                name: check::STATE_FILE,
                level: Some(Level::Degraded),
                detail: format!(
                    "the ledger's last save failed: {}; a restart would re-announce what is \
                     firing",
                    short(why)
                ),
            },
        });
    }

    out.push(if i.firing_refused == 0 {
        Check {
            name: check::FIRING,
            level: Some(Level::Ok),
            detail: "every rule with something announced has its firing document".into(),
        }
    } else {
        Check {
            name: check::FIRING,
            level: Some(Level::Degraded),
            detail: format!(
                "{} rule(s) with something announced have no firing document: the bound of \
                 {FIRING_CARDINALITY} refuses them (core §2.7)",
                i.firing_refused
            ),
        }
    });
    out
}

// ── The service ───────────────────────────────────────────────────────────

/// What the service starts with.
#[derive(Debug, Clone)]
pub struct Plan {
    pub rules: usize,
    pub sinks: usize,
    /// A `doctor` block is configured: `state/doctor` is exposed, else
    /// listed unavailable (cause `config`).
    pub doctor: bool,
}

/// Why the service did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// `hostid.v1` gave it no system (§2.6): refused input, exit 2.
    #[error("{}", unminted(.0))]
    Unminted(std::sync::Arc<zenkey::HostIdError>),
    /// The runtime refused the bring-up.
    #[error("zenwatch's own service did not start: {0}")]
    Runtime(#[from] zenkey::Error),
}

/// One `firing/{rule_id}` member: its writer, what it last published, and
/// whether the bound refuses it now.
struct Firing {
    writer: StateWriter<zenkey::codec::Json<FiringRule>>,
    published: Option<FiringRule>,
    refused: bool,
}

/// The daemon, up as a zk2 service.
pub struct SelfService {
    svc: Service,
    server: zenwatch_v1::Server,
    health: Health,
    firing: BTreeMap<String, Firing>,
    firing_refused: u64,
    counters: Option<ZenwatchCounters>,
    /// The checks as last put, by name.
    checks: BTreeMap<&'static str, (Option<Level>, String)>,
}

impl SelfService {
    /// Brings the service up on `session` (spec §8.2): `config`'s address
    /// resolved through `hostid` (`hostid.v1` §2.7), `health.v1` and
    /// `zenwatch.v1` declared, the status declared `OK` and `checks` put
    /// before the tokens (`health.v1` §2.3), and so are `counters` (§8.2,
    /// "State values": a GET made when the tokens appear finds them), then
    /// started.
    pub async fn start(
        session: &zenoh::Session,
        config: ServiceConfig,
        hostid: &HostIdMinter,
        plan: &Plan,
        checks: &[Check],
        counters: &ZenwatchCounters,
    ) -> Result<SelfService, StartError> {
        let mut b = ServiceBuilder::with_hostid(session, config, hostid);
        if let Err(zenkey::Error::HostId(e)) = b.address() {
            return Err(StartError::Unminted(e));
        }
        let health = b.health().await?;
        let iface = zenwatch_v1::iface();
        b.implement(zenwatch_v1::implementation())?;
        if !plan.doctor {
            b.unavailable(
                &iface,
                zenwatch_v1::resource::DOCTOR,
                Cause::Config,
                Some("no `doctor` block: the scheduled doctor is not configured"),
            )?;
        }
        let server = zenwatch_v1::Server::declare(&mut b).await?;
        health
            .set_status(
                Level::Ok,
                format!("watching {} rule(s), {} sink(s)", plan.rules, plan.sinks),
            )
            .await?;
        let mut applied = BTreeMap::new();
        for c in checks {
            put_check(&health, c).await?;
            applied.insert(c.name, (c.level, c.detail.clone()));
        }
        server.counters.put(counters).await?;
        let svc = b.start().await?;
        tracing::info!(address = %svc.address(), instance = %svc.instance(),
            "zenwatch is up as a zk2 service");
        Ok(SelfService {
            svc,
            server,
            health,
            firing: BTreeMap::new(),
            firing_refused: 0,
            counters: Some(counters.clone()),
            checks: applied,
        })
    }

    /// The address this service publishes under, resolved.
    pub fn address(&self) -> &Addr {
        self.svc.address()
    }

    /// Its `health.v1` handle.
    pub fn health(&self) -> &Health {
        &self.health
    }

    /// `firing/*` documents refused at the bound so far: once each time a
    /// rule's document became refused.
    pub fn firing_refused(&self) -> u64 {
        self.firing_refused
    }

    /// Rules with something announced whose document the bound refuses now.
    pub fn firing_refused_now(&self) -> usize {
        self.firing.values().filter(|f| f.refused).count()
    }

    /// Brings the published `firing/*` set to `docs`: vanished documents
    /// deleted first (each frees a member), then changed ones put. A new
    /// member past the bound is the runtime's `Error::Budget` (core §2.7):
    /// nothing is published, the refusal is counted once and said, and the
    /// put is tried again on the next tick, when a member may have freed.
    pub async fn publish_firing(
        &mut self,
        docs: &BTreeMap<String, FiringRule>,
    ) -> zenkey::Result<()> {
        let gone: Vec<String> = self
            .firing
            .keys()
            .filter(|id| !docs.contains_key(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(f) = self.firing.remove(&id)
                && f.published.is_some()
            {
                f.writer.delete().await?;
            }
        }
        for (rule_id, doc) in docs {
            if self
                .firing
                .get(rule_id)
                .is_some_and(|f| f.published.as_ref() == Some(doc))
            {
                continue;
            }
            if !self.firing.contains_key(rule_id) {
                let writer = self.server.firing(&mut self.svc, rule_id).await?;
                self.firing.insert(
                    rule_id.clone(),
                    Firing {
                        writer,
                        published: None,
                        refused: false,
                    },
                );
            }
            let f = self.firing.get_mut(rule_id).expect("inserted above");
            match f.writer.put(doc).await {
                Ok(_) => {
                    f.published = Some(doc.clone());
                    f.refused = false;
                }
                Err(zenkey::Error::Budget(why)) => {
                    if !f.refused {
                        f.refused = true;
                        self.firing_refused += 1;
                        tracing::warn!(rule_id, "firing/{rule_id} not published: {why}");
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Puts the counters when they changed since the last put.
    pub async fn publish_counters(&mut self, c: &ZenwatchCounters) -> zenkey::Result<()> {
        if self.counters.as_ref() == Some(c) {
            return Ok(());
        }
        self.server.counters.put(c).await?;
        self.counters = Some(c.clone());
        Ok(())
    }

    /// The scheduled doctor's last run (#390), after every run, whatever its
    /// outcome. Nothing when `state/doctor` is unavailable.
    pub async fn publish_doctor(&self, doc: &ZenwatchDoctor) -> zenkey::Result<()> {
        match &self.server.doctor {
            Some(w) => w.put(doc).await.map(|_| ()),
            None => Ok(()),
        }
    }

    /// Brings the `health.v1` checks to `checks`: a changed one put (the
    /// runtime orders the status's put around it, §2.2), a vanished one
    /// retired. Each call is one put per change, never a re-put.
    pub async fn set_checks(&mut self, checks: &[Check]) -> zenkey::Result<()> {
        for c in checks {
            let now = (c.level, c.detail.clone());
            if self.checks.get(c.name) == Some(&now) {
                continue;
            }
            put_check(&self.health, c).await?;
            self.checks.insert(c.name, now);
        }
        let gone: Vec<&'static str> = self
            .checks
            .keys()
            .filter(|n| !checks.iter().any(|c| c.name == **n))
            .copied()
            .collect();
        for name in gone {
            self.health.retire_check(name).await?;
            self.checks.remove(name);
        }
        Ok(())
    }

    /// Every `firing/*` document deleted, then the service closed: its
    /// state writers' re-puts stopped, the tokens undeclared, then the
    /// queryables. The doctor's report and the health status stay as they
    /// were last put.
    pub async fn shutdown(mut self) -> zenkey::Result<()> {
        for (_, f) in std::mem::take(&mut self.firing) {
            if f.published.is_some() {
                f.writer.delete().await?;
            }
        }
        self.svc.close().await
    }
}

async fn put_check(h: &Health, c: &Check) -> zenkey::Result<()> {
    match c.level {
        Some(l) => h.set_check(c.name, l, c.detail.clone()).await.map(|_| ()),
        None => h
            .set_check_unknown(c.name, c.detail.clone())
            .await
            .map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_file() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/zenwatch.v1.toml")
    }

    /// Rust-first (#614, FL3): every type the contract binds to a struct,
    /// and the fleet's report and delta inside the doctor's, say what the
    /// committed schema says, read through the zk2 subset.
    #[test]
    fn every_rust_first_type_matches_its_committed_schema() {
        let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/schemas/zenwatch.json");
        let check = |r: Result<(), String>, name: &str| {
            r.unwrap_or_else(|e| panic!("{name} and {}:\n{e}", file.display()))
        };
        check(
            zenkey_build::check_schema::<FiringRule>(&file, "FiringRule"),
            "FiringRule",
        );
        check(
            zenkey_build::check_schema::<ZenwatchDoctor>(&file, "ZenwatchDoctor"),
            "ZenwatchDoctor",
        );
        check(
            zenkey_build::check_schema::<ZenwatchCounters>(&file, "ZenwatchCounters"),
            "ZenwatchCounters",
        );
        check(
            zenkey_build::check_schema::<DoctorReport>(&file, "DoctorReport"),
            "DoctorReport",
        );
        check(
            zenkey_build::check_schema::<DoctorDelta>(&file, "DoctorDelta"),
            "DoctorDelta",
        );
    }

    /// What the daemon puts decodes as what the contract declares: each
    /// document validated against its type in the embedded bundle, as an
    /// explorer validates it (spec §7.3).
    #[test]
    fn what_is_published_validates_against_the_bundle() {
        let bundle = zenkey_model::bundle::Bundle::verify(zenwatch_v1::BUNDLE).unwrap();
        let valid = |template: &str, value: serde_json::Value| {
            let ty = zenkey_model::decode::type_of(&bundle, "state", template, "type")
                .unwrap_or_else(|| panic!("state/{template} declares a type"));
            zenkey_model::validate::validate(&bundle, ty, &value)
                .unwrap_or_else(|e| panic!("state/{template}: {e:?}\n{value}"));
        };
        let firing = FiringRule {
            rule: "tc-gone".into(),
            id: "tc-gone:h-1/tc".into(),
            state: "firing".into(),
            severity: "error".into(),
            since: "2026-10-10T00:00:00Z".into(),
            labels: BTreeMap::from([("team".into(), "infra".into())]),
            count: 2,
        };
        valid("firing/{rule_id}", serde_json::to_value(&firing).unwrap());
        let report = zenkey_fleet::report::DoctorReport {
            scope: zenkey_fleet::report::DoctorScope {
                namespace: "site".into(),
                presence: zenkey_fleet::report::Asked::NotAsked,
                routers: zenkey_fleet::report::Asked::Asked(1),
                health: zenkey_fleet::report::Asked::NotAsked,
                config: zenkey_fleet::report::Asked::NotAsked,
            },
            checks: vec![zenkey_fleet::report::CheckReport::not_asked(
                zenkey_fleet::report::CheckId::SplitBrain,
            )],
            unobservable: None,
        };
        let doctor = ZenwatchDoctor {
            ran_at: "2026-10-10T06:00:00Z".into(),
            next_at: "2026-10-10T12:00:00Z".into(),
            every_s: 21600.0,
            outcome: DoctorOutcome::Ok,
            error: None,
            findings: 0,
            new: 0,
            fixed: 0,
            report: serde_json::to_value(&report).unwrap(),
            report_at: Some("2026-10-10T06:00:00Z".into()),
            delta: Some(serde_json::to_value(DoctorDelta::default()).unwrap()),
        };
        valid("doctor", serde_json::to_value(&doctor).unwrap());
        let failed = ZenwatchDoctor {
            outcome: DoctorOutcome::Failed,
            error: Some("no zk2 token visible".into()),
            report: serde_json::Value::Null,
            report_at: None,
            delta: None,
            ..doctor
        };
        valid("doctor", serde_json::to_value(&failed).unwrap());
    }

    /// The contract the build embedded is the file, loaded with no finding
    /// at all and published in its append-only history; its bound is the
    /// one this module names, and its three resources the ones published.
    #[test]
    fn the_embedded_contract_is_the_file() {
        let l = zenkey_model::contract::load_path(&contract_file());
        assert!(
            l.report.0.is_empty(),
            "no finding, warnings included:\n{}",
            l.report
        );
        let c = l.contract.expect("the contract loads");
        assert_eq!(
            zenkey_model::bundle::Bundle::build(&c).to_bytes(),
            zenwatch_v1::BUNDLE,
            "the embedded bundle is the file's"
        );
        let history = contract_file().with_file_name(".history");
        assert!(
            zenkey_model::history::check(&history).is_empty(),
            "{:?}",
            zenkey_model::history::check(&history)
        );
        let published = history.join(format!(
            "zenwatch.v1/{}.bundle.json",
            zenwatch_v1::FINGERPRINT.trim_start_matches("sha256:")
        ));
        assert_eq!(
            std::fs::read(&published).unwrap_or_else(|e| panic!(
                "{}: {e}; publish it with `zk2 contract bundle zenwatch/contracts/zenwatch.v1.toml \
                 --history zenwatch/contracts/.history`",
                published.display()
            )),
            zenwatch_v1::BUNDLE,
            "the revision built is the one published"
        );
        let imp = zenwatch_v1::implementation();
        let firing = imp.resource(zenwatch_v1::resource::FIRING).unwrap();
        assert_eq!(firing.cardinality, Some(FIRING_CARDINALITY));
        assert!(
            imp.resource(zenwatch_v1::resource::DOCTOR)
                .unwrap()
                .optional
        );
        assert!(imp.resource(zenwatch_v1::resource::COUNTERS).is_ok());
        assert_eq!(imp.contract().resources.len(), 3);
    }

    /// The address: `@hostid.v1/zenwatch` by default, ephemeral only when
    /// asked, a literal one as named, and `ephemeral` beside a literal
    /// address refused (hostid.v1 §2.3).
    #[test]
    fn the_address_defaults_to_the_minted_one() {
        let at = |s: PublishSettings| service_config(&s).map(|c| c.address);
        assert_eq!(
            at(PublishSettings::default()).unwrap(),
            Address::HostId {
                service: Name::new("service", "zenwatch").unwrap(),
                ephemeral: false
            }
        );
        assert_eq!(
            at(PublishSettings {
                ephemeral: true,
                ..PublishSettings::default()
            })
            .unwrap(),
            Address::HostId {
                service: Name::new("service", "zenwatch").unwrap(),
                ephemeral: true
            }
        );
        assert_eq!(
            at(PublishSettings {
                address: Some("ops-1/watch".into()),
                ephemeral: false,
            })
            .unwrap()
            .to_string(),
            "ops-1/watch"
        );
        assert_eq!(
            at(PublishSettings {
                address: Some("@hostid.v1/watch".into()),
                ephemeral: true,
            })
            .unwrap(),
            Address::HostId {
                service: Name::new("service", "watch").unwrap(),
                ephemeral: true
            }
        );
        let e = at(PublishSettings {
            address: Some("ops-1/watch".into()),
            ephemeral: true,
        })
        .unwrap_err();
        assert!(e.contains("hostid.v1 §2.3"), "{e}");
        let e = at(PublishSettings {
            address: Some("not an address".into()),
            ephemeral: false,
        })
        .unwrap_err();
        assert!(e.contains("not an address"), "{e}");
    }

    fn sinks(outcomes: &[(&str, Option<Option<&str>>)]) -> BTreeMap<String, Option<SinkOutcome>> {
        outcomes
            .iter()
            .map(|(n, o)| {
                (
                    n.to_string(),
                    o.map(|e| SinkOutcome {
                        at: "2026-10-10T00:00:00Z".into(),
                        error: e.map(str::to_string),
                    }),
                )
            })
            .collect()
    }

    fn level_of(cs: &[Check], name: &str) -> Option<Option<Level>> {
        cs.iter().find(|c| c.name == name).map(|c| c.level)
    }

    /// The mapping of the module doc's table, pole by pole: a part the
    /// configuration does not have is no check, and nothing here is ever
    /// more than seven of health.v1's 64.
    #[test]
    fn each_part_of_the_job_maps_onto_a_check() {
        let none = sinks(&[("ops", None), ("mail", None)]);
        let base = HealthInputs {
            sinks: &none,
            presence: None,
            tokens: None,
            alarms: None,
            doctor: None,
            state_file: None,
            firing_refused: 0,
        };
        let cs = checks(&base);
        assert_eq!(
            cs.iter().map(|c| c.name).collect::<Vec<_>>(),
            [check::SINKS, check::FIRING]
        );
        assert_eq!(cs[0].level, Some(Level::Ok));
        assert_eq!(cs[0].detail, "no delivery yet");

        let one = sinks(&[("ops", Some(Some("HTTP 500: down"))), ("mail", Some(None))]);
        let cs = checks(&HealthInputs {
            sinks: &one,
            ..base
        });
        assert_eq!(level_of(&cs, check::SINKS), Some(Some(Level::Degraded)));
        assert!(cs[0].detail.contains("ops (at"), "{}", cs[0].detail);
        let all = sinks(&[("ops", Some(Some("x"))), ("mail", Some(Some("y")))]);
        let cs = checks(&HealthInputs {
            sinks: &all,
            ..base
        });
        assert_eq!(level_of(&cs, check::SINKS), Some(Some(Level::Failed)));
        let fine = sinks(&[("ops", Some(None)), ("mail", None)]);
        assert_eq!(
            level_of(
                &checks(&HealthInputs {
                    sinks: &fine,
                    ..base
                }),
                check::SINKS
            ),
            Some(Some(Level::Ok))
        );

        for (o, want) in [
            (Observer::Complete, Some(Level::Ok)),
            (Observer::Unseeded, None),
            (
                Observer::Incomplete {
                    cut_off_during: false,
                },
                Some(Level::Degraded),
            ),
            (
                Observer::Incomplete {
                    cut_off_during: true,
                },
                Some(Level::Degraded),
            ),
            (Observer::Lost { dropped: 3 }, Some(Level::Degraded)),
            (Observer::CutOff, Some(Level::Degraded)),
            (Observer::Stopped, Some(Level::Degraded)),
        ] {
            let cs = checks(&HealthInputs {
                presence: Some(o),
                tokens: Some(o),
                alarms: Some(o),
                ..base
            });
            assert_eq!(level_of(&cs, check::PRESENCE), Some(want), "{o:?}");
            assert_eq!(level_of(&cs, check::TOKENS), Some(want), "{o:?}");
            assert_eq!(level_of(&cs, check::ALARMS), Some(want), "{o:?}");
        }

        let doc = ZenwatchDoctor {
            ran_at: "2026-10-10T06:00:00Z".into(),
            next_at: "2026-10-10T12:00:00Z".into(),
            every_s: 21600.0,
            outcome: DoctorOutcome::Failed,
            error: Some("no zk2 token visible".into()),
            findings: 0,
            new: 0,
            fixed: 0,
            report: serde_json::Value::Null,
            report_at: None,
            delta: None,
        };
        for (status, last, want) in [
            (DoctorStatus::Pending, None, Level::Ok),
            (DoctorStatus::Ok, Some(&doc), Level::Ok),
            (DoctorStatus::Failed, Some(&doc), Level::Degraded),
        ] {
            let cs = checks(&HealthInputs {
                doctor: Some(DoctorCheck {
                    status,
                    every: "6h",
                    last,
                }),
                ..base
            });
            assert_eq!(level_of(&cs, check::DOCTOR), Some(Some(want)), "{status:?}");
        }

        let cs = checks(&HealthInputs {
            state_file: Some(Some("disk full")),
            firing_refused: 2,
            ..base
        });
        assert_eq!(
            level_of(&cs, check::STATE_FILE),
            Some(Some(Level::Degraded))
        );
        assert_eq!(level_of(&cs, check::FIRING), Some(Some(Level::Degraded)));
        assert_eq!(
            level_of(
                &checks(&HealthInputs {
                    state_file: Some(None),
                    ..base
                }),
                check::STATE_FILE
            ),
            Some(Some(Level::Ok))
        );

        let every = checks(&HealthInputs {
            presence: Some(Observer::Complete),
            tokens: Some(Observer::Complete),
            alarms: Some(Observer::Complete),
            doctor: Some(DoctorCheck {
                status: DoctorStatus::Pending,
                every: "6h",
                last: None,
            }),
            state_file: Some(None),
            ..base
        });
        assert_eq!(every.len(), 7);
        assert!(every.len() as u64 <= zenkey_model::health::MAX_CHECKS);
    }

    /// The doctor document's wire shape (#390): the report rides verbatim,
    /// `not asked` as absence inside it, and the whole thing round-trips
    /// through the type an explorer decodes.
    #[test]
    fn the_doctor_document_shape_is_pinned() {
        let d = ZenwatchDoctor {
            ran_at: "2026-09-06T01:00:00Z".into(),
            next_at: "2026-09-06T07:00:00Z".into(),
            every_s: 21600.0,
            outcome: DoctorOutcome::Failed,
            error: Some("timed out".into()),
            findings: 1,
            new: 0,
            fixed: 0,
            report: serde_json::json!({"scope": {"namespace": "site"}, "checks": [{
                "check": "split-brain", "section": "§6",
                "verdict": {"answer": "established"},
                "findings": [{"severity": "error", "check": "split-brain",
                    "subject": "h1/tc tc.v1", "evidence": "two holders"}]}]}),
            report_at: Some("2026-09-05T19:00:00Z".into()),
            delta: None,
        };
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(
            j,
            serde_json::json!({
                "ran_at": "2026-09-06T01:00:00Z",
                "next_at": "2026-09-06T07:00:00Z",
                "every_s": 21600.0,
                "outcome": "failed",
                "error": "timed out",
                "findings": 1,
                "new": 0,
                "fixed": 0,
                "report": d.report,
                "report_at": "2026-09-05T19:00:00Z",
                "delta": null,
            })
        );
        let back: ZenwatchDoctor = serde_json::from_value(j).unwrap();
        assert_eq!(back, d);
    }

    /// The counters' wire shape: flat counts, the discipline's nested, every
    /// field present (a zero is a zero, never absence).
    #[test]
    fn the_counters_shape_is_pinned() {
        let c = ZenwatchCounters {
            started_at: "2026-10-10T00:00:00Z".into(),
            rules: 4,
            sinks: 2,
            firing: 1,
            unobservable: 0,
            delivered: 7,
            deliveries_failed: 1,
            dropped: 0,
            forgotten: 0,
            state_entries: 3,
            firing_refused: 0,
            doctor_runs: 2,
            discipline: DisciplineCounters {
                sent: 7,
                deduped: 1,
                cancelled: 0,
                baseline: 2,
                grouped: 0,
                repeats: 0,
                evicted: 0,
            },
        };
        let j = serde_json::to_value(&c).unwrap();
        assert_eq!(
            j,
            serde_json::json!({
                "started_at": "2026-10-10T00:00:00Z",
                "rules": 4, "sinks": 2, "firing": 1, "unobservable": 0,
                "delivered": 7, "deliveries_failed": 1, "dropped": 0, "forgotten": 0,
                "state_entries": 3, "firing_refused": 0, "doctor_runs": 2,
                "discipline": {"sent": 7, "deduped": 1, "cancelled": 0, "baseline": 2,
                    "grouped": 0, "repeats": 0, "evicted": 0},
            })
        );
        let back: ZenwatchCounters = serde_json::from_value(j).unwrap();
        assert_eq!(back, c);
    }
}
