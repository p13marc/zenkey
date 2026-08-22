//! The list families: rows, grouped, with a count at the end.

use zenkey_fleet::report::{NodeList, StorageList, TopicList};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for TopicList {
    const FAMILY: &'static str = "topic-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        // The `--budget` coverage statement (#221) is a fact about the whole
        // observation, not about any row — without it in the stream,
        // "observed 3" reads as "the population is 3" (O5/O6).
        if let Some(window) = &self.budget {
            e.insert(
                "budget".into(),
                serde_json::to_value(window).expect("a BudgetWindow serializes"),
            );
        }
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.subjects {
            out(Row::of("subject", s));
        }
    }

    fn table(&self, t: &mut Table) {
        let budgeted = self.budget.is_some();
        let mut current: Option<&String> = None;
        let mut grid = Grid::unheaded(if budgeted { 5 } else { 4 }).max(1, 44);
        for s in &self.subjects {
            if current != Some(&s.producer) {
                grid.group(format!("{}  (registry {})", s.producer, s.registry_version));
                current = Some(&s.producer);
            }
            let mut tail = s.type_name.clone();
            if s.open_ended {
                tail.push_str("  [open-ended]");
            }
            // A retirement gets its own cell rather than being appended to
            // the type, so the word can carry a mark. Empty for every other
            // row, which costs no width (#200).
            let mut retired = String::new();
            if s.deprecated {
                retired.push_str("DEPRECATED");
                if let Some(v) = &s.deprecated_since {
                    retired.push_str(&format!(" since {v}"));
                }
                if let Some(r) = &s.replaced_by {
                    retired.push_str(&format!(" → {r}"));
                }
            }
            let class = Cell::text(format!("  {}", s.class));
            let path = Cell::text(&s.path);
            let tail = Cell::text(tail);
            let retired = Cell::styled(retired, crate::render::style::DEPRECATED);
            // Indented under the group heading, which is what tells a row
            // from a producer name at a glance.
            if budgeted {
                grid.row([class, path, tail, budget_cell(s.budget.as_ref()), retired]);
            } else {
                grid.row([class, path, tail, retired]);
            }
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.subjects.is_empty() {
            // Used to be an early `return` out of the table arm, which is why
            // the machine formats got an empty stream and no explanation.
            return vec![Note::silence(
                "no subjects match. An empty set is not a verdict: the registry may \
                 not be loaded, or no producer declares one here — `zenctl node list` \
                 says who is up",
            )];
        }
        let mut notes = vec![Note::summary(format!(
            "{} registered subject(s).",
            self.subjects.len()
        ))];
        let open_ended = self.subjects.iter().filter(|s| s.open_ended).count();
        if open_ended > 0 {
            let is = if open_ended == 1 { "is" } else { "are" };
            notes.push(Note::coverage(format!(
                "{open_ended} {is} open-ended ({{var...}}): the registry fixes their \
                 shape, not their members. Use `zenctl topic echo` to see what a live \
                 fleet actually publishes"
            )));
        }
        if let Some(w) = &self.budget {
            // The coverage statement the column's numbers rest on (#221):
            // what was watched, for how long, under what bound (O5/O6) —
            // and the O4 rule that keeps "saw 3" from reading as "is 3".
            let mut coverage = format!(
                "budget: observed for {}s over {} scope(s), {} distinct key(s) \
                 retained; counts are lower bounds on the population, so under \
                 the declared cardinality is not a verdict — only over is \
                 (RFC 04 §1.2), and {{var...}} families are exempt: rest-variable \
                 (RFC 08 §6.1)",
                w.window_s,
                w.scopes.len(),
                w.keys,
            );
            if w.evicted > 0 {
                coverage.push_str(&format!(
                    ". {} key(s) were evicted by the observer's bound — every \
                     count above is weakened by that",
                    w.evicted
                ));
            }
            notes.push(Note::coverage(coverage));
        }
        notes
    }
}

