//! Writing to the bus: the Send pane's publish stream and its operation
//! calls (#175, #184; zk2's since #614, FL10).
//!
//! Everything here goes through the engine — `declare_publication` for a
//! write (P7: a declared publisher, never an ad-hoc put), on the QoS axes the
//! form picked (`WireQos`), and `call_operation` for a call, through the
//! runtime's client (spec §5.1). Nothing on this side encodes: a foreign key
//! has no contract to encode against, so a publish writes the bytes as
//! typed, and a call's request was planned and encoded (`plan_call`,
//! `encode_request`) before its task started — every refusal of the input
//! is the form's, before anything is sent.
//!
//! The Config tool's acts (#768, CD) are changes too: one `config.v1`
//! operation at one address through `zenkey_fleet::call_config`, its
//! request typed and checked by the owner's own validator in the form
//! before the task started ([`ConfigAct`]).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::SystemTime;

use iced::Task;
use zenkey_fleet::report::ConfigChecked;
use zenkey_fleet::{CallPlan, Publication, Revision, WireQos};
use zenkey_model::grammar::Addr;

use crate::configure::{Act, ActLanding};
use crate::message::{Message, PaneMsg, PublishOutcome};
use crate::services::ServiceError;
use crate::view::configure::ConfigMsg;
use crate::view::send::SendMsg;

/// One `config.v1` act, gathered before the future starts. No `Debug`: the
/// request carries a sensitive value as typed, and only `shown` may be
/// printed (§2.15).
pub struct ConfigAct {
    /// The session **in** the deployment's namespace: the operation's key
    /// is base-relative.
    pub session: zenoh::Session,
    pub namespace: String,
    pub addr: Addr,
    pub act: Act,
    /// The `SetRequest` or `ControlRequest` sent.
    pub request: serde_json::Value,
    /// The request as the report shows it, every sensitive value redacted.
    pub shown: serde_json::Value,
    /// The sensitive values sent: taken out of whatever the owner returns.
    pub secrets: Vec<String>,
    /// The parameters the view declares sensitive, by group and name.
    pub sensitive: BTreeSet<(String, String)>,
    pub timeout: std::time::Duration,
    /// What a `set` read and checked first.
    pub checked: Option<ConfigChecked>,
}

/// The call metadata a Config act claims (core O7): the operator's login
/// as the actor, a fresh ULID as the request id — claimed, never
/// authentication, as `zenctl config` claims it.
fn claimed() -> zenkey::CallMetadata {
    zenkey::CallMetadata {
        actor: std::env::var("USER").ok().filter(|u| !u.is_empty()),
        request_id: Some(zenkey::writer::ulid(SystemTime::now())),
    }
}

/// One act through the runtime's client (O1: one address, `BestMatching`;
/// none is idempotent, so none is retried, O4), landing on the Config tool
/// with the address and namespace it ran against. The report keeps the
/// reply union, the envelope and silence apart (O5); an `Err` is the
/// runtime or the bus failing, never silence.
pub fn config(c: ConfigAct) -> Task<Message> {
    Task::perform(
        async move {
            let ConfigAct {
                session,
                namespace,
                addr,
                act,
                request,
                shown,
                secrets,
                sensitive,
                timeout,
                checked,
            } = c;
            let result = zenkey_fleet::call_config(
                &session,
                zenkey_fleet::ConfigCall {
                    namespace: &namespace,
                    address: &addr,
                    op: act.op(),
                    request,
                    shown,
                    secrets,
                    sensitive,
                    metadata: Some(claimed()),
                    timeout,
                    across_face: false,
                    checked,
                },
            )
            .await
            .map(Arc::new)
            .map_err(ServiceError::of);
            ActLanding {
                addr,
                namespace,
                act,
                at: SystemTime::now(),
                result,
            }
        },
        |l| Message::Pane(PaneMsg::Config(ConfigMsg::Answered(l))),
    )
}

/// One planned call, gathered before the future starts: the revision it was
/// planned against, the plan (`plan_call` already refused a forbidden
/// fan-out, O2), the request's bytes and how long to wait.
pub struct Call {
    /// The session **in** the deployment's namespace: an operation's key is
    /// base-relative, and the runtime's client speaks it there.
    pub session: zenoh::Session,
    pub revision: Arc<Revision>,
    pub plan: CallPlan,
    pub request: Vec<u8>,
    pub timeout: std::time::Duration,
}

