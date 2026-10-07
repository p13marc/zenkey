//! The zk2 key grammar (r3 §3.1, r3.3 D3).
//!
//! ```text
//! zk2/<system>/<service>/<iface>.v<major>/<kind>/<resource…>              data
//! zk2/<system>/<service>/@zk/instance/<instance>                          instance token + descriptor
//! zk2/<system>/<service>/@zk/alive/<iface>.v<major>/<instance>/<fp16>     interface token
//! zk2/<system>/<service>/@zk/member/<iface>.v<major>/<member>/<epoch>     member token (r3.3 D9b)
//! zk2/@zk/contract/<iface>.v<major>/<sha256>                              contract bundle
//! ```
//!
//! Positions 1–5 have fixed arity, so `zk2/*/*/*/state/**` is exact. Every
//! key here is *base-relative*: a deployment's optional namespace is added
//! by the Zenoh session, never spelled by code.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;
use zenoh_keyexpr::OwnedKeyExpr;

use crate::chunk::{is_ident, is_lower_hex, is_plain_chunk};

/// The grammar major: the first chunk of every zk2 key. A plain chunk,
/// never verbatim (v1's `@v1` broke zenoh-ext's `@adv` parsing).
pub const GRAMMAR: &str = "zk2";
/// The control token, verbatim, at position 4 (per service) or 2 (contracts).
pub const CONTROL: &str = "@zk";

