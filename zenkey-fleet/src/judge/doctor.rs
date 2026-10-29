//! zk2's doctor (#612, FJ6): the deployment judged against the core, one
//! check per rule a tool can see broken from outside.
//!
//! **Two halves.** [`observe`] reads the bus into a
//! [`DoctorObservation`] — values, every read with how it ended — and
//! [`judge`] turns one into a [`DoctorReport`] without a session, so every
//! verdict's honesty is testable from values ([`run_doctor`] is the two in
//! a row). The checks and their questions are [`CheckId`]'s; each verdict
//! is in the judgement shape, its finding on the *yes*
//! (`docs/zk2/tooling-guide.md` §1).
//!
//! **Two sessions** ([`DoctorBus`], decided 2026-10-08): everything about
//! the deployment is read through a session **in** its namespace —
//! presence, descriptors, bundles, archives, state — as its own consumers
//! read it; the routers' admin space and the presence domain's token count
//! are read through one in **no** namespace, because neither belongs to a
//! deployment.
//!
//! **What reading presence costs a verdict** (§8.1). Two presence reads,
//! [`DoctorSpec::grace`] apart, feed the checks that ask whether something
//! *lasts*: split-brain (§6's own procedure, through the runtime's
//! [`zenkey::ownership::compare`]) and token-missing, whose findings must hold
//! in both reads, because start-up and a re-mint pass through the same
//! shapes for a moment. A read that ended at its timeout can miss a token
//! and never invents one, so an absence it would claim is left unjudged;
//! one that saw no zk2 token at all is the empty scope, and the checks
//! that read presence are unobservable with it — never clean (tooling guide
//! §1). Absence is worded as what this reader could see: a read access
//! control refuses is complete and empty too (0.8).
//!
//! **What reading a contract costs a verdict** (§8.4). The revisions the
//! descriptors name are retrieved through a [`BundleStore`], absences
//! forgotten first so each run asks again. One no holder serves is
//! `contract-unavailable`'s finding, and every other check that needed it
//! leaves that subject unjudged with the reason, rather than guessing.
//!
//! **Not asked.** A check the spec leaves out is `NotAsked`, and so are
//! `state-stamp-foreign` and `population-over-bound` without
//! [`DoctorSpec::deep`]: their GETs, one per owner interface and shared
//! between them, cost the owners' data plane, the frugality v1's `--deep`
//! checks had.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::compat::Class;
use zenkey_model::contract::Contract;
use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
use zenkey_model::grammar::{Addr, Fp16, GRAMMAR, IfaceId, InstanceId, ZkKey};
use zenoh::Session;
use zenoh::query::{ConsolidationMode, QueryTarget};
use zenoh::sample::SampleKind;

use crate::bus::admin::{
    AdminEntry, ROUTERS, RouterVerification, STORAGES, admin_read, merge_storage_rows,
    router_from_admin_entry, storage_from_admin_entry,
};
use zenkey_model::health::{Level, Listing, Presence as HealthPresence, Reason, Verdict};

use crate::bus::config::{ConfigGet, ConfigTarget, ServiceDocs};
use crate::bus::contracts::BundleStore;
use crate::bus::health::{HealthGet, HealthTarget};
use crate::bus::presence::Scope;
use crate::judge::common::{FINDING_CAP, s1_premise};
use crate::model::catalog::zid_value;
use crate::model::catalog::{Catalog, ContractState, DescriptorRead, Observed, Revision};
use crate::model::examples::Examples;
use crate::model::health::{Got, Trust, agreement_over, listing, reading};
use crate::report::{
    Asked, CheckId, CheckReport, ConfigPremise, DoctorConfig, DoctorFinding, DoctorHealth,
    DoctorPresence, DoctorReport, DoctorScope, DoctorSeverity, HealthAnswer, HealthAnswerToken,
    RouterInfo, StorageInfo, Unjudged,
};

/// How far apart the two presence reads are, by default. Above the one
/// second an owner SHOULD keep a re-mint's overlap below (§6, §8.1), so a
/// make-before-break re-mint is in one read at most.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(2);

/// The presence budget judged against, by default: the low end of §8.3's
/// "about 10–15k tokens" per presence domain.
pub const DEFAULT_PRESENCE_BUDGET: usize = 10_000;

/// The selectors the presence domain's tokens are counted through, in no
/// namespace: every token whose key holds no verbatim chunk, and every zk2
/// token under any namespace (`*` and `**` never cross a verbatim chunk,
/// so `@zk` is named). Tokens of other applications under a verbatim chunk
/// are not counted, and the evidence says so (O5).
pub const DOMAIN_SELECTORS: [&str; 2] = ["**", "**/zk2/*/*/@zk/**"];

/// `archive.v1`'s interface id: its providers are the archives.
const ARCHIVE: &str = "archive.v1";

/// The checks that read `health.v1` (#721, PF).
const HEALTH_CHECKS: [CheckId; 4] = [
    CheckId::HealthFailed,
    CheckId::HealthDegraded,
    CheckId::HealthStale,
    CheckId::HealthInconsistent,
];

/// The checks that read `config.v1` (#768, CC).
const CONFIG_CHECKS: [CheckId; 4] = [
    CheckId::ConfigProjection,
    CheckId::ConfigOverdue,
    CheckId::ConfigUnpersisted,
    CheckId::ConfigDesired,
];

/// The checks that read `alarms.v1` (#800, AC).
const ALARM_CHECKS: [CheckId; 2] = [CheckId::AlarmKey, CheckId::AlarmStale];

/// The checks that read `desired.v1` (#840, DD).
const DESIRED_CHECKS: [CheckId; 3] = [
    CheckId::DesiredRefused,
    CheckId::DesiredBehind,
    CheckId::DesiredLastKnown,
];

/// How long a target may be behind a version its author stamped before
/// `desired-behind` finds it, by default: past spike S12's 1.6 s to
/// converge after a heal by far, and short of a deployment's re-read period
/// across a face, which a longer bound suits (`--desired-behind`).
pub const DEFAULT_DESIRED_BEHIND: Duration = Duration::from_secs(60);

/// What a doctor run asks.
#[derive(Debug, Clone)]
pub struct DoctorSpec {
    /// Each read's timeout: a liveliness GET, a descriptor GET, a
    /// retrieval attempt, an admin GET, an archive or state GET.
    pub timeout: Duration,
    /// How far apart the two presence reads are taken (§6). It SHOULD
    /// exceed the longest re-mint overlap the deployment allows.
    pub grace: Duration,
    /// The token count `presence-over-budget` judges against (§8.3).
    pub presence_budget: usize,
    /// Ask the checks that read the owners' data plane
    /// ([`CheckId::reads_data`]): `state-stamp-foreign` and
    /// `population-over-bound`, whose GETs of every owner's state cost it.
    pub deep: bool,
    /// The checks to ask; the rest are `NotAsked`.
    pub checks: BTreeSet<CheckId>,
    /// Trust every admin-space answer, on the operator's word that the
    /// deployment's grants deny `@/**` queryables to every principal
    /// (§4.2, §11.1, 0.12), which no tool can observe. Off by default:
    /// an answer is then trusted only when it is verifiably a router's.
    pub trust_admin: bool,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1; #721, PF). The
    /// doctor listens to no status long enough to measure a clock, so
    /// without it a status reply's age is unobservable, and so are the
    /// `health-*` checks of every service implementing `health.v1`.
    pub clocks_synced: bool,
    /// How long a `desired.v1` target may be "not yet" on a version its
    /// author stamped, by a clock trusted to the HLC delta, before
    /// `desired-behind` finds it (§2.17, text 0.4; #840).
    pub desired_behind: Duration,
}

impl DoctorSpec {
    /// Every check, with the default grace and budget, not deep.
    pub fn new(timeout: Duration) -> DoctorSpec {
        DoctorSpec {
            timeout,
            grace: DEFAULT_GRACE,
            presence_budget: DEFAULT_PRESENCE_BUDGET,
            deep: false,
            checks: CheckId::ALL.into_iter().collect(),
            trust_admin: false,
            clocks_synced: false,
            desired_behind: DEFAULT_DESIRED_BEHIND,
        }
    }

    /// Whether the run asks a `desired.v1` check, which reads every
    /// target's reports and each slice from its author (#840, DD).
    fn asks_desired(&self) -> bool {
        DESIRED_CHECKS.into_iter().any(|c| self.asks(c))
    }

    /// Whether the run asks a `health.v1` check, which reads every owner's
    /// `health.v1/state/**` (#721, PF).
    fn asks_health(&self) -> bool {
        HEALTH_CHECKS.into_iter().any(|c| self.asks(c))
    }

    /// Whether the run asks a `config.v1` check, which reads every owner's
    /// `config.v1/state/**` (#768, CC).
    fn asks_config(&self) -> bool {
        CONFIG_CHECKS.into_iter().any(|c| self.asks(c))
    }

    /// Whether the run asks an `alarms.v1` check, which reads every
    /// owner's `alarms.v1/state/alarms/*` (#800, AC).
    fn asks_alarms(&self) -> bool {
        ALARM_CHECKS.into_iter().any(|c| self.asks(c))
    }

    /// Whether the run asks `check`.
    pub fn asks(&self, check: CheckId) -> bool {
        self.checks.contains(&check) && (!check.reads_data() || self.deep)
    }

    fn asks_presence(&self) -> bool {
        CheckId::ALL
            .into_iter()
            .any(|c| c.reads_presence() && self.asks(c))
    }

    fn asks_admin(&self) -> bool {
        [
            CheckId::StorageOnState,
            CheckId::AdminUnreachable,
            CheckId::RouterVersionSkew,
            // §4.2 (0.17): an owner's stamp is clean only against the
            // routers this run verified.
            CheckId::StateStampForeign,
        ]
        .into_iter()
        .any(|c| self.asks(c))
    }
}

/// Where a doctor run reads (decided 2026-10-08): a session in the
/// deployment's namespace for the deployment, and one in no namespace for
/// the routers' admin space and the presence domain.
#[derive(Debug, Clone)]
pub struct DoctorBus {
    /// A session **in** the namespace.
    pub session: Session,
    /// A session in **no** namespace.
    pub raw: Session,
    /// The namespace `session` is in; empty for the bus root.
    pub namespace: String,
    /// A **watch session** for the namespace (#830, core §8.1, 0.37; #875):
    /// a session of its own, in no namespace on the deployment's wire keys
    /// ([`crate::open_watch`]), that holds a live reader's liveliness
    /// subscribers — the watchdog's `instance-gone` monitor, `alert-firing`'s
    /// and `check conform`'s instance tokens, a telemetry reading's tokens —
    /// without history, and makes no read. `session` and `raw` then hold
    /// none, and every read is made there. The doctor never uses it; `None`
    /// makes a reader that needs one unobservable, or refused, saying why.
    pub watch: Option<crate::WatchSession>,
}

// ─── what one run read ──────────────────────────────────────────────────────

/// Everything one doctor run read, as values: what [`judge`] decides from.
/// `None` is "not read", because no check that needs it was asked; an
/// `Err` is a read that could not be put on the bus, with why.
#[derive(Debug, Clone, Default)]
pub struct DoctorObservation {
    /// The namespace the deployment was read in.
    pub namespace: String,
    /// How far apart `before` and `after` were taken.
    pub grace: Duration,
    /// The first presence read: its tokens, and, when `hostid-duplicate` is
    /// asked, the descriptors of its instances on systems in the minted
    /// shape (#721, PF), read during the grace.
    pub before: Option<Result<Observed, String>>,
    /// The second, with every instance's descriptor.
    pub after: Option<Result<Observed, String>>,
    /// What asking for each revision the descriptors name found.
    pub contracts: BTreeMap<(IfaceId, Fingerprint), Result<ContractState, String>>,
    /// The routers' admin space.
    pub admin: Option<Result<AdminSpace, String>>,
    /// The presence domain's tokens.
    pub domain: Option<Result<DomainTokens, String>>,
    /// Each archive's keys.
    pub archives: BTreeMap<Addr, Result<ArchiveKeys, String>>,
    /// Each owner's state replies, by service and interface.
    pub stamps: BTreeMap<(Addr, IfaceId), Result<StateStamps, String>>,
    /// This process's `RLIMIT_MEMLOCK`.
    pub memlock: Option<Memlock>,
    /// `health.v1`'s readings (#721, PF), the first first: one GET of every
    /// owner's `health.v1/state/**` with the second presence read, and one
    /// with the first when `health-inconsistent` is asked. Empty when no
    /// `health-*` check was asked.
    pub health: Vec<Result<HealthGet, String>>,
    /// `config.v1`'s readings (#768, CC), the first first: one GET of every
    /// owner's `config.v1/state/**` with the second presence read, and one
    /// with the first when `config-projection` is asked. Empty when no
    /// `config-*` check was asked.
    pub config: Vec<Result<ConfigGet, String>>,
    /// `alarms.v1`'s reading (#800, AC): one S4 GET of every owner's
    /// `alarms.v1/state/alarms/*`, with the second presence read. `None`
    /// when no `alarm-*` check was asked.
    pub alarms: Option<Result<crate::model::alarms::SeedRead, String>>,
    /// `desired.v1`'s reading (#840, DD): every target's roles over the
    /// second presence read and the revisions retrieved, its reports and
    /// each slice from its author. `None` when no `desired-*` check was
    /// asked.
    pub desired: Option<crate::model::desired::DesiredRead>,
}

/// The routers' admin space, as two reads found it.
#[derive(Debug, Clone, Default)]
pub struct AdminSpace {
    /// The routers whose answers were verified (below).
    pub routers: Vec<RouterInfo>,
    /// The storages those routers run.
    pub storages: Vec<StorageInfo>,
    /// Whether both reads ended at the routers' final reply.
    pub complete: bool,
    /// Every zid this run knows to be a router, by value (§4.2, "A tool's
    /// S1 check", 0.17): the routers its session is connected to, the
    /// verified ones, and every zid a verified router lists as a `router`
    /// session. Never this session's own.
    pub router_zids: BTreeSet<String>,
    /// Answers that could not be shown to be a router's (§4.2, 0.12,
    /// F-80), one line each: any session can answer under
    /// `@/<zid>/router`. An answer is verified when its replier id is the
    /// zid its key names and that zid is a router this session is
    /// connected to, or the session itself. Never counted toward a clean
    /// verdict.
    pub unverified: Vec<String>,
}

/// The presence domain's tokens, counted through [`DOMAIN_SELECTORS`].
#[derive(Debug, Clone, Default)]
pub struct DomainTokens {
    /// Distinct tokens across both reads.
    pub tokens: usize,
    /// Of which zk2 tokens, under any namespace.
    pub zk2: usize,
    /// The namespaces those sit under, the bus root included.
    pub namespaces: usize,
    /// Whether both reads ended at the routers' final reply.
    pub complete: bool,
}

/// One archive's keys (§4.4): what a GET over its `@state` served.
#[derive(Debug, Clone, Default)]
pub struct ArchiveKeys {
    /// Values served.
    pub values: usize,
    /// Values served `confirmed: false`, or with no readable attachment,
    /// which §4.4 reads the same way.
    pub unconfirmed: usize,
    /// The first of them, by key.
    pub examples: Vec<String>,
    /// Tombstones served (`reply_del`): positive evidence, never unaligned.
    pub tombstones: usize,
    /// Whether the GET ended at the final reply.
    pub complete: bool,
}

/// An owner's state replies (S4's GET), grouped by the clock that stamped
/// them: `None` for an unstamped reply.
#[derive(Debug, Clone, Default)]
pub struct StateStamps {
    /// Per clock: how many replies, and the first keys.
    pub by_clock: BTreeMap<Option<String>, (usize, Vec<String>)>,
    /// Every key answered with a value, not a `reply_del`: the members
    /// `population-over-bound` counts (§2.7, 0.24).
    pub values: BTreeSet<String>,
    /// The value replies of the interface's metric resources, when
    /// `telemetry-sample` is asked: what it reads (#819, TC). At most
    /// [`STATE_REPLY_CAP`] of them and [`STATE_REPLY_BYTES`] of payload and
    /// attachment; every other reply keeps its stamp alone.
    pub replies: Vec<StateReply>,
    /// Metric value replies past either cap, not kept.
    pub replies_capped: u64,
    /// Whether every GET ended at the final reply.
    pub complete: bool,
}

/// How many value replies of one owner's interface a run keeps for
/// `telemetry-sample`.
pub const STATE_REPLY_CAP: usize = 4_096;

/// How many bytes of payload and attachment of one owner's interface a run
/// keeps for `telemetry-sample`: 8 MiB.
pub const STATE_REPLY_BYTES: usize = 8 * 1024 * 1024;

/// What `telemetry-sample` keeps of one owner interface's state replies
/// (#819, TC): the replies on its metric resources' keys alone, by the
/// revision the owner's descriptors name, within [`STATE_REPLY_CAP`] and
/// [`STATE_REPLY_BYTES`]; a metric reply past either is counted in
/// [`StateStamps::replies_capped`], so the check says what it could not
/// read. No revision in hand, or `telemetry-sample` not asked: nothing.
#[derive(Debug, Default)]
struct ReplyKeeper {
    revision: Option<Arc<Revision>>,
    bytes: usize,
}

impl ReplyKeeper {
    fn offer(
        &mut self,
        out: &mut StateStamps,
        key: &str,
        size: usize,
        reply: impl FnOnce() -> StateReply,
    ) {
        let Some(rev) = &self.revision else {
            return;
        };
        if metric_of(rev.contract(), key).is_none() {
            return;
        }
        if out.replies.len() < STATE_REPLY_CAP && self.bytes + size <= STATE_REPLY_BYTES {
            self.bytes += size;
            out.replies.push(reply());
        } else {
            out.replies_capped += 1;
        }
    }
}

/// The metric resource `key` is a member of in `contract` (§2.2: match,
/// then rank), when it is one (`telemetry.v1` §2.1).
fn metric_of(contract: &Contract, key: &str) -> Option<crate::model::telemetry::MetricType> {
    let Ok(ZkKey::Data { kind, resource, .. }) = zenkey_model::grammar::parse(key) else {
        return None;
    };
    let candidates: Vec<&zenkey_model::contract::Resource> = contract
        .resources
        .iter()
        .filter(|r| r.token == kind)
        .collect();
    let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
    let (i, _) = zenkey_model::template::resolve(candidates.iter().map(|r| &r.template), &refs)?;
    crate::model::telemetry::MetricType::of(contract, candidates[i])
}

/// The one revision of `iface` `addr`'s served descriptors name, when it is
/// in hand: what `telemetry-sample` reads its replies by.
fn metric_revision(
    observed: &Observed,
    contracts: &BTreeMap<(IfaceId, Fingerprint), Result<ContractState, String>>,
    addr: &Addr,
    iface: &IfaceId,
) -> Option<Arc<Revision>> {
    let want = iface.to_string();
    let fps: BTreeSet<&str> = served(observed)
        .filter(|(a, _, _)| *a == addr)
        .filter_map(|(_, _, d)| d.interfaces.iter().find(|e| e.iface == want))
        .map(|e| e.contract.as_str())
        .collect();
    let [fp] = fps.into_iter().collect::<Vec<_>>()[..] else {
        return None;
    };
    match contracts.get(&(iface.clone(), Fingerprint::parse(fp).ok()?)) {
        Some(Ok(ContractState::Held(r))) => Some(Arc::clone(r)),
        _ => None,
    }
}

/// One value reply of an owner's state GET, as it came.
#[derive(Debug, Clone, Default)]
pub struct StateReply {
    /// The key, base-relative.
    pub key: String,
    pub payload: Vec<u8>,
    pub attachment: Option<Vec<u8>>,
}

/// `RLIMIT_MEMLOCK`, as `zenkey::shm` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memlock {
    /// The soft limit, in bytes.
    Limited(u64),
    /// No limit.
    Unlimited,
    /// The limit could not be read (#677): the check is unobservable.
    Unknown,
}

// ─── the run ────────────────────────────────────────────────────────────────

/// One doctor run: [`observe`], then [`judge`].
///
/// `store` keeps verified revisions across runs (a bundle is
/// content-addressed); its absences are forgotten at the start of each, so
/// a revision is asked about again every run.
pub async fn run_doctor(bus: &DoctorBus, store: &BundleStore, spec: &DoctorSpec) -> DoctorReport {
    let observation = observe(bus, store, spec).await;
    judge(&observation, spec)
}

/// Reads what the checks `spec` asks need. Reads that do not depend on one
/// another run at once; the presence reads take `grace` between them.
pub async fn observe(bus: &DoctorBus, store: &BundleStore, spec: &DoctorSpec) -> DoctorObservation {
    let t = spec.timeout;
    let presence = async {
        if !spec.asks_presence() {
            return Default::default();
        }
        presence_phase(bus, store, spec).await
    };
    let admin = async {
        if !spec.asks_admin() {
            return None;
        }
        Some(admin_space(&bus.raw, t, spec.trust_admin).await)
    };
    let domain = async {
        if !spec.asks(CheckId::PresenceOverBudget) {
            return None;
        }
        Some(domain_tokens(&bus.raw, t).await)
    };
    let (p, admin, domain) = tokio::join!(presence, admin, domain);
    DoctorObservation {
        namespace: bus.namespace.clone(),
        grace: spec.grace,
        before: p.before,
        after: p.after,
        contracts: p.contracts,
        admin,
        domain,
        archives: p.archives,
        stamps: p.stamps,
        memlock: spec
            .asks(CheckId::ShmMemlockLow)
            .then(|| match zenkey::shm::memlock() {
                zenkey::shm::Memlock::Limited(l) => Memlock::Limited(l),
                zenkey::shm::Memlock::Unlimited => Memlock::Unlimited,
                zenkey::shm::Memlock::Unknown => Memlock::Unknown,
            }),
        health: p.health,
        config: p.config,
        alarms: p.alarms,
        desired: p.desired,
    }
}

/// What the presence half of a run read.
#[derive(Default)]
struct PresencePhase {
    before: Option<Result<Observed, String>>,
    after: Option<Result<Observed, String>>,
    contracts: BTreeMap<(IfaceId, Fingerprint), Result<ContractState, String>>,
    archives: BTreeMap<Addr, Result<ArchiveKeys, String>>,
    stamps: BTreeMap<(Addr, IfaceId), Result<StateStamps, String>>,
    health: Vec<Result<HealthGet, String>>,
    config: Vec<Result<ConfigGet, String>>,
    alarms: Option<Result<crate::model::alarms::SeedRead, String>>,
    desired: Option<crate::model::desired::DesiredRead>,
}

async fn presence_phase(bus: &DoctorBus, store: &BundleStore, spec: &DoctorSpec) -> PresencePhase {
    let (s, t) = (&bus.session, spec.timeout);
    let scope = Scope::all();
    let health = HealthTarget::All.state_selector();
    let config = ConfigTarget::All.state_selector(false);
    let mut readings = Vec::new();
    let mut config_readings = Vec::new();
    let before = if [
        CheckId::SplitBrain,
        CheckId::TokenMissing,
        CheckId::HostidDuplicate,
        CheckId::HealthInconsistent,
        CheckId::ConfigProjection,
        CheckId::RedundancyVacancy,
    ]
    .into_iter()
    .any(|c| spec.asks(c))
    {
        let read = crate::bus::presence::read_tokens(s, &scope, t)
            .await
            .map_err(|e| crate::one_line(&e));
        // During the grace: the first health reading, and the descriptors
        // hostid-duplicate counts (hostid.v1 §2.12): every instance on a
        // system in the minted shape, the only systems a minting can name.
        let first = async {
            if spec.asks(CheckId::HealthInconsistent) {
                Some(crate::bus::health::get(s, &health, t).await)
            } else {
                None
            }
        };
        let first_config = async {
            if spec.asks(CheckId::ConfigProjection) {
                Some(crate::bus::config::get(s, &config, t).await)
            } else {
                None
            }
        };
        let described = async {
            let mut read = read;
            if let (true, Ok(o)) = (spec.asks(CheckId::HostidDuplicate), &mut read) {
                crate::bus::presence::describe_where(s, o, t, |a, _| {
                    zenkey_model::hostid::is_minted_shape(a.system.as_str())
                })
                .await;
            }
            read
        };
        let (_, first, first_config, read) = tokio::join!(
            tokio::time::sleep(spec.grace),
            first,
            first_config,
            described
        );
        readings.extend(first);
        config_readings.extend(first_config);
        Some(read)
    } else {
        None
    };
    let after = crate::bus::presence::observe(s, &scope, t)
        .await
        .map_err(|e| crate::one_line(&e));
    let Ok(observed) = &after else {
        return PresencePhase {
            before,
            after: Some(after),
            ..Default::default()
        };
    };
    let mut wanted = Catalog::new(observed).wanted();
    if let Some(Ok(b)) = &before {
        // The revisions the first read's descriptors name: an instance gone
        // by the second read is still counted by what it implements.
        let more = Catalog::new(b).wanted();
        wanted.extend(
            more.into_iter()
                .filter(|w| !wanted.contains(w))
                .collect::<Vec<_>>(),
        );
    }
    store.forget_absences();
    let contracts = async {
        let answers = store.fetch_all(s, &wanted).await;
        wanted
            .iter()
            .cloned()
            .zip(
                answers
                    .into_iter()
                    .map(|a| a.map_err(|e| crate::one_line(&e))),
            )
            .collect::<BTreeMap<_, _>>()
    };
    let archives = async {
        if !spec.asks(CheckId::ArchiveUnaligned) {
            return BTreeMap::new();
        }
        let mut out = BTreeMap::new();
        for addr in archives_in(observed) {
            let read = archive_keys(s, &addr, t).await;
            out.insert(addr, read);
        }
        out
    };
    // The owners' state GETs follow the retrieval: `telemetry-sample` keeps
    // the replies of the metric resources alone, by the revisions retrieved.
    let contracts_then_stamps = async {
        let contracts = contracts.await;
        let mut out = BTreeMap::new();
        if !spec.asks(CheckId::StateStampForeign)
            && !spec.asks(CheckId::PopulationOverBound)
            && !spec.asks(CheckId::TelemetrySample)
        {
            return (contracts, out);
        }
        for (addr, iface) in owners_in(observed) {
            let keep = ReplyKeeper {
                revision: spec
                    .asks(CheckId::TelemetrySample)
                    .then(|| metric_revision(observed, &contracts, &addr, &iface))
                    .flatten(),
                bytes: 0,
            };
            let read = state_stamps(s, &addr, &iface, t, keep).await;
            out.insert((addr, iface), read);
        }
        (contracts, out)
    };
    let second = async {
        if spec.asks_health() {
            Some(crate::bus::health::get(s, &health, t).await)
        } else {
            None
        }
    };
    let second_config = async {
        if spec.asks_config() {
            Some(crate::bus::config::get(s, &config, t).await)
        } else {
            None
        }
    };
    let alarms = async {
        if spec.asks_alarms() {
            Some(crate::bus::alarms::get(s, crate::model::alarms::ALL, t).await)
        } else {
            None
        }
    };
    let ((contracts, stamps), archives, second, second_config, alarms) = tokio::join!(
        contracts_then_stamps,
        archives,
        second,
        second_config,
        alarms
    );
    readings.extend(second);
    config_readings.extend(second_config);
    // desired.v1 (#840, DD): over this presence read and these revisions,
    // nothing read again but the targets' reports and the slices.
    let desired = if spec.asks_desired() {
        let mut ds = crate::bus::desired::DesiredSpec::new(t);
        ds.clocks_synced = spec.clocks_synced;
        // No check reads an archive's last-known: its presence read would
        // cost a third.
        ds.archives = false;
        Some(
            crate::bus::desired::observe_in(
                s,
                observed.clone(),
                contracts.clone(),
                &ds,
                store,
                &crate::model::catalog::ContractSet::new(),
            )
            .await,
        )
    } else {
        None
    };
    PresencePhase {
        before,
        after: Some(after),
        contracts,
        archives,
        stamps,
        health: readings,
        config: config_readings,
        alarms,
        desired,
    }
}

/// `telemetry.v1` §2.2 (#819, TC): every metric resource of every revision
/// the descriptors name, its declaration read from the contract
/// (`zenkey_model::telemetry::declaration_of`). An invalid one is the
/// warning a contract linter gives: no reader reads its members' kind,
/// conformance or continuity. A revision not in hand is unjudged.
fn telemetry_declaration(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::TelemetryDeclaration;
    use crate::model::telemetry::{MetricType, invalid_words};
    use zenkey_model::telemetry::Declaration;
    let mut named: BTreeMap<(IfaceId, Fingerprint), BTreeSet<String>> = BTreeMap::new();
    for (a, _, d) in p.served() {
        for e in &d.interfaces {
            if let (Ok(iface), Ok(fp)) =
                (IfaceId::from_str(&e.iface), Fingerprint::parse(&e.contract))
            {
                named.entry((iface, fp)).or_default().insert(a.to_string());
            }
        }
    }
    let mut findings = Vec::new();
    let mut undecided = p.undescribed("the revisions it implements are unknown");
    let mut metrics = 0usize;
    for ((iface, fp), by) in &named {
        let rev = match p.revision(iface, fp) {
            Ok(r) => r,
            Err(why) => {
                undecided.push(unjudged(format!("{iface} {fp}"), why));
                continue;
            }
        };
        let c = rev.contract();
        for r in &c.resources {
            let Some(ty) = MetricType::of(c, r) else {
                continue;
            };
            metrics += 1;
            if let Declaration::Invalid(why) = &ty.declaration {
                findings.push(finding(
                    C,
                    DoctorSeverity::Warning,
                    format!("{iface} {fp} {}", zenkey::implementation::resource_name(r)),
                    format!(
                        "implemented by {}; its declaration is invalid: {} — no reader reads its \
                         members' kind, conformance or continuity (telemetry.v1 §2.2)",
                        by.iter().cloned().collect::<Vec<_>>().join(", "),
                        invalid_words(why)
                    ),
                ));
            }
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{metrics} metric resource(s) in {} revision(s) named by descriptors, each \
             declaration valid",
            named.len()
        ),
    )
}

