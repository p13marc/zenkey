//! A producer's declared `[budget]` against what its health document says
//! it costs (#391): the agent that grew from 110 MB to 355 MB on a 1 GB
//! host, was OOM-killed, and reported `Healthy` throughout.
//!
//! RFC 08 §2 (v1.32) lets a producer or service file declare what it may
//! cost — `rss_mb`, and a bound per named table — and RFC 04 §1.2 names the
//! object on the health document that reports the same quantities,
//! `self_stats { rss_bytes, budget_bytes?, tables?: [{name, entries,
//! bytes}] }`. RFC 13 §3 says what a judge owes the declaration:
//!
//! - **Not asked** when the slice declares no `[budget]`. Nothing here runs
//!   for such a slice; the doctor's deep phase never fetches its health.
//! - **Unobservable** when no health document carrying `self_stats` was
//!   seen — and the reason reads *"this producer does not say how big it
//!   is"*, because that is itself the finding an operator wants. One
//!   Warning per origin, never folded into a pass. A declared table the
//!   document does not report is unobservable for that table, the same way.
//! - **Established(no)** — `budget-exceeded`, severity Error — per origin,
//!   when `rss_bytes` exceeds `rss_mb · 2^20` or a named table exceeds
//!   `max_entries` or `max_bytes`.
//! - **Established(yes)** otherwise, for the sample read: no finding.
//!
//! A fetch of health documents costs the data plane, so it is asked for
//! explicitly — `doctor --deep` — never folded into an ambient render (RFC
//! 13 §3's frugality note). Pure, the house pattern of
//! [`crate::judge::field`] and [`crate::judge::budget`] (which is the
//! *cardinality* budget — hence this module's name): nothing here takes a
//! session, so the same reading judges a recorded health document.

use serde_json::Value;
use zenkey::RegistrySlice;

use crate::report::{CheckId, DoctorFinding, DoctorSeverity};

/// What a health document says about its producer's size (RFC 04 §1.2,
/// `self_stats`). Every field is optional on the wire and stays optional
/// here: an absent number is *unobservable*, which is a different answer
/// from zero.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelfStats {
    /// The resident set, in bytes.
    pub rss_bytes: Option<u64>,
    /// The budget the producer believes it runs under, in bytes — absent
    /// when none is configured. Reported, not judged: the registry's
    /// `rss_mb` is the declaration, and this is the producer's echo of it.
    pub budget_bytes: Option<u64>,
    /// Each bounded structure the producer keeps, with its occupancy.
    pub tables: Vec<TableStats>,
}

/// One `self_stats.tables[]` row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableStats {
    pub name: String,
    pub entries: Option<u64>,
    pub bytes: Option<u64>,
}

/// Read `self_stats` off a health document.
///
/// `None` when the document carries no `self_stats`, or carries one that is
/// not an object — both are "this producer does not say how big it is".
/// Within the object every field is read when it is a non-negative integer
/// and left `None` otherwise; a `tables` row without a `name` is dropped,
/// because nothing could be matched against it.
pub fn read_self_stats(doc: &Value) -> Option<SelfStats> {
    let stats = doc.get("self_stats")?.as_object()?;
    let count = |v: Option<&Value>| v.and_then(Value::as_u64);
    let tables = stats
        .get("tables")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some(TableStats {
                name: row.get("name")?.as_str()?.to_string(),
                entries: count(row.get("entries")),
                bytes: count(row.get("bytes")),
            })
        })
        .collect();
    Some(SelfStats {
        rss_bytes: count(stats.get("rss_bytes")),
        budget_bytes: count(stats.get("budget_bytes")),
        tables,
    })
}

