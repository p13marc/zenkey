//! The v1 keyspace context: origin + producer in one value.
//!
//! One value carries everything a producer needs to build conforming keys:
//! the host origin (`h-<12hex>`, minted once per process via the application's
//! [`AppProfile`]) and the producer chunk. All framework keys flow through
//! here — producers never spell `v1` by hand.
//!
//! **Keys built here are base-relative** — they start at the `v1` chunk. The
//! deployment base rides the Zenoh session `namespace` (RFC 03 §1.1 / 09 §0),
//! which prefixes it on egress and strips it on ingress. So there is
//! deliberately no way to ask a context for the base: application code has no
//! vocabulary for it, and that is the point — a base you cannot spell is a
//! base you cannot spell *wrong*.

use std::num::NonZeroU32;

use crate::grammar::{self, KeyError, Origin, Producer};
use crate::key::Key;
use crate::profile::AppProfile;
use crate::slug::chunk_slug;

/// Everything needed to build this producer's v1 keys.
///
/// Note what is *not* here: the deployment base. Every key below is
/// base-relative (`v1/…`); the session namespace supplies the rest.
#[derive(Debug, Clone)]
pub struct V1Context {
    origin: Origin,
    producer: Producer,
}

impl V1Context {
    /// Build the context for one producer on this host: origin = the host id
    /// minted through `profile`, producer = `name`, which MUST be a legal
    /// producer chunk (RFC 03 §1.5).
    ///
    /// **Errs rather than renaming** (issue #322). This used to slug a bad
    /// name and, failing that, fall back to `Producer::new("sensor")` — so a
    /// misconfigured producer published its *entire keyspace under a different
    /// identity*, with no `Err`, no panic and no log, and every misconfigured
    /// producer in the fleet collided on the same fallback name. In a crate
    /// whose thesis is that a non-conforming key should have no spelling, that
    /// was the one place a wrong key was spelled for you.
    ///
    /// A name that genuinely is foreign data has a boundary to cross:
    /// `Producer::new(Chunk::slug(name).as_str())`. Making the caller write
    /// that is the point — slugging an *identity* is a decision, not a
    /// fallback.
    pub fn for_producer(profile: &'static AppProfile, name: &str) -> Result<Self, KeyError> {
        Self::with_origin(Origin::Host(profile.host_id().clone()), name)
    }

    /// As [`for_producer`](Self::for_producer) with an explicit origin — for
    /// tests, and for consumers that mint their identity differently.
    pub fn with_origin(origin: Origin, name: &str) -> Result<Self, KeyError> {
        Ok(Self::with_producer(origin, Producer::new(name)?))
    }

    /// As [`with_origin`](Self::with_origin) with an already-validated
    /// [`Producer`] — the infallible form, and the one that carries an
    /// instance built by [`Producer::with_instance`].
    #[must_use]
    pub fn with_producer(origin: Origin, producer: Producer) -> Self {
        Self { origin, producer }
    }

