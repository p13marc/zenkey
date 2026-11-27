//! `desired.v1` (#840, DD) and `desired.v2` (#986's J4), the reads: one
//! target's roles against their authors' documents, read as §2.17's tool
//! reads them (`spec/profiles/desired/v1.md`, `v2.md`), through a session
//! in the deployment's namespace. Each role is read by the major its
//! author's revision lists (`desired.v2` §2.18,
//! [`crate::model::desired::RoleRead::settle`]).
//!
//! What one reading reads ([`observe`]), in order:
//! 1. whether the session reaches any other zenoh node
//!    ([`crate::bus::presence::cut_off`], core §8.1, 0.37): one cut off
//!    reads nothing, and says why;
//! 2. presence and each instance's descriptor (core §8.1, §3.3) on that
//!    session, which holds no liveliness subscriber (#830): the targets',
//!    one address's or every one's; then each author a binding names whose
//!    presence that read did not cover. Across a constrained face neither
//!    crosses, and the deployment's word stands for the target's
//!    descriptor (§2.17 step 2);
//! 3. the contract revisions the descriptors name (§8.4, through a
//!    [`BundleStore`] seeded with what is held offline): the target's, for
//!    its report resources by type identity (§2.10) and the requirement
//!    that declares each role, and the author's, for its declaration
//!    (§2.2), which with the binding gives the slice (§2.3);
//! 4. S4's GET of each target's report resources, its replies merged per
//!    key (observer §4.1; two instances' reports on one key are one
//!    reply) and each key's read as a `desired.v1.Report` (§2.10's
//!    reading, `zenkey_model::desired::decode`); and of the key of each
//!    role's `desired.v2` report, the member of the resource whose
//!    `desired.report` names the role (v2 §2.10), merged the same way and
//!    read as the packed `Report` under its envelope's tag;
//! 5. S4's GET of each slice from its author, present or across a face
//!    (§2.17 step 6): under `desired.v1` each reply's Zenoh timestamp,
//!    under `desired.v2` a member read's snapshot, its entry, its
//!    tombstone or no entry (v2 §2.5), and none at all for a `desired.v2`
//!    role with no revision of its author's in hand (v2 §2.17); and, for
//!    an author absent by a complete read, each archive's last-known of
//!    the slice, shown and never current (core §4.4, S6).
//!
//! Each step's reads are made at once, at most [`CONCURRENCY`] in flight
//! (review of #856): the authors' presence reads, the targets' report GETs
//! beside the archives' presence read, then every slice's GET and every
//! archive's last-known, so one reading costs about one `--timeout` a step,
//! never one per target or role.
//!
//! Nothing is judged here. [`run_desired`] makes the reading, once, or,
//! with a window, again every [`REREAD`] while a role's answer may still
//! move, judging each with [`crate::judge::desired`]: a target "not yet"
//! converged on one version of its author's for the whole window is behind
//! for at least that long (§2.17, text 0.4).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};

use futures_util::future::BoxFuture;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::descriptor::{Descriptor, RequireEntry};
use zenkey_model::desired::{self as dm, Listing, Seen};
use zenkey_model::grammar::{Addr, IfaceId};
use zenoh::Session;
use zenoh::sample::SampleKind;

use crate::bus::contracts::BundleStore;
use crate::bus::health::Stamped;
use crate::bus::presence::Scope;
use crate::model::catalog::{Catalog, ContractSet, Observed};
use crate::model::desired::{
    Archived, Behind, Contracts, DesiredRead, FaceWord, Major, MemberReply, Named, PackedReply,
    PackedUnreadable, ReportReply, ReportWhere, ReportsGet, RoleRead, RoleResolution, SliceFrom,
    SliceGet, SliceReply, TargetRead, author_contract, descriptor_in, members_selector,
    named_revision, packed_read, report_resources, requirement_names, resolve_role, resolve_unread,
    seen_in,
};
use crate::report::DesiredReport;

/// `archive.v1`'s interface id: its providers are the archives asked.
const ARCHIVE: &str = "archive.v1";

/// How long a run with a window waits between two readings.
pub const REREAD: Duration = Duration::from_millis(500);

/// What a reading asks.
#[derive(Debug, Clone)]
pub struct DesiredSpec {
    /// Each read's timeout: presence, a descriptor, a retrieval, a GET.
    pub timeout: Duration,
    /// The longest a target may be behind its author's current version
    /// (§2.17, text 0.4): the reading is made again every [`REREAD`] while
    /// a role's answer may still move, until this has passed. `None`: one
    /// reading, and "not yet" is never a finding.
    pub window: Option<Duration>,
    /// The operator's word that this host's clock and the owners' agree
    /// within the HLC delta (`freshness.v1` §2.6, ground 1): a report's
    /// stamp is aged against this clock, and, with a window, an author's
    /// version older than it by this clock is one a target is behind.
    pub clocks_synced: bool,
    /// One role asked of; `None`: every role on a desired resource.
    pub role: Option<String>,
    /// The one target sits across a constrained face (§2.14, §2.17 step
    /// 2): no presence and no descriptor is read; this is the deployment's
    /// word for them, and the contracts are the ones held offline.
    pub face: Option<FaceWord>,
    /// Read the archives' last-known of a slice whose author is absent.
    pub archives: bool,
    /// The deployment's catalogs (core §8.6): the epoch a `desired.v2`
    /// report's confirmation is dated under, where its target's contract
    /// uses `freshness.v2` (§2.17 step 4). None: such a report is undated,
    /// so its age is unobservable.
    pub catalogs: crate::model::deployment::Catalogs,
}

impl DesiredSpec {
    /// One reading, every role, archives read.
    pub fn new(timeout: Duration) -> DesiredSpec {
        DesiredSpec {
            timeout,
            window: None,
            clocks_synced: false,
            role: None,
            face: None,
            archives: true,
            catalogs: crate::model::deployment::Catalogs::default(),
        }
    }
}

/// The selector a target's presence is read through, base-relative.
pub fn presence_selector(target: Option<&Addr>) -> String {
    target.map_or_else(Scope::all, Scope::service).selector()
}

/// One run: the reading, again while a role's answer may move within the
/// window, each judged. `store` is seeded with `offline` first.
/// `on_read` is called after each reading, with how many have been made.
pub async fn run_desired(
    session: &Session,
    namespace: &str,
    target: Option<Addr>,
    spec: DesiredSpec,
    store: &BundleStore,
    offline: &ContractSet,
    mut on_read: impl FnMut(u32),
) -> DesiredReport {
    store.seed(offline);
    let start = Instant::now();
    let mut behind = Behind::default();
    let mut readings = 0;
    loop {
        store.forget_absences();
        let read = observe(session, target.as_ref(), &spec, store, offline).await;
        readings += 1;
        on_read(readings);
        let report = crate::judge::desired::judge(
            namespace,
            target.as_ref(),
            &spec,
            &read,
            &mut behind,
            readings,
        );
        let Some(window) = spec.window else {
            return report;
        };
        if report.settled() || read.began.saturating_duration_since(start) >= window {
            return report;
        }
        let next = (Instant::now() + REREAD).min(start + window);
        tokio::time::sleep_until(next.into()).await;
    }
}

/// One reading: the module doc's.
pub async fn observe(
    session: &Session,
    target: Option<&Addr>,
    spec: &DesiredSpec,
    store: &BundleStore,
    offline: &ContractSet,
) -> DesiredRead {
    observe_with(
        session,
        target,
        spec,
        store,
        offline,
        None,
        Contracts::new(),
    )
    .await
}

/// [`observe`] of every target, over a presence read a caller already made
/// of every service, its descriptors described, and the revisions it
/// already retrieved: the doctor's (#840, DD). Nothing is read again.
pub async fn observe_in(
    session: &Session,
    observed: Observed,
    contracts: Contracts,
    spec: &DesiredSpec,
    store: &BundleStore,
    offline: &ContractSet,
) -> DesiredRead {
    observe_with(
        session,
        None,
        spec,
        store,
        offline,
        Some(observed),
        contracts,
    )
    .await
}

