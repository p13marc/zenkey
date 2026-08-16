//! What an explorer can honestly say about one observed key (RFC 09 §5.1).
//!
//! Born as zengui's `keyfacts` module and moved into the engine (issue #34)
//! when RFC v1.9 made the classification ladder normative for *every* observer
//! — shared policy belongs in the shared crate. The explorers' cores stay
//! key-agnostic ([`crate::KeyTreeSnapshot`] groups on a plain `split('/')`);
//! this is the enrichment layer on top: it projects a wire key onto the
//! keyspace-v2 grammar when it can, and degrades to a stated reason when it
//! cannot (O2). Nothing here ever rejects a key (O1).
//!
//! Two properties are load-bearing and are pinned by the tests below:
//!
//! - **Owned.** [`zenkey::grammar::StructuralKey`] borrows from the key string,
//!   so it cannot live in widget state. [`KeyFacts`] is the owned projection,
//!   computed *once* when a key is first observed — never per render.
//! - **Base-relative, never by absolute index** (RFC 03 §1.1). Positions are
//!   resolved after [`strip_base`](zenkey::grammar::strip_base); a multi-chunk
//!   base (`acme/fleet-a`) and the empty base must give identical facts for the
//!   same subject.

use crate::registry::SliceSet;
use zenkey::grammar::{self, BlobTier, Class, ClassOrPlane, Origin, Plane, StructuralKey};

/// Everything zengui knows about one wire key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFacts {
    pub shape: KeyShape,
    pub registration: Registration,
}

/// How far the key got through the grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyShape {
    /// Parses as `v1/<origin>/<class>/<producer>/<subject…>` under the active base.
    V1(Box<V1Facts>),
    /// The key does not sit under the active base. A *fact*, not a guess —
    /// and unreachable when the base is empty, since `strip_base("", k)` is
    /// the identity (RFC 03 §1.1).
    ///
    /// Deliberately does **not** try to name the key's own base: with no fixed
    /// arity for a subject tail, guessing would mean a left-to-right "first
    /// `v1`" scan, which RFC 09 §5 forbids for base attribution. Naming other
    /// bases is the base picker's job (`discover_bases`), which attributes
    /// fixed-arity from the right.
    NotUnderBase,
    /// Under the base, but not a v1 key — an ordinary plain Zenoh key. The
    /// grammar's own message is kept verbatim; it already cites the RFC section.
    Unparsed { reason: String },
}

/// Positions 3–6 of a conforming key, owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V1Facts {
    /// The origin chunk, verbatim.
    pub origin: String,
    pub origin_kind: OriginKind,
    /// The class/plane chunk, verbatim.
    pub class: String,
    pub class_kind: ClassKind,
    /// Producer base name. `None` under a service origin and under `@blob`,
    /// where position 5 is a tier token instead (RFC 03 §1.5).
    pub producer: Option<String>,
    pub instance: Option<u32>,
    /// Tier token, only under `@blob`.
    pub blob_tier: Option<String>,
    /// Everything after the producer/tier position.
    pub subject: Vec<String>,
}

/// RFC 03 §1.3 licenses tooling to rely on the `h-[0-9a-f]{12}` shape to tell
/// these apart — and RFC 03 §1.5 makes it the *sole* discriminator for whether
/// position 5 is a producer or already subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginKind {
    Host,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassKind {
    Telemetry,
    State,
    Events,
    Rpc,
    Media,
    Blob,
}

/// Whether the registry recognises this subject.
///
/// RFC 08 §6.4 asks for a "registered-vs-wild flag", but a `bool` cannot be
/// honest: it renders "we have not loaded a registry yet" identically to "this
/// subject is not registered". That is the false-verdict failure of RFC 05
/// §3.1 / RFC 12 §9 applied to a badge — *silence is never a verdict*, and
/// neither is a not-yet-asked question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// No slice set loaded yet. We have not asked. Render as "—", never "wild".
    Unknown,
    /// Slices are loaded, but none declares this producer.
    NoSliceForProducer,
    /// The producer's slice is loaded and does not declare this subject.
    /// "A subject that is not registered does not exist" (RFC 08) — for a
    /// *conforming producer*. On the wire it is simply unregistered traffic.
    Unregistered,
    Registered(Box<SubjectFacts>),
    /// The key has no registry surface to check: not under the base, unparsed,
    /// or on a verbatim plane (the slice carries subjects, not plane keys).
    NotApplicable,
}

