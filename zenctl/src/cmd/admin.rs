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
        println!("{}", render_dot(&report, &attachments));
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
            for (a, b, both, links) in deduped_edges(&report) {
                println!(
                    "  {a} —— {b}{}{}",
                    if both { "  (both report it)" } else { "" },
                    if links.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", links.join(", "))
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

/// Edges collapsed to unordered pairs for display; reciprocal reports are
/// corroboration, marked as such.
fn deduped_edges(
    report: &zenkey_fleet::TopologyReport,
) -> Vec<(String, String, bool, Vec<String>)> {
    let mut out: Vec<(String, String, bool, Vec<String>)> = Vec::new();
    for e in &report.edges {
        let (a, b) = if e.reporter <= e.peer {
            (e.reporter.clone(), e.peer.clone())
        } else {
            (e.peer.clone(), e.reporter.clone())
        };
        match out.iter_mut().find(|(x, y, _, _)| *x == a && *y == b) {
            Some(row) => row.2 = true,
            None => out.push((a, b, false, e.links.clone())),
        }
    }
    out
}

/// Graphviz, self-contained: routers as boxes, peers/clients as ellipses,
/// heard-of nodes dashed, our own session bold.
fn render_dot(
    report: &zenkey_fleet::TopologyReport,
    attachments: &[zenkey_fleet::OriginAttachment],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("graph zenoh_mesh {\n");
    for n in &report.nodes {
        let shape = if n.whatami == "router" {
            "box"
        } else {
            "ellipse"
        };
        let mut style = Vec::new();
        if !n.answered {
            style.push("dashed");
        }
        if n.zid == report.self_zid {
            style.push("bold");
        }
        let label = match (&n.answered, &n.version) {
            (false, _) => format!("{}\\n{} (heard of)", n.zid, n.whatami),
            (true, Some(v)) => format!("{}\\n{} {v}", n.zid, n.whatami),
            (true, None) => format!("{}\\n{}", n.zid, n.whatami),
        };
        let _ = writeln!(
            out,
            "  \"{}\" [shape={shape}, label=\"{label}\"{}];",
            n.zid,
            if style.is_empty() {
                String::new()
            } else {
                format!(", style=\"{}\"", style.join(","))
            }
        );
    }
    for (a, b, _, links) in deduped_edges(report) {
        let proto = links
            .first()
            .and_then(|l| l.split('/').next())
            .unwrap_or("");
        let _ = writeln!(
            out,
            "  \"{a}\" -- \"{b}\"{};",
            if proto.is_empty() {
                String::new()
            } else {
                format!(" [label=\"{proto}\"]")
            }
        );
    }
    // Origins as their own small nodes (#131): attached by a solid edge to
    // the session the admin sources named, or by a dotted one to the mere
    // reporter — the picture keeps the evidence distinction the join made.
    for (i, a) in attachments.iter().enumerate() {
        let id = format!("origin_{i}");
        let _ = writeln!(
            out,
            "  \"{id}\" [shape=hexagon, label=\"{}\", fontsize=10];",
            a.origin
        );
        match &a.session_zid {
            Some(z) => {
                let _ = writeln!(out, "  \"{id}\" -- \"{z}\";");
            }
            None => {
                let _ = writeln!(
                    out,
                    "  \"{id}\" -- \"{}\" [style=dotted, label=\"reported\"];",
                    a.reporter_zid
                );
            }
        }
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::{TopologyEdge, TopologyNode, TopologyReport};

    fn report() -> TopologyReport {
        TopologyReport {
            nodes: vec![
                TopologyNode {
                    zid: "aaa".into(),
                    whatami: "router".into(),
                    version: Some("1.9.0".into()),
                    locators: vec!["tcp/10.0.0.1:7447".into()],
                    answered: true,
                },
                TopologyNode {
                    zid: "bbb".into(),
                    whatami: "peer".into(),
                    version: None,
                    locators: vec![],
                    answered: false,
                },
            ],
            edges: vec![
                TopologyEdge {
                    reporter: "aaa".into(),
                    peer: "bbb".into(),
                    whatami: "peer".into(),
                    links: vec!["tcp/10.0.0.1:7447 -> tcp/10.0.0.2:53210".into()],
                },
                TopologyEdge {
                    reporter: "bbb".into(),
                    peer: "aaa".into(),
                    whatami: "router".into(),
                    links: vec![],
                },
            ],
            asked: "@/*/*".into(),
            answered: 1,
            self_zid: "bbb".into(),
        }
    }

    /// Reciprocal reports collapse to one undirected edge, marked as
    /// corroborated — not drawn twice, not silently merged.
    #[test]
    fn reciprocal_reports_corroborate_one_edge() {
        let edges = deduped_edges(&report());
        assert_eq!(edges.len(), 1);
        let (a, b, both, _) = &edges[0];
        assert_eq!((a.as_str(), b.as_str()), ("aaa", "bbb"));
        assert!(both, "both ends reported it");
    }

    /// The DOT form: routers boxed, heard-of nodes dashed, our session
    /// bold, edges labeled by protocol — pipeable to `dot -Tsvg` as-is.
    #[test]
    fn the_dot_form_marks_what_the_join_knows() {
        let dot = render_dot(&report(), &[]);
        assert!(dot.starts_with("graph zenoh_mesh {"), "{dot}");
        assert!(dot.contains("\"aaa\" [shape=box"), "{dot}");
        assert!(dot.contains("heard of"), "{dot}");
        assert!(dot.contains("style=\"dashed,bold\""), "{dot}");
        assert!(dot.contains("\"aaa\" -- \"bbb\" [label=\"tcp\"]"), "{dot}");
        assert!(dot.ends_with('}'), "{dot}");
    }

    /// The origin overlay (#131): a sources-named attachment is a solid
    /// edge; a reporter-only one is dotted and says "reported" — the DOT
    /// keeps the evidence distinction the join made.
    #[test]
    fn the_dot_form_keeps_the_attachment_evidence_distinction() {
        let attachments = vec![
            zenkey_fleet::OriginAttachment {
                origin: "h-cccccccccccc".into(),
                session_zid: Some("bbb".into()),
                reporter_zid: "aaa".into(),
                token_key: "v1/h-cccccccccccc/state/demo/alive".into(),
            },
            zenkey_fleet::OriginAttachment {
                origin: "h-dddddddddddd".into(),
                session_zid: None,
                reporter_zid: "aaa".into(),
                token_key: "v1/h-dddddddddddd/state/demo/alive".into(),
            },
        ];
        let dot = render_dot(&report(), &attachments);
        assert!(dot.contains("label=\"h-cccccccccccc\""), "{dot}");
        assert!(dot.contains("\"origin_0\" -- \"bbb\";"), "{dot}");
        assert!(
            dot.contains("\"origin_1\" -- \"aaa\" [style=dotted, label=\"reported\"]"),
            "{dot}"
        );
    }
}
