//! The Traffic tab (#542): where the traffic is — five tiles, the whole
//! session's rate over time, and the heaviest keys with their share.
//!
//! A stream of the session, so a tab of the Activity dock beside echo and
//! the doctor. Everything here was already computed — the monitor's
//! snapshot, the tick's totals, the echo ring's loss counters — and surfaced
//! only as one status-strip sentence and numbers on individual tree rows.
//!
//! Honest the same way the strip is (RFC 09 §5.1): with no session a tile
//! says "not asked", never 0; with nothing watched it says so by choice; the
//! three ways a sample can go missing are three numbers, never summed; and a
//! share is a share of the *retained* keys whenever the bound has retired any.

use std::time::Instant;

use iced::widget::{Column, column, row};
use iced::{Element, Length};

use super::spark;
use super::theme::SeriesTone;
use super::tokens::{Spacing, face};
use crate::message::{Message, PaneMsg, Subject, SubjectMsg};
use crate::series::Series;
use crate::traffic::{TrafficSort, TrafficTable, share};
use crate::view::kit::{self, human_bytes, human_rate};

/// The tab's interactions.
#[derive(Debug, Clone)]
pub enum TrafficMsg {
    Sort(TrafficSort),
}

fn msg(m: TrafficMsg) -> Message {
    Message::Pane(PaneMsg::Traffic(m))
}

/// Everything the tab reads, as plain data.
pub struct TrafficData<'a> {
    /// `None` before the tab was first shown with a session behind it.
    pub table: Option<&'a TrafficTable>,
    pub sort: TrafficSort,
    pub total_rate: &'a Series,
    pub cache: &'a iced::widget::canvas::Cache,
    /// Whether a monitor exists at all — without one, nothing was asked.
    pub session: bool,
    pub watched: usize,
    pub replaying: bool,
    pub totals: crate::message::WatchedTotals,
    pub keys: usize,
    pub keys_evicted: u64,
    pub keys_unwatched: u64,
    /// The broadcast outran us / our own batch cap / the ring's bound — the
    /// echo ring's three loss counters, each its own number (O6).
    pub lagged: u64,
    pub coalesced: u64,
    pub evicted: u64,
    pub sp: Spacing,
}

