//! The bench plane (#612, FJ8a): an operation called N times, each reply
//! timed and attributed by its key (spec §5.1 O3), the way the runtime's
//! `Fleet` attributes them.
//!
//! The report keeps its populations apart, because a benchmark that folds
//! them lies (the tooling guide's O6): a value reply's latency belongs to
//! the replier whose key carried it; an envelope is a refusal, a population
//! of its own and unattributed (a `reply_err` carries no key); a call that
//! drew no value and no envelope is silence, counted and never averaged in
//! (O5); a call that panicked inside this tool is news about the tool. v1's
//! `bench rpc` (`OriginLatency`, by `@rpc` origin) went with the `@rpc`
//! plane.

use std::collections::BTreeMap;

use serde::Serialize;

use super::operation::CallMode;

/// `zenctl bench call`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BenchReport {
    /// The address called, `<system>/<service>`; a `*` position is a
    /// fan-out's selection.
    pub address: String,
    /// `<name>.v<major>`.
    pub iface: String,
    /// The revision the calls were planned and their request encoded
    /// against.
    pub fingerprint: String,
    /// `@op/<template>`.
    pub operation: String,
    /// The template values given, unslugged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Vec<String>>,
    /// The key expressions each call went out on, base-relative.
    pub selectors: Vec<String>,
    pub mode: CallMode,
    /// Calls asked for.
    pub requested: usize,
    /// Calls that ran to their end (the query completed or timed out).
    pub completed: usize,
    /// Calls in flight at once.
    pub concurrency: usize,
    /// How long each call waited for replies, seconds.
    pub timeout_s: f64,
    pub elapsed_s: f64,
    pub calls_per_s: f64,
    /// Whose clock every latency here is on (the tooling guide's O7).
    pub clock: LatencyClock,
    /// Value replies, one population per key that carried them (O3), in
    /// order of first arrival.
    pub repliers: Vec<ReplierLatency>,
    /// Envelopes: their own population, unattributed (§5.1).
    pub refusals: RefusalTally,
    /// Replies claiming an envelope encoding that do not decode (§5.2).
    pub malformed: u64,
    /// The transport's own error replies, such as a query's timeout.
    pub transport: u64,
    /// Calls that drew no value and no envelope: silence, counted apart and
    /// never averaged in (O5).
    pub silent: u64,
    /// Value replies on a key that is not concrete (R6), or not a member of
    /// the operation called.
    pub discarded: u64,
    /// Calls that panicked inside this tool: they measured nothing.
    pub panicked: u64,
    /// The selection's interface tokens, read once before the first call:
    /// who could have answered.
    pub presence: BenchPresence,
}

/// The clock a latency is measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClock {
    /// This tool's monotonic clock: from the query sent to the reply
    /// received. No stamp, the replier's or a router's, enters it.
    RoundTrip,
}

/// A distribution of latencies, milliseconds, by nearest rank.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Latency {
    pub min_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// The value replies one key carried.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplierLatency {
    /// `<system>/<service>`, from the key.
    pub address: String,
    /// The operation key the replies went on, base-relative.
    pub key: String,
    pub replies: u64,
    #[serde(flatten)]
    pub latency: Latency,
}

/// The envelopes a bench drew (§5.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RefusalTally {
    pub count: u64,
    /// Envelopes by code.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub codes: BTreeMap<String, u64>,
    /// How long a refusal took to arrive; absent with none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency: Option<Latency>,
}

/// Who held the interface's token in the selection when the bench began.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchPresence {
    /// The liveliness selector read, base-relative.
    pub selector: String,
    /// Whether the read completed (§8.1). Possibly incomplete: a holder it
    /// missed is not listed.
    pub complete: bool,
    /// Why the read could not be made, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Each holder, and how many calls it sent no value in.
    pub holders: Vec<HolderTally>,
}

/// One token holder of the selection, against the calls it sent no value in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HolderTally {
    /// `<system>/<service>`.
    pub address: String,
    /// Calls this holder sent no value in: it refused or was silent, which
    /// a caller cannot tell apart (§5.1, "Attribution").
    pub without_value: u64,
}

