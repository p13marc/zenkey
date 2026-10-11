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
    // A version-1 file (#614, FL10b): what it excluded it never said, its
    // rows named v1 profiles zk2 does not read, and the replay resolves
    // offline — through no contract here, and no presence read.
    assert!(
        ui.find("excluded: not stated — a version-1 file predates the statement")
            .is_ok()
    );
    assert!(
        ui.find("QoS not recorded — its rows name v1 profiles, which zk2 does not read")
            .is_ok()
    );
    assert!(
        ui.find(
            "resolved offline: 0 revisions held, no presence read — the live catalog describes \
             another world"
        )
        .is_ok()
    );
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

/// A version-3 capture states what its selectors excluded (#614, FL10b; the
/// tooling guide's O5), and a row it carried without wire axes is counted
/// as "QoS not recorded" by row, never the whole file.
#[test]
fn a_version_three_banner_states_its_exclusions() {
    use zengui::replay::ReplayState;
    let file = [
        r#"{"zrec":3,"selectors":["**"],"base":"","captured_at":"2026-10-05T03:00:00Z","excluded":["@stream","@state","@op","@zk","@adv"]}"#,
        r#"{"key":"demo/a","t":0,"bytes":"MQ==","qos_axes":"data/drop/reliable"}"#,
        r#"{"key":"demo/b","t":1,"bytes":"Mg=="}"#,
    ]
    .join("\n");
    let state = ReplayState::load("v3.zrec", file.as_bytes()).expect("load");
    let mut ui = simulator::<Message, _, _>(zengui::view::replay::banner(&state));
    assert!(
        ui.find(
            "excluded: @stream, @state, @op, @zk, @adv — no selector names them, so no sample \
             under them is in this file"
        )
        .is_ok()
    );
    assert!(ui.find("QoS not recorded on 1 of 2 rows").is_ok());

    let every = [
        r#"{"zrec":3,"selectors":["**","@stream/**","@state/**","@op/**","@zk/**","@adv/**"],"base":"","captured_at":"x","excluded":[]}"#,
        r#"{"key":"demo/a","t":0,"bytes":"MQ==","qos_axes":"data/drop/reliable"}"#,
    ]
    .join("\n");
    let state = ReplayState::load("all.zrec", every.as_bytes()).expect("load");
    let mut ui = simulator::<Message, _, _>(zengui::view::replay::banner(&state));
    assert!(
        ui.find("excluded: nothing — the selectors name every verbatim chunk")
            .is_ok()
    );
    assert!(
        ui.find("QoS not recorded on 1 of 2 rows").is_err(),
        "every row recorded its axes"
    );
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
