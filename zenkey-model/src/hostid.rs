//! `hostid.v1`: a system name minted from the machine id
//! (`spec/profiles/hostid/v1.md`, #719).
//!
//! The derivation-only profile of core §10 (0.19): it defines no contract,
//! annotation or kind, only how an owner derives its system from a machine
//! id, so that system = host (core §1.5). This module is its session-free
//! half, the pure derivation of §2.1:
//!
//! ```text
//! norm   = lower(trim(input))        32 hex digits, not all zeros, or refused
//! system = "h-" ++ hex(sha256(utf8(norm) ++ utf8(salt)))[0..12]
//! ```
//!
//! There is no file I/O here. Reading the inputs in order (§2.4), the
//! shared file (§2.5), failing closed and the `ephemeral` opt-in (§2.6)
//! belong to the runtime, which calls [`system`] on what it reads.

use sha2::{Digest, Sha256};

use crate::chunk::is_lower_hex;
use crate::grammar::Name;

/// The one salt of `hostid.v1` (§2.2; architecture D26). Every system is
/// derived with it, so a machine is one system across every zk2
/// application. It is not configurable: [`derive()`] takes a salt only so
/// that the test vectors and the v1 migration table (Appendix B) can be
/// computed.
pub const SALT: &str = "zk2-hostid-v1";

/// The prefix of a minted system (§2.1).
pub const PREFIX: &str = "h-";

/// How many hex digits of the digest a system keeps: 48 bits (§2.1, §2.12).
pub const DIGITS: usize = 12;

/// Normalises a machine id (§2.1 steps 1–2), or refuses it.
///
/// Trims the five ASCII whitespace bytes (TAB, LF, FF, CR, SPACE) from both
/// ends, and no other character: not 0x0B, as Python's `strip()` would, and
/// not Unicode spaces, as Rust's `str::trim` would. Then lowercases `A–Z`,
/// and nothing else (no Unicode case folding). The check is on characters,
/// never a number parse, which would accept a sign.
/// The result must be exactly 32 hex digits and not all zeros (an all-zero
/// id is not a machine id, machine-id(5)). Anything else is refused with
/// `None`: absent, empty, systemd's `uninitialized`, a UUID with hyphens,
/// 31 or 33 digits, a non-hex digit (§2.4 then tries the next input).
#[must_use]
pub fn normalise(input: &str) -> Option<String> {
    // `str::trim_ascii` trims exactly `u8::is_ascii_whitespace`: the five
    // bytes above.
    let norm = input.trim_ascii().to_ascii_lowercase();
    (is_lower_hex(&norm, 32) && norm.bytes().any(|b| b != b'0')).then_some(norm)
}

/// Derives a system from a machine id with `salt` (§2.1), or refuses the
/// input ([`normalise`]).
///
/// The system is `h-` and the first 12 lowercase hex digits of
/// `sha256(norm ++ salt)`, both UTF-8, with no separator. A deployment
/// always derives with [`SALT`] ([`system`]); another salt is for the
/// vectors and the migration table only (§2.2).
#[must_use]
pub fn derive(machine_id: &str, salt: &str) -> Option<String> {
    let norm = normalise(machine_id)?;
    let digest = Sha256::new()
        .chain_update(norm.as_bytes())
        .chain_update(salt.as_bytes())
        .finalize();
    let mut system = String::with_capacity(PREFIX.len() + DIGITS);
    system.push_str(PREFIX);
    for b in &digest[..DIGITS / 2] {
        system.push(char::from_digit(u32::from(b >> 4), 16).expect("a nibble"));
        system.push(char::from_digit(u32::from(b & 0xf), 16).expect("a nibble"));
    }
    Some(system)
}

/// The system `hostid.v1` mints from a machine id, with the one salt
/// ([`SALT`]), or `None` when the input is refused (§2.1, §2.4).
///
/// A minted system is a plain chunk (core §1.2) and its own slug (core
/// §1.4), so it is a system [`Name`] as it stands.
#[must_use]
pub fn system(machine_id: &str) -> Option<Name> {
    let s = derive(machine_id, SALT)?;
    Some(Name::new("system", &s).expect("h- and 12 hex digits is a plain chunk"))
}

