//! The key tree: the identity rung badges, a narrow tree, density, keys the
//! lens stops short of and foreign keys, the empty tree, the identity
//! cache's bound, the budget badges and the contract skeleton (#614, FL7:
//! zk2's identity ladder where v1's registration badges stood).

use super::*;

use zengui::keyfacts::Rung;

/// The tree pane over `flat`, with `identities` and `budgets`, at the
/// comfortable grid — the one call shape the tests below share.
fn pane<'a>(
    flat: &'a tree::Flattened,
    identities: &'a IdentityIndex,
    namespace: &'a str,
    budgets: Option<&'a zengui::budget::BudgetBadges>,
) -> iced::Element<'a, Message> {
    let watches: &'static BTreeSet<String> = Box::leak(Box::default());
    let verdicts: &'static zengui::verdict::VerdictCache = Box::leak(Box::default());
    tree::pane(tree::TreeData {
        sp: sp(),
        flat,
        pivot: tree::Pivot::Chunks,
        search: "",
        viewport: Default::default(),
        identities,
        namespace,
        verdicts,
        budgets,
        watches: tree::Watches {
            mine: watches,
            seeding: watches,
        },
        selected: None,
    })
}

/// Every rung's word, for asserting that none is drawn.
fn any_rung_word(ui: &mut iced_test::Simulator<'_, Message>) -> Option<&'static str> {
    Rung::ALL
        .iter()
        .map(|r| r.label())
        .find(|w| ui.find(*w).is_ok())
}

/// #193's acceptance on zk2's ladder (#614, FL7): every rung renders a
/// *distinct* glyph + word pair. The glyph comes from the tone type — no
/// call site can omit it — so colour is structurally never the only
/// carrier, and the word names the rung where the glyph names its class.
#[test]
fn every_identity_rung_renders_a_distinct_glyph_word_pair() {
    let pairs: Vec<(&str, &str)> = Rung::ALL
        .iter()
        .map(|r| (r.tone().glyph(), r.label()))
        .collect();
    for (i, a) in pairs.iter().enumerate() {
        for b in &pairs[i + 1..] {
            assert_ne!(a, b, "two rungs render the same glyph + word pair");
        }
    }
    // …and each pair actually reaches the screen, through the same
    // `tone_badge` every pane uses.
    let badges = iced::widget::Column::with_children(
        Rung::ALL
            .iter()
            .map(|r| zengui::view::kit::tone_badge::<Message>(r.tone(), r.label())),
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

/// #614 (FL7): the rungs on resolutions the lens really made, each key
/// drawn alone so nothing else on screen can answer for it. Under the
/// namespace `spray`: an owned stream resolves; an address nobody provides
/// stops at "no provider"; a stream the contract does not declare at "no
/// resource"; an address's instance token is a control key; a `v1/…` key
/// in the namespace reads "not zk2"; another deployment's zk2 key "not in
/// namespace" — and another application's key is drawn plainly, no badge
/// and no role.
#[test]
fn the_tree_badges_the_rung_each_key_reached() {
    let world = common::World::new();
    let cases: [(&str, Option<Rung>); 7] = [
        (
            "spray/zk2/robot1/nav/nav.v2/stream/position",
            Some(Rung::Resolved),
        ),
        (
            "spray/zk2/robot9/nav/nav.v2/stream/position",
            Some(Rung::NoProvider),
        ),
        (
            "spray/zk2/robot1/nav/nav.v2/stream/nowhere",
            Some(Rung::NoResource),
        ),
        (
            "spray/zk2/robot1/nav/@zk/instance/0000000000000001",
            Some(Rung::Control),
        ),
        (
            "spray/v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load",
            Some(Rung::NotZk2),
        ),
        (
            "elsewhere/zk2/robot1/nav/nav.v2/stream/position",
            Some(Rung::NotInNamespace),
        ),
        ("demo/example/foo", None),
    ];
    for (key, want) in cases {
        let identities = world.identities("spray", &[key]);
        let snap = snapshot(&[key]);
        let flat = tree::flatten(&snap, "spray", &expand_all(&[key]), 500, Instant::now());
        let mut ui = simulator::<Message, _, _>(pane(&flat, &identities, "spray", None));
        assert_eq!(any_rung_word(&mut ui), want.map(Rung::label), "{key}");
    }
}

/// #614 (FL7): what only a read can answer is "not asked" until one is
/// made. Without a presence read every zk2 key stops at "presence not
/// read"; with presence and no contract in hand, at "contract not held" —
/// neither ever reads as resolved or as "no provider", and the positions
/// are still labelled.
#[test]
fn an_unread_tree_claims_neither_way() {
    let key = common::POSITION;
    let (flat, identities) = render(&[key], false);
    let mut ui = simulator::<Message, _, _>(pane(&flat, &identities, "", None));
    assert_eq!(any_rung_word(&mut ui), Some("presence not read"));
    assert!(
        ui.find("system").is_ok(),
        "the positions are labelled whatever was read"
    );

    let world = common::World::new();
    let catalog = world.catalog();
    let none = zenkey_fleet::ContractSet::new();
    let lens = zenkey_fleet::Lens::new("", Some(&catalog), &none);
    let mut identities = IdentityIndex::default();
    identities.ensure(&lens, key);
    let mut ui = simulator::<Message, _, _>(pane(&flat, &identities, "", None));
    assert_eq!(any_rung_word(&mut ui), Some("contract not held"));
}

/// The positions reach the screen: a zk2 key's chunks wear zk2's roles at
/// a pane wide enough for them, and a foreign key's wear none — no
/// invented labels, and no rung badge either.
#[test]
fn zk2_roles_reach_the_screen_and_foreign_keys_wear_none() {
    let (flat, identities) = render(&[common::ETH0], true);
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1400.0, 800.0),
        pane(&flat, &identities, "", None),
    );
    for role in ["grammar", "system", "service", "iface", "kind", "resource"] {
        assert!(ui.find(role).is_ok(), "missing role label {role:?}");
    }
    assert!(ui.find("resolved").is_ok());

    let (flat, identities) = render(&[FOREIGN], true);
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1400.0, 800.0),
        pane(&flat, &identities, "", None),
    );
    for role in ["grammar", "system", "service", "iface", "kind", "resource"] {
        assert!(
            ui.find(role).is_err(),
            "a foreign key must not be labelled {role:?}"
        );
    }
    assert_eq!(any_rung_word(&mut ui), None, "nor badged");
}