/// Draw one row's [`BudgetCell`] (#221). The wording is the honesty rule:
/// "saw", never "has" — and an empty cell for a literal row claims nothing,
/// which is not a pass.
fn budget_cell(budget: Option<&zenkey_fleet::report::BudgetCell>) -> Cell {
    let Some(b) = budget else {
        return Cell::text(String::new());
    };
    if b.exempt.is_some() {
        return Cell::text(format!("exempt: rest-variable (saw {})", b.observed));
    }
    let Some(declared) = b.declared else {
        return Cell::text(format!("no declared bound, saw {}", b.worst_observed));
    };
    let spread = if b.origins > 1 {
        format!(" (max of {} origins)", b.origins)
    } else {
        String::new()
    };
    if b.over {
        let origin = b.worst_origin.as_deref().unwrap_or("?");
        return Cell::styled(
            format!(
                "OVER declared {declared}: saw {} on {origin}{spread}",
                b.worst_observed
            ),
            crate::render::style::WARNING,
        );
    }
    Cell::text(format!(
        "declared {declared}, saw {}{spread}",
        b.worst_observed
    ))
}

impl Render for NodeList {
    const FAMILY: &'static str = "node-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        // O4 on the envelope, not only on the rows: a `None` app means "no
        // slice served" when a join was attempted and "not asked" when it was
        // not, and nothing in a row can say which.
        e.insert("slices_joined".into(), self.slices_joined.into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for n in &self.nodes {
            out(Row::of("node", n));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(2);
        let mut last = "";
        for row in &self.nodes {
            if row.origin != last {
                grid.group(&row.origin);
                last = &row.origin;
            }
            let detail = match (&row.app, &row.registry_version) {
                (Some(app), Some(v)) => Cell::text(format!("(app {app}, registry {v})")),
                // Asked and unanswered is a fact; not asked is not (O4), and
                // these are now two different cells rather than two strings.
                _ if self.slices_joined => Cell::text("(no served slice)"),
                _ => Cell::Unknown,
            };
            grid.row([Cell::text(format!("  {}", row.producer)), detail]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.nodes.is_empty() {
            return vec![Note::silence(
                "no live producers. Silence is not a verdict: a producer that is up \
                 but not declaring liveliness is invisible here, and so is one on \
                 another base",
            )];
        }
        let origins: std::collections::BTreeSet<&str> =
            self.nodes.iter().map(|r| r.origin.as_str()).collect();
        vec![Note::summary(format!(
            "{} producer(s) on {} origin(s).",
            self.nodes.len(),
            origins.len()
        ))]
    }
}

impl Render for StorageList {
    const FAMILY: &'static str = "storage-list";

    /// **Two row kinds on one stream**, and this is the family that shows why
    /// the tag matters: storages and coverage rows used to be concatenated
    /// with nothing to tell them apart, so a consumer identified a line by
    /// probing for a field.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.storages {
            out(Row::of("storage", s));
        }
        for c in &self.coverage {
            out(Row::of("coverage", c));
        }
    }

    fn table(&self, t: &mut Table) {
        if !self.storages.is_empty() {
            t.line("configured storages:").blank();
            let mut grid = Grid::unheaded(2);
            for s in &self.storages {
                grid.row([
                    Cell::text(format!("  {}", s.name)),
                    Cell::text(format!(
                        "@{}  {}",
                        s.zid,
                        s.key_expr.as_deref().unwrap_or("—")
                    )),
                ]);
                // The admin document omitting a field and a storage having no
                // strip prefix are different facts, and the old rendering
                // spelled both `-`.
                grid.detail([format!(
                    "    strip {}  ·  volume {}",
                    s.strip_prefix.as_deref().unwrap_or("—"),
                    s.volume.as_deref().unwrap_or("—")
                )]);
            }
            t.grid(grid);
        }
        if !self.coverage.is_empty() {
            t.blank()
                .line("declared state families vs storage coverage:")
                .blank();
            let mut grid = Grid::unheaded(3).max(1, 36);
            for row in &self.coverage {
                use zenkey_fleet::Coverage;
                let (mark, detail) = match &row.coverage {
                    Coverage::Covered(s) => ("✓", format!("covered by {s}")),
                    Coverage::Partial(s) => ("~", format!("PARTIAL via {s}")),
                    Coverage::Uncovered => ("·", "uncovered".to_string()),
                };
                grid.row([
                    Cell::text(format!("  {mark} {}", row.producer)),
                    Cell::text(&row.path),
                    Cell::text(detail),
                ]);
            }
            t.grid(grid);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.storages.is_empty() {
            notes.push(Note::silence(
                "no storages found in the admin space — a peer-only mesh, a router \
                 without the storage manager, or the admin space is disabled",
            ));
        }
        if !self.coverage.is_empty() {
            notes.push(
                Note::coverage(
                    "an uncovered ttl'd family is not automatically a defect — \
                     volatile-state seeding may ride the advanced-pub/sub cache; \
                     storage is authoritative for durable data",
                )
                .cite("RFC 04 §3.5"),
            );
        }
        notes
    }
}

impl Render for zenkey_fleet::NodeInfo {
    const FAMILY: &'static str = "node-info";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("origin".into(), self.origin.clone().into());
        e.insert("producers".into(), self.producers.len().into());
        e
    }

    /// **Two row kinds**: what this origin runs, and how fresh its declared
    /// state is. This is #198's headline example — `node info --format ndjson`
    /// emitted a *pretty multi-line document*, so the one command whose answer
    /// is a list of producers could not be read a producer at a time.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for p in &self.producers {
            out(Row::of("producer", p));
        }
        for f in &self.freshness {
            out(Row::of("freshness", f));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("origin    {}", self.origin));
        let mut g = Grid::unheaded(3).max(0, 16);
        for p in &self.producers {
            let alive = if p.alive { "alive" } else { "no token" };
            let detail = match (&p.app, &p.registry_version) {
                (Some(app), Some(v)) => {
                    let mut d = format!(
                        "app {app} · registry v{v} · {} subject(s) · {} procedure(s)",
                        p.subjects, p.procedures
                    );
                    if !p.blob_tiers.is_empty() {
                        d.push_str(&format!(" · blob: {}", p.blob_tiers.join(",")));
                    }
                    if !p.media.is_empty() {
                        d.push_str(&format!(
                            " · media: {}",
                            p.media
                                .iter()
                                .map(|m| format!("{} ({})", m.path, m.encoding))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if p.deprecated_served > 0 {
                        d.push_str(&format!(
                            " · {} DEPRECATED still served",
                            p.deprecated_served
                        ));
                    }
                    d
                }
                // Unknown, not absent: no introspect reply is a fact about the
                // fetch, never about the producer's capabilities (O4).
                _ => "(no introspect reply — capabilities unknown, not absent)".to_string(),
            };
            g.row([
                Cell::text(format!("  {}", p.name)),
                Cell::text(format!("[{alive}]")),
                Cell::text(detail),
            ]);
        }
        t.grid(g);
        if !self.freshness.is_empty() {
            t.line("state freshness (declared ttl_s):");
            let mut f = Grid::unheaded(3);
            for row in &self.freshness {
                f.row([
                    Cell::text(format!("  {}/{}", row.producer, row.path)),
                    // "No sample seen" is not "zero seconds old".
                    Cell::asked(row.age_s.map(|a| format!("{a}s old"))),
                    Cell::text(format!(
                        "(ttl {}s){}",
                        row.ttl_s,
                        if row.stale { "  STALE" } else { "" }
                    )),
                ]);
            }
            t.grid(f);
        }
    }

    fn notes(&self) -> Vec<Note> {
        if self.producers.is_empty() {
            return vec![Note::silence(
                "no liveliness token and no introspect reply — not on the roster, \
                 which is not proof it does not exist",
            )];
        }
        Vec::new()
    }
}
