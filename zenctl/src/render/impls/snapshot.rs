//! The two snapshot families (#219, RFC 13 §4.4): what taking one did, and
//! what two disagree about.
//!
//! `snapshot` is row-less like `record` — a file and the counts of what
//! went into it. `snapshot-diff` has rows, five kinds of them, and the
//! envelope carries **both** headers whole because the section's one
//! non-negotiable is that every rendering states both spans.
//!
//! Under `--normalize-origins` (#220) the diff carries three more things
//! and the table draws each: the map, one line per pair with its evidence
//! (`explicit` / `label <source>` / `producer set`); the unpaired, one
//! line per origin with the reason, under a **NOT COMPARED** word when
//! there are any, because the comparison was refused over them; and the
//! per-subject roll-up — "`sysinfo/cpu/usage` differs on 3 of 12
//! origins" — with the subjects that agree everywhere counted, not
//! listed.

use zenkey_fleet::report::{
    AnsweredBy, Holder, KeyChange, MapEvidence, RegistrationWire, Side, SnapshotDiff,
    SnapshotReport, SubjectDelta, VerdictWire, ZsnapHeader,
};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_of,
    envelope_without,
};

impl Render for SnapshotReport {
    const FAMILY: &'static str = "snapshot";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        let h = &self.header;
        t.line(format!(
            "snapshot of {}: {} key(s) — {} live, {} storage-only, {} unattributed{}",
            h.selectors.join(" + "),
            self.live + self.storage_only + self.unattributed,
            self.live,
            self.storage_only,
            self.unattributed,
            self.out
                .as_deref()
                .map(|o| format!(" → {o}"))
                .unwrap_or_default(),
        ));
        t.line(format!(
            "collected over {:.2}s from {} ({} asked, {} answered)",
            h.collection_span_s, h.collected_at, h.asked, h.answered,
        ));
        if !self.incomplete.is_empty() {
            let mut g = Grid::unheaded(2);
            for s in &self.incomplete {
                g.row([
                    Cell::text("  !"),
                    Cell::text(format!("{s}: could not be asked")),
                ]);
            }
            t.grid(g);
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![
            BoundCost::new(
                BoundKind::Refused,
                self.header.elided,
                "repl(y|ies) arrived past the reply bound and were not kept — raise \
                 --max-replies to keep them",
            ),
            BoundCost::new(
                BoundKind::Coalesced,
                self.header.superseded,
                "answer(s) lost last-writer-wins to a newer reply on the same key",
            ),
        ]
    }

    fn notes(&self) -> Vec<Note> {
        let h = &self.header;
        let mut notes = vec![
            Note::caveat(format!(
                "collected over {:.2}s, not at an instant — a fan-in GET has no single moment",
                h.collection_span_s
            ))
            .cite("RFC 13 §4.4"),
        ];
        if h.selectors.iter().any(|s| s.contains("**")) {
            notes.push(
                Note::coverage("`**` cannot cross `@`-planes; they are excluded, not empty")
                    .cite("RFC 03 §4 D2"),
            );
        }
        if h.roster.is_not_asked() {
            notes.push(
                Note::coverage("roster not asked: every holder is unattributed")
                    .cite("RFC 09 §5.1 O4"),
            );
        }
        if h.errors > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} error repl(y|ies) — refusals, counted apart from values and from silence",
                    h.errors
                ))
                .cite("RFC 05 §3"),
            );
        }
        if !self.incomplete.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "{} selector(s) could not be asked at all; the file does not cover them",
                    self.incomplete.len()
                ))
                .cite("RFC 09 §5.1 O5"),
            );
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.header.selectors.clone(),
            window_s: Some(self.header.collection_span_s),
        })
    }
}

