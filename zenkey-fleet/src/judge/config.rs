//! `config.v1` (#768, CC): §5's questions, "what a tool may conclude",
//! each answered in the profile's three states
//! (`spec/profiles/config/v1.md` §5).
//!
//! **Session-free.** Every answer here is decided from a
//! [`ConfigReading`] ([`crate::model::config`]); [`judge`] builds one from
//! the [`ConfigObservation`] [`crate::bus::config`] gathered and hands back
//! the [`ConfigReport`] `zenctl config get` prints.
//!
//! | Question | Here |
//! |---|---|
//! | Is a change pending? | [`pending`] |
//! | Did my change land? | [`landed`] |
//! | Was its undo clean? | [`undo_clean`] |
//! | Is this group writable? | [`writable`] |
//! | Will this value survive a restart? | [`survives`] |
//! | Has it converged to its desired document? | [`converged`] |
//!
//! **The premise first, for every row** (§5): the service implements
//! `config.v1`, as its descriptor says; otherwise, or when presence shows
//! it absent, the question is not asked. Its descriptor unread, or the
//! presence read possibly incomplete, is unobservable. Across a constrained
//! face, the deployment's word stands for the descriptor.
//!
//! **Only a fresh document is the owner's word.** A view or a status that is
//! stale, or that a GET did not draw, answers nothing: unobservable, never a
//! no (core O5). An event is evidence whenever it was read (§2.17): a change
//! ends once.

use std::collections::BTreeSet;

use zenkey_model::config::{self as cm, Class, ConfigChangeEvent, DesiredOutcome, Outcome};
use zenkey_model::freshness::{self as fresh, Observation};

use crate::bus::config::{ConfigObservation, ServiceDocs, status_key, view_key};
use crate::model::config::{ConfigReading, DocRead, Got, doc_read, premise_in, projection_holds};
use crate::model::health::Trust;
use crate::report::{
    Asked, ChangeAnswers, ConfigAnswer, ConfigAnswerToken as A, ConfigPremise, ConfigPresenceRead,
    ConfigReport, DesiredAnswer, GroupAnswer, ParameterAnswer,
};

/// The answer a premise forces, or `None` when the question is put (§5).
pub fn premise(p: ConfigPremise) -> Option<ConfigAnswer> {
    match p {
        ConfigPremise::Listed | ConfigPremise::AcrossFace => None,
        ConfigPremise::NotListed => Some(ConfigAnswer::new(
            A::NotAsked,
            "not_listed",
            "its descriptor does not list config.v1: it implements none (config.v1 §2.1)",
        )),
        ConfigPremise::Absent => Some(ConfigAnswer::new(
            A::NotAsked,
            "absent",
            "no instance token of it visible to this reader: absent, which is presence's word \
             (core §8.1)",
        )),
        ConfigPremise::DescriptorUnread => Some(ConfigAnswer::new(
            A::Unobservable,
            "descriptor_unread",
            "no descriptor of it read: whether it implements config.v1 cannot be told",
        )),
        ConfigPremise::Incomplete => Some(ConfigAnswer::new(
            A::Unobservable,
            "presence_incomplete",
            "no instance token of it seen, and the presence read is possibly incomplete or was \
             not made: unobservable (core §8.1)",
        )),
    }
}

/// Why no fresh document answered: each one read, and its freshness;
/// `need_view` when only the view could answer (its class, `writable`,
/// `startup`).
pub fn none_fresh(r: &ConfigReading, need_view: bool) -> ConfigAnswer {
    let of = |name: &str, fresh: fresh::Judged, read: bool| {
        if read {
            format!("the {name} {} ({})", fresh.reason.says(), fresh.verdict)
        } else {
            format!("the {name} was not read ({})", fresh.reason.says())
        }
    };
    let mut parts = Vec::new();
    match &r.view {
        Some(v) => parts.push(of("view", v.fresh, v.value.is_some())),
        None => parts.push("the view was not asked (--status)".to_owned()),
    }
    if !need_view {
        parts.push(of("status", r.status.fresh, r.status.value.is_some()));
    }
    let hint = if [r.view.as_ref().map(|v| v.fresh), Some(r.status.fresh)]
        .into_iter()
        .flatten()
        .any(|f| f.reason == fresh::Reason::ClockUntrusted)
    {
        "; a reply is aged only against a clock trusted to the HLC delta: measured on a live put \
         (--for) or on the deployment's word (--clocks-synced), freshness.v1 §2.6"
    } else {
        ""
    };
    let what = if need_view {
        "no fresh view: the status carries no class, no writable and no startup value"
    } else {
        "neither the view nor the status is fresh"
    };
    ConfigAnswer::new(
        A::Unobservable,
        if need_view {
            "no_fresh_view"
        } else {
            "no_fresh_document"
        },
        format!("{what} — {}{hint}", parts.join("; ")),
    )
}

