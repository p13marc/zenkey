//! The write facade (issue #36): the only two ways an explorer writes to the
//! bus — a declared publication, and a disciplined RPC call.
//!
//! Reading stayed the engine's whole job until now; both frontends need the
//! same two write paths (`zenctl pub` / `service call`, the zengui
//! publish/call pane), and the discipline they must share is exactly the kind
//! that fails silently when duplicated:
//!
//! - **P7**: telemetry/state publishers are *declared*, never one-shot ad-hoc
//!   puts — so [`Publication`] wraps a declared publisher, and there is no
//!   bare-put helper here at all;
//! - **QoS is the closed enum** (RFC 04 §3), mapped to the wire in one place,
//!   including the v1.5 `express` axis (alert/frame) that nothing set before;
//! - **fan-out refusal is layered** (RFC 05 §2.1): generated builders make a
//!   forbidden-fanout write unspellable; this facade adds the *registry*
//!   layer for dynamic callers — a `*`-origin call to a procedure whose slice
//!   declares `fanout = "forbidden"` is refused before any GET leaves.

use std::time::Duration;

use crate::{Error, Result};
use zenkey::origin::{HostId, ServiceOrigin};
use zenkey::qos::QosProfile;
use zenkey::{Declared, Fanout, ProcedureKind};
use zenoh::Session;

use crate::bus::query::FleetAnswer;
use crate::model::registry::SliceSet;
use crate::report::{CallAnswer, CallError, CallOutcome, CallReport};

/// A declared publisher with its QoS profile applied — the only publish path.
pub struct Publication {
    publisher: zenoh::pubsub::Publisher<'static>,
    encoding: Option<String>,
}

impl std::fmt::Debug for Publication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Publication")
            .field("key", &self.publisher.key_expr().as_str())
            .finish_non_exhaustive()
    }
}

/// Declare a publication on a **full wire key** (explorers are un-namespaced;
/// compose with `with_base` first).
///
/// The profile maps to the wire in one place: reliability, congestion
/// control, priority, and the express bit (RFC 04 §3 — `alert` and `frame`
/// are the express profiles; nothing in the workspace ever set it before).
///
/// A wildcard key is refused, never declared ([`check_concrete`], #504).
pub async fn declare_publication(
    session: &Session,
    key: &str,
    qos: QosProfile,
    encoding: Option<&str>,
) -> Result<Publication> {
    declare_publication_with(session, key, WireQos::of_profile(qos), encoding).await
}

/// A publication's QoS axes as zenoh takes them (#612, FJ8a): a v1
/// profile's, or the axes a row recorded (`qos_axes`), which a replay and
/// `pub --from ndjson` publish with exactly. A zk2 owner's QoS is per
/// resource (spec §2.4), and v1's five profile names spell little of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireQos {
    pub priority: zenoh::qos::Priority,
    pub congestion: zenoh::qos::CongestionControl,
    pub reliability: zenoh::qos::Reliability,
    pub express: bool,
}

impl WireQos {
    /// A v1 profile's axes (RFC 04 §3).
    pub fn of_profile(p: QosProfile) -> WireQos {
        WireQos {
            priority: p.priority(),
            congestion: p.congestion_control(),
            reliability: p.reliability(),
            express: p.express(),
        }
    }

    /// The axes a `qos_axes` token spells
    /// ([`crate::report::qos_axes_token`]), or `None` for one it could not
    /// have written.
    pub fn parse(token: &str) -> Option<WireQos> {
        let (priority, congestion, reliability, express) = crate::report::parse_qos_axes(token)?;
        Some(WireQos {
            priority,
            congestion,
            reliability,
            express,
        })
    }
}

/// [`declare_publication`] with the axes given one by one: what a recorded
/// row's `qos_axes` asks for (#612, FJ8a).
pub async fn declare_publication_with(
    session: &Session,
    key: &str,
    qos: WireQos,
    encoding: Option<&str>,
) -> Result<Publication> {
    // The only publish path refuses the blast radius itself (#504), so no
    // frontend can forget to: a caller that wants the refusal before a
    // session opens asks `check_concrete` first, as `zenctl pub` does.
    check_concrete(key, WriteAct::Put)?;
    let publisher = session
        .declare_publisher(key.to_string())
        .reliability(qos.reliability)
        .congestion_control(qos.congestion)
        .priority(qos.priority)
        .express(qos.express)
        .await
        .map_err(|e| Error::bus("declare publisher", key, e))?;
    Ok(Publication {
        publisher,
        encoding: encoding.map(str::to_string),
    })
}

impl Publication {
    /// Publish one payload, with an optional attachment riding beside it
    /// (#117 — attachments are outside the registry's vocabulary and are
    /// never schema-encoded). Sets the wire `Encoding` when one was declared
    /// (RFC 04 v1.5's recommendation: publishers say what they carry).
    pub async fn send(&self, payload: Vec<u8>, attachment: Option<Vec<u8>>) -> Result<()> {
        self.send_stamped(payload, attachment, None).await
    }

    /// [`send`](Self::send), with an explicit HLC timestamp when the caller
    /// mints one (`session.new_timestamp()`). `None` leaves stamping to the
    /// deployment's config — the default put behaviour. The generator uses
    /// this to stamp state samples for LWW (RFC 04 §4) and, for its
    /// `unstamped` fault (#163), to deliberately omit the stamp.
    pub async fn send_stamped(
        &self,
        payload: Vec<u8>,
        attachment: Option<Vec<u8>>,
        timestamp: Option<zenoh::time::Timestamp>,
    ) -> Result<()> {
        let put = self.publisher.put(payload);
        let put = match &self.encoding {
            Some(e) => put.encoding(e.as_str()),
            None => put,
        };
        let put = match attachment {
            Some(a) => put.attachment(a),
            None => put,
        };
        let put = match timestamp {
            Some(ts) => put.timestamp(ts),
            None => put,
        };
        put.await
            .map_err(|e| Error::bus("put", self.publisher.key_expr().as_str(), e))
    }

