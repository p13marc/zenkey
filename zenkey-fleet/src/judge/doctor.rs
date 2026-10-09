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
//! [`zk2::ownership::compare`]) and token-missing, whose findings must hold
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
//! **Not asked.** A check the spec leaves out is `NotAsked`, and so is
//! `state-stamp-foreign` without [`DoctorSpec::deep`]: its GETs cost the
//! owners' data plane, the frugality v1's `--deep` checks had.

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
    ROUTERS, STORAGES, admin_read, merge_storage_rows, router_from_admin_entry,
    storage_from_admin_entry,
};
use crate::bus::contracts::BundleStore;
use crate::bus::presence::Scope;
use crate::judge::common::FINDING_CAP;
use crate::model::catalog::{Catalog, ContractState, DescriptorRead, Observed, Revision};
use crate::model::examples::Examples;
use crate::report::{
    Asked, CheckId, CheckReport, DoctorFinding, DoctorPresence, DoctorReport, DoctorScope,
    DoctorSeverity, RouterInfo, StorageInfo, Unjudged,
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
    /// Ask `state-stamp-foreign`, whose GETs cost the owners' data plane.
    pub deep: bool,
    /// The checks to ask; the rest are `NotAsked`.
    pub checks: BTreeSet<CheckId>,
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
        }
    }

    /// Whether the run asks `check`.
    pub fn asks(&self, check: CheckId) -> bool {
        self.checks.contains(&check) && (check != CheckId::StateStampForeign || self.deep)
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
    /// The first presence read, tokens only.
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
}

/// The routers' admin space, as two reads found it.
#[derive(Debug, Clone, Default)]
pub struct AdminSpace {
    pub routers: Vec<RouterInfo>,
    pub storages: Vec<StorageInfo>,
    /// Whether both reads ended at the routers' final reply.
    pub complete: bool,
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
    /// Whether every GET ended at the final reply.
    pub complete: bool,
}

/// `RLIMIT_MEMLOCK`, as `zenkey::shm` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Memlock {
    /// The soft limit, in bytes.
    Limited(u64),
    /// Unlimited — or unreadable, which `zenkey::shm::memlock_limit` does
    /// not tell apart.
    Unlimited,
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
        Some(admin_space(&bus.raw, t).await)
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
            .then(|| zk2::shm::memlock_limit().map_or(Memlock::Unlimited, Memlock::Limited)),
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
}

