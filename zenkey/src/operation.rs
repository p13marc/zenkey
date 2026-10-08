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
//!   failure is a `reply_err` carrying the envelope, encoded as §5.2 says.
//!   The active instance answers `unavailable`, with its cause, for every
//!   optional operation it does not expose; a standby declares nothing.
//! - **O7:** the call metadata (`actor`, `request_id`) is read from the
//!   request's attachment. It is claimed, never authenticated.
//!
//! Calling is [`crate::client`]'s.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, RwLock};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use zenkey_model::authoring::{Encoding as WireEncoding, Kind};
use zenkey_model::contract::{Body, Contract, Fanout, Operation, Replies, Resource};
use zenkey_model::descriptor::Cause;
use zenkey_model::envelope::{self, Detail, Envelope};
use zenkey_model::grammar::{Addr, IfaceId, KindToken, ZkKey, parse};
use zenkey_model::schema::TypeId;
use zenkey_model::template::{Bindings, Segment};
use zenoh::Wait;
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::{KeyExpr, OwnedKeyExpr};
use zenoh::query::{Query, Queryable};

use crate::client::Client;
use crate::error::{Error, Result, zenoh};
use crate::implementation::{missing_capability, resource_name};
use crate::service::{ImplState, Service, ServiceBuilder, exposes_any};
use crate::writer::{encode_value, wire_encoding};

/// The attachment that marks a summary reply (O6).
pub const SUMMARY: &[u8] = b"summary";

/// The schema suffix of a protobuf envelope (§5.2).
const PROTOBUF_ERROR: &str = "zk2.core.v1.Error";

/// The optional call metadata (O7): who claims to call, and the caller's
/// id for the request. It travels as the request's attachment, the JSON
/// object `{"actor", "request_id"}`. Claimed, never authentication: any
/// session can write any value here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl CallMetadata {
    #[must_use]
    pub fn new(actor: &str, request_id: &str) -> Self {
        Self {
            actor: Some(actor.to_owned()),
            request_id: Some(request_id.to_owned()),
        }
    }

    /// The attachment's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("two optional strings serialize")
    }

    /// Reads an attachment as call metadata: a JSON object whose `actor`
    /// and `request_id`, when present, are strings. Other members are
    /// ignored. Anything else is not call metadata, and `None`.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let serde_json::Value::Object(m) = serde_json::from_slice(bytes).ok()? else {
            return None;
        };
        let text = |k: &str| match m.get(k) {
            None => Some(None),
            Some(serde_json::Value::String(s)) => Some(Some(s.clone())),
            Some(_) => None,
        };
        Some(Self {
            actor: text("actor")?,
            request_id: text("request_id")?,
        })
    }
}

/// A failed call, as its owner refuses it (O3): the error envelope of §5.2.
/// The runtime encodes it as the operation's types say.
#[derive(Debug, Clone, PartialEq)]
pub struct OpError(Envelope);

/// The name §5.2 gives a cause.
#[must_use]
pub fn cause_name(c: Cause) -> &'static str {
    match c {
        Cause::Build => "build",
        Cause::Config => "config",
        Cause::Capability => "capability",
    }
}

impl OpError {
    fn new(code: &str, message: impl Into<String>, cause: Option<Cause>) -> Self {
        Self(Envelope {
            code: code.to_owned(),
            message: message.into(),
            cause: cause.map(|c| cause_name(c).to_owned()),
            detail: None,
        })
    }

    /// The request does not decode, or breaks the operation's rules.
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message, None)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message, None)
    }

    /// The operation is not available here, and why (§2.3, §3.3).
    pub fn unavailable(cause: Cause, message: impl Into<String>) -> Self {
        Self::new("unavailable", message, Some(cause))
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new("forbidden", message, None)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new("busy", message, None)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message, None)
    }

    /// The runtime's alone (O2): a handler never sees a fan-out call to an
    /// operation that forbids one.
    pub(crate) fn fanout_forbidden(message: impl Into<String>) -> Self {
        Self::new("fanout_forbidden", message, None)
    }

    /// `app`, with a value of the operation's declared `error` type: for a
    /// JSON or CBOR envelope (§5.2).
    pub fn app<T: Serialize>(message: impl Into<String>, detail: &T) -> Self {
        match serde_json::to_value(detail) {
            Ok(v) => Self(Envelope {
                code: "app".to_owned(),
                message: message.into(),
                cause: None,
                detail: Some(Detail::Value(v)),
            }),
            Err(e) => Self::internal(format!("the app detail does not serialize: {e}")),
        }
    }

    /// `app`, with the declared protobuf `error` message, already encoded:
    /// for a protobuf envelope (§5.2).
    pub fn app_bytes(message: impl Into<String>, detail: Vec<u8>) -> Self {
        Self(Envelope {
            code: "app".to_owned(),
            message: message.into(),
            cause: None,
            detail: Some(Detail::Bytes(detail)),
        })
    }

    #[must_use]
    pub fn envelope(&self) -> &Envelope {
        &self.0
    }

    #[must_use]
    pub fn into_envelope(self) -> Envelope {
        self.0
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.0.code, self.0.message)
    }
}

