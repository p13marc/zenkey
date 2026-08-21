//! Output rendering (issue #12): one report, three renderings.
//!
//! `table` keeps the human-first prose the tool always had; `json` is the
//! whole report as stable serde; `ndjson` is one JSON object per row for
//! stream processing (`| jq`). `auto` = table on a tty, ndjson when piped.

use anyhow::Result;
use serde::Serialize;

use crate::report::*;

/// Re-exported from [`crate::render`], which is where the one `match` on it
/// lives (#198). The name stays reachable here so the twenty-odd `cmd/*` call
/// sites keep compiling while the renderers move family by family.
pub use crate::render::Format;

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
    crate::render::emit(&mut std::io::stdout(), report, format)
}

pub fn topic_info(report: &TopicInfo, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}
pub fn service_list(report: &ServiceList, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `service info` (issue #211).
pub fn service_info(report: &ServiceInfo, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

pub fn interface_list(report: &InterfaceList, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

pub fn interface_show(report: &InterfaceShow, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl bench rpc` (issue #52).
pub fn bench(report: &BenchReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl registry diff` (issue #50).
pub fn registry_diff(report: &RegistryDiff, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl schema <producer>` (issue #51).
pub fn schema_dump(report: &SchemaDump, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

pub fn node_list(report: &NodeList, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

pub fn base_list(report: &BaseList, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `blob list` — the registry's `[[blob]]` declarations (RFC 07 §2.7 / 08 §2).
///
/// The footer is not decoration: a declaration is a *capability*, and the one
/// mistake this table invites is reading it as possession. So the prose says
/// what would actually answer that question.
pub fn blob_list(report: &BlobList, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `blob probe` — who answered, and at which root (RFC 07 §2.5).
pub fn blob_probe(report: &BlobProbeReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `blob fetch tree/<root>` — the validated index summary (RFC 07 §2.3,
/// v1.17). Inspection, not download: no destination file, no content store,
/// and the pin is the key itself.
pub fn blob_tree(report: &BlobTreeIndexReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `blob fetch` — what one transfer cost and proved.
pub fn blob_fetch(report: &BlobFetchReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

pub fn storage_list(report: &StorageList, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `topic hz` / `topic bw` (issue #46): the typed report renders through the
/// global `--format` like every other command; the O6 eviction note prints in
/// every mode.
pub fn rate(report: &RateReport, format: Format, bandwidth: bool, loss: bool, latency: bool) {
    if latency {
        // The caveat is part of the measurement (#119) and names *which*
        // clock (#213), because "the publisher's HLC" was only ever true on a
        // fleet where publishers do the timestamping. Worded by the engine so
        // the GUI cannot describe the same number differently (RFC 09 §5.1 O7).
        let sample = report.rows.iter().find_map(|r| r.latency.as_ref());
        match sample {
            Some(l) => eprintln!("latency = {}", l.caveat()),
            None => eprintln!(
                "latency = arrival wall-clock − the sample's HLC, which is stamped by \
                 the first node with timestamping enabled — not necessarily the \
                 publisher (RFC 09 §5.1 O7)"
            ),
        }
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
                            // One clause per population, never one median
                            // across them: a publisher-stamped sample and a
                            // router-stamped one measure from different
                            // clocks (#213).
                            Some(l) => {
                                for (label, s) in l.populations() {
                                    print!(
                                        "  lat[{label}] med {} p95 {} (min {} max {}, {})",
                                        human_us(s.median_us),
                                        human_us(s.p95_us),
                                        human_us(s.min_us),
                                        human_us(s.max_us),
                                        s.samples,
                                    );
                                }
                                print!("  ({} unstamped)", row.unstamped);
                            }
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
    crate::render::emit(&mut std::io::stdout(), report, format)
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

/// `zenctl expect`'s report (#160): the window's facts, then the verdict
/// with every failed requirement spelled out.
pub fn expect(report: &zenkey_fleet::report::ExpectReport, format: Format) {
    match format.resolved() {
        Format::Json => json_doc(report),
        Format::Ndjson => json_line(report),
        _ => {
            println!(
                "{}: {} sample(s) on {} key(s) over {:.1}s{}{}",
                report.selector,
                report.samples,
                report.keys_seen,
                report.window_s,
                if report.ended_early {
                    " (met early)"
                } else {
                    ""
                },
                match report.rate_hz {
                    Some(r) => format!(", {r:.2} Hz over the full window"),
                    None => String::new(),
                }
            );
            if report.dropped > 0 {
                println!(
                    "{} sample(s) dropped while behind (O6) — counted into the verdict",
                    report.dropped
                );
            }
            if !report.violations.is_empty() {
                println!(
                    "violations ({} shown of {}):",
                    report.violations.len(),
                    report.violations_total
                );
                for v in &report.violations {
                    println!("  ✗ {v}");
                }
            }
            match report.verdict {
                zenkey_fleet::report::ExpectVerdict::Met => println!("MET"),
                zenkey_fleet::report::ExpectVerdict::NotMet => {
                    println!("NOT MET — on a clean observation:");
                    for u in &report.unmet {
                        println!("  ✗ {u}");
                    }
                }
                zenkey_fleet::report::ExpectVerdict::Impaired => {
                    println!(
                        "IMPAIRED — the observation cannot carry the claim \
                         (RFC 09 §5.1 O6); this is not a verdict either way:"
                    );
                    for u in &report.unmet {
                        println!("  ! {u}");
                    }
                }
            }
        }
    }
}
