//! The lens a raw observer reads zk2 through (#612, FJ8b): one namespace,
//! one presence read (or none), and the revisions in hand.
//!
//! `echo`, `rate`, `field`, `timeline`, `snapshot`, the watchdog and the
//! trigger capture see the bus un-namespaced — full wire keys, every
//! deployment's and none — and each one tells a person how far each key
//! resolved, by the tooling guide's O2 ladder:
//!
//! 1. in this namespace ([`Unresolved::NotInNamespace`] when not, terminal,
//!    and not guessed at, O3);
//! 2. one of the five key forms (`NotZk2`, or a control key);
//! 3. a provider in presence (`NoProvider`; `PresenceNotRead` when no
//!    presence read stood behind the lens);
//! 4. a descriptor naming the revision (`NoRevision`, `Ambiguous`);
//! 5. the bundle held (`ContractNotHeld`, `ContractUnavailable`,
//!    `ContractUnreadable`);
//! 6. a resource template and its member (`NoResource`, `NoMember`);
//! 7. the bytes decode as the declared type ([`Rendered`]), and for a JSON
//!    Schema type, the value satisfies it ([`Conformance`]).
//!
//! Session-free, so a `.zrec` resolves through the same lens as live
//! traffic: a capture read with `--contracts` and no presence resolves each
//! interface by the one revision held offline, and says it did.

use std::collections::BTreeSet;
use std::sync::Arc;

use zenkey_model::authoring::{Congestion, Priority};
use zenkey_model::contract::{Body, Resource};
use zenkey_model::decode;
use zenkey_model::grammar::{Addr, IfaceId, ZkKey};
use zenkey_model::template::{Bindings, resolve};

use crate::model::catalog::{Catalog, ContractSet, ContractState, Contracts, Revision, zid_value};
use crate::model::render::Member;
use crate::model::structural::{structural, structural_value};
use crate::report::{
    Conformance, KeyGroup, KeyIdentity, LensPresence, LensScope, PayloadRendering, Provenance,
    QosAxes, QosMismatch, Rendered, ResolvedResource, Unresolved,
};

/// What a raw observer resolves wire keys with.
#[derive(Clone, Copy)]
pub struct Lens<'a> {
    namespace: &'a str,
    catalog: Option<&'a Catalog>,
    contracts: &'a (dyn Contracts + Sync),
    offline: Option<&'a ContractSet>,
    held: usize,
}

/// A key resolved down to its resource (rungs 1–6).
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The namespace-relative key.
    pub key: String,
    pub address: Addr,
    pub iface: IfaceId,
    pub revision: Arc<Revision>,
    index: usize,
    /// The template's values on the key, unslugged.
    pub values: Bindings,
}

impl Resolved {
    /// The resource the key resolved to.
    pub fn resource(&self) -> &Resource {
        &self.revision.contract().resources[self.index]
    }

    /// `<kind token>/<template>`.
    pub fn resource_name(&self) -> String {
        zenkey::implementation::resource_name(self.resource())
    }
}

/// How far one key resolved: its identity always, the resolution when the
/// ladder reached the resource.
#[derive(Debug, Clone)]
pub struct Resolution {
    pub identity: KeyIdentity,
    pub resolved: Option<Resolved>,
}

/// One payload through the lens: the key's identity, what the payload
/// rendered as, and its conformance to the declared type.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub identity: KeyIdentity,
    pub rendering: PayloadRendering,
    pub conformance: Conformance,
}

