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

use std::collections::BTreeSet;

use zenkey_model::authoring::Kind;
use zenkey_model::contract::{Body, Fanout, Resource};
use zenkey_model::schema::TypeId;

use crate::bus::conform::{ConformObservation, FanoutSeen, Heard, OpObserved};
use crate::judge::common::s1_premise;
use crate::judge::doctor::AdminSpace;
use crate::model::catalog::{ContractState, Revision, zid_value};
use crate::model::lens::conformance;
use crate::report::{
    CaseId, ConformCase, ConformReport, Conformance, OperationAnswer, OperationReport,
    PayloadRendering, PresenceAttribution, StateReport, StateValue, WatchEvent,
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
        report.cases.extend(profile_cases());
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
        report.cases.extend(profile_cases());
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
    }
    report.cases.extend(profile_cases());
    let mut seen = BTreeSet::new();
    report.asked.retain(|s| seen.insert(s.clone()));
    report
}

/// The profile-backed cases, never asked until their profiles exist (#613).
fn profile_cases() -> [ConformCase; 2] {
    [
        ConformCase::not_asked(
            CaseId::Freshness,
            "service",
            "judged against a declared freshness (freshness.v1), a profile that does not exist \
             yet (#613): not asked is neither a pass nor a violation",
        ),
        ConformCase::not_asked(
            CaseId::Budget,
            "service",
            "judged against a declared rate and population budget, a profile that does not \
             exist yet (#613): not asked is neither a pass nor a violation",
        ),
    ]
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
    use crate::bus::conform::ConformSpec;
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
            "[interface]\nname = \"m\"\nmajor = 1\n[schemas]\njsonschema = [\"s.json\"]\n\
             [resources.\"bandwidth/{dev}\"]\nkind = \"stream\"\ntype = \"json:Bw\"\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
             [resources.\"status/{dev}\"]\nkind = \"state\"\ntype = \"json:Status\"\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
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

    fn heard(samples: Vec<WatchSample>) -> Result<Heard, String> {
        Ok(Heard {
            received: samples.len() as u64,
            samples,
            selectors: vec!["zk2/lab/m/m.v1/stream/bandwidth/*".into()],
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
            },
            asked_fp: None,
            presence: Ok(presence(Some(OWNER))),
            served: Some(Ok(ContractState::Held(Arc::clone(&rev)))),
            revision: Some(rev),
            heard: BTreeMap::new(),
            gets: BTreeMap::new(),
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
        let r = judge(&conforming());
        assert_eq!(judgement_exit_code(&r.judgement()), 0, "{r:#?}");
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
        // fan-out probe; and the profile cases wait for their profiles.
        for (id, subject) in [
            (CaseId::Operation, "@op/set/{dev}"),
            (CaseId::FanoutRefused, "@op/set/{dev}"),
            (CaseId::Freshness, "service"),
            (CaseId::Budget, "service"),
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
        let r = judge(&o);
        assert!(matches!(
            case(&r, CaseId::FanoutRefused, "@op/set/{dev}").verdict,
            Judgement::NotEstablished { .. }
        ));
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
}
