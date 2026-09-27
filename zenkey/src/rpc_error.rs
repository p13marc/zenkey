//! The `@rpc` error vocabulary (RFC 05 §3): the names every conforming caller
//! understands, and the shape of a producer's own.
//!
//! A failure rides Zenoh's reply-error channel as `{ "error": "<name>",
//! "message": "…" }`. The convention reserves the names below; a producer's
//! own live under `error/<producer>/…` and are declared as `[[error]]`
//! entries in its registry (RFC 08 §2, v1.40) — linted, pinned, served by
//! `introspect` and retired through `[[deprecated]]` like a subject. This
//! module is the runtime half: the reserved list, the collision check the
//! lint uses, and the spelling of a producer's name on the wire.

/// The reserved names (RFC 05 §3), in the order the RFC lists them.
pub const RESERVED: [&str; 7] = [
    "error/invalid-args",
    "error/unauthorized",
    "error/not-found",
    "error/unsupported",
    "error/busy",
    "error/gated",
    "error/fanout-forbidden",
];

/// Whether `name` is one of [`RESERVED`].
#[must_use]
pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

/// The wire name of a producer's own error: `error/<producer>/<name>`.
///
/// `name` is the `[[error]]` entry's `name` column; the producer prefix is
/// what keeps two producers' `not-ready` apart, and what an ACL or a
/// consumer can match on.
#[must_use]
pub fn producer_error(producer: &str, name: &str) -> String {
    format!("error/{producer}/{name}")
}

/// Whether `name` is a well-formed `[[error]]` name: one or more of
/// `[a-z0-9-]`, neither starting nor ending with `-`. The same charset as a
/// plain key chunk, because the wire name is spelled like a key (RFC 05 §3).
#[must_use]
pub fn is_valid_error_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reserved_list_is_the_rfcs() {
        assert_eq!(RESERVED.len(), 7);
        assert!(is_reserved("error/gated"));
        assert!(is_reserved("error/fanout-forbidden"));
        assert!(!is_reserved("error/modem/restart-required"));
    }

    #[test]
    fn a_producer_error_is_namespaced_like_a_key() {
        assert_eq!(
            producer_error("modem", "restart-required"),
            "error/modem/restart-required"
        );
    }

    #[test]
    fn error_names_are_plain_chunks() {
        for ok in ["restart-required", "device-refused", "busy2"] {
            assert!(is_valid_error_name(ok), "{ok}");
        }
        for bad in ["", "-x", "x-", "Restart", "a/b", "a b", "a_b"] {
            assert!(!is_valid_error_name(bad), "{bad}");
        }
    }
}
