//! Building the descriptor from what the service declared (spec §3.3),
//! never from observed traffic.

use std::collections::BTreeSet;

use zenkey_model::descriptor::{
    self as d, Descriptor, FORMAT, InterfaceEntry, RequireEntry, Unavailable,
};
use zenkey_model::grammar::{Addr, IfaceId, InstanceId};

use crate::config::ServiceConfig;
use crate::error::{Error, Result};
use crate::implementation::missing_capability;
use crate::service::{ImplState, Role};

/// Who this instance is, beyond its configuration: the resolved address,
/// whether its system is minted, and the facts `meta` states.
pub(crate) struct Identity<'a> {
    pub(crate) addr: &'a Addr,
    /// The system is minted by `hostid.v1`: `profiles` lists it (§2.8).
    pub(crate) minted: bool,
    /// The host name, stated as `meta.host` for a minted system (§2.13).
    pub(crate) host: Option<&'a str>,
    /// The session's zid, stated as `meta.zid` (§3.3).
    pub(crate) zid: &'a str,
}

/// The descriptor's `profiles` (§3.3): the `uses` of the contracts
/// implemented, and the derivation-only profiles followed (0.19),
/// deduplicated and sorted as §9.5 sorts `uses`, by (name as a string,
/// major as a number), which is [`IfaceId`]'s order (0.20). A string sort
/// differs: it puts `a.v10` before `a.v2`, and `a.b.v1` before `a.v1`.
pub(crate) fn profiles<'a>(
    uses: impl IntoIterator<Item = &'a IfaceId>,
    minted: bool,
) -> Vec<String> {
    let mut set: BTreeSet<IfaceId> = uses.into_iter().cloned().collect();
    if minted {
        set.insert(
            zenkey_model::hostid::PROFILE
                .parse()
                .expect("hostid.v1 is a profile id"),
        );
    }
    set.iter().map(ToString::to_string).collect()
}

/// The descriptor a service with this state serves.
pub(crate) fn build(
    id: &Identity<'_>,
    instance: &InstanceId,
    impls: &[ImplState],
    roles: &[Role],
    config: &ServiceConfig,
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
                optional: r.optional,
            }
        })
        .collect();
    let profiles = profiles(
        impls.iter().flat_map(|s| s.imp.contract().uses.iter()),
        id.minted,
    );
    // What the deployment states wins: `meta.host` only as it wants it
    // seen (hostid.v1 §2.10).
    let mut meta = config.meta.clone();
    meta.entry("zid".to_owned())
        .or_insert_with(|| id.zid.to_owned().into());
    if let (true, Some(host)) = (id.minted, id.host) {
        meta.entry("host".to_owned())
            .or_insert_with(|| host.to_owned().into());
    }
    Descriptor {
        format: FORMAT.to_owned(),
        service: id.addr.to_string(),
        instance: instance.to_string(),
        interfaces,
        capabilities: held.iter().cloned().collect(),
        requires,
        profiles,
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

#[cfg(test)]
mod tests {
    use super::profiles;
    use zenkey_model::grammar::IfaceId;

    fn ids(s: &[&str]) -> Vec<IfaceId> {
        s.iter().map(|p| p.parse().unwrap()).collect()
    }

    /// §3.3 (0.20): §9.5's order, (name, numeric major), deduplicated. A
    /// string sort would put `views.v10` first, and `a.b.v1` before `a.v1`.
    #[test]
    fn profiles_sort_as_uses_do() {
        let uses = ids(&["views.v10", "views.v2", "a.v1", "a.b.v1", "views.v2"]);
        assert_eq!(
            profiles(&uses, false),
            ["a.v1", "a.b.v1", "views.v2", "views.v10"]
        );
        assert_eq!(
            profiles(&uses, true),
            ["a.v1", "a.b.v1", "hostid.v1", "views.v2", "views.v10"]
        );
        // A contract that lists it as well: once (§3.3, hostid.v1 §2.8).
        assert_eq!(
            profiles(&ids(&["hostid.v1", "hostid.v10"]), true),
            ["hostid.v1", "hostid.v10"]
        );
        assert!(profiles(&[], false).is_empty());
    }
}
