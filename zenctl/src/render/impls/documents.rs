//! The single-fact families — the five that emitted a **pretty, multi-line
//! JSON document** under `--format ndjson` (#232 item 3, #198's headline).
//!
//! They collapsed `Json | Ndjson` into one arm and called `to_string_pretty`,
//! so the output could not be read a line at a time — which is the only reason
//! ndjson exists. There is nothing to fix here beyond writing the impl: three
//! of them genuinely have no rows, so their envelope *is* the document, on one
//! line. `interface show` turns out to have two row kinds, and `blob locate`
//! has one plus the envelope it already argued for in a comment.

use zenkey_fleet::report::{
    BlobFetchReport, BlobProbeReport, BlobTreeIndexReport, DriftVerdict, InterfaceShow, TopicInfo,
};

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_of};

impl Render for TopicInfo {
    const FAMILY: &'static str = "topic-info";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        envelope_of(self)
    }

    /// No rows: one key, described as far as the RFC 09 §5.1 ladder reached.
    /// Saying so explicitly is what the required `rows()` buys — the old code
    /// said it by falling into the json arm.
    fn rows(&self, _out: &mut dyn FnMut(Row)) {}

    fn table(&self, t: &mut Table) {
        // **One** grid for every label/value pair, not one per section: two
        // grids compute their widths independently, so the labels stop lining
        // up down the page — which is the whole reason a reader can scan this
        // rendering at all.
        let mut g = Grid::unheaded(2);
        let mut field = |k: &str, v: String| {
            g.row([Cell::text(k.to_string()), Cell::text(v)]);
        };
        field("key", self.key.clone());
        field("verdict", verdict_word(&self.verdict));
        if !self.note.is_empty() {
            field("", self.note.clone());
        }
        for (k, v) in [
            ("origin", &self.origin),
            ("producer", &self.producer),
            ("class", &self.class),
            ("subject", &self.subject),
        ] {
            if let Some(v) = v {
                field(k, v.clone());
            }
        }
        if !self.variables.is_empty() {
            field("variables", String::new());
            // Indented continuation lines, which is what they are — a
            // `{var}` binding is not a column.
            g.detail(
                self.variables
                    .iter()
                    .map(|(name, value)| format!("  {name} = {value}")),
            );
        }
        if let Some(payload) = &self.payload_type {
            g.row([Cell::text("payload"), Cell::text(payload)]);
            // Since RFC 08 §7 the shape is served, so point at it rather than
            // at the old "lives with the application" dead end.
            g.detail([format!(
                "  (`zenctl interface show {payload} --schema` for the served shape)"
            )]);
        }
        for (k, v) in [
            ("unit", &self.unit),
            ("qos", &self.qos),
            ("rate", &self.rate),
            ("encoding", &self.encoding),
            ("since", &self.since),
            ("about", &self.description),
        ] {
            if let Some(v) = v {
                g.row([Cell::text(k), Cell::text(v)]);
            }
        }
        if let Some(ttl) = self.ttl_s {
            g.row([
                Cell::text("ttl"),
                Cell::text(format!(
                    "{ttl}s  (refresh <= {}s; stale after {ttl}s)",
                    ttl / 2
                )),
            ]);
        }
        if let Some(cardinality) = self.cardinality {
            g.row([Cell::text("cardinality"), Cell::int(cardinality as u64)]);
        }
        t.grid(g);
    }
}

/// The table's spelling of the ladder verdict — **the wire's**, read off the
/// serialization rather than written out again.
///
/// It used to be `{:?}`, so `--format table` said `Registered` while the
/// document said `"registered"`: one value, two spellings, and the CLI corpus
/// pinned both side by side while it lasted. Taking it from serde rather than
/// from a hand-written match means the two cannot drift again, and that a new
/// rung on the ladder needs no edit here.
fn verdict_word(v: &zenkey_fleet::report::TopicVerdict) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        _ => unreachable!("the ladder verdict is a string vocabulary"),
    }
}

