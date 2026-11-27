//! What a consumer's build makes of a service contract that carries
//! `alarms.v2`'s table of rules (`spec/profiles/alarms/v2.md` §4; #984's
//! S1): a contract that declares a valid table lints and generates, and
//! an invalid one fails the build, as `health.v2`'s and `desired.v2`'s
//! lints do. An owner would write every alarm of such a table by words,
//! and a reader would show numbers.

use std::path::{Path, PathBuf};

use zenkey_build::{Config, Error};

/// §4's example: a service's own contract, holding its table on
/// `[defaults]` (`spec/conformance/contracts/ok-alarms-v2-rules.toml`).
const PLANT: &str = r#"[interface]
name  = "plant"
major = 1
minor = 0
uses  = ["alarms.v2"]

[[defaults.annotations."alarms.rules"]]
name     = "hot"
category = "sensor_health"
summary  = "Temperature {argument} C"
argument = { minimum = "-40", resolution = "0.1" }

[[defaults.annotations."alarms.rules"]]
name    = "link_down"
summary = "Link to {peer} on {port} is down"
labels  = [
  { name = "peer", values = ["r1", "r2", "r3"] },
  { name = "port", values = ["eth0", "eth1"] },
]

[[defaults.annotations."alarms.rules"]]
name    = "sensor_lost"
summary = "Sensor {sensor} lost"
labels  = [{ name = "sensor", minimum = 0, maximum = 63 }]

[resources.level]
kind = "state"
type = { raw = "text/plain" }
"#;

/// A scratch directory holding `plant.v1.toml`, edited.
fn scratch(name: &str, edit: impl Fn(&str) -> String) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zk2-alarms-v2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let edited = edit(PLANT);
    assert!(
        name == "ok" || edited != PLANT,
        "{name}: the edit changed the file"
    );
    std::fs::write(d.join("plant.v1.toml"), &edited).unwrap();
    d
}

fn generate(d: &Path) -> Result<zenkey_build::Generated, Error> {
    Config::new()
        .contracts_dir(d)
        .out_file(d.join("out/zk2.rs"))
        .generate()
}

#[test]
fn a_contract_that_declares_its_rules_generates() {
    let d = scratch("ok", str::to_owned);
    let g = generate(&d).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.interfaces, ["plant.v1"]);
    assert!(g.warnings.is_empty(), "{:?}", g.warnings);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_invalid_table_fails_the_build() {
    for (name, from, to, token) in [
        // Two rules of one name.
        ("twice", r#""sensor_lost""#, r#""hot""#, "rule_name"),
        // A placeholder that names no label of its rule.
        (
            "placeholder",
            "{sensor} lost",
            "{probe} lost",
            "placeholder",
        ),
        // A label's values beyond 24 bits.
        (
            "width",
            "maximum = 63",
            "maximum = 16777216",
            "labels_width",
        ),
        // A host-scoped label: the key's system names the host.
        ("host", r#"name = "peer""#, r#"name = "host""#, "host_label"),
        // A rule without its summary is no rule of §4's shape.
        (
            "value",
            "summary = \"Link to {peer} on {port} is down\"\n",
            "",
            "value",
        ),
    ] {
        let d = scratch(name, |text| text.replacen(from, to, 1));
        let e = generate(&d).unwrap_err();
        let Error::Lint { file, report } = &e else {
            panic!("{name}: {e}")
        };
        assert!(file.ends_with("plant.v1.toml"), "{file}");
        assert!(
            report.contains(&format!(
                "error[alarms.v2 {token}] state/level: alarms.rules: "
            )),
            "{name}: {report}"
        );
        assert!(
            !d.join("out/zk2.rs").exists(),
            "{name}: nothing is generated"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    // `disagrees`: a resource that writes the key again, with another
    // list. The key is written once, on [defaults].
    let d = scratch("disagrees", |text| {
        format!(
            "{text}\n[resources.other]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n\
             annotations = {{ \"alarms.rules\" = [{{ name = \"cold\", summary = \"Cold\" }}] }}\n"
        )
    });
    let e = generate(&d).unwrap_err();
    let Error::Lint { report, .. } = &e else {
        panic!("{e}")
    };
    assert!(
        report.contains("error[alarms.v2 disagrees] state/") && report.contains("alarms.rules"),
        "{report}"
    );
    std::fs::remove_dir_all(d).unwrap();
}

/// Under `alarms.v1` the list is no table of the service's (§4,
/// `not_used`): the contract draws core W105, a warning, and builds.
#[test]
fn a_table_under_alarms_v1_is_a_warning() {
    let d = scratch("v1", |text| {
        text.replace(r#"uses  = ["alarms.v2"]"#, r#"uses  = ["alarms.v1"]"#)
    });
    let g = generate(&d).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(g.interfaces, ["plant.v1"]);
    assert!(
        g.warnings
            .iter()
            .any(|w| w.contains("W105") && w.contains("alarms.rules")),
        "{:?}",
        g.warnings
    );
    std::fs::remove_dir_all(d).unwrap();
}
