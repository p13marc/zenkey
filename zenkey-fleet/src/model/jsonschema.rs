//! The bit of JSON Schema this crate has to walk itself (#384).
//!
//! Two consumers read a served schema document structurally rather than
//! handing it to the validator: [`crate::judge::field`]'s declared-path
//! surface, and [`crate::tape::synth`]'s instance synthesizer. Both hit the
//! same two shapes, because both are handed whatever `schemars` emits —
//! combinators (`oneOf`/`anyOf`/`allOf`, which is how a Rust sum type
//! reaches the wire) and `$ref` into `$defs` (which is where every nested
//! named type goes).
//!
//! Neither walker is a JSON Schema implementation and neither should become
//! one — validation is `zenkey::schema::validate`'s job, against the real
//! `jsonschema` crate. What lives here is only the pointer resolution both
//! walkers need, kept in one place so they cannot disagree about what a
//! `$ref` means.

use serde_json::Value;

/// The combinator keywords whose branches a structural walk must consider.
/// `allOf` composes, `oneOf`/`anyOf` alternate; for both the *union* of the
/// branches is what the document declares.
pub const COMBINATORS: [&str; 3] = ["oneOf", "anyOf", "allOf"];

/// Resolve a same-document JSON Pointer `$ref` (RFC 6901) against the root.
///
/// Only same-document pointers resolve — `#`, `#/$defs/X`, `#/definitions/X`.
/// An external `$ref` names a document the walker was never handed: the
/// served schema is one payload (RFC 08 §7), and inventing a fetch for it
/// would put a network call inside a pure walk.
///
/// `None` means **could not follow**, which every caller must render as
/// "unjudgeable" rather than "absent" — a `$ref` that does not resolve says
/// nothing about what is underneath it (RFC 13 §3 O4).
pub fn resolve_ref<'d>(root: &'d Value, pointer: &str) -> Option<&'d Value> {
    let rest = pointer.strip_prefix('#')?;
    if rest.is_empty() {
        return Some(root);
    }
    let mut node = root;
    for token in rest.strip_prefix('/')?.split('/') {
        // `~1` before `~0`, per RFC 6901 §3 — the other order turns an
        // escaped tilde into a slash.
        let token = token.replace("~1", "/").replace("~0", "~");
        node = match node {
            Value::Object(map) => map.get(&token)?,
            Value::Array(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pointers_resolve_and_unfollowable_ones_say_so() {
        let root = json!({
            "$defs": {"A": {"type": "string"}, "a/b": {"type": "number"}, "c~d": {"x": 1}},
            "arr": [{"first": true}],
        });
        assert_eq!(resolve_ref(&root, "#"), Some(&root), "the root itself");
        assert_eq!(
            resolve_ref(&root, "#/$defs/A"),
            Some(&json!({"type": "string"}))
        );
        assert_eq!(
            resolve_ref(&root, "#/$defs/a~1b"),
            Some(&json!({"type": "number"})),
            "~1 is an escaped slash, not a path separator"
        );
        assert_eq!(resolve_ref(&root, "#/$defs/c~0d"), Some(&json!({"x": 1})));
        assert_eq!(
            resolve_ref(&root, "#/arr/0"),
            Some(&json!({"first": true})),
            "array indices are pointer tokens too"
        );

        for unfollowable in [
            "#/$defs/Missing",              // dangling
            "https://example.invalid/s#/A", // external document
            "$defs/A",                      // not a fragment
            "#/$defs/A/nope",               // through a scalar
            "#/arr/9",                      // past the end
        ] {
            assert_eq!(
                resolve_ref(&root, unfollowable),
                None,
                "{unfollowable} must read as could-not-follow"
            );
        }
    }
}
