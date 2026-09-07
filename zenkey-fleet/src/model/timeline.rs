//! The fleet timeline (#216): **deliberately no edges** — a merged ordering
//! shows *when* things were seen on which clock, never that one caused
//! another. A line drawn between two lanes would claim a causality no
//! observer on this bus can establish, so there is no line; there is a
//! position on a named axis, and the axis says what a position means.
//!
//! Every pane an explorer had was one key or one flat scrollback. The
//! question an operator asks is "what happened, across the fleet, in that
//! ten seconds?" — and answering it honestly means three things at once:
//! one ordering, stated per axis; lanes per origin and producer, so the
//! fleet's parallelism is visible; and the three provenances of a position
//! (arrival, HLC, sequence number) kept apart, because they are three
//! different measurements with three different claims.
//!
//! # The two axes, and why they never mix
//!
//! **Arrival** is this observer's monotonic clock, µs since the window
//! epoch. Every sample has one, it is total, and it says nothing about when
//! anything was produced — only when it was seen here.
//!
//! **HLC** is the sample's `uhlc` timestamp, whose id names the node that
//! stamped it (RFC 09 §5.1 **O7** — not necessarily the publisher). What an
//! HLC order can claim rests on a fact about zenoh 1.10, verified in its
//! source: `treat_timestamp!` in `net/routing/dispatcher/pubsub.rs` updates
//! a node's HLC on receive **only when the node has an HLC**, and
//! `net/runtime/mod.rs` builds one only where `timestamping.enabled` is true
//! for the node's whatami — routers default **true**, peers and clients
//! **false**. The explorer session sets no timestamping, so an observer
//! holds no HLC at all. Three consequences, each of which is a field here:
//!
//! * values from **one** stamper are causally ordered — that node's HLC is
//!   monotonic and was updated by everything it forwarded
//!   ([`HlcClaim::HappensBefore`]);
//! * values from **different** stampers compare as skewed wall clocks
//!   unless they passed through a common timestamping node, which this
//!   observer cannot see ([`HlcClaim::SkewedWallClock`]);
//! * an observer can never place "now" on the HLC axis, so a break — a
//!   drop this observer suffered — has an arrival position and **no HLC
//!   position**. [`HlcOrdering`] carries no breaks; the report states the
//!   total and points at `--order arrival` for where they fell.
//!
//! `drop_future_timestamp = false` also lets a router re-stamp a sample
//! whose HLC runs ahead; O7's `Foreign` provenance covers it, and the
//! per-lane provenance counts keep that population visible.
//!
//! An **unstamped** sample has no HLC position, and that is enforced by the
//! type system rather than by a warning: [`Placed<HlcAxis>`] has exactly one
//! constructor, [`Placed::<HlcAxis>::new`], which returns
//! `Err(`[`Unstamped`]`)` for a row without an HLC — so an unstamped row
//! cannot be *selected onto* the HLC axis at all. It lives in
//! [`LaneId::Unstamped`] on the arrival axis, and the HLC report counts the
//! exclusion ([`HlcOrdering::unstamped_excluded`]).
//!
//! # The third provenance
//!
//! A per-`SampleSource` sequence-number lane would be the one ordering a
//! *publisher* vouches for. zenoh 1.9 and 1.10 deliver no `SourceInfo` to a
//! subscriber (`tests/stamper.rs` pins it), so [`SnLane::Unavailable`] is a
//! structural state with a fixed reason, never an empty vector: an empty
//! lane would read as "nobody skipped a number".
//!
//! # Live and replay are one projection
//!
//! [`TimelineRow::from_view`] and [`Ingested::from_zrec`] build the same row
//! from a live [`SampleView`] and from a `.zrec` line, and the test in
//! `tests/timeline_identity.rs` asserts the two reports are equal. That is
//! what this layer is for (nothing here takes a session), and it is what
//! makes "the same ten seconds, from the file" a claim rather than a hope.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::str::FromStr;
use std::time::Instant;

use crate::bus::monitor::{SampleView, StampProvenance};
use crate::model::facts::{KeyFacts, KeyShape};
use crate::report::{
    ARRIVAL_CLOCK, AxisLabel, BreakKind, HlcClaim, LaneId, LaneSummary, OrderLabel, Provenance,
    ProvenanceCounts, RowKind, SN_UNAVAILABLE_REASON, SnLaneReport, TimelineEntry, TimelineReport,
    TimelineSource,
};
use crate::tape::record::ZrecItem;

