//! The one seam where a key becomes convention-aware: zk2's identity ladder
//! (#614, FL7).
//!
//! This module *was* v1's classification ladder, re-exported from the v1
//! engine — `KeyFacts`, `KeyShape`, the five-state `Registration`. zk2's
//! answer is the lens's: [`Lens::resolve`](zenkey_fleet::Lens::resolve)
//! walks the tooling guide's O2 rungs (in this namespace, a zk2 data key, a
//! provider in presence, a revision named, the contract held, a resource
//! and its member) and says where it stopped, and an [`IdentityCache`]
//! keeps each key's [`Resolution`], bounded and generation-tagged. What
//! this module adds is the GUI's reading of that answer — which rung a
//! badge names ([`Rung`]), and whether a key earns one at all
//! ([`badged`]) — so the tree, the Inspector and the jump-to list draw one
//! vocabulary.
//!
//! **v1 keys are foreign** after the port: a `v1/…` key reads
//! [`Rung::NotZk2`], with no v1 overlay. The one surface still on v1, the
//! Inspector's type section until FL8, takes a v1 projection on demand
//! ([`v1_facts`]), never from a cache.

pub use zenkey_fleet::report::{KeyGroup, KeyIdentity, Unresolved};
pub use zenkey_fleet::{IdentityCache, Resolution, Resolved};

use crate::view::theme::IdentityTone;

/// The rung a key's identity reached, as a badge names it: one per rung of
/// the lens's ladder (`Unresolved`'s variants), plus the two answers that
/// carry no rung — resolved, and a control key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rung {
    /// Resolved to a resource of the contract (rung 6).
    Resolved,
    /// A zk2 control key (`@zk`): a token, a descriptor, a bundle — a zk2
    /// key no contract types.
    Control,
    /// Not under the deployment's namespace (rung 1): another
    /// deployment's, or none, and not guessed at (O3).
    NotInNamespace,
    /// In the namespace, and not a zk2 data key (rung 2).
    NotZk2,
    /// No presence read stood behind the lens (rung 3 not asked).
    PresenceNotRead,
    /// No instance of the address provides the interface (rung 3).
    NoProvider,
    /// A provider, and no descriptor named its revision (rung 4).
    NoRevision,
    /// The provider names more than one revision (rung 4).
    Ambiguous,
    /// The revision was not looked for, or not yet (rung 5 not asked).
    ContractNotHeld,
    /// Retrieval found no valid bundle (rung 5, §8.4).
    ContractUnavailable,
    /// A verified bundle this build cannot read (rung 5).
    ContractUnreadable,
    /// No resource of the contract matches the key (rung 6).
    NoResource,
    /// The resource declares no such member (rung 6).
    NoMember,
    /// The decode was switched off: nothing is claimed (O4).
    DecodeNotAsked,
}

impl Rung {
    /// Every rung, for the distinctness tests.
    pub const ALL: [Rung; 14] = [
        Rung::Resolved,
        Rung::Control,
        Rung::NotInNamespace,
        Rung::NotZk2,
        Rung::PresenceNotRead,
        Rung::NoProvider,
        Rung::NoRevision,
        Rung::Ambiguous,
        Rung::ContractNotHeld,
        Rung::ContractUnavailable,
        Rung::ContractUnreadable,
        Rung::NoResource,
        Rung::NoMember,
        Rung::DecodeNotAsked,
    ];

    /// The rung an identity names: where the ladder stopped, or the group
    /// when it did not stop.
    pub fn of(identity: &KeyIdentity) -> Rung {
        match &identity.unresolved {
            Some(u) => match u {
                Unresolved::NotInNamespace { .. } => Rung::NotInNamespace,
                Unresolved::NotZk2 { .. } => Rung::NotZk2,
                Unresolved::ControlKey { .. } => Rung::Control,
                Unresolved::PresenceNotRead => Rung::PresenceNotRead,
                Unresolved::NoProvider => Rung::NoProvider,
                Unresolved::NoRevision => Rung::NoRevision,
                Unresolved::Ambiguous { .. } => Rung::Ambiguous,
                Unresolved::ContractNotHeld { .. } => Rung::ContractNotHeld,
                Unresolved::ContractUnavailable { .. } => Rung::ContractUnavailable,
                Unresolved::ContractUnreadable { .. } => Rung::ContractUnreadable,
                Unresolved::NoResource { .. } => Rung::NoResource,
                Unresolved::NoMember { .. } => Rung::NoMember,
                Unresolved::DecodeNotAsked => Rung::DecodeNotAsked,
            },
            None => match identity.group {
                KeyGroup::Control { .. } => Rung::Control,
                KeyGroup::NotInNamespace => Rung::NotInNamespace,
                KeyGroup::NotZk2 => Rung::NotZk2,
                KeyGroup::Resource { .. } => Rung::Resolved,
            },
        }
    }