impl<'a> Lens<'a> {
    /// A lens over `namespace` (empty: the bus root), resolving addresses
    /// through `catalog` when a presence read was made, and decoding
    /// through `contracts`.
    pub fn new(
        namespace: &'a str,
        catalog: Option<&'a Catalog>,
        contracts: &'a (dyn Contracts + Sync),
    ) -> Lens<'a> {
        Lens {
            namespace,
            catalog,
            contracts,
            offline: None,
            held: 0,
        }
    }

    /// Revisions known offline (`--contracts`): without a presence read, an
    /// interface of which exactly one revision is held resolves through it
    /// (a `.zrec` read offline). With a presence read, the descriptors
    /// name the revision and this is not consulted.
    pub fn offline(mut self, set: &'a ContractSet) -> Lens<'a> {
        self.offline = Some(set);
        self
    }

    /// How many revisions are in hand, for [`Lens::scope`].
    pub fn held(mut self, n: usize) -> Lens<'a> {
        self.held = n;
        self
    }

    /// The namespace this lens strips; empty for the bus root.
    pub fn namespace(&self) -> &str {
        self.namespace
    }

    /// The presence read behind the lens, when one was made.
    pub fn catalog(&self) -> Option<&'a Catalog> {
        self.catalog
    }

    /// What the lens resolves with, as a report states it.
    pub fn scope(&self) -> LensScope {
        LensScope {
            namespace: self.namespace.to_owned(),
            presence: self.catalog.map(|c| LensPresence {
                selector: c.selector().to_owned(),
                complete: c.complete(),
                services: c.service_count(),
            }),
            contracts: self.held,
        }
    }

    /// Rung 1: the key relative to the namespace, or `None` when it does
    /// not sit under it.
    pub fn relative<'k>(&self, wire: &'k str) -> Option<&'k str> {
        crate::model::namespace::strip(self.namespace, wire)
    }

    /// Rungs 1–6 for one wire key.
    pub fn resolve(&self, wire: &str) -> Resolution {
        let stop = |group: KeyGroup, why: Unresolved| Resolution {
            identity: KeyIdentity {
                group,
                values: Bindings::new(),
                unresolved: Some(why),
            },
            resolved: None,
        };
        let Some(rel) = self.relative(wire) else {
            return stop(
                KeyGroup::NotInNamespace,
                Unresolved::NotInNamespace {
                    namespace: self.namespace.to_owned(),
                },
            );
        };
        let (addr, iface, kind, chunks) = match zenkey_model::grammar::parse(rel) {
            Ok(ZkKey::Data {
                addr,
                iface,
                kind,
                resource,
            }) => (addr, iface, kind, resource),
            Ok(other) => {
                let (address, form) = control(&other);
                return Resolution {
                    identity: KeyIdentity {
                        group: KeyGroup::Control {
                            address,
                            form: form.to_owned(),
                        },
                        values: Bindings::new(),
                        unresolved: None,
                    },
                    resolved: None,
                };
            }
            Err(e) => {
                return stop(
                    KeyGroup::NotZk2,
                    Unresolved::NotZk2 {
                        detail: e.to_string(),
                    },
                );
            }
        };
        let group = |resource: Option<String>| KeyGroup::Resource {
            address: addr.to_string(),
            iface: iface.to_string(),
            token: kind.as_str().to_owned(),
            resource,
        };
        let revision = match self.revision_of(&addr, &iface) {
            Ok(r) => r,
            Err(why) => return stop(group(None), why),
        };
        let candidates: Vec<(usize, &Resource)> = revision
            .contract()
            .resources
            .iter()
            .enumerate()
            .filter(|(_, r)| r.token == kind)
            .collect();
        let refs: Vec<&str> = template_chunks(kind, &chunks)
            .iter()
            .map(String::as_str)
            .collect();
        let Some((i, values)) = resolve(candidates.iter().map(|(_, r)| &r.template), &refs) else {
            return stop(
                group(None),
                Unresolved::NoResource {
                    fingerprint: revision.fingerprint().to_string(),
                },
            );
        };
        let index = candidates[i].0;
        let name = zenkey::implementation::resource_name(&revision.contract().resources[index]);
        Resolution {
            identity: KeyIdentity {
                group: group(Some(name)),
                values: values.clone(),
                unresolved: None,
            },
            resolved: Some(Resolved {
                key: rel.to_owned(),
                address: addr,
                iface,
                revision,
                index,
                values,
            }),
        }
    }

    /// Rungs 3–5: the revision `addr`'s descriptors name for `iface`, held.
    fn revision_of(&self, addr: &Addr, iface: &IfaceId) -> Result<Arc<Revision>, Unresolved> {
        let fp = match self.catalog {
            Some(catalog) => {
                let fps = catalog.revisions_of(addr, iface);
                match fps.len() {
                    0 if catalog.provides(addr, iface) => return Err(Unresolved::NoRevision),
                    0 => return Err(Unresolved::NoProvider),
                    1 => fps.into_iter().next().expect("one revision"),
                    _ => {
                        return Err(Unresolved::Ambiguous {
                            fingerprints: fps.iter().map(ToString::to_string).collect(),
                        });
                    }
                }
            }
            None => {
                let Some(set) = self.offline else {
                    return Err(Unresolved::PresenceNotRead);
                };
                let revs: Vec<&Arc<Revision>> = set.of_iface(iface).collect();
                return match revs.as_slice() {
                    [] => Err(Unresolved::PresenceNotRead),
                    [one] => Ok(Arc::clone(one)),
                    several => Err(Unresolved::Ambiguous {
                        fingerprints: several
                            .iter()
                            .map(|r| r.fingerprint().to_string())
                            .collect(),
                    }),
                };
            }
        };
        match self.contracts.state(iface, &fp) {
            Some(ContractState::Held(revision)) => Ok(revision),
            None => Err(Unresolved::ContractNotHeld {
                fingerprint: fp.to_string(),
            }),
            Some(ContractState::Unavailable { refused }) => Err(Unresolved::ContractUnavailable {
                fingerprint: fp.to_string(),
                refused,
            }),
            Some(ContractState::Unreadable { reason }) => Err(Unresolved::ContractUnreadable {
                fingerprint: fp.to_string(),
                detail: reason,
            }),
        }
    }

    /// The key's identity alone.
    pub fn identity(&self, wire: &str) -> KeyIdentity {
        self.resolve(wire).identity
    }

    /// Rungs 1–7 for one payload of `member`, and its conformance (§7.2,
    /// §7.3). The rendering's key is namespace-relative when the key sits in
    /// the namespace, and the wire key otherwise.
    pub fn check(
        &self,
        wire: &str,
        member: Member,
        encoding: Option<&str>,
        bytes: &[u8],
    ) -> Checked {
        let resolution = self.resolve(wire);
        let key = self.relative(wire).unwrap_or(wire);
        match &resolution.resolved {
            Some(r) => {
                let (rendering, conformance) = check_resolved(r, member, encoding, bytes);
                Checked {
                    identity: resolution.identity,
                    rendering,
                    conformance,
                }
            }
            None => {
                let why =
                    resolution.identity.unresolved.clone().unwrap_or_else(|| {
                        match &resolution.identity.group {
                            KeyGroup::Control { form, .. } => {
                                Unresolved::ControlKey { form: form.clone() }
                            }
                            _ => Unresolved::PresenceNotRead,
                        }
                    });
                let conformance = Conformance::NotChecked {
                    reason: why.words(),
                };
                Checked {
                    rendering: structural_rendering(key, why, bytes),
                    conformance,
                    identity: resolution.identity,
                }
            }
        }
    }

    /// [`Lens::check`]'s rendering alone.
    pub fn render(
        &self,
        wire: &str,
        member: Member,
        encoding: Option<&str>,
        bytes: &[u8],
    ) -> PayloadRendering {
        self.check(wire, member, encoding, bytes).rendering
    }

    /// The QoS `wire`'s resource declares (§2.4), when the key resolves to
    /// a stream, state or event resource; why not otherwise.
    pub fn declared_qos(&self, wire: &str) -> Result<QosAxes, Unresolved> {
        let resolution = self.resolve(wire);
        let Some(r) = resolution.resolved else {
            return Err(resolution
                .identity
                .unresolved
                .unwrap_or(Unresolved::ControlKey {
                    form: "control".into(),
                }));
        };
        declared_qos(r.resource()).ok_or_else(|| Unresolved::NoMember {
            resource: r.resource_name(),
            member: "qos".into(),
        })
    }

    /// Whether `address` held an instance token in the presence read:
    /// `None` when no read stood behind the lens.
    pub fn has_instance(&self, address: &str) -> Option<bool> {
        let catalog = self.catalog?;
        let addr: Addr = address.parse().ok()?;
        Some(catalog.has_instance(&addr))
    }

    /// Whether `address`'s descriptor carries a mock owner's synthetic
    /// marker (`meta.synthetic`, FJ8a); `false` when unknown.
    pub fn is_synthetic(&self, address: &str) -> bool {
        let (Some(catalog), Ok(addr)) = (self.catalog, address.parse::<Addr>()) else {
            return false;
        };
        catalog.is_synthetic(&addr)
    }

    /// Whose clock `stamper` is, for a key of `address` (the tooling guide's
    /// O7; spec §3.3 0.10, §4.2 "Observing S1"): the owner's when it is the
    /// session zid the address's descriptors state as `meta.zid`, compared
    /// by value; another clock when they name one and it differs; and
    /// unattributable when nothing names the owner's — never foreign.
    pub fn provenance(&self, address: Option<&str>, stamper: &str) -> Provenance {
        let owners = self.owner_zids(address);
        if owners.is_empty() {
            Provenance::Unattributable
        } else if owners.contains(&zid_value(stamper)) {
            Provenance::Owner
        } else {
            Provenance::Other
        }
    }

    /// The owner zids of `address` (by value), empty when unknown.
    pub fn owner_zids(&self, address: Option<&str>) -> BTreeSet<String> {
        let (Some(catalog), Some(address)) = (self.catalog, address) else {
            return BTreeSet::new();
        };
        match address.parse::<Addr>() {
            Ok(addr) => catalog.owner_zids(&addr),
            Err(_) => BTreeSet::new(),
        }
    }
}

