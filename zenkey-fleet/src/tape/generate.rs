//! `zenctl gen` (#612, FJ8a): a contract-driven mock owner.
//!
//! Given contracts and an address, it brings a zk2 service up there
//! ([`crate::tape::mock`]: a real instance, a descriptor and tokens, P3) and
//! publishes every stream, state and event resource of each interface
//! through the runtime's writers, which give each sample the contract's
//! QoS and `Encoding`, each state put the owner's stamp (S1) and each event
//! a fresh ULID key (§2.6). Every operation is served over its template,
//! answering each call with one synthesized `response` (and its `summary`,
//! when `replies = "many"` declares one).
//!
//! Payloads come from [`crate::tape::synth`]: deterministic per
//! `(seed, tick)`, and a JSON Schema value that would not validate is never
//! sent. A templated resource publishes the members `--member` names, or a
//! small synthetic set the plan states; the template that declares `epoch`
//! holds a member token per value (§8.1).
//!
//! **An event keeps its declared rate** (core §2.7, #744). The runtime's
//! event writer refuses an occurrence beyond the member's rate (at most `n`
//! within any period of its own length), so a mock that out-paced it would
//! only count refusals. An event's rate is its declared one whatever
//! `--rate` asks, and the plan says so once, on each entry it held back;
//! its occurrences are capped over the run ([`GenPlanEntry::events_cap`])
//! and paced so that none is attempted before the runtime's own window
//! would admit it, whatever the `--pattern`. A write the runtime refuses
//! all the same is counted apart ([`GenReport::refused`]), never as a
//! failure and never silently.
//!
//! **A governed interface elects** (`redundancy.v1`, #841, RC). With
//! [`GenSpec::elect`], the mock owner is a candidate of every interface
//! whose contract lists `redundancy.v1`: it claims, stands by and takes
//! over through the runtime ([`zenkey::ServiceBuilder::redundancy`]), its
//! writers and servers held undeclared while it stands by. A governed
//! state's first value is put at the takeover (§2.6 step 3), never before
//! start; a put refused while it stands by is counted apart
//! ([`GenReport::standing_by`]); its member tokens follow its standing
//! through the election's own watch, each declared once per incumbency
//! (§2.6 step 4), a failed declaration made again. Without `elect`, the
//! runtime refuses a governed interface at start.
//!
//! The plan ([`build_plan`]) is session-free and printed before anything
//! is brought up, the replay dry-run precedent. It replaced v1's
//! registry-driven generator: the registry walk, the schema ladder through
//! a served `describe`, the impersonated `--origin`, `--serve-describe` and
//! the fault injector went with it (the `v1` branch keeps them).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey::OpError;
use zenkey_model::authoring::{Kind, ParamType};
use zenkey_model::contract::{Body, Rate, Replies, Resource};
use zenkey_model::grammar::{Addr, IfaceId, data_key};
use zenkey_model::template::{Bindings, Segment};
use zenoh::Session;

use crate::model::catalog::Revision;
use crate::model::render::Member;
use crate::report::{GenInterface, GenPlan, GenPlanEntry, GenReport, MemberSource, QosView};
use crate::tape::mock::{
    MockAnswer, capabilities, check_roles, config, default_value, implementation, marker,
    runtime_error, serve_answer,
};
use crate::tape::synth::Synth;
use crate::{Error, Result};

/// The send-timing shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenPattern {
    /// Fixed interval.
    Steady,
    /// Interval jittered ±30%, seeded: reproducible irregularity.
    Jitter,
    /// The per-second budget sent at once, then a pause.
    Burst,
    /// Rate climbs linearly from ~0 to the full rate over the duration.
    Ramp,
}

/// How many members a templated resource gets when `--member` names none.
pub const DEFAULT_MEMBERS: usize = 2;

/// One `--member`: a resource, and the members it publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberArg {
    /// The resource: its template, or `<kind token>/<template>`.
    pub resource: String,
    /// Each member's parameter values, unslugged, in template order; a
    /// rest parameter takes the values left over.
    pub members: Vec<Vec<String>>,
}

impl MemberArg {
    /// Parses `<resource>=<member>[,<member>…]`, each member its parameter
    /// values in template order joined by `/`: `bandwidth/{ns}/{iface}=
    /// default/eth0,default/eth1`. A value holding `/` or `,` cannot be
    /// spelled here.
    pub fn parse(s: &str) -> Result<MemberArg> {
        let refuse = |why: &str| Error::unaskable(format!("--member {s}"), why);
        let (resource, members) = s
            .split_once('=')
            .ok_or_else(|| refuse("expected <resource>=<member>[,<member>…]"))?;
        if resource.is_empty() {
            return Err(refuse("names no resource"));
        }
        let members: Vec<Vec<String>> = members
            .split(',')
            .map(|m| m.split('/').map(str::to_owned).collect())
            .collect();
        if members
            .iter()
            .any(|m: &Vec<String>| m.iter().all(String::is_empty))
        {
            return Err(refuse("a member is empty"));
        }
        Ok(MemberArg {
            resource: resource.to_owned(),
            members,
        })
    }
}

/// What to bring up and publish.
#[derive(Debug, Clone)]
pub struct GenSpec {
    /// The address the mock owner runs at: the operator's to name.
    pub address: Addr,
    /// The revisions it implements, one per interface.
    pub revisions: Vec<Arc<Revision>>,
    /// Members for templated resources (`--member`).
    pub members: Vec<MemberArg>,
    /// Role bindings (R1), `role → providers`.
    pub bindings: BTreeMap<String, Vec<String>>,
    /// Override every stream's and state's rate (Hz); an event's stays
    /// within its declared rate whatever this says, and the plan says so
    /// where this asked for more (#744).
    pub rate_hz: Option<f64>,
    pub pattern: GenPattern,
    pub duration: Duration,
    /// Drives synthesis and jitter: the same seed is the same run.
    pub seed: u64,
    /// Stamped into the descriptor's marker.
    pub tool: String,
    /// How the mock owner takes part in `redundancy.v1`'s election, for
    /// every governed interface it implements: its reader and its watch
    /// (§2.5 step 1, §2.7). `None`: a governed interface is refused at
    /// start, as the runtime refuses it.
    pub elect: Option<zenkey::redundancy::RedundancyStartup>,
}

/// An event's cap on occurrences over a run of `duration`: its declared
/// rate (§2.6), at least one.
fn events_cap(rate: Rate, duration: Duration) -> u64 {
    let per_h: u64 = match rate {
        Rate::Rare => 1,
        Rate::Low => 60,
        Rate::Burst(n) => u64::from(n),
    };
    let run = (per_h.min(3600) as f64 * duration.as_secs_f64() / 3600.0).floor() as u64;
    run.max(1).min(per_h)
}

