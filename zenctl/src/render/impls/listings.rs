//! The list families: rows, grouped, with a count at the end.

use zenkey_fleet::report::{NodeList, StorageList, TopicList};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for TopicList {
    const FAMILY: &'static str = "topic-list";

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.subjects {
            out(Row::of("subject", s));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut current: Option<&String> = None;
        let mut grid = Grid::unheaded(3).max(1, 44);
        for s in &self.subjects {
            if current != Some(&s.producer) {
                grid.group(format!("{}  (registry {})", s.producer, s.registry_version));
                current = Some(&s.producer);
            }
            let mut tail = s.type_name.clone();
            if s.open_ended {
                tail.push_str("  [open-ended]");
            }
            if s.deprecated {
                tail.push_str("  DEPRECATED");
                if let Some(v) = &s.deprecated_since {
                    tail.push_str(&format!(" since {v}"));
                }
                if let Some(r) = &s.replaced_by {
                    tail.push_str(&format!(" → {r}"));
                }
            }
            grid.row([Cell::text(&s.class), Cell::text(&s.path), Cell::text(tail)]);
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
        notes
    }
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
