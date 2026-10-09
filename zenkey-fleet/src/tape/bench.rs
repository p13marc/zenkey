//! `bench call` (#612, FJ8a): how fast a zk2 operation answers, and which
//! replier is slow.
//!
//! **Latency is per reply.** Each reply is timed where it arrives, in the
//! query's own callback, from the moment the call went out: a fan-out
//! completes when its slowest replier has, and charging that to every
//! replier would report the fastest one's latency as the worst one's. The
//! calls go out as the runtime's do — one concrete key with `BestMatching`
//! and `None` (O1), a fan-out with `All` and `None` (O2), the operation's
//! recommended priority — on the keys the runtime builds; each value reply
//! is attributed by the key it went on (O3), as `Fleet` attributes them.
//!
//! **The populations stay apart** (the tooling guide's O6): values per
//! replier, envelopes as a refusal population of their own (unattributed:
//! a `reply_err` carries no key, §5.1), malformed envelopes, the
//! transport's own errors, calls that drew nothing at all (silence, never
//! averaged in: O5), replies R6 discards, and calls that panicked here.
//!
//! **A benchmark repeats a call**, so an operation that is not `idempotent`
//! is refused unless the caller means it (O4's reasoning: repeating a
//! write is a different act from measuring it). A fan-out to an operation
//! that forbids one was refused by the plan (O2), and nothing moves that.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use zenkey_model::grammar::{KindToken, ZkKey, parse};
use zenoh::Session;
use zenoh::bytes::ZBytes;
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};

use crate::model::catalog::Revision;
use crate::model::target::CallPlan;
use crate::report::{
    BenchPresence, BenchReport, CallMode, HolderTally, Latency, LatencyClock, RefusalTally,
    ReplierLatency,
};
use crate::{Error, Result};

/// What to measure.
pub struct BenchSpec<'a> {
    pub revision: &'a Revision,
    pub plan: &'a CallPlan,
    /// The request, encoded as the operation's `request` type.
    pub request: Vec<u8>,
    /// Calls to make.
    pub calls: usize,
    /// Calls in flight at once; 1 is strictly sequential.
    pub concurrency: usize,
    /// How long each call waits for replies.
    pub timeout: Duration,
    /// Bench an operation that is not `idempotent`: the caller means a
    /// repeated write.
    pub force: bool,
}

/// One reply, timed from the call's start.
type Timed = (Duration, Reply);

/// What one call drew.
#[derive(Debug, Default)]
struct CallOutcome {
    /// Value replies: `(address, key, latency)`.
    values: Vec<(String, String, Duration)>,
    /// Envelopes: `(code, latency)`.
    refusals: Vec<(String, Duration)>,
    malformed: u64,
    transport: u64,
    discarded: u64,
}

/// Sorts one call's replies into their populations: a value reply by the
/// key it went on, when that key is concrete and a member of the operation
/// called (O3, R6); an error reply by its `Encoding` (§5.2).
fn classify(revision: &Revision, plan: &CallPlan, replies: Vec<Timed>) -> CallOutcome {
    let mut out = CallOutcome::default();
    for (at, reply) in replies {
        match reply.into_result() {
            Ok(sample) => {
                let key = sample.key_expr().as_str();
                match member_of(revision, plan, key) {
                    Some(addr) => out.values.push((addr, key.to_owned(), at)),
                    None => out.discarded += 1,
                }
            }
            Err(e) => match zk2::client::classify(&e) {
                zk2::client::ErrorReply::Envelope(env) => out.refusals.push((env.code, at)),
                zk2::client::ErrorReply::Malformed(_) => out.malformed += 1,
                zk2::client::ErrorReply::Transport(_) => out.transport += 1,
            },
        }
    }
    out
}

/// The address a value reply's key names, when it is a concrete member key
/// of the operation called.
fn member_of(revision: &Revision, plan: &CallPlan, key: &str) -> Option<String> {
    if key.contains('*') {
        return None;
    }
    match parse(key).ok()? {
        ZkKey::Data {
            addr,
            iface,
            kind: KindToken::Op,
            resource,
        } if &iface == revision.iface() => {
            let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
            plan.resource.template.matches(&refs)?;
            Some(addr.to_string())
        }
        _ => None,
    }
}

/// Percentile by nearest rank over a sorted slice, in milliseconds.
fn percentile(sorted: &[Duration], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx].as_secs_f64() * 1000.0
}

/// A distribution, or `None` for no sample.
fn latency(mut samples: Vec<Duration>) -> Option<Latency> {
    samples.sort_unstable();
    let (first, last) = (*samples.first()?, *samples.last()?);
    Some(Latency {
        min_ms: first.as_secs_f64() * 1000.0,
        p50_ms: percentile(&samples, 50.0),
        p95_ms: percentile(&samples, 95.0),
        p99_ms: percentile(&samples, 99.0),
        max_ms: last.as_secs_f64() * 1000.0,
    })
}

