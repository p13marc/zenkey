//! The per-key statistics table (issue #203).
//!
//! `stats` feeds every `hz`/`bw` figure, both frontends' key trees, and the
//! bound accounting that RFC 09 §5.1 **O6** obliges an observer to report. It
//! had no test file of its own, which is the part worth naming: **untested
//! eviction is untested honesty**. A table that retires keys correctly but
//! forgets to count it looks identical, from the outside, to one that never
//! retired anything — and the whole point of O6 is that those two must not
//! look alike.
//!
//! No bus: the table is pure, and `now` is injected so nothing here sleeps.

use std::time::{Duration, Instant};

use zenkey_fleet::model::stats::{StampClass, StatsTable};

#[test]
fn a_bounded_table_reports_what_it_retired() {
    let mut t = StatsTable::with_capacity(4);
    let t0 = Instant::now();

    assert_eq!(t.evicted(), 0, "nothing retired before anything arrived");
    assert_eq!(
        t.max_keys(),
        4,
        "the bound is readable, so it can be stated"
    );

    for i in 0..4 {
        t.record(
            &format!("v1/h-a/telemetry/p/k{i}"),
            8,
            None,
            t0 + Duration::from_millis(i),
            None,
            None,
        );
    }
    assert_eq!(t.len(), 4);
    assert_eq!(t.evicted(), 0, "a full table has still retired nothing");

    // One more distinct key than the bound allows.
    t.record(
        "v1/h-a/telemetry/p/k4",
        8,
        None,
        t0 + Duration::from_secs(1),
        None,
        None,
    );
    assert_eq!(t.len(), 4, "the bound holds");
    assert_eq!(
        t.evicted(),
        1,
        "and the cost of holding it is counted, not swallowed (O6)"
    );

    // The bound is on *distinct keys*, not on samples: re-recording a known
    // key must never retire anything.
    let before = t.evicted();
    for _ in 0..50 {
        t.record(
            "v1/h-a/telemetry/p/k4",
            8,
            None,
            t0 + Duration::from_secs(2),
            None,
            None,
        );
    }
    assert_eq!(t.len(), 4);
    assert_eq!(
        t.evicted(),
        before,
        "50 samples on a known key are not 50 evictions"
    );
}

#[test]
fn a_sequence_gap_is_counted_and_a_contiguous_run_is_not() {
    let mut t = StatsTable::new();
    let now = Instant::now();
    let key = "v1/h-a/telemetry/p/m";

    for sn in 1..=5 {
        t.record(key, 4, Some(sn), now, None, None);
    }
    assert_eq!(
        t.get(key).unwrap().sn_gaps,
        0,
        "a contiguous run has lost nothing"
    );

    // 6, 7, 8 never arrived.
    t.record(key, 4, Some(9), now, None, None);
    assert_eq!(
        t.get(key).unwrap().sn_gaps,
        3,
        "three missing sequence numbers are three gaps, not one event"
    );

    // A publisher that attaches no SourceInfo yields no sequence numbers at
    // all — which is zero gaps *observed*, never proof of losslessness. The
    // report's own doc says so; this pins that the counter does not invent.
    let quiet = "v1/h-a/telemetry/p/quiet";
    for _ in 0..5 {
        t.record(quiet, 4, None, now, None, None);
    }
    assert_eq!(t.get(quiet).unwrap().sn_gaps, 0);
}

