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
pub async fn declare_publication(
    session: &Session,
    key: &str,
    qos: QosProfile,
    encoding: Option<&str>,
) -> Result<Publication> {
    let publisher = session
        .declare_publisher(key.to_string())
        .reliability(qos.reliability())
        .congestion_control(qos.congestion_control())
        .priority(qos.priority())
        .express(qos.express())
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
    if key.contains('*') || key.contains('$') {
        return Err(Error::unaskable(
            key,
            "is a wildcard — a tombstone is addressed to one concrete key; a \
             wildcard delete is not an operator act, it is a blast radius \
             (RFC 04 §1.2, v1.12). Not overridable.",
        ));
    }
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
    /// The loaded registry, for the fan-out guard below. `None` = none
    /// loaded, and the guard says so rather than judging.
    pub slices: Option<&'a SliceSet>,
}

/// Call a procedure and report every attributed answer.
///
/// - The key composes through the typed builders (never `format!`), lifted to
///   the wire with the configured base.
/// - `params` ride the selector (`?k=v;k=v`), the body rides the payload
///   (RFC 05 §1).
/// - **Fan-out guard**: a [`CallTarget::Fleet`] call is refused when the
///   loaded slices declare the procedure `fanout = "forbidden"` — or when a
///   `kind = "write"` procedure declares nothing, because RFC 08 §2 defaults
///   a write to forbidden and introspect serves the TOML verbatim. With no
///   slices loaded the registry layer cannot judge — the call proceeds, and
///   the builder/ACL layers remain (documented, not silent: the report's key
///   is the caller's audit trail).
/// - Exit-code semantics stay on [`CallReport::exit_code`]: an error reply is
///   a failure, zero replies stay a distinct non-verdict (RFC 05 §3.1).
pub async fn call(fleet: &crate::Fleet<'_>, spec: CallSpec<'_>) -> Result<CallReport> {
    let CallSpec {
        target,
        producer,
        procedure,
        params,
        body,
        attachment,
        timeout,
        slices,
    } = spec;
    if matches!(target, CallTarget::Fleet)
        && let Some(slices) = slices
        && let Some(slice) = slices.get(producer)
        && let Some(proc_decl) = slice.procedures.iter().find(|p| p.path == procedure)
    {
        // Introspect serves the TOML verbatim, so an omitted `fanout` reaches
        // this layer as `None` — and RFC 08 §2 *defaults* a `kind = "write"`
        // procedure to forbidden. The default has to be applied here, or a
        // dynamic caller fans out a write the generated builders refuse to
        // spell.
        let forbidden = match proc_decl.fanout.as_ref().and_then(Declared::known) {
            Some(Fanout::Forbidden) => true,
            Some(Fanout::Allowed) => false,
            // An unrecognised token is not a licence: RFC 08 §2 defaults a
            // `write` to forbidden, and a `fanout` spelling this build cannot
            // read is exactly the case where guessing "allowed" would fan out
            // a write the generated builders refuse to spell.
            None => matches!(
                proc_decl.kind.as_ref().and_then(Declared::known),
                Some(ProcedureKind::Write)
            ),
        };
        if forbidden {
            let declared = if proc_decl.fanout.is_some() {
                "declares fanout = \"forbidden\""
            } else {
                "is a write with no declared fanout, which defaults to forbidden (RFC 08 §2)"
            };
            return Err(Error::unaskable(
                format!("procedure {producer}/{procedure}"),
                format!(
                    "{declared} — a fleet (`*`) call to it is refused \
                     (RFC 05 §2.1); name one origin"
                ),
            ));
        }
    }

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
    Ok(CallReport {
        key: key.clone(),
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
    })
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

    /// The registry layer of the three-layer refusal: a fleet call to a
    /// declared forbidden-fanout write never leaves the process.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fleet_calls_to_forbidden_fanout_are_refused() {
        let session = crate::bus::session::open(&[], &[], false).await.unwrap();
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
            },
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("defaults to forbidden"), "{err}");
        assert!(err.contains("RFC 08 §2"), "{err}");
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
                },
            )
            .await
            .unwrap();
            assert_eq!(report.exit_code(), 2, "silence stays exit 2");
        }
    }
}
