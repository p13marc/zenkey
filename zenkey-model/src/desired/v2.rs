//! `desired.v2`: desired documents, on a wire a far target can afford
//! (`spec/profiles/desired/v2.md`, #986's J2).
//!
//! The profile's session-free half. It restates `desired.v1` by reference,
//! so what did not change is [`super`]'s, used as it stands: the binding
//! (§2.3), a document's comparison and its own revision (§2.7), the state
//! machine's rules (`super::machine`) and §5's judges. What it replaces
//! is here:
//!
//! ```text
//! version_text(v) / parse_version(text)     ─▶ <run>/<seq>: the in-band stamp, 4 or 8 hex digits and a decimal  §2.4
//! declaration(kind, template, …, uses)      ─▶ v1 §2.2's reading, and horizon_v1 checked last                   §2.2
//! report_declaration(resource, roles, others) ─▶ declared {role | param, tables} │ invalid [reasons] │ not this profile's  §2.10
//! Tables::place(pointer) / line(text)       ─▶ an index, the nearest listed ancestor's, or out_of_range        §2.10
//! Tables::pointer(index) / detail(index)    ─▶ the word, the number past the table, or out_of_range            §2.10
//! Report::write() / put(stamp)              ─▶ the packed record (tag 12), and its put's payload                §2.10, §3
//! read(tag, bytes)                          ─▶ Read {fields, findings, report} │ core §7.6's refusal            §2.10
//! Fields::check()                           ─▶ escape │ run_fields, observed_outcome, refusal, source, in_force, …  §2.10
//! Target::observe(sample)                   ─▶ act │ revert │ adopted │ held │ unstamped │ same_version │ older │ confirmed  §2.6–§2.9
//! Target::observe_no_entry(run, asof)       ─▶ revert │ held │ older │ no_verdict                              §2.8
//! Target::unchanged(sent)                   ─▶ a conditional read answered unchanged: its last-known confirmed  §2.5, §2.9
//! Target::finish(ended) / call() / held()   ─▶ the report updated; the stamp a conditional read sends         §2.5, §2.7
//! Watch::heartbeat(beat, holds)             ─▶ revision │ refused │ older │ no_section │ unmoved │ outstanding │ read  §2.5
//! Watch::answer(answer, holds) / heard(…)   ─▶ what the target expects of its author's section                 §2.5
//! report_at(target, iface, role, resources) ─▶ none │ two │ invalid [reasons] │ at {key, tables}; then found(reply)  §2.10
//! converged(target, author)                 ─▶ §5's first question, by run and seq, a snapshot with no entry read  §2.17
//! super::refused / last_known / in_force    ─▶ §5's other three, over a `Read` as over v1's `Report`
//! ```
//!
//! There is no I/O and no clock here. A target's runtime subscribes, reads
//! its slice (conditionally, on a period or when its [`Watch`] says so),
//! hands each version to [`Target::observe`], each snapshot with no entry
//! to [`Target::observe_no_entry`] and each unchanged answer to
//! [`Target::unchanged`], performs each [`Act`], tells [`Target::finish`]
//! how it ended, and puts [`Target::report`] whenever it changes.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde_json::{Map, Value, json};

use super::{
    ActKind, Answer, Content, Finding, Invalid, Judged, Mutation, Reason, Reported, Seen, Source,
    TargetReading, Via, machine,
};
use crate::authoring::{Kind, ParamType};
use crate::contract::{Body, Contract, Resource};
use crate::grammar::{self, Addr, IfaceId, Key, KindToken};
use crate::heartbeat::Digest;
use crate::packed::{self, Layout, ReadRefusal, Record, WriteRefusal};
use crate::schema::TypeId;
use crate::template::{Bindings, Template};
use crate::wire::{Envelope, Run, Seq, Stamp, Tag};
use crate::words;

/// The profile's id, as a contract lists it in `uses`.
pub const PROFILE: &str = "desired.v2";

/// `desired.report` (§4): the role a report resource reports.
pub const REPORT_KEY: &str = "desired.report";
/// `desired.pointers` (§4): the places a refusal's `pointer` indexes.
pub const POINTERS: &str = "desired.pointers";
/// `desired.details` (§4): the lines a refusal's `detail` indexes.
pub const DETAILS: &str = "desired.details";

/// The published vocabulary (§4): the keys under `desired.` W105 reads for
/// a contract that lists `desired.v2` (core §10, Appendix D).
pub const VOCABULARY: &[&str] = &["target_param", "report", "pointers", "details"];

/// The standard schema's stem (§3): `desired.v2.json`.
pub const SCHEMA_STEM: &str = "desired.v2";
/// The report's type name in the standard schema (§2.10, §3).
pub const REPORT: &str = "Report";

/// The standard schema's artifact ids the profile publishes (§3), oldest
/// first. A packed `Report` of one of these artifacts is the profile's.
pub const PUBLISHED: &[&str] =
    &["sha256:1ac893a1cb14a102cc1b01ea526a317b19390a8ed6c40707f08754f4429532dd"];

/// The `Report`'s tag (§3, core §7.6): CH is `0x0c` under a 16-bit run of
/// its target, `0x1c` under a 32-bit one.
pub const TAG: u8 = 12;

/// The greatest revision a document may carry (§2.4): a report holds it in
/// 32 bits.
pub const MAX_REVISION: u64 = 4_294_967_295;

/// How many runs a target remembers (§2.6): `desired.v1`'s sixteen ids.
pub const REMEMBERED_RUNS: usize = super::REMEMBERED_IDS;

/// The reserved word of `pointer` and `detail` (§2.10, §3).
pub use crate::words::OUT_OF_RANGE;

/// The greatest index a table holds (§2.10): 255 entries, from 0.
pub const MAX_INDEX: u8 = 254;

/// Whether `id` is one of the standard schema's published artifact ids.
#[must_use]
pub fn is_published(id: &str) -> bool {
    PUBLISHED.contains(&id)
}

/// Whether a contract lists `desired.v2` in `uses`: its desired resources
/// and its report resources are of this major (§0).
#[must_use]
pub fn uses_v2(c: &Contract) -> bool {
    words::uses(c, PROFILE)
}

/// Which major of the profile a revision uses (§2.18): a target and a tool
/// read it per author.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Major {
    V1,
    V2,
}

impl Major {
    /// The major `c` lists in `uses`, `None` when it lists neither.
    #[must_use]
    pub fn of(c: &Contract) -> Option<Major> {
        if uses_v2(c) {
            Some(Major::V2)
        } else if c.uses.iter().any(|u| u.to_string() == super::PROFILE) {
            Some(Major::V1)
        } else {
            None
        }
    }
}

// --- §2.4: the version -----------------------------------------------------------

/// One mutation of a document's key (§2.4): its in-band stamp, the run,
/// with its width, and the seq. Two are the same when both are equal.
pub type Version = Stamp;

/// A version as §2.4 writes it: the run as a descriptor writes one (4
/// lowercase hexadecimal digits for a 16-bit run, 8 for a 32-bit one), `/`,
/// and the seq in decimal without leading zeros.
#[must_use]
pub fn version_text(v: Version) -> String {
    format!("{}/{}", v.run, v.seq.get())
}

/// [`version_text`] read back. `None` for any other spelling: a run that
/// is not 4 or 8 lowercase hexadecimal digits, a seq with a leading zero,
/// a sign or a digit too many, or one above 16,777,215.
#[must_use]
pub fn parse_version(s: &str) -> Option<Version> {
    let (run, seq) = s.split_once('/')?;
    let digits = !seq.is_empty() && seq.len() <= 8 && seq.bytes().all(|b| b.is_ascii_digit());
    if !digits || (seq.len() > 1 && seq.starts_with('0')) {
        return None;
    }
    Some(Stamp {
        run: Run::from_hex(run)?,
        seq: Seq::new(seq.parse().ok()?)?,
    })
}

// --- §2.2: the desired resource's declaration ---------------------------------

/// Reads a desired resource's declaration under this major (§2.2): v1
/// §2.2's reading, word for word ([`super::declaration`]), and one reason
/// more, checked last and skipped where the resource is `not_state`:
/// [`Invalid::HorizonV1`], a `freshness.ttl_s` above 0 in a revision whose
/// `uses` does not list `freshness.v2`. `uses` says whether the revision
/// lists a profile.
#[must_use]
pub fn declaration(
    kind: Kind,
    template: &Template,
    params: &BTreeMap<String, ParamType>,
    annotations: &BTreeMap<String, Value>,
    uses: impl Fn(&str) -> bool,
) -> super::Declaration {
    let v1 = super::declaration(kind, template, params, annotations);
    // The horizon core §4.3's table reads: a state's, above 0.
    let horizon_v1 = kind == Kind::State
        && crate::freshness::horizon(kind, annotations).ttl().is_some()
        && !uses(crate::freshness::PROFILE_V2);
    match v1 {
        super::Declaration::NotThisProfiles => v1,
        super::Declaration::Declared(_) if !horizon_v1 => v1,
        super::Declaration::Declared(_) => super::Declaration::Invalid(vec![Invalid::HorizonV1]),
        super::Declaration::Invalid(mut why) => {
            if horizon_v1 {
                why.push(Invalid::HorizonV1);
            }
            super::Declaration::Invalid(why)
        }
    }
}

/// [`declaration`] of a resource of a contract.
#[must_use]
pub fn declaration_of(c: &Contract, r: &Resource) -> super::Declaration {
    declaration(r.kind, &r.template, &r.params, &r.annotations, |p| {
        c.uses.iter().any(|u| u.to_string() == p)
    })
}

// --- §2.10: the report resource's declaration ----------------------------------

/// What a `desired.report` value names (§2.10).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Names {
    /// A role's name: the resource's one member is that role's report.
    Role(String),
    /// The resource's one parameter, written in braces: each member is the
    /// report of the role its key's value of that parameter names.
    Param(String),
}

