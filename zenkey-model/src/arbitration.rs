//! `arbitration.v1`: one commander at a time, in the binding's order
//! (`spec/profiles/arbitration/v1.md`, #909).
//!
//! The profile's session-free half: a role's declaration read from its
//! contract's requirement and the revision of the required interface the
//! consumer holds (§2.2, §2.3), the first profile to read a requirement's
//! annotations; the ranking of a binding's providers (§2.4); one role's
//! arbitration as a pure state machine, which the runtime and the tools
//! both drive (§2.5 to §2.11); and §5's three questions, which never fold
//! an unestablished answer into no (§2.14).
//!
//! | Rule | Item |
//! |---|---|
//! | §2.2 the declaration: arbitrated, invalid, unread | [`declaration`], [`declaration_of`], [`Declaration`], [`Invalid`], [`Role`], [`Revision`] |
//! | §2.3 the deadline, from the revision held | [`deadline_of`], [`Arbitrated::deadline`] |
//! | §2.4 the ranking, wildcards, shadowed entries | [`Binding`], [`Entry`], [`Rank`] |
//! | §2.5 samples and who sent them | [`Arbiter::identify`], [`Sample`], [`Outcome`] |
//! | §2.6 to §2.8 eligibility, withdrawals, a move | [`Arbiter`], [`Event`] |
//! | §2.9 to §2.11 selection, switches, the dead-man | [`Arbiter::observe`], [`Step`], [`Switch`], [`Cause`] |
//! | §2.14, §5 what a tool concludes | [`judge_declaration`], [`judge_selection`], [`judge_binding`], [`held_over`] |
//!
//! ```text
//! declaration(role, revision held, params) ─▶ arbitrated {priority, stream, deadline} │ invalid [reasons] │ unread │ not arbitrated   §2.2
//! Binding::rank(address)                   ─▶ (entry, system, service), compared in that order                          §2.4
//! Arbiter::start / observe(at, event)      ─▶ switches {start│sample│deadline│withdrawn}, an outcome, reads again        §2.5–§2.11
//! Arbiter::due()                           ─▶ the instant to arm the timer for: the active provider's deadline
//! judge_declaration / _selection / _binding ─▶ §5's questions, in five states                                         §2.14
//! ```
//!
//! There is no I/O and no clock here. Every instant is a [`Duration`] since
//! an origin the caller chooses on its own monotonic clock (`timing.v1`'s
//! deadline runs on the consumer's). A consumer's runtime starts the
//! [`Arbiter`] at its start, hands it each sample of the role's
//! subscription ([`Event::Sample`]), each interface token its watch hears
//! declared or withdrawn ([`Event::Declared`], [`Event::Withdrawn`]), each
//! move of the watch session's transport and the round trip after it
//! ([`Event::Moved`], [`Event::Stood`]), and its timer ([`Event::Tick`]),
//! armed for [`Arbiter::due`]; it reads again each address a [`Step`]
//! names in `reread` and hands the read back ([`Event::Read`]); it
//! delivers a sample whose [`Outcome`] is [`Outcome::Delivered`], and
//! tells its application of each [`Switch`].
//!
//! `timing.v1`'s own declaration is read here by [`deadline_of`] as
//! `arbitration.v1` text 0.1 cites it; once `zenkey_model::timing` lands
//! (#908), it is that module's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::authoring::RequireCardinality;
use crate::contract::{Contract, Requirement};
use crate::grammar::{Addr, IfaceId, InstanceId, KeyError, KindToken, Name, ZkKey, parse};
use crate::template::Template;

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "arbitration.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "arbitration";

/// `arbitration.policy` (§4), on a requirement.
pub const POLICY: &str = "arbitration.policy";

/// The published vocabulary (§4): the keys under `arbitration.`, which
/// W105 reads in place of core Appendix D's interim row (core 0.54), the
/// same key.
pub const VOCABULARY: &[&str] = &["policy"];

/// `timing.v1`'s profile name, and the keys of its this profile reads
/// (§2.3).
const TIMING: &str = "timing";
const DEADLINE_MS: &str = "timing.deadline_ms";
const PERIOD_MS: &str = "timing.period_ms";

// --- §2.2: the declaration -------------------------------------------------------

/// How a consumer chooses among a role's providers (§4). 1.0 defines one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Policy {
    /// The first eligible provider in the binding's order (§2.4 to §2.11).
    Priority,
}

impl Policy {
    /// The vocabulary's value, and the fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Priority => "priority",
        }
    }
}

impl fmt::Display for Policy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a declaration is invalid (§2.2), in the order checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invalid {
    /// The policy carried, and the declaring contract's `uses` naming no
    /// major of the profile (core E020's).
    NotUsed,
    /// A component's manifest declares the role.
    ManifestRole,
    /// `freshest` or `lease`, reserved (§4).
    PolicyUnsupported,
    /// Not one of the strings `priority`, `freshest`, `lease`.
    Value,
    /// The role's cardinality is not `many`.
    Cardinality,
    /// The role does not consume exactly one stream.
    Resource,
    /// That stream has no deadline as `timing.v1` reads it.
    NoDeadline,
    /// A parameter of that stream's template is not bound (core R2).
    ParamsUnbound,
}