/// The registry's description of a matched subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectFacts {
    /// The declared pattern, e.g. `disk/{mount}/used`.
    pub path: String,
    pub type_name: String,
    /// Variable bindings from the match, e.g. `[("mount", "var-log")]`.
    pub vars: Vec<(String, String)>,
    pub unit: Option<String>,
    pub qos: Option<String>,
    pub encoding: Option<String>,
    pub ttl_s: Option<i64>,
    /// The declared events rate class (`rare` | `low` | `burst(n/h)`,
    /// RFC 04 §1.3) — carried so observers can judge over-rate (#161).
    pub rate: Option<String>,
}

impl SubjectFacts {
    /// The declared QoS profile, parsed into the closed RFC 04 §3 vocabulary.
    ///
    /// `None` means the registry declares no profile for this subject — or
    /// names one outside the vocabulary, which the RFC 08 §5 lints reject at
    /// the producer's build; a live slice can still carry anything, and an
    /// unparseable name must not be mistaken for a parsed one (#158).
    pub fn declared_qos(&self) -> Option<zenkey::qos::QosProfile> {
        self.qos
            .as_deref()
            .and_then(zenkey::qos::QosProfile::from_name)
    }
}

impl KeyFacts {
    /// Project a full wire key against the active base. Infallible by design.
    ///
    /// Registration starts [`Registration::Unknown`] for a conforming data key;
    /// call [`KeyFacts::resolve`] once a [`SliceSet`] is available. The two
    /// steps are separate because they are invalidated by different things —
    /// the base changes the shape, the slice set changes only the registration.
    pub fn project(base: &str, wire_key: &str) -> KeyFacts {
        let Some(relative) = grammar::strip_base(base, wire_key) else {
            return KeyFacts {
                shape: KeyShape::NotUnderBase,
                registration: Registration::NotApplicable,
            };
        };
        match grammar::parse(relative) {
            Ok(parsed) => {
                let facts = V1Facts::from_parsed(&parsed);
                let registration = if facts.class_kind.is_data_class() {
                    Registration::Unknown
                } else {
                    // A verbatim plane has no `[[subject]]` surface to match.
                    Registration::NotApplicable
                };
                KeyFacts {
                    shape: KeyShape::V1(Box::new(facts)),
                    registration,
                }
            }
            Err(e) => KeyFacts {
                shape: KeyShape::Unparsed {
                    reason: e.to_string(),
                },
                registration: Registration::NotApplicable,
            },
        }
    }

    /// Resolve the registration against a loaded slice set.
    ///
    /// Uses [`SliceSet::refine`], which applies RFC 08 §2's most-literal-first
    /// precedence (literal beats `{var}` beats `{var...}`). Note `zenctl`'s
    /// `offline::topic_info` predates `refine` and matches in *declaration*
    /// order instead — do not copy it.
    pub fn resolve(&mut self, slices: &SliceSet) {
        let KeyShape::V1(facts) = &self.shape else {
            return;
        };
        if !facts.class_kind.is_data_class() {
            return;
        }
        // A service origin omits the producer chunk (RFC 03 §1.5), so its slice
        // is found by the origin it serves, not by a producer name.
        let producer = match facts.origin_kind {
            OriginKind::Host => facts.producer.clone(),
            OriginKind::Service => slices
                .by_service_origin(&facts.origin)
                .map(|s| s.name.clone()),
        };
        let Some(producer) = producer else {
            self.registration = Registration::NoSliceForProducer;
            return;
        };
        if slices.get(&producer).is_none() {
            self.registration = Registration::NoSliceForProducer;
            return;
        }
        let tail: Vec<&str> = facts.subject.iter().map(String::as_str).collect();
        self.registration = match slices.refine(&producer, &facts.class, &tail) {
            Some((decl, vars)) => Registration::Registered(Box::new(SubjectFacts {
                path: decl.path.clone(),
                type_name: decl.type_name.clone(),
                vars,
                unit: decl.unit.clone(),
                qos: decl.qos.clone(),
                encoding: decl.encoding.clone(),
                ttl_s: decl.ttl_s,
                rate: decl.rate.clone(),
            })),
            None => Registration::Unregistered,
        };
    }

