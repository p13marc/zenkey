//! The `@blob` plane, as an explorer sees it (RFC 07 §2; issues #58, #68).
//!
//! RFC v1.8 modelled `[[blob]]` in the registry, in its own words, "so that an
//! explorer can see which origins serve blobs, and of which tier". This module
//! is the half that makes that true: the registry projection ([`blob_list`]),
//! the addressing type ([`BlobTarget`]), and — behind the `blob` feature — the
//! two bus operations §2.5 sanctions, in the order it sanctions them.
//!
//! **The shape of the plane, and why it is not just another fan-out.** Every
//! other plane an explorer reads answers in kilobytes. `@blob` answers in
//! files. So RFC 07 §2.5 splits the interaction in two: *probe* across origins
//! with a tiny reply (`have`, `manifest`), then *fetch* from the one origin you
//! chose, at its concrete key. A wildcard-origin bulk fetch is not a slow path
//! to be discouraged — Zenoh cannot cancel replies already in flight, so N
//! holders cost N× the bytes with no way to stop them — and this crate makes it
//! unspellable rather than unfashionable: the wide form is a
//! [`BlobProbePrefix`], which is not a [`Key`] and does not convert into one,
//! and [`blob_fetch`] takes a concrete origin that goes through
//! [`RemoteOrigin::parse`].
//!
//! **What this module does not implement.** Verified streaming. RFC 07 §2
//! names [`zblob`] the reference client and §2.1 makes per-reply verification
//! *before disk* normative; a second implementation of an integrity anchor is a
//! second thing that can be wrong about the same bytes. So the fetch path is
//! zblob's, and this module's job is to spell the keys through zenkey's typed
//! builders, attribute replies the way RFC 05 §2.1 requires, and report the
//! result in a shape both frontends can render.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use zenkey::grammar::{self, BlobTier, ContentHash, Origin};
use zenkey::{BlobProbePrefix, Key, RegistrySlice};

use crate::report::{BlobList, BlobListSource, BlobTierRow};

/// The three reserved tier tokens, in RFC 07 §2's order.
const KNOWN_TIERS: [&str; 3] = ["artifact", "tree", "store"];

/// What a blob command addresses: RFC 07 §2's three shapes, each validated.
///
/// There is deliberately no `String` constructor for the content-addressed
/// tiers — a `tree` or `store` address is a [`ContentHash`] or it does not
/// exist. RFC 07 §2.3 revoked the caller-chosen tree name (`tree/nightly`) in
/// v1.7, and the generated builders have refused to spell one ever since; this
/// type refuses for the same reason, at the point where an operator's typing
/// enters the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobTarget {
    /// Tier-1: a named artifact, whose endpoints live under it (RFC 07 §2.2).
    Artifact { id: String },
    /// Tier-2: a directory index, keyed by its own root (RFC 07 §2.3).
    Tree { root: ContentHash },
    /// Tier-2: one content-addressed chunk (RFC 07 §2.4).
    Store { algo: String, hash: ContentHash },
}

impl BlobTarget {
    /// Parse the one spelling both frontends accept:
    ///
    /// ```text
    /// <id>  |  artifact/<id>  |  tree/<hex>  |  store/<algo>/<hex>
    /// ```
    ///
    /// A bare id is Tier-1, because that is the tier an operator has an id
    /// *for*: Tier-2 addresses are hashes, and nobody types one from memory.
    ///
    /// An id that is not a valid RFC 03 §2 plain chunk is refused with the
    /// citation rather than lowercased. A canonical ULID is uppercase Crockford
    /// base32 and a key chunk has no uppercase spelling (RFC 07 §2.2 as
    /// amended in v1.11) — but silently rewriting the caller's id would mean
    /// probing for something they did not ask for and reporting holders of it,
    /// which is worse than refusing.
    pub fn parse(spec: &str) -> Result<BlobTarget> {
        let spec = spec.trim().trim_matches('/');
        if spec.is_empty() {
            bail!(
                "empty blob target: expected <id>, artifact/<id>, tree/<hex>, or store/<algo>/<hex>"
            );
        }
        let parts: Vec<&str> = spec.split('/').collect();
        match parts.as_slice() {
            ["artifact", id] => Self::artifact(id),
            ["tree"] => bail!(
                "tree/ needs the tree's root hash: `tree/<hex>` (RFC 07 §2.3 — a tree is keyed by its own root, and a caller-chosen name has no spelling)"
            ),
            ["tree", root] => Ok(BlobTarget::Tree {
                root: content_hash(root, "tree")?,
            }),
            ["store"] | ["store", _] => {
                bail!("store/ needs both chunks: `store/<algo>/<hex>` (RFC 07 §2.4)")
            }
            ["store", algo, hash] => {
                if !grammar::is_valid_plain_chunk(algo) {
                    bail!(
                        "`{algo}` is not a valid algorithm chunk: RFC 03 §2 requires [a-z0-9]([a-z0-9._-]*[a-z0-9])?"
                    );
                }
                Ok(BlobTarget::Store {
                    algo: (*algo).to_string(),
                    hash: content_hash(hash, "store")?,
                })
            }
            [id] => Self::artifact(id),
            _ => bail!(
                "`{spec}` is not a blob target: expected <id>, artifact/<id>, tree/<hex>, or store/<algo>/<hex>"
            ),
        }
    }

