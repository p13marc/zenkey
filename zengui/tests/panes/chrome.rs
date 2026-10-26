//! The chrome: preferences, the connect pane, the palette, the location bar
//! (#185), the key-expression editor (#187) and the Settings overlay
//! (#188).

use super::*;

/// Persisted preferences (#73) reach the window: the theme button says what
/// you *get*, the zoom reads back as a percentage, and a preferences file that
/// could not be read explains itself in the strip rather than looking like a
/// reset.
#[test]
fn preferences_are_visible_and_a_broken_file_says_so() {
    use zengui::prefs::{Prefs, ThemeChoice};
    use zengui::view::status::{self, Status};

    // The theme choice drives the actual iced theme, not just a label.
    // Since #534 it is zengui's own palette rather than iced's stock one, so
    // what is pinned is the property: each choice renders as itself.
    assert!(!ThemeChoice::Light.theme().extended_palette().is_dark);
    assert!(ThemeChoice::Dark.theme().extended_palette().is_dark);
    assert_ne!(ThemeChoice::Light.theme(), ThemeChoice::Dark.theme());

    let mut prefs = Prefs::default();
    prefs.zoom_in();
    assert!(prefs.zoom > 1.0);

    let link = zengui::message::LinkState::Pumping;
    let watched: Vec<String> = vec![];
    let mut ui = simulator::<Message, _, _>(status::strip(Status {
        link: &link,
        namespace_label: "acme",
        watched: &watched,
        skeleton: None,
        keys_unwatched: 0,
        fetched: None,
        last_known: None,
        scope_label: "all",
        scope_coverage: "",
        keys: 0,
        keys_evicted: 0,
        identity: (0, 0),
        totals: Default::default(),
        contracts: Default::default(),
        presence: None,
        seeding: 0,
        seeded_watches: 0,
        seed_totals: Default::default(),
        unreachable: false,
        prefs_note: Some("zengui.toml does not parse (bad) — using defaults"),
        replaying: false,
        retention: None,
    }));
    assert!(
        ui.find("preferences: zengui.toml does not parse (bad) — using defaults")
            .is_ok(),
        "an unreadable prefs file must not look like a reset"
    );
}

/// The connection pane (#67) explains scouting rather than labelling it, and
/// carries RFC 09 §0.1's actual semantics: two independent switches, and the
/// reading of an empty result under an isolated session.
#[test]
fn the_connect_pane_states_what_scouting_means() {
    use zengui::view::contexts::{ContextForm, pane};

    let form = ContextForm {
        known: vec!["lab".into(), "prod".into()],
        active: Some("lab".into()),
        ..ContextForm::default()
    };
    let mut ui = simulator::<Message, _, _>(pane(&form, false));
    assert!(
        ui.find(
            "RFC 09 §0.1: multicast scouting and gossip are independent. This toggle is \
             the multicast half only. The session is a client of its endpoints — no \
             listener, no gossip — unless it listens, and a listening peer still \
             learns about others through gossip over an established link."
        )
        .is_ok(),
        "the two-switch distinction must be on screen, not just in the RFC"
    );
    assert!(
        ui.find(
            "Contexts are shared with zenctl — one file, two explorers \
             (~/.config/zenkey-explorer/config.toml)."
        )
        .is_ok(),
        "the shared store is the feature; say so"
    );

    // #559: every box is labelled by a label, not a placeholder — the
    // placeholder disappears once typed into, and iced's `find` never sees
    // it anyway, which is how its words used to go unpinned.
    for label in [
        "NAME",
        "NAMESPACE",
        "TIMEOUT (S)",
        "CONNECT",
        "LISTEN",
        "ZENOH CONFIG",
    ] {
        assert!(
            ui.find(label).is_ok(),
            "the {label} field has lost its label"
        );
    }
    // No registry row since #614 (FL6a): zk2 reads contracts, which this
    // window remembers itself — a context's v1 `registry` is carried
    // through a save untouched, and never shown as if it did something.
    assert!(ui.find("REGISTRY").is_err(), "connect has no registry row");
    assert!(ui.find("empty = the bus root: keys start at zk2/").is_ok());

    // A session that reaches nothing is called out where the fix is — as an
    // error, the mark beside the words (#559).
    let mut ui = simulator::<Message, _, _>(pane(&form, true));
    assert!(
        ui.find("this session has no endpoints and multicast scouting is off — it reaches nothing")
            .is_ok()
    );
    assert!(
        ui.find("✗").is_ok(),
        "an error carries its mark, not colour alone"
    );
}

