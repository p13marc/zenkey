//! zk2's catalog (spec §8.1, §3.3, §9): who provides which interface at
//! which revision, who requires it, and what each revision's contract says —
//! from presence, descriptors and the contracts in hand.
//!
//! **Session-free.** [`crate::bus::presence`] reads the tokens and the
//! descriptors into an [`Observed`], and [`crate::bus::contracts`] retrieves
//! bundles into a [`BundleStore`](crate::bus::contracts::BundleStore); this
//! module joins values already in hand, so a frontend can build the same
//! views from a recording, a fixture or a test as from the bus.
//!
//! Three things live here:
//!
//! - **[`Observed`]**, what one presence read brought back: parsed tokens,
//!   the keys that did not parse, the completeness flag (§8.1) and each
//!   instance's [`DescriptorRead`]. It is the bus layer's output and this
//!   layer's input.
//! - **Revisions in hand**: a [`Revision`] is one contract revision, its
//!   verified bundle and the contract read from it; [`Contracts`] is how a
//!   view asks for one by `(iface, fingerprint)`, answered by the
//!   retrieving [`BundleStore`](crate::bus::contracts::BundleStore) or by an
//!   offline [`ContractSet`] loaded from authoring files or a `.history`
//!   root (§9.7).
//! - **[`Catalog`]**, the index over an [`Observed`], and the views zenctl
//!   prints: [`Catalog::services`] (`service list`), [`Catalog::iface`]
//!   (`iface show`) and [`Catalog::graph`] (`graph`, through the runtime's
//!   own [`zenkey::presence::edges`], so the tool draws the graph the runtime
//!   computes).
//!
//! Nothing here judges. A token whose fingerprint prefix disagrees with its
//! descriptor, a role whose bindings match nothing, a second instance: all
//! are shown as found, and saying which of them is wrong is `doctor`'s
//! (FJ6).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use zenkey_model::authoring::Kind;
use zenkey_model::bundle::{Bundle, BundleError};
use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::{Body, Contract, Resource};
use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
use zenkey_model::grammar::{Addr, Fp16, IfaceId, InstanceId, ZkKey};
use zenkey_model::schema::{ArtifactData, TypeId};

use crate::report::{
    Asked, BindingGraph, ContractAnswer, ContractSource, ContractView, DescriptorAnswer, GraphEdge,
    GraphNode, GraphRole, IfaceConsumer, IfaceListing, IfaceProvider, IfaceRevision, IfaceSighting,
    IfaceSummary, IfaceView, InstanceRef, InstanceSighting, MemberSighting, NamespaceListing,
    NamespaceSighting, RequirementView, ResourceBody, ResourceView, SchemaArtifact, SchemaDocument,
    SchemaMember, SchemaView, ServiceListing, ServiceSighting, ServiceView, TypeView,
};

// ─── what presence brought back ─────────────────────────────────────────────

/// One presence read (§8.1), and the descriptors of the instances it named
/// (§3.3), as values.
#[derive(Debug, Clone, Default)]
pub struct Observed {
    /// The liveliness selector read, base-relative.
    pub selector: String,
    /// Whether the liveliness GET completed before its timeout. `false` is
    /// "possibly incomplete": what is missing here may still be up (O5).
    pub complete: bool,
    /// The zk2 tokens read: instance, interface and member tokens.
    pub tokens: Vec<ZkKey>,
    /// Keys the selector matched that are not zk2 tokens, verbatim.
    pub unparsed: Vec<String>,
    /// Each instance's descriptor read, when descriptors were asked for;
    /// `None` when they were not.
    pub descriptors: Option<BTreeMap<(Addr, InstanceId), DescriptorRead>>,
}

impl Observed {
    /// The tokens of one liveliness read, parsed (§1.1). A key that is not a
    /// zk2 token is kept verbatim in `unparsed`, not dropped: under `@zk`
    /// it is a non-conforming key, which is a fact about the bus.
    pub fn from_keys(selector: impl Into<String>, keys: &[String], complete: bool) -> Observed {
        let mut tokens = Vec::new();
        let mut unparsed = Vec::new();
        for key in keys {
            match zenkey_model::grammar::parse(key) {
                Ok(k @ (ZkKey::Instance { .. } | ZkKey::Alive { .. } | ZkKey::Member { .. })) => {
                    tokens.push(k)
                }
                _ => unparsed.push(key.clone()),
            }
        }
        Observed {
            selector: selector.into(),
            complete,
            tokens,
            unparsed,
            descriptors: None,
        }
    }

