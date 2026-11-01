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
    let health = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spec/profiles/.history/health.v1")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .display()
        .to_string();

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
