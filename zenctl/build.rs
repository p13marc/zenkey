//! Names the build: `zenctl --version` carries `git describe` (#513).
//!
//! Every production `zenctl` is a local build — a `cargo install --git …
//! --tag X.Y.Z`, a build of the release tarball, or the release lane's own —
//! so the crate version alone cannot say which build a bug report came from,
//! and the commit can. This embeds a description as `ZENCTL_GIT_DESCRIBE`,
//! from the first source that can establish one:
//!
//! 1. `git describe --tags --always --dirty`, in a repository that tracks
//!    this crate (a tarball unpacked inside somebody else's checkout would
//!    otherwise describe *their* commit);
//! 2. `describe.txt`, which `git archive` fills in through `export-subst`
//!    (`.gitattributes`) — the release tarball has no `.git`, and this is how
//!    it still names its tag;
//! 3. `unknown`: no git and no archive — a `cargo package` tarball, say.
//!
//! Nothing here may fail the build. A missing fact is `unknown`, never an
//! error: the version line is a courtesy to a bug report, not a gate.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let describe = describe(&manifest).unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=ZENCTL_GIT_DESCRIBE={describe}");
    // Without any rerun-if line cargo reruns this on every file change in
    // the package; with only this one, it reruns when this file changes.
    println!("cargo:rerun-if-changed=build.rs");
    rerun_on_head_moves(&manifest);
}

/// Run git in the crate's directory; `None` on any failure or empty output.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!s.is_empty()).then_some(s)
}

fn describe(manifest: &Path) -> Option<String> {
    from_git(manifest).or_else(|| from_archive(manifest))
}

fn from_git(manifest: &Path) -> Option<String> {
    // The repository has to be *this crate's*: its manifest must be a
    // tracked file. Fails outside git and inside a foreign checkout alike.
    git(manifest, &["ls-files", "--error-unmatch", "Cargo.toml"])?;
    git(manifest, &["describe", "--tags", "--always", "--dirty"])
}

/// `describe.txt` as `git archive` substituted it; in a checkout it still
/// holds its own `$Format:…$` placeholder, which describes nothing.
fn from_archive(manifest: &Path) -> Option<String> {
    let path = manifest.join("describe.txt");
    watch(&path);
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty() && !text.contains("$Format")).then(|| text.to_owned())
}

/// Re-describe when HEAD moves: the worktree's `HEAD` (a commit, a branch
/// switch), the branch ref it points at (a commit on it), `packed-refs`, and
/// the tag refs (a new tag changes the description).
///
/// `--dirty` is as fresh as the last rerun, deliberately: watching the index
/// would rebuild zenctl on every `git status`. A release checkout is clean,
/// and its description is exact.
fn rerun_on_head_moves(manifest: &Path) {
    // A worktree's HEAD lives in its own git dir and its refs in the common
    // one; in a plain clone they are the same directory.
    let Some(git_dir) = git(manifest, &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from)
    else {
        return;
    };
    // `--path-format` is git 2.31; an older git watches the one directory.
    let common = git(
        manifest,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map_or_else(|| git_dir.clone(), PathBuf::from);
    let head = git_dir.join("HEAD");
    watch(&head);
    if let Ok(text) = std::fs::read_to_string(&head)
        && let Some(target) = text.trim().strip_prefix("ref: ")
    {
        watch(&common.join(target));
    }
    watch(&common.join("packed-refs"));
    watch(&common.join("refs").join("tags"));
}

/// Only paths that exist: cargo treats a missing rerun-if-changed path as
/// always changed, which would rerun this on every build.
fn watch(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