/// `timing.v1` §2.2, §2.3, §2.5 (#908, MC): every resource and every
/// requirement of every revision the descriptors name, its timing
/// declaration read from the contract (`zenkey_model::timing::declaration_of`
/// and `requirement_declaration`). The warnings a contract linter gives,
/// each something no reader judges: an invalid value (no question of its
/// members is answered), a deadline below its period (no deadline), and
/// timing keys on a state, an event, an operation or a requirement, where
/// they mean nothing (`[defaults]` included, which `[defaults.stream]`
/// would have scoped). A revision not in hand is unjudged.
fn timing_declaration(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::TimingDeclaration;
    use zenkey_model::timing::{Declaration, declaration_of, requirement_declaration};
    let mut named: BTreeMap<(IfaceId, Fingerprint), BTreeSet<String>> = BTreeMap::new();
    for (a, _, d) in p.served() {
        for e in &d.interfaces {
            if let (Ok(iface), Ok(fp)) =
                (IfaceId::from_str(&e.iface), Fingerprint::parse(&e.contract))
            {
                named.entry((iface, fp)).or_default().insert(a.to_string());
            }
        }
    }
    let mut findings = Vec::new();
    let mut undecided = p.undescribed("the revisions it implements are unknown");
    let mut timed = 0usize;
    for ((iface, fp), by) in &named {
        let rev = match p.revision(iface, fp) {
            Ok(r) => r,
            Err(why) => {
                undecided.push(unjudged(format!("{iface} {fp}"), why));
                continue;
            }
        };
        let by = by.iter().cloned().collect::<Vec<_>>().join(", ");
        let c = rev.contract();
        let mut warn = |subject: String, why: String| {
            findings.push(finding(
                C,
                DoctorSeverity::Warning,
                format!("{iface} {fp} {subject}"),
                format!("implemented by {by}; {why}"),
            ));
        };
        for r in &c.resources {
            let name = zenkey::implementation::resource_name(r);
            match declaration_of(r) {
                Declaration::NotThisProfiles => {}
                Declaration::Ignored => warn(
                    name,
                    format!(
                        "timing keys on a {} resource mean nothing (timing.v1 §2.3): only a \
                         stream has a period, a deadline or a lifespan; a key in [defaults] \
                         reaches every kind, where [defaults.stream] would not",
                        r.kind.as_str()
                    ),
                ),
                Declaration::Invalid(why) => {
                    timed += 1;
                    warn(
                        name,
                        format!(
                            "its timing declaration is invalid: {} — no reader judges its \
                             period, deadline or lifespan (timing.v1 §2.2)",
                            why.iter()
                                .map(|w| format!("{} ({})", w.says(), w.as_str()))
                                .collect::<Vec<_>>()
                                .join("; ")
                        ),
                    );
                }
                Declaration::Declared(d) => {
                    timed += 1;
                    if let (true, Some(period), Some(deadline)) =
                        (d.below_period(), d.period, d.deadline)
                    {
                        warn(
                            name,
                            format!(
                                "its deadline, {} ms, is below its period, {} ms, so it is no \
                                 deadline: an owner keeping its period would be missed between \
                                 every two samples, and no reader judges one (timing.v1 §2.5)",
                                deadline.as_millis(),
                                period.as_millis()
                            ),
                        );
                    }
                }
            }
        }
        for (role, req) in &c.requires {
            if requirement_declaration(&req.annotations) == Declaration::Ignored {
                warn(
                    format!("requires.{role}"),
                    "timing keys on a requirement mean nothing at this version (timing.v1 \
                     §2.3): a descriptor's requirement entry carries no annotations, and a \
                     tightened deadline belongs to a binding, which has no field for it"
                        .to_owned(),
                );
            }
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{timed} timed resource(s) in {} revision(s) named by descriptors, each declaration \
             one a reader judges",
            named.len()
        ),
    )
}

/// `telemetry.v1` §2.6 (#819, TC), with `--deep`: every value reply of the
/// owners' state GETs (the ones `state-stamp-foreign` makes) that is a
/// sample of a state metric member, read by `zenkey_model::telemetry::check`.
/// One that does not conform is the finding. One that does not decode, or
/// of a member whose declaration is invalid, is unjudged: its conformance is
/// unobservable, never skipped (§5, 0.5; zk2py round 48's F-157), and the
/// declaration's warning stays `telemetry-declaration`'s. Streams are not
/// read here: the
/// doctor listens to no window, and `zenctl telemetry` and `check conform`
/// read them.
fn telemetry_sample(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::TelemetrySample;
    use crate::model::telemetry::problem_words;
    use zenkey_model::telemetry::{Conformance as Tc, Unasked};
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let (mut judged, mut capped) = (0usize, 0u64);
    for ((addr, iface), read) in &p.obs.stamps {
        let subject = format!("{addr} {iface}");
        let stamps = match read {
            Ok(s) => s,
            Err(e) => {
                undecided.push(unjudged(
                    subject,
                    format!("its state could not be read: {e}"),
                ));
                continue;
            }
        };
        let want = iface.to_string();
        let fps: BTreeSet<&str> = p
            .served()
            .filter(|(a, _, _)| *a == addr)
            .filter_map(|(_, _, d)| d.interfaces.iter().find(|e| e.iface == want))
            .map(|e| e.contract.as_str())
            .collect();
        let entry = p
            .served()
            .filter(|(a, _, _)| *a == addr)
            .find_map(|(_, _, d)| d.interfaces.iter().find(|e| e.iface == want));
        let Some(entry) = entry else {
            continue;
        };
        if fps.len() > 1 {
            undecided.push(unjudged(
                subject,
                format!(
                    "its instances claim {} revisions of {iface}: which declaration its samples \
                     are read by cannot be told",
                    fps.len()
                ),
            ));
            continue;
        }
        let rev = match p.entry_revision(entry) {
            Ok(r) => r,
            Err(why) => {
                undecided.push(unjudged(subject, why));
                continue;
            }
        };
        let contract = rev.contract();
        capped += stamps.replies_capped;
        for reply in &stamps.replies {
            let Some(ty) = metric_of(contract, &reply.key) else {
                continue;
            };
            match ty
                .check(&reply.payload, reply.attachment.as_deref())
                .conformance
            {
                Tc::Conforms => judged += 1,
                Tc::DoesNot(ps) => {
                    judged += 1;
                    findings.push(finding(
                        C,
                        DoctorSeverity::Error,
                        reply.key.clone(),
                        format!(
                            "its owner's reply does not conform: {}",
                            ps.into_iter()
                                .map(problem_words)
                                .collect::<Vec<_>>()
                                .join("; ")
                        ),
                    ));
                }
                Tc::Unobservable(Unasked::Undecodable) => undecided.push(unjudged(
                    reply.key.clone(),
                    "its owner's reply does not decode as §2.6 reads bytes (strict JSON, or \
                     CBOR): unobservable here, the core's verdict standing (undecodable)",
                )),
                Tc::Unobservable(Unasked::DeclarationInvalid) => undecided.push(unjudged(
                    reply.key.clone(),
                    "its resource's declaration is invalid, so no reader reads its samples: \
                     unobservable here, the warning telemetry-declaration's \
                     (declaration_invalid, telemetry.v1 §2.2)",
                )),
                Tc::Unobservable(_) | Tc::NotAsked(_) => {}
            }
        }
    }
    let capped = if capped > 0 {
        format!(
            "; {capped} reply(ies) past the run's caps ({STATE_REPLY_CAP} replies, 8 MiB per owner \
             interface) were not read"
        )
    } else {
        String::new()
    };
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{judged} state metric sample(s) in the owners' GET replies, each conforming{capped}; \
             streams are read over a window, by zenctl telemetry and check conform"
        ),
    )
}

/// Every address presence shows providing `archive.v1`, by token or by
/// descriptor.
fn archives_in(observed: &Observed) -> BTreeSet<Addr> {
    let archive = IfaceId::from_str(ARCHIVE).expect("archive.v1 is an interface id");
    Catalog::new(observed)
        .providers_of(&archive)
        .into_iter()
        .collect()
}

/// Every (service, interface) a served descriptor lists, `archive.v1`
/// aside: an archive answers with the stamps of the owners it recorded
/// (§4.4), never its own.
fn owners_in(observed: &Observed) -> BTreeSet<(Addr, IfaceId)> {
    served(observed)
        .flat_map(|(addr, _, d)| {
            d.interfaces
                .iter()
                .filter(|e| e.iface != ARCHIVE)
                .filter_map(move |e| Some((addr.clone(), IfaceId::from_str(&e.iface).ok()?)))
        })
        .collect()
}

/// Every served descriptor, with its instance.
fn served(observed: &Observed) -> impl Iterator<Item = (&Addr, &InstanceId, &Descriptor)> {
    observed
        .descriptors
        .iter()
        .flatten()
        .filter_map(|((a, i), read)| Some((a, i, read.descriptor()?)))
}

/// The routers' admin space: `@/*/router` and the storages, in no
/// namespace. Either read failing is the whole answer failing. Each answer
/// is verified as a router's (§4.2, 0.12–0.13): its replier id is the zid
/// its key names, and that zid is a router this session is connected to
/// (or the session itself), or one a verified router's document lists as
/// a `router` session; or, with `trust`, on the operator's word.
pub(crate) async fn admin_space(
    raw: &Session,
    timeout: Duration,
    trust: bool,
) -> Result<AdminSpace, String> {
    let here = crate::bus::admin::here(raw).await;
    let (routers, storages) = tokio::join!(
        admin_read(raw, ROUTERS, timeout),
        admin_read(raw, STORAGES, timeout)
    );
    let routers = routers.map_err(|e| crate::one_line(&e))?;
    let storages = storages.map_err(|e| crate::one_line(&e))?;
    let complete = routers.complete && storages.complete;
    // Routers, verified outward (§4.2, 0.12–0.13): the rule is the admin
    // module's, shared with `admin graph`'s instance join (#705).
    let verification = RouterVerification::new(here, &routers.entries, trust);
    let own = zid_value(&raw.zid().to_string());
    let router_zids = verification
        .listed()
        .iter()
        .chain(verification.routers())
        .filter(|z| **z != own)
        .cloned()
        .collect();
    let mut unverified = BTreeSet::new();
    let mut verified = |e: &AdminEntry| -> bool {
        match verification.check(e) {
            Ok(()) => true,
            Err(line) => {
                unverified.insert(line);
                false
            }
        }
    };
    let routers: Vec<RouterInfo> = routers
        .entries
        .into_iter()
        .filter(|e| verified(e))
        .map(router_from_admin_entry)
        .collect();
    let storages = merge_storage_rows(
        storages
            .entries
            .iter()
            .filter(|e| verified(e))
            .filter_map(|e| storage_from_admin_entry(&e.key, &e.value))
            .collect(),
    );
    Ok(AdminSpace {
        complete,
        routers,
        router_zids,
        storages,
        unverified: unverified.into_iter().collect(),
    })
}

/// The presence domain's tokens through [`DOMAIN_SELECTORS`], in no
/// namespace, on the liveliness chokepoint's unbounded handler (§8.1).
async fn domain_tokens(raw: &Session, timeout: Duration) -> Result<DomainTokens, String> {
    let [all, zk2] = DOMAIN_SELECTORS;
    let (plain, zk2_read) = tokio::join!(
        crate::bus::presence::liveliness_read(raw, all, timeout),
        crate::bus::presence::liveliness_read(raw, zk2, timeout)
    );
    let plain = plain.map_err(|e| crate::one_line(&e))?;
    let zk2_read = zk2_read.map_err(|e| crate::one_line(&e))?;
    let namespaces = crate::model::catalog::namespaces(zk2, &zk2_read.keys, zk2_read.complete)
        .namespaces
        .len();
    let distinct: BTreeSet<&String> = plain.keys.iter().chain(&zk2_read.keys).collect();
    Ok(DomainTokens {
        tokens: distinct.len(),
        zk2: zk2_read.keys.len(),
        namespaces,
        complete: plain.complete && zk2_read.complete,
    })
}

/// One archive's keys: a GET over its whole `@state` (§4.4), target `All`
/// and consolidation `None`, each value's attachment read for
/// `confirmed`.
async fn archive_keys(s: &Session, addr: &Addr, timeout: Duration) -> Result<ArchiveKeys, String> {
    let selector = [
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        ARCHIVE,
        "@state",
        "**",
    ]
    .join("/");
    let replies = s
        .get(&selector)
        .target(QueryTarget::All)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .await
        .map_err(|e| format!("GET {selector}: {e}"))?;
    let mut keys = ArchiveKeys {
        complete: true,
        ..ArchiveKeys::default()
    };
    let mut examples = Examples::new(FINDING_CAP);
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            keys.complete = false;
            continue;
        };
        if sample.kind() == SampleKind::Delete {
            keys.tombstones += 1;
            continue;
        }
        keys.values += 1;
        let confirmed = sample
            .attachment()
            .and_then(|a| serde_json::from_slice::<serde_json::Value>(&a.to_bytes()).ok())
            .and_then(|v| v["confirmed"].as_bool())
            .unwrap_or(false);
        if !confirmed {
            keys.unconfirmed += 1;
            examples.push(sample.key_expr().as_str().to_owned());
        }
    }
    keys.examples = examples.into_vec();
    Ok(keys)
}

/// An owner's state, read as S4 reads it — target `All`, consolidation
/// `Latest` — over `state/**` and `@state/**` of one interface, each reply
/// grouped by the clock that stamped it.
async fn state_stamps(
    s: &Session,
    addr: &Addr,
    iface: &IfaceId,
    timeout: Duration,
    mut keep: ReplyKeeper,
) -> Result<StateStamps, String> {
    let iface = iface.to_string();
    let mut out = StateStamps {
        complete: true,
        ..StateStamps::default()
    };
    for token in ["state", "@state"] {
        let selector = [
            GRAMMAR,
            addr.system.as_str(),
            addr.service.as_str(),
            iface.as_str(),
            token,
            "**",
        ]
        .join("/");
        let replies = s
            .get(&selector)
            .target(QueryTarget::All)
            .consolidation(ConsolidationMode::Latest)
            .timeout(timeout)
            .await
            .map_err(|e| format!("GET {selector}: {e}"))?;
        while let Ok(reply) = replies.recv_async().await {
            let Ok(sample) = reply.result() else {
                out.complete = false;
                continue;
            };
            let clock = sample.timestamp().map(|t| t.get_id().to_string());
            let (n, keys) = out.by_clock.entry(clock).or_default();
            *n += 1;
            if keys.len() < crate::judge::common::EXPANSION_CAP {
                keys.push(sample.key_expr().as_str().to_owned());
            }
            if sample.kind() == SampleKind::Put {
                let key = sample.key_expr().as_str();
                out.values.insert(key.to_owned());
                let size = sample.payload().len() + sample.attachment().map_or(0, |a| a.len());
                keep.offer(&mut out, key, size, || StateReply {
                    key: key.to_owned(),
                    payload: sample.payload().to_bytes().into_owned(),
                    attachment: sample.attachment().map(|a| a.to_bytes().into_owned()),
                });
            }
        }
    }
    Ok(out)
}

// ─── the judgement ──────────────────────────────────────────────────────────

/// The checks `spec` asks, judged from `obs` — no session, so every
/// verdict below is decided from values a test can write.
pub fn judge(obs: &DoctorObservation, spec: &DoctorSpec) -> DoctorReport {
    let presence: Result<Presence<'_>, String> = match &obs.after {
        None => Err("presence was not read".into()),
        Some(Err(e)) => Err(format!("the presence read failed: {e}")),
        Some(Ok(after)) if after.tokens.is_empty() => Err(empty_scope(&obs.namespace, after)),
        Some(Ok(after)) => Ok(Presence::new(obs, after)),
    };
    let checks = CheckId::ALL
        .into_iter()
        .map(|check| {
            if !spec.asks(check) {
                return CheckReport::not_asked(check);
            }
            if !check.reads_presence() {
                return match check {
                    CheckId::PresenceOverBudget => presence_over_budget(obs, spec),
                    CheckId::StorageOnState => storage_on_state(obs),
                    CheckId::ShmMemlockLow => shm_memlock_low(obs),
                    CheckId::AdminUnreachable => admin_unreachable(obs),
                    CheckId::RouterVersionSkew => router_version_skew(obs),
                    _ => unreachable!("every check that reads no presence is above"),
                };
            }
            let p = match &presence {
                Ok(p) => p,
                Err(why) => return CheckReport::unobservable(check, why.clone()),
            };
            match check {
                CheckId::SplitBrain => split_brain(p),
                CheckId::BindingUnsatisfied => binding_unsatisfied(p),
                CheckId::ContractDrift => contract_drift(p),
                CheckId::ContractUnavailable => contract_unavailable(p),
                CheckId::DescriptorInvalid => descriptor_invalid(p),
                CheckId::TokenMissing => token_missing(p),
                CheckId::ArchiveUnaligned => archive_unaligned(p),
                CheckId::StateStampForeign => state_stamp_foreign(p),
                CheckId::HealthFailed => health_level(p, spec, Level::Failed),
                CheckId::HealthDegraded => health_level(p, spec, Level::Degraded),
                CheckId::HealthStale => health_stale(p, spec),
                CheckId::HealthInconsistent => health_inconsistent(p, spec),
                CheckId::HostidDuplicate => hostid_duplicate(p),
                CheckId::PopulationOverBound => population_over_bound(p),
                CheckId::ConfigProjection => config_projection(p),
                CheckId::ConfigOverdue => config_overdue(p),
                CheckId::ConfigUnpersisted => config_unpersisted(p, spec),
                CheckId::ConfigDesired => config_desired(p, spec),
                CheckId::AlarmKey => alarm_key(p, spec),
                CheckId::AlarmStale => alarm_stale(p, spec),
                CheckId::TelemetryDeclaration => telemetry_declaration(p),
                CheckId::TelemetrySample => telemetry_sample(p),
                CheckId::DesiredRefused => desired_refused(p, spec),
                CheckId::DesiredBehind => desired_behind(p, spec),
                CheckId::DesiredLastKnown => desired_last_known(p, spec),
                CheckId::RedundancyVacancy => redundancy_vacancy(p),
                CheckId::TimingDeclaration => timing_declaration(p),
                _ => unreachable!("every check that reads presence is above"),
            }
        })
        .collect();
    let unobservable = match (&obs.after, &presence) {
        (Some(_), Err(why)) => Some(why.clone()),
        _ => None,
    };
    DoctorReport {
        scope: scope_of(obs, spec),
        checks,
        unobservable,
    }
}

/// The empty scope's reason: what was read, and that nothing in it was a
/// zk2 token visible to this reader.
fn empty_scope(namespace: &str, after: &Observed) -> String {
    let ns = if namespace.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {namespace:?}")
    };
    let how = after
        .incomplete_reason("ended at its timeout")
        .map(|why| format!(" (and the read {why})"))
        .unwrap_or_default();
    format!(
        "no zk2 token visible to this reader in {ns} (`{}`){how}: a run over an empty scope \
         judged nothing, which is not a healthy deployment",
        after.selector
    )
}

fn scope_of(obs: &DoctorObservation, spec: &DoctorSpec) -> DoctorScope {
    let presence = match &obs.after {
        Some(Ok(after)) => {
            let catalog = Catalog::new(after);
            let instances = after.instances();
            let undescribed = after
                .descriptors
                .iter()
                .flatten()
                .filter(|(_, r)| r.descriptor().is_none())
                .count();
            Asked::Asked(DoctorPresence {
                selector: after.selector.clone(),
                complete: after.complete && !matches!(&obs.before, Some(Ok(b)) if !b.complete),
                grace_s: obs.grace.as_secs_f64(),
                services: catalog.addresses().count(),
                instances: instances.len(),
                tokens: after.tokens.len(),
                undescribed,
                revisions: obs.contracts.len(),
                held: obs
                    .contracts
                    .values()
                    .filter(|r| matches!(r, Ok(ContractState::Held(_))))
                    .count(),
            })
        }
        _ => Asked::NotAsked,
    };
    DoctorScope {
        namespace: obs.namespace.clone(),
        presence,
        routers: match &obs.admin {
            Some(Ok(a)) => Asked::Asked(a.routers.len()),
            _ => Asked::NotAsked,
        },
        health: match (&obs.after, obs.health.is_empty()) {
            (Some(Ok(after)), false) => {
                let catalog = Catalog::new(after);
                Asked::Asked(DoctorHealth {
                    selector: HealthTarget::All.state_selector(),
                    services: catalog
                        .addresses()
                        .filter(|a| catalog.has_instance(a))
                        .filter(|a| matches!(listing(after, a), Listing::Listed { .. }))
                        .count(),
                    clocks_synced: spec.clocks_synced,
                })
            }
            _ => Asked::NotAsked,
        },
        config: match (&obs.after, obs.config.is_empty()) {
            (Some(Ok(after)), false) => {
                let catalog = Catalog::new(after);
                Asked::Asked(DoctorConfig {
                    selector: ConfigTarget::All.state_selector(false),
                    services: catalog
                        .addresses()
                        .filter(|a| catalog.has_instance(a))
                        .filter(|a| {
                            crate::model::config::listing(after, a) == ConfigPremise::Listed
                        })
                        .count(),
                    readings: obs.config.len(),
                    clocks_synced: spec.clocks_synced,
                })
            }
            _ => Asked::NotAsked,
        },
    }
}

/// One instance, as one presence read shows it.
#[derive(Debug, Default)]
struct Inst<'o> {
    instance_token: bool,
    /// Interface tokens, by interface: the revision prefixes they carry.
    alive: BTreeMap<IfaceId, BTreeSet<Fp16>>,
    descriptor: Option<&'o DescriptorRead>,
}

fn index(observed: &Observed) -> BTreeMap<(Addr, InstanceId), Inst<'_>> {
    let mut out: BTreeMap<(Addr, InstanceId), Inst<'_>> = BTreeMap::new();
    for t in &observed.tokens {
        match t {
            ZkKey::Instance { addr, instance } => {
                out.entry((addr.clone(), instance.clone()))
                    .or_default()
                    .instance_token = true;
            }
            ZkKey::Alive {
                addr,
                iface,
                instance,
                fp,
            } => {
                out.entry((addr.clone(), instance.clone()))
                    .or_default()
                    .alive
                    .entry(iface.clone())
                    .or_default()
                    .insert(fp.clone());
            }
            _ => {}
        }
    }
    for (key, read) in observed.descriptors.iter().flatten() {
        out.entry(key.clone()).or_default().descriptor = Some(read);
    }
    out
}

/// The presence half of an observation, indexed once for every check that
/// reads it.
struct Presence<'o> {
    obs: &'o DoctorObservation,
    after: &'o Observed,
    /// The first read, or why there is none to compare with.
    before: Result<&'o Observed, String>,
    now: BTreeMap<(Addr, InstanceId), Inst<'o>>,
    then: BTreeMap<(Addr, InstanceId), Inst<'o>>,
    /// Every read taken ended at the routers' final reply. A first read
    /// not taken — no check that compares two was asked — or one that
    /// failed is not counted here: the checks that need it say so.
    complete: bool,
    /// A read taken was cut off while it ran (core §8.1, 0.38).
    cut_off_during: bool,
}

impl<'o> Presence<'o> {
    fn new(obs: &'o DoctorObservation, after: &'o Observed) -> Presence<'o> {
        let before = match &obs.before {
            Some(Ok(b)) => Ok(b),
            Some(Err(e)) => Err(format!("the first presence read failed: {e}")),
            None => Err("the first presence read was not taken".to_owned()),
        };
        let then = before.as_ref().map(|b| index(b)).unwrap_or_default();
        Presence {
            obs,
            after,
            complete: after.complete && !before.as_ref().is_ok_and(|b| !b.complete),
            cut_off_during: after.cut_off_during || before.as_ref().is_ok_and(|b| b.cut_off_during),
            before,
            now: index(after),
            then,
        }
    }

    fn served(&self) -> impl Iterator<Item = (&'o Addr, &'o InstanceId, &'o Descriptor)> {
        served(self.after)
    }

    fn descriptors(&self) -> Vec<Descriptor> {
        self.served().map(|(_, _, d)| d.clone()).collect()
    }

    /// The instances whose descriptor was asked for and did not read, each
    /// as an [`Unjudged`] for `what`.
    fn undescribed(&self, what: &str) -> Vec<Unjudged> {
        self.now
            .iter()
            .filter_map(|((a, i), inst)| {
                let read = inst.descriptor?;
                read.descriptor().is_none().then(|| Unjudged {
                    subject: format!("{a}@{i}"),
                    reason: format!(
                        "its descriptor did not read ({}): {what}",
                        undescribed_why(read)
                    ),
                })
            })
            .collect()
    }

    /// The revision `(iface, fp)`, or why it cannot be had.
    fn revision(&self, iface: &IfaceId, fp: &Fingerprint) -> Result<&'o Arc<Revision>, String> {
        match self.obs.contracts.get(&(iface.clone(), fp.clone())) {
            Some(Ok(ContractState::Held(r))) => Ok(r),
            Some(Ok(ContractState::Unavailable { .. })) => Err(format!(
                "{iface} {fp} is unavailable: no holder served it verified"
            )),
            Some(Ok(ContractState::Unreadable { reason })) => {
                Err(format!("{iface} {fp} is unreadable: {reason}"))
            }
            Some(Err(e)) => Err(format!("{iface} {fp} could not be retrieved: {e}")),
            None => Err(format!("{iface} {fp} was not retrieved")),
        }
    }

    /// The revision an interface entry names, or why it cannot be had.
    fn entry_revision(&self, e: &InterfaceEntry) -> Result<&'o Arc<Revision>, String> {
        let iface = IfaceId::from_str(&e.iface)
            .map_err(|err| format!("{:?} is not an interface id: {err}", e.iface))?;
        let fp = Fingerprint::parse(&e.contract)
            .map_err(|err| format!("{:?} is not a fingerprint: {err}", e.contract))?;
        self.revision(&iface, &fp)
    }

    fn grace_s(&self) -> f64 {
        self.obs.grace.as_secs_f64()
    }
}

fn undescribed_why(read: &DescriptorRead) -> String {
    match read {
        DescriptorRead::Served(_) => "served".into(),
        DescriptorRead::Invalid(why) => format!("invalid: {why}"),
        DescriptorRead::Silent => "no reply within the timeout".into(),
        DescriptorRead::Failed(why) => format!("the GET failed: {why}"),
    }
}

fn finding(
    check: CheckId,
    severity: DoctorSeverity,
    subject: impl Into<String>,
    evidence: impl Into<String>,
) -> DoctorFinding {
    DoctorFinding {
        severity,
        check,
        subject: subject.into(),
        evidence: evidence.into(),
    }
}

fn unjudged(subject: impl Into<String>, reason: impl Into<String>) -> Unjudged {
    Unjudged {
        subject: subject.into(),
        reason: reason.into(),
    }
}

/// The "possibly incomplete" subject a check adds when a presence read
/// ended at its timeout, or was cut off while it ran (core §8.1, 0.38), and
/// the check claims an absence.
fn incomplete(p: &Presence<'_>, what: &str) -> Unjudged {
    let how = if p.cut_off_during {
        crate::bus::presence::CUT_OFF_DURING
    } else {
        "ended at its timeout"
    };
    unjudged(
        "presence",
        format!("a presence read {how}: it can miss a token, so {what} (§8.1)"),
    )
}

// ─── the checks that read presence ──────────────────────────────────────────

