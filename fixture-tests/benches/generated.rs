//! Baseline benchmarks for the generated registry surface (issue #17).
//!
//! Same contract as `zenkey/benches/keys.rs`: recorded before the 0.3 codegen
//! rewrite; numbers in `docs/bench-baseline.md`.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use zenkey::grammar::Class;
use zenkey::origin::{HostId, LocalOrigin};
use zenkey_fixture_tests::registry::{self, netring};

fn host() -> LocalOrigin {
    LocalOrigin::from_host_id(HostId::parse("h-3fa9c2d41b7e").unwrap())
}

fn bench_generated(c: &mut Criterion) {
    let origin = host();
    let subject = netring::Subject::flow_red("p95_ms");
    c.bench_function("generated/key_build", |b| {
        b.iter(|| netring::key(black_box(&origin), black_box(&subject)))
    });
    c.bench_function("generated/subject_parse_hit", |b| {
        b.iter(|| netring::Subject::parse(Class::Telemetry, black_box(&["flow", "red", "p95_ms"])))
    });
    c.bench_function("generated/subject_parse_miss", |b| {
        b.iter(|| netring::Subject::parse(Class::Telemetry, black_box(&["flow", "bogus"])))
    });
    c.bench_function("generated/parse_metric", |b| {
        b.iter(|| netring::Subject::parse_metric(black_box("flow/red/p95_ms")))
    });
    c.bench_function("generated/any_subject_dispatch", |b| {
        b.iter(|| {
            registry::parse_subject(
                black_box("netring"),
                Class::Telemetry,
                black_box(&["flow", "red", "p95_ms"]),
            )
        })
    });
}

criterion_group!(benches, bench_generated);
criterion_main!(benches);
