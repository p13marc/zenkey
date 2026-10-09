//! The families whose rows are *findings*: doctor, conform, check retired.
//!
//! What they share is the thing this seam is for — an empty result means
//! something, and it never means "nothing is wrong": `doctor` with no findings
//! has to say what it checked, and an entry whose wire was not listened to
//! is not a silent one.

use zenkey_fleet::report::{
    AssertionState, CheckReport, ConformReport, ConformVerdict, CutoverVerdict, DoctorReport,
    DoctorSeverity, Judgement, RetiredEntry, RetiredReport,
};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

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

/// The conformance suite (#222): one row per assertion, three states and
/// the exemption kept apart in every medium — the word carries the state,
/// the mark repeats it, and `unknowable` is dim rather than red because it
/// is the absence of a verdict, not a milder failure (RFC 13 §3).
impl Render for ConformReport {
    const FAMILY: &'static str = "conform";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything but the assertions: who was asked, the summary, the
        // verdict and what was not asked survive a truncated pipe.
        envelope_without(self, &["assertions"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for a in &self.assertions {
            out(Row::of("assertion", a));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3);
        for a in &self.assertions {
            let state = match (&a.state, a.exempt.is_some()) {
                (AssertionState::Met, false) => Cell::styled("✓ met", crate::render::style::PASS),
                (AssertionState::Met, true) => Cell::styled("✓ exempt", crate::render::style::PASS),
                (AssertionState::NotMet, _) => {
                    Cell::styled("✗ not met", crate::render::style::ERROR)
                }
                (AssertionState::Unknowable { .. }, _) => {
                    Cell::styled("? unknowable", crate::render::style::UNPROVEN)
                }
            };
            let mut evidence = a.evidence.clone();
            if let AssertionState::Unknowable { reason } = &a.state {
                evidence = format!("{reason} — {evidence}");
            }
            if let Some(e) = &a.exempt {
                evidence = format!("{e} — {evidence}");
            }
            if let Some(c) = &a.citation {
                evidence.push_str(&format!("  [{c}]"));
            }
            grid.row([state, Cell::text(a.id.clone()), Cell::text(evidence)]);
        }
        t.grid(grid);
        let (word, style) = match self.verdict {
            ConformVerdict::Conforms => ("CONFORMS", crate::render::style::PASS),
            ConformVerdict::Violates => ("VIOLATES", crate::render::style::ERROR),
            ConformVerdict::Unproven => ("UNPROVEN", crate::render::style::UNPROVEN),
        };
        t.line_styled(word, style);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        notes.push(
            Note::coverage(if self.origins_asked.is_empty() {
                format!(
                    "no origin of {} was on the roster, and no --origin named one — \
                     nothing was called, and every run-time assertion says so",
                    self.producer
                )
            } else {
                format!(
                    "called {} on {} origin(s): {}",
                    self.producer,
                    self.origins_asked.len(),
                    self.origins_asked.join(", ")
                )
            })
            .cite("RFC 13 §2"),
        );
        if let Some(obs) = &self.observation {
            notes.push(
                Note::coverage(format!(
                    "listened {:.0}s over {} scope(s): {} sample(s) on {} key(s) — a \
                     window proves presence, never absence",
                    obs.window_s,
                    obs.scopes.len(),
                    obs.samples,
                    obs.keys_seen
                ))
                .cite("RFC 09 §5.1 O5"),
            );
            if obs.synthetic_marked > 0 {
                notes.push(Note::caveat(format!(
                    "{} sample(s) carried the synthetic marker — generated traffic, \
                     judged like any other (RFC 09 §5.3)",
                    obs.synthetic_marked
                )));
            }
        }
        for n in &self.not_asked {
            notes.push(Note::coverage(format!("not asked: {n}")).cite("RFC 09 §5.1 O4"));
        }
        let s = &self.summary;
        notes.push(Note::summary(format!(
            "{} assertion(s): {} met ({} exempt), {} not met, {} unknowable — \
             unknowable is skipped, never failed",
            self.assertions.len(),
            s.met,
            s.exempt,
            s.not_met,
            s.unknowable
        )));
        notes
    }

    fn bounds(&self) -> Vec<BoundCost> {
        let Some(obs) = &self.observation else {
            return Vec::new();
        };
        vec![
            BoundCost::new(
                BoundKind::Missed,
                obs.dropped,
                "sample(s) dropped while behind during the window — every observed \
                 count is a lower bound",
            ),
            BoundCost::new(
                BoundKind::Retired,
                obs.facts_evicted,
                "key projection(s) retired by the bounded facts cache — presence is \
                 judged over the retained keys only",
            ),
        ]
    }

    /// The probes and the window: every origin's `@rpc` plane that was
    /// called, and the listen phase's selectors when one ran.
    fn scope(&self) -> Option<ObservedScope> {
        let mut asked: Vec<String> = self
            .origins_asked
            .iter()
            .map(|o| format!("{o}/@rpc/{}", self.producer))
            .collect();
        if let Some(obs) = &self.observation {
            asked.extend(obs.scopes.iter().cloned());
        }
        Some(ObservedScope {
            asked,
            window_s: self.observation.as_ref().map(|o| o.window_s),
        })
    }
}

/// One entry's four facts as prose cells, each honest about whether it was
/// obtainable at all: "not listened" is not silence, "no admin space" is not
/// zero subscribers (RFC 09 §5.1 O4).
fn retired_facts(e: &RetiredEntry) -> String {
    let wire = match e.wire_samples.get() {
        None => "wire not listened".to_string(),
        Some(0) => "wire silent".to_string(),
        Some(n) => format!("wire {n} sample(s)"),
    };
    let served = match e.still_declared {
        None => "no served slice".to_string(),
        Some(true) => "STILL SERVED".to_string(),
        Some(false) => "not served".to_string(),
    };
    let subs = match e.subscribers {
        None => "subscribers unknown".to_string(),
        Some(n) => format!("{n} subscriber(s)"),
    };
    let replacement = match (&e.replaced_by, e.replacement_samples.get()) {
        (None, _) => "no replacement declared".to_string(),
        (Some(p), None) => format!("→ {p}: not listened"),
        (Some(p), Some(n)) => format!("→ {p}: {n} sample(s)"),
    };
    format!("{wire} · {served} · {subs} · {replacement}")
}

impl Render for RetiredReport {
    const FAMILY: &'static str = "registry-retired";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the entries themselves — the coverage claim (which
        // registries were read, what listened, what answered) must survive a
        // truncated pipe.
        let mut e = envelope_without(self, &["entries"]);
        e.insert("entries".into(), self.entries.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for e in &self.entries {
            out(Row::of("entry", e));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(3).max(1, 40);
        for e in &self.entries {
            // The word is the carrier; the mark repeats it (#200).
            let mark = match e.verdict {
                CutoverVerdict::Pass => Cell::styled("✓", crate::render::style::PASS),
                CutoverVerdict::OldStillSpeaks => Cell::styled("✗", crate::render::style::ERROR),
                CutoverVerdict::Unproven => Cell::styled("?", crate::render::style::UNPROVEN),
            };
            grid.row([
                mark,
                Cell::text(format!("{}: {}", e.producer, e.path)),
                Cell::text(retired_facts(e)),
            ]);
        }
        t.grid(grid);
        let (word, style) = match self.verdict {
            CutoverVerdict::Pass => ("PASS", crate::render::style::PASS),
            CutoverVerdict::OldStillSpeaks => ("FAIL", crate::render::style::ERROR),
            // Dim, not yellow: the absence of a verdict, not a milder failure.
            CutoverVerdict::Unproven => ("UNPROVEN", crate::render::style::UNPROVEN),
        };
        t.line_styled(word, style);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // The coverage claim is exactly the files that were read: a
        // `--registry` dir may be one team's slice of the fleet's ledger, and
        // this report must never read as fleet totality.
        notes.push(
            Note::coverage(format!(
                "ledger: {} entr(y|ies) from {} — coverage is exactly these files, \
                 which may be one checkout's slice of the fleet's ledger",
                self.entries.len(),
                self.registries.join(", "),
            ))
            .cite("RFC 09 §5.1 O5"),
        );
        match (self.window_s.get(), self.plane_samples.get()) {
            (Some(w), Some(p)) => notes.push(Note::coverage(format!(
                "listened {w}s: {p} sample(s) on the v1 plane — the proof-of-life \
                 half for entries with no declared replacement"
            ))),
            _ => notes.push(
                Note::coverage(
                    "no listen window ran — wire facts read \"not listened\", never \
                     \"absent\"; pass --for <SECS> to observe the retired families",
                )
                .cite("RFC 09 §5.1 O4"),
            ),
        }
        notes.push(match self.admin_entities {
            Some(n) => Note::coverage(format!(
                "{} served slice(s) answered introspect; {} declared entit(y|ies) \
                 from the admin space",
                self.introspect_answered, n
            )),
            None => Note::coverage(format!(
                "{} served slice(s) answered introspect; no admin space answered — \
                 declared subscribers are unknown, not zero",
                self.introspect_answered
            ))
            .cite("RFC 09 §5.1 O4"),
        });
        if self.entries.is_empty() {
            notes.push(Note::summary(
                "the ledger declares no [[deprecated]] entries — nothing retired, \
                 nothing to burn down.",
            ));
        }
        notes.push(match self.verdict {
            CutoverVerdict::Pass => Note::coverage(
                "every ledger entry is silent while its replacement (or the v1 \
                 plane) carries traffic — the burn-down holds",
            )
            .cite("RFC 08 §3"),
            CutoverVerdict::OldStillSpeaks => Note::coverage(
                "a retired subject still shows life — on the wire, in a served \
                 introspect slice (the registry MUST NOT lie), or in a declared \
                 subscriber",
            )
            .cite("RFC 08 §6.1"),
            CutoverVerdict::Unproven => Note::silence(
                "at least one entry has no positive evidence either way — a silent \
                 replacement, or no listen window: silence is not a pass",
            ),
        });
        notes
    }

    /// R6, restated as data: `dropped` rides only when a window ran (it is
    /// `NotAsked` otherwise), so an unlistened run declares a zero cost and
    /// no note claims a clean observation nobody made.
    fn bounds(&self) -> Vec<BoundCost> {
        vec![BoundCost::new(
            BoundKind::Missed,
            self.dropped.get().unwrap_or(0),
            "sample(s) dropped while behind — every silence claim covers only \
             what was seen",
        )]
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.entries.iter().map(|e| e.selector.clone()).collect(),
            window_s: self.window_s.get(),
        })
    }
}
