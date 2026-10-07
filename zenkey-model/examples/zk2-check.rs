//! Validates contract files and prints each one's fingerprint.
//!
//! ```text
//! cargo run -p zenkey-model --example zk2-check -- examples/zk2/walkthrough/*.toml
//! ```
//!
//! Exit 0 when every file is valid, 1 when one is not.

use std::path::Path;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::load_path;

fn main() {
    let mut ok = true;
    for arg in std::env::args().skip(1) {
        let l = load_path(Path::new(&arg));
        print!("{}", l.report);
        match l.contract {
            Some(c) => println!("{arg}: {} {}", c.iface, Fingerprint::of(&c)),
            None => {
                println!("{arg}: invalid");
                ok = false;
            }
        }
    }
    std::process::exit(i32::from(!ok));
}