/// A sample's HLC, decomposed: the value, who stamped it, and how far that
/// stamper can be attributed to the publisher (#213).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlcStamp {
    pub ntp64: u64,
    /// The stamping node's id, as `uhlc` prints it — the `.zrec` spelling.
    pub stamper: String,
    pub provenance: Provenance,
}

impl HlcStamp {
    /// `<ntp64>/<stamper>` — what `uhlc::Timestamp: Display` prints and
    /// `FromStr` reads, so it round-trips through a `.zrec`.
    pub fn to_wire(&self) -> String {
        format!("{}/{}", self.ntp64, self.stamper)
    }
}

/// One sample, reduced to what a timeline needs: where it sits on each
/// axis, which lane it belongs to, and what it was. No payload — the
/// timeline is about *when*, and a pane that wants the bytes has the
/// retained window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineRow {
    pub key: String,
    pub lane: LaneId,
    /// Arrival, µs since the window epoch (the observer's monotonic clock).
    pub t_us: u64,
    /// `None` exactly when the lane is [`LaneId::Unstamped`].
    pub hlc: Option<HlcStamp>,
    /// The publishing entity, `zid:eid`, when `SourceInfo` rode the sample.
    pub source: Option<String>,
    /// That entity's sequence number for this sample.
    pub sn: Option<u32>,
    pub kind: RowKind,
    pub payload_bytes: usize,
}

impl TimelineRow {
    /// A row from a live sample, `t_us` measured from `epoch` — the instant
    /// the caller's watches were all declared. A sample received before the
    /// epoch saturates to 0, as `ZrecWriter` does.
    pub fn from_view(view: &SampleView, epoch: Instant, base: &str) -> TimelineRow {
        let t_us = u64::try_from(view.received.saturating_duration_since(epoch).as_micros())
            .unwrap_or(u64::MAX);
        let hlc = view.timestamp.as_ref().map(|t| HlcStamp {
            ntp64: t.get_time().as_u64(),
            stamper: t.get_id().to_string(),
            provenance: match view.stamped_by {
                Some(StampProvenance::SelfStamped) => Provenance::SelfStamped,
                Some(StampProvenance::Foreign { .. }) => Provenance::Foreign,
                Some(StampProvenance::Unattributable { .. }) | None => Provenance::Unattributable,
            },
        });
        TimelineRow::assemble(
            view.key.clone(),
            t_us,
            hlc,
            view.source.map(|s| format!("{}:{}", s.zid, s.eid)),
            view.source.map(|s| s.sn),
            if view.kind == zenoh::sample::SampleKind::Delete {
                RowKind::Delete
            } else {
                RowKind::Put
            },
            view.payload.len(),
            base,
        )
    }

    /// The one place a lane is decided: unstamped first, then the key.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        key: String,
        t_us: u64,
        hlc: Option<HlcStamp>,
        source: Option<String>,
        sn: Option<u32>,
        kind: RowKind,
        payload_bytes: usize,
        base: &str,
    ) -> TimelineRow {
        let lane = match &hlc {
            None => LaneId::Unstamped,
            Some(_) => match KeyFacts::project(base, &key).shape {
                KeyShape::V1(f) => LaneId::Origin {
                    origin: f.origin,
                    producer: f.producer,
                },
                KeyShape::NotUnderBase | KeyShape::Unparsed { .. } => LaneId::Foreign,
            },
        };
        TimelineRow {
            key,
            lane,
            t_us,
            hlc,
            source,
            sn,
            kind,
            payload_bytes,
        }
    }
}

/// A break in the sequence: what the observer did not see, or merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Break {
    /// Samples missed while behind (`StreamItem::Dropped`, or a `.zrec`
    /// drop record) — O6.
    Dropped(u64),
    /// Distinct samples a consumer merged into fewer (a GUI tick).
    Coalesced(u64),
}

impl Break {
    pub fn n(self) -> u64 {
        match self {
            Break::Dropped(n) | Break::Coalesced(n) => n,
        }
    }

    fn kind(self) -> BreakKind {
        match self {
            Break::Dropped(_) => BreakKind::Dropped,
            Break::Coalesced(_) => BreakKind::Coalesced,
        }
    }
}

