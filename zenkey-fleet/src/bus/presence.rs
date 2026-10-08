//! Presence (spec §8.1): liveliness reads.
//!
//! **Every liveliness GET this crate issues goes through
//! [`liveliness_read`]**, v1's roster sweeps included, the way every fleet
//! GET goes through [`crate::bus::query::fleet_get`]. It runs on the
//! runtime's unbounded handler ([`zk2::presence::liveliness_read`]): zenoh's
//! default 256-slot handler hangs a liveliness GET on a session that also
//! holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2), and this crate's sessions hold one whenever a
//! [`crate::Monitor`] watches the roster. The spec forbids the default
//! handler there, and the rule lives in one place so a new sweep cannot
//! forget it.

use std::time::Duration;

use zenoh::Session;

use crate::{Error, Result};

/// The one liveliness GET of this crate (spec §8.1): every token matching
/// `selector`, sorted, and whether the GET completed before `timeout`.
///
/// A read that ran to its timeout comes back with `complete: false`: it may
/// have missed tokens, and a caller reports it as possibly incomplete,
/// never as absence (§8.1, O5). Reply errors are skipped; a token carries
/// no payload, so a reply without a key has nothing to say.
pub async fn liveliness_read(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<zk2::presence::PresenceRead> {
    zk2::presence::liveliness_read(session, selector, timeout)
        .await
        .map_err(|e| Error::bus("liveliness get", selector, e))
}
