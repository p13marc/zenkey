//! `zk2`: the contract CI binary (#618).
//!
//! ```text
//! zk2 contract lint <file.toml>...
//! zk2 contract fingerprint <file.toml>
//! zk2 contract bundle <file.toml> [--history <root>]
//! zk2 contract compat <old> <new>           (each a .toml or a .bundle.json)
//! zk2 contract compat --history <root> <new.toml>
//! zk2 contract check-history <root>
//! zk2 catalog new [--epoch <secs>] [<contract>...]
//! zk2 catalog add <catalog.json> <contract>...
//! zk2 catalog id <catalog.json>
//! zk2 catalog check <catalog.json> [--previous <catalog.json>] [<contract>...]
//! zk2 config derive <file.toml>
//! ```
//!
//! A `<contract>` is a `.toml` contract or a `.bundle.json` bundle, as a
//! history holds one. `catalog new` and `add` write the deployment catalog
//! (core §8.6) to stdout as its JCS bytes: `new` a first catalog, and `add`
//! the successor that appends what its input lacks, or that input unchanged
//! when it lacks nothing.
//!
//! `config derive` reads a `config.v2` contract's `config.groups` and its
//! packed files, and prints the types derived from them (config.v2 §3):
//! `Record`, a `Set<T>` per hot or reach group, `Control` and `Error`, as
//! a packed file's `types`, for the contract's packed file to hold. It reads
//! the contract unlinted, since it lints only once they are there.
//!
//! `contract lint` runs a profile's contract lint where the profile has
//! one the model implements: config.v2's (its §2.2), whose findings are
//! errors.
//!
//! Exit codes follow zenctl's contract: 0 asked and clean, 1 asked and a
//! finding (an invalid contract, a review or breaking change, a history
//! problem, a catalog refused, one that does not succeed its predecessor,
//! or a contract it does not hold), 2 no verdict (usage, an unreadable
//! input). `compat --history` of a revision the history holds reports its
//! review as accepted and exits 0 (core §9.7); breaking is still 1.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::catalog::{Catalog, MAX_EPOCH, Pushed};
use zenkey_model::compat::{Class, Revision, check_history, same_revision};
use zenkey_model::contract::{Contract, load_path};
use zenkey_model::grammar::IfaceId;
use zenkey_model::history;

const USAGE: &str = "usage:
  zk2 contract lint <file.toml>...
  zk2 contract fingerprint <file.toml>
  zk2 contract bundle <file.toml> [--history <root>]
  zk2 contract compat <old> <new>
  zk2 contract compat --history <root> <new.toml>
  zk2 contract check-history <root>
  zk2 catalog new [--epoch <secs>] [<contract>...]
  zk2 catalog add <catalog.json> <contract>...
  zk2 catalog id <catalog.json>
  zk2 catalog check <catalog.json> [--previous <catalog.json>] [<contract>...]
  zk2 config derive <file.toml>";

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
        ["catalog", "new", rest @ ..] => catalog_new(rest),
        ["catalog", "add", file, contracts @ ..] if !contracts.is_empty() => {
            catalog_add(file, contracts)
        }
        ["catalog", "id", file] => catalog_id(file),
        ["catalog", "check", file, rest @ ..] => catalog_check(file, rest),
        ["config", "derive", file] => config_derive(file),
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
            // config.v2 §2.2: its lint's findings are errors.
            Some(c) => match zenkey_model::config::v2::lint_contract(&c).as_slice() {
                [] => println!("{f}: {} {}", c.iface, Fingerprint::of(&c)),
                findings => {
                    eprintln!("{f}:");
                    for x in findings {
                        eprintln!("  error[config.v2 {}] {}", x.reason.token(), x.at);
                    }
                    bad = true;
                }
            },
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

/// Prints a verdict's findings, warnings and class.
fn print_verdict(class: Class, v: &zenkey_model::compat::Verdict) {
    for f in &v.findings {
        println!("{} [{}] {}: {}", f.class.as_str(), f.rule, f.at, f.detail);
    }
    for w in &v.warnings {
        println!("warning [{}] {}: {}", w.rule, w.at, w.detail);
    }
    println!("{}", class.as_str());
}

