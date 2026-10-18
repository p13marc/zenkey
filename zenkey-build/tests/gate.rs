//! What fails a consumer's build (#611, `docs/zk2/codegen.md`): a lint
//! error, and a breaking revision against the history, published or not; a
//! review revision only warns, and only until it is published (spec §9.7,
//! §9.8; #768).

use std::path::{Path, PathBuf};

use zenkey_build::{Config, Error, Generated};
use zenkey_model::contract::load_path;

const V0: &str = r#"[interface]
name = "gate"
major = 1
minor = 0

[resources.level]
kind = "state"
type = { raw = "text/plain" }

[resources.ping]
kind = "operation"
request = { raw = "text/plain" }
response = { raw = "text/plain" }
"#;

/// A scratch directory with `contracts/` and its `.history`, holding `V0`
/// published.
fn published(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zk2-gate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("contracts")).unwrap();
    let f = d.join("contracts/gate.v1.toml");
    std::fs::write(&f, V0).unwrap();
    let c = load_path(&f).contract.unwrap();
    zenkey_model::history::append(&d.join("contracts/.history"), &c).unwrap();
    d
}

fn revise(d: &Path, text: &str) -> Result<Generated, Error> {
    std::fs::write(d.join("contracts/gate.v1.toml"), text).unwrap();
    Config::new()
        .contracts_dir(d.join("contracts"))
        .history_dir(d.join("contracts/.history"))
        .out_file(d.join("out/zk2.rs"))
        .generate()
}

#[test]
fn the_published_revision_generates_without_a_word() {
    let d = published("same");
    let g = revise(&d, V0).unwrap();
    assert_eq!(g.interfaces, ["gate.v1"]);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    let code = std::fs::read_to_string(&g.out_file).unwrap();
    assert!(code.contains("pub mod gate_v1 {"));
    assert!(d.join("out/zk2.d/gate.v1.bundle.json").is_file());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_breaking_revision_fails_the_build() {
    let d = published("breaking");
    let removed = V0.replace(
        "[resources.ping]\nkind = \"operation\"\nrequest = { raw = \"text/plain\" }\nresponse = { raw = \"text/plain\" }\n",
        "",
    );
    let e = revise(&d, &removed).unwrap_err();
    let Error::Breaking { iface, findings } = &e else {
        panic!("{e}")
    };
    assert_eq!(iface, "gate.v1");
    assert!(findings.contains("resource_removed"), "{findings}");
    assert!(
        !d.join("out/zk2.rs").exists(),
        "nothing is generated from a refused revision"
    );
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_review_revision_warns() {
    let d = published("review");
    let used = V0.replace("minor = 0\n", "minor = 1\nuses = [\"freshness.v1\"]\n");
    let g = revise(&d, &used).unwrap();
    assert!(
        g.warnings.iter().any(|w| w.contains("review uses_changed")),
        "{:?}",
        g.warnings
    );
    std::fs::remove_dir_all(d).unwrap();
}

/// Publishes `text` as a further revision of `d`'s history.
fn publish(d: &Path, text: &str) {
    let f = d.join("contracts/gate.v1.toml");
    std::fs::write(&f, text).unwrap();
    let c = load_path(&f).contract.unwrap();
    let (_, written) = zenkey_model::history::append(&d.join("contracts/.history"), &c).unwrap();
    assert!(written, "a new revision");
}

/// A review revision once published is accepted: building it again is
/// silent, while a review change not yet published still warns against
/// every entry (#768: zenwatch.v1 1.1 re-warned its accepted review on
/// every build).
#[test]
fn a_published_review_revision_builds_without_a_word() {
    let d = published("accepted");
    let v1_1 = V0.replace("minor = 0\n", "minor = 1\nuses = [\"freshness.v1\"]\n");
    publish(&d, &v1_1);
    let g = revise(&d, &v1_1).unwrap();
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);

    // 1.2, not yet published: review against 1.0 and against 1.1.
    let v1_2 = V0.replace("minor = 0\n", "minor = 2\nuses = [\"health.v1\"]\n");
    let g = revise(&d, &v1_2).unwrap();
    let review: Vec<_> = g
        .warnings
        .iter()
        .filter(|w| w.contains("review uses_changed"))
        .collect();
    assert_eq!(review.len(), 2, "{:?}", g.warnings);
    std::fs::remove_dir_all(d).unwrap();
}

/// Publishing runs no classifier, so a breaking revision can sit in a
/// history; building it still fails.
#[test]
fn a_published_breaking_revision_still_fails_the_build() {
    let d = published("breaking-published");
    let removed = V0.replace(
        "[resources.ping]\nkind = \"operation\"\nrequest = { raw = \"text/plain\" }\nresponse = { raw = \"text/plain\" }\n",
        "",
    );
    publish(&d, &removed);
    let e = revise(&d, &removed).unwrap_err();
    let Error::Breaking { findings, .. } = &e else {
        panic!("{e}")
    };
    assert!(findings.contains("resource_removed"), "{findings}");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_lint_error_fails_and_a_lint_warning_warns() {
    let d = published("lint");
    let e = revise(
        &d,
        &V0.replace("kind = \"state\"", "kind = \"state\"\nhistory = 0"),
    )
    .unwrap_err();
    assert!(matches!(e, Error::Lint { .. }), "{e}");
    // W104: no `minor`. A warning, so the build goes on.
    let g = revise(&d, &V0.replace("minor = 0\n", "")).unwrap();
    assert!(
        g.warnings.iter().any(|w| w.contains("W104")),
        "{:?}",
        g.warnings
    );
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn a_hint_that_names_nothing_warns_and_one_that_is_no_reference_fails() {
    let d = published("hints");
    std::fs::write(d.join("contracts/gate.v1.toml"), V0).unwrap();
    let base = Config::new()
        .contracts_dir(d.join("contracts"))
        .out_file(d.join("out/zk2.rs"));
    let g = base
        .clone()
        .json_type("json:Nope", "crate::Nope")
        .generate()
        .unwrap();
    assert!(
        g.warnings.iter().any(|w| w.contains("json:Nope")),
        "{:?}",
        g.warnings
    );
    let e = base
        .json_type("Nope", "crate::Nope")
        .generate()
        .unwrap_err();
    assert!(matches!(e, Error::Config(_)), "{e}");
    std::fs::remove_dir_all(d).unwrap();
}
