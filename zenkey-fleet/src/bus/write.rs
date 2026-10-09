//! The write facade (issue #36): the one way an explorer writes to the bus
//! — a declared publication. v1's disciplined `@rpc` call left with the
//! last verb that made one (`config`, #612 FJ9); zk2's operations are called
//! through the runtime's client ([`crate::bus::operation`]).
//!
//! - **P7**: publishers are *declared*, never one-shot ad-hoc puts — so
//!   [`Publication`] wraps a declared publisher, and there is no bare-put
//!   helper here at all;
//! - **QoS is mapped to the wire in one place**, express axis included.

use crate::{Error, Result};
use zenkey::qos::QosProfile;
use zenoh::Session;

use crate::model::registry::SliceSet;

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

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::slice::{RegistrySlice, SubjectDecl};

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
}
