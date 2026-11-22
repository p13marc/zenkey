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
//!   until a write of it goes through (F-461). Its time is core §7.5's
//!   ahead time while the guard holds, on a heartbeat and on an owner
//!   heartbeat, whatever the clock reads (v2 0.12, core 0.113; #986's Q0):
//!   the owner's clock is ahead, and the time is its word of it. It is
//!   never the unknown time, which says only that the clock reads no
//!   instant the epoch codes.
//! - **The owner heartbeat** (v2 §2.4, text 0.9; #1110's H3), behind
//!   [`crate::ServiceConfig::owner_heartbeat()`], off unless the deployment
//!   sets it: one put per class of exposure on
//!   `…/freshness.v2/stream/heartbeats/<class>` (`OwnerBeat`), a group
//!   for each interface the service implements that its catalog lists and
//!   that has a collection of the class, in ascending order of their
//!   indices. A group's map holds the collections of the key's class
//!   alone. A covered interface runs no task of its own (never both
//!   forms), and an interface the catalog does not list, or a sixteenth
//!   on a key, keeps its heartbeat. A governed interface's group is left
//!   out while its instance stands by, asked at once and never waited for,
//!   so the class key keeps its period with the groups beside it; a put
//!   that would hold no group is not sent. Every [`REFRESH_LEAD`] × half the shortest horizon among
//!   the collections of the key's class, on the key's own stream seq, in
//!   the run the descriptor states: a reader finds the owner's catalog by
//!   it.
//! - **The time** counts from the deployment epoch (core §7.6, §8.6;
//!   v2 §2.4, #1080): the catalog's the service was given, the one its
//!   frames and packed records count from, else 0 (Unix seconds). An
//!   instant with no code under it, before the epoch or 2^32 s or more
//!   after it, is written as the unknown time, all ones, while the guard
//!   does not hold.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use zenkey_model::grammar::IfaceId;
use zenkey_model::heartbeat::{Class, Group, Layout, OwnerHeartbeat, Time};
use zenoh::key_expr::OwnedKeyExpr;

use crate::gate::Gate;
use crate::implementation::Implementation;
use crate::state::{Minter, REFRESH_LEAD, Store};
use crate::writer::Writer;

/// The standard contract's bundle, revision 1.1 as published in
/// `spec/profiles/.history/freshness.v2/` (v2 §3), byte for byte: 1.0's
/// `heartbeat/{iface}` and the optional `heartbeats/{class}`, the owner
/// heartbeat's key (text 0.9; #1110).
/// The crate reads it through `bundles/`, a link to that file, so its
/// package carries the bytes (#929).
pub const BUNDLE: &[u8] = include_bytes!(
    "../bundles/freshness.v2/9e07704d114d5124a0d177a003c125ec01b80da0f3f1bad594ab300e0ae1e5d1.bundle.json"
);

/// Its fingerprint (v2 §3).
pub const FINGERPRINT: &str = "9e07704d114d5124a0d177a003c125ec01b80da0f3f1bad594ab300e0ae1e5d1";

/// The heartbeat's resource, as the descriptor names it.
pub const HEARTBEAT: &str = "stream/heartbeat/{iface}";

/// The template's parameter: the covered interface's id.
pub const PARAM: &str = "iface";

/// The owner heartbeat's resource (revision 1.1, optional), as the
/// descriptor names it. A service exposes it while its owner heartbeats
/// cover an interface ([`crate::ServiceConfig::owner_heartbeat()`]), and
/// otherwise lists it unavailable, cause `config`: core §3.3's cause says
/// "why the resource is absent here", and here it is the deployment's
/// configuration that leaves it out, the option or the catalog.
pub const OWNER_HEARTBEATS: &str = "stream/heartbeats/{class}";

/// Its template's parameter: the class, `link` or `host`.
pub const OWNER_PARAM: &str = "class";

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

/// How often a class's owner heartbeat goes out (v2 §2.4, "The period"):
/// [`REFRESH_LEAD`] of half the shortest horizon among the collections of
/// `class` in `layouts`, the revisions its groups cover. `None` where none
/// of them has a collection of the class, which sends none.
#[must_use]
pub fn owner_period<'a>(
    layouts: impl IntoIterator<Item = &'a Layout>,
    class: Class,
) -> Option<Duration> {
    layouts
        .into_iter()
        .filter_map(|l| l.class_horizon(class))
        .min()
        .map(|h| (h / 2).mul_f64(REFRESH_LEAD))
        .filter(|p| !p.is_zero())
}