/// "Is a change pending?" (§5, §2.7, §2.9): yes with its token, groups,
/// deadline and whether it is applied; no from a fresh document without
/// one. The pending change the answer rests on comes beside it.
pub fn pending(r: &ConfigReading) -> (ConfigAnswer, Option<cm::Pending>) {
    if let Some(a) = premise(r.premise) {
        return (a, None);
    }
    let docs = r.fresh();
    let Some(doc) = docs.first() else {
        return (none_fresh(r, false), None);
    };
    match doc.pending() {
        Some(p) => (
            ConfigAnswer::new(
                A::Yes,
                "pending",
                format!(
                    "change {} is pending on {}, its deadline {} (the owner's clock), {} — by \
                     the fresh {}",
                    p.token,
                    p.groups.join(", "),
                    p.deadline,
                    if p.applied {
                        "applied"
                    } else {
                        "not every group applied yet"
                    },
                    doc.name()
                ),
            ),
            Some(p.clone()),
        ),
        None => (
            ConfigAnswer::new(
                A::No,
                "none_pending",
                format!("no change is pending, by the fresh {}", doc.name()),
            ),
            None,
        ),
    }
}

/// The event that ended the change `token`, when one was read.
fn event_of<'r>(r: &'r ConfigReading, token: &str) -> Option<&'r ConfigChangeEvent> {
    r.events.iter().rev().find(|e| e.token == token)
}

/// "Did my change land?" (§5): the change named by its token, as the `set`
/// that started it was answered, and the instance that answered it when the
/// caller knows it.
pub fn landed(r: &ConfigReading, token: &str, instance: Option<&str>) -> ConfigAnswer {
    if let Some(a) = premise(r.premise) {
        return a;
    }
    if let Some(e) = event_of(r, token) {
        return match e.outcome {
            Outcome::Applied | Outcome::Confirmed => ConfigAnswer::new(
                A::Yes,
                e.outcome.as_str(),
                format!(
                    "its event says {}: the change was kept, at revision {} (config.v1 §2.17)",
                    e.outcome.as_str(),
                    e.revision
                ),
            ),
            Outcome::RolledBack | Outcome::Partial => ConfigAnswer::new(
                A::No,
                e.outcome.as_str(),
                format!(
                    "its event says {}: the change was undone (config.v1 §2.11)",
                    e.outcome.as_str()
                ),
            ),
        };
    }
    let docs = r.fresh();
    let Some(doc) = docs.first() else {
        return none_fresh(r, false);
    };
    if let Some(i) = instance
        && doc.instance() != i
    {
        return ConfigAnswer::new(
            A::No,
            "restarted",
            format!(
                "the instance that answered the set ({i}) is gone, and the fresh {} names \
                 another ({}): a restart rolls a runtime change back (config.v1 §2.12)",
                doc.name(),
                doc.instance()
            ),
        );
    }
    if doc.pending().is_some_and(|p| p.token == token) {
        let p = doc.pending().expect("checked");
        return ConfigAnswer::new(
            A::NotYet,
            "still_pending",
            format!(
                "it is still pending, its deadline {} (the owner's clock): confirm it, or it is \
                 undone then",
                p.deadline
            ),
        );
    }
    match doc.last_change() {
        Some(l) if l.token == token => ConfigAnswer::new(
            A::Yes,
            "last_change",
            format!(
                "it is the last change of instance {}, by the fresh {} (config.v1 §2.6)",
                doc.instance(),
                doc.name()
            ),
        ),
        Some(l) => ConfigAnswer::new(
            A::Unobservable,
            "another_change",
            format!(
                "the last change is another ({}), and no event of this one was read: a later \
                 change explains that as well as an undo",
                l.token
            ),
        ),
        None => ConfigAnswer::new(
            A::Unobservable,
            "no_event",
            format!(
                "it is neither pending nor the last change of the fresh {}, and no event of it \
                 was read",
                doc.name()
            ),
        ),
    }
}

