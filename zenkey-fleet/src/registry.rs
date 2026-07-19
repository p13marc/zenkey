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
}
