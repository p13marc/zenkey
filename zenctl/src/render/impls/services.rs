//! Procedures, types and bases: the families whose whole rendering is a list
//! plus the sentence that makes an empty one legible.

use zenkey_fleet::report::{BaseList, InterfaceList, ServiceInfo, ServiceList};

use crate::render::{Cell, Grid, Note, Render, Row, Table};

impl Render for ServiceList {
    const FAMILY: &'static str = "service-list";

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for p in &self.procedures {
            out(Row::of("procedure", p));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut current = None;
        let mut grid = Grid::unheaded(3).max(1, 24);
        for p in &self.procedures {
            if current != Some(&p.producer) {
                grid.group(format!("{}  (registry {})", p.producer, p.registry_version));
                current = Some(&p.producer);
            }
            grid.row([
                Cell::text(format!("  {}", p.kind)),
                Cell::text(&p.path),
                // A procedure that declares no reply type and one whose
                // declaration was never read are different facts; `-` used to
                // be both.
                match &p.reply {
                    Some(r) => Cell::text(format!("→ {r}")),
                    None => Cell::Unknown,
                },
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.procedures.is_empty() {
            return vec![Note::silence(
                "no procedures match. That is what the registry says, not what a \
                 producer is capable of",
            )];
        }
        vec![
            Note::summary(format!(
                "{} registered procedure(s).",
                self.procedures.len()
            )),
            Note::next_step("call one with: zenctl service call <origin|*> <producer> <procedure>"),
        ]
    }
}

impl Render for ServiceInfo {
    const FAMILY: &'static str = "service-info";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut e = serde_json::Map::new();
        e.insert("producer".into(), self.producer.clone().into());
        e.insert(
            "registry_version".into(),
            self.registry_version.clone().into(),
        );
        // Absent, not null: a producer with no service origin and one whose
        // slice never said are different (O4), and the same for a description.
        if let Some(o) = &self.service_origin {
            e.insert("service_origin".into(), o.clone().into());
        }
        if let Some(d) = &self.description {
            e.insert("description".into(), d.clone().into());
        }
        e
    }

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for p in &self.procedures {
            out(Row::of("procedure", p));
        }
    }

    /// Four lines per record and no columns at all — which is why `Table` is a
    /// document over blocks rather than a grid.
    fn table(&self, t: &mut Table) {
        let mut head = Grid::unheaded(2);
        head.row([
            Cell::text("producer"),
            Cell::text(format!(
                "{}  (registry {})",
                self.producer, self.registry_version
            )),
        ]);
        if let Some(origin) = &self.service_origin {
            head.row([
                Cell::text("origin"),
                Cell::text(format!(
                    "{origin}  (a service origin — its keys carry no producer chunk)"
                )),
            ]);
        }
        if let Some(d) = &self.description {
            head.row([Cell::text("about"), Cell::text(d)]);
        }
        t.grid(head);
        if self.procedures.is_empty() {
            return;
        }
        t.blank()
            .line(format!("{} procedure(s):", self.procedures.len()))
            .blank();
        let mut grid = Grid::unheaded(2);
        for p in &self.procedures {
            grid.row([Cell::text(format!("  {}", p.kind)), Cell::text(&p.path)]);
            let mut detail = vec![format!("           {}", p.key)];
            let types = match (&p.request, &p.reply) {
                (Some(rq), Some(rp)) => format!("{rq} → {rp}"),
                (None, Some(rp)) => format!("→ {rp}"),
                (Some(rq), None) => format!("{rq} →"),
                (None, None) => String::new(),
            };
            if !types.is_empty() {
                detail.push(format!("           {types}"));
            }
            if let Some(f) = &p.fanout {
                // Worth seeing *before* reaching for `*`: a forbidden fan-out
                // has no fleet spelling at all (RFC 08 §1.1 G2).
                detail.push(format!(
                    "           fanout {f}{}",
                    if f == "forbidden" {
                        " — no fleet spelling exists; call one origin"
                    } else {
                        ""
                    }
                ));
            }
            if let Some(i) = p.idempotent {
                detail.push(format!("           idempotent {i}"));
            }
            if let Some(d) = &p.description {
                detail.push(format!("           {d}"));
            }
            grid.detail(detail);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.procedures.is_empty() {
            return vec![Note::coverage(
                "no procedures declared. That is what the registry says, not what \
                 the producer is capable of",
            )];
        }
        vec![Note::next_step(format!(
            "call one with: zenctl service call <origin|*> {} <procedure>",
            self.producer
        ))]
    }
}

impl Render for InterfaceList {
    const FAMILY: &'static str = "interface-list";

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for ty in &self.types {
            out(Row::of("type", ty));
        }
    }

    fn table(&self, t: &mut Table) {
        if self.types.is_empty() {
            return;
        }
        t.line("declared payload types:").blank();
        let mut grid = Grid::unheaded(2).max(0, 24);
        for ty in &self.types {
            grid.row([
                Cell::text(format!("  {}", ty.name)),
                Cell::text(format!("{} carrier(s)", ty.carriers)),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.types.is_empty() {
            return vec![Note::silence("no payload types declared")];
        }
        vec![
            Note::summary(format!("{} type(s).", self.types.len())),
            Note::coverage(
                "schema definitions live with the owning application, not in the \
                 registry: a type name here is a name, not a shape",
            )
            .cite("RFC 08 §5"),
        ]
    }
}

impl Render for BaseList {
    const FAMILY: &'static str = "base-list";

    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for b in &self.bases {
            out(Row::of("base", b));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut grid = Grid::unheaded(4).max(0, 24).right(1).right(2);
        for b in &self.bases {
            // The empty base is a real deployment, not a missing value: it is
            // the RFC v1.6 default, and rendering it as `—` would file it
            // under "not asked".
            let display = if b.base.is_empty() {
                "(empty)".to_string()
            } else {
                b.base.clone()
            };
            let mut tail = String::new();
            if !b.storages.is_empty() {
                tail.push_str(&format!("  storage: {}", b.storages.join(", ")));
            }
            if b.origins.is_empty() {
                tail.push_str("  (storage config only — nothing alive)");
            }
            grid.row([
                Cell::text(display),
                Cell::text(format!("{} origin(s)", b.origins.len())),
                Cell::text(format!("{} producer(s)", b.producers.len())),
                Cell::text(tail),
            ]);
        }
        t.grid(grid);
    }

    fn notes(&self) -> Vec<Note> {
        if self.bases.is_empty() {
            return vec![Note::silence(
                "no bases discovered — nothing held a liveliness token matching \
                 **/v1/*/state/*/alive and no storage config names one. Producers \
                 may be down, the mesh unreachable (--connect?), or holding no tokens",
            )];
        }
        let mut notes = vec![Note::next_step(format!(
            "{} base(s). Pin one: zenctl context create <name> --base <base> \
             -c <endpoint> --select",
            self.bases.len()
        ))];
        if self.bases.iter().any(|b| b.base.is_empty()) {
            notes.push(Note::coverage(
                "the (empty) base is selected with --base \"\" — its keys start at \
                 v1/ on the wire",
            ));
        }
        notes
    }
}
