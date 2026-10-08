//! Calling a zk2 operation (spec §5.1) through the runtime: its `Client` at
//! one address, its `Fleet` over a selection.
//!
//! The runtime holds the discipline — `BestMatching` and `None` for a
//! concrete call (O1), `All` and `None` for a fan-out (O2), a retry only of
//! an idempotent operation and only after silence (O4), every reply
//! attributed by its key (O3, O6). This module plans nothing (that is
//! [`crate::model::target`], before anything is sent) and judges nothing:
//! it turns what came back into an [`OperationReport`], rendering every
//! payload through the revision in hand ([`crate::model::render`]).
//!
//! **Silence is attributed through presence** (O5). For one address the
//! runtime reads it ([`zk2::Client::attribute`]); for a fan-out or a
//! many-reply call this module reads the selection's interface tokens
//! through the crate's one liveliness chokepoint
//! ([`crate::bus::presence::liveliness_read`]), and lists who holds a token
//! and sent no value: each refused or was silent, which no caller can tell
//! apart, because a `reply_err` carries no key (§5.1, "Attribution").

use std::collections::BTreeSet;
use std::time::Duration;

use zenkey_model::contract::{Replies as RepliesKind, Resource};
use zenkey_model::envelope::Envelope;
use zenkey_model::grammar::{ZkKey, parse};
use zenoh::Session;

use crate::model::catalog::Revision;
use crate::model::render::{Member, render_detail, render_with};
use crate::model::target::CallPlan;
use crate::report::{
    CallMode, EnvelopeView, MalformedReply, OperationAnswer, OperationReport, PayloadRendering,
    PresenceAttribution, ReplierView, RepliesView, SelectionPresence, SilenceView,
};
use crate::{Error, Result};

/// One call, planned and encoded.
pub struct OperationCall<'a> {
    pub revision: &'a Revision,
    pub plan: &'a CallPlan,
    /// The request, encoded as the operation's `request` type.
    pub request: Vec<u8>,
    /// How long to wait for replies.
    pub timeout: Duration,
    /// Retries after silence: honoured for an idempotent operation alone
    /// (O4), and never for a fan-out.
    pub retries: u32,
}

/// A runtime error, in the crate's terms: a contract the call breaks is
/// the caller's input; anything else is the bus.
fn runtime(op: &'static str, target: &str, e: zk2::Error) -> Error {
    match e {
        zk2::Error::Contract(_) | zk2::Error::NoResource { .. } | zk2::Error::Key(_) => {
            Error::unaskable(target, e.to_string())
        }
        other => Error::bus(op, target, other),
    }
}

/// Makes the call: through a `Client` when the plan names one concrete key,
/// through a `Fleet` when it fans out. Nothing here refuses a fan-out: the
/// plan already did (O2), and the runtime refuses it again.
pub async fn call(session: &Session, c: OperationCall<'_>) -> Result<OperationReport> {
    let OperationCall {
        revision,
        plan,
        request,
        timeout,
        retries,
    } = c;
    let contract = revision.shared_contract();
    let address = plan.target.address.as_str();
    let name = plan.name.as_str();
    let (mode, selectors, answer) = match &plan.target.concrete {
        Some(addr) if !plan.is_fanout() => {
            let client = zk2::Client::new(session, contract, &[address])
                .map_err(|e| runtime("call", address, e))?
                .with_timeout(timeout)
                .with_retries(retries);
            let key = client
                .key(addr, name, &plan.values)
                .map_err(|e| runtime("call", address, e))?
                .as_str()
                .to_owned();
            let answer = if plan.operation.replies == RepliesKind::Many {
                let replies = client
                    .call_many(addr, name, &plan.values, request)
                    .await
                    .map_err(|e| runtime("call", &key, e))?;
                let presence =
                    selection_presence(session, address, revision, &replies, timeout).await;
                OperationAnswer::Replies {
                    replies: replies_view(revision, &plan.resource, replies, presence),
                }
            } else {
                let outcome = client
                    .call(addr, name, &plan.values, request)
                    .await
                    .map_err(|e| runtime("call", &key, e))?;
                one_reply(revision, &plan.resource, outcome)
            };
            (CallMode::Concrete, vec![key], answer)
        }
        _ => {
            let fleet = zk2::Fleet::new(session, contract, &[address])
                .map_err(|e| runtime("call", address, e))?
                .with_timeout(timeout);
            let selectors = fleet
                .selectors(name, &plan.values)
                .map_err(|e| runtime("call", address, e))?
                .iter()
                .map(|k| k.as_str().to_owned())
                .collect();
            let replies = fleet
                .call(name, &plan.values, request)
                .await
                .map_err(|e| runtime("call", address, e))?;
            let presence = selection_presence(session, address, revision, &replies, timeout).await;
            (
                CallMode::Fanout,
                selectors,
                OperationAnswer::Replies {
                    replies: replies_view(revision, &plan.resource, replies, presence),
                },
            )
        }
    };
    Ok(OperationReport {
        address: address.to_owned(),
        iface: revision.iface().to_string(),
        fingerprint: revision.fingerprint().to_string(),
        operation: name.to_owned(),
        values: plan.values.clone(),
        selectors,
        mode,
        timeout_s: timeout.as_secs_f64(),
        answer,
    })
}