impl Names {
    /// Reads a `desired.report` value: a string in one of the two forms, a
    /// name of at least one character and no brace, or such a name in
    /// braces. `None` for a value in neither form, which names nothing.
    #[must_use]
    pub fn read(v: &Value) -> Option<Names> {
        let s = v.as_str()?;
        let name = |n: &str| !n.is_empty() && !n.contains(['{', '}']);
        if let Some(inner) = s.strip_prefix('{').and_then(|x| x.strip_suffix('}')) {
            return name(inner).then(|| Names::Param(inner.to_owned()));
        }
        name(s).then(|| Names::Role(s.to_owned()))
    }

    /// Whether the value can report `role`: by its name, or by a parameter
    /// in braces.
    #[must_use]
    pub fn names(&self, role: &str) -> bool {
        match self {
            Names::Role(r) => r == role,
            Names::Param(_) => true,
        }
    }
}

/// Why a report resource's declaration is invalid (§2.10), in the order
/// checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportInvalid {
    /// The resource is not `state` or `@state`.
    NotState,
    /// Not a string in one of the two forms. No later check reads the
    /// value.
    Value,
    /// A name that is no requirement of the contract.
    NotARole,
    /// Braces around a name that is no parameter of the template.
    NotAParam,
    /// Braces around a rest parameter.
    RestParam,
    /// A template parameter the value does not name.
    ParamNotRole,
    /// The resource's type is not the profile's `Report` (§3).
    NotReport,
    /// `desired.pointers` is no table of places.
    Pointers,
    /// `desired.details` is no table of lines.
    Details,
    /// Another report resource of the contract names the same role, or one
    /// of the two names a parameter.
    TwoReports,
}

impl ReportInvalid {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotState => "not_state",
            Self::Value => "value",
            Self::NotARole => "not_a_role",
            Self::NotAParam => "not_a_param",
            Self::RestParam => "rest_param",
            Self::ParamNotRole => "param_not_role",
            Self::NotReport => "not_report",
            Self::Pointers => "pointers",
            Self::Details => "details",
            Self::TwoReports => "two_reports",
        }
    }

    /// What it says, for a linter's line.
    #[must_use]
    pub fn says(self) -> &'static str {
        match self {
            Self::NotState => "a report is state of its target",
            Self::Value => {
                "desired.report is a role's name, or the resource's one parameter in braces"
            }
            Self::NotARole => "the name is no requirement of the contract",
            Self::NotAParam => "the name in braces is no parameter of the template",
            Self::RestParam => "a rest parameter cannot name a role",
            Self::ParamNotRole => {
                "a template parameter the value does not name: one report per role"
            }
            Self::NotReport => "a report resource's type is the profile's packed Report",
            Self::Pointers => {
                "desired.pointers is 1 to 255 JSON Pointers, the first \"\", none twice"
            }
            Self::Details => "desired.details is at most 255 lines of at least one character",
            Self::TwoReports => "another report resource of the contract can report the same role",
        }
    }
}

impl std::fmt::Display for ReportInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.says())
    }
}

/// An index into one of a report resource's two tables, as a report
/// carries it (§2.10, §3): a writer refuses a place above [`MAX_INDEX`],
/// 255 being `out_of_range`'s code. What it reads as, with the table a
/// reader holds, is a [`Word`]: the entry, the number past the table or
/// with no revision of the target's contract in hand, or `out_of_range`
/// (§2.10, "A reader reads an index"). The mechanism is [`crate::words`]'s,
/// which `health.v2` shares.
pub use crate::words::{Index, Word};

/// The word of `index` in `table`, `None` when the reader holds no
/// revision of the target's contract (§2.10, "A reader reads an index").
#[must_use]
pub fn word(table: Option<&[String]>, index: Index) -> Word<'_> {
    words::read(table, index)
}

/// A report resource's two tables (§2.10), as a reader holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tables {
    /// `desired.pointers`: `[""]` for a resource without the key.
    pub pointers: Vec<String>,
    /// `desired.details`: empty for a resource without the key.
    pub details: Vec<String>,
}

impl Default for Tables {
    fn default() -> Self {
        Self {
            pointers: vec![String::new()],
            details: Vec::new(),
        }
    }
}

fn index_of(i: usize) -> Index {
    Index::at(i, MAX_INDEX)
}

impl Tables {
    /// Where a refusal points, as a target writes it (§2.10): a place the
    /// table holds its index, and one it does not hold its nearest listed
    /// ancestor, the longest listed pointer other than `""` that the place
    /// continues with a `/`. With none, `out_of_range`: index 0 says the
    /// document itself is refused, never "somewhere".
    #[must_use]
    pub fn place(&self, pointer: &str) -> Index {
        if let Some(i) = self.pointers.iter().position(|p| p == pointer) {
            return index_of(i);
        }
        self.pointers
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                !p.is_empty()
                    && pointer
                        .strip_prefix(p.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|(_, p)| p.len())
            .map_or(Index::OutOfRange, |(i, _)| index_of(i))
    }

    /// A refusal's line, as a target writes it (§2.10): its index, or
    /// `out_of_range` for a line the table does not hold, whose words stay
    /// in the target's log.
    #[must_use]
    pub fn line(&self, text: &str) -> Index {
        words::write(&self.details, text, MAX_INDEX)
    }

    /// The place `index` names.
    #[must_use]
    pub fn pointer(&self, index: Index) -> Word<'_> {
        word(Some(&self.pointers), index)
    }

    /// The line `index` names.
    #[must_use]
    pub fn detail(&self, index: Index) -> Word<'_> {
        word(Some(&self.details), index)
    }
}

/// Whether a string is a JSON Pointer (RFC 6901): empty, or starting with
/// `/`, every `~` followed by `0` or `1`.
fn is_json_pointer(s: &str) -> bool {
    if !(s.is_empty() || s.starts_with('/')) {
        return false;
    }
    let b = s.as_bytes();
    (0..b.len()).all(|i| b[i] != b'~' || matches!(b.get(i + 1), Some(b'0' | b'1')))
}

/// `desired.pointers` read (§2.10): 1 to 255 JSON Pointers, the first
/// `""`, none twice.
fn read_pointers(v: &Value) -> Option<Vec<String>> {
    words::list(v, usize::from(MAX_INDEX) + 1, is_json_pointer, true)
        .filter(|out| out.first().is_some_and(String::is_empty))
}

/// `desired.details` read (§2.10): at most 255 strings of at least one
/// character each. A line may be there twice.
fn read_details(v: &Value) -> Option<Vec<String>> {
    words::list(v, usize::from(MAX_INDEX) + 1, |d| !d.is_empty(), false)
}

/// A report resource's declaration (§2.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportDeclaration {
    /// No `desired.report`: no report resource, whatever tables it holds.
    NotThisProfiles,
    /// The reasons, in the order checked.
    Invalid(Vec<ReportInvalid>),
    /// What it reports, and its two tables as a reader holds them.
    Declared { names: Names, tables: Tables },
}

impl ReportDeclaration {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotThisProfiles => "not_this_profiles",
            Self::Invalid(_) => "invalid",
            Self::Declared { .. } => "declared",
        }
    }
}

/// Whether a resource's type is the profile's `Report` (§3): the packed
/// type of that name in an artifact the profile publishes, and in no
/// other, whatever its types are named (core §7.1).
#[must_use]
pub fn is_report(ty: &TypeId) -> bool {
    matches!(ty, TypeId::Packed { name, schema } if name == REPORT && is_published(schema))
}

/// A resource as §2.10 reads one: what a report resource's declaration and
/// a tool's finding of a report need of it.
#[derive(Debug, Clone, Copy)]
pub struct ReportResource<'a> {
    pub kind: Kind,
    pub token: KindToken,
    pub template: &'a Template,
    /// Whether its type is the profile's `Report` ([`is_report`]).
    pub is_report: bool,
    /// Its resolved annotations (core §9.3).
    pub annotations: &'a BTreeMap<String, Value>,
}

impl<'a> ReportResource<'a> {
    /// A resource of a contract.
    #[must_use]
    pub fn of(r: &'a Resource) -> Self {
        Self {
            kind: r.kind,
            token: r.token,
            template: &r.template,
            is_report: matches!(&r.body, Body::Data(d) if is_report(&d.type_)),
            annotations: &r.annotations,
        }
    }
}

/// Reads a report resource's declaration (§2.10) from its resolved
/// annotations: `is_role` says whether a name is a requirement of the
/// same contract, and `others` are the `desired.report` values of the
/// contract's other resources. Each check runs alone, in the text's order.
/// After `value`, no later check reads the value, so `two_reports` is
/// skipped with the three that read a name; after `not_a_param` or
/// `rest_param`, `param_not_role` is skipped.
#[must_use]
pub fn report_declaration<'v>(
    r: &ReportResource<'_>,
    is_role: impl Fn(&str) -> bool,
    others: impl IntoIterator<Item = &'v Value>,
) -> ReportDeclaration {
    let Some(v) = r.annotations.get(REPORT_KEY) else {
        return ReportDeclaration::NotThisProfiles;
    };
    let mut why = Vec::new();
    if r.kind != Kind::State {
        why.push(ReportInvalid::NotState);
    }
    let names = Names::read(v);
    let params: Vec<(&str, bool)> = r.template.params().collect();
    match &names {
        None => why.push(ReportInvalid::Value),
        Some(Names::Role(name)) => {
            if !is_role(name) {
                why.push(ReportInvalid::NotARole);
            }
            if !params.is_empty() {
                why.push(ReportInvalid::ParamNotRole);
            }
        }
        Some(Names::Param(name)) => match params.iter().find(|(p, _)| p == name) {
            None => why.push(ReportInvalid::NotAParam),
            Some((_, true)) => why.push(ReportInvalid::RestParam),
            Some((_, false)) => {
                if params.iter().any(|(p, _)| p != name) {
                    why.push(ReportInvalid::ParamNotRole);
                }
            }
        },
    }
    if !r.is_report {
        why.push(ReportInvalid::NotReport);
    }
    let pointers = match r.annotations.get(POINTERS) {
        None => Some(Tables::default().pointers),
        Some(v) => read_pointers(v),
    };
    if pointers.is_none() {
        why.push(ReportInvalid::Pointers);
    }
    let details = match r.annotations.get(DETAILS) {
        None => Some(Vec::new()),
        Some(v) => read_details(v),
    };
    if details.is_none() {
        why.push(ReportInvalid::Details);
    }
    if let Some(mine) = &names {
        let clash = |other: &Names| match (mine, other) {
            (Names::Role(a), Names::Role(b)) => a == b,
            _ => true,
        };
        if others
            .into_iter()
            .filter_map(Names::read)
            .any(|o| clash(&o))
        {
            why.push(ReportInvalid::TwoReports);
        }
    }
    match (names, pointers, details) {
        (Some(names), Some(pointers), Some(details)) if why.is_empty() => {
            ReportDeclaration::Declared {
                names,
                tables: Tables { pointers, details },
            }
        }
        _ => ReportDeclaration::Invalid(why),
    }
}

