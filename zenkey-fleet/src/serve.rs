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
}

/// A declared queryable answering every query with one static body.
pub struct MockResponder {
    queryable: zenoh::query::Queryable<FifoChannelHandler<zenoh::query::Query>>,
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
    let queryable = session
        .declare_queryable(keyexpr.to_string())
        .complete(complete)
        .await
        .map_err(|e| anyhow!("declare queryable {keyexpr}: {e}"))?;
    Ok(MockResponder {
        queryable,
        reply,
        encoding: encoding.map(str::to_string),
    })
}

impl MockResponder {
    /// Answer the next query and return its view, or `None` once the
    /// queryable is gone. The reply is addressed to the query's own key —
    /// concrete where the query was concrete — and errors on the reply
    /// path surface in the view's place at the caller's log, not silently.
    pub async fn next(&self) -> Option<ServedQuery> {
        let query = self.queryable.recv_async().await.ok()?;
        let view = ServedQuery {
            selector: query.selector().to_string(),
            parameters: query.parameters().to_string(),
            payload: query.payload().cloned(),
            encoding: query.encoding().map(|e| e.to_string()),
            attachment: query.attachment().cloned(),
        };
        let key = query.key_expr().clone();
        let reply = query.reply(key, self.reply.clone());
        let reply = match &self.encoding {
            Some(e) => reply.encoding(e.as_str()),
            None => reply,
        };
        // A failed reply is the *asker's* silence, not ours to hide — but
        // the view still surfaces so the log records the ask.
        let _ = reply.await;
        Some(view)
    }

    /// Undeclare, acknowledged — like the write facade's publications.
    pub async fn undeclare(self) -> Result<()> {
        self.queryable
            .undeclare()
            .await
            .map_err(|e| anyhow!("undeclare queryable: {e}"))
    }
}
