//! What a deployment configures for a service, never its code (spec §1.5,
//! §3.2 R1, §8.1).
//!
//! The format is a recommendation, not a rule (R1): this is the shape the
//! runtime reads, and it deserializes from any serde format.
//!
//! **The address** is literal (`vehicle-01/nav`) or asks for a system
//! minted by `hostid.v1` (`@hostid.v1/sysinfo`,
//! `spec/profiles/hostid/v1.md` §2.3). Either way it is resolved into one
//! [`Addr`] when the service starts, and so is a binding's
//! `self.system/<service>` (R1, 0.20). Resolving reads the host, so it is
//! the runtime's ([`crate::hostid`]); this module only parses, and builds
//! without the `zenoh` feature.
//!
//! ```toml
//! address = "@hostid.v1/sysinfo"   # the system is minted by hostid.v1
//! hostid  = { ephemeral = true }   # optional, false by default (§2.6)
//!
//! [bindings.logs]
//! providers = ["self.system/journal"]   # resolved at start (R1, 0.20)
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use zenkey_model::grammar::{Addr, IfaceId, Name};

/// The spelling of the system position that asks for a system minted by
/// `hostid.v1` (`hostid.v1` §2.3).
pub const HOSTID_SYSTEM: &str = "@hostid.v1";

/// The spelling of the system position of a binding's provider that names
/// the service's own system (R1, 0.20), and of a parameter bound to it
/// (R2).
pub const SELF_SYSTEM: &str = "self.system";

/// A service's address as the deployment configures it (spec §1.5;
/// `hostid.v1` §2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// `<system>/<service>`: the deployment names the system. A name in the
    /// minted shape (`h-3fa9c2d41b7e/nav`) is literal too: nothing is
    /// minted, and the descriptor does not list `hostid.v1` (§2.3, §2.8).
    Literal(Addr),
    /// `@hostid.v1/<service>`: the system is minted by `hostid.v1` from the
    /// host's machine id, once per run (§2.4–§2.7).
    HostId {
        service: Name,
        /// `hostid.ephemeral` (§2.6): allow the ephemeral rung, when every
        /// input gave no id and the shared file was not created.
        ephemeral: bool,
    },
}

/// Why an address does not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddressError {
    /// Not `<system>/<service>` with two plain chunks.
    #[error(transparent)]
    Key(#[from] zenkey_model::grammar::KeyError),
    /// A system position starting with `@` other than `@hostid.v1`
    /// (`hostid.v1` §2.3): no literal system is spelled so, and no other
    /// derivation is known.
    #[error(
        "{0:?} at the system position is not a system this runtime can mint: \
         only `@hostid.v1` is (hostid.v1 §2.3), and a literal system is a plain chunk"
    )]
    UnknownDerivation(String),
}

impl Address {
    /// Whether this address asks for a minted system (§2.3).
    #[must_use]
    pub fn is_minted(&self) -> bool {
        matches!(self, Self::HostId { .. })
    }

    /// The service position, which the deployment always names.
    #[must_use]
    pub fn service(&self) -> &Name {
        match self {
            Self::Literal(a) => &a.service,
            Self::HostId { service, .. } => service,
        }
    }
}

impl FromStr for Address {
    type Err = AddressError;

    /// Parses `<system>/<service>` or `@hostid.v1/<service>`. The second
    /// is not ephemeral: that is the configuration's `hostid` table.
    fn from_str(s: &str) -> Result<Self, AddressError> {
        match s.split_once('/') {
            Some((HOSTID_SYSTEM, svc)) => Ok(Self::HostId {
                service: Name::new("service", svc)?,
                ephemeral: false,
            }),
            Some((sys, _)) if sys.starts_with('@') => {
                Err(AddressError::UnknownDerivation(sys.to_owned()))
            }
            _ => Ok(Self::Literal(s.parse()?)),
        }
    }
}