/// The command palette (#75): every action is on screen and every one of
/// them emits a message the UI path also emits — the property that keeps a
/// palette from becoming a second implementation of the app.
#[test]
fn the_palette_offers_the_apps_own_actions_and_the_help_lists_the_real_map() {
    use zengui::view::contexts::ContextForm;
    use zengui::view::palette::{Overlay, PaletteState, overlay};
    use zengui::view::scope_editor::{ScopeEditorData, ScopeForm};

    let form = ContextForm {
        known: vec!["lab".to_string()],
        ..ContextForm::default()
    };
    let scope_form = ScopeForm::default();
    fn scope(form: &ScopeForm) -> ScopeEditorData<'_> {
        ScopeEditorData {
            scope: zengui::scope::ScopePreset::Everything,
            namespace: "",
            selectors: &[],
            form,
        }
    }
    let cfg = launch_settings();
    let settings_form = zengui::view::settings::SettingsForm::default();
    fn settings<'a>(
        settings: &'a zengui::config::Settings,
        form: &'a zengui::view::settings::SettingsForm,
    ) -> zengui::view::settings::SettingsData<'a> {
        zengui::view::settings::SettingsData {
            settings,
            form,
            theme: "dark",
            zoom: 1.0,
            echo: (0, 0, 0),
            history: None,
            keys: (0, 0),
        }
    }
    let keys = [
        "v1/h-3fa9c2d41b7e/state/sysinfo/health".to_string(),
        "demo/example/foo".to_string(),
    ];

    let mut state = PaletteState::default();
    state.open(Overlay::Commands);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(|k| (k.as_str(), None)),
        )
        .expect("commands overlay");
        let mut ui = simulator::<Message, _, _>(element);
        // The doctor stopped being a place and became an action (#183): its run
        // is a palette command and its verdict lands in the Activity dock.
        assert!(ui.find("run doctor").is_ok());
        assert!(
            ui.find("go to doctor pane").is_err(),
            "a pane that no longer exists must not still be offered"
        );
        assert!(ui.find("activity: doctor").is_ok());
        assert!(ui.find("context: lab").is_ok(), "contexts are offered");
        // Connect stopped being a pane too (#185): the overlay is offered
        // instead, and a tab that is gone must not still be spoken of.
        assert!(ui.find("connect — contexts and endpoints").is_ok());
        assert!(ui.find("go to connect pane").is_err());
    }

    // The list is bounded, so reaching an action past the first screenful is
    // what typing is for — which is also the fuzzy match's real workload.
    state.query = "ndjson".into();
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(|k| (k.as_str(), None)),
        )
        .expect("commands overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("export echo as ndjson").is_ok());
        assert!(
            ui.find("go to doctor pane").is_err(),
            "a query narrows; it does not merely reorder"
        );
    }
    state.query.clear();

    // Jump-to-key offers only observed keys, and says so — it must not read
    // as an inventory of the keyspace (O4).
    state.open(Overlay::Keys);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(|k| (k.as_str(), None)),
        )
        .expect("keys overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("v1/h-3fa9c2d41b7e/state/sysinfo/health").is_ok());
        assert!(
            ui.find("fuzzy over keys observed so far — nothing here is a guess (O4)")
                .is_ok()
        );
    }

    // The `?` overlay renders the shortcut map itself — modifier-less
    // chords included (#190) — so it cannot drift from what the app
    // dispatches.
    state.open(Overlay::Help);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(|k| (k.as_str(), None)),
        )
        .expect("help overlay");
        let mut ui = simulator::<Message, _, _>(element);
        for binding in zengui::shortcuts::map() {
            assert!(
                ui.find(binding.keys).is_ok(),
                "the help overlay omits {:?} ({})",
                binding.keys,
                binding.what
            );
        }
        // …and nothing else: the hardcoded trailing lines that restated
        // `Ctrl P`, `Ctrl K` and `?` by hand are gone (#190) — the table's
        // own rows for Esc and `?` are what remains.
        assert!(
            ui.find("this list · Esc closes").is_err(),
            "the help must render exactly the table"
        );
        assert!(ui.find("Esc").is_ok(), "Esc is a row of the table now");
    }

    // The Connect overlay is the connection pane, floated (#185) — same
    // form state, same messages, a different surface.
    state.open(Overlay::Connect);
    {
        let element = overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(|k| (k.as_str(), None)),
        )
        .expect("connect overlay");
        let mut ui = simulator::<Message, _, _>(element);
        assert!(ui.find("Connection").is_ok(), "the pane renders inside it");
        assert!(
            ui.find("session setup — Esc closes").is_ok(),
            "a modal says how to leave it"
        );
    }

    // Closed means nothing renders.
    state.close();
    assert!(
        overlay(
            &state,
            &form,
            false,
            scope(&scope_form),
            settings(&cfg, &settings_form),
            keys.iter().map(|k| (k.as_str(), None))
        )
        .is_none()
    );
}

