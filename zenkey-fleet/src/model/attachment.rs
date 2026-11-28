//! An archive reply's attachment as a tool shows it (core §4.4; #980 C3).
//!
//! An archive's reply on its own key (`…/archive.v1/@state/…`) carries an
//! attachment in one of two forms, told apart by their length: exactly 3
//! bytes is the compact form, any other the JSON form. A tool shows the
//! JSON form as JSON, as the structural ladder does, and the compact form
//! by its fields: the revision's index in the archive's catalog, the
//! resource's place in that revision, and `confirmed` (or the refusal of
//! its bits), never sniffed as text. Its identity is read through the
//! catalog the archive's descriptor names, which this does not hold:
//! `zenkey::archive::last_known_through` reads it.

use serde_json::{Value, json};
use zenkey_model::attachment::{Compact, Form};
use zenkey_model::grammar::{KindToken, ZkKey, parse};

/// Whether `key` is an archive's own key (core §4.4): `archive.v1`'s
/// `@state`, base-relative or under a namespace (core §1.6), which no
/// `zk2` chunk spells.
#[must_use]
pub fn is_archive_key(key: &str) -> bool {
    let key = match key.find("/zk2/") {
        Some(at) if !key.starts_with("zk2/") => &key[at + 1..],
        _ => key,
    };
    matches!(
        parse(key),
        Ok(ZkKey::Data { iface, kind: KindToken::ExplicitState, .. }) if iface.to_string() == "archive.v1"
    )
}

/// The compact attachment of an archive's reply on `key`, by its fields
/// (core §4.4): `{"form": "compact", "index", "resource", "confirmed"}`,
/// or `{"form": "compact", "refused": <token>}` for bits it refuses.
/// `None` for any other key, and for the JSON form.
#[must_use]
pub fn archive_attachment(key: &str, bytes: &[u8]) -> Option<Value> {
    if Form::of(bytes) != Form::Compact || !is_archive_key(key) {
        return None;
    }
    Some(match Compact::read(bytes) {
        Ok(c) => json!({
            "form": "compact",
            "index": c.index,
            "resource": c.resource,
            "confirmed": c.confirmed,
        }),
        Err(r) => json!({"form": "compact", "refused": r.token()}),
    })
}

/// An attachment as a JSON value: an archive's compact one by its fields
/// ([`archive_attachment`]), else the structural ladder's value, else its
/// text.
#[must_use]
pub fn attachment_value(key: &str, bytes: &[u8]) -> Value {
    archive_attachment(key, bytes)
        .or_else(|| super::structural::structural_value(bytes))
        .unwrap_or_else(|| Value::String(super::structural::structural(bytes)))
}

/// An attachment as one line of text: an archive's compact one by its
/// fields, else the structural ladder's rendering.
#[must_use]
pub fn attachment_text(key: &str, bytes: &[u8]) -> String {
    match archive_attachment(key, bytes) {
        Some(v) => v.to_string(),
        None => super::structural::structural(bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::{archive_attachment, attachment_text, is_archive_key};

    const KEY: &str =
        "zk2/vehicle-01/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/v1";

    /// Core §4.4: 3 bytes on an archive's key are the compact form, shown by
    /// its fields; the same bytes on another key, and the JSON form, are
    /// the structural ladder's.
    #[test]
    fn an_archives_compact_attachment_is_shown_by_its_fields() {
        assert!(is_archive_key(KEY));
        assert!(!is_archive_key("zk2/g/s/nav.v2/state/x"));
        assert!(is_archive_key(&format!("dep1/{KEY}")), "under a namespace");
        assert_eq!(
            archive_attachment(KEY, &[0x00, 0x01, 0xc0]),
            Some(serde_json::json!({
                "form": "compact", "index": 0, "resource": 3, "confirmed": true
            }))
        );
        assert_eq!(
            archive_attachment(KEY, b"{} "),
            Some(serde_json::json!({"form": "compact", "refused": "extension"}))
        );
        assert_eq!(archive_attachment("zk2/g/s/nav.v2/state/x", b"abc"), None);
        assert_eq!(archive_attachment(KEY, br#"{"confirmed":true}"#), None);
        assert_eq!(
            attachment_text(KEY, br#"{"confirmed":true}"#),
            r#"{"confirmed":true}"#
        );
    }
}