impl Invalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotUsed => "not_used",
            Self::ManifestRole => "manifest_role",
            Self::PolicyUnsupported => "policy_unsupported",
            Self::Value => "value",
            Self::Cardinality => "cardinality",
            Self::Resource => "resource",
            Self::NoDeadline => "no_deadline",
            Self::ParamsUnbound => "params_unbound",
        }
    }

    /// What it says, for a linter's line or a refusal.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotUsed => "arbitration.policy is written, and uses does not list arbitration.v1",
            Self::ManifestRole => {
                "only a contract's role is arbitrated: a manifest's role reaches no descriptor with its policy"
            }
            Self::PolicyUnsupported => {
                "freshest and lease are reserved; arbitration.v1 1.0 defines priority only"
            }
            Self::Value => "arbitration.policy is the string priority",
            Self::Cardinality => "an arbitrated role has cardinality many",
            Self::Resource => "an arbitrated role consumes exactly one stream resource",
            Self::NoDeadline => {
                "the role's stream declares a timing.v1 deadline, at or above its period"
            }
            Self::ParamsUnbound => {
                "every parameter of the role's stream is bound by the role's parameter bindings"
            }
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.says())
    }
}

/// A valid declaration (§2.2): what an [`Arbiter`] needs of the role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arbitrated {
    pub policy: Policy,
    /// The required interface.
    pub iface: IfaceId,
    /// The role's stream: its kind token (`stream` or `@stream`) and its
    /// template.
    pub token: KindToken,
    pub template: Template,
    /// The stream's deadline in the revision held (§2.3).
    pub deadline: Duration,
    /// Its period, when written.
    pub period: Option<Duration>,
}

impl Arbitrated {
    /// The stream as the fixture writes it, `<token>/<template>`.
    #[must_use]
    pub fn resource(&self) -> String {
        format!("{}/{}", self.token.as_str(), self.template.as_str())
    }
}

/// A role's declaration (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// No `arbitration.policy`, or another major's: not this profile's. A
    /// `many` role receives from every bound provider, merged (core §3.2).
    NotArbitrated,
    /// No revision of the required interface held, checks 1 to 5 passing.
    Unread,
    /// The reasons, in the order checked.
    Invalid(Vec<Invalid>),
    /// Arbitrated.
    Arbitrated(Arbitrated),
}

impl Declaration {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotArbitrated => "not_arbitrated",
            Self::Unread => "unread",
            Self::Invalid(_) => "invalid",
            Self::Arbitrated(_) => "arbitrated",
        }
    }

    /// The valid declaration, if it is one.
    #[must_use]
    pub fn arbitrated(&self) -> Option<&Arbitrated> {
        match self {
            Self::Arbitrated(a) => Some(a),
            _ => None,
        }
    }

    /// Whether the role carries a policy: arbitrated, invalid or unread.
    #[must_use]
    pub fn carries_policy(&self) -> bool {
        !matches!(self, Self::NotArbitrated)
    }
}

/// A role, as §2.2 reads it.
#[derive(Debug, Clone, Copy)]
pub struct Role<'a> {
    /// The declaring contract's `uses`, or `None` for a role a component's
    /// manifest declares.
    pub declared_by: Option<&'a [IfaceId]>,
    /// The requirement's `resources` (templates), or `None`: every one.
    pub resources: Option<&'a [String]>,
    pub cardinality: RequireCardinality,
    pub annotations: &'a BTreeMap<String, Value>,
}

impl<'a> Role<'a> {
    /// A role its contract declares.
    #[must_use]
    pub fn of(declaring: &'a Contract, req: &'a Requirement) -> Self {
        Self {
            declared_by: Some(declaring.uses.as_slice()),
            resources: req.resources.as_deref(),
            cardinality: req.cardinality,
            annotations: &req.annotations,
        }
    }

    /// A role a component's manifest declares.
    #[must_use]
    pub fn of_manifest(req: &'a Requirement) -> Self {
        Self {
            declared_by: None,
            resources: req.resources.as_deref(),
            cardinality: req.cardinality,
            annotations: &req.annotations,
        }
    }
}

/// One resource of the required interface's revision, as §2.2 reads it.
#[derive(Debug, Clone, Copy)]
pub struct Offered<'a> {
    pub token: KindToken,
    pub template: &'a Template,
    /// Its resolved annotations (core §9.3).
    pub annotations: &'a BTreeMap<String, Value>,
}

/// The revision of the required interface a reader holds (§2.3).
#[derive(Debug, Clone)]
pub struct Revision<'a> {
    pub iface: &'a IfaceId,
    pub uses: &'a [IfaceId],
    pub resources: Vec<Offered<'a>>,
}

impl<'a> Revision<'a> {
    /// A contract's revision.
    #[must_use]
    pub fn of(c: &'a Contract) -> Self {
        Self {
            iface: &c.iface,
            uses: &c.uses,
            resources: c
                .resources
                .iter()
                .map(|r| Offered {
                    token: r.token,
                    template: &r.template,
                    annotations: &r.annotations,
                })
                .collect(),
        }
    }
}

/// The deadline and the period of one stream of a revision, as `timing.v1`
/// reads them (§2.3, §2.2 check 7; `timing.v1` §2.? (declaration)): its
/// revision's `uses` lists `timing.v1`; `timing.deadline_ms` is an integer
/// of 1 or more; `timing.period_ms` is absent or an integer of 1 or more,
/// and not above the deadline. `None` is no deadline.
#[must_use]
pub fn deadline_of(rev: &Revision<'_>, r: &Offered<'_>) -> Option<(Duration, Option<Duration>)> {
    if !rev
        .uses
        .iter()
        .any(|u| u.name() == TIMING && u.major() == 1)
    {
        return None;
    }
    let ms = |v: &Value| v.as_u64().filter(|n| *n > 0);
    let deadline = ms(r.annotations.get(DEADLINE_MS)?)?;
    let period = match r.annotations.get(PERIOD_MS) {
        None => None,
        Some(v) => Some(ms(v)?),
    };
    if period.is_some_and(|p| deadline < p) {
        return None;
    }
    Some((
        Duration::from_millis(deadline),
        period.map(Duration::from_millis),
    ))
}

