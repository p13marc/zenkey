//! `redundancy.v1`: one instance serves, the others stand by
//! (`spec/profiles/redundancy/v1.md`, #841).
//!
//! The profile's session-free half: the declaration read from a contract
//! (§2.2), how instance ids compare (§2.3), the descriptor's standby mark
//! read and checked (§2.4, §2.14, core §3.3), the election's decision over
//! one read (§2.5, §2.11), the heal's over two (§2.9), one candidate's
//! election as a pure state machine, two-phase like
//! [`crate::config::Owner`] (§2.5 to §2.11), and §5's four questions as
//! judgements that never fold an unestablished answer into no (§2.15).
//!
//! | Rule | Item |
//! |---|---|
//! | §2.2 the declaration: governed, the election, invalid | [`declaration`], [`declaration_of`], [`Declaration`], [`Invalid`] |
//! | §2.3 the claim set, the incumbent, ids compared | [`Read`], [`Seen`], [`Listing`], [`Read::holders`], [`lower`] |
//! | §2.4, §2.14 the standby mark and its checks | [`Mark`], [`mark`], [`read_mark`], [`MarkFinding`] |
//! | §2.5 the election under `claim` | [`elect`], [`Decision`], [`Why`] |
//! | §2.6, §2.8 taking over, a vacancy | [`Elector`], [`Step::TakeOver`], [`Event::Withdrawn`] |
//! | §2.7 following the address | [`Event::Appeared`], [`Event::Withdrawn`] (the runtime's watch) |
//! | §2.9 healing a split | [`heal`], [`Heal`], [`Step::StepDown`] |
//! | §2.11 `election = "external"` | [`Election::External`], [`Event::Lock`] |
//! | §2.15, §5 what a tool concludes | [`one_incumbent`], [`vacancy`], [`standbys`], [`twin`] |
//!
//! ```text
//! declaration(uses, annotations, tokenless) ─▶ governed {claim│external} │ invalid [reasons] │ not governed   §2.2
//! read_mark(entry json, governance)        ─▶ standby │ serving │ malformed, with findings                  §2.4, §2.14
//! elect(own, read)                         ─▶ take over │ stand by {incumbent, incomplete, own claim missing,
//!                                               descriptor unread, lower candidate}                      §2.5
//! heal(own, election, first, second)       ─▶ step down {lower holder} │ stay {external, not holding,
//!                                               no split, lowest}                                        §2.9
//! Elector::start / observe(event) / finish ─▶ claim │ read │ grace │ stand by │ take over │ step down │ nothing
//! one_incumbent / vacancy / standbys / twin ─▶ §5's questions, in five states                            §2.15
//! ```
//!
//! There is no I/O and no clock here. A candidate's runtime declares what
//! [`Step::Claim`] says, makes each read [`Step::Read`] asks for and hands
//! it back as [`Event::Read`], arms the retry timer when a
//! [`Step::StandBy`] says `retry` and the grace timer on [`Step::Grace`],
//! hands each to [`Elector::observe`] as [`Event::Timer`], tells it what
//! its watch hears ([`Event::Appeared`], [`Event::Withdrawn`]) and, under
//! `external`, its lock ([`Event::Lock`]); after a [`Step::Claim`], a
//! [`Step::TakeOver`] or a [`Step::StepDown`] it acts, in core §8.2's order
//! or its reverse, then calls [`Elector::finish`].

use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value;

use crate::contract::Contract;
use crate::descriptor::InterfaceEntry;
use crate::grammar::{IfaceId, InstanceId};

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "redundancy.v1";

/// The profile's name, as an annotation key's prefix carries it.
pub const NAME: &str = "redundancy";

/// `redundancy.election` (§4).
pub const ELECTION: &str = "redundancy.election";

/// The published vocabulary (§4): the keys under `redundancy.`, which W105
/// reads in place of core Appendix D's interim row (core 0.39), the same
/// key.
pub const VOCABULARY: &[&str] = &["election"];

// --- §2.2: the declaration -----------------------------------------------------

/// How a governed interface's serving instance is chosen (§4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Election {
    /// This profile's protocol (§2.5 to §2.9), the default.
    Claim,
    /// A lock outside the bus (§2.11).
    External,
}

impl Election {
    /// The vocabulary's value, and the fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::External => "external",
        }
    }

    /// Reads an annotation's value: the string `claim` or `external`, and
    /// nothing else.
    #[must_use]
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str() {
            Some("claim") => Some(Self::Claim),
            Some("external") => Some(Self::External),
            _ => None,
        }
    }
}

impl fmt::Display for Election {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a declaration is invalid (§2.2), in the order checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Invalid {
    /// `redundancy.election` carried, and `uses` names no major of the
    /// profile (core E020's).
    NotUsed,
    /// A value that is not the string `claim` or `external`.
    Value,
    /// Two resources resolve to two elections.
    Disagrees,
    /// The interface is governed, and the deployment's tokenless set names
    /// it (§2.14).
    Tokenless,
}

impl Invalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotUsed => "not_used",
            Self::Value => "value",
            Self::Disagrees => "disagrees",
            Self::Tokenless => "tokenless",
        }
    }

    /// What it says, for a linter's line.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotUsed => "redundancy.election is written, and uses does not list redundancy.v1",
            Self::Value => "redundancy.election is the string claim or external",
            Self::Disagrees => "every resource of a governed interface resolves to one election",
            Self::Tokenless => {
                "a governed interface is never tokenless: its incumbent is the holder of its token"
            }
        }
    }
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.says())
    }
}

