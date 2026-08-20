//! Context reciprocity (issue #67): one store, two explorers.
//!
//! Its **own** test binary on purpose. The context store resolves its path
//! through `ZENKEY_EXPLORER_CONFIG_DIR`, which is process-global — a test that
//! moves it inside a binary shared with other tests silently redirects them
//! too. (That exact hazard bit zenctl's completion tests; the lesson is
//! recorded here rather than re-learned.)
//!
//! One test, not several, for a related reason: the store's write path is
//! read-modify-write over a single file, so two tests saving concurrently is a
//! last-writer-wins race in the *test*, not a bug in the store. Sharing a
//! process means sharing the file; the honest response is one test that owns
//! it.

use zengui::view::contexts::ContextForm;
use zenkey_fleet::StoredContext;

/// A context written the way `zenctl context create` writes one is a context
/// this pane lists, loads and can write back — the acceptance criterion, at
/// the seam where both explorers actually meet. Includes the empty base,
/// which is a real deployment (the bus root, RFC v1.6) and must round-trip as
/// a *choice* rather than as an unset field.
#[test]
fn contexts_round_trip_between_the_two_explorers() {
    let dir = std::env::temp_dir().join("zengui-context-reciprocity");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    // SAFETY: set before any store call, in a binary with one test.
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &dir) };

    // Write two contexts exactly as zenctl does — a based one and a bus-root
    // one, because those take different paths through the base resolution.
    let mut config = zenkey_fleet::context_store::load().expect("an absent config is empty");
    config.contexts.insert(
        "lab".into(),
        StoredContext {
            base: Some("acme".into()),
            connect: vec!["tcp/127.0.0.1:7449".into()],
            listen: vec![],
            registry: vec!["/abs/registry".into()],
            scouting: Some(false),
            timeout: Some(9),
            zenoh_config: None,
        },
    );
    config.contexts.insert(
        "root".into(),
        StoredContext {
            base: Some(String::new()),
            connect: vec!["tcp/127.0.0.1:7447".into()],
            listen: vec![],
            registry: vec![],
            scouting: Some(true),
            timeout: None,
            zenoh_config: None,
        },
    );
    config.current = Some("lab".into());
    zenkey_fleet::context_store::save(&config).expect("save");

    // The GUI side reads them back, field for field.
    let reloaded = zenkey_fleet::context_store::load().expect("reload");
    assert_eq!(reloaded.current.as_deref(), Some("lab"));
    let names: Vec<&String> = reloaded.contexts.keys().collect();
    assert_eq!(names, ["lab", "root"], "the picker's option list");

    let mut form = ContextForm::default();
    form.load_from("lab", reloaded.contexts.get("lab").expect("lab"));
    assert_eq!(form.name, "lab");
    assert_eq!(form.base, "acme");
    assert_eq!(form.connect, "tcp/127.0.0.1:7449");
    assert!(!form.scouting);
    // The two fields the pane had no widget for and therefore used to destroy
    // (issue #194) are now loaded like the rest.
    assert_eq!(form.timeout, "9", "the timeout reaches the editor");
    assert_eq!(
        form.registry, "/abs/registry",
        "and so do the registry dirs"
    );

    // …and what the pane would write re-reads as the same thing.
    let round = form.to_stored().expect("valid");
    assert_eq!(round.base.as_deref(), Some("acme"));
    assert_eq!(round.connect, ["tcp/127.0.0.1:7449"]);
    assert_eq!(round.scouting, Some(false));
    assert_eq!(round.timeout, Some(9), "a zenctl-authored timeout survives");
    assert_eq!(round.registry, [std::path::PathBuf::from("/abs/registry")]);

    // The structural guard, for the *next* field added to the store: an edit
    // merges over what is there rather than replacing it. Simulated by a form
    // that has not loaded the field at all.
    let mut cfg = zenkey_fleet::context_store::load().expect("reload");
    let bare = ContextForm {
        name: "lab".into(),
        connect: "tcp/127.0.0.1:7450".into(),
        ..ContextForm::default()
    };
    zenkey_fleet::context_store::upsert(&mut cfg, "lab", |c| {
        // A form that renders only endpoints writes only endpoints.
        c.connect = zengui::view::contexts::endpoints(&bare.connect);
    });
    let merged = cfg.contexts.get("lab").expect("lab");
    assert_eq!(merged.connect, ["tcp/127.0.0.1:7450"], "the edit landed");
    assert_eq!(
        merged.timeout,
        Some(9),
        "an edit must keep what it did not render"
    );
    assert_eq!(merged.registry, [std::path::PathBuf::from("/abs/registry")]);

    // The empty base survives as an empty base, not as "unset" — a context
    // that pins the bus root must override a differently-based default rather
    // than fall through to it.
    form.load_from("root", reloaded.contexts.get("root").expect("root"));
    assert_eq!(form.base, "");
    assert!(form.scouting);
    assert_eq!(
        form.to_stored().expect("valid").base,
        Some(String::new()),
        "an empty base is stored, not dropped"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
