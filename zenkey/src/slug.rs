//! Canonical, injective slugging of foreign values into key chunks (RFC 03 §2).
//!
//! Rules:
//! - **IPs are always slugged**, even charset-legal dotted IPv4 (dotted forms
//!   are non-canonical chunks). IPv6 is canonicalized per RFC 5952 and IPv4 to
//!   minimal dotted-quad first, then `.`/`:` → `-`.
//! - Other values (unit names, filenames, config names) stay literal when
//!   already legal per the chunk charset; otherwise each excluded character is
//!   escaped losslessly as `_xNN_` (lowercase hex of the byte). Plain `-`
//!   substitution is forbidden — it is not injective.

use std::net::IpAddr;

use crate::grammar::is_valid_plain_chunk;

/// Slug an IP address (RFC 03 §2). `std`'s `Display` for `Ipv6Addr` is
/// RFC 5952-conformant (lowercase, `::` compression), so parsing + formatting
/// *is* the canonicalization.
pub fn ip_slug(ip: IpAddr) -> String {
    ip.to_string().replace(['.', ':'], "-")
}

/// Parse-and-slug a textual IP; returns `None` when the text is not an IP
/// (callers then fall back to [`chunk_slug`] for hostnames).
pub fn ip_slug_str(text: &str) -> Option<String> {
    text.parse::<IpAddr>().ok().map(ip_slug)
}

/// Lowercase a ULID-shaped identifier for key encoding — or refuse.
///
/// RFC 03 §2: identifiers whose canonical text form is uppercase MUST be
/// **lowercased at key-build time** where the domain is case-insensitive —
/// in particular ULIDs, whose Crockford base32 decodes case-insensitively
/// (payloads MAY keep the canonical uppercase display form). Lowercasing a
/// ULID therefore names the *same* identifier in its one key spelling;
/// escaping its uppercase bytes (as [`chunk_slug`] would) mints a
/// different chunk nobody serves.
///
/// Returns `Some(lowercased)` when `id` is ULID-shaped — 26 Crockford
/// base32 characters (`0-9`, `A-Z` without `I`/`L`/`O`/`U`), either case —
/// and `None` otherwise, so a caller refuses or falls back deliberately
/// instead of lowercasing a value from a domain that might be
/// case-sensitive (the v1.4 exemption).
///
/// This is also the `events` id path (RFC 04 §1.3): every event key ends in
/// a unique, time-sortable id — ULID recommended, key-encoded lowercase —
/// and this function *is* that encoding. Route an event id through it and
/// the trailing chunk of `events/<producer>/…/<ulid>` is the RFC's one
/// spelling; a refusal (`None`) means the id was never a ULID, which a
/// producer should treat as its own bug, not a value to escape.
pub fn ulid_slug(id: &str) -> Option<String> {
    fn crockford(b: u8) -> bool {
        let b = b.to_ascii_uppercase();
        b.is_ascii_digit() || (b.is_ascii_uppercase() && !matches!(b, b'I' | b'L' | b'O' | b'U'))
    }
    (id.len() == 26 && id.bytes().all(crockford)).then(|| id.to_ascii_lowercase())
}

