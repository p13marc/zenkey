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

/// `admin graph` — the mesh as the admin space answered it (#118), as a
/// table, `--dot` Graphviz for piping (`| dot -Tsvg`), or json/ndjson.
pub async fn graph(dot: bool, origins: bool, args: &BusArgs) -> Result<()> {
    let session = args.session().await?;
    let report = zenkey_fleet::topology(&session, args.timeout()).await?;
    // The origin join is opt-in (#131): it costs one more admin sweep, and
    // the lazy rule holds even for pictures.
    let attachments = if origins {
        zenkey_fleet::origin_attachments(&session, args.base(), args.timeout()).await?
    } else {
        Vec::new()
    };
    if dot {
        println!("{}", zenkey_fleet::render_dot(&report, &attachments));
        honesty(&report);
        return Ok(());
    }
    match args.format.resolved() {
        output::Format::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        output::Format::Ndjson => {
            for n in &report.nodes {
                println!("{}", serde_json::to_string(n)?);
            }
            for e in &report.edges {
                println!("{}", serde_json::to_string(e)?);
            }
            for a in &attachments {
                println!("{}", serde_json::to_string(a)?);
            }
        }
        _ => {
            for n in &report.nodes {
                let you = if n.zid == report.self_zid {
                    "  ← you"
                } else {
                    ""
                };
                if n.answered {
                    println!(
                        "{}  {}  {}  {}{you}",
                        n.zid,
                        n.whatami,
                        n.version.as_deref().unwrap_or("-"),
                        n.locators.join(" "),
                    );
                } else {
                    println!("{}  {}  (heard of, not queryable){you}", n.zid, n.whatami);
                }
            }
            for a in &attachments {
                match &a.session_zid {
                    Some(z) => println!("  {}  ⚓ session {z}  (token {})", a.origin, a.token_key),
                    None => println!(
                        "  {}  reported by {} — sources named no single session; \
                         shown as reported, not attached (O4)",
                        a.origin, a.reporter_zid
                    ),
                }
            }
            for link in zenkey_fleet::mesh_links(&report) {
                println!(
                    "  {} —— {}{}{}",
                    link.a,
                    link.b,
                    if link.corroborated {
                        "  (both report it)"
                    } else {
                        ""
                    },
                    if link.links.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", link.links.join(", "))
                    }
                );
            }
            honesty(&report);
        }
    }
    Ok(())
}

fn honesty(report: &zenkey_fleet::TopologyReport) {
    match report.answered {
        0 => eprintln!(
            "no admin space answered {} — adminspace.enabled defaults off; this is a \
             reading about reachability, never an empty mesh (RFC 05 §3.1)",
            report.asked
        ),
        n => eprintln!(
            "{n} root doc(s) answered {}; {} node(s) total ({} only heard of)",
            report.asked,
            report.nodes.len(),
            report.nodes.iter().filter(|x| !x.answered).count()
        ),
    }
}
