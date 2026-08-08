//! Output rendering (issue #12): one report, three renderings.
//!
//! `table` keeps the human-first prose the tool always had; `json` is the
//! whole report as stable serde; `ndjson` is one JSON object per row for
//! stream processing (`| jq`). `auto` = table on a tty, ndjson when piped.

use anyhow::Result;
use serde::Serialize;

use crate::report::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Auto,
    Table,
    Json,
    Ndjson,
}

impl Format {
    /// Resolve `auto` against the output stream.
    pub fn resolved(self) -> Format {
        match self {
            Format::Auto => {
                if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                    Format::Table
                } else {
                    Format::Ndjson
                }
            }
            other => other,
        }
    }
}

fn json_line<T: Serialize>(row: &T) {
    println!("{}", serde_json::to_string(row).expect("report serializes"));
}

fn json_doc<T: Serialize>(report: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(report).expect("report serializes")
    );
}

pub fn topic_list(report: &TopicList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.subjects.iter().for_each(json_line),
        _ => {
            if report.subjects.is_empty() {
                println!("no subjects match.");
                return Ok(());
            }
            let mut current = None;
            let mut open_ended = 0usize;
            for s in &report.subjects {
                if current != Some(&s.producer) {
                    println!("\n{}  (registry {})", s.producer, s.registry_version);
                    current = Some(&s.producer);
                }
                if s.open_ended {
                    open_ended += 1;
                }
                println!(
                    "  {:<10} {:<44} {}{}",
                    s.class,
                    s.path,
                    s.type_name,
                    if s.open_ended { "  [open-ended]" } else { "" }
                );
            }
            println!("\n{} registered subject(s).", report.subjects.len());
            if open_ended > 0 {
                let is = if open_ended == 1 { "is" } else { "are" };
                println!(
                    "{open_ended} {is} open-ended ({{var...}}): the registry fixes their shape, not their\n\
                     members. Use `zenctl topic echo` to see what a live fleet actually publishes."
                );
            }
        }
    }
    Ok(())
}

pub fn topic_info(report: &TopicInfo, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json | Format::Ndjson => {
            json_doc(report);
            Ok(())
        }
        _ => {
            println!("key       {}", report.key);
            println!("verdict   {:?}", report.verdict);
            if !report.note.is_empty() {
                println!("          {}", report.note);
            }
            if let Some(origin) = &report.origin {
                println!("origin    {origin}");
            }
            if let Some(producer) = &report.producer {
                println!("producer  {producer}");
            }
            if let Some(class) = &report.class {
                println!("class     {class}");
            }
            if let Some(subject) = &report.subject {
                println!("subject   {subject}");
            }
            if !report.variables.is_empty() {
                println!("variables");
                for (name, value) in &report.variables {
                    println!("  {name} = {value}");
                }
            }
            if let Some(payload) = &report.payload_type {
                println!("payload   {payload}");
                println!("  (schema lives with the owning application — RFC 08 §5)");
            }
            if let Some(unit) = &report.unit {
                println!("unit      {unit}");
            }
            if let Some(qos) = &report.qos {
                println!("qos       {qos}");
            }
            if let Some(ttl) = report.ttl_s {
                println!(
                    "ttl       {ttl}s  (refresh <= {}s; stale after {ttl}s)",
                    ttl / 2
                );
            }
            if let Some(rate) = &report.rate {
                println!("rate      {rate}");
            }
            if let Some(cardinality) = report.cardinality {
                println!("cardinality  {cardinality}");
            }
            if let Some(encoding) = &report.encoding {
                println!("encoding  {encoding}");
            }
            if let Some(since) = &report.since {
                println!("since     {since}");
            }
            if let Some(description) = &report.description {
                println!("about     {description}");
            }
            Ok(())
        }
    }
}
pub fn service_list(report: &ServiceList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.procedures.iter().for_each(json_line),
        _ => {
            if report.procedures.is_empty() {
                println!("no procedures match.");
                return Ok(());
            }
            let mut current = None;
            for p in &report.procedures {
                if current != Some(&p.producer) {
                    println!("\n{}  (registry {})", p.producer, p.registry_version);
                    current = Some(&p.producer);
                }
                println!(
                    "  {:<6} {:<24} → {}",
                    p.kind,
                    p.path,
                    p.reply.as_deref().unwrap_or("-")
                );
            }
            println!("\n{} registered procedure(s).", report.procedures.len());
            println!("call one with: zenctl service call <origin|*> <producer> <procedure>");
        }
    }
    Ok(())
}

pub fn interface_list(report: &InterfaceList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.types.iter().for_each(json_line),
        _ => {
            if report.types.is_empty() {
                println!("no payload types declared.");
                return Ok(());
            }
            println!("declared payload types:\n");
            for t in &report.types {
                println!("  {:<24} {} carrier(s)", t.name, t.carriers);
            }
            println!(
                "\n{} type(s). Schema definitions live with the owning application (RFC 08 §5).",
                report.types.len()
            );
        }
    }
    Ok(())
}

