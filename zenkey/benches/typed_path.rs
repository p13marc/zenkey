//! Spike S9's budget (#619): the typed path must cost only what wraps
//! zenoh's own `put`. Spike S9 measured 126 ns per control message for its
//! typed layer; here `writer/put` (the contract's QoS and `Encoding`, set
//! once at declaration) is measured against `raw/put`, a publisher declared
//! by hand with the same settings, on one session with no subscriber.
//!
//! `cargo bench -p zenkey --bench typed_path`. CI only builds it
//! (`cargo bench --no-run`).

use criterion::{Criterion, criterion_group, criterion_main};
use zenkey::model::contract::load_path;
use zenkey::model::template::Bindings;
use zenkey::{Implementation, ServiceBuilder, ServiceConfig};

fn typed_path(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("a runtime");
    let (writer, raw) = rt.block_on(async {
        let mut cfg = zenoh::Config::default();
        cfg.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        let s = zenoh::open(cfg).await.expect("a session");
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/contracts/sensor.v1.toml");
        let contract = load_path(&path).contract.expect("sensor.v1 loads");
        let iface = contract.iface.clone();
        let mut b = ServiceBuilder::new(&s, ServiceConfig::new("bench/sensor".parse().unwrap()));
        b.implement(Implementation::new(contract)).unwrap();
        let w = b
            .declare_writer(&iface, "stream/fast", &Bindings::new())
            .await
            .unwrap();
        let raw = s
            .declare_publisher("zk2/bench/raw/sensor.v1/stream/fast")
            .encoding(zenoh::bytes::Encoding::TEXT_PLAIN)
            .priority(zenoh::qos::Priority::DataHigh)
            .express(true)
            .await
            .unwrap();
        // Keep the session alive with the handles.
        std::mem::forget(s);
        (w, raw)
    });
    let payload = vec![0u8; 64];
    c.bench_function("writer/put", |b| {
        b.iter(|| rt.block_on(writer.put(payload.clone())).unwrap())
    });
    c.bench_function("raw/put", |b| {
        b.iter(|| {
            rt.block_on(async { raw.put(payload.clone()).await })
                .unwrap()
        })
    });
}

criterion_group!(benches, typed_path);
criterion_main!(benches);
