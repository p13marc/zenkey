//! The payload-conformance verdict cache (#164): what has been checked, for
//! whom, and never on a render path.
//!
//! Since #614 (FL6a) a check is zk2's: the key resolved through the lens
//! (`zenkey_fleet::Lens::check`, spec §7.2–§7.3) and the payload decoded as
//! the type its contract declares, a JSON Schema value then validated. The
//! lens itself is session-free, but resolving a key can *nudge* the lens
//! feed into a background re-read, and a decode is unbounded work — so a
//! bounded per-tick batch is checked in a `Task`
//! (`crate::services::value::validate`), the verdicts land here, and the
//! tree, the echo rows and the Inspector *look up* — a `HashMap` read,
//! never a decode.
//!
//! ## What a cached entry claims
//!
//! An entry is the [`Conformance`] of the **most recently checked sample**
//! on that key — one of four poles, kept apart in every word and glyph:
//! valid, invalid, undecodable, and not checked (with the rung that stopped
//! it). A key with no entry has not been checked by *this cache*, which
//! renders as [`UNCHECKED_LABEL`] — "not yet checked" — and is never a
//! verdict of any pole (RFC 09 §5.1 O4).
//!
//! Entries go stale against what they were judged under. A namespace move
//! and a contracts reload clear the cache: those verdicts describe another
//! deployment, or contracts no longer held. A lens generation move — a
//! provider appeared, a revision was retrieved — only marks every entry
//! *due*: the badge on screen stays the last true reading until the
//! recheck lands, rather than every badge in the tree blinking out on each
//! presence move.
//!
//! ## The bound
//!
//! Bounded like every other table here, and the bound reports its cost
//! (RFC 09 §5.1 O6): past [`CAPACITY`] keys, new entries are refused and
//! counted — the [`crate::echo`] path-table shape, not a silent LRU that
//! would make "checked" flicker.

use std::collections::HashMap;

use zenkey_fleet::report::Conformance;

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
/// key whose provider appears upgrades its "not checked".
pub const REVALIDATE_TICKS: u64 = 20;

/// Payloads larger than this are not validated — the same reasoning as
/// echo's decode limit: a structural decode of megabytes per tick would burn
/// the budget for a badge. The key stays unchecked, which is stated.
pub const VALIDATE_LIMIT: usize = 64 * 1024;

/// The wording for a key with no cache entry: the cache's own state,
/// spelled so it can never read as a verdict — of any of the four poles,
/// "not checked" included.
pub const UNCHECKED_LABEL: &str = "not yet checked";

/// One key's cached verdict — of the most recently checked sample.
#[derive(Debug, Clone)]
pub struct CachedVerdict {
    pub verdict: Conformance,
    /// The cache's logical tick at which the check landed.
    at: u64,
    /// The cache's generation at which it landed: an older one is due.
    generation: u64,
}

