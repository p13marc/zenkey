//! Calling operations (spec §5.1): a [`Client`] calls one concrete key at an
//! explicit address, a [`Fleet`] fans out to a selection spelled by name.
//!
//! - **Concrete calls (O1):** target `BestMatching`, consolidation `None`,
//!   the operation's recommended `priority` (§2.4: replies inherit the
//!   query's QoS). A one-reply call returns on the first value or owner's
//!   error, without waiting for the query to complete.
//! - **Fan-out calls (O2):** only to an operation declaring
//!   `fanout = "allowed"`, with target `All` and consolidation `None`; every
//!   reply is collected and attributed to its replier by its key (O3).
//! - **The request form** (core §5.1, #985's C4c): every call writes its
//!   request as `CH · record` under `zenoh/bytes`, CH's width bit 0 and its
//!   tag the `request` type's; the record is what the caller hands in,
//!   already encoded as that type.
//! - **O3, the forms** (#985's C4b to C4d): every reply is read by its
//!   `Encoding` alone, without the contract. Under `zenoh/bytes` a value
//!   is §7.5's value form ([`Answer::form`], its as-of mark
//!   [`Answer::mark`]) and an error its error form. A value the form's
//!   parse refuses, or under any other `Encoding`, is malformed and an
//!   answer all the same; an error reply so is the transport's, and
//!   silence (§5.1, §5.2): [`classify`]. Core 0.99 removed the bare wire,
//!   so an earlier core's envelope reads as the transport's.
//! - **O4:** only an `idempotent` operation is retried, and only after
//!   silence. A refusal is an answer.
//! - **O5:** silence is never a verdict. [`Outcome::NoAnswer`] carries what
//!   presence says of the service, never "no such operation".
//! - **O6:** a many-reply call keeps every value, and each replier's summary
//!   (the reply whose attachment is `summary`).
//! - **O7:** a client MAY send call metadata, claimed only.
//!
//! A client from a role takes its providers and parameter bindings from the
//! configuration (R1, R2), like a [`crate::consumer::Consumer`]; a tool's
//! client names its own.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use zenkey_model::authoring::Encoding as WireEncoding;
use zenkey_model::contract::{Body, Contract, Fanout, Operation, Replies as RepliesKind, Resource};
use zenkey_model::envelope::{self, ErrorReading, FormRefusal};
use zenkey_model::forms::Forms;
use zenkey_model::grammar::{Addr, IfaceId, Key, KindToken, ZkKey, data_key};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::{Bindings, Segment};
use zenkey_model::wire::{self, Stamp, Tag};
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply, ReplyError};
use zenoh::sample::Sample;

use crate::call::Replier as ReplierOf;
pub use crate::call::{Attribution, ErrorMark, Refusal, Silence, Transport};
use crate::consumer::{Presence, Provider};
use crate::error::{Error, Result, zenoh};
use crate::operation::{CallMetadata, SUMMARY, decode_value, find_resource, operation, values_of};
use crate::writer::encode_value;

/// The reply timeout when neither the client nor the contract's
/// `timeout_ms` sets one: zenoh's own default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// How a value reply came, read by its `Encoding` alone (core §5.1, §7.5;
/// observer §7.3, "A call's forms first").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyForm {
    /// §7.5's value form: CH's tag (the `response` type's, or the `summary`
    /// type's on a summary reply) and the replying instance's as-of mark,
    /// its record after them.
    Value { tag: Tag, stamp: Stamp },
    /// Under `zenoh/bytes`, bytes the form's parse refuses: malformed,
    /// undecodable, never read as a record with no form, and an answer all
    /// the same, since it came on the call's key (§5.1, "Retries").
    Refused(wire::Refusal),
    /// Under an `Encoding` other than `zenoh/bytes`, which carries no form
    /// (§5.1, "Reading a value"): malformed and undecodable as a refused
    /// one is, whatever its bytes, and an answer all the same.
    NoForm,
}

/// A value reply, attributed to the service whose key carried it (O3).
#[derive(Debug, Clone)]
pub struct Answer {
    pub replier: Addr,
    pub sample: Sample,
    form: ReplyForm,
    /// The operation's `encoding`, the wire a form's JSON Schema record is
    /// read on (§7.2: `zenoh/bytes` picks none).
    wire: Option<WireEncoding>,
}