/// A one-reply call's outcome (O5): four cases, kept apart.
fn one_reply(revision: &Revision, r: &Resource, outcome: zk2::Outcome) -> OperationAnswer {
    match outcome {
        zk2::call::Outcome::Value(a) => OperationAnswer::Value {
            reply: reply(revision, &a, Member::Response),
        },
        zk2::call::Outcome::Refused(env) => OperationAnswer::Refused {
            envelope: envelope_view(revision, r, &env),
        },
        zk2::call::Outcome::Malformed(m) => OperationAnswer::Malformed {
            malformed: malformed(&m),
        },
        zk2::call::Outcome::NoAnswer(s) => OperationAnswer::Silent {
            silence: SilenceView {
                attempts: s.attempts,
                transport: s.transport,
                presence: attribution(s.presence),
            },
        },
    }
}

/// The runtime's attribution, as the report spells it.
pub fn attribution(a: zk2::call::Attribution) -> PresenceAttribution {
    use zk2::call::Attribution as A;
    match a {
        A::Present => PresenceAttribution::Present,
        A::InstanceOnly => PresenceAttribution::InstanceOnly,
        A::Absent => PresenceAttribution::NoTokenVisible,
        A::Unknown => PresenceAttribution::Unknown,
        A::Unobservable => PresenceAttribution::Unobservable,
    }
}

/// One reply, rendered through the contract's `member` type.
fn reply(revision: &Revision, a: &zk2::client::Answer, member: Member) -> PayloadRendering {
    let encoding = a.sample.encoding().to_string();
    render_with(
        revision,
        a.key(),
        member,
        Some(&encoding),
        &a.payload().to_bytes(),
    )
}

fn envelope_view(revision: &Revision, r: &Resource, env: &Envelope) -> EnvelopeView {
    EnvelopeView {
        code: env.code.clone(),
        message: env.message.clone(),
        cause: env.cause.clone(),
        detail: env.detail.as_ref().map(|d| render_detail(revision, r, d)),
    }
}

fn malformed(m: &zk2::call::Malformed) -> MalformedReply {
    MalformedReply {
        encoding: m.encoding.clone(),
        error: m.error.to_string(),
    }
}

/// Every reply of a many-reply call or a fan-out, rendered and attributed
/// (O3, O6).
fn replies_view(
    revision: &Revision,
    r: &Resource,
    replies: zk2::client::Replies,
    presence: SelectionPresence,
) -> RepliesView {
    let declared = replies.summary_declared;
    RepliesView {
        summary_declared: declared,
        repliers: replies
            .repliers
            .iter()
            .map(|p| ReplierView {
                address: p.addr.to_string(),
                key: p.key.clone(),
                values: p.params.clone(),
                replies: p
                    .values
                    .iter()
                    .map(|a| reply(revision, a, Member::Response))
                    .collect(),
                summaries: p
                    .summaries
                    .iter()
                    .map(|a| reply(revision, a, Member::Summary))
                    .collect(),
                possibly_partial: declared.then(|| p.summary().is_none()),
            })
            .collect(),
        refusals: replies
            .refusals
            .iter()
            .map(|e| envelope_view(revision, r, e))
            .collect(),
        malformed: replies.malformed.iter().map(malformed).collect(),
        transport: replies.transport,
        discarded: replies.discarded,
        presence,
    }
}

/// Who in the selection holds the interface's token and sent no value
/// (§5.1, "Attribution"): one liveliness read of the selection's interface
/// tokens, through the crate's chokepoint. A read that fails is said, never
/// read as nobody.
async fn selection_presence(
    session: &Session,
    address: &str,
    revision: &Revision,
    replies: &zk2::client::Replies,
    timeout: Duration,
) -> SelectionPresence {
    let (system, service) = address.split_once('/').unwrap_or((address, "*"));
    let selector = format!("zk2/{system}/{service}/@zk/alive/{}/**", revision.iface());
    let heard: BTreeSet<String> = replies
        .repliers
        .iter()
        .map(|p| p.addr.to_string())
        .collect();
    match crate::bus::presence::liveliness_read(session, &selector, timeout).await {
        Ok(read) => {
            let holders: BTreeSet<String> = read
                .keys
                .iter()
                .filter_map(|k| match parse(k) {
                    Ok(ZkKey::Alive { addr, .. }) => Some(addr.to_string()),
                    _ => None,
                })
                .collect();
            SelectionPresence {
                selector,
                complete: read.complete,
                unheard: holders.difference(&heard).cloned().collect(),
                error: None,
            }
        }
        Err(e) => SelectionPresence {
            selector,
            complete: false,
            unheard: Vec::new(),
            error: Some(crate::one_line(&e)),
        },
    }
}
