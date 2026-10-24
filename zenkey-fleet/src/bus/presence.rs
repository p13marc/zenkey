//! Presence (spec §8.1): liveliness reads, and zk2's services from their
//! tokens and descriptors.
//!
//! **Every liveliness GET this crate issues goes through
//! [`liveliness_read`]**, v1's roster sweeps included, the way every fleet
//! GET goes through [`crate::bus::query::fleet_get`]. It runs on the
//! runtime's unbounded handler ([`zenkey::presence::liveliness_read`]): zenoh's
//! default 256-slot handler hangs a liveliness GET on a session that also
//! holds a liveliness subscriber, at every size measured from 996 tokens
//! (zenoh#2678, spike S2). The spec forbids the default handler there, and
//! the rule lives in one place so a new sweep cannot forget it.
//!
//! **Two reads are not made** (core §8.1, 0.37), and say why in
//! [`crate::Error::NotAsked`]:
//! - **beside a liveliness subscriber** (#830). A liveliness GET from a
//!   session that holds one, history or not, can complete without tokens
//!   that are live (zenoh 1.10.1, and main at 766b455). Every liveliness
//!   subscriber of this crate is declared through
//!   [`zenkey::presence::declare_subscriber`], which marks its session a
//!   watch session, and the runtime refuses a read on it. A reader that
//!   follows presence live holds its [`crate::Monitor`] on a watch session
//!   of its own and reads here on the deployment's session, which holds
//!   none; a [`PresenceFeed`] refuses a seed session with the monitor's zid
//!   before it reads;
//! - **through a session cut off** from every other zenoh node
//!   ([`cut_off`]): zenoh answers it at once, complete and empty, though
//!   nothing was asked of anyone.
//!
//! **A read cut off while it runs was made** (core §8.1, 0.38, F-185): the
//! runtime checks the session again when the read ends, and one answered at
//! the cut comes back possibly incomplete, `cut_off_during` set. Every view
//! here carries it beside `complete` ([`Observed::cut_off_during`]), and
//! says why in one phrase ([`CUT_OFF_DURING`]): its tokens were seen, and
//! what it shows absent establishes nothing. A [`PresenceFeed`] reads it as
//! any seed possibly incomplete.
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
//! `Dropped(n)`, after every withdrawal that leaves something with nothing
//! held (core §8.1, 0.37), and after a read not made. The monitor watches
//! [`presence_selectors`] (or [`crate::MonitorSpec::with_presence`]) on a
//! watch session of its own; the seed reads through the session in the
//! namespace, which holds no liveliness subscriber (#830).

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use zenkey_model::grammar::{Addr, CONTROL, GRAMMAR, Name};
use zenoh::Session;

use crate::bus::monitor::{EventStream, FleetEvent, StreamItem};
use crate::model::catalog::{Catalog, DescriptorRead, Observed};
use crate::model::presence_live::{LivePresence, PresenceBasis};
use crate::report::PresenceTransition;
use crate::report::{NamespaceListing, ServiceListing};
use crate::{Error, Result};

/// The one liveliness GET of this crate (spec §8.1): every token matching
/// `selector`, sorted, and whether the GET completed before `timeout`.
///
/// A read that ran to its timeout comes back with `complete: false`: it may
/// have missed tokens, and a caller reports it as possibly incomplete,
/// never as absence (§8.1, O5). So does one whose session was cut off while
/// it ran, `cut_off_during` set (core §8.1, 0.38): it was made, and its
/// tokens were seen. Reply errors are skipped; a token carries no payload,
/// so a reply without a key has nothing to say.
///
/// Not made, and [`Error::NotAsked`] (core §8.1, 0.37): on a watch session,
/// which holds a liveliness subscriber (#830), or through a session cut off
/// from every other zenoh node.
pub async fn liveliness_read(
    session: &Session,
    selector: &str,
    timeout: Duration,
) -> Result<zenkey::presence::PresenceRead> {
    zenkey::presence::liveliness_read(session, selector, timeout)
        .await
        .map_err(|e| {
            Error::from_runtime_refusal(&e)
                .unwrap_or_else(|| Error::bus("liveliness get", selector, e))
        })
}