impl Answer {
    /// A reply to a call of an operation whose `encoding` is `contract`,
    /// read by its `Encoding` ([`ReplyForm`]).
    #[must_use]
    pub fn new(replier: Addr, sample: Sample, contract: Option<WireEncoding>) -> Self {
        let form =
            match wire::read_value(&sample.encoding().to_string(), &sample.payload().to_bytes()) {
                Ok(e) => ReplyForm::Value {
                    tag: e.tag,
                    stamp: e.stamp,
                },
                Err(FormRefusal::Form(r)) => ReplyForm::Refused(r),
                Err(FormRefusal::Encoding(_)) => ReplyForm::NoForm,
            };
        Self {
            replier,
            sample,
            form,
            wire: contract,
        }
    }

    /// The concrete key the reply went on.
    #[must_use]
    pub fn key(&self) -> &str {
        self.sample.key_expr().as_str()
    }

    /// The reply's bytes as they arrived, a value form's CH and mark
    /// included: [`Answer::record`] is what its type reads.
    #[must_use]
    pub fn payload(&self) -> &ZBytes {
        self.sample.payload()
    }

    /// How the reply came: in the value form, in bytes the form's parse
    /// refuses, or under another `Encoding`.
    #[must_use]
    pub fn form(&self) -> ReplyForm {
        self.form
    }

    /// The replying instance's as-of mark (core §5.1), when the reply came
    /// in the value form: its run names an instance only through the
    /// descriptor that states it.
    #[must_use]
    pub fn mark(&self) -> Option<Stamp> {
        match self.form {
            ReplyForm::Value { stamp, .. } => Some(stamp),
            _ => None,
        }
    }

    /// CH's tag, when the reply came in the value form: a packed type's
    /// record is read by it (core §7.6).
    #[must_use]
    pub fn tag(&self) -> Option<Tag> {
        match self.form {
            ReplyForm::Value { tag, .. } => Some(tag),
            _ => None,
        }
    }

    /// The record its type reads: a value form's, after CH and the mark,
    /// read on the contract's `encoding` (observer §7.3: `zenoh/bytes`
    /// picks no wire). A malformed reply has none: why is the refusal, its
    /// parse's or its `Encoding`'s.
    pub fn record(&self) -> std::result::Result<Vec<u8>, FormRefusal> {
        let bytes = self.sample.payload().to_bytes();
        match self.form {
            ReplyForm::Value { stamp, .. } => Ok(bytes[stamp.envelope_bytes()..].to_vec()),
            ReplyForm::Refused(r) => Err(FormRefusal::Form(r)),
            ReplyForm::NoForm => Err(FormRefusal::Encoding(self.sample.encoding().to_string())),
        }
    }

    /// The record, decoded as a JSON Schema value on the operation's
    /// `encoding` (§7.2). A malformed reply does not decode.
    pub fn value<T: DeserializeOwned>(&self) -> Result<T> {
        let record = self
            .record()
            .map_err(|r| Error::Contract(format!("the reply is no value form: {}", r.reason())))?;
        decode_value(&record, self.wire).map_err(Error::Contract)
    }
}

/// What an error reply is (§5.2), by its `Encoding` alone: an owner's
/// error in its form, or the transport's own error.
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorReply {
    /// The error form under `zenoh/bytes`.
    Refused(Refusal),
    /// A querier's or a router's own `Timeout`, a `zenoh/bytes` error the
    /// form's parse refuses, or one under an `Encoding` that carries none,
    /// an earlier core's envelope among them: a malformed error.
    Transport(Transport),
}

/// Classifies an error reply by its `Encoding` alone (core §5.2, "Reading
/// an error"): [`zenkey_model::envelope::read_error`]'s reading, kept. A
/// malformed error is the transport's (§5.2, "Transport errors"): an error
/// form the parse refuses, and a reply error under another `Encoding`,
/// which a caller reads as silence and retries after.
#[must_use]
pub fn classify(e: &ReplyError) -> ErrorReply {
    let encoding = e.encoding().to_string();
    let bytes = e.payload().to_bytes();
    match envelope::read_error(&encoding, &bytes) {
        ErrorReading::Form(f) => ErrorReply::Refused(Refusal::of_form(&f)),
        ErrorReading::Transport(refusal) => ErrorReply::Transport(Transport {
            payload: bytes.into_owned(),
            encoding,
            refusal,
        }),
    }
}

/// The outcome of a one-reply call, the reply attributed (O5).
pub type Outcome = crate::call::Outcome<Answer>;

/// Every value and summary one concrete key carried (O6).
pub type Replier = crate::call::Replier<Answer, Answer>;

/// Every reply to a many-reply or fan-out call, attributed (O6).
pub type Replies = crate::call::Replies<Answer, Answer>;

