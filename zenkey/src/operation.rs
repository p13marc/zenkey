//! Serving operations (spec §5, §6): a `complete` queryable per operation,
//! wrapped so that the rules are the runtime's, not each handler's.
//!
//! ```text
//! a call ─► O2: not concrete, and fanout forbidden?  ─ reply_err fanout_forbidden
//!        ─► O3: optional, and not exposed here?      ─ reply_err unavailable + cause
//!        ─► the handler, on the server's tokio runtime:
//!             reply / reply_value      one value ("one"), any number ("many")
//!             summary / summary_value  O6: the replier's last reply,
//!                                      attachment `summary`
//!             refuse(OpError)          O3: the error envelope (§5.2)
//!        ─► the handler returned: an error is refused for it, and a call
//!           left without its reply gets `internal`, never silence
//! ```
//!
//! - **O1:** the queryable is declared on the concrete key or on the
//!   template, and is `complete`. A concrete call with `BestMatching` then
//!   runs on at most one instance **while one instance serves**. A
//!   split-brain across routers runs a call on each side: exclusivity across
//!   instances is `redundancy.v1`'s (#613), never the core's, and
//!   [`crate::ownership`] diagnoses it.
//! - **`serving = "replicated"`** changes nothing here. Any number of
//!   instances may serve, at one execution per router per call, which is why
//!   the contract requires `idempotent` (E018).
//! - **O3:** a value reply goes on the operation's own concrete key, and a
//!   failure is a `reply_err` carrying the envelope, encoded as §5.2 says;
//!   a refusal that does not fit its envelope goes out as `internal`.
//!   The active instance answers `unavailable`, with its cause, for every
//!   optional operation it does not expose; a standby declares nothing, and
//!   a replica nothing on an exclusive operation (§5.1, "Beside replicas").
//! - **Over a template** (§5.1, 0.7): a fan-out to a server declared over
//!   the whole template replies on the key of the member it answers for,
//!   named by the key when it binds every parameter, else by the handler
//!   ([`Call::member`], [`crate::CallInfo::member`]); one that names none
//!   is answered `internal`. A concrete key that names no member is
//!   `invalid_request`.
//! - **O7:** the call metadata (`actor`, `request_id`) is read from the
//!   request's attachment. It is claimed, never authenticated.
//!
//! Calling is [`crate::client`]'s.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use serde::Serialize;
use serde::de::DeserializeOwned;
use zenkey_model::authoring::{Encoding as WireEncoding, Kind, Serving};
use zenkey_model::contract::{Body, Contract, Fanout, Operation, Replies, Resource};
use zenkey_model::descriptor::Cause;
use zenkey_model::envelope::{self, Detail, Envelope};
use zenkey_model::grammar::{Addr, IfaceId, KindToken, ZkKey, parse};
use zenkey_model::schema::TypeId;
use zenkey_model::slug::chunk_unslug;
use zenkey_model::template::{Bindings, Segment, Template};
use zenoh::Wait;
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};
use zenoh::query::{Query, Queryable};

pub use crate::call::{CallMetadata, OpError, cause_name};
use crate::client::Client;
use crate::error::{Error, Result, zenoh};
use crate::implementation::{missing_capability, resource_name};
use crate::service::{ImplState, Service, ServiceBuilder, exposes_any};
use crate::writer::{encode_value, wire_encoding};

/// The attachment that marks a summary reply (O6).
pub const SUMMARY: &[u8] = b"summary";

/// The schema suffix of a protobuf envelope (§5.2).
const PROTOBUF_ERROR: &str = "zk2.core.v1.Error";

/// The operation body of a resource.
pub(crate) fn operation(iface: &IfaceId, r: &Resource) -> Result<Operation> {
    match &r.body {
        Body::Operation(op) => Ok(op.clone()),
        Body::Data(_) => Err(Error::Contract(format!(
            "{iface} {:?} is not an operation",
            resource_name(r)
        ))),
    }
}

/// The resource named `<kind token>/<template>` in `contract`.
pub(crate) fn find_resource<'c>(contract: &'c Contract, name: &str) -> Result<&'c Resource> {
    name.split_once('/')
        .and_then(|(t, tpl)| contract.resource(t.parse().ok()?, tpl))
        .ok_or_else(|| Error::NoResource {
            iface: contract.iface.clone(),
            resource: name.to_owned(),
        })
}

/// The envelope's encoding for `op` (§5.2): its `error` type's kind when it
/// declares one, else its `response` type's.
#[must_use]
pub fn envelope_encoding(op: &Operation) -> &'static str {
    match op.error.as_ref().unwrap_or(&op.response) {
        TypeId::JsonSchema { .. } => match op.encoding {
            Some(WireEncoding::Cbor) => envelope::CBOR,
            _ => envelope::JSON,
        },
        TypeId::Protobuf { .. } => envelope::PROTOBUF,
        TypeId::Raw { .. } => envelope::JSON,
    }
}

/// The reply `Encoding` an envelope encoding travels with (§5.2).
fn envelope_zenoh_encoding(e: &str) -> Encoding {
    match e {
        envelope::CBOR => Encoding::APPLICATION_CBOR,
        envelope::PROTOBUF => Encoding::APPLICATION_PROTOBUF.with_schema(PROTOBUF_ERROR),
        _ => Encoding::APPLICATION_JSON,
    }
}