/// #614 (FL7): v1 keys are foreign since the port — a `v1/…` key reads
/// "not zk2", and wears no v1 role: no v1 overlay.
#[test]
fn a_v1_key_reads_not_zk2() {
    let (flat, identities) = render(&[REGISTERED], true);
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1400.0, 800.0),
        pane(&flat, &identities, "", None),
    );
    assert!(ui.find("not zk2").is_ok());
    for role in [
        "version", "origin", "class", "producer", "subject", "system", "service",
    ] {
        assert!(ui.find(role).is_err(), "a v1 key wears no {role:?}");
    }
}

/// #557: a narrow Locator shows less of a row, never two rows at once. At
/// 240px the deep leaves are in the narrow tier — the counts collapse to the
/// rate, on one line, inside the pane — and a badge with no room is left
/// out whole and *counted* (`+1`), never cut and never silently gone. At the
/// default width the whole row is back.
#[test]
fn a_narrow_tree_keeps_each_row_on_its_line() {
    use zengui::view::tokens::CAPTION_LINE;

    let keys = [common::POSITION, common::NAV_STATUS];
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
    let identities = identities(&keys, true);
    let leaf = (0..flat.rows.len())
        .map(|i| flat.row(i))
        .find(|r| r.target.as_deref() == Some(common::POSITION))
        .expect("the position leaf is a row");
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

    let width = 240.0;
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (width, 600.0),
        pane(&flat, &identities, "", None),
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
        ui.find("resolved").is_err(),
        "no room for a badge at this depth"
    );
    assert!(
        ui.find("+1").is_ok(),
        "…so it is counted, not silently dropped (an absent badge reads as not asked)"
    );

    let mut ui = simulator::<Message, _, _>(pane(&flat, &identities, "", None));
    assert!(
        ui.find(wide.as_str()).is_ok(),
        "the default width is the whole row"
    );
    assert!(ui.find("resolved").is_ok());
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

    let keys = [common::POSITION, REGISTERED, FOREIGN];
    let (flat, identities) = render(&keys, true);
    let watches = BTreeSet::new();
    for density in Density::ALL {
        let verdicts = zengui::verdict::VerdictCache::default();
        let mut ui = simulator::<Message, _, _>(tree::pane(tree::TreeData {
            sp: Spacing::of(density),
            flat: &flat,
            pivot: tree::Pivot::Chunks,
            search: "",
            viewport: Default::default(),
            identities: &identities,
            namespace: "",
            verdicts: &verdicts,
            budgets: None,
            watches: tree::Watches {
                mine: &watches,
                seeding: &watches,
            },
            selected: None,
        }));
        for claim in ["resolved", "not zk2"] {
            assert!(
                ui.find(claim).is_ok(),
                "{claim:?} must survive {} density",
                density.label()
            );
        }
    }
}

