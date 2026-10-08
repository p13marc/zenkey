//! `config get|set|confirm|cancel|extend|persist` — the configuration
//! convention's client side (RFC 05 §5.1, v1.42).
//!
//! Six verbs on the `@rpc` plane, and every one of them is a v1 `@rpc` call
//! with the key spelled by the convention rather than by the operator. What
//! the noun adds is the part a generic call cannot do: it reads the served
//! schema first, so a change is typed against the declared kind rather than
//! guessed from its spelling, refused here in the producer's own words when
//! it would be refused there (`ConfigSchema::validate`, the same validator),
//! and — for a `reach` group, or a windowed change to a group whose class
//! no read-back established (#508) — never sent without a rollback armed and
//! a person saying yes, because a reach change can cut the link the reply
//! would travel on.
//!
//! A write addresses one origin. `*` is refused at the edge, before a
//! session opens: RFC 05 §2.1 forbids a fan-out write and the server refuses
//! one too (`error/fanout-forbidden`, v1.38), but this tool should not need
//! the server to tell it.

use std::io::IsTerminal as _;

use anyhow::Result;
use zenkey::config::{ConfigChange, ConfigView, ControlRequest, ParamClass, ParamValue};
use zenkey_fleet::report::{CallOutcome, CallReport};
use zenkey_fleet::{CallSpec, CallTarget};

use crate::Bus;
use crate::cli::{
    ConfigExtendArgs, ConfigGetArgs, ConfigPersistArgs, ConfigSetArgs, ConfigTokenArgs,
};
use crate::exit::unaskable;
use crate::render::{ConfigDocument, ConfigReport};

/// `config get`: the read-back — schema beside every running value.
pub async fn get(cli: ConfigGetArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let target = CallTarget::parse(&cli.origin)?;
    let path = format!("config/{}", cli.resource);
    let report = call(&bus, &target, &cli.producer, &path, None, &[]).await?;
    let code = report.exit_code();
    emit_read_back(&bus, report)?;
    exit(code)
}

