//! What zk2's port refuses, through the one gate both `run` and
//! `check-config` pass (#614, FL2): [`zenwatch::load_checked`]. Each
//! refusal is an exit 2 naming where its question went — `alerts` over
//! v1's alert plane (what to write instead, #800 AD), v1's `origin-down`,
//! inhibition switched on, a version-1 state file — and nothing is
//! silently ignored. (v1's live `alerts` suite came back on alarms.v1 in
//! `tests/alerts.rs`; the transcripts in `tests/cmd/` pin the same
//! refusals as the binary prints them.)

use std::path::{Path, PathBuf};

use zenwatch::cli::DeploymentArgs;
use zenwatch::exit::{REFUSED, code_for};

const SINKS: &str = r#"sinks: { hook: { kind: "webhook", url: "https://example.org/h" } }"#;

/// A config file with `body` (rules, and whatever else) beside one sink.
fn config(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("{{ {body}, {SINKS} }}")).unwrap();
    path
}

/// Refused with exit 2, and the message (with every problem printed above
/// it, as `check-config` prints them) names `needle`.
fn refused(path: &Path, needle: &str) {
    let e = match zenwatch::load_checked(path, &DeploymentArgs::default()) {
        Ok(_) => panic!("{} was not refused", path.display()),
        Err(e) => e,
    };
    assert_eq!(code_for(&e), REFUSED, "{e}");
    let text = format!("{e:#}");
    assert!(
        text.contains(needle) || text.contains("problem(s) — nothing was run"),
        "{text}"
    );
}

#[test]
fn the_ports_refusals_are_exit_two_through_the_shared_gate() {
    // SAFETY: a test-local config root nobody else reads, set once.
    let home = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", home.path()) };
    let dir = tempfile::tempdir().unwrap();

    let alerts = config(
        dir.path(),
        "alerts.json5",
        r#"rules: [{ name: "a", rule: "alerts v1/*/state/*/alert/*", sinks: ["hook"] }]"#,
    );
    refused(&alerts, "v1's alert plane");
    let problems = zenwatch::config::check(&zenwatch::config::load(&alerts).unwrap());
    assert!(
        problems[0].message.contains("v1's alert plane")
            && problems[0]
                .message
                .contains("zk2/*/*/alarms.v1/state/alarms/*"),
        "{problems:?}"
    );

    let origin = config(
        dir.path(),
        "origin-down.json5",
        r#"rules: [{ name: "h", rule: "origin-down h-aaaaaaaaaaaa", sinks: ["hook"] }]"#,
    );
    refused(&origin, "instance-gone");
    let problems = zenwatch::config::check(&zenwatch::config::load(&origin).unwrap());
    assert!(
        problems[0].message.contains("instance-gone"),
        "{problems:?}"
    );

    let inhibit = config(
        dir.path(),
        "inhibit.json5",
        r#"rules: [{ name: "d", rule: "dropped", sinks: ["hook"] }],
           discipline: { inhibit: { enabled: true, depth: 4 } }"#,
    );
    refused(&inhibit, "inhibition was removed");

    // A version-1 state file: refused by the gate, before any session.
    let state = dir.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"version":1,"saved_at":"x","entries":[],"evicted_total":0}"#,
    )
    .unwrap();
    let with_state = config(
        dir.path(),
        "state.json5",
        &format!(
            r#"rules: [{{ name: "d", rule: "dropped", sinks: ["hook"] }}],
               state_file: {:?}"#,
            state.display().to_string()
        ),
    );
    refused(&with_state, "version 1");

    // The same configs, fixed, pass the gate. (A literal address for the
    // daemon's own service: nothing minted, whatever this host holds.)
    let fine = config(
        dir.path(),
        "fine.json5",
        r#"rules: [{ name: "tc", rule: "instance-gone */tc", sinks: ["hook"] }],
           discipline: { inhibit: { enabled: false } },
           publish: { address: "ops-1/zenwatch" }"#,
    );
    let loaded = zenwatch::load_checked(&fine, &DeploymentArgs::default()).expect("runnable");
    assert_eq!(loaded.bus.namespace, "");
    assert!(loaded.contracts.is_empty());
    assert_eq!(
        loaded.publish.map(|p| p.address.to_string()).as_deref(),
        Some("ops-1/zenwatch")
    );

    // Contracts that load nothing, and a namespace that is not one.
    let empty = tempfile::tempdir().unwrap();
    let e = zenwatch::load_checked(
        &fine,
        &DeploymentArgs {
            contracts: vec![empty.path().to_path_buf()],
            ..DeploymentArgs::default()
        },
    )
    .err()
    .expect("refused");
    assert_eq!(code_for(&e), REFUSED);
    let e = zenwatch::load_checked(
        &fine,
        &DeploymentArgs {
            namespace: Some("prod/*".into()),
            ..DeploymentArgs::default()
        },
    )
    .err()
    .expect("refused");
    assert_eq!(code_for(&e), REFUSED);
    // Real contracts load, by flag beside the config's.
    let tcgui = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/tcgui");
    let loaded = zenwatch::load_checked(
        &fine,
        &DeploymentArgs {
            contracts: vec![tcgui],
            namespace: Some("site".into()),
            ..DeploymentArgs::default()
        },
    )
    .expect("runnable");
    assert!(loaded.contracts.len() >= 3, "{}", loaded.contracts.len());
    assert_eq!(loaded.bus.namespace, "site");
}