    /// Every instance a token names: by its instance token, or by an
    /// interface or member token alone. A member token's epoch is not an
    /// instance id, so it names the service but no instance.
    pub fn instances(&self) -> BTreeSet<(Addr, InstanceId)> {
        self.tokens
            .iter()
            .filter_map(|k| match k {
                ZkKey::Instance { addr, instance } | ZkKey::Alive { addr, instance, .. } => {
                    Some((addr.clone(), instance.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

/// What one descriptor GET found (§3.3).
#[derive(Debug, Clone, PartialEq)]
pub enum DescriptorRead {
    /// A descriptor that passes the syntax checks.
    Served(Box<Descriptor>),
    /// A reply that fails the descriptor check, with its findings.
    Invalid(String),
    /// No reply within the timeout. Not a verdict.
    Silent,
    /// The GET could not be put on the bus.
    Failed(String),
}

impl From<zenkey::presence::Found> for DescriptorRead {
    fn from(found: zenkey::presence::Found) -> DescriptorRead {
        match found {
            zenkey::presence::Found::Descriptor(d, _) => DescriptorRead::Served(d),
            zenkey::presence::Found::Invalid(why) => DescriptorRead::Invalid(why),
            zenkey::presence::Found::Nothing => DescriptorRead::Silent,
        }
    }
}

impl DescriptorRead {
    /// The descriptor, when one was served.
    pub fn descriptor(&self) -> Option<&Descriptor> {
        match self {
            DescriptorRead::Served(d) => Some(d),
            _ => None,
        }
    }

    /// The answer as the report spells it.
    pub fn answer(&self) -> DescriptorAnswer {
        match self {
            DescriptorRead::Served(d) => DescriptorAnswer::Served {
                descriptor: d.clone(),
            },
            DescriptorRead::Invalid(findings) => DescriptorAnswer::Invalid {
                findings: findings.clone(),
            },
            DescriptorRead::Silent => DescriptorAnswer::Silent,
            DescriptorRead::Failed(reason) => DescriptorAnswer::Failed {
                reason: reason.clone(),
            },
        }
    }

    /// The answer's tag: `served`, `invalid`, `silent` or `failed`.
    pub fn tag(&self) -> &'static str {
        match self {
            DescriptorRead::Served(_) => "served",
            DescriptorRead::Invalid(_) => "invalid",
            DescriptorRead::Silent => "silent",
            DescriptorRead::Failed(_) => "failed",
        }
    }
}

// ─── revisions in hand ──────────────────────────────────────────────────────

/// One contract revision in hand: its fingerprint, its verified bundle, and
/// the contract read from it.
///
/// The bundle is kept beside the contract because decoding a sample needs
/// it (`zenkey_model::decode` reads the schema artifacts it carries), and
/// the contract is shared because a consumer or a client built for a tool
/// takes an `Arc<Contract>` ([`zenkey::consumer::Consumer::for_tool`],
/// [`zenkey::Client::new`]).
#[derive(Debug, Clone)]
pub struct Revision {
    fingerprint: Fingerprint,
    bundle: Bundle,
    contract: Arc<Contract>,
    source: ContractSource,
}

impl Revision {
    /// The revision of a contract loaded from its authoring file. The
    /// bundle is built from it, so the fingerprint is the one a service
    /// implementing it would serve.
    pub fn from_contract(contract: Contract, source: ContractSource) -> Revision {
        let bundle = Bundle::build(&contract);
        Revision {
            fingerprint: bundle.fingerprint(),
            bundle,
            contract: Arc::new(contract),
            source,
        }
    }

    /// The revision a verified bundle carries, its contract read back with
    /// [`Contract::from_bundle`]. The bundle must already be verified
    /// (`Bundle::verify_expecting`); this only reads it.
    pub fn from_bundle(bundle: Bundle, source: ContractSource) -> Result<Revision, BundleError> {
        let contract = Contract::from_bundle(&bundle)?;
        Ok(Revision {
            fingerprint: bundle.fingerprint(),
            bundle,
            contract: Arc::new(contract),
            source,
        })
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    pub fn iface(&self) -> &IfaceId {
        &self.contract.iface
    }

    pub fn bundle(&self) -> &Bundle {
        &self.bundle
    }

    pub fn contract(&self) -> &Contract {
        &self.contract
    }

    /// The contract, shared: what a tool's consumer or client is built on.
    pub fn shared_contract(&self) -> Arc<Contract> {
        Arc::clone(&self.contract)
    }

    pub fn source(&self) -> ContractSource {
        self.source
    }

    /// The contract as `iface show` and `schema show` print it.
    pub fn view(&self) -> ContractView {
        let c = &*self.contract;
        ContractView {
            iface: c.iface.to_string(),
            fingerprint: self.fingerprint.to_string(),
            minor: c.minor,
            summary: c.summary.clone(),
            uses: c.uses.iter().map(ToString::to_string).collect(),
            resources: c.resources.iter().map(resource_view).collect(),
            requires: c
                .requires
                .iter()
                .map(|(role, r)| RequirementView {
                    role: role.clone(),
                    interface: r.interface.to_string(),
                    cardinality: r.cardinality,
                    optional: r.optional,
                    resources: r.resources.clone(),
                })
                .collect(),
            schemas: c
                .artifacts
                .iter()
                .map(|(id, a)| SchemaArtifact {
                    id: id.clone(),
                    kind: a.kind.as_str().to_owned(),
                    name: a.name.clone(),
                })
                .collect(),
        }
    }

    /// The resource `want` names among those of `kinds` (every kind when
    /// empty): `<kind token>/<template>` (`stream/bandwidth/{ns}/{iface}`,
    /// `@op/diagnostics`), or its template alone when only one such resource
    /// has it. What `schema show`, `call`, `get state` and `watch` take.
    ///
    /// The error is a sentence naming the resources there are: a resource
    /// the revision does not declare is the caller's input to fix.
    pub fn resource(&self, want: &str, kinds: &[Kind]) -> Result<&Resource, String> {
        let c = &*self.contract;
        let eligible = |r: &&Resource| kinds.is_empty() || kinds.contains(&r.kind);
        let exact: Vec<&Resource> = c
            .resources
            .iter()
            .filter(eligible)
            .filter(|r| zenkey::implementation::resource_name(r) == want)
            .collect();
        let by_template: Vec<&Resource> = c
            .resources
            .iter()
            .filter(eligible)
            .filter(|r| r.template.as_str() == want)
            .collect();
        let what = match kinds {
            [] => "resource".to_owned(),
            ks => format!(
                "{} resource",
                ks.iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(" or ")
            ),
        };
        match (exact.as_slice(), by_template.as_slice()) {
            ([r], _) | ([], [r]) => Ok(*r),
            ([], []) => {
                let names: Vec<String> = c
                    .resources
                    .iter()
                    .filter(eligible)
                    .map(zenkey::implementation::resource_name)
                    .collect();
                Err(if names.is_empty() {
                    format!("{} declares no {what}", c.iface)
                } else {
                    format!(
                        "{} declares no {what} {want:?}; it declares: {}",
                        c.iface,
                        names.join(", ")
                    )
                })
            }
            (_, several) => {
                let names: Vec<String> = several
                    .iter()
                    .map(|r| zenkey::implementation::resource_name(r))
                    .collect();
                Err(format!(
                    "{want:?} is the template of more than one resource of {}: {} — \
                     name one with its kind token",
                    c.iface,
                    names.join(", ")
                ))
            }
        }
    }

    /// `schema show`: every member's type and the artifacts they live in,
    /// for the whole revision or one resource — named in full
    /// (`stream/bandwidth/{ns}/{iface}`) or by its template when only one
    /// resource has it. With `documents`, each artifact's document rides
    /// along ([`SchemaDocument::document`]).
    ///
    /// The error is a sentence naming the resources there are: a resource
    /// the revision does not declare is the caller's input to fix.
    pub fn schema_view(
        &self,
        resource: Option<&str>,
        documents: bool,
    ) -> Result<SchemaView, String> {
        let c = &*self.contract;
        let selected: Vec<&Resource> = match resource {
            None => c.resources.iter().collect(),
            Some(want) => vec![self.resource(want, &[])?],
        };
        let members: Vec<SchemaMember> = selected.iter().flat_map(|r| members_of(r)).collect();
        let wanted: BTreeSet<&str> = match resource {
            None => c.artifacts.keys().map(String::as_str).collect(),
            Some(_) => members
                .iter()
                .filter_map(|m| m.type_.schema.as_deref())
                .collect(),
        };
        let artifacts = c
            .artifacts
            .iter()
            .filter(|(id, _)| wanted.contains(id.as_str()))
            .map(|(id, a)| SchemaDocument {
                id: id.clone(),
                kind: a.kind.as_str().to_owned(),
                name: a.name.clone(),
                document: if documents {
                    Asked::Asked(artifact_document(&a.data))
                } else {
                    Asked::NotAsked
                },
            })
            .collect();
        Ok(SchemaView {
            iface: c.iface.to_string(),
            fingerprint: self.fingerprint.to_string(),
            source: self.source,
            resource: selected
                .first()
                .filter(|_| resource.is_some())
                .map(|r| zenkey::implementation::resource_name(r)),
            members,
            artifacts,
        })
    }
}

/// Every member of a resource that names a type, in the spec's order.
fn members_of(r: &Resource) -> Vec<SchemaMember> {
    let resource = zenkey::implementation::resource_name(r);
    let named: Vec<(&str, Option<&TypeId>)> = match &r.body {
        Body::Data(d) => vec![
            ("type", Some(&d.type_)),
            ("attachment", d.attachment.as_ref()),
        ],
        Body::Operation(o) => vec![
            ("request", Some(&o.request)),
            ("response", Some(&o.response)),
            ("error", o.error.as_ref()),
            ("summary", o.summary.as_ref()),
        ],
    };
    named
        .into_iter()
        .filter_map(|(member, t)| {
            Some(SchemaMember {
                resource: resource.clone(),
                member: member.to_owned(),
                type_: type_view(t?),
            })
        })
        .collect()
}

/// An artifact as a document a person or a script can read: a JSON Schema
/// as it is carried, a protobuf descriptor set as its files, messages and
/// enums. A set that does not decode says so in the document, never by
/// vanishing.
fn artifact_document(data: &ArtifactData) -> serde_json::Value {
    match data {
        ArtifactData::Json(v) => v.clone(),
        ArtifactData::Protobuf(bytes) => match prost_reflect::DescriptorPool::decode(&bytes[..]) {
            Ok(pool) => serde_json::json!({
                "files": pool.files().map(|f| serde_json::json!({
                    "name": f.name(),
                    "package": f.package_name(),
                    "messages": f.messages().flat_map(|m| message_docs(&m)).collect::<Vec<_>>(),
                    "enums": f.enums().map(|e| enum_doc(&e)).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }),
            Err(e) => serde_json::json!({ "unreadable": e.to_string() }),
        },
    }
}

/// One message and every message and enum nested in it, flattened by full
/// name. A map's synthetic entry message is not a message anyone wrote, so
/// it is folded into its field's `map<K, V>` type instead.
fn message_docs(m: &prost_reflect::MessageDescriptor) -> Vec<serde_json::Value> {
    let fields: Vec<serde_json::Value> = m
        .fields()
        .map(|f| {
            let mut doc = serde_json::json!({
                "name": f.name(),
                "number": f.number(),
                "type": field_type(&f),
            });
            let label = if f.is_map() {
                None
            } else if f.is_list() {
                Some("repeated")
            } else if f.cardinality() == prost_reflect::Cardinality::Required {
                Some("required")
            } else if f.supports_presence() && f.containing_oneof().is_none() {
                Some("optional")
            } else {
                None
            };
            if let Some(label) = label {
                doc["label"] = label.into();
            }
            if let Some(o) = f.containing_oneof() {
                doc["oneof"] = o.name().into();
            }
            doc
        })
        .collect();
    let mut out = vec![serde_json::json!({ "name": m.full_name(), "fields": fields })];
    for e in m.child_enums() {
        out.push(enum_doc(&e));
    }
    for child in m.child_messages().filter(|c| !c.is_map_entry()) {
        out.extend(message_docs(&child));
    }
    out
}

fn enum_doc(e: &prost_reflect::EnumDescriptor) -> serde_json::Value {
    serde_json::json!({
        "name": e.full_name(),
        "values": e.values().map(|v| serde_json::json!({"name": v.name(), "number": v.number()})).collect::<Vec<_>>(),
    })
}

fn field_type(f: &prost_reflect::FieldDescriptor) -> String {
    use prost_reflect::Kind;
    fn scalar(k: &Kind) -> String {
        match k {
            Kind::Double => "double".into(),
            Kind::Float => "float".into(),
            Kind::Int32 => "int32".into(),
            Kind::Int64 => "int64".into(),
            Kind::Uint32 => "uint32".into(),
            Kind::Uint64 => "uint64".into(),
            Kind::Sint32 => "sint32".into(),
            Kind::Sint64 => "sint64".into(),
            Kind::Fixed32 => "fixed32".into(),
            Kind::Fixed64 => "fixed64".into(),
            Kind::Sfixed32 => "sfixed32".into(),
            Kind::Sfixed64 => "sfixed64".into(),
            Kind::Bool => "bool".into(),
            Kind::String => "string".into(),
            Kind::Bytes => "bytes".into(),
            Kind::Message(m) => m.full_name().to_owned(),
            Kind::Enum(e) => e.full_name().to_owned(),
        }
    }
    if f.is_map()
        && let Kind::Message(entry) = f.kind()
    {
        return format!(
            "map<{}, {}>",
            scalar(&entry.map_entry_key_field().kind()),
            scalar(&entry.map_entry_value_field().kind())
        );
    }
    scalar(&f.kind())
}

/// A resolved type as the report names it.
pub fn type_view(t: &TypeId) -> TypeView {
    match t {
        TypeId::Protobuf { name, schema } => TypeView {
            kind: "protobuf".into(),
            name: name.clone(),
            schema: Some(schema.clone()),
        },
        TypeId::JsonSchema { name, schema } => TypeView {
            kind: "jsonschema".into(),
            name: name.clone(),
            schema: Some(schema.clone()),
        },
        TypeId::Raw {
            media_type,
            media_param,
        } => TypeView {
            kind: "raw".into(),
            name: match media_param {
                Some(p) => format!("{media_type};{p}"),
                None => media_type.clone(),
            },
            schema: None,
        },
    }
}

fn resource_view(r: &Resource) -> ResourceView {
    let body = match &r.body {
        Body::Data(d) => ResourceBody::Data {
            type_: type_view(&d.type_),
            attachment: d.attachment.as_ref().map(type_view),
            encoding: d.encoding,
            attachment_encoding: d.attachment_encoding,
            reliability: d.reliability,
            congestion: d.congestion,
            priority: d.priority,
            express: d.express,
            history: d.history.clone(),
            rate: d.rate.map(|r| r.to_string()),
            retention_s: d.retention_s,
        },
        Body::Operation(o) => ResourceBody::Operation {
            request: type_view(&o.request),
            response: type_view(&o.response),
            error: o.error.as_ref().map(type_view),
            summary: o.summary.as_ref().map(type_view),
            encoding: o.encoding,
            idempotent: o.idempotent,
            fanout: o.fanout,
            serving: o.serving,
            replies: o.replies,
            timeout_ms: o.timeout_ms,
            priority: o.priority,
        },
    };
    ResourceView {
        name: zenkey::implementation::resource_name(r),
        kind: r.kind,
        token: r.token.as_str().to_owned(),
        template: r.template.as_str().to_owned(),
        params: r.params.clone(),
        optional: r.optional,
        gate: r.gate.clone(),
        cardinality: r.cardinality,
        epoch: r.epoch.clone(),
        deprecated: r.deprecated.clone(),
        doc: r.doc.clone(),
        body,
    }
}

/// What asking for one revision found.
#[derive(Debug, Clone)]
pub enum ContractState {
    /// In hand, verified.
    Held(Arc<Revision>),
    /// No reply verified after both attempts (§8.4), with each refused
    /// reply's tag.
    Unavailable { refused: Vec<String> },
    /// A bundle that verified and whose contract does not read.
    Unreadable { reason: String },
}

impl ContractState {
    /// The revision, when one is held.
    pub fn revision(&self) -> Option<&Arc<Revision>> {
        match self {
            ContractState::Held(r) => Some(r),
            _ => None,
        }
    }

    /// The answer as the report spells it.
    pub fn answer(&self) -> ContractAnswer {
        match self {
            ContractState::Held(r) => ContractAnswer::Held {
                source: r.source(),
                contract: Box::new(r.view()),
            },
            ContractState::Unavailable { refused } => ContractAnswer::Unavailable {
                refused: refused.clone(),
            },
            ContractState::Unreadable { reason } => ContractAnswer::Unreadable {
                reason: reason.clone(),
            },
        }
    }
}

/// Where a view finds a revision: the retrieving store, or an offline set.
///
/// `None` is "not asked": the revision was never looked for here, which a
/// view reports as absence, never as unavailable (O4).
pub trait Contracts {
    fn state(&self, iface: &IfaceId, fingerprint: &Fingerprint) -> Option<ContractState>;
}

/// Contract revisions loaded offline: from authoring files, or from a
/// `.history` root (§9.7). No session, no retrieval: what is here is what
/// was loaded, and a revision that is not here was not asked about.
#[derive(Debug, Clone, Default)]
pub struct ContractSet {
    revisions: BTreeMap<(IfaceId, Fingerprint), Arc<Revision>>,
}

/// Something that did not load into a [`ContractSet`]: a file that is not a
/// valid contract, a bundle that does not verify, a set-level lint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadProblem {
    /// The path, or `set` for a finding about the files together.
    pub at: String,
    pub message: String,
}

impl ContractSet {
    pub fn new() -> ContractSet {
        ContractSet::default()
    }

    /// Adds a revision; a revision already held under its fingerprint is
    /// kept (the fingerprint is the identity).
    pub fn insert(&mut self, revision: Revision) -> Arc<Revision> {
        let key = (revision.iface().clone(), revision.fingerprint().clone());
        Arc::clone(
            self.revisions
                .entry(key)
                .or_insert_with(|| Arc::new(revision)),
        )
    }

    pub fn get(&self, iface: &IfaceId, fingerprint: &Fingerprint) -> Option<&Arc<Revision>> {
        self.revisions.get(&(iface.clone(), fingerprint.clone()))
    }

    /// Every revision of `iface` held, by fingerprint.
    pub fn of_iface(&self, iface: &IfaceId) -> impl Iterator<Item = &Arc<Revision>> + use<'_> {
        let iface = iface.clone();
        self.revisions
            .iter()
            .filter(move |((i, _), _)| *i == iface)
            .map(|(_, r)| r)
    }

    /// Every revision held, by interface then fingerprint.
    pub fn iter(&self) -> impl Iterator<Item = &Arc<Revision>> {
        self.revisions.values()
    }

    pub fn len(&self) -> usize {
        self.revisions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.revisions.is_empty()
    }

    /// Loads authoring files (draft 1). Every lint runs (`load_path`), and
    /// so do the set-level checks (`check_set`) over what loaded: a file
    /// that is not a valid contract is a problem, never a silent skip.
    pub fn load_files<P: AsRef<Path>>(paths: &[P]) -> (ContractSet, Vec<LoadProblem>) {
        let mut set = ContractSet::new();
        let mut problems = Vec::new();
        let mut loaded = Vec::new();
        for path in paths {
            let path = path.as_ref();
            let l = zenkey_model::contract::load_path(path);
            match l.contract {
                Some(c) => loaded.push(c),
                None => problems.push(LoadProblem {
                    at: path.display().to_string(),
                    message: l.report.to_string().trim().to_owned(),
                }),
            }
        }
        let refs: Vec<&Contract> = loaded.iter().collect();
        let report = zenkey_model::contract::check_set(&refs);
        problems.extend(report.errors().map(|d| LoadProblem {
            at: "set".to_owned(),
            message: format!("{} {}: {}", d.code, d.at, d.message),
        }));
        for c in loaded {
            set.insert(Revision::from_contract(c, ContractSource::File));
        }
        (set, problems)
    }

    /// Loads every `*.toml` directly in `dir`, in name order, as
    /// [`ContractSet::load_files`] does. A file there that is not a
    /// contract (a bindings file, say) is reported, not guessed about.
    pub fn load_dir(dir: &Path) -> (ContractSet, Vec<LoadProblem>) {
        let files: Result<Vec<PathBuf>, _> = std::fs::read_dir(dir).map(|rd| {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "toml"))
                .collect();
            v.sort();
            v
        });
        match files {
            Ok(files) => ContractSet::load_files(&files),
            Err(e) => (
                ContractSet::new(),
                vec![LoadProblem {
                    at: dir.display().to_string(),
                    message: e.to_string(),
                }],
            ),
        }
    }

    /// Loads whatever `path` is: an authoring file (`*.toml`), a directory
    /// holding authoring files, or else a `.history` root (§9.7). What a
    /// directory of authoring files is not — a bindings file beside them —
    /// is a problem, reported as [`ContractSet::load_dir`] reports it.
    pub fn load_path(path: &Path) -> (ContractSet, Vec<LoadProblem>) {
        if path.is_file() {
            return ContractSet::load_files(&[path]);
        }
        let authoring = std::fs::read_dir(path).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                let p = e.path();
                p.is_file() && p.extension().is_some_and(|x| x == "toml")
            })
        });
        if authoring {
            ContractSet::load_dir(path)
        } else {
            ContractSet::load_history(path)
        }
    }

    /// Adds every revision of `other`; one already held is kept.
    pub fn extend(&mut self, other: ContractSet) {
        for (key, r) in other.revisions {
            self.revisions.entry(key).or_insert(r);
        }
    }

    /// Loads a `.history` root (§9.7): every bundle that verifies against
    /// its file name and lies in its interface's directory. What does not —
    /// the history check's findings (`history::check_tagged`), and a
    /// verified bundle whose contract does not read — is a problem.
    pub fn load_history(root: &Path) -> (ContractSet, Vec<LoadProblem>) {
        let mut set = ContractSet::new();
        if !root.is_dir() {
            return (
                set,
                vec![LoadProblem {
                    at: root.display().to_string(),
                    message: "not a directory".to_owned(),
                }],
            );
        }
        let mut problems: Vec<LoadProblem> = zenkey_model::history::check_tagged(root)
            .into_iter()
            .map(|p| LoadProblem {
                at: p.at,
                message: format!("{}: {}", p.tag, p.message),
            })
            .collect();
        let refused: BTreeSet<String> = problems.iter().map(|p| p.at.clone()).collect();
        let mut dirs: Vec<_> = std::fs::read_dir(root)
            .map(|rd| rd.flatten().collect())
            .unwrap_or_default();
        dirs.sort_by_key(std::fs::DirEntry::file_name);
        for d in dirs {
            let name = d.file_name().to_string_lossy().into_owned();
            if IfaceId::from_str(&name).is_err() {
                continue;
            }
            let mut files: Vec<_> = std::fs::read_dir(d.path())
                .map(|rd| rd.flatten().collect())
                .unwrap_or_default();
            files.sort_by_key(std::fs::DirEntry::file_name);
            for f in files {
                let at = format!("{name}/{}", f.file_name().to_string_lossy());
                if refused.contains(&at) {
                    continue;
                }
                let Some(fp) = f
                    .file_name()
                    .to_string_lossy()
                    .strip_suffix(".bundle.json")
                    .and_then(|h| Fingerprint::parse(&format!("sha256:{h}")).ok())
                else {
                    continue;
                };
                let read = std::fs::read(f.path())
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        Bundle::verify_expecting(&bytes, &fp).map_err(|e| e.to_string())
                    })
                    .and_then(|b| {
                        Revision::from_bundle(b, ContractSource::History)
                            .map_err(|e| format!("unreadable: {e}"))
                    });
                match read {
                    Ok(r) => {
                        set.insert(r);
                    }
                    Err(message) => problems.push(LoadProblem { at, message }),
                }
            }
        }
        (set, problems)
    }
}

