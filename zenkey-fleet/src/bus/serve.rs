//! A mock responder for the dev loop (#121): stand up a queryable so a
//! consumer can be tested — the explorers could ask and observe but never
//! answer.
//!
//! The declaration discipline lives here, not in a frontend: one declared
//! queryable, an acknowledged undeclare, and every incoming query surfaced
//! as an owned view — the log of who asked *is* half the feature, doubling
//! as a "who queries this key" probe.
//!
//! Deliberately no reply scripting: static bytes only. nuze and zsak own
//! the embedded-language lane, at the cost of a Nushell dependency and a
//! linked libpython; the shell covers dynamic cases by restarting the
//! responder.

use anyhow::{Result, anyhow};
use zenoh::Session;
use zenoh::handlers::FifoChannelHandler;

/// One incoming query, owned — what the responder answered, and what a
/// frontend logs.
#[derive(Debug, Clone)]
pub struct ServedQuery {
    /// The query's full selector, verbatim (key expression + parameters).
    pub selector: String,
    /// The parameters half alone, verbatim ("" when none).
    pub parameters: String,
    /// The query body, when it carried one (refcounted, like a payload).
    pub payload: Option<zenoh::bytes::ZBytes>,
    /// The body's declared encoding, when said.
    pub encoding: Option<String>,
    /// The query's attachment, when it carried one (#117).
    pub attachment: Option<zenoh::bytes::ZBytes>,
    /// Why the reply failed to send, when it did — `None` is a sent reply.
    ///
    /// Surfaced, not swallowed (deep-review D7): the ask itself is still a
    /// fact worth logging either way, but a responder whose answers never
    /// leave the process must say so, or its log reads as service
    /// (RFC 05 §3.1 — silence needs attribution, on the answering side
    /// too).
    pub reply_error: Option<String>,
}

/// A declared queryable answering every query with one static body.
pub struct MockResponder {
    queryable: zenoh::query::Queryable<FifoChannelHandler<zenoh::query::Query>>,
    /// The declared expression — the responder's own key when concrete
    /// (RFC 05 §2.1: replies ride the responder's key, G-05b).
    keyexpr: String,
    /// Whether `keyexpr` is concrete (no wildcards) — decided once at
    /// declaration.
    concrete: bool,
    reply: Vec<u8>,
    encoding: Option<String>,
}

impl std::fmt::Debug for MockResponder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockResponder").finish_non_exhaustive()
    }
}

/// Declare the responder on `keyexpr` (a full wire expression — explorers
/// are un-namespaced). `complete` sets zenoh's completeness flag: a claim
/// that this responder holds *all* the data the expression names — say it
/// only when you mean it.
pub async fn declare_responder(
    session: &Session,
    keyexpr: &str,
    reply: Vec<u8>,
    encoding: Option<&str>,
    complete: bool,
) -> Result<MockResponder> {
    let parsed = zenoh::key_expr::KeyExpr::try_from(keyexpr.to_string())
        .map_err(|e| anyhow!("declare queryable {keyexpr}: {e}"))?;
    let concrete = !parsed.is_wild();
    let queryable = session
        .declare_queryable(parsed)
        .complete(complete)
        .await
        .map_err(|e| anyhow!("declare queryable {keyexpr}: {e}"))?;
    Ok(MockResponder {
        queryable,
        keyexpr: keyexpr.to_string(),
        concrete,
        reply,
        encoding: encoding.map(str::to_string),
    })
}

impl MockResponder {
    /// The next query, or `None` once the queryable is gone.
    ///
    /// **One await, and it consumes nothing it does not hand back** (#333).
    /// Receiving and answering used to be a single `next()` with an await at
    /// each end: a caller that raced it against anything — the moment a `--for`
    /// deadline or a count timeout joins the `select!` that today holds only
    /// `ctrl_c` — could be dropped between the two, and the query was then
    /// taken off the channel, never answered, and never logged. Invisible on
    /// both sides: the asker sees silence it cannot attribute (RFC 05 §3.1),
    /// and the responder's log — half the point of this type — never records
    /// the ask at all (RFC 13 §3 O6).
    ///
    /// The split is [`crate::bus::producer::Responder`]'s, right next door.
    /// Hand what this returns to [`answer`](Self::answer): a query in the
    /// caller's hand can still be answered after a cancelled poll, and one
    /// never received was never taken.
    pub async fn next(&self) -> Option<zenoh::query::Query> {
        self.queryable.recv_async().await.ok()
    }

    /// Answer one query and return its view.
    ///
    /// The reply is addressed to the responder's **own declared key** when
    /// that key is concrete (RFC 05 §2.1: attribution and consolidation both
    /// read the reply key, so echoing a wildcard selector back — the G-05b
    /// violation this used to commit — collapses a mocked fleet to one
    /// surviving reply). A responder declared on a wildcard has no own
    /// concrete key; it falls back to the query's key, which is concrete
    /// exactly when the asker named a real key. An error on the reply path
    /// rides the view ([`ServedQuery::reply_error`]) at the caller's log, not
    /// silently.
    pub async fn answer(&self, query: zenoh::query::Query) -> ServedQuery {
        let mut view = ServedQuery {
            selector: query.selector().to_string(),
            parameters: query.parameters().to_string(),
            payload: query.payload().cloned(),
            encoding: query.encoding().map(|e| e.to_string()),
            attachment: query.attachment().cloned(),
            reply_error: None,
        };
        let key = if self.concrete {
            self.keyexpr.clone()
        } else {
            query.key_expr().to_string()
        };
        let reply = query.reply(key, self.reply.clone());
        let reply = match &self.encoding {
            Some(e) => reply.encoding(e.as_str()),
            None => reply,
        };
        // The view still surfaces on failure so the log records the ask —
        // but a reply that never left carries its reason with it (D7).
        view.reply_error = reply.await.err().map(|e| e.to_string());
        view
    }

    /// Undeclare, acknowledged — like the write facade's publications.
    pub async fn undeclare(self) -> Result<()> {
        self.queryable
            .undeclare()
            .await
            .map_err(|e| anyhow!("undeclare queryable: {e}"))
    }
}