    /// The badge word: a few words, distinct per rung, so a glyph shared
    /// by a class never makes two rungs read alike (#193).
    pub fn label(self) -> &'static str {
        match self {
            Rung::Resolved => "resolved",
            Rung::Control => "control key",
            Rung::NotInNamespace => "not in namespace",
            Rung::NotZk2 => "not zk2",
            Rung::PresenceNotRead => "presence not read",
            Rung::NoProvider => "no provider",
            Rung::NoRevision => "no revision",
            Rung::Ambiguous => "ambiguous revision",
            Rung::ContractNotHeld => "contract not held",
            Rung::ContractUnavailable => "contract unavailable",
            Rung::ContractUnreadable => "contract unreadable",
            Rung::NoResource => "no resource",
            Rung::NoMember => "no member",
            Rung::DecodeNotAsked => "decode not asked",
        }
    }

    /// The rung's class, which picks the glyph and the swatch: a "no"
    /// obtained never wears a "not asked" mark, and neither reads as
    /// resolved (O4).
    pub fn tone(self) -> IdentityTone {
        match self {
            Rung::Resolved => IdentityTone::Resolved,
            Rung::NoProvider | Rung::NoRevision | Rung::Ambiguous | Rung::NoMember => {
                IdentityTone::Short
            }
            Rung::ContractUnavailable | Rung::ContractUnreadable | Rung::NoResource => {
                IdentityTone::Broken
            }
            Rung::PresenceNotRead | Rung::ContractNotHeld | Rung::DecodeNotAsked => {
                IdentityTone::NotAsked
            }
            Rung::NotZk2 | Rung::NotInNamespace | Rung::Control => IdentityTone::Inapplicable,
        }
    }
}

/// The rung a list row badges for `wire`, or `None` for a foreign key.
///
/// Every key has an identity (O1), but a badge on every foreign key of a
/// shared bus would say "not zk2" ten thousand times and nothing else. So a
/// key outside the namespace is badged only when it is somebody's zk2 key
/// (a `zk2` chunk in it: another deployment's), and a key in the namespace
/// that is not zk2 only when it claims a convention: it starts with `zk2`
/// and breaks the grammar, or with `v1` — keyspace-v2's own first grammar,
/// foreign since the port. Anything else is a foreign key, drawn plainly,
/// as v1's tree drew its not-applicable keys.
pub fn badged(wire: &str, namespace: &str, identity: &KeyIdentity) -> Option<Rung> {
    let rung = Rung::of(identity);
    match rung {
        Rung::NotInNamespace => wire
            .split('/')
            .any(|c| c == zenkey_model::grammar::GRAMMAR)
            .then_some(rung),
        Rung::NotZk2 => {
            let rel = zenkey_fleet::strip_namespace(namespace, wire).unwrap_or(wire);
            matches!(
                rel.split('/').next(),
                Some(zenkey_model::grammar::GRAMMAR | V1_CHUNK)
            )
            .then_some(rung)
        }
        _ => Some(rung),
    }
}

/// The first chunk of every key of keyspace-v2's v1 grammar: what makes a
/// foreign key worth its "not zk2" badge. Spelled here rather than read
/// from v1's crate, because it is a fact about the past, not an overlay.
const V1_CHUNK: &str = "v1";

/// A key's v1 projection, on demand (#614, FL7): the classification the
/// Inspector's type section still reads, until FL8. Its other readers, the
/// send form (FL10), the Config tool and the blob and media planes (FL11),
/// are gone. No slices since FL6a, so it costs a key parse; nothing caches
/// it, since nothing zk2 needs it.
pub fn v1_facts(namespace: &str, key: &str) -> zenkey_fleet_v1::KeyFacts {
    zenkey_fleet_v1::describe_key(namespace, key, None).facts
}

