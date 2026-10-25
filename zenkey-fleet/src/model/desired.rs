//! `desired.v1` (#840, DD): one target's roles as values in hand
//! (`spec/profiles/desired/v1.md` §2.3, §2.10, §2.17, §5).
//!
//! **Values in hand → the model's reading.** A [`DesiredRead`] is what one
//! reading of the bus found ([`crate::bus::desired`] fills it): the
//! targets' presence and descriptors, the contracts, and per role its
//! resolution ([`resolve_role`]: the slice's key from the author's
//! declaration and the target's binding, §2.2 and §2.3), the author's
//! presence and S4's GET of the slice, and the target's report replies.
//! [`RoleRead::reading`] projects one role into `zenkey_model::desired`'s
//! [`TargetReading`] and [`AuthorReading`], which §5's four judges answer
//! from ([`crate::judge::desired`]): no verdict is decided here, and none
//! is re-derived.
//!
//! **Finding a slice without its author's contract** (text 0.4). The
//! slice's key needs the desired resource's kind token and its target
//! parameters, the author's declaration. With the author's contract in hand
//! (its descriptor's revision, retrieved, §8.4, or held offline), the
//! model's [`dm::binding`] reads them. Without it, an absent author's for
//! one, the target's own requirement gives the template, its binding gives
//! each parameter, and the kind token is the one a report's key carries
//! ([`SliceFrom::Requirement`]): the slice §2.3 builds, as far as the target
//! alone says it.
//!
//! **How long behind** (text 0.4, §2.17): [`Behind`] follows each role
//! across readings, the author's version and the target's "not yet" on it,
//! so that a run with a window can say a target was behind one version for
//! at least that long, and, on a clock trusted to the HLC delta, how old
//! the version it is behind is. Neither moves §5's answer, which stays
//! "not yet".

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime};

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Contract, Resource};
use zenkey_model::descriptor::{Descriptor, RequireEntry};
use zenkey_model::desired::{
    self as dm, AuthorReading, Binding, BindingProblem, Current, Declaration, Invalid, Listing,
    ReportRead, Seen, TargetReading,
};
use zenkey_model::freshness::{self, ClockTrust, Horizon, Observation, Reply, StampAge};
use zenkey_model::grammar::{Addr, IfaceId, KindToken};
use zenkey_model::schema::TypeId;
use zenkey_model::seed::Stamp;
use zenkey_model::template::{Segment, Template};

use crate::bus::health::Stamped;
use crate::model::catalog::{Catalog, ContractSet, ContractState, Observed, Revision};
use crate::model::health::Trust;

/// What a reading found for each revision it asked about, by `(iface,
/// fingerprint)`: what [`crate::BundleStore::fetch`] answered, or why it
/// could not ask.
pub type Contracts = BTreeMap<(IfaceId, Fingerprint), Result<ContractState, String>>;

// ─── the report resources and the slice ────────────────────────────────────

/// Whether `r` holds `desired.v1.Report`s (§2.10): a state resource typed
/// `desired.v1.Report` from an artifact the profile publishes (§3), by its
/// id, never by its name alone (core §7.1).
pub fn is_report_resource(r: &Resource) -> bool {
    matches!(r.token, KindToken::State | KindToken::ExplicitState)
        && matches!(&r.body, Body::Data(d) if matches!(
            &d.type_,
            TypeId::JsonSchema { name, schema } if name == dm::REPORT && dm::is_published(schema)
        ))
}

/// A contract's report resources (§2.10).
pub fn report_resources(c: &Contract) -> Vec<&Resource> {
    c.resources
        .iter()
        .filter(|r| is_report_resource(r))
        .collect()
}

/// The key expression every member of `r`, of `addr`'s `iface`, is under,
/// base-relative: each parameter `*`, a rest one `**`.
pub fn members_selector(addr: &Addr, iface: &IfaceId, r: &Resource) -> String {
    let tail: Vec<String> = r
        .template
        .segments()
        .iter()
        .map(|s| match s {
            Segment::Literal(l) => l.clone(),
            Segment::Param(_) => "*".to_owned(),
            Segment::Rest(_) => "**".to_owned(),
        })
        .collect();
    format!(
        "zk2/{}/{}/{iface}/{}/{}",
        addr.system,
        addr.service,
        r.token,
        tail.join("/")
    )
}

/// Where a role's slice came from (text 0.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceFrom {
    /// The author's revision, as its descriptor names it: its declaration
    /// read (§2.2) and the binding checked (§2.3).
    AuthorContract,
    /// A revision of the required interface held offline (core §8.5).
    OfflineContract,
    /// No contract of the author's in hand: the target's requirement and
    /// binding, the kind token the report's key carries.
    Requirement,
}

