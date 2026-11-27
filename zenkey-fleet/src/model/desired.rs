//! `desired.v1` (#840, DD) and `desired.v2` (#986's J4): one target's roles
//! as values in hand (`spec/profiles/desired/v1.md` §2.3, §2.10, §2.17, §5;
//! `v2.md` §2.10, §2.17, §2.18).
//!
//! **Either major, per role** (`desired.v2` §2.18). A role is read by the
//! major its author's revision lists in `uses` ([`RoleRead::major`]): the
//! author's version is its reply's Zenoh timestamp under `desired.v1`, and
//! its snapshot entry's in-band stamp under `desired.v2`
//! ([`current_v2_of`]); the target's report is a `desired.v1.Report` found
//! by type identity and its `key`, or the packed `Report` on the resource
//! whose `desired.report` names the role ([`packed_report_of`]). With no
//! revision of the author's in hand, the target's contract says which: a
//! role a `desired.report` names is read as `desired.v2`'s, unless only a
//! `desired.v1.Report` claims it ([`RoleRead::settle`]). A `desired.v2`
//! role has no slice from the target alone (v2 §2.17).
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
use zenkey_model::desired::v2 as d2;
use zenkey_model::desired::{
    self as dm, AuthorReading, Binding, BindingProblem, Current, Declaration, Invalid, Listing,
    ReportRead, Seen, TargetReading,
};
use zenkey_model::freshness::{self, ClockTrust, Horizon, Observation, Reply, StampAge};
use zenkey_model::grammar::{Addr, IfaceId, KindToken};
use zenkey_model::schema::TypeId;
use zenkey_model::seed::Stamp;
use zenkey_model::template::{Segment, Template};
use zenkey_model::wire;

use crate::bus::health::Stamped;
use crate::model::catalog::{Catalog, ContractSet, ContractState, Observed, Revision};
use crate::model::health::Trust;

pub use zenkey_model::desired::v2::Major;

/// The profile's id of a major, as a row names it.
pub fn profile_of(major: Major) -> &'static str {
    match major {
        Major::V1 => dm::PROFILE,
        Major::V2 => d2::PROFILE,
    }
}

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
    /// The contract the descriptor names as declaring the role, when this
    /// tool holds no revision of it: what the requirement names is unread,
    /// so no binding problem is read and there is no slice, never every
    /// resource (text 0.8, F-225). The tool's sight, not the target's
    /// problem.
    pub requirement_unread: Option<String>,
    /// The major the author's revision in hand lists in `uses`
    /// (`desired.v2` §2.18), `desired.v1` for one that lists neither;
    /// `None` with no revision of the author's interface in hand.
    pub author_major: Option<Major>,
    /// Where `desired.v2` finds the role's report (v2 §2.10), by the
    /// contract of the target's that declares the role; `None` when that
    /// contract is not in hand, or the role is a manifest's.
    pub report_at: Option<ReportWhere>,
}

/// Where a role's `desired.v2` report is (v2 §2.10): the model's finding
/// over the contract that declares the role, and that contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportWhere {
    /// The interface of the contract that declares the role.
    pub iface: IfaceId,
    pub at: d2::ReportAt,
    /// The report resource's horizon (v2 §2.10, "Its horizon"), when one
    /// resource names the role.
    pub horizon: Horizon,
    /// The declaring contract uses `freshness.v2` (§2.17 step 4): the
    /// report is aged by its confirmation, its snapshot entry's (core §4.3:
    /// its put carries no Zenoh timestamp), never by a Zenoh timestamp.
    pub aged: bool,
}

impl ReportWhere {
    /// [`d2::report_at_of`] in `declaring`, the contract of `target`'s that
    /// declares `role`.
    pub fn of(target: &Addr, declaring: &Contract, role: &str) -> ReportWhere {
        let at = d2::report_at_of(target, declaring, role);
        let naming: Vec<&Resource> = declaring
            .resources
            .iter()
            .filter(|r| {
                r.annotations
                    .get(d2::REPORT_KEY)
                    .and_then(d2::Names::read)
                    .is_some_and(|n| n.names(role))
            })
            .collect();
        ReportWhere {
            iface: declaring.iface.clone(),
            at,
            horizon: match naming.as_slice() {
                [one] => freshness::horizon_of(one),
                _ => Horizon::Undeclared,
            },
            aged: freshness::uses_v2(declaring),
        }
    }

    /// Whether a resource of the contract names the role by
    /// `desired.report`, validly or not: the target's contract speaks
    /// `desired.v2` of it.
    pub fn names(&self) -> bool {
        self.at != d2::ReportAt::None
    }

    /// The report's key, when one resource names the role and is declared.
    pub fn key(&self) -> Option<String> {
        self.at.key().map(ToString::to_string)
    }
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
            requirement_unread: None,
            author_major: None,
            report_at: None,
        }
    }

    /// Whether the role is bound (§2.3), its kill switch off.
    pub fn bound(&self) -> bool {
        matches!(self.listing, Some(Listing::Listed { bound: true }))
    }

    /// Whether the role is listed, bound or the kill switch: a target
    /// reports either before its tokens (`desired.v2` §2.10, §2.12).
    pub fn listed(&self) -> bool {
        matches!(self.listing, Some(Listing::Listed { .. }))
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
/// (every one when the requirement names none), each with its declaration
/// (§2.2), in the contract's order. One role per document type (§2.1):
/// more than one is `two_documents` (text 0.6, F-218), and none makes the
/// role no target's (F-217).
fn desired_resources<'c>(
    author: &'c Contract,
    templates: Option<&[String]>,
) -> Vec<(&'c Resource, Declaration)> {
    author
        .resources
        .iter()
        .filter(|r| templates.is_none_or(|t| t.iter().any(|t| t == r.template.as_str())))
        .map(|r| (r, declaration_in(author, r)))
        .filter(|(_, d)| *d != Declaration::NotThisProfiles)
        .collect()
}

/// A desired resource's declaration by the major its revision lists
/// (`desired.v2` §2.2): v1 §2.2's reading, and under `desired.v2`
/// `horizon_v1` after it.
fn declaration_in(c: &Contract, r: &Resource) -> Declaration {
    if d2::uses_v2(c) {
        d2::declaration_of(c, r)
    } else {
        dm::declaration_of(r)
    }
}

/// What a role's requirement names, as a tool reads it (core §3.1, §3.3;
/// text 0.7, 0.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// Every resource of the required interface: a requirement naming
    /// none, or a role the component's manifest declares (its descriptor
    /// names no declaring contract, core §3.3; text 0.7).
    Every,
    /// These templates, as the declaring contract lists them.
    These(Vec<String>),
    /// Unread (text 0.8, F-225): the descriptor names this contract as
    /// declaring the role, and no revision of it is in hand, or the one in
    /// hand declares no such role. Never every resource.
    Unread(String),
}

impl Named {
    /// The templates, as [`resolve_role`] takes them: `None` for every
    /// resource, and for an unread requirement, which [`resolve_unread`]
    /// reads instead.
    pub fn templates(&self) -> Option<&[String]> {
        match self {
            Named::These(t) => Some(t),
            Named::Every | Named::Unread(_) => None,
        }
    }
}

/// What `entry`'s requirement names: from the revision of its declaring
/// contract that `descriptor` names, retrieved or held offline; every
/// resource for a manifest's role; unread when that revision is not in
/// hand (text 0.8, F-225).
pub fn requirement_names(
    descriptor: &Descriptor,
    entry: &RequireEntry,
    contracts: &Contracts,
    offline: &ContractSet,
) -> Named {
    let Some(by) = entry.declared_by.as_deref() else {
        return Named::Every;
    };
    match named_revision(descriptor, by, contracts, offline)
        .and_then(|rev| rev.contract().requires.get(&entry.role))
    {
        Some(req) => req.resources.clone().map_or(Named::Every, Named::These),
        None => Named::Unread(by.to_owned()),
    }
}

