//! The consumers join and the blast radius (#224): two families whose
//! honesty *is* the product.
//!
//! What they draw is a join over the admin space — declared subscribers and
//! queriers related to a target by key algebra, attributed to sessions and
//! origins on evidence. What they must never draw is matching status:
//! RFC 12 §9 defers foreign matching permanently, and the standing false
//! verdict it records is "nobody is listening". So the vocabulary here is
//! *declared*, *relates*, *intersects* — never "listening", "matching" or
//! "unmatched", and an empty result is never "no consumers". The render
//! corpus greps both fixtures for those words.

use zenkey_fleet::report::{
    AdminAnswer, Attribution, ConsumerRow, ConsumersReport, Relation, SubjectImpact,
};

use crate::render::{
    BoundCost, BoundKind, Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without,
};

/// The relation column's words. "Hears a subset" and "declared over a
/// superset" are set relations between two key expressions, which is all a
/// declaration supports.
fn relation_words(relation: Relation) -> &'static str {
    match relation {
        Relation::Exact => "exact — declared on the target itself",
        Relation::Narrower => "narrower — declared on a subset of the target",
        Relation::Wider => "wider — declared over a superset of the target",
        Relation::Intersects => "intersects — overlaps the target in part",
        Relation::Total => "total — a whole-base declaration, intersects everything",
    }
}

/// The origin column: the attached origins, or the honest absence.
fn origin_cell(row: &ConsumerRow) -> Cell {
    if !row.origins.is_empty() {
        return Cell::text(row.origins.join(" "));
    }
    match row.attribution {
        Attribution::Session => Cell::text("session only, unattributed"),
        // The zid is the reporter's, not a session's: the declaration is
        // known to exist behind that admin space and no more.
        Attribution::ReportedOnly => Cell::text("reported only — no session named"),
    }
}

fn consumer_grid(report: &ConsumersReport) -> Grid {
    let mut g = Grid::new(["zid", "whatami", "origin", "declared", "relation"]);
    for row in &report.rows {
        let zid = if row.is_self {
            format!("{}  (this zenctl session)", row.zid)
        } else {
            row.zid.clone()
        };
        g.row([
            Cell::text(zid),
            // The topology heard of the zid or it did not; not heard of is
            // not "no kind" (O4).
            Cell::asked(row.whatami.clone()),
            origin_cell(row),
            Cell::text(format!("{} {}", kind_word(row), row.keyexpr)),
            Cell::text(relation_words(row.relation)),
        ]);
    }
    g
}

fn kind_word(row: &ConsumerRow) -> &'static str {
    match row.kind {
        zenkey_fleet::EntityKind::Subscriber => "subscriber",
        zenkey_fleet::EntityKind::Querier => "querier",
        zenkey_fleet::EntityKind::Publisher => "publisher",
        zenkey_fleet::EntityKind::Queryable => "queryable",
        zenkey_fleet::EntityKind::Token => "token",
    }
}

/// The notes both families share: the admin answer, what a row is
/// evidence of, and what a total wildcard reaches.
fn consumer_notes(report: &ConsumersReport) -> Vec<Note> {
    let mut notes = Vec::new();
    match report.admin {
        AdminAnswer::NotAvailable => notes.push(
            Note::coverage(format!(
                "no admin space answered {} — zenoh's `adminspace.enabled` is off by \
                 default (routers ship with it on); the declared readers of {} are \
                 *not asked*, never none",
                report.asked.join(", "),
                report.target
            ))
            .cite("RFC 13 §3 O4"),
        ),
        AdminAnswer::Answered { answered, nodes } => {
            notes.push(
                Note::coverage(format!(
                    "{answered} admin space(s) answered ({nodes} node(s) heard of): a \
                     declared subscriber or querier is a declaration, not proof of use, \
                     and sessions behind an admin space that did not answer are not shown"
                ))
                .cite("RFC 13 §3 O5"),
            );
            if report.rows.is_empty() {
                notes.push(Note::silence(format!(
                    "nothing declared in the answering admin space(s) relates to {} — a \
                     reading of what was declared there, not a verdict about who reads it",
                    report.target
                )));
            }
        }
    }
    if report.rows.iter().any(|r| r.total_wildcard) {
        notes.push(
            Note::caveat(
                "a whole-base declaration (`**`) intersects every key under the base and \
                 says nothing about this subject in particular; it still never crosses \
                 an `@`-chunk, so `@rpc`/`@media`/`@blob` sidecars and service origins \
                 are outside it",
            )
            .cite("RFC 03 §4 D2"),
        );
    }
    notes
}

