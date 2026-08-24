//! `bench rpc` (issue #52) — how fast does the fleet answer, and which
//! origin is slow.
//!
//! Two design decisions worth stating, because both are refusals:
//!
//! **Latency is per reply, not per call.** A fan-out GET finishes when the
//! *slowest* origin answers, so attributing the call's duration to every
//! responder would report the fastest node's latency as the worst one's. The
//! measurement therefore rides
//! [`RepeatingQuery::fetch_timed`](crate::bus::query::RepeatingQuery::fetch_timed),
//! which stamps each reply where it is drained — inside the RFC 05 §2.1
//! chokepoint, not around it.
//!
//! **Benching writes is refused by default.** The registry declares
//! `idempotent`, and a benchmark is by definition N repetitions: repeating a
//! non-idempotent write into a live fleet is a different act from measuring
//! it. The refusal is registry-driven, so it is only as good as the
//! declaration — which is why a producer that declares *nothing* is also
//! refused rather than assumed safe (O4: "not declared" is not "declared
//! idempotent").

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

use crate::bus::query::{Answer, RepeatingQuery, declare_repeating};
use crate::bus::write::CallTarget;
use crate::model::registry::SliceSet;
use crate::report::{BenchReport, OriginLatency};

/// What to measure.
pub struct BenchSpec<'a> {
    pub target: &'a CallTarget,
    pub producer: &'a str,
    pub procedure: &'a str,
    /// Total calls to issue.
    pub count: usize,
    /// How many may be in flight at once. 1 = strictly sequential.
    pub concurrency: usize,
    pub timeout: Duration,
    /// Proceed even when the registry does not declare the procedure
    /// idempotent. The caller must have meant it.
    pub force: bool,
}

/// The procedures **this convention** defines, rather than an application:
/// `introspect` (RFC 08 §6) and `describe` (RFC 08 §7). Both are reads that
/// return a document, both are MUST/SHOULD for every producer, and neither is
/// an application's to declare differently — so their idempotence is a fact
/// about the convention, not something to look up in a registry that may not
/// bother listing them.
const FRAMEWORK_READS: [&str; 2] = ["introspect", "describe"];

/// Refuse a benchmark that would repeat a non-idempotent call.
///
/// With no slices loaded the registry layer cannot judge — and unlike the
/// fan-out guard, which has builder and ACL layers behind it, there is nothing
/// behind this one. So it refuses rather than proceeding, and says how to
/// override.
fn check_idempotent(slices: Option<&SliceSet>, producer: &str, procedure: &str) -> Result<()> {
    if FRAMEWORK_READS.contains(&procedure) {
        return Ok(());
    }
    let Some(slices) = slices else {
        bail!(
            "no registry loaded, so {producer}/{procedure}'s idempotence is unknown — a \
             benchmark repeats a call N times, and \"not asked\" is not \"safe to repeat\" \
             (RFC 09 §5.1 O4). Load a registry, or pass --i-know."
        );
    };
    let decl = slices
        .get(producer)
        .and_then(|s| s.procedures.iter().find(|p| p.path == procedure));
    match decl {
        Some(d) if d.idempotent == Some(true) => Ok(()),
        Some(d) => bail!(
            "{producer}/{procedure} declares kind = {:?}, idempotent = {} — repeating it is a \
             write into a live fleet, not a measurement. Pass --i-know to mean it.",
            d.kind,
            match d.idempotent {
                Some(false) => "false",
                _ => "(undeclared)",
            }
        ),
        None => bail!(
            "the loaded registry does not declare {producer}/{procedure}, so nothing says it \
             is safe to repeat. Pass --i-know to bench it anyway."
        ),
    }
}

/// The four populations a benchmark keeps apart, and the one place a joined
/// call is sorted into them.
///
/// Keeping them apart is the whole honesty claim of this report (RFC 13 §3 O6,
/// RFC 05 §3.1): an error reply is the fleet refusing, silence is the fleet not
/// answering, and a **panicked** call is this tool falling over — three
/// different facts that a single "failed" counter would flatten into a lie.
/// The fold lives here rather than inline so the fourth one can be tested
/// against a real `JoinError` (#329), which is what the loop above cannot
/// manufacture.
#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    completed: usize,
    errors: usize,
    silent: usize,
    panicked: usize,
}