impl fmt::Display for Address {
    /// As configured: `ephemeral` is not part of the spelling.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(a) => a.fmt(f),
            Self::HostId { service, .. } => write!(f, "{HOSTID_SYSTEM}/{service}"),
        }
    }
}

impl From<Addr> for Address {
    fn from(a: Addr) -> Self {
        Self::Literal(a)
    }
}

/// A service's deployment configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "RawConfig")]
pub struct ServiceConfig {
    /// `<system>/<service>` (spec §1.5): the deployment chooses both, or
    /// asks `hostid.v1` to mint the system (`@hostid.v1/<service>`). Its
    /// serialized form is two members: `address`, and `hostid` for the
    /// minted form's options.
    pub address: Address,
    /// Interfaces for which this service holds no interface token (§8.1,
    /// U22). Their descriptor entries say `"token": false`.
    pub tokenless: BTreeSet<IfaceId>,
    /// The capabilities this instance holds (§3.3). A resource gated on one
    /// it does not hold is not exposed, and need not be listed unavailable.
    pub capabilities: BTreeSet<String>,
    /// Role bindings, by role name (R1, R2).
    pub bindings: BTreeMap<String, Binding>,
    /// The tombstone window, in seconds (S3): 60 unless set. For state an
    /// archive records, at least the longest outage expected between them.
    pub tombstone_window_s: Option<u64>,
    /// A router-stamped key this owner watches to detect its own clock
    /// running ahead (§4.3, "Ahead"). Unset: no detection.
    pub clock_reference: Option<String>,
    /// Facts about this run that live in the descriptor only (§1.5): host,
    /// build, and anything else the deployment records.
    pub meta: BTreeMap<String, serde_json::Value>,
}

/// The providers a role is bound to (R1), and its parameter bindings (R2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// Service addresses, exact (`vehicle-01/teleop`) or wildcard
    /// (`vehicle-01/*`), or on the service's own system
    /// (`self.system/teleop`, `self.system/*`; R1, 0.20).
    #[serde(default)]
    pub providers: Vec<String>,
    /// Template parameters bound for this role, such as
    /// `vehicle = "self.system"`.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

impl Binding {
    /// The providers with `self.system` spelled out as `system` (R1,
    /// 0.20): `self.system/<service>` becomes `<system>/<service>`, and
    /// `self.system/*` every service on `system` that implements the
    /// interface, as `<system>/*` would. Any other provider is kept as
    /// written, for the checks that read it (R1, D009). The parameters keep
    /// their spelling: R3 lists them as configured.
    #[must_use]
    pub fn resolved(&self, system: &Name) -> Self {
        let providers = self
            .providers
            .iter()
            .map(|p| match p.split_once('/') {
                Some((SELF_SYSTEM, svc)) => format!("{system}/{svc}"),
                _ => p.clone(),
            })
            .collect();
        Self {
            providers,
            params: self.params.clone(),
        }
    }
}

/// `hostid`: the options of a minted address (`hostid.v1` §2.3).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HostIdTable {
    #[serde(default)]
    ephemeral: bool,
}

/// The configuration as it is written: `address` a string, and `hostid`
/// beside it (§2.3).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    address: String,
    #[serde(default)]
    hostid: Option<HostIdTable>,
    #[serde(default)]
    tokenless: BTreeSet<IfaceId>,
    #[serde(default)]
    capabilities: BTreeSet<String>,
    #[serde(default)]
    bindings: BTreeMap<String, Binding>,
    #[serde(default)]
    tombstone_window_s: Option<u64>,
    #[serde(default)]
    clock_reference: Option<String>,
    #[serde(default)]
    meta: BTreeMap<String, serde_json::Value>,
}

impl TryFrom<RawConfig> for ServiceConfig {
    type Error = String;

