//! Registry-slice sets (RFC 08 §6): one type over both sources.
//!
//! A slice is a slice regardless of where it was read — a producer's served
//! `introspect` reply off the live bus, or a local `registry/*.toml` file.
//! [`SliceSet`] carries them uniformly (with an optional on-disk cache so
//! repeated invocations and shell completion answer instantly), and exposes
//! the subject-refinement lookups every renderer needs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::report::{ProducerDiff, RegistryDiff};
use anyhow::{Result, anyhow};
use zenkey::{RegistrySlice, parse_slice};
use zenoh::Session;

/// One slice's subject patterns, parsed once and grouped by class.
///
/// `refine` runs **per sample** on zenctl's decode path and per first-sight
/// key in zengui, and it used to parse every subject pattern of the class on
/// every call — then clone them all again to hand `best_match` a contiguous
/// slice. Parsing at construction turns that into a map lookup
/// (`docs/zero-copy.md`).
#[derive(Debug, Clone, Default)]
struct ParsedSubjects {
    /// Index into the slice's own `subjects`, parallel to `pats`.
    idx: Vec<usize>,
    /// Contiguous, so `best_match` takes it borrowed.
    pats: Vec<zenkey::pattern::SubjectPattern>,
}

/// A set of registry slices, indexed by producer/service base name.
#[derive(Debug, Clone, Default)]
pub struct SliceSet {
    slices: Vec<RegistrySlice>,
    /// The raw TOML per slice, kept for the disk cache (slices do not
    /// re-serialize; the served text is the artifact).
    raw: Vec<String>,
    /// Parsed subject patterns per slice, keyed by class. Rebuilt wholesale
    /// with its slice — the two vectors are index-parallel, and `push` is the
    /// only place either grows.
    parsed: Vec<std::collections::BTreeMap<String, ParsedSubjects>>,
}

/// Group one slice's subjects by class, parsing each pattern once. A subject
/// whose pattern does not parse is dropped here exactly as it was dropped
/// per-call before — a malformed declaration refines nothing.
fn parse_subjects(slice: &RegistrySlice) -> std::collections::BTreeMap<String, ParsedSubjects> {
    let mut out: std::collections::BTreeMap<String, ParsedSubjects> = Default::default();
    for (i, s) in slice.subjects.iter().enumerate() {
        if let Ok(p) = zenkey::pattern::SubjectPattern::parse(&s.path) {
            let entry = out.entry(s.class.clone()).or_default();
            entry.idx.push(i);
            entry.pats.push(p);
        }
    }
    out
}

