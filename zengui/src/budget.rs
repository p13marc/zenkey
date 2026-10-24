//! Population budgets in the tree (#221; zk2's since #614, FL7): each
//! templated resource's bound in its instance (core §2.7) against what the
//! tree's window heard of its members, judged by the core's own judge,
//! `zenkey_model::budget` — the one zenctl's doctor and conformance checks
//! use.
//!
//! The judge answers in four states and a reason, and the badge says both:
//! **within** and **exceeds** with their numbers, **unobservable** with why
//! (the window is lossy, the owner was not present throughout, the window
//! is shorter than a liveness span, it heard nothing, or it is a state,
//! which only an owner's GET counts), and **not asked** for a no-ceiling
//! bound, said rather than skipped. Expect mostly *unobservable*: a stream's
//! liveness span is an hour, a best-effort stream is lossy by declaration,
//! and "lost no delivery" is established, never assumed (core 0.25) — the
//! badge is honest about what a GUI's window can know, which is the point.
//!
//! The window is the tree's ([`Window`] from what the observer already
//! holds, at no bus cost):
//!
//! - **heard**: each member's last instant (`TreeNode::last_seen`). The
//!   tree keeps no other, so a count is exact for the members live at the
//!   window's end and a lower bound for an earlier peak — an *exceeds* is
//!   conclusive, and a *within* rests on the end;
//! - **listened**: from when this window first saw a watch covering the
//!   whole family, plus the reply timeout (0.25: the subscription is taken
//!   to have reached the owner then), to one lease before now, since the
//!   owner must stay present a lease after the window's end;
//! - **lossless**: `zenkey_model::budget::lossless` over the resource's
//!   declared QoS, what the reader lost (the monitor's broadcast drops and
//!   the key table's evictions) and the longest gap between two of this
//!   window's ticks against the stall bound (0.27);
//! - **present**: the presence projection's basis complete, and an
//!   instance of the address holding its token since before the window
//!   began.
//!
//! A family no watch covers whole has no window, so no badge — the
//! question was not put (O4). Without a presence read there is no owner
//! to be present, and no badge either.
//!
//! Badges are keyed by the family's **subtree display path** — the wire
//! path down to the template's literal prefix, the namespace included — so
//! the tree looks a row up by path, off the render path. Computed in a
//! `Task` on a throttled cadence (`update/bus.rs`), never per frame.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use zenkey_fleet::{
    AddressStatus, BundleStore, Catalog, ContractState, Contracts as _, KeyTreeSnapshot,
    LivePresence, PresenceBasis, TreeNode,
};
use zenkey_model::authoring::Kind;
use zenkey_model::budget::{self, Bound, Judged, Reading, Reason, Verdict, Window};
use zenkey_model::contract::Resource;
use zenkey_model::grammar::{Addr, GRAMMAR, KindToken};
use zenkey_model::template::Segment;

use crate::view::theme::BudgetTone;

/// One family's judgement, at its subtree path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetBadge {
    /// The bound in force (the contract's, or the greatest an instance's
    /// descriptor states, §2.7).
    pub bound: Bound,
    /// The core judge's answer: verdict, reason, and what it counted.
    pub judged: Judged,
}

impl BudgetBadge {
    /// The badge's tone: the verdict, never a boolean.
    pub fn tone(&self) -> BudgetTone {
        match self.judged.verdict {
            Verdict::Exceeds => BudgetTone::Exceeds,
            Verdict::Within => BudgetTone::Within,
            Verdict::Unobservable => BudgetTone::Unobservable,
            Verdict::NotAsked => BudgetTone::NotAsked,
        }
    }

    /// The badge word. The numbers ride in the words for an answer and the
    /// reason for a non-answer, so colour is never the only carrier.
    pub fn label(&self) -> String {
        let n = self.judged.counted;
        match self.judged.verdict {
            Verdict::Exceeds => format!("exceeds budget: {n} of {}", self.bound),
            Verdict::Within => format!("within budget: {n} of {}", self.bound),
            Verdict::Unobservable => {
                format!("budget unobservable ({})", reason_words(self.judged.reason))
            }
            Verdict::NotAsked => {
                format!("budget not asked ({})", reason_words(self.judged.reason))
            }
        }
    }