/// An interface's declaration (§2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// `uses` lists no `redundancy.v1`: not this profile's.
    NotGoverned,
    /// The reasons, in the order checked.
    Invalid(Vec<Invalid>),
    /// Governed, with its election.
    Governed(Election),
}

impl Declaration {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotGoverned => "not_governed",
            Self::Invalid(_) => "invalid",
            Self::Governed(_) => "governed",
        }
    }

    /// The election of a valid declaration.
    #[must_use]
    pub fn election(&self) -> Option<Election> {
        match self {
            Self::Governed(e) => Some(*e),
            _ => None,
        }
    }
}

/// Reads an interface's declaration (§2.2) from its contract's `uses`, the
/// resolved annotations of each of its resources (core §9.3, in any
/// order), and whether the deployment's tokenless set names it. Each check
/// runs alone; a resource whose value is found wrong takes no part in
/// `disagrees`, and one carrying none reads as `claim`.
#[must_use]
pub fn declaration<'a, U, A>(uses: U, annotations: A, tokenless: bool) -> Declaration
where
    U: IntoIterator<Item = &'a IfaceId>,
    A: IntoIterator<Item = &'a std::collections::BTreeMap<String, Value>>,
{
    let mut governed = false;
    let mut another_major = false;
    for u in uses {
        if u.name() == NAME {
            if u.major() == 1 {
                governed = true;
            } else {
                another_major = true;
            }
        }
    }
    if another_major && !governed {
        // Another major's text: its key is its own.
        return Declaration::NotGoverned;
    }
    let values: Vec<Option<&Value>> = annotations.into_iter().map(|a| a.get(ELECTION)).collect();
    let carried = values.iter().any(Option::is_some);
    if !governed && !carried {
        return Declaration::NotGoverned;
    }
    let mut why = Vec::new();
    if !governed {
        why.push(Invalid::NotUsed);
    }
    let mut elections = BTreeSet::new();
    let mut wrong = false;
    for v in &values {
        match v {
            None => {
                elections.insert(Election::Claim);
            }
            Some(v) => match Election::parse(v) {
                Some(e) => {
                    elections.insert(e);
                }
                None => wrong = true,
            },
        }
    }
    if wrong {
        why.push(Invalid::Value);
    }
    if elections.len() > 1 {
        why.push(Invalid::Disagrees);
    }
    if governed && tokenless {
        why.push(Invalid::Tokenless);
    }
    if why.is_empty() {
        Declaration::Governed(elections.into_iter().next().unwrap_or(Election::Claim))
    } else {
        Declaration::Invalid(why)
    }
}

/// [`declaration`] of a contract, `tokenless` saying whether the
/// deployment's tokenless set names its interface.
#[must_use]
pub fn declaration_of(c: &Contract, tokenless: bool) -> Declaration {
    declaration(&c.uses, c.resources.iter().map(|r| &r.annotations), tokenless)
}

/// Whether a contract governs its interface: its `uses` lists
/// `redundancy.v1` (§2.2).
#[must_use]
pub fn is_governed(c: &Contract) -> bool {
    c.uses.iter().any(|u| u.name() == NAME && u.major() == 1)
}

// --- §2.3: ids, the claim set, the incumbent ----------------------------------------

/// Whether `a` is lower than `b` (§2.3): their 16 lowercase hexadecimal
/// digits compared character by character, which is their order as
/// unsigned 64-bit integers.
#[must_use]
pub fn lower(a: &InstanceId, b: &InstanceId) -> bool {
    a.as_str() < b.as_str()
}

/// What an instance's descriptor says of the governed interface, as a read
/// found it (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Listing {
    /// Its descriptor was not read.
    Unread,
    /// It does not list the interface: no candidate.
    NotListed,
    /// It lists the interface, marked standby or not (core §3.3).
    Listed { standby: bool },
}

impl Listing {
    /// Whether the instance is a candidate (§2.3).
    #[must_use]
    pub fn lists(self) -> bool {
        matches!(self, Self::Listed { .. })
    }
}

/// One instance of the address, as a read found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub id: InstanceId,
    /// It holds the governed interface's token, at any revision.
    pub token: bool,
    pub descriptor: Listing,
}

/// One read of the address (§0): its instances, with their tokens and
/// descriptors, and whether it is complete (core §8.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Read {
    pub complete: bool,
    pub instances: Vec<Seen>,
}

impl Read {
    /// The incumbents: the instances holding the interface's token,
    /// lowest first, each once (§2.3).
    #[must_use]
    pub fn holders(&self) -> Vec<&InstanceId> {
        let set: BTreeSet<&InstanceId> = self
            .instances
            .iter()
            .filter(|s| s.token)
            .map(|s| &s.id)
            .collect();
        set.into_iter().collect()
    }

    /// What the read found of `id`, if it found it.
    #[must_use]
    pub fn get(&self, id: &InstanceId) -> Option<&Seen> {
        self.instances.iter().find(|s| &s.id == id)
    }

    fn holders_owned(&self) -> Vec<InstanceId> {
        self.holders().into_iter().cloned().collect()
    }
}

// --- §2.4, §2.14: the standby mark ------------------------------------------------

/// What an interface entry says the instance does with the interface (core
/// §3.3, 0.39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mark {
    /// Unmarked: it serves the interface, or claims to.
    Serving,
    /// `"standby": true`: it exposes nothing of it and holds none of its
    /// tokens (core §6).
    Standby,
}

impl Mark {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Serving => "serving",
            Self::Standby => "standby",
        }
    }
}

/// The mark of a parsed entry.
#[must_use]
pub fn mark(e: &InterfaceEntry) -> Mark {
    if e.standby {
        Mark::Standby
    } else {
        Mark::Serving
    }
}

