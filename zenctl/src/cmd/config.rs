//! `config get|set|confirm|cancel|extend|persist` (#768, CC) — a running
//! service's configuration, through `config.v1`
//! (`spec/profiles/config/v1.md`).
//!
//! v1's six verbs on the `@rpc` plane went dark at FJ9 (#612) until the
//! profile existed; they come back as zk2's. The reading and the acts are
//! the engine's ([`zenkey_fleet::run_config`], [`zenkey_fleet::call_config`])
//! and so is every rule they apply — the premise, freshness, §5's answers,
//! the value typing, the owner's own validator, the consent a reach change
//! asks, redaction. This command is orchestration, input and rendering:
//! resolve the deployment, open a session **in** its namespace, read the
//! command line (a sensitive value from a file or standard input), ask the
//! operator when the engine says a change can cut the link, call, print,
//! and exit by the report's own code.
//!
//! **Exit codes** (`crate::exit`). `get` is a read: 0 a document read, 1 the
//! owner answered with bytes that are not its document, 2 nothing read. An
//! act keeps its 1 (`crate::exit`, "Acts keep their 1"): 0 the owner
//! answered with its reply type, 1 a refusal (the act failed), 2 silence —
//! unestablished, never a failure (core O5) — or an input this tool
//! refuses before sending anything: a `NAME=VALUE` that is not one, a
//! sensitive value given inline, a change the owner's own checks refuse
//! against the view just read, a reach change with no yes.

use std::collections::BTreeSet;
use std::io::{IsTerminal as _, Read as _};
use std::time::SystemTime;

use anyhow::Result;
use zenkey_fleet::model::config::{self as mc, Assignment, Given};
use zenkey_fleet::report::{ConfigActAnswer, ConfigChecked, ConfigCheckedRead};
use zenkey_fleet::{ConfigCall, ConfigOp, ConfigSpec, ConfigTarget};
use zenkey_model::config::{Answer, ConfigStatus, ConfigView, ControlRequest, SetRequest};

use crate::bus::Deployment;
use crate::cli::{ClaimArgs, ConfigExtendArgs, ConfigGetArgs, ConfigSetArgs, ConfigTokenArgs};
use crate::exit::unaskable;

/// The exit a report chose: through `main` for 0, the process's for the
/// rest, as `call` exits.
fn exit(code: i32) -> Result<()> {
    match code {
        crate::exit::CLEAN => Ok(()),
        code => std::process::exit(code),
    }
}

/// `config get`: one reading, §5's answers beside it.
pub async fn get(cli: ConfigGetArgs) -> Result<()> {
    let ConfigGetArgs {
        address,
        status,
        token,
        instance,
        desired_revision,
        for_secs,
        clocks_synced,
        across_face,
        ns,
    } = cli;
    let dep = Deployment::resolve(&ns)?;
    let window = for_secs
        .map(|s| super::positive_secs("--for", s))
        .transpose()?;
    let spec = ConfigSpec {
        timeout: dep.timeout(),
        window,
        clocks_synced,
        across_face,
        status_only: status,
        token,
        instance,
        desired_revision,
    };
    let session = dep.session().await?;
    if let Some(w) = window {
        eprintln!(
            "config get: listening {}s to the view, the status and the change events before the \
             GET (--for)",
            w.as_secs_f64()
        );
    }
    let report = zenkey_fleet::run_config(&session, dep.namespace(), address, spec).await;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    exit(report.exit_code())
}

/// One `NAME=VALUE` of the command line: inline, from a file (`@FILE`), or
/// from standard input (`@-`, once); `@@…` is a text that begins with `@`.
/// A value read is never echoed, in an error either.
fn assignments(args: &[String]) -> Result<Vec<Assignment>> {
    let mut stdin_taken = false;
    let mut out = Vec::with_capacity(args.len());
    for a in args {
        let Some((name, value)) = a.split_once('=').filter(|(n, _)| !n.is_empty()) else {
            return Err(unaskable!(
                "{a:?} is not NAME=VALUE (a value from a file is NAME=@FILE, from standard input \
                 NAME=@-)"
            ));
        };
        let trimmed = |mut s: String| {
            if s.ends_with('\n') {
                s.pop();
                if s.ends_with('\r') {
                    s.pop();
                }
            }
            s
        };
        let given = if value == "@-" {
            if stdin_taken {
                return Err(unaskable!(
                    "{name}=@-: standard input gives one value only, and another took it"
                ));
            }
            stdin_taken = true;
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| unaskable!("{name}=@-: standard input could not be read: {e}"))?;
            Given::Read(trimmed(s))
        } else if let Some(text) = value.strip_prefix("@@") {
            Given::Inline(format!("@{text}"))
        } else if let Some(path) = value.strip_prefix('@') {
            let s = std::fs::read_to_string(path)
                .map_err(|e| unaskable!("{name}=@{path}: the file could not be read: {e}"))?;
            Given::Read(trimmed(s))
        } else {
            Given::Inline(value.to_owned())
        };
        out.push(Assignment {
            name: name.to_owned(),
            given,
        });
    }
    Ok(out)
}