impl Render for InterfaceShow {
    const FAMILY: &'static str = "interface-show";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("type_name".into(), self.type_name.clone().into());
        e.insert("carriers".into(), self.carriers.len().into());
        // R4: the count rides only when `--schema` asked — the envelope used
        // to write an unconditional `"schemas": 0`, which read as "asked,
        // none served" on a run that never asked (RFC 09 §5.1 O4).
        if let Some(schemas) = self.schemas.as_option() {
            e.insert("schemas".into(), schemas.len().into());
            // Same guard: a drift count of 0 is a real clean bill only on a
            // run that asked (#410).
            e.insert("drift".into(), self.drift.len().into());
        }
        e
    }

    /// **Two row kinds**, which is where the discriminator earns its keep: a
    /// carrier and a served schema are both "things about this type" and have
    /// nothing else in common.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for c in &self.carriers {
            out(Row::of("carrier", c));
        }
        for s in self.schemas.as_deref().into_iter().flatten() {
            out(Row::of("schema", s));
        }
        // A third kind (#410): the engine's verdict on whether the carriers
        // agree, per origin — what a script acting on a disagreement needs,
        // and what the schema rows above cannot say (no origin, and an
        // unserved hash flattened to `""`).
        for d in &self.drift {
            out(Row::of("drift", d));
        }
    }

    fn table(&self, t: &mut Table) {
        t.line(format!("type      {}", self.type_name));
        t.blank().line(format!(
            "carried by {} subject(s)/procedure(s):",
            self.carriers.len()
        ));
        let mut g = Grid::unheaded(3).max(0, 10).max(1, 10);
        for c in self.carriers.iter().take(20) {
            g.row([
                Cell::text(format!("  {}", c.producer)),
                Cell::text(&c.class),
                Cell::text(&c.path),
            ]);
        }
        t.grid(g);
        if let Some(schemas) = self.schemas.as_deref()
            && !schemas.is_empty()
        {
            t.blank().line("served schema (RFC 08 §7):");
            let mut s = Grid::unheaded(3).max(0, 10).max(1, 12);
            for sc in schemas {
                s.row([
                    Cell::text(format!("  {}", sc.producer)),
                    Cell::text(&sc.kind),
                    Cell::text(&sc.hash),
                ]);
            }
            t.grid(s);
            for sc in schemas {
                if let Some(doc) = &sc.document {
                    t.blank().line(format!("  {} says:", sc.producer));
                    t.line(serde_json::to_string_pretty(doc).unwrap_or_default());
                }
            }
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = Vec::new();
        if self.carriers.len() > 20 {
            // A *rendering* note, not a fleet one: json and ndjson emit every
            // carrier as a row, so nothing was actually dropped — only this
            // table is short.
            notes.push(Note::rendering(format!(
                "{} carrier(s) shown of {}; the machine formats carry them all",
                20,
                self.carriers.len()
            )));
        }
        match self.schemas.as_deref() {
            // R4: not asked is not "none served".
            None => notes.push(
                Note::coverage(
                    "served schemas not asked — pass --schema to query the carriers; \
                     an unasked bus is not one serving nothing",
                )
                .cite("RFC 09 §5.1 O4"),
            ),
            Some([]) => notes.push(Note::silence(
                "schemas asked and no carrier served one — describe is a SHOULD \
                 (RFC 08 §7), so undescribed is not shapeless",
            )),
            // Whether the carriers *agree* is `self.drift`'s business, below
            // — the rows cannot say (#410).
            Some(_) => {}
        }
        // The engine's verdict, per origin, not a recompute over the rows
        // (#410). The recompute this replaced compared `hash` strings across
        // producers: it could not see two hosts of one producer disagree
        // (`SchemaRow` has no origin), and two producers that served no
        // identity compared equal as `""` and read as agreement — the O4 bug
        // #370 fixed in `schema_drift`, which is now the one implementation.
        for drift in &self.drift {
            match drift.verdict {
                DriftVerdict::Disagree => {
                    let identities: std::collections::BTreeSet<&String> = drift
                        .servers
                        .iter()
                        .filter_map(|s| s.hash.as_option())
                        .collect();
                    let servers: Vec<String> = drift
                        .servers
                        .iter()
                        .map(|s| match s.hash.as_option() {
                            Some(h) => format!("{}@{} ({h})", s.producer, s.origin),
                            None => format!("{}@{} (no identity served)", s.producer, s.origin),
                        })
                        .collect();
                    notes.push(
                        Note::coverage(format!(
                            "⚠ {} is served under {} identities — {} — a schema-drift \
                             finding; `zenctl doctor` carries it as one",
                            drift.type_name,
                            identities.len(),
                            servers.join(", ")
                        ))
                        .cite("RFC 08 §7"),
                    );
                }
                // Not a disagreement: a question nobody's evidence answers,
                // and saying "agree" here was the bug.
                DriftVerdict::Unjudgeable => {
                    let silent: Vec<String> = drift
                        .servers
                        .iter()
                        .filter(|s| s.hash.is_not_asked())
                        .map(|s| format!("{}@{}", s.producer, s.origin))
                        .collect();
                    notes.push(
                        Note::coverage(format!(
                            "agreement on {} cannot be established: {} served no identity",
                            drift.type_name,
                            silent.join(", ")
                        ))
                        .cite("RFC 09 §5.1 O4"),
                    );
                }
            }
        }
        notes
    }
}

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
