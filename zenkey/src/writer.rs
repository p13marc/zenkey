//! Writing a resource's data (spec §2.4–§2.6, §7.2): the contract's QoS and
//! `Encoding` on every sample, advanced publication when the contract
//! declares `history`, and events as one-shot puts on fresh ULID keys.
//!
//! Every `stream`, `state`, `@state` and `events` sample carries core §7.5's
//! envelope under the owner's stamps (§4.3), its record after it, under the
//! `Encoding` `zenoh/bytes`; an `@stream` sample keeps its record alone,
//! under its predefined `Encoding`. State's own rules (tombstones, GET
//! answering, S1–S7) are [`crate::state`]'s, built on [`Writer`].

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use zenkey_model::authoring::{Encoding as WireEncoding, Kind};
use zenkey_model::contract::{Body, Contract, Data, Resource};
use zenkey_model::grammar::KindToken;
use zenkey_model::packed::Layout;
use zenkey_model::schema::TypeId;
use zenkey_model::template::Bindings;
use zenkey_model::wire::Tag;
use zenoh::bytes::{Encoding, ZBytes};
use zenoh::key_expr::OwnedKeyExpr;
use zenoh::pubsub::Publisher;
use zenoh::qos::{CongestionControl, Priority, Reliability};
use zenoh_ext::{AdvancedPublisher, AdvancedPublisherBuilderExt, CacheConfig, MissDetectionConfig};

use crate::budget::Budget;
use crate::call::BoxFuture;
use crate::error::{Error, Result, zenoh};
use crate::gate::{Gate, Slot};
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
        // A packed record always travels in §7.5's envelope (core §7.2,
        // §7.6), whose `Encoding` is `zenoh/bytes` ([`PackedType`]).
        TypeId::Packed { .. } => Encoding::ZENOH_BYTES,
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

impl Inner {
    async fn undeclare(self) -> Result<()> {
        match self {
            Self::Plain(p) => p.undeclare().await,
            Self::Advanced(p) => p.undeclare().await,
        }
        .map_err(zenoh)
    }
}

/// A write admitted by its writer's gate ([`Writer::serving`]), held until
/// it is published: the proof every stamped put and delete takes, so that
/// none reaches a governed interface's keys outside its gate
/// (`redundancy.v1` §2.4, §2.9).
pub(crate) struct Admitted<'a>(#[allow(dead_code)] Option<tokio::sync::RwLockReadGuard<'a, ()>>);

/// A member's publisher, declared at once, or only while its governed
/// interface serves ([`crate::gate`], `redundancy.v1` §2.4): a slot of its
/// gate. Puts hold it for reading, so that undeclaring it waits for the
/// puts in flight.
pub(crate) struct PubSlot {
    session: zenoh::Session,
    key: OwnedKeyExpr,
    encoding: Encoding,
    data: Data,
    inner: tokio::sync::RwLock<Option<Inner>>,
}

impl PubSlot {
    async fn declare_now(&self) -> Result<()> {
        let mut held = self.inner.write().await;
        if held.is_some() {
            return Ok(());
        }
        let d = &self.data;
        let key = self.key.clone();
        *held = Some(match &d.history {
            None => Inner::Plain(
                self.session
                    .declare_publisher(key)
                    .encoding(self.encoding.clone())
                    .reliability(qos::reliability(d.reliability))
                    .congestion_control(qos::congestion(d.congestion))
                    .priority(qos::priority(d.priority))
                    .express(d.express)
                    .await
                    .map_err(zenoh)?,
            ),
            Some(h) => {
                // Core §2.5: an owner's advanced publisher numbers its
                // samples, since its session runs without timestamping
                // (§8.2) and zenoh-ext refuses a cache whose samples carry
                // neither a number nor a timestamp; `miss_detection_ms`
                // adds the heartbeat.
                let miss = match h.miss_detection_ms {
                    Some(ms) => MissDetectionConfig::default().heartbeat(Duration::from_millis(ms)),
                    None => MissDetectionConfig::default(),
                };
                let b = self
                    .session
                    .declare_publisher(key)
                    .encoding(self.encoding.clone())
                    .reliability(qos::reliability(d.reliability))
                    .congestion_control(qos::congestion(d.congestion))
                    .priority(qos::priority(d.priority))
                    .express(d.express)
                    .cache(CacheConfig::default().max_samples(h.depth as usize))
                    .publisher_detection()
                    .sample_miss_detection(miss);
                Inner::Advanced(b.await.map_err(zenoh)?)
            }
        });
        Ok(())
    }
}

