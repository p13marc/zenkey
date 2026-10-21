//! `telemetry.v1` in the runtime: an owner held to what a metric is
//! (`spec/profiles/telemetry/v1.md`, #819, TB).
//!
//! The owner's half of the profile. The declaration, a sample's
//! conformance and a pair's continuity are `zenkey-model`'s
//! ([`zenkey_model::telemetry`]), and so are the types an owner puts
//! ([`Point`], [`Value`], [`Histogram`], from
//! [`zenkey_model::telemetry::wire`]); this module makes the runtime's
//! writers refuse what a reader would read as a fault, and write the epoch
//! a reader reads, the way [`crate::budget`] holds an owner to core §2.7.
//!
//! ```text
//! Writer::put / StateWriter::put on a metric resource (its declaration `declared`)
//!   1. decode the bytes as the resource carries them (JSON or CBOR)   undecodable ─▶ refused
//!   2. a counter's or histogram's Point without `epoch`: the epoch filled in
//!      a bare one whose attachment is telemetry.v1.Epoch, given none: the epoch attached
//!   3. zenkey_model::telemetry::check                                 does_not ─▶ refused
//!   4. an in-band epoch other than the owner's                        ─▶ refused
//!   5. below the last value this writer put in this epoch             decreased ─▶ refused
//!   6. the budget (core §2.7), then the put; the value kept as the writer's last
//! ```
//!
//! | Rule | Here |
//! |---|---|
//! | §2.1, §2.2 a metric resource: a stream or a state whose declaration is valid | each writer of a resource whose [`zenkey_model::telemetry::declaration_of`] is `declared`; one that is `invalid` is logged as a warning when the service implements it, and its writers check nothing (a reader reads nothing of it) |
//! | §2.4 a raw type carries `binary`, bytes, always | a raw resource's writers check nothing: no byte string can fail |
//! | §2.6 a sample that does not conform | refused, [`Reason::DoesNot`] with the problems in §2.6's order; nothing published |
//! | §2.6 a sample the core could not decode | refused, [`Reason::Undecodable`] |
//! | §2.7 within one epoch, no counter below one put before for that member | refused, [`Reason::Decreased`], against the last counter this writer put in the epoch, judged by [`zenkey_model::telemetry::pair`] |
//! | §2.8 within one epoch, no `count` or bucket's count below one put before | refused, [`Reason::Decreased`], as a counter, the last histogram this writer put |
//! | §2.9 the in-band epoch: the instance id, or a member epoch on a template that declares `epoch` | filled when left out ([`Point::epoch`], the Epoch attachment); any other refused, [`Reason::Epoch`] |
//! | §2.9 a new epoch whenever a counter restarts (core §1.5) | [`crate::Service::new_epoch`] moves the epoch every writer writes, and a member's token cycled ([`crate::Service::cycle_member`]) moves that member's |
//! | §2.8 the shape: counts per bucket, upper-inclusive, the overflow last, `count` their sum | [`Histogram::new`] over the declared bounds, [`Histogram::observe`] |
//! | §2.12 the Point's `timestamp` is the measurement time | the application's: [`point`] stamps one with this host's clock now, for a value taken now |
//! | §2.14 a counter that crosses a face SHOULD carry its epoch | every counter and histogram Point carries it; a bare one does where its contract declares the Epoch attachment |
//!
//! **The epoch, filled.** A Point whose kind is `counter` or `histogram`
//! (the declared kind, else its tag) and which carries no `epoch` gets the
//! writer's epoch as its `epoch`: the JSON object gains the member, as
//! `"epoch":"<id>",` written after its `{`, and the CBOR map gains an entry,
//! last; every other byte is the application's. A Point of another kind is
//! left as it is: its epoch would be a cost on a constrained face (§2.14)
//! that no reader reads. A bare counter or histogram whose contract types
//! its attachment `telemetry.v1.Epoch` and which is put with none gets the
//! epoch as its attachment, encoded as the attachment is (18 bytes as
//! JSON). An epoch the application writes, in either place, must be the
//! writer's.
//!
//! **The writer's epoch** is the service's current instance id. For a
//! member of the template that declares `epoch` (core §2.2), it is that
//! member's epoch, the one its member token carries (core §8.1): a write
//! on a member whose token is not declared yet gives it its epoch, which
//! the token takes when [`crate::Service::declare_member`] declares it,
//! since a member exists from its first value (core §8.1).
//!
//! **The last value** is the writer's: each [`crate::writer::Writer`] and
//! [`crate::state::StateWriter`] keeps the last counter and the last
//! histogram it put, with the epoch they were put in. It is replaced after
//! each put that is published, and forgotten when the epoch moves: a value
//! put in a new epoch is compared with nothing. A delete forgets nothing:
//! the epoch does not move, and a reader pairs the value after a delete
//! with the one before it (§2.10), so a member whose counters restart
//! takes a new epoch (§2.15). A put holds the writer's last value from its
//! check until it is published, so two puts on one writer reach the bus in
//! the order they were checked. A second writer of the same member keeps
//! its own, and a raw publisher ([`crate::ServiceBuilder::declare_publisher`],
//! [`crate::writer::Writer::publisher`], the session) checks nothing:
//! keeping the text with either is its caller's.
//!
//! **No off switch.** A writer of a metric resource always checks: an
//! owner the text does not allow is what a test writes from its session,
//! as the scenarios' raw steps do, and a switch on the writer would be one
//! for every application. The typed writers of generated code
//! ([`crate::typed`]) put through the same writers, so through the same
//! checks.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