fn consumer_bounds(report: &ConsumersReport) -> Vec<BoundCost> {
    vec![BoundCost::new(
        BoundKind::Refused,
        report.reply_elided,
        "admin reply(ies) past the bound not kept — a declaration among them is a reader \
         this report cannot show",
    )]
}

impl Render for ConsumersReport {
    const FAMILY: &'static str = "registry-consumers";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_without(self, &["rows"])
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.rows {
            out(Row::of("consumer", r));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("declared readers of {}", self.target));
        if !self.rows.is_empty() {
            t.blank().grid(consumer_grid(self));
        }
    }

    fn notes(&self) -> Vec<Note> {
        consumer_notes(self)
    }

    fn bounds(&self) -> Vec<BoundCost> {
        consumer_bounds(self)
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: None,
        })
    }
}

impl Render for SubjectImpact {
    const FAMILY: &'static str = "registry-impact";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_without(self, &["consumers", "coverage"]);
        // The nested document's facts ride flat: what was asked, who
        // answered, who we are — a script reads one envelope.
        for (k, v) in envelope_without(&self.consumers, &["rows", "target"]) {
            e.entry(k).or_insert(v);
        }
        e
    }

    /// **Two row kinds on one stream**, tagged: the consumers, then the
    /// coverage rows — `storage list`'s lesson.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.consumers.rows {
            out(Row::of("consumer", r));
        }
        for c in self.coverage.iter().flatten() {
            out(Row::of("coverage", c));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!(
            "impact of {} {} {}  (selector {})",
            self.producer, self.class, self.path, self.selector
        ));
        if let Some(d) = &self.deprecated {
            t.line(format!(
                "  DEPRECATED{}{}",
                d.since
                    .as_deref()
                    .map(|s| format!(" since {s}"))
                    .unwrap_or_default(),
                d.replaced_by
                    .as_deref()
                    .map(|r| format!(" → replaced by {r}"))
                    .unwrap_or_default()
            ));
        }
        t.blank().line("declared readers:");
        if !self.consumers.rows.is_empty() {
            t.blank().grid(consumer_grid(&self.consumers));
        }
        t.blank().line("also declared on the family:");
        let mut g = Grid::unheaded(2);
        g.row([
            Cell::text("  publishers"),
            Cell::asked(self.declared_publishers.map(|n| format!("{n} session(s)"))),
        ]);
        g.row([
            Cell::text("  queryables"),
            Cell::asked(self.declared_queryables.map(|n| format!("{n} session(s)"))),
        ]);
        t.grid(g);
        t.blank().line("storage coverage:");
        match &self.coverage {
            None => t.line("  —  (not asked: no admin space answered)"),
            Some(rows) if rows.is_empty() => {
                t.line("  not a declared state family — storage coverage does not apply")
            }
            Some(rows) => {
                let mut g = Grid::unheaded(2);
                for row in rows {
                    use zenkey_fleet::Coverage;
                    let (mark, detail) = match &row.coverage {
                        Coverage::Covered(s) => ("✓", format!("covered by {s}")),
                        Coverage::Partial(s) => ("~", format!("PARTIAL via {s}")),
                        Coverage::Uncovered => ("·", "uncovered".to_string()),
                    };
                    let ttl = row
                        .ttl_s
                        .map(|s| format!("  (ttl_s {s})"))
                        .unwrap_or_default();
                    g.row([
                        Cell::text(format!("  {mark} {}", row.path)),
                        Cell::text(format!("{detail}{ttl}")),
                    ]);
                }
                t.grid(g)
            }
        };
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = consumer_notes(&self.consumers);
        if self.coverage.is_none() {
            notes.push(
                Note::coverage(
                    "the storage sweep was not made because no admin space answered: an \
                     empty storage list would read as \"uncovered\", which nobody \
                     established",
                )
                .cite("RFC 13 §3 O4"),
            );
        }
        if self.deprecated.is_some() {
            notes.push(
                Note::caveat(
                    "the subject is retired in the registry ledger; a declared reader of \
                     it is the burn-down `zenctl check retired` counts",
                )
                .cite("RFC 08 §3"),
            );
        }
        notes
    }

    fn bounds(&self) -> Vec<BoundCost> {
        consumer_bounds(&self.consumers)
    }

    fn scope(&self) -> Option<ObservedScope> {
        let mut asked = self.consumers.asked.clone();
        if self.coverage.is_some() {
            asked.push("@/*/router/**/storage_manager/storages/**".into());
        }
        Some(ObservedScope {
            asked,
            window_s: None,
        })
    }
}