impl Slot for PubSlot {
    fn expose(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(self.declare_now())
    }

    fn unexpose(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let inner = self.inner.write().await.take();
            match inner {
                Some(i) => i.undeclare().await,
                None => Ok(()),
            }
        })
    }
}

/// A packed type (core §7.6), as each writer of its resource holds it: CH
/// carries its layout's tag, and every record is admitted by its layout
/// before anything is published, as a reader admits it.
#[derive(Debug, Clone)]
pub(crate) struct PackedType {
    name: String,
    tag: Tag,
    layout: Arc<Layout>,
    /// The deployment epoch its `time` fields count from, in seconds: the
    /// service's catalog's, else 0 (core §7.6, §8.6).
    epoch: i64,
}

impl PackedType {
    /// `r`'s packed type, read from `contract`'s artifact, its `time`
    /// fields counted from `epoch`; `None` for a type of another kind. A
    /// packed type whose layout does not read, or whose tag is not CH's
    /// four bits, is refused, [`Error::Contract`].
    pub(crate) fn of(contract: &Contract, r: &Resource, epoch: i64) -> Result<Option<Self>> {
        let d = data(r)?;
        let TypeId::Packed { name, .. } = &d.type_ else {
            return Ok(None);
        };
        let refused = |e: String| Error::Contract(format!("{}/{}: {e}", r.token, r.template));
        let layout = contract
            .packed_layout(&d.type_)
            .map_err(refused)?
            .ok_or_else(|| refused(format!("packed:{name}: no layout")))?;
        let tag = Tag::new(layout.tag())
            .ok_or_else(|| refused(format!("packed:{name}: tag {} past 15", layout.tag())))?;
        Ok(Some(Self {
            name: name.clone(),
            tag,
            layout: Arc::new(layout),
            epoch,
        }))
    }

    pub(crate) fn tag(&self) -> Tag {
        self.tag
    }

    pub(crate) fn epoch(&self) -> i64 {
        self.epoch
    }

