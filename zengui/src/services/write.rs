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
//! is the form's, before anything is sent. The one v1 call left is the
//! Config tool's ([`V1Call`]), which leaves with it in FL11.

use std::sync::Arc;

use iced::Task;
use zenkey_fleet::{CallPlan, Publication, Revision, WireQos};

use crate::message::{Message, PaneMsg, PublishOutcome};
use crate::services::ServiceError;
use crate::view::send::SendMsg;

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

/// One v1 `@rpc` call, as the Config tool (#481) still makes it — the last
/// v1 call in the window, and it leaves with that tool (FL11; config.v1
/// does not exist, #613). The Send pane calls zk2 operations ([`Call`]).
///
/// A struct rather than ten positional parameters (#360): `base`, `target`,
/// `producer` and `procedure` are four adjacent `String`s, and any permutation
/// of them compiles into a call to the wrong thing.
pub struct V1Call {
    pub session: zenoh::Session,
    pub base: String,
    /// The call target, as typed — the engine parses it (a bad target is an
    /// answer, not a panic).
    pub target: String,
    pub producer: String,
    pub procedure: String,
    pub params: Vec<String>,
    pub body: Option<Vec<u8>>,
    pub timeout: std::time::Duration,
}

/// One v1 call, landing wherever `land` says (#481: the Config tool's calls
/// are answered into its own form).
pub fn v1_call_then(
    c: V1Call,
    land: impl FnOnce(Result<Arc<zenkey_fleet_v1::report::CallReport>, ServiceError>) -> Message
    + Send
    + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            let V1Call {
                session,
                base,
                target,
                producer,
                procedure,
                params,
                body,
                timeout,
            } = c;
            let target = zenkey_fleet_v1::CallTarget::parse(&target).map_err(ServiceError::of)?;
            zenkey_fleet_v1::call(
                &zenkey_fleet_v1::Fleet::new(&session, &base),
                zenkey_fleet_v1::CallSpec {
                    target: &target,
                    producer: &producer,
                    procedure: &procedure,
                    params: &params,
                    body,
                    attachment: None,
                    timeout,
                    slices: None,
                    force: false,
                },
            )
            .await
            .map(Arc::new)
            .map_err(ServiceError::of)
        },
        land,
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
