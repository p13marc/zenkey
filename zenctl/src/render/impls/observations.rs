//! Scouting and the admin space's routers: two families whose empty answer is
//! the interesting one (#236).
//!
//! Both used to state their coverage in the table arm only. `zenctl scout
//! --format json` was worse than that — it printed the sentence to **stdout**
//! *before* the format match and returned, so an empty segment emitted prose
//! where a JSON document was promised, and `| jq` failed on it. The ndjson path
//! had it right, which is what makes it a slip rather than a decision.
//!
//! Under the trait the coverage statement is a note, so it reaches every
//! format and stderr both, and the empty case renders an empty document rather
//! than a sentence.

use zenkey_fleet::report::{RouterList, ScoutReport};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for ScoutReport {
    const FAMILY: &'static str = "scout";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("asked".into(), serde_json::json!(self.asked));
        e.insert("timeout_s".into(), self.timeout_s.into());
        e.insert("heard".into(), self.heard.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for h in &self.heard {
            out(Row::of("hello", h));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(3);
        for h in &self.heard {
            g.row([
                Cell::text(&h.zid),
                Cell::text(&h.whatami),
                // A node that advertises no locator and one we did not ask
                // about are not the same; an empty list is the former.
                Cell::text(h.locators.join(" ")),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.heard.is_empty() {
            // The boundary, not an absence — and now in the document too.
            return vec![
                Note::coverage(format!(
                    "no Hellos within {}s on this segment — scouting reaches only the \
                 local multicast domain (or configured gossip); that is a boundary, \
                 not a claim that nothing is running",
                    self.timeout_s
                ))
                .cite("RFC 09 §5.1 O5"),
            ];
        }
        vec![
            Note::summary(format!(
                "{} node(s) heard within {}s",
                self.heard.len(),
                self.timeout_s
            )),
            Note::coverage(
                "scouting reaches the local multicast domain and the configured \
                 gossip — a node outside both is not absent, it is out of range",
            )
            .cite("RFC 09 §5.1 O5"),
        ]
    }
}

impl Render for RouterList {
    const FAMILY: &'static str = "admin-routers";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("asked".into(), self.asked.clone().into());
        e.insert("routers".into(), self.routers.len().into());
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for r in &self.routers {
            out(Row::of("router", r));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut g = Grid::unheaded(3);
        for r in &self.routers {
            g.row([
                Cell::text(&r.zid),
                // A router whose admin document omits its version is not one
                // we failed to ask (O4).
                Cell::asked(r.version.clone()),
                Cell::text(r.locators.join(", ")),
            ]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        if self.routers.is_empty() {
            // `[]` on its own cannot tell these two apart, and they are
            // different facts about the deployment.
            return vec![
                Note::coverage(format!(
                    "no routers answered {} — a peer-only mesh, or the admin space is \
                 disabled",
                    self.asked
                ))
                .cite("RFC 09 §5.1 O5"),
            ];
        }
        vec![Note::summary(format!(
            "{} router(s) answered {}",
            self.routers.len(),
            self.asked
        ))]
    }
}
