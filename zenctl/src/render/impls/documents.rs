//! The single-fact blob families: documents with at most one row kind,
//! which once emitted a **pretty, multi-line JSON document** under `--format
//! ndjson` (#232 item 3, #198's headline). Two genuinely have no rows, so
//! their envelope *is* the document, on one line; `blob locate` has one row
//! kind plus the envelope it argued for in a comment.

use zenkey_fleet::report::{BlobFetchReport, BlobProbeReport, BlobTreeIndexReport};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_of};

impl Render for BlobTreeIndexReport {
    const FAMILY: &'static str = "blob-tree";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(format!("tree/{}", self.root));
        let mut g = Grid::unheaded(2);
        g.row([
            Cell::text("  from"),
            Cell::text(format!("{} ({})", self.origin, self.key)),
        ]);
        g.row([
            Cell::text("  index"),
            Cell::text(format!(
                "{} entr{}, {} file(s)",
                self.entries,
                if self.entries == 1 { "y" } else { "ies" },
                self.files
            )),
        ]);
        g.row([
            Cell::text("  content"),
            Cell::text(format!(
                "{} bytes in {} distinct chunk(s)",
                self.total_size, self.chunks
            )),
        ]);
        g.row([Cell::text("  priority"), Cell::text(&self.priority)]);
        g.row([Cell::text("  root"), Cell::text(&self.root)]);
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        vec![
            Note::coverage(
                "the summary is the index only — nothing was fetched, and it needs no \
                 content store",
            )
            .cite("RFC 07 §2.3"),
            Note::coverage("the root is pinned by construction: the key is the identity")
                .cite("RFC 07 §2.1"),
        ]
    }
}

impl Render for BlobFetchReport {
    const FAMILY: &'static str = "blob-fetch";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        t.line(&self.dest);
        let mut g = Grid::unheaded(2);
        g.row([
            Cell::text("  from"),
            Cell::text(format!("{} ({})", self.origin, self.key)),
        ]);
        g.row([
            Cell::text("  bytes"),
            Cell::text(format!(
                "{} in {} chunk(s){}",
                self.bytes,
                self.chunks,
                if self.chunks_resumed > 0 {
                    format!(", {} resumed", self.chunks_resumed)
                } else {
                    String::new()
                }
            )),
        ]);
        g.row([Cell::text("  priority"), Cell::text(&self.priority)]);
        g.row([
            Cell::text("  root"),
            if self.root_pinned {
                Cell::text(format!("{} (pinned)", self.root))
            } else {
                Cell::text("trust-on-first-use — this origin chose the content")
            },
        ]);
        if self.retries > 0 {
            g.row([Cell::text("  retries"), Cell::int(self.retries as u64)]);
        }
        t.grid(g);
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = vec![Note::summary(format!("{} ms", self.elapsed_ms))];
        if !self.root_pinned {
            notes.push(
                Note::coverage(
                    "no root was pinned, so the first origin to answer chose the content",
                )
                .cite("RFC 07 §2.1"),
            );
        }
        // Nonzero rejections mean a replier served bytes that did not verify.
        // The transfer succeeded anyway, which is the point of verifying
        // before disk — but it is a fact about the fleet.
        if self.rejected > 0 {
            notes.push(Note::coverage(format!(
                "{} repl{} failed verification before disk — the transfer succeeded, \
                 which is what verifying before disk is for, but a replier served \
                 bytes that did not verify",
                self.rejected,
                if self.rejected == 1 { "y" } else { "ies" }
            )));
        }
        notes
    }
}

impl Render for BlobProbeReport {
    const FAMILY: &'static str = "blob-probe";

