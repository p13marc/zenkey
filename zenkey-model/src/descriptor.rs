//! The descriptor record (spec `core.md` §3.3, r4 §3.10).
//!
//! An instance answers a GET on its instance key with this JSON document,
//! and puts it on every change. Exposure is *compact* (r3.3 D8): an
//! interface's exposed resources are its contract's, minus the optional ones
//! gated on a capability the instance does not hold, minus the listed
//! `unavailable` exceptions.
//!
//! `spec/descriptor.schema.json` is generated from these types (schemars),
//! and [`check`] reports findings with stable `D…` codes, which
//! `spec/conformance/descriptors/` pins.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical::Fingerprint;
use crate::chunk::{is_ident, is_lower_hex, is_plain_chunk};
use crate::contract::Contract;
use crate::diag::{Diagnostic, Report};
use crate::grammar::{IfaceId, KindToken};

/// The format tag of this version of the record.
pub const FORMAT: &str = "zk2-descriptor/0.1";

/// One instance's descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    /// `zk2-descriptor/0.1`.
    pub format: String,
    /// `<system>/<service>`.
    pub service: String,
    /// The instance id: 16 lowercase hex digits.
    pub instance: String,
    /// The interfaces this instance implements.
    pub interfaces: Vec<InterfaceEntry>,
    /// The capabilities this instance holds, naming the `capability:` gates
    /// of its contracts.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Every requirement, with its bindings as declared (R3).
    #[serde(default)]
    pub requires: Vec<RequireEntry>,
    /// The profiles this instance follows, as `<name>.v<major>`.
    #[serde(default)]
    pub profiles: Vec<String>,
    /// Host, process, build: informative, never interpreted. `zid` is the
    /// owner's session zid, which an owner SHOULD state: a tool attributes
    /// a state stamp to the owner by it (§3.3, 0.10; S1).
    #[serde(default)]
    pub meta: BTreeMap<String, Value>,
}

/// One implemented interface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InterfaceEntry {
    /// `<name>.v<major>`.
    pub iface: String,
    /// The contract's fingerprint, `sha256:` + 64 lowercase hex digits.
    pub contract: String,
    /// The contract's informative minor.
    pub minor: u64,
    /// Whether the instance holds this interface's token. `false` for an
    /// interface in the deployment's tokenless set (U22, spec §8.1).
    #[serde(default = "yes")]
    pub token: bool,
    /// Optional resources that are absent here although no missing
    /// capability implies it.
    #[serde(default)]
    pub unavailable: Vec<Unavailable>,
    /// A lower population bound for this instance, per templated resource,
    /// keyed `<kind token>/<template>`.
    #[serde(default)]
    pub cardinality: BTreeMap<String, u64>,
}

/// An optional resource this instance does not expose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Unavailable {
    /// `<kind token>/<template>`, as in `state/covariance` or `@op/arm`.
    pub resource: String,
    pub cause: Cause,
    /// For a human; never parsed.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Why a resource is absent here (r3.3 D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Cause {
    Build,
    Config,
    Capability,
}