/// The envelope as `op`'s envelope carries it (§5.2), or `None` for a
/// detail that does not fit, which is never sent:
/// - any detail on an operation that declares no `error` type;
/// - a raw `error` type's detail that is not its bytes: bytes go as base64
///   text, and a value only when it is that text;
/// - bytes in a JSON or CBOR envelope, or a value in a protobuf one, which
///   [`envelope::encode`] refuses.
fn fit(op: &Operation, env: &Envelope) -> Option<Envelope> {
    let detail = match (&env.detail, &op.error) {
        (None, _) => None,
        (Some(_), None) => return None,
        (Some(Detail::Bytes(b)), Some(TypeId::Raw { .. })) => Some(Detail::raw(b)),
        (Some(d), Some(TypeId::Raw { .. })) => Some(d.clone()).filter(|d| d.raw_bytes().is_some()),
        (Some(d), Some(_)) => Some(d.clone()),
    };
    if env.detail.is_some() && detail.is_none() {
        return None;
    }
    Some(Envelope {
        detail,
        ..env.clone()
    })
}

/// The envelope's bytes and `Encoding` for `op` (§5.2).
///
/// A refusal the reference cannot send as given goes out as `internal`
/// instead, never as a malformed envelope (§5.2, 0.7, F-65):
/// - a detail that does not fit ([`fit`]): on an operation with no `error`
///   type, bytes in a JSON or CBOR envelope other than a raw type's, a value
///   in a protobuf one;
/// - an envelope a tool's decoder would refuse (§5.2, "Decoding"): an
///   unknown code, `unavailable` without a valid cause or a cause on
///   another code, a detail on a code but `app`. Only an envelope relayed
///   with `From<Envelope>` can be one.
pub(crate) fn encode_envelope(op: &Operation, e: &OpError) -> (Vec<u8>, Encoding) {
    let enc = envelope_encoding(op);
    let sent = fit(op, e.envelope())
        .and_then(|env| envelope::encode(&env, enc))
        .filter(|bytes| envelope::decode(enc, bytes).is_ok());
    let bytes = sent.unwrap_or_else(|| {
        tracing::warn!(
            code = e.envelope().code,
            encoding = enc,
            "a refusal does not fit the operation's envelope; sent as internal (spec §5.2)"
        );
        let fallback = OpError::internal(format!(
            "the {} refusal does not fit this operation's envelope (§5.2)",
            e.envelope().code
        ));
        envelope::encode(fallback.envelope(), enc).expect("an envelope without a detail encodes")
    });
    (bytes, envelope_zenoh_encoding(enc))
}

/// Decodes a JSON Schema value by its sample's `Encoding`, else by the
/// contract's `encoding` (§7.2's decode order).
pub(crate) fn decode_value<T: DeserializeOwned>(
    bytes: &[u8],
    sample: Option<&Encoding>,
    wire: Option<WireEncoding>,
) -> std::result::Result<T, String> {
    let sample = sample.map(ToString::to_string);
    crate::codec::decode_json(bytes, crate::codec::wire_of(sample.as_deref(), wire))
}

/// Every member of an operation's template on `addr`: each parameter
/// becomes `*`, a rest parameter `**`.
pub(crate) fn pattern_of(addr: &Addr, iface: &IfaceId, r: &Resource) -> Result<OwnedKeyExpr> {
    let mut chunks = vec![
        "zk2".to_owned(),
        addr.system.to_string(),
        addr.service.to_string(),
        iface.to_string(),
        r.token.to_string(),
    ];
    chunks.extend(r.template.segments().iter().map(|seg| match seg {
        Segment::Literal(l) => l.clone(),
        Segment::Param(_) => "*".to_owned(),
        Segment::Rest(_) => "**".to_owned(),
    }));
    OwnedKeyExpr::try_from(chunks.join("/")).map_err(zenoh)
}

/// The template values of a concrete operation key of `iface`, if it is one
/// of `r`'s members.
pub(crate) fn values_of(iface: &IfaceId, r: &Resource, key: &str) -> Option<(Addr, Bindings)> {
    match parse(key).ok()? {
        ZkKey::Data {
            addr,
            iface: got,
            kind: KindToken::Op,
            resource,
        } if got == *iface => {
            let refs: Vec<&str> = resource.iter().map(String::as_str).collect();
            Some((addr, r.template.matches(&refs)?))
        }
        _ => None,
    }
}

/// What a key expression's resource chunks bind of `template` (§5.1, "Over
/// a template"): each parameter at a concrete chunk, unslugged, and none at
/// a wildcard. Past a `**` chunk, positions are not fixed, and nothing more
/// is bound. `Err` when a concrete parameter chunk is not a canonical slug
/// (§1.4): the key names no member, and is malformed.
pub(crate) fn bound_by(
    template: &Template,
    chunks: &[&str],
) -> std::result::Result<Bindings, String> {
    let unslug = |c: &str| {
        chunk_unslug(c).ok_or_else(|| format!("chunk {c:?} is not a canonical slug (§1.4)"))
    };
    let mut out = Bindings::new();
    for (i, seg) in template.segments().iter().enumerate() {
        let Some(&c) = chunks.get(i) else { break };
        if c == "**" {
            break;
        }
        match seg {
            Segment::Literal(_) => {}
            Segment::Param(n) => {
                if !c.contains('*') {
                    out.insert(n.clone(), vec![unslug(c)?]);
                }
            }
            Segment::Rest(n) => {
                let rest = &chunks[i..];
                if rest.iter().all(|c| !c.contains('*')) {
                    let values = rest
                        .iter()
                        .map(|c| unslug(c))
                        .collect::<std::result::Result<_, _>>()?;
                    out.insert(n.clone(), values);
                }
                break;
            }
        }
    }
    Ok(out)
}