    /// Publish a tombstone — an authoritative retirement (RFC 04 §1.2),
    /// never a payload marker. The only delete path: it rides the declared
    /// publisher, and `Session::delete` stays unexposed for the same reason
    /// there is no bare-put helper. Gate dynamic keys through
    /// [`check_retire`] first — the class semantics live there.
    pub async fn retire(&self) -> Result<()> {
        self.publisher
            .delete()
            .await
            .map_err(|e| Error::bus("delete", self.publisher.key_expr().as_str(), e))
    }

    /// Undeclare, acknowledged.
    pub async fn undeclare(self) -> Result<()> {
        self.publisher
            .undeclare()
            .await
            .map_err(|e| Error::bus("undeclare publisher", "", e))
    }

    /// Whether any subscriber currently matches **this publication** — a
    /// routing fact about the publisher *this process declared* (RFC 12 §9's
    /// allowed half). It says nothing about other publishers on the key, and
    /// `false` is not a fleet verdict ("no subscriber matched *our*
    /// publication", never "nobody listens here" — RFC 05 §3.1 applied to a
    /// badge).
    pub async fn matching_status(&self) -> Result<bool> {
        self.publisher
            .matching_status()
            .await
            .map(|s| s.matching())
            .map_err(|e| Error::bus("matching status", "", e))
    }

    /// Event-driven matching changes for this publication — the badge feed.
    /// Same honesty bounds as [`matching_status`](Self::matching_status).
    pub async fn matching_events(&self) -> Result<MatchingEvents> {
        let listener = self
            .publisher
            .matching_listener()
            .await
            .map_err(|e| Error::bus("matching listener", "", e))?;
        Ok(MatchingEvents { listener })
    }
}

/// The two acts a key is written with — what [`check_concrete`] names in
/// its refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteAct {
    /// A put: a sample on a declared publisher.
    Put,
    /// A tombstone (RFC 04 §1.2).
    Retire,
}

/// Refuse a write addressed to a wildcard — the one refusal every write
/// path shares, and the one no `force` overrides (#504).
///
/// A tombstone was the first act to carry it (RFC 04 §1.2, v1.12); a put
/// is the same blast radius with a payload attached: it is delivered to
/// every subscriber whose subscription intersects it, across every
/// producer's keys, and RFC 07 §3 makes a publisher's key concrete by rule.
/// `zenctl pub 'prod/v1/**' x` used to exit 0 with "published". `$` counts:
/// the only place it may stand in a key expression is `$*`.
pub fn check_concrete(key: &str, act: WriteAct) -> Result<()> {
    if !key.contains('*') && !key.contains('$') {
        return Ok(());
    }
    let why = match act {
        WriteAct::Retire => {
            "a tombstone is addressed to one concrete key; a wildcard delete is not \
             an operator act, it is a blast radius (RFC 04 §1.2, v1.12)"
        }
        WriteAct::Put => {
            "a put is addressed to one concrete key (RFC 07 §3: a publisher always \
             names its own); a wildcard put reaches every subscriber it intersects, \
             across every producer's keys — not an operator act, a blast radius"
        }
    };
    Err(Error::unaskable(
        key,
        format!("is a wildcard — {why}. Not overridable."),
    ))
}

/// Every chunk of the key [`call`] is about to build is a plain chunk
/// (RFC 03 §2) — or the call is refused, naming the chunk (#561).
///
/// The selector builders assert this, because to a generated builder the
/// producer and the procedure are registry constants and an illegal one is
/// a programmer error. A dynamic caller's are not: zengui's Send tool takes
/// a declared path such as `config/{device}/{group}/set` as it is written,
/// and before this check that template reached the assert and took the
/// window down. A refusal (exit 2 in zenctl's terms) is what an input that
/// cannot be asked deserves.
fn check_chunks(target: &CallTarget, producer: &str, procedure: &str) -> Result<()> {
    // A service call names no producer: its origin is the service.
    let producer = match target {
        CallTarget::Service(_) => None,
        CallTarget::Host(_) | CallTarget::Fleet => Some(producer),
    };
    let bad = producer
        .into_iter()
        .chain(procedure.split('/'))
        .find(|c| !zenkey::grammar::is_valid_plain_chunk(c));
    match bad {
        None => Ok(()),
        Some(chunk) => Err(Error::unaskable(
            procedure,
            format!(
                "has the chunk {chunk:?}, which is not a plain chunk (RFC 03 §2: lowercase \
                 letters, digits, `.`, `_` and `-`, alphanumeric at both ends) — a declared \
                 template such as `{{device}}` is filled in before it is called"
            ),
        )),
    }
}

/// What a key is, for the purpose of retiring it — the guard's positive
/// verdict, so callers print facts instead of re-deriving them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetireClass {
    /// State-shaped: retirement is the class's own semantics (RFC 04 §1.2).
    State {
        /// Whether a loaded registry recognises the subject. An unregistered
        /// state key still tombstones authoritatively — but no `ttl_s`
        /// bounds how long the tombstone stays observable.
        registered: bool,
        /// The registry's `ttl_s`, when declared: storages keep the
        /// tombstone observable at least this long (RFC 04 §1.2).
        ttl_s: Option<i64>,
    },
    /// A v1 key off the state class (telemetry/events, or a verbatim
    /// plane) — retired anyway, as a forced operator cleanup (v1.12).
    NonState { class: String },
    /// The grammar could not say what the key is — retired blind, forced.
    Unclassified { reason: String },
}

