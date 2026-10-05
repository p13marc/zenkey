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

use iced::widget::{column, row, scrollable, text};
use iced::{Element, Length};
use zenkey_fleet::report::StorageList;
use zenkey_fleet::{Coverage, CoverageRow, RouterInfo, StorageInfo};

use crate::admin::{AdminState, AdminSweep, router_row_id, storage_row_id};
use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::{Accent, CoverageTone, alpha, colors};
use crate::view::tokens::{Spacing, face, font};

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
    Done(Result<std::sync::Arc<AdminSweep>, crate::services::ServiceError>),
    /// Show or hide one row's raw admin document.
    RawToggled(String),
    /// A coverage row's producer, sent to the tree's search box. A pane never
    /// reimplements an action another pane owns.
    FilterProducer(String),
    /// Copy the swept mesh as Graphviz — the engine's `render_dot` (#234),
    /// clipboard-bound like echo's ndjson export.
    CopyDot,
}

fn msg(m: AdminMsg) -> Message {
    Message::Pane(PaneMsg::Admin(m))
}

pub fn pane(state: &AdminState, sp: Spacing) -> Element<'_, Message> {
    let run_label = if state.in_flight {
        "sweeping…"
    } else {
        "sweep admin space"
    };
    let mut run = kit::primary(kit::caption(run_label)).padding(sp.xs);
    if !state.in_flight {
        run = run.on_press(msg(AdminMsg::Run));
    }

    let mut col = column![
        kit::section_header("Admin & storage", None),
        row![run].spacing(sp.sm),
        kit::muted(
            "the admin space is queried on demand — routers, the storage-manager subtree \
             and the declared entities, one press, never ambient",
        ),
        kit::muted(
            "read through the un-namespaced session every explorer runs (RFC 09 §5): a \
             namespaced session's @ selector is rewritten and matches nothing",
        ),
    ]
    .spacing(sp.sm);

    if let Some(e) = &state.error {
        col = col.push(
            kit::body(format!("sweep failed: {e}")).style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            }),
        );
    }

    let Some(sweep) = state.sweep.as_deref() else {
        // O4: never swept is not "no routers".
        col = col.push(kit::empty(
            kit::EmptyKind::NotAsked,
            "admin space not swept yet",
            "nothing has been asked — this is \"not asked\", not \"no routers\" \
             (RFC 09 §5.1 O4)",
        ));
        return scrollable(col.padding(sp.sm)).height(Length::Fill).into();
    };

    col = col.push(topology(sweep, &state.mesh_cache, sp));
    col = col.push(routers(&sweep.routers, state, sp));
    col = col.push(storages(&sweep.storage, state, sp));
    col = col.push(coverage(&sweep.storage, sweep.coverage_note.as_deref(), sp));
    col = col.push(entities(sweep, sp));

    scrollable(col.padding(sp.sm)).height(Length::Fill).into()
}

fn routers<'a>(rows: &'a [RouterInfo], state: &'a AdminState, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![kit::section_header("Routers", None)].spacing(sp.xs);
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
            .spacing(sp.sm)
            .align_y(iced::Alignment::Center),
            kit::muted(if r.locators.is_empty() {
                // Normal on zenoh 1.10+: loopback listen endpoints are
                // filtered from the admin doc (eclipse-zenoh/zenoh#2671,
                // #155) — said only when this node's own version says the
                // filter applies, so an older doc's blank stays a blank.
                if r.version
                    .as_deref()
                    .is_some_and(zenkey_fleet::admin_doc_omits_loopback)
                {
                    "no locators listed — zenoh 1.10+ omits loopback listen endpoints \
                     from the admin doc"
                        .to_string()
                } else {
                    "no locators listed".to_string()
                }
            } else {
                r.locators.join("  ")
            }),
            raw_toggle(&id, state, sp),
        ]
        .spacing(sp.xs);
        if state.expanded_raw.contains(&id) {
            body = body.push(kit::mono(raw_text(&r.raw)));
        }
        col = col.push(kit::card(body));
    }
    col.into()
}