/// Reads a role's declaration (§2.2) from the role, the revision of the
/// required interface held (`None`: none held), and the role's parameter
/// bindings as configured (core R2). Each check runs alone, in §2.2's
/// order; checks 7 and 8 only when check 6 passes, and 6 to 8 only with a
/// revision held.
#[must_use]
pub fn declaration(
    role: &Role<'_>,
    provider: Option<&Revision<'_>>,
    params: &BTreeMap<String, String>,
) -> Declaration {
    let Some(policy) = role.annotations.get(POLICY) else {
        return Declaration::NotArbitrated;
    };
    let mut why = Vec::new();
    match role.declared_by {
        Some(uses) => {
            let v1 = uses.iter().any(|u| u.name() == NAME && u.major() == 1);
            let any = uses.iter().any(|u| u.name() == NAME);
            if any && !v1 {
                // Another major's text: its key is its own.
                return Declaration::NotArbitrated;
            }
            if !any {
                why.push(Invalid::NotUsed);
            }
        }
        None => why.push(Invalid::ManifestRole),
    }
    match policy.as_str() {
        Some("priority") => {}
        Some("freshest" | "lease") => why.push(Invalid::PolicyUnsupported),
        _ => why.push(Invalid::Value),
    }
    if role.cardinality != RequireCardinality::Many {
        why.push(Invalid::Cardinality);
    }
    let Some(rev) = provider else {
        return if why.is_empty() {
            Declaration::Unread
        } else {
            Declaration::Invalid(why)
        };
    };
    let consumed: Vec<&Offered<'_>> = rev
        .resources
        .iter()
        .filter(|r| match role.resources {
            None => true,
            Some(names) => names.iter().any(|n| n == r.template.as_str()),
        })
        .collect();
    let one = match consumed.as_slice() {
        [r] if matches!(r.token, KindToken::Stream | KindToken::ExplicitStream) => Some(*r),
        _ => {
            why.push(Invalid::Resource);
            None
        }
    };
    let mut deadline = None;
    if let Some(r) = one {
        deadline = deadline_of(rev, r);
        if deadline.is_none() {
            why.push(Invalid::NoDeadline);
        }
        if r.template.params().any(|(p, _)| !params.contains_key(p)) {
            why.push(Invalid::ParamsUnbound);
        }
    }
    match (one, deadline) {
        (Some(r), Some((deadline, period))) if why.is_empty() => {
            Declaration::Arbitrated(Arbitrated {
                policy: Policy::Priority,
                iface: rev.iface.clone(),
                token: r.token,
                template: r.template.clone(),
                deadline,
                period,
            })
        }
        _ => Declaration::Invalid(why),
    }
}

/// [`declaration`] of the role `role` of the contract `declaring`, over the
/// revision `provider` of its required interface (`None`: none held).
/// `None` when the contract declares no such role.
#[must_use]
pub fn declaration_of(
    declaring: &Contract,
    role: &str,
    provider: Option<&Contract>,
    params: &BTreeMap<String, String>,
) -> Option<Declaration> {
    let req = declaring.requires.get(role)?;
    let rev = provider.map(Revision::of);
    Some(declaration(&Role::of(declaring, req), rev.as_ref(), params))
}

// --- §2.4: the ranking -----------------------------------------------------------

/// One entry of a binding: `<system>/<service>`, each chunk a plain chunk
/// or `*` (core §1.2, D009), a `self.system` already resolved (core §3.2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entry {
    /// `None` is `*`.
    system: Option<Name>,
    service: Option<Name>,
}

impl Entry {
    /// Parses an entry.
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let (sys, svc) = s.split_once('/').ok_or(KeyError::Shape(
            s.to_owned(),
            "a binding entry is <system>/<service>",
        ))?;
        let chunk = |what: &'static str, c: &str| -> Result<Option<Name>, KeyError> {
            if c == "*" {
                Ok(None)
            } else {
                Name::new(what, c).map(Some)
            }
        };
        Ok(Self {
            system: chunk("system", sys)?,
            service: chunk("service", svc)?,
        })
    }

    /// Whether either chunk is `*`.
    #[must_use]
    pub fn is_wildcard(&self) -> bool {
        self.system.is_none() || self.service.is_none()
    }

    /// The address of an exact entry.
    #[must_use]
    pub fn exact(&self) -> Option<Addr> {
        Some(Addr {
            system: self.system.clone()?,
            service: self.service.clone()?,
        })
    }

    /// Whether it matches an address: each chunk `*` or equal (§0).
    #[must_use]
    pub fn matches(&self, a: &Addr) -> bool {
        self.system.as_ref().is_none_or(|s| *s == a.system)
            && self.service.as_ref().is_none_or(|s| *s == a.service)
    }

    /// Whether it matches every address `later` does: each of its chunks
    /// `*` or equal to `later`'s (§2.4, a shadowed entry).
    #[must_use]
    pub fn covers(&self, later: &Entry) -> bool {
        fn chunk(a: Option<&Name>, b: Option<&Name>) -> bool {
            match (a, b) {
                (None, _) => true,
                (Some(a), Some(b)) => a == b,
                (Some(_), None) => false,
            }
        }
        chunk(self.system.as_ref(), later.system.as_ref())
            && chunk(self.service.as_ref(), later.service.as_ref())
    }
}

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = |n: &Option<Name>| n.as_ref().map_or("*", Name::as_str).to_owned();
        write!(f, "{}/{}", c(&self.system), c(&self.service))
    }
}

/// A provider's rank (§2.4): the position of the first entry matching it,
/// then its system, then its service, each compared byte by byte. Lower is
/// higher priority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rank {
    entry: usize,
    system: String,
    service: String,
}

