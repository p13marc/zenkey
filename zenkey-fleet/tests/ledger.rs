//! The O6 ledger: every shed message is accounted for (issue #45).
//!
//! `zenkey-fleet/src/tree.rs` claims "a hot bus cannot melt a zengui redraw"
//! and `sub.rs` claims overflow "surfaces as an explicit `Dropped` count …
//! never silently". Those are conservation laws, and nothing verified them.
//!
//! What is asserted here is an **identity between counters the code already
//! maintains**, never a threshold on a number the scheduler picks:
//!
//! ```text
//! received_events + Σ Dropped(n) == sent_events
//! inserted_keys               == stats.len() + evicted + unwatched
//! facts.inserted()            == facts.len() + facts.evicted()
//! ```
//!
//! Note the third law is over **insertions**, not over samples and not over
//! distinct keys. `ensure` on a key already held is a cache hit; a key evicted
//! and later re-observed is projected *again*. Both wrong formulations were
//! tried first and both were caught by the soak, which is the argument for
//! keeping a soak at all.
//!
//! The third law is the one issue #107 was about: an explorer's *projection*
//! cache shadows the key table, and until that issue it was unbounded while the
//! table it shadowed was not — "merely a leak with better manners", in
//! `stats.rs`'s own words. It lives here rather than in a frontend for the same
//! reason the other two do: `MonitorCore` and `FactsCache` are both pure, so the
//! identity is checkable without a bus, a port or a window.
//!
//! The interleaving decides *how many* are dropped; it cannot decide whether
//! the books balance. That is what makes these ordinary fast tests rather than
//! a benchmark: a regression that unbounds a buffer breaks the identity in one
//! of two visible ways — loss with no `Dropped` (silent), or `Dropped` with
//! nothing lost (phantom).
//!
//! Three traps, each worked around rather than papered over:
//!
//! - **`Lagged(n)` counts events, not samples.** `tick()` puts `StatsTick` on
//!   the same broadcast channel, so a raw `samples_sent == samples_recv +
//!   dropped` is simply false. The ledger is per event class, and the harness
//!   knows exactly how many of each it sent.
//! - **`EventStream::recv` never returns `None`.** The stream holds an
//!   `Arc<MonitorCore>`, which owns the sender, so `Closed` is unreachable and
//!   a drain-until-`None` would hang forever. Every phase ends with a
//!   **sentinel** `NodeDown`, which cannot be lost: once production stops
//!   nothing can overwrite the last slot written.
//! - **`dropped()` sums across receivers**, and only moves when a receiver
//!   *polls*. So the balance test opens exactly one stream, and the two other
//!   behaviours get tests of their own.
//!
//! Session-free throughout — `MonitorCore` is "pure and deterministically
//! testable" by its own doc, so none of this needs a bus or a port.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use zenkey_fleet::{EventStream, FactsCache, FleetEvent, MonitorCore, SampleView, StreamItem};

/// The last event of every phase. A `NodeDown` because it is trivially
/// distinguishable from anything else the harness sends.
const SENTINEL: &str = "ledger/sentinel";

/// The synthetic sample. `SampleView`'s fields are public precisely so a
/// harness can build one without a bus.
fn view(key: &str, len: usize) -> SampleView {
    SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(vec![0u8; len]),
        encoding: "application/json".to_string(),
        kind: zenoh::sample::SampleKind::Put,
        timestamp: None,
        received: Instant::now(),
    }
}

fn synth_key(i: usize) -> String {
    format!("v1/h-3fa9c2d41b7e/telemetry/synth/g{}/k{i}", i / 100)
}

/// Events counted per class — the shape both sides of the ledger take.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Tally {
    samples: u64,
    ticks: u64,
    nodes: u64,
    /// Receiver side only: the sum of every `Dropped(n)` seen.
    dropped: u64,
}

impl Tally {
    fn events(&self) -> u64 {
        self.samples + self.ticks + self.nodes
    }
}

/// Drain until the sentinel arrives, tallying by class.
async fn drain_to_sentinel(events: &mut EventStream, seen: &mut Tally) {
    loop {
        let item = events
            .recv()
            .await
            .expect("the stream holds an Arc<MonitorCore>, so it cannot close");
        match item {
            StreamItem::Dropped(n) => seen.dropped += n,
            StreamItem::Event(FleetEvent::Sample(_)) => seen.samples += 1,
            StreamItem::Event(FleetEvent::StatsTick) => seen.ticks += 1,
            StreamItem::Event(FleetEvent::NodeDown(k)) => {
                seen.nodes += 1;
                if k == SENTINEL {
                    return;
                }
            }
            StreamItem::Event(FleetEvent::NodeUp(_)) => seen.nodes += 1,
            // Not produced by this harness; counted so a future variant
            // cannot silently unbalance the ledger.
            StreamItem::Event(_) => seen.nodes += 1,
        }
    }
}

