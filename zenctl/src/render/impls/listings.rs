//! The list families: rows, grouped, with a count at the end.

use zenkey_fleet::report::StorageList;

use crate::render::{Cell, Grid, Note, Render, Row, Table};

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
