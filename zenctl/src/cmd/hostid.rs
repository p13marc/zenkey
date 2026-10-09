//! `zenctl hostid` — this host's system, as `hostid.v1` mints it (#719).
//!
//! The runtime's own ladder (`zenkey::hostid`), read-only: the inputs in
//! order (`spec/profiles/hostid/v1.md` §2.4), the shared file read but
//! never created, the system derived with the one salt (§2.1, §2.2). What
//! it prints is what a service on this host asking for
//! `@hostid.v1/<service>` would get, unless the shared file is still to be
//! made. `--machine-id` derives from a given id instead, and each
//! `--v1-salt` adds the v1 origin the id had under that salt, for the
//! migration table (Appendix B). It opens no session, and never prints a
//! machine id (§2.10).
//!
//! Exit (the contract in `exit.rs`): 0 with a system; 2 when the host has
//! no id a service could mint from, every path named with its outcome
//! ([`Unanswered`](crate::exit::Unanswered)), or when `--machine-id` is not
//! a machine id ([`Unaskable`](crate::exit::Unaskable)).

use anyhow::Result;
use zenkey::hostid::{DBUS_MACHINE_ID, ETC_MACHINE_ID, HostIdError, HostIdSource, Outcome};
use zenkey_model::hostid as derivation;

use crate::cli::HostidArgs;
use crate::exit::{unanswered, unaskable};
use crate::render::{HostIdInput, HostIdReport, V1Origin};

/// `zenctl hostid`.
pub fn run(cli: HostidArgs) -> Result<()> {
    let report = report(&cli)?;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report,
        cli.out.format,
        cli.out.color,
    )
}

/// The report, or why there is none.
pub fn report(cli: &HostidArgs) -> Result<HostIdReport> {
    if let Some(hex) = &cli.machine_id {
        let Some(system) = derivation::system(hex) else {
            return Err(unaskable!(
                "--machine-id is not a machine id: hostid.v1 takes 32 hex digits, not all \
                 zeros, once ASCII whitespace is trimmed (hostid.v1 §2.1)"
            ));
        };
        return Ok(HostIdReport {
            system: system.to_string(),
            from: "--machine-id".to_owned(),
            salt: derivation::SALT.to_owned(),
            inputs: Vec::new(),
            v1: cli
                .v1_salt
                .iter()
                .map(|salt| V1Origin {
                    salt: salt.clone(),
                    origin: derivation::derive(hex, salt),
                })
                .collect(),
        });
    }
    let source = match &cli.root {
        Some(root) => HostIdSource::at(root),
        None => HostIdSource::host(),
    }
    .read_only();
    let minted = source
        .mint(false)
        .map_err(|e| unanswered!("{}", no_id(&e)))?;
    let from = minted.source().unwrap_or("?").to_owned();
    // v1 never read the shared file: an id from it has no v1 origin
    // (Appendix B).
    let v1_read_it = from == ETC_MACHINE_ID || from == DBUS_MACHINE_ID;
    Ok(HostIdReport {
        system: minted.system().to_string(),
        from,
        salt: derivation::SALT.to_owned(),
        inputs: minted
            .trail()
            .iter()
            .map(|a| HostIdInput {
                path: a.path.to_owned(),
                outcome: a.outcome.label().to_owned(),
            })
            .collect(),
        v1: cli
            .v1_salt
            .iter()
            .map(|salt| V1Origin {
                salt: salt.clone(),
                origin: v1_read_it.then(|| minted.with_salt(salt)).flatten(),
            })
            .collect(),
    })
}