/// Deep-review D6: the gap arithmetic survives the `u32` wrap. The old
/// `cur > prev + 1` overflowed in debug at `u32::MAX` and read the wrap as
/// a ~2^32 gap in release; wrapping arithmetic makes `u32::MAX → 0` the
/// next sample, a gap across the wrap the real count, and a backwards jump
/// a counted *reset*, never invented loss.
#[test]
fn sequence_arithmetic_survives_the_wrap_and_counts_resets_apart() {
    let mut t = StatsTable::new();
    let now = Instant::now();

    // The wrap itself is contiguous: no gap, no reset, no overflow panic.
    let key = "v1/h-a/telemetry/p/wrap";
    t.record(key, 4, Some(u32::MAX), now, None, None);
    t.record(key, 4, Some(0), now, None, None);
    let s = t.get(key).unwrap();
    assert_eq!(
        (s.sn_gaps, s.sn_resets),
        (0, 0),
        "u32::MAX → 0 lost nothing"
    );

    // A gap *across* the wrap counts what was actually missed.
    let key = "v1/h-a/telemetry/p/wrapgap";
    t.record(key, 4, Some(u32::MAX - 1), now, None, None);
    t.record(key, 4, Some(2), now, None, None);
    let s = t.get(key).unwrap();
    assert_eq!(
        (s.sn_gaps, s.sn_resets),
        (3, 0),
        "u32::MAX, 0 and 1 never arrived — three gaps, not four billion"
    );

    // A backwards jump is a publisher restart: one reset, zero loss.
    let key = "v1/h-a/telemetry/p/restart";
    t.record(key, 4, Some(500), now, None, None);
    t.record(key, 4, Some(0), now, None, None);
    let s = t.get(key).unwrap();
    assert_eq!(
        (s.sn_gaps, s.sn_resets),
        (0, 1),
        "a restart is a reset, not invented loss (O6: the kinds stay apart)"
    );
    // …and numbering picks up cleanly from the new epoch.
    t.record(key, 4, Some(1), now, None, None);
    t.record(key, 4, Some(3), now, None, None);
    let s = t.get(key).unwrap();
    assert_eq!(
        (s.sn_gaps, s.sn_resets),
        (1, 1),
        "post-reset gaps still count"
    );

    // A duplicate is neither loss nor a reset.
    let key = "v1/h-a/telemetry/p/dup";
    t.record(key, 4, Some(7), now, None, None);
    t.record(key, 4, Some(7), now, None, None);
    let s = t.get(key).unwrap();
    assert_eq!(
        (s.sn_gaps, s.sn_resets),
        (0, 0),
        "a retransmit is not a gap"
    );
}

/// #213 at the table level: the three stamp populations are summarised apart,
/// and an unstamped sample is counted rather than folded in as zero.
#[test]
fn the_stamp_populations_reach_the_summary_separately() {
    let mut t = StatsTable::new();
    let now = Instant::now();
    let key = "v1/h-a/state/p/health";
    let router = zenoh::time::TimestampId::rand();

    for us in [100, 200, 300] {
        t.record(key, 4, None, now, Some((us, StampClass::SelfStamped)), None);
    }
    for us in [8_000, 9_000] {
        t.record(
            key,
            4,
            None,
            now,
            Some((us, StampClass::Foreign)),
            Some(router),
        );
    }
    t.record(key, 4, None, now, None, None);

    let s = t.get(key).expect("the key was recorded");
    assert_eq!(s.unstamped, 1, "no latency is not zero latency");

    let lat = s.latency().expect("something was stamped");
    let own = lat.self_stamped.expect("the publisher-stamped population");
    let far = lat.foreign.expect("the router-stamped population");
    assert_eq!((own.samples, far.samples), (3, 2));
    assert!(
        own.max_us < far.min_us,
        "two clocks, two distributions: {own:?} vs {far:?}"
    );
    assert_eq!(lat.stampers, vec![router.to_string()]);

    // Both populations are named in the rendering, so neither can be read as
    // "the latency" on its own.
    let labels: Vec<&str> = lat.populations().iter().map(|(l, _)| *l).collect();
    assert_eq!(labels, ["publisher-stamped", "router-stamped"]);
}

/// Retiring a watch retires the keys only it covered, and says how many —
/// the other half of the O6 ledger, beside the capacity bound.
#[test]
fn dropping_a_watch_retires_only_the_keys_it_alone_covered() {
    let mut t = StatsTable::new();
    let now = Instant::now();
    t.record("v1/h-a/telemetry/p/one", 4, None, now, None, None);
    t.record("v1/h-b/telemetry/p/two", 4, None, now, None, None);

    // Dropping a watch that covered `h-a`, while a watch covering `h-b`
    // stands: only the first key loses its last cover.
    let retired = t.retire_unwatched("v1/h-a/**", &["v1/h-b/**".to_string()]);
    assert_eq!(retired, 1, "one key lost its last cover");
    assert_eq!(t.unwatched(), 1, "and the table says so (O6)");
    assert!(t.get("v1/h-a/telemetry/p/one").is_none());
    assert!(
        t.get("v1/h-b/telemetry/p/two").is_some(),
        "a key another watch still covers stays"
    );
}
