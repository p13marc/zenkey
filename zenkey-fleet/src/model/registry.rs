//! Registry-slice sets (RFC 08 §6): one type over both sources.
//!
//! A slice is a slice regardless of where it was read — a producer's served
//! `introspect` reply off the live bus, or a local `registry/*.toml` file.
//! [`SliceSet`] carries them uniformly (with an optional on-disk cache so
//! repeated invocations and shell completion answer instantly), and exposes
//! the subject-refinement lookups every renderer needs.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::report::SliceDisagreement;
use crate::report::{Asked, CollapsedProducer, ProducerDiff, RegistryDiff};
use crate::{Error, Result};
use zenkey::{Declared, RegistrySlice, parse_slice};

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
    /// Producer base name → index into the three parallel vectors.
    ///
    /// [`get`](Self::get) and [`refine`](Self::refine) run **per sample** on
    /// the decode path, and both used to scan `slices` by name — a linear
    /// walk over a fleet's whole producer set, per key, to answer a question
    /// a map answers.
    ///
    /// **First wins**, because that is `find`/`position`'s rule and the
    /// shadowing it implies is observable: [`from_slices`](Self::from_slices)
    /// does not go through `push` and can be handed the same name twice, and
    /// the one that answers is the earlier. `push` replaces in place, so a
    /// re-pushed producer keeps its index — and its position in
    /// [`slices`](Self::slices) and [`entries`](Self::entries).
    by_name: std::collections::BTreeMap<String, usize>,
    /// Producers more than one origin answered for, and whether they agreed
    /// (#385) — `NotAsked` for a set built from files or from bare slices,
    /// which have no origin to collapse (#399). See
    /// [`collapsed`](Self::collapsed).
    collapsed: Asked<Vec<CollapsedProducer>>,
}

