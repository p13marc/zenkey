//! Subject-pattern matching (RFC 08 §1/§2, issue #7).
//!
//! One implementation of the registry's `{var}` / `{var...}` pattern
//! semantics, shared by the codegen (`zenkey-build` orders generated parse
//! arms by [`SubjectPattern::precedence`]) and by runtime tools (zenctl's
//! subject refinement delegates here). Before v1.5 the two carried separate
//! hand-rolled copies of the same rules with no parity guarantee.
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

    /// The parse-precedence key: per-position ranks (literal < var < rest),
    /// compared lexicographically, ties broken by pattern text — exactly the
    /// order the generated parse arms are emitted in.
    pub fn precedence(&self) -> (Vec<u8>, &str) {
        let ranks = self
            .chunks
            .iter()
            .map(|c| match c {
                PatternChunk::Literal(_) => 0u8,
                PatternChunk::Var(_) => 1,
                PatternChunk::Rest(_) => 2,
            })
            .collect();
        (ranks, &self.text)
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
    order.sort_by(|&a, &b| patterns[a].precedence().cmp(&patterns[b].precedence()));
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
