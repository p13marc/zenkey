//! The selection detail pane — §6.4 item 5's inspector plus issue #66's
//! metadata view, merged: one selection, everything honestly known about it.
//!
//! Three sections, each stating its provenance:
//!
//! - **Identity** — where zk2's O2 ladder got for the key (#614, FL7): the
//!   address, interface, kind and resource it resolved to, or the rung it
//!   stopped at and why (fields absent are absent, never defaulted);
//! - **Value** — the read the key's identity asks (#614, FL8a): S4's owner
//!   GET for state, an archive's last-known behind its own button and never
//!   as current, the owner's `@adv` history for a stream declaring one, an
//!   event replay on request, a raw GET for a foreign key — and during a
//!   replay the capture's own row, the bus asked nothing. Rendered **hex
//!   beside decoded** where the read kept bytes: the decoded side is
//!   contract-decoded named fields when the lens resolves the key (#614,
//!   FL6a), and the honest ladder otherwise — tagged with *how* it was
//!   produced, because schema-decoded and sniffed must never look alike.
//! - **Series** (issue #64) — sparklines for a numeric leaf and for the
//!   observed message rate, drawn from the same history the timeline reads,
//!   with gaps drawn as gaps. Absent entirely when there is nothing numeric
//!   to plot: an explorer that renders an empty chart for a string payload
//!   has invented an error state out of an ordinary fact. A `telemetry.v1`
//!   metric (#819, TD) plots what the profile reads instead: its value in
//!   its unit (§2.5), a counter's §2.11 rate with a gap at every reset, and
//!   each decrease within one epoch as the finding it is (§2.10).

use iced::widget::{Column, row};
use iced::{Element, Length};

use zenkey_fleet::report::{Provenance, QosAxes, Stamp, StateValue};

use crate::keyfacts::{KeyGroup, Resolution, Rung, Unresolved};
use crate::message::{Message, PaneMsg, SlotId};
use crate::metric::{MetricRead, Unit};
use crate::series::{NumericLeaves, Series};
use crate::value::{
    DecodedValue, NotRead, ReadKind, ReadPlan, Replies, Reply, StateAnswer, ValueRead,
};
use crate::view::kit;
use crate::view::spark;
use crate::view::theme::{IdentityTone, SeriesTone};
use crate::view::tokens::{Spacing, face};

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
/// scroll (tooling guide O6 — the bound is stated where it bites).
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
/// all — the same limit the echo line stops at — and what
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
    let rendered = zenkey_fleet::structural(bytes);
    let (shown, elided) = clamp_document(&rendered);
    col = col.push(kit::mono(shown.to_string()));
    if elided > 0 {
        col = col.push(kit::muted(format!("… {elided} more bytes not shown")));
    }
    // Set into the panel like its hex beside it (#563): the decoded value's
    // pane always was, and the attachment's was the one left on the panel.
    kit::inset(col).width(Length::FillPortion(1)).into()
}

/// One read, named for the status strip (#614, FL8a): which question was
/// asked, and the answer's shape in a few words — "owner GET (S4)",
/// "silent", "raw GET". Never the enum's `Debug` name (#563).
pub(crate) fn read_summary(read: &ValueRead) -> String {
    match read {
        ValueRead::Owner(answer) => match answer.row().map(|r| &r.value) {
            None => format!(
                "silent — owner GET (S4) unanswered in {}s (not a verdict)",
                answer.report.timeout_s
            ),
            Some(StateValue::Deleted) => "owner GET (S4): deleted".to_string(),
            Some(StateValue::Value { payload }) => {
                format!("owner GET (S4): {}", kit::human_bytes(payload.size as u64))
            }
        },
        ValueRead::Replies(r) => match r.replies.first() {
            None => format!("{}: no reply (not a verdict)", r.kind.word()),
            Some(reply) => format!(
                "{}: {}",
                r.kind.word(),
                kit::human_bytes(reply.payload.len() as u64)
            ),
        },
        ValueRead::Recorded { reply, t_us, .. } => format!(
            "the recorded row at {:.3}s, {} — nothing asked of the bus",
            *t_us as f64 / 1e6,
            kit::human_bytes(reply.payload.len() as u64)
        ),
    }
}

/// An archive's last-known answer, named for the status strip and the pane
/// alike (#614, FL8a): it says whose, and that it is not current (S6).
pub(crate) fn last_known_label(archive: &zenkey_model::grammar::Addr) -> String {
    format!("last-known from {archive} — not current")
}

/// Messages the pane emits.
#[derive(Debug, Clone)]
pub enum DetailMsg {
    /// A numeric leaf was picked for the value sparkline.
    LeafSelected(String),
    /// Ask this archive for the key's last-known state (S5) — the explicit
    /// act a last-known read needs (#614, FL8a).
    LastKnown(zenkey_model::grammar::Addr),
    /// An archive's last-known answer landed: (key, archive, the answer).
    LastKnownRead(
        String,
        zenkey_model::grammar::Addr,
        Result<std::sync::Arc<StateAnswer>, crate::services::ServiceError>,
    ),
    /// Replay the event member's occurrences within its retention (§2.6):
    /// an event is read on request, never on selection.
    ReplayEvents,
}

/// What the pane knows about the read for the key it is showing (#181).
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
    /// A read is in flight for this key, and which question it asked
    /// (#614, FL8a): "asking", never "not asked".
    Asking(ReadKind),
    /// A fetch landed, but for a different subject. The answer is real; it is
    /// just not about what is on screen.
    Superseded,
    /// The answer to the question this pane is showing.
    Landed(&'a Result<std::sync::Arc<ValueRead>, crate::services::ServiceError>),
}

pub struct DetailData<'a> {
    /// The subject slot this section renders (#257) — every message it
    /// emits carries it home.
    pub slot: SlotId,
    pub key: &'a str,
    /// The key's zk2 identity, as the identity cache holds it (#614, FL7);
    /// `None` until the key is observed live or selected.
    pub identity: Option<&'a Resolution>,
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
    /// Whether the panes show a replay (#614, FL8a): the bus is asked
    /// nothing, and the value is the capture's row.
    pub replaying: bool,
    /// The `archive.v1` providers presence shows (#614, FL8a): who may be
    /// asked for last-known state. `None` = presence not read — no archive
    /// is known, which is not "none exists".
    pub archives: Option<Vec<zenkey_model::grammar::Addr>>,
    /// An archive's last-known answer for this slot, when one was asked.
    pub last_known: Option<&'a LastKnown>,
    /// The dock's resolved spacing grid (#192).
    pub sp: Spacing,
}