/// Bounded key → verdict map, filled by `services::value` tasks and read by
/// the render paths.
#[derive(Debug)]
pub struct VerdictCache {
    map: HashMap<String, CachedVerdict>,
    capacity: usize,
    /// Logical clock, bumped once per bus tick — drives revalidation.
    tick: u64,
    /// Bumped when what a check resolves through moved (the lens
    /// generation): every entry judged before is due.
    generation: u64,
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
            generation: 0,
            refused: 0,
            checked: 0,
        }
    }

    /// Advance the logical clock — once per bus tick.
    pub fn advance(&mut self) {
        self.tick += 1;
    }

    /// What a check resolves through moved (#614, FL6a): every entry stays
    /// on screen as the last true reading, and every one is due for a
    /// recheck.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    /// The cached verdict for a key, if one was ever recorded. A miss is
    /// "not yet checked" ([`UNCHECKED_LABEL`]), never a verdict.
    pub fn get(&self, key: &str) -> Option<&CachedVerdict> {
        self.map.get(key)
    }

    /// Whether this key is worth spending one of the tick's validation slots
    /// on: never checked, checked under an older lens, or checked long
    /// enough ago to re-ask.
    pub fn should_check(&self, key: &str) -> bool {
        match self.map.get(key) {
            None => true,
            Some(e) => {
                e.generation != self.generation
                    || self.tick.saturating_sub(e.at) >= REVALIDATE_TICKS
            }
        }
    }

    /// Record one checked sample's verdict. Past the bound, a *new* key is
    /// refused and counted (an existing key always updates — refreshing a
    /// verdict costs no capacity).
    pub fn record(&mut self, key: &str, verdict: Conformance) {
        self.checked += 1;
        if let Some(entry) = self.map.get_mut(key) {
            entry.verdict = verdict;
            entry.at = self.tick;
            entry.generation = self.generation;
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
                generation: self.generation,
            },
        );
    }

    /// Forget every verdict. The contracts (or fleet) they were judged under
    /// are gone; keeping them would claim checks against a contract that no
    /// longer exists. The `refused` counter survives — a cleared cache does
    /// not un-refuse what was already refused (the #188 counter rule).
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
/// so the tree, echo and the Inspector cannot disagree about which pole is
/// which.
pub fn tone(v: &Conformance) -> VerdictTone {
    match v {
        Conformance::Valid => VerdictTone::Valid,
        Conformance::Invalid { .. } => VerdictTone::Invalid,
        Conformance::Undecodable { .. } => VerdictTone::Undecodable,
        Conformance::NotChecked { .. } => VerdictTone::NotChecked,
    }
}

/// The badge word for a verdict — compact, but each pole (and each
/// not-checked *rung*) distinct: "no provider of this address in presence"
/// must not share a spelling with "not a zk2 data key" (RFC 09 §5.1 O4;
/// #246). The rung is the engine's own words (`Unresolved::words`), so this
/// window and `zenctl echo` say the same thing.
pub fn label(v: &Conformance) -> String {
    match v {
        Conformance::Valid => "valid".to_string(),
        Conformance::Invalid { violations } => format!(
            "invalid ({})",
            crate::view::kit::plural(violations.len(), "violation")
        ),
        Conformance::Undecodable { declared, .. } => format!("undecodable (as {declared})"),
        Conformance::NotChecked { reason } => format!("not checked ({reason})"),
    }
}

