//! The family whose rows are *findings*: doctor.
//!
//! What this seam is for: an empty result means something, and it never
//! means "nothing is wrong" — `doctor` with no findings has to say what it
//! checked.

use zenkey_fleet::report::{CheckReport, DoctorReport, DoctorSeverity, Judgement};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

/// zk2's doctor (#612, FJ6): one row per check, its verdict in the
/// judgement shape. Each pole has its own word and its own mark in the
/// table — `✗`/`⚠`/`·` a finding by its worst severity, `✓ clean`,
/// `? unobservable`, `— not asked` — and its own `answer` in a row, so a
/// script branches on the answer and never on the prose. A finding is the
/// yes of every check's question (tooling guide §1).
impl Render for DoctorReport {
    const FAMILY: &'static str = "doctor";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // The scope and the empty-scope reason, so a document whose checks
        // were cut off still says what it read.
        envelope_without(self, &["checks"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for c in &self.checks {
            out(Row::of("check", c));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3);
        for c in &self.checks {
            let (mark, style, word) = verdict_cell(c);
            // The lists ride the detail lines below; the line itself says
            // the pole and how many subjects, or the reason when there are
            // none to list.
            let unjudged = match c.unjudged.len() {
                0 => String::new(),
                n => format!(", {n} unjudged"),
            };
            let what = match &c.verdict {
                Judgement::Established => {
                    format!("{word} — {} subject(s){unjudged}", c.findings.len())
                }
                Judgement::NotEstablished { reason } => format!("{word} — {reason}"),
                Judgement::Unobservable { .. } if !c.unjudged.is_empty() => {
                    format!("{word} — {} subject(s) unjudged", c.unjudged.len())
                }
                Judgement::Unobservable { reason } => format!("{word} — {reason}"),
                Judgement::NotAsked => word.to_owned(),
            };
            grid.row([
                // The mark and the word both say the pole; colour only
                // repeats it, so stripping the escape loses nothing (#200).
                Cell::styled(mark, style),
                Cell::text(format!("{} ({})", c.check, c.section)),
                Cell::text(what),
            ]);
            let findings = c.findings.iter().map(|f| {
                format!(
                    "    {} {}: {} — {}",
                    crate::render::style::mark(f.severity),
                    f.severity.as_str(),
                    f.subject,
                    f.evidence
                )
            });
            let unjudged = c
                .unjudged
                .iter()
                .map(|u| format!("    ? unjudged {}: {}", u.subject, u.reason));
            let detail: Vec<String> = findings.chain(unjudged).collect();
            if !detail.is_empty() {
                grid.detail(detail);
            }
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        let ns = if self.scope.namespace.is_empty() {
            "the bus root".to_owned()
        } else {
            format!("namespace {:?}", self.scope.namespace)
        };
        if let Some(p) = self.scope.presence.as_option() {
            notes.push(
                Note::coverage(format!(
                    "read {ns} through `{}`, twice, {:.1}s apart: {} service(s), {} \
                     instance(s), {} token(s), {} instance(s) with no readable descriptor; \
                     {} of {} revision(s) named retrieved and verified",
                    p.selector,
                    p.grace_s,
                    p.services,
                    p.instances,
                    p.tokens,
                    p.undescribed,
                    p.held,
                    p.revisions
                ))
                .cite("spec §8.1"),
            );
            if !p.complete {
                notes.push(
                    Note::coverage(
                        "a presence read ran to its timeout, so it is possibly incomplete: \
                         an absence it would claim is left unjudged — raise --timeout to \
                         ask again",
                    )
                    .cite("spec §8.1"),
                );
            }
        }
        if let Some(n) = self.scope.routers.get() {
            notes.push(Note::coverage(format!(
                "{n} router(s) answered the admin space, read in no namespace"
            )));
        }
        let not_asked: Vec<String> = self
            .checks
            .iter()
            .filter(|c| c.verdict.is_not_asked())
            .map(|c| match c.check {
                zenkey_fleet::report::CheckId::StateStampForeign => {
                    format!("{} (pass --deep)", c.check)
                }
                _ => c.check.to_string(),
            })
            .collect();
        if !not_asked.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "not asked: {} — not asked is neither clean nor a finding",
                    not_asked.join(", ")
                ))
                .cite("tooling guide O4"),
            );
        }
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let count = |s| self.count(s);
        let unobservable = self
            .checks
            .iter()
            .filter(|c| c.verdict.is_unobservable())
            .count();
        let clean = self
            .checks
            .iter()
            .filter(|c| matches!(c.verdict, Judgement::NotEstablished { .. }))
            .count();
        notes.push(Note::summary(if self.unobservable.is_some() {
            "nothing in scope — no verdict on the deployment.".to_owned()
        } else {
            format!(
                "{} finding(s): {} error(s), {} warning(s), {} info; {clean} check(s) clean, \
                 {unobservable} unobservable.",
                self.findings().count(),
                count(DoctorSeverity::Error),
                count(DoctorSeverity::Warning),
                count(DoctorSeverity::Info),
            )
        }));
        notes
    }

    /// What the run put to the bus: the presence selector in the namespace,
    /// and the admin space in none.
    fn scope(&self) -> Option<ObservedScope> {
        let mut asked = Vec::new();
        if let Some(p) = self.scope.presence.as_option() {
            asked.push(p.selector.clone());
        }
        if self.scope.routers.is_asked() {
            asked.push("@/*/router".to_owned());
        }
        Some(ObservedScope {
            asked,
            window_s: None,
        })
    }
}

