//! `admin routers` — the zenoh admin space (`@/**`), the middleware's own
//! introspection. The generic admin browse moved to the first-class
//! `zenctl get` (#114); what stays here is the genuinely admin-shaped.

use anyhow::Result;

use crate::Bus;

pub async fn routers(args: &Bus) -> Result<()> {
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
pub async fn graph(dot: bool, origins: bool, args: &Bus) -> Result<()> {
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
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::TopologyView {
            report: &report,
            attachments: &attachments,
        },
        args.format(),
        args.color(),
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
