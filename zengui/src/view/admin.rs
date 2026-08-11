//! The admin & storage panel (#70) — routers, storages, and the coverage table
//! RFC 09 §2's concern deserves a picture of.
//!
//! **Never ambient.** An `@/**` sweep queries every reachable node, so it costs
//! exactly one button press — the same ground rule as the doctor pane, and for
//! the same reason: the cost is real and it should be the user's to spend.
//!
//! **Every empty state is a sentence, and the sentences are the CLI's.** Where
//! `zenctl admin routers` and `zenctl storage list` already say why a table is
//! empty, this pane says it verbatim, so an operator moving between the two
//! tools is not left wondering whether they disagree.
//!
//! The distinction the panel exists to draw is between three things that all
//! render as "nothing here" if nobody makes them render differently: **not
//! swept**, **swept and the admin space did not answer**, and **swept, answered,
//! and there is genuinely nothing**. The middle one is what `declared_entities`'
//! `Option` carries, and it is why the entity section is load-bearing rather
//! than decoration.

use iced::widget::{button, column, row, scrollable, text};
use iced::{Element, Length};
use zenkey_fleet::report::StorageList;
use zenkey_fleet::{Coverage, CoverageRow, RouterInfo, StorageInfo};

use crate::admin::{AdminState, AdminSweep, router_row_id, storage_row_id};
use crate::message::Message;
use crate::view::kit;
use crate::view::theme::{CoverageTone, colors};
use crate::view::tokens::{font, space};

/// How many declared entities the list renders before it stops and says so.
///
/// A large mesh declares thousands, and an uncapped list is the same bug as a
/// tree that draws every key — so the bound is disclosed rather than silent
/// (RFC 09 §5.1 O6).
const MAX_ENTITIES: usize = 200;

#[derive(Debug, Clone)]
pub enum AdminMsg {
    /// The sweep button — the only way the admin space is ever queried.
    Run,
    Done(Result<std::sync::Arc<AdminSweep>, String>),
    /// Show or hide one row's raw admin document.
    RawToggled(String),
    /// A coverage row's producer, sent to the tree's search box. A pane never
    /// reimplements an action another pane owns.
    FilterProducer(String),
}

fn msg(m: AdminMsg) -> Message {
    Message::Admin(m)
}

pub fn pane(state: &AdminState) -> Element<'_, Message> {
    let run_label = if state.in_flight {
        "sweeping…"
    } else {
        "sweep admin space"
    };
    let mut run = button(text(run_label).size(font::CAPTION)).padding(4);
    if !state.in_flight {
        run = run.on_press(msg(AdminMsg::Run));
    }

    let mut col = column![
        kit::section_header("admin & storage", None),
        row![run].spacing(space::SM),
        kit::muted(
            "the admin space is queried on demand — routers, the storage-manager subtree \
             and the declared entities, one press, never ambient",
        ),
        kit::muted(
            "read through the un-namespaced session every explorer runs (RFC 09 §5): a \
             namespaced session's @ selector is rewritten and matches nothing",
        ),
    ]
    .spacing(space::SM);

    if let Some(e) = &state.error {
        col = col.push(
            text(format!("sweep failed: {e}"))
                .size(font::CAPTION)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                }),
        );
    }

    let Some(sweep) = state.sweep.as_deref() else {
        // O4: never swept is not "no routers".
        col = col.push(kit::empty_state(
            "admin space not swept yet",
            "nothing has been asked — this is \"not asked\", not \"no routers\" \
             (RFC 09 §5.1 O4)",
        ));
        return scrollable(col.padding(space::SM))
            .height(Length::Fill)
            .into();
    };

    col = col.push(topology(sweep));
    col = col.push(routers(&sweep.routers, state));
    col = col.push(storages(&sweep.storage, state));
    col = col.push(coverage(&sweep.storage, sweep.coverage_note.as_deref()));
    col = col.push(entities(sweep));

    scrollable(col.padding(space::SM))
        .height(Length::Fill)
        .into()
}