impl Contracts for ContractSet {
    fn state(&self, iface: &IfaceId, fingerprint: &Fingerprint) -> Option<ContractState> {
        self.get(iface, fingerprint)
            .map(|r| ContractState::Held(Arc::clone(r)))
    }
}

/// A zid by its value (§3.3, 0.11): zenoh writes it as lowercase hex
/// without leading zeros, and another writer may not, so two spellings of
/// one id compare equal here. Text that is not hex is kept as written, and
/// so matches only itself.
pub fn zid_value(z: &str) -> String {
    let t = z.trim().to_ascii_lowercase();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return z.to_owned();
    }
    match t.trim_start_matches('0') {
        "" => "0".to_owned(),
        v => v.to_owned(),
    }
}

// ─── the catalog ────────────────────────────────────────────────────────────

/// One instance, as its tokens and descriptor show it.
#[derive(Debug, Clone, Default)]
struct Instance {
    instance_token: bool,
    /// Interface tokens, by interface: the fingerprint prefixes. One per
    /// interface on a conforming instance; kept as a set so a second one is
    /// shown rather than overwritten.
    alive: BTreeMap<IfaceId, BTreeSet<Fp16>>,
    /// `None` when descriptors were not asked for.
    descriptor: Option<DescriptorRead>,
}

