//! zk2's `check conform` (#703): one service against the contract revision
//! it claims, one verdict per case, decided from values.
//!
//! **Session-free.** [`judge`] reads a [`ConformObservation`] that
//! [`crate::bus::conform`] gathered. Each case asks whether the service
//! breaks one rule, so its finding is the yes (`docs/zk2/tooling-guide.md`
//! §1); see [`CaseId`] for the cases and the sections they read.
//!
//! **Silence is never a pass, and rarely a violation.** A resource nothing
//! was heard from in the window is unobservable, not unserved: a stream
//! with nothing to say, a state member not yet written, an event the
//! contract calls rare. The one silence that is a violation is an
//! operation's while its service holds its tokens: every call that reaches
//! an owner's queryable MUST get a value or an envelope (O3) — and even
//! then the finding names the access control that returns empty too (O5).
//!
//! **What a case judged is what arrived.** A subscription keeps at most
//! [`crate::bus::conform::SAMPLE_CAP`] samples per resource; the evidence
//! says how many it judged of how many delivered.
//!
//! **A population is bounded from below until a reading is complete**
//! (core §2.7, 0.24; #735). The `budget` case counts a templated state's
//! live members against its bound in this instance, from the owner's GET;
//! `budget-window` a templated stream's or event's, over the window; and
//! `rate` an event's occurrences per member — all through
//! `zenkey_model::budget`. More than the bound, from any reading, is the
//! finding. Within it is clean only after a complete reading: a GET that
//! ran to its final reply, or a window of a whole liveness span that lost
//! nothing while the owner held its instance token throughout
//! ([`crate::bus::conform::WindowPresence`]); otherwise unobservable.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use zenkey_model::authoring::Kind;
use zenkey_model::budget::{self, Bound, Occurrences, Reading, Window};
use zenkey_model::contract::{Body, Fanout, Resource};
use zenkey_model::freshness::{
    self as fresh, ClockMeasure, ClockTrust, Horizon, Judged, Observation, Reason, Reply, StampAge,
    Verdict,
};
use zenkey_model::schema::TypeId;

use crate::bus::conform::{Arrival, ConformObservation, FanoutSeen, Heard, OpObserved};
use crate::judge::common::s1_premise;
use crate::judge::doctor::AdminSpace;
use crate::model::catalog::{ContractState, Revision, zid_value};
use crate::model::lens::conformance;
use crate::report::{
    CaseId, ConformCase, ConformReport, Conformance, HealthAnswerToken, HealthVerdict,
    OperationAnswer, OperationReport, PayloadRendering, PresenceAttribution, Stamp, StateReport,
    StateValue, WatchEvent,
};

/// Every case `obs` supports, in [`CaseId::ALL`] order per resource.
pub fn judge(obs: &ConformObservation) -> ConformReport {
    let mut report = ConformReport {
        address: obs.address.to_string(),
        iface: obs.iface.to_string(),
        fingerprint: None,
        namespace: obs.namespace.clone(),
        window_s: obs.spec.window.as_secs_f64(),
        asked: Vec::new(),
        cases: Vec::new(),
        unobservable: None,
    };
    if let Ok(o) = &obs.presence {
        report.asked.push(o.selector.clone());
    }
    let whole = |mut report: ConformReport, why: String| {
        report.cases.push(ConformCase::unobservable(
            CaseId::ContractServed,
            obs.iface.to_string(),
            why.clone(),
        ));
        report.cases.extend(health_cases(obs));
        report.unobservable = Some(why);
        report
    };
    // The service, present and describing itself.
    match &obs.presence {
        Err(e) => return whole(report, format!("the presence read could not be made: {e}")),
        Ok(o) if o.tokens.is_empty() => {
            let why = if o.complete {
                format!(
                    "no token of {} visible to this reader (`{}`): the service is not running \
                     here, or access control refuses this reader its presence (0.8) — nothing \
                     to judge",
                    obs.address, o.selector
                )
            } else {
                format!(
                    "the read of `{}` ended at its timeout with no token: possibly incomplete",
                    o.selector
                )
            };
            return whole(report, why);
        }
        Ok(_) => {}
    }
    let fp = match obs.claimed() {
        Ok(fp) => fp,
        Err(why) => {
            return whole(
                report,
                format!(
                    "{why}: the suite tests a revision the service claims, and it claims none \
                     to test"
                ),
            );
        }
    };
    report.fingerprint = Some(fp.to_string());
    if let Some(p) = &obs.asked_fp
        && !fp.hex().to_string().starts_with(p.as_str())
    {
        return whole(
            report,
            format!(
                "{} claims {} at {fp}, not the revision asked ({p}): the suite tests the \
                 revision a service claims",
                obs.address, obs.iface
            ),
        );
    }
    report.cases.push(contract_served(obs, &fp.to_string()));
    let Some(rev) = &obs.revision else {
        report.unobservable = Some(format!(
            "{} {fp} is in hand neither from its holders nor from --contracts: there is nothing \
             to judge the resources against",
            obs.iface
        ));
        report.cases.extend(health_cases(obs));
        return report;
    };
    let owners: BTreeSet<String> = match &obs.presence {
        Ok(o) => o
            .descriptors
            .iter()
            .flatten()
            .filter_map(|(_, r)| r.descriptor())
            .filter_map(|d| d.meta.get("zid").and_then(|z| z.as_str()))
            .map(zid_value)
            .collect(),
        Err(_) => BTreeSet::new(),
    };
    let mut exposed = obs.exposed();
    exposed.sort_by_key(|r| zenkey::implementation::resource_name(r));
    let present = matches!(&obs.presence, Ok(o) if !o.tokens.is_empty());
    let clocks = clock_offsets(obs);
    for r in exposed {
        let name = zenkey::implementation::resource_name(r);
        if r.kind == Kind::Operation {
            match obs.calls.get(&name) {
                None | Some(OpObserved::NotCalled) => {
                    let why = "not idempotent: each call is a write, which this suite makes \
                               only under --i-know";
                    report
                        .cases
                        .push(ConformCase::not_asked(CaseId::Operation, &name, why));
                    if forbids_fanout(r) {
                        report.cases.push(ConformCase::not_asked(
                            CaseId::FanoutRefused,
                            &name,
                            why,
                        ));
                    }
                }
                Some(OpObserved::Called { call, fanout }) => {
                    if let Ok(c) = call {
                        report.asked.extend(c.selectors.iter().cloned());
                    }
                    report
                        .cases
                        .push(operation(obs, rev, &name, call.as_deref(), present));
                    if let Some(f) = fanout {
                        if let Ok(seen) = f {
                            report.asked.push(seen.selector.clone());
                        }
                        report.cases.push(fanout_refused(
                            &name,
                            f.as_ref(),
                            present,
                            obs.spec.calls_granted,
                        ));
                    }
                }
            }
            report.cases.extend(budget_case(obs, r, &name, None, None));
            continue;
        }
        let heard = obs.heard.get(&name);
        let get = obs.gets.get(&name);
        if let Some(Ok(h)) = heard {
            report.asked.extend(h.selectors.iter().cloned());
        }
        if let Some(Ok(g)) = get {
            report.asked.extend(g.selectors.iter().cloned());
        }
        report
            .cases
            .push(resource_served(obs, r, &name, heard, get));
        report.cases.push(payload_type(rev, r, &name, heard, get));
        report.cases.push(qos(&name, heard));
        if r.kind == Kind::State {
            report
                .cases
                .push(state_stamp(&name, &owners, obs.admin.as_ref(), heard, get));
            report.cases.push(state_get(&name, get));
        }
        report
            .cases
            .push(freshness(obs, r, &name, heard, get, &clocks));
        report.cases.extend(budget_case(obs, r, &name, heard, get));
        report
            .cases
            .extend(rate_case(Some(rev.contract()), r, &name, heard));
    }
    report.cases.extend(health_cases(obs));
    let mut seen = BTreeSet::new();
    report.asked.retain(|s| seen.insert(s.clone()));
    report
}

/// The value each instance exposing `name` states for its bound, or none:
/// one entry per descriptor that exposes it (§2.7, §3.3).
fn stated(obs: &ConformObservation, name: &str) -> Vec<Option<u64>> {
    let (Ok(o), Some(rev)) = (&obs.presence, &obs.revision) else {
        return Vec::new();
    };
    let want = obs.iface.to_string();
    o.descriptors
        .iter()
        .flatten()
        .filter_map(|(_, read)| read.descriptor())
        .filter(|d| {
            d.exposed(rev.contract()).is_some_and(|rs| {
                rs.iter()
                    .any(|r| zenkey::implementation::resource_name(r) == name)
            })
        })
        .map(|d| {
            d.interfaces
                .iter()
                .find(|e| e.iface == want)
                .and_then(|e| e.cardinality.get(name).copied())
        })
        .collect()
}

/// "its bound of 8 (the descriptor's)": the bound, and whose it is.
fn bound_words(r: &Resource, b: Bound) -> String {
    match (b, r.cardinality) {
        (Bound::Of(n), Some(c)) if n < c => {
            format!("its bound of {n} (its descriptor's; the contract's is {c})")
        }
        (Bound::Of(n), _) => format!("its bound of {n} (the contract's)"),
        (other, _) => format!("its bound, {other}"),
    }
}

/// A period as a reader says it.
fn period_words(d: std::time::Duration) -> String {
    match d.as_secs() {
        60 => "a minute".to_owned(),
        3600 => "an hour".to_owned(),
        s if s % 86_400 == 0 => format!("{} d", s / 86_400),
        s if s % 3600 == 0 => format!("{} h", s / 3600),
        s => format!("{s} s"),
    }
}

/// An event's member is its key without the ULID chunk (§2.6, §2.7); any
/// other key is its own member.
fn member_of(kind: Kind, key: &str) -> &str {
    match (kind, key.rsplit_once('/')) {
        (Kind::Event, Some((member, _ulid))) => member,
        _ => key,
    }
}

/// Whether `key` is a member of `r` (§2.2: match, then rank): a key a more
/// specific template of the same kind token wins is that template's, never
/// `r`'s, though `r`'s selector reaches it. Without a contract, every key
/// is taken as `r`'s.
fn resolves_to(
    contract: Option<&zenkey_model::contract::Contract>,
    r: &Resource,
    key: &str,
) -> bool {
    let Some(contract) = contract else {
        return true;
    };
    let Ok(zenkey_model::grammar::ZkKey::Data { kind, resource, .. }) =
        zenkey_model::grammar::parse(key)
    else {
        return false;
    };
    let candidates: Vec<&Resource> = contract
        .resources
        .iter()
        .filter(|x| x.token == kind)
        .collect();
    let refs: Vec<&str> = crate::model::lens::template_chunks(kind, &resource)
        .iter()
        .map(String::as_str)
        .collect();
    zenkey_model::template::resolve(candidates.iter().map(|x| &x.template), &refs).is_some_and(
        |(i, _)| candidates[i].token == r.token && candidates[i].template == r.template,
    )
}

/// Each member of `r` a window heard, with the instants of its puts on this
/// host's receive clock, which every member shares (§2.7).
fn members_heard(
    contract: Option<&zenkey_model::contract::Contract>,
    r: &Resource,
    h: &Heard,
) -> BTreeMap<String, Vec<std::time::Duration>> {
    let mut out: BTreeMap<String, Vec<std::time::Duration>> = BTreeMap::new();
    for (key, arrivals) in h
        .arrivals
        .iter()
        .filter(|(k, _)| resolves_to(contract, r, k))
    {
        out.entry(member_of(r.kind, key).to_owned())
            .or_default()
            .extend(arrivals.iter().map(|a| a.at));
    }
    out
}

/// One member's occurrences as instants (§2.7, "Spans"): between their
/// stamps when every one carries a stamp of one clock, else on this host's
/// receive clock, where transit can shorten a span.
fn occurrence_instants(arrivals: &[&Arrival]) -> Vec<std::time::Duration> {
    let stamped: Option<Vec<(String, SystemTime)>> = arrivals
        .iter()
        .map(|a| {
            let s = a.stamp.as_ref()?;
            Some((zid_value(&s.clock), stamp_time(s)?))
        })
        .collect();
    if let Some(stamped) = stamped
        && let Some((clock, _)) = stamped.first()
        && stamped.iter().all(|(c, _)| c == clock)
        && let Some(origin) = stamped.iter().map(|(_, t)| *t).min()
    {
        return stamped
            .iter()
            .map(|(_, t)| t.duration_since(origin).unwrap_or_default())
            .collect();
    }
    arrivals.iter().map(|a| a.at).collect()
}