/// The resource chunks a template is matched against (§2.6): an event
/// key's last chunk is the occurrence's ULID, which "a reader strips before
/// resolving the template"; every other kind's chunks are the template's
/// own (#702, which found event keys resolving to no resource here).
pub fn template_chunks(kind: zenkey_model::grammar::KindToken, chunks: &[String]) -> &[String] {
    match (kind, chunks.split_last()) {
        (zenkey_model::grammar::KindToken::Events, Some((_ulid, template))) => template,
        _ => chunks,
    }
}

/// A control key's address and form.
fn control(k: &ZkKey) -> (Option<String>, &'static str) {
    match k {
        ZkKey::Instance { addr, .. } => (Some(addr.to_string()), "instance"),
        ZkKey::Alive { addr, .. } => (Some(addr.to_string()), "alive"),
        ZkKey::Member { addr, .. } => (Some(addr.to_string()), "member"),
        ZkKey::Contract { .. } => (None, "contract"),
        ZkKey::Data { addr, .. } => (Some(addr.to_string()), "data"),
    }
}

/// Rung 7 through a resolved resource: decode the member, then check the
/// value against its JSON Schema type.
fn check_resolved(
    r: &Resolved,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
) -> (PayloadRendering, Conformance) {
    let resource = r.resource();
    let name = r.resource_name();
    let bundle = r.revision.bundle();
    let Some(ty) = decode::type_of(
        bundle,
        resource.token.as_str(),
        resource.template.as_str(),
        member.as_str(),
    ) else {
        let why = Unresolved::NoMember {
            resource: name,
            member: member.as_str().to_owned(),
        };
        let reason = why.words();
        return (
            structural_rendering(&r.key, why, bytes),
            Conformance::NotChecked { reason },
        );
    };
    // zenoh's default, `zenoh/bytes`, is "unsaid": the contract speaks next.
    let encoding = encoding.filter(|e| *e != "zenoh/bytes");
    let (rendered, conformance) = match decode::decode(bundle, ty, encoding, bytes) {
        decode::Rendered::Value(value) => {
            let declared = decode::declared(ty);
            let conformance = if ty["kind"] == "jsonschema" {
                match zenkey_model::validate::validate(bundle, ty, &value) {
                    Ok(()) => Conformance::Valid,
                    Err(violations) => Conformance::Invalid {
                        violations: violations.iter().map(ToString::to_string).collect(),
                    },
                }
            } else {
                Conformance::Valid
            };
            (Rendered::Value { declared, value }, conformance)
        }
        decode::Rendered::Opaque { media_type, .. } => (
            Rendered::Opaque {
                media_type: media_type.clone(),
            },
            Conformance::NotChecked {
                reason: format!("a raw type ({media_type}) declares no structure to check"),
            },
        ),
        decode::Rendered::Undecodable {
            declared, reason, ..
        } => (
            Rendered::Undecodable {
                declared: declared.clone(),
                reason: reason.clone(),
            },
            Conformance::Undecodable { declared, reason },
        ),
    };
    (
        PayloadRendering {
            key: r.key.clone(),
            size: bytes.len(),
            resource: Some(ResolvedResource {
                iface: r.iface.to_string(),
                fingerprint: r.revision.fingerprint().to_string(),
                resource: name,
                member: member.as_str().to_owned(),
                values: r.values.clone(),
            }),
            rendered,
        },
        conformance,
    )
}

