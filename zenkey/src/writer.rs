//! Writing a resource's data (spec §2.4–§2.6, §7.2): the contract's QoS and
//! `Encoding` on every sample, advanced publication when the contract
//! declares `history`, and events as one-shot puts on fresh ULID keys.
//!
//! State's own rules (stamps, tombstones, GET answering, S1–S7) are #620's,
//! built on [`Writer`]; this module only publishes.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use zenkey_model::authoring::{Encoding as WireEncoding, Kind};
use zenkey_model::contract::{Body, Data, Resource};
use zenkey_model::schema::TypeId;
use zenkey_model::template::Bindings;
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::pubsub::Publisher;
use zenoh::qos::{CongestionControl, Priority, Reliability};
use zenoh_ext::{AdvancedPublisher, AdvancedPublisherBuilderExt, CacheConfig, MissDetectionConfig};

use crate::budget::Budget;
use crate::error::{Error, Result, zenoh};
use crate::qos;
use crate::telemetry::{Meter, Pending, Reason};

/// The `Encoding` a sample of `ty` carries (§7.2): zenoh's predefined one
/// for protobuf, JSON and CBOR, and for a raw media type the predefined one
/// when zenoh has it, its custom encoding otherwise (zenoh does that by
/// itself from the string). A raw family (`video/*` with `media_param`)
/// takes its subtype from that parameter's value. Never a schema suffix.
#[must_use]
pub fn wire_encoding(ty: &TypeId, encoding: Option<WireEncoding>, values: &Bindings) -> Encoding {
    match ty {
        TypeId::Protobuf { .. } => Encoding::APPLICATION_PROTOBUF,
        TypeId::JsonSchema { .. } => match encoding {
            Some(WireEncoding::Cbor) => Encoding::APPLICATION_CBOR,
            _ => Encoding::APPLICATION_JSON,
        },
        TypeId::Raw {
            media_type,
            media_param,
        } => {
            let concrete = match (media_type.strip_suffix("/*"), media_param) {
                (Some(top), Some(p)) => values
                    .get(p)
                    .and_then(|v| v.first())
                    .map_or_else(|| media_type.clone(), |sub| format!("{top}/{sub}")),
                _ => media_type.clone(),
            };
            Encoding::from(concrete)
        }
    }
}

fn data(r: &Resource) -> Result<&Data> {
    match &r.body {
        Body::Data(d) => Ok(d),
        Body::Operation(_) => Err(Error::Contract(format!(
            "{} is an operation: it is served by a queryable, not written",
            r.template
        ))),
    }
}