fn storages<'a>(list: &'a StorageList, state: &'a AdminState, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![kit::section_header("Storages", None)].spacing(sp.xs);
    if list.storages.is_empty() {
        // Verbatim from `zenctl storage list`.
        col = col.push(kit::muted(
            "no storages found in the admin space — a peer-only mesh, a router without \
             the storage manager, or the admin space is disabled.",
        ));
        return col.into();
    }
    for s in &list.storages {
        col = col.push(storage_row(s, state, sp));
    }
    col.into()
}

fn storage_row<'a>(s: &'a StorageInfo, state: &'a AdminState, sp: Spacing) -> Element<'a, Message> {
    let id = storage_row_id(&s.name, &s.zid);
    // `-` where the layout did not say. Absent is not empty: a storage with no
    // strip_prefix and one whose document omits the field are different facts.
    let dash = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
    let mut body = column![
        row![
            kit::mono(format!("{} @{}", s.name, s.zid)),
            kit::muted(dash(&s.key_expr)),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
        kit::muted(format!(
            "strip {}  ·  volume {}",
            dash(&s.strip_prefix),
            dash(&s.volume)
        )),
        raw_toggle(&id, state, sp),
    ]
    .spacing(sp.xs);
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
fn coverage<'a>(list: &'a StorageList, note: Option<&'a str>, sp: Spacing) -> Element<'a, Message> {
    let mut col = column![kit::section_header(
        "declared state families vs storage coverage",
        None
    )]
    .spacing(sp.xs);

    if let Some(why) = note {
        col = col.push(kit::empty(
            kit::EmptyKind::NotAsked,
            "coverage not judged",
            why.to_string(),
        ));
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
        col = col.push(coverage_row(r, sp));
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

fn coverage_row(r: &CoverageRow, sp: Spacing) -> Element<'_, Message> {
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
            kit::ghost(kit::caption(r.producer.clone()))
                .padding([0.0, sp.xs])
                .on_press(msg(AdminMsg::FilterProducer(r.producer.clone()))),
            kit::mono(r.path.clone()),
            kit::muted(ttl),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
    )
}

/// Declared entities — load-bearing, not decoration: the `Option` here is the
/// only thing separating "the admin space is reachable and empty" from "the
/// admin space is unreachable", which is #70's explicit reachability ask.
fn entities(sweep: &AdminSweep, sp: Spacing) -> Element<'_, Message> {
    let mut col = column![kit::section_header("Declared entities", None)].spacing(sp.xs);
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
            .spacing(sp.sm)
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

fn raw_toggle<'a>(id: &str, state: &AdminState, sp: Spacing) -> Element<'a, Message> {
    let shown = state.expanded_raw.contains(id);
    kit::ghost(kit::caption(if shown {
        "hide raw document"
    } else {
        "show raw document"
    }))
    .padding([0.0, sp.xs])
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
    role: MeshRole,
    answered: bool,
    is_self: bool,
    has_storage: bool,
}

/// What a node said it is (#540) — its fill's hue and the initial inside it.
/// Neither is a verdict: the hues are the router primary and two accents
/// chosen to sit clear of every verdict colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshRole {
    Router,
    Peer,
    Client,
    Other,
}

impl MeshRole {
    fn of(whatami: &str) -> MeshRole {
        match whatami {
            "router" => MeshRole::Router,
            "peer" => MeshRole::Peer,
            "client" => MeshRole::Client,
            _ => MeshRole::Other,
        }
    }

    fn initial(self) -> &'static str {
        match self {
            MeshRole::Router => "R",
            MeshRole::Peer => "P",
            MeshRole::Client => "C",
            MeshRole::Other => "?",
        }
    }
}

/// One origin satellite (#131): drawn beside the node its evidence names.
struct MeshOrigin {
    label: String,
    /// Index into `nodes` of the session (solid) or reporter (dotted).
    anchor: usize,
    /// `true` = only reported-by — the weaker evidence, drawn dotted.
    reported: bool,
}

