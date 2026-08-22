//! The media viewer (issue #69) — the preview rung first.
//!
//! RFC 07 §1 shapes everything here: `@media` carries raw encoded frames on
//! **exact keys** — the viewer never wildcards anything (the pin lives in
//! [`crate::scope::media_key`], the single place a media key is built), the
//! codec rides the wire `Encoding`, and frame metadata rides the attachment.
//! Discovery is v1.16's gift (#77): declared streams come off the bus in
//! the introspect slices, so the pane can enumerate shapes with nothing
//! compiled in. Image-decodable rungs render; video rungs are **listed as
//! metadata only** until a decode story exists — shown, never pretended.
//!
//! Unviewed streams cost zero subscriptions: the watch is declared on
//! *view*, released on *stop*, and nothing is subscribed for merely opening
//! the pane (issue #85's posture, applied to the highest-bandwidth plane).

use std::collections::VecDeque;
use std::time::Instant;

use iced::Length;
use iced::widget::{Column, button, column, row, text, text_input};
use zenkey_fleet::{SampleView, SliceSet, WatchId};

use super::kit;
use super::tokens::{font, space};
use crate::message::{Message, PaneMsg};

/// Media pane interactions.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaMsg {
    OriginChanged(String),
    ProducerChanged(String),
    SubpathChanged(String),
    /// A declared stream row was clicked — prefill producer + path shape.
    DeclPicked {
        producer: String,
        path: String,
    },
    /// Subscribe to the composed exact key.
    View,
    /// The watch was declared (or refused).
    Watched(Result<WatchId, String>),
    /// Release the watch.
    Stop,
    /// The watch was released.
    Stopped,
}

/// How many arrival instants the fps window keeps.
pub const FPS_WINDOW: usize = 30;

/// One live viewing: the exact key, its watch, and what arrived.
#[derive(Debug)]
pub struct Viewing {
    pub key: String,
    pub watch: Option<WatchId>,
    /// The newest frame's bytes and declared encoding — handed to the image
    /// widget when the encoding is image-decodable.
    pub frame: Option<Frame>,
    /// The newest frame's attachment, rendered structurally (never
    /// schema-decoded — RFC 07 §1 puts metadata on the attachment).
    pub meta: Option<String>,
    pub frames: u64,
    pub bytes: u64,
    /// Arrival instants of the last [`FPS_WINDOW`] frames — the fps shown
    /// is measured client-side, over this window, and labeled as such.
    pub arrivals: VecDeque<Instant>,
}

impl Viewing {
    pub fn new(key: String) -> Viewing {
        Viewing {
            key,
            watch: None,
            frame: None,
            meta: None,
            frames: 0,
            bytes: 0,
            arrivals: VecDeque::with_capacity(FPS_WINDOW),
        }
    }

    /// Feed one arrived frame.
    pub fn on_frame(&mut self, sample: &SampleView) {
        let bytes = sample.payload.to_bytes();
        self.frames += 1;
        self.bytes += bytes.len() as u64;
        if self.arrivals.len() == FPS_WINDOW {
            self.arrivals.pop_front();
        }
        self.arrivals.push_back(sample.received);
        // The decode happens **here**, once per arrival, not once per redraw
        // (#178). `Handle` is reference-counted, so the view clones a pointer;
        // `Handle::from_bytes(bytes.clone())` in the view copied the whole
        // frame ~60 times a second, on top of the one copy above that is
        // actually needed.
        self.frame = Some(Frame {
            handle: decodable(&sample.encoding)
                .then(|| iced::widget::image::Handle::from_bytes(bytes.to_vec())),
            encoding: sample.encoding.clone(),
            len: bytes.len(),
        });
        self.meta = sample.attachment.as_ref().map(|a| {
            let b = a.to_bytes();
            match serde_json::from_slice::<serde_json::Value>(&b) {
                Ok(v) => v.to_string(),
                Err(_) => String::from_utf8_lossy(&b).to_string(),
            }
        });
    }

    /// Frames per second over the arrival window; `None` below two frames.
    pub fn fps(&self) -> Option<f64> {
        let (first, last) = (self.arrivals.front()?, self.arrivals.back()?);
        let span = last.saturating_duration_since(*first).as_secs_f64();
        (self.arrivals.len() >= 2 && span > 0.0).then(|| (self.arrivals.len() - 1) as f64 / span)
    }
}

/// The newest frame, decoded once on arrival.
///
/// Holding the `Handle` rather than the bytes is the point: it is
/// reference-counted, so a redraw clones a pointer. The bytes are kept only
/// inside it, and only when the codec is one this app can actually draw —
/// `len` and `encoding` are what the undecodable branch has to say, and it
/// should not need the payload to say it.
#[derive(Debug, Clone)]
pub struct Frame {
    /// `Some` iff [`decodable`] accepted the encoding.
    pub handle: Option<iced::widget::image::Handle>,
    pub encoding: String,
    pub len: usize,
}

/// The pane's state.
#[derive(Debug, Default)]
pub struct MediaState {
    pub origin: String,
    pub producer: String,
    pub subpath: String,
    pub viewing: Option<Viewing>,
    pub error: Option<String>,
}