/// [`report_declaration`] of a resource of a contract: its roles the
/// contract's requirements, and the others the contract's other resources.
#[must_use]
pub fn report_declaration_of(c: &Contract, r: &Resource) -> ReportDeclaration {
    report_declaration(
        &ReportResource::of(r),
        |name| c.requires.contains_key(name),
        c.resources
            .iter()
            .filter(|o| !(o.token == r.token && o.template.as_str() == r.template.as_str()))
            .filter_map(|o| o.annotations.get(REPORT_KEY)),
    )
}

/// One finding of [`lint_contract`]: a resource, by its kind token and
/// template, and the reason's token and words. [`crate::words`]'s shape,
/// its `key` none: a reason of this profile names its key in its words.
pub use crate::words::Lint;

/// What a linter reports of a contract that lists `desired.v2` (§2.2,
/// §2.10): each desired resource's and each report resource's invalid
/// declaration, a reason a finding. None for any other contract.
#[must_use]
pub fn lint_contract(c: &Contract) -> Vec<Lint> {
    if !uses_v2(c) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in &c.resources {
        let finding = |token, says| Lint {
            profile: PROFILE,
            at: Lint::place(r),
            key: None,
            token,
            says,
        };
        if let super::Declaration::Invalid(why) = declaration_of(c, r) {
            out.extend(why.iter().map(|w| finding(w.as_str(), w.says())));
        }
        if let ReportDeclaration::Invalid(why) = report_declaration_of(c, r) {
            out.extend(why.iter().map(|w| finding(w.as_str(), w.says())));
        }
    }
    out
}

// --- §2.10, §3: the report -------------------------------------------------------

/// The last act's outcome (§2.10), `not_acted` with no act yet in this
/// process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    NotActed,
    Converged,
    Refused,
    Failed,
    Reverted,
}

impl Outcome {
    const ALL: [Outcome; 5] = [
        Outcome::NotActed,
        Outcome::Converged,
        Outcome::Refused,
        Outcome::Failed,
        Outcome::Reverted,
    ];

    /// The record's and the fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotActed => "not_acted",
            Self::Converged => "converged",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Reverted => "reverted",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// `desired.v1`'s outcome, `None` for `not_acted`, which v1 writes as
    /// no `outcome`.
    #[must_use]
    pub fn acted(self) -> Option<super::Outcome> {
        match self {
            Self::NotActed => None,
            Self::Converged => Some(super::Outcome::Converged),
            Self::Refused => Some(super::Outcome::Refused),
            Self::Failed => Some(super::Outcome::Failed),
            Self::Reverted => Some(super::Outcome::Reverted),
        }
    }
}

impl From<Option<super::Outcome>> for Outcome {
    fn from(o: Option<super::Outcome>) -> Self {
        match o {
            None => Self::NotActed,
            Some(super::Outcome::Converged) => Self::Converged,
            Some(super::Outcome::Refused) => Self::Refused,
            Some(super::Outcome::Failed) => Self::Failed,
            Some(super::Outcome::Reverted) => Self::Reverted,
        }
    }
}

/// A refusal's code (§2.10): the profile's five, as in v1, or the type's
/// own, which the record writes as the escape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    Undecodable,
    Reach,
    Sensitive,
    Invalid,
    Apply,
    /// A code of the document type's own: the escape. A tool says "the
    /// type's own".
    Own,
}

impl Code {
    const NAMED: [Code; 5] = [
        Code::Undecodable,
        Code::Reach,
        Code::Sensitive,
        Code::Invalid,
        Code::Apply,
    ];

    /// The record's token, `None` for the type's own.
    #[must_use]
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Self::Undecodable => Some(super::UNDECODABLE),
            Self::Reach => Some(super::REACH),
            Self::Sensitive => Some(super::SENSITIVE),
            Self::Invalid => Some(super::INVALID),
            Self::Apply => Some(super::APPLY),
            Self::Own => None,
        }
    }

    /// A code as a target's handler names one: the profile's five by their
    /// names, and any other the type's own.
    #[must_use]
    pub fn of(code: &str) -> Code {
        Self::NAMED
            .into_iter()
            .find(|c| c.as_str() == Some(code))
            .unwrap_or(Code::Own)
    }

    fn to_json(self) -> Value {
        self.as_str().map_or(Value::Null, Value::from)
    }

    /// The record's reading, or the profile's: a name, or `null` for the
    /// escape.
    fn of_json(v: &Value) -> Result<Code, WriteRefusal> {
        match v {
            Value::Null => Ok(Code::Own),
            Value::String(s) => Self::NAMED
                .into_iter()
                .find(|c| c.as_str() == Some(s.as_str()))
                .ok_or(WriteRefusal::Value),
            _ => Err(WriteRefusal::Value),
        }
    }
}

/// Why a document was refused, or failed to apply (§2.10): a code, and a
/// place and a line as indices into the report resource's two tables
/// ([`Tables::place`], [`Tables::line`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Refusal {
    pub code: Code,
    pub pointer: Option<Index>,
    pub detail: Option<Index>,
}

impl Refusal {
    /// A refusal with a code alone.
    #[must_use]
    pub fn new(code: Code) -> Self {
        Self {
            code,
            pointer: None,
            detail: None,
        }
    }
}

/// A version with its document's own revision, where its type carries one
/// (§2.4, §2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Observed {
    pub version: Version,
    /// At most [`MAX_REVISION`] in a report a target puts; a writer
    /// refuses one above it, `range`.
    pub revision: Option<u64>,
}

/// Which version is wholly in force, as the record says it (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum InForce {
    /// No version: the baseline, a call, or a failed act.
    #[default]
    None,
    /// The observed version.
    Observed,
    /// Another version, which the four `in_force` fields name.
    Other,
}

impl InForce {
    const ALL: [InForce; 3] = [InForce::None, InForce::Observed, InForce::Other];

    /// The record's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Observed => "observed",
            Self::Other => "other",
        }
    }
}

/// The standard schema's `Report`, without its descriptions, which are no
/// part of a layout (core §7.6): `spec/profiles/desired/desired.v2.json`'s
/// fields, in order. `tests/profiles.rs`'s `desired_v2_schema` holds this
/// to the file's layout, its form's hash and its tag.
fn schema() -> Value {
    let int = |name: &str, maximum: u64| json!({"name": name, "type": "int", "minimum": 0, "maximum": maximum, "optional": true});
    let index = |name: &str| {
        json!({"name": name, "type": "int", "minimum": 0, "maximum": MAX_INDEX,
               "reserved": [OUT_OF_RANGE], "optional": true})
    };
    json!({"types": {(REPORT): {"fields": [
        {"name": "source", "type": "enum", "values": ["baseline", "desired", "call"], "escape": true},
        {"name": "outcome", "type": "enum",
         "values": ["not_acted", "converged", "refused", "failed", "reverted"], "escape": true},
        {"name": "last_known", "type": "bool"},
        {"name": "in_force", "type": "enum", "values": ["none", "observed", "other"]},
        int("observed_run16", 65_535),
        int("observed_run32", MAX_REVISION),
        int("observed_seq", 16_777_215),
        int("revision", MAX_REVISION),
        int("in_force_run16", 65_535),
        int("in_force_run32", MAX_REVISION),
        int("in_force_seq", 16_777_215),
        int("in_force_revision", MAX_REVISION),
        {"name": "code", "type": "enum",
         "values": ["undecodable", "reach", "sensitive", "invalid", "apply"],
         "escape": true, "optional": true},
        index("pointer"),
        index("detail"),
        packed::MARKER,
    ]}}})
}

/// The `Report`'s layout (§3, core §7.6): its tag is [`TAG`], a record is
/// 20 bits at the least, and a consistent one 215 at the most.
#[must_use]
pub fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        packed::layout_in(&schema(), REPORT).expect("the standard Report's declaration reads")
    })
}

/// A `Report` record's fields, as core §7.6 reads them (§3): an optional
/// field absent is `None`, and an enum's escape is `None` for `source` and
/// `outcome`, [`Code::Own`] for `code`. What a record says before §2.10's
/// consistency is asked of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fields {
    /// `None`: the escape, a value a later text names.
    pub source: Option<Source>,
    /// `None`: the escape.
    pub outcome: Option<Outcome>,
    pub last_known: bool,
    pub in_force: InForce,
    pub observed_run16: Option<u16>,
    pub observed_run32: Option<u32>,
    pub observed_seq: Option<u32>,
    pub revision: Option<u32>,
    pub in_force_run16: Option<u16>,
    pub in_force_run32: Option<u32>,
    pub in_force_seq: Option<u32>,
    pub in_force_revision: Option<u32>,
    pub code: Option<Code>,
    pub pointer: Option<Index>,
    pub detail: Option<Index>,
}

/// A version **stated** by its seq and exactly one of its two runs (§2.10,
/// `run_fields`).
fn stated(run16: Option<u16>, run32: Option<u32>, seq: Option<u32>) -> Option<Version> {
    let run = match (run16, run32) {
        (Some(r), None) => Run::U16(r),
        (None, Some(r)) => Run::U32(r),
        _ => return None,
    };
    Some(Stamp {
        run,
        seq: Seq::new(seq?)?,
    })
}