    /// Its layout's admission of a record (core §7.6): refused,
    /// [`Error::Payload`], with nothing published.
    pub(crate) fn admit(&self, record: &[u8]) -> Result<()> {
        self.layout.check(self.layout.tag(), record).map_err(|r| {
            Error::Payload(format!(
                "packed:{}: refused by its layout, {r}: nothing published",
                self.name
            ))
        })
    }
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
///
/// On a governed interface (`redundancy.v1`, [`crate::redundancy`]), its
/// publisher is declared only while the instance serves the interface, and
/// a put while it stands by is refused, [`Error::Standby`], with nothing
/// published (§2.4).
///
/// **The in-band stamp** (core §4.3, §7.5). A `stream` or state sample's
/// payload is §7.5's envelope, its record after it, under the `Encoding`
/// `zenoh/bytes` (§7.2's exception): a stream key's seq is its own in the
/// run, kept by the service's [`crate::state::Minter`] across a publisher
/// declared again, and a state mutation's is the owner's. An `@stream`
/// sample carries no envelope (§4.3, "Explicit streams"): its record
/// alone, under its predefined `Encoding`.
///
/// **A Zenoh timestamp** goes only on a resource core §4.3's table lists:
/// a stream that declares `timing.lifespan_ms` (`timing.v1` §2.6,
/// [`crate::timing`]) is stamped from its service's clock, the stamp's id
/// the session's zid, which the descriptor states as `meta.zid`, so a
/// reader can age the sample against its owner's clock; a state with a
/// horizon above 0, a `desired.v1` desired resource and a `telemetry.v1`
/// state metric resource on every mutation. On a session whose
/// timestamping is enabled, every put is (core §8.2). Its period is the
/// application's to keep (§2.4): the writer publishes only what it is
/// given, and never a sample again on its own.
pub struct Writer {
    key: zenoh::key_expr::KeyExpr<'static>,
    owned: OwnedKeyExpr,
    slot: Arc<PubSlot>,
    gate: Option<Arc<Gate>>,
    /// Every sample's `Encoding`: `zenoh/bytes` under an envelope.
    encoding: Encoding,
    /// The record's, as the contract's type says (§7.2).
    record_encoding: Encoding,
    wire: Option<WireEncoding>,
    json_typed: bool,
    budget: Option<Arc<Budget>>,
    meter: Option<Meter>,
    /// Its packed type, when it has one (core §7.6): the tag CH carries,
    /// and the layout every record is admitted by.
    packed: Option<PackedType>,
    /// The resource's `timing.v1` values, when it is a timed stream.
    timing: Option<zenkey_model::timing::Declared>,
    /// The service's stamps (core §4.3).
    minter: Arc<crate::state::Minter>,
    /// The resource's kind token: a stream's seq is its key's, a state's the
    /// owner's, an `@stream` carries no envelope.
    token: KindToken,
    /// Whether each write carries a Zenoh timestamp: §4.3's table, or a
    /// session whose timestamping is enabled.
    timed: bool,
    /// Held from a stream sample's mint to its put, so that one writer's
    /// samples go out in their seqs' order.
    order: tokio::sync::Mutex<()>,
}

/// Whether `r` is a resource core §4.3's table lists for every mutation of
/// it: a state with a horizon above 0 whose contract does not use
/// `freshness.v2` (`v2`: its heartbeats confirm it instead), a `desired.v1`
/// desired resource, or a stream that declares a `timing.v1` lifespan. A
/// `telemetry.v1` state metric resource is the fourth, known once its
/// writer is metered ([`Writer::metered`]).
fn in_table(r: &Resource, v2: bool) -> bool {
    let state = matches!(r.token, KindToken::State | KindToken::ExplicitState);
    // Row 1 is freshness.v1's: under freshness.v2 a heartbeat confirms the
    // state, and its puts carry no Zenoh timestamp (#978's F3).
    let horizon = !v2
        && matches!(
            zenkey_model::freshness::horizon_of(r),
            zenkey_model::freshness::Horizon::Within(_)
        );
    let desired = matches!(
        zenkey_model::desired::declaration_of(r),
        zenkey_model::desired::Declaration::Declared(_)
    );
    let lifespan = zenkey_model::timing::declaration_of(r)
        .declared()
        .is_some_and(|t| t.lifespan.is_some());
    (state && (horizon || desired)) || lifespan
}

impl Writer {
    pub(crate) async fn declare(
        session: &zenoh::Session,
        minter: &Arc<crate::state::Minter>,
        key: OwnedKeyExpr,
        r: &Resource,
        values: &Bindings,
    ) -> Result<Self> {
        Self::declare_gated(session, minter, key, r, values, None).await
    }

    /// [`Writer::declare`] on a governed interface's `gate`: its publisher
    /// declared only while the gate is open. A frame resource's is
    /// refused: a mutation on its key has no reading (core §7.7), so its
    /// samples go through a [`crate::frame::FrameWriter`].
    pub(crate) async fn declare_gated(
        session: &zenoh::Session,
        minter: &Arc<crate::state::Minter>,
        key: OwnedKeyExpr,
        r: &Resource,
        values: &Bindings,
        gate: Option<Arc<Gate>>,
    ) -> Result<Self> {
        if data(r)?.frame.is_some() {
            return Err(Error::Contract(format!(
                "{} is a frame resource: write it with a FrameWriter, never a mutation \
                 (core §7.7)",
                r.template
            )));
        }
        Self::declare_any(session, minter, key, r, values, gate).await
    }