/// An archive's last-known state of one key (S5), asked by the Detail
/// section's button (#614, FL8a): which archive, and its answer — `None`
/// while the read is in flight.
#[derive(Debug, Clone)]
pub struct LastKnown {
    pub key: String,
    pub archive: zenkey_model::grammar::Addr,
    pub answer: Option<Result<std::sync::Arc<StateAnswer>, crate::services::ServiceError>>,
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

/// The wire's QoS axes as one stable lowercase token.
///
/// The engine's spelling, not a copy of it: this is the token
/// `SampleRow.qos_axes` carries and `parse_row` reads back, so the two
/// frontends cannot describe one sample's QoS differently — which they
/// previously guaranteed with a comment (#353).
pub fn qos_token(entry: &crate::history::HistoryEntry) -> String {
    zenkey_fleet::report::qos_axes_token(
        entry.priority,
        entry.congestion_control,
        entry.reliability,
        entry.express,
    )
}

/// The QoS a key's resource declares (§2.4), read off its identity (#614,
/// FL8a) — `Lens::declared_qos`'s answer from the resolution the identity
/// cache already holds, so the pane resolves nothing on a frame. `Err` is
/// why nothing declares one, in words.
pub fn declared_qos(identity: Option<&Resolution>) -> Result<QosAxes, String> {
    let Some(r) = identity else {
        return Err(
            "no identity yet — the declared QoS is the contract's, read once \
                    the key resolves"
                .to_string(),
        );
    };
    match (&r.resolved, &r.identity.unresolved) {
        (Some(resolved), _) => zenkey_fleet::declared_qos(resolved.resource()).ok_or_else(|| {
            "an operation declares no QoS — its replies ride the caller's".to_string()
        }),
        // A foreign key's rung is a fact about the key (O1), said once in
        // the identity above; here it is only why nothing declares QoS.
        (None, Some(Unresolved::NotZk2 { .. } | Unresolved::NotInNamespace { .. })) => {
            Err("a foreign key — no contract declares its QoS".to_string())
        }
        (None, Some(why)) => Err(format!("nothing declares it — {}", why.words())),
        (None, None) => Err("a control key — no contract types it, so nothing declares \
                             its QoS"
            .to_string()),
    }
}

/// The contract's spelling of three axes: `data/block`, `+express` when set.
pub fn axes_token(axes: &QosAxes) -> String {
    format!(
        "{}/{}{}",
        axes.priority,
        axes.congestion,
        if axes.express { "+express" } else { "" }
    )
}

/// The axes an observed sample rode, in the contract's spelling (§2.4) —
/// what [`zenkey_fleet::qos_mismatch`] compares against the declaration.
pub fn observed_axes(entry: &crate::history::HistoryEntry) -> QosAxes {
    zenkey_fleet::observed_qos(entry.priority, entry.congestion_control, entry.express)
}

/// The declared-vs-observed QoS lines (#120; zk2's since #614, FL8a), when
/// there is anything to say: the resource's declared axes (§2.4) against
/// the axes the newest observed sample rode, compared by
/// [`zenkey_fleet::qos_mismatch`] and named axis by axis.
fn qos_section<'a>(data: &DetailData<'a>) -> Option<Element<'a, Message>> {
    let observed = data.observed;
    if data.identity.is_none() && observed.is_none() {
        return None;
    }
    let declared = declared_qos(data.identity);
    // Fields since #563: the claim, the fact and where the fact came from,
    // each under its eyebrow — "—" where there is none, said beneath.
    let sp = data.sp;
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::eyebrow("QOS — DECLARED VS OBSERVED (§2.4)"));
    let mono = |s: String| kit::caption(s).font(face::MONO);
    let mut fields = vec![kit::field(
        "DECLARED",
        mono(declared.as_ref().map_or("—".to_string(), axes_token)),
    )];
    match observed {
        // A replayed row that recorded no axes (#614, FL10b): its view's
        // axes are defaults, never shown as observed.
        Some(entry) if !entry.qos_recorded => {
            fields.push(kit::field(
                "OBSERVED",
                mono(crate::replay::QOS_NOT_RECORDED.to_string()),
            ));
        }
        Some(entry) => {
            fields.push(kit::field("OBSERVED", mono(qos_token(entry))));
            if let Some(src) = &entry.source {
                fields.push(kit::field(
                    "SOURCE",
                    mono(format!("{}:{}#{}", src.zid, src.eid, src.sn)),
                ));
            }
        }
        None => fields.push(kit::field("OBSERVED", mono("—".to_string()))),
    }
    col = col.push(kit::fields(fields));
    if let Err(why) = &declared {
        col = col.push(kit::muted(why.clone()));
    }
    match observed {
        // Nothing observed to judge: said, and no verdict drawn.
        Some(entry) if !entry.qos_recorded => {
            col = col.push(kit::muted(
                "this capture's row carries no wire axes — a version-1 or -2 .zrec named a \
                 v1 profile, which zk2 does not read",
            ));
        }
        // The verdict, a badge whose glyph comes from its tone (#193): three
        // axes compared, each that differs named with both sides.
        Some(entry) => {
            if let Ok(declared) = &declared {
                match zenkey_fleet::qos_mismatch(declared, &observed_axes(entry)) {
                    None => {
                        col = col.push(kit::badge_verdict(
                            crate::view::theme::VerdictTone::Valid,
                            "observed axes match the declared QoS",
                        ));
                    }
                    Some(mismatch) => {
                        col = col.push(kit::badge_severity(
                            crate::view::theme::SeverityTone::Warning,
                            "observed axes differ from the declared QoS",
                        ));
                        col = col.push(kit::muted(
                            "the wire is the fact, the contract the claim (§2.4)",
                        ));
                        for axis in &mismatch.differs {
                            let (d, o) = match axis.as_str() {
                                "priority" => (
                                    mismatch.declared.priority.clone(),
                                    mismatch.observed.priority.clone(),
                                ),
                                "congestion" => (
                                    mismatch.declared.congestion.clone(),
                                    mismatch.observed.congestion.clone(),
                                ),
                                _ => (
                                    mismatch.declared.express.to_string(),
                                    mismatch.observed.express.to_string(),
                                ),
                            };
                            col =
                                col.push(kit::muted(format!("{axis}: declared {d}, observed {o}")));
                        }
                    }
                }
                col = col.push(kit::muted(
                    "reliability is the link's, as received — shown, not compared",
                ));
            }
        }
        None => {
            col = col.push(kit::muted(
                "nothing recorded yet — the axes appear once the key is observed (O4)",
            ));
        }
    }
    Some(col.into())
}

/// Everything the series section plots, computed by the app from the history
/// ring the timeline reads.
pub struct SeriesData {
    /// The numeric leaves the newest payload offers — none for a
    /// `telemetry.v1` metric, which plots the profile's value instead.
    pub leaves: NumericLeaves,
    /// Which one is plotted.
    pub leaf: Option<String>,
    /// That leaf's value over the retained history.
    pub value: Series,
    /// The observed message rate — samples per second, the engine's EWMA —
    /// one point per stats tick. Never a counter's rate (§2.11).
    pub rate: Series,
    /// The plotted value's unit, or why it has none (#819, TD): a metric's
    /// declared unit, else its Point's, else unknown — never dimensionless
    /// (`telemetry.v1` §2.5); and for any other key, why nothing declares
    /// one.
    pub unit: Unit,
    /// The key's `telemetry.v1` metric member, when it resolves to a metric
    /// resource (#819, TD): its kind, its unit, its plotted value, a
    /// counter's rate, and every pair's continuity — each the fleet's
    /// reading (`crate::metric`).
    pub metric: Option<std::sync::Arc<MetricRead>>,
    /// The key's `timing.v1` stream (#908, MC): its declared period,
    /// deadline and lifespan, and the window's own view of its deadline and
    /// period over the recorded history (`crate::timing`).
    pub timing: Option<std::sync::Arc<crate::timing::TimingRead>>,
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
    /// A counter's §2.11 rate (#819, TD).
    pub counter_rate: iced::widget::canvas::Cache,
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
    col = col.push(kit::emphasis(data.key).font(face::MONO));

