//! A zk2 sample, rendered through the contract in hand (spec §7.2), for a
//! tool that was never compiled against it.
//!
//! The decode order: a payload whose declared type the tool holds is
//! decoded by that type, on the wire its sample's `Encoding` names, and any
//! other payload is sniffed (observer §7.3). [`render_with`] decodes through a [`Revision`] with
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
use zenkey_model::wire::{self, Reading};

use crate::model::catalog::{Catalog, ContractState, Contracts, Revision};
use crate::model::deployment::Epoch;
use crate::model::inband::{self, Payload};
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
///
/// The envelope first (core §7.5, observer §7.3): a sample of a type
/// member carrying a zk2c envelope renders its record, and a refused one is
/// undecodable, its refusal named, never sniffed
/// ([`crate::model::inband`]).
pub fn render(
    catalog: &Catalog,
    contracts: &dyn Contracts,
    key: &str,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
) -> PayloadRendering {
    let payload = read(key, member, encoding, bytes);
    let fallback = |why| fallback_rendering(key, why, payload, bytes.len());
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
        Some(ContractState::Held(revision)) => render_read(
            &revision,
            key,
            member,
            payload.encoding(encoding),
            payload,
            bytes.len(),
            Epoch::Assumed,
        ),
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
/// type, and `zenkey_model::decode` decodes or renders it honestly. The
/// envelope first, as [`render`] reads it. A packed record's `time` fields
/// are read under an epoch assumed and said so: [`render_with_in`] reads
/// them under the epoch in force.
pub fn render_with(
    revision: &Revision,
    key: &str,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
) -> PayloadRendering {
    render_with_in(revision, key, member, encoding, bytes, Epoch::Assumed)
}

/// [`render_with`], a packed record's `time` fields read under `epoch`, the
/// epoch in force for the key's owner (core §8.6,
/// [`crate::model::deployment::Catalogs::epoch`]).
pub fn render_with_in(
    revision: &Revision,
    key: &str,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
    epoch: Epoch,
) -> PayloadRendering {
    let payload = read(key, member, encoding, bytes);
    render_read(
        revision,
        key,
        member,
        payload.encoding(encoding),
        payload,
        bytes.len(),
        epoch,
    )
}

/// A record the runtime already read out of its envelope (a consumer's
/// `Delivery`, a `StateAnswer`'s value), rendered through one revision in
/// hand: its run and seq and CH's tag in hand, so its bytes are never
/// stripped twice and a packed type is decoded by its tag (core §7.6), its
/// `time` fields under `epoch` (§8.6).
pub fn render_record(
    revision: &Revision,
    key: &str,
    record: &[u8],
    stamp: zenkey_model::wire::Stamp,
    tag: zenkey_model::wire::Tag,
    epoch: Epoch,
) -> PayloadRendering {
    let payload = Payload::Record { record, stamp, tag };
    render_read(
        revision,
        key,
        Member::Type,
        None,
        payload,
        record.len(),
        epoch,
    )
}

/// A frame the runtime already read out of its envelope (a consumer's
/// refusal of one), rendered through one revision in hand as core §7.7
/// reads it, alone (observer §7.6): its body read with the frame
/// resource's layout under CH's tag, its record type's `time` fields under
/// `epoch` (§8.6), or undecodable with why.
pub fn render_frame(
    revision: &Revision,
    key: &str,
    body: &[u8],
    stamp: zenkey_model::wire::Stamp,
    tag: zenkey_model::wire::Tag,
    epoch: Epoch,
) -> PayloadRendering {
    let payload = Payload::Frame { body, stamp, tag };
    render_read(
        revision,
        key,
        Member::Type,
        None,
        payload,
        body.len(),
        epoch,
    )
}

/// A payload of `member` on `key`, its envelope read: a type member's
/// (core §7.5), and a reply's, the `response` or the `summary`, as the
/// value form under `zenoh/bytes` and a bare record under any other
/// `Encoding` (observer §7.3, "A call's forms first"; core §5.1, "The bare
/// wire"). An attachment carries none, and a request is read bare until
/// callers write its form (#980's C4c).
fn read<'a>(key: &str, member: Member, encoding: Option<&str>, bytes: &'a [u8]) -> Payload<'a> {
    match (member, encoding) {
        (Member::Type, _) => inband::payload(key, encoding, bytes),
        (Member::Response | Member::Summary, Some(e)) => match wire::read_value(e, bytes) {
            Reading::Enveloped(v) => Payload::Record {
                record: v.record,
                stamp: v.stamp,
                tag: v.tag,
            },
            Reading::Refused(r) => Payload::Refused(r),
            _ => Payload::Plain(bytes),
        },
        _ => Payload::Plain(bytes),
    }
}

