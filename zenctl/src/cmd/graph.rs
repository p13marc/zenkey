//! `zenctl graph` (#612, FJ4): the data-flow graph, read from descriptors
//! and interface tokens only (spec §3.2 R3) — the edges the runtime itself
//! computes (`zenkey::presence::edges`), joined with each role's interface.

use anyhow::Result;
use zenkey_fleet::PresenceScope;

use crate::bus::Deployment;
use crate::cmd::zk2;

/// `graph [--dot]`.
pub async fn run(cli: crate::cli::GraphArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let session = dep.session().await?;
    let catalog = zk2::presence(&dep, &session, &PresenceScope::all()).await?;
    let graph = catalog.graph();
    if cli.dot {
        // A foreign schema: Graphviz's, not one of zenctl's renderings
        // (#243). Completeness rides the graph's own label.
        print!("{}", crate::render::graph_dot(&graph));
        return Ok(());
    }
    crate::render::emit_with(&mut std::io::stdout(), &graph, dep.format(), dep.color())
}