/// Group one slice's subjects by class, parsing each pattern once. A subject
/// whose pattern does not parse is dropped here exactly as it was dropped
/// per-call before — a malformed declaration refines nothing.
fn parse_subjects(slice: &RegistrySlice) -> std::collections::BTreeMap<String, ParsedSubjects> {
    let mut out: std::collections::BTreeMap<String, ParsedSubjects> = Default::default();
    for (i, s) in slice.subjects.iter().enumerate() {
        if let Ok(p) = zenkey::pattern::SubjectPattern::parse(&s.path) {
            let entry = out.entry(s.class.token().to_string()).or_default();
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
                .map_err(|e| Error::io(dir, e))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "toml"))
                .filter(|p| p.file_name().is_none_or(|n| n != "types.toml"))
                .collect();
            paths.sort();
            for path in paths {
                let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
                let slice = parse_slice(&text)
                    .map_err(|e| Error::malformed_from(path.display().to_string(), e))?;
                set.push(slice, text);
            }
        }
        Ok(set)
    }

    /// Discover every live producer's served slice from the bus
    /// ([`crate::fleet_registry_by_origin`]), collapsed to one slice per
    /// producer.
    ///
    /// The collapse is recorded rather than silent — see
    /// [`collapsed`](Self::collapsed). A caller whose question is *which
    /// host* should not come here at all: go one layer down to
    /// [`crate::fleet_registry_by_origin`], which does not deduplicate.
    pub async fn from_bus(fleet: &crate::Fleet<'_>, timeout: Duration) -> Result<SliceSet> {
        Ok(SliceSet::from_served(
            crate::bus::query::fleet_registry_by_origin(fleet, timeout).await?,
        ))
    }

    /// Fold an origin-attributed sweep into one slice per producer, keeping
    /// a receipt of what the fold discarded (#385).
    ///
    /// Pure, so the collapse is testable without a bus — and separable, so a
    /// caller that ran its own sweep can reuse the fold without re-querying.
    pub fn from_served(served: Vec<crate::ServedSlice>) -> SliceSet {
        // name -> (origins, versions, the first raw text, still-agreeing)
        let mut answers: std::collections::BTreeMap<
            String,
            (Vec<String>, Vec<String>, String, bool),
        > = Default::default();
        let mut set = SliceSet::default();
        for s in served {
            let entry = answers
                .entry(s.slice.name.clone())
                .or_insert_with(|| (Vec::new(), Vec::new(), s.raw.clone(), true));
            entry.0.push(s.origin);
            entry.1.push(s.slice.version.clone());
            // Byte equality of the served TOML, not of the parse: two builds
            // that differ only in a comment still differ, and a set that
            // called them equal would be guessing.
            if s.raw != entry.2 {
                entry.3 = false;
            }
            set.push(s.slice, s.raw);
        }
        // `Asked` even when the fold discarded nothing: a bus sweep in which
        // every producer had one origin *has* asked, and must not read like a
        // set built from files that never could (#399, RFC 13 §3 O4).
        set.collapsed = Asked::Asked(
            answers
                .into_iter()
                // One answer is not a collapse.
                .filter(|(_, (origins, ..))| origins.len() > 1)
                .map(
                    |(producer, (origins, versions, _, agreed))| CollapsedProducer {
                        producer,
                        origins,
                        versions,
                        agreed,
                    },
                )
                .collect(),
        );
        set
    }

    /// Producers this set folded more than one origin's answer into, and
    /// whether those origins agreed (#385).
    ///
    /// Three-state on purpose (#399). `NotAsked` is a set built from files or
    /// from bare slices: neither carries an origin, so nothing *could* be
    /// collapsed and no question was put. `Asked(&[])` is a bus sweep in
    /// which every producer had exactly one origin answer — the fleet agrees,
    /// and it agrees because it was asked. A bare empty slice conflated the
    /// two, which is the O4 failure this receipt exists to avoid
    /// (RFC 13 §3 O4).
    pub fn collapsed(&self) -> Asked<&[CollapsedProducer]> {
        match &self.collapsed {
            Asked::NotAsked => Asked::NotAsked,
            Asked::Asked(v) => Asked::Asked(v.as_slice()),
        }
    }

    fn push(&mut self, slice: RegistrySlice, raw: String) {
        // One slice per base name; last one wins (a fleet mid-rollout serves
        // several versions — the newest reply is as good a pick as any, and
        // `doctor` is where disagreement is *reported*). The discard is no
        // longer silent on the bus path: `from_served` records who answered
        // and whether they agreed, in `collapsed` (#385).
        let parsed = parse_subjects(&slice);
        if let Some(&i) = self.by_name.get(&slice.name) {
            self.slices[i] = slice;
            self.raw[i] = raw;
            self.parsed[i] = parsed;
        } else {
            self.by_name.insert(slice.name.clone(), self.slices.len());
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
        self.by_name.get(name).map(|&i| &self.slices[i])
    }

    /// The slice declaring a service origin (`@catalog`) — service keys have
    /// no producer chunk, so refinement resolves through this.
    pub fn by_service_origin(&self, origin: &str) -> Option<&RegistrySlice> {
        self.slices
            .iter()
            .find(|s| s.service_origin.as_ref().map(Declared::token) == Some(origin))
    }

    /// Refine a subject tail against one producer's slice: the matching
    /// subject declaration plus its named variable bindings.
    pub fn refine<'s>(
        &'s self,
        producer: &str,
        class: &str,
        tail: &[&str],
    ) -> Option<(&'s zenkey::slice::SubjectDecl, Vec<(String, String)>)> {
        let i = *self.by_name.get(producer)?;
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
        // `or_insert`, not `insert`: first wins, which is what the linear
        // `find` this replaced did with a duplicated name.
        let mut by_name = std::collections::BTreeMap::new();
        for (i, s) in slices.iter().enumerate() {
            by_name.entry(s.name.clone()).or_insert(i);
        }
        SliceSet {
            slices,
            raw,
            parsed,
            by_name,
            // Bare slices carry no origin, so nothing here *could* be
            // collapsed across origins — a duplicated name shadows (first
            // wins), and that is a different fact from a fleet disagreeing
            // (#385). Not asked, therefore, and not "asked and agreed" (#399).
            collapsed: Asked::NotAsked,
        }
    }

    /// Write the raw slice TOMLs to a cache dir (one file per producer).
    /// Repeated invocations and dynamic shell completion read this instead
    /// of round-tripping the bus.
    pub fn write_cache(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        for (slice, raw) in self.slices.iter().zip(&self.raw) {
            if raw.is_empty() {
                continue; // from_slices sets: nothing faithful to persist
            }
            let path = dir.join(format!("{}.toml", slice.name));
            std::fs::write(&path, raw).map_err(|e| Error::io(&path, e))?;
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
        fleet: &crate::Fleet<'_>,
        dirs: &[std::path::PathBuf],
        timeout: std::time::Duration,
    ) -> Result<UnionOutcome> {
        let bus = SliceSet::from_bus(fleet, timeout).await.unwrap_or_default();
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
        // The union is built fresh, so the bus set's receipt has to ride
        // across or it is lost exactly where it matters most (#385): this is
        // the constructor both explorers actually call. A dirs-only producer
        // adds nothing to it — a file has no origin to disagree with.
        merged.collapsed = bus.collapsed;

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
        RegistryDiff {
            producers,
            // The receipt of the fold that produced `served` (#399). Carried
            // rather than recomputed: the diff above is *already* built from
            // one slice per producer, so this is the record of what that cost.
            collapsed: match served.collapsed() {
                Asked::NotAsked => Asked::NotAsked,
                Asked::Asked(c) => Asked::Asked(c.to_vec()),
            },
        }
    }
}

