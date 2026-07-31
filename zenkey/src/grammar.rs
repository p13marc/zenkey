//! Chunk lexical rules, reserved tokens, and structural key assembly/parsing.
//!
//! Normative source: RFC 03 (`rfcs/03-grammar.md`). Every
//! rule here cites its section. Keys are **base-relative**: they start at the
//! `v1` version chunk; the deployment base rides the session namespace
//! (RFC 03 §1.1) and never appears in application-built keys.

use std::fmt;

use crate::key::Key;

/// The convention major this crate implements (RFC 03 §1.2).
///
/// **Plain, not verbatim** — and that is load-bearing, not an oversight.
///
/// It was `@v1` through the migration. A verbatim chunk is invisible to `*`
/// and `**`, which bought *legacy hermeticity*: the pre-v1 firehose selector
/// `zensight/**` could not see v1 keys, so an un-migrated consumer could not
/// receive samples it would then mis-decode. That mattered exactly once, during
/// the cutover, and the pre-v1 keyspace is now retired.
///
/// What it cost was permanent. zenoh-ext's advanced tier parks its
/// publisher-detection tokens at `<key>/@adv/pub/<zid>/<eid>/…` and parses them
/// with `${remaining:**}/@adv/…` — and `**` never matches a chunk beginning
/// with `@`. So `remaining` could not span a key containing `@v1`: **every**
/// `@adv` token we declared was unparseable by the only thing that reads them.
/// `detect_late_publishers()` was silently dead and every subscriber's log
/// filled with "malformed liveliness token key expression". No upstream fix is
/// possible with a wildcard — the `@`-exclusion is a Zenoh matching rule.
///
/// A plain `v1` costs nothing that survives the migration and restores the
/// advanced tier, because the property we actually wanted is **version
/// isolation**, and that never depended on the `@`: `v1` and `v2` are different
/// literal chunks, so a `v1/**` selector can never match a v2 key. The chunks
/// that stay verbatim are the ones still doing daily work — the planes
/// (`@rpc`/`@media`/`@blob`, design property D2) and service origins
/// (`@catalog`, D4).
///
/// Pinned by `tests/adv_token.rs` (the token must parse) and
/// `tests/guard.rs::d1_version_isolation`.
pub const VERSION_CHUNK: &str = "v1";

/// Data classes (RFC 03 §1.4). Plain chunks — they participate in wildcards.
pub const CLASS_TELEMETRY: &str = "telemetry";
pub const CLASS_STATE: &str = "state";
pub const CLASS_EVENTS: &str = "events";

/// Verbatim planes (RFC 03 §1.4). Hermetic — no `*`/`**` reaches them.
pub const PLANE_RPC: &str = "@rpc";
pub const PLANE_MEDIA: &str = "@media";
pub const PLANE_BLOB: &str = "@blob";

/// The reserved service origin (RFC 03 §3).
pub const SERVICE_CATALOG: &str = "@catalog";

/// Blob tier tokens — position 5 under `@blob` (RFC 03 §1.5, 07 §2).
pub const BLOB_TIER_ARTIFACT: &str = "artifact";
pub const BLOB_TIER_TREE: &str = "tree";
pub const BLOB_TIER_STORE: &str = "store";

/// Reserved liveliness subject leaf (RFC 03 §3, 04 §5). Never a data subject.
pub const SUBJECT_ALIVE: &str = "alive";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("invalid plain chunk {0:?}: must match [a-z0-9]([a-z0-9._-]*[a-z0-9])? (RFC 03 §2)")]
    InvalidPlainChunk(String),
    #[error("invalid verbatim chunk {0:?}: must match @[a-z0-9][a-z0-9_-]* (RFC 03 §2)")]
    InvalidVerbatimChunk(String),
    #[error("invalid host origin {0:?}: must match h-[0-9a-f]{{12}} (RFC 03 §1.3)")]
    InvalidHostOrigin(String),
    #[error("invalid producer {0:?}: {1} (RFC 03 §1.5)")]
    InvalidProducer(String, &'static str),
    #[error("empty subject: keys need >= 1 subject chunk (RFC 03 §1.6)")]
    EmptySubject,
    #[error("blob tier token expected (artifact|tree|store), got {0:?} (RFC 03 §1.5)")]
    InvalidBlobTier(String),
    #[error("reserved token {0:?} may not be used as a {1} (RFC 03 §3)")]
    ReservedToken(String, &'static str),
    #[error(
        "invalid content hash {0:?}: must be lowercase hex, even length, 8..=128 digits (RFC 07 §2.3/§2.4)"
    )]
    InvalidContentHash(String),
    #[error("malformed @blob/{0} key: {1} (RFC 07 §2)")]
    MalformedBlobKey(&'static str, &'static str),
    #[error("not a v1 key: {0}")]
    Parse(String),
}

// Chunk lexical rules (RFC 03 §2, §1.3). The registry linter (`zenkey-build`)
// calls these same functions, so codegen and runtime validate with
// byte-identical rules.

/// RFC 03 §2: `[a-z0-9]([a-z0-9._-]*[a-z0-9])?` — lowercase, must start and
/// end alphanumeric, no wildcards, no `%`, no uppercase.
pub fn is_valid_plain_chunk(chunk: &str) -> bool {
    let bytes = chunk.as_bytes();
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    match bytes {
        [] => false,
        [one] => alnum(*one),
        [first, mid @ .., last] => {
            alnum(*first)
                && alnum(*last)
                && mid
                    .iter()
                    .all(|&b| alnum(b) || b == b'.' || b == b'_' || b == b'-')
        }
    }
}

