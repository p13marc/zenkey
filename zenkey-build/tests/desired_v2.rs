//! What a consumer's build makes of a contract that lists `desired.v2`
//! (`spec/profiles/desired/v2.md`, #986's J2): the scenarios' `tasking.v2`
//! and `worker.v2` (`zenkey/tests/contracts/`) lint and generate, the
//! target's report typed by the standard schema's packed `Report`; and an
//! invalid declaration of either kind fails the build, as `config.v2`'s
//! lint does.

use std::path::{Path, PathBuf};

use zenkey_build::{Config, Error};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// A scratch copy of the two contracts and what they list, at the paths
/// they name: `worker.v2` lists the profile's schema by its path from
/// `zenkey/tests/contracts/`.
fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zk2-desired-v2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let contracts = d.join("zenkey/tests/contracts");
    let schema = d.join("spec/profiles/desired");
    std::fs::create_dir_all(&contracts).unwrap();
    std::fs::create_dir_all(&schema).unwrap();
    for f in ["tasking.v2.toml", "worker.v2.toml", "tasking.json"] {
        std::fs::copy(
            repo().join("zenkey/tests/contracts").join(f),
            contracts.join(f),
        )
        .unwrap();
    }
    std::fs::copy(
        repo().join("spec/profiles/desired/desired.v2.json"),
        schema.join("desired.v2.json"),
    )
    .unwrap();
    d
}

fn generate(d: &Path) -> Result<zenkey_build::Generated, Error> {
    Config::new()
        .contracts_dir(d.join("zenkey/tests/contracts"))
        .out_file(d.join("out/zk2.rs"))
        .generate()
}

#[test]
fn the_scenarios_contracts_generate_the_packed_report() {
    let d = scratch("ok");
    let g = generate(&d).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.interfaces, ["tasking.v2", "worker.v2"]);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    let code = std::fs::read_to_string(&g.out_file).unwrap();
    for item in ["pub mod tasking_v2 {", "pub mod worker_v2 {"] {
        assert!(code.contains(item), "{item}");
    }
    // The standard schema's one type, under its file's stem, with the
    // fields core §7.6 reads: a struct, and its module's enums.
    let packed = std::fs::read_to_string(d.join("out/zk2.d/packed/desired_v2.rs"))
        .unwrap_or_else(|_| code.clone());
    for item in [
        "pub struct Report",
        "pub source:",
        "pub outcome:",
        "pub last_known: bool",
        "pub in_force:",
        "pub observed_run16:",
        "pub observed_run32:",
        "pub observed_seq:",
        "pub revision:",
        "pub in_force_revision:",
        "pub code:",
        "pub pointer:",
        "pub detail:",
    ] {
        assert!(packed.contains(item), "{item} in:\n{packed}");
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_invalid_declaration_fails_the_build() {
    // §2.2: a horizon above 0 under freshness.v1.
    let d = scratch("horizon");
    let f = d.join("zenkey/tests/contracts/tasking.v2.toml");
    let text = std::fs::read_to_string(&f).unwrap();
    let text = text
        .replace("\"freshness.v2\"", "\"freshness.v1\"")
        .replace("\"freshness.ttl_s\" = 0", "\"freshness.ttl_s\" = 600");
    std::fs::write(&f, text).unwrap();
    let e = generate(&d).unwrap_err();
    let Error::Lint { file, report } = &e else {
        panic!("{e}")
    };
    assert!(file.ends_with("tasking.v2.toml"), "{file}");
    assert!(
        report.contains("error[desired.v2 horizon_v1] state/tasks/{system}/{service}"),
        "{report}"
    );
    std::fs::remove_dir_all(d).unwrap();

    // §2.10: a report resource naming a role that is no requirement.
    let d = scratch("role");
    let f = d.join("zenkey/tests/contracts/worker.v2.toml");
    let text = std::fs::read_to_string(&f).unwrap();
    std::fs::write(
        &f,
        text.replace(
            "\"desired.report\"   = \"task\"",
            "\"desired.report\"   = \"tasks\"",
        ),
    )
    .unwrap();
    let e = generate(&d).unwrap_err();
    let Error::Lint { report, .. } = &e else {
        panic!("{e}")
    };
    assert!(
        report.contains("error[desired.v2 not_a_role] state/desired/task"),
        "{report}"
    );
    assert!(!d.join("out/zk2.rs").exists(), "nothing is generated");
    std::fs::remove_dir_all(d).unwrap();
}
