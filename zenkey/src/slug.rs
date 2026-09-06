//! Canonical, injective slugging of foreign values into key chunks (RFC 03 §2).
//!
//! Rules:
//! - **IPs are always slugged**, even charset-legal dotted IPv4 (dotted forms
//!   are non-canonical chunks). IPv6 is canonicalized per RFC 5952 and IPv4 to
//!   minimal dotted-quad first, then `.`/`:` → `-`.
//! - Other values (unit names, filenames, device names) stay literal when
//!   already legal per the chunk charset *and* not starting with the reserved
//!   prefix `x-`; otherwise the chunk is `x-` plus an escaped body in which
//!   every byte outside `[a-z0-9]` is written `_xHH` (lowercase hex, no
//!   closing underscore), except that `.` and `-` stay literal unless they
//!   are the value's last byte. Plain `-` substitution is forbidden — it is
//!   not injective. [`chunk_unslug`] is the decoder, shipped beside the
//!   encoder as the RFC requires.

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

/// The reserved prefix (RFC 03 §2, v1.31): refused on passthrough,
/// mandatory on escape.
const RESERVED_PREFIX: &str = "x-";

/// Slug an arbitrary value (unit name, filename, device name) into a single
/// legal chunk, losslessly (RFC 03 §2, as amended by the v1.31 erratum).
///
/// The value passes through literally iff it is already a legal plain chunk
/// **and** does not start with the reserved prefix `x-`. Otherwise the chunk
/// is `x-` followed by the escaped body: `[a-z0-9]` literal, `.` and `-`
/// literal unless they are the value's last byte, every other byte
/// (`_` included) as `_xHH` — lowercase hex, no closing underscore. The
/// empty value is spelled `x-_x`.
///
/// Injectivity, in two sentences. A passthrough never starts with `x-` and
/// an escaped chunk always does, so the two classes cannot meet; within the
/// escaped class `_` is never literal, so every `_` opens exactly one
/// `_xHH` escape and the body decodes left to right without lookahead.
/// The v1.4 scheme this replaces failed on both counts — its escaped output
/// was itself passthrough-legal (`_myns` and the literal `x_x5f_myns`), and
/// its leading marker `x` was also a legal first byte (`x@b` and `@b`) — see
/// the erratum for the record. [`chunk_unslug`] is the decoder.
///
/// ```
/// use zenkey::slug::chunk_slug;
/// assert_eq!(chunk_slug("sshd.service"), "sshd.service");
/// assert_eq!(chunk_slug("foo@1.service"), "x-foo_x401.service");
/// assert_eq!(chunk_slug("_myns"), "x-_x5fmyns");
/// assert_eq!(chunk_slug("x-foo"), "x-x-foo");
/// assert_eq!(chunk_slug(""), "x-_x");
/// ```
pub fn chunk_slug(value: &str) -> String {
    if is_valid_plain_chunk(value) && !value.starts_with(RESERVED_PREFIX) {
        return value.to_string();
    }
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len() + 8);
    out.push_str(RESERVED_PREFIX);
    if bytes.is_empty() {
        // `x-` alone would end in `-`; the RFC spells the empty value `x-_x`,
        // an escape with no digits that is legal only as the entire body.
        out.push_str("_x");
        return out;
    }
    let last = bytes.len() - 1;
    for (i, &b) in bytes.iter().enumerate() {
        let literal =
            b.is_ascii_lowercase() || b.is_ascii_digit() || ((b == b'.' || b == b'-') && i != last);
        if literal {
            out.push(b as char);
        } else {
            out.push_str(&format!("_x{b:02x}"));
        }
    }
    out
}