impl Rank {
    /// The 0-based position of the entry it ranks at.
    #[must_use]
    pub fn entry(&self) -> usize {
        self.entry
    }
}

/// A role's binding, its entries in their configured order (core R1, R3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Binding {
    entries: Vec<Entry>,
}

impl Binding {
    /// Parses a binding as a descriptor lists it.
    pub fn parse<S: AsRef<str>>(entries: &[S]) -> Result<Self, KeyError> {
        Ok(Self {
            entries: entries
                .iter()
                .map(|e| Entry::parse(e.as_ref()))
                .collect::<Result<_, _>>()?,
        })
    }

    /// The entries, in order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether it has no entry: an unbound role.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// An address's rank, `None` when no entry matches it (§2.4).
    #[must_use]
    pub fn rank(&self, a: &Addr) -> Option<Rank> {
        let entry = self.entries.iter().position(|e| e.matches(a))?;
        Some(Rank {
            entry,
            system: a.system.as_str().to_owned(),
            service: a.service.as_str().to_owned(),
        })
    }

    /// The providers in rank order: every exact entry's address, heard or
    /// not, and every heard address an entry matches, once each (§2.4).
    #[must_use]
    pub fn ranked<'a, I: IntoIterator<Item = &'a Addr>>(&self, heard: I) -> Vec<Addr> {
        let mut by: BTreeMap<Rank, Addr> = BTreeMap::new();
        let mut add = |a: &Addr| {
            if let Some(r) = self.rank(a) {
                by.entry(r).or_insert_with(|| a.clone());
            }
        };
        for a in self.entries.iter().filter_map(Entry::exact) {
            add(&a);
        }
        for a in heard {
            add(a);
        }
        by.into_values().collect()
    }

    /// The heard addresses no entry matches, by system then service, once
    /// each.
    #[must_use]
    pub fn unbound<'a, I: IntoIterator<Item = &'a Addr>>(&self, heard: I) -> Vec<Addr> {
        heard
            .into_iter()
            .filter(|a| self.rank(a).is_none())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// The 0-based positions of the wildcard entries.
    #[must_use]
    pub fn wildcards(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.is_wildcard())
            .map(|(i, _)| i)
            .collect()
    }

    /// The 0-based positions of the shadowed entries: an entry an earlier
    /// one covers (§2.4).
    #[must_use]
    pub fn shadowed(&self) -> Vec<usize> {
        (0..self.entries.len())
            .filter(|&j| self.entries[..j].iter().any(|e| e.covers(&self.entries[j])))
            .collect()
    }
}

// --- §2.5 to §2.11: one role's arbitration -----------------------------------------

/// What became of a sample (§2.5, §2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Outcome {
    /// An arrival of the active provider: delivered.
    Delivered,
    /// An arrival of another provider: counted, never delivered.
    NotActive,
    /// Its key is not concrete (core R6).
    Wild,
    /// Not the role's stream's key at any address.
    Unresolved,
    /// No entry matches its address.
    Unbound,
    /// `timing.v1` reads it expired: no arrival.
    Expired,
    /// A governed interface's sample `redundancy.v1` §2.10 does not
    /// accept: no arrival.
    NotIncumbent,
}

impl Outcome {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::NotActive => "not_active",
            Self::Wild => "wild",
            Self::Unresolved => "unresolved",
            Self::Unbound => "unbound",
            Self::Expired => "expired",
            Self::NotIncumbent => "not_incumbent",
        }
    }

    /// Whether the sample is delivered to the application.
    #[must_use]
    pub fn delivered(self) -> bool {
        self == Self::Delivered
    }
}

/// A sample of the role's subscription, as the runtime hands it over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample<'a> {
    /// Its key, base-relative (core §1.6).
    pub key: &'a str,
    /// `timing.v1` reads it expired, on the deployment's stated bound.
    pub expired: bool,
    /// The required interface is governed, and `redundancy.v1` §2.10 does
    /// not accept it.
    pub not_incumbent: bool,
}

impl<'a> Sample<'a> {
    /// A sample neither expired nor refused by `redundancy.v1`.
    #[must_use]
    pub fn new(key: &'a str) -> Self {
        Self {
            key,
            expired: false,
            not_incumbent: false,
        }
    }
}

/// Why a switch happened (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The consumer's start: the dead-man, from none.
    Start,
    /// An arrival made its provider active.
    Sample,
    /// The active provider missed its deadline.
    Deadline,
    /// The active provider was withdrawn.
    Withdrawn,
}

impl Cause {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Sample => "sample",
            Self::Deadline => "deadline",
            Self::Withdrawn => "withdrawn",
        }
    }
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A switch (§2.10): `None` is no provider, the dead-man.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub from: Option<Addr>,
    pub to: Option<Addr>,
    pub cause: Cause,
}

/// What the runtime tells an [`Arbiter`], each at its instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event<'a> {
    /// A sample of the role's subscription (§2.5).
    Sample(Sample<'a>),
    /// The watch heard an interface token declared: its key (§2.7).
    Declared(&'a str),
    /// The watch heard an interface token withdrawn: its key (§2.7).
    Withdrawn(&'a str),
    /// A read of an address's presence: whether it is complete (core §8.1),
    /// and the instances it shows holding the required interface's token
    /// there (§2.7).
    Read {
        address: Addr,
        complete: bool,
        held: BTreeSet<InstanceId>,
    },
    /// The watch session's transport closed or opened (§2.8).
    Moved,
    /// The round trip asked after the last move stands (§2.8).
    Stood,
    /// The timer fired (§2.9).
    Tick,
}

/// What an [`Arbiter`] decided at one event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Step {
    /// At most two: the active provider's miss (cause `deadline`), then
    /// the event's own (§2.9, §2.10).
    pub switches: Vec<Switch>,
    /// For a sample: what became of it.
    pub outcome: Option<Outcome>,
    /// The addresses to read again (§2.7), in rank order: the reference
    /// reads each 0.1 s later, one read for a burst.
    pub reread: Vec<Addr>,
}

/// A provider's counts of the samples counted for it (§2.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub delivered: u64,
    pub not_active: u64,
    pub expired: u64,
    pub not_incumbent: u64,
}