pub use zenkey_model::telemetry::wire::{Histogram, Point, Value};
pub use zenkey_model::telemetry::{Declared, Measure, MetricKind, Problem};

#[cfg(feature = "zenoh")]
pub(crate) use meter::{Epochs, Meter, Metric, Pending};

/// This host's clock now, in Unix epoch milliseconds: a Point's
/// `timestamp` for a value taken now (§2.12).
#[must_use]
pub fn now_ms() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// A Point of `value`, taken now ([`now_ms`]).
#[must_use]
pub fn point(value: impl Into<Value>) -> Point {
    Point::new(now_ms(), value)
}

/// Why a writer refused a metric sample (§2.6–§2.9): nothing was published.
#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    /// The sample does not conform (§2.6): the problems, in its order.
    DoesNot(Vec<Problem>),
    /// The bytes are not the resource's encoding: no data item to read
    /// (§2.6), what the decoder said.
    Undecodable(String),
    /// An in-band epoch (the Point's `epoch`, or the Epoch attachment) that
    /// is not the writer's (§2.9).
    Epoch { given: String, current: String },
    /// A counter, or a histogram's `count` or a bucket's count, below the
    /// last one this writer put in the same epoch (§2.7, §2.8).
    Decreased {
        epoch: String,
        before: Box<Measure>,
        after: Box<Measure>,
    },
}

impl Reason {
    /// The problems, for a sample that does not conform.
    #[must_use]
    pub fn problems(&self) -> &[Problem] {
        match self {
            Self::DoesNot(p) => p,
            _ => &[],
        }
    }
}

fn measure_words(m: &Measure) -> String {
    match m {
        Measure::Counter(n) => n.to_string(),
        Measure::Histogram(h) => format!("count {} counts {:?}", h.count, h.counts),
        Measure::Gauge(x) => x.to_string(),
        Measure::Text(s) => format!("{s:?}"),
        Measure::Boolean(b) => b.to_string(),
        Measure::Binary(b) => format!("{} bytes", b.len()),
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DoesNot(problems) => {
                f.write_str("the sample does not conform (§2.6): ")?;
                for (i, p) in problems.iter().enumerate() {
                    if i > 0 {
                        f.write_str("; ")?;
                    }
                    write!(f, "{p}, {}", p.says())?;
                }
                Ok(())
            }
            Self::Undecodable(why) => write!(f, "the payload does not decode (§2.6): {why}"),
            Self::Epoch { given, current } => write!(
                f,
                "its in-band epoch {given:?} is not this owner's, {current} (§2.9)"
            ),
            Self::Decreased {
                epoch,
                before,
                after,
            } => {
                let (what, rule) = match **after {
                    Measure::Histogram(_) => ("a histogram's count went down", "§2.8"),
                    _ => ("a counter went down", "§2.7"),
                };
                write!(
                    f,
                    "{what} within epoch {epoch}: {} after {} ({rule})",
                    measure_words(after),
                    measure_words(before)
                )
            }
        }
    }
}