impl Fields {
    /// The observed version, where the fields state one.
    #[must_use]
    pub fn observed(&self) -> Option<Version> {
        stated(self.observed_run16, self.observed_run32, self.observed_seq)
    }

    /// The version in force the four `in_force` fields state, where they
    /// state one.
    #[must_use]
    pub fn other_in_force(&self) -> Option<Version> {
        stated(self.in_force_run16, self.in_force_run32, self.in_force_seq)
    }

    /// The record's JSON reading (core §7.6): an absent optional field no
    /// member, an enum's escape `null`, a reserved code its name.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                m.insert(k.to_owned(), v);
            }
        };
        put(
            "source",
            Some(self.source.map_or(Value::Null, |s| json!(s.as_str()))),
        );
        put(
            "outcome",
            Some(self.outcome.map_or(Value::Null, |o| json!(o.as_str()))),
        );
        put("last_known", Some(json!(self.last_known)));
        put("in_force", Some(json!(self.in_force.as_str())));
        put("observed_run16", self.observed_run16.map(Value::from));
        put("observed_run32", self.observed_run32.map(Value::from));
        put("observed_seq", self.observed_seq.map(Value::from));
        put("revision", self.revision.map(Value::from));
        put("in_force_run16", self.in_force_run16.map(Value::from));
        put("in_force_run32", self.in_force_run32.map(Value::from));
        put("in_force_seq", self.in_force_seq.map(Value::from));
        put("in_force_revision", self.in_force_revision.map(Value::from));
        put("code", self.code.map(Code::to_json));
        put("pointer", self.pointer.map(Index::to_json));
        put("detail", self.detail.map(Index::to_json));
        Value::Object(m)
    }

    /// The fields of a record's JSON reading, as [`Layout::read`] gives
    /// it.
    ///
    /// # Errors
    /// The writer's refusal a value outside the type would draw: `value`
    /// for a name the type does not declare or a wrong JSON type, `range`
    /// for a number with no code.
    pub fn of_json(v: &Value) -> Result<Fields, WriteRefusal> {
        let Value::Object(o) = v else {
            return Err(WriteRefusal::Value);
        };
        let number = |k: &str, max: u64| -> Result<Option<u64>, WriteRefusal> {
            match o.get(k) {
                None => Ok(None),
                Some(Value::Number(n)) => match n.as_u64() {
                    Some(x) if x <= max => Ok(Some(x)),
                    _ => Err(WriteRefusal::Range),
                },
                Some(_) => Err(WriteRefusal::Value),
            }
        };
        #[allow(clippy::cast_possible_truncation)]
        let u16_of = |k: &str| number(k, u64::from(u16::MAX)).map(|x| x.map(|x| x as u16));
        #[allow(clippy::cast_possible_truncation)]
        let u32_of = |k: &str| number(k, u64::from(u32::MAX)).map(|x| x.map(|x| x as u32));
        let named = |k: &str| match o.get(k) {
            Some(Value::String(s)) => Ok(Some(s.as_str())),
            Some(Value::Null) => Ok(None),
            Some(_) => Err(WriteRefusal::Value),
            None => Err(WriteRefusal::Missing),
        };
        let source = match named("source")? {
            None => None,
            Some(s) => Some(
                [Source::Baseline, Source::Desired, Source::Call]
                    .into_iter()
                    .find(|x| x.as_str() == s)
                    .ok_or(WriteRefusal::Value)?,
            ),
        };
        let outcome = match named("outcome")? {
            None => None,
            Some(s) => Some(Outcome::parse(s).ok_or(WriteRefusal::Value)?),
        };
        let in_force = match o.get("in_force") {
            Some(Value::String(s)) => InForce::ALL
                .into_iter()
                .find(|x| x.as_str() == s)
                .ok_or(WriteRefusal::Value)?,
            Some(_) => return Err(WriteRefusal::Value),
            None => return Err(WriteRefusal::Missing),
        };
        let last_known = match o.get("last_known") {
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(WriteRefusal::Value),
            None => return Err(WriteRefusal::Missing),
        };
        Ok(Fields {
            source,
            outcome,
            last_known,
            in_force,
            observed_run16: u16_of("observed_run16")?,
            observed_run32: u32_of("observed_run32")?,
            observed_seq: u32_of("observed_seq")?,
            revision: u32_of("revision")?,
            in_force_run16: u16_of("in_force_run16")?,
            in_force_run32: u32_of("in_force_run32")?,
            in_force_seq: u32_of("in_force_seq")?,
            in_force_revision: u32_of("in_force_revision")?,
            code: o.get("code").map(Code::of_json).transpose()?,
            pointer: o.get("pointer").map(Index::of_json).transpose()?,
            detail: o.get("detail").map(Index::of_json).transpose()?,
        })
    }

    /// The record (§3), written by core §7.6's rules.
    ///
    /// # Errors
    /// [`WriteRefusal::Range`] for a value with no code: a seq above
    /// 16,777,215, or an index above [`MAX_INDEX`].
    pub fn record(&self) -> Result<Record, WriteRefusal> {
        layout().write(&self.to_json(), None)
    }

    /// §2.10's consistency: the findings, every one that applies, in the
    /// order checked. `escape` is reported alone. A report with a finding
    /// is malformed, and answers nothing (§2.17).
    #[must_use]
    pub fn check(&self) -> Vec<Finding> {
        let (Some(source), Some(outcome)) = (self.source, self.outcome) else {
            return vec![Finding::Escape];
        };
        let mut out = Vec::new();
        let observed = self.observed();
        let other = self.other_in_force();
        let any_observed = self.observed_run16.is_some()
            || self.observed_run32.is_some()
            || self.observed_seq.is_some()
            || self.revision.is_some();
        let any_other = self.in_force_run16.is_some()
            || self.in_force_run32.is_some()
            || self.in_force_seq.is_some()
            || self.in_force_revision.is_some();
        if (any_observed && observed.is_none()) || (any_other && other.is_none()) {
            out.push(Finding::RunFields);
        }
        let acted = matches!(
            outcome,
            Outcome::Converged | Outcome::Refused | Outcome::Failed
        );
        if (observed.is_some() && outcome == Outcome::NotActed) || (acted && observed.is_none()) {
            out.push(Finding::ObservedOutcome);
        }
        let refusing = matches!(outcome, Outcome::Refused | Outcome::Failed);
        let refusal = self.code.is_some() || self.pointer.is_some() || self.detail.is_some();
        if (refusal && !refusing) || (refusing && self.code.is_none()) {
            out.push(Finding::Refusal);
        }
        if (source == Source::Desired && observed.is_none())
            || (outcome == Outcome::Converged && source == Source::Baseline)
            || (outcome == Outcome::Reverted && source == Source::Desired)
        {
            out.push(Finding::Source);
        }
        let names = self.in_force != InForce::None;
        let misplaced = names && (source != Source::Desired || outcome == Outcome::Failed);
        let not_observed = outcome == Outcome::Converged
            && source == Source::Desired
            && self.in_force != InForce::Observed;
        let no_observed = self.in_force == InForce::Observed && observed.is_none();
        let no_other = self.in_force == InForce::Other && (other.is_none() || other == observed);
        let stray = self.in_force != InForce::Other && any_other;
        if misplaced || not_observed || no_observed || no_other || stray {
            out.push(Finding::InForce);
        }
        if outcome == Outcome::Reverted && self.revision.is_some() {
            out.push(Finding::RevisionOnRevert);
        }
        if self.last_known && observed.is_none() {
            out.push(Finding::LastKnown);
        }
        out
    }

    /// The profile's reading of consistent fields, `None` with a finding.
    #[must_use]
    pub fn report(&self) -> Option<Report> {
        if !self.check().is_empty() {
            return None;
        }
        let observed = self.observed().map(|version| Observed {
            version,
            revision: self.revision.map(u64::from),
        });
        let in_force = match self.in_force {
            InForce::None => None,
            InForce::Observed => observed,
            InForce::Other => self.other_in_force().map(|version| Observed {
                version,
                revision: self.in_force_revision.map(u64::from),
            }),
        };
        Some(Report {
            source: self.source?,
            outcome: self.outcome?,
            last_known: self.last_known,
            observed,
            in_force,
            refusal: self.code.map(|code| Refusal {
                code,
                pointer: self.pointer,
                detail: self.detail,
            }),
        })
    }
}

/// A target's report of one role (§2.10), in the profile's reading: what
/// the packed `Report` says once its fields hold together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// What is in force.
    pub source: Source,
    /// The last act's, which an adoption keeps.
    pub outcome: Outcome,
    /// `true` when the observed version was read from an archive (§2.9).
    pub last_known: bool,
    /// The last version acted on or adopted in this process. None after a
    /// revert on a snapshot with no entry (§2.8).
    pub observed: Option<Observed>,
    /// The version wholly in force: absent is the record's `none`, one
    /// whose version is the observed one `observed`, and any other
    /// `other`.
    pub in_force: Option<Observed>,
    pub refusal: Option<Refusal>,
}

fn revision_field(r: Option<u64>) -> Result<Option<u32>, WriteRefusal> {
    r.map(|r| u32::try_from(r).map_err(|_| WriteRefusal::Range))
        .transpose()
}

fn observed_json(o: &Observed) -> Value {
    let mut m = Map::new();
    m.insert("version".into(), json!(version_text(o.version)));
    if let Some(r) = o.revision {
        m.insert("revision".into(), json!(r));
    }
    Value::Object(m)
}

fn observed_of(v: &Value) -> Result<Observed, WriteRefusal> {
    let version = v
        .get("version")
        .and_then(Value::as_str)
        .and_then(parse_version)
        .ok_or(WriteRefusal::Value)?;
    let revision = match v.get("revision") {
        None => None,
        Some(r) => Some(r.as_u64().ok_or(WriteRefusal::Value)?),
    };
    Ok(Observed { version, revision })
}

impl Report {
    /// A target's first report of a role, and the kill switch's (§2.10,
    /// §2.12): `source` `baseline`, `not_acted`. Its record is the 3 bytes
    /// `000000`.
    #[must_use]
    pub fn baseline() -> Self {
        Self {
            source: Source::Baseline,
            outcome: Outcome::NotActed,
            last_known: false,
            observed: None,
            in_force: None,
            refusal: None,
        }
    }