// ── Location bar (#185) ──────────────────────────────────────────────────

/// The location bar names the empty namespace — the bus-root deployment
/// is a first-class value, not a missing one — and says when nothing is
/// selected rather than rendering a gap.
#[test]
fn the_location_bar_names_the_empty_namespace_and_the_empty_selection() {
    use zengui::config::NamespaceChoice;
    use zengui::message::Subject;
    use zengui::view::location::{self, LocationData};

    let options = [NamespaceChoice::bus_root(), NamespaceChoice::new("acme")];
    // The picker's closed face and its rows both draw `NamespaceChoice`'s
    // `Display`, and a `pick_list` draws that text itself — it is not a
    // `Text` widget, so `iced_test`'s text selector cannot see inside it.
    // The label is pinned at the seam the picker draws from instead; that
    // this is the picker's row type is what `location::breadcrumb`'s
    // signature enforces.
    assert_eq!(
        options[0].to_string(),
        "(empty — keys start at zk2/)",
        "the empty namespace renders its label, never a blank row"
    );
    let mut ui = simulator::<Message, _, _>(location::breadcrumb(LocationData {
        context: None,
        namespace: "",
        namespace_options: &options,
        scope: zengui::scope::ScopePreset::Everything,
        observing: false,
        subject: &Subject::None,
    }));
    assert!(
        ui.find("no context").is_ok(),
        "an unnamed context is a state, not an omission"
    );
    assert!(
        ui.find("nothing selected — pick a key in the tree").is_ok(),
        "an empty key trail explains itself"
    );
}

/// Every chunk of the subject reaches the screen, and clicking an ancestor
/// selects that subtree — the acceptance the bar exists for (#185).
#[test]
fn the_location_bar_renders_the_trail_and_an_ancestor_click_selects_its_subtree() {
    use zengui::config::NamespaceChoice;
    use zengui::message::{Subject, SubjectMsg};
    use zengui::view::location::{self, LocationData};

    let subject = Subject::Key("v1/h-a/telemetry/sysinfo/disk".into());
    let options = [NamespaceChoice::bus_root()];
    let mut ui = simulator::<Message, _, _>(location::breadcrumb(LocationData {
        context: Some("lab"),
        namespace: "",
        namespace_options: &options,
        scope: zengui::scope::ScopePreset::Streams,
        observing: false,
        subject: &subject,
    }));
    for chunk in ["v1", "h-a", "telemetry", "sysinfo", "disk"] {
        assert!(
            ui.find(chunk).is_ok(),
            "chunk {chunk:?} should be on screen"
        );
    }
    assert!(
        ui.find("context: lab").is_ok(),
        "the active context is display as well as control"
    );

    ui.click("telemetry")
        .expect("an ancestor chunk is a button");
    let messages: Vec<String> = ui.into_messages().map(|m| format!("{m:?}")).collect();
    let want = format!(
        "{:?}",
        Message::Subject(SubjectMsg::Select(Subject::Prefix(
            "v1/h-a/telemetry".into()
        )))
    );
    assert!(
        messages.contains(&want),
        "clicking an ancestor selects that subtree, got: {messages:?}"
    );
}