impl SliceFrom {
    /// The wire token.
    pub fn as_str(self) -> &'static str {
        match self {
            SliceFrom::AuthorContract => "author_contract",
            SliceFrom::OfflineContract => "offline_contract",
            SliceFrom::Requirement => "requirement",
        }
    }
}

/// One role a target's descriptor lists, resolved (§2.2, §2.3).
#[derive(Debug, Clone, PartialEq)]
pub struct RoleResolution {
    pub role: String,
    /// The required interface.
    pub interface: Option<IfaceId>,
    /// What the descriptor says of the role; `None` when it was not read,
    /// [`Listing::NotListed`] when it does not list the role.
    pub listing: Option<Listing>,
    /// The one author its binding names, when it names one exact address.
    pub author: Option<Addr>,
    /// The desired resource, `<kind token>/<template>`, when known.
    pub resource: Option<String>,
    /// The author's declaration of it, when invalid (§2.2): a contract's
    /// finding, and no slice.
    pub declaration_invalid: Vec<Invalid>,
    /// The binding's problems (§2.3): no slice.
    pub binding_problems: Vec<BindingProblem>,
    /// The slice's key; two candidates, under `state` and `@state`, when
    /// it came from the requirement alone, the report's key choosing.
    pub slice: Vec<String>,
    pub slice_from: Option<SliceFrom>,
}

impl RoleResolution {
    /// A role the descriptor does not list, or one it could not be read
    /// for: nothing more is known.
    pub fn bare(role: impl Into<String>, listing: Option<Listing>) -> RoleResolution {
        RoleResolution {
            role: role.into(),
            interface: None,
            listing,
            author: None,
            resource: None,
            declaration_invalid: Vec::new(),
            binding_problems: Vec::new(),
            slice: Vec::new(),
            slice_from: None,
        }
    }

    /// Whether the role is bound (§2.3), its kill switch off.
    pub fn bound(&self) -> bool {
        matches!(self.listing, Some(Listing::Listed { bound: true }))
    }
}

/// The one exact address `bindings` name (§2.3), or `None`.
fn one_author(bindings: &[String]) -> Option<Addr> {
    let [one] = bindings else { return None };
    if one.split('/').any(|c| c == "*") {
        return None;
    }
    one.parse().ok()
}

/// The desired resources of `author` among the requirement's `templates`
/// (every one when the requirement names none), with each declaration
/// (§2.2): the first in the contract's order, by its `(token, template)`.
fn desired_resource<'c>(
    author: &'c Contract,
    templates: Option<&[String]>,
) -> Option<(&'c Resource, Declaration)> {
    author
        .resources
        .iter()
        .filter(|r| templates.is_none_or(|t| t.iter().any(|t| t == r.template.as_str())))
        .map(|r| (r, dm::declaration_of(r)))
        .find(|(_, d)| *d != Declaration::NotThisProfiles)
}