/// Why there is no system, one path per line with its outcome (§2.6).
///
/// Two cases. The host fails closed: a service would not start either. Or
/// no input holds an id and the shared file is absent: a service would
/// create it (§2.5), and its random content would decide the system, but
/// this verb only reads.
fn no_id(e: &HostIdError) -> String {
    let undecided = e.trail().last().is_some_and(|a| {
        a.path == zenkey::hostid::SHARED_FILE && matches!(a.outcome, Outcome::Absent)
    });
    let mut out = String::from(if undecided {
        "this host's system is not decided yet: no input holds a machine id, and a service \
         asking for a minted system would create the shared file, whose random content \
         decides it (hostid.v1 §2.5). zenctl only reads:"
    } else {
        "this host has no id hostid.v1 can mint a system from, so a service asking for one \
         would not start (hostid.v1 §2.6):"
    });
    for a in e.trail() {
        out.push_str(&format!("\n  {}: {}", a.path, a.outcome));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const M1: &str = "b642b4217b34b1e8d3bd915fc65c4452";

    fn args(root: Option<&Path>, machine_id: Option<&str>, salts: &[&str]) -> HostidArgs {
        HostidArgs {
            machine_id: machine_id.map(str::to_owned),
            v1_salt: salts.iter().map(|s| (*s).to_owned()).collect(),
            root: root.map(Path::to_path_buf),
            out: crate::cli::OutputArgs {
                format: crate::render::Format::Json,
                color: crate::render::ColorChoice::Never,
            },
        }
    }

    fn put(root: &Path, abs: &str, content: &str) {
        let p = root.join(abs.trim_start_matches('/'));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    /// Exit 0: the first input with an id, and the v1 origins of Appendix B.
    #[test]
    fn a_host_with_a_machine_id() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), ETC_MACHINE_ID, &format!("{M1}\n"));
        let r = report(&args(Some(root.path()), None, &["tcgui-host-id-v1"])).unwrap();
        assert_eq!(r.system, "h-bbd1aa1db10b");
        assert_eq!(r.from, ETC_MACHINE_ID);
        assert_eq!(r.v1[0].origin.as_deref(), Some("h-eed9ac91da20"));
    }

    /// Exit 2, unanswered: every path named, and nothing written.
    #[test]
    fn a_host_without_an_id() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), ETC_MACHINE_ID, "uninitialized\n");
        let e = report(&args(Some(root.path()), None, &[])).unwrap_err();
        assert_eq!(crate::exit::code_for(&e), crate::exit::NO_VERDICT);
        let text = e.to_string();
        for line in [
            "/etc/machine-id: refused",
            "/var/lib/dbus/machine-id: absent",
            "/var/lib/zk2/hostid: absent",
        ] {
            assert!(text.contains(line), "{text}");
        }
        assert!(!root.path().join("var").exists(), "nothing created");
    }

    /// Exit 2: an unreadable input fails closed, with the OS's error.
    #[test]
    fn an_unreadable_input() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("etc/machine-id")).unwrap();
        let e = report(&args(Some(root.path()), None, &[])).unwrap_err();
        assert_eq!(crate::exit::code_for(&e), crate::exit::NO_VERDICT);
        assert!(
            e.to_string()
                .contains("/etc/machine-id: unreadable (not a regular file"),
            "{e}"
        );
    }

    /// Exit 2, unaskable: a `--machine-id` §2.1 refuses. The id is never
    /// echoed.
    #[test]
    fn a_given_id_that_is_not_one() {
        let e = report(&args(None, Some("0000"), &[])).unwrap_err();
        assert_eq!(crate::exit::code_for(&e), crate::exit::NO_VERDICT);
        let r = report(&args(None, Some(&M1.to_uppercase()), &["example-salt-v1"])).unwrap();
        assert_eq!(r.system, "h-bbd1aa1db10b");
        assert_eq!(r.v1[0].origin.as_deref(), Some("h-20609002f7b6"));
    }

    /// The shared file's id has no v1 origin: no v1 application read it.
    #[test]
    fn an_id_from_the_shared_file_has_no_v1_origin() {
        let root = tempfile::tempdir().unwrap();
        put(root.path(), "/var/lib/zk2/hostid", &format!("{M1}\n"));
        let r = report(&args(Some(root.path()), None, &["tcgui-host-id-v1"])).unwrap();
        assert_eq!(r.from, "/var/lib/zk2/hostid");
        assert_eq!(r.v1[0].origin, None);
    }
}
