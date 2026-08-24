//! `zenctl serve` — a mock queryable for the dev loop (#121).
//!
//! Deliberately no reply scripting: static bytes, prepared once through the
//! same encode ladder as `pub`. The incoming-query log is half the
//! feature — it doubles as a "who is querying this key" probe.

use anyhow::Result;

use crate::Bus;
use crate::input::Source;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    keyexpr: &str,
    reply: &Source,
    encoding: Option<&str>,
    no_validate: bool,
    raw: bool,
    complete: bool,
    count: usize,
    args: &Bus,
) -> Result<()> {
    // RFC 05 §2.1 (G-05c): `@rpc` queryables are **never** declared
    // complete — one complete queryable short-circuits every default
    // (`BestMatching`) fleet call to a single reply. Refused before any
    // session opens, so the refusal is offline-testable like gen's guards.
    let key_part = keyexpr.split('?').next().unwrap_or(keyexpr);
    if complete && key_part.split('/').any(|c| c == "@rpc") {
        anyhow::bail!(
            "--complete on an @rpc key expression: RFC 05 §2.1 forbids it — a \
             `complete` @rpc queryable short-circuits every BestMatching fleet \
             call to this one responder, silently collapsing the fleet to a \
             single reply. Serve the procedure without --complete."
        );
    }

    let typed = reply.read()?;

    let session = args.session().await?;
    // The reply body rides the pub encode ladder: a concrete keyexpr that
    // refines to a registered subject gets the served schema's encoding; a
    // wildcard degrades honestly to as-typed, with the note.
    let slices = if raw {
        None
    } else {
        args.slices_optional().await?
    };
    let store = zenkey_fleet::model::decode::SchemaStore::new(args.base(), args.timeout());
    let key_part = keyexpr.split('?').next().unwrap_or(keyexpr);
    let prepared = zenkey_fleet::prepare_publish(
        &args.fleet(&session),
        &store,
        slices.as_ref(),
        key_part,
        zenkey_fleet::PrepareSpec {
            declared_encoding: encoding,
            body: &typed,
            mode: super::publish::mode(raw, no_validate),
        },
    )
    .await?;
    if let Some(note) = &prepared.note {
        eprintln!("note: {note}");
    }

    let responder = zenkey_fleet::declare_responder(
        &session,
        keyexpr,
        prepared.bytes.clone(),
        prepared.encoding.as_deref(),
        complete,
    )
    .await?;

    // One resolution for the whole run, and it happens in `Mode::of` (#198).
    // A streaming verb's question is only ever "is a program reading this" —
    // it has rows for one and prose for the other, and no third answer.
    let ndjson = crate::render::Mode::of(args.format()).machine();
    if !ndjson {
        eprintln!(
            "serving {keyexpr} — replying {} bytes{}{} per query (ctrl-c to stop)",
            prepared.bytes.len(),
            prepared
                .encoding
                .as_deref()
                .map(|e| format!(" as {e}"))
                .unwrap_or_default(),
            if complete { ", declared complete" } else { "" },
        );
    }

    let mut served = 0usize;
    // One listener for the whole run (#334): a fresh `ctrl_c()` per query was
    // registered only while the `select!` was parked, so a SIGINT arriving
    // while a reply was being rendered was lost — and with SIGINT's default
    // disposition already displaced by the first call, nothing ended the
    // process either.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let view = tokio::select! {
            biased;
            _ = &mut ctrl_c => None,
            v = responder.next() => v,
        };
        let Some(view) = view else { break };
        served += 1;
        if ndjson {
            let mut obj = serde_json::json!({
                "n": served,
                "selector": view.selector,
                "parameters": view.parameters,
            });
            // Present only when the query carried one — absent, never null.
            if let Some(p) = &view.payload {
                obj["payload"] = zenkey_fleet::model::decode::structural_value(&p.to_bytes())
                    .unwrap_or_else(|| {
                        serde_json::Value::String(zenkey_fleet::model::decode::structural(
                            &p.to_bytes(),
                        ))
                    });
                obj["payload_bytes"] = p.len().into();
            }
            if let Some(e) = &view.encoding {
                obj["encoding"] = serde_json::Value::String(e.clone());
            }
            if let Some(a) = &view.attachment {
                obj["attachment"] = super::sample::attachment_json(a);
                obj["attachment_bytes"] = a.len().into();
            }
            // Present only when the reply failed to send — the doc-promised
            // surfacing of the reply path's error (never silently dropped).
            if let Some(e) = &view.reply_error {
                obj["reply_error"] = serde_json::Value::String(e.clone());
            }
            println!("{}", crate::render::Row::tagged("query", obj).into_line());
        } else {
            let body = match &view.payload {
                Some(p) => format!(
                    "  body: {} ({} bytes)",
                    zenkey_fleet::model::decode::structural(&p.to_bytes()),
                    p.len()
                ),
                None => String::new(),
            };
            println!("[{served}] {}{body}", view.selector);
            if let Some(e) = &view.reply_error {
                // The ask is logged either way; a reply that never left says
                // why, or the log reads as service.
                eprintln!("  reply failed: {e}");
            }
        }
        if count > 0 && served >= count {
            break;
        }
    }
    responder.undeclare().await?;
    if !ndjson {
        eprintln!(
            "{served} quer{} served",
            if served == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}
