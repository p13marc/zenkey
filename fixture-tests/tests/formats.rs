//! The corpus in both spellings (RFC 08 §5.1, v1.44; #374).
//!
//! `registry-kdl/` is the KDL mirror of `registry/`: every file respelled by
//! `write_kdl(parse_raw(toml))` — at the level of the document tree, so a
//! column this build never reads (logs' `reason`) crosses too — and the
//! three `.lock` ledgers copied unchanged, because they stay line files
//! beside a `.kdl` file as beside a `.toml` one.
//!
//! What this pins is §5.1's central sentence, "a KDL registry file means
//! exactly the TOML document the mapping produces from it", three ways:
//! every twin reads to the same document and the same slice; the mirror is
//! byte-for-byte what the writer produces today (regenerate with
//! `REGENERATE_KDL_MIRROR=1`); and the whole mirror **generates the same
//! code** as the TOML dir, byte for byte, but for the two lines per
//! producer a spelling is allowed to change — the `include_str!` path and
//! `REGISTRY_ENCODING`. A reader that let a spelling mean something else
//! would show up in that diff.

use std::path::{Path, PathBuf};

use zenkey::SliceFormat;
use zenkey::registry_doc::{parse_raw, write_kdl};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn toml_dir() -> PathBuf {
    crate_dir().join("registry")
}

fn kdl_dir() -> PathBuf {
    crate_dir().join("registry-kdl")
}

/// Every file of a dir with the given extension, sorted.
fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    out.sort();
    out
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn stem(p: &Path) -> String {
    p.file_stem().unwrap().to_string_lossy().to_string()
}

/// The KDL twin of a TOML registry file, as the mirror holds it.
fn twin(toml: &str) -> String {
    let raw = parse_raw(toml, SliceFormat::Toml).expect("the corpus parses");
    let body = write_kdl(&raw).expect("every corpus file has a KDL spelling");
    format!(
        "// Generated from ../registry/ by fixture-tests/tests/formats.rs — the KDL\n\
         // mirror of the corpus (RFC 08 §5.1). Regenerate, never edit:\n\
         //   REGENERATE_KDL_MIRROR=1 cargo test -p zenkey-fixture-tests --test formats\n\n\
         {body}"
    )
}

/// The mirror is exactly what the writer produces today, and its ledgers
/// are the TOML dir's, byte for byte. `REGENERATE_KDL_MIRROR=1` rewrites it
/// instead of comparing — the one sanctioned way to move it.
#[test]
fn the_kdl_mirror_is_the_writers_output_byte_for_byte() {
    let regenerate = std::env::var_os("REGENERATE_KDL_MIRROR").is_some_and(|v| v == "1");
    let (from, to) = (toml_dir(), kdl_dir());
    if regenerate {
        // Written in place, never removed and recreated: the other tests in
        // this binary read the mirror concurrently.
        std::fs::create_dir_all(&to).unwrap();
    }
    let mut expected: Vec<String> = Vec::new();
    for toml in files(&from, "toml") {
        let name = format!("{}.kdl", stem(&toml));
        let want = twin(&read(&toml));
        let path = to.join(&name);
        if regenerate {
            std::fs::write(&path, &want).unwrap();
        } else {
            assert!(
                read(&path) == want,
                "{} is not the writer's output for {} — regenerate with \
                 REGENERATE_KDL_MIRROR=1, never edit",
                path.display(),
                toml.display()
            );
        }
        expected.push(name);
    }
    for lock in files(&from, "lock") {
        let name = lock.file_name().unwrap().to_string_lossy().to_string();
        let path = to.join(&name);
        if regenerate {
            std::fs::copy(&lock, &path).unwrap();
        } else {
            assert_eq!(
                read(&path),
                read(&lock),
                "{name}: the ledgers stay line files, unchanged"
            );
        }
        expected.push(name);
    }
    // Nothing else lives there: a stray file would be a stem the TOML dir
    // does not have.
    let mut present: Vec<String> = std::fs::read_dir(&to)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    present.sort();
    expected.sort();
    if regenerate {
        for stray in present.iter().filter(|p| !expected.contains(p)) {
            std::fs::remove_file(to.join(stray)).unwrap();
        }
        return;
    }
    assert_eq!(present, expected);
}

