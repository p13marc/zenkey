//! Baseline benchmarks for the fleet engine's hot paths (issue #44).
//!
//! Same contract as `zenkey/benches/keys.rs`: numbers live in
//! `docs/bench-baseline.md`, and the point estimate is what we compare.
//! `docs/zero-copy.md` names which id pins which rule — these are the numbers
//! the discipline doc is written against, not decoration.
//!
//! Two axes, because the engine has two cadences:
//!
//! - **per sample** — `stats/record_*`, `decode/structural_*`, `lens/*`,
//!   `monitor/ingest`. A 100 kHz bus runs these 100 000 times a second.
//! - **per tick** — `tree/build_*`, `monitor/tick_*`. These run
//!   four times a second and scale with the *key population*, not the sample
//!   rate; that separation is the whole of the "a hot bus cannot melt a render
//!   loop" claim (`zenkey-fleet/src/tree.rs`), and #45 soaks it.
//!
//! Everything here is session-free. The fan-in path (`fleet_get`,
//! `collect_answers`) is deliberately absent: a `zenoh::query::Reply` cannot be
//! synthesised, so benching it would mean benching a mock. v1's `skeleton/*`,
//! `facts/*`, `registry/*` and `fanin/origin_attribution_*` left with the v1
//! registry (#612, FJ9); `lens/*` is the per-sample resolution that replaced
//! them.
//!
//! **`tree/build_50k` is release-only in practice** —
//! tens of milliseconds an iteration, so a debug run takes minutes. Run the
//! whole file with `cargo bench -p zenkey-fleet`, or a group with
//! `cargo bench -p zenkey-fleet -- stats/`.

use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use zenkey_fleet::KeyTreeSnapshot;
use zenkey_fleet::model::stats::StatsTable;

/// A zk2 key shape with 100 keys per group, so the tree has realistic
/// fan-out rather than one flat level.
fn synth_key(i: usize) -> String {
    format!("prod/zk2/host-a/synth/synth.v1/stream/g{}/k{i}", i / 100)
}

/// A stats table of `n` distinct synthetic keys, one sample each.
fn table_of(n: usize) -> StatsTable {
    let mut stats = StatsTable::new();
    let now = Instant::now();
    for i in 0..n {
        stats.record(&synth_key(i), 64, None, now, None, None);
    }
    stats
}

// ── per sample ───────────────────────────────────────────────────────────