enum Inner {
    Plain(Publisher<'static>),
    Advanced(AdvancedPublisher<'static>),
}

/// A publisher on one stream or state member, with the contract's QoS and
/// `Encoding` (§2.4, §7.2). With `history` in the contract it is zenoh-ext's
/// advanced publisher (§2.5): a cache of that depth, publisher detection,
/// and heartbeats when `miss_detection_ms` is set.
///
/// Made by its service, it keeps the resource's budget (§2.7, #741): a put
/// that would make its member a new live one while the live members are at
/// the bound is refused, [`Error::Budget`], and nothing is published.
///
/// On a metric resource (`telemetry.v1`, [`crate::telemetry`]), each put is
/// checked against the resource's declaration before the budget: a sample
/// that does not conform, a counter or histogram below the last one this
/// writer put in its epoch, or an in-band epoch other than the owner's is
/// refused, [`Error::Telemetry`], and nothing is published; a counter's or
/// histogram's epoch the application left out is written in.
pub struct Writer {
    inner: Inner,
    encoding: Encoding,
    wire: Option<WireEncoding>,
    json_typed: bool,
    budget: Option<Arc<Budget>>,
    meter: Option<Meter>,
}

impl Writer {
    pub(crate) async fn declare(
        session: &zenoh::Session,
        key: OwnedKeyExpr,
        r: &Resource,
        values: &Bindings,
    ) -> Result<Self> {
        let d = data(r)?;
        if r.kind == Kind::Event {
            return Err(Error::Contract(format!(
                "{} is an event: write it with an EventWriter (one key per occurrence)",
                r.template
            )));
        }
        let encoding = wire_encoding(&d.type_, d.encoding, values);
        let inner = match &d.history {
            None => Inner::Plain(
                session
                    .declare_publisher(key)
                    .encoding(encoding.clone())
                    .reliability(qos::reliability(d.reliability))
                    .congestion_control(qos::congestion(d.congestion))
                    .priority(qos::priority(d.priority))
                    .express(d.express)
                    .await
                    .map_err(zenoh)?,
            ),
            Some(h) => {
                let mut b = session
                    .declare_publisher(key)
                    .encoding(encoding.clone())
                    .reliability(qos::reliability(d.reliability))
                    .congestion_control(qos::congestion(d.congestion))
                    .priority(qos::priority(d.priority))
                    .express(d.express)
                    .cache(CacheConfig::default().max_samples(h.depth as usize))
                    .publisher_detection();
                if let Some(ms) = h.miss_detection_ms {
                    b = b.sample_miss_detection(
                        MissDetectionConfig::default().heartbeat(Duration::from_millis(ms)),
                    );
                }
                Inner::Advanced(b.await.map_err(zenoh)?)
            }
        };
        if r.token == zenkey_model::grammar::KindToken::ExplicitStream {
            crate::shm::warn_if_memlock_low();
        }
        Ok(Self {
            inner,
            encoding,
            wire: d.encoding,
            json_typed: matches!(d.type_, TypeId::JsonSchema { .. }),
            budget: None,
            meter: None,
        })
    }

    /// Keeps `budget`, its resource's in its service (§2.7).
    pub(crate) fn bounded(mut self, budget: Option<Arc<Budget>>) -> Self {
        self.budget = budget;
        self
    }

    /// Holds its puts to `telemetry.v1` through `meter`, for a member of a
    /// metric resource ([`crate::telemetry`]).
    pub(crate) fn metered(mut self, meter: Option<Meter>) -> Self {
        self.meter = meter;
        self
    }

    /// The resource's `telemetry.v1` declaration, when it is a metric
    /// resource whose puts this writer checks ([`crate::telemetry`]): its
    /// kind, unit and bounds, the bounds a [`crate::telemetry::Histogram`]
    /// is made over.
    #[must_use]
    pub fn telemetry(&self) -> Option<&zenkey_model::telemetry::Declared> {
        self.meter.as_ref().map(Meter::declared)
    }

    /// The in-band epoch this writer writes now (`telemetry.v1` §2.9): the
    /// service's instance id, or the member's epoch on the template that
    /// declares `epoch`. `None` for a resource that is not a metric one.
    #[must_use]
    pub fn epoch(&self) -> Option<zenkey_model::grammar::InstanceId> {
        self.meter.as_ref().map(Meter::epoch)
    }

    /// Checks a metric sample before it is published (`telemetry.v1`):
    /// the payload and attachment to publish, the epoch written in, and the
    /// writer's last value held until [`Pending::commit`]. Unmetered, they
    /// pass as they are.
    pub(crate) async fn meter(
        &self,
        payload: ZBytes,
        attachment: Option<ZBytes>,
    ) -> Result<(ZBytes, Option<ZBytes>, Option<Pending<'_>>)> {
        let Some(m) = &self.meter else {
            return Ok((payload, attachment, None));
        };
        let (p, a, pending) = m
            .prepare(payload, attachment)
            .await
            .map_err(|reason| self.refused(reason))?;
        Ok((p, a, Some(pending)))
    }

    fn refused(&self, reason: Reason) -> Error {
        Error::Telemetry {
            member: self.key_expr().as_str().to_owned(),
            reason: Box::new(reason),
        }
    }

    /// The member's write admitted by its resource's budget now, or refused
    /// (§2.7): a put of a state member, a publication of a stream one.
    pub(crate) fn admit(&self) -> Result<()> {
        match &self.budget {
            Some(b) => Ok(b.admit(self.key_expr().as_str(), Instant::now())?),
            None => Ok(()),
        }
    }

    /// The member deleted: a state member is live no more (§2.7).
    pub(crate) fn release(&self) {
        if let Some(b) = &self.budget {
            b.release(self.key_expr().as_str());
        }
    }

    /// The `Encoding` every sample carries.
    #[must_use]
    pub fn encoding(&self) -> &Encoding {
        &self.encoding
    }

    /// The member's key expression.
    #[must_use]
    pub fn key_expr(&self) -> &zenoh::key_expr::KeyExpr<'static> {
        match &self.inner {
            Inner::Plain(p) => p.key_expr(),
            Inner::Advanced(p) => p.key_expr(),
        }
    }

    /// The plain zenoh publisher, when the resource has no `history`.
    #[must_use]
    pub fn publisher(&self) -> Option<&Publisher<'static>> {
        match &self.inner {
            Inner::Plain(p) => Some(p),
            Inner::Advanced(_) => None,
        }
    }

