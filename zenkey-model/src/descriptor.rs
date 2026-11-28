//! The descriptor record (spec `core.md` §3.3, r4 §3.10).
//!
//! An instance answers a GET on its instance key with this JSON document,
//! and puts it on every change. Exposure is *compact* (r3.3 D8): an
//! interface's exposed resources are its contract's, minus the optional ones
//! gated on a capability the instance does not hold, minus the listed
//! `unavailable` exceptions.
//!
//! `spec/descriptor.schema.json` is generated from these types (schemars;
//! a member left out when empty states its `default` through `extend`,
//! since schemars derives none for a member that skips serializing it),
//! and [`check`] reports findings with stable `D…` codes, which
//! `spec/conformance/descriptors/` pins.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::canonical::Fingerprint;
use crate::catalog::CatalogId;
use crate::chunk::{is_ident, is_lower_hex, is_plain_chunk};
use crate::contract::Contract;
use crate::diag::{Diagnostic, Report};
use crate::grammar::{IfaceId, KindToken};
use crate::wire::Run;

pub mod compact;

/// The format tag of this version of the record.
pub const FORMAT: &str = "zk2-descriptor/0.1";

/// The form a descriptor travels in (core §3.3, "What carries it"), told
/// by its put's or its reply's `Encoding` alone, never by its bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Form {
    /// The JSON form, under `application/json`.
    Json,
    /// The compact form ([`compact`]), under `zenoh/bytes`.
    Compact,
}

impl Form {
    /// The form `encoding` says, as zenoh writes an `Encoding`:
    /// `zenoh/bytes` exactly ([`crate::wire::ENCODING`]) is the compact
    /// form, and any other is read as JSON, where one that does not read is
    /// D000. So a reader never decides by the bytes.
    #[must_use]
    pub fn of(encoding: &str) -> Form {
        if encoding == crate::wire::ENCODING {
            Form::Compact
        } else {
            Form::Json
        }
    }
}

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
    /// The run the instance writes under now (core §3.3, §4.3), as its bytes
    /// in lowercase hexadecimal: 4 digits for a 16-bit run, 8 for a 32-bit
    /// one. An owner states it; a checker reads it as optional, so a
    /// descriptor that states none is not D000, and any other spelling is
    /// D002. Never `null`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "some_string"
    )]
    #[schemars(with = "String")]
    pub run: Option<String>,
    /// The id of the deployment catalog the instance writes under (core
    /// §3.3, §8.6), as 8 lowercase hex digits. An owner given a catalog
    /// states it, and one with none leaves it out; a checker reads it as
    /// optional, and any other spelling is D002. Never `null`.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "some_string"
    )]
    #[schemars(with = "String")]
    pub catalog: Option<String>,
    /// The interfaces this instance implements.
    pub interfaces: Vec<InterfaceEntry>,
    /// The capabilities this instance holds, naming the `capability:` gates
    /// of its contracts. Written only when one is held: absent is none
    /// (§3.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub capabilities: Vec<String>,
    /// Every requirement, with its bindings as declared (R3). Written only
    /// when there is one: absent is none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub requires: Vec<RequireEntry>,
    /// The profiles this instance follows, as `<name>.v<major>`. Written
    /// only when it follows one: absent is none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub profiles: Vec<String>,
    /// Host, process, build: informative, never interpreted. `zid` is the
    /// owner's session zid, which an owner SHOULD state: a tool attributes
    /// a state stamp to the owner by it (§3.3, 0.10; S1). Written only when
    /// it holds a member.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("default" = {}))]
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
    /// The minor of the contract revision the entry names, as its authoring
    /// file states it (§3.3). An owner that holds only the revision's bundle,
    /// which keeps no minor (§9.5), leaves it out, never writing one its
    /// revision does not have; a checker reads it as optional, and
    /// `null` is D000.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "some_u64"
    )]
    #[schemars(with = "u64")]
    pub minor: Option<u64>,
    /// Whether the instance stands by on this interface (§3.3, §6, 0.39):
    /// it lists it, and serves none of it now. Written only when `true`;
    /// absent is serving.
    #[serde(default, skip_serializing_if = "is_false")]
    pub standby: bool,
    /// Whether the instance holds this interface's token. `false` for an
    /// interface in the deployment's tokenless set (U22, spec §8.1).
    #[serde(default = "yes")]
    pub token: bool,
    /// Optional resources that are absent here although no missing
    /// capability implies it. Written only when one is listed: absent is
    /// none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub unavailable: Vec<Unavailable>,
    /// A lower population bound for this instance, per templated resource,
    /// keyed `<kind token>/<template>`. Written only when one is stated:
    /// absent is none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("default" = {}))]
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
    /// `self.system`, `self.service`, or a value. Written only when one is
    /// bound: absent is none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[schemars(extend("default" = {}))]
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

