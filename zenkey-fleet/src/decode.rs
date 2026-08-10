//! The schema-aware decode seam (issues #11/#15): wire key → type name →
//! served schema → named-field JSON, with the honest fallbacks a generic
//! tool owes its user.
//!
//! [`SchemaStore`] caches each producer's served `describe` reply (RFC 08
//! §7) and fetches on first miss through a declared
//! [`crate::query::RepeatingQuery`] (the RFC 05 §2.1 discipline, kept warm
//! across the negative-TTL re-asks — #37). [`decode_sample`] is the whole
//! pipeline in one call; encoding resolution is **sample > registry > sniff**
//! and the sniff never goes away.

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
    /// producer → what we know about its `describe` (see [`Cached`]).
    ///
    /// A served set is behind an `Arc` because it is read **per sample**:
    /// handing out a deep clone of every type's document to answer "what is
    /// the schema for this one type" was the other half of issue #100's cost,
    /// and the quieter half — a descriptor pool rebuild at least looks
    /// expensive.
    sets: Mutex<HashMap<String, Cached>>,
    /// One declared querier per producer's describe key (#37), reused across
    /// the negative-TTL re-asks. Bounded by fleet producer count; entries
    /// live for the store's lifetime (no eviction — a fleet's producer set
    /// is small and a stale querier is only idle routing state).
    queriers: Mutex<HashMap<String, std::sync::Arc<crate::query::RepeatingQuery>>>,
    decoders: DecoderRegistry,
}

/// How long "asked, and answered with nothing usable" stays authoritative
/// before re-asking. A producer that genuinely serves no `describe` must not
/// be re-asked per sample, and 60s is the bound for that.
const NOT_SERVED_TTL: Duration = Duration::from_secs(60);

/// The first backoff after a GET that drew **zero replies** (issue #101).
///
/// Zero replies is the RFC 05 §3.1 non-verdict this codebase refuses to treat
/// as an answer anywhere else, and it is what an explorer started before its
/// fleet sees. Doubling from here, capped at [`NOT_SERVED_TTL`], means a
/// routing race resolves in well under a second while a producer that is
/// simply absent still converges on the same 60s bound.
const NO_REPLY_BACKOFF: Duration = Duration::from_millis(250);

/// Why a producer has no cached set, which decides how soon we re-ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MissReason {
    /// The GET returned no replies at all. Nobody said anything — including
    /// "no". Could be a producer that does not exist, or a connector whose
    /// GET went out before the producer's queryable was routable.
    NoReplies,
    /// Somebody replied, and nothing in the replies parsed as a `SchemaSet`.
    /// That *is* an answer about this producer, and it earns the full TTL.
    AnsweredUnusable,
}

/// A producer we asked and got nothing usable from.
#[derive(Debug, Clone, Copy)]
struct Missing {
    reason: MissReason,
    asked: std::time::Instant,
    /// Consecutive zero-reply asks, driving the backoff.
    attempts: u32,
}

impl Missing {
    /// How long this miss stays authoritative before the next ask.
    fn backoff(&self) -> Duration {
        match self.reason {
            MissReason::AnsweredUnusable => NOT_SERVED_TTL,
            MissReason::NoReplies => NO_REPLY_BACKOFF
                .saturating_mul(1u32 << self.attempts.saturating_sub(1).min(16))
                .min(NOT_SERVED_TTL),
        }
    }

    fn may_reask(&self) -> bool {
        self.asked.elapsed() >= self.backoff()
    }
}

/// What the store knows about one producer's `describe`.
enum Cached {
    Served(std::sync::Arc<SchemaSet>),
    Missing(Missing),
}

/// What one `describe` GET produced — the distinction issue #101 exists for.
enum Fetched {
    Served(SchemaSet),
    NoReplies,
    AnsweredUnusable,
}