async fn observe_with(
    session: &Session,
    target: Option<&Addr>,
    spec: &DesiredSpec,
    store: &BundleStore,
    offline: &ContractSet,
    given: Option<Observed>,
    contracts: Contracts,
) -> DesiredRead {
    let began = Instant::now();
    let mut read = DesiredRead {
        began,
        at: SystemTime::now(),
        cut_off: None,
        presence: None,
        targets: Vec::new(),
        asked: Vec::new(),
    };
    if let Some(why) = crate::bus::presence::cut_off(session).await {
        read.cut_off = Some(why);
        if let Some(a) = target {
            read.targets
                .push(unread(a, spec, Seen::Incomplete, read.cut_off.clone()));
        }
        return read;
    }
    let t = spec.timeout;
    let mut contracts = contracts;
    if let (Some(word), Some(a)) = (&spec.face, target) {
        read.targets.push(across_face(a, word, spec, offline));
    } else {
        let presence = match given {
            Some(o) => Ok(o),
            None => {
                read.asked.push(presence_selector(target));
                let scope = target.map_or_else(Scope::all, Scope::service);
                crate::bus::presence::observe(session, &scope, t)
                    .await
                    .map_err(|e| crate::one_line(&e))
            }
        };
        read.presence = Some(presence);
        let observed = read.presence.as_ref().and_then(|p| p.as_ref().ok());
        // The targets: the one asked of, or every service with an instance.
        let addresses: Vec<Addr> = match (target, observed) {
            (Some(a), _) => vec![a.clone()],
            (None, Some(o)) => {
                let c = Catalog::new(o);
                c.addresses()
                    .filter(|a| c.has_instance(a))
                    .cloned()
                    .collect()
            }
            (None, None) => Vec::new(),
        };
        // Their revisions, retrieved (§8.4).
        let mut descriptors = BTreeMap::new();
        for a in &addresses {
            let d = match observed {
                Some(o) if Catalog::new(o).has_instance(a) => descriptor_in(o, a),
                _ => Ok(None),
            };
            descriptors.insert(a.clone(), d);
        }
        let wanted: BTreeSet<(IfaceId, Fingerprint)> = descriptors
            .values()
            .filter_map(|d| d.as_ref().ok().cloned().flatten())
            .flat_map(|d| revisions(&d))
            .collect();
        fetch(session, store, &wanted, &mut contracts).await;
        // Each author the bindings name, its presence read when the
        // targets' read did not cover it.
        let mut uncovered: BTreeSet<Addr> = BTreeSet::new();
        for d in descriptors
            .values()
            .filter_map(|d| d.as_ref().ok().cloned().flatten())
        {
            for e in roles_of(&d, spec) {
                let Some(x) = one_author(&e.bindings) else {
                    continue;
                };
                if target.is_some() && target != Some(&x) {
                    uncovered.insert(x);
                }
            }
        }
        read.asked
            .extend(uncovered.iter().map(|x| Scope::service(x).selector()));
        // Each read at once (review of #856): one reading costs about one
        // --timeout, whatever the number of authors.
        let reads = bounded(
            uncovered
                .iter()
                .cloned()
                .map(|x| {
                    let s = session.clone();
                    Box::pin(async move {
                        crate::bus::presence::observe(&s, &Scope::service(&x), t)
                            .await
                            .map_err(|e| crate::one_line(&e))
                    }) as BoxFuture<'static, _>
                })
                .collect(),
        )
        .await;
        let authors: BTreeMap<Addr, Result<Observed, String>> =
            uncovered.into_iter().zip(reads).collect();
        let mut more = BTreeSet::new();
        for d in descriptors
            .values()
            .filter_map(|d| d.as_ref().ok().cloned().flatten())
        {
            for e in roles_of(&d, spec) {
                let (Some(x), Ok(iface)) =
                    (one_author(&e.bindings), IfaceId::from_str(&e.interface))
                else {
                    continue;
                };
                let o = authors.get(&x).or(read.presence.as_ref());
                if let Some(Ok(o)) = o {
                    for fp in Catalog::new(o).revisions_of(&x, &iface) {
                        more.insert((iface.clone(), fp));
                    }
                }
            }
        }
        fetch(session, store, &more, &mut contracts).await;
        for a in &addresses {
            let (seen, seen_why) = seen_in(read.presence.as_ref(), a);
            let descriptor = descriptors.remove(a).unwrap_or(Ok(None));
            let mut tr = TargetRead {
                addr: a.clone(),
                seen,
                seen_why,
                word: None,
                descriptor,
                roles: Vec::new(),
                reports: None,
            };
            if seen == Seen::Present {
                tr.roles = resolve_roles(&tr, spec, &contracts, offline, &read, &authors);
            }
            // Every service in the namespace: one with no desired role is no
            // target of this profile's, and is left out.
            let desired = tr
                .roles
                .iter()
                .any(|r| r.resolution.listing != Some(Listing::NotListed))
                || tr.descriptor.is_err();
            if target.is_none() && !desired {
                continue;
            }
            if tr.roles.is_empty() {
                let listing = match (&tr.seen, &tr.descriptor) {
                    (Seen::Present, Ok(Some(_))) => Some(Listing::NotListed),
                    _ => None,
                };
                tr.roles.push(RoleRead::new(RoleResolution::bare(
                    role_asked(spec),
                    listing,
                )));
            }
            read.targets.push(tr);
        }
        for tr in &mut read.targets {
            for r in &mut tr.roles {
                if let Some(x) = r.resolution.author.clone() {
                    let o = authors.get(&x).or(read.presence.as_ref());
                    r.author_seen = Some(seen_in(o, &x));
                }
            }
        }
    }
    // Every target's reports at once, and the archives beside them when an
    // author is absent; then every slice and every archive's last-known at
    // once (review of #856): one reading costs about one --timeout per
    // phase, not one per target or role.
    let plans: Vec<(usize, ReportPlan)> = read
        .targets
        .iter()
        .enumerate()
        .filter(|(_, tr)| {
            tr.roles.iter().any(|r| {
                !r.resolution.slice.is_empty()
                    || (r.resolution.listed()
                        && r.resolution.report_at.as_ref().is_some_and(|w| w.names()))
            })
        })
        .map(|(i, tr)| (i, report_selectors(tr, &contracts, offline)))
        .collect();
    for (_, p) in &plans {
        if let Ok(sels) = p {
            read.asked.extend(sels.iter().map(|s| s.selector.clone()));
        }
    }
    let needs_archives = spec.archives
        && read.targets.iter().any(|tr| {
            tr.roles.iter().any(|r| {
                !r.resolution.slice.is_empty() && matches!(r.author_seen, Some((Seen::Absent, _)))
            })
        });
    // desired.v2 §2.17 step 4: under freshness.v2, a report's snapshot
    // entry is dated by its confirmation, under the epoch its owner's
    // descriptor names (core §8.6).
    let epochs = match &read.presence {
        Some(Ok(observed)) => crate::bus::query::EntryEpochs::observed(&spec.catalogs, observed),
        _ => crate::bus::query::EntryEpochs::of_catalogs(&spec.catalogs),
    };
    let reports = bounded(
        plans
            .into_iter()
            .map(|(i, plan)| {
                let s = session.clone();
                let epochs = epochs.clone();
                Box::pin(async move {
                    (
                        i,
                        match plan {
                            Err(why) => Err(why),
                            Ok(sels) => Ok(reports_get(&s, sels, t, &epochs).await),
                        },
                    )
                }) as BoxFuture<'static, (usize, Result<ReportsGet, String>)>
            })
            .collect(),
    );
    let providers = async {
        if needs_archives {
            Some(archive_providers(session, t).await)
        } else {
            None
        }
    };
    let (reports, archives) = tokio::join!(reports, providers);
    for (i, r) in reports {
        read.targets[i].reports = Some(r);
    }
    // Each role's major, now that its target's reports are read
    // (desired.v2 §2.18): a desired.v2 role with no revision of its
    // author's in hand has no slice, so nothing more is read of it.
    for tr in &mut read.targets {
        let TargetRead { roles, reports, .. } = tr;
        for r in roles.iter_mut() {
            r.settle(reports.as_ref());
        }
    }
    let mut legs = Vec::new();
    for (i, tr) in read.targets.iter().enumerate() {
        for (j, r) in tr.roles.iter().enumerate() {
            if r.resolution.slice.is_empty() {
                continue;
            }
            let slice = slice_asked(&r.resolution, tr.reports.as_ref());
            match r.author_seen.as_ref().map(|(s, _)| *s) {
                Some(Seen::Present | Seen::AcrossFace) => {
                    read.asked.push(slice.clone());
                    legs.push((i, j, slice, Some(r.major)));
                }
                Some(Seen::Absent) if spec.archives => legs.push((i, j, slice, None)),
                _ => {}
            }
        }
    }
    let done = bounded(
        legs.into_iter()
            .map(|(i, j, slice, current)| {
                let s = session.clone();
                let at = archives.clone();
                Box::pin(async move {
                    let leg = if let Some(major) = current {
                        Leg::Current(match major {
                            Major::V1 => slice_get(&s, &slice, t).await,
                            Major::V2 => slice_get_v2(&s, &slice, t).await,
                        })
                    } else {
                        Leg::Archived(match at {
                            Some(Ok(at)) => Ok(archived(&s, &at, &slice, t).await),
                            Some(Err(why)) => Err(why),
                            None => Err("the archives were not read".to_owned()),
                        })
                    };
                    (i, j, leg)
                }) as BoxFuture<'static, (usize, usize, Leg)>
            })
            .collect(),
    )
    .await;
    for (i, j, leg) in done {
        let r = &mut read.targets[i].roles[j];
        match leg {
            Leg::Current(c) => r.current = Some(c),
            Leg::Archived(a) => r.archived = Some(a),
        }
    }
    read.at = SystemTime::now();
    let mut seen = BTreeSet::new();
    read.asked.retain(|s| seen.insert(s.clone()));
    read
}