/// Decode a chunk produced by [`chunk_slug`] back to the value (RFC 03 §2).
///
/// This is the left inverse on the image of `chunk_slug` and refuses
/// everything else: `chunk_unslug(&chunk_slug(v)) == Some(v)` for every
/// `v`, and `chunk_unslug(c)` is `None` for any `c` that `chunk_slug` could
/// not have produced — a chunk that is neither a passthrough-legal plain
/// chunk nor `x-` plus a well-formed body (`_` must open `_xHH` with two
/// lowercase hex digits; `_x` alone is the empty value), a body whose
/// bytes are not UTF-8, or a non-canonical spelling (`x-abc`, `x-`, an
/// uppercase hex digit) that decodes to a value which slugs differently.
/// The canonicality check is what makes the function one-to-one on its
/// domain rather than merely a parser.
///
/// ```
/// use zenkey::slug::chunk_unslug;
/// assert_eq!(chunk_unslug("sshd.service").as_deref(), Some("sshd.service"));
/// assert_eq!(chunk_unslug("x-foo_x401.service").as_deref(), Some("foo@1.service"));
/// assert_eq!(chunk_unslug("x-_x").as_deref(), Some(""));
/// assert_eq!(chunk_unslug("x-abc"), None, "abc would pass through");
/// assert_eq!(chunk_unslug("Foo"), None, "not a chunk at all");
/// ```
#[must_use]
pub fn chunk_unslug(chunk: &str) -> Option<String> {
    let Some(body) = chunk.strip_prefix(RESERVED_PREFIX) else {
        return is_valid_plain_chunk(chunk).then(|| chunk.to_string());
    };
    let decoded = if body == "_x" {
        String::new()
    } else {
        let bytes = body.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'_' {
                let hex = bytes.get(i + 1..i + 4)?;
                if hex[0] != b'x'
                    || !hex[1..]
                        .iter()
                        .all(|h| matches!(h, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return None;
                }
                let hi = (hex[1] as char).to_digit(16)? as u8;
                let lo = (hex[2] as char).to_digit(16)? as u8;
                out.push((hi << 4) | lo);
                i += 4;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).ok()?
    };
    (chunk_slug(&decoded) == chunk).then_some(decoded)
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

    /// The corpus every slug property is checked over: the RFC's motivating
    /// counterexamples, boundary bytes, both v1.31 collision pairs, and the
    /// shapes that sit on the edge of the reserved prefix.
    const CORPUS: &[&str] = &[
        "foo@1.service",
        "foo-1.service",
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
        "_myns",
        "e_myns",
        "x_myns",
        "myns_",
        "_",
        "x-foo",
        "x_x5f_myns",
        "x@b",
        "@b",
        "a_",
        "a_x",
        "",
        "x-",
        "x",
    ];

    #[test]
    fn legal_values_stay_literal() {
        assert_eq!(chunk_slug("sshd.service"), "sshd.service");
        assert_eq!(chunk_slug("cam0"), "cam0");
        // Legal spellings that *look* like escapes are values in their own
        // right and pass through — the escaped class is `x-`-prefixed, so
        // nothing escaped can ever land on them.
        assert_eq!(chunk_slug("x_x5f_myns"), "x_x5f_myns");
        assert_eq!(chunk_slug("a_x"), "a_x");
    }

    /// RFC 03 §2 (v1.31): a charset-legal value that starts with `x-` is
    /// escaped, not passed through — the prefix is reserved on both sides of
    /// the boundary.
    #[test]
    fn the_reserved_prefix_is_escaped_not_passed_through() {
        assert!(crate::grammar::is_valid_plain_chunk("x-foo"));
        assert_eq!(chunk_slug("x-foo"), "x-x-foo");
        assert_eq!(chunk_slug("x-1"), "x-x-1");
        // `x` alone and `x_…` are not the prefix.
        assert_eq!(chunk_slug("x"), "x");
        assert_eq!(chunk_slug("x_1"), "x_1");
    }

    /// The pinned table (RFC 03 §2 v1.31). **This is the table adopters
    /// copy**: every row is a spelling the fleet may already carry in a
    /// key, so a row here changing is a re-keying, and it must fail this
    /// build before it re-keys a fleet.
    #[test]
    fn slug_outputs_are_pinned() {
        let table = [
            ("sshd.service", "sshd.service"),
            ("cam0", "cam0"),
            ("x_x5f_myns", "x_x5f_myns"),
            ("a_x", "a_x"),
            ("x-foo", "x-x-foo"),
            ("_myns", "x-_x5fmyns"),
            ("x@b", "x-x_x40b"),
            ("@b", "x-_x40b"),
            ("foo@1.service", "x-foo_x401.service"),
            ("has spaces", "x-has_x20spaces"),
            ("a_", "x-a_x5f"),
            ("_", "x-_x5f"),
            (".ab", "x-.ab"),
            ("ab.", "x-ab_x2e"),
            ("A", "x-_x41"),
            ("ETH0", "x-_x45_x54_x480"),
            ("café", "x-caf_xc3_xa9"),
            ("", "x-_x"),
        ];
        for (value, chunk) in table {
            assert_eq!(chunk_slug(value), chunk, "slug of {value:?}");
        }
    }

    #[test]
    fn escape_is_injective() {
        // The RFC's motivating counterexample: these MUST NOT share a chunk.
        assert_ne!(chunk_slug("foo@1.service"), chunk_slug("foo-1.service"));

        let slugs: Vec<String> = CORPUS.iter().map(|v| chunk_slug(v)).collect();
        let unique: HashSet<&String> = slugs.iter().collect();
        assert_eq!(unique.len(), CORPUS.len(), "collision in {slugs:?}");
        for s in &slugs {
            assert!(
                crate::grammar::is_valid_plain_chunk(s),
                "illegal slug {s:?}"
            );
        }
    }

    /// RFC 03 §2: a conforming slugger ships the decoder beside the encoder
    /// with a round-trip test over both classes.
    #[test]
    fn unslug_round_trips_the_corpus() {
        for v in CORPUS.iter().copied().chain(["日本", "\u{0}", "a\tb"]) {
            let chunk = chunk_slug(v);
            assert_eq!(
                chunk_unslug(&chunk).as_deref(),
                Some(v),
                "round trip of {v:?} via {chunk:?}"
            );
        }
    }

    /// The decoder is the left inverse on the image of the encoder and
    /// refuses everything else: malformed escapes, non-UTF-8 bytes, and
    /// spellings that decode to a value which slugs differently.
    #[test]
    fn unslug_refuses_malformed_and_non_canonical() {
        for bad in [
            "x-a_",    // `_` opens an escape; nothing follows
            "x-a_x4",  // one hex digit
            "x-a_xzz", // not hex
            "x-a_X41", // the escape marker is lowercase
            "x-a_x4A", // uppercase hex digit
            "x-_xff",  // not UTF-8
            "x-abc",   // `abc` passes through, so this spelling is not canonical
            "x-",      // the empty value is spelled `x-_x`
            "x-_xa",   // `_x` is the empty value only as the entire body
            "Foo",     // not a chunk at all
        ] {
            assert_eq!(chunk_unslug(bad), None, "{bad:?} must be refused");
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

    /// The v1.31 erratum, by its own two witnesses (RFC 03 §2): the pairs
    /// the v1.4 scheme collapsed stay distinct, and each side decodes back
    /// to itself.
    #[test]
    fn v131_erratum_examples_are_the_rfcs() {
        for (a, b) in [("_myns", "x_x5f_myns"), ("x@b", "@b")] {
            let (sa, sb) = (chunk_slug(a), chunk_slug(b));
            assert_ne!(sa, sb, "{a:?} and {b:?} must not share a chunk");
            assert_eq!(chunk_unslug(&sa).as_deref(), Some(a));
            assert_eq!(chunk_unslug(&sb).as_deref(), Some(b));
        }
        assert_eq!(chunk_slug("_myns"), "x-_x5fmyns");
        assert_eq!(chunk_slug("x_x5f_myns"), "x_x5f_myns");
        assert_eq!(chunk_slug("x@b"), "x-x_x40b");
        assert_eq!(chunk_slug("@b"), "x-_x40b");
    }
}
