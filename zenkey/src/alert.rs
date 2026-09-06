//! The alert-key derivation — the ZenSight reference recipe (RFC 11 §3.1).
//!
//! The neutral requirement is RFC 04 §1.2: an alert key is a **stable,
//! origin-excluded hash of rule identity + discriminating labels**, byte-
//! precise per application profile. The hash function, input framing, and
//! rendered width are *profile constants* — this module implements the
//! reference profile's binding (RFC 11 §3.1, v1.25), with the same byte
//! precision RFC 06 §1 gives the origin derivation, so two independent
//! implementations mint the same key for the same alert. An application on
//! a different profile defines its own recipe in its profile chapter; this
//! one is normative for ZenSight and reference for everyone else.
//!
//! ```text
//! input      = rule_name
//!              ++ ( "\n" ++ label_name ++ "=" ++ label_value )*   ascending
//!              by label_name, byte (memcmp) collation
//! alert_key  = lowercase_hex(fnv1a_64(utf8(input)))               16 chars
//! ```
//!
//! The origin never enters the input — origin and producer are already in
//! the key (`…/state/<producer>/alert/<alert_key>`), which is what makes
//! the same alert on two hosts the same key under two origins.
//!
//! The **alert ref** (RFC 11 §3.2, v1.29) is the other half: one chunk that
//! names one firing alert across the fleet, `origin.producer.alert_key`,
//! read from the *key* an alert was published on — never from the payload,
//! whose `source` is the polled device for a proxy producer. [`alert_ref`]
//! mints one and [`parse_alert_ref`] splits it back, on the first two `.`s.

/// Why [`alert_key`] refused its input — each variant is one of RFC 11
/// §3.1's injectivity conditions, without which two different alerts could
/// frame to the same bytes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AlertKeyError {
    /// The rule name is empty or contains `\n` — the framing separates rule
    /// from labels by `\n`, so a newline in the rule forges a label.
    #[error("alert rule name {0:?} must be nonempty and contain no newline (RFC 11 §3.1)")]
    BadRule(String),
    /// A label name contains `\n` or `=` — names are `snake_case`
    /// identifiers precisely so neither framing byte can appear in one.
    #[error("alert label name {0:?} must contain no newline or '=' (RFC 11 §3.1)")]
    BadLabelName(String),
    /// A label value contains `\n` — the one byte a value must not carry.
    #[error("alert label value {0:?} for {1:?} must contain no newline (RFC 11 §3.1)")]
    BadLabelValue(String, String),
}

/// FNV-1a, 64-bit (offset basis `0xcbf29ce484222325`, prime
/// `0x100000001b3`) — the profile's hash (RFC 11 §3.1).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// Derive an alert key (RFC 11 §3.1, v1.25): 16 lowercase hex chars — all
/// 64 bits of FNV-1a over `rule` + `\n<name>=<value>` per discriminating
/// label, ascending by name under byte collation.
///
/// `labels` are the **discriminating** labels — the ones that distinguish
/// instances of one rule (`peer`, `port`, `unit`…). The label named `host`
/// is host-scoped by the RFC and excluded here before sorting; any *other*
/// label the producer documents as host-scoped is the caller's to exclude,
/// since only the producer knows its vocabulary. A rule with no
/// discriminating labels hashes the bare rule name.
///
/// The result is a legal plain key chunk by construction, ready for
/// `state/<producer>/alert/<alert_key>` ([`crate::CommonState::Alert`]).
///
/// ```
/// // The RFC 11 §3.1 test vector: `host` drops out, `peer` sorts before
/// // `port`, and the input is `link_down\npeer=r2\nport=eth0`.
/// let key = zenkey::alert::alert_key(
///     "link_down",
///     &[("port", "eth0"), ("host", "h-3fa9c2d41b7e"), ("peer", "r2")],
/// )
/// .unwrap();
/// assert_eq!(key, "a659f813308ad1da");
/// ```
pub fn alert_key(rule: &str, labels: &[(&str, &str)]) -> Result<String, AlertKeyError> {
    if rule.is_empty() || rule.contains('\n') {
        return Err(AlertKeyError::BadRule(rule.to_string()));
    }
    let mut discriminating: Vec<(&str, &str)> = Vec::with_capacity(labels.len());
    for &(name, value) in labels {
        // Host-scoped exclusion happens *before* sorting (RFC 11 §3.1):
        // hashing the host in would break the one property the exclusion
        // exists for.
        if name == "host" {
            continue;
        }
        if name.is_empty() || name.contains('\n') || name.contains('=') {
            return Err(AlertKeyError::BadLabelName(name.to_string()));
        }
        if value.contains('\n') {
            return Err(AlertKeyError::BadLabelValue(
                value.to_string(),
                name.to_string(),
            ));
        }
        discriminating.push((name, value));
    }
    // Ascending byte (memcmp) collation on the name; the value tiebreak
    // keeps the input deterministic if a caller ever passes a duplicate
    // name (Rust's str ordering *is* byte collation).
    discriminating.sort_unstable();
    let mut input = String::from(rule);
    for (name, value) in discriminating {
        input.push('\n');
        input.push_str(name);
        input.push('=');
        input.push_str(value);
    }
    Ok(format!("{:016x}", fnv1a_64(input.as_bytes())))
}

