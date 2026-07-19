//! The schema-aware decode seam (issues #11/#15): wire key → type name →
//! served schema → named-field JSON, with the honest fallbacks a generic
//! tool owes its user.
//!
//! [`SchemaStore`] caches each producer's served `describe` reply (RFC 08
//! §7) and fetches on first miss through the disciplined fan-in
//! ([`crate::query::fleet_get`]). [`decode_sample`] is the whole pipeline in
//! one call; encoding resolution is **sample > registry > sniff** and the
//! sniff never goes away.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use zenkey::schema::decode::{DecodeError, DecodedPayload, DecoderRegistry};
use zenkey::schema::{SchemaSet, TypeSchema, WireEncoding};
use zenoh::Session;

use crate::registry::SliceSet;

/// Per-producer schema sets, fetched lazily and cached for the process.
pub struct SchemaStore {
    base: String,
    timeout: Duration,
    /// producer → served set (None = asked and not served; don't re-ask).
    sets: Mutex<HashMap<String, Option<SchemaSet>>>,
    decoders: DecoderRegistry,
}

impl SchemaStore {
    pub fn new(base: impl Into<String>, timeout: Duration) -> Self {
        SchemaStore {
            base: base.into(),
            timeout,
            sets: Mutex::new(HashMap::new()),
            decoders: DecoderRegistry::new(),
        }
    }

    /// The decoder table (register custom kinds through this).
    pub fn decoders_mut(&mut self) -> &mut DecoderRegistry {
        &mut self.decoders
    }

    /// The schema for `type_name` as served by `producer`, fetching
    /// `@rpc/<producer>/describe` on first miss. `None` = the producer does
    /// not serve describe or does not describe this type — render
    /// structurally (never an error; RFC 08 §7 is a SHOULD for
    /// self-describing encodings).
    pub async fn schema_for(
        &self,
        session: &Session,
        producer: &str,
        type_name: &str,
    ) -> Option<TypeSchema> {
        {
            let sets = self.sets.lock().expect("store lock");
            if let Some(cached) = sets.get(producer) {
                return cached.as_ref().and_then(|s| s.get(type_name).cloned());
            }
        }
        let fetched = self.fetch(session, producer).await;
        let mut sets = self.sets.lock().expect("store lock");
        let entry = sets.entry(producer.to_string()).or_insert(fetched);
        entry.as_ref().and_then(|s| s.get(type_name).cloned())
    }

    async fn fetch(&self, session: &Session, producer: &str) -> Option<SchemaSet> {
        let key = zenkey::grammar::with_base(
            &self.base,
            zenkey::selector::fleet_rpc(producer, &["describe"]),
        );
        let answers = crate::query::fleet_get(session, &self.base, &key, None, self.timeout)
            .await
            .ok()?;
        // Any well-formed reply will do; hashes make same-name drift a
        // doctor finding, not a decode concern.
        for a in answers {
            if let crate::query::Answer::Value(bytes) = a.answer {
                let cow = bytes.to_bytes();
                if let Ok(text) = std::str::from_utf8(&cow)
                    && let Ok(set) = SchemaSet::parse(text)
                {
                    return Some(set);
                }
            }
        }
        None
    }

    /// Decode `bytes` under a schema, if one resolves.
    pub fn decode(
        &self,
        schema: &TypeSchema,
        encoding: &WireEncoding,
        bytes: &[u8],
    ) -> Result<DecodedPayload, DecodeError> {
        self.decoders.decode(schema, encoding, bytes)
    }
}

/// How a rendered payload was produced — a tool surfaces this honestly
/// instead of letting decoded and sniffed output look alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rendering {
    /// Schema-decoded into named fields.
    Typed(DecodedPayload),
    /// No schema (or an undecodable kind): structural sniff — JSON if it
    /// parses, CBOR diagnostic, UTF-8 text, else a byte count.
    Structural(String),
}