/// Resolves a role whose requirement is unread (text 0.8, F-225): its
/// listing and its author, no binding problem read and no slice, never
/// every resource. `None` when the author's revision in hand, named or
/// held, declares no desired resource at all: whatever the requirement
/// names, the role is not this profile's, `not_a_target` (F-217).
pub fn resolve_unread(
    entry: &RequireEntry,
    declaring: &str,
    author: Option<(&Contract, SliceFrom)>,
) -> Option<RoleResolution> {
    if author.is_some_and(|(c, _)| desired_resources(c, None).is_empty()) {
        return None;
    }
    let mut out = RoleResolution::bare(
        entry.role.clone(),
        Some(Listing::Listed {
            bound: !entry.bindings.is_empty(),
        }),
    );
    out.interface = IfaceId::from_str(&entry.interface).ok();
    out.author = one_author(&entry.bindings);
    out.requirement_unread = Some(declaring.to_owned());
    out.author_major = author.map(|(c, _)| Major::of(c).unwrap_or(Major::V1));
    Some(out)
}

/// Resolves one `requires` entry of `own`'s descriptor (§2.3): its
/// listing, its author, and its slice from `author`'s contract when one is
/// in hand, else from the requirement's `templates` (the target's contract
/// declaring the role). `None` when the author's contract in hand declares
/// no desired resource the requirement names: the role is not this
/// profile's, not asked, `not_a_target` (text 0.6, F-217). One naming two
/// or more is `two_documents`, no slice (F-218).
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
    out.author_major = author.map(|(c, _)| Major::of(c).unwrap_or(Major::V1));
    let mut req = dm::Requirement {
        bindings: &entry.bindings,
        params: &entry.params,
        optional: entry.optional,
        documents: 1,
    };
    if let Some((contract, from)) = author {
        let named = desired_resources(contract, templates);
        let (r, decl) = named.first().cloned()?;
        out.resource = Some(format!("{}/{}", r.token, r.template.as_str()));
        if named.len() > 1 {
            // One role per document type (§2.1, text 0.6, F-218): the
            // binding gives no slice, whichever resource it would bind.
            req.documents = named.len();
            let slice = dm::Slice {
                iface: &contract.iface,
                token: r.token,
                template: &r.template,
                targets: &[],
            };
            if let Binding::Invalid(why) = dm::binding(&slice, &req, own) {
                out.binding_problems = why;
            }
            return Some(out);
        }
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
/// declares a desired resource `templates` names; else any held offline,
/// which shows the role is no target's (text 0.6, F-217). `None` with no
/// revision of the interface in hand: only then is the slice the target's
/// alone.
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
    // A held revision declaring a desired resource the requirement names;
    // else any held revision of the interface, which shows the role is no
    // target's (text 0.6, F-217): the target alone is read only with none
    // in hand.
    let held: Vec<&Contract> = offline.of_iface(iface).map(|r| r.contract()).collect();
    held.iter()
        .find(|c| !desired_resources(c, templates).is_empty())
        .or(held.first())
        .map(|c| (*c, SliceFrom::OfflineContract))
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

/// Why a reply on a `desired.v2` report's key does not read (v2 §2.10): the
/// refusal's token, core §7.6's or the envelope's, and its words. Never
/// guessed at: an unreadable report answers nothing (v2 §2.17).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedUnreadable {
    pub token: String,
    /// The refusal in words, without its token in front.
    pub says: String,
}

impl PackedUnreadable {
    /// A refusal by its token and its words, a `<token>: ` the words
    /// already lead with taken off.
    pub fn new(token: &str, says: &str) -> PackedUnreadable {
        PackedUnreadable {
            token: token.to_owned(),
            says: says
                .strip_prefix(token)
                .and_then(|rest| rest.strip_prefix(": "))
                .unwrap_or(says)
                .to_owned(),
        }
    }
}

impl std::fmt::Display for PackedUnreadable {
    /// `<token>: <words>`, as a row's `unreadable` spells it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.token, self.says)
    }
}

/// One reply on a `desired.v2` report resource's key (v2 §2.10), as read:
/// the packed `Report` under its envelope's tag.
#[derive(Debug, Clone)]
pub struct PackedReply {
    /// The key it came on, base-relative.
    pub key: String,
    /// The record read, malformed where it has a finding, or why it does
    /// not read.
    pub read: Result<d2::Read, PackedUnreadable>,
    /// Its put's in-band stamp, the target's own (core §4.3).
    pub inband: Option<wire::Stamp>,
    /// What ages it (§2.17 step 4): its Zenoh timestamp, where its
    /// resource's puts carry one; under `freshness.v2`, its snapshot
    /// entry's confirmation instead, dated under the deployment epoch and
    /// stamped under its owner's clock (`freshness.v2` §2.6), none where
    /// the confirmation's time is unknown or no epoch is in hand.
    pub stamp: Option<Stamped>,
    /// Its resource's horizon (v2 §2.10).
    pub horizon: Horizon,
}

/// Reads a reply's record as a `desired.v2` `Report` (v2 §2.10, "How a
/// reader reads one"): under the tag its envelope's CH carried. A put with
/// no envelope carries no tag, so core §7.6 refuses it, `tag`.
pub fn packed_read(tag: Option<u8>, record: &[u8]) -> Result<d2::Read, PackedUnreadable> {
    use zenkey_model::packed::ReadRefusal;
    match tag {
        Some(tag) => {
            d2::read(tag, record).map_err(|r| PackedUnreadable::new(r.token(), r.reason()))
        }
        None => Err(PackedUnreadable::new(
            ReadRefusal::Tag.token(),
            "the put carries no envelope, so no tag to read its record under",
        )),
    }
}

/// S4's GET of one target's report resources.
#[derive(Debug, Clone)]
pub struct ReportsGet {
    pub selectors: Vec<String>,
    /// Whether every GET ran to its final reply.
    pub complete: bool,
    pub replies: Vec<ReportReply>,
    /// The replies on `desired.v2` report resources' keys (v2 §2.10), one a
    /// key after the merge.
    pub packed: Vec<PackedReply>,
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

/// What one reply of S4's GET of a `desired.v2` slice said (v2 §2.5): a
/// member read of one concrete key, answered by snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberReply {
    /// A live entry: a put, with its version.
    Entry(wire::Stamp),
    /// A tombstone: a delete within the author's window, with its version.
    Tombstone(wire::Stamp),
    /// A snapshot with no entry: the author's word that it holds no
    /// document for the target, under its header's run.
    NoEntry { run: wire::Run, asof: wire::Seq },
    /// A reply with no envelope, which carries no version (core S1).
    Unstamped,
    /// A reply whose envelope or page is refused, or a held stamp this
    /// reader did not send (core §4.2): no answer, with why.
    Refused(String),
}

/// S4's GET of a slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceGet {
    pub selector: String,
    /// It ran to its final reply, with no error reply.
    pub complete: bool,
    /// `desired.v1`: each reply on the key, by its Zenoh timestamp.
    pub replies: Vec<SliceReply>,
    /// `desired.v2`: what each reply on the key said, in arrival order,
    /// the entries merged per key (observer §4.1).
    pub member: Vec<MemberReply>,
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
    /// Its in-band stamp, which the archive's record keeps with the
    /// envelope (core §4.4): a `desired.v2` document's version.
    pub inband: Option<wire::Stamp>,
    /// The document's own revision (§2.4), when it carries one.
    pub revision: Option<u64>,
    pub confirmed: bool,
}