/// What a reader knows of an interface's governance (§2.15 premise 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Governance {
    /// Governed, by contracts read and agreeing, with their election.
    Governed(Election),
    /// No candidate's contract uses the profile: not this profile's.
    NotGoverned,
    /// A candidate's contract was not read.
    NoContract,
    /// The candidates' revisions disagree on governance or the election
    /// (§2.2).
    Disagree,
}

impl Governance {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Governed(e) => e.as_str(),
            Self::NotGoverned => "not_governed",
            Self::NoContract => "no_contract",
            Self::Disagree => "disagree",
        }
    }

    /// Reads a fixture's token.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claim" => Some(Self::Governed(Election::Claim)),
            "external" => Some(Self::Governed(Election::External)),
            "not_governed" => Some(Self::NotGoverned),
            "no_contract" => Some(Self::NoContract),
            "disagree" => Some(Self::Disagree),
            _ => None,
        }
    }
}

/// What a reader finds of an entry's mark (§2.14), in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MarkFinding {
    /// `standby` or `token` present and not a boolean, or the entry not an
    /// object: not the record's shape (core D000). Nothing else is read.
    Malformed,
    /// Marked `"standby": true` and `"token": false` (core D012).
    StandbyTokenless,
    /// Marked `"token": false`, the interface governed (§2.14).
    GovernedTokenless,
    /// Marked standby, the interface not governed: the mark says what core
    /// §6 says, and none of §5's questions is this profile's.
    NotGoverned,
}

impl MarkFinding {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::StandbyTokenless => "standby_tokenless",
            Self::GovernedTokenless => "governed_tokenless",
            Self::NotGoverned => "not_governed",
        }
    }
}

/// An entry's mark, `None` when it is malformed, and the findings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkRead {
    pub mark: Option<Mark>,
    pub findings: Vec<MarkFinding>,
}

/// Reads the mark of an interface entry as JSON (core §3.3, §2.14):
/// `standby` absent is serving, `token` absent is `true`.
#[must_use]
pub fn read_mark(entry: &Value, governance: Governance) -> MarkRead {
    let malformed = MarkRead {
        mark: None,
        findings: vec![MarkFinding::Malformed],
    };
    let Some(o) = entry.as_object() else {
        return malformed;
    };
    let flag = |name: &str, absent: bool| match o.get(name) {
        None => Some(absent),
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => None,
    };
    let (Some(standby), Some(token)) = (flag("standby", false), flag("token", true)) else {
        return malformed;
    };
    let mut findings = Vec::new();
    if standby && !token {
        findings.push(MarkFinding::StandbyTokenless);
    }
    if matches!(governance, Governance::Governed(_)) && !token {
        findings.push(MarkFinding::GovernedTokenless);
    }
    if standby && governance == Governance::NotGoverned {
        findings.push(MarkFinding::NotGoverned);
    }
    MarkRead {
        mark: Some(if standby { Mark::Standby } else { Mark::Serving }),
        findings,
    }
}

// --- §2.5, §2.11: the election over one read -----------------------------------------

/// Why an instance stands by (§2.5, §2.11).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Why {
    /// Another instance holds the interface's token: a live incumbent wins.
    Incumbent(InstanceId),
    /// The read was not complete.
    Incomplete,
    /// The read did not show the instance's own claim.
    OwnClaimMissing,
    /// A lower instance's descriptor was not read.
    DescriptorUnread(InstanceId),
    /// A lower candidate is in the claim set.
    LowerCandidate(InstanceId),
    /// `external`: the instance does not hold the lock.
    NoLock,
}

impl Why {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Incumbent(_) => "incumbent",
            Self::Incomplete => "incomplete",
            Self::OwnClaimMissing => "own_claim_missing",
            Self::DescriptorUnread(_) => "descriptor_unread",
            Self::LowerCandidate(_) => "lower_candidate",
            Self::NoLock => "no_lock",
        }
    }

    /// The instance it names, if it names one.
    #[must_use]
    pub fn instance(&self) -> Option<&InstanceId> {
        match self {
            Self::Incumbent(i) | Self::DescriptorUnread(i) | Self::LowerCandidate(i) => Some(i),
            Self::Incomplete | Self::OwnClaimMissing | Self::NoLock => None,
        }
    }

    /// Whether the instance campaigns again after its retry interval
    /// (§2.5): when it saw no incumbent, whose withdrawal its watch would
    /// hear, and holds its lock where one elects.
    #[must_use]
    pub fn retries(&self) -> bool {
        !matches!(self, Self::Incumbent(_) | Self::NoLock)
    }
}

/// What one read decides (§2.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    TakeOver,
    StandBy(Why),
}

impl Decision {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TakeOver => "take_over",
            Self::StandBy(_) => "stand_by",
        }
    }
}

/// §2.5 steps 2 and 3's first two conditions, shared by both elections:
/// an incumbent, then the read's completeness and the instance's own claim.
fn premises(own: &InstanceId, read: &Read) -> Option<Why> {
    if let Some(h) = read.holders().into_iter().find(|h| *h != own) {
        return Some(Why::Incumbent(h.clone()));
    }
    if !read.complete {
        return Some(Why::Incomplete);
    }
    if read.get(own).is_none() {
        return Some(Why::OwnClaimMissing);
    }
    None
}

