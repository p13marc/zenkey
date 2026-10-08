//! The walkthrough's and the tcgui pilot's contracts (`examples/zk2/`),
//! generated against their published history, with one name hint: the
//! tcgui error detail is the hand-written `crate::model::TcError`.

use std::path::PathBuf;

fn main() {
    let examples =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../examples/zk2");
    zenkey_build::Config::new()
        .contracts_dir(examples.join("walkthrough"))
        .contracts_dir(examples.join("tcgui"))
        .history_dir(examples.join(".history"))
        .json_type("json:TcError", "crate::model::TcError")
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
