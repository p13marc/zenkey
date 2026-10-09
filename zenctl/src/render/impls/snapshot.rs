//! The two snapshot families (#219, RFC 13 §4.4; zk2's since #612, FJ8b):
//! what taking one did, and what two disagree about.
//!
//! `snapshot` is row-less like `record` — a file and the counts of what
//! went into it, holders and conformance among them. `snapshot-diff` has
//! rows, three kinds of them, and the envelope carries **both** headers
//! whole because the section's one non-negotiable is that every rendering
//! states both spans. Keys are zk2 keys, each relative to its own file's
//! namespace.

use zenkey_fleet::report::{
    AnsweredBy, Conformance, Holder, KeyChange, SnapshotDiff, SnapshotReport, ZsnapHeader,
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
            "snapshot of {}: {} key(s) — {} live, {} with no instance, {} unattributed; {} \
             not conforming to their type{}",
            h.selectors.join(" + "),
            self.live + self.no_instance + self.unattributed,
            self.live,
            self.no_instance,
            self.unattributed,
            self.nonconforming,
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
                g.row([Cell::text("  !"), Cell::text(format!("{s}: not asked"))]);
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
                "answer(s) for one key lost to a newer one from another selector",
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
        let excluded = zenkey_fleet::zrec_excluded(&h.selectors);
        if excluded.iter().any(|v| v == "@state") {
            notes.push(
                Note::coverage(
                    "@state keys are excluded, not empty: no selector names `@state`, and \
                     `*`/`**` never match a verbatim chunk",
                )
                .cite("tooling guide O5"),
            );
        }
        if h.presence.is_none() {
            notes.push(
                Note::coverage(
                    "presence not read: every holder is unattributed and every stamp \
                     unattributable",
                )
                .cite("tooling guide O4"),
            );
        }
        if h.discarded > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} repl(y|ies) on a wildcard key discarded by rule — not losses",
                    h.discarded
                ))
                .cite("spec §3.2 R6"),
            );
        }
        if h.errors > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} error repl(y|ies) — refusals, counted apart from values and from silence",
                    h.errors
                ))
                .cite("spec §5.2"),
            );
        }
        if self.no_instance > 0 {
            notes.push(
                Note::coverage(format!(
                    "{} value(s) answered for an address with no instance visible to this \
                     reader: an owner gone, or a store answering on its keys, which S4 \
                     forbids",
                    self.no_instance
                ))
                .cite("spec §4.2 S4"),
            );
        }
        if !self.incomplete.is_empty() {
            notes.push(
                Note::coverage(format!(
                    "{} selector(s) were not asked — no state key, or the GET could not be \
                     issued; the file does not cover them",
                    self.incomplete.len()
                ))
                .cite("tooling guide O5"),
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
        envelope_without(self, &["added", "removed", "changed"])
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
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("a: {}", side(&self.a)));
        t.line(format!("b: {}", side(&self.b)));
        t.line(format!(
            "{} added, {} removed, {} changed, {} unchanged",
            self.added.len(),
            self.removed.len(),
            self.changed.len(),
            self.unchanged,
        ));
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
        // The word is the carrier; the colour repeats it (#200).
        if self.differs() {
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
        if self.a.base != self.b.base {
            notes.push(
                Note::caveat(format!(
                    "keys compared relative to each file's namespace (a: {:?}, b: {:?}); two \
                     deployments' clocks are not compared, so a stamp that moved alone is not \
                     a change here",
                    self.a.base, self.b.base
                ))
                .cite("spec §1.6"),
            );
        }
        notes
    }
}

/// One side's provenance line: rows, span, moment.
fn side(h: &ZsnapHeader) -> String {
    format!(
        "{} key(s), span {:.2}s at {} in {} ({})",
        h.answered.saturating_sub(h.superseded),
        h.collection_span_s,
        h.collected_at,
        if h.base.is_empty() {
            "the bus root".to_owned()
        } else {
            format!("namespace {:?}", h.base)
        },
        h.selectors.join(" + "),
    )
}

/// The facets that moved on one key, each named — so a reader sees *which*
/// of them changed without opening the row.
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
    if let Some((a, b)) = &c.conformance {
        parts.push(format!(
            "conformance: {} → {}",
            conformance(a),
            conformance(b)
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

fn conformance(c: &Conformance) -> String {
    match c {
        Conformance::Valid => "valid".into(),
        Conformance::Invalid { violations } => format!("invalid({})", violations.len()),
        Conformance::Undecodable { .. } => "undecodable".into(),
        Conformance::NotChecked { .. } => "not_checked".into(),
    }
}

fn holder(h: &Holder) -> String {
    match h {
        Holder::Live { answered_by, .. } => format!(
            "live({})",
            match answered_by {
                AnsweredBy::Owner => "owner",
                AnsweredBy::Other => "other",
                AnsweredBy::Unknown => "unknown",
            }
        ),
        Holder::NoInstance { .. } => "no_instance".into(),
        Holder::Unattributed { .. } => "unattributed".into(),
    }
}