/// Resolves one `requires` entry of `own`'s descriptor (§2.3): its
/// listing, its author, and its slice from `author`'s contract when one is
/// in hand, else from the requirement's `templates` (the target's contract
/// declaring the role). `None` when the author's contract declares no
/// desired resource the requirement names: the role is not this profile's.
pub fn resolve_role(
    own: &Addr,
    entry: &RequireEntry,
    templates: Option<&[String]>,
    author: Option<(&Contract, SliceFrom)>,
) -> Option<RoleResolution> {
    let mut out = RoleResolution::bare(
        entry.role.clone(),
        Some(Listing::Listed {
            bound: !entry.bindings.is_empty(),
        }),
    );
    out.interface = IfaceId::from_str(&entry.interface).ok();
    out.author = one_author(&entry.bindings);
    let req = dm::Requirement {
        bindings: &entry.bindings,
        params: &entry.params,
        optional: entry.optional,
    };
    if let Some((contract, from)) = author {
        let (r, decl) = desired_resource(contract, templates)?;
        out.resource = Some(format!("{}/{}", r.token, r.template.as_str()));
        match decl {
            Declaration::Invalid(why) => out.declaration_invalid = why,
            Declaration::Declared(targets) => {
                let slice = dm::Slice {
                    iface: &contract.iface,
                    token: r.token,
                    template: &r.template,
                    targets: &targets,
                };
                match dm::binding(&slice, &req, own) {
                    Binding::Bound { key } => {
                        out.slice = vec![key.to_string()];
                        out.slice_from = Some(from);
                    }
                    Binding::Unbound => {}
                    Binding::Invalid(why) => out.binding_problems = why,
                }
            }
            Declaration::NotThisProfiles => return None,
        }
        return Some(out);
    }
    // No contract of the author's: the requirement's own template, every
    // parameter bound as the binding says (§2.3), under either kind token.
    if entry.bindings.is_empty() {
        return Some(out);
    }
    let Some(iface) = out.interface.clone() else {
        out.binding_problems = vec![BindingProblem::NotOneAuthor];
        return Some(out);
    };
    for t in templates.into_iter().flatten() {
        let Ok(template) = Template::parse(t) else {
            continue;
        };
        let targets: Vec<dm::TargetParam> = template
            .params()
            .filter(|(_, rest)| !rest)
            .map(|(p, _)| dm::TargetParam {
                param: p.to_owned(),
                binds: dm::Binds::Either,
            })
            .collect();
        out.resource.get_or_insert_with(|| t.clone());
        for token in [KindToken::State, KindToken::ExplicitState] {
            let slice = dm::Slice {
                iface: &iface,
                token,
                template: &template,
                targets: &targets,
            };
            match dm::binding(&slice, &req, own) {
                Binding::Bound { key } => out.slice.push(key.to_string()),
                Binding::Unbound => {}
                Binding::Invalid(why) => {
                    if out.binding_problems.is_empty() {
                        out.binding_problems = why;
                    }
                }
            }
        }
    }
    if !out.slice.is_empty() {
        out.binding_problems.clear();
        out.slice_from = Some(SliceFrom::Requirement);
    }
    Some(out)
}

/// The revision of `iface` a role's slice is read by (§2.3): the one
/// `author`'s descriptors name, in hand; else one held offline that
/// declares a desired resource `templates` names. `None` when neither.
pub fn author_contract<'c>(
    catalog: Option<&Catalog>,
    author: Option<&Addr>,
    iface: &IfaceId,
    templates: Option<&[String]>,
    contracts: &'c Contracts,
    offline: &'c ContractSet,
) -> Option<(&'c Contract, SliceFrom)> {
    if let (Some(c), Some(a)) = (catalog, author) {
        for fp in c.revisions_of(a, iface) {
            if let Some(Ok(ContractState::Held(r))) = contracts.get(&(iface.clone(), fp)) {
                return Some((r.contract(), SliceFrom::AuthorContract));
            }
        }
    }
    offline
        .of_iface(iface)
        .map(|r| r.contract())
        .find(|c| desired_resource(c, templates).is_some())
        .map(|c| (c, SliceFrom::OfflineContract))
}

/// The contract revision `iface` of `descriptor` names, when in hand:
/// retrieved, else held offline by the same fingerprint.
pub fn named_revision<'c>(
    descriptor: &Descriptor,
    iface: &str,
    contracts: &'c Contracts,
    offline: &'c ContractSet,
) -> Option<&'c Revision> {
    let e = descriptor.interfaces.iter().find(|e| e.iface == iface)?;
    let key = (
        IfaceId::from_str(&e.iface).ok()?,
        Fingerprint::parse(&e.contract).ok()?,
    );
    match contracts.get(&key) {
        Some(Ok(ContractState::Held(r))) => Some(r),
        _ => offline.get(&key.0, &key.1).map(AsRef::as_ref),
    }
}

// ─── what one reading found ─────────────────────────────────────────────────

/// One reply of a target's report resources (§2.10), as read.
#[derive(Debug, Clone)]
pub struct ReportReply {
    /// The key it came on, base-relative.
    pub key: String,
    /// It read as a report, or why it did not.
    pub read: Result<dm::Report, dm::Unreadable>,
    /// Its stamp, the target's (core S1).
    pub stamp: Option<Stamped>,
    /// Its resource's horizon (§2.10, `freshness.v1`).
    pub horizon: Horizon,
}

/// S4's GET of one target's report resources.
#[derive(Debug, Clone)]
pub struct ReportsGet {
    pub selectors: Vec<String>,
    /// Whether every GET ran to its final reply.
    pub complete: bool,
    pub replies: Vec<ReportReply>,
    /// This host's clock as the GETs ended: what a reply is aged at.
    pub at: SystemTime,
}

/// One reply of S4's GET of a slice from its author (§2.17 step 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceReply {
    pub key: String,
    pub delete: bool,
    /// Its stamp, as the model orders it, and its time on the wall clock.
    pub stamp: Option<(Stamp, SystemTime)>,
}

