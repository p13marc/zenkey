//! Presence (spec §8.1): liveliness reads, and zk2's services from their
//! tokens and descriptors.
//!
//! **Every liveliness GET this crate issues goes through
//! [`liveliness_read`]**, v1's roster sweeps included, the way every fleet
//! GET goes through [`crate::bus::query::fleet_get`]. It runs on the
//! runtime's unbounded handler ([`zenkey::presence::liveliness_read`]): zenoh's
//! default 256-slot handler hangs a liveliness GET on a session that also
//! holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2), and this crate's sessions hold one whenever a
//! [`crate::Monitor`] watches the roster. The spec forbids the default
//! handler there, and the rule lives in one place so a new sweep cannot
//! forget it.
//!
//! **zk2's services** (§8.1, §3.3) are read in two steps: one liveliness GET
//! for the tokens in a [`Scope`] ([`read_tokens`]), then the descriptor of
//! every instance they name ([`describe`]), which is how a tokenless
//! interface's provider is found at all (U22). Both land in an
//! [`Observed`], a value the [`Catalog`] indexes without a session; the
//! completeness flag rides along, so a read that timed out is reported as
//! possibly incomplete, never as absence.
//!
//! The zk2 functions take a bare `&Session` and spell base-relative keys: a
//! zk2 tool reads through a session in the deployment's namespace (decided
//! 2026-10-08, FJ plan item 1), unlike v1's un-namespaced explorer.
//!
//! **Presence kept current** (#614, FL1): a [`PresenceFeed`] seeds a
//! [`LivePresence`] from one read of its scope, then folds a
//! [`crate::Monitor`]'s liveliness events into it, re-seeding after every
//! `Dropped(n)`. The monitor watches [`presence_selectors`] (or
//! [`crate::MonitorSpec::with_presence`]) on whichever session it runs; the
//! seed reads through the session in the namespace.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use zenkey_model::grammar::{Addr, CONTROL, GRAMMAR, Name};
use zenoh::Session;

use crate::bus::monitor::{EventStream, FleetEvent, StreamItem};
use crate::model::catalog::{Catalog, DescriptorRead, Observed};
use crate::model::presence_live::{LivePresence, PresenceTransition};
use crate::report::{NamespaceListing, ServiceListing};
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
) -> Result<zenkey::presence::PresenceRead> {
    zenkey::presence::liveliness_read(session, selector, timeout)
        .await
        .map_err(|e| Error::bus("liveliness get", selector, e))
}

/// How many descriptor GETs [`describe`] keeps in flight at once.
pub const DESCRIBE_CONCURRENCY: usize = 64;

/// Which zk2 services a presence read covers: every one, one system's, or
/// one address. Interfaces are not a scope: a tokenless provider (U22) has
/// no interface token to select, so a per-interface view reads its scope
/// whole and filters ([`Catalog::iface`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    system: Option<Name>,
    service: Option<Name>,
}

impl Scope {
    /// Every zk2 service: `zk2/*/*/@zk/**`.
    pub fn all() -> Scope {
        Scope::default()
    }

    /// One system's services: `zk2/<system>/*/@zk/**`. A name that is not a
    /// plain chunk is refused, and nothing is asked.
    pub fn system(system: &str) -> Result<Scope> {
        let system = Name::new("system", system).map_err(|e| Error::unaskable_from(system, e))?;
        Ok(Scope {
            system: Some(system),
            service: None,
        })
    }

    /// One service address: `zk2/<system>/<service>/@zk/**`.
    pub fn service(addr: &Addr) -> Scope {
        Scope {
            system: Some(addr.system.clone()),
            service: Some(addr.service.clone()),
        }
    }

    /// The liveliness selector, base-relative. `@zk` is named because `*`
    /// and `**` never cross a verbatim chunk; after it, `**` reaches every
    /// token form (`instance/…`, `alive/…`, `member/…`).
    pub fn selector(&self) -> String {
        self.under_control(&["**"])
    }

    /// The scope's three token selectors, base-relative, one per token form
    /// (§1.1): the instance tokens (`…/@zk/instance/*`), the interface
    /// tokens (`…/@zk/alive/**`) and the member tokens
    /// (`…/@zk/member/**`). What a live watch subscribes to, a form at a
    /// time; [`Scope::selector`] is the one GET that reads all three.
    pub fn token_selectors(&self) -> [String; 3] {
        [
            self.under_control(&["instance", "*"]),
            self.under_control(&["alive", "**"]),
            self.under_control(&["member", "**"]),
        ]
    }

    /// Whether `addr` is one of the scope's services.
    pub fn contains(&self, addr: &Addr) -> bool {
        self.system.as_ref().is_none_or(|s| *s == addr.system)
            && self.service.as_ref().is_none_or(|s| *s == addr.service)
    }