/// One payload of `member` of `resource`, checked against its declared type
/// offline (#612, FJ8b; `zenctl check schema`): decoded through the bundle
/// as `encoding` (else the contract's), a JSON Schema value then validated
/// (§7.3). `Err` names why it cannot be checked at all — the resource
/// declares no such member, or the type is raw and declares no structure.
pub fn check_payload(
    revision: &Revision,
    resource: &Resource,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
) -> std::result::Result<crate::report::PayloadCheck, String> {
    let bundle = revision.bundle();
    let name = zenkey::implementation::resource_name(resource);
    let ty = decode::type_of(
        bundle,
        resource.token.as_str(),
        resource.template.as_str(),
        member.as_str(),
    )
    .ok_or_else(|| format!("{name} declares no {}", member.as_str()))?;
    let declared = decode::declared(ty);
    if ty["kind"] == "raw" {
        return Err(format!(
            "{declared} is a raw type: it declares no structure to check bytes against"
        ));
    }
    let (conformance, value) = match decode::decode(bundle, ty, encoding, bytes) {
        decode::Rendered::Value(value) => {
            let conformance = if ty["kind"] == "jsonschema" {
                match zenkey_model::validate::validate(bundle, ty, &value) {
                    Ok(()) => Conformance::Valid,
                    Err(violations) => Conformance::Invalid {
                        violations: violations.iter().map(ToString::to_string).collect(),
                    },
                }
            } else {
                Conformance::Valid
            };
            (conformance, Some(value))
        }
        decode::Rendered::Undecodable {
            declared, reason, ..
        } => (Conformance::Undecodable { declared, reason }, None),
        decode::Rendered::Opaque { media_type, .. } => {
            return Err(format!(
                "{media_type} is a raw type: it declares no structure to check bytes against"
            ));
        }
    };
    Ok(crate::report::PayloadCheck {
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        resource: name,
        member: member.as_str().to_owned(),
        declared,
        encoding: encoding.map(str::to_owned),
        size: bytes.len(),
        conformance,
        value,
    })
}

