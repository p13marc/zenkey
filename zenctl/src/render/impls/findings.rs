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