    /// The declared payload type, when the registry named one. Drives the echo
    /// pane's type tag and, later, the schema lookup of RFC 08 §7.
    pub fn type_name(&self) -> Option<&str> {
        match &self.registration {
            Registration::Registered(s) => Some(&s.type_name),
            _ => None,
        }
    }
}

/// Fraction of the cache dropped when the bound is hit — the amortisation
/// argument is [`crate::stats`]'s, verbatim: evicting one entry per insert
/// would make every projection past the bound a full scan.
const EVICT_FRACTION: usize = 16;

struct Entry {
    facts: KeyFacts,
    /// Monotone observation counter, not an `Instant`: recency here means
    /// last-*observed*, the ordering is all that is read, and a counter is
    /// deterministic in tests and one word per entry.
    seen: u64,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry").field("seen", &self.seen).finish()
    }
}

/// A bounded cache of key projections, sized off the same `max_keys` as the
/// [`StatsTable`](crate::stats::StatsTable) it shadows, counting what the bound
/// costs (RFC 09 §5.1 O6).
///
/// **Why this exists** (issue #107). Projecting a key is not free — a
/// [`KeyFacts`] owns a `String` per subject chunk plus the resolved
/// [`SubjectFacts`] — so every observer caches it, and zengui's cache was a
/// plain `HashMap` that grew one entry per distinct key *ever seen*. The engine's
/// key table is bounded and counts its evictions; the projection cache shadowing
/// it was, in `stats.rs`'s own words, "merely a leak with better manners".
///
/// **Why an LRU and not "prune to the stats table"**, which is the obvious fix:
///
/// - the table is fed from samples, and an observer also projects **liveliness
///   token keys**, which never enter it. Pruning to the table would delete and
///   re-project those on every tick, and drop them from any "keys seen" list;
/// - a frontend holds a key-*tree* snapshot, not a key list, so membership means
///   walking the tree per tick — O(n) allocation on the render thread at up to
///   50k keys, where this is O(1) amortised on the insert path;
/// - "evicted because the table evicted it" and "evicted because it was never in
///   the table" are different facts, and one counter over both is exactly what
///   O6 forbids.
///
/// Recency is last-**observed**, not last-rendered, which is what keeps
/// [`get`](Self::get) a pure read: a `&self` render path can look keys up
/// without touching the ordering, so no interior mutability and no signature
/// churn in the views.
#[derive(Debug)]
pub struct FactsCache {
    entries: std::collections::HashMap<String, Entry>,
    max_keys: usize,
    inserted: u64,
    evicted: u64,
    seq: u64,
}

impl Default for FactsCache {
    fn default() -> Self {
        FactsCache::with_capacity(crate::stats::DEFAULT_MAX_KEYS)
    }
}

impl FactsCache {
    /// A cache bounded at `max_keys` projections. Pass the same bound the
    /// stats table was built with: the cache cannot usefully outgrow the table
    /// it shadows, and one number makes that one sentence.
    pub fn with_capacity(max_keys: usize) -> FactsCache {
        FactsCache {
            entries: std::collections::HashMap::new(),
            max_keys: max_keys.max(1),
            inserted: 0,
            evicted: 0,
            seq: 0,
        }
    }