/// The caption + canvas pair.
fn topology<'a>(
    sweep: &'a AdminSweep,
    mesh_cache: &'a iced::widget::canvas::Cache,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = column![kit::section_header("Topology", None)].spacing(sp.xs);
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
            role: MeshRole::of(&n.whatami),
            answered: n.answered,
            is_self: n.zid == report.self_zid,
            has_storage: storage_zids.contains(n.zid.as_str()),
        })
        .collect();
    let index_of = |zid: &str| report.nodes.iter().position(|n| n.zid == zid);
    // The undirected mesh is the engine's `mesh_links` (#234): it collapses
    // the per-reporter edges by unordered zid pair and marks reciprocal
    // reports as corroboration — a distinction the hand-rolled dedup this
    // replaces used to throw away.
    let links = zenkey_fleet::mesh_links(report);
    let edges: Vec<(usize, usize, bool)> = links
        .iter()
        .filter_map(|l| Some((index_of(&l.a)?, index_of(&l.b)?, l.corroborated)))
        .collect();
    // The origin overlay (#131): anchor each attachment to the node its
    // evidence names — session zid when the admin sources named one, the
    // mere reporter (drawn dotted) otherwise. Unanchorable rows are counted
    // out loud rather than dropped silently.
    let mut origins: Vec<MeshOrigin> = Vec::new();
    let mut unanchored = 0usize;
    for a in &sweep.origins {
        let (zid, reported) = match &a.session_zid {
            Some(z) => (z.as_str(), false),
            None => (a.reporter_zid.as_str(), true),
        };
        match index_of(zid) {
            Some(anchor) => origins.push(MeshOrigin {
                label: a.origin.clone(),
                anchor,
                reported,
            }),
            None => unanchored += 1,
        }
    }
    col = col.push(kit::muted(format!(
        "{} node(s) ({} answered, {heard_of} heard of, not queryable) · {} link(s) · \
         filled = admin space answered · ⌂ = hosts a storage · ring = this session · \
         base {:?} (bases are deployment-global; the drawing is base-blind)",
        report.nodes.len(),
        report.answered,
        edges.len(),
        sweep.base,
    )));
    // The letters inside the nodes (#540), said in words.
    col = col.push(kit::muted(
        "R router · P peer · C client — the role each node reported",
    ));
    if !sweep.origins.is_empty() || unanchored > 0 {
        let attached = origins.iter().filter(|o| !o.reported).count();
        col = col.push(kit::muted(format!(
            "origins: {} attached by declaration · {} reported-only (dotted) · \
             {unanchored} naming a session outside the drawn mesh (listed, not drawn)",
            attached,
            origins.len() - attached,
        )));
    }
    if !edges.is_empty() {
        let corroborated = edges.iter().filter(|(_, _, c)| *c).count();
        // Reciprocal reports are corroboration, not duplication — a link
        // only one end mentions is weaker evidence, and the caption keeps
        // the distinction the engine's dedup makes.
        col = col.push(kit::muted(format!(
            "{corroborated} of {} link(s) corroborated by both ends — a link \
             only one end mentions is weaker evidence (drawn thinner)",
            edges.len(),
        )));
    }
    col = col.push(kit::muted(
        "drag to pan · scroll to zoom · right-click resets",
    ));
    col = col.push(
        row![
            kit::secondary(kit::caption("copy graphviz (dot)"))
                .padding([0.0, sp.xs])
                .on_press(msg(AdminMsg::CopyDot)),
            kit::muted(
                "the engine's render_dot — the same graph `zenctl admin graph` \
                 emits, so one `dot` invocation reads both explorers",
            ),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
    );
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
        iced::widget::canvas(Mesh {
            nodes,
            edges,
            origins,
            cache: mesh_cache,
        })
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

const MESH_HEIGHT: f32 = 260.0;

/// The background dot grid's pitch (#540).
const GRID: f32 = 16.0;
const NODE_R: f32 = 10.0;

struct Mesh<'a> {
    nodes: Vec<MeshNode>,
    /// `(a, b, corroborated)` — corroborated links draw wider, because both
    /// ends reported them (the engine's `mesh_links` distinction, #234).
    edges: Vec<(usize, usize, bool)>,
    origins: Vec<MeshOrigin>,
    /// Retained geometry (#178).
    ///
    /// The drawing depends on the sweep *and* on the pan/zoom viewport, and
    /// the viewport lives inside the canvas widget's own `State` where the
    /// app cannot see it — so the cache is invalidated from `update`, which is
    /// the only thing that moves it, and by `AdminState::finish` replacing the
    /// whole `AdminState` when a sweep lands. Between those, a redraw is a
    /// pointer.
    cache: &'a iced::widget::canvas::Cache,
}

