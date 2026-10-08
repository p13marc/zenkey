//! Building the descriptor from what the service declared (spec §3.3),
//! never from observed traffic.

use std::collections::BTreeSet;

use zenkey_model::descriptor::{
    self as d, Descriptor, FORMAT, InterfaceEntry, RequireEntry, Unavailable,
};
use zenkey_model::grammar::{Addr, InstanceId};

use crate::config::ServiceConfig;
use crate::error::{Error, Result};
use crate::implementation::missing_capability;
use crate::service::{ImplState, Role};

/// The descriptor a service with this state serves.
pub(crate) fn build(
    addr: &Addr,
    instance: &InstanceId,
    impls: &[ImplState],
    roles: &[Role],
    config: &ServiceConfig,
    zid: &str,
) -> Descriptor {
    let held = &config.capabilities;
    let interfaces = impls
        .iter()
        .map(|s| {
            // Compact exposure (§3.3): an entry the missing capabilities
            // already imply is left out (it would be D006).
            let unavailable = s
                .unavailable
                .iter()
                .filter(|(name, _)| {
                    s.imp
                        .resource(name)
                        .is_ok_and(|r| missing_capability(r, held).is_none())
                })
                .map(|(name, (cause, reason))| Unavailable {
                    resource: name.clone(),
                    cause: *cause,
                    reason: reason.clone(),
                })
                .collect();
            InterfaceEntry {
                iface: s.imp.iface().to_string(),
                contract: s.imp.fingerprint().to_string(),
                minor: u64::from(s.imp.contract().minor.unwrap_or(0)),
                token: !config.tokenless.contains(s.imp.iface()),
                unavailable,
                cardinality: s.cardinality.clone(),
            }
        })
        .collect();
    let requires = roles
        .iter()
        .map(|r| {
            let b = config.bindings.get(&r.role);
            RequireEntry {
                role: r.role.clone(),
                interface: r.interface.to_string(),
                declared_by: r.declared_by.as_ref().map(ToString::to_string),
                bindings: b.map(|b| b.providers.clone()).unwrap_or_default(),
                params: b.map(|b| b.params.clone()).unwrap_or_default(),
            }
        })
        .collect();
    let profiles: BTreeSet<String> = impls
        .iter()
        .flat_map(|s| s.imp.contract().uses.iter().map(ToString::to_string))
        .collect();
    let mut meta = config.meta.clone();
    meta.entry("zid".to_owned())
        .or_insert_with(|| zid.to_owned().into());
    Descriptor {
        format: FORMAT.to_owned(),
        service: addr.to_string(),
        instance: instance.to_string(),
        interfaces,
        capabilities: held.iter().cloned().collect(),
        requires,
        profiles: profiles.into_iter().collect(),
        meta,
    }
}

/// The descriptor's bytes, after the descriptor check (§3.3) against the
/// contracts implemented. A finding is a bug in the runtime or its caller,
/// so it refuses rather than serve a descriptor a tool would reject.
pub(crate) fn encode_checked(desc: &Descriptor, impls: &[ImplState]) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(desc).expect("a descriptor serializes");
    let text = std::str::from_utf8(&bytes).expect("JSON is UTF-8");
    let contracts: Vec<_> = impls.iter().map(|s| s.imp.contract()).collect();
    let (_, report) = d::check(text, &contracts);
    if report.has_errors() {
        let codes: Vec<String> = report
            .errors()
            .map(|e| format!("{} {}", e.code, e.message))
            .collect();
        return Err(Error::Descriptor(codes.join("; ")));
    }
    if bytes.len() > 1024 {
        tracing::warn!(
            service = desc.service,
            bytes = bytes.len(),
            "the descriptor is over the 1 KB it SHOULD stay within (spec §3.3)"
        );
    }
    Ok(bytes)
}
