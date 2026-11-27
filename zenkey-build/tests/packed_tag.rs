//! What a consumer's build makes of a packed type whose layout tag is 0
//! (core §7.6, §9.2's E051; #1152): the tag a record of any other kind
//! carries, so a tool holding no type would sniff its records. The lint is
//! `zenkey-model`'s, and the build runs it: the contract fails it, and
//! nothing is generated. The same type with a bound moved has the tag 11,
//! and generates. A type a profile derives is set aside (core §7.6, F-619):
//! config.v2's `Record` at 0 generates.

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

/// A type a profile derives may have the tag 0, and E051 does not read it
/// (core §7.6, config.v2 0.6 §3; F-619): `derive.json` case 2's
/// declaration, a contract group alone, derives a `Record` at 0. Its
/// contract, `rfcfg.v1`'s shape with no `set/<group>` and the types
/// `zk2 config derive` writes, builds.
#[test]
fn a_config_contracts_derived_record_at_the_tag_0_generates() {
    use serde_json::{Value, json};
    use zenkey_model::config::v2::{CONTROLS, Interface};

    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles/config");
    let derive: Value = serde_json::from_str(
        &std::fs::read_to_string(spec.join("conformance/v2/derive.json")).unwrap(),
    )
    .unwrap();
    let annotation = &derive["cases"][2]["groups"];
    assert_eq!(
        annotation,
        &json!([{"name": "modulation", "class": "contract", "type": "packed:Modulation"}])
    );
    let modulation = derive["schema"]["types"]["Modulation"].clone();
    let group = json!({ "types": { "Modulation": modulation } });
    let i = Interface::read(annotation, &[("modcfg", &group)]).expect("case 2 is valid");
    assert_eq!(i.derived.record().tag(), 0, "derive.json case 2's Record");
    let mut types: Value =
        serde_json::from_str::<Value>(&i.derived.to_text()).unwrap()["types"].take();
    types["Modulation"] = modulation;

    let d = std::env::temp_dir().join(format!("zk2-packed-tag-derived-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(spec.join("config.v2.json"), d.join("config.v2.json")).unwrap();
    std::fs::write(
        d.join("modcfg.json"),
        serde_json::to_string(&json!({ "types": types })).unwrap(),
    )
    .unwrap();
    let mut text = r#"[interface]
name  = "modcfg"
major = 1
minor = 0
uses  = ["config.v2", "freshness.v2"]

[schemas]
packed     = ["modcfg.json"]
jsonschema = ["config.v2.json"]

[defaults.operation]
idempotent = false
fanout     = "forbidden"

[resources.config]
kind = "state"
type = "packed:Record"

[resources.config.annotations]
"freshness.ttl_s" = 60
"config.groups"   = [{ name = "modulation", class = "contract", type = "packed:Modulation" }]

[resources.change]
kind      = "event"
type      = "json:ConfigChangeEvent"
rate      = "burst(600/h)"
retention = "7d"
"#
    .to_owned();
    for op in CONTROLS {
        text += &format!(
            "\n[resources.{op}]\nkind     = \"operation\"\nrequest  = \"packed:Control\"\n\
             response = \"packed:Record\"\nerror    = \"packed:Error\"\n"
        );
        if op == "persist" {
            text += "optional = true\n";
        }
    }
    std::fs::write(d.join("modcfg.v1.toml"), text).unwrap();
    let g = generate(&d).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.interfaces, ["modcfg.v1"]);
    assert!(d.join("out/zk2.rs").exists());
    std::fs::remove_dir_all(d).unwrap();
}
