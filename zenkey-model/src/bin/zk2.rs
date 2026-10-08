//! `zk2`: the contract CI binary (#618).
//!
//! ```text
//! zk2 contract lint <file.toml>...
//! zk2 contract fingerprint <file.toml>
//! zk2 contract bundle <file.toml> [--history <root>]
//! zk2 contract compat <old> <new>           (each a .toml or a .bundle.json)
//! zk2 contract compat --history <root> <new.toml>
//! zk2 contract check-history <root>
//! ```
//!
//! Exit codes follow zenctl's contract: 0 asked and clean, 1 asked and a
//! finding (an invalid contract, a review or breaking change, a history
//! problem), 2 no verdict (usage, an unreadable input).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::compat::{Class, Revision, check_history, same_revision};
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::history;

const USAGE: &str = "usage:
  zk2 contract lint <file.toml>...
  zk2 contract fingerprint <file.toml>
  zk2 contract bundle <file.toml> [--history <root>]
  zk2 contract compat <old> <new>
  zk2 contract compat --history <root> <new.toml>
  zk2 contract check-history <root>";

fn no_verdict(msg: &str) -> ExitCode {
    eprintln!("zk2: {msg}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["contract", "lint", files @ ..] if !files.is_empty() => lint(files),
        ["contract", "fingerprint", file] => fingerprint(file),
        ["contract", "bundle", file] => bundle(file, None),
        ["contract", "bundle", file, "--history", root] => bundle(file, Some(root)),
        ["contract", "compat", "--history", root, file] => compat_history(root, file),
        ["contract", "compat", old, new] => compat(old, new),
        ["contract", "check-history", root] => check(root),
        ["--help" | "-h" | "help"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => no_verdict(&format!("unknown arguments\n{USAGE}")),
    }
}

/// Loads a contract, printing its findings; `None` when it is invalid.
fn load(file: &str) -> Option<Contract> {
    let l = load_path(Path::new(file));
    if !l.report.0.is_empty() {
        eprint!("{file}:\n{}", l.report);
    }
    l.contract
}

fn lint(files: &[&str]) -> ExitCode {
    let mut bad = false;
    for f in files {
        match load(f) {
            Some(c) => println!("{f}: {} {}", c.iface, Fingerprint::of(&c)),
            None => bad = true,
        }
    }
    if bad {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn fingerprint(file: &str) -> ExitCode {
    match load(file) {
        Some(c) => {
            println!("{}", Fingerprint::of(&c));
            ExitCode::SUCCESS
        }
        None => ExitCode::from(1),
    }
}

fn bundle(file: &str, root: Option<&str>) -> ExitCode {
    let Some(c) = load(file) else {
        return ExitCode::from(1);
    };
    match root {
        None => {
            let b = Bundle::build(&c);
            println!("{}", String::from_utf8_lossy(&b.to_bytes()));
            ExitCode::SUCCESS
        }
        Some(root) => match history::append(Path::new(root), &c) {
            Ok((path, written)) => {
                let what = if written {
                    "published"
                } else {
                    "already published"
                };
                println!("{what}: {}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => no_verdict(&format!("{root}: {e}")),
        },
    }
}

/// A revision from a contract file or a bundle file.
fn revision(file: &str) -> Result<Revision, ExitCode> {
    if file.ends_with(".bundle.json") {
        let bytes = std::fs::read(file).map_err(|e| no_verdict(&format!("{file}: {e}")))?;
        let b = Bundle::verify(&bytes).map_err(|e| {
            eprintln!("{file}: {e}");
            ExitCode::from(1)
        })?;
        Ok(Revision::of_bundle(&b))
    } else {
        load(file)
            .map(|c| Revision::of(&c))
            .ok_or(ExitCode::from(1))
    }
}

fn report(class: Class, v: &zenkey_model::compat::Verdict) -> ExitCode {
    for f in &v.findings {
        println!("{} [{}] {}: {}", f.class.as_str(), f.rule, f.at, f.detail);
    }
    for w in &v.warnings {
        println!("warning [{}] {}: {}", w.rule, w.at, w.detail);
    }
    println!("{}", class.as_str());
    if class == Class::Compatible {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn compat(old: &str, new: &str) -> ExitCode {
    let (o, n) = match (revision(old), revision(new)) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    let v = zenkey_model::compat::compare(&o, &n);
    report(v.class(), &v)
}

fn compat_history(root: &str, file: &str) -> ExitCode {
    let Some(c) = load(file) else {
        return ExitCode::from(1);
    };
    let dir = PathBuf::from(root).join(c.iface.to_string());
    let mut revs = Vec::new();
    let mut entries: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(d) => d.flatten().map(|e| e.path()).collect(),
        Err(_) => Vec::new(),
    };
    entries.sort();
    for p in entries {
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) => return no_verdict(&format!("{}: {e}", p.display())),
        };
        match Bundle::verify(&bytes) {
            Ok(b) => revs.push(Revision::of_bundle(&b)),
            Err(e) => {
                eprintln!("{}: {e}", p.display());
                return ExitCode::from(1);
            }
        }
    }
    let new = Revision::of(&c);
    if revs.iter().any(|r| same_revision(r, &new)) {
        println!("same revision as a published one: the retention rule keeps its bundle");
    }
    println!(
        "against {} published revision(s) of {}",
        revs.len(),
        c.iface
    );
    let v = check_history(&revs, &new);
    report(v.class(), &v)
}

fn check(root: &str) -> ExitCode {
    let problems = history::check_tagged(Path::new(root));
    for p in &problems {
        println!("{} [{}]: {}", p.at, p.tag, p.message);
    }
    if problems.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