/// "Was its undo clean?" (§5, §2.11), of a change that did not land: yes on
/// its `rolled-back` event, or a restart's rollback, clean by construction
/// (§2.12); no on `partial` — the read-back is the truth.
pub fn undo_clean(r: &ConfigReading, token: &str, instance: Option<&str>) -> ConfigAnswer {
    let landed = landed(r, token, instance);
    match landed.answer {
        A::NotAsked | A::Unobservable if premise(r.premise).is_some() => return landed,
        A::Yes => {
            return ConfigAnswer::new(
                A::NotAsked,
                "landed",
                "it landed: there was no undo to judge",
            );
        }
        A::NotYet => {
            return ConfigAnswer::new(
                A::NotAsked,
                "not_ended",
                "it is still pending: no undo has run yet",
            );
        }
        _ => {}
    }
    match event_of(r, token).map(|e| e.outcome) {
        Some(Outcome::RolledBack) => ConfigAnswer::new(
            A::Yes,
            "rolled-back",
            "its event says rolled-back: every parameter it set reads its value from before \
             (config.v1 §2.11)",
        ),
        Some(Outcome::Partial) => ConfigAnswer::new(
            A::No,
            "partial",
            "its event says partial: the undo failed, and the read-back is the truth — the \
             device may hold what the change set (config.v1 §2.11)",
        ),
        _ if landed.reason == "restarted" => ConfigAnswer::new(
            A::Yes,
            "restarted",
            "a restart rolled it back, which is clean by construction (config.v1 §2.12)",
        ),
        _ => ConfigAnswer::new(
            A::Unobservable,
            "no_event",
            "no event of it was read: its undo's outcome is the event's to say (config.v1 \
             §2.17)",
        ),
    }
}

/// "Is this group writable?" (§5, §2.16): a fresh view's group, `writable`
/// and hot or reach. The status carries no class and no `writable`.
pub fn writable(r: &ConfigReading, group: &str) -> ConfigAnswer {
    if let Some(a) = premise(r.premise) {
        return a;
    }
    let Some(v) = r.fresh_view() else {
        return none_fresh(r, true);
    };
    let Some(g) = v.groups.iter().find(|g| g.name == group) else {
        return ConfigAnswer::new(
            A::NotAsked,
            "no_such_group",
            format!("the schema declares no group {group:?}"),
        );
    };
    match (g.writable, g.class) {
        (_, Class::Contract) => ConfigAnswer::new(
            A::No,
            "contract",
            format!(
                "{group} is contract: what the transport was started against, changed in the \
                 startup configuration and a restart (config.v1 §2.4)"
            ),
        ),
        (false, _) => ConfigAnswer::new(
            A::No,
            "outside_allowlist",
            format!("{group} is outside this service's startup allowlist (config.v1 §2.16)"),
        ),
        (true, class) => ConfigAnswer::new(
            A::Yes,
            class.as_str(),
            format!(
                "{group} is {} and writable{}",
                class.as_str(),
                if class == Class::Reach {
                    ": a set needs a window, and is confirmed over the new link (config.v1 \
                     §2.4)"
                } else {
                    ""
                }
            ),
        ),
    }
}

/// "Will this value survive a restart?" (§5, §2.3): a fresh view's
/// parameter with a value and no `startup` runs its startup value; one with
/// `startup` runs a runtime change a restart would not keep.
pub fn survives(r: &ConfigReading, group: &str, parameter: &str) -> ConfigAnswer {
    if let Some(a) = premise(r.premise) {
        return a;
    }
    let Some(v) = r.fresh_view() else {
        return none_fresh(r, true);
    };
    let Some(p) = v
        .groups
        .iter()
        .find(|g| g.name == group)
        .and_then(|g| g.parameters.iter().find(|p| p.name == parameter))
    else {
        return ConfigAnswer::new(
            A::NotAsked,
            "no_such_parameter",
            format!("the schema declares no parameter {group}.{parameter}"),
        );
    };
    if p.sensitive {
        return ConfigAnswer::new(
            A::Unobservable,
            "sensitive",
            "write-only: its startup value is never shown (config.v1 §2.15)",
        );
    }
    match (&p.value, &p.startup) {
        (None, _) => ConfigAnswer::new(
            A::Unobservable,
            "value_unknown",
            "its owner cannot read its value back: unknown (config.v1 §2.3)",
        ),
        (Some(value), Some(startup)) => ConfigAnswer::new(
            A::No,
            "startup_differs",
            format!(
                "it runs {value}, and a restart would run {startup}: a divergence an operator \
                 may want persisted or undone (config.v1 §2.13)"
            ),
        ),
        (Some(value), None) => ConfigAnswer::new(
            A::Yes,
            "runs_startup",
            format!("it runs {value}, its startup value"),
        ),
    }
}

