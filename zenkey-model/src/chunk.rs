//! Chunk lexical rules (core §1.2; r3 §3.1, inherited from v1's).
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

/// Crockford's base32 in lowercase (core §1.2): the digits, then the
/// letters but `i`, `l`, `o` and `u`.
const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A Crockford base32 character's value, `None` outside the lowercase
/// alphabet.
fn crockford(c: u8) -> Option<u64> {
    CROCKFORD
        .iter()
        .position(|&a| a == c)
        .and_then(|i| u64::try_from(i).ok())
}

/// Whether `s` is an occurrence chunk (core §1.2; #980's C2a):
/// [`occurrence`] reads a stamp from it.
#[must_use]
pub fn is_occurrence(s: &str) -> bool {
    occurrence(s).is_some()
}

/// The run and the seq an occurrence chunk names (core §1.2, §2.6; #980's
/// C2a): run ‖ seq in Crockford's base32 in lowercase, most significant
/// first, zero-filled. 8 characters hold a 16-bit run's 40 bits, and 12 a
/// 32-bit run's 56, the first of them `0` or `1`. `None` for any other
/// chunk, a ULID's included. The decode is exact: every chunk it accepts
/// is [`write_occurrence`]'s for its stamp.
#[must_use]
pub fn occurrence(s: &str) -> Option<crate::wire::Stamp> {
    use crate::wire::{Run, Seq, Stamp};
    let bits = match s.len() {
        8 => 40,
        12 => 56,
        _ => return None,
    };
    let mut v: u64 = 0;
    for &c in s.as_bytes() {
        v = (v << 5) | crockford(c)?;
    }
    if v >> bits != 0 {
        return None;
    }
    let seq = Seq::new(u32::try_from(v & 0x00ff_ffff).ok()?)?;
    let high = v >> 24;
    let run = if bits == 40 {
        Run::U16(u16::try_from(high).ok()?)
    } else {
        Run::U32(u32::try_from(high).ok()?)
    };
    Some(Stamp { run, seq })
}

/// The occurrence chunk of `stamp` (core §1.2): 8 characters for a 16-bit
/// run, 12 for a 32-bit one.
#[must_use]
pub fn write_occurrence(stamp: crate::wire::Stamp) -> String {
    let n = if stamp.run.is_wide() { 12 } else { 8 };
    let mut v = (u64::from(stamp.run.value()) << 24) | u64::from(stamp.seq.get());
    let mut out = vec![b'0'; n];
    for c in out.iter_mut().rev() {
        *c = CROCKFORD[usize::try_from(v & 31).unwrap_or(0)];
        v >>= 5;
    }
    String::from_utf8(out).unwrap_or_default()
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
    fn hex_occurrence_ident() {
        assert!(is_lower_hex("8f3a5c2e9b1d4f70", 16));
        assert!(!is_lower_hex("8F3A5C2E9B1D4F70", 16));
        assert!(!is_lower_hex("8f3a", 16));
        assert!(!is_occurrence("01jgxqz4yqk8v6txw3m9f2a7cd"), "a ULID");
        assert!(is_ident("set_origin"));
        // An occurrence chunk (core §1.2; #980's C2a).
        use crate::wire::{Run, Seq, Stamp};
        let at = |run, seq| Stamp {
            run,
            seq: Seq::new(seq).unwrap(),
        };
        assert_eq!(occurrence("002g0009"), Some(at(Run::U16(5), 9)));
        assert_eq!(
            occurrence("1qndqvqg0009"),
            Some(at(Run::U32(0xdead_beef), 9))
        );
        assert_eq!(occurrence("2qndqvqg0009"), None, "57 bits");
        assert_eq!(occurrence("002G0009"), None, "uppercase");
        assert_eq!(occurrence("002i0009"), None, "outside the alphabet");
        assert_eq!(occurrence("01jgxqz4yqk8v6txw3m9f2a7cd"), None, "a ULID");
        for stamp in [
            at(Run::U16(0), 0),
            at(Run::U16(0xffff), 0x00ff_ffff),
            at(Run::U32(0x1234_5678), 0x009a_bcde),
            at(Run::U32(u32::MAX), 0x00ff_ffff),
        ] {
            let chunk = write_occurrence(stamp);
            assert!(is_plain_chunk(&chunk), "{chunk}");
            assert_eq!(occurrence(&chunk), Some(stamp), "{chunk}");
        }
        assert_eq!(
            write_occurrence(at(Run::U32(0x1234_5678), 0x009a_bcde)),
            "04hmasw9nf6y"
        );
        assert!(!is_ident("Set"));
        assert!(!is_ident("1a"));
        assert!(!is_ident("a-b"));
    }
}
