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

use crate::message::{Message, RightPane, WorkspaceMsg};
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
    /// The chart data, borrowed — the app rebuilds it when its inputs move,
    /// not when the frame does (#178).
    pub series: Option<&'a SeriesData>,
    /// How many history entries have been recorded for this key, when a
    /// recording is running — the link into the history pane.
    pub history_entries: Option<usize>,
    /// The newest recorded sample for this key, when observation has seen
    /// one — the source of the *observed* QoS axes (#120).
    pub observed: Option<&'a crate::history::HistoryEntry>,
    /// The key's observed skewed-latency summary and unstamped count,
    /// refreshed per bus tick (#119). Absent = nothing stamped, which is
    /// not zero latency.
    pub latency: Option<(zenkey_fleet::LatencyReport, u64)>,
}

/// Microseconds humanised with the sign kept — a negative latency is the
/// skew evidence (#119), same spelling as `zenctl topic hz --latency`.
pub fn human_us(us: i64) -> String {
    let sign = if us < 0 { "-" } else { "" };
    let abs = us.unsigned_abs();
    if abs >= 1_000_000 {
        format!("{sign}{:.2}s", abs as f64 / 1_000_000.0)
    } else if abs >= 1_000 {
        format!("{sign}{:.1}ms", abs as f64 / 1_000.0)
    } else {
        format!("{sign}{abs}µs")
    }
}

/// The wire's QoS axes as one stable lowercase token — the same spelling
/// `zenctl topic echo --fmt %q` prints, so the two frontends agree.
pub fn qos_token(entry: &crate::history::HistoryEntry) -> String {
    use zenoh::qos::{CongestionControl as Cc, Priority as P, Reliability as R};
    let p = match entry.priority {
        P::RealTime => "real_time",
        P::InteractiveHigh => "interactive_high",
        P::InteractiveLow => "interactive_low",
        P::DataHigh => "data_high",
        P::Data => "data",
        P::DataLow => "data_low",
        P::Background => "background",
    };
    let c = match entry.congestion_control {
        Cc::Drop => "drop",
        Cc::Block => "block",
        _ => "other",
    };
    let r = match entry.reliability {
        R::BestEffort => "best_effort",
        R::Reliable => "reliable",
    };
    format!("{p}/{c}/{r}{}", if entry.express { "+express" } else { "" })
}

/// Declared-vs-observed (#120): the comparison nobody else in the field can
/// render, because nobody else holds a registry that declares QoS.
/// `None` = the declared name is not a profile, so there is nothing to
/// judge (O4).
pub fn qos_verdict(declared: &str, entry: &crate::history::HistoryEntry) -> Option<bool> {
    let profile = zenkey::qos::QosProfile::from_name(declared)?;
    Some(
        entry.priority == profile.priority()
            && entry.congestion_control == profile.congestion_control()
            && entry.reliability == profile.reliability()
            && entry.express == profile.express(),
    )
}

/// The declared-vs-observed QoS lines, when there is anything to say.
fn qos_section<'a>(data: &DetailData<'a>) -> Option<Element<'a, Message>> {
    let declared = data.facts.and_then(|f| match &f.registration {
        zenkey_fleet::Registration::Registered(s) => s.qos.clone(),
        _ => None,
    });
    let observed = data.observed;
    if declared.is_none() && observed.is_none() {
        return None;
    }
    let mut col = Column::new().spacing(2);
    col = col.push(kit::muted("QoS — declared vs observed (RFC 04 §3)"));
    match &declared {
        Some(q) => col = col.push(kit::mono(format!("declared: {q}"))),
        None => col = col.push(kit::muted("declared: (registry names no profile)")),
    }
    match observed {
        Some(entry) => {
            let mut line = format!("observed: {}", qos_token(entry));
            if let Some(src) = &entry.source {
                use std::fmt::Write as _;
                let _ = write!(line, "  ·  source {}:{}#{}", src.zid, src.eid, src.sn);
            }
            col = col.push(kit::mono(line));
            if let Some(declared) = &declared {
                match qos_verdict(declared, entry) {
                    Some(true) => {
                        col = col.push(kit::muted("observed axes match the declared profile"))
                    }
                    Some(false) => {
                        col = col.push(kit::muted(
                            "⚠ observed axes differ from the declared profile — the wire \
                             is the fact, the registry is the claim (RFC 04 §3)",
                        ));
                    }
                    None => {}
                }
            }
        }
        None => {
            col = col.push(kit::muted(
                "observed: nothing recorded yet — axes appear once the key is observed (O4)",
            ));
        }
    }
    Some(col.into())
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
    /// Retained geometry for the two charts (#178).
    ///
    /// A `canvas::Cache` has to outlive the frame to be worth anything, and
    /// this struct is now the thing that does: it is rebuilt when the chart's
    /// inputs move, which is exactly when the geometry stops being valid. A
    /// fresh `SeriesData` therefore *is* a cleared cache, and there is no
    /// second invalidation rule to keep in step with the first.
    ///
    /// `view/spark.rs` used to justify having no cache by the 250 ms tick —
    /// but that is the rate the *data* changes at, and a canvas redraws with
    /// the frame.
    pub caches: SeriesCaches,
}