/// The empty tree must explain itself rather than implying the bus is quiet
/// (RFC 05 §3.1).
#[test]
fn the_empty_tree_explains_itself() {
    let flat = tree::Flattened::empty();
    let identities = IdentityIndex::default();
    let mut ui = simulator::<Message, _, _>(pane(&flat, &identities, "", None));

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

fn strip(identity: (usize, u64), keys: (usize, u64)) -> iced::Element<'static, Message> {
    use zengui::view::status::{self, Status};
    let link: &'static zengui::message::LinkState =
        Box::leak(Box::new(zengui::message::LinkState::Pumping));
    let watched: &'static Vec<String> = Box::leak(Box::default());
    status::strip(Status {
        link,
        namespace_label: "acme",
        watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        scope_label: "all",
        scope_coverage: "",
        keys: keys.0,
        keys_evicted: keys.1,
        identity,
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
    })
}

/// #107's "the bound is not a silent one", on zk2's identity cache (#614,
/// FL7): its retirement count reaches the strip, and reads as its own
/// number rather than being folded into the key count. Two bounds, two
/// populations, two sentences (O6).
#[test]
fn the_identity_cache_discloses_its_bound() {
    use zengui::view::status;

    let mut ui = simulator::<Message, _, _>(strip((118, 312), (120, 7)));
    assert!(
        ui.find(status::identity_text(118, 312).as_str()).is_ok(),
        "the cache's bound must be visible, not merely enforced"
    );
    assert!(
        ui.find(status::keys_text(120, 7).as_str()).is_ok(),
        "and the key table's own count is still its own"
    );
    assert_ne!(
        status::identity_text(118, 312),
        status::keys_text(118, 312),
        "the two bounds must not read as one number"
    );
}

/// A quiet cache has nothing to disclose: the key count already states the
/// population, so an untripped bound must not add a line.
#[test]
fn an_untripped_cache_bound_says_nothing() {
    use zengui::view::status;

    let mut ui = simulator::<Message, _, _>(strip((12, 0), (12, 0)));
    assert!(ui.find(status::identity_text(12, 0).as_str()).is_err());
}