/// The member a call answers for (O3): its concrete key, and its template
/// values, unslugged.
#[derive(Debug, Clone)]
struct Member {
    key: KeyExpr<'static>,
    values: Bindings,
}

/// One served operation: what the runtime needs to answer its calls.
#[derive(Debug)]
struct OpSpec {
    addr: Addr,
    iface: IfaceId,
    /// `<kind token>/<template>`, the descriptor's naming.
    name: String,
    resource: Resource,
    op: Operation,
}

impl OpSpec {
    fn new(addr: &Addr, iface: &IfaceId, r: &Resource) -> Result<Self> {
        Ok(Self {
            addr: addr.clone(),
            iface: iface.clone(),
            name: resource_name(r),
            resource: r.clone(),
            op: operation(iface, r)?,
        })
    }

    /// The concrete key of this server's member with `values`.
    fn member_key(&self, values: &Bindings) -> Result<KeyExpr<'static>> {
        let chunks = self
            .resource
            .template
            .build(values)
            .map_err(|e| Error::Contract(format!("{} {:?}: {e}", self.iface, self.name)))?;
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let key = zenkey_model::grammar::data_key(&self.addr, &self.iface, KindToken::Op, &refs)?;
        Ok(KeyExpr::from(key.into_keyexpr()))
    }

    /// What a call's key expression binds of the template ([`bound_by`]).
    /// Its resource chunks follow `zk2/<system>/<service>/<iface>/@op`; a
    /// `**` among those five leaves them unplaced, and binds nothing.
    fn bound(&self, ke: &KeyExpr<'_>) -> std::result::Result<Bindings, String> {
        let chunks: Vec<&str> = ke.as_str().split('/').collect();
        match chunks.split_at_checked(5) {
            Some((head, tail)) if !head.contains(&"**") => bound_by(&self.resource.template, tail),
            _ => Ok(Bindings::new()),
        }
    }

    /// The member a fan-out names by its key alone: when the key binds every
    /// parameter, this server's member with those values, if the call
    /// selected it (§5.1, "Over a template").
    fn named_by_key(&self, bound: &Bindings, ke: &KeyExpr<'_>) -> Option<Member> {
        if !self
            .resource
            .template
            .params()
            .all(|(n, _)| bound.contains_key(n))
        {
            return None;
        }
        let key = self.member_key(bound).ok()?;
        ke.intersects(&key).then(|| Member {
            key,
            values: bound.clone(),
        })
    }

    /// Answers `q` with an envelope, from inside a queryable callback.
    fn refuse_now(&self, q: &Query, e: &OpError) {
        let (bytes, encoding) = encode_envelope(&self.op, e);
        if let Err(err) = q.reply_err(bytes).encoding(encoding).wait() {
            tracing::warn!(%err, key = %q.key_expr(), "an operation refusal was not sent");
        }
    }

    /// O2, then O3's `unavailable`: the refusal a call gets before any
    /// handler runs, if any.
    fn gate(&self, q: &Query, availability: &Availability) -> Option<OpError> {
        if q.key_expr().is_wild() && self.op.fanout == Fanout::Forbidden {
            return Some(OpError::fanout_forbidden(format!(
                "{} {:?} is fanout = \"forbidden\": call one concrete key (O2)",
                self.iface, self.name
            )));
        }
        availability
            .get(&self.iface, &self.name)
            .map(|(cause, why)| OpError::unavailable(cause, why))
    }
}

/// The optional operations this instance does not expose now, and why
/// (O3, §3.3). Shared by a service and its operation servers, and refreshed
/// whenever the capabilities or the `unavailable` list change.
#[derive(Debug, Default)]
pub(crate) struct Availability(RwLock<BTreeMap<(IfaceId, String), (Cause, String)>>);

impl Availability {
    /// Recomputes the map: an optional operation gated on a capability not
    /// held is absent for that cause; one listed unavailable, for its own.
    pub(crate) fn refresh(&self, impls: &[ImplState], held: &BTreeSet<String>) {
        let mut gone = BTreeMap::new();
        for s in impls {
            for r in &s.imp.contract().resources {
                if r.kind != Kind::Operation || !r.optional {
                    continue;
                }
                let name = resource_name(r);
                let why = match missing_capability(r, held) {
                    Some(cap) => {
                        Some((Cause::Capability, format!("capability {cap:?} is not held")))
                    }
                    None => s.unavailable.get(&name).map(|(cause, reason)| {
                        let why = reason.clone().unwrap_or_else(|| {
                            format!("not available here ({})", cause_name(*cause))
                        });
                        (*cause, why)
                    }),
                };
                if let Some(w) = why {
                    gone.insert((s.imp.iface().clone(), name), w);
                }
            }
        }
        *self.0.write().expect("not poisoned") = gone;
    }

    fn get(&self, iface: &IfaceId, name: &str) -> Option<(Cause, String)> {
        self.0
            .read()
            .expect("not poisoned")
            .get(&(iface.clone(), name.to_owned()))
            .cloned()
    }
}

/// A service's operation state: the shared [`Availability`], and the
/// queryables that answer `unavailable` for the optional operations it does
/// not serve (O3).
pub(crate) struct Ops {
    pub(crate) availability: Arc<Availability>,
    fallbacks: Vec<Queryable<()>>,
}

/// Whether an operation resource is `serving = "replicated"` (§6).
pub(crate) fn is_replicated(r: &Resource) -> bool {
    matches!(&r.body, Body::Operation(op) if op.serving == Serving::Replicated)
}

