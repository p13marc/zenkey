//! What the render path costs, per frame and per tick (#178).
//!
//! Same contract as `zenkey-fleet/benches/fleet.rs`: numbers live in
//! `docs/bench-baseline.md`, and the point estimate is what we compare.
//!
//! ## What is here, and what deliberately is not
//!
//! Everything below is **renderer-free**: the work a `view::*` function does
//! *before* it builds a widget, plus the two per-tick joins that feed it. A
//! bench needing a GPU surface would not run in CI, and one going through
//! `iced_test`'s simulator would measure the simulator.
//!
//! So the widget-shaped half of #178 — `line_view`'s borrows, the toolbar's
//! pickers, the media handle, the two `canvas::Cache`s — is **not** benched
//! here. It is held by `tests/panes.rs`, which draws the same panes and fails
//! if a rendering changes, and by the compiler, which is what makes a borrow a
//! borrow. Saying so is better than a bench that implies a measurement it did
//! not take.
//!
//! Two cadences, as in the engine:
//!
//! - **per frame** — `echo/admits_*`, `hex/dump_1k`. `view` runs at frame rate.
//! - **per tick** — `roster/refresh_*`, `series/*`. Four times a second.

use std::time::Instant;

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

/// The canonical fixture origin, shared with the pane tests, the engine
/// benches and `spray`.
const HOST: &str = "h-3fa9c2d41b7e";

fn sample(key: &str, payload: &[u8]) -> zenkey_fleet::SampleView {
    zenkey_fleet::SampleView {
        key: key.to_string(),
        payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
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

// ── per frame ────────────────────────────────────────────────────────────

/// The echo pane's filter pass over a full ring.
///
/// The one #178 called worse than the issue said: `admits` parsed the *filter*
/// and the *line's key* on every line of every frame, so a 2,000-line ring did
/// 4,000 key-expression validations per frame. Both are paid once now — the
/// filter on a keystroke, the key on arrival.
fn bench_echo(c: &mut Criterion) {
    let mut ring = zengui::echo::EchoRing::new(2_000);
    for i in 0..2_000 {
        let class = if i % 2 == 0 { "state" } else { "telemetry" };
        ring.push(&sample(
            &format!("v1/{HOST}/{class}/sysinfo/g{}/k{i}", i / 100),
            br#"{"value":1}"#,
        ));
    }

    let mut group = c.benchmark_group("echo");
    // No filter: the floor — every line admitted, no key-expression work.
    group.bench_function("admits_2k_unfiltered", |b| {
        let view = zengui::view::echo::EchoView::new();
        b.iter(|| {
            black_box(
                ring.iter()
                    .filter(|l| view.admits(l, black_box(None)))
                    .count(),
            )
        })
    });
    // A key-expression filter admitting half: what the frame actually pays.
    group.bench_function("admits_2k_keyexpr", |b| {
        let mut view = zengui::view::echo::EchoView::new();
        view.set_key_filter(format!("v1/{HOST}/state/**"));
        b.iter(|| {
            black_box(
                ring.iter()
                    .filter(|l| view.admits(l, black_box(None)))
                    .count(),
            )
        })
    });
    // The substring filter shares the loop but not the parsing.
    group.bench_function("admits_2k_substring", |b| {
        let mut view = zengui::view::echo::EchoView::new();
        view.filter = "sysinfo".to_string();
        b.iter(|| {
            black_box(
                ring.iter()
                    .filter(|l| view.admits(l, black_box(None)))
                    .count(),
            )
        })
    });
    group.finish();
}

/// The detail pane's hex dump, at its bound.
fn bench_hex(c: &mut Criterion) {
    let bytes: Vec<u8> = (0..1024).map(|i| (i * 37 % 256) as u8).collect();
    c.bench_function("hex/dump_1k", |b| {
        b.iter(|| black_box(zengui::view::detail::hex_dump(black_box(&bytes))))
    });
}

// ── per tick ─────────────────────────────────────────────────────────────

/// The nodes pane's watched-freshness join.
///
/// Quadratic in (producers × watch selectors) by construction, which is why
/// the per-pair key-expression work mattered: `zenkey_fleet::skeleton::merge`
/// was fixed to validate borrowed `&keyexpr` once per tick, and this site was
/// missed.
fn bench_roster(c: &mut Criterion) {
    let mut stats = zenkey_fleet::stats::StatsTable::new();
    let now = Instant::now();
    for i in 0..40_000 {
        stats.record(
            &format!("v1/{HOST}/state/p{}/g{}/k{i}", i % 40, i / 100),
            64,
            None,
            now,
            None,
            None,
        );
    }
    let tree = zenkey_fleet::KeyTreeSnapshot::build(&stats);
    let watched: Vec<String> = (0..8).map(|i| format!("v1/*/state/p{i}/**")).collect();

    let mut roster = zengui::nodes::NodeRoster::default();
    let transitions: Vec<(String, bool)> = (0..40)
        .map(|i| (format!("v1/{HOST}/state/p{i}/alive"), true))
        .collect();
    roster.apply_transitions("", &transitions, now);

    c.bench_function("roster/refresh_40p_8w", |b| {
        b.iter(|| {
            roster.refresh(
                black_box(&tree),
                black_box(""),
                black_box(&watched),
                black_box(now),
            )
        })
    });
}

/// The detail pane's chart data — the two ring walks `series_data` does.
///
/// Per *tick* now; it used to be per frame, which is the change this bench
/// exists to make visible.
fn bench_series(c: &mut Criterion) {
    let mut rec = zengui::history::HistoryRecorder::new("v1/h/state/s/health", 600);
    for i in 0..600 {
        rec.observe(&sample(
            "v1/h/state/s/health",
            format!(r#"{{"cpu":{i}.5,"mem":{{"used":{i}}},"up":true}}"#).as_bytes(),
        ));
    }
    let newest = rec
        .ring
        .iter()
        .find_map(|e| e.value.as_ref())
        .expect("a document")
        .clone();

    let mut group = c.benchmark_group("series");
    group.bench_function("numeric_leaves", |b| {
        b.iter(|| black_box(zengui::series::numeric_leaves(black_box(&newest))))
    });
    group.bench_function("value_series_600", |b| {
        b.iter(|| black_box(zengui::series::value_series(black_box(&rec.ring), "cpu")))
    });
    group.finish();
}

/// Collapsing a subtree of the expansion set (#179).
///
/// A `BTreeSet` range rather than a scan of the whole set, so the cost is the
/// subtree's size and not the session's history.
fn bench_expansion(c: &mut Criterion) {
    c.bench_function("expansion/collapse_1k_of_10k", |b| {
        b.iter_batched(
            || {
                let mut e = zengui::expansion::Expansion::new();
                for i in 0..10_000 {
                    e.open(format!("v1/h-{i:04}/state/sysinfo"));
                }
                for i in 0..1_000 {
                    e.open(format!("v1/h-0000/state/sysinfo/leaf{i}"));
                }
                e
            },
            |mut e| e.toggle(black_box("v1/h-0000/state/sysinfo")),
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(
    benches,
    bench_echo,
    bench_hex,
    bench_roster,
    bench_series,
    bench_expansion
);
criterion_main!(benches);
