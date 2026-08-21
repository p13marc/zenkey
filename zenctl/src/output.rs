//! Output rendering (issue #12): one report, three renderings.
//!
//! `table` keeps the human-first prose the tool always had; `json` is the
//! whole report as stable serde; `ndjson` is one JSON object per row for
//! stream processing (`| jq`). `auto` = table on a tty, ndjson when piped.

use anyhow::Result;

use crate::report::*;

/// Re-exported from [`crate::render`], which is where the one `match` on it
/// lives (#198). The name stays reachable here so the twenty-odd `cmd/*` call
/// sites keep compiling while the renderers move family by family.
pub use crate::render::Format;

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

/// `doctor` (issue #46): findings as a table (severity mark, check id,
/// subject, evidence, citation) or as the whole typed report in JSON —
/// `zenctl doctor --format json | jq '.findings[]'` is the contract.
pub fn doctor(report: &DoctorReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl record`'s closing report — the counts are the honest record;
/// the progress line was never more than progress (issue #53).
pub fn record(report: &zenkey_fleet::RecordReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl replay`'s closing report: what was (or would have been)
/// published, and what the capture itself had already lost (issue #53).
pub fn replay(report: &zenkey_fleet::ReplayReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl cutover`'s report (issue #59): both halves of RFC 09 §6, and a
/// three-state verdict — quiet-everywhere is a non-verdict, not a pass.
pub fn cutover(report: &zenkey_fleet::report::CutoverReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}

/// `zenctl expect`'s report (#160): the window's facts, then the verdict
/// with every failed requirement spelled out.
pub fn expect(report: &zenkey_fleet::report::ExpectReport, format: Format) -> Result<()> {
    crate::render::emit(&mut std::io::stdout(), report, format)
}