    fn artifact(id: &str) -> Result<BlobTarget> {
        if !grammar::is_valid_plain_chunk(id) {
            let hint = if id.chars().any(|c| c.is_ascii_uppercase()) {
                " — a ULID is key-encoded in lowercase (RFC 03 §2, RFC 07 §2.2); lowercase it at the source rather than here, so the id you probe for is the id you were given"
            } else {
                ""
            };
            bail!(
                "`{id}` is not a valid artifact id: RFC 03 §2 requires one plain chunk matching [a-z0-9]([a-z0-9._-]*[a-z0-9])?{hint}"
            );
        }
        Ok(BlobTarget::Artifact { id: id.to_string() })
    }

    pub fn tier(&self) -> BlobTier {
        match self {
            BlobTarget::Artifact { .. } => BlobTier::Artifact,
            BlobTarget::Tree { .. } => BlobTier::Tree,
            BlobTarget::Store { .. } => BlobTier::Store,
        }
    }

    /// The `*`-origin probe prefix (RFC 07 §2.5) — the only wildcard form that
    /// exists for this plane, and not a [`Key`].
    pub fn probe_prefix(&self) -> BlobProbePrefix {
        BlobProbePrefix::new(self.tier())
    }

    /// This target's concrete key under one origin — the only fetchable form.
    ///
    /// For Tier-1 that is the artifact's base key, which the endpoint tails of
    /// RFC 07 §2.2 hang off; for Tier-2 the key *is* the object.
    pub fn key_at(&self, origin: &Origin) -> Result<Key> {
        let key = match self {
            BlobTarget::Artifact { id } => grammar::blob_key(origin, BlobTier::Artifact, &[id])?,
            BlobTarget::Tree { root } => grammar::blob_tree_key(origin, root)?,
            BlobTarget::Store { algo, hash } => grammar::blob_store_key(origin, algo, hash)?,
        };
        Ok(key)
    }

    /// The tier prefix under one origin: `v1/<origin>/@blob/<tier>`. This is
    /// what the reference client's endpoint helpers append to.
    pub fn prefix_at(&self, origin: &Origin) -> Key {
        grammar::blob_tier_prefix(origin, self.tier())
    }

    /// The canonical spelling, which round-trips through [`parse`](Self::parse).
    pub fn spelling(&self) -> String {
        match self {
            BlobTarget::Artifact { id } => format!("artifact/{id}"),
            BlobTarget::Tree { root } => format!("tree/{root}"),
            BlobTarget::Store { algo, hash } => format!("store/{algo}/{hash}"),
        }
    }

    /// The tier-1 id, for the reference client's per-id endpoint helpers.
    pub(crate) fn artifact_id(&self) -> Option<&str> {
        match self {
            BlobTarget::Artifact { id } => Some(id),
            _ => None,
        }
    }
}

fn content_hash(text: &str, tier: &str) -> Result<ContentHash> {
    ContentHash::parse(text).map_err(|e| {
        anyhow::anyhow!(
            "`{text}` is not a content hash for `{tier}`: {e} (RFC 07 §2.3/§2.4 — the key is the digest, so it is lowercase hex of even length)"
        )
    })
}