    fn try_from(raw: RawConfig) -> Result<Self, String> {
        let address = match (raw.address.parse::<Address>(), raw.hostid) {
            (Err(e), _) => return Err(e.to_string()),
            (Ok(Address::HostId { service, .. }), table) => Address::HostId {
                service,
                ephemeral: table.unwrap_or_default().ephemeral,
            },
            (Ok(Address::Literal(a)), Some(_)) => {
                return Err(format!(
                    "`hostid` is set, but the address {a} does not ask for a minted system: \
                     write `{HOSTID_SYSTEM}/{}`, or remove `hostid` (hostid.v1 §2.3)",
                    a.service
                ));
            }
            (Ok(literal), None) => literal,
        };
        Ok(Self {
            address,
            tokenless: raw.tokenless,
            capabilities: raw.capabilities,
            bindings: raw.bindings,
            tombstone_window_s: raw.tombstone_window_s,
            clock_reference: raw.clock_reference,
            meta: raw.meta,
        })
    }
}

impl ServiceConfig {
    /// A configuration with a literal address and nothing else.
    #[must_use]
    pub fn new(address: Addr) -> Self {
        Self::at(Address::Literal(address))
    }

    /// A configuration with `address`, literal or minted, and nothing else.
    #[must_use]
    pub fn at(address: Address) -> Self {
        Self {
            address,
            tokenless: BTreeSet::new(),
            capabilities: BTreeSet::new(),
            bindings: BTreeMap::new(),
            tombstone_window_s: None,
            clock_reference: None,
            meta: BTreeMap::new(),
        }
    }

    /// A configuration whose system is minted by `hostid.v1`
    /// (`@hostid.v1/<service>`, §2.3), failing closed when the host has
    /// no id (§2.6).
    #[must_use]
    pub fn minted(service: Name) -> Self {
        Self::at(Address::HostId {
            service,
            ephemeral: false,
        })
    }

    /// As [`ServiceConfig::minted`], with `hostid.ephemeral = true`: the
    /// ephemeral rung is allowed (§2.6).
    #[must_use]
    pub fn minted_ephemeral(service: Name) -> Self {
        Self::at(Address::HostId {
            service,
            ephemeral: true,
        })
    }

    /// Adds `iface` to the tokenless set.
    #[must_use]
    pub fn tokenless(mut self, iface: IfaceId) -> Self {
        self.tokenless.insert(iface);
        self
    }

    /// Adds a held capability.
    #[must_use]
    pub fn capability(mut self, name: &str) -> Self {
        self.capabilities.insert(name.to_owned());
        self
    }

    /// Binds `role` to `providers`.
    #[must_use]
    pub fn bind(mut self, role: &str, providers: &[&str]) -> Self {
        self.bindings.entry(role.to_owned()).or_default().providers =
            providers.iter().map(|p| (*p).to_owned()).collect();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{Address, Binding, ServiceConfig};
    use zenkey_model::grammar::Name;

    #[test]
    fn a_configuration_holds_only_what_a_key_would_accept() {
        let c: ServiceConfig = serde_json::from_str(
            r#"{"address": "vehicle-01/nav", "tokenless": ["health.v1"],
                "capabilities": ["imu"],
                "bindings": {"cmd": {"providers": ["vehicle-01/*"],
                                     "params": {"vehicle": "self.system"}}}}"#,
        )
        .unwrap();
        assert_eq!(c.address.to_string(), "vehicle-01/nav");
        assert!(!c.address.is_minted());
        assert!(c.tokenless.contains(&"health.v1".parse().unwrap()));
        for bad in [
            r#"{"address": "Vehicle/nav"}"#,
            r#"{"address": "vehicle-01"}"#,
            r#"{"address": "v/n", "tokenless": ["health"]}"#,
            r#"{"address": "v/n", "unknown": 1}"#,
            r#"{"address": "v/n", "bindings": {"cmd": {"provider": []}}}"#,
        ] {
            assert!(serde_json::from_str::<ServiceConfig>(bad).is_err(), "{bad}");
        }
    }