/// #614 (FL7): the tree badges the core judge's population answer on the
/// family's subtree row (core §2.7). Through the real judge over the
/// world's deployment, the two answers a GUI window mostly gets: a
/// best-effort stream is lossy, and a state is no window's to count. Then
/// every other answer, drawn as the judge would hand it over: within and
/// exceeds with their numbers, and unobservable with its reason — the owner
/// absent, the window too short — never a pass.
#[test]
fn the_tree_badges_the_budget_judgement() {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};
    use zengui::budget::{BudgetBadge, BudgetBadges, BudgetInputs, ReaderLoss, badges};
    use zenkey_model::budget::{Bound, Judged, Reason, Verdict};

    let world = common::World::new();
    let bandwidth = "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0";
    let keys = [bandwidth, common::ETH0, common::WLAN0];
    let mut stats = StatsTable::new();
    let start = Instant::now();
    for k in &keys {
        stats.record(k, 8, None, start + Duration::from_secs(1), None, None);
    }
    let judged = badges(&BudgetInputs {
        namespace: String::new(),
        catalog: Some(world.catalog()),
        store: world.store(),
        presence: Some(world.presence()),
        observed: Arc::new(KeyTreeSnapshot::build(&stats)),
        watches: vec![("**".into(), start)],
        reader: ReaderLoss::default(),
        timeout: Duration::from_secs(2),
        now: start + Duration::from_secs(120),
        wall: SystemTime::now() + Duration::from_secs(120),
    });
    let snap = snapshot(&keys);
    let flat = tree::flatten(&snap, "", &expand_all(&keys), 500, Instant::now());
    let identities = identities(&keys, true);
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1400.0, 800.0),
        pane(&flat, &identities, "", Some(&judged)),
    );
    assert!(
        ui.find("budget unobservable (lossy)").is_ok(),
        "a best-effort stream is lossy by declaration: {judged:?}"
    );
    assert!(
        ui.find("budget unobservable (a window does not count state)")
            .is_ok(),
        "a state's members are an owner GET's to count"
    );

    let badge = |verdict, reason, counted| BudgetBadge {
        bound: Bound::Of(1024),
        judged: Judged {
            verdict,
            reason,
            counted,
        },
    };
    let rows = [
        (
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth",
            badge(Verdict::Within, Reason::Complete, 2),
        ),
        (
            "zk2/host-a/tc/tc.netif.v1/state/interfaces",
            badge(Verdict::Exceeds, Reason::OverBound, 1025),
        ),
        (
            "zk2/host-a/tc/tc.netif.v1/state/interfaces/default",
            badge(Verdict::Unobservable, Reason::OwnerAbsent, 2),
        ),
        (
            "zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default",
            badge(Verdict::Unobservable, Reason::WindowTooShort, 1),
        ),
    ];
    let drawn: BudgetBadges = rows.iter().map(|(p, b)| ((*p).to_owned(), *b)).collect();
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1400.0, 800.0),
        pane(&flat, &identities, "", Some(&drawn)),
    );
    for word in [
        "within budget: 2 of 1024",
        "exceeds budget: 1025 of 1024",
        "budget unobservable (owner absent)",
        "budget unobservable (window too short)",
    ] {
        assert!(ui.find(word).is_ok(), "{word:?} must reach the screen");
    }
    // Each answer wears its own glyph: the finding, the kept budget and the
    // reading that could not say never look alike.
    use zengui::view::theme::BudgetTone;
    assert_eq!(rows[0].1.tone(), BudgetTone::Within);
    assert_eq!(rows[1].1.tone(), BudgetTone::Exceeds);
    assert_eq!(rows[2].1.tone(), BudgetTone::Unobservable);
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
/// else is. Since FL7 the declared nodes wear zk2's roles too.
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
        assert_eq!(
            row.role,
            Some(tree::Role::Resource),
            "{path}: a declared leaf is a resource chunk"
        );
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
    let at = |path: &str| {
        let i = flat
            .rows
            .iter()
            .position(|r| flat.arena.display(r.path) == path)
            .unwrap_or_else(|| panic!("no node at {path}"));
        flat.row(i)
    };
    for explicit in [&frames, &calibration] {
        assert!(
            matches!(at(explicit).status, NodeStatus::DeclaredOnly(_)),
            "{explicit}: `**` cannot reach it, so no watch covers it"
        );
    }
    assert!(
        matches!(at(&reading).status, NodeStatus::WatchedQuiet(_)),
        "a plain resource is covered by `**`, and quiet is an observation"
    );
    // The symbolic address holds its positions (#614, FL7).
    assert_eq!(at("zk2/{system}").role, Some(tree::Role::System));
    assert_eq!(at("zk2/{system}/{service}").role, Some(tree::Role::Service));
    assert_eq!(at(&format!("{base}/@stream")).role, Some(tree::Role::Kind));
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
    let identities = IdentityIndex::default();
    let verdicts = zengui::verdict::VerdictCache::default();
    let seeding = BTreeSet::new();
    let eye = Icon::Watched.codepoint().to_string();
    fn draw<'a>(
        flat: &'a tree::Flattened,
        identities: &'a IdentityIndex,
        verdicts: &'a zengui::verdict::VerdictCache,
        mine: &'a BTreeSet<String>,
        seeding: &'a BTreeSet<String>,
    ) -> iced_test::Simulator<'a, Message> {
        simulator::<Message, _, _>(tree::pane(tree::TreeData {
            flat,
            pivot: tree::Pivot::Chunks,
            search: "",
            viewport: Default::default(),
            identities,
            namespace: "",
            verdicts,
            budgets: None,
            watches: tree::Watches { mine, seeding },
            selected: None,
            sp: sp(),
        }))
    }
    let held_on_reset = BTreeSet::from([reset.clone(), format!("{nav}/@op")]);
    let mut ui = draw(&flat, &identities, &verdicts, &held_on_reset, &seeding);
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
    let mut ui = draw(&flat, &identities, &verdicts, &held_on_position, &seeding);
    assert!(
        ui.find(eye.as_str()).is_ok(),
        "a stream's row does, and shows the watch held"
    );
}

/// #614 (FL7): a resolved rung is the expected answer, and with no room for
/// both it gives way to the verdict, which may be a finding — at no width is
/// "resolved" drawn while an "invalid" beside it is counted away.
#[test]
fn a_resolved_rung_yields_to_a_finding_verdict() {
    let key = common::WLAN0;
    let (flat, identities) = render(&[key], true);
    let invalid = zenkey_fleet::report::Conformance::Invalid {
        violations: vec!["/index: -1 is less than 0".into()],
    };
    let mut verdicts = zengui::verdict::VerdictCache::default();
    verdicts.record(key, invalid.clone());
    let watches = BTreeSet::new();
    let mut both = false;
    for width in (240..=900).step_by(20) {
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            (width as f32, 600.0),
            tree::pane(tree::TreeData {
                sp: sp(),
                flat: &flat,
                pivot: tree::Pivot::Chunks,
                search: "",
                viewport: Default::default(),
                identities: &identities,
                namespace: "",
                verdicts: &verdicts,
                budgets: None,
                watches: tree::Watches {
                    mine: &watches,
                    seeding: &watches,
                },
                selected: None,
            }),
        );
        let resolved = ui.find("resolved").is_ok();
        let finding = ui.find(zengui::verdict::short_label(&invalid)).is_ok()
            || ui.find(zengui::verdict::label(&invalid).as_str()).is_ok();
        assert!(
            !resolved || finding,
            "at {width}px the resolved badge is drawn and the finding is not"
        );
        both |= resolved && finding;
    }
    assert!(both, "at some width both fit, or the test pins nothing");
}
