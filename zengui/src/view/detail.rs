//! The selection detail pane — §6.4 item 5's inspector plus issue #66's
//! metadata view, merged: one selection, everything honestly known about it.
//!
//! Three sections, each stating its provenance:
//!
//! - **Key facts** — the RFC 09 §5.1 ladder verdict, worded per rung;
//! - **Registry** — the declared metadata, when the subject is registered
//!   (fields absent are absent, never defaulted);
//! - **Value** — the on-demand fetch (which rung answered), rendered **hex
//!   beside decoded**: the decoded side is schema-decoded named fields when
//!   RFC 08 §7 resolves, and the honest ladder otherwise — tagged with *how*
//!   it was produced, because schema-decoded and sniffed must never look
//!   alike.
//! - **Series** (issue #64) — sparklines for a numeric leaf and for the
//!   observed rate, drawn from the same history the timeline reads, with gaps
//!   drawn as gaps. Absent entirely when there is nothing numeric to plot: an
//!   explorer that renders an empty chart for a string payload has invented an
//!   error state out of an ordinary fact.

use iced::widget::{Column, button, row, text};
use iced::{Element, Length};
use zenkey_fleet::decode::Rendering;
use zenkey_fleet::{FetchOutcome, KeyFacts, KeyShape, Registration};

use crate::message::{Message, RightPane};
use crate::series::{NumericLeaves, Series};
use crate::view::kit;
use crate::view::spark;
use crate::view::theme::{RegistrationTone, SeriesTone, colors};
use crate::view::tokens::{font, space};

/// How much payload the hex view shows before truncating (with a note).
const HEX_VIEW_BYTES: usize = 1024;

/// Messages the pane emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailMsg {
    /// A numeric leaf was picked for the value sparkline.
    LeafSelected(String),
}

/// What the app hands the pane.
pub struct DetailData<'a> {
    pub key: &'a str,
    pub facts: Option<&'a KeyFacts>,
    pub fetched: Option<&'a Result<std::sync::Arc<FetchOutcome>, String>>,
    /// The decode of the fetched value, when it has completed:
    /// (declared type name if any, rendering).
    pub decoded: Option<&'a (Option<String>, Rendering)>,
    /// The plotted series (issue #64). Owned rather than borrowed: they are
    /// derived per frame from the history ring, and a pane cannot borrow a
    /// per-frame local.
    pub series: Option<SeriesData>,
    /// How many history entries have been recorded for this key, when a
    /// recording is running — the link into the history pane.
    pub history_entries: Option<usize>,
}

/// Everything the series section plots, computed by the app from the history
/// ring the timeline reads.
pub struct SeriesData {
    /// The numeric leaves the newest payload offers.
    pub leaves: NumericLeaves,
    /// Which one is plotted.
    pub leaf: Option<String>,
    /// That leaf's value over the retained history.
    pub value: Series,
    /// The observed rate, one point per stats tick.
    pub rate: Series,
    /// The registry-declared unit, when the subject declares one.
    pub unit: Option<String>,
}