/// How many reads of one phase a reading keeps in flight at once.
pub const CONCURRENCY: usize = 64;

/// `futs`, at most [`CONCURRENCY`] at once, their outputs in order. Each
/// is boxed and owns what it reads, so a reading's future stays `Send`
/// wherever it is spawned.
async fn bounded<T>(futs: Vec<BoxFuture<'static, T>>) -> Vec<T> {
    use futures_util::StreamExt as _;
    futures_util::stream::iter(futs)
        .buffered(CONCURRENCY)
        .collect()
        .await
}

/// One GET of a target's reports: a `desired.v1.Report` resource's members,
/// or the one key of a role's `desired.v2` report.
#[derive(Debug, Clone)]
struct ReportSelector {
    selector: String,
    horizon: zenkey_model::freshness::Horizon,
    /// The major its replies are read by.
    major: Major,
    /// A `desired.v2` report whose target's contract uses `freshness.v2`:
    /// aged by its snapshot entry's confirmation (§2.17 step 4).
    aged: bool,
}

/// A target's report selectors, or why there are none to read.
type ReportPlan = Result<Vec<ReportSelector>, String>;

/// One read of a role's: S4's GET of its slice, or the archives'
/// last-known of it.
enum Leg {
    Current(Result<SliceGet, String>),
    Archived(Result<Vec<Archived>, String>),
}

/// A target nothing could be read of: one row, its presence as given.
fn unread(a: &Addr, spec: &DesiredSpec, seen: Seen, why: Option<String>) -> TargetRead {
    TargetRead {
        addr: a.clone(),
        seen,
        seen_why: why,
        word: None,
        descriptor: Err("not read".to_owned()),
        roles: vec![RoleRead::new(RoleResolution::bare(role_asked(spec), None))],
        reports: None,
    }
}

/// The role a row stands for when nothing more is known: the one asked
/// of, or `*`.
fn role_asked(spec: &DesiredSpec) -> String {
    spec.role.clone().unwrap_or_else(|| "*".to_owned())
}

/// The one exact address `bindings` name.
fn one_author(bindings: &[String]) -> Option<Addr> {
    let [one] = bindings else { return None };
    if one.split('/').any(|c| c == "*") {
        return None;
    }
    one.parse().ok()
}

/// The revisions a descriptor names.
fn revisions(d: &Descriptor) -> Vec<(IfaceId, Fingerprint)> {
    d.interfaces
        .iter()
        .filter_map(|e| {
            Some((
                IfaceId::from_str(&e.iface).ok()?,
                Fingerprint::parse(&e.contract).ok()?,
            ))
        })
        .collect()
}

/// The `requires` entries asked of: `spec.role`'s, or every one.
fn roles_of<'d>(d: &'d Descriptor, spec: &DesiredSpec) -> Vec<&'d RequireEntry> {
    d.requires
        .iter()
        .filter(|e| spec.role.as_ref().is_none_or(|r| *r == e.role))
        .collect()
}

/// Retrieves each revision of `wanted` not asked yet (§8.4).
async fn fetch(
    session: &Session,
    store: &BundleStore,
    wanted: &BTreeSet<(IfaceId, Fingerprint)>,
    into: &mut Contracts,
) {
    let wanted: Vec<(IfaceId, Fingerprint)> = wanted
        .iter()
        .filter(|w| !into.contains_key(w))
        .cloned()
        .collect();
    let got = store.fetch_all(session, &wanted).await;
    for (w, g) in wanted.into_iter().zip(got) {
        into.insert(w, g.map_err(|e| crate::one_line(&e)));
    }
}

/// A target's roles on desired resources (§2.3), resolved: from its
/// descriptor, the contract declaring each role, and the author's.
fn resolve_roles(
    t: &TargetRead,
    spec: &DesiredSpec,
    contracts: &Contracts,
    offline: &ContractSet,
    read: &DesiredRead,
    authors: &BTreeMap<Addr, Result<Observed, String>>,
) -> Vec<RoleRead> {
    let Ok(Some(d)) = &t.descriptor else {
        return Vec::new();
    };
    // A target of this profile holds a report resource (§2.10), of
    // either major: typed `desired.v1.Report`, or named by `desired.report`.
    let reporting = d.interfaces.iter().any(|e| {
        named_revision(d, &e.iface, contracts, offline).is_some_and(|r| {
            !report_resources(r.contract()).is_empty()
                || r.contract().resources.iter().any(|x| {
                    x.annotations
                        .contains_key(zenkey_model::desired::v2::REPORT_KEY)
                })
        })
    });
    // Whether every revision it names is in hand: when one is not, a role
    // whose author's contract is not either may be desired, and cannot be
    // told apart from one that is not.
    let all_held = d
        .interfaces
        .iter()
        .all(|e| named_revision(d, &e.iface, contracts, offline).is_some());
    let mut out = Vec::new();
    for e in roles_of(d, spec) {
        // What the requirement names, from its declaring contract; unread
        // when that is not in hand, never every resource (text 0.8, F-225).
        let names = requirement_names(d, e, contracts, offline);
        let templates = names.templates();
        let author = one_author(&e.bindings);
        let observed = author
            .as_ref()
            .and_then(|x| authors.get(x).or(read.presence.as_ref()))
            .and_then(|o| o.as_ref().ok())
            .map(Catalog::new);
        let author_c = IfaceId::from_str(&e.interface).ok().and_then(|iface| {
            author_contract(
                observed.as_ref(),
                author.as_ref(),
                &iface,
                templates,
                contracts,
                offline,
            )
        });
        let held = author_c.is_some();
        let resolved = match &names {
            Named::Unread(by) => resolve_unread(e, by, author_c),
            _ => resolve_role(&t.addr, e, templates, author_c),
        };
        // Where desired.v2 finds the role's report (v2 §2.10): by the
        // contract that declares the role, when it is in hand.
        let report_at = e
            .declared_by
            .as_deref()
            .and_then(|by| named_revision(d, by, contracts, offline))
            .filter(|rev| rev.contract().requires.contains_key(&e.role))
            .map(|rev| ReportWhere::of(&t.addr, rev.contract(), &e.role));
        match resolved {
            Some(mut res) if held || reporting || !all_held => {
                res.report_at = report_at;
                out.push(RoleRead::new(res));
            }
            _ if spec.role.is_some() => out.push(RoleRead::new(RoleResolution::bare(
                e.role.clone(),
                Some(Listing::NotListed),
            ))),
            _ => {}
        }
    }
    if out.is_empty()
        && let Some(r) = &spec.role
    {
        out.push(RoleRead::new(RoleResolution::bare(
            r.clone(),
            Some(Listing::NotListed),
        )));
    }
    out
}