impl BenchReport {
    /// 0 every measured reply a value, 1 an envelope or a malformed one
    /// among them (the latencies above are timings of failures), 2 no value
    /// at all: a benchmark that reached nobody measured nothing (O5).
    pub fn exit_code(&self) -> i32 {
        if self.repliers.is_empty() {
            2
        } else if self.refusals.count > 0 || self.malformed > 0 {
            1
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn latency(ms: f64) -> Latency {
        Latency {
            min_ms: ms,
            p50_ms: ms,
            p95_ms: ms,
            p99_ms: ms,
            max_ms: ms,
        }
    }

    pub(crate) fn report() -> BenchReport {
        BenchReport {
            address: "*/tc".into(),
            iface: "tc.netif.v1".into(),
            fingerprint: format!("sha256:{}", "ab".repeat(32)),
            operation: "@op/diagnostics".into(),
            values: BTreeMap::new(),
            selectors: vec!["zk2/*/tc/tc.netif.v1/@op/diagnostics".into()],
            mode: CallMode::Fanout,
            requested: 10,
            completed: 10,
            concurrency: 1,
            timeout_s: 2.0,
            elapsed_s: 1.0,
            calls_per_s: 10.0,
            clock: LatencyClock::RoundTrip,
            repliers: vec![ReplierLatency {
                address: "h1/tc".into(),
                key: "zk2/h1/tc/tc.netif.v1/@op/diagnostics".into(),
                replies: 10,
                latency: latency(1.5),
            }],
            refusals: RefusalTally {
                count: 3,
                codes: [("busy".to_owned(), 3)].into(),
                latency: Some(latency(0.5)),
            },
            malformed: 1,
            transport: 2,
            silent: 4,
            discarded: 5,
            panicked: 6,
            presence: BenchPresence {
                selector: "zk2/*/tc/@zk/alive/tc.netif.v1/**".into(),
                complete: true,
                error: None,
                holders: vec![HolderTally {
                    address: "h2/tc".into(),
                    without_value: 10,
                }],
            },
        }
    }

    /// Every population its own field, every count non-zero so no two can
    /// hide in one another; a replier's latency is flat beside its key.
    #[test]
    fn bench_report_json_shape_is_pinned() {
        assert_eq!(
            serde_json::to_value(report()).unwrap(),
            json!({
                "address": "*/tc",
                "iface": "tc.netif.v1",
                "fingerprint": format!("sha256:{}", "ab".repeat(32)),
                "operation": "@op/diagnostics",
                "selectors": ["zk2/*/tc/tc.netif.v1/@op/diagnostics"],
                "mode": "fanout",
                "requested": 10,
                "completed": 10,
                "concurrency": 1,
                "timeout_s": 2.0,
                "elapsed_s": 1.0,
                "calls_per_s": 10.0,
                "clock": "round_trip",
                "repliers": [{
                    "address": "h1/tc",
                    "key": "zk2/h1/tc/tc.netif.v1/@op/diagnostics",
                    "replies": 10,
                    "min_ms": 1.5, "p50_ms": 1.5, "p95_ms": 1.5, "p99_ms": 1.5, "max_ms": 1.5,
                }],
                "refusals": {
                    "count": 3,
                    "codes": {"busy": 3},
                    "latency": {"min_ms": 0.5, "p50_ms": 0.5, "p95_ms": 0.5, "p99_ms": 0.5,
                                "max_ms": 0.5},
                },
                "malformed": 1,
                "transport": 2,
                "silent": 4,
                "discarded": 5,
                "panicked": 6,
                "presence": {
                    "selector": "zk2/*/tc/@zk/alive/tc.netif.v1/**",
                    "complete": true,
                    "holders": [{"address": "h2/tc", "without_value": 10}],
                },
            })
        );
    }

    /// Silence is the 2 whatever else came back; an envelope among values
    /// is the 1; values alone the 0.
    #[test]
    fn the_exit_code_keeps_silence_apart() {
        let mut r = report();
        assert_eq!(r.exit_code(), 1);
        r.refusals.count = 0;
        r.malformed = 0;
        assert_eq!(
            r.exit_code(),
            0,
            "silent calls beside values are counted, not failed"
        );
        r.repliers.clear();
        r.refusals.count = 3;
        assert_eq!(r.exit_code(), 2);
    }
}
