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

/// `zenctl bench rpc` (issue #52).
pub fn bench(report: &BenchReport, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.origins.iter().for_each(json_line),
        _ => {
            println!("→ {}", report.key);
            println!(
                "{} call(s), concurrency {}, {:.2}s — {:.1} calls/s",
                report.completed, report.concurrency, report.elapsed_s, report.calls_per_s
            );
            if report.completed < report.requested {
                println!(
                    "  {} of {} calls did not complete",
                    report.requested - report.completed,
                    report.requested
                );
            }
            if report.origins.is_empty() {
                // The same non-verdict `service call` reports, with the same
                // wording: nothing answered, and that is not proof of absence.
                println!(
                    "\nno origin answered — a non-verdict, not proof of absence \
                     (RFC 05 §3.1); `zenctl node list` says who is up."
                );
                return Ok(());
            }
            println!(
                "\n{:<16} {:>8} {:>9} {:>9} {:>9} {:>9} {:>9}",
                "origin", "replies", "min ms", "p50 ms", "p95 ms", "p99 ms", "max ms"
            );
            for o in &report.origins {
                println!(
                    "{:<16} {:>8} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>9.2}",
                    o.origin, o.replies, o.min_ms, o.p50_ms, o.p95_ms, o.p99_ms, o.max_ms
                );
            }
            // Errors and silence are counted apart on purpose: averaging a
            // non-answer into a latency figure is how a benchmark lies.
            if report.errors > 0 || report.silent > 0 {
                println!(
                    "\n{} error repl(ies), {} call(s) drew no reply at all.",
                    report.errors, report.silent
                );
            }
        }
    }
    Ok(())
}

