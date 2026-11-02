//! The `zk2` binary's exit contract (#618): 0 asked and clean, 1 a finding,
//! 2 no verdict.

use std::path::{Path, PathBuf};
use std::process::Command;

fn spec() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/conformance")
}

fn zk2(args: &[&str]) -> i32 {
    Command::new(env!("CARGO_BIN_EXE_zk2"))
        .args(args)
        .output()
        .expect("runs")
        .status
        .code()
        .expect("exits")
}

fn p(rel: &str) -> String {
    spec().join(rel).display().to_string()
}

#[test]
fn exit_codes() {
    assert_eq!(
        zk2(&["contract", "lint", &p("contracts/ok-minimal.toml")]),
        0
    );
    assert_eq!(
        zk2(&["contract", "lint", &p("contracts/e015-event.toml")]),
        1
    );
    assert_eq!(
        zk2(&["contract", "fingerprint", &p("contracts/ok-kinds.toml")]),
        0
    );
    let c = |case: &str, side: &str| p(&format!("compat/contract/{case}/{side}.toml"));
    assert_eq!(
        zk2(&[
            "contract",
            "compat",
            &c("optional-role-added", "old"),
            &c("optional-role-added", "new")
        ]),
        0
    );
    assert_eq!(
        zk2(&[
            "contract",
            "compat",
            &c("priority-changed", "old"),
            &c("priority-changed", "new")
        ]),
        1
    );
    assert_eq!(
        zk2(&[
            "contract",
            "compat",
            &c("resource-removed", "old"),
            &c("resource-removed", "new")
        ]),
        1
    );
    assert_eq!(zk2(&["contract", "check-history", &p("history/ok")]), 0);
    assert_eq!(
        zk2(&["contract", "check-history", &p("history/tampered")]),
        1
    );
    assert_eq!(
        zk2(&[
            "contract",
            "compat",
            &p("bundles/valid.bundle.json"),
            &p("bundles/valid.bundle.json")
        ]),
        0
    );
    assert_eq!(zk2(&["contract", "nope"]), 2);
    assert_eq!(zk2(&["contract", "fingerprint", "/nonexistent.toml"]), 1);
}

/// What `zk2` prints and how it exits, for one run.
fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_zk2"))
        .args(args)
        .output()
        .expect("runs");
    (
        out.status.code().expect("exits"),
        String::from_utf8(out.stdout).expect("UTF-8"),
    )
}

