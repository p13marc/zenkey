//! The cross-explorer contract of issue #35: a context created by one
//! frontend resolves identically in the other, because both go through this
//! store. Runs in its own test binary so the env-var override cannot race
//! other tests.

use zenkey_fleet::context_store::{ConfigFile, StoredContext, active, config_path, load, save};

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
