//! A zk2 sample, rendered through the contract in hand (spec §7.2), for a
//! tool that was never compiled against it.
//!
//! The decode order is the sample's `Encoding`, then the contract's type,
//! then sniffing. [`render_with`] decodes through a [`Revision`] with
//! `zenkey_model::decode`, which owns the codecs (protobuf through the
//! bundle's descriptor set, JSON Schema types as JSON or CBOR, raw types as
//! their media type and size). [`render`] first finds the revision: the
//! key names a provider and an interface, the [`Catalog`] says which
//! revision that provider's descriptors name, and [`Contracts`] says whether
//! it is in hand.
//!
//! **Honest rendering** (§7.2, `[Sc: types.md §2]`): whatever stops the
//! decode becomes the reason on a structural rendering
//! ([`crate::model::structural`]), never an empty one and never a sniffed
//! document dressed as a decoded one. A key that is not zk2's is rendered
//! the same way, because the bus is shared (§1.7).
//!
//! Session-free: the samples come from wherever the caller got them — a
//! live subscriber, a `.zrec`, a test.

use std::str::FromStr;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::decode::{self, Rendered as Decoded};
use zenkey_model::grammar::{IfaceId, ZkKey};
use zenkey_model::template::resolve;

use crate::model::catalog::{Catalog, ContractState, Contracts, Revision};
use crate::model::structural::{structural, structural_value};
use crate::report::{PayloadRendering, Rendered, ResolvedResource, Unresolved};

/// Which member of a resource a payload is (§7.1). A sample on a data key
/// is its `type` or its `attachment`; on an `@op` key, a query carries the
/// `request` and a reply the `response`, an `error` or a `summary`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Member {
    Type,
    Attachment,
    Request,
    Response,
    Error,
    Summary,
}

impl Member {
    /// The member's name in the canonical contract.
    pub fn as_str(self) -> &'static str {
        match self {
            Member::Type => "type",
            Member::Attachment => "attachment",
            Member::Request => "request",
            Member::Response => "response",
            Member::Error => "error",
            Member::Summary => "summary",
        }
    }
}

/// Renders a sample through the catalog: the key's provider and interface
/// pick the revision its descriptors name, and `contracts` holds it or
/// says why not. `encoding` is the sample's `Encoding` as zenoh spells it.
pub fn render(
    catalog: &Catalog,
    contracts: &dyn Contracts,
    key: &str,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
) -> PayloadRendering {
    let fallback = |why| structural_rendering(key, why, bytes);
    let (addr, iface) = match zenkey_model::grammar::parse(key) {
        Ok(ZkKey::Data { addr, iface, .. }) => (addr, iface),
        Ok(_) => {
            return fallback(Unresolved::NotZk2 {
                detail: "a control key, not a data key".to_owned(),
            });
        }
        Err(e) => {
            return fallback(Unresolved::NotZk2 {
                detail: e.to_string(),
            });
        }
    };
    let fps = catalog.revisions_of(&addr, &iface);
    let fp = match fps.len() {
        0 if catalog.provides(&addr, &iface) => return fallback(Unresolved::NoRevision),
        0 => return fallback(Unresolved::NoProvider),
        1 => fps.into_iter().next().expect("one revision"),
        _ => {
            return fallback(Unresolved::Ambiguous {
                fingerprints: fps.iter().map(ToString::to_string).collect(),
            });
        }
    };
    match contracts.state(&iface, &fp) {
        Some(ContractState::Held(revision)) => render_with(&revision, key, member, encoding, bytes),
        None => fallback(Unresolved::ContractNotHeld {
            fingerprint: fp.to_string(),
        }),
        Some(ContractState::Unavailable { refused }) => fallback(Unresolved::ContractUnavailable {
            fingerprint: fp.to_string(),
            refused,
        }),
        Some(ContractState::Unreadable { reason }) => fallback(Unresolved::ContractUnreadable {
            fingerprint: fp.to_string(),
            detail: reason,
        }),
    }
}

