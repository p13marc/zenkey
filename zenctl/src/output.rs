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
                let deprecation = if s.deprecated {
                    format!(
                        "  DEPRECATED{}{}",
                        s.deprecated_since
                            .as_deref()
                            .map(|v| format!(" since {v}"))
                            .unwrap_or_default(),
                        s.replaced_by
                            .as_deref()
                            .map(|r| format!(" → {r}"))
                            .unwrap_or_default()
                    )
                } else {
                    String::new()
                };
                println!(
                    "  {:<10} {:<44} {}{}{}",
                    s.class,
                    s.path,
                    s.type_name,
                    if s.open_ended { "  [open-ended]" } else { "" },
                    deprecation
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
                // Since RFC 08 §7 the shape is served, so point at it rather
                // than at the old "lives with the application" dead end.
                println!("  (`zenctl interface show {payload} --schema` for the served shape)");
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
            if !report.schemas.is_empty() {
                println!("\nserved schema (RFC 08 §7):");
                for s in &report.schemas {
                    println!("  {:<10} {:<12} {}", s.producer, s.kind, s.hash);
                }
                // Same name, different hash across producers: §7 says this is
                // a finding, and the type's own page is where it is worth
                // seeing.
                let first = &report.schemas[0].hash;
                if report.schemas.iter().any(|s| &s.hash != first) {
                    println!(
                        "\n  ⚠ producers disagree about {}'s shape — a schema-drift finding \
                         (RFC 08 §7); `zenctl doctor` carries it as one",
                        report.type_name
                    );
                }
                for s in &report.schemas {
                    if let Some(doc) = &s.document {
                        println!("\n  {} says:", s.producer);
                        println!("{}", serde_json::to_string_pretty(doc).unwrap_or_default());
                    }
                }
            }
        }
    }
    Ok(())
}

/// `zenctl schema <producer>` (issue #51).
pub fn schema_dump(report: &SchemaDump, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.types.iter().for_each(json_line),
        _ => {
            if !report.served {
                // §7 is a SHOULD. "Serves no describe" is a fact about the
                // producer, not a verdict about its payloads (RFC 05 §3.1's
                // rule applied to a schema fetch).
                println!(
                    "{} serves no `describe` — its payload shapes are undescribed, which is \
                     not the same as having none (RFC 08 §7 is a SHOULD).",
                    report.producer
                );
                return Ok(());
            }
            println!(
                "producer  {}{}",
                report.producer,
                report
                    .app
                    .as_deref()
                    .map(|a| format!("   (app {a})"))
                    .unwrap_or_default()
            );
            if report.types.is_empty() {
                println!("\nthe served set declares no matching types.");
            } else {
                println!("\n{} type(s):\n", report.types.len());
                for t in &report.types {
                    println!("  {:<24} {:<12} {}", t.type_name, t.kind, t.hash);
                }
            }
            for t in &report.types {
                if let Some(doc) = &t.document {
                    println!("\n{}:", t.type_name);
                    println!("{}", serde_json::to_string_pretty(doc).unwrap_or_default());
                }
            }
            if !report.missing.is_empty() {
                // RFC 08 §7's totality clause, checked where the user is
                // already looking rather than only in `doctor`.
                println!(
                    "\n⚠ the registry references {} type(s) this set does not cover: {}\n  \
                     (RFC 08 §7 totality — the set MUST cover every referenced name)",
                    report.missing.len(),
                    report.missing.join(", ")
                );
            }
        }
    }
    Ok(())
}