#[cfg(feature = "zenoh")]
mod meter {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, RwLock};

    use zenkey_model::authoring::Encoding as WireEncoding;
    use zenkey_model::contract::{Body, Contract, Resource};
    use zenkey_model::grammar::{IfaceId, InstanceId};
    use zenkey_model::schema::TypeId;
    use zenkey_model::slug::chunk_slug;
    use zenkey_model::telemetry::{
        Carrier, Conformance, Continuity, Datum, Declaration, Declared, EpochIn, Measure,
        MetricKind, Payload, Sampled, check, declaration_of, pair,
    };
    use zenkey_model::template::Bindings;
    use zenoh::bytes::ZBytes;

    use super::Reason;

    /// The epochs a service's writers write (§2.9): its instance id, and
    /// each member's on the template that declares `epoch` (core §8.1).
    /// Shared by the service and every metric writer it made.
    #[derive(Debug)]
    pub(crate) struct Epochs {
        instance: RwLock<InstanceId>,
        /// Each member's epoch, by interface and slugged member value: its
        /// token's, or one a write gave it before its token.
        members: Mutex<HashMap<(IfaceId, String), InstanceId>>,
    }

    fn mint() -> InstanceId {
        InstanceId::from_u64(rand::random())
    }

    impl Epochs {
        pub(crate) fn new(instance: InstanceId) -> Arc<Self> {
            Arc::new(Self {
                instance: RwLock::new(instance),
                members: Mutex::new(HashMap::new()),
            })
        }

        pub(crate) fn instance(&self) -> InstanceId {
            self.instance.read().expect("not poisoned").clone()
        }

        /// The service's instance id moved (core §1.5).
        pub(crate) fn set_instance(&self, id: InstanceId) {
            *self.instance.write().expect("not poisoned") = id;
        }

        /// The member's epoch: its token's, or, before its token, one given
        /// now and kept for it.
        fn member(&self, iface: &IfaceId, member: &str) -> InstanceId {
            self.members
                .lock()
                .expect("not poisoned")
                .entry((iface.clone(), member.to_owned()))
                .or_insert_with(mint)
                .clone()
        }

        /// The epoch a member's first token takes: the one a write gave it,
        /// else a new one.
        pub(crate) fn first_token(&self, iface: &IfaceId, member: &str) -> InstanceId {
            self.member(iface, member)
        }

        /// A member's token cycled, or declared again: a new epoch.
        pub(crate) fn cycled(&self, iface: &IfaceId, member: &str) -> InstanceId {
            let epoch = mint();
            self.members
                .lock()
                .expect("not poisoned")
                .insert((iface.clone(), member.to_owned()), epoch.clone());
            epoch
        }

        /// A member's token withdrawn: its next write gives it a new epoch.
        pub(crate) fn removed(&self, iface: &IfaceId, member: &str) {
            self.members
                .lock()
                .expect("not poisoned")
                .remove(&(iface.clone(), member.to_owned()));
        }
    }

    /// One metric resource of an implemented interface: its declaration,
    /// and how its samples are carried.
    #[derive(Debug)]
    pub(crate) struct Metric {
        /// Always `Declared`.
        declaration: Declaration,
        /// The payload's encoding: JSON unless CBOR.
        wire: Option<WireEncoding>,
        /// The attachment's, when it is the Epoch.
        attachment_wire: Option<WireEncoding>,
        /// The template's `epoch` parameter, if it declares one.
        epoch_param: Option<String>,
    }

    impl Metric {
        /// The metric of `r`, a resource of `c`: `None` when it is not one
        /// a writer checks (§2.1, §2.4): not `declared`, or raw. An
        /// `invalid` declaration is logged.
        pub(crate) fn of(c: &Contract, r: &Resource) -> Option<Self> {
            let Body::Data(d) = &r.body else {
                return None;
            };
            match declaration_of(c, r) {
                Declaration::Invalid(why) => {
                    let why: Vec<&str> = why.iter().map(|w| w.as_str()).collect();
                    tracing::warn!(
                        interface = %c.iface,
                        resource = %crate::implementation::resource_name(r),
                        reasons = %why.join(", "),
                        "telemetry.v1: this resource's declaration is invalid, so no reader \
                         reads its members, and its writers check nothing (§2.2)"
                    );
                    None
                }
                Declaration::Declared(_) if matches!(d.type_, TypeId::Raw { .. }) => None,
                decl @ Declaration::Declared(_) => Some(Self {
                    declaration: decl,
                    wire: d.encoding,
                    attachment_wire: d.attachment_encoding,
                    epoch_param: r.epoch.clone(),
                }),
                Declaration::Ignored | Declaration::NotThisProfiles => None,
            }
        }

        fn declared(&self) -> &Declared {
            self.declaration
                .declared()
                .expect("a metric's declaration is declared")
        }

        fn decode(wire: Option<WireEncoding>, bytes: &[u8]) -> Result<Datum, Reason> {
            match wire {
                Some(WireEncoding::Cbor) => Datum::from_cbor_bytes(bytes)
                    .ok_or_else(|| Reason::Undecodable("not one CBOR data item".to_owned())),
                _ => Datum::from_json_bytes(bytes)
                    .ok_or_else(|| Reason::Undecodable("not one JSON text".to_owned())),
            }
        }

        /// `epoch` encoded as the Epoch attachment is (§2.4).
        fn epoch_attachment(&self, epoch: &str) -> Vec<u8> {
            match self.attachment_wire {
                Some(WireEncoding::Cbor) => {
                    let mut out = Vec::new();
                    ciborium::into_writer(&epoch, &mut out).expect("a string encodes");
                    out
                }
                _ => serde_json::to_vec(epoch).expect("a string encodes"),
            }
        }

        /// The Point `bytes` with `"epoch": epoch` added, every other byte
        /// kept: written after a JSON object's `{`, appended to a CBOR map.
        fn with_epoch(&self, bytes: &[u8], epoch: &str) -> Result<Vec<u8>, Reason> {
            let refuse = || Reason::Undecodable("the Point could not take its epoch".to_owned());
            match self.wire {
                Some(WireEncoding::Cbor) => {
                    let mut v: ciborium::Value =
                        ciborium::from_reader(bytes).map_err(|_| refuse())?;
                    let ciborium::Value::Map(m) = &mut v else {
                        return Err(refuse());
                    };
                    m.push((
                        ciborium::Value::Text("epoch".to_owned()),
                        ciborium::Value::Text(epoch.to_owned()),
                    ));
                    let mut out = Vec::new();
                    ciborium::into_writer(&v, &mut out).map_err(|_| refuse())?;
                    Ok(out)
                }
                _ => {
                    let ws = |b: &u8| matches!(b, b' ' | b'\t' | b'\n' | b'\r');
                    let open = bytes.iter().position(|b| !ws(b)).ok_or_else(refuse)?;
                    if bytes[open] != b'{' {
                        return Err(refuse());
                    }
                    let next = bytes[open + 1..]
                        .iter()
                        .position(|b| !ws(b))
                        .ok_or_else(refuse)?;
                    let empty = bytes[open + 1 + next] == b'}';
                    let member = format!(
                        "\"epoch\":{}{}",
                        serde_json::to_string(epoch).map_err(|_| refuse())?,
                        if empty { "" } else { "," }
                    );
                    let mut out = Vec::with_capacity(bytes.len() + member.len());
                    out.extend_from_slice(&bytes[..=open]);
                    out.extend_from_slice(member.as_bytes());
                    out.extend_from_slice(&bytes[open + 1..]);
                    Ok(out)
                }
            }
        }
    }

    /// Whether a Point's datum is one the writer gives its epoch: a counter
    /// or a histogram (declared, else its tag) without an `epoch` member.
    fn takes_epoch(d: &Declared, datum: &Datum) -> bool {
        let Datum::Map(m) = datum else {
            return false;
        };
        if m.contains_key("epoch") {
            return false;
        }
        let tag = || match m.get("value") {
            Some(Datum::Map(v)) => match v.get("type") {
                Some(Datum::Text(t)) => MetricKind::parse(t),
                _ => None,
            },
            _ => None,
        };
        d.kind.or_else(tag).is_some_and(MetricKind::is_cumulative)
    }

    /// What one writer last put (§2.7, §2.8): the epoch, and its last
    /// counter and last histogram in it.
    #[derive(Debug, Default)]
    struct Last {
        epoch: Option<String>,
        counter: Option<Measure>,
        histogram: Option<Measure>,
    }

    /// One writer's hold on its member's metric: the checks of the module
    /// doc, and its last value.
    #[derive(Debug)]
    pub(crate) struct Meter {
        metric: Arc<Metric>,
        epochs: Arc<Epochs>,
        /// The member, for a member of the template that declares `epoch`.
        member: Option<(IfaceId, String)>,
        last: tokio::sync::Mutex<Last>,
    }

    /// A checked sample, until it is published: the writer's last value is
    /// held, and becomes this one on [`Pending::commit`].
    pub(crate) struct Pending<'a> {
        last: tokio::sync::MutexGuard<'a, Last>,
        epoch: String,
        measure: Option<Measure>,
    }

    impl Pending<'_> {
        /// The sample was published: it is the writer's last value now.
        pub(crate) fn commit(mut self) {
            if self.last.epoch.as_deref() != Some(self.epoch.as_str()) {
                *self.last = Last {
                    epoch: Some(self.epoch),
                    ..Last::default()
                };
            }
            match self.measure {
                Some(m @ Measure::Counter(_)) => self.last.counter = Some(m),
                Some(m @ Measure::Histogram(_)) => self.last.histogram = Some(m),
                _ => {}
            }
        }
    }

    impl Meter {
        /// The meter of a writer of `metric` on the member `values` binds.
        pub(crate) fn new(
            metric: Arc<Metric>,
            epochs: Arc<Epochs>,
            iface: &IfaceId,
            values: &Bindings,
        ) -> Self {
            let member = metric
                .epoch_param
                .as_ref()
                .and_then(|p| values.get(p))
                .and_then(|v| v.first())
                .map(|v| (iface.clone(), chunk_slug(v)));
            Self {
                metric,
                epochs,
                member,
                last: tokio::sync::Mutex::new(Last::default()),
            }
        }

        /// The resource's declaration.
        pub(crate) fn declared(&self) -> &Declared {
            self.metric.declared()
        }

        /// The epoch this writer writes now (§2.9).
        pub(crate) fn epoch(&self) -> InstanceId {
            match &self.member {
                Some((iface, m)) => self.epochs.member(iface, m),
                None => self.epochs.instance(),
            }
        }

        /// Checks one sample (the module doc's steps 1 to 5), and returns
        /// what to publish: the payload and attachment, the epoch written
        /// in where the application left it out. Refused, with nothing
        /// changed, as [`Reason`] says.
        pub(crate) async fn prepare(
            &self,
            payload: ZBytes,
            attachment: Option<ZBytes>,
        ) -> Result<(ZBytes, Option<ZBytes>, Pending<'_>), Reason> {
            let last = self.last.lock().await;
            let epoch = self.epoch().to_string();
            let d = self.metric.declared();
            let bytes = payload.to_bytes();
            let mut datum = Metric::decode(self.metric.wire, &bytes)?;
            let mut payload_out = None;
            if d.carrier == Carrier::Point && takes_epoch(d, &datum) {
                payload_out = Some(self.metric.with_epoch(&bytes, &epoch)?);
                if let Datum::Map(m) = &mut datum {
                    m.insert("epoch".to_owned(), Datum::Text(epoch.clone()));
                }
            }
            drop(bytes);
            let payload = payload_out.map_or(payload, ZBytes::from);
            let (attachment, epoch_datum) = match (d.epoch, attachment) {
                (Some(EpochIn::Attachment), None) => (
                    Some(ZBytes::from(self.metric.epoch_attachment(&epoch))),
                    Some(Datum::Text(epoch.clone())),
                ),
                (Some(EpochIn::Attachment), Some(a)) => {
                    let datum = Metric::decode(self.metric.attachment_wire, &a.to_bytes())
                        .map_err(|_| {
                            Reason::Undecodable(
                                "the attachment is not a telemetry.v1.Epoch's encoding".to_owned(),
                            )
                        })?;
                    (Some(a), Some(datum))
                }
                (_, a) => (a, None),
            };
            let checked = check(
                &self.metric.declaration,
                &Payload::Decoded {
                    value: &datum,
                    attachment: epoch_datum.as_ref(),
                },
            );
            if let Conformance::DoesNot(problems) = checked.conformance {
                return Err(Reason::DoesNot(problems));
            }
            let reading = checked.reading;
            if let Some(given) = reading.as_ref().and_then(|r| r.epoch.as_ref())
                && *given != epoch
            {
                return Err(Reason::Epoch {
                    given: given.clone(),
                    current: epoch,
                });
            }
            let measure = reading.map(|r| r.value);
            if let Some(after) = &measure
                && last.epoch.as_deref() == Some(epoch.as_str())
            {
                let before = match after {
                    Measure::Counter(_) => last.counter.as_ref(),
                    Measure::Histogram(_) => last.histogram.as_ref(),
                    _ => None,
                };
                if let Some(before) = before {
                    let at = |value| Sampled {
                        value,
                        epoch: Some(epoch.as_str()),
                        timestamp_ms: None,
                        stamp_ns: None,
                        arrival_ns: 0,
                    };
                    if pair(&at(before), &at(after), None, None).continuity
                        == Continuity::Decreased
                    {
                        return Err(Reason::Decreased {
                            epoch,
                            before: Box::new(before.clone()),
                            after: Box::new(after.clone()),
                        });
                    }
                }
            }
            Ok((
                payload,
                attachment,
                Pending {
                    last,
                    epoch,
                    measure,
                },
            ))
        }
    }
}
