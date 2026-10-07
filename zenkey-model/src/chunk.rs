//! Chunk lexical rules (r3 §3.1, inherited from v1's RFC 03 §2).
//!
//! A *plain* chunk is `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`: lowercase ASCII
//! letters, digits, `.`, `_` and `-`, starting and ending alphanumeric.
//! System names, service names, literal template chunks and slugged
//! parameter values are all plain chunks. Verbatim chunks (`@…`) are never
//! user-supplied in zk2: only the reserved tokens `@stream`, `@state`,
//! `@op` and `@zk` use them.

/// Whether `s` is a legal plain chunk.
#[must_use]
pub fn is_plain_chunk(s: &str) -> bool {
    let b = s.as_bytes();
    let Some((&first, rest)) = b.split_first() else {
        return false;
    };
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    if !alnum(first) {
        return false;
    }
    match rest.split_last() {
        None => true,
        Some((&last, middle)) => {
            alnum(last)
                && middle
                    .iter()
                    .all(|&c| alnum(c) || matches!(c, b'.' | b'_' | b'-'))
        }
    }
}

/// Whether `s` is `n` lowercase hex digits.
#[must_use]
pub fn is_lower_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

/// Whether `s` is a lowercase ULID: 26 Crockford base32 characters
/// (`0-9`, `a-z` without `i`, `l`, `o`, `u`).
#[must_use]
pub fn is_lower_ulid(s: &str) -> bool {
    s.len() == 26
        && s.bytes().all(|c| {
            c.is_ascii_digit()
                || (c.is_ascii_lowercase() && !matches!(c, b'i' | b'l' | b'o' | b'u'))
        })
}

/// Whether `s` is an identifier as the authoring format uses them for
/// parameter names, role names and interface-name segments:
/// `[a-z][a-z0-9_]*`.
#[must_use]
pub fn is_ident(s: &str) -> bool {
    let b = s.as_bytes();
    matches!(b.first(), Some(c) if c.is_ascii_lowercase())
        && b.iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_chunks() {
        for ok in [
            "a",
            "0",
            "cam0",
            "sshd.service",
            "a-b",
            "a_b",
            "h-3fa9c2d41b7e",
            "snmp.router01",
        ] {
            assert!(is_plain_chunk(ok), "{ok}");
        }
        for bad in [
            "", "A", "-a", "a-", ".a", "a.", "_a", "a_", "a b", "@zk", "a/b", "*", "é",
        ] {
            assert!(!is_plain_chunk(bad), "{bad}");
        }
    }

    #[test]
    fn hex_ulid_ident() {
        assert!(is_lower_hex("8f3a5c2e9b1d4f70", 16));
        assert!(!is_lower_hex("8F3A5C2E9B1D4F70", 16));
        assert!(!is_lower_hex("8f3a", 16));
        assert!(is_lower_ulid("01jgxqz4yqk8v6txw3m9f2a7cd"));
        assert!(!is_lower_ulid("01JGXQZ4YQK8V6TXW3M9F2A7CD"));
        assert!(!is_lower_ulid("01jgxqz4yqk8v6txw3m9f2a7ci"));
        assert!(is_ident("set_origin"));
        assert!(!is_ident("Set"));
        assert!(!is_ident("1a"));
        assert!(!is_ident("a-b"));
    }
}
