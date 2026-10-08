//! The walkthrough's interfaces, in contract-crate mode: every runtime item
//! behind the crate's `zenoh` feature.

use std::path::PathBuf;

fn main() {
    let examples =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../examples/zk2");
    zenkey_build::Config::new()
        .contracts_dir(examples.join("walkthrough"))
        .history_dir(examples.join(".history"))
        .contract_crate(true)
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
