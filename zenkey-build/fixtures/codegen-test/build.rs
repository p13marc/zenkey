//! The walkthrough's and the tcgui pilot's contracts (`examples/zk2/`),
//! with `health.v1`, the standard contract of its profile
//! (`spec/profiles/health/`, #721), generated against their published
//! history, with one name hint: the tcgui error detail is the hand-written
//! `crate::model::TcError`.
//!
//! One history root serves all three directories: `health.v1`'s revision
//! 1.0 was published in `examples/zk2/.history` before the contract moved,
//! and that root is append-only (its own root is `spec/profiles/.history`).

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let examples = root.join("examples/zk2");
    zenkey_build::Config::new()
        .contracts_dir(examples.join("walkthrough"))
        .contracts_dir(root.join("spec/profiles/health"))
        .contracts_dir(examples.join("tcgui"))
        .history_dir(examples.join(".history"))
        .json_type("json:TcError", "crate::model::TcError")
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