/// Which producers declare which `@blob` tiers, from registry slices.
///
/// **No bus traffic.** This is what a slice *says*, which is the only thing
/// RFC 08 §2's `[[blob]]` table can tell anyone: a producer declaring a tier
/// claims it serves that tier's endpoints, never that it holds any particular
/// blob. Possession is a probe's answer, and only a probe's.
///
/// `roster` is the liveliness map as [`crate::roster`] returns it (origin →
/// producers), inverted here to fill `origins`. Pass `None` when it was not
/// asked — an offline `--registry` read has learned nothing about who is up,
/// and `origins: None` is how that stays distinguishable from "declared, but
/// nobody is serving it" (RFC 09 §5.1 O4).
pub fn blob_list(
    slices: &[RegistrySlice],
    roster: Option<&BTreeMap<String, Vec<String>>>,
    source: BlobListSource,
) -> BlobList {
    // roster is origin → producers; the row wants producer → origins.
    let by_producer: Option<BTreeMap<&str, Vec<String>>> = roster.map(|r| {
        let mut out: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (origin, producers) in r {
            for producer in producers {
                out.entry(producer.as_str())
                    .or_default()
                    .push(origin.clone());
            }
        }
        out
    });

    let mut tiers = Vec::new();
    let mut slices_without_blob = 0usize;
    for slice in slices {
        if slice.blob.is_empty() {
            slices_without_blob += 1;
            continue;
        }
        for decl in &slice.blob {
            tiers.push(BlobTierRow {
                producer: slice.name.clone(),
                registry_version: slice.version.clone(),
                known_tier: KNOWN_TIERS.contains(&decl.tier.as_str()),
                tier: decl.tier.clone(),
                endpoints: decl.endpoints.clone(),
                algo: decl.algo.clone(),
                reference: decl.reference.clone(),
                encoding: decl.encoding.clone(),
                since: decl.since.clone(),
                description: decl.description.clone(),
                origins: by_producer
                    .as_ref()
                    .map(|m| m.get(slice.name.as_str()).cloned().unwrap_or_default()),
            });
        }
    }
    tiers.sort_by(|a, b| (&a.producer, &a.tier).cmp(&(&b.producer, &b.tier)));

    BlobList {
        tiers,
        source,
        slices_considered: slices.len(),
        slices_without_blob,
    }
}

/// Producers whose slice declares `tier` — the capability claim behind a
/// probe, so silence stays legible (RFC 05 §3.1).
pub fn declared_by(slices: &[RegistrySlice], tier: BlobTier) -> Vec<String> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for slice in slices {
        if slice.serves_blob_tier(tier.chunk()) {
            names.insert(slice.name.clone());
        }
    }
    names.into_iter().collect()
}