/// The declared type of the value a resolved key carries, as the report
/// names types: a data resource's `type`, an operation's `response`.
pub fn type_name(resolved: &Resolved) -> String {
    use zenkey_model::contract::Body;
    match &resolved.resource().body {
        Body::Data(d) => zenkey_fleet::type_view(&d.type_).name,
        Body::Operation(o) => zenkey_fleet::type_view(&o.response).name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn identity(group: KeyGroup, unresolved: Option<Unresolved>) -> KeyIdentity {
        KeyIdentity {
            group,
            values: BTreeMap::new(),
            unresolved,
        }
    }

    fn resource() -> KeyGroup {
        KeyGroup::Resource {
            address: "robot1/nav".into(),
            iface: "nav.v2".into(),
            token: "stream".into(),
            resource: None,
        }
    }

    /// Fourteen rungs, fourteen words: the badge is read, and two rungs
    /// that read alike are one rung to the reader.
    #[test]
    fn every_rung_has_its_own_word() {
        let words: std::collections::BTreeSet<&str> = Rung::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(words.len(), Rung::ALL.len());
    }

    /// The not-asked rungs never wear an answer's class, and the broken
    /// ones never read as merely short.
    #[test]
    fn not_asked_is_never_an_answer() {
        for r in [
            Rung::PresenceNotRead,
            Rung::ContractNotHeld,
            Rung::DecodeNotAsked,
        ] {
            assert_eq!(r.tone(), IdentityTone::NotAsked, "{r:?}");
        }
        assert_eq!(Rung::NoProvider.tone(), IdentityTone::Short);
        assert_eq!(Rung::NoResource.tone(), IdentityTone::Broken);
        assert_eq!(Rung::Resolved.tone(), IdentityTone::Resolved);
    }

    /// A `v1/…` key is foreign since the port and says so; a plain foreign
    /// key is drawn plainly; a broken `zk2/…` key is badged; another
    /// deployment's zk2 key is, and another application's key is not.
    #[test]
    fn foreign_keys_are_unlabelled_and_convention_keys_are_not() {
        let not_zk2 = || {
            identity(
                KeyGroup::NotZk2,
                Some(Unresolved::NotZk2 { detail: "x".into() }),
            )
        };
        let v1 = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/system/load";
        assert_eq!(badged(v1, "", &not_zk2()), Some(Rung::NotZk2));
        assert_eq!(
            badged(&format!("acme/{v1}"), "acme", &not_zk2()),
            Some(Rung::NotZk2),
            "a v1 key under the namespace"
        );
        assert_eq!(badged("demo/example/foo", "", &not_zk2()), None);
        assert_eq!(
            badged("zk2/robot1/nav/Bad/stream/x", "", &not_zk2()),
            Some(Rung::NotZk2),
            "a key that claims zk2 and breaks its grammar"
        );
        let elsewhere = || {
            identity(
                KeyGroup::NotInNamespace,
                Some(Unresolved::NotInNamespace {
                    namespace: "spray".into(),
                }),
            )
        };
        assert_eq!(
            badged(
                "elsewhere/zk2/robot1/nav/nav.v2/stream/position",
                "spray",
                &elsewhere()
            ),
            Some(Rung::NotInNamespace)
        );
        assert_eq!(badged("demo/example/foo", "spray", &elsewhere()), None);
        assert_eq!(
            badged(
                "zk2/robot1/nav/nav.v2/stream/position",
                "",
                &identity(resource(), Some(Unresolved::NoProvider))
            ),
            Some(Rung::NoProvider)
        );
    }

    /// A resource with no reason to stop is resolved; a control key is its
    /// own answer, never "resolved".
    #[test]
    fn the_groups_without_a_rung() {
        assert_eq!(Rung::of(&identity(resource(), None)), Rung::Resolved);
        assert_eq!(
            Rung::of(&identity(
                KeyGroup::Control {
                    address: Some("robot1/nav".into()),
                    form: "instance".into(),
                },
                None
            )),
            Rung::Control
        );
    }
}