/// S4's GET of a slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceGet {
    pub selector: String,
    /// It ran to its final reply, with no error reply.
    pub complete: bool,
    pub replies: Vec<SliceReply>,
    pub at: SystemTime,
}

/// An archive's last-known of a slice (core §4.4, S6): shown, never
/// current, and never part of a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archived {
    /// The archive.
    pub archive: Addr,
    /// A tombstone (`reply_del`).
    pub deleted: bool,
    pub stamp: Option<Stamp>,
    /// The document's own revision (§2.4), when it carries one.
    pub revision: Option<u64>,
    pub confirmed: bool,
}

/// What one reading read of one role.
#[derive(Debug, Clone)]
pub struct RoleRead {
    pub resolution: RoleResolution,
    /// The author's presence, and why it is not known when it is not.
    pub author_seen: Option<(Seen, Option<String>)>,
    /// S4's GET of the slice; `None` when none was made.
    pub current: Option<Result<SliceGet, String>>,
    /// Each archive's last-known of the slice, when its author is absent;
    /// `None` when none was read.
    pub archived: Option<Result<Vec<Archived>, String>>,
}

/// What one reading read of one target.
#[derive(Debug, Clone)]
pub struct TargetRead {
    pub addr: Addr,
    /// Its presence, and why it is not known when it is not.
    pub seen: Seen,
    pub seen_why: Option<String>,
    /// Across a constrained face: the deployment's word.
    pub word: Option<FaceWord>,
    /// Its descriptor: `None` when not read, with why.
    pub descriptor: Result<Option<Descriptor>, String>,
    pub roles: Vec<RoleRead>,
    /// S4's GET of its report resources; `None` when none was made.
    pub reports: Option<Result<ReportsGet, String>>,
}

/// What the tool was told across a constrained face (§2.14, §2.17 step 2):
/// what the target's descriptor would say of the role, and its author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaceWord {
    pub listing: Listing,
    /// The role's author, as its binding would name it.
    pub author: Option<Addr>,
    /// Its parameter bindings, as the descriptor would list them (core
    /// R2): each target parameter `self.system` or `self.service`.
    pub params: BTreeMap<String, String>,
    /// The author sits across the face too: its presence does not cross.
    pub author_across: bool,
}

/// One reading of the bus, as values.
#[derive(Debug, Clone)]
pub struct DesiredRead {
    /// When it began, on this host's monotonic clock.
    pub began: Instant,
    /// This host's wall clock as its GETs ended.
    pub at: SystemTime,
    /// The session reached no other zenoh node: nothing was read (core
    /// §8.1, 0.37).
    pub cut_off: Option<String>,
    /// The targets' presence read, with descriptors; `None` when none was
    /// made (across a face, or cut off).
    pub presence: Option<Result<Observed, String>>,
    pub targets: Vec<TargetRead>,
    /// The selectors put to the bus, base-relative.
    pub asked: Vec<String>,
}

/// A presence read's answer for `addr` (core §8.1): present by its
/// instance token; absent by a complete read without it; otherwise
/// unobservable, with why.
pub fn seen_in(read: Option<&Result<Observed, String>>, addr: &Addr) -> (Seen, Option<String>) {
    match read {
        None => (
            Seen::Incomplete,
            Some("no presence read was made".to_owned()),
        ),
        Some(Err(why)) => (
            Seen::Incomplete,
            Some(format!("the presence read could not be made: {why}")),
        ),
        Some(Ok(o)) if Catalog::new(o).has_instance(addr) => (Seen::Present, None),
        Some(Ok(o)) if o.complete => (Seen::Absent, None),
        Some(Ok(o)) => (
            Seen::Incomplete,
            Some(format!(
                "the presence read {}",
                o.incomplete_reason("ended at its timeout")
                    .unwrap_or("is possibly incomplete")
            )),
        ),
    }
}

/// `addr`'s descriptor in `o` (§3.3): the first an instance of it served;
/// `Ok(None)` for an address with no instance; `Err` when every instance's
/// descriptor GET failed or drew nothing.
pub fn descriptor_in(o: &Observed, addr: &Addr) -> Result<Option<Descriptor>, String> {
    let Some(descriptors) = &o.descriptors else {
        return Err("descriptors were not asked".to_owned());
    };
    let mut why = None;
    for ((a, i), read) in descriptors {
        if a != addr {
            continue;
        }
        match read.descriptor() {
            Some(d) => return Ok(Some(d.clone())),
            None => why = Some(format!("instance {i}'s descriptor: {}", read.tag())),
        }
    }
    match why {
        Some(w) => Err(w),
        None => Ok(None),
    }
}