/// Slug an arbitrary value (unit name, filename, device name) into a single
/// legal chunk, losslessly (RFC 03 §2).
///
/// Boundary handling follows the v1.4 erratum: a chunk must start and end
/// alphanumeric, so a charset-legal-but-non-alphanumeric byte (`.`, `_`, `-`)
/// at either boundary is escaped like any illegal byte, and when the escaped
/// form still leads (or ends) with the escape's own `_`, the reserved marker
/// `x` is affixed on that side — `_myns` → `x_x5f_myns`, the RFC's example.
/// The marker is part of the injective encoding: it appears only next to an
/// `_xNN_` escape, so it never collides with a value that starts with `x`
/// literally (the old sentinel-only form did collide: `_myns` and `e_myns`
/// shared a chunk).
pub fn chunk_slug(value: &str) -> String {
    if is_valid_plain_chunk(value) {
        return value.to_string();
    }
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len() + 8);
    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
        let alnum = c.is_ascii_lowercase() || c.is_ascii_digit();
        let legal_inner = alnum || c == '.' || c == '_' || c == '-';
        let boundary = i == 0 || i == bytes.len() - 1;
        if legal_inner && (alnum || !boundary) {
            out.push(c);
        } else {
            out.push_str(&format!("_x{b:02x}_"));
        }
    }
    // `_xNN_` starts and ends with `_`, which the charset forbids at
    // boundaries; the erratum's marker makes the boundary alphanumeric in
    // one pass instead of re-escaping forever.
    if out.starts_with('_') {
        out.insert(0, 'x');
    }
    if out.ends_with('_') {
        out.push('x');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ipv4_always_slugged() {
        assert_eq!(ip_slug_str("10.0.0.7").unwrap(), "10-0-0-7");
        assert_eq!(ip_slug_str("93.184.216.34").unwrap(), "93-184-216-34");
    }

    #[test]
    fn ipv6_rfc5952_canonical_before_slugging() {
        // Two spellings of one address MUST slug identically (RFC 03 §2).
        let a = ip_slug_str("2001:db8::1").unwrap();
        let b = ip_slug_str("2001:db8:0:0:0:0:0:1").unwrap();
        let c = ip_slug_str("2001:DB8::1").unwrap();
        assert_eq!(a, "2001-db8--1");
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn legal_values_stay_literal() {
        assert_eq!(chunk_slug("sshd.service"), "sshd.service");
        assert_eq!(chunk_slug("cam0"), "cam0");
    }

    #[test]
    fn escape_is_injective() {
        // The RFC's motivating counterexample: these MUST NOT share a chunk.
        let a = chunk_slug("foo@1.service");
        let b = chunk_slug("foo-1.service");
        assert_ne!(a, b);
        assert_eq!(a, "foo_x40_1.service");

        // Property check over a corpus of near-collisions.
        let corpus = [
            "getty@tty1.service",
            "getty-tty1.service",
            "a b",
            "a_b",
            "a-b",
            "A",
            "a",
            "Ab",
            "a.b",
            ".ab",
            "ab.",
            "café",
            "unit@.service",
            // The v1.4 erratum's collision pair: the old sentinel-only
            // boundary fix mapped `_myns` to `e_myns`, colliding with the
            // literal value `e_myns`.
            "_myns",
            "e_myns",
            "x_myns",
            "myns_",
            "_",
        ];
        let slugs: Vec<String> = corpus.iter().map(|v| chunk_slug(v)).collect();
        let unique: HashSet<&String> = slugs.iter().collect();
        assert_eq!(unique.len(), corpus.len(), "collision in {slugs:?}");
        for s in &slugs {
            assert!(
                crate::grammar::is_valid_plain_chunk(s),
                "illegal slug {s:?}"
            );
        }
    }

    /// RFC 03 §2: a ULID is key-encoded in lowercase — both cases of one
    /// ULID name one chunk, and non-ULID shapes are refused rather than
    /// guessed at.
    #[test]
    fn ulid_shapes_lowercase_and_others_refuse() {
        let canonical = "01JGXQZ4YQK8V6TXW3M9F2A7CD";
        assert_eq!(
            ulid_slug(canonical).as_deref(),
            Some("01jgxqz4yqk8v6txw3m9f2a7cd")
        );
        assert_eq!(
            ulid_slug("01jgxqz4yqk8v6txw3m9f2a7cd").as_deref(),
            Some("01jgxqz4yqk8v6txw3m9f2a7cd"),
            "already-lowercase is the fixed point"
        );
        // Not ULID-shaped: wrong length, excluded letters, illegal bytes.
        assert_eq!(ulid_slug("01HQXK8F9C2N4PZQ"), None, "16 chars");
        assert_eq!(ulid_slug("01JGXQZ4YQK8V6TXW3M9F2A7CI"), None, "I excluded");
        assert_eq!(ulid_slug("01jgxqz4yqk8v6txw3m9f2a7c."), None);
        assert_eq!(ulid_slug(""), None);
    }

    /// RFC 04 §1.3's event-id recommendation rides this same function: a
    /// canonical-uppercase ULID slugs to the trailing chunk of an events
    /// key, verbatim to the RFC 11 §2 worked example.
    #[test]
    fn event_ids_are_ulid_slugs() {
        let id = ulid_slug("01JGXQZ4YQK8V6TXW3M9F2A7CD").unwrap();
        let key = crate::grammar::data_key(
            &crate::grammar::Origin::Host(crate::origin::HostId::parse("h-3fa9c2d41b7e").unwrap()),
            crate::grammar::Class::Events,
            Some(&crate::grammar::Producer::new("netring").unwrap()),
            &["capture", &id],
        )
        .unwrap();
        assert_eq!(
            key,
            "v1/h-3fa9c2d41b7e/events/netring/capture/01jgxqz4yqk8v6txw3m9f2a7cd"
        );
    }

    /// The v1.4 erratum, by its own example: escaping must converge to an
    /// alphanumeric first character, and the `x` marker is affixed *with* the
    /// boundary byte escaped — not instead of escaping it (RFC 03 §2).
    #[test]
    fn erratum_boundary_escape_is_the_rfcs() {
        assert_eq!(chunk_slug("_myns"), "x_x5f_myns");
        // The collision the erratum exists to prevent: a `_`-leading value
        // and the literal spelling of the old sentinel form stay distinct.
        assert_ne!(chunk_slug("_myns"), chunk_slug("e_myns"));
        assert_eq!(chunk_slug("e_myns"), "e_myns");
        // The trailing boundary converges the same way.
        assert_eq!(chunk_slug("myns_"), "myns_x5f_x");
        assert_eq!(chunk_slug(".ab"), "x_x2e_ab");
    }
}