/// Why a key, or one of its parts, was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum KeyError {
    #[error("{what} {value:?} is not a plain chunk ([a-z0-9]([a-z0-9._-]*[a-z0-9])?)")]
    NotPlain { what: &'static str, value: String },
    #[error("interface {0:?} is not <name>.v<major> with name segments [a-z][a-z0-9_]*")]
    Interface(String),
    #[error("{what} {value:?} must be {len} lowercase hex digits")]
    Hex {
        what: &'static str,
        value: String,
        len: usize,
    },
    #[error("unknown kind token {0:?} (stream, @stream, state, @state, events, @op)")]
    Kind(String),
    #[error("{0:?} is not a zk2 key: {1}")]
    Shape(String, &'static str),
    #[error("{0:?} is not a valid Zenoh key expression: {1}")]
    KeyExpr(String, String),
}

/// A system or service name: one plain chunk, chosen by deployment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(String);

impl Name {
    pub fn new(what: &'static str, s: &str) -> Result<Self, KeyError> {
        if is_plain_chunk(s) {
            Ok(Self(s.to_owned()))
        } else {
            Err(KeyError::NotPlain {
                what,
                value: s.to_owned(),
            })
        }
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A service address, `<system>/<service>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Addr {
    pub system: Name,
    pub service: Name,
}

impl Addr {
    pub fn new(system: &str, service: &str) -> Result<Self, KeyError> {
        Ok(Self {
            system: Name::new("system", system)?,
            service: Name::new("service", service)?,
        })
    }
}

impl FromStr for Addr {
    type Err = KeyError;
    /// Parses `<system>/<service>`.
    fn from_str(s: &str) -> Result<Self, KeyError> {
        let (sys, svc) = s.split_once('/').ok_or(KeyError::Shape(
            s.to_owned(),
            "an address is <system>/<service>",
        ))?;
        Self::new(sys, svc)
    }
}

impl fmt::Display for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.system, self.service)
    }
}

/// An interface identity: a dotted name plus a major, spelled
/// `<name>.v<major>` in a key (`nav.v2`, `tc.netem.v1`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IfaceId {
    name: String,
    major: u32,
}

impl IfaceId {
    /// `name` is one or more `[a-z][a-z0-9_]*` segments joined by `.`, and
    /// may not itself end in `.v<digits>` (that would make the chunk
    /// ambiguous).
    pub fn new(name: &str, major: u32) -> Result<Self, KeyError> {
        let segments_ok = !name.is_empty() && name.split('.').all(is_ident);
        let ends_like_major = name
            .rsplit_once('.')
            .is_some_and(|(_, last)| is_major_segment(last));
        if segments_ok && !ends_like_major && !is_major_segment(name) {
            Ok(Self {
                name: name.to_owned(),
                major,
            })
        } else {
            Err(KeyError::Interface(name.to_owned()))
        }
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn major(&self) -> u32 {
        self.major
    }
}

fn is_major_segment(s: &str) -> bool {
    s.strip_prefix('v')
        .is_some_and(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
}

impl FromStr for IfaceId {
    type Err = KeyError;
    /// Parses the key chunk `<name>.v<major>`.
    fn from_str(s: &str) -> Result<Self, KeyError> {
        let err = || KeyError::Interface(s.to_owned());
        let (name, major) = s.rsplit_once(".v").ok_or_else(err)?;
        if major.is_empty() || !major.bytes().all(|c| c.is_ascii_digit()) {
            return Err(err());
        }
        // No leading zeros: one spelling per major.
        if major.len() > 1 && major.starts_with('0') {
            return Err(err());
        }
        let major: u32 = major.parse().map_err(|_| err())?;
        Self::new(name, major).map_err(|_| err())
    }
}

impl fmt::Display for IfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.v{}", self.name, self.major)
    }
}

/// The kind token at position 5 (r3.3 D3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KindToken {
    /// `stream`: an ambient stream (rides `zk2/<system>/**`).
    Stream,
    /// `@stream`: an explicit-only stream (high rate, large).
    ExplicitStream,
    /// `state`: latest value, answerable on GET.
    State,
    /// `@state`: explicit-only state (large populations).
    ExplicitState,
    /// `events`: one key per occurrence, ending in a ULID.
    Events,
    /// `@op`: request/reply.
    Op,
}

impl KindToken {
    pub const ALL: [KindToken; 6] = [
        Self::Stream,
        Self::ExplicitStream,
        Self::State,
        Self::ExplicitState,
        Self::Events,
        Self::Op,
    ];
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stream => "stream",
            Self::ExplicitStream => "@stream",
            Self::State => "state",
            Self::ExplicitState => "@state",
            Self::Events => "events",
            Self::Op => "@op",
        }
    }
    /// Verbatim tokens are reached by no `*`/`**`.
    #[must_use]
    pub fn is_verbatim(self) -> bool {
        self.as_str().starts_with('@')
    }
}

impl FromStr for KindToken {
    type Err = KeyError;
    fn from_str(s: &str) -> Result<Self, KeyError> {
        Self::ALL
            .into_iter()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| KeyError::Kind(s.to_owned()))
    }
}

impl fmt::Display for KindToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

macro_rules! hex_id {
    ($(#[$m:meta])* $t:ident, $what:literal, $len:literal) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $t(String);
        impl $t {
            pub fn new(s: &str) -> Result<Self, KeyError> {
                if is_lower_hex(s, $len) {
                    Ok(Self(s.to_owned()))
                } else {
                    Err(KeyError::Hex { what: $what, value: s.to_owned(), len: $len })
                }
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl FromStr for $t {
            type Err = KeyError;
            fn from_str(s: &str) -> Result<Self, KeyError> {
                Self::new(s)
            }
        }
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

hex_id!(
    /// One run of a service, and its continuity epoch (r3.2 §3.5): 64 random
    /// bits as 16 lowercase hex digits, re-minted whenever counters reset.
    InstanceId,
    "instance id",
    16
);
hex_id!(
    /// The first 64 bits of a contract fingerprint, in presence tokens.
    Fp16,
    "fingerprint prefix",
    16
);
hex_id!(
    /// A full sha256, 64 lowercase hex digits (a contract fingerprint without
    /// its `sha256:` prefix).
    Sha256Hex,
    "sha256",
    64
);

impl InstanceId {
    #[must_use]
    pub fn from_u64(v: u64) -> Self {
        Self(format!("{v:016x}"))
    }
}

impl Sha256Hex {
    /// The 64-bit prefix used in presence tokens.
    #[must_use]
    pub fn fp16(&self) -> Fp16 {
        Fp16(self.0[..16].to_owned())
    }
}

/// A validated, canonical, base-relative zk2 key, convertible to the
/// Zenoh key-expression type without re-parsing.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key(OwnedKeyExpr);

impl Key {
    fn from_chunks(chunks: &[&str]) -> Result<Self, KeyError> {
        let s = chunks.join("/");
        OwnedKeyExpr::try_from(s.clone())
            .map(Self)
            .map_err(|e| KeyError::KeyExpr(s, e.to_string()))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    #[must_use]
    pub fn into_keyexpr(self) -> OwnedKeyExpr {
        self.0
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A data key: `zk2/<system>/<service>/<iface>/<kind>/<resource…>`.
/// `resource` chunks must already be plain chunks (template building slugs
/// parameter values, see [`crate::template`]).
pub fn data_key(
    addr: &Addr,
    iface: &IfaceId,
    kind: KindToken,
    resource: &[&str],
) -> Result<Key, KeyError> {
    if resource.is_empty() {
        return Err(KeyError::Shape(
            String::new(),
            "a data key needs a resource path",
        ));
    }
    for c in resource {
        if !is_plain_chunk(c) {
            return Err(KeyError::NotPlain {
                what: "resource chunk",
                value: (*c).to_owned(),
            });
        }
    }
    let iface = iface.to_string();
    let mut chunks = vec![
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        &iface,
        kind.as_str(),
    ];
    chunks.extend_from_slice(resource);
    Key::from_chunks(&chunks)
}

/// The instance token's key, which is also the descriptor's key.
pub fn instance_key(addr: &Addr, instance: &InstanceId) -> Result<Key, KeyError> {
    Key::from_chunks(&[
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        CONTROL,
        "instance",
        instance.as_str(),
    ])
}

/// An interface token: `…/@zk/alive/<iface>/<instance>/<fp16>`.
pub fn alive_key(
    addr: &Addr,
    iface: &IfaceId,
    instance: &InstanceId,
    fp: &Fp16,
) -> Result<Key, KeyError> {
    let iface = iface.to_string();
    Key::from_chunks(&[
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        CONTROL,
        "alive",
        &iface,
        instance.as_str(),
        fp.as_str(),
    ])
}

/// A member token (r3.3 D9b): `…/@zk/member/<iface>/<member>/<epoch>`.
/// `member` is the slugged value of the template's `epoch` parameter.
pub fn member_key(
    addr: &Addr,
    iface: &IfaceId,
    member: &str,
    epoch: &InstanceId,
) -> Result<Key, KeyError> {
    if !is_plain_chunk(member) {
        return Err(KeyError::NotPlain {
            what: "member",
            value: member.to_owned(),
        });
    }
    let iface = iface.to_string();
    Key::from_chunks(&[
        GRAMMAR,
        addr.system.as_str(),
        addr.service.as_str(),
        CONTROL,
        "member",
        &iface,
        member,
        epoch.as_str(),
    ])
}

/// A contract bundle's location-free key: `zk2/@zk/contract/<iface>/<sha256>`.
pub fn contract_key(iface: &IfaceId, fingerprint: &Sha256Hex) -> Result<Key, KeyError> {
    let iface = iface.to_string();
    Key::from_chunks(&[GRAMMAR, CONTROL, "contract", &iface, fingerprint.as_str()])
}

/// A parsed zk2 key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZkKey {
    Data {
        addr: Addr,
        iface: IfaceId,
        kind: KindToken,
        resource: Vec<String>,
    },
    Instance {
        addr: Addr,
        instance: InstanceId,
    },
    Alive {
        addr: Addr,
        iface: IfaceId,
        instance: InstanceId,
        fp: Fp16,
    },
    Member {
        addr: Addr,
        iface: IfaceId,
        member: String,
        epoch: InstanceId,
    },
    Contract {
        iface: IfaceId,
        fingerprint: Sha256Hex,
    },
}

impl ZkKey {
    /// Builds the key back, which must give the parsed string exactly.
    pub fn to_key(&self) -> Result<Key, KeyError> {
        match self {
            Self::Data {
                addr,
                iface,
                kind,
                resource,
            } => {
                let r: Vec<&str> = resource.iter().map(String::as_str).collect();
                data_key(addr, iface, *kind, &r)
            }
            Self::Instance { addr, instance } => instance_key(addr, instance),
            Self::Alive {
                addr,
                iface,
                instance,
                fp,
            } => alive_key(addr, iface, instance, fp),
            Self::Member {
                addr,
                iface,
                member,
                epoch,
            } => member_key(addr, iface, member, epoch),
            Self::Contract { iface, fingerprint } => contract_key(iface, fingerprint),
        }
    }
}

/// Parses a base-relative key. Anything not in the grammar is refused with
/// the reason; a key outside `zk2/` is not an error of the key, but it is
/// not a zk2 key either.
pub fn parse(key: &str) -> Result<ZkKey, KeyError> {
    let shape = |why| KeyError::Shape(key.to_owned(), why);
    let c: Vec<&str> = key.split('/').collect();
    if c.first() != Some(&GRAMMAR) {
        return Err(shape("it does not start with zk2/"));
    }
    if c.get(1) == Some(&CONTROL) {
        return match c.as_slice() {
            [_, _, "contract", iface, sha] => Ok(ZkKey::Contract {
                iface: iface.parse()?,
                fingerprint: sha.parse()?,
            }),
            _ => Err(shape("zk2/@zk/ holds only contract/<iface>/<sha256>")),
        };
    }
    if c.len() < 6 {
        return Err(shape("too few chunks"));
    }
    let addr = Addr::new(c[1], c[2])?;
    if c[3] == CONTROL {
        return match &c[4..] {
            ["instance", inst] => Ok(ZkKey::Instance {
                addr,
                instance: inst.parse()?,
            }),
            ["alive", iface, inst, fp] => Ok(ZkKey::Alive {
                addr,
                iface: iface.parse()?,
                instance: inst.parse()?,
                fp: fp.parse()?,
            }),
            ["member", iface, member, epoch] => {
                if !is_plain_chunk(member) {
                    return Err(KeyError::NotPlain {
                        what: "member",
                        value: (*member).to_owned(),
                    });
                }
                Ok(ZkKey::Member {
                    addr,
                    iface: iface.parse()?,
                    member: (*member).to_owned(),
                    epoch: epoch.parse()?,
                })
            }
            _ => Err(shape("unknown @zk control key")),
        };
    }
    let iface: IfaceId = c[3].parse()?;
    let kind: KindToken = c[4].parse()?;
    let resource: Vec<String> = c[5..].iter().map(|s| (*s).to_owned()).collect();
    for r in &resource {
        if !is_plain_chunk(r) {
            return Err(KeyError::NotPlain {
                what: "resource chunk",
                value: r.clone(),
            });
        }
    }
    if kind == KindToken::Events
        && !resource
            .last()
            .is_some_and(|l| crate::chunk::is_lower_ulid(l))
    {
        return Err(shape("an events key ends in a lowercase ULID"));
    }
    Ok(ZkKey::Data {
        addr,
        iface,
        kind,
        resource,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> Addr {
        Addr::new("vehicle-01", "navigation").unwrap()
    }

    #[test]
    fn iface_chunks() {
        let n: IfaceId = "nav.v2".parse().unwrap();
        assert_eq!((n.name(), n.major()), ("nav", 2));
        let t: IfaceId = "tc.netem.v1".parse().unwrap();
        assert_eq!((t.name(), t.major()), ("tc.netem", 1));
        for bad in [
            "nav",
            "nav.v",
            "nav.vx",
            "Nav.v2",
            "nav.v02",
            ".v2",
            "nav..x.v1",
            "a.v2.v3",
            "v2.v3",
        ] {
            assert!(bad.parse::<IfaceId>().is_err(), "{bad}");
        }
        assert!(
            IfaceId::new("acme.v2", 1).is_err(),
            "a name may not end in .v<int>"
        );
    }

    #[test]
    fn data_keys_round_trip() {
        let k = data_key(
            &addr(),
            &"nav.v2".parse().unwrap(),
            KindToken::Stream,
            &["position"],
        )
        .unwrap();
        assert_eq!(
            k.as_str(),
            "zk2/vehicle-01/navigation/nav.v2/stream/position"
        );
        let p = parse(k.as_str()).unwrap();
        assert_eq!(p.to_key().unwrap(), k);
        let op = data_key(
            &addr(),
            &"nav.v2".parse().unwrap(),
            KindToken::Op,
            &["set_origin"],
        )
        .unwrap();
        assert_eq!(
            op.as_str(),
            "zk2/vehicle-01/navigation/nav.v2/@op/set_origin"
        );
        assert_eq!(parse(op.as_str()).unwrap().to_key().unwrap(), op);
    }

    #[test]
    fn control_keys_round_trip() {
        let i = InstanceId::from_u64(0x8f3a_5c2e_9b1d_4f70);
        let sha = Sha256Hex::new(&"3fa9c2d41b7e9a01".repeat(4)).unwrap();
        let iface: IfaceId = "nav.v2".parse().unwrap();
        for k in [
            instance_key(&addr(), &i).unwrap(),
            alive_key(&addr(), &iface, &i, &sha.fp16()).unwrap(),
            member_key(&addr(), &iface, "rf0", &i).unwrap(),
            contract_key(&iface, &sha).unwrap(),
        ] {
            assert_eq!(parse(k.as_str()).unwrap().to_key().unwrap(), k, "{k}");
        }
        assert_eq!(
            alive_key(&addr(), &iface, &i, &sha.fp16())
                .unwrap()
                .as_str(),
            "zk2/vehicle-01/navigation/@zk/alive/nav.v2/8f3a5c2e9b1d4f70/3fa9c2d41b7e9a01"
        );
    }

    #[test]
    fn refusals() {
        for bad in [
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            "zk2/vehicle-01/navigation/nav.v2/telemetry/x",
            "zk2/Vehicle/navigation/nav.v2/stream/x",
            "zk2/vehicle-01/navigation/nav.v2/stream",
            "zk2/vehicle-01/navigation/nav.v2/events/applied/not-a-ulid",
            "zk2/@zk/contract/nav.v2/abc",
            "zk2/vehicle-01/navigation/@zk/instance/XYZ",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn verbatim_tokens() {
        assert!(KindToken::ExplicitStream.is_verbatim());
        assert!(KindToken::ExplicitState.is_verbatim());
        assert!(KindToken::Op.is_verbatim());
        assert!(!KindToken::Events.is_verbatim());
        assert!(!KindToken::Stream.is_verbatim());
    }
}