/// Refuse a tombstone the class semantics do not license, unless forced.
///
/// The judgment mirrors `bench`'s idempotence guard: the refusal is
/// grammar- and registry-driven, and the messages cite what they know.
/// Unlike `bench`, a missing registry does not blind us on the happy path —
/// the class is written in the key itself, so a state key passes with no
/// slices loaded. The one unconditional refusal is a wildcard: a tombstone
/// is addressed to one concrete key (RFC 04 §1.2, v1.12), and no `force`
/// overrides a blast radius.
pub fn check_retire(
    base: &str,
    key: &str,
    slices: Option<&SliceSet>,
    force: bool,
) -> Result<RetireClass> {
    check_concrete(key, WriteAct::Retire)?;
    let facts = crate::model::facts::describe_key(base, key, slices).facts;
    use crate::model::facts::{ClassKind, KeyShape, Registration};
    match &facts.shape {
        KeyShape::V1(v) if v.class_kind == ClassKind::State => {
            let (registered, ttl_s) = match &facts.registration {
                Registration::Registered(s) => (true, s.ttl_s),
                _ => (false, None),
            };
            Ok(RetireClass::State { registered, ttl_s })
        }
        KeyShape::V1(v) if matches!(v.class_kind, ClassKind::Telemetry | ClassKind::Events) => {
            if force {
                return Ok(RetireClass::NonState {
                    class: v.class.clone(),
                });
            }
            Err(Error::unaskable(
                key,
                format!(
                    "is {}-shaped — RFC 04 §1: a delete there is meaningless and \
                     MUST NOT be sent by the class's publisher. Retiring it anyway \
                     is an operator cleanup (RFC 04 §1.2, v1.12) — pass --i-know \
                     to mean it.",
                    v.class
                ),
            ))
        }
        KeyShape::V1(v) => {
            if force {
                return Ok(RetireClass::NonState {
                    class: v.class.clone(),
                });
            }
            Err(Error::unaskable(
                key,
                format!(
                    "sits on the {} plane — a plane key answers GETs or carries \
                     frames; a tombstone there is at most a storage purge \
                     (RFC 04 §1.2, v1.12) — pass --i-know to mean it.",
                    v.class
                ),
            ))
        }
        KeyShape::NotUnderBase | KeyShape::Unparsed { .. } => {
            let reason = match &facts.shape {
                KeyShape::Unparsed { reason } => reason.clone(),
                _ => format!("not under base {base:?}"),
            };
            if force {
                return Ok(RetireClass::Unclassified { reason });
            }
            Err(Error::unaskable(
                key,
                format!(
                    "cannot be classified under base {base:?} ({reason}) — 'not \
                     asked' is not 'state' (RFC 09 §5.1 O4); pass --i-know to \
                     retire an unclassified key."
                ),
            ))
        }
    }
}

/// A stream of matching changes for one **self-declared** entity (a
/// [`Publication`] or a [`crate::RepeatingQuery`]). These are the only two
/// places a matching claim can be made from — a foreign publisher's consumers
/// are not observable without publishing on their key, and that half stays
/// deferred (RFC 12 §9; #38/#80 adoption note).
pub struct MatchingEvents {
    listener: zenoh::matching::MatchingListener<
        zenoh::handlers::FifoChannelHandler<zenoh::matching::MatchingStatus>,
    >,
}

impl MatchingEvents {
    pub(crate) async fn for_querier(querier: &zenoh::query::Querier<'_>) -> Result<Self> {
        let listener = querier
            .matching_listener()
            .await
            .map_err(|e| Error::bus("matching listener", "", e))?;
        Ok(MatchingEvents { listener })
    }

    /// The next change: `Some(true)` = at least one matcher appeared,
    /// `Some(false)` = the last one left, `None` = the entity was undeclared.
    pub async fn recv(&self) -> Option<bool> {
        self.listener.recv_async().await.ok().map(|s| s.matching())
    }

    /// The same changes as a [`Stream`](futures_core::Stream) (#343).
    ///
    /// Borrows, so a caller can keep gating on `recv` elsewhere; the item is
    /// the same projected `bool` rather than the raw `MatchingStatus`, because
    /// what a caller acts on is "is anyone there", not the status object.
    pub fn stream(&self) -> impl futures_core::Stream<Item = bool> + '_ {
        futures_util::StreamExt::map(self.listener.stream(), |s| s.matching())
    }
}

/// Who a call is addressed to. Typed — a fleet call is a deliberate variant,
/// never a string that happens to contain `*` (RFC 08 §1.1's origin-argument
/// rule for dynamic callers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallTarget {
    /// One host, by validated origin id.
    Host(HostId),
    /// Every host serving the procedure — requires the RFC 05 §2.1 fan-in
    /// discipline, which [`call`] applies.
    Fleet,
    /// A registered service origin (`@catalog`, …) — no producer chunk.
    Service(ServiceOrigin),
}