    /// `zk2/<system>/<service>/@zk/<rest…>`, `*` for what the scope leaves
    /// open. `@zk` is always named: `*` and `**` never cross a verbatim
    /// chunk (§1.3; RFC 03 §4 D2, carried into zk2).
    fn under_control(&self, rest: &[&str]) -> String {
        let system = self.system.as_ref().map_or("*", Name::as_str);
        let service = self.service.as_ref().map_or("*", Name::as_str);
        let mut chunks = vec![GRAMMAR, system, service, CONTROL];
        chunks.extend_from_slice(rest);
        chunks.join("/")
    }
}

/// The tokens in `scope`: one liveliness GET on the unbounded handler
/// (§8.1), parsed. Descriptors are not asked: see [`describe`].
pub async fn read_tokens(session: &Session, scope: &Scope, timeout: Duration) -> Result<Observed> {
    let selector = scope.selector();
    let read = liveliness_read(session, &selector, timeout).await?;
    Ok(Observed::from_keys(selector, &read.keys, read.complete))
}

/// GETs the descriptor of every instance `observed` names (§3.3), at most
/// [`DESCRIBE_CONCURRENCY`] at once, each bounded by `timeout`, and records
/// each answer — silence and a failed GET included — in
/// `observed.descriptors`.
pub async fn describe(session: &Session, observed: &mut Observed, timeout: Duration) {
    describe_where(session, observed, timeout, |_, _| true).await;
}

/// [`describe`], for the instances `keep` selects only: the doctor's first
/// presence read describes the instances `hostid-duplicate` counts (#721,
/// PF), and no other. The ones left out have no entry.
pub async fn describe_where(
    session: &Session,
    observed: &mut Observed,
    timeout: Duration,
    keep: impl Fn(&Addr, &zenkey_model::grammar::InstanceId) -> bool,
) {
    let wanted: Vec<_> = observed
        .instances()
        .into_iter()
        .filter(|(a, i)| keep(a, i))
        .collect();
    let mut pending = wanted.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    let mut found = BTreeMap::new();
    loop {
        while tasks.len() < DESCRIBE_CONCURRENCY {
            let Some((addr, instance)) = pending.next() else {
                break;
            };
            let session = session.clone();
            tasks.spawn(async move {
                let read =
                    match zenkey::presence::descriptor(&session, &addr, &instance, timeout).await {
                        Ok(found) => DescriptorRead::from(found),
                        Err(e) => DescriptorRead::Failed(e.to_string()),
                    };
                ((addr, instance), read)
            });
        }
        match tasks.join_next().await {
            Some(Ok((key, read))) => {
                found.insert(key, read);
            }
            Some(Err(e)) => tracing::warn!("a descriptor GET task failed: {e}"),
            None => break,
        }
    }
    observed.descriptors = Some(found);
}

/// [`read_tokens`], then [`describe`]: what every zk2 view starts from.
pub async fn observe(session: &Session, scope: &Scope, timeout: Duration) -> Result<Observed> {
    let mut observed = read_tokens(session, scope, timeout).await?;
    describe(session, &mut observed, timeout).await;
    Ok(observed)
}

/// The selector `namespace list` reads, **un-namespaced**: every zk2
/// instance token under any prefix. `**` matches the empty prefix too, so
/// the bus-root deployment is in it; `@zk` is named because `*` and `**`
/// never cross a verbatim chunk.
pub const NAMESPACE_SELECTOR: &str = "**/zk2/*/*/@zk/instance/*";

/// `namespace list` (#612, FJ4): which deployment namespaces hold zk2
/// instance tokens, from one liveliness GET on a session that is **not** in
/// a namespace — the raw half of the FJ decision, like the admin space.
/// Through a namespaced session the prefix would be stripped and every
/// namespace but its own invisible.
pub async fn namespace_listing(session: &Session, timeout: Duration) -> Result<NamespaceListing> {
    let read = liveliness_read(session, NAMESPACE_SELECTOR, timeout).await?;
    Ok(crate::model::catalog::namespaces(
        NAMESPACE_SELECTOR,
        &read.keys,
        read.complete,
    ))
}

/// `service list` (§8.1): every service in `scope`, from its tokens and its
/// instances' descriptors, with the read's completeness carried through.
pub async fn service_listing(
    session: &Session,
    scope: &Scope,
    timeout: Duration,
) -> Result<ServiceListing> {
    Ok(Catalog::new(&observe(session, scope, timeout).await?).services())
}

// ─── presence kept current ──────────────────────────────────────────────────

/// `scope`'s three token selectors ([`Scope::token_selectors`]) as the wire
/// spells them in `namespace`: what a [`crate::Monitor`] watches for a
/// [`PresenceFeed`]. The namespace is the one the monitor's session sees on
/// keys — the deployment's for a session in no namespace, empty for one in
/// the namespace.
pub fn presence_selectors(namespace: &str, scope: &Scope) -> Vec<String> {
    scope
        .token_selectors()
        .iter()
        .map(|s| crate::model::namespace::join(namespace, s))
        .collect()
}