/// `zenctl registry diff` (issue #50).
pub fn registry_diff(report: &RegistryDiff, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.producers.iter().for_each(json_line),
        _ => {
            if report.producers.is_empty() {
                println!("no producers on either side.");
                return Ok(());
            }
            for p in &report.producers {
                let versions = match (&p.served_version, &p.local_version) {
                    (Some(s), Some(l)) if s == l => format!("registry {s}"),
                    (Some(s), Some(l)) => format!("served {s} · local {l}"),
                    (Some(s), None) => format!("served {s} · local —"),
                    (None, Some(l)) => format!("served — · local {l}"),
                    (None, None) => String::new(),
                };
                if p.findings.is_empty() {
                    println!("  {:<16} {versions}   agree", p.producer);
                } else {
                    println!("✗ {:<16} {versions}", p.producer);
                    for f in &p.findings {
                        println!("      {f}");
                    }
                }
            }
            // RFC 08 §6: a disagreement is a finding, not a verdict — the
            // count is reported, the exit code is not touched.
            println!(
                "\n{} producer(s), {} disagreeing (RFC 08 §6: a disagreement is a finding).",
                report.producers.len(),
                report.disagreeing()
            );
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

/// `blob list` — the registry's `[[blob]]` declarations (RFC 07 §2.7 / 08 §2).
///
/// The footer is not decoration: a declaration is a *capability*, and the one
/// mistake this table invites is reading it as possession. So the prose says
/// what would actually answer that question.
pub fn blob_list(report: &BlobList, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => report.tiers.iter().for_each(json_line),
        _ => {
            if report.tiers.is_empty() {
                println!(
                    "no producer declares an @blob tier ({} slice(s) read). \
                     An empty table here is what the registry says, not what the bus holds.",
                    report.slices_considered
                );
                return Ok(());
            }
            println!("declared @blob tiers:\n");
            for row in &report.tiers {
                let flag = if row.known_tier {
                    ""
                } else {
                    "  (unreserved tier token)"
                };
                println!(
                    "  {:<12} {:<9} v{}{}",
                    row.producer, row.tier, row.registry_version, flag
                );
                if !row.endpoints.is_empty() {
                    println!("      endpoints  {}", row.endpoints.join(", "));
                }
                if let Some(algo) = &row.algo {
                    println!("      algo       {algo}");
                }
                if let Some(t) = &row.reference {
                    println!("      reference  {t}  (carries the content root — RFC 07 §2.1)");
                }
                if let Some(e) = &row.encoding {
                    println!("      encoding   {e}");
                }
                // Three different facts, three different sentences. An empty
                // roster is not a fleet verdict (RFC 05 §3.1) and an unasked
                // one is not an empty one (RFC 09 §5.1 O4).
                match &row.origins {
                    Some(origins) if origins.is_empty() => println!(
                        "      origins    — (no liveliness token answered; silence is not a \
                         verdict — RFC 05 §3.1)"
                    ),
                    Some(origins) => println!("      origins    {}", origins.join(" ")),
                    None => println!("      origins    — (roster not asked)"),
                }
                if let Some(d) = &row.description {
                    println!("      {d}");
                }
            }
            println!(
                "\n{} of {} slice(s) declare a tier. A declaration is a capability, never \
                 possession — `zenctl blob probe <id>` asks who actually holds one.",
                report.slices_considered - report.slices_without_blob,
                report.slices_considered
            );
        }
    }
    Ok(())
}

/// `blob probe` — who answered, and at which root (RFC 07 §2.5).
pub fn blob_probe(report: &BlobProbeReport, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => {
            // An envelope line first, *always*, then one line per holder.
            //
            // Row-per-line alone would mean a probe that found nobody — or one
            // that was never issued at all, as the content-addressed tiers are
            // — emits an empty stream, which reads as "asked, nobody holds it".
            // The coverage statement (O5) and the reason nothing was asked (O4)
            // have to survive the pipe, so they get their own object.
            json_line(&serde_json::json!({
                "probe": report.target,
                "tier": report.tier,
                "asked": report.asked,
                "not_probed": report.not_probed,
                "answered": report.answered,
                "roots": report.roots,
                "declared_by": report.declared_by,
            }));
            report.holders.iter().for_each(json_line);
        }
        _ => {
            println!("target  {}  (tier {})", report.target, report.tier);

            // O5: name the coverage before the result, so "no holders" is read
            // against what was actually asked.
            if let Some(why) = &report.not_probed {
                println!("\nnot probed: {why}");
                if !report.declared_by.is_empty() {
                    println!(
                        "\ndeclared by: {} — a capability claim from the registry, not a \
                         statement that any of them holds this object.",
                        report.declared_by.join(", ")
                    );
                }
                return Ok(());
            }
            println!("asked:");
            for selector in &report.asked {
                println!("  {selector}");
            }

            if report.holders.is_empty() {
                println!(
                    "\nno replies. Silence is not a verdict (RFC 05 §3.1): no origin holds \
                     this id, none is up, or the timeout was too short."
                );
                if !report.declared_by.is_empty() {
                    println!(
                        "declared by: {} — so the tier exists in the registry; \
                         `zenctl node list` says whether they are up.",
                        report.declared_by.join(", ")
                    );
                }
                return Ok(());
            }

            println!("\nholders:\n");
            // Only tier 1 has a manifest endpoint, so "no manifest reply" is
            // a verdict there and a category error on the tier-2 rows.
            let tier1 = report.tier == "artifact";
            for h in &report.holders {
                print!("  {:<16}", h.origin);
                match (&h.availability, &h.manifest) {
                    (Some(a), Some(m)) => print!(
                        "  {}/{} chunks · {} bytes · root {}",
                        a.have,
                        a.chunk_count,
                        m.total_len,
                        short(&m.root)
                    ),
                    (Some(a), None) if tier1 => {
                        print!("  {}/{} chunks · no manifest reply", a.have, a.chunk_count)
                    }
                    (Some(a), None) => print!("  {}/{} chunks", a.have, a.chunk_count),
                    (None, Some(m)) => print!(
                        "  {} bytes · root {} · no have reply",
                        m.total_len,
                        short(&m.root)
                    ),
                    (None, None) => print!("  answered, said nothing readable"),
                }
                println!();
                if let Some(n) = &h.note {
                    println!("      note: {n}");
                }
                if let Some(u) = &h.unreadable {
                    println!("      unreadable: {u}");
                }
                if let Some(e) = &h.error {
                    println!("      ✗ {} — {}", e.name, e.message);
                }
                println!("      {}", h.key);
            }

            if report.roots.len() > 1 {
                println!(
                    "\nROOT DIFFERS — {} distinct content roots under one id:",
                    report.roots.len()
                );
                for r in &report.roots {
                    println!("  {r}");
                }
                println!(
                    "The id is a name; the root is the anchor (RFC 07 §2.1). This is a \
                     finding, not a tie-break: pass --root <hex> to say which content you \
                     mean, and the fetch will refuse anything else."
                );
            }
        }
    }
    Ok(())
}