/// The interfaces a service's owner heartbeats cover (v2 §2.4, "Which
/// interfaces"), from `candidates`: each interface it implements that has
/// a collection, with the index its catalog lists its revision at and the
/// classes its collections have. In ascending order of the indices, as a
/// put's groups go. The classes are the revision's, whatever this
/// instance exposes: v2 §2.4 has "one group for each covered interface
/// whose revision has a collection of that class", and the exposure is
/// the group's map bits ("a bit for each one the owner exposes"). So a
/// group may hold a map of zeros, as a heartbeat does. An owner heartbeat holds at most 15 groups (core
/// §7.5), so an interface that would be a sixteenth on either of its
/// classes' keys is left out, whole: it keeps its heartbeat, and each of
/// its collections one key and one sequence.
#[must_use]
pub(crate) fn cover(mut candidates: Vec<(u16, IfaceId, Vec<Class>)>) -> Vec<(IfaceId, u16)> {
    candidates.sort_by_key(|(index, ..)| *index);
    let mut held = [0usize; Class::ALL.len()];
    let slot = |c: Class| Class::ALL.iter().position(|x| *x == c).expect("a class");
    let mut covered = Vec::new();
    for (index, iface, classes) in candidates {
        if classes
            .iter()
            .any(|c| held[slot(*c)] >= zenkey_model::heartbeat::MAX_GROUPS)
        {
            continue;
        }
        for c in classes {
            held[slot(c)] += 1;
        }
        covered.push((iface, index));
    }
    covered
}

/// A heartbeat's time, and an owner heartbeat's, for the owner's clock
/// reading `wall` under the deployment epoch `epoch` in Unix seconds
/// ([`Time::written`]).
///
/// - **While the clock guard holds** (`guard`, core §4.3's "Ahead") it is
///   core §7.5's ahead time, whatever `wall` reads, never the unknown time
///   (v2 §2.10).
/// - **Otherwise** it is `wall`'s instant, or unknown, all ones, for an
///   instant with no code: before the epoch or 2^32 s or more after it
///   (core §7.6), a clock before the Unix epoch included (v2 §2.4).
#[must_use]
pub(crate) fn time_at(guard: bool, wall: std::time::SystemTime, epoch: i64) -> Time {
    let at = wall
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i128::try_from(d.as_millis()).ok());
    Time::written(guard, at, epoch)
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
        // heartbeat's. While the clock guard holds, the time is the ahead
        // time (v2 §2.10, core §7.5), and the replies' stamp is from the
        // clock ahead.
        let at = self.minter.time();
        let time = time_at(
            self.minter.is_ahead(),
            at.get_time().to_system_time(),
            self.epoch,
        );
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

/// One covered interface's group on a class's owner heartbeat key.
pub(crate) struct Covered {
    /// The revision's index in the catalog the service was given.
    pub(crate) index: u16,
    pub(crate) layout: Arc<Layout>,
    /// For each collection in map order, its map bit on this key: whether
    /// it is of the key's class and this instance exposes it (v2 §2.4,
    /// "Only its class"). Fixed at start, as exposure is.
    pub(crate) exposed: Vec<bool>,
    /// A governed interface's gate: its group is left out while the
    /// instance stands by (`redundancy.v1` §2.4).
    pub(crate) gate: Option<Arc<Gate>>,
}

/// One class's owner heartbeat, as its task sends it (v2 §2.4).
pub(crate) struct OwnerBeat {
    /// The class key's writer. `freshness.v2` is no governed interface, so
    /// it is declared at once; each group asks its own interface's gate.
    pub(crate) writer: Writer,
    pub(crate) class: Class,
    /// Its groups, in ascending order of their indices.
    pub(crate) groups: Vec<Covered>,
    pub(crate) store: Arc<Store>,
    pub(crate) minter: Arc<Minter>,
    pub(crate) period: Duration,
    /// The deployment epoch its time counts from: the catalog's.
    pub(crate) epoch: i64,
}