/// The conformance of a payload already rendered through `revision` (a
/// `watch` sample, a `check expect` window): a decoded JSON Schema value is
/// validated against its type (§7.3), a decoded protobuf message is valid,
/// a raw type and a structural rendering are not checked, with why.
pub fn conformance(revision: &Revision, rendering: &PayloadRendering) -> Conformance {
    match &rendering.rendered {
        Rendered::Value { value, .. } => {
            let Some(res) = &rendering.resource else {
                return Conformance::NotChecked {
                    reason: "no resource resolved the key".into(),
                };
            };
            let (token, template) = res.resource.split_once('/').unwrap_or(("", ""));
            let bundle = revision.bundle();
            match decode::type_of(bundle, token, template, &res.member) {
                Some(ty) if ty["kind"] == "jsonschema" => {
                    match zenkey_model::validate::validate(bundle, ty, value) {
                        Ok(()) => Conformance::Valid,
                        Err(violations) => Conformance::Invalid {
                            violations: violations.iter().map(ToString::to_string).collect(),
                        },
                    }
                }
                Some(_) => Conformance::Valid,
                None => Conformance::NotChecked {
                    reason: format!("{} declares no {}", res.resource, res.member),
                },
            }
        }
        Rendered::Opaque { media_type } => Conformance::NotChecked {
            reason: format!("a raw type ({media_type}) declares no structure to check"),
        },
        Rendered::Undecodable { declared, reason } => Conformance::Undecodable {
            declared: declared.clone(),
            reason: reason.clone(),
        },
        Rendered::Structural { why, .. } => Conformance::NotChecked {
            reason: why.words(),
        },
    }
}