/// What one reply was, as [`Replies::push`] filed it (#698).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// A value, attributed to the replier its key names (O3).
    Value,
    /// A summary (O6).
    Summary,
    /// An owner's error in its form: a refusal, which carries no key (O3).
    Refusal,
    /// The transport's own error reply, such as a timeout.
    Transport,
    /// A value on a key that is not concrete (R6), or not a member of the
    /// operation called.
    Discarded,
}

/// One reply as [`Fleet::call_timed`] saw it arrive (#698).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    /// The key a value or summary came on; `None` for an error reply,
    /// which carries none.
    pub key: Option<String>,
    /// How long after the queries went out it arrived, on this session's
    /// clock.
    pub after: Duration,
    pub kind: ReplyKind,
}

impl crate::call::Replies<Answer, Answer> {
    /// An empty collection for a call to `op`, for a tool that runs its own
    /// query and files each reply with [`Replies::push`] (#698).
    #[must_use]
    pub fn for_operation(op: &Operation) -> Self {
        Self::new(op)
    }

    /// Files one reply to a call of `r`, an operation of `iface`: a value
    /// or summary under the replier its key names (O3, O6), malformed or
    /// not, an owner's error as a refusal, anything else apart. Returns
    /// what it was.
    pub fn push(&mut self, reply: Reply, iface: &IfaceId, r: &Resource) -> ReplyKind {
        let wire = match &r.body {
            Body::Operation(op) => op.encoding,
            Body::Data(_) => None,
        };
        match reply.into_result() {
            Ok(sample) => {
                let key = sample.key_expr().as_str().to_owned();
                let Some((addr, params)) = (!key.contains('*'))
                    .then(|| values_of(iface, r, &key))
                    .flatten()
                else {
                    self.discarded += 1;
                    return ReplyKind::Discarded;
                };
                let summary = sample
                    .attachment()
                    .is_some_and(|a| a.to_bytes().as_ref() == SUMMARY);
                let i = match self.repliers.iter().position(|x| x.key == key) {
                    Some(i) => i,
                    None => {
                        self.repliers.push(ReplierOf {
                            addr: addr.clone(),
                            key,
                            params,
                            values: Vec::new(),
                            summaries: Vec::new(),
                        });
                        self.repliers.len() - 1
                    }
                };
                let answer = Answer::new(addr, sample, wire);
                if summary {
                    self.repliers[i].summaries.push(answer);
                    ReplyKind::Summary
                } else {
                    self.repliers[i].values.push(answer);
                    ReplyKind::Value
                }
            }
            Err(e) => match classify(&e) {
                ErrorReply::Refused(refusal) => {
                    self.refusals.push(refusal);
                    ReplyKind::Refusal
                }
                ErrorReply::Transport(t) => {
                    self.transport.push(t);
                    ReplyKind::Transport
                }
            },
        }
    }
}

/// What a call needs of the client's settings.
#[derive(Clone)]
struct Settings {
    timeout: Option<Duration>,
    metadata: Option<CallMetadata>,
    /// The reply timeout of the face the client calls across (core §8.1,
    /// `link.v2` §2.20): what a call waits at least, unless the client
    /// sets its own.
    face: Option<Duration>,
    /// The deployment epoch a packed record's `time` fields count from
    /// (core §7.6, §8.6): the service's, a tool's catalog's, else 0.
    epoch: i64,
}

impl Settings {
    fn new() -> Self {
        Self {
            timeout: None,
            metadata: None,
            face: None,
            epoch: crate::codec::EPOCH_WITHOUT_CATALOG,
        }
    }

    /// The client's own timeout; else the operation's `timeout_ms`, else
    /// [`DEFAULT_TIMEOUT`], raised to the face's reply timeout across a
    /// face (spec §5.1, "The timeout"; §8.1).
    fn timeout(&self, op: &Operation) -> Duration {
        self.timeout.unwrap_or_else(|| {
            let recommended = op.timeout_ms.map_or(DEFAULT_TIMEOUT, Duration::from_millis);
            self.face.map_or(recommended, |f| recommended.max(f))
        })
    }
}

/// A call's request, as a caller writes it (core §5.1, "The forms"): the
/// record already encoded as the `request` type, under CH, its width bit 0
/// and its tag the `request` type's, carried under `zenoh/bytes`.
fn request_form(contract: &Contract, op: &Operation, record: ZBytes) -> Result<ZBytes> {
    let forms = Forms::of(contract, op).map_err(Error::Contract)?;
    Ok(ZBytes::from(forms.request(&record.to_bytes())))
}