/// RFC 03 §2: `@[a-z0-9][a-z0-9_-]*` (the `@v<int>` version form is a special
/// case of this shape).
pub fn is_valid_verbatim_chunk(chunk: &str) -> bool {
    let Some(rest) = chunk.strip_prefix('@') else {
        return false;
    };
    let bytes = rest.as_bytes();
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    match bytes {
        [] => false,
        [first, rest @ ..] => {
            alnum(*first) && rest.iter().all(|&b| alnum(b) || b == b'_' || b == b'-')
        }
    }
}

/// RFC 03 §1.3: host origins MUST match `h-[0-9a-f]{12}` exactly.
pub fn is_valid_host_origin(chunk: &str) -> bool {
    let Some(hex) = chunk.strip_prefix("h-") else {
        return false;
    };
    hex.len() == 12
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The publishing identity in position 3 (RFC 03 §1.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// `h-<12hex>` — a machine (see [`crate::origin`] for minting).
    Host(crate::origin::HostId),
    /// A verbatim service origin, e.g. `@catalog`. Producer chunk omitted.
    Service(String),
}

impl Origin {
    pub fn catalog() -> Self {
        Origin::Service(SERVICE_CATALOG.to_string())
    }

    pub fn service(name: &str) -> Result<Self, KeyError> {
        if !is_valid_verbatim_chunk(name) {
            return Err(KeyError::InvalidVerbatimChunk(name.to_string()));
        }
        Ok(Origin::Service(name.to_string()))
    }

    pub fn chunk(&self) -> &str {
        match self {
            Origin::Host(id) => id.as_str(),
            Origin::Service(s) => s,
        }
    }

    /// Service origins omit the producer position (RFC 03 §1.5).
    pub fn has_producer_chunk(&self) -> bool {
        matches!(self, Origin::Host(_))
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.chunk())
    }
}

/// The producing component: `<name>` or `<name>-<instance>` (RFC 03 §1.5).
///
/// The instance suffix is `-<positive int>` and base names MUST NOT end in
/// `-<int>`, so the chunk parses back unambiguously.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Producer {
    name: String,
    instance: Option<u32>,
}

impl Producer {
    pub fn new(name: &str) -> Result<Self, KeyError> {
        Self::validate_name(name)?;
        Ok(Producer {
            name: name.to_string(),
            instance: None,
        })
    }

    pub fn with_instance(name: &str, instance: u32) -> Result<Self, KeyError> {
        Self::validate_name(name)?;
        if instance == 0 {
            return Err(KeyError::InvalidProducer(
                name.to_string(),
                "instance numbers start at 1 (the first instance uses the bare name)",
            ));
        }
        Ok(Producer {
            name: name.to_string(),
            instance: Some(instance),
        })
    }

    fn validate_name(name: &str) -> Result<(), KeyError> {
        if !is_valid_plain_chunk(name) {
            return Err(KeyError::InvalidProducer(
                name.to_string(),
                "not a valid plain chunk",
            ));
        }
        if Self::split_trailing_int(name).is_some() {
            return Err(KeyError::InvalidProducer(
                name.to_string(),
                "base names must not end in -<int> (reserved for instance suffixes)",
            ));
        }
        if name == BLOB_TIER_ARTIFACT || name == BLOB_TIER_TREE || name == BLOB_TIER_STORE {
            return Err(KeyError::ReservedToken(name.to_string(), "producer name"));
        }
        Ok(())
    }

    fn split_trailing_int(chunk: &str) -> Option<(&str, u32)> {
        let (base, tail) = chunk.rsplit_once('-')?;
        if base.is_empty() || tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        tail.parse().ok().map(|n| (base, n))
    }

    /// Parse a producer chunk back into (name, instance) — RFC 03 §1.5.
    pub fn parse_chunk(chunk: &str) -> Result<Self, KeyError> {
        if !is_valid_plain_chunk(chunk) {
            return Err(KeyError::InvalidProducer(
                chunk.to_string(),
                "not a valid plain chunk",
            ));
        }
        match Self::split_trailing_int(chunk) {
            Some((base, n)) if n >= 1 => Ok(Producer {
                name: base.to_string(),
                instance: Some(n),
            }),
            _ => Ok(Producer {
                name: chunk.to_string(),
                instance: None,
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn instance(&self) -> Option<u32> {
        self.instance
    }

    /// Append the producer chunk to a key under construction without an
    /// intermediate allocation (the builders' hot path).
    pub(crate) fn push_chunk(&self, out: &mut String) {
        out.push_str(&self.name);
        if let Some(i) = self.instance {
            use std::fmt::Write as _;
            let _ = write!(out, "-{i}");
        }
    }

    pub fn chunk(&self) -> String {
        match self.instance {
            None => self.name.clone(),
            Some(n) => format!("{}-{n}", self.name),
        }
    }
}

/// Data classes (RFC 03 §1.4 / 04 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Telemetry,
    State,
    Events,
}

impl Class {
    pub fn chunk(self) -> &'static str {
        match self {
            Class::Telemetry => CLASS_TELEMETRY,
            Class::State => CLASS_STATE,
            Class::Events => CLASS_EVENTS,
        }
    }

    pub fn from_chunk(chunk: &str) -> Option<Self> {
        match chunk {
            CLASS_TELEMETRY => Some(Class::Telemetry),
            CLASS_STATE => Some(Class::State),
            CLASS_EVENTS => Some(Class::Events),
            _ => None,
        }
    }
}

/// Verbatim planes (RFC 03 §1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plane {
    Rpc,
    Media,
    Blob,
}

impl Plane {
    pub fn chunk(self) -> &'static str {
        match self {
            Plane::Rpc => PLANE_RPC,
            Plane::Media => PLANE_MEDIA,
            Plane::Blob => PLANE_BLOB,
        }
    }