/// The report of a slice among a target's replies (§2.10, §2.17 step 3).
#[derive(Debug, Clone, Copy)]
pub enum ReportFound<'a> {
    /// The readable report whose `key` is the slice's.
    One(&'a ReportReply),
    /// None readable claims the slice, and one did not read: it may be the
    /// slice's (text 0.4).
    Unreadable(&'a ReportReply),
    /// Two readable reports claim the slice, and they differ (text 0.4).
    Two,
    /// None.
    None,
}

/// Finds the report of `slice` (one key, or the candidates a requirement
/// gave) among `replies`: the readable one whose `key` is one of them.
pub fn report_of<'a>(replies: &'a [ReportReply], slice: &[String]) -> ReportFound<'a> {
    let claiming: Vec<&ReportReply> = replies
        .iter()
        .filter(|r| {
            r.read
                .as_ref()
                .is_ok_and(|d| d.key.as_ref().is_some_and(|k| slice.contains(k)))
        })
        .collect();
    match claiming.as_slice() {
        [one] => ReportFound::One(one),
        [first, rest @ ..] => {
            if rest
                .iter()
                .all(|r| r.read.as_ref().ok() == first.read.as_ref().ok())
            {
                ReportFound::One(first)
            } else {
                ReportFound::Two
            }
        }
        [] => replies
            .iter()
            .find(|r| r.read.is_err())
            .map_or(ReportFound::None, ReportFound::Unreadable),
    }
}

/// The report reply's freshness (§2.17 step 4, `freshness.v1` §2.6): its
/// resource's horizon, its stamp aged at `at` against a trusted clock.
pub fn report_freshness(
    reply: &ReportReply,
    at: SystemTime,
    trust: &Trust<'_>,
) -> freshness::Judged {
    let observation = Observation::Got {
        reply: Some(Reply::Put {
            stamp_age: reply.stamp.as_ref().map(|s| StampAge::between(s.time, at)),
        }),
        clock: reply
            .stamp
            .as_ref()
            .map_or(ClockTrust::Untrusted, |s| trust.of(&s.clock)),
    };
    freshness::judge_all(&reply.horizon, &[observation])
}

/// The author's current version of `slice` by S4's GET (§2.17 step 6), and
/// why it is no version when it is none: an unstamped reply carries none
/// (core S1; text 0.4), read as a GET that could not answer.
pub fn current_of(
    get: Option<&Result<SliceGet, String>>,
    slice: &str,
) -> (Current, Option<String>) {
    match get {
        None => (Current::NotRead, None),
        Some(Err(why)) => (
            Current::NotRead,
            Some(format!("the GET could not be made: {why}")),
        ),
        Some(Ok(g)) if !g.complete => (
            Current::Incomplete,
            Some("the GET ended at its timeout, or with an error reply".to_owned()),
        ),
        Some(Ok(g)) => {
            let mine: Vec<&SliceReply> = g.replies.iter().filter(|r| r.key == slice).collect();
            if mine.is_empty() {
                return (Current::Silent, None);
            }
            if mine.iter().any(|r| r.stamp.is_none()) {
                return (
                    Current::Incomplete,
                    Some(
                        "a reply carries no stamp, so no version (core S1, S2): its author \
                         broke S1"
                            .to_owned(),
                    ),
                );
            }
            let latest = mine
                .into_iter()
                .max_by_key(|r| r.stamp.map(|(s, _)| s))
                .expect("not empty");
            let (s, _) = latest.stamp.expect("every reply stamped");
            (
                if latest.delete {
                    Current::Delete(s)
                } else {
                    Current::Put(s)
                },
                None,
            )
        }
    }
}

/// The wall-clock time of the author's current version, when its GET
/// answered one: what a clock trusted to the HLC delta ages it by.
pub fn current_time(get: Option<&Result<SliceGet, String>>, slice: &str) -> Option<SystemTime> {
    let Some(Ok(g)) = get else { return None };
    g.replies
        .iter()
        .filter(|r| r.key == slice)
        .filter_map(|r| r.stamp)
        .max_by_key(|(s, _)| *s)
        .map(|(_, t)| t)
}

/// What a role's reading needs of the report, kept beside it.
#[derive(Debug, Clone)]
pub struct RoleReading<'a> {
    pub target: TargetReading<'a>,
    pub author: AuthorReading,
    /// The report found, when one was.
    pub report: Option<&'a ReportReply>,
    /// Why step 3 read none, when it read none.
    pub no_report: Option<String>,
    pub freshness: freshness::Judged,
    /// Why the author's presence or its version is not known.
    pub author_why: Option<String>,
}