    // — Identity: zk2's ladder, worded per rung (#614, FL7).
    match data.identity {
        None => {
            col = col.push(kit::muted(
                "no identity yet — a key is resolved through the lens once it is \
                 observed live or selected",
            ));
        }
        Some(r) => {
            col = col.push(identity_section(
                r,
                data.series.and_then(|s| s.metric.as_deref()),
                data.series.and_then(|s| s.timing.as_deref()),
                sp,
            ));
        }
    }
    if let Some(qos) = qos_section(&data) {
        col = col.push(qos);
    }
    if let Some((lat, unstamped)) = &data.latency {
        // One row per population. Folding a publisher-stamped sample and a
        // router-stamped one into a single median produces a number that
        // describes neither (#213).
        col = col.push(kit::eyebrow("OBSERVED SKEWED LATENCY"));
        for (label, s) in lat.populations() {
            col = col.push(
                row![
                    kit::caption(label.to_string()).font(face::MONO),
                    kit::data_chip(format!("med {}", human_us(s.median_us))),
                    kit::data_chip(format!("p95 {}", human_us(s.p95_us))),
                    kit::data_chip(format!("min {}", human_us(s.min_us))),
                    kit::data_chip(format!("max {}", human_us(s.max_us))),
                    kit::muted(kit::plural(s.samples, "sample")),
                ]
                .spacing(sp.xs)
                .align_y(iced::Alignment::Center),
            );
        }
        col = col.push(kit::data_chip(format!("{unstamped} unstamped")));
        // The caveat comes from the engine, so this pane and `zenctl rate
        // --latency` cannot describe the same measurement differently.
        col = col.push(kit::muted(lat.caveat()));
    }

    // — Value: the read the key's identity asks (#614, FL8a), hex beside
    // decoded where the read kept bytes.
    col = col.push(value_section(&data));

    // — Series: sparklines over the recorded history (issue #64).
    if let Some(series) = data.series
        && let Some(section) = series_section(series, data.slot, sp)
    {
        col = col.push(section);
    }

    // — Timing: a `timing.v1` stream's deadline and period over the
    // recorded history, the window's own view (#908, MC).
    if let Some(t) = data.series.and_then(|s| s.timing.as_deref()) {
        col = col.push(timing_section(t, sp));
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
/// a problem (#64's second acceptance line). A `telemetry.v1` metric always
/// has a section (#819, TD): what the profile read of it is said even when
/// nothing plots.
fn series_section<'a>(
    data: &'a SeriesData,
    slot: SlotId,
    sp: Spacing,
) -> Option<Element<'a, Message>> {
    if let Some(m) = data.metric.as_deref().filter(|m| m.declared()) {
        return Some(metric_section(m, data, sp));
    }
    let plottable = !data.leaves.leaves.is_empty();
    if !plottable && !data.rate.has_data() && data.metric.is_none() {
        return None;
    }
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::section_header("Series", None));
    col = col.push(kit::muted(
        "plotted from the recorded history's structural values — a schema decode \
         per sample would hit the bus on a render path, so a protobuf or CDR leaf \
         offers no chart",
    ));
    // A metric resource whose declaration is invalid (§2.2): its members
    // are not read, so its leaves plot structurally, with the reasons said.
    if let Some(why) = data.metric.as_deref().and_then(MetricRead::invalid) {
        col = col.push(kit::muted(format!(
            "telemetry.v1: the declaration is invalid — {why}: the member's kind, unit, \
             conformance and continuity are unobservable (§2.2)"
        )));
    }

    if plottable {
        // The leaf picker. Small buttons rather than a dropdown: the list is
        // short by construction and the choice is one click either way.
        let picker = kit::segmented(
            data.leaves
                .leaves
                .iter()
                .map(|(path, _)| kit::Segment {
                    value: path.clone(),
                    label: path.clone(),
                    icon: None,
                    count: None,
                    tip: None,
                })
                .collect(),
            data.leaf.clone(),
            move |path| msg(slot, DetailMsg::LeafSelected(path)),
        );
        col = col.push(iced::widget::scrollable(picker).width(Length::Fill));
        if data.leaves.truncated > 0 {
            col = col.push(kit::muted(format!(
                "… {} not offered (the leaf list is bounded)",
                kit::plural(data.leaves.truncated, "further numeric leaf"),
            )));
        }
        let label = data.leaf.as_deref().unwrap_or("value");
        if let Some(why) = data.unit.why_none() {
            col = col.push(kit::muted(why));
        }
        col = col.push(spark::chart(
            label,
            &data.value,
            SeriesTone::Value,
            data.unit.unit(),
            &data.caches.value,
            sp,
        ));
    }

    col = col.push(message_rate(data, sp));
    Some(col.into())
}

/// The engine's message rate, named as such: samples per second on the key,
/// the observer's measurement — never a counter's rate (§2.11).
fn message_rate<'a>(data: &'a SeriesData, sp: Spacing) -> Element<'a, Message> {
    spark::chart(
        "message rate (samples/s)",
        &data.rate,
        SeriesTone::Rate,
        None,
        &data.caches.rate,
        sp,
    )
}

/// A `telemetry.v1` metric's series (#819, TD): what the profile read of the
/// recorded history — the plotted value in the member's unit, a counter's
/// §2.11 rate, and every pair's continuity — each the fleet's reading
/// (`crate::metric`), and beside them the engine's message rate.
fn metric_section<'a>(
    m: &'a MetricRead,
    data: &'a SeriesData,
    sp: Spacing,
) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::section_header("Series", None));
    col = col.push(kit::muted(format!(
        "read by telemetry.v1 over the recorded history: {} (§2.5, §2.6)",
        m.plotted_from()
    )));
    if m.samples > 0 {
        let mut line = format!(
            "{} of {} conform (§2.6)",
            m.conforming,
            kit::plural(m.samples as usize, "sample")
        );
        if m.does_not > 0 {
            line.push_str(&format!(" · {} do not", m.does_not));
        }
        if m.unobservable > 0 {
            line.push_str(&format!(" · {} undecodable", m.unobservable));
        }
        col = col.push(kit::muted(line));
    }
    let reasons = m.reasons();
    if !reasons.is_empty() {
        col = col.push(kit::badge_verdict(
            crate::view::theme::VerdictTone::Invalid,
            if m.newest_conforms {
                "an earlier sample did not conform — its reasons, the latest such"
            } else {
                "the newest sample does not conform — not plotted, and no value of any pair"
            },
        ));
        for r in reasons {
            col = col.push(kit::muted(r));
        }
    }
    match m.plotted_label() {
        Some(label) => {
            if let Some(why) = data.unit.why_none() {
                col = col.push(kit::muted(why));
            }
            col = col.push(spark::chart(
                label,
                &m.value,
                SeriesTone::Value,
                data.unit.unit(),
                &data.caches.value,
                sp,
            ));
        }
        None => {
            col = col.push(kit::muted(match m.kind {
                Some((k, _)) => {
                    format!("a {k} metric is not plotted — its values are the timeline's, below")
                }
                None => "nothing plotted — no conforming sample has said the member's kind \
                         (§2.5)"
                    .to_string(),
            }));
        }
    }
    if let Some(rate) = &m.rate {
        col = col.push(spark::chart(
            "rate (§2.11)",
            rate,
            SeriesTone::Value,
            m.rate_unit.as_deref(),
            &data.caches.counter_rate,
            sp,
        ));
    }
    let cumulative = m.kind.is_some_and(|(k, _)| k.is_cumulative());
    // The finding first, under the charts it explains; the words on how
    // the rate was had and how each pair answered after it.
    if cumulative {
        col = col.push(decreases(m, sp));
    }
    if m.rate.is_some() {
        col = col.push(rate_notes(m, data, sp));
    }
    if cumulative {
        col = col.push(pairs(m, sp));
    }
    col = col.push(message_rate(data, sp));
    col.into()
}