impl CallTarget {
    /// Parse a CLI-shaped target: `*` = fleet, `@name` = service, else a host
    /// origin id (validated — a hostname here is the RFC 06 §6 bridge bug,
    /// and it fails loudly instead of being string-glued into a key).
    pub fn parse(s: &str) -> Result<CallTarget> {
        if s == "*" {
            return Ok(CallTarget::Fleet);
        }
        if s.starts_with('@') {
            return Ok(CallTarget::Service(ServiceOrigin::new(s)?));
        }
        HostId::parse(s).map(CallTarget::Host).map_err(|e| {
            Error::unaskable(
                "origin",
                format!("{e} — a hostname is not an origin; resolve it first (RFC 06 §6)"),
            )
        })
    }
}

/// A reply attachment, projected for the report: JSON if it parses, UTF-8
/// text if it decodes, else a size tag. Never schema-decoded — an attachment
/// is outside the registry's vocabulary (#117) — and deliberately
/// dependency-free: the report shapes are unconditional while the decode
/// module is feature-gated.
fn attachment_value(bytes: &[u8]) -> serde_json::Value {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        v
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        serde_json::Value::String(s.to_string())
    } else {
        serde_json::Value::String(format!("<{} bytes>", bytes.len()))
    }
}

/// The procedures this convention defines, with the kind it gives them —
/// so their kind is a fact about the convention, not something a registry
/// has to bother declaring (`bench`'s `FRAMEWORK_READS`, widened): RFC 08
/// §6's `introspect` and §7's `describe` are reads every producer serves,
/// and RFC 05 §5.1's table fixes the configuration keys — the read-back a
/// read, every other key under it a write.
///
/// Consulted only after the loaded slices: a producer's own declaration of
/// one of these paths is still the registry layer's to judge.
fn convention_kind(procedure: &str) -> Option<ProcedureKind> {
    let chunks: Vec<&str> = procedure.split('/').collect();
    match chunks.as_slice() {
        ["introspect" | "describe"] => Some(ProcedureKind::Read),
        ["config", _] => Some(ProcedureKind::Read),
        ["config", _, "confirm" | "cancel" | "extend" | "persist"] | ["config", _, _, "set"] => {
            Some(ProcedureKind::Write)
        }
        _ => None,
    }
}

/// A slice's declaration of a called procedure: the literal path first,
/// then a declared pattern whose `{var}` chunks the call fills
/// (`config/{device}/access/set` declares `config/wlan0/access/set`). A
/// literal-only lookup made every templated write look undeclared — which
/// was harmless while undeclared meant "proceed", and is not now.
fn declared_procedure<'a>(
    slice: &'a zenkey::slice::RegistrySlice,
    procedure: &str,
) -> Option<&'a zenkey::slice::ProcedureDecl> {
    let fills = |pattern: &str| {
        let (p, c): (Vec<&str>, Vec<&str>) =
            (pattern.split('/').collect(), procedure.split('/').collect());
        p.len() == c.len()
            && p.iter().zip(&c).all(|(p, c)| {
                p == c || (p.starts_with('{') && p.ends_with('}') && !p.ends_with("...}"))
            })
    };
    slice
        .procedures
        .iter()
        .find(|p| p.path == procedure)
        .or_else(|| slice.procedures.iter().find(|p| fills(&p.path)))
}

/// The registry layer of RFC 05 §2.1's write fan-out refusal, for dynamic
/// callers — the generated builders make a forbidden fan-out unspellable,
/// and a CLI argument has no builder.
///
/// Only a [`CallTarget::Fleet`] call is judged. Three outcomes:
///
/// - **allowed** — the procedure is declared `fanout = "allowed"`, or is a
///   read (declared, or by the convention's own table: `introspect`,
///   `describe`, and RFC 05 §5.1's configuration read-back);
/// - **forbidden, unconditionally** — declared `fanout = "forbidden"`, or a
///   write with no readable `fanout`: RFC 08 §2 defaults a write to
///   forbidden, and introspect serves the TOML verbatim, so the default is
///   this guard's to apply — and a write by the convention's own table
///   (every RFC 05 §5.1 `config/` key but the read-back). No `force` moves
///   it;
/// - **not established, refused unless `force`** (#505) — no slices loaded
///   (`--no-validate`, a degraded introspect sweep), a producer or procedure
///   they do not declare, or a declaration whose kind this build cannot
///   read. Until #505 every one of these *proceeded*: the guard ran only
///   when it could find a declaration, so `service call '*' p reset
///   --no-validate` fanned a write to every origin. Not knowing the
///   declaration is not a licence (RFC 09 §5.1 O4).
pub fn check_fanout(
    target: &CallTarget,
    slices: Option<&SliceSet>,
    producer: &str,
    procedure: &str,
    force: bool,
) -> Result<()> {
    if !matches!(target, CallTarget::Fleet) {
        return Ok(());
    }
    let what = || format!("procedure {producer}/{procedure}");
    let refuse = |declared: String| {
        Err(Error::unaskable(
            what(),
            format!(
                "{declared} — a fleet (`*`) call to it is refused \
                 (RFC 05 §2.1); name one origin"
            ),
        ))
    };
    let slice = slices.and_then(|s| s.get(producer));
    let unestablished = match slice.and_then(|s| declared_procedure(s, procedure)) {
        Some(decl) => {
            let kind = decl.kind.as_ref().and_then(Declared::known);
            match (decl.fanout.as_ref(), kind) {
                (Some(f), _) if f.is(&Fanout::Allowed) => return Ok(()),
                // Three refusals, because there are three reasons. Reading
                // `fanout.is_some()` once folded the second into the first
                // and told the operator the slice "declares fanout =
                // \"forbidden\"" when it declared something this build
                // cannot read — a claim that sends them grepping the
                // registry for a string that is not in it.
                (Some(f), _) if f.is(&Fanout::Forbidden) => {
                    return refuse("declares fanout = \"forbidden\"".to_string());
                }
                (Some(f), Some(ProcedureKind::Write)) => {
                    return refuse(format!(
                        "declares fanout = {:?}, a token this build does not know — \
                         RFC 08 §2 defaults a write to forbidden and an unreadable \
                         spelling is not a licence",
                        f.token()
                    ));
                }
                (None, Some(ProcedureKind::Write)) => {
                    return refuse(
                        "is a write with no declared fanout, which defaults to forbidden \
                         (RFC 08 §2)"
                            .to_string(),
                    );
                }
                (_, Some(ProcedureKind::Read)) => return Ok(()),
                (_, None) => match &decl.kind {
                    Some(k) => format!(
                        "is declared kind = {:?}, a token this build does not know",
                        k.token()
                    ),
                    None => "is declared with no kind".to_string(),
                },
            }
        }
        None => match convention_kind(procedure) {
            Some(ProcedureKind::Read) => return Ok(()),
            Some(ProcedureKind::Write) => {
                return refuse(
                    "is a write by the convention's own table (RFC 05 §5.1), and a \
                     write with no declared fanout defaults to forbidden (RFC 08 §2)"
                        .to_string(),
                );
            }
            None => match (slices, slice) {
                (None, _) => "has no registry loaded to declare it".to_string(),
                (Some(_), None) => {
                    format!("is not declared: the loaded registry has no producer {producer:?}")
                }
                (Some(_), Some(_)) => {
                    format!("is not declared: producer {producer:?} declares no such procedure")
                }
            },
        },
    };
    if force {
        return Ok(());
    }
    Err(Error::unaskable(
        what(),
        format!(
            "{unestablished}, so whether it is a write could not be established — \
             RFC 08 §2 defaults a write to fanout = \"forbidden\", and not knowing the \
             declaration is not a licence (RFC 09 §5.1 O4). A fleet (`*`) call to it is \
             refused (RFC 05 §2.1): name one origin, or pass --i-know to fan it out anyway"
        ),
    ))
}

