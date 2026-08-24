//! Validated key types (RFC 08 §1.2, issue #5).
//!
//! A [`Key`] is a **canonical, concrete, base-relative** v1 key: no wildcards,
//! starts at the `v1` chunk (the deployment base is the session namespace,
//! RFC 09 §0). A [`Selector`] is the same, except it may contain `*`/`**`.
//! Both wrap [`zenoh_keyexpr::OwnedKeyExpr`] — the exact type the `zenoh`
//! crate re-exports — so handing a key to the middleware is a move, never a
//! re-parse (the `OwnedKeyExpr::try_from(string).expect(..)` wrapper every
//! adopter wrote is the bug this module retires).
//!
//! A [`Chunk`] is one validated plain chunk (RFC 03 §2) — the unit of key
//! construction. `Chunk::slug` is the boundary where foreign values become
//! grammar-legal; generated subject constructors call it so call sites never
//! slug by hand.

use std::fmt;
use std::ops::Deref;

use zenoh_keyexpr::{OwnedKeyExpr, keyexpr};

use crate::grammar::KeyError;
use crate::slug::chunk_slug;

/// A validated, canonical, **concrete**, base-relative v1 key.
///
/// Obtained from the grammar/context/generated builders — there is no public
/// constructor from a raw string on purpose (parse wire keys with
/// [`crate::grammar::parse`] instead; build keys through builders).
///
/// "Concrete" is enforced, not merely documented (issue #312). The wrapping
/// constructor was `#[doc(hidden)] pub`, which hides an item from rustdoc and
/// from nobody else, and it was shared verbatim with [`Selector`] — so
/// `Key::from_canonical("v1/*/state/**")` succeeded and the two newtypes were
/// one type wearing two names. The constructor is now `pub(crate)`, reachable
/// from outside only through [`crate::__private`] (which generated code names
/// explicitly), and it *refuses* a wildcard.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(OwnedKeyExpr);

/// A validated, base-relative key expression that may contain `*`/`**`.
///
/// The one structural difference from [`Key`]: this constructor admits
/// wildcards and that one does not.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Selector(OwnedKeyExpr);

impl Key {
    /// Wrap a builder-produced, already-canonical, concrete key string.
    ///
    /// `pub(crate)`: builders are the only sound producers of this invariant,
    /// and outside the crate the sole path is [`crate::__private`]. The
    /// `expect` is pinned by the canonicality property test below — every
    /// grammar-legal key is already a canonical zenoh key expression, so this
    /// never re-canonizes and never fails. The wildcard assertion is the
    /// *structural* half of the type's claim: a builder that reaches here
    /// with a `*` has composed a selector, not a key, and says so at the
    /// point of the mistake rather than on the wire.
    pub(crate) fn from_canonical(s: String) -> Self {
        let ke = OwnedKeyExpr::try_from(s).expect("builder output is a canonical keyexpr");
        assert!(
            !is_wild(&ke),
            "a Key is concrete (RFC 08 §1.2): {ke} carries a wildcard — build a Selector"
        );
        Key(ke)
    }
}

/// Does this key expression carry a wildcard (`*`, `**`, `$*`)?
///
/// `keyexpr::is_wild` is gated behind zenoh-keyexpr's `internal` feature and
/// `#[doc(hidden)]`, so it is not ours to depend on. Its body is this test,
/// and the equivalence is exact: a canonical key expression admits `*` in no
/// other role — RFC 03 §2 excludes it from both chunk charsets.
fn is_wild(ke: &keyexpr) -> bool {
    ke.as_str().contains('*')
}

impl Selector {
    /// Wrap a builder-produced, already-canonical selector string. Wildcards
    /// are the point here; see [`Key::from_canonical`] for the rest.
    pub(crate) fn from_canonical(s: String) -> Self {
        Selector(OwnedKeyExpr::try_from(s).expect("builder output is a canonical keyexpr"))
    }
}

/// Not public API, and not a hiding place: the generated registry module
/// (zenkey-build) is compiled into a *foreign* crate, so it needs a reachable
/// path to the wrapping constructors. It names this one explicitly, which is
/// the whole design — a hand-written call site that types `__private` has
/// stated it is reaching past the contract, where `#[doc(hidden)] pub fn
/// from_canonical` merely looked like API with the docs turned off (#312).
///
/// Nothing here is covered by semver.
#[doc(hidden)]
pub mod __private {
    use super::{Key, Selector};