/// §2.7: a templated resource's live members against its bound in this
/// instance — `budget`, from the owner's GET, for a state; `budget-window`,
/// from the window, for a stream or an event. `None` for a resource
/// without parameters, which has no bound to keep. An operation's row is a
/// `budget` not asked.
fn budget_case(
    obs: &ConformObservation,
    r: &Resource,
    name: &str,
    heard: Option<&Result<Heard, String>>,
    get: Option<&Result<StateReport, String>>,
) -> Option<ConformCase> {
    if !r.template.has_params() {
        return None;
    }
    let c = match r.kind {
        Kind::Stream | Kind::Event => CaseId::BudgetWindow,
        Kind::State | Kind::Operation => CaseId::Budget,
    };
    if r.kind == Kind::Operation {
        return Some(ConformCase::not_asked(
            c,
            name,
            "an operation's members are the values its callers name, and no reading of the \
             owner counts them (§2.7)",
        ));
    }
    let contract = obs.revision.as_deref().map(Revision::contract);
    let b = budget::bound_of(r, stated(obs, name));
    let of = bound_words(r, b);
    let retention = budget::retention_of(r);
    let span = budget::liveness(r.kind, retention);
    let live = match span {
        Some(p) if r.kind == Kind::Event => format!(
            "with an occurrence within its retention of {}",
            period_words(p)
        ),
        _ => "heard within one hour".to_owned(),
    };
    let present = match &obs.window_presence {
        Some(Ok(p)) => p.throughout(),
        _ => false,
    };
    let mut lost = 0u64;
    let mut w = obs.spec.window.as_secs_f64();
    let (reading, capped) = match r.kind {
        Kind::State => match get {
            Some(Ok(g)) => (
                Reading::Get {
                    members: rows(g).filter(|(k, _)| resolves_to(contract, r, k)).count() as u64,
                    complete: obs.gets_complete.get(name).copied().unwrap_or(false),
                },
                String::new(),
            ),
            Some(Err(e)) => {
                return Some(ConformCase::unobservable(
                    c,
                    name,
                    format!("the GET failed: {e}"),
                ));
            }
            None => return Some(ConformCase::unobservable(c, name, "the GET was not made")),
        },
        _ => match heard {
            Some(Ok(h)) => {
                lost = h.lagged + h.arrivals_capped;
                w = h.listened.as_secs_f64();
                (
                    Reading::Window(Window {
                        heard: members_heard(contract, r, h),
                        listened: h.listened,
                        lossless: lost == 0,
                        present,
                    }),
                    if h.arrivals_capped > 0 {
                        format!(
                            " ({} put(s) past the {} this run keeps not counted)",
                            h.arrivals_capped,
                            crate::bus::conform::ARRIVAL_CAP
                        )
                    } else {
                        String::new()
                    },
                )
            }
            Some(Err(e)) => {
                return Some(ConformCase::unobservable(
                    c,
                    name,
                    format!("the subscription failed: {e}"),
                ));
            }
            None => return Some(ConformCase::unobservable(c, name, "not subscribed")),
        },
    };
    let j = budget::population(r.kind, b, retention, &reading);
    let n = j.counted;
    let need = span.map_or(u64::MAX, |p| p.as_secs());
    Some(match (j.verdict, j.reason) {
        (budget::Verdict::NotAsked, budget::Reason::NoCeiling) => ConformCase::not_asked(
            c,
            name,
            "its bound is the no-ceiling 4294967295: no bound, never a population to budget \
             with (§2.2)",
        ),
        (budget::Verdict::NotAsked, why) => ConformCase::not_asked(c, name, why.as_str()),
        (budget::Verdict::Exceeds, _) => ConformCase::failed(
            c,
            name,
            match &reading {
                Reading::Get { complete, .. } => format!(
                    "{n} member(s) answered with a value by the owner's GET{}, above {of}: an \
                     owner MUST NOT hold more live members than its bound (§2.7)",
                    if *complete {
                        ""
                    } else {
                        ", which did not run to its final reply: a lower bound already above it"
                    }
                ),
                Reading::Window(_) => format!(
                    "{n} member(s) {live} in the {w:.1}s window, above {of}: an owner MUST NOT \
                     hold more live members than its bound, and a lower bound already exceeds \
                     it (§2.7){capped}"
                ),
            },
        ),
        (budget::Verdict::Within, _) => ConformCase::passed(
            c,
            name,
            match &reading {
                Reading::Get { .. } => format!(
                    "{n} member(s) answered with a value by the owner's GET, which ran to its \
                     final reply: within {of} (§2.7)"
                ),
                Reading::Window(_) => format!(
                    "{n} member(s) {live} over a {w:.1}s window of at least one liveness span, \
                     nothing lost, the owner present throughout: within {of} (§2.7)"
                ),
            },
        ),
        (budget::Verdict::Unobservable, budget::Reason::Empty) => ConformCase::unobservable(
            c,
            name,
            match &reading {
                Reading::Get { .. } => "the owner's GET answered no member with a value: an \
                                        empty reply set is never a verdict (O5)"
                    .to_owned(),
                Reading::Window(_) => {
                    format!("no member heard in the {w:.1}s window: nothing to count (O5)")
                }
            },
        ),
        (budget::Verdict::Unobservable, budget::Reason::Incomplete) => ConformCase::unobservable(
            c,
            name,
            format!(
                "{n} member(s) answered with a value, within {of}, by a GET that did not run to \
                 its final reply: a member it did not answer may be live (§2.7)"
            ),
        ),
        (budget::Verdict::Unobservable, budget::Reason::WindowTooShort) => {
            ConformCase::unobservable(
                c,
                name,
                format!(
                    "{n} member(s) {live} in the {w:.1}s window, within {of}: a window shows a \
                     population within its bound only after one liveness span, so a window of \
                     at least {need} s (`--for {need}`), or --skip budget-window (§2.7){capped}"
                ),
            )
        }
        (budget::Verdict::Unobservable, budget::Reason::Lossy) => ConformCase::unobservable(
            c,
            name,
            format!(
                "{n} member(s) {live} in the {w:.1}s window, within {of}, and this run lost \
                 {lost} delivery(ies): a member it did not hear may be live (§2.7){capped}"
            ),
        ),
        (budget::Verdict::Unobservable, budget::Reason::OwnerAbsent) => ConformCase::unobservable(
            c,
            name,
            format!(
                "{n} member(s) {live} in the {w:.1}s window, within {of}, and the owner was not \
                 present throughout: {} — a member published while it was away, or before this \
                 run was matched, may be live (§2.7)",
                match &obs.window_presence {
                    Some(Ok(p)) if !p.complete => {
                        "the read of its instance tokens ended at its timeout".to_owned()
                    }
                    Some(Ok(p)) if p.held.is_empty() => {
                        "no instance token of it was visible when the window opened".to_owned()
                    }
                    Some(Ok(p)) => format!(
                        "{} of its instance token(s) went while the window listened",
                        p.gone.len()
                    ),
                    Some(Err(e)) => format!("its presence could not be watched: {e}"),
                    None => "its presence was not watched".to_owned(),
                }
            ),
        ),
        (budget::Verdict::Unobservable, why) => ConformCase::unobservable(c, name, why.as_str()),
    })
}

/// §2.7: an event's occurrences per member, against its declared `rate`.
/// `None` for a resource that is not an event.
fn rate_case(
    contract: Option<&zenkey_model::contract::Contract>,
    r: &Resource,
    name: &str,
    heard: Option<&Result<Heard, String>>,
) -> Option<ConformCase> {
    const C: CaseId = CaseId::Rate;
    if r.kind != Kind::Event {
        return None;
    }
    let Some(rate) = budget::rate_of(r) else {
        return Some(ConformCase::not_asked(
            C,
            name,
            "it declares no rate (§2.6)",
        ));
    };
    let h = match heard {
        Some(Ok(h)) => h,
        Some(Err(e)) => {
            return Some(ConformCase::unobservable(
                C,
                name,
                format!("the subscription failed: {e}"),
            ));
        }
        None => return Some(ConformCase::unobservable(C, name, "not subscribed")),
    };
    let mut by_member: BTreeMap<&str, Vec<&Arrival>> = BTreeMap::new();
    for (key, arrivals) in h
        .arrivals
        .iter()
        .filter(|(k, _)| resolves_to(contract, r, k))
    {
        by_member
            .entry(member_of(Kind::Event, key))
            .or_default()
            .extend(arrivals.iter());
    }
    let members: BTreeMap<String, Vec<std::time::Duration>> = by_member
        .iter()
        .map(|(m, a)| ((*m).to_owned(), occurrence_instants(a)))
        .collect();
    let lost = h.lagged + h.arrivals_capped;
    let o = Occurrences {
        listened: h.listened,
        complete: lost == 0,
        members,
    };
    let (n, period) = rate.limit();
    let period = period_words(period);
    let w = h.listened.as_secs_f64();
    let j = budget::rate(Some(rate), &o);
    Some(match j.reason {
        budget::Reason::RateExceeded => {
            let (member, _) = o
                .members
                .iter()
                .find(|(m, ts)| {
                    let one = Occurrences {
                        members: BTreeMap::from([((*m).clone(), (*ts).clone())]),
                        ..o.clone()
                    };
                    budget::rate(Some(rate), &one).verdict == budget::Verdict::Exceeds
                })
                .expect("an exceeding member");
            ConformCase::failed(
                C,
                name,
                format!(
                    "{} occurrences of {member} within {period}, where its rate {rate} allows \
                     {n}: an owner MUST NOT publish more (§2.7)",
                    j.counted
                ),
            )
        }
        budget::Reason::RateKept => ConformCase::passed(
            C,
            name,
            format!(
                "{} member(s) over a {w:.1}s window, none with more than {n} occurrence(s) within \
                 {period} ({rate}, §2.7)",
                o.members.len()
            ),
        ),
        budget::Reason::Lossy => ConformCase::unobservable(
            C,
            name,
            format!(
                "this run lost {lost} delivery(ies) in the window: an occurrence it did not hear \
                 could exceed {rate}"
            ),
        ),
        budget::Reason::WindowTooShort => ConformCase::unobservable(
            C,
            name,
            format!(
                "the {w:.1}s window is shorter than {period}, {rate}'s period: no excess heard, \
                 and listening at least that long (--for) is what could show the rate kept \
                 (§2.7)"
            ),
        ),
        budget::Reason::Empty => ConformCase::unobservable(
            C,
            name,
            format!("no occurrence heard in the {w:.1}s window: nothing to count (O5)"),
        ),
        other => ConformCase::unobservable(C, name, other.as_str()),
    })
}

