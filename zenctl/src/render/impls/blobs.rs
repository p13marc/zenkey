//! The `@blob` plane's list (RFC 07 §2.7): a declaration is a capability, and
//! the footer exists because the one mistake this table invites is reading it
//! as possession.

use zenkey_fleet::report::BlobList;

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for BlobList {
    const FAMILY: &'static str = "blob-list";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        // The coverage claim, on the first line: how many slices were read at
        // all, and how many of them said nothing about blobs (O5). An empty
        // table means one of two very different things without it.
        e.insert("slices_considered".into(), self.slices_considered.into());
        e.insert(
            "slices_without_blob".into(),
            self.slices_without_blob.into(),
        );
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for tier in &self.tiers {
            out(Row::of("tier", tier));
        }
    }

    fn table(&self, t: &mut Table) {
        if self.tiers.is_empty() {
            return;
        }
        t.line("declared @blob tiers:").blank();
        let mut grid = Grid::unheaded(3).max(0, 12);
        for row in &self.tiers {
            grid.row([
                Cell::text(format!("  {}", row.producer)),
                Cell::text(&row.tier),
                Cell::text(format!(
                    "v{}{}",
                    row.registry_version,
                    if row.known_tier {
                        ""
                    } else {
                        "  (unreserved tier token)"
                    }
                )),
            ]);
            let mut detail = Vec::new();
            if !row.endpoints.is_empty() {
                detail.push(format!("      endpoints  {}", row.endpoints.join(", ")));
            }
            if let Some(algo) = &row.algo {
                detail.push(format!("      algo       {algo}"));
            }
            if let Some(r) = &row.reference {
                detail.push(format!(
                    "      reference  {r}  (carries the content root — RFC 07 §2.1)"
                ));
            }
            if let Some(e) = &row.encoding {
                detail.push(format!("      encoding   {e}"));
            }
            // Three different facts, three different sentences. An empty
            // roster is not a fleet verdict (RFC 05 §3.1) and an unasked one
            // is not an empty one (RFC 09 §5.1 O4).
            detail.push(match &row.origins {
                Some(origins) if origins.is_empty() => "      origins    — (no liveliness \
                     token answered; silence is not a verdict — RFC 05 §3.1)"
                    .to_string(),
                Some(origins) => format!("      origins    {}", origins.join(" ")),
                None => "      origins    — (roster not asked)".to_string(),
            });
            if let Some(d) = &row.description {
                detail.push(format!("      {d}"));
            }
            grid.detail(detail);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.tiers.is_empty() {
            return vec![Note::coverage(format!(
                "no producer declares an @blob tier ({} slice(s) read). An empty \
                 table here is what the registry says, not what the bus holds",
                self.slices_considered
            ))];
        }
        vec![Note::coverage(format!(
            "{} of {} slice(s) declare a tier. A declaration is a capability, never \
             possession — `zenctl blob probe <id>` asks who actually holds one",
            self.slices_considered - self.slices_without_blob,
            self.slices_considered
        ))]
    }
}
