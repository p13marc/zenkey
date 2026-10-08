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