    /// Project `key` if it is not cached yet; bump its recency either way.
    ///
    /// The single insert point — the whole bound rests on that being true.
    pub fn ensure(&mut self, base: &str, key: &str, slices: Option<&SliceSet>) {
        self.seq += 1;
        let seq = self.seq;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.seen = seq;
            return;
        }
        if self.entries.len() >= self.max_keys {
            self.evict();
        }
        let mut facts = KeyFacts::project(base, key);
        if let Some(slices) = slices {
            facts.resolve(slices);
        }
        self.entries
            .insert(key.to_string(), Entry { facts, seen: seq });
        self.inserted += 1;
    }

    /// A cached projection, if it is still held. Pure: recency is not touched,
    /// so this is safe to call from a `&self` render path.
    pub fn get(&self, key: &str) -> Option<&KeyFacts> {
        self.entries.get(key).map(|e| &e.facts)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn max_keys(&self) -> usize {
        self.max_keys
    }

    /// Projections retired to stay within the bound.
    ///
    /// Displayed, never hidden: a cache that stopped growing and a bus that
    /// went quiet look identical from the outside (RFC 09 §5.1 O6).
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    /// Projections *made* since the last [`clear`](Self::clear).
    ///
    /// The other half of the O6 ledger, and the reason it is a public number
    /// rather than an internal one: `inserted == len() + evicted()` is the
    /// conservation law, and without this counter it cannot be checked from
    /// outside. Note it counts insertions, not distinct keys — a key evicted
    /// and later re-observed is projected again, which is precisely the cost
    /// the bound is trading against.
    pub fn inserted(&self) -> u64 {
        self.inserted
    }

    /// Re-resolve every held projection against a newly-loaded slice set —
    /// what a registry arriving after the first samples calls for.
    pub fn resolve_all(&mut self, slices: &SliceSet) {
        for entry in self.entries.values_mut() {
            entry.facts.resolve(slices);
        }
    }

    /// Base change / reconnect / context switch. Keeps the bound and resets
    /// the counter: retirements under another deployment are not this one's.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.inserted = 0;
        self.evicted = 0;
        self.seq = 0;
    }

    /// Drop the least-recently-observed entries until there is room.
    fn evict(&mut self) {
        let target = self.max_keys - (self.max_keys / EVICT_FRACTION).max(1);
        let mut seen: Vec<(u64, String)> = self
            .entries
            .iter()
            .map(|(k, e)| (e.seen, k.clone()))
            .collect();
        seen.sort_unstable_by_key(|(seen, _)| *seen);
        for (_, key) in seen.into_iter().take(self.entries.len() - target) {
            self.entries.remove(&key);
            self.evicted += 1;
        }
    }
}

impl V1Facts {
    fn from_parsed(parsed: &StructuralKey<'_>) -> V1Facts {
        let (origin, origin_kind) = match &parsed.origin {
            Origin::Host(id) => (id.as_str().to_string(), OriginKind::Host),
            Origin::Service(s) => (s.clone(), OriginKind::Service),
        };
        let (class, class_kind) = match parsed.class {
            ClassOrPlane::Class(c) => (c.chunk().to_string(), ClassKind::from_class(c)),
            ClassOrPlane::Plane(p) => (p.chunk().to_string(), ClassKind::from_plane(p)),
        };
        V1Facts {
            origin,
            origin_kind,
            class,
            class_kind,
            producer: parsed.producer.as_ref().map(|p| p.name().to_string()),
            instance: parsed.producer.as_ref().and_then(|p| p.instance()),
            blob_tier: parsed.blob_tier.map(|t| tier_chunk(t).to_string()),
            subject: parsed.subject.iter().map(|s| (*s).to_string()).collect(),
        }
    }
}

fn tier_chunk(tier: BlobTier) -> &'static str {
    tier.chunk()
}

impl ClassKind {
    fn from_class(c: Class) -> ClassKind {
        match c {
            Class::Telemetry => ClassKind::Telemetry,
            Class::State => ClassKind::State,
            Class::Events => ClassKind::Events,
        }
    }

    fn from_plane(p: Plane) -> ClassKind {
        match p {
            Plane::Rpc => ClassKind::Rpc,
            Plane::Media => ClassKind::Media,
            Plane::Blob => ClassKind::Blob,
        }
    }

    /// The three data classes carry `[[subject]]` entries; the verbatim planes
    /// do not (RFC 03 §1.4).
    pub fn is_data_class(self) -> bool {
        matches!(
            self,
            ClassKind::Telemetry | ClassKind::State | ClassKind::Events
        )
    }
}

/// A key, fully described as far as the ladder reaches — the engine-side
/// replacement for zenctl's old `offline::topic_info`, which hard-errored on
/// non-v1 keys (an O1 violation) and matched subjects in declaration order
/// (diverging from [`SliceSet::refine`]'s most-literal-first precedence).
///
/// Infallible by design: every key gets a description; the description says
/// how far it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyDescription {
    /// The key as given (full wire form).
    pub key: String,
    pub facts: KeyFacts,
}