pub fn section<'a>(d: TrafficData<'a>) -> Column<'a, Message> {
    let sp = d.sp;
    let sort = kit::segmented(
        TrafficSort::ALL
            .into_iter()
            .map(|s| kit::Segment {
                value: s,
                label: s.label().to_string(),
                icon: None,
                count: None,
                tip: None,
            })
            .collect(),
        Some(d.sort),
        |s| msg(TrafficMsg::Sort(s)),
    );
    let mut col = column![kit::section_header("Traffic", Some(sort))].spacing(sp.sm);

    // What every tile says when its number was never taken.
    let unasked: Option<&'static str> = if !d.session {
        Some("not asked — no session yet")
    } else if d.watched == 0 {
        Some("nothing observed by choice — watch a subtree or the scope")
    } else {
        None
    };
    let source = if d.replaying {
        " · from the replayed file"
    } else {
        ""
    };
    let tile = |icon, eyebrow, value: String, footer: String| match unasked {
        Some(why) => kit::stat(icon, eyebrow, "—".to_string(), why.to_string()),
        None => kit::stat(icon, eyebrow, value, format!("{footer}{source}")),
    };
    col = col.push(
        row![
            tile(
                kit::Icon::Rate,
                "RATE",
                human_rate(d.totals.rate_hz),
                format!("across {}", kit::plural(d.watched, "watch")),
            ),
            tile(
                kit::Icon::Samples,
                "SAMPLES",
                d.totals.samples.to_string(),
                "since the watches began".to_string(),
            ),
            tile(
                kit::Icon::Volume,
                "VOLUME",
                human_bytes(d.totals.bytes),
                "payload bytes".to_string(),
            ),
            tile(
                kit::Icon::Keys,
                "KEYS",
                d.keys.to_string(),
                keys_footer(d.keys_evicted, d.keys_unwatched),
            ),
            tile(
                kit::Icon::Dropped,
                "DROPPED",
                d.lagged.to_string(),
                // Three facts, three numbers — never one sum (O6).
                format!(
                    "lagged by the broadcast · coalesced {} · evicted {}",
                    d.coalesced, d.evicted
                ),
            ),
        ]
        .spacing(sp.sm),
    );

    col = col.push(spark::chart(
        "rate, all watched keys",
        d.total_rate,
        SeriesTone::Rate,
        None,
        d.cache,
        sp,
    ));

    if unasked.is_some() {
        return col;
    }
    let Some(table) = d.table else {
        return col.push(kit::muted("ranking the keys…"));
    };
    if table.rows.is_empty() {
        return col.push(kit::empty(
            kit::EmptyKind::Silent,
            "No key carried traffic yet",
            "The watches are active and nothing has arrived. That is not a statement \
             about the bus (RFC 05 §3.1).",
        ));
    }
    let mut note = format!(
        "top {} of {} — ranked by {}",
        table.rows.len(),
        kit::plural(table.considered, "key"),
        table.sort.label()
    );
    if d.keys_evicted > 0 {
        // The bound retired keys: their traffic is in no row and in no total
        // this table divides by.
        note.push_str(" · share of the retained keys' rate");
    } else {
        note.push_str(" · share of the watched rate");
    }
    col = col.push(kit::muted(note));
    let total: f64 = d.totals.rate_hz;
    let now = Instant::now();
    let mut rows = Column::new().spacing(sp.xs);
    rows = rows.push(header(sp));
    for r in &table.rows {
        let age = r
            .last_seen
            .map(|t| now.saturating_duration_since(t).as_secs_f32());
        let line = row![
            kit::caption(r.key.as_str())
                .font(face::MONO)
                .width(Length::FillPortion(6)),
            num(r.count.to_string()),
            num(human_bytes(r.bytes)),
            num(human_rate(r.rate_hz)),
            iced::widget::container(kit::share_bar(share(r.rate_hz, total)))
                .width(Length::FillPortion(3)),
            iced::widget::container(match age {
                Some(a) => kit::freshness(a),
                None => kit::muted("—"),
            })
            .width(Length::FillPortion(1)),
        ]
        .spacing(sp.sm)
        .align_y(iced::Alignment::Center);
        let key = r.key.clone();
        rows = rows.push(
            kit::row_button(line, false)
                .padding([sp.xs, sp.xs])
                .on_press(Message::Subject(SubjectMsg::Select(Subject::Key(key)))),
        );
    }
    // The table scrolls within the dock; the tiles and the chart above it
    // stay put, so the totals never scroll out of sight of the ranking.
    col.push(
        iced::widget::scrollable(rows)
            .height(Length::Fill)
            .width(Length::Fill),
    )
}

fn keys_footer(evicted: u64, unwatched: u64) -> String {
    let mut out = if evicted > 0 {
        format!("+{evicted} retired — bound reached")
    } else {
        "within the bound".to_string()
    };
    if unwatched > 0 {
        out.push_str(&format!(" · +{unwatched} retired by unwatch"));
    }
    out
}

fn num<'a>(s: String) -> Element<'a, Message> {
    iced::widget::container(kit::caption(s).font(face::MONO))
        .width(Length::FillPortion(2))
        .into()
}

fn header<'a>(sp: Spacing) -> Element<'a, Message> {
    let cell = |s: &'static str, p: u16| {
        iced::widget::container(kit::eyebrow(s)).width(Length::FillPortion(p))
    };
    row![
        cell("KEY", 6),
        cell("SAMPLES", 2),
        cell("BYTES", 2),
        cell("RATE", 2),
        cell("SHARE", 3),
        cell("SEEN", 1),
    ]
    .spacing(sp.sm)
    .padding([0.0, sp.xs])
    .into()
}