/// The structural ladder, with the reason it was reached.
fn structural_rendering(key: &str, why: Unresolved, bytes: &[u8]) -> PayloadRendering {
    let value = structural_value(bytes);
    let text = match &value {
        Some(v) => serde_json::to_string(v).unwrap_or_default(),
        None => structural(bytes),
    };
    PayloadRendering {
        key: key.to_owned(),
        size: bytes.len(),
        resource: None,
        rendered: Rendered::Structural { why, value, text },
    }
}

/// The QoS a stream, state or event resource declares (§2.4); `None` for
/// an operation, whose replies inherit the caller's.
pub fn declared_qos(r: &Resource) -> Option<QosAxes> {
    let Body::Data(d) = &r.body else {
        return None;
    };
    Some(QosAxes {
        priority: priority_name(d.priority).to_owned(),
        congestion: congestion_name(d.congestion).to_owned(),
        express: d.express,
    })
}

/// The axes a sample rode, in the contract's spelling.
pub fn observed_qos(
    priority: zenoh::qos::Priority,
    congestion: zenoh::qos::CongestionControl,
    express: bool,
) -> QosAxes {
    use zenoh::qos::{CongestionControl as Cc, Priority as P};
    QosAxes {
        priority: match priority {
            P::RealTime => "real_time",
            P::InteractiveHigh => "interactive_high",
            P::InteractiveLow => "interactive_low",
            P::DataHigh => "data_high",
            P::Data => "data",
            P::DataLow => "data_low",
            P::Background => "background",
        }
        .to_owned(),
        congestion: match congestion {
            Cc::Drop => "drop",
            Cc::Block => "block",
            // `#[non_exhaustive]` upstream: a variant this build has never
            // heard of is neither of the two a contract can declare.
            _ => "other",
        }
        .to_owned(),
        express,
    }
}

/// `declared` against `observed`: `None` when the three axes agree.
pub fn qos_mismatch(declared: &QosAxes, observed: &QosAxes) -> Option<QosMismatch> {
    let mut differs = Vec::new();
    if declared.priority != observed.priority {
        differs.push("priority".to_owned());
    }
    if declared.congestion != observed.congestion {
        differs.push("congestion".to_owned());
    }
    if declared.express != observed.express {
        differs.push("express".to_owned());
    }
    (!differs.is_empty()).then(|| QosMismatch {
        declared: declared.clone(),
        observed: observed.clone(),
        differs,
    })
}

fn priority_name(p: Priority) -> &'static str {
    match p {
        Priority::RealTime => "real_time",
        Priority::InteractiveHigh => "interactive_high",
        Priority::InteractiveLow => "interactive_low",
        Priority::DataHigh => "data_high",
        Priority::Data => "data",
        Priority::DataLow => "data_low",
        Priority::Background => "background",
    }
}

