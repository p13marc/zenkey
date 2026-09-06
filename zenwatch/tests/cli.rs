//! The CLI floor (#201's discipline, for the third binary): `--help` for
//! every verb, and `check-config` on each fixture with its exit code — the
//! refusals, pinned as transcripts. Nothing here needs a bus.
//!
//! Accept new output with `TRYCMD=overwrite cargo test -p zenwatch`, and read
//! the diff.

use std::path::PathBuf;

fn test_home() -> PathBuf {
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli-home");
    std::fs::create_dir_all(&home).expect("create the corpus' config root");
    home
}

fn cases() -> trycmd::TestCases {
    let t = trycmd::TestCases::new();
    t.env("ZENKEY_EXPLORER_CONFIG_DIR", test_home().to_str().unwrap())
        // The fixtures name env secrets; the corpus supplies them, and
        // leaves `ZENWATCH_UNSET_TOKEN` unset on purpose.
        .env("NTFY_TOKEN", "test-token")
        .env("HOOK_KEY", "test-key")
        .env("SMTP_PASSWORD", "test-password")
        .env("TERM", "dumb")
        // zenoh's own diagnostics carry build-machine paths; the corpus pins
        // what zenwatch says.
        .env("RUST_LOG", "off");
    t
}

/// `ZENWATCH_CONFIG` would redirect every case; `TestCases` cannot unset it.
#[test]
fn the_corpus_refuses_an_environment_it_does_not_control() {
    for var in [
        "ZENWATCH_CONFIG",
        "ZENWATCH_UNSET_TOKEN",
        "ZENKEY_EXPLORER_CONTEXT",
        "ZENCTL_CONTEXT",
    ] {
        assert!(
            std::env::var_os(var).is_none(),
            "{var} is set in this shell and would redirect the corpus; unset it"
        );
    }
}

#[test]
fn the_offline_surface_prints_what_it_has_always_printed() {
    cases().case("tests/cmd/*.trycmd");
}

/// Every verb is named by at least one case — walked off the real clap
/// tree, not a list somebody has to remember to update.
#[test]
fn the_corpus_names_every_verb() {
    use clap::CommandFactory as _;
    let cmd = zenwatch::cli::Cli::command();
    let corpus: String = std::fs::read_dir("tests/cmd")
        .expect("the corpus directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "trycmd"))
        .map(|e| std::fs::read_to_string(e.path()).expect("a case file"))
        .collect();
    let missing: Vec<String> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .map(|c| c.get_name().to_string())
        .filter(|verb| !corpus.contains(&format!("$ zenwatch {verb} ")))
        .collect();
    assert!(missing.is_empty(), "verb(s) in no case: {missing:?}");
}
