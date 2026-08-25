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
                // Empty is normal on zenoh 1.10+ (loopback endpoints are
                // filtered from the admin doc, eclipse-zenoh/zenoh#2671):
                // stated per-row so a blank never reads as unreachable.
                Cell::text(if r.locators.is_empty() {
                    if r.version
                        .as_deref()
                        .is_some_and(zenkey_fleet::admin_doc_omits_loopback)
                    {
                        "no locators listed (zenoh 1.10+ omits loopback listen endpoints)"
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
/// two things the report does not carry: the origin→session attachments,
/// which are a separate query, and the derived links. Both are already
/// engine-computed; this only says how they are laid out.
pub struct TopologyView<'a> {
    pub report: &'a zenkey_fleet::TopologyReport,
    pub attachments: &'a [zenkey_fleet::OriginAttachment],
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
        // much of it answered.
        envelope_without(self.report, &["nodes", "edges"])
    }

    /// **Three row kinds on one stream.** They used to be concatenated with no
    /// discriminator at all, so a consumer told a node from an edge from an
    /// attachment by probing for fields — the same defect `storage list` had,
    /// one command over.
    fn rows(&self, out: &mut dyn FnMut(Row)) {
        for n in &self.report.nodes {
            out(Row::of("node", n));
        }
        for e in &self.report.edges {
            out(Row::of("edge", e));
        }
        for a in self.attachments {
            out(Row::of("attachment", a));
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
        for a in self.attachments {
            rest.row([Cell::text(match &a.session_zid {
                Some(z) => format!("  {}  ⚓ session {z}  (token {})", a.origin, a.token_key),
                // Reported, not attached: sources named no single session, and
                // saying "attached" would invent one (O4).
                None => format!(
                    "  {}  reported by {} — sources named no single session; shown as \
                     reported, not attached",
                    a.origin, a.reporter_zid
                ),
            })]);
        }
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
    }

    fn notes(&self) -> Vec<Note> {
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
                // An answered 1.10+ node declaring no locators is the
                // normal loopback-only answer, not a reachability gap —
                // said out loud so the empty column reads as what it is.
                if self.report.nodes.iter().any(|x| {
                    x.answered
                        && x.locators.is_empty()
                        && x.version
                            .as_deref()
                            .is_some_and(zenkey_fleet::admin_doc_omits_loopback)
                }) {
                    notes.push(Note::coverage(
                        "a root doc listing no locators is normal on zenoh 1.10+: \
                         loopback listen endpoints are filtered from the admin doc \
                         (eclipse-zenoh/zenoh#2671); a \"via session link\" address \
                         is what a live link used, not a listen-endpoint claim",
                    ));
                }
                notes
            }
        }
    }

    fn scope(&self) -> Option<ObservedScope> {
        Some(ObservedScope {
            asked: vec![self.report.asked.clone()],
            window_s: None,
        })
    }
}

/// The locator column for an answered node: root-doc locators verbatim;
/// where the doc declared none (normal on zenoh 1.10+ loopback-only nodes,
/// eclipse-zenoh/zenoh#2671), the link-corroborated endpoints ride with
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
