//! Preparing a body for the wire (issue #97) — the encode half of the codec
//! seam, on the path a write actually takes.
//!
//! Until this module, both frontends *validated* an outgoing body by encoding
//! it against the producer's served schema and then **threw the encoded bytes
//! away**, putting the operator's JSON text on the wire. Everything downstream
//! was therefore a lie for any subject whose declared encoding was not JSON: a
//! `application/protobuf` subject could be described, refined, decoded — and
//! not published to. The fix is one seam, here, so that "the body was checked"
//! and "the body was encoded" stop being two different things.
//!
//! Three obligations this owes its callers, all of them RFC 09 §5.1 O4 in
//! different clothes:
//!
//! - a body that was **not** encoded says so ([`BodySource`]) — publishing
//!   as-typed is a legitimate outcome, silently publishing as-typed is not;
//! - the wire `Encoding` is resolved from what was *declared*, never sniffed
//!   off the operator's text ([`encode_encoding`]);
//! - a refusal happens **before** the bus, and the caller can opt out of the
//!   refusal ([`PrepareMode`]) without opting out of the labelling.

use anyhow::{Result, anyhow};
use zenkey::schema::{SchemaKind, TypeSchema, WireEncoding};
use zenoh::Session;

use crate::decode::SchemaStore;
use crate::registry::SliceSet;

/// How the bytes on the wire came to be — carried out of [`prepare_publish`]
/// so a frontend can say it, not guess it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodySource {
    /// Encoded through the producer's served schema for this type name.
    Encoded { type_name: String },
    /// No schema resolved — an unregistered key, an untyped subject, or a
    /// producer that serves no `describe`. The body ships as the caller typed
    /// it, which is honest only because it is labelled.
    AsTyped,
    /// The caller asked for verbatim bytes ([`PrepareMode::Raw`]).
    Raw,
}

/// What to do when a schema resolves and the body does not fit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepareMode {
    /// Refuse before the bus (`zenctl` default).
    Encode,
    /// Try to encode; on failure ship the body as typed, with a note
    /// (`--no-validate`). "Do not refuse" — never "do not tell me".
    Lenient,
    /// Never encode; ship verbatim (`--raw`).
    Raw,
}

/// A body ready for the wire, with the provenance a caller must surface.
#[derive(Debug, Clone)]
pub struct PreparedBody {
    pub bytes: Vec<u8>,
    /// The wire `Encoding` to set, when one is known. `None` means nothing was
    /// declared anywhere — the publisher says nothing rather than guessing.
    pub encoding: Option<String>,
    pub source: BodySource,
    /// A line for the caller to print/render verbatim. `None` when the
    /// ordinary thing happened (encoded against a served schema).
    pub note: Option<String>,
}

impl PreparedBody {
    fn raw(bytes: Vec<u8>, encoding: Option<String>, note: Option<String>) -> PreparedBody {
        PreparedBody {
            bytes,
            encoding,
            source: BodySource::Raw,
            note,
        }
    }
}

/// The encoding a body should be **encoded into**, which is a different
/// question from the one [`crate::decode::resolve_encoding`] answers.
///
/// Decoding resolves *sample > registry > sniff* because a received payload
/// has bytes to sniff. An outgoing body has none that mean anything — the
/// operator typed JSON whatever the subject carries, so sniffing it would
/// label every protobuf subject `application/json`. The ladder is therefore
/// **declared flag > registry `encoding` > the schema kind's native
/// encoding**, and the kind is the authority of last resort precisely because
/// it is the one thing that cannot be wrong.
pub fn encode_encoding(
    declared: Option<&str>,
    registry: Option<&str>,
    schema: Option<&TypeSchema>,
) -> Option<String> {
    if let Some(e) = declared {
        return Some(e.to_string());
    }
    if let Some(e) = registry {
        return Some(e.to_string());
    }
    schema.and_then(|s| match s.kind().as_str() {
        SchemaKind::JSON_SCHEMA => Some("application/json".to_string()),
        SchemaKind::PROTOBUF => Some("application/protobuf".to_string()),
        // An unknown kind's native framing is exactly what this tool does not
        // know. Saying nothing beats naming the wrong one.
        _ => None,
    })
}

