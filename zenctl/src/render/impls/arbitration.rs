//! `arbitration.v1` (#909, BC): `zenctl arbitration`'s report.

use zenkey_fleet::report::{
    ArbitrationAnswerToken, ArbitrationReport, ArbitrationRole, DoctorSeverity,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// A role's mark: `✗` a declaration invalid or the view in the dead-man,
/// `!` a binding's warning alone, `·` every question asked answered on its
/// other pole, `?` one unobservable, `—` none asked.
fn mark(r: &ArbitrationRole) -> (&'static str, anstyle::Style) {
    use crate::render::style;
    if r.declaration.finding || r.selection.finding {
        return ("✗", style::severity(DoctorSeverity::Error));
    }
    if r.binding.finding {
        return ("!", style::severity(DoctorSeverity::Warning));
    }
    let answers = r.answers();
    if answers
        .iter()
        .any(|a| a.1 == ArbitrationAnswerToken::Unobservable)
    {
        return ("?", style::UNPROVEN);
    }
    if answers.iter().all(|a| !a.1.asked()) {
        return ("—", style::UNPROVEN);
    }
    ("·", style::PASS)
}

/// The selection in a line: its token and reason, the provider named, and,
/// when a view answered it, whose view it is.
fn selected(r: &ArbitrationRole) -> String {
    let s = &r.selection;
    let mut parts = vec![s.reason.clone()];
    if let Some(p) = &s.provider {
        parts.push(p.clone());
    }
    if r.view.is_some() {
        parts.push(s.whose.to_owned());
    }
    format!("{} ({})", s.answer.as_str(), parts.join(", "))
}

/// The binding in a line, naming its entries.
fn bound(r: &ArbitrationRole) -> String {
    let b = &r.binding;
    let mut named = Vec::new();
    if !b.wildcard_entries.is_empty() {
        named.push(format!("wildcard {}", b.wildcard_entries.join(", ")));
    }
    if !b.shadowed_entries.is_empty() {
        named.push(format!("shadowed {}", b.shadowed_entries.join(", ")));
    }
    if named.is_empty() {
        format!("{} ({})", b.answer.as_str(), b.reason)
    } else {
        format!("{} ({})", b.answer.as_str(), named.join("; "))
    }
}

/// `arbitration.v1` (#909, BC): §5's three answers in the profile's tokens,
/// one `role` row per consumer's role (its subject, its declaration, the
/// tool's own view and the binding, with the view's switches and counts
/// when it listened). The run's judgement leads the envelope.
impl Render for ArbitrationReport {
    const FAMILY: &'static str = "arbitration";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["roles"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.roles {
            out(Row::of("role", r));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "arbitration of {} in {} — {} role(s) read, each view listening up to {:.1}s",
            self.target.as_deref().unwrap_or("every consumer"),
            namespace_words(&self.namespace),
            self.roles.len(),
            self.listen_s,
        ));
        let mut g = Grid::unheaded(3);
        for r in &self.roles {
            let (mark, st) = mark(r);
            g.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} → {}", r.subject(), r.iface)),
                Cell::text(format!(
                    "declaration {} ({}); selection {}; binding {}",
                    r.declaration.answer.as_str(),
                    r.declaration.reason,
                    selected(r),
                    bound(r)
                )),
            ]);
            let mut detail = vec![format!("    {}", r.declaration.says)];
            match &r.bindings {
                Some(b) => detail.push(format!("    bound, in order: [{}]", b.join(", "))),
                None => detail.push("    no consumer's descriptor: no binding read".to_owned()),
            }
            if let Some(v) = &r.view {
                detail.push(format!(
                    "    {} over {:.0} ms: {}{}{}",
                    v.whose,
                    v.listened_ms,
                    v.active
                        .as_deref()
                        .map_or_else(|| "the dead-man".to_owned(), |a| format!("follows {a}")),
                    if v.watching {
                        format!(", its watch's round trips {}", v.round_trips)
                    } else {
                        ", no watch (the deadline alone)".to_owned()
                    },
                    if v.lost {
                        ", samples lost within the last deadline"
                    } else {
                        ""
                    }
                ));
                for s in &v.switches {
                    detail.push(format!(
                        "    switch at {:.1} ms: {} → {}, {}",
                        s.at_ms,
                        s.from.as_deref().unwrap_or("none"),
                        s.to.as_deref().unwrap_or("none"),
                        s.cause
                    ));
                }
                if v.switches_dropped > 0 {
                    detail.push(format!("    and {} switch(es) more", v.switches_dropped));
                }
                for p in &v.providers {
                    detail.push(format!(
                        "    {}: delivered {}, not_active {}, deleted {}, expired {}, \
                         not_incumbent {}{}",
                        p.address,
                        p.delivered,
                        p.not_active,
                        p.deleted,
                        p.expired,
                        p.not_incumbent,
                        if p.withdrawn { ", withdrawn" } else { "" }
                    ));
                }
            }
            for n in &r.notes {
                detail.push(format!("    note: {n}"));
            }
            g.detail(detail);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        notes.push(
            Note::coverage(format!(
                "read {ns}: each role from the contract its descriptor's declared_by names, at \
                 the revision it lists, over every revision of the required interface in hand"
            ))
            .cite("arbitration.v1 §2.14"),
        );
        if self.roles.iter().any(|r| r.view.is_some()) {
            notes.push(
                Note::caveat(
                    "each selection is the tool's own view, on its own subscriptions, watch and \
                     monotonic clock: never the consumer's, whose samples ride another path; its \
                     switches are its own to tell",
                )
                .cite("arbitration.v1 §2.14"),
            );
        }
        if self.roles.iter().any(|r| r.binding.finding) {
            notes.push(
                Note::caveat(
                    "a wildcard entry ranks its addresses by system, then service, byte by \
                     byte: a priority nobody chose; a shadowed entry ranks nothing of its own",
                )
                .cite("arbitration.v1 §2.4"),
            );
        }
        for s in &self.skipped {
            notes.push(Note::caveat(format!("not read: {s}")));
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let count = |t: ArbitrationAnswerToken, q: usize| {
            self.roles.iter().filter(|r| r.answers()[q].1 == t).count()
        };
        notes.push(Note::summary(format!(
            "{} role(s); declaration valid {}, invalid {}; selection selected {}, dead-man {}; \
             binding warned {}.",
            self.roles.len(),
            count(ArbitrationAnswerToken::Yes, 0),
            count(ArbitrationAnswerToken::No, 0),
            count(ArbitrationAnswerToken::Yes, 1),
            self.roles
                .iter()
                .filter(|r| r.selection.reason == "dead_man")
                .count(),
            count(ArbitrationAnswerToken::Yes, 2),
        )));
        notes
    }

    /// What the reading put to the bus.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: Some(self.listen_s),
        })
    }
}