/// zk2's `why` (#702): one row per rung, in ladder order, each pole its
/// own mark and word — `✗ cause`, `✓ healthy`, `? unobservable`, `— not
/// asked` — and its own `answer` in a row, so a script branches on the
/// rung and the answer, never on the prose. A cause is the yes of every
/// rung's question (tooling guide §1), and the finding of the run.
impl Render for zenkey_fleet::WhyReport {
    const FAMILY: &'static str = "why";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // The verdict, the stop and what was asked lead, so a document cut
        // short still says what the ladder established.
        envelope_without(self, &["rungs"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rungs {
            out(Row::of("rung", r));
        }
    }

    fn table(&self, t: &mut Table) {
        use crate::render::style;
        let headline = match (&self.verdict, self.stopped_at) {
            (Judgement::Established, Some(at)) => format!("a cause, at {at}"),
            (Judgement::NotEstablished { .. }, _) => "every rung healthy".to_owned(),
            (Judgement::Unobservable { .. }, Some(at)) => format!("no verdict, at {at}"),
            (Judgement::NotAsked, _) => "no verdict: the answer was not asked".to_owned(),
            _ => "no verdict".to_owned(),
        };
        t.line(format!("why {} — {headline}", self.target));
        let mut grid = Grid::unheaded(3);
        for r in &self.rungs {
            let (mark, st, word) = match &r.verdict {
                Judgement::Established => ("✗", style::severity(DoctorSeverity::Error), "cause"),
                Judgement::NotEstablished { .. } => ("✓", style::PASS, "healthy"),
                Judgement::Unobservable { .. } => ("?", style::UNPROVEN, "unobservable"),
                Judgement::NotAsked => ("—", style::UNPROVEN, "not asked"),
            };
            let what = match (&r.verdict, &r.cause) {
                (Judgement::Established, Some(c)) => format!("{word} — {c}"),
                (Judgement::NotEstablished { reason }, _)
                | (Judgement::Unobservable { reason }, _) => format!("{word} — {reason}"),
                _ => word.to_owned(),
            };
            grid.row([
                Cell::styled(mark, st),
                Cell::text(format!("{} ({})", r.rung, r.section)),
                Cell::text(what),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if let Some(lk) = &self.last_known {
            notes.push(
                Note::caveat(format!(
                    "{} still holds a last-known value of {} ({}): last-known, never current — \
                     the owner's silence is not explained by it",
                    lk.archive,
                    lk.key,
                    if lk.confirmed {
                        "confirmed by alignment"
                    } else {
                        "not confirmed by alignment"
                    }
                ))
                .cite("spec §4.2 S6"),
            );
        }
        if self.verdict.is_not_asked() {
            notes.push(Note::coverage(
                "an operation's answer is not asked: why calls nothing — `zenctl call` asks it, \
                 and its silence is attributed through presence",
            ));
        }
        let not_asked = self
            .rungs
            .iter()
            .filter(|r| r.verdict.is_not_asked())
            .count();
        notes.push(Note::summary(match (&self.verdict, self.cause()) {
            (Judgement::Established, Some((rung, _))) => format!(
                "a cause at {rung}: the ladder stopped there, {not_asked} rung(s) not asked"
            ),
            (Judgement::NotEstablished { .. }, _) => {
                "every rung is healthy, and the key answers.".to_owned()
            }
            _ => {
                "no verdict — a rung could not be observed, or the answer was not asked.".to_owned()
            }
        }));
        notes
    }

    /// What the ladder put to the bus: presence, the key, the archives —
    /// and over what window, when it listened to a stream.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: self.window_s,
        })
    }
}

/// A check's pole as the table spells it: the mark, its style, and the word.
fn verdict_cell(c: &CheckReport) -> (&'static str, anstyle::Style, &'static str) {
    use crate::render::style;
    match &c.verdict {
        Judgement::Established => {
            let worst = c
                .findings
                .iter()
                .map(|f| f.severity)
                .find(|s| *s == DoctorSeverity::Error)
                .or_else(|| {
                    c.findings
                        .iter()
                        .map(|f| f.severity)
                        .find(|s| *s == DoctorSeverity::Warning)
                })
                .unwrap_or(DoctorSeverity::Info);
            (style::mark(worst), style::severity(worst), "finding")
        }
        Judgement::NotEstablished { .. } => ("✓", style::PASS, "clean"),
        Judgement::Unobservable { .. } => ("?", style::UNPROVEN, "unobservable"),
        Judgement::NotAsked => ("—", style::UNPROVEN, "not asked"),
    }
}