/// What a payload that resolved no type renders as: the structural ladder
/// over its bytes (an envelope's record), a refused envelope undecodable.
fn fallback_rendering(
    key: &str,
    why: Unresolved,
    payload: Payload<'_>,
    size: usize,
) -> PayloadRendering {
    match payload {
        Payload::Refused(refusal) => inband::refused_rendering(key, size, None, None, refusal),
        Payload::Heartbeat { body, .. } => inband::heartbeat_rendering(key, why, body.len()),
        Payload::Frame { body, stamp, .. } => {
            crate::model::frame::unread_rendering(key, why, stamp, body.len())
        }
        Payload::Plain(b) | Payload::Record { record: b, .. } => structural_rendering(key, why, b),
    }
}

/// [`render_with`] over a payload already read: `size` is the bytes' as
/// they arrived, which a refused envelope's rendering states.
fn render_read(
    revision: &Revision,
    key: &str,
    member: Member,
    encoding: Option<&str>,
    payload: Payload<'_>,
    size: usize,
    epoch: Epoch,
) -> PayloadRendering {
    let fingerprint = revision.fingerprint().to_string();
    let fallback = |why| fallback_rendering(key, why, payload, size);
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
    let resource = ResolvedResource {
        iface: iface.to_string(),
        fingerprint,
        resource,
        member: member.as_str().to_owned(),
        values,
    };
    // Core §7.7: a frame resource's put is a frame, read alone here, as a
    // reader's first (observer §7.6); a frame anywhere else, and a
    // mutation on a frame resource, have no reading.
    if member == Member::Type
        && let Some(framed) = frame_rendering(
            contract,
            r,
            key,
            Some(resource.clone()),
            ty,
            payload,
            size,
            epoch,
        )
    {
        return framed;
    }
    let (bytes, tag) = match payload {
        Payload::Refused(refusal) => {
            return inband::refused_rendering(
                key,
                size,
                Some(resource),
                Some(decode::declared(ty)),
                refusal,
            );
        }
        Payload::Plain(b) => (b, None),
        Payload::Record { record, tag, .. } => (record, Some(tag)),
        // A frame's every case is [`frame_rendering`]'s, answered above.
        Payload::Frame { .. } => {
            return crate::model::frame::undecodable(
                key,
                Some(resource),
                decode::declared(ty),
                size,
                crate::model::frame::FRAME_AS_RECORD.to_owned(),
            )
            .0;
        }
        // A heartbeat on a key whose resource has a type (core §7.5's form
        // 6): a body, never a record of the type.
        Payload::Heartbeat { .. } => {
            return PayloadRendering {
                key: key.to_owned(),
                size,
                resource: Some(resource),
                rendered: Rendered::Undecodable {
                    declared: decode::declared(ty),
                    reason: crate::model::inband::HEARTBEAT_AS_RECORD.to_owned(),
                },
            };
        }
    };
    // zenoh's default, `zenoh/bytes`, is "unsaid", not "bytes on purpose":
    // the contract speaks next (§7.2's order). A packed record is read by
    // its envelope's tag (core §7.6), its `time` fields under the epoch in
    // force (§8.6), or one assumed and said so.
    let encoding = encoding.filter(|e| *e != "zenoh/bytes");
    let (decoded, epoch_assumed) = decode_under(revision.bundle(), ty, encoding, bytes, tag, epoch);
    let rendered = rendered(ty, decoded, epoch_assumed);
    PayloadRendering {
        key: key.to_owned(),
        size: bytes.len(),
        resource: Some(resource),
        rendered,
    }
}