/// Whether every resource `s` exposes now is a replicated operation: a
/// replica, beside which another instance may serve the exclusive ones
/// (§5.1, "Beside replicas").
fn replica_only(s: &ImplState, held: &BTreeSet<String>) -> bool {
    s.imp
        .contract()
        .resources
        .iter()
        .filter(|r| s.exposed.contains(&resource_name(r)) && missing_capability(r, held).is_none())
        .all(is_replicated)
}

impl Ops {
    /// At start, after §8.2 step 2: for every interface this instance is
    /// active on (it exposes at least one resource of it), a `complete`
    /// queryable over each optional operation it does not expose, answering
    /// `unavailable` with the cause. A standby is active on nothing, so it
    /// declares none and intercepts no call (O3, §6).
    ///
    /// **Beside replicas** (§5.1, 0.7, O-10): an instance whose exposed
    /// resources of an interface are all replicated operations declares none
    /// on the interface's exclusive operations. The instance serving one may
    /// be another, and a concrete call reaches whichever `complete`
    /// queryable is nearest (O1), so an `unavailable` there would intercept
    /// it. It still answers `unavailable` on a replicated operation it does
    /// not expose, which replicas should all expose.
    pub(crate) async fn start(b: &ServiceBuilder) -> Result<Self> {
        let availability = Arc::clone(b.availability());
        let held = &b.config().capabilities;
        availability.refresh(b.impls(), held);
        let mut fallbacks = Vec::new();
        for s in b.impls().iter().filter(|s| exposes_any(s, held)) {
            let replica = replica_only(s, held);
            for r in &s.imp.contract().resources {
                let name = resource_name(r);
                if r.kind != Kind::Operation
                    || !r.optional
                    || s.exposed.contains(&name)
                    || (replica && !is_replicated(r))
                {
                    continue;
                }
                let spec = Arc::new(OpSpec::new(&b.config().address, s.imp.iface(), r)?);
                let avail = Arc::clone(&availability);
                let ke = pattern_of(&b.config().address, s.imp.iface(), r)?;
                let q = b
                    .session()
                    .declare_queryable(ke)
                    .complete(true)
                    .callback(move |q: Query| {
                        let e = spec.gate(&q, &avail).unwrap_or_else(|| {
                            // Neither gated nor listed, yet not served:
                            // start refuses that (§8.2 step 2), so only a
                            // later `set_unavailable(.., None)` gets here.
                            OpError::unavailable(Cause::Config, "not served by this instance")
                        });
                        spec.refuse_now(&q, &e);
                    })
                    .await
                    .map_err(zenoh)?;
                fallbacks.push(q);
            }
        }
        Ok(Self {
            availability,
            fallbacks,
        })
    }

    /// The `unavailable` queryables declared at start.
    pub(crate) fn fallbacks(&self) -> &[Queryable<()>] {
        &self.fallbacks
    }
}

/// An operation's server: its `complete` queryable (O1). Dropping it
/// undeclares it.
pub struct OperationServer {
    queryable: Queryable<()>,
    key_expr: OwnedKeyExpr,
}

impl OperationServer {
    /// The key expression served: a member's concrete key, or the template's
    /// pattern.
    #[must_use]
    pub fn key_expr(&self) -> &OwnedKeyExpr {
        &self.key_expr
    }

    /// The zenoh queryable.
    #[must_use]
    pub fn queryable(&self) -> &Queryable<()> {
        &self.queryable
    }

    pub async fn undeclare(self) -> Result<()> {
        self.queryable.undeclare().await.map_err(zenoh)
    }
}

/// What a call has sent so far: `one` allows one reply, `many` any number
/// of values, then at most one summary; a refusal ends either.
#[derive(Debug, Default)]
struct Sent {
    values: u32,
    summary: bool,
    refused: bool,
}

#[derive(Clone, Copy)]
enum Reply {
    Value,
    Summary,
    Refusal,
}

/// One call, as its handler receives it.
///
/// Clones share the call: what was sent, and the member it answers for. The
/// query completes (O6's completion) when the handler's future has resolved
/// and the last clone has dropped, so a handler replies before it returns.
///
/// **The member** (O3, §5.1 "Over a template", 0.7): a reply goes on the
/// key of the member the call answers for. It is known at once for a
/// concrete call, for a server declared on one member, and for a fan-out
/// whose key binds every parameter; otherwise the handler names it
/// ([`Call::member`]) from what the key binds ([`Call::bound`]). A call
/// answers for one member.
#[derive(Clone)]
pub struct Call {
    query: Query,
    spec: Arc<OpSpec>,
    bound: Bindings,
    member: Arc<OnceLock<Member>>,
    metadata: Option<CallMetadata>,
    sent: Arc<Mutex<Sent>>,
}

impl Call {
    fn new(query: Query, spec: Arc<OpSpec>, bound: Bindings, member: Option<Member>) -> Self {
        let metadata = query
            .attachment()
            .and_then(|a| CallMetadata::from_bytes(&a.to_bytes()));
        Self {
            query,
            spec,
            bound,
            member: Arc::new(member.map(OnceLock::from).unwrap_or_default()),
            metadata,
            sent: Arc::default(),
        }
    }

    /// The zenoh query.
    #[must_use]
    pub fn query(&self) -> &Query {
        &self.query
    }

