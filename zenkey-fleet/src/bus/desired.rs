//! `desired.v1` (#840, DD), the reads: one target's roles against their
//! authors' documents, read as §2.17's tool reads them
//! (`spec/profiles/desired/v1.md`), through a session in the deployment's
//! namespace.
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
//! 4. S4's GET of each target's report resources, every reply read as a
//!    `desired.v1.Report` (§2.10's reading, `zenkey_model::desired::decode`);
//! 5. S4's GET of each slice from its author, present or across a face
//!    (§2.17 step 6); and, for an author absent by a complete read, each
//!    archive's last-known of the slice, shown and never current (core
//!    §4.4, S6).
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
    Archived, Behind, Contracts, DesiredRead, FaceWord, ReportReply, ReportsGet, RoleRead,
    RoleResolution, SliceFrom, SliceGet, SliceReply, TargetRead, author_contract, descriptor_in,
    members_selector, named_revision, report_resources, resolve_role, seen_in,
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
                tr.roles.push(RoleRead {
                    resolution: RoleResolution::bare(role_asked(spec), listing),
                    author_seen: None,
                    current: None,
                    archived: None,
                });
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
        .filter(|(_, tr)| tr.roles.iter().any(|r| !r.resolution.slice.is_empty()))
        .map(|(i, tr)| (i, report_selectors(tr, &contracts, offline)))
        .collect();
    for (_, p) in &plans {
        if let Ok(sels) = p {
            read.asked.extend(sels.iter().map(|(s, _)| s.clone()));
        }
    }
    let needs_archives = spec.archives
        && read.targets.iter().any(|tr| {
            tr.roles.iter().any(|r| {
                !r.resolution.slice.is_empty() && matches!(r.author_seen, Some((Seen::Absent, _)))
            })
        });
    let reports = bounded(
        plans
            .into_iter()
            .map(|(i, plan)| {
                let s = session.clone();
                Box::pin(async move {
                    (
                        i,
                        match plan {
                            Err(why) => Err(why),
                            Ok(sels) => Ok(reports_get(&s, sels, t).await),
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
                    legs.push((i, j, slice, true));
                }
                Some(Seen::Absent) if spec.archives => legs.push((i, j, slice, false)),
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
                    let leg = if current {
                        Leg::Current(slice_get(&s, &slice, t).await)
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

/// A target's report selectors, or why there are none to read.
type ReportPlan = Result<Vec<(String, zenkey_model::freshness::Horizon)>, String>;

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
        roles: vec![RoleRead {
            resolution: RoleResolution::bare(role_asked(spec), None),
            author_seen: None,
            current: None,
            archived: None,
        }],
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
    // A target of this profile holds a report resource (§2.10).
    let reporting = d.interfaces.iter().any(|e| {
        named_revision(d, &e.iface, contracts, offline)
            .is_some_and(|r| !report_resources(r.contract()).is_empty())
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
        let templates = e.declared_by.as_deref().and_then(|by| {
            let rev = named_revision(d, by, contracts, offline)?;
            rev.contract().requires.get(&e.role)?.resources.clone()
        });
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
                templates.as_deref(),
                contracts,
                offline,
            )
        });
        let held = author_c.is_some();
        match resolve_role(&t.addr, e, templates.as_deref(), author_c) {
            Some(res) if held || reporting || !all_held => out.push(RoleRead {
                resolution: res,
                author_seen: None,
                current: None,
                archived: None,
            }),
            _ if spec.role.is_some() => out.push(RoleRead {
                resolution: RoleResolution::bare(e.role.clone(), Some(Listing::NotListed)),
                author_seen: None,
                current: None,
                archived: None,
            }),
            _ => {}
        }
    }
    if out.is_empty()
        && let Some(r) = &spec.role
    {
        out.push(RoleRead {
            resolution: RoleResolution::bare(r.clone(), Some(Listing::NotListed)),
            author_seen: None,
            current: None,
            archived: None,
        });
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
            // target's, whatever the deployment's word (text 0.6, F-217).
            resolve_role(a, &entry, req.resources.as_deref(), author_c).unwrap_or_else(|| {
                RoleResolution::bare(role.clone(), Some(Listing::NotListed))
            })
        }
        None => RoleResolution::bare(role.clone(), Some(word.listing)),
    };
    if res.listing != Some(Listing::NotListed) {
        res.listing = Some(word.listing);
    }
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
    t.roles.push(RoleRead {
        resolution: res,
        author_seen,
        current: None,
        archived: None,
    });
    t
}

/// The selectors of `t`'s report resources (§2.10), from the revisions its
/// descriptor names, or across a face every one held offline; `Err` when
/// one it names is not in hand and none found holds a report.
fn report_selectors(
    t: &TargetRead,
    contracts: &Contracts,
    offline: &ContractSet,
) -> Result<Vec<(String, zenkey_model::freshness::Horizon)>, String> {
    let mut out = Vec::new();
    let mut missing = Vec::new();
    match &t.descriptor {
        Ok(Some(d)) => {
            for e in &d.interfaces {
                match named_revision(d, &e.iface, contracts, offline) {
                    Some(rev) => {
                        for r in report_resources(rev.contract()) {
                            out.push((
                                members_selector(&t.addr, rev.iface(), r),
                                zenkey_model::freshness::horizon_of(r),
                            ));
                        }
                    }
                    None => missing.push(format!("{} {}", e.iface, e.contract)),
                }
            }
        }
        _ => {
            for rev in offline.iter() {
                for r in report_resources(rev.contract()) {
                    out.push((
                        members_selector(&t.addr, rev.iface(), r),
                        zenkey_model::freshness::horizon_of(r),
                    ));
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
    out.retain(|(s, _)| seen.insert(s.clone()));
    Ok(out)
}

/// S4's GET of each report resource (core §4.2), every reply read as a
/// `desired.v1.Report` (§2.10). A reply on a wildcard key is no member (R6).
async fn reports_get(
    session: &Session,
    selectors: Vec<(String, zenkey_model::freshness::Horizon)>,
    t: Duration,
) -> ReportsGet {
    let mut out = ReportsGet {
        selectors: selectors.iter().map(|(s, _)| s.clone()).collect(),
        complete: true,
        replies: Vec::new(),
        at: SystemTime::now(),
    };
    // Each resource's GET at once.
    let gets = bounded(
        selectors
            .into_iter()
            .map(|(sel, horizon)| {
                let session = session.clone();
                Box::pin(async move {
                    let mut replies_out = Vec::new();
                    let mut complete = true;
                    let replies = match crate::bus::query::state_get(
                        &session,
                        &sel,
                        &crate::bus::query::GetOpts::new(t),
                    )
                    .await
                    {
                        Ok(r) => r,
                        Err(_) => return (false, replies_out),
                    };
                    while let Ok(reply) = replies.recv_async().await {
                        let Ok(sample) = reply.result() else {
                            complete = false;
                            continue;
                        };
                        let key = sample.key_expr().as_str();
                        if key.contains('*') || sample.kind() == SampleKind::Delete {
                            continue;
                        }
                        replies_out.push(ReportReply {
                            key: key.to_owned(),
                            read: dm::decode(&sample.payload().to_bytes()),
                            stamp: sample.timestamp().map(|ts| Stamped {
                                time: ts.get_time().to_system_time(),
                                clock: ts.get_id().to_string(),
                            }),
                            horizon: horizon.clone(),
                        });
                    }
                    (complete, replies_out)
                }) as BoxFuture<'static, _>
            })
            .collect(),
    )
    .await;
    for (complete, replies) in gets {
        out.complete &= complete;
        out.replies.extend(replies);
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
/// consolidation `Latest`, through the crate's one state GET.
async fn slice_get(session: &Session, slice: &str, t: Duration) -> Result<SliceGet, String> {
    let replies = crate::bus::query::state_get(session, slice, &crate::bus::query::GetOpts::new(t))
        .await
        .map_err(|e| format!("GET {slice}: {}", crate::one_line(&e)))?;
    let mut out = SliceGet {
        selector: slice.to_owned(),
        complete: true,
        replies: Vec::new(),
        at: SystemTime::now(),
    };
    while let Ok(reply) = replies.recv_async().await {
        let Ok(sample) = reply.result() else {
            out.complete = false;
            continue;
        };
        let key = sample.key_expr().as_str();
        if key.contains('*') {
            continue;
        }
        out.replies.push(SliceReply {
            key: key.to_owned(),
            delete: sample.kind() == SampleKind::Delete,
            stamp: sample
                .timestamp()
                .map(|ts| (zenkey::desired::stamp(ts), ts.get_time().to_system_time())),
        });
    }
    out.at = SystemTime::now();
    Ok(out)
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