/// How long a [`PresenceFeed`] waits before reading again after a seed that
/// failed, ended possibly incomplete or lost events; doubled at each one in
/// a row, up to [`RESEED_BACKOFF_MAX`].
pub const RESEED_BACKOFF: Duration = Duration::from_secs(1);

/// The longest a [`PresenceFeed`] waits between two seeds while none
/// completes.
pub const RESEED_BACKOFF_MAX: Duration = Duration::from_secs(30);

/// How many reads one re-seed makes while each one loses events in its
/// window, before it backs off.
const RESEED_ATTEMPTS: usize = 3;

/// What a [`PresenceFeed`]'s last seed found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedOutcome {
    /// Complete: the basis stands (§8.1).
    Complete,
    /// Ended at its timeout, or with an error reply: possibly incomplete.
    /// What it read was folded; what it did not is unobservable, and the
    /// feed reads again.
    Incomplete,
    /// The subscription lost events while the seed was read, so the seed
    /// could not stand and the feed reads again.
    Overrun { dropped: u64 },
    /// The read could not be put on the bus.
    Failed(String),
}

/// A [`LivePresence`] kept current from a [`crate::Monitor`] (#614, FL1):
/// seeded from one liveliness read of its scope, fed every token event the
/// monitor delivers, and re-seeded after every `Dropped(n)`.
///
/// **The order is the honesty.** The event stream is taken before the seed
/// is read, so nothing between the two is missed; the events delivered
/// while the read was in flight are folded into the seed in one step
/// ([`LivePresence::seed_then`]), so a token the seed already reflects does
/// not read as gone and back. A `Dropped(n)` makes every address
/// unobservable at once, and the re-seed that follows restores what a
/// complete read can (a read that loses events itself is read again, up to
/// three times, then retried with backoff). A seed that failed or ended
/// possibly incomplete is retried with backoff too ([`RESEED_BACKOFF`]),
/// while the feed goes on folding live events.
///
/// The monitor must watch the projection's scope: [`presence_selectors`]
/// with the projection's wire namespace
/// ([`LivePresence::wire_namespace`]). Its other events (samples, ticks,
/// other liveliness) pass through, and a liveliness key that is not a
/// presence token of the scope is counted ([`LivePresence::ignored`]).
pub struct PresenceFeed {
    session: Session,
    timeout: Duration,
    events: EventStream,
    presence: LivePresence,
    last_seed: SeedOutcome,
    /// The next re-seed's time and the backoff that set it, while one is
    /// owed.
    retry: Option<(tokio::time::Instant, Duration)>,
}

impl std::fmt::Debug for PresenceFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceFeed")
            .field("presence", &self.presence)
            .field("last_seed", &self.last_seed)
            .finish_non_exhaustive()
    }
}

/// What woke a feed.
enum Wake {
    Item(Option<StreamItem>),
    Retry,
}

impl PresenceFeed {
    /// Seed `presence` through `session` (in the namespace; each read
    /// bounded by `timeout`) and start folding `events` into it. `events`
    /// must be taken from the monitor before this is called: that is what
    /// lets the seed miss nothing. Returns the feed and the seed's
    /// transitions; a seed that could not be made leaves the basis as it was
    /// and is retried ([`PresenceFeed::last_seed`] says why).
    pub async fn open(
        session: &Session,
        events: EventStream,
        presence: LivePresence,
        timeout: Duration,
    ) -> (PresenceFeed, Vec<PresenceTransition>) {
        let mut feed = PresenceFeed {
            session: session.clone(),
            timeout,
            events,
            presence,
            last_seed: SeedOutcome::Failed("not read yet".to_owned()),
            retry: None,
        };
        let seeded = feed.reseed().await;
        (feed, seeded)
    }

    /// The projection, as folded so far.
    pub fn presence(&self) -> &LivePresence {
        &self.presence
    }

    /// The projection, to [track](LivePresence::track) an address.
    pub fn presence_mut(&mut self) -> &mut LivePresence {
        &mut self.presence
    }

    /// What the last seed found.
    pub fn last_seed(&self) -> &SeedOutcome {
        &self.last_seed
    }