/// Every twin reads to the same document — the whole tree, unknown columns
/// included — and every producer's to the same slice (§5.1: "one document,
/// two spellings").
#[test]
fn every_twin_reads_to_the_same_document_and_slice() {
    for toml in files(&toml_dir(), "toml") {
        let kdl = kdl_dir().join(format!("{}.kdl", stem(&toml)));
        let (t, k) = (read(&toml), read(&kdl));
        assert_eq!(
            parse_raw(&t, SliceFormat::Toml).unwrap(),
            parse_raw(&k, SliceFormat::Kdl).unwrap_or_else(|e| panic!("{}: {e}", kdl.display())),
            "{}: the trees differ",
            kdl.display()
        );
        if stem(&toml) == "types" {
            continue; // the type table is not a slice
        }
        let slice = zenkey::parse_slice_as(&t, SliceFormat::Toml).unwrap();
        assert_eq!(
            zenkey::parse_slice_as(&k, SliceFormat::Kdl).unwrap(),
            slice,
            "{}",
            kdl.display()
        );
        // The sniff reads each for what it is, as an undeclared reply is.
        assert_eq!(zenkey::parse_slice(&k).unwrap(), slice);
        // And `to_kdl` — the export path — round-trips every carried column.
        let exported = zenkey::slice_to_kdl(&slice);
        assert_eq!(
            zenkey::parse_slice_as(&exported, SliceFormat::Kdl).unwrap(),
            slice,
            "{}: to_kdl does not round-trip",
            toml.display()
        );
    }
}

/// The shapes outside the corpus: the dogfooded notifier's own registry and
/// the inferred draft zenctl's corpus pins. No committed mirror — just the
/// same reading, twice.
#[test]
fn the_other_registries_in_the_workspace_read_the_same_in_kdl() {
    let root = crate_dir().join("..");
    for dir in [
        "zenwatch/registry",
        "zenctl/tests/cmd/registry-infer.out/draft",
    ] {
        for toml in files(&root.join(dir), "toml") {
            let t = read(&toml);
            let raw = parse_raw(&t, SliceFormat::Toml).unwrap();
            let k = twin(&t);
            assert_eq!(
                parse_raw(&k, SliceFormat::Kdl).unwrap(),
                raw,
                "{}",
                toml.display()
            );
            if stem(&toml) != "types" {
                assert_eq!(
                    zenkey::parse_slice_as(&k, SliceFormat::Kdl).unwrap(),
                    zenkey::parse_slice_as(&t, SliceFormat::Toml).unwrap(),
                    "{}",
                    toml.display()
                );
            }
        }
    }
}

/// The KDL mirror generates the TOML dir's code, byte for byte, but for the
/// two lines per producer a spelling changes: the `include_str!` path, and
/// `REGISTRY_ENCODING`. Lints, lock and ledgers pass unchanged on it — the
/// copied `registry.lock` matching is the proof that the pins are
/// spelling-neutral.
#[test]
fn the_mirror_generates_byte_identical_code_but_for_two_lines() {
    let generate = |dir: PathBuf| {
        zenkey_build::Config::new()
            .registry_dir(dir)
            .no_rerun_if_changed()
            .generate_string()
            .unwrap_or_else(|e| panic!("{e}"))
    };
    let (toml, kdl) = (generate(toml_dir()), generate(kdl_dir()));
    let (t, k): (Vec<&str>, Vec<&str>) = (toml.lines().collect(), kdl.lines().collect());
    assert_eq!(t.len(), k.len(), "the two generate different line counts");

    let producers = files(&toml_dir(), "toml")
        .iter()
        .filter(|p| stem(p) != "types")
        .count();
    let mut differing = Vec::new();
    for (i, (a, b)) in t.iter().zip(&k).enumerate() {
        if a == b {
            continue;
        }
        let allowed = (a.contains("include_str!(")
            && a.contains(".toml\")")
            && b.contains("include_str!(")
            && b.contains(".kdl\")")
            && a.replace("/registry/", "/registry-kdl/")
                .replace(".toml\")", ".kdl\")")
                == *b)
            || (*a == "    pub const REGISTRY_ENCODING: &str = \"application/toml\";"
                && *b == "    pub const REGISTRY_ENCODING: &str = \"application/kdl\";");
        assert!(
            allowed,
            "line {}: a spelling changed the code:\n  {a}\n  {b}",
            i + 1
        );
        differing.push(i);
    }
    assert_eq!(
        differing.len(),
        2 * producers,
        "exactly the include_str! and REGISTRY_ENCODING line of each of {producers} producers"
    );
}