/// The verdict alone, without its parenthetical (#557) — what a narrow tree
/// row has room for. Four words for four poles, so the badge still never
/// collapses one into another; the reason and the count are one click away,
/// on the Inspector.
pub fn short_label(v: &Conformance) -> &'static str {
    match v {
        Conformance::Valid => "valid",
        Conformance::Invalid { .. } => "invalid",
        Conformance::Undecodable { .. } => "undecodable",
        Conformance::NotChecked { .. } => "not checked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invalid(n: usize) -> Conformance {
        Conformance::Invalid {
            violations: (0..n).map(|i| format!("/v{i}: wrong")).collect(),
        }
    }

    fn undecodable() -> Conformance {
        Conformance::Undecodable {
            declared: "nav.v2.Origin".into(),
            reason: "buffer underflow".into(),
        }
    }

    fn not_checked(reason: &str) -> Conformance {
        Conformance::NotChecked {
            reason: reason.into(),
        }
    }

    /// The cache is bounded, refuses past the bound, and counts what it
    /// refused (RFC 09 §5.1 O6) — while an existing key always refreshes.
    #[test]
    fn the_cache_is_bounded_and_counts_refusals() {
        let mut c = VerdictCache::new(2);
        c.record("a", Conformance::Valid);
        c.record("b", Conformance::Valid);
        c.record("c", Conformance::Valid);
        assert_eq!(c.keys_checked(), 2);
        assert_eq!(c.refused(), 1);
        assert!(c.get("c").is_none(), "refused, not evicted-someone-else");
        // Updating a held key costs no capacity and is never refused.
        c.record("a", invalid(1));
        assert_eq!(c.refused(), 1);
        assert!(matches!(
            c.get("a").unwrap().verdict,
            Conformance::Invalid { .. }
        ));
    }

    /// Revalidation is clocked: fresh entries are skipped, stale ones and
    /// never-checked keys are offered.
    #[test]
    fn revalidation_is_offered_only_when_stale() {
        let mut c = VerdictCache::new(8);
        assert!(c.should_check("k"), "never checked is always offered");
        c.record("k", Conformance::Valid);
        assert!(!c.should_check("k"), "just checked is not re-offered");
        for _ in 0..REVALIDATE_TICKS {
            c.advance();
        }
        assert!(c.should_check("k"), "stale entries are re-offered");
    }

    /// A lens generation move (#614, FL6a) makes every entry due at once,
    /// and leaves it on screen: the badge is the last true reading until
    /// the recheck lands, never a blank.
    #[test]
    fn a_lens_move_makes_every_entry_due_and_keeps_it() {
        let mut c = VerdictCache::new(8);
        c.record("k", not_checked("no provider of this address in presence"));
        assert!(!c.should_check("k"));
        c.invalidate();
        assert!(c.should_check("k"), "judged under the old lens: due");
        assert!(c.get("k").is_some(), "and still shown meanwhile");
        c.record("k", Conformance::Valid);
        assert!(!c.should_check("k"), "rechecked under the new one");
    }

    /// Clearing drops the verdicts (they were judged under a contract that
    /// is gone) but not the refusal count — a cleared cache does not
    /// un-refuse what it already refused.
    #[test]
    fn clearing_keeps_the_refusal_counter() {
        let mut c = VerdictCache::new(1);
        c.record("a", Conformance::Valid);
        c.record("b", Conformance::Valid);
        assert_eq!(c.refused(), 1);
        c.clear();
        assert_eq!(c.keys_checked(), 0);
        assert_eq!(c.refused(), 1);
        assert!(c.get("a").is_none());
    }

    /// The four poles map to four tones and four words, and every
    /// not-checked rung keeps its own words — the two silences (#246) among
    /// them: "no provider" (asked, and nobody serves it) never shares a
    /// spelling with "presence was not read" (nobody looked).
    #[test]
    fn labels_keep_the_four_poles_and_the_rungs_apart() {
        use crate::view::theme::VerdictTone;
        assert_eq!(tone(&Conformance::Valid), VerdictTone::Valid);
        assert_eq!(tone(&invalid(0)), VerdictTone::Invalid);
        assert_eq!(tone(&undecodable()), VerdictTone::Undecodable);
        assert_eq!(tone(&not_checked("x")), VerdictTone::NotChecked);

        assert_eq!(label(&Conformance::Valid), "valid");
        assert_eq!(label(&invalid(2)), "invalid (2 violations)");
        assert_eq!(label(&undecodable()), "undecodable (as nav.v2.Origin)");
        let no_provider = label(&not_checked("no provider of this address in presence"));
        let not_read = label(&not_checked(
            "presence was not read, so no revision is named for the key",
        ));
        assert_eq!(
            no_provider,
            "not checked (no provider of this address in presence)"
        );
        assert_ne!(no_provider, not_read, "the two silences must not merge");

        // And the cache's own wording is not any pole's wording.
        for v in [
            Conformance::Valid,
            invalid(1),
            undecodable(),
            not_checked("no provider of this address in presence"),
        ] {
            assert_ne!(label(&v), UNCHECKED_LABEL);
            assert_ne!(short_label(&v), UNCHECKED_LABEL);
        }
    }

    /// #557: the narrow tree's badge drops the parenthetical and keeps the
    /// pole. The rungs merge there on purpose — the claim ("not checked") is
    /// the same, only its why is one click away — but the four poles never
    /// do, and none reads as the unchecked wording.
    #[test]
    fn the_short_label_keeps_the_four_poles_apart() {
        let four = [
            short_label(&Conformance::Valid),
            short_label(&invalid(1)),
            short_label(&undecodable()),
            short_label(&not_checked("x")),
        ];
        assert_eq!(four, ["valid", "invalid", "undecodable", "not checked"]);
    }
}
