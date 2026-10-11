//! The traffic tab (#542).

use super::*;

/// #542: the Traffic tab's tiles never turn "not asked" into a number. With
/// no session every tile says why it has none; with one, the three ways a
/// sample goes missing are three numbers, never one sum (O6), and a share is
/// labelled by what it divides.
#[test]
fn the_traffic_tab_never_reports_a_number_it_did_not_take() {
    use zengui::series::Series;
    use zengui::traffic::{TrafficSort, top};
    use zengui::view::traffic::{TrafficData, section};

    let cache = iced::widget::canvas::Cache::new();
    let empty = Series::new();
    let base = |session: bool, watched: usize| TrafficData {
        table: None,
        sort: TrafficSort::Rate,
        total_rate: &empty,
        cache: &cache,
        session,
        watched,
        replaying: false,
        totals: Default::default(),
        keys: 0,
        keys_evicted: 0,
        keys_unwatched: 0,
        lagged: 0,
        coalesced: 0,
        evicted: 0,
        sp: sp(),
    };

    let mut ui = simulator::<Message, _, _>(section(base(false, 0)));
    assert!(ui.find("not asked — no session yet").is_ok());
    assert!(ui.find("—").is_ok(), "a tile with no number says so");
    assert!(ui.find("0").is_err(), "never-asked must not read as zero");

    let mut ui = simulator::<Message, _, _>(section(base(true, 0)));
    assert!(
        ui.find("nothing observed by choice — watch a subtree or the scope")
            .is_ok()
    );

    let mut stats = zenkey_fleet::StatsTable::new();
    stats.record(REGISTERED, 64, None, Instant::now(), None, None);
    let observed = KeyTreeSnapshot::build(&stats);
    let table = top(&observed, TrafficSort::Rate, 50);
    let mut ui = simulator::<Message, _, _>(section(TrafficData {
        table: Some(&table),
        keys: 1,
        keys_evicted: 3,
        lagged: 2,
        coalesced: 5,
        evicted: 7,
        ..base(true, 1)
    }));
    assert!(
        ui.find("lagged by the broadcast · coalesced 5 · evicted 7")
            .is_ok(),
        "three counters, three numbers"
    );
    assert!(ui.find("2").is_ok(), "the lag is the tile's number");
    assert!(
        ui.find("top 1 of 1 key — ranked by rate · share of the retained keys' rate")
            .is_ok(),
        "with keys retired by the bound, the share says what it divides"
    );
    assert!(ui.find(REGISTERED).is_ok(), "the ranked key is on screen");
}