impl RoleRead {
    /// This role's reading, as `zenkey_model::desired` takes it (§2.17):
    /// the target's presence, its descriptor or the deployment's word, its
    /// report of the slice and its age, the author's presence and its
    /// version. `trust` ages a reply's stamp.
    pub fn reading<'a>(&'a self, t: &'a TargetRead, trust: &Trust<'_>) -> RoleReading<'a> {
        let res = &self.resolution;
        let mut no_report = None;
        let mut found = None;
        let mut report = ReportRead::NotRead;
        let mut fresh = freshness::judge_all(&Horizon::Undeclared, &[]);
        if !res.declaration_invalid.is_empty() {
            no_report = Some(format!(
                "the author's declaration of its desired resource is invalid ({}), so the role \
                 has no slice (§2.2)",
                res.declaration_invalid
                    .iter()
                    .map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        } else if !res.binding_problems.is_empty() {
            no_report = Some(format!(
                "its binding is invalid ({}), so the role has no slice (§2.3)",
                res.binding_problems
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        } else if res.slice.is_empty() {
            no_report = Some(if res.resource.is_none() {
                "no contract in hand says which resource the role binds: the author's \
                 revision not retrieved, none held offline, and the requirement names none"
                    .to_owned()
            } else {
                "its slice could not be built from its binding (§2.3)".to_owned()
            });
        } else {
            match &t.reports {
                None => no_report = Some("its report resources were not read".to_owned()),
                Some(Err(why)) => {
                    no_report = Some(format!("the GET of its reports could not be made: {why}"));
                }
                Some(Ok(g)) => match report_of(&g.replies, &res.slice) {
                    ReportFound::One(r) => {
                        found = Some(r);
                        report = match &r.read {
                            Ok(d) => ReportRead::Read(d),
                            Err(_) => ReportRead::Unreadable,
                        };
                        fresh = report_freshness(r, g.at, trust);
                    }
                    ReportFound::Unreadable(r) => {
                        found = Some(r);
                        report = ReportRead::Unreadable;
                    }
                    ReportFound::Two => {
                        report = ReportRead::Unreadable;
                        no_report = Some(
                            "two reports claim the slice, and they differ (text 0.4)".to_owned(),
                        );
                    }
                    ReportFound::None => {
                        no_report = Some(if g.complete {
                            format!(
                                "no report of its report resources names the slice ({} \
                                 reply(ies))",
                                g.replies.len()
                            )
                        } else {
                            "no report names the slice, and the GET ended at its timeout".to_owned()
                        });
                    }
                },
            }
        }
        let (descriptor, deployment) = match (&t.word, &t.descriptor) {
            (Some(w), _) => (None, Some(w.listing)),
            (None, Ok(Some(_))) | (None, Ok(None)) => (res.listing, None),
            (None, Err(_)) => (None, None),
        };
        let target = TargetReading {
            presence: t.seen,
            descriptor,
            deployment,
            report,
            freshness: fresh.verdict,
        };
        let slice = res.slice.first().map(String::as_str).unwrap_or_default();
        let slice = found
            .and_then(|r| r.read.as_ref().ok())
            .and_then(|d| d.key.as_deref())
            .unwrap_or(slice);
        let (author_seen, seen_why) = self
            .author_seen
            .clone()
            .unwrap_or((Seen::Incomplete, Some("the author was not read".to_owned())));
        let (current, current_why) = current_of(self.current.as_ref(), slice);
        RoleReading {
            target,
            author: AuthorReading {
                presence: author_seen,
                current,
            },
            report: found,
            no_report,
            freshness: fresh,
            author_why: seen_why.or(current_why),
        }
    }
}

// ─── how long behind ─────────────────────────────────────────────────────────

/// How a target was found behind its author's version for at least the
/// window (§2.17's "behind for longer than it expects", text 0.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehindBy {
    /// "Not yet" on the same version in readings at least the window apart.
    Window,
    /// The version stamped more than the window and the HLC delta before
    /// this host's clock, trusted to the delta (`freshness.v1` §2.6).
    Clock,
}

/// A role found behind one version of its author's for at least the
/// window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BehindFor {
    pub version: Stamp,
    /// How long, at least.
    pub since: Duration,
    pub by: BehindBy,
}

/// Follows each role across readings: the first reading of an unbroken run
/// that found it "not yet" on one version of its author's (§2.17 step 7).
#[derive(Debug, Clone, Default)]
pub struct Behind {
    since: BTreeMap<(String, String), (Stamp, Instant)>,
}