    /// The key expression called.
    #[must_use]
    pub fn key_expr(&self) -> &KeyExpr<'static> {
        self.query.key_expr()
    }

    /// Whether the call named one concrete key. A fan-out call does not
    /// (only an operation with `fanout = "allowed"` sees one, O2).
    #[must_use]
    pub fn is_concrete(&self) -> bool {
        !self.query.key_expr().is_wild()
    }

    /// The template's values, unslugged, of the member this call answers
    /// for: the concrete key called, the member this server was declared
    /// on, or the member named by the key or the handler. `None` for a
    /// fan-out over a template until a member is named.
    #[must_use]
    pub fn values(&self) -> Option<&Bindings> {
        self.member.get().map(|m| &m.values)
    }

    /// What the key called binds (§5.1, "Over a template"): each parameter
    /// at a concrete chunk, unslugged, and none at a wildcard. Every
    /// parameter, for a concrete call.
    #[must_use]
    pub fn bound(&self) -> &Bindings {
        &self.bound
    }

    /// The call metadata the caller claims (O7), if its attachment is that
    /// object.
    #[must_use]
    pub fn metadata(&self) -> Option<&CallMetadata> {
        self.metadata.as_ref()
    }

    /// The request's encoded bytes, as sent.
    #[must_use]
    pub fn payload(&self) -> Option<&ZBytes> {
        self.query.payload()
    }

    /// The request, decoded as its JSON Schema type: by the request's
    /// `Encoding`, else the contract's `encoding` (§7.2). A request that
    /// does not decode is `invalid_request` (O3). The Rust type is the
    /// validator: the runtime does not evaluate the schema itself.
    pub fn request<T: DeserializeOwned>(&self) -> Result<T, OpError> {
        if !self.spec.op.request.is_json() {
            return Err(OpError::internal(
                "request() decodes JSON Schema types; this operation's request is not one",
            ));
        }
        let bytes = self
            .query
            .payload()
            .map(|p| p.to_bytes().into_owned())
            .unwrap_or_default();
        decode_value(&bytes, self.query.encoding(), self.spec.op.encoding)
            .map_err(|e| OpError::invalid_request(format!("the request does not decode: {e}")))
    }

    /// Names the member this call answers for (§5.1, "Over a template";
    /// O-1, C-2): its replies go on that member's key, which the call's key
    /// expression must select. Every clone sees it, so a typed handler
    /// names it through its [`crate::CallInfo`] and the runtime replies on
    /// it. Only a fan-out to a server declared over the whole template, on
    /// a key that leaves a parameter unbound, needs it.
    ///
    /// A call answers for one member: naming another once one is known is
    /// refused, and naming the same one again is not.
    pub fn member(&self, values: &Bindings) -> Result<()> {
        let key = self.spec.member_key(values)?;
        if !self.query.key_expr().intersects(&key) {
            return Err(Error::Contract(format!(
                "{key} is not a member this call selected"
            )));
        }
        let named = self.member.get_or_init(|| Member {
            key: key.clone(),
            values: values.clone(),
        });
        if named.key == key {
            Ok(())
        } else {
            Err(Error::Contract(format!(
                "this call answers for {} already: one member per call (§5.1)",
                named.key
            )))
        }
    }

    fn claim(&self, what: Reply) -> Result<()> {
        let mut s = self.sent.lock().expect("not poisoned");
        let many = self.spec.op.replies == Replies::Many;
        let refuse = |why: &str| Err(Error::Contract(format!("{}: {why}", self.spec.name)));
        if s.refused || s.summary {
            return refuse("the call has ended (a summary or a refusal was sent)");
        }
        match what {
            Reply::Value if !many && s.values > 0 => return refuse("replies = \"one\": answered"),
            Reply::Value => s.values += 1,
            Reply::Summary if !many || self.spec.op.summary.is_none() => {
                return refuse("this operation declares no summary (O6)");
            }
            Reply::Summary => s.summary = true,
            Reply::Refusal if !many && s.values > 0 => {
                return refuse("replies = \"one\": answered");
            }
            Reply::Refusal => s.refused = true,
        }
        Ok(())
    }

    fn reply_key(&self) -> Result<KeyExpr<'static>> {
        self.member.get().map(|m| m.key.clone()).ok_or_else(|| {
            Error::Contract(format!(
                "a fan-out call over {:?}'s template names no member: name one first \
                 (Call::member, CallInfo::member; §5.1)",
                self.spec.name
            ))
        })
    }

    fn encoding_of(&self, ty: &TypeId) -> Encoding {
        let none = Bindings::new();
        wire_encoding(ty, self.spec.op.encoding, self.values().unwrap_or(&none))
    }

    /// A value reply, already encoded as the `response` type, on the
    /// operation's concrete key with its `Encoding` (O3, §7.2).
    pub async fn reply(&self, payload: impl Into<ZBytes>) -> Result<()> {
        let key = self.reply_key()?;
        self.claim(Reply::Value)?;
        self.query
            .reply(key, payload)
            .encoding(self.encoding_of(&self.spec.op.response))
            .await
            .map_err(zenoh)
    }

    /// Encodes a value of a JSON Schema type `ty` as the contract says
    /// (§7.2), before any await: the value need not be `Sync`.
    fn encode_json<T: Serialize>(&self, ty: Option<&TypeId>, value: &T) -> Result<Vec<u8>> {
        if !ty.is_some_and(TypeId::is_json) {
            return Err(Error::Contract(format!(
                "{}: the value's type is not a JSON Schema type",
                self.spec.name
            )));
        }
        encode_value(value, self.spec.op.encoding)
    }

    /// A value reply of a JSON Schema `response` type, encoded as the
    /// contract says (§7.2).
    pub fn reply_value<T: Serialize>(
        &self,
        value: &T,
    ) -> impl Future<Output = Result<()>> + Send + '_ {
        let bytes = self.encode_json(Some(&self.spec.op.response), value);
        async move { self.reply(bytes?).await }
    }

    /// The summary reply (O6): the replier's last, its attachment the bytes
    /// `summary`. Only for `replies = "many"` with a declared `summary`.
    pub async fn summary(&self, payload: impl Into<ZBytes>) -> Result<()> {
        let key = self.reply_key()?;
        self.claim(Reply::Summary)?;
        let ty = self.spec.op.summary.as_ref().expect("claimed");
        self.query
            .reply(key, payload)
            .encoding(self.encoding_of(ty))
            .attachment(SUMMARY)
            .await
            .map_err(zenoh)
    }

    /// The summary, of a JSON Schema `summary` type.
    pub fn summary_value<T: Serialize>(
        &self,
        value: &T,
    ) -> impl Future<Output = Result<()>> + Send + '_ {
        let bytes = self.encode_json(self.spec.op.summary.as_ref(), value);
        async move { self.summary(bytes?).await }
    }

    /// Refuses the call with the error envelope (O3, §5.2), in the
    /// encoding the operation's types give it.
    pub async fn refuse(&self, error: OpError) -> Result<()> {
        self.claim(Reply::Refusal)?;
        let (bytes, encoding) = encode_envelope(&self.spec.op, &error);
        self.query
            .reply_err(bytes)
            .encoding(encoding)
            .await
            .map_err(zenoh)
    }

    /// After the handler: its error is refused for it, and a call it left
    /// without a reply (or, with a declared summary, without the summary)
    /// gets `internal`. A caller never meets silence from a live server's
    /// own bug.
    async fn finish(&self, outcome: Result<(), OpError>) {
        let pending = {
            let s = self.sent.lock().expect("not poisoned");
            let ended = s.refused || s.summary;
            match self.spec.op.replies {
                Replies::One => !ended && s.values == 0,
                Replies::Many => !ended && self.spec.op.summary.is_some(),
            }
        };
        let error = match outcome {
            Err(e) => Some(e),
            Ok(()) if pending => Some(OpError::internal(match self.spec.op.replies {
                Replies::One => "the handler returned without replying",
                Replies::Many => "the handler returned without its summary (O6)",
            })),
            Ok(()) => None,
        };
        if let Some(e) = error
            && let Err(err) = self.refuse(e.clone()).await
        {
            tracing::warn!(%err, error = %e, "an operation's error was not sent");
        }
    }
}

