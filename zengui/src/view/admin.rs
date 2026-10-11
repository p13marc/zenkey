//! The admin & storage panel (#70; zk2's since #614, FL9) — routers,
//! storages, and each storage judged against S4.
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
//! swept**, **swept and the admin space did not answer**, and **swept,
//! answered, and there is genuinely nothing**.
//!
//! **The storage table is S4's** (D14): v1's read "this storage covers the
//! state" as good, because its `latest` storage was the late-joiner seed.
//! zk2's owner answers for its own state and a storage there is the finding
//! — so each row says whether the storage answers on an owner's state in
//! this namespace, in the doctor's `storage-on-state` words.

use iced::widget::{column, row, scrollable};
use iced::{Element, Length};
use zenkey_fleet::DeclaredEntities;
use zenkey_fleet::report::{RouterInfo, StorageInfo};

use crate::admin::{AdminState, AdminSweep, OnState, S4Row, router_row_id, storage_row_id};
use crate::message::{Message, PaneMsg};
use crate::view::kit;
use crate::view::theme::JudgementTone;
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
}

fn msg(m: AdminMsg) -> Message {
    Message::Pane(PaneMsg::Admin(m))
}

/// The pane. `declared` is the deployment's own read of the declared
/// entities (`Deployment::admin_declared`, the skeleton's input): `None`
/// until an answer lands, or when no admin space answered — never "nothing
/// declared" (O4).
pub fn pane<'a>(
    state: &'a AdminState,
    declared: Option<&'a DeclaredEntities>,
    sp: Spacing,
) -> Element<'a, Message> {
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
    let instances = match sweep.topology.instances.as_option() {
        None => String::new(),
        Some(join) => {
            let (attached, unattached, unattributable) = join.counts();
            format!(
                " · {} attached, {unattached} unattached, {unattributable} unattributable",
                kit::plural(attached, "instance")
            )
        }
    };
    col = col.push(
        row![
            kit::muted(format!(
                "topology: {} ({} answered){instances} —",
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
    col = col.push(storages(&sweep.storages, state, sp));
    col = col.push(s4_table(sweep, sp));
    col = col.push(entities(declared, sp));

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
                // Can be normal on zenoh 1.10+: loopback listen endpoints
                // may be filtered from the admin doc (all of them on 1.10.0,
                // eclipse-zenoh/zenoh#2671; from 1.10.1, only those an
                // unspecified listener resolves to; #155) — said only when
                // this node's own version says the filter applies, so an
                // older doc's blank stays a blank.
                if r.version
                    .as_deref()
                    .is_some_and(zenkey_fleet::admin_doc_omits_loopback)
                {
                    "no locators listed — zenoh 1.10+ may omit loopback listen endpoints \
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

fn storages<'a>(
    list: &'a [StorageInfo],
    state: &'a AdminState,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = column![kit::section_header("Storages", None)].spacing(sp.xs);
    if list.is_empty() {
        // Verbatim from `zenctl storage list`.
        col = col.push(kit::muted(
            "no storages found in the admin space — a peer-only mesh, a router without \
             the storage manager, or the admin space is disabled.",
        ));
        return col.into();
    }
    for s in list {
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

/// The centrepiece since FL9: each storage against S4 (core §4.2) — does it
/// answer on an owner's state in this namespace? A finding, clean, or not
/// judged, each its own row; and what S4 says instead, once.
fn s4_table(sweep: &AdminSweep, sp: Spacing) -> Element<'_, Message> {
    let mut col =
        column![kit::section_header("S4: storages on owners' state", None)].spacing(sp.xs);
    col = col.push(kit::muted(format!(
        "judged in {} against every owner's state/** and @state/** — named apart, \
         because * and ** never cross a verbatim chunk (core §1.3)",
        if sweep.namespace.is_empty() {
            "the bus root".to_string()
        } else {
            format!("namespace {:?}", sweep.namespace)
        }
    )));
    if sweep.s4.is_empty() {
        col = col.push(kit::muted(
            "no storage to judge — the sweep found none (above)",
        ));
        return col.into();
    }
    let findings = sweep
        .s4
        .iter()
        .filter(|r| matches!(r.on_state, OnState::Finding(_)))
        .count();
    for r in &sweep.s4 {
        col = col.push(s4_row(r, sp));
    }
    if findings > 0 {
        // S4's consequence, in the doctor's words: the finding is what the
        // owner's authority makes of a storage there.
        col = col.push(kit::muted(
            "a deployment MUST NOT run a storage on owners' state (core §4.2 S4): a late \
             joiner seeds from the owner, and last-known state is an archive.v1's (§4.4)",
        ));
    }
    col.into()
}

fn s4_row(r: &S4Row, sp: Spacing) -> Element<'_, Message> {
    let (tone, said, detail) = match &r.on_state {
        OnState::Finding(on) => (
            JudgementTone::Established,
            "answers on owner state",
            format!("intersects {}", on.join(", ")),
        ),
        OnState::Clean => (
            JudgementTone::NotEstablished,
            "not on owner state",
            "no owner's state in this namespace is under it".to_string(),
        ),
        OnState::Unjudged(why) => (JudgementTone::Unobservable, "not judged", why.clone()),
    };
    kit::card(
        row![
            kit::badge_judgement(tone, said),
            kit::mono(r.storage.clone()),
            kit::muted(r.key_expr.clone().unwrap_or_else(|| "-".into())),
            kit::muted(detail),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center),
    )
}

/// Declared entities — the `Option` here is the only thing separating "the
/// admin space answered and declared nothing" from "no answer (yet)", which
/// is #70's explicit reachability ask.
fn entities(declared: Option<&DeclaredEntities>, sp: Spacing) -> Element<'_, Message> {
    let mut col = column![kit::section_header("Declared entities", None)].spacing(sp.xs);
    let Some(declared) = declared else {
        col = col.push(kit::muted(
            "declared entities: n/a — not read yet, or nothing answered. zenoh's \
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