impl Behind {
    /// One reading's answer for a role (`target`, `role`): "not yet" on
    /// `version`, read at `began`; anything else ends its run.
    pub fn note(&mut self, target: &str, role: &str, not_yet_on: Option<Stamp>, began: Instant) {
        let key = (target.to_owned(), role.to_owned());
        match not_yet_on {
            Some(v) => match self.since.get(&key) {
                Some((held, _)) if *held == v => {}
                _ => {
                    self.since.insert(key, (v, began));
                }
            },
            None => {
                self.since.remove(&key);
            }
        }
    }

    /// Whether the role is behind for at least `window` at the reading of
    /// `began`, by the readings, or else by the version's time on a trusted
    /// clock (`clocks_synced`): `version_time` its stamp's wall-clock time
    /// and `at` this host's clock at the reading.
    pub fn behind(
        &self,
        target: &str,
        role: &str,
        window: Duration,
        began: Instant,
        clock: Option<(SystemTime, SystemTime)>,
    ) -> Option<BehindFor> {
        let (version, since) = self.since.get(&(target.to_owned(), role.to_owned()))?;
        let by_readings = began.saturating_duration_since(*since);
        if by_readings >= window {
            return Some(BehindFor {
                version: *version,
                since: by_readings,
                by: BehindBy::Window,
            });
        }
        let (version_time, at) = clock?;
        let age = at.duration_since(version_time).ok()?;
        (age >= window + freshness::DEFAULT_DELTA).then_some(BehindFor {
            version: *version,
            since: age,
            by: BehindBy::Clock,
        })
    }

