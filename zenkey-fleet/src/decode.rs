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
    /// producer → (served set or None = asked-and-not-served, with the ask
    /// instant — a `None` expires after [`NOT_SERVED_TTL`], so a producer
    /// that *starts* serving describe is noticed without a restart).
    sets: Mutex<HashMap<String, (Option<SchemaSet>, std::time::Instant)>>,
    decoders: DecoderRegistry,
}

/// How long "asked, not served" stays authoritative before re-asking.
const NOT_SERVED_TTL: Duration = Duration::from_secs(60);

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
            if let Some((cached, asked)) = sets.get(producer) {
                let expired = cached.is_none() && asked.elapsed() > NOT_SERVED_TTL;
                if !expired {
                    return cached.as_ref().and_then(|s| s.get(type_name).cloned());
                }
            }
        }
        let fetched = self.fetch(session, producer).await;
        let mut sets = self.sets.lock().expect("store lock");
        let entry = sets.insert(producer.to_string(), (fetched, std::time::Instant::now()));
        let _ = entry;
        sets.get(producer)
            .and_then(|(s, _)| s.as_ref())
            .and_then(|s| s.get(type_name).cloned())
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

/// Two producers serving one type name with different hashes — "a `doctor`
/// finding" by RFC 08 §7's own words (issue #41).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SchemaDrift {
    pub type_name: String,
    /// Every (producer, hash) pair observed for the name.
    pub servers: Vec<(String, String)>,
}

/// A type the producer's slice references that its served describe set does
/// not cover — a violation of RFC 08 §7's totality clause.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TotalityGap {
    pub producer: String,
    pub missing: Vec<String>,
}

/// Compute drift across a described fleet. Pure — feed it whatever describe
/// replies were gathered (the store's cache, or a fresh sweep).
pub fn schema_drift(described: &[(String, SchemaSet)]) -> Vec<SchemaDrift> {
    use std::collections::BTreeMap;
    let mut by_name: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
    for (producer, set) in described {
        for (name, schema) in set.iter() {
            by_name
                .entry(name)
                .or_default()
                .push((producer.clone(), schema.hash().to_string()));
        }
    }
    by_name
        .into_iter()
        .filter(|(_, servers)| servers.iter().any(|(_, h)| h != &servers[0].1))
        .map(|(name, servers)| SchemaDrift {
            type_name: name.to_string(),
            servers,
        })
        .collect()
}

/// Totality per producer: every type name the slice references (subjects,
/// procedure request/reply, blob references) must appear in the served set
/// (RFC 08 §7). A producer that served no describe at all is NOT a gap here —
/// that is "describe absent", a different finding with a different fix.
pub fn totality_gaps(described: &[(String, SchemaSet)], slices: &SliceSet) -> Vec<TotalityGap> {
    let mut gaps = Vec::new();
    for (producer, set) in described {
        let Some(slice) = slices.get(producer) else {
            continue;
        };
        let mut names: Vec<&str> = Vec::new();
        names.extend(slice.subjects.iter().map(|s| s.type_name.as_str()));
        for p in &slice.procedures {
            names.extend(p.request.as_deref());
            names.extend(p.reply.as_deref());
        }
        for b in &slice.blob {
            names.extend(b.reference.as_deref());
        }
        names.sort();
        names.dedup();
        let missing: Vec<String> = names
            .into_iter()
            .filter(|n| set.get(n).is_none())
            .map(str::to_string)
            .collect();
        if !missing.is_empty() {
            gaps.push(TotalityGap {
                producer: producer.clone(),
                missing,
            });
        }
    }
    gaps
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
    let utf8 = std::str::from_utf8(bytes).ok().filter(|t| !t.is_empty());
    if let Some(v) = cbor_whole(bytes)
        // A bare CBOR scalar over bytes that are *also* valid text is the
        // ambiguous case, and plain text is the likelier reading on a bus that
        // carries anything. Structured CBOR (a map, an array) is unambiguous
        // and still wins.
        && !(utf8.is_some() && is_scalar(&v))
        && let Ok(text) = serde_json::to_string(&v)
    {
        return text;
    }
    match utf8 {
        Some(text) => text.to_string(),
        None => format!("<{} bytes>", bytes.len()),
    }
}

/// Decode CBOR only if it accounts for **every** byte.
///
/// `ciborium::from_reader` decodes one value from the front and ignores the
/// rest, which makes it a false-positive machine on plain text: `j` is `0x6A`,
/// "text string of length 10", so `just a plain string` decodes as the CBOR
/// text `"ust a plai"` with eight bytes left over — and an explorer that shows
/// that has silently corrupted the payload it was asked to display. Any
/// lowercase-initial ASCII text is a candidate. Requiring total consumption is
/// what makes the sniff honest (RFC 08 §7 — sniffing is the last resort, so it
/// must at least be self-consistent).
fn cbor_whole(bytes: &[u8]) -> Option<ciborium::Value> {
    let mut cursor = std::io::Cursor::new(bytes);
    let value = ciborium::from_reader::<ciborium::Value, _>(&mut cursor).ok()?;
    (cursor.position() as usize == bytes.len()).then_some(value)
}