/// The index over one [`Observed`]: services, their instances, and what
/// tokens and descriptors say each provides and requires.
#[derive(Debug, Clone)]
pub struct Catalog {
    selector: String,
    complete: bool,
    unparsed: Vec<String>,
    services: BTreeMap<Addr, BTreeMap<InstanceId, Instance>>,
    /// Member tokens, by service: `(iface, member, epoch)`. A member's epoch
    /// is not an instance id, so they belong to the service.
    members: BTreeMap<Addr, BTreeSet<(IfaceId, String, InstanceId)>>,
    /// Every token, for [`zenkey::presence::edges`].
    tokens: Vec<ZkKey>,
}

impl Catalog {
    pub fn new(observed: &Observed) -> Catalog {
        let mut services: BTreeMap<Addr, BTreeMap<InstanceId, Instance>> = BTreeMap::new();
        let mut members: BTreeMap<Addr, BTreeSet<(IfaceId, String, InstanceId)>> = BTreeMap::new();
        for k in &observed.tokens {
            match k {
                ZkKey::Instance { addr, instance } => {
                    services
                        .entry(addr.clone())
                        .or_default()
                        .entry(instance.clone())
                        .or_default()
                        .instance_token = true;
                }
                ZkKey::Alive {
                    addr,
                    iface,
                    instance,
                    fp,
                } => {
                    services
                        .entry(addr.clone())
                        .or_default()
                        .entry(instance.clone())
                        .or_default()
                        .alive
                        .entry(iface.clone())
                        .or_default()
                        .insert(fp.clone());
                }
                ZkKey::Member {
                    addr,
                    iface,
                    member,
                    epoch,
                } => {
                    services.entry(addr.clone()).or_default();
                    members.entry(addr.clone()).or_default().insert((
                        iface.clone(),
                        member.clone(),
                        epoch.clone(),
                    ));
                }
                ZkKey::Data { .. } | ZkKey::Contract { .. } => {}
            }
        }
        if let Some(descriptors) = &observed.descriptors {
            for ((addr, instance), read) in descriptors {
                services
                    .entry(addr.clone())
                    .or_default()
                    .entry(instance.clone())
                    .or_default()
                    .descriptor = Some(read.clone());
            }
        }
        Catalog {
            selector: observed.selector.clone(),
            complete: observed.complete,
            unparsed: observed.unparsed.clone(),
            services,
            members,
            tokens: observed.tokens.clone(),
        }
    }

