//! Calling operations (spec §5.1): a [`Client`] calls one concrete key at an
//! explicit address, a [`Fleet`] fans out to a selection spelled by name.
//!
//! - **Concrete calls (O1):** target `BestMatching`, consolidation `None`,
//!   the operation's recommended `priority` (§2.4: replies inherit the
//!   query's QoS). A one-reply call returns on the first value or envelope,
//!   without waiting for the query to complete.
//! - **Fan-out calls (O2):** only to an operation declaring
//!   `fanout = "allowed"`, with target `All` and consolidation `None`; every
//!   reply is collected and attributed to its replier by its key (O3).
//! - **O3:** an error reply decodes as the envelope by its `Encoding`, without
//!   the contract (§5.2). Any other error reply is the transport's.
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
use zenkey_model::contract::{Contract, Fanout, Operation, Replies as RepliesKind, Resource};
use zenkey_model::envelope::{self, Envelope};
use zenkey_model::grammar::{Addr, IfaceId, Key, KindToken, ZkKey, data_key};
use zenkey_model::slug::chunk_slug;
use zenkey_model::template::{Bindings, Segment};
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::query::{ConsolidationMode, QueryTarget, Reply, ReplyError};
use zenoh::sample::Sample;

use crate::call::Replier as ReplierOf;
pub use crate::call::{Attribution, Malformed, Silence};
use crate::consumer::{Presence, Provider};
use crate::error::{Error, Result, zenoh};
use crate::operation::{CallMetadata, SUMMARY, decode_value, find_resource, operation, values_of};
use crate::writer::{encode_value, wire_encoding};

/// The reply timeout when neither the client nor the contract's
/// `timeout_ms` sets one: zenoh's own default.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// A value reply, attributed to the service whose key carried it (O3).
#[derive(Debug, Clone)]
pub struct Answer {
    pub replier: Addr,
    pub sample: Sample,
}

impl Answer {
    /// The concrete key the reply went on.
    #[must_use]
    pub fn key(&self) -> &str {
        self.sample.key_expr().as_str()
    }

    #[must_use]
    pub fn payload(&self) -> &ZBytes {
        self.sample.payload()
    }

    /// The reply, decoded as a JSON Schema value by its `Encoding` (§7.2).
    pub fn value<T: DeserializeOwned>(&self) -> Result<T> {
        decode_value(
            &self.sample.payload().to_bytes(),
            Some(self.sample.encoding()),
            None,
        )
        .map_err(Error::Contract)
    }
}

/// What an error reply is (§5.2): an envelope, a malformed one, or the
/// transport's own error (a query timeout, `zenoh/string`).
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorReply {
    Envelope(Envelope),
    Malformed(Malformed),
    Transport(String),
}

/// Classifies an error reply by its `Encoding` alone (§5.2): only the
/// three envelope encodings carry an envelope.
#[must_use]
pub fn classify(e: &ReplyError) -> ErrorReply {
    let encoding = e.encoding().to_string();
    let bytes = e.payload().to_bytes();
    if [envelope::JSON, envelope::CBOR, envelope::PROTOBUF].contains(&encoding.as_str()) {
        match envelope::decode(&encoding, &bytes) {
            Ok(env) => ErrorReply::Envelope(env),
            Err(error) => ErrorReply::Malformed(Malformed { encoding, error }),
        }
    } else {
        ErrorReply::Transport(format!("{encoding}: {}", String::from_utf8_lossy(&bytes)))
    }
}

/// The outcome of a one-reply call, the reply attributed (O5).
pub type Outcome = crate::call::Outcome<Answer>;

/// Every value and summary one concrete key carried (O6).
pub type Replier = crate::call::Replier<Answer, Answer>;

/// Every reply to a many-reply or fan-out call, attributed (O6).
pub type Replies = crate::call::Replies<Answer, Answer>;

impl crate::call::Replies<Answer, Answer> {
    pub(crate) fn push(&mut self, reply: Reply, iface: &IfaceId, r: &Resource) {
        match reply.into_result() {
            Ok(sample) => {
                let key = sample.key_expr().as_str().to_owned();
                let Some((addr, params)) = (!key.contains('*'))
                    .then(|| values_of(iface, r, &key))
                    .flatten()
                else {
                    self.discarded += 1;
                    return;
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
                let answer = Answer {
                    replier: addr,
                    sample,
                };
                if summary {
                    self.repliers[i].summaries.push(answer);
                } else {
                    self.repliers[i].values.push(answer);
                }
            }
            Err(e) => match classify(&e) {
                ErrorReply::Envelope(env) => self.refusals.push(env),
                ErrorReply::Malformed(m) => self.malformed.push(m),
                ErrorReply::Transport(t) => self.transport.push(t),
            },
        }
    }
}

/// What a call needs of the client's settings.
#[derive(Clone)]
struct Settings {
    timeout: Option<Duration>,
    metadata: Option<CallMetadata>,
}

impl Settings {
    fn timeout(&self, op: &Operation) -> Duration {
        self.timeout
            .or(op.timeout_ms.map(Duration::from_millis))
            .unwrap_or(DEFAULT_TIMEOUT)
    }
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
            settings: Settings {
                timeout: None,
                metadata: None,
            },
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