    /// As [`for_producer`](Self::for_producer) with an explicit producer
    /// instance (RFC 03 §1.5).
    ///
    /// Takes a [`NonZeroU32`] because instance numbers start at 1 — the first
    /// instance uses the bare name. It used to take a `u32` and *swallow*
    /// `Producer::with_instance`'s rejection of 0, so `.with_instance(0)` was
    /// a no-op that read like a configuration (issue #322). Zero now has no
    /// spelling at all.
    #[must_use]
    pub fn with_instance(mut self, instance: NonZeroU32) -> Self {
        self.producer = Producer::with_instance(self.producer.name(), instance.get())
            .expect("the name is already validated and a NonZeroU32 is never zero");
        self
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    pub fn producer(&self) -> &Producer {
        &self.producer
    }

    /// The telemetry prefix: `v1/<origin>/telemetry/<producer>`.
    /// Metric suffixes append below it ({metric...} / {device}/{metric...}
    /// registry families).
    #[must_use]
    pub fn telemetry_prefix(&self) -> Key {
        Key::from_canonical(format!(
            "{}/{}/{}/{}",
            grammar::VERSION_CHUNK,
            self.origin.chunk(),
            grammar::CLASS_TELEMETRY,
            self.producer.chunk()
        ))
    }

    /// A `state/<producer>/<subject...>` key. Subject chunks are slugged
    /// where not already legal.
    ///
    /// Errs on the reserved `alive` token (RFC 03 §3) with the same
    /// [`KeyError::ReservedToken`] [`grammar::data_key`] returns for the
    /// identical input — one rule, one failure mode. It used to `assert!`,
    /// so the same mistake was a recoverable error one layer down and a panic
    /// here (issue #322). Liveliness keys come from [`Self::alive_key`].
    pub fn state_key(&self, subject: &[&str]) -> Result<Key, KeyError> {
        grammar::reject_reserved_chunks(subject, "data subject chunk")?;
        Ok(self.build_key(grammar::CLASS_STATE, subject))
    }

    /// Single-pass slug-and-assemble (v1.5 perf: one buffer, no intermediate
    /// Vecs — the double-`Vec` per build was a measured hotspot).
    fn build_key(&self, class_or_plane: &str, subject: &[&str]) -> Key {
        debug_assert!(!subject.is_empty());
        let mut key = String::with_capacity(
            8 + self.origin.chunk().len()
                + class_or_plane.len()
                + self.producer.name().len()
                + subject.iter().map(|c| c.len() + 1).sum::<usize>()
                + 8,
        );
        key.push_str(grammar::VERSION_CHUNK);
        key.push('/');
        key.push_str(self.origin.chunk());
        key.push('/');
        key.push_str(class_or_plane);
        key.push('/');
        self.producer.push_chunk(&mut key);
        for c in subject {
            key.push('/');
            if grammar::is_valid_plain_chunk(c) {
                key.push_str(c);
            } else {
                key.push_str(&chunk_slug(c));
            }
        }
        Key::from_canonical(key)
    }

    // The framework subjects below are literal constants, so the reserved
    // token cannot occur and they stay infallible — `build_key` is the same
    // assembly `state_key` runs after its check.

    #[must_use]
    pub fn health_key(&self) -> Key {
        self.build_key(grammar::CLASS_STATE, &["health"])
    }

    #[must_use]
    pub fn errors_key(&self) -> Key {
        self.build_key(grammar::CLASS_STATE, &["errors"])
    }

    /// The registration document (RFC: `state/<producer>/sensor`).
    #[must_use]
    pub fn sensor_info_key(&self) -> Key {
        self.build_key(grammar::CLASS_STATE, &["sensor"])
    }

    #[must_use]
    pub fn evidence_self_key(&self) -> Key {
        self.build_key(grammar::CLASS_STATE, &["evidence", "self"])
    }

    /// `device` is a foreign value, so this carries [`Self::state_key`]'s
    /// contract: slugged where not already legal, refused where reserved.
    pub fn evidence_device_key(&self, device: &str) -> Result<Key, KeyError> {
        self.state_key(&["evidence", "device", device])
    }

    /// Liveliness token key (RFC 04 §5) — machinery, not a data subject.
    #[must_use]
    pub fn alive_key(&self) -> Key {
        grammar::alive_key(&self.origin, Some(&self.producer)).expect("producer context is valid")
    }

    /// Device liveliness token key (RFC 04 §5).
    #[must_use]
    pub fn device_alive_key(&self, device: &str) -> Key {
        let device = chunk_slug(device);
        grammar::device_alive_key(&self.origin, &self.producer, &device)
            .expect("slugged device chunk is valid")
    }

    /// An `@rpc/<producer>/<procedure...>` key (RFC 05).
    ///
    /// Returns the error [`grammar::rpc_key`] returns; it used to `.expect()`
    /// on it, which made an illegal procedure chunk a panic here and a
    /// recoverable `Err` one layer down (issue #322).
    pub fn rpc_key(&self, procedure: &[&str]) -> Result<Key, KeyError> {
        grammar::reject_reserved_chunks(procedure, "procedure chunk")?;
        grammar::rpc_key(&self.origin, Some(&self.producer), procedure)
    }

    /// Media plane video key (RFC 07 §1): the last chunk is a viewer-chosen
    /// **tier** (`low`/`medium`/`high`), not a codec profile — the viewer
    /// subscribes to it exactly (keyspace v1.3).
    pub fn media_video_key(&self, stream: &str, codec: &str, tier: &str) -> Result<Key, KeyError> {
        self.media_key(&[stream, "video", codec, tier])
    }

    /// A general `@media/<producer>/<stream...>` key (RFC 07 §1). Chunks are
    /// slugged where not already legal.
    ///
    /// The reserved token is refused here too (RFC 03 §3 reserves `alive` at
    /// any position of a media pattern, not only a data subject): slugging
    /// leaves `alive` untouched — it is already a legal chunk — so without
    /// the check this builder was the one that minted a presence-shaped media
    /// key in silence (issue #322).
    pub fn media_key(&self, stream: &[&str]) -> Result<Key, KeyError> {
        grammar::reject_reserved_chunks(stream, "media stream chunk")?;
        Ok(self.build_key(grammar::PLANE_MEDIA, stream))
    }

    /// The `@blob` tier prefix (RFC 07 §2): `v1/<origin>/@blob/<tier>` —
    /// Tier-1 `artifact`, Tier-2 `tree`/`store`.
    ///
    /// Note this value travels **inside payloads**, so it is a base-relative
    /// keyexpr in a document: meaningful only to a session set to the same
    /// deployment namespace. An un-namespaced reader must
    /// [`grammar::with_base`] it.
    #[must_use]
    pub fn blob_prefix(&self, tier: grammar::BlobTier) -> Key {
        grammar::blob_tier_prefix(&self.origin, tier)
    }

    /// A Tier-2 **tree** key under this origin (RFC 07 §2.3):
    /// `v1/<origin>/@blob/tree/<root>`.
    pub fn blob_tree_key(&self, root: &grammar::ContentHash) -> Result<Key, grammar::KeyError> {
        grammar::blob_tree_key(&self.origin, root)
    }

    /// A Tier-2 **chunk** key under this origin (RFC 07 §2.4):
    /// `v1/<origin>/@blob/store/<algo>/<hash>`.
    pub fn blob_store_key(
        &self,
        algo: &str,
        hash: &grammar::ContentHash,
    ) -> Result<Key, grammar::KeyError> {
        grammar::blob_store_key(&self.origin, algo, hash)
    }
}

/// A `*`-origin `@blob` prefix, for **probing only** (RFC 07 §2.5).
///
/// Deliberately a distinct type from [`Key`], and deliberately not
/// convertible into one: a fleet prefix and a concrete prefix are
/// interchangeable as strings, which is exactly how a probe turns into a
/// wildcard-origin *bulk fetch* — every holder ships the full payload and
/// Zenoh cannot cancel remote replies in flight, so N holders cost N× the
/// bytes (RFC 07 §2.5, §3). Keeping the types apart makes that mistake fail
/// to compile rather than fail on a link.
///
/// The sanctioned shape is: probe across origins with a *tiny* reply
/// (`have` availability, or a manifest), pick one origin, then fetch from
/// that origin's concrete [`V1Context::blob_prefix`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlobProbePrefix(String);

impl BlobProbePrefix {
    /// The `*`-origin prefix for `tier`: `v1/*/@blob/<tier>`.
    #[must_use]
    pub fn new(tier: grammar::BlobTier) -> Self {
        BlobProbePrefix(format!(
            "{}/*/{}/{}",
            grammar::VERSION_CHUNK,
            grammar::PLANE_BLOB,
            tier.chunk()
        ))
    }