    /// Whether the presence read completed before its timeout (§8.1).
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// The liveliness selector the read was made with, base-relative.
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// How many services the read saw.
    pub fn service_count(&self) -> usize {
        self.services.len()
    }

    /// Whether some instance of `addr` holds its instance token in this
    /// read (§8.1).
    pub fn has_instance(&self, addr: &Addr) -> bool {
        self.services
            .get(addr)
            .is_some_and(|instances| instances.values().any(|i| i.instance_token))
    }

    /// Whether a descriptor of `addr` carries the synthetic marker a mock
    /// owner puts in `meta` (`gen`, `serve`; #612, FJ8a): traffic made to be
    /// judged, which a judge says out loud rather than paging on.
    pub fn is_synthetic(&self, addr: &Addr) -> bool {
        self.services
            .get(addr)
            .into_iter()
            .flat_map(|instances| instances.values())
            .filter_map(|i| i.descriptor.as_ref()?.descriptor())
            .any(|d| match d.meta.get("synthetic") {
                Some(serde_json::Value::Bool(b)) => *b,
                Some(serde_json::Value::Object(m)) => m.get("synthetic") == Some(&true.into()),
                _ => false,
            })
    }

    /// The session zids `addr`'s descriptors state as `meta.zid`, each by
    /// its value ([`zid_value`]): whose clock is the owner's (§3.3, 0.10).
    /// Empty when no descriptor of `addr` names one, and then a stamp's
    /// clock is unattributable, never foreign.
    pub fn owner_zids(&self, addr: &Addr) -> BTreeSet<String> {
        self.services
            .get(addr)
            .into_iter()
            .flat_map(|instances| instances.values())
            .filter_map(|i| i.descriptor.as_ref()?.descriptor())
            .filter_map(|d| d.meta.get("zid").and_then(|v| v.as_str()))
            .map(zid_value)
            .collect()
    }

    /// Every service address seen, sorted.
    pub fn addresses(&self) -> impl Iterator<Item = &Addr> {
        self.services.keys()
    }

    /// Every descriptor served, by address then instance.
    pub fn descriptors(&self) -> impl Iterator<Item = &Descriptor> {
        self.services
            .values()
            .flat_map(|instances| instances.values())
            .filter_map(|i| i.descriptor.as_ref()?.descriptor())
    }

    /// The revisions the descriptors name, deduplicated: what a
    /// [`BundleStore`](crate::bus::contracts::BundleStore) retrieves before
    /// a view is built. A token's fingerprint prefix cannot be retrieved by
    /// (§8.4), so only descriptors name revisions.
    pub fn wanted(&self) -> Vec<(IfaceId, Fingerprint)> {
        let set: BTreeSet<(IfaceId, Fingerprint)> = self
            .descriptors()
            .flat_map(|d| d.interfaces.iter())
            .filter_map(|e| {
                Some((
                    IfaceId::from_str(&e.iface).ok()?,
                    Fingerprint::parse(&e.contract).ok()?,
                ))
            })
            .collect();
        set.into_iter().collect()
    }

    /// Whether any instance of `addr` provides `iface`, by token or by
    /// descriptor.
    pub fn provides(&self, addr: &Addr, iface: &IfaceId) -> bool {
        self.services.get(addr).is_some_and(|instances| {
            instances.values().any(|i| {
                i.alive.contains_key(iface)
                    || i.descriptor
                        .as_ref()
                        .and_then(DescriptorRead::descriptor)
                        .is_some_and(|d| d.interfaces.iter().any(|e| e.iface == iface.to_string()))
            })
        })
    }

    /// The revisions of `iface` that `addr`'s instances' descriptors name.
    /// More than one is a rolling upgrade or a split brain; a data key
    /// names no instance, so a decode cannot choose between them.
    pub fn revisions_of(&self, addr: &Addr, iface: &IfaceId) -> BTreeSet<Fingerprint> {
        let want = iface.to_string();
        self.services
            .get(addr)
            .into_iter()
            .flat_map(|instances| instances.values())
            .filter_map(|i| i.descriptor.as_ref()?.descriptor())
            .flat_map(|d| d.interfaces.iter())
            .filter(|e| e.iface == want)
            .filter_map(|e| Fingerprint::parse(&e.contract).ok())
            .collect()
    }

    /// `service list`: every service, instance by instance, each interface
    /// with what its token and its descriptor say.
    pub fn services(&self) -> ServiceListing {
        let services = self
            .services
            .iter()
            .map(|(addr, instances)| ServiceSighting {
                address: addr.to_string(),
                instances: instances
                    .iter()
                    .map(|(id, inst)| InstanceSighting {
                        instance: id.to_string(),
                        instance_token: inst.instance_token,
                        interfaces: iface_rows(inst),
                        descriptor: inst.descriptor.as_ref().map(DescriptorRead::answer).into(),
                    })
                    .collect(),
                members: self
                    .members
                    .get(addr)
                    .into_iter()
                    .flatten()
                    .map(|(iface, member, epoch)| MemberSighting {
                        iface: iface.to_string(),
                        member: member.clone(),
                        epoch: epoch.to_string(),
                    })
                    .collect(),
            })
            .collect();
        ServiceListing {
            selector: self.selector.clone(),
            complete: self.complete,
            services,
            unparsed: self.unparsed.clone(),
        }
    }

    /// `service show`: one address, as [`Catalog::services`] shows it. An
    /// address presence did not show has no instances, and the view says
    /// whether the read was complete enough for that to mean anything.
    pub fn service(&self, addr: &Addr) -> ServiceView {
        let want = addr.to_string();
        let found = self
            .services()
            .services
            .into_iter()
            .find(|s| s.address == want);
        let (instances, members) = found.map_or((vec![], vec![]), |s| (s.instances, s.members));
        ServiceView {
            address: want,
            selector: self.selector.clone(),
            complete: self.complete,
            instances,
            members,
            unparsed: self.unparsed.clone(),
        }
    }