/// A break at its arrival position: after the first `after` rows, in the
/// order they were ingested. An index rather than a time, because a drop
/// has no instant of its own — the observer learns of it *between* two
/// samples — and an index is what a live drain and a `.zrec` reader can
/// agree on exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedBreak {
    pub after: usize,
    /// The lane the break belongs to, when a consumer coalesced one lane.
    /// A monitor drop is lane-less: the broadcast does not know whose
    /// samples it lost.
    pub lane: Option<LaneId>,
    pub kind: Break,
}

/// What one `.zrec` line turns into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingested {
    Row(TimelineRow),
    Break(Break),
    /// A version-2 preamble row (RFC 13 §4.1): state at capture start,
    /// not an arrival — it has no place on either axis, and a reader
    /// counts it apart from observed rows (O6). The key, so a consumer can
    /// say which.
    Preamble {
        key: String,
    },
    /// A version-2 trigger record: what fired, and to what. A marker the
    /// consumer may place where it fell; never a row.
    Trigger {
        rule: String,
        to: crate::report::CondState,
    },
}

impl Ingested {
    /// A `.zrec` line as the live path would have seen it (RFC 09 §5.2):
    /// `t` verbatim as the arrival offset, `timestamp` parsed back through
    /// `uhlc::Timestamp: FromStr`, and the stamper judged against the
    /// row's `source` exactly as [`StampProvenance`] judges a live sample
    /// — `Unattributable` unless the row carried one, so live and replay
    /// classify identically.
    ///
    /// A row without `t` (a hand-piped ndjson row rather than a capture)
    /// sits at 0: it has no pacing, and inventing one would be an ordering
    /// claim. A `timestamp` that does not parse reads as unstamped — the
    /// row is kept (O1), and the arrival axis still holds it.
    pub fn from_zrec(item: &ZrecItem, base: &str) -> Ingested {
        match item {
            ZrecItem::Dropped(n) => Ingested::Break(Break::Dropped(*n)),
            ZrecItem::Preamble { row, .. } => Ingested::Preamble {
                key: row.key.clone(),
            },
            ZrecItem::Trigger(t) => Ingested::Trigger {
                rule: t.rule.clone(),
                to: t.to,
            },
            ZrecItem::Sample {
                row,
                t_us,
                timestamp,
                source,
            } => {
                let (entity, sn) = match source.as_deref() {
                    Some(s) => match s.rsplit_once('#') {
                        Some((entity, sn)) => (Some(entity.to_string()), sn.parse::<u32>().ok()),
                        None => (Some(s.to_string()), None),
                    },
                    None => (None, None),
                };
                let hlc = timestamp
                    .as_deref()
                    .and_then(|s| zenoh::time::Timestamp::from_str(s).ok())
                    .map(|t| HlcStamp {
                        ntp64: t.get_time().as_u64(),
                        stamper: t.get_id().to_string(),
                        provenance: provenance_of(t.get_id(), entity.as_deref()),
                    });
                Ingested::Row(TimelineRow::assemble(
                    row.key.clone(),
                    t_us.unwrap_or(0),
                    hlc,
                    entity,
                    sn,
                    if row.delete {
                        RowKind::Delete
                    } else {
                        RowKind::Put
                    },
                    row.payload.len(),
                    base,
                ))
            }
        }
    }
}

/// [`StampProvenance::of`]'s judgement over the `.zrec` spelling of a
/// source: the entity's zid against the stamper's id, exact, and
/// `Unattributable` when there is nothing to compare against (O4).
fn provenance_of(stamper: &zenoh::time::TimestampId, entity: Option<&str>) -> Provenance {
    let Some(entity) = entity else {
        return Provenance::Unattributable;
    };
    let zid = entity.split(':').next().unwrap_or(entity);
    match zenoh::config::ZenohId::from_str(zid) {
        Ok(zid) if zenoh::time::TimestampId::from(zid) == *stamper => Provenance::SelfStamped,
        Ok(_) => Provenance::Foreign,
        Err(_) => Provenance::Unattributable,
    }
}

/// Which axis to order on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Arrival,
    Hlc,
}

impl Order {
    pub fn label(self) -> OrderLabel {
        match self {
            Order::Arrival => OrderLabel::Arrival,
            Order::Hlc => OrderLabel::Hlc,
        }
    }
}

/// The arrival axis: every row has a position on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrivalAxis;

/// The HLC axis: only a stamped row has a position on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HlcAxis;

/// The refusal: this row carries no HLC and has no place on that axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unstamped;

