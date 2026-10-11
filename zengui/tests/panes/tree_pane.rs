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
    let skel = zenkey_fleet::Skeleton::build(
        &zenkey_fleet::Lens::new("", None, &zenkey_fleet::ContractSet::new()),
        None,
    );
    let snap =
        zenkey_fleet::merge_skeleton(&skel, &KeyTreeSnapshot::build(&stats), &["**".to_string()]);
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
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        namespace_label: "acme",
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
        identity: (0, 0),
        totals: Default::default(),
        contracts: Default::default(),
        presence: None,
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
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        namespace_label: "acme",
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
        identity: (0, 0),
        totals: Default::default(),
        contracts: Default::default(),
        presence: None,
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
    use zenkey_fleet::StatsTable;

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

/// Every path the world's contracts declare for its addresses, as the
/// skeleton spells them (`{param}` symbolic, an event's occurrence a leaf of
/// its own), with the resource's kind.
fn declared_paths() -> Vec<(String, zenkey_model::authoring::Kind)> {
    let set = common::contracts();
    let mut out = Vec::new();
    for (address, iface) in common::World::ADDRESSES {
        let id: zenkey_model::grammar::IfaceId = iface.parse().expect("an interface");
        let revision = set.set.of_iface(&id).next().expect("held");
        for r in &revision.contract().resources {
            let mut path = format!("zk2/{address}/{iface}/{}/{}", r.token.as_str(), r.template);
            if r.token == zenkey_model::grammar::KindToken::Events {
                path.push_str("/{occurrence}");
            }
            out.push((path, r.kind));
        }
    }
    out
}

/// #614 (FL6b): the tree starts from zk2's contract skeleton. Every resource
/// of every address the world's presence names is a node before a byte of
/// it arrives — declared, and placed — and reads "declared" while nothing
/// watches it; a declared operation's leaf is marked as one, and nothing
/// else is.
#[test]
fn the_tree_shows_the_contract_skeleton() {
    use zenkey_fleet::NodeStatus;

    let world = common::World::new();
    let skel = world.skeleton();
    let paths = declared_paths();
    assert!(paths.len() >= 10, "both interfaces declare resources");
    assert_eq!(skel.coverage.revisions, 2);
    assert_eq!(skel.coverage.addresses, 2);
    assert_eq!(skel.coverage.unheld, 0);
    assert_eq!(skel.coverage.admin_entities, None, "admin not asked (O4)");

    // Nothing observed, nothing watched: every declared node reads declared.
    let merged = zenkey_fleet::merge_skeleton(&skel, &KeyTreeSnapshot::default(), &[]);
    let keys: Vec<&str> = paths.iter().map(|(p, _)| p.as_str()).collect();
    let flat = tree::flatten(&merged, "", &expand_all(&keys), 500, Instant::now());
    for (path, kind) in &paths {
        let i = flat
            .rows
            .iter()
            .position(|r| flat.arena.display(r.path) == *path)
            .unwrap_or_else(|| panic!("no declared node at {path}"));
        let row = flat.row(i);
        let NodeStatus::DeclaredOnly(e) = row.status else {
            panic!("{path} is not declared-only: {:?}", row.status)
        };
        assert!(e.declared && e.present, "{path}: declared and placed");
        assert_eq!(
            row.operation,
            *kind == zenkey_model::authoring::Kind::Operation,
            "{path}: only an operation's leaf is marked as one"
        );
        assert!(row.decl_type.is_some(), "{path} carries its declared type");
    }
    assert!(
        paths
            .iter()
            .any(|(p, k)| p.contains("/@op/") && *k == zenkey_model::authoring::Kind::Operation),
        "the fixture declares operations"
    );
}

