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
//! - **per tick** — `roster/refresh_*`, `series/*`, `tree/*`. Four times a
//!   second.
//!
//! ## The tree group had no numbers at all before #177
//!
//! `reflatten` is `skeleton::merge` **plus** a flatten, both unconditional at
//! tick cadence, and neither had ever been measured from this side. The engine
//! has `skeleton/merge_10k`, but `docs/bench-baseline.md` marks that row `~`:
//! the same code read 19.5 ms and 74.8 ms on this machine. So #177's headline —
//! "50,000 tree rows four times a second" — was a count, never a duration.
//!
//! Fixtures are built in `iter_batched`'s setup, never inside `b.iter`.
//! `docs/bench-baseline.md` records `stats/retire_unwatched_1k` being
//! invalidated by exactly that mistake, and a 50k-key fixture would swamp the
//! thing being measured far worse than a 1k one did.

use std::sync::Arc;
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

/// The tree pipeline: what `reflatten` costs, and what the row cap does not
/// bound (#177, #249).
///
/// `TREE_KEYS` mirrors `soak_flatten_50k_keys` (`view/tree.rs`) and
/// `zenkey-fleet/benches/fleet.rs`'s `synth_key`: 100 keys per group, so the
/// tree has realistic fan-out rather than one flat level.
fn bench_tree(c: &mut Criterion) {
    const TREE_KEYS: usize = 50_000;

    fn keys(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("v1/{HOST}/telemetry/synth/g{}/k{i}", i / 100))
            .collect()
    }

    /// The observed half, and the empty skeleton `reflatten` merges against
    /// when no registry has answered — which is the common case on a foreign
    /// bus, and the one `app.rs` rebuilds from scratch on every call.
    fn inputs(n: usize) -> (zenkey_fleet::Skeleton, zenkey_fleet::KeyTreeSnapshot) {
        let mut stats = zenkey_fleet::stats::StatsTable::new();
        let now = Instant::now();
        for k in keys(n) {
            stats.record(&k, 64, None, now, None, None);
        }
        let observed = zenkey_fleet::KeyTreeSnapshot::build(&stats);
        let skel = zenkey_fleet::Skeleton::build(
            "",
            &zenkey_fleet::SliceSet::default(),
            &std::collections::BTreeMap::new(),
            None,
        );
        (skel, observed)
    }

    /// Every prefix of every key — the worst case, and what the tree looks
    /// like after `NodesMsg::ShowInTree` or a doctor finding walks it open.
    fn expand_all(n: usize) -> std::collections::BTreeSet<String> {
        let mut all = std::collections::BTreeSet::new();
        for k in keys(n) {
            let mut acc = String::new();
            for chunk in k.split('/') {
                if !acc.is_empty() {
                    acc.push('/');
                }
                acc.push_str(chunk);
                all.insert(acc.clone());
            }
        }
        all
    }

    let watched = ["**".to_string()];
    let (skel, observed) = inputs(TREE_KEYS);
    let merged = zenkey_fleet::skeleton::merge(&skel, &observed, &watched);
    let expanded = expand_all(TREE_KEYS);
    let now = Instant::now();

    let mut group = c.benchmark_group("tree");
    // Criterion's default 100 samples would take minutes at this size.
    group.sample_size(10);

    // The dominant term #177 barely mentions: `reflatten` pays this *before*
    // it flattens anything, on every one of its nine call sites.
    group.bench_function("merge_50k", |b| {
        b.iter(|| {
            black_box(zenkey_fleet::skeleton::merge(
                black_box(&skel),
                black_box(&observed),
                black_box(&watched),
            ))
        })
    });

    // The cold rebuild — today's per-tick cost, and after #177 the cost of a
    // key-set change or an expand. The split moved work from the tick to the
    // frame rather than removing it, so this number is deliberately unchanged;
    // what changes is how often it is paid.
    group.bench_function("flatten_50k_expanded", |b| {
        b.iter(|| {
            black_box(zengui::view::tree::flatten(
                black_box(&merged),
                "",
                black_box(&expanded),
                60_000,
                now,
            ))
        })
    });

    // #249, half one: retention is decided bottom-up, so every node gets a
    // full `TreeRow` — four owned `String`s — before the walk knows whether to
    // keep it. A query matching nothing builds and discards all of them, per
    // keystroke.
    group.bench_function("search_50k_no_match", |b| {
        b.iter(|| {
            black_box(zengui::view::tree::search_flatten(
                black_box(&merged),
                "",
                black_box("zzzznomatchzzzz"),
                60_000,
                now,
            ))
        })
    });

    // #249, half two: the opposite failure. `rows` grows to node count
    // unchecked and the cap is applied afterwards, so the memory bound
    // `app.rs` documents does not hold here at all.
    group.bench_function("search_50k_all_match", |b| {
        b.iter(|| {
            black_box(zengui::view::tree::search_flatten(
                black_box(&merged),
                "",
                black_box("k"),
                1_000,
                now,
            ))
        })
    });

    // **The O(1)-in-rows evidence, and it needs both sizes.** One size proves
    // nothing; two that agree prove the cost is independent of the tree.
    // Criterion cannot count allocations, so this pair plus the
    // `shape_reused`/`shape_rebuilt` unit tests are what stand in for #177's
    // "allocation is O(1)" acceptance, which is not a thing a bench can say.
    let (small_skel, small_observed) = inputs(1_000);
    let small_merged = zenkey_fleet::skeleton::merge(&small_skel, &small_observed, &watched);
    let small_expanded = expand_all(1_000);
    let small_snapshot = Arc::new(small_observed);
    let big_snapshot = Arc::new(observed);

    for (label, m, e, snap) in [
        ("1k", &small_merged, &small_expanded, &small_snapshot),
        ("50k", &merged, &expanded, &big_snapshot),
    ] {
        group.bench_function(format!("retarget_{label}"), |b| {
            let mut flat = zengui::view::tree::flatten(m, "", e, 60_000, now);
            b.iter(|| black_box(flat.retarget(Arc::clone(black_box(snap)), now)))
        });
        // Materialising one screenful — the work that moved from the tick to
        // the frame. Forty rows out of a thousand and forty out of fifty
        // thousand must cost the same.
        group.bench_function(format!("window_rows_40_of_{label}"), |b| {
            let mut flat = zengui::view::tree::flatten(m, "", e, 60_000, now);
            flat.retarget(Arc::clone(snap), now);
            b.iter(|| {
                for i in 0..40.min(flat.rows.len()) {
                    black_box(flat.row(black_box(i)));
                }
            })
        });
    }

    // The pivot cold path — three full materialisations of the same data, and
    // the number that sizes the arena follow-up.
    group.bench_function("pivot_50k_producer", |b| {
        b.iter(|| {
            black_box(zengui::view::tree::pivot_flatten(
                black_box(&merged),
                "",
                zengui::view::tree::Pivot::Producer,
                black_box(&expanded),
                "",
                60_000,
                now,
            ))
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_echo,
    bench_hex,
    bench_roster,
    bench_series,
    bench_expansion,
    bench_tree
);
criterion_main!(benches);
