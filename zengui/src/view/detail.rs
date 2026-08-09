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

use iced::widget::{Column, row, text};
use iced::{Element, Length};
use zenkey_fleet::decode::Rendering;
use zenkey_fleet::{FetchOutcome, KeyFacts, KeyShape, Registration};

use crate::message::Message;
use crate::view::kit;
use crate::view::theme::{RegistrationTone, colors};
use crate::view::tokens::{font, space};

/// How much payload the hex view shows before truncating (with a note).
const HEX_VIEW_BYTES: usize = 1024;

/// What the app hands the pane.
pub struct DetailData<'a> {
    pub key: &'a str,
    pub facts: Option<&'a KeyFacts>,
    pub fetched: Option<&'a Result<std::sync::Arc<FetchOutcome>, String>>,
    /// The decode of the fetched value, when it has completed:
    /// (declared type name if any, rendering).
    pub decoded: Option<&'a (Option<String>, Rendering)>,
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

    iced::widget::scrollable(col).height(Length::Fill).into()
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