    /// Wrap a generated builder's concrete key string. Panics on a wildcard.
    pub fn key_from_canonical(s: String) -> Key {
        Key::from_canonical(s)
    }

    /// Wrap a generated builder's selector string.
    pub fn selector_from_canonical(s: String) -> Selector {
        Selector::from_canonical(s)
    }
}

macro_rules! keyexpr_newtype {
    ($ty:ident) => {
        impl $ty {
            /// The key as a borrowed [`keyexpr`] (alloc-free `intersects`/
            /// `includes` live there).
            pub fn as_keyexpr(&self) -> &keyexpr {
                &self.0
            }

            /// The key as a string slice.
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }
        }

        impl Deref for $ty {
            type Target = keyexpr;
            fn deref(&self) -> &keyexpr {
                &self.0
            }
        }

        impl fmt::Display for $ty {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl AsRef<str> for $ty {
            fn as_ref(&self) -> &str {
                self.0.as_str()
            }
        }

        impl From<$ty> for OwnedKeyExpr {
            /// Zero cost: the wrapped value *is* the middleware's type.
            fn from(k: $ty) -> OwnedKeyExpr {
                k.0
            }
        }

        impl From<$ty> for String {
            fn from(k: $ty) -> String {
                k.0.to_string()
            }
        }

        impl PartialEq<str> for $ty {
            fn eq(&self, other: &str) -> bool {
                self.as_str() == other
            }
        }

        impl PartialEq<&str> for $ty {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }

        impl PartialEq<String> for $ty {
            fn eq(&self, other: &String) -> bool {
                self.as_str() == other
            }
        }

        impl PartialEq<$ty> for str {
            fn eq(&self, other: &$ty) -> bool {
                self == other.as_str()
            }
        }

        impl PartialEq<$ty> for &str {
            fn eq(&self, other: &$ty) -> bool {
                *self == other.as_str()
            }
        }
    };
}

keyexpr_newtype!(Key);
keyexpr_newtype!(Selector);

impl From<Key> for Selector {
    /// Every concrete key is a valid selector.
    fn from(k: Key) -> Selector {
        Selector(k.0)
    }
}

/// One validated plain chunk (RFC 03 §2): `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chunk(String);

impl Chunk {
    /// Slug an arbitrary foreign value into a legal chunk (RFC 03 §2's
    /// injective `_xNN_` escape; case-sensitive domains survive, G4).
    /// Always succeeds — this is the API boundary where application values
    /// become grammar-legal.
    pub fn slug(value: impl AsRef<str>) -> Chunk {
        Chunk(chunk_slug(value.as_ref()))
    }

    /// Accept a value that must already be a legal chunk (no slugging).
    pub fn parse(value: &str) -> Result<Chunk, KeyError> {
        if crate::grammar::is_valid_plain_chunk(value) {
            Ok(Chunk(value.to_string()))
        } else {
            Err(KeyError::InvalidPlainChunk(value.to_string()))
        }
    }

    /// The chunk as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for Chunk {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Chunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Chunk {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Chunk {
    /// Slugs — total, like [`Chunk::slug`]; struct-literal construction of
    /// generated subjects stays boundary-safe.
    fn from(v: &str) -> Chunk {
        Chunk::slug(v)
    }
}

impl From<String> for Chunk {
    fn from(v: String) -> Chunk {
        Chunk::slug(&v)
    }
}

impl From<&crate::origin::HostId> for Chunk {
    /// A host origin is `h-[0-9a-f]{12}` (RFC 03 §1.3), which is a legal plain
    /// chunk by construction — the one conversion that is total *and* needs no
    /// slugging. This replaced `Chunk::from_valid`, which took the caller's
    /// word for it and re-checked only under `debug_assert`, so a release
    /// build admitted an illegal chunk (#312). The generated `{host}`
    /// constructor in a service registry is the caller; the wire-parse path
    /// uses `Chunk::parse(..).ok()?`, where untrusted input belongs.
    fn from(id: &crate::origin::HostId) -> Chunk {
        Chunk(id.as_str().to_string())
    }
}

impl PartialEq<str> for Chunk {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Chunk {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar::{self, Class, Origin, Producer};
    use crate::origin::HostId;

    fn host() -> Origin {
        Origin::Host(HostId::parse("h-3fa9c2d41b7e").unwrap())
    }