/// Encode `body` against `producer`'s served schema for `type_name`, or
/// explain why it could not.
///
/// `Ok` with [`BodySource::AsTyped`] when no schema resolves — that is not a
/// failure, and it is not silence either (RFC 08 §7 is a SHOULD; a producer
/// that serves no `describe` has said nothing about this type, which is
/// different from having said "any bytes will do").
#[allow(clippy::too_many_arguments)]
pub async fn prepare_request(
    session: &Session,
    store: &SchemaStore,
    producer: &str,
    type_name: &str,
    declared_encoding: Option<&str>,
    registry_encoding: Option<&str>,
    body: &[u8],
    mode: PrepareMode,
) -> Result<PreparedBody> {
    if mode == PrepareMode::Raw {
        return Ok(PreparedBody::raw(
            body.to_vec(),
            encode_encoding(declared_encoding, registry_encoding, None),
            Some("raw: bytes sent verbatim, not encoded against the served schema".into()),
        ));
    }
    let schema = store.schema_for(session, producer, type_name).await;
    let encoding = encode_encoding(declared_encoding, registry_encoding, schema.as_ref());
    let Some(schema) = schema else {
        return Ok(PreparedBody {
            bytes: body.to_vec(),
            encoding,
            source: BodySource::AsTyped,
            note: Some(format!(
                "{producer} serves no schema for {type_name} — body sent as typed, unchecked \
                 (RFC 08 §7 describe is a SHOULD; \"not served\" is not \"anything goes\")"
            )),
        });
    };

    let lenient = mode == PrepareMode::Lenient;
    let value: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) if lenient => {
            return Ok(PreparedBody {
                bytes: body.to_vec(),
                encoding,
                source: BodySource::AsTyped,
                note: Some(format!(
                    "body is not JSON, so it could not be encoded as {type_name} ({e}) — \
                     sent as typed"
                )),
            });
        }
        Err(e) => {
            return Err(anyhow!(
                "body is not JSON but {producer} declares schema-validated type {type_name} — {e}"
            ));
        }
    };

    let target = encoding
        .as_deref()
        .map(WireEncoding::from_encoding_str)
        // With nothing declared anywhere the target is the schema's own kind,
        // and for `json-schema` that is JSON — the framing an operator typed.
        .unwrap_or(WireEncoding::Json);
    match store.encode(&schema, &value, &target) {
        Ok(bytes) => Ok(PreparedBody {
            bytes,
            encoding,
            source: BodySource::Encoded {
                type_name: type_name.to_string(),
            },
            note: None,
        }),
        Err(e) if lenient => Ok(PreparedBody {
            bytes: body.to_vec(),
            encoding,
            source: BodySource::AsTyped,
            note: Some(format!(
                "body rejected by {type_name}'s served schema ({e}) — sent as typed anyway"
            )),
        }),
        Err(e) => Err(anyhow!("body rejected by {type_name}'s served schema: {e}")),
    }
}

/// The publish-side entry point: refine a **full wire key** against the loaded
/// slices, then [`prepare_request`] on whatever type it names.
///
/// An unregistered key is not an error — it is the ordinary case on a bus this
/// convention does not govern, and the note says which case happened.
#[allow(clippy::too_many_arguments)]
pub async fn prepare_publish(
    session: &Session,
    store: &SchemaStore,
    slices: Option<&SliceSet>,
    base: &str,
    wire_key: &str,
    declared_encoding: Option<&str>,
    body: &[u8],
    mode: PrepareMode,
) -> Result<PreparedBody> {
    if mode == PrepareMode::Raw {
        return Ok(PreparedBody::raw(
            body.to_vec(),
            declared_encoding.map(str::to_string),
            Some("raw: bytes sent verbatim, not encoded against the served schema".into()),
        ));
    }

    let description = crate::facts::describe_key(base, wire_key, slices);
    let crate::facts::Registration::Registered(subject) = &description.facts.registration else {
        return Ok(PreparedBody {
            bytes: body.to_vec(),
            encoding: declared_encoding.map(str::to_string),
            source: BodySource::AsTyped,
            note: Some(match slices {
                // O4: with no slices loaded the tool has not asked, and
                // "not asked" is not "unregistered".
                None => format!(
                    "no registry loaded, so {wire_key} was never classified — body sent as typed"
                ),
                Some(_) => format!(
                    "{wire_key} is not a registered subject ({:?}) — body sent as typed",
                    description.facts.registration
                ),
            }),
        });
    };
    let Some(producer) = subject_producer(&description) else {
        return Ok(PreparedBody {
            bytes: body.to_vec(),
            encoding: encode_encoding(declared_encoding, subject.encoding.as_deref(), None),
            source: BodySource::AsTyped,
            note: Some(format!(
                "{wire_key} refines to a registered subject with no producer chunk to ask for a \
                 schema — body sent as typed"
            )),
        });
    };
    if subject.type_name.is_empty() {
        return Ok(PreparedBody {
            bytes: body.to_vec(),
            encoding: encode_encoding(declared_encoding, subject.encoding.as_deref(), None),
            source: BodySource::AsTyped,
            note: Some(format!(
                "{wire_key} is registered but declares no payload type — body sent as typed"
            )),
        });
    }

    prepare_request(
        session,
        store,
        &producer,
        &subject.type_name,
        declared_encoding,
        subject.encoding.as_deref(),
        body,
        mode,
    )
    .await
}

/// The producer a registered description refined through. `SubjectFacts` does
/// not carry it (a service slice's name is not a key chunk), so it is derived
/// from the key shape.
pub fn subject_producer(description: &crate::facts::KeyDescription) -> Option<String> {
    match &description.facts.shape {
        crate::facts::KeyShape::V1(v) => v.producer.clone(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_encode_ladder_never_sniffs_the_operators_text() {
        let protobuf = TypeSchema::protobuf("t.Blob", b"\x0a\x00");
        // Nothing declared: the kind decides — not the JSON the operator typed.
        assert_eq!(
            encode_encoding(None, None, Some(&protobuf)).as_deref(),
            Some("application/protobuf")
        );
        // The registry outranks the kind…
        assert_eq!(
            encode_encoding(None, Some("application/cbor"), Some(&protobuf)).as_deref(),
            Some("application/cbor")
        );
        // …and the flag outranks the registry.
        assert_eq!(
            encode_encoding(Some("application/json"), Some("application/cbor"), None).as_deref(),
            Some("application/json")
        );
        // An unknown kind's framing is unknown, and saying nothing is the
        // honest answer (O4).
        let json = TypeSchema::json_schema(json!({"type": "object"}));
        assert_eq!(
            encode_encoding(None, None, Some(&json)).as_deref(),
            Some("application/json")
        );
        assert_eq!(encode_encoding(None, None, None), None);
    }
}