impl Render for SnapshotDiff {
    const FAMILY: &'static str = "snapshot-diff";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(
            self,
            &["added", "removed", "changed", "unmapped", "by_subject"],
        )
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for k in &self.added {
            out(Row::tagged("added", serde_json::json!({ "key": k })));
        }
        for k in &self.removed {
            out(Row::tagged("removed", serde_json::json!({ "key": k })));
        }
        for c in &self.changed {
            out(Row::of("changed", c));
        }
        for u in &self.unmapped {
            out(Row::of("unmapped", u));
        }
        if let Some(subjects) = self.by_subject.as_option() {
            for s in subjects {
                out(Row::of("subject", s));
            }
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("a: {}", side(&self.a)));
        t.line(format!("b: {}", side(&self.b)));
        if !self.refused() {
            t.line(format!(
                "{} added, {} removed, {} changed, {} unchanged",
                self.added.len(),
                self.removed.len(),
                self.changed.len(),
                self.unchanged,
            ));
        }
        if !self.added.is_empty() || !self.removed.is_empty() || !self.changed.is_empty() {
            let mut g = Grid::unheaded(3);
            for k in &self.added {
                g.row([Cell::text("  +"), Cell::text(k), Cell::text("")]);
            }
            for k in &self.removed {
                g.row([Cell::text("  -"), Cell::text(k), Cell::text("")]);
            }
            for c in &self.changed {
                g.row([Cell::text("  ~"), Cell::text(&c.key), Cell::text(facets(c))]);
            }
            t.grid(g);
        }
        if let Some(pairs) = self.origin_map.as_option() {
            t.line(format!("origins aligned: {}", pairs.len()));
            if !pairs.is_empty() {
                let mut g = Grid::unheaded(3);
                for p in pairs {
                    g.row([
                        Cell::text("  ="),
                        Cell::text(format!("{} ↔ {}", p.a, p.b)),
                        Cell::text(evidence(&p.evidence)),
                    ]);
                }
                t.grid(g);
            }
        }
        if !self.unmapped.is_empty() {
            t.line(format!("origins not paired: {}", self.unmapped.len()));
            let mut g = Grid::unheaded(3);
            for u in &self.unmapped {
                let side = match u.side {
                    Side::A => "a",
                    Side::B => "b",
                };
                g.row([
                    Cell::text("  ?"),
                    Cell::text(format!("{} (in {side})", u.origin)),
                    Cell::text(&u.reason),
                ]);
            }
            t.grid(g);
        }
        if let Some(subjects) = self.by_subject.as_option() {
            let (differing, agreeing): (Vec<&SubjectDelta>, Vec<&SubjectDelta>) = subjects
                .iter()
                .partition(|s| s.differing > 0 || s.only_in_a > 0 || s.only_in_b > 0);
            if !differing.is_empty() {
                let mut g = Grid::unheaded(3);
                for s in &differing {
                    g.row([
                        Cell::text(&s.subject),
                        Cell::text(subject_line(s)),
                        Cell::text(s.example.as_ref().map(facets).unwrap_or_default()),
                    ]);
                }
                t.grid(g);
            }
            t.line(format!(
                "{} subject(s) identical on every origin, {} not",
                agreeing.len(),
                differing.len()
            ));
        }
        // The word is the carrier; the colour repeats it (#200).
        if self.refused() {
            t.line_styled("NOT COMPARED", crate::render::style::UNPROVEN);
        } else if self.differs() {
            t.line_styled("DIFFERENT", crate::render::style::ERROR);
        } else {
            t.line_styled("IDENTICAL", crate::render::style::PASS);
        }
    }

    fn bounds(&self) -> Vec<BoundCost> {
        vec![BoundCost::new(
            BoundKind::Refused,
            self.truncated,
            "differing key(s) past the listing bound — counted, not listed",
        )]
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![
            Note::caveat(format!(
                "a: span {:.2}s at {}, b: span {:.2}s at {} — each side was collected over its \
                 span, not at an instant",
                self.a.collection_span_s,
                self.a.collected_at,
                self.b.collection_span_s,
                self.b.collected_at,
            ))
            .cite("RFC 13 §4.4"),
        ];
        if self.origin_map.is_not_asked() {
            notes.push(
                Note::coverage(
                    "keys compared verbatim — no origin alignment was asked, so the same host \
                     under a different origin reads as removed and added",
                )
                .cite("RFC 09 §5.1 O4"),
            );
        }
        if let Some(pairs) = self.origin_map.as_option() {
            if self.a.base != self.b.base {
                notes.push(
                    Note::caveat(format!(
                        "b's keys re-based from `{}` onto `{}` for the comparison; the two \
                         deployments' clocks are not compared, so a stamp that moved alone \
                         is not a change here",
                        self.b.base, self.a.base
                    ))
                    .cite("RFC 03 §1.1"),
                );
            }
            let by_set = pairs
                .iter()
                .filter(|p| p.evidence == MapEvidence::ProducerSet)
                .count();
            if by_set > 0 {
                notes.push(
                    Note::coverage(format!(
                        "{by_set} origin(s) paired by producer set alone — no verified label on \
                         both sides; labels ride `state/*/health`, and a snapshot that did not \
                         include it never asked for one"
                    ))
                    .cite("RFC 06 §6.2"),
                );
            }
        }
        if !self.unmapped.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "{} origin(s) could not be paired — listed, never dropped, and the \
                     comparison was not made over them (exit 2); pair them with --map A=B",
                    self.unmapped.len()
                ))
                .cite("RFC 13 §4.4"),
            );
        }
        notes
    }
}

