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

use iced::widget::{Column, row, text};
use iced::{Element, Length};
use zenkey_fleet::model::decode::Rendering;
use zenkey_fleet::{FetchOutcome, KeyFacts, KeyShape, Registration};

use crate::message::{Message, PaneMsg, SlotId};
use crate::series::{NumericLeaves, Series};
use crate::value::DecodedValue;
use crate::view::kit;
use crate::view::spark;
use crate::view::theme::{RegistrationTone, SeriesTone, colors};
use crate::view::tokens::Spacing;

/// How much payload the hex view shows before truncating (with a note).
const HEX_VIEW_BYTES: usize = 1024;

/// How many characters of a rendered document this pane draws (#345).
///
/// Both bounded sides of a value row now say the same kind of thing: the hex
/// side stops at [`HEX_VIEW_BYTES`], the rendered side stops here, and each
/// says how much it left out. Before this the rendered side had no bound at
/// all — a megabyte of decoded document was re-serialized and re-copied into
/// a text widget on **every redraw**, next to a hex pane clamped at 1 KiB.
///
/// 64 KiB is far past what anyone reads in a pane and far short of what
/// costs a frame; a payload that wants more than this wants a file, not a
/// scroll (RFC 13 §3 O6 — the bound is stated where it bites).
const DOCUMENT_VIEW_CHARS: usize = 64 * 1024;

/// Clamp a rendered document to [`DOCUMENT_VIEW_CHARS`], on a char boundary.
///
/// Returns what to draw and how many bytes were left out — zero when it all
/// fits, which is the common case and renders no note.
fn clamp_document(s: &str) -> (&str, usize) {
    if s.len() <= DOCUMENT_VIEW_CHARS {
        return (s, 0);
    }
    let mut end = DOCUMENT_VIEW_CHARS;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], s.len() - end)
}

/// The structural rendering of an attachment, bounded on both axes (#345).
///
/// The decode used to run over the *whole* attachment on every redraw, with
/// no cap, beside a hex pane that deliberately clamps at 1 KiB. Two bounds
/// now, both stated: nothing over [`crate::echo::DECODE_LIMIT`] is decoded at
/// all — the same limit the echo line and the media viewer stop at — and what
/// is decoded is drawn up to [`DOCUMENT_VIEW_CHARS`].
fn attachment_pane<'a>(bytes: &[u8], sp: Spacing) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    if bytes.len() > crate::echo::DECODE_LIMIT {
        col = col.push(kit::mono(format!(
            "<{} bytes — too large to render structurally>",
            bytes.len()
        )));
        return col.into();
    }
    let rendered = zenkey_fleet::model::decode::structural(bytes);
    let (shown, elided) = clamp_document(&rendered);
    col = col.push(kit::mono(shown.to_string()));
    if elided > 0 {
        col = col.push(kit::muted(format!("… {elided} more bytes not shown")));
    }
    col.into()
}

/// Messages the pane emits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailMsg {
    /// A numeric leaf was picked for the value sparkline.
    LeafSelected(String),
}

/// What the app hands the pane.
/// What the pane knows about the fetch for the key it is showing (#181).
///
/// Three states, and the third is why this is an enum. An `Option` made
/// "superseded" indistinguishable from "not asked", so a fetch that landed for
/// a key the user had already moved past rendered as *nothing was asked* —
/// which is an O4 violation in the one pane whose whole job is saying what was
/// and was not asked.
#[derive(Debug, Clone, Copy, Default)]
pub enum Fetched<'a> {
    /// No fetch has landed for anything.
    #[default]
    NotAsked,
    /// A fetch landed, but for a different subject. The answer is real; it is
    /// just not about what is on screen.
    Superseded,
    /// The answer to the question this pane is showing.
    Landed(&'a Result<std::sync::Arc<FetchOutcome>, String>),
}

pub struct DetailData<'a> {
    /// The subject slot this section renders (#257) — every message it
    /// emits carries it home.
    pub slot: SlotId,
    pub key: &'a str,
    pub facts: Option<&'a KeyFacts>,
    pub fetched: Fetched<'a>,
    /// The decode of the fetched value, when it has completed — the whole
    /// [`DecodedValue`]: rendering, verdict, the decode error behind an
    /// `Undecodable` (#164), and the document rendered once (#345).
    pub decoded: Option<&'a DecodedValue>,
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
    /// The dock's resolved spacing grid (#192).
    pub sp: Spacing,
}

/// Microseconds humanised with the sign kept — a negative latency is the
/// skew evidence (#119), same spelling as `zenctl rate --latency`.
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
/// `zenctl echo --fmt %q` prints, so the two frontends agree.
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
    let mut col = Column::new().spacing(data.sp.xs);
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
fn msg(slot: SlotId, m: DetailMsg) -> Message {
    Message::Pane(PaneMsg::Detail(slot, m))
}