/// §6, through the runtime's own diagnosis ([`zenkey::ownership::compare`])
/// over the two reads, the served descriptors and the held contracts.
fn split_brain(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::SplitBrain;
    let before = match &p.before {
        Ok(b) => *b,
        Err(why) => return CheckReport::unobservable(C, why.clone()),
    };
    let descriptors = p.descriptors();
    let held: Vec<Arc<Contract>> = p
        .obs
        .contracts
        .values()
        .filter_map(|r| match r {
            Ok(ContractState::Held(rev)) => Some(rev.shared_contract()),
            _ => None,
        })
        .collect();
    let refs: Vec<&Contract> = held.iter().map(|c| &**c).collect();
    let mut d = zenkey::ownership::compare(&before.tokens, &p.after.tokens, &descriptors, &refs);
    let candidates = zenkey::ownership::candidates(&before.tokens, &p.after.tokens);
    // redundancy.v1 §2.15 (#841, RC): of a governed interface, a holder is
    // an incumbent whatever its mark or its exposure, so the first
    // question, the model's own judge over the same two reads, decides:
    // its no, `split`, is the finding; core §6's exposure rule decides the
    // rest.
    use zenkey_model::redundancy::{self as rm, Election, Governance};
    let governed: Vec<(zenkey::ownership::SplitBrain, Election, rm::Judged)> = candidates
        .iter()
        .filter_map(|c| {
            let g = crate::model::redundancy::governance(
                &[Some(before), Some(p.after)],
                &c.service,
                &c.iface,
                &p.obs.contracts,
            )
            .governance;
            let Governance::Governed(e) = g else {
                return None;
            };
            let reading = crate::model::redundancy::reading(
                Some(before),
                Some(p.after),
                &c.service,
                &c.iface,
                g,
                false,
            );
            Some((c.clone(), e, rm::one_incumbent(&reading)))
        })
        .collect();
    let is_governed = |svc: &Addr, iface: &IfaceId| {
        governed
            .iter()
            .any(|(c, _, _)| c.service == *svc && c.iface == *iface)
    };
    d.findings.retain(|sb| !is_governed(&sb.service, &sb.iface));
    d.undecided
        .retain(|u| !is_governed(&u.holders.service, &u.holders.iface));
    // The governed ones are the first question's, never core §6's.
    let candidates = candidates.len() - governed.len();
    let mut governed_undecided: Vec<Unjudged> = governed
        .iter()
        .filter(|(_, _, j)| j.answer == rm::Answer::Unobservable)
        .map(|(sb, _, j)| {
            unjudged(
                format!("{} {}", sb.service, sb.iface),
                format!(
                    "redundancy.v1 governs it, and \"is the address served by exactly one \
                     incumbent?\" is unobservable: {} (redundancy.v1 §2.15)",
                    j.reason.as_str()
                ),
            )
        })
        .collect();
    let mut findings: Vec<DoctorFinding> = governed
        .iter()
        .filter(|(_, _, j)| j.answer == rm::Answer::No && j.reason == rm::Reason::Split)
        .map(|(sb, election, _)| {
            let instances: Vec<String> = sb.instances.iter().map(ToString::to_string).collect();
            finding(
                C,
                DoctorSeverity::Error,
                format!("{} {}", sb.service, sb.iface),
                format!(
                    "{} instances hold its interface token in two presence reads {:.1}s apart \
                     ({}), and redundancy.v1 governs it (election {election}): a split, \
                     whatever their marks (redundancy.v1 §2.15). {}",
                    instances.len(),
                    p.grace_s(),
                    instances.join(", "),
                    if *election == Election::Claim {
                        "Its heal leaves the lowest serving; until then a call runs on each side"
                    } else {
                        "Under external nothing heals it: a lock failed, or two hosts run one \
                         address, and a person decides"
                    }
                ),
            )
        })
        .collect();
    let mut exposing: Vec<DoctorFinding> = d
        .findings
        .iter()
        .map(|sb| {
            let instances: Vec<String> = sb.instances.iter().map(ToString::to_string).collect();
            finding(
                C,
                DoctorSeverity::Error,
                format!("{} {}", sb.service, sb.iface),
                format!(
                    "{} instances hold its interface token in two presence reads {:.1}s apart \
                     ({}), and at least two expose an exclusive resource. Nothing is fenced: \
                     exclusivity is redundancy.v1's",
                    instances.len(),
                    p.grace_s(),
                    instances.join(", ")
                ),
            )
        })
        .collect();
    findings.append(&mut exposing);
    let mut undecided: Vec<Unjudged> = d
        .undecided
        .iter()
        .map(|u| {
            unjudged(
                format!("{} {}", u.holders.service, u.holders.iface),
                format!(
                    "held by {} instances, and whether two expose an exclusive resource \
                     cannot be decided: {}",
                    u.holders.instances.len(),
                    u.why.join("; ")
                ),
            )
        })
        .collect();
    undecided.append(&mut governed_undecided);
    if !p.complete {
        undecided.push(incomplete(p, "no split-brain seen is not none"));
    }
    let explained = if candidates > 0 {
        format!(
            "; {candidates} held by several instances in both reads, of which at most one \
             exposes an exclusive resource (replicated serving)"
        )
    } else {
        String::new()
    };
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "no interface held by two instances of one service in both reads {:.1}s apart \
             exposes an exclusive resource{explained}; the tokenless set holds no interface \
             token and is not covered",
            p.grace_s()
        ),
    )
}

/// Whether a role must be bound, as far as can be told.
enum Need {
    Required,
    Optional,
    /// The descriptor does not say: its declaring contract could not be
    /// read, or declares no such role.
    Unknown(String),
}

/// §3.2 R5 and the unbound-role rule: each role's bindings against the
/// providers this reader sees, through the runtime's own edges (R3).
fn binding_unsatisfied(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::BindingUnsatisfied;
    let descriptors = p.descriptors();
    let edges: BTreeSet<(String, String)> = zenkey::presence::edges(&descriptors, &p.after.tokens)
        .into_iter()
        .map(|e| (e.consumer, e.role))
        .collect();
    let undescribed: Vec<Addr> = p
        .now
        .iter()
        .filter(|(_, i)| i.descriptor.is_some_and(|r| r.descriptor().is_none()))
        .map(|((a, _), _)| a.clone())
        .collect();
    let mut findings = Vec::new();
    let mut undecided = p.undescribed("its roles cannot be read");
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let (mut bound, mut unbound) = (0usize, 0usize);
    for (_, _, d) in p.served() {
        for r in &d.requires {
            let key = (d.service.clone(), r.role.clone());
            if !seen.insert(key.clone()) {
                continue;
            }
            let subject = format!("{} {}", d.service, r.role);
            let need = need_of(p, d, r);
            if r.bindings.is_empty() {
                unbound += 1;
                if let Need::Required = need {
                    findings.push(finding(
                        C,
                        DoctorSeverity::Error,
                        subject,
                        format!(
                            "required role {} ({}) is bound to nothing: an owner whose \
                             configuration leaves a required role unbound MUST NOT start (§3.2)",
                            r.role, r.interface
                        ),
                    ));
                }
                continue;
            }
            bound += 1;
            if edges.contains(&key) {
                continue;
            }
            if !p.complete {
                undecided.push(unjudged(
                    subject,
                    if p.cut_off_during {
                        "a presence read was cut off while it ran: a provider may be present and \
                         unseen"
                    } else {
                        "a presence read ended at its timeout: a provider may be present and unseen"
                    },
                ));
                continue;
            }
            let hidden: Vec<String> = undescribed
                .iter()
                .filter(|a| {
                    r.bindings.iter().any(|b| {
                        zenkey::consumer::Provider::parse(b).is_ok_and(|pat| pat.matches(a))
                    })
                })
                .map(ToString::to_string)
                .collect();
            if !hidden.is_empty() {
                undecided.push(unjudged(
                    subject,
                    format!(
                        "{} match its bindings and their descriptors did not read: they may \
                         provide {} tokenlessly",
                        hidden.join(", "),
                        r.interface
                    ),
                ));
                continue;
            }
            let (severity, why) = match &need {
                Need::Required => (DoctorSeverity::Error, "a required role".to_owned()),
                Need::Optional => (
                    DoctorSeverity::Info,
                    "an optional role: the consumer runs without it".to_owned(),
                ),
                Need::Unknown(why) => (
                    DoctorSeverity::Warning,
                    format!("whether it is required cannot be told: {why}"),
                ),
            };
            findings.push(finding(
                C,
                severity,
                subject,
                format!(
                    "{why}; its bindings ({}) select no provider of {} visible to this reader — \
                     no interface token, no descriptor listing it (a read access control \
                     refuses is empty too, §8.1)",
                    r.bindings.join(", "),
                    r.interface
                ),
            ));
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{bound} bound role(s), each selecting a provider present now{}",
            if unbound > 0 {
                format!("; {unbound} optional role(s) left unbound, as §3.2 allows")
            } else {
                String::new()
            }
        ),
    )
}

fn need_of(p: &Presence<'_>, d: &Descriptor, r: &zenkey_model::descriptor::RequireEntry) -> Need {
    let Some(by) = &r.declared_by else {
        // A role the component's manifest declares states its own need
        // (§3.3, 0.10): `optional`, absent being required.
        return if r.optional {
            Need::Optional
        } else {
            Need::Required
        };
    };
    let Some(entry) = d.interfaces.iter().find(|e| &e.iface == by) else {
        return Need::Unknown(format!(
            "it is declared by {by}, which the descriptor does not list"
        ));
    };
    match p.entry_revision(entry) {
        Ok(rev) => match rev.contract().requires.get(&r.role) {
            Some(q) if q.optional => Need::Optional,
            Some(_) => Need::Required,
            None => Need::Unknown(format!("{by}'s contract declares no role {}", r.role)),
        },
        Err(why) => Need::Unknown(why),
    }
}

/// §9.8: every pair of revisions one interface's providers serve, through
/// the contract CI's own classifier.
fn contract_drift(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::ContractDrift;
    let mut by_iface: BTreeMap<IfaceId, BTreeMap<Fingerprint, Vec<String>>> = BTreeMap::new();
    // The minor each revision's descriptors state: an order hint, read only
    // when they agree (see `classify`).
    let mut minors: BTreeMap<Fingerprint, BTreeSet<u64>> = BTreeMap::new();
    for (a, i, d) in p.served() {
        for e in &d.interfaces {
            let (Ok(iface), Ok(fp)) =
                (IfaceId::from_str(&e.iface), Fingerprint::parse(&e.contract))
            else {
                continue;
            };
            minors.entry(fp.clone()).or_default().insert(e.minor);
            by_iface
                .entry(iface)
                .or_default()
                .entry(fp)
                .or_default()
                .push(format!("{a}@{i}"));
        }
    }
    let minor = |fp: &Fingerprint| match minors.get(fp) {
        Some(m) if m.len() == 1 => m.first().copied(),
        _ => None,
    };
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    // A token held by an instance whose descriptor did not read, at a prefix
    // no descriptor names: a revision that exists and cannot be retrieved.
    for ((a, i), inst) in &p.now {
        if inst.descriptor.is_some_and(|r| r.descriptor().is_some()) {
            continue;
        }
        for (iface, fps) in &inst.alive {
            for fp16 in fps {
                let named = by_iface
                    .get(iface)
                    .is_some_and(|m| m.keys().any(|f| f.hex().fp16() == *fp16));
                if !named {
                    undecided.push(unjudged(
                        format!("{iface} {fp16}"),
                        format!(
                            "{a}@{i} holds a token at this prefix and its descriptor did not \
                             read: a revision is retrieved by its full fingerprint, never a \
                             prefix (§8.4)"
                        ),
                    ));
                }
            }
        }
    }
    let mut compatible = 0usize;
    let mut single = 0usize;
    for (iface, revisions) in &by_iface {
        let fps: Vec<&Fingerprint> = revisions.keys().collect();
        if fps.len() == 1 {
            single += 1;
        }
        for (n, a) in fps.iter().enumerate() {
            for b in &fps[n + 1..] {
                let subject = format!("{iface} {} {}", a.hex().fp16(), b.hex().fp16());
                let (ra, rb) = match (p.revision(iface, a), p.revision(iface, b)) {
                    (Ok(ra), Ok(rb)) => (ra, rb),
                    (Err(why), _) | (_, Err(why)) => {
                        undecided.push(unjudged(subject, format!("not classified: {why}")));
                        continue;
                    }
                };
                let served =
                    |fp: &Fingerprint| format!("{fp} (served by {})", revisions[fp].join(", "));
                match classify((ra, minor(a)), (rb, minor(b))) {
                    Drift::Compatible => compatible += 1,
                    Drift::Found { class, why } => findings.push(finding(
                        C,
                        if class == Class::Breaking {
                            DoctorSeverity::Error
                        } else {
                            DoctorSeverity::Warning
                        },
                        subject,
                        format!(
                            "{} and {}: {why} — providers of one interface at revisions a \
                             consumer cannot bind to alike (R4)",
                            served(a),
                            served(b)
                        ),
                    )),
                    Drift::Unordered(why) => undecided.push(unjudged(subject, why)),
                }
            }
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{} interface(s): {single} served at one revision{}",
            by_iface.len(),
            if compatible > 0 {
                format!(", and {compatible} pair(s) of revisions the classifier calls compatible")
            } else {
                String::new()
            }
        ),
    )
}

/// How a pair of revisions served at once classifies.
enum Drift {
    Compatible,
    Found {
        class: Class,
        why: String,
    },
    /// The class depends on which revision came first, and nothing says.
    Unordered(String),
}

/// One pair through the classifier (§9.8). It classifies an earlier
/// revision against a later one, never the reverse — its rules already
/// cover an old reader of a new writer and the other way round — so the
/// pair needs an order. The bus carries none that binds: the `minor` the
/// descriptors state for each revision (§3.3), informative and outside the
/// fingerprint, is read as a hint when every descriptor naming a revision
/// agrees on it and the two differ. Otherwise both orders are classified:
/// compatible either way is clean, review or breaking either way is the
/// finding (the milder class), and a disagreement is left unjudged,
/// because the answer is the order's.
fn classify((a, ma): (&Revision, Option<u64>), (b, mb): (&Revision, Option<u64>)) -> Drift {
    use zenkey_model::compat::{Revision as Compared, compare};
    let (ca, cb) = (
        Compared::of_bundle(a.bundle()),
        Compared::of_bundle(b.bundle()),
    );
    let say = |earlier: &Revision, later: &Revision, v: &zenkey_model::compat::Verdict| {
        let class = v.class();
        let first = v
            .findings
            .iter()
            .find(|f| f.class == class)
            .map(|f| format!(" ({} at {}: {})", f.rule, f.at, f.detail))
            .unwrap_or_default();
        let more = match v.findings.len() {
            0 | 1 => String::new(),
            n => format!(" and {} more change(s)", n - 1),
        };
        format!(
            "{} → {} is {}{first}{more}",
            earlier.fingerprint().hex().fp16(),
            later.fingerprint().hex().fp16(),
            class.as_str()
        )
    };
    let ordered = match (ma, mb) {
        (Some(x), Some(y)) if x < y => Some((a, &ca, x, b, &cb, y)),
        (Some(x), Some(y)) if y < x => Some((b, &cb, y, a, &ca, x)),
        _ => None,
    };
    if let Some((e, ce, me, l, cl, ml)) = ordered {
        let v = compare(ce, cl);
        return match v.class() {
            Class::Compatible => Drift::Compatible,
            class => Drift::Found {
                class,
                why: format!(
                    "{}, ordered by the minors their descriptors state ({me} < {ml})",
                    say(e, l, &v)
                ),
            },
        };
    }
    let (ab, ba) = (compare(&ca, &cb), compare(&cb, &ca));
    match (ab.class(), ba.class()) {
        (Class::Compatible, Class::Compatible) => Drift::Compatible,
        (x, y) if x != Class::Compatible && y != Class::Compatible => Drift::Found {
            class: x.min(y),
            why: format!(
                "{}; {} — whichever came first",
                say(a, b, &ab),
                say(b, a, &ba)
            ),
        },
        _ => Drift::Unordered(format!(
            "the class depends on which revision came first, and their minors do not say: \
             {}; {}",
            say(a, b, &ab),
            say(b, a, &ba)
        )),
    }
}

/// §8.4: every revision a descriptor names, as retrieval found it.
fn contract_unavailable(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::ContractUnavailable;
    let mut named: BTreeMap<(IfaceId, Fingerprint), Vec<String>> = BTreeMap::new();
    for (a, i, d) in p.served() {
        for e in &d.interfaces {
            if let (Ok(iface), Ok(fp)) =
                (IfaceId::from_str(&e.iface), Fingerprint::parse(&e.contract))
            {
                named
                    .entry((iface, fp))
                    .or_default()
                    .push(format!("{a}@{i}"));
            }
        }
    }
    let mut findings = Vec::new();
    let mut undecided = p.undescribed("the revisions it implements are unknown");
    for ((iface, fp), by) in &named {
        let subject = format!("{iface} {fp}");
        match p.obs.contracts.get(&(iface.clone(), fp.clone())) {
            Some(Ok(ContractState::Held(_))) => {}
            Some(Ok(ContractState::Unavailable { refused })) => findings.push(finding(
                C,
                DoctorSeverity::Error,
                subject,
                format!(
                    "named by {}; no holder served a bundle that verified ({}): an owner MUST \
                     hold the bundle of every interface it implements (§8.2), and a tool never \
                     decodes with an unverified one",
                    by.join(", "),
                    if refused.is_empty() {
                        "no reply".to_owned()
                    } else {
                        format!("refused: {}", refused.join(", "))
                    }
                ),
            )),
            Some(Ok(ContractState::Unreadable { reason })) => findings.push(finding(
                C,
                DoctorSeverity::Error,
                subject,
                format!(
                    "named by {}; a bundle verified and its contract does not read: {reason}",
                    by.join(", ")
                ),
            )),
            Some(Err(e)) => undecided.push(unjudged(
                subject,
                format!("the retrieval could not be put on the bus: {e}"),
            )),
            None => undecided.push(unjudged(subject, "not retrieved")),
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{} revision(s) named by descriptors, each retrieved from a holder and verified",
            named.len()
        ),
    )
}

/// §3.3: every descriptor through `zenkey_model::descriptor::check`, with
/// the contracts its entries name.
///
/// The presence read already checked each reply without contracts, which
/// is every code but D004–D007: a reply that failed is `Invalid`, with its
/// codes. A served one is checked again here against the revisions it
/// names, as the runtime checks its own before serving it — re-encoded
/// from the parsed record, which the first check proved the reply's shape.
fn descriptor_invalid(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::DescriptorInvalid;
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let mut checked = 0usize;
    for ((a, i), inst) in &p.now {
        let subject = format!("{a}@{i}");
        let Some(read) = inst.descriptor else {
            continue;
        };
        let d = match read {
            DescriptorRead::Served(d) => d,
            DescriptorRead::Invalid(why) => {
                findings.push(finding(C, DoctorSeverity::Error, subject, why.clone()));
                continue;
            }
            other => {
                undecided.push(unjudged(
                    subject,
                    format!("{}: silence is not a verdict", undescribed_why(other)),
                ));
                continue;
            }
        };
        checked += 1;
        let mut contracts: Vec<&Contract> = Vec::new();
        let mut syntax_only = Vec::new();
        for e in &d.interfaces {
            match p.entry_revision(e) {
                Ok(rev) => contracts.push(rev.contract()),
                Err(why) => syntax_only.push(why),
            }
        }
        let text = serde_json::to_string(&**d).expect("a descriptor serializes");
        let (_, report) = zenkey_model::descriptor::check(&text, &contracts);
        let codes: Vec<String> = report
            .errors()
            .chain(report.warnings())
            .map(|f| format!("{} {}: {}", f.code, f.at, f.message))
            .collect();
        if !codes.is_empty() {
            let severity = if report.has_errors() {
                DoctorSeverity::Error
            } else {
                DoctorSeverity::Warning
            };
            findings.push(finding(C, severity, subject, codes.join("; ")));
        } else if !syntax_only.is_empty() {
            undecided.push(unjudged(
                subject,
                format!(
                    "checked for syntax only, D004–D007 need the contract: {}",
                    syntax_only.join("; ")
                ),
            ));
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{checked} descriptor(s) pass the descriptor check against the contracts they name"
        ),
    )
}

/// §8.1: each instance's tokens against its descriptor. A finding must
/// hold in both reads: start-up declares the instance token before the
/// interface tokens, and a capability lost undeclares a token as the
/// descriptor changes.
fn token_missing(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::TokenMissing;
    if let Err(why) = &p.before {
        return CheckReport::unobservable(C, why.clone());
    }
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let mut checked = 0usize;
    for (key, now) in &p.now {
        let (a, i) = key;
        let subject = format!("{a}@{i}");
        let Some(then) = p.then.get(key) else {
            undecided.push(unjudged(
                subject,
                format!(
                    "it appeared within the {:.1}s grace period: a token's absence is judged \
                     only when it lasts",
                    p.grace_s()
                ),
            ));
            continue;
        };
        if !now.instance_token && !then.instance_token && !now.alive.is_empty() {
            if p.complete {
                findings.push(finding(
                    C,
                    DoctorSeverity::Error,
                    subject.clone(),
                    "holds interface tokens and no instance token, in both reads: every \
                     instance MUST hold one",
                ));
            } else {
                undecided.push(incomplete(p, "an instance token unseen may be held"));
            }
        }
        let d = match now.descriptor {
            Some(DescriptorRead::Served(d)) => d,
            Some(other) => {
                undecided.push(unjudged(
                    subject,
                    format!(
                        "its descriptor did not read ({}): its tokens cannot be checked \
                         against it",
                        undescribed_why(other)
                    ),
                ));
                continue;
            }
            None => continue,
        };
        checked += 1;
        let listed: BTreeMap<&str, &InterfaceEntry> =
            d.interfaces.iter().map(|e| (e.iface.as_str(), e)).collect();
        // Tokens the descriptor does not account for.
        for (iface, fps) in &now.alive {
            let at = format!("{subject} {iface}");
            let held_then = then.alive.contains_key(iface);
            match listed.get(iface.to_string().as_str()) {
                None if held_then => findings.push(finding(
                    C,
                    DoctorSeverity::Error,
                    at,
                    format!("holds a token for {iface}, which its descriptor does not list"),
                )),
                Some(e) if !e.token && held_then => findings.push(finding(
                    C,
                    DoctorSeverity::Error,
                    at,
                    format!(
                        "holds a token for {iface}, which its descriptor marks tokenless \
                         (`\"token\": false`)"
                    ),
                )),
                Some(e) => {
                    let named = Fingerprint::parse(&e.contract).ok().map(|f| f.hex().fp16());
                    let other: Vec<String> = fps
                        .iter()
                        .filter(|f| Some(*f) != named.as_ref())
                        .map(ToString::to_string)
                        .collect();
                    if e.token && !other.is_empty() && held_then {
                        findings.push(finding(
                            C,
                            DoctorSeverity::Error,
                            at,
                            format!(
                                "its token names revision prefix {}, and its descriptor lists \
                                 {iface} at {}",
                                other.join(", "),
                                e.contract
                            ),
                        ));
                    }
                }
                None => {}
            }
        }
        // Interfaces the descriptor lists, outside the tokenless set.
        for e in d.interfaces.iter().filter(|e| e.token) {
            let Ok(iface) = IfaceId::from_str(&e.iface) else {
                continue;
            };
            let at = format!("{subject} {iface}");
            let held = now.alive.contains_key(&iface);
            let held_then = then.alive.contains_key(&iface);
            let exposed = match p.entry_revision(e) {
                Ok(rev) => d.exposed(rev.contract()).map(|r| r.len()),
                Err(why) => {
                    if !held {
                        undecided.push(unjudged(
                            at,
                            format!("whether it exposes a resource needs its contract: {why}"),
                        ));
                    }
                    continue;
                }
            };
            match (exposed, held) {
                (Some(0), true) if held_then && e.standby => findings.push(finding(
                    C,
                    DoctorSeverity::Error,
                    at,
                    format!(
                        "holds a token for {iface}, which its descriptor marks standby: a \
                         standby exposes nothing and holds none of its tokens (core §6, \
                         redundancy.v1 §2.15, marked_holding)"
                    ),
                )),
                (Some(0), true) if held_then => findings.push(finding(
                    C,
                    DoctorSeverity::Error,
                    at,
                    format!(
                        "holds a token for {iface} and exposes none of its resources: an \
                         instance holds one per interface of which it exposes a resource, and \
                         none otherwise"
                    ),
                )),
                (Some(n), false) if n > 0 && !held_then => {
                    if p.complete {
                        findings.push(finding(
                            C,
                            DoctorSeverity::Error,
                            at,
                            format!(
                                "exposes {n} resource(s) of {iface}, outside the tokenless set, \
                                 and holds no interface token for it in two reads {:.1}s apart",
                                p.grace_s()
                            ),
                        ));
                    } else {
                        undecided.push(incomplete(p, "a token unseen may be held"));
                    }
                }
                _ => {}
            }
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{checked} instance(s): every token agrees with its descriptor, and each interface \
             exposed outside the tokenless set holds its token"
        ),
    )
}

/// §4.4: each archive's keys, as their `confirmed` flags say.
fn archive_unaligned(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::ArchiveUnaligned;
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let (mut values, mut tombstones) = (0usize, 0usize);
    for (addr, read) in &p.obs.archives {
        let subject = addr.to_string();
        let keys = match read {
            Ok(k) => k,
            Err(e) => {
                undecided.push(unjudged(
                    subject,
                    format!("its keys could not be read: {e}"),
                ));
                continue;
            }
        };
        values += keys.values;
        tombstones += keys.tombstones;
        if keys.unconfirmed > 0 {
            findings.push(finding(
                C,
                DoctorSeverity::Warning,
                subject,
                format!(
                    "{} of {} key(s) served `confirmed: false` (e.g. {}): no alignment has \
                     confirmed them by re-reading their owner or a peer archive, and a \
                     consumer reads them as last-known and unconfirmed",
                    keys.unconfirmed,
                    keys.values,
                    keys.examples.join(", ")
                ),
            ));
        } else if !keys.complete {
            undecided.push(unjudged(
                subject,
                "the read ended at its timeout: a key unseen may be unconfirmed",
            ));
        }
    }
    if p.obs.archives.is_empty() && !p.complete {
        undecided.push(incomplete(p, "an archive unseen may be present"));
    }
    let clean = if p.obs.archives.is_empty() {
        "no archive.v1 provider visible to this reader: nothing to align".to_owned()
    } else {
        format!(
            "{} archive(s) serve {values} value(s), every one confirmed, and {tombstones} \
             tombstone(s)",
            p.obs.archives.len()
        )
    };
    CheckReport::of(C, findings, undecided, clean)
}

