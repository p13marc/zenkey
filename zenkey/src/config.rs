//! What a deployment configures for a service, never its code (spec §1.5,
//! §3.2 R1, §8.1).
//!
//! The format is a recommendation, not a rule (R1): this is the shape the
//! runtime reads, and it deserializes from any serde format.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use zenkey_model::grammar::{Addr, IfaceId};

/// A service's deployment configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// `<system>/<service>` (spec §1.5): the deployment chooses both.
    pub address: Addr,
    /// Interfaces for which this service holds no interface token (§8.1,
    /// U22). Their descriptor entries say `"token": false`.
    #[serde(default)]
    pub tokenless: BTreeSet<IfaceId>,
    /// The capabilities this instance holds (§3.3). A resource gated on one
    /// it does not hold is not exposed, and need not be listed unavailable.
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
    /// Role bindings, by role name (R1, R2).
    #[serde(default)]
    pub bindings: BTreeMap<String, Binding>,
    /// The tombstone window, in seconds (S3): 60 unless set. For state an
    /// archive records, at least the longest outage expected between them.
    #[serde(default)]
    pub tombstone_window_s: Option<u64>,
    /// A router-stamped key this owner watches to detect its own clock
    /// running ahead (§4.3, "Ahead"). Unset: no detection.
    #[serde(default)]
    pub clock_reference: Option<String>,
    /// Facts about this run that live in the descriptor only (§1.5): host,
    /// build, and anything else the deployment records.
    #[serde(default)]
    pub meta: BTreeMap<String, serde_json::Value>,
}

/// The providers a role is bound to (R1), and its parameter bindings (R2).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// Service addresses, exact (`vehicle-01/teleop`) or wildcard
    /// (`vehicle-01/*`).
    #[serde(default)]
    pub providers: Vec<String>,
    /// Template parameters bound for this role, such as
    /// `vehicle = "self.system"`.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
}

impl ServiceConfig {
    /// A configuration with an address and nothing else.
    #[must_use]
    pub fn new(address: Addr) -> Self {
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
    use super::ServiceConfig;

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
        assert!(c.tokenless.contains(&"health.v1".parse().unwrap()));
        for bad in [
            r#"{"address": "Vehicle/nav"}"#,
            r#"{"address": "vehicle-01"}"#,
            r#"{"address": "v/n", "tokenless": ["health"]}"#,
            r#"{"address": "v/n", "unknown": 1}"#,
        ] {
            assert!(serde_json::from_str::<ServiceConfig>(bad).is_err(), "{bad}");
        }
    }
}