/// Resolve the wire encoding: sample `Encoding` > registry `encoding` > sniff
/// (RFC 08 §7).
pub fn resolve_encoding(
    sample_encoding: Option<&str>,
    registry_encoding: Option<&str>,
    bytes: &[u8],
) -> WireEncoding {
    // Zenoh's default when a publisher sets nothing is the opaque
    // `zenoh/bytes` — that is "unsaid", not "bytes on purpose".
    if let Some(e) = sample_encoding
        && e != "zenoh/bytes"
    {
        return WireEncoding::from_encoding_str(e);
    }
    if let Some(e) = registry_encoding {
        return WireEncoding::from_encoding_str(e);
    }
    // The sniff: JSON text starts with a JSON-ish byte; otherwise call it
    // CBOR (the reference profile default) and let the decoder's error path
    // fall through to structural rendering.
    match bytes.first() {
        Some(b'{' | b'[' | b'"') => WireEncoding::Json,
        _ => WireEncoding::Cbor,
    }
}

/// Structural fallback rendering — what the wire honestly says when no
/// schema resolves.
pub fn structural(bytes: &[u8]) -> String {
    let looks_json = bytes.first().is_some_and(|b| {
        matches!(
            b,
            b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        )
    });
    if looks_json && let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        return serde_json::to_string(&v).unwrap_or_default();
    }
    if let Ok(v) = ciborium::from_reader::<ciborium::Value, _>(bytes)
        && let Ok(text) = serde_json::to_string(&v)
    {
        return text;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.is_empty() => text.to_string(),
        _ => format!("<{} bytes>", bytes.len()),
    }
}

/// The whole decode pipeline for one sample: refine the key against the
/// slices, resolve the schema through the store, decode — or fall back
/// structurally, tagged with whatever we did learn.
pub async fn decode_sample(
    store: &SchemaStore,
    session: &Session,
    slices: &SliceSet,
    base: &str,
    wire_key: &str,
    sample_encoding: Option<&str>,
    bytes: &[u8],
) -> (Option<String>, Rendering) {
    use zenkey::grammar::ClassOrPlane;
    let refined = zenkey::grammar::parse_full(base, wire_key).and_then(|parsed| {
        let producer = match (&parsed.producer, &parsed.origin) {
            (Some(p), _) => p.name().to_string(),
            (None, zenkey::grammar::Origin::Service(s)) => {
                slices.by_service_origin(s)?.name.clone()
            }
            _ => return None,
        };
        let ClassOrPlane::Class(class) = parsed.class else {
            return None;
        };
        let (subject, _) = slices.refine(&producer, class.chunk(), &parsed.subject)?;
        Some((
            producer,
            subject.type_name.clone(),
            subject.encoding.clone(),
        ))
    });
    let Some((producer, type_name, registry_encoding)) = refined else {
        return (None, Rendering::Structural(structural(bytes)));
    };
    let encoding = resolve_encoding(sample_encoding, registry_encoding.as_deref(), bytes);
    match store.schema_for(session, &producer, &type_name).await {
        Some(schema) => match store.decode(&schema, &encoding, bytes) {
            Ok(decoded) => (Some(type_name), Rendering::Typed(decoded)),
            // Wrong schema/encoding is a finding for the *user*, not a crash:
            // fall back to structure, keep the type tag.
            Err(_) => (Some(type_name), Rendering::Structural(structural(bytes))),
        },
        None => (Some(type_name), Rendering::Structural(structural(bytes))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_resolution_order() {
        // Sample wins…
        assert_eq!(
            resolve_encoding(Some("application/json"), Some("application/cbor"), b"x"),
            WireEncoding::Json
        );
        // …but the opaque default is "unsaid", so the registry speaks…
        assert_eq!(
            resolve_encoding(Some("zenoh/bytes"), Some("application/cbor"), b"{"),
            WireEncoding::Cbor
        );
        // …and with neither, the sniff.
        assert_eq!(
            resolve_encoding(None, None, b"{\"a\":1}"),
            WireEncoding::Json
        );
        assert_eq!(resolve_encoding(None, None, &[0xa1]), WireEncoding::Cbor);
    }

    #[test]
    fn structural_rendering_is_honest() {
        assert_eq!(structural(b"{\"a\":1}"), "{\"a\":1}");
        // CBOR map {1: 2} renders as structure.
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert!(structural(&cbor).contains("\"x\""));
        assert_eq!(structural(&[0xff, 0xfe, 0x00]), "<3 bytes>");
    }
}
