//! Subject-pattern matching (RFC 08 §1/§2, issue #7).
//!
//! One implementation of the registry's `{var}` / `{var...}` pattern
//! semantics, shared by the codegen (`zenkey-build` parses every registry
//! path through [`SubjectPattern::parse`] and orders generated parse arms by
//! [`SubjectPattern::precedence_cmp`]) and by runtime tools (zenctl's subject
//! refinement delegates here).
//!
//! *This paragraph was aspirational until #320.* v1.5 moved precedence here
//! and said the rest had followed; in fact `zenkey-build` kept its own
//! `Chunk` enum and its own copy of the grammar — trailing-rest rule,
//! variable-name check and all — reaching for this module in exactly one
//! place. The parity it claimed was a coincidence for five minor versions.
//! The codegen now names these types, and what stays on its side is only
//! what does not belong here: `alive` is reserved at any position of a
//! *locally registered* pattern (RFC 03 §3, v1.25 A5b), while this type also
//! parses patterns served by a foreign fleet, where local reservations do
//! not apply.
//!
//! Semantics (byte-compatible with the generated parse):
//! - a **literal** chunk matches itself, exactly;
//! - `{var}` matches exactly one chunk and binds it;
//! - `{var...}` is trailing-only and matches **one or more** chunks (an
//!   empty rest is not a match — the family key without a tail is a
//!   different, unregistered key);
//! - precedence at the first differing position: literal < var < rest;
//!   ties break by pattern text (stable).
//!
//! The zenoh `KeFormat` engine can express the literal/`{var}` subset
//! ([`SubjectPattern::ke_format_spec`]); it cannot express our rest
//! semantics (`**` matches zero chunks; `{var...}` requires ≥ 1) nor the
//! verbatim-chunk rules, so the hand matcher here is normative and the
//! KeFormat bridge is an interop convenience, parity-pinned in tests.

use std::fmt;

use crate::grammar::is_valid_plain_chunk;

/// One position of a subject pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternChunk {
    /// A literal chunk (`flow`).
    Literal(String),
    /// A one-chunk variable (`{quantile}`).
    Var(String),
    /// A trailing rest variable (`{metric...}`), binds one or more chunks.
    Rest(String),
}

/// A pattern parse failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatternError {
    #[error("empty pattern")]
    Empty,
    #[error("{{var...}} only in trailing position: {0:?}")]
    RestNotTrailing(String),
    #[error("bad variable name {0:?}")]
    BadVarName(String),
    #[error("chunk {0:?} violates RFC 03 §2")]
    BadChunk(String),
}

/// A parsed registry subject pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectPattern {
    chunks: Vec<PatternChunk>,
    text: String,
}