/// Fills a call's template values from R2's parameter bindings. A value the
/// caller gives for a bound parameter must be the bound one.
fn bind(params: &BTreeMap<String, String>, r: &Resource, values: &Bindings) -> Result<Bindings> {
    let mut out = values.clone();
    for (name, _) in r.template.params() {
        let Some(bound) = params.get(name) else {
            continue;
        };
        match out.get(name) {
            Some(v) if v.as_slice() != std::slice::from_ref(bound) => {
                return Err(Error::Contract(format!(
                    "parameter {name:?} is bound to {bound:?} by the role (R2), not {v:?}"
                )));
            }
            _ => {
                out.insert(name.to_owned(), vec![bound.clone()]);
            }
        }
    }
    Ok(out)
}

/// A client of one interface's operations: concrete calls at an explicit
/// address.
#[derive(Clone)]
pub struct Client {
    session: zenoh::Session,
    contract: Arc<Contract>,
    role: Option<String>,
    providers: Vec<Provider>,
    params: BTreeMap<String, String>,
    settings: Settings,
    retries: u32,
    presence: Presence,
}

impl Client {
    /// A tool's client, with no role: `providers` names the services it may
    /// call, exact or wildcard (`*/*` for any). Tools and scripts use one
    /// without a `Service`.
    pub fn new(
        session: &zenoh::Session,
        contract: Arc<Contract>,
        providers: &[&str],
    ) -> Result<Self> {
        Ok(Self {
            session: session.clone(),
            contract,
            role: None,
            providers: providers
                .iter()
                .map(|p| Provider::parse(p))
                .collect::<Result<_>>()?,
            params: BTreeMap::new(),
            settings: Settings::new(),
            retries: 0,
            presence: Presence::Observable,
        })
    }

    pub(crate) fn for_role(
        session: &zenoh::Session,
        me: &Addr,
        role: &str,
        contract: Arc<Contract>,
        providers: &[String],
        params: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let refs: Vec<&str> = providers.iter().map(String::as_str).collect();
        let mut c = Self::new(session, contract, &refs)?;
        c.role = Some(role.to_owned());
        c.params = params
            .iter()
            .map(|(k, v)| {
                let v = match v.as_str() {
                    "self.system" => me.system.to_string(),
                    "self.service" => me.service.to_string(),
                    other => other.to_owned(),
                };
                (k.clone(), v)
            })
            .collect();
        Ok(c)
    }

