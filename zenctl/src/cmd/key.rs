//! `zenctl key` — keyexpr algebra without a session (#124).
//!
//! Pure `zenoh-keyexpr`: `includes` and `intersects` answer "does my
//! selector reach that key" without trying it against a live bus, and
//! `canon` shows the canonical spelling (or the parse error, verbatim).
//! What makes this more than nuze's equivalent is one sentence: when the
//! answer is the *convention's* doing rather than the algebra's — a `**`
//! stopped by an `@`-chunk, a `*` that will not match a verbatim origin —
//! the RFC citation says so.

use anyhow::Result;
use zenoh::key_expr::KeyExpr;

use crate::render::Format;

/// The answer as an exit code: 0 yes, 1 no, 2 either expression invalid.
///
/// Named for the question it answers — whether two key expressions stand in a
/// [`KeyOp`](crate::render::KeyOp) relation — rather than `Verdict`, which
/// collided by name with `zenkey_fleet::Verdict` and four `*Verdict` report
/// types (#360).
pub enum RelationVerdict {
    Yes,
    No { note: Option<String> },
    Invalid { which: &'static str, error: String },
}

impl RelationVerdict {
    pub fn exit_code(&self) -> i32 {
        match self {
            RelationVerdict::Yes => 0,
            RelationVerdict::No { .. } => 1,
            RelationVerdict::Invalid { .. } => 2,
        }
    }
}

/// Whether a "no" is explained by zenoh's verbatim-chunk semantics, which
/// the convention leans on (RFC 03 §4): D2 — `**` never crosses an
/// `@`-chunk; D4 — `*` never matches a verbatim chunk.
fn convention_note(a: &str, b: &str) -> Option<String> {
    let has_at = |s: &str| s.split('/').any(|c| c.starts_with('@'));
    let one_has_at = has_at(a) || has_at(b);
    if !one_has_at {
        return None;
    }
    let (wild, other) = if a.contains('*') { (a, b) } else { (b, a) };
    if !wild.contains('*') || !has_at(other) {
        return None;
    }
    if wild.contains("**") {
        Some(
            "note: `**` never crosses an `@`-chunk (zenoh verbatim semantics; \
             RFC 03 §4 D2) — this 'no' is the convention's doing, not a typo. \
             Name the `@`-chunk explicitly to reach past it."
                .into(),
        )
    } else {
        Some(
            "note: `*` never matches a verbatim `@`-chunk (zenoh verbatim \
             semantics; RFC 03 §4 D4) — a service origin must be named \
             explicitly."
                .into(),
        )
    }
}

/// Evaluate one relation between two expressions.
pub fn judge(op: crate::render::KeyOp, a: &str, b: &str) -> RelationVerdict {
    let ka = match KeyExpr::new(a) {
        Ok(k) => k,
        Err(e) => {
            return RelationVerdict::Invalid {
                which: "first",
                error: crate::errors::without_source_locations(&e.to_string()),
            };
        }
    };
    let kb = match KeyExpr::new(b) {
        Ok(k) => k,
        Err(e) => {
            return RelationVerdict::Invalid {
                which: "second",
                error: crate::errors::without_source_locations(&e.to_string()),
            };
        }
    };
    // Exhaustive: a third relation is a compile error here rather than a
    // silent `intersects` (#356).
    let yes = match op {
        crate::render::KeyOp::Includes => ka.includes(&kb),
        crate::render::KeyOp::Intersects => ka.intersects(&kb),
    };
    if yes {
        RelationVerdict::Yes
    } else {
        RelationVerdict::No {
            note: convention_note(a, b),
        }
    }
}

/// `key includes <a> <b>`. The op literal lives with the verb, not in the
/// dispatcher that used to name both of them (#354).
pub fn includes(cli: crate::cli::KeyIncludesArgs) -> Result<()> {
    let crate::cli::KeyIncludesArgs { a, b, out } = cli;
    relate(
        crate::render::KeyOp::Includes,
        &a,
        &b,
        out.format,
        out.color,
    )
}

/// `key intersects <a> <b>`.
pub fn intersects(cli: crate::cli::KeyIntersectsArgs) -> Result<()> {
    let crate::cli::KeyIntersectsArgs { a, b, out } = cli;
    relate(
        crate::render::KeyOp::Intersects,
        &a,
        &b,
        out.format,
        out.color,
    )
}

fn relate(
    op: crate::render::KeyOp,
    a: &str,
    b: &str,
    format: Format,
    color: crate::render::ColorChoice,
) -> Result<()> {
    let verdict = judge(op, a, b);
    match &verdict {
        RelationVerdict::Yes | RelationVerdict::No { .. } => {
            let report = crate::render::KeyRelation {
                op,
                a: a.to_string(),
                b: b.to_string(),
                answer: matches!(verdict, RelationVerdict::Yes),
                note: match &verdict {
                    RelationVerdict::No { note } => note.clone(),
                    _ => None,
                },
            };
            crate::render::emit_with(&mut std::io::stdout(), &report, format, color)?;
        }
        RelationVerdict::Invalid { which, error } => {
            // The grammar's own message, verbatim (the keyfacts rule) — minus
            // the build machine's source location (#240).
            eprintln!("the {which} expression does not parse: {error}");
        }
    }
    let code = verdict.exit_code();
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// `key canon <expr>` — canonical spelling, or the parse error verbatim.
pub fn canon(expr: &str, format: Format, color: crate::render::ColorChoice) -> Result<()> {
    match KeyExpr::autocanonize(expr.to_string()) {
        Ok(k) => {
            let report = crate::render::KeyCanon {
                input: expr.to_string(),
                canon: k.as_str().to_string(),
                changed: k.as_str() != expr,
            };
            crate::render::emit_with(&mut std::io::stdout(), &report, format, color)
        }
        Err(e) => {
            eprintln!(
                "does not parse: {}",
                crate::errors::without_source_locations(&e.to_string())
            );
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The algebra, and its exit codes: 0 yes / 1 no / 2 invalid.
    #[test]
    fn the_relations_answer_and_the_codes_follow() {
        assert!(matches!(
            judge(crate::render::KeyOp::Includes, "a/**", "a/b/c"),
            RelationVerdict::Yes
        ));
        assert!(matches!(
            judge(crate::render::KeyOp::Includes, "a/b/c", "a/**"),
            RelationVerdict::No { .. }
        ));
        assert!(matches!(
            judge(crate::render::KeyOp::Intersects, "a/*/c", "a/b/*"),
            RelationVerdict::Yes
        ));
        assert!(matches!(
            judge(crate::render::KeyOp::Intersects, "a/b", "a/c"),
            RelationVerdict::No { .. }
        ));
        assert_eq!(
            judge(crate::render::KeyOp::Includes, "a/**", "a/b").exit_code(),
            0
        );
        assert_eq!(
            judge(crate::render::KeyOp::Includes, "a/b", "a/c").exit_code(),
            1
        );
        assert_eq!(
            judge(crate::render::KeyOp::Includes, "a//b", "a").exit_code(),
            2
        );
    }

    /// The two footguns the convention leans on, cited when they bite:
    /// D2 — `**` never crosses an `@`-chunk (which is why the raw scope
    /// cannot see `@rpc`); D4 — `*` never matches a verbatim origin.
    #[test]
    fn a_convention_shaped_no_cites_the_rfc() {
        // D2: the media-safe scope really cannot see the plane.
        let RelationVerdict::No { note } = judge(
            crate::render::KeyOp::Includes,
            "v1/**",
            "v1/h-1/@rpc/p/introspect",
        ) else {
            panic!("** must not cross @rpc (D2)");
        };
        let note = note.expect("the convention explains this no");
        assert!(note.contains("D2"), "{note}");

        // D4: a wildcard origin position does not match a service origin.
        let RelationVerdict::No { note } = judge(
            crate::render::KeyOp::Intersects,
            "v1/*/state/x",
            "v1/@catalog/state/x",
        ) else {
            panic!("* must not match @catalog (D4)");
        };
        let note = note.expect("the convention explains this no");
        assert!(note.contains("D4"), "{note}");

        // A plain algebra no gets no citation — the note is a diagnosis,
        // not a banner.
        let RelationVerdict::No { note } = judge(crate::render::KeyOp::Intersects, "a/b", "a/c")
        else {
            panic!();
        };
        assert!(note.is_none());
    }
}