    /// Which badge a row keeps when two families share it (a template that
    /// opens on a parameter hangs on its kind's row): the finding first,
    /// then the reading that could not say, then the kept budget.
    fn rank(&self) -> u8 {
        match self.judged.verdict {
            Verdict::Exceeds => 0,
            Verdict::Unobservable => 1,
            Verdict::Within => 2,
            Verdict::NotAsked => 3,
        }
    }
}

/// A reason as a person reads it — the fixture's token, unsnaked.
pub fn reason_words(reason: Reason) -> &'static str {
    match reason {
        Reason::OverBound => "over its bound",
        Reason::RateExceeded => "rate exceeded",
        Reason::Complete => "complete",
        Reason::RateKept => "rate kept",
        Reason::Empty => "nothing heard",
        Reason::Incomplete => "incomplete",
        Reason::Lossy => "lossy",
        Reason::WindowTooShort => "window too short",
        Reason::OwnerAbsent => "owner absent",
        Reason::ReadingKind => "a window does not count state",
        Reason::NoCeiling => "no ceiling",
        Reason::Untemplated => "untemplated",
        Reason::Kind => "an operation",
        Reason::NoRate => "no rate",
    }
}

/// Subtree display path → badge, as of one judgement of the tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BudgetBadges {
    map: BTreeMap<String, BudgetBadge>,
}

impl BudgetBadges {
    pub fn get(&self, path: &str) -> Option<&BudgetBadge> {
        self.map.get(path)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Keep `badge` at `path`, unless a weightier one is there already.
    fn keep(&mut self, path: String, badge: BudgetBadge) {
        match self.map.get(&path) {
            Some(held) if held.rank() <= badge.rank() => {}
            _ => {
                self.map.insert(path, badge);
            }
        }
    }
}

impl FromIterator<(String, BudgetBadge)> for BudgetBadges {
    fn from_iter<I: IntoIterator<Item = (String, BudgetBadge)>>(iter: I) -> Self {
        let mut out = BudgetBadges::default();
        for (path, badge) in iter {
            out.keep(path, badge);
        }
        out
    }
}

/// The longest gap between two of the window's ticks (core §2.7, 0.27):
/// the GUI's own ticker. A tick is the monitor's, landed on the update
/// thread, so a gap here is the window not reading — never shorter than
/// the reader's own stall, which is the side a bound must err on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickGap {
    last: Option<Instant>,
    longest: Duration,
}

impl TickGap {
    /// One live tick, landed at `at`.
    pub fn tick(&mut self, at: Instant) {
        if let Some(last) = self.last {
            self.longest = self.longest.max(at.saturating_duration_since(last));
        }
        self.last = Some(at);
    }

    /// The longest gap so far, the one still open at `now` included.
    pub fn longest(&self, now: Instant) -> Duration {
        let open = self
            .last
            .map_or(Duration::ZERO, |l| now.saturating_duration_since(l));
        self.longest.max(open)
    }
}

/// What the window's reader saw of its own path (core §2.7, 0.25, 0.27).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReaderLoss {
    /// Deliveries the reader lost: the monitor's broadcast drops and the
    /// key table's evictions, since the monitor started.
    pub lost: u64,
    /// The longest gap between two of the window's ticks.
    pub gap: Duration,
}

/// Everything one judgement reads, owned, for a task.
#[derive(Clone)]
pub struct BudgetInputs {
    /// The deployment namespace; the tree's paths carry it.
    pub namespace: String,
    /// The presence read the lens resolves with: which addresses provide
    /// which interfaces at which revision, and what each instance states.
    pub catalog: Option<Arc<Catalog>>,
    /// Where a revision is held (seeded with the offline contracts).
    pub store: BundleStore,
    /// The presence projection: whether an owner was present throughout.
    pub presence: Option<Arc<LivePresence>>,
    /// The observed tree: each member's last instant.
    pub observed: Arc<KeyTreeSnapshot>,
    /// Each active watch selector (a wire selector, in no namespace) and
    /// when this window first saw it.
    pub watches: Vec<(String, Instant)>,
    pub reader: ReaderLoss,
    /// The reply timeout a subscription is taken to need to reach an owner.
    pub timeout: Duration,
    /// The judgement's clock, monotonic and wall.
    pub now: Instant,
    pub wall: SystemTime,
}