pub fn node_list(report: &NodeList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.nodes.iter().for_each(json_line),
        _ => {
            if report.nodes.is_empty() {
                println!("no live producers.");
                return Ok(());
            }
            let mut last_origin = "";
            for row in &report.nodes {
                if row.origin != last_origin {
                    println!("{}", row.origin);
                    last_origin = &row.origin;
                }
                match (&row.app, &row.registry_version) {
                    (Some(app), Some(v)) => {
                        println!("  {}  (app {app}, registry {v})", row.producer)
                    }
                    // Asked and unanswered is a fact; not asked is not (O4).
                    _ if report.slices_joined => {
                        println!("  {}  (no served slice)", row.producer)
                    }
                    _ => println!("  {}", row.producer),
                }
            }
            let origins: std::collections::BTreeSet<&str> =
                report.nodes.iter().map(|r| r.origin.as_str()).collect();
            println!(
                "\n{} producer(s) on {} origin(s).",
                report.nodes.len(),
                origins.len()
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

/// `topic hz` / `topic bw` (issue #46): the typed report renders through the
/// global `--format` like every other command; the O6 eviction note prints in
/// every mode.
pub fn rate(report: &RateReport, format: Format, bandwidth: bool, loss: bool) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => {
            report.rows.iter().for_each(json_line);
            // The totals ride a final object so a stream consumer gets them
            // (and the eviction honesty) without re-summing.
            json_line(&serde_json::json!({
                "selector": report.selector,
                "window_s": report.window_s,
                "total_count": report.total_count,
                "total_bytes": report.total_bytes,
                "keys": report.keys,
                "evicted": report.evicted,
                "max_keys": report.max_keys,
                "sn_gaps": report.sn_gaps,
            }));
        }
        _ => {
            let secs = report.window_s as f64;
            for row in &report.rows {
                if bandwidth {
                    println!("{:>12.1} B/s  {}", row.bytes as f64 / secs, row.key);
                } else {
                    print!("{:>8.2} Hz  {}", row.count as f64 / secs, row.key);
                    if loss {
                        print!("  ({} sn gap(s))", row.sn_gaps);
                    }
                    println!();
                }
            }
            if report.evicted > 0 {
                // The table is bounded; a shrunken key set must say so (O6).
                eprintln!(
                    "note: {} key(s) retired to stay within the {}-key bound — totals \
                     cover the retained set",
                    report.evicted, report.max_keys
                );
            }
            if bandwidth {
                println!(
                    "total: {:.1} B/s over {} key(s) ({} bytes / {}s)",
                    report.total_bytes as f64 / secs,
                    report.keys,
                    report.total_bytes,
                    report.window_s
                );
            } else {
                print!(
                    "total: {:.2} Hz over {} key(s) ({} samples / {}s)",
                    report.total_count as f64 / secs,
                    report.keys,
                    report.total_count,
                    report.window_s
                );
                if let Some(gaps) = report.sn_gaps {
                    print!(
                        "  — {gaps} source-sn gap(s) (zero also means \"publishers attach \
                         no SourceInfo\")"
                    );
                }
                println!();
            }
        }
    }
}

/// `doctor` (issue #46): findings as a table (severity mark, check id,
/// subject, evidence, citation) or as the whole typed report in JSON —
/// `zenctl doctor --format json | jq '.findings[]'` is the contract.
pub fn doctor(report: &DoctorReport, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => {
            report.findings.iter().for_each(json_line);
        }
        _ => {
            for s in &report.synced {
                println!("✓ {s}: in sync");
            }
            for f in &report.findings {
                let mark = match f.severity {
                    DoctorSeverity::Error => "✗",
                    DoctorSeverity::Warning => "⚠",
                    DoctorSeverity::Info => "·",
                };
                let citation = f
                    .citation
                    .as_deref()
                    .map(|c| format!("  [{c}]"))
                    .unwrap_or_default();
                println!(
                    "{mark} {}: {} — {}{citation}",
                    f.check, f.subject, f.evidence
                );
            }
            println!(
                "\n{} introspect repl(y|ies) from {} live producer(s); {} producer(s) \
                 serve describe, {} do not; {} router(s){}.",
                report.introspect_answered,
                report.live_producers,
                report.describe_served,
                report.describe_missing,
                report.routers,
                report
                    .router_version
                    .as_deref()
                    .map(|v| format!(" (version {v})"))
                    .unwrap_or_default(),
            );
            let errors = report.count(DoctorSeverity::Error);
            let warnings = report.count(DoctorSeverity::Warning);
            if errors == 0 && warnings == 0 {
                println!("no findings — the fleet agrees with this build.");
            } else {
                println!(
                    "{} finding(s): {errors} error(s), {warnings} warning(s), {} info.",
                    report.findings.len(),
                    report.count(DoctorSeverity::Info)
                );
            }
        }
    }
    Ok(())
}