/// How a counter's rate was had (§2.11): its time base, its unit when the
/// counter's is unknown, and the pairs that have none.
fn rate_notes<'a>(m: &'a MetricRead, data: &'a SeriesData, sp: Spacing) -> Element<'a, Message> {
    let mut col = Column::new().spacing(sp.xs);
    if let Some(base) = m.rate_base {
        col = col.push(kit::muted(match base {
            "timestamp" => "timed by the Points' timestamps, the measurement times (§2.11)",
            "stamp" => "timed by the owner's stamps (§2.11)",
            _ => {
                "timed by arrival on this reader's clock — no Point timestamp or owner \
                 stamp rides these samples (§2.11)"
            }
        }));
    }
    if matches!(data.unit, Unit::Unknown) {
        col = col.push(kit::muted(
            "the counter's unit is unknown, so its rate's is too (§2.11)",
        ));
    }
    if m.no_rate.reset + m.no_rate.decreased > 0 {
        col = col.push(kit::muted(format!(
            "no rate across a reset or a decrease: {} drawn as gaps, never \
             restarted from zero (§2.11)",
            kit::plural((m.no_rate.reset + m.no_rate.decreased) as usize, "pair"),
        )));
    }
    if m.no_rate.elapsed > 0 {
        col = col.push(kit::muted(format!(
            "{} with no elapsed time between its samples, so no rate (§2.11)",
            kit::plural(m.no_rate.elapsed as usize, "pair"),
        )));
    }
    col.into()
}

/// Each decrease within an established epoch, as the finding it is
/// (§2.10), with how its epoch was had (§2.9's residual risk).
fn decreases<'a>(m: &'a MetricRead, sp: Spacing) -> Element<'a, Message> {
    use crate::view::theme::SeverityTone;
    use zenkey_model::telemetry::Via;
    let t = &m.tally;
    let mut col = Column::new().spacing(sp.xs);
    for d in &m.decreases {
        col = col.push(kit::finding(
            SeverityTone::Error,
            "decreased",
            format!(
                "{} → {}",
                crate::metric::measure_words(&d.earlier),
                crate::metric::measure_words(&d.later)
            ),
            match d.via {
                Via::InBand => {
                    "a decrease within one epoch, established in-band: both samples carry \
                     the same epoch, so the owner went backwards (§2.7) — the finding, \
                     never a reset"
                }
                Via::Presence => {
                    "a decrease within one epoch, established by presence — §2.9's residual \
                     risk: a sample in transit across a re-mint can read as the new \
                     instance's"
                }
            },
        ));
    }
    if t.decreased > m.decreases.len() as u64 {
        col = col.push(kit::muted(format!(
            "+{} earlier not shown",
            kit::plural(
                (t.decreased - m.decreases.len() as u64) as usize,
                "decrease"
            )
        )));
    }
    col.into()
}

/// A counter's or a histogram's pairs (§2.10): how each answered and how
/// its epoch was had — and, where no epoch was ever established, that no
/// decrease could have been found (§5, "over many pairs").
fn pairs<'a>(m: &'a MetricRead, sp: Spacing) -> Element<'a, Message> {
    let t = &m.tally;
    let mut col = Column::new().spacing(sp.xs);
    let mut line = format!(
        "pairs (§2.10): {} same epoch · {} reset · {} decreased",
        t.same_epoch, t.reset, t.decreased
    );
    if t.unobservable > 0 {
        line.push_str(&format!(" · {} incomparable", t.unobservable));
    }
    line.push_str(&format!(
        " — epoch in-band in {}, assumed in {}; presence is not read for an epoch here \
         (its tokens and these samples arrive on two monitors, §2.9)",
        t.in_band, t.assumed
    ));
    col = col.push(kit::muted(line));
    if t.asked() > 0 && t.in_band + t.by_presence == 0 {
        col = col.push(kit::muted(
            "no decrease could be found: every pair's epoch was assumed — no in-band \
             epoch rides these samples — so a decrease reads as a reset, never a finding \
             (§2.10, §5)",
        ));
    }
    let r = &m.resets;
    if r.total() > 0 {
        col = col.push(kit::muted(format!(
            "{}: {} in-band (another epoch), {} assumed (a decrease no epoch places)",
            kit::plural(r.total() as usize, "reset"),
            r.in_band + r.by_presence,
            r.assumed,
        )));
    }
    col.into()
}

/// The value section (#614, FL8a): what was asked of the key's value and
/// what came back — or why nothing was asked — then, for a state key, the
/// archives a last-known read may ask, behind their own buttons.
fn value_section<'a>(data: &DetailData<'a>) -> Column<'a, Message> {
    let sp = data.sp;
    let mut col = Column::new().spacing(sp.sm);
    match data.fetched {
        Fetched::NotAsked => col = col.push(not_asked(data)),
        Fetched::Asking(kind) => {
            col = col.push(kit::muted(format!("asking — {} …", kind.word())));
        }
        Fetched::Superseded => {
            col = col.push(kit::muted(
                "superseded — the last fetch answered a different subject, so \
                 nothing here describes this key. Re-select it to ask again.",
            ));
        }
        Fetched::Landed(Err(e)) => {
            col = col.push(kit::error(format!("read failed: {e}")));
        }
        Fetched::Landed(Ok(read)) => {
            col = col.push(match read.as_ref() {
                ValueRead::Owner(answer) => owner_value(answer, data),
                ValueRead::Replies(r) => replies_value(r, data),
                ValueRead::Recorded { reply, t_us, .. } => recorded_value(reply, *t_us, data),
            });
        }
    }
    // Last-known is a state key's, an archive's, and asked only by the
    // button (S5); a replay asks the bus nothing at all.
    if !data.replaying && crate::value::plan(data.key, data.identity) == ReadPlan::Owner {
        col = col.push(last_known_section(data));
    }
    col
}

