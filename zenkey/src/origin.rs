//! Host-origin minting: `h-<12hex>` (RFC 06 §1).
//!
//! Byte-precise reference derivation — two independent implementations MUST
//! mint the same id for the same machine:
//!
//! ```text
//! input   = machine_id_hex ++ salt        (UTF-8, no separator)
//! machine_id_hex = the 32 lowercase-hex chars of /etc/machine-id, trimmed
//! origin  = "h-" ++ lowercase_hex(sha256(input))[0..12]
//! ```
//!
//! The salt is an **application constant** (RFC 06 §1), declared in the
//! application's [`crate::AppProfile`] — compiled in, not
//! operator-configurable, identical across deployments. Changing it re-keys
//! every fleet.

use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Write as _;
use std::path::Path;

use crate::grammar::{KeyError, is_valid_host_origin};

/// A validated `h-<12hex>` host origin id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostId(String);

impl HostId {
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        if !is_valid_host_origin(s) {
            return Err(KeyError::InvalidHostOrigin(s.to_string()));
        }
        Ok(HostId(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reference derivation from a machine-id (RFC 06 §1).
    ///
    /// `machine_id` is trimmed of surrounding whitespace/newlines (the
    /// `/etc/machine-id` file ends in a newline) and lowercased before
    /// hashing, so both file forms derive identically.
    pub fn from_machine_id(machine_id: &str, salt: &str) -> Self {
        let normalized = machine_id.trim().to_ascii_lowercase();
        Self::digest(normalized.as_bytes(), salt)
    }

    /// Fallback derivation from the most stable hardware identity available
    /// (primary MAC, serial) — RFC 06 §1.1 option 2. Same function, different
    /// input; the catalog's evidence model absorbs the confidence difference.
    pub fn from_hardware_id(hardware_id: &str, salt: &str) -> Self {
        let normalized = hardware_id.trim().to_ascii_lowercase();
        Self::digest(normalized.as_bytes(), salt)
    }

    fn digest(id_bytes: &[u8], salt: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(id_bytes);
        hasher.update(salt.as_bytes());
        let hex = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        HostId(format!("h-{}", &hex[..12]))
    }

    /// The full minting ladder (RFC 06 §1/§1.1):
    /// 1. `/etc/machine-id` (or the platform equivalent at `machine_id_path`);
    /// 2. a persisted random id at `fallback_path`, created atomically
    ///    (create-exclusive; a racing loser re-reads the winner's file);
    /// 3. as a last resort, a fresh random id persisted best-effort.
    pub fn mint(machine_id_path: &Path, fallback_path: &Path, salt: &str) -> Self {
        if let Ok(machine_id) = std::fs::read_to_string(machine_id_path) {
            let trimmed = machine_id.trim();
            if !trimmed.is_empty() {
                return Self::from_machine_id(trimmed, salt);
            }
        }
        Self::mint_persisted(fallback_path)
    }

    fn mint_persisted(path: &Path) -> Self {
        // Fast path: the file exists and holds a valid id.
        if let Ok(existing) = std::fs::read_to_string(path)
            && let Ok(id) = HostId::parse(existing.trim())
        {
            return id;
        }
        let fresh = Self::random();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Atomic create-exclusive: exactly one racing producer wins; losers
        // re-read the winner's id (RFC 06 §1.1).
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut f) => {
                let _ = f.write_all(fresh.as_str().as_bytes());
                fresh
            }
            Err(_) => match std::fs::read_to_string(path) {
                Ok(existing) => HostId::parse(existing.trim()).unwrap_or(fresh),
                Err(_) => fresh,
            },
        }
    }

    /// 6 random bytes as 12 hex (RFC 06 §1.1 option 1). Not stable on its
    /// own — always persisted by [`Self::mint`].
    fn random() -> Self {
        // No rand dependency: hash process-unique entropy sources. This runs
        // once per host lifetime (then persists), so quality over speed.
        let mut hasher = Sha256::new();
        hasher.update(std::process::id().to_le_bytes());
        if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            hasher.update(now.as_nanos().to_le_bytes());
        }
        if let Ok(hn) = std::env::var("HOSTNAME") {
            hasher.update(hn.as_bytes());
        }
        let hex = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        HostId(format!("h-{}", &hex[..12]))
    }
}