/// Why `session` reaches no other zenoh node, when it does not (core §8.1,
/// 0.37; #800 AD): a read through it is answered at once, complete and
/// empty, though nothing was asked of anyone — every owner would read
/// absent, and a GET would heal a loss nobody answered. A router's restart
/// cuts a client off so until it reconnects; a reader then makes no read,
/// and says why. A peer or a router alone reads nothing so too, its own
/// process's owners included: it cannot tell alone from cut off, and a
/// reader shares no session with an owner it judges (zenctl, zengui and
/// zenwatch open their own).
pub async fn cut_off(session: &Session) -> Option<String> {
    zenkey::presence::cut_off(session)
        .await
        .then(|| crate::error::NotAsked::CutOff.to_string())
}

/// Why a read is possibly incomplete when its session was cut off while it
/// ran (core §8.1, 0.38, F-185), as a phrase after "the presence read …":
/// the runtime's own words.
pub const CUT_OFF_DURING: &str = zenkey::presence::CUT_OFF_DURING;

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
    Ok(Observed::from_read(selector, &read))
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
    observed.descriptors = Some(describe_each(session, wanted, timeout).await);
}

/// The descriptor of each instance in `wanted` (§3.3), at most
/// [`DESCRIBE_CONCURRENCY`] GETs in flight, each bounded by `timeout`; every
/// answer recorded, silence and a failed GET included. What [`describe`]
/// and a [`crate::LensFeed`] following presence both run.
pub(crate) async fn describe_each(
    session: &Session,
    wanted: Vec<(Addr, zenkey_model::grammar::InstanceId)>,
    timeout: Duration,
) -> BTreeMap<(Addr, zenkey_model::grammar::InstanceId), DescriptorRead> {
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
    found
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
    Ok(crate::model::catalog::namespaces_read(
        NAMESPACE_SELECTOR,
        &read,
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

/// How long a [`PresenceFeed`] waits after a withdrawal heard live before
/// it reads again (core §8.1, 0.37): a burst of withdrawals — every token
/// at once, when the reader's router restarts or an owner's lease runs out —
/// is one read. Later withdrawals never push it back.
pub const WITHDRAWAL_SETTLE: Duration = Duration::from_millis(100);

/// What a [`PresenceFeed`]'s last seed found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedOutcome {
    /// Complete: the basis stands (§8.1).
    Complete,
    /// Ended at its timeout, or with an error reply, or with its session cut
    /// off while it ran (core §8.1, 0.38): possibly incomplete, and why, as
    /// a phrase after "the read …". What it read was folded; what it did not
    /// is unobservable, and the feed reads again.
    Incomplete(String),
    /// The subscription lost events while the seed was read, so the seed
    /// could not stand and the feed reads again.
    Overrun { dropped: u64 },
    /// The read was not made (core §8.1, 0.37): the session it reads
    /// through, or the monitor's, reaches no other zenoh node, so zenoh
    /// would have answered it at once, complete and empty. Every address is
    /// unobservable until a complete read; the feed reads again.
    CutOff(String),
    /// The read could not be put on the bus.
    Failed(String),
}

/// A [`LivePresence`] kept current from a [`crate::Monitor`] (#614, FL1):
/// seeded from one liveliness read of its scope, fed every token event the
/// monitor delivers, and re-seeded after every `Dropped(n)`.
///
/// **Two sessions** (#830, core §8.1, 0.37). The monitor holds its
/// liveliness subscribers on a watch session of its own; the feed reads
/// through another, the deployment's, which holds none, and refuses one
/// with the monitor's zid ([`PresenceFeed::open`]). A read through either
/// session while it reaches no other zenoh node is not made
/// ([`SeedOutcome::CutOff`]): every address is unobservable until a
/// complete read, retried with backoff, and at once when a token is heard
/// again.
///
/// **A withdrawal heard live is a reason to read again** (core §8.1, 0.37).
/// One that leaves something with nothing held owes a read
/// [`WITHDRAWAL_SETTLE`] later ([`LivePresence::awaits_read`]): a burst of
/// them is one read, and its complete answer decides each address it left
/// unobservable. A crash still pages: the owner's lease runs out, its
/// tokens are withdrawn, the read shows none, and the address is down, a
/// settle and one read's round trip after the withdrawal.
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
/// **Cancel-safe** (#614, FL2). [`PresenceFeed::next`] may be dropped at
/// any await — a notifier's `select!` loop drops it whenever another branch
/// wins — and nothing is lost: every transition folded is held in the feed
/// until a call hands it out, and a re-seed is owed *before* it is read, so
/// one dropped mid-read is read again by the next call. Without that, a
/// `Dropped(n)` whose re-seed was cancelled lost both its transitions and
/// the re-seed itself, and the projection stayed unobservable with nobody
/// told.
///
/// The monitor must watch the projection's scope: [`presence_selectors`]
/// with the projection's wire namespace
/// ([`LivePresence::wire_namespace`]). Its other events (samples, ticks,
/// other liveliness) pass through, and a liveliness key that is not a
/// presence token of the scope is counted ([`LivePresence::ignored`]).
pub struct PresenceFeed {
    session: Session,
    /// The monitor's session: a watch session, read only for whether it is
    /// cut off (#830, core §8.1).
    watch: Session,
    timeout: Duration,
    events: EventStream,
    presence: LivePresence,
    last_seed: SeedOutcome,
    /// Reads made: liveliness GETs asked of the bus, whatever they found.
    /// A read not made is not one.
    reads: u64,
    /// The next re-seed's time and the backoff that set it, while one is
    /// owed. Set before a re-seed is read and cleared only by a complete
    /// one, so a read that was cancelled is still owed.
    retry: Option<(tokio::time::Instant, Duration)>,
    /// Transitions folded and not yet handed out: what makes
    /// [`PresenceFeed::next`] cancel-safe.
    pending: Vec<PresenceTransition>,
}

impl std::fmt::Debug for PresenceFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PresenceFeed")
            .field("presence", &self.presence)
            .field("last_seed", &self.last_seed)
            .finish_non_exhaustive()
    }
}