/// The retained geometry, one per chart.
#[derive(Default)]
pub struct SeriesCaches {
    pub value: iced::widget::canvas::Cache,
    pub rate: iced::widget::canvas::Cache,
}

/// Wrap one of this pane's messages for the app (#176).
///
/// `Message::Workspace(WorkspaceMsg::PaneSelected)` below is deliberately *not* routed through
/// this: it names another region, and a pane reaching across is a fact
/// worth leaving visible at its call site.
fn msg(m: DetailMsg) -> Message {
    Message::Detail(m)
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
    if let Some(qos) = qos_section(&data) {
        col = col.push(qos);
    }
    if let Some((lat, unstamped)) = &data.latency {
        // One row per population. Folding a publisher-stamped sample and a
        // router-stamped one into a single median produces a number that
        // describes neither (#213).
        for (label, s) in lat.populations() {
            col = col.push(kit::mono(format!(
                "observed skewed latency [{label}]: med {} · p95 {} (min {} · max {}, {})",
                human_us(s.median_us),
                human_us(s.p95_us),
                human_us(s.min_us),
                human_us(s.max_us),
                s.samples,
            )));
        }
        col = col.push(kit::mono(format!("{unstamped} unstamped")));
        // The caveat comes from the engine, so this pane and `zenctl hz
        // --latency` cannot describe the same measurement differently.
        col = col.push(kit::muted(lat.caveat()));
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
                // Borrow through the Cow: `to_bytes()` is already free for a
                // contiguous payload, and `.to_vec()` on top of it was the
                // double copy the redesign flagged (`docs/zero-copy.md`).
                let bytes = v.payload.to_bytes();
                let len = bytes.len();
                col = col.push(
                    row![hex_pane(&bytes), decoded_pane(data.decoded, len)].spacing(space::MD),
                );
                // The attachment, when the value carried one (#117): rendered
                // structurally beside its hex — the registry does not describe
                // attachments, so structural is where the rendering stops.
                if let Some(att) = &v.attachment {
                    let abytes = att.to_bytes();
                    col = col.push(kit::muted(format!(
                        "attachment: {} bytes — rendered structurally; the registry \
                         does not describe attachments",
                        abytes.len()
                    )));
                    col = col.push(
                        row![
                            hex_pane(&abytes),
                            kit::mono(zenkey_fleet::decode::structural(&abytes))
                        ]
                        .spacing(space::MD),
                    );
                }
            }
        },
    }

    // — Series: sparklines over the recorded history (issue #64).
    if let Some(series) = data.series
        && let Some(section) = series_section(series)
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
            .on_press(Message::Workspace(WorkspaceMsg::PaneSelected(
                RightPane::History,
            )))
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
fn series_section<'a>(data: &'a SeriesData) -> Option<Element<'a, Message>> {
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
                msg(DetailMsg::LeafSelected(path.clone())),
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
            &data.caches.value,
        ));
    }

    col = col.push(spark::chart(
        "rate",
        &data.rate,
        SeriesTone::Rate,
        None,
        &data.caches.rate,
    ));
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
/// `000000  de ad be ef` — sixteen bytes to a row, offset first.
///
/// One pre-sized `String` and a nibble table, rather than a `format!` per byte
/// plus one per offset (#178). The pane runs at frame rate over up to 1,024
/// bytes, so it was ~1,090 allocations per redraw for a rendering that has not
/// changed since the last one. `docs/zero-copy.md` records this pane being
/// fixed once already, for the double *copy*; the allocation churn is a
/// different defect at the same site.
///
/// Split out from the pane so the bytes-to-text half is testable — the old
/// version's exact output is what the test pins, because a hand-rolled
/// formatter is only an improvement if it agrees with `{:02x}`.
pub fn hex_dump(bytes: &[u8]) -> String {
    /// Lowercase nibbles, indexed by the half-byte.
    const HEX: [u8; 16] = *b"0123456789abcdef";

    let rows = bytes.len().div_ceil(16);
    let mut out = String::with_capacity(rows * (8 + 16 * 3 + 1));
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let offset = i * 16;
        for shift in (0..24).step_by(4).rev() {
            out.push(HEX[(offset >> shift) & 0xf] as char);
        }
        out.push_str("  ");
        for b in chunk {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
            out.push(' ');
        }
        out.push('\n');
    }
    out
}

