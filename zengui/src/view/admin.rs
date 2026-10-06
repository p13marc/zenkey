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

use iced::widget::{column, row, scrollable};
use iced::{Element, Length};
use zenkey_fleet::report::StorageList;
use zenkey_fleet::{Coverage, CoverageRow, RouterInfo, StorageInfo};

use crate::admin::{AdminState, AdminSweep, router_row_id, storage_row_id};
use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::CoverageTone;
use crate::view::tokens::Spacing;

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
        col = col.push(kit::error(format!("sweep failed: {e}")));
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

    // The mesh is a tool of its own since #541; the admin pane says how big
    // it is and where it went.
    col = col.push(
        row![
            kit::muted(format!(
                "topology: {} ({} answered) —",
                kit::plural(sweep.topology.nodes.len(), "node"),
                sweep.topology.answered
            )),
            kit::link(kit::caption("open the mesh tool"))
                .padding([0.0, sp.xs])
                .on_press(Message::Workspace(
                    crate::message::WorkspaceMsg::PaneSelected(crate::message::RightPane::Mesh,)
                )),
        ]
        .spacing(sp.xs)
        .align_y(iced::Alignment::Center),
    );
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