/// Across a constrained face (§2.14, §2.17 step 2): the deployment's word
/// for the descriptor, the role resolved from the contracts held offline,
/// the one declaring the role and the author's.
fn across_face(a: &Addr, word: &FaceWord, spec: &DesiredSpec, offline: &ContractSet) -> TargetRead {
    let role = role_asked(spec);
    let mut t = TargetRead {
        addr: a.clone(),
        seen: Seen::AcrossFace,
        seen_why: None,
        word: Some(word.clone()),
        descriptor: Ok(None),
        roles: Vec::new(),
        reports: None,
    };
    let none = Contracts::new();
    let declaring = offline
        .iter()
        .find(|r| r.contract().requires.contains_key(&role));
    let mut res = match declaring {
        Some(rev) => {
            let req = &rev.contract().requires[&role];
            let entry = RequireEntry {
                role: role.clone(),
                interface: req.interface.to_string(),
                declared_by: Some(rev.iface().to_string()),
                bindings: word.author.iter().map(Addr::to_string).collect(),
                params: word.params.clone(),
                optional: req.optional,
            };
            let author_c = author_contract(
                None,
                word.author.as_ref(),
                &req.interface,
                req.resources.as_deref(),
                &none,
                offline,
            );
            // A revision of the author's held offline that declares no
            // desired resource the requirement names shows the role is no
            // target's, whatever the deployment's word (text 0.6, F-217):
            // the one `None` of `resolve_role`, which only a revision in
            // hand gives. With none in hand, the word's listing stands.
            let held = author_c.is_some();
            match resolve_role(a, &entry, req.resources.as_deref(), author_c) {
                Some(r) => r,
                None => RoleResolution::bare(
                    role.clone(),
                    Some(if held {
                        Listing::NotListed
                    } else {
                        word.listing
                    }),
                ),
            }
        }
        None => RoleResolution::bare(role.clone(), Some(word.listing)),
    };
    if res.listing != Some(Listing::NotListed) {
        res.listing = Some(word.listing);
    }
    res.report_at = declaring.map(|rev| ReportWhere::of(a, rev.contract(), &role));
    let author_seen = word.author.as_ref().map(|_| {
        if word.author_across {
            (Seen::AcrossFace, None)
        } else {
            (
                Seen::Incomplete,
                Some("its presence is read only when the target's is".to_owned()),
            )
        }
    });
    let mut role = RoleRead::new(res);
    role.author_seen = author_seen;
    t.roles.push(role);
    t
}

/// The selectors of `t`'s report resources (§2.10), from the revisions its
/// descriptor names, or across a face every one held offline; `Err` when
/// one it names is not in hand and none found holds a report.
fn report_selectors(t: &TargetRead, contracts: &Contracts, offline: &ContractSet) -> ReportPlan {
    let v1 = |selector: String, r: &zenkey_model::contract::Resource| ReportSelector {
        selector,
        horizon: zenkey_model::freshness::horizon_of(r),
        major: Major::V1,
        aged: false,
    };
    // desired.v2 (§2.10): the key of each listed role's report, the member
    // of the resource whose `desired.report` names the role; an unbound
    // role's too, which holds the baseline's record (§2.12).
    let mut out: Vec<ReportSelector> = t
        .roles
        .iter()
        .filter(|r| r.resolution.listed())
        .filter_map(|r| r.resolution.report_at.as_ref())
        .filter_map(|w| {
            Some(ReportSelector {
                selector: w.key()?,
                horizon: w.horizon.clone(),
                major: Major::V2,
                aged: w.aged,
            })
        })
        .collect();
    let mut missing = Vec::new();
    match &t.descriptor {
        Ok(Some(d)) => {
            for e in &d.interfaces {
                match named_revision(d, &e.iface, contracts, offline) {
                    Some(rev) => {
                        for r in report_resources(rev.contract()) {
                            out.push(v1(members_selector(&t.addr, rev.iface(), r), r));
                        }
                    }
                    None => missing.push(format!("{} {}", e.iface, e.contract)),
                }
            }
        }
        _ => {
            for rev in offline.iter() {
                for r in report_resources(rev.contract()) {
                    out.push(v1(members_selector(&t.addr, rev.iface(), r), r));
                }
            }
        }
    }
    if out.is_empty() && !missing.is_empty() {
        return Err(format!(
            "its contract revision(s) not in hand, so no report resource is known: {} (§8.4)",
            missing.join(", ")
        ));
    }
    let mut seen = BTreeSet::new();
    out.retain(|s| seen.insert(s.selector.clone()));
    Ok(out)
}

/// One report resource's replies as §2.10 reads them (desired.v1 0.14;
/// observer §4.1's MUST): merged per key first (`bus::query::merged`), so
/// two instances' reports on one key are one reply, within one run the
/// higher seq and across two runs the later arrival, never two that
/// differ. Then the reply each key kept is read as a `desired.v1.Report`,
/// and a key whose kept reply is a delete holds no report.
fn report_replies(
    samples: Vec<zenoh::sample::Sample>,
    horizon: &zenkey_model::freshness::Horizon,
) -> Vec<ReportReply> {
    let mut out = Vec::new();
    for sample in crate::bus::query::merged(samples) {
        // After the merge, never before it: a delete newer than a put
        // leaves no report on the key.
        if sample.kind() == SampleKind::Delete {
            continue;
        }
        let key = sample.key_expr().as_str().to_owned();
        let stamp = sample.timestamp().map(|ts| Stamped {
            time: ts.get_time().to_system_time(),
            clock: ts.get_id().to_string(),
        });
        // The envelope first (core §7.5): a report is its record, a
        // refused envelope unreadable, its stamp still the Zenoh one, the
        // report's time.
        let read = match crate::model::inband::read(sample) {
            crate::model::inband::Read::Refused { refusal, .. } => {
                Err(dm::Unreadable::Json(refusal.reason()))
            }
            r => dm::decode(&r.sample().payload().to_bytes()),
        };
        out.push(ReportReply {
            key,
            read,
            stamp,
            horizon: horizon.clone(),
        });
    }
    out
}

/// One `desired.v2` report key's replies as v2 §2.10 reads them: merged
/// per key first, as [`report_replies`] merges `desired.v1`'s (observer
/// §4.1: two instances' reports on one key are one reply, within one run
/// the higher seq, across two runs the later arrival), a delete skipped
/// after the merge. Then the reply the key kept is read as the packed
/// `Report` under the tag its envelope's CH carried
/// ([`packed_read`]): a refused envelope, or a record core §7.6 refuses, is
/// unreadable with its token, never guessed at. A reply on any key but
/// `key` is not this role's report.
fn packed_replies(
    samples: Vec<zenoh::sample::Sample>,
    key: &str,
    horizon: &zenkey_model::freshness::Horizon,
) -> Vec<PackedReply> {
    let mut out = Vec::new();
    for sample in crate::bus::query::merged(samples) {
        if sample.kind() == SampleKind::Delete || sample.key_expr().as_str() != key {
            continue;
        }
        let stamp = sample.timestamp().map(|ts| Stamped {
            time: ts.get_time().to_system_time(),
            clock: ts.get_id().to_string(),
        });
        let (read, inband) = match zenkey::inband::read(sample) {
            zenkey::inband::Read::Enveloped { sample, stamp, tag } => (
                packed_read(Some(tag.get()), &sample.payload().to_bytes()),
                Some(stamp),
            ),
            zenkey::inband::Read::Plain(sample) => {
                (packed_read(None, &sample.payload().to_bytes()), None)
            }
            zenkey::inband::Read::Refused { refusal, .. } => (
                Err(PackedUnreadable::new(refusal.token(), &refusal.reason())),
                None,
            ),
            // A heartbeat, a frame or a page on a report's key: never a
            // record of the key's type.
            other => (
                Err(PackedUnreadable::new(
                    zenkey_model::heartbeat::FormRefusal::TOKEN,
                    &format!(
                        "the reply carries no mutation's record ({})",
                        other
                            .undecodable()
                            .map(|u| u.to_string())
                            .unwrap_or_else(|| "another form".to_owned())
                    ),
                )),
                None,
            ),
        };
        out.push(PackedReply {
            key: key.to_owned(),
            read,
            inband,
            stamp,
            horizon: horizon.clone(),
        });
    }
    out
}

