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
//! The plan ([`build_plan`]) is session-free and printed before anything
//! is brought up, the replay dry-run precedent. It replaced v1's
//! registry-driven generator: the registry walk, the schema ladder through
//! a served `describe`, the impersonated `--origin`, `--serve-describe` and
//! the fault injector went with it (the `v1` branch keeps them).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
    /// within its declared rate whatever this says.
    pub rate_hz: Option<f64>,
    pub pattern: GenPattern,
    pub duration: Duration,
    /// Drives synthesis and jitter: the same seed is the same run.
    pub seed: u64,
    /// Stamped into the descriptor's marker.
    pub tool: String,
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
            let rate_hz = match (r.kind, cap) {
                (Kind::Event, Some(cap)) => spec
                    .rate_hz
                    .unwrap_or((cap as f64 / secs).min(1.0))
                    .clamp(0.001, 1000.0),
                (Kind::State, _) => spec.rate_hz.unwrap_or(0.5).clamp(0.001, 1000.0),
                _ => spec.rate_hz.unwrap_or(1.0).clamp(0.001, 1000.0),
            };
            let made = synth.sample(rev, r, Member::Type, 0).and_then(|_| {
                if d.attachment.is_some() {
                    synth.sample(rev, r, Member::Attachment, 0).map(|_| ())
                } else {
                    Ok(())
                }
            });
            let mut notes = Vec::new();
            if let Err(e) = &made {
                notes.push(format!("cannot synthesize ({e}): publishes nothing"));
            }
            if d.attachment.is_some() {
                notes.push(if r.kind == Kind::Stream {
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
    Stream(zenkey::writer::Writer),
    State(zenkey::state::StateWriter),
    Event(zenkey::writer::EventWriter),
}

/// What one entry's task did.
#[derive(Default)]
struct Tally {
    sent: u64,
    failed: u64,
    errors: Vec<String>,
}

impl Tally {
    fn fail(&mut self, e: String) {
        self.failed += 1;
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
    let calls = Arc::new(AtomicU64::new(0));
    let mut servers = Vec::new();
    let mut outs: Vec<(usize, Out)> = Vec::new();
    let mut first_errors = Vec::new();
    let mut failed = 0u64;
    let mut sent = 0u64;
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
                let w = b
                    .declare_state_writer(iface, &e.resource, &e.values)
                    .await
                    .map_err(|err| runtime_error(rev, err))?;
                // A state value held at start goes out before the tokens
                // (§8.2), so a GET made when they appear finds it.
                let first = synth.sample(rev, r, Member::Type, 0);
                match first {
                    Ok(s) => match w.put(s.bytes).await {
                        Ok(_) => sent += 1,
                        Err(err) => {
                            failed += 1;
                            first_errors.push(format!("{}: {err}", e.key));
                        }
                    },
                    Err(why) => {
                        failed += 1;
                        first_errors.push(format!("{}: {why}", e.key));
                    }
                }
                Out::State(w)
            }
            Kind::Event => Out::Event(
                b.event_writer(iface, &e.resource, &e.values)
                    .map_err(|err| runtime_error(rev, err))?,
            ),
            _ => Out::Stream(
                b.declare_writer(iface, &e.resource, &e.values)
                    .await
                    .map_err(|err| runtime_error(rev, err))?,
            ),
        };
        outs.push((i, out));
    }
    let mut service = b
        .start()
        .await
        .map_err(|e| runtime_error(&spec.revisions[0], e))?;
    // A member exists from its first declaration (§8.1): one token per
    // value of the template that declares `epoch`.
    let mut members = BTreeSet::new();
    for e in plan.entries.iter().filter(|e| e.member_token) {
        let (rev, r) = lookup(spec, e)?;
        let Some(epoch) = &r.epoch else { continue };
        let Some(value) = e.values.get(epoch).and_then(|v| v.first()) else {
            continue;
        };
        if members.insert((rev.iface().clone(), value.clone())) {
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
            loop {
                if e.events_cap.is_some_and(|cap| tally.sent >= cap) {
                    (&mut run_over).await;
                    break;
                }
                match synth.sample(&rev, &r, Member::Type, tick) {
                    Ok(s) => {
                        let put = match &out {
                            Out::State(w) => w.put(s.bytes).await.map(|_| ()),
                            Out::Event(w) => w.put(s.bytes).await.map(|_| ()),
                            Out::Stream(w) => {
                                match synth.sample(&rev, &r, Member::Attachment, tick) {
                                    Ok(a) => w.put_with(s.bytes, Some(a.bytes)).await,
                                    Err(_) => w.put(s.bytes).await,
                                }
                            }
                        };
                        match put {
                            Ok(()) => tally.sent += 1,
                            Err(err) => tally.fail(format!("{}: {err}", e.key)),
                        }
                    }
                    Err(why) => tally.fail(format!("{}: {why}", e.key)),
                }
                tick += 1;
                let interval = match pattern {
                    GenPattern::Steady => base,
                    GenPattern::Jitter => base.mul_f64(0.7 + 0.6 * halton(seed ^ i as u64 ^ tick)),
                    GenPattern::Burst => {
                        if tick.is_multiple_of(rate.ceil().max(1.0) as u64) {
                            Duration::from_secs(1)
                        } else {
                            Duration::ZERO
                        }
                    }
                    GenPattern::Ramp => {
                        let progress = (started.elapsed().as_secs_f64() / total).clamp(0.05, 1.0);
                        base.div_f64(progress)
                    }
                };
                tokio::select! {
                    () = tokio::time::sleep(interval) => {}
                    () = &mut run_over => break,
                }
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
            }
            (i, tally)
        });
    }
    // Joined in completion order, reported in plan order.
    let mut done: BTreeMap<usize, Tally> = BTreeMap::new();
    let mut fatal = None;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok((i, t)) => {
                done.insert(i, t);
            }
            Err(e) => {
                fatal = Some(Error::Internal(format!("a gen task did not join: {e}")));
                break;
            }
        }
    }
    tasks.shutdown().await;
    let instance = service.instance().to_string();
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
    for t in done.into_values() {
        sent += t.sent;
        failed += t.failed;
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
        first_errors,
    })
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

    #[test]
    fn an_events_cap_follows_its_rate_over_the_run() {
        let ten = Duration::from_secs(10);
        assert_eq!(events_cap(Rate::Rare, ten), 1);
        assert_eq!(events_cap(Rate::Low, Duration::from_secs(120)), 2);
        assert_eq!(events_cap(Rate::Burst(3600), ten), 10);
        assert_eq!(events_cap(Rate::Burst(2), Duration::from_secs(36_000)), 2);
    }
}
