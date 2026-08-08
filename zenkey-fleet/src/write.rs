//! The write facade (issue #36): the only two ways an explorer writes to the
//! bus — a declared publication, and a disciplined RPC call.
//!
//! Reading stayed the engine's whole job until now; both frontends need the
//! same two write paths (`zenctl topic pub` / `service call`, the zengui
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

use anyhow::{Result, anyhow, bail};
use zenkey::origin::{HostId, ServiceOrigin};
use zenkey::qos::QosProfile;
use zenoh::Session;

use crate::registry::SliceSet;
use crate::report::{CallAnswer, CallError, CallReport};

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
        .map_err(|e| anyhow!("declare publisher {key}: {e}"))?;
    Ok(Publication {
        publisher,
        encoding: encoding.map(str::to_string),
    })
}

impl Publication {
    /// Publish one payload. Sets the wire `Encoding` when one was declared
    /// (RFC 04 v1.5's recommendation: publishers say what they carry).
    pub async fn send(&self, payload: Vec<u8>) -> Result<()> {
        let put = self.publisher.put(payload);
        let put = match &self.encoding {
            Some(e) => put.encoding(e.as_str()),
            None => put,
        };
        put.await
            .map_err(|e| anyhow!("put {}: {e}", self.publisher.key_expr()))
    }

    /// Undeclare, acknowledged.
    pub async fn undeclare(self) -> Result<()> {
        self.publisher
            .undeclare()
            .await
            .map_err(|e| anyhow!("undeclare publisher: {e}"))
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
            return Ok(CallTarget::Service(
                ServiceOrigin::new(s).map_err(|e| anyhow!("{e}"))?,
            ));
        }
        HostId::parse(s)
            .map(CallTarget::Host)
            .map_err(|e| anyhow!("{e} — a hostname is not an origin; resolve it first (RFC 06 §6)"))
    }
}

/// Call a procedure and report every attributed answer.
///
/// - The key composes through the typed builders (never `format!`), lifted to
///   the wire with the configured base.
/// - `params` ride the selector (`?k=v;k=v`), the body rides the payload
///   (RFC 05 §1).
/// - **Fan-out guard**: a [`CallTarget::Fleet`] call is refused when the
///   loaded slices declare the procedure `fanout = "forbidden"`. With no
///   slices loaded the registry layer cannot judge — the call proceeds, and
///   the builder/ACL layers remain (documented, not silent: the report's key
///   is the caller's audit trail).
/// - Exit-code semantics stay on [`CallReport::exit_code`]: an error reply is
///   a failure, zero replies stay a distinct non-verdict (RFC 05 §3.1).
#[allow(clippy::too_many_arguments)]
pub async fn call(
    session: &Session,
    base: &str,
    target: &CallTarget,
    producer: &str,
    procedure: &str,
    params: &[String],
    body: Option<Vec<u8>>,
    timeout: Duration,
    slices: Option<&SliceSet>,
) -> Result<CallReport> {
    if matches!(target, CallTarget::Fleet)
        && let Some(slices) = slices
        && let Some(slice) = slices.get(producer)
        && let Some(proc_decl) = slice.procedures.iter().find(|p| p.path == procedure)
        && proc_decl.fanout.as_deref() == Some("forbidden")
    {
        bail!(
            "procedure {producer}/{procedure} declares fanout = \"forbidden\" — a \
             fleet (`*`) call to it is refused (RFC 05 §2.1); name one origin"
        );
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
    let mut key = zenkey::grammar::with_base(base, relative);
    if !params.is_empty() {
        key.push('?');
        key.push_str(&params.join(";"));
    }

    let answers = crate::query::fleet_get(session, base, &key, body, timeout).await?;
    Ok(CallReport {
        key: key.clone(),
        answers: answers
            .iter()
            .map(|a| match &a.answer {
                crate::query::Answer::Value(bytes) => {
                    let bytes = bytes.to_bytes();
                    match serde_json::from_slice::<serde_json::Value>(&bytes) {
                        Ok(v) => CallAnswer {
                            origin: a.origin.clone(),
                            ok: true,
                            value: Some(v),
                            text: None,
                            error: None,
                        },
                        Err(_) => CallAnswer {
                            origin: a.origin.clone(),
                            ok: true,
                            value: None,
                            text: Some(String::from_utf8_lossy(&bytes).to_string()),
                            error: None,
                        },
                    }
                }
                crate::query::Answer::Error { name, message } => CallAnswer {
                    origin: a.origin.clone(),
                    ok: false,
                    value: None,
                    text: None,
                    error: Some(CallError {
                        name: name.clone(),
                        message: message.clone(),
                    }),
                },
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::slice::{ProcedureDecl, RegistrySlice};

    fn slice_with_proc(fanout: Option<&str>) -> SliceSet {
        SliceSet::from_slices(vec![RegistrySlice {
            version: "1.0".into(),
            app: "t".into(),
            convention: 1,
            name: "netring".into(),
            service_origin: None,
            description: None,
            subjects: vec![],
            procedures: vec![ProcedureDecl {
                path: "capture/trigger".into(),
                kind: "write".into(),
                reply: Some("Ack".into()),
                request: None,
                encoding: None,
                fanout: fanout.map(str::to_string),
                idempotent: Some(false),
                since: None,
                description: None,
            }],
            blob: vec![],
            deprecated: vec![],
        }])
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
        let session = crate::session::open(&[], &[], false).await.unwrap();
        let slices = slice_with_proc(Some("forbidden"));
        let err = call(
            &session,
            "",
            &CallTarget::Fleet,
            "netring",
            "capture/trigger",
            &[],
            None,
            Duration::from_millis(100),
            Some(&slices),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("fanout"), "{err}");
        assert!(err.contains("RFC 05 §2.1"), "{err}");

        // Unconstrained procedures fan out fine (zero replies here — a
        // non-verdict, not an error).
        let report = call(
            &session,
            "",
            &CallTarget::Fleet,
            "netring",
            "capture/trigger",
            &[],
            None,
            Duration::from_millis(100),
            Some(&slice_with_proc(None)),
        )
        .await
        .unwrap();
        assert_eq!(report.exit_code(), 2, "silence stays exit 2");
    }
}