/// What one reading read of one role.
#[derive(Debug, Clone)]
pub struct RoleRead {
    /// The major the role is read by (`desired.v2` §2.18): its author's
    /// revision's, [`RoleRead::settle`]d once the reports are read.
    pub major: Major,
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

impl FaceWord {
    /// A tool's word for `role` across a face (`desired.v1` §2.14, §2.17
    /// step 2; `desired.v2` §2.14; observer §7.5): listed, bound to
    /// `author` or, with none, unbound, the kill switch. Neither presence
    /// is read: the target's does not cross, and its author's is the
    /// word's too. The parameter bindings are `params` where any is given,
    /// else as the author's declaration in `offline` orders them (§2.2,
    /// §2.3): its first target parameter `self.system`, its second
    /// `self.service`. Where `offline` does not show that declaration (no
    /// revision declares the role, none of its author's interface is in
    /// hand, or the requirement names no one valid desired resource), the
    /// bindings are left empty, and the reading says what it can.
    ///
    /// # Errors
    ///
    /// A declaration of one target parameter, with no `params`: which of
    /// the two the parameter binds is the deployment's choice (§2.2), so
    /// it is never guessed. The text names the parameter.
    pub fn of_contracts(
        role: &str,
        author: Option<Addr>,
        params: BTreeMap<String, String>,
        offline: &ContractSet,
    ) -> Result<FaceWord, String> {
        let bound = author.is_some();
        let params = if !bound || !params.is_empty() {
            params
        } else {
            declared_params(role, offline)?
        };
        Ok(FaceWord {
            listing: Listing::Listed { bound },
            author,
            params,
            author_across: true,
        })
    }
}

/// The bindings the author's declaration orders for `role`'s target
/// parameters (§2.2): empty where `offline` does not show one declaration.
/// Every revision in `offline` that requires `role` is read, and they must
/// agree: several that order its parameters differently are refused, never
/// the set's first taken (review-bot on #1174).
fn declared_params(role: &str, offline: &ContractSet) -> Result<BTreeMap<String, String>, String> {
    let mut found: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for rev in offline.iter() {
        let Some(req) = rev.contract().requires.get(role) else {
            continue;
        };
        let got = declared_params_of(req, offline)?;
        if !got.is_empty() && !found.iter().any(|(_, f)| f == &got) {
            found.push((rev.contract().iface.to_string(), got));
        }
    }
    match found.len() {
        0 => Ok(BTreeMap::new()),
        1 => Ok(found.remove(0).1),
        _ => Err(format!(
            "--contracts holds several revisions that require role {role} and order its target \
             parameters differently ({}): say each with --param NAME=self.system or \
             NAME=self.service",
            found
                .iter()
                .map(|(i, _)| i.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// [`declared_params`] for one requirement.
fn declared_params_of(
    req: &zenkey_model::contract::Requirement,
    offline: &ContractSet,
) -> Result<BTreeMap<String, String>, String> {
    let none = Contracts::new();
    let Some((author, _)) = author_contract(
        None,
        None,
        &req.interface,
        req.resources.as_deref(),
        &none,
        offline,
    ) else {
        return Ok(BTreeMap::new());
    };
    let named = desired_resources(author, req.resources.as_deref());
    let [(r, decl)] = named.as_slice() else {
        return Ok(BTreeMap::new());
    };
    let Some(targets) = decl.targets() else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for t in targets {
        let bound = match t.binds {
            dm::Binds::System => dm::SELF_SYSTEM,
            dm::Binds::Service => dm::SELF_SERVICE,
            dm::Binds::Either => {
                return Err(format!(
                    "{}'s `{}/{}` has one target parameter, `{}`, which the deployment binds \
                     to `self.system` or `self.service` (desired.v1 §2.2): say which with \
                     --param {}=self.system or --param {}=self.service",
                    author.iface, r.token, r.template, t.param, t.param, t.param
                ));
            }
        };
        out.insert(t.param.clone(), bound.to_owned());
    }
    Ok(out)
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
#[derive(Debug, Clone)]
pub enum ReportFound<'a> {
    /// The readable report whose `key` is the slice's.
    One(&'a ReportReply),
    /// None readable claims the slice, and one did not read: it may be the
    /// slice's (text 0.4).
    Unreadable(&'a ReportReply),
    /// Two readable reports, of two report resources, claim the slice and
    /// differ: unreadable, since the target says two things of one role
    /// (§2.10, text 0.5, F-212). Every readable one claiming it, in reply
    /// order, so a row can name each one's key.
    Two(Vec<&'a ReportReply>),
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
    let Some(&first) = claiming.first() else {
        return replies
            .iter()
            .find(|r| r.read.is_err())
            .map_or(ReportFound::None, ReportFound::Unreadable);
    };
    if claiming
        .iter()
        .all(|r| r.read.as_ref().ok() == first.read.as_ref().ok())
    {
        ReportFound::One(first)
    } else {
        ReportFound::Two(claiming)
    }
}

/// The report reply's freshness (§2.17 step 4, `freshness.v1` §2.6): its
/// resource's horizon, its stamp aged at `at` against a trusted clock.
pub fn report_freshness(
    reply: &ReportReply,
    at: SystemTime,
    trust: &Trust<'_>,
) -> freshness::Judged {
    reply_freshness(reply.stamp.as_ref(), &reply.horizon, at, trust)
}

/// [`report_freshness`] of a reply of either major: its stamp, where its
/// put carried one, aged at `at` under its resource's `horizon`.
pub fn reply_freshness(
    stamp: Option<&Stamped>,
    horizon: &Horizon,
    at: SystemTime,
    trust: &Trust<'_>,
) -> freshness::Judged {
    let observation = Observation::Got {
        reply: Some(Reply::Put {
            stamp_age: stamp.map(|s| StampAge::between(s.time, at)),
        }),
        clock: stamp.map_or(ClockTrust::Untrusted, |s| trust.of(&s.clock)),
    };
    freshness::judge_all(horizon, &[observation])
}

/// A tool's finding of one role's `desired.v2` report (v2 §2.10, "A tool
/// finds a target's report"), among the replies of S4's GET.
#[derive(Debug, Clone)]
pub enum PackedFound<'a> {
    /// None was read: no resource names the role, the one that does is
    /// invalidly declared (its reasons), or no reply is on its key.
    None(Vec<d2::ReportInvalid>),
    /// Two or more resources name the role: unreadable, `two_reports`.
    Two,
    /// The reply on the report's key, its record refused: unreadable.
    Unreadable(&'a PackedReply),
    /// The reply read; malformed where it has a finding.
    One(&'a PackedReply),
}

impl PackedFound<'_> {
    /// The fixture's token (`v2/slices.json`).
    pub fn as_str(&self) -> &'static str {
        match self {
            PackedFound::None(_) => "none",
            PackedFound::Two | PackedFound::Unreadable(_) => "unreadable",
            PackedFound::One(_) => "one",
        }
    }
}

/// Finds a role's `desired.v2` report (v2 §2.10): the reply on the key the
/// target's contract gives it (`at`), among `replies` as the tool's merge
/// left them, one a key. Never by a resource's name, and never a reply on
/// another key.
pub fn packed_report_of<'a>(at: &d2::ReportAt, replies: &'a [PackedReply]) -> PackedFound<'a> {
    match at {
        d2::ReportAt::None => PackedFound::None(Vec::new()),
        d2::ReportAt::Two => PackedFound::Two,
        d2::ReportAt::Invalid(why) => PackedFound::None(why.clone()),
        d2::ReportAt::At { key, .. } => {
            let key = key.to_string();
            match replies.iter().find(|p| p.key == key) {
                None => PackedFound::None(Vec::new()),
                Some(p) if p.read.is_ok() => PackedFound::One(p),
                Some(p) => PackedFound::Unreadable(p),
            }
        }
    }
}

/// The author's answer to S4's GET of a `desired.v2` slice (v2 §2.17 step
/// 6), and why it is no version when it is none. The replies' entries were
/// merged per key (observer §4.1), so at most one entry or tombstone is
/// here. An entry or a tombstone answers before a snapshot with no entry:
/// two instances answering one key, one holding a document. A reply that
/// does not read, or one with no envelope, is a GET that could not answer.
pub fn current_v2_of(get: Option<&Result<SliceGet, String>>) -> (d2::Current, Option<String>) {
    use d2::Current as C;
    match get {
        None => (C::NotRead, None),
        Some(Err(why)) => (
            C::NotRead,
            Some(format!("the GET could not be made: {why}")),
        ),
        Some(Ok(g)) if !g.complete => (
            C::Incomplete,
            Some("the GET ended at its timeout, or with an error reply".to_owned()),
        ),
        Some(Ok(g)) => {
            if g.member.is_empty() {
                return (C::Silent, None);
            }
            if let Some(why) = g.member.iter().find_map(|m| match m {
                MemberReply::Refused(why) => Some(why),
                _ => None,
            }) {
                return (
                    C::Incomplete,
                    Some(format!(
                        "a reply does not read, so it gives no version: {why}"
                    )),
                );
            }
            if g.member.contains(&MemberReply::Unstamped) {
                return (
                    C::Unstamped,
                    Some(
                        "a reply carries no envelope, so no version (core S1, §7.5): its author \
                         broke S1"
                            .to_owned(),
                    ),
                );
            }
            let version = g.member.iter().rev().find_map(|m| match m {
                MemberReply::Entry(v) => Some(C::Entry(*v)),
                MemberReply::Tombstone(v) => Some(C::Tombstone(*v)),
                _ => None,
            });
            let no_entry = g.member.iter().rev().find_map(|m| match m {
                MemberReply::NoEntry { run, .. } => Some(C::NoEntry(*run)),
                _ => None,
            });
            (version.or(no_entry).unwrap_or(C::Silent), None)
        }
    }
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

/// The author's current version dated, when its GET answered one: the
/// version's stamp on the wall clock, and this host's clock as that GET was
/// answered ([`SliceGet::at`]), which a clock trusted to the HLC delta ages
/// it between (review of #856). The GET ends before the reading does, so
/// the age is never longer than the reading's own end would make it.
pub fn current_dated(
    get: Option<&Result<SliceGet, String>>,
    slice: &str,
) -> Option<(SystemTime, SystemTime)> {
    let Some(Ok(g)) = get else { return None };
    g.replies
        .iter()
        .filter(|r| r.key == slice)
        .filter_map(|r| r.stamp)
        .max_by_key(|(s, _)| *s)
        .map(|(_, t)| (t, g.at))
}

/// What a role's reading needs of the report, kept beside it.
#[derive(Debug, Clone)]
pub struct RoleReading<'a> {
    pub target: TargetReading<'a>,
    pub author: AuthorReading,
    /// The report found, when one was.
    pub report: Option<&'a ReportReply>,
    /// The readable reports, of two report resources or more, that claim
    /// the slice and differ, when they did: unreadable together (§2.10,
    /// text 0.5, F-212), never as if none were read. Empty otherwise.
    pub differing: Vec<&'a ReportReply>,
    /// Why step 3 read none, when it read none.
    pub no_report: Option<String>,
    pub freshness: freshness::Judged,
    /// Why the author's presence or its version is not known.
    pub author_why: Option<String>,
}

/// What a `desired.v2` role's reading needs, kept beside it (v2 §2.17).
#[derive(Debug, Clone)]
pub struct RoleReadingV2<'a> {
    pub target: TargetReading<'a, d2::Read>,
    pub author: d2::AuthorReading,
    /// The reply found on the report's key, when one was.
    pub report: Option<&'a PackedReply>,
    /// The report resource's two tables, as the revision in hand of the
    /// target's contract holds them (v2 §2.10, "A reader reads an index");
    /// `None` with no declared report resource in hand: numbers are shown.
    pub tables: Option<&'a d2::Tables>,
    /// Two or more resources name the role: unreadable, `two_reports`.
    pub two_reports: bool,
    /// The reasons of the one report resource that names the role, when
    /// its declaration is invalid.
    pub report_invalid: &'a [d2::ReportInvalid],
    /// Why step 3 read none, when it read none.
    pub no_report: Option<String>,
    pub freshness: freshness::Judged,
    /// Why the author's presence or its version is not known.
    pub author_why: Option<String>,
}

impl RoleRead {
    /// A role resolved and nothing read of it yet, its major its author's
    /// revision's where one is in hand.
    pub fn new(resolution: RoleResolution) -> RoleRead {
        RoleRead {
            major: resolution.author_major.unwrap_or(Major::V1),
            resolution,
            author_seen: None,
            current: None,
            archived: None,
        }
    }

    /// Settles the role's major once its target's reports are read
    /// (`desired.v2` §2.18): its author's revision's, where one is in hand.
    /// With none, the target's contract says: a role a `desired.report`
    /// names is `desired.v2`'s, unless no reply is on that report's key and
    /// a `desired.v1.Report` claims one of the requirement's candidate
    /// slices (a target that serves both majors puts the report of its
    /// author's). A `desired.v2` role has no slice from the target alone
    /// (v2 §2.17): the candidates are dropped.
    pub fn settle(&mut self, reports: Option<&Result<ReportsGet, String>>) {
        let res = &mut self.resolution;
        self.major = res.author_major.unwrap_or_else(|| {
            let Some(at) = res.report_at.as_ref().filter(|w| w.names()) else {
                return Major::V1;
            };
            let replies = reports.and_then(|r| r.as_ref().ok());
            let packed = at
                .key()
                .is_some_and(|k| replies.is_some_and(|g| g.packed.iter().any(|p| p.key == k)));
            let claimed = replies.is_some_and(|g| {
                g.replies.iter().any(|r| {
                    r.read
                        .as_ref()
                        .is_ok_and(|d| d.key.as_ref().is_some_and(|k| res.slice.contains(k)))
                })
            });
            if packed || !claimed {
                Major::V2
            } else {
                Major::V1
            }
        });
        if self.major == Major::V2 && res.slice_from == Some(SliceFrom::Requirement) {
            res.slice.clear();
            res.slice_from = None;
        }
    }

    /// What the target's descriptor, or across a face the deployment's
    /// word, says of the role (§2.17 step 2).
    fn listings(&self, t: &TargetRead) -> (Option<Listing>, Option<Listing>) {
        let res = &self.resolution;
        match (&t.word, &t.descriptor) {
            // The deployment's word stands for the descriptor, but for a
            // role a revision in hand shows on no desired resource
            // (text 0.6, F-217).
            (Some(_), _) if res.listing == Some(Listing::NotListed) => {
                (None, Some(Listing::NotListed))
            }
            (Some(w), _) => (None, Some(w.listing)),
            (None, Ok(Some(_))) | (None, Ok(None)) => (res.listing, None),
            (None, Err(_)) => (None, None),
        }
    }

    /// Why the role has no slice, when it has none.
    fn no_slice(&self) -> String {
        let res = &self.resolution;
        if !res.declaration_invalid.is_empty() {
            format!(
                "the author's declaration of its desired resource is invalid ({}), so the role \
                 has no slice (§2.2)",
                res.declaration_invalid
                    .iter()
                    .map(|i| i.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else if !res.binding_problems.is_empty() {
            format!(
                "its binding is invalid ({}), so the role has no slice (§2.3)",
                res.binding_problems
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else if let Some(by) = &res.requirement_unread {
            format!(
                "its requirement is unread: its descriptor names {by} as the contract declaring \
                 role {}, and no revision of it in hand declares the role, so which resource the \
                 role binds is not known, never every resource (§2.17, text 0.8)",
                res.role
            )
        } else if res.author_major.is_none() {
            "no revision of its author's interface is in hand, so the role has no slice: a \
             desired.v2 report names no key, and the target alone gives none (desired.v2 §2.17)"
                .to_owned()
        } else {
            "its slice could not be built from its binding (§2.3)".to_owned()
        }
    }

    /// This role's reading under `desired.v2` (v2 §2.17): the target's
    /// presence and listing as v1's, its report found by `desired.report`
    /// with no slice needed (v2 §2.10), and the author's version by its
    /// snapshot. `trust` ages a reply's stamp.
    pub fn reading_v2<'a>(&'a self, t: &'a TargetRead, trust: &Trust<'_>) -> RoleReadingV2<'a> {
        let res = &self.resolution;
        let mut out = RoleReadingV2 {
            target: TargetReading {
                presence: t.seen,
                descriptor: None,
                deployment: None,
                report: ReportRead::NotRead,
                freshness: freshness::Verdict::NotAsked,
            },
            author: d2::AuthorReading {
                presence: Seen::Incomplete,
                current: d2::Current::NotRead,
            },
            report: None,
            tables: None,
            two_reports: false,
            report_invalid: &[],
            no_report: None,
            freshness: freshness::judge_all(&Horizon::Undeclared, &[]),
            author_why: None,
        };
        match (&res.requirement_unread, &res.report_at) {
            (Some(by), _) => {
                out.no_report = Some(format!(
                    "its requirement is unread: its descriptor names {by} as the contract \
                     declaring role {}, and no revision of it is in hand, so the resource whose \
                     `desired.report` names the role is not known (desired.v2 §2.10)",
                    res.role
                ));
            }
            (None, None) => {
                out.no_report = Some(format!(
                    "no revision in hand of the contract that declares role {}, so the resource \
                     whose `desired.report` names it is not known (desired.v2 §2.10)",
                    res.role
                ));
            }
            (None, Some(w)) => match &w.at {
                d2::ReportAt::None => {
                    out.no_report = Some(format!(
                        "no resource of {} names role {} by `desired.report`: no report was read \
                         (desired.v2 §2.10)",
                        w.iface, res.role
                    ));
                }
                d2::ReportAt::Two => {
                    out.two_reports = true;
                    out.target.report = ReportRead::Unreadable;
                    out.no_report = Some(format!(
                        "two or more resources of {} name role {} by `desired.report`: the \
                         target's contract says two things of one role (desired.v2 §2.10, \
                         two_reports)",
                        w.iface, res.role
                    ));
                }
                d2::ReportAt::Invalid(why) => {
                    out.report_invalid = why;
                    out.no_report = Some(format!(
                        "the resource of {} whose `desired.report` names role {} is invalidly \
                         declared ({}): no report was read (desired.v2 §2.10)",
                        w.iface,
                        res.role,
                        why.iter()
                            .map(|r| r.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                d2::ReportAt::At { key, tables } => {
                    out.tables = Some(tables);
                    match &t.reports {
                        None => {
                            out.no_report = Some("its report resources were not read".to_owned());
                        }
                        Some(Err(why)) => {
                            out.no_report =
                                Some(format!("the GET of its reports could not be made: {why}"));
                        }
                        Some(Ok(g)) => match packed_report_of(&w.at, &g.packed) {
                            PackedFound::One(p) | PackedFound::Unreadable(p) => {
                                out.report = Some(p);
                                match &p.read {
                                    Ok(r) => {
                                        out.target.report = ReportRead::Read(r);
                                        out.freshness = reply_freshness(
                                            p.stamp.as_ref(),
                                            &p.horizon,
                                            g.at,
                                            trust,
                                        );
                                    }
                                    Err(_) => out.target.report = ReportRead::Unreadable,
                                }
                            }
                            _ => {
                                out.no_report = Some(if g.complete {
                                    format!(
                                        "no reply on its report's key, {key} (desired.v2 §2.10)"
                                    )
                                } else {
                                    format!(
                                        "no reply on its report's key, {key}, and the GET ended \
                                         at its timeout"
                                    )
                                });
                            }
                        },
                    }
                }
            },
        }
        let (descriptor, deployment) = self.listings(t);
        out.target.descriptor = descriptor;
        out.target.deployment = deployment;
        out.target.freshness = out.freshness.verdict;
        let (author_seen, seen_why) = self
            .author_seen
            .clone()
            .unwrap_or((Seen::Incomplete, Some("the author was not read".to_owned())));
        let (current, current_why) = if res.slice.is_empty() {
            (d2::Current::NotRead, Some(self.no_slice()))
        } else {
            current_v2_of(self.current.as_ref())
        };
        out.author = d2::AuthorReading {
            presence: author_seen,
            current,
        };
        out.author_why = seen_why.or(current_why);
        out
    }

    /// This role's reading, as `zenkey_model::desired` takes it (§2.17):
    /// the target's presence, its descriptor or the deployment's word, its
    /// report of the slice and its age, the author's presence and its
    /// version. `trust` ages a reply's stamp.
    pub fn reading<'a>(&'a self, t: &'a TargetRead, trust: &Trust<'_>) -> RoleReading<'a> {
        let res = &self.resolution;
        let mut no_report = None;
        let mut found = None;
        let mut differing = Vec::new();
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
        } else if let Some(by) = &res.requirement_unread {
            no_report = Some(format!(
                "its requirement is unread: its descriptor names {by} as the contract declaring \
                 role {}, and no revision of it in hand declares the role, so which resource the \
                 role binds is not known, never every resource (§2.17, text 0.8)",
                res.role
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
                    ReportFound::Two(all) => {
                        report = ReportRead::Unreadable;
                        no_report = Some(format!(
                            "{} readable reports, of {}, claim the slice and differ: the target \
                             says two things of one role (§2.10)",
                            all.len(),
                            all.iter()
                                .map(|r| r.key.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                        differing = all;
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
        let (descriptor, deployment) = self.listings(t);
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
            differing,
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

/// What an author answered that a target is "not yet" on, of either major:
/// a version, or under `desired.v2` a snapshot with no entry, the author's
/// word that it holds no document (v2 §2.17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorVersion {
    /// `desired.v1`: a Zenoh timestamp, `<time>/<id>`.
    V1(Stamp),
    /// `desired.v2`: an in-band stamp, `<run>/<seq>`. It has no time.
    V2(wire::Stamp),
    /// `desired.v2`: no document, under this run.
    NoEntry(wire::Run),
}

impl std::fmt::Display for AuthorVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthorVersion::V1(s) => write!(f, "{s}"),
            AuthorVersion::V2(v) => f.write_str(&d2::version_text(*v)),
            AuthorVersion::NoEntry(run) => write!(f, "no entry under run {run}"),
        }
    }
}

/// A role found behind one version of its author's for at least the
/// window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BehindFor {
    pub version: AuthorVersion,
    /// How long, at least.
    pub since: Duration,
    pub by: BehindBy,
}

/// Follows each role across readings: the first reading of an unbroken run
/// that found it "not yet" on one version of its author's (§2.17 step 7).
#[derive(Debug, Clone, Default)]
pub struct Behind {
    since: BTreeMap<(String, String), (AuthorVersion, Instant)>,
}

impl Behind {
    /// One reading's answer for a role (`target`, `role`): "not yet" on
    /// `version`, read at `began`; anything else ends its run.
    pub fn note(
        &mut self,
        target: &str,
        role: &str,
        not_yet_on: Option<AuthorVersion>,
        began: Instant,
    ) {
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
    /// clock (`clocks_synced`): `clock` the version's stamp on the wall
    /// clock and this host's clock as its GET was answered
    /// ([`current_dated`]). Only a `desired.v1` version is dated: a
    /// `desired.v2` version has no time, so it is behind by the window
    /// alone (v2 §2.17).
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
        if !matches!(version, AuthorVersion::V1(_)) {
            return None;
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
    /// readable claims it; two that differ are unreadable, each named.
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
        match report_of(&two, &[D.to_owned()]) {
            ReportFound::Two(all) => assert_eq!(
                all.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
                [rp, "zk2/x/y/w.v1/state/r"]
            ),
            other => panic!("two that differ: {other:?}"),
        }
        assert!(matches!(report_of(&[], &[D.to_owned()]), ReportFound::None));
    }

    /// Text 0.5, F-210: with the slice from the target alone, every
    /// template the requirement names, under either kind token, is a
    /// candidate; the one a readable report claims is the slice, and with
    /// none claimed none was read.
    #[test]
    fn the_reports_key_chooses_among_the_requirements_candidates() {
        let own: Addr = "lab/worker".parse().unwrap();
        let templates = vec![
            "plans/{system}/{service}".to_owned(),
            "tasks/{system}/{service}".to_owned(),
        ];
        let r = resolve_role(&own, &entry(&["lab/author"]), Some(&templates), None).unwrap();
        assert_eq!(r.slice_from, Some(SliceFrom::Requirement));
        assert_eq!(
            r.slice,
            [
                "zk2/lab/author/tasking.v1/state/plans/lab/worker",
                "zk2/lab/author/tasking.v1/@state/plans/lab/worker",
                D,
                "zk2/lab/author/tasking.v1/@state/tasks/lab/worker",
            ]
        );
        let rp = "zk2/lab/worker/worker.v1/state/desired/task";
        let plans = "zk2/lab/author/tasking.v1/@state/plans/lab/worker";
        let claimed = vec![reply(
            rp,
            &format!(r#"{{"key": "{plans}", "source": "baseline"}}"#),
        )];
        match report_of(&claimed, &r.slice) {
            ReportFound::One(x) => {
                assert_eq!(x.read.as_ref().unwrap().key.as_deref(), Some(plans));
            }
            other => panic!("{other:?}"),
        }
        let none = vec![reply(
            rp,
            r#"{"key": "zk2/lab/author/tasking.v1/state/other/lab/worker", "source": "baseline"}"#,
        )];
        assert!(matches!(report_of(&none, &r.slice), ReportFound::None));
    }

    /// Text 0.5, F-213: a revision held offline gives the slice, the
    /// contract's kind token winning over the report's: a report keyed under
    /// the other kind token is none.
    #[test]
    fn a_held_revisions_kind_token_wins_over_the_reports() {
        let own: Addr = "lab/worker".parse().unwrap();
        let mut offline = ContractSet::new();
        offline.insert(Revision::from_contract(
            tasking(),
            crate::report::ContractSource::File,
        ));
        let templates = vec!["tasks/{system}/{service}".to_owned()];
        let iface: IfaceId = "tasking.v1".parse().unwrap();
        let author: Addr = "lab/author".parse().unwrap();
        let none = Contracts::new();
        let held = author_contract(
            None,
            Some(&author),
            &iface,
            Some(&templates),
            &none,
            &offline,
        );
        assert!(matches!(held, Some((_, SliceFrom::OfflineContract))));
        let r = resolve_role(&own, &entry(&["lab/author"]), Some(&templates), held).unwrap();
        assert_eq!(r.slice, [D]);
        let rp = "zk2/lab/worker/worker.v1/state/desired/task";
        let explicit = vec![reply(
            rp,
            r#"{"key": "zk2/lab/author/tasking.v1/@state/tasks/lab/worker", "source": "baseline"}"#,
        )];
        assert!(matches!(report_of(&explicit, &r.slice), ReportFound::None));
    }

    /// One revision of the author's interface `iface` from `slices.json`'s
    /// resources, each as `declarations.json` gives one.
    /// Text 0.8, F-225, the shape round 62 measured: a target whose
    /// descriptor names its role's declaring contract at a fingerprint no
    /// one serves, its author's revision in hand declaring two desired
    /// resources. The requirement is unread: no binding problem, no slice,
    /// never `two_documents`, and an author declaring no desired resource
    /// at all still shows the role no target's. Held, the declaring
    /// contract's requirement is read; a manifest's role names every
    /// resource (text 0.7).
    #[test]
    fn a_requirement_whose_declaring_contract_is_not_in_hand_is_unread() {
        use serde_json::json;
        let desired = |t: &str| {
            json!({"annotations": {"desired.target_param": ["system", "service"]},
                   "kind": "state", "params": {"system": "string", "service": "string"},
                   "template": t})
        };
        let two = revision_of(
            "tasking.v1",
            &json!([
                desired("plans/{system}/{service}"),
                desired("tasks/{system}/{service}")
            ]),
        );
        let plain = revision_of(
            "tasking.v1",
            &json!([{"annotations": {}, "kind": "state", "params": {}, "template": "status"}]),
        );
        let w = worker();
        let fp = Revision::from_contract(w.clone(), crate::report::ContractSource::File)
            .fingerprint()
            .to_string();
        let d: Descriptor = serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1", "service": "lab/worker",
            "instance": "0123456789abcdef",
            "interfaces": [{"iface": "worker.v1", "contract": fp, "minor": 0}],
            "requires": [{"role": "task", "interface": "tasking.v1", "declared_by": "worker.v1",
                          "bindings": ["lab/author"],
                          "params": {"system": "self.system", "service": "self.service"}}],
        }))
        .unwrap();
        let e = &d.requires[0];
        let own: Addr = "lab/worker".parse().unwrap();
        let (none, empty) = (Contracts::new(), ContractSet::new());
        let names = requirement_names(&d, e, &none, &empty);
        assert_eq!(names, Named::Unread("worker.v1".into()));
        assert_eq!(names.templates(), None);
        let r = resolve_unread(e, "worker.v1", Some((&two, SliceFrom::AuthorContract))).unwrap();
        assert!(r.bound() && r.slice.is_empty() && r.binding_problems.is_empty());
        assert_eq!(r.requirement_unread.as_deref(), Some("worker.v1"));
        assert_eq!(r.author, Some("lab/author".parse().unwrap()));
        assert!(
            resolve_unread(e, "worker.v1", Some((&plain, SliceFrom::AuthorContract))).is_none(),
            "no desired resource at all: not a target, whatever it names"
        );
        let r = resolve_unread(e, "worker.v1", None).unwrap();
        assert!(r.slice.is_empty() && r.requirement_unread.is_some());
        // Held, the requirement is read: one of the two, the slice.
        let mut held = ContractSet::new();
        held.insert(Revision::from_contract(
            w,
            crate::report::ContractSource::File,
        ));
        let names = requirement_names(&d, e, &none, &held);
        assert_eq!(
            names,
            Named::These(vec!["tasks/{system}/{service}".to_owned()])
        );
        let r = resolve_role(
            &own,
            e,
            names.templates(),
            Some((&two, SliceFrom::AuthorContract)),
        )
        .unwrap();
        assert_eq!(r.slice, [D]);
        assert_eq!(r.requirement_unread, None);
        // A manifest's role: every resource, so two_documents (text 0.7).
        let mut m = e.clone();
        m.declared_by = None;
        assert_eq!(requirement_names(&d, &m, &none, &empty), Named::Every);
        let r = resolve_role(&own, &m, None, Some((&two, SliceFrom::AuthorContract))).unwrap();
        assert_eq!(r.binding_problems, [BindingProblem::TwoDocuments]);
    }

    fn revision_of(iface: &str, resources: &serde_json::Value) -> Contract {
        let (name, major) = iface.rsplit_once(".v").expect("<name>.v<major>");
        let q = |v: &serde_json::Value| serde_json::to_string(v).expect("JSON");
        let mut toml = format!(
            "[interface]\nname = \"{name}\"\nmajor = {major}\nminor = 0\nuses = [\"desired.v1\"]\n"
        );
        for r in resources.as_array().expect("resources") {
            toml.push_str(&format!("[resources.{}]\n", q(&r["template"])));
            toml.push_str(&format!("kind = {}\n", q(&r["kind"])));
            if r["explicit"] == true {
                toml.push_str("explicit = true\n");
            }
            toml.push_str("type = { raw = \"application/octet-stream\" }\n");
            let inline = |m: &serde_json::Value| {
                m.as_object()
                    .expect("an object")
                    .iter()
                    .map(|(k, v)| format!("{} = {}", q(&serde_json::Value::from(k.as_str())), q(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            if r["params"].as_object().is_some_and(|p| !p.is_empty()) {
                toml.push_str(&format!(
                    "params = {{ {} }}\ncardinality = 16\n",
                    inline(&r["params"])
                ));
            }
            if r["annotations"].as_object().is_some_and(|a| !a.is_empty()) {
                toml.push_str(&format!(
                    "annotations = {{ {} }}\n",
                    inline(&r["annotations"])
                ));
            }
        }
        let l = zenkey_model::contract::load_str(&toml, std::path::Path::new("."), None);
        l.contract.unwrap_or_else(|| panic!("{toml}\n{}", l.report))
    }

    /// `spec/profiles/desired/conformance/slices.json` (text 0.5): every
    /// reading of a role before §2.17 answers, through the reference's
    /// resolution and its choice of the report, as the fixture says.
    #[test]
    fn every_slice_reads_as_the_fixture() {
        use serde_json::{Value, json};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/desired/conformance/slices.json");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 20, "{} cases", cases.len());
        for case in cases {
            let r = &case["reading"];
            let iface_s = r["iface"].as_str().unwrap();
            let iface: IfaceId = iface_s.parse().unwrap();
            let own: Addr = r["target"].as_str().unwrap().parse().unwrap();
            let req = &r["requirement"];
            let strings = |v: &Value| -> Vec<String> {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_owned())
                    .collect()
            };
            let entry = RequireEntry {
                role: "task".into(),
                interface: iface_s.into(),
                declared_by: Some("worker.v1".into()),
                bindings: strings(&req["bindings"]),
                params: req["params"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                    .collect(),
                optional: req["optional"] == true,
            };
            let templates = (!req["resources"].is_null()).then(|| strings(&req["resources"]));
            let named = (!r["author"]["named"].is_null())
                .then(|| revision_of(iface_s, &r["author"]["named"]["resources"]));
            let mut offline = ContractSet::new();
            for h in r["author"]["held"].as_array().unwrap() {
                offline.insert(Revision::from_contract(
                    revision_of(iface_s, &h["resources"]),
                    crate::report::ContractSource::File,
                ));
            }
            let none = Contracts::new();
            let author: Option<Addr> = entry.bindings.first().and_then(|b| b.parse().ok());
            let held = match &named {
                Some(c) => Some((c, SliceFrom::AuthorContract)),
                None => author_contract(
                    None,
                    author.as_ref(),
                    &iface,
                    templates.as_deref(),
                    &none,
                    &offline,
                ),
            };
            // A requirement unread (text 0.8, F-225): its declaring
            // contract named and not in hand.
            let unread = req["unread"] == true;
            let resolved = if unread {
                resolve_unread(&entry, "worker.v1", held)
            } else {
                resolve_role(&own, &entry, templates.as_deref(), held)
            };
            let Some(res) = resolved else {
                // A revision in hand declaring no desired resource the
                // requirement names: no target's (text 0.6, F-217).
                let got = json!({
                    "slice": [],
                    "from": null,
                    "binding": [],
                    "declaration": [],
                    "report": {"found": "not_a_target"},
                });
                assert_eq!(got, case["expect"], "{}", case["note"]);
                continue;
            };
            let replies: Vec<ReportReply> = r["replies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| reply(x["key"].as_str().unwrap(), x["text"].as_str().unwrap()))
                .collect();
            let report = if res.requirement_unread.is_some() {
                assert!(res.binding_problems.is_empty(), "{}", case["note"]);
                json!({"found": "requirement_unread"})
            } else if res.slice.is_empty() {
                json!({"found": "no_slice"})
            } else {
                match report_of(&replies, &res.slice) {
                    ReportFound::One(x) => json!({
                        "found": "one",
                        "claims": x.read.as_ref().unwrap().key,
                    }),
                    ReportFound::Two(_) => json!({"found": "unreadable", "why": "two_differ"}),
                    ReportFound::Unreadable(_) => {
                        json!({"found": "unreadable", "why": "unreadable_reply"})
                    }
                    ReportFound::None => json!({"found": "none"}),
                }
            };
            let got = json!({
                "slice": res.slice,
                "from": res.slice_from.map(SliceFrom::as_str),
                "binding": res.binding_problems.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                "declaration": res.declaration_invalid.iter().map(|i| i.as_str()).collect::<Vec<_>>(),
                "report": report,
            });
            assert_eq!(got, case["expect"], "{}", case["note"]);
        }
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
                member: vec![],
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
        let v1 = |t: u64| Some(AuthorVersion::V1(st(t, "a1")));
        b.note("lab/worker", "task", v1(1), t0);
        assert!(b.behind("lab/worker", "task", w, t0, None).is_none());
        b.note("lab/worker", "task", v1(1), t0 + w);
        let got = b.behind("lab/worker", "task", w, t0 + w, None).unwrap();
        assert_eq!((got.by, got.since), (BehindBy::Window, w));
        b.note("lab/worker", "task", v1(2), t0 + w);
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

    fn version(text: &str) -> wire::Stamp {
        d2::parse_version(text).expect("a version")
    }

    /// desired.v2 §2.17: a version has no time, so a target is behind one
    /// by readings a window apart alone, never by a clock, whatever its
    /// trust; a snapshot with no entry is followed as a version is, and a
    /// version after it starts the run again.
    #[test]
    fn a_v2_version_is_behind_by_the_window_alone() {
        let w = Duration::from_secs(5);
        let t0 = Instant::now();
        let now = SystemTime::now();
        let long_ago = Some((now - Duration::from_secs(3600), now));
        let mut b = Behind::default();
        let v6 = AuthorVersion::V2(version("0007/6"));
        b.note("lab/worker", "task", Some(v6), t0);
        assert!(
            b.behind("lab/worker", "task", w, t0, long_ago).is_none(),
            "no clock dates a desired.v2 version"
        );
        b.note("lab/worker", "task", Some(v6), t0 + w);
        let got = b
            .behind("lab/worker", "task", w, t0 + w, long_ago)
            .expect("behind by the readings");
        assert_eq!(
            (got.by, got.version.to_string().as_str()),
            (BehindBy::Window, "0007/6")
        );
        let none = AuthorVersion::NoEntry(version("0007/0").run);
        b.note("lab/worker", "task", Some(none), t0 + w);
        assert!(
            b.behind("lab/worker", "task", w, t0 + w, long_ago)
                .is_none()
        );
        b.note("lab/worker", "task", Some(none), t0 + w + w);
        let got = b.behind("lab/worker", "task", w, t0 + w + w, None).unwrap();
        assert_eq!(got.version.to_string(), "no entry under run 0007");
    }

    /// The author's answer to a desired.v2 member read (v2 §2.5, §2.17
    /// step 6): silent, incomplete, an entry, a tombstone, a snapshot with
    /// no entry under its run, an entry before another instance's no
    /// entry, and a reply that gives no version.
    #[test]
    fn the_authors_answer_to_a_member_read() {
        use d2::Current as C;
        let get = |complete: bool, member: Vec<MemberReply>| {
            Ok(SliceGet {
                selector: D.into(),
                complete,
                replies: vec![],
                member,
                at: SystemTime::now(),
            })
        };
        let v = version("0007/12");
        let no_entry = MemberReply::NoEntry {
            run: version("0008/0").run,
            asof: version("0008/3").seq,
        };
        assert_eq!(current_v2_of(None).0, C::NotRead);
        assert_eq!(current_v2_of(Some(&Err("refused".into()))).0, C::NotRead);
        assert_eq!(current_v2_of(Some(&get(true, vec![]))).0, C::Silent);
        assert_eq!(
            current_v2_of(Some(&get(false, vec![MemberReply::Entry(v)]))).0,
            C::Incomplete
        );
        assert_eq!(
            current_v2_of(Some(&get(true, vec![MemberReply::Entry(v)]))).0,
            C::Entry(v)
        );
        assert_eq!(
            current_v2_of(Some(&get(true, vec![MemberReply::Tombstone(v)]))).0,
            C::Tombstone(v)
        );
        assert_eq!(
            current_v2_of(Some(&get(true, vec![no_entry.clone()]))).0,
            C::NoEntry(version("0008/0").run)
        );
        for both in [
            vec![no_entry.clone(), MemberReply::Entry(v)],
            vec![MemberReply::Entry(v), no_entry.clone()],
        ] {
            assert_eq!(current_v2_of(Some(&get(true, both))).0, C::Entry(v));
        }
        let (c, why) = current_v2_of(Some(&get(
            true,
            vec![MemberReply::Entry(v), MemberReply::Unstamped],
        )));
        assert_eq!(c, C::Unstamped);
        assert!(why.unwrap().contains("no envelope"));
        let (c, why) = current_v2_of(Some(&get(
            true,
            vec![MemberReply::Refused("held: not sent".into()), no_entry],
        )));
        assert_eq!(c, C::Incomplete);
        assert!(why.unwrap().contains("held: not sent"));
    }

    fn worker_v2() -> Contract {
        let l = zenkey_model::contract::load_path(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/worker.v2.toml"),
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    fn tasking_v2() -> Contract {
        let l = zenkey_model::contract::load_path(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/tasking.v2.toml"),
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    const D_V2: &str = "zk2/lab/author/tasking.v2/state/tasks/lab/worker";
    const RP_V2: &str = "zk2/lab/worker/worker.v2/state/desired/task";

    fn packed(key: &str, report: &d2::Report) -> PackedReply {
        PackedReply {
            key: key.to_owned(),
            read: packed_read(Some(d2::TAG), &report.write().unwrap().bytes),
            inband: None,
            stamp: None,
            horizon: Horizon::Undeclared,
        }
    }

    fn reports(replies: Vec<ReportReply>, packed: Vec<PackedReply>) -> Result<ReportsGet, String> {
        Ok(ReportsGet {
            selectors: vec![],
            complete: true,
            replies,
            packed,
            at: SystemTime::now(),
        })
    }

    /// desired.v2 §2.18: a role is read by its author's revision's major;
    /// the scenarios' `worker.v2` reports it on the resource whose
    /// `desired.report` names it, with its two tables.
    #[test]
    fn a_role_is_read_by_its_authors_major() {
        let own: Addr = "lab/worker".parse().unwrap();
        let mut e = entry(&["lab/author"]);
        e.interface = "tasking.v2".into();
        e.declared_by = Some("worker.v2".into());
        let templates = vec!["tasks/{system}/{service}".to_owned()];
        let author = tasking_v2();
        let mut res = resolve_role(
            &own,
            &e,
            Some(&templates),
            Some((&author, SliceFrom::AuthorContract)),
        )
        .unwrap();
        assert_eq!(res.author_major, Some(Major::V2));
        assert_eq!(res.slice, [D_V2]);
        res.report_at = Some(ReportWhere::of(&own, &worker_v2(), "task"));
        let w = res.report_at.as_ref().unwrap();
        assert!(w.names());
        assert_eq!(w.key().as_deref(), Some(RP_V2));
        assert_eq!(w.horizon, Horizon::Undeclared);
        let d2::ReportAt::At { tables, .. } = &w.at else {
            panic!("{:?}", w.at);
        };
        assert_eq!(tables.pointers[3], "/endpoint");
        let mut r = RoleRead::new(res);
        assert_eq!(r.major, Major::V2);
        r.settle(None);
        assert_eq!((r.major, r.resolution.slice.len()), (Major::V2, 1));
        // A desired.v1 author's role stays desired.v1's, whatever the
        // target's contract reports in.
        let res = resolve_role(
            &own,
            &entry(&["lab/author"]),
            Some(&templates),
            Some((&tasking(), SliceFrom::AuthorContract)),
        )
        .unwrap();
        assert_eq!(res.author_major, Some(Major::V1));
        let mut r = RoleRead::new(res);
        r.resolution.report_at = Some(ReportWhere::of(&own, &worker_v2(), "task"));
        r.settle(Some(&reports(
            vec![],
            vec![packed(RP_V2, &d2::Report::baseline())],
        )));
        assert_eq!(r.major, Major::V1);
        assert_eq!(r.resolution.slice, [D]);
    }

    /// desired.v2 §2.17, "The slice from the target alone goes": with no
    /// revision of the author's interface in hand, a role its target's
    /// contract reports by `desired.report` is desired.v2's and has no
    /// slice, the requirement's candidates dropped; a role only a
    /// `desired.v1.Report` claims stays desired.v1's with its candidates;
    /// and a target that serves both is read by the report it put.
    #[test]
    fn with_no_author_revision_the_targets_contract_says_the_major() {
        let own: Addr = "lab/worker".parse().unwrap();
        let templates = vec!["tasks/{system}/{service}".to_owned()];
        let alone = || {
            let res = resolve_role(&own, &entry(&["lab/author"]), Some(&templates), None).unwrap();
            assert_eq!(res.author_major, None);
            assert_eq!(res.slice_from, Some(SliceFrom::Requirement));
            RoleRead::new(res)
        };
        let v1_report = || {
            reply(
                "zk2/lab/worker/worker.v1/state/desired/task",
                &format!(r#"{{"key": "{D}", "source": "baseline"}}"#),
            )
        };
        // No `desired.report` names the role: desired.v1, its candidates kept.
        let mut r = alone();
        r.resolution.report_at = Some(ReportWhere::of(&own, &worker(), "task"));
        assert!(!r.resolution.report_at.as_ref().unwrap().names());
        r.settle(Some(&reports(vec![v1_report()], vec![])));
        assert_eq!((r.major, r.resolution.slice.len()), (Major::V1, 2));
        // The target's contract not in hand either: desired.v1's reading.
        let mut r = alone();
        r.settle(None);
        assert_eq!(r.major, Major::V1);
        // `desired.report` names it: desired.v2, and no slice.
        let at = || Some(ReportWhere::of(&own, &worker_v2(), "task"));
        let mut r = alone();
        r.resolution.report_at = at();
        r.settle(Some(&reports(
            vec![],
            vec![packed(RP_V2, &d2::Report::baseline())],
        )));
        assert_eq!(r.major, Major::V2);
        assert!(r.resolution.slice.is_empty() && r.resolution.slice_from.is_none());
        // Named, and nothing read on its key nor claimed: desired.v2 still.
        let mut r = alone();
        r.resolution.report_at = at();
        r.settle(Some(&reports(vec![], vec![])));
        assert_eq!(r.major, Major::V2);
        // A target that serves both, its author on desired.v1: only the
        // desired.v1.Report was put, and it claims a candidate.
        let mut r = alone();
        r.resolution.report_at = at();
        r.settle(Some(&reports(vec![v1_report()], vec![])));
        assert_eq!((r.major, r.resolution.slice.len()), (Major::V1, 2));
        // Both put: the packed one on the role's key says desired.v2.
        let mut r = alone();
        r.resolution.report_at = at();
        r.settle(Some(&reports(
            vec![v1_report()],
            vec![packed(RP_V2, &d2::Report::baseline())],
        )));
        assert_eq!(r.major, Major::V2);
    }

    /// `spec/profiles/desired/conformance/v2/slices.json` (desired.v2
    /// §2.10), through the fleet's reading of a reply ([`packed_read`]) and
    /// its choice of the role's report among a GET's replies
    /// ([`packed_report_of`]): a tool finds a role's report exactly as the
    /// fixture does, and never by a resource's name.
    #[test]
    fn every_v2_slice_reads_as_the_fixture() {
        use serde_json::{Value, json};
        use zenkey_model::authoring::Kind;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/desired/conformance/v2/slices.json");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 17);
        let unhex = |h: &str| -> Vec<u8> {
            (0..h.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
                .collect()
        };
        for case in cases {
            let what = &case["note"];
            let r = &case["reading"];
            let target: Addr = r["target"].as_str().unwrap().parse().unwrap();
            let iface: IfaceId = r["iface"].as_str().unwrap().parse().unwrap();
            let requires: Vec<&str> = r["requires"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_str().unwrap())
                .collect();
            struct Held {
                kind: Kind,
                template: Template,
                token: KindToken,
                is_report: bool,
                annotations: BTreeMap<String, Value>,
            }
            let held: Vec<Held> = r["resources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| {
                    // A report resource of any kind (text 0.2): its kind
                    // and its token as the fixture writes them.
                    let (kind, token) = match (x["kind"].as_str().unwrap(), x["explicit"] == true) {
                        ("state", true) => (Kind::State, KindToken::ExplicitState),
                        ("state", false) => (Kind::State, KindToken::State),
                        ("stream", _) => (Kind::Stream, KindToken::Stream),
                        ("event", _) => (Kind::Event, KindToken::Events),
                        ("operation", _) => (Kind::Operation, KindToken::Op),
                        (other, _) => panic!("{what}: kind {other:?}"),
                    };
                    let ty = &x["type"];
                    Held {
                        kind,
                        template: Template::parse(x["template"].as_str().unwrap()).unwrap(),
                        token,
                        is_report: ty["kind"] == "packed"
                            && ty["name"] == d2::REPORT
                            && d2::is_published(ty["artifact"].as_str().unwrap()),
                        annotations: x["annotations"]
                            .as_object()
                            .unwrap()
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    }
                })
                .collect();
            let views: Vec<d2::ReportResource<'_>> = held
                .iter()
                .map(|h| d2::ReportResource {
                    kind: h.kind,
                    token: h.token,
                    template: &h.template,
                    is_report: h.is_report,
                    annotations: &h.annotations,
                })
                .collect();
            let at = d2::report_at(&target, &iface, r["role"].as_str().unwrap(), &views, |n| {
                requires.contains(&n)
            });
            // The GET's replies, each read as the bus reads one.
            let replies: Vec<PackedReply> = r["replies"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| PackedReply {
                    key: x["key"].as_str().unwrap().to_owned(),
                    read: packed_read(
                        Some(u8::try_from(x["tag"].as_u64().unwrap()).unwrap()),
                        &unhex(x["hex"].as_str().unwrap()),
                    ),
                    inband: None,
                    stamp: None,
                    horizon: Horizon::Undeclared,
                })
                .collect();
            let found = packed_report_of(&at, &replies);
            let reasons = |why: &[d2::ReportInvalid]| {
                json!(why.iter().map(|w| w.as_str()).collect::<Vec<_>>())
            };
            let mut got = json!({
                "found": found.as_str(),
                "key": at.key().map(|k| k.to_string()),
                "reasons": match &found {
                    PackedFound::None(why) => reasons(why),
                    _ => json!([]),
                },
            });
            match &found {
                PackedFound::None(_) => {}
                PackedFound::Two => got["why"] = json!("two_reports"),
                PackedFound::Unreadable(p) => {
                    got["why"] = json!(p.read.as_ref().unwrap_err().token);
                }
                PackedFound::One(p) => {
                    got["findings"] = json!(
                        p.read
                            .as_ref()
                            .unwrap()
                            .findings
                            .iter()
                            .map(|f| f.as_str())
                            .collect::<Vec<_>>()
                    );
                }
            }
            assert_eq!(got, case["expect"], "{what}");
        }
        // A put with no envelope carries no tag: core §7.6 refuses it.
        let bare = packed_read(None, &[0, 0, 0]).unwrap_err();
        assert_eq!(bare.token, "tag");
    }
}