/// Take at most `n` items without blocking past what is buffered.
async fn drain_some(events: &mut EventStream, n: usize, seen: &mut Tally) {
    for _ in 0..n {
        match tokio::time::timeout(Duration::ZERO, events.recv()).await {
            Ok(Some(StreamItem::Dropped(d))) => seen.dropped += d,
            Ok(Some(StreamItem::Event(FleetEvent::Sample(_)))) => seen.samples += 1,
            Ok(Some(StreamItem::Event(FleetEvent::StatsTick))) => seen.ticks += 1,
            Ok(Some(StreamItem::Event(_))) => seen.nodes += 1,
            Ok(None) => unreachable!("the stream cannot close"),
            // Nothing buffered right now.
            Err(_) => return,
        }
    }
}

/// The whole ledger, asserted. The fast test and the soak call this same
/// function — only the numbers differ.
fn assert_ledger(core: &MonitorCore, sent: &Tally, seen: &Tally) {
    assert_eq!(
        seen.events() + seen.dropped,
        sent.events(),
        "the O6 ledger does not balance: sent {sent:?}, saw {seen:?} — \
         either something was shed without a Dropped count, or a Dropped \
         count was reported for something that arrived"
    );
    assert_eq!(
        core.dropped(),
        seen.dropped,
        "with exactly one receiver, the core's count is that receiver's count"
    );
    assert!(
        seen.samples <= sent.samples,
        "more samples arrived than were sent"
    );
}

/// The headline: with a bounded channel and a consumer that stalls, nothing
/// vanishes unaccounted for.
#[tokio::test]
async fn the_o6_ledger_balances() {
    // A power of two: tokio rounds broadcast capacity up, and a test that
    // depended on the rounding would be testing tokio.
    let core = MonitorCore::bounded(64, 4096);
    let mut events = core.events();
    let mut sent = Tally::default();
    let mut seen = Tally::default();

    // Bursts sized to straddle the channel bound, with deliberate stall
    // rounds where the consumer drains nothing at all — the "slow consumer
    // phase" #45 asks for. Pure functions of the round, so the run is
    // reproducible.
    for round in 0..20usize {
        let burst = 8 + (round * 13) % 200;
        for i in 0..burst {
            core.ingest(view(&synth_key(round * 200 + i), 64), None);
            sent.samples += 1;
        }
        core.tick();
        sent.ticks += 1;

        // Every third round the consumer is asleep.
        if round % 3 != 0 {
            drain_some(&mut events, 512, &mut seen).await;
            // The GUI pulls the tree here rather than accumulating; doing it
            // keeps the harness honest about what a tick costs a consumer.
            let _ = core.tree();
        }
    }

    core.node_event(SENTINEL.to_string(), false);
    sent.nodes += 1;
    drain_to_sentinel(&mut events, &mut seen).await;

    assert_ledger(&core, &sent, &seen);
    assert!(
        seen.dropped > 0,
        "the fixture must actually overflow the bound, or it proves nothing \
         (sent {} events into a 64-slot channel)",
        sent.events()
    );
}

/// `dropped()` is the sum across receivers, not a per-receiver number. Pinned
/// so nobody "fixes" it into the other thing.
#[tokio::test]
async fn dropped_is_the_sum_across_receivers() {
    let core = MonitorCore::bounded(8, 4096);
    let mut a = core.events();
    let mut b = core.events();

    for i in 0..64usize {
        core.ingest(view(&synth_key(i), 8), None);
    }
    core.node_event(SENTINEL.to_string(), false);

    let (mut seen_a, mut seen_b) = (Tally::default(), Tally::default());
    drain_to_sentinel(&mut a, &mut seen_a).await;
    drain_to_sentinel(&mut b, &mut seen_b).await;

    let sent = Tally {
        samples: 64,
        nodes: 1,
        ..Tally::default()
    };
    // Each receiver has its own cursor, so each balances independently…
    assert_eq!(seen_a.events() + seen_a.dropped, sent.events());
    assert_eq!(seen_b.events() + seen_b.dropped, sent.events());
    // …and the core reports their sum.
    assert_eq!(core.dropped(), seen_a.dropped + seen_b.dropped);
    assert!(seen_a.dropped > 0 && seen_b.dropped > 0);
}

