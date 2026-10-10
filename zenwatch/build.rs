//! zenwatch's own contract (#614, FL3), compiled through zk2's
//! `zenkey-build`: `contracts/zenwatch.v1.toml` linted (every error fails
//! this build), checked against its append-only history
//! (`contracts/.history`: breaking fails, review warns), its bundle
//! embedded, and the typed `zenwatch_v1` module generated.
//!
//! **Rust-first.** The contract's JSON Schema types are the structs the
//! daemon already publishes (`crate::publish`), bound by name hints, so
//! nothing is generated for them; `schemas/zenwatch.json` is what they
//! derive, committed, and a test keeps the two in step
//! (`zenkey_build::check_schema`).

fn main() {
    zenkey_build::Config::new()
        .contracts_dir("contracts")
        .history_dir("contracts/.history")
        .json_type("json:FiringRule", "crate::publish::FiringRule")
        .json_type("json:ZenwatchDoctor", "crate::publish::ZenwatchDoctor")
        .json_type("json:ZenwatchCounters", "crate::publish::ZenwatchCounters")
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