/// `across_face` (#990 V7; core §8.5, observer §7.5): zenoh-modem's rf0
/// owner across its `rf` face. An engine rule on `frames/fast` loads, its
/// selector the scope's exact key, the timeout at least the face's 15 s
/// reply timeout and the sessions' far side the face's; a rule whose
/// selector meets none of the scope, one that reads presence, the
/// scheduled doctor, a wildcard address, and a policy without its face
/// are each refused, exit 2.
#[test]
fn across_a_face_a_rule_is_narrowed_to_the_faces_scope_or_refused() {
    // SAFETY: a test-local config root nobody else reads, set once.
    let home = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("ZENKEY_EXPLORER_CONFIG_DIR", home.path()) };
    let dir = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let modem = root.join("examples/zk2/zenoh-modem/modem.v3.toml");
    let policy = root.join("examples/zk2/zenoh-modem/faces/zenohd-rf0.face.toml");
    let across = |extra: &str| {
        format!(
            r#"across_face: {{ addresses: ["h-3fa9c2d41b7e/rf0"], contracts: [{modem:?}]{extra} }},
               publish: false"#
        )
    };
    let face = format!(r#", policy: {policy:?}, face: "rf""#);
    let fast = "zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/frames/fast";

    let ok = config(
        dir.path(),
        "ok.json5",
        &format!(
            r#"{}, rules: [{{ name: "fast", rule: "rate-below {fast} 1", sinks: ["hook"] }}]"#,
            across(&face)
        ),
    );
    let loaded = zenwatch::load_checked(&ok, &DeploymentArgs::default()).expect("runnable");
    let a = loaded.across.expect("across a face");
    assert_eq!(a.selectors, [fast]);
    assert_eq!(a.scope.face(), Some("rf"));
    assert_eq!(loaded.bus.far.as_ref().map(|f| f.face.as_str()), Some("rf"));
    assert!(
        loaded.bus.timeout >= std::time::Duration::from_secs(15),
        "{:?}",
        loaded.bus.timeout
    );

    let elsewhere = config(
        dir.path(),
        "elsewhere.json5",
        &format!(
            r#"{}, rules: [{{ name: "x", rule: "rate-below zk2/lab/other/** 1", sinks: ["hook"] }}]"#,
            across(&face)
        ),
    );
    refused(&elsewhere, "reaches none of the selectors across the face");

    let presence = config(
        dir.path(),
        "presence.json5",
        &format!(
            r#"{}, rules: [{{ name: "gone", rule: "instance-gone h-3fa9c2d41b7e/rf0", sinks: ["hook"] }}]"#,
            across("")
        ),
    );
    refused(&presence, "do not cross a constrained face");

    let doctor = config(
        dir.path(),
        "doctor.json5",
        &format!(
            r#"{}, doctor: {{ every_h: 6, sinks: ["hook"] }},
               rules: [{{ name: "fast", rule: "rate-below {fast} 1", sinks: ["hook"] }}]"#,
            across("")
        ),
    );
    refused(&doctor, "do not cross a constrained face");

    let wildcard = config(
        dir.path(),
        "wildcard.json5",
        &format!(
            r#"across_face: {{ addresses: ["*/rf0"], contracts: [{modem:?}] }}, publish: false,
               rules: [{{ name: "fast", rule: "rate-below {fast} 1", sinks: ["hook"] }}]"#
        ),
    );
    refused(&wildcard, "SYSTEM/SERVICE");

    let half = config(
        dir.path(),
        "half.json5",
        &format!(
            r#"{}, rules: [{{ name: "fast", rule: "rate-below {fast} 1", sinks: ["hook"] }}]"#,
            across(&format!(", policy: {policy:?}"))
        ),
    );
    refused(&half, "a policy needs the face");
}