fn routers<'a>(rows: &'a [RouterInfo], state: &'a AdminState) -> Element<'a, Message> {
    let mut col = column![kit::section_header("routers", None)].spacing(space::XS);
    if rows.is_empty() {
        // Verbatim from `zenctl admin routers`, so the two tools say one thing.
        col = col.push(kit::muted(
            "no routers answered @/*/router — a peer-only mesh, or the admin space is \
             disabled.",
        ));
        col = col.push(kit::muted("silence is not a verdict (RFC 05 §3.1)"));
        return col.into();
    }
    for r in rows {
        let id = router_row_id(&r.zid);
        let mut body = column![
            row![
                kit::mono(r.zid.clone()),
                kit::muted(r.version.clone().unwrap_or_else(|| "-".into())),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
            kit::muted(if r.locators.is_empty() {
                "no locators listed".to_string()
            } else {
                r.locators.join("  ")
            }),
            raw_toggle(&id, state),
        ]
        .spacing(2);
        if state.expanded_raw.contains(&id) {
            body = body.push(kit::mono(raw_text(&r.raw)));
        }
        col = col.push(kit::card(body));
    }
    col.into()
}

fn storages<'a>(list: &'a StorageList, state: &'a AdminState) -> Element<'a, Message> {
    let mut col = column![kit::section_header("storages", None)].spacing(space::XS);
    if list.storages.is_empty() {
        // Verbatim from `zenctl storage list`.
        col = col.push(kit::muted(
            "no storages found in the admin space — a peer-only mesh, a router without \
             the storage manager, or the admin space is disabled.",
        ));
        return col.into();
    }
    for s in &list.storages {
        col = col.push(storage_row(s, state));
    }
    col.into()
}

fn storage_row<'a>(s: &'a StorageInfo, state: &'a AdminState) -> Element<'a, Message> {
    let id = storage_row_id(&s.name, &s.zid);
    // `-` where the layout did not say. Absent is not empty: a storage with no
    // strip_prefix and one whose document omits the field are different facts.
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
    let mut body = column![
        row![
            kit::mono(format!("{} @{}", s.name, s.zid)),
            kit::muted(dash(&s.key_expr)),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center),
        kit::muted(format!(
            "strip {}  ·  volume {}",
            dash(&s.strip_prefix),
            dash(&s.volume)
        )),
        raw_toggle(&id, state),
    ]
    .spacing(2);
    if state.expanded_raw.contains(&id) {
        body = body.push(kit::mono(raw_text(&s.raw)));
    }
    kit::card(body)
}

/// The centrepiece: declared state families against the storages that would
/// seed them.
///
/// Three mutually exclusive states, and the difference between the first two is
/// the whole point. "No registry loaded, so nothing was judged" and "judged,
/// and nothing covers it" are different facts, and only one of them is a
/// finding.
fn coverage<'a>(list: &'a StorageList, note: Option<&'a str>) -> Element<'a, Message> {
    let mut col = column![kit::section_header(
        "declared state families vs storage coverage",
        None
    )]
    .spacing(space::XS);

    if let Some(why) = note {
        col = col.push(kit::empty_state("coverage not judged", why.to_string()));
        return col.into();
    }
    if list.coverage.is_empty() {
        col = col.push(kit::muted(
            "0 declared state families — the loaded registry declares no state subjects",
        ));
        return col.into();
    }

    let mut uncovered = 0usize;
    for r in &list.coverage {
        if matches!(r.coverage, Coverage::Uncovered) {
            uncovered += 1;
        }
        col = col.push(coverage_row(r));
    }

    if uncovered > 0 {
        // RFC 09 §2's consequence, which is the visual #70 exists to add.
        col = col.push(kit::muted(
            "no storage captures an uncovered family — while its producer is down, a late \
             joiner GETs nothing: the `latest` storage is the fleet-wide late-joiner seed \
             (RFC 09 §2)",
        ));
    }
    // Verbatim from `zenctl storage list`. This is why `ttl_s` is on screen:
    // the note is unusable unless the reader can see which families are ttl'd.
    col = col.push(kit::muted(
        "note: an uncovered ttl'd family is not automatically a defect — volatile-state \
         seeding may ride the advanced-pub/sub cache (RFC 04 §3.5); storage is \
         authoritative for durable data.",
    ));
    col.into()
}

fn coverage_row(r: &CoverageRow) -> Element<'_, Message> {
    // Detail strings verbatim from `zenctl storage list`.
    let (tone, detail) = match &r.coverage {
        Coverage::Covered(s) => (CoverageTone::Covered, format!("covered by {s}")),
        Coverage::Partial(s) => (CoverageTone::Partial, format!("PARTIAL via {s}")),
        Coverage::Uncovered => (CoverageTone::Uncovered, "uncovered".to_string()),
    };
    let ttl = match r.ttl_s {
        Some(n) => format!("ttl {n}s"),
        None => "no ttl".to_string(),
    };
    kit::card(
        row![
            kit::badge_coverage(tone, detail),
            button(text(r.producer.clone()).size(font::CAPTION))
                .padding(2)
                .on_press(msg(AdminMsg::FilterProducer(r.producer.clone()))),
            kit::mono(r.path.clone()),
            kit::muted(ttl),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center),
    )
}

