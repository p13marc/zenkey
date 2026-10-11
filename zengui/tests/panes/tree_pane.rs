//! The key tree: the registration badges, a narrow tree, density, unresolved
//! and foreign keys, the empty tree, the projection cache's bound and the
//! budget join.

use super::*;

/// #193's acceptance: the five `Registration` states render five *distinct*
/// glyph + word pairs. The glyph comes from the tone type — no call site can
/// omit it or give two states the same one — so colour is structurally never
/// the only carrier.
#[test]
fn the_five_registration_states_render_five_distinct_glyph_word_pairs() {
    use zengui::keyfacts::{Registration, SubjectFacts};
    use zengui::view::tree::{registration_label, tone};

    let states = [
        Registration::Registered(Box::new(SubjectFacts {
            path: "disk/{mount}/used".into(),
            type_name: "TelemetryPoint".into(),
            vars: vec![],
            unit: None,
            kind: None,
            buckets: None,
            qos: None,
            encoding: None,
            when: None,
            gate_note: None,
            exposure: None,
            ttl_s: None,
            rate: None,
            cardinality: None,
            since: None,
            description: None,
        })),
        Registration::Unregistered,
        Registration::NoSliceForProducer,
        Registration::Unknown,
        Registration::NotApplicable,
    ];

    let pairs: Vec<(&str, &str)> = states
        .iter()
        .map(|r| (tone(r).glyph(), registration_label(r)))
        .collect();
    for (i, a) in pairs.iter().enumerate() {
        for b in &pairs[i + 1..] {
            assert_ne!(
                a, b,
                "two registration states render the same glyph + word pair"
            );
        }
    }

    // …and each pair actually reaches the screen, through the same
    // `tone_badge` every pane uses.
    let badges = iced::widget::Column::with_children(
        states
            .iter()
            .map(|r| zengui::view::kit::tone_badge::<Message>(tone(r), registration_label(r))),
    );
    let mut ui = simulator::<Message, _, _>(badges);
    for &(glyph, word) in &pairs {
        assert!(
            ui.find(glyph).is_ok(),
            "glyph {glyph:?} must reach the screen"
        );
        assert!(ui.find(word).is_ok(), "word {word:?} must reach the screen");
    }
}

/// The registration tri-state must actually reach the screen, and its states
/// must read differently (RFC 09 §5.1 O4).
#[test]
fn the_tree_renders_the_registration_state() {
    let keys = [REGISTERED, UNREGISTERED, FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    assert!(
        ui.find("registered").is_ok(),
        "a registered subject should be badged"
    );
    assert!(
        ui.find("unregistered").is_ok(),
        "an unregistered subject should be badged differently"
    );
    // The registry-declared type name is the payoff of the whole overlay.
    assert!(
        ui.find("TelemetryPoint").is_ok(),
        "the declared type should be shown"
    );
    // The positional labels for the conforming subtree.
    for role in ["version", "origin", "class", "producer", "subject"] {
        assert!(ui.find(role).is_ok(), "missing role label {role:?}");
    }
}

/// #557: a narrow Locator shows less of a row, never two rows at once. At
/// 240px (300px since #566 halved a narrow pane's indent, and gave the
/// badge its room back) the deep leaves are in the narrow tier — the counts collapse to the
/// rate, on one line, inside the pane — and a badge with no room is left
/// out whole and *counted* (`+1`), never cut and never silently gone. At the
/// default width the whole row is back.
#[test]
fn a_narrow_tree_keeps_each_row_on_its_line() {
    use zengui::view::tokens::CAPTION_LINE;

    let keys = [REGISTERED, UNREGISTERED];
    let mut stats = StatsTable::new();
    let t0 = Instant::now();
    for k in &keys {
        for i in 0..5u64 {
            stats.record(
                k,
                512,
                None,
                t0 + std::time::Duration::from_millis(200 * i),
                None,
                None,
            );
        }
    }
    let skel = zenkey_fleet_v1::Skeleton::build("", &SliceSet::default(), &BTreeMap::new(), None);
    let snap = zenkey_fleet_v1::model::skeleton::merge(
        &skel,
        &KeyTreeSnapshot::build(&stats),
        &["**".to_string()],
    );
    let flat = tree::flatten(&snap, "", &expand_all(&keys), 500, Instant::now());
    let facts = index(&keys, true);
    let leaf = (0..flat.rows.len())
        .map(|i| flat.row(i))
        .find(|r| r.target.as_deref() == Some(REGISTERED))
        .expect("the registered leaf is a row");
    let rate = zengui::view::kit::human_rate(leaf.rate_hz);
    let wide = format!(
        "{} · {} · {rate}",
        leaf.count,
        zengui::view::kit::human_bytes(leaf.bytes)
    );
    assert_ne!(
        rate, "—",
        "the leaf must carry a rate for this test to mean anything"
    );

    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let data = tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    };

    let width = 240.0;
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (width, 600.0),
        tree::pane(data),
    );
    let counts = ui
        .find(rate.as_str())
        .expect("the narrow tier keeps the rate");
    let b = counts.bounds();
    assert!(
        b.height <= CAPTION_LINE + 0.5,
        "the counts are one caption line tall, got {}",
        b.height
    );
    assert!(
        b.x + b.width <= width,
        "the counts are laid out inside the pane, not pushed past it: {b:?}"
    );
    assert!(ui.find(wide.as_str()).is_err(), "the bytes go first");
    assert!(
        ui.find("TelemetryPoint").is_err(),
        "the type name is the lowest priority"
    );
    assert!(
        ui.find("registered").is_err() && ui.find("unregistered").is_err(),
        "no room for a badge at this depth"
    );
    assert!(
        ui.find("+1").is_ok(),
        "…so it is counted, not silently dropped (an absent badge reads as not asked)"
    );

    let mut ui = simulator::<Message, _, _>(tree::pane(data));
    assert!(
        ui.find(wide.as_str()).is_ok(),
        "the default width is the whole row"
    );
    assert!(ui.find("registered").is_ok());
    assert!(ui.find("unregistered").is_ok());
    assert!(ui.find("+1").is_err(), "nothing is held back when it fits");
}

