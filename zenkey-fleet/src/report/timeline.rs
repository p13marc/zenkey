//! The fleet timeline (#216): one merged ordering of a window's samples,
//! partitioned into lanes, with the clock it was ordered on stated **per
//! report** and the stamper stated **per lane**.
//!
//! Every row a script reads carries `order_by`, so a line cut out of an
//! ndjson stream still says which axis its `pos` is a position on. The
//! shapes here are what [`crate::model::timeline`] computes and what
//! `zenctl timeline` emits; the reasoning about *why* the two axes never
//! mix lives with the computation.

use std::collections::BTreeSet;

use serde::Serialize;

/// Which clock a report was ordered on — the envelope's `order_by`, and
/// every row's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderLabel {
    /// The observer's monotonic clock, µs since the window epoch. Every
    /// sample has one.
    Arrival,
    /// The sample's HLC. Only stamped samples have one; the rest are
    /// excluded and counted, never defaulted.
    Hlc,
}

/// What ordering on the HLC axis can claim (RFC 09 §5.1 O7).
///
/// zenoh updates a node's HLC on receive only when the node *has* one
/// (`treat_timestamp!` in `net/routing/dispatcher/pubsub.rs`), and
/// `net/runtime/mod.rs` builds one only where `timestamping.enabled` is
/// true for the node's whatami — routers default true, peers and clients
/// false. So one stamper's values are that node's monotonic clock, ordered
/// by what it forwarded; two stampers' values are two wall clocks unless
/// both passed through a common timestamping node, which an observer
/// cannot see.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "claim", rename_all = "snake_case")]
pub enum HlcClaim {
    /// Every stamped sample in the window was stamped by one node: the
    /// order is that node's happened-before.
    HappensBefore { stamper: String },
    /// More than one stamper: the order compares clocks that were never
    /// synchronised by anything this observer can vouch for.
    SkewedWallClock { stampers: BTreeSet<String> },
    /// No sample in the window carried an HLC: the axis is empty, and an
    /// empty axis claims nothing (O4).
    NoStampedSamples,
}

/// Which axis the report is ordered on, and what that axis is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "axis", rename_all = "snake_case")]
pub enum AxisLabel {
    Arrival {
        /// The one spelling of the arrival clock.
        clock: &'static str,
    },
    Hlc {
        #[serde(flatten)]
        claim: HlcClaim,
    },
}

/// The arrival clock, spelled once.
pub const ARRIVAL_CLOCK: &str = "observer monotonic, µs since window start";

/// Who a lane belongs to.
///
/// Lanes are keyed by origin and producer because that is the unit a
/// fleet publishes as (RFC 03 §1). The two remaining variants are the two
/// ways a sample can fail to belong: its key says nothing under this base,
/// or it carries no HLC and therefore exists on the arrival axis only.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaneId {
    /// A conforming `v1/<origin>/<class>/<producer>/…` key. `producer` is
    /// absent under a service origin and under `@blob` (RFC 03 §1.5).
    Origin {
        origin: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        producer: Option<String>,
    },
    /// The key does not parse as a v1 key under the base — not under it,
    /// or under it and something else. Kept, never dropped (O1).
    Foreign,
    /// No HLC rode the sample. This lane has a place on the arrival axis
    /// and **no place on the HLC axis** — the type system in
    /// [`crate::model::timeline`] enforces that, not a warning.
    Unstamped,
}

impl LaneId {
    /// The lane as a human label — the table's group heading.
    pub fn label(&self) -> String {
        match self {
            LaneId::Origin {
                origin,
                producer: Some(p),
            } => format!("{origin}/{p}"),
            LaneId::Origin {
                origin,
                producer: None,
            } => origin.clone(),
            LaneId::Foreign => "foreign (not a v1 key under this base)".into(),
            LaneId::Unstamped => "unstamped (arrival axis only)".into(),
        }
    }
}

/// Who stamped a sample, as the wire spells it — the serialized form of
/// [`crate::StampProvenance`], stamper identity carried beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    SelfStamped,
    Foreign,
    Unattributable,
}

/// How many of a lane's stamped samples fell in each provenance class
/// (#213): three populations, never one number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ProvenanceCounts {
    pub self_stamped: usize,
    pub foreign: usize,
    pub unattributable: usize,
}