// ── The key-expression editor (#187) ─────────────────────────────────────

/// On a preset the editor is read-only truth: the selectors actually
/// resolved by `scope::selectors` — the Deployment preset's ambient
/// selector in the namespace (#614, FL6a) — each with what it cannot see,
/// under the verbatim rule, with the fork as the only way to edit.
#[test]
fn the_selector_editor_shows_the_resolved_truth_and_its_blind_spots() {
    use zengui::scope::ScopePreset;
    use zengui::view::scope_editor::{self, ScopeEditorData, ScopeForm};

    let form = ScopeForm::default();
    let mut ui = simulator::<Message, _, _>(scope_editor::pane(ScopeEditorData {
        scope: ScopePreset::Deployment,
        namespace: "zensight",
        selectors: &[],
        form: &form,
    }));

    // The verbatim rule is stated inline — the exact wording, pinned.
    assert!(
        ui.find(scope_editor::VERBATIM_RULE).is_ok(),
        "the verbatim rule is on screen"
    );

    // The resolved selectors are the ones `scope::selectors` builds — the
    // namespace's ambient selector (#614, FL6a).
    for sel in ScopePreset::Deployment.selectors("zensight", &[]) {
        assert!(
            ui.find(sel.as_str()).is_ok(),
            "resolved selector {sel:?} not shown"
        );
    }
    assert!(ui.find("zensight/zk2/**").is_ok());

    // Every selector names what it cannot see — zk2's verbatim chunks.
    let spot = zengui::scope::blind_spot("zensight/zk2/**");
    assert!(spot.contains("@stream") && spot.contains("@zk"), "{spot}");
    assert!(
        ui.find(format!("cannot see: {spot}")).is_ok(),
        "the ambient selector's blind spot is on screen"
    );
    assert!(ui.find("fork into custom and edit").is_ok());
    assert!(
        ui.find("apply — the scope becomes custom").is_err(),
        "a preset is read-only until forked"
    );
}

/// In custom mode the editor validates per keystroke: an invalid row shows
/// the validator's own words (`$*` names RFC 03 §2), a valid row shows its
/// blind spot, and the apply row is offered.
#[test]
fn the_selector_editor_validates_each_row_as_typed() {
    use zengui::scope::ScopePreset;
    use zengui::view::scope_editor::{self, ScopeEditorData, ScopeForm};

    let form = ScopeForm {
        editing: true,
        rows: vec!["demo/**".into(), "demo/$*/x".into()],
        status: None,
    };
    let active = ["demo/**".to_string()];
    let mut ui = simulator::<Message, _, _>(scope_editor::pane(ScopeEditorData {
        scope: ScopePreset::Custom,
        namespace: "",
        selectors: &active,
        form: &form,
    }));

    // The valid row carries its blind spot…
    assert!(
        ui.find(format!(
            "cannot see: {}",
            zengui::scope::blind_spot("demo/**")
        ))
        .is_ok()
    );
    // …and the invalid row carries the validator's verdict, verbatim —
    // which is where RFC 03 §2 reaches the screen.
    let err = zengui::scope::validate_selector("demo/$*/x").unwrap_err();
    assert!(
        // Its own words, no leading spaces since #559: the row's error is a
        // `kit::error` callout under it, grouped by the frame, not indented.
        ui.find(err.to_string()).is_ok(),
        "the row's own error must be beside it"
    );
    assert!(ui.find("apply — the scope becomes custom").is_ok());
    assert!(ui.find("add selector").is_ok());
    assert!(
        ui.find("fork into custom and edit").is_err(),
        "already editing — nothing to fork"
    );
}

// ── The Settings overlay (#188) ──────────────────────────────────────────