/// #192's acceptance, from the render side: density moves the grid and the
/// row heights, never the words or their sizes. The same pane at Comfortable
/// and Compact makes the same claims — and the type side is pinned
/// structurally in `view/tokens.rs` (`Spacing` has no font field; every
/// `.size(` is gated onto `font::`, whose constants take no density).
#[test]
fn density_changes_the_grid_never_the_claims() {
    use zengui::prefs::Density;

    let keys = [REGISTERED, UNREGISTERED, FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    for density in Density::ALL {
        let verdicts = zengui::verdict::VerdictCache::default();
        let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
            sp: Spacing::of(density),
            flat: &flat,
            pivot: tree::Pivot::Chunks,
            search: "",
            viewport: Default::default(),
            facts: &facts,
            verdicts: &verdicts,
            budgets: None,
            watches: tree::Watches {
                mine: &watches,
                seeding: &watches,
            },
            selected: None,
        }));
        for claim in ["registered", "unregistered", "TelemetryPoint"] {
            assert!(
                ui.find(claim).is_ok(),
                "{claim:?} must survive {} density",
                density.label()
            );
        }
    }
}

/// Before a registry is loaded, nothing may be badged either way — that would
/// report a verdict never obtained (RFC 09 §5.1 O4).
#[test]
fn an_unresolved_tree_claims_neither_way() {
    let keys = [REGISTERED];
    let (flat, facts) = render(&keys, false);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    assert!(
        ui.find("unregistered").is_err(),
        "a key we never checked must not read as unregistered"
    );
    assert!(ui.find("registered").is_err(), "…nor as registered");
    // It is still a fully labelled conforming key — only the registry verdict
    // is withheld.
    assert!(ui.find("producer").is_ok());
}

/// A foreign key is rendered, and rendered *plainly* — no invented labels.
#[test]
fn foreign_keys_render_without_convention_labels() {
    let keys = [FOREIGN];
    let (flat, facts) = render(&keys, true);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    for role in ["version", "origin", "class", "producer", "subject"] {
        assert!(
            ui.find(role).is_err(),
            "a foreign key must not be labelled {role:?}"
        );
    }
    // …and no registry verdict is invented for it either.
    assert!(ui.find("unregistered").is_err());
}