/// The `@rpc` key a slice's procedure is asked at — a service origin's
/// verbatim `@` chunk is structurally unmatchable by a fleet selector's `*`
/// (property D4), so it takes its own key. That is the grammar working, not
/// an exception to it.
///
/// Pure: it reads the slice and spells a key, which is why it lives in the
/// model and not beside the sweep that sends it (#410) — `bus/` may lean on
/// `model/`, never the other way round, and `judge/` on both. It used to be
/// private to the doctor, which meant the describe sweep could not leave the
/// doctor without dragging the judge layer into the bus.
pub(crate) fn rpc_key(base: &str, slice: &RegistrySlice, procedure: &str) -> Result<String> {
    Ok(match &slice.service_origin {
        Some(origin) => {
            // The slice already validated it on parse — `Other` here means the
            // chunk is not a legal verbatim origin, which is the same finding
            // the hand-rolled `ServiceOrigin::new` used to report.
            // A *served* slice said this, so it is the peer that is
            // malformed — not the caller, and not the fabric.
            let o = origin.known().ok_or_else(|| {
                Error::malformed(
                    format!("slice {}", slice.name),
                    format!("carries {:?} as a service origin", origin.token()),
                )
            })?;
            zenkey::grammar::with_base(base, zenkey::selector::service_rpc(o, &[procedure]))
        }
        None => {
            zenkey::grammar::with_base(base, zenkey::selector::fleet_rpc(&slice.name, &[procedure]))
        }
    })
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

    /// The fold to one-slice-per-producer keeps a receipt of what it
    /// discarded (#385).
    ///
    /// The collapse itself is right — a decoder refining a key needs *a*
    /// slice per producer and does not care which host served it. What was
    /// wrong is that the resulting set looked complete while being one
    /// arbitrary host's answer, so a `diff` computed from it read as
    /// fleet-wide truth.
    #[test]
    fn the_fold_to_one_slice_per_producer_records_what_it_discarded() {
        let served = |origin: &str, raw: &str| crate::ServedSlice {
            origin: origin.to_string(),
            slice: parse_slice(raw).unwrap(),
            raw: raw.to_string(),
        };

        // Two hosts, one producer, disagreeing bodies: a fleet mid-rollout.
        let mut b_variant = A.to_string();
        b_variant.push_str(
            "\n[[subject]]\npath = \"extra\"\nclass = \"state\"\ntype = \"E\"\nttl_s = 1\n",
        );
        let set = SliceSet::from_served(vec![
            served("h-aaaaaaaaaaaa", A),
            served("h-bbbbbbbbbbbb", &b_variant),
        ]);
        assert_eq!(set.slices().len(), 1, "still one slice per producer");

        let collapsed = set
            .collapsed()
            .as_option()
            .copied()
            .expect("a bus fold asked");
        assert_eq!(collapsed.len(), 1, "{collapsed:?}");
        assert_eq!(collapsed[0].producer, "alpha");
        assert_eq!(
            collapsed[0].origins,
            vec!["h-aaaaaaaaaaaa", "h-bbbbbbbbbbbb"]
        );
        assert!(
            !collapsed[0].agreed,
            "the discarded answer differed — that is the finding"
        );

        // Agreement is a fact about the answers, not about how many replied.
        let agreeing = SliceSet::from_served(vec![
            served("h-aaaaaaaaaaaa", A),
            served("h-bbbbbbbbbbbb", A),
        ]);
        assert!(agreeing.collapsed().as_option().copied().expect("asked")[0].agreed);

        // One answer is not a collapse — but the sweep *asked*, and the two
        // zeros are not the same zero (#399, RFC 13 §3 O4).
        let one_host = SliceSet::from_served(vec![served("h-aaaaaaaaaaaa", A)]);
        assert_eq!(
            one_host.collapsed(),
            Asked::Asked(&[][..]),
            "asked, and nothing was collapsed"
        );
        // A set with no origins to collapse never could have been asked.
        assert_eq!(
            SliceSet::from_slices(vec![parse_slice(A).unwrap()]).collapsed(),
            Asked::NotAsked,
            "not asked is not \"the fleet agrees\""
        );
        assert_eq!(SliceSet::default().collapsed(), Asked::NotAsked);
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

    /// The name index answers exactly what the linear scan answered.
    ///
    /// Two rules, both observable, both easy to lose to a map: a re-pushed
    /// producer replaces **in place** (so `slices()` order is stable and the
    /// newest slice is the one that refines), and a set built through
    /// [`SliceSet::from_slices`] — which does not go through `push` — can
    /// hold the same name twice, where the **first** answers.
    #[test]
    fn a_re_pushed_producer_keeps_its_place_and_shadowing_is_first_wins() {
        let newer = A.replace("version = \"1.0\"", "version = \"9.9\"");
        let other = A.replace("name = \"alpha\"", "name = \"beta\"");

        let mut set = SliceSet::default();
        set.push(parse_slice(A).unwrap(), A.to_string());
        set.push(parse_slice(&other).unwrap(), other.clone());
        set.push(parse_slice(&newer).unwrap(), newer.clone());

        assert_eq!(set.slices().len(), 2, "a re-push replaces, never appends");
        assert_eq!(
            set.slices()[0].name,
            "alpha",
            "the replacement keeps the producer's position"
        );
        assert_eq!(set.get("alpha").unwrap().version, "9.9", "last push wins");
        assert_eq!(
            set.entries().next().unwrap().1,
            newer,
            "the raw TOML rides with the slice it was parsed from"
        );
        assert!(set.get("gamma").is_none());
        // …and refinement still resolves through the replaced slice.
        assert_eq!(
            set.refine("alpha", "telemetry", &["flow", "special"])
                .unwrap()
                .0
                .type_name,
            "Special"
        );
        assert!(set.refine("gamma", "telemetry", &["flow"]).is_none());

        // The shadowing `from_slices` can produce: first wins, both ways.
        let shadowed =
            SliceSet::from_slices(vec![parse_slice(A).unwrap(), parse_slice(&newer).unwrap()]);
        assert_eq!(
            shadowed.get("alpha").unwrap().version,
            "1.0",
            "the earlier of two same-named slices answers"
        );
        assert_eq!(shadowed.slices().len(), 2, "neither is dropped");
    }

    /// Union semantics without a bus: dirs fill everything, nothing claimed
    /// from the bus, no invented disagreements.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn union_degrades_to_dirs_when_the_bus_is_silent() {
        let session = crate::bus::session::open(&[], &[], false).await.unwrap();
        let dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
        let out = SliceSet::from_union(
            &crate::Fleet::new(&session, ""),
            &[dir],
            std::time::Duration::from_millis(200),
        )
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