    /// The record's fields (§3).
    ///
    /// # Errors
    /// [`WriteRefusal::Range`] for a revision above [`MAX_REVISION`],
    /// which has no code (§2.4).
    pub fn fields(&self) -> Result<Fields, WriteRefusal> {
        let mut f = Fields {
            source: Some(self.source),
            outcome: Some(self.outcome),
            last_known: self.last_known,
            ..Fields::default()
        };
        if let Some(o) = &self.observed {
            match o.version.run {
                Run::U16(r) => f.observed_run16 = Some(r),
                Run::U32(r) => f.observed_run32 = Some(r),
            }
            f.observed_seq = Some(o.version.seq.get());
            f.revision = revision_field(o.revision)?;
        }
        f.in_force = match (&self.in_force, &self.observed) {
            (None, _) => InForce::None,
            (Some(i), Some(o)) if i.version == o.version => {
                // The observed version's own fields say it; its revision
                // has to have a code all the same.
                revision_field(i.revision)?;
                InForce::Observed
            }
            (Some(i), _) => {
                match i.version.run {
                    Run::U16(r) => f.in_force_run16 = Some(r),
                    Run::U32(r) => f.in_force_run32 = Some(r),
                }
                f.in_force_seq = Some(i.version.seq.get());
                f.in_force_revision = revision_field(i.revision)?;
                InForce::Other
            }
        };
        if let Some(r) = &self.refusal {
            f.code = Some(r.code);
            f.pointer = r.pointer;
            f.detail = r.detail;
        }
        Ok(f)
    }

    /// The packed record a target puts (§2.10, §3).
    ///
    /// # Errors
    /// The writer's refusal: `range` for a revision above
    /// [`MAX_REVISION`] or an index above [`MAX_INDEX`].
    pub fn write(&self) -> Result<Record, WriteRefusal> {
        self.fields()?.record()
    }

    /// The payload of the report's put under the target's own `stamp`
    /// (core §7.5): a mutation's envelope, CH's tag [`TAG`], then the
    /// record.
    ///
    /// # Errors
    /// [`Report::write`]'s.
    pub fn put(&self, stamp: Stamp) -> Result<Vec<u8>, WriteRefusal> {
        let record = self.write()?;
        let tag = Tag::new(TAG).expect("12 is a tag");
        Ok(Envelope::mutation(tag, stamp, &record.bytes).write())
    }

    /// The findings of the record this report writes, none for one a
    /// [`Target`] made.
    #[must_use]
    pub fn check(&self) -> Vec<Finding> {
        self.fields().map(|f| f.check()).unwrap_or_default()
    }

    /// The profile's reading as JSON, as `reports.json` writes one:
    /// `{source, outcome, last_known, observed?, in_force?, refusal?}`,
    /// a version `<run>/<seq>`, a code `null` for the type's own, an
    /// index a number or `"out_of_range"`.
    #[must_use]
    pub fn reading(&self) -> Value {
        let mut m = Map::new();
        m.insert("source".into(), json!(self.source.as_str()));
        m.insert("outcome".into(), json!(self.outcome.as_str()));
        m.insert("last_known".into(), json!(self.last_known));
        if let Some(o) = &self.observed {
            m.insert("observed".into(), observed_json(o));
        }
        if let Some(i) = &self.in_force {
            m.insert("in_force".into(), observed_json(i));
        }
        if let Some(r) = &self.refusal {
            let mut x = Map::new();
            x.insert("code".into(), r.code.to_json());
            if let Some(p) = r.pointer {
                x.insert("pointer".into(), p.to_json());
            }
            if let Some(d) = r.detail {
                x.insert("detail".into(), d.to_json());
            }
            m.insert("refusal".into(), Value::Object(x));
        }
        Value::Object(m)
    }

    /// [`Report::reading`] read back.
    ///
    /// # Errors
    /// The writer's refusal of what the reading holds: `value` for a name
    /// the type does not declare, a wrong JSON type or a version that is
    /// not `<run>/<seq>`, `range` for an index above 255.
    pub fn from_reading(v: &Value) -> Result<Report, WriteRefusal> {
        let name = |k: &str| v.get(k).and_then(Value::as_str).ok_or(WriteRefusal::Value);
        let source = name("source")?;
        let source = [Source::Baseline, Source::Desired, Source::Call]
            .into_iter()
            .find(|s| s.as_str() == source)
            .ok_or(WriteRefusal::Value)?;
        let outcome = Outcome::parse(name("outcome")?).ok_or(WriteRefusal::Value)?;
        let last_known = v
            .get("last_known")
            .and_then(Value::as_bool)
            .ok_or(WriteRefusal::Value)?;
        let refusal = match v.get("refusal") {
            None => None,
            Some(r) => Some(Refusal {
                code: Code::of_json(r.get("code").ok_or(WriteRefusal::Value)?)?,
                pointer: r.get("pointer").map(Index::of_json).transpose()?,
                detail: r.get("detail").map(Index::of_json).transpose()?,
            }),
        };
        Ok(Report {
            source,
            outcome,
            last_known,
            observed: v.get("observed").map(observed_of).transpose()?,
            in_force: v.get("in_force").map(observed_of).transpose()?,
            refusal,
        })
    }
}

/// A report as a reader read it (§2.10): the record's fields, their
/// findings, and the profile's reading where there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub fields: Fields,
    /// §2.10's findings, in the order checked: a report with one is
    /// malformed.
    pub findings: Vec<Finding>,
    /// `Some` exactly with no finding.
    pub report: Option<Report>,
}

/// Reads a report's record under the tag its envelope's CH carries
/// (§2.10): core §7.6's reading first, a later text's fields behind the
/// marker ignored, then §2.10's checks.
///
/// # Errors
/// Core §7.6's refusal, for which the report is unreadable: `tag`,
/// `length`, `code`, `extension` or `pad`.
pub fn read(tag: u8, bytes: &[u8]) -> Result<Read, ReadRefusal> {
    let value = layout().read(tag, bytes, None)?;
    // Every value the layout reads is one the type declares.
    let fields = Fields::of_json(&value).map_err(|_| ReadRefusal::Code)?;
    Ok(Read {
        fields,
        findings: fields.check(),
        report: fields.report(),
    })
}

impl Reported for Read {
    fn facts(&self) -> Option<super::Facts> {
        let r = self.report.as_ref()?;
        Some(super::Facts {
            source: r.source,
            outcome: r.outcome.acted(),
            last_known: r.last_known,
            in_force: r.in_force.is_some(),
        })
    }
}

// --- §2.6–§2.10: a target ------------------------------------------------------

/// What an act is of (§2.6, §2.8): a version, a put's or a delete's, or a
/// snapshot with no entry, which names none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    Version(Version),
    /// The author's word that it holds no document for the target: its
    /// snapshot's run, and its asof.
    NoEntry {
        run: Run,
        asof: Seq,
    },
}

impl machine::Mark for Cause {
    type Epoch = Run;
    type Tick = Seq;
    fn epoch(&self) -> Run {
        match self {
            Cause::Version(v) => v.run,
            Cause::NoEntry { run, .. } => *run,
        }
    }
    fn tick(&self) -> Seq {
        match self {
            Cause::Version(v) => v.seq,
            Cause::NoEntry { asof, .. } => *asof,
        }
    }
    fn floor(&self) -> bool {
        matches!(self, Cause::NoEntry { .. })
    }
}

impl machine::Mark for Stamp {
    type Epoch = Run;
    type Tick = Seq;
    fn epoch(&self) -> Run {
        self.run
    }
    fn tick(&self) -> Seq {
        self.seq
    }
}

/// One sample or entry of a target's slice, as its runtime received it
/// (§2.5): a put or a live entry, a delete or a tombstone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub via: Via,
    pub mutation: Mutation,
    /// The in-band stamp, `None` for a sample with no envelope (§2.6
    /// step 1).
    pub version: Option<Version>,
    /// The document type's own revision, when it carries one (§2.4). A
    /// delete has none. One above [`MAX_REVISION`] is carried here, and
    /// refused when the act ends ([`Act::bound`]).
    pub revision: Option<u64>,
    /// A put's document ([`Content`]); `None` for a delete, and for a put
    /// whose runtime did not compare it.
    pub content: Option<Content>,
}

/// An act the target's runtime performs, then reports by
/// [`Target::finish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Act {
    pub kind: ActKind,
    pub cause: Cause,
    pub revision: Option<u64>,
    pub via: Via,
    pub content: Option<Content>,
}

impl Act {
    /// The act's version, none for a revert on a snapshot with no entry.
    #[must_use]
    pub fn version(&self) -> Option<Version> {
        match self.cause {
            Cause::Version(v) => Some(v),
            Cause::NoEntry { .. } => None,
        }
    }

    /// §2.4's bound: the refusal of a put whose document's revision is
    /// above [`MAX_REVISION`], whole, with the code `invalid`. `None` for
    /// a document within the bound, or with no revision.
    #[must_use]
    pub fn bound(&self) -> Option<Refusal> {
        (self.kind == ActKind::Apply && self.revision.is_some_and(|r| r > MAX_REVISION))
            .then(|| Refusal::new(Code::Invalid))
    }
}

impl From<machine::Doing<Cause>> for Act {
    fn from(d: machine::Doing<Cause>) -> Self {
        Self {
            kind: d.kind,
            cause: d.mark,
            revision: d.revision,
            via: d.via,
            content: d.content,
        }
    }
}

/// How an act ended (§2.7, §2.10), its refusal this major's.
pub type Ended = super::Ended<Refusal>;

