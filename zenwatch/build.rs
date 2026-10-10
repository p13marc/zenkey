//! The daemon's own v1 registry (RFC 08 §5), compiled through v1's
//! `zenkey-build` — the write side, until FL3 (#614) replaces it with a zk2
//! contract.
//!
//! The generated code spells v1's runtime `::zenkey::`, and on `main` that
//! name is not v1's in this package: since FL2 the v1 runtime is the
//! renamed dependency `zenkey_v1`, so no read-side line can reach it. The
//! one substitution below points the generated paths at it; FL3 deletes
//! this file whole.

use std::path::PathBuf;

fn main() {
    zenkey_build_v1::Config::new()
        .registry_dir("registry")
        .generate()
        .unwrap();
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"))
        .join("zenkey_registry.rs");
    let generated = std::fs::read_to_string(&out).expect("the generated registry");
    std::fs::write(&out, generated.replace("::zenkey::", "::zenkey_v1::"))
        .expect("the generated registry, re-pathed");
}