    /// `iface list`: every interface a token or a descriptor names as
    /// provided, or a descriptor's role requires, with who and at which
    /// revisions.
    pub fn ifaces(&self) -> IfaceListing {
        #[derive(Default)]
        struct Seen {
            providers: BTreeSet<String>,
            consumers: BTreeSet<String>,
            revisions: BTreeSet<String>,
            tokenless: bool,
        }
        let mut by: BTreeMap<String, Seen> = BTreeMap::new();
        for (addr, instances) in &self.services {
            for inst in instances.values() {
                for row in iface_rows(inst) {
                    let seen = by.entry(row.iface).or_default();
                    seen.providers.insert(addr.to_string());
                    seen.revisions.extend(row.contract);
                    seen.tokenless |= row.tokenless;
                }
                let requires = inst
                    .descriptor
                    .as_ref()
                    .and_then(DescriptorRead::descriptor)
                    .map(|d| d.requires.as_slice())
                    .unwrap_or_default();
                for r in requires {
                    by.entry(r.interface.clone())
                        .or_default()
                        .consumers
                        .insert(addr.to_string());
                }
            }
        }
        IfaceListing {
            selector: self.selector.clone(),
            complete: self.complete,
            interfaces: by
                .into_iter()
                .map(|(iface, s)| IfaceSummary {
                    iface,
                    providers: s.providers.into_iter().collect(),
                    consumers: s.consumers.into_iter().collect(),
                    revisions: s.revisions.into_iter().collect(),
                    tokenless: s.tokenless,
                })
                .collect(),
            undescribed: self.undescribed(),
        }
    }

    /// Every fingerprint the descriptors name for `iface`, across every
    /// address: what an `<iface>@<prefix>` is resolved against.
    pub fn fingerprints_of(&self, iface: &IfaceId) -> BTreeSet<Fingerprint> {
        self.services
            .keys()
            .flat_map(|addr| self.revisions_of(addr, iface))
            .collect()
    }

    /// `iface show <iface>@<fingerprint>`: [`Catalog::iface`] narrowed to
    /// one revision — the providers whose descriptor names it, or whose
    /// token carries its prefix when no descriptor was read — and that
    /// revision's contract even when no provider names it now.
    pub fn iface_at(
        &self,
        iface: &IfaceId,
        fingerprint: &Fingerprint,
        contracts: &dyn Contracts,
    ) -> IfaceView {
        let mut view = self.iface(iface, contracts);
        let full = fingerprint.to_string();
        let prefix = fingerprint.hex().fp16().to_string();
        view.providers.retain(|p| match &p.contract {
            Some(c) => *c == full,
            None => p.token.as_deref() == Some(prefix.as_str()),
        });
        view.revisions.retain(|r| r.fingerprint == full);
        if view.revisions.is_empty() {
            view.revisions.push(IfaceRevision {
                fingerprint: full,
                providers: vec![],
                contract: contracts
                    .state(iface, fingerprint)
                    .map(|s| s.answer())
                    .into(),
            });
        }
        view
    }

    /// `iface show`: who provides `iface` at which revision, who requires
    /// it, and each revision's contract as `contracts` has it. An empty
    /// [`ContractSet`] asks nothing, and every contract is then absent.
    pub fn iface(&self, iface: &IfaceId, contracts: &dyn Contracts) -> IfaceView {
        let want = iface.to_string();
        let mut providers = Vec::new();
        let mut consumers = Vec::new();
        let mut revisions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (addr, instances) in &self.services {
            for (id, inst) in instances {
                let descriptor = inst
                    .descriptor
                    .as_ref()
                    .and_then(DescriptorRead::descriptor);
                for row in iface_rows(inst).into_iter().filter(|r| r.iface == want) {
                    let entry = descriptor.and_then(|d| {
                        d.interfaces
                            .iter()
                            .find(|e| Some(&e.contract) == row.contract.as_ref())
                    });
                    let exposes = match (descriptor, entry) {
                        (Some(d), Some(e)) => Fingerprint::parse(&e.contract)
                            .ok()
                            .and_then(|fp| contracts.state(iface, &fp))
                            .and_then(|s| s.revision().cloned())
                            .map(|r| exposure(r.contract(), e, &d.capabilities))
                            .into(),
                        _ => Asked::NotAsked,
                    };
                    if let Some(fp) = &row.contract {
                        revisions
                            .entry(fp.clone())
                            .or_default()
                            .push(format!("{addr}@{id}"));
                    }
                    providers.push(IfaceProvider {
                        address: addr.to_string(),
                        instance: id.to_string(),
                        token: row.token,
                        contract: row.contract,
                        minor: row.minor,
                        tokenless: row.tokenless,
                        unavailable: entry.map(|e| e.unavailable.clone()).unwrap_or_default(),
                        cardinality: entry.map(|e| e.cardinality.clone()).unwrap_or_default(),
                        exposes,
                    });
                }
                for r in descriptor
                    .into_iter()
                    .flat_map(|d| d.requires.iter())
                    .filter(|r| r.interface == want)
                {
                    consumers.push(IfaceConsumer {
                        address: addr.to_string(),
                        instance: id.to_string(),
                        role: r.role.clone(),
                        bindings: r.bindings.clone(),
                        params: r.params.clone(),
                        declared_by: r.declared_by.clone(),
                    });
                }
            }
        }
        let revisions = revisions
            .into_iter()
            .map(|(fp, providers)| IfaceRevision {
                contract: Fingerprint::parse(&fp)
                    .ok()
                    .and_then(|f| contracts.state(iface, &f))
                    .map(|s| s.answer())
                    .into(),
                fingerprint: fp,
                providers,
            })
            .collect();
        IfaceView {
            iface: want,
            complete: self.complete,
            providers,
            consumers,
            revisions,
            undescribed: self.undescribed(),
        }
    }

    /// The edges of the data-flow graph (R3), exactly as the runtime
    /// computes them from these descriptors and tokens.
    pub fn edges(&self) -> Vec<zenkey::presence::Edge> {
        let descriptors: Vec<Descriptor> = self.descriptors().cloned().collect();
        zenkey::presence::edges(&descriptors, &self.tokens)
    }

    /// `graph`: every service as a node, and every binding that matches a
    /// provider present now as an edge ([`Catalog::edges`]).
    pub fn graph(&self) -> BindingGraph {
        let mut nodes = Vec::new();
        let mut role_iface: BTreeMap<(String, String), String> = BTreeMap::new();
        for (addr, instances) in &self.services {
            let mut provides = BTreeSet::new();
            let mut requires = BTreeMap::new();
            for inst in instances.values() {
                provides.extend(inst.alive.keys().map(ToString::to_string));
                if let Some(d) = inst
                    .descriptor
                    .as_ref()
                    .and_then(DescriptorRead::descriptor)
                {
                    provides.extend(d.interfaces.iter().map(|e| e.iface.clone()));
                    for r in &d.requires {
                        role_iface
                            .entry((addr.to_string(), r.role.clone()))
                            .or_insert_with(|| r.interface.clone());
                        requires.entry(r.role.clone()).or_insert_with(|| GraphRole {
                            interface: r.interface.clone(),
                            bindings: r.bindings.clone(),
                        });
                    }
                }
            }
            nodes.push(GraphNode {
                address: addr.to_string(),
                instances: instances.len(),
                provides: provides.into_iter().collect(),
                requires,
            });
        }
        let mut edges: Vec<GraphEdge> = self
            .edges()
            .into_iter()
            .map(|e| GraphEdge {
                interface: role_iface
                    .get(&(e.consumer.clone(), e.role.clone()))
                    .cloned()
                    .unwrap_or_default(),
                consumer: e.consumer,
                role: e.role,
                provider: e.provider,
            })
            .collect();
        edges.sort();
        BindingGraph {
            complete: self.complete,
            nodes,
            edges,
            undescribed: self.undescribed(),
        }
    }