/// Project and resolve in one call.
///
/// `slices` is an `Option` on purpose: `None` means *no registry was loaded*,
/// which must stay distinguishable from `Some(empty)` — a registry that was
/// loaded and covers nothing. "Not asked" is not "answered no" (O4).
pub fn describe_key(base: &str, key: &str, slices: Option<&SliceSet>) -> KeyDescription {
    let mut facts = KeyFacts::project(base, key);
    if let Some(slices) = slices {
        facts.resolve(slices);
    }
    KeyDescription {
        key: key.to_string(),
        facts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1(facts: &KeyFacts) -> &V1Facts {
        match &facts.shape {
            KeyShape::V1(f) => f,
            other => panic!("expected a v1 key, got {other:?}"),
        }
    }

    #[test]
    fn projects_a_host_telemetry_key() {
        let f = KeyFacts::project(
            "zensight",
            "zensight/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage",
        );
        let v = v1(&f);
        assert_eq!(v.origin, "h-3fa9c2d41b7e");
        assert_eq!(v.origin_kind, OriginKind::Host);
        assert_eq!(v.class, "telemetry");
        assert_eq!(v.producer.as_deref(), Some("sysinfo"));
        assert_eq!(v.instance, None);
        assert_eq!(v.subject, ["cpu", "usage"]);
        // No slice set has been consulted yet — that is not "unregistered".
        assert_eq!(f.registration, Registration::Unknown);
    }

    /// RFC 03 §1.1: positions are resolved *relative to the configured base*,
    /// never by absolute index. The empty base, a one-chunk base and a
    /// multi-chunk base must all yield identical facts for the same subject.
    #[test]
    fn positions_are_base_relative_never_absolute() {
        let subject = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage";
        let cases = [
            ("", subject.to_string()),
            ("zensight", format!("zensight/{subject}")),
            ("acme/fleet-a", format!("acme/fleet-a/{subject}")),
        ];
        let projected: Vec<V1Facts> = cases
            .iter()
            .map(|(base, key)| v1(&KeyFacts::project(base, key)).clone())
            .collect();
        assert_eq!(projected[0], projected[1]);
        assert_eq!(projected[1], projected[2]);
        assert_eq!(projected[0].producer.as_deref(), Some("sysinfo"));
    }

    /// RFC 03 §1.5: chunk 5 is producer-or-subject, disambiguated by the origin
    /// chunk *alone*. A service origin omits the producer position entirely.
    #[test]
    fn origin_chunk_alone_decides_whether_chunk_five_is_a_producer() {
        let host = KeyFacts::project("", "v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(v1(&host).producer.as_deref(), Some("sysinfo"));
        assert_eq!(v1(&host).subject, ["health"]);

        let service = KeyFacts::project("", "v1/@catalog/state/entity/x");
        assert_eq!(v1(&service).origin_kind, OriginKind::Service);
        assert_eq!(v1(&service).origin, "@catalog");
        assert_eq!(v1(&service).producer, None);
        // `entity` is already subject here, not a producer.
        assert_eq!(v1(&service).subject, ["entity", "x"]);
    }

    #[test]
    fn parses_a_producer_instance_suffix() {
        let f = KeyFacts::project("", "v1/h-3fa9c2d41b7e/telemetry/snmp-2/if/eth0/in");
        assert_eq!(v1(&f).producer.as_deref(), Some("snmp"));
        assert_eq!(v1(&f).instance, Some(2));
    }

    /// Under `@blob` position 5 is a tier token, not a producer (RFC 03 §1.5).
    #[test]
    fn blob_tier_occupies_the_producer_position() {
        let f = KeyFacts::project("", "v1/h-3fa9c2d41b7e/@blob/store/sha256/abcdef01");
        let v = v1(&f);
        assert_eq!(v.class_kind, ClassKind::Blob);
        assert_eq!(v.producer, None);
        assert_eq!(v.blob_tier.as_deref(), Some("store"));
        // A verbatim plane has no `[[subject]]` surface — not "unregistered".
        assert_eq!(f.registration, Registration::NotApplicable);
    }

    #[test]
    fn a_key_under_another_base_is_a_fact_not_an_error() {
        let f = KeyFacts::project("zensight", "other/v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(f.shape, KeyShape::NotUnderBase);
        // We deliberately do not name `other` — that would need the
        // "first v1" scan RFC 09 §5 forbids.
    }

    /// `strip_base("", k)` is the identity, so with the (default) empty base
    /// every key is under the base and `NotUnderBase` is unreachable.
    #[test]
    fn empty_base_makes_not_under_base_unreachable() {
        for key in [
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "zensight/v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "demo/example/foo",
            "",
        ] {
            assert_ne!(
                KeyFacts::project("", key).shape,
                KeyShape::NotUnderBase,
                "{key}"
            );
        }
    }

    /// The whole point of the key-agnostic core: a plain Zenoh key is not an
    /// error, it is a key we can still count, group and render.
    #[test]
    fn arbitrary_keys_degrade_to_a_stated_reason() {
        for key in ["demo/example/foo", "v2/h-3fa9c2d41b7e/state/x/y", "a", ""] {
            let f = KeyFacts::project("", key);
            match f.shape {
                KeyShape::Unparsed { reason } => assert!(!reason.is_empty(), "{key}"),
                other => panic!("{key} should be unparsed, got {other:?}"),
            }
            assert_eq!(f.registration, Registration::NotApplicable);
        }
    }

    /// An `@`-chunk in an otherwise foreign key must not panic or be mistaken
    /// for a plane — this is the shape a hostile/foreign publisher produces.
    #[test]
    fn foreign_keys_with_verbatim_chunks_are_merely_unparsed() {
        let f = KeyFacts::project("", "demo/@thing/foo");
        assert!(matches!(f.shape, KeyShape::Unparsed { .. }));
    }

    #[test]
    fn unknown_registration_is_not_unregistered() {
        // The distinction the tri-state exists for.
        assert_ne!(Registration::Unknown, Registration::Unregistered);
    }

    /// `describe_key` must use refine's most-literal-first precedence: a
    /// literal leaf beats a `{var}` even when the var is declared first.
    /// (The old zenctl `topic_info` matched in declaration order — the exact
    /// divergence issue #34 exists to kill.)
    #[test]
    fn describe_key_prefers_the_literal_over_the_variable() {
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let subject = |path: &str| SubjectDecl {
            path: path.to_string(),
            class: "telemetry".to_string(),
            type_name: if path.contains('{') {
                "VarPoint"
            } else {
                "SpecialPoint"
            }
            .to_string(),
            common: None,
            since: None,
            description: None,
            qos: None,
            ttl_s: None,
            unit: None,
            rate: None,
            cardinality: None,
            encoding: None,
        };
        let slice = RegistrySlice {
            version: "1.0".into(),
            app: "test".into(),
            convention: 1,
            name: "flowd".into(),
            service_origin: None,
            description: None,
            // The {var} pattern is declared FIRST — declaration order must not win.
            subjects: vec![subject("flow/{q}"), subject("flow/special")],
            procedures: vec![],
            blob: vec![],
            media: vec![],
            deprecated: vec![],
        };
        let slices = SliceSet::from_slices(vec![slice]);
        let d = describe_key(
            "",
            "v1/h-3fa9c2d41b7e/telemetry/flowd/flow/special",
            Some(&slices),
        );
        match &d.facts.registration {
            Registration::Registered(s) => {
                assert_eq!(s.path, "flow/special", "literal must beat {{var}}");
                assert_eq!(s.type_name, "SpecialPoint");
            }
            other => panic!("expected Registered, got {other:?}"),
        }
        // …and the variable pattern still catches everything else.
        let d = describe_key(
            "",
            "v1/h-3fa9c2d41b7e/telemetry/flowd/flow/p95",
            Some(&slices),
        );
        match &d.facts.registration {
            Registration::Registered(s) => assert_eq!(s.path, "flow/{q}"),
            other => panic!("expected Registered, got {other:?}"),
        }
    }

    /// #158: the declared profile parses into the closed vocabulary, and an
    /// out-of-vocabulary name degrades to `None` instead of a wrong profile.
    #[test]
    fn declared_qos_parses_the_closed_vocabulary_only() {
        let facts = |qos: Option<&str>| SubjectFacts {
            path: "cpu/usage".into(),
            type_name: "Point".into(),
            vars: vec![],
            unit: None,
            qos: qos.map(str::to_string),
            encoding: None,
            ttl_s: None,
            rate: None,
        };
        assert_eq!(
            facts(Some("transition")).declared_qos(),
            Some(zenkey::qos::QosProfile::Transition)
        );
        assert_eq!(facts(None).declared_qos(), None);
        assert_eq!(facts(Some("best-effort-ish")).declared_qos(), None);
    }

    /// O1: a key that does not parse still gets a full description.
    #[test]
    fn describe_key_never_fails() {
        for key in ["demo/example/foo", "", "v2/x", "@weird/key"] {
            let d = describe_key("", key, None);
            assert_eq!(d.key, key);
            assert!(matches!(d.facts.shape, KeyShape::Unparsed { .. }), "{key}");
        }
        let d = describe_key("zensight", "other/v1/h-3fa9c2d41b7e/state/x/y", None);
        assert_eq!(d.facts.shape, KeyShape::NotUnderBase);
    }
}

// ── FactsCache (#107) ───────────────────────────────────────────────────

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn key(i: usize) -> String {
        format!("v1/h-3fa9c2d41b7e/telemetry/sysinfo/k{i}")
    }

    #[test]
    fn the_bound_holds_and_every_drop_is_counted() {
        let mut cache = FactsCache::with_capacity(100);
        for i in 0..1_000 {
            cache.ensure("", &key(i), None);
        }
        assert!(cache.len() <= 100, "held {}", cache.len());
        assert!(cache.evicted() > 0, "the fixture must trip the bound");
        // The ledger #107 asks for: nothing vanishes unaccounted.
        assert_eq!(cache.inserted(), 1_000, "every key here was distinct");
        assert_eq!(cache.len() as u64 + cache.evicted(), cache.inserted());
    }

    /// A key evicted and later re-observed is projected *again* — the cost the
    /// bound trades against, and the reason the ledger counts insertions rather
    /// than distinct keys.
    #[test]
    fn a_re_observed_eviction_is_projected_again() {
        let mut cache = FactsCache::with_capacity(2);
        for i in 0..10 {
            cache.ensure("", &key(i), None);
        }
        let after_first_pass = cache.inserted();
        for i in 0..10 {
            cache.ensure("", &key(i), None);
        }
        assert!(
            cache.inserted() > after_first_pass,
            "a second pass over evicted keys re-projects them"
        );
        assert_eq!(cache.len() as u64 + cache.evicted(), cache.inserted());
    }

    #[test]
    fn the_least_recently_observed_is_the_one_that_goes() {
        let mut cache = FactsCache::with_capacity(4);
        for i in 0..4 {
            cache.ensure("", &key(i), None);
        }
        // Re-observing k0 makes k1 the oldest, so the next eviction takes k1
        // and spares k0 — recency is last-*observed*, and this is what says so.
        cache.ensure("", &key(0), None);
        cache.ensure("", &key(99), None);
        assert!(cache.get(&key(0)).is_some(), "the re-observed key survives");
        assert!(cache.get(&key(1)).is_none(), "the oldest went instead");
    }

    #[test]
    fn ensure_is_idempotent_and_does_not_reproject() {
        let slices = SliceSet::default();
        let mut cache = FactsCache::with_capacity(10);
        cache.ensure("", &key(0), None);
        let before = cache.get(&key(0)).cloned();
        cache.ensure("", &key(0), Some(&slices));
        assert_eq!(
            cache.get(&key(0)).cloned(),
            before,
            "a second ensure must not re-resolve behind the caller's back"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn resolve_all_reaches_entries_projected_before_the_registry_arrived() {
        // The ordinary startup order: samples first, slices second.
        let mut cache = FactsCache::with_capacity(10);
        cache.ensure("", &key(0), None);
        assert_eq!(
            cache.get(&key(0)).map(|f| f.registration.clone()),
            Some(Registration::Unknown)
        );
        cache.resolve_all(&SliceSet::default());
        assert_ne!(
            cache.get(&key(0)).map(|f| f.registration.clone()),
            Some(Registration::Unknown),
            "a registry that arrives late still reaches what was already cached"
        );
    }

    #[test]
    fn clearing_keeps_the_bound_and_forgets_the_count() {
        let mut cache = FactsCache::with_capacity(4);
        for i in 0..40 {
            cache.ensure("", &key(i), None);
        }
        assert!(cache.evicted() > 0);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.max_keys(), 4, "the bound is a setting, not a state");
        assert_eq!(
            cache.evicted(),
            0,
            "retirements under another deployment are not this one's"
        );
        assert_eq!(cache.inserted(), 0);
    }

    #[test]
    fn a_degenerate_bound_is_still_a_bound() {
        let mut cache = FactsCache::with_capacity(0);
        for i in 0..10 {
            cache.ensure("", &key(i), None);
        }
        assert_eq!(cache.max_keys(), 1);
        assert!(cache.len() <= 1);
    }
}