/// What a candidate whose election is `claim` does with one read (§2.5,
/// steps 2 to 4): stand by for a live incumbent, whatever the read's
/// completeness; for a read not complete, or missing its own claim; for
/// the lowest instance below its own that lists the interface (a lower
/// candidate) or whose descriptor was not read; and otherwise take over.
#[must_use]
pub fn elect(own: &InstanceId, read: &Read) -> Decision {
    if let Some(why) = premises(own, read) {
        return Decision::StandBy(why);
    }
    let lowest = read
        .instances
        .iter()
        .filter(|s| lower(&s.id, own) && s.descriptor != Listing::NotListed)
        .min_by(|a, b| a.id.cmp(&b.id));
    match lowest {
        Some(s) if s.descriptor == Listing::Unread => {
            Decision::StandBy(Why::DescriptorUnread(s.id.clone()))
        }
        Some(s) => Decision::StandBy(Why::LowerCandidate(s.id.clone())),
        None => Decision::TakeOver,
    }
}

/// What a candidate whose election is `external`, holding its lock, does
/// with one read (§2.11): §2.5's step 2 and step 3's first two conditions,
/// never step 4, since the lock decided.
#[must_use]
pub fn elect_external(own: &InstanceId, read: &Read) -> Decision {
    match premises(own, read) {
        Some(why) => Decision::StandBy(why),
        None => Decision::TakeOver,
    }
}

// --- §2.9: the heal ------------------------------------------------------------------

/// Why an incumbent stays (§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stay {
    /// `external`: never by this rule (§2.11).
    External,
    /// It holds no token in the second read.
    NotHolding,
    /// Fewer than two holders in one of the reads: no split.
    NoSplit,
    /// It is the lowest holder of the second read.
    Lowest,
}

impl Stay {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::NotHolding => "not_holding",
            Self::NoSplit => "no_split",
            Self::Lowest => "lowest",
        }
    }
}

/// What two reads a grace apart decide for an incumbent (§2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heal {
    /// Step down: the second read shows this lower holder.
    StepDown(InstanceId),
    Stay(Stay),
}

/// Decides a heal for `own` from the holders of two reads a grace apart,
/// `first` then `second` (§2.9): under `claim`, an incumbent of a split,
/// two or more holders in both reads, steps down when the second read
/// shows a holder lower than itself. A read's completeness plays no part: a
/// read cannot invent a holder.
#[must_use]
pub fn heal(own: &InstanceId, election: Election, first: &[InstanceId], second: &[InstanceId]) -> Heal {
    if election == Election::External {
        return Heal::Stay(Stay::External);
    }
    let first: BTreeSet<&InstanceId> = first.iter().collect();
    let second: BTreeSet<&InstanceId> = second.iter().collect();
    if !second.contains(own) {
        return Heal::Stay(Stay::NotHolding);
    }
    if first.len() < 2 || second.len() < 2 {
        return Heal::Stay(Stay::NoSplit);
    }
    match second.into_iter().next() {
        Some(low) if lower(low, own) => Heal::StepDown(low.clone()),
        _ => Heal::Stay(Stay::Lowest),
    }
}

// --- §2.5 to §2.11: one candidate's election -----------------------------------------

/// What the runtime tells an [`Elector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The read a [`Step::Read`] asked for (§2.5, §2.8, §2.9).
    Read(Read),
    /// The watch heard an interface token of the interface appear at the
    /// address, held by this instance (§2.7, §2.9).
    Appeared(InstanceId),
    /// The watch heard the address's interface tokens withdrawn, leaving
    /// none held: one event for a burst (§2.8).
    Withdrawn,
    /// The timer the last step armed fired: the retry interval, or the
    /// grace.
    Timer,
    /// `external`: the deployment's lock taken (`true`) or lost (§2.11).
    Lock(bool),
}

/// Why an incumbent steps down (§2.9, §2.11).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StepDownWhy {
    /// Under `claim`, a split whose second read shows this lower holder.
    LowerHolder(InstanceId),
    /// Under `external`, the lock lost.
    LockLost,
}

impl StepDownWhy {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LowerHolder(_) => "lower_holder",
            Self::LockLost => "lock_lost",
        }
    }
}

/// What an [`Elector`] asks its runtime to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Put the descriptor, the interface marked standby, declare the
    /// instance token and the watch with its round trip (§2.5 step 1,
    /// §2.7), then call [`Elector::finish`].
    Claim,
    /// Make one read of the address (§0), then hand it back as
    /// [`Event::Read`].
    Read,
    /// Arm the grace timer, then hand it back as [`Event::Timer`] (§2.9).
    Grace,
    /// Stand by (§2.4), following the address; arm the retry timer when
    /// `retry` (§2.5).
    StandBy { why: Why, retry: bool },
    /// Take over in core §8.2's order (§2.6), then call
    /// [`Elector::finish`].
    TakeOver,
    /// Step down in the reverse order (§2.9), then call
    /// [`Elector::finish`].
    StepDown(StepDownWhy),
    /// Nothing to do.
    Nothing,
}

impl Step {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::Read => "read",
            Self::Grace => "grace",
            Self::StandBy { .. } => "stand_by",
            Self::TakeOver => "take_over",
            Self::StepDown(_) => "step_down",
            Self::Nothing => "nothing",
        }
    }
}

/// What the instance's descriptor says of the interface now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Not claimed yet.
    Unclaimed,
    /// Marked standby: campaigning, standing by, or taking over.
    Standby,
    /// Serving: an incumbent, healing, or stepping down.
    Serving,
}

impl Role {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unclaimed => "unclaimed",
            Self::Standby => "standby",
            Self::Serving => "serving",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    New,
    Claiming,
    Campaigning,
    Standby { retry: bool },
    TakingOver,
    Serving,
    HealFirst,
    HealGrace(Vec<InstanceId>),
    HealSecond(Vec<InstanceId>),
    SteppingDown(StepDownWhy),
}

