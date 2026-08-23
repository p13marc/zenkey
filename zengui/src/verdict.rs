//! The payload-conformance verdict cache (#164): what has been checked, for
//! whom, and never on a render path.
//!
//! The engine's [`zenkey_fleet::decode::decode_sample`] may fetch a
//! producer's `describe` on a first miss, so it can touch the bus — which is
//! why the justfile walkthrough bans schema decode from render paths, and why
//! echo deliberately renders through the sync structural decode only
//! (`crate::echo`). This cache is the other half of that bargain: a bounded
//! per-tick batch of samples is validated in a `Task`
//! (`crate::services::value::validate`), the verdicts land here, and the
//! tree, the echo rows and the Inspector *look up* — a `HashMap` read, never
//! a decode.
//!
//! ## What a cached entry claims
//!
//! An entry is the verdict of the **most recently checked sample** on that
//! key, and the surfaces word it that way. A key with no entry has not been
//! checked, which renders as [`UNCHECKED_LABEL`] — "not validated (not yet
//! checked)" — never as either answer (RFC 09 §5.1 O4). Entries go stale
//! against the schema they were judged under, so everything that can change
//! a schema clears the cache: a base change (`Verdicts::forget`), a registry
//! (re)load, and the doctor's "re-ask schemas". That is the `(key, schema
//! hash)` discipline the issue asks for, kept without a per-lookup hash
//! fetch: the hash for a key can only change through one of those three
//! doors, and each door clears.
//!
//! ## The bound
//!
//! Bounded like every other table here, and the bound reports its cost
//! (RFC 09 §5.1 O6): past [`CAPACITY`] keys, new entries are refused and
//! counted — the [`crate::echo`] path-table shape, not a silent LRU that
//! would make "checked" flicker.

use std::collections::HashMap;

use zenkey::schema::validate::{NotValidated, Verdict};

use crate::view::theme::VerdictTone;

/// Distinct keys the cache retains verdicts for. Matches the order of the
/// engine's key-table default; a refused key renders as unchecked, which is
/// honest, where an evicted one would flicker between checked and not.
pub const CAPACITY: usize = 4096;

/// Samples validated per bus tick, at most (#164: off the render path, and
/// bounded so a hot bus costs a fixed slice of CPU, not a proportional one).
pub const VALIDATE_PER_TICK: usize = 16;

/// Ticks before a checked key is offered for revalidation (~5 s at the
/// 250 ms tick): a key that goes invalid mid-session is re-caught, and a
/// producer that starts serving `describe` upgrades its `NotValidated`.
pub const REVALIDATE_TICKS: u64 = 20;

/// Payloads larger than this are not validated — the same reasoning as
/// echo's decode limit: a structural decode of megabytes per tick would burn
/// the budget for a badge. The key stays unchecked, which is stated.
pub const VALIDATE_LIMIT: usize = 64 * 1024;

/// The echo row's wording for a key with no cache entry: the third state,
/// spelled so it can never read as a verdict.
pub const UNCHECKED_LABEL: &str = "not validated (not yet checked)";

/// One key's cached verdict — of the most recently checked sample.
#[derive(Debug, Clone)]
pub struct CachedVerdict {
    pub verdict: Verdict,
    /// The cache's logical tick at which the check landed.
    at: u64,
}

/// Bounded key → verdict map, filled by `services::value` tasks and read by
/// the render paths.
#[derive(Debug)]
pub struct VerdictCache {
    map: HashMap<String, CachedVerdict>,
    capacity: usize,
    /// Logical clock, bumped once per bus tick — drives revalidation.
    tick: u64,
    /// Entries refused to stay within the bound (RFC 09 §5.1 O6).
    refused: u64,
    /// Verdicts recorded since the cache was last cleared.
    checked: u64,
}

impl Default for VerdictCache {
    fn default() -> VerdictCache {
        VerdictCache::new(CAPACITY)
    }
}