/// The call metadata an act claims (core O7): `--actor`, else `$USER`;
/// `--request-id`, else a fresh ULID.
fn claimed(claim: &ClaimArgs) -> zenkey::CallMetadata {
    zenkey::CallMetadata {
        actor: claim
            .actor
            .clone()
            .or_else(|| std::env::var("USER").ok().filter(|u| !u.is_empty())),
        request_id: Some(
            claim
                .request_id
                .clone()
                .unwrap_or_else(|| zenkey::writer::ulid(SystemTime::now())),
        ),
    }
}

/// The operator's yes to a change that can cut the link (§2.4), or this
/// tool's refusal to send it: at a terminal it asks; from a script it takes
/// `--i-know`, and refuses without it — never sent silently, never dropped
/// silently.
fn consent(claim: &str, i_know: bool) -> Result<()> {
    if i_know {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(unaskable!(
            "{claim}. Pass --i-know to send it from a script, or run at a terminal to be asked: \
             not sent"
        ));
    }
    eprint!("{claim}. Send it, and confirm over the new link? [y/N] ");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    match line.trim() {
        "y" | "Y" | "yes" => Ok(()),
        _ => Err(unaskable!("not sent")),
    }
}

/// `config set`: one group, typed and checked against the view first.
pub async fn set(cli: ConfigSetArgs) -> Result<()> {
    let ConfigSetArgs {
        address,
        group,
        values,
        confirm_s,
        join,
        expected_revision,
        idempotency_key,
        dry_run,
        no_precheck,
        across_face,
        i_know,
        claim,
        ns,
    } = cli;
    // Everything this tool can refuse of the command line, before a session.
    let given = assignments(&values)?;
    let dep = Deployment::resolve(&ns)?;
    let session = dep.session().await?;
    let t = dep.timeout();
    // What to check against: the view (its schema and its facts), or across
    // a face the status (its values), or nothing.
    let read = if no_precheck {
        ConfigCheckedRead::Nothing
    } else if across_face {
        ConfigCheckedRead::Status
    } else {
        ConfigCheckedRead::View
    };
    let (view, status): (Option<ConfigView>, Option<ConfigStatus>) = match read {
        ConfigCheckedRead::Nothing => (None, None),
        ConfigCheckedRead::View | ConfigCheckedRead::Status => {
            let selector = match read {
                ConfigCheckedRead::View => zenkey_fleet::bus::config::view_key(&address),
                _ => ConfigTarget::One(address.clone()).state_selector(true),
            };
            let got = zenkey_fleet::get_config(&session, &selector, t)
                .await
                .map_err(|e| anyhow::anyhow!("config set: reading {selector}: {e}"))?;
            let docs = got.services.get(&address);
            let view = docs
                .and_then(|d| d.view.as_ref())
                .and_then(|r| r.value.clone())
                .and_then(|v| v.ok());
            let status = docs
                .and_then(|d| d.status.as_ref())
                .and_then(|r| r.value.clone())
                .and_then(|v| v.ok());
            if view.is_none() && status.is_none() {
                eprintln!(
                    "config set: `{selector}` answered no document within {}s: the values are \
                     typed by their spelling, nothing is checked here, and the owner judges the \
                     rest",
                    t.as_secs_f64()
                );
            }
            (view, status)
        }
    };
    let typed = mc::type_values(view.as_ref(), status.as_ref(), &group, &given)
        .map_err(|e| unaskable!("{e}"))?;
    let request = SetRequest {
        values: typed.values.clone(),
        confirm_s,
        token: join,
        expected_revision,
        idempotency_key,
        dry_run,
    };
    if let Some(v) = &view
        && let Err(refusal) = mc::precheck(v, &group, &request)
    {
        let refusal = mc::scrub(&refusal.to_string(), &secrets(&given, &typed.read));
        return Err(unaskable!(
            "refused here, nothing sent — {refusal}: the owner refuses this change so against \
             the view just read (instance {}, revision {}; config.v1 §2.5). --no-precheck sends \
             it anyway",
            v.instance,
            v.revision
        ));
    }
    let no_class = match read {
        ConfigCheckedRead::Nothing => "--no-precheck read no view",
        ConfigCheckedRead::Status => "across a face the status is read, which carries no class",
        ConfigCheckedRead::View if view.is_some() => "the view declares no such group",
        ConfigCheckedRead::View => "no view answered",
    };
    if let Some(claim) = mc::consent(
        view.as_ref(),
        &address.to_string(),
        &group,
        &request,
        no_class,
    ) {
        consent(&claim, i_know)?;
    }
    let checked = ConfigChecked {
        read,
        typed_by: typed.typed_by,
        instance: view
            .as_ref()
            .map(|v| v.instance.clone())
            .or_else(|| status.as_ref().map(|s| s.instance.clone())),
        revision: view
            .as_ref()
            .map(|v| v.revision)
            .or_else(|| status.as_ref().map(|s| s.revision)),
        class: view
            .as_ref()
            .and_then(|v| v.groups.iter().find(|g| g.name == group))
            .map(|g| g.class),
    };
    let report = zenkey_fleet::call_config(
        &session,
        ConfigCall {
            namespace: dep.namespace(),
            address: &address,
            op: ConfigOp::Set {
                group: group.clone(),
            },
            request: serde_json::to_value(&request)?,
            shown: mc::shown_request(&request, view.as_ref(), &group, &typed.read),
            secrets: secrets(&given, &typed.read),
            sensitive: mc::sensitive_params(view.as_ref()),
            metadata: Some(claimed(&claim)),
            timeout: dep.chosen_timeout().unwrap_or(t),
            across_face,
            checked: Some(checked),
        },
    )
    .await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    if let (ConfigActAnswer::Pending { pending }, Some(instance)) = (
        &report.answer,
        report.checked.as_ref().and_then(|c| c.instance.as_ref()),
    ) {
        eprintln!(
            "config set: whether it lands: zenctl config get {address} --token {} --instance \
             {instance}",
            pending.token
        );
    }
    exit(report.exit_code())
}