/// One candidate's election on one governed interface (§2.5 to §2.11): a
/// pure state machine, two-phase like [`crate::config::Owner`]. A restart
/// is a new `Elector`, and a new instance id.
///
/// Events that arrive while an act runs are kept for its end: a token
/// appearing while it takes over starts a heal once it serves, a
/// withdrawal while it steps down a campaign once it stands by, and a lock
/// lost while it takes over a step-down. A timer or a read nothing waits
/// for is ignored.
#[derive(Debug, Clone)]
pub struct Elector {
    own: InstanceId,
    election: Election,
    phase: Phase,
    lock: bool,
    appeared: bool,
    withdrawn: bool,
}

impl Elector {
    /// An instance `own` of an interface whose election is `election`,
    /// before its claim. Under `external`, it holds no lock until told.
    #[must_use]
    pub fn new(own: InstanceId, election: Election) -> Self {
        Self {
            own,
            election,
            phase: Phase::New,
            lock: false,
            appeared: false,
            withdrawn: false,
        }
    }

    /// This instance's id.
    #[must_use]
    pub fn own(&self) -> &InstanceId {
        &self.own
    }

    /// The interface's election.
    #[must_use]
    pub fn election(&self) -> Election {
        self.election
    }

    /// What its descriptor says now.
    #[must_use]
    pub fn role(&self) -> Role {
        match self.phase {
            Phase::New | Phase::Claiming => Role::Unclaimed,
            Phase::Campaigning | Phase::Standby { .. } | Phase::TakingOver => Role::Standby,
            Phase::Serving
            | Phase::HealFirst
            | Phase::HealGrace(_)
            | Phase::HealSecond(_)
            | Phase::SteppingDown(_) => Role::Serving,
        }
    }

    /// Whether it serves the interface: an incumbent, which its role says.
    #[must_use]
    pub fn serving(&self) -> bool {
        self.role() == Role::Serving
    }

    /// Starts: claim before reading (§2.5 step 1). Called once.
    pub fn start(&mut self) -> Step {
        if self.phase != Phase::New {
            return Step::Nothing;
        }
        self.phase = Phase::Claiming;
        Step::Claim
    }

    /// The act a [`Step::Claim`], a [`Step::TakeOver`] or a
    /// [`Step::StepDown`] asked for is done: the next step.
    pub fn finish(&mut self) -> Step {
        let appeared = std::mem::take(&mut self.appeared);
        let withdrawn = std::mem::take(&mut self.withdrawn);
        match std::mem::replace(&mut self.phase, Phase::New) {
            Phase::Claiming => self.campaign(),
            Phase::TakingOver => {
                self.phase = Phase::Serving;
                match self.election {
                    Election::External if !self.lock => self.step_down(StepDownWhy::LockLost),
                    Election::Claim if appeared => {
                        self.phase = Phase::HealFirst;
                        Step::Read
                    }
                    _ => Step::Nothing,
                }
            }
            Phase::SteppingDown(_) => {
                self.phase = Phase::Standby { retry: false };
                if withdrawn {
                    self.campaign()
                } else {
                    Step::Nothing
                }
            }
            other => {
                self.phase = other;
                Step::Nothing
            }
        }
    }

    /// Hands it an event: the next step.
    pub fn observe(&mut self, event: Event) -> Step {
        match event {
            Event::Read(r) => self.on_read(&r),
            Event::Appeared(id) => self.on_appeared(&id),
            Event::Withdrawn => self.on_withdrawn(),
            Event::Timer => self.on_timer(),
            Event::Lock(held) => self.on_lock(held),
        }
    }

    /// A campaign: a read, but under `external` without the lock.
    fn campaign(&mut self) -> Step {
        if self.election == Election::External && !self.lock {
            return self.stand_by(Why::NoLock);
        }
        self.phase = Phase::Campaigning;
        Step::Read
    }

    fn stand_by(&mut self, why: Why) -> Step {
        let retry = why.retries();
        self.phase = Phase::Standby { retry };
        Step::StandBy { why, retry }
    }

    fn step_down(&mut self, why: StepDownWhy) -> Step {
        self.phase = Phase::SteppingDown(why.clone());
        Step::StepDown(why)
    }

    /// A heal read's holders, its own counted: it knows it holds.
    fn holding(&self, r: &Read) -> Vec<InstanceId> {
        let mut h = r.holders_owned();
        if !h.contains(&self.own) {
            h.push(self.own.clone());
            h.sort();
        }
        h
    }

    fn on_read(&mut self, r: &Read) -> Step {
        match self.phase.clone() {
            Phase::Campaigning => {
                let decision = match self.election {
                    Election::Claim => elect(&self.own, r),
                    Election::External if !self.lock => Decision::StandBy(Why::NoLock),
                    Election::External => elect_external(&self.own, r),
                };
                match decision {
                    Decision::TakeOver => {
                        self.phase = Phase::TakingOver;
                        Step::TakeOver
                    }
                    Decision::StandBy(why) => self.stand_by(why),
                }
            }
            Phase::HealFirst => {
                let holders = self.holding(r);
                if holders.len() >= 2 {
                    self.phase = Phase::HealGrace(holders);
                    Step::Grace
                } else {
                    self.phase = Phase::Serving;
                    Step::Nothing
                }
            }
            Phase::HealSecond(first) => {
                let second = self.holding(r);
                match heal(&self.own, self.election, &first, &second) {
                    Heal::StepDown(low) => self.step_down(StepDownWhy::LowerHolder(low)),
                    Heal::Stay(_) => {
                        self.phase = Phase::Serving;
                        Step::Nothing
                    }
                }
            }
            _ => Step::Nothing,
        }
    }