/// A single scalar, as opposed to a map or array.
fn is_scalar(v: &ciborium::Value) -> bool {
    !matches!(v, ciborium::Value::Map(_) | ciborium::Value::Array(_))
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

    /// Regression: plain text must not be eaten by the CBOR sniff.
    ///
    /// `ciborium` decodes one value from the front and ignores trailing bytes,
    /// so `just a plain string` used to render as `"ust a plai"` — `j` is
    /// `0x6A`, "text string of length 10". Every lowercase-initial ASCII
    /// payload was a candidate, which on an arbitrary bus is most of them.
    #[test]
    fn plain_text_is_not_mistaken_for_cbor() {
        assert_eq!(structural(b"just a plain string"), "just a plain string");
        assert_eq!(
            structural(b"a v2 key: not this convention"),
            "a v2 key: not this convention"
        );
        // The whole lowercase range is the danger zone (0x60..=0x7b).
        for first in b'a'..=b'z' {
            let mut payload = vec![first];
            payload.extend_from_slice(b" some trailing words here");
            let text = String::from_utf8(payload.clone()).unwrap();
            assert_eq!(structural(&payload), text, "mangled {text:?}");
        }
    }

    /// The ambiguous case: bytes that are *both* a complete CBOR text string
    /// and valid UTF-8. Plain text is the likelier reading on a bus that
    /// carries anything, and it is the lossless one.
    #[test]
    fn an_exact_cbor_text_string_still_reads_as_text() {
        // 0x6A = text(10), followed by exactly 10 bytes: fully consumed CBOR.
        let payload = b"just a plai";
        assert!(cbor_whole(payload).is_some(), "setup: this is valid CBOR");
        assert_eq!(structural(payload), "just a plai");
    }

    /// …but structured CBOR is unambiguous and must still win, even when the
    /// bytes happen to be valid UTF-8.
    #[test]
    fn structured_cbor_still_wins_over_text() {
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"ok": true}), &mut cbor).unwrap();
        let rendered = structural(&cbor);
        assert!(rendered.contains("\"ok\""), "{rendered}");
        assert!(rendered.starts_with('{'), "{rendered}");
    }

    /// Trailing bytes mean the buffer is not one CBOR value, whatever the
    /// front of it looks like.
    #[test]
    fn cbor_must_account_for_every_byte() {
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert!(cbor_whole(&cbor).is_some());
        cbor.push(0x00);
        assert!(cbor_whole(&cbor).is_none(), "trailing byte must reject");
    }

    fn set_with(name: &str, schema: serde_json::Value) -> SchemaSet {
        SchemaSet::builder("app")
            .entry(name, zenkey::schema::TypeSchema::json_schema(schema))
            .build()
    }

    /// RFC 08 §7: same name, different hash, across producers — one finding
    /// listing every server; agreement is silent.
    #[test]
    fn drift_findings_name_every_server() {
        let a = SchemaSet::builder("app")
            .entry(
                "T",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"object"})),
            )
            .build();
        let b = SchemaSet::builder("app")
            .entry(
                "T",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"string"})),
            )
            .build();
        let c = SchemaSet::builder("app")
            .entry(
                "T",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"object"})),
            )
            .build();
        let described = vec![
            ("p1".to_string(), a),
            ("p2".to_string(), b),
            ("p3".to_string(), c),
        ];
        let drift = schema_drift(&described);
        assert_eq!(drift.len(), 1);
        assert_eq!(drift[0].type_name, "T");
        assert_eq!(drift[0].servers.len(), 3, "every server is named");
        // p1 and p3 agree; p2 is the odd one out — the caller can see which.
        assert_eq!(drift[0].servers[0].1, drift[0].servers[2].1);
        assert_ne!(drift[0].servers[0].1, drift[0].servers[1].1);

        // All agreeing: no finding.
        let described = vec![
            (
                "p1".to_string(),
                set_with("T", serde_json::json!({"type":"object"})),
            ),
            (
                "p3".to_string(),
                set_with("T", serde_json::json!({"type":"object"})),
            ),
        ];
        assert!(schema_drift(&described).is_empty());
    }

    /// Totality: a slice-referenced type absent from the served describe is a
    /// gap; a producer that served no describe is not judged here.
    #[test]
    fn totality_gaps_check_only_served_producers() {
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let slice = RegistrySlice {
            version: "1".into(),
            app: "a".into(),
            convention: 1,
            name: "sysinfo".into(),
            service_origin: None,
            description: None,
            subjects: vec![SubjectDecl {
                path: "cpu".into(),
                class: "telemetry".into(),
                type_name: "TelemetryPoint".into(),
                since: None,
                description: None,
                qos: None,
                ttl_s: None,
                unit: None,
                rate: None,
                cardinality: None,
                encoding: None,
            }],
            procedures: vec![],
            blob: vec![],
            deprecated: vec![],
        };
        let slices = crate::registry::SliceSet::from_slices(vec![slice]);

        // Served describe missing the referenced type: one gap.
        let incomplete = SchemaSet::builder("a")
            .entry(
                "Other",
                zenkey::schema::TypeSchema::json_schema(serde_json::json!({"type":"object"})),
            )
            .build();
        let gaps = totality_gaps(&[("sysinfo".to_string(), incomplete)], &slices);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].missing, ["TelemetryPoint"]);

        // No describe served at all: not judged by totality.
        assert!(totality_gaps(&[], &slices).is_empty());
    }
}