/// The key expressions a call goes out on, as the runtime builds them: the
/// one concrete key (`Client::key`), or one selector per selected provider
/// (`Fleet::selectors`).
fn selectors(session: &Session, revision: &Revision, plan: &CallPlan) -> Result<Vec<OwnedKeyExpr>> {
    let contract = revision.shared_contract();
    let address = plan.target.address.as_str();
    let refuse = |e: zk2::Error| Error::unaskable(address, e.to_string());
    match &plan.target.concrete {
        Some(addr) if !plan.is_fanout() => {
            let client = zk2::Client::new(session, contract, &[address]).map_err(refuse)?;
            Ok(vec![
                client
                    .key(addr, &plan.name, &plan.values)
                    .map_err(refuse)?
                    .into_keyexpr(),
            ])
        }
        _ => zk2::Fleet::new(session, contract, &[address])
            .and_then(|f| f.selectors(&plan.name, &plan.values))
            .map_err(refuse),
    }
}

/// The selection's interface tokens, read once: who could have answered.
async fn holders(
    session: &Session,
    revision: &Revision,
    plan: &CallPlan,
    timeout: Duration,
) -> (BenchPresence, BTreeSet<String>) {
    let address = plan.target.address.as_str();
    let (system, service) = address.split_once('/').unwrap_or((address, "*"));
    let selector = format!("zk2/{system}/{service}/@zk/alive/{}/**", revision.iface());
    match crate::bus::presence::liveliness_read(session, &selector, timeout).await {
        Ok(read) => {
            let held: BTreeSet<String> = read
                .keys
                .iter()
                .filter_map(|k| match parse(k) {
                    Ok(ZkKey::Alive { addr, .. }) => Some(addr.to_string()),
                    _ => None,
                })
                .collect();
            (
                BenchPresence {
                    selector,
                    complete: read.complete,
                    error: None,
                    holders: Vec::new(),
                },
                held,
            )
        }
        Err(e) => (
            BenchPresence {
                selector,
                complete: false,
                error: Some(crate::one_line(&e)),
                holders: Vec::new(),
            },
            BTreeSet::new(),
        ),
    }
}

/// How one call goes out: the query's settings, shared by every call.
#[derive(Clone)]
struct Query {
    session: Session,
    selectors: Vec<OwnedKeyExpr>,
    target: QueryTarget,
    encoding: zenoh::bytes::Encoding,
    priority: zenoh::qos::Priority,
    request: ZBytes,
    timeout: Duration,
}

impl Query {
    /// One call: every selector's query out first, then every reply,
    /// timed in its callback as it arrives, until each query completes.
    async fn call(&self) -> Result<Vec<Timed>> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Timed>();
        let started = Instant::now();
        for ke in &self.selectors {
            let tx = tx.clone();
            self.session
                .get(ke.clone())
                .payload(self.request.clone())
                .encoding(self.encoding.clone())
                .target(self.target)
                .consolidation(ConsolidationMode::None)
                .timeout(self.timeout)
                .priority(self.priority)
                .callback(move |reply| {
                    let _ = tx.send((started.elapsed(), reply));
                })
                .await
                .map_err(|e| Error::bus("bench call", ke.as_str(), e))?;
        }
        drop(tx);
        let mut out = Vec::new();
        while let Some(timed) = rx.recv().await {
            out.push(timed);
        }
        Ok(out)
    }
}

/// Refuses a bench that would repeat a write — an operation not declared
/// `idempotent`, unless `force` (O4's reasoning) — or measure nothing,
/// before anything is asked. [`run_bench`] asks it first; a caller asks it
/// before it opens a session.
pub fn check_bench(revision: &Revision, plan: &CallPlan, calls: usize, force: bool) -> Result<()> {
    if !plan.operation.idempotent && !force {
        return Err(Error::unaskable(
            format!("{} {}", revision.iface(), plan.name),
            "is not idempotent, and a benchmark calls it again and again: that is a \
             repeated write into a live deployment, not a measurement (O4). Pass --i-know \
             to mean it",
        ));
    }
    if calls == 0 {
        return Err(Error::unaskable("--calls 0", "measures nothing"));
    }
    Ok(())
}