/// Why [`alert_ref`] refused a component — a ref that would need escaping
/// is a ref that will be wrong somewhere (RFC 11 §3.2), so nothing here
/// truncates or escapes.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AlertRefError {
    /// Not a host origin (`h-<12hex>`) and not a verbatim service origin
    /// (`@catalog`).
    #[error("alert ref origin {0:?} is not an origin chunk (RFC 11 §3.2)")]
    BadOrigin(String),
    /// Not a plain chunk, or one containing the `.` separator — a producer
    /// chunk's alphabet excludes it, which is what makes the split
    /// unambiguous.
    #[error("alert ref producer {0:?} is not a dot-free plain chunk (RFC 11 §3.2)")]
    BadProducer(String),
    /// Not a plain chunk — the ref is used as one (`ack/<alert_ref>`).
    #[error("alert ref alert_key {0:?} is not a plain chunk (RFC 11 §3.2)")]
    BadAlertKey(String),
}

/// Mint an alert ref (RFC 11 §3.2): `origin ++ "." ++ producer ++ "." ++
/// alert_key`, every component read from the key the alert rode on.
///
/// Refused rather than escaped when a component would not survive as a key
/// chunk: the origin must be a host origin or a verbatim service origin,
/// the producer a plain chunk with no `.` in it (the separator), the alert
/// key a plain chunk.
///
/// ```
/// // The RFC 11 §3.2 test vector: the §3.1 alert, published by `netlink`
/// // on origin `h-3fa9c2d41b7e`.
/// let r = zenkey::alert::alert_ref("h-3fa9c2d41b7e", "netlink", "a659f813308ad1da").unwrap();
/// assert_eq!(r, "h-3fa9c2d41b7e.netlink.a659f813308ad1da");
/// ```
pub fn alert_ref(origin: &str, producer: &str, alert_key: &str) -> Result<String, AlertRefError> {
    use crate::grammar::{is_valid_host_origin, is_valid_plain_chunk, is_valid_verbatim_chunk};
    if !(is_valid_host_origin(origin) || is_valid_verbatim_chunk(origin)) {
        return Err(AlertRefError::BadOrigin(origin.to_string()));
    }
    if !is_valid_plain_chunk(producer) || producer.contains('.') {
        return Err(AlertRefError::BadProducer(producer.to_string()));
    }
    if !is_valid_plain_chunk(alert_key) {
        return Err(AlertRefError::BadAlertKey(alert_key.to_string()));
    }
    Ok(format!("{origin}.{producer}.{alert_key}"))
}