pub fn interface_show(report: &InterfaceShow, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json | Format::Ndjson => json_doc(report),
        _ => {
            println!("type      {}", report.type_name);
            println!("          (schema lives with the owning application — RFC 08 §5)");
            println!(
                "\ncarried by {} subject(s)/procedure(s):",
                report.carriers.len()
            );
            for c in report.carriers.iter().take(20) {
                println!("  {:<10} {:<10} {}", c.producer, c.class, c.path);
            }
            if report.carriers.len() > 20 {
                println!("  … and {} more", report.carriers.len() - 20);
            }
        }
    }
    Ok(())
}

pub fn node_list(report: &NodeList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json | Format::Ndjson => json_doc(report),
        _ => {
            if report.origins.is_empty() {
                println!("no live producers.");
                return Ok(());
            }
            for (origin, producers) in &report.origins {
                println!("{origin}");
                for p in producers {
                    println!("  {p}");
                }
            }
            let total: usize = report.origins.values().map(Vec::len).sum();
            println!(
                "\n{total} producer(s) on {} origin(s).",
                report.origins.len()
            );
        }
    }
    Ok(())
}

pub fn base_list(report: &BaseList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.bases.iter().for_each(json_line),
        _ => {
            if report.bases.is_empty() {
                println!(
                    "no bases discovered — nothing held a liveliness token matching \
                     **/v1/*/state/*/alive and no storage config names one.\n\
                     Silence is not a verdict (RFC 05 §3.1): producers may be down, the \
                     mesh unreachable (--connect?), or holding no tokens."
                );
                return Ok(());
            }
            let mut saw_empty = false;
            for b in &report.bases {
                let display = if b.base.is_empty() {
                    saw_empty = true;
                    "(empty)"
                } else {
                    b.base.as_str()
                };
                print!(
                    "{display:<24} {:>3} origin(s)  {:>3} producer(s)",
                    b.origins.len(),
                    b.producers.len()
                );
                if !b.storages.is_empty() {
                    print!("  storage: {}", b.storages.join(", "));
                }
                if b.origins.is_empty() {
                    print!("  (storage config only — nothing alive)");
                }
                println!();
            }
            println!(
                "\n{} base(s). Pin one: zenctl context create <name> --base <base> \
                 -c <endpoint> --select",
                report.bases.len()
            );
            if saw_empty {
                println!(
                    "the (empty) base is selected with --base \"\" (keys start at v1/ \
                     on the wire)."
                );
            }
        }
    }
    Ok(())
}

pub fn call(report: &CallReport, format: Format, render_text: impl Fn(&CallAnswer) -> String) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.answers.iter().for_each(json_line),
        _ => {
            eprintln!("GET {}", report.key);
            for a in &report.answers {
                if a.ok {
                    println!("{}:\n{}", a.origin, render_text(a));
                } else if let Some(e) = &a.error {
                    println!("{}: ✗ {} — {}", a.origin, e.name, e.message);
                }
            }
            match report.answers.len() {
                0 => println!(
                    "no replies. Silence is not a verdict (RFC 05 §3.1): the origin may be \
                     down, the procedure unregistered, or the timeout too short — \
                     `zenctl node list` says who is up."
                ),
                n => eprintln!("{n} repl{}", if n == 1 { "y" } else { "ies" }),
            }
        }
    }
}

pub fn storage_list(report: &StorageList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => {
            report.storages.iter().for_each(json_line);
            report.coverage.iter().for_each(json_line);
        }
        _ => {
            if report.storages.is_empty() {
                println!(
                    "no storages found in the admin space — a peer-only mesh, a router \
                     without the storage manager, or the admin space is disabled."
                );
            } else {
                println!("configured storages:\n");
                for s in &report.storages {
                    println!(
                        "  {:<16} @{}  {}",
                        s.name,
                        s.zid,
                        s.key_expr.as_deref().unwrap_or("-")
                    );
                }
            }
            if !report.coverage.is_empty() {
                println!("\ndeclared state families vs storage coverage:\n");
                for row in &report.coverage {
                    use zenkey_fleet::Coverage;
                    let (mark, detail) = match &row.coverage {
                        Coverage::Covered(s) => ("✓", format!("covered by {s}")),
                        Coverage::Partial(s) => ("~", format!("PARTIAL via {s}")),
                        Coverage::Uncovered => ("·", "uncovered".to_string()),
                    };
                    println!("  {mark} {:<10} {:<36} {}", row.producer, row.path, detail);
                }
                println!(
                    "\nnote: an uncovered ttl'd family is not automatically a defect — \
                     volatile-state seeding may ride the advanced-pub/sub cache \
                     (RFC 04 §3.5); storage is authoritative for durable data."
                );
            }
        }
    }
    Ok(())
}
