//! What a consumer's build makes of a governed interface's invalid
//! declaration (`redundancy.v1` §2.2, `redundancy.v2` §2.2; #987's N1): a
//! `cargo:warning`, never a failure. Its runtime refuses to start it, and
//! the core has no code for it. Under `redundancy.v2` an explicit stream is
//! one (`explicit_stream`); under `redundancy.v1` it is not, and an
//! election outside the vocabulary is one under either (`value`).

use std::path::{Path, PathBuf};

use zenkey_build::{Config, Generated};

fn scratch(name: &str, uses: &str, election: &str, explicit: bool) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zk2-redundancy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("ledger.v1.toml"),
        format!(
            r#"[interface]
name  = "ledger"
major = 1
minor = 0
uses  = ["{uses}"]

[defaults]
annotations = {{ "redundancy.election" = "{election}" }}

[resources.balance]
kind = "state"
type = {{ raw = "text/plain" }}

[resources.feed]
kind     = "stream"
explicit = {explicit}
type     = {{ raw = "text/plain" }}
"#
        ),
    )
    .unwrap();
    d
}

fn generate(d: &Path) -> Generated {
    Config::new()
        .contracts_dir(d)
        .out_file(d.join("out/zk2.rs"))
        .generate()
        .unwrap_or_else(|e| panic!("{e}"))
}

fn redundancy_warnings(g: &Generated) -> Vec<&String> {
    g.warnings
        .iter()
        .filter(|w| w.contains("warning[redundancy."))
        .collect()
}

#[test]
fn a_governed_interface_with_no_finding_builds_without_a_word() {
    let d = scratch("clean", "redundancy.v2", "claim", false);
    let g = generate(&d);
    assert_eq!(g.interfaces, ["ledger.v1"]);
    assert!(redundancy_warnings(&g).is_empty(), "{:?}", g.warnings);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_explicit_stream_under_redundancy_v2_warns_and_builds() {
    let d = scratch("explicit", "redundancy.v2", "claim", true);
    let g = generate(&d);
    assert_eq!(g.interfaces, ["ledger.v1"], "a warning fails nothing");
    let w = redundancy_warnings(&g);
    assert_eq!(w.len(), 1, "{:?}", g.warnings);
    assert!(
        w[0].ends_with(
            "ledger.v1.toml: warning[redundancy.v2 explicit_stream] @stream/feed: an explicit \
             stream's samples carry no envelope, so no run a follower can fence: a redundancy.v2 \
             interface declares none"
        ),
        "{}",
        w[0]
    );
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_explicit_stream_under_redundancy_v1_is_no_finding() {
    let d = scratch("v1-explicit", "redundancy.v1", "claim", true);
    let g = generate(&d);
    assert!(redundancy_warnings(&g).is_empty(), "{:?}", g.warnings);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn an_election_outside_the_vocabulary_warns_under_either_major() {
    for (major, uses) in [("v1", "redundancy.v1"), ("v2", "redundancy.v2")] {
        let d = scratch(&format!("value-{major}"), uses, "quorum", false);
        let g = generate(&d);
        let w = redundancy_warnings(&g);
        let want = [
            format!("warning[{uses} value] state/balance: "),
            format!("warning[{uses} value] stream/feed: "),
        ];
        assert_eq!(w.len(), 2, "{:?}", g.warnings);
        for want in &want {
            assert!(w.iter().any(|w| w.contains(want.as_str())), "{want}: {w:?}");
        }
        std::fs::remove_dir_all(d).unwrap();
    }
}