/// `config set`: one group of a resource, typed against the served schema.
pub async fn set(cli: ConfigSetArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let target = one_origin(&cli.origin)?;
    let pairs = pairs(&cli.values)?;

    // The served document first: the kinds to type against, the group's
    // class, and the revision a guarded change names. With `--no-validate`,
    // or with no document served, values ride by their spelling and the
    // producer judges the rest — said out loud, because silence would look
    // like a check.
    let view = if cli.no_validate {
        None
    } else {
        read_back(&bus, &target, &cli.producer, &cli.resource).await?
    };

    let values = match &view {
        Some(view) => {
            let group = view.group(&cli.group).ok_or_else(|| {
                unaskable!(
                    "resource {:?} of {} declares no group {:?}; it declares: {}",
                    cli.resource,
                    cli.producer,
                    cli.group,
                    names(view.groups.iter().map(|g| g.name.as_str()))
                )
            })?;
            let mut values = Vec::with_capacity(pairs.len());
            for (name, text) in &pairs {
                let spec = group
                    .parameters
                    .iter()
                    .map(|p| &p.spec)
                    .find(|p| p.name == *name)
                    .ok_or_else(|| {
                        unaskable!(
                            "group {:?} declares no parameter {:?}; it declares: {}",
                            cli.group,
                            name,
                            names(group.parameters.iter().map(|p| p.spec.name.as_str()))
                        )
                    })?;
                let value = ParamValue::parse_as(&spec.kind, text)
                    .map_err(|reason| unaskable!("{name}: {reason}"))?;
                values.push((name.clone(), value));
            }
            values
        }
        None => {
            eprintln!(
                "note: no read-back to type against — values ride by their spelling \
                 (true/false, an integer, else text) and the producer judges the rest"
            );
            pairs
                .iter()
                .map(|(name, text)| (name.clone(), by_spelling(text)))
                .collect()
        }
    };

    let mut change = ConfigChange::of(values);
    change.expected_revision = cli.expect_revision;
    change.idempotency_key = cli.idempotency_key.clone();
    change.dry_run = cli.dry_run;
    change.confirm_s = cli.confirm;
    change.token = cli.token.clone();
    // The window this change is sent under: its own, or the pending
    // change's it joins (RFC v1.50).
    let window = match (&cli.token, cli.confirm) {
        (Some(token), _) => Window::Joining(token.clone()),
        (None, Some(s)) => Window::Secs(s),
        (None, None) => Window::Secs(0),
    };

    if let Some(view) = &view {
        // The producer's validator, run here with the producer's words: a
        // contract group, a reach group without a window, a value out of
        // bounds — all refused before anything leaves, as this tool's own
        // refusal of the input (exit 2), which is what they are.
        view.schema()
            .validate(&cli.group, &change)
            .map_err(|e| unaskable!("{e}"))?;
        let class = view.group(&cli.group).map(|g| g.class);
        if class == Some(ParamClass::Reach) && !cli.dry_run {
            consent(&cli.origin, &cli.group, Reach::Declared, &window, cli.yes)?;
        }
    } else if !cli.dry_run {
        // No schema, so the class is unknown — and RFC 05 §5.1 makes the
        // window the line: a reach `set` without `confirm_s` is the
        // producer's `invalid-args`, never applied, so a bare change cannot
        // cut the link and the producer is the right place to refuse it.
        // *With* `--confirm`, a reach change is exactly what this is shaped
        // as, and nothing here can say it is not one (#508). Until #508 the
        // consent above was the only one, so the form with no schema and no
        // validation — `--no-validate`, or a read-back that met silence —
        // was the one that asked nothing. A dry run touches nothing either
        // way.
        // A token joins a change that has a window, so it is shaped like a
        // reach change exactly as `--confirm` is (RFC v1.50).
        if cli.confirm.is_some() || cli.token.is_some() {
            consent(
                &cli.origin,
                &cli.group,
                Reach::Unestablished(if cli.no_validate {
                    "--no-validate skipped the read-back"
                } else {
                    "no read-back document was served"
                }),
                &window,
                cli.yes,
            )?;
        } else {
            eprintln!("note: no `--confirm`: a reach group will refuse this (RFC 05 §5.1)");
        }
    }

    let body = serde_json::to_vec(&change)?;
    let mut params = Vec::new();
    if let Some(actor) = &cli.actor {
        params.push(format!("actor={actor}"));
    }
    if let Some(id) = &cli.request_id {
        params.push(format!("request_id={id}"));
    }
    let path = format!("config/{}/{}/set", cli.resource, cli.group);
    let report = call(&bus, &target, &cli.producer, &path, Some(body), &params).await?;
    let code = report.exit_code();
    // A hot `set` answers with the read-back; a reach one with `{token,
    // apply_at}`; a dry run with what would change. The first draws as a
    // document, the others as the reply they are.
    emit_read_back(&bus, report)?;
    exit(code)
}

/// `config confirm`: make a pending change permanent.
pub async fn confirm(cli: ConfigTokenArgs) -> Result<()> {
    control(cli, "confirm", None).await
}

/// `config cancel`: undo a pending change now.
pub async fn cancel(cli: ConfigTokenArgs) -> Result<()> {
    control(cli, "cancel", None).await
}

/// `config persist`: write a change into the producer's persisted layer —
/// its own key, its own grant (RFC 05 §5.1). With no token, the read-back's
/// `last_change` (v1.50): how a change made without a window survives a
/// restart. Never the pending change by default — persisting what is not
/// yet confirmed is a decision, so it takes its token.
pub async fn persist(cli: ConfigPersistArgs) -> Result<()> {
    let ConfigPersistArgs {
        origin,
        producer,
        resource,
        token,
        bus,
    } = cli;
    let token = match token {
        Some(token) => token,
        None => {
            let b = Bus::resolve(&bus)?;
            let target = one_origin(&origin)?;
            let view = read_back(&b, &target, &producer, &resource)
                .await?
                .ok_or_else(|| {
                    unaskable!(
                        "no token given, and {origin} served no read-back to take \
                         `last_change` from — name the change's token"
                    )
                })?;
            match (view.last_change, view.pending) {
                (Some(last), _) => {
                    eprintln!(
                        "note: persisting last_change {} ({})",
                        last.token,
                        last.groups.join(", ")
                    );
                    last.token
                }
                (None, Some(p)) => {
                    return Err(unaskable!(
                        "the read-back names no last_change, and change {} is pending — \
                         confirm it first, or persist it by its token",
                        p.token
                    ));
                }
                (None, None) => {
                    return Err(unaskable!(
                        "the read-back names no last_change: nothing has been changed at \
                         runtime to persist (RFC 05 §5.1)"
                    ));
                }
            }
        }
    };
    control(
        ConfigTokenArgs {
            origin,
            producer,
            resource,
            token,
            bus,
        },
        "persist",
        None,
    )
    .await
}