fn congestion_name(c: Congestion) -> &'static str {
    match c {
        Congestion::Drop => "drop",
        Congestion::Block => "block",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::catalog::{DescriptorRead, Observed};
    use crate::report::ContractSource;
    use serde_json::json;

    fn status() -> Revision {
        let dir = std::env::temp_dir().join(format!("zenkey-fleet-lens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("s.json"),
            r#"{"$defs": {"Status": {"type": "object", "properties": {"up": {"type": "boolean"}}, "required": ["up"]}}}"#,
        )
        .expect("write");
        let toml = "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n\
             [schemas]\njsonschema = [\"s.json\"]\n\
             [resources.\"status/{dev}\"]\nkind = \"state\"\ntype = \"json:Status\"\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
             [resources.frame]\nkind = \"stream\"\ntype = { raw = \"image/*\" }\npriority = \"data_low\"\n";
        let l = zenkey_model::contract::load_str(toml, &dir, None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        Revision::from_contract(c, ContractSource::File)
    }

    fn catalog(fp: &str, zid: Option<&str>) -> Catalog {
        let inst = "8f3a5c2e9b1d4f70";
        let mut o = Observed::from_keys(
            "zk2/*/*/@zk/**",
            &[format!("zk2/lab/m/@zk/instance/{inst}")],
            true,
        );
        let mut d = json!({
            "format": "zk2-descriptor/0.1",
            "service": "lab/m",
            "instance": inst,
            "interfaces": [{"iface": "m.v1", "contract": fp, "minor": 0}],
        });
        if let Some(z) = zid {
            d["meta"] = json!({"zid": z});
        }
        let descriptor: zenkey_model::descriptor::Descriptor =
            serde_json::from_value(d).expect("descriptor");
        o.descriptors = Some(BTreeMap::from([(
            ("lab/m".parse().expect("addr"), inst.parse().expect("id")),
            DescriptorRead::Served(Box::new(descriptor)),
        )]));
        Catalog::new(&o)
    }

    /// Every rung keeps its own spelling: outside the namespace, not zk2, a
    /// control key, no presence read, no provider, resolved.
    #[test]
    fn the_ladder_stops_where_it_stops_and_says_so() {
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let mut set = ContractSet::new();
        set.insert(rev);
        let cat = catalog(&fp, Some("0a1b"));

        let lens = Lens::new("prod", Some(&cat), &set);
        let out = lens.identity("staging/zk2/lab/m/m.v1/state/status/eth0");
        assert_eq!(out.group, KeyGroup::NotInNamespace);
        assert_eq!(
            out.unresolved,
            Some(Unresolved::NotInNamespace {
                namespace: "prod".into()
            })
        );
        let foreign = lens.identity("prod/v1/h-3fa9c2d41b7e/state/x");
        assert_eq!(foreign.group, KeyGroup::NotZk2);
        let ctl = lens.identity("prod/zk2/lab/m/@zk/instance/8f3a5c2e9b1d4f70");
        assert_eq!(
            ctl.group,
            KeyGroup::Control {
                address: Some("lab/m".into()),
                form: "instance".into()
            }
        );
        assert_eq!(ctl.unresolved, None);

        let resolved = lens.resolve("prod/zk2/lab/m/m.v1/state/status/eth0");
        let r = resolved.resolved.expect("resolved");
        assert_eq!(r.resource_name(), "state/status/{dev}");
        assert_eq!(r.key, "zk2/lab/m/m.v1/state/status/eth0");
        assert_eq!(resolved.identity.values["dev"], ["eth0"]);

        let other = lens.identity("prod/zk2/lab/n/m.v1/state/status/eth0");
        assert_eq!(other.unresolved, Some(Unresolved::NoProvider));

        let blind = Lens::new("prod", None, &set);
        assert_eq!(
            blind
                .identity("prod/zk2/lab/m/m.v1/state/status/eth0")
                .unresolved,
            Some(Unresolved::PresenceNotRead)
        );
        let offline = Lens::new("prod", None, &set).offline(&set);
        assert!(
            offline
                .resolve("prod/zk2/lab/m/m.v1/state/status/eth0")
                .resolved
                .is_some(),
            "one revision held offline resolves without presence"
        );
    }

    /// A value that decodes and validates, one that decodes and does not,
    /// bytes that do not decode, and a raw type: four conformances.
    #[test]
    fn a_payload_is_checked_against_its_declared_type() {
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let mut set = ContractSet::new();
        set.insert(rev);
        let cat = catalog(&fp, None);
        let lens = Lens::new("", Some(&cat), &set);
        let key = "zk2/lab/m/m.v1/state/status/eth0";
        let ok = lens.check(
            key,
            Member::Type,
            Some("application/json"),
            br#"{"up":true}"#,
        );
        assert_eq!(ok.conformance, Conformance::Valid);
        let bad = lens.check(key, Member::Type, Some("application/json"), br#"{"up":1}"#);
        assert!(
            matches!(&bad.conformance, Conformance::Invalid { violations } if !violations.is_empty()),
            "{:?}",
            bad.conformance
        );
        let junk = lens.check(key, Member::Type, Some("application/json"), b"{not");
        assert!(matches!(junk.conformance, Conformance::Undecodable { .. }));
        let raw = lens.check(
            "zk2/lab/m/m.v1/stream/frame",
            Member::Type,
            Some("image/png"),
            b"\x89PNG",
        );
        assert!(matches!(raw.conformance, Conformance::NotChecked { .. }));
        let foreign = lens.check("v1/x", Member::Type, None, b"42");
        assert!(matches!(
            foreign.rendering.rendered,
            Rendered::Structural {
                why: Unresolved::NotZk2 { .. },
                ..
            }
        ));
    }

    /// The declared QoS, and a sample that rode another: the axes that
    /// differ, named.
    #[test]
    fn qos_is_compared_axis_by_axis() {
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let mut set = ContractSet::new();
        set.insert(rev);
        let cat = catalog(&fp, None);
        let lens = Lens::new("", Some(&cat), &set);
        let declared = lens
            .declared_qos("zk2/lab/m/m.v1/stream/frame")
            .expect("a stream declares its QoS");
        assert_eq!(declared.priority, "data_low");
        let same = observed_qos(
            zenoh::qos::Priority::DataLow,
            zenoh::qos::CongestionControl::Drop,
            false,
        );
        assert_eq!(qos_mismatch(&declared, &same), None);
        let other = observed_qos(
            zenoh::qos::Priority::RealTime,
            zenoh::qos::CongestionControl::Drop,
            true,
        );
        let m = qos_mismatch(&declared, &other).expect("a mismatch");
        assert_eq!(m.differs, ["priority", "express"]);
    }

    /// An event key resolves to its template with its ULID chunk stripped
    /// (§2.6), through the lens and through `render_with` alike (#702).
    #[test]
    fn an_event_key_resolves_without_its_ulid() {
        let l = zenkey_model::contract::load_str(
            "[interface]\nname = \"m\"\nmajor = 1\n\
             [resources.\"link/{iface}\"]\nkind = \"event\"\ntype = { raw = \"text/plain\" }\n\
             params = { iface = \"string\" }\ncardinality = 8\nrate = \"rare\"\nretention = \"1d\"\n",
            std::path::Path::new("."),
            None,
        );
        let rev = Revision::from_contract(
            l.contract.unwrap_or_else(|| panic!("{}", l.report)),
            crate::report::ContractSource::File,
        );
        let fp = rev.fingerprint().to_string();
        let mut set = ContractSet::new();
        set.insert(rev.clone());
        let cat = catalog(&fp, None);
        let key = "zk2/lab/m/m.v1/events/link/eth0/01jqz3m6v2b8d9e0f1g2h3j4k5";
        let r = Lens::new("", Some(&cat), &set)
            .resolve(key)
            .resolved
            .expect("an event key resolves");
        assert_eq!(r.resource_name(), "events/link/{iface}");
        assert_eq!(r.values["iface"], ["eth0"]);
        let rendered = crate::model::render::render_with(&rev, key, Member::Type, None, b"up");
        assert_eq!(
            rendered.resource.expect("resolved").resource,
            "events/link/{iface}"
        );
    }

    /// Whose clock: the owner's by value, another's, or nobody named one.
    #[test]
    fn a_stamp_is_attributed_to_its_owner_by_value() {
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let set = ContractSet::new();
        let named = catalog(&fp, Some("0A1B"));
        let lens = Lens::new("", Some(&named), &set);
        assert_eq!(lens.provenance(Some("lab/m"), "a1b"), Provenance::Owner);
        assert_eq!(lens.provenance(Some("lab/m"), "ffff"), Provenance::Other);
        assert_eq!(lens.provenance(None, "a1b"), Provenance::Unattributable);
        let unnamed = catalog(&fp, None);
        let lens = Lens::new("", Some(&unnamed), &set);
        assert_eq!(
            lens.provenance(Some("lab/m"), "a1b"),
            Provenance::Unattributable,
            "no meta.zid: unattributable, never foreign"
        );
    }
}