    /// Whether a subscriber matches (zenoh's matching status).
    pub async fn matching(&self) -> Result<bool> {
        Ok(match &self.inner {
            Inner::Plain(p) => p.matching_status().await.map_err(zenoh)?.matching(),
            Inner::Advanced(p) => p.matching_status().await.map_err(zenoh)?.matching(),
        })
    }

    /// Puts an encoded payload (already in the contract's type). An SHM
    /// buffer passes through as it is (§7.4).
    pub async fn put(&self, payload: impl Into<ZBytes>) -> Result<()> {
        self.put_with(payload, None::<ZBytes>).await
    }

    /// Puts an encoded payload with an attachment, encoded per the
    /// contract's `attachment` type and `attachment_encoding` (§2.3, §7.2).
    /// Refused, nothing published, when the member would be a new live one
    /// at the bound (§2.7), or, on a metric resource, when `telemetry.v1`
    /// forbids the sample ([`crate::telemetry`]).
    pub async fn put_with(
        &self,
        payload: impl Into<ZBytes>,
        attachment: Option<impl Into<ZBytes>>,
    ) -> Result<()> {
        let (payload, attachment, pending) = self
            .meter(payload.into(), attachment.map(Into::into))
            .await?;
        self.admit()?;
        match &self.inner {
            Inner::Plain(p) => p.put(payload).attachment(attachment).await,
            Inner::Advanced(p) => p.put(payload).attachment(attachment).await,
        }
        .map_err(zenoh)?;
        if let Some(p) = pending {
            p.commit();
        }
        Ok(())
    }

    /// Puts with the owner's stamp (state, S1).
    pub(crate) async fn put_stamped(
        &self,
        payload: ZBytes,
        attachment: Option<ZBytes>,
        ts: zenoh::time::Timestamp,
    ) -> Result<()> {
        match &self.inner {
            Inner::Plain(p) => p.put(payload).attachment(attachment).timestamp(ts).await,
            Inner::Advanced(p) => p.put(payload).attachment(attachment).timestamp(ts).await,
        }
        .map_err(zenoh)
    }

    /// Deletes with the owner's stamp (state, S1).
    pub(crate) async fn delete_stamped(&self, ts: zenoh::time::Timestamp) -> Result<()> {
        match &self.inner {
            Inner::Plain(p) => p.delete().timestamp(ts).await,
            Inner::Advanced(p) => p.delete().timestamp(ts).await,
        }
        .map_err(zenoh)
    }

    /// A value of this resource's JSON Schema type, in its wire encoding.
    pub(crate) fn encode<T: Serialize>(&self, value: &T) -> Result<Vec<u8>> {
        if !self.json_typed {
            return Err(Error::Contract(
                "a JSON Schema value for a resource whose type is not one".to_owned(),
            ));
        }
        encode_value(value, self.wire)
    }

    /// Puts a value of a JSON Schema type, encoded as the contract says:
    /// JSON by default, CBOR when `encoding = "cbor"` (§7.2).
    pub async fn put_value<T: Serialize>(&self, value: &T) -> Result<()> {
        if !self.json_typed {
            return Err(Error::Contract(
                "put_value encodes JSON Schema types; this resource's type is not one".to_owned(),
            ));
        }
        self.put(encode_value(value, self.wire)?).await
    }
}

/// A value of a JSON Schema type, in its wire encoding (§7.2).
pub(crate) fn encode_value<T: Serialize>(value: &T, wire: Option<WireEncoding>) -> Result<Vec<u8>> {
    crate::codec::encode_json(value, wire).map_err(Error::Contract)
}

/// One-shot puts on `…/events/<template>/<ulid>`, one key per occurrence
/// (§2.6), with the contract's QoS and `Encoding`.
///
/// Made by its service, it keeps the resource's budget (§2.7, #741): an
/// occurrence that would make its member a new live one while the live
/// members are at the bound, or that the member's rate does not allow, is
/// refused, [`Error::Budget`], and nothing is published.
pub struct EventWriter {
    session: zenoh::Session,
    prefix: String,
    encoding: Encoding,
    reliability: Reliability,
    congestion: CongestionControl,
    priority: Priority,
    express: bool,
    budget: Option<Arc<Budget>>,
}