impl SliceSet {
    /// Load from local `registry/*.toml` dirs — the offline source. What a
    /// checked-out application *declares*. (`types.toml` is the type table,
    /// not a slice — skipped.)
    pub fn from_dirs(dirs: &[PathBuf]) -> Result<SliceSet> {
        let mut set = SliceSet::default();
        for dir in dirs {
            let mut paths: Vec<_> = std::fs::read_dir(dir)
                .map_err(|e| anyhow!("--registry {}: {e}", dir.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .filter(|p| p.file_name().is_none_or(|n| n != "types.toml"))
                .collect();
            paths.sort();
            for path in paths {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| anyhow!("{}: {e}", path.display()))?;
                let slice = parse_slice(&text).map_err(|e| {
                    anyhow!(
                        "{}: does not parse as a registry slice: {e}",
                        path.display()
                    )
                })?;
                set.push(slice, text);
            }
        }
        Ok(set)
    }

    /// Discover every live producer's served slice from the bus
    /// ([`crate::query::fleet_registry`]).
    pub async fn from_bus(session: &Session, base: &str, timeout: Duration) -> Result<SliceSet> {
        let pairs = crate::query::fleet_registry_raw(session, base, timeout).await?;
        let mut set = SliceSet::default();
        for (slice, raw) in pairs {
            set.push(slice, raw);
        }
        Ok(set)
    }

    fn push(&mut self, slice: RegistrySlice, raw: String) {
        // One slice per base name; last one wins (a fleet mid-rollout serves
        // several versions — the newest reply is as good a pick as any, and
        // `doctor` is where disagreement is *reported*).
        let parsed = parse_subjects(&slice);
        if let Some(i) = self.slices.iter().position(|s| s.name == slice.name) {
            self.slices[i] = slice;
            self.raw[i] = raw;
            self.parsed[i] = parsed;
        } else {
            self.slices.push(slice);
            self.raw.push(raw);
            self.parsed.push(parsed);
        }
    }

    /// Each slice with the raw TOML it was parsed from — the pair
    /// `write_cache` persists. The text is empty for a set built by
    /// [`from_slices`](Self::from_slices), which has none to give.
    pub fn entries(&self) -> impl Iterator<Item = (&RegistrySlice, &str)> {
        self.slices.iter().zip(self.raw.iter().map(String::as_str))
    }

    pub fn slices(&self) -> &[RegistrySlice] {
        &self.slices
    }

    pub fn get(&self, name: &str) -> Option<&RegistrySlice> {
        self.slices.iter().find(|s| s.name == name)
    }

    /// The slice declaring a service origin (`@catalog`) — service keys have
    /// no producer chunk, so refinement resolves through this.
    pub fn by_service_origin(&self, origin: &str) -> Option<&RegistrySlice> {
        self.slices
            .iter()
            .find(|s| s.service_origin.as_deref() == Some(origin))
    }

    /// Refine a subject tail against one producer's slice: the matching
    /// subject declaration plus its named variable bindings.
    pub fn refine<'s>(
        &'s self,
        producer: &str,
        class: &str,
        tail: &[&str],
    ) -> Option<(&'s zenkey::slice::SubjectDecl, Vec<(String, String)>)> {
        let i = self.slices.iter().position(|s| s.name == producer)?;
        let slice = &self.slices[i];
        // Precedence-ordered via the shared matcher (issue #7): the class's
        // patterns were parsed at construction, so this is a map lookup and a
        // borrowed slice — no parse, no clone, per sample.
        let candidates = self.parsed[i].get(class)?;
        let (winner, binds) = zenkey::pattern::best_match(&candidates.pats, tail)?;
        let subject_idx = candidates.idx[winner];
        Some((
            &slice.subjects[subject_idx],
            binds.into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
        ))
    }

    /// Build from already-parsed slices (no raw TOML retained — such a set
    /// is skipped by `write_cache`).
    pub fn from_slices(slices: Vec<RegistrySlice>) -> SliceSet {
        let raw = vec![String::new(); slices.len()];
        let parsed = slices.iter().map(parse_subjects).collect();
        SliceSet {
            slices,
            raw,
            parsed,
        }
    }

    /// Write the raw slice TOMLs to a cache dir (one file per producer).
    /// Repeated invocations and dynamic shell completion read this instead
    /// of round-tripping the bus.
    pub fn write_cache(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        for (slice, raw) in self.slices.iter().zip(&self.raw) {
            if raw.is_empty() {
                continue; // from_slices sets: nothing faithful to persist
            }
            std::fs::write(dir.join(format!("{}.toml", slice.name)), raw)?;
        }
        Ok(())
    }

    /// Read a previously written cache dir. Same forgiving posture as
    /// `from_dirs`, but a missing dir is an empty set, not an error.
    pub fn read_cache(dir: &Path) -> SliceSet {
        if !dir.is_dir() {
            return SliceSet::default();
        }
        SliceSet::from_dirs(&[dir.to_path_buf()]).unwrap_or_default()
    }
}

/// Where a slice set came from — the §6.1 decision made typed: `--registry`
/// and the bus stop being exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceSource {
    Bus,
    Dirs,
    Union,
}

/// One producer where the served slice and the on-disk slice disagree.
///
/// A disagreement is **data**, not an error: served wins in the union (the
/// bus is the runtime truth, RFC 08 §6.1), and the difference is retained for
/// `doctor` to report instead of being silently overwritten.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SliceDisagreement {
    pub producer: String,
    pub bus_version: String,
    pub dirs_version: String,
    /// Whether anything beyond the version string differs (subjects,
    /// procedures, blob tiers).
    pub shape_differs: bool,
}

/// A union load's full outcome.
#[derive(Debug, Clone)]
pub struct UnionOutcome {
    pub set: SliceSet,
    /// Producers whose slice came from the bus.
    pub from_bus: Vec<String>,
    /// Producers only the dirs supplied.
    pub dirs_only: Vec<String>,
    pub disagreements: Vec<SliceDisagreement>,
}

