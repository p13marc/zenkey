//! The walkthrough's interfaces, with `health.v1`, the standard contract of
//! its profile (`spec/profiles/health/`, #721), in contract-crate mode:
//! every runtime item behind the crate's `zenoh` feature.
//!
//! `health.v1`'s revision 1.0 was published in `examples/zk2/.history`
//! before the contract moved, so that one root serves both directories.

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let examples = root.join("examples/zk2");
    zenkey_build::Config::new()
        .contracts_dir(examples.join("walkthrough"))
        .contracts_dir(root.join("spec/profiles/health"))
        .history_dir(examples.join(".history"))
        .contract_crate(true)
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