/// What a step of a target does (§2.6–§2.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Start this act now.
    Act(Act),
    /// Admitted, and the same document as the last version acted on or
    /// adopted, or no document after none: no act, and the report moved.
    Adopted,
    /// Admitted while an act runs: held, replacing what waited.
    Held,
    /// No envelope: never acted on (§2.6 step 1).
    Unstamped,
    /// A version the target acted on, adopted or holds waiting, received
    /// again (§2.6 steps 3 and 4).
    SameVersion,
    /// A lower seq of a remembered run, one at or below a remembered asof,
    /// or a snapshot with no entry built before the held version (§2.6,
    /// §2.8).
    Older,
    /// The author's own delivery of a version held from an archive
    /// (§2.9). When no act runs, the report changed: its `last_known`
    /// went.
    Confirmed,
    /// An act ended, and nothing waits.
    Done,
    /// A snapshot with no entry that reverts nothing (§2.8): the target
    /// holds no put's version of its run.
    NoVerdict,
}

impl Step {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Act(Act {
                kind: ActKind::Apply,
                ..
            }) => "act",
            Self::Act(Act {
                kind: ActKind::Revert,
                ..
            }) => "revert",
            Self::Adopted => "adopted",
            Self::Held => "held",
            Self::Unstamped => "unstamped",
            Self::SameVersion => "same_version",
            Self::Older => "older",
            Self::Confirmed => "confirmed",
            Self::Done => "done",
            Self::NoVerdict => "no_verdict",
        }
    }
}

/// One process of a target's role (§2.6–§2.10): the version it last acted
/// on or adopted, the one it holds while an act runs, per run the highest
/// seq it acted on or adopted (or the asof it reverted on), and its
/// report. A new process is a new `Target`, holding nothing (§2.7). Its
/// rules are `desired.v1`'s machine's (`machine::Core`), read over runs
/// and seqs.
#[derive(Debug, Clone)]
pub struct Target {
    core: machine::Core<Cause, Refusal>,
}

impl Default for Target {
    fn default() -> Self {
        Self::new()
    }
}

impl Target {
    /// A role at its start: nothing held, its report the baseline's. An
    /// unbound role's is the same, and stays so (§2.12).
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: machine::Core::new(),
        }
    }

    /// The report, as the target puts it (§2.10). A revision above
    /// [`MAX_REVISION`] has no code, so its version is reported without
    /// one (§2.4).
    #[must_use]
    pub fn report(&self) -> Report {
        let seen = |(cause, revision): (Cause, Option<u64>)| match cause {
            Cause::Version(version) => Some(Observed {
                version,
                revision: revision.filter(|r| *r <= MAX_REVISION),
            }),
            Cause::NoEntry { .. } => None,
        };
        let l = self.core.ledger();
        Report {
            source: l.source,
            outcome: l.outcome.into(),
            last_known: l.last_known,
            observed: l.observed.and_then(seen),
            in_force: l.in_force.and_then(seen),
            refusal: l.refusal,
        }
    }

    /// The act that runs, if one does.
    #[must_use]
    pub fn running(&self) -> Option<Act> {
        self.core.running().map(|d| Act::from(*d))
    }

    /// The highest seq this process acted on or adopted a version of under
    /// `run`, or the asof it reverted on, if it remembers the run (§2.6
    /// step 4, §2.8).
    #[must_use]
    pub fn remembered(&self, run: Run) -> Option<Seq> {
        self.core.remembered(run)
    }

    /// The stamp a conditional read sends (§2.5): that of the latest
    /// version the target admitted, the one it holds waiting, else the
    /// running act's, else the last it acted on or adopted. `None` where
    /// it holds no version: nothing yet, or a snapshot with no entry it
    /// admitted last.
    #[must_use]
    pub fn held(&self) -> Option<Version> {
        self.holds().map(|(_, v)| v)
    }

    /// [`Target::held`], with whether that version is a put's or a
    /// delete's, as a [`Watch`] moves its digest by.
    #[must_use]
    pub fn holds(&self) -> Option<(Mutation, Version)> {
        let d = self.core.latest()?;
        match d.mark {
            Cause::Version(v) => Some((
                match d.kind {
                    ActKind::Apply => Mutation::Put,
                    ActKind::Revert => Mutation::Delete,
                },
                v,
            )),
            Cause::NoEntry { .. } => None,
        }
    }

    /// Judges a version (§2.6): against the one last acted on or adopted
    /// (the running act's while one runs) and every run remembered, then,
    /// while an act runs, against what waits: an act to start, adopted,
    /// held while one runs, or ignored with why. After [`Step::Adopted`]
    /// and [`Step::Confirmed`] the report may have changed.
    pub fn observe(&mut self, s: Sample) -> Step {
        let Some(version) = s.version else {
            return Step::Unstamped;
        };
        let put = s.mutation == Mutation::Put;
        match self.core.observe(machine::Doing {
            kind: if put { ActKind::Apply } else { ActKind::Revert },
            mark: Cause::Version(version),
            revision: s.revision.filter(|_| put),
            via: s.via,
            content: s.content.filter(|_| put),
        }) {
            machine::Judged::Act(d) => Step::Act(d.into()),
            machine::Judged::Adopted => Step::Adopted,
            machine::Judged::Held => Step::Held,
            machine::Judged::SameVersion => Step::SameVersion,
            machine::Judged::Older => Step::Older,
            machine::Judged::Confirmed => Step::Confirmed,
        }
    }

    /// Judges a snapshot with no entry (§2.8), its header's `run` and its
    /// `asof`: it reverts only where the target holds a document's
    /// version of that run with a seq at or below the asof. Under another
    /// run, over a delete or with nothing held it is no verdict, and a
    /// MUST NOT; built before the held version it is older. While an act
    /// runs it is judged against the running act's version, then against
    /// what waits, and admitted by both it waits as a delete does.
    pub fn observe_no_entry(&mut self, run: Run, asof: Seq) -> Step {
        match self.core.observe_floor(Cause::NoEntry { run, asof }) {
            machine::Floored::NoVerdict => Step::NoVerdict,
            machine::Floored::Older => Step::Older,
            machine::Floored::Held => Step::Held,
            machine::Floored::Act(d) => Step::Act(d.into()),
        }
    }

    /// A conditional read answered unchanged, by the held stamp `sent`
    /// (§2.5): the author's own word of that version, which judges
    /// nothing and confirms a last-known one (§2.9). [`Step::Confirmed`]
    /// where the target holds `sent` from an archive, else
    /// [`Step::SameVersion`].
    pub fn unchanged(&mut self, sent: Version) -> Step {
        if self.core.unchanged(Cause::Version(sent)) {
            Step::Confirmed
        } else {
            Step::SameVersion
        }
    }

    /// Ends the running act (§2.7, §2.10): the report updated, then what
    /// waits, if anything does, adopted or its act started, else
    /// [`Step::Done`]. With no act running, nothing changes.
    pub fn finish(&mut self, ended: Ended) -> Step {
        match self.core.finish(ended) {
            machine::Finished::Done => Step::Done,
            machine::Finished::Adopted => Step::Adopted,
            machine::Finished::Act(d) => Step::Act(d.into()),
        }
    }

    /// The target's own operation changed what is in force (§2.7):
    /// `source` is `call` and `in_force` `none`. The rest stays.
    pub fn call(&mut self) {
        self.core.call();
    }
}

// --- §2.5: the watch --------------------------------------------------------------

/// One section of the author's heartbeat (§2.5, core §7.5): its desired
/// collection's digest and live count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Section {
    pub digest: Digest,
    pub live: u64,
}

/// One heartbeat of the author's, as `freshness.v2` §2.5's steps read it
/// with the author's revision (§2.5, "What it reads"). A group of an owner
/// heartbeat is read as a heartbeat is, its sequence the owner heartbeat
/// key's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beat {
    /// Of another revision than the author's: not read.
    Revision,
    /// One core §7.5 refuses, or a group whose sections are not read: not
    /// read.
    Refused,
    /// Read: its key's stamp, and the collection's section, `None` where
    /// its map bit is 0, or an owner heartbeat holds no group for the
    /// author's interface.
    Read {
        stamp: Stamp,
        section: Option<Section>,
    },
}

/// What a watching target does with one heartbeat (§2.5's rule), the first
/// of its six steps that answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Not read, of another revision: it expects none, and makes no read.
    Revision,
    /// Not read, refused: the same.
    Refused,
    /// Not after the last read one, in its run: nothing changes.
    Older,
    /// The collection's map bit is 0: it expects none, and makes no read.
    NoSection,
    /// The section is the one it expects: nothing is read.
    Unmoved,
    /// The section moved, or it expects none, and a read is outstanding:
    /// it makes none.
    Outstanding,
    /// Make one conditional read of the slice, carrying this held stamp,
    /// or none where the target holds no version.
    Read { held: Option<Version> },
}

impl Decision {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Revision => "revision",
            Self::Refused => "refused",
            Self::Older => "older",
            Self::NoSection => "no_section",
            Self::Unmoved => "unmoved",
            Self::Outstanding => "outstanding",
            Self::Read { .. } => "read",
        }
    }
}

/// How the watch's read ended (§2.5, "The answer").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answered {
    /// By the held stamp the target sent: its author holds that version
    /// still.
    Unchanged,
    /// By a snapshot: an entry, a tombstone or no entry, which the target
    /// judges (§2.6, §2.8).
    Whole,
    /// No answer: a timeout, or a held stamp it did not send (core §4.2,
    /// `held`).
    NoAnswer,
}

impl Answered {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Whole => "whole",
            Self::NoAnswer => "no_answer",
        }
    }
}

/// What a version of its own slice, heard live, did to what the target
/// expects (§2.5, "Its own change costs nothing").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Heard {
    /// The digest and the live count it expects moved with its change.
    Moved,
    /// Nothing moved: it does not move what it expects, it expects none,
    /// or it held no version. It reads once at the next heartbeat.
    Kept,
}

impl Heard {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Moved => "moved",
            Self::Kept => "kept",
        }
    }
}

/// The read a heartbeat made the target make: that heartbeat's section,
/// and the held stamp the read carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Outstanding {
    section: Section,
    sent: Option<Version>,
}

