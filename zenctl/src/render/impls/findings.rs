//! The families whose rows are *findings*: doctor, conform, check retired.
//!
//! What they share is the thing this seam is for — an empty result means
//! something, and it never means "nothing is wrong": `doctor` with no findings
//! has to say what it checked, and an entry whose wire was not listened to
//! is not a silent one.

use zenkey_fleet::report::{
    AssertionState, ConformReport, ConformVerdict, CutoverVerdict, DoctorSeverity, RetiredEntry,
    RetiredReport, V1DoctorReport,
};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

impl Render for V1DoctorReport {
    const FAMILY: &'static str = "doctor";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // Everything except the findings themselves, so an empty findings list
        // stays legible: what was checked, not just what was found.
        envelope_without(self, &["findings"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for f in &self.findings {
            out(Row::of("finding", f));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(2);
        for s in self.synced.as_deref().into_iter().flatten() {
            grid.row([
                Cell::styled("✓", crate::render::style::PASS),
                Cell::text(format!("{s}: in sync")),
            ]);
        }
        for f in &self.findings {
            let mark = crate::render::style::mark(f.severity);
            let citation = f
                .citation
                .as_deref()
                .map(|c| format!("  [{c}]"))
                .unwrap_or_default();
            grid.row([
                // The mark already says which severity it is; colour only
                // repeats it, so stripping the escape loses nothing (#200).
                Cell::styled(mark, crate::render::style::severity(f.severity)),
                Cell::text(format!(
                    "{}: {} — {}{citation}",
                    f.check, f.subject, f.evidence
                )),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // R1: the degradation used to be a bare eprintln in `cmd/doctor.rs`,
        // invisible to `--format json` — a machine consumer read "no synced
        // slices" where the truth was "the diff never ran".
        if self.synced.is_not_asked() {
            notes.push(
                Note::coverage(
                    "no local registry given — the served-vs-declared diff never ran; \
                     only bus-derived checks did, and \"not checked\" is not \"in sync\"",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        // The listen phase's scope statement (#161): what was watched, for
        // how long, and what rode. Its costs — drops, evictions, refusals —
        // are declared in `bounds()` and the emit path writes them.
        if let Some(obs) = &self.observation {
            let mut text = format!(
                "listened {:.0}s over {} scope(s): {} sample(s) on {} key(s), {} dropped",
                obs.window_s,
                obs.scopes.len(),
                obs.samples,
                obs.keys_seen,
                obs.dropped
            );
            if obs.synthetic_marked > 0 {
                text.push_str(&format!(
                    "; {} sample(s) carried the synthetic marker",
                    obs.synthetic_marked
                ));
            }
            notes.push(Note::coverage(text).cite("RFC 09 §5.1 O5"));
        }
        notes.push(Note::coverage(format!(
            "{} introspect repl(y|ies) from {} live producer(s); {} producer(s) serve \
             describe, {} do not; {} router(s){}",
            self.introspect_answered,
            self.live_producers,
            self.describe_served,
            self.describe_missing,
            self.routers,
            self.router_version
                .as_deref()
                .map(|v| format!(" (version {v})"))
                .unwrap_or_default(),
        )));
        // #510: the empty scope, stated in every format — the silence note
        // reaches `--format json` too, and the summary below stops calling
        // an empty bus a fleet that agrees with this build.
        if let Some(why) = &self.unobservable {
            notes.push(Note::silence(why.clone()));
        }
        let errors = self.count(DoctorSeverity::Error);
        let warnings = self.count(DoctorSeverity::Warning);
        notes.push(if self.unobservable.is_some() {
            Note::summary("nothing judged — no verdict on the fleet.")
        } else if errors == 0 && warnings == 0 {
            Note::summary("no findings — the fleet agrees with this build.")
        } else {
            Note::summary(format!(
                "{} finding(s): {errors} error(s), {warnings} warning(s), {} info.",
                self.findings.len(),
                self.count(DoctorSeverity::Info)
            ))
        });
        notes
    }

    /// The listen phase's costs (#161, #107, #223) — migrated from clauses
    /// inside a composite note onto the closed O6 vocabulary.
    fn bounds(&self) -> Vec<BoundCost> {
        let Some(obs) = &self.observation else {
            return Vec::new();
        };
        vec![
            BoundCost::new(
                BoundKind::Missed,
                obs.dropped,
                "sample(s) dropped while behind during the listen phase — \
                 findings cover only what was seen",
            ),
            BoundCost::new(
                BoundKind::Retired,
                obs.facts_evicted,
                "key projection(s) retired by the bounded facts cache — key \
                 population figures cover the retained keys only",
            ),
            BoundCost::new(
                BoundKind::Refused,
                obs.field_paths_dropped,
                "path observation(s) refused at the field path table's bound",
            ),
        ]
    }

    /// The listen phase is the one subscription this report carries; the
    /// control-plane sweeps state their coverage in `notes()`.
    fn scope(&self) -> Option<ObservedScope> {
        self.observation.as_ref().map(|obs| ObservedScope {
            asked: obs.scopes.clone(),
            window_s: Some(obs.window_s),
        })
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