    /// The reply timeout for every call, over the contract's `timeout_ms`
    /// and the face's (core §8.1: the timeout is the caller's).
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.settings.timeout = Some(timeout);
        self
    }

    /// Calls across a constrained face (core §8.1, `link.v2` §2.20): unless
    /// [`Client::with_timeout`] sets one, every call waits at least the
    /// face's `reply_timeout_ms`, over the contract's `timeout_ms` and
    /// [`DEFAULT_TIMEOUT`]. A reply later than the query's timeout is
    /// dropped by the router, so no router setting makes up for a caller
    /// that gave up first (core Appendix B). The session is the caller's,
    /// opened with the face's transport ([`zenkey_model::link::FarSide::session_keys`]).
    #[must_use]
    pub fn across(mut self, far: &zenkey_model::link::FarSide) -> Self {
        self.settings.face = Some(far.reply_timeout());
        self
    }

    /// The deployment epoch a packed request's and reply's `time` fields
    /// count from (core §7.6, §8.6): a tool's from the catalog it holds for
    /// the owner. A client from a service's role takes the service's.
    #[must_use]
    pub fn with_deployment_epoch(mut self, epoch: i64) -> Self {
        self.settings.epoch = epoch;
        self
    }

    /// The deployment epoch the typed calls write and read under.
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.settings.epoch
    }

    /// Retries after silence, for idempotent operations only (O4). A
    /// non-idempotent operation is called once, whatever this says.
    #[must_use]
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }

    /// The call metadata every request carries (O7).
    #[must_use]
    pub fn with_metadata(mut self, metadata: CallMetadata) -> Self {
        self.settings.metadata = Some(metadata);
        self
    }

    /// Marks presence unobservable for these providers (R7): silence is then
    /// attributed [`Attribution::Unobservable`].
    #[must_use]
    pub fn with_presence(mut self, presence: Presence) -> Self {
        self.presence = presence;
        self
    }

    #[must_use]
    pub fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    #[must_use]
    pub fn interface(&self) -> &IfaceId {
        &self.contract.iface
    }

    /// The services this client may call (R1).
    #[must_use]
    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub(crate) fn op(&self, resource: &str) -> Result<(Resource, Operation)> {
        let r = find_resource(&self.contract, resource)?;
        Ok((r.clone(), operation(&self.contract.iface, r)?))
    }

    /// The concrete key a call to `addr` uses: the address must be one the
    /// providers name (R1), and every parameter bound, by the caller or by
    /// the role (R2).
    pub fn key(&self, addr: &Addr, resource: &str, values: &Bindings) -> Result<Key> {
        let (r, _) = self.op(resource)?;
        self.key_of(addr, &r, &bind(&self.params, &r, values)?)
    }

    fn key_of(&self, addr: &Addr, r: &Resource, values: &Bindings) -> Result<Key> {
        if !self.providers.iter().any(|p| p.matches(addr)) {
            return Err(Error::Contract(format!(
                "{addr} is not among the providers this client may call (R1)"
            )));
        }
        let chunks = r
            .template
            .build(values)
            .map_err(|e| Error::Contract(format!("{} {}: {e}", self.contract.iface, r.template)))?;
        let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
        Ok(data_key(addr, &self.contract.iface, KindToken::Op, &refs)?)
    }

    /// One query, `request` in its form already ([`request_form`]).
    async fn get(
        &self,
        key: OwnedKeyExpr,
        target: QueryTarget,
        op: &Operation,
        request: ZBytes,
    ) -> Result<flume::Receiver<Reply>> {
        self.session
            .get(key)
            .payload(request)
            .encoding(Encoding::ZENOH_BYTES)
            .attachment(self.settings.metadata.as_ref().map(CallMetadata::to_bytes))
            .target(target)
            .consolidation(ConsolidationMode::None)
            .timeout(self.settings.timeout(op))
            .priority(crate::qos::priority(op.priority))
            .with(flume::unbounded::<Reply>())
            .await
            .map_err(zenoh)
    }

    /// Calls a one-reply operation at `addr` (O1): `request` is already
    /// encoded as the request type, its record, which the call writes in
    /// the request form (core §5.1). Returns on the first value or owner's
    /// error; after silence, the transport's own error
    /// replies included, an idempotent operation is retried (O4) and the
    /// silence is attributed through presence (O5).
    pub async fn call(
        &self,
        addr: &Addr,
        resource: &str,
        values: &Bindings,
        request: impl Into<ZBytes>,
    ) -> Result<Outcome> {
        let (r, op) = self.op(resource)?;
        if op.replies == RepliesKind::Many {
            return Err(Error::Contract(format!(
                "{} {resource:?} is replies = \"many\": use call_many (O6)",
                self.contract.iface
            )));
        }
        let values = bind(&self.params, &r, values)?;
        let key = self.key_of(addr, &r, &values)?;
        let request = request_form(&self.contract, &op, request.into())?;
        let attempts = if op.idempotent { 1 + self.retries } else { 1 };
        let mut transport = None;
        for _ in 0..attempts {
            let rx = self
                .get(
                    key.clone().into_keyexpr(),
                    QueryTarget::BestMatching,
                    &op,
                    request.clone(),
                )
                .await?;
            while let Ok(reply) = rx.recv_async().await {
                match reply.into_result() {
                    // O3: a value reply goes on the call's own key; another
                    // key is not this call's answer.
                    Ok(sample) if sample.key_expr().as_str() == key.as_str() => {
                        return Ok(Outcome::Value(Answer::new(
                            addr.clone(),
                            sample,
                            op.encoding,
                        )));
                    }
                    Ok(_) => {}
                    Err(e) => match classify(&e) {
                        ErrorReply::Refused(refusal) => return Ok(Outcome::Refused(refusal)),
                        ErrorReply::Transport(t) => transport = Some(t),
                    },
                }
            }
        }
        Ok(Outcome::NoAnswer(Silence {
            attempts,
            transport,
            presence: self.attribute(addr, self.settings.timeout(&op)).await?,
        }))
    }

    /// [`Client::call`] with a request of a JSON Schema type, encoded as the
    /// contract says (§7.2).
    pub async fn call_value<T: Serialize>(
        &self,
        addr: &Addr,
        resource: &str,
        values: &Bindings,
        request: &T,
    ) -> Result<Outcome> {
        let (_, op) = self.op(resource)?;
        let bytes = encode_value(request, op.encoding)?;
        self.call(addr, resource, values, bytes).await
    }

    /// Calls a many-reply operation at `addr`, collecting every value and the
    /// summary until the query completes (O6). Retried after silence only
    /// when idempotent (O4).
    pub async fn call_many(
        &self,
        addr: &Addr,
        resource: &str,
        values: &Bindings,
        request: impl Into<ZBytes>,
    ) -> Result<Replies> {
        let (r, op) = self.op(resource)?;
        let values = bind(&self.params, &r, values)?;
        let key = self.key_of(addr, &r, &values)?;
        let request = request_form(&self.contract, &op, request.into())?;
        let attempts = if op.idempotent { 1 + self.retries } else { 1 };
        let mut out = Replies::new(&op);
        for _ in 0..attempts {
            out = Replies::new(&op);
            let rx = self
                .get(
                    key.clone().into_keyexpr(),
                    QueryTarget::BestMatching,
                    &op,
                    request.clone(),
                )
                .await?;
            while let Ok(reply) = rx.recv_async().await {
                out.push(reply, &self.contract.iface, &r);
            }
            if !out.is_silent() {
                break;
            }
        }
        Ok(out)
    }

    /// What presence says of `addr` (O5): its interface token, else its
    /// instance token, else nothing, from one read of its `@zk` subtree. A
    /// read that may be incomplete (§8.1) and saw no interface token is
    /// [`Attribution::Unknown`], never absence.
    pub async fn attribute(&self, addr: &Addr, timeout: Duration) -> Result<Attribution> {
        if self.presence == Presence::Unavailable {
            return Ok(Attribution::Unobservable);
        }
        let sel = format!("zk2/{}/{}/@zk/**", addr.system, addr.service);
        let read = crate::presence::liveliness_read(&self.session, &sel, timeout).await?;
        let tokens: Vec<ZkKey> = read
            .keys
            .iter()
            .filter_map(|k| zenkey_model::grammar::parse(k).ok())
            .collect();
        let iface = &self.contract.iface;
        Ok(Attribution::of_read(
            tokens
                .iter()
                .any(|t| matches!(t, ZkKey::Alive { iface: i, .. } if i == iface)),
            tokens.iter().any(|t| matches!(t, ZkKey::Instance { .. })),
            read.complete,
        ))
    }

    /// The providers serving this interface now (§8.1): each holding its
    /// token, or, for an interface in its tokenless set, an instance token
    /// whose descriptor lists it `"token": false` (#1015).
    pub async fn present(&self, timeout: Duration) -> Result<Vec<Addr>> {
        Ok(self.presence(timeout).await?.providers)
    }

    /// [`Client::present`], with whether the reads were complete (§8.1):
    /// a provider missing from a possibly incomplete read is not absent
    /// (O5, #671; observer §3.6).
    pub async fn presence(&self, timeout: Duration) -> Result<Present> {
        crate::presence::serving(
            &self.session,
            &self.providers,
            &self.contract.iface,
            timeout,
            None,
        )
        .await
    }

    /// A [`Fleet`] over this client's providers, with its parameter
    /// bindings, timeout and metadata.
    #[must_use]
    pub fn fleet(&self) -> Fleet {
        Fleet {
            session: self.session.clone(),
            contract: Arc::clone(&self.contract),
            selection: covering(&self.providers),
            params: self.params.clone(),
            settings: self.settings.clone(),
        }
    }
}