impl SliceSet {
    /// Load the union of the live bus and local dirs: **served wins per
    /// producer**, dirs fill the gaps, and every producer where the two
    /// disagree is retained as a [`SliceDisagreement`].
    ///
    /// Degrades honestly: an unreachable bus yields a dirs-only union (the
    /// outcome's `from_bus` is empty — the caller can see which case it got).
    pub async fn from_union(
        session: &zenoh::Session,
        base: &str,
        dirs: &[std::path::PathBuf],
        timeout: std::time::Duration,
    ) -> Result<UnionOutcome> {
        let bus = SliceSet::from_bus(session, base, timeout)
            .await
            .unwrap_or_default();
        let disk = if dirs.is_empty() {
            SliceSet::default()
        } else {
            SliceSet::from_dirs(dirs)?
        };

        // Carry each slice's raw TOML through the merge (issue #54): a union
        // that dropped it produced a set `write_cache` silently skipped, so
        // the `--registry` path — the offline one, where a warm completion
        // cache matters most — cached nothing at all.
        let mut merged = SliceSet::default();
        let mut from_bus = Vec::new();
        let mut dirs_only = Vec::new();
        let mut disagreements = Vec::new();

        for (served, raw) in bus.entries() {
            from_bus.push(served.name.clone());
            if let Some(local) = disk.get(&served.name)
                && (local.version != served.version || local != served)
            {
                disagreements.push(SliceDisagreement {
                    producer: served.name.clone(),
                    bus_version: served.version.clone(),
                    dirs_version: local.version.clone(),
                    shape_differs: {
                        // Same version but different content is the worse lie.
                        let mut a = served.clone();
                        let mut b = local.clone();
                        a.version = String::new();
                        b.version = String::new();
                        a != b
                    },
                });
            }
            merged.push(served.clone(), raw.to_string());
        }
        for (local, raw) in disk.entries() {
            if bus.get(&local.name).is_none() {
                dirs_only.push(local.name.clone());
                merged.push(local.clone(), raw.to_string());
            }
        }

        Ok(UnionOutcome {
            set: merged,
            from_bus,
            dirs_only,
            disagreements,
        })
    }
}