/// §4.2 S1–S2: each owner's state replies, by the clock that stamped them,
/// against the owner's own session — the `meta.zid` its descriptors name.
fn state_stamp_foreign(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::StateStampForeign;
    let mut zids: BTreeMap<&Addr, BTreeSet<String>> = BTreeMap::new();
    for (a, _, d) in p.served() {
        if let Some(z) = d.meta.get("zid").and_then(|v| v.as_str()) {
            zids.entry(a).or_default().insert(zid_value(z));
        }
    }
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let (mut replies, mut owners) = (0usize, 0usize);
    for ((addr, iface), read) in &p.obs.stamps {
        let subject = format!("{addr} {iface}");
        let stamps = match read {
            Ok(s) => s,
            Err(e) => {
                undecided.push(unjudged(
                    subject,
                    format!("its state could not be read: {e}"),
                ));
                continue;
            }
        };
        let n: usize = stamps.by_clock.values().map(|(n, _)| n).sum();
        if n == 0 {
            if !stamps.complete {
                undecided.push(unjudged(subject, "the GET ended at its timeout"));
            }
            continue;
        }
        let Some(own) = zids.get(addr) else {
            undecided.push(unjudged(
                subject,
                format!(
                    "no descriptor of {addr} names its session's zid (`meta.zid`): whose clock \
                     is the owner's cannot be told"
                ),
            ));
            continue;
        };
        let mut wrong = Vec::new();
        for (clock, (count, keys)) in &stamps.by_clock {
            match clock {
                None => wrong.push(format!(
                    "{count} unstamped (e.g. {}): a reply MUST carry its mutation's timestamp",
                    keys.join(", ")
                )),
                Some(c) if !own.contains(&zid_value(c)) => wrong.push(format!(
                    "{count} stamped by clock {c} (e.g. {}), not the owner's session",
                    keys.join(", ")
                )),
                Some(_) => {}
            }
        }
        // §4.2 "A tool's S1 check" (0.16, 0.17): a foreign stamp is a
        // finding whatever else was read, since an owner that is its own
        // router stamps with its own `meta.zid`.
        if !wrong.is_empty() {
            findings.push(finding(
                C,
                DoctorSeverity::Error,
                subject,
                format!(
                    "of {n} state repl(y|ies), {}; the owner's session is {}",
                    wrong.join("; "),
                    own.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            ));
            continue;
        }
        if let Err(why) = s1_premise(p.obs.admin.as_ref(), own) {
            undecided.push(unjudged(subject, why));
        } else if !stamps.complete {
            undecided.push(unjudged(
                subject,
                "the GET ended at its timeout: a reply unseen may be stamped elsewhere",
            ));
        } else {
            owners += 1;
            replies += n;
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        if replies == 0 {
            "no owner answered a state GET with a value: no stamp to attribute".to_owned()
        } else {
            format!(
                "{replies} state repl(y|ies) from {owners} owner interface(s), each stamped by \
                 its owner's own session"
            )
        },
    )
}

/// §2.7 (0.24; #735): each owner interface's templated state resources,
/// counted from the GET `state-stamp-foreign` reads — every key answered
/// with a value — against the bound its descriptors state, else its
/// contract's. More members than the bound is the finding, whether or not
/// the GET ran to its final reply; within it from a GET that did not is
/// unjudged. A resource the GET answered no member of adds nothing (an
/// empty reply set is never a verdict, O5), as an owner that answered no
/// state adds nothing to `state-stamp-foreign`; the evidence counts them.
/// Streams and events are not read here: the doctor listens to no window,
/// and `check conform` counts them.
fn population_over_bound(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::PopulationOverBound;
    use zenkey_model::budget::{self, Bound, Reading, Verdict};
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let (mut judged, mut members, mut empty, mut unbounded) = (0usize, 0u64, 0usize, 0usize);
    for ((addr, iface), read) in &p.obs.stamps {
        let subject = format!("{addr} {iface}");
        let stamps = match read {
            Ok(s) => s,
            Err(e) => {
                undecided.push(unjudged(
                    subject,
                    format!("its state could not be read: {e}"),
                ));
                continue;
            }
        };
        let want = iface.to_string();
        let entries: Vec<(&Descriptor, &InterfaceEntry)> = p
            .served()
            .filter(|(a, _, _)| *a == addr)
            .filter_map(|(_, _, d)| Some((d, d.interfaces.iter().find(|e| e.iface == want)?)))
            .collect();
        let fps: BTreeSet<&str> = entries.iter().map(|(_, e)| e.contract.as_str()).collect();
        if fps.len() > 1 {
            undecided.push(unjudged(
                subject,
                format!(
                    "its instances claim {} revisions of {iface}: whose bound applies to one \
                     population cannot be told",
                    fps.len()
                ),
            ));
            continue;
        }
        let Some((_, entry)) = entries.first() else {
            continue;
        };
        let rev = match p.entry_revision(entry) {
            Ok(r) => r,
            Err(why) => {
                undecided.push(unjudged(subject, why));
                continue;
            }
        };
        let contract = rev.contract();
        // Every key answered with a value, resolved to its template (§2.2).
        let mut counted: BTreeMap<usize, u64> = BTreeMap::new();
        for key in &stamps.values {
            let Ok(ZkKey::Data { kind, resource, .. }) = zenkey_model::grammar::parse(key) else {
                continue;
            };
            let candidates: Vec<(usize, &zenkey_model::contract::Resource)> = contract
                .resources
                .iter()
                .enumerate()
                .filter(|(_, r)| r.token == kind)
                .collect();
            let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
            if let Some((i, _)) =
                zenkey_model::template::resolve(candidates.iter().map(|(_, r)| &r.template), &refs)
            {
                *counted.entry(candidates[i].0).or_default() += 1;
            }
        }
        for (i, r) in contract.resources.iter().enumerate() {
            if r.kind != zenkey_model::authoring::Kind::State || !r.template.has_params() {
                continue;
            }
            let name = zenkey::implementation::resource_name(r);
            let exposing: Vec<Option<u64>> = entries
                .iter()
                .filter(|(d, _)| {
                    d.exposed(contract)
                        .is_some_and(|rs| rs.iter().any(|x| std::ptr::eq(*x, r)))
                })
                .map(|(_, e)| e.cardinality.get(&name).copied())
                .collect();
            if exposing.is_empty() {
                continue;
            }
            let b = budget::bound_of(r, exposing);
            let n = counted.get(&i).copied().unwrap_or(0);
            let j = budget::population(
                r.kind,
                b,
                None,
                &Reading::Get {
                    members: n,
                    complete: stamps.complete,
                },
            );
            let of = match (b, r.cardinality) {
                (Bound::Of(b), Some(c)) if b < c => {
                    format!("its bound of {b} (its descriptor's; the contract's is {c})")
                }
                (b, _) => format!("its bound of {b} (the contract's)"),
            };
            match j.verdict {
                Verdict::Exceeds => findings.push(finding(
                    C,
                    DoctorSeverity::Error,
                    format!("{subject} {name}"),
                    format!(
                        "{n} member(s) answered with a value, above {of}: an owner MUST NOT \
                         hold more live members than its bound (§2.7){}",
                        if stamps.complete {
                            ""
                        } else {
                            "; the GET ended at its timeout, so this is a lower bound"
                        }
                    ),
                )),
                Verdict::Within => {
                    judged += 1;
                    members += n;
                }
                Verdict::NotAsked => unbounded += 1,
                Verdict::Unobservable if n == 0 => empty += 1,
                Verdict::Unobservable => undecided.push(unjudged(
                    format!("{subject} {name}"),
                    format!(
                        "{n} member(s) within {of}, and the GET ended at its timeout: a member \
                         unanswered may be live (§2.7)"
                    ),
                )),
            }
        }
    }
    let mut clean = format!(
        "{members} member(s) of {judged} templated state resource(s), each within its bound, \
         by GETs that ran to their final reply"
    );
    if empty > 0 {
        clean.push_str(&format!(
            "; {empty} resource(s) answered no member, which is no pass for them (O5)"
        ));
    }
    if unbounded > 0 {
        clean.push_str(&format!(
            "; {unbounded} with the no-ceiling bound, not asked"
        ));
    }
    clean.push_str("; streams and events are check conform's");
    CheckReport::of(C, findings, undecided, clean)
}

// ─── health.v1 (#721, PF) ───────────────────────────────────────────────────

/// One service the `health-*` checks judge: present in the second read, its
/// descriptor listing `health.v1` (§2.7).
struct HealthSubject {
    addr: Addr,
    /// "Is this service healthy?", by the second reading (§2.11).
    last: zenkey_model::health::Judged,
    /// "Does its status agree with its checks?", over both readings (§2.2).
    agreement: HealthAnswer,
    /// The reason its owner gave with the status, as read.
    said: Option<String>,
}

/// What the `health-*` checks read: the subjects, the services they could
/// not decide, and how many present services do not list `health.v1`.
struct HealthRead {
    subjects: Vec<HealthSubject>,
    undecided: Vec<Unjudged>,
    unlisted: usize,
}

/// Every present service's `health.v1` readings, as `zenctl health` builds
/// them ([`crate::model::health::reading`]): presence from the instance
/// token and the descriptor's listing, never the interface token (§2.7);
/// the status's freshness from the GET's reply alone, aged on the
/// deployment's word ([`DoctorSpec::clocks_synced`]) or not at all, since
/// the doctor listens to no status; the checks of the same answer (§2.4).
/// The first reading counts for a service whose instance token the first
/// presence read held.
fn health_read(p: &Presence<'_>, spec: &DoctorSpec) -> HealthRead {
    let trust = if spec.clocks_synced {
        Trust::Word
    } else {
        Trust::None
    };
    let (first, second) = match p.obs.health.as_slice() {
        [first, second] => (Some(first), second),
        [second] => (None, second),
        _ => {
            return HealthRead {
                subjects: Vec::new(),
                undecided: vec![unjudged(
                    "health.v1",
                    "no reading of the owners' health.v1/state/** was taken",
                )],
                unlisted: 0,
            };
        }
    };
    let catalog = Catalog::new(p.after);
    let before = p.before.as_ref().ok().map(|b| Catalog::new(b));
    let mut out = HealthRead {
        subjects: Vec::new(),
        undecided: Vec::new(),
        unlisted: 0,
    };
    for addr in catalog.addresses().filter(|a| catalog.has_instance(a)) {
        let listed = listing(p.after, addr);
        match listed {
            Listing::NotListed => {
                out.unlisted += 1;
                continue;
            }
            Listing::Unread => {
                out.undecided.push(unjudged(
                    addr.to_string(),
                    "no descriptor of it read: whether it implements health.v1 cannot be told, \
                     and its interface token says nothing (health.v1 §2.7)",
                ));
                continue;
            }
            Listing::Listed { .. } => {}
        }
        let second = match second {
            Ok(g) => g,
            Err(e) => {
                out.undecided.push(unjudged(
                    addr.to_string(),
                    format!("the reading's GET could not be made: {e}"),
                ));
                continue;
            }
        };
        let presence = HealthPresence::Present(listed);
        let last = reading(presence, Got::of(Some(second), addr), None, &trust, false);
        let mut agreements = Vec::new();
        if let (Some(first), Some(before)) = (first, &before)
            && before.has_instance(addr)
        {
            let got = Got::of(first.as_ref().ok(), addr);
            agreements.push(reading(presence, got, None, &trust, false).agreement());
        }
        agreements.push(last.agreement());
        out.subjects.push(HealthSubject {
            addr: addr.clone(),
            last: last.judged(),
            agreement: agreement_over(&agreements),
            said: second
                .services
                .get(addr)
                .and_then(|g| g.status.as_ref())
                .and_then(|r| r.value.as_ref())
                .map(|v| v.reason.clone()),
        });
    }
    if !p.complete {
        out.undecided
            .push(incomplete(p, "a service unseen may implement health.v1"));
    }
    out
}

/// Why a subject's verdict is not this check's to decide, with the cure
/// for an untrusted clock.
fn health_unjudged(s: &HealthSubject, what: &str) -> Unjudged {
    let hint =
        if s.last.reason == Reason::Freshness(zenkey_model::freshness::Reason::ClockUntrusted) {
            " — the doctor measures no clock: pass --clocks-synced on the deployment's word that \
         this host's clock and the owners' agree (freshness.v1 §2.6)"
        } else {
            ""
        };
    unjudged(
        s.addr.to_string(),
        format!(
            "{}: {} ({}){hint}",
            what,
            s.last.reason.says(),
            s.last.reason.as_str()
        ),
    )
}

// ─── alarms.v1 (#800, AC) ───────────────────────────────────────────────────

/// Every owner's alarms as the run's GET read them, beside the second
/// presence read: a reader's book with no subscription (§2.14 with its GET
/// alone), the reply's stamp aged on the operator's word only.
fn alarm_book(
    p: &Presence<'_>,
    spec: &DoctorSpec,
) -> std::result::Result<(crate::model::alarms::AlarmBook, bool), String> {
    use crate::model::alarms::{AlarmBook, AlarmScope, BookSpec};
    let read = match &p.obs.alarms {
        None => return Err("no GET of the owners' alarms.v1/state/alarms/* was made".into()),
        Some(Err(e)) => {
            return Err(format!(
                "the GET of the owners' alarms could not be made: {e}"
            ));
        }
        Some(Ok(r)) => r,
    };
    let mut bs = BookSpec::new(spec.timeout);
    bs.clocks_synced = spec.clocks_synced;
    let mut book = AlarmBook::new(AlarmScope::all(), bs);
    let (now, wall) = (std::time::Instant::now(), std::time::SystemTime::now());
    book.presence_read(p.after, now);
    book.seed(read.clone(), now, wall);
    Ok((book, read.complete))
}

/// The alarms' GET ended at its timeout or with an error reply: a reply it
/// did not bring back is unjudged (core §2.7).
fn get_incomplete(what: &str) -> Unjudged {
    unjudged(
        crate::model::alarms::ALL,
        format!("the GET ended at its timeout or with an error reply, so {what}"),
    )
}

/// How many present owners' descriptors list `alarms.v1`, said beside a
/// clean verdict.
fn alarm_listing(book: &crate::model::alarms::AlarmBook) -> usize {
    book.owners()
        .filter(|a| {
            matches!(
                book.presence_of(a),
                zenkey_model::alarms::Presence::Present(
                    zenkey_model::alarms::Listing::Listed { .. }
                )
            )
        })
        .count()
}

/// `alarm-key` (`alarms.v1` §2.2, §2.3, §5): an alarm key that does not
/// re-derive from its document — a phantom of another derivation, of a
/// renamed rule, or an owner that hashed a host-scoped label — is an error;
/// one that re-derives only through the sanitizing step is a warning, the
/// SHOULD of §2.3 broken. A put that does not read re-derives nothing:
/// unjudged. A finding stands on an incomplete GET; clean needs a complete
/// one.
fn alarm_key(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    const C: CheckId = CheckId::AlarmKey;
    let (book, complete) = match alarm_book(p, spec) {
        Ok(b) => b,
        Err(why) => return CheckReport::unobservable(C, why),
    };
    let (now, wall) = (std::time::Instant::now(), std::time::SystemTime::now());
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let mut strict = 0usize;
    for key in book.keys() {
        let Some(r) = book.reading(key, now, wall) else {
            continue;
        };
        let j = r.rederives();
        let derived = book
            .seen(key)
            .and_then(|s| s.document)
            .map(|d| d.alert_key())
            .unwrap_or_default();
        match (j.answer, j.reason) {
            (zenkey_model::alarms::Answer::No, _) => findings.push(finding(
                C,
                DoctorSeverity::Error,
                key,
                format!(
                    "the key is not its document's: the document derives {derived} — a phantom \
                     of another derivation, of a renamed rule, or an owner that hashed a \
                     host-scoped label (alarms.v1 §2.2, §2.9)"
                ),
            )),
            (zenkey_model::alarms::Answer::Yes, zenkey_model::alarms::Reason::Sanitized) => {
                findings.push(finding(
                    C,
                    DoctorSeverity::Warning,
                    key,
                    "the key is its document's only through the sanitizing step: the owner puts \
                     input that is not framed, which alarms.v1 §2.3 says it SHOULD NOT",
                ))
            }
            (zenkey_model::alarms::Answer::Yes, _) => strict += 1,
            (zenkey_model::alarms::Answer::Unobservable, _) => undecided.push(unjudged(
                key,
                format!("{} ({})", j.reason.says(), j.reason.as_str()),
            )),
            (zenkey_model::alarms::Answer::NotAsked, _) => {}
        }
    }
    if !complete && findings.is_empty() {
        undecided.push(get_incomplete(
            "an alarm the GET did not bring back may not re-derive",
        ));
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{strict} alarm key(s), each its document's by the strict recipe (alarms.v1 §2.2); \
             {} present service(s) list alarms.v1",
            alarm_listing(&book)
        ),
    )
}

/// `alarm-stale` (`alarms.v1` §2.7, §2.14, §5): a firing alarm of a present,
/// listed owner, its latest put the present instance's, not confirmed
/// within 900 s — about the owner, which broke §2.7's confirmation or whose
/// clock is ahead, which the alarm alone does not tell apart. A warning, as
/// `health-stale` is. A reply is aged only on the operator's word
/// (`--clocks-synced`): the doctor listens to no alarm long enough to
/// measure a clock.
fn alarm_stale(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    const C: CheckId = CheckId::AlarmStale;
    let (book, complete) = match alarm_book(p, spec) {
        Ok(b) => b,
        Err(why) => return CheckReport::unobservable(C, why),
    };
    let (now, wall) = (std::time::Instant::now(), std::time::SystemTime::now());
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let mut fresh = 0usize;
    for key in book.keys() {
        let Some(r) = book.reading(key, now, wall) else {
            continue;
        };
        let j = r.stale();
        match j.answer {
            zenkey_model::alarms::Answer::Yes => findings.push(finding(
                C,
                DoctorSeverity::Warning,
                key,
                "its owner holds its instance token and lists alarms.v1, and the alarm fires \
                 unconfirmed beyond its 900 s horizon: stale, never firing now and never \
                 resolved; the cause (its owner hung, its writer closed, its clock ahead) is \
                 unobservable from the alarm alone (alarms.v1 §2.7, §5)",
            )),
            zenkey_model::alarms::Answer::No => fresh += 1,
            zenkey_model::alarms::Answer::Unobservable => {
                let hint = if j.reason.as_str() == "clock_untrusted" {
                    "; a reply is aged only on the operator's word that the clocks agree \
                     (--clocks-synced, freshness.v1 §2.6)"
                } else {
                    ""
                };
                undecided.push(unjudged(
                    key,
                    format!("{} ({}){hint}", j.reason.says(), j.reason.as_str()),
                ));
            }
            zenkey_model::alarms::Answer::NotAsked => {}
        }
    }
    if !complete && findings.is_empty() {
        undecided.push(get_incomplete(
            "an alarm the GET did not bring back may be stale",
        ));
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{fresh} firing alarm(s), each confirmed within its 900 s horizon; {} present \
             service(s) list alarms.v1",
            alarm_listing(&book)
        ),
    )
}

/// The clean reason's tail: the services not asked.
fn unlisted_note(n: usize) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("; {n} present service(s) do not list health.v1, and are not asked")
    }
}

/// `health-failed` and `health-degraded` (§2.1, §2.11): a present service
/// read unhealthy at `level` — by its fresh status, by a current check worse
/// than that status (the effective level, §2.2), or by a check under a
/// status at an unknown level. FAILED is an error: the owner says it cannot
/// do its primary job. DEGRADED is a warning: it serves with reduced
/// function, which a person should look at. A stale status is never a
/// level (§2.4): its current level is unknown, so it is not decided here.
fn health_level(p: &Presence<'_>, spec: &DoctorSpec, level: Level) -> CheckReport {
    let (check, severity) = match level {
        Level::Failed => (CheckId::HealthFailed, DoctorSeverity::Error),
        Level::Degraded => (CheckId::HealthDegraded, DoctorSeverity::Warning),
        Level::Ok => unreachable!("OK is no finding"),
    };
    let h = health_read(p, spec);
    let mut findings = Vec::new();
    let mut undecided = h.undecided;
    let mut fresh = 0usize;
    for s in &h.subjects {
        match (s.last.verdict, s.last.level) {
            (Verdict::Unhealthy, Some(l)) if l == level => {
                let why = match s.last.reason {
                    Reason::Inconsistent => format!(
                        "a current check is {level} and its fresh status is better: read at the \
                         effective level, {level} (health.v1 §2.2, §2.11)"
                    ),
                    Reason::Check => format!(
                        "its status's level is unknown and a current check is {level}: a no can \
                         rest on a check (health.v1 §2.11)"
                    ),
                    _ => format!(
                        "its status is {level}, fresh: {}",
                        if level == Level::Failed {
                            "present, and unable to do its primary job, as its owner says \
                             (health.v1 §2.1)"
                        } else {
                            "it serves with reduced function, as its owner says (health.v1 §2.1)"
                        }
                    ),
                };
                let said = s
                    .said
                    .as_deref()
                    .filter(|r| !r.is_empty())
                    .map(|r| format!(" — its reason: {r:?}"))
                    .unwrap_or_default();
                findings.push(finding(
                    check,
                    severity,
                    s.addr.to_string(),
                    format!("{why}{said}"),
                ));
            }
            (Verdict::Healthy | Verdict::Unhealthy, _) => fresh += 1,
            (Verdict::Stale, _) => undecided.push(health_unjudged(
                s,
                "its status is stale, never a level, so its current level is unknown",
            )),
            (Verdict::Unobservable, _) => {
                undecided.push(health_unjudged(s, "its health could not be read"))
            }
            (Verdict::NotAsked, _) => {}
        }
    }
    let clean = if h.subjects.is_empty() && undecided.is_empty() {
        format!(
            "no present service's descriptor lists health.v1: health is asked of none (health.v1 \
             §2.7){}",
            unlisted_note(h.unlisted)
        )
    } else {
        format!(
            "{fresh} service(s) implementing health.v1, none at {level}: each fresh status, and \
             the checks it vouches for, read better{}",
            unlisted_note(h.unlisted)
        )
    };
    CheckReport::of(check, findings, undecided, clean)
}

/// `health-stale` (§2.4): a present, listed service whose status was not
/// confirmed within its 60 s horizon, by `freshness.v1`. A warning: the
/// owner has not confirmed its level, and why — stopped, its writer
/// closed, its clock ahead, the link — is unobservable from the status
/// alone (§5). Stale is never FAILED, and never down.
fn health_stale(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    const C: CheckId = CheckId::HealthStale;
    let h = health_read(p, spec);
    let mut findings = Vec::new();
    let mut undecided = h.undecided;
    let mut fresh = 0usize;
    for s in &h.subjects {
        match s.last.verdict {
            Verdict::Stale => findings.push(finding(
                C,
                DoctorSeverity::Warning,
                s.addr.to_string(),
                format!(
                    "its instance token is held, and its status is stale: {} ({}) — not \
                     confirmed within its 60 s horizon, which is never FAILED and never down; the \
                     cause (its owner stopped, its writer closed, its clock ahead) is \
                     unobservable from the status alone (health.v1 §2.4, §5)",
                    s.last.reason.says(),
                    s.last.reason.as_str()
                ),
            )),
            Verdict::Healthy | Verdict::Unhealthy => fresh += 1,
            Verdict::Unobservable => undecided.push(health_unjudged(
                s,
                "its status's freshness could not be read",
            )),
            Verdict::NotAsked => {}
        }
    }
    let clean = if h.subjects.is_empty() && undecided.is_empty() {
        format!(
            "no present service's descriptor lists health.v1: health is asked of none (health.v1 \
             §2.7){}",
            unlisted_note(h.unlisted)
        )
    } else {
        format!(
            "{fresh} service(s) implementing health.v1, each status confirmed within its 60 s \
             horizon{}",
            unlisted_note(h.unlisted)
        )
    };
    CheckReport::of(C, findings, undecided, clean)
}

/// `health-inconsistent` (§2.2): an owner holding a status better than a
/// current check, in both of two readings a grace apart, each one GET that
/// answers the status and its checks together. An error: an owner MUST NOT
/// hold it. One reading's is unobservable, never clean and never the
/// finding.
fn health_inconsistent(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    const C: CheckId = CheckId::HealthInconsistent;
    let h = health_read(p, spec);
    let mut findings = Vec::new();
    let mut undecided = h.undecided;
    let mut agree = 0usize;
    for s in &h.subjects {
        match s.agreement.answer {
            HealthAnswerToken::No => findings.push(finding(
                C,
                DoctorSeverity::Error,
                s.addr.to_string(),
                format!(
                    "{}, {:.1}s apart: an owner MUST NOT hold a status better than its worst \
                     current check; a reader reads it at the check's level (health.v1 §2.2)",
                    s.agreement.says,
                    p.grace_s()
                ),
            )),
            HealthAnswerToken::Yes => agree += 1,
            HealthAnswerToken::Unobservable => {
                let hint = if s.agreement.reason == "clock_untrusted" {
                    " — the doctor measures no clock: pass --clocks-synced on the deployment's \
                     word (freshness.v1 §2.6)"
                } else {
                    ""
                };
                undecided.push(unjudged(
                    s.addr.to_string(),
                    format!(
                        "whether its status agrees with its checks cannot be told: {} ({}){hint}",
                        s.agreement.says, s.agreement.reason
                    ),
                ));
            }
            HealthAnswerToken::NotAsked => {}
        }
    }
    let clean = if h.subjects.is_empty() && undecided.is_empty() {
        format!(
            "no present service's descriptor lists health.v1: health is asked of none (health.v1 \
             §2.7){}",
            unlisted_note(h.unlisted)
        )
    } else {
        format!(
            "{agree} service(s) implementing health.v1, each fresh status no better than its \
             current checks{}",
            unlisted_note(h.unlisted)
        )
    };
    CheckReport::of(C, findings, undecided, clean)
}

/// §5's first question of `hostid.v1`, for one instance: is its system
/// minted?
enum Minted {
    /// Its descriptor lists `hostid.v1`, and no contract it implements
    /// lists it in `uses` (§2.8).
    Yes,
    /// Its descriptor does not list it.
    No,
    /// It lists it, and whether that is a minting cannot be told.
    Unobservable(String),
}

fn minted(p: &Presence<'_>, d: &Descriptor) -> Minted {
    if !d
        .profiles
        .iter()
        .any(|x| x == zenkey_model::hostid::PROFILE)
    {
        return Minted::No;
    }
    for e in &d.interfaces {
        match p.entry_revision(e) {
            Ok(rev) => {
                if rev
                    .contract()
                    .uses
                    .iter()
                    .any(|u| u.to_string() == zenkey_model::hostid::PROFILE)
                {
                    return Minted::Unobservable(format!(
                        "{} lists hostid.v1 in uses, so its listing is the contract's and \
                         minting cannot be read from it (hostid.v1 §2.8, §5)",
                        e.iface
                    ));
                }
            }
            Err(why) => {
                return Minted::Unobservable(format!(
                    "a contract it implements could not be read: {why}"
                ));
            }
        }
    }
    Minted::Yes
}

/// What one presence read shows of one address, for `hostid-duplicate`.
#[derive(Default)]
struct ZidRead {
    /// The session zids the counted instances state, by value, with each
    /// instance and its `meta.host`.
    zids: BTreeMap<String, Vec<(String, Option<String>)>>,
    /// What keeps the read from settling a *no*: a descriptor not read, an
    /// instance stating no `meta.zid`, one listing `hostid.v1` whose minting
    /// is unobservable.
    impediments: Vec<String>,
}

/// `hostid-duplicate` (`hostid.v1` §2.12, §5): an address whose instances
/// on a minted system state at least two session zids in both presence
/// reads of the scope's `@zk/**` (instance and interface tokens, 0.7), not
/// necessarily the same instances. Only instances whose system is known to
/// be minted are counted (§5's first question), and a recognised standby
/// (0.6, `redundancy.v1` §2.15) is not. The finding's
/// cause is undecided — a collision, a cloned machine id and a second
/// process look alike on the bus, and so does a standby not recognised — so
/// the evidence names none of them as the cause, and shows each instance's
/// `meta.host`. A warning: the deployment may expect it (a standby), and a
/// person decides. *No* is a read that ended at the routers' final reply,
/// every descriptor in it read, showing at most one zid. An address with
/// no instance listing `hostid.v1` is not this profile's; one on a system
/// in the minted shape with a descriptor unread may be, since only a minted
/// system has the shape (§2.1). A zid is constant for an instance, so an
/// instance's descriptor from either read stands for both.
fn hostid_duplicate(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::HostidDuplicate;
    let before = match &p.before {
        Ok(b) => *b,
        Err(why) => return CheckReport::unobservable(C, why.clone()),
    };
    let descriptor = |key: &(Addr, InstanceId)| -> Option<&DescriptorRead> {
        fn served<'o>(i: &Inst<'o>) -> Option<&'o DescriptorRead> {
            i.descriptor.filter(|r| r.descriptor().is_some())
        }
        p.now
            .get(key)
            .and_then(served)
            .or_else(|| p.then.get(key).and_then(served))
            .or_else(|| p.now.get(key).and_then(|i| i.descriptor))
            .or_else(|| p.then.get(key).and_then(|i| i.descriptor))
    };
    let addrs: BTreeSet<&Addr> = p
        .now
        .iter()
        .chain(p.then.iter())
        .filter(|(_, i)| i.instance_token)
        .map(|((a, _), _)| a)
        .collect();
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let (mut counted, mut literal) = (0usize, 0usize);
    let mut recognised: Vec<String> = Vec::new();
    // hostid.v1 §2.12 (0.6, 0.7): a recognised standby, `redundancy.v1`'s
    // fourth question answering no, standby, of every interface it lists,
    // all governed, from the same two reads of the address's @zk/**, is not
    // counted.
    use crate::judge::redundancy::Standby;
    let standby = |addr: &Addr, id: &InstanceId| {
        crate::judge::redundancy::recognised_standby(
            Some(before),
            Some(p.after),
            &p.obs.contracts,
            addr,
            id,
        )
    };
    let standbys_at = |addr: &Addr| -> Vec<String> {
        p.now
            .keys()
            .filter(|(a, i)| a == addr && standby(a, i) == Standby::Recognised)
            .map(|(a, i)| format!("{a}@{i}"))
            .collect()
    };
    for addr in addrs {
        let shaped = zenkey_model::hostid::is_minted_shape(addr.system.as_str());
        let mut lists = false;
        let mut unread = false;
        let read_of = |index: &BTreeMap<(Addr, InstanceId), Inst<'_>>| -> ZidRead {
            let mut r = ZidRead::default();
            for (key, _) in index
                .iter()
                .filter(|((a, _), i)| a == addr && i.instance_token)
            {
                let at = format!("{}@{}", key.0, key.1);
                match standby(&key.0, &key.1) {
                    Standby::Recognised => continue,
                    Standby::Unobservable(why) => {
                        r.impediments.push(format!(
                            "{at} lists only governed interfaces, or ones whose governance \
                             cannot be read, and whether it is a standby is unobservable: {why}"
                        ));
                        continue;
                    }
                    Standby::Counted(_) => {}
                }
                let d = match descriptor(key) {
                    Some(DescriptorRead::Served(d)) => d,
                    Some(other) => {
                        r.impediments.push(format!(
                            "the descriptor of {at} did not read ({})",
                            undescribed_why(other)
                        ));
                        continue;
                    }
                    None => {
                        r.impediments
                            .push(format!("the descriptor of {at} was not read"));
                        continue;
                    }
                };
                match minted(p, d) {
                    Minted::No => {}
                    Minted::Unobservable(why) => r.impediments.push(format!(
                        "{at} lists hostid.v1 and whether its system is minted is unobservable: \
                         {why}"
                    )),
                    Minted::Yes => match d.meta.get("zid").and_then(|z| z.as_str()) {
                        Some(z) => r.zids.entry(zid_value(z)).or_default().push((
                            at,
                            d.meta
                                .get("host")
                                .and_then(|h| h.as_str())
                                .map(str::to_owned),
                        )),
                        None => r.impediments.push(format!("{at} states no meta.zid")),
                    },
                }
            }
            r
        };
        for key in p.now.keys().chain(p.then.keys()).filter(|(a, _)| a == addr) {
            match descriptor(key) {
                Some(DescriptorRead::Served(d)) => {
                    lists |= d
                        .profiles
                        .iter()
                        .any(|x| x == zenkey_model::hostid::PROFILE);
                }
                _ => unread = true,
            }
        }
        if !lists && !(shaped && unread) {
            if shaped {
                literal += 1;
            }
            continue;
        }
        let (then, now) = (read_of(&p.then), read_of(&p.now));
        let standbys = standbys_at(addr);
        let established = |r: &ZidRead| r.zids.len() >= 2;
        let settles_no =
            |r: &ZidRead, complete: bool| complete && r.impediments.is_empty() && r.zids.len() <= 1;
        if established(&then) && established(&now) {
            let shown: Vec<String> = now
                .zids
                .iter()
                .flat_map(|(z, insts)| {
                    insts.iter().map(move |(at, host)| {
                        format!(
                            "{at} (zid {z}{})",
                            host.as_deref()
                                .map(|h| format!(", meta.host {h:?}"))
                                .unwrap_or_default()
                        )
                    })
                })
                .collect();
            findings.push(finding(
                C,
                DoctorSeverity::Warning,
                addr.to_string(),
                format!(
                    "in both presence reads {:.1}s apart, instances of {addr} on a system their \
                     descriptors declare minted state {} session zids, {} then and {} now: {} — \
                     two sessions claim one address, and the cause is undecided: a collision, a \
                     cloned machine id and a second process started as the same service look \
                     alike on the bus, and so does a standby not recognised (hostid.v1 §2.12)",
                    p.grace_s(),
                    now.zids.len().max(then.zids.len()),
                    then.zids.len(),
                    now.zids.len(),
                    shown.join(", ")
                ),
            ));
        } else if settles_no(&then, before.complete) || settles_no(&now, p.after.complete) {
            counted += 1;
            recognised.extend(standbys);
        } else {
            let mut why: Vec<String> = Vec::new();
            for (name, r, complete) in [
                ("the first", &then, before.complete),
                ("the second", &now, p.after.complete),
            ] {
                if established(r) {
                    why.push(format!("{name} read shows {} session zids", r.zids.len()));
                } else if !complete {
                    why.push(format!(
                        "{name} read ended at its timeout and shows at most one zid, which may \
                         have missed one"
                    ));
                }
                why.extend(r.impediments.iter().map(|i| format!("in {name} read, {i}")));
            }
            let mut seen = BTreeSet::new();
            why.retain(|w| seen.insert(w.clone()));
            undecided.push(unjudged(
                addr.to_string(),
                format!(
                    "the counted instances do not settle it: {} (hostid.v1 §2.12)",
                    why.join("; ")
                ),
            ));
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{counted} address(es) on minted systems, each with its counted instances stating one \
             session zid in a complete read with every descriptor read{}{}",
            if recognised.is_empty() {
                String::new()
            } else {
                format!(
                    "; not counted, {} recognised standby(s), redundancy.v1 §2.15's \"no, \
                     standby\" of every interface each lists: {}",
                    recognised.len(),
                    recognised.join(", ")
                )
            },
            if literal > 0 {
                format!(
                    "; {literal} on systems in the minted shape whose descriptors do not list \
                     hostid.v1, which is not this profile's"
                )
            } else {
                String::new()
            }
        ),
    )
}