/// The counts of the samples with no provider (§2.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Discards {
    pub wild: u64,
    pub unresolved: u64,
    pub unbound: u64,
}

#[derive(Debug, Clone)]
struct Provider {
    addr: Addr,
    /// Its last arrival since its last re-admission (§2.6).
    last: Option<Duration>,
    withdrawn: bool,
    /// The instances known holding the required interface's token (§2.7).
    held: BTreeSet<InstanceId>,
    counts: Counts,
}

impl Provider {
    fn new(addr: Addr) -> Self {
        Self {
            addr,
            last: None,
            withdrawn: false,
            held: BTreeSet::new(),
            counts: Counts::default(),
        }
    }
}

/// A presence event kept from a move until its round trip stands (§2.8).
#[derive(Debug, Clone)]
enum Kept {
    Declared(Rank, InstanceId),
    Withdrawn(Rank, InstanceId),
    Read(Rank, bool, BTreeSet<InstanceId>),
}

/// One consumer's arbitration of one role (§2.5 to §2.11): a pure state
/// machine over events that carry their instants, with no clock and no
/// I/O. It asks reads ([`Step::reread`]) and is told what they found
/// ([`Event::Read`]); it is told of a move ([`Event::Moved`]) and of the
/// round trip after it ([`Event::Stood`]). A restart is a new `Arbiter`.
#[derive(Debug, Clone)]
pub struct Arbiter {
    iface: IfaceId,
    token: KindToken,
    template: Template,
    params: BTreeMap<String, String>,
    deadline: Duration,
    binding: Binding,
    /// Every provider known, in rank order.
    providers: BTreeMap<Rank, Provider>,
    active: Option<Rank>,
    started: bool,
    /// A move pending: from [`Event::Moved`] until [`Event::Stood`].
    moving: bool,
    kept: Vec<Kept>,
    discards: Discards,
}

impl Arbiter {
    /// The arbitration of a role declared `declared`, bound by `binding`,
    /// its parameter bindings `params` resolved (a `self.system` or
    /// `self.service` replaced by the consumer's own; core R2). It holds
    /// nothing until [`Arbiter::start`].
    #[must_use]
    pub fn new(declared: &Arbitrated, binding: Binding, params: BTreeMap<String, String>) -> Self {
        let providers = binding
            .entries()
            .iter()
            .filter_map(Entry::exact)
            .filter_map(|a| Some((binding.rank(&a)?, Provider::new(a))))
            .collect();
        Self {
            iface: declared.iface.clone(),
            token: declared.token,
            template: declared.template.clone(),
            params,
            deadline: declared.deadline,
            binding,
            providers,
            active: None,
            started: false,
            moving: false,
            kept: Vec::new(),
            discards: Discards::default(),
        }
    }

    /// The deadline it judges by (§2.3).
    #[must_use]
    pub fn deadline(&self) -> Duration {
        self.deadline
    }

    /// Its binding.
    #[must_use]
    pub fn binding(&self) -> &Binding {
        &self.binding
    }

    /// The active provider, `None` in the dead-man (§2.9, §2.11).
    #[must_use]
    pub fn active(&self) -> Option<&Addr> {
        self.active
            .as_ref()
            .and_then(|r| self.providers.get(r))
            .map(|p| &p.addr)
    }

    /// Whether it holds the dead-man (§2.11).
    #[must_use]
    pub fn dead_man(&self) -> bool {
        self.active.is_none()
    }

    /// The instant to arm the timer for: the active provider's last arrival
    /// plus the deadline. `None` in the dead-man, where only an arrival can
    /// make a provider eligible (§2.9).
    #[must_use]
    pub fn due(&self) -> Option<Duration> {
        let p = self.providers.get(self.active.as_ref()?)?;
        Some(p.last?.saturating_add(self.deadline))
    }

    /// Whether a move is pending (§2.8).
    #[must_use]
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// The providers known, in rank order (§2.4).
    pub fn providers(&self) -> impl Iterator<Item = &Addr> {
        self.providers.values().map(|p| &p.addr)
    }

    /// The counts of a provider's samples (§2.5).
    #[must_use]
    pub fn counts(&self, a: &Addr) -> Counts {
        self.binding
            .rank(a)
            .and_then(|r| self.providers.get(&r))
            .map(|p| p.counts)
            .unwrap_or_default()
    }

    /// The counts of the samples with no provider (§2.5).
    #[must_use]
    pub fn discards(&self) -> Discards {
        self.discards
    }

    /// Whether a provider is withdrawn (§2.7).
    #[must_use]
    pub fn withdrawn(&self, a: &Addr) -> bool {
        self.binding
            .rank(a)
            .and_then(|r| self.providers.get(&r))
            .is_some_and(|p| p.withdrawn)
    }