/// "Has it converged to its desired document?" (§5, §2.18), of the
/// document at `revision`, or, with none, of the last one the owner acted
/// on.
pub fn converged(r: &ConfigReading, revision: Option<u64>) -> ConfigAnswer {
    if let Some(a) = premise(r.premise) {
        return a;
    }
    let docs = r.fresh();
    let Some(doc) = docs.first() else {
        return none_fresh(r, false);
    };
    let Some(d) = doc.desired() else {
        return ConfigAnswer::new(
            A::NotAsked,
            "no_desired",
            "no desired member: the owner converges no desired state (config.v1 §2.18)",
        );
    };
    let want = revision.unwrap_or(d.observed_revision);
    let refusal = || {
        d.refusal
            .as_ref()
            .map(|f| {
                format!(
                    ": {} on {}{}",
                    f.code.as_str(),
                    f.group,
                    f.parameter
                        .as_deref()
                        .map(|p| format!(".{p}"))
                        .unwrap_or_default()
                )
            })
            .unwrap_or_default()
    };
    if d.observed_revision < want {
        return ConfigAnswer::new(
            A::NotYet,
            "below_revision",
            format!(
                "it has acted on revision {}, below {want}",
                d.observed_revision
            ),
        );
    }
    match d.outcome {
        DesiredOutcome::Converged => ConfigAnswer::new(
            A::Yes,
            "converged",
            format!(
                "converged at revision {}: every value it names is the running value",
                d.observed_revision
            ),
        ),
        outcome if d.observed_revision == want => {
            let word = if outcome == DesiredOutcome::Refused {
                "refused"
            } else {
                "failed"
            };
            ConfigAnswer::new(
                A::No,
                word,
                format!("{word} at revision {want}{}", refusal()),
            )
        }
        _ => ConfigAnswer::new(
            A::Unobservable,
            "later_document",
            format!(
                "it acted on a later document, revision {}, which it did not converge{}: this \
                 one's outcome is not shown",
                d.observed_revision,
                refusal()
            ),
        ),
    }
}

/// Whether the status read is the view's projection, in one reading
/// (§2.19, F-112): informative, since a view and a status a put apart pair
/// wrongly; a finding is two readings' (`check conform`, `doctor`).
pub fn projection(r: &ConfigReading) -> ConfigAnswer {
    let Some(v) = &r.view else {
        return ConfigAnswer::new(
            A::NotAsked,
            "view_not_asked",
            "only the status was read (--status)",
        );
    };
    match (v.value(), r.status.value()) {
        (Some(view), Some(status)) if projection_holds(view, status) => ConfigAnswer::new(
            A::Yes,
            "projection",
            "the status is the view's projection (config.v1 §2.19)",
        ),
        (Some(_), Some(_)) => ConfigAnswer::new(
            A::No,
            "differs",
            "the status read is not the view's projection: one reading can pair a view and a \
             status a put apart, so each is taken by itself (config.v1 §2.3)",
        ),
        _ => ConfigAnswer::new(
            A::Unobservable,
            "not_both_read",
            "the view and the status were not both read",
        ),
    }
}

/// `config get`'s reading of one service, from what [`crate::bus::config`]
/// read: presence, the GET, the window and a replay.
pub fn reading(obs: &ConfigObservation) -> ConfigReading {
    let window = match &obs.window {
        Some(Ok(w)) => Some(w),
        _ => None,
    };
    let trust = if obs.spec.clocks_synced {
        Trust::Word
    } else if let Some(w) = window {
        Trust::Measured(&w.clocks)
    } else {
        Trust::None
    };
    let premise = match &obs.presence {
        None => ConfigPremise::AcrossFace,
        Some(Ok(o)) => premise_in(o, &obs.address),
        Some(Err(_)) => ConfigPremise::Incomplete,
    };
    let empty = ServiceDocs::default();
    let (docs, at) = match &obs.get {
        Ok(g) => (
            g.services.get(&obs.address).unwrap_or(&empty),
            Some(g.read_at),
        ),
        Err(_) => (&empty, None),
    };
    let got_of = |made: bool| made && at.is_some();
    let view_got = match (&docs.view, at) {
        (Some(reply), Some(at)) => Got::Answered { reply, at },
        _ if got_of(!obs.spec.status_only) => Got::Silent,
        _ => Got::NotMade,
    };
    let status_got = match (&docs.status, at) {
        (Some(reply), Some(at)) => Got::Answered { reply, at },
        _ if got_of(true) => Got::Silent,
        _ => Got::NotMade,
    };
    let unheard = |w: &crate::bus::config::ConfigWindow| Observation::Subscribed {
        last: None,
        listened: w.listened,
        complete: true,
    };
    let view = (!obs.spec.status_only).then(|| {
        doc_read(
            view_got,
            window.map(|w| {
                (
                    w.seen
                        .get(&view_key(&obs.address))
                        .copied()
                        .unwrap_or(unheard(w)),
                    w.view.as_ref(),
                )
            }),
            &trust,
        )
    });
    let status = doc_read(
        status_got,
        window.map(|w| {
            (
                w.seen
                    .get(&status_key(&obs.address))
                    .copied()
                    .unwrap_or(unheard(w)),
                w.status.as_ref(),
            )
        }),
        &trust,
    );
    let mut events: Vec<ConfigChangeEvent> = Vec::new();
    if let Some(Ok(rp)) = &obs.replay {
        events.extend(rp.events.iter().cloned());
    }
    if let Some(w) = window {
        let seen: BTreeSet<(String, String)> = events
            .iter()
            .map(|e| (e.instance.clone(), e.token.clone()))
            .collect();
        events.extend(
            w.events
                .iter()
                .filter(|e| !seen.contains(&(e.instance.clone(), e.token.clone())))
                .cloned(),
        );
    }
    ConfigReading {
        premise,
        view,
        status,
        events,
    }
}