/// Judge every templated resource of every placed address against the
/// tree. Pure: the throttle and the task around it stay trivial.
pub fn badges(inputs: &BudgetInputs) -> BudgetBadges {
    let mut out = BudgetBadges::default();
    let Some(catalog) = inputs.catalog.as_deref() else {
        return out;
    };
    let base: Vec<&str> = inputs
        .namespace
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();
    for addr in catalog.addresses() {
        for iface in catalog.provided(addr) {
            let fps = catalog.revisions_of(addr, &iface);
            // One revision named and held: what the lens resolves keys
            // through. Two (a rolling upgrade) and the keys resolve as
            // ambiguous, so no bound can be read for them.
            if fps.len() != 1 {
                continue;
            }
            let Some(fp) = fps.iter().next() else {
                continue;
            };
            let Some(ContractState::Held(revision)) = inputs.store.state(&iface, fp) else {
                continue;
            };
            let at = Family {
                base: &base,
                addr,
                iface: iface.to_string(),
            };
            for r in &revision.contract().resources {
                if r.kind == Kind::Operation || !r.template.has_params() {
                    continue;
                }
                // `<kind token>/<template>`, the key a descriptor's
                // lowered bound is stated under (§3.3).
                let name = format!("{}/{}", r.token, r.template);
                let stated: Vec<Option<u64>> = catalog
                    .descriptors()
                    .filter(|d| d.service == addr.to_string())
                    .flat_map(|d| d.interfaces.iter())
                    .filter(|e| e.iface == at.iface && e.contract == fp.to_string())
                    .map(|e| e.cardinality.get(&name).copied())
                    .collect();
                let bound = budget::bound_of(r, stated);
                if let Some((path, badge)) = judge(inputs, &at, r, bound) {
                    out.keep(path, badge);
                }
            }
        }
    }
    out
}

/// Where a family sits: the namespace's chunks, its address and interface.
struct Family<'a> {
    base: &'a [&'a str],
    addr: &'a Addr,
    iface: String,
}

impl Family<'_> {
    /// `<ns>/zk2/<system>/<service>/<iface>/<token>`, chunk by chunk.
    fn head<'s>(&'s self, token: &'s str) -> Vec<&'s str> {
        let mut out: Vec<&str> = self.base.to_vec();
        out.extend([
            GRAMMAR,
            self.addr.system.as_str(),
            self.addr.service.as_str(),
            self.iface.as_str(),
            token,
        ]);
        out
    }
}

/// One family's badge and the path it hangs on, or `None` when no watch
/// covers the family whole (no window, no question).
fn judge(
    inputs: &BudgetInputs,
    at: &Family<'_>,
    r: &Resource,
    bound: Bound,
) -> Option<(String, BudgetBadge)> {
    let token = r.token.as_str();
    let events = r.token == KindToken::Events;
    let literal: Vec<&str> = r
        .template
        .segments()
        .iter()
        .map_while(|s| match s {
            Segment::Literal(l) => Some(l.as_str()),
            _ => None,
        })
        .collect();
    let mut path = at.head(token);
    path.extend(&literal);
    let mut selector = at.head(token);
    selector.extend(r.template.segments().iter().map(|s| match s {
        Segment::Literal(l) => l.as_str(),
        Segment::Param(_) => "*",
        Segment::Rest(_) => "**",
    }));
    if events {
        selector.push("*");
    }
    let family = selector.join("/");
    let start = inputs
        .watches
        .iter()
        .filter(|(sel, _)| includes(sel, &family))
        .map(|(_, since)| *since)
        .min()?;

    let mut heard: BTreeMap<String, Vec<Duration>> = BTreeMap::new();
    if let Some(node) = inputs.observed.node(&path) {
        let mut chunks: Vec<String> = literal.iter().map(|c| (*c).to_owned()).collect();
        members(node, &mut chunks, &mut |chunks, seen| {
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            let refs = match (events, refs.split_last()) {
                (true, Some((_ulid, template))) => template,
                _ => &refs[..],
            };
            if r.template.matches(refs).is_some() {
                heard
                    .entry(refs.join("/"))
                    .or_default()
                    .push(seen.saturating_duration_since(start));
            }
        });
    }

    let end = inputs
        .now
        .checked_sub(budget::DEFAULT_LEASE)
        .unwrap_or(start);
    let listened = end.saturating_duration_since(start + inputs.timeout);
    let stall = budget::ReaderStall {
        gap: inputs.reader.gap,
        bound: budget::stall_bound(
            budget::DEFAULT_WAIT_BEFORE_CLOSE,
            budget::DEFAULT_LEASE,
            budget::DEFAULT_KEEP_ALIVE,
        ),
        transport_closed: false,
    };
    let lossless = budget::delivery_of(r, inputs.reader.lost, stall, None)
        .is_some_and(|d| budget::lossless(&d).is_lossless());
    let began = inputs
        .wall
        .checked_sub(inputs.now.saturating_duration_since(start))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let present = inputs.presence.as_deref().is_some_and(|p| {
        p.basis() == PresenceBasis::Complete
            && p.address(at.addr).is_some_and(|a| {
                a.status == AddressStatus::Up
                    && a.instances
                        .values()
                        .any(|i| i.instance_token && i.since.is_some_and(|s| s <= began))
            })
    });
    let judged = budget::population(
        r.kind,
        bound,
        budget::retention_of(r),
        &Reading::Window(Window {
            heard,
            listened,
            lossless,
            present,
        }),
    );
    // Not asked for want of parameters or for an operation never reaches
    // here; a no-ceiling bound does, and says so.
    Some((path.join("/"), BudgetBadge { bound, judged }))
}