/// Why nothing was read for the key, in the plan's own terms (#614, FL8a) —
/// and for an event, the button that replays it.
fn not_asked<'a>(data: &DetailData<'a>) -> Element<'a, Message> {
    if data.replaying {
        return kit::muted(
            "nothing recorded for this key up to the playhead — a replay asks the bus \
             nothing, so this is the capture's silence, not the bus's",
        );
    }
    let line = match crate::value::plan(data.key, data.identity) {
        ReadPlan::Owner => {
            "not asked — the owner GET (S4) goes out on the session in the namespace, \
             and none is open"
        }
        ReadPlan::History | ReadPlan::Raw => {
            "not asked — the read goes out on the session in no namespace, and none is \
             open"
        }
        ReadPlan::NotRead(NotRead::Symbolic) => {
            "a templated skeleton path names no key — there is no value to read; \
             select a concrete key"
        }
        ReadPlan::NotRead(NotRead::Operation) => {
            "an operation's key holds no value — a GET on it is a call (spec §5), so \
             selecting it asks nothing"
        }
        ReadPlan::NotRead(NotRead::Stream) => {
            "a stream that declares no history keeps no value to read (§2.5) — watch \
             it to see its samples"
        }
        ReadPlan::NotRead(NotRead::Event { retention_s }) => {
            let ask = kit::secondary(kit::caption(match retention_s {
                Some(s) => format!("replay occurrences (last {})", human_span(s)),
                None => "replay occurrences".to_string(),
            }))
            .padding(data.sp.xs);
            let ask = match retention_s {
                Some(_) => ask.on_press(msg(data.slot, DetailMsg::ReplayEvents)),
                None => ask,
            };
            return Column::new()
                .spacing(data.sp.xs)
                .push(kit::muted(
                    "an event is replayed on request, never on selection — a GET over \
                     its member, bounded by the retention (§2.6)",
                ))
                .push(ask)
                .into();
        }
        ReadPlan::NotRead(NotRead::Unresolved) if data.identity.is_none() => {
            "no value read — selecting a concrete key reads it once, as its identity \
             says (the owner's GET for state, a raw GET for a foreign key), nothing \
             ambient"
        }
        ReadPlan::NotRead(NotRead::Unresolved) => {
            "not read — the owner's GET needs the key's revision, and its identity \
             stops short of it (above); why? explains the rung"
        }
    };
    kit::muted(line)
}

/// `seconds` in the largest unit that divides it: `7d`, `90m`, `45s`.
fn human_span(seconds: u64) -> String {
    for (unit, n) in [("w", 604_800), ("d", 86_400), ("h", 3_600), ("m", 60)] {
        if seconds >= n && seconds.is_multiple_of(n) {
            return format!("{}{unit}", seconds / n);
        }
    }
    format!("{seconds}s")
}

/// A stamp's chip: whose clock issued it (O7), in words.
fn stamp_chip<'a>(stamp: Option<&Stamp>, provenance: Option<Provenance>) -> Element<'a, Message> {
    kit::data_chip(match (stamp, provenance) {
        (None, _) => "unstamped",
        (Some(_), Some(Provenance::Owner)) => "stamped by the owner's clock (S1)",
        (Some(_), Some(Provenance::Other)) => "stamped by another clock (O7)",
        (Some(_), _) => "stamp unattributable (O7)",
    })
}

/// The stamp itself, beside its chip: the time and the clock's id.
fn stamp_line<'a>(stamp: Option<&Stamp>) -> Option<Element<'a, Message>> {
    stamp.map(|s| kit::mono(format!("stamp {} · clock {}", s.time, s.clock)))
}

/// S4's answer (#614, FL8a): the owner's value decoded through its
/// contract, its bytes in hex beside it (#765), its deletion, or its
/// silence — never "no value" (S6).
fn owner_value<'a>(answer: &'a StateAnswer, data: &DetailData<'a>) -> Element<'a, Message> {
    let sp = data.sp;
    let mut col = Column::new().spacing(sp.xs);
    let Some(row) = answer.row() else {
        return kit::muted(format!(
            "silent — the owner did not answer S4's GET within {}s: not a verdict \
             about the key (S6). An archive may still hold its last-known value.",
            answer.report.timeout_s
        ));
    };
    let size = match &row.value {
        StateValue::Value { payload } => Some(payload.size),
        StateValue::Deleted => None,
    };
    let mut chips = row![kit::eyebrow("VALUE")]
        .spacing(sp.xs)
        .align_y(iced::Alignment::Center);
    if let Some(size) = size {
        chips = chips.push(kit::data_chip(kit::human_bytes(size as u64)));
    }
    chips = chips
        .push(kit::data_chip(format!("via {}", ReadKind::Owner.word())))
        .push(stamp_chip(row.timestamp.as_ref(), answer.provenance));
    col = col.push(chips);
    if let Some(line) = stamp_line(row.timestamp.as_ref()) {
        col = col.push(line);
    }
    match size {
        None => {
            col = col.push(kit::callout(
                crate::view::theme::Tone::Info,
                kit::caption(
                    "deleted — the owner answered with a deletion within its tombstone \
                     window (S3): retirement, not an empty value",
                ),
            ));
        }
        Some(size) => {
            // The owner's reply as it arrived, kept on the row (#765): its
            // hex beside the decode, as a GET's reply is shown.
            let left = match &row.bytes {
                Some(bytes) => hex_pane(&bytes.to_bytes(), sp),
                None => no_bytes_pane(
                    "no hex — this read handed back its rendering with no bytes beside it",
                    sp,
                ),
            };
            col = col
                .push(row![left, decoded_pane(answer.decoded.as_deref(), size, sp)].spacing(sp.md));
        }
    }
    if answer.report.rows.len() > 1 {
        col = col.push(kit::muted(format!(
            "+{} more rows — showing the first; a resolved key names one member",
            answer.report.rows.len() - 1
        )));
    }
    col.into()
}

/// The left half of a value row when the read kept no bytes: said, never a
/// blank well.
fn no_bytes_pane<'a>(why: &'static str, sp: Spacing) -> Element<'a, Message> {
    kit::inset(Column::new().spacing(sp.xs).push(kit::muted(why)))
        .width(Length::FillPortion(1))
        .into()
}

/// What a history, an event replay or a raw GET answered (#614, FL8a): the
/// newest reply, hex beside decoded, and every reply that was not shown or
/// not a value counted.
fn replies_value<'a>(r: &'a Replies, data: &DetailData<'a>) -> Element<'a, Message> {
    let sp = data.sp;
    let mut col = Column::new().spacing(sp.xs);
    match r.replies.first() {
        None => {
            col = col.push(kit::muted(match r.kind {
                ReadKind::History => {
                    "the owner's @adv cache answered nothing — no advanced publisher is \
                     present, or its history is empty: silence, not absence (observer §2)"
                }
                ReadKind::Events => {
                    "no occurrence answered within the retention — a union storage keeps \
                     events (§2.6), and silence is not absence (observer §2)"
                }
                _ => "raw GET: no reply — a non-verdict, not proof of absence (observer §2)",
            }));
        }
        Some(reply) => {
            col = col.push(reply_chips(reply, format!("via {}", r.kind.word()), sp));
            if r.replies.len() > 1 {
                col = col.push(kit::muted(format!(
                    "{} replies — showing the newest by stamp",
                    r.replies.len()
                )));
            }
            col = col.push(reply_body(reply, data));
        }
    }
    if let Some(retention) = r.retention {
        col = col.push(kit::muted(format!(
            "bounded by the event's retention ({}) — {} filtered here as older (§2.6)",
            human_span(retention.seconds),
            kit::plural(retention.older, "occurrence"),
        )));
    }
    if r.errors > 0 {
        col = col.push(kit::muted(format!(
            "{} — a refusal is not a value, and not silence",
            kit::plural(r.errors, "error reply"),
        )));
    }
    if r.elided > 0 {
        col = col.push(kit::muted(format!(
            "{} past the reply bound, not kept (O6)",
            kit::plural(r.elided as usize, "reply"),
        )));
    }
    col = col.push(kit::mono(format!("asked {}", r.selector)));
    col.into()
}

