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
//! - **The clock guard** (v2 §2.10, #978's F6): on a session without
//!   timestamping it holds no write of a covered member, which carries no
//!   Zenoh timestamp, so each heartbeat vouches as at any other time. On a
//!   session that stamps every write, a `redundancy.v1` candidate's (core
//!   §8.2), it holds them all, and a held write clears its member's bit
//!   until a write of it goes through (F-461). Its time is unknown while
//!   the guard holds: the owner's clock is ahead, and the time is its word
//!   of it.
//! - **The time** counts from the deployment epoch (core §7.6, §8.6;
//!   v2 §2.4, #1080): the catalog's the service was given, the one its
//!   frames and packed records count from, else 0 (Unix seconds). An
//!   instant with no code under it, before the epoch or 2^32 s or more
//!   after it, is written as the unknown time, all ones.

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
/// The crate reads it through `bundles/`, a link to that file, so its
/// package carries the bytes (#929).
pub const BUNDLE: &[u8] = include_bytes!(
    "../bundles/freshness.v2/8756e6a64a25424c962af374d844cf56c2ed1e831a4bac349fc17f6c0544a890.bundle.json"
);

/// Its fingerprint (v2 §3).
pub const FINGERPRINT: &str = "8756e6a64a25424c962af374d844cf56c2ed1e831a4bac349fc17f6c0544a890";

/// The heartbeat's resource, as the descriptor names it.
pub const HEARTBEAT: &str = "stream/heartbeat/{iface}";

/// The template's parameter: the covered interface's id.
pub const PARAM: &str = "iface";

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

/// A heartbeat's time for the owner's clock reading `wall`, under the
/// deployment epoch `epoch` in Unix seconds (v2 §2.4): unknown, all ones,
/// for an instant with no code, before the epoch or 2^32 s or more after
/// it (core §7.6), a clock before the Unix epoch included.
#[must_use]
pub(crate) fn time_at(wall: std::time::SystemTime, epoch: i64) -> Time {
    wall.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_millis()).ok())
        .and_then(|ms| Time::from_unix_ms(ms, epoch))
        .unwrap_or(Time::Unknown)
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
    /// The deployment epoch its time counts from, in Unix seconds: the
    /// service's ([`crate::Service::deployment_epoch`]), as its frames'
    /// (v2 §2.4).
    pub(crate) epoch: i64,
}

impl Beat {
    /// Sends one heartbeat now, unless its interface stands by.
    pub(crate) async fn once(&self) {
        // redundancy.v1 §2.4: a standby writes nothing of the interface.
        let Ok(admitted) = self.writer.serving().await else {
            return;
        };
        // One instant, taken before the put: the body's time, and the last
        // confirmation the replies of each member it vouches for carry
        // (S2), so a reply never claims a confirmation later than the
        // heartbeat's. While the clock guard holds, the time is unknown
        // (v2 §2.10), and the replies' stamp is from the clock ahead.
        let at = self.minter.time();
        let time = if self.minter.is_ahead() {
            Time::Unknown
        } else {
            time_at(at.get_time().to_system_time(), self.epoch)
        };
        let iface = self.layout.iface.to_string();
        let Some((hb, vouched)) = self.store.heartbeat(&iface, &self.exposed, time) else {
            return;
        };
        // The body before the seq: a heartbeat that cannot be written takes
        // none, so its readers read no gap (§2.5).
        let body = match hb.write(&self.layout) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(%iface, "freshness.v2: a heartbeat could not be written: {e}");
                return;
            }
        };
        let key = OwnedKeyExpr::from(self.writer.key_expr().clone().into_owned());
        let stamped = self.minter.mint_stream(&key, self.writer.is_stamped());
        let bytes = zenkey_model::wire::Envelope {
            form: zenkey_model::wire::Form::Heartbeat,
            tag: self.layout.tag,
            stamp: stamped.stamp,
            record: &body,
        }
        .write();
        match self
            .writer
            .put_stamped(&admitted, bytes.into(), None, stamped)
            .await
        {
            Ok(()) => self.store.confirm(&vouched, at),
            Err(e) => {
                // A put never sent gives its seq back, so the next heartbeat
                // follows the last one sent in sequence.
                self.minter.unmint_stream(&key, stamped.stamp);
                tracing::warn!(%iface, "freshness.v2: a heartbeat failed: {e}");
            }
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

    /// v2 §2.4 (#1080): the time counts from the epoch given, and an
    /// instant with no code under it is the unknown time, as
    /// `spec/conformance/heartbeat/times.json` writes it.
    #[test]
    fn a_time_counts_from_the_epoch_and_one_with_no_code_is_unknown() {
        use std::time::{Duration, UNIX_EPOCH};
        let e = 1_767_225_600;
        let at = |ms: u64| UNIX_EPOCH + Duration::from_millis(ms);
        assert_eq!(
            time_at(at(1_794_062_759_123), e),
            Time::At {
                secs: 26_837_159,
                ms: 123
            }
        );
        assert_eq!(
            time_at(at(1_794_062_759_123), 0),
            Time::At {
                secs: 1_794_062_759,
                ms: 123
            },
            "no catalog: Unix seconds"
        );
        assert_eq!(time_at(at(1_767_225_599_999), e), Time::Unknown);
        assert_eq!(time_at(at(0), e), Time::Unknown, "a clock with no RTC");
        assert_eq!(
            time_at(at(6_062_192_896_000), e),
            Time::Unknown,
            "2^32 s on"
        );
        assert_eq!(
            time_at(UNIX_EPOCH - Duration::from_millis(1), 0),
            Time::Unknown,
            "before the Unix epoch, with no catalog"
        );
    }

    #[test]
    fn the_embedded_bundle_is_the_published_standard_contract() {
        let imp = implementation();
        assert_eq!(imp.iface().to_string(), "freshness.v2");
        assert_eq!(imp.fingerprint().hex().as_str(), FINGERPRINT);
        assert!(imp.resource(HEARTBEAT).is_ok());
    }

    /// A seq minted for a heartbeat never sent is given back, so the next
    /// one carries it and its readers read no gap; one minted since, or a
    /// run moved on, keeps the gap.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_seq_minted_for_a_put_never_sent_is_given_back() {
        let mut c = zenoh::Config::default();
        c.insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        c.insert_json5("listen/endpoints", "[]").unwrap();
        let session = zenoh::open(c).await.unwrap();
        let minter = Minter::new(&session);
        let key = OwnedKeyExpr::try_from("zk2/lab/b/freshness.v2/stream/heartbeat/b.v2").unwrap();
        let first = minter.mint_stream(&key, false).stamp;
        let lost = minter.mint_stream(&key, false).stamp;
        assert_eq!(lost.seq.get(), first.seq.get() + 1);
        assert!(minter.unmint_stream(&key, lost));
        assert_eq!(
            minter.mint_stream(&key, false).stamp,
            lost,
            "the next put takes it again"
        );
        let third = minter.mint_stream(&key, false).stamp;
        assert!(
            !minter.unmint_stream(&key, lost),
            "minted since: the gap stands"
        );
        assert_eq!(
            minter.mint_stream(&key, false).stamp.seq.get(),
            third.seq.get() + 1
        );
    }
}