/// The canvas viewport (#131): pan by drag, zoom by wheel, double-click
/// resets. Pure view state — nothing here touches the report.
#[derive(Debug)]
struct Viewport {
    offset: iced::Vector,
    zoom: f32,
    drag: Option<iced::Point>,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            offset: iced::Vector::new(0.0, 0.0),
            zoom: 1.0,
            drag: None,
        }
    }
}

impl iced::widget::canvas::Program<Message> for Mesh<'_> {
    type State = Viewport;

    fn update(
        &self,
        state: &mut Viewport,
        event: &iced::Event,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Option<iced::widget::canvas::Action<Message>> {
        use iced::mouse;
        use iced::widget::canvas::Action;
        let over = cursor.position_in(bounds);
        match event {
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) if over.is_some() => {
                let ticks = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => *y,
                    mouse::ScrollDelta::Pixels { y, .. } => *y / 40.0,
                };
                state.zoom = (state.zoom * (1.0 + ticks * 0.1)).clamp(0.25, 6.0);
                self.cache.clear();
                Some(Action::request_redraw().and_capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(p) = over {
                    state.drag = Some(p);
                    Some(Action::capture())
                } else {
                    None
                }
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let (Some(from), Some(to)) = (state.drag, over) else {
                    return None;
                };
                state.offset += to - from;
                state.drag = Some(to);
                self.cache.clear();
                Some(Action::request_redraw().and_capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.drag.is_some() =>
            {
                state.drag = None;
                Some(Action::capture())
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
                // Right-click resets too — double-click detection is not
                // worth a timer; the caption says both.
                *state = Viewport::default();
                self.cache.clear();
                Some(Action::request_redraw().and_capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &Viewport,
        renderer: &iced::Renderer,
        theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::Point;
        use iced::widget::canvas;
        let n = self.nodes.len();
        if n == 0 {
            return vec![canvas::Frame::new(renderer, bounds.size()).into_geometry()];
        }
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let palette = colors(theme);
            // A dot grid under everything (#540): the canvas reads as a
            // surface to pan, and the grid stays put while the mesh moves.
            let mut x = GRID / 2.0;
            while x < bounds.width {
                let mut y = GRID / 2.0;
                while y < bounds.height {
                    frame.fill(&canvas::Path::circle(Point::new(x, y), 0.9), palette.line());
                    y += GRID;
                }
                x += GRID;
            }
            // Pan/zoom (#131): translate then scale around the panned center.
            frame.translate(state.offset);
            let cx = bounds.width / 2.0;
            let cy = bounds.height / 2.0;
            frame.translate(iced::Vector::new(cx, cy));
            frame.scale(state.zoom);
            frame.translate(iced::Vector::new(-cx, -cy));
            let radius = (cx.min(cy) - 34.0).max(10.0);
            let pos = |i: usize| {
                let angle = std::f32::consts::TAU * (i as f32) / (n as f32);
                Point::new(cx + radius * angle.cos(), cy + radius * angle.sin())
            };
            for (a, b, corroborated) in &self.edges {
                let path = canvas::Path::line(pos(*a), pos(*b));
                // Corroborated by both ends: heavier and surer. One end's
                // word alone: thinner and fainter — the same evidence grade
                // as before, drawn on a darker ground.
                frame.stroke(
                    &path,
                    canvas::Stroke::default()
                        .with_width(if *corroborated { 2.0 } else { 1.0 })
                        .with_color(alpha(
                            palette.text_dim(),
                            if *corroborated { 0.85 } else { 0.5 },
                        )),
                );
            }
            for (i, node) in self.nodes.iter().enumerate() {
                let p = pos(i);
                let tone = match node.role {
                    MeshRole::Router => palette.primary(),
                    MeshRole::Peer => palette.accent(Accent::Sky),
                    MeshRole::Client => palette.accent(Accent::Fuchsia),
                    MeshRole::Other => palette.text_muted(),
                };
                // This session: a layered glow in the primary — not success,
                // which would claim this node is "good" (#540).
                if node.is_self {
                    for (grow, a) in [(4.0, 0.55), (7.5, 0.28), (11.0, 0.12)] {
                        frame.stroke(
                            &canvas::Path::circle(p, NODE_R + grow),
                            canvas::Stroke::default()
                                .with_width(2.0)
                                .with_color(alpha(palette.primary(), a)),
                        );
                    }
                }
                let dot = canvas::Path::circle(p, NODE_R);
                let ink = if node.answered {
                    // Answered: a filled claim, the initial knocked out of it.
                    frame.fill(&dot, tone);
                    palette.panel()
                } else {
                    // Heard of, not queryable: an outline, not a filled claim.
                    frame.fill(&dot, palette.panel());
                    frame.stroke(
                        &dot,
                        canvas::Stroke::default().with_width(1.5).with_color(tone),
                    );
                    tone
                };
                frame.fill_text(canvas::Text {
                    content: node.role.initial().to_string(),
                    position: p,
                    color: ink,
                    size: iced::Pixels(font::CAPTION),
                    font: face::SEMIBOLD,
                    align_x: iced::widget::text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
                if node.has_storage {
                    let mark = canvas::Path::rectangle(
                        Point::new(p.x + NODE_R, p.y - NODE_R - 5.0),
                        iced::Size::new(6.0, 6.0),
                    );
                    frame.fill(&mark, palette.text_muted());
                }
                frame.fill_text(canvas::Text {
                    content: node.label.clone(),
                    position: Point::new(p.x + NODE_R + 5.0, p.y + 4.0),
                    color: palette.text(),
                    // On the scale, in the data face (#533): a zid is data.
                    size: iced::Pixels(font::CAPTION),
                    font: face::MONO,
                    ..canvas::Text::default()
                });
            }
            // Origin satellites (#131): hexagon-ish diamonds fanned around
            // their anchor, solid line for a sources-named session, dotted for
            // reported-only — the drawing keeps the evidence distinction.
            let mut fan: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for o in &self.origins {
                let slot = fan.entry(o.anchor).or_insert(0);
                let k = *slot;
                *slot += 1;
                let a = pos(o.anchor);
                let angle = std::f32::consts::TAU * (k as f32) / 6.0 + 0.5;
                let d = NODE_R + 22.0 + 10.0 * (k / 6) as f32;
                let p = Point::new(a.x + d * angle.cos(), a.y + d * angle.sin());
                // Neutral (#540): the evidence grade is the drawing — solid
                // and filled for a declared session, dotted and outlined for
                // the reporter's word alone — never a verdict colour.
                let tone = palette.text_muted();
                if o.reported {
                    // Dotted: short dashes along the anchor line.
                    let steps = 6;
                    for t in 0..steps {
                        let f0 = t as f32 / steps as f32;
                        let f1 = f0 + 0.5 / steps as f32;
                        let seg = canvas::Path::line(
                            Point::new(a.x + (p.x - a.x) * f0, a.y + (p.y - a.y) * f0),
                            Point::new(a.x + (p.x - a.x) * f1, a.y + (p.y - a.y) * f1),
                        );
                        frame.stroke(
                            &seg,
                            canvas::Stroke::default()
                                .with_width(1.0)
                                .with_color(palette.text_muted()),
                        );
                    }
                } else {
                    frame.stroke(
                        &canvas::Path::line(a, p),
                        canvas::Stroke::default().with_width(1.0).with_color(tone),
                    );
                }
                let dot = canvas::Path::circle(p, NODE_R - 5.0);
                if o.reported {
                    frame.stroke(
                        &dot,
                        canvas::Stroke::default()
                            .with_width(1.2)
                            .with_color(palette.text_muted()),
                    );
                } else {
                    frame.fill(&dot, tone);
                }
                frame.fill_text(canvas::Text {
                    content: o.label.clone(),
                    position: Point::new(p.x + NODE_R, p.y + 3.0),
                    color: palette.text_muted(),
                    size: iced::Pixels(font::CAPTION),
                    font: face::MONO,
                    ..canvas::Text::default()
                });
            }
        });
        vec![geometry]
    }
}