    /// The Tier-2 **store** probe: `v1/*/@blob/store/<algo>/have`
    /// (RFC 07 §2.4/§2.5, v1.17).
    ///
    /// The request carries a list of content addresses; each holder answers
    /// a bitfield over exactly that list, so the reply is O(request) and
    /// §3's cost gate is satisfied by construction — which is what makes
    /// the wildcard origin legitimate here. `have` is a reserved Tier-2
    /// token and never a valid content address (§2.4). `algo` is slugged at
    /// the boundary like every generated variable.
    #[must_use]
    pub fn store_have(algo: impl AsRef<str>) -> Self {
        let mut p = Self::new(grammar::BlobTier::Store).0;
        p.push('/');
        p.push_str(crate::key::Chunk::slug(algo).as_str());
        p.push_str("/have");
        BlobProbePrefix(p)
    }

    /// The Tier-2 **tree** probe: `v1/*/@blob/tree/<root>/have`
    /// (RFC 07 §2.4/§2.5, v1.17).
    ///
    /// Each holder answers has-index plus chunks present / total — a flag
    /// and two counters, O(request) whatever the tree's size. The `<root>`
    /// is a validated [`grammar::ContentHash`], so the revoked
    /// caller-chosen name (§2.3) has no spelling here either.
    #[must_use]
    pub fn tree_have(root: &grammar::ContentHash) -> Self {
        let mut p = Self::new(grammar::BlobTier::Tree).0;
        p.push('/');
        p.push_str(root.as_str());
        p.push_str("/have");
        BlobProbePrefix(p)
    }