    pub fn from_chunk(chunk: &str) -> Option<Self> {
        match chunk {
            PLANE_RPC => Some(Plane::Rpc),
            PLANE_MEDIA => Some(Plane::Media),
            PLANE_BLOB => Some(Plane::Blob),
            _ => None,
        }
    }
}

/// Position 4: a data class or a verbatim plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassOrPlane {
    Class(Class),
    Plane(Plane),
}

/// Blob tier token (position 5 under `@blob`, RFC 03 §1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlobTier {
    Artifact,
    Tree,
    Store,
}

impl BlobTier {
    pub fn chunk(self) -> &'static str {
        match self {
            BlobTier::Artifact => BLOB_TIER_ARTIFACT,
            BlobTier::Tree => BLOB_TIER_TREE,
            BlobTier::Store => BLOB_TIER_STORE,
        }
    }

    pub fn from_chunk(chunk: &str) -> Option<Self> {
        match chunk {
            BLOB_TIER_ARTIFACT => Some(BlobTier::Artifact),
            BLOB_TIER_TREE => Some(BlobTier::Tree),
            BLOB_TIER_STORE => Some(BlobTier::Store),
            _ => None,
        }
    }
}

/// A content address: the hex digest naming an immutable `@blob` object
/// (RFC 07 §2.3 tree roots, §2.4 chunk hashes).
///
/// This type exists so the convention's content-addressing rule is
/// **structural rather than advisory** — the same move that makes a fan-out
/// write unspellable by keeping [`Fleet`](crate::Fleet) out of
/// [`ConcreteOrigin`](crate::ConcreteOrigin). RFC 07 v1.2 *asserted* that
/// tree ids were root hashes and rested fleet-wide cacheability, the storage
/// PUT exemption, and no-op last-writer-wins on it — but nothing enforced it,
/// so a caller-chosen name (`tree/nightly`) silently falsified all three.
/// v1.7 made the rule normative; this type is what makes it hold.
///
/// The check is lexical (lowercase hex, even length, 8..=128 digits): it
/// rejects *names*, which is the realistic mistake. It is not an
/// authenticity check — that is the consumer's job, and it is why the
/// address must be verified against the bytes on receipt.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(String);

impl ContentHash {
    /// Validate a hex digest.
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        let ok = (8..=128).contains(&s.len())
            && s.len().is_multiple_of(2)
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if ok {
            Ok(ContentHash(s.to_string()))
        } else {
            Err(KeyError::InvalidContentHash(s.to_string()))
        }
    }

    /// The digest as it appears in a key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "serde")]
mod content_hash_serde {
    //! Wire representation (feature `serde`): the plain hex digest, validated
    //! on deserialize — same posture as the origin types. Payloads that
    //! reference a blob carry its content root (RFC 07 §2.1), so the address
    //! is checked where it enters the program, not at each use site.
    use super::ContentHash;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    impl Serialize for ContentHash {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for ContentHash {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let raw = String::deserialize(d)?;
            ContentHash::parse(&raw).map_err(D::Error::custom)
        }
    }
}

/// The schema mirrors [`ContentHash::parse`] exactly: lowercase hex pairs,
/// 8..=128 digits.
#[cfg(feature = "schemars")]
impl schemars::JsonSchema for ContentHash {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ContentHash".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^(?:[0-9a-f]{2}){4,64}$"
        })
    }
}

#[cfg(test)]
mod content_addressing {
    use super::*;

    fn host() -> Origin {
        Origin::Host(crate::origin::HostId::parse("h-3fa9c2d41b7e").unwrap())
    }

    /// RFC 07 v1.2 said tree ids were root hashes and nothing enforced it, so
    /// `tree/nightly` was spellable and silently falsified fleet-wide
    /// cacheability, the storage PUT exemption, and no-op last-writer-wins.
    /// v1.7 made it normative; this asserts it is *structural* — a name has
    /// no spelling in this crate.
    #[test]
    fn a_named_tree_key_is_unspellable() {
        for name in ["nightly", "snap-1", "latest", "v2", "my.snapshot"] {
            assert!(
                blob_key(&host(), BlobTier::Tree, &[name]).is_err(),
                "tree/{name} must not be constructible"
            );
        }
        // A root hash is the only accepted spelling.
        let root = ContentHash::parse(&"ab".repeat(32)).unwrap();
        let key = blob_tree_key(&host(), &root).unwrap();
        assert!(key.as_str().ends_with(&format!("@blob/tree/{root}")));
        // …and the shape is fixed: no extra chunks, no missing ones.
        assert!(blob_key(&host(), BlobTier::Tree, &[root.as_str(), "x"]).is_err());
    }