/// A frame resource's payload, or a frame on any resource, as core §7.7
/// reads it: a frame on a frame resource read alone, by a reader that held
/// nothing (observer §7.6), and undecodable on a resource with no `frame`
/// table; a mutation, or a sample with no envelope, on a frame resource
/// undecodable. `None` for any other payload, which the type reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_rendering(
    contract: &zenkey_model::contract::Contract,
    r: &zenkey_model::contract::Resource,
    key: &str,
    resource: Option<ResolvedResource>,
    ty: &serde_json::Value,
    payload: Payload<'_>,
    size: usize,
    epoch: Epoch,
) -> Option<PayloadRendering> {
    frame_check(
        contract,
        r,
        key,
        resource,
        ty,
        payload,
        size,
        &mut zenkey_model::frame::Reader::new(),
        epoch,
    )
    .map(|(r, _)| r)
}

/// [`frame_rendering`] with the key's reader, and its conformance.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_check(
    contract: &zenkey_model::contract::Contract,
    r: &zenkey_model::contract::Resource,
    key: &str,
    resource: Option<ResolvedResource>,
    ty: &serde_json::Value,
    payload: Payload<'_>,
    size: usize,
    reader: &mut zenkey_model::frame::Reader,
    epoch: Epoch,
) -> Option<(PayloadRendering, crate::report::Conformance)> {
    use crate::model::frame;
    let layout = zenkey_model::frame::Layout::of_resource(contract, r);
    let declared = decode::declared(ty);
    match (payload, layout) {
        (Payload::Frame { body, stamp, tag }, Ok(Some(l))) => Some(frame::frame_check(
            key,
            resource,
            frame::declared(&declared),
            &l,
            stamp,
            tag,
            body,
            reader,
            epoch,
        )),
        (Payload::Frame { .. }, Ok(None)) => Some(frame::undecodable(
            key,
            resource,
            declared,
            size,
            frame::FRAME_AS_RECORD.to_owned(),
        )),
        (Payload::Frame { .. }, Err(why))
        | (Payload::Plain(_) | Payload::Record { .. }, Err(why)) => Some(frame::undecodable(
            key,
            resource,
            frame::declared(&declared),
            size,
            format!("the frame resource's layout does not read: {why}"),
        )),
        (Payload::Plain(_) | Payload::Record { .. }, Ok(Some(_))) => Some(frame::undecodable(
            key,
            resource,
            frame::declared(&declared),
            size,
            frame::MUTATION_ON_FRAME.to_owned(),
        )),
        _ => None,
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
        rendered: rendered(
            ty,
            decode::decode(revision.bundle(), ty, encoding, bytes),
            None,
        ),
    }
}

/// `zenkey_model::decode` of `ty` under the epoch in force (core §7.6,
/// §8.6): a record holding a `time` field whose epoch the reader refuses is
/// undecodable, the refusal its reason, never dated by a guess; any other
/// is decoded under the epoch given, with the epoch assumed where none was
/// ([`decode::epoch_assumed`]).
pub(crate) fn decode_under(
    bundle: &zenkey_model::bundle::Bundle,
    ty: &serde_json::Value,
    encoding: Option<&str>,
    bytes: &[u8],
    tag: Option<zenkey_model::wire::Tag>,
    epoch: Epoch,
) -> (Decoded, Option<i64>) {
    let bare = decode::Context { tag, epoch: None };
    if let Some(reason) = epoch.refusal()
        && decode::epoch_assumed(bundle, ty, bare).is_some()
    {
        let declared = decode::declared(ty);
        let size = bytes.len();
        return (
            Decoded::Undecodable {
                declared,
                size,
                reason,
            },
            None,
        );
    }
    let cx = decode::Context {
        tag,
        epoch: epoch.given(),
    };
    (
        decode::decode_in(bundle, ty, encoding, bytes, cx),
        decode::epoch_assumed(bundle, ty, cx),
    )
}

/// The model's decode, as the report spells it, with the epoch a packed
/// record's `time` fields were read under when none was stated
/// ([`decode::epoch_assumed`]).
fn rendered(ty: &serde_json::Value, d: Decoded, epoch_assumed: Option<i64>) -> Rendered {
    match d {
        Decoded::Value(value) => Rendered::Value {
            declared: decode::declared(ty),
            value,
            epoch_assumed,
        },
        Decoded::Opaque { media_type, .. } => Rendered::Opaque { media_type },
        Decoded::Undecodable {
            declared, reason, ..
        } => Rendered::Undecodable { declared, reason },
    }
}