fn hex_pane<'a>(bytes: &[u8]) -> Element<'a, Message> {
    let shown = &bytes[..bytes.len().min(HEX_VIEW_BYTES)];
    let out = hex_dump(shown);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The hand-rolled formatter agrees with `{:06x}`/`{:02x}`, byte for byte,
    /// which is the only thing that makes it an improvement rather than a
    /// rewrite (#178). Checked against the exact expression it replaced.
    #[test]
    fn the_hex_dump_matches_the_formatter_it_replaced() {
        fn reference(bytes: &[u8]) -> String {
            let mut out = String::new();
            for (i, chunk) in bytes.chunks(16).enumerate() {
                out.push_str(&format!("{:06x}  ", i * 16));
                for b in chunk {
                    out.push_str(&format!("{b:02x} "));
                }
                out.push('\n');
            }
            out
        }
        for len in [0usize, 1, 15, 16, 17, 255, 256, 1024] {
            // A deterministic spread that exercises both nibbles of every
            // byte value, and offsets past 0xff so the six-digit offset is
            // not always five zeros and a digit.
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
            assert_eq!(hex_dump(&bytes), reference(&bytes), "len {len}");
        }
    }

    fn entry(profile: zenkey::qos::QosProfile) -> crate::history::HistoryEntry {
        crate::history::HistoryEntry {
            seq: 0,
            received: Instant::now(),
            timestamp: None,
            is_delete: false,
            len: 2,
            encoding: "application/json".into(),
            payload: zenoh::bytes::ZBytes::from("{}"),
            priority: profile.priority(),
            congestion_control: profile.congestion_control(),
            reliability: profile.reliability(),
            express: profile.express(),
            source: None,
            value: None,
            preview: "{}".into(),
        }
    }

    /// #120: the verdict compares axes, not names — and an undeclarable
    /// name judges nothing rather than judging falsely (O4).
    #[test]
    fn declared_vs_observed_judges_axes_not_names() {
        use zenkey::qos::QosProfile;
        let observed_alert = entry(QosProfile::Alert);
        assert_eq!(qos_verdict("alert", &observed_alert), Some(true));
        assert_eq!(qos_verdict("sampled", &observed_alert), Some(false));
        assert_eq!(qos_verdict("not-a-profile", &observed_alert), None);
    }

    /// The token spelling is byte-for-byte the CLI's %q, so the two
    /// frontends agree on what the wire said.
    #[test]
    fn the_token_matches_the_cli_spelling() {
        use zenkey::qos::QosProfile;
        let e = entry(QosProfile::Alert);
        // alert = interactive_high / block / reliable / express (RFC 04 §3).
        assert_eq!(qos_token(&e), "interactive_high/block/reliable+express");
    }
}