/// Declared entities — load-bearing, not decoration: the `Option` here is the
/// only thing separating "the admin space is reachable and empty" from "the
/// admin space is unreachable", which is #70's explicit reachability ask.
fn entities(sweep: &AdminSweep) -> Element<'_, Message> {
    let mut col = column![kit::section_header("declared entities", None)].spacing(space::XS);
    let Some(declared) = &sweep.declared else {
        col = col.push(kit::muted(
            "declared entities: n/a — nothing answered the admin sweep. zenoh's \
             adminspace.enabled defaults to false and a peer mesh has none, so this is \
             \"not available\", never \"nothing declared\" (RFC 09 §5.1 O4).",
        ));
        return col.into();
    };
    if declared.entities.is_empty() {
        col = col.push(kit::muted(
            "declared entities: 0 — the admin space answered and declared none",
        ));
        return col.into();
    }
    col = col.push(kit::muted(format!(
        "declared entities: {}",
        declared.entities.len()
    )));
    for e in declared.entities.iter().take(MAX_ENTITIES) {
        col = col.push(
            row![
                kit::muted(format!("{:?}", e.kind).to_lowercase()),
                kit::mono(e.keyexpr.clone()),
                kit::muted(e.node_zid.clone()),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        );
    }
    if declared.entities.len() > MAX_ENTITIES {
        col = col.push(kit::muted(format!(
            "+{} more not shown (display bound)",
            declared.entities.len() - MAX_ENTITIES
        )));
    }
    col.into()
}

fn raw_toggle<'a>(id: &str, state: &AdminState) -> Element<'a, Message> {
    let shown = state.expanded_raw.contains(id);
    button(
        text(if shown {
            "hide raw document"
        } else {
            "show raw document"
        })
        .size(font::CAPTION),
    )
    .padding(2)
    .on_press(msg(AdminMsg::RawToggled(id.to_string())))
    .into()
}