/// A row placed on one axis. The axis is a type parameter so that a
/// `Vec<Placed<HlcAxis>>` *cannot* hold an unstamped row: the only way to
/// make one is [`Placed::<HlcAxis>::new`], and it refuses.
///
/// The arrival constructor is total:
///
/// ```
/// use zenkey_fleet::model::timeline::{ArrivalAxis, Placed, TimelineRow};
/// use zenkey_fleet::report::{LaneId, RowKind};
/// let row = TimelineRow {
///     key: "plain/key".into(), lane: LaneId::Unstamped, t_us: 1, hlc: None,
///     source: None, sn: None, kind: RowKind::Put, payload_bytes: 0,
/// };
/// let placed: Placed<ArrivalAxis> = Placed::<ArrivalAxis>::new(row);
/// assert_eq!(placed.row().t_us, 1);
/// ```
///
/// The HLC constructor is not — it returns a `Result`, and there is no
/// other constructor, so a function that promises a `Placed<HlcAxis>` for
/// an arbitrary row does not compile:
///
/// ```compile_fail
/// use zenkey_fleet::model::timeline::{HlcAxis, Placed, TimelineRow};
/// fn onto_the_hlc_axis(row: TimelineRow) -> Placed<HlcAxis> {
///     Placed::<HlcAxis>::new(row)
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed<A> {
    pos: usize,
    seq: usize,
    row: TimelineRow,
    _axis: PhantomData<A>,
}

impl<A> Placed<A> {
    /// Position in the merged ordering on this axis, 0-based. Assigned by
    /// the ordering that ranked it; 0 until then.
    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn row(&self) -> &TimelineRow {
        &self.row
    }

    pub fn into_row(self) -> TimelineRow {
        self.row
    }
}

impl Placed<ArrivalAxis> {
    /// Every row arrives; every row is placeable here.
    pub fn new(row: TimelineRow) -> Placed<ArrivalAxis> {
        Placed {
            pos: 0,
            seq: 0,
            row,
            _axis: PhantomData,
        }
    }
}

impl Placed<HlcAxis> {
    /// The **only** constructor: a row without an HLC has no position on
    /// this axis and is refused, never defaulted to its arrival time.
    ///
    /// Belt and braces on the lane too — a row assembled by hand into
    /// [`LaneId::Unstamped`] is refused whatever its `hlc` field says, so
    /// the lane's invariant ("exists only on the arrival axis") cannot be
    /// broken by a struct literal.
    pub fn new(row: TimelineRow) -> Result<Placed<HlcAxis>, Unstamped> {
        if row.hlc.is_none() || row.lane == LaneId::Unstamped {
            return Err(Unstamped);
        }
        Ok(Placed {
            pos: 0,
            seq: 0,
            row,
            _axis: PhantomData,
        })
    }

    fn stamp(&self) -> &HlcStamp {
        self.row
            .hlc
            .as_ref()
            .expect("a Placed<HlcAxis> is stamped by construction")
    }
}

/// One slot of a merged ordering: which lane, and which entry in it — or a
/// break, which belongs to the sequence and to no lane's vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    Sample { lane: LaneId, index: usize },
    Break(usize),
}

/// The window ordered by arrival: every row placed, breaks at the position
/// the observer learned of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrivalOrdering {
    pub lanes: BTreeMap<LaneId, Vec<Placed<ArrivalAxis>>>,
    pub breaks: Vec<PlacedBreak>,
    /// The merged sequence, `pos` order.
    pub merged: Vec<Slot>,
}

/// The window ordered by HLC: stamped rows placed, the rest counted, and
/// the claim the axis can make. **No breaks**: a drop has no HLC position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlcOrdering {
    pub lanes: BTreeMap<LaneId, Vec<Placed<HlcAxis>>>,
    /// Rows refused by [`Placed::<HlcAxis>::new`] — reported, so an HLC
    /// listing that is shorter than the arrival one says why.
    pub unstamped_excluded: usize,
    pub claim: HlcClaim,
    pub merged: Vec<Slot>,
}

/// The sequence-number provenance, present or structurally not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnLane {
    /// No row carried a sequence number. The reason is fixed because the
    /// cause is: this zenoh delivers none.
    Unavailable { reason: &'static str },
    /// Some rows did — per publishing entity, `(seq, sn)` pairs in arrival
    /// order, which is what a gap or a reset would be read from.
    Present {
        sources: BTreeMap<String, Vec<(usize, u32)>>,
    },
}