/// A target's watch of its author's heartbeat (§2.5): the section it
/// expects, a digest and a live count, or none. It reads the section for
/// one thing, whether it moved: an unmoved section is never its author's
/// confirmation of the version it holds, and confirms no last-known one.
/// A pure state machine: the runtime reads each heartbeat, makes the read
/// a [`Decision::Read`] asks for, at most one per heartbeat and never
/// while one is outstanding, and tells how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watch {
    key: String,
    moves: bool,
    sequence: crate::freshness::Sequence,
    expects: Option<Section>,
    outstanding: Option<Outstanding>,
}

impl Watch {
    /// A watch of the slice at `key`, the member's base-relative key
    /// (core §7.5's digest reads it): it expects none, has read no
    /// heartbeat, and has no read outstanding. `moves` is §2.5's MAY: the
    /// target moves the digest it expects at a version of its slice heard
    /// live.
    #[must_use]
    pub fn new(key: impl Into<String>, moves: bool) -> Self {
        Self {
            key: key.into(),
            moves,
            sequence: crate::freshness::Sequence::default(),
            expects: None,
            outstanding: None,
        }
    }

    /// The section it expects, or none.
    #[must_use]
    pub fn expects(&self) -> Option<Section> {
        self.expects
    }

    /// Whether a read the watch asked for has not ended.
    #[must_use]
    pub fn outstanding(&self) -> bool {
        self.outstanding.is_some()
    }

    /// Decides one heartbeat (§2.5's rule, steps 1 to 6). `holds` is the
    /// version the target holds ([`Target::held`]): the stamp the read
    /// carries.
    pub fn heartbeat(&mut self, beat: Beat, holds: Option<Version>) -> Decision {
        let (stamp, section) = match beat {
            Beat::Revision => {
                self.expects = None;
                return Decision::Revision;
            }
            Beat::Refused => {
                self.expects = None;
                return Decision::Refused;
            }
            Beat::Read { stamp, section } => (stamp, section),
        };
        let Some(read) = self.sequence.read(stamp) else {
            return Decision::Older;
        };
        // Out of sequence: it follows a change the target did not see.
        if read.gap || read.run {
            self.expects = None;
        }
        let Some(section) = section else {
            self.expects = None;
            return Decision::NoSection;
        };
        if self.expects == Some(section) {
            return Decision::Unmoved;
        }
        if self.outstanding.is_some() {
            return Decision::Outstanding;
        }
        self.outstanding = Some(Outstanding {
            section,
            sent: holds,
        });
        Decision::Read { held: holds }
    }

    /// The outstanding read ended (§2.5, "The answer"). `holds` is the
    /// version the target holds now, after it judged a whole answer.
    /// Unchanged, it expects the section of the heartbeat that made it
    /// read where it still holds the version it sent, else none; whole,
    /// it expects none; with no answer, what it expects does not change,
    /// and the next heartbeat is its next trigger.
    pub fn answer(&mut self, answer: Answered, holds: Option<Version>) -> Answered {
        let Some(out) = self.outstanding.take() else {
            return Answered::NoAnswer;
        };
        match answer {
            Answered::Unchanged => {
                self.expects = (out.sent.is_some() && out.sent == holds).then_some(out.section);
            }
            Answered::Whole => self.expects = None,
            Answered::NoAnswer => {}
        }
        answer
    }

    /// A version of the slice heard live and admitted (§2.5, "Its own
    /// change costs nothing"): `old` is what the target held before it, a
    /// put's or a delete's version, and `new` what it holds now. It moves
    /// only where it expects a section and held a version: a put over a
    /// put XORs the old value out and the new one in, a delete XORs the
    /// old value out and the live count falls by 1, and a put over a
    /// delete XORs the new value in and the count rises by 1.
    pub fn heard(&mut self, old: Option<(Mutation, Version)>, new: (Mutation, Version)) -> Heard {
        let (Some(section), Some(old), true) = (self.expects.as_mut(), old, self.moves) else {
            return Heard::Kept;
        };
        if old.0 == Mutation::Put {
            section.digest.toggle(&self.key, old.1);
            section.live = section.live.saturating_sub(1);
        }
        if new.0 == Mutation::Put {
            section.digest.toggle(&self.key, new.1);
            section.live = section.live.saturating_add(1);
        }
        Heard::Moved
    }
}

// --- §2.10: a tool finds a target's report --------------------------------------

/// Where a role's report is, by the target's contract (§2.10, "A tool
/// finds a target's report"): by type identity and by the annotation,
/// never by a resource's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportAt {
    /// No resource names the role: no report was read.
    None,
    /// Two or more name it: the target's contract says two things of one
    /// role, and the report is unreadable.
    Two,
    /// One names it, and its declaration is invalid: no report was read,
    /// the reasons beside.
    Invalid(Vec<ReportInvalid>),
    /// One names it, declared: the report is the reply on this key, the
    /// resource's with its parameter filled with the role's name.
    At { key: Key, tables: Tables },
}

/// Why a report is unreadable (§2.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    /// Two or more resources name the role.
    TwoReports,
    /// Core §7.6 refuses the reply's record.
    Record(ReadRefusal),
}

impl Unreadable {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TwoReports => "two_reports",
            Self::Record(r) => r.token(),
        }
    }
}

/// A tool's finding of one role's report (§2.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// None was read, with the reasons of an invalid declaration.
    None(Vec<ReportInvalid>),
    /// Unreadable: a tool answers nothing from it (§2.17).
    Unreadable(Unreadable),
    /// The reply read; malformed where it has a finding.
    One(Read),
}

impl Found {
    /// The fixture's token.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None(_) => "none",
            Self::Unreadable(_) => "unreadable",
            Self::One(_) => "one",
        }
    }
}

impl ReportAt {
    /// The report's key, where one resource names the role and is
    /// declared: the key whose reply S4's GET is read for.
    #[must_use]
    pub fn key(&self) -> Option<&Key> {
        match self {
            Self::At { key, .. } => Some(key),
            _ => None,
        }
    }

    /// The finding, given the reply on [`ReportAt::key`] as a tool's merge
    /// left it (observer §4.1): its record's CH tag and its bytes, or
    /// `None` with no reply on that key.
    #[must_use]
    pub fn found(&self, reply: Option<(u8, &[u8])>) -> Found {
        match self {
            Self::None => Found::None(Vec::new()),
            Self::Two => Found::Unreadable(Unreadable::TwoReports),
            Self::Invalid(why) => Found::None(why.clone()),
            Self::At { .. } => match reply {
                None => Found::None(Vec::new()),
                Some((tag, bytes)) => match read(tag, bytes) {
                    Ok(r) => Found::One(r),
                    Err(e) => Found::Unreadable(Unreadable::Record(e)),
                },
            },
        }
    }
}

/// Finds where `role`'s report is (§2.10): among `resources`, those of the
/// contract of the target `target` that declares the role, interface
/// `iface`, the ones whose `desired.report` names it, by its name or by a
/// parameter in braces. `is_role` says whether a name is a requirement of
/// that contract.
#[must_use]
pub fn report_at(
    target: &Addr,
    iface: &IfaceId,
    role: &str,
    resources: &[ReportResource<'_>],
    is_role: impl Fn(&str) -> bool,
) -> ReportAt {
    let naming: Vec<usize> = resources
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r.annotations
                .get(REPORT_KEY)
                .and_then(Names::read)
                .is_some_and(|n| n.names(role))
        })
        .map(|(i, _)| i)
        .collect();
    let at = match naming.as_slice() {
        [] => return ReportAt::None,
        [one] => *one,
        _ => return ReportAt::Two,
    };
    let r = &resources[at];
    let others = resources
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != at)
        .filter_map(|(_, o)| o.annotations.get(REPORT_KEY));
    let (names, tables) = match report_declaration(r, is_role, others) {
        ReportDeclaration::Declared { names, tables } => (names, tables),
        ReportDeclaration::Invalid(why) => return ReportAt::Invalid(why),
        ReportDeclaration::NotThisProfiles => return ReportAt::None,
    };
    let mut values = Bindings::new();
    if let Names::Param(p) = names {
        values.insert(p, vec![role.to_owned()]);
    }
    let key = r.template.build(&values).ok().and_then(|chunks| {
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        grammar::data_key(target, iface, r.token, &refs).ok()
    });
    match key {
        Some(key) => ReportAt::At { key, tables },
        // A declared resource has no parameter but the one filled.
        None => ReportAt::None,
    }
}

/// [`report_at`] in a contract of the target's, the one that declares the
/// role.
#[must_use]
pub fn report_at_of(target: &Addr, c: &Contract, role: &str) -> ReportAt {
    let resources: Vec<ReportResource<'_>> = c.resources.iter().map(ReportResource::of).collect();
    report_at(target, &c.iface, role, &resources, |name| {
        c.requires.contains_key(name)
    })
}

// --- §2.17, §5: what a tool may conclude ---------------------------------------

/// The author's answer to S4's GET of the slice (§2.17 step 6): a member
/// read of one concrete key, answered by snapshot (§2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Current {
    /// No GET made: with no revision of the author's interface in hand, a
    /// tool has no slice (§2.17).
    NotRead,
    /// A GET that ended at its timeout or with an error reply.
    Incomplete,
    /// A reply with no envelope, which carries no version.
    Unstamped,
    /// A GET that drew no reply.
    Silent,
    /// A live entry's version.
    Entry(Version),
    /// A tombstone's version.
    Tombstone(Version),
    /// A snapshot with no entry: its header's run.
    NoEntry(Run),
}

/// What the tool read of the author (§2.17 step 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorReading {
    pub presence: Seen,
    pub current: Current,
}