async fn presence_phase(bus: &DoctorBus, store: &BundleStore, spec: &DoctorSpec) -> PresencePhase {
    let (s, t) = (&bus.session, spec.timeout);
    let scope = Scope::all();
    let before = if spec.asks(CheckId::SplitBrain) || spec.asks(CheckId::TokenMissing) {
        let read = crate::bus::presence::read_tokens(s, &scope, t)
            .await
            .map_err(|e| crate::one_line(&e));
        tokio::time::sleep(spec.grace).await;
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
    let wanted = Catalog::new(observed).wanted();
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
    let stamps = async {
        if !spec.asks(CheckId::StateStampForeign) {
            return BTreeMap::new();
        }
        let mut out = BTreeMap::new();
        for (addr, iface) in owners_in(observed) {
            let read = state_stamps(s, &addr, &iface, t).await;
            out.insert((addr, iface), read);
        }
        out
    };
    let (contracts, archives, stamps) = tokio::join!(contracts, archives, stamps);
    PresencePhase {
        before,
        after: Some(after),
        contracts,
        archives,
        stamps,
    }
}

/// Every address presence shows providing `archive.v1`, by token or by
/// descriptor.
fn archives_in(observed: &Observed) -> BTreeSet<Addr> {
    let archive = IfaceId::from_str(ARCHIVE).expect("archive.v1 is an interface id");
    let catalog = Catalog::new(observed);
    catalog
        .addresses()
        .filter(|a| catalog.provides(a, &archive))
        .cloned()
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
/// namespace. Either read failing is the whole answer failing.
async fn admin_space(raw: &Session, timeout: Duration) -> Result<AdminSpace, String> {
    let (routers, storages) = tokio::join!(
        admin_read(raw, ROUTERS, timeout),
        admin_read(raw, STORAGES, timeout)
    );
    let routers = routers.map_err(|e| crate::one_line(&e))?;
    let storages = storages.map_err(|e| crate::one_line(&e))?;
    Ok(AdminSpace {
        complete: routers.complete && storages.complete,
        routers: routers
            .entries
            .into_iter()
            .map(router_from_admin_entry)
            .collect(),
        storages: merge_storage_rows(
            storages
                .entries
                .iter()
                .filter_map(|e| storage_from_admin_entry(&e.key, &e.value))
                .collect(),
        ),
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
                _ => unreachable!("every check that reads presence is above"),
            }
        })
        .collect();
    let unobservable = match (&obs.after, &presence) {
        (Some(_), Err(why)) => Some(why.clone()),
        _ => None,
    };
    DoctorReport {
        scope: scope_of(obs),
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
    let how = if after.complete {
        ""
    } else {
        " (and the read ended at its timeout)"
    };
    format!(
        "no zk2 token visible to this reader in {ns} (`{}`){how}: a run over an empty scope \
         judged nothing, which is not a healthy deployment",
        after.selector
    )
}

fn scope_of(obs: &DoctorObservation) -> DoctorScope {
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
/// ended at its timeout and the check claims an absence.
fn incomplete(what: &str) -> Unjudged {
    unjudged(
        "presence",
        format!("a presence read ended at its timeout: it can miss a token, so {what} (§8.1)"),
    )
}

// ─── the checks that read presence ──────────────────────────────────────────

/// §6, through the runtime's own diagnosis ([`zk2::ownership::compare`])
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
    let d = zk2::ownership::compare(&before.tokens, &p.after.tokens, &descriptors, &refs);
    let candidates = zk2::ownership::candidates(&before.tokens, &p.after.tokens).len();
    let findings = d
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
    if !p.complete {
        undecided.push(incomplete("no split-brain seen is not none"));
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
    /// The descriptor does not say: a role a component's own manifest
    /// declares carries no `optional` (§3.3), or its contract could not
    /// be read.
    Unknown(String),
}

/// §3.2 R5 and the unbound-role rule: each role's bindings against the
/// providers this reader sees, through the runtime's own edges (R3).
fn binding_unsatisfied(p: &Presence<'_>) -> CheckReport {
    const C: CheckId = CheckId::BindingUnsatisfied;
    let descriptors = p.descriptors();
    let edges: BTreeSet<(String, String)> = zk2::presence::edges(&descriptors, &p.after.tokens)
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
                    "a presence read ended at its timeout: a provider may be present and unseen",
                ));
                continue;
            }
            let hidden: Vec<String> = undescribed
                .iter()
                .filter(|a| {
                    r.bindings
                        .iter()
                        .any(|b| zk2::consumer::Provider::parse(b).is_ok_and(|pat| pat.matches(a)))
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
        return Need::Unknown(
            "its component's manifest declares it, and a descriptor carries no `optional` \
             for such a role (§3.3)"
                .into(),
        );
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
                undecided.push(incomplete("an instance token unseen may be held"));
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
                        undecided.push(incomplete("a token unseen may be held"));
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
        undecided.push(incomplete("an archive unseen may be present"));
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
            zids.entry(a).or_default().insert(z.to_owned());
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
        owners += 1;
        replies += n;
        let mut wrong = Vec::new();
        for (clock, (count, keys)) in &stamps.by_clock {
            match clock {
                None => wrong.push(format!(
                    "{count} unstamped (e.g. {}): a reply MUST carry its mutation's timestamp",
                    keys.join(", ")
                )),
                Some(c) if !own.contains(c) => wrong.push(format!(
                    "{count} stamped by clock {c} (e.g. {}), not the owner's session",
                    keys.join(", ")
                )),
                Some(_) => {}
            }
        }
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
        } else if !stamps.complete {
            undecided.push(unjudged(
                subject,
                "the GET ended at its timeout: a reply unseen may be stamped elsewhere",
            ));
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
    if a.complete {
        format!(
            "no router answered `{ROUTERS}` through this reader: the admin space is disabled, \
             the mesh is peer-only, or access control denies it"
        )
    } else {
        format!("the admin read of `{ROUTERS}` ended at its timeout with no router")
    }
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
            let full = zenkey::grammar::with_base(&obs.namespace, rel);
            let ke = zenoh::key_expr::OwnedKeyExpr::try_from(full.clone()).ok()?;
            Some((full, ke))
        })
        .collect();
    let mut findings = Vec::new();
    let mut undecided = Vec::new();
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
    let mut undecided = Vec::new();
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
    let floor = zk2::shm::MEMLOCK_FLOOR;
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
        Some(Memlock::Unlimited) => CheckReport::of(
            C,
            vec![],
            vec![],
            "RLIMIT_MEMLOCK is unlimited, as zenkey::shm reads it (it reads an unreadable \
             limit the same way)",
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
        // A manifest role selecting nothing: its need cannot be told.
        let o = obs(bound(&c, None, &["h9/tc"]), &[&c], &[]);
        let r = check(&o, CheckId::BindingUnsatisfied);
        let f = found(&r);
        assert_eq!(f.subject, "ws/gui netif");
        assert_eq!(f.severity, DoctorSeverity::Warning);
        assert!(f.evidence.contains("visible to this reader"), "{f:?}");
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
            storages: vec![],
            complete: true,
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
        let report = judge(&o, &s);
        assert!(
            report
                .check(CheckId::StateStampForeign)
                .unwrap()
                .verdict
                .is_not_asked()
        );
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
                routers,
                storages,
                complete,
            })),
            ..Default::default()
        }
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
        let mut read = |clocks: &[Option<&str>]| {
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
            check(&o, CheckId::StateStampForeign)
        };
        let own = format!("zid-{A}");
        clean(&read(&[Some(&own)]));
        assert!(
            found(&read(&[Some("feed")]))
                .evidence
                .contains("clock feed")
        );
        assert!(found(&read(&[None])).evidence.contains("unstamped"));
        assert!(clean(&read(&[])).contains("no stamp to attribute"));
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
        clean(&at(Memlock::Limited(zk2::shm::MEMLOCK_FLOOR)));
        clean(&at(Memlock::Unlimited));
    }
}