fn bench_stats(c: &mut Criterion) {
    let now = Instant::now();
    let key = synth_key(0);

    // The overwhelmingly common case: a key already in the table. Allocation-
    // free by design (`stats.rs` header), so this is the floor.
    c.bench_function("stats/record_hit", |b| {
        let mut stats = StatsTable::new();
        stats.record(&key, 64, None, now, None, None);
        b.iter(|| {
            stats.record(
                black_box(&key),
                black_box(64),
                black_box(None),
                now,
                None,
                None,
            )
        })
    });

    // The insert branch: one `key.to_string()` and a possible map grow.
    c.bench_function("stats/record_new_key", |b| {
        let mut stats = StatsTable::new();
        let mut i = 0usize;
        b.iter(|| {
            i += 1;
            stats.record(
                black_box(&synth_key(i)),
                black_box(64),
                None,
                now,
                None,
                None,
            )
        })
    });

    // Steady state at the bound, so the amortised `evict` scan is in the
    // number rather than hidden behind it.
    c.bench_function("stats/record_past_the_bound", |b| {
        let mut stats = StatsTable::with_capacity(1024);
        let mut i = 0usize;
        b.iter(|| {
            i += 1;
            stats.record(
                black_box(&synth_key(i)),
                black_box(64),
                None,
                Instant::now(),
                None,
                None,
            )
        })
    });

    // The O(keys) fold the GUI pump takes under the ingest mutex every tick.
    let ten_k = table_of(10_000);
    c.bench_function("stats/totals_10k", |b| {
        b.iter(|| black_box(&ten_k).totals())
    });

    // The unwatch path: one keyexpr per table key. `iter_batched` because
    // `retire_unwatched` consumes the table it is given — building a fresh
    // one inside `iter` would put table construction in the number, and a
    // bench whose id says `retire_unwatched` must measure that.
    c.bench_function("stats/retire_unwatched_1k", |b| {
        let kept = vec!["prod/zk2/*/*/*/state/**".to_string()];
        b.iter_batched(
            || table_of(1_000),
            |mut stats| {
                stats.retire_unwatched(black_box("prod/zk2/*/*/*/stream/**"), black_box(&kept))
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

fn bench_decode(c: &mut Criterion) {
    use zenkey_fleet::{structural, structural_value};

    let json = br#"{"value":42.0,"unit":"percent","inodes":1188}"#;
    let mut cbor = Vec::new();
    ciborium::into_writer(
        &serde_json::json!({"value": 42.0, "unit": "percent", "inodes": 1188}),
        &mut cbor,
    )
    .expect("fixture cbor");
    let text = b"just a plain string";
    let opaque = [0xff_u8, 0xfe, 0x00, 0x13];

    c.bench_function("decode/structural_json", |b| {
        b.iter(|| structural(black_box(json)))
    });
    c.bench_function("decode/structural_value_json", |b| {
        b.iter(|| structural_value(black_box(json)))
    });
    c.bench_function("decode/structural_cbor", |b| {
        b.iter(|| structural(black_box(&cbor)))
    });
    c.bench_function("decode/structural_text", |b| {
        b.iter(|| structural(black_box(text)))
    });
    c.bench_function("decode/structural_opaque", |b| {
        b.iter(|| structural(black_box(&opaque)))
    });
}

// ── per tick ─────────────────────────────────────────────────────────────

fn bench_tree(c: &mut Criterion) {
    for n in [1_000usize, 10_000, 50_000] {
        let stats = table_of(n);
        // `rows` is what the stats tick holds the ingest lock for, `build` is
        // rows + the fold that now happens after releasing it (#330). Both are
        // benched so the ratio between them stays visible: the whole claim is
        // that the network callback thread waits behind the first number and
        // not the second.
        c.bench_function(&format!("tree/rows_{}k", n / 1_000), |b| {
            b.iter(|| black_box(&stats).rows())
        });
        c.bench_function(&format!("tree/build_{}k", n / 1_000), |b| {
            b.iter(|| KeyTreeSnapshot::build(black_box(&stats)))
        });
    }
}

// ── the lens ─────────────────────────────────────────────────────────────

fn bench_lens(c: &mut Criterion) {
    use zenkey_fleet::{ContractSet, Lens};

    // The rungs a raw observer's key stops at before any presence read: the
    // cheap refusals every sample of a busy foreign bus pays.
    let contracts = ContractSet::new();
    let lens = Lens::new("prod", None, &contracts);
    let zk2 = "prod/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    let foreign = "plant/line-1/temp";
    let elsewhere = "staging/zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    c.bench_function("lens/resolve_zk2_unread", |b| {
        b.iter(|| lens.resolve(black_box(zk2)))
    });
    c.bench_function("lens/resolve_not_zk2", |b| {
        b.iter(|| lens.resolve(black_box(foreign)))
    });
    c.bench_function("lens/resolve_not_in_namespace", |b| {
        b.iter(|| lens.resolve(black_box(elsewhere)))
    });
}

// ── the monitor, end to end ──────────────────────────────────────────────

fn bench_monitor(c: &mut Criterion) {
    use zenkey_fleet::{MonitorCore, SampleView};

    fn view(key: &str) -> SampleView {
        SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(vec![0u8; 64]),
            encoding: "application/json".to_string(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: Instant::now(),
        }
    }

    // The whole per-sample cost: stats lock, record, `Arc` alloc, broadcast
    // send. This is the number that answers "can it take 100k msg/s".
    // A receiver is held so the send has somewhere to go, and deliberately
    // never drained — a lagging consumer is the case that must stay cheap.
    let core = MonitorCore::new(1024);
    let _rx = core.events();
    let key = synth_key(0);
    c.bench_function("monitor/ingest", |b| {
        b.iter(|| core.ingest(black_box(view(&key)), black_box(None)))
    });

    let loaded = MonitorCore::new(1024);
    let now = Instant::now();
    loaded.with_stats_mut(|s| {
        for i in 0..10_000 {
            s.record(&synth_key(i), 64, None, now, None, None);
        }
    });
    c.bench_function("monitor/tick_10k", |b| b.iter(|| black_box(&loaded).tick()));
}

criterion_group!(
    benches,
    bench_stats,
    bench_decode,
    bench_tree,
    bench_lens,
    bench_monitor
);
criterion_main!(benches);
