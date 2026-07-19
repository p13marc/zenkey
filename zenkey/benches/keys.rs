//! Baseline benchmarks for the key hot paths (issue #17).
//!
//! Recorded BEFORE the 0.3 core rewrite so the redesign's perf claims are
//! measured, not asserted. Numbers live in `docs/bench-baseline.md`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use zenkey::grammar::{self, Class, Origin, Producer};
use zenkey::origin::HostId;
use zenkey::slug::chunk_slug;

fn host() -> Origin {
    Origin::Host(HostId::parse("h-3fa9c2d41b7e").unwrap())
}

fn bench_build(c: &mut Criterion) {
    let origin = host();
    let producer = Producer::new("netring").unwrap();
    c.bench_function("build/data_key_3_chunks", |b| {
        b.iter(|| {
            grammar::data_key(
                black_box(&origin),
                Class::Telemetry,
                Some(black_box(&producer)),
                black_box(&["flow", "red", "p95_ms"]),
            )
            .unwrap()
        })
    });
    c.bench_function("build/rpc_key", |b| {
        b.iter(|| {
            grammar::rpc_key(
                black_box(&origin),
                Some(black_box(&producer)),
                black_box(&["capture_disk", "set"]),
            )
            .unwrap()
        })
    });
    c.bench_function("build/producer_chunk", |b| {
        let inst = Producer::with_instance("netring", 2).unwrap();
        b.iter(|| black_box(&inst).chunk())
    });
}

fn bench_parse(c: &mut Criterion) {
    let keys = [
        "v1/h-3fa9c2d41b7e/telemetry/netring/flow/red/p95_ms",
        "v1/h-3fa9c2d41b7e/state/netring/alert/9f2c81ab04d7e3f1",
        "v1/h-3fa9c2d41b7e/@rpc/netring/capture_disk/set",
        "v1/@catalog/state/entity/9f2c81ab04d7e3f1",
        "v1/h-3fa9c2d41b7e/@blob/store/sha256/9f2c81ab04d7e3f1",
    ];
    c.bench_function("parse/structural_5_keys", |b| {
        b.iter(|| {
            for k in keys {
                black_box(grammar::parse(black_box(k)).unwrap());
            }
        })
    });
}

fn bench_slug(c: &mut Criterion) {
    c.bench_function("slug/clean_chunk", |b| {
        b.iter(|| chunk_slug(black_box("p95_ms")))
    });
    c.bench_function("slug/dirty_chunk", |b| {
        b.iter(|| chunk_slug(black_box("Röuter 1/ETH0")))
    });
}

fn bench_intersects(c: &mut Criterion) {
    use zenoh_keyexpr::keyexpr;
    let sel = keyexpr::new("v1/*/telemetry/**").unwrap();
    let key = keyexpr::new("v1/h-3fa9c2d41b7e/telemetry/netring/flow/red/p95_ms").unwrap();
    c.bench_function("match/keyexpr_intersects", |b| {
        b.iter(|| black_box(sel).intersects(black_box(key)))
    });
}

criterion_group!(
    benches,
    bench_build,
    bench_parse,
    bench_slug,
    bench_intersects
);
criterion_main!(benches);