/// S4's GET of each report resource (core §4.2), its replies merged per
/// key and each key's read as a `desired.v1.Report` (§2.10,
/// [`report_replies`]) or, on a `desired.v2` report's key, as the packed
/// `Report` (v2 §2.10, [`packed_replies`]). A reply on a wildcard key is
/// no member (R6).
async fn reports_get(
    session: &Session,
    selectors: Vec<ReportSelector>,
    t: Duration,
    epochs: &crate::bus::query::EntryEpochs,
) -> ReportsGet {
    let mut out = ReportsGet {
        selectors: selectors.iter().map(|s| s.selector.clone()).collect(),
        complete: true,
        replies: Vec::new(),
        packed: Vec::new(),
        at: SystemTime::now(),
    };
    // Each resource's GET at once.
    let gets = bounded(
        selectors
            .into_iter()
            .map(
                |ReportSelector {
                     selector: sel,
                     horizon,
                     major,
                     aged,
                 }| {
                    let session = session.clone();
                    let epochs = epochs.clone();
                    Box::pin(async move {
                        let mut replies_out = Vec::new();
                        let mut packed_out = Vec::new();
                        let mut complete = true;
                        let replies = match crate::bus::query::state_get(
                            &session,
                            &sel,
                            &crate::bus::query::GetOpts::new(t),
                        )
                        .await
                        {
                            Ok(r) => r,
                            Err(_) => return (false, replies_out, packed_out),
                        };
                        let mut samples = Vec::new();
                        while let Ok(reply) = replies.recv_async().await {
                            let Ok(sample) = reply.result() else {
                                complete = false;
                                continue;
                            };
                            if sample.key_expr().as_str().contains('*') {
                                continue;
                            }
                            // A snapshot's page is its entries (core §4.2;
                            // #977's E3): under freshness.v2, each dated by
                            // its confirmation (§2.17 step 4, freshness.v2
                            // §2.6), what ages the report.
                            if aged {
                                samples.extend(
                                    crate::bus::query::reply_entries_aged(sample, &epochs)
                                        .into_iter()
                                        .map(|e| e.sample),
                                );
                            } else {
                                samples.extend(crate::bus::query::reply_samples(sample));
                            }
                        }
                        match major {
                            Major::V1 => replies_out.extend(report_replies(samples, &horizon)),
                            Major::V2 => packed_out.extend(packed_replies(samples, &sel, &horizon)),
                        }
                        (complete, replies_out, packed_out)
                    }) as BoxFuture<'static, _>
                },
            )
            .collect(),
    )
    .await;
    for (complete, replies, packed) in gets {
        out.complete &= complete;
        out.replies.extend(replies);
        out.packed.extend(packed);
    }
    out.at = SystemTime::now();
    out
}

/// The key a role's slice is asked on: its one key, or of a requirement's
/// two candidates the one its report names (else the first).
fn slice_asked(res: &RoleResolution, reports: Option<&Result<ReportsGet, String>>) -> String {
    if res.slice.len() > 1
        && res.slice_from == Some(SliceFrom::Requirement)
        && let Some(Ok(g)) = reports
        && let Some(k) = g.replies.iter().find_map(|r| {
            let k = r.read.as_ref().ok()?.key.as_ref()?;
            res.slice.contains(k).then(|| k.clone())
        })
    {
        return k;
    }
    res.slice[0].clone()
}

/// S4's GET of one slice from its author (core §4.2): target `All`,
/// consolidation `None`, through the crate's one state GET, every reply
/// merged per key by its stamp, never the last to arrive
/// (`bus::query::merged`).
async fn slice_get(session: &Session, slice: &str, t: Duration) -> Result<SliceGet, String> {
    let replies = crate::bus::query::state_get(session, slice, &crate::bus::query::GetOpts::new(t))
        .await
        .map_err(|e| format!("GET {slice}: {}", crate::one_line(&e)))?;
    let mut out = SliceGet {
        selector: slice.to_owned(),
        complete: true,
        replies: Vec::new(),
        member: Vec::new(),
        at: SystemTime::now(),
    };
    let mut samples = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            out.complete = false;
            continue;
        };
        if sample.key_expr().as_str().contains('*') {
            continue;
        }
        // A snapshot's entry stands for its reply (core §4.2; #977's E3).
        samples.extend(crate::bus::query::reply_samples(sample));
    }
    for sample in crate::bus::query::merged(samples) {
        out.replies.push(SliceReply {
            key: sample.key_expr().as_str().to_owned(),
            delete: sample.kind() == SampleKind::Delete,
            stamp: sample
                .timestamp()
                .map(|ts| (zenkey::desired::stamp(ts), ts.get_time().to_system_time())),
        });
    }
    out.at = SystemTime::now();
    Ok(out)
}

/// S4's GET of one `desired.v2` slice from its author (v2 §2.5, §2.17 step
/// 6): a member read of one concrete key, with no held stamp, so it is
/// answered whole. Each reply on the key (R6) is a snapshot's page: its
/// entry on the key, a live one or a tombstone, with its in-band stamp, or
/// no entry, the author's word that it holds no document, under the page's
/// run. The entries are merged per key (observer §4.1). A per-key reply (a
/// `redundancy.v1` author's, v2 §2.4) is read by its envelope alike; a
/// reply with no envelope carries no version; a refused one, and a held
/// stamp, which this reader did not send (core §4.2), are no answer.
async fn slice_get_v2(session: &Session, slice: &str, t: Duration) -> Result<SliceGet, String> {
    let replies = crate::bus::query::state_get(session, slice, &crate::bus::query::GetOpts::new(t))
        .await
        .map_err(|e| format!("GET {slice}: {}", crate::one_line(&e)))?;
    let mut out = SliceGet {
        selector: slice.to_owned(),
        complete: true,
        replies: Vec::new(),
        member: Vec::new(),
        at: SystemTime::now(),
    };
    let mut samples = Vec::new();
    while let Ok(reply) = replies.recv_async().await {
        match reply.result() {
            Ok(sample) if sample.key_expr().as_str() == slice => samples.push(sample.clone()),
            // R6: a reply on any key but the slice's is discarded.
            Ok(_) => {}
            Err(_) => out.complete = false,
        }
    }
    out.member = member_replies(samples, slice);
    out.at = SystemTime::now();
    Ok(out)
}

/// What the replies on a `desired.v2` slice's key said ([`slice_get_v2`]),
/// in arrival order: each reply that gave no version, each snapshot with
/// no entry, then the one entry or tombstone the key kept after the merge.
fn member_replies(samples: Vec<zenoh::sample::Sample>, slice: &str) -> Vec<MemberReply> {
    let mut out = Vec::new();
    let mut entries = Vec::new();
    for s in samples {
        if let Some(h) = zenkey::inband::held(&s) {
            out.push(MemberReply::Refused(format!(
                "held: a held stamp, {}, which this reader did not send (core §4.2)",
                zenkey_model::desired::v2::version_text(h.stamp)
            )));
            continue;
        }
        match zenkey::inband::read(s.clone()) {
            zenkey::inband::Read::Snapshot {
                sample,
                page: Ok(page),
            } => {
                let mine: Vec<_> = zenkey::inband::replies(&sample, &page)
                    .into_iter()
                    .filter(|m| m.key_expr().as_str() == slice)
                    .collect();
                if !mine.is_empty() {
                    entries.extend(mine);
                } else if page.index == 0 && !page.more {
                    // A member's snapshot is one page: with no entry, the
                    // author's word that it holds no document.
                    out.push(MemberReply::NoEntry {
                        run: page.run,
                        asof: page.asof,
                    });
                }
            }
            zenkey::inband::Read::Snapshot {
                page: Err(refusal), ..
            } => out.push(MemberReply::Refused(format!(
                "a snapshot's page whose body is refused: {}",
                refusal.token()
            ))),
            zenkey::inband::Read::Refused { refusal, .. } => {
                out.push(MemberReply::Refused(refusal.reason()));
            }
            zenkey::inband::Read::Plain(sample) if sample.kind() == SampleKind::Put => {
                out.push(MemberReply::Unstamped);
            }
            // A per-key reply in its envelope, and a `reply_del`.
            _ => entries.push(s),
        }
    }
    for sample in crate::bus::query::merged(entries) {
        let delete = sample.kind() == SampleKind::Delete;
        match zenkey::inband::read(sample).stamp() {
            Some(v) if delete => out.push(MemberReply::Tombstone(v)),
            Some(v) => out.push(MemberReply::Entry(v)),
            None => out.push(MemberReply::Unstamped),
        }
    }
    out
}

/// The archives presence shows, by token or descriptor (core §4.4).
async fn archive_providers(session: &Session, t: Duration) -> Result<Vec<Addr>, String> {
    let all = crate::bus::presence::observe(session, &Scope::all(), t)
        .await
        .map_err(|e| crate::one_line(&e))?;
    let archive = IfaceId::from_str(ARCHIVE).expect("archive.v1 is an interface id");
    Ok(Catalog::new(&all).providers_of(&archive))
}

