//! The cross-explorer contract of issue #35: a context created by one
//! frontend resolves identically in the other, because both go through this
//! store. Runs in its own test binary so the env-var override cannot race
//! other tests.

use zenkey_fleet::context_store::{
    ConfigFile, StoredContext, active, config_path, load, save, upsert,
};

#[test]
fn create_select_resolve_round_trip() {
    let dir = std::env::temp_dir().join(format!("zenkey-explorer-test-{}", std::process::id()));
    // One test, one binary: the env override is ours alone.
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", &dir) };

    // Empty store: no context, no error.
    assert!(load().unwrap().contexts.is_empty());
    assert!(active(None).unwrap().is_none());

    // "zenctl context create lab --base zensight -c tcp/…" writes…
    let mut cfg = ConfigFile::default();
    cfg.contexts.insert(
        "lab".into(),
        StoredContext {
            base: Some("zensight".into()),
            connect: vec!["tcp/127.0.0.1:7447".into()],
            scouting: Some(false),
            timeout: Some(7),
            ..Default::default()
        },
    );
    cfg.current = Some("lab".into());
    save(&cfg).unwrap();
    assert!(config_path().starts_with(&dir));

    // …and "zengui --context lab" (or the current pointer) resolves the same.
    let by_pointer = active(None).unwrap().expect("current pointer resolves");
    assert_eq!(by_pointer.base.as_deref(), Some("zensight"));
    assert_eq!(by_pointer.timeout, Some(7));
    let by_name = active(Some("lab")).unwrap().expect("named resolves");
    assert_eq!(by_name, by_pointer);

    // An explicitly named missing context is an error (the user asked for
    // something specific)…
    assert!(active(Some("nope")).is_err());

    // …while a dangling `current` pointer is a stale file, not a hard error.
    let mut cfg = load().unwrap();
    cfg.current = Some("gone".into());
    save(&cfg).unwrap();
    assert!(active(None).unwrap().is_none());

    std::fs::remove_dir_all(&dir).ok();
}

/// The hazard `upsert` exists to remove (issue #194): both explorers share one
/// store, and they do not render the same fields. An edit must keep what it
/// could not show — otherwise saving from a form with no registry widget
/// deletes the registry that `zenctl context create --registry …` wrote.
///
/// Purely in-memory, deliberately: this is about the merge, and the module doc
/// above explains why a second test that touched the env override would race
/// the first one in this same binary.
#[test]
fn an_edit_keeps_the_fields_it_could_not_show() {
    // "zenctl context create lab --base zensight --registry … --timeout 10"
    let mut cfg = ConfigFile::default();
    upsert(&mut cfg, "lab", |c| {
        c.base = Some("zensight".into());
        c.registry = vec!["/abs/registry".into()];
        c.timeout = Some(10);
    });

    // …then a connect pane whose widgets cover endpoints and scouting only.
    upsert(&mut cfg, "lab", |c| {
        c.connect = vec!["tcp/127.0.0.1:7447".into()];
        c.scouting = Some(false);
    });

    let lab = cfg.contexts.get("lab").expect("lab");
    assert_eq!(lab.connect, ["tcp/127.0.0.1:7447"], "the edit landed");
    assert_eq!(
        lab.registry,
        [std::path::PathBuf::from("/abs/registry")],
        "a field the form cannot show must survive the save"
    );
    assert_eq!(lab.timeout, Some(10), "and so must the timeout");
    assert_eq!(lab.base.as_deref(), Some("zensight"), "and the base");

    // A name that does not exist yet starts from the default, not from nothing.
    upsert(&mut cfg, "fresh", |c| c.base = Some(String::new()));
    assert_eq!(
        cfg.contexts.get("fresh").unwrap().base,
        Some(String::new()),
        "an empty base is a choice, not an absence (RFC v1.6)"
    );
    assert_eq!(cfg.contexts.get("fresh").unwrap().timeout, None);
}