pub fn pane<'a>(data: DetailData<'a>) -> Element<'a, Message> {
    let mut col = Column::new().spacing(space::SM);
    col = col.push(kit::section_header("Detail", None));
    col = col.push(kit::mono(data.key.to_string()));

    // — Key facts: the ladder verdict, worded per rung.
    match data.facts {
        None => {
            col = col.push(kit::muted(
                "no projection yet — facts appear once the key is observed or fetched",
            ));
        }
        Some(f) => {
            col = col.push(facts_section(f));
        }
    }

    // — Value: the fetch outcome + hex-beside-decoded.
    match data.fetched {
        None => {
            col = col.push(kit::muted(
                "no value fetched — selecting a concrete key fetches once \
                 (storage → cache → window), nothing ambient",
            ));
        }
        Some(Err(e)) => {
            col = col.push(
                text(format!("fetch failed: {e}"))
                    .size(font::CAPTION)
                    .style(|theme: &iced::Theme| text::Style {
                        color: Some(colors(theme).danger()),
                    }),
            );
        }
        Some(Ok(outcome)) => match outcome.as_ref() {
            FetchOutcome::None { attempted } => {
                col = col.push(kit::muted(format!(
                    "no value — asked {} — a non-verdict, not proof of absence \
                     (RFC 05 §3.1)",
                    attempted.join(", ")
                )));
            }
            FetchOutcome::Value(v) => {
                col = col.push(kit::muted(format!(
                    "value: {} bytes via {:?} · encoding {}",
                    v.payload.len(),
                    v.source,
                    if v.encoding.is_empty() {
                        "(unset)"
                    } else {
                        &v.encoding
                    },
                )));
                let bytes: Vec<u8> = v.payload.to_bytes().to_vec();
                let len = bytes.len();
                col = col.push(
                    row![hex_pane(bytes), decoded_pane(data.decoded, len)].spacing(space::MD),
                );
            }
        },
    }

    // — Series: sparklines over the recorded history (issue #64).
    if let Some(series) = data.series
        && let Some(section) = series_section(&series)
    {
        col = col.push(section);
    }

    // — The link into the timeline, so the two panes are not two islands.
    if let Some(n) = data.history_entries {
        col = col.push(
            button(
                text(format!(
                    "history: {} recorded — open (Alt 8)",
                    kit::plural(n, "sample")
                ))
                .size(font::CAPTION),
            )
            .on_press(Message::PaneSelected(RightPane::History))
            .style(iced::widget::button::text)
            .padding(0),
        );
    }

    iced::widget::scrollable(col).height(Length::Fill).into()
}

/// The sparkline section, or `None` when there is nothing numeric to plot.
///
/// Returning `None` is the point: a payload that carries no number is an
/// ordinary fact, and rendering an empty chart or an error for it would invent
/// a problem (#64's second acceptance line).
fn series_section<'a>(data: &SeriesData) -> Option<Element<'a, Message>> {
    let plottable = !data.leaves.leaves.is_empty();
    if !plottable && !data.rate.has_data() {
        return None;
    }
    let mut col = Column::new().spacing(space::XS);
    col = col.push(kit::section_header("Series", None));
    col = col.push(kit::muted(
        "plotted from the recorded history's structural values — a schema decode \
         per sample would hit the bus on a render path, so a protobuf or CDR leaf \
         offers no chart",
    ));

    if plottable {
        // The leaf picker. Small buttons rather than a dropdown: the list is
        // short by construction and the choice is one click either way.
        let mut picker = row![].spacing(space::XS);
        for (path, _) in &data.leaves.leaves {
            let active = data.leaf.as_deref() == Some(path.as_str());
            picker = picker.push(kit::tab(
                path.clone(),
                active,
                Message::Detail(DetailMsg::LeafSelected(path.clone())),
            ));
        }
        col = col.push(iced::widget::scrollable(picker).width(Length::Fill));
        if data.leaves.truncated > 0 {
            col = col.push(kit::muted(format!(
                "… {} not offered (the leaf list is bounded)",
                kit::plural(data.leaves.truncated, "further numeric leaf"),
            )));
        }
        let label = data.leaf.as_deref().unwrap_or("value");
        col = col.push(spark::chart(
            label,
            &data.value,
            SeriesTone::Value,
            data.unit.as_deref(),
        ));
    }

    col = col.push(spark::chart("rate", &data.rate, SeriesTone::Rate, None));
    Some(col.into())
}

