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
use zenoh::Session;

/// A declared publisher with its QoS axes applied — the only publish path.
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

/// A publication's QoS axes as zenoh takes them (#612, FJ8a): what a
/// recorded row's `qos_axes` asks for, which a replay and `pub --from
/// ndjson` publish with exactly, or [`WireQos::DEFAULT`]. A zk2 owner's QoS
/// is per resource (spec §2.4); a tool writing a foreign key has no
/// contract to read it from, so it publishes what it recorded or what it is
/// told. v1's five profile names (RFC 04 §3) left with the v1 dependency
/// (#612, FJ9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireQos {
    pub priority: zenoh::qos::Priority,
    pub congestion: zenoh::qos::CongestionControl,
    pub reliability: zenoh::qos::Reliability,
    pub express: bool,
}

impl WireQos {
    /// zenoh's own defaults — `data/drop/reliable`, no express: what a
    /// publisher declared with nothing said publishes with.
    pub const DEFAULT: WireQos = WireQos {
        priority: zenoh::qos::Priority::DEFAULT,
        congestion: zenoh::qos::CongestionControl::DEFAULT,
        reliability: zenoh::qos::Reliability::DEFAULT,
        express: false,
    };

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

    /// The `qos_axes` token for these axes, which [`WireQos::parse`] reads
    /// back.
    pub fn token(&self) -> String {
        crate::report::qos_axes_token(
            self.priority,
            self.congestion,
            self.reliability,
            self.express,
        )
    }
}

/// Declare a publication on a **full wire key** (a raw session is in no
/// namespace; join one first with [`crate::with_namespace`]), with its QoS
/// axes applied in one place: reliability, congestion control, priority and
/// the express bit.
///
/// A wildcard key is refused, never declared ([`check_concrete`], #504).
pub async fn declare_publication(
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

/// Refuse a tombstone a tool has no licence to send, unless forced.
///
/// A wildcard is refused whatever `force` says: a tombstone is addressed to
/// one concrete key, and no flag overrides a blast radius ([`check_concrete`]).
/// A concrete key is retired only when `force` says the operator means it.
/// What a delete means is its owner's to say: a zk2 service's own key is
/// refused before this is asked (P3, spec §6), and a key no contract
/// describes has no class a tool could read a licence from — v1 read one
/// off the key's class and its registry (RFC 04 §1.2), and that reading
/// left with the v1 grammar (#612, FJ9).
pub fn check_retire(key: &str, force: bool) -> Result<()> {
    check_concrete(key, WriteAct::Retire)?;
    if force {
        return Ok(());
    }
    Err(Error::unaskable(
        key,
        "is a tombstone on a key no contract describes: nothing says a delete is \
         its writer's to send, so this tool will not presume it — pass --i-know \
         to retire it anyway",
    ))
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

    /// A wildcard tombstone is refused whatever `force` says.
    #[test]
    fn a_wildcard_retire_is_refused_unconditionally() {
        for force in [false, true] {
            let err = check_retire("prod/zk2/host-a/tc/**", force)
                .unwrap_err()
                .to_string();
            assert!(err.contains("blast radius"), "{err}");
        }
    }

    /// #504: the wildcard refusal is one function, and a put meets it as
    /// a tombstone does — `$*` included, a concrete key untouched.
    #[test]
    fn a_wildcard_write_is_refused_whatever_the_act() {
        for key in ["prod/zk2/**", "plant/*/temp", "plant/line-1/$*"] {
            for act in [WriteAct::Put, WriteAct::Retire] {
                let err = check_concrete(key, act).unwrap_err();
                assert!(err.is_unaskable(), "a refused input, exit 2: {err}");
                let err = err.to_string();
                assert!(err.contains("is a wildcard"), "{err}");
                assert!(err.contains("blast radius"), "{err}");
                assert!(err.contains("Not overridable"), "{err}");
            }
        }
        let put = check_concrete("prod/zk2/**", WriteAct::Put)
            .unwrap_err()
            .to_string();
        assert!(put.contains("RFC 07 §3"), "{put}");
        assert!(put.contains("every subscriber"), "{put}");
        assert!(check_concrete("plant/line-1/temp", WriteAct::Put).is_ok());
    }

    /// … and the only publish path asks it, so no frontend can forget to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_wildcard_publication_is_never_declared() {
        let session = crate::bus::session::standalone().await;
        let err = declare_publication(&session, "prod/zk2/**", WireQos::DEFAULT, None)
            .await
            .unwrap_err();
        assert!(err.is_unaskable(), "{err}");
        assert!(err.to_string().contains("blast radius"), "{err}");
    }

    /// A concrete tombstone needs `--i-know`, and says so: no contract
    /// describes the key, so no class licenses the delete.
    #[test]
    fn a_concrete_retire_needs_i_know() {
        let err = check_retire("plant/line-1/temp", false).unwrap_err();
        assert!(err.is_unaskable(), "a refused input, exit 2: {err}");
        assert!(err.to_string().contains("--i-know"), "{err}");
        assert!(check_retire("plant/line-1/temp", true).is_ok());
    }

    /// The default axes are zenoh's own, and every axis set round-trips
    /// through its `qos_axes` token.
    #[test]
    fn the_default_is_zenohs_and_a_token_round_trips() {
        assert_eq!(WireQos::DEFAULT.token(), "data/drop/reliable");
        let urgent = WireQos {
            priority: zenoh::qos::Priority::RealTime,
            congestion: zenoh::qos::CongestionControl::Block,
            reliability: zenoh::qos::Reliability::Reliable,
            express: true,
        };
        assert_eq!(urgent.token(), "real_time/block/reliable+express");
        assert_eq!(WireQos::parse(&urgent.token()), Some(urgent));
        assert_eq!(
            WireQos::parse("sampled"),
            None,
            "a v1 profile name is no token"
        );
    }
}
