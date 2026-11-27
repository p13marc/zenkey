//! What a consumer's build makes of a service contract that carries
//! `health.v2`'s tables of words (`spec/profiles/health/v2.md` §2.10, §4;
//! #986's Q2): the scenarios' `tc.v1` (`zenkey/tests/contracts/health/`)
//! lints and generates, and an invalid table fails the build, as
//! `desired.v2`'s and `config.v2`'s lints do. An owner would write no word
//! of such a table, and a reader would show numbers.

use std::path::{Path, PathBuf};

use zenkey_build::{Config, Error};

/// A scratch copy of the scenarios' `tc.v1`, alone: the directory beside
/// it holds an earlier revision of the same interface.
fn scratch(name: &str, edit: impl Fn(String) -> String) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zk2-health-v2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts/health/tc.v1.toml"),
    )
    .unwrap();
    let edited = edit(text.clone());
    std::fs::write(d.join("tc.v1.toml"), &edited).unwrap();
    assert!(
        name == "ok" || edited != text,
        "{name}: the edit changed the file"
    );
    d
}

fn generate(d: &Path) -> Result<zenkey_build::Generated, Error> {
    Config::new()
        .contracts_dir(d)
        .out_file(d.join("out/zk2.rs"))
        .generate()
}

#[test]
fn a_contract_that_declares_its_tables_generates() {
    let d = scratch("ok", |text| text);
    let g = generate(&d).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.interfaces, ["tc.v1"]);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_invalid_table_fails_the_build() {
    // `value`: a table holds names, each `[a-z][a-z0-9_]*`, none twice.
    for (name, codes) in [
        ("form", r#"["TC restarted"]"#),
        ("twice", r#"["tc_netns_restarted", "tc_netns_restarted"]"#),
    ] {
        let d = scratch(name, |text| {
            text.replace(r#"["tc_netns_restarted"]"#, codes)
        });
        let e = generate(&d).unwrap_err();
        let Error::Lint { file, report } = &e else {
            panic!("{e}")
        };
        assert!(file.ends_with("tc.v1.toml"), "{file}");
        assert!(
            report.contains("error[health.v2 value] state/qdisc: health.codes: "),
            "{name}: {report}"
        );
        assert!(!d.join("out/zk2.rs").exists(), "nothing is generated");
        std::fs::remove_dir_all(d).unwrap();
    }

    // `disagrees`: a resource that writes the key again, with another
    // list. The key is written once, on [defaults].
    let d = scratch("disagrees", |text| {
        format!(
            "{text}\n[resources.shaper]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n\
             annotations = {{ \"health.reasons\" = [\"tc_other\"] }}\n"
        )
    });
    let e = generate(&d).unwrap_err();
    let Error::Lint { report, .. } = &e else {
        panic!("{e}")
    };
    assert!(
        report.contains("error[health.v2 disagrees] state/") && report.contains("health.reasons"),
        "{report}"
    );
    assert!(
        !report.contains("health.codes"),
        "the codes' table is one list on every resource: {report}"
    );
    std::fs::remove_dir_all(d).unwrap();
}