    /// The prefix as a selector string, for handing to a probing client.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlobProbePrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origin::HostId;

    fn ctx() -> V1Context {
        V1Context::with_origin(
            Origin::Host(HostId::parse("h-3fa9c2d41b7e").unwrap()),
            "sysinfo",
        )
        .unwrap()
    }

    #[test]
    fn key_shapes() {
        let c = ctx();
        assert_eq!(c.telemetry_prefix(), "v1/h-3fa9c2d41b7e/telemetry/sysinfo");
        assert_eq!(c.health_key(), "v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(
            c.evidence_self_key(),
            "v1/h-3fa9c2d41b7e/state/sysinfo/evidence/self"
        );
        assert_eq!(c.alive_key(), "v1/h-3fa9c2d41b7e/state/sysinfo/alive");
        assert_eq!(
            c.device_alive_key("router01"),
            "v1/h-3fa9c2d41b7e/state/sysinfo/device/router01/alive"
        );
        // Foreign device names slug injectively (RFC 03 §2) — never lossy
        // lowercasing ("Router01" and "router01" must not share a key).
        assert_ne!(
            c.device_alive_key("Router01"),
            c.device_alive_key("router01")
        );
        assert_eq!(
            c.rpc_key(&["introspect"]).unwrap(),
            "v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect"
        );
        assert_eq!(
            c.media_video_key("cam0", "h264", "high").unwrap(),
            "v1/h-3fa9c2d41b7e/@media/sysinfo/cam0/video/h264/high"
        );
    }

    /// Application keys are base-relative. The base is the session namespace,
    /// so a context has no way to spell it — which is what makes it impossible
    /// to spell wrong.
    #[test]
    fn keys_are_base_relative() {
        let c = ctx();
        for key in [
            c.telemetry_prefix(),
            c.health_key(),
            c.alive_key(),
            c.rpc_key(&["introspect"]).unwrap(),
            c.media_key(&["cam0", "preview", "jpeg"]).unwrap(),
            c.blob_prefix(grammar::BlobTier::Store),
        ] {
            assert!(
                key.starts_with("v1/"),
                "an application key must start at the version chunk: {key}"
            );
        }
    }

    /// RFC 07 §2.5/§3: the probe form spells the `*`-origin prefix and only
    /// that. It is deliberately not a `Key` and has no conversion into one —
    /// Rust cannot assert a *missing* impl, so this pins the half that can be
    /// asserted and documents the invariant next to what depends on it.
    #[test]
    fn blob_probe_prefix_spells_the_wildcard_origin_form() {
        for (tier, token) in [
            (grammar::BlobTier::Artifact, "artifact"),
            (grammar::BlobTier::Tree, "tree"),
            (grammar::BlobTier::Store, "store"),
        ] {
            let probe = BlobProbePrefix::new(tier);
            assert_eq!(probe.as_str(), format!("v1/*/@blob/{token}"));
            assert_eq!(probe.to_string(), probe.as_str());
        }
        // The concrete counterpart differs exactly at the origin chunk.
        let concrete = ctx().blob_prefix(grammar::BlobTier::Store);
        assert_ne!(
            BlobProbePrefix::new(grammar::BlobTier::Store).as_str(),
            concrete.as_str()
        );
    }

    /// RFC 07 §2.4/§2.5 (v1.17): the Tier-2 probe forms. `have` is a
    /// reserved Tier-2 token — and, deliberately, not a valid content
    /// address, which is what keeps `store/<algo>/<chunk>` parseable
    /// positionally.
    #[test]
    fn blob_probe_prefix_spells_the_tier2_have_forms() {
        assert_eq!(
            BlobProbePrefix::store_have("blake3").as_str(),
            "v1/*/@blob/store/blake3/have"
        );
        let root = grammar::ContentHash::parse("a1b2c3d4e5f60718").unwrap();
        assert_eq!(
            BlobProbePrefix::tree_have(&root).as_str(),
            "v1/*/@blob/tree/a1b2c3d4e5f60718/have"
        );
        // The reserved token itself can never be a content address (§2.4):
        // `have` and `batch` contain non-hex bytes by construction.
        assert!(grammar::ContentHash::parse("have").is_err());
        assert!(grammar::ContentHash::parse("batch").is_err());
    }