/// The `budget-exceeded` findings for one slice's health documents.
///
/// `answers` is one entry per origin that answered the health GET: `Some`
/// when its document carried `self_stats`, `None` when it did not (or was
/// not a document at all). `asked` is how many origins the roster showed
/// running this producer, so silence can be stated with its scope (RFC 05
/// §3.1). A slice without a `[budget]` yields nothing — not asked.
pub fn judge_self_stats(
    slice: &RegistrySlice,
    answers: &[(String, Option<SelfStats>)],
    asked: usize,
) -> Vec<DoctorFinding> {
    let Some(budget) = &slice.budget else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let finding = |severity, subject: String, evidence: String, citation: &str| DoctorFinding {
        severity,
        check: CheckId::BudgetExceeded,
        subject,
        evidence,
        citation: Some(citation.to_string()),
    };

    if answers.is_empty() {
        out.push(finding(
            DoctorSeverity::Warning,
            slice.name.clone(),
            format!(
                "no health document answered for {} (asked {asked} origin(s)) — its \
                 declared budget is unobservable this run",
                slice.name
            ),
            "RFC 13 §3",
        ));
        return out;
    }

    for (origin, stats) in answers {
        let subject = format!("{origin}/{}", slice.name);
        let Some(stats) = stats else {
            out.push(finding(
                DoctorSeverity::Warning,
                subject,
                "unobservable: this producer does not say how big it is (no `self_stats` \
                 on its health document)"
                    .to_string(),
                "RFC 04 §1.2",
            ));
            continue;
        };

        if let Some(rss_mb) = budget.rss_mb {
            match stats.rss_bytes {
                Some(rss_bytes) if rss_bytes > mib_to_bytes(rss_mb) => out.push(finding(
                    DoctorSeverity::Error,
                    subject.clone(),
                    format!(
                        "resident set {} exceeds the declared budget of {rss_mb} MiB \
                         (rss_bytes = {rss_bytes})",
                        mib(rss_bytes)
                    ),
                    "RFC 08 §2",
                )),
                Some(_) => {}
                None => out.push(finding(
                    DoctorSeverity::Warning,
                    subject.clone(),
                    format!(
                        "unobservable: `self_stats` carries no `rss_bytes` to judge the \
                         declared {rss_mb} MiB against"
                    ),
                    "RFC 04 §1.2",
                )),
            }
        }

        for table in &budget.tables {
            let Some(seen) = stats.tables.iter().find(|t| t.name == table.name) else {
                out.push(finding(
                    DoctorSeverity::Warning,
                    subject.clone(),
                    format!(
                        "unobservable: table `{}` is budgeted but `self_stats.tables` does \
                         not report it",
                        table.name
                    ),
                    "RFC 04 §1.2",
                ));
                continue;
            };
            for (what, bound, observed) in [
                ("entries", table.max_entries, seen.entries),
                ("bytes", table.max_bytes, seen.bytes),
            ] {
                let Some(bound) = bound else {
                    continue;
                };
                // A negative bound cannot come from a linted registry (#313)
                // and is unsatisfiable from a foreign one: judged as zero.
                let bound = u64::try_from(bound).unwrap_or(0);
                if let Some(observed) = observed
                    && observed > bound
                {
                    out.push(finding(
                        DoctorSeverity::Error,
                        subject.clone(),
                        format!(
                            "table `{}` holds {observed} {what}, over its declared \
                             max_{what} of {bound}",
                            table.name
                        ),
                        "RFC 08 §2",
                    ));
                }
            }
        }
    }
    out
}

/// `rss_mb · 2^20`, saturating — a bound too large to spell in bytes is
/// one nothing exceeds. A negative bound cannot come from a linted registry
/// (#313); from a foreign one it is judged as zero.
fn mib_to_bytes(mib: i64) -> u64 {
    u64::try_from(mib).unwrap_or(0).saturating_mul(1 << 20)
}