/// One procedure call, as a spec rather than nine positional arguments —
/// the shape [`crate::BenchSpec`] already uses next door, for the same
/// call.
///
/// `body` and `attachment` are owned because they are consumed: they ride
/// the GET out and nothing reads them again.
pub struct CallSpec<'a> {
    pub target: &'a CallTarget,
    pub producer: &'a str,
    /// Slash-separated, as the registry spells it (`capture/trigger`).
    pub procedure: &'a str,
    /// Selector parameters, joined with `;` onto the key (RFC 05 §1).
    pub params: &'a [String],
    /// The request payload, already through the encode ladder.
    pub body: Option<Vec<u8>>,
    /// Verbatim, never schema-encoded — an attachment is outside the
    /// registry's vocabulary (#117).
    pub attachment: Option<Vec<u8>>,
    pub timeout: Duration,
    /// The loaded registry, for the fan-out guard ([`check_fanout`]).
    /// `None` = none loaded — and for a fleet call that is a refusal unless
    /// [`force`](Self::force), never a licence (#505).
    pub slices: Option<&'a SliceSet>,
    /// Fan out a fleet call whose procedure's kind could not be established
    /// — no registry, or one that does not declare it. The caller must have
    /// meant it (`--i-know`). It never overrides a declared or defaulted
    /// `fanout = "forbidden"`: that refusal is RFC 05 §2.1's MUST.
    pub force: bool,
}

/// Call a procedure and report every attributed answer.
///
/// - The key composes through the typed builders (never `format!`), lifted to
///   the wire with the configured base.
/// - `params` ride the selector (`?k=v;k=v`), the body rides the payload
///   (RFC 05 §1).
/// - **Fan-out guard** ([`check_fanout`]): a [`CallTarget::Fleet`] call to a
///   write the registry forbids to fan out is refused, and so — unless
///   [`CallSpec::force`] — is one whose kind nobody could establish.
/// - Exit-code semantics stay on [`CallReport::exit_code`]: an error reply is
///   a failure, zero replies stay a distinct non-verdict (RFC 05 §3.1).
pub async fn call(fleet: &crate::Fleet<'_>, spec: CallSpec<'_>) -> Result<CallReport> {
    let (key, timeout, answers) = call_answers(fleet, spec).await?;
    Ok(project_call(key, timeout, &answers))
}

/// The GET half of [`call`]: the guard, the key, the fan-in — and the raw
/// [`FleetAnswer`]s, which carry what the [`CallReport`] projection drops
/// (the reply's HLC, for one). `check conform` reads those; [`call`] does
/// not, so the split keeps the report shape untouched.
pub(crate) async fn call_answers(
    fleet: &crate::Fleet<'_>,
    spec: CallSpec<'_>,
) -> Result<(String, Duration, Vec<FleetAnswer>)> {
    let CallSpec {
        target,
        producer,
        procedure,
        params,
        body,
        attachment,
        timeout,
        slices,
        force,
    } = spec;
    check_chunks(target, producer, procedure)?;
    check_fanout(target, slices, producer, procedure, force)?;

    let segments: Vec<&str> = procedure.split('/').collect();
    let relative = match target {
        CallTarget::Host(id) => {
            let origin = zenkey::origin::RemoteOrigin::from_host(id.clone());
            zenkey::selector::rpc_at(&origin, producer, &segments).to_string()
        }
        CallTarget::Fleet => zenkey::selector::fleet_rpc(producer, &segments).to_string(),
        CallTarget::Service(origin) => zenkey::selector::service_rpc(origin, &segments).to_string(),
    };
    let mut key = fleet.wire(relative);
    if !params.is_empty() {
        key.push('?');
        key.push_str(&params.join(";"));
    }

    let answers = crate::bus::query::fleet_get(
        fleet,
        &key,
        &crate::bus::query::GetOpts::new(timeout)
            .payload(body)
            .attachment(attachment),
    )
    .await?;
    Ok((key, timeout, answers))
}