/// An optional member present as a string: `null` is not its shape (D000).
fn some_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    String::deserialize(d).map(Some)
}

/// An optional member present as an integer: `null` is not its shape (D000).
fn some_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}

/// Parses and checks a descriptor. `contracts` are the contracts it is
/// checked against, by fingerprint; an interface whose contract is not
/// supplied is checked for syntax only.
#[must_use]
pub fn check(text: &str, contracts: &[&Contract]) -> (Option<Descriptor>, Report) {
    let value = match crate::strict::parse_json(text) {
        Ok(v) => v,
        Err(e) => {
            let mut report = Report::default();
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
            let mut report = Report::default();
            report.push(Diagnostic::error(
                "D000",
                "descriptor",
                format!("shape: {e}"),
            ));
            return (None, report);
        }
    };
    let report = check_descriptor(&d, contracts);
    (Some(d), report)
}

/// Checks a descriptor already read: every code but D000, which is the
/// reading's. What [`check`] reports once the JSON reads, and what a
/// checker reports on a compact descriptor's decoding (core §3.3, "The
/// checks on a decoded descriptor"; [`compact::decode`]).
#[must_use]
pub fn check_descriptor(d: &Descriptor, contracts: &[&Contract]) -> Report {
    let mut report = Report::default();
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
    if let Some(run) = d.run.as_deref()
        && Run::from_hex(run).is_none()
    {
        report.push(Diagnostic::error(
            "D002",
            "run",
            format!("{run:?} is not 4 or 8 lowercase hex digits"),
        ));
    }
    if let Some(catalog) = d.catalog.as_deref()
        && CatalogId::from_hex(catalog).is_none()
    {
        report.push(Diagnostic::error(
            "D002",
            "catalog",
            format!("{catalog:?} is not 8 lowercase hex digits"),
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
        // §4.4 (0.17): an archive is never tokenless. D011 reads `iface` and
        // `token` only, so a malformed `contract` below does not suppress it
        // (cascade 6, 0.18).
        if e.iface == "archive.v1" && !e.token {
            report.push(Diagnostic::error(
                "D011",
                &at,
                "archive.v1 is marked \"token\": false: an archive is never tokenless (§4.4)",
            ));
        }
        // §3.3, §6 (0.39): an interface with a standby is never tokenless.
        // D012 reads `standby` and `token` only, as D011 reads `iface` and
        // `token` (cascade 7).
        if e.standby && !e.token {
            report.push(Diagnostic::error(
                "D012",
                &at,
                format!(
                    "{} is marked \"standby\": true and \"token\": false: an interface with a \
                     standby is never tokenless (§6, §8.1)",
                    e.iface
                ),
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
    report
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

/// The bytes an owner's descriptor SHOULD stay within (core §3.3): 1 KB.
pub const SIZE_BYTES: usize = 1024;

/// What the rest of a descriptor stays within where the descriptor MAY
/// exceed [`SIZE_BYTES`] (core §3.3): the rest is [`Size::other`].
pub const OTHER_BYTES: usize = 512;

/// A descriptor's bytes against core §3.3's size rule, as
/// [`Descriptor::size_of`] counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    /// The whole document's, as its owner wrote it.
    pub bytes: usize,
    /// The rest (core §3.3): the size of the same document with
    /// `interfaces`, `capabilities`, `requires` and `profiles` taken out.
    /// That is its two braces, every other member and the commas between
    /// those.
    pub other: usize,
}

impl Size {
    /// Whether the descriptor meets the rule: within [`SIZE_BYTES`], or
    /// over it with a rest within [`OTHER_BYTES`].
    #[must_use]
    pub fn meets(&self) -> bool {
        self.bytes <= SIZE_BYTES || self.other <= OTHER_BYTES
    }
}

impl Descriptor {
    /// The size of `served`, the document this descriptor's owner put and
    /// answers, as the owner wrote it (core §3.3): its count of bytes, and
    /// the rest, which is that count less each of `interfaces`,
    /// `capabilities`, `requires` and `profiles` the descriptor states.
    ///
    /// A member is counted from the opening quote of its name to the last
    /// byte of its value, and one comma with it. A member taken out of an
    /// object that keeps another takes exactly one comma with it, wherever
    /// it stands, and `format`, `service` and `instance` are always
    /// written. So the rest is exact for the compact form the reference
    /// writes, every empty member left out. A member's value is counted
    /// as it serializes here, so the rest of a document its owner spaced
    /// otherwise counts that member's spacing too.
    ///
    /// Nothing here can fail: a value that does not serialize counts for
    /// nothing, and the rest never falls below 0.
    #[must_use]
    pub fn size_of(&self, served: &[u8]) -> Size {
        fn member<T: Serialize>(name: &str, value: &T, written: bool) -> usize {
            if !written {
                return 0;
            }
            // `"name":` + the value + `,`.
            serde_json::to_vec(value).map_or(0, |value| name.len() + 3 + value.len() + 1)
        }
        let bytes = served.len();
        let lists = member("interfaces", &self.interfaces, true)
            + member(
                "capabilities",
                &self.capabilities,
                !self.capabilities.is_empty(),
            )
            + member("requires", &self.requires, !self.requires.is_empty())
            + member("profiles", &self.profiles, !self.profiles.is_empty());
        Size {
            bytes,
            other: bytes.saturating_sub(lists),
        }
    }

    /// This descriptor's size as it serializes here ([`Descriptor::size_of`]
    /// over its compact JSON, every empty member left out): what an owner
    /// that has not written it yet would serve.
    ///
    /// # Errors
    /// The descriptor does not serialize.
    pub fn size(&self) -> Result<Size, serde_json::Error> {
        Ok(self.size_of(&serde_json::to_vec(self)?))
    }

    /// The run the descriptor states (core §3.3), `None` when it states
    /// none or spells it otherwise than 4 or 8 lowercase hex digits (D002).
    #[must_use]
    pub fn stated_run(&self) -> Option<Run> {
        self.run.as_deref().and_then(Run::from_hex)
    }

    /// The catalog the descriptor names (core §3.3, §8.6), `None` when it
    /// names none or spells it otherwise than 8 lowercase hex digits (D002).
    #[must_use]
    pub fn stated_catalog(&self) -> Option<CatalogId> {
        self.catalog.as_deref().and_then(CatalogId::from_hex)
    }

    /// The resources of `contract` this instance exposes (§3.3): the
    /// contract's, minus the optional ones gated on a capability it does not
    /// hold, minus those its entry lists `unavailable`; none at all when the
    /// entry marks the interface `"standby": true` (§3.3, §6, 0.39), whose
    /// `unavailable` says what it will expose once it serves. `None` when it
    /// lists no entry for the contract's interface.
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
        if entry.standby {
            return Some(Vec::new());
        }
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
    use super::{Descriptor, Form};

    /// §3.3, "What carries it": `zenoh/bytes` exactly is the compact form,
    /// and every other `Encoding` the JSON one, a schema suffix included.
    #[test]
    fn the_encoding_alone_tells_the_two_forms_apart() {
        assert_eq!(Form::of("zenoh/bytes"), Form::Compact);
        for other in [
            "application/json",
            "text/plain",
            "zenoh/bytes;x",
            "application/octet-stream",
        ] {
            assert_eq!(Form::of(other), Form::Json, "{other}");
        }
    }

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
        // A standby exposes nothing, whatever else its entry says (0.39).
        let mut standby = d("\"x\"", "");
        standby.interfaces[0].standby = true;
        assert!(names(&standby).is_empty());
        let mut other = d("", "");
        other.interfaces[0].iface = "u.v1".into();
        assert!(other.exposed(&c).is_none());
    }

    /// §3.3: a member the schema defaults is written only when it holds
    /// something (#1126), and `minor` only when the revision has one
    /// (#1127); each reads back as it was, and a `null` minor is D000.
    #[test]
    fn empty_members_and_an_unknown_minor_are_left_out() {
        let fp = format!("sha256:{}", "0".repeat(64));
        let mut d: Descriptor = serde_json::from_str(&format!(
            r#"{{"format": "zk2-descriptor/0.1", "service": "s/t", "instance": "000000000000000a",
                "interfaces": [{{"iface": "t.v1", "contract": "{fp}"}}]}}"#
        ))
        .unwrap();
        assert_eq!(d.interfaces[0].minor, None);
        assert!(d.interfaces[0].token, "absent is true");
        let text = serde_json::to_string(&d).unwrap();
        assert_eq!(
            text,
            format!(
                r#"{{"format":"zk2-descriptor/0.1","service":"s/t","instance":"000000000000000a","interfaces":[{{"iface":"t.v1","contract":"{fp}","token":true}}]}}"#
            )
        );
        d.interfaces[0].minor = Some(4);
        let text = serde_json::to_string(&d).unwrap();
        assert!(text.contains(r#""minor":4"#), "{text}");
        assert_eq!(serde_json::from_str::<Descriptor>(&text).unwrap(), d);
        let null = text.replace(r#""minor":4"#, r#""minor":null"#);
        let (_, report) = super::check(&null, &[]);
        assert_eq!(report.codes(), ["D000"], "{report}");
    }

    /// §3.3's size rule: within 1 KB, or over it with a rest within
    /// 512 B, the rest being the document with `interfaces`,
    /// `capabilities`, `requires` and `profiles` taken out.
    #[test]
    fn a_descriptor_over_1_kb_is_judged_by_its_other_members() {
        let fp = format!("sha256:{}", "0".repeat(64));
        let mut d: Descriptor = serde_json::from_str(&format!(
            r#"{{"format": "zk2-descriptor/0.1", "service": "s/t", "instance": "000000000000000a",
                "interfaces": [{{"iface": "t.v1", "contract": "{fp}"}}],
                "meta": {{"zid": "1"}}}}"#
        ))
        .unwrap();
        let text = serde_json::to_string(&d).unwrap();
        let small = d.size().unwrap();
        assert_eq!(small, d.size_of(text.as_bytes()), "as it serializes here");
        assert_eq!(small.bytes, text.len());
        // The rest, where a member follows the four.
        let other = r#"{"format":"zk2-descriptor/0.1","service":"s/t","instance":"000000000000000a","meta":{"zid":"1"}}"#;
        assert_eq!(small.other, other.len());
        assert!(small.bytes <= super::SIZE_BYTES && small.meets());

        // Many interfaces and capabilities: over 1 KB by what it must state.
        let entry = d.interfaces[0].clone();
        for n in 2..=8 {
            let mut e = entry.clone();
            e.iface = format!("t{n}.v1");
            d.interfaces.push(e);
        }
        d.capabilities = (0..30).map(|n| format!("capability_{n}")).collect();
        d.profiles = vec!["freshness.v2".into()];
        let many = d.size().unwrap();
        assert_eq!(many.bytes, serde_json::to_string(&d).unwrap().len());
        assert!(many.bytes > super::SIZE_BYTES, "{many:?}");
        assert_eq!(many.other, other.len(), "the four count for nothing");
        assert!(many.meets());

        // The four come last, `profiles` the last member written: a member
        // takes one comma with it wherever it stands, so the rest is the
        // three members left and their braces, to the byte.
        let mut last = d.clone();
        last.meta.clear();
        let text = serde_json::to_string(&last).unwrap();
        assert!(text.ends_with(r#""profiles":["freshness.v2"]}"#), "{text}");
        let rest =
            r#"{"format":"zk2-descriptor/0.1","service":"s/t","instance":"000000000000000a"}"#;
        assert_eq!(
            last.size().unwrap(),
            super::Size {
                bytes: text.len(),
                other: rest.len()
            }
        );
        // `interfaces` alone of the four, and the last member written.
        last.capabilities.clear();
        last.profiles.clear();
        let text = serde_json::to_string(&last).unwrap();
        assert!(text.ends_with("]}") && !text.contains("profiles"), "{text}");
        assert_eq!(last.size().unwrap().other, rest.len());

        // The same lists beside a `meta` of its own 600 B: not met.
        d.meta.insert("build".into(), "x".repeat(600).into());
        let padded = d.size().unwrap();
        assert!(
            padded.other > super::OTHER_BYTES && !padded.meets(),
            "{padded:?}"
        );
        // A small descriptor may spend its 1 KB as it likes.
        d.interfaces.truncate(1);
        d.capabilities.clear();
        d.profiles.clear();
        let lone = d.size().unwrap();
        assert!(lone.other > super::OTHER_BYTES && lone.bytes <= super::SIZE_BYTES);
        assert!(lone.meets());

        // The size is the served document's, and the rest never falls
        // below 0: fewer bytes than the four members take leave none.
        assert_eq!(
            d.size_of(b"{}"),
            super::Size { bytes: 2, other: 0 },
            "saturating"
        );
    }
}