pub(crate) fn facts_section(f: &KeyFacts) -> Element<'_, Message> {
    let mut col = Column::new().spacing(2);
    match &f.shape {
        KeyShape::V1(v) => {
            col = col.push(kit::muted(format!(
                "origin {} ({:?}) · class {}{}",
                v.origin,
                v.origin_kind,
                v.class,
                v.producer
                    .as_deref()
                    .map(|p| format!(" · producer {p}"))
                    .unwrap_or_default(),
            )));
        }
        KeyShape::NotUnderBase => {
            col = col.push(kit::muted(
                "under a different deployment base (RFC 03 §1.1) — a fact, not an error",
            ));
        }
        KeyShape::Unparsed { reason } => {
            col = col.push(kit::muted(format!(
                "not a keyspace-v2 key (O1: a fact): {reason}"
            )));
        }
    }
    match &f.registration {
        Registration::Registered(s) => {
            col = col.push(kit::tone_badge(RegistrationTone::Registered, "registered"));
            let mut meta = format!("subject {} · type {}", s.path, s.type_name);
            if let Some(u) = &s.unit {
                meta.push_str(&format!(" · unit {u}"));
            }
            if let Some(q) = &s.qos {
                meta.push_str(&format!(" · qos {q}"));
            }
            if let Some(t) = s.ttl_s {
                meta.push_str(&format!(" · ttl {t}s"));
            }
            if let Some(e) = &s.encoding {
                meta.push_str(&format!(" · encoding {e}"));
            }
            col = col.push(kit::muted(meta));
            if !s.vars.is_empty() {
                col = col.push(kit::muted(
                    s.vars
                        .iter()
                        .map(|(k, v)| format!("{k} = {v}"))
                        .collect::<Vec<_>>()
                        .join(" · "),
                ));
            }
        }
        Registration::Unregistered => {
            col = col.push(kit::tone_badge(
                RegistrationTone::Unregistered,
                "unregistered",
            ));
        }
        Registration::NoSliceForProducer => {
            col = col.push(kit::tone_badge(RegistrationTone::NoSlice, "no slice"));
        }
        Registration::Unknown => {
            col = col.push(kit::tone_badge(
                RegistrationTone::Unknown,
                "registry not asked",
            ));
        }
        Registration::NotApplicable => {}
    }
    col.into()
}

/// The hex side: offset + bytes, bounded, truncation stated.
fn hex_pane<'a>(bytes: Vec<u8>) -> Element<'a, Message> {
    let shown = &bytes[..bytes.len().min(HEX_VIEW_BYTES)];
    let mut out = String::with_capacity(shown.len() * 4);
    for (i, chunk) in shown.chunks(16).enumerate() {
        out.push_str(&format!("{:06x}  ", i * 16));
        for b in chunk {
            out.push_str(&format!("{b:02x} "));
        }
        out.push('\n');
    }
    let mut col = Column::new().spacing(2);
    col = col.push(kit::muted("hex"));
    col = col.push(kit::mono(out));
    if bytes.len() > HEX_VIEW_BYTES {
        col = col.push(kit::muted(format!(
            "… {} more bytes not shown",
            bytes.len() - HEX_VIEW_BYTES
        )));
    }
    iced::widget::container(col)
        .width(Length::FillPortion(1))
        .into()
}

/// The decoded side, tagged with how it was produced.
fn decoded_pane<'a>(
    decoded: Option<&'a (Option<String>, Rendering)>,
    payload_len: usize,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(2);
    match decoded {
        None => {
            col = col.push(kit::muted("decoding…"));
        }
        Some((type_name, rendering)) => match rendering {
            Rendering::Typed(d) => {
                col = col.push(kit::tone_badge(
                    RegistrationTone::Registered,
                    format!("schema-decoded <{}>", type_name.as_deref().unwrap_or("?")),
                ));
                col = col.push(kit::mono(
                    serde_json::to_string_pretty(&d.value).unwrap_or_default(),
                ));
                for note in &d.notes {
                    col = col.push(kit::muted(format!("note: {note}")));
                }
            }
            Rendering::Structural(s) => {
                // The honest ladder: typed-but-undecoded vs plain structural.
                let tag = match type_name {
                    Some(t) => format!("<{t}?> structural (schema did not decode)"),
                    None => "structural (no schema resolves — RFC 08 §7's fallback)".into(),
                };
                col = col.push(kit::muted(tag));
                col = col.push(kit::mono(if s.is_empty() {
                    format!("<{payload_len} bytes>")
                } else {
                    s.clone()
                }));
            }
        },
    }
    iced::widget::container(col)
        .width(Length::FillPortion(1))
        .into()
}