impl OwnerBeat {
    /// Sends one owner heartbeat now: a group for each covered interface
    /// the instance serves, and nothing when it serves none of them.
    pub(crate) async fn once(&self) {
        // The class key's writer is ungated (freshness.v2 is no governed
        // interface), so this waits for nothing.
        let Ok(admitted) = self.writer.serving().await else {
            return;
        };
        // redundancy.v1 §2.4: a standby writes nothing of the interface, so
        // its group is left out. Each governed interface is asked at once,
        // never waited for (`Gate::try_write`): one standing by, or one
        // whose step-down is closing its writes, is left out of this put,
        // and takes nothing from the groups beside it. A serving one's
        // write guard is held from here to the put, which is the only
        // await under it, so its step-down waits for this put alone.
        let mut serving = Vec::with_capacity(self.groups.len());
        for g in &self.groups {
            match &g.gate {
                None => serving.push((g, None)),
                Some(gate) => {
                    if let Some(guard) = gate.try_write() {
                        serving.push((g, Some(guard)));
                    }
                }
            }
        }
        // One instant for the put, as a heartbeat's: the body's time, the
        // ahead time while the clock guard holds (v2 §2.10), one for every
        // group, and the confirmation of each member any group vouches for.
        let at = self.minter.time();
        let time = time_at(
            self.minter.is_ahead(),
            at.get_time().to_system_time(),
            self.epoch,
        );
        let class = self.class;
        let mut groups = Vec::with_capacity(serving.len());
        let mut vouched = Vec::new();
        for (g, _) in &serving {
            let iface = g.layout.iface.to_string();
            let Some((hb, v)) = self.store.heartbeat(&iface, &g.exposed, time) else {
                continue;
            };
            match Group::of(g.index, &g.layout, &hb.sections) {
                Ok(group) => {
                    groups.push(group);
                    vouched.extend(v);
                }
                // Its group is left out, and the others go: one interface's
                // sections that cannot be written take nothing from the
                // interfaces beside it.
                Err(e) => tracing::warn!(
                    %iface, %class,
                    "freshness.v2: a group of an owner heartbeat could not be written: {e}"
                ),
            }
        }
        // v2 §2.4: no owner heartbeat that would hold no group.
        if groups.is_empty() {
            return;
        }
        // The body before the seq, as a heartbeat's: one that cannot be
        // written takes none, so its readers read no gap (§2.5).
        let body = match (OwnerHeartbeat { time, groups }).write() {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(%class, "freshness.v2: an owner heartbeat could not be written: {e}");
                return;
            }
        };
        let key = OwnedKeyExpr::from(self.writer.key_expr().clone().into_owned());
        // The key's own seq, in the run the descriptor states (core §4.3):
        // a reader finds the catalog the indices are read through by it.
        let stamped = self.minter.mint_stream(&key, self.writer.is_stamped());
        let bytes = zenkey_model::wire::Envelope {
            form: zenkey_model::wire::Form::Heartbeat,
            tag: zenkey_model::wire::Tag::NONE,
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
                self.minter.unmint_stream(&key, stamped.stamp);
                tracing::warn!(%class, "freshness.v2: an owner heartbeat failed: {e}");
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
            time_at(false, at(1_794_062_759_123), e),
            Time::At {
                secs: 26_837_159,
                ms: 123
            }
        );
        assert_eq!(
            time_at(false, at(1_794_062_759_123), 0),
            Time::At {
                secs: 1_794_062_759,
                ms: 123
            },
            "no catalog: Unix seconds"
        );
        assert_eq!(time_at(false, at(1_767_225_599_999), e), Time::Unknown);
        assert_eq!(
            time_at(false, at(0), e),
            Time::Unknown,
            "a clock with no RTC"
        );
        assert_eq!(
            time_at(false, at(6_062_192_896_000), e),
            Time::Unknown,
            "2^32 s on"
        );
        assert_eq!(
            time_at(false, UNIX_EPOCH - Duration::from_millis(1), 0),
            Time::Unknown,
            "before the Unix epoch, with no catalog"
        );
    }

    /// v2 0.12 §2.10 (#986 Q0): while the clock guard holds the time is
    /// the ahead time, whatever the clock reads, as `times.json`'s three
    /// writes under `guard` have it: an instant the epoch codes, no
    /// catalog, and an instant with no code, never the unknown time.
    #[test]
    fn a_time_is_ahead_while_the_guard_holds_whatever_the_clock_reads() {
        use std::time::{Duration, UNIX_EPOCH};
        let e = 1_767_225_600;
        let at = |ms: u64| UNIX_EPOCH + Duration::from_millis(ms);
        assert_eq!(time_at(true, at(1_794_062_761_123), e), Time::Ahead);
        assert_eq!(time_at(true, at(1_794_062_761_123), 0), Time::Ahead);
        assert_eq!(time_at(true, at(6_062_192_896_000), e), Time::Ahead);
        assert_eq!(
            time_at(true, UNIX_EPOCH - Duration::from_millis(1), e),
            Time::Ahead,
            "a clock that reads no instant, under the guard"
        );
    }

    #[test]
    fn the_embedded_bundle_is_the_published_standard_contract() {
        let imp = implementation();
        assert_eq!(imp.iface().to_string(), "freshness.v2");
        assert_eq!(imp.fingerprint().hex().as_str(), FINGERPRINT);
        assert!(imp.resource(HEARTBEAT).is_ok());
        let owner = imp.resource(OWNER_HEARTBEATS).unwrap();
        assert!(owner.optional, "revision 1.1's owner heartbeat key");
        assert_eq!(
            owner.template.params().map(|(p, _)| p).collect::<Vec<_>>(),
            [OWNER_PARAM],
            "its one parameter, the class"
        );
    }

    /// v2 §2.4, "Which interfaces": an owner heartbeat holds at most 15
    /// groups (core §7.5), so a sixteenth interface on a key keeps its
    /// heartbeat, whole, and the groups go in ascending order of their
    /// indices whatever order the service implemented them in.
    #[test]
    fn a_sixteenth_interface_on_a_key_keeps_its_heartbeat() {
        let iface = |n: u16| -> IfaceId { format!("i{n}.v1").parse().unwrap() };
        // Seventeen interfaces with a `host` collection, given highest
        // index first; the one at index 3 has a `link` collection too.
        let candidates: Vec<(u16, IfaceId, Vec<Class>)> = (0..17u16)
            .rev()
            .map(|n| {
                let classes = if n == 3 {
                    vec![Class::Link, Class::Host]
                } else {
                    vec![Class::Host]
                };
                (n * 2, iface(n), classes)
            })
            .collect();
        let covered = cover(candidates);
        assert_eq!(
            covered,
            (0..15u16).map(|n| (iface(n), n * 2)).collect::<Vec<_>>(),
            "the fifteen lowest indices, ascending"
        );

        // An interface of both classes whose `host` group would be the
        // sixteenth is left out of the `link` key too: never both forms.
        let mut candidates: Vec<(u16, IfaceId, Vec<Class>)> = (0..15u16)
            .map(|n| (n, iface(n), vec![Class::Host]))
            .collect();
        candidates.push((15, iface(15), vec![Class::Link, Class::Host]));
        candidates.push((16, iface(16), vec![Class::Link]));
        let covered = cover(candidates);
        assert_eq!(covered.len(), 16);
        assert!(
            !covered.contains(&(iface(15), 15)),
            "it keeps its heartbeat"
        );
        assert!(
            covered.contains(&(iface(16), 16)),
            "the `link` key has room"
        );
        assert!(cover(Vec::new()).is_empty());
    }

    /// v2 §2.4, "The period": per key, at most half the shortest horizon
    /// among the collections of its class that its groups cover.
    #[test]
    fn an_owner_heartbeats_period_is_its_classs_shortest_half_horizon() {
        use zenkey_model::contract::load_str;
        let layout = |name: &str, body: &str| {
            let text = format!(
                "[interface]\nname = \"{name}\"\nmajor = 1\nminor = 0\n\
                 uses = [\"freshness.v2\", \"link.v1\"]\n{body}"
            );
            let l = load_str(&text, std::path::Path::new("."), None);
            Layout::of(&l.contract.unwrap_or_else(|| panic!("{}", l.report)))
        };
        let state = |name: &str, ttl: u64, link: bool| {
            let exposure = if link {
                ", \"link.exposure\" = \"link\""
            } else {
                ""
            };
            format!(
                "[resources.{name}]\nkind = \"state\"\ntype = {{ raw = \"text/plain\" }}\n\
                 annotations = {{ \"freshness.ttl_s\" = {ttl}{exposure} }}\n"
            )
        };
        let a = layout("a", &(state("slow", 900, false) + &state("far", 60, true)));
        let b = layout("b", &state("fast", 10, false));
        let both = [&a, &b];
        assert_eq!(
            owner_period(both, Class::Host),
            Some(Duration::from_secs(5).mul_f64(REFRESH_LEAD)),
            "the 900-s collection rides the 10-s one's period"
        );
        assert_eq!(
            owner_period(both, Class::Link),
            Some(Duration::from_secs(30).mul_f64(REFRESH_LEAD)),
            "the `link` key's own, whatever the `host` key's"
        );
        assert_eq!(owner_period([&b], Class::Link), None, "no group, no put");
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