/// Renders a sample through one revision in hand (§7.2): the key resolves
/// to a resource by the most-literal-first rule (D1), the member names the
/// type, and `zenkey_model::decode` decodes or renders it honestly.
pub fn render_with(
    revision: &Revision,
    key: &str,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
) -> PayloadRendering {
    let fingerprint = revision.fingerprint().to_string();
    let fallback = |why| structural_rendering(key, why, bytes);
    let (iface, kind, chunks) = match zenkey_model::grammar::parse(key) {
        Ok(ZkKey::Data {
            iface,
            kind,
            resource,
            ..
        }) => (iface, kind, resource),
        Ok(_) => {
            return fallback(Unresolved::NotZk2 {
                detail: "a control key, not a data key".to_owned(),
            });
        }
        Err(e) => {
            return fallback(Unresolved::NotZk2 {
                detail: e.to_string(),
            });
        }
    };
    let contract = revision.contract();
    if &iface != revision.iface() {
        return fallback(Unresolved::NoResource { fingerprint });
    }
    let candidates: Vec<_> = contract
        .resources
        .iter()
        .filter(|r| r.token == kind)
        .collect();
    let refs: Vec<&str> = crate::model::lens::template_chunks(kind, &chunks)
        .iter()
        .map(String::as_str)
        .collect();
    let Some((i, values)) = resolve(candidates.iter().map(|r| &r.template), &refs) else {
        return fallback(Unresolved::NoResource { fingerprint });
    };
    let r = candidates[i];
    let resource = zenkey::implementation::resource_name(r);
    let Some(ty) = decode::type_of(
        revision.bundle(),
        kind.as_str(),
        r.template.as_str(),
        member.as_str(),
    ) else {
        return fallback(Unresolved::NoMember {
            resource,
            member: member.as_str().to_owned(),
        });
    };
    // zenoh's default, `zenoh/bytes`, is "unsaid", not "bytes on purpose":
    // the contract speaks next (§7.2's order).
    let encoding = encoding.filter(|e| *e != "zenoh/bytes");
    let rendered = rendered(ty, decode::decode(revision.bundle(), ty, encoding, bytes));
    PayloadRendering {
        key: key.to_owned(),
        size: bytes.len(),
        resource: Some(ResolvedResource {
            iface: iface.to_string(),
            fingerprint,
            resource,
            member: member.as_str().to_owned(),
            values,
        }),
        rendered,
    }
}

/// Renders a payload of `member` of `r`, a resource of `revision` already
/// in hand, with the template `values` its key bound (#612, FJ8a). What a
/// server knows of the call it serves: the key may be a fan-out's selector,
/// which [`render_with`]'s resolution cannot read, and the resource is
/// known without it.
pub fn render_resource(
    revision: &Revision,
    r: &zenkey_model::contract::Resource,
    key: &str,
    member: Member,
    values: zenkey_model::template::Bindings,
    encoding: Option<&str>,
    bytes: &[u8],
) -> PayloadRendering {
    let fingerprint = revision.fingerprint().to_string();
    let resource = zenkey::implementation::resource_name(r);
    let Some(ty) = decode::type_of(
        revision.bundle(),
        r.token.as_str(),
        r.template.as_str(),
        member.as_str(),
    ) else {
        return structural_rendering(
            key,
            Unresolved::NoMember {
                resource,
                member: member.as_str().to_owned(),
            },
            bytes,
        );
    };
    let encoding = encoding.filter(|e| *e != "zenoh/bytes");
    PayloadRendering {
        key: key.to_owned(),
        size: bytes.len(),
        resource: Some(ResolvedResource {
            iface: revision.iface().to_string(),
            fingerprint,
            resource,
            member: member.as_str().to_owned(),
            values,
        }),
        rendered: rendered(ty, decode::decode(revision.bundle(), ty, encoding, bytes)),
    }
}

/// The model's decode, as the report spells it.
fn rendered(ty: &serde_json::Value, d: Decoded) -> Rendered {
    match d {
        Decoded::Value(value) => Rendered::Value {
            declared: decode::declared(ty),
            value,
        },
        Decoded::Opaque { media_type, .. } => Rendered::Opaque { media_type },
        Decoded::Undecodable {
            declared, reason, ..
        } => Rendered::Undecodable { declared, reason },
    }
}