/// A replay's recorded row (#614, FL8a): the capture's evidence at the
/// playhead, checked through the replay's own lens; the bus asked nothing.
fn recorded_value<'a>(reply: &'a Reply, t_us: u64, data: &DetailData<'a>) -> Element<'a, Message> {
    let sp = data.sp;
    Column::new()
        .spacing(sp.xs)
        .push(reply_chips(
            reply,
            format!("recorded at {:.3}s (capture clock)", t_us as f64 / 1e6),
            sp,
        ))
        .push(reply_body(reply, data))
        .push(kit::muted(
            "a replay asks the bus nothing: this is the capture's latest row for the key \
             at the playhead, checked through the replay's offline lens",
        ))
        .into()
}

/// A reply's facts as chips (#563): its size — or, for a tombstone, that it
/// is one, never "0 B" (#765) — where it came from, its encoding and whose
/// clock stamped it.
fn reply_chips<'a>(reply: &Reply, source: String, sp: Spacing) -> Element<'a, Message> {
    row![
        kit::eyebrow("VALUE"),
        if reply.delete {
            kit::retired_chip("DELETE (tombstone)")
        } else {
            kit::data_chip(kit::human_bytes(reply.payload.len() as u64))
        },
        kit::data_chip(source),
        kit::data_chip(if reply.encoding.is_empty() {
            "encoding unset".to_string()
        } else {
            reply.encoding.clone()
        }),
        stamp_chip(reply.stamp.as_ref(), reply.provenance),
    ]
    .spacing(sp.xs)
    .align_y(iced::Alignment::Center)
    .into()
}

/// A reply's bytes: a tombstone said, a value hex beside decoded, and its
/// attachment beside its own hex (#117).
fn reply_body<'a>(reply: &'a Reply, data: &DetailData<'a>) -> Element<'a, Message> {
    let sp = data.sp;
    if reply.delete {
        return kit::callout(
            crate::view::theme::Tone::Info,
            kit::caption("a tombstone — retirement, not an empty value (core §4.2)"),
        );
    }
    let mut col = Column::new().spacing(sp.xs);
    if let Some(line) = stamp_line(reply.stamp.as_ref()) {
        col = col.push(line);
    }
    // Borrow through the Cow: `to_bytes()` is already free for a
    // contiguous payload, and `.to_vec()` on top of it was the double copy
    // the redesign flagged (`docs/zero-copy.md`).
    let bytes = reply.payload.to_bytes();
    let len = bytes.len();
    col = col.push(row![hex_pane(&bytes, sp), decoded_pane(data.decoded, len, sp)].spacing(sp.md));
    // The attachment, when the value carried one (#117): rendered
    // structurally beside its hex — no contract types it here, so
    // structural is where the rendering stops.
    if let Some(att) = &reply.attachment {
        let abytes = att.to_bytes();
        col = col.push(kit::muted(format!(
            "attachment: {} bytes — rendered structurally",
            abytes.len()
        )));
        col = col.push(row![hex_pane(&abytes, sp), attachment_pane(&abytes, sp)].spacing(sp.md));
    }
    col.into()
}

/// The archives a last-known read may ask, and the answer of the one asked
/// (#614, FL8a; §4.4): an archive's, behind an explicit button, and never
/// worded as current (S5, S6).
fn last_known_section<'a>(data: &DetailData<'a>) -> Element<'a, Message> {
    let sp = data.sp;
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::eyebrow(
        "LAST-KNOWN (S5) — AN ARCHIVE'S, NEVER CURRENT",
    ));
    let asked = data.last_known.filter(|l| l.key == data.key);
    let in_flight = asked.is_some_and(|l| l.answer.is_none());
    match &data.archives {
        None => {
            col = col.push(kit::muted(
                "presence not read — no archive.v1 provider is known to ask",
            ));
        }
        Some(archives) if archives.is_empty() => {
            col = col.push(kit::muted(
                "no archive.v1 provider in presence — no archive is there to hold this \
                 key's last-known value",
            ));
        }
        Some(archives) => {
            let buttons = archives.iter().map(|a| {
                let b = kit::secondary(kit::caption(format!("ask {a}"))).padding(sp.xs);
                let b = if in_flight {
                    b
                } else {
                    b.on_press(msg(data.slot, DetailMsg::LastKnown(a.clone())))
                };
                b.into()
            });
            col = col.push(
                iced::widget::Row::from_iter(buttons)
                    .spacing(sp.xs)
                    .wrap()
                    .vertical_spacing(sp.xs),
            );
        }
    }
    let Some(asked) = asked else {
        return col.into();
    };
    match &asked.answer {
        None => {
            col = col.push(kit::muted(format!(
                "asking {} for last-known …",
                asked.archive
            )));
        }
        Some(Err(e)) => {
            col = col.push(kit::error(format!(
                "last-known from {} failed: {e}",
                asked.archive
            )));
        }
        Some(Ok(answer)) => {
            col = col.push(kit::callout(
                crate::view::theme::Tone::Info,
                kit::caption(format!("{} (S5, S6)", last_known_label(&asked.archive))),
            ));
            let Some(row) = answer.row() else {
                return col
                    .push(kit::muted(format!(
                        "{} holds no last-known value of this key",
                        asked.archive
                    )))
                    .into();
            };
            col = col.push(
                row![
                    kit::data_chip(match row.confirmed {
                        Some(true) => "confirmed by alignment (§4.4)",
                        Some(false) => "not confirmed by alignment (§4.4)",
                        None => "alignment not stated",
                    }),
                    stamp_chip(row.timestamp.as_ref(), answer.provenance),
                ]
                .spacing(sp.xs)
                .align_y(iced::Alignment::Center),
            );
            if let Some(line) = stamp_line(row.timestamp.as_ref()) {
                col = col.push(line);
            }
            col = col.push(match &row.value {
                StateValue::Deleted => {
                    kit::muted("a tombstone the archive holds — the key was retired, not emptied")
                }
                StateValue::Value { payload } => {
                    decoded_pane(answer.decoded.as_deref(), payload.size, sp)
                }
            });
        }
    }
    col.into()
}