fn report(class: Class, v: &zenkey_model::compat::Verdict) -> ExitCode {
    print_verdict(class, v);
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
    let published = revs.iter().any(|r| same_revision(r, &new));
    if published {
        println!("same revision as a published one: the retention rule keeps its bundle");
    }
    println!(
        "against {} published revision(s) of {}",
        revs.len(),
        c.iface
    );
    let v = check_history(&revs, &new);
    // Core §9.7: a revision in the history root passed its review when it
    // was published. Its review is reported as accepted, never refused; a
    // breaking change still is.
    if published && v.class() == Class::Review {
        print_verdict(v.class(), &v);
        println!("accepted: the revision is published (core §9.7)");
        return ExitCode::SUCCESS;
    }
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

/// A catalog file, read; its refusal printed (exit 1), or an unreadable file
/// (exit 2).
fn read_catalog(file: &str) -> Result<Catalog, ExitCode> {
    let bytes = std::fs::read(file).map_err(|e| no_verdict(&format!("{file}: {e}")))?;
    Catalog::parse_bytes(&bytes).map_err(|e| {
        println!("{file}: refused [{}]: {e}", e.token());
        ExitCode::from(1)
    })
}

/// The interface and fingerprint of a contract or a bundle file.
fn revision_of(file: &str) -> Result<(IfaceId, Fingerprint), ExitCode> {
    if file.ends_with(".bundle.json") {
        let bytes = std::fs::read(file).map_err(|e| no_verdict(&format!("{file}: {e}")))?;
        let b = Bundle::verify(&bytes).map_err(|e| {
            eprintln!("{file}: {e}");
            ExitCode::from(1)
        })?;
        let iface = b
            .contract
            .get("interface")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<IfaceId>().ok())
            .ok_or_else(|| {
                eprintln!("{file}: the bundle names no interface id");
                ExitCode::from(1)
            })?;
        Ok((iface, b.fingerprint()))
    } else {
        if !Path::new(file).is_file() {
            return Err(no_verdict(&format!("{file}: no such file")));
        }
        load(file)
            .map(|c| (c.iface.clone(), Fingerprint::of(&c)))
            .ok_or(ExitCode::from(1))
    }
}

/// Appends each revision, saying on stderr where each one went.
fn push_all(c: &mut Catalog, files: &[&str]) -> Result<(), ExitCode> {
    for f in files {
        let (iface, fp) = revision_of(f)?;
        match c.push(iface.clone(), fp.clone()) {
            Ok(Pushed::Added(i)) => eprintln!("index {i}: {iface} {fp}"),
            Ok(Pushed::Held(i)) => eprintln!("already at index {i}: {iface} {fp}"),
            Err(e) => {
                println!("{f}: refused [{}]: {e}", e.token());
                return Err(ExitCode::from(1));
            }
        }
    }
    Ok(())
}

/// The catalog's JCS bytes on stdout, and its id on stderr.
fn write_catalog(c: &Catalog) -> ExitCode {
    // What a reader refuses is never written: the bytes are read back first.
    match Catalog::parse_bytes(&c.to_jcs()) {
        Ok(read) if &read == c => {}
        other => {
            eprintln!("refused as written: {other:?}");
            return ExitCode::from(1);
        }
    }
    println!("{}", String::from_utf8_lossy(&c.to_jcs()));
    eprintln!(
        "catalog {}: epoch {}, {} entries",
        c.id(),
        c.epoch(),
        c.entries().len()
    );
    ExitCode::SUCCESS
}

fn catalog_new(args: &[&str]) -> ExitCode {
    let (epoch, files) = match args {
        ["--epoch", e, rest @ ..] => match epoch_arg(e) {
            Some(e) => (e, rest),
            None => {
                return no_verdict(&format!(
                    "--epoch {e}: not whole seconds from 0 to 2^53 - 1\n{USAGE}"
                ));
            }
        },
        rest => (zenkey_model::catalog::EPOCH_WITHOUT_CATALOG, rest),
    };
    if files.iter().any(|f| f.starts_with("--")) {
        return no_verdict(&format!("unknown arguments\n{USAGE}"));
    }
    let mut c = Catalog::new(epoch).expect("the epoch is in range");
    if let Err(code) = push_all(&mut c, files) {
        return code;
    }
    write_catalog(&c)
}