    /// The next transitions: waits for an event that moves something, or
    /// for a re-seed that is owed, and returns what moved. `None` only when
    /// the monitor's broadcast closed.
    pub async fn next(&mut self) -> Option<Vec<PresenceTransition>> {
        loop {
            let wake = match self.retry {
                Some((when, _)) => tokio::select! {
                    item = self.events.recv() => Wake::Item(item),
                    () = tokio::time::sleep_until(when) => Wake::Retry,
                },
                None => Wake::Item(self.events.recv().await),
            };
            let moved = match wake {
                Wake::Retry => self.reseed().await,
                Wake::Item(None) => return None,
                Wake::Item(Some(StreamItem::Event(FleetEvent::NodeUp(key)))) => {
                    self.presence.token(&key, true, SystemTime::now())
                }
                Wake::Item(Some(StreamItem::Event(FleetEvent::NodeDown(key)))) => {
                    self.presence.token(&key, false, SystemTime::now())
                }
                Wake::Item(Some(StreamItem::Event(_))) => continue,
                Wake::Item(Some(StreamItem::Dropped(n))) => {
                    let mut moved = self.presence.dropped(n, SystemTime::now());
                    moved.extend(self.reseed().await);
                    moved
                }
            };
            if !moved.is_empty() {
                return Some(moved);
            }
        }
    }

    /// Read the scope again now and fold it, with what the subscription
    /// delivered meanwhile; returns what moved.
    pub async fn reseed(&mut self) -> Vec<PresenceTransition> {
        let mut moved = Vec::new();
        for _ in 0..RESEED_ATTEMPTS {
            let read = read_tokens(&self.session, self.presence.scope(), self.timeout).await;
            let observed = match read {
                Ok(observed) => observed,
                Err(e) => {
                    self.last_seed = SeedOutcome::Failed(crate::one_line(&e));
                    self.back_off();
                    return moved;
                }
            };
            let mut then = Vec::new();
            let mut lost = 0u64;
            while let Some(item) = self.events.try_recv() {
                match item {
                    StreamItem::Event(FleetEvent::NodeUp(key)) => then.push((key, true)),
                    StreamItem::Event(FleetEvent::NodeDown(key)) => then.push((key, false)),
                    StreamItem::Event(_) => {}
                    StreamItem::Dropped(n) => lost = lost.saturating_add(n),
                }
            }
            let at = SystemTime::now();
            if lost > 0 {
                // The read's window has a hole in it. What was drained is
                // reflected in the next read, which starts after it.
                moved.extend(self.presence.dropped(lost, at));
                self.last_seed = SeedOutcome::Overrun { dropped: lost };
                continue;
            }
            moved.extend(self.presence.seed_then(
                &observed,
                then.iter().map(|(key, held)| (key.as_str(), *held)),
                at,
            ));
            if observed.complete {
                self.last_seed = SeedOutcome::Complete;
                self.retry = None;
            } else {
                self.last_seed = SeedOutcome::Incomplete;
                self.back_off();
            }
            return moved;
        }
        self.back_off();
        moved
    }

    /// Owe a re-seed, a backoff from now: [`RESEED_BACKOFF`] first, doubled
    /// each time in a row, up to [`RESEED_BACKOFF_MAX`].
    fn back_off(&mut self) {
        let wait = match self.retry {
            Some((_, last)) => (last * 2).min(RESEED_BACKOFF_MAX),
            None => RESEED_BACKOFF,
        };
        self.retry = Some((tokio::time::Instant::now() + wait, wait));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_spell_their_selectors() {
        assert_eq!(Scope::all().selector(), "zk2/*/*/@zk/**");
        assert_eq!(
            Scope::system("host-a").expect("a chunk").selector(),
            "zk2/host-a/*/@zk/**"
        );
        let addr: Addr = "host-a/tc".parse().expect("an address");
        assert_eq!(Scope::service(&addr).selector(), "zk2/host-a/tc/@zk/**");
        let refused = Scope::system("Host A").expect_err("not a plain chunk");
        assert!(refused.is_unaskable(), "{refused}");
    }

    /// A live watch's selectors: one per token form, `@zk` named, each as
    /// the wire spells it in the monitor's namespace; and a scope contains
    /// exactly the addresses it spells.
    #[test]
    fn scopes_spell_their_token_selectors() {
        assert_eq!(
            Scope::all().token_selectors(),
            [
                "zk2/*/*/@zk/instance/*",
                "zk2/*/*/@zk/alive/**",
                "zk2/*/*/@zk/member/**",
            ]
        );
        let addr: Addr = "host-a/tc".parse().expect("an address");
        assert_eq!(
            presence_selectors("prod", &Scope::service(&addr)),
            [
                "prod/zk2/host-a/tc/@zk/instance/*",
                "prod/zk2/host-a/tc/@zk/alive/**",
                "prod/zk2/host-a/tc/@zk/member/**",
            ]
        );
        let other: Addr = "host-b/tc".parse().expect("an address");
        let system = Scope::system("host-a").expect("a chunk");
        assert!(Scope::all().contains(&other));
        assert!(system.contains(&addr) && !system.contains(&other));
        let sibling: Addr = "host-a/gui".parse().expect("an address");
        assert!(!Scope::service(&addr).contains(&sibling));
    }
}