/// The Inspector's key sections, as a column the caller scrolls (#182).
///
/// It returns a `Column` rather than an `Element` because it is now composed
/// with other sections into one surface: a scrollable nested inside a
/// scrollable is a layout bug, so exactly one caller owns the scroll.
pub fn section<'a>(data: DetailData<'a>) -> Column<'a, Message> {
    let sp = data.sp;
    let mut col = Column::new().spacing(sp.sm);
    col = col.push(kit::section_header("Detail", None));
    // The subject's key, restated where its facts are. The window's one
    // TITLE moved to the location bar (#185) — the bar is where "where am I"
    // is answered now — so this line is a key value, not a page title.
    col = col.push(kit::emphasis(data.key).font(iced::Font::MONOSPACE));

    // — Key facts: the ladder verdict, worded per rung.
    match data.facts {
        None => {
            col = col.push(kit::muted(
                "no projection yet — facts appear once the key is observed or fetched",
            ));
        }
        Some(f) => {
            col = col.push(facts_section(f, sp));
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
        // The caveat comes from the engine, so this pane and `zenctl rate
        // --latency` cannot describe the same measurement differently.
        col = col.push(kit::muted(lat.caveat()));
    }

    // — Value: the fetch outcome + hex-beside-decoded.
    match data.fetched {
        Fetched::NotAsked => {
            col = col.push(kit::muted(
                "no value fetched — selecting a concrete key fetches once \
                 (storage → cache → window), nothing ambient",
            ));
        }
        Fetched::Superseded => {
            col = col.push(kit::muted(
                "superseded — the last fetch answered a different subject, so \
                 nothing here describes this key. Re-select it to ask again.",
            ));
        }
        Fetched::Landed(Err(e)) => {
            col = col.push(
                kit::body(format!("fetch failed: {e}")).style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                }),
            );
        }
        Fetched::Landed(Ok(outcome)) => match outcome.as_ref() {
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
                    row![hex_pane(&bytes, sp), decoded_pane(data.decoded, len, sp)].spacing(sp.md),
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
                        row![hex_pane(&abytes, sp), attachment_pane(&abytes, sp)].spacing(sp.md),
                    );
                }
            }
        },
    }

    // — Series: sparklines over the recorded history (issue #64).
    if let Some(series) = data.series
        && let Some(section) = series_section(series, data.slot, sp)
    {
        col = col.push(section);
    }

    // The count, and no longer a link: the timeline is the next section down
    // rather than another tab (#182). It used to read "open (Alt 8)", which
    // was the two-islands problem stated in its own label.
    if let Some(n) = data.history_entries {
        col = col.push(kit::muted(format!(
            "history: {} recorded",
            kit::plural(n, "sample")
        )));
    }

    col
}

/// The sparkline section, or `None` when there is nothing numeric to plot.
///
/// Returning `None` is the point: a payload that carries no number is an
/// ordinary fact, and rendering an empty chart or an error for it would invent
/// a problem (#64's second acceptance line).
fn series_section<'a>(
    data: &'a SeriesData,
    slot: SlotId,
    sp: Spacing,
) -> Option<Element<'a, Message>> {
    let plottable = !data.leaves.leaves.is_empty();
    if !plottable && !data.rate.has_data() {
        return None;
    }
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::section_header("Series", None));
    col = col.push(kit::muted(
        "plotted from the recorded history's structural values — a schema decode \
         per sample would hit the bus on a render path, so a protobuf or CDR leaf \
         offers no chart",
    ));

    if plottable {
        // The leaf picker. Small buttons rather than a dropdown: the list is
        // short by construction and the choice is one click either way.
        let mut picker = row![].spacing(sp.xs);
        for (path, _) in &data.leaves.leaves {
            let active = data.leaf.as_deref() == Some(path.as_str());
            picker = picker.push(kit::tab(
                path.clone(),
                active,
                msg(slot, DetailMsg::LeafSelected(path.clone())),
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
            sp,
        ));
    }

    col = col.push(spark::chart(
        "rate",
        &data.rate,
        SeriesTone::Rate,
        None,
        &data.caches.rate,
        sp,
    ));
    Some(col.into())
}

pub(crate) fn facts_section(f: &KeyFacts, sp: Spacing) -> Element<'_, Message> {
    let mut col = Column::new().spacing(sp.xs);
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