/// A receiver that never polls is not losing anything yet — the count moves
/// when it drains, which is why the ledger is only read at quiescence.
#[tokio::test]
async fn a_stalled_receiver_reports_nothing_until_it_drains() {
    let core = MonitorCore::bounded(8, 4096);
    let mut events = core.events();

    for i in 0..64usize {
        core.ingest(view(&synth_key(i), 8), None);
    }
    assert_eq!(
        core.dropped(),
        0,
        "a stalled receiver has reported nothing, because it has not looked"
    );

    core.node_event(SENTINEL.to_string(), false);
    let mut seen = Tally::default();
    drain_to_sentinel(&mut events, &mut seen).await;

    let sent = Tally {
        samples: 64,
        nodes: 1,
        ..Tally::default()
    };
    assert_ledger(&core, &sent, &seen);
    assert!(core.dropped() > 0, "and then it reports the whole gap");
}

/// The key axis: the bounded table forgets, and says exactly how much.
/// Keys are distinct and monotonic, so no evicted key ever returns — the
/// identity is then exact for the whole run.
#[tokio::test]
async fn the_key_ledger_balances_under_eviction_and_retirement() {
    const KEYS: usize = 2_000;
    let core = MonitorCore::bounded(1024, 128);

    for i in 0..KEYS {
        core.ingest(view(&synth_key(i), 8), None);
    }
    let (len, evicted, unwatched) = (
        core.with_stats(|s| s.len()) as u64,
        core.keys_evicted(),
        core.keys_unwatched(),
    );
    assert_eq!(
        len + evicted + unwatched,
        KEYS as u64,
        "every key offered is either held, evicted, or retired — never simply gone"
    );
    assert!(evicted > 0, "the bound must actually have bitten");
    assert!(
        len <= 128,
        "the table must stay within its bound, held {len}"
    );

    // Retirement is the third category, and must not be counted as eviction.
    let before = core.keys_evicted();
    let retired = core.with_stats_mut(|s| {
        s.retire_unwatched("v1/*/telemetry/**", &["v1/*/state/**".to_string()])
    });
    assert!(retired > 0, "the retirement must actually have retired");
    assert_eq!(
        core.keys_evicted(),
        before,
        "retiring a watch is not an eviction — three counters, three failures"
    );
    assert_eq!(core.keys_unwatched(), retired as u64);
    assert_eq!(
        core.with_stats(|s| s.len()) as u64 + core.keys_evicted() + core.keys_unwatched(),
        KEYS as u64,
        "and the identity still holds after retirement"
    );
}

/// The soak: a genuinely hot bus, measured rather than asserted.
///
/// Run with
/// `cargo test --release -p zenkey-fleet --test ledger -- --ignored --nocapture`
/// (or `just soak`). Bounded by message count, not by duration, so `sent` is
/// reproducible and a failure is diagnosable; the achieved rate is reported
/// The third law, on its own: far more distinct keys than the bound, and the
/// books still balance (issue #107).
///
/// Fast, session-free and *not* `#[ignore]`d — it is the regression guard, and
/// a guard that only runs under `just soak` is one nobody runs. The soak below
/// exercises the same identity against a hot bus.
#[test]
fn the_projection_cache_balances_its_own_ledger() {
    const KEYS: usize = 200_000;
    const BOUND: usize = 1_000;

    let mut cache = FactsCache::with_capacity(BOUND);
    for i in 0..KEYS {
        cache.ensure("", &synth_key(i), None);
    }

    assert!(
        cache.len() <= BOUND,
        "the cache is bounded: held {} against a bound of {BOUND}",
        cache.len()
    );
    assert!(
        cache.evicted() > 0,
        "the fixture must actually trip the bound, or this asserts nothing"
    );
    assert_eq!(cache.inserted(), KEYS as u64, "every key here was distinct");
    assert_eq!(
        cache.len() as u64 + cache.evicted(),
        cache.inserted(),
        "every projection is either held or counted as retired — nothing \
         disappears unaccounted (RFC 09 §5.1 O6)"
    );
}