    #[test]
    fn store_keys_are_algo_then_hash() {
        let hash = ContentHash::parse("ab12cd34ef56").unwrap();
        let key = blob_store_key(&host(), "blake3", &hash).unwrap();
        assert!(key.as_str().ends_with("@blob/store/blake3/ab12cd34ef56"));
        // Wrong arity, or a name where the hash goes, are both refused.
        assert!(blob_key(&host(), BlobTier::Store, &["blake3"]).is_err());
        assert!(blob_key(&host(), BlobTier::Store, &["blake3", "nightly"]).is_err());
        assert!(
            blob_key(
                &host(),
                BlobTier::Store,
                &["blake3", &hash.to_string(), "x"]
            )
            .is_err()
        );
    }

    /// Tier-1 keeps a free-form id (an RPC-minted ULID) plus an endpoint
    /// tail — RFC 07 §2.2 is unchanged by v1.7.
    #[test]
    fn artifact_keys_keep_their_id_and_endpoint_tail() {
        let key = blob_key(&host(), BlobTier::Artifact, &["01hqxk8f9c2n4p", "manifest"]).unwrap();
        assert!(
            key.as_str()
                .ends_with("@blob/artifact/01hqxk8f9c2n4p/manifest")
        );
    }

    #[test]
    fn content_hash_rejects_names_accepts_digests() {
        for good in ["ab12cd34ef56", &"0".repeat(64), &"f".repeat(128)] {
            ContentHash::parse(good).unwrap_or_else(|e| panic!("{good:?}: {e}"));
        }
        for bad in [
            "",               // empty
            "cafe",           // hex but too short to be a digest
            "nightly",        // a name
            "AB12CD34EF56",   // uppercase: one spelling per digest
            "ab12cd34ef5",    // odd length
            "ab12cd34ef5g",   // non-hex digit
            &"a".repeat(130), // implausibly long
        ] {
            assert!(ContentHash::parse(bad).is_err(), "{bad:?} must be refused");
        }
    }

    /// The wire form is the plain digest, and deserialization runs the same
    /// gate as [`ContentHash::parse`] — a name cannot enter through a payload.
    #[cfg(feature = "serde")]
    #[test]
    fn content_hash_serde_round_trips_through_parse() {
        let hash = ContentHash::parse("ab12cd34ef56").unwrap();
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, "\"ab12cd34ef56\"");
        let back: ContentHash = serde_json::from_str(&json).unwrap();
        assert_eq!(back, hash);
        for bad in ["\"nightly\"", "\"AB12CD34EF56\"", "\"ab12cd34ef5\""] {
            assert!(
                serde_json::from_str::<ContentHash>(bad).is_err(),
                "{bad} must be refused on deserialize"
            );
        }
    }

    #[cfg(feature = "schemars")]
    #[test]
    fn content_hash_schema_is_a_patterned_string() {
        let schema = serde_json::to_value(schemars::schema_for!(ContentHash)).unwrap();
        assert_eq!(schema["type"], "string");
        assert_eq!(schema["pattern"], "^(?:[0-9a-f]{2}){4,64}$");
    }
}

fn validate_subject(subject: &[&str]) -> Result<(), KeyError> {
    if subject.is_empty() {
        return Err(KeyError::EmptySubject);
    }
    for chunk in subject {
        if !is_valid_plain_chunk(chunk) {
            return Err(KeyError::InvalidPlainChunk((*chunk).to_string()));
        }
    }
    Ok(())
}

fn push_key(parts: &mut String, chunk: &str) {
    push_key_sep(parts);
    parts.push_str(chunk);
}

fn push_key_sep(parts: &mut String) {
    if !parts.is_empty() {
        parts.push('/');
    }
}

/// Build a data-class key: `v1/<origin>/<class>[/<producer>]/<subject...>`.
///
/// The producer chunk is omitted under service origins (RFC 03 §1.5).
pub fn data_key(
    origin: &Origin,
    class: Class,
    producer: Option<&Producer>,
    subject: &[&str],
) -> Result<Key, KeyError> {
    validate_subject(subject)?;
    if origin.has_producer_chunk() != producer.is_some() {
        return Err(KeyError::Parse(
            "host origins require a producer chunk; service origins forbid one (RFC 03 §1.5)"
                .to_string(),
        ));
    }
    // `alive` is a reserved liveliness-only token under `state` (RFC 03 §3):
    // state keys carrying it come only from the dedicated builders below.
    if class == Class::State && subject.contains(&SUBJECT_ALIVE) {
        return Err(KeyError::ReservedToken(
            SUBJECT_ALIVE.to_string(),
            "data subject chunk",
        ));
    }
    let mut key = String::new();
    push_key(&mut key, VERSION_CHUNK);
    push_key(&mut key, origin.chunk());
    push_key(&mut key, class.chunk());
    if let Some(p) = producer {
        push_key_sep(&mut key);
        p.push_chunk(&mut key);
    }
    for chunk in subject {
        push_key(&mut key, chunk);
    }
    Ok(Key::from_canonical(key))
}