/// The empty tree must explain itself rather than implying the bus is quiet
/// (RFC 05 §3.1).
#[test]
fn the_empty_tree_explains_itself() {
    let flat = tree::Flattened::empty();
    let facts = FactsIndex::default();
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        sp: sp(),
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        facts: &facts,
        verdicts: &verdicts,
        budgets: None,
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
    }));

    assert!(ui.find("Nothing observed yet").is_ok());
    // Selectors match a widget's whole text, so this is the full disclaimer.
    assert!(
        ui.find(
            "An empty tree is not a verdict about the bus (RFC 05 §3.1) — \
             it may simply mean nothing has published on the current scope."
        )
        .is_ok(),
        "the empty state must disclaim, not just name the emptiness"
    );
}

/// #107's "the bound is not a silent one": the projection cache's retirement
/// count reaches the strip, and reads as its own number rather than being
/// folded into the key count. Two bounds, two populations, two sentences (O6).
#[test]
fn the_projection_cache_discloses_its_bound() {
    use zengui::prefs::Prefs;
    use zengui::view::status::{self, Status};

    let _ = Prefs::default();
    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec![];
    let source = status::SliceSource::None;
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        base_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        scope_coverage: "",
        keys: 120,
        keys_evicted: 7,
        facts_cached: 118,
        facts_evicted: 312,
        totals: Default::default(),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: Default::default(),
        unreachable: false,
        prefs_note: None,
        replaying: false,
        retention: None,
    }));
    assert!(
        ui.find(status::facts_text(118, 312).as_str()).is_ok(),
        "the cache's bound must be visible, not merely enforced"
    );
    assert!(
        ui.find(status::keys_text(120, 7).as_str()).is_ok(),
        "and the key table's own count is still its own"
    );
    assert_ne!(
        status::facts_text(118, 312),
        status::keys_text(118, 312),
        "the two bounds must not read as one number"
    );
}

/// A quiet cache has nothing to disclose: the key count already states the
/// population, so an untripped bound must not add a line.
#[test]
fn an_untripped_cache_bound_says_nothing() {
    use zengui::view::status::{self, Status};

    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec![];
    let source = status::SliceSource::None;
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        base_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        scope_coverage: "",
        keys: 12,
        keys_evicted: 0,
        facts_cached: 12,
        facts_evicted: 0,
        totals: Default::default(),
        slices: &source,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: Default::default(),
        unreachable: false,
        prefs_note: None,
        replaying: false,
        retention: None,
    }));
    assert!(ui.find(status::facts_text(12, 0).as_str()).is_err());
}

/// #221: the tree badges an over-budget `{var}` family on its subtree row
/// with the numbers; under-declared draws nothing; a `{path...}` family is
/// exempt and says so — never a silent pass.
#[test]
fn the_tree_badges_the_budget_join() {
    use zengui::budget;
    use zenkey_fleet_v1::model::stats::StatsTable;

    // A registry that declares a tight budget (2) on the disk family and a
    // rest-variable family beside it.
    let subject = |path: &str, class: &str, cardinality: Option<i64>| {
        let mut d = zenkey_v1::slice::SubjectDecl::new(path, zenkey_v1::Declared::parse(class));
        d.type_name = "T".into();
        d.cardinality = cardinality;
        d
    };
    let mut tight_slice = zenkey_v1::slice::RegistrySlice::new("1.0", "t", "sysinfo");
    tight_slice.subjects = vec![
        subject("disk/{mount}/used", "telemetry", Some(2)),
        subject("log/{path...}", "events", Some(1)),
    ];
    let tight = SliceSet::from_slices(vec![tight_slice]);

    let over = [
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/root/used",
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/var/used",
        "v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/tmp/used",
        "v1/h-3fa9c2d41b7e/events/sysinfo/log/var/log/syslog",
    ];
    let mut stats = StatsTable::new();
    let now = std::time::Instant::now();
    for k in &over {
        stats.record(k, 1, None, now, None, None);
    }
    let badges = budget::badges("", &tight, &KeyTreeSnapshot::build(&stats));

    let (flat, facts) = render(&over, true);
    let watches = BTreeSet::new();
    let verdicts = zengui::verdict::VerdictCache::default();
    let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
        flat: &flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        facts: &facts,
        verdicts: &verdicts,
        budgets: Some(&badges),
        watches: tree::Watches {
            mine: &watches,
            seeding: &watches,
        },
        selected: None,
        sp: sp(),
    }));
    assert!(
        ui.find("over budget: observed 3 of 2 declared").is_ok(),
        "the offending family's subtree carries the numbers"
    );
    assert!(
        ui.find("exempt: rest-variable").is_ok(),
        "a rest-variable family is exempt and says so, never a silent pass"
    );
}
