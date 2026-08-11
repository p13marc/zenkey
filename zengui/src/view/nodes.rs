//! The node dashboard (#61): one card per origin, fed by liveliness (free)
//! and the watched-only freshness join; per-node detail loads on selection
//! through one `node_info` GET — the pane's only data-plane cost (the
//! laziness ground rule).

use std::sync::Arc;

use iced::widget::{column, row, scrollable, text};
use iced::{Element, Length};
use zenkey_fleet::NodeInfo;

use crate::message::Message;
use crate::nodes::{CatalogPresence, NodeRoster, ProducerPresence};
use crate::view::kit;
use crate::view::theme::{PresenceTone, colors};
use crate::view::tokens::{font, space};

/// The pane's interactions, nested per the `CallMsg` precedent.
#[derive(Debug, Clone)]
pub enum NodesMsg {
    /// An origin card was selected — loads its `node_info` one-shot.
    Selected(String),
    /// The one-shot `node_info` landed.
    InfoLoaded(String, String, Result<Arc<NodeInfo>, String>),
    /// Click-through: land on the origin's subtree in the key tree.
    ShowInTree(String),
}

/// The selected node's detail state — a value, an error, or "not asked".
#[derive(Debug, Default)]
pub enum DetailState {
    #[default]
    NotAsked,
    Loading(String),
    Loaded(String, Result<Arc<NodeInfo>, String>),
}

/// Everything the pane renders (args struct per the `DetailData` precedent).
pub struct NodesData<'a> {
    pub roster: &'a NodeRoster,
    pub selected: Option<&'a str>,
    pub detail: &'a DetailState,
}

pub fn pane(d: NodesData<'_>) -> Element<'_, Message> {
    let mut col = column![kit::section_header("nodes", None)].spacing(space::SM);

    // The catalog line comes first, always, by name: "catalog dead" and
    // "no entities" must never look alike (D4 / RFC 04 §5).
    col = col.push(match d.roster.catalog() {
        CatalogPresence::Alive => kit::badge_presence(PresenceTone::Alive, "catalog: alive"),
        CatalogPresence::Suspect => {
            kit::badge_presence(PresenceTone::Suspect, "catalog: suspect (token retracted)")
        }
        CatalogPresence::NoTokenObserved => kit::badge_presence(
            PresenceTone::Unknown,
            "catalog: no token observed — not proof none exists (RFC 05 §3.1)",
        ),
    });

    if !d.roster.is_seeded() {
        col = col.push(kit::empty_state(
            "no presence asked yet",
            "the roster seeds on connect from liveliness tokens (RFC 04 §5)",
        ));
        return col.into();
    }
    if d.roster.is_empty() {
        col = col.push(kit::empty_state(
            "no liveliness tokens observed",
            "producers may be down, unreachable, or holding no tokens — \
             silence is not a verdict (RFC 05 §3.1)",
        ));
        return col.into();
    }

    let mut cards = column![].spacing(space::SM);
    for (origin, producers) in d.roster.iter() {
        let selected = d.selected == Some(origin.as_str());
        cards = cards.push(origin_card(origin, producers, selected, d.detail));
    }
    col = col.push(scrollable(cards).height(Length::Fill));
    col.into()
}

fn origin_card<'a>(
    origin: &'a str,
    producers: &'a std::collections::BTreeMap<String, ProducerPresence>,
    selected: bool,
    detail: &'a DetailState,
) -> Element<'a, Message> {
    let mut body = column![].spacing(space::XS);

    let header = row![
        iced::widget::button(
            text(origin)
                .size(font::EMPHASIS)
                .font(iced::Font::MONOSPACE)
        )
        .style(iced::widget::button::text)
        .padding(0)
        .on_press(Message::Nodes(NodesMsg::Selected(origin.to_string()))),
        iced::widget::space::horizontal(),
        iced::widget::button(text("show in tree").size(font::CAPTION))
            .padding(2)
            .on_press(Message::Nodes(NodesMsg::ShowInTree(origin.to_string()))),
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);
    body = body.push(header);

    for (producer, p) in producers {
        let (tone, presence_label) = if p.alive {
            (PresenceTone::Alive, "alive".to_string())
        } else {
            let since = p
                .suspect_since
                .map(|t| format!(" since {}s", t.elapsed().as_secs()))
                .unwrap_or_default();
            (
                PresenceTone::Suspect,
                format!("suspect{since} (token retracted)"),
            )
        };
        let freshness = match (p.watched, p.last_state_age) {
            (true, Some(age)) => format!("last state sample {}s ago", age.as_secs()),
            (true, None) => "watched — no state sample seen".to_string(),
            (false, _) => "not watched — freshness unknown".to_string(),
        };
        body = body.push(
            row![
                kit::badge_presence(tone, format!("{producer}: {presence_label}")),
                iced::widget::space::horizontal(),
                kit::muted(freshness),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        );
    }

    if selected {
        body = body.push(detail_view(detail));
    }
    kit::card(body)
}

fn detail_view(detail: &DetailState) -> Element<'_, Message> {
    match detail {
        DetailState::NotAsked => kit::muted("select to ask node_info"),
        DetailState::Loading(origin) => kit::muted(format!("asking node_info for {origin}…")),
        DetailState::Loaded(_, Err(e)) => text(format!("node_info failed: {e}"))
            .size(font::CAPTION)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            })
            .into(),
        DetailState::Loaded(_, Ok(info)) => {
            let mut col = column![].spacing(space::XS);
            if info.producers.is_empty() {
                col = col.push(kit::muted(
                    "no introspect reply — capabilities unknown, not absent",
                ));
            }
            for p in &info.producers {
                let caps = match (&p.app, &p.registry_version) {
                    (Some(app), Some(v)) => format!(
                        "app {app} · registry v{v} · {} subject(s) · {} procedure(s){}{}",
                        p.subjects,
                        p.procedures,
                        if p.blob_tiers.is_empty() {
                            String::new()
                        } else {
                            format!(" · blob: {}", p.blob_tiers.join(","))
                        },
                        if p.deprecated_served > 0 {
                            format!(" · {} DEPRECATED still served", p.deprecated_served)
                        } else {
                            String::new()
                        },
                    ),
                    _ => "no introspect reply — capabilities unknown, not absent".to_string(),
                };
                col = col.push(kit::muted(format!("{}: {caps}", p.name)));
            }
            // Freshness rows exist only for ttl-declaring subjects; their
            // absence is "no ttl declared", never "fresh" (O4).
            if info.freshness.is_empty() {
                col = col.push(kit::muted(
                    "no ttl declared on any state subject — freshness unknown (O4)",
                ));
            } else {
                for f in &info.freshness {
                    let age = f
                        .age_s
                        .map(|a| format!("{a}s old"))
                        .unwrap_or_else(|| "no sample answered — stale".into());
                    let line = format!(
                        "{}/{}  {age}  (ttl {}s){}",
                        f.producer,
                        f.path,
                        f.ttl_s,
                        if f.stale { "  STALE" } else { "" }
                    );
                    if f.stale {
                        col = col.push(text(line).size(font::CAPTION).style(
                            |theme: &iced::Theme| text::Style {
                                color: Some(colors(theme).danger()),
                            },
                        ));
                    } else {
                        col = col.push(kit::muted(line));
                    }
                }
            }
            col.into()
        }
    }
}
