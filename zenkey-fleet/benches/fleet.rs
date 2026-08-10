//! Baseline benchmarks for the fleet engine's hot paths (issue #44).
//!
//! Same contract as `zenkey/benches/keys.rs`: numbers live in
//! `docs/bench-baseline.md`, and the point estimate is what we compare.
//! `docs/zero-copy.md` names which id pins which rule — these are the numbers
//! the discipline doc is written against, not decoration.
//!
//! Two axes, because the engine has two cadences:
//!
//! - **per sample** — `stats/record_*`, `decode/structural_*`, `monitor/ingest`.
//!   A 100 kHz bus runs these 100 000 times a second.
//! - **per tick** — `tree/build_*`, `skeleton/*`, `monitor/tick_*`. These run
//!   four times a second and scale with the *key population*, not the sample
//!   rate; that separation is the whole of the "a hot bus cannot melt a render
//!   loop" claim (`zenkey-fleet/src/tree.rs`), and #45 soaks it.
//!
//! Everything here is session-free. The fan-in path (`fleet_get`,
//! `collect_answers`) is deliberately absent: a `zenoh::query::Reply` cannot be
//! synthesised, so benching it would mean benching a mock. `fanin/origin_of`
//! covers the one pure fragment.
//!
//! **`tree/build_50k` and `skeleton/merge_10k` are release-only in practice** —
//! tens of milliseconds an iteration, so a debug run takes minutes. Run the
//! whole file with `cargo bench -p zenkey-fleet`, or a group with
//! `cargo bench -p zenkey-fleet -- stats/`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use zenkey_fleet::stats::StatsTable;
use zenkey_fleet::{KeyFacts, KeyTreeSnapshot, SliceSet};

/// The canonical fixture origin, shared with `zenkey/benches/keys.rs`, the
/// pane tests and `spray`.
const HOST: &str = "h-3fa9c2d41b7e";

/// The key shape `spray --keys N` generates: 100 keys per group, so the tree
/// has realistic fan-out rather than one flat level.
fn synth_key(i: usize) -> String {
    format!("v1/{HOST}/telemetry/synth/g{}/k{i}", i / 100)
}

/// A stats table of `n` distinct synthetic keys, one sample each.
fn table_of(n: usize) -> StatsTable {
    let mut stats = StatsTable::new();
    let now = Instant::now();
    for i in 0..n {
        stats.record(&synth_key(i), 64, None, now);
    }
    stats
}

/// The fixture registry — the same corpus the pane tests and codegen use.
fn slices() -> SliceSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
    SliceSet::from_dirs(&[dir]).expect("fixture registry")
}

// ── per sample ───────────────────────────────────────────────────────────

fn bench_stats(c: &mut Criterion) {
    let now = Instant::now();
    let key = synth_key(0);

    // The overwhelmingly common case: a key already in the table. Allocation-
    // free by design (`stats.rs` header), so this is the floor.
    c.bench_function("stats/record_hit", |b| {
        let mut stats = StatsTable::new();
        stats.record(&key, 64, None, now);
        b.iter(|| stats.record(black_box(&key), black_box(64), black_box(None), now))
    });

    // The insert branch: one `key.to_string()` and a possible map grow.
    c.bench_function("stats/record_new_key", |b| {
        let mut stats = StatsTable::new();
        let mut i = 0usize;
        b.iter(|| {
            i += 1;
            stats.record(black_box(&synth_key(i)), black_box(64), None, now)
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
            )
        })
    });

    // The O(keys) fold the GUI pump takes under the ingest mutex every tick.
    let ten_k = table_of(10_000);
    c.bench_function("stats/totals_10k", |b| {
        b.iter(|| black_box(&ten_k).totals())
    });

    // The unwatch path: one keyexpr per table key, before the #44 fix.
    c.bench_function("stats/retire_unwatched_1k", |b| {
        let kept = vec!["v1/*/state/**".to_string()];
        b.iter(|| {
            let mut stats = table_of(1_000);
            stats.retire_unwatched(black_box("v1/*/telemetry/**"), black_box(&kept))
        })
    });
}

