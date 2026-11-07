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
//! Before any rung reads a payload, a sample carrying a zk2c envelope has
//! it stripped (core §7.5, observer §7.3's "the envelope first",
//! [`crate::model::inband`]): its record is what rung 7 decodes and the
//! structural ladder sniffs, and a refused envelope is undecodable with
//! its refusal named, whatever rung the key stopped at.
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
use crate::model::deployment::{Catalogs, Epoch, Stated};
use crate::model::inband;
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
    generation: u64,
    /// The deployment catalogs given (`--catalog`, core §8.6).
    deployment: Option<&'a Catalogs>,
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
    /// The resource's index in its revision's contract.
    pub fn index(&self) -> usize {
        self.index
    }

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
            generation: 0,
            deployment: None,
        }
    }

    /// The deployment catalogs the tool was given (`--catalog`, core
    /// §8.6): a packed record's `time` fields are then read under the
    /// epoch in force for its owner ([`Lens::epoch_of`]).
    pub fn catalogs(mut self, catalogs: &'a Catalogs) -> Lens<'a> {
        self.deployment = Some(catalogs);
        self
    }

    /// The deployment catalogs given, when they were.
    pub fn deployment_catalogs(&self) -> Option<&'a Catalogs> {
        self.deployment
    }

    /// The epoch `addr`'s packed `time` fields count from (core §8.6): the
    /// catalog its descriptors name, read through the catalogs given; 0
    /// when they name none; with no descriptor in hand, the catalogs'
    /// epoch, the deployment's word; else 0, assumed.
    pub fn epoch_of(&self, addr: &Addr) -> Epoch {
        let stated = self
            .catalog
            .map_or(Stated::Unread, |c| Stated::of(&c.stated_catalogs(addr)));
        self.deployment
            .map_or_else(|| Catalogs::default().epoch(stated), |d| d.epoch(stated))
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

    /// The generation of what the lens resolves with (#614, FL4): a number
    /// that moves whenever its presence read or the revisions in hand do,
    /// such as [`crate::LensFeed::generation`]. An
    /// [`IdentityCache`](crate::model::identity::IdentityCache) drops what it
    /// resolved under another generation, so a key that resolved short of a
    /// provider resolves again once one appears. Zero unless set: a lens
    /// over inputs that never move.
    pub fn at_generation(mut self, generation: u64) -> Lens<'a> {
        self.generation = generation;
        self
    }

    /// The generation set by [`Lens::at_generation`].
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The namespace this lens strips; empty for the bus root.
    pub fn namespace(&self) -> &'a str {
        self.namespace
    }

    /// Where the lens finds a revision by fingerprint.
    pub(crate) fn contracts(&self) -> &'a (dyn Contracts + Sync) {
        self.contracts
    }

    /// The revisions known offline, when a set was given.
    pub(crate) fn offline_set(&self) -> Option<&'a ContractSet> {
        self.offline
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
                cut_off_during: c.cut_off_during(),
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
        self.resolve_with(wire, |candidates, kind, chunks| {
            let refs: Vec<&str> = template_chunks(kind, chunks)
                .iter()
                .map(String::as_str)
                .collect();
            resolve(candidates.iter().map(|(_, r)| &r.template), &refs)
        })
    }

    /// Rungs 1–6 for the key a snapshot's page was heard on (core §7.5's
    /// form 7; #977's E2), whose resource is the one its collection's
    /// index `coll` names among the revision's resources of the key's kind
    /// token (`zenkey_model::collection::collection_of`): a collection
    /// read's snapshot key names no member, and a member read's page rides
    /// its member's key, whose values the template gives. A coll the
    /// revision has no collection at is `NoResource`.
    pub fn resolve_snapshot(&self, wire: &str, coll: u32) -> Resolution {
        self.resolve_with(wire, |candidates, kind, chunks| {
            let i = usize::try_from(coll).ok()?;
            let (_, r) = candidates.get(i)?;
            if !matches!(
                kind,
                zenkey_model::grammar::KindToken::State
                    | zenkey_model::grammar::KindToken::ExplicitState
            ) {
                return None;
            }
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            let values = r.template.matches(&refs).unwrap_or_default();
            Some((i, values))
        })
    }

    /// Rungs 1–6, the resource picked by `pick` among the revision's
    /// resources of the key's kind token, in the canonical order, from the
    /// key's resource chunks: its place among them, and the template's
    /// values.
    fn resolve_with(
        &self,
        wire: &str,
        pick: impl FnOnce(
            &[(usize, &Resource)],
            zenkey_model::grammar::KindToken,
            &[String],
        ) -> Option<(usize, Bindings)>,
    ) -> Resolution {
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
        let Some((i, values)) = pick(&candidates, kind, &chunks) else {
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
    /// the namespace, and the wire key otherwise. A metric resource's
    /// payload is read by `telemetry.v1` too ([`Lens::check_sample`]), with
    /// no attachment.
    pub fn check(
        &self,
        wire: &str,
        member: Member,
        encoding: Option<&str>,
        bytes: &[u8],
    ) -> Checked {
        self.check_sample(wire, member, encoding, bytes, None)
    }

    /// [`Lens::check`], with the sample's attachment when it carried one:
    /// a metric resource's payload (`telemetry.v1` §2.1) is then read by
    /// the profile after its type's own check (§2.6, #819 TC), and a
    /// sample that does not conform is `invalid` with the profile's reasons
    /// named — a Point whose tag or unit disagrees with its declaration, a
    /// counter out of range, a histogram over other bounds. A bare
    /// counter's in-band epoch rides its attachment (§2.4), which only a
    /// caller holding the sample can give.
    pub fn check_sample(
        &self,
        wire: &str,
        member: Member,
        encoding: Option<&str>,
        bytes: &[u8],
        attachment: Option<&[u8]>,
    ) -> Checked {
        self.check_read(wire, member, encoding, bytes, attachment, None)
    }

    /// [`Lens::check_sample`] for a window that reads a key's samples in
    /// the order they arrived: a frame (core §7.7) is applied by its key's
    /// reader in `readers`, so a frame lost before it turns the fields it
    /// omits `unknown` (observer §7.6). [`Lens::check_sample`] reads each
    /// frame alone, as a reader's first.
    pub fn check_frames(
        &self,
        wire: &str,
        member: Member,
        encoding: Option<&str>,
        bytes: &[u8],
        attachment: Option<&[u8]>,
        readers: &mut crate::model::frame::Readers,
    ) -> Checked {
        self.check_read(wire, member, encoding, bytes, attachment, Some(readers))
    }

    fn check_read(
        &self,
        wire: &str,
        member: Member,
        encoding: Option<&str>,
        bytes: &[u8],
        attachment: Option<&[u8]>,
        readers: Option<&mut crate::model::frame::Readers>,
    ) -> Checked {
        let resolution = self.resolve(wire);
        let key = self.relative(wire).unwrap_or(wire);
        // The envelope first (core §7.5, observer §7.3): its record is the
        // payload every rung below reads, on its contract's `encoding`; a
        // refused one is undecodable, never sniffed.
        let payload = match member {
            Member::Type => inband::payload(wire, encoding, bytes),
            _ => inband::Payload::Plain(bytes),
        };
        let tag = payload.tag();
        // Core §7.7: a frame resource's put is a frame, applied by the
        // key's reader; a frame elsewhere, and a mutation on a frame
        // resource, have no reading.
        if let Some(checked) = self.frames(key, &resolution, member, payload, bytes.len(), readers)
        {
            return Checked {
                rendering: checked.0,
                conformance: checked.1,
                identity: resolution.identity,
            };
        }
        let (bytes, encoding) = match payload {
            inband::Payload::Refused(refusal) => {
                let rendering = refused_rendering(
                    resolution.resolved.as_ref(),
                    key,
                    member,
                    bytes.len(),
                    refusal,
                );
                return Checked {
                    conformance: inband::refused_conformance(&rendering, refusal),
                    rendering,
                    identity: resolution.identity,
                };
            }
            // A record is read on its contract's `encoding`, never on
            // `zenoh/bytes` (observer §7.3).
            inband::Payload::Plain(b) => (b, encoding),
            inband::Payload::Record { record, .. } => (record, None),
            // A frame [`Lens::frames`] did not read: its declaration is not
            // in hand, so its envelope and size (observer §7.6).
            inband::Payload::Frame { body, stamp, .. } => {
                let why = resolution
                    .identity
                    .unresolved
                    .clone()
                    .unwrap_or(Unresolved::PresenceNotRead);
                let reason = why.words();
                return Checked {
                    rendering: crate::model::frame::unread_rendering(key, why, stamp, body.len()),
                    conformance: Conformance::NotChecked { reason },
                    identity: resolution.identity,
                };
            }
            // A snapshot's page (core §7.5's form 7; #977's E2): resolved
            // by its collection's index, each entry checked as the per-key
            // reply it stands for.
            inband::Payload::Snapshot { body, header, tag } => {
                return self.snapshot(wire, key, body, header, tag);
            }
            // A heartbeat (core §7.5's form 6): read with the revision of
            // the interface it covers, never as a record.
            inband::Payload::Heartbeat { body, stamp, tag } => {
                let (rendering, conformance) =
                    self.heartbeat(wire, key, &resolution, body, stamp, tag);
                return Checked {
                    rendering,
                    conformance,
                    identity: resolution.identity,
                };
            }
        };
        match &resolution.resolved {
            Some(r) => {
                let epoch = self.epoch_of(&r.address);
                let (rendering, conformance) =
                    check_resolved(r, member, encoding, bytes, attachment, tag, epoch);
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

    /// A frame, or a sample of a frame resource, on a key that resolved
    /// (core §7.7, #981's K2): read by `readers`' reader of `key`, its
    /// namespace-relative key, or alone, as a reader's first; `None` for
    /// any other sample, which its type reads.
    #[allow(clippy::too_many_arguments)]
    fn frames(
        &self,
        key: &str,
        resolution: &Resolution,
        member: Member,
        payload: inband::Payload<'_>,
        size: usize,
        readers: Option<&mut crate::model::frame::Readers>,
    ) -> Option<(PayloadRendering, Conformance)> {
        let r = resolution.resolved.as_ref()?;
        if member != Member::Type {
            return None;
        }
        let resource = r.resource();
        let ty = decode::type_of(
            r.revision.bundle(),
            resource.token.as_str(),
            resource.template.as_str(),
            member.as_str(),
        )?;
        let resolved = ResolvedResource {
            iface: r.iface.to_string(),
            fingerprint: r.revision.fingerprint().to_string(),
            resource: r.resource_name(),
            member: member.as_str().to_owned(),
            values: r.values.clone(),
        };
        let mut alone = zenkey_model::frame::Reader::new();
        // Keyed by the namespace-relative key, as a consumer's subscription
        // keys its readers: a session in the namespace hands it that key.
        let reader = match readers {
            Some(rs) => rs.reader(key),
            None => &mut alone,
        };
        crate::model::render::frame_check(
            r.revision.contract(),
            resource,
            key,
            Some(resolved),
            ty,
            payload,
            size,
            reader,
            self.epoch_of(&r.address),
        )
    }

    /// A heartbeat heard on `wire` (#978's F2): read with the revision its
    /// provider's descriptors name for the interface it covers, when that
    /// is held; else its form and size, with where resolution stopped. On
    /// a key that is no heartbeat's and resolves to a resource, it is
    /// undecodable: a body where a record was read.
    fn heartbeat(
        &self,
        wire: &str,
        key: &str,
        resolution: &Resolution,
        body: &[u8],
        stamp: zenkey_model::wire::Stamp,
        tag: zenkey_model::wire::Tag,
    ) -> (PayloadRendering, Conformance) {
        let Some((addr, iface)) = self
            .relative(wire)
            .and_then(zenkey_model::heartbeat::covered)
        else {
            if resolution.resolved.is_some() {
                let declared = inband::heartbeat_declared(key);
                let reason = inband::HEARTBEAT_AS_RECORD.to_owned();
                return (
                    PayloadRendering {
                        key: key.to_owned(),
                        size: body.len(),
                        resource: None,
                        rendered: Rendered::Undecodable {
                            declared: declared.clone(),
                            reason: reason.clone(),
                        },
                    },
                    Conformance::Undecodable { declared, reason },
                );
            }
            let why = resolution
                .identity
                .unresolved
                .clone()
                .unwrap_or(Unresolved::PresenceNotRead);
            let reason = why.words();
            return (
                inband::heartbeat_rendering(key, why, body.len()),
                Conformance::NotChecked { reason },
            );
        };
        match self.revision_of(&addr, &iface) {
            Ok(rev) => {
                let layout = zenkey_model::heartbeat::Layout::of(rev.contract());
                inband::heartbeat_check(key, &layout, stamp, tag, body)
            }
            Err(why) => {
                let reason = why.words();
                (
                    inband::heartbeat_rendering(key, why, body.len()),
                    Conformance::NotChecked { reason },
                )
            }
        }
    }

    /// A snapshot's page heard on `wire` (#977's E2): its key resolved by
    /// its collection's index ([`Lens::resolve_snapshot`]), and rendered as
    /// a document of its header and its entries, each live entry's record
    /// checked on its member's key as the per-key reply it stands for, each
    /// tombstone a deletion. Its conformance is its entries': valid when
    /// every one is, else the first entry's failure, named by its key. A
    /// body refused is undecodable, its refusal named.
    fn snapshot(
        &self,
        wire: &str,
        key: &str,
        body: &[u8],
        header: zenkey_model::wire::Stamp,
        tag: zenkey_model::wire::Tag,
    ) -> Checked {
        use zenkey_model::collection::{Confirmation, Page, Time};
        const DECLARED: &str = "zk2c snapshot";
        let envelope = zenkey_model::wire::Envelope::snapshot(tag, header.run, header.seq, body);
        let page = match Page::read(&envelope) {
            Ok(p) => p,
            Err(r) => {
                let resolution = self.resolve(wire);
                let reason = format!("a zk2c snapshot's body refused: {r}");
                return Checked {
                    rendering: PayloadRendering {
                        key: key.to_owned(),
                        size: body.len(),
                        resource: None,
                        rendered: Rendered::Undecodable {
                            declared: DECLARED.to_owned(),
                            reason: reason.clone(),
                        },
                    },
                    conformance: Conformance::Undecodable {
                        declared: DECLARED.to_owned(),
                        reason,
                    },
                    identity: resolution.identity,
                };
            }
        };
        let resolution = self.resolve_snapshot(wire, page.coll);
        let time = |t: Time| match t {
            Time::Unknown => serde_json::Value::Null,
            Time::Known { s, ms } => serde_json::json!({"s": s, "ms": ms}),
        };
        let mut live = Vec::new();
        let mut tombs = Vec::new();
        let mut conformance = Conformance::Valid;
        for m in page.members(wire) {
            let member = self.relative(&m.key).unwrap_or(&m.key).to_owned();
            let mut entry = serde_json::json!({
                "key": member,
                "run": m.stamp.run.hex(),
                "seq": m.stamp.seq.get(),
            });
            let Some(payload) = m.payload() else {
                tombs.push(entry);
                continue;
            };
            match m.confirmation {
                Confirmation::None => {}
                Confirmation::AtTime => {
                    entry["confirmed"] = serde_json::json!("at the page's time")
                }
                Confirmation::At(t) => entry["confirmed"] = time(t),
            }
            let c = self.check(
                &m.key,
                Member::Type,
                Some(zenkey_model::wire::ENCODING),
                &payload,
            );
            entry["value"] = match &c.rendering.rendered {
                Rendered::Value { value, .. } => value.clone(),
                Rendered::Structural { value: Some(v), .. } => v.clone(),
                Rendered::Structural { text, .. } => serde_json::Value::String(text.clone()),
                Rendered::Opaque { media_type } => {
                    serde_json::json!({"opaque": media_type, "bytes": c.rendering.size})
                }
                Rendered::Undecodable { reason, .. } => serde_json::json!({"undecodable": reason}),
            };
            conformance = match (conformance, c.conformance) {
                (Conformance::Valid, Conformance::Valid) => Conformance::Valid,
                (Conformance::Valid, Conformance::Invalid { violations }) => Conformance::Invalid {
                    violations: violations
                        .iter()
                        .map(|v| format!("{member}: {v}"))
                        .collect(),
                },
                (Conformance::Valid, Conformance::Undecodable { declared, reason }) => {
                    Conformance::Undecodable {
                        declared,
                        reason: format!("{member}: {reason}"),
                    }
                }
                (Conformance::Valid, other) => other,
                (held, _) => held,
            };
            live.push(entry);
        }
        let resource = resolution.resolved.as_ref().map(|r| ResolvedResource {
            iface: r.iface.to_string(),
            fingerprint: r.revision.fingerprint().to_string(),
            resource: r.resource_name(),
            member: Member::Type.as_str().to_owned(),
            values: r.values.clone(),
        });
        let declared = match &resource {
            Some(r) => format!("{DECLARED} ({} {})", r.iface, r.resource),
            None => DECLARED.to_owned(),
        };
        let value = serde_json::json!({
            "page": page.index,
            "more": page.more,
            "coll": page.coll,
            "run": page.run.hex(),
            "asof": page.asof.get(),
            "time": time(page.time),
            "live": live,
            "tombs": tombs,
        });
        Checked {
            rendering: PayloadRendering {
                key: key.to_owned(),
                size: body.len(),
                resource,
                rendered: Rendered::Value {
                    declared,
                    value,
                    epoch_assumed: None,
                },
            },
            conformance,
            identity: resolution.identity,
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

    /// Whether `wire`'s resource is on core §4.3's table, whose resources
    /// carry the owner's Zenoh timestamp beside its in-band stamp
    /// ([`inband::on_the_table`]): `None` when the key does not resolve to
    /// a resource in hand. Off the table (`Some(false)`), a router's stamp
    /// on an owner's sample is expected, never a finding (observer §4.4).
    pub fn on_the_table(&self, wire: &str) -> Option<bool> {
        let r = self.resolve(wire).resolved?;
        Some(inband::on_the_table(r.revision.contract(), r.resource()))
    }

    /// Whose clock a Zenoh timestamp's `stamper` is, for a key of `address`
    /// (observer §4.4, §8.3): the owner's when it is the session zid the
    /// address's descriptors state as `meta.zid`, compared by value; another
    /// clock when they name one and it differs; and unattributable when
    /// nothing names the owner's — never foreign. It is about the Zenoh
    /// timestamp only: an in-band stamp is the owner's by construction,
    /// since only the owner writes its payload (core §4.2, "Observing S1").
    /// Off core §4.3's table ([`Lens::on_the_table`]) another clock is what
    /// an owner's sample is expected to carry, so a renderer says so, never
    /// a finding.
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

/// A refused envelope's rendering (core §7.5): undecodable, as the
/// resource's declared type when the key resolved to one that declares
/// `member`, else as the envelope its key and Encoding declared.
fn refused_rendering(
    resolved: Option<&Resolved>,
    key: &str,
    member: Member,
    size: usize,
    refusal: zenkey_model::wire::Refusal,
) -> PayloadRendering {
    let Some(r) = resolved else {
        return inband::refused_rendering(key, size, None, None, refusal);
    };
    let resource = r.resource();
    let declared = decode::type_of(
        r.revision.bundle(),
        resource.token.as_str(),
        resource.template.as_str(),
        member.as_str(),
    )
    .map(decode::declared);
    inband::refused_rendering(
        &r.key,
        size,
        Some(ResolvedResource {
            iface: r.iface.to_string(),
            fingerprint: r.revision.fingerprint().to_string(),
            resource: r.resource_name(),
            member: member.as_str().to_owned(),
            values: r.values.clone(),
        }),
        declared,
        refusal,
    )
}

/// Rung 7 through a resolved resource: decode the member, then check the
/// value against its JSON Schema type. A packed record is decoded by the
/// tag its envelope's CH carried (core §7.6), its `time` fields under
/// `epoch` (§8.6), its layout's refusal its conformance.
fn check_resolved(
    r: &Resolved,
    member: Member,
    encoding: Option<&str>,
    bytes: &[u8],
    attachment: Option<&[u8]>,
    tag: Option<zenkey_model::wire::Tag>,
    epoch: Epoch,
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
    let (decoded, epoch_assumed) =
        crate::model::render::decode_under(bundle, ty, encoding, bytes, tag, epoch);
    let (rendered, conformance) = match decoded {
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
            // A packed record's `time` fields, under the epoch in force,
            // or one assumed and said so (core §7.6, §8.6).
            (
                Rendered::Value {
                    declared,
                    value,
                    epoch_assumed,
                },
                conformance,
            )
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
    let conformance = match member {
        Member::Type => telemetry_conformance(
            conformance,
            r.revision.contract(),
            resource,
            bytes,
            attachment,
        ),
        _ => conformance,
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
    attachment: Option<&[u8]>,
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
    let conformance = match member {
        Member::Type => telemetry_conformance(
            conformance,
            revision.contract(),
            resource,
            bytes,
            attachment,
        ),
        _ => conformance,
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

/// [`conformance`] of a sample in hand (a `watch` delivery, #819 TC): the
/// rendering's, then, for a metric resource's payload, `telemetry.v1`'s
/// reading of its bytes and its attachment (§2.6), as
/// [`Lens::check_sample`] reads them.
pub fn sample_conformance(
    revision: &Revision,
    rendering: &PayloadRendering,
    payload: &[u8],
    attachment: Option<&[u8]>,
) -> Conformance {
    let c = conformance(revision, rendering);
    let Some(res) = &rendering.resource else {
        return c;
    };
    if res.member != Member::Type.as_str() {
        return c;
    }
    let contract = revision.contract();
    match contract
        .resources
        .iter()
        .find(|r| zenkey::implementation::resource_name(r) == res.resource)
    {
        Some(r) => telemetry_conformance(c, contract, r, payload, attachment),
        None => c,
    }
}

/// A payload's conformance, read again by `telemetry.v1` when its resource
/// is a metric resource (§2.1, §2.6; #819 TC): a sample that does not
/// conform is `invalid`, the profile's reasons named after the type's own.
/// The type's verdict stands otherwise: bytes the core could not decode
/// stay undecodable, a raw type stays not checked, and a sample the
/// profile cannot read (an invalid declaration, bytes that are not strict
/// JSON) adds nothing — `zenctl telemetry` and the checks say why.
fn telemetry_conformance(
    c: Conformance,
    contract: &zenkey_model::contract::Contract,
    resource: &Resource,
    payload: &[u8],
    attachment: Option<&[u8]>,
) -> Conformance {
    if !matches!(c, Conformance::Valid | Conformance::Invalid { .. }) {
        return c;
    }
    let Some(ty) = crate::model::telemetry::MetricType::of(contract, resource) else {
        return c;
    };
    let zenkey_model::telemetry::Conformance::DoesNot(problems) =
        ty.check(payload, attachment).conformance
    else {
        return c;
    };
    let words = problems
        .into_iter()
        .map(crate::model::telemetry::problem_words);
    match c {
        Conformance::Invalid { mut violations } => {
            violations.extend(words);
            Conformance::Invalid { violations }
        }
        _ => Conformance::Invalid {
            violations: words.collect(),
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

    /// A directory per call (#766): the tests build this contract at once,
    /// and one truncating `s.json` while another read it made the reader
    /// see half a file. The loaded contract carries the schema, so the
    /// directory goes as soon as the load returns.
    fn status() -> Revision {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NTH: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zenkey-fleet-lens-{}-{}",
            std::process::id(),
            NTH.fetch_add(1, Ordering::Relaxed)
        ));
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
        std::fs::remove_dir_all(&dir).expect("remove the temp dir");
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

    /// A frame (core §7.7; #981's K2) reads through its resource's layout:
    /// alone, as a reader's first, by `check_sample`, and by its key's
    /// reader in `check_frames`, so a frame lost turns the next one's
    /// omitted fields unknown; a frame on a resource with no `frame` table,
    /// and a record that came in no frame on a frame resource, are
    /// undecodable; with no contract in hand, its envelope and size. A
    /// `.zrec` row reads through the same calls, held offline.
    #[test]
    fn a_frame_reads_through_its_resources_layout() {
        use zenkey_model::frame::{Frame, Layout};
        use zenkey_model::wire::{Envelope, Run, Seq, Stamp};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/frames/linkframes.v1.toml");
        let (set, problems) = ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        let lens = Lens::new("prod", None, &set).offline(&set);
        let c = zenkey_model::contract::load_path(&path).contract.unwrap();
        let r = c
            .resources
            .iter()
            .find(|r| r.template.as_str() == "frames/link")
            .unwrap();
        let l = Layout::of_resource(&c, r).unwrap().unwrap();
        let put = |seq: u32, v: serde_json::Value| {
            Frame::from_json(&v, &l, None)
                .unwrap()
                .envelope(
                    &l,
                    Stamp {
                        run: Run::U16(7),
                        seq: Seq::new(seq).unwrap(),
                    },
                )
                .unwrap()
        };
        let wire = "prod/zk2/lab/modem/linkframes.v1/stream/frames/link";
        let full = put(
            1,
            json!({"time": {"s": 1, "ms": 0}, "records": [{"full": true,
                "values": {"dropped": 4, "queue_depth": 2}}]}),
        );
        let delta = put(
            3,
            json!({"time": {"s": 3, "ms": 0}, "records": [{"full": false,
                "values": {"dropped": 5}}]}),
        );
        let value = |c: &Checked| match &c.rendering.rendered {
            Rendered::Value {
                declared, value, ..
            } => {
                assert_eq!(declared, "zk2c frame of packed:Link");
                value.clone()
            }
            r => panic!("{r:?}"),
        };
        let mut readers = crate::model::frame::Readers::new();
        let enc = Some("zenoh/bytes");
        let first = lens.check_frames(wire, Member::Type, enc, &full, None, &mut readers);
        assert_eq!(first.conformance, Conformance::Valid);
        let q = &value(&first)["records"][0]["fields"]["queue_depth"];
        assert_eq!(q, &json!({"state": "known", "value": 2}));
        let gap = lens.check_frames(wire, Member::Type, enc, &delta, None, &mut readers);
        let v = value(&gap);
        assert_eq!(
            (v["follows"].clone(), v["lost"].clone()),
            (json!("gap"), json!(1))
        );
        assert_eq!(
            v["records"][0]["fields"]["queue_depth"],
            json!({"state": "unknown", "why": "gap"})
        );
        let alone = lens.check_sample(wire, Member::Type, enc, &delta, None);
        assert_eq!(value(&alone)["follows"], "first");
        // Under the namespace `prod`, the reader is the relative key's, the
        // one a consumer's subscription in the namespace keys its own by.
        assert!(readers.holds("zk2/lab/modem/linkframes.v1/stream/frames/link"));
        assert!(!readers.holds(wire));
        assert_eq!(readers.len(), 1);
        // A record in no frame on the frame resource; a frame on a plain
        // stream of the same record type.
        let mutation = Envelope::mutation(
            l.tag(),
            Stamp {
                run: Run::U16(7),
                seq: Seq::new(4).unwrap(),
            },
            &[0; 4],
        )
        .write();
        let c = lens.check_sample(wire, Member::Type, enc, &mutation, None);
        assert!(
            matches!(&c.conformance, Conformance::Undecodable { reason, .. }
                if reason == crate::model::frame::MUTATION_ON_FRAME),
            "{:?}",
            c.conformance
        );
        let level = "prod/zk2/lab/modem/linkframes.v1/stream/level";
        let c = lens.check_sample(level, Member::Type, enc, &full, None);
        assert!(
            matches!(&c.conformance, Conformance::Undecodable { reason, .. }
                if reason == crate::model::frame::FRAME_AS_RECORD),
            "{:?}",
            c.conformance
        );
        // No contract in hand: the envelope and the size, never sniffed.
        let empty = ContractSet::new();
        let blind = Lens::new("prod", None, &empty);
        let c = blind.check_sample(wire, Member::Type, enc, &full, None);
        assert!(
            matches!(&c.rendering.rendered, Rendered::Structural { text, value: None, .. }
                if text.starts_with("<zk2c frame, run 0007, seq 1, ")),
            "{:?}",
            c.rendering.rendered
        );
    }

    /// A heartbeat (core §7.5's form 6; #978's F2) reads with the revision
    /// its provider names for the interface it covers: its value, valid; on
    /// another revision's tag undecodable, the tag named; on a resolved
    /// data key undecodable, a body where a record was read.
    #[test]
    fn a_heartbeat_reads_with_the_revision_it_covers() {
        use zenkey_model::heartbeat::{Heartbeat, Layout, Time};
        use zenkey_model::wire::{Run, Seq, Stamp};
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let layout = Layout::of(rev.contract());
        let mut set = ContractSet::new();
        set.insert(rev);
        let cat = catalog(&fp, None);
        let lens = Lens::new("prod", Some(&cat), &set);
        let stamp = Stamp {
            run: Run::U16(7),
            seq: Seq::new(1).expect("a seq"),
        };
        let hb = Heartbeat {
            time: Time::Unknown,
            sections: vec![],
        };
        let bytes = hb.envelope(&layout, stamp).expect("writes");
        let wire = "prod/zk2/lab/m/freshness.v2/stream/heartbeat/m.v1";
        let ok = lens.check(wire, Member::Type, Some("zenoh/bytes"), &bytes);
        assert_eq!(ok.conformance, Conformance::Valid, "{:?}", ok.rendering);
        assert!(
            matches!(&ok.rendering.rendered,
                Rendered::Value { declared, value, .. } if declared == "zk2c heartbeat (m.v1)"
                    && value == &json!({"covers": "m.v1", "run": "0007", "seq": 1,
                                        "time": null, "sections": {}})),
            "{:?}",
            ok.rendering.rendered
        );
        let mut other = bytes.clone();
        other[0] = (other[0] & 0xf0) | ((layout.tag.get() + 1) % 16);
        let bad = lens.check(wire, Member::Type, Some("zenoh/bytes"), &other);
        assert!(
            matches!(&bad.conformance, Conformance::Undecodable { reason, .. } if reason.contains("not m.v1's")),
            "{:?}",
            bad.conformance
        );
        let on_data = lens.check(
            "prod/zk2/lab/m/m.v1/state/status/eth0",
            Member::Type,
            Some("zenoh/bytes"),
            &bytes,
        );
        assert!(
            matches!(&on_data.conformance, Conformance::Undecodable { reason, .. }
                if reason == inband::HEARTBEAT_AS_RECORD),
            "{:?}",
            on_data.conformance
        );
    }

    /// The envelope first (core §7.5, observer §7.3; #976's A2): a state
    /// sample under `zenoh/bytes` carries a zk2c envelope, told by its key
    /// and Encoding alone. Its record renders as the resource's type and
    /// is checked against it, read on the contract's `encoding`; a refused
    /// one is undecodable with its refusal named, resolved or not, never
    /// sniffed; the same bytes under their predefined Encoding read as
    /// today.
    #[test]
    fn an_enveloped_record_renders_as_its_type_and_a_refused_one_is_undecodable() {
        use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};
        let rev = status();
        let fp = rev.fingerprint().to_string();
        let mut set = ContractSet::new();
        set.insert(rev);
        let cat = catalog(&fp, None);
        let lens = Lens::new("prod", Some(&cat), &set);
        let wire = "prod/zk2/lab/m/m.v1/state/status/eth0";
        let envelope = Envelope {
            form: zenkey_model::wire::Form::Mutation,
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U16(7),
                seq: Seq::new(1).expect("a seq"),
            },
            record: br#"{"up":true}"#,
        }
        .write();
        let ok = lens.check(wire, Member::Type, Some("zenoh/bytes"), &envelope);
        assert_eq!(ok.conformance, Conformance::Valid);
        assert_eq!(ok.rendering.size, br#"{"up":true}"#.len(), "the record's");
        assert!(
            matches!(&ok.rendering.rendered,
                Rendered::Value { declared, value, .. } if declared == "json:Status" && value == &json!({"up": true})),
            "{:?}",
            ok.rendering.rendered
        );
        let bad = Envelope {
            record: br#"{"up":1}"#,
            ..Envelope::parse(&envelope).expect("an envelope")
        }
        .write();
        assert!(matches!(
            lens.check(wire, Member::Type, Some("zenoh/bytes"), &bad)
                .conformance,
            Conformance::Invalid { .. }
        ));
        // Today's bytes under zenoh/bytes: an envelope refused, never the
        // document they would sniff as.
        let refused = lens.check(wire, Member::Type, Some("zenoh/bytes"), br#"{"up":true}"#);
        let Rendered::Undecodable { declared, reason } = &refused.rendering.rendered else {
            panic!("{:?}", refused.rendering.rendered);
        };
        assert_eq!(declared, "json:Status");
        assert!(reason.starts_with("zk2c envelope refused: "), "{reason}");
        assert!(matches!(
            &refused.conformance,
            Conformance::Undecodable { reason: r, .. } if r == reason
        ));
        assert!(refused.rendering.resource.is_some(), "the key resolved");
        // Unresolved (no presence read): still refused, as the envelope.
        let blind = Lens::new("prod", None, &set);
        let unresolved = blind.check(wire, Member::Type, Some("zenoh/bytes"), b"{}");
        assert!(
            matches!(&unresolved.rendering.rendered,
                Rendered::Undecodable { declared, .. } if declared == "zk2c envelope"),
            "{:?}",
            unresolved.rendering.rendered
        );
        assert_eq!(
            unresolved.identity.unresolved,
            Some(Unresolved::PresenceNotRead),
            "the identity still says where resolution stopped"
        );
        // An unresolved envelope's record falls to the structural ladder.
        let sniffed = blind.check(wire, Member::Type, Some("zenoh/bytes"), &envelope);
        assert!(
            matches!(&sniffed.rendering.rendered,
                Rendered::Structural { value: Some(v), .. } if v == &json!({"up": true})),
            "{:?}",
            sniffed.rendering.rendered
        );
        // Its predefined Encoding: a sample with no envelope.
        let today = lens.check(
            wire,
            Member::Type,
            Some("application/json"),
            br#"{"up":true}"#,
        );
        assert_eq!(today.conformance, Conformance::Valid);
    }

    /// A `.zrec` keeps a sample's bytes as they came (#976's A2): a
    /// recording of an enveloped sample replays through the same lens,
    /// its record rendered as its type, with no version bump.
    #[test]
    fn a_recorded_enveloped_sample_replays_through_the_lens() {
        use crate::tape::record::{ZREC_VERSION, ZrecItem, ZrecReader, ZrecWriter};
        use zenkey_model::wire::{Envelope, Run, Seq, Stamp, Tag};
        let rev = status();
        let mut set = ContractSet::new();
        set.insert(rev);
        let lens = Lens::new("prod", None, &set).offline(&set);
        let wire = "prod/zk2/lab/m/m.v1/state/status/eth0";
        let envelope = Envelope {
            form: zenkey_model::wire::Form::Mutation,
            tag: Tag::NONE,
            stamp: Stamp {
                run: Run::U32(70_000),
                seq: Seq::new(9).expect("a seq"),
            },
            record: br#"{"up":false}"#,
        }
        .write();
        let view = crate::SampleView {
            key: wire.into(),
            payload: zenoh::bytes::ZBytes::from(envelope.clone()),
            encoding: "zenoh/bytes".into(),
            kind: zenoh::sample::SampleKind::Put,
            timestamp: None,
            stamped_by: None,
            attachment: None,
            priority: zenoh::qos::Priority::Data,
            congestion_control: zenoh::qos::CongestionControl::Block,
            reliability: zenoh::qos::Reliability::Reliable,
            express: false,
            source: None,
            received: std::time::Instant::now(),
        };
        let header = crate::ZrecHeader {
            zrec: ZREC_VERSION,
            selectors: vec!["prod/zk2/**".into()],
            base: "prod".into(),
            captured_at: "2026-11-01T00:00:00Z".into(),
            excluded: None,
            preamble: None,
            pre_roll: None,
        };
        let mut sink = Vec::new();
        let mut w = ZrecWriter::new(&mut sink, &header).expect("a writer");
        w.write_sample(&view).expect("a row");
        let _ = w.finish().expect("finished");
        let mut reader = ZrecReader::new(sink.as_slice()).expect("a reader");
        let Some(Ok(ZrecItem::Sample { row, .. })) = reader.next() else {
            panic!("a row");
        };
        assert_eq!(row.payload, envelope, "the bytes as they came");
        let checked = lens.check(
            &row.key,
            Member::Type,
            row.encoding.as_deref(),
            &row.payload,
        );
        assert_eq!(checked.conformance, Conformance::Valid);
        assert!(
            matches!(&checked.rendering.rendered,
                Rendered::Value { value, .. } if value == &json!({"up": false})),
            "{:?}",
            checked.rendering.rendered
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

    /// Core §4.3's table through the lens: a `freshness.v1` state with a
    /// horizon is on it, a plain state is not, nor a `freshness.v2` one
    /// with a horizon (`tc.netif.v1` since #978's F5), and a key resolving
    /// to no resource in hand says nothing.
    #[test]
    fn the_lens_says_whether_a_keys_resource_is_on_the_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let (mut set, problems) =
            ContractSet::load_path(&root.join("examples/zk2/tcgui/tc.netif.v1.toml"));
        assert!(problems.is_empty(), "{problems:?}");
        let (beacon, problems) =
            ContractSet::load_path(&root.join("zenkey/tests/contracts/beacon.v1.toml"));
        assert!(problems.is_empty(), "{problems:?}");
        set.extend(beacon);
        let lens = Lens::new("", None, &set).offline(&set);
        assert_eq!(
            lens.on_the_table("zk2/lab/b/beacon.v1/state/status"),
            Some(true),
            "freshness.v1, freshness.ttl_s 2"
        );
        assert_eq!(
            lens.on_the_table("zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0"),
            Some(false),
            "freshness.v2, freshness.ttl_s 60"
        );
        assert_eq!(
            lens.on_the_table("zk2/host-a/tc/tc.netif.v1/state/namespaces"),
            Some(false)
        );
        assert_eq!(
            lens.on_the_table("zk2/host-a/tc/tc.netif.v1/stream/bandwidth/default/eth0"),
            Some(false)
        );
        assert_eq!(lens.on_the_table("plain/key"), None);
        assert_eq!(lens.on_the_table("zk2/host-a/tc/other.v1/state/x"), None);
    }

    /// A metric resource's sample is read by `telemetry.v1` after its type
    /// (#819, TC): a Point the schema admits whose tag or unit disagrees with
    /// the declaration is `invalid` with the profile's reasons, and a bare
    /// counter's Epoch attachment is read where the sample gives one.
    #[test]
    fn a_metric_sample_is_read_by_telemetry_v1() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/meter.v1.toml");
        let (set, problems) = ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        let lens = Lens::new("", None, &set).offline(&set);
        let frames = "zk2/lab/meter/meter.v1/stream/frames";
        let bytes = |v: serde_json::Value| serde_json::to_vec(&v).unwrap();
        let check = |key: &str, v: serde_json::Value, a: Option<&[u8]>| {
            lens.check_sample(key, Member::Type, None, &bytes(v), a)
                .conformance
        };
        assert_eq!(
            check(
                frames,
                json!({"timestamp": 1, "value": {"type": "counter", "value": 3}}),
                None
            ),
            Conformance::Valid
        );
        let Conformance::Invalid { violations } = check(
            frames,
            json!({"timestamp": 1, "value": {"type": "gauge", "value": 3.5}, "unit": "By"}),
            None,
        ) else {
            panic!("a Point whose tag and unit disagree is invalid");
        };
        assert_eq!(violations.len(), 2, "{violations:?}");
        assert!(violations[0].starts_with("telemetry.v1 §2.6 tag_disagrees"));
        assert!(violations[1].starts_with("telemetry.v1 §2.6 unit_disagrees"));
        let bytes_key = "zk2/lab/meter/meter.v1/stream/bytes";
        assert_eq!(
            check(bytes_key, json!(60), Some(b"\"e1\"")),
            Conformance::Valid
        );
        let Conformance::Invalid { violations } = check(bytes_key, json!(60), Some(b"\"\"")) else {
            panic!("an empty epoch is epoch_invalid");
        };
        assert!(violations[0].contains("epoch_invalid"), "{violations:?}");
        // The level gauge is a bare number: a string is the schema's
        // violation first, then the profile's.
        let Conformance::Invalid { violations } =
            check("zk2/lab/meter/meter.v1/stream/level", json!("x"), None)
        else {
            panic!("a string is no gauge");
        };
        assert!(
            violations.last().unwrap().contains("wrong_type"),
            "{violations:?}"
        );
    }
}