impl VerdictCache {
    pub fn new(capacity: usize) -> VerdictCache {
        VerdictCache {
            map: HashMap::new(),
            capacity: capacity.max(1),
            tick: 0,
            refused: 0,
            checked: 0,
        }
    }

    /// Advance the logical clock — once per bus tick.
    pub fn advance(&mut self) {
        self.tick += 1;
    }

    /// The cached verdict for a key, if one was ever recorded. A miss is
    /// "not yet checked" ([`UNCHECKED_LABEL`]), never a verdict.
    pub fn get(&self, key: &str) -> Option<&CachedVerdict> {
        self.map.get(key)
    }

    /// Whether this key is worth spending one of the tick's validation slots
    /// on: never checked, or checked long enough ago to re-ask.
    pub fn should_check(&self, key: &str) -> bool {
        match self.map.get(key) {
            None => true,
            Some(e) => self.tick.saturating_sub(e.at) >= REVALIDATE_TICKS,
        }
    }

    /// Record one checked sample's verdict. Past the bound, a *new* key is
    /// refused and counted (an existing key always updates — refreshing a
    /// verdict costs no capacity).
    pub fn record(&mut self, key: &str, verdict: Verdict) {
        self.checked += 1;
        if let Some(entry) = self.map.get_mut(key) {
            entry.verdict = verdict;
            entry.at = self.tick;
            return;
        }
        if self.map.len() >= self.capacity {
            self.refused += 1;
            return;
        }
        self.map.insert(
            key.to_string(),
            CachedVerdict {
                verdict,
                at: self.tick,
            },
        );
    }

    /// Forget every verdict. The schema (or registry, or fleet) they were
    /// judged under is gone; keeping them would claim checks against a
    /// contract that no longer exists. The `refused` counter survives — a
    /// cleared cache does not un-refuse what was already refused (the #188
    /// counter rule).
    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Distinct keys with a cached verdict.
    pub fn keys_checked(&self) -> usize {
        self.map.len()
    }

    /// Entries refused for the bound (O6).
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// The bound itself, for the strip that states it.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Verdicts recorded since the last clear.
    pub fn checked(&self) -> u64 {
        self.checked
    }

    /// The logical clock's reading — the bus-tick count, which the budget
    /// throttle shares rather than keeping a second counter.
    pub fn tick_count(&self) -> u64 {
        self.tick
    }
}

/// The tone a verdict reads as (#164) — the one mapping every surface uses,
/// so the tree, echo and the Inspector cannot disagree about which state is
/// which.
pub fn tone(v: &Verdict) -> VerdictTone {
    match v {
        Verdict::Valid => VerdictTone::Valid,
        Verdict::Invalid(_) => VerdictTone::Invalid,
        Verdict::NotValidated(_) => VerdictTone::NotValidated,
    }
}

/// The badge word for a verdict — compact, but each state (and each
/// not-validated *reason*) distinct: `NoRegistry`'s "nobody looked" must not
/// share a spelling with `NoSchema`'s "asked, and the type has none"
/// (RFC 09 §5.1 O4; #246).
pub fn label(v: &Verdict) -> String {
    match v {
        Verdict::Valid => "valid".to_string(),
        Verdict::Invalid(errors) => format!(
            "invalid ({})",
            crate::view::kit::plural(errors.len(), "violation")
        ),
        Verdict::NotValidated(reason) => format!("not validated ({})", short_reason(*reason)),
    }
}

