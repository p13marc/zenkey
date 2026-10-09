//! The list families: rows, grouped, with a count at the end.

use zenkey_fleet::report::StorageList;

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for StorageList {
    const FAMILY: &'static str = "storage-list";

    /// One row kind, tagged like every heterogeneous stream's: v1's
    /// `coverage` rows beside it left with the v1 registry (#612, FJ9).
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.storages {
            out(Row::of("storage", s));
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
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.storages.is_empty() {
            notes.push(Note::silence(
                "no storages found in the admin space — a peer-only mesh, a router \
                 without the storage manager, or the admin space is disabled",
            ));
        }
        notes
    }
}