/// Build an `@rpc` procedure key: `v1/<origin>/@rpc[/<producer>]/<procedure...>`.
pub fn rpc_key(
    origin: &Origin,
    producer: Option<&Producer>,
    procedure: &[&str],
) -> Result<Key, KeyError> {
    validate_subject(procedure)?;
    if origin.has_producer_chunk() != producer.is_some() {
        return Err(KeyError::Parse(
            "host origins require a producer chunk; service origins forbid one (RFC 03 §1.5)"
                .to_string(),
        ));
    }
    let mut key = String::new();
    push_key(&mut key, VERSION_CHUNK);
    push_key(&mut key, origin.chunk());
    push_key(&mut key, PLANE_RPC);
    if let Some(p) = producer {
        push_key_sep(&mut key);
        p.push_chunk(&mut key);
    }
    for chunk in procedure {
        push_key(&mut key, chunk);
    }
    Ok(Key::from_canonical(key))
}

/// Build an `@media` key: `v1/<origin>/@media/<producer>/<stream...>`.
pub fn media_key(origin: &Origin, producer: &Producer, stream: &[&str]) -> Result<Key, KeyError> {
    validate_subject(stream)?;
    let mut key = String::new();
    push_key(&mut key, VERSION_CHUNK);
    push_key(&mut key, origin.chunk());
    push_key(&mut key, PLANE_MEDIA);
    push_key_sep(&mut key);
    producer.push_chunk(&mut key);
    for chunk in stream {
        push_key(&mut key, chunk);
    }
    Ok(Key::from_canonical(key))
}

/// Build an `@blob` key: `v1/<origin>/@blob/<tier>/<rest...>` (RFC 07 §2).
///
/// The content-addressed tiers are **shape-checked** here, so a
/// non-conformant key has no spelling in this crate at all — not merely none
/// in the convenience builders:
///
/// - `tree` takes exactly one chunk, a [`ContentHash`] (the root, RFC 07 §2.3);
/// - `store` takes exactly two, `<algo>/<hash>` (RFC 07 §2.4);
/// - `artifact` takes an id and any endpoint tail (RFC 07 §2.2).
///
/// Prefer [`blob_tree_key`] / [`blob_store_key`], which take the hash as a
/// value and cannot be called wrongly.
pub fn blob_key(origin: &Origin, tier: BlobTier, rest: &[&str]) -> Result<Key, KeyError> {
    validate_subject(rest)?;
    match tier {
        BlobTier::Tree => {
            if rest.len() != 1 {
                return Err(KeyError::MalformedBlobKey(
                    "tree",
                    "expected exactly one chunk: the tree's root hash",
                ));
            }
            ContentHash::parse(rest[0])?;
        }
        BlobTier::Store => {
            if rest.len() != 2 {
                return Err(KeyError::MalformedBlobKey(
                    "store",
                    "expected exactly two chunks: <algo>/<hash>",
                ));
            }
            ContentHash::parse(rest[1])?;
        }
        BlobTier::Artifact => {}
    }
    let mut key = String::new();
    push_key(&mut key, VERSION_CHUNK);
    push_key(&mut key, origin.chunk());
    push_key(&mut key, PLANE_BLOB);
    push_key(&mut key, tier.chunk());
    for chunk in rest {
        push_key(&mut key, chunk);
    }
    Ok(Key::from_canonical(key))
}

/// Build a Tier-2 **tree** key: `v1/<origin>/@blob/tree/<root>` (RFC 07 §2.3).
///
/// A snapshot is named by its own root, so the key is immutable and the
/// consumer's request states the identity it demands. Human snapshot names
/// are mutable facts and belong on `state`, pointing at a root.
pub fn blob_tree_key(origin: &Origin, root: &ContentHash) -> Result<Key, KeyError> {
    blob_key(origin, BlobTier::Tree, &[root.as_str()])
}

/// Build a Tier-2 **chunk** key: `v1/<origin>/@blob/store/<algo>/<hash>`
/// (RFC 07 §2.4). `hash` addresses the chunk's *content*; the value carried
/// under the key is a self-describing container.
pub fn blob_store_key(origin: &Origin, algo: &str, hash: &ContentHash) -> Result<Key, KeyError> {
    blob_key(origin, BlobTier::Store, &[algo, hash.as_str()])
}

/// Liveliness token key for a producer: `v1/<origin>/state/<producer>/alive`
/// (RFC 04 §5). Service origins: `v1/@<service>/state/alive`.
pub fn alive_key(origin: &Origin, producer: Option<&Producer>) -> Result<Key, KeyError> {
    if origin.has_producer_chunk() != producer.is_some() {
        return Err(KeyError::Parse(
            "host origins require a producer chunk; service origins forbid one (RFC 03 §1.5)"
                .to_string(),
        ));
    }
    let mut key = String::new();
    push_key(&mut key, VERSION_CHUNK);
    push_key(&mut key, origin.chunk());
    push_key(&mut key, CLASS_STATE);
    if let Some(p) = producer {
        push_key_sep(&mut key);
        p.push_chunk(&mut key);
    }
    push_key(&mut key, SUBJECT_ALIVE);
    Ok(Key::from_canonical(key))
}