/// A `Settings` for the overlay tests, spelled out — the same shape
/// `app.rs`'s `test_app` uses.
fn launch_settings() -> zengui::config::Settings {
    zengui::config::Settings {
        namespace: String::new(),
        connect: vec![],
        listen: vec![],
        scouting: None,
        zenoh_config: None,
        contracts: vec![],
        timeout_secs: 5,
        scope: zengui::scope::ScopePreset::Everything,
        selectors: vec![],
        eager: false,
        admin_declared: false,
        echo_lines: 2000,
        history_entries: 200,
        max_keys: 50_000,
    }
}

/// The Settings overlay (#188): every knob it controls is labelled live or
/// on-reconnect, each bound states the cost of raising it — the key table in
/// the status strip's own words — and every `Settings` field it does not
/// control is documented as owned elsewhere.
#[test]
fn the_settings_overlay_labels_live_against_reconnect_and_states_each_cost() {
    use zengui::view::settings::{self, SettingsData, SettingsForm};

    let cfg = launch_settings();
    let mut form = SettingsForm::default();
    form.seed(&cfg);
    let mut ui = simulator::<Message, _, _>(settings::pane(SettingsData {
        settings: &cfg,
        form: &form,
        theme: "dark",
        zoom: 1.0,
        echo: (1200, 34, 5),
        history: Some((80, 3)),
        keys: (120, 7),
    }));

    // The groups, and the live-vs-reconnect split.
    assert!(ui.find("bounds — applied live").is_ok());
    assert!(ui.find("bounds — take effect on reconnect").is_ok());

    // Each bound's cost, in the voice the status strip already uses — the
    // key table literally through `status::keys_text`.
    assert!(
        ui.find(format!(
            "takes effect on reconnect — a larger key table is more memory. \
             now: {}",
            zengui::view::status::keys_text(120, 7),
        ))
        .is_ok(),
        "the key-table bound must state its cost in the strip's words"
    );
    assert!(
        ui.find(
            "applies live — a larger ring is more memory and a longer filter \
             scan. now: 1200 lines held (+34 evicted, 5 lagged)"
        )
        .is_ok(),
        "the echo bound must state memory and the filter scan"
    );
    assert!(
        ui.find(
            "applies live — history keeps whole payloads so it can diff them: \
             the costliest bound per entry. now: 80 entries (+3 evicted)"
        )
        .is_ok()
    );
    // The invariant, stated where the bounds are set.
    assert!(ui.find(settings::BOUND_INVARIANT).is_ok());

    // Eager is labelled, not silently inert.
    assert!(
        ui.find(
            "takes effect on reconnect — the live equivalent is the location \
             bar's observe-scope toggle (#85)"
        )
        .is_ok()
    );

    // The contracts (#614, FL6a) name their blast radius and where they
    // are remembered — never the shared context; no registry row survives.
    assert!(ui.find("CONTRACTS").is_ok(), "settings has contracts");
    assert!(
        ui.find(
            "contracts held offline seed the store, so a revision held here is \
             never retrieved from the bus (spec §8.5) — applying a change \
             reloads them, and every verdict and decode judged under the old \
             ones is dropped rather than left on screen (O4). Remembered by this \
             window, never written into a shared context."
        )
        .is_ok()
    );
    assert!(ui.find("REGISTRY DIRS").is_err(), "no registry row");

    // Every field the overlay does not control is documented as owned
    // elsewhere — namespace, session setup, scope.
    assert!(
        ui.find(
            "namespace: (empty — keys start at zk2/) — the location bar's \
             namespace picker owns it"
        )
        .is_ok()
    );
    assert!(
        ui.find(
            "connect (none), listen (none), scouting (unset — off unless a \
             config file says otherwise), zenoh config (none) — session \
             setup, owned by the Connect overlay (Ctrl+Shift+C); a change \
             there reopens the session"
        )
        .is_ok()
    );
    assert!(
        ui.find(
            "scope (everything, 0 custom selectors) — the location bar's \
             scope picker and its selectors editor own them"
        )
        .is_ok()
    );

    // The apply and the reconnect it labels toward are both offered.
    assert!(ui.find("apply").is_ok());
    assert!(ui.find("reconnect now").is_ok());
}