/// Every key under `node` a sample landed on, with its chunks below the
/// family's literal prefix appended to `chunks`.
fn members(node: &TreeNode, chunks: &mut Vec<String>, f: &mut dyn FnMut(&[String], Instant)) {
    for (chunk, child) in &node.children {
        chunks.push(chunk.clone());
        if let Some(seen) = child.last_seen {
            f(chunks, seen);
        }
        members(child, chunks, f);
        chunks.pop();
    }
}

/// Whether the wire selector `sel` covers every key of `family` — a
/// family half watched is not a window over it. A selector that is no key
/// expression covers nothing.
fn includes(sel: &str, family: &str) -> bool {
    use zenoh::key_expr::OwnedKeyExpr;
    match (
        OwnedKeyExpr::autocanonize(sel.to_owned()),
        OwnedKeyExpr::autocanonize(family.to_owned()),
    ) {
        (Ok(a), Ok(b)) => a.includes(&b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;
    use zenkey_fleet::model::catalog::{DescriptorRead, Observed};
    use zenkey_fleet::{ContractSet, Revision, StatsTable};
    use zenkey_model::descriptor::{Descriptor, InterfaceEntry};
    use zenkey_model::grammar::{IfaceId, InstanceId, alive_key, instance_key};

    /// A contract with a lossless templated stream, a best-effort one, a
    /// templated state and a no-ceiling stream: what no example declares
    /// all of.
    const PROBE: &str = r#"
[interface]
name = "probe"
major = 1

[resources."m/{n}"]
kind = "stream"
type = { raw = "application/octet-stream" }
params = { n = "string" }
cardinality = 4
reliability = "reliable"
congestion = "block"

[resources."lossy/{n}"]
kind = "stream"
type = { raw = "application/octet-stream" }
params = { n = "string" }
cardinality = 4

[resources."s/{n}"]
kind = "state"
type = { raw = "application/octet-stream" }
params = { n = "string" }
cardinality = 4

[resources."free/{n}"]
kind = "stream"
type = { raw = "application/octet-stream" }
params = { n = "string" }
cardinality = 4294967295
reliability = "reliable"
congestion = "block"
"#;

    struct Fixture {
        catalog: Arc<Catalog>,
        store: BundleStore,
        presence: Arc<LivePresence>,
    }

    /// `site/probe` serving `probe.v1`, its instance seen at `since`.
    fn fixture(since: SystemTime) -> Fixture {
        let loaded = zenkey_model::contract::load_str(PROBE, std::path::Path::new("."), None);
        let contract = loaded
            .contract
            .unwrap_or_else(|| panic!("does not load:\n{}", loaded.report));
        let mut set = ContractSet::new();
        let revision = set.insert(Revision::from_contract(
            contract,
            zenkey_fleet::report::ContractSource::File,
        ));
        let addr: Addr = "site/probe".parse().expect("an address");
        let iface: IfaceId = "probe.v1".parse().expect("an interface");
        let instance = InstanceId::from_u64(1);
        let keys = vec![
            instance_key(&addr, &instance).expect("a key").to_string(),
            alive_key(
                &addr,
                &iface,
                &instance,
                &revision.fingerprint().hex().fp16(),
            )
            .expect("a key")
            .to_string(),
        ];
        let mut descriptors = Map::new();
        descriptors.insert(
            (addr.clone(), instance.clone()),
            DescriptorRead::Served(Box::new(Descriptor {
                format: "zk2-descriptor/0.1".into(),
                service: addr.to_string(),
                instance: instance.to_string(),
                interfaces: vec![InterfaceEntry {
                    iface: iface.to_string(),
                    contract: revision.fingerprint().to_string(),
                    minor: 0,
                    standby: false,
                    token: true,
                    unavailable: Vec::new(),
                    cardinality: Map::new(),
                }],
                capabilities: Vec::new(),
                requires: Vec::new(),
                profiles: Vec::new(),
                meta: Map::new(),
            })),
        );
        let mut observed = Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
        observed.descriptors = Some(descriptors);
        let catalog = Arc::new(Catalog::new(&observed));
        let mut presence = LivePresence::new(zenkey_fleet::PresenceScope::all());
        presence.seed(&observed, since);
        let store = BundleStore::new(Duration::from_secs(1));
        store.seed(&set);
        Fixture {
            catalog,
            store,
            presence: Arc::new(presence),
        }
    }

    const M: &str = "zk2/site/probe/probe.v1/stream/m";

    /// A window that began at `start` and is judged at `now`, over `keys`
    /// heard at `heard`, under one `**` watch.
    fn inputs(
        f: &Fixture,
        keys: &[&str],
        start: Instant,
        heard: Instant,
        now: Instant,
        wall: SystemTime,
    ) -> BudgetInputs {
        let mut stats = StatsTable::new();
        for k in keys {
            stats.record(k, 8, None, heard, None, None);
        }
        BudgetInputs {
            namespace: String::new(),
            catalog: Some(Arc::clone(&f.catalog)),
            store: f.store.clone(),
            presence: Some(Arc::clone(&f.presence)),
            observed: Arc::new(KeyTreeSnapshot::build(&stats)),
            watches: vec![("**".into(), start)],
            reader: ReaderLoss::default(),
            timeout: Duration::from_secs(2),
            now,
            wall,
        }
    }

    fn members(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}/k{i}")).collect()
    }

    /// Two hours of a lossless window with its owner present throughout:
    /// two members of a bound of four is within, five is the finding, and
    /// the badge carries the numbers.
    #[test]
    fn a_whole_lossless_window_with_its_owner_present_is_answered() {
        let start = Instant::now();
        let now = start + Duration::from_secs(7200);
        let wall = SystemTime::now() + Duration::from_secs(7200);
        // Seen a moment before the window began: present throughout.
        let f = fixture(SystemTime::now() - Duration::from_secs(1));
        let heard = now - Duration::from_secs(60);

        let two = members(M, 2);
        let keys: Vec<&str> = two.iter().map(String::as_str).collect();
        let b = badges(&inputs(&f, &keys, start, heard, now, wall));
        let badge = b.get(M).expect("the family is badged");
        assert_eq!(badge.judged.verdict, Verdict::Within, "{badge:?}");
        assert_eq!(badge.label(), "within budget: 2 of 4");
        assert_eq!(badge.tone(), BudgetTone::Within);

        let five = members(M, 5);
        let keys: Vec<&str> = five.iter().map(String::as_str).collect();
        let b = badges(&inputs(&f, &keys, start, heard, now, wall));
        assert_eq!(
            b.get(M).map(BudgetBadge::label).as_deref(),
            Some("exceeds budget: 5 of 4")
        );
    }

    /// The window's premises, each its own reason: a best-effort stream is
    /// lossy, a lossless window shorter than an hour is too short, an owner
    /// whose instance appeared after the window began was not present
    /// throughout, and a state is not a window's to count.
    #[test]
    fn a_window_that_cannot_answer_says_why() {
        let start = Instant::now();
        let wall0 = SystemTime::now();
        let f = fixture(wall0);
        let keys = [
            "zk2/site/probe/probe.v1/stream/m/a",
            "zk2/site/probe/probe.v1/stream/lossy/a",
            "zk2/site/probe/probe.v1/state/s/a",
            "zk2/site/probe/probe.v1/stream/free/a",
        ];
        let now = start + Duration::from_secs(120);
        let b = badges(&inputs(
            &f,
            &keys,
            start,
            start + Duration::from_secs(30),
            now,
            wall0 + Duration::from_secs(120),
        ));
        let label = |p: &str| b.get(p).map(BudgetBadge::label);
        assert_eq!(
            label(M).as_deref(),
            Some("budget unobservable (window too short)")
        );
        assert_eq!(
            label("zk2/site/probe/probe.v1/stream/lossy").as_deref(),
            Some("budget unobservable (lossy)")
        );
        assert_eq!(
            label("zk2/site/probe/probe.v1/state/s").as_deref(),
            Some("budget unobservable (a window does not count state)")
        );
        assert_eq!(
            label("zk2/site/probe/probe.v1/stream/free").as_deref(),
            Some("budget not asked (no ceiling)")
        );

        // The owner's instance appeared an hour into a two-hour window.
        let late = fixture(SystemTime::now() + Duration::from_secs(3600));
        let now = start + Duration::from_secs(7200);
        let b = badges(&inputs(
            &late,
            &keys[..1],
            start,
            now - Duration::from_secs(60),
            now,
            SystemTime::now() + Duration::from_secs(7200),
        ));
        assert_eq!(
            b.get(M).map(BudgetBadge::label).as_deref(),
            Some("budget unobservable (owner absent)")
        );
    }

    /// The reader's own loss makes even a reliable, blocking window lossy:
    /// never assumed lossless (0.25), and a stall past the bound the same
    /// (0.27).
    #[test]
    fn the_readers_own_loss_and_stall_are_losses() {
        let start = Instant::now();
        let now = start + Duration::from_secs(7200);
        let wall = SystemTime::now() + Duration::from_secs(7200);
        // Seen a moment before the window began: present throughout.
        let f = fixture(SystemTime::now() - Duration::from_secs(1));
        let keys = ["zk2/site/probe/probe.v1/stream/m/a"];
        for reader in [
            ReaderLoss {
                lost: 3,
                gap: Duration::ZERO,
            },
            ReaderLoss {
                lost: 0,
                gap: Duration::from_secs(9),
            },
        ] {
            let mut i = inputs(&f, &keys, start, now - Duration::from_secs(60), now, wall);
            i.reader = reader;
            assert_eq!(
                badges(&i).get(M).map(BudgetBadge::label).as_deref(),
                Some("budget unobservable (lossy)"),
                "{reader:?}"
            );
        }
    }

    /// No watch covering the family whole, no window, no badge (O4) — a
    /// watch on a sibling family is not one over this one; and without a
    /// presence read nothing is badged at all.
    #[test]
    fn no_window_no_badge() {
        let start = Instant::now();
        // Seen a moment before the window began: present throughout.
        let f = fixture(SystemTime::now() - Duration::from_secs(1));
        let mut i = inputs(
            &f,
            &["zk2/site/probe/probe.v1/stream/m/a"],
            start,
            start,
            start + Duration::from_secs(120),
            SystemTime::now(),
        );
        i.watches = vec![("zk2/site/probe/probe.v1/stream/lossy/**".into(), start)];
        assert!(badges(&i).get(M).is_none());
        i.watches = vec![("zk2/site/probe/probe.v1/stream/m/a".into(), start)];
        assert!(badges(&i).get(M).is_none(), "half a family is no window");
        i.watches = vec![("**".into(), start)];
        i.catalog = None;
        assert!(badges(&i).is_empty());
    }

    /// The badge path carries the namespace, as the tree's display paths
    /// are full wire keys; and the weightier badge keeps a shared row.
    #[test]
    fn badge_paths_carry_the_namespace_and_the_finding_keeps_its_row() {
        let start = Instant::now();
        // Seen a moment before the window began: present throughout.
        let f = fixture(SystemTime::now() - Duration::from_secs(1));
        let mut i = inputs(
            &f,
            &["acme/zk2/site/probe/probe.v1/stream/m/a"],
            start,
            start,
            start + Duration::from_secs(120),
            SystemTime::now(),
        );
        i.namespace = "acme".into();
        assert!(badges(&i).get(&format!("acme/{M}")).is_some());

        let judged = |verdict, reason| BudgetBadge {
            bound: Bound::Of(4),
            judged: Judged {
                verdict,
                reason,
                counted: 5,
            },
        };
        let b: BudgetBadges = [
            ("p".to_owned(), judged(Verdict::Within, Reason::Complete)),
            ("p".to_owned(), judged(Verdict::Exceeds, Reason::OverBound)),
            ("p".to_owned(), judged(Verdict::Unobservable, Reason::Lossy)),
        ]
        .into_iter()
        .collect();
        assert_eq!(b.get("p").map(|b| b.judged.verdict), Some(Verdict::Exceeds));
    }
}
