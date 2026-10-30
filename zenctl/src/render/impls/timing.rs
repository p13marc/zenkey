//! `timing.v1` (#908, MC): `zenctl timing`'s report.

use zenkey_fleet::report::{
    DoctorSeverity, TimingAnswer, TimingAnswerToken, TimingMember, TimingReport, TimingRollUp,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// One answer in a line: its token and its reason's token.
fn said(a: &TimingAnswer) -> String {
    format!("{} ({})", a.answer.as_str(), a.reason)
}

/// One roll-up in a line: its token, and the member that gave it.
fn rolled(r: &TimingRollUp) -> String {
    match (&r.first, &r.reason) {
        (Some(k), Some(why)) if r.answer != TimingAnswerToken::NotAsked => {
            format!("{} ({why}, {k})", r.answer.as_str())
        }
        _ => r.answer.as_str().to_owned(),
    }
}

/// A member's mark: `✗` a finding, `·` every question asked answered on
/// its other pole, `?` one unobservable, `—` none asked.
fn mark(m: &TimingMember) -> (&'static str, anstyle::Style) {
    use crate::render::style;
    let answers = [&m.deadline, &m.period, &m.lifespan];
    if answers.iter().any(|a| a.finding) {
        return ("✗", style::severity(DoctorSeverity::Error));
    }
    if answers
        .iter()
        .any(|a| a.answer == TimingAnswerToken::Unobservable)
    {
        return ("?", style::UNPROVEN);
    }
    if answers.iter().all(|a| {
        matches!(
            a.answer,
            TimingAnswerToken::NotAsked | TimingAnswerToken::NotThisProfiles
        )
    }) {
        return ("—", style::UNPROVEN);
    }
    ("·", style::PASS)
}

/// `timing.v1` (#908, MC): §5's three answers in the profile's tokens. One
/// row per timed resource (`resource`, with its declaration, its owner as
/// the window saw it, and its questions rolled up) and per member held
/// (`member`, with its span held, its arrivals, its misses with the owner's
/// state at each, its end, its lifespans counted, and its three answers),
/// each its own mark in the table. The run's judgement, and its three
/// questions rolled up, lead the envelope.
impl Render for TimingReport {
    const FAMILY: &'static str = "timing";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["resources", "members"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.resources {
            out(Row::of("resource", r));
        }
        for m in &self.members {
            out(Row::of("member", m));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        t.line(format!(
            "timing under {} in {} — {} timed resource(s), {} member(s) held over {:.1}s: \
             deadline {}, period {}, lifespan {}",
            self.selector.as_deref().unwrap_or("every timed stream"),
            namespace_words(&self.namespace),
            self.resources.len(),
            self.members.len(),
            self.window_s,
            rolled(&self.run.deadline),
            rolled(&self.run.period),
            rolled(&self.run.lifespan),
        ));
        let mut resources = Grid::unheaded(3);
        for r in &self.resources {
            let (mark, st) = match r.declaration.declaration.as_str() {
                "declared" if !r.declaration.below_period => ("✓", style::PASS),
                _ => ("!", style::severity(DoctorSeverity::Warning)),
            };
            resources.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} {} {}", r.address, r.iface, r.resource)),
                Cell::text(format!(
                    "{}; the owner {}{}; {} member(s) held",
                    r.declaration.says,
                    if r.owner.present_throughout {
                        "present throughout".to_owned()
                    } else {
                        format!("not present throughout ({} at the end)", r.owner.at_end)
                    },
                    if r.beyond_bound.is_empty() {
                        String::new()
                    } else {
                        format!("; {} past its bound", r.beyond_bound.len())
                    },
                    r.members
                )),
            ]);
        }
        t.grid(resources);
        let mut members = Grid::unheaded(3);
        for m in &self.members {
            let (mark, st) = mark(m);
            members.row([
                Cell::styled(mark, st),
                Cell::text(&m.key),
                Cell::text(format!(
                    "deadline {}, period {}, lifespan {}",
                    said(&m.deadline),
                    said(&m.period),
                    said(&m.lifespan)
                )),
            ]);
            let mut detail = vec![format!(
                "    held {:.0} ms, {} arrival(s){}{}",
                m.held_ms,
                m.arrivals,
                m.span_ms
                    .map(|s| format!(" over a {s:.0} ms span"))
                    .unwrap_or_default(),
                match &m.ended {
                    Some(e) => format!("; ended at {:.0} ms ({})", e.at_ms, e.reason),
                    None => String::new(),
                }
            )];
            for x in &m.misses {
                detail.push(format!(
                    "    missed at {:.1} ms, due {:.1} ms (by {:.1} ms), its owner {}",
                    x.at_ms, x.due_ms, x.by_ms, x.owner
                ));
            }
            if m.missed as usize > m.misses.len() {
                detail.push(format!(
                    "    the first {} of its {} miss(es)",
                    m.misses.len(),
                    m.missed
                ));
            }
            let l = &m.lifespans;
            if l.judged > 0 {
                detail.push(format!(
                    "    lifespans: {} live, {} expired, {} unobservable{}",
                    l.live,
                    l.expired,
                    l.unobservable,
                    if l.reasons.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " ({})",
                            l.reasons
                                .iter()
                                .map(|(r, n)| format!("{r} ×{n}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ));
            }
            members.detail(detail);
        }
        t.grid(members);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        if self.across_face {
            notes.push(
                Note::coverage(
                    "across a constrained face: presence and descriptors do not cross, so every \
                     owner is unseen, the deadline and the period are unobservable, and a \
                     lifespan is attributed only to the meta.zids --owner-zid states",
                )
                .cite("timing.v1 §2.8"),
            );
        } else if let Some(o) = &self.opening {
            notes.push(
                Note::coverage(format!(
                    "read {ns}: the window started on a {} presence read through `{}`, {} of {} \
                     instance(s) in scope described{}",
                    o.presence,
                    o.selector,
                    o.descriptors,
                    o.instances,
                    if o.stood {
                        String::new()
                    } else {
                        "; its start does not stand, so an owner it did not see is unseen at \
                         first"
                            .to_owned()
                    }
                ))
                .cite("timing.v1 §2.7"),
            );
        }
        notes.push(
            Note::coverage(format!(
                "a {:.1}s window on this host's monotonic clock, each arrival read in its \
                 subscriber's callback; a miss is reported within max(deadline/50, 1 ms) of its \
                 due",
                self.window_s
            ))
            .cite("timing.v1 §2.5"),
        );
        match self.clock_bound_ms {
            Some(ms) => notes.push(
                Note::coverage(format!(
                    "lifespans judged under ε = {ms} ms, the deployment's word: a word the \
                     clocks do not keep makes the verdicts wrong, and the error is the \
                     deployment's"
                ))
                .cite("timing.v1 §2.6"),
            ),
            None => notes.push(
                Note::caveat(
                    "no --clock-bound: every lifespan is unobservable (no_bound), never judged \
                     on the HLC's delta",
                )
                .cite("timing.v1 §2.6"),
            ),
        }
        if self.members.iter().any(|m| m.missed > 0) {
            notes.push(
                Note::caveat(
                    "a miss is never \"down\": it says nothing arrived within the deadline, as \
                     this reader observed; whether the owner is gone is presence's word",
                )
                .cite("timing.v1 §2.5"),
            );
        }
        if self.lost > 0 {
            notes.push(Note::caveat(format!(
                "this reader lost {} delivery(ies) or stalled: no miss and no period not kept is \
                 the owner's",
                self.lost
            )));
        }
        if self.unresolved > 0 {
            notes.push(Note::caveat(format!(
                "{} sample(s) on a key no timed resource holds, or on a wildcard key, were not \
                 read",
                self.unresolved
            )));
        }
        for s in &self.skipped {
            notes.push(Note::caveat(format!("not read: {s}")));
        }
        notes.extend(super::reach_notes(self.scope.as_ref(), "timed stream"));
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        notes.push(Note::summary(format!(
            "{} member(s) of {} timed resource(s); deadline {}, period {}, lifespan {}.",
            self.members.len(),
            self.resources.len(),
            self.run.deadline.answer.as_str(),
            self.run.period.answer.as_str(),
            self.run.lifespan.answer.as_str()
        )));
        notes
    }

    /// What the reading put to the bus, over its window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: Some(self.window_s),
        })
    }
}