    /// The envelope #232 item 2 is about.
    ///
    /// It was hand-built, and it did three things wrong at once: it renamed
    /// `target` to `probe`, so `--format json` and `--format ndjson` disagreed
    /// about the key for one value; and it wrote `not_probed` and
    /// `declared_by` unconditionally, nulling exactly the fields the struct
    /// `skip_serializing_if`-omits (O4). Deriving it from the serialization
    /// makes all three unrepeatable rather than repaired.
    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = envelope_of(self);
        e.remove("holders");
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for h in &self.holders {
            out(Row::of("holder", h));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("target  {}  (tier {})", self.target, self.tier));
        if self.not_probed.is_some() {
            return;
        }
        t.blank().line("asked:");
        let mut asked = Grid::unheaded(1);
        for selector in &self.asked {
            asked.row([Cell::text(format!("  {selector}"))]);
        }
        t.grid(asked);
        if self.holders.is_empty() {
            return;
        }
        t.blank().line("holders:").blank();
        // Only tier 1 has a manifest endpoint, so "no manifest reply" is a
        // verdict there and a category error on the tier-2 rows.
        let tier1 = self.tier == "artifact";
        let mut g = Grid::unheaded(2).max(0, 16);
        for h in &self.holders {
            let what = match (&h.availability, &h.manifest) {
                (Some(a), Some(m)) => format!(
                    "{}/{} chunks · {} bytes · root {}",
                    a.have,
                    a.chunk_count,
                    m.total_len,
                    short(&m.root)
                ),
                (Some(a), None) if tier1 => {
                    format!("{}/{} chunks · no manifest reply", a.have, a.chunk_count)
                }
                (Some(a), None) => format!("{}/{} chunks", a.have, a.chunk_count),
                (None, Some(m)) => format!(
                    "{} bytes · root {} · no have reply",
                    m.total_len,
                    short(&m.root)
                ),
                // Answering unreadably is not not answering (O4).
                (None, None) => "answered, said nothing readable".to_string(),
            };
            g.row([Cell::text(format!("  {}", h.origin)), Cell::text(what)]);
            let mut detail = Vec::new();
            if let Some(n) = &h.note {
                detail.push(format!("      note: {n}"));
            }
            if let Some(u) = &h.unreadable {
                detail.push(format!("      unreadable: {u}"));
            }
            if let Some(e) = &h.error {
                detail.push(format!("      ✗ {} — {}", e.name, e.message));
            }
            detail.push(format!("      {}", h.key));
            g.detail(detail);
        }
        t.grid(g);
    }

    /// The selectors actually asked; an unissued probe (`not_probed`) asked
    /// nothing, and its scope says exactly that.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: None,
        })
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        // O5 before the result, so "no holders" is read against what was
        // actually asked — and O4 when nothing was asked at all, because an
        // unissued probe must never read as "nobody holds it".
        if let Some(why) = &self.not_probed {
            notes.push(Note::coverage(format!("not probed: {why}")).cite("RFC 09 §5.1 O4"));
        } else if self.holders.is_empty() {
            // R7: the third silence — with no registry read, "nobody declares
            // this tier" was never established either, and the note must not
            // leave only the two bus-side readings on the table.
            if self.declared_by.is_empty() && self.slices_considered == 0 {
                notes.push(Note::silence(
                    "no replies — no origin holds this object, none that could answer \
                     was up, or (no registry loaded) whether anyone even declares \
                     this tier is unknown",
                ));
            } else {
                notes.push(Note::silence(
                    "no replies — no origin holds this object, or none that could answer \
                     was up",
                ));
            }
        }
        if !self.declared_by.is_empty() {
            notes.push(Note::coverage(format!(
                "declared by: {} — a capability claim from the registry, not a \
                 statement that any of them holds this object",
                self.declared_by.join(", ")
            )));
        }
        if self.roots.len() > 1 {
            // More than one root is a finding, not a tie-break: the id is a
            // name and the root is what disambiguates it, so a caller facing
            // two must pin one rather than trust whoever answered first.
            notes.push(
                Note::coverage(format!(
                    "⚠ {} distinct content roots answered — the id is a name and the \
                     root disambiguates it, so pin one rather than trusting whoever \
                     answered first",
                    self.roots.len()
                ))
                .cite("RFC 07 §2.1"),
            );
        }
        notes
    }
}

/// A content root, short enough to read. The full value is in the document.
fn short(hash: &str) -> String {
    match hash.len() > 12 {
        true => format!("{}…", &hash[..12]),
        false => hash.to_string(),
    }
}