/// `zk2 catalog` (core §8.6; #980 C1a): `new` writes a first catalog, `add`
/// its successor, `id` and `check` read one; 1 for a refused catalog, one
/// that does not succeed its predecessor or lacks a contract, 2 for usage
/// and unreadable files.
#[test]
fn catalog_exit_codes() {
    let dir = std::env::temp_dir().join(format!("zk2-catalog-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let at = |name: &str| dir.join(name).display().to_string();
    let modem = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples/zk2/zenoh-modem/modem.v3.toml")
        .display()
        .to_string();
    // The history's first file by name, so the pick never hangs on the
    // file system's order.
    let mut revisions: Vec<_> = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/profiles/.history/health.v1")
        .read_dir()
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    revisions.sort();
    let health = revisions[0].display().to_string();

    let (code, first) = run(&["catalog", "new", "--epoch", "1767225600", &modem]);
    assert_eq!(code, 0);
    std::fs::write(at("a.json"), &first).unwrap();
    let (code, id_a) = run(&["catalog", "id", &at("a.json")]);
    assert_eq!(code, 0);
    assert_eq!(id_a.trim().len(), 8);

    // A bundle from a history is a revision too; the successor names `a`.
    let (code, second) = run(&["catalog", "add", &at("a.json"), &health, &modem]);
    assert_eq!(code, 0);
    assert!(second.contains(id_a.trim()), "{second}");
    std::fs::write(at("b.json"), &second).unwrap();
    assert_eq!(
        zk2(&[
            "catalog",
            "check",
            &at("b.json"),
            "--previous",
            &at("a.json"),
            &modem,
            &health
        ]),
        0
    );
    // Succession is one way, and adding nothing writes the input unchanged.
    assert_eq!(
        zk2(&[
            "catalog",
            "check",
            &at("a.json"),
            "--previous",
            &at("b.json")
        ]),
        1
    );
    assert_eq!(run(&["catalog", "add", &at("b.json"), &modem]), (0, second));
    // A contract the catalog does not hold is a finding.
    assert_eq!(zk2(&["catalog", "check", &at("a.json"), &health]), 1);
    // The smallest catalog, and its default epoch of 0.
    let (code, empty) = run(&["catalog", "new"]);
    assert_eq!(code, 0);
    assert_eq!(
        empty.trim(),
        r#"{"entries":[],"epoch":0,"format":"zk2-catalog/1"}"#
    );

    std::fs::write(
        at("refused.json"),
        r#"{"format": "zk2-catalog/1", "epoch": -0, "entries": []}"#,
    )
    .unwrap();
    assert_eq!(zk2(&["catalog", "id", &at("refused.json")]), 1);
    assert_eq!(zk2(&["catalog", "check", &at("refused.json")]), 1);
    assert_eq!(zk2(&["catalog", "add", &at("refused.json"), &modem]), 1);

    assert_eq!(zk2(&["catalog", "id", &at("missing.json")]), 2);
    assert_eq!(zk2(&["catalog", "new", "--epoch", "-1"]), 2);
    assert_eq!(zk2(&["catalog", "new", "--epoch", "01"]), 2);
    assert_eq!(zk2(&["catalog", "new", "--epoch", "9007199254740992"]), 2);
    assert_eq!(zk2(&["catalog", "new", &at("missing.toml")]), 2);
    assert_eq!(zk2(&["catalog", "add", &at("a.json")]), 2);
    assert_eq!(
        zk2(&["catalog", "check", &at("a.json"), "--previus", "x"]),
        2
    );
    let invalid = p("contracts/e015-event.toml");
    assert_eq!(zk2(&["catalog", "new", &invalid]), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `zk2 contract compat --history` of a revision the history root holds
/// (core §9.7, zk2py round 97's F-456): it passed its review when it was
/// published, so its review is reported as accepted and the exit is 0.
/// The same revision against a root that holds only its predecessor is a
/// review not yet accepted, exit 1. `health.v1`'s revision 1.1 uses
/// `freshness.v2` where 1.0 used `freshness.v1` (`uses_changed`).
#[test]
fn a_published_review_is_accepted() {
    let profiles = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles");
    let contract = profiles.join("health/health.v1.toml").display().to_string();
    let history = profiles.join(".history").display().to_string();
    let (code, out) = run(&["contract", "compat", "--history", &history, &contract]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("same revision as a published one"), "{out}");
    assert!(out.contains("review [uses_changed]"), "{out}");
    assert!(out.contains("accepted: the revision is published"), "{out}");

    let dir = std::env::temp_dir().join(format!("zk2-accepted-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("health.v1")).unwrap();
    // Revision 1.0 alone.
    let first = "e9dbcdcb2fc325ca3121fdcacf7badd0091f62d97ab4c7d7a5f022dd8d6671df.bundle.json";
    std::fs::copy(
        profiles.join(".history/health.v1").join(first),
        dir.join("health.v1").join(first),
    )
    .unwrap();
    let root = dir.display().to_string();
    let (code, out) = run(&["contract", "compat", "--history", &root, &contract]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("review [uses_changed]"), "{out}");
    assert!(!out.contains("accepted"), "{out}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `zk2 config derive` on zenoh-modem's `sx1262_config.v1` (#985 G4)
/// prints exactly the five derived types its packed file holds after its
/// three group types, and `contract lint` finds nothing in it.
#[test]
fn config_derive_reproduces_the_sx1262s_derived_types() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2/zenoh-modem/config");
    let contract = dir.join("sx1262_config.v1.toml").display().to_string();
    let out = Command::new(env!("CARGO_BIN_EXE_zk2"))
        .args(["config", "derive", &contract])
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let packed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("schemas/sx1262_config.json")).expect("the packed file"),
    )
    .expect("JSON");
    let names = |v: &serde_json::Value| {
        let mut n: Vec<String> = v["types"]
            .as_object()
            .expect("types")
            .keys()
            .cloned()
            .collect();
        n.sort_unstable();
        n
    };
    let derived = names(&printed);
    assert_eq!(
        derived,
        ["Control", "Error", "Record", "SetDuty", "SetRadio"],
        "Record, a Set<T> per hot or reach group, Control and Error"
    );
    assert_eq!(
        names(&packed),
        [
            "Control",
            "Duty",
            "Error",
            "Modulation",
            "Radio",
            "Record",
            "SetDuty",
            "SetRadio"
        ],
        "the three group types and the five derived ones"
    );
    for n in &derived {
        assert_eq!(printed["types"][n], packed["types"][n], "{n}");
    }
    assert_eq!(zk2(&["contract", "lint", &contract]), 0);
}

/// `zk2 config derive` prints the types config.v2 derives from a contract's
/// declaration (config.v2 §3): for the test contract `rfcfg.v1`, exactly
/// the eight derived types its packed file holds.
#[test]
fn config_derive_writes_the_derived_types() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/config/conformance");
    let contract = dir.join("contracts/rfcfg.v1.toml");
    let out = Command::new(env!("CARGO_BIN_EXE_zk2"))
        .args(["config", "derive", &contract.display().to_string()])
        .output()
        .expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let packed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("contracts/rfcfg.json")).expect("rfcfg.json"),
    )
    .expect("JSON");
    let types = printed["types"].as_object().expect("types");
    let names: Vec<&str> = types.keys().map(String::as_str).collect();
    let mut want = vec![
        "Control", "Error", "Record", "SetDuty", "SetLink", "SetQueue", "SetRadio", "SetSim",
    ];
    want.sort_unstable();
    let mut got = names.clone();
    got.sort_unstable();
    assert_eq!(got, want);
    for n in names {
        assert_eq!(printed["types"][n], packed["types"][n], "{n}");
    }
    // `contract lint` runs config.v2's lint: the test contract has no finding.
    assert_eq!(
        zk2(&["contract", "lint", &contract.display().to_string()]),
        0
    );
    assert_eq!(
        zk2(&["config", "derive", &p("contracts/ok-minimal.toml")]),
        1
    );
    assert_eq!(
        zk2(&["config", "derive", &p("contracts/no-such-file.toml")]),
        2
    );
}