    /// Instances whose descriptor was asked for and did not read.
    fn undescribed(&self) -> Vec<InstanceRef> {
        self.services
            .iter()
            .flat_map(|(addr, instances)| {
                instances.iter().filter_map(move |(id, inst)| {
                    let read = inst.descriptor.as_ref()?;
                    read.descriptor().is_none().then(|| InstanceRef {
                        address: addr.to_string(),
                        instance: id.to_string(),
                        descriptor: read.tag().to_owned(),
                    })
                })
            })
            .collect()
    }
}

/// `namespace list`: the instance tokens of one un-namespaced read, by the
/// namespace each sits under — the chunks before its last six, which are
/// `zk2/<system>/<service>/@zk/instance/<id>`. A key whose last six chunks
/// are not an instance token is kept verbatim in `unparsed`.
pub fn namespaces(
    selector: impl Into<String>,
    keys: &[String],
    complete: bool,
) -> NamespaceListing {
    let mut by: BTreeMap<String, (BTreeSet<String>, usize)> = BTreeMap::new();
    let mut unparsed = Vec::new();
    for key in keys {
        let chunks: Vec<&str> = key.split('/').collect();
        let parsed = chunks
            .len()
            .checked_sub(6)
            .map(|at| chunks.split_at(at))
            .and_then(
                |(ns, token)| match zenkey_model::grammar::parse(&token.join("/")) {
                    Ok(ZkKey::Instance { addr, .. }) => Some((ns.join("/"), addr)),
                    _ => None,
                },
            );
        match parsed {
            Some((ns, addr)) => {
                let (services, instances) = by.entry(ns).or_default();
                services.insert(addr.to_string());
                *instances += 1;
            }
            None => unparsed.push(key.clone()),
        }
    }
    NamespaceListing {
        selector: selector.into(),
        complete,
        namespaces: by
            .into_iter()
            .map(|(namespace, (services, instances))| NamespaceSighting {
                namespace,
                services: services.into_iter().collect(),
                instances,
            })
            .collect(),
        unparsed,
    }
}

/// One instance's interface rows: each interface token, then each
/// descriptor entry, merged where they agree — same interface, and the
/// token's prefix is the descriptor fingerprint's. Where they disagree both
/// rows stand: which one is wrong is a judgement, not this projection's.
fn iface_rows(inst: &Instance) -> Vec<IfaceSighting> {
    let mut rows: Vec<IfaceSighting> = inst
        .alive
        .iter()
        .flat_map(|(iface, fps)| {
            fps.iter().map(move |fp| IfaceSighting {
                iface: iface.to_string(),
                token: Some(fp.to_string()),
                contract: None,
                minor: None,
                tokenless: false,
            })
        })
        .collect();
    let entries = inst
        .descriptor
        .as_ref()
        .and_then(DescriptorRead::descriptor)
        .map(|d| d.interfaces.as_slice())
        .unwrap_or_default();
    for e in entries {
        let prefix = Fingerprint::parse(&e.contract)
            .ok()
            .map(|f| f.hex().fp16().to_string());
        let merged = rows.iter_mut().find(|r| {
            r.iface == e.iface && r.contract.is_none() && r.token.is_some() && r.token == prefix
        });
        match merged {
            Some(r) => {
                r.contract = Some(e.contract.clone());
                r.minor = Some(e.minor);
                r.tokenless = !e.token;
            }
            None => rows.push(IfaceSighting {
                iface: e.iface.clone(),
                token: None,
                contract: Some(e.contract.clone()),
                minor: Some(e.minor),
                tokenless: !e.token,
            }),
        }
    }
    rows.sort_by(|a, b| (&a.iface, &a.token, &a.contract).cmp(&(&b.iface, &b.token, &b.contract)));
    rows
}