/// Declares an operation server on `ke`.
async fn declare<H, Fut>(
    session: &zenoh::Session,
    spec: OpSpec,
    ke: OwnedKeyExpr,
    availability: Arc<Availability>,
    handler: H,
) -> Result<OperationServer>
where
    H: Fn(Call) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), OpError>> + Send + 'static,
{
    let rt = tokio::runtime::Handle::try_current()
        .map_err(|e| Error::Contract(format!("serving needs a tokio runtime: {e}")))?;
    let spec = Arc::new(spec);
    let handler = Arc::new(handler);
    let declared: Option<KeyExpr<'static>> = (!ke.is_wild()).then(|| KeyExpr::from(ke.clone()));
    let queryable = session
        .declare_queryable(ke.clone())
        .complete(true)
        .callback(move |q: Query| {
            if let Some(e) = spec.gate(&q, &availability) {
                spec.refuse_now(&q, &e);
                return;
            }
            let not_a_member = |why: &str| {
                OpError::invalid_request(format!(
                    "{} names no member of {:?}: {why} (§5.1)",
                    q.key_expr(),
                    spec.name
                ))
            };
            let (bound, member) = if q.key_expr().is_wild() {
                // A fan-out (O2): what the key binds, and the member it
                // answers for, when it is known before the handler runs.
                let bound = match spec.bound(q.key_expr()) {
                    Ok(b) => b,
                    Err(why) => {
                        spec.refuse_now(&q, &not_a_member(&why));
                        return;
                    }
                };
                let member = match &declared {
                    Some(k) => {
                        values_of(&spec.iface, &spec.resource, k.as_str()).map(|(_, v)| Member {
                            key: k.clone(),
                            values: v,
                        })
                    }
                    None => spec.named_by_key(&bound, q.key_expr()),
                };
                (bound, member)
            } else {
                match values_of(&spec.iface, &spec.resource, q.key_expr().as_str()) {
                    Some((_, v)) => (
                        v.clone(),
                        Some(Member {
                            key: q.key_expr().clone(),
                            values: v,
                        }),
                    ),
                    None => {
                        spec.refuse_now(&q, &not_a_member("a chunk is not a canonical slug"));
                        return;
                    }
                }
            };
            let call = Call::new(q, Arc::clone(&spec), bound, member);
            let h = Arc::clone(&handler);
            rt.spawn(async move {
                let outcome = h(call.clone()).await;
                call.finish(outcome).await;
            });
        })
        .await
        .map_err(zenoh)?;
    Ok(OperationServer {
        queryable,
        key_expr: ke,
    })
}