    /// A frame resource's publisher, which only its
    /// [`crate::frame::FrameWriter`] puts on.
    pub(crate) async fn declare_framed(
        session: &zenoh::Session,
        minter: &Arc<crate::state::Minter>,
        key: OwnedKeyExpr,
        r: &Resource,
        gate: Option<Arc<Gate>>,
    ) -> Result<Self> {
        Self::declare_any(session, minter, key, r, &Bindings::default(), gate).await
    }

    async fn declare_any(
        session: &zenoh::Session,
        minter: &Arc<crate::state::Minter>,
        key: OwnedKeyExpr,
        r: &Resource,
        values: &Bindings,
        gate: Option<Arc<Gate>>,
    ) -> Result<Self> {
        let d = data(r)?;
        if r.kind == Kind::Event {
            return Err(Error::Contract(format!(
                "{} is an event: write it with an EventWriter (one key per occurrence)",
                r.template
            )));
        }
        let record_encoding = wire_encoding(&d.type_, d.encoding, values);
        // Core §4.3, §7.2: an envelope's sample is `zenoh/bytes`; an
        // `@stream`'s keeps its predefined Encoding.
        let encoding = if r.token == KindToken::ExplicitStream {
            record_encoding.clone()
        } else {
            Encoding::ZENOH_BYTES
        };
        let slot = Arc::new(PubSlot {
            session: session.clone(),
            key: key.clone(),
            encoding: encoding.clone(),
            data: d.clone(),
            inner: tokio::sync::RwLock::new(None),
        });
        match &gate {
            None => slot.declare_now().await?,
            Some(g) => g.join(Arc::clone(&slot) as Arc<dyn Slot>).await?,
        }
        if r.token == KindToken::ExplicitStream {
            crate::shm::warn_if_memlock_low();
        }
        let timed = in_table(r, minter.covers(key.as_str())) || minter.session_timestamping();
        Ok(Self {
            owned: key.clone(),
            key: zenoh::key_expr::KeyExpr::from(key),
            slot,
            gate,
            encoding,
            record_encoding,
            wire: d.encoding,
            json_typed: matches!(d.type_, TypeId::JsonSchema { .. }),
            budget: None,
            meter: None,
            packed: None,
            timing: zenkey_model::timing::declaration_of(r).declared().copied(),
            minter: Arc::clone(minter),
            token: r.token,
            timed,
            order: tokio::sync::Mutex::new(()),
        })
    }

    /// The resource's `timing.v1` values (`timing.v1` §2.2), when it is a
    /// timed stream: its period, the cadence the application's loop keeps
    /// (§2.4), its deadline and its lifespan.
    #[must_use]
    pub fn timing(&self) -> Option<&zenkey_model::timing::Declared> {
        self.timing.as_ref()
    }

    /// Whether each put carries a Zenoh timestamp from the service's clock
    /// (core §4.3's table: a lifespan's stream, `timing.v1` §2.6, among
    /// them), or from its session's (§8.2).
    #[must_use]
    pub fn is_stamped(&self) -> bool {
        self.timed
    }

    /// Keeps `budget`, its resource's in its service (§2.7).
    pub(crate) fn bounded(mut self, budget: Option<Arc<Budget>>) -> Self {
        self.budget = budget;
        self
    }

    /// Holds its puts to `telemetry.v1` through `meter`, for a member of a
    /// metric resource ([`crate::telemetry`]). A state metric resource is
    /// on core §4.3's table: its rate's time base is its owner's stamp
    /// (`telemetry.v1` §2.11).
    pub(crate) fn metered(mut self, meter: Option<Meter>) -> Self {
        if meter.is_some() && matches!(self.token, KindToken::State | KindToken::ExplicitState) {
            self.timed = true;
        }
        self.meter = meter;
        self
    }

    /// Seals and admits its records as its packed type's (core §7.6).
    pub(crate) fn packed(mut self, packed: Option<PackedType>) -> Self {
        self.packed = packed;
        self
    }

    /// The tag CH carries on its records: its packed type's (core §7.6),
    /// else 0.
    #[must_use]
    pub fn tag(&self) -> Tag {
        self.packed.as_ref().map_or(Tag::NONE, PackedType::tag)
    }