    /// The provider a sample's key names, or why it has none (§2.5's
    /// outcomes 1 to 3): never from the payload.
    pub fn identify(&self, key: &str) -> Result<Addr, Outcome> {
        if key.contains('*') {
            return Err(Outcome::Wild);
        }
        let Ok(ZkKey::Data {
            addr,
            iface,
            kind,
            resource,
        }) = parse(key)
        else {
            return Err(Outcome::Unresolved);
        };
        if iface != self.iface || kind != self.token {
            return Err(Outcome::Unresolved);
        }
        let chunks: Vec<&str> = resource.iter().map(String::as_str).collect();
        let Some(values) = self.template.matches(&chunks) else {
            return Err(Outcome::Unresolved);
        };
        for (name, vs) in &values {
            match (self.params.get(name), vs.as_slice()) {
                (Some(p), [v]) if p == v => {}
                _ => return Err(Outcome::Unresolved),
            }
        }
        if self.binding.rank(&addr).is_none() {
            return Err(Outcome::Unbound);
        }
        Ok(addr)
    }

    /// Starts: the dead-man, cause `start` (§2.11). Called once.
    pub fn start(&mut self, _at: Duration) -> Step {
        if self.started {
            return Step::default();
        }
        self.started = true;
        self.active = None;
        Step {
            switches: vec![Switch {
                from: None,
                to: None,
                cause: Cause::Start,
            }],
            ..Step::default()
        }
    }

    /// Hands it an event at instant `at`, never before the last one's: it
    /// decides the time first, then the event, then selects (§2.9).
    pub fn observe(&mut self, at: Duration, event: Event<'_>) -> Step {
        let mut step = Step::default();
        if !self.started {
            return step;
        }
        // The time first: the active provider's miss at or before `at`.
        if let Some(r) = self.active.clone()
            && !self.eligible(&r, at)
        {
            let next = self.select(at);
            step.switches.push(self.switch_to(next, Cause::Deadline));
        }
        let mut reread = Vec::new();
        let mut arrival = None;
        match event {
            Event::Sample(s) => match self.identify(s.key) {
                Err(o) => {
                    match o {
                        Outcome::Wild => self.discards.wild += 1,
                        Outcome::Unbound => self.discards.unbound += 1,
                        _ => self.discards.unresolved += 1,
                    }
                    step.outcome = Some(o);
                }
                Ok(addr) => {
                    let r = self.provider(addr);
                    let p = self.providers.get_mut(&r).expect("just inserted");
                    if s.expired {
                        p.counts.expired += 1;
                        step.outcome = Some(Outcome::Expired);
                    } else if s.not_incumbent {
                        p.counts.not_incumbent += 1;
                        step.outcome = Some(Outcome::NotIncumbent);
                    } else {
                        if !p.withdrawn {
                            p.last = Some(at);
                        }
                        arrival = Some(r);
                    }
                }
            },
            Event::Declared(key) => {
                if let Some((r, i)) = self.token(key) {
                    if self.moving {
                        self.kept.push(Kept::Declared(r, i));
                    } else {
                        let p = self.providers.get_mut(&r).expect("known");
                        p.held.insert(i);
                        p.withdrawn = false;
                    }
                }
            }
            Event::Withdrawn(key) => {
                if let Some((r, i)) = self.token(key) {
                    if self.moving {
                        self.kept.push(Kept::Withdrawn(r, i));
                    } else {
                        let p = self.providers.get_mut(&r).expect("known");
                        p.held.remove(&i);
                        if p.held.is_empty() {
                            p.withdrawn = true;
                            p.last = None;
                            reread.push(r);
                        }
                    }
                }
            }
            Event::Read {
                address,
                complete,
                held,
            } => {
                if self.binding.rank(&address).is_some() {
                    let r = self.provider(address);
                    if self.moving {
                        self.kept.push(Kept::Read(r, complete, held));
                    } else {
                        let p = self.providers.get_mut(&r).expect("known");
                        read_into(&mut p.held, complete, held);
                        if !p.held.is_empty() {
                            p.withdrawn = false;
                        }
                    }
                }
            }
            Event::Moved => self.moving = true,
            Event::Stood => {
                if self.moving {
                    self.stand(&mut reread);
                }
            }
            Event::Tick => {}
        }
        let next = self.select(at);
        if next != self.active {
            let cause = if arrival.is_some() && next == arrival {
                Cause::Sample
            } else {
                // Only a withdrawal makes the active provider ineligible
                // within an event (§2.10).
                Cause::Withdrawn
            };
            step.switches.push(self.switch_to(next, cause));
        }
        if let Some(r) = arrival {
            let delivered = self.active.as_ref() == Some(&r);
            let p = self.providers.get_mut(&r).expect("known");
            step.outcome = Some(if delivered {
                p.counts.delivered += 1;
                Outcome::Delivered
            } else {
                p.counts.not_active += 1;
                Outcome::NotActive
            });
        }
        reread.sort();
        reread.dedup();
        step.reread = reread
            .iter()
            .filter_map(|r| self.providers.get(r))
            .map(|p| p.addr.clone())
            .collect();
        step
    }

    /// Whether a provider is eligible at `at` (§2.6).
    fn eligible(&self, r: &Rank, at: Duration) -> bool {
        self.providers.get(r).is_some_and(|p| {
            !p.withdrawn
                && p.last
                    .is_some_and(|l| at < l.saturating_add(self.deadline))
        })
    }

    /// The eligible provider of highest rank at `at` (§2.9).
    fn select(&self, at: Duration) -> Option<Rank> {
        self.providers
            .keys()
            .find(|r| self.eligible(r, at))
            .cloned()
    }

    fn switch_to(&mut self, next: Option<Rank>, cause: Cause) -> Switch {
        let addr = |r: &Option<Rank>| {
            r.as_ref()
                .and_then(|r| self.providers.get(r))
                .map(|p| p.addr.clone())
        };
        let switch = Switch {
            from: addr(&self.active),
            to: addr(&next),
            cause,
        };
        self.active = next;
        switch
    }

