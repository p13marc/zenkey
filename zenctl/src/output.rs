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
        Format::Json | Format::Ndjson => json_doc(report),
        _ => {
            println!("key       {}", report.key);
            println!("origin    {}", report.origin);
            println!("producer  {}", report.producer);
            println!("class     {}", report.class);
            println!("subject   {}", report.subject);
            if !report.variables.is_empty() {
                println!("variables");
                for (name, value) in &report.variables {
                    println!("  {name} = {value}");
                }
            }
            println!("payload   {}", report.payload_type);
            println!("  (schema lives with the owning application — RFC 08 §5)");
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
            if let Some(c) = report.cardinality {
                println!("cardinality  ~{c} keys expected");
            }
            if let Some(e) = &report.encoding {
                println!("encoding  {e}");
            }
            if let Some(since) = &report.since {
                println!("since     {since}");
            }
            if let Some(desc) = &report.description {
                println!("note      {desc}");
            }
        }
    }
    Ok(())
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