/// rather than demanded, because a shared CI runner cannot promise one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "soak — run explicitly, read the printed numbers"]
async fn hot_bus_soak() {
    const MESSAGES: usize = 2_000_000;
    const KEYS: usize = 10_000;

    let core = MonitorCore::bounded(1024, 50_000);
    let mut events = core.events();
    let sent_samples = Arc::new(AtomicU64::new(0));
    let sent_ticks = Arc::new(AtomicU64::new(0));

    let producer = {
        let core = Arc::clone(&core);
        let sent = Arc::clone(&sent_samples);
        tokio::spawn(async move {
            let started = Instant::now();
            for i in 0..MESSAGES {
                core.ingest(view(&synth_key(i % KEYS), 64), None);
                sent.fetch_add(1, Ordering::Relaxed);
                // Yield periodically so the consumer and ticker are not
                // starved on a small worker pool.
                if i % 4096 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            started.elapsed()
        })
    };

    let ticker = {
        let core = Arc::clone(&core);
        let sent = Arc::clone(&sent_ticks);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_millis(250));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                let at = Instant::now();
                core.tick();
                sent.fetch_add(1, Ordering::Relaxed);
                let build = at.elapsed();
                if build > Duration::from_millis(100) {
                    println!("  slow tick: snapshot build took {build:?}");
                }
            }
        })
    };

    // The consumer, shaped like the GUI pump: drain a batch, pull the tree,
    // and stall outright every so often.
    let mut seen = Tally::default();
    let mut tick_gaps: Vec<Duration> = Vec::new();
    let mut last_tick = Instant::now();
    // The projection cache a frontend keeps beside the table, fed exactly as
    // `apply_tick` feeds it — one `ensure` per sample (#107).
    let mut facts = FactsCache::with_capacity(1_000);
    // Distinct keys the consumer saw — printed beside the insertion count, so
    // the re-projection cost of a small bound over a large key space is a
    // number the soak actually reports rather than an argument in a comment.
    let mut distinct: std::collections::HashSet<String> = std::collections::HashSet::new();
    let finisher = {
        let core = Arc::clone(&core);
        tokio::spawn(async move {
            let elapsed = producer.await.expect("producer");
            ticker.abort();
            let _ = ticker.await;
            core.node_event(SENTINEL.to_string(), false);
            elapsed
        })
    };

    loop {
        let item = events.recv().await.expect("the stream cannot close");
        match item {
            StreamItem::Dropped(n) => seen.dropped += n,
            StreamItem::Event(FleetEvent::Sample(ref view)) => {
                seen.samples += 1;
                facts.ensure("", &view.key, None);
                distinct.insert(view.key.clone());
            }
            StreamItem::Event(FleetEvent::StatsTick) => {
                seen.ticks += 1;
                tick_gaps.push(last_tick.elapsed());
                last_tick = Instant::now();
                let _ = core.tree();
            }
            StreamItem::Event(FleetEvent::NodeDown(ref k)) if k == SENTINEL => {
                seen.nodes += 1;
                break;
            }
            StreamItem::Event(_) => seen.nodes += 1,
        }
    }
    let elapsed = finisher.await.expect("finisher");

    let sent = Tally {
        samples: sent_samples.load(Ordering::Relaxed),
        ticks: sent_ticks.load(Ordering::Relaxed),
        nodes: 1,
        dropped: 0,
    };

    tick_gaps.sort_unstable();
    let pct = |p: f64| -> Duration {
        if tick_gaps.is_empty() {
            return Duration::ZERO;
        }
        tick_gaps[((tick_gaps.len() - 1) as f64 * p) as usize]
    };
    let rate = sent.samples as f64 / elapsed.as_secs_f64();

    println!(
        "soak: {} samples in {elapsed:?} = {rate:.0}/s",
        sent.samples
    );
    println!(
        "  delivered {} · dropped {} · ticks {}",
        seen.samples, seen.dropped, seen.ticks
    );
    println!(
        "  tick-to-tick  p50 {:?}  p99 {:?}  max {:?}",
        pct(0.50),
        pct(0.99),
        tick_gaps.last().copied().unwrap_or_default()
    );
    println!(
        "  keys held {} · evicted {} · unwatched {}",
        core.with_stats(|s| s.len()),
        core.keys_evicted(),
        core.keys_unwatched()
    );
    println!(
        "  facts cached {} · retired {} · projected {} over {} distinct keys",
        facts.len(),
        facts.evicted(),
        facts.inserted(),
        distinct.len()
    );

    // The same identity the fast test asserts — that is the point of sharing
    // the function.
    assert_ledger(&core, &sent, &seen);
    assert!(
        core.with_stats(|s| s.len()) <= 50_000,
        "the key table must stay within its bound under sustained load"
    );
    // The third law, under load: the projection cache is bounded too, and says
    // what the bound cost (#107).
    assert!(
        facts.len() <= 1_000,
        "the projection cache must stay within its bound: held {}",
        facts.len()
    );
    assert_eq!(
        facts.len() as u64 + facts.evicted(),
        facts.inserted(),
        "every projection is held or counted as retired"
    );
    assert!(
        facts.inserted() >= distinct.len() as u64,
        "a bound smaller than the key space re-projects; it cannot under-project"
    );
    // Generous on purpose: the claim is "bounded", not "fast on this machine".
    assert!(
        pct(0.99) < Duration::from_secs(2),
        "tick-to-tick p99 {:?} — the render loop is being starved",
        pct(0.99)
    );
}