fn hex_pane<'a>(bytes: &[u8], sp: Spacing) -> Element<'a, Message> {
    let shown = &bytes[..bytes.len().min(HEX_VIEW_BYTES)];
    let out = hex_dump(shown);
    let mut col = Column::new().spacing(sp.xs);
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

/// How many `Invalid` violation sentences render before the list stops and
/// says so — a schema with a hundred failed clauses is one bad payload, not a
/// hundred rows of pane.
const VIOLATION_ROWS: usize = 8;

/// The decoded side, tagged with how it was produced — and with the payload's
/// conformance verdict (#164): three states, never a boolean, and each
/// not-validated *reason* spelled (`NoRegistry`'s "nobody looked" never wears
/// `NoSchema`'s "asked, and the type has none" — #246).
fn decoded_pane<'a>(
    decoded: Option<&'a DecodedValue>,
    payload_len: usize,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    match decoded {
        None => {
            col = col.push(kit::muted("decoding…"));
        }
        Some(value) => {
            let sample = &value.sample;
            // The document was rendered once, in the decode task (#345); all
            // this pane decides is how much of it to draw.
            let (shown, elided) = clamp_document(&value.document);
            match &sample.rendering {
                Rendering::Typed(d) => {
                    col = col.push(kit::tone_badge(
                        RegistrationTone::Registered,
                        format!(
                            "schema-decoded <{}>",
                            sample.type_name.as_deref().unwrap_or("?")
                        ),
                    ));
                    col = col.push(kit::mono(shown.to_string()));
                    for note in &d.notes {
                        col = col.push(kit::muted(format!("note: {note}")));
                    }
                }
                Rendering::Structural(_) => {
                    // The honest ladder: typed-but-undecoded vs plain structural.
                    let tag = match &sample.type_name {
                        Some(t) => format!("<{t}?> structural (schema did not decode)"),
                        None => "structural (no schema resolves — RFC 08 §7's fallback)".into(),
                    };
                    col = col.push(kit::muted(tag));
                    col = col.push(kit::mono(if shown.is_empty() {
                        format!("<{payload_len} bytes>")
                    } else {
                        shown.to_string()
                    }));
                }
            }
            if elided > 0 {
                col = col.push(kit::muted(format!("… {elided} more bytes not shown")));
            }
            // The verdict, glyph and word (#164/#193).
            col = col.push(kit::badge_verdict(
                crate::verdict::tone(&sample.verdict),
                crate::verdict::label(&sample.verdict),
            ));
            match &sample.verdict {
                zenkey::schema::validate::Verdict::Invalid(errors) => {
                    for e in errors.iter().take(VIOLATION_ROWS) {
                        col = col.push(kit::muted(format!("violation: {e}")));
                    }
                    if errors.len() > VIOLATION_ROWS {
                        col = col.push(kit::muted(format!(
                            "+{} more not shown (display bound)",
                            errors.len() - VIOLATION_ROWS
                        )));
                    }
                }
                zenkey::schema::validate::Verdict::NotValidated(reason) => {
                    // The full sentence under the compact badge, so the
                    // reason is never only a parenthetical.
                    col = col.push(kit::muted(reason.to_string()));
                }
                zenkey::schema::validate::Verdict::Valid => {}
            }
            if let Some(e) = &sample.decode_error {
                col = col.push(kit::muted(format!("decode error: {e}")));
            }
        }
    }
    iced::widget::container(col)
        .width(Length::FillPortion(1))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// #345: both sides of a value row are bounded now, and each says what it
    /// left out. The rendered side had no bound at all — a whole decoded
    /// document was re-serialized into a text widget on every redraw, next to
    /// a hex pane deliberately clamped at 1 KiB.
    #[test]
    fn a_document_within_the_bound_is_whole_and_one_past_it_says_what_it_hid() {
        let small = "{\"value\":42.0}";
        assert_eq!(clamp_document(small), (small, 0), "no note, nothing hidden");

        let big = "x".repeat(DOCUMENT_VIEW_CHARS + 137);
        let (shown, elided) = clamp_document(&big);
        assert_eq!(shown.len(), DOCUMENT_VIEW_CHARS);
        assert_eq!(elided, 137, "the count is the bound's cost, stated (O6)");
        assert_eq!(
            shown.len() + elided,
            big.len(),
            "nothing is unaccounted for"
        );
    }

    /// The clamp lands on a char boundary — a bound that can panic on a
    /// multi-byte payload is not a bound, it is a crash waiting for traffic.
    #[test]
    fn the_clamp_never_splits_a_character() {
        // A three-byte character straddling the limit whichever way it falls.
        let s = "é".repeat(DOCUMENT_VIEW_CHARS);
        let (shown, elided) = clamp_document(&s);
        assert!(shown.len() <= DOCUMENT_VIEW_CHARS);
        assert!(s.is_char_boundary(shown.len()));
        assert_eq!(shown.len() + elided, s.len());
    }

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
