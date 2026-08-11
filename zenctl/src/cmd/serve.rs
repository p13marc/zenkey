//! `zenctl serve` — a mock queryable for the dev loop (#121).
//!
//! Deliberately no reply scripting: static bytes, prepared once through the
//! same encode ladder as `topic pub`. The incoming-query log is half the
//! feature — it doubles as a "who is querying this key" probe.

use anyhow::Result;

use crate::{BusArgs, output};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    keyexpr: &str,
    reply: &str,
    encoding: Option<&str>,
    no_validate: bool,
    raw: bool,
    complete: bool,
    count: usize,
    args: &BusArgs,
) -> Result<()> {
    let typed = match reply {
        "-" => {
            use std::io::Read as _;
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
        b => match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        },
    };

    let session = args.session().await?;
    // The reply body rides the pub encode ladder: a concrete keyexpr that
    // refines to a registered subject gets the served schema's encoding; a
    // wildcard degrades honestly to as-typed, with the note.
    let slices = if raw {
        None
    } else {
        args.slice_set().await.ok()
    };
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let key_part = keyexpr.split('?').next().unwrap_or(keyexpr);
    let prepared = zenkey_fleet::prepare_publish(
        &session,
        &store,
        slices.as_ref(),
        args.base(),
        key_part,
        encoding,
        &typed,
        super::publish::mode(raw, no_validate),
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

    let ndjson = matches!(args.format.resolved(), output::Format::Ndjson);
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
    loop {
        let view = tokio::select! {
            v = responder.next() => v,
            _ = tokio::signal::ctrl_c() => None,
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
                obj["payload"] = zenkey_fleet::decode::structural_value(&p.to_bytes())
                    .unwrap_or_else(|| {
                        serde_json::Value::String(zenkey_fleet::decode::structural(&p.to_bytes()))
                    });
                obj["payload_bytes"] = p.len().into();
            }
            if let Some(e) = &view.encoding {
                obj["encoding"] = serde_json::Value::String(e.clone());
            }
            if let Some(a) = &view.attachment {
                obj["attachment"] = super::render::attachment_json(a);
                obj["attachment_bytes"] = a.len().into();
            }
            println!("{obj}");
        } else {
            let body = match &view.payload {
                Some(p) => format!(
                    "  body: {} ({} bytes)",
                    zenkey_fleet::decode::structural(&p.to_bytes()),
                    p.len()
                ),
                None => String::new(),
            };
            println!("[{served}] {}{body}", view.selector);
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