    /// A bound address's provider, known from now on (§2.4).
    fn provider(&mut self, addr: Addr) -> Rank {
        let r = self.binding.rank(&addr).expect("a bound address");
        self.providers
            .entry(r.clone())
            .or_insert_with(|| Provider::new(addr));
        r
    }

    /// An interface token of the required interface at a bound address:
    /// its provider and its instance. Any other key is ignored (§2.7).
    fn token(&mut self, key: &str) -> Option<(Rank, InstanceId)> {
        let Ok(ZkKey::Alive {
            addr,
            iface,
            instance,
            ..
        }) = parse(key)
        else {
            return None;
        };
        if iface != self.iface || self.binding.rank(&addr).is_none() {
            return None;
        }
        Some((self.provider(addr), instance))
    }

    /// The round trip after the last move stands: what was kept, applied in
    /// order as one change (§2.8).
    fn stand(&mut self, reread: &mut Vec<Rank>) {
        self.moving = false;
        // Per provider: withdrawn before, and whether a withdrawal was kept.
        let mut touched: BTreeMap<Rank, (bool, bool)> = BTreeMap::new();
        for k in std::mem::take(&mut self.kept) {
            let r = match &k {
                Kept::Declared(r, _) | Kept::Withdrawn(r, _) | Kept::Read(r, _, _) => r.clone(),
            };
            let Some(p) = self.providers.get_mut(&r) else {
                continue;
            };
            let t = touched.entry(r).or_insert((p.withdrawn, false));
            match k {
                Kept::Declared(_, i) => {
                    p.held.insert(i);
                }
                Kept::Withdrawn(_, i) => {
                    p.held.remove(&i);
                    t.1 = true;
                }
                Kept::Read(_, complete, held) => read_into(&mut p.held, complete, held),
            }
        }
        for (r, (before, withdrawal)) in touched {
            let p = self.providers.get_mut(&r).expect("known");
            if p.held.is_empty() && withdrawal {
                p.withdrawn = true;
                p.last = None;
                reread.push(r);
            } else if !p.held.is_empty() && before {
                p.withdrawn = false;
            }
        }
    }
}

/// A read's instances into those known held (§2.7): a complete read
/// replaces them, one not complete adds to them.
fn read_into(known: &mut BTreeSet<InstanceId>, complete: bool, held: BTreeSet<InstanceId>) {
    if complete {
        *known = held;
    } else {
        known.extend(held);
    }
}

// --- §2.14, §5: what a tool may conclude ---------------------------------------------

/// A question of §5, answered in its states (core O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    No,
    /// The question was put, and the reading could not answer it.
    Unobservable,
    /// Nobody put the question.
    NotAsked,
    /// The role carries no policy.
    NotThisProfiles,
}

impl Answer {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
            Self::NotThisProfiles => "not_this_profiles",
        }
    }
}

impl fmt::Display for Answer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an answer is what it is: the fixture's reason (§2.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    NoContract,
    NotArbitrated,
    RevisionsDisagree,
    NoProviderContract,
    Invalid,
    Valid,
    Unbound,
    NotListened,
    Lost,
    Selected,
    Listening,
    DeadMan,
    Wildcard,
    Shadowed,
    Exact,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoContract => "no_contract",
            Self::NotArbitrated => "not_arbitrated",
            Self::RevisionsDisagree => "revisions_disagree",
            Self::NoProviderContract => "no_provider_contract",
            Self::Invalid => "invalid",
            Self::Valid => "valid",
            Self::Unbound => "unbound",
            Self::NotListened => "not_listened",
            Self::Lost => "lost",
            Self::Selected => "selected",
            Self::Listening => "listening",
            Self::DeadMan => "dead_man",
            Self::Wildcard => "wildcard",
            Self::Shadowed => "shadowed",
            Self::Exact => "exact",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An answer and its reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Judged {
    pub answer: Answer,
    pub reason: Reason,
}

impl Judged {
    const fn new(answer: Answer, reason: Reason) -> Self {
        Self { answer, reason }
    }
}

/// What a tool holds of a role's declaration (§2.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    /// The declaring contract not held.
    NoContract,
    /// The revisions of the required interface held give two declarations.
    RevisionsDisagree,
    /// The one declaration they give, unread when none is held. A
    /// manifest's role, whose policy no tool sees, is
    /// [`Declaration::NotArbitrated`].
    Read(Declaration),
}

/// §2.14's declaration over every revision of the required interface a
/// tool holds: the one they all give, [`Held::RevisionsDisagree`] when two
/// differ, unread when none is held.
#[must_use]
pub fn held_over(
    role: &Role<'_>,
    revisions: &[Revision<'_>],
    params: &BTreeMap<String, String>,
) -> Held {
    let mut seen: Option<Declaration> = None;
    for rev in revisions {
        let d = declaration(role, Some(rev), params);
        match &seen {
            None => seen = Some(d),
            Some(s) if *s == d => {}
            Some(_) => return Held::RevisionsDisagree,
        }
    }
    Held::Read(seen.unwrap_or_else(|| declaration(role, None, params)))
}

/// A tool's own view of a role (§2.14): never the consumer's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// How long it has listened.
    pub listened: Duration,
    /// Whether it knows it lost a sample within the last deadline.
    pub lost: bool,
    /// The provider its own [`Arbiter`] selects, `None` in the dead-man.
    pub active: Option<Addr>,
}

/// What a tool read of one consumer's role (§2.14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub declaration: Held,
    /// The descriptor's binding, in order.
    pub binding: Binding,
    /// Its own view, `None` when it did not listen.
    pub view: Option<View>,
}