impl std::error::Error for OpError {}

/// A runtime error inside a handler is the owner's failure: `internal`.
impl From<Error> for OpError {
    fn from(e: Error) -> Self {
        Self::internal(e.to_string())
    }
}

/// An envelope received from elsewhere (a proxy relaying a refusal).
impl From<Envelope> for OpError {
    fn from(e: Envelope) -> Self {
        Self(e)
    }
}

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

/// The envelope's bytes and `Encoding` for `op`. A detail of the wrong form
/// for the encoding (a value in a protobuf envelope, bytes in a JSON one) is
/// the handler's bug, and goes out as `internal`.
pub(crate) fn encode_envelope(op: &Operation, e: &OpError) -> (Vec<u8>, Encoding) {
    let enc = envelope_encoding(op);
    let bytes = envelope::encode(e.envelope(), enc).unwrap_or_else(|| {
        tracing::warn!(
            code = e.envelope().code,
            encoding = enc,
            "an app detail does not fit the operation's envelope encoding (spec §5.2)"
        );
        let fallback = OpError::internal("the app detail does not fit the envelope's encoding");
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
    let cbor = match sample.map(ToString::to_string) {
        Some(e) if e.starts_with("application/cbor") => true,
        Some(e) if e.starts_with("application/json") => false,
        _ => wire == Some(WireEncoding::Cbor),
    };
    if cbor {
        ciborium::from_reader(bytes).map_err(|e| format!("CBOR: {e}"))
    } else {
        serde_json::from_slice(bytes).map_err(|e| format!("JSON: {e}"))
    }
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
#[derive(Default)]
pub(crate) struct Ops {
    pub(crate) availability: Arc<Availability>,
    fallbacks: Vec<Queryable<()>>,
}

impl Ops {
    /// At start, after §8.2 step 2: for every interface this instance is
    /// active on (it exposes at least one resource of it), a `complete`
    /// queryable over each optional operation it does not expose, answering
    /// `unavailable` with the cause. A standby is active on nothing, so it
    /// declares none and intercepts no call (O3, §6).
    pub(crate) async fn start(b: &ServiceBuilder) -> Result<Self> {
        let availability = Arc::clone(b.availability());
        let held = &b.config().capabilities;
        availability.refresh(b.impls(), held);
        let mut fallbacks = Vec::new();
        for s in b.impls().iter().filter(|s| exposes_any(s, held)) {
            for r in &s.imp.contract().resources {
                let name = resource_name(r);
                if r.kind != Kind::Operation || !r.optional || s.exposed.contains(&name) {
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
                            // Neither gated nor listed, yet not served: a
                            // configuration the descriptor check would
                            // already have refused at start.
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
/// Clones share the call. The query completes (O6's completion) when the
/// handler's future has resolved and the last clone has dropped, so a
/// handler replies before it returns.
#[derive(Clone)]
pub struct Call {
    query: Query,
    spec: Arc<OpSpec>,
    reply_key: Option<KeyExpr<'static>>,
    values: Option<Bindings>,
    metadata: Option<CallMetadata>,
    sent: Arc<Mutex<Sent>>,
}

impl Call {
    fn new(
        query: Query,
        spec: Arc<OpSpec>,
        reply_key: Option<KeyExpr<'static>>,
        values: Option<Bindings>,
    ) -> Self {
        let metadata = query
            .attachment()
            .and_then(|a| CallMetadata::from_bytes(&a.to_bytes()));
        Self {
            query,
            spec,
            reply_key,
            values,
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

    /// The template's values, unslugged: those of the concrete key called,
    /// or of the member this server was declared on.
    #[must_use]
    pub fn values(&self) -> Option<&Bindings> {
        self.values.as_ref()
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

    /// Names the member a fan-out call over a template answers for: its
    /// replies go on that member's concrete key (O3). Only a fan-out call
    /// to a server declared over the whole template needs it.
    pub fn member(&mut self, values: &Bindings) -> Result<()> {
        let chunks = self.spec.resource.template.build(values).map_err(|e| {
            Error::Contract(format!("{} {:?}: {e}", self.spec.iface, self.spec.name))
        })?;
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        let key = zenkey_model::grammar::data_key(
            &self.spec.addr,
            &self.spec.iface,
            KindToken::Op,
            &refs,
        )?;
        let ke = KeyExpr::from(key.into_keyexpr());
        if !self.query.key_expr().intersects(&ke) {
            return Err(Error::Contract(format!(
                "{ke} is not a member this call selected"
            )));
        }
        self.reply_key = Some(ke);
        self.values = Some(values.clone());
        Ok(())
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
        self.reply_key.clone().ok_or_else(|| {
            Error::Contract(format!(
                "a fan-out call over {:?}'s template: name the member first (Call::member)",
                self.spec.name
            ))
        })
    }

    fn encoding_of(&self, ty: &TypeId) -> Encoding {
        let none = Bindings::new();
        wire_encoding(
            ty,
            self.spec.op.encoding,
            self.values.as_ref().unwrap_or(&none),
        )
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
            let (reply_key, values) = if q.key_expr().is_wild() {
                let values = declared
                    .as_ref()
                    .and_then(|k| values_of(&spec.iface, &spec.resource, k.as_str()))
                    .map(|(_, v)| v);
                (declared.clone(), values)
            } else {
                match values_of(&spec.iface, &spec.resource, q.key_expr().as_str()) {
                    Some((_, v)) => (Some(q.key_expr().clone()), Some(v)),
                    None => {
                        let e = OpError::invalid_request(format!(
                            "{} is not a member key of {:?}: a chunk is not a canonical slug",
                            q.key_expr(),
                            spec.name
                        ));
                        spec.refuse_now(&q, &e);
                        return;
                    }
                }
            };
            let call = Call::new(q, Arc::clone(&spec), reply_key, values);
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
    /// serve.
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
    use super::{CallMetadata, OpError, envelope_encoding};
    use zenkey_model::envelope;

    #[test]
    fn call_metadata_is_two_optional_strings() {
        let m = CallMetadata::new("op@ws-01", "r-1");
        assert_eq!(CallMetadata::from_bytes(&m.to_bytes()), Some(m));
        assert_eq!(
            CallMetadata::from_bytes(br#"{"actor": "a", "other": 1}"#),
            Some(CallMetadata {
                actor: Some("a".into()),
                request_id: None
            })
        );
        for bad in [&b"[]"[..], br#"{"actor": 3}"#, b"not json"] {
            assert_eq!(CallMetadata::from_bytes(bad), None);
        }
    }

    #[test]
    fn an_app_detail_that_does_not_fit_goes_out_as_internal() {
        let op = |error: Option<&str>| {
            let toml = format!(
                "[interface]\nname = \"t\"\nmajor = 1\nminor = 0\n\
                 [resources.op]\nkind = \"operation\"\n\
                 request = {{ raw = \"text/plain\" }}\nresponse = {{ raw = \"text/plain\" }}\n{}",
                error.map(|e| format!("error = {e}\n")).unwrap_or_default()
            );
            let l = zenkey_model::contract::load_str(&toml, std::path::Path::new("."), None);
            let c = l.contract.unwrap_or_else(|| panic!("{}", l.report));
            super::operation(&c.iface, &c.resources[0]).unwrap()
        };
        let raw = op(None);
        assert_eq!(envelope_encoding(&raw), envelope::JSON);
        let (bytes, enc) = super::encode_envelope(&raw, &OpError::app_bytes("x", vec![1]));
        assert_eq!(enc.to_string(), envelope::JSON);
        assert_eq!(
            envelope::decode(envelope::JSON, &bytes).unwrap().code,
            "internal"
        );
    }
}