/// Liveliness token key for a tracked downstream device (RFC 04 §5):
/// `v1/<origin>/state/<producer>/device/<device>/alive`.
pub fn device_alive_key(
    origin: &Origin,
    producer: &Producer,
    device: &str,
) -> Result<Key, KeyError> {
    if !is_valid_plain_chunk(device) {
        return Err(KeyError::InvalidPlainChunk(device.to_string()));
    }
    let mut key = String::new();
    push_key(&mut key, VERSION_CHUNK);
    push_key(&mut key, origin.chunk());
    push_key(&mut key, CLASS_STATE);
    push_key_sep(&mut key);
    producer.push_chunk(&mut key);
    push_key(&mut key, "device");
    push_key(&mut key, device);
    push_key(&mut key, SUBJECT_ALIVE);
    Ok(Key::from_canonical(key))
}

/// A structurally parsed v1 key (positions 2–5; the subject tail is opaque
/// here — registry-generated parsers refine it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuralKey<'k> {
    pub origin: Origin,
    pub class: ClassOrPlane,
    /// `None` for service origins and for `@blob` (tier token instead).
    pub producer: Option<Producer>,
    /// Tier token, only under `@blob`.
    pub blob_tier: Option<BlobTier>,
    /// Everything after the producer/tier position, borrowed from the parsed
    /// key (v1.5 perf: parsing allocates no per-chunk `String`s — the parse
    /// result lives within the key string's scope, which is how every known
    /// caller uses it).
    pub subject: Vec<&'k str>,
}

impl StructuralKey<'_> {
    /// The typed remote origin, when this key came from a host (RFC 08
    /// §1.1's parse-side bridge): a parsed wire key is exactly where a
    /// consumer legitimately obtains a [`crate::origin::RemoteOrigin`].
    pub fn remote_origin(&self) -> Option<crate::origin::RemoteOrigin> {
        match &self.origin {
            Origin::Host(id) => Some(crate::origin::RemoteOrigin::from_host(id.clone())),
            Origin::Service(_) => None,
        }
    }
}

/// Parse a base-relative v1 key (`v1/...`). Structural only: subject tails
/// stay opaque (RFC 03 §1). Rejects anything that is not under `v1`.
pub fn parse(key: &str) -> Result<StructuralKey<'_>, KeyError> {
    let mut chunks = key.split('/');
    let version = chunks
        .next()
        .ok_or_else(|| KeyError::Parse("empty key".into()))?;
    if version != VERSION_CHUNK {
        return Err(KeyError::Parse(format!(
            "expected {VERSION_CHUNK} first, got {version:?}"
        )));
    }
    let origin_chunk = chunks
        .next()
        .ok_or_else(|| KeyError::Parse("missing origin chunk".into()))?;
    let origin = if is_valid_host_origin(origin_chunk) {
        Origin::Host(crate::origin::HostId::parse(origin_chunk).expect("validated"))
    } else if is_valid_verbatim_chunk(origin_chunk) {
        Origin::Service(origin_chunk.to_string())
    } else {
        return Err(KeyError::InvalidHostOrigin(origin_chunk.to_string()));
    };
    let class_chunk = chunks
        .next()
        .ok_or_else(|| KeyError::Parse("missing class chunk".into()))?;
    let class = if let Some(c) = Class::from_chunk(class_chunk) {
        ClassOrPlane::Class(c)
    } else if let Some(p) = Plane::from_chunk(class_chunk) {
        ClassOrPlane::Plane(p)
    } else {
        return Err(KeyError::Parse(format!(
            "unknown class/plane chunk {class_chunk:?}"
        )));
    };

    let mut producer = None;
    let mut blob_tier = None;
    match (&origin, &class) {
        (_, ClassOrPlane::Plane(Plane::Blob)) => {
            let tier = chunks
                .next()
                .ok_or_else(|| KeyError::Parse("missing blob tier".into()))?;
            blob_tier = Some(
                BlobTier::from_chunk(tier)
                    .ok_or_else(|| KeyError::InvalidBlobTier(tier.to_string()))?,
            );
        }
        (Origin::Host(_), _) => {
            let chunk = chunks
                .next()
                .ok_or_else(|| KeyError::Parse("missing producer chunk".into()))?;
            producer = Some(Producer::parse_chunk(chunk)?);
        }
        (Origin::Service(_), _) => {}
    }

    let subject: Vec<&str> = chunks.collect();
    if subject.is_empty() {
        return Err(KeyError::EmptySubject);
    }
    Ok(StructuralKey {
        origin,
        class,
        producer,
        blob_tier,
        subject,
    })
}

/// Prepend an explicit base — for router-side artifacts (storage selectors,
/// ACL rules) and tests. Application sessions use the namespace instead
/// (RFC 09 §0).
///
/// The empty base is the identity: a wire whose keys start at `v1/` is the
/// base-less bus-root deployment — legal, and the default, since RFC v1.6
/// (RFC 03 §1.1) — and an observer names it with `--base ""` (RFC 09 §5).
pub fn with_base(base: &str, key_or_selector: impl AsRef<str>) -> String {
    if base.is_empty() {
        return key_or_selector.as_ref().to_string();
    }
    format!("{base}/{}", key_or_selector.as_ref())
}