/// One requirement and its bindings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequireEntry {
    /// The role's name, `[a-z][a-z0-9_]*`.
    pub role: String,
    /// The required interface, `<name>.v<major>`.
    pub interface: String,
    /// The interface whose contract declares the role, or `null` when the
    /// component's own manifest does.
    #[serde(default)]
    pub declared_by: Option<String>,
    /// Service addresses, `<system>/<service>`; either chunk may be `*`.
    pub bindings: Vec<String>,
    /// Template parameters bound by the binding (R2): parameter name →
    /// `self.system`, `self.service`, or a value.
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// Whether the role is optional: the instance works without a provider
    /// (§3.2, 0.10). Written only when `true`; absent is required. A role a
    /// contract declares is optional as that contract says; one the
    /// component's manifest declares, as the manifest says.
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// Parses and checks a descriptor. `contracts` are the contracts it is
/// checked against, by fingerprint; an interface whose contract is not
/// supplied is checked for syntax only.
#[must_use]
pub fn check(text: &str, contracts: &[&Contract]) -> (Option<Descriptor>, Report) {
    let mut report = Report::default();
    let value = match crate::strict::parse_json(text) {
        Ok(v) => v,
        Err(e) => {
            report.push(Diagnostic::error(
                "D000",
                "descriptor",
                format!("not strict JSON: {e}"),
            ));
            return (None, report);
        }
    };
    let d: Descriptor = match serde_json::from_value(value) {
        Ok(d) => d,
        Err(e) => {
            report.push(Diagnostic::error(
                "D000",
                "descriptor",
                format!("shape: {e}"),
            ));
            return (None, report);
        }
    };
    if d.format != FORMAT {
        report.push(Diagnostic::error(
            "D001",
            "format",
            format!("{:?} is not {FORMAT:?}", d.format),
        ));
    }
    let service_ok = d
        .service
        .split_once('/')
        .is_some_and(|(sys, svc)| is_plain_chunk(sys) && is_plain_chunk(svc));
    if !service_ok {
        report.push(Diagnostic::error(
            "D002",
            "service",
            format!("{:?} is not <system>/<service> (plain chunks)", d.service),
        ));
    }
    if !is_lower_hex(&d.instance, 16) {
        report.push(Diagnostic::error(
            "D002",
            "instance",
            format!("{:?} is not 16 lowercase hex digits", d.instance),
        ));
    }
    for (i, c) in d.capabilities.iter().enumerate() {
        if !is_name(c) {
            report.push(Diagnostic::error(
                "D008",
                format!("capabilities[{i}]"),
                format!("{c:?} is not [a-z0-9][a-z0-9_.-]*"),
            ));
        }
    }
    if d.capabilities.iter().collect::<BTreeSet<_>>().len() != d.capabilities.len() {
        report.push(Diagnostic::error(
            "D008",
            "capabilities",
            "a capability is listed twice",
        ));
    }
    let held: BTreeSet<&str> = d.capabilities.iter().map(String::as_str).collect();
    let mut seen = BTreeSet::new();
    let mut ifaces = BTreeSet::new();
    for (i, e) in d.interfaces.iter().enumerate() {
        let at = format!("interfaces[{i}]");
        if IfaceId::from_str(&e.iface).is_err() {
            report.push(Diagnostic::error(
                "D003",
                &at,
                format!("{:?} is not <name>.v<major>", e.iface),
            ));
            continue;
        }
        if !seen.insert(e.iface.clone()) {
            report.push(Diagnostic::error(
                "D003",
                &at,
                format!("{} is listed twice", e.iface),
            ));
        }
        ifaces.insert(e.iface.clone());
        // §4.4 (0.17): an archive is never tokenless, whatever its contract.
        if e.iface == "archive.v1" && !e.token {
            report.push(Diagnostic::error(
                "D011",
                &at,
                "archive.v1 is marked \"token\": false: an archive is never tokenless (§4.4)",
            ));
        }
        let fp_ok = e
            .contract
            .strip_prefix("sha256:")
            .is_some_and(|h| is_lower_hex(h, 64));
        if !fp_ok {
            report.push(Diagnostic::error(
                "D003",
                &at,
                format!(
                    "contract {:?} is not sha256: + 64 lowercase hex digits",
                    e.contract
                ),
            ));
            continue;
        }
        let same_iface: Vec<&&Contract> = contracts
            .iter()
            .filter(|c| c.iface.to_string() == e.iface)
            .collect();
        let Some(c) = same_iface
            .iter()
            .find(|c| Fingerprint::of(c).to_string() == e.contract)
        else {
            if !same_iface.is_empty() {
                report.push(Diagnostic::error(
                    "D004",
                    &at,
                    format!(
                        "{} names a contract revision that was not supplied",
                        e.iface
                    ),
                ));
            }
            continue;
        };
        check_exposure(e, c, &held, &at, &mut report);
    }
    for (i, r) in d.requires.iter().enumerate() {
        let at = format!("requires[{i}]");
        let mut bad = Vec::new();
        if !is_ident(&r.role) {
            bad.push(format!("role {:?} is not [a-z][a-z0-9_]*", r.role));
        }
        if IfaceId::from_str(&r.interface).is_err() {
            bad.push(format!(
                "interface {:?} is not <name>.v<major>",
                r.interface
            ));
        }
        if let Some(by) = &r.declared_by
            && !ifaces.contains(by)
        {
            bad.push(format!(
                "declared_by {by:?} is not one of this instance's interfaces"
            ));
        }
        for (k, v) in &r.params {
            if !is_ident(k) || v.is_empty() {
                bad.push(format!(
                    "parameter binding {k:?} = {v:?} is not <name> = a non-empty value"
                ));
            }
        }
        for b in &r.bindings {
            let ok = b.split_once('/').is_some_and(|(sys, svc)| {
                (sys == "*" || is_plain_chunk(sys)) && (svc == "*" || is_plain_chunk(svc))
            });
            if !ok {
                bad.push(format!(
                    "binding {b:?} is not <system>/<service> (a chunk may be *)"
                ));
            }
        }
        for m in bad {
            report.push(Diagnostic::error("D009", &at, m));
        }
    }
    for (i, p) in d.profiles.iter().enumerate() {
        if IfaceId::from_str(p).is_err() {
            report.push(Diagnostic::error(
                "D010",
                format!("profiles[{i}]"),
                format!("{p:?} is not <name>.v<major>"),
            ));
        }
    }
    // Like a capability (D008) and an interface (D003), a profile is listed
    // once.
    if d.profiles.iter().collect::<BTreeSet<_>>().len() != d.profiles.len() {
        report.push(Diagnostic::error(
            "D010",
            "profiles",
            "a profile is listed twice",
        ));
    }
    (Some(d), report)
}

