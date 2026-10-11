//! Replay: the banner (#74) and the status strip during a replay.

use super::*;

/// The REPLAY banner (issue #74) is unmistakable and carries the capture's
/// own account of itself: the file, its selectors and base, when it was
/// captured, the drop ledger (O6 applied to a file), the transport — and
/// the scrubber's axis names its clock (RFC 09 §5.2's two-clock rule).
#[test]
fn the_replay_banner_says_everything() {
    use zengui::replay::ReplayState;

    let file = [
        r#"{"zrec":1,"selectors":["v1/**"],"base":"acme","captured_at":"2026-08-12T03:00:00Z"}"#,
        r#"{"key":"v1/h-0123456789ab/state/p/a","t":0,"bytes":"MQ=="}"#,
        r#"{"dropped":4}"#,
        r#"{"key":"v1/h-0123456789ab/state/p/b","t":2000000,"bytes":"Mg=="}"#,
    ]
    .join("\n");
    let mut state = ReplayState::load("incident.zrec", file.as_bytes()).expect("load");
    let _ = state.scrub_to(600_000);

    let mut ui = simulator::<Message, _, _>(zengui::view::replay::banner(&state));
    assert!(ui.find("REPLAY").is_ok(), "the mode must be unmistakable");
    assert!(
        ui.find(
            "incident.zrec — v1/** under base \"acme\", captured 2026-08-12T03:00:00Z · 2 row(s)"
        )
        .is_ok(),
        "the capture's own account: file, selectors, base, when, rows"
    );
    assert!(
        ui.find("capture dropped 4 sample(s) — partial view")
            .is_ok(),
        "the drop ledger must reach the banner (O6)"
    );
    assert!(ui.find("exit replay").is_ok());
    assert!(ui.find("live link off").is_ok());
    // The transport moved to the Activity dock's Replay tab (#183). The
    // banner stayed put, and it stayed put for a reason — a mode indicator
    // behind a tab is one that can lie about what the panes are showing.
    assert!(
        ui.find("0.6s / 2.0s (capture clock t)").is_err(),
        "the scrubber is a stream control, not part of the mode banner"
    );

    let mut ui = simulator::<Message, _, _>(zengui::view::replay::scrubber(&state, sp()));
    assert!(
        ui.find("0.6s / 2.0s (capture clock t)").is_ok(),
        "the scrubber axis names which clock it plots"
    );
    assert!(ui.find("play").is_ok(), "paused replay offers play");
}

/// While replaying, the status strip must not describe the link it is not
/// pumping (#74): the replay statement wins over any link state.
#[test]
fn the_strip_reports_replay_over_the_link() {
    use zengui::view::status::{self, Status};

    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec!["v1/**".to_string()];
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        namespace_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        scope_coverage: "",
        keys: 3,
        keys_evicted: 0,
        facts_cached: 3,
        facts_evicted: 0,
        identity: (0, 0),
        totals: Default::default(),
        contracts: Default::default(),
        presence: None,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: Default::default(),
        unreachable: false,
        prefs_note: None,
        replaying: true,
        retention: None,
    }));
    assert!(ui.find("REPLAY — live link off").is_ok());
    assert!(
        ui.find("observing 1 watch").is_err(),
        "a link nobody is pumping must not read as observing"
    );
}