/// `health.v1` (#721, PF): §5's "is this service healthy?" (`health`) and
/// "does its status agree with its checks?" (`health-aggregation`), from
/// the reading taken over the suite's window, its subject the service.
/// Unhealthy or stale, and a break of §2.2 seen in both readings, are the
/// findings: `health.v1`'s own, about the service. A reply's stamp is aged
/// on the operator's word, or against a clock measured on a live put of the
/// same clock — the suite's subscriptions' puts included, since the owner
/// stamps every one with its session's clock. Not asked of a service whose
/// descriptor does not list `health.v1` (§2.7).
fn health_cases(obs: &ConformObservation) -> [ConformCase; 2] {
    const S: &str = "service";
    fn unseen(c: CaseId, why: String) -> ConformCase {
        ConformCase::unobservable(c, S, why)
    }
    fn not(c: CaseId, why: String) -> ConformCase {
        ConformCase::not_asked(c, S, why)
    }
    let both = |f: fn(CaseId, String) -> ConformCase, why: String| {
        [
            f(CaseId::Health, why.clone()),
            f(CaseId::HealthAggregation, why),
        ]
    };
    let Some(h) = &obs.health else {
        return match &obs.presence {
            Err(e) => both(unseen, format!("the presence read could not be made: {e}")),
            Ok(o) => match crate::model::health::presence_in(o, &obs.address) {
                zenkey_model::health::Presence::Absent => both(
                    not,
                    format!(
                        "no instance token of {} visible to this reader: absent, which is \
                         presence's word, not a level (health.v1 §2.1)",
                        obs.address
                    ),
                ),
                zenkey_model::health::Presence::Present(
                    zenkey_model::health::Listing::NotListed,
                ) => both(
                    not,
                    "its descriptor does not list health.v1, so health is not asked of it \
                     (health.v1 §2.7)"
                        .to_owned(),
                ),
                _ => both(
                    unseen,
                    "its presence or its descriptor could not be read: whether it implements \
                     health.v1 cannot be told, and its interface token says nothing (health.v1 \
                     §2.7)"
                        .to_owned(),
                ),
            },
        };
    };
    let mut h = h.clone();
    if let Some(Ok(w)) = &mut h.window {
        for (clock, m) in clock_offsets(obs) {
            w.clocks
                .entry(clock)
                .and_modify(|all| *all = all.merge(m))
                .or_insert(m);
        }
    }
    let report = crate::judge::health::judge(&h);
    let address = obs.address.to_string();
    let Some(row) = report.services.iter().find(|r| r.address == address) else {
        return both(unseen, "the health reading read no row of it".to_owned());
    };
    let hint = |reason: &str| {
        if reason == "clock_untrusted" {
            "; a reply is aged only against a clock trusted to the HLC delta, measured on a live \
             put of the same clock or on the operator's word (--clocks-synced, freshness.v1 §2.6)"
        } else {
            ""
        }
    };
    let level = row
        .level
        .map(|l| format!(", at {}", l.as_str()))
        .unwrap_or_default();
    let health = match row.verdict {
        HealthVerdict::Healthy => ConformCase::passed(
            CaseId::Health,
            S,
            format!("healthy ({}{level}): {}", row.reason, row.says),
        ),
        HealthVerdict::Unhealthy | HealthVerdict::Stale => ConformCase::failed(
            CaseId::Health,
            S,
            format!(
                "{} ({}{level}): {} — health.v1's finding about the service (§5); stale is never \
                 a level, and never down",
                row.verdict.as_str(),
                row.reason,
                row.says
            ),
        ),
        HealthVerdict::Unobservable => ConformCase::unobservable(
            CaseId::Health,
            S,
            format!("{} ({}){}", row.says, row.reason, hint(&row.reason)),
        ),
        HealthVerdict::NotAsked => {
            ConformCase::not_asked(CaseId::Health, S, format!("{} ({})", row.says, row.reason))
        }
    };
    let a = &row.agrees;
    let aggregation = match a.answer {
        HealthAnswerToken::Yes => ConformCase::passed(CaseId::HealthAggregation, S, a.says.clone()),
        HealthAnswerToken::No => ConformCase::failed(
            CaseId::HealthAggregation,
            S,
            format!("{} (health.v1 §2.2)", a.says),
        ),
        HealthAnswerToken::Unobservable => ConformCase::unobservable(
            CaseId::HealthAggregation,
            S,
            format!("{} ({}){}", a.says, a.reason, hint(&a.reason)),
        ),
        HealthAnswerToken::NotAsked => ConformCase::not_asked(
            CaseId::HealthAggregation,
            S,
            format!("{} ({})", a.says, a.reason),
        ),
    };
    [health, aggregation]
}

/// A stamp's time, as [`crate::bus::consume::stamp`] writes it (RFC 3339).
fn stamp_time(s: &Stamp) -> Option<SystemTime> {
    zenoh::time::NTP64::parse_rfc3339(&s.time)
        .ok()
        .map(|t| t.to_system_time())
}

/// The window's measurements of every stamping clock, across every
/// resource's subscription, as one reading's (`freshness.v1` §2.6,
/// ground 2, 0.2).
fn clock_offsets(obs: &ConformObservation) -> BTreeMap<String, ClockMeasure> {
    let mut out: BTreeMap<String, ClockMeasure> = BTreeMap::new();
    for h in obs.heard.values().flatten() {
        for (clock, m) in &h.clocks {
            out.entry(zid_value(clock))
                .and_modify(|all| *all = all.merge(*m))
                .or_insert(*m);
        }
    }
    out
}

/// `freshness.v1` (#720): every member of a stream or state resource that
/// declares a horizon, from the window's deliveries (§2.5) and the GET's
/// replies (§2.6), its observations combined (§2.7), all judged at the
/// window's end; the resource's verdict is §5's second question. A reply's
/// stamp is aged only against a clock trusted to the delta: on the
/// operator's word (`clocks_synced`), or measured on a live put of the
/// same clock in this window.
fn freshness(
    obs: &ConformObservation,
    r: &Resource,
    name: &str,
    heard: Option<&Result<Heard, String>>,
    get: Option<&Result<StateReport, String>>,
    clocks: &BTreeMap<String, ClockMeasure>,
) -> ConformCase {
    const C: CaseId = CaseId::Freshness;
    let h = fresh::horizon_of(r);
    match &h {
        Horizon::Undeclared => {
            return ConformCase::not_asked(
                C,
                name,
                "it declares no freshness.ttl_s, so its freshness is not asked (freshness.v1 \
                 §2.3)",
            );
        }
        Horizon::Ignored => {
            return ConformCase::not_asked(
                C,
                name,
                "an event's freshness.ttl_s is ignored: an occurrence is never confirmed again \
                 (freshness.v1 §2.2)",
            );
        }
        Horizon::Invalid(v) => {
            return ConformCase::unobservable(
                C,
                name,
                format!(
                    "its freshness.ttl_s, {v}, is not a horizon: a whole number of seconds from \
                     0 to 2^53-1 (freshness.v1 §2.1)"
                ),
            );
        }
        Horizon::Never | Horizon::Within(_) => {}
    }
    let delta = fresh::DEFAULT_DELTA;
    let trust = |clock: &str| {
        if obs.spec.clocks_synced {
            return ClockTrust::Trusted { delta };
        }
        clocks
            .get(&zid_value(clock))
            .map_or(ClockTrust::Untrusted, |m| m.trust(delta))
    };
    let mut members: BTreeMap<String, Vec<Observation>> = BTreeMap::new();
    let sub = match heard {
        Some(Ok(h)) => Some(h),
        _ => None,
    };
    if let Some(h) = sub {
        for (k, o) in &h.members {
            members.entry(k.clone()).or_default().push(*o);
        }
    }
    if let (Some(Ok(g)), Some(at)) = (get, obs.read_at) {
        for row in &g.rows {
            let reply = match &row.value {
                StateValue::Deleted => Reply::Delete,
                StateValue::Value { .. } => Reply::Put {
                    stamp_age: row
                        .timestamp
                        .as_ref()
                        .and_then(stamp_time)
                        .map(|t| StampAge::between(t, at)),
                },
            };
            let clock = row
                .timestamp
                .as_ref()
                .map_or(ClockTrust::Untrusted, |s| trust(&s.clock));
            members
                .entry(row.key.clone())
                .or_default()
                .push(Observation::Got {
                    reply: Some(reply),
                    clock,
                });
        }
        // Delivered in the window, and absent from the GET's answer.
        for (k, os) in &mut members {
            if !g.rows.iter().any(|row| &row.key == k) {
                os.push(Observation::Got {
                    reply: None,
                    clock: ClockTrust::Untrusted,
                });
            }
        }
    }
    // Answered by the GET, and never delivered while the window listened.
    if let Some(h) = sub {
        for (k, os) in &mut members {
            if !h.members.contains_key(k) {
                os.push(Observation::Subscribed {
                    last: None,
                    listened: h.listened,
                    complete: true,
                });
            }
        }
    }
    let judged: Vec<(String, Judged)> = members
        .iter()
        .map(|(k, os)| (k.clone(), fresh::judge_all(&h, os)))
        .collect();
    let with = |v: Verdict| -> Vec<&(String, Judged)> {
        judged.iter().filter(|(_, j)| j.verdict == v).collect()
    };
    let horizon = match &h {
        Horizon::Within(t) => format!("its horizon of {} s", t.as_secs()),
        _ => "ttl_s = 0".to_owned(),
    };
    let first = |of: &[&(String, Judged)]| -> String {
        let (k, j) = of[0];
        let more = if of.len() > 1 {
            format!(", and {} more", of.len() - 1)
        } else {
            String::new()
        };
        format!("{k}: {}{more}", j.reason.says())
    };
    let stale = with(Verdict::Stale);
    if !stale.is_empty() {
        return ConformCase::failed(
            C,
            name,
            format!(
                "{} of {} member(s) stale against {horizon}, as this run observed them: {} \
                 (freshness.v1 §2.5–§2.7) — a finding about the value, never the owner's \
                 presence",
                stale.len(),
                judged.len(),
                first(&stale)
            ),
        );
    }
    let unseen = with(Verdict::Unobservable);
    if !unseen.is_empty() {
        let hint = if unseen
            .iter()
            .any(|(_, j)| j.reason == Reason::ClockUntrusted)
        {
            "; a reply is aged only against a clock trusted to the HLC delta, measured on a live \
             put of the same clock or on the operator's word (--clocks-synced, freshness.v1 \
             §2.6)"
        } else {
            ""
        };
        return ConformCase::unobservable(
            C,
            name,
            format!(
                "{} of {} member(s) could not be aged against {horizon}: {}{hint}",
                unseen.len(),
                judged.len(),
                first(&unseen)
            ),
        );
    }
    let fresh_ones = with(Verdict::Fresh);
    if fresh_ones.is_empty() {
        return ConformCase::unobservable(
            C,
            name,
            if judged.is_empty() {
                "no member was delivered in the window or answered by the GET: nothing to age \
                 (freshness.v1 §5)"
                    .to_owned()
            } else {
                format!(
                    "every member read ({}) is deleted: no value to age",
                    judged.len()
                )
            },
        );
    }
    ConformCase::passed(
        C,
        name,
        format!(
            "{} member(s) fresh against {horizon}{}",
            fresh_ones.len(),
            match &h {
                Horizon::Never => ", never stale".to_owned(),
                _ => format!(
                    ", the last confirmation within it, by this run's receive clock or a reply's \
                     stamp against a clock trusted to {} ms",
                    delta.as_millis()
                ),
            }
        ),
    )
}

fn forbids_fanout(r: &Resource) -> bool {
    matches!(&r.body, Body::Operation(op) if op.fanout != Fanout::Allowed)
        && r.template.has_params()
}

/// §8.2, §8.4: the claimed bundle, as its holders served it.
fn contract_served(obs: &ConformObservation, fp: &str) -> ConformCase {
    let subject = format!("{} {fp}", obs.iface);
    match &obs.served {
        None => ConformCase::unobservable(CaseId::ContractServed, subject, "not retrieved"),
        Some(Err(e)) => ConformCase::unobservable(
            CaseId::ContractServed,
            subject,
            format!("the retrieval could not be put on the bus: {e}"),
        ),
        Some(Ok(ContractState::Held(_))) => ConformCase::passed(
            CaseId::ContractServed,
            subject,
            "a holder served the bundle the descriptor names, and it verified",
        ),
        Some(Ok(ContractState::Unavailable { refused })) => ConformCase::failed(
            CaseId::ContractServed,
            subject,
            format!(
                "no holder served a bundle that verified ({}): an owner holds the bundle of \
                 every interface it implements (§8.2), and a consumer never decodes with an \
                 unverified one{}",
                if refused.is_empty() {
                    "no reply".to_owned()
                } else {
                    format!("refused: {}", refused.join(", "))
                },
                if obs.revision.is_some() {
                    " — the suite judges the rest against --contracts"
                } else {
                    ""
                }
            ),
        ),
        Some(Ok(ContractState::Unreadable { reason })) => ConformCase::failed(
            CaseId::ContractServed,
            subject,
            format!("a bundle verified, and its contract does not read: {reason}"),
        ),
    }
}

/// The values a resource's subscription delivered.
fn puts(h: &Heard) -> impl Iterator<Item = (&str, &PayloadRendering, &Conformance)> {
    h.samples.iter().filter_map(|s| match &s.event {
        WatchEvent::Put {
            payload,
            conformance,
            ..
        } => Some((s.key.as_str(), &**payload, conformance)),
        WatchEvent::Delete => None,
    })
}

