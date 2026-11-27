//! What a consumer's build makes of a packed type whose layout tag is 0
//! (core §7.6, §9.2's E051; #1152): the tag a record of any other kind
//! carries, so a tool holding no type would sniff its records. The lint is
//! `zenkey-model`'s, and the build runs it: the contract fails it, and
//! nothing is generated. The same type with a bound moved has the tag 11,
//! and generates.

use std::path::{Path, PathBuf};

use zenkey_build::{Config, Error};

/// An int from 0 to `maximum`, then a marker: at 127 its layout form has
/// the tag 0, at 100 the tag 11 (`spec/conformance/contracts/e051-packed-tag`).
fn scratch(name: &str, maximum: u32) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zk2-packed-tag-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("beacon.json"),
        format!(
            r#"{{"types": {{"Level": {{"fields": [{{"name": "a", "type": "int", "minimum": 0, "maximum": {maximum}}}, "..."]}}}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        d.join("beacon.v1.toml"),
        r#"[interface]
name = "beacon"
major = 1
minor = 0

[schemas]
packed = ["beacon.json"]

[resources.level]
kind = "stream"
type = "packed:Level"
"#,
    )
    .unwrap();
    d
}

fn generate(d: &Path) -> Result<zenkey_build::Generated, Error> {
    Config::new()
        .contracts_dir(d)
        .out_file(d.join("out/zk2.rs"))
        .generate()
}

#[test]
fn a_packed_type_at_the_tag_0_fails_the_build() {
    let d = scratch("zero", 127);
    let e = generate(&d).unwrap_err();
    let Error::Lint { file, report } = &e else {
        panic!("{e}")
    };
    assert!(file.ends_with("beacon.v1.toml"), "{file}");
    assert!(report.contains("E051"), "{report}");
    assert!(report.contains("types.Level"), "{report}");
    assert!(!d.join("out/zk2.rs").exists(), "nothing is generated");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn the_same_type_with_a_bound_moved_generates() {
    let d = scratch("moved", 100);
    let g = generate(&d).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.interfaces, ["beacon.v1"]);
    std::fs::remove_dir_all(d).unwrap();
}