/// Runs the benchmark.
pub async fn run_bench(session: &Session, spec: BenchSpec<'_>) -> Result<BenchReport> {
    let BenchSpec {
        revision,
        plan,
        request,
        calls,
        concurrency,
        timeout,
        force,
    } = spec;
    check_bench(revision, plan, calls, force)?;
    let keys = selectors(session, revision, plan)?;
    let fanout = plan.is_fanout();
    let query = Query {
        session: session.clone(),
        selectors: keys.clone(),
        target: if fanout {
            QueryTarget::All
        } else {
            QueryTarget::BestMatching
        },
        encoding: zk2::writer::wire_encoding(
            &plan.operation.request,
            plan.operation.encoding,
            &plan.values,
        ),
        priority: zk2::qos::priority(plan.operation.priority),
        request: ZBytes::from(request),
        timeout,
    };
    let (mut presence, held) = holders(session, revision, plan, timeout).await;

    let concurrency = concurrency.clamp(1, calls);
    let started = Instant::now();
    let mut repliers: Vec<(String, String, Vec<Duration>)> = Vec::new();
    let mut codes: BTreeMap<String, u64> = BTreeMap::new();
    let mut refusal_times = Vec::new();
    let mut without_value: BTreeMap<String, u64> = held.iter().map(|a| (a.clone(), 0)).collect();
    let (mut completed, mut silent, mut panicked) = (0usize, 0u64, 0u64);
    let (mut malformed, mut transport, mut discarded) = (0u64, 0u64, 0u64);
    let mut fatal = None;
    let mut issued = 0usize;
    while issued < calls && fatal.is_none() {
        let batch = concurrency.min(calls - issued);
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..batch {
            let q = query.clone();
            set.spawn(async move { q.call().await });
        }
        issued += batch;
        while let Some(joined) = set.join_next().await {
            let replies = match joined {
                Err(_) => {
                    panicked += 1;
                    continue;
                }
                Ok(Err(e)) => {
                    fatal.get_or_insert(e);
                    continue;
                }
                Ok(Ok(r)) => r,
            };
            completed += 1;
            let o = classify(revision, plan, replies);
            if o.values.is_empty() && o.refusals.is_empty() && o.malformed == 0 {
                silent += 1;
            }
            let heard: BTreeSet<&str> = o.values.iter().map(|(a, _, _)| a.as_str()).collect();
            for (addr, n) in &mut without_value {
                if !heard.contains(addr.as_str()) {
                    *n += 1;
                }
            }
            for (addr, key, at) in o.values {
                match repliers.iter_mut().find(|(_, k, _)| *k == key) {
                    Some((_, _, v)) => v.push(at),
                    None => repliers.push((addr, key, vec![at])),
                }
            }
            for (code, at) in o.refusals {
                *codes.entry(code).or_default() += 1;
                refusal_times.push(at);
            }
            malformed += o.malformed;
            transport += o.transport;
            discarded += o.discarded;
        }
    }
    if let Some(e) = fatal {
        return Err(e);
    }
    let elapsed = started.elapsed();
    presence.holders = without_value
        .into_iter()
        .map(|(address, without_value)| HolderTally {
            address,
            without_value,
        })
        .collect();
    Ok(BenchReport {
        address: plan.target.address.clone(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        operation: plan.name.clone(),
        values: plan.values.clone(),
        selectors: keys.iter().map(|k| k.as_str().to_owned()).collect(),
        mode: if fanout {
            CallMode::Fanout
        } else {
            CallMode::Concrete
        },
        requested: calls,
        completed,
        concurrency,
        timeout_s: timeout.as_secs_f64(),
        elapsed_s: elapsed.as_secs_f64(),
        calls_per_s: if elapsed.as_secs_f64() > 0.0 {
            completed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        },
        clock: LatencyClock::RoundTrip,
        repliers: repliers
            .into_iter()
            .filter_map(|(address, key, times)| {
                let replies = times.len() as u64;
                latency(times).map(|latency| ReplierLatency {
                    address,
                    key,
                    replies,
                    latency,
                })
            })
            .collect(),
        refusals: RefusalTally {
            count: codes.values().sum(),
            codes,
            latency: latency(refusal_times),
        },
        malformed,
        transport,
        silent,
        discarded,
        panicked,
        presence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_nearest_rank_and_survive_one_sample() {
        let d = |ms: u64| Duration::from_millis(ms);
        assert_eq!(percentile(&[d(7)], 50.0), 7.0);
        assert_eq!(percentile(&[d(7)], 99.0), 7.0);
        let ten: Vec<Duration> = (1..=10).map(d).collect();
        assert_eq!(percentile(&ten, 50.0), 5.0);
        assert_eq!(percentile(&ten, 95.0), 10.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
        assert_eq!(latency(Vec::new()), None, "no sample, no distribution");
        let l = latency(vec![d(3), d(1), d(2)]).unwrap();
        assert_eq!((l.min_ms, l.p50_ms, l.max_ms), (1.0, 2.0, 3.0));
    }
}
