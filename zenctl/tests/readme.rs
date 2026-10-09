//! The operator README cannot rot (#516).
//!
//! `zenctl/README.md` is the document an operator takes to production, and it
//! drifted once already: accurate where it spoke, silent on a third of the
//! tree. Two checks keep it honest, both against the real thing rather than a
//! list somebody has to remember:
//!
//! * every top-level verb in the clap tree — and every `check` assertion — is
//!   named in the README as `zenctl <verb>`;
//! * the production quickstart's zenoh config (`examples/prod.json5`) is one
//!   zenctl actually opens a session through.

use std::path::{Path, PathBuf};
use std::time::Duration;

fn readme() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("zenctl/README.md")
}

/// Whether `text` names `zenctl <words>` as a whole command — `zenctl get`
/// must not be satisfied by `zenctl gen`, nor `zenctl check` by nothing but
/// `zenctl checkout`.
fn names(text: &str, words: &str) -> bool {
    let needle = format!("zenctl {words}");
    text.match_indices(&needle).any(|(i, _)| {
        text[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    })
}

#[test]
fn the_readme_names_every_top_level_verb() {
    use clap::CommandFactory as _;

    let text = readme();
    let cmd = zenctl::cli::Cli::command();
    let mut wanted: Vec<String> = Vec::new();
    for sub in cmd.get_subcommands().filter(|c| c.get_name() != "help") {
        wanted.push(sub.get_name().to_string());
        // `check` is the monitoring surface; each assertion is its own verb
        // to an operator wiring CI or a pager.
        if sub.get_name() == "check" {
            for leaf in sub.get_subcommands().filter(|c| c.get_name() != "help") {
                wanted.push(format!("check {}", leaf.get_name()));
            }
        }
    }
    let missing: Vec<&String> = wanted.iter().filter(|w| !names(&text, w)).collect();
    assert!(
        missing.is_empty(),
        "zenctl/README.md never names {missing:?} — an operator reading it would \
         not know the verb exists"
    );
    // 34 since FJ9 (#612) removed `config`, `blob`, `export`, `why` and
    // `check cutover|retired|conform`; the floor catches a walk that stopped
    // short, not a tree that shrank on purpose.
    assert!(
        wanted.len() >= 30,
        "the walk found only {} verbs",
        wanted.len()
    );
}

fn prod_json5() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/prod.json5")
}

/// The README quotes it; this keeps the quote true.
#[test]
fn the_readme_quotes_the_example_config_it_ships() {
    let text = readme();
    assert!(
        text.contains("examples/prod.json5"),
        "the README's production quickstart points at examples/prod.json5"
    );
}

/// The quickstart's file is a zenoh config, and it states the posture the
/// README promises: a client, multicast off, TLS with a CA, usrpwd.
#[test]
fn the_example_config_states_the_production_posture() {
    let cfg = zenoh::Config::from_file(prod_json5()).expect("examples/prod.json5 parses");
    let get = |key: &str| cfg.get_json(key).unwrap_or_else(|e| panic!("{key}: {e}"));
    assert_eq!(get("mode"), r#""client""#);
    assert_eq!(get("scouting/multicast/enabled"), "false");
    assert!(get("connect/endpoints").contains("tls/"), "a TLS endpoint");
    assert!(get("transport/link/tls/root_ca_certificate").contains("ca.pem"));
    assert_eq!(get("transport/link/tls/enable_mtls"), "true");
    assert_eq!(get("transport/auth/usrpwd/user"), r#""zenctl-ops""#);
}

/// Through zenctl's own door: the file builds a session config (the TLS
/// paths are placeholders, and nothing opens them before a TLS link is
/// dialled), and with a dead endpoint in place of the file's the failure is
/// the *transport* half — never `Config`, which is what a file zenctl
/// refuses would be.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zenctl_opens_a_session_through_the_example_config() {
    let file = prod_json5();
    let dead = ["tcp/127.0.0.1:1".to_string()];
    match zenkey_fleet::open_reporting_within(
        Some(&file),
        &dead,
        &[],
        None,
        Duration::from_secs(10),
    )
    .await
    {
        Err(zenkey_fleet::OpenFailure::Transport(_)) => {}
        Err(zenkey_fleet::OpenFailure::Config(e)) => {
            panic!("zenctl refuses examples/prod.json5: {e}")
        }
        Ok(_) => panic!("a session opened onto tcp/127.0.0.1:1, where nothing listens"),
    }
}