/// Every answer of one reading, as `config get` reports it.
pub fn judge(obs: &ConfigObservation) -> ConfigReport {
    let r = reading(obs);
    let window = match &obs.window {
        Some(Ok(w)) => Some(w),
        _ => None,
    };
    let mut unobservable = None;
    if let Err(e) = &obs.get {
        unobservable = Some(format!("the GET could not be made: {e}"));
    }
    // A read not made is the premise's possibly incomplete reading, and its
    // own reason is kept beside the GET's (core §8.1, 0.38, F-191).
    if let Some(Err(e)) = &obs.presence {
        let read = format!("the presence read could not be made: {e}");
        unobservable = Some(match unobservable {
            Some(get) => format!("{get}; {read}"),
            None => read,
        });
    }
    if let Some(Err(e)) = &obs.window {
        unobservable
            .get_or_insert_with(|| format!("the window's subscription could not be made: {e}"));
    }
    let (pending, pending_change) = pending(&r);
    let change = obs.spec.token.as_ref().map(|t| ChangeAnswers {
        token: t.clone(),
        instance: obs.spec.instance.clone(),
        landed: landed(&r, t, obs.spec.instance.as_deref()),
        undo_clean: undo_clean(&r, t, obs.spec.instance.as_deref()),
        event: event_of(&r, t).cloned(),
    });
    let shown = r.view.as_ref().and_then(DocRead::value);
    let writable_rows = shown
        .map(|v| {
            v.groups
                .iter()
                .map(|g| GroupAnswer {
                    group: g.name.clone(),
                    answer: writable(&r, &g.name),
                })
                .collect()
        })
        .unwrap_or_default();
    let survives_rows = shown
        .map(|v| {
            v.groups
                .iter()
                .flat_map(|g| {
                    g.parameters.iter().map(|p| ParameterAnswer {
                        group: g.name.clone(),
                        parameter: p.name.clone(),
                        answer: survives(&r, &g.name, &p.name),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let trust_ground = if obs.spec.clocks_synced {
        crate::report::ClockGround::DeploymentWord
    } else if window.is_some() {
        crate::report::ClockGround::Measured
    } else {
        crate::report::ClockGround::None
    };
    let presence = match &obs.presence {
        Some(Ok(o)) => Some(ConfigPresenceRead {
            selector: o.selector.clone(),
            complete: o.complete,
            cut_off_during: o.cut_off_during,
            instances: o
                .instances()
                .into_iter()
                .filter(|(a, _)| a == &obs.address)
                .map(|(_, i)| i.to_string())
                .collect(),
        }),
        _ => None,
    };
    ConfigReport {
        namespace: obs.namespace.clone(),
        service: obs.address.to_string(),
        premise: r.premise,
        presence,
        window_s: window.map(|w| w.listened.as_secs_f64()),
        clock: trust_ground,
        asked: obs.asked(),
        view: match &r.view {
            Some(v) => Asked::Asked(v.report()),
            None => Asked::NotAsked,
        },
        status: r.status.report(),
        projection: projection(&r),
        pending,
        pending_change,
        change,
        writable: writable_rows,
        survives_restart: survives_rows,
        converged: DesiredAnswer {
            revision: obs.spec.desired_revision,
            answer: converged(&r, obs.spec.desired_revision),
        },
        events: r.events.clone(),
        events_undecodable: window.map_or(0, |w| w.events_undecodable)
            + match &obs.replay {
                Some(Ok(rp)) => rp.undecodable,
                _ => 0,
            },
        unobservable,
    }
}

#[cfg(test)]
mod tests {
    //! §5's six questions, each pole from values, and `scenarios.md` §14's
    //! answers.

    use super::*;
    use crate::bus::health::Stamped;
    use serde_json::json;
    use std::time::{Duration, SystemTime};
    use zenkey_model::config::{ConfigStatus, ConfigView};
    use zenkey_model::freshness::{Judged, Reason, Verdict};

    fn view(v: serde_json::Value) -> ConfigView {
        let mut base = json!({
            "service": "lab/rf0", "instance": "i1", "revision": 0, "confirm_floor_s": 10,
            "groups": [
                {"name": "duty", "class": "hot", "description": "d", "writable": true,
                 "parameters": [{"name": "fraction_ppm", "kind": "integer",
                                 "description": "p", "value": 10000, "source": "file"}]},
                {"name": "radio", "class": "reach", "description": "d", "writable": true,
                 "parameters": [{"name": "frequency_hz", "kind": "integer",
                                 "description": "p", "value": 868100000, "source": "file"}]},
                {"name": "sim", "class": "hot", "description": "d", "writable": true,
                 "parameters": [{"name": "pin", "kind": "text", "description": "p",
                                 "sensitive": true, "source": "file"}]},
                {"name": "queue", "class": "hot", "description": "d", "writable": false,
                 "parameters": [{"name": "tx_queue_len", "kind": "integer",
                                 "description": "p", "value": 100, "source": "default"}]},
                {"name": "modulation", "class": "contract", "description": "d",
                 "writable": true, "parameters": []}
            ]
        });
        for (k, val) in v.as_object().unwrap() {
            base[k] = val.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn doc<T>(value: Option<T>, verdict: Verdict, reason: Reason) -> DocRead<T> {
        DocRead {
            value: value.map(Ok),
            via: None,
            stamp: None,
            fresh: Judged { verdict, reason },
        }
    }

    fn fresh_reading(v: ConfigView) -> ConfigReading {
        let s = ConfigStatus::of(&v);
        ConfigReading {
            premise: ConfigPremise::Listed,
            view: Some(doc(Some(v), Verdict::Fresh, Reason::WithinHorizon)),
            status: doc(Some(s), Verdict::Fresh, Reason::WithinHorizon),
            events: vec![],
        }
    }

    fn event(token: &str, outcome: &str) -> ConfigChangeEvent {
        serde_json::from_value(json!({
            "service": "lab/rf0", "instance": "i1", "revision": 2, "token": token,
            "outcome": outcome, "edits": []
        }))
        .unwrap()
    }

    /// A presence read not made is the premise's possibly incomplete
    /// reading, unobservable, and its own reason stays in the report beside
    /// the GET's when both failed (core §8.1, 0.38, F-191).
    #[test]
    fn a_read_not_made_keeps_its_reason_beside_the_premise() {
        let obs = ConfigObservation {
            namespace: String::new(),
            address: "lab/rf0".parse().unwrap(),
            spec: crate::bus::config::ConfigSpec::new(Duration::from_secs(1)),
            presence: Some(Err(crate::error::NotAsked::CutOff.to_string())),
            get: Err("GET zk2/lab/rf0/config.v1/@state/**: refused".into()),
            window: None,
            replay: None,
        };
        let r = judge(&obs);
        assert_eq!(r.premise, ConfigPremise::Incomplete);
        assert_eq!(r.pending.answer, A::Unobservable);
        assert_eq!(
            r.pending.reason, "presence_incomplete",
            "the profile's reason"
        );
        let why = r.unobservable.as_deref().expect("why");
        assert!(
            why.contains("the GET could not be made") && why.contains("no router and no peer"),
            "{why}"
        );
    }

    /// The premise answers every row first: not listed and absent are not
    /// asked, an unread descriptor and an incomplete read unobservable,
    /// across a face the deployment's word.
    #[test]
    fn the_premise_answers_first() {
        let mut r = fresh_reading(view(json!({})));
        for (p, want) in [
            (ConfigPremise::NotListed, A::NotAsked),
            (ConfigPremise::Absent, A::NotAsked),
            (ConfigPremise::DescriptorUnread, A::Unobservable),
            (ConfigPremise::Incomplete, A::Unobservable),
        ] {
            r.premise = p;
            assert_eq!(pending(&r).0.answer, want, "{p:?}");
            assert_eq!(writable(&r, "duty").answer, want);
            assert_eq!(landed(&r, "t", None).answer, want);
            assert_eq!(undo_clean(&r, "t", None).answer, want);
            assert_eq!(survives(&r, "duty", "fraction_ppm").answer, want);
            assert_eq!(converged(&r, None).answer, want);
        }
        r.premise = ConfigPremise::AcrossFace;
        assert_eq!(pending(&r).0.answer, A::No);
    }

    /// "Is a change pending?": yes with the change, no without, and
    /// unobservable when neither document is fresh — never a no.
    #[test]
    fn a_pending_change_is_read_from_a_fresh_document() {
        let r = fresh_reading(view(json!({})));
        assert_eq!(pending(&r).0.answer, A::No);
        let p = json!({"token": "i1-1", "groups": ["radio"],
                       "deadline": "2026-10-18T00:02:00.000Z", "applied": false});
        let r = fresh_reading(view(json!({"pending": p})));
        let (a, change) = pending(&r);
        assert_eq!(a.answer, A::Yes);
        assert!(a.says.contains("i1-1") && a.says.contains("not every group applied"));
        assert_eq!(change.unwrap().token, "i1-1");
        let mut stale = fresh_reading(view(json!({})));
        stale.view.as_mut().unwrap().fresh = Judged {
            verdict: Verdict::Stale,
            reason: Reason::BeyondHorizon,
        };
        stale.status.fresh = Judged {
            verdict: Verdict::Unobservable,
            reason: Reason::ClockUntrusted,
        };
        let a = pending(&stale).0;
        assert_eq!(a.answer, A::Unobservable);
        assert!(a.says.contains("--clocks-synced"), "{}", a.says);
    }

    /// The later-stamped fresh document answers when two disagree.
    #[test]
    fn the_later_put_answers() {
        let p = json!({"token": "i1-1", "groups": ["duty"],
                       "deadline": "2026-10-18T00:02:00.000Z", "applied": true});
        let mut r = fresh_reading(view(json!({"pending": p})));
        let stamp = |s: u64| {
            Some(Stamped {
                time: SystemTime::UNIX_EPOCH + Duration::from_secs(s),
                clock: "ab".into(),
            })
        };
        r.status.value = Some(Ok(ConfigStatus::of(&view(json!({})))));
        r.view.as_mut().unwrap().stamp = stamp(10);
        r.status.stamp = stamp(11);
        assert_eq!(pending(&r).0.answer, A::No, "the status was put later");
        r.status.stamp = stamp(9);
        assert_eq!(pending(&r).0.answer, A::Yes);
    }

    /// "Did my change land?" and "was its undo clean?" (scenarios.md §14
    /// steps 2 and 3): not yet while pending; no and clean on rolled-back;
    /// partial is the undo's no; the last change is a yes; a restart is a
    /// no, clean by construction; another last change with no event is
    /// unobservable.
    #[test]
    fn a_change_lands_or_is_undone() {
        let p = json!({"token": "i1-1", "groups": ["radio"],
                       "deadline": "2026-10-18T00:02:00.000Z", "applied": true});
        let pending_r = fresh_reading(view(json!({"pending": p})));
        assert_eq!(landed(&pending_r, "i1-1", None).answer, A::NotYet);
        assert_eq!(undo_clean(&pending_r, "i1-1", None).answer, A::NotAsked);
        let mut rolled = fresh_reading(view(json!({"revision": 2})));
        rolled.events.push(event("i1-1", "rolled-back"));
        assert_eq!(landed(&rolled, "i1-1", None).answer, A::No);
        assert_eq!(landed(&rolled, "i1-1", None).reason, "rolled-back");
        assert_eq!(undo_clean(&rolled, "i1-1", None).answer, A::Yes);
        let mut partial = fresh_reading(view(json!({"revision": 2})));
        partial.events.push(event("i1-1", "partial"));
        assert_eq!(undo_clean(&partial, "i1-1", None).answer, A::No);
        let kept = fresh_reading(view(json!({"revision": 1,
            "last_change": {"token": "i1-1", "groups": ["duty"]}})));
        assert_eq!(landed(&kept, "i1-1", None).answer, A::Yes);
        assert_eq!(undo_clean(&kept, "i1-1", None).answer, A::NotAsked);
        let other = fresh_reading(view(json!({"revision": 3,
            "last_change": {"token": "i1-3", "groups": ["duty"]}})));
        assert_eq!(landed(&other, "i1-1", None).answer, A::Unobservable);
        let restarted = fresh_reading(view(json!({"instance": "i2"})));
        assert_eq!(landed(&restarted, "i1-1", Some("i1")).answer, A::No);
        assert_eq!(undo_clean(&restarted, "i1-1", Some("i1")).answer, A::Yes);
        let mut nothing = fresh_reading(view(json!({})));
        nothing.view = None;
        nothing.status.fresh = Judged {
            verdict: Verdict::Unobservable,
            reason: Reason::Silent,
        };
        assert_eq!(landed(&nothing, "i1-1", None).answer, A::Unobservable);
        assert_eq!(undo_clean(&nothing, "i1-1", None).answer, A::Unobservable);
    }

    /// "Is this group writable?" needs a fresh view: the status carries no
    /// class; a contract or an unwritable group is no; an undeclared group
    /// is not asked.
    #[test]
    fn a_groups_writability_is_the_views() {
        let r = fresh_reading(view(json!({})));
        assert_eq!(writable(&r, "duty").answer, A::Yes);
        assert_eq!(writable(&r, "radio").reason, "reach");
        assert_eq!(writable(&r, "queue").answer, A::No);
        assert_eq!(writable(&r, "modulation").reason, "contract");
        assert_eq!(writable(&r, "nope").answer, A::NotAsked);
        let mut status_only = r.clone();
        status_only.view = None;
        assert_eq!(writable(&status_only, "duty").answer, A::Unobservable);
    }

    /// "Will this value survive a restart?" (scenarios.md §14 step 4): no
    /// with `startup`, yes without; a sensitive or unknown value is
    /// unobservable.
    #[test]
    fn a_value_survives_a_restart_when_it_is_its_startup_value() {
        let mut v = view(json!({}));
        let r = fresh_reading(v.clone());
        assert_eq!(survives(&r, "duty", "fraction_ppm").answer, A::Yes);
        assert_eq!(survives(&r, "sim", "pin").answer, A::Unobservable);
        assert_eq!(survives(&r, "duty", "nope").answer, A::NotAsked);
        v.groups[0].parameters[0].value = Some(cm::Value::Integer(20000));
        v.groups[0].parameters[0].startup = Some(cm::Value::Integer(10000));
        let r = fresh_reading(v.clone());
        let a = survives(&r, "duty", "fraction_ppm");
        assert_eq!(a.answer, A::No);
        assert!(a.says.contains("10000"), "{}", a.says);
        v.groups[0].parameters[0].value = None;
        assert_eq!(
            survives(&fresh_reading(v), "duty", "fraction_ppm").reason,
            "value_unknown"
        );
    }

    /// "Has it converged?": not asked without a desired member; yes at or
    /// beyond the revision; not yet below it; no when refused or failed at
    /// it.
    #[test]
    fn a_desired_document_converges_or_is_refused() {
        let r = fresh_reading(view(json!({})));
        assert_eq!(converged(&r, None).answer, A::NotAsked);
        let ok = fresh_reading(view(json!({"desired":
            {"observed_revision": 3, "outcome": "converged"}})));
        assert_eq!(converged(&ok, Some(2)).answer, A::Yes);
        assert_eq!(converged(&ok, Some(4)).answer, A::NotYet);
        let refused = fresh_reading(view(json!({"desired": {"observed_revision": 2,
            "outcome": "refused", "refusal": {"code": "reach", "group": "radio"}}})));
        let a = converged(&refused, None);
        assert_eq!(a.answer, A::No);
        assert!(a.says.contains("reach on radio"), "{}", a.says);
        assert_eq!(converged(&refused, Some(1)).answer, A::Unobservable);
    }

    /// The projection, in one reading: yes or no, never a finding here.
    #[test]
    fn one_reading_pairs_the_status_with_the_view() {
        let r = fresh_reading(view(json!({})));
        assert_eq!(projection(&r).answer, A::Yes);
        let mut off = r.clone();
        let mut s = ConfigStatus::of(&view(json!({})));
        s.revision = 9;
        off.status.value = Some(Ok(s));
        assert_eq!(projection(&off).answer, A::No);
        off.view = None;
        assert_eq!(projection(&off).answer, A::NotAsked);
    }

    /// scenarios.md §14 step 5, after the owner's last put: a reader that
    /// does not read presence holds a status heard 70 s ago, stale: "is a
    /// change pending?" is unobservable, never no.
    #[test]
    fn a_stale_status_answers_nothing() {
        let mut r = fresh_reading(view(json!({})));
        r.premise = ConfigPremise::AcrossFace;
        r.view = None;
        r.status.fresh = fresh::judge(
            &crate::model::config::horizon(),
            &Observation::Subscribed {
                last: Some(fresh::Last::Put(Duration::from_secs(70))),
                listened: Duration::from_secs(120),
                complete: true,
            },
        );
        assert_eq!(r.status.fresh.verdict, Verdict::Stale);
        let a = pending(&r).0;
        assert_eq!(a.answer, A::Unobservable);
        assert!(
            a.says.contains("not confirmed within its horizon"),
            "{}",
            a.says
        );
    }
}
