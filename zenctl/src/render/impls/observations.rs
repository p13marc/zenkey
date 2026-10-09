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

use crate::render::{Cell, Grid, Note, ObservedScope, Render, Row, Table, envelope_without};

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

    /// The asked node kinds (empty = all three) and the round's window.
    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: self.asked.clone(),
            window_s: Some(self.timeout_s),
        })
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
                // Empty can be normal on zenoh 1.10+ (loopback endpoints may
                // be filtered from the admin doc, eclipse-zenoh/zenoh#2671):
                // stated per-row so a blank never reads as unreachable.
                Cell::text(if r.locators.is_empty() {
                    if r.version
                        .as_deref()
                        .is_some_and(zenkey_fleet::admin_doc_omits_loopback)
                    {
                        "no locators listed (zenoh 1.10+ may omit loopback listen endpoints)"
                            .to_string()
                    } else {
                        "no locators listed".to_string()
                    }
                } else {
                    r.locators.join(", ")
                }),
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

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.asked.clone()],
            window_s: None,
        })
    }
}

/// The mesh as the admin space reports it (#198).
///
/// A wrapper over the engine's `TopologyReport`, because the rendering needs
/// what the report does not carry: the derived links, already
/// engine-computed; this only says how they are laid out. v1's
/// origin→session attachments (`--origins`) left with the v1 grammar
/// (#612, FJ9).
pub struct TopologyView<'a> {
    pub report: &'a zenkey_fleet::TopologyReport,
}

impl serde::Serialize for TopologyView<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.report.serialize(s)
    }
}

impl Render for TopologyView<'_> {
    const FAMILY: &'static str = "admin-graph";

    fn envelope(&self) -> serde_json::Map<String, serde_json::Value> {
        // The rows carry these; the envelope carries what was asked and how
        // much of it answered — and, for the instance join, what it read in
        // which namespace, without the instances themselves (#705).
        let mut e = envelope_without(self.report, &["nodes", "edges", "instances"]);
        if let Some(join) = self.report.instances.as_option() {
            e.insert(
                "instances".into(),
                serde_json::Value::Object(envelope_without(join, &["instances"])),
            );
        }
        e
    }

    /// **Three row kinds on one stream.** Nodes and edges used to be
    /// concatenated with no discriminator at all, so a consumer told a node
    /// from an edge by probing for fields — the same defect `storage list`
    /// had, one command over. zk2's instances are the third (#705), each
    /// with its `attachment` tag.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for n in &self.report.nodes {
            out(Row::of("node", n));
        }
        for e in &self.report.edges {
            out(Row::of("edge", e));
        }
        for i in self
            .report
            .instances
            .as_option()
            .into_iter()
            .flat_map(|j| &j.instances)
        {
            out(Row::of("instance", i));
        }
    }

    fn table(&self, t: &mut Table) {
        let mut nodes = Grid::unheaded(4);
        for n in &self.report.nodes {
            let you = if n.zid == self.report.self_zid {
                "  ← you"
            } else {
                ""
            };
            if n.answered {
                nodes.row([
                    Cell::text(&n.zid),
                    Cell::text(&n.whatami),
                    // Heard of but never queried is not "no version".
                    Cell::asked(n.version.clone()),
                    Cell::text(format!("{}{you}", locator_cell(n))),
                ]);
            } else {
                // A heard-of node can still carry link evidence: the
                // address its reporter reached it at, labelled as such.
                let via = if n.locators_via_links.is_empty() {
                    String::new()
                } else {
                    format!("  via session link: {}", n.locators_via_links.join(" "))
                };
                nodes.row([
                    Cell::text(&n.zid),
                    Cell::text(&n.whatami),
                    Cell::Unknown,
                    Cell::text(format!("(heard of, not queryable){via}{you}")),
                ]);
            }
        }
        t.grid(nodes);

        let mut rest = Grid::unheaded(1);
        for link in zenkey_fleet::mesh_links(self.report) {
            rest.row([Cell::text(format!(
                "  {} —— {}{}{}",
                link.a,
                link.b,
                if link.corroborated {
                    "  (both report it)"
                } else {
                    ""
                },
                if link.links.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", link.links.join(", "))
                }
            ))]);
        }
        t.grid(rest);

        // zk2's instances (#705): each attachment its own mark and word, so
        // stripping colour or reading in black and white loses nothing.
        if let Some(join) = self.report.instances.as_option()
            && !join.instances.is_empty()
        {
            use zenkey_fleet::report::Attachment;
            t.blank().line("zk2 instances:");
            let mut g = Grid::unheaded(3);
            for i in &join.instances {
                let what = match &i.attachment {
                    Attachment::Attached { routers } => format!(
                        "→ {}",
                        routers
                            .iter()
                            .map(|r| format!("{} (as {})", r.router, r.listed_as))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Attachment::Unattached { reason } => format!("✗ unattached: {reason}"),
                    Attachment::Unattributable { reason } => {
                        format!("? unattributable: {reason}")
                    }
                };
                g.row([
                    Cell::text(format!("{}@{}", i.address, i.instance)),
                    // No zid is not an empty one: the descriptor named none.
                    Cell::asked(i.zid.clone()),
                    Cell::text(what),
                ]);
            }
            t.grid(g);
        }
    }

    fn notes(&self) -> Vec<Note> {
        let mut notes = self.mesh_notes();
        if let Some(join) = self.report.instances.as_option() {
            notes.extend(instance_notes(join));
        }
        notes
    }

    fn scope(&self) -> Option<ObservedScope> {
        let mut asked = vec![self.report.asked.clone()];
        if let Some(join) = self.report.instances.as_option() {
            asked.push(join.selector.clone());
        }
        Some(ObservedScope {
            asked,
            window_s: None,
        })
    }
}