/// Wraps a typed one-reply handler: decode, call, encode.
fn typed<Req, Resp, H, Fut>(
    handler: H,
) -> impl Fn(Call) -> std::pin::Pin<Box<dyn Future<Output = Result<(), OpError>> + Send>>
+ Send
+ Sync
+ 'static
where
    Req: DeserializeOwned + Send + 'static,
    Resp: Serialize + Send + 'static,
    H: Fn(Call, Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Resp, OpError>> + Send + 'static,
{
    let handler = Arc::new(handler);
    move |call: Call| {
        let h = Arc::clone(&handler);
        Box::pin(async move {
            let req = call.request::<Req>()?;
            let resp = h(call.clone(), req).await?;
            call.reply_value(&resp).await?;
            Ok(())
        })
    }
}

/// `serve_value` serves one-reply operations of JSON Schema types only.
fn check_typed(iface: &IfaceId, r: &Resource) -> Result<()> {
    let op = operation(iface, r)?;
    if op.replies != Replies::One || !op.request.is_json() || !op.response.is_json() {
        return Err(Error::Contract(format!(
            "{iface} {:?}: serve_value is for replies = \"one\" with JSON Schema request and \
             response types; use serve",
            resource_name(r)
        )));
    }
    Ok(())
}

impl ServiceBuilder {
    /// Serves an operation (O1): a `complete` queryable on a member's
    /// concrete key, or on every member when `values` is `None`, and exposes
    /// the resource. Each call that passes O2 and O3's checks runs `handler`
    /// on the current tokio runtime; see [`Call`] for replying.
    ///
    /// At most once per call holds only while one instance serves:
    /// exclusivity across instances is `redundancy.v1`'s (#613).
    pub async fn serve<H, Fut>(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: Option<&Bindings>,
        handler: H,
    ) -> Result<OperationServer>
    where
        H: Fn(Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), OpError>> + Send + 'static,
    {
        let r = self.resource_of(iface, resource)?;
        let spec = OpSpec::new(&self.config().address, iface, &r)?;
        let ke = match values {
            Some(v) => self.key(iface, resource, v)?.into_keyexpr(),
            None => pattern_of(&self.config().address, iface, &r)?,
        };
        self.expose(iface, resource)?;
        let availability = Arc::clone(self.availability());
        declare(self.session(), spec, ke, availability, handler).await
    }

    /// Serves a one-reply operation of JSON Schema types: the request is
    /// decoded (a failure is `invalid_request`), `handler` answers, and its
    /// value is the reply. An `Err` is refused with its envelope.
    pub async fn serve_value<Req, Resp, H, Fut>(
        &mut self,
        iface: &IfaceId,
        resource: &str,
        values: Option<&Bindings>,
        handler: H,
    ) -> Result<OperationServer>
    where
        Req: DeserializeOwned + Send + 'static,
        Resp: Serialize + Send + 'static,
        H: Fn(Call, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, OpError>> + Send + 'static,
    {
        check_typed(iface, &self.resource_of(iface, resource)?)?;
        self.serve(iface, resource, values, typed(handler)).await
    }

    fn resource_of(&self, iface: &IfaceId, resource: &str) -> Result<Resource> {
        self.impls()
            .iter()
            .find(|s| s.imp.iface() == iface)
            .ok_or_else(|| Error::NotImplemented(iface.clone()))?
            .imp
            .resource(resource)
            .cloned()
    }
}

impl Service {
    /// Serves an operation exposed before start (§8.2): for a template whose
    /// members appear while the service runs. See [`ServiceBuilder::serve`].
    pub async fn serve<H, Fut>(
        &self,
        iface: &IfaceId,
        resource: &str,
        values: Option<&Bindings>,
        handler: H,
    ) -> Result<OperationServer>
    where
        H: Fn(Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), OpError>> + Send + 'static,
    {
        let r = self.exposed_op(iface, resource)?;
        let spec = OpSpec::new(self.address(), iface, &r)?;
        let ke = match values {
            Some(v) => self.key(iface, resource, v)?.into_keyexpr(),
            None => pattern_of(self.address(), iface, &r)?,
        };
        let availability = Arc::clone(&self.ops().availability);
        declare(self.session(), spec, ke, availability, handler).await
    }

    /// [`ServiceBuilder::serve_value`], after start.
    pub async fn serve_value<Req, Resp, H, Fut>(
        &self,
        iface: &IfaceId,
        resource: &str,
        values: Option<&Bindings>,
        handler: H,
    ) -> Result<OperationServer>
    where
        Req: DeserializeOwned + Send + 'static,
        Resp: Serialize + Send + 'static,
        H: Fn(Call, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Resp, OpError>> + Send + 'static,
    {
        check_typed(iface, &self.exposed_op(iface, resource)?)?;
        self.serve(iface, resource, values, typed(handler)).await
    }

    /// The client of `role`, compiled against `contract`, the required
    /// interface's (R4). Its providers and parameter bindings are the
    /// configuration's (R1, R2), as a consumer's are.
    pub fn client(&self, role: &str, contract: Arc<Contract>) -> Result<Client> {
        let r = self
            .roles()
            .iter()
            .find(|r| r.role == role)
            .ok_or_else(|| Error::Contract(format!("this service has no role {role:?}")))?;
        if r.interface != contract.iface {
            return Err(Error::Contract(format!(
                "role {role:?} requires {}, not {}",
                r.interface, contract.iface
            )));
        }
        let b = self
            .config()
            .bindings
            .get(role)
            .cloned()
            .unwrap_or_default();
        Client::for_role(
            self.session(),
            self.address(),
            role,
            contract,
            &b.providers,
            &b.params,
        )
    }

    /// The `unavailable` queryables this instance declared at start (O3):
    /// one per optional operation of an active interface that it does not
    /// serve, an exclusive one excepted where it is a replica (§5.1,
    /// "Beside replicas").
    #[must_use]
    pub fn unavailable_queryables(&self) -> &[Queryable<()>] {
        self.ops().fallbacks()
    }

    fn exposed_op(&self, iface: &IfaceId, resource: &str) -> Result<Resource> {
        let s = self
            .impls()
            .iter()
            .find(|s| s.imp.iface() == iface)
            .ok_or_else(|| Error::NotImplemented(iface.clone()))?;
        let r = s.imp.resource(resource)?;
        if !s.exposed.contains(&resource_name(r)) {
            return Err(Error::Contract(format!(
                "{iface} {resource:?} was not exposed before start (§8.2)"
            )));
        }
        Ok(r.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{OpError, bound_by, envelope_encoding};
    use crate::codec::Raw;
    use serde_json::json;
    use zenkey_model::envelope::{self, Detail, Envelope};
    use zenkey_model::template::{Bindings, Template};

    /// An operation `op` of raw request and response, with `error` if given.
    fn op(error: Option<&str>) -> zenkey_model::contract::Operation {
        let toml = format!(
            "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n\
             [schemas]\njsonschema = [\"t.json\"]\n\
             [resources.op]\nkind = \"operation\"\n\
             request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n{}",
            error.map(|e| format!("error = {e}\n")).unwrap_or_default()
        );
        let dir = std::env::temp_dir().join(format!("zk2-op-envelope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t.json"),
            r#"{"$defs": {"E": {"type": "object", "properties": {"k": {"type": "string"}}}}}"#,
        )
        .unwrap();
        let l = zenkey_model::contract::load_str(&toml, &dir, None);
        let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
        super::operation(&c.iface, &c.resources[0]).unwrap()
    }

    /// What goes on the wire for `e`, decoded.
    fn sent(op: &zenkey_model::contract::Operation, e: &OpError) -> Envelope {
        let (bytes, enc) = super::encode_envelope(op, e);
        envelope::decode(&enc.to_string(), &bytes).unwrap()
    }

    /// Spec §5.2 (0.7, F-65): `app` for any operation; no detail without an
    /// `error` type; a raw type's detail as base64 text in JSON; a detail
    /// that does not fit goes out as `internal`, never malformed.
    #[test]
    fn an_app_detail_that_does_not_fit_goes_out_as_internal() {
        let none = op(None);
        assert_eq!(envelope_encoding(&none), envelope::JSON);
        let bare = sent(&none, &OpError::app_without_detail("x"));
        assert_eq!((bare.code.as_str(), bare.detail), ("app", None));
        for detail in [
            OpError::app_bytes("x", vec![1]),
            OpError::app("x", &json!({})),
        ] {
            assert_eq!(
                sent(&none, &detail).code,
                "internal",
                "no error type, no detail"
            );
        }

        let raw = op(Some("{ raw = \"image/jpeg\" }"));
        assert_eq!(envelope_encoding(&raw), envelope::JSON);
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0];
        for e in [
            OpError::app_bytes("camera fault", jpeg.clone()),
            OpError::app_as::<Raw>("camera fault", &jpeg),
        ] {
            let got = sent(&raw, &e);
            assert_eq!(got.code, "app");
            assert_eq!(got.detail, Some(Detail::Value(json!("/9j/4A=="))));
            assert_eq!(got.detail.unwrap().raw_bytes(), Some(jpeg.clone()));
        }
        assert_eq!(
            sent(&raw, &OpError::app("x", &json!("not base64!"))).code,
            "internal"
        );
        assert_eq!(sent(&raw, &OpError::app_without_detail("x")).detail, None);

        let json_error = op(Some("\"json:E\""));
        assert_eq!(
            sent(&json_error, &OpError::app_bytes("x", vec![1])).code,
            "internal"
        );
        let ok = sent(&json_error, &OpError::app("x", &json!({"k": "v"})));
        assert_eq!(ok.detail, Some(Detail::Value(json!({"k": "v"}))));

        // A relayed envelope a decoder would refuse is not sent as given.
        let relayed = OpError::from(Envelope {
            code: "busy".into(),
            message: "x".into(),
            cause: None,
            detail: Some(Detail::Value(json!(1))),
        });
        assert_eq!(sent(&json_error, &relayed).code, "internal");
    }

    /// Spec §5.1 "Over a template" (0.7): a key binds each parameter at a
    /// concrete chunk, unslugged, and none at a wildcard.
    #[test]
    fn what_a_key_binds_of_a_template() {
        let t = Template::parse("ns/{ns}/interfaces/{if}/reset").unwrap();
        let b = |pairs: &[(&str, &str)]| -> Bindings {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), vec![(*v).to_owned()]))
                .collect()
        };
        let bound = |k: &str| bound_by(&t, &k.split('/').collect::<Vec<_>>());
        assert_eq!(bound("ns/lab/interfaces/*/reset"), Ok(b(&[("ns", "lab")])));
        assert_eq!(
            bound("ns/lab/interfaces/x-foo_x401/reset"),
            Ok(b(&[("ns", "lab"), ("if", "foo@1")]))
        );
        assert_eq!(bound("ns/*/interfaces/eth$*/reset"), Ok(Bindings::new()));
        assert_eq!(bound("ns/lab/**"), Ok(b(&[("ns", "lab")])));
        assert!(bound("ns/LAB/interfaces/*/reset").is_err(), "not a slug");
        let rest = Template::parse("files/{path...}").unwrap();
        let bound = |k: &str| bound_by(&rest, &k.split('/').collect::<Vec<_>>());
        assert_eq!(
            bound("files/a/b"),
            Ok([("path".to_owned(), vec!["a".to_owned(), "b".to_owned()])].into())
        );
        assert_eq!(bound("files/a/*"), Ok(Bindings::new()));
    }
}