impl SnLane {
    fn report(&self) -> SnLaneReport {
        match self {
            SnLane::Unavailable { reason } => SnLaneReport::Unavailable { reason },
            SnLane::Present { sources } => SnLaneReport::Present {
                sources: sources.len(),
                samples: sources.values().map(Vec::len).sum(),
            },
        }
    }
}

/// The sequence-number lane over a window.
pub fn sn_lane(rows: &[TimelineRow]) -> SnLane {
    let mut sources: BTreeMap<String, Vec<(usize, u32)>> = BTreeMap::new();
    for (seq, row) in rows.iter().enumerate() {
        if let (Some(source), Some(sn)) = (&row.source, row.sn) {
            sources.entry(source.clone()).or_default().push((seq, sn));
        }
    }
    if sources.is_empty() {
        SnLane::Unavailable {
            reason: SN_UNAVAILABLE_REASON,
        }
    } else {
        SnLane::Present { sources }
    }
}

/// Order a window by arrival. Rows are ranked by `(t_us, ingest order)`,
/// so two samples that saturated to the same offset keep the order they
/// were seen in; a break with `after = k` is emitted before the row that
/// was ingested `k`-th.
pub fn arrival(rows: &[TimelineRow], breaks: &[PlacedBreak]) -> ArrivalOrdering {
    let mut placed: Vec<Placed<ArrivalAxis>> = rows
        .iter()
        .cloned()
        .enumerate()
        .map(|(seq, row)| Placed {
            seq,
            ..Placed::<ArrivalAxis>::new(row)
        })
        .collect();
    placed.sort_by_key(|p| (p.row.t_us, p.seq));

    let mut pending: Vec<(usize, &PlacedBreak)> = breaks.iter().enumerate().collect();
    pending.sort_by_key(|(i, b)| (b.after, *i));
    let mut pending = pending.into_iter().peekable();

    let mut lanes: BTreeMap<LaneId, Vec<Placed<ArrivalAxis>>> = BTreeMap::new();
    let mut merged = Vec::with_capacity(placed.len() + breaks.len());
    let mut pos = 0usize;
    for mut p in placed {
        while let Some((i, b)) = pending.peek()
            && b.after <= p.seq
        {
            merged.push(Slot::Break(*i));
            pos += 1;
            pending.next();
        }
        p.pos = pos;
        pos += 1;
        let lane = p.row.lane.clone();
        let entries = lanes.entry(lane.clone()).or_default();
        merged.push(Slot::Sample {
            lane,
            index: entries.len(),
        });
        entries.push(p);
    }
    for (i, _) in pending {
        merged.push(Slot::Break(i));
    }
    ArrivalOrdering {
        lanes,
        breaks: breaks.to_vec(),
        merged,
    }
}

/// Order a window by HLC. Rows are ranked by `(ntp64, stamper, ingest
/// order)`: the stamper tie-break keeps the listing deterministic across
/// two clocks that happen to agree, and the ingest tie-break keeps it so
/// for one clock that stamped twice at once.
pub fn hlc(rows: &[TimelineRow]) -> HlcOrdering {
    let mut unstamped_excluded = 0usize;
    let mut placed: Vec<Placed<HlcAxis>> = Vec::with_capacity(rows.len());
    for (seq, row) in rows.iter().cloned().enumerate() {
        match Placed::<HlcAxis>::new(row) {
            Ok(p) => placed.push(Placed { seq, ..p }),
            Err(Unstamped) => unstamped_excluded += 1,
        }
    }
    placed.sort_by(|a, b| {
        let (sa, sb) = (a.stamp(), b.stamp());
        (sa.ntp64, &sa.stamper, a.seq).cmp(&(sb.ntp64, &sb.stamper, b.seq))
    });

    let stampers: BTreeSet<String> = placed.iter().map(|p| p.stamp().stamper.clone()).collect();
    let claim = match stampers.len() {
        0 => HlcClaim::NoStampedSamples,
        1 => HlcClaim::HappensBefore {
            stamper: stampers.into_iter().next().expect("one stamper"),
        },
        _ => HlcClaim::SkewedWallClock { stampers },
    };

    let mut lanes: BTreeMap<LaneId, Vec<Placed<HlcAxis>>> = BTreeMap::new();
    let mut merged = Vec::with_capacity(placed.len());
    for (pos, mut p) in placed.into_iter().enumerate() {
        p.pos = pos;
        let lane = p.row.lane.clone();
        let entries = lanes.entry(lane.clone()).or_default();
        merged.push(Slot::Sample {
            lane,
            index: entries.len(),
        });
        entries.push(p);
    }
    HlcOrdering {
        lanes,
        unstamped_excluded,
        claim,
        merged,
    }
}

