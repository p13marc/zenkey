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

/// A validated, canonical, concrete, base-relative v1 key.
///
/// Obtained from the grammar/context/generated builders — there is no public
/// constructor from a raw string on purpose (parse wire keys with
/// [`crate::grammar::parse`] instead; build keys through builders).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(OwnedKeyExpr);

/// A validated, base-relative key expression that may contain `*`/`**`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Selector(OwnedKeyExpr);

macro_rules! keyexpr_newtype {
    ($ty:ident) => {
        impl $ty {
            /// Wrap a builder-produced, already-canonical string.
            ///
            /// Not part of the public contract — builders are the only sound
            /// producers of this invariant. The `expect` is pinned by the
            /// canonicality property test below: every grammar-legal key is
            /// already a canonical zenoh key expression, so this never
            /// re-canonizes and never fails.
            #[doc(hidden)]
            pub fn from_canonical(s: String) -> Self {
                Self(OwnedKeyExpr::try_from(s).expect("builder output is a canonical keyexpr"))
            }

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

    /// Wrap a chunk that arrived from the wire and was therefore already
    /// validated by the grammar. Debug-asserted, not re-validated — the
    /// generated parse path calls this per bound variable.
    #[doc(hidden)]
    pub fn from_valid(value: &str) -> Chunk {
        debug_assert!(
            crate::grammar::is_valid_plain_chunk(value),
            "from_valid on an illegal chunk: {value:?}"
        );
        Chunk(value.to_string())
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
}