/// The values a state GET answered.
fn rows(g: &StateReport) -> impl Iterator<Item = (&str, &PayloadRendering)> {
    g.rows.iter().filter_map(|r| match &r.value {
        StateValue::Value { payload } => Some((r.key.as_str(), &**payload)),
        StateValue::Deleted => None,
    })
}

/// "judged N of M delivered", when the cap kept fewer than arrived.
fn capped(h: &Heard) -> String {
    if h.received as usize > h.samples.len() {
        format!(
            " (judged the first {} of {} delivered)",
            h.samples.len(),
            h.received
        )
    } else {
        String::new()
    }
}

/// §8.2: an exposed stream, state or event resource is served when the
/// window heard it or a state GET answered for it.
fn resource_served(
    obs: &ConformObservation,
    r: &Resource,
    name: &str,
    heard: Option<&Result<Heard, String>>,
    get: Option<&Result<StateReport, String>>,
) -> ConformCase {
    const C: CaseId = CaseId::ResourceServed;
    let w = obs.spec.window.as_secs_f64();
    let h = match heard {
        Some(Ok(h)) => h,
        Some(Err(e)) => {
            return ConformCase::unobservable(C, name, format!("the subscription failed: {e}"));
        }
        None => return ConformCase::unobservable(C, name, "not subscribed"),
    };
    let replies = match get {
        Some(Ok(g)) => g.rows.len(),
        _ => 0,
    };
    if h.received > 0 || replies > 0 {
        let mut what = Vec::new();
        if h.received > 0 {
            what.push(format!("{} sample(s) in the {w}s window", h.received));
        }
        if replies > 0 {
            what.push(format!("{replies} state repl(ies)"));
        }
        return ConformCase::passed(C, name, what.join(", "));
    }
    let rare = match (&r.kind, &r.body) {
        (Kind::Event, Body::Data(d)) => d
            .rate
            .map(|rate| format!(" (an event, its declared rate {rate})"))
            .unwrap_or_default(),
        _ => String::new(),
    };
    ConformCase::unobservable(
        C,
        name,
        format!(
            "nothing heard in the {w}s window{}{rare}: a resource with nothing to say is not \
             unserved, and silence is not a verdict — listen longer with --for",
            if r.kind == Kind::State {
                ", and the state GET drew no reply"
            } else {
                ""
            }
        ),
    )
}