    /// The roles followed.
    pub fn followed(&self) -> BTreeSet<(String, String)> {
        self.since.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    fn tasking() -> Contract {
        let l = zenkey_model::contract::load_path(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/tasking.v1.toml"),
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn worker() -> Contract {
        let l = zenkey_model::contract::load_path(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/worker.v1.toml"),
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn entry(bindings: &[&str]) -> RequireEntry {
        RequireEntry {
            role: "task".into(),
            interface: "tasking.v1".into(),
            declared_by: Some("worker.v1".into()),
            bindings: bindings.iter().map(|b| (*b).to_owned()).collect(),
            params: Map::from([
                ("system".to_owned(), "self.system".to_owned()),
                ("service".to_owned(), "self.service".to_owned()),
            ]),
            optional: true,
        }
    }

    const D: &str = "zk2/lab/author/tasking.v1/state/tasks/lab/worker";

    /// The report resource is found by type identity (§2.10), and the
    /// worker's `running`, a `Task`, is not one.
    #[test]
    fn report_resources_are_found_by_type_identity() {
        let w = worker();
        let found: Vec<String> = report_resources(&w)
            .iter()
            .map(|r| r.template.as_str().to_owned())
            .collect();
        assert_eq!(found, ["desired/task"]);
        let own: Addr = "lab/worker".parse().unwrap();
        assert_eq!(
            members_selector(&own, &w.iface, report_resources(&w)[0]),
            "zk2/lab/worker/worker.v1/state/desired/task"
        );
        assert!(report_resources(&tasking()).is_empty());
    }

    /// The slice from the author's contract, as the model's binding builds
    /// it; from the requirement alone, under either kind token; the kill
    /// switch; and a binding the model refuses.
    #[test]
    fn a_role_resolves_its_slice() {
        let own: Addr = "lab/worker".parse().unwrap();
        let t = tasking();
        let templates = vec!["tasks/{system}/{service}".to_owned()];
        let r = resolve_role(
            &own,
            &entry(&["lab/author"]),
            Some(&templates),
            Some((&t, SliceFrom::AuthorContract)),
        )
        .unwrap();
        assert_eq!(r.slice, [D]);
        assert_eq!(r.slice_from, Some(SliceFrom::AuthorContract));
        assert_eq!(
            r.resource.as_deref(),
            Some("state/tasks/{system}/{service}")
        );
        assert!(r.bound());
        let r = resolve_role(&own, &entry(&["lab/author"]), Some(&templates), None).unwrap();
        assert_eq!(
            r.slice,
            [
                D.to_owned(),
                "zk2/lab/author/tasking.v1/@state/tasks/lab/worker".to_owned()
            ]
        );
        assert_eq!(r.slice_from, Some(SliceFrom::Requirement));
        let r = resolve_role(&own, &entry(&[]), Some(&templates), None).unwrap();
        assert!(!r.bound() && r.slice.is_empty());
        let r = resolve_role(
            &own,
            &entry(&["*/author"]),
            Some(&templates),
            Some((&t, SliceFrom::AuthorContract)),
        )
        .unwrap();
        assert_eq!(r.binding_problems, [BindingProblem::NotOneAuthor]);
        assert!(r.slice.is_empty());
        // A requirement on no desired resource is not this profile's.
        let none = vec!["nothing".to_owned()];
        assert!(
            resolve_role(
                &own,
                &entry(&["lab/author"]),
                Some(&none),
                Some((&t, SliceFrom::AuthorContract))
            )
            .is_none()
        );
    }

    fn reply(key: &str, doc: &str) -> ReportReply {
        ReportReply {
            key: key.to_owned(),
            read: dm::decode(doc.as_bytes()),
            stamp: None,
            horizon: Horizon::Undeclared,
        }
    }

    /// The report whose key is the slice's; an unreadable one when none
    /// readable claims it; two that differ are none.
    #[test]
    fn the_report_of_a_slice() {
        let rp = "zk2/lab/worker/worker.v1/state/desired/task";
        let ok = format!(r#"{{"key": "{D}", "source": "baseline"}}"#);
        let replies = vec![reply(rp, &ok), reply(rp, r#"{"source": "baseline"}"#)];
        assert!(matches!(
            report_of(&replies, &[D.to_owned()]),
            ReportFound::One(_)
        ));
        let bad = vec![
            reply(rp, "not json"),
            reply(rp, r#"{"source": "baseline"}"#),
        ];
        assert!(matches!(
            report_of(&bad, &[D.to_owned()]),
            ReportFound::Unreadable(_)
        ));
        let other = format!(r#"{{"key": "{D}", "source": "call"}}"#);
        let two = vec![reply(rp, &ok), reply("zk2/x/y/w.v1/state/r", &other)];
        assert!(matches!(report_of(&two, &[D.to_owned()]), ReportFound::Two));
        assert!(matches!(report_of(&[], &[D.to_owned()]), ReportFound::None));
    }

    fn st(time: u64, id: &str) -> Stamp {
        Stamp::parse(time, id).unwrap()
    }

    /// The author's version: silent, incomplete, the latest of its replies,
    /// a delete, and an unstamped reply, which is no version.
    #[test]
    fn the_authors_current_version() {
        let at = SystemTime::now();
        let get = |complete: bool, replies: Vec<SliceReply>| {
            Ok(SliceGet {
                selector: D.into(),
                complete,
                replies,
                at,
            })
        };
        let r = |delete: bool, s: Option<Stamp>| SliceReply {
            key: D.into(),
            delete,
            stamp: s.map(|s| (s, at)),
        };
        assert_eq!(current_of(None, D).0, Current::NotRead);
        assert_eq!(current_of(Some(&get(true, vec![])), D).0, Current::Silent);
        assert_eq!(
            current_of(Some(&get(false, vec![r(false, Some(st(1, "a1")))])), D).0,
            Current::Incomplete
        );
        assert_eq!(
            current_of(
                Some(&get(
                    true,
                    vec![r(false, Some(st(1, "a1"))), r(true, Some(st(2, "a1")))]
                )),
                D
            )
            .0,
            Current::Delete(st(2, "a1"))
        );
        assert_eq!(
            current_of(Some(&get(true, vec![r(false, None)])), D).0,
            Current::Incomplete
        );
    }

    /// Behind by readings a window apart on one version; a new version
    /// starts the run again; and by a trusted clock at once.
    #[test]
    fn behind_for_a_window() {
        let w = Duration::from_secs(5);
        let t0 = Instant::now();
        let mut b = Behind::default();
        b.note("lab/worker", "task", Some(st(1, "a1")), t0);
        assert!(b.behind("lab/worker", "task", w, t0, None).is_none());
        b.note("lab/worker", "task", Some(st(1, "a1")), t0 + w);
        let got = b.behind("lab/worker", "task", w, t0 + w, None).unwrap();
        assert_eq!((got.by, got.since), (BehindBy::Window, w));
        b.note("lab/worker", "task", Some(st(2, "a1")), t0 + w);
        assert!(b.behind("lab/worker", "task", w, t0 + w, None).is_none());
        let now = SystemTime::now();
        let old = now - Duration::from_secs(6);
        let got = b
            .behind("lab/worker", "task", w, t0 + w, Some((old, now)))
            .unwrap();
        assert_eq!(got.by, BehindBy::Clock);
        b.note("lab/worker", "task", None, t0 + w);
        assert!(b.followed().is_empty());
    }
}