/// The evidence column of the map: what a pair rests on.
fn evidence(e: &MapEvidence) -> String {
    match e {
        MapEvidence::Explicit => "explicit".into(),
        MapEvidence::Label { source } => format!("label `{source}`"),
        MapEvidence::ProducerSet => "producer set".into(),
    }
}

/// One subject's line: "differs on 3 of 12 origins; 1 only in a".
fn subject_line(s: &SubjectDelta) -> String {
    let mut parts = Vec::new();
    if s.differing > 0 {
        parts.push(format!(
            "differs on {} of {} origin(s)",
            s.differing, s.compared
        ));
    } else if s.compared > 0 {
        parts.push(format!("same on {} origin(s)", s.compared));
    }
    if s.only_in_a > 0 {
        parts.push(format!("{} only in a", s.only_in_a));
    }
    if s.only_in_b > 0 {
        parts.push(format!("{} only in b", s.only_in_b));
    }
    parts.join("; ")
}

/// One side's provenance line: rows, span, moment.
fn side(h: &ZsnapHeader) -> String {
    format!(
        "{} key(s), span {:.2}s at {} ({})",
        h.answered.saturating_sub(h.superseded),
        h.collection_span_s,
        h.collected_at,
        h.selectors.join(" + "),
    )
}

/// The facets that moved on one key, each named — so a reader sees *which*
/// of the four changed without opening the row.
fn facets(c: &KeyChange) -> String {
    let mut parts = Vec::new();
    if let Some(v) = &c.value {
        let first = v.changes.first().map(|ch| match ch {
            zenkey_fleet::Change::Changed { path, old, new } => {
                format!(" ({path}: {} → {})", brief(old), brief(new))
            }
            zenkey_fleet::Change::Added { path, .. } => format!(" (+{path})"),
            zenkey_fleet::Change::Removed { path, .. } => format!(" (-{path})"),
        });
        parts.push(format!(
            "value: {} change(s){}{}",
            v.changes.len() + v.truncated,
            first.unwrap_or_default(),
            if v.truncated > 0 {
                format!(", {} not listed", v.truncated)
            } else {
                String::new()
            }
        ));
    }
    if let Some(b) = &c.bytes {
        parts.push(format!(
            "bytes: {} → {} byte(s), differ from offset {}",
            b.old_len, b.new_len, b.common_prefix
        ));
    }
    if let Some((a, b)) = &c.verdict {
        parts.push(format!("verdict: {} → {}", verdict(a), verdict(b)));
    }
    if let Some((a, b)) = &c.registration {
        parts.push(format!(
            "registration: {} → {}",
            registration(*a),
            registration(*b)
        ));
    }
    if let Some((a, b)) = &c.holder {
        parts.push(format!("holder: {} → {}", holder(a), holder(b)));
    }
    if parts.is_empty() && c.timestamp.0 != c.timestamp.1 {
        parts.push("re-stamped, same value".into());
    }
    parts.join("; ")
}

fn brief(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 24 {
        let mut cut: String = s.chars().take(23).collect();
        cut.push('…');
        cut
    } else {
        s
    }
}

fn verdict(v: &VerdictWire) -> String {
    match v {
        VerdictWire::Valid => "valid".into(),
        VerdictWire::Invalid { violations } => format!("invalid({})", violations.len()),
        VerdictWire::NotValidated { reason } => format!("not_validated:{reason}"),
    }
}

fn registration(r: RegistrationWire) -> &'static str {
    match r {
        RegistrationWire::Registered => "registered",
        RegistrationWire::Unregistered => "unregistered",
        RegistrationWire::NoSliceForProducer => "no_slice_for_producer",
        RegistrationWire::NotADataClass => "not_a_data_class",
        RegistrationWire::NotV1 => "not_v1",
        RegistrationWire::NotUnderBase => "not_under_base",
        RegistrationWire::RegistryNotLoaded => "registry_not_loaded",
    }
}

fn holder(h: &Holder) -> String {
    match h {
        Holder::Live { answered_by, .. } => format!(
            "live({})",
            match answered_by {
                AnsweredBy::Stamper => "stamper",
                AnsweredBy::Other => "other",
                AnsweredBy::Unknown => "unknown",
            }
        ),
        Holder::StorageOnly { .. } => "storage_only".into(),
        Holder::Unattributed { .. } => "unattributed".into(),
    }
}