/// Who serves an interface, read from presence (§8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Present {
    /// The providers serving it, sorted: each holding the interface's
    /// token, or, for an interface in its tokenless set (U22), an instance
    /// token whose descriptor lists the interface `"token": false` (§3.3,
    /// #1015).
    pub providers: Vec<Addr>,
    /// `false` when any read behind it was possibly incomplete: a
    /// liveliness read that got an error reply, the timeout's included, was
    /// cut off while it ran or ran to the session's interests timeout (core
    /// §8.1), a read not made once the session was cut off, or a descriptor
    /// in [`Present::undescribed`]. A provider missing from it may be there
    /// (§8.1, O5; observer §3.6). A complete read can still have been
    /// refused by access control, which looks the same as absence (§8.1,
    /// 0.8).
    pub complete: bool,
    /// Every error reply of those reads, as
    /// [`crate::presence::PresenceRead::errors`] spells them, then why each
    /// descriptor in [`Present::undescribed`] was not read, as `the
    /// descriptor of <system>/<service> (<instance>): <why>`.
    pub errors: Vec<String>,
    /// The addresses holding an instance token and no interface token whose
    /// descriptor was not read: its GET failed, had no reply, answered one
    /// that fails its checks (§3.3) or was not asked before a deadline.
    /// Whether they serve the interface, tokenless, is unknown. Sorted.
    pub undescribed: Vec<Addr>,
}

