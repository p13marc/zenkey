//! `admin get` / `admin routers` — the zenoh admin space (`@/**`), the
//! middleware's own introspection. Key layouts vary between zenoh versions,
//! so this is a raw, honest browse.

use anyhow::Result;

use crate::{BusArgs, output};

pub async fn get(selector: &str, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let entries = zenkey_fleet::admin_get(&session, selector, args.timeout()).await?;
    match args.format.resolved() {
        output::Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(
                &entries
                    .iter()
                    .map(|e| serde_json::json!({"key": e.key, "value": e.value}))
                    .collect::<Vec<_>>()
            )?
        ),
        output::Format::Ndjson => {
            for e in &entries {
                println!("{}", serde_json::json!({"key": e.key, "value": e.value}));
            }
        }
        _ => {
            for e in &entries {
                println!("{}\n  {}", e.key, e.value);
            }
            eprintln!("{} admin entr(ies)", entries.len());
        }
    }
    Ok(())
}

pub async fn routers(args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let routers = zenkey_fleet::routers(&session, args.timeout()).await?;
    match args.format.resolved() {
        output::Format::Json => {
            println!("{}", serde_json::to_string_pretty(&routers)?)
        }
        output::Format::Ndjson => {
            for r in &routers {
                println!("{}", serde_json::to_string(r)?);
            }
        }
        _ => {
            if routers.is_empty() {
                println!(
                    "no routers answered @/*/router — a peer-only mesh, or the admin \
                     space is disabled."
                );
            }
            for r in &routers {
                println!(
                    "{}  {}  {}",
                    r.zid,
                    r.version.as_deref().unwrap_or("-"),
                    r.locators.join(", ")
                );
            }
        }
    }
    Ok(())
}