/// §5's first question, "has the target converged on its author's current
/// document?" (§2.17), of a `desired.v2` report: v1's premises and the
/// author's presence, then the author's version by its snapshot, compared
/// by run and seq. A snapshot with no entry answers only under the run of
/// the report's observed version, by the outcome: yes `reverted`, else not
/// yet `behind`; under another run, or with no observed version, it is
/// `author_silent`. No is the finding.
#[must_use]
pub fn converged(t: &TargetReading<'_, Read>, a: &AuthorReading) -> Judged {
    use Answer::{NotYet, Unobservable, Yes};
    let judged = |answer, reason| Judged { answer, reason };
    let report = match super::converging(t, a.presence) {
        Ok((
            Read {
                report: Some(r), ..
            },
            _,
        )) => *r,
        Ok(_) => return judged(Unobservable, Reason::ReportInvalid),
        Err(j) => return j,
    };
    let observed = report.observed.map(|o| o.version);
    let current = match a.current {
        Current::NotRead => return judged(Unobservable, Reason::AuthorNotRead),
        Current::Incomplete | Current::Unstamped => {
            return judged(Unobservable, Reason::AuthorReadIncomplete);
        }
        Current::Silent => return judged(Unobservable, Reason::AuthorSilent),
        Current::NoEntry(run) => {
            return match observed {
                Some(o) if o.run == run && report.outcome == Outcome::Reverted => {
                    judged(Yes, Reason::Reverted)
                }
                Some(o) if o.run == run => judged(NotYet, Reason::Behind),
                _ => judged(Unobservable, Reason::AuthorSilent),
            };
        }
        Current::Entry(v) | Current::Tombstone(v) => v,
    };
    match (observed, report.outcome.acted()) {
        (Some(o), Some(outcome)) => super::compared(&o, &current, outcome),
        _ => judged(NotYet, Reason::NotActed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        parse_version(text).expect("a version")
    }

    fn put(text: &str, revision: u64) -> Sample {
        Sample {
            via: Via::Owner,
            mutation: Mutation::Put,
            version: Some(v(text)),
            revision: Some(revision),
            content: Some(Content::of_json(&json!({"revision": revision}))),
        }
    }

    #[test]
    fn a_version_is_written_as_its_run_and_its_seq() {
        assert_eq!(version_text(v("0007/12")), "0007/12");
        assert_eq!(version_text(v("00000007/12")), "00000007/12");
        assert_ne!(v("0007/12"), v("00000007/12"), "two widths are two runs");
        assert_eq!(v("ffff/16777215").seq, Seq::MAX);
        for bad in [
            "",
            "7/12",
            "007/12",
            "00007/12",
            "0007/",
            "/12",
            "0007/012",
            "0007/+1",
            "0007/1 ",
            "000G/1",
            "000A/1",
            "0007/16777216",
            "0007/1/2",
            "0007",
            "0007/-1",
            "0007/000000001",
        ] {
            assert_eq!(parse_version(bad), None, "{bad:?}");
        }
        assert_eq!(parse_version("0007/0"), Some(v("0007/0")));
    }

    #[test]
    fn the_layout_is_the_texts() {
        let l = layout();
        assert_eq!(l.tag(), TAG);
        assert_eq!(l.min_bits(), 20);
        assert_eq!(Report::baseline().write().unwrap().bytes, [0, 0, 0]);
        // The longest consistent report (§3): refused over a document in
        // force, both versions on 32-bit runs.
        let longest = Report {
            source: Source::Desired,
            outcome: Outcome::Refused,
            last_known: true,
            observed: Some(Observed {
                version: v("00a1b2c3/9"),
                revision: Some(4),
            }),
            in_force: Some(Observed {
                version: v("00a1b2c3/4"),
                revision: Some(3),
            }),
            refusal: Some(Refusal {
                code: Code::Reach,
                pointer: Some(Index::At(3)),
                detail: Some(Index::OutOfRange),
            }),
        };
        let record = longest.write().unwrap();
        assert_eq!((record.bits, record.bytes.len()), (215, 27));
        assert_eq!(
            read(TAG, &record.bytes).unwrap().report,
            Some(longest),
            "every report reads back"
        );
    }

    #[test]
    fn every_report_the_machine_makes_is_consistent() {
        let mut t = Target::new();
        let mut seen = vec![t.report()];
        let mut note = |t: &Target| seen.push(t.report());
        assert!(matches!(t.observe(put("0007/3", 1)), Step::Act(_)));
        assert_eq!(t.observe(put("0007/4", 2)), Step::Held);
        assert!(matches!(t.finish(Ended::Done), Step::Act(_)));
        note(&t);
        t.finish(Ended::Refused(Refusal::new(Code::Reach)));
        note(&t);
        t.call();
        note(&t);
        // A revision above the bound: refused whole, reported without one.
        let big = put("0007/5", MAX_REVISION + 1);
        let Step::Act(act) = t.observe(big) else {
            panic!("a new document is acted on");
        };
        let refusal = act.bound().expect("above the bound");
        t.finish(Ended::Refused(refusal));
        assert_eq!(t.report().observed.unwrap().revision, None);
        note(&t);
        // A snapshot with no entry under the held run reverts; a revert
        // that could not finish keeps the version it was reverting.
        let run = v("0007/0").run;
        let asof = Seq::new(20).unwrap();
        assert!(matches!(t.observe_no_entry(run, asof), Step::Act(_)));
        t.finish(Ended::Failed(Refusal::new(Code::Apply)));
        assert_eq!(t.report().outcome, Outcome::Failed);
        assert_eq!(t.report().observed.map(|o| o.version), Some(v("0007/5")));
        note(&t);
        assert_eq!(t.observe(put("0007/20", 3)), Step::Older, "at the asof");
        assert!(matches!(t.observe(put("0007/21", 3)), Step::Act(_)));
        t.finish(Ended::Done);
        note(&t);
        assert!(matches!(
            t.observe_no_entry(run, Seq::new(30).unwrap()),
            Step::Act(_)
        ));
        t.finish(Ended::Done);
        assert_eq!(t.report().observed, None);
        assert_eq!(t.held(), None, "after it the target holds no document");
        note(&t);
        for r in &seen {
            assert!(r.check().is_empty(), "{r:?}: {:?}", r.check());
            let record = r.write().expect("a record");
            assert_eq!(read(TAG, &record.bytes).unwrap().report.as_ref(), Some(r));
        }
    }

    #[test]
    fn an_asof_at_the_held_seq_makes_that_version_older() {
        let mut t = Target::new();
        assert!(matches!(t.observe(put("0007/20", 1)), Step::Act(_)));
        t.finish(Ended::Done);
        assert_eq!(t.observe(put("0007/20", 1)), Step::SameVersion);
        let run = v("0007/0").run;
        assert!(matches!(
            t.observe_no_entry(run, Seq::new(20).unwrap()),
            Step::Act(_)
        ));
        t.finish(Ended::Done);
        // §2.8, "What it records": at or below the asof is older.
        assert_eq!(t.observe(put("0007/20", 1)), Step::Older);
        assert_eq!(t.remembered(run), Seq::new(20));
    }

    #[test]
    fn an_unchanged_answer_confirms_only_the_version_held() {
        let archived = Sample {
            via: Via::Archive,
            ..put("0007/3", 1)
        };
        let mut t = Target::new();
        assert!(matches!(t.observe(archived), Step::Act(_)));
        t.finish(Ended::Done);
        assert!(t.report().last_known);
        assert_eq!(t.unchanged(v("0007/2")), Step::SameVersion);
        assert!(t.report().last_known, "another version's word");
        assert_eq!(t.unchanged(v("0007/3")), Step::Confirmed);
        assert!(!t.report().last_known);
        assert_eq!(t.unchanged(v("0007/3")), Step::SameVersion);
        // A run forgotten is never admitted by an unchanged answer.
        assert_eq!(t.unchanged(v("0099/1")), Step::SameVersion);
        assert_eq!(t.held(), Some(v("0007/3")));
    }

    #[test]
    fn a_place_is_its_nearest_listed_ancestor() {
        let t = Tables {
            pointers: ["", "/a", "/a/b", "/a~1b"].map(str::to_owned).to_vec(),
            details: vec!["x".into()],
        };
        assert_eq!(t.place(""), Index::At(0));
        assert_eq!(t.place("/a/b/c/d"), Index::At(2));
        assert_eq!(t.place("/a/c"), Index::At(1));
        assert_eq!(t.place("/ab"), Index::OutOfRange);
        assert_eq!(t.place("/z"), Index::OutOfRange, "never index 0");
        assert_eq!(t.line("x"), Index::At(0));
        assert_eq!(t.line("y"), Index::OutOfRange);
        assert_eq!(t.pointer(Index::At(3)), Word::Entry("/a~1b"));
        assert_eq!(t.pointer(Index::At(9)), Word::Number(9));
        assert_eq!(word(None, Index::At(1)), Word::Number(1));
        assert_eq!(t.detail(Index::OutOfRange), Word::OutOfRange);
    }

    #[test]
    fn a_watch_makes_no_read_while_one_is_outstanding() {
        let key = "zk2/lab/author/tasking.v2/state/tasks/lab/worker";
        let held = v("0007/3");
        let section = |seq: u32| Section {
            digest: Digest::of([(key, v(&format!("0007/{seq}")))]),
            live: 1,
        };
        let beat = |seq: u32, s: Section| Beat::Read {
            stamp: v(&format!("0021/{seq}")),
            section: Some(s),
        };
        let mut w = Watch::new(key, true);
        assert_eq!(
            w.heartbeat(beat(0, section(3)), Some(held)),
            Decision::Read { held: Some(held) }
        );
        assert!(w.outstanding());
        assert_eq!(
            w.heartbeat(beat(1, section(3)), Some(held)),
            Decision::Outstanding
        );
        assert_eq!(
            w.answer(Answered::Unchanged, Some(held)),
            Answered::Unchanged
        );
        assert_eq!(w.expects(), Some(section(3)));
        // An answer with no read outstanding changes nothing.
        assert_eq!(w.answer(Answered::Whole, Some(held)), Answered::NoAnswer);
        assert_eq!(w.expects(), Some(section(3)));
        // Its own change, moved: the next section is the one it expects.
        let new = v("0007/9");
        assert_eq!(
            w.heard(Some((Mutation::Put, held)), (Mutation::Put, new)),
            Heard::Moved
        );
        assert_eq!(
            w.heartbeat(beat(2, section(9)), Some(new)),
            Decision::Unmoved
        );
        // It held nothing: nothing to move.
        assert_eq!(w.heard(None, (Mutation::Put, new)), Heard::Kept);
    }
}