/// The most a steady rate can be and stay within `rate` (core §2.7: `n`
/// occurrences within any period), Hz.
fn declared_hz(rate: Rate) -> f64 {
    let (n, period) = rate.limit();
    f64::from(n) / period.as_secs_f64()
}

/// A declared rate, spelled and said: `low (at most 1 per minute)`.
fn rate_words(rate: Rate) -> String {
    let (n, period) = rate.limit();
    let per = match period.as_secs() {
        60 => "minute".to_owned(),
        3600 => "hour".to_owned(),
        s => format!("{s} s"),
    };
    format!("{rate} (at most {n} per {per})")
}

/// An event member's pacing within its declared rate (core §2.7, #744):
/// the instants of its occurrences within the rate's period, so the next
/// one is attempted only when the runtime's own window admits it.
///
/// The runtime counts an occurrence at its admission, inside the put; this
/// records it when the put has returned, never earlier, so its window lags
/// the runtime's and never offers an instant the runtime would refuse.
#[derive(Debug)]
struct Pace {
    allowed: usize,
    period: Duration,
    recent: VecDeque<Instant>,
}

impl Pace {
    fn new(rate: Rate) -> Pace {
        let (n, period) = rate.limit();
        Pace {
            allowed: usize::try_from(n).unwrap_or(usize::MAX).max(1),
            period,
            recent: VecDeque::new(),
        }
    }

    /// The earliest instant from `now` at which one more occurrence is
    /// within the rate: `now` while the period holds fewer than `n`, else
    /// when its oldest leaves it.
    fn ready_at(&mut self, now: Instant) -> Instant {
        while self
            .recent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.period)
        {
            self.recent.pop_front();
        }
        match self.recent.front() {
            Some(oldest) if self.recent.len() >= self.allowed => *oldest + self.period,
            _ => now,
        }
    }

    /// An occurrence the runtime admitted, its put returned at `at`.
    fn record(&mut self, at: Instant) {
        self.recent.push_back(at);
        while self.recent.len() > self.allowed {
            self.recent.pop_front();
        }
    }
}

/// The wait after tick `tick`'s sample under `pattern`: `base` is one over
/// the entry's rate, `elapsed` how far into a run of `total` seconds it is.
/// An event's [`Pace`] may hold the next one back further.
fn interval(
    pattern: GenPattern,
    base: Duration,
    rate: f64,
    tick: u64,
    salt: u64,
    elapsed: f64,
    total: f64,
) -> Duration {
    match pattern {
        GenPattern::Steady => base,
        GenPattern::Jitter => base.mul_f64(0.7 + 0.6 * halton(salt ^ tick)),
        GenPattern::Burst => {
            if tick.is_multiple_of(rate.ceil().max(1.0) as u64) {
                Duration::from_secs(1)
            } else {
                Duration::ZERO
            }
        }
        GenPattern::Ramp => base.div_f64((elapsed / total).clamp(0.05, 1.0)),
    }
}

/// The base-relative key expression of every member of `r`: each parameter
/// `*`, a rest parameter `**`.
fn pattern_of(addr: &Addr, iface: &IfaceId, r: &Resource) -> String {
    let mut chunks = vec![
        "zk2".to_owned(),
        addr.system.to_string(),
        addr.service.to_string(),
        iface.to_string(),
        r.token.to_string(),
    ];
    chunks.extend(r.template.segments().iter().map(|seg| match seg {
        Segment::Literal(l) => l.clone(),
        Segment::Param(_) => "*".to_owned(),
        Segment::Rest(_) => "**".to_owned(),
    }));
    chunks.join("/")
}

/// Whether a `--member` names `r`.
fn names(arg: &MemberArg, r: &Resource) -> bool {
    arg.resource == zenkey::implementation::resource_name(r) || arg.resource == r.template.as_str()
}