/// The selection without the providers another one already covers, so that
/// no service is called twice by one fan-out.
fn covering(providers: &[Provider]) -> Vec<Provider> {
    let covers = |a: &Provider, b: &Provider| {
        let ((asys, asvc), (bsys, bsvc)) = (a.chunks(), b.chunks());
        (asys == "*" || asys == bsys) && (asvc == "*" || asvc == bsvc)
    };
    let mut out: Vec<Provider> = Vec::new();
    for (i, p) in providers.iter().enumerate() {
        let covered = providers
            .iter()
            .enumerate()
            .any(|(j, q)| j != i && covers(q, p) && (!covers(p, q) || j < i));
        if !covered {
            out.push(p.clone());
        }
    }
    out
}

/// Fan-out calls (O2) to a selection spelled by name or wildcard, never
/// defaulted from the local process (§1.5): `*/tc` is every system's `tc`.
#[derive(Clone)]
pub struct Fleet {
    session: zenoh::Session,
    contract: Arc<Contract>,
    selection: Vec<Provider>,
    params: BTreeMap<String, String>,
    settings: Settings,
}

impl Fleet {
    /// A fleet over `selection`, each `<system>/<service>` with either
    /// position `*`.
    pub fn new(
        session: &zenoh::Session,
        contract: Arc<Contract>,
        selection: &[&str],
    ) -> Result<Self> {
        let selection: Vec<Provider> = selection
            .iter()
            .map(|p| Provider::parse(p))
            .collect::<Result<_>>()?;
        Ok(Self {
            session: session.clone(),
            contract,
            selection: covering(&selection),
            params: BTreeMap::new(),
            settings: Settings::new(),
        })
    }

