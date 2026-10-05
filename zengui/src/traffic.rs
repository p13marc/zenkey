//! Where the traffic is (#542): the heaviest keys of the observed tree,
//! ranked, and each one's share of the whole.
//!
//! Pure, like [`crate::series`]: built from a [`KeyTreeSnapshot`] the monitor
//! already published, so it costs no lock and no subscription — and it is
//! rebuilt on a tick, only while the Traffic tab is showing, never on a
//! frame. Selection, not a full sort: a 50k-key tree is walked once and only
//! the kept rows are ordered.

use std::time::Instant;

use zenkey_fleet::KeyTreeSnapshot;
use zenkey_fleet::TreeNode;

/// How many rows the table keeps. The disclosure says "top N of M" whenever
/// more keys carried traffic than this.
pub const TOP: usize = 50;

/// What the table ranks by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrafficSort {
    #[default]
    Rate,
    Count,
    Bytes,
}

impl TrafficSort {
    pub const ALL: [TrafficSort; 3] = [TrafficSort::Rate, TrafficSort::Count, TrafficSort::Bytes];

    pub fn label(self) -> &'static str {
        match self {
            TrafficSort::Rate => "rate",
            TrafficSort::Count => "samples",
            TrafficSort::Bytes => "bytes",
        }
    }
}

/// One key that carried traffic of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct TrafficRow {
    pub key: String,
    pub count: u64,
    pub bytes: u64,
    pub rate_hz: f64,
    pub last_seen: Option<Instant>,
}

/// The ranked rows, and how many keys they were chosen from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrafficTable {
    pub rows: Vec<TrafficRow>,
    /// Keys that carried traffic of their own — the `M` of "top N of M".
    pub considered: usize,
    pub sort: TrafficSort,
}

/// The `limit` heaviest keys of `snapshot` by `sort`, heaviest first; ties
/// broken by key, so a frame never reshuffles equal rows.
pub fn top(snapshot: &KeyTreeSnapshot, sort: TrafficSort, limit: usize) -> TrafficTable {
    let mut rows = Vec::new();
    let mut path = Vec::new();
    collect(&snapshot.root, &mut path, &mut rows);
    let considered = rows.len();
    let order = |a: &TrafficRow, b: &TrafficRow| {
        let by = match sort {
            TrafficSort::Rate => b.rate_hz.total_cmp(&a.rate_hz),
            TrafficSort::Count => b.count.cmp(&a.count),
            TrafficSort::Bytes => b.bytes.cmp(&a.bytes),
        };
        by.then_with(|| a.key.cmp(&b.key))
    };
    if rows.len() > limit && limit > 0 {
        rows.select_nth_unstable_by(limit - 1, order);
        rows.truncate(limit);
    } else if limit == 0 {
        rows.clear();
    }
    rows.sort_by(order);
    TrafficTable {
        rows,
        considered,
        sort,
    }
}

fn collect<'a>(node: &'a TreeNode, path: &mut Vec<&'a str>, out: &mut Vec<TrafficRow>) {
    // A node with samples of its own is a key — interior or leaf, since a key
    // may also be the prefix of deeper ones.
    if node.count > 0 {
        out.push(TrafficRow {
            key: path.join("/"),
            count: node.count,
            bytes: node.bytes,
            rate_hz: node.rate_hz,
            last_seen: node.last_seen,
        });
    }
    for (chunk, child) in &node.children {
        path.push(chunk);
        collect(child, path, out);
        path.pop();
    }
}

/// The Traffic tab's own state (#542), held on the Activity dock: the sort,
/// the last ranking, and the whole session's rate over time.
#[derive(Default)]
pub struct TrafficState {
    pub sort: TrafficSort,
    /// `None` until the tab has been shown with a snapshot to rank — not an
    /// empty ranking.
    pub table: Option<TrafficTable>,
    /// The watched total's rate, one point per tick — a gap when no sample
    /// arrived, like every rate in this app (`RateSampler`'s rule).
    pub total_rate: crate::series::RateSampler,
    pub cache: iced::widget::canvas::Cache,
}

impl TrafficState {
    /// One tick's worth: the rate series always (O(1)), the ranking only
    /// when the tab is showing — a hidden tab walks no tree.
    pub fn tick(
        &mut self,
        totals: crate::message::WatchedTotals,
        snapshot: &KeyTreeSnapshot,
        showing: bool,
    ) {
        self.total_rate.tick(Some((totals.samples, totals.rate_hz)));
        self.cache.clear();
        if showing {
            self.rank(snapshot);
        }
    }

    /// Rank now — on a tick while showing, on the tab being opened, on the
    /// sort changing.
    pub fn rank(&mut self, snapshot: &KeyTreeSnapshot) {
        self.table = Some(top(snapshot, self.sort, TOP));
    }
}

/// `part`'s share of `total`, `0.0..=1.0` — or `None` when the total is zero:
/// a share of nothing is not 0%, it is no share at all.
pub fn share(part: f64, total: f64) -> Option<f32> {
    (total > 0.0).then(|| (part / total).clamp(0.0, 1.0) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::model::stats::StatsTable;

    fn snapshot(rows: &[(&str, usize)]) -> KeyTreeSnapshot {
        let mut stats = StatsTable::new();
        let t0 = Instant::now();
        for (key, n) in rows {
            for i in 0..*n {
                stats.record(
                    key,
                    10,
                    None,
                    t0 + std::time::Duration::from_millis(100 * i as u64),
                    None,
                    None,
                );
            }
        }
        KeyTreeSnapshot::build(&stats)
    }

    #[test]
    fn the_heaviest_keys_come_first_and_the_rest_are_counted() {
        let snap = snapshot(&[("a/b", 3), ("a/c", 9), ("d", 5), ("a/b/deeper", 1)]);
        let t = top(&snap, TrafficSort::Count, 2);
        assert_eq!(
            t.rows.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
            ["a/c", "d"]
        );
        assert_eq!(t.considered, 4, "a key that is also a prefix still counts");
        let all = top(&snap, TrafficSort::Bytes, TOP);
        assert_eq!(all.rows.len(), 4);
        assert_eq!(all.rows[0].bytes, 90);
    }

    #[test]
    fn a_share_of_nothing_is_no_share() {
        assert_eq!(share(1.0, 0.0), None);
        assert_eq!(share(0.0, 4.0), Some(0.0));
        assert_eq!(share(1.0, 4.0), Some(0.25));
    }
}