impl SliceSet {
    /// Compare this set — what the fleet **serves** — against what a checkout
    /// **declares**, per producer.
    ///
    /// Pure, so the comparison is testable without a bus, and engine-side so
    /// both explorers can make it (issue #208). The per-producer comparison
    /// is already `zenkey::slice::diff`; this is the set-level join that
    /// decides what to do about a producer only one side knows.
    pub fn diff(&self, local: &SliceSet) -> RegistryDiff {
        let served = self;
        let mut names: Vec<&str> = served
            .slices()
            .iter()
            .chain(local.slices())
            .map(|s| s.name.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();

        let mut producers = Vec::new();
        for name in names {
            let s = served.get(name);
            let l = local.get(name);
            producers.push(match (s, l) {
                (Some(s), Some(l)) => ProducerDiff {
                    producer: name.to_string(),
                    served_version: Some(s.version.clone()),
                    local_version: Some(l.version.clone()),
                    findings: zenkey::slice::diff(s, l)
                        .iter()
                        .map(|f| f.summary())
                        .collect(),
                },
                // Present on one side only. Neither is an error: a producer the
                // bus serves and the checkout does not know may simply be newer,
                // and one the checkout declares that nothing serves may simply be
                // down (RFC 05 §3.1 — silence is not a verdict).
                (Some(s), None) => ProducerDiff {
                    producer: name.to_string(),
                    served_version: Some(s.version.clone()),
                    local_version: None,
                    findings: vec!["served by the fleet, absent from the local registry".into()],
                },
                (None, Some(l)) => ProducerDiff {
                    producer: name.to_string(),
                    served_version: None,
                    local_version: Some(l.version.clone()),
                    findings: vec![
                        "declared locally, not served by any origin — down, or not deployed \
                         (silence is not a verdict, RFC 05 §3.1)"
                            .into(),
                    ],
                },
                (None, None) => unreachable!("name came from one of the two sets"),
            });
        }
        RegistryDiff { producers }
    }
}

#[cfg(test)]
impl SliceSet {
    /// Test constructor from one slice TOML (crate-internal).
    pub(crate) fn from_toml_for_tests(toml: &str) -> SliceSet {
        let mut set = SliceSet::default();
        set.push(parse_slice(toml).unwrap(), toml.to_string());
        set
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = r#"
        [registry]
        version = "1.0"
        app = "t"
        convention = 1
        [producer]
        name = "alpha"
        [[subject]]
        path = "flow/{q}"
        class = "telemetry"
        type = "Point"
        [[subject]]
        path = "flow/special"
        class = "telemetry"
        type = "Special"
    "#;

    #[test]
    fn refine_uses_shared_precedence() {
        let mut set = SliceSet::default();
        set.push(parse_slice(A).unwrap(), A.to_string());
        // Literal beats {var} — the shared best_match ordering.
        let (s, binds) = set
            .refine("alpha", "telemetry", &["flow", "special"])
            .unwrap();
        assert_eq!(s.type_name, "Special");
        assert!(binds.is_empty());
        let (s, binds) = set.refine("alpha", "telemetry", &["flow", "p95"]).unwrap();
        assert_eq!(s.type_name, "Point");
        assert_eq!(binds, vec![("q".to_string(), "p95".to_string())]);
        assert!(set.refine("alpha", "state", &["flow", "p95"]).is_none());
    }

    #[test]
    fn cache_round_trips_and_last_slice_wins() {
        let mut set = SliceSet::default();
        set.push(parse_slice(A).unwrap(), A.to_string());
        // A newer slice for the same producer replaces, never duplicates.
        set.push(parse_slice(A).unwrap(), A.to_string());
        assert_eq!(set.slices().len(), 1);

        let dir = std::env::temp_dir().join(format!("zenkey-fleet-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        set.write_cache(&dir).unwrap();
        let back = SliceSet::read_cache(&dir);
        assert_eq!(back.slices().len(), 1);
        assert_eq!(back.get("alpha").unwrap().subjects.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
        // Missing dir: empty set, not an error.
        assert!(
            SliceSet::read_cache(Path::new("/nonexistent-zkf"))
                .slices()
                .is_empty()
        );
    }

    /// Union semantics without a bus: dirs fill everything, nothing claimed
    /// from the bus, no invented disagreements.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn union_degrades_to_dirs_when_the_bus_is_silent() {
        let session = crate::session::open(&[], &[], false).await.unwrap();
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
        let out = SliceSet::from_union(&session, "", &[dir], std::time::Duration::from_millis(200))
            .await
            .unwrap();
        assert!(out.from_bus.is_empty(), "no bus answered");
        assert!(!out.dirs_only.is_empty(), "dirs supplied the slices");
        assert!(out.disagreements.is_empty());
        assert_eq!(out.set.slices().len(), out.dirs_only.len());
    }

    fn set(toml: &str) -> SliceSet {
        SliceSet::from_slices(vec![zenkey::parse_slice(toml).unwrap()])
    }

    const SERVED: &str = r#"
[registry]
version = "2.0"
app = "t"
convention = 1
[producer]
name = "netring"
[[subject]]
path = "flows"
class = "telemetry"
type = "TelemetryPoint"
[[subject]]
path = "brand/new"
class = "telemetry"
type = "TelemetryPoint"
"#;

    const LOCAL: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "netring"
[[subject]]
path = "flows"
class = "telemetry"
type = "TelemetryPoint"
"#;

    /// The diff reports exactly the edited subject, plus the version skew —
    /// #50's acceptance, without a bus.
    #[test]
    fn the_diff_names_the_one_subject_that_moved() {
        let report = set(SERVED).diff(&set(LOCAL));
        assert_eq!(report.producers.len(), 1);
        let p = &report.producers[0];
        assert_eq!(p.served_version.as_deref(), Some("2.0"));
        assert_eq!(p.local_version.as_deref(), Some("1.0"));
        assert!(
            p.findings.iter().any(|f| f.contains("brand/new")),
            "{:?}",
            p.findings
        );
        assert!(
            p.findings.iter().any(|f| f.contains("2.0")),
            "the version skew is a finding too: {:?}",
            p.findings
        );
    }

    /// One-sided presence is a fact with a reason, never an error — and the
    /// two sides read differently.
    #[test]
    fn one_sided_producers_explain_themselves() {
        let empty = SliceSet::from_slices(vec![]);
        let served_only = set(SERVED).diff(&empty);
        assert!(served_only.producers[0].findings[0].contains("absent from the local registry"));
        assert!(served_only.producers[0].local_version.is_none());

        let local_only = empty.diff(&set(LOCAL));
        assert!(local_only.producers[0].findings[0].contains("silence is not a verdict"));
        assert!(local_only.producers[0].served_version.is_none());
    }
}