/// One call through the runtime's client (one address) or fleet (an
/// allowed fan-out), landing on the Send tool. The report keeps a value, an
/// envelope and silence apart (O5); an `Err` is the bus failing, never
/// silence.
pub fn call(c: Call) -> Task<Message> {
    Task::perform(
        async move {
            let Call {
                session,
                revision,
                plan,
                request,
                timeout,
            } = c;
            zenkey_fleet::call_operation(
                &session,
                zenkey_fleet::OperationCall {
                    revision: &revision,
                    plan: &plan,
                    request,
                    timeout,
                    // A tool's call is asked once: a retry is the operator's
                    // to make, by calling again.
                    retries: 0,
                },
            )
            .await
            .map(Arc::new)
            .map_err(ServiceError::of)
        },
        |r| Message::Pane(PaneMsg::Send(SendMsg::Done(r))),
    )
}

/// What one `Send` needs, gathered from the form before the future starts.
pub struct Publish {
    /// The session in **no** namespace: the key is a full wire key.
    pub session: zenoh::Session,
    pub key: String,
    /// The wire `Encoding`, when the form named one.
    pub encoding: Option<String>,
    /// The bytes, as typed.
    pub body: Arc<Vec<u8>>,
    pub qos: WireQos,
    pub attachment: Option<Arc<Vec<u8>>>,
    pub repeat: bool,
}

/// Declare, send — and hand the declaration back only if it repeats.
pub fn publish(p: Publish) -> Task<Message> {
    Task::perform(
        async move {
            let Publish {
                session,
                key,
                encoding,
                body,
                qos,
                attachment,
                repeat,
            } = p;
            let publication =
                zenkey_fleet::declare_publication(&session, &key, qos, encoding.as_deref())
                    .await
                    .map_err(ServiceError::of)?;
            publication
                .send(
                    body.as_ref().clone(),
                    attachment.as_ref().map(|a| a.as_ref().clone()),
                )
                .await
                .map_err(ServiceError::of)?;
            // The badge is a routing fact about this publisher and nothing
            // else; an error asking is "not asked" (O4), never "nobody
            // listens".
            let matching = publication.matching_status().await.ok();
            // A one-shot undeclares here, acknowledged, exactly as
            // `zenctl pub` does; only a repeating publish hands the
            // declaration back to be held.
            let publication = if repeat {
                Some(Arc::new(publication))
            } else {
                publication.undeclare().await.map_err(ServiceError::of)?;
                None
            };
            Ok(Arc::new(PublishOutcome {
                key,
                bytes: body,
                qos,
                encoding,
                publication,
                matching,
                attachment,
            }))
        },
        |r| Message::Pane(PaneMsg::Send(SendMsg::Ready(r))),
    )
}

/// One beat of an armed repeating publish.
pub fn repeat(
    publication: Arc<Publication>,
    bytes: Arc<Vec<u8>>,
    attachment: Option<Arc<Vec<u8>>>,
) -> Task<Message> {
    Task::perform(
        async move {
            publication
                .send(
                    bytes.as_ref().clone(),
                    attachment.as_ref().map(|a| a.as_ref().clone()),
                )
                .await
                .map(|()| bytes.len())
                .map_err(ServiceError::of)
        },
        |r| Message::Pane(PaneMsg::Send(SendMsg::Sent(r))),
    )
}

/// Acknowledged undeclare. A drop would undeclare too, but silently.
pub fn undeclare(publication: Publication) -> Task<Message> {
    Task::perform(
        async move { publication.undeclare().await.map_err(ServiceError::of) },
        |r| Message::Pane(PaneMsg::Send(SendMsg::Stopped(r))),
    )
}

/// Retire a key: a tombstone (RFC 04 §1.2) on the axes the form picked. The
/// engine has already judged whether this key may be retired
/// (`check_retire`, behind `--i-know`); this is the write.
pub fn retire(session: zenoh::Session, key: String, qos: WireQos) -> Task<Message> {
    Task::perform(
        async move {
            let publication = zenkey_fleet::declare_publication(&session, &key, qos, None)
                .await
                .map_err(ServiceError::of)?;
            let matching = publication.matching_status().await.ok();
            publication.retire().await.map_err(ServiceError::of)?;
            publication.undeclare().await.map_err(ServiceError::of)?;
            Ok(matching)
        },
        |r| Message::Pane(PaneMsg::Send(SendMsg::Retired(r))),
    )
}