/// The key's identity (#614, FL7): where zk2's ladder got, worded per rung.
///
/// The badge names the rung — the tree's own badge, so the two never read
/// differently — then the reason when the ladder stopped short, then the
/// facts as fields (#539): the address, interface, kind and resource a key
/// resolved to, its template's values, and the type and revision a contract
/// gave it. A field appears only when its fact exists, so absent stays
/// absent rather than becoming a blank box. v1's facts ladder stood here;
/// a v1 key is foreign now, and reads "not zk2" like any other.
pub(crate) fn identity_section<'a>(
    resolution: &'a Resolution,
    metric: Option<&'a MetricRead>,
    timing: Option<&'a crate::timing::TimingRead>,
    sp: Spacing,
) -> Element<'a, Message> {
    let identity = &resolution.identity;
    let rung = Rung::of(identity);
    let mut col = Column::new().spacing(sp.sm);
    col = col.push(kit::tone_badge(rung.tone(), rung.label()));
    if let Some(why) = &identity.unresolved {
        col = col.push(kit::muted(why_line(why)));
    }
    let mut grid: Vec<Element<'_, Message>> = Vec::new();
    match &identity.group {
        KeyGroup::Resource {
            address,
            iface,
            token,
            resource,
        } => {
            grid.push(kit::field("ADDRESS", kit::mono(address.clone())));
            grid.push(kit::field("IFACE", kit::mono(iface.clone())));
            grid.push(kit::field("KIND", kit::mono(token.clone())));
            if let Some(r) = resource {
                grid.push(kit::field("RESOURCE", kit::mono(r.clone())));
            }
        }
        KeyGroup::Control { address, form } => {
            if let Some(a) = address {
                grid.push(kit::field("ADDRESS", kit::mono(a.clone())));
            }
            grid.push(kit::field("CONTROL", kit::mono(format!("@zk/{form}"))));
        }
        KeyGroup::NotInNamespace | KeyGroup::NotZk2 => {}
    }
    if let Some(r) = &resolution.resolved {
        grid.push(kit::field("TYPE", kit::mono(crate::keyfacts::type_name(r))));
        // The metric's kind and unit (#819, TD; telemetry.v1 §2.5), beside
        // the type that carries them, each with where it came from.
        if let Some(m) = metric {
            grid.push(kit::field("METRIC KIND", kit::mono(m.kind_words())));
            grid.push(kit::field("UNIT", kit::mono(m.unit().words())));
        }
        // A timed stream's values (#908, MC; timing.v1 §2.2), beside the
        // type they time, each with where it came from: a value appears
        // only when it is declared.
        if let Some(d) = timing.and_then(crate::timing::TimingRead::declared) {
            // The source is the revision REVISION names beside it.
            let source = |v: std::time::Duration| format!("{} ms (the contract's)", v.as_millis());
            if let Some(p) = d.period {
                grid.push(kit::field("PERIOD", kit::mono(source(p))));
            }
            if let Some(dl) = d.deadline {
                grid.push(kit::field(
                    "DEADLINE",
                    kit::mono(if d.below_period() {
                        format!(
                            "{} ms, below its period: no deadline (§2.5)",
                            dl.as_millis()
                        )
                    } else {
                        source(dl)
                    }),
                ));
            }
            if let Some(l) = d.lifespan {
                grid.push(kit::field("LIFESPAN", kit::mono(source(l))));
            }
        }
        grid.push(kit::field(
            "REVISION",
            kit::mono(short_fingerprint(&r.revision.fingerprint().to_string())),
        ));
    }
    if !grid.is_empty() {
        col = col.push(kit::fields(grid));
    }
    if let Some(why) = metric.and_then(MetricRead::invalid) {
        col = col.push(kit::muted(format!(
            "telemetry.v1 declaration invalid (§2.2): {why}"
        )));
    }
    if let Some(zenkey_model::timing::Declaration::Invalid(why)) = timing.map(|t| &t.declaration) {
        col = col.push(kit::muted(format!(
            "timing.v1 declaration invalid (§2.2): {} — no period, deadline or lifespan is \
             judged",
            why.iter()
                .map(|w| w.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    if !identity.values.is_empty() {
        col = col.push(kit::muted(
            identity
                .values
                .iter()
                .map(|(k, v)| format!("{k} = {}", v.join("/")))
                .collect::<Vec<_>>()
                .join(" · "),
        ));
    }
    col.into()
}

/// One of §5's answers as a judgement badge (#908, MC): the question's
/// finding wears the finding's mark, its other pole the clean one, and the
/// unestablished kinds their own; the fixture's token and reason always in
/// the text.
fn timing_badge<'a>(
    q: zenkey_model::timing::Question,
    what: &'static str,
    j: &zenkey_model::timing::Judged,
) -> Element<'a, Message> {
    use crate::view::theme::JudgementTone as T;
    use zenkey_model::timing::Answer;
    let tone = match j.answer {
        a if a == q.finding() => T::Established,
        Answer::Yes | Answer::No => T::NotEstablished,
        Answer::Unobservable => T::Unobservable,
        Answer::NotAsked | Answer::NotThisProfiles => T::NotAsked,
    };
    kit::badge_judgement(
        tone,
        format!("{what}: {} ({})", j.answer.as_str(), j.reason.as_str()),
    )
}

/// A `timing.v1` stream over the recorded history (#908, MC): the window's
/// own view of §5's deadline and period questions, each the model's
/// answer, the misses with the owner's state the presence link gave, and
/// the lifespan said not judged here (no ε).
fn timing_section<'a>(t: &'a crate::timing::TimingRead, sp: Spacing) -> Element<'a, Message> {
    use zenkey_model::timing::Question;
    let mut col = Column::new().spacing(sp.xs);
    col = col.push(kit::section_header("Timing", None));
    if let Some(why) = t.unobservable {
        col = col.push(kit::muted(format!("the window's own view: {why}")));
        return col.into();
    }
    col = col.push(kit::muted(format!(
        "the window's own view (timing.v1 §2.7): {} over the recorded {:.1}s, each taken at \
         the data pump's callback; the owner {} now, by the presence link",
        kit::plural(t.arrivals, "arrival"),
        t.span.as_secs_f64(),
        t.owner_now.as_str()
    )));
    if t.evicted > 0 {
        col = col.push(kit::muted(format!(
            "the history ring dropped {} older, so this window starts at the oldest it kept",
            kit::plural(usize::try_from(t.evicted).unwrap_or(usize::MAX), "entry")
        )));
    }
    if let Some(j) = &t.deadline {
        col = col.push(timing_badge(Question::Deadline, "deadline", j));
        col = col.push(kit::muted(j.reason.says()));
    }
    for (due, late, owner) in &t.latest_misses {
        col = col.push(kit::muted(format!(
            "missed: due at {due:.0} ms, reported {late:.0} ms after, its owner {}",
            owner.as_str()
        )));
    }
    if t.misses as usize > t.latest_misses.len() {
        col = col.push(kit::muted(format!(
            "the latest {} of {} misses",
            t.latest_misses.len(),
            t.misses
        )));
    }
    if t.misses > 0 {
        col = col.push(kit::muted(
            "a miss is never \"down\": nothing arrived within the deadline, as this window \
             saw it; whether the owner is gone is presence's word (§2.5)",
        ));
    }
    if let Some(j) = &t.period {
        col = col.push(timing_badge(Question::Period, "period", j));
        let span = t
            .period_reading
            .map(|p| {
                format!(
                    "{} in a {:.0} ms span: ",
                    kit::plural(p.arrivals as usize, "arrival"),
                    p.span.unwrap_or_default().as_secs_f64() * 1e3
                )
            })
            .unwrap_or_default();
        col = col.push(kit::muted(format!("{span}{}", j.reason.says())));
    }
    if t.declared().is_some_and(|d| d.lifespan.is_some()) {
        col = col.push(kit::muted(
            "lifespan: not judged here — it takes the deployment's stated clock bound, which \
             zengui is not told (§2.6); zenctl timing --clock-bound judges it",
        ));
    }
    col.into()
}

/// Why the ladder stopped, as a sentence: the engine's words for the rung,
/// and for the two rungs that are facts about the key rather than about
/// the deployment, that they are no error (O1).
fn why_line(why: &Unresolved) -> String {
    match why {
        Unresolved::NotZk2 { .. } | Unresolved::NotInNamespace { .. } => {
            format!("{} — a fact, not an error (O1)", why.words())
        }
        _ => why.words(),
    }
}

/// `sha256:` and sixteen hex digits — the engine's own short form.
fn short_fingerprint(fp: &str) -> String {
    let hex = fp.strip_prefix("sha256:").unwrap_or(fp);
    match hex.get(..16) {
        Some(head) if hex.len() > 16 => format!("sha256:{head}…"),
        _ => fp.to_owned(),
    }
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
    // Set into the panel as an inset well (#539).
    kit::inset(col).width(Length::FillPortion(1)).into()
}

