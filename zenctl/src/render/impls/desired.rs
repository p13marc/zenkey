//! `desired.v1` (#840, DD): `zenctl desired`'s report.

use zenkey_fleet::report::{
    ClockGround, DesiredAnswerToken, DesiredBehindBy, DesiredListing, DesiredQuestion,
    DesiredReport,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// An answer's word, as the table spells it.
fn answer_word(a: DesiredAnswerToken) -> &'static str {
    match a {
        DesiredAnswerToken::Yes => "yes",
        DesiredAnswerToken::No => "no",
        DesiredAnswerToken::NotYet => "not yet",
        DesiredAnswerToken::Unobservable => "unobservable",
        DesiredAnswerToken::NotAsked => "not asked",
    }
}

fn says(q: &DesiredQuestion) -> String {
    format!("{} ({}) — {}", answer_word(q.answer), q.reason, q.says)
}

/// `desired.v1` (#840, DD): §5's four answers per role, in the profile's
/// tokens and in words. One row per target's role (`desired`), with what
/// was read beside the answers: the slice, the report as the target put
/// it, the author's version, an archive's last-known (never current), and
/// a target behind past the window. In the table, "has it converged?"
/// leads each row with its own mark — `✓ yes`, `✗ no` or behind, `… not
/// yet`, `? unobservable`, `— not asked` — and the other three follow as
/// lines. The run's judgement leads the envelope.
impl Render for DesiredReport {
    const FAMILY: &'static str = "desired";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["rows"]);
        e.insert(
            "judgement".into(),
            serde_json::to_value(self.judgement()).expect("a judgement serializes"),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rows {
            out(Row::of("desired", r));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        t.line(format!(
            "desired.v1 in {}{}{} — {} role(s), {} reading(s)",
            namespace_words(&self.namespace),
            self.target
                .as_ref()
                .map(|a| format!(", target {a}"))
                .unwrap_or_default(),
            self.role
                .as_ref()
                .map(|r| format!(", role {r}"))
                .unwrap_or_default(),
            self.rows.len(),
            self.readings
        ));
        let mut rows = Grid::unheaded(3);
        for r in &self.rows {
            let c = &r.answers.converged;
            let (mark, st) = match (r.finding().is_some(), c.answer) {
                (true, _) => (
                    "✗",
                    style::severity(zenkey_fleet::report::DoctorSeverity::Error),
                ),
                (false, DesiredAnswerToken::Yes) => ("✓", style::PASS),
                (false, DesiredAnswerToken::No) => (
                    "✗",
                    style::severity(zenkey_fleet::report::DoctorSeverity::Error),
                ),
                (false, DesiredAnswerToken::NotYet) => ("…", style::UNPROVEN),
                (false, DesiredAnswerToken::Unobservable) => ("?", style::UNPROVEN),
                (false, DesiredAnswerToken::NotAsked) => ("—", style::UNPROVEN),
            };
            rows.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} {}", r.target, r.role)),
                Cell::text(format!("converged: {}", says(c))),
            ]);
            let mut detail = Vec::new();
            let listing = match r.listing {
                DesiredListing::Bound => "bound",
                DesiredListing::Unbound => "unbound: the kill switch",
                DesiredListing::NotListed => "not listed on a desired resource",
                DesiredListing::Unread => "its descriptor not read",
            };
            detail.push(format!(
                "    role {}{}{}",
                listing,
                r.interface
                    .as_ref()
                    .map(|i| format!(", requiring {i}"))
                    .unwrap_or_default(),
                r.author
                    .as_ref()
                    .map(|a| format!(", its author {}", a.address))
                    .unwrap_or_default()
            ));
            if let Some(s) = &r.slice {
                detail.push(format!(
                    "    slice {s}{}",
                    r.slice_from
                        .as_ref()
                        .map(|f| format!(" (from the {})", f.replace('_', " ")))
                        .unwrap_or_default()
                ));
            }
            if !r.declaration_invalid.is_empty() {
                detail.push(format!(
                    "    the author's declaration is invalid: {}",
                    r.declaration_invalid.join(", ")
                ));
            }
            if !r.binding_invalid.is_empty() {
                detail.push(format!(
                    "    its binding is invalid: {}",
                    r.binding_invalid.join(", ")
                ));
            }
            if let Some(rep) = &r.report {
                let what = match (&rep.document, &rep.unreadable) {
                    (Some(d), _) => serde_json::to_string(d).unwrap_or_default(),
                    (None, Some(u)) => format!("unreadable: {u}"),
                    (None, None) => "nothing".to_owned(),
                };
                detail.push(format!(
                    "    report {} {what}{}",
                    if rep.differing.is_empty() {
                        rep.key.clone()
                    } else {
                        format!("{} and {}", rep.key, rep.differing.join(", "))
                    },
                    if rep.findings.is_empty() {
                        String::new()
                    } else {
                        format!(" — malformed: {}", rep.findings.join(", "))
                    }
                ));
            }
            if let Some(a) = &r.author
                && let Some(s) = &a.stamp
            {
                detail.push(format!(
                    "    its author's current version: {} {s}",
                    match a.current {
                        zenkey_fleet::report::DesiredCurrentState::Delete => "delete",
                        _ => "put",
                    }
                ));
            }
            for a in &r.archived {
                detail.push(format!(
                    "    last-known at {}: {}{} ({}) — last-known, never current",
                    a.archive,
                    if a.deleted { "deleted " } else { "" },
                    a.stamp.as_deref().unwrap_or("unstamped"),
                    if a.confirmed {
                        "confirmed by alignment"
                    } else {
                        "not confirmed by alignment"
                    }
                ));
            }
            detail.push(format!("    refused: {}", says(&r.answers.refused)));
            detail.push(format!("    last-known: {}", says(&r.answers.last_known)));
            detail.push(format!("    in force: {}", says(&r.answers.in_force)));
            if let Some(b) = &r.behind {
                detail.push(format!(
                    "    behind {} for at least {:.1}s, by {} (text 0.4)",
                    b.version,
                    b.since_s,
                    match b.by {
                        DesiredBehindBy::Window => "readings the window apart",
                        DesiredBehindBy::Clock => "a clock trusted to the HLC delta",
                    }
                ));
            }
            rows.detail(detail);
        }
        t.grid(rows);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = namespace_words(&self.namespace);
        match (&self.presence, self.across_face) {
            (_, true) => notes.push(
                Note::coverage(
                    "across a constrained face: presence and the descriptor do not cross, and \
                     the deployment's word stands for the target's descriptor; the contracts \
                     are the ones held offline",
                )
                .cite("desired.v1 §2.14, §2.17"),
            ),
            (Some(p), false) => {
                notes.push(
                    Note::coverage(format!(
                        "read {ns} through `{}`: {} service(s) holding an instance token; a \
                         target is one whose descriptor lists a role on a desired resource, its \
                         report found by type identity",
                        p.selector, p.services
                    ))
                    .cite("desired.v1 §2.3, §2.10"),
                );
                if let Some(why) = zenkey_fleet::model::catalog::incomplete_reason(
                    p.complete,
                    p.cut_off_during,
                    "ran to its timeout",
                ) {
                    notes.push(
                        Note::coverage(format!(
                            "the presence read {why}, so it is possibly incomplete: a target or \
                             an author unseen is unobservable, never absent"
                        ))
                        .cite("spec §8.1"),
                    );
                }
            }
            (None, false) => {}
        }
        notes.push(
            Note::coverage(match self.window_s {
                Some(w) => format!(
                    "{} reading(s) over a {w:.1}s window, each S4's GET of the targets' reports \
                     and of each slice from its author: a target \"not yet\" on one version of \
                     its author's for the whole window is behind",
                    self.readings
                ),
                None => "one reading, S4's GET of the targets' reports and of each slice from \
                         its author: \"not yet\" is no verdict, and no target is found behind \
                         without a window (--for)"
                    .to_owned(),
            })
            .cite("desired.v1 §2.17"),
        );
        notes.push(
            Note::caveat(match self.clock {
                ClockGround::DeploymentWord => {
                    "a report's stamp is aged against this host's clock, on the deployment's \
                     word that it and the owners' agree (--clocks-synced)"
                }
                _ => {
                    "no clock was trusted, so a report whose resource declares a horizon has an \
                     unobservable age (--clocks-synced)"
                }
            })
            .cite("freshness.v1 §2.6"),
        );
        if self.rows.iter().any(|r| !r.archived.is_empty()) {
            notes.push(
                Note::caveat(
                    "an archive's last-known of an absent author's slice is shown beside, never \
                     as its author's current document",
                )
                .cite("desired.v1 §2.17, core S6"),
            );
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let count = |a: DesiredAnswerToken| {
            self.rows
                .iter()
                .filter(|r| r.answers.converged.answer == a)
                .count()
        };
        notes.push(Note::summary(format!(
            "{} role(s): converged {} yes, {} no, {} not yet, {} unobservable, {} not asked; {} \
             finding(s).",
            self.rows.len(),
            count(DesiredAnswerToken::Yes),
            count(DesiredAnswerToken::No),
            count(DesiredAnswerToken::NotYet),
            count(DesiredAnswerToken::Unobservable),
            count(DesiredAnswerToken::NotAsked),
            self.rows.iter().filter(|r| r.finding().is_some()).count()
        )));
        notes
    }

    /// What the reading put to the bus, over its window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: self.window_s,
        })
    }
}