    /// The deployment epoch its packed records' `time` fields count from,
    /// in seconds: its service's catalog's, else 0 (core §7.6, §8.6).
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.packed.as_ref().map_or(
            zenkey_model::catalog::EPOCH_WITHOUT_CATALOG,
            PackedType::epoch,
        )
    }

    /// Admits a record before anything is minted or published: a packed
    /// type's by its layout, as its reader admits it (core §7.6), any other
    /// as it is.
    pub(crate) fn admit_record(&self, record: &[u8]) -> Result<()> {
        self.packed.as_ref().map_or(Ok(()), |p| p.admit(record))
    }

    /// The governed interface's gate, when its interface is one
    /// (`redundancy.v1`).
    pub(crate) fn gate(&self) -> Option<&Arc<Gate>> {
        self.gate.as_ref()
    }

    /// A write admitted by the governed interface's gate, held until it is
    /// published: always, on an interface that is not governed; while it
    /// serves, on one that is (`redundancy.v1` §2.4). Every publication of
    /// the writer takes one: [`Writer::put_with`] itself, and the stamped
    /// puts and deletes ([`Writer::put_stamped`], [`Writer::delete_stamped`])
    /// from their caller, which needs it to call them. A step-down's
    /// [`Gate::close_writes`] waits for every one held.
    pub(crate) async fn serving(&self) -> Result<Admitted<'_>> {
        match &self.gate {
            Some(g) => Ok(Admitted(Some(g.write().await?))),
            None => Ok(Admitted(None)),
        }
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

    /// The `Encoding` every sample carries: `zenoh/bytes` under an
    /// envelope (core §4.3, §7.2), an `@stream`'s predefined one.
    #[must_use]
    pub fn encoding(&self) -> &Encoding {
        &self.encoding
    }

    /// The record's `Encoding`, as the contract's type says (§7.2): the
    /// sample's own on an `@stream`, the envelope's record's otherwise.
    #[must_use]
    pub fn record_encoding(&self) -> &Encoding {
        &self.record_encoding
    }

    /// Whether its samples carry §7.5's envelope: every kind but `@stream`
    /// (core §4.3).
    #[must_use]
    pub fn is_enveloped(&self) -> bool {
        self.token != KindToken::ExplicitStream
    }

    /// The stamps of its next write: a stream key's seq, or the owner's
    /// state seq, which the clock guard holds (core §4.3).
    pub(crate) fn mint(&self) -> Result<crate::state::Stamped> {
        match self.token {
            KindToken::State | KindToken::ExplicitState => self.minter.mint_state(self.timed),
            _ => Ok(self.minter.mint_stream(&self.owned, self.timed)),
        }
    }

