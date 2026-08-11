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

use crate::output::Format;

/// The answer as an exit code: 0 yes, 1 no, 2 either expression invalid.
pub enum Verdict {
    Yes,
    No { note: Option<String> },
    Invalid { which: &'static str, error: String },
}

impl Verdict {
    pub fn exit_code(&self) -> i32 {
        match self {
            Verdict::Yes => 0,
            Verdict::No { .. } => 1,
            Verdict::Invalid { .. } => 2,
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
pub fn judge(op: &str, a: &str, b: &str) -> Verdict {
    let ka = match KeyExpr::new(a) {
        Ok(k) => k,
        Err(e) => {
            return Verdict::Invalid {
                which: "first",
                error: e.to_string(),
            };
        }
    };
    let kb = match KeyExpr::new(b) {
        Ok(k) => k,
        Err(e) => {
            return Verdict::Invalid {
                which: "second",
                error: e.to_string(),
            };
        }
    };
    let yes = match op {
        "includes" => ka.includes(&kb),
        _ => ka.intersects(&kb),
    };
    if yes {
        Verdict::Yes
    } else {
        Verdict::No {
            note: convention_note(a, b),
        }
    }
}

/// `key includes <a> <b>` / `key intersects <a> <b>`.
pub fn relate(op: &str, a: &str, b: &str, format: Format) -> Result<()> {
    let verdict = judge(op, a, b);
    let json = matches!(format.resolved(), Format::Json | Format::Ndjson);
    match &verdict {
        Verdict::Yes => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "op": op, "a": a, "b": b, "answer": true })
                );
            } else {
                println!(
                    "yes — {a} {}",
                    if op == "includes" {
                        format!("includes every key {b} names")
                    } else {
                        format!("and {b} can name a common key")
                    }
                );
            }
        }
        Verdict::No { note } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "op": op, "a": a, "b": b, "answer": false, "note": note })
                );
            } else {
                println!(
                    "no — {a} {}",
                    if op == "includes" {
                        format!("does not include all of {b}")
                    } else {
                        format!("and {b} share no key")
                    }
                );
                if let Some(n) = note {
                    println!("{n}");
                }
            }
        }
        Verdict::Invalid { which, error } => {
            // The grammar's own message, verbatim (the keyfacts rule).
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
pub fn canon(expr: &str, format: Format) -> Result<()> {
    match KeyExpr::autocanonize(expr.to_string()) {
        Ok(k) => {
            if matches!(format.resolved(), Format::Json | Format::Ndjson) {
                println!(
                    "{}",
                    serde_json::json!({ "input": expr, "canon": k.as_str(), "changed": k.as_str() != expr })
                );
            } else if k.as_str() == expr {
                println!("{expr} is already canonical");
            } else {
                println!("{}", k.as_str());
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("does not parse: {e}");
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
        assert!(matches!(judge("includes", "a/**", "a/b/c"), Verdict::Yes));
        assert!(matches!(
            judge("includes", "a/b/c", "a/**"),
            Verdict::No { .. }
        ));
        assert!(matches!(
            judge("intersects", "a/*/c", "a/b/*"),
            Verdict::Yes
        ));
        assert!(matches!(
            judge("intersects", "a/b", "a/c"),
            Verdict::No { .. }
        ));
        assert_eq!(judge("includes", "a/**", "a/b").exit_code(), 0);
        assert_eq!(judge("includes", "a/b", "a/c").exit_code(), 1);
        assert_eq!(judge("includes", "a//b", "a").exit_code(), 2);
    }

    /// The two footguns the convention leans on, cited when they bite:
    /// D2 — `**` never crosses an `@`-chunk (which is why the raw scope
    /// cannot see `@rpc`); D4 — `*` never matches a verbatim origin.
    #[test]
    fn a_convention_shaped_no_cites_the_rfc() {
        // D2: the media-safe scope really cannot see the plane.
        let Verdict::No { note } = judge("includes", "v1/**", "v1/h-1/@rpc/p/introspect") else {
            panic!("** must not cross @rpc (D2)");
        };
        let note = note.expect("the convention explains this no");
        assert!(note.contains("D2"), "{note}");

        // D4: a wildcard origin position does not match a service origin.
        let Verdict::No { note } = judge("intersects", "v1/*/state/x", "v1/@catalog/state/x")
        else {
            panic!("* must not match @catalog (D4)");
        };
        let note = note.expect("the convention explains this no");
        assert!(note.contains("D4"), "{note}");

        // A plain algebra no gets no citation — the note is a diagnosis,
        // not a banner.
        let Verdict::No { note } = judge("intersects", "a/b", "a/c") else {
            panic!();
        };
        assert!(note.is_none());
    }
}