impl fmt::Display for HostId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Typed origins (RFC 08 §1.1, amendments B/G5; hoisted from the tcgui
// reference implementation in v1.5/H1). The kind of an origin is a *type*:
// "I built a key for my own host by accident" is a compile error, and a
// fan-out *write* is unspellable because [`Fleet`] is not a [`ConcreteOrigin`].
// ---------------------------------------------------------------------------

/// This process's own minted origin. Publish/serve builders take exactly this;
/// obtain it from [`crate::AppProfile::local_origin`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocalOrigin(HostId);

impl LocalOrigin {
    /// Wrap an explicitly minted [`HostId`] (tests, or an application that
    /// minted through its own ladder). The type distinction is a discipline,
    /// not a secret: the point is that *call* paths never mint one by
    /// accident, because every other constructor lives on the profile.
    pub fn from_host_id(id: HostId) -> Self {
        LocalOrigin(id)
    }

    /// Derive from an explicit seed with the application's salt (tests,
    /// containers, operator-provided ids).
    pub fn from_seed(seed: &str, salt: &str) -> Self {
        LocalOrigin(HostId::from_machine_id(seed, salt))
    }

    /// The underlying host id.
    pub fn host_id(&self) -> &HostId {
        &self.0
    }
}

/// An origin this process *read from the wire* — a received key, a health
/// document, a catalog entity. Call/address builders take `&impl
/// ConcreteOrigin`; this is the consumer-side implementation (identity
/// bridge, RFC 06 §6: display the payload name, key on the host id).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RemoteOrigin(HostId);

impl RemoteOrigin {
    /// Parse a concrete host origin received over the wire. Rejects
    /// wildcards, service origins, and malformed values — a `RemoteOrigin` is
    /// always one concrete host, which is what keeps a fan-out write
    /// unspellable (G2).
    pub fn parse(s: &str) -> Result<Self, KeyError> {
        HostId::parse(s).map(RemoteOrigin)
    }

    /// Wrap an already-validated [`HostId`] (e.g. from
    /// [`crate::StructuralKey::remote_origin`]).
    pub fn from_host(id: HostId) -> Self {
        RemoteOrigin(id)
    }

    /// The underlying host id.
    pub fn host_id(&self) -> &HostId {
        &self.0
    }
}

/// A registered service origin (`@catalog`, `@desired`, …) — a verbatim
/// chunk, single logical writer (RFC 06 §5, 07 §3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceOrigin(String);

impl ServiceOrigin {
    /// Validate a verbatim service-origin chunk (`@name`).
    pub fn new(chunk: &str) -> Result<Self, KeyError> {
        if crate::grammar::is_valid_verbatim_chunk(chunk) {
            Ok(ServiceOrigin(chunk.to_string()))
        } else {
            Err(KeyError::InvalidVerbatimChunk(chunk.to_string()))
        }
    }

    /// The `@catalog` identity service (RFC 06 §5).
    pub fn catalog() -> Self {
        ServiceOrigin(crate::grammar::SERVICE_CATALOG.to_string())
    }

    /// The origin chunk as it appears in a key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The deliberate `*` — a fleet of hosts. Spellable only where the grammar
/// allows fan-out: selector builders and fanout-allowed procedures. Not a
/// [`ConcreteOrigin`], by design (RFC 08 §1.1: a `*` origin is reachable only
/// by asking for it by name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fleet;

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::LocalOrigin {}
    impl Sealed for super::RemoteOrigin {}
    impl Sealed for super::ServiceOrigin {}
}

/// A **concrete** (never wildcard) origin. Sealed: exactly [`LocalOrigin`],
/// [`RemoteOrigin`], and [`ServiceOrigin`] implement it — a fleet selector
/// does not, which is what makes a fan-out write a type error (G2/G5).
pub trait ConcreteOrigin: sealed::Sealed {
    /// The origin chunk to place in the key.
    fn chunk(&self) -> &str;

    /// The parse-side [`crate::Origin`] equivalent, for interop with the
    /// grammar's structural types.
    fn to_origin(&self) -> crate::grammar::Origin;
}

/// A concrete **host** origin (`h-…`): [`LocalOrigin`] or [`RemoteOrigin`],
/// never a service. Host-shaped builders (producer keys, producer `@rpc`)
/// take this — a service origin has no producer chunk, so passing one would
/// be grammar-illegal; the trait split makes it unrepresentable.
pub trait HostOrigin: ConcreteOrigin {}
impl HostOrigin for LocalOrigin {}
impl HostOrigin for RemoteOrigin {}

impl ConcreteOrigin for LocalOrigin {
    fn chunk(&self) -> &str {
        self.0.as_str()
    }
    fn to_origin(&self) -> crate::grammar::Origin {
        crate::grammar::Origin::Host(self.0.clone())
    }
}

impl ConcreteOrigin for RemoteOrigin {
    fn chunk(&self) -> &str {
        self.0.as_str()
    }
    fn to_origin(&self) -> crate::grammar::Origin {
        crate::grammar::Origin::Host(self.0.clone())
    }
}

impl ConcreteOrigin for ServiceOrigin {
    fn chunk(&self) -> &str {
        &self.0
    }
    fn to_origin(&self) -> crate::grammar::Origin {
        crate::grammar::Origin::Service(self.0.clone())
    }
}

#[cfg(feature = "serde")]
mod serde_impls {
    //! Wire representations (feature `serde`): plain strings, validated on
    //! deserialize. Applications carry origins in health documents (RFC 06
    //! §6 — the payload `host_id` *is* the origin id).
    use super::{HostId, RemoteOrigin};
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    impl Serialize for HostId {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.as_str())
        }
    }

    impl<'de> Deserialize<'de> for HostId {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let raw = String::deserialize(d)?;
            HostId::parse(&raw).map_err(D::Error::custom)
        }
    }

    impl Serialize for RemoteOrigin {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(self.host_id().as_str())
        }
    }

    impl<'de> Deserialize<'de> for RemoteOrigin {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            HostId::deserialize(d).map(RemoteOrigin::from_host)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_origin_rejects_wildcards_services_and_junk() {
        assert!(RemoteOrigin::parse("h-3fa9c2d41b7e").is_ok());
        assert!(RemoteOrigin::parse("*").is_err());
        assert!(RemoteOrigin::parse("@catalog").is_err());
        assert!(RemoteOrigin::parse("lab-router").is_err());
        assert!(RemoteOrigin::parse("h-3fa9c2d41b7").is_err()); // 11 hex
        assert!(RemoteOrigin::parse("h-3FA9C2D41B7E").is_err()); // uppercase
    }

    #[test]
    fn concrete_origin_chunks_and_bridges() {
        let local = LocalOrigin::from_seed("machine-a", "example-salt-v1");
        let remote = RemoteOrigin::parse("h-3fa9c2d41b7e").unwrap();
        let svc = ServiceOrigin::catalog();
        fn chunk_of(o: &impl ConcreteOrigin) -> String {
            o.chunk().to_string()
        }
        assert!(chunk_of(&local).starts_with("h-"));
        assert_eq!(chunk_of(&remote), "h-3fa9c2d41b7e");
        assert_eq!(chunk_of(&svc), "@catalog");
        assert_eq!(
            svc.to_origin(),
            crate::grammar::Origin::Service("@catalog".into())
        );
        // Parse-side bridge: a parsed host key yields a RemoteOrigin.
        let parsed = crate::grammar::parse("v1/h-3fa9c2d41b7e/state/tc/health").unwrap();
        assert_eq!(parsed.remote_origin(), Some(remote));
        let svc_key = crate::grammar::parse("v1/@catalog/state/entity/x").unwrap();
        assert_eq!(svc_key.remote_origin(), None);
    }

    #[test]
    fn service_origin_validates_verbatim() {
        assert!(ServiceOrigin::new("@desired").is_ok());
        assert!(ServiceOrigin::new("desired").is_err());
        assert!(ServiceOrigin::new("@Desired").is_err());
    }

    /// The RFC 06 §1 normative test vector: implementations MUST reproduce it.
    #[test]
    fn rfc_test_vector() {
        let id = HostId::from_machine_id("b642b4217b34b1e8d3bd915fc65c4452", "example-salt-v1");
        assert_eq!(id.as_str(), "h-20609002f7b6");
    }

    #[test]
    fn machine_id_trim_and_case_are_normalized() {
        let a = HostId::from_machine_id("b642b4217b34b1e8d3bd915fc65c4452\n", "s");
        let b = HostId::from_machine_id("  B642B4217B34B1E8D3BD915FC65C4452  ", "s");
        assert_eq!(a, b);
    }

    #[test]
    fn parse_enforces_shape() {
        assert!(HostId::parse("h-20609002f7b6").is_ok());
        assert!(HostId::parse("h_20609002f7b6").is_err()); // legacy separator
        assert!(HostId::parse("h-20609002f7b").is_err()); // 11 hex
        assert!(HostId::parse("h-20609002F7B6").is_err()); // uppercase
    }

    #[test]
    fn persisted_fallback_is_stable_and_atomic() {
        let dir = std::env::temp_dir().join(format!("zsks-test-{}", std::process::id()));
        let path = dir.join("host-id");
        let _ = std::fs::remove_file(&path);
        let first = HostId::mint(Path::new("/nonexistent/machine-id"), &path, "s");
        let second = HostId::mint(Path::new("/nonexistent/machine-id"), &path, "s");
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