// ─── redundancy.v1 (#841, RC) ───────────────────────────────────────────────

/// `redundancy-vacancy` (`redundancy.v1` §2.15, §5): an address and
/// governed interface whose candidates hold no incumbent in both of two
/// complete reads a grace apart, §5's *yes*, `lasting`, from the model's
/// own judge over the doctor's two reads. A warning: the election did not
/// end, or, under `external`, no instance holds the lock, and a person
/// decides. A vacancy in the second read only is a takeover due, and no
/// finding. An interface whose governance cannot be read is asked only
/// where its shape would be the finding were it governed, and then it is
/// unjudged; one known not governed is not this profile's.
fn redundancy_vacancy(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::RedundancyVacancy;
    use zenkey_model::redundancy::{self as rm, Election, Governance};
    let before = match &p.before {
        Ok(b) => *b,
        Err(why) => return CheckReport::unobservable(C, why.clone()),
    };
    let reads = [Some(before), Some(p.after)];
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
    let mut asked = 0usize;
    for (addr, iface) in crate::model::redundancy::subjects(&reads, None, None) {
        let governed =
            crate::model::redundancy::governance(&reads, &addr, &iface, &p.obs.contracts);
        let subject = format!("{addr} {iface}");
        let reading = |g| {
            crate::model::redundancy::reading(Some(before), Some(p.after), &addr, &iface, g, false)
        };
        let lasting =
            |j: rm::Judged| j.answer == rm::Answer::Yes && j.reason == rm::Reason::Lasting;
        match governed.governance {
            Governance::NotGoverned => {}
            Governance::Governed(e) => {
                asked += 1;
                let j = rm::vacancy(&reading(governed.governance));
                if lasting(j) {
                    findings.push(finding(
                        C,
                        DoctorSeverity::Warning,
                        subject,
                        format!(
                            "no instance holds its token in two complete presence reads {:.1}s \
                             apart, and a candidate lists it: a lasting vacancy (redundancy.v1 \
                             §2.15). {}",
                            p.grace_s(),
                            if e == Election::Claim {
                                "The election did not end: a candidate's reads may be refused, or \
                                 not complete"
                            } else {
                                "Under external, no instance holds the lock"
                            }
                        ),
                    ));
                } else if j.answer == rm::Answer::Unobservable {
                    undecided.push(unjudged(
                        subject,
                        format!("is there a vacancy? unobservable, {}", j.reason.as_str()),
                    ));
                }
            }
            Governance::NoContract | Governance::Disagree => {
                if lasting(rm::vacancy(&reading(Governance::Governed(Election::Claim)))) {
                    undecided.push(unjudged(
                        subject,
                        format!(
                            "no holder in two complete reads and a candidate listing it, a \
                             lasting vacancy were it governed, and whether it is cannot be read: \
                             {}",
                            governed.says
                        ),
                    ));
                }
            }
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{asked} interface(s) redundancy.v1 governs, none without an incumbent in both reads \
             {:.1}s apart",
            p.grace_s()
        ),
    )
}

// ─── config.v1 (#768, CC) ───────────────────────────────────────────────────

/// One service the `config-*` checks judge: present in the second read, its
/// descriptor listing `config.v1` (§2.1), with each reading's documents.
struct ConfigSubject<'o> {
    addr: Addr,
    /// The first reading's documents, when one was taken and the first
    /// presence read held the service's instance token.
    first: Option<&'o ServiceDocs>,
    /// The second reading's.
    second: &'o ServiceDocs,
    /// When the second reading's GET ended, this host's clock.
    read_at: std::time::SystemTime,
}

/// What the `config-*` checks read: the subjects, the services they could
/// not decide, and how many present services do not list `config.v1`.
struct ConfigRead<'o> {
    subjects: Vec<ConfigSubject<'o>>,
    undecided: Vec<Unjudged>,
    unlisted: usize,
}

/// Every present service's `config.v1` documents, two readings a grace
/// apart when `config-projection` is asked, one otherwise: the premise from
/// the instance token and the descriptor's listing (§2.1, §5).
fn config_read<'o>(p: &Presence<'o>) -> ConfigRead<'o> {
    let (first, second) = match p.obs.config.as_slice() {
        [first, second] => (Some(first), second),
        [second] => (None, second),
        _ => {
            return ConfigRead {
                subjects: Vec::new(),
                undecided: vec![unjudged(
                    "config.v1",
                    "no reading of the owners' config.v1/state/** was taken",
                )],
                unlisted: 0,
            };
        }
    };
    let catalog = Catalog::new(p.after);
    let before = p.before.as_ref().ok().map(|b| Catalog::new(b));
    let mut out = ConfigRead {
        subjects: Vec::new(),
        undecided: Vec::new(),
        unlisted: 0,
    };
    static NONE: std::sync::LazyLock<ServiceDocs> = std::sync::LazyLock::new(ServiceDocs::default);
    for addr in catalog.addresses().filter(|a| catalog.has_instance(a)) {
        match crate::model::config::listing(p.after, addr) {
            ConfigPremise::NotListed => {
                out.unlisted += 1;
                continue;
            }
            ConfigPremise::Listed => {}
            _ => {
                out.undecided.push(unjudged(
                    addr.to_string(),
                    "no descriptor of it read: whether it implements config.v1 cannot be told \
                     (config.v1 §2.1)",
                ));
                continue;
            }
        }
        let second = match second {
            Ok(g) => g,
            Err(e) => {
                out.undecided.push(unjudged(
                    addr.to_string(),
                    format!("the reading's GET could not be made: {e}"),
                ));
                continue;
            }
        };
        let first = match (first, &before) {
            (Some(Ok(g)), Some(b)) if b.has_instance(addr) => {
                Some(g.services.get(addr).unwrap_or(&NONE))
            }
            _ => None,
        };
        out.subjects.push(ConfigSubject {
            addr: addr.clone(),
            first,
            second: second.services.get(addr).unwrap_or(&NONE),
            read_at: second.read_at,
        });
    }
    if !p.complete {
        out.undecided
            .push(incomplete(p, "a service unseen may implement config.v1"));
    }
    out
}

/// The clean reason's tail: the services not asked.
fn config_unlisted(n: usize) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("; {n} present service(s) do not list config.v1, and are not asked")
    }
}

/// `config-projection` (§2.19): an owner holding a status that is not its
/// view's projection, in both of two readings a grace apart. One reading
/// can pair a view and a status a put apart (§2.3, F-112), so one reading's
/// mismatch is unjudged, never the finding; the last reading's match is
/// clean.
fn config_projection(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::ConfigProjection;
    let r = config_read(p);
    let mut findings = Vec::new();
    let mut undecided = r.undecided;
    let mut holds = 0usize;
    for s in &r.subjects {
        let last = crate::model::config::projection_of(s.second);
        let first = s.first.and_then(crate::model::config::projection_of);
        match (first, last) {
            (_, Some(true)) => holds += 1,
            (Some(false), Some(false)) => findings.push(finding(
                C,
                DoctorSeverity::Error,
                s.addr.to_string(),
                format!(
                    "in both of two readings {:.1}s apart, its status is not its view's \
                     projection: an owner keeps the status equal to the view's projection \
                     (config.v1 §2.19)",
                    p.grace_s()
                ),
            )),
            (_, Some(false)) => undecided.push(unjudged(
                s.addr.to_string(),
                "its status is not its view's projection in the last reading only: one reading \
                 can pair a view and a status a put apart (config.v1 §2.3)",
            )),
            (_, None) => undecided.push(unjudged(
                s.addr.to_string(),
                "its view and its status were not both read and decoded",
            )),
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{holds} service(s) implementing config.v1, each status its view's projection{}",
            config_unlisted(r.unlisted)
        ),
    )
}

/// How far past its pending change's deadline an owner's own stamp may put
/// a view still listing it: the view's horizon, the time an owner takes to
/// confirm it twice (§2.3), so an undo the device is still making is not
/// the finding.
pub const OVERDUE_GRACE: Duration = zenkey_model::config::TTL;

/// `config-overdue` (§2.11): an owner whose view, stamped by its own clock
/// more than [`OVERDUE_GRACE`] after its pending change's deadline, still
/// lists that change. The deadline and the stamp are both the owner's clock
/// (§2.8, S1), so no clock of this host's is trusted for it.
fn config_overdue(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::ConfigOverdue;
    let r = config_read(p);
    let mut findings = Vec::new();
    let mut undecided = r.undecided;
    let mut fine = 0usize;
    for s in &r.subjects {
        let subject = s.addr.to_string();
        let Some(reply) = &s.second.view else {
            undecided.push(unjudged(subject, "its view was not read"));
            continue;
        };
        let view = match &reply.value {
            Some(Ok(v)) => v,
            Some(Err(why)) => {
                undecided.push(unjudged(
                    subject,
                    format!("its view does not decode: {why}"),
                ));
                continue;
            }
            None => {
                undecided.push(unjudged(subject, "its view was deleted"));
                continue;
            }
        };
        let Some(pending) = &view.pending else {
            fine += 1;
            continue;
        };
        let Some(stamp) = &reply.stamp else {
            undecided.push(unjudged(
                subject,
                "its view's reply carries no stamp (core S2): when the owner put it cannot be \
                 told",
            ));
            continue;
        };
        let Some(deadline) = zenkey_model::config::Time::parse(&pending.deadline) else {
            undecided.push(unjudged(
                subject,
                format!(
                    "its pending change's deadline {:?} is no RFC 3339 instant (config.v1 §2.8)",
                    pending.deadline
                ),
            ));
            continue;
        };
        let put = stamp
            .time
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        let late = put - deadline.millis();
        if late > i64::try_from(OVERDUE_GRACE.as_millis()).unwrap_or(i64::MAX) {
            findings.push(finding(
                C,
                DoctorSeverity::Error,
                format!("{} {}", s.addr, pending.token),
                format!(
                    "its view, put {:.1}s after the deadline {} by its own clock, still lists \
                     change {} pending: at the deadline a change still pending is undone, once \
                     (config.v1 §2.11)",
                    late as f64 / 1000.0,
                    pending.deadline,
                    pending.token
                ),
            ));
        } else {
            fine += 1;
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{fine} service(s) implementing config.v1, none listing a change pending past its \
             deadline by its own clock{}",
            config_unlisted(r.unlisted)
        ),
    )
}

/// One subject's second reading as §5 reads it: the documents' freshness
/// from the GET's reply alone, aged on the deployment's word or not at all,
/// since the doctor listens to no document.
fn config_reading(s: &ConfigSubject<'_>, spec: &DoctorSpec) -> crate::model::config::ConfigReading {
    use crate::model::config::{Got, doc_read};
    let trust = if spec.clocks_synced {
        Trust::Word
    } else {
        Trust::None
    };
    crate::model::config::ConfigReading {
        premise: ConfigPremise::Listed,
        view: Some(doc_read(
            Got::of(s.second.view.as_ref(), s.read_at),
            None,
            &trust,
        )),
        status: doc_read(Got::of(s.second.status.as_ref(), s.read_at), None, &trust),
        events: Vec::new(),
    }
}

/// Why a §5 answer leaves a subject to someone else, with the cure for an
/// untrusted clock.
fn config_unjudged(s: &ConfigSubject<'_>, a: &crate::report::ConfigAnswer) -> Unjudged {
    let hint =
        if a.says.contains("clock_untrusted") || a.says.contains("not trusted to the HLC delta") {
            " — the doctor measures no clock: pass --clocks-synced on the deployment's word that \
         this host's clock and the owners' agree (freshness.v1 §2.6)"
        } else {
            ""
        };
    unjudged(s.addr.to_string(), format!("{}{hint}", a.says))
}

/// `config-unpersisted` (§5, §2.3): a parameter a fresh view shows with a
/// `startup` value, running a runtime change a restart would not keep. Info:
/// "a divergence an operator may want persisted or undone" (§5), never a
/// defect. A sensitive or unknown value is never shown, so never judged.
fn config_unpersisted(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    use crate::report::ConfigAnswerToken as A;
    const C: CheckId = CheckId::ConfigUnpersisted;
    let r = config_read(p);
    let mut findings = Vec::new();
    let mut undecided = r.undecided;
    let mut judged = 0usize;
    for s in &r.subjects {
        let reading = config_reading(s, spec);
        let Some(view) = reading.fresh_view() else {
            let a = crate::judge::config::none_fresh(&reading, true);
            undecided.push(config_unjudged(s, &a));
            continue;
        };
        judged += 1;
        for g in &view.groups {
            for prm in &g.parameters {
                let a = crate::judge::config::survives(&reading, &g.name, &prm.name);
                if a.answer == A::No {
                    findings.push(finding(
                        C,
                        DoctorSeverity::Info,
                        format!("{} {}.{}", s.addr, g.name, prm.name),
                        format!("{}: a restart would not keep it", a.says),
                    ));
                }
            }
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{judged} service(s) implementing config.v1, each fresh view running its startup \
             values (a sensitive value is never shown, so never judged){}",
            config_unlisted(r.unlisted)
        ),
    )
}

/// `config-desired` (§2.18, §5): an owner whose last desired document was
/// refused, or failed to converge, by a fresh view or status. An owner that
/// converges no desired state is clean here: the question is not asked of
/// it.
fn config_desired(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    use crate::report::ConfigAnswerToken as A;
    const C: CheckId = CheckId::ConfigDesired;
    let r = config_read(p);
    let mut findings = Vec::new();
    let mut undecided = r.undecided;
    let (mut converged, mut none) = (0usize, 0usize);
    for s in &r.subjects {
        let reading = config_reading(s, spec);
        let a = crate::judge::config::converged(&reading, None);
        match a.answer {
            A::No => findings.push(finding(
                C,
                DoctorSeverity::Warning,
                s.addr.to_string(),
                format!("its desired document was {} (config.v1 §2.18)", a.says),
            )),
            A::Yes => converged += 1,
            A::NotAsked => none += 1,
            A::NotYet | A::Unobservable => undecided.push(config_unjudged(s, &a)),
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{converged} service(s) converged to the last desired document they acted on, {none} \
             converging none{}",
            config_unlisted(r.unlisted)
        ),
    )
}

/// The run's `desired.v1` rows (#840, DD), judged as `zenctl desired`
/// judges a reading: the doctor's one reading, its window the bound, so a
/// target is found behind by a trusted clock alone, never by one reading.
fn desired_rows(
    p: &Presence<'_>,
    spec: &DoctorSpec,
) -> Result<Vec<crate::report::DesiredRow>, String> {
    let Some(read) = &p.obs.desired else {
        return Err("desired.v1 was not read".to_owned());
    };
    if let Some(why) = &read.cut_off {
        return Err(format!("desired.v1 was not read: {why}"));
    }
    let mut ds = crate::bus::desired::DesiredSpec::new(spec.timeout);
    ds.window = Some(spec.desired_behind);
    ds.clocks_synced = spec.clocks_synced;
    let mut behind = crate::model::desired::Behind::default();
    Ok(crate::judge::desired::judge(&p.obs.namespace, None, &ds, read, &mut behind, 1).rows)
}

/// A desired row's subject: the target and its role.
fn desired_subject(r: &crate::report::DesiredRow) -> String {
    format!("{} {}", r.target, r.role)
}

/// Why a row's answer leaves it unjudged: the answer and its words.
fn desired_unjudged(r: &crate::report::DesiredRow, q: &crate::report::DesiredQuestion) -> Unjudged {
    unjudged(
        desired_subject(r),
        format!("{} ({}): {}", q.answer.as_str(), q.reason, q.says),
    )
}

/// `desired-refused` (§2.7, §5): a target whose last act was refused or
/// failed, by its report. A warning: the document, or the target, needs a
/// person — a refusal is the target's answer to that version, and its next
/// is the author's to make.
fn desired_refused(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    use crate::report::DesiredAnswerToken as A;
    const C: CheckId = CheckId::DesiredRefused;
    let rows = match desired_rows(p, spec) {
        Ok(r) => r,
        Err(why) => return CheckReport::unobservable(C, why),
    };
    let (mut findings, mut undecided, mut clean) = (Vec::new(), Vec::new(), 0usize);
    for r in &rows {
        let q = &r.answers.refused;
        match q.answer {
            A::Yes => findings.push(finding(
                C,
                DoctorSeverity::Warning,
                desired_subject(r),
                format!("{} (desired.v1 §2.7)", q.says),
            )),
            A::Unobservable => undecided.push(desired_unjudged(r, q)),
            A::No | A::NotAsked | A::NotYet => clean += 1,
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!("{clean} desired.v1 role(s), none whose last act was refused or failed"),
    )
}

/// `desired-behind` (§2.17, text 0.4): a target "not yet" converged on a
/// version its author stamped longer ago than the bound, by a clock trusted
/// to the HLC delta. One reading alone never finds it: without
/// `--clocks-synced`, a target not yet converged is unjudged.
fn desired_behind(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    use crate::report::DesiredAnswerToken as A;
    const C: CheckId = CheckId::DesiredBehind;
    let rows = match desired_rows(p, spec) {
        Ok(r) => r,
        Err(why) => return CheckReport::unobservable(C, why),
    };
    let (mut findings, mut undecided, mut clean) = (Vec::new(), Vec::new(), 0usize);
    let bound = spec.desired_behind.as_secs_f64();
    for r in &rows {
        let q = &r.answers.converged;
        match (q.answer, &r.behind) {
            (A::NotYet, Some(b)) => findings.push(finding(
                C,
                DoctorSeverity::Warning,
                desired_subject(r),
                format!(
                    "{}; its author stamped {} {:.1}s ago, past the {bound:.0}s bound, by a clock \
                     trusted to the HLC delta (desired.v1 §2.17)",
                    q.says, b.version, b.since_s
                ),
            )),
            (A::NotYet, None) if !spec.clocks_synced => undecided.push(unjudged(
                desired_subject(r),
                format!(
                    "{} — how long it has been so cannot be told from one reading without a \
                     trusted clock: pass --clocks-synced on the deployment's word that this \
                     host's clock and the owners' agree (freshness.v1 §2.6)",
                    q.says
                ),
            )),
            (A::Unobservable, _) => undecided.push(desired_unjudged(r, q)),
            _ => clean += 1,
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{clean} desired.v1 role(s), none behind a version its author stamped more than \
             {bound:.0}s ago"
        ),
    )
}

/// `desired-last-known` (§2.9): a target running a document it read from
/// an archive, its author not answering for it. Info: last-known is what
/// §2.9 lets a target act on, and its author's confirmation clears it.
fn desired_last_known(p: &Presence<'_>, spec: &DoctorSpec) -> CheckReport {
    use crate::report::DesiredAnswerToken as A;
    const C: CheckId = CheckId::DesiredLastKnown;
    let rows = match desired_rows(p, spec) {
        Ok(r) => r,
        Err(why) => return CheckReport::unobservable(C, why),
    };
    let (mut findings, mut undecided, mut clean) = (Vec::new(), Vec::new(), 0usize);
    for r in &rows {
        let q = &r.answers.last_known;
        match q.answer {
            A::Yes => findings.push(finding(
                C,
                DoctorSeverity::Info,
                desired_subject(r),
                format!("{} (desired.v1 §2.9)", q.says),
            )),
            A::Unobservable => undecided.push(desired_unjudged(r, q)),
            A::No | A::NotAsked | A::NotYet => clean += 1,
        }
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!("{clean} desired.v1 role(s), none running a document read from an archive"),
    )
}

// ─── the checks that read no presence ───────────────────────────────────────

/// §8.3: the domain's token count against the budget. A read that ended at
/// its timeout counted a lower bound: over the budget is still a finding,
/// within it is not established.
fn presence_over_budget(obs: &DoctorObservation, spec: &DoctorSpec) -> CheckReport {
    const C: CheckId = CheckId::PresenceOverBudget;
    let d = match &obs.domain {
        Some(Ok(d)) => d,
        Some(Err(e)) => {
            return CheckReport::unobservable(C, format!("the token count failed: {e}"));
        }
        None => return CheckReport::not_asked(C),
    };
    let budget = spec.presence_budget;
    let counted = format!(
        "{} token(s) visible to this reader in the presence domain ({} zk2, under {} \
         namespace(s); {} other), through `{}`{}",
        d.tokens,
        d.zk2,
        d.namespaces,
        d.tokens - d.zk2.min(d.tokens),
        DOMAIN_SELECTORS.join("` and `"),
        if d.complete {
            ""
        } else {
            " — a lower bound: a read ended at its timeout"
        }
    );
    if d.tokens > budget {
        return CheckReport::of(
            C,
            vec![finding(
                C,
                DoctorSeverity::Warning,
                "presence domain",
                format!(
                    "{counted}; the budget is {budget}: about 10–15k tokens per domain kept \
                     discovery within 2–4 s, and 50k took 46–49 s or never finished (spike S2). \
                     Tokens under another application's verbatim chunk are not counted"
                ),
            )],
            vec![],
            "unused",
        );
    }
    if !d.complete {
        return CheckReport::unobservable(C, format!("{counted}, within the budget {budget}"));
    }
    CheckReport::of(
        C,
        vec![],
        vec![],
        format!("{counted}; within the budget {budget}"),
    )
}

/// The admin space, or the verdict a check takes without it.
fn admin_of(obs: &DoctorObservation, check: CheckId) -> Result<&AdminSpace, CheckReport> {
    match &obs.admin {
        Some(Ok(a)) => Ok(a),
        Some(Err(e)) => Err(CheckReport::unobservable(
            check,
            format!("the admin space could not be asked: {e}"),
        )),
        None => Err(CheckReport::not_asked(check)),
    }
}

/// The reason an admin-space check is unobservable when no router answered.
fn no_router(a: &AdminSpace) -> String {
    let base = if a.complete {
        format!(
            "no router answered `{ROUTERS}` through this reader: the admin space is disabled, \
             the mesh is peer-only, or access control denies it"
        )
    } else {
        format!("the admin read of `{ROUTERS}` ended at its timeout with no router")
    };
    if a.unverified.is_empty() {
        base
    } else {
        format!(
            "{base}; {} answer(s) could not be shown to be a router's, and are not trusted \
             (§4.2, 0.12): {}",
            a.unverified.len(),
            a.unverified.join("; ")
        )
    }
}

/// The answers no check may count, as unjudged subjects (§4.2, 0.12).
fn unverified_answers(a: &AdminSpace) -> Vec<Unjudged> {
    a.unverified
        .iter()
        .map(|u| {
            unjudged(
                "admin space",
                format!(
                    "{u}: any session can answer under `@/<zid>/router`, so only a router's own \
                     reply, from a router this session is connected to, is trusted (§4.2, 0.12)"
                ),
            )
        })
        .collect()
}

/// §4.2 S4: each storage's key expression against every owner's state
/// keys in this namespace — `state/**` and `@state/**`, named apart
/// because `*` and `**` never cross a verbatim chunk.
fn storage_on_state(obs: &DoctorObservation) -> CheckReport {
    const C: CheckId = CheckId::StorageOnState;
    let a = match admin_of(obs, C) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if a.routers.is_empty() {
        return CheckReport::unobservable(C, no_router(a));
    }
    let owners: Vec<(String, zenoh::key_expr::OwnedKeyExpr)> = ["state", "@state"]
        .into_iter()
        .filter_map(|token| {
            let rel = [GRAMMAR, "*", "*", "*", token, "**"].join("/");
            let full = crate::model::namespace::join(&obs.namespace, rel);
            let ke = zenoh::key_expr::OwnedKeyExpr::try_from(full.clone()).ok()?;
            Some((full, ke))
        })
        .collect();
    let mut findings = Vec::new();
    let mut undecided = unverified_answers(a);
    for s in &a.storages {
        let subject = format!("{}@{}", s.name, s.zid);
        let Some(text) = &s.key_expr else {
            undecided.push(unjudged(subject, "its admin document names no key_expr"));
            continue;
        };
        let Ok(ke) = zenoh::key_expr::OwnedKeyExpr::try_from(text.clone()) else {
            undecided.push(unjudged(
                subject,
                format!("{text:?} is not a key expression"),
            ));
            continue;
        };
        let hits: Vec<&str> = owners
            .iter()
            .filter(|(_, o)| ke.intersects(o))
            .map(|(full, _)| full.as_str())
            .collect();
        if !hits.is_empty() {
            findings.push(finding(
                C,
                DoctorSeverity::Error,
                subject,
                format!(
                    "captures `{text}`, which answers on owners' state keys (`{}`): the owner \
                     is authoritative for its state, and last-known state is an archive.v1's \
                     (§4.4)",
                    hits.join("`, `")
                ),
            ));
        }
    }
    if !a.complete {
        undecided.push(unjudged(
            "admin space",
            "a read ended at its timeout: a storage may be unseen",
        ));
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{} storage(s) on {} router(s), none answering on an owner's `state/**` or \
             `@state/**` keys in this namespace",
            a.storages.len(),
            a.routers.len()
        ),
    )
}

/// The admin space S4's check reads: a complete read with no router is the
/// finding, worth knowing rather than a defect.
fn admin_unreachable(obs: &DoctorObservation) -> CheckReport {
    const C: CheckId = CheckId::AdminUnreachable;
    let a = match admin_of(obs, C) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !a.routers.is_empty() {
        return CheckReport::of(
            C,
            vec![],
            vec![],
            format!("{} router(s) answered `{ROUTERS}`", a.routers.len()),
        );
    }
    if !a.complete {
        return CheckReport::unobservable(C, no_router(a));
    }
    CheckReport::of(
        C,
        vec![finding(
            C,
            DoctorSeverity::Info,
            "admin space",
            format!(
                "{}; storage-on-state and router-version-skew cannot be judged",
                no_router(a)
            ),
        )],
        vec![],
        "unused",
    )
}

