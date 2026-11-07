//! What crates.io receives (#929): a published crate is its own directory
//! and nothing beside it, so a file the library compiles in from outside
//! that directory builds in the workspace and fails for everyone who
//! depends on the release. The four lib crates are checked here, since
//! `cargo publish --dry-run` is not part of the gate.

use std::path::{Component, Path, PathBuf};

/// The crates `publish-crates.yml` publishes, in its order.
const PUBLISHED: [&str; 4] = ["zenkey-model", "zenkey", "zenkey-build", "zenkey-fleet"];

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `path` with its `.` and `..` folded away, never following a link: a
/// link inside the crate is packaged as the file it points at.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// Each string literal an `include_bytes!`, `include_str!` or `include!`
/// names, the literal on the macro's line or the next.
fn included(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for mac in ["include_bytes!(", "include_str!(", "include!("] {
        let mut rest = text;
        while let Some(at) = rest.find(mac) {
            rest = &rest[at + mac.len()..];
            let arg = rest.trim_start();
            if let Some(lit) = arg.strip_prefix('"')
                && let Some(end) = lit.find('"')
            {
                found.push(lit[..end].to_string());
            }
        }
    }
    found
}

#[test]
fn a_published_crate_compiles_in_nothing_from_outside_its_directory() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut outside = Vec::new();
    for krate in PUBLISHED {
        let root = lexical(&workspace.join(krate));
        let mut files = Vec::new();
        sources(&root.join("src"), &mut files);
        assert!(!files.is_empty(), "{krate} has sources");
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for lit in included(&text) {
                let target = lexical(&file.parent().unwrap().join(&lit));
                if !target.starts_with(&root) {
                    outside.push(format!("{}: {lit}", file.display()));
                }
            }
        }
    }
    assert!(
        outside.is_empty(),
        "included from outside the crate, so missing from its package:\n{}",
        outside.join("\n")
    );
}