fn bench_decode(c: &mut Criterion) {
    use zenkey_fleet::decode::{structural, structural_value};

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
        c.bench_function(&format!("tree/build_{}k", n / 1_000), |b| {
            b.iter(|| KeyTreeSnapshot::build(black_box(&stats)))
        });
    }
}

fn bench_skeleton(c: &mut Criterion) {
    let slices = slices();
    let roster: BTreeMap<String, Vec<String>> = (0..100)
        .map(|i| (format!("h-{i:012x}"), vec!["sysinfo".to_string()]))
        .collect();

    c.bench_function("skeleton/build_fixture", |b| {
        b.iter(|| zenkey_fleet::Skeleton::build(black_box(""), &slices, &roster, None))
    });

    let skel = zenkey_fleet::Skeleton::build("", &slices, &roster, None);
    let observed = KeyTreeSnapshot::build(&table_of(10_000));
    let watched = vec!["v1/**".to_string()];
    c.bench_function("skeleton/merge_10k", |b| {
        b.iter(|| {
            zenkey_fleet::skeleton::merge(
                black_box(&skel),
                black_box(&observed),
                black_box(&watched),
            )
        })
    });
}

// ── projection and refinement ────────────────────────────────────────────

fn bench_facts(c: &mut Criterion) {
    let slices = slices();
    let registered = format!("v1/{HOST}/telemetry/sysinfo/disk/var-log/used");
    let unregistered = format!("v1/{HOST}/telemetry/sysinfo/no/such/thing");
    let foreign = "demo/example/foo";
    let elsewhere = format!("otherbase/v1/{HOST}/state/sysinfo/health");

    c.bench_function("facts/project_v1", |b| {
        b.iter(|| KeyFacts::project(black_box(""), black_box(&registered)))
    });
    c.bench_function("facts/project_unparsed", |b| {
        b.iter(|| KeyFacts::project(black_box(""), black_box(foreign)))
    });
    c.bench_function("facts/project_not_under_base", |b| {
        b.iter(|| KeyFacts::project(black_box("zensight"), black_box(&elsewhere)))
    });
    c.bench_function("facts/resolve_registered", |b| {
        b.iter(|| {
            let mut f = KeyFacts::project("", black_box(&registered));
            f.resolve(black_box(&slices));
            f
        })
    });
    c.bench_function("facts/resolve_unregistered", |b| {
        b.iter(|| {
            let mut f = KeyFacts::project("", black_box(&unregistered));
            f.resolve(black_box(&slices));
            f
        })
    });
    c.bench_function("facts/describe_key", |b| {
        b.iter(|| zenkey_fleet::describe_key(black_box(""), black_box(&registered), Some(&slices)))
    });
}

fn bench_registry(c: &mut Criterion) {
    let slices = slices();
    c.bench_function("registry/refine_hit", |b| {
        b.iter(|| {
            slices.refine(
                black_box("sysinfo"),
                black_box("telemetry"),
                black_box(&["disk", "var-log", "used"]),
            )
        })
    });
    c.bench_function("registry/refine_miss", |b| {
        b.iter(|| {
            slices.refine(
                black_box("sysinfo"),
                black_box("telemetry"),
                black_box(&["no", "such", "thing"]),
            )
        })
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
            s.record(&synth_key(i), 64, None, now);
        }
    });
    c.bench_function("monitor/tick_10k", |b| b.iter(|| black_box(&loaded).tick()));
}

fn bench_fanin(c: &mut Criterion) {
    // The one pure fragment of the fan-in path: attributing a reply key to the
    // origin that answered. Everything else in `fleet_get` needs a session.
    let keys: Vec<String> = (0..256)
        .map(|i| format!("v1/h-{i:012x}/@rpc/sysinfo/introspect"))
        .collect();
    c.bench_function("fanin/origin_attribution_256", |b| {
        b.iter(|| {
            for k in &keys {
                black_box(zenkey::grammar::parse_full(black_box(""), black_box(k)));
            }
        })
    });
}

criterion_group!(
    benches,
    bench_stats,
    bench_decode,
    bench_tree,
    bench_skeleton,
    bench_facts,
    bench_registry,
    bench_monitor,
    bench_fanin
);
criterion_main!(benches);