/// An `app` error's detail (§5.2), rendered through operation `r`'s
/// declared `error` type. From an error form (`form` its CH tag): the
/// type's own bytes, decoded as the type says on the contract's
/// `encoding`, a packed one under that tag, a raw one shown as its media
/// type. From the bare wire's envelope (`form` `None`): a JSON or CBOR
/// envelope's value as carried, a protobuf envelope's bytes decoded as the
/// message, a raw type's base64 text shown as its media type. An operation
/// that declares no `error` type has no detail to send, and one that
/// arrives anyway is rendered structurally, saying so.
pub fn render_detail(
    revision: &Revision,
    r: &zenkey_model::contract::Resource,
    detail: &zenkey_model::envelope::Detail,
    form: Option<zenkey_model::wire::Tag>,
) -> Rendered {
    use zenkey_model::envelope::Detail;
    let ty = decode::type_of(
        revision.bundle(),
        r.token.as_str(),
        r.template.as_str(),
        Member::Error.as_str(),
    );
    match (ty, detail) {
        (Some(ty), Detail::Bytes(bytes)) if form.is_some() => {
            let (decoded, assumed) =
                decode_under(revision.bundle(), ty, None, bytes, form, Epoch::Assumed);
            rendered(ty, decoded, assumed)
        }
        (Some(ty), Detail::Bytes(bytes)) => rendered(
            ty,
            decode::decode(revision.bundle(), ty, Some("application/protobuf"), bytes),
            None,
        ),
        (Some(ty), Detail::Value(_)) if ty["kind"] == "raw" => Rendered::Opaque {
            media_type: decode::declared(ty),
        },
        (Some(ty), Detail::Value(value)) => Rendered::Value {
            declared: decode::declared(ty),
            value: value.clone(),
            epoch_assumed: None,
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

    /// A directory per call (#766), not per contract: the tests build the
    /// same contract at once, and one truncating a schema file while
    /// another read it made the reader see half a file. The loaded
    /// contract carries its schemas, so the directory goes as soon as the
    /// load returns.
    fn revision(toml: &str, files: &[(&str, &str)]) -> Revision {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zenkey-fleet-render-{}-{}",
            std::process::id(),
            NTH.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (name, text) in files {
            std::fs::write(dir.join(name), text).expect("write");
        }
        let l = zenkey_model::contract::load_str(toml, &dir, None);
        std::fs::remove_dir_all(&dir).expect("remove the temp dir");
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
    /// encoding speaks when the sample says nothing: a zk2c envelope's
    /// record, under `zenoh/bytes` (core §7.5, observer §7.3), and a
    /// sample whose Encoding is unknown. The predefined Encoding speaks
    /// for itself (§7.2).
    #[test]
    fn a_json_schema_type_decodes_through_the_contract() {
        use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};
        let rev = status();
        let mut cbor = Vec::new();
        ciborium::into_writer(&json!({"up": true}), &mut cbor).expect("cbor");
        let envelope = Envelope {
            form: zenkey_model::wire::Form::Mutation,
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(1),
                seq: Seq::new(2).expect("a seq"),
            },
            record: &cbor,
        }
        .write();
        let key = "zk2/lab/m/m.v1/state/status/eth0";
        for (encoding, bytes) in [
            (Some("zenoh/bytes"), envelope.as_slice()),
            (None, cbor.as_slice()),
            (Some("application/cbor"), cbor.as_slice()),
        ] {
            let r = render_with(&rev, key, Member::Type, encoding, bytes);
            assert_eq!(
                r.rendered,
                Rendered::Value {
                    declared: "json:Status".into(),
                    value: json!({"up": true}),
                    epoch_assumed: None,
                },
                "{encoding:?}"
            );
            assert_eq!(r.size, cbor.len(), "the record's size");
        }
        // Today's bytes under zenoh/bytes: an envelope refused, never
        // decoded as the CBOR they are.
        let refused = render_with(&rev, key, Member::Type, Some("zenoh/bytes"), &cbor);
        assert!(
            matches!(&refused.rendered,
                Rendered::Undecodable { declared, reason }
                    if declared == "json:Status" && reason.starts_with("zk2c envelope refused: ")),
            "{:?}",
            refused.rendered
        );
        let r = render_with(&rev, key, Member::Type, Some("zenoh/bytes"), &envelope);
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
                value: json!({"up": false}),
                epoch_assumed: None,
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
