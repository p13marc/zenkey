//! Every example contract under `examples/zk2/` validates with no finding at
//! all, fingerprints and bundles (#608), and the set's cross-contract checks
//! pass.

use std::path::{Path, PathBuf};

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, check_set, load_path};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
}

/// Every contract file: `*.toml`, except deployment files (`*.bindings.toml`).
fn contract_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(contract_files(&p));
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str())
            && n.ends_with(".toml")
            && !n.ends_with(".bindings.toml")
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn every_example_contract_validates_fingerprints_and_bundles() {
    let mut failures = Vec::new();
    let mut contracts: Vec<Contract> = Vec::new();
    for p in contract_files(&examples()) {
        let l = load_path(&p);
        let rel = p.strip_prefix(examples()).unwrap().display().to_string();
        if !l.report.0.is_empty() {
            eprintln!("== {rel}\n{}", l.report);
            failures.push(rel.clone());
        }
        match l.contract {
            None => {}
            Some(c) => {
                let b = Bundle::build(&c);
                let v = Bundle::verify_expecting(&b.to_bytes(), &Fingerprint::of(&c));
                assert!(v.is_ok(), "{rel}: {:?}", v.err());
                contracts.push(c);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "example contracts with findings: {failures:?}"
    );
    let refs: Vec<&Contract> = contracts.iter().collect();
    let set = check_set(&refs);
    assert!(!set.has_errors(), "{set}");
    assert!(
        contracts.len() >= 24,
        "only {} contracts found",
        contracts.len()
    );
}