    fn on_appeared(&mut self, id: &InstanceId) -> Step {
        if id == &self.own || self.election != Election::Claim {
            return Step::Nothing;
        }
        match self.phase {
            Phase::Serving => {
                self.phase = Phase::HealFirst;
                Step::Read
            }
            Phase::TakingOver => {
                self.appeared = true;
                Step::Nothing
            }
            _ => Step::Nothing,
        }
    }

    fn on_withdrawn(&mut self) -> Step {
        match self.phase {
            Phase::Standby { .. } => {
                if self.election == Election::External && !self.lock {
                    Step::Nothing
                } else {
                    self.campaign()
                }
            }
            Phase::SteppingDown(_) => {
                self.withdrawn = true;
                Step::Nothing
            }
            _ => Step::Nothing,
        }
    }

    fn on_timer(&mut self) -> Step {
        match self.phase.clone() {
            Phase::Standby { retry: true } => self.campaign(),
            Phase::HealGrace(first) => {
                self.phase = Phase::HealSecond(first);
                Step::Read
            }
            _ => Step::Nothing,
        }
    }

    fn on_lock(&mut self, held: bool) -> Step {
        if self.election != Election::External {
            return Step::Nothing;
        }
        self.lock = held;
        match self.phase {
            Phase::Standby { .. } if held => self.campaign(),
            Phase::Serving if !held => self.step_down(StepDownWhy::LockLost),
            _ => Step::Nothing,
        }
    }
}

// --- §2.15, §5: what a tool may conclude ---------------------------------------------

/// What a tool read of an address and a governed interface (§2.15): two
/// reads a grace apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    /// Across a constrained face, where presence does not cross (§2.13).
    pub across_face: bool,
    pub governance: Governance,
    pub first: Read,
    pub second: Read,
}

/// A question of §5, answered in its states (core O5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    No,
    /// The question was put, and the reading could not answer it.
    Unobservable,
    /// Nobody put the question.
    NotAsked,
    /// The interface is not governed.
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

/// Why an answer is what it is: the fixture's reason (§2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    AcrossFace,
    NotGoverned,
    NoContract,
    RevisionsDisagree,
    Absent,
    PresenceIncomplete,
    Split,
    InTransition,
    One,
    NoDescriptor,
    NoCandidate,
    Vacant,
    Held,
    Lasting,
    StandingBy,
    NoneStanding,
    SplitLowest,
    NotACandidate,
    Unlisted,
    Incumbent,
    MarkedHolding,
    Standby,
    Unmarked,
}

impl Reason {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AcrossFace => "across_face",
            Self::NotGoverned => "not_governed",
            Self::NoContract => "no_contract",
            Self::RevisionsDisagree => "revisions_disagree",
            Self::Absent => "absent",
            Self::PresenceIncomplete => "presence_incomplete",
            Self::Split => "split",
            Self::InTransition => "in_transition",
            Self::One => "one",
            Self::NoDescriptor => "no_descriptor",
            Self::NoCandidate => "no_candidate",
            Self::Vacant => "vacant",
            Self::Held => "held",
            Self::Lasting => "lasting",
            Self::StandingBy => "standing_by",
            Self::NoneStanding => "none",
            Self::SplitLowest => "split_lowest",
            Self::NotACandidate => "not_a_candidate",
            Self::Unlisted => "unlisted",
            Self::Incumbent => "incumbent",
            Self::MarkedHolding => "marked_holding",
            Self::Standby => "standby",
            Self::Unmarked => "unmarked",
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

/// How many stand by (§2.15): exact, or a lower bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count {
    pub count: usize,
    pub exact: bool,
}

/// §2.15's premises, in order: a face, the presence of the address (or of
/// one instance) in the second read, the governance.
fn premises_of(r: &Reading, present: bool) -> Result<(), Judged> {
    use Answer::{NotAsked, NotThisProfiles, Unobservable};
    if r.across_face {
        return Err(Judged::new(Unobservable, Reason::AcrossFace));
    }
    if !present {
        return Err(if r.second.complete {
            Judged::new(NotAsked, Reason::Absent)
        } else {
            Judged::new(Unobservable, Reason::PresenceIncomplete)
        });
    }
    match r.governance {
        Governance::Governed(_) => Ok(()),
        Governance::NotGoverned => Err(Judged::new(NotThisProfiles, Reason::NotGoverned)),
        Governance::NoContract => Err(Judged::new(Unobservable, Reason::NoContract)),
        Governance::Disagree => Err(Judged::new(Unobservable, Reason::RevisionsDisagree)),
    }
}

fn address_premises(r: &Reading) -> Result<(), Judged> {
    premises_of(r, !r.second.instances.is_empty())
}

/// A read with no candidate: `no_descriptor` when a descriptor in it was
/// not read, else `no_candidate`. `None` when it holds a candidate.
fn no_candidate(read: &Read) -> Option<Judged> {
    if read.instances.iter().any(|s| s.descriptor.lists()) {
        return None;
    }
    Some(if read.instances.iter().any(|s| s.descriptor == Listing::Unread) {
        Judged::new(Answer::Unobservable, Reason::NoDescriptor)
    } else {
        Judged::new(Answer::NotAsked, Reason::NoCandidate)
    })
}

