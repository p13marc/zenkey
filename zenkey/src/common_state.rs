//! The framework state subjects, shared by every producer (issue #475).
//!
//! RFC 08 §1 makes the codegen contract *two* directions, and the **parse**
//! direction exists for one reason: to delete positional `split('/')` from
//! consumers. But a consumer that subscribes to the whole `state` class sees
//! eleven different producers' `Subject` enums, and matching each one
//! separately would trade a positional parse for a combinatorial one.
//!
//! Almost all of the state class is the *same* small set of subjects on every
//! producer — health, errors, the registration doc, alerts, evidence.
//! [`CommonState`] is that set — the subjects the RFCs themselves name
//! (RFC 04 §1.2/§5, RFC 06 §4/§5). The registry codegen (`zenkey-build`)
//! generates `AnySubject::common_state()` from the `common =` field on a
//! consumer's registry entries, so the mapping cannot drift from the registry.
//! App-specific state groupings beyond this set are the consumer's to define
//! as a wrapper over its generated `AnySubject`.
//!
//! A consumer therefore writes one match over a dozen typed variants, with the
//! variables already extracted and named, and gets a compile error if the
//! registry moves under it — which is what the parse direction was for.
//!
//! Telemetry deliberately has no equivalent: a subscriber decoding a
//! `TelemetryPoint` does not need to know *which* metric it is, and a consumer
//! that does (a view) already knows its producer and matches that producer's
//! `Subject` directly.

/// A framework state subject, refined from any producer's `Subject`.
///
/// Borrows from the subject it was refined from — no allocation on the decode
/// path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommonState<'a> {
    /// `health` — the sensor health document.
    Health,
    /// `errors` — the rolling error window.
    Errors,
    /// `sensor` — the registration document (identity, version, capabilities).
    Sensor,
    /// `alert/{alert_key}` — firing→resolved on one key; a delete is a tombstone.
    Alert { alert_key: &'a str },
    /// `evidence/self` — the producer's own identity claim (RFC 06 §4).
    EvidenceSelf,
    /// `evidence/device/{device}` — an observed device's identity claim.
    EvidenceDevice { device: &'a str },
    /// `evidence/names/{ip_slug}` — a passive-DNS name observation.
    EvidenceNames { ip_slug: &'a str },
    /// `@catalog` `entity/{entity_id}` — the merged entity document (RFC 06 §5).
    CatalogEntity { entity_id: &'a str },
    /// `@catalog` `alias/{old_id}` — old-id → entity-id re-pointing (RFC 06 §5).
    CatalogAlias { old_id: &'a str },
    /// `@catalog` `pdns/{ip_slug}` — the accumulated IP↔name record (RFC 06 §5).
    CatalogPdns { ip_slug: &'a str },
}

/// A cross-producer framework state family — the fieldless sibling of
/// [`CommonState`] (issue #168).
///
/// [`CommonState`] refines a subject a consumer has *received*, so its
/// variants borrow the received key's variables. This enum names the family
/// itself — no key in hand, no borrow — so a consumer can ask the fleet-wide
/// question *before* any sample arrives: "every producer's health", "the
/// firing alert set anywhere". [`crate::selector::common_family`] turns one
/// of these into that selector.
///
/// Only the families that live under a *producer* chunk are here. The
/// `@catalog` subjects ([`CommonState::CatalogEntity`] and friends) are one
/// service's state (RFC 06 §5, `v1/@catalog/state/entity/…` — no producer
/// position to wildcard), not a family across producers; and a `*` origin
/// scope could not reach them anyway (RFC 03 §4 D4: `*` never matches a
/// verbatim service origin).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommonFamily {
    /// `health` — the sensor health document (RFC 04 §1.2).
    Health,
    /// `errors` — the rolling error window (RFC 04 §1.2).
    Errors,
    /// `sensor` — the registration document (RFC 04 §5).
    Sensor,
    /// `alert/{alert_key}` — the firing set (RFC 04 §2: alerts are state,
    /// so "what is firing anywhere" is one selector).
    Alert,
    /// `evidence/self` — producers' own identity claims (RFC 06 §4).
    EvidenceSelf,
    /// `evidence/device/{device}` — observed-device identity claims (RFC 06 §4).
    EvidenceDevice,
    /// `evidence/names/{ip_slug}` — passive-DNS name observations (RFC 06 §4).
    EvidenceNames,
}

impl CommonFamily {
    /// Every cross-producer family, for iteration (views, lints).
    pub const ALL: [CommonFamily; 7] = [
        CommonFamily::Health,
        CommonFamily::Errors,
        CommonFamily::Sensor,
        CommonFamily::Alert,
        CommonFamily::EvidenceSelf,
        CommonFamily::EvidenceDevice,
        CommonFamily::EvidenceNames,
    ];

    /// The `common = "…"` registry token that declares a subject as this
    /// family (RFC 08 §5) — the vocabulary `zenkey-build` lints against.
    pub fn token(self) -> &'static str {
        match self {
            CommonFamily::Health => "health",
            CommonFamily::Errors => "errors",
            CommonFamily::Sensor => "sensor",
            CommonFamily::Alert => "alert",
            CommonFamily::EvidenceSelf => "evidence_self",
            CommonFamily::EvidenceDevice => "evidence_device",
            CommonFamily::EvidenceNames => "evidence_names",
        }
    }

    /// The family's fixed subject chunks under `state/<producer>/`, as the
    /// RFCs spell them.
    pub fn prefix(self) -> &'static [&'static str] {
        match self {
            CommonFamily::Health => &["health"],
            CommonFamily::Errors => &["errors"],
            CommonFamily::Sensor => &["sensor"],
            CommonFamily::Alert => &["alert"],
            CommonFamily::EvidenceSelf => &["evidence", "self"],
            CommonFamily::EvidenceDevice => &["evidence", "device"],
            CommonFamily::EvidenceNames => &["evidence", "names"],
        }
    }

    /// The trailing population variable's name, for the families that are
    /// population-keyed (RFC 04 §1.2); [`crate::selector::common_family`]
    /// wildcards it. The names match the [`CommonState`] variant fields, and
    /// the registry lint holds `common = "…"` subjects to them.
    pub fn var(self) -> Option<&'static str> {
        match self {
            CommonFamily::Alert => Some("alert_key"),
            CommonFamily::EvidenceDevice => Some("device"),
            CommonFamily::EvidenceNames => Some("ip_slug"),
            CommonFamily::Health
            | CommonFamily::Errors
            | CommonFamily::Sensor
            | CommonFamily::EvidenceSelf => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::is_valid_plain_chunk;

    /// The table stays inside the grammar: every fixed chunk is a legal plain
    /// chunk (RFC 03 §2), and the registry tokens are distinct.
    #[test]
    fn family_table_is_grammar_legal_and_distinct() {
        let mut tokens = Vec::new();
        for f in CommonFamily::ALL {
            assert!(
                f.prefix().iter().all(|c| is_valid_plain_chunk(c)),
                "{f:?} prefix violates RFC 03 §2"
            );
            assert!(!f.prefix().is_empty(), "{f:?} has no path");
            tokens.push(f.token());
        }
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), CommonFamily::ALL.len());
    }
}