#[cfg(feature = "blob")]
mod bus;
#[cfg(feature = "blob")]
pub use bus::{BlobFetchSpec, FETCH_PRIORITY, blob_fetch, blob_probe};

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn origin() -> Origin {
        Origin::Host(zenkey::HostId::parse("h-3fa9c2d41b7e").unwrap())
    }

    #[test]
    fn a_bare_id_is_tier_one() {
        assert_eq!(
            BlobTarget::parse("01jqz3demo0001").unwrap(),
            BlobTarget::Artifact {
                id: "01jqz3demo0001".into()
            }
        );
        assert_eq!(
            BlobTarget::parse("artifact/01jqz3demo0001").unwrap(),
            BlobTarget::parse("01jqz3demo0001").unwrap()
        );
    }

    #[test]
    fn every_target_round_trips_through_its_spelling() {
        for spec in [
            "artifact/01jqz3demo0001",
            &format!("tree/{HASH}"),
            &format!("store/blake3/{HASH}"),
        ] {
            let target = BlobTarget::parse(spec).unwrap();
            assert_eq!(target.spelling(), spec);
            assert_eq!(BlobTarget::parse(&target.spelling()).unwrap(), target);
        }
    }

    #[test]
    fn an_uppercase_ulid_is_refused_with_the_citation() {
        // The canonical display form of a ULID. RFC 03 §2 has no uppercase
        // spelling, so this key cannot exist — and lowercasing it silently
        // would probe for an id the caller never gave us.
        let err = BlobTarget::parse("01HQXK8F9C2N4PZQ")
            .unwrap_err()
            .to_string();
        assert!(err.contains("RFC 03 §2"), "{err}");
        assert!(err.contains("lowercase"), "{err}");
    }

    #[test]
    fn a_wildcard_is_not_a_target() {
        for spec in ["*", "**", "artifact/*", "v1/*/@blob/artifact", "a/b/c/d"] {
            assert!(
                BlobTarget::parse(spec).is_err(),
                "`{spec}` must not parse as a blob target"
            );
        }
    }

    #[test]
    fn tier_two_needs_a_hash_not_a_name() {
        // RFC 07 §2.3's revoked spelling, and the shapes around it.
        for spec in ["tree/nightly", "tree", "store", "store/blake3", "tree/abc"] {
            assert!(
                BlobTarget::parse(spec).is_err(),
                "`{spec}` must not parse as a blob target"
            );
        }
        assert!(BlobTarget::parse(&format!("tree/{HASH}")).is_ok());
    }

    #[test]
    fn keys_come_out_of_the_typed_builders() {
        let o = origin();
        assert_eq!(
            BlobTarget::parse("01jqz3demo0001")
                .unwrap()
                .key_at(&o)
                .unwrap()
                .as_str(),
            "v1/h-3fa9c2d41b7e/@blob/artifact/01jqz3demo0001"
        );
        assert_eq!(
            BlobTarget::parse(&format!("store/blake3/{HASH}"))
                .unwrap()
                .key_at(&o)
                .unwrap()
                .as_str(),
            format!("v1/h-3fa9c2d41b7e/@blob/store/blake3/{HASH}")
        );
        assert_eq!(
            BlobTarget::parse("01jqz3demo0001")
                .unwrap()
                .prefix_at(&o)
                .as_str(),
            "v1/h-3fa9c2d41b7e/@blob/artifact"
        );
        assert_eq!(
            BlobTarget::parse("01jqz3demo0001")
                .unwrap()
                .probe_prefix()
                .as_str(),
            "v1/*/@blob/artifact"
        );
    }

    fn slice_with_blob(name: &str, body: &str) -> RegistrySlice {
        let toml = format!(
            "[registry]\nversion = \"7\"\napp = \"demo\"\nconvention = 1\n\n\
             [producer]\nname = \"{name}\"\n\n{body}"
        );
        zenkey::parse_slice(&toml).unwrap()
    }

    #[test]
    fn a_declaration_without_a_roster_says_so() {
        let slices = vec![
            slice_with_blob(
                "netring",
                "[[blob]]\ntier = \"artifact\"\nendpoints = [\"manifest\", \"have\"]\n",
            ),
            slice_with_blob("quiet", ""),
        ];
        let list = blob_list(&slices, None, BlobListSource::RegistryDirs);
        assert_eq!(list.tiers.len(), 1);
        assert_eq!(list.slices_considered, 2);
        assert_eq!(list.slices_without_blob, 1);
        // O4: nobody asked who is up, so this is not "no origin serves it".
        assert!(list.tiers[0].origins.is_none());

        let roster = BTreeMap::from([("h-3fa9c2d41b7e".to_string(), vec!["netring".to_string()])]);
        let joined = blob_list(&slices, Some(&roster), BlobListSource::Bus);
        assert_eq!(
            joined.tiers[0].origins.as_deref(),
            Some(["h-3fa9c2d41b7e".to_string()].as_slice())
        );
    }

    #[test]
    fn an_unreserved_tier_survives_flagged_rather_than_dropped() {
        // O1: a declaration this build does not understand is a fact about the
        // fleet, and dropping it would report a registry we did not read.
        let slices = vec![slice_with_blob("future", "[[blob]]\ntier = \"hologram\"\n")];
        let list = blob_list(&slices, None, BlobListSource::Bus);
        assert_eq!(list.tiers.len(), 1);
        assert_eq!(list.tiers[0].tier, "hologram");
        assert!(!list.tiers[0].known_tier);
    }

    #[test]
    fn declared_by_names_the_claimants() {
        let slices = vec![
            slice_with_blob("netring", "[[blob]]\ntier = \"artifact\"\n"),
            slice_with_blob("logs", "[[blob]]\ntier = \"store\"\nalgo = \"blake3\"\n"),
        ];
        assert_eq!(declared_by(&slices, BlobTier::Artifact), vec!["netring"]);
        assert_eq!(declared_by(&slices, BlobTier::Store), vec!["logs"]);
        assert!(declared_by(&slices, BlobTier::Tree).is_empty());
    }
}