/// Split an alert ref back into `(origin, producer, alert_key)` — on the
/// **first two** separators, keeping the remainder as the alert key
/// (RFC 11 §3.2: an application whose alert-key binding differs may
/// legitimately carry a `.` there). `None` when the ref does not have three
/// non-empty parts or its origin and producer would not pass [`alert_ref`].
///
/// ```
/// let (o, p, k) = zenkey::alert::parse_alert_ref("h-3fa9c2d41b7e.netlink.a659f813308ad1da").unwrap();
/// assert_eq!((o, p, k), ("h-3fa9c2d41b7e", "netlink", "a659f813308ad1da"));
/// // The remainder is the key, dots included.
/// let (_, _, k) = zenkey::alert::parse_alert_ref("h-3fa9c2d41b7e.sysinfo.cpu.usage").unwrap();
/// assert_eq!(k, "cpu.usage");
/// ```
pub fn parse_alert_ref(r: &str) -> Option<(&str, &str, &str)> {
    use crate::grammar::{is_valid_host_origin, is_valid_plain_chunk, is_valid_verbatim_chunk};
    let mut parts = r.splitn(3, '.');
    let origin = parts.next()?;
    let producer = parts.next()?;
    let alert_key = parts.next()?;
    if !(is_valid_host_origin(origin) || is_valid_verbatim_chunk(origin)) {
        return None;
    }
    if !is_valid_plain_chunk(producer) || alert_key.is_empty() {
        return None;
    }
    Some((origin, producer, alert_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 11 §3.1 test vector, which implementations MUST reproduce:
    /// rule `link_down`, labels `{peer: r2, port: eth0, host: …}` — `host`
    /// drops out, `peer` sorts before `port`, the input is the 27 bytes
    /// `link_down\npeer=r2\nport=eth0`, FNV-1a-64 is `0xa659f813308ad1da`.
    #[test]
    fn the_rfc_test_vector() {
        let key = alert_key(
            "link_down",
            &[("peer", "r2"), ("port", "eth0"), ("host", "h-3fa9c2d41b7e")],
        )
        .unwrap();
        assert_eq!(key, "a659f813308ad1da");
        // Byte-precise intermediate: the framed input and its hash.
        assert_eq!(
            "link_down\npeer=r2\nport=eth0".len(),
            27,
            "the RFC counts 27 bytes"
        );
        assert_eq!(
            fnv1a_64(b"link_down\npeer=r2\nport=eth0"),
            0xa659_f813_308a_d1da
        );
    }

    /// Label order on the way in is irrelevant; the `host` exclusion and the
    /// byte-collation sort make the key stable.
    #[test]
    fn ordering_and_host_exclusion_are_canonical() {
        let a = alert_key("link_down", &[("port", "eth0"), ("peer", "r2")]).unwrap();
        let b = alert_key("link_down", &[("peer", "r2"), ("port", "eth0")]).unwrap();
        assert_eq!(a, b);
        // With and without a host label: same alert, same key — the origin
        // already scopes it.
        let c = alert_key(
            "link_down",
            &[("host", "h-ffffffffffff"), ("peer", "r2"), ("port", "eth0")],
        )
        .unwrap();
        assert_eq!(a, c);
        // No discriminating labels: the bare rule name hashes.
        assert_eq!(
            alert_key("link_down", &[]).unwrap(),
            format!("{:016x}", fnv1a_64(b"link_down"))
        );
    }

    /// The injectivity conditions are refusals, not silent escapes: a
    /// newline or `=` in a name, or a newline in a value, could frame two
    /// different alerts to the same bytes.
    #[test]
    fn injectivity_conditions_refuse() {
        assert!(matches!(alert_key("", &[]), Err(AlertKeyError::BadRule(_))));
        assert!(matches!(
            alert_key("a\nb", &[]),
            Err(AlertKeyError::BadRule(_))
        ));
        assert!(matches!(
            alert_key("r", &[("pe=er", "x")]),
            Err(AlertKeyError::BadLabelName(_))
        ));
        assert!(matches!(
            alert_key("r", &[("pe\ner", "x")]),
            Err(AlertKeyError::BadLabelName(_))
        ));
        assert!(matches!(
            alert_key("r", &[("", "x")]),
            Err(AlertKeyError::BadLabelName(_))
        ));
        assert!(matches!(
            alert_key("r", &[("peer", "x\ny")]),
            Err(AlertKeyError::BadLabelValue(..))
        ));
        // `=` in a *value* is legal — only the name side frames on it.
        assert!(alert_key("r", &[("peer", "a=b")]).is_ok());
    }

    /// The rendered key is a legal plain chunk (all 64 bits, 16 lowercase
    /// hex chars) — it drops into `alert/{alert_key}` unchanged.
    #[test]
    fn key_is_a_legal_chunk() {
        let key = alert_key("link_down", &[("peer", "r2")]).unwrap();
        assert_eq!(key.len(), 16);
        assert!(crate::grammar::is_valid_plain_chunk(&key));
    }

    /// The RFC 11 §3.2 test vector, and the parse that splits on the first
    /// two separators only.
    #[test]
    fn the_alert_ref_vector_round_trips() {
        let r = alert_ref("h-3fa9c2d41b7e", "netlink", "a659f813308ad1da").unwrap();
        assert_eq!(r, "h-3fa9c2d41b7e.netlink.a659f813308ad1da");
        assert!(crate::grammar::is_valid_plain_chunk(&r), "a ref is a chunk");
        assert_eq!(
            parse_alert_ref(&r),
            Some(("h-3fa9c2d41b7e", "netlink", "a659f813308ad1da"))
        );
        // A service origin refs the same way; the remainder keeps its dots.
        let r = alert_ref("@catalog", "catalog", "x.y").unwrap();
        assert_eq!(parse_alert_ref(&r), Some(("@catalog", "catalog", "x.y")));
    }

    /// Refusals, never escapes (RFC 11 §3.2): a bad origin, a producer that
    /// carries the separator, an alert key that is not a chunk.
    #[test]
    fn a_ref_that_would_need_escaping_is_refused() {
        assert!(matches!(
            alert_ref("host", "netlink", "a659f813308ad1da"),
            Err(AlertRefError::BadOrigin(_))
        ));
        assert!(matches!(
            alert_ref("h-3fa9c2d41b7e", "net.link", "a659f813308ad1da"),
            Err(AlertRefError::BadProducer(_))
        ));
        assert!(matches!(
            alert_ref("h-3fa9c2d41b7e", "netlink", "A659"),
            Err(AlertRefError::BadAlertKey(_))
        ));
        assert_eq!(parse_alert_ref("h-3fa9c2d41b7e.netlink"), None);
        assert_eq!(parse_alert_ref("nope.netlink.k"), None);
        assert_eq!(parse_alert_ref("h-3fa9c2d41b7e.netlink."), None);
    }
}