/// `config extend`: move a pending change's deadline.
pub async fn extend(cli: ConfigExtendArgs) -> Result<()> {
    let ConfigExtendArgs { change, by } = cli;
    control(change, "extend", Some(by)).await
}

async fn control(cli: ConfigTokenArgs, verb: &str, confirm_s: Option<u64>) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let target = one_origin(&cli.origin)?;
    let mut request = ControlRequest::of(cli.token);
    request.confirm_s = confirm_s;
    let body = serde_json::to_vec(&request)?;
    let path = format!("config/{}/{verb}", cli.resource);
    let report = call(&bus, &target, &cli.producer, &path, Some(body), &[]).await?;
    let code = report.exit_code();
    // Each answers with the read-back after the act (RFC 05 §5.1, v1.47):
    // drawn as the document it is, like `get`'s.
    emit_read_back(&bus, report)?;
    exit(code)
}

/// One GET on `@rpc/<producer>/<path>`, through the engine's typed builders.
///
/// No slices are loaded: the body is JSON by the convention's own rule, so
/// there is no encode ladder to run, and the fan-out guard has nothing to
/// judge because every write here already refused `*` at the edge.
async fn call(
    bus: &Bus,
    target: &CallTarget,
    producer: &str,
    path: &str,
    body: Option<Vec<u8>>,
    params: &[String],
) -> Result<CallReport> {
    let session = bus.session().await?;
    let fleet = bus.fleet(&session);
    let spec = CallSpec {
        target,
        producer,
        procedure: path,
        params,
        body,
        attachment: None,
        timeout: bus.timeout(),
        slices: None,
        force: false,
    };
    Ok(zenkey_fleet::call(&fleet, spec).await?)
}

/// The served document, if the origin answered with one.
///
/// `None` is *not served* — silence, an error reply, or a reply that is not
/// an RFC 05 §5.1 read-back — and the caller says so; it is never a guessed
/// empty schema.
async fn read_back(
    bus: &Bus,
    target: &CallTarget,
    producer: &str,
    resource: &str,
) -> Result<Option<ConfigView>> {
    let report = call(
        bus,
        target,
        producer,
        &format!("config/{resource}"),
        None,
        &[],
    )
    .await?;
    Ok(report.answers.into_iter().find_map(|a| match a.outcome {
        CallOutcome::Ok { value: Some(v), .. } => serde_json::from_value(v).ok(),
        _ => None,
    }))
}

/// Draw a report whose replies are read-backs as documents, and anything
/// else as the reply it is.
fn emit_read_back(bus: &Bus, report: CallReport) -> Result<()> {
    let mut documents = Vec::new();
    let mut other = Vec::new();
    for a in report.answers {
        match &a.outcome {
            CallOutcome::Ok { value: Some(v), .. } => {
                match serde_json::from_value::<ConfigView>(v.clone()) {
                    Ok(view) => documents.push(ConfigDocument {
                        origin: a.origin,
                        view,
                    }),
                    Err(_) => other.push(a),
                }
            }
            _ => other.push(a),
        }
    }
    let report = ConfigReport {
        key: report.key,
        timeout_s: report.timeout_s,
        documents,
        other,
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
}

/// A write's target: one host, never the fleet.
fn one_origin(origin: &str) -> Result<CallTarget> {
    let target = CallTarget::parse(origin)?;
    if matches!(target, CallTarget::Fleet) {
        return Err(unaskable!(
            "a configuration change addresses one origin; a fleet (`*`) write is refused \
             (RFC 05 §2.1) — name the host"
        ));
    }
    Ok(target)
}

/// `name=value`, each; the refusal names the argument that is not one.
fn pairs(values: &[String]) -> Result<Vec<(String, String)>> {
    if values.is_empty() {
        return Err(unaskable!("nothing to set: give at least one `name=value`"));
    }
    values
        .iter()
        .map(|v| {
            v.split_once('=')
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, val)| (k.to_string(), val.to_string()))
                .ok_or_else(|| unaskable!("{v:?} is not `name=value`"))
        })
        .collect()
}