impl SchemaStore {
    pub fn new(base: impl Into<String>, timeout: Duration) -> Self {
        SchemaStore {
            base: base.into(),
            timeout,
            sets: Mutex::new(HashMap::new()),
            queriers: Mutex::new(HashMap::new()),
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
        self.set_for(session, producer)
            .await
            .and_then(|set| set.get(type_name).cloned())
    }

    /// The producer's **whole** served set, on the same fetch-and-cache path
    /// as [`schema_for`](Self::schema_for) (issue #51: `zenctl schema
    /// <producer>` dumps the inventory, and asking type-by-type would be a
    /// different question than the one `describe` answers).
    ///
    /// `None` = the producer does not serve `describe` — an honest
    /// degradation, never an error.
    pub async fn set_for(
        &self,
        session: &Session,
        producer: &str,
    ) -> Option<std::sync::Arc<SchemaSet>> {
        // How many consecutive zero-reply asks precede this one — carried
        // across so the backoff actually grows.
        let attempts = {
            let sets = self.sets.lock().expect("store lock");
            match sets.get(producer) {
                Some(Cached::Served(set)) => return Some(std::sync::Arc::clone(set)),
                Some(Cached::Missing(m)) if !m.may_reask() => return None,
                Some(Cached::Missing(m)) => m.attempts,
                None => 0,
            }
        };
        let entry = match self.fetch(session, producer).await {
            Fetched::Served(set) => Cached::Served(std::sync::Arc::new(set)),
            Fetched::NoReplies => Cached::Missing(Missing {
                reason: MissReason::NoReplies,
                asked: std::time::Instant::now(),
                attempts: attempts.saturating_add(1),
            }),
            // An answer resets the streak: this is a verdict about the
            // producer, not a routing race.
            Fetched::AnsweredUnusable => Cached::Missing(Missing {
                reason: MissReason::AnsweredUnusable,
                asked: std::time::Instant::now(),
                attempts: 0,
            }),
        };
        let mut sets = self.sets.lock().expect("store lock");
        let served = match &entry {
            Cached::Served(set) => Some(std::sync::Arc::clone(set)),
            Cached::Missing(_) => None,
        };
        sets.insert(producer.to_string(), entry);
        served
    }

    /// Forget what we learned about one producer, so the next question goes
    /// to the bus (issue #101).
    ///
    /// The queriers are kept: they are idle routing state, and re-declaring
    /// them is exactly the cost #37 removed.
    pub fn forget(&self, producer: &str) {
        self.sets.lock().expect("store lock").remove(producer);
    }

    /// Forget every producer — the "re-ask schemas" action a frontend offers.
    ///
    /// Covers the case the backoff cannot: a *positive* entry never expires,
    /// so a producer that changes its served set mid-session is otherwise
    /// read with the schemas it had at first contact.
    pub fn forget_all(&self) {
        self.sets.lock().expect("store lock").clear();
    }

    /// Producers currently answered-for, and whether each served a set —
    /// what a frontend shows next to its re-ask button.
    pub fn known(&self) -> Vec<(String, bool)> {
        let sets = self.sets.lock().expect("store lock");
        let mut out: Vec<(String, bool)> = sets
            .iter()
            .map(|(p, c)| (p.clone(), matches!(c, Cached::Served(_))))
            .collect();
        out.sort();
        out
    }

    async fn fetch(&self, session: &Session, producer: &str) -> Fetched {
        let cached = {
            let queriers = self.queriers.lock().expect("querier lock");
            queriers.get(producer).cloned()
        };
        let querier = match cached {
            Some(q) => q,
            None => {
                let key = zenkey::grammar::with_base(
                    &self.base,
                    zenkey::selector::fleet_rpc(producer, &["describe"]),
                );
                let declared =
                    match crate::query::declare_repeating(session, &self.base, &key, self.timeout)
                        .await
                    {
                        Ok(q) => std::sync::Arc::new(q),
                        // We could not even ask. Nobody said anything about
                        // this producer, so this is the non-verdict case, not
                        // a 60s verdict.
                        Err(_) => return Fetched::NoReplies,
                    };
                // A concurrent miss may have declared first; keep whichever
                // landed (the loser undeclares itself on drop — idle state,
                // not a leak).
                let mut queriers = self.queriers.lock().expect("querier lock");
                queriers
                    .entry(producer.to_string())
                    .or_insert(declared)
                    .clone()
            }
        };
        let Ok(answers) = querier.fetch().await else {
            return Fetched::NoReplies;
        };
        if answers.is_empty() {
            return Fetched::NoReplies;
        }
        // Any well-formed reply will do; hashes make same-name drift a
        // doctor finding, not a decode concern.
        for a in answers {
            if let crate::query::Answer::Value(bytes) = a.answer {
                let cow = bytes.to_bytes();
                if let Ok(text) = std::str::from_utf8(&cow)
                    && let Ok(set) = SchemaSet::parse(text)
                {
                    return Fetched::Served(set);
                }
            }
        }
        // Somebody answered — with an error, or with something that is not a
        // SchemaSet. That is a statement about this producer.
        Fetched::AnsweredUnusable
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

    /// The other direction (issue #97): a JSON value framed for the wire.
    /// The store owns the decoder table, so the write path resolves its codec
    /// exactly where the read path does — one registration, both directions.
    pub fn encode(
        &self,
        schema: &TypeSchema,
        value: &serde_json::Value,
        target: &WireEncoding,
    ) -> Result<Vec<u8>, DecodeError> {
        self.decoders.encode(schema, value, target)
    }
}

/// The registry type names one producer's slice references — RFC 08 §7's
/// totality set for that producer.
fn referenced_types(slice: &zenkey::slice::RegistrySlice) -> Vec<String> {
    let mut names: Vec<&str> = slice
        .subjects
        .iter()
        .map(|s| s.type_name.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    for p in &slice.procedures {
        names.extend(p.request.as_deref());
        names.extend(p.reply.as_deref());
    }
    for b in &slice.blob {
        names.extend(b.reference.as_deref());
    }
    names.sort_unstable();
    names.dedup();
    names.into_iter().map(str::to_string).collect()
}

/// One type's schema, as a report row.
fn row(
    producer: &str,
    type_name: &str,
    schema: &TypeSchema,
    full: bool,
) -> crate::report::SchemaRow {
    crate::report::SchemaRow {
        producer: producer.to_string(),
        type_name: type_name.to_string(),
        kind: schema.kind().as_str().to_string(),
        hash: schema.hash().to_string(),
        document: full.then(|| schema_document(schema)),
    }
}

/// A schema's document in a renderable form. `json-schema` has one natively;
/// every other kind is summarised structurally rather than faked — a codec
/// this build cannot read still gets to say what it is.
fn schema_document(schema: &TypeSchema) -> serde_json::Value {
    if let Some(doc) = schema.json_document() {
        return doc.clone();
    }
    let mut obj = serde_json::Map::new();
    obj.insert(
        "kind".into(),
        serde_json::Value::String(schema.kind().as_str().to_string()),
    );
    if let Some(m) = schema.protobuf_message() {
        obj.insert("message".into(), serde_json::Value::String(m.to_string()));
    }
    if let Some(bytes) = schema.protobuf_descriptor_set() {
        obj.insert(
            "descriptor_set_bytes".into(),
            serde_json::Value::from(bytes.len()),
        );
    }
    if let Some(fields) = schema.cdr_fields() {
        obj.insert("fields".into(), fields.clone());
    }
    if let Some(types) = schema.cdr_types() {
        obj.insert("types".into(), serde_json::Value::Object(types.clone()));
    }
    serde_json::Value::Object(obj)
}

/// Dump one producer's served `describe` reply (issue #51), joined against
/// its registry slice so the RFC 08 §7 totality gap is visible where the user
/// is already looking.
///
/// A producer serving no `describe` yields `served: false` — the honest
/// degradation, never an error: §7 is a SHOULD, and silence about a type is
/// not a claim about it.
pub async fn schema_dump(
    store: &SchemaStore,
    session: &Session,
    slices: &SliceSet,
    producer: &str,
    type_filter: Option<&str>,
    full: bool,
) -> crate::report::SchemaDump {
    let set = store.set_for(session, producer).await;
    let Some(set) = set else {
        return crate::report::SchemaDump {
            producer: producer.to_string(),
            served: false,
            app: None,
            types: Vec::new(),
            missing: Vec::new(),
        };
    };
    let types: Vec<crate::report::SchemaRow> = set
        .iter()
        .filter(|(name, _)| type_filter.is_none_or(|f| f == *name))
        .map(|(name, schema)| row(producer, name, schema, full || type_filter.is_some()))
        .collect();
    let missing = slices
        .get(producer)
        .map(|slice| {
            referenced_types(slice)
                .into_iter()
                .filter(|n| set.get(n).is_none())
                .collect()
        })
        .unwrap_or_default();
    crate::report::SchemaDump {
        producer: producer.to_string(),
        served: true,
        app: Some(set.app().to_string()),
        types,
        missing,
    }
}

/// Every producer's schema for one type name (issue #51's `interface show
/// --schema`). Asking all of them is the point: same name, different hash is
/// RFC 08 §7's drift finding, and the type's own page is where it is worth
/// seeing.
pub async fn schemas_for_type(
    store: &SchemaStore,
    session: &Session,
    producers: &[String],
    type_name: &str,
    full: bool,
) -> Vec<crate::report::SchemaRow> {
    let mut out = Vec::new();
    for producer in producers {
        if let Some(schema) = store.schema_for(session, producer, type_name).await {
            out.push(row(producer, type_name, &schema, full));
        }
    }
    out
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
        // An untyped subject (empty `type`) references nothing — without this
        // filter it would demand a schema for "" and report a phantom gap.
        names.extend(
            slice
                .subjects
                .iter()
                .map(|s| s.type_name.as_str())
                .filter(|t| !t.is_empty()),
        );
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

/// The structural sniff as a **value** rather than as text — the same ladder
/// [`structural`] renders, stopped one step earlier.
///
/// `Some` means the bytes carry a self-describing document (JSON, or CBOR that
/// accounts for every byte and is not the text-vs-scalar ambiguity below).
/// `None` means they do not: plain text, or opaque bytes. That distinction is
/// what lets a caller diff two payloads field-by-field when it can, and say so
/// honestly — a byte comparison — when it cannot.
///
/// Deliberately sync and schema-free: this runs on render paths, where the
/// async [`decode_sample`] (which may GET a `describe` on a miss) must never
/// sit.
pub fn structural_value(bytes: &[u8]) -> Option<serde_json::Value> {
    let looks_json = bytes.first().is_some_and(|b| {
        matches!(
            b,
            b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
        )
    });
    if looks_json && let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        return Some(v);
    }
    let is_text = std::str::from_utf8(bytes).is_ok_and(|t| !t.is_empty());
    if let Some(v) = cbor_whole(bytes)
        // A bare CBOR scalar over bytes that are *also* valid text is the
        // ambiguous case, and plain text is the likelier reading on a bus that
        // carries anything. Structured CBOR (a map, an array) is unambiguous
        // and still wins.
        && !(is_text && is_scalar(&v))
        // A CBOR map keyed by anything but strings has no JSON form; that is a
        // failure of the *rendering*, not of the payload, so it degrades to
        // text like any other unreadable shape rather than being invented.
        && let Ok(value) = serde_json::to_value(&v)
    {
        return Some(value);
    }
    None
}

/// Structural fallback rendering — what the wire honestly says when no
/// schema resolves.
pub fn structural(bytes: &[u8]) -> String {
    if let Some(v) = structural_value(bytes) {
        return serde_json::to_string(&v).unwrap_or_default();
    }
    match std::str::from_utf8(bytes).ok().filter(|t| !t.is_empty()) {
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

    /// The value form answers the question a diff actually asks: is there a
    /// document here to compare field by field, or only bytes?
    #[test]
    fn structural_value_yields_documents_and_nothing_else() {
        assert_eq!(
            structural_value(br#"{"value":42.0}"#),
            Some(serde_json::json!({"value": 42.0}))
        );
        let mut cbor = Vec::new();
        ciborium::into_writer(&serde_json::json!({"x": 1}), &mut cbor).unwrap();
        assert_eq!(structural_value(&cbor), Some(serde_json::json!({"x": 1})));
        // Plain text and opaque bytes are not documents — the caller falls
        // back to a byte comparison rather than being handed a fake one.
        assert_eq!(structural_value(b"just a plain string"), None);
        assert_eq!(structural_value(&[0xff, 0xfe, 0x00]), None);
        assert_eq!(structural_value(b""), None);
    }

    /// The two must not drift: `structural` is the rendering of
    /// `structural_value` wherever one exists.
    #[test]
    fn the_rendering_agrees_with_the_value() {
        for payload in [
            &br#"{"a":1}"#[..],
            &b"[1,2,3]"[..],
            &b"just a plain string"[..],
            &[0xff, 0xfe, 0x00][..],
        ] {
            if let Some(v) = structural_value(payload) {
                assert_eq!(structural(payload), serde_json::to_string(&v).unwrap());
            }
        }
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
                common: None,
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

    /// An untyped subject (empty `type`) references nothing — it must not
    /// demand a schema for `""` (regression: phantom gap found while
    /// consolidating doctor's totality check onto this function, #55).
    #[test]
    fn an_untyped_subject_is_not_a_totality_gap() {
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let slice = RegistrySlice {
            version: "1".into(),
            app: "a".into(),
            convention: 1,
            name: "sysinfo".into(),
            service_origin: None,
            description: None,
            subjects: vec![SubjectDecl {
                path: "raw".into(),
                class: "telemetry".into(),
                type_name: String::new(),
                common: None,
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
        let served = SchemaSet::builder("a").build();
        assert!(
            totality_gaps(&[("sysinfo".to_string(), served)], &slices).is_empty(),
            "empty type names must be filtered, not reported as gaps"
        );
    }

    /// Issue #101: the two ways of learning nothing are different facts and
    /// must not share a bound. Zero replies is the RFC 05 §3.1 non-verdict —
    /// it backs off in milliseconds and grows; an answer that served nothing
    /// usable keeps the full 60s.
    #[test]
    fn a_zero_reply_ask_backs_off_fast_and_an_answered_one_does_not() {
        let now = std::time::Instant::now();
        let no_reply = |attempts| Missing {
            reason: MissReason::NoReplies,
            asked: now,
            attempts,
        };
        assert_eq!(no_reply(1).backoff(), NO_REPLY_BACKOFF);
        assert_eq!(no_reply(2).backoff(), NO_REPLY_BACKOFF * 2);
        assert_eq!(no_reply(3).backoff(), NO_REPLY_BACKOFF * 4);
        // …and it converges on the same bound a genuinely absent producer
        // deserves, rather than re-asking forever.
        assert_eq!(no_reply(30).backoff(), NOT_SERVED_TTL);

        let answered = Missing {
            reason: MissReason::AnsweredUnusable,
            asked: now,
            attempts: 0,
        };
        assert_eq!(
            answered.backoff(),
            NOT_SERVED_TTL,
            "a producer that answered and served nothing is asked once per TTL"
        );
    }

    /// The first zero-reply backoff must be short enough that an explorer
    /// started before its fleet is not blind for a human-noticeable time.
    #[test]
    fn the_first_reask_is_sub_second() {
        let m = Missing {
            reason: MissReason::NoReplies,
            asked: std::time::Instant::now(),
            attempts: 1,
        };
        assert!(m.backoff() < Duration::from_secs(1));
        assert!(!m.may_reask(), "and not before it elapses");
    }
}