/// What the instance join read and how far it got (#705): in which
/// namespace, through which routers, and what it could not count.
fn instance_notes(join: &zenkey_fleet::report::InstanceJoin) -> Vec<Note> {
    let ns = if join.namespace.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {:?}", join.namespace)
    };
    let mut notes = Vec::new();
    if let Some(why) = &join.unobservable {
        notes.push(Note::silence(format!(
            "zk2 instances in {ns}: {why} — no instance is attached or unattached, the join \
             is unobservable"
        )));
        return notes;
    }
    let (attached, unattached, unattributable) = join.counts();
    notes.push(
        Note::coverage(format!(
            "{} zk2 instance(s) read in {ns} through `{}`: {attached} attached, {unattached} \
             unattached, {unattributable} unattributable; each joined by the zid its \
             descriptor states, compared by value, onto {} verified router(s) — only a \
             router's own answer counts",
            join.instances.len(),
            join.selector,
            join.verified.len()
        ))
        .cite("spec §3.3, §4.2"),
    );
    if join.instances.is_empty() {
        notes.push(
            Note::coverage(format!(
                "no zk2 token visible to this reader in {ns}: nothing to join"
            ))
            .cite("spec §8.1"),
        );
    }
    if !join.complete {
        notes.push(
            Note::coverage(
                "the presence read ran to its timeout, so it is possibly incomplete: an \
                 instance missing here may still be up",
            )
            .cite("spec §8.1"),
        );
    }
    if !join.unverified.is_empty() {
        notes.push(
            Note::caveat(format!(
                "{} admin answer(s) could not be shown to be a router's and attach nothing: {}",
                join.unverified.len(),
                join.unverified.join("; ")
            ))
            .cite("spec §4.2"),
        );
    }
    notes
}

impl TopologyView<'_> {
    /// The mesh's own notes: what answered, and what an empty locator
    /// column means.
    fn mesh_notes(&self) -> Vec<Note> {
        match self.report.answered {
            0 => vec![Note::silence(format!(
                "no admin space answered {} — adminspace.enabled defaults off; this is \
                 a reading about reachability, never an empty mesh",
                self.report.asked
            ))],
            n => {
                let mut notes = vec![Note::coverage(format!(
                    "{n} root doc(s) answered {}; {} node(s) total ({} only heard of)",
                    self.report.asked,
                    self.report.nodes.len(),
                    self.report.nodes.iter().filter(|x| !x.answered).count()
                ))];
                // An answered 1.10+ node declaring no locators may be the
                // filtered loopback answer, not a reachability gap —
                // said out loud so the empty column reads as what it is.
                if self.report.nodes.iter().any(|x| {
                    x.answered
                        && x.locators.is_empty()
                        && x.version
                            .as_deref()
                            .is_some_and(zenkey_fleet::admin_doc_omits_loopback)
                }) {
                    notes.push(Note::coverage(
                        "a root doc listing no locators can be normal on zenoh 1.10+: \
                         loopback listen endpoints may be filtered from the admin doc \
                         (all of them on 1.10.0, eclipse-zenoh/zenoh#2671; from 1.10.1, \
                         only those an unspecified listener resolves to); a \
                         \"via session link\" address is what a live link used, \
                         not a listen-endpoint claim",
                    ));
                }
                notes
            }
        }
    }
}

/// The locator column for an answered node: root-doc locators verbatim;
/// where the doc declared none (possible on zenoh 1.10+ loopback-only
/// nodes, eclipse-zenoh/zenoh#2671), the link-corroborated endpoints ride with
/// their provenance labelled — never folded into the locator claim — and
/// a node no link names says so rather than rendering blank.
fn locator_cell(n: &zenkey_fleet::TopologyNode) -> String {
    if !n.locators.is_empty() {
        n.locators.join(" ")
    } else if !n.locators_via_links.is_empty() {
        format!("via session link: {}", n.locators_via_links.join(" "))
    } else {
        "no locators listed".to_string()
    }
}