/// Strip an explicit base from a **full** (wire-form) key — the inverse of
/// [`with_base`].
///
/// For un-namespaced observers only: `zenctl`, the `v1_probe` example, and
/// router-side tooling all see the base on the wire because they deliberately
/// do not set a session namespace (RFC 09 §5). A *namespaced* session — every
/// application session — has already had the base stripped for it on ingress,
/// and must call [`parse`] directly.
///
/// Returns `None` if `key` does not sit under `base`, which for an observer is
/// the meaningful answer: the key belongs to another deployment.
///
/// ```
/// use zenkey::grammar::strip_base;
///
/// assert_eq!(
///     strip_base("zensight", "zensight/v1/h-3fa9c2d41b7e/state/sysinfo/health"),
///     Some("v1/h-3fa9c2d41b7e/state/sysinfo/health"),
/// );
/// // A multi-chunk base works the same way (RFC 03 §1.1).
/// assert_eq!(strip_base("acme/fleet-a", "acme/fleet-a/v1/x"), Some("v1/x"));
/// // Another deployment's traffic is not ours to parse.
/// assert_eq!(strip_base("zensight", "other/v1/h-3fa9c2d41b7e/state/sysinfo/health"), None);
/// // Not a prefix *boundary* — `zensightly` is a different base.
/// assert_eq!(strip_base("zensight", "zensightly/v1/x"), None);
/// // The empty base is the identity — the base-less bus-root deployment,
/// // legal since RFC v1.6 (RFC 03 §1.1; named with `--base ""`, RFC 09 §5).
/// assert_eq!(strip_base("", "v1/x"), Some("v1/x"));
/// ```
pub fn strip_base<'k>(base: &str, key: &'k str) -> Option<&'k str> {
    if base.is_empty() {
        return Some(key);
    }
    key.strip_prefix(base)?.strip_prefix('/')
}

/// Structurally parse a **full** key as it appears on the wire, given the
/// deployment base — for un-namespaced observers only (bus explorers,
/// router-side tooling; RFC 09 §5). A *namespaced* session never sees the
/// base and must call [`parse`] directly.
///
/// `None` when the key belongs to another deployment (a different base) or
/// does not parse — for an observer both are the meaningful answer rather
/// than an error.
pub fn parse_full<'k>(base: &str, key: &'k str) -> Option<StructuralKey<'k>> {
    parse(strip_base(base, key)?).ok()
}