/// A window and what was asked to get it — the input to [`timeline`].
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub rows: Vec<TimelineRow>,
    pub breaks: Vec<PlacedBreak>,
    /// The selectors watched (O5).
    pub scopes: Vec<String>,
    pub window_s: Option<f64>,
    pub source: TimelineSource,
    /// Keys the bounded statistics table retired during the window (O6).
    pub keys_evicted: u64,
}

/// The report for one window on one axis.
pub fn timeline(window: &Window, order: Order) -> TimelineReport {
    let order_by = order.label();
    let sn = sn_lane(&window.rows).report();
    let dropped = window
        .breaks
        .iter()
        .filter_map(|b| match b.kind {
            Break::Dropped(n) => Some(n),
            Break::Coalesced(_) => None,
        })
        .sum();
    let coalesced = window
        .breaks
        .iter()
        .filter_map(|b| match b.kind {
            Break::Coalesced(n) => Some(n),
            Break::Dropped(_) => None,
        })
        .sum();
    let (axis, lanes, unstamped_excluded, rows) = match order {
        Order::Arrival => {
            let o = arrival(&window.rows, &window.breaks);
            let lanes = summarise(&o.lanes, |p| &p.row);
            let rows = o
                .merged
                .iter()
                .enumerate()
                .map(|(pos, slot)| match slot {
                    Slot::Sample { lane, index } => {
                        entry(order_by, &o.lanes[lane][*index].row, pos)
                    }
                    Slot::Break(i) => {
                        let b = &o.breaks[*i];
                        TimelineEntry::Break {
                            order_by,
                            pos,
                            lane: b.lane.clone(),
                            kind: b.kind.kind(),
                            n: b.kind.n(),
                        }
                    }
                })
                .collect();
            (
                AxisLabel::Arrival {
                    clock: ARRIVAL_CLOCK,
                },
                lanes,
                0,
                rows,
            )
        }
        Order::Hlc => {
            let o = hlc(&window.rows);
            let lanes = summarise(&o.lanes, |p| &p.row);
            let rows = o
                .merged
                .iter()
                .enumerate()
                .map(|(pos, slot)| match slot {
                    Slot::Sample { lane, index } => {
                        entry(order_by, &o.lanes[lane][*index].row, pos)
                    }
                    Slot::Break(_) => unreachable!("an HLC ordering places no breaks"),
                })
                .collect();
            (
                AxisLabel::Hlc { claim: o.claim },
                lanes,
                o.unstamped_excluded,
                rows,
            )
        }
    };
    TimelineReport {
        order_by,
        axis,
        scopes: window.scopes.clone(),
        window_s: window.window_s,
        source: window.source.clone(),
        lanes,
        sn_lane: sn,
        unstamped_excluded,
        dropped,
        coalesced,
        keys_evicted: window.keys_evicted,
        rows,
    }
}

fn entry(order_by: OrderLabel, row: &TimelineRow, pos: usize) -> TimelineEntry {
    TimelineEntry::Sample {
        order_by,
        pos,
        lane: row.lane.clone(),
        key: row.key.clone(),
        t_us: row.t_us,
        hlc: row.hlc.as_ref().map(HlcStamp::to_wire),
        stamped_by: row.hlc.as_ref().map(|h| h.stamper.clone()),
        provenance: row.hlc.as_ref().map(|h| h.provenance),
        kind: row.kind,
    }
}