/// The compact exposure rule (r3.3 D8, §3.3): the contract's resources,
/// minus the optional ones gated on a capability not held, minus the listed
/// exceptions.
fn exposure(c: &Contract, entry: &InterfaceEntry, held: &[String]) -> Vec<String> {
    c.resources
        .iter()
        .filter(|r| {
            let gated_off = r.optional
                && r.gate.iter().any(|g| {
                    g.strip_prefix("capability:")
                        .is_some_and(|cap| !held.iter().any(|h| h == cap))
                });
            let name = zenkey::implementation::resource_name(r);
            !gated_off && !entry.unavailable.iter().any(|u| u.resource == name)
        })
        .map(zenkey::implementation::resource_name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NETIF_FP: &str = "abababababababababababababababababababababababababababababababab";

    fn descriptor(service: &str, instance: &str, value: serde_json::Value) -> Descriptor {
        let mut v = json!({
            "format": "zk2-descriptor/0.1",
            "service": service,
            "instance": instance,
            "interfaces": [],
        });
        v.as_object_mut()
            .expect("an object")
            .extend(value.as_object().expect("an object").clone());
        serde_json::from_value(v).expect("a descriptor")
    }

    fn observed(keys: &[&str]) -> Observed {
        let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
        Observed::from_keys("zk2/*/*/@zk/**", &keys, true)
    }

    /// A token and a descriptor that agree are one row; a tokenless
    /// interface is a row with no token; a key under `@zk` that is not a
    /// token is shown, not dropped.
    #[test]
    fn tokens_and_descriptors_merge_where_they_agree() {
        let inst = "8f3a5c2e9b1d4f70";
        let mut o = observed(&[
            &format!("zk2/host-a/tc/@zk/instance/{inst}"),
            &format!(
                "zk2/host-a/tc/@zk/alive/tc.netif.v1/{inst}/{}",
                &NETIF_FP[..16]
            ),
            "zk2/host-a/tc/@zk/alive/nope/x/y",
        ]);
        let health = "cd".repeat(32);
        o.descriptors = Some(BTreeMap::from([(
            (
                "host-a/tc".parse().expect("addr"),
                inst.parse().expect("id"),
            ),
            DescriptorRead::Served(Box::new(descriptor(
                "host-a/tc",
                inst,
                json!({"interfaces": [
                    {"iface": "tc.netif.v1", "contract": format!("sha256:{NETIF_FP}"), "minor": 1},
                    {"iface": "health.v1", "contract": format!("sha256:{health}"), "minor": 0, "token": false},
                ]}),
            ))),
        )]));
        let listing = Catalog::new(&o).services();
        assert_eq!(listing.unparsed, ["zk2/host-a/tc/@zk/alive/nope/x/y"]);
        let rows = &listing.services[0].instances[0].interfaces;
        assert_eq!(
            rows,
            &[
                IfaceSighting {
                    iface: "health.v1".into(),
                    token: None,
                    contract: Some(format!("sha256:{health}")),
                    minor: Some(0),
                    tokenless: true,
                },
                IfaceSighting {
                    iface: "tc.netif.v1".into(),
                    token: Some(NETIF_FP[..16].into()),
                    contract: Some(format!("sha256:{NETIF_FP}")),
                    minor: Some(1),
                    tokenless: false,
                },
            ]
        );
    }

    /// A token whose prefix is not the descriptor's fingerprint stays a row
    /// of its own: the projection does not decide which one is right.
    #[test]
    fn a_disagreeing_token_is_not_merged() {
        let inst = "8f3a5c2e9b1d4f70";
        let mut o = observed(&[&format!(
            "zk2/host-a/tc/@zk/alive/tc.netif.v1/{inst}/0000000000000000"
        )]);
        o.descriptors = Some(BTreeMap::from([(
            (
                "host-a/tc".parse().expect("addr"),
                inst.parse().expect("id"),
            ),
            DescriptorRead::Served(Box::new(descriptor(
                "host-a/tc",
                inst,
                json!({"interfaces": [
                    {"iface": "tc.netif.v1", "contract": format!("sha256:{NETIF_FP}"), "minor": 1},
                ]}),
            ))),
        )]));
        let listing = Catalog::new(&o).services();
        let i = &listing.services[0].instances[0];
        assert!(!i.instance_token, "known by its interface token alone");
        assert_eq!(i.interfaces.len(), 2);
    }

    /// `namespace list`'s projection: the prefix before the last six chunks
    /// is the namespace (empty at the bus root), and a key whose tail is not
    /// an instance token is kept, verbatim.
    #[test]
    fn namespaces_are_the_prefix_before_the_instance_token() {
        let inst = "8f3a5c2e9b1d4f70";
        let keys: Vec<String> = [
            format!("zk2/host-a/tc/@zk/instance/{inst}"),
            format!("site/prod/zk2/host-a/tc/@zk/instance/{inst}"),
            format!("site/prod/zk2/host-a/tc/@zk/instance/{}", "0".repeat(16)),
            format!("site/prod/zk2/host-b/tc/@zk/instance/{inst}"),
            "short/key".to_owned(),
            "x/zk2/host-a/tc/@zk/instance/NOT-HEX".to_owned(),
        ]
        .into();
        let l = namespaces("**/zk2/*/*/@zk/instance/*", &keys, true);
        let got: Vec<(&str, Vec<&str>, usize)> = l
            .namespaces
            .iter()
            .map(|n| {
                (
                    n.namespace.as_str(),
                    n.services.iter().map(String::as_str).collect(),
                    n.instances,
                )
            })
            .collect();
        assert_eq!(
            got,
            [
                ("", vec!["host-a/tc"], 1),
                ("site/prod", vec!["host-a/tc", "host-b/tc"], 3),
            ]
        );
        assert_eq!(l.unparsed.len(), 2, "{:?}", l.unparsed);
    }

    /// `iface list`: a provider by token and descriptor, a tokenless one, and
    /// an interface only required — each named once, with who and where.
    #[test]
    fn ifaces_name_providers_consumers_and_revisions() {
        let (a, b) = ("8f3a5c2e9b1d4f70", "0000000000000002");
        let mut o = observed(&[
            &format!("zk2/host-a/tc/@zk/instance/{a}"),
            &format!(
                "zk2/host-a/tc/@zk/alive/tc.netif.v1/{a}/{}",
                &NETIF_FP[..16]
            ),
            &format!("zk2/ws-01/gui/@zk/instance/{b}"),
        ]);
        let health = "cd".repeat(32);
        o.descriptors = Some(BTreeMap::from([
            (
                ("host-a/tc".parse().expect("addr"), a.parse().expect("id")),
                DescriptorRead::Served(Box::new(descriptor(
                    "host-a/tc",
                    a,
                    json!({"interfaces": [
                        {"iface": "tc.netif.v1", "contract": format!("sha256:{NETIF_FP}"), "minor": 1},
                        {"iface": "health.v1", "contract": format!("sha256:{health}"), "minor": 0, "token": false},
                    ]}),
                ))),
            ),
            (
                ("ws-01/gui".parse().expect("addr"), b.parse().expect("id")),
                DescriptorRead::Served(Box::new(descriptor(
                    "ws-01/gui",
                    b,
                    json!({"requires": [
                        {"role": "netif", "interface": "tc.netif.v1", "bindings": ["*/tc"]},
                        {"role": "scenario", "interface": "tc.scenario.v1", "bindings": ["*/tc"]},
                    ]}),
                ))),
            ),
        ]));
        let c = Catalog::new(&o);
        let l = c.ifaces();
        let rows: Vec<(&str, usize, usize, usize, bool)> = l
            .interfaces
            .iter()
            .map(|i| {
                (
                    i.iface.as_str(),
                    i.providers.len(),
                    i.consumers.len(),
                    i.revisions.len(),
                    i.tokenless,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("health.v1", 1, 0, 1, true),
                ("tc.netif.v1", 1, 1, 1, false),
                ("tc.scenario.v1", 0, 1, 0, false),
            ]
        );

        // One revision: its providers; another nobody names: no providers,
        // and its contract not asked (an empty set asks nothing).
        let netif: IfaceId = "tc.netif.v1".parse().expect("iface");
        let fp = Fingerprint::parse(&format!("sha256:{NETIF_FP}")).expect("fp");
        let at = c.iface_at(&netif, &fp, &ContractSet::new());
        assert_eq!(at.providers.len(), 1);
        assert_eq!(at.consumers.len(), 1);
        let other = Fingerprint::parse(&format!("sha256:{}", "e".repeat(64))).expect("fp");
        let at = c.iface_at(&netif, &other, &ContractSet::new());
        assert!(at.providers.is_empty());
        assert_eq!(at.revisions.len(), 1);
        assert!(at.revisions[0].contract.is_not_asked());
        assert_eq!(c.fingerprints_of(&netif), BTreeSet::from([fp]));

        let view = c.service(&"ws-01/gui".parse().expect("addr"));
        assert_eq!(view.instances.len(), 1);
        let none = c.service(&"ws-02/gui".parse().expect("addr"));
        assert!(none.instances.is_empty() && none.complete);
    }

    /// `schema show`: the whole revision names every artifact; one resource
    /// names its members' types and only the artifacts they live in, a
    /// protobuf set read as messages; a template alone finds its resource,
    /// and one the revision does not declare is refused with the list.
    #[test]
    fn a_schema_view_reads_the_bundle_s_artifacts() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/zk2/walkthrough/camera.v1.toml");
        let (set, problems) = ContractSet::load_path(&path);
        assert!(problems.is_empty(), "{problems:?}");
        let r = set.iter().next().expect("camera.v1");

        let whole = r.schema_view(None, false).expect("whole");
        assert!(whole.resource.is_none());
        assert_eq!(whole.artifacts.len(), 1);
        assert!(whole.artifacts[0].document.is_not_asked());

        let image = r.schema_view(Some("image"), true).expect("by template");
        assert_eq!(image.resource.as_deref(), Some("@stream/image"));
        let members: Vec<(&str, &str, &str)> = image
            .members
            .iter()
            .map(|m| {
                (
                    m.member.as_str(),
                    m.type_.kind.as_str(),
                    m.type_.name.as_str(),
                )
            })
            .collect();
        assert_eq!(
            members,
            [
                ("type", "raw", "image/jpeg"),
                ("attachment", "protobuf", "camera.v1.FrameMeta")
            ]
        );
        let Asked::Asked(doc) = &image.artifacts[0].document else {
            panic!("a named resource carries its documents");
        };
        let messages: Vec<&str> = doc["files"]
            .as_array()
            .expect("files")
            .iter()
            .flat_map(|f| f["messages"].as_array().into_iter().flatten())
            .filter_map(|m| m["name"].as_str())
            .collect();
        assert!(messages.contains(&"camera.v1.FrameMeta"), "{doc}");

        let refused = r
            .schema_view(Some("nope"), false)
            .expect_err("not declared");
        assert!(refused.contains("@stream/image"), "{refused}");
    }

    /// A silent descriptor is an answer of its own, and the instance is
    /// named as undescribed wherever a tokenless provider could hide.
    #[test]
    fn a_silent_descriptor_is_named() {
        let inst = "8f3a5c2e9b1d4f70";
        let mut o = observed(&[&format!("zk2/host-a/tc/@zk/instance/{inst}")]);
        o.descriptors = Some(BTreeMap::from([(
            (
                "host-a/tc".parse().expect("addr"),
                inst.parse().expect("id"),
            ),
            DescriptorRead::Silent,
        )]));
        let c = Catalog::new(&o);
        assert_eq!(
            c.services().services[0].instances[0].descriptor,
            Asked::Asked(DescriptorAnswer::Silent)
        );
        let view = c.iface(&"tc.netif.v1".parse().expect("iface"), &ContractSet::new());
        assert_eq!(view.undescribed.len(), 1);
        assert_eq!(view.undescribed[0].descriptor, "silent");
        assert_eq!(c.graph().undescribed.len(), 1);
    }
}
