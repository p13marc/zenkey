//! `admin routers` — the zenoh admin space (`@/**`), the middleware's own
//! introspection. The generic admin browse moved to the first-class
//! `zenctl get` (#114); what stays here is the genuinely admin-shaped.

use anyhow::Result;

use crate::{BusArgs, output};

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