    /// The base composes back on for the parties that genuinely see the wire:
    /// router storages, ACL rules, and un-namespaced debug tools (RFC 09 §0/§5).
    /// Multi-chunk bases are legal, and are a *config* value, not an API.
    #[test]
    fn the_base_composes_back_on_for_the_wire_view() {
        let c = ctx();
        assert_eq!(
            grammar::with_base("acme", c.telemetry_prefix()),
            "acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo"
        );
        assert_eq!(
            grammar::with_base("acme/fleet-a", c.telemetry_prefix()),
            "acme/fleet-a/v1/h-3fa9c2d41b7e/telemetry/sysinfo"
        );
        // ...and back off again, losslessly.
        let wire = grammar::with_base("acme/fleet-a", c.telemetry_prefix());
        assert_eq!(
            grammar::strip_base("acme/fleet-a", &wire),
            Some(c.telemetry_prefix().as_str())
        );
    }

    /// `for_producer` mints through the profile — end to end, once.
    #[test]
    fn for_producer_uses_profile_origin() {
        static PROFILE: AppProfile = AppProfile::new(
            crate::AppName::new("zenkey-ctx-test"),
            crate::OriginSalt::new("ctx-test-salt"),
        );
        let a = V1Context::for_producer(&PROFILE, "sysinfo").unwrap();
        let b = V1Context::for_producer(&PROFILE, "netlink").unwrap();
        assert_eq!(a.origin(), b.origin());
        assert!(a.health_key().starts_with("v1/h-"));
    }

    /// Issue #322: a context refuses a producer name it cannot honour rather
    /// than publishing the whole keyspace under a different identity. The
    /// fallback that used to run — slug, then `Producer::new("sensor")` — had
    /// no `Err`, no panic and no log, and collided every misconfigured
    /// producer in the fleet onto one name.
    #[test]
    fn a_bad_producer_name_errs_instead_of_renaming() {
        static PROFILE: AppProfile = AppProfile::new(
            crate::AppName::new("zenkey-ctx-test"),
            crate::OriginSalt::new("ctx-test-salt"),
        );
        for bad in ["has spaces", "Sysinfo", "ipv6-2", "-lead", "", "store"] {
            assert!(
                V1Context::for_producer(&PROFILE, bad).is_err(),
                "{bad:?} must not become a producer identity"
            );
        }
        // Slugging an identity stays available — as a decision the caller
        // writes down, which is the whole difference.
        let slugged = V1Context::with_origin(
            Origin::Host(HostId::parse("h-3fa9c2d41b7e").unwrap()),
            crate::key::Chunk::slug("has spaces").as_str(),
        )
        .unwrap();
        assert_eq!(slugged.producer().name(), "has_x20_spaces");
    }

    /// …and instance 0 has no spelling. It used to be silently ignored, so
    /// `.with_instance(0)` read as a configuration and was a no-op.
    #[test]
    fn instance_numbers_start_at_one() {
        let c = ctx().with_instance(NonZeroU32::new(2).unwrap());
        assert_eq!(c.producer().chunk(), "sysinfo-2");
        assert_eq!(c.health_key(), "v1/h-3fa9c2d41b7e/state/sysinfo-2/health");
        assert!(NonZeroU32::new(0).is_none());
    }

    /// Issue #322: one reserved-token rule, one failure mode. `state_key`
    /// used to `assert!` where `grammar::data_key` returned an `Err` for the
    /// identical input, and the plane builders checked nothing at all.
    #[test]
    fn the_reserved_token_fails_the_same_way_everywhere() {
        let c = ctx();
        let reserved = |e: KeyError| matches!(e, KeyError::ReservedToken(t, _) if t == "alive");
        assert!(reserved(c.state_key(&["alive"]).unwrap_err()));
        assert!(reserved(c.state_key(&["foo", "alive"]).unwrap_err()));
        assert!(reserved(c.evidence_device_key("alive").unwrap_err()));
        assert!(reserved(c.rpc_key(&["alive"]).unwrap_err()));
        assert!(reserved(c.media_key(&["cam0", "alive"]).unwrap_err()));
        // The same input, the same error, one layer down.
        assert!(reserved(
            grammar::data_key(
                c.origin(),
                grammar::Class::State,
                Some(c.producer()),
                &["alive"]
            )
            .unwrap_err()
        ));
        // Liveliness keys are still spellable — through their own builders.
        assert_eq!(c.alive_key(), "v1/h-3fa9c2d41b7e/state/sysinfo/alive");
    }
}