/// How many `Invalid` violation sentences render before the list stops and
/// says so — a schema with a hundred failed clauses is one bad payload, not a
/// hundred rows of pane.
const VIOLATION_ROWS: usize = 8;

/// The decoded side, tagged with how it was produced — and with the payload's
/// conformance (#164) through the lens (#614, FL6a): four poles, never a
/// boolean, and a "not checked" always names the rung that stopped it
/// ("no provider of this address in presence" never wears "not a zk2 data
/// key"'s words — #246).
fn decoded_pane<'a>(
    decoded: Option<&'a DecodedValue>,
    payload_len: usize,
    sp: Spacing,
) -> Element<'a, Message> {
    use zenkey_fleet::report::{Conformance, Rendered};

    let mut col = Column::new().spacing(sp.xs);
    match decoded {
        None => {
            col = col.push(kit::muted("decoding…"));
        }
        Some(value) => {
            let checked = &value.checked;
            // The document was rendered once, in the check's task (#345);
            // all this pane decides is how much of it to draw.
            let (shown, elided) = clamp_document(&value.document);
            let body = |shown: &str| {
                if shown.is_empty() {
                    format!("<{payload_len} bytes>")
                } else {
                    shown.to_string()
                }
            };
            match &checked.rendering.rendered {
                Rendered::Value { declared, .. } => {
                    col = col.push(kit::tone_badge(
                        IdentityTone::Resolved,
                        format!("contract-decoded <{declared}>"),
                    ));
                    col = col.push(kit::mono(shown.to_string()));
                }
                Rendered::Structural { why, .. } => {
                    // The honest ladder: the rung the lens stopped at, then
                    // the structural sniff.
                    col = col.push(kit::muted(format!("structural — {}", why.words())));
                    col = col.push(kit::mono(body(shown)));
                }
                Rendered::Undecodable { declared, .. } => {
                    col = col.push(kit::muted(format!(
                        "<{declared}?> structural (the bytes did not decode as their \
                         declared type)"
                    )));
                    col = col.push(kit::mono(body(shown)));
                }
                Rendered::Opaque { media_type } => {
                    // A raw type: the contract says these bytes are not this
                    // tool's to read, so only their size is (§7.1).
                    col = col.push(kit::muted(format!(
                        "raw {media_type} — the contract declares no structure to render"
                    )));
                    col = col.push(kit::mono(format!("<{payload_len} bytes>")));
                }
            }
            if elided > 0 {
                col = col.push(kit::muted(format!("… {elided} more bytes not shown")));
            }
            // Where the key resolved to, when it did: the interface, the
            // resource and the revision a reader would hold.
            if let Some(r) = &checked.rendering.resource {
                col = col.push(kit::muted(format!(
                    "resolved: {} · {} · {}",
                    r.iface, r.resource, r.fingerprint
                )));
            }
            // The verdict, glyph and word (#164/#193).
            col = col.push(kit::badge_verdict(
                crate::verdict::tone(&checked.conformance),
                crate::verdict::label(&checked.conformance),
            ));
            match &checked.conformance {
                Conformance::Invalid { violations } => {
                    for e in violations.iter().take(VIOLATION_ROWS) {
                        col = col.push(kit::muted(format!("violation: {e}")));
                    }
                    if violations.len() > VIOLATION_ROWS {
                        col = col.push(kit::muted(format!(
                            "+{} more not shown (display bound)",
                            violations.len() - VIOLATION_ROWS
                        )));
                    }
                }
                Conformance::Undecodable { reason, .. } => {
                    col = col.push(kit::muted(format!("decode error: {reason}")));
                }
                // The rung is in the badge's parenthetical already, and a
                // valid value needs no sentence.
                Conformance::NotChecked { .. } | Conformance::Valid => {}
            }
        }
    }
    // Set into the panel as an inset well (#539).
    kit::inset(col).width(Length::FillPortion(1)).into()
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

    fn entry(
        priority: zenoh::qos::Priority,
        congestion: zenoh::qos::CongestionControl,
        express: bool,
    ) -> crate::history::HistoryEntry {
        crate::history::HistoryEntry {
            seq: 0,
            received: Instant::now(),
            timestamp: None,
            stamp: None,
            is_delete: false,
            len: 2,
            encoding: "application/json".into(),
            payload: zenoh::bytes::ZBytes::from("{}"),
            priority,
            congestion_control: congestion,
            reliability: zenoh::qos::Reliability::Reliable,
            express,
            qos_recorded: true,
            source: None,
            attachment: None,
            value: None,
            preview: "{}".into(),
        }
    }

    /// #120 on zk2 (#614, FL8a): declared against observed compares the
    /// three axes a sample carries, in the contract's spelling, and names
    /// every one that differs — reliability is the link's and is not among
    /// them.
    #[test]
    fn declared_vs_observed_compares_axes_and_names_each_that_differs() {
        use zenoh::qos::{CongestionControl, Priority};
        let declared = QosAxes {
            priority: "data".into(),
            congestion: "block".into(),
            express: false,
        };
        let same = entry(Priority::Data, CongestionControl::Block, false);
        assert_eq!(
            zenkey_fleet::qos_mismatch(&declared, &observed_axes(&same)),
            None
        );
        let off = entry(Priority::RealTime, CongestionControl::Block, true);
        let m =
            zenkey_fleet::qos_mismatch(&declared, &observed_axes(&off)).expect("two axes differ");
        assert_eq!(m.differs, ["priority", "express"]);
        assert_eq!(axes_token(&declared), "data/block");
        assert_eq!(axes_token(&m.observed), "real_time/block+express");
    }

    /// Nothing declares a QoS the lens could not reach, and the pane says
    /// why in the rung's words — never a guess, never a mismatch (O4).
    #[test]
    fn an_unresolved_key_declares_no_qos_and_says_why() {
        let set = zenkey_fleet::ContractSet::new();
        let lens = zenkey_fleet::Lens::new("", None, &set);
        let foreign = lens.resolve("demo/example/foo");
        let why = declared_qos(Some(&foreign)).expect_err("a foreign key declares nothing");
        assert_eq!(why, "a foreign key — no contract declares its QoS");
        let ghost = lens.resolve("zk2/robot9/nav/nav.v2/state/status");
        let why = declared_qos(Some(&ghost)).expect_err("a ghost declares nothing");
        assert!(why.starts_with("nothing declares it — "), "{why}");
        assert!(declared_qos(None).is_err());
    }

    /// The token spelling is byte-for-byte the CLI's %q, so the two
    /// frontends agree on what the wire said.
    #[test]
    fn the_token_matches_the_cli_spelling() {
        use zenoh::qos::{CongestionControl, Priority};
        let e = entry(Priority::InteractiveHigh, CongestionControl::Block, true);
        assert_eq!(qos_token(&e), "interactive_high/block/reliable+express");
    }

    /// An event's retention reads in the largest unit that divides it.
    #[test]
    fn a_retention_reads_in_its_largest_whole_unit() {
        assert_eq!(human_span(7 * 86_400), "1w");
        assert_eq!(human_span(3 * 86_400), "3d");
        assert_eq!(human_span(5_400), "90m");
        assert_eq!(human_span(45), "45s");
    }
}