/// §7.2, §7.3: every value, from the window and the GET, against its type.
fn payload_type(
    rev: &Revision,
    r: &Resource,
    name: &str,
    heard: Option<&Result<Heard, String>>,
    get: Option<&Result<StateReport, String>>,
) -> ConformCase {
    const C: CaseId = CaseId::PayloadType;
    if let Body::Data(d) = &r.body
        && let TypeId::Raw { media_type, .. } = &d.type_
    {
        return ConformCase::not_asked(
            C,
            name,
            format!("a raw type ({media_type}) declares no structure to check bytes against"),
        );
    }
    let mut judged: Vec<(String, Conformance)> = Vec::new();
    let mut note = String::new();
    if let Some(Ok(h)) = heard {
        judged.extend(puts(h).map(|(k, _, c)| (k.to_owned(), c.clone())));
        note = capped(h);
    }
    if let Some(Ok(g)) = get {
        judged.extend(rows(g).map(|(k, p)| (k.to_owned(), conformance(rev, p))));
    }
    let checked: Vec<&(String, Conformance)> = judged
        .iter()
        .filter(|(_, c)| !matches!(c, Conformance::NotChecked { .. }))
        .collect();
    if checked.is_empty() {
        let why = judged
            .iter()
            .find_map(|(_, c)| match c {
                Conformance::NotChecked { reason } => Some(reason.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "no value arrived to check".to_owned());
        return ConformCase::unobservable(C, name, why);
    }
    let bad: Vec<String> = checked
        .iter()
        .filter_map(|(k, c)| match c {
            Conformance::Invalid { violations } => Some(format!("{k}: {}", violations.join("; "))),
            Conformance::Undecodable { declared, reason } => {
                Some(format!("{k}: does not decode as {declared}: {reason}"))
            }
            _ => None,
        })
        .collect();
    if let Some(first) = bad.first() {
        return ConformCase::failed(
            C,
            name,
            format!(
                "{} of {} value(s) do not conform to the declared type{note}; the first, {first}",
                bad.len(),
                checked.len()
            ),
        );
    }
    ConformCase::passed(
        C,
        name,
        format!(
            "{} value(s) decode as the declared type and satisfy it{note}",
            checked.len()
        ),
    )
}

/// §2.4: every sample the window heard, against the declared QoS.
fn qos(name: &str, heard: Option<&Result<Heard, String>>) -> ConformCase {
    const C: CaseId = CaseId::Qos;
    let h = match heard {
        Some(Ok(h)) => h,
        Some(Err(e)) => {
            return ConformCase::unobservable(C, name, format!("the subscription failed: {e}"));
        }
        None => return ConformCase::unobservable(C, name, "not subscribed"),
    };
    if h.samples.is_empty() {
        return ConformCase::unobservable(
            C,
            name,
            "no sample arrived: a state GET's replies ride the caller's QoS, so only a sample \
             shows the owner's",
        );
    }
    let off: Vec<&crate::report::WatchSample> = h
        .samples
        .iter()
        .filter(|s| s.qos_mismatch.is_some())
        .collect();
    if let Some(first) = off.first() {
        let m = first.qos_mismatch.as_ref().expect("filtered");
        return ConformCase::failed(
            C,
            name,
            format!(
                "{} of {} sample(s) rode off the declared QoS{}; the first, {}: {} — an owner \
                 MUST publish with its contract's QoS",
                off.len(),
                h.samples.len(),
                capped(h),
                first.key,
                m.summary()
            ),
        );
    }
    ConformCase::passed(
        C,
        name,
        format!(
            "{} sample(s), every one on the declared priority, congestion control and \
             express{}",
            h.samples.len(),
            capped(h)
        ),
    )
}

/// S1: every mutation and reply carries the owner's own stamp.
fn state_stamp(
    name: &str,
    owners: &BTreeSet<String>,
    admin: Option<&Result<AdminSpace, String>>,
    heard: Option<&Result<Heard, String>>,
    get: Option<&Result<StateReport, String>>,
) -> ConformCase {
    const C: CaseId = CaseId::StateStamp;
    let mut stamps: Vec<(String, Option<String>)> = Vec::new();
    if let Some(Ok(h)) = heard {
        stamps.extend(
            h.samples
                .iter()
                .map(|s| (s.key.clone(), s.timestamp.as_ref().map(|t| t.clock.clone()))),
        );
    }
    if let Some(Ok(g)) = get {
        stamps.extend(
            g.rows
                .iter()
                .map(|r| (r.key.clone(), r.timestamp.as_ref().map(|t| t.clock.clone()))),
        );
    }
    if stamps.is_empty() {
        return ConformCase::unobservable(C, name, "no mutation and no reply arrived to attribute");
    }
    if owners.is_empty() {
        return ConformCase::unobservable(
            C,
            name,
            "no descriptor states its session's zid (`meta.zid`): whose clock is the owner's \
             cannot be told — unattributable, never foreign",
        );
    }
    let unstamped: Vec<&String> = stamps
        .iter()
        .filter(|(_, c)| c.is_none())
        .map(|(k, _)| k)
        .collect();
    let foreign: Vec<(String, String)> = stamps
        .iter()
        .filter_map(|(k, c)| {
            let c = c.as_ref()?;
            (!owners.contains(&zid_value(c))).then(|| (k.clone(), c.clone()))
        })
        .collect();
    if !unstamped.is_empty() || !foreign.is_empty() {
        let mut why = Vec::new();
        if let Some(k) = unstamped.first() {
            why.push(format!(
                "{} unstamped (e.g. {k}): every mutation MUST carry a timestamp the owner set",
                unstamped.len()
            ));
        }
        if let Some((k, c)) = foreign.first() {
            why.push(format!(
                "{} stamped by clock {c} (e.g. {k}), not the owner's session",
                foreign.len()
            ));
        }
        return ConformCase::failed(
            C,
            name,
            format!(
                "of {} mutation(s) and repl(ies), {}; the owner's session is {}",
                stamps.len(),
                why.join("; "),
                owners.iter().cloned().collect::<Vec<_>>().join(", ")
            ),
        );
    }
    // §4.2 "A tool's S1 check" (0.17): the owner's own stamp passes only
    // against the routers this run verified.
    if let Err(why) = s1_premise(admin, owners) {
        return ConformCase::unobservable(C, name, why);
    }
    ConformCase::passed(
        C,
        name,
        format!(
            "{} mutation(s) and repl(ies), every one stamped by the owner's own session, \
             compared by value, against {} verified router(s)",
            stamps.len(),
            admin
                .and_then(|a| a.as_ref().ok())
                .map_or(0, |a| a.routers.len())
        ),
    )
}

/// S2: the owner answers a GET over the resource, each reply stamped.
fn state_get(name: &str, get: Option<&Result<StateReport, String>>) -> ConformCase {
    const C: CaseId = CaseId::StateGet;
    let g = match get {
        Some(Ok(g)) => g,
        Some(Err(e)) => return ConformCase::unobservable(C, name, format!("the GET failed: {e}")),
        None => return ConformCase::unobservable(C, name, "not asked for"),
    };
    if g.rows.is_empty() {
        return ConformCase::unobservable(
            C,
            name,
            format!(
                "the GET (`{}`) drew no reply within {}s: a member the owner has not written is \
                 absent from its answer, which no reader can tell from a reply that has not \
                 crossed (§8.2) — silence is not a verdict",
                g.selectors.join("`, `"),
                g.timeout_s
            ),
        );
    }
    let bare: Vec<&str> = g
        .rows
        .iter()
        .filter(|r| r.timestamp.is_none())
        .map(|r| r.key.as_str())
        .collect();
    if let Some(first) = bare.first() {
        return ConformCase::failed(
            C,
            name,
            format!(
                "{} of {} repl(ies) carry no timestamp (e.g. {first}): a GET reply MUST carry \
                 the timestamp of the mutation it represents",
                bare.len(),
                g.rows.len()
            ),
        );
    }
    ConformCase::passed(
        C,
        name,
        format!(
            "{} repl(ies), each with its mutation's timestamp",
            g.rows.len()
        ),
    )
}

/// Why a present owner's silence is not the O3 finding without the
/// operator's word (§5.1, 0.17).
const UNGRANTED: &str = "an access-control refusal is silent too (O5), and no tool can observe \
                         its grants (§11.3); this is the O3 finding only when the operator says \
                         the grants let this tool call (§5.1)";

/// O1–O7: what one call drew.
fn operation(
    obs: &ConformObservation,
    rev: &Revision,
    name: &str,
    call: Result<&OperationReport, &String>,
    present: bool,
) -> ConformCase {
    const C: CaseId = CaseId::Operation;
    let report = match call {
        Ok(r) => r,
        Err(e) => {
            return ConformCase::unobservable(C, name, format!("the call could not be made: {e}"));
        }
    };
    let t = report.timeout_s;
    let response = |reply: &PayloadRendering| -> Option<String> {
        match conformance(rev, reply) {
            Conformance::Invalid { violations } => Some(format!(
                "a value reply does not satisfy the response type: {}",
                violations.join("; ")
            )),
            Conformance::Undecodable { declared, reason } => Some(format!(
                "a value reply does not decode as {declared}: {reason}"
            )),
            _ => None,
        }
    };
    let silent = |present_now: bool| {
        if present_now && !obs.spec.calls_granted {
            ConformCase::unobservable(
                C,
                name,
                format!(
                    "{} holds its tokens, and the call drew silence within {t}s: {UNGRANTED}",
                    obs.address
                ),
            )
        } else if present_now {
            ConformCase::failed(
                C,
                name,
                format!(
                    "{} holds its tokens, and the call drew neither a value nor an envelope \
                     within {t}s: every call that reaches an owner's queryable MUST get one or \
                     the other, never silence (O3) — unless access control refused it, which \
                     returns empty too (O5)",
                    obs.address
                ),
            )
        } else {
            ConformCase::unobservable(
                C,
                name,
                format!(
                    "silence within {t}s, and presence cannot attribute it: no token of {} \
                     visible to this reader",
                    obs.address
                ),
            )
        }
    };
    match &report.answer {
        OperationAnswer::Value { reply } => match response(reply) {
            Some(why) => ConformCase::failed(C, name, why),
            None => ConformCase::passed(
                C,
                name,
                format!("answered with a value on its own key, {}", reply.key),
            ),
        },
        OperationAnswer::Refused { envelope } => ConformCase::passed(
            C,
            name,
            format!(
                "answered with an envelope ({}{}): a refusal is an answer (O3)",
                envelope.code,
                envelope
                    .cause
                    .as_deref()
                    .map(|c| format!(", {c}"))
                    .unwrap_or_default()
            ),
        ),
        OperationAnswer::Malformed { malformed } => ConformCase::failed(
            C,
            name,
            format!(
                "a reply_err claims {} and is no envelope: {} (§5.2)",
                malformed.encoding, malformed.error
            ),
        ),
        OperationAnswer::Silent { silence } => silent(matches!(
            silence.presence,
            PresenceAttribution::Present | PresenceAttribution::InstanceOnly
        )),
        OperationAnswer::Replies { replies } => {
            if let Some(m) = replies.malformed.first() {
                return ConformCase::failed(
                    C,
                    name,
                    format!(
                        "a reply_err claims {} and is no envelope: {} (§5.2)",
                        m.encoding, m.error
                    ),
                );
            }
            if let Some(p) = replies
                .repliers
                .iter()
                .find(|r| r.possibly_partial == Some(true))
            {
                return ConformCase::failed(
                    C,
                    name,
                    format!(
                        "the replier on {} ended with {} summar(ies), not exactly one: possibly \
                         partial (O6)",
                        p.key,
                        p.summaries.len()
                    ),
                );
            }
            if let Some(why) = replies
                .repliers
                .iter()
                .flat_map(|r| r.replies.iter())
                .find_map(response)
            {
                return ConformCase::failed(C, name, why);
            }
            if replies.repliers.is_empty() && replies.refusals.is_empty() {
                return silent(present);
            }
            ConformCase::passed(
                C,
                name,
                format!(
                    "{} replier(s) and {} envelope(s){}",
                    replies.repliers.len(),
                    replies.refusals.len(),
                    if replies.summary_declared {
                        ", each replier ending with its one summary"
                    } else {
                        ""
                    }
                ),
            )
        }
    }
}

/// O2: a call over the template's wildcard, refused `fanout_forbidden`.
fn fanout_refused(
    name: &str,
    seen: Result<&FanoutSeen, &String>,
    present: bool,
    granted: bool,
) -> ConformCase {
    const C: CaseId = CaseId::FanoutRefused;
    let seen = match seen {
        Ok(s) => s,
        Err(e) => {
            return ConformCase::unobservable(C, name, format!("the call could not be made: {e}"));
        }
    };
    if seen.values > 0 {
        return ConformCase::failed(
            C,
            name,
            format!(
                "a call over `{}` ran: {} value repl(ies) to a fan-out the operation forbids — \
                 an owner MUST refuse it fanout_forbidden, whatever the access control allows",
                seen.selector, seen.values
            ),
        );
    }
    if let Some(m) = seen.malformed.first() {
        return ConformCase::failed(C, name, format!("a refusal is no envelope: {m} (§5.2)"));
    }
    let other: Vec<&String> = seen
        .codes
        .iter()
        .filter(|c| *c != "fanout_forbidden")
        .collect();
    if let Some(code) = other.first() {
        return ConformCase::failed(
            C,
            name,
            format!(
                "a call over `{}` was refused {code}, not fanout_forbidden: an owner refuses a \
                 call whose key is not concrete before anything else (§5.1, the order of \
                 refusals)",
                seen.selector
            ),
        );
    }
    if !seen.codes.is_empty() {
        return ConformCase::passed(
            C,
            name,
            format!(
                "a call over `{}` was refused fanout_forbidden, {} time(s)",
                seen.selector,
                seen.codes.len()
            ),
        );
    }
    if present && !granted {
        ConformCase::unobservable(
            C,
            name,
            format!(
                "a call over `{}` drew no refusal while the service holds its tokens: {UNGRANTED}",
                seen.selector
            ),
        )
    } else if present {
        ConformCase::failed(
            C,
            name,
            format!(
                "a call over `{}` drew no refusal: the service holds its tokens, and a call \
                 that reaches its queryable MUST be answered (O3) — unless access control \
                 refused it, which returns empty too (O5){}",
                seen.selector,
                if seen.transport.is_empty() {
                    String::new()
                } else {
                    format!("; the transport said: {}", seen.transport.join("; "))
                }
            ),
        )
    } else {
        ConformCase::unobservable(C, name, "no refusal, and no token visible to attribute it")
    }
}

#[cfg(test)]
mod tests {
    //! Each case's three poles from values: a conforming service, one with
    //! a wrong type, a QoS mismatch and a silent operation, and what could
    //! not be observed.

    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::bus::conform::{ConformSpec, WindowPresence};
    use crate::model::catalog::{DescriptorRead, Observed};
    use crate::model::render::{Member, render_with};
    use crate::report::{
        CallMode, Judgement, QosAxes, QosMismatch, ReplierView, RepliesView, SelectionPresence,
        SilenceView, Stamp, StateReading, StateRow, WatchSample, judgement_exit_code,
    };
    use serde_json::json;
    use zenkey_model::canonical::Fingerprint;
    use zenkey_model::contract::Contract;

    const INST: &str = "3fa9c2d41b7e0012";
    const OWNER: &str = "00ab12";

    /// `m.v1`: a stream and a state of JSON types, an idempotent
    /// operation, a templated one that forbids fan-out, a many-reply one
    /// with a summary.
    fn contract() -> Contract {
        let dir = std::env::temp_dir().join(format!("zenkey-fleet-conform-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("s.json"),
            r#"{"$defs": {
                "Bw": {"type": "object", "properties": {"rx": {"type": "integer"}}, "required": ["rx"]},
                "Status": {"type": "object", "properties": {"up": {"type": "boolean"}}, "required": ["up"]},
                "Req": {"type": "object"},
                "Resp": {"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]}
            }}"#,
        )
        .expect("write");
        let l = zenkey_model::contract::load_str(
            "[interface]\nname = \"m\"\nmajor = 1\nuses = [\"freshness.v1\"]\n\
             [schemas]\njsonschema = [\"s.json\"]\n\
             [resources.\"bandwidth/{dev}\"]\nkind = \"stream\"\ntype = \"json:Bw\"\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
             [resources.\"status/{dev}\"]\nkind = \"state\"\ntype = \"json:Status\"\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
             annotations = { \"freshness.ttl_s\" = 60 }\n\
             [resources.diag]\nkind = \"operation\"\nrequest = \"json:Req\"\nresponse = \"json:Resp\"\n\
             idempotent = true\n\
             [resources.\"set/{dev}\"]\nkind = \"operation\"\nrequest = \"json:Req\"\n\
             response = \"json:Resp\"\nparams = { dev = \"string\" }\ncardinality = 8\n\
             [resources.list]\nkind = \"operation\"\nrequest = \"json:Req\"\nresponse = \"json:Resp\"\n\
             idempotent = true\nreplies = \"many\"\nsummary = \"json:Resp\"\n",
            &dir,
            None,
        );
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// Built once: the schema file is written once, and every test reads
    /// the one revision (they run concurrently).
    fn revision() -> Arc<Revision> {
        static REV: std::sync::OnceLock<Arc<Revision>> = std::sync::OnceLock::new();
        Arc::clone(REV.get_or_init(|| {
            Arc::new(Revision::from_contract(
                contract(),
                crate::report::ContractSource::Bus,
            ))
        }))
    }

    fn fp() -> Fingerprint {
        revision().fingerprint().clone()
    }

    fn presence(zid: Option<&str>) -> Observed {
        let keys = vec![
            format!("zk2/lab/m/@zk/instance/{INST}"),
            format!("zk2/lab/m/@zk/alive/m.v1/{INST}/{}", fp().hex().fp16()),
        ];
        let mut o = Observed::from_keys("zk2/lab/m/@zk/**", &keys, true);
        let mut d = json!({
            "format": "zk2-descriptor/0.1",
            "service": "lab/m",
            "instance": INST,
            "interfaces": [{"iface": "m.v1", "contract": fp().to_string(), "minor": 0}],
        });
        if let Some(z) = zid {
            d["meta"] = json!({"zid": z});
        }
        o.descriptors = Some(BTreeMap::from([(
            ("lab/m".parse().unwrap(), INST.parse().unwrap()),
            DescriptorRead::Served(Box::new(serde_json::from_value(d).unwrap())),
        )]));
        o
    }

    fn stamp(clock: &str) -> Option<Stamp> {
        Some(Stamp {
            time: "2026-10-09T10:00:00.000000000Z".into(),
            clock: clock.into(),
        })
    }

    fn sample(key: &str, conformance: Conformance, qos: bool, clock: Option<&str>) -> WatchSample {
        let rev = revision();
        WatchSample {
            provider: "lab/m".into(),
            key: key.into(),
            values: BTreeMap::new(),
            timestamp: clock.and_then(stamp),
            qos_mismatch: qos.then(|| QosMismatch {
                declared: QosAxes {
                    priority: "data".into(),
                    congestion: "drop".into(),
                    express: false,
                },
                observed: QosAxes {
                    priority: "real_time".into(),
                    congestion: "drop".into(),
                    express: false,
                },
                differs: vec!["priority".into()],
            }),
            event: WatchEvent::Put {
                payload: Box::new(render_with(
                    &rev,
                    key,
                    Member::Type,
                    Some("application/json"),
                    b"{}",
                )),
                conformance,
                attachment: None,
            },
        }
    }

    /// How long the fixtures' window listened.
    const WINDOW: Duration = Duration::from_secs(2);

    /// The window's end on the wall clock: 2 s after the fixtures' stamp.
    fn read_at() -> SystemTime {
        stamp_time(&stamp("x").unwrap()).unwrap() + WINDOW
    }

    /// Each sample's member delivered 1 s before the window's end, and each
    /// stamp's clock measured 10 ms off this host's.
    fn heard(samples: Vec<WatchSample>) -> Result<Heard, String> {
        let members = samples
            .iter()
            .map(|s| {
                let o = Observation::Subscribed {
                    last: Some(fresh::Last::Put(Duration::from_secs(1))),
                    listened: WINDOW,
                    complete: true,
                };
                (s.key.clone(), o)
            })
            .collect();
        let clocks = samples
            .iter()
            .filter_map(|s| s.timestamp.as_ref())
            .map(|t| {
                (
                    t.clock.clone(),
                    ClockMeasure::new(StampAge::behind(Duration::from_millis(10))),
                )
            })
            .collect();
        let mut arrivals: BTreeMap<String, Vec<Arrival>> = BTreeMap::new();
        for s in &samples {
            arrivals.entry(s.key.clone()).or_default().push(Arrival {
                at: Duration::from_secs(1),
                stamp: s.timestamp.clone(),
            });
        }
        Ok(Heard {
            received: samples.len() as u64,
            samples,
            selectors: vec!["zk2/lab/m/m.v1/stream/bandwidth/*".into()],
            members,
            clocks,
            listened: WINDOW,
            arrivals,
            ..Heard::default()
        })
    }

    const STATUS: &str = "zk2/lab/m/m.v1/state/status/eth0";

    fn get(value: &[u8], clock: Option<&str>) -> Result<StateReport, String> {
        let rev = revision();
        Ok(StateReport {
            reading: StateReading::Current,
            address: "lab/m".into(),
            iface: "m.v1".into(),
            fingerprint: fp().to_string(),
            resource: "state/status/{dev}".into(),
            values: BTreeMap::new(),
            selectors: vec!["zk2/lab/m/m.v1/state/status/*".into()],
            archive: None,
            timeout_s: 1.0,
            rows: vec![StateRow {
                key: STATUS.into(),
                value: StateValue::Value {
                    payload: Box::new(render_with(
                        &rev,
                        STATUS,
                        Member::Type,
                        Some("application/json"),
                        value,
                    )),
                },
                timestamp: clock.and_then(stamp),
                confirmed: None,
                identity: None,
            }],
        })
    }

    fn report(op: &str, answer: OperationAnswer) -> OperationReport {
        OperationReport {
            address: "lab/m".into(),
            iface: "m.v1".into(),
            fingerprint: fp().to_string(),
            operation: op.into(),
            values: BTreeMap::new(),
            selectors: vec![format!("zk2/lab/m/m.v1/{op}")],
            mode: CallMode::Concrete,
            timeout_s: 1.0,
            answer,
        }
    }

    fn value(op: &str, body: &[u8]) -> OperationReport {
        let rev = revision();
        let key = format!("zk2/lab/m/m.v1/{op}");
        report(
            op,
            OperationAnswer::Value {
                reply: render_with(&rev, &key, Member::Response, Some("application/json"), body),
            },
        )
    }

    fn called(r: OperationReport) -> OpObserved {
        OpObserved::Called {
            call: Ok(Box::new(r)),
            fanout: None,
        }
    }

    fn summary_replies(summaries: usize) -> OperationReport {
        let rev = revision();
        let key = "zk2/lab/m/m.v1/@op/list";
        let r = |m| render_with(&rev, key, m, Some("application/json"), br#"{"ok":true}"#);
        report(
            "@op/list",
            OperationAnswer::Replies {
                replies: RepliesView {
                    summary_declared: true,
                    repliers: vec![ReplierView {
                        address: "lab/m".into(),
                        key: key.into(),
                        values: BTreeMap::new(),
                        replies: vec![r(Member::Response)],
                        summaries: (0..summaries).map(|_| r(Member::Summary)).collect(),
                        possibly_partial: Some(summaries != 1),
                    }],
                    refusals: vec![],
                    malformed: vec![],
                    transport: vec![],
                    discarded: 0,
                    presence: SelectionPresence {
                        selector: "zk2/lab/m/@zk/alive/m.v1/**".into(),
                        complete: true,
                        unheard: vec![],
                        error: None,
                    },
                },
            },
        )
    }

    /// A conforming service: every case asked passes.
    fn conforming() -> ConformObservation {
        let rev = revision();
        let mut o = ConformObservation {
            namespace: "acme".into(),
            address: "lab/m".parse().unwrap(),
            iface: "m.v1".parse().unwrap(),
            spec: ConformSpec {
                timeout: Duration::from_secs(1),
                window: Duration::from_secs(2),
                call_all: false,
                seed: 42,
                trust_admin: false,
                calls_granted: true,
                clocks_synced: false,
            },
            asked_fp: None,
            presence: Ok(presence(Some(OWNER))),
            served: Some(Ok(ContractState::Held(Arc::clone(&rev)))),
            revision: Some(rev),
            heard: BTreeMap::new(),
            gets: BTreeMap::new(),
            gets_complete: BTreeMap::from([("state/status/{dev}".to_owned(), true)]),
            window_presence: Some(Ok(WindowPresence {
                held: [format!("zk2/lab/m/@zk/instance/{INST}")].into(),
                complete: true,
                gone: BTreeSet::new(),
            })),
            calls: BTreeMap::new(),
            admin: Some(Ok(AdminSpace {
                routers: vec![crate::report::RouterInfo {
                    zid: "r1".into(),
                    version: None,
                    locators: vec![],
                    raw: json!({}),
                }],
                router_zids: ["r1".to_owned()].into(),
                ..Default::default()
            })),
            read_at: Some(read_at()),
            health: None,
        };
        o.heard.insert(
            "stream/bandwidth/{dev}".into(),
            heard(vec![sample(
                "zk2/lab/m/m.v1/stream/bandwidth/eth0",
                Conformance::Valid,
                false,
                None,
            )]),
        );
        o.heard.insert(
            "state/status/{dev}".into(),
            heard(vec![sample(
                STATUS,
                Conformance::Valid,
                false,
                Some("AB12"),
            )]),
        );
        o.gets.insert(
            "state/status/{dev}".into(),
            get(br#"{"up":true}"#, Some("ab12")),
        );
        o.calls.insert(
            "@op/diag".into(),
            called(value("@op/diag", br#"{"ok":true}"#)),
        );
        o.calls
            .insert("@op/list".into(), called(summary_replies(1)));
        o.calls
            .insert("@op/set/{dev}".into(), OpObserved::NotCalled);
        o
    }

    fn case<'r>(r: &'r ConformReport, id: CaseId, subject: &str) -> &'r ConformCase {
        r.cases
            .iter()
            .find(|c| c.case == id && c.subject == subject)
            .unwrap_or_else(|| panic!("{id} {subject}: {r:#?}"))
    }

    #[track_caller]
    fn failed(r: &ConformReport, id: CaseId, subject: &str) -> String {
        let c = case(r, id, subject);
        assert_eq!(c.verdict, Judgement::Established, "{c:#?}");
        c.detail.clone().expect("a finding says what broke")
    }

    #[test]
    fn a_conforming_service_passes_every_case_asked() {
        let mut r = judge(&conforming());
        // A 2 s window is short of a stream's hour (§2.7): its
        // `budget-window` is the one case left unobservable, and skipping
        // it is what leaves the run clean, the state's `budget` still asked.
        let bw = case(&r, CaseId::BudgetWindow, "stream/bandwidth/{dev}");
        assert!(
            matches!(&bw.verdict, Judgement::Unobservable { reason }
                if reason.contains("at least 3600 s (`--for 3600`)")),
            "{bw:#?}"
        );
        assert!(matches!(
            &r.judgement(),
            Judgement::Unobservable { reason }
                if reason.contains("1 case(s)") && reason.contains("budget-window stream/bandwidth/{dev}")
        ));
        r.skip(&[CaseId::BudgetWindow]);
        assert_eq!(judgement_exit_code(&r.judgement()), 0, "{r:#?}");
        assert!(matches!(
            case(&r, CaseId::Budget, "state/status/{dev}").verdict,
            Judgement::NotEstablished { .. }
        ));
        let r = judge(&conforming());
        assert_eq!(r.fingerprint, Some(fp().to_string()));
        for (id, subject) in [
            (CaseId::ContractServed, format!("m.v1 {}", fp())),
            (CaseId::ResourceServed, "stream/bandwidth/{dev}".to_owned()),
            (CaseId::PayloadType, "state/status/{dev}".to_owned()),
            (CaseId::Qos, "stream/bandwidth/{dev}".to_owned()),
            (CaseId::StateStamp, "state/status/{dev}".to_owned()),
            (CaseId::StateGet, "state/status/{dev}".to_owned()),
            (CaseId::Operation, "@op/diag".to_owned()),
            (CaseId::Operation, "@op/list".to_owned()),
            (CaseId::Freshness, "state/status/{dev}".to_owned()),
            (CaseId::Budget, "state/status/{dev}".to_owned()),
        ] {
            assert!(
                matches!(
                    case(&r, id, &subject).verdict,
                    Judgement::NotEstablished { .. }
                ),
                "{id} {subject}: {:#?}",
                case(&r, id, &subject)
            );
        }
        // Not idempotent, and no --i-know: not asked, with why; so is its
        // fan-out probe; a resource with no horizon is not asked its
        // freshness; and an operation's members are its callers' values.
        for (id, subject) in [
            (CaseId::Operation, "@op/set/{dev}"),
            (CaseId::FanoutRefused, "@op/set/{dev}"),
            (CaseId::Freshness, "stream/bandwidth/{dev}"),
            (CaseId::Budget, "@op/set/{dev}"),
        ] {
            let c = case(&r, id, subject);
            assert_eq!(c.verdict, Judgement::NotAsked, "{c:#?}");
            assert!(c.detail.is_some(), "not asked says why: {c:#?}");
        }
        assert!(
            r.asked.contains(&"zk2/lab/m/@zk/**".to_owned()),
            "{:?}",
            r.asked
        );
        // Untemplated resources have no bound; no event, no rate.
        assert!(
            !r.cases.iter().any(|c| c.case == CaseId::Budget
                && ["@op/diag", "@op/list"].contains(&c.subject.as_str()))
        );
        assert!(!r.cases.iter().any(|c| c.case == CaseId::Rate));
        // §4.2 (0.17): without a verified router, the owner's own stamp
        // proves nothing, and the run is no pass.
        let mut o = conforming();
        o.admin = Some(Err("refused".into()));
        let r = judge(&o);
        assert!(matches!(
            &case(&r, CaseId::StateStamp, "state/status/{dev}").verdict,
            Judgement::Unobservable { reason } if reason.contains("could not be read")
        ));
        assert_eq!(judgement_exit_code(&r.judgement()), 2);
    }

    /// A wrong type, a QoS mismatch, a silent operation while present: three
    /// findings, exit 1 — and a stamp of another clock, S1's.
    #[test]
    fn a_wrong_type_a_qos_mismatch_and_a_silent_operation_are_findings() {
        let mut o = conforming();
        o.heard.insert(
            "stream/bandwidth/{dev}".into(),
            heard(vec![
                sample(
                    "zk2/lab/m/m.v1/stream/bandwidth/eth0",
                    Conformance::Invalid {
                        violations: vec!["/rx: not an integer".into()],
                    },
                    false,
                    None,
                ),
                sample(
                    "zk2/lab/m/m.v1/stream/bandwidth/eth1",
                    Conformance::Valid,
                    true,
                    None,
                ),
            ]),
        );
        o.calls.insert(
            "@op/diag".into(),
            called(report(
                "@op/diag",
                OperationAnswer::Silent {
                    silence: SilenceView {
                        attempts: 1,
                        transport: Some("zenoh/string: Timeout".into()),
                        presence: PresenceAttribution::Present,
                    },
                },
            )),
        );
        o.gets.insert(
            "state/status/{dev}".into(),
            get(br#"{"up":true}"#, Some("ffff")),
        );
        let r = judge(&o);
        assert_eq!(judgement_exit_code(&r.judgement()), 1);
        let t = failed(&r, CaseId::PayloadType, "stream/bandwidth/{dev}");
        assert!(
            t.contains("1 of 2 value(s) do not conform") && t.contains("/rx"),
            "{t}"
        );
        let q = failed(&r, CaseId::Qos, "stream/bandwidth/{dev}");
        assert!(
            q.contains("1 of 2 sample(s) rode off the declared QoS"),
            "{q}"
        );
        assert!(
            q.contains("priority declared data, observed real_time"),
            "{q}"
        );
        let s = failed(&r, CaseId::Operation, "@op/diag");
        assert!(
            s.contains("never silence (O3)") && s.contains("(O5)"),
            "{s}"
        );
        let st = failed(&r, CaseId::StateStamp, "state/status/{dev}");
        assert!(st.contains("stamped by clock ffff"), "{st}");
        assert_eq!(r.failures().count(), 4);
        // §5.1 (0.17): without the operator's word on the grants, the
        // silence is unobservable, never the finding.
        o.spec.calls_granted = false;
        let r = judge(&o);
        assert!(matches!(
            &case(&r, CaseId::Operation, "@op/diag").verdict,
            Judgement::Unobservable { reason } if reason.contains("§11.3")
        ));
        assert_eq!(r.failures().count(), 3);
    }

    /// O6 and O2: a replier without exactly one summary, a fan-out that ran
    /// or was refused for another reason — each a finding; refused
    /// `fanout_forbidden`, a pass.
    #[test]
    fn a_partial_replier_and_a_fanout_not_refused_are_findings() {
        let mut o = conforming();
        o.calls
            .insert("@op/list".into(), called(summary_replies(0)));
        let fanout = |seen: FanoutSeen| OpObserved::Called {
            call: Ok(Box::new(value("@op/set/{dev}", br#"{"ok":true}"#))),
            fanout: Some(Ok(seen)),
        };
        let seen = |values: usize, codes: &[&str]| FanoutSeen {
            selector: "zk2/lab/m/m.v1/@op/set/*".into(),
            values,
            codes: codes.iter().map(|c| (*c).to_owned()).collect(),
            ..FanoutSeen::default()
        };
        o.calls.insert("@op/set/{dev}".into(), fanout(seen(2, &[])));
        let r = judge(&o);
        assert!(failed(&r, CaseId::Operation, "@op/list").contains("possibly partial (O6)"));
        assert!(failed(&r, CaseId::FanoutRefused, "@op/set/{dev}").contains("ran: 2 value"));
        // No refusal and no value from a present service: the O3 finding
        // only on the operator's word (§5.1, 0.17).
        o.calls.insert("@op/set/{dev}".into(), fanout(seen(0, &[])));
        let r = judge(&o);
        assert!(failed(&r, CaseId::FanoutRefused, "@op/set/{dev}").contains("drew no refusal"));
        o.spec.calls_granted = false;
        let r = judge(&o);
        assert!(matches!(
            &case(&r, CaseId::FanoutRefused, "@op/set/{dev}").verdict,
            Judgement::Unobservable { reason } if reason.contains("§11.3")
        ));
        o.spec.calls_granted = true;
        o.calls
            .insert("@op/set/{dev}".into(), fanout(seen(0, &["not_found"])));
        let r = judge(&o);
        assert!(failed(&r, CaseId::FanoutRefused, "@op/set/{dev}").contains("refused not_found"));
        o.calls.insert(
            "@op/set/{dev}".into(),
            fanout(seen(0, &["fanout_forbidden"])),
        );
        o.calls
            .insert("@op/list".into(), called(summary_replies(1)));
        let mut r = judge(&o);
        assert!(matches!(
            case(&r, CaseId::FanoutRefused, "@op/set/{dev}").verdict,
            Judgement::NotEstablished { .. }
        ));
        // The stream's population, short of an hour's window, skipped.
        r.skip(&[CaseId::BudgetWindow]);
        assert_eq!(judgement_exit_code(&r.judgement()), 0, "{r:#?}");
    }

    /// What could not be observed: a stream silent in the window, a state
    /// no `meta.zid` attributes, the service absent, a revision it does not
    /// claim — unobservable, exit 2, never a pass. A bundle no holder
    /// serves is a finding.
    #[test]
    fn silence_and_unattributable_stamps_are_unobservable() {
        let mut o = conforming();
        o.heard
            .insert("stream/bandwidth/{dev}".into(), heard(vec![]));
        o.presence = Ok(presence(None));
        let r = judge(&o);
        assert_eq!(judgement_exit_code(&r.judgement()), 2, "{r:#?}");
        assert!(
            case(&r, CaseId::ResourceServed, "stream/bandwidth/{dev}")
                .verdict
                .is_unobservable()
        );
        assert!(
            case(&r, CaseId::StateStamp, "state/status/{dev}")
                .verdict
                .is_unobservable()
        );

        let mut absent = conforming();
        absent.presence = Ok(Observed::from_keys("zk2/lab/m/@zk/**", &[], true));
        let r = judge(&absent);
        assert_eq!(judgement_exit_code(&r.judgement()), 2);
        assert!(
            r.unobservable
                .as_deref()
                .is_some_and(|u| u.contains("no token of lab/m visible to this reader")),
            "{r:#?}"
        );

        let mut other = conforming();
        other.asked_fp = Some("ffff".into());
        let r = judge(&other);
        assert!(
            r.unobservable
                .as_deref()
                .is_some_and(|u| u.contains("not the revision asked"))
        );

        let mut unserved = conforming();
        unserved.served = Some(Ok(ContractState::Unavailable { refused: vec![] }));
        let r = judge(&unserved);
        assert!(
            failed(&r, CaseId::ContractServed, &format!("m.v1 {}", fp()))
                .contains("no holder served a bundle that verified")
        );
    }

    /// `freshness.v1` (#720), per resource, judged at the window's end: a
    /// member not confirmed within its horizon is the finding; a reply's
    /// stamp against a clock nobody measured is unobservable, unless the
    /// operator gives their word; no member read is unobservable; no
    /// horizon is not asked (the conforming case above).
    #[test]
    fn freshness_is_judged_per_resource_at_the_windows_end() {
        const S: &str = "state/status/{dev}";
        let status = |last: Option<fresh::Last>, listened: Duration, measured: bool| {
            let mut h = heard(vec![sample(
                STATUS,
                Conformance::Valid,
                false,
                Some("ab12"),
            )])
            .unwrap();
            h.listened = listened;
            h.members.clear();
            if let Some(l) = last {
                h.members.insert(
                    STATUS.into(),
                    Observation::Subscribed {
                        last: Some(l),
                        listened,
                        complete: true,
                    },
                );
            }
            if !measured {
                h.clocks.clear();
            }
            Ok(h)
        };
        // The reply's stamp is 70 s old at the window's end.
        let late = read_at() + Duration::from_secs(68);

        // The last delivery 70 s old, the reply as old against a measured
        // clock: stale, the finding.
        let mut o = conforming();
        o.heard.insert(
            S.into(),
            status(
                Some(fresh::Last::Put(Duration::from_secs(70))),
                Duration::from_secs(80),
                true,
            ),
        );
        o.read_at = Some(late);
        let r = judge(&o);
        let d = failed(&r, CaseId::Freshness, S);
        assert!(
            d.contains("1 of 1 member(s) stale against its horizon of 60 s") && d.contains(STATUS),
            "{d}"
        );
        assert!(d.contains("never the owner's presence"), "{d}");
        assert_eq!(judgement_exit_code(&r.judgement()), 1);

        // Nothing delivered in a 2 s window, and no live put measured the
        // owner's clock: unobservable, naming the way out.
        let mut o = conforming();
        o.heard.insert(S.into(), status(None, WINDOW, false));
        o.read_at = Some(late);
        let r = judge(&o);
        assert!(
            matches!(
                &case(&r, CaseId::Freshness, S).verdict,
                Judgement::Unobservable { reason } if reason.contains("--clocks-synced")
            ),
            "{r:#?}"
        );
        assert_eq!(judgement_exit_code(&r.judgement()), 2);
        // On the operator's word, the same reply is aged: stale.
        o.spec.clocks_synced = true;
        let r = judge(&o);
        assert!(failed(&r, CaseId::Freshness, S).contains("not confirmed within its horizon"));

        // No member delivered, none answered: nothing to age.
        let mut o = conforming();
        o.heard.insert(S.into(), status(None, WINDOW, true));
        o.gets.insert(
            S.into(),
            Ok(StateReport {
                rows: vec![],
                ..get(b"{}", None).unwrap()
            }),
        );
        let r = judge(&o);
        assert!(
            matches!(
                &case(&r, CaseId::Freshness, S).verdict,
                Judgement::Unobservable { reason } if reason.contains("no member")
            ),
            "{r:#?}"
        );
    }

    /// health.v1 (#721, PF): a service whose descriptor does not list it is
    /// not asked; one that does is asked "is it healthy?" and "does its
    /// status agree with its checks?" from the reading over the window —
    /// FAILED and a break in both readings are findings, a fresh OK passes,
    /// and a reply aged against no trusted clock is unobservable.
    #[test]
    fn the_health_cases_read_health_v1_over_the_window() {
        use crate::bus::health::{
            CheckValue, HealthGet, HealthObservation, HealthSpec, HealthTarget, Replied,
            ServiceGot, Stamped, StatusValue,
        };
        use zenkey_model::health::{Level, Read};
        let r = judge(&conforming());
        for id in [CaseId::Health, CaseId::HealthAggregation] {
            let c = case(&r, id, "service");
            assert_eq!(c.verdict, Judgement::NotAsked, "{c:#?}");
            assert!(
                c.detail.as_deref().unwrap().contains("does not list"),
                "{c:#?}"
            );
        }
        let at = read_at();
        let got = |status: Level, check: Level| HealthGet {
            selector: "zk2/lab/m/health.v1/state/**".into(),
            read_at: at,
            complete: true,
            services: BTreeMap::from([(
                "lab/m".parse().unwrap(),
                ServiceGot {
                    status: Some(Replied {
                        value: Some(StatusValue {
                            level: Read::Level(status),
                            reason: "r".into(),
                            since_ns: 1,
                        }),
                        stamp: Some(Stamped {
                            time: at - Duration::from_secs(5),
                            clock: OWNER.into(),
                        }),
                    }),
                    checks: BTreeMap::from([(
                        "disk".to_owned(),
                        Replied {
                            value: Some(CheckValue {
                                level: Read::Level(check),
                                detail: String::new(),
                            }),
                            stamp: None,
                        },
                    )]),
                },
            )]),
        };
        let with = |first: HealthGet, second: HealthGet, synced: bool| {
            let mut o = conforming();
            let mut p = presence(Some(OWNER));
            if let Some(ds) = &mut p.descriptors {
                for read in ds.values_mut() {
                    if let DescriptorRead::Served(d) = read {
                        d.interfaces.push(
                            serde_json::from_value(json!({
                                "iface": "health.v1",
                                "contract": format!("sha256:{}", zenkey::health::FINGERPRINT),
                                "minor": 0, "token": false,
                            }))
                            .unwrap(),
                        );
                    }
                }
            }
            o.health = Some(HealthObservation {
                namespace: String::new(),
                target: HealthTarget::One("lab/m".parse().unwrap()),
                spec: HealthSpec {
                    timeout: WINDOW,
                    window: None,
                    grace: WINDOW,
                    clocks_synced: synced,
                    face: None,
                },
                presence: Some(Ok(p)),
                first: Ok(first),
                second: Ok(second),
                window: None,
                archived: None,
            });
            judge(&o)
        };
        let liar = || got(Level::Ok, Level::Failed);
        let r = with(liar(), liar(), true);
        assert!(failed(&r, CaseId::Health, "service").contains("inconsistent"));
        assert!(failed(&r, CaseId::HealthAggregation, "service").contains("§2.2"));
        assert_eq!(judgement_exit_code(&r.judgement()), 1);
        let ok = || got(Level::Ok, Level::Ok);
        let r = with(ok(), ok(), true);
        for id in [CaseId::Health, CaseId::HealthAggregation] {
            let c = case(&r, id, "service");
            assert!(
                matches!(c.verdict, Judgement::NotEstablished { .. }),
                "{c:#?}"
            );
        }
        // A break in one reading only: unobservable, never the finding.
        let r = with(ok(), liar(), true);
        assert!(matches!(
            &case(&r, CaseId::HealthAggregation, "service").verdict,
            Judgement::Unobservable { reason } if reason.contains("one_reading")
        ));
        // No word, and no window measured the owner's clock: unobservable.
        let r = with(ok(), ok(), false);
        assert!(matches!(
            &case(&r, CaseId::Health, "service").verdict,
            Judgement::Unobservable { reason } if reason.contains("--clocks-synced")
        ));
    }

    /// `budget` (core §2.7, 0.24; #735) per templated state resource, from
    /// the owner's GET against the bound its descriptor lowers: more members
    /// than the bound is the finding, complete or not; within it is clean
    /// only after a GET that ran to its final reply; an empty answer is
    /// unobservable (O5). A window over a stream is a lower bound: above
    /// the bound, the finding; within it, unobservable.
    #[test]
    fn the_budget_is_judged_per_templated_resource() {
        const S: &str = "state/status/{dev}";
        const BW: &str = "stream/bandwidth/{dev}";
        let lowered = |n: u64| {
            let mut p = presence(Some(OWNER));
            if let Some(d) = p.descriptors.as_mut().and_then(|m| m.values_mut().next()) {
                let DescriptorRead::Served(d) = d else {
                    unreachable!()
                };
                d.interfaces[0].cardinality.insert(S.to_owned(), n);
            }
            p
        };
        let two_members = || {
            let mut g = get(br#"{"up":true}"#, Some("ab12")).unwrap();
            let mut other = g.rows[0].clone();
            other.key = "zk2/lab/m/m.v1/state/status/eth1".into();
            g.rows.push(other);
            Ok(g)
        };
        // Two members against a descriptor's bound of 1: the finding.
        let mut o = conforming();
        o.presence = Ok(lowered(1));
        o.gets.insert(S.into(), two_members());
        let r = judge(&o);
        let msg = failed(&r, CaseId::Budget, S);
        assert!(
            msg.contains("2 member(s)") && msg.contains("its bound of 1 (its descriptor's"),
            "{msg}"
        );
        assert_eq!(judgement_exit_code(&r.judgement()), 1);
        // The same from a GET that did not run to its final reply: a lower
        // bound already above it.
        o.gets_complete.insert(S.into(), false);
        assert!(failed(&judge(&o), CaseId::Budget, S).contains("lower bound"));
        // Within the descriptor's bound of 2, complete: clean.
        let mut o = conforming();
        o.presence = Ok(lowered(2));
        o.gets.insert(S.into(), two_members());
        assert!(matches!(
            &case(&judge(&o), CaseId::Budget, S).verdict,
            Judgement::NotEstablished { reason } if reason.contains("within its bound of 2")
        ));
        // Within it, incomplete: unobservable.
        o.gets_complete.insert(S.into(), false);
        assert!(matches!(
            &case(&judge(&o), CaseId::Budget, S).verdict,
            Judgement::Unobservable { reason } if reason.contains("did not run to its final reply")
        ));
        // No member answered: never a verdict (O5).
        let mut o = conforming();
        if let Some(Ok(g)) = o.gets.get_mut(S) {
            g.rows.clear();
        }
        assert!(matches!(
            &case(&judge(&o), CaseId::Budget, S).verdict,
            Judgement::Unobservable { reason } if reason.contains("O5")
        ));
        // A bound above the contract's is D007's: the contract's (8) holds.
        let mut o = conforming();
        o.presence = Ok(lowered(64));
        o.gets.insert(S.into(), two_members());
        assert!(matches!(
            &case(&judge(&o), CaseId::Budget, S).verdict,
            Judgement::NotEstablished { reason } if reason.contains("its bound of 8 (the contract's)")
        ));
        // A stream heard with nine members in the window, above its 8.
        let mut o = conforming();
        let samples: Vec<WatchSample> = (0..9)
            .map(|i| {
                sample(
                    &format!("zk2/lab/m/m.v1/stream/bandwidth/eth{i}"),
                    Conformance::Valid,
                    false,
                    None,
                )
            })
            .collect();
        o.heard.insert(BW.into(), heard(samples));
        let msg = failed(&judge(&o), CaseId::BudgetWindow, BW);
        assert!(
            msg.contains("9 member(s) heard within one hour") && msg.contains("lower bound"),
            "{msg}"
        );
    }

    /// An event's contract: `budget` and `rate` (core §2.7) over a window.
    fn alarms() -> Arc<Revision> {
        let dir = std::env::temp_dir().join(format!("zenkey-fleet-budget-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let l = zenkey_model::contract::load_str(
            "[interface]\nname = \"al\"\nmajor = 1\n\
             [resources.\"alarms/{src}\"]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
             params = { src = \"string\" }\ncardinality = 2\nrate = \"low\"\nretention = \"1h\"\n\
             [resources.\"journal/{src}\"]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
             params = { src = \"string\" }\ncardinality = 4294967295\nrate = \"burst(10/h)\"\n\
             retention = \"1d\"\n",
            &dir,
            None,
        );
        Arc::new(Revision::from_contract(
            l.contract.unwrap_or_else(|| panic!("{}", l.report)),
            crate::report::ContractSource::Bus,
        ))
    }

    /// One occurrence of `src` arriving at `at` seconds, stamped by
    /// `clock` at `stamped` seconds past the fixtures' stamp when given.
    fn occurrence(src: &str, n: usize, at: f64, clock: Option<(&str, f64)>) -> (String, Arrival) {
        let base = stamp_time(&stamp("x").unwrap()).unwrap();
        let stamp = clock.map(|(c, stamped)| {
            let t = base + Duration::from_secs_f64(stamped);
            let since = t.duration_since(std::time::UNIX_EPOCH).unwrap();
            Stamp {
                time: format!("{:#}", zenoh::time::NTP64::from(since)),
                clock: c.into(),
            }
        });
        (
            format!("zk2/lab/al/al.v1/events/alarms/{src}/01hzzzzzzzzzzzzzzzzzzzzz{n:02}"),
            Arrival {
                at: Duration::from_secs_f64(at),
                stamp,
            },
        )
    }

    /// `rate` (core §2.7) per member over a window: two occurrences of one
    /// member less than a minute apart are the finding under `low`, in a
    /// window of seconds; members once each are no excess, and a window
    /// shorter than a minute cannot show the rate kept. A stamped span is
    /// measured between the stamps of one clock, not on arrival. `budget`
    /// on an event counts members within its retention; the no-ceiling
    /// cardinality is not asked.
    #[test]
    fn an_events_rate_and_population_are_judged_over_the_window() {
        let rev = alarms();
        let alarms_r = rev
            .contract()
            .resources
            .iter()
            .find(|r| r.template.as_str() == "alarms/{src}")
            .unwrap()
            .clone();
        let journal = rev
            .contract()
            .resources
            .iter()
            .find(|r| r.template.as_str() == "journal/{src}")
            .unwrap()
            .clone();
        let window = |of: Vec<(String, Arrival)>, listened: f64| -> Result<Heard, String> {
            let mut h = Heard {
                listened: Duration::from_secs_f64(listened),
                ..Heard::default()
            };
            for (k, a) in of {
                h.received += 1;
                h.arrivals.entry(k).or_default().push(a);
            }
            Ok(h)
        };
        let rate = |h: &Result<Heard, String>| {
            rate_case(
                Some(rev.contract()),
                &alarms_r,
                "events/alarms/{src}",
                Some(h),
            )
            .unwrap()
        };
        // Two of `a` 10 s apart, in a 20 s window: beyond `low`.
        let twice = window(
            vec![
                occurrence("a", 1, 1.0, None),
                occurrence("a", 2, 11.0, None),
            ],
            20.0,
        );
        let c = rate(&twice);
        assert_eq!(c.verdict, Judgement::Established, "{c:#?}");
        assert!(
            c.detail
                .as_deref()
                .unwrap()
                .contains("2 occurrences of zk2/lab/al/al.v1/events/alarms/a"),
            "{c:#?}"
        );
        // `a` and `b` once each: no excess, and 20 s is shorter than a minute.
        let once = window(
            vec![occurrence("a", 1, 1.0, None), occurrence("b", 2, 2.0, None)],
            20.0,
        );
        assert!(matches!(&rate(&once).verdict,
            Judgement::Unobservable { reason } if reason.contains("shorter than a minute")));
        // Over 61 s, nothing lost: kept.
        let long = window(
            vec![occurrence("a", 1, 1.0, None), occurrence("b", 2, 2.0, None)],
            61.0,
        );
        assert!(matches!(
            rate(&long).verdict,
            Judgement::NotEstablished { .. }
        ));
        // Lost deliveries leave it unobservable.
        let mut lossy = window(vec![occurrence("a", 1, 1.0, None)], 61.0);
        lossy.as_mut().unwrap().lagged = 3;
        assert!(matches!(&rate(&lossy).verdict,
            Judgement::Unobservable { reason } if reason.contains("lost 3")));
        // Arrivals 1 s apart, stamps of one clock (compared by value) 70 s
        // apart: the span is the stamps', and keeps `low`.
        let stamped = window(
            vec![
                occurrence("a", 1, 1.0, Some(("ab12", 1.0))),
                occurrence("a", 2, 2.0, Some(("AB12", 71.0))),
            ],
            61.0,
        );
        assert!(matches!(
            rate(&stamped).verdict,
            Judgement::NotEstablished { .. }
        ));
        // Stamps of two clocks: the receive clock's span, 1 s, beyond it.
        let mixed = window(
            vec![
                occurrence("a", 1, 1.0, Some(("ab12", 1.0))),
                occurrence("a", 2, 2.0, Some(("cd34", 71.0))),
            ],
            61.0,
        );
        assert_eq!(rate(&mixed).verdict, Judgement::Established);
        // Budget: three members within an hour's retention, above 2.
        let three = window(
            vec![
                occurrence("a", 1, 1.0, None),
                occurrence("b", 2, 2.0, None),
                occurrence("c", 3, 3.0, None),
            ],
            20.0,
        );
        let mut o = conforming();
        o.revision = Some(Arc::clone(&rev));
        o.presence = Ok(Observed::from_keys("zk2/lab/al/@zk/**", &[], true));
        let b = budget_case(&o, &alarms_r, "events/alarms/{src}", Some(&three), None).unwrap();
        assert_eq!(b.verdict, Judgement::Established, "{b:#?}");
        assert!(
            b.detail
                .as_deref()
                .unwrap()
                .contains("within its retention of an hour"),
            "{b:#?}"
        );
        assert_eq!(b.case, CaseId::BudgetWindow);
        // Two members within the bound, over 20 s of an hour's retention.
        let b = budget_case(&o, &alarms_r, "events/alarms/{src}", Some(&once), None).unwrap();
        assert!(
            matches!(&b.verdict, Judgement::Unobservable { reason }
                if reason.contains("at least 3600 s (`--for 3600`)")),
            "{b:#?}"
        );
        // Over the whole hour, nothing lost, the owner present: clean.
        let hour = window(
            vec![occurrence("a", 1, 1.0, None), occurrence("b", 2, 2.0, None)],
            3600.0,
        );
        o.window_presence = Some(Ok(WindowPresence {
            held: ["zk2/lab/al/@zk/instance/3fa9c2d41b7e0012".to_owned()].into(),
            complete: true,
            gone: BTreeSet::new(),
        }));
        let b = budget_case(&o, &alarms_r, "events/alarms/{src}", Some(&hour), None).unwrap();
        assert!(
            matches!(&b.verdict, Judgement::NotEstablished { reason }
                if reason.contains("the owner present throughout")),
            "{b:#?}"
        );
        // The same hour, a delivery lost: unobservable.
        let mut lossy_hour = hour.clone();
        lossy_hour.as_mut().unwrap().lagged = 1;
        let b = budget_case(
            &o,
            &alarms_r,
            "events/alarms/{src}",
            Some(&lossy_hour),
            None,
        )
        .unwrap();
        assert!(
            matches!(&b.verdict, Judgement::Unobservable { reason } if reason.contains("lost 1")),
            "{b:#?}"
        );
        // The owner's token went mid-window: unobservable, and said so.
        if let Some(Ok(p)) = &mut o.window_presence {
            p.gone = p.held.clone();
        }
        let b = budget_case(&o, &alarms_r, "events/alarms/{src}", Some(&hour), None).unwrap();
        assert!(
            matches!(&b.verdict, Judgement::Unobservable { reason }
                if reason.contains("not present throughout")
                    && reason.contains("went while the window listened")),
            "{b:#?}"
        );
        // The no-ceiling cardinality: not asked.
        let b = budget_case(&o, &journal, "events/journal/{src}", Some(&three), None).unwrap();
        assert_eq!(b.verdict, Judgement::NotAsked, "{b:#?}");
        assert!(b.detail.as_deref().unwrap().contains("no-ceiling"));
    }

    /// A key a more specific template wins is that template's member, never
    /// the wider one's, though its selector reaches it (§2.2): `items/total`
    /// is the literal resource's, so `items/{item}` counts one member, not
    /// two, against its bound of 1.
    #[test]
    fn a_member_is_counted_for_the_template_it_resolves_to() {
        let dir = std::env::temp_dir().join(format!("zenkey-fleet-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let l = zenkey_model::contract::load_str(
            "[interface]\nname = \"inv\"\nmajor = 1\n\
             [resources.\"items/{item}\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n\
             params = { item = \"string\" }\ncardinality = 1\n\
             [resources.\"items/total\"]\nkind = \"state\"\ntype = { raw = \"text/plain\" }\n",
            &dir,
            None,
        );
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        let items = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == "items/{item}")
            .unwrap();
        let total = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == "items/total")
            .unwrap();
        let a = "zk2/lab/inv/inv.v1/state/items/a";
        let t = "zk2/lab/inv/inv.v1/state/items/total";
        assert!(resolves_to(Some(&c), items, a));
        assert!(!resolves_to(Some(&c), items, t), "the literal wins");
        assert!(resolves_to(Some(&c), total, t));
        assert!(!resolves_to(
            Some(&c),
            items,
            "zk2/lab/inv/inv.v1/stream/items/a"
        ));
        assert!(resolves_to(None, items, t), "no contract, no filter");
        let h = Heard {
            arrivals: [a, t]
                .into_iter()
                .map(|k| {
                    (
                        k.to_owned(),
                        vec![Arrival {
                            at: Duration::from_secs(1),
                            stamp: None,
                        }],
                    )
                })
                .collect(),
            ..Heard::default()
        };
        assert_eq!(members_heard(Some(&c), items, &h).len(), 1);
    }
}