/// Every router's version, as its admin document states it: the core
/// relies on zenoh 1.10.1's behaviour (Appendix B).
fn router_version_skew(obs: &DoctorObservation) -> CheckReport {
    const C: CheckId = CheckId::RouterVersionSkew;
    let a = match admin_of(obs, C) {
        Ok(a) => a,
        Err(r) => return r,
    };
    if a.routers.is_empty() {
        return CheckReport::unobservable(C, no_router(a));
    }
    let mut versions: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut undecided = unverified_answers(a);
    for r in &a.routers {
        match &r.version {
            Some(v) => versions.entry(v).or_default().push(&r.zid),
            None => undecided.push(unjudged(
                format!("router {}", r.zid),
                "its admin document states no version",
            )),
        }
    }
    let mut findings = Vec::new();
    if versions.len() > 1 {
        findings.push(finding(
            C,
            DoctorSeverity::Warning,
            "mesh",
            format!(
                "the routers run {}: the core relies on zenoh 1.10.1's behaviour (Appendix B), \
                 which a mixed mesh may not hold",
                versions
                    .iter()
                    .map(|(v, zids)| format!("{v} ({})", zids.join(", ")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    } else if !a.complete {
        undecided.push(unjudged(
            "admin space",
            "a read ended at its timeout: a router may be unseen",
        ));
    }
    CheckReport::of(
        C,
        findings,
        undecided,
        format!(
            "{} router(s), all at {}",
            a.routers.len(),
            versions.keys().next().copied().unwrap_or("?")
        ),
    )
}

/// §7.4, locally: below the floor, an `@stream` writer's shared memory
/// falls back to TCP and nothing tells its sender.
fn shm_memlock_low(obs: &DoctorObservation) -> CheckReport {
    const C: CheckId = CheckId::ShmMemlockLow;
    const MIB: u64 = 1024 * 1024;
    let floor = zenkey::shm::MEMLOCK_FLOOR;
    match obs.memlock {
        None => CheckReport::not_asked(C),
        Some(Memlock::Limited(l)) if l < floor => CheckReport::of(
            C,
            vec![finding(
                C,
                DoctorSeverity::Info,
                "this host",
                format!(
                    "RLIMIT_MEMLOCK is {} KiB, below the {} MiB floor: a shared-memory pool of \
                     a service started under it falls back to TCP silently (a deployment \
                     SHOULD allow the pool plus 2 MiB). This is zenctl's own limit, as \
                     inherited where it runs",
                    l / 1024,
                    floor / MIB
                ),
            )],
            vec![],
            "unused",
        ),
        Some(Memlock::Limited(l)) => CheckReport::of(
            C,
            vec![],
            vec![],
            format!(
                "RLIMIT_MEMLOCK is {} KiB, at or above the {} MiB floor",
                l / 1024,
                floor / MIB
            ),
        ),
        Some(Memlock::Unlimited) => {
            CheckReport::of(C, vec![], vec![], "RLIMIT_MEMLOCK is unlimited")
        }
        Some(Memlock::Unknown) => CheckReport::unobservable(
            C,
            "RLIMIT_MEMLOCK could not be read on this host (getrlimit failed)",
        ),
    }
}

#[cfg(test)]
mod tests {
    //! Every check's three poles from values: the finding, the clean
    //! answer, and the subject it could not decide — and the empty scope.

    use super::*;
    use crate::report::{ContractSource, Judgement};
    use serde_json::json;

    const A: &str = "000000000000000a";
    const B: &str = "000000000000000b";

    fn load(text: &str) -> Contract {
        let l = zenkey_model::contract::load_str(text, std::path::Path::new("."), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// `tc.v1`: `set`, optional and exclusive, `diagnostics` beside it,
    /// replicated when `replicated`; `extra` adds an optional operation,
    /// and `minor` is the informative minor.
    fn tc(minor: u32, replicated: bool, extra: bool) -> Contract {
        let op = |name: &str, more: &str| {
            format!(
                "[resources.{name}]\nkind = \"operation\"\n{more}\
                 request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n"
            )
        };
        let mut t = format!("[interface]\nname = \"tc\"\nmajor = 1\nminor = {minor}\n");
        t += &op("set", "optional = true\n");
        t += &op(
            "diagnostics",
            if replicated {
                "idempotent = true\nserving = \"replicated\"\n"
            } else {
                "idempotent = true\n"
            },
        );
        if extra {
            t += &op("extra", "optional = true\n");
        }
        load(&t)
    }

    /// `gui.v1`, declaring a role `netif` on `tc.v1`, optional or not.
    fn gui(optional: bool) -> Contract {
        load(&format!(
            "[interface]\nname = \"gui\"\nmajor = 1\n\
             [resources.status]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n\
             [requires.netif]\ninterface = \"tc.v1\"\nresources = [\"set\"]\noptional = {optional}\n"
        ))
    }

    fn fp(c: &Contract) -> Fingerprint {
        Fingerprint::of(c)
    }

    fn alive(service: &str, iface: &str, instance: &str, c: &Contract) -> String {
        format!(
            "zk2/{service}/@zk/alive/{iface}/{instance}/{}",
            fp(c).hex().fp16()
        )
    }

    fn inst(service: &str, instance: &str) -> String {
        format!("zk2/{service}/@zk/instance/{instance}")
    }

    /// A descriptor of `service@instance` implementing `entries` (each an
    /// interface entry's JSON), requiring `requires`.
    fn descriptor(
        service: &str,
        instance: &str,
        entries: &[serde_json::Value],
        requires: &[serde_json::Value],
    ) -> DescriptorRead {
        let d: Descriptor = serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1",
            "service": service,
            "instance": instance,
            "interfaces": entries,
            "requires": requires,
            "meta": {"zid": format!("zid-{instance}")},
        }))
        .expect("a descriptor");
        DescriptorRead::Served(Box::new(d))
    }

    fn entry(c: &Contract) -> serde_json::Value {
        json!({"iface": c.iface.to_string(), "contract": fp(c).to_string(),
               "minor": c.minor.unwrap_or(0)})
    }

    fn observed(keys: &[String], descriptors: Vec<((&str, &str), DescriptorRead)>) -> Observed {
        let mut o = Observed::from_keys("zk2/*/*/@zk/**", keys, true);
        o.descriptors = Some(
            descriptors
                .into_iter()
                .map(|((s, i), r)| ((s.parse().unwrap(), i.parse().unwrap()), r))
                .collect(),
        );
        o
    }

    /// An observation of `after`, read twice alike, with `held` retrieved
    /// and `unavailable` not.
    fn obs(after: Observed, held: &[&Contract], unavailable: &[&Contract]) -> DoctorObservation {
        let mut contracts = BTreeMap::new();
        for c in held {
            let r = Revision::from_contract((*c).clone(), ContractSource::Bus);
            contracts.insert(
                (c.iface.clone(), fp(c)),
                Ok(ContractState::Held(Arc::new(r))),
            );
        }
        for c in unavailable {
            contracts.insert(
                (c.iface.clone(), fp(c)),
                Ok(ContractState::Unavailable { refused: vec![] }),
            );
        }
        let mut before = after.clone();
        before.descriptors = None;
        DoctorObservation {
            namespace: "acme".into(),
            grace: Duration::from_secs(2),
            before: Some(Ok(before)),
            after: Some(Ok(after)),
            contracts,
            ..Default::default()
        }
    }

    fn spec() -> DoctorSpec {
        DoctorSpec {
            deep: true,
            ..DoctorSpec::new(Duration::from_secs(1))
        }
    }

    fn check(o: &DoctorObservation, id: CheckId) -> CheckReport {
        judge(o, &spec()).check(id).expect("every check").clone()
    }

    #[test]
    fn a_zid_compares_by_value_not_by_its_text() {
        // §3.3 (0.11, F-78): zenoh drops leading zeros; case is no meaning.
        assert_eq!(zid_value("00AB12"), zid_value("ab12"));
        assert_eq!(zid_value("0"), zid_value("000"));
        assert_ne!(zid_value("ab12"), zid_value("ab120"));
        assert_eq!(zid_value("zid-x"), "zid-x", "not hex: kept as written");
    }

    #[track_caller]
    fn found(r: &CheckReport) -> &DoctorFinding {
        assert_eq!(r.verdict, Judgement::Established, "{r:#?}");
        &r.findings[0]
    }

    #[track_caller]
    fn clean(r: &CheckReport) -> String {
        match &r.verdict {
            Judgement::NotEstablished { reason } => reason.clone(),
            other => panic!("not clean: {other:?}\n{r:#?}"),
        }
    }

    #[track_caller]
    fn unseen(r: &CheckReport) -> String {
        match &r.verdict {
            Judgement::Unobservable { reason } => reason.clone(),
            other => panic!("not unobservable: {other:?}\n{r:#?}"),
        }
    }

    fn after_mut(o: &mut DoctorObservation) -> &mut Observed {
        match &mut o.after {
            Some(Ok(after)) => after,
            _ => panic!("an observation with a presence read"),
        }
    }

    // ── split-brain (§6) ────────────────────────────────────────────────

    #[test]
    fn split_brain_is_two_exclusive_holders_in_both_reads() {
        let c = tc(0, false, false);
        let keys = [
            inst("h1/tc", A),
            inst("h1/tc", B),
            alive("h1/tc", "tc.v1", A, &c),
            alive("h1/tc", "tc.v1", B, &c),
        ];
        let o = obs(observed(&keys, vec![]), &[&c], &[]);
        let f = found(&check(&o, CheckId::SplitBrain)).clone();
        assert_eq!(f.subject, "h1/tc tc.v1");
        assert_eq!(f.severity, DoctorSeverity::Error);
        assert!(f.evidence.contains(A) && f.evidence.contains(B), "{f:?}");

        // A re-mint: the old instance in the first read only.
        let mut o = obs(
            observed(&[inst("h1/tc", B), alive("h1/tc", "tc.v1", B, &c)], vec![]),
            &[&c],
            &[],
        );
        o.before = Some(Ok(observed(&keys, vec![])));
        assert!(clean(&check(&o, CheckId::SplitBrain)).contains("tokenless set"));

        // The contract unread: undecided, never clear (O5).
        let o = obs(observed(&keys, vec![]), &[], &[&c]);
        assert!(unseen(&check(&o, CheckId::SplitBrain)).contains("cannot be decided"));

        // Replicated serving: one holder lists the exclusive `set`
        // unavailable, so at most one exposes it.
        let r = tc(0, true, false);
        let both = [
            inst("h1/tc", A),
            inst("h1/tc", B),
            alive("h1/tc", "tc.v1", A, &r),
            alive("h1/tc", "tc.v1", B, &r),
        ];
        let mut replica = entry(&r);
        replica["unavailable"] = json!([{"resource": "@op/set", "cause": "config"}]);
        let o = obs(
            observed(
                &both,
                vec![
                    (("h1/tc", A), descriptor("h1/tc", A, &[entry(&r)], &[])),
                    (("h1/tc", B), descriptor("h1/tc", B, &[replica], &[])),
                ],
            ),
            &[&r],
            &[],
        );
        assert!(clean(&check(&o, CheckId::SplitBrain)).contains("replicated serving"));

        // A read that ended at its timeout: no split-brain seen is not none.
        let mut o = obs(observed(&keys[..2], vec![]), &[&c], &[]);
        after_mut(&mut o).complete = false;
        assert!(unseen(&check(&o, CheckId::SplitBrain)).contains("timeout"));
    }

    // ── binding-unsatisfied (§3.2) ──────────────────────────────────────

    fn role(declared_by: Option<&str>, bindings: &[&str]) -> serde_json::Value {
        json!({"role": "netif", "interface": "tc.v1", "declared_by": declared_by,
               "bindings": bindings})
    }

    /// [`bound`] with the consumer's manifest role stated optional (§3.3,
    /// 0.10).
    fn bound_optional(c: &Contract, to: &[&str]) -> Observed {
        let mut r = role(None, to);
        r["optional"] = json!(true);
        observed(
            &[
                inst("h1/tc", A),
                alive("h1/tc", "tc.v1", A, c),
                inst("ws/gui", B),
            ],
            vec![
                (("h1/tc", A), descriptor("h1/tc", A, &[entry(c)], &[])),
                (("ws/gui", B), descriptor("ws/gui", B, &[], &[r])),
            ],
        )
    }

    /// The tc provider `h1/tc` and a consumer `ws/gui` whose role `netif`
    /// is bound to `to`, declared by `gui.v1` when `by_contract`.
    fn bound(c: &Contract, g: Option<&Contract>, to: &[&str]) -> Observed {
        let entries: Vec<serde_json::Value> = g.iter().map(|g| entry(g)).collect();
        observed(
            &[
                inst("h1/tc", A),
                alive("h1/tc", "tc.v1", A, c),
                inst("ws/gui", B),
            ],
            vec![
                (("h1/tc", A), descriptor("h1/tc", A, &[entry(c)], &[])),
                (
                    ("ws/gui", B),
                    descriptor("ws/gui", B, &entries, &[role(g.map(|_| "gui.v1"), to)]),
                ),
            ],
        )
    }

    #[test]
    fn a_role_whose_bindings_select_nothing_is_graded_by_whether_it_is_required() {
        let c = tc(0, false, false);
        // Satisfied by the provider's token.
        let o = obs(bound(&c, None, &["*/tc"]), &[&c], &[]);
        assert!(clean(&check(&o, CheckId::BindingUnsatisfied)).starts_with("1 bound role"));
        // A manifest role selecting nothing: required unless its entry says
        // `optional` (§3.3, 0.10).
        let o = obs(bound(&c, None, &["h9/tc"]), &[&c], &[]);
        let r = check(&o, CheckId::BindingUnsatisfied);
        let f = found(&r);
        assert_eq!(f.subject, "ws/gui netif");
        assert_eq!(f.severity, DoctorSeverity::Error);
        assert!(f.evidence.contains("visible to this reader"), "{f:?}");
        let o = obs(bound_optional(&c, &["h9/tc"]), &[&c], &[]);
        assert_eq!(
            found(&check(&o, CheckId::BindingUnsatisfied)).severity,
            DoctorSeverity::Info
        );
        // A run that asked no check comparing two reads took one: its read
        // is complete, and the finding stands.
        let mut o = obs(bound(&c, None, &["h9/tc"]), &[&c], &[]);
        o.before = None;
        found(&check(&o, CheckId::BindingUnsatisfied));
        // A contract's required role: an error; an optional one: info.
        for (optional, severity) in [(false, DoctorSeverity::Error), (true, DoctorSeverity::Info)] {
            let g = gui(optional);
            let o = obs(bound(&c, Some(&g), &["h9/tc"]), &[&c, &g], &[]);
            assert_eq!(
                found(&check(&o, CheckId::BindingUnsatisfied)).severity,
                severity
            );
        }
        // A required role bound to nothing at all.
        let g = gui(false);
        let o = obs(bound(&c, Some(&g), &[]), &[&c, &g], &[]);
        assert!(
            found(&check(&o, CheckId::BindingUnsatisfied))
                .evidence
                .contains("MUST NOT start")
        );
        // An optional one unbound is what §3.2 allows.
        let g = gui(true);
        let o = obs(bound(&c, Some(&g), &[]), &[&c, &g], &[]);
        assert!(clean(&check(&o, CheckId::BindingUnsatisfied)).contains("left unbound"));
        // A read that ended at its timeout: the provider may be unseen.
        let mut o = obs(bound(&c, None, &["h9/tc"]), &[&c], &[]);
        after_mut(&mut o).complete = false;
        assert!(unseen(&check(&o, CheckId::BindingUnsatisfied)).contains("present and unseen"));
    }

    // ── contract-drift (§9.8) ───────────────────────────────────────────

    #[test]
    fn providers_at_two_revisions_are_classified_in_their_order() {
        let two = |a: &Contract, b: &Contract| {
            observed(
                &[
                    inst("h1/tc", A),
                    inst("h2/tc", B),
                    alive("h1/tc", "tc.v1", A, a),
                    alive("h2/tc", "tc.v1", B, b),
                ],
                vec![
                    (("h1/tc", A), descriptor("h1/tc", A, &[entry(a)], &[])),
                    (("h2/tc", B), descriptor("h2/tc", B, &[entry(b)], &[])),
                ],
            )
        };
        // An optional operation added, minor 0 → 1: compatible.
        let (old, added) = (tc(0, false, false), tc(1, false, true));
        let o = obs(two(&old, &added), &[&old, &added], &[]);
        assert!(clean(&check(&o, CheckId::ContractDrift)).contains("compatible"));
        // The same change the other way, by the minors' order: removed.
        let (removed, base) = (tc(0, false, true), tc(1, false, false));
        let o = obs(two(&removed, &base), &[&removed, &base], &[]);
        let r = check(&o, CheckId::ContractDrift);
        let f = found(&r);
        assert!(f.evidence.contains("ordered by the minors"), "{f:?}");
        assert_ne!(f.severity, DoctorSeverity::Info);
        // No order to read: the minors agree, and the classes do not.
        let same_minor = tc(0, false, true);
        let o = obs(two(&old, &same_minor), &[&old, &same_minor], &[]);
        assert!(unseen(&check(&o, CheckId::ContractDrift)).contains("came first"));
        // A revision that cannot be had is not classified.
        let o = obs(two(&old, &added), &[&old], &[&added]);
        assert!(unseen(&check(&o, CheckId::ContractDrift)).contains("unavailable"));
        // One revision: nothing to classify.
        let o = obs(two(&old, &old), &[&old], &[]);
        assert!(clean(&check(&o, CheckId::ContractDrift)).contains("1 served at one revision"));
    }

    // ── contract-unavailable (§8.4) ─────────────────────────────────────

    #[test]
    fn a_revision_no_holder_serves_is_the_finding_and_silence_is_not() {
        let c = tc(0, false, false);
        let one = |read: DescriptorRead| {
            observed(
                &[inst("h1/tc", A), alive("h1/tc", "tc.v1", A, &c)],
                vec![(("h1/tc", A), read)],
            )
        };
        let served = || descriptor("h1/tc", A, &[entry(&c)], &[]);
        let o = obs(one(served()), &[], &[&c]);
        let f = found(&check(&o, CheckId::ContractUnavailable)).clone();
        assert_eq!(f.subject, format!("tc.v1 {}", fp(&c)));
        assert!(f.evidence.contains("named by h1/tc@"), "{f:?}");
        let o = obs(one(served()), &[&c], &[]);
        clean(&check(&o, CheckId::ContractUnavailable));
        let o = obs(one(DescriptorRead::Silent), &[], &[]);
        assert!(unseen(&check(&o, CheckId::ContractUnavailable)).contains("did not read"));
    }

    // ── descriptor-invalid (§3.3) ───────────────────────────────────────

    #[test]
    fn a_descriptor_is_checked_against_the_contracts_it_names() {
        let c = tc(0, false, false);
        let with = |e: serde_json::Value| {
            observed(
                &[inst("h1/tc", A), alive("h1/tc", "tc.v1", A, &c)],
                vec![(("h1/tc", A), descriptor("h1/tc", A, &[e], &[]))],
            )
        };
        let mut bad = entry(&c);
        bad["unavailable"] = json!([{"resource": "@op/nope", "cause": "config"}]);
        let o = obs(with(bad.clone()), &[&c], &[]);
        let f = found(&check(&o, CheckId::DescriptorInvalid)).clone();
        assert!(f.evidence.starts_with("D005"), "{f:?}");
        // Its contract unheld: syntax only, said so.
        let o = obs(with(bad), &[], &[&c]);
        assert!(unseen(&check(&o, CheckId::DescriptorInvalid)).contains("syntax only"));
        let o = obs(with(entry(&c)), &[&c], &[]);
        clean(&check(&o, CheckId::DescriptorInvalid));
        // A reply the presence read refused carries its codes.
        let o = obs(
            observed(
                &[inst("h1/tc", A)],
                vec![(("h1/tc", A), DescriptorRead::Invalid("D002 bad".into()))],
            ),
            &[],
            &[],
        );
        assert_eq!(
            found(&check(&o, CheckId::DescriptorInvalid)).evidence,
            "D002 bad"
        );
    }

    // ── token-missing (§8.1) ────────────────────────────────────────────

    #[test]
    fn tokens_must_agree_with_the_descriptor_in_both_reads() {
        let c = tc(0, false, false);
        let d = || descriptor("h1/tc", A, &[entry(&c)], &[]);
        let bare = || observed(&[inst("h1/tc", A)], vec![(("h1/tc", A), d())]);
        // Exposes tc.v1 and holds no token for it.
        let o = obs(bare(), &[&c], &[]);
        let f = found(&check(&o, CheckId::TokenMissing)).clone();
        assert_eq!(f.subject, format!("h1/tc@{A} tc.v1"));
        // Holding it: clean.
        let keys = [inst("h1/tc", A), alive("h1/tc", "tc.v1", A, &c)];
        let o = obs(observed(&keys, vec![(("h1/tc", A), d())]), &[&c], &[]);
        clean(&check(&o, CheckId::TokenMissing));
        // Tokenless: no token expected.
        let mut e = entry(&c);
        e["token"] = json!(false);
        let o = obs(
            observed(
                &[inst("h1/tc", A)],
                vec![(("h1/tc", A), descriptor("h1/tc", A, &[e], &[]))],
            ),
            &[&c],
            &[],
        );
        clean(&check(&o, CheckId::TokenMissing));
        // A token its descriptor does not list.
        let o = obs(
            observed(
                &keys,
                vec![(("h1/tc", A), descriptor("h1/tc", A, &[], &[]))],
            ),
            &[&c],
            &[],
        );
        assert!(
            found(&check(&o, CheckId::TokenMissing))
                .evidence
                .contains("does not list")
        );
        // Came up within the grace period: not judged yet.
        let mut o = obs(bare(), &[&c], &[]);
        o.before = Some(Ok(observed(&[], vec![])));
        assert!(unseen(&check(&o, CheckId::TokenMissing)).contains("grace period"));
        // Its contract unheld: whether it exposes anything is unknown.
        let o = obs(bare(), &[], &[&c]);
        assert!(unseen(&check(&o, CheckId::TokenMissing)).contains("needs its contract"));
        // A read that ended at its timeout: the token may be unseen.
        let mut o = obs(bare(), &[&c], &[]);
        after_mut(&mut o).complete = false;
        assert!(unseen(&check(&o, CheckId::TokenMissing)).contains("timeout"));
    }

    // ── the empty scope, and what is not asked ──────────────────────────

    #[test]
    fn an_empty_scope_is_unobservable_and_the_rest_still_answer() {
        let mut o = obs(observed(&[], vec![]), &[], &[]);
        o.namespace = "wrong".into();
        o.admin = Some(Ok(AdminSpace {
            routers: vec![router("r1", Some("1.10.1"))],
            router_zids: ["r1".to_owned()].into(),
            storages: vec![],
            complete: true,
            unverified: vec![],
        }));
        o.memlock = Some(Memlock::Limited(64 * 1024 * 1024));
        let report = judge(&o, &spec());
        let why = report.unobservable.clone().expect("the empty scope");
        assert!(
            why.contains("no zk2 token visible") && why.contains("\"wrong\""),
            "{why}"
        );
        for c in &report.checks {
            if c.check.reads_presence() {
                assert!(c.verdict.is_unobservable(), "{c:?}");
            }
        }
        clean(report.check(CheckId::RouterVersionSkew).unwrap());
        clean(report.check(CheckId::ShmMemlockLow).unwrap());
        assert_eq!(
            crate::report::judgement_exit_code(&report.judgement(DoctorSeverity::Warning)),
            2
        );
    }

    #[test]
    fn checks_not_asked_say_so() {
        let o = obs(observed(&[inst("h1/tc", A)], vec![]), &[], &[]);
        let mut s = spec();
        s.checks = [CheckId::SplitBrain].into();
        let report = judge(&o, &s);
        for c in &report.checks {
            assert_eq!(
                c.verdict.is_not_asked(),
                c.check != CheckId::SplitBrain,
                "{c:?}"
            );
        }
        let s = DoctorSpec::new(Duration::from_secs(1));
        assert!(!s.asks(CheckId::StateStampForeign), "deep only");
        assert!(!s.asks(CheckId::PopulationOverBound), "deep only");
        let report = judge(&o, &s);
        for c in [CheckId::StateStampForeign, CheckId::PopulationOverBound] {
            assert!(report.check(c).unwrap().verdict.is_not_asked(), "{c}");
        }
    }

    // ── presence-over-budget (§8.3) ─────────────────────────────────────

    #[test]
    fn the_budget_reads_a_lower_bound_honestly() {
        let mut s = spec();
        s.presence_budget = 3;
        let domain = |tokens, complete| {
            let o = DoctorObservation {
                domain: Some(Ok(DomainTokens {
                    tokens,
                    zk2: tokens,
                    namespaces: 1,
                    complete,
                })),
                ..Default::default()
            };
            judge(&o, &s)
                .check(CheckId::PresenceOverBudget)
                .unwrap()
                .clone()
        };
        assert_eq!(found(&domain(4, true)).severity, DoctorSeverity::Warning);
        found(&domain(4, false));
        clean(&domain(3, true));
        assert!(unseen(&domain(3, false)).contains("lower bound"));
    }

    // ── the admin space (§4.2) ──────────────────────────────────────────

    fn router(zid: &str, version: Option<&str>) -> RouterInfo {
        RouterInfo {
            zid: zid.into(),
            version: version.map(str::to_owned),
            locators: vec![],
            raw: json!({}),
        }
    }

    fn storage(ke: Option<&str>) -> StorageInfo {
        StorageInfo {
            zid: "r1".into(),
            name: "st".into(),
            key_expr: ke.map(str::to_owned),
            strip_prefix: None,
            volume: None,
            raw: json!({}),
        }
    }

    fn admin(
        routers: Vec<RouterInfo>,
        storages: Vec<StorageInfo>,
        complete: bool,
    ) -> DoctorObservation {
        DoctorObservation {
            namespace: "acme".into(),
            admin: Some(Ok(AdminSpace {
                router_zids: routers.iter().map(|r| zid_value(&r.zid)).collect(),
                routers,
                storages,
                complete,
                unverified: vec![],
            })),
            ..Default::default()
        }
    }

    #[test]
    fn an_admin_answer_no_router_can_be_shown_to_have_sent_is_never_clean() {
        // §4.2 (0.12, F-80): a session answering `@/<zid>/router` itself.
        let spoof = "`@/r1/router`, answered by c0ffee".to_owned();
        let mut o = admin(vec![], vec![], true);
        if let Some(Ok(a)) = &mut o.admin {
            a.unverified.push(spoof.clone());
        }
        for c in [CheckId::StorageOnState, CheckId::RouterVersionSkew] {
            assert!(
                unseen(&check(&o, c)).contains("could not be shown"),
                "{c:?}"
            );
        }
        assert!(
            found(&check(&o, CheckId::AdminUnreachable))
                .evidence
                .contains("c0ffee")
        );
        // Beside a verified router, still no clean verdict.
        let mut o = admin(vec![router("r1", Some("1.10.1"))], vec![], true);
        if let Some(Ok(a)) = &mut o.admin {
            a.unverified.push(spoof);
        }
        let r = check(&o, CheckId::StorageOnState);
        assert!(
            !matches!(r.verdict, Judgement::NotEstablished { .. }),
            "{r:#?}"
        );
    }

    #[test]
    fn a_storage_on_an_owner_s_state_keys_is_the_s4_finding() {
        let r = || vec![router("r1", Some("1.10.1"))];
        for ke in ["acme/zk2/**", "acme/zk2/h1/tc/tc.v1/@state/**", "**"] {
            let o = admin(r(), vec![storage(Some(ke))], true);
            let f = found(&check(&o, CheckId::StorageOnState)).clone();
            assert_eq!(f.subject, "st@r1", "{ke}");
        }
        // Another namespace's keys, and a stream's, are not this owner's state.
        for ke in ["other/zk2/**", "acme/zk2/*/*/*/stream/**"] {
            let o = admin(r(), vec![storage(Some(ke))], true);
            clean(&check(&o, CheckId::StorageOnState));
        }
        let o = admin(r(), vec![storage(None)], true);
        assert!(unseen(&check(&o, CheckId::StorageOnState)).contains("no key_expr"));
        // No router answered: S4 cannot be judged, and admin-unreachable fires.
        let o = admin(vec![], vec![], true);
        assert!(unseen(&check(&o, CheckId::StorageOnState)).contains("no router answered"));
        assert_eq!(
            found(&check(&o, CheckId::AdminUnreachable)).severity,
            DoctorSeverity::Info
        );
        let o = admin(vec![], vec![], false);
        assert!(unseen(&check(&o, CheckId::AdminUnreachable)).contains("timeout"));
        let o = admin(r(), vec![], true);
        clean(&check(&o, CheckId::AdminUnreachable));
    }

    #[test]
    fn routers_at_two_versions_are_a_skew() {
        let o = admin(
            vec![router("r1", Some("1.10.1")), router("r2", Some("1.9.0"))],
            vec![],
            true,
        );
        assert!(
            found(&check(&o, CheckId::RouterVersionSkew))
                .evidence
                .contains("1.9.0")
        );
        let o = admin(vec![router("r1", Some("1.10.1"))], vec![], true);
        assert!(clean(&check(&o, CheckId::RouterVersionSkew)).contains("1.10.1"));
        let o = admin(vec![router("r1", None)], vec![], true);
        assert!(unseen(&check(&o, CheckId::RouterVersionSkew)).contains("no version"));
    }

    // ── archive-unaligned (§4.4) ────────────────────────────────────────

    #[test]
    fn an_archive_serving_unconfirmed_keys_is_unaligned() {
        let keys = [inst("g/archive", A)];
        let mut o = obs(observed(&keys, vec![]), &[], &[]);
        let addr: Addr = "g/archive".parse().unwrap();
        let mut read = |k: ArchiveKeys| {
            o.archives = [(addr.clone(), Ok(k))].into();
            check(&o, CheckId::ArchiveUnaligned)
        };
        let f = found(&read(ArchiveKeys {
            values: 2,
            unconfirmed: 1,
            examples: vec!["zk2/g/archive/archive.v1/@state/x".into()],
            tombstones: 0,
            complete: true,
        }))
        .clone();
        assert_eq!(f.subject, "g/archive");
        assert!(f.evidence.contains("1 of 2"), "{f:?}");
        clean(&read(ArchiveKeys {
            values: 2,
            complete: true,
            ..Default::default()
        }));
        unseen(&read(ArchiveKeys::default()));
        o.archives.clear();
        assert!(clean(&check(&o, CheckId::ArchiveUnaligned)).contains("no archive.v1"));
    }

    // ── state-stamp-foreign (§4.2 S1–S2) ────────────────────────────────

    #[test]
    fn a_state_reply_is_stamped_by_its_owner_or_it_is_the_finding() {
        let c = tc(0, false, false);
        let keys = [inst("h1/tc", A), alive("h1/tc", "tc.v1", A, &c)];
        let mut o = obs(
            observed(
                &keys,
                vec![(("h1/tc", A), descriptor("h1/tc", A, &[entry(&c)], &[]))],
            ),
            &[&c],
            &[],
        );
        let at: (Addr, IfaceId) = ("h1/tc".parse().unwrap(), "tc.v1".parse().unwrap());
        let read = |o: &mut DoctorObservation, clocks: &[Option<&str>]| {
            let mut s = StateStamps {
                complete: true,
                ..Default::default()
            };
            for c in clocks {
                let e = s.by_clock.entry(c.map(str::to_owned)).or_default();
                e.0 += 1;
                e.1.push("zk2/h1/tc/tc.v1/state/x".into());
            }
            o.stamps = [(at.clone(), Ok(s))].into();
            check(o, CheckId::StateStampForeign)
        };
        let own = format!("zid-{A}");
        let foreign = |r: &CheckReport| found(r).evidence.contains("clock feed");
        // §4.2 (0.17): with no router verified, a foreign stamp is still the
        // finding, and the owner's own proves nothing.
        assert!(unseen(&read(&mut o, &[Some(&own)])).contains("the admin space was not read"));
        assert!(foreign(&read(&mut o, &[Some("feed")])));
        assert!(found(&read(&mut o, &[None])).evidence.contains("unstamped"));
        assert!(clean(&read(&mut o, &[])).contains("no stamp to attribute"));
        o.admin = Some(Ok(AdminSpace::default()));
        assert!(unseen(&read(&mut o, &[Some(&own)])).contains("no router's admin answer"));
        o.admin = Some(Err("refused".into()));
        assert!(unseen(&read(&mut o, &[Some(&own)])).contains("could not be read"));
        // Against a verified router that is not the owner, the owner's stamp
        // is clean.
        o.admin = Some(Ok(AdminSpace {
            routers: vec![router("r1", Some("1.10.1"))],
            router_zids: ["r1".to_owned()].into(),
            ..Default::default()
        }));
        clean(&read(&mut o, &[Some(&own)]));
        assert!(foreign(&read(&mut o, &[Some("feed")])));
        // §4.2 (0.16): an owner that is a router this run knows, verified or
        // only listed, proves nothing by its stamp; a foreign stamp is still
        // the finding (0.17).
        o.admin = Some(Ok(AdminSpace {
            routers: vec![router("r1", Some("1.10.1"))],
            router_zids: ["r1".to_owned(), zid_value(&own)].into(),
            ..Default::default()
        }));
        assert!(unseen(&read(&mut o, &[Some(&own)])).contains("A tool's S1 check"));
        assert!(foreign(&read(&mut o, &[Some("feed")])));
    }

    // ── population-over-bound (§2.7, 0.24) ───────────────────────────────

    /// `inv.v1`: a templated state of bound 4, a no-ceiling one, a plain one.
    fn inv() -> Contract {
        load(
            "[interface]\nname = \"inv\"\nmajor = 1\n\
             [resources.\"items/{item}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             params = { item = \"string\" }\ncardinality = 4\n\
             [resources.\"log/{entry}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             params = { entry = \"string\" }\ncardinality = 4294967295\n\
             [resources.summary]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n",
        )
    }

    /// §2.7: each owner interface's templated state, counted from the deep
    /// GET's keys answered with a value, against the descriptor's lowered
    /// bound, else the contract's. Above it is the finding, whether or not
    /// the GET ran to its final reply; within it, complete, is clean, and
    /// incomplete is unjudged; a resource with no member adds nothing; the
    /// no-ceiling bound is not asked; a `reply_del` is no member.
    #[test]
    fn a_state_population_above_its_bound_is_the_finding() {
        let c = inv();
        let items = |n: usize| -> Vec<String> {
            (0..n)
                .map(|i| format!("zk2/h1/inv/inv.v1/state/items/i{i}"))
                .collect()
        };
        let at: (Addr, IfaceId) = ("h1/inv".parse().unwrap(), "inv.v1".parse().unwrap());
        let with = |lowered: Option<u64>, keys: Vec<String>, complete: bool| {
            let mut e = entry(&c);
            if let Some(n) = lowered {
                e["cardinality"] = json!({"state/items/{item}": n});
            }
            let ks = [inst("h1/inv", A), alive("h1/inv", "inv.v1", A, &c)];
            let mut o = obs(
                observed(
                    &ks,
                    vec![(("h1/inv", A), descriptor("h1/inv", A, &[e], &[]))],
                ),
                &[&c],
                &[],
            );
            let s = StateStamps {
                complete,
                values: keys.into_iter().collect(),
                ..Default::default()
            };
            o.stamps = [(at.clone(), Ok(s))].into();
            check(&o, CheckId::PopulationOverBound)
        };
        // Three members against the descriptor's bound of 2.
        let r = with(Some(2), items(3), true);
        let f = found(&r).clone();
        assert_eq!(f.subject, "h1/inv inv.v1 state/items/{item}");
        assert_eq!(f.severity, DoctorSeverity::Error);
        assert!(
            f.evidence.contains("3 member(s)") && f.evidence.contains("its bound of 2"),
            "{f:?}"
        );
        // Five against the contract's 4, from a GET that timed out: still
        // the finding, said to be a lower bound.
        assert!(
            found(&with(None, items(5), false))
                .evidence
                .contains("lower bound")
        );
        // Within, complete: clean, with the counts.
        assert!(clean(&with(None, items(3), true)).starts_with("3 member(s) of 1 templated"));
        // Within, incomplete: unjudged.
        assert!(unseen(&with(None, items(3), false)).contains("may be live"));
        // No member: nothing to count, and the evidence says so.
        assert!(clean(&with(None, vec![], true)).contains("1 resource(s) answered no member"));
        // The no-ceiling log, however long, is not asked; the plain summary
        // has no bound.
        let mut keys = items(1);
        keys.extend((0..100).map(|i| format!("zk2/h1/inv/inv.v1/state/log/e{i}")));
        keys.push("zk2/h1/inv/inv.v1/state/summary".into());
        let report = with(None, keys, true);
        assert!(
            clean(&report).contains("1 with the no-ceiling bound"),
            "{report:?}"
        );
        // A revision no holder serves leaves the interface unjudged.
        let mut o = obs(
            observed(
                &[inst("h1/inv", A)],
                vec![(("h1/inv", A), descriptor("h1/inv", A, &[entry(&c)], &[]))],
            ),
            &[],
            &[&c],
        );
        o.stamps = [(
            at.clone(),
            Ok(StateStamps {
                complete: true,
                values: items(9).into_iter().collect(),
                ..Default::default()
            }),
        )]
        .into();
        assert!(unseen(&check(&o, CheckId::PopulationOverBound)).contains("unavailable"));
    }

    // ── shm-memlock-low (§7.4) ──────────────────────────────────────────

    #[test]
    fn a_memlock_below_the_floor_is_worth_knowing() {
        let at = |m| {
            let o = DoctorObservation {
                memlock: Some(m),
                ..Default::default()
            };
            check(&o, CheckId::ShmMemlockLow)
        };
        let f = found(&at(Memlock::Limited(64 * 1024))).clone();
        assert_eq!(f.severity, DoctorSeverity::Info);
        assert!(f.evidence.contains("64 KiB"), "{f:?}");
        clean(&at(Memlock::Limited(zenkey::shm::MEMLOCK_FLOOR)));
        clean(&at(Memlock::Unlimited));
        // An unreadable limit is not an unlimited one (#677).
        assert!(unseen(&at(Memlock::Unknown)).contains("could not be read"));
    }

    // ── health.v1 (#721, PF) ────────────────────────────────────────────

    use crate::bus::health::{CheckValue, Replied, ServiceGot, Stamped, StatusValue};
    use zenkey_model::health::Read;

    const C: &str = "000000000000000c";
    const D: &str = "000000000000000d";

    fn now() -> std::time::SystemTime {
        std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    /// A descriptor of `service@instance` listing `health.v1`, its token as
    /// `token` says.
    fn health_descriptor(service: &str, instance: &str, token: bool) -> DescriptorRead {
        descriptor(
            service,
            instance,
            &[json!({"iface": "health.v1",
                     "contract": format!("sha256:{}", zenkey::health::FINGERPRINT),
                     "minor": 0, "token": token})],
            &[],
        )
    }

    /// An owner's answer: its status at `level`, `age_s` old, and its checks.
    fn answered(level: Level, age_s: u64, checks: &[(&str, Level)]) -> ServiceGot {
        ServiceGot {
            status: Some(Replied {
                value: Some(StatusValue {
                    level: Read::Level(level),
                    reason: "netns gone".into(),
                    since_ns: 1,
                }),
                stamp: Some(Stamped {
                    time: now() - Duration::from_secs(age_s),
                    clock: "ab12".into(),
                }),
            }),
            checks: checks
                .iter()
                .map(|(n, l)| {
                    (
                        (*n).to_owned(),
                        Replied {
                            value: Some(CheckValue {
                                level: Read::Level(*l),
                                detail: String::new(),
                            }),
                            stamp: None,
                        },
                    )
                })
                .collect(),
        }
    }

    fn reading_of(services: Vec<(&str, ServiceGot)>) -> HealthGet {
        HealthGet {
            selector: "zk2/*/*/health.v1/state/**".into(),
            read_at: now(),
            complete: true,
            services: services
                .into_iter()
                .map(|(a, g)| (a.parse().unwrap(), g))
                .collect(),
        }
    }

    /// Four owners implementing `health.v1` (one in its tokenless set) and
    /// one that does not, each reading as `first` and `second` say.
    fn health_obs(
        first: Vec<(&str, ServiceGot)>,
        second: Vec<(&str, ServiceGot)>,
    ) -> DoctorObservation {
        let keys = [
            inst("lab/f", A),
            inst("lab/d", B),
            inst("lab/s", C),
            inst("lab/l", D),
            inst("lab/n", "000000000000000e"),
        ];
        let o = observed(
            &keys,
            vec![
                (("lab/f", A), health_descriptor("lab/f", A, true)),
                (("lab/d", B), health_descriptor("lab/d", B, false)),
                (("lab/s", C), health_descriptor("lab/s", C, true)),
                (("lab/l", D), health_descriptor("lab/l", D, true)),
                (
                    ("lab/n", "000000000000000e"),
                    descriptor("lab/n", "000000000000000e", &[], &[]),
                ),
            ],
        );
        let mut o = obs(o, &[], &[]);
        o.health = vec![Ok(reading_of(first)), Ok(reading_of(second))];
        o
    }

    fn synced() -> DoctorSpec {
        DoctorSpec {
            clocks_synced: true,
            ..spec()
        }
    }

    fn health_check(o: &DoctorObservation, s: &DoctorSpec, id: CheckId) -> CheckReport {
        judge(o, s).check(id).expect("every check").clone()
    }

    fn deployment() -> Vec<(&'static str, ServiceGot)> {
        vec![
            ("lab/f", answered(Level::Failed, 5, &[])),
            ("lab/d", answered(Level::Degraded, 5, &[])),
            ("lab/s", answered(Level::Ok, 70, &[])),
            ("lab/l", answered(Level::Ok, 5, &[("disk", Level::Failed)])),
        ]
    }

    #[test]
    fn health_checks_find_each_level_and_never_a_stale_status_at_one() {
        let o = health_obs(deployment(), deployment());
        let s = synced();
        // FAILED: by its status, and by a check worse than an OK status.
        let r = health_check(&o, &s, CheckId::HealthFailed);
        assert_eq!(r.verdict, Judgement::Established, "{r:#?}");
        let subjects: Vec<&str> = r.findings.iter().map(|f| f.subject.as_str()).collect();
        assert_eq!(subjects, ["lab/f", "lab/l"]);
        assert!(
            r.findings
                .iter()
                .all(|f| f.severity == DoctorSeverity::Error)
        );
        assert!(r.findings[0].evidence.contains("netns gone"), "{r:#?}");
        assert!(r.findings[1].evidence.contains("effective level"), "{r:#?}");
        // The stale one is undecided here, never FAILED.
        assert!(
            r.unjudged
                .iter()
                .any(|u| u.subject == "lab/s" && u.reason.contains("never a level"))
        );
        // DEGRADED, a warning; the tokenless owner found by its descriptor.
        let r = health_check(&o, &s, CheckId::HealthDegraded);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/d", DoctorSeverity::Warning)
        );
        // Stale: a finding of its own, a warning, never FAILED nor down.
        let r = health_check(&o, &s, CheckId::HealthStale);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/s", DoctorSeverity::Warning)
        );
        assert!(f.evidence.contains("never FAILED and never down"), "{f:?}");
        // The break of §2.2, in both readings: an error about the owner.
        let r = health_check(&o, &s, CheckId::HealthInconsistent);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/l", DoctorSeverity::Error)
        );
        assert!(f.evidence.contains("MUST NOT"), "{f:?}");
    }

    #[test]
    fn health_checks_are_clean_unobservable_and_not_asked_apart() {
        let healthy = || {
            vec![
                ("lab/f", answered(Level::Ok, 5, &[])),
                ("lab/d", answered(Level::Ok, 5, &[("disk", Level::Ok)])),
                ("lab/s", answered(Level::Ok, 5, &[])),
                ("lab/l", answered(Level::Failed, 5, &[("disk", Level::Ok)])),
            ]
        };
        let o = health_obs(healthy(), healthy());
        let s = synced();
        for id in [
            CheckId::HealthDegraded,
            CheckId::HealthStale,
            CheckId::HealthInconsistent,
        ] {
            let why = clean(&health_check(&o, &s, id));
            assert!(why.contains("4 service(s)"), "{id}: {why}");
            assert!(
                why.contains("1 present service(s) do not list"),
                "{id}: {why}"
            );
        }
        // A status worse than its checks is allowed (§2.2).
        assert_eq!(
            found(&health_check(&o, &s, CheckId::HealthFailed)).subject,
            "lab/l"
        );

        // No word for the clock: every reply's age is unobservable, and the
        // reason names the cure.
        let r = health_check(&o, &spec(), CheckId::HealthStale);
        assert!(unseen(&r).contains("4 subjects"), "{r:#?}");
        assert!(r.unjudged[0].reason.contains("--clocks-synced"), "{r:#?}");

        // A break in the last reading only: unobservable, never the finding.
        let o = health_obs(healthy(), deployment());
        let r = health_check(&o, &s, CheckId::HealthInconsistent);
        assert!(r.findings.is_empty());
        assert!(
            r.unjudged
                .iter()
                .any(|u| u.subject == "lab/l" && u.reason.contains("one_reading")),
            "{r:#?}"
        );

        // A GET that answered nothing for an owner: silent, undecided.
        let o = health_obs(vec![], vec![]);
        assert!(unseen(&health_check(&o, &s, CheckId::HealthFailed)).contains("silent"));

        // Nobody implements it: clean, health asked of none.
        let mut o = obs(
            observed(
                &[inst("lab/n", A)],
                vec![(("lab/n", A), descriptor("lab/n", A, &[], &[]))],
            ),
            &[],
            &[],
        );
        o.health = vec![Ok(reading_of(vec![])), Ok(reading_of(vec![]))];
        assert!(clean(&health_check(&o, &s, CheckId::HealthFailed)).contains("asked of none"));
        // A descriptor that did not read: whether it implements it is unknown.
        let mut o = obs(
            observed(
                &[inst("lab/n", A)],
                vec![(("lab/n", A), DescriptorRead::Silent)],
            ),
            &[],
            &[],
        );
        o.health = vec![Ok(reading_of(vec![]))];
        assert!(unseen(&health_check(&o, &s, CheckId::HealthStale)).contains("interface token"));
        // Not asked by the run: not read.
        let mut not = s.clone();
        not.checks.remove(&CheckId::HealthFailed);
        assert_eq!(
            health_check(&o, &not, CheckId::HealthFailed).verdict,
            Judgement::NotAsked
        );
    }

    // ── hostid-duplicate (hostid.v1 §2.12) ──────────────────────────────

    const SYSINFO: &str = "h-bbd1aa1db10b/sysinfo";

    /// A descriptor of `sysinfo@instance` on a minted system: `profiles`
    /// lists `hostid.v1`, `meta` states `zid` (unless `None`) and a host.
    fn minted_descriptor(
        instance: &str,
        zid: Option<&str>,
        entries: &[serde_json::Value],
    ) -> DescriptorRead {
        let mut meta = serde_json::Map::new();
        if let Some(z) = zid {
            meta.insert("zid".into(), json!(z));
        }
        meta.insert("host".into(), json!(format!("host-{instance}")));
        let d: Descriptor = serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1",
            "service": SYSINFO,
            "instance": instance,
            "interfaces": entries,
            "profiles": ["hostid.v1"],
            "meta": meta,
        }))
        .expect("a descriptor");
        DescriptorRead::Served(Box::new(d))
    }

    fn two_sysinfos(a: DescriptorRead, b: DescriptorRead) -> Observed {
        observed(
            &[inst(SYSINFO, A), inst(SYSINFO, B)],
            vec![((SYSINFO, A), a), ((SYSINFO, B), b)],
        )
    }

    #[test]
    fn two_sessions_claiming_a_minted_address_are_a_finding_whose_cause_is_undecided() {
        // §6 step 1: two zids in both reads.
        let o = obs(
            two_sysinfos(
                minted_descriptor(A, Some("aa01"), &[]),
                minted_descriptor(B, Some("bb02"), &[]),
            ),
            &[],
            &[],
        );
        let r = check(&o, CheckId::HostidDuplicate);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            (SYSINFO, DoctorSeverity::Warning)
        );
        assert!(f.evidence.contains("cause is undecided"), "{f:?}");
        assert!(f.evidence.contains("meta.host \"host-"), "{f:?}");
        // One zid, stated twice: one session, the re-mint rules' (§2.12).
        let o = obs(
            two_sysinfos(
                minted_descriptor(A, Some("aa01"), &[]),
                minted_descriptor(B, Some("00AA01"), &[]),
            ),
            &[],
            &[],
        );
        clean(&check(&o, CheckId::HostidDuplicate));
        // §6 step 3: one states no zid — undecided, never clean.
        let o = obs(
            two_sysinfos(
                minted_descriptor(A, Some("aa01"), &[]),
                minted_descriptor(B, None, &[]),
            ),
            &[],
            &[],
        );
        assert!(unseen(&check(&o, CheckId::HostidDuplicate)).contains("states no meta.zid"));
        // A re-mint: the second instance in one read only.
        let mut o = obs(
            observed(
                &[inst(SYSINFO, B)],
                vec![((SYSINFO, B), minted_descriptor(B, Some("bb02"), &[]))],
            ),
            &[],
            &[],
        );
        o.before = Some(Ok(two_sysinfos(
            minted_descriptor(A, Some("aa01"), &[]),
            minted_descriptor(B, Some("bb02"), &[]),
        )));
        clean(&check(&o, CheckId::HostidDuplicate));
        // A read that ended at its timeout shows one zid it may have missed.
        let mut o = obs(
            observed(
                &[inst(SYSINFO, A)],
                vec![((SYSINFO, A), minted_descriptor(A, Some("aa01"), &[]))],
            ),
            &[],
            &[],
        );
        after_mut(&mut o).complete = false;
        if let Some(Ok(b)) = &mut o.before {
            b.complete = false;
        }
        assert!(unseen(&check(&o, CheckId::HostidDuplicate)).contains("timeout"));
    }

    #[test]
    fn a_minting_a_contract_lists_in_uses_is_unobservable_and_not_counted() {
        // §6 step 5: both list hostid.v1, but a contract each implements
        // does too, so neither is counted.
        let x = load(
            "[interface]\nname = \"sysinfo_x\"\nmajor = 1\nminor = 0\nuses = [\"hostid.v1\"]\n\
             [resources.cpu]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n",
        );
        let mut e = entry(&x);
        e["token"] = json!(false);
        let o = obs(
            two_sysinfos(
                minted_descriptor(A, Some("aa01"), std::slice::from_ref(&e)),
                minted_descriptor(B, Some("bb02"), &[e]),
            ),
            &[&x],
            &[],
        );
        let why = unseen(&check(&o, CheckId::HostidDuplicate));
        assert!(why.contains("lists hostid.v1 in uses"), "{why}");
        // §6 step 4: a literal system in the minted shape is not minted, and
        // not this profile's.
        let literal = descriptor("h-504c6767c349/logger", A, &[], &[]);
        let o = obs(
            observed(
                &[inst("h-504c6767c349/logger", A)],
                vec![(("h-504c6767c349/logger", A), literal)],
            ),
            &[],
            &[],
        );
        assert!(clean(&check(&o, CheckId::HostidDuplicate)).contains("not this profile's"));
        // §6 step 2: two systems, one instance each.
        let other = "h-3f6d94515669/sysinfo";
        let mut b = minted_descriptor(B, Some("bb02"), &[]);
        if let DescriptorRead::Served(d) = &mut b {
            d.service = other.into();
        }
        let o = obs(
            observed(
                &[inst(SYSINFO, A), inst(other, B)],
                vec![
                    ((SYSINFO, A), minted_descriptor(A, Some("aa01"), &[])),
                    ((other, B), b),
                ],
            ),
            &[],
            &[],
        );
        assert!(clean(&check(&o, CheckId::HostidDuplicate)).starts_with("2 address(es)"));
    }

    // ── config.v1 (#768, CC) ────────────────────────────────────────────

    use crate::bus::config::{ConfigGet, ServiceDocs};
    use zenkey_model::config::{ConfigStatus, ConfigView};

    /// A descriptor of `service@instance` listing `config.v1`.
    fn config_descriptor(service: &str, instance: &str) -> DescriptorRead {
        descriptor(
            service,
            instance,
            &[json!({"iface": "config.v1",
                     "contract": format!("sha256:{}", zenkey::config_v1::FINGERPRINT),
                     "minor": 0})],
            &[],
        )
    }

    /// A view of `lab/rf0`, its `duty` running `value` with `startup` when
    /// given, and `extra` members over it.
    fn rf_view(value: i64, startup: Option<i64>, extra: serde_json::Value) -> ConfigView {
        let mut p = json!({"name": "fraction_ppm", "kind": "integer", "description": "p",
                           "value": value, "source": "runtime"});
        if let Some(s) = startup {
            p["startup"] = json!(s);
        }
        let mut v = json!({"service": "lab/rf0", "instance": "i1", "revision": 1,
                           "confirm_floor_s": 10,
                           "groups": [{"name": "duty", "class": "hot", "description": "d",
                                       "writable": true, "parameters": [p]}]});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        serde_json::from_value(v).unwrap()
    }

    /// One owner's documents: the view, stamped `age_s` before `now()`, and
    /// its status, the view's projection unless `status` says otherwise.
    fn docs(view: &ConfigView, status: Option<ConfigStatus>, age_s: u64) -> ServiceDocs {
        let stamp = Some(Stamped {
            time: now() - Duration::from_secs(age_s),
            clock: "ab12".into(),
        });
        ServiceDocs {
            view: Some(Replied {
                value: Some(Ok(view.clone())),
                stamp: stamp.clone(),
            }),
            status: Some(Replied {
                value: Some(Ok(status.unwrap_or_else(|| ConfigStatus::of(view)))),
                stamp,
            }),
        }
    }

    fn config_get(services: Vec<(&str, ServiceDocs)>) -> ConfigGet {
        ConfigGet {
            selector: "zk2/*/*/config.v1/state/**".into(),
            read_at: now(),
            complete: true,
            services: services
                .into_iter()
                .map(|(a, d)| (a.parse().unwrap(), d))
                .collect(),
        }
    }

    /// `lab/rf0` implementing `config.v1`, and `lab/n` implementing nothing.
    fn config_obs(
        first: Vec<(&str, ServiceDocs)>,
        second: Vec<(&str, ServiceDocs)>,
    ) -> DoctorObservation {
        let o = observed(
            &[inst("lab/rf0", A), inst("lab/n", B)],
            vec![
                (("lab/rf0", A), config_descriptor("lab/rf0", A)),
                (("lab/n", B), descriptor("lab/n", B, &[], &[])),
            ],
        );
        let mut o = obs(o, &[], &[]);
        o.config = vec![Ok(config_get(first)), Ok(config_get(second))];
        o
    }

    fn config_check(o: &DoctorObservation, s: &DoctorSpec, id: CheckId) -> CheckReport {
        judge(o, s).check(id).expect("every check").clone()
    }

    /// `config-projection` is two readings' finding; one reading's mismatch
    /// is unjudged; a service whose descriptor does not list config.v1 is
    /// not asked; the scope says what was read.
    #[test]
    fn config_projection_needs_both_readings() {
        let v = rf_view(20000, None, json!({}));
        let mut off = ConfigStatus::of(&v);
        off.revision = 7;
        let both = config_obs(
            vec![("lab/rf0", docs(&v, Some(off.clone()), 5))],
            vec![("lab/rf0", docs(&v, Some(off.clone()), 5))],
        );
        let r = config_check(&both, &spec(), CheckId::ConfigProjection);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/rf0", DoctorSeverity::Error)
        );
        assert!(f.evidence.contains("§2.19"), "{f:?}");
        let once = config_obs(
            vec![("lab/rf0", docs(&v, None, 5))],
            vec![("lab/rf0", docs(&v, Some(off), 5))],
        );
        let r = config_check(&once, &spec(), CheckId::ConfigProjection);
        assert!(unseen(&r).contains("last reading only"), "{r:#?}");
        let fine = config_obs(
            vec![("lab/rf0", docs(&v, None, 5))],
            vec![("lab/rf0", docs(&v, None, 5))],
        );
        let why = clean(&config_check(&fine, &spec(), CheckId::ConfigProjection));
        assert!(
            why.starts_with("1 service(s)") && why.contains("1 present service(s) do not list"),
            "{why}"
        );
        match judge(&fine, &spec()).scope.config {
            Asked::Asked(c) => {
                assert_eq!((c.services, c.readings), (1, 2));
                assert_eq!(c.selector, "zk2/*/*/config.v1/state/**");
            }
            Asked::NotAsked => panic!("the config readings were taken"),
        }
    }

    /// A desired.v1 reading of three targets (#840, DD): `lab/w2` refused,
    /// `lab/w3` not yet on a version its author stamped a minute ago, and
    /// `lab/w4` running an archive's last-known, its author absent.
    fn desired_obs() -> DoctorObservation {
        use crate::model::desired::{
            DesiredRead, ReportReply, ReportsGet, RoleRead, RoleResolution, SliceFrom, SliceGet,
            SliceReply, TargetRead,
        };
        use zenkey_model::desired::{Listing, Seen};
        use zenkey_model::seed::Stamp;
        let now = std::time::SystemTime::now();
        let target = |name: &str, author: Seen, report: &str, current: Option<(u64, u64)>| {
            let slice = format!("zk2/lab/author/tasking.v1/state/tasks/lab/{name}");
            let rp = format!("zk2/lab/{name}/worker.v1/state/desired/task");
            let mut res = RoleResolution::bare("task", Some(Listing::Listed { bound: true }));
            res.author = Some("lab/author".parse().unwrap());
            res.slice = vec![slice.clone()];
            res.slice_from = Some(SliceFrom::AuthorContract);
            TargetRead {
                addr: format!("lab/{name}").parse().unwrap(),
                seen: Seen::Present,
                seen_why: None,
                word: None,
                descriptor: Ok(None),
                roles: vec![RoleRead {
                    resolution: res,
                    author_seen: Some((author, None)),
                    current: current.map(|(t, ago)| {
                        Ok(SliceGet {
                            selector: slice.clone(),
                            complete: true,
                            replies: vec![SliceReply {
                                key: slice.clone(),
                                delete: false,
                                stamp: Some((
                                    Stamp::parse(t, "a1").unwrap(),
                                    now - Duration::from_secs(ago),
                                )),
                            }],
                            at: now,
                        })
                    }),
                    archived: None,
                }],
                reports: Some(Ok(ReportsGet {
                    selectors: vec![rp.clone()],
                    complete: true,
                    replies: vec![ReportReply {
                        key: rp,
                        read: zenkey_model::desired::decode(
                            report.replace("$D", &slice).as_bytes(),
                        ),
                        stamp: None,
                        horizon: zenkey_model::freshness::Horizon::Undeclared,
                    }],
                    at: now,
                })),
            }
        };
        let refused = r#"{"key":"$D","source":"desired","in_force":{"stamp":"100/a1","revision":1},"observed":{"stamp":"200/a1","revision":2},"outcome":"refused","refusal":{"code":"reach","pointer":"/endpoint"}}"#;
        let converged = r#"{"key":"$D","source":"desired","in_force":{"stamp":"100/a1","revision":1},"observed":{"stamp":"100/a1","revision":1},"outcome":"converged"}"#;
        let last_known = r#"{"key":"$D","source":"desired","in_force":{"stamp":"100/a1","revision":1},"observed":{"stamp":"100/a1","revision":1},"outcome":"converged","last_known":true}"#;
        let o = observed(&[inst("lab/w2", A), inst("lab/w3", B)], vec![]);
        let mut o = obs(o, &[], &[]);
        o.desired = Some(DesiredRead {
            began: std::time::Instant::now(),
            at: now,
            cut_off: None,
            presence: None,
            targets: vec![
                target("w2", Seen::Present, refused, Some((200, 0))),
                target("w3", Seen::Present, converged, Some((300, 61))),
                target("w4", Seen::Absent, last_known, None),
            ],
            asked: vec![],
        });
        o
    }

    /// `desired-refused` finds the refused act, `desired-last-known` the
    /// archive's document (info), and `desired-behind` the target behind a
    /// version stamped past the bound only on a trusted clock: from one
    /// reading without one, it is unjudged, the cure named.
    #[test]
    fn desired_checks_read_each_targets_report() {
        let o = desired_obs();
        let r = health_check(&o, &spec(), CheckId::DesiredRefused);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/w2 task", DoctorSeverity::Warning)
        );
        assert!(f.evidence.contains("reach"), "{f:?}");
        let r = health_check(&o, &spec(), CheckId::DesiredLastKnown);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/w4 task", DoctorSeverity::Info)
        );
        let why = unseen(&health_check(&o, &spec(), CheckId::DesiredBehind));
        assert!(
            why.contains("lab/w3 task") && why.contains("--clocks-synced"),
            "{why}"
        );
        let r = health_check(&o, &synced(), CheckId::DesiredBehind);
        let f = found(&r);
        assert_eq!(f.subject, "lab/w3 task");
        assert!(f.evidence.contains("past the 60s bound"), "{f:?}");
        let lenient = DoctorSpec {
            desired_behind: Duration::from_secs(120),
            ..synced()
        };
        let why = clean(&health_check(&o, &lenient, CheckId::DesiredBehind));
        assert!(why.contains("120s"), "{why}");
        let mut none = desired_obs();
        none.desired = None;
        assert!(
            unseen(&health_check(&none, &spec(), CheckId::DesiredRefused)).contains("not read")
        );
    }

    /// `config-overdue` reads the owner's own clock: a view stamped more
    /// than 60 s past its pending change's deadline is the finding, one
    /// within it is not, whatever this host's clock.
    #[test]
    fn config_overdue_reads_the_owners_own_clock() {
        let deadline = |past_s: u64| {
            let ms = (now() - Duration::from_secs(past_s))
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis();
            zenkey_model::config::Time::from_millis(i64::try_from(ms).unwrap()).rfc3339()
        };
        let pending = |past_s: u64| {
            rf_view(
                20000,
                Some(10000),
                json!({"pending": {"token": "i1-2", "groups": ["duty"],
                                   "deadline": deadline(past_s), "applied": true}}),
            )
        };
        let late = config_obs(vec![], vec![("lab/rf0", docs(&pending(90), None, 0))]);
        let f = found(&config_check(&late, &spec(), CheckId::ConfigOverdue)).clone();
        assert_eq!(f.subject, "lab/rf0 i1-2");
        assert!(f.evidence.contains("§2.11"), "{f:?}");
        let undoing = config_obs(vec![], vec![("lab/rf0", docs(&pending(20), None, 0))]);
        let why = clean(&config_check(&undoing, &spec(), CheckId::ConfigOverdue));
        assert!(why.starts_with("1 service(s)"), "{why}");
        let mut unstamped = docs(&pending(90), None, 0);
        unstamped.view.as_mut().unwrap().stamp = None;
        let o = config_obs(vec![], vec![("lab/rf0", unstamped)]);
        assert!(unseen(&config_check(&o, &spec(), CheckId::ConfigOverdue)).contains("no stamp"));
    }

    /// `config-unpersisted` and `config-desired` conclude only from a fresh
    /// document: on the deployment's word for the clock, each finds its §5
    /// "no"; without it, every service is unjudged, the reason naming the
    /// cure.
    #[test]
    fn config_findings_of_section_5_need_a_fresh_document() {
        let v = rf_view(
            20000,
            Some(10000),
            json!({"desired": {"observed_revision": 2, "outcome": "refused",
                               "refusal": {"code": "reach", "group": "radio"}}}),
        );
        let o = config_obs(vec![], vec![("lab/rf0", docs(&v, None, 5))]);
        let s = DoctorSpec {
            clocks_synced: true,
            ..spec()
        };
        let r = config_check(&o, &s, CheckId::ConfigUnpersisted);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/rf0 duty.fraction_ppm", DoctorSeverity::Info)
        );
        assert!(f.evidence.contains("10000"), "{f:?}");
        let r = config_check(&o, &s, CheckId::ConfigDesired);
        let f = found(&r);
        assert_eq!(f.severity, DoctorSeverity::Warning);
        assert!(f.evidence.contains("reach on radio"), "{f:?}");
        for id in [CheckId::ConfigUnpersisted, CheckId::ConfigDesired] {
            let r = config_check(&o, &spec(), id);
            assert!(r.findings.is_empty(), "{id}");
            assert!(
                r.unjudged[0].reason.contains("--clocks-synced"),
                "{id}: {r:#?}"
            );
        }
        let kept = rf_view(10000, None, json!({}));
        let o = config_obs(vec![], vec![("lab/rf0", docs(&kept, None, 5))]);
        let why = clean(&config_check(&o, &s, CheckId::ConfigUnpersisted));
        assert!(why.starts_with("1 service(s)"), "{why}");
        let why = clean(&config_check(&o, &s, CheckId::ConfigDesired));
        assert!(why.contains("1 converging none"), "{why}");
        // A GET that drew nothing for the owner: undecided, never clean.
        let o = config_obs(vec![], vec![]);
        assert!(
            config_check(&o, &s, CheckId::ConfigOverdue)
                .verdict
                .is_unobservable()
        );
    }

    // ─── alarms.v1 (#800, AC) ──────────────────────────────────────────────

    /// `lab/al` lists alarms.v1, its descriptor's `meta.zid` the stamps'
    /// clock, and its GET answers `replies`: each `(key, document, age)`.
    fn alarms_obs(replies: &[(&str, serde_json::Value, u64)]) -> DoctorObservation {
        use crate::model::alarms::{AlarmSample, SeedRead};
        use crate::report::AlarmVia;
        use zenoh::time::{NTP64, Timestamp, TimestampId};
        let d: Descriptor = serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1", "service": "lab/al", "instance": A,
            "interfaces": [{"iface": "alarms.v1",
                "contract": format!("sha256:{}", zenkey::alarms::FINGERPRINT),
                "minor": 0, "token": false}],
            "meta": {"zid": "a1"},
        }))
        .unwrap();
        let o = observed(
            &[inst("lab/al", A)],
            vec![(("lab/al", A), DescriptorRead::Served(Box::new(d)))],
        );
        let mut o = obs(o, &[], &[]);
        let now = std::time::SystemTime::now();
        let at = std::time::Instant::now();
        o.alarms = Some(Ok(SeedRead {
            asked_at: at,
            complete: true,
            discarded: 0,
            replies: replies
                .iter()
                .map(|(key, doc, age)| AlarmSample {
                    key: format!("zk2/lab/al/alarms.v1/state/alarms/{key}"),
                    delete: false,
                    payload: serde_json::to_vec(doc).unwrap(),
                    stamp: Some(Timestamp::new(
                        NTP64::from(
                            (now - Duration::from_secs(*age))
                                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                                .unwrap(),
                        ),
                        "a1".parse::<TimestampId>().unwrap(),
                    )),
                    at,
                    wall: now,
                    via: AlarmVia::Get,
                })
                .collect(),
        }));
        o
    }

    fn alarm(rule: &str, labels: serde_json::Value) -> serde_json::Value {
        json!({"timestamp": 1, "rule": rule, "severity": "warning", "state": "firing",
               "summary": "s", "labels": labels})
    }

    /// `alarm-key` (§2.2, §2.3, §5): a key that is not its document's is an
    /// error, one that is only through the sanitizing step a warning, a
    /// strict one clean — scenarios.md §9's three keys.
    #[test]
    fn alarm_key_finds_a_phantom_and_warns_of_a_sanitized_key() {
        let a = alarm("link_down", json!({"peer": "r2", "port": "eth0"}));
        let unframed = alarm("link\ndown", json!({"port": "22"}));
        let sanitized = zenkey_model::alarms::derive("link\ndown", [("port", "22")]).key;
        assert_eq!(sanitized, "16991d91fb818ca6", "scenarios.md §9's key");
        let o = alarms_obs(&[
            ("0000000000000001", a.clone(), 10),
            (&sanitized, unframed, 10),
            ("a659f813308ad1da", a, 10),
        ]);
        let r = check(&o, CheckId::AlarmKey);
        assert_eq!(r.verdict, Judgement::Established, "{r:#?}");
        let by: Vec<(&str, DoctorSeverity)> = r
            .findings
            .iter()
            .map(|f| (f.subject.rsplit('/').next().unwrap(), f.severity))
            .collect();
        assert_eq!(
            by,
            [
                ("0000000000000001", DoctorSeverity::Error),
                ("16991d91fb818ca6", DoctorSeverity::Warning),
            ]
        );
        assert!(
            r.findings[0].evidence.contains("a659f813308ad1da"),
            "{r:#?}"
        );
        let clean = alarms_obs(&[(
            "a659f813308ad1da",
            alarm("link_down", json!({"peer": "r2", "port": "eth0"})),
            10,
        )]);
        let r = check(&clean, CheckId::AlarmKey);
        assert!(
            matches!(&r.verdict, Judgement::NotEstablished { reason } if reason.contains("1 alarm key")),
            "{r:#?}"
        );
    }

    /// `alarm-stale` (§2.7, §5): a firing alarm of a present, listed owner
    /// unconfirmed beyond 900 s is the finding; a fresh one clean; without
    /// the operator's word on the clocks, unobservable, never clean.
    #[test]
    fn alarm_stale_finds_an_unconfirmed_firing_alarm_on_the_operators_word() {
        let a = alarm("link_down", json!({"peer": "r2", "port": "eth0"}));
        let b = alarm("disk_full", json!({"mount": "/var"}));
        let b_key = zenkey_model::alarms::derive("disk_full", [("mount", "/var")]).key;
        let o = alarms_obs(&[("a659f813308ad1da", a, 10), (&b_key, b, 1000)]);
        let r = health_check(&o, &synced(), CheckId::AlarmStale);
        let f = found(&r);
        assert!(f.subject.ends_with(&b_key), "{r:#?}");
        assert_eq!(f.severity, DoctorSeverity::Warning);
        let r = health_check(&o, &spec(), CheckId::AlarmStale);
        assert!(r.verdict.is_unobservable(), "{r:#?}");
        assert!(
            r.unjudged
                .iter()
                .any(|u| u.reason.contains("--clocks-synced")),
            "{r:#?}"
        );
        let mut none = alarms_obs(&[]);
        none.alarms = None;
        assert!(check(&none, CheckId::AlarmStale).verdict.is_unobservable());
    }

    /// `meter.v1`, the telemetry scenarios' contract, as the runtime's tests
    /// keep it.
    fn meter() -> Contract {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/meter.v1.toml");
        let text = std::fs::read_to_string(&path).expect("meter.v1");
        let l = zenkey_model::contract::load_str(&text, path.parent().unwrap(), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// `telemetry-declaration` (#819, TC): an invalid declaration is a
    /// warning, read from the contract a descriptor names; valid ones are
    /// clean, and a revision not in hand unjudged.
    #[test]
    fn an_invalid_metric_declaration_is_a_warning() {
        let bad = load(
            "[interface]\nname = \"odd\"\nmajor = 1\nuses = [\"telemetry.v1\"]\n\
             [resources.blob]\nkind = \"stream\"\ntype = { raw = \"application/octet-stream\" }\n\
             annotations = { \"telemetry.kind\" = \"counter\" }\n",
        );
        let good = meter();
        let after = observed(
            &[inst("lab/meter", A)],
            vec![(
                ("lab/meter", A),
                descriptor("lab/meter", A, &[entry(&bad), entry(&good)], &[]),
            )],
        );
        let r = check(
            &obs(after.clone(), &[&bad, &good], &[]),
            CheckId::TelemetryDeclaration,
        );
        let f = found(&r);
        assert_eq!(f.severity, DoctorSeverity::Warning);
        assert!(f.subject.ends_with("stream/blob"), "{f:?}");
        assert!(f.evidence.contains("not_a_carrier"), "{f:?}");
        assert_eq!(r.findings.len(), 1, "meter.v1's six are valid");
        let only_good = observed(
            &[inst("lab/meter", A)],
            vec![(
                ("lab/meter", A),
                descriptor("lab/meter", A, &[entry(&good)], &[]),
            )],
        );
        assert!(
            clean(&check(
                &obs(only_good.clone(), &[&good], &[]),
                CheckId::TelemetryDeclaration
            ))
            .contains("6 metric resource(s)")
        );
        assert!(
            unseen(&check(
                &obs(only_good, &[], &[&good]),
                CheckId::TelemetryDeclaration
            ))
            .contains("unavailable")
        );
    }

    /// `pace.v1`, the timing scenarios' contract, as the runtime's tests keep
    /// it.
    fn pace() -> Contract {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/pace.v1.toml");
        let text = std::fs::read_to_string(&path).expect("pace.v1");
        let l = zenkey_model::contract::load_str(&text, path.parent().unwrap(), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// `timing-declaration` (#908, MC): an invalid value, a deadline below
    /// its period, and timing keys on a state or a requirement are each a
    /// warning, read from the contract a descriptor names; `pace.v1`'s two
    /// timed streams are clean, and a revision not in hand unjudged.
    #[test]
    fn a_timing_declaration_no_reader_judges_is_a_warning() {
        let bad = load(
            "[interface]\nname = \"odd\"\nmajor = 1\nuses = [\"timing.v1\"]\n\
             [resources.fast]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"timing.period_ms\" = 0 }\n\
             [resources.slow]\nkind = \"stream\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"timing.period_ms\" = 100, \"timing.deadline_ms\" = 50 }\n\
             [resources.level]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             annotations = { \"timing.period_ms\" = 100 }\n\
             [requires.cmd]\ninterface = \"pace.v1\"\n\
             annotations = { \"timing.deadline_ms\" = 100 }\n",
        );
        let good = pace();
        let after = observed(
            &[inst("lab/odd", A)],
            vec![(
                ("lab/odd", A),
                descriptor("lab/odd", A, &[entry(&bad), entry(&good)], &[]),
            )],
        );
        let r = check(&obs(after, &[&bad, &good], &[]), CheckId::TimingDeclaration);
        assert_eq!(r.verdict, Judgement::Established, "{r:#?}");
        let by: BTreeMap<String, &DoctorFinding> = r
            .findings
            .iter()
            .map(|f| (f.subject.rsplit(' ').next().unwrap().to_owned(), f))
            .collect();
        assert_eq!(
            by.keys().map(String::as_str).collect::<Vec<_>>(),
            ["requires.cmd", "state/level", "stream/fast", "stream/slow"],
            "pace.v1's two are clean: {r:#?}"
        );
        assert!(by.values().all(|f| f.severity == DoctorSeverity::Warning));
        assert!(by["stream/fast"].evidence.contains("period_value"));
        assert!(by["stream/slow"].evidence.contains("below its period"));
        assert!(by["state/level"].evidence.contains("mean nothing"));
        assert!(by["requires.cmd"].evidence.contains("requirement"));
        let only_good = observed(
            &[inst("lab/pace", A)],
            vec![(
                ("lab/pace", A),
                descriptor("lab/pace", A, &[entry(&good)], &[]),
            )],
        );
        assert!(
            clean(&check(
                &obs(only_good.clone(), &[&good], &[]),
                CheckId::TimingDeclaration
            ))
            .contains("2 timed resource(s)")
        );
        assert!(
            unseen(&check(
                &obs(only_good, &[], &[&good]),
                CheckId::TimingDeclaration
            ))
            .contains("unavailable")
        );
    }

    /// `telemetry-sample` (#819, TC), with `--deep`: a state metric member's
    /// reply that does not conform is the finding; a reply on a resource
    /// that is no metric's is not read.
    #[test]
    fn a_state_metric_reply_that_does_not_conform_is_the_finding() {
        let c = meter();
        let mut o = obs(
            observed(
                &[inst("lab/meter", A)],
                vec![(
                    ("lab/meter", A),
                    descriptor("lab/meter", A, &[entry(&c)], &[]),
                )],
            ),
            &[&c],
            &[],
        );
        let at: (Addr, IfaceId) = ("lab/meter".parse().unwrap(), "meter.v1".parse().unwrap());
        let read = |o: &mut DoctorObservation, payload: &str| {
            let s = StateStamps {
                complete: true,
                replies: vec![StateReply {
                    key: "zk2/lab/meter/meter.v1/state/total".into(),
                    payload: payload.as_bytes().to_vec(),
                    attachment: None,
                }],
                ..Default::default()
            };
            o.stamps = [(at.clone(), Ok(s))].into();
            check(o, CheckId::TelemetrySample)
        };
        assert!(clean(&read(&mut o, "12")).contains("1 state metric sample(s)"));
        let r = read(&mut o, "\"twelve\"");
        let f = found(&r);
        assert_eq!(f.severity, DoctorSeverity::Error);
        assert!(f.evidence.contains("wrong_type"), "{f:?}");
        assert!(
            read(&mut o, "1e400").verdict.is_unobservable(),
            "undecodable"
        );
        assert!(
            !DoctorSpec::new(Duration::from_secs(1)).asks(CheckId::TelemetrySample),
            "deep only"
        );
    }

    /// `telemetry-sample` (#819, TF; zk2py round 48's F-157): a state metric
    /// member's reply under an invalid declaration is read as unobservable,
    /// never skipped, as an undecodable one is; the declaration's warning
    /// stays `telemetry-declaration`'s.
    #[test]
    fn a_reply_under_an_invalid_declaration_is_unobservable() {
        let odd = load(
            "[interface]\nname = \"odd\"\nmajor = 1\nuses = [\"telemetry.v1\"]\n\
             [resources.total]\nkind = \"state\"\ntype = { raw = \"application/octet-stream\" }\n\
             annotations = { \"telemetry.kind\" = \"counter\" }\n",
        );
        let mut o = obs(
            observed(
                &[inst("lab/odd", A)],
                vec![(
                    ("lab/odd", A),
                    descriptor("lab/odd", A, &[entry(&odd)], &[]),
                )],
            ),
            &[&odd],
            &[],
        );
        let at: (Addr, IfaceId) = ("lab/odd".parse().unwrap(), "odd.v1".parse().unwrap());
        o.stamps = [(
            at,
            Ok(StateStamps {
                complete: true,
                replies: vec![StateReply {
                    key: "zk2/lab/odd/odd.v1/state/total".into(),
                    payload: b"12".to_vec(),
                    attachment: None,
                }],
                ..Default::default()
            }),
        )]
        .into();
        let why = unseen(&check(&o, CheckId::TelemetrySample));
        assert!(
            why.contains("zk2/lab/odd/odd.v1/state/total") && why.contains("declaration_invalid"),
            "{why}"
        );
        assert_eq!(
            found(&check(&o, CheckId::TelemetryDeclaration)).severity,
            DoctorSeverity::Warning
        );
    }

    /// Review of PR #827: `telemetry-sample` keeps the replies of the
    /// interface's metric resources alone, within 4096 replies and 8 MiB of
    /// payload and attachment per owner interface; a metric reply past either
    /// is counted, and every other reply keeps its stamp alone.
    #[test]
    fn the_doctor_keeps_metric_replies_alone_and_within_its_caps() {
        let rev = Arc::new(Revision::from_contract(meter(), ContractSource::Bus));
        let reply = |key: &str| StateReply {
            key: key.to_owned(),
            payload: b"12".to_vec(),
            attachment: None,
        };
        let total = "zk2/lab/meter/meter.v1/state/total";
        let other = "zk2/lab/meter/meter.v1/state/other";
        let mut out = StateStamps::default();
        let mut keep = ReplyKeeper {
            revision: Some(Arc::clone(&rev)),
            bytes: 0,
        };
        keep.offer(&mut out, total, 2, || reply(total));
        keep.offer(&mut out, other, 2, || {
            panic!("no metric resource's: never copied")
        });
        assert_eq!((out.replies.len(), out.replies_capped), (1, 0));
        keep.offer(&mut out, total, STATE_REPLY_BYTES, || reply(total));
        assert_eq!(
            (out.replies.len(), out.replies_capped),
            (1, 1),
            "past 8 MiB: counted"
        );
        let mut none = ReplyKeeper::default();
        none.offer(&mut out, total, 2, || panic!("telemetry-sample not asked"));
        assert_eq!(out.replies.len(), 1);
    }

    // ── redundancy.v1 (#841, RC) ────────────────────────────────────────

    /// `ledger.v1`, which `redundancy.v1` governs (election `claim`), or
    /// the same interface with no `uses`.
    fn ledger(governed: bool) -> Contract {
        let mut t = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/ledger.v1.toml"),
        )
        .unwrap();
        if !governed {
            t = t.replace("uses  = [\"redundancy.v1\"]\n", "").replace(
                "[defaults]\nannotations = { \"redundancy.election\" = \"claim\" }\n",
                "",
            );
        }
        load(&t)
    }

    fn standby_entry(c: &Contract) -> serde_json::Value {
        let mut e = entry(c);
        e["standby"] = json!(true);
        e
    }

    const LEDGER: &str = "lab/ledger";

    /// `lab/ledger`'s A and B, A listing `ledger.v1` serving and B marked
    /// standby, each holding its interface token when told.
    fn candidates(l: &Contract, a_holds: bool, b_holds: bool) -> Observed {
        let mut keys = vec![inst(LEDGER, A), inst(LEDGER, B)];
        if a_holds {
            keys.push(alive(LEDGER, "ledger.v1", A, l));
        }
        if b_holds {
            keys.push(alive(LEDGER, "ledger.v1", B, l));
        }
        observed(
            &keys,
            vec![
                ((LEDGER, A), descriptor(LEDGER, A, &[entry(l)], &[])),
                ((LEDGER, B), descriptor(LEDGER, B, &[standby_entry(l)], &[])),
            ],
        )
    }

    #[test]
    fn a_split_of_a_governed_interface_is_the_first_questions_whatever_the_marks() {
        // redundancy.v1 §2.15, scenarios §12 step 7: two holders in both
        // reads, one marked standby, which exposes nothing.
        let l = ledger(true);
        let o = obs(candidates(&l, true, true), &[&l], &[]);
        let r = check(&o, CheckId::SplitBrain);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/ledger ledger.v1", DoctorSeverity::Error)
        );
        assert!(f.evidence.contains("whatever their marks"), "{f:?}");
        assert!(f.evidence.contains("heal"), "{f:?}");
        // Not governed, core §6 decides (0.39): a holder marked standby is
        // still a holder, and with no replicated resource every holder
        // exposes an exclusive one.
        let plain = ledger(false);
        let o = obs(candidates(&plain, true, true), &[&plain], &[]);
        let r = check(&o, CheckId::SplitBrain);
        let f = found(&r);
        assert!(f.evidence.contains("expose an exclusive resource"), "{f:?}");
        assert!(!f.evidence.contains("redundancy.v1 governs"), "{f:?}");
        // A holder of a governed interface and a standby holding nothing:
        // no split.
        let o = obs(candidates(&l, true, false), &[&l], &[]);
        clean(&check(&o, CheckId::SplitBrain));
    }

    #[test]
    fn token_missing_never_finds_a_standby_and_names_a_marked_holder() {
        // Core §6 (0.39): a standby exposes nothing and holds no token.
        let l = ledger(true);
        let o = obs(candidates(&l, true, false), &[&l], &[]);
        let r = check(&o, CheckId::TokenMissing);
        assert!(
            r.findings.iter().all(|f| !f.subject.contains(B)),
            "a standby is never token-missing: {r:#?}"
        );
        // A holder marked standby is still a holder, and its token is one
        // it must not hold (redundancy.v1 §2.15, marked_holding).
        let o = obs(candidates(&l, false, true), &[&l], &[]);
        let r = check(&o, CheckId::TokenMissing);
        let f = r
            .findings
            .iter()
            .find(|f| f.subject.contains(B))
            .unwrap_or_else(|| panic!("{r:#?}"));
        assert!(f.evidence.contains("marks standby"), "{f:?}");
        assert!(f.evidence.contains("marked_holding"), "{f:?}");
    }

    #[test]
    fn a_lasting_vacancy_of_a_governed_interface_is_a_warning() {
        // redundancy.v1 §2.15's second question: no holder in two complete
        // reads, and a candidate listing it.
        let l = ledger(true);
        let o = obs(candidates(&l, false, false), &[&l], &[]);
        let r = check(&o, CheckId::RedundancyVacancy);
        let f = found(&r);
        assert_eq!(
            (f.subject.as_str(), f.severity),
            ("lab/ledger ledger.v1", DoctorSeverity::Warning)
        );
        assert!(f.evidence.contains("lasting vacancy"), "{f:?}");
        // Held: no vacancy.
        let o = obs(candidates(&l, true, false), &[&l], &[]);
        let why = clean(&check(&o, CheckId::RedundancyVacancy));
        assert!(why.starts_with("1 interface(s)"), "{why}");
        // Held in the first read only: a takeover due, no finding.
        let mut o = obs(candidates(&l, false, false), &[&l], &[]);
        o.before = Some(Ok(candidates(&l, true, false)));
        clean(&check(&o, CheckId::RedundancyVacancy));
        // The second read not complete: not decided.
        let mut o = obs(candidates(&l, false, false), &[&l], &[]);
        after_mut(&mut o).complete = false;
        assert!(unseen(&check(&o, CheckId::RedundancyVacancy)).contains("presence_incomplete"));
        // Not governed: not this profile's, and no subject.
        let plain = ledger(false);
        let o = obs(candidates(&plain, false, false), &[&plain], &[]);
        assert!(clean(&check(&o, CheckId::RedundancyVacancy)).starts_with("0 interface(s)"));
        // The revision not in hand, the shape a vacancy's: undecided.
        let o = obs(candidates(&l, false, false), &[], &[&l]);
        assert!(unseen(&check(&o, CheckId::RedundancyVacancy)).contains("unavailable"));
    }

    #[test]
    fn a_recognised_standby_is_not_counted_by_hostid_duplicate() {
        // hostid.v1 §2.12 (0.6, 0.7), scenarios §12 step 6.
        let on = |l: &Contract, b: serde_json::Value| -> Observed {
            observed(
                &[
                    inst(SYSINFO, A),
                    inst(SYSINFO, B),
                    alive(SYSINFO, "ledger.v1", A, l),
                ],
                vec![
                    (
                        (SYSINFO, A),
                        minted_descriptor(A, Some("aa01"), &[entry(l)]),
                    ),
                    ((SYSINFO, B), minted_descriptor(B, Some("bb02"), &[b])),
                ],
            )
        };
        let l = ledger(true);
        let o = obs(on(&l, standby_entry(&l)), &[&l], &[]);
        let why = clean(&check(&o, CheckId::HostidDuplicate));
        assert!(why.contains("1 recognised standby"), "{why}");
        // The interface not governed: B is counted, and two zids are the
        // finding.
        let plain = ledger(false);
        let o = obs(on(&plain, standby_entry(&plain)), &[&plain], &[]);
        found(&check(&o, CheckId::HostidDuplicate));
        // B lists the governed interface serving and holds nothing, the
        // same in both reads: unmarked, a twin, counted.
        let mut o = obs(on(&l, entry(&l)), &[&l], &[]);
        o.before = o.after.clone();
        found(&check(&o, CheckId::HostidDuplicate));
        // Read once, its listing may be moving: unobservable.
        let o = obs(on(&l, entry(&l)), &[&l], &[]);
        assert!(unseen(&check(&o, CheckId::HostidDuplicate)).contains("in_transition"));
        // Whether it is governed cannot be read: unobservable, never clean.
        let o = obs(on(&l, standby_entry(&l)), &[], &[&l]);
        let why = unseen(&check(&o, CheckId::HostidDuplicate));
        assert!(
            why.contains("whether it is a standby is unobservable"),
            "{why}"
        );
    }
}
