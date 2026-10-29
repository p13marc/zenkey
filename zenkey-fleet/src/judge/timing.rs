//! `timing.v1` (#908, MC): a tool's window over timed streams judged, every
//! verdict decided from values (`spec/profiles/timing/v1.md` §5).
//!
//! **Session-free.** [`judge`] reads a [`TimingObservation`] that
//! [`crate::bus::timing`] gathered and hands back a [`TimingReport`]. Every
//! answer is `zenkey_model::timing`'s, through the book
//! ([`crate::model::timing`]): `deadline_met` over the tracker's account and
//! the span held, `period_kept` over `period_window`'s verdict,
//! `lifespan_expired` over each sample's `LifespanReader` judgement, each
//! under the window's premises (across a face, lost, the owner present
//! throughout); and `roll_up` over a resource's members and the run's, read
//! by each question's finding (text 0.2, F-264), every member set aside not
//! asked (text 0.4, F-277). Nothing is re-derived
//! here: this module maps the model's answers onto the report's tokens.
//!
//! **The run's verdict** ([`TimingReport::judgement`]): a finding of any
//! question over the run is the finding; every question asked answered on
//! its other pole is clean; anything else is no verdict.

use zenkey_model::timing::{
    Answer, Declaration, Judged, Question, Reason, deadline_met, lifespan_expired, period_kept,
    roll_up,
};

use crate::bus::timing::TimingObservation;
use crate::model::timing::{MemberWindow, TimingBook};
use crate::report::{
    TimingAnswer, TimingAnswerToken, TimingDeclaration, TimingEnd, TimingLifespans, TimingMember,
    TimingMiss, TimingOwner, TimingQuestions, TimingReport, TimingResource, TimingRollUp,
};

/// The report's token for one of the model's answers.
pub fn token(a: Answer) -> TimingAnswerToken {
    match a {
        Answer::Yes => TimingAnswerToken::Yes,
        Answer::No => TimingAnswerToken::No,
        Answer::Unobservable => TimingAnswerToken::Unobservable,
        Answer::NotAsked => TimingAnswerToken::NotAsked,
        Answer::NotThisProfiles => TimingAnswerToken::NotThisProfiles,
    }
}

fn model_answer(t: TimingAnswerToken) -> Answer {
    match t {
        TimingAnswerToken::Yes => Answer::Yes,
        TimingAnswerToken::No => Answer::No,
        TimingAnswerToken::Unobservable => Answer::Unobservable,
        TimingAnswerToken::NotAsked => Answer::NotAsked,
        TimingAnswerToken::NotThisProfiles => Answer::NotThisProfiles,
    }
}

/// One question's answer for a member, as the report carries it.
pub fn answer(q: Question, j: Judged) -> TimingAnswer {
    TimingAnswer {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: j.reason.says().to_owned(),
        finding: j.answer == q.finding(),
    }
}

/// One question over `members` (each its key and its answer), by the
/// model's `roll_up` alone (F-264; text 0.4, F-277: every member set
/// aside is not asked, or not this profile's when every one is; no member
/// at all is unobservable).
pub fn roll<'a>(
    q: Question,
    members: impl IntoIterator<Item = (&'a str, &'a TimingAnswer)> + Clone,
) -> TimingRollUp {
    let mut counts = std::collections::BTreeMap::new();
    for (_, a) in members.clone() {
        *counts.entry(a.answer.as_str().to_owned()).or_insert(0) += 1;
    }
    let answer = token(roll_up(
        q,
        members
            .clone()
            .into_iter()
            .map(|(_, a)| model_answer(a.answer)),
    ));
    let first = members
        .into_iter()
        .find(|(_, a)| a.answer == answer)
        .map(|(k, a)| (k.to_owned(), a.reason.clone()));
    TimingRollUp {
        answer,
        first: first.as_ref().map(|(k, _)| k.clone()),
        reason: first.map(|(_, r)| r),
        members: counts,
        finding: model_answer(answer) == q.finding(),
    }
}

