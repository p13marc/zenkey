//! `admin routers` — the zenoh admin space (`@/**`), the middleware's own
//! introspection. The generic admin browse moved to the first-class
//! `zenctl get` (#114); what stays here is the genuinely admin-shaped.

use anyhow::Result;

use crate::bus::Link;

pub async fn routers(args: &Link) -> Result<()> {
    let session = args.session().await?;
    let routers = zenkey_fleet::routers(&session, args.timeout()).await?;
    // `[]` on its own cannot tell a peer-only mesh from an admin space that is
    // disabled, and those are different facts about the deployment (#236). The
    // selector rides with the answer so the coverage claim is exactly what was
    // asked, and no wider.
    let report = zenkey_fleet::report::RouterList {
        asked: "@/*/router".to_string(),
        routers,
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

/// `admin graph` — the mesh as the admin space answered it (#118), as a
/// table, `--dot` Graphviz for piping (`| dot -Tsvg`), or json/ndjson.
pub async fn graph(cli: crate::cli::AdminGraphArgs) -> Result<()> {
    let link = Link::resolve(&cli.session)?;
    let session = link.session().await?;
    let report = zenkey_fleet::topology(&session, link.timeout()).await?;
    if cli.dot {
        println!("{}", zenkey_fleet::render_dot(&report));
        honesty(&report);
        return Ok(());
    }
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::TopologyView { report: &report },
        link.format(),
        link.color(),
    )
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
