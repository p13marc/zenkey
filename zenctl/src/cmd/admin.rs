//! `admin routers` and `admin graph` — the zenoh admin space (`@/**`), the
//! middleware's own introspection. The generic admin browse moved to the
//! first-class `zenctl get` (#114); what stays here is the genuinely
//! admin-shaped.

use anyhow::Result;

use crate::bus::{Deployment, Link};

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

/// `admin graph` — the mesh as the admin space answered it (#118), with the
/// deployment's zk2 instances joined onto it (#705), as a table, `--dot`
/// Graphviz for piping (`| dot -Tsvg`), or json/ndjson.
///
/// Two sessions, as the doctor opens them (decided 2026-10-08): the admin
/// space in no namespace, presence and descriptors in the deployment's.
pub async fn graph(cli: crate::cli::AdminGraphArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let raw = dep.link().session().await?;
    let deployment = dep.session().await?;
    let report = zenkey_fleet::topology_with_instances(
        &raw,
        &deployment,
        dep.namespace(),
        dep.timeout(),
        cli.trust_admin_space,
    )
    .await?;
    if cli.dot {
        println!("{}", zenkey_fleet::render_dot(&report));
        // The notes are what the picture cannot say — what answered, what
        // was only heard of, what attaches nothing — so they ride stderr.
        for note in
            crate::render::notes_with_bounds(&crate::render::TopologyView { report: &report })
        {
            eprintln!("{}", note.to_line());
        }
        return Ok(());
    }
    crate::render::emit_with(
        &mut std::io::stdout(),
        &crate::render::TopologyView { report: &report },
        dep.format(),
        dep.color(),
    )
}
