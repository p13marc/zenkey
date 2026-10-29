//! `timing.v1` in the Inspector (#908, MC): a timed stream's declared
//! period, deadline and lifespan beside its type, each the contract's, and
//! the window's own view of its deadline and its period over the recorded
//! history — each the model's answer, with the owner's state the presence
//! link gave at each miss.

use super::*;

fn scene(name: &str) -> zengui::app::Zengui {
    let s = common::scenes::ALL
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("scene {name}"));
    (s.build)(zengui::prefs::ThemeChoice::Dark)
}

/// `robot1/teleop`'s `cmd` at 250 ms: its 100 ms deadline missed between
/// every two samples, the owner present at each miss (the presence link's
/// word): the finding, a miss never "down"; its period not kept; its
/// lifespan said not judged here, for want of ε.
#[test]
fn the_inspector_reads_a_timed_stream_by_timing_v1() {
    let app = scene("inspector-timing-missed");
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    for text in [
        "twist_cmd.v1.Twist",
        "deadline: no (missed)",
        "period: no (not_kept)",
        "nothing arrived within its deadline while its owner was present",
        "a miss is never \"down\": nothing arrived within the deadline, as this window saw it; \
         whether the owner is gone is presence's word (§2.5)",
        "lifespan: not judged here — it takes the deployment's stated clock bound, which zengui \
         is not told (§2.6); zenctl timing --clock-bound judges it",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    // 29 samples 250 ms apart: 28 misses, each due 100 ms after an
    // arrival and reported at the next, 150 ms later; the latest four
    // listed, the count beside them.
    for text in [
        "missed: due at 6850 ms, reported 150 ms after, its owner present",
        "missed: due at 6100 ms, reported 150 ms after, its owner present",
        "the latest 4 of 28 misses",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
}

/// `robot1/nav`'s position: period and lifespan declared, no deadline —
/// not asked, never a pass — and the period not kept at 250 ms.
#[test]
fn a_stream_with_no_deadline_is_not_asked_one() {
    let app = scene("inspector-timing-period");
    let mut ui = simulator::<Message, _, _>(app.view(iced::window::Id::unique()));
    for text in ["deadline: not_asked (no_deadline)", "period: no (not_kept)"] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    assert!(ui.find("missed: due").is_err(), "no deadline, no miss");
}

/// A replay's rows are no arrival of the live member: nothing is judged,
/// and the declared values still read.
#[test]
fn a_replay_reads_no_arrival() {
    let world = common::World::timed();
    let resolved = world.resolve(common::TWIST_CMD).resolved.expect("resolves");
    let rec = HistoryRecorder::new(common::TWIST_CMD, 8);
    let read = zengui::timing::read(
        &resolved,
        &rec.ring,
        &zengui::timing::OwnerTrack::default(),
        false,
        true,
        Instant::now(),
    )
    .expect("a timed stream");
    assert!(read.unobservable.is_some_and(|w| w.contains("replay")));
    assert!(read.deadline.is_none());
    let d = read.declared().expect("declared");
    assert_eq!(d.deadline, Some(std::time::Duration::from_millis(100)));
}

/// A ring that dropped older entries says its window starts at the oldest
/// it kept, never at the slot's start (review-bot on PR #917).
#[test]
fn an_evicting_ring_says_where_its_window_starts() {
    let world = common::World::timed();
    let resolved = world.resolve(common::TWIST_CMD).resolved.expect("resolves");
    let mut rec = HistoryRecorder::new(common::TWIST_CMD, 2);
    let t0 = Instant::now();
    for i in 0..3u64 {
        rec.observe(&common::sample(
            common::TWIST_CMD,
            b"{}",
            "application/json",
            t0 + std::time::Duration::from_millis(20 * i),
        ));
    }
    let read = zengui::timing::read(
        &resolved,
        &rec.ring,
        &zengui::timing::OwnerTrack::default(),
        false,
        false,
        t0 + std::time::Duration::from_millis(60),
    )
    .expect("a timed stream");
    assert_eq!(read.evicted, 1);
    assert_eq!(read.arrivals, 2, "the window holds what the ring kept");
}
