//! `zenctl get` — the fleet discipline on any selector (#114), and `get
//! state`, a zk2 state resource read through its contract (#612, FJ5).
//!
//! The two forms never share a positional: `get <SELECTOR>` is RAW — a wire
//! selector on a session in no namespace, any bus — and
//! `get state <ADDRESS> <IFACE> <STATE>` is RESOLVED, in the deployment's
//! namespace, through the runtime's consumer ([`state`]).
//!
//! A plain fan-in GET: target `All`, consolidation `None`, every reply
//! attributed by its own key (RFC 05 §2.1), error envelopes rendered as
//! errors — through `zenkey_fleet::fleet_get`, never the JSON-lossy admin
//! browse. Each reply is resolved through the same lens `echo` reads a
//! sample through (#612, FJ9): the deployment in `--namespace`, its
//! presence read, the contract each descriptor names. A zk2 key's payload
//! is decoded as its declared type and a JSON Schema value checked against
//! it; where the ladder stops, the rung is named; a foreign key renders
//! structurally. v1's served-schema ladder left with the v1 registry.

use anyhow::Result;
use zenkey_model::authoring::Kind;

use super::sample::{
    Line, Payload, SampleLine, attachment_display, attachment_json, format_sample, hex,
};
use crate::bus::Deployment;
use crate::cmd::zk2;
use zenkey_fleet::report::Conformance;
use zenkey_fleet::{Answer, FleetAnswer, Lens};

/// The reply discipline as an exit code: 0 = value replies only, 1 = at
/// least one error envelope, 2 = silence (which is still not a verdict —
/// the code is for scripts, the paragraph is for people).
fn exit_code(answers: &[FleetAnswer]) -> i32 {
    if answers.is_empty() {
        2
    } else if answers
        .iter()
        .any(|a| matches!(a.answer, Answer::Error { .. }))
    {
        1
    } else {
        0
    }
}