/// The members `--member` gives `r`, refused when they do not fit its
/// template; `None` when no `--member` names it.
fn given_members(r: &Resource, args: &[MemberArg]) -> Result<Option<Vec<Bindings>>> {
    let name = zenkey::implementation::resource_name(r);
    let mut out: Option<Vec<Bindings>> = None;
    let params: Vec<(&str, bool)> = r.template.params().collect();
    for arg in args.iter().filter(|a| names(a, r)) {
        let what = format!("--member {}", arg.resource);
        if r.kind == Kind::Operation {
            return Err(Error::unaskable(
                what,
                format!("{name} is an operation: a mock serves it over its whole template"),
            ));
        }
        if params.is_empty() {
            return Err(Error::unaskable(
                what,
                format!("{name} has no template parameters: it is its one member"),
            ));
        }
        for member in &arg.members {
            let rest = params.last().is_some_and(|(_, rest)| *rest);
            let fits = if rest {
                member.len() >= params.len()
            } else {
                member.len() == params.len()
            };
            if !fits {
                return Err(Error::unaskable(
                    what,
                    format!(
                        "{} is {} value(s), and {} takes {} ({}), in template order, joined by `/`",
                        member.join("/"),
                        member.len(),
                        r.template,
                        params.len(),
                        params
                            .iter()
                            .map(|(n, _)| *n)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
            let mut values = Bindings::new();
            for (i, (n, is_rest)) in params.iter().enumerate() {
                let v = if *is_rest {
                    member[i..].to_vec()
                } else {
                    vec![member[i].clone()]
                };
                values.insert((*n).to_owned(), v);
            }
            out.get_or_insert_with(Vec::new).push(values);
        }
    }
    if let Some(members) = &out
        && let Some(card) = r.cardinality
        && members.len() as u64 > card
    {
        return Err(Error::unaskable(
            format!("--member {name}"),
            format!(
                "{} members, above the contract's cardinality {card} (§2.2)",
                members.len()
            ),
        ));
    }
    Ok(out)
}

/// The synthetic members of `r`: `<param>-1`, `<param>-2` (a `uint`
/// parameter's are `1`, `2`), at most its cardinality.
fn default_members(r: &Resource) -> Vec<Bindings> {
    let n = r
        .cardinality
        .map_or(DEFAULT_MEMBERS, |c| {
            DEFAULT_MEMBERS.min(usize::try_from(c).unwrap_or(usize::MAX))
        })
        .max(1);
    (1..=n)
        .map(|i| {
            r.template
                .params()
                .map(|(name, _)| {
                    let v = match r.params.get(name) {
                        Some(ParamType::Uint) => i.to_string(),
                        _ => default_value(name, i),
                    };
                    (name.to_owned(), vec![v])
                })
                .collect()
        })
        .collect()
}

fn qos_of(r: &Resource) -> Option<QosView> {
    match &r.body {
        Body::Data(d) => Some(QosView {
            reliability: d.reliability,
            congestion: d.congestion,
            priority: d.priority,
            express: d.express,
        }),
        Body::Operation(_) => None,
    }
}

/// A type, named as §7.2 names it.
fn declared(rev: &Revision, r: &Resource, member: Member) -> String {
    zenkey_model::decode::type_of(
        rev.bundle(),
        r.token.as_str(),
        r.template.as_str(),
        member.as_str(),
    )
    .map(zenkey_model::decode::declared)
    .unwrap_or_default()
}

/// The plan, before anything touches the bus: every member of every
/// stream, state and event resource, and every operation, with its key,
/// type, `Encoding`, QoS and rate. Session-free.
///
/// Refused (an [`Error::Unaskable`]): no contract, an interface given
/// twice, a required role left unbound (R1), a `--member` that names no
/// resource of these contracts or does not fit its template. A type the
/// synthesizer cannot satisfy is not refused: its entry says so, and
/// publishes nothing.
pub fn build_plan(spec: &GenSpec) -> Result<GenPlan> {
    if spec.revisions.is_empty() {
        return Err(Error::unaskable(
            "gen",
            "no contract: name the interfaces to serve, with --contracts",
        ));
    }
    let mut seen = BTreeSet::new();
    for rev in &spec.revisions {
        if !seen.insert(rev.iface().clone()) {
            return Err(Error::unaskable(
                rev.iface().to_string(),
                "is given twice: a service implements an interface major once",
            ));
        }
    }
    check_roles(&spec.revisions, &spec.bindings)?;
    for arg in &spec.members {
        let known = spec
            .revisions
            .iter()
            .any(|rev| rev.contract().resources.iter().any(|r| names(arg, r)));
        if !known {
            return Err(Error::unaskable(
                format!("--member {}", arg.resource),
                "names no resource of the contracts given",
            ));
        }
    }
    let synth = Synth::new(spec.seed);
    let secs = spec.duration.as_secs_f64();
    let mut entries = Vec::new();
    for rev in &spec.revisions {
        let iface = rev.iface();
        for r in &rev.contract().resources {
            let name = zenkey::implementation::resource_name(r);
            // A `--member` for an operation is refused here.
            let given = given_members(r, &spec.members)?;
            let d = match &r.body {
                Body::Operation(op) => {
                    entries.push(operation_entry(spec, rev, r, op, &synth));
                    continue;
                }
                Body::Data(d) => d,
            };
            let (members, source) = if !r.template.has_params() {
                (vec![Bindings::new()], MemberSource::Fixed)
            } else {
                match given {
                    Some(m) => (m, MemberSource::Given),
                    None => (default_members(r), MemberSource::Default),
                }
            };
            let cap = d.rate.map(|rate| events_cap(rate, spec.duration));
            // An event keeps its declared rate (#744): `--rate` above it is
            // held back to it, and the entry says so.
            let mut kept = None;
            let rate_hz = match (r.kind, d.rate, cap) {
                (Kind::Event, Some(rate), Some(cap)) => match spec.rate_hz {
                    None => (cap as f64 / secs).min(1.0).clamp(0.001, 1000.0),
                    Some(asked) => {
                        let declared = declared_hz(rate);
                        if asked > declared {
                            kept = Some(format!(
                                "--rate {asked} Hz is above its declared rate {}: it keeps \
                                 its declared rate (core §2.7)",
                                rate_words(rate)
                            ));
                        }
                        asked.clamp(0.001, 1000.0).min(declared)
                    }
                },
                (Kind::State, ..) => spec.rate_hz.unwrap_or(0.5).clamp(0.001, 1000.0),
                _ => spec.rate_hz.unwrap_or(1.0).clamp(0.001, 1000.0),
            };
            // telemetry.v1 §2.9: an Epoch attachment is the runtime's
            // writer's to write, never synthesized (#819, TC).
            let epoch_attached = zenkey_model::telemetry::declaration_of(rev.contract(), r)
                .declared()
                .is_some_and(|d| d.epoch == Some(zenkey_model::telemetry::EpochIn::Attachment));
            let made = synth.sample(rev, r, Member::Type, 0).and_then(|_| {
                if d.attachment.is_some() && !epoch_attached {
                    synth.sample(rev, r, Member::Attachment, 0).map(|_| ())
                } else {
                    Ok(())
                }
            });
            let mut notes = Vec::new();
            match &made {
                Err(e) => notes.push(format!("cannot synthesize ({e}): publishes nothing")),
                Ok(()) => notes.extend(kept),
            }
            if d.attachment.is_some() {
                notes.push(if epoch_attached {
                    "its attachment is telemetry.v1's Epoch, which the runtime's writer writes: \
                     the instance id (telemetry.v1 §2.9)"
                        .to_owned()
                } else if r.kind == Kind::Stream {
                    "each sample carries a synthesized attachment".to_owned()
                } else {
                    "the runtime's state and event writers put no attachment: samples go \
                     without one"
                        .to_owned()
                });
            }
            if source == MemberSource::Default {
                notes.push("synthetic members (name them with --member)".to_owned());
            }
            for values in members {
                let chunks = r
                    .template
                    .build(&values)
                    .map_err(|e| Error::unaskable(format!("--member {name}"), e))?;
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                let key = data_key(&spec.address, iface, r.token, &refs)
                    .map_err(|e| Error::unaskable(format!("--member {name}"), e.to_string()))?
                    .to_string();
                entries.push(GenPlanEntry {
                    iface: iface.to_string(),
                    resource: name.clone(),
                    kind: r.kind,
                    key,
                    encoding: zenkey::writer::wire_encoding(&d.type_, d.encoding, &values)
                        .to_string(),
                    values,
                    members: source,
                    declared: declared(rev, r, Member::Type),
                    qos: qos_of(r),
                    rate_hz: made.is_ok().then_some(rate_hz),
                    events_cap: cap.filter(|_| made.is_ok()),
                    member_token: r.epoch.is_some(),
                    note: (!notes.is_empty()).then(|| notes.join("; ")),
                });
            }
        }
    }
    Ok(GenPlan {
        address: spec.address.to_string(),
        interfaces: spec
            .revisions
            .iter()
            .map(|r| GenInterface {
                iface: r.iface().to_string(),
                fingerprint: r.fingerprint().to_string(),
            })
            .collect(),
        duration_s: secs,
        seed: spec.seed,
        marker: marker(&spec.tool, Some(spec.seed)),
        entries,
    })
}

/// An operation's entry: served over its template, answered with one
/// synthesized response, or `internal` when none can be made.
fn operation_entry(
    spec: &GenSpec,
    rev: &Revision,
    r: &Resource,
    op: &zenkey_model::contract::Operation,
    synth: &Synth,
) -> GenPlanEntry {
    let made = synth.sample(rev, r, Member::Response, 0);
    let mut note = match &made {
        Ok(_) => format!(
            "answers each call with one synthesized response{}",
            if op.replies == Replies::Many && op.summary.is_some() {
                ", then its summary"
            } else {
                ""
            }
        ),
        Err(e) => format!("cannot synthesize a response ({e}): answers `internal`"),
    };
    if r.template.has_params() {
        note.push_str("; served over its whole template");
    }
    GenPlanEntry {
        iface: rev.iface().to_string(),
        resource: zenkey::implementation::resource_name(r),
        kind: Kind::Operation,
        key: pattern_of(&spec.address, rev.iface(), r),
        values: BTreeMap::new(),
        members: MemberSource::Fixed,
        declared: declared(rev, r, Member::Response),
        encoding: zenkey::writer::wire_encoding(&op.response, op.encoding, &Bindings::new())
            .to_string(),
        qos: None,
        rate_hz: None,
        events_cap: None,
        member_token: false,
        note: Some(note),
    }
}

/// The resource an entry names, and its revision.
fn lookup<'a>(spec: &'a GenSpec, e: &GenPlanEntry) -> Result<(&'a Arc<Revision>, &'a Resource)> {
    spec.revisions
        .iter()
        .find(|rev| rev.iface().to_string() == e.iface)
        .and_then(|rev| {
            rev.contract()
                .resources
                .iter()
                .find(|r| zenkey::implementation::resource_name(r) == e.resource)
                .map(|r| (rev, r))
        })
        .ok_or_else(|| {
            Error::Internal(format!(
                "the plan names {} {}, which no contract declares",
                e.iface, e.resource
            ))
        })
}

/// One data entry's writer.
enum Out {
    /// Boxed: a publisher is the largest of the three.
    Stream(Box<zenkey::writer::Writer>),
    State(Arc<zenkey::state::StateWriter>),
    Event(zenkey::writer::EventWriter),
}

/// What one entry's task did.
#[derive(Default)]
struct Tally {
    sent: u64,
    failed: u64,
    /// Writes the runtime's budget refused (core §2.7): apart from `failed`.
    refused: u64,
    /// Writes refused while the instance stood by (`redundancy.v1` §2.4):
    /// apart from both.
    standing: u64,
    errors: Vec<String>,
}

impl Tally {
    fn fail(&mut self, e: String) {
        self.failed += 1;
        self.example(e);
    }

    /// One put's outcome: sent, refused by the runtime's budget, or failed.
    fn put(&mut self, key: &str, put: zenkey::Result<()>) {
        match put {
            Ok(()) => self.sent += 1,
            Err(zenkey::Error::Standby(_)) => self.standing += 1,
            Err(e @ zenkey::Error::Budget(_)) => {
                self.refused += 1;
                self.example(format!("{key}: {e}"));
            }
            Err(e) => self.fail(format!("{key}: {e}")),
        }
    }

    fn example(&mut self, e: String) {
        if self.errors.len() < 3 {
            self.errors.push(e);
        }
    }
}

/// The answer an operation entry is served with.
fn answer_of(rev: &Revision, r: &Resource, synth: &Synth) -> MockAnswer {
    let Body::Operation(op) = &r.body else {
        return MockAnswer::Refuse(OpError::internal("not an operation"));
    };
    let reply = synth.sample(rev, r, Member::Response, 0);
    let summary = match (&op.replies, &op.summary) {
        (Replies::Many, Some(_)) => Some(synth.sample(rev, r, Member::Summary, 0)),
        _ => None,
    };
    match (reply, summary) {
        (Ok(reply), None) => MockAnswer::Reply {
            bytes: reply.bytes,
            summary: None,
        },
        (Ok(reply), Some(Ok(s))) => MockAnswer::Reply {
            bytes: reply.bytes,
            summary: Some(s.bytes),
        },
        (Err(why), _) | (_, Some(Err(why))) => MockAnswer::Refuse(OpError::internal(format!(
            "this mock cannot synthesize its answer: {why}"
        ))),
    }
}

/// Brings the mock owner up and runs the plan until the duration elapses,
/// then takes it down: its operation servers, then its tokens and
/// queryables.
///
/// The caller has checked the address ([`crate::tape::mock::check_address`])
/// and printed the plan. `on_up` is told the instance id once the service
/// is up, before the first scheduled sample.
///
/// **Nothing outlives this call** (#326, kept from v1): the entries run in a
/// `JoinSet` that is shut down, aborted *and* awaited, before it returns
/// for any reason, and dropping the future aborts every entry.
pub async fn run_gen(
    session: &Session,
    spec: &GenSpec,
    plan: &GenPlan,
    on_up: impl FnOnce(&str),
) -> Result<GenReport> {
    let synth = Synth::new(spec.seed);
    let held = capabilities(
        spec.revisions
            .iter()
            .flat_map(|r| r.contract().resources.iter()),
    );
    let mut b = zenkey::ServiceBuilder::new(
        session,
        config(
            &spec.address,
            held,
            &spec.bindings,
            marker(&spec.tool, Some(spec.seed)),
        ),
    );
    for rev in &spec.revisions {
        b.implement(implementation(rev)?)
            .map_err(|e| runtime_error(rev, e))?;
    }
    // redundancy.v1 (#841, RC): every interface whose contract uses the
    // profile, valid or not; the runtime refuses an invalid one at start.
    let governed: BTreeSet<IfaceId> = spec
        .revisions
        .iter()
        .filter(|r| {
            zenkey_model::redundancy::declaration_of(r.contract(), false)
                != zenkey_model::redundancy::Declaration::NotGoverned
        })
        .map(|r| r.iface().clone())
        .collect();
    let mut takeover = GenTakeover {
        seed: spec.seed,
        states: Vec::new(),
    };
    let calls = Arc::new(AtomicU64::new(0));
    let mut servers = Vec::new();
    let mut outs: Vec<(usize, Out)> = Vec::new();
    // The puts made before start: each state's first value.
    let mut before = Tally::default();
    for (i, e) in plan.entries.iter().enumerate() {
        let (rev, r) = lookup(spec, e)?;
        let iface = rev.iface();
        if r.kind == Kind::Operation {
            let answer = answer_of(rev, r, &synth);
            servers.push(serve_answer(&mut b, rev, r, answer, Arc::clone(&calls), None).await?);
            continue;
        }
        if e.rate_hz.is_none() {
            // Unsynthesizable: exposed, so the service starts, and silent.
            b.expose(iface, &e.resource)
                .map_err(|err| runtime_error(rev, err))?;
            continue;
        }
        let out = match r.kind {
            Kind::State => {
                let w = Arc::new(
                    b.declare_state_writer(iface, &e.resource, &e.values)
                        .await
                        .map_err(|err| runtime_error(rev, err))?,
                );
                if governed.contains(iface) {
                    // A governed state's first value is put at the
                    // takeover, before the interface's token (§2.6 step 3).
                    takeover.states.push((
                        iface.clone(),
                        e.key.clone(),
                        Arc::clone(&w),
                        Arc::clone(rev),
                        r.clone(),
                    ));
                } else {
                    // A state value held at start goes out before the
                    // tokens (§8.2), so a GET made when they appear finds
                    // it.
                    match synth.sample_at(
                        rev,
                        r,
                        Member::Type,
                        0,
                        Some(zenkey::telemetry::now_ms()),
                    ) {
                        Ok(s) => before.put(&e.key, w.put(s.bytes).await.map(|_| ())),
                        Err(why) => before.fail(format!("{}: {why}", e.key)),
                    }
                }
                Out::State(w)
            }
            Kind::Event => Out::Event(
                b.event_writer(iface, &e.resource, &e.values)
                    .map_err(|err| runtime_error(rev, err))?,
            ),
            _ => Out::Stream(Box::new(
                b.declare_writer(iface, &e.resource, &e.values)
                    .await
                    .map_err(|err| runtime_error(rev, err))?,
            )),
        };
        outs.push((i, out));
    }
    let red = match (&spec.elect, governed.is_empty()) {
        (_, true) => None,
        (None, false) => None,
        (Some(startup), false) => Some(
            b.redundancy(startup.clone().with_takeover(Arc::new(takeover)))
                .map_err(|e| runtime_error(&spec.revisions[0], e))?,
        ),
    };
    let mut service = b
        .start()
        .await
        .map_err(|e| runtime_error(&spec.revisions[0], e))?;
    // A member exists from its first declaration (§8.1): one token per
    // value of the template that declares `epoch`. A governed interface's
    // only while the instance serves it, declared again at each takeover
    // (redundancy.v1 §2.6 step 4, §2.12).
    let mut members = BTreeSet::new();
    let mut governed_members: BTreeMap<IfaceId, Vec<String>> = BTreeMap::new();
    for e in plan.entries.iter().filter(|e| e.member_token) {
        let (rev, r) = lookup(spec, e)?;
        let Some(epoch) = &r.epoch else { continue };
        let Some(value) = e.values.get(epoch).and_then(|v| v.first()) else {
            continue;
        };
        if members.insert((rev.iface().clone(), value.clone())) {
            if governed.contains(rev.iface()) {
                governed_members
                    .entry(rev.iface().clone())
                    .or_default()
                    .push(value.clone());
                continue;
            }
            service
                .declare_member(rev.iface(), value)
                .await
                .map_err(|err| runtime_error(rev, err))?;
        }
    }
    on_up(&service.instance().to_string());

    let deadline = tokio::time::Instant::now() + spec.duration;
    let mut tasks: tokio::task::JoinSet<(usize, Tally)> = tokio::task::JoinSet::new();
    for (i, out) in outs {
        let e = plan.entries[i].clone();
        let (rev, r) = lookup(spec, &e)?;
        let (rev, r) = (Arc::clone(rev), r.clone());
        let (pattern, seed, total) = (spec.pattern, spec.seed, spec.duration.as_secs_f64());
        // An event member's own window on its declared rate (#744).
        let mut pace = match (&out, zenkey_model::budget::rate_of(&r)) {
            (Out::Event(_), Some(rate)) => Some(Pace::new(rate)),
            _ => None,
        };
        tasks.spawn(async move {
            let mut tally = Tally::default();
            let rate = e.rate_hz.unwrap_or(1.0);
            let base = Duration::from_secs_f64(1.0 / rate);
            let started = tokio::time::Instant::now();
            let run_over = tokio::time::sleep_until(deadline);
            tokio::pin!(run_over);
            // A state's tick 0 went out before start: its first re-put
            // waits one interval.
            let state = matches!(out, Out::State(_));
            let mut tick = u64::from(state);
            if state {
                tokio::select! {
                    () = tokio::time::sleep(base) => {}
                    () = &mut run_over => return (i, tally),
                }
            }
            let epoch_attached = zenkey_model::telemetry::declaration_of(rev.contract(), &r)
                .declared()
                .is_some_and(|d| d.epoch == Some(zenkey_model::telemetry::EpochIn::Attachment));
            loop {
                if e.events_cap.is_some_and(|cap| tally.sent >= cap) {
                    (&mut run_over).await;
                    break;
                }
                // An event waits for its rate's window, whatever the
                // pattern asked: the runtime would refuse it sooner.
                if let Some(pace) = &mut pace {
                    let ready = pace.ready_at(Instant::now());
                    if ready > Instant::now() {
                        tokio::select! {
                            () = tokio::time::sleep_until(ready.into()) => {}
                            () = &mut run_over => break,
                        }
                    }
                }
                // A Point's timestamp is when its value is taken (§2.12),
                // so a rate timed by the timestamps is the one published at.
                let now = Some(zenkey::telemetry::now_ms());
                match synth.sample_at(&rev, &r, Member::Type, tick, now) {
                    Ok(s) => {
                        let put = match &out {
                            Out::State(w) => w.put(s.bytes).await.map(|_| ()),
                            Out::Event(w) => w.put(s.bytes).await.map(|_| ()),
                            // telemetry.v1 §2.9: an Epoch attachment is the
                            // runtime's writer's to write (#819, TB).
                            Out::Stream(w) if epoch_attached => w.put(s.bytes).await,
                            Out::Stream(w) => {
                                match synth.sample(&rev, &r, Member::Attachment, tick) {
                                    Ok(a) => w.put_with(s.bytes, Some(a.bytes)).await,
                                    Err(_) => w.put(s.bytes).await,
                                }
                            }
                        };
                        if let (Ok(()), Some(pace)) = (&put, &mut pace) {
                            pace.record(Instant::now());
                        }
                        tally.put(&e.key, put);
                    }
                    Err(why) => tally.fail(format!("{}: {why}", e.key)),
                }
                tick += 1;
                let wait = interval(
                    pattern,
                    base,
                    rate,
                    tick,
                    seed ^ i as u64,
                    started.elapsed().as_secs_f64(),
                    total,
                );
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = &mut run_over => break,
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            (i, tally)
        });
    }
    // Joined in completion order, reported in plan order; meanwhile a
    // governed interface's members follow its standing (§2.6 step 4,
    // §2.12): each change its watch tells (`Redundancy::subscribe`, never
    // a poll of the role) declares, while it serves, each member it does
    // not hold, and a declaration that failed for another reason than its
    // standing is made again after `MEMBER_RETRY`. A member held is never
    // declared again: the runtime would cycle it, a new epoch.
    let mut done: BTreeMap<usize, Tally> = BTreeMap::new();
    let mut fatal = None;
    let (moved_tx, mut moved) = tokio::sync::mpsc::unbounded_channel::<IfaceId>();
    let mut watches: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();
    if let Some(red) = &red {
        for iface in governed_members.keys() {
            let mut rx = red.subscribe(iface);
            let (tx, iface) = (moved_tx.clone(), iface.clone());
            watches.spawn(async move {
                // Its standing now, then every change.
                if tx.send(iface.clone()).is_err() {
                    return;
                }
                while rx.changed().await.is_ok() {
                    if tx.send(iface.clone()).is_err() {
                        return;
                    }
                }
            });
        }
    }
    drop(moved_tx);
    // Interfaces serving with a member not yet declared.
    let mut owed: BTreeSet<IfaceId> = BTreeSet::new();
    let mut retry = tokio::time::interval(MEMBER_RETRY);
    retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            joined = tasks.join_next() => match joined {
                None => break,
                Some(Ok((i, t))) => {
                    done.insert(i, t);
                }
                Some(Err(e)) => {
                    fatal = Some(Error::Internal(format!("a gen task did not join: {e}")));
                    break;
                }
            },
            Some(iface) = moved.recv() => {
                if let (Some(red), Some(values)) = (&red, governed_members.get(&iface)) {
                    if declare_members(&mut service, red, &iface, values).await {
                        owed.remove(&iface);
                    } else {
                        owed.insert(iface);
                    }
                }
            }
            _ = retry.tick(), if !owed.is_empty() => {
                let Some(red) = &red else { continue };
                for iface in std::mem::take(&mut owed) {
                    if let Some(values) = governed_members.get(&iface)
                        && !declare_members(&mut service, red, &iface, values).await
                    {
                        owed.insert(iface);
                    }
                }
            }
        }
    }
    watches.shutdown().await;
    tasks.shutdown().await;
    let instance = service.instance().to_string();
    let roles: BTreeMap<String, String> = match &red {
        Some(red) => governed
            .iter()
            .map(|i| (i.to_string(), red.role(i).as_str().to_owned()))
            .collect(),
        None => BTreeMap::new(),
    };
    // Down: the operation servers, then the tokens and queryables.
    for s in servers {
        if let Err(e) = s.undeclare().await {
            tracing::warn!("a mock operation did not undeclare: {e}");
        }
    }
    let closed = service.close().await;
    if let Some(e) = fatal {
        return Err(e);
    }
    closed.map_err(|e| Error::bus("undeclare", "the mock owner", e))?;
    let Tally {
        mut sent,
        mut failed,
        mut refused,
        mut standing,
        errors: mut first_errors,
    } = before;
    for t in done.into_values() {
        sent += t.sent;
        failed += t.failed;
        refused += t.refused;
        standing += t.standing;
        for e in t.errors {
            if first_errors.len() < 5 {
                first_errors.push(e);
            }
        }
    }
    Ok(GenReport {
        address: spec.address.to_string(),
        instance,
        duration_s: spec.duration.as_secs_f64(),
        entries: plan.entries.len(),
        sent,
        calls: calls.load(Ordering::SeqCst),
        failed,
        refused,
        standing_by: standing,
        roles,
        first_errors,
    })
}

/// One governed state a takeover puts: its interface, its key, its
/// writer, and what its value is synthesized from.
type TakeoverState = (
    IfaceId,
    String,
    Arc<zenkey::state::StateWriter>,
    Arc<Revision>,
    Resource,
);

/// How long a member declaration that failed waits to be made again,
/// while its interface serves.
const MEMBER_RETRY: Duration = Duration::from_millis(500);

/// Declares, while `iface` serves, each of `values`' member tokens the
/// instance does not hold (`redundancy.v1` §2.6 step 4, §2.12). `true` when
/// nothing is owed: every member held, or the interface not served (a
/// step-down withdrew them; the next takeover's change declares them
/// again). `false` when a declaration failed for another reason, to be
/// made again: the members declared before it stay, never declared twice,
/// since [`zenkey::Service::declare_member`] of a member held cycles it to
/// a new epoch.
///
/// **Bounded, inside the run's `select!` arm** (review on PR #867): each
/// `declare_member` is a local liveliness declaration (and, cycling, an
/// undeclaration) whose `.await` returns once zenoh has queued it; no
/// reply is awaited. At worst, its control priority's queue full, it waits
/// for room, which the link's `wait_before_close` bounds by closing it.
/// The run's entries go on meanwhile, on their own tasks.
async fn declare_members(
    service: &mut zenkey::Service,
    red: &zenkey::redundancy::Redundancy,
    iface: &IfaceId,
    values: &[String],
) -> bool {
    for v in values {
        if red.role(iface) != zenkey::redundancy::Role::Serving {
            return true;
        }
        if service.holds_member(iface, v) {
            continue;
        }
        match service.declare_member(iface, v).await {
            Ok(_) => {}
            // Stepped down meanwhile: the change that follows says so.
            Err(zenkey::Error::Standby(_)) => return true,
            Err(e) => {
                tracing::warn!("{iface}'s member {v:?} was not declared, made again: {e}");
                return false;
            }
        }
    }
    true
}

/// What a takeover puts (`redundancy.v1` §2.6 step 3): each governed
/// state's value at tick 0, synthesized when it takes over.
struct GenTakeover {
    seed: u64,
    states: Vec<TakeoverState>,
}

impl zenkey::redundancy::Takeover for GenTakeover {
    fn taking_over<'a>(&'a self, iface: &'a IfaceId) -> zenkey::call::BoxFuture<'a, ()> {
        Box::pin(async move {
            let synth = Synth::new(self.seed);
            for (i, key, w, rev, r) in &self.states {
                if i != iface {
                    continue;
                }
                match synth.sample_at(rev, r, Member::Type, 0, Some(zenkey::telemetry::now_ms())) {
                    Ok(s) => {
                        if let Err(e) = w.put(s.bytes).await {
                            tracing::warn!("{key}: a takeover's put failed: {e}");
                        }
                    }
                    Err(why) => tracing::warn!("{key}: no value to put at the takeover: {why}"),
                }
            }
        })
    }
}

/// A low-discrepancy pseudo-random in [0,1): deterministic, no RNG.
fn halton(n: u64) -> f64 {
    let mut f = 1.0;
    let mut r = 0.0;
    let mut i = n.wrapping_mul(2_654_435_761) % 4096 + 1;
    while i > 0 {
        f /= 2.0;
        r += f * (i % 2) as f64;
        i /= 2;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::catalog::ContractSet;

    fn examples(dir: &str) -> ContractSet {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2")
            .join(dir);
        ContractSet::load_path(&root).0
    }

    fn revision(set: &ContractSet, iface: &str) -> Arc<Revision> {
        let iface: IfaceId = iface.parse().unwrap();
        Arc::clone(set.of_iface(&iface).next().expect("in the examples"))
    }

    fn spec(members: Vec<MemberArg>) -> GenSpec {
        let set = examples("tcgui");
        GenSpec {
            address: "host-a/tc".parse().unwrap(),
            revisions: vec![revision(&set, "tc.netif.v1"), revision(&set, "tc.netem.v1")],
            members,
            bindings: BTreeMap::new(),
            rate_hz: None,
            pattern: GenPattern::Steady,
            duration: Duration::from_secs(10),
            seed: 42,
            tool: "zenctl gen".into(),
            elect: None,
        }
    }

    /// Every stream, state and event resource has an entry per member,
    /// every operation one; keys are the runtime's, QoS the contract's;
    /// defaults are stated.
    #[test]
    fn the_plan_covers_every_resource_with_the_contracts_qos() {
        let plan = build_plan(&spec(vec![])).unwrap();
        assert_eq!(plan.address, "host-a/tc");
        assert_eq!(plan.marker["synthetic"], true);
        let ns = plan
            .entries
            .iter()
            .find(|e| e.resource == "state/namespaces")
            .unwrap();
        assert_eq!(ns.key, "zk2/host-a/tc/tc.netif.v1/state/namespaces");
        assert_eq!(ns.members, MemberSource::Fixed);
        assert_eq!(ns.rate_hz, Some(0.5));
        let q = ns.qos.unwrap();
        assert_eq!(
            (q.reliability, q.congestion),
            (
                zenkey_model::authoring::Reliability::Reliable,
                zenkey_model::authoring::Congestion::Block
            ),
            "state's defaults (§2.4)"
        );
        let bw: Vec<&GenPlanEntry> = plan
            .entries
            .iter()
            .filter(|e| e.resource == "stream/bandwidth/{ns}/{iface}")
            .collect();
        assert_eq!(bw.len(), DEFAULT_MEMBERS);
        assert_eq!(
            bw[0].key,
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/ns-1/iface-1"
        );
        assert_eq!(bw[0].members, MemberSource::Default);
        assert!(bw[0].note.as_deref().unwrap().contains("--member"));
        let diag = plan
            .entries
            .iter()
            .find(|e| e.resource == "@op/diagnostics")
            .unwrap();
        assert_eq!(diag.kind, Kind::Operation);
        assert_eq!(diag.qos, None);
        assert_eq!(diag.rate_hz, None);
        assert_eq!(diag.declared, "json:DiagnosticsResponse");
        let set = plan
            .entries
            .iter()
            .find(|e| e.resource == "@op/interfaces/{ns}/{iface}/set")
            .unwrap();
        assert_eq!(set.key, "zk2/host-a/tc/tc.netif.v1/@op/interfaces/*/*/set");
        let events: Vec<&GenPlanEntry> = plan
            .entries
            .iter()
            .filter(|e| e.kind == Kind::Event)
            .collect();
        assert!(!events.is_empty(), "tc.netem.v1 declares events");
        for e in events {
            let cap = e.events_cap.expect("an event is capped");
            assert!(e.rate_hz.unwrap() <= 1.0 && cap >= 1, "{e:?}");
        }
    }

    /// `--member` names the members, in template order; one that does not
    /// fit, names nothing, or names an untemplated resource or an
    /// operation is refused.
    #[test]
    fn members_are_given_or_refused() {
        let arg = MemberArg::parse("bandwidth/{ns}/{iface}=default/eth0,lab/eth1").unwrap();
        let plan = build_plan(&spec(vec![arg])).unwrap();
        let keys: Vec<&str> = plan
            .entries
            .iter()
            .filter(|e| e.resource == "stream/bandwidth/{ns}/{iface}")
            .map(|e| e.key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0",
                "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/lab/eth1"
            ]
        );
        for bad in [
            "bandwidth/{ns}/{iface}=eth0",
            "namespaces=x",
            "nope/{x}=a",
            "interfaces/{ns}/{iface}/set=a/b",
        ] {
            let e = build_plan(&spec(vec![MemberArg::parse(bad).unwrap()])).unwrap_err();
            assert!(e.is_unaskable(), "{bad}: {e}");
        }
        assert!(MemberArg::parse("no-equals").is_err());
        assert!(MemberArg::parse("x=").is_err());
    }

    /// A required role unbound is refused before anything is declared
    /// (R1), and a binding for a role nobody declares too.
    #[test]
    fn a_required_role_must_be_bound() {
        let set = examples("walkthrough");
        let mut s = spec(vec![]);
        s.revisions = vec![revision(&set, "detections.v1")];
        let e = build_plan(&s).unwrap_err();
        assert!(e.to_string().contains("--bind input="), "{e}");
        s.bindings.insert("input".into(), vec!["v1/cam".into()]);
        assert!(build_plan(&s).is_ok());
        s.bindings.insert("nobody".into(), vec!["v1/cam".into()]);
        assert!(
            build_plan(&s)
                .unwrap_err()
                .to_string()
                .contains("no implemented contract")
        );
    }

    /// #744: an event keeps its declared rate whatever `--rate` asks, and
    /// its entry says so once, naming the rate; a stream and a state take
    /// `--rate` as given. Without `--rate`, or with one within the rate,
    /// nothing is held back and nothing is said.
    #[test]
    fn an_event_keeps_its_declared_rate_and_the_plan_says_so() {
        let netem = |rate_hz| {
            let mut s = spec(vec![]);
            s.revisions = vec![revision(&examples("tcgui"), "tc.netem.v1")];
            s.rate_hz = rate_hz;
            s.duration = Duration::from_secs(300);
            build_plan(&s).unwrap()
        };
        let applied = |plan: &GenPlan| -> GenPlanEntry {
            plan.entries
                .iter()
                .find(|e| e.resource == "events/applied")
                .cloned()
                .expect("tc.netem.v1 declares events/applied, rate low")
        };

        let fast = netem(Some(10.0));
        let e = applied(&fast);
        assert_eq!(e.rate_hz, Some(1.0 / 60.0), "one a minute: low (§2.7)");
        assert_eq!(e.events_cap, Some(5), "five minutes of low");
        assert_eq!(
            e.note.as_deref(),
            Some(
                "--rate 10 Hz is above its declared rate low (at most 1 per minute): it keeps \
                 its declared rate (core §2.7)"
            )
        );
        for other in fast.entries.iter().filter(|e| e.kind != Kind::Event) {
            if other.kind != Kind::Operation {
                assert_eq!(other.rate_hz, Some(10.0), "{other:?}");
            }
            assert!(
                !other
                    .note
                    .as_deref()
                    .unwrap_or("")
                    .contains("declared rate"),
                "{other:?}"
            );
        }

        // No --rate: the plan an event had before #744, and no note.
        let e = applied(&netem(None));
        assert_eq!(e.rate_hz, Some(5.0 / 300.0));
        assert_eq!(e.note, None);
        // A --rate within the declared rate is taken as given.
        let e = applied(&netem(Some(0.01)));
        assert_eq!(e.rate_hz, Some(0.01));
        assert_eq!(e.note, None);
    }

    /// Runs one event entry's schedule on a simulated clock, the way
    /// `run_gen`'s loop does, against the runtime's own budget for the
    /// resource (`zenkey::budget::Budget`, core §2.7): how many occurrences
    /// it admitted, and how many it refused. `paced` is the loop's
    /// [`Pace`]; without it, the schedule `--rate` and the pattern make.
    fn simulate(
        rev: &Revision,
        resource: &str,
        pattern: GenPattern,
        rate_hz: f64,
        run: Duration,
        paced: bool,
    ) -> (u64, u64) {
        let r = rev
            .contract()
            .resources
            .iter()
            .find(|r| zenkey::implementation::resource_name(r) == resource)
            .expect("the resource");
        let rate = zenkey_model::budget::rate_of(r).expect("an event with a rate");
        let budget = zenkey::budget::Budget::new(rev.iface(), r).expect("a data resource");
        let cap = events_cap(rate, run);
        let mut pace = paced.then(|| Pace::new(rate));
        let base = Duration::from_secs_f64(1.0 / rate_hz);
        let start = Instant::now();
        let deadline = start + run;
        let (mut now, mut tick, mut admitted, mut refused) = (start, 0u64, 0u64, 0u64);
        while admitted < cap {
            if let Some(pace) = &mut pace {
                now = now.max(pace.ready_at(now));
            }
            if now >= deadline {
                break;
            }
            match budget.admit("member", now) {
                Ok(()) => {
                    admitted += 1;
                    if let Some(pace) = &mut pace {
                        pace.record(now);
                    }
                }
                Err(_) => refused += 1,
            }
            tick += 1;
            let elapsed = (now - start).as_secs_f64();
            now += interval(pattern, base, rate_hz, tick, 42, elapsed, run.as_secs_f64());
        }
        (admitted, refused)
    }

    /// #744: whatever the pattern and however fast `--rate`, no occurrence
    /// `gen` attempts is one the runtime's budget refuses, and every one
    /// the cap allows is published. The unpaced schedule is what the
    /// runtime refused before: the test has teeth.
    #[test]
    fn pacing_keeps_every_pattern_within_the_runtimes_budget() {
        let netem = revision(&examples("tcgui"), "tc.netem.v1");
        let snmp = revision(&examples("zensight"), "zs.snmp.v1");
        let modem = revision(&examples("zenoh-modem"), "modem.v3");
        let cases = [
            (&netem, "events/applied", 300),       // low: 1 a minute
            (&snmp, "events/traps", 600),          // burst(1000/h)
            (&modem, "events/oper_change", 7_200), // rare: 1 an hour
        ];
        let patterns = [
            GenPattern::Steady,
            GenPattern::Jitter,
            GenPattern::Burst,
            GenPattern::Ramp,
        ];
        for (rev, resource, secs) in cases {
            let run = Duration::from_secs(secs);
            let r = rev
                .contract()
                .resources
                .iter()
                .find(|r| zenkey::implementation::resource_name(r) == resource)
                .unwrap_or_else(|| panic!("{resource} in {}", rev.iface()));
            let cap = events_cap(zenkey_model::budget::rate_of(r).expect("a rate"), run);
            for pattern in patterns {
                for hz in [10.0, 1000.0] {
                    let (admitted, refused) = simulate(rev, resource, pattern, hz, run, true);
                    assert_eq!(
                        (admitted, refused),
                        (cap, 0),
                        "{resource} {pattern:?} at {hz} Hz over {secs} s"
                    );
                }
            }
        }
        let (_, refused) = simulate(
            &netem,
            "events/applied",
            GenPattern::Steady,
            10.0,
            Duration::from_secs(300),
            false,
        );
        assert!(
            refused > 0,
            "the runtime refuses an unpaced 10 Hz low event"
        );
        // Without --rate, the plan's own rate: a steady run never needed
        // the pace, and is what it was; `--pattern burst` did, and was
        // refused too.
        let run = Duration::from_secs(300);
        let planned = 5.0 / 300.0;
        let steady = simulate(
            &netem,
            "events/applied",
            GenPattern::Steady,
            planned,
            run,
            false,
        );
        assert_eq!(steady, (5, 0));
        let (_, refused) = simulate(
            &netem,
            "events/applied",
            GenPattern::Burst,
            planned,
            run,
            false,
        );
        assert!(refused > 0, "an unpaced burst at the planned rate");
    }

    #[test]
    fn an_events_cap_follows_its_rate_over_the_run() {
        let ten = Duration::from_secs(10);
        assert_eq!(events_cap(Rate::Rare, ten), 1);
        assert_eq!(events_cap(Rate::Low, Duration::from_secs(120)), 2);
        assert_eq!(events_cap(Rate::Burst(3600), ten), 10);
        assert_eq!(events_cap(Rate::Burst(2), Duration::from_secs(36_000)), 2);
    }
}