/// Why a re-seed read nothing.
enum NotRead {
    /// Not made: a session reaches no other zenoh node (core §8.1, 0.37).
    CutOff(String),
    /// Could not be put on the bus.
    Failed(String),
}

/// What woke a feed.
enum Wake {
    Item(Option<StreamItem>),
    Retry,
}

impl PresenceFeed {
    /// Seed `presence` through `session` (in the namespace; each read
    /// bounded by `timeout`) and start folding `monitor`'s events into it.
    /// The event stream is taken here, before the seed is read: that is
    /// what lets the seed miss nothing. Returns the feed and the seed's
    /// transitions; a seed that could not be made leaves the basis as it was
    /// and is retried ([`PresenceFeed::last_seed`] says why).
    ///
    /// **Refused** (#830, core §8.1, 0.37), with [`Error::NotAsked`] and
    /// nothing read: a `session` that is the monitor's (the same zid), or
    /// any watch session ([`zenkey::presence::watching`]). A liveliness
    /// read beside a liveliness subscriber can complete without tokens that
    /// are live; the monitor runs on a session of its own.
    pub async fn open(
        session: &Session,
        monitor: &crate::Monitor,
        presence: LivePresence,
        timeout: Duration,
    ) -> Result<(PresenceFeed, Vec<PresenceTransition>)> {
        refuse_beside(session, monitor, "a presence feed's seed")?;
        let events = monitor.events();
        let mut feed = PresenceFeed {
            session: session.clone(),
            watch: monitor.session().clone(),
            timeout,
            events,
            presence,
            last_seed: SeedOutcome::Failed("not read yet".to_owned()),
            reads: 0,
            retry: None,
            pending: Vec::new(),
        };
        let seeded = feed.reseed().await;
        Ok((feed, seeded))
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

    /// How many presence reads the feed has made, its first seed's
    /// included: liveliness GETs asked of the bus, whatever they found. A
    /// read not made (core §8.1, 0.37) is not counted. What
    /// `spec/scenarios/presence.md` §8 observes: one read for a burst of
    /// withdrawals, none for a make-before-break re-mint.
    pub fn reads(&self) -> u64 {
        self.reads
    }

    /// The next transitions: waits for an event that moves something, or
    /// for a re-seed that is owed, and returns what moved — empty when a
    /// re-seed moved the basis alone. `None` only when the monitor's
    /// broadcast closed. Cancel-safe: see the type's doc.
    ///
    /// A `Dropped(n)` hands out its transitions (every address unobservable)
    /// first, and the re-seed it owes on the next call: cause before effect,
    /// and a loss is never hidden behind a read that may not land.
    pub async fn next(&mut self) -> Option<Vec<PresenceTransition>> {
        loop {
            if !self.pending.is_empty() {
                return Some(std::mem::take(&mut self.pending));
            }
            let wake = match self.retry {
                // A re-seed that is due goes first: a stream that never
                // pauses must not starve it.
                Some((when, _)) => tokio::select! {
                    biased;
                    () = tokio::time::sleep_until(when) => Wake::Retry,
                    item = self.events.recv() => Wake::Item(item),
                },
                None => Wake::Item(self.events.recv().await),
            };
            match wake {
                Wake::Retry => {
                    // A basis that moved with no address moving is handed
                    // out too, so a reader's sight says so: a read not made
                    // through a session cut off, or the complete one after.
                    let before = self.presence.basis();
                    self.reseed_pending().await;
                    if self.pending.is_empty() && self.presence.basis() != before {
                        return Some(Vec::new());
                    }
                }
                Wake::Item(None) => return None,
                Wake::Item(Some(StreamItem::Event(FleetEvent::NodeUp(key)))) => {
                    let moved = self.presence.token(&key, true, SystemTime::now());
                    self.pending.extend(moved);
                    // A token heard while cut off: the session reaches
                    // someone again, and the read it owes is due now.
                    if self.presence.basis() == PresenceBasis::CutOff {
                        self.owe_within(Duration::ZERO);
                    }
                }
                Wake::Item(Some(StreamItem::Event(FleetEvent::NodeDown(key)))) => {
                    let moved = self.presence.token(&key, false, SystemTime::now());
                    self.pending.extend(moved);
                    // A withdrawal heard live is a reason to read again,
                    // never absence by itself (core §8.1, 0.37).
                    if self.presence.awaits_read() {
                        self.owe_within(WITHDRAWAL_SETTLE);
                    }
                }
                Wake::Item(Some(StreamItem::Event(_))) => {}
                Wake::Item(Some(StreamItem::Dropped(n))) => {
                    let moved = self.presence.dropped(n, SystemTime::now());
                    self.pending.extend(moved);
                    self.owe_now();
                }
            }
        }
    }

    /// Read the scope again now and fold it, with what the subscription
    /// delivered meanwhile; returns what moved (and anything folded before
    /// it that no call had handed out yet).
    pub async fn reseed(&mut self) -> Vec<PresenceTransition> {
        self.reseed_pending().await;
        std::mem::take(&mut self.pending)
    }

    /// [`PresenceFeed::reseed`], folding into the feed's `pending`.
    /// The re-seed is owed until a complete read lands, so a read dropped at
    /// its await is read again; nothing is folded before the read returns.
    async fn reseed_pending(&mut self) {
        if self.retry.is_none() {
            self.owe_now();
        }
        for _ in 0..RESEED_ATTEMPTS {
            // Not made through a session that reaches nobody, the monitor's
            // included: zenoh would answer it complete and empty, and a
            // subscription that hears nothing cannot follow it (core §8.1,
            // 0.37).
            let cut = match cut_off(&self.session).await {
                Some(why) => Some(why),
                None => cut_off(&self.watch)
                    .await
                    .map(|why| format!("the monitor's session: {why}")),
            };
            let read = match cut {
                Some(why) => Err(NotRead::CutOff(why)),
                None => read_tokens(&self.session, self.presence.scope(), self.timeout)
                    .await
                    .map_err(|e| match e.not_asked() {
                        Some(crate::error::NotAsked::CutOff) => {
                            NotRead::CutOff(crate::one_line(&e))
                        }
                        _ => NotRead::Failed(crate::one_line(&e)),
                    }),
            };
            if read.is_ok() {
                self.reads += 1;
            }
            let observed = match read {
                Ok(observed) => observed,
                Err(NotRead::CutOff(why)) => {
                    let moved = self.presence.cut_off(SystemTime::now());
                    self.pending.extend(moved);
                    self.last_seed = SeedOutcome::CutOff(why);
                    self.back_off();
                    return;
                }
                Err(NotRead::Failed(why)) => {
                    self.last_seed = SeedOutcome::Failed(why);
                    self.back_off();
                    return;
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
                let moved = self.presence.dropped(lost, at);
                self.pending.extend(moved);
                self.last_seed = SeedOutcome::Overrun { dropped: lost };
                continue;
            }
            let moved = self.presence.seed_then(
                &observed,
                then.iter().map(|(key, held)| (key.as_str(), *held)),
                at,
            );
            self.pending.extend(moved);
            if observed.complete {
                self.last_seed = SeedOutcome::Complete;
                self.retry = None;
                // A withdrawal heard while the read was in flight owes the
                // next one.
                if self.presence.awaits_read() {
                    self.owe_within(WITHDRAWAL_SETTLE);
                }
            } else {
                let why = observed
                    .incomplete_reason("ended at its timeout, or with an error reply")
                    .unwrap_or_default();
                self.last_seed = SeedOutcome::Incomplete(why.to_owned());
                self.back_off();
            }
            return;
        }
        self.back_off();
    }

    /// Owe a re-seed within `wait` from now: sooner than one already owed,
    /// never later, so a burst of withdrawals is one read and a stream of
    /// them never pushes it back. The backoff a run of failures reached is
    /// kept.
    fn owe_within(&mut self, wait: Duration) {
        let when = tokio::time::Instant::now() + wait;
        self.retry = Some(match self.retry {
            Some((owed, last)) => (owed.min(when), last),
            None => (when, Duration::ZERO),
        });
    }

    /// Owe a re-seed now, keeping the backoff a run of failures has reached.
    fn owe_now(&mut self) {
        let last = self.retry.map_or(Duration::ZERO, |(_, last)| last);
        self.retry = Some((tokio::time::Instant::now(), last));
    }

    /// Owe a re-seed, a backoff from now: [`RESEED_BACKOFF`] first, doubled
    /// each time in a row, up to [`RESEED_BACKOFF_MAX`].
    fn back_off(&mut self) {
        let wait = match self.retry {
            Some((_, last)) if !last.is_zero() => (last * 2).min(RESEED_BACKOFF_MAX),
            _ => RESEED_BACKOFF,
        };
        self.retry = Some((tokio::time::Instant::now() + wait, wait));
    }
}

/// Refuse `session` as `what`'s read beside `monitor` (#830, core §8.1,
/// 0.37): the monitor's own session, or any watch session — a reader that
/// seeds beside a monitor calls this before it reads, as
/// [`PresenceFeed::open`] does.
pub fn refuse_beside(session: &Session, monitor: &crate::Monitor, what: &str) -> Result<()> {
    if session.zid() == monitor.zid() || zenkey::presence::watching(session) {
        return Err(Error::not_asked_for(
            what,
            crate::error::NotAsked::BesideSubscriber {
                zid: session.zid().to_string(),
            },
        ));
    }
    Ok(())
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