pub async fn run(cli: crate::cli::GetArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let crate::cli::GetArgs {
        selector,
        body,
        raw,
        hex: hex_payload,
        fmt,
        no_decode,
        cmd: _,
        contracts: _,
        ns: _,
    } = cli;
    // Clap requires it whenever `state` is not given.
    let selector = selector.ok_or_else(|| crate::exit::unaskable!("a selector is required"))?;
    let namespace = dep.namespace().to_owned();
    // The raw seam (`$*` never reaches the session, RFC 03 §2), and the
    // one-line hint for a base-relative `zk2/…` typed under a namespace.
    let selector = zk2::wire_selector(Some(&selector), &namespace)?;
    let payload = body.as_ref().map(crate::input::Source::read).transpose()?;

    // The lens before the GET: presence and the revisions it names, read in
    // the namespace — unless nothing is to be decoded, which asks nothing
    // of it.
    let store = zk2::store(&dep, &contracts);
    let structural_only = raw || no_decode;
    let catalog = if structural_only {
        None
    } else {
        let ns_session = dep.session().await?;
        zk2::read_lens(&dep, &ns_session, &store).await
    };
    let lens = Lens::new(&namespace, catalog.as_ref(), &store).offline(&contracts);

    // The GET itself, on a session in no namespace: the wire key as typed.
    let session = dep.link().session().await?;
    // Named, because the reply bound's *cost* rides on the options that state
    // it (#339): a GET that read only some of the replies must say so, or the
    // rendering claims a fan-in it did not have (RFC 13 §3 O6).
    let opts = zenkey_fleet::GetOpts::new(dep.timeout()).payload(payload);
    let answers = zenkey_fleet::fleet_get(&session, &selector, &opts).await?;
    let elided = opts.elided();

    let secs = dep.timeout().as_secs_f64();
    // A fan-in GET *looks* like a stream and is not: it waits for the window,
    // then has every answer in hand. So it is a document, and the one place
    // that decides which format to print it in is `emit` (#198).
    if crate::render::Mode::of(dep.format()).machine() {
        let rows = answers
            .iter()
            .map(|a| row(a, &lens, raw, structural_only))
            .collect();
        let report = crate::render::GetReport {
            selector: selector.clone(),
            timeout_s: secs,
            elided,
            answers: rows,
        };
        crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    } else {
        let mut n = 0usize;
        for a in &answers {
            let payload = match &a.answer {
                Answer::Error { name, message } => {
                    // An error reply has no sample, so no key to name.
                    println!("✗ {name} — {message}");
                    continue;
                }
                Answer::Value(payload) => payload,
            };
            n += 1;
            let bytes = payload.to_bytes().into_owned();
            if raw {
                println!("{}\n  {}", a.key, hex(&bytes));
                if let Some(att) = &a.attachment {
                    println!("  attachment: {}", hex(&att.to_bytes()));
                }
                continue;
            }
            let line = line_of(a, bytes, &lens, structural_only);
            if hex_payload {
                println!("{}\n  {} {}", a.key, line.tag(), hex(&line.bytes));
                if let Some(att) = &a.attachment {
                    println!("  attachment: {}", hex(&att.to_bytes()));
                }
                continue;
            }
            if let Some(fmt) = fmt.as_deref() {
                let value = line.value_text();
                let declared = line.declared();
                let stamp = a.timestamp.map(|t| t.to_string());
                let attachment = line
                    .attachment
                    .as_ref()
                    .map(crate::render::payload_text)
                    .or_else(|| {
                        a.attachment
                            .as_ref()
                            .map(|t| attachment_display(&t.to_bytes()))
                    });
                println!(
                    "{}",
                    format_sample(
                        fmt,
                        &SampleLine {
                            n,
                            wire_key: &a.key,
                            relative: zenkey_fleet::strip_namespace(&namespace, &a.key),
                            identity: &line.identity,
                            type_name: declared.as_deref(),
                            encoding: a.encoding.as_deref().unwrap_or(""),
                            payload_len: line.bytes.len(),
                            // A reply is not a subscribe-path sample: no QoS
                            // axes and no SourceInfo, and an empty field is
                            // honest (#120). The HLC is the one thing a reply
                            // may carry — only when the responder stamped it
                            // (#215).
                            timestamp: stamp.as_deref(),
                            value: &value,
                            attachment: attachment.as_deref(),
                            qos: None,
                            source: None,
                        },
                    )
                );
                continue;
            }
            print(a, &line);
        }
        match answers.len() {
            0 => println!(
                "no replies within {secs}s. Silence is not a verdict (RFC 05 §3.1): \
                 nothing may hold the selector, its holders may be down, or the \
                 timeout too short — `zenctl admin graph` says who is attached."
            ),
            len => eprintln!(
                "{len} repl{} within {secs}s — an observation, not totality \
                 (RFC 05 §2.1)",
                if len == 1 { "y" } else { "ies" }
            ),
        }
        if elided > 0 {
            eprintln!(
                "{elided} further repl(y|ies) arrived and were not read — the \
                 reply bound bit, so the above is a sample of the answers"
            );
        }
    }
    // Exit-code discipline shared with `call`: 1 = an error reply, 2 = zero
    // replies (silence stays a distinct non-verdict — RFC 05 §3.1).
    let code = exit_code(&answers);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// One reply through the lens: a response on an `@op` key, its resource's
/// type otherwise.
fn line_of(a: &FleetAnswer, bytes: Vec<u8>, lens: &Lens<'_>, structural_only: bool) -> Line {
    let member = Line::member_of(&lens.identity(&a.key));
    let attachment = a.attachment.as_ref().map(|t| t.to_bytes());
    Line::of(
        Payload {
            key: &a.key,
            encoding: a.encoding.as_deref().filter(|e| !e.is_empty()),
            bytes,
            attachment: attachment.as_deref(),
            delete: false,
        },
        member,
        lens,
        structural_only,
    )
}

/// The table form: the reply's key, then the rendering with the rung it
/// stopped at, and what failed against the declared type on stderr.
fn print(a: &FleetAnswer, line: &Line) {
    let Some((rendering, conformance)) = &line.checked else {
        return;
    };
    println!("{}\n  {}", a.key, crate::render::payload_text(rendering));
    if let Some(att) = &line.attachment {
        println!("  attachment: {}", crate::render::payload_text(att));
    } else if let Some(att) = &a.attachment {
        println!("  attachment: {}", attachment_display(&att.to_bytes()));
    }
    if let Conformance::Invalid { violations } = conformance {
        for v in violations {
            eprintln!("  invalid: {v}");
        }
    }
}

/// One reply as a JSON row — the ndjson line and the json array element.
fn row(a: &FleetAnswer, lens: &Lens<'_>, raw: bool, structural_only: bool) -> serde_json::Value {
    match &a.answer {
        Answer::Error { name, message } => serde_json::json!({
            "error": { "name": name, "message": message },
        }),
        Answer::Value(payload) => {
            let bytes = payload.to_bytes().into_owned();
            if raw {
                let mut obj = serde_json::json!({
                    "key": a.key,
                    "encoding": a.encoding,
                    "hex": hex(&bytes),
                });
                if let Some(att) = &a.attachment {
                    obj["attachment"] = serde_json::Value::String(hex(&att.to_bytes()));
                    obj["attachment_bytes"] = att.len().into();
                }
                return obj;
            }
            value_row(a, &line_of(a, bytes, lens, structural_only))
        }
    }
}

/// The decoded half of `row`, pure so the verdict terms are testable: the
/// row dialect `echo --format ndjson` writes (so `pub --from ndjson` reads
/// it back), with the reply's HLC when the responder stamped one.
fn value_row(a: &FleetAnswer, line: &Line) -> serde_json::Value {
    let mut row = zenkey_fleet::SampleRow::of_key(&a.key);
    row.encoding = a.encoding.clone().filter(|e| !e.is_empty());
    row.timestamp = a.timestamp.map(|t| t.to_string());
    let attachment = a.attachment.as_ref().map(|t| t.to_bytes());
    line.fill(&mut row, attachment.as_deref());
    if let (None, Some(att)) = (&row.attachment, attachment.as_deref()) {
        row.attachment = Some(attachment_json(att));
    }
    serde_json::to_value(&row).expect("a sample row serializes")
}

/// `get state <address> <iface>[@fp] <state> [--last-known <archive>]`:
/// the owner's current state (S4), or an archive's last-known state (S5),
/// through the contract. Exit 2 on silence: never "no value" (S6).
pub async fn state(cli: crate::cli::StateGetArgs) -> Result<()> {
    use crate::cmd::zk2;
    let dep = crate::bus::Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let target = zenkey_fleet::ResolvedTarget::parse(&cli.address.to_string())?;
    let values = zk2::bindings(&cli.params);
    let mut session = None;
    let revision =
        zk2::revision_at(&dep, &contracts, &cli.target, Some(&target), &mut session).await?;
    let r = zenkey_fleet::resolve_resource(&revision, &cli.resource, &[Kind::State])?;
    zenkey_fleet::check_values(r, &values)?;
    let session = match session {
        Some(s) => s,
        None => dep.session().await?,
    };
    let read = zenkey_fleet::StateRead {
        revision: &revision,
        owner: &cli.address,
        resource: r,
        values: &values,
        timeout: dep.timeout(),
    };
    let report = match &cli.last_known {
        None => zenkey_fleet::get_state(&session, read).await?,
        Some(archive) => zenkey_fleet::last_known_state(&session, read, archive).await?,
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    if report.rows.is_empty() {
        std::process::exit(crate::exit::NO_VERDICT);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(key: &str) -> FleetAnswer {
        FleetAnswer {
            key: key.into(),
            encoding: None,
            attachment: None,
            timestamp: None,
            answer: Answer::Value(zenoh::bytes::ZBytes::from("1")),
        }
    }

    fn error() -> FleetAnswer {
        FleetAnswer {
            key: String::new(),
            encoding: None,
            attachment: None,
            timestamp: None,
            answer: Answer::Error {
                name: "error/unavailable".into(),
                message: "busy".into(),
            },
        }
    }

    /// 0 = values only, 1 = any error envelope, 2 = silence — the same
    /// discipline as `call`'s, on raw answers.
    #[test]
    fn get_exit_codes_follow_the_reply_discipline() {
        assert_eq!(exit_code(&[]), 2);
        assert_eq!(exit_code(&[value("a"), value("b")]), 0);
        assert_eq!(exit_code(&[value("a"), error()]), 1);
    }

    /// A reply row is the row dialect: its key, the identity the lens made
    /// of it, and the conformance verdict — a key no contract reaches
    /// renders structurally and says why, never refused, and `--no-decode`
    /// says it did not ask (#246's terms, kept: the verdict is present
    /// whenever a row carries a value).
    #[test]
    fn a_get_row_carries_the_identity_and_the_verdict() {
        let contracts = zenkey_fleet::ContractSet::new();
        let lens = Lens::new("prod", None, &contracts);

        let foreign = value("plant/line-1/temp");
        let r = row(&foreign, &lens, false, false);
        assert_eq!(r["key"], "plant/line-1/temp");
        assert_eq!(r["identity"]["is"], "not_in_namespace");
        assert_eq!(r["value"], 1);
        assert!(
            r["verdict"].as_str().unwrap().starts_with("not-checked"),
            "{r}"
        );
        assert!(r.get("origin").is_none(), "v1's origin left at FJ9");

        let zk2 = value("prod/zk2/host-a/tc/tc.netif.v1/state/namespaces");
        let r = row(&zk2, &lens, false, false);
        assert_eq!(r["identity"]["is"], "resource");
        assert_eq!(r["identity"]["address"], "host-a/tc");

        let skipped = row(&zk2, &lens, false, true);
        assert!(
            skipped["verdict"]
                .as_str()
                .unwrap()
                .contains("decode not asked")
                || skipped["verdict"]
                    .as_str()
                    .unwrap()
                    .starts_with("not-checked"),
            "{skipped}"
        );

        let raw = row(&zk2, &lens, true, true);
        assert_eq!(raw["hex"], "31");
        assert!(raw.get("identity").is_none(), "--raw resolves nothing");

        let err = row(&error(), &lens, false, false);
        assert_eq!(err["error"]["name"], "error/unavailable");
    }
}