/// Each archive's last-known of `slice` (core §4.4, S5): an explicit read,
/// never current.
async fn archived(session: &Session, at: &[Addr], slice: &str, t: Duration) -> Vec<Archived> {
    // Each archive at once.
    let read = bounded(
        at.iter()
            .cloned()
            .map(|a| {
                let s = session.clone();
                let slice = slice.to_owned();
                Box::pin(async move {
                    match zenkey::archive::last_known(&s, &a, &slice, t).await {
                        Ok(Some(lk)) => Some(Archived {
                            deleted: lk.value.is_none(),
                            stamp: lk.timestamp.as_ref().map(zenkey::desired::stamp),
                            inband: lk.inband,
                            revision: lk.value.as_deref().and_then(dm::revision_of_text),
                            confirmed: lk.confirmed,
                            archive: a,
                        }),
                        _ => None,
                    }
                }) as BoxFuture<'static, _>
            })
            .collect(),
    )
    .await;
    read.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;
    use zenkey_model::contract::{Contract, load_path, load_str};

    fn dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts")
    }

    fn file(name: &str) -> Contract {
        let l = load_path(&dir().join(format!("{name}.toml")));
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn set(contracts: Vec<Contract>) -> ContractSet {
        let mut s = ContractSet::new();
        for c in contracts {
            s.insert(crate::model::catalog::Revision::from_contract(
                c,
                crate::report::ContractSource::File,
            ));
        }
        s
    }

    /// desired.v2 0.2 §2.17 step 4 (J4's first leftover): a report whose
    /// target's contract uses freshness.v2 carries no Zenoh timestamp
    /// (core §4.3), so its snapshot entry's confirmation ages it, dated
    /// under the deployment epoch and stamped under its owner's clock
    /// (freshness.v2 §2.6): fresh within its horizon, stale past it. With
    /// no epoch in hand, and under confirmation 0, it is undated, so its
    /// age is unobservable, never fresh.
    #[test]
    fn a_v2_report_under_freshness_v2_is_aged_by_its_confirmation() {
        use zenkey_model::collection::{Confirmation, Entry, Page, Time};
        use zenkey_model::desired::v2::{self as d2, Report};
        use zenkey_model::freshness::{Horizon, Verdict};
        use zenkey_model::wire::{Run, Seq, Tag};
        use zenoh::bytes::Encoding;
        use zenoh::sample::{Sample, SampleBuilder};

        const RP: &str = "zk2/lab/worker/worker.v2/state/desired/task";
        const EPOCH: i64 = 1_767_225_600;
        let page = |confirmation: Confirmation| -> Sample {
            let p = Page {
                tag: Tag::new(d2::TAG).unwrap(),
                run: Run::U16(7),
                asof: Seq::new(1).unwrap(),
                time: Time::Known { s: 100, ms: 0 },
                index: 0,
                more: false,
                coll: 0,
                live: vec![Entry {
                    id: String::new(),
                    run: None,
                    seq: Seq::new(1).unwrap(),
                    confirmation,
                    record: Report::baseline().write().unwrap().bytes,
                }],
                tombs: Vec::new(),
            };
            SampleBuilder::put(
                zenoh::key_expr::KeyExpr::try_from(RP).unwrap(),
                p.write().unwrap(),
            )
            .encoding(Encoding::ZENOH_BYTES)
            .into()
        };
        let horizon = Horizon::Within(Duration::from_secs(60));
        let read = |sample: &Sample, epoch: Option<i64>| -> PackedReply {
            let dated: Vec<Sample> = crate::bus::query::reply_entries_at(sample, epoch)
                .into_iter()
                .map(|e| e.sample)
                .collect();
            let mut got = packed_replies(dated, RP, &horizon);
            assert_eq!(got.len(), 1);
            let p = got.remove(0);
            assert_eq!(p.read.as_ref().unwrap().report, Some(Report::baseline()));
            p
        };
        let built = std::time::UNIX_EPOCH + Duration::from_secs(EPOCH as u64 + 100);
        let verdict = |p: &PackedReply, at: SystemTime| {
            crate::model::desired::reply_freshness(
                p.stamp.as_ref(),
                &p.horizon,
                at,
                &crate::model::health::Trust::Word,
            )
            .verdict
        };
        // Confirmation 1: at the snapshot's time, under the owner's clock.
        let p = read(&page(Confirmation::AtTime), Some(EPOCH));
        let stamp = p.stamp.clone().expect("dated by its confirmation");
        assert_eq!(stamp.time, built);
        assert_eq!(stamp.clock, crate::bus::query::owner_clock(RP).to_string());
        assert_eq!(verdict(&p, built + Duration::from_secs(30)), Verdict::Fresh);
        assert_eq!(verdict(&p, built + Duration::from_secs(90)), Verdict::Stale);
        // Confirmation 2: at its conf.
        let at = Time::Known { s: 40, ms: 0 };
        let p = read(&page(Confirmation::At(at)), Some(EPOCH));
        assert_eq!(
            p.stamp.unwrap().time,
            std::time::UNIX_EPOCH + Duration::from_secs(EPOCH as u64 + 40)
        );
        // No epoch, and confirmation 0: undated, unobservable.
        for p in [
            read(&page(Confirmation::AtTime), None),
            read(&page(Confirmation::None), Some(EPOCH)),
        ] {
            assert!(p.stamp.is_none());
            assert_eq!(verdict(&p, built), Verdict::Unobservable);
        }
    }

    /// desired.v1 0.14 §2.10 and observer §4.1 (zk2py round 112's F-545):
    /// two instances' reports on one key are one reply after the merge,
    /// in each shape and in both orders, where every reply used to reach
    /// §2.10's rule on two report resources and answer `two_differ`.
    /// Within one run the higher seq is kept, across two runs the later
    /// arrival, of two with no stamp the later arrival; a newer delete
    /// leaves no report on its key, and another key keeps its own.
    #[test]
    fn two_reports_on_one_key_are_merged_before_they_are_read() {
        use zenkey_model::desired::Source;
        use zenkey_model::freshness::Horizon;
        use zenkey_model::wire::{DeleteStamp, Envelope, Run, Seq, Stamp, Tag};
        use zenoh::bytes::Encoding;
        use zenoh::sample::{Sample, SampleBuilder};

        const RP: &str = "zk2/lab/worker/worker.v1/state/desired/task";
        const OTHER: &str = "zk2/lab/worker/worker2.v1/state/desired/task";
        const SLICE: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";
        let stamp = |run: u16, seq: u32| Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        };
        let ke = |k: &str| zenoh::key_expr::KeyExpr::try_from(k.to_owned()).unwrap();
        let doc = |source: &str| format!(r#"{{"key":"{SLICE}","source":"{source}"}}"#).into_bytes();
        // A report as its target puts it: its record under its run and seq.
        let put = |key: &str, at: Stamp, source: &str| -> Sample {
            let payload = Envelope::mutation(Tag::NONE, at, &doc(source)).write();
            SampleBuilder::put(ke(key), payload)
                .encoding(Encoding::ZENOH_BYTES)
                .into()
        };
        // One with no envelope and no Zenoh timestamp.
        let bare = |source: &str| -> Sample {
            SampleBuilder::put(ke(RP), doc(source))
                .encoding(Encoding::APPLICATION_JSON)
                .into()
        };
        let delete = |at: Stamp| -> Sample {
            SampleBuilder::delete(ke(RP))
                .attachment(DeleteStamp { stamp: at }.write())
                .into()
        };
        let sources = |samples: Vec<Sample>| -> Vec<(String, Source)> {
            report_replies(samples, &Horizon::Undeclared)
                .into_iter()
                .map(|r| (r.key, r.read.expect("a report").source))
                .collect()
        };
        let one = |source: Source| vec![(RP.to_owned(), source)];

        // One run, seqs 1 and 2, differing: seq 2's, in both orders.
        let (first, second) = (
            put(RP, stamp(7, 1), "baseline"),
            put(RP, stamp(7, 2), "call"),
        );
        assert_eq!(
            sources(vec![first.clone(), second.clone()]),
            one(Source::Call)
        );
        assert_eq!(sources(vec![second, first.clone()]), one(Source::Call));
        // Two runs, which nothing orders: the later arrival, in both orders.
        let other_run = put(RP, stamp(8, 1), "call");
        assert_eq!(
            sources(vec![first.clone(), other_run.clone()]),
            one(Source::Call)
        );
        assert_eq!(
            sources(vec![other_run, first.clone()]),
            one(Source::Baseline)
        );
        // No envelope and no stamp: the later arrival, in both orders.
        assert_eq!(
            sources(vec![bare("baseline"), bare("call")]),
            one(Source::Call)
        );
        assert_eq!(
            sources(vec![bare("call"), bare("baseline")]),
            one(Source::Baseline)
        );
        // A delete is skipped after the merge: a newer one leaves no
        // report on the key, an older one the report.
        assert_eq!(
            sources(vec![first.clone(), delete(stamp(7, 2))]),
            Vec::new()
        );
        assert_eq!(
            sources(vec![delete(stamp(7, 2)), first.clone()]),
            Vec::new()
        );
        assert_eq!(
            sources(vec![put(RP, stamp(7, 3), "call"), delete(stamp(7, 2))]),
            one(Source::Call)
        );
        // Two report resources keep a reply each: §2.10's rule on two that
        // differ is theirs to meet.
        assert_eq!(
            sources(vec![first, put(OTHER, stamp(7, 2), "call")]),
            [
                (RP.to_owned(), Source::Baseline),
                (OTHER.to_owned(), Source::Call)
            ]
        );
    }

    /// desired.v2 §2.10, "Two instances' reports on one key are one reply
    /// after a tool's merge (observer §4.1), as in v1": the packed report's
    /// replies go through the same merge as `desired.v1`'s, in each shape
    /// and in both orders. Within one run the higher seq is kept, across
    /// two runs the later arrival; a newer delete leaves no report; a
    /// reply on another key is not the role's; and the kept reply is read
    /// under its envelope's tag, a refused record unreadable with core
    /// §7.6's token, a put with no envelope `tag`.
    #[test]
    fn two_packed_reports_on_one_key_are_merged_before_they_are_read() {
        use zenkey_model::desired::Source;
        use zenkey_model::desired::v2::{self as d2, Outcome, Report};
        use zenkey_model::freshness::Horizon;
        use zenkey_model::wire::{DeleteStamp, Envelope, Run, Seq, Stamp, Tag};
        use zenoh::bytes::Encoding;
        use zenoh::sample::{Sample, SampleBuilder};

        const RP: &str = "zk2/lab/worker/worker.v2/state/desired/task";
        const OTHER: &str = "zk2/lab/worker/worker.v2/state/desired/plan";
        let stamp = |run: u16, seq: u32| Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        };
        let ke = |k: &str| zenoh::key_expr::KeyExpr::try_from(k.to_owned()).unwrap();
        let baseline = Report::baseline();
        let call = Report {
            source: Source::Call,
            ..Report::baseline()
        };
        // A report as its target puts it: the record under tag 12, its
        // own run and seq.
        let put = |key: &str, at: Stamp, r: &Report| -> Sample {
            SampleBuilder::put(ke(key), r.put(at).unwrap())
                .encoding(Encoding::ZENOH_BYTES)
                .into()
        };
        let delete = |at: Stamp| -> Sample {
            SampleBuilder::delete(ke(RP))
                .attachment(DeleteStamp { stamp: at }.write())
                .into()
        };
        let read = |samples: Vec<Sample>| packed_replies(samples, RP, &Horizon::Undeclared);
        let sources = |samples: Vec<Sample>| -> Vec<(Source, Option<Stamp>)> {
            read(samples)
                .into_iter()
                .map(|p| {
                    assert_eq!(p.key, RP);
                    let r = p.read.expect("a report").report.expect("consistent");
                    assert_eq!(r.outcome, Outcome::NotActed);
                    (r.source, p.inband)
                })
                .collect()
        };
        // One run, seqs 1 and 2, differing: seq 2's, in both orders.
        let (first, second) = (put(RP, stamp(7, 1), &baseline), put(RP, stamp(7, 2), &call));
        let kept = vec![(Source::Call, Some(stamp(7, 2)))];
        assert_eq!(sources(vec![first.clone(), second.clone()]), kept);
        assert_eq!(sources(vec![second, first.clone()]), kept);
        // Two runs, which nothing orders: the later arrival, in both orders.
        let other_run = put(RP, stamp(8, 1), &call);
        assert_eq!(
            sources(vec![first.clone(), other_run.clone()]),
            [(Source::Call, Some(stamp(8, 1)))]
        );
        assert_eq!(
            sources(vec![other_run, first.clone()]),
            [(Source::Baseline, Some(stamp(7, 1)))]
        );
        // A delete is skipped after the merge: a newer one leaves no report.
        assert!(read(vec![first.clone(), delete(stamp(7, 2))]).is_empty());
        assert!(read(vec![delete(stamp(7, 2)), first.clone()]).is_empty());
        assert_eq!(
            sources(vec![put(RP, stamp(7, 3), &call), delete(stamp(7, 2))]),
            [(Source::Call, Some(stamp(7, 3)))]
        );
        // A reply on another key is not this role's report.
        assert_eq!(
            sources(vec![first.clone(), put(OTHER, stamp(7, 2), &call)]),
            [(Source::Baseline, Some(stamp(7, 1)))]
        );
        // A record of another tag, one cut short, and a put with no
        // envelope: unreadable, each with its token, never guessed at.
        let record = baseline.write().unwrap().bytes;
        let tagged = |tag: u8, bytes: &[u8]| -> Sample {
            let payload = Envelope::mutation(Tag::new(tag).unwrap(), stamp(7, 4), bytes).write();
            SampleBuilder::put(ke(RP), payload)
                .encoding(Encoding::ZENOH_BYTES)
                .into()
        };
        let token = |samples: Vec<Sample>| -> String {
            let mut got = read(samples);
            assert_eq!(got.len(), 1);
            got.remove(0).read.expect_err("unreadable").token
        };
        assert_eq!(token(vec![tagged(3, &record)]), "tag");
        assert_eq!(token(vec![tagged(d2::TAG, &record[..2])]), "length");
        let bare: Sample = SampleBuilder::put(ke(RP), record.clone())
            .encoding(Encoding::APPLICATION_OCTET_STREAM)
            .into();
        assert_eq!(token(vec![bare]), "tag");
        // A malformed one reads, with its finding: found, never unreadable.
        let malformed = read(vec![tagged(d2::TAG, &[0x04, 0, 0])]);
        let got = malformed[0].read.as_ref().expect("it reads");
        assert!(got.report.is_none() && !got.findings.is_empty());
    }

    /// desired.v2 §2.5, §2.17 step 6: what a member read's replies say. A
    /// page's live entry and its tombstone, with their in-band stamps; a
    /// page of no entry, the author's word under the page's run; two
    /// instances' entries merged per key; a reply with no envelope, which
    /// carries no version; and a held stamp, which this reader never sent,
    /// no answer.
    #[test]
    fn a_member_reads_replies_say_an_entry_a_tombstone_or_no_entry() {
        use zenkey_model::collection::{Confirmation, Entry, Snapshot, Time, Tombstone};
        use zenkey_model::wire::{HeldStamp, Run, Seq, Stamp, Tag};
        use zenoh::bytes::Encoding;
        use zenoh::sample::{Sample, SampleBuilder};

        const D: &str = "zk2/lab/author/tasking.v2/state/tasks/lab/worker";
        let stamp = |run: u16, seq: u32| Stamp {
            run: Run::U16(run),
            seq: Seq::new(seq).unwrap(),
        };
        let ke = || zenoh::key_expr::KeyExpr::try_from(D.to_owned()).unwrap();
        let page = |at: Stamp, live: Option<u32>, tomb: Option<u32>| -> Sample {
            let snapshot = Snapshot {
                tag: Tag::NONE,
                run: at.run,
                asof: at.seq,
                time: Time::Unknown,
                coll: 0,
                live: live
                    .map(|seq| Entry {
                        id: String::new(),
                        run: None,
                        seq: Seq::new(seq).unwrap(),
                        confirmation: Confirmation::None,
                        record: br#"{"revision": 1}"#.to_vec(),
                    })
                    .into_iter()
                    .collect(),
                tombs: tomb
                    .map(|seq| Tombstone {
                        id: String::new(),
                        run: None,
                        seq: Seq::new(seq).unwrap(),
                    })
                    .into_iter()
                    .collect(),
            };
            let bytes = snapshot.paginate(1024).expect("a page")[0]
                .write()
                .expect("a page writes");
            SampleBuilder::put(ke(), bytes)
                .encoding(Encoding::ZENOH_BYTES)
                .into()
        };
        let read = |samples: Vec<Sample>| member_replies(samples, D);
        assert_eq!(
            read(vec![page(stamp(7, 12), Some(12), None)]),
            [MemberReply::Entry(stamp(7, 12))]
        );
        assert_eq!(
            read(vec![page(stamp(7, 14), None, Some(13))]),
            [MemberReply::Tombstone(stamp(7, 13))]
        );
        assert_eq!(
            read(vec![page(stamp(8, 3), None, None)]),
            [MemberReply::NoEntry {
                run: Run::U16(8),
                asof: Seq::new(3).unwrap(),
            }]
        );
        // Two instances of one run: the higher seq, in both orders; and an
        // entry beside another instance's snapshot of no entry keeps both.
        let (old, new) = (
            page(stamp(7, 12), Some(12), None),
            page(stamp(7, 15), Some(15), None),
        );
        for both in [vec![old.clone(), new.clone()], vec![new, old.clone()]] {
            assert_eq!(read(both), [MemberReply::Entry(stamp(7, 15))]);
        }
        assert_eq!(
            read(vec![page(stamp(8, 3), None, None), old]),
            [
                MemberReply::NoEntry {
                    run: Run::U16(8),
                    asof: Seq::new(3).unwrap(),
                },
                MemberReply::Entry(stamp(7, 12)),
            ]
        );
        // A put with no envelope carries no version.
        let bare: Sample = SampleBuilder::put(ke(), br#"{"revision": 1}"#.to_vec())
            .encoding(Encoding::APPLICATION_JSON)
            .into();
        assert_eq!(read(vec![bare]), [MemberReply::Unstamped]);
        // A held stamp: this reader sent none, so it answers nothing.
        let held: Sample = SampleBuilder::put(ke(), HeldStamp::new(stamp(7, 12)).write())
            .encoding(Encoding::ZENOH_BYTES)
            .into();
        match read(vec![held]).as_slice() {
            [MemberReply::Refused(why)] => assert!(why.starts_with("held: "), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    fn word(bound: bool) -> FaceWord {
        FaceWord {
            listing: Listing::Listed { bound },
            author: bound.then(|| "lab/author".parse().unwrap()),
            params: Map::from([
                ("system".to_owned(), "self.system".to_owned()),
                ("service".to_owned(), "self.service".to_owned()),
            ]),
            author_across: false,
        }
    }

    fn read(word: &FaceWord, offline: &ContractSet) -> TargetRead {
        let mut spec = DesiredSpec::new(Duration::from_secs(1));
        spec.role = Some("task".to_owned());
        across_face(&"lab/worker".parse().unwrap(), word, &spec, offline)
    }

    /// Across a face (§2.14, §2.17 step 2; text 0.6, F-217): a revision of
    /// the author's held offline that declares no desired resource the
    /// requirement names makes the role not a target, whatever the
    /// deployment's word; with none held, the word's listing stands,
    /// bound or the kill switch, and the slice is the target's alone.
    #[test]
    fn across_a_face_only_a_held_revision_shows_a_role_no_targets() {
        let plain = load_str(
            &std::fs::read_to_string(dir().join("tasking.v1.toml"))
                .unwrap()
                .replace(
                    "annotations = { \"desired.target_param\" = [\"system\", \"service\"], ",
                    "annotations = { ",
                )
                .replace("\"desired.v1\", ", ""),
            &dir(),
            Some("tasking.v1.toml"),
        );
        let plain = plain.contract.unwrap_or_else(|| panic!("{}", plain.report));
        let not_desired = set(vec![file("worker.v1"), plain]);
        for bound in [true, false] {
            let t = read(&word(bound), &not_desired);
            assert_eq!(
                t.roles[0].resolution.listing,
                Some(Listing::NotListed),
                "a held revision shows the role on no desired resource, bound {bound}"
            );
        }
        let none_held = set(vec![file("worker.v1")]);
        let t = read(&word(true), &none_held);
        let r = &t.roles[0].resolution;
        assert_eq!(r.listing, Some(Listing::Listed { bound: true }));
        assert_eq!(r.slice_from, Some(SliceFrom::Requirement));
        let t = read(&word(false), &none_held);
        assert_eq!(
            t.roles[0].resolution.listing,
            Some(Listing::Listed { bound: false }),
            "the kill switch, the word's"
        );
        let desired = set(vec![file("worker.v1"), file("tasking.v1")]);
        let t = read(&word(true), &desired);
        let r = &t.roles[0].resolution;
        assert_eq!(r.listing, Some(Listing::Listed { bound: true }));
        assert_eq!(r.slice_from, Some(SliceFrom::OfflineContract));
        assert_eq!(
            r.slice,
            ["zk2/lab/author/tasking.v1/state/tasks/lab/worker"]
        );
    }

    /// `zenctl desired --across-face`'s word (#986 J5): bound to the
    /// author, each target parameter as the author's declaration in hand
    /// orders it (§2.2, §2.3), under either major, so the slice resolves
    /// with no binding problem; both presences the word's; `--param` kept
    /// as given; the kill switch with no author; and one target parameter,
    /// which the deployment binds, never guessed.
    #[test]
    fn a_tools_word_binds_as_the_authors_declaration_orders() {
        let author: Addr = "lab/author".parse().unwrap();
        let ordered = Map::from([
            ("system".to_owned(), "self.system".to_owned()),
            ("service".to_owned(), "self.service".to_owned()),
        ]);
        for (worker, tasking) in [("worker.v1", "tasking.v1"), ("worker.v2", "tasking.v2")] {
            let offline = set(vec![file(worker), file(tasking)]);
            let w =
                FaceWord::of_contracts("task", Some(author.clone()), Map::new(), &offline).unwrap();
            assert_eq!(w.listing, Listing::Listed { bound: true });
            assert_eq!(w.params, ordered, "{tasking}");
            assert!(w.author_across, "the author's presence is the word's too");
            let t = read(&w, &offline);
            let r = &t.roles[0].resolution;
            assert!(r.binding_problems.is_empty(), "{tasking}: {r:?}");
            assert_eq!(
                r.slice,
                [format!("zk2/lab/author/{tasking}/state/tasks/lab/worker")]
            );
            assert_eq!(
                t.roles[0].author_seen,
                Some((Seen::AcrossFace, None)),
                "{tasking}"
            );
        }
        let offline = set(vec![file("worker.v2"), file("tasking.v2")]);
        let given = Map::from([("system".to_owned(), "self.service".to_owned())]);
        let w =
            FaceWord::of_contracts("task", Some(author.clone()), given.clone(), &offline).unwrap();
        assert_eq!(
            w.params, given,
            "--param stands as given, a wrong one the reading's"
        );
        let w = FaceWord::of_contracts("task", None, Map::new(), &offline).unwrap();
        assert_eq!(w.listing, Listing::Listed { bound: false });
        assert!(w.params.is_empty() && w.author.is_none());
        // With nothing of the author's in hand, nothing is derived.
        let alone = set(vec![file("worker.v2")]);
        let w = FaceWord::of_contracts("task", Some(author.clone()), Map::new(), &alone).unwrap();
        assert!(w.params.is_empty());
        // One target parameter: the deployment's choice, never guessed.
        let one = |text: &str, name: &str| {
            let l = load_str(
                &text
                    .replace("tasks/{system}/{service}", "tasks/{service}")
                    .replace(
                        "{ system = \"string\", service = \"string\" }",
                        "{ service = \"string\" }",
                    )
                    .replace("[\"system\", \"service\"]", "\"service\""),
                &dir(),
                Some(name),
            );
            l.contract.unwrap_or_else(|| panic!("{}", l.report))
        };
        let read_file = |n: &str| std::fs::read_to_string(dir().join(n)).unwrap();
        let single = set(vec![
            one(&read_file("worker.v1.toml"), "worker.v1.toml"),
            one(&read_file("tasking.v1.toml"), "tasking.v1.toml"),
        ]);
        let why =
            FaceWord::of_contracts("task", Some(author.clone()), Map::new(), &single).unwrap_err();
        assert!(
            why.contains("`service`") && why.contains("--param service=self.system"),
            "{why}"
        );
        let given = Map::from([("service".to_owned(), "self.service".to_owned())]);
        let w = FaceWord::of_contracts("task", Some(author), given, &single).unwrap();
        let t = read(&w, &single);
        assert_eq!(
            t.roles[0].resolution.slice,
            ["zk2/lab/author/tasking.v1/state/tasks/worker"]
        );
    }
}