/// Whether iced can decode this declared encoding today. The honest list,
/// not an aspiration: everything else is listed as metadata only.
pub fn decodable(encoding: &str) -> bool {
    matches!(
        encoding,
        "image/png" | "image/jpeg" | "image/gif" | "image/bmp" | "image/webp"
    )
}

/// Wrap one of this pane's messages for the app (#176).
///
/// One place the pane's name is spelled, rather than at every widget —
/// which is what the other seven panes already did, and what makes the
/// six-group regroup a one-line change here instead of 6.
fn msg(m: MediaMsg) -> Message {
    Message::Pane(PaneMsg::Media(m))
}

/// The Inspector's `@media`-plane sections (#182). See
/// [`super::detail::section`] for why this is a `Column`.
pub fn section<'a>(state: &'a MediaState, slices: Option<&'a SliceSet>) -> Column<'a, Message> {
    let mut col = column![kit::section_header("Media", None)].spacing(space::SM);

    // Declared streams, off the bus (#77): the enumeration RFC 07 §1's
    // no-wildcard rule depends on. No registry loaded = "not asked" (O4).
    match slices {
        None => {
            col = col.push(kit::muted(
                "no registry loaded — declared streams unknown (not asked, O4); \
                 the bus serves them via introspect (RFC 08 §6, v1.16)",
            ));
        }
        Some(slices) => {
            let declaring: Vec<_> = slices
                .slices()
                .iter()
                .filter(|s| !s.media.is_empty())
                .collect();
            if declaring.is_empty() {
                col = col.push(kit::muted(
                    "no producer declares a media stream — an observation over the \
                     loaded slices, not a verdict about the bus",
                ));
            }
            for slice in declaring {
                col = col.push(kit::muted(format!("{} declares:", slice.name)));
                for m in &slice.media {
                    let label = format!(
                        "  {} ({}){}",
                        m.path,
                        m.encoding,
                        if decodable(&m.encoding) {
                            ""
                        } else {
                            " — metadata only, no decode story yet"
                        }
                    );
                    if decodable(&m.encoding) {
                        col = col.push(
                            button(kit::caption(label))
                                .on_press(msg(MediaMsg::DeclPicked {
                                    producer: slice.name.clone(),
                                    path: m.path.clone(),
                                }))
                                .padding(2),
                        );
                    } else {
                        col = col.push(kit::muted(label));
                    }
                }
            }
        }
    }

    let origin = text_input("origin: h-… (exact — never a wildcard)", &state.origin)
        .on_input(|s| msg(MediaMsg::OriginChanged(s)))
        .size(font::CAPTION);
    let producer = text_input("producer", &state.producer)
        .on_input(|s| msg(MediaMsg::ProducerChanged(s)))
        .size(font::CAPTION);
    let subpath = text_input(
        "stream path: fill every {var} (e.g. cam0/preview/png)",
        &state.subpath,
    )
    .on_input(|s| msg(MediaMsg::SubpathChanged(s)))
    .size(font::CAPTION);
    let controls = row![
        origin,
        producer,
        subpath,
        match &state.viewing {
            None => button(kit::caption("view"))
                .on_press(msg(MediaMsg::View))
                .padding(4),
            Some(_) => button(kit::caption("stop"))
                .on_press(msg(MediaMsg::Stop))
                .padding(4),
        },
    ]
    .spacing(space::SM)
    .align_y(iced::Alignment::Center);
    col = col.push(controls);

    if let Some(e) = &state.error {
        col = col.push(
            kit::body(e.clone()).style(|theme: &iced::Theme| text::Style {
                color: Some(super::theme::colors(theme).danger()),
            }),
        );
    }

    if let Some(v) = &state.viewing {
        col = col.push(kit::mono(format!("▶ {}", v.key)));
        let stats = format!(
            "{} frame(s) · {} · {} (measured client-side, arrival clock)",
            v.frames,
            kit::human_bytes(v.bytes),
            match v.fps() {
                Some(f) => format!("{f:.1} fps"),
                None => "fps: not yet measurable".to_string(),
            }
        );
        col = col.push(kit::muted(stats));
        match &v.frame {
            None => {
                col = col.push(kit::muted(
                    "subscribed — no frame yet (a stream with no viewer may cost \
                     nothing to publish either; RFC 07 §1's demand-driven tiers)",
                ));
            }
            Some(Frame {
                handle: Some(handle),
                ..
            }) => {
                col = col.push(iced::widget::image(handle.clone()).width(Length::Fill));
            }
            Some(frame) => {
                col = col.push(kit::muted(format!(
                    "frame arrived: {} B as {} — no decode story for this \
                     codec; shown as metadata only (RFC 07 §1: the codec is the \
                     wire Encoding, and pretending to render it would be a lie)",
                    frame.len, frame.encoding
                )));
            }
        }
        if let Some(meta) = &v.meta {
            col = col.push(kit::muted(format!("frame meta (attachment): {meta}")));
        }
    }

    col
}
