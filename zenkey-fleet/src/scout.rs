//! Raw scouting: the Hello layer, below sessions and liveliness (#116).
//!
//! `discover` answers "which deployments hold liveliness tokens or storages"
//! — a question that presumes a working session. Scouting answers the two
//! questions that come *before* one: "is anything out there at all" and "is
//! multicast scouting working on this segment". It is a third, independent
//! signal, not a replacement for either.
//!
//! Multicast is deliberately **on** here — scouting is what this module *is*.
//! The session-opening default stays off (see `session::open`'s contamination
//! warning): a scout only listens for Hellos and joins nothing.

use anyhow::{Context, Result};
use zenoh::config::WhatAmIMatcher;
use zenoh::handlers::FifoChannelHandler;
use zenoh::scouting::Hello;

/// One Hello, owned — zenoh's [`Hello`] is a wrapper we flatten so callers
/// (a CLI row, a widget) hold plain strings, serialized as they render.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HelloView {
    /// The node's Zenoh id, hex-formatted.
    pub zid: String,
    /// What the node says it is: `router`, `peer`, or `client`.
    pub whatami: String,
    /// The locators the node advertises, verbatim.
    pub locators: Vec<String>,
}

impl HelloView {
    fn of(hello: &Hello) -> Self {
        HelloView {
            zid: hello.zid().to_string(),
            whatami: hello.whatami().to_string(),
            locators: hello.locators().iter().map(|l| l.to_string()).collect(),
        }
    }
}

/// A running scout. Hellos arrive as the segment answers; the caller owns the
/// deadline (wrap [`ScoutStream::recv`] in a timeout), because how long to
/// listen is a question about the network, not about this crate.
pub struct ScoutStream {
    inner: zenoh::scouting::Scout<FifoChannelHandler<Hello>>,
}

impl std::fmt::Debug for ScoutStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScoutStream").finish_non_exhaustive()
    }
}

impl ScoutStream {
    /// The next Hello, or `None` once the scout has stopped.
    pub async fn recv(&self) -> Option<HelloView> {
        self.inner
            .recv_async()
            .await
            .ok()
            .map(|h| HelloView::of(&h))
    }

    /// Stop scouting, explicitly — a drop would stop it too, but silently.
    pub fn stop(self) {
        self.inner.stop();
    }
}

/// Listen for scouting Hellos: multicast on, plus gossip via any `connect`
/// endpoints, so a segment with filtered multicast can still answer.
///
/// `what` filters by advertised kind; combine with `|`
/// (`WhatAmI::Router | WhatAmI::Peer`) or pass a parsed [`WhatAmIMatcher`].
pub async fn scout(
    what: WhatAmIMatcher,
    connect: &[String],
    listen: &[String],
) -> Result<ScoutStream> {
    let config = crate::session::explorer_config(connect, listen, true);
    let inner = zenoh::scout(what, config)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("failed to start scouting")?;
    Ok(ScoutStream { inner })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CLI's ndjson row and the GUI both hang off this exact shape; a
    /// field rename here is a wire-format change for every script piping
    /// `zenctl scout --format ndjson`.
    #[test]
    fn a_hello_serializes_flat_and_stable() {
        let view = HelloView {
            zid: "a1b2".into(),
            whatami: "router".into(),
            locators: vec!["tcp/10.0.0.1:7447".into()],
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "zid": "a1b2",
                "whatami": "router",
                "locators": ["tcp/10.0.0.1:7447"],
            })
        );
    }
}