/// §5's first question, "is the address served by exactly one
/// incumbent?" (§2.15). No is the finding.
#[must_use]
pub fn one_incumbent(r: &Reading) -> Judged {
    use Answer::{No, Unobservable, Yes};
    if let Err(j) = address_premises(r) {
        return j;
    }
    let h1 = r.first.holders().len();
    let h2 = r.second.holders().len();
    if h1 >= 2 && h2 >= 2 {
        return Judged::new(No, Reason::Split);
    }
    if h2 >= 2 {
        return Judged::new(Unobservable, Reason::InTransition);
    }
    if h2 == 1 {
        return if r.second.complete {
            Judged::new(Yes, Reason::One)
        } else {
            Judged::new(Unobservable, Reason::PresenceIncomplete)
        };
    }
    if !r.second.complete {
        return Judged::new(Unobservable, Reason::PresenceIncomplete);
    }
    if let Some(j) = no_candidate(&r.second) {
        return j;
    }
    if h1 >= 1 {
        return Judged::new(Unobservable, Reason::InTransition);
    }
    if !r.first.complete {
        return Judged::new(Unobservable, Reason::PresenceIncomplete);
    }
    Judged::new(No, Reason::Vacant)
}

/// §5's second question, "is there a vacancy?" (§2.15). Yes, `lasting`, is
/// the finding; `vacant` is a takeover due.
#[must_use]
pub fn vacancy(r: &Reading) -> Judged {
    use Answer::{No, Unobservable, Yes};
    if let Err(j) = address_premises(r) {
        return j;
    }
    if !r.second.holders().is_empty() {
        return Judged::new(No, Reason::Held);
    }
    if !r.second.complete {
        return Judged::new(Unobservable, Reason::PresenceIncomplete);
    }
    if let Some(j) = no_candidate(&r.second) {
        return j;
    }
    if r.first.complete && r.first.holders().is_empty() {
        Judged::new(Yes, Reason::Lasting)
    } else {
        Judged::new(Yes, Reason::Vacant)
    }
}

/// §5's third question, "does any instance stand by, and how many?"
/// (§2.15): the instances of the second read marked standby and holding no
/// token. Informative.
#[must_use]
pub fn standbys(r: &Reading) -> (Judged, Count) {
    use Answer::{No, Unobservable, Yes};
    let unknown = Count {
        count: 0,
        exact: false,
    };
    if let Err(j) = address_premises(r) {
        return (j, unknown);
    }
    let read = &r.second;
    let n = read
        .instances
        .iter()
        .filter(|s| !s.token && s.descriptor == Listing::Listed { standby: true })
        .count();
    let unread = read.instances.iter().any(|s| s.descriptor == Listing::Unread);
    if n >= 1 {
        return (
            Judged::new(Yes, Reason::StandingBy),
            Count {
                count: n,
                exact: read.complete && !unread,
            },
        );
    }
    if !read.complete {
        return (Judged::new(Unobservable, Reason::PresenceIncomplete), unknown);
    }
    if unread {
        return (Judged::new(Unobservable, Reason::NoDescriptor), unknown);
    }
    (
        Judged::new(No, Reason::NoneStanding),
        Count {
            count: 0,
            exact: true,
        },
    )
}