    /// The member's key expression.
    #[must_use]
    pub fn key_expr(&self) -> &zenoh::key_expr::KeyExpr<'static> {
        &self.key
    }

    /// Whether its publisher is declared now: always, on an interface that
    /// is not governed; while the instance serves, on one that is.
    pub async fn is_declared(&self) -> bool {
        self.slot.inner.read().await.is_some()
    }

    /// Whether a subscriber matches (zenoh's matching status); `false`
    /// while its governed interface stands by, with no publisher declared.
    pub async fn matching(&self) -> Result<bool> {
        Ok(match &*self.slot.inner.read().await {
            None => false,
            Some(Inner::Plain(p)) => p.matching_status().await.map_err(zenoh)?.matching(),
            Some(Inner::Advanced(p)) => p.matching_status().await.map_err(zenoh)?.matching(),
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
    /// forbids the sample ([`crate::telemetry`]), or on a governed
    /// interface the instance stands by on ([`Error::Standby`]).
    pub async fn put_with(
        &self,
        payload: impl Into<ZBytes>,
        attachment: Option<impl Into<ZBytes>>,
    ) -> Result<()> {
        let _write = self.serving().await?;
        let (payload, attachment, pending) = self
            .meter(payload.into(), attachment.map(Into::into))
            .await?;
        // Core §7.6 (#979): a packed record its layout refuses is refused,
        // nothing published.
        self.admit_record(&payload.to_bytes())?;
        let held = self.slot.inner.read().await;
        let Some(inner) = &*held else {
            return Err(self.standby());
        };
        let order = self.order.lock().await;
        self.admit()?;
        // Core §4.3: the envelope under the next stamps, and a Zenoh
        // timestamp only on §4.3's table (timing.v1 §2.6: a lifespan's
        // sample carries its owner's stamp). An `@stream` keeps its wire.
        // Core §7.6: CH carries a packed record's tag.
        let (payload, ts) = if self.is_enveloped() {
            let stamped = self.mint()?;
            let record = payload.to_bytes();
            (
                ZBytes::from(stamped.seal_tagged(self.tag(), &record)),
                stamped.time,
            )
        } else {
            let ts = self.timed.then(|| self.minter.time());
            (payload, ts)
        };
        match inner {
            Inner::Plain(p) => p.put(payload).attachment(attachment).timestamp(ts).await,
            Inner::Advanced(p) => p.put(payload).attachment(attachment).timestamp(ts).await,
        }
        .map_err(zenoh)?;
        drop(order);
        drop(held);
        if let Some(p) = pending {
            p.commit();
        }
        Ok(())
    }

    fn standby(&self) -> Error {
        match &self.gate {
            Some(g) => Error::Standby(g.iface().clone()),
            None => Error::Zenoh(format!("{}: no publisher declared", self.key)),
        }
    }

    /// Puts `payload`, already sealed under `stamped` (state, S1), with its
    /// Zenoh timestamp where it has one, under the write `admitted` by
    /// [`Writer::serving`], held from the stamps' minting to here.
    pub(crate) async fn put_stamped(
        &self,
        _admitted: &Admitted<'_>,
        payload: ZBytes,
        attachment: Option<ZBytes>,
        stamped: crate::state::Stamped,
    ) -> Result<()> {
        let held = self.slot.inner.read().await;
        match &*held {
            None => Err(self.standby()),
            Some(Inner::Plain(p)) => p
                .put(payload)
                .attachment(attachment)
                .timestamp(stamped.time)
                .await
                .map_err(zenoh),
            Some(Inner::Advanced(p)) => p
                .put(payload)
                .attachment(attachment)
                .timestamp(stamped.time)
                .await
                .map_err(zenoh),
        }
    }

    /// Deletes under `stamped` (state, S1): its attachment the delete's
    /// envelope, its Zenoh timestamp where it has one, under the write
    /// `admitted` as [`Writer::put_stamped`].
    pub(crate) async fn delete_stamped(
        &self,
        _admitted: &Admitted<'_>,
        stamped: crate::state::Stamped,
    ) -> Result<()> {
        let held = self.slot.inner.read().await;
        let attachment = ZBytes::from(stamped.delete_attachment());
        match &*held {
            None => Err(self.standby()),
            Some(Inner::Plain(p)) => p
                .delete()
                .attachment(attachment)
                .timestamp(stamped.time)
                .await
                .map_err(zenoh),
            Some(Inner::Advanced(p)) => p
                .delete()
                .attachment(attachment)
                .timestamp(stamped.time)
                .await
                .map_err(zenoh),
        }
    }

    /// Puts a stream sample sealed under its key's next seq and given the
    /// Zenoh timestamp `time`, where the owner sets one (`health.v1` §2.5's
    /// fault: core §4.3's table), under the write `admitted`.
    pub(crate) async fn put_at(
        &self,
        _admitted: &Admitted<'_>,
        payload: &[u8],
        time: zenoh::time::Timestamp,
    ) -> Result<()> {
        let held = self.slot.inner.read().await;
        let Some(inner) = &*held else {
            return Err(self.standby());
        };
        let order = self.order.lock().await;
        self.admit()?;
        let payload = if self.is_enveloped() {
            let mut stamped = self.minter.mint_stream(&self.owned, false);
            stamped.time = Some(time);
            ZBytes::from(stamped.seal_tagged(self.tag(), payload))
        } else {
            ZBytes::from(payload.to_vec())
        };
        let r = match inner {
            Inner::Plain(p) => p.put(payload).timestamp(time).await,
            Inner::Advanced(p) => p.put(payload).timestamp(time).await,
        }
        .map_err(zenoh);
        drop(order);
        r
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
    /// The record's `Encoding` (§7.2); every occurrence is `zenoh/bytes`,
    /// under its envelope (core §4.3).
    record_encoding: Encoding,
    reliability: Reliability,
    congestion: CongestionControl,
    priority: Priority,
    express: bool,
    budget: Option<Arc<Budget>>,
    gate: Option<Arc<Gate>>,
    /// The service's stamps: an occurrence takes the owner's next state and
    /// event seq (core §4.3).
    minter: Arc<crate::state::Minter>,
    /// Its packed type, when it has one (core §7.6).
    packed: Option<PackedType>,
}

impl EventWriter {
    pub(crate) fn new(
        session: &zenoh::Session,
        minter: &Arc<crate::state::Minter>,
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
            record_encoding: wire_encoding(&d.type_, d.encoding, values),
            reliability: qos::reliability(d.reliability),
            congestion: qos::congestion(d.congestion),
            priority: qos::priority(d.priority),
            express: d.express,
            budget: None,
            gate: None,
            minter: Arc::clone(minter),
            packed: None,
        })
    }

    /// Seals and admits its occurrences as its packed type's (core §7.6).
    pub(crate) fn packed(mut self, packed: Option<PackedType>) -> Self {
        self.packed = packed;
        self
    }

    /// The tag CH carries on its occurrences: its packed type's (core
    /// §7.6), else 0.
    #[must_use]
    pub fn tag(&self) -> Tag {
        self.packed.as_ref().map_or(Tag::NONE, PackedType::tag)
    }

    /// The deployment epoch its packed records' `time` fields count from,
    /// in seconds: its service's catalog's, else 0 (core §7.6, §8.6).
    #[must_use]
    pub fn deployment_epoch(&self) -> i64 {
        self.packed.as_ref().map_or(
            zenkey_model::catalog::EPOCH_WITHOUT_CATALOG,
            PackedType::epoch,
        )
    }

    /// The record's `Encoding`, as the contract's type says (§7.2). Every
    /// occurrence carries `zenoh/bytes`, under its envelope (core §4.3).
    #[must_use]
    pub fn record_encoding(&self) -> &Encoding {
        &self.record_encoding
    }

    /// Keeps `budget`, its resource's in its service (§2.7).
    pub(crate) fn bounded(mut self, budget: Option<Arc<Budget>>) -> Self {
        self.budget = budget;
        self
    }

    /// Writes only while its governed interface serves (`redundancy.v1`
    /// §2.4): refused, [`Error::Standby`], while it stands by.
    pub(crate) fn gated(mut self, gate: Option<Arc<Gate>>) -> Self {
        self.gate = gate;
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
        let _write = match &self.gate {
            Some(g) => Some(g.write().await?),
            None => None,
        };
        let payload: ZBytes = payload.into();
        // Core §7.6 (#979): a packed record its layout refuses is refused,
        // nothing published.
        if let Some(p) = &self.packed {
            p.admit(&payload.to_bytes())?;
        }
        if let Some(b) = &self.budget {
            b.admit(&self.prefix, Instant::now())?;
        }
        let key = format!("{}/{}", self.prefix, ulid(at));
        let attachment: Option<ZBytes> = attachment.map(Into::into);
        // Core §4.3: the occurrence under the owner's next seq, sealed in
        // §7.5's envelope; a Zenoh timestamp only where its session stamps
        // (§8.2).
        let stamped = self.minter.mint_event(false);
        self.session
            .put(&key, stamped.seal_tagged(self.tag(), &payload.to_bytes()))
            .attachment(attachment)
            .encoding(Encoding::ZENOH_BYTES)
            .timestamp(stamped.time)
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
