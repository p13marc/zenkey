//! The CLI floor: what `zenctl` prints and what it exits with (#201).
//!
//! Deliberately landed **before** the renderer rewrite (#198/#199) rather than
//! after it. A corpus written afterwards can only pin whatever the refactor
//! happened to produce; this one guards the refactor, so a clap tree that
//! loses a flag or a verb during the move fails here rather than in a release.
//!
//! `trycmd` cases are literate: `tests/cmd/*.trycmd` reads as documentation of
//! the offline surface and is checked as a test. Accept new output with
//! `just snapshots` (`TRYCMD=overwrite cargo test -p zenctl`) — and read the
//! diff, because that reading is the point of the corpus rather than its cost.
//!
//! ## What is in here, and what is not
//!
//! Only verbs that answer **without a bus**, because anything else is a flake
//! wearing a test's name. That is more than it sounds: `--help` for every leaf
//! verb (which alone would have caught #195's runs of spaces), the key-expression
//! algebra, `registry lint`, `schema check --schema-set`, and both halves of
//! #196's open-failure fork. Verbs that must reach a producer to say anything
//! are `--help`-only; their *rendering* is pinned by the render snapshots
//! instead, which exercise the same code with no process and no network.
//!
//! #201 also asks for a *mechanical* floor — every leaf verb named by at least
//! one case, checked rather than remembered. That test walks the clap tree, so
//! it needs the tree to be importable, and it lands with the library target in
//! the next commit. Until then the list was built by walking `--help` by hand:
//! 51 leaves, 14 group nodes, and the root.
//!
//! ## Exit 2 is overloaded, and the corpus says so out loud
//!
//! `zenctl key includes 'bad[' x` exits 2 because the expression is invalid
//! (`cmd/key.rs:29`); `zenctl key includes` exits 2 because clap rejected the
//! usage. Both are pinned, side by side, so the collision is visible in the
//! corpus rather than discovered by someone branching on the code.
//!
//! ## Hermeticity
//!
//! Three things would otherwise make this corpus a function of the developer's
//! shell rather than of the code:
//!
//! * `ZENKEY_EXPLORER_CONFIG_DIR` overrides both the config file and the slice
//!   cache (`context_store.rs`). Unset, a developer with an active context
//!   gets different output than CI.
//! * `--format` carries `env = "ZENCTL_FORMAT"`, and a *set* value makes clap
//!   render the flag as **required** in every usage line — `Usage: zenctl key
//!   includes --format <FORMAT> <A> <B>`, which is not what a user sees. So it
//!   is asserted unset, and the cases that want a table pass `--format table`
//!   explicitly, which reads better in a literate file anyway.
//! * `ZENCTL_CONTEXT` and `ZENCTL_ZENOH_CONFIG` would each silently redirect a
//!   run. `TestCases` has `env()` but no `env_remove()`, so they are guarded by
//!   an assertion instead — see `the_corpus_refuses_an_environment_it_does_not_control`.

use std::path::PathBuf;

/// A config/cache root under the target dir: writable, per-run, and nowhere
/// near the developer's own `~/.config/zenctl`.
fn test_home() -> PathBuf {
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli-home");
    std::fs::create_dir_all(&home).expect("create the corpus' config root");
    home
}

fn cases() -> trycmd::TestCases {
    let t = trycmd::TestCases::new();
    t.env("ZENKEY_EXPLORER_CONFIG_DIR", test_home().to_str().unwrap())
        // Set *empty* rather than left alone: empty is the documented default
        // (the base-less bus-root deployment, RFC v1.6), clap renders
        // `[env: ZENCTL_BASE=]` for unset and empty alike, and pinning it stops
        // a developer's export from rewriting every case. `ZENCTL_FORMAT` gets
        // the opposite treatment — see the guard test above.
        .env("ZENCTL_BASE", "")
        // Colour is not in the tree yet (#200), but the corpus is what will
        // prove it changes nothing — so it is pinned off from the start.
        .env("NO_COLOR", "1")
        .env("CLICOLOR_FORCE", "0")
        .env("TERM", "dumb")
        // zenoh logs its connect failures at WARN, and the text carries
        // absolute paths into the *build machine's* cargo registry — a string
        // no snapshot can ever be stable against. Silencing the subscriber is
        // the honest trade: this corpus pins what zenctl says, and zenoh's own
        // diagnostics are not that.
        .env("RUST_LOG", "off");
    t
}

/// A corpus is only reproducible in an environment it controls, and two of the
/// variables that would break it cannot be unset through `TestCases`. Say so
/// in one sentence rather than letting a stray export become a mystery.
#[test]
fn the_corpus_refuses_an_environment_it_does_not_control() {
    for var in [
        "ZENCTL_CONTEXT",
        "ZENKEY_EXPLORER_CONTEXT",
        "ZENCTL_ZENOH_CONFIG",
        // Not merely a different default: a set value makes clap print
        // `Usage: zenctl key includes --format <FORMAT> <A> <B>` — the flag
        // rendered as *required* — in all 65 help texts.
        "ZENCTL_FORMAT",
    ] {
        assert!(
            std::env::var_os(var).is_none(),
            "{var} is set in this shell and would redirect every case in the \
             corpus; `trycmd::TestCases` can set variables but not clear them, \
             so unset it before running the tests"
        );
    }
}

#[test]
fn the_offline_surface_prints_what_it_has_always_printed() {
    cases().case("tests/cmd/*.trycmd");
}