fn summarise<A>(
    lanes: &BTreeMap<LaneId, Vec<Placed<A>>>,
    row: impl Fn(&Placed<A>) -> &TimelineRow,
) -> Vec<LaneSummary> {
    lanes
        .iter()
        .map(|(lane, placed)| {
            let mut provenance = ProvenanceCounts::default();
            let mut stampers = BTreeSet::new();
            let (mut first, mut last) = (u64::MAX, 0u64);
            for p in placed {
                let r = row(p);
                first = first.min(r.t_us);
                last = last.max(r.t_us);
                if let Some(h) = &r.hlc {
                    stampers.insert(h.stamper.clone());
                    match h.provenance {
                        Provenance::SelfStamped => provenance.self_stamped += 1,
                        Provenance::Foreign => provenance.foreign += 1,
                        Provenance::Unattributable => provenance.unattributable += 1,
                    }
                }
            }
            LaneSummary {
                lane: lane.clone(),
                samples: placed.len(),
                first_t_us: if placed.is_empty() { 0 } else { first },
                last_t_us: last,
                stampers,
                provenance,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "acme";

    fn stamped(key: &str, t_us: u64, ntp64: u64, stamper: &str) -> TimelineRow {
        TimelineRow::assemble(
            key.into(),
            t_us,
            Some(HlcStamp {
                ntp64,
                stamper: stamper.into(),
                provenance: Provenance::Unattributable,
            }),
            None,
            None,
            RowKind::Put,
            3,
            BASE,
        )
    }

    fn unstamped(key: &str, t_us: u64) -> TimelineRow {
        TimelineRow::assemble(key.into(), t_us, None, None, None, RowKind::Put, 3, BASE)
    }

    fn keys(report: &TimelineReport) -> Vec<(&str, usize)> {
        report
            .rows
            .iter()
            .filter_map(|e| match e {
                TimelineEntry::Sample { key, pos, .. } => Some((key.as_str(), *pos)),
                TimelineEntry::Break { .. } => None,
            })
            .collect()
    }

    fn window(rows: Vec<TimelineRow>, breaks: Vec<PlacedBreak>) -> Window {
        Window {
            rows,
            breaks,
            scopes: vec!["acme/v1/**".into()],
            window_s: Some(1.0),
            source: TimelineSource::Live,
            keys_evicted: 0,
        }
    }

    const A: &str = "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/a";
    const B: &str = "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/b";

    /// The acceptance criterion: a synthetic reorder — A seen first, B
    /// stamped first — is visible as hlc-order ≠ arrival-order.
    #[test]
    fn a_reorder_is_visible_as_two_different_orders() {
        let w = window(
            vec![stamped(A, 10, 200, "33"), stamped(B, 20, 100, "33")],
            vec![],
        );
        let by_arrival = timeline(&w, Order::Arrival);
        let by_hlc = timeline(&w, Order::Hlc);
        assert_eq!(keys(&by_arrival), [(A, 0), (B, 1)]);
        assert_eq!(keys(&by_hlc), [(B, 0), (A, 1)]);
        assert_eq!(
            by_hlc.axis,
            AxisLabel::Hlc {
                claim: HlcClaim::HappensBefore {
                    stamper: "33".into()
                }
            }
        );
        assert_eq!(
            by_arrival.axis,
            AxisLabel::Arrival {
                clock: ARRIVAL_CLOCK
            }
        );
        // Both rows share one lane; the lane says who stamped it.
        assert_eq!(by_hlc.lanes.len(), 1);
        assert_eq!(by_hlc.lanes[0].stampers.len(), 1);
        assert_eq!(by_hlc.lanes[0].provenance.unattributable, 2);
    }

    /// Two stampers: the order is a comparison of wall clocks and says so.
    #[test]
    fn mixed_stampers_make_the_hlc_axis_a_skewed_wall_clock() {
        let w = window(
            vec![stamped(A, 10, 200, "33"), stamped(B, 20, 100, "44")],
            vec![],
        );
        let by_hlc = timeline(&w, Order::Hlc);
        assert_eq!(
            by_hlc.axis,
            AxisLabel::Hlc {
                claim: HlcClaim::SkewedWallClock {
                    stampers: ["33".to_string(), "44".to_string()].into_iter().collect()
                }
            }
        );
    }

    /// An unstamped row: its own lane on arrival, refused on HLC, counted.
    #[test]
    fn an_unstamped_row_has_an_arrival_lane_and_no_hlc_position() {
        let plain = unstamped("plain/key", 5);
        assert_eq!(plain.lane, LaneId::Unstamped);
        assert_eq!(Placed::<HlcAxis>::new(plain.clone()), Err(Unstamped));

        let w = window(vec![stamped(A, 10, 200, "33"), plain], vec![]);
        let by_arrival = timeline(&w, Order::Arrival);
        assert_eq!(by_arrival.unstamped_excluded, 0);
        assert!(
            by_arrival
                .lanes
                .iter()
                .any(|l| l.lane == LaneId::Unstamped && l.samples == 1)
        );
        let by_hlc = timeline(&w, Order::Hlc);
        assert_eq!(by_hlc.unstamped_excluded, 1);
        assert_eq!(keys(&by_hlc), [(A, 0)]);
        assert!(by_hlc.lanes.iter().all(|l| l.lane != LaneId::Unstamped));
    }

    /// A v1 key from an origin, unstamped, still goes to the unstamped
    /// lane — the lane is about the axis it can live on, not the key.
    #[test]
    fn an_unstamped_v1_key_is_in_the_unstamped_lane_not_its_origins() {
        let row = unstamped(A, 1);
        assert_eq!(row.lane, LaneId::Unstamped);
        // And a hand-built row cannot smuggle itself onto the HLC axis by
        // lying about its lane.
        let lying = TimelineRow {
            lane: LaneId::Unstamped,
            ..stamped(A, 1, 1, "33")
        };
        assert_eq!(Placed::<HlcAxis>::new(lying), Err(Unstamped));
    }

    /// Nothing under the base is a lane of its own, never a dropped row.
    #[test]
    fn a_key_outside_the_base_is_foreign_and_kept() {
        let row = stamped("other/v1/h-3fa9c2d41b7e/state/x/y", 1, 1, "33");
        assert_eq!(row.lane, LaneId::Foreign);
        let w = window(vec![row], vec![]);
        assert_eq!(timeline(&w, Order::Hlc).rows.len(), 1);
    }

    #[test]
    fn the_sn_lane_is_unavailable_never_empty_until_a_row_carries_one() {
        assert_eq!(
            sn_lane(&[unstamped(A, 1)]),
            SnLane::Unavailable {
                reason: SN_UNAVAILABLE_REASON
            }
        );
        let with_sn = TimelineRow {
            source: Some("zid:7".into()),
            sn: Some(41),
            ..unstamped(A, 1)
        };
        assert!(matches!(sn_lane(&[with_sn]), SnLane::Present { .. }));
    }

    /// Breaks keep their arrival position on the arrival axis and have
    /// none on the HLC axis — the total still rides the envelope.
    #[test]
    fn breaks_keep_position_on_arrival_and_are_totalled_on_hlc() {
        let w = window(
            vec![stamped(A, 10, 100, "33"), stamped(B, 20, 200, "33")],
            vec![PlacedBreak {
                after: 1,
                lane: None,
                kind: Break::Dropped(7),
            }],
        );
        let by_arrival = timeline(&w, Order::Arrival);
        let shape: Vec<&str> = by_arrival
            .rows
            .iter()
            .map(|e| match e {
                TimelineEntry::Sample { key, .. } => key.as_str(),
                TimelineEntry::Break { .. } => "<break>",
            })
            .collect();
        assert_eq!(shape, [A, "<break>", B]);
        assert!(matches!(
            by_arrival.rows[1],
            TimelineEntry::Break {
                pos: 1,
                kind: BreakKind::Dropped,
                n: 7,
                ..
            }
        ));
        assert_eq!(by_arrival.dropped, 7);
        let by_hlc = timeline(&w, Order::Hlc);
        assert_eq!(by_hlc.dropped, 7);
        assert!(
            by_hlc
                .rows
                .iter()
                .all(|e| matches!(e, TimelineEntry::Sample { .. }))
        );
    }

    #[test]
    fn a_zrec_drop_record_is_a_break_and_a_row_reads_its_stamp_back() {
        assert_eq!(
            Ingested::from_zrec(&ZrecItem::Dropped(3), BASE),
            Ingested::Break(Break::Dropped(3))
        );
        let item = ZrecItem::Sample {
            row: crate::tape::ingest::IngestRow {
                key: A.into(),
                payload: vec![1, 2, 3],
                encoding: None,
                qos: None,
                delete: false,
                attachment: None,
            },
            t_us: Some(42),
            timestamp: Some("100/33".into()),
            source: None,
        };
        let Ingested::Row(row) = Ingested::from_zrec(&item, BASE) else {
            panic!("a sample line is a row");
        };
        assert_eq!(row.t_us, 42);
        assert_eq!(
            row.hlc,
            Some(HlcStamp {
                ntp64: 100,
                stamper: "33".into(),
                provenance: Provenance::Unattributable
            })
        );
        assert_eq!(row.payload_bytes, 3);
        assert_eq!(
            row.lane,
            LaneId::Origin {
                origin: "h-3fa9c2d41b7e".into(),
                producer: Some("sysinfo".into())
            }
        );
    }
}
