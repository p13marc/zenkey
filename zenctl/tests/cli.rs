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
//! algebra, `registry lint`, `check schema --schema-set`, and both halves of
//! #196's open-failure fork. Verbs that must reach a producer to say anything
//! are `--help`-only; their *rendering* is pinned by the render snapshots
//! instead, which exercise the same code with no process and no network.
//!
//! The files are grouped the way the tree is (#307): `help-<noun>` per noun
//! family, `help-wire` for the verbs that hang off the root, `help-check` for
//! the exit-coded assertions, and the behaviour cases beside them.
//!
//! #201 also asks for a *mechanical* floor — every leaf verb named by at least
//! one case, checked rather than remembered. `the_corpus_names_every_leaf_verb`
//! is that check, and being able to write it is the whole reason `zenctl`
//! grew a library target: it walks the real clap tree rather than a list
//! somebody has to remember to update.
//!
//! ## Exit 2 has one meaning, and the corpus is where that was settled
//!
//! `zenctl key includes 'bad[' x` exits 2 because the expression is invalid
//! (`cmd/key.rs`); `zenctl key includes` exits 2 because clap rejected the
//! usage. Those used to be pinned side by side as a *collision*. They are the
//! same claim now — **2 = no verdict, the question could not be asked** — and
//! the corpus is what made the disagreement visible: two files argued opposite
//! doctrines (`key-algebra` for "your input is a 2",
//! `session-config-error` for "your input is a 1") until #307 wrote the
//! contract down once in `zenctl/src/exit.rs` and moved every refusal onto it.
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
    cases_in(test_home())
}

fn cases_in(home: PathBuf) -> trycmd::TestCases {
    let t = trycmd::TestCases::new();
    t.env("ZENKEY_EXPLORER_CONFIG_DIR", home.to_str().unwrap())
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
        // rendered as *required* — in every help text.
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

/// The context/cache family (`tests/cmd/contexts/`) runs in a home of its
/// own. Its cases *write* the home they run in — a context they create is
/// every concurrent case's active context, and any case that loads slices
/// (`topic info --registry …`, session-transport-fallback) fills the cache
/// keyed by whatever context is active at that instant — so sharing the
/// glob's home makes `cache clear`'s pinned `existed:false` a race against
/// trycmd's file scheduling. Same `cli-home` root, so one
/// `rm -rf target/tmp/cli-home` still resets the whole corpus.
#[test]
fn the_stateful_context_family_runs_in_a_home_of_its_own() {
    let home = test_home().join("contexts");
    std::fs::create_dir_all(&home).expect("create the context family's config root");
    cases_in(home).case("tests/cmd/contexts/*.trycmd");
}

/// Walk the tree the binary actually builds, and collect the leaves.
fn leaves(cmd: &clap::Command, path: &mut Vec<String>, out: &mut Vec<String>) {
    let subs: Vec<&clap::Command> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .collect();
    if subs.is_empty() {
        out.push(path.join(" "));
        return;
    }
    for sub in subs {
        path.push(sub.get_name().to_string());
        leaves(sub, path, out);
        path.pop();
    }
}

/// Every leaf verb is named by at least one case, so the tree cannot grow in
/// silence.
///
/// A list of verbs kept by hand is a list that goes stale the first time
/// somebody is in a hurry. This walks `Cli::command()` — the same tree the
/// binary parses with — which is what the library target bought.
#[test]
fn the_corpus_names_every_leaf_verb() {
    use clap::CommandFactory as _;

    let cmd = zenctl::cli::Cli::command();
    let mut found = Vec::new();
    leaves(&cmd, &mut Vec::new(), &mut found);

    // Both corpora count: the flat glob and the stateful context family in
    // its own subdirectory (one level is all the layout has).
    let corpus: String = std::fs::read_dir("tests/cmd")
        .expect("the corpus directory")
        .chain(std::fs::read_dir("tests/cmd/contexts").expect("the context family"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "trycmd"))
        .map(|e| std::fs::read_to_string(e.path()).expect("a case file"))
        .collect();

    let missing: Vec<&String> = found
        .iter()
        .filter(|verb| !corpus.contains(&format!("$ zenctl {verb} ")))
        .collect();
    assert!(
        missing.is_empty(),
        "{} leaf verb(s) appear in no case: {missing:?}\n\
         Every verb owes the corpus at least its `--help`; that alone would \
         have caught the runs of spaces in #195.",
        missing.len()
    );
    assert!(
        found.len() >= 50,
        "the walk found only {} leaves — it stopped short somewhere",
        found.len()
    );
}
