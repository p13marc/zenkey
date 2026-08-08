//! The RFC-overlay seam: what zengui can honestly say about one observed key.
//!
//! zengui's core is key-agnostic — [`zenkey_fleet::KeyTreeSnapshot`] groups on a
//! plain `split('/')` and never consults the grammar, so an arbitrary Zenoh bus
//! renders fine. This module is the *enrichment* layer on top: it projects a
//! wire key onto the keyspace-v2 grammar when it can, and degrades to a stated
//! reason when it cannot. Nothing here ever rejects a key.
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

use zenkey::grammar::{self, BlobTier, Class, ClassOrPlane, Origin, Plane, StructuralKey};
use zenkey_fleet::SliceSet;

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
}