impl SubjectPattern {
    /// Parse a registry pattern (`flow/red/{quantile}`, `{device}/{metric...}`).
    ///
    /// Same lexical rules as `zenkey-build`'s registry linter, minus the
    /// reserved-leaf checks (this type also parses patterns *served* by a
    /// foreign fleet, where local reservations do not apply).
    pub fn parse(pattern: &str) -> Result<Self, PatternError> {
        if pattern.is_empty() {
            return Err(PatternError::Empty);
        }
        let parts: Vec<&str> = pattern.split('/').collect();
        let mut chunks = Vec::with_capacity(parts.len());
        for (i, part) in parts.iter().enumerate() {
            if let Some(var) = part.strip_prefix('{').and_then(|p| p.strip_suffix("...}")) {
                if i != parts.len() - 1 {
                    return Err(PatternError::RestNotTrailing(pattern.to_string()));
                }
                if !is_valid_plain_chunk(var) {
                    return Err(PatternError::BadVarName(var.to_string()));
                }
                chunks.push(PatternChunk::Rest(var.to_string()));
            } else if let Some(var) = part.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
                if !is_valid_plain_chunk(var) {
                    return Err(PatternError::BadVarName(var.to_string()));
                }
                chunks.push(PatternChunk::Var(var.to_string()));
            } else {
                if !is_valid_plain_chunk(part) {
                    return Err(PatternError::BadChunk(part.to_string()));
                }
                chunks.push(PatternChunk::Literal(part.to_string()));
            }
        }
        Ok(SubjectPattern {
            chunks,
            text: pattern.to_string(),
        })
    }

    /// The pattern's positions.
    pub fn chunks(&self) -> &[PatternChunk] {
        &self.chunks
    }

    /// The pattern text as written.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Match a concrete subject tail, binding named variables. A rest
    /// variable binds the remaining chunks joined with `/`.
    pub fn matches(&self, tail: &[&str]) -> Option<Vec<(&str, String)>> {
        let has_rest = matches!(self.chunks.last(), Some(PatternChunk::Rest(_)));
        let fixed = if has_rest {
            self.chunks.len() - 1
        } else {
            self.chunks.len()
        };
        if has_rest {
            // Rest binds one or more chunks (matches the generated parse).
            if tail.len() <= fixed {
                return None;
            }
        } else if tail.len() != fixed {
            return None;
        }
        let mut binds = Vec::new();
        for (i, c) in self.chunks.iter().enumerate() {
            match c {
                PatternChunk::Literal(l) => {
                    if tail[i] != l {
                        return None;
                    }
                }
                PatternChunk::Var(v) => binds.push((v.as_str(), tail[i].to_string())),
                PatternChunk::Rest(v) => binds.push((v.as_str(), tail[i..].join("/"))),
            }
        }
        Some(binds)
    }

    /// One position's parse-precedence rank: literal < var < rest.
    fn rank(chunk: &PatternChunk) -> u8 {
        match chunk {
            PatternChunk::Literal(_) => 0,
            PatternChunk::Var(_) => 1,
            PatternChunk::Rest(_) => 2,
        }
    }

    /// Compare two patterns by parse precedence: per-position ranks
    /// (literal < var < rest) lexicographically, ties broken by pattern text
    /// — exactly the order the generated parse arms are emitted in.
    ///
    /// A comparison, not a key, and that is the point (#321). The key form
    /// built a `Vec<u8>` per call, so a `sort_by` over *n* patterns allocated
    /// on the order of *n log n* times — inside the comparator, where the one
    /// thing that must stay cheap is the comparison. Nothing about the
    /// ordering changes; `Iterator::cmp` over the ranks is lexicographic in
    /// the same way `Vec`'s `Ord` is, including on the prefix case.
    pub fn precedence_cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.chunks
            .iter()
            .map(Self::rank)
            .cmp(other.chunks.iter().map(Self::rank))
            .then_with(|| self.text.as_str().cmp(other.text.as_str()))
    }

    /// The selector tail for this family: `{var}` → `*`, `{var...}` → `**`.
    pub fn selector_tail(&self) -> String {
        let parts: Vec<&str> = self
            .chunks
            .iter()
            .map(|c| match c {
                PatternChunk::Literal(l) => l.as_str(),
                PatternChunk::Var(_) => "*",
                PatternChunk::Rest(_) => "**",
            })
            .collect();
        parts.join("/")
    }

    /// The zenoh `KeFormat` spec for this pattern (`${var:*}` fields), when
    /// the pattern is expressible in that engine: rest variables are **not**
    /// (KeFormat's `**` matches zero chunks; `{var...}` requires one or
    /// more), so they return `None` and the hand matcher stays normative.
    pub fn ke_format_spec(&self) -> Option<String> {
        let mut out = String::new();
        for (i, c) in self.chunks.iter().enumerate() {
            if i > 0 {
                out.push('/');
            }
            match c {
                PatternChunk::Literal(l) => out.push_str(l),
                PatternChunk::Var(v) => {
                    out.push_str("${");
                    out.push_str(v);
                    out.push_str(":*}");
                }
                PatternChunk::Rest(_) => return None,
            }
        }
        Some(out)
    }
}