/// The [`CallReport`] projection of a fan-in's answers.
fn project_call(key: String, timeout: Duration, answers: &[FleetAnswer]) -> CallReport {
    CallReport {
        key,
        // The wait is part of the claim (R5): a silent call must be readable
        // against how long it listened.
        timeout_s: timeout.as_secs_f64(),
        answers: answers
            .iter()
            .map(|a| {
                // The reply attachment used to be visible at the fleet_get
                // layer and dropped at this projection (#126) — carried now,
                // present only when the wire carried one.
                let (att, att_bytes) = match &a.attachment {
                    Some(z) => {
                        let bytes = z.to_bytes();
                        (Some(attachment_value(&bytes)), Some(bytes.len()))
                    }
                    None => (None, None),
                };
                let outcome = match &a.answer {
                    crate::bus::query::Answer::Value(bytes) => {
                        let bytes = bytes.to_bytes();
                        match serde_json::from_slice::<serde_json::Value>(&bytes) {
                            Ok(v) => CallOutcome::Ok {
                                value: Some(v),
                                text: None,
                            },
                            Err(_) => CallOutcome::Ok {
                                value: None,
                                text: Some(String::from_utf8_lossy(&bytes).to_string()),
                            },
                        }
                    }
                    crate::bus::query::Answer::Error { name, message } => {
                        CallOutcome::Err(CallError {
                            name: name.clone(),
                            message: message.clone(),
                        })
                    }
                };
                CallAnswer {
                    origin: a.origin.clone(),
                    outcome,
                    attachment: att,
                    attachment_bytes: att_bytes,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::slice::{ProcedureDecl, RegistrySlice, SubjectDecl};

    fn slice_with_state_subject() -> SliceSet {
        let mut health = SubjectDecl::new("health", zenkey::Class::State);
        health.type_name = "Health".into();
        health.ttl_s = Some(900);
        let mut slice = RegistrySlice::new("1.0", "t", "sysinfo");
        slice.subjects = vec![health];
        SliceSet::from_slices(vec![slice])
    }

    /// The five outcomes of the retire guard (RFC 04 §1.2, v1.12), each
    /// citing what it knows.
    #[test]
    fn a_wildcard_retire_is_refused_unconditionally() {
        for force in [false, true] {
            let err = check_retire("", "v1/h-3fa9c2d41b7e/state/sysinfo/**", None, force)
                .unwrap_err()
                .to_string();
            assert!(err.contains("blast radius"), "{err}");
        }
    }

    /// #504: the wildcard refusal is one function, and a put meets it as
    /// a tombstone does — `$*` included, a concrete key untouched.
    #[test]
    fn a_wildcard_write_is_refused_whatever_the_act() {
        for key in [
            "prod/v1/**",
            "v1/*/state/sysinfo/health",
            "v1/h-3fa9c2d41b7e/$*",
        ] {
            for act in [WriteAct::Put, WriteAct::Retire] {
                let err = check_concrete(key, act).unwrap_err();
                assert!(err.is_unaskable(), "a refused input, exit 2: {err}");
                let err = err.to_string();
                assert!(err.contains("is a wildcard"), "{err}");
                assert!(err.contains("blast radius"), "{err}");
                assert!(err.contains("Not overridable"), "{err}");
            }
        }
        let put = check_concrete("prod/v1/**", WriteAct::Put)
            .unwrap_err()
            .to_string();
        assert!(put.contains("RFC 07 §3"), "{put}");
        assert!(put.contains("every subscriber"), "{put}");
        assert!(check_concrete("v1/h-3fa9c2d41b7e/state/sysinfo/health", WriteAct::Put).is_ok());
    }

    /// … and the only publish path asks it, so no frontend can forget to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_wildcard_publication_is_never_declared() {
        let session = crate::bus::session::standalone().await;
        let err = declare_publication(&session, "prod/v1/**", QosProfile::Sampled, None)
            .await
            .unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        assert!(err.to_string().contains("blast radius"), "{err}");
    }

    #[test]
    fn a_state_key_retires_without_a_registry() {
        // The class is written in the key itself — unlike bench's
        // idempotence, a missing registry does not blind the guard.
        let got = check_retire("", "v1/h-3fa9c2d41b7e/state/sysinfo/health", None, false).unwrap();
        assert_eq!(
            got,
            RetireClass::State {
                registered: false,
                ttl_s: None
            }
        );
        // With the registry loaded, the tombstone-visibility bound rides out.
        let slices = slice_with_state_subject();
        let got = check_retire(
            "",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
            Some(&slices),
            false,
        )
        .unwrap();
        assert_eq!(
            got,
            RetireClass::State {
                registered: true,
                ttl_s: Some(900)
            }
        );
    }

    /// #561: a procedure path that is not plain chunks is a refusal, never
    /// the selector builder's assert — the producer too, except on a
    /// service call, which names none.
    #[test]
    fn a_call_on_a_template_or_a_bad_chunk_is_refused_not_panicked_on() {
        let host = CallTarget::parse("h-0123456789ab").unwrap();
        let err = check_chunks(&host, "modem", "config/{device}/{group}/set").unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        assert!(err.to_string().contains("\"{device}\""), "{err}");
        assert!(check_chunks(&host, "modem", "config/rf0/radio/set").is_ok());
        assert!(
            check_chunks(&host, "modem", "config//set").is_err(),
            "an empty chunk"
        );
        assert!(
            check_chunks(&host, "Modem", "config/rf0").is_err(),
            "the producer too"
        );
        assert!(check_chunks(&CallTarget::Fleet, "modem", "a/B").is_err());
        let service = CallTarget::Service(zenkey::origin::ServiceOrigin::catalog());
        assert!(
            check_chunks(&service, "", "describe").is_ok(),
            "a service call has no producer to check"
        );
    }

    #[test]
    fn a_telemetry_retire_needs_i_know_and_cites_the_rfc() {
        let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage";
        let err = check_retire("", key, None, false).unwrap_err().to_string();
        assert!(err.contains("MUST NOT"), "{err}");
        assert!(err.contains("v1.12"), "{err}");
        assert!(err.contains("--i-know"), "{err}");
        assert_eq!(
            check_retire("", key, None, true).unwrap(),
            RetireClass::NonState {
                class: "telemetry".to_string()
            }
        );
    }

    #[test]
    fn a_plane_retire_needs_i_know_too() {
        let key = "v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect";
        let err = check_retire("", key, None, false).unwrap_err().to_string();
        assert!(err.contains("plane"), "{err}");
        assert!(matches!(
            check_retire("", key, None, true).unwrap(),
            RetireClass::NonState { class } if class == "@rpc"
        ));
    }

    #[test]
    fn an_unclassified_retire_needs_i_know_and_names_o4() {
        // A foreign key under an empty base parses as... nothing v1.
        let err = check_retire("", "some/foreign/key", None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("O4"), "{err}");
        assert!(matches!(
            check_retire("", "some/foreign/key", None, true).unwrap(),
            RetireClass::Unclassified { .. }
        ));
        // And a key under another base is unclassified, not misclassified.
        let err = check_retire("acme", "other/v1/h-3fa9c2d41b7e/state/x/y", None, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot be classified"), "{err}");
    }

    fn slice_with_proc(kind: &str, fanout: Option<&str>) -> SliceSet {
        let mut trigger = ProcedureDecl::new("capture/trigger");
        trigger.kind = Some(Declared::parse(kind));
        trigger.reply = Some("Ack".into());
        trigger.fanout = fanout.map(Declared::parse);
        trigger.idempotent = Some(false);
        let mut slice = RegistrySlice::new("1.0", "t", "netring");
        slice.procedures = vec![trigger];
        SliceSet::from_slices(vec![slice])
    }

    #[test]
    fn call_targets_parse_and_validate() {
        assert_eq!(CallTarget::parse("*").unwrap(), CallTarget::Fleet);
        assert!(matches!(
            CallTarget::parse("@catalog").unwrap(),
            CallTarget::Service(_)
        ));
        assert!(matches!(
            CallTarget::parse("h-3fa9c2d41b7e").unwrap(),
            CallTarget::Host(_)
        ));
        // The RFC 06 §6 bridge bug fails loudly, with the pointer.
        let err = CallTarget::parse("toolbx").unwrap_err().to_string();
        assert!(err.contains("RFC 06 §6"), "{err}");
    }

    /// The judgement of [`check_fanout`] alone, row by row.
    fn fanout(slices: Option<&SliceSet>, procedure: &str, force: bool) -> Result<()> {
        check_fanout(&CallTarget::Fleet, slices, "netring", procedure, force)
    }

    /// #505: a fleet call whose procedure nobody could establish is refused
    /// unless forced — with no registry, with one that does not declare the
    /// producer or the procedure, and with a kind this build cannot read.
    /// Before, each of these skipped the guard and fanned out.
    #[test]
    fn an_unestablished_fleet_call_is_refused_unless_forced() {
        let declared = slice_with_proc("write", None);
        let mut unknown_kind = ProcedureDecl::new("capture/trigger");
        unknown_kind.kind = Some(Declared::parse("actuate"));
        let mut slice = RegistrySlice::new("1.0", "t", "netring");
        slice.procedures = vec![unknown_kind];
        let unknown_kind = SliceSet::from_slices(vec![slice]);
        let mut no_kind = ProcedureDecl::new("capture/trigger");
        no_kind.kind = None;
        let mut slice = RegistrySlice::new("1.0", "t", "netring");
        slice.procedures = vec![no_kind];
        let no_kind = SliceSet::from_slices(vec![slice]);
        let other = SliceSet::from_slices(vec![RegistrySlice::new("1.0", "t", "other")]);

        for (slices, procedure, says) in [
            (None, "reset", "no registry loaded"),
            (Some(&other), "reset", "no producer \"netring\""),
            (Some(&declared), "reset", "declares no such procedure"),
            (Some(&unknown_kind), "capture/trigger", "\"actuate\""),
            (Some(&no_kind), "capture/trigger", "with no kind"),
        ] {
            let err = fanout(slices, procedure, false).unwrap_err();
            assert!(err.is_unaskable(), "exit 2: {err}");
            let err = err.to_string();
            assert!(err.contains(says), "{says:?} in {err}");
            assert!(err.contains("could not be established"), "{err}");
            assert!(err.contains("RFC 05 §2.1"), "{err}");
            assert!(err.contains("--i-know"), "{err}");
            fanout(slices, procedure, true).expect("forced, it fans out");
        }

        // One origin is never a fan-out, whatever is known.
        let host = CallTarget::parse("h-3fa9c2d41b7e").unwrap();
        check_fanout(&host, None, "netring", "reset", false).unwrap();
    }

    /// What `force` never moves: a declared — or defaulted — forbidden
    /// fan-out, and the convention's own writes. And what needs no force:
    /// a declared read, an allowed write, the convention's reads (with no
    /// registry at all — `config get '*'` and `bench rpc '*'` of
    /// introspect keep working), and a templated declaration the call fills.
    #[test]
    fn force_moves_only_the_unknown() {
        for slices in [
            slice_with_proc("write", Some("forbidden")),
            slice_with_proc("write", None),
            slice_with_proc("write", Some("per-iface")),
        ] {
            let err = fanout(Some(&slices), "capture/trigger", true).unwrap_err();
            assert!(err.to_string().contains("RFC 05 §2.1"), "{err}");
        }
        for procedure in [
            "config/wlan0/link/set",
            "config/wlan0/confirm",
            "config/wlan0/persist",
        ] {
            let err = fanout(None, procedure, true).unwrap_err().to_string();
            assert!(err.contains("RFC 05 §5.1"), "{err}");
        }

        fanout(
            Some(&slice_with_proc("read", None)),
            "capture/trigger",
            false,
        )
        .unwrap();
        fanout(
            Some(&slice_with_proc("write", Some("allowed"))),
            "capture/trigger",
            false,
        )
        .unwrap();
        for procedure in ["introspect", "describe", "config/wlan0"] {
            fanout(None, procedure, false).unwrap();
        }

        let mut set = ProcedureDecl::new("config/{device}/access/set");
        set.kind = Some(Declared::parse("write"));
        let mut lanes = ProcedureDecl::new("device/{device}/lanes");
        lanes.kind = Some(Declared::parse("read"));
        let mut slice = RegistrySlice::new("1.0", "t", "netring");
        slice.procedures = vec![set, lanes];
        let templated = SliceSet::from_slices(vec![slice]);
        let err = fanout(Some(&templated), "config/wlan0/access/set", true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("defaults to forbidden"), "{err}");
        fanout(Some(&templated), "device/eth0/lanes", false).unwrap();
        // A pattern fills chunk for chunk, never across a slash.
        assert!(fanout(Some(&templated), "device/eth0/x/lanes", false).is_err());
    }

    /// The registry layer of the three-layer refusal: a fleet call to a
    /// declared forbidden-fanout write never leaves the process.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fleet_calls_to_forbidden_fanout_are_refused() {
        let session = crate::bus::session::standalone().await;
        let slices = slice_with_proc("write", Some("forbidden"));
        let err = call(
            &crate::Fleet::new(&session, ""),
            CallSpec {
                target: &CallTarget::Fleet,
                producer: "netring",
                procedure: "capture/trigger",
                params: &[],
                body: None,
                attachment: None,
                timeout: Duration::from_millis(100),
                slices: Some(&slices),
                force: false,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("fanout"), "{err}");
        assert!(err.contains("RFC 05 §2.1"), "{err}");

        // A write whose TOML *omits* fanout is refused the same way: RFC 08
        // §2 defaults `kind = "write"` to forbidden, and introspect serves
        // the TOML verbatim — the default is this guard's to apply.
        let err = call(
            &crate::Fleet::new(&session, ""),
            CallSpec {
                target: &CallTarget::Fleet,
                producer: "netring",
                procedure: "capture/trigger",
                params: &[],
                body: None,
                attachment: None,
                timeout: Duration::from_millis(100),
                slices: Some(&slice_with_proc("write", None)),
                force: false,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("defaults to forbidden"), "{err}");
        assert!(err.contains("RFC 08 §2"), "{err}");
        assert!(err.contains("RFC 05 §2.1"), "{err}");

        // A token this build cannot read is refused too — and the refusal
        // says which token, rather than claiming the slice declared
        // "forbidden" and sending the operator to grep for a string that is
        // not in their registry.
        let err = call(
            &crate::Fleet::new(&session, ""),
            CallSpec {
                target: &CallTarget::Fleet,
                producer: "netring",
                procedure: "capture/trigger",
                params: &[],
                body: None,
                attachment: None,
                timeout: Duration::from_millis(100),
                slices: Some(&slice_with_proc("write", Some("per-iface"))),
                force: false,
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("per-iface"), "{err}");
        assert!(err.contains("does not know"), "{err}");
        assert!(
            !err.contains("declares fanout = \"forbidden\""),
            "the slice declared no such thing: {err}"
        );
        assert!(err.contains("RFC 05 §2.1"), "{err}");

        // An explicit `fanout = "allowed"` write still fans out, and a read
        // with nothing declared keeps its allowed default (zero replies here
        // — a non-verdict, not an error).
        for slices in [
            slice_with_proc("write", Some("allowed")),
            slice_with_proc("read", None),
        ] {
            let report = call(
                &crate::Fleet::new(&session, ""),
                CallSpec {
                    target: &CallTarget::Fleet,
                    producer: "netring",
                    procedure: "capture/trigger",
                    params: &[],
                    body: None,
                    attachment: None,
                    timeout: Duration::from_millis(100),
                    slices: Some(&slices),
                    force: false,
                },
            )
            .await
            .unwrap();
            assert_eq!(report.exit_code(), 2, "silence stays exit 2");
        }
    }
}