/// Bytes rendered in MiB, one decimal, so the finding reads in the unit
/// the budget was declared in.
fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1u64 << 20) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zenkey::slice::{BudgetDecl, TableBudget};

    fn budgeted() -> RegistrySlice {
        let mut slice = RegistrySlice::new("1.0", "t", "demo");
        let mut budget = BudgetDecl::new();
        budget.rss_mb = Some(64);
        let mut flows = TableBudget::new("flows");
        flows.max_entries = Some(65536);
        flows.max_bytes = Some(16_777_216);
        budget.tables.push(flows);
        slice.budget = Some(budget);
        slice
    }

    fn errors(findings: &[DoctorFinding]) -> Vec<&DoctorFinding> {
        findings
            .iter()
            .filter(|f| f.severity == DoctorSeverity::Error)
            .collect()
    }

    /// `self_stats` is read when it is an object and every field is
    /// optional; absent or malformed, the answer is `None` — unobservable,
    /// not zero.
    #[test]
    fn self_stats_is_read_when_present_and_none_otherwise() {
        assert_eq!(read_self_stats(&json!({"status": "Healthy"})), None);
        assert_eq!(read_self_stats(&json!({"self_stats": 12})), None);
        assert_eq!(read_self_stats(&json!("Healthy")), None);
        let stats = read_self_stats(&json!({
            "self_stats": {
                "rss_bytes": 123_456_789u64,
                "tables": [
                    {"name": "flows", "entries": 4096},
                    {"entries": 1},
                    {"name": "names", "bytes": -5}
                ]
            }
        }))
        .expect("an object is read");
        assert_eq!(stats.rss_bytes, Some(123_456_789));
        assert_eq!(stats.budget_bytes, None);
        assert_eq!(stats.tables.len(), 2, "a nameless row is dropped");
        assert_eq!(stats.tables[0].entries, Some(4096));
        assert_eq!(stats.tables[0].bytes, None);
        assert_eq!(
            stats.tables[1].bytes, None,
            "a negative count is not a count"
        );
    }

    /// The bound is exact: 64 MiB is within a 64 MiB budget, one byte more
    /// is over it — and the finding says both numbers in MiB.
    #[test]
    fn rss_is_judged_against_rss_mb_exactly() {
        let slice = budgeted();
        let at = SelfStats {
            rss_bytes: Some(64 << 20),
            ..Default::default()
        };
        let over = SelfStats {
            rss_bytes: Some((64 << 20) + 1),
            ..Default::default()
        };
        let stats = |s: SelfStats| {
            let mut s = s;
            s.tables.push(TableStats {
                name: "flows".into(),
                entries: Some(1),
                bytes: Some(1),
            });
            s
        };
        let f = judge_self_stats(&slice, &[("h-1".into(), Some(stats(at)))], 1);
        assert!(f.is_empty(), "at the bound is within it: {f:?}");
        let f = judge_self_stats(&slice, &[("h-1".into(), Some(stats(over)))], 1);
        let e = errors(&f);
        assert_eq!(e.len(), 1, "{f:?}");
        assert_eq!(e[0].check, CheckId::BudgetExceeded);
        assert_eq!(e[0].subject, "h-1/demo");
        assert!(e[0].evidence.contains("64.0 MiB"), "{}", e[0].evidence);
        assert!(e[0].evidence.contains("64 MiB"), "{}", e[0].evidence);
        assert_eq!(e[0].citation.as_deref(), Some("RFC 08 §2"));
    }

    /// Tables match by name: a budgeted table over either bound is an Error
    /// naming the table and the numbers; one the document does not report
    /// is unobservable, said so; one the budget never named is nobody's
    /// business.
    #[test]
    fn tables_are_matched_by_name_and_judged_per_bound() {
        let slice = budgeted();
        let stats = SelfStats {
            rss_bytes: Some(1 << 20),
            tables: vec![
                TableStats {
                    name: "flows".into(),
                    entries: Some(70_000),
                    bytes: Some(16_777_216),
                },
                TableStats {
                    name: "unbudgeted".into(),
                    entries: Some(u64::MAX),
                    bytes: None,
                },
            ],
            ..Default::default()
        };
        let f = judge_self_stats(&slice, &[("h-1".into(), Some(stats))], 1);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, DoctorSeverity::Error);
        assert!(f[0].evidence.contains("`flows`"), "{}", f[0].evidence);
        assert!(f[0].evidence.contains("70000 entries"), "{}", f[0].evidence);
        assert!(f[0].evidence.contains("65536"), "{}", f[0].evidence);

        let missing = SelfStats {
            rss_bytes: Some(1 << 20),
            ..Default::default()
        };
        let f = judge_self_stats(&slice, &[("h-1".into(), Some(missing))], 1);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, DoctorSeverity::Warning);
        assert!(f[0].evidence.contains("unobservable"), "{}", f[0].evidence);
        assert!(f[0].evidence.contains("`flows`"), "{}", f[0].evidence);
        assert_eq!(f[0].citation.as_deref(), Some("RFC 04 §1.2"));
    }

    /// Per origin, never pooled: one host over and one under is one finding,
    /// on the host that is over.
    #[test]
    fn origins_are_judged_one_by_one() {
        let mut slice = budgeted();
        slice.budget.as_mut().unwrap().tables.clear();
        let f = judge_self_stats(
            &slice,
            &[
                (
                    "h-1".into(),
                    Some(SelfStats {
                        rss_bytes: Some(100 << 20),
                        ..Default::default()
                    }),
                ),
                (
                    "h-2".into(),
                    Some(SelfStats {
                        rss_bytes: Some(10 << 20),
                        ..Default::default()
                    }),
                ),
            ],
            2,
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].subject, "h-1/demo");
    }

    /// A document without `self_stats` is the stated unobservability, with
    /// the reason an operator wants; no document at all is silence with its
    /// scope; and a slice without a `[budget]` is never asked.
    #[test]
    fn unobservable_and_not_asked_are_said_not_folded() {
        let slice = budgeted();
        let f = judge_self_stats(&slice, &[("h-1".into(), None)], 1);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, DoctorSeverity::Warning);
        assert!(
            f[0].evidence.contains("does not say how big it is"),
            "{}",
            f[0].evidence
        );

        let f = judge_self_stats(&slice, &[], 3);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, DoctorSeverity::Warning);
        assert!(
            f[0].evidence.contains("asked 3 origin(s)"),
            "{}",
            f[0].evidence
        );

        let unbudgeted = RegistrySlice::new("1.0", "t", "demo");
        assert!(judge_self_stats(&unbudgeted, &[("h-1".into(), None)], 1).is_empty());
        assert!(judge_self_stats(&unbudgeted, &[], 0).is_empty());
    }
}
