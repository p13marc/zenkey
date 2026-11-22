//! The walkthrough's and the tcgui pilot's contracts (`examples/zk2/`),
//! with `health.v1`, the standard contract of its profile
//! (`spec/profiles/health/`, #721), generated against their published
//! history, with one name hint: the tcgui error detail is the hand-written
//! `crate::model::TcError`. And `packed.v1` (`zenkey/tests/contracts/packed/`),
//! the packed vectors' types (#979 B2), `linkframes.v1`
//! (`zenkey/tests/contracts/frames/`), frame resources (#981 K3), and
//! `packedops.v1` (`zenkey/tests/contracts/packed-ops/`), packed operation
//! types (#985 C4c), and `rfcfg.v1`
//! (`spec/profiles/config/conformance/contracts/`), config.v2's derived
//! types (#985 G2), and desired.v2's scenarios' `tasking.v2` and
//! `worker.v2` (`zenkey/tests/contracts/`), whose report is the standard
//! schema's packed `Report` (#986 J2).
//!
//! One history root serves all three directories: `health.v1`'s revision
//! 1.0 was published in `examples/zk2/.history` before the contract moved,
//! and that root is append-only (its own root is `spec/profiles/.history`).
//! Its revision 1.1, which uses `freshness.v2` (#978's F4), is published in
//! both, so a build here does not warn its review again.

use std::path::{Path, PathBuf};

/// desired.v2's two scenario contracts, alone in a directory of their own:
/// they live among the runtime's forty test contracts, and a
/// `contracts_dir` takes every file of its directory. The copies keep
/// their paths from the root, so `worker.v2`'s path to the profile's
/// schema resolves as it is written.
fn desired_v2(root: &Path) -> PathBuf {
    let mirror = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("desired-v2");
    let contracts = "zenkey/tests/contracts";
    for file in [
        format!("{contracts}/tasking.v2.toml"),
        format!("{contracts}/worker.v2.toml"),
        format!("{contracts}/tasking.json"),
        "spec/profiles/desired/desired.v2.json".to_owned(),
    ] {
        let from = root.join(&file);
        println!("cargo:rerun-if-changed={}", from.display());
        let bytes = std::fs::read(&from).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
        let to = mirror.join(&file);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        if std::fs::read(&to).ok().as_deref() != Some(bytes.as_slice()) {
            std::fs::write(&to, bytes).unwrap();
        }
    }
    mirror.join(contracts)
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let examples = root.join("examples/zk2");
    zenkey_build::Config::new()
        .contracts_dir(examples.join("walkthrough"))
        .contracts_dir(root.join("spec/profiles/health"))
        .contracts_dir(examples.join("tcgui"))
        // core §7.6 (#979 B2): the packed vectors' types, generated.
        .contracts_dir(root.join("zenkey/tests/contracts/packed"))
        // core §7.7 (#981 K3): linkframes.v1's frame resources, a frame
        // writer each.
        .contracts_dir(root.join("zenkey/tests/contracts/frames"))
        // core §5.1, §7.6 (#985 C4c): packedops.v1's packed operation
        // types, served and called in the forms.
        .contracts_dir(root.join("zenkey/tests/contracts/packed-ops"))
        // config.v2 (#985 G2): the spec's rfcfg.v1, whose packed file holds
        // its groups' types and the types derived from them.
        .contracts_dir(root.join("spec/profiles/config/conformance/contracts"))
        // desired.v2 (#986 J2): the scenarios' tasking.v2 and worker.v2,
        // the target's report the standard schema's packed `Report`.
        .contracts_dir(desired_v2(&root))
        .history_dir(examples.join(".history"))
        .json_type("json:TcError", "crate::model::TcError")
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