    /// The reply timeout for every call, over the contract's `timeout_ms`.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.settings.timeout = Some(timeout);
        self
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

    async fn get(
        &self,
        key: OwnedKeyExpr,
        target: QueryTarget,
        op: &Operation,
        encoding: Encoding,
        request: ZBytes,
    ) -> Result<flume::Receiver<Reply>> {
        self.session
            .get(key)
            .payload(request)
            .encoding(encoding)
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
    /// encoded as the request type. Returns on the first value or envelope;
    /// after silence, an idempotent operation is retried (O4) and the
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
        let encoding = wire_encoding(&op.request, op.encoding, &values);
        let request: ZBytes = request.into();
        let attempts = if op.idempotent { 1 + self.retries } else { 1 };
        let mut transport = None;
        for _ in 0..attempts {
            let rx = self
                .get(
                    key.clone().into_keyexpr(),
                    QueryTarget::BestMatching,
                    &op,
                    encoding.clone(),
                    request.clone(),
                )
                .await?;
            while let Ok(reply) = rx.recv_async().await {
                match reply.into_result() {
                    // O3: a value reply goes on the call's own key; another
                    // key is not this call's answer.
                    Ok(sample) if sample.key_expr().as_str() == key.as_str() => {
                        return Ok(Outcome::Value(Answer {
                            replier: addr.clone(),
                            sample,
                        }));
                    }
                    Ok(_) => {}
                    Err(e) => match classify(&e) {
                        ErrorReply::Envelope(env) => return Ok(Outcome::Refused(env)),
                        ErrorReply::Malformed(m) => return Ok(Outcome::Malformed(m)),
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
        let encoding = wire_encoding(&op.request, op.encoding, &values);
        let request: ZBytes = request.into();
        let attempts = if op.idempotent { 1 + self.retries } else { 1 };
        let mut out = Replies::new(&op);
        for _ in 0..attempts {
            out = Replies::new(&op);
            let rx = self
                .get(
                    key.clone().into_keyexpr(),
                    QueryTarget::BestMatching,
                    &op,
                    encoding.clone(),
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

    /// The providers holding this interface's token now (§8.1).
    pub async fn present(&self, timeout: Duration) -> Result<Vec<Addr>> {
        present(
            &self.session,
            &self.providers,
            &self.contract.iface,
            timeout,
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

async fn present(
    session: &zenoh::Session,
    providers: &[Provider],
    iface: &IfaceId,
    timeout: Duration,
) -> Result<Vec<Addr>> {
    let mut out = Vec::new();
    for p in providers {
        let (sys, svc) = p.chunks();
        let sel = format!("zk2/{sys}/{svc}/@zk/alive/{iface}/**");
        for t in crate::presence::tokens(session, &sel, timeout).await? {
            if let ZkKey::Alive { addr, .. } = t
                && !out.contains(&addr)
            {
                out.push(addr);
            }
        }
    }
    out.sort();
    Ok(out)
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
            settings: Settings {
                timeout: None,
                metadata: None,
            },
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
        let (r, op) = self.op(resource)?;
        let values = bind(&self.params, &r, values)?;
        let encoding = wire_encoding(&op.request, op.encoding, &values);
        let request: ZBytes = request.into();
        // Every query goes out before any is drained, so they run together.
        let mut receivers = Vec::new();
        for ke in self.selectors_of(&r, &values)? {
            receivers.push(
                self.session
                    .get(ke)
                    .payload(request.clone())
                    .encoding(encoding.clone())
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
        let mut out = Replies::new(&op);
        for rx in receivers {
            while let Ok(reply) = rx.recv_async().await {
                out.push(reply, &self.contract.iface, &r);
            }
        }
        Ok(out)
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

    /// The selected services holding this interface's token now (§8.1): who
    /// a fan-out should hear from, to attribute silence (O5).
    pub async fn present(&self, timeout: Duration) -> Result<Vec<Addr>> {
        present(
            &self.session,
            &self.selection,
            &self.contract.iface,
            timeout,
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