/// Without a schema: the spelling decides, and the note above says so.
fn by_spelling(text: &str) -> ParamValue {
    match text {
        "true" => ParamValue::Bool(true),
        "false" => ParamValue::Bool(false),
        _ => text
            .parse::<i64>()
            .map_or_else(|_| ParamValue::Text(text.to_string()), ParamValue::Integer),
    }
}

fn names<'a>(it: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = it.collect();
    if v.is_empty() {
        "(none)".to_string()
    } else {
        v.join(", ")
    }
}

/// The window a change is sent under, as the consent prompt names it.
#[derive(Debug, Clone)]
enum Window {
    /// Its own `--confirm`.
    Secs(u64),
    /// The pending change's, which it joins (`--token`, RFC v1.50).
    Joining(String),
}

impl Window {
    fn phrase(&self) -> String {
        match self {
            Window::Secs(s) => format!("with a {s}s rollback window"),
            Window::Joining(token) => {
                format!("under pending change {token}'s rollback window")
            }
        }
    }
}

/// Why a change needs a person's yes.
#[derive(Debug, Clone, Copy)]
enum Reach {
    /// The served schema says the group is `reach`.
    Declared,
    /// No schema to say what the group is, and the change carries a window
    /// — the shape of a reach change (#508). The reason there is no schema.
    Unestablished(&'static str),
}

impl Reach {
    /// The sentence the refusal and the prompt both open with.
    fn claim(self, origin: &str, group: &str) -> String {
        match self {
            Reach::Declared => format!("group {group:?} is reach: it can cut the link to {origin}"),
            Reach::Unestablished(why) => format!(
                "the class of group {group:?} could not be established ({why}), and a change \
                 with a window — --confirm, or --token joining one — is how a reach group is \
                 changed: it may cut the link to {origin}"
            ),
        }
    }
}

/// A reach change — or one that may be — is sent with a person's yes, or
/// with `--yes` from a script that has decided. Not at a terminal and not
/// told: refused, as this tool's own refusal (exit 2) — never silently
/// sent, never silently dropped.
fn consent(origin: &str, group: &str, reach: Reach, window: &Window, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    let claim = reach.claim(origin, group);
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Err(unaskable!(
            "{claim}. Pass --yes to send it from a script, or run at a terminal to be asked"
        ));
    }
    eprint!(
        "{claim}. Apply {}, then confirm over the new link? [y/N] ",
        window.phrase()
    );
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    match line.trim() {
        "y" | "Y" | "yes" => Ok(()),
        _ => Err(unaskable!("not sent")),
    }
}

fn exit(code: i32) -> Result<()> {
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_refuse_what_is_not_name_equals_value() {
        assert_eq!(
            pairs(&["a=1".into(), "b=x=y".into()]).unwrap(),
            vec![("a".into(), "1".into()), ("b".into(), "x=y".into())]
        );
        assert!(pairs(&[]).is_err());
        assert!(pairs(&["novalue".into()]).is_err());
        assert!(pairs(&["=1".into()]).is_err());
    }

    #[test]
    fn by_spelling_is_the_fallback_and_says_so_in_its_name() {
        assert_eq!(by_spelling("true"), ParamValue::Bool(true));
        assert_eq!(by_spelling("-7"), ParamValue::Integer(-7));
        assert_eq!(by_spelling("eu868"), ParamValue::Text("eu868".into()));
    }

    #[test]
    fn a_fleet_write_is_refused_before_a_session_opens() {
        assert!(one_origin("*").is_err());
        assert!(one_origin("h-0123456789ab").is_ok());
    }
}