/// The exposure rules of one interface against its contract.
fn check_exposure(
    e: &InterfaceEntry,
    c: &Contract,
    held: &BTreeSet<&str>,
    at: &str,
    report: &mut Report,
) {
    for u in &e.unavailable {
        let Some(r) = resource(c, &u.resource) else {
            report.push(Diagnostic::error(
                "D005",
                at,
                format!(
                    "unavailable {:?} is not a resource of {}",
                    u.resource, c.iface
                ),
            ));
            continue;
        };
        if !r.optional {
            report.push(Diagnostic::error(
                "D005",
                at,
                format!("unavailable {:?} is a required resource", u.resource),
            ));
            continue;
        }
        let implied = r.gate.iter().any(|g| {
            g.strip_prefix("capability:")
                .is_some_and(|cap| !held.contains(cap))
        });
        if implied {
            report.push(Diagnostic::warning(
                "D006",
                at,
                format!(
                    "unavailable {:?} is already implied by a capability not held",
                    u.resource
                ),
            ));
        }
    }
    for (name, n) in &e.cardinality {
        let ceiling = resource(c, name).and_then(|r| r.cardinality);
        match ceiling {
            None => report.push(Diagnostic::error(
                "D007",
                at,
                format!(
                    "cardinality {name:?} names no templated resource of {}",
                    c.iface
                ),
            )),
            Some(max) if *n == 0 || *n > max => report.push(Diagnostic::error(
                "D007",
                at,
                format!("cardinality {name:?} = {n} is not within 1..={max}"),
            )),
            Some(_) => {}
        }
    }
}

impl Descriptor {
    /// The resources of `contract` this instance exposes (§3.3): the
    /// contract's, minus the optional ones gated on a capability it does not
    /// hold, minus those its entry lists `unavailable`. `None` when it lists
    /// no entry for the contract's interface.
    ///
    /// What a tool decides a split-brain from, beside replicated serving
    /// (§6, 0.7, #660). The entry's revision is the caller's to match: this
    /// reads `contract` as given.
    #[must_use]
    pub fn exposed<'c>(
        &self,
        contract: &'c Contract,
    ) -> Option<Vec<&'c crate::contract::Resource>> {
        let iface = contract.iface.to_string();
        let entry = self.interfaces.iter().find(|e| e.iface == iface)?;
        let held = |cap: &str| self.capabilities.iter().any(|h| h == cap);
        Some(
            contract
                .resources
                .iter()
                .filter(|r| {
                    let name = format!("{}/{}", r.token, r.template);
                    let gated_off = r
                        .gate
                        .iter()
                        .any(|g| g.strip_prefix("capability:").is_some_and(|cap| !held(cap)));
                    let listed = entry.unavailable.iter().any(|u| u.resource == name);
                    !(r.optional && (gated_off || listed))
                })
                .collect(),
        )
    }
}

fn yes() -> bool {
    true
}

/// A resource named `<kind token>/<template>`.
fn resource<'c>(c: &'c Contract, name: &str) -> Option<&'c crate::contract::Resource> {
    let (token, template) = name.split_once('/')?;
    let token = KindToken::from_str(token).ok()?;
    c.resource(token, template)
}

/// A capability name: `[a-z0-9][a-z0-9_.-]*`, as in a `capability:` gate.
fn is_name(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::Descriptor;

    /// §3.3's compact exposure: a missing capability implies an optional
    /// resource absent, as a listed one is; a required resource is always
    /// exposed.
    #[test]
    fn exposure_is_the_contract_minus_the_gated_off_and_the_listed() {
        let op = |name: &str, extra: &str| {
            format!(
                "[resources.{name}]\nkind = \"operation\"\n{extra}\
                 request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n"
            )
        };
        let toml = format!(
            "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n{}{}{}",
            op("a", ""),
            op("b", "optional = true\n"),
            op("c", "optional = true\ngate = [\"capability:x\"]\n"),
        );
        let l = crate::contract::load_str(&toml, std::path::Path::new("."), None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        let d = |caps: &str, unavailable: &str| -> Descriptor {
            serde_json::from_str(&format!(
                r#"{{"format": "zk2-descriptor/0.1", "service": "s/t", "instance": "000000000000000a",
                    "interfaces": [{{"iface": "t.v1", "contract": "sha256:{}", "minor": 0,
                                     "unavailable": [{unavailable}]}}],
                    "capabilities": [{caps}]}}"#,
                "0".repeat(64)
            ))
            .unwrap()
        };
        let names = |d: &Descriptor| -> Vec<String> {
            d.exposed(&c)
                .unwrap()
                .iter()
                .map(|r| r.template.as_str().to_owned())
                .collect()
        };
        assert_eq!(names(&d("", "")), ["a", "b"]);
        assert_eq!(names(&d("\"x\"", "")), ["a", "b", "c"]);
        assert_eq!(
            names(&d("\"x\"", r#"{"resource": "@op/b", "cause": "config"}"#)),
            ["a", "c"]
        );
        let mut other = d("", "");
        other.interfaces[0].iface = "u.v1".into();
        assert!(other.exposed(&c).is_none());
    }
}