/// §5's fourth question, "is this instance a standby, or a twin?"
/// (§2.15). Yes, a twin, is the finding.
#[must_use]
pub fn twin(r: &Reading, x: &InstanceId) -> Judged {
    use Answer::{No, NotAsked, Unobservable, Yes};
    let x2 = r.second.get(x);
    if let Err(j) = premises_of(r, x2.is_some()) {
        return j;
    }
    let Some(x2) = x2 else {
        return Judged::new(NotAsked, Reason::Absent);
    };
    let x1 = r.first.get(x);
    let h1 = r.first.holders();
    let h2 = r.second.holders();
    let same = x1.is_some_and(|a| a.token == x2.token && a.descriptor == x2.descriptor);
    if x2.token && x1.is_some_and(|a| a.token) && h1.len() >= 2 && h2.len() >= 2 {
        return if h2.first() == Some(&x) {
            Judged::new(Yes, Reason::SplitLowest)
        } else {
            Judged::new(Yes, Reason::Split)
        };
    }
    match x2.descriptor {
        Listing::Unread => Judged::new(Unobservable, Reason::NoDescriptor),
        Listing::NotListed if !x2.token => Judged::new(NotAsked, Reason::NotACandidate),
        Listing::NotListed if same => Judged::new(Yes, Reason::Unlisted),
        Listing::NotListed => Judged::new(Unobservable, Reason::InTransition),
        Listing::Listed { standby } if x2.token => {
            if h2.len() >= 2 {
                Judged::new(Unobservable, Reason::InTransition)
            } else if standby {
                if same {
                    Judged::new(Yes, Reason::MarkedHolding)
                } else {
                    Judged::new(Unobservable, Reason::InTransition)
                }
            } else if r.second.complete {
                Judged::new(No, Reason::Incumbent)
            } else {
                Judged::new(Unobservable, Reason::PresenceIncomplete)
            }
        }
        Listing::Listed { standby: true } => {
            if r.second.complete {
                Judged::new(No, Reason::Standby)
            } else {
                Judged::new(Unobservable, Reason::PresenceIncomplete)
            }
        }
        Listing::Listed { standby: false } => {
            if !same {
                Judged::new(Unobservable, Reason::InTransition)
            } else if r.first.complete && r.second.complete {
                Judged::new(Yes, Reason::Unmarked)
            } else {
                Judged::new(Unobservable, Reason::PresenceIncomplete)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> InstanceId {
        InstanceId::from_u64(n)
    }

    fn seen(n: u64, token: bool, descriptor: Listing) -> Seen {
        Seen {
            id: id(n),
            token,
            descriptor,
        }
    }

    const STANDBY: Listing = Listing::Listed { standby: true };
    const SERVING: Listing = Listing::Listed { standby: false };

    fn read(complete: bool, instances: Vec<Seen>) -> Read {
        Read {
            complete,
            instances,
        }
    }

    #[test]
    fn ids_compare_as_unsigned_integers() {
        assert!(lower(&id(0x9), &id(0xa)));
        assert!(lower(&id(0xff), &id(0x100)));
        assert!(!lower(&id(5), &id(5)));
    }

    #[test]
    fn a_live_incumbent_wins_whatever_the_read() {
        let r = read(false, vec![seen(9, true, Listing::Unread)]);
        assert_eq!(elect(&id(1), &r), Decision::StandBy(Why::Incumbent(id(9))));
    }

    #[test]
    fn the_lowest_candidate_wins_a_complete_read() {
        let own = id(5);
        let r = read(true, vec![seen(5, false, STANDBY), seen(7, false, STANDBY)]);
        assert_eq!(elect(&own, &r), Decision::TakeOver);
        let r = read(
            true,
            vec![
                seen(2, false, Listing::NotListed),
                seen(3, false, STANDBY),
                seen(5, false, STANDBY),
            ],
        );
        assert_eq!(elect(&own, &r), Decision::StandBy(Why::LowerCandidate(id(3))));
        let r = read(true, vec![seen(3, false, STANDBY)]);
        assert_eq!(elect(&own, &r), Decision::StandBy(Why::OwnClaimMissing));
    }

    #[test]
    fn a_split_leaves_the_lowest() {
        let two = [id(2), id(8)];
        assert_eq!(heal(&id(8), Election::Claim, &two, &two), Heal::StepDown(id(2)));
        assert_eq!(heal(&id(2), Election::Claim, &two, &two), Heal::Stay(Stay::Lowest));
        assert_eq!(heal(&id(8), Election::External, &two, &two), Heal::Stay(Stay::External));
        assert_eq!(heal(&id(8), Election::Claim, &[id(8)], &two), Heal::Stay(Stay::NoSplit));
    }

    #[test]
    fn a_standby_takes_over_at_a_vacancy_and_steps_down_to_a_lower_one() {
        let mut e = Elector::new(id(8), Election::Claim);
        assert_eq!(e.start(), Step::Claim);
        assert_eq!(e.finish(), Step::Read);
        let incumbent = read(true, vec![seen(4, true, SERVING), seen(8, false, STANDBY)]);
        assert_eq!(
            e.observe(Event::Read(incumbent)),
            Step::StandBy {
                why: Why::Incumbent(id(4)),
                retry: false
            }
        );
        assert_eq!(e.role(), Role::Standby);
        assert_eq!(e.observe(Event::Timer), Step::Nothing);
        assert_eq!(e.observe(Event::Withdrawn), Step::Read);
        let vacant = read(true, vec![seen(8, false, STANDBY)]);
        assert_eq!(e.observe(Event::Read(vacant)), Step::TakeOver);
        assert_eq!(e.finish(), Step::Nothing);
        assert!(e.serving());
        assert_eq!(e.observe(Event::Appeared(id(2))), Step::Read);
        let split = read(false, vec![seen(2, true, SERVING), seen(8, true, SERVING)]);
        assert_eq!(e.observe(Event::Read(split.clone())), Step::Grace);
        assert_eq!(e.observe(Event::Timer), Step::Read);
        assert_eq!(
            e.observe(Event::Read(split)),
            Step::StepDown(StepDownWhy::LowerHolder(id(2)))
        );
        assert_eq!(e.finish(), Step::Nothing);
        assert_eq!(e.role(), Role::Standby);
    }

    #[test]
    fn external_serves_only_with_its_lock() {
        let mut e = Elector::new(id(1), Election::External);
        assert_eq!(e.start(), Step::Claim);
        assert_eq!(
            e.finish(),
            Step::StandBy {
                why: Why::NoLock,
                retry: false
            }
        );
        assert_eq!(e.observe(Event::Withdrawn), Step::Nothing);
        assert_eq!(e.observe(Event::Lock(true)), Step::Read);
        let alone = read(true, vec![seen(1, false, STANDBY), seen(9, false, STANDBY)]);
        assert_eq!(e.observe(Event::Read(alone)), Step::TakeOver);
        assert_eq!(e.finish(), Step::Nothing);
        assert_eq!(e.observe(Event::Appeared(id(0))), Step::Nothing);
        assert_eq!(
            e.observe(Event::Lock(false)),
            Step::StepDown(StepDownWhy::LockLost)
        );
    }

    #[test]
    fn a_tool_tells_a_standby_from_a_twin() {
        let one = read(true, vec![seen(4, true, SERVING), seen(8, false, STANDBY)]);
        let r = Reading {
            across_face: false,
            governance: Governance::Governed(Election::Claim),
            first: one.clone(),
            second: one,
        };
        assert_eq!(one_incumbent(&r).reason, Reason::One);
        assert_eq!(vacancy(&r).reason, Reason::Held);
        assert_eq!(standbys(&r).1, Count { count: 1, exact: true });
        assert_eq!(twin(&r, &id(4)).reason, Reason::Incumbent);
        assert_eq!(twin(&r, &id(8)).reason, Reason::Standby);
        let split = read(false, vec![seen(4, true, SERVING), seen(8, true, SERVING)]);
        let r = Reading {
            first: split.clone(),
            second: split,
            ..r
        };
        assert_eq!(one_incumbent(&r).reason, Reason::Split);
        assert_eq!(twin(&r, &id(4)).reason, Reason::SplitLowest);
        assert_eq!(twin(&r, &id(8)).reason, Reason::Split);
    }
}