fn whole_ms(d: Option<std::time::Duration>) -> Option<u64> {
    d.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A resource's declaration, in the report's words.
pub fn declaration_row(d: &Declaration) -> TimingDeclaration {
    match d {
        Declaration::Declared(x) => TimingDeclaration {
            declaration: "declared".into(),
            reasons: vec![],
            period_ms: whole_ms(x.period),
            deadline_ms: whole_ms(x.deadline),
            lifespan_ms: whole_ms(x.lifespan),
            below_period: x.below_period(),
            says: declaration_says(d),
        },
        Declaration::Invalid(why) => TimingDeclaration {
            declaration: "invalid".into(),
            reasons: why.iter().map(|w| w.as_str().to_owned()).collect(),
            period_ms: None,
            deadline_ms: None,
            lifespan_ms: None,
            below_period: false,
            says: declaration_says(d),
        },
        other => TimingDeclaration {
            declaration: other.as_str().into(),
            reasons: vec![],
            period_ms: None,
            deadline_ms: None,
            lifespan_ms: None,
            below_period: false,
            says: declaration_says(d),
        },
    }
}

/// A declaration in words: its values, or why it is invalid or ignored.
pub fn declaration_says(d: &Declaration) -> String {
    match d {
        Declaration::Declared(x) => {
            let mut parts = Vec::new();
            if let Some(p) = whole_ms(x.period) {
                parts.push(format!("period {p} ms"));
            }
            if let Some(dl) = whole_ms(x.deadline) {
                if x.below_period() {
                    parts.push(format!(
                        "deadline {dl} ms, below its period, so no deadline"
                    ));
                } else {
                    let bound =
                        zenkey_model::timing::detection_bound(x.deadline.unwrap_or_default());
                    parts.push(format!(
                        "deadline {dl} ms, seen within {} ms",
                        dl + whole_ms(Some(bound)).unwrap_or(1)
                    ));
                }
            }
            if let Some(l) = whole_ms(x.lifespan) {
                parts.push(format!("lifespan {l} ms"));
            }
            parts.join(", ")
        }
        Declaration::Invalid(why) => format!(
            "invalid: {}",
            why.iter()
                .map(|w| format!("{} ({})", w.says(), w.as_str()))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        Declaration::Ignored => {
            "timing keys on a resource that is not a stream, or a requirement: they mean \
             nothing there (§2.3)"
                .to_owned()
        }
        Declaration::NotThisProfiles => "no timing key".to_owned(),
    }
}

/// One member's three answers, and its row.
pub fn member_row(book: &TimingBook, key: &str, m: &MemberWindow) -> TimingMember {
    let r = &book.resources()[m.resource];
    let w = book.window_of(key).unwrap_or_default();
    let period = book.period(key);
    let deadline = answer(
        Question::Deadline,
        deadline_met(r.kind, &r.declaration, &w, &m.deadline_account()),
    );
    let period_answer = answer(
        Question::Period,
        match period {
            Some(p) => period_kept(r.kind, &r.declaration, &w, p.verdict),
            // No period declared (or an invalid declaration): the model's
            // premises answer before the verdict is read.
            None => period_kept(
                r.kind,
                &r.declaration,
                &w,
                zenkey_model::timing::PeriodVerdict::NotHeard,
            ),
        },
    );
    let lifespan = answer(
        Question::Lifespan,
        lifespan_expired(r.kind, &r.declaration, &m.samples),
    );
    let mut lifespans = TimingLifespans {
        judged: 0,
        live: 0,
        expired: 0,
        unobservable: 0,
        reasons: Default::default(),
    };
    // A sample of a resource that declares no lifespan is not asked, and
    // judged none.
    for ((verdict, reason), n) in &m.lifespans {
        match *verdict {
            "live" => lifespans.live += n,
            "expired" => lifespans.expired += n,
            "unobservable" => {
                lifespans.unobservable += n;
                *lifespans.reasons.entry((*reason).to_owned()).or_insert(0) += n;
            }
            _ => continue,
        }
        lifespans.judged += n;
    }
    TimingMember {
        key: key.to_owned(),
        address: r.address.to_string(),
        iface: r.iface.to_string(),
        resource: r.resource.clone(),
        known: m.known,
        held_ms: m.held.as_secs_f64() * 1e3,
        arrivals: m.arrivals.len() as u64 + m.arrivals_dropped,
        span_ms: period.and_then(|p| p.span).map(|s| s.as_secs_f64() * 1e3),
        missed: m.missed(),
        missed_present: m.account.missed_present,
        misses: m
            .misses
            .iter()
            .filter_map(|r| match r {
                zenkey_model::timing::Report::Missed {
                    at, due, by, owner, ..
                } => Some(TimingMiss {
                    at_ms: book.ms_since_start(*at),
                    due_ms: book.ms_since_start(*due),
                    by_ms: book.ms_since_start(*by),
                    owner: owner.as_str().to_owned(),
                }),
                _ => None,
            })
            .collect(),
        met: m.mets,
        ended: m.ended.map(|(at, reason)| TimingEnd {
            at_ms: book.ms_since_start(at),
            reason: reason.as_str().to_owned(),
        }),
        deletes: m.deletes,
        lifespans,
        deadline,
        period: period_answer,
        lifespan,
    }
}

/// The three questions over `members`.
pub fn questions(members: &[&TimingMember]) -> TimingQuestions {
    TimingQuestions {
        deadline: roll(
            Question::Deadline,
            members.iter().map(|m| (m.key.as_str(), &m.deadline)),
        ),
        period: roll(
            Question::Period,
            members.iter().map(|m| (m.key.as_str(), &m.period)),
        ),
        lifespan: roll(
            Question::Lifespan,
            members.iter().map(|m| (m.key.as_str(), &m.lifespan)),
        ),
    }
}

/// One reading judged: see the module doc.
pub fn judge(obs: &TimingObservation) -> TimingReport {
    let book = &obs.book;
    let members: Vec<TimingMember> = book
        .members()
        .map(|(k, m)| member_row(book, k, m))
        .collect();
    let resources = book
        .resources()
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let of: Vec<&TimingMember> = members
                .iter()
                .filter(|m| {
                    m.address == r.address.to_string()
                        && m.iface == r.iface.to_string()
                        && m.resource == r.resource
                })
                .collect();
            TimingResource {
                address: r.address.to_string(),
                iface: r.iface.to_string(),
                fingerprint: r.fingerprint.clone(),
                resource: r.resource.clone(),
                selector: r.selector.clone(),
                declaration: declaration_row(&r.declaration),
                bound: r.bound,
                owner: TimingOwner {
                    present_throughout: book.owner_present_throughout(&r.address, &r.iface),
                    at_end: book.owner(&r.address, &r.iface).as_str().to_owned(),
                    zids: book
                        .owner_zids(&r.address)
                        .into_iter()
                        .map(|z| format!("{z:x}"))
                        .collect(),
                    tokenless: book.tokenless(&r.address, &r.iface),
                },
                members: of.len(),
                beyond_bound: book.beyond_bound(i).cloned().collect(),
                questions: questions(&of),
            }
        })
        .collect();
    let all: Vec<&TimingMember> = members.iter().collect();
    let run = questions(&all);
    TimingReport {
        namespace: obs.namespace.clone(),
        selector: obs.scope.selector().map(str::to_owned),
        across_face: obs.spec.across_face,
        clock_bound_ms: whole_ms(obs.spec.clock_bound),
        opening: obs.opening.clone(),
        window_s: book.window().as_secs_f64(),
        asked: obs.asked.clone(),
        resources,
        members,
        run,
        lost: book.lost(),
        refused: book.refused(),
        unresolved: book.unresolved() + obs.discarded,
        skipped: obs.skipped.clone(),
        unobservable: obs.unobservable.clone(),
    }
}

