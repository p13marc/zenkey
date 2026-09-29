//! The corpus in both spellings (RFC 08 §5.1, v1.44; #374).
//!
//! `registry-kdl/` is the KDL mirror of `registry/`, and it is exactly what
//! `zenctl registry migrate --to kdl --out` writes from it — the migrator
//! (`zenkey_build::migrate`) is called here as a library: every file
//! respelled at the level of the document tree, so a column this build
//! never reads (logs' `reason`) crosses too, its comments carried onto the
//! nodes they stood before, and the three `.lock` ledgers copied unchanged,
//! because they stay line files beside a `.kdl` file as beside a `.toml`
//! one.
//!
//! What this pins is §5.1's central sentence, "a KDL registry file means
//! exactly the TOML document the mapping produces from it", three ways:
//! every twin reads to the same document and the same slice; the mirror is
//! byte-for-byte what the migrator produces today (regenerate with
//! `REGENERATE_KDL_MIRROR=1`, never by hand); and the whole mirror
//! **generates the same code** as the TOML dir, byte for byte, but for the
//! two lines per producer a spelling is allowed to change — the
//! `include_str!` path and `REGISTRY_ENCODING`. A reader that let a spelling
//! mean something else would show up in that diff.

use std::path::{Path, PathBuf};

use zenkey::SliceFormat;
use zenkey::registry_doc::parse_raw;

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

/// Every file name of a dir, sorted.
fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

/// The mirror is exactly what the migrator stages from `registry/` today —
/// every `.kdl` file and every ledger, byte for byte, and nothing else.
/// `REGENERATE_KDL_MIRROR=1` copies the staged directory over the mirror
/// instead of comparing — the one sanctioned way to move it.
#[test]
fn the_kdl_mirror_is_the_migrators_output_byte_for_byte() {
    let regenerate = std::env::var_os("REGENERATE_KDL_MIRROR").is_some_and(|v| v == "1");
    let staged = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("registry-kdl-staged");
    let _ = std::fs::remove_dir_all(&staged);
    std::fs::create_dir_all(&staged).unwrap();
    let migration = zenkey_build::migrate::stage_kdl(&toml_dir(), &staged)
        .unwrap_or_else(|e| panic!("the corpus migrates: {e}"));
    assert_eq!(
        migration.files.len(),
        files(&toml_dir(), "toml").len(),
        "every TOML file respelled"
    );
    assert!(
        migration.files.iter().any(|(_, _, c)| c.comments > 0),
        "the corpus' comments cross"
    );
    let to = kdl_dir();
    let want = names(&staged);
    if regenerate {
        // Written in place, never removed and recreated: the other tests in
        // this binary read the mirror concurrently.
        std::fs::create_dir_all(&to).unwrap();
        for name in &want {
            std::fs::copy(staged.join(name), to.join(name)).unwrap();
        }
        for stray in names(&to).iter().filter(|p| !want.contains(p)) {
            std::fs::remove_file(to.join(stray)).unwrap();
        }
    } else {
        for name in &want {
            assert!(
                read(&to.join(name)) == read(&staged.join(name)),
                "registry-kdl/{name} is not the migrator's output — regenerate with \
                 REGENERATE_KDL_MIRROR=1, never edit"
            );
        }
        // Nothing else lives there: a stray file would be a stem the TOML
        // dir does not have.
        assert_eq!(names(&to), want);
    }
    // The ledgers are the TOML dir's, byte for byte — the lock's pins name
    // entries, not spellings.
    for lock in files(&toml_dir(), "lock") {
        let name = lock.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(read(&to.join(&name)), read(&lock), "{name}: unchanged");
    }
    let _ = std::fs::remove_dir_all(&staged);
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
/// the inferred draft zenctl's corpus pins. No committed mirror — each TOML
/// file migrates to a KDL file reading the same, and each KDL file (the
/// notifier's, migrated for real) reads as a slice.
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
            let k = zenkey_build::migrate::toml_to_kdl(&t)
                .unwrap_or_else(|e| panic!("{}: {e}", toml.display()))
                .kdl;
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
        for kdl in files(&root.join(dir), "kdl") {
            let k = read(&kdl);
            parse_raw(&k, SliceFormat::Kdl).unwrap_or_else(|e| panic!("{}: {e}", kdl.display()));
            if stem(&kdl) != "types" {
                zenkey::parse_slice_as(&k, SliceFormat::Kdl)
                    .unwrap_or_else(|e| panic!("{}: {e}", kdl.display()));
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