/// The premises every row reads first (§5): the contract held, and the
/// role carrying a policy.
fn premises(r: &Reading) -> Result<(), Judged> {
    match &r.declaration {
        Held::NoContract => Err(Judged::new(Answer::Unobservable, Reason::NoContract)),
        Held::Read(Declaration::NotArbitrated) => Err(Judged::new(
            Answer::NotThisProfiles,
            Reason::NotArbitrated,
        )),
        _ => Ok(()),
    }
}

/// §5's first question, "is the role's declaration valid?" (§2.14), with
/// the reasons of an invalid one. No is the finding.
#[must_use]
pub fn judge_declaration(r: &Reading) -> (Judged, Vec<Invalid>) {
    match &r.declaration {
        Held::NoContract => (
            Judged::new(Answer::Unobservable, Reason::NoContract),
            Vec::new(),
        ),
        Held::Read(Declaration::NotArbitrated) => (
            Judged::new(Answer::NotThisProfiles, Reason::NotArbitrated),
            Vec::new(),
        ),
        Held::RevisionsDisagree => (
            Judged::new(Answer::Unobservable, Reason::RevisionsDisagree),
            Vec::new(),
        ),
        Held::Read(Declaration::Unread) => (
            Judged::new(Answer::Unobservable, Reason::NoProviderContract),
            Vec::new(),
        ),
        Held::Read(Declaration::Invalid(why)) => {
            (Judged::new(Answer::No, Reason::Invalid), why.clone())
        }
        Held::Read(Declaration::Arbitrated(_)) => {
            (Judged::new(Answer::Yes, Reason::Valid), Vec::new())
        }
    }
}

/// §5's second question, "which provider does the tool's view select?"
/// (§2.14), with the provider when it selects one. Its answer is the
/// tool's own view, never the consumer's.
#[must_use]
pub fn judge_selection(r: &Reading) -> (Judged, Option<Addr>) {
    let deadline = match &r.declaration {
        Held::Read(Declaration::Arbitrated(a)) => a.deadline,
        Held::Read(Declaration::Invalid(_)) => {
            return (Judged::new(Answer::Unobservable, Reason::Invalid), None);
        }
        // The premises, and a declaration not read: the first question's
        // answer, without its reasons.
        _ => return (judge_declaration(r).0, None),
    };
    if r.binding.is_empty() {
        return (Judged::new(Answer::No, Reason::Unbound), None);
    }
    let Some(view) = &r.view else {
        return (Judged::new(Answer::NotAsked, Reason::NotListened), None);
    };
    if view.lost {
        return (Judged::new(Answer::Unobservable, Reason::Lost), None);
    }
    if let Some(a) = &view.active {
        return (Judged::new(Answer::Yes, Reason::Selected), Some(a.clone()));
    }
    if view.listened < deadline {
        return (Judged::new(Answer::Unobservable, Reason::Listening), None);
    }
    (Judged::new(Answer::No, Reason::DeadMan), None)
}

/// §5's third question, "does the binding leave a priority to an order
/// nobody chose?" (§2.4, §2.14), with the wildcard and the shadowed
/// entries' positions. Yes is the finding, a warning.
#[must_use]
pub fn judge_binding(r: &Reading) -> (Judged, Vec<usize>, Vec<usize>) {
    if let Err(j) = premises(r) {
        return (j, Vec::new(), Vec::new());
    }
    let wild = r.binding.wildcards();
    let shadowed = r.binding.shadowed();
    if !wild.is_empty() {
        return (Judged::new(Answer::Yes, Reason::Wildcard), wild, shadowed);
    }
    if !shadowed.is_empty() {
        return (
            Judged::new(Answer::Yes, Reason::Shadowed),
            Vec::new(),
            shadowed,
        );
    }
    (Judged::new(Answer::No, Reason::Exact), Vec::new(), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arbitrated(deadline_ms: u64) -> Arbitrated {
        Arbitrated {
            policy: Policy::Priority,
            iface: "steer.v1".parse().unwrap(),
            token: KindToken::Stream,
            template: Template::parse("cmd").unwrap(),
            deadline: Duration::from_millis(deadline_ms),
            period: None,
        }
    }

    fn arbiter() -> Arbiter {
        Arbiter::new(
            &arbitrated(300),
            Binding::parse(&["lab/safety", "lab/teleop"]).unwrap(),
            BTreeMap::new(),
        )
    }

    const SAFETY: &str = "zk2/lab/safety/steer.v1/stream/cmd";

    /// Before its start an arbiter decides nothing, and it starts once.
    #[test]
    fn nothing_before_the_start_and_one_start() {
        let mut a = arbiter();
        let ms = Duration::from_millis;
        assert_eq!(
            a.observe(ms(0), Event::Sample(Sample::new(SAFETY))),
            Step::default()
        );
        assert_eq!(a.start(ms(1)).switches.len(), 1);
        assert_eq!(a.start(ms(2)), Step::default());
        assert!(a.dead_man());
        let s = a.observe(ms(3), Event::Sample(Sample::new(SAFETY)));
        assert_eq!(s.outcome, Some(Outcome::Delivered));
        assert_eq!(a.due(), Some(ms(303)));
        assert_eq!(a.counts(&"lab/safety".parse().unwrap()).delivered, 1);
    }

    /// An entry's text round-trips, `*` included; a malformed one is
    /// refused.
    #[test]
    fn entries_parse_and_print() {
        for e in ["lab/safety", "lab/*", "*/teleop", "*/*"] {
            assert_eq!(Entry::parse(e).unwrap().to_string(), e);
        }
        for bad in ["lab", "lab/Safety", "lab/**", "/x"] {
            assert!(Entry::parse(bad).is_err(), "{bad}");
        }
    }
}
