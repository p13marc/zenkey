//! Report types this crate owns.
//!
//! `zenkey_fleet::report` is the contract shared with zengui and pinned by
//! `report_contract.rs`. These are not that: the slice cache is *this tool's
//! own disk footprint*, no second frontend renders it, and putting it in the
//! shared surface would freeze a shape nobody else reads. The governing rule
//! catches the difference — a report belongs to the engine iff it is the
//! return value of a fleet function *and* a second frontend would plausibly
//! render it.

use serde::Serialize;

use crate::render::{Cell, Grid, Note, Render, Row, Table};

/// One producer's slice, as the completion cache holds it.
#[derive(Debug, Clone, Serialize)]
pub struct CachedSlice {
    pub producer: String,
    pub registry_version: String,
    pub subjects: usize,
    pub procedures: usize,
}

/// What `zenctl cache show` found on disk.
#[derive(Debug, Clone, Serialize)]
pub struct CacheReport {
    /// The directory itself — the point of the command is that a tool which
    /// leaves files on a user's disk can be asked where they are (#54).
    pub dir: String,
    pub slices: Vec<CachedSlice>,
}

impl Render for CacheReport {
    const FAMILY: &'static str = "cache";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("dir".into(), self.dir.clone().into());
        e.insert("slices".into(), self.slices.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for s in &self.slices {
            out(Row::of("slice", s));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(&self.dir);
        let mut g = Grid::unheaded(3).max(0, 16);
        for s in &self.slices {
            g.row([
                Cell::text(format!("  {}", s.producer)),
                Cell::text(format!("registry {}", s.registry_version)),
                Cell::text(format!(
                    "{} subject(s), {} procedure(s)",
                    s.subjects, s.procedures
                )),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.slices.is_empty() {
            return vec![Note::coverage(
                "empty — completion falls back to the static command tree. Any command \
                 that loads slices fills it (or `zenctl cache refresh`)",
            )];
        }
        vec![Note::coverage(format!(
            "{} producer(s), from the last sighting — suggestions, not an inventory. \
             Nothing reads this except shell completion, and no command answers from it",
            self.slices.len()
        ))]
    }
}