// The wire-observer wildcard helpers (`fleet_rpc_key`, `service_rpc_key`,
// `all_liveliness_wildcard`, `service_alive_key`) moved to the typed
// [`crate::selector`] module in v1.5 (issue #7) — selectors are values of
// [`crate::Selector`], not ad-hoc strings.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::HostId;

    fn host() -> Origin {
        Origin::Host(HostId::parse("h-3fa9c2d41b7e").unwrap())
    }

    #[test]
    fn plain_chunk_rules() {
        for ok in [
            "a",
            "cpu",
            "sys_uptime",
            "10-0-0-7",
            "sshd.service",
            "p95_ms",
            "h-3fa9c2d41b7e",
        ] {
            assert!(is_valid_plain_chunk(ok), "{ok}");
        }
        for bad in ["", "-a", "a-", ".a", "A", "Cpu", "a/b", "a*", "@v1", "é"] {
            assert!(!is_valid_plain_chunk(bad), "{bad}");
        }
    }

    #[test]
    fn verbatim_chunk_rules() {
        for ok in ["@v1", "@rpc", "@catalog", "@adv"] {
            assert!(is_valid_verbatim_chunk(ok), "{ok}");
        }
        for bad in ["@", "@-x", "v1", "@V1", "@a/b"] {
            assert!(!is_valid_verbatim_chunk(bad), "{bad}");
        }
    }

    #[test]
    fn producer_instance_split_is_unambiguous() {
        // RFC 03 §1.5: base names must not end in -<int>.
        assert!(Producer::new("snmp").is_ok());
        assert!(Producer::new("net-ring").is_ok());
        assert!(Producer::new("ipv6-2").is_err());
        assert!(Producer::with_instance("snmp", 0).is_err());
        let p = Producer::with_instance("snmp", 2).unwrap();
        assert_eq!(p.chunk(), "snmp-2");
        let back = Producer::parse_chunk("snmp-2").unwrap();
        assert_eq!(back.name(), "snmp");
        assert_eq!(back.instance(), Some(2));
        let bare = Producer::parse_chunk("net-ring").unwrap();
        assert_eq!(bare.name(), "net-ring");
        assert_eq!(bare.instance(), None);
    }

    #[test]
    fn blob_tiers_are_not_producers() {
        assert!(Producer::new("store").is_err());
        assert!(Producer::new("tree").is_err());
        assert!(Producer::new("artifact").is_err());
    }

    #[test]
    fn normative_examples_build_and_roundtrip() {
        // The RFC 03 §5 example set, base-relative.
        let p = |n| Producer::new(n).unwrap();
        let cases = [
            data_key(
                &host(),
                Class::Telemetry,
                Some(&p("sysinfo")),
                &["cpu", "usage"],
            )
            .unwrap(),
            data_key(
                &host(),
                Class::Telemetry,
                Some(&p("snmp")),
                &["router01", "system", "sys_uptime"],
            )
            .unwrap(),
            data_key(&host(), Class::State, Some(&p("netring")), &["health"]).unwrap(),
            data_key(
                &host(),
                Class::State,
                Some(&p("netlink")),
                &["alert", "9f2c81ab04d7e3f1"],
            )
            .unwrap(),
            data_key(
                &host(),
                Class::State,
                Some(&p("netring")),
                &["evidence", "names", "10-0-0-7"],
            )
            .unwrap(),
            data_key(
                &host(),
                Class::Events,
                Some(&p("netring")),
                &["capture", "01jgxqz4yqk8v6txw3m9f2a7cd"],
            )
            .unwrap(),
            rpc_key(&host(), Some(&p("netlink")), &["sockets"]).unwrap(),
            media_key(&host(), &p("parallax"), &["cam0", "video", "h264", "high"]).unwrap(),
            blob_key(&host(), BlobTier::Store, &["sha256", "ab12cd34ef56"]).unwrap(),
            data_key(
                &Origin::catalog(),
                Class::State,
                None,
                &["entity", "h-3fa9c2d41b7e"],
            )
            .unwrap(),
            data_key(
                &Origin::catalog(),
                Class::State,
                None,
                &["pdns", "93-184-216-34"],
            )
            .unwrap(),
        ];
        let expected = [
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage",
            "v1/h-3fa9c2d41b7e/telemetry/snmp/router01/system/sys_uptime",
            "v1/h-3fa9c2d41b7e/state/netring/health",
            "v1/h-3fa9c2d41b7e/state/netlink/alert/9f2c81ab04d7e3f1",
            "v1/h-3fa9c2d41b7e/state/netring/evidence/names/10-0-0-7",
            "v1/h-3fa9c2d41b7e/events/netring/capture/01jgxqz4yqk8v6txw3m9f2a7cd",
            "v1/h-3fa9c2d41b7e/@rpc/netlink/sockets",
            "v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/high",
            "v1/h-3fa9c2d41b7e/@blob/store/sha256/ab12cd34ef56",
            "v1/@catalog/state/entity/h-3fa9c2d41b7e",
            "v1/@catalog/state/pdns/93-184-216-34",
        ];
        for (built, want) in cases.iter().zip(expected) {
            assert_eq!(built, want);
            let parsed = parse(built).unwrap();
            // Rebuild from parts must reproduce the key (canon round-trip).
            let subject = &parsed.subject;
            let rebuilt = match parsed.class {
                ClassOrPlane::Class(c) => {
                    data_key(&parsed.origin, c, parsed.producer.as_ref(), subject).unwrap()
                }
                ClassOrPlane::Plane(Plane::Rpc) => {
                    rpc_key(&parsed.origin, parsed.producer.as_ref(), subject).unwrap()
                }
                ClassOrPlane::Plane(Plane::Media) => {
                    media_key(&parsed.origin, parsed.producer.as_ref().unwrap(), subject).unwrap()
                }
                ClassOrPlane::Plane(Plane::Blob) => {
                    blob_key(&parsed.origin, parsed.blob_tier.unwrap(), subject).unwrap()
                }
            };
            assert_eq!(&rebuilt, want);
        }
    }

    #[test]
    fn alive_is_liveliness_only() {
        assert!(
            data_key(
                &host(),
                Class::State,
                Some(&Producer::new("netlink").unwrap()),
                &["alive"]
            )
            .is_err()
        );
        assert_eq!(
            alive_key(&host(), Some(&Producer::new("netlink").unwrap())).unwrap(),
            "v1/h-3fa9c2d41b7e/state/netlink/alive"
        );
        assert_eq!(
            alive_key(&Origin::catalog(), None).unwrap(),
            "v1/@catalog/state/alive"
        );
        assert_eq!(
            device_alive_key(&host(), &Producer::new("snmp").unwrap(), "router01").unwrap(),
            "v1/h-3fa9c2d41b7e/state/snmp/device/router01/alive"
        );
    }

    #[test]
    fn service_origin_omits_producer() {
        assert!(
            data_key(
                &Origin::catalog(),
                Class::State,
                Some(&Producer::new("x").unwrap()),
                &["entity", "a"]
            )
            .is_err()
        );
        assert!(data_key(&host(), Class::State, None, &["health"]).is_err());
    }

    #[test]
    fn parse_rejects_foreign_keys() {
        assert!(parse("zensight/netlink/host/@/health").is_err());
        assert!(parse("@v2/h-3fa9c2d41b7e/state/x/health").is_err());
        assert!(parse("v1/h-3fa9c2d41b7e/bogus/x/health").is_err());
        assert!(parse("v1/h-3fa9c2d41b7e/@blob/bogus/x").is_err());
    }

    #[test]
    fn empty_base_is_the_identity_for_observers() {
        // A wire whose keys start at `v1/` is the base-less bus-root
        // deployment — legal, and the default, since RFC v1.6 (RFC 03 §1.1);
        // observers name it with `--base ""` (RFC 09 §5).
        let key = "v1/h-3fa9c2d41b7e/state/sysinfo/alive";
        assert_eq!(with_base("", key), key);
        assert_eq!(with_base("", "v1/*/**"), "v1/*/**");
        assert_eq!(strip_base("", key), Some(key));
        let parsed = parse_full("", key).unwrap();
        assert_eq!(parsed.origin.chunk(), "h-3fa9c2d41b7e");
        assert_eq!(parsed.subject, vec!["alive"]);
        // Non-empty bases keep their boundary semantics.
        assert_eq!(with_base("zs", key), format!("zs/{key}"));
        assert_eq!(strip_base("zs", &format!("zs/{key}")), Some(key));
    }
}
