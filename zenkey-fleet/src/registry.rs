//! Registry-slice sets (RFC 08 §6): one type over both sources.
//!
//! A slice is a slice regardless of where it was read — a producer's served
//! `introspect` reply off the live bus, or a local `registry/*.toml` file.
//! [`SliceSet`] carries them uniformly (with an optional on-disk cache so
//! repeated invocations and shell completion answer instantly), and exposes
//! the subject-refinement lookups every renderer needs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use zenkey::{RegistrySlice, parse_slice};
use zenoh::Session;

/// A set of registry slices, indexed by producer/service base name.
#[derive(Debug, Clone, Default)]
pub struct SliceSet {
    slices: Vec<RegistrySlice>,
    /// The raw TOML per slice, kept for the disk cache (slices do not
    /// re-serialize; the served text is the artifact).
    raw: Vec<String>,
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
        if let Some(i) = self.slices.iter().position(|s| s.name == slice.name) {
            self.slices[i] = slice;
            self.raw[i] = raw;
        } else {
            self.slices.push(slice);
            self.raw.push(raw);
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
        let slice = self.get(producer)?;
        // Precedence-ordered via the shared matcher (issue #7): collect the
        // class's patterns and let best_match pick — same order the codegen
        // compiles.
        let candidates: Vec<(usize, zenkey::pattern::SubjectPattern)> = slice
            .subjects
            .iter()
            .enumerate()
            .filter(|(_, s)| s.class == class)
            .filter_map(|(i, s)| {
                zenkey::pattern::SubjectPattern::parse(&s.path)
                    .ok()
                    .map(|p| (i, p))
            })
            .collect();
        let patterns: Vec<zenkey::pattern::SubjectPattern> =
            candidates.iter().map(|(_, p)| p.clone()).collect();
        let (winner, binds) = zenkey::pattern::best_match(&patterns, tail)?;
        let (subject_idx, _) = candidates[winner];
        Some((
            &slice.subjects[subject_idx],
            binds.into_iter().map(|(n, v)| (n.to_string(), v)).collect(),
        ))
    }

    /// Build from already-parsed slices (no raw TOML retained — such a set
    /// is skipped by `write_cache`).
    pub fn from_slices(slices: Vec<RegistrySlice>) -> SliceSet {
        let raw = vec![String::new(); slices.len()];
        SliceSet { slices, raw }
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
}
