//! `freshness.v2`'s owner (`spec/profiles/freshness/v2.md` §2.4, #978's
//! F3): one heartbeat per interface a service serves whose contract uses
//! the profile, which vouches for each member of its collections by a bit
//! in place of `freshness.v1`'s re-puts.
//!
//! ```text
//! ServiceBuilder::implement(c uses freshness.v2) ─ the Store and the Minter cover c's interface;
//!        │                                         freshness.v2's standard contract implemented
//!        │                                         beside it, its `heartbeat/{iface}` exposed
//!        ▼ start(), after the tokens (core §8.2 step 4)
//!   a task per covered interface with a collection:
//!     every REFRESH_LEAD × (its shortest horizon) / 2:
//!       the body from the Store's books (digest, live count, bits; a map bit per
//!       collection the instance exposes), its time from the owner's clock,
//!       sealed as core §7.5's form 6 on the heartbeat key's own stream seq,
//!       put; each member it vouched for confirmed then (its replies' Zenoh
//!       timestamp, S2)
//!        ▼ Service::close: the tasks stop first, then everything else
//! ```
//!
//! - **The key** is `zk2/<system>/<service>/freshness.v2/stream/heartbeat/<iface>`
//!   ([`zenkey_model::heartbeat::key`]), a resource of the standard contract
//!   embedded here ([`BUNDLE`]), with its QoS: reliable, `drop`, priority
//!   `data`.
//! - **A standby sends none**: the writer is declared through the covered
//!   interface's gate, so a governed interface's heartbeat goes out only
//!   while its instance serves it (`redundancy.v1` §2.4).
//! - **The clock guard** (v2 §2.10): while it holds the owner's state
//!   writes, each heartbeat vouches for nothing and says its time is
//!   unknown, so the members go stale by design, as `freshness.v1`'s re-puts
//!   stop.
//! - **The time** is under the deployment epoch, 0 until #980's catalog
//!   carries one (core §7.6), as a packed record's `time` fields are.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use zenkey_model::grammar::IfaceId;
use zenkey_model::heartbeat::{Layout, Time};
use zenoh::key_expr::OwnedKeyExpr;

use crate::implementation::Implementation;
use crate::state::{Minter, REFRESH_LEAD, Store};
use crate::writer::Writer;

/// The standard contract's bundle, revision 2.0 as published in
/// `spec/profiles/.history/freshness.v2/` (v2 §3), byte for byte.
pub const BUNDLE: &[u8] = include_bytes!(
    "../../spec/profiles/.history/freshness.v2/8756e6a64a25424c962af374d844cf56c2ed1e831a4bac349fc17f6c0544a890.bundle.json"
);

/// Its fingerprint (v2 §3).
pub const FINGERPRINT: &str = "8756e6a64a25424c962af374d844cf56c2ed1e831a4bac349fc17f6c0544a890";

/// The heartbeat's resource, as the descriptor names it.
pub const HEARTBEAT: &str = "stream/heartbeat/{iface}";

/// The template's parameter: the covered interface's id.
pub const PARAM: &str = "iface";

/// The deployment epoch a heartbeat's time counts from, in Unix seconds:
/// 0 until #980's catalog carries one (core §7.6, §7.5).
pub const EPOCH: i64 = 0;

static IMPLEMENTATION: LazyLock<Implementation> = LazyLock::new(|| {
    Implementation::from_bundle(BUNDLE).expect("the published freshness.v2 bundle verifies")
});

/// The standard contract's interface id, `freshness.v2`.
#[must_use]
pub fn iface() -> IfaceId {
    zenkey_model::heartbeat::PROFILE
        .parse()
        .expect("freshness.v2 is an interface id")
}

/// The standard contract, implemented from the embedded bundle: what a
/// service whose contract uses `freshness.v2` implements beside it, and
/// what a reader of heartbeats is compiled against.
#[must_use]
pub fn implementation() -> Implementation {
    IMPLEMENTATION.clone()
}

/// How often a covered interface's heartbeat goes out: [`REFRESH_LEAD`] of
/// half its shortest horizon, so a timer's lateness never takes an
/// interval past the bound (v2 §2.4). `None` with no collection, which
/// sends none.
#[must_use]
pub fn period(layout: &Layout) -> Option<Duration> {
    layout
        .horizon()
        .map(|h| (h / 2).mul_f64(REFRESH_LEAD))
        .filter(|p| !p.is_zero())
}

/// One covered interface's heartbeat, as its task sends it.
pub(crate) struct Beat {
    pub(crate) writer: Writer,
    pub(crate) layout: Arc<Layout>,
    /// For each collection in map order, whether this instance exposes it:
    /// its map bit (v2 §2.4). Fixed at start, as exposure is.
    pub(crate) exposed: Vec<bool>,
    pub(crate) store: Arc<Store>,
    pub(crate) minter: Arc<Minter>,
    pub(crate) period: Duration,
}

impl Beat {
    /// Sends one heartbeat now, unless its interface stands by.
    pub(crate) async fn once(&self) {
        // redundancy.v1 §2.4: a standby writes nothing of the interface.
        let Ok(admitted) = self.writer.serving().await else {
            return;
        };
        let held = self.minter.is_ahead();
        let time = if held {
            Time::Unknown
        } else {
            let now = self.minter.now().get_time().to_system_time();
            let ms = now
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i128);
            Time::from_unix_ms(ms, EPOCH).unwrap_or(Time::Unknown)
        };
        let iface = self.layout.iface.to_string();
        let Some((hb, vouched)) = self.store.heartbeat(&iface, &self.exposed, time, !held) else {
            return;
        };
        let key = OwnedKeyExpr::from(self.writer.key_expr().clone().into_owned());
        let stamped = self.minter.mint_stream(&key, self.writer.is_stamped());
        let bytes = match hb.envelope(&self.layout, stamped.stamp) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(%iface, "freshness.v2: a heartbeat could not be written: {e}");
                return;
            }
        };
        match self
            .writer
            .put_stamped(&admitted, bytes.into(), None, stamped)
            .await
        {
            Ok(()) => self.store.confirm(&vouched, self.minter.time()),
            Err(e) => tracing::warn!(%iface, "freshness.v2: a heartbeat failed: {e}"),
        }
    }

    /// Sends one at once, then one every period, until the service closes.
    pub(crate) async fn run(self, mut closed: tokio::sync::watch::Receiver<bool>) {
        loop {
            if *closed.borrow() {
                return;
            }
            self.once().await;
            tokio::select! {
                () = tokio::time::sleep(self.period) => {}
                changed = closed.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        }
    }
}

/// A service's heartbeat tasks: aborted when dropped, so a service dropped
/// unclosed sends none after its tokens go.
#[derive(Default)]
pub(crate) struct Tasks(pub(crate) Vec<tokio::task::JoinHandle<()>>);

impl Tasks {
    /// Stops every task, and waits for each.
    pub(crate) async fn stop(&mut self) {
        for t in std::mem::take(&mut self.0) {
            t.abort();
            let _ = t.await;
        }
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_bundle_is_the_published_standard_contract() {
        let imp = implementation();
        assert_eq!(imp.iface().to_string(), "freshness.v2");
        assert_eq!(imp.fingerprint().hex().as_str(), FINGERPRINT);
        assert!(imp.resource(HEARTBEAT).is_ok());
    }
}