/// The parenthetical for each not-validated reason. Shorter than the
/// engine's `Display` sentence (these ride tree rows), but one word pair per
/// reason — never shared.
fn short_reason(reason: NotValidated) -> &'static str {
    match reason {
        NotValidated::NoSchema => "no schema served",
        NotValidated::NoRegistry => "no registry",
        NotValidated::FeatureOff => "validator compiled out",
        NotValidated::KindUnsupported => "decode is the check",
        NotValidated::Undecodable => "undecodable",
        NotValidated::BadSchema => "schema does not compile",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache is bounded, refuses past the bound, and counts what it
    /// refused (RFC 09 §5.1 O6) — while an existing key always refreshes.
    #[test]
    fn the_cache_is_bounded_and_counts_refusals() {
        let mut c = VerdictCache::new(2);
        c.record("a", Verdict::Valid);
        c.record("b", Verdict::Valid);
        c.record("c", Verdict::Valid);
        assert_eq!(c.keys_checked(), 2);
        assert_eq!(c.refused(), 1);
        assert!(c.get("c").is_none(), "refused, not evicted-someone-else");
        // Updating a held key costs no capacity and is never refused.
        c.record("a", Verdict::Invalid(vec!["x".into()]));
        assert_eq!(c.refused(), 1);
        assert!(matches!(c.get("a").unwrap().verdict, Verdict::Invalid(_)));
    }

    /// Revalidation is clocked: fresh entries are skipped, stale ones and
    /// never-checked keys are offered.
    #[test]
    fn revalidation_is_offered_only_when_stale() {
        let mut c = VerdictCache::new(8);
        assert!(c.should_check("k"), "never checked is always offered");
        c.record("k", Verdict::Valid);
        assert!(!c.should_check("k"), "just checked is not re-offered");
        for _ in 0..REVALIDATE_TICKS {
            c.advance();
        }
        assert!(c.should_check("k"), "stale entries are re-offered");
    }

    /// Clearing drops the verdicts (they were judged under a contract that
    /// is gone) but not the refusal count — a cleared cache does not
    /// un-refuse what it already refused.
    #[test]
    fn clearing_keeps_the_refusal_counter() {
        let mut c = VerdictCache::new(1);
        c.record("a", Verdict::Valid);
        c.record("b", Verdict::Valid);
        assert_eq!(c.refused(), 1);
        c.clear();
        assert_eq!(c.keys_checked(), 0);
        assert_eq!(c.refused(), 1);
        assert!(c.get("a").is_none());
    }

    /// The three states map to three tones, and every not-validated reason
    /// keeps its own words — in particular the two silences (#246): "no
    /// registry" (nobody looked) never shares a spelling with "no schema
    /// served" (asked, and the type has none).
    #[test]
    fn labels_keep_the_three_states_and_the_two_silences_apart() {
        use crate::view::theme::VerdictTone;
        assert_eq!(tone(&Verdict::Valid), VerdictTone::Valid);
        assert_eq!(tone(&Verdict::Invalid(vec![])), VerdictTone::Invalid);
        assert_eq!(
            tone(&Verdict::NotValidated(NotValidated::NoSchema)),
            VerdictTone::NotValidated
        );

        assert_eq!(label(&Verdict::Valid), "valid");
        assert_eq!(
            label(&Verdict::Invalid(vec!["a".into(), "b".into()])),
            "invalid (2 violations)"
        );
        let no_registry = label(&Verdict::NotValidated(NotValidated::NoRegistry));
        let no_schema = label(&Verdict::NotValidated(NotValidated::NoSchema));
        assert_eq!(no_registry, "not validated (no registry)");
        assert_eq!(no_schema, "not validated (no schema served)");
        assert_ne!(no_registry, no_schema, "the two silences must not merge");

        // Every reason's label is distinct from every other's — a shared
        // spelling would collapse two facts into one.
        let all = [
            NotValidated::NoSchema,
            NotValidated::NoRegistry,
            NotValidated::FeatureOff,
            NotValidated::KindUnsupported,
            NotValidated::Undecodable,
            NotValidated::BadSchema,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(short_reason(*a), short_reason(*b));
            }
        }
        // And the unchecked wording is not any verdict's wording.
        for reason in all {
            assert_ne!(label(&Verdict::NotValidated(reason)), UNCHECKED_LABEL);
        }
    }
}