    /// The time every fan-out waits for replies, over the contract's
    /// `timeout_ms`. A fan-out always waits it out: the query completes
    /// when every replier has.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.settings.timeout = Some(timeout);
        self
    }

    /// The deployment epoch the typed fan-outs write and read under
    /// ([`Client::with_deployment_epoch`]).
    #[must_use]
    pub fn with_deployment_epoch(mut self, epoch: i64) -> Self {
        self.settings.epoch = epoch;
        self
    }

    /// The deployment epoch the typed fan-outs write and read under.
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.settings.epoch
    }

    /// Fans out across a constrained face: [`Client::across`]'s timeout.
    #[must_use]
    pub fn across(mut self, far: &zenkey_model::link::FarSide) -> Self {
        self.settings.face = Some(far.reply_timeout());
        self
    }

    /// The call metadata every request carries (O7).
    #[must_use]
    pub fn with_metadata(mut self, metadata: CallMetadata) -> Self {
        self.settings.metadata = Some(metadata);
        self
    }

    /// The selection, minus providers another already covers.
    #[must_use]
    pub fn selection(&self) -> &[Provider] {
        &self.selection
    }

    pub(crate) fn op(&self, resource: &str) -> Result<(Resource, Operation)> {
        let r = find_resource(&self.contract, resource)?;
        let op = operation(&self.contract.iface, r)?;
        if op.fanout != Fanout::Allowed {
            return Err(Error::Contract(format!(
                "{} {resource:?} is fanout = \"forbidden\": call one address with Client::call (O2)",
                self.contract.iface
            )));
        }
        Ok((r.clone(), op))
    }

    /// The key expressions a fan-out on `resource` uses: one per selected
    /// provider, each given value slugged into its chunk and every other
    /// parameter a wildcard.
    pub fn selectors(&self, resource: &str, values: &Bindings) -> Result<Vec<OwnedKeyExpr>> {
        let (r, _) = self.op(resource)?;
        self.selectors_of(&r, &bind(&self.params, &r, values)?)
    }

    fn selectors_of(&self, r: &Resource, values: &Bindings) -> Result<Vec<OwnedKeyExpr>> {
        let tail: Vec<String> = r
            .template
            .segments()
            .iter()
            .map(|seg| match seg {
                Segment::Literal(l) => l.clone(),
                Segment::Param(n) => match values.get(n).map(Vec::as_slice) {
                    Some([v]) => chunk_slug(v),
                    _ => "*".to_owned(),
                },
                Segment::Rest(n) => match values.get(n) {
                    Some(vs) if !vs.is_empty() => vs
                        .iter()
                        .map(|v| chunk_slug(v))
                        .collect::<Vec<_>>()
                        .join("/"),
                    _ => "**".to_owned(),
                },
            })
            .collect();
        self.selection
            .iter()
            .map(|p| {
                let (sys, svc) = p.chunks();
                let mut chunks = vec![
                    "zk2".to_owned(),
                    sys.to_owned(),
                    svc.to_owned(),
                    self.contract.iface.to_string(),
                    KindToken::Op.to_string(),
                ];
                chunks.extend(tail.iter().cloned());
                OwnedKeyExpr::try_from(chunks.join("/")).map_err(zenoh)
            })
            .collect()
    }

    /// Fans a call out (O2): refused unless the operation declares
    /// `fanout = "allowed"`; target `All`, consolidation `None`, and every
    /// reply collected until the query completes, attributed by its key
    /// (O3, O6). A fan-out is never retried.
    pub async fn call(
        &self,
        resource: &str,
        values: &Bindings,
        request: impl Into<ZBytes>,
    ) -> Result<Replies> {
        Ok(self.call_timed(resource, values, request).await?.0)
    }

    /// [`Fleet::call`], with when each reply arrived (#698): a bench's
    /// per-reply latency, attributed by key (O3), with refusals and
    /// transport replies timed apart. The replies are drained as they
    /// arrive, query by query.
    pub async fn call_timed(
        &self,
        resource: &str,
        values: &Bindings,
        request: impl Into<ZBytes>,
    ) -> Result<(Replies, Vec<Arrival>)> {
        let (r, op) = self.op(resource)?;
        let values = bind(&self.params, &r, values)?;
        let request = request_form(&self.contract, &op, request.into())?;
        // Every query goes out before any is drained, so they run together.
        let started = std::time::Instant::now();
        let mut receivers = Vec::new();
        for ke in self.selectors_of(&r, &values)? {
            receivers.push(
                self.session
                    .get(ke)
                    .payload(request.clone())
                    .encoding(Encoding::ZENOH_BYTES)
                    .attachment(self.settings.metadata.as_ref().map(CallMetadata::to_bytes))
                    .target(QueryTarget::All)
                    .consolidation(ConsolidationMode::None)
                    .timeout(self.settings.timeout(&op))
                    .priority(crate::qos::priority(op.priority))
                    .with(flume::unbounded::<Reply>())
                    .await
                    .map_err(zenoh)?,
            );
        }
        // Each query's replies are drained by a task of its own and stamped
        // on arrival, so one slow replier does not delay another's time.
        let (tx, rx) = flume::unbounded::<(Reply, Duration)>();
        for q in receivers {
            let tx = tx.clone();
            tokio::spawn(async move {
                while let Ok(reply) = q.recv_async().await {
                    if tx.send((reply, started.elapsed())).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut out = Replies::new(&op);
        let mut arrivals = Vec::new();
        while let Ok((reply, after)) = rx.recv_async().await {
            let key = reply
                .result()
                .ok()
                .map(|s| s.key_expr().as_str().to_owned());
            let kind = out.push(reply, &self.contract.iface, &r);
            arrivals.push(Arrival { key, after, kind });
        }
        Ok((out, arrivals))
    }

    /// [`Fleet::call`] with a request of a JSON Schema type.
    pub async fn call_value<T: Serialize>(
        &self,
        resource: &str,
        values: &Bindings,
        request: &T,
    ) -> Result<Replies> {
        let (_, op) = self.op(resource)?;
        self.call(resource, values, encode_value(request, op.encoding)?)
            .await
    }

    /// The selected services serving this interface now (§8.1), by its
    /// token or, tokenless, by instance token and descriptor (#1015): who a
    /// fan-out should hear from, to attribute silence (O5).
    pub async fn present(&self, timeout: Duration) -> Result<Vec<Addr>> {
        Ok(self.presence(timeout).await?.providers)
    }

    /// [`Fleet::present`], with whether the reads were complete (§8.1): a
    /// service missing from a possibly incomplete read is not one that sent
    /// nothing (O5, #671).
    pub async fn presence(&self, timeout: Duration) -> Result<Present> {
        crate::presence::serving(
            &self.session,
            &self.selection,
            &self.contract.iface,
            timeout,
            None,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{Provider, covering};

    #[test]
    fn a_fan_out_selection_calls_no_service_twice() {
        let p = |s: &str| Provider::parse(s).unwrap();
        let sel = |v: &[&str]| -> Vec<String> {
            covering(&v.iter().map(|s| p(s)).collect::<Vec<_>>())
                .iter()
                .map(|p| {
                    let (a, b) = p.chunks();
                    format!("{a}/{b}")
                })
                .collect()
        };
        assert_eq!(sel(&["v1/*", "v1/teleop", "v2/x"]), ["v1/*", "v2/x"]);
        assert_eq!(sel(&["*/tc", "*/tc"]), ["*/tc"]);
        assert_eq!(sel(&["a/b", "*/*"]), ["*/*"]);
    }
}