impl fmt::Display for SubjectPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// The most-literal-first winner across a pattern set: the pattern that the
/// generated parse (which emits arms in precedence order) would select.
/// Returns the winning pattern's index and its bindings.
pub fn best_match<'p>(
    patterns: &'p [SubjectPattern],
    tail: &[&str],
) -> Option<(usize, Vec<(&'p str, String)>)> {
    let mut order: Vec<usize> = (0..patterns.len()).collect();
    order.sort_by(|&a, &b| patterns[a].precedence_cmp(&patterns[b]));
    for idx in order {
        if let Some(binds) = patterns[idx].matches(tail) {
            return Some((idx, binds));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> SubjectPattern {
        SubjectPattern::parse(s).unwrap()
    }

    /// Ordering is unchanged by #321 — the key became a comparison, not a
    /// different order. Pinned against the *old* key form, computed here, so
    /// a future edit to `precedence_cmp` cannot quietly reorder generated
    /// parse arms.
    #[test]
    fn precedence_cmp_agrees_with_the_key_it_replaced() {
        fn key(p: &SubjectPattern) -> (Vec<u8>, &str) {
            let ranks = p
                .chunks()
                .iter()
                .map(|c| match c {
                    PatternChunk::Literal(_) => 0u8,
                    PatternChunk::Var(_) => 1,
                    PatternChunk::Rest(_) => 2,
                })
                .collect();
            (ranks, p.as_str())
        }
        let pats: Vec<SubjectPattern> = [
            "flow/red/count",
            "flow/{quantile}/count",
            "flow/{rest...}",
            "{device}/count",
            "{device}/{metric...}",
            "a",
            "a/b",
            // Same ranks, different text — the tie-break.
            "flow/{a}/count",
            "flow/{b}/count",
        ]
        .iter()
        .map(|p| SubjectPattern::parse(p).unwrap())
        .collect();

        for a in &pats {
            for b in &pats {
                assert_eq!(
                    a.precedence_cmp(b),
                    key(a).cmp(&key(b)),
                    "{} vs {}",
                    a.as_str(),
                    b.as_str()
                );
            }
        }
        // And the property the ordering exists for: most literal first.
        let mut order: Vec<&SubjectPattern> = pats.iter().collect();
        order.sort_by(|a, b| a.precedence_cmp(b));
        assert_eq!(order[0].as_str(), "a");
        assert_eq!(order.last().unwrap().as_str(), "{device}/{metric...}");
    }

    #[test]
    fn parse_rules() {
        assert!(SubjectPattern::parse("").is_err());
        assert!(matches!(
            SubjectPattern::parse("{rest...}/x"),
            Err(PatternError::RestNotTrailing(_))
        ));
        assert!(matches!(
            SubjectPattern::parse("{Bad Var}"),
            Err(PatternError::BadVarName(_))
        ));
        assert!(matches!(
            SubjectPattern::parse("UPPER/x"),
            Err(PatternError::BadChunk(_))
        ));
        assert_eq!(
            p("flow/red/{quantile}").chunks(),
            &[
                PatternChunk::Literal("flow".into()),
                PatternChunk::Literal("red".into()),
                PatternChunk::Var("quantile".into()),
            ]
        );
    }

    #[test]
    fn matching_binds_named_vars() {
        assert_eq!(
            p("flow/red/{quantile}").matches(&["flow", "red", "p95_ms"]),
            Some(vec![("quantile", "p95_ms".to_string())])
        );
        assert_eq!(p("flow/red/{quantile}").matches(&["flow", "red"]), None);
        assert_eq!(p("health").matches(&["health"]), Some(vec![]));
        // Rest binds one or more, joined.
        let dev = p("{device}/{metric...}");
        assert_eq!(
            dev.matches(&["sw1", "if", "eth0", "rx"]),
            Some(vec![
                ("device", "sw1".to_string()),
                ("metric", "if/eth0/rx".to_string()),
            ])
        );
        assert_eq!(dev.matches(&["sw1"]), None, "rest requires >= 1 chunk");
    }

    #[test]
    fn precedence_literal_beats_var_beats_rest() {
        let patterns = [p("{device}/{metric...}"), p("health"), p("{var}")];
        // "health" matches both the literal and {var}: literal must win.
        let (idx, _) = best_match(&patterns, &["health"]).unwrap();
        assert_eq!(patterns[idx].as_str(), "health");
        // A non-literal single chunk goes to {var}.
        let (idx, binds) = best_match(&patterns, &["other"]).unwrap();
        assert_eq!(patterns[idx].as_str(), "{var}");
        assert_eq!(binds, vec![("var", "other".to_string())]);
        // Two chunks only fit the rest pattern.
        let (idx, _) = best_match(&patterns, &["sw1", "x"]).unwrap();
        assert_eq!(patterns[idx].as_str(), "{device}/{metric...}");
    }

    #[test]
    fn selector_tails() {
        assert_eq!(p("flow/red/{quantile}").selector_tail(), "flow/red/*");
        assert_eq!(p("{device}/{metric...}").selector_tail(), "*/**");
        assert_eq!(p("health").selector_tail(), "health");
    }

    #[test]
    fn ke_format_bridge_parity_on_expressible_patterns() {
        use zenoh_keyexpr::format::KeFormat;
        use zenoh_keyexpr::keyexpr;
        let pat = p("flow/red/{quantile}");
        let spec = pat.ke_format_spec().unwrap();
        let format = KeFormat::new(&spec).unwrap();
        let parsed = format
            .parse(keyexpr::new("flow/red/p95_ms").unwrap())
            .unwrap();
        let ke_bound: &str = parsed.get("quantile").unwrap();
        let hand_bound = pat.matches(&["flow", "red", "p95_ms"]).unwrap();
        assert_eq!(hand_bound, vec![("quantile", ke_bound.to_string())]);
        // Rest patterns are deliberately inexpressible (zero-chunk `**`).
        assert_eq!(p("{device}/{metric...}").ke_format_spec(), None);
    }
}