/// An epoch argument: digits alone, as the catalog writes them, so `+1`,
/// `01` and `1e9` are refused, from 0 to 2^53 − 1.
fn epoch_arg(e: &str) -> Option<i64> {
    let digits = !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit());
    let canonical = e == "0" || !e.starts_with('0');
    let e: i64 = e.parse().ok().filter(|_| digits && canonical)?;
    (0..=MAX_EPOCH).contains(&e).then_some(e)
}

fn catalog_add(file: &str, files: &[&str]) -> ExitCode {
    let old = match read_catalog(file) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let mut new = old.successor();
    if let Err(code) = push_all(&mut new, files) {
        return code;
    }
    if new.entries().len() == old.entries().len() {
        eprintln!("nothing to add: {file} is unchanged");
        return write_catalog(&old);
    }
    eprintln!("succeeds {}", old.id());
    write_catalog(&new)
}

fn catalog_id(file: &str) -> ExitCode {
    match read_catalog(file) {
        Ok(c) => {
            println!("{}", c.id());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn catalog_check(file: &str, args: &[&str]) -> ExitCode {
    let (previous, files) = match args {
        ["--previous", p, rest @ ..] => (Some(*p), rest),
        rest => (None, rest),
    };
    if files.iter().any(|f| f.starts_with("--")) {
        return no_verdict(&format!("unknown arguments\n{USAGE}"));
    }
    let c = match read_catalog(file) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let previous = match previous.map(read_catalog).transpose() {
        Ok(p) => p,
        Err(code) => return code,
    };
    println!(
        "{file}: catalog {}, epoch {}, {} entries",
        c.id(),
        c.epoch(),
        c.entries().len()
    );
    for (i, e) in c.entries().iter().enumerate() {
        println!("  {i} {} {}", e.iface, e.contract);
    }
    let mut finding = false;
    if let Some(p) = &previous {
        match c.succeeds(p) {
            Ok(()) => println!("succeeds {}", p.id()),
            Err(s) => {
                println!("does not succeed {} [{}]: {s}", p.id(), s.token());
                finding = true;
            }
        }
    }
    for f in files {
        let (iface, fp) = match revision_of(f) {
            Ok(r) => r,
            Err(code) => return code,
        };
        match c.index_of(&iface, &fp) {
            Some(i) => println!("{f}: {iface} {fp} at index {i}"),
            None => {
                println!("{f}: {iface} {fp} is not held");
                finding = true;
            }
        }
    }
    if finding {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// `config derive`: the types config.v2 derives from a contract's
/// declaration (§3). 1 when the contract does not parse, declares no
/// `config.groups` on `config`, or declares them invalid.
fn config_derive(file: &str) -> ExitCode {
    use zenkey_model::authoring::ContractFile;
    use zenkey_model::config::v2::{CONFIG, GROUPS, Interface};
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => return no_verdict(&format!("{file}: {e}")),
    };
    let c: ContractFile = match toml::from_str(&text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{file}: {}", e.to_string().trim());
            return ExitCode::from(1);
        }
    };
    let Some(annotation) = c
        .resources
        .get(CONFIG)
        .and_then(|r| r.annotations.get(GROUPS))
    else {
        eprintln!("{file}: no resource `{CONFIG}` declares `{GROUPS}`");
        return ExitCode::from(1);
    };
    let dir = Path::new(file).parent().unwrap_or(Path::new("."));
    let mut docs = Vec::new();
    for f in &c.schemas.packed {
        let path = dir.join(f);
        let doc: serde_json::Value = match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
        {
            Ok(d) => d,
            Err(e) => return no_verdict(&format!("{}: {e}", path.display())),
        };
        let base = f.rsplit('/').next().unwrap_or(f);
        docs.push((base.strip_suffix(".json").unwrap_or(base).to_owned(), doc));
    }
    let files: Vec<(&str, &serde_json::Value)> =
        docs.iter().map(|(s, d)| (s.as_str(), d)).collect();
    match Interface::read(annotation, &files) {
        Ok(i) => {
            print!("{}", i.derived.to_text());
            ExitCode::SUCCESS
        }
        Err(reasons) => {
            for r in reasons {
                match r.entry {
                    Some(e) => eprintln!("{file}: {GROUPS}[{e}]: {}", r.reason.token()),
                    None => eprintln!("{file}: {GROUPS}: {}", r.reason.token()),
                }
            }
            ExitCode::from(1)
        }
    }
}