/// The untrimmed admin document. Shown on request because admin layouts vary
/// by zenoh version, so the fields this build knows how to name are never the
/// whole story.
fn raw_text(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

// — Topology (#118): the mesh the sweep answered, drawn.
//
// The canvas is opaque to iced_test's find-by-text, so the caption above it
// carries the facts (the spark.rs rule); the drawing is the picture, the
// caption is the evidence. Circular layout, deterministic: correctness of
// the overlay is the differentiator, not force-directed prettiness.

/// One node prepared for drawing.
#[derive(Debug, Clone)]
struct MeshNode {
    label: String,
    router: bool,
    answered: bool,
    is_self: bool,
    has_storage: bool,
}

/// The caption + canvas pair.
fn topology<'a>(sweep: &'a AdminSweep) -> Element<'a, Message> {
    let mut col = column![kit::section_header("topology", None)].spacing(space::XS);
    let report = &sweep.topology;
    if report.answered == 0 {
        // Verbatim posture from `zenctl admin graph`: a reading about
        // reachability, never an empty mesh.
        col = col.push(kit::muted(
            "no admin space answered @/*/* — adminspace.enabled defaults off; this is a \
             reading about reachability, never an empty mesh (RFC 05 §3.1)",
        ));
        return col.into();
    }
    let heard_of = report.nodes.iter().filter(|n| !n.answered).count();
    let storage_zids: std::collections::BTreeSet<&str> = sweep
        .storage
        .storages
        .iter()
        .map(|s| s.zid.as_str())
        .collect();
    let nodes: Vec<MeshNode> = report
        .nodes
        .iter()
        .map(|n| MeshNode {
            label: short_zid(&n.zid),
            router: n.whatami == "router",
            answered: n.answered,
            is_self: n.zid == report.self_zid,
            has_storage: storage_zids.contains(n.zid.as_str()),
        })
        .collect();
    let index_of = |zid: &str| report.nodes.iter().position(|n| n.zid == zid);
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for e in &report.edges {
        if let (Some(a), Some(b)) = (index_of(&e.reporter), index_of(&e.peer)) {
            let (a, b) = if a <= b { (a, b) } else { (b, a) };
            if !edges.contains(&(a, b)) {
                edges.push((a, b));
            }
        }
    }
    col = col.push(kit::muted(format!(
        "{} node(s) ({} answered, {heard_of} heard of, not queryable) · {} link(s) · \
         filled = admin space answered · ⌂ = hosts a storage · ring = this session",
        report.nodes.len(),
        report.answered,
        edges.len(),
    )));
    // The caption *is* the testable surface: a bounded node roll-call.
    const LISTED: usize = 12;
    for n in report.nodes.iter().take(LISTED) {
        col = col.push(kit::mono(format!(
            "{}  {}{}{}{}",
            short_zid(&n.zid),
            n.whatami,
            if n.answered { "" } else { "  (heard of)" },
            if storage_zids.contains(n.zid.as_str()) {
                "  ⌂ storage"
            } else {
                ""
            },
            if n.zid == report.self_zid {
                "  ← you"
            } else {
                ""
            },
        )));
    }
    if report.nodes.len() > LISTED {
        col = col.push(kit::muted(format!(
            "… and {} more (the drawing shows all)",
            report.nodes.len() - LISTED
        )));
    }
    col = col.push(
        iced::widget::canvas(Mesh { nodes, edges })
            .width(Length::Fill)
            .height(Length::Fixed(MESH_HEIGHT)),
    );
    col.into()
}

/// zids are long; eight chars tell nodes apart on a drawing.
fn short_zid(zid: &str) -> String {
    if zid.len() > 8 {
        format!("{}…", &zid[..8])
    } else {
        zid.to_string()
    }
}

const MESH_HEIGHT: f32 = 220.0;
const NODE_R: f32 = 7.0;

struct Mesh {
    nodes: Vec<MeshNode>,
    edges: Vec<(usize, usize)>,
}

impl iced::widget::canvas::Program<Message> for Mesh {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::Point;
        use iced::widget::canvas;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let n = self.nodes.len();
        if n == 0 {
            return vec![frame.into_geometry()];
        }
        let cx = bounds.width / 2.0;
        let cy = MESH_HEIGHT / 2.0;
        let radius = (MESH_HEIGHT / 2.0 - 30.0).max(10.0);
        let pos = |i: usize| {
            let angle = std::f32::consts::TAU * (i as f32) / (n as f32);
            Point::new(cx + radius * angle.cos(), cy + radius * angle.sin())
        };
        let palette = colors(theme);
        for (a, b) in &self.edges {
            let path = canvas::Path::line(pos(*a), pos(*b));
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_width(1.0)
                    .with_color(palette.axis()),
            );
        }
        for (i, node) in self.nodes.iter().enumerate() {
            let p = pos(i);
            let dot = canvas::Path::circle(p, NODE_R);
            let tone = if node.router {
                palette.primary()
            } else {
                palette.text_muted()
            };
            if node.answered {
                frame.fill(&dot, tone);
            } else {
                // Heard of, not queryable: an outline, not a filled claim.
                frame.stroke(
                    &dot,
                    canvas::Stroke::default().with_width(1.5).with_color(tone),
                );
            }
            if node.is_self {
                let ring = canvas::Path::circle(p, NODE_R + 3.5);
                frame.stroke(
                    &ring,
                    canvas::Stroke::default()
                        .with_width(1.5)
                        .with_color(palette.success()),
                );
            }
            if node.has_storage {
                let mark = canvas::Path::rectangle(
                    Point::new(p.x + NODE_R, p.y - NODE_R - 4.0),
                    iced::Size::new(5.0, 5.0),
                );
                frame.fill(&mark, palette.warning());
            }
            frame.fill_text(canvas::Text {
                content: node.label.clone(),
                position: Point::new(p.x + NODE_R + 3.0, p.y + 3.0),
                color: palette.text(),
                size: iced::Pixels(11.0),
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }
}