    /// `hostid.v1` §2.3: the recommended spelling, in TOML as the profile
    /// writes it.
    #[test]
    fn a_minted_address_and_its_options() {
        let c: ServiceConfig = toml::from_str(
            r#"
            address = "@hostid.v1/sysinfo"
            hostid  = { ephemeral = true }
            "#,
        )
        .unwrap();
        assert_eq!(
            c.address,
            Address::HostId {
                service: Name::new("service", "sysinfo").unwrap(),
                ephemeral: true
            }
        );
        assert_eq!(c.address.to_string(), "@hostid.v1/sysinfo");
        assert!(c.address.is_minted());

        // `hostid` is optional, and `ephemeral` false by default.
        for text in [
            r#"address = "@hostid.v1/sysinfo""#,
            "address = \"@hostid.v1/sysinfo\"\nhostid = {}",
            "address = \"@hostid.v1/sysinfo\"\nhostid = { ephemeral = false }",
        ] {
            let c: ServiceConfig = toml::from_str(text).unwrap();
            assert_eq!(
                c.address,
                ServiceConfig::minted(Name::new("service", "sysinfo").unwrap()).address,
                "{text}"
            );
        }

        // A literal name in the minted shape is literal (§2.3).
        let c: ServiceConfig = toml::from_str(r#"address = "h-3fa9c2d41b7e/nav""#).unwrap();
        assert_eq!(
            c.address,
            Address::Literal("h-3fa9c2d41b7e/nav".parse().unwrap())
        );
    }

    /// `hostid.v1` §2.3's configuration errors, each refused when it is
    /// read: another `@` system, `hostid` beside a literal address, and
    /// anything but `ephemeral` in it.
    #[test]
    fn the_minted_forms_configuration_errors() {
        for (text, says) in [
            (r#"address = "@hostid.v2/sysinfo""#, "@hostid.v2"),
            (r#"address = "@HOSTID.v1/sysinfo""#, "@HOSTID.v1"),
            (r#"address = "@/sysinfo""#, "\"@\""),
            (r#"address = "@hostid.v1/Sysinfo""#, "Sysinfo"),
            (r#"address = "@hostid.v1""#, "address"),
            (
                "address = \"vehicle-01/sysinfo\"\nhostid = { ephemeral = true }",
                "does not ask for a minted system",
            ),
            (
                "address = \"h-3fa9c2d41b7e/sysinfo\"\nhostid = {}",
                "does not ask for a minted system",
            ),
            (
                "address = \"@hostid.v1/sysinfo\"\nhostid = { salt = \"x\" }",
                "salt",
            ),
            ("address = \"@hostid.v1/sysinfo\"\nsalt = \"x\"", "salt"),
        ] {
            let e = toml::from_str::<ServiceConfig>(text)
                .unwrap_err()
                .to_string();
            assert!(e.contains(says), "{text}: {e}");
        }
        assert!("@hostid.v2/x".parse::<Address>().is_err());
        assert!("@hostid.v1/x".parse::<Address>().is_ok());
    }

    /// R1 (0.20): `self.system` at a provider's system position is spelled
    /// out, `*` included; anything else is kept for the checks.
    #[test]
    fn a_binding_on_the_services_own_system() {
        let b = Binding {
            providers: vec![
                "self.system/journal".into(),
                "self.system/*".into(),
                "ground/fleet-mgr".into(),
                "*/self.system".into(),
            ],
            params: [("vehicle".to_owned(), "self.system".to_owned())].into(),
        };
        let r = b.resolved(&Name::new("system", "h-bbd1aa1db10b").unwrap());
        assert_eq!(
            r.providers,
            [
                "h-bbd1aa1db10b/journal",
                "h-bbd1aa1db10b/*",
                "ground/fleet-mgr",
                "*/self.system"
            ]
        );
        assert_eq!(r.params, b.params, "R3 lists parameters as configured");
    }
}