impl EventWriter {
    pub(crate) fn new(
        session: &zenoh::Session,
        prefix: String,
        r: &Resource,
        values: &Bindings,
    ) -> Result<Self> {
        let d = data(r)?;
        if r.kind != Kind::Event {
            return Err(Error::Contract(format!("{} is not an event", r.template)));
        }
        Ok(Self {
            session: session.clone(),
            prefix,
            encoding: wire_encoding(&d.type_, d.encoding, values),
            reliability: qos::reliability(d.reliability),
            congestion: qos::congestion(d.congestion),
            priority: qos::priority(d.priority),
            express: d.express,
            budget: None,
        })
    }

    /// Keeps `budget`, its resource's in its service (§2.7).
    pub(crate) fn bounded(mut self, budget: Option<Arc<Budget>>) -> Self {
        self.budget = budget;
        self
    }

    /// Publishes one occurrence now, and returns its key.
    pub async fn put(&self, payload: impl Into<ZBytes>) -> Result<String> {
        self.put_at(payload, SystemTime::now()).await
    }

    /// Publishes one occurrence whose ULID carries `at`: for an owner that
    /// relays occurrences it observed earlier.
    pub async fn put_at(&self, payload: impl Into<ZBytes>, at: SystemTime) -> Result<String> {
        self.put_at_with(payload, None::<ZBytes>, at).await
    }

    /// [`EventWriter::put`] with an attachment, encoded per the contract's
    /// `attachment` type and `attachment_encoding` (§2.3, §7.2; #698).
    pub async fn put_with(
        &self,
        payload: impl Into<ZBytes>,
        attachment: Option<impl Into<ZBytes>>,
    ) -> Result<String> {
        self.put_at_with(payload, attachment, SystemTime::now())
            .await
    }

    /// [`EventWriter::put_at`] with an attachment (§2.3; #698). The budget
    /// counts the occurrence when it is published, not at `at` (§2.7).
    pub async fn put_at_with(
        &self,
        payload: impl Into<ZBytes>,
        attachment: Option<impl Into<ZBytes>>,
        at: SystemTime,
    ) -> Result<String> {
        if let Some(b) = &self.budget {
            b.admit(&self.prefix, Instant::now())?;
        }
        let key = format!("{}/{}", self.prefix, ulid(at));
        let attachment: Option<ZBytes> = attachment.map(Into::into);
        self.session
            .put(&key, payload)
            .attachment(attachment)
            .encoding(self.encoding.clone())
            .reliability(self.reliability)
            .congestion_control(self.congestion)
            .priority(self.priority)
            .express(self.express)
            .await
            .map_err(zenoh)?;
        Ok(key)
    }
}

const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// A lowercase ULID (§1.2): 48 bits of milliseconds since the epoch, then
/// 80 random bits, in Crockford's base32.
#[must_use]
pub fn ulid(at: SystemTime) -> String {
    let ms = at
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        & ((1 << 48) - 1);
    let random: u128 = rand::random::<u128>() & ((1 << 80) - 1);
    let v = (ms << 80) | random;
    (0..26)
        .map(|i| CROCKFORD[((v >> (125 - 5 * i)) & 31) as usize] as char)
        .collect()
}

/// The time a ULID chunk carries, if it is one (§1.2).
#[must_use]
pub fn ulid_time(chunk: &str) -> Option<SystemTime> {
    if chunk.len() != 26 {
        return None;
    }
    let mut v: u128 = 0;
    for b in chunk.bytes() {
        let d = CROCKFORD.iter().position(|&c| c == b)?;
        v = (v << 5) | d as u128;
    }
    let ms = (v >> 80) as u64;
    Some(UNIX_EPOCH + Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{ulid, ulid_time};

    #[test]
    fn a_ulid_is_a_lowercase_chunk_carrying_its_time() {
        let at = UNIX_EPOCH + Duration::from_millis(1_760_000_000_123);
        let u = ulid(at);
        assert_eq!(u.len(), 26);
        assert!(zenkey_model::grammar::parse(&format!("zk2/s/v/i.v1/events/e/{u}")).is_ok());
        assert_eq!(ulid_time(&u), Some(at));
        assert_ne!(ulid(at), ulid(at), "80 random bits");
        assert_eq!(ulid_time("not-a-ulid"), None);
    }
}