/// Whether `s` is in the minted shape: exactly `h-` followed by 12
/// lowercase hex digits, the whole string (§2.11).
///
/// The shape is a hint, never proof: a deployment may name a system in it
/// literally. Only the descriptor's `profiles` establishes a minted system
/// (§2.8, §5).
#[must_use]
pub fn is_minted_shape(s: &str) -> bool {
    s.strip_prefix(PREFIX)
        .is_some_and(|hex| is_lower_hex(hex, DIGITS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slug::chunk_slug;

    const MID: &str = "b642b4217b34b1e8d3bd915fc65c4452";

    /// v1's RFC 06 §1 vector, which the construction keeps (Appendix A).
    #[test]
    fn rfc06_vector() {
        assert_eq!(
            derive(MID, "example-salt-v1").as_deref(),
            Some("h-20609002f7b6")
        );
    }

    /// The same machine id under the one salt (§2.2).
    #[test]
    fn zk2_vector() {
        assert_eq!(derive(MID, SALT).as_deref(), Some("h-bbd1aa1db10b"));
        assert_eq!(system(MID).unwrap().as_str(), "h-bbd1aa1db10b");
    }

    #[test]
    fn normalisation_trims_five_bytes_and_lowercases() {
        for input in [
            "b642b4217b34b1e8d3bd915fc65c4452\n",
            "b642b4217b34b1e8d3bd915fc65c4452\r\n",
            "  b642b4217b34b1e8d3bd915fc65c4452\t ",
            "\x0cb642b4217b34b1e8d3bd915fc65c4452\x0c",
            "B642B4217B34B1E8D3BD915FC65C4452",
        ] {
            assert_eq!(normalise(input).as_deref(), Some(MID), "{input:?}");
        }
    }

    #[test]
    fn refused_inputs_yield_nothing() {
        for input in [
            "",
            "\n",
            "uninitialized\n",
            "00000000000000000000000000000000",
            "b642b4217b34b1e8d3bd915fc65c445",
            "b642b4217b34b1e8d3bd915fc65c44520",
            "b642b4217b34b1e8d3bd915fc65c445g",
            "b642b421-7b34-b1e8-d3bd-915fc65c4452",
            "b642b4217b34b1e8 d3bd915fc65c4452",
            "\x0bb642b4217b34b1e8d3bd915fc65c4452\x0b",
            "\u{a0}b642b4217b34b1e8d3bd915fc65c4452\u{a0}",
            "+642b4217b34b1e8d3bd915fc65c4452",
            "b642b4217b34b1e8_3bd915fc65c4452",
            "b642b4217b34b1e8d3bd915fc65c44\u{fb00}",
        ] {
            assert_eq!(normalise(input), None, "{input:?}");
            assert_eq!(derive(input, SALT), None, "{input:?}");
            assert_eq!(system(input), None, "{input:?}");
        }
    }

    #[test]
    fn a_minted_system_is_a_plain_chunk_and_its_own_slug() {
        for input in [MID, "00000000000000000000000000000001"] {
            let s = system(input).unwrap();
            assert_eq!(s.as_str().len(), 14);
            assert!(is_minted_shape(s.as_str()));
            assert_eq!(chunk_slug(s.as_str()), s.as_str());
        }
    }

    #[test]
    fn the_shape() {
        for yes in ["h-20609002f7b6", "h-000000000000"] {
            assert!(is_minted_shape(yes), "{yes}");
        }
        for no in [
            "H-20609002f7b6",
            "h-20609002F7B6",
            "h-20609002f7b",
            "h-20609002f7b6a",
            "h_20609002f7b6",
            "20609002f7b6",
            "h-20609002f7b6\n",
            "",
            "vehicle-01",
        ] {
            assert!(!is_minted_shape(no), "{no:?}");
        }
    }
}