/// An `app` envelope's detail (§5.2), rendered through operation `r`'s
/// declared `error` type: a JSON or CBOR envelope's value as carried, a
/// protobuf envelope's bytes decoded as the message, a raw type's base64
/// text shown as its media type. An operation that declares no `error`
/// type has no detail to send, and one that arrives anyway is rendered
/// structurally, saying so.
pub fn render_detail(
    revision: &Revision,
    r: &zenkey_model::contract::Resource,
    detail: &zenkey_model::envelope::Detail,
) -> Rendered {
    use zenkey_model::envelope::Detail;
    let ty = decode::type_of(
        revision.bundle(),
        r.token.as_str(),
        r.template.as_str(),
        Member::Error.as_str(),
    );
    match (ty, detail) {
        (Some(ty), Detail::Bytes(bytes)) => rendered(
            ty,
            decode::decode(revision.bundle(), ty, Some("application/protobuf"), bytes),
        ),
        (Some(ty), Detail::Value(_)) if ty["kind"] == "raw" => Rendered::Opaque {
            media_type: decode::declared(ty),
        },
        (Some(ty), Detail::Value(value)) => Rendered::Value {
            declared: decode::declared(ty),
            value: value.clone(),
        },
        (None, detail) => {
            let why = Unresolved::NoMember {
                resource: zenkey::implementation::resource_name(r),
                member: Member::Error.as_str().to_owned(),
            };
            match detail {
                Detail::Value(v) => Rendered::Structural {
                    why,
                    value: Some(v.clone()),
                    text: serde_json::to_string(v).unwrap_or_default(),
                },
                Detail::Bytes(b) => Rendered::Structural {
                    why,
                    value: structural_value(b),
                    text: structural(b),
                },
            }
        }
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

/// The interface and fingerprint a resolved rendering decoded against, for
/// a caller that keeps a per-revision cache of its own.
pub fn resolved_revision(r: &PayloadRendering) -> Option<(IfaceId, Fingerprint)> {
    let res = r.resource.as_ref()?;
    Some((
        IfaceId::from_str(&res.iface).ok()?,
        Fingerprint::parse(&res.fingerprint).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::model::catalog::{ContractSet, DescriptorRead, Observed};
    use crate::report::ContractSource;
    use serde_json::json;

    fn revision(toml: &str, files: &[(&str, &str)]) -> Revision {
        let dir = std::env::temp_dir().join(format!(
            "zenkey-fleet-render-{}-{}",
            std::process::id(),
            toml.len()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (name, text) in files {
            std::fs::write(dir.join(name), text).expect("write");
        }
        let l = zenkey_model::contract::load_str(toml, &dir, None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        Revision::from_contract(c, ContractSource::File)
    }

    fn status() -> Revision {
        revision(
            "[interface]\nname = \"m\"\nmajor = 1\nminor = 0\n\
             [schemas]\njsonschema = [\"s.json\"]\n\
             [resources.\"status/{dev}\"]\nkind = \"state\"\ntype = \"json:Status\"\nencoding = \"cbor\"\n\
             params = { dev = \"string\" }\ncardinality = 8\n\
             [resources.frame]\nkind = \"stream\"\ntype = { raw = \"image/*\" }\n",
            &[(
                "s.json",
                r#"{"$defs": {"Status": {"type": "object", "properties": {"up": {"type": "boolean"}}}}}"#,
            )],
        )
    }

    /// The key picks the resource and its parameters; the contract's
    /// encoding speaks when the sample says nothing (`zenoh/bytes`).
    #[test]
    fn a_json_schema_type_decodes_through_the_contract() {
        let rev = status();
        let mut cbor = Vec::new();
        ciborium::into_writer(&json!({"up": true}), &mut cbor).expect("cbor");
        let r = render_with(
            &rev,
            "zk2/lab/m/m.v1/state/status/eth0",
            Member::Type,
            Some("zenoh/bytes"),
            &cbor,
        );
        assert_eq!(
            r.rendered,
            Rendered::Value {
                declared: "json:Status".into(),
                value: json!({"up": true})
            }
        );
        let res = r.resource.expect("resolved");
        assert_eq!(res.resource, "state/status/{dev}");
        assert_eq!(res.values["dev"], ["eth0"]);
        assert_eq!(
            resolved_revision(&PayloadRendering {
                resource: Some(res),
                ..r
            }),
            Some(("m.v1".parse().expect("iface"), rev.fingerprint().clone()))
        );
    }

    /// Raw is its media type and size; bytes that do not decode as their
    /// type say so; a member the resource lacks is a reason, not a guess.
    #[test]
    fn honest_renderings() {
        let rev = status();
        let r = render_with(
            &rev,
            "zk2/lab/m/m.v1/stream/frame",
            Member::Type,
            Some("image/jpeg"),
            &[0xff, 0xd8],
        );
        assert_eq!(
            r.rendered,
            Rendered::Opaque {
                media_type: "image/jpeg".into()
            }
        );
        assert_eq!(r.size, 2);

        let r = render_with(
            &rev,
            "zk2/lab/m/m.v1/state/status/eth0",
            Member::Type,
            Some("application/json"),
            b"{not json",
        );
        assert!(
            matches!(&r.rendered, Rendered::Undecodable { declared, .. } if declared.starts_with("json:Status")),
            "{:?}",
            r.rendered
        );

        let r = render_with(
            &rev,
            "zk2/lab/m/m.v1/stream/frame",
            Member::Attachment,
            None,
            b"x",
        );
        assert!(matches!(
            r.rendered,
            Rendered::Structural {
                why: Unresolved::NoMember { .. },
                ..
            }
        ));
        let r = render_with(
            &rev,
            "zk2/lab/m/m.v1/stream/nothing",
            Member::Type,
            None,
            b"x",
        );
        assert!(matches!(
            r.rendered,
            Rendered::Structural {
                why: Unresolved::NoResource { .. },
                ..
            }
        ));
    }

    /// Through the catalog: each reason the revision is not in hand is
    /// named, and a foreign key is rendered structurally, not refused.
    #[test]
    fn the_catalog_picks_the_revision_or_says_why_not() {
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let inst = "8f3a5c2e9b1d4f70";
        let mut o = Observed::from_keys(
            "zk2/*/*/@zk/**",
            &[format!("zk2/lab/m/@zk/instance/{inst}")],
            true,
        );
        let descriptor: zenkey_model::descriptor::Descriptor = serde_json::from_value(json!({
            "format": "zk2-descriptor/0.1",
            "service": "lab/m",
            "instance": inst,
            "interfaces": [{"iface": "m.v1", "contract": fp, "minor": 0}],
        }))
        .expect("descriptor");
        o.descriptors = Some(BTreeMap::from([(
            ("lab/m".parse().expect("addr"), inst.parse().expect("id")),
            DescriptorRead::Served(Box::new(descriptor)),
        )]));
        let catalog = Catalog::new(&o);
        let key = "zk2/lab/m/m.v1/state/status/eth0";

        let r = render(
            &catalog,
            &ContractSet::new(),
            key,
            Member::Type,
            None,
            b"{}",
        );
        assert!(matches!(
            r.rendered,
            Rendered::Structural {
                why: Unresolved::ContractNotHeld { .. },
                ..
            }
        ));

        let mut set = ContractSet::new();
        set.insert(rev);
        let r = render(
            &catalog,
            &set,
            key,
            Member::Type,
            Some("application/json"),
            br#"{"up":false}"#,
        );
        assert_eq!(
            r.rendered,
            Rendered::Value {
                declared: "json:Status".into(),
                value: json!({"up": false})
            }
        );

        let r = render(
            &catalog,
            &set,
            "zk2/lab/other/m.v1/state/status/eth0",
            Member::Type,
            None,
            b"{}",
        );
        assert!(matches!(
            r.rendered,
            Rendered::Structural {
                why: Unresolved::NoProvider,
                ..
            }
        ));

        let r = render(
            &catalog,
            &set,
            "v1/h-3fa9c2d41b7e/state/x",
            Member::Type,
            None,
            b"42",
        );
        match r.rendered {
            Rendered::Structural {
                why: Unresolved::NotZk2 { .. },
                value,
                text,
            } => {
                assert_eq!(value, Some(json!(42)));
                assert_eq!(text, "42");
            }
            other => panic!("{other:?}"),
        }
    }
}