/// One lane's account of itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaneSummary {
    pub lane: LaneId,
    /// Samples placed on this report's axis in this lane.
    pub samples: usize,
    /// Arrival offset of the lane's first and last placed sample, µs.
    pub first_t_us: u64,
    pub last_t_us: u64,
    /// Every stamper seen in the lane — one is the happened-before case,
    /// more is the skew case, none is the unstamped lane.
    pub stampers: BTreeSet<String>,
    pub provenance: ProvenanceCounts,
}

/// The per-`SampleSource` sequence-number lane.
///
/// `Unavailable` is a structural state, never an empty vector: zenoh 1.9
/// and 1.10 deliver no `SourceInfo` to a subscriber, so an empty lane would
/// read as "nobody skipped a number" when the truth is "nobody numbered
/// anything we could see" (O4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SnLaneReport {
    Unavailable { reason: &'static str },
    Present { sources: usize, samples: usize },
}

/// The fixed reason the sequence-number lane is unavailable on this zenoh.
pub const SN_UNAVAILABLE_REASON: &str = "zenoh 1.9/1.10 deliver no SourceInfo to subscribers \
     (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it";

/// Where the window came from — the same projection runs on both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TimelineSource {
    Live,
    Zrec { path: String },
}

/// Put or delete — a tombstone is a fact of its own (RFC 04 §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    Put,
    Delete,
}

/// What kind of break interrupted the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    /// Samples the bounded observer missed while behind (O6).
    Dropped,
    /// Distinct samples a consumer merged into fewer.
    Coalesced,
}

/// One line of the merged ordering — **every** variant carries `order_by`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum TimelineEntry {
    Sample {
        order_by: OrderLabel,
        /// Position in the merged ordering on `order_by`'s axis, 0-based.
        pos: usize,
        lane: LaneId,
        key: String,
        /// Arrival, µs since the window epoch — carried on both axes, so a
        /// reorder is visible on the HLC listing as a non-monotonic `t_us`.
        t_us: u64,
        /// The HLC as `<ntp64>/<stamper>` — the `.zrec` spelling, so it
        /// round-trips (`uhlc::Timestamp: FromStr`).
        #[serde(skip_serializing_if = "Option::is_none")]
        hlc: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stamped_by: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        provenance: Option<Provenance>,
        kind: RowKind,
    },
    Break {
        order_by: OrderLabel,
        pos: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        lane: Option<LaneId>,
        kind: BreakKind,
        n: u64,
    },
}

/// The report `zenctl timeline` emits and a pane renders.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimelineReport {
    pub order_by: OrderLabel,
    #[serde(flatten)]
    pub axis: AxisLabel,
    /// The selectors watched — coverage is exactly this list (O5).
    pub scopes: Vec<String>,
    /// The passive window, seconds. `None` for a `.zrec`: the file's span
    /// is in its rows, and no window was asked for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    pub source: TimelineSource,
    pub lanes: Vec<LaneSummary>,
    pub sn_lane: SnLaneReport,
    /// Samples that carried no HLC and were therefore **not placed** on the
    /// HLC axis. Always 0 on the arrival axis, where they have a lane.
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub unstamped_excluded: usize,
    /// Samples the observer missed while behind, totalled (O6).
    pub dropped: u64,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub coalesced: u64,
    /// Keys the bounded statistics table retired during the window (O6).
    pub keys_evicted: u64,
    pub rows: Vec<TimelineEntry>,
}

fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lane() -> LaneId {
        LaneId::Origin {
            origin: "h-3fa9c2d41b7e".into(),
            producer: Some("sysinfo".into()),
        }
    }

    /// The happened-before claim, flattened into the envelope beside
    /// `order_by`; the per-row `order_by` on both row kinds.
    #[test]
    fn a_happens_before_report_is_pinned() {
        let report = TimelineReport {
            order_by: OrderLabel::Hlc,
            axis: AxisLabel::Hlc {
                claim: HlcClaim::HappensBefore {
                    stamper: "33".into(),
                },
            },
            scopes: vec!["v1/**".into()],
            window_s: Some(10.0),
            source: TimelineSource::Live,
            lanes: vec![LaneSummary {
                lane: lane(),
                samples: 1,
                first_t_us: 5,
                last_t_us: 5,
                stampers: ["33".to_string()].into_iter().collect(),
                provenance: ProvenanceCounts {
                    unattributable: 1,
                    ..Default::default()
                },
            }],
            sn_lane: SnLaneReport::Unavailable {
                reason: SN_UNAVAILABLE_REASON,
            },
            unstamped_excluded: 2,
            dropped: 0,
            coalesced: 0,
            keys_evicted: 0,
            rows: vec![TimelineEntry::Sample {
                order_by: OrderLabel::Hlc,
                pos: 0,
                lane: lane(),
                key: "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu".into(),
                t_us: 5,
                hlc: Some("100/33".into()),
                stamped_by: Some("33".into()),
                provenance: Some(Provenance::Unattributable),
                kind: RowKind::Put,
            }],
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "order_by": "hlc",
                "axis": "hlc",
                "claim": "happens_before",
                "stamper": "33",
                "scopes": ["v1/**"],
                "window_s": 10.0,
                "source": {"kind": "live"},
                "lanes": [{
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "samples": 1,
                    "first_t_us": 5,
                    "last_t_us": 5,
                    "stampers": ["33"],
                    "provenance": {"self_stamped": 0, "foreign": 0, "unattributable": 1}
                }],
                "sn_lane": {
                    "state": "unavailable",
                    "reason": "zenoh 1.9/1.10 deliver no SourceInfo to subscribers (eclipse-zenoh/zenoh#2563); `tests/stamper.rs` pins it"
                },
                "unstamped_excluded": 2,
                "dropped": 0,
                "keys_evicted": 0,
                "rows": [{
                    "row": "sample",
                    "order_by": "hlc",
                    "pos": 0,
                    "lane": {"kind": "origin", "origin": "h-3fa9c2d41b7e", "producer": "sysinfo"},
                    "key": "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
                    "t_us": 5,
                    "hlc": "100/33",
                    "stamped_by": "33",
                    "provenance": "unattributable",
                    "kind": "put"
                }]
            })
        );
    }

    /// The skew claim, the arrival axis, the unstamped lane, a break row,
    /// and the zero fields that vanish.
    #[test]
    fn an_arrival_report_with_a_break_is_pinned_and_the_skew_claim_spells_its_stampers() {
        let report = TimelineReport {
            order_by: OrderLabel::Arrival,
            axis: AxisLabel::Arrival {
                clock: ARRIVAL_CLOCK,
            },
            scopes: vec!["v1/**".into()],
            window_s: None,
            source: TimelineSource::Zrec {
                path: "bus.zrec".into(),
            },
            lanes: vec![],
            sn_lane: SnLaneReport::Present {
                sources: 1,
                samples: 3,
            },
            unstamped_excluded: 0,
            dropped: 7,
            coalesced: 0,
            keys_evicted: 0,
            rows: vec![
                TimelineEntry::Sample {
                    order_by: OrderLabel::Arrival,
                    pos: 0,
                    lane: LaneId::Unstamped,
                    key: "plain/key".into(),
                    t_us: 1,
                    hlc: None,
                    stamped_by: None,
                    provenance: None,
                    kind: RowKind::Delete,
                },
                TimelineEntry::Break {
                    order_by: OrderLabel::Arrival,
                    pos: 1,
                    lane: None,
                    kind: BreakKind::Dropped,
                    n: 7,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "order_by": "arrival",
                "axis": "arrival",
                "clock": "observer monotonic, µs since window start",
                "scopes": ["v1/**"],
                "source": {"kind": "zrec", "path": "bus.zrec"},
                "lanes": [],
                "sn_lane": {"state": "present", "sources": 1, "samples": 3},
                "dropped": 7,
                "keys_evicted": 0,
                "rows": [
                    {
                        "row": "sample",
                        "order_by": "arrival",
                        "pos": 0,
                        "lane": {"kind": "unstamped"},
                        "key": "plain/key",
                        "t_us": 1,
                        "kind": "delete"
                    },
                    {"row": "break", "order_by": "arrival", "pos": 1, "kind": "dropped", "n": 7}
                ]
            })
        );
        let skew = AxisLabel::Hlc {
            claim: HlcClaim::SkewedWallClock {
                stampers: ["33".to_string(), "44".to_string()].into_iter().collect(),
            },
        };
        assert_eq!(
            serde_json::to_value(&skew).unwrap(),
            json!({"axis": "hlc", "claim": "skewed_wall_clock", "stampers": ["33", "44"]})
        );
        let empty = AxisLabel::Hlc {
            claim: HlcClaim::NoStampedSamples,
        };
        assert_eq!(
            serde_json::to_value(&empty).unwrap(),
            json!({"axis": "hlc", "claim": "no_stamped_samples"})
        );
    }
}