impl Tally {
    /// Fold one joined call in, routing its per-reply latencies to their
    /// origins.
    fn record(
        &mut self,
        joined: std::result::Result<
            Result<Vec<(crate::bus::query::FleetAnswer, Duration)>>,
            tokio::task::JoinError,
        >,
        per_origin: &mut BTreeMap<String, Vec<Duration>>,
    ) {
        // A panicked call reached no ledger at all before #329: the
        // `let Ok(result) = handle.await else { continue }` that stood here
        // skipped `completed`, `errors` and `silent` in one line.
        let Ok(result) = joined else {
            self.panicked += 1;
            return;
        };
        let Ok(answers) = result else {
            self.errors += 1;
            return;
        };
        self.completed += 1;
        if answers.is_empty() {
            // RFC 05 §3.1: zero replies is its own outcome, counted apart
            // from an error so a benchmark cannot average silence away.
            self.silent += 1;
            return;
        }
        for (answer, at) in answers {
            match answer.answer {
                Answer::Value(_) => per_origin.entry(answer.origin).or_default().push(at),
                Answer::Error { .. } => self.errors += 1,
            }
        }
    }
}

/// Percentile by nearest-rank over a sorted slice. Reported in milliseconds.
fn percentile(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx].as_secs_f64() * 1000.0
}

