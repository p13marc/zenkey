//! `zenctl get` — the fleet discipline on any selector (#114).
//!
//! A plain fan-in GET: target `All`, consolidation `None`, every reply
//! attributed by its own key (RFC 05 §2.1), error envelopes rendered as
//! errors (RFC 05 §3) — through `zenkey_fleet::fleet_get`, never the
//! JSON-lossy admin browse. Payloads ride the same rendering ladder as
//! `topic echo`: served-schema decode → structural → text → hex.

use anyhow::Result;

use super::sample::{self, attachment_display, attachment_json, format_sample, hex, type_tag};
use crate::BusArgs;
use crate::input::Source;
use zenkey_fleet::{Answer, FleetAnswer};

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

#[allow(clippy::too_many_arguments)]
pub async fn run(
    selector: &str,
    body: Option<&Source>,
    raw: bool,
    hex_payload: bool,
    fmt: Option<&str>,
    no_decode: bool,
    args: &BusArgs,
) -> Result<()> {
    let base = args.base().to_string();
    let slices = if raw {
        zenkey_fleet::SliceSet::default()
    } else {
        args.slice_set().await?
    };
    let store = zenkey_fleet::decode::SchemaStore::new(&base, args.timeout());
    let session = args.session().await?;

    // Optional query body, riding the same encode ladder as `topic pub`:
    // when the selector's key part refines to a registered subject the
    // served schema encodes it; otherwise it ships as typed, with the note.
    let payload = match body {
        None => None,
        Some(b) => {
            let typed = b.read()?;
            let key_part = selector.split('?').next().unwrap_or(selector);
            let prepared = zenkey_fleet::prepare_publish(
                &session,
                &store,
                if raw { None } else { Some(&slices) },
                &base,
                key_part,
                None,
                &typed,
                super::publish::mode(raw, false),
            )
            .await?;
            if let Some(note) = &prepared.note {
                eprintln!("note: {note}");
            }
            if let zenkey_fleet::BodySource::Encoded { type_name } = &prepared.source {
                eprintln!(
                    "encoded {} bytes as {type_name} → {}",
                    prepared.bytes.len(),
                    prepared.encoding.as_deref().unwrap_or("(no encoding set)")
                );
            }
            Some(prepared.bytes)
        }
    };

    let answers =
        zenkey_fleet::fleet_get(&session, &base, selector, payload, args.timeout()).await?;

    let secs = args.timeout().as_secs();
    // A fan-in GET *looks* like a stream and is not: it waits for the window,
    // then has every answer in hand. So it is a document, and the one place
    // that decides which format to print it in is `emit` (#198). The decode
    // that builds a row is async, which is why the rows are built here beside
    // the session and rendered there.
    if crate::render::Mode::of(args.format()).machine() {
        let mut rows = Vec::with_capacity(answers.len());
        for a in &answers {
            rows.push(row(a, &store, &session, &slices, &base, raw, no_decode).await);
        }
        let report = crate::render::GetReport {
            selector: selector.to_string(),
            timeout_s: secs,
            answers: rows,
        };
        crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    } else {
        {
            let mut n = 0usize;
            for a in &answers {
                match &a.answer {
                    Answer::Error { name, message } => {
                        println!("{}: ✗ {name} — {message}", a.origin);
                    }
                    Answer::Value(payload) => {
                        n += 1;
                        let bytes = payload.to_bytes();
                        let encoding = a.encoding.as_deref();
                        if raw {
                            println!("{}\n  [{}] {}", a.key, a.origin, hex(&bytes));
                            if let Some(att) = &a.attachment {
                                println!("  attachment: {}", hex(&att.to_bytes()));
                            }
                            continue;
                        }
                        let d = sample::decode(
                            &store, &session, &slices, &base, &a.key, encoding, &bytes, no_decode,
                        )
                        .await;
                        let type_name = d.type_name;
                        if hex_payload {
                            // The tag names the type; bytes at the user's
                            // request are not a failed decode — no `?`.
                            let tag = type_tag(type_name.as_deref(), true);
                            println!("{}\n  [{}] {tag} {}", a.key, a.origin, hex(&bytes));
                            if let Some(att) = &a.attachment {
                                println!("  attachment: {}", hex(&att.to_bytes()));
                            }
                            continue;
                        }
                        let v = sample::value_of(&d.rendering);
                        if let Some(fmt) = fmt {
                            println!(
                                "{}",
                                format_sample(
                                    fmt,
                                    n,
                                    &a.key,
                                    &base,
                                    type_name.as_deref(),
                                    encoding.unwrap_or(""),
                                    bytes.len(),
                                    None,
                                    &v.text,
                                    a.attachment.as_ref().map(attachment_display).as_deref(),
                                    // A reply is not a subscribe-path sample:
                                    // FleetAnswer carries no QoS axes, and an
                                    // empty field is honest (#120).
                                    None,
                                    None,
                                )
                            );
                        } else {
                            let tag = type_tag(type_name.as_deref(), v.typed);
                            println!("{}\n  [{}] {tag} {}", a.key, a.origin, v.text);
                            if let Some(att) = &a.attachment {
                                println!("  attachment: {}", attachment_display(att));
                            }
                            for note in v.notes {
                                eprintln!("  note: {note}");
                            }
                        }
                    }
                }
            }
            match answers.len() {
                0 => println!(
                    "no replies within {secs}s. Silence is not a verdict (RFC 05 §3.1): \
                     nothing may hold the selector, its holders may be down, or the \
                     timeout too short — `zenctl node list` says who is up."
                ),
                len => eprintln!(
                    "{len} repl{} within {secs}s — an observation, not totality \
                     (RFC 05 §2.1)",
                    if len == 1 { "y" } else { "ies" }
                ),
            }
        }
    }
    // Exit-code discipline shared with `service call`: 1 = an error reply,
    // 2 = zero replies (silence stays a distinct non-verdict — RFC 05 §3.1).
    let code = exit_code(&answers);
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// One reply as a JSON row — the ndjson line and the json array element.
async fn row(
    a: &FleetAnswer,
    store: &zenkey_fleet::decode::SchemaStore,
    session: &zenoh::Session,
    slices: &zenkey_fleet::SliceSet,
    base: &str,
    raw: bool,
    no_decode: bool,
) -> serde_json::Value {
    match &a.answer {
        Answer::Error { name, message } => serde_json::json!({
            "origin": a.origin,
            "error": { "name": name, "message": message },
        }),
        Answer::Value(payload) => {
            let bytes = payload.to_bytes();
            if raw {
                let mut obj = serde_json::json!({
                    "key": a.key,
                    "origin": a.origin,
                    "encoding": a.encoding,
                    "hex": hex(&bytes),
                });
                if let Some(att) = &a.attachment {
                    obj["attachment"] = serde_json::Value::String(hex(&att.to_bytes()));
                    obj["attachment_bytes"] = att.len().into();
                }
                return obj;
            }
            let d = sample::decode(
                store,
                session,
                slices,
                base,
                &a.key,
                a.encoding.as_deref(),
                &bytes,
                no_decode,
            )
            .await;
            let v = sample::value_of(&d.rendering);
            let mut obj = serde_json::json!({
                "key": a.key,
                "origin": a.origin,
                "type": d.type_name,
                "typed": v.typed,
                "encoding": a.encoding,
                "value": serde_json::from_str::<serde_json::Value>(&v.text)
                    .unwrap_or(serde_json::Value::String(v.text)),
            });
            // Present only when the wire carried one (#117).
            if let Some(att) = &a.attachment {
                obj["attachment"] = attachment_json(att);
                obj["attachment_bytes"] = att.len().into();
            }
            obj
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(origin: &str) -> FleetAnswer {
        FleetAnswer {
            origin: origin.into(),
            key: format!("v1/{origin}/state/x/y"),
            encoding: None,
            attachment: None,
            answer: Answer::Value(zenoh::bytes::ZBytes::from("1")),
        }
    }

    fn error(origin: &str) -> FleetAnswer {
        FleetAnswer {
            origin: origin.into(),
            key: String::new(),
            encoding: None,
            attachment: None,
            answer: Answer::Error {
                name: "error/unavailable".into(),
                message: "busy".into(),
            },
        }
    }

    /// 0 = values only, 1 = any error envelope, 2 = silence — the same
    /// discipline as `CallReport::exit_code`, on raw answers.
    #[test]
    fn get_exit_codes_follow_the_reply_discipline() {
        assert_eq!(exit_code(&[]), 2);
        assert_eq!(exit_code(&[value("a"), value("b")]), 0);
        assert_eq!(exit_code(&[value("a"), error("b")]), 1);
    }
}