/// The values read from a file or standard input: never echoed, and taken
/// out of anything the owner sends back (§2.15).
fn secrets(given: &[Assignment], read: &BTreeSet<String>) -> Vec<String> {
    given
        .iter()
        .filter(|a| read.contains(&a.name))
        .filter_map(|a| match &a.given {
            Given::Read(s) => Some(s.clone()),
            Given::Inline(_) => None,
        })
        .collect()
}

/// A control operation: its `ControlRequest`, sent, and its reply drawn.
async fn control(cli: ConfigTokenArgs, op: ConfigOp, confirm_s: Option<u64>) -> Result<()> {
    let ConfigTokenArgs {
        address,
        token,
        status,
        across_face,
        claim,
        ns,
    } = cli;
    let dep = Deployment::resolve(&ns)?;
    let request = ControlRequest {
        token,
        confirm_s,
        answer: status.then_some(Answer::Status),
    };
    let session = dep.session().await?;
    let request = serde_json::to_value(&request)?;
    let report = zenkey_fleet::call_config(
        &session,
        ConfigCall {
            namespace: dep.namespace(),
            address: &address,
            op,
            shown: request.clone(),
            request,
            secrets: Vec::new(),
            sensitive: BTreeSet::new(),
            metadata: Some(claimed(&claim)),
            timeout: dep.timeout(),
            across_face,
            checked: None,
        },
    )
    .await?;
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    exit(report.exit_code())
}

/// `config confirm`.
pub async fn confirm(cli: ConfigTokenArgs) -> Result<()> {
    control(cli, ConfigOp::Confirm, None).await
}

/// `config cancel`.
pub async fn cancel(cli: ConfigTokenArgs) -> Result<()> {
    control(cli, ConfigOp::Cancel, None).await
}

/// `config extend`.
pub async fn extend(cli: ConfigExtendArgs) -> Result<()> {
    let ConfigExtendArgs { change, confirm_s } = cli;
    control(change, ConfigOp::Extend, Some(confirm_s)).await
}

/// `config persist`.
pub async fn persist(cli: ConfigTokenArgs) -> Result<()> {
    control(cli, ConfigOp::Persist, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `NAME=VALUE`, each; `@@` a text beginning with `@`; the refusal names
    /// the argument that is not one, and never a value read.
    #[test]
    fn assignments_refuse_what_is_not_name_equals_value() {
        let a = assignments(&["a=1".into(), "b=x=y".into(), "c=@@at".into()]).unwrap();
        assert_eq!(a[0].given, Given::Inline("1".into()));
        assert_eq!(a[1].given, Given::Inline("x=y".into()));
        assert_eq!(a[2].given, Given::Inline("@at".into()));
        assert!(assignments(&["novalue".into()]).is_err());
        assert!(assignments(&["=1".into()]).is_err());
        let dir = std::env::temp_dir().join(format!("zenctl-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pin = dir.join("pin");
        std::fs::write(&pin, "1234\n").unwrap();
        let a = assignments(&[format!("pin=@{}", pin.display())]).unwrap();
        assert_eq!(a[0].given, Given::Read("1234".into()));
        let missing = assignments(&["pin=@/nonexistent/pin".into()]).unwrap_err();
        assert_eq!(crate::exit::code_for(&missing), crate::exit::NO_VERDICT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only a value read is a secret.
    #[test]
    fn a_value_read_is_a_secret() {
        let given = vec![
            Assignment {
                name: "pin".into(),
                given: Given::Read("1234".into()),
            },
            Assignment {
                name: "x".into(),
                given: Given::Inline("5".into()),
            },
        ];
        let read = BTreeSet::from(["pin".to_owned(), "x".to_owned()]);
        assert_eq!(secrets(&given, &read), ["1234"]);
    }
}