    /// The invariant `from_canonical` rests on: every grammar-legal key is
    /// *already* a canonical zenoh keyexpr — wrapping never rewrites.
    #[test]
    fn grammar_output_is_already_canonical() {
        let producer = Producer::new("netring").unwrap();
        let built = [
            grammar::data_key(
                &host(),
                Class::Telemetry,
                Some(&producer),
                &["flow", "red", "p95_ms"],
            )
            .unwrap(),
            grammar::rpc_key(&host(), Some(&producer), &["capture_disk", "set"]).unwrap(),
            grammar::alive_key(&host(), Some(&producer)).unwrap(),
            grammar::data_key(&Origin::catalog(), Class::State, None, &["entity", "abc"]).unwrap(),
        ];
        for s in built {
            let ke = OwnedKeyExpr::autocanonize(s.to_string()).unwrap();
            assert_eq!(ke.as_str(), s.as_str(), "canonization rewrote {s}");
            // And the wrap itself works.
            let key = Key::from_canonical(s.to_string());
            assert_eq!(key, s.as_str());
        }
    }

    #[test]
    fn key_moves_into_owned_keyexpr() {
        let key = Key::from_canonical("v1/h-3fa9c2d41b7e/state/netring/health".to_string());
        let ke: OwnedKeyExpr = key.clone().into();
        assert_eq!(ke.as_str(), key.as_str());
        let sel: Selector = key.into();
        assert_eq!(sel, "v1/h-3fa9c2d41b7e/state/netring/health");
    }

    #[test]
    fn selector_intersects_via_deref() {
        let sel = Selector::from_canonical("v1/*/telemetry/**".to_string());
        let key = Key::from_canonical("v1/h-3fa9c2d41b7e/telemetry/netring/flow".to_string());
        assert!(sel.intersects(&key));
    }

    #[test]
    fn chunk_slug_and_parse() {
        assert_eq!(Chunk::slug("p95_ms"), "p95_ms");
        // Foreign values get the injective escape and stay legal.
        let dirty = Chunk::slug("Röuter 1/ETH0");
        assert!(crate::grammar::is_valid_plain_chunk(dirty.as_str()));
        assert!(Chunk::parse("p95_ms").is_ok());
        assert!(Chunk::parse("Not A Chunk").is_err());
        assert!(Chunk::parse("").is_err());
    }

    /// The `h-<12hex>` shape is a legal plain chunk, so the conversion is
    /// total and lossless — never the slug's `_xNN_` escape.
    #[test]
    fn a_host_id_converts_to_a_chunk_verbatim() {
        let id = HostId::parse("h-3fa9c2d41b7e").unwrap();
        let chunk = Chunk::from(&id);
        assert_eq!(chunk, "h-3fa9c2d41b7e");
        assert!(crate::grammar::is_valid_plain_chunk(chunk.as_str()));
    }

    /// Issue #312: a wildcard string cannot become a `Key` by any public
    /// path. The two the crate exposes are the builders — which run the
    /// grammar first — and `__private`, which asserts. What Rust cannot
    /// assert is a *missing* item, so the half that can be asserted is
    /// pinned here and the reasoning sits beside it.
    #[test]
    #[should_panic(expected = "a Key is concrete")]
    fn a_wildcard_is_not_a_key() {
        let _ = crate::__private::key_from_canonical("v1/*/state/**".to_string());
    }

    /// …and the same string *is* a selector. This is the structural
    /// difference the doc comment claimed while both types shared one
    /// constructor.
    #[test]
    fn the_same_wildcard_is_a_selector() {
        let sel = crate::__private::selector_from_canonical("v1/*/state/**".to_string());
        assert_eq!(sel, "v1/*/state/**");
    }

    /// Every wildcard shape the grammar can produce is refused, not just the
    /// `*` in position 2: `**`, a wild subject leaf, a `$*` verbatim match.
    #[test]
    fn every_wildcard_shape_is_refused() {
        for wild in [
            "v1/*/state/netring/health",
            "v1/h-3fa9c2d41b7e/state/*/health",
            "v1/h-3fa9c2d41b7e/state/netring/**",
            "v1/h-3fa9c2d41b7e/**/health",
        ] {
            let attempt =
                std::panic::catch_unwind(|| crate::__private::key_from_canonical(wild.to_string()));
            assert!(attempt.is_err(), "{wild} must not become a Key");
            // The selector newtype takes all of them.
            assert_eq!(
                crate::__private::selector_from_canonical(wild.to_string()),
                wild
            );
        }
    }
}