/// A reason's words, for a frontend.
pub fn reason_says(token: &str) -> &'static str {
    Reason::parse(token).map_or("", Reason::says)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unobs_early() -> TimingAnswer {
        a(TimingAnswerToken::Unobservable, "window_too_short")
    }

    fn a(t: TimingAnswerToken, reason: &str) -> TimingAnswer {
        TimingAnswer {
            answer: t,
            reason: reason.into(),
            says: String::new(),
            finding: false,
        }
    }

    /// F-264 through the model's roll-up: one member expired beside one
    /// live answers the lifespan yes, naming the expired one; a member not
    /// asked is set aside; nobody asked is not asked.
    #[test]
    fn a_roll_up_is_the_models_read_by_its_finding() {
        let (live, expired) = (
            a(TimingAnswerToken::No, "live"),
            a(TimingAnswerToken::Yes, "expired"),
        );
        let r = roll(Question::Lifespan, [("x", &live), ("y", &expired)]);
        assert_eq!(r.answer, TimingAnswerToken::Yes);
        assert!(r.finding);
        assert_eq!(
            (r.first.as_deref(), r.reason.as_deref()),
            (Some("y"), Some("expired"))
        );
        let not_asked = a(TimingAnswerToken::NotAsked, "no_lifespan");
        let r = roll(Question::Lifespan, [("x", &live), ("z", &not_asked)]);
        assert_eq!((r.answer, r.finding), (TimingAnswerToken::No, false));
        let r = roll(Question::Lifespan, [("z", &not_asked)]);
        assert_eq!(r.answer, TimingAnswerToken::NotAsked, "F-277");
        let other = a(TimingAnswerToken::NotThisProfiles, "kind");
        let r = roll(Question::Period, [("s", &other)]);
        assert_eq!(
            (r.answer, r.finding),
            (TimingAnswerToken::NotThisProfiles, false),
            "every member not this profile's"
        );
        let r = roll(
            Question::Deadline,
            [("z", &not_asked), ("u", &unobs_early())],
        );
        assert_eq!(
            (r.answer, r.finding),
            (TimingAnswerToken::Unobservable, false),
            "an unobservable member is never set aside"
        );
        let r = roll(Question::Lifespan, std::iter::empty());
        assert_eq!(
            (r.answer, r.finding),
            (TimingAnswerToken::Unobservable, false),
            "no member at all"
        );
        let unobs = a(TimingAnswerToken::Unobservable, "no_bound");
        let r = roll(Question::Lifespan, [("x", &live), ("u", &unobs)]);
        assert_eq!(
            (r.answer, r.reason.as_deref()),
            (TimingAnswerToken::Unobservable, Some("no_bound"))
        );
    }
}