/// #614 (FL6b): `**` never crosses a verbatim chunk (core §1.3), so under an
/// ambient watch an explicit `@stream` or `@state` leaf stays declared-only
/// — the truth about what that watch can see — while a plain resource beside
/// it is watched and quiet.
#[test]
fn explicit_leaves_stay_declared_only_under_an_ambient_watch() {
    use zenkey_fleet::NodeStatus;

    let loaded = zenkey_model::contract::load_str(SENSOR, std::path::Path::new("."), None);
    let contract = loaded
        .contract
        .unwrap_or_else(|| panic!("does not load:\n{}", loaded.report));
    let mut set = zenkey_fleet::ContractSet::new();
    set.insert(zenkey_fleet::Revision::from_contract(
        contract,
        zenkey_fleet::report::ContractSource::File,
    ));
    // No presence read: the revision sits under a symbolic address.
    let lens = zenkey_fleet::Lens::new("", None, &set).offline(&set);
    let skel = zenkey_fleet::Skeleton::build(&lens, None);
    let merged =
        zenkey_fleet::merge_skeleton(&skel, &KeyTreeSnapshot::default(), &["**".to_string()]);
    let base = "zk2/{system}/{service}/sensor.v1";
    let frames = format!("{base}/@stream/frames");
    let calibration = format!("{base}/@state/calibration/{{axis}}");
    let reading = format!("{base}/stream/reading");
    let flat = tree::flatten(
        &merged,
        "",
        &expand_all(&[frames.as_str(), calibration.as_str(), reading.as_str()]),
        500,
        Instant::now(),
    );
    let status = |path: &str| {
        let i = flat
            .rows
            .iter()
            .position(|r| flat.arena.display(r.path) == path)
            .unwrap_or_else(|| panic!("no node at {path}"));
        flat.row(i).status
    };
    for explicit in [&frames, &calibration] {
        assert!(
            matches!(status(explicit), NodeStatus::DeclaredOnly(_)),
            "{explicit}: `**` cannot reach it, so no watch covers it"
        );
    }
    assert!(
        matches!(status(&reading), NodeStatus::WatchedQuiet(_)),
        "a plain resource is covered by `**`, and quiet is an observation"
    );
}

/// A contract with an explicit stream and an explicit state beside a plain
/// stream: what no example the world loads declares.
const SENSOR: &str = r#"
[interface]
name = "sensor"
major = 1

[resources.reading]
kind = "stream"
type = { raw = "application/octet-stream" }

[resources.frames]
kind = "stream"
explicit = true
type = { raw = "application/octet-stream" }

[resources."calibration/{axis}"]
kind = "state"
explicit = true
type = { raw = "application/octet-stream" }
params = { axis = "string" }
cardinality = 3
"#;

/// #614 (FL6b): a declared operation's row offers no watch control, and
/// neither does the `@op` row holding it — a queryable carries no samples,
/// and a watch's seed would GET it, which is a call. Pinned on the open eye
/// a held watch draws: put the operation's path (or its `@op` parent) among
/// this app's watches, and no eye appears; put a stream's there, and one
/// does.
#[test]
fn an_operation_row_offers_no_watch_control() {
    use zengui::view::kit::Icon;

    let world = common::World::new();
    let merged = zenkey_fleet::merge_skeleton(&world.skeleton(), &KeyTreeSnapshot::default(), &[]);
    let nav = "zk2/robot1/nav/nav.v2";
    let reset = format!("{nav}/@op/reset");
    let position = format!("{nav}/stream/position");
    let flat = tree::flatten(
        &merged,
        "",
        &expand_all(&[reset.as_str(), position.as_str()]),
        500,
        Instant::now(),
    );
    let facts = FactsIndex::default();
    let verdicts = zengui::verdict::VerdictCache::default();
    let seeding = BTreeSet::new();
    let eye = Icon::Watched.codepoint().to_string();
    fn draw<'a>(
        flat: &'a tree::Flattened,
        facts: &'a FactsIndex,
        verdicts: &'a zengui::verdict::VerdictCache,
        mine: &'a BTreeSet<String>,
        seeding: &'a BTreeSet<String>,
    ) -> iced_test::Simulator<'a, Message> {
        simulator::<Message, _, _>(tree::pane(tree::TreeData {
            flat,
            pivot: tree::Pivot::Chunks,
            search: "",
            viewport: Default::default(),
            facts,
            verdicts,
            budgets: None,
            watches: tree::Watches { mine, seeding },
            selected: None,
            sp: sp(),
        }))
    }
    let held_on_reset = BTreeSet::from([reset.clone(), format!("{nav}/@op")]);
    let mut ui = draw(&flat, &facts, &verdicts, &held_on_reset, &seeding);
    assert!(ui.find("reset").is_ok(), "the operation's leaf is drawn");
    assert!(
        ui.find(eye.as_str()).is_err(),
        "an operation's rows have no watch control to show a watch in"
    );
    for (i, r) in flat.rows.iter().enumerate() {
        let path = flat.arena.display(r.path);
        assert_eq!(
            flat.row(i).operation,
            path.split('/').any(|c| c == "@op"),
            "{path}: marked exactly when it stands under `@op`"
        );
    }
    let held_on_position = BTreeSet::from([position.clone()]);
    let mut ui = draw(&flat, &facts, &verdicts, &held_on_position, &seeding);
    assert!(
        ui.find(eye.as_str()).is_ok(),
        "a stream's row does, and shows the watch held"
    );
}