/// `blob fetch` — what one transfer cost and proved.
/// `blob fetch tree/<root>` — the validated index summary (RFC 07 §2.3,
/// v1.17). Inspection, not download: no destination file, no content store,
/// and the pin is the key itself.
pub fn blob_tree(report: &zenkey_fleet::report::BlobTreeIndexReport, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json | Format::Ndjson => {
            json_doc(report);
            Ok(())
        }
        _ => {
            println!("tree/{}", report.root);
            println!("  from      {} ({})", report.origin, report.key);
            println!(
                "  index     {} entr{}, {} file(s)",
                report.entries,
                if report.entries == 1 { "y" } else { "ies" },
                report.files
            );
            println!(
                "  content   {} bytes in {} distinct chunk(s) — not fetched; \
                 the summary needs no content store (RFC 07 §2.3)",
                report.total_size, report.chunks
            );
            println!("  priority  {} (RFC 07 §2.6)", report.priority);
            println!(
                "  root      {} (pinned by construction — the key is the identity)",
                report.root
            );
            Ok(())
        }
    }
}

pub fn blob_fetch(report: &BlobFetchReport, format: Format) -> Result<()> {
    match format.resolved() {
        Format::Json | Format::Ndjson => json_doc(report),
        _ => {
            println!("{}", report.dest);
            println!("  from      {} ({})", report.origin, report.key);
            println!(
                "  bytes     {} in {} chunk(s){}",
                report.bytes,
                report.chunks,
                if report.chunks_resumed > 0 {
                    format!(", {} resumed", report.chunks_resumed)
                } else {
                    String::new()
                }
            );
            println!("  priority  {} (RFC 07 §2.6)", report.priority);
            if report.root_pinned {
                println!("  root      {} (pinned)", report.root);
            } else {
                println!(
                    "  root      trust-on-first-use — this origin chose the content \
                     (RFC 07 §2.1)"
                );
            }
            // Nonzero rejections mean a replier served bytes that did not
            // verify. The transfer succeeded anyway, which is the point of
            // verifying before disk — but it is a fact about the fleet.
            if report.rejected > 0 {
                println!(
                    "  rejected  {} repl{} failed verification before disk",
                    report.rejected,
                    if report.rejected == 1 { "y" } else { "ies" }
                );
            }
            if report.retries > 0 {
                println!("  retries   {}", report.retries);
            }
            eprintln!("{} ms", report.elapsed_ms);
        }
    }
    Ok(())
}

fn short(hash: &str) -> String {
    if hash.len() <= 16 {
        return hash.to_string();
    }
    format!("{}…", &hash[..16])
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
                    // `-` where the layout did not say. Absent is not empty:
                    // a storage with no strip_prefix and one whose admin
                    // document omits the field are different facts.
                    println!(
                        "  {:<16} strip {}  ·  volume {}",
                        "",
                        s.strip_prefix.as_deref().unwrap_or("-"),
                        s.volume.as_deref().unwrap_or("-")
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
pub fn rate(report: &RateReport, format: Format, bandwidth: bool, loss: bool, latency: bool) {
    if latency {
        // The caveat is part of the measurement (#119): both clocks named,
        // never a verdict on the transport.
        eprintln!(
            "latency = arrival wall-clock − publisher HLC: observed *skewed*              latency — it contains clock skew, and negative values are the              skew evidence, not an error (RFC 09 §5.1)"
        );
    }
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
                    if latency {
                        match &row.latency {
                            Some(l) => print!(
                                "  lat med {} p95 {} (min {} max {}, {} stamped, {} unstamped)",
                                human_us(l.median_us),
                                human_us(l.p95_us),
                                human_us(l.min_us),
                                human_us(l.max_us),
                                l.samples,
                                row.unstamped,
                            ),
                            None => print!(
                                "  lat — ({} unstamped: no HLC, no latency — not zero)",
                                row.unstamped
                            ),
                        }
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

/// Microseconds, humanised with sign kept — a negative latency is the skew
/// evidence (#119).
fn human_us(us: i64) -> String {
    let sign = if us < 0 { "-" } else { "" };
    let abs = us.unsigned_abs();
    if abs >= 1_000_000 {
        format!("{sign}{:.2}s", abs as f64 / 1_000_000.0)
    } else if abs >= 1_000 {
        format!("{sign}{:.1}ms", abs as f64 / 1_000.0)
    } else {
        format!("{sign}{abs}µs")
    }
}

/// `zenctl record`'s closing report — the counts are the honest record;
/// the progress line was never more than progress (issue #53).
pub fn record(report: &zenkey_fleet::RecordReport, format: Format) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => json_line(report),
        _ => {
            println!(
                "recorded {} sample(s) in {:.1}s{}",
                report.samples,
                report.duration_ms as f64 / 1000.0,
                report
                    .out
                    .as_deref()
                    .map(|o| format!(" to {o}"))
                    .unwrap_or_default(),
            );
            if report.dropped > 0 {
                println!(
                    "{} sample(s) dropped while behind — recorded as in-file drop \
                     records where the gaps happened; the capture is a partial \
                     view and says so (RFC 09 §5.1 O6)",
                    report.dropped
                );
            }
        }
    }
}

/// `zenctl replay`'s closing report: what was (or would have been)
/// published, and what the capture itself had already lost (issue #53).
pub fn replay(report: &zenkey_fleet::ReplayReport, format: Format) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => json_line(report),
        _ => {
            let verb = if report.dry_run {
                "would replay"
            } else {
                "replayed"
            };
            println!(
                "{verb} {} put(s) and {} tombstone(s) from {} (captured {})",
                report.published,
                report.tombstones,
                report.header.selectors.join(" + "),
                report.header.captured_at,
            );
            if report.capture_dropped > 0 {
                println!(
                    "the capture dropped {} sample(s) at record time — this \
                     replay is a partial view of a partial view (RFC 09 §5.2)",
                    report.capture_dropped
                );
            }
            if report.malformed > 0 || report.refused > 0 {
                println!(
                    "{} malformed row(s), {} refused delete row(s) — counted, \
                     not silently skipped:",
                    report.malformed, report.refused
                );
                for e in &report.first_errors {
                    println!("  {e}");
                }
            }
        }
    }
}

