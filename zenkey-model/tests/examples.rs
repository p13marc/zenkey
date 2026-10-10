//! Every example contract under `examples/zk2/` validates with no finding at
//! all, fingerprints and bundles (#608), and the set's cross-contract checks
//! pass, with the profiles' standard contracts in the set (`health.v1`,
//! #721), which `tests/profiles.rs` checks on their own.

use std::path::{Path, PathBuf};

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Contract, check_set, load_path};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/zk2")
}

/// Every contract file: `*.toml`, except deployment files (`*.bindings.toml`,
/// and the access-control enrollments, `*.enrollment.toml`, #612 FJ7).
fn contract_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(contract_files(&p));
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str())
            && n.ends_with(".toml")
            && !n.ends_with(".bindings.toml")
            && !n.ends_with(".enrollment.toml")
        {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// The profiles' standard contracts, `spec/profiles/<name>/<name>.v<N>.toml`
/// (`spec/profiles/README.md`): an example deployment implements them beside
/// its own contracts (tcgui and ZenSight implement `health.v1`).
fn profile_contracts() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/profiles");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&root).unwrap().flatten() {
        let dir = e.path();
        let Some(name) = dir.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if !dir.is_dir() || name.starts_with('.') {
            continue;
        }
        for f in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = f.path();
            if p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&format!("{name}.v")) && n.ends_with(".toml"))
            {
                out.push(p);
            }
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
    // 23 since `health.v1` moved under `spec/profiles/` (#721).
    assert!(
        contracts.len() >= 23,
        "only {} contracts found",
        contracts.len()
    );
    // The set check sees the profiles' standard contracts too, so no example
    // declares one of their interfaces again (E036).
    let standard: Vec<Contract> = profile_contracts()
        .iter()
        .map(|p| load_path(p).contract.expect("tests/profiles.rs: it loads"))
        .collect();
    assert!(!standard.is_empty(), "health.v1's standard contract");
    let refs: Vec<&Contract> = contracts.iter().chain(&standard).collect();
    let set = check_set(&refs);
    assert!(!set.has_errors(), "{set}");
}

/// `examples/zk2/.history` (#618): every bundle verifies, every example's
/// current revision is published there, and it is compatible with every
/// earlier revision of its interface. After an intended change, publish the
/// new revision with `zk2 contract bundle <file> --history examples/zk2/.history`.
#[test]
fn every_example_is_published_and_compatible_with_its_history() {
    use zenkey_model::compat::{Class, Revision, check_history, same_revision};
    let root = examples().join(".history");
    let problems = zenkey_model::history::check_tagged(&root);
    assert!(problems.is_empty(), "{problems:#?}");
    for p in contract_files(&examples()) {
        let rel = p.strip_prefix(examples()).unwrap().display().to_string();
        let c = load_path(&p).contract.expect("validated above");
        let dir = root.join(c.iface.to_string());
        let mut revs = Vec::new();
        for e in std::fs::read_dir(&dir)
            .unwrap_or_else(|_| panic!("{rel}: no history at {}", dir.display()))
        {
            let bytes = std::fs::read(e.unwrap().path()).unwrap();
            revs.push(Revision::of_bundle(&Bundle::verify(&bytes).unwrap()));
        }
        let new = Revision::of(&c);
        assert!(
            revs.iter().any(|r| same_revision(r, &new)),
            "{rel}: the current revision is not published; run zk2 contract bundle {rel} --history examples/zk2/.history"
        );
        let v = check_history(&revs, &new);
        assert_eq!(v.class(), Class::Compatible, "{rel}: {:#?}", v.findings);
    }
}