/// Run the benchmark.
pub async fn run_bench(
    fleet: &crate::Fleet<'_>,
    spec: BenchSpec<'_>,
    slices: Option<&SliceSet>,
) -> Result<BenchReport> {
    if !spec.force {
        check_idempotent(slices, spec.producer, spec.procedure)?;
    }
    if spec.count == 0 {
        bail!("--calls 0 measures nothing");
    }

    let segments: Vec<&str> = spec.procedure.split('/').collect();
    let relative = match spec.target {
        CallTarget::Host(id) => {
            let origin = zenkey::origin::RemoteOrigin::from_host(id.clone());
            zenkey::selector::rpc_at(&origin, spec.producer, &segments).to_string()
        }
        CallTarget::Fleet => zenkey::selector::fleet_rpc(spec.producer, &segments).to_string(),
        CallTarget::Service(origin) => zenkey::selector::service_rpc(origin, &segments).to_string(),
    };
    let key = fleet.wire(relative);

    // One declared querier for the whole run (#37): re-declaring per call
    // would measure zenoh's declaration path rather than the fleet's answers.
    let querier = std::sync::Arc::new(
        declare_repeating(fleet, &key, spec.timeout)
            .await
            .map_err(|e| anyhow!("declare querier {key}: {e}"))?,
    );

    let concurrency = spec.concurrency.max(1).min(spec.count);
    let started = Instant::now();
    let mut per_origin: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
    let mut tally = Tally::default();

    let mut issued = 0usize;
    while issued < spec.count {
        let batch = concurrency.min(spec.count - issued);
        let mut set = Vec::with_capacity(batch);
        for _ in 0..batch {
            let q: std::sync::Arc<RepeatingQuery> = querier.clone();
            set.push(tokio::spawn(async move { q.fetch_timed().await }));
        }
        issued += batch;
        for handle in set {
            tally.record(handle.await, &mut per_origin);
        }
    }
    let Tally {
        completed,
        errors,
        silent,
        panicked,
    } = tally;
    let elapsed = started.elapsed();
    std::sync::Arc::try_unwrap(querier)
        .map_err(|_| anyhow!("bench tasks outlived the run"))?
        .undeclare()
        .await?;

    let origins = per_origin
        .into_iter()
        .map(|(origin, mut samples)| {
            samples.sort_unstable();
            OriginLatency {
                origin,
                replies: samples.len(),
                min_ms: samples[0].as_secs_f64() * 1000.0,
                p50_ms: percentile(&samples, 50.0),
                p95_ms: percentile(&samples, 95.0),
                p99_ms: percentile(&samples, 99.0),
                max_ms: samples[samples.len() - 1].as_secs_f64() * 1000.0,
            }
        })
        .collect();

    Ok(BenchReport {
        key,
        requested: spec.count,
        completed,
        concurrency,
        errors,
        silent,
        panicked,
        elapsed_s: elapsed.as_secs_f64(),
        calls_per_s: if elapsed.as_secs_f64() > 0.0 {
            completed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        },
        origins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::slice::{ProcedureDecl, RegistrySlice};

    fn slices(kind: &str, idempotent: Option<bool>) -> SliceSet {
        SliceSet::from_slices(vec![RegistrySlice {
            version: "1.0".into(),
            app: "t".into(),
            convention: 1,
            name: "netring".into(),
            service_origin: None,
            description: None,
            subjects: vec![],
            procedures: vec![ProcedureDecl {
                path: "capture/trigger".into(),
                kind: kind.into(),
                reply: Some("Ack".into()),
                request: None,
                encoding: None,
                fanout: None,
                idempotent,
                cardinality: None,
                since: None,
                description: None,
            }],
            blob: vec![],
            media: vec![],
            deprecated: vec![],
        }])
    }

    /// The guard: only an explicit `idempotent = true` passes. "Undeclared"
    /// and "not in the registry at all" both refuse — a benchmark repeats,
    /// and O4 forbids reading an unasked question as a yes.
    #[test]
    fn only_a_declared_idempotent_procedure_benches_by_default() {
        let ok = slices("read", Some(true));
        assert!(check_idempotent(Some(&ok), "netring", "capture/trigger").is_ok());

        for (kind, idem) in [("write", Some(false)), ("read", None)] {
            let s = slices(kind, idem);
            let err = check_idempotent(Some(&s), "netring", "capture/trigger")
                .unwrap_err()
                .to_string();
            assert!(err.contains("--i-know"), "{err}");
        }

        // Unknown procedure, and no registry at all.
        let s = slices("read", Some(true));
        assert!(check_idempotent(Some(&s), "netring", "other").is_err());
        let err = check_idempotent(None, "netring", "capture/trigger")
            .unwrap_err()
            .to_string();
        assert!(err.contains("O4"), "{err}");
    }

    /// The convention's own reads bench without a registry entry: RFC 08 §6
    /// makes `introspect` a MUST for every producer and §7 makes `describe` a
    /// SHOULD, so their idempotence is not an application's to declare — and
    /// requiring a slice to restate it would refuse the one call the tool
    /// already fans out on by design.
    #[test]
    fn the_conventions_own_reads_need_no_registry_permission() {
        for p in ["introspect", "describe"] {
            assert!(check_idempotent(None, "anything", p).is_ok(), "{p}");
        }
        // …and nothing else gets the exemption by resembling them.
        assert!(check_idempotent(None, "anything", "introspect/all").is_err());
    }

    /// The four populations, each landing in exactly one ledger — and a
    /// panicked task landing in the fourth rather than in none (#329). The
    /// `JoinError` is a real one: nothing else produces the value the loop
    /// used to throw away.
    #[tokio::test]
    async fn a_panicked_call_is_its_own_population_and_reaches_a_ledger() {
        let mut per_origin: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
        let mut tally = Tally::default();

        let join_error = tokio::spawn(async { panic!("a call fell over") })
            .await
            .expect_err("the task panicked");
        tally.record(Err(join_error), &mut per_origin);
        assert_eq!(
            tally,
            Tally {
                completed: 0,
                errors: 0,
                silent: 0,
                panicked: 1,
            },
            "the panic reaches its own ledger and no other"
        );

        // The three it must not be confused with.
        tally.record(Ok(Err(anyhow!("the GET failed"))), &mut per_origin);
        tally.record(Ok(Ok(vec![])), &mut per_origin);
        tally.record(
            Ok(Ok(vec![(
                crate::bus::query::FleetAnswer {
                    origin: "h-3fa9c2d41b7e".into(),
                    key: "v1/h-3fa9c2d41b7e/@rpc/netring/capture/trigger".into(),
                    encoding: None,
                    attachment: None,
                    answer: Answer::Value(zenoh::bytes::ZBytes::from(b"{}".to_vec())),
                },
                Duration::from_millis(3),
            )])),
            &mut per_origin,
        );
        assert_eq!(
            tally,
            Tally {
                completed: 2,
                errors: 1,
                silent: 1,
                panicked: 1,
            }
        );
        assert_eq!(per_origin["h-3fa9c2d41b7e"], vec![Duration::from_millis(3)]);
    }

    #[test]
    fn percentiles_are_nearest_rank_and_survive_one_sample() {
        let d = |ms: u64| Duration::from_millis(ms);
        let one = [d(7)];
        assert_eq!(percentile(&one, 50.0), 7.0);
        assert_eq!(percentile(&one, 99.0), 7.0);

        let ten: Vec<Duration> = (1..=10).map(d).collect();
        assert_eq!(percentile(&ten, 50.0), 5.0);
        assert_eq!(percentile(&ten, 95.0), 10.0);
        assert_eq!(percentile(&ten, 100.0), 10.0);
        // Empty is 0, not a panic — a bench with no replies still reports.
        assert_eq!(percentile(&[], 50.0), 0.0);
    }
}