/// `zenctl cutover`'s report (issue #59): both halves of RFC 09 §6, and a
/// three-state verdict — quiet-everywhere is a non-verdict, not a pass.
pub fn cutover(report: &zenkey_fleet::report::CutoverReport, format: Format) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => json_line(report),
        _ => {
            println!(
                "old root {}: {} sample(s) on {} key(s) over {}s",
                report.old_root, report.old_samples, report.old_keys_seen, report.window_s
            );
            for k in &report.old_examples {
                println!("  ✗ {k}");
            }
            println!(
                "new plane {}**: {} sample(s)",
                report.new_prefix, report.new_samples
            );
            if report.leak_samples > 0 {
                println!(
                    "leaks (outside {} and not the old root): {} sample(s) on {} key(s)",
                    report.new_prefix, report.leak_samples, report.leaked_keys_seen
                );
                for k in &report.leak_examples {
                    println!("  ! {k}");
                }
            }
            if report.dropped > 0 {
                println!(
                    "{} sample(s) dropped while behind (O6) — the silence claim \
                     covers only what was seen",
                    report.dropped
                );
            }
            match report.verdict {
                zenkey_fleet::report::CutoverVerdict::Pass => println!(
                    "PASS — the retired family is silent while the new plane \
                     carries traffic (RFC 09 §6, both halves)"
                ),
                zenkey_fleet::report::CutoverVerdict::OldStillSpeaks => println!(
                    "FAIL — the retired family still speaks; a migration you can \
                     assert the absence of is a migration you can finish (RFC 09 §6)"
                ),
                zenkey_fleet::report::CutoverVerdict::Unproven => println!(
                    "UNPROVEN — the old root was silent but so was the new plane: \
                     a dead fleet passes the silence half for free (RFC 05 §3.1); \
                     bring the fleet up and run it again"
                ),
            }
        }
    }
}

/// `zenctl probe`'s report (issue #59): the resolution provenance, then the
/// call rendered like any `service call`.
pub fn probe(report: &zenkey_fleet::report::ProbeReport, format: Format) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => json_line(report),
        _ => {
            println!(
                "probe {} → origin {} (via {})",
                report.input, report.origin, report.via
            );
            call(&report.call, Format::Table, |a| match (&a.value, &a.text) {
                (Some(v), _) => serde_json::to_string_pretty(v).unwrap_or_default(),
                (None, Some(t)) => t.clone(),
                _ => String::new(),
            });
            if report.call.answers.is_empty() {
                println!(
                    "the origin resolved but did not answer — the probe reached a \
                     name, not a responder; absence of replies and absence of \
                     callers must not look alike (RFC 09 §6)"
                );
            }
        }
    }
}
