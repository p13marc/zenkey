//! `instance-gone`'s reading of the live presence projection (#747): per
//! address a pattern matches, held or gone, as one watcher of the pattern
//! has seen it. One implementation, two callers: zenctl's watchdog and
//! `record --on` fold it into one state per rule
//! ([`crate::judge::condition::judge_instance_gone`]), and zenwatch turns
//! each of its moves into a notification of its own.
//!
//! **Per address, and only on evidence.** An address is *held* while the
//! projection has it [up](AddressStatus::Up) — an instance token held — and
//! *gone* when the projection has it [down](AddressStatus::Down): none
//! held, a complete read behind the claim (spec §8.1). An address the
//! projection cannot see ([unobservable](AddressStatus::Unobservable): no
//! seed yet, a seed that ended at its timeout, events lost to a
//! `Dropped(n)`) moves nothing here: what was last said of it stands, and
//! the projection's basis is the reader's to report ([`LivePresence::basis`]).
//! So an address that went before a loss is still gone across it, and one
//! whose going was hidden in the loss is found by the re-read that restores
//! the basis.
//!
//! **What a pattern can call gone.** Under a wildcard (`*/tc`, `host-a/*`)
//! an address is watched from the first time it is seen held: a second
//! address appearing under the pattern is one more address, and its going
//! is gone even while others hold. Nothing can be gone that was never seen
//! held, so a pattern that never matched calls nothing gone. An exact
//! address (`host-a/tc`) is watched from the start, tracked on the
//! projection ([`AddressWatch::track`]) so a complete read that shows none
//! of it calls it down: the dead-man's switch.
//!
//! Session-free: the projection is fed by [`crate::bus::presence::PresenceFeed`]
//! live, or by the same values from a file.

use std::collections::BTreeMap;
use std::time::SystemTime;

use zenkey_model::grammar::Addr;

use crate::bus::presence::Scope;
use crate::model::presence_live::LivePresence;
use crate::model::target::Target;
use crate::report::{AddressStatus, PresenceChange, PresenceTransition};

/// One watcher's reading of the addresses a pattern matches: what it last
/// said of each, held (`true`) or gone (`false`). See the module doc.
#[derive(Debug, Clone)]
pub struct AddressWatch {
    target: Target,
    said: BTreeMap<Addr, bool>,
}

/// One address whose reading moved.
#[derive(Debug, Clone, PartialEq)]
pub struct AddressMove {
    pub address: Addr,
    /// Whether it holds an instance token now; `false` is gone.
    pub held: bool,
    /// What the watch last said of it: `None` the first time, which only an
    /// exact address gone can be (a held one seen first is remembered, not
    /// moved).
    pub prior: Option<bool>,
    /// The projection's move that decided it: heard as it happened, or
    /// found by a re-read.
    pub by: PresenceTransition,
}

impl AddressWatch {
    pub fn new(target: Target) -> AddressWatch {
        AddressWatch {
            target,
            said: BTreeMap::new(),
        }
    }

    /// The pattern watched.
    pub fn target(&self) -> &Target {
        &self.target
    }

    /// Whether the pattern is one address, watched from the start.
    pub fn is_exact(&self) -> bool {
        self.target.concrete.is_some()
    }

    /// Track the pattern on `presence`, before it is seeded: every address
    /// it matches keeps its record, and an exact one has a record now, which
    /// a complete seed can call down ([`LivePresence::track_pattern`]).
    pub fn track(&self, presence: &mut LivePresence, at: SystemTime) {
        presence.track_pattern(&self.target, at);
    }

    /// The first look, once `presence` is seeded: every address the pattern
    /// matches that it has up is held, and an exact address it has down is
    /// gone; each is said from here. Under a wildcard a record down is one
    /// another watcher tracked, never one this watch saw held, so it is
    /// left out, and so is every address the projection cannot see.
    pub fn first_look(&mut self, presence: &LivePresence) -> BTreeMap<Addr, bool> {
        let mut out = BTreeMap::new();
        for (addr, record) in presence.matching(&self.target) {
            let held = match record.status {
                AddressStatus::Up => true,
                AddressStatus::Down if self.is_exact() => false,
                AddressStatus::Down | AddressStatus::Unobservable => continue,
            };
            self.said.insert(addr.clone(), held);
            out.insert(addr.clone(), held);
        }
        out
    }

    /// Remember `address` gone with no move to show for it: a notifier's
    /// ledger after a restart, so that its return is news.
    pub fn remember_gone(&mut self, address: Addr) {
        self.said.insert(address, false);
    }

    /// Fold one batch of the projection's moves, in order: each address of
    /// the pattern whose reading changed, with what was said before and the
    /// move that decided it. A held address seen for the first time is
    /// remembered and not moved; a gone one is moved only when the pattern
    /// is exact.
    pub fn fold(&mut self, moved: &[PresenceTransition]) -> Vec<AddressMove> {
        let mut out = Vec::new();
        for t in moved {
            if !self.target.matches(&t.address) {
                continue;
            }
            let held = match &t.change {
                PresenceChange::Status {
                    to: AddressStatus::Up,
                    ..
                } => true,
                PresenceChange::Status {
                    to: AddressStatus::Down,
                    ..
                } => false,
                _ => continue,
            };
            let prior = self.said.get(&t.address).copied();
            if prior == Some(held) {
                continue;
            }
            if prior.is_none() && (held || !self.is_exact()) {
                if held {
                    self.said.insert(t.address.clone(), true);
                }
                continue;
            }
            self.said.insert(t.address.clone(), held);
            out.push(AddressMove {
                address: t.address.clone(),
                held,
                prior,
                by: t.clone(),
            });
        }
        out
    }

    /// What the watch last said of `address`.
    pub fn said(&self, address: &Addr) -> Option<bool> {
        self.said.get(address).copied()
    }

    /// The addresses it says are gone, in order.
    pub fn gone(&self) -> impl Iterator<Item = &Addr> {
        self.said
            .iter()
            .filter(|(_, held)| !**held)
            .map(|(addr, _)| addr)
    }

    /// The addresses it says are held, in order.
    pub fn held(&self) -> impl Iterator<Item = &Addr> {
        self.said
            .iter()
            .filter(|(_, held)| **held)
            .map(|(addr, _)| addr)
    }
}

/// The narrowest presence scope covering every pattern in `targets`: one
/// service when they all name it, one system when they all name a service
/// of it, else every service.
pub fn covering_scope<'t>(targets: impl IntoIterator<Item = &'t Target>) -> Scope {
    let mut targets = targets.into_iter();
    let Some(first) = targets.next() else {
        return Scope::all();
    };
    let system = |t: &Target| t.address.split('/').next().map(str::to_owned);
    let (mut one, mut sys) = (first.concrete.clone(), system(first));
    for t in targets {
        if one.is_some() && one != t.concrete {
            one = None;
        }
        if sys != system(t) {
            sys = None;
        }
    }
    match (one, sys) {
        (Some(addr), _) => Scope::service(&addr),
        (None, Some(s)) if s != "*" => Scope::system(&s).unwrap_or_else(|_| Scope::all()),
        _ => Scope::all(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::catalog::Observed;
    use std::time::Duration;
    use zenkey_model::grammar::{InstanceId, instance_key};

    fn addr(s: &str) -> Addr {
        s.parse().expect("an address")
    }

    fn target(s: &str) -> Target {
        Target::parse(s).expect("a pattern")
    }

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(s)
    }

    fn instance(a: &str, i: u64) -> String {
        instance_key(&addr(a), &InstanceId::from_u64(i))
            .expect("a key")
            .as_str()
            .to_owned()
    }

    fn seed(keys: &[String], complete: bool) -> Observed {
        Observed::from_keys(Scope::all().selector(), keys, complete)
    }

    fn moves(m: &[AddressMove]) -> Vec<(String, bool, Option<bool>)> {
        m.iter()
            .map(|m| (m.address.to_string(), m.held, m.prior))
            .collect()
    }

    /// Under a wildcard: what is up at the first look is held, a second
    /// address appearing is remembered, not moved, and each one's going is
    /// gone even while the other holds; its return is held again.
    #[test]
    fn a_wildcard_watches_every_address_it_saw_held() {
        let mut p = LivePresence::new(Scope::all());
        let mut w = AddressWatch::new(target("*/tc"));
        w.track(&mut p, at(0));
        p.seed(&seed(&[instance("h1/tc", 1)], true), at(1));
        assert_eq!(w.first_look(&p), BTreeMap::from([(addr("h1/tc"), true)]));
        let t = p.token(&instance("h2/tc", 2), true, at(2));
        assert!(w.fold(&t).is_empty(), "held at first sight is not a move");
        assert_eq!(w.said(&addr("h2/tc")), Some(true));
        let t = p.token(&instance("h2/tc", 2), false, at(3));
        assert_eq!(moves(&w.fold(&t)), [("h2/tc".into(), false, Some(true))]);
        assert_eq!(w.gone().collect::<Vec<_>>(), [&addr("h2/tc")]);
        assert_eq!(w.held().collect::<Vec<_>>(), [&addr("h1/tc")]);
        let t = p.token(&instance("h2/tc", 3), true, at(4));
        assert_eq!(moves(&w.fold(&t)), [("h2/tc".into(), true, Some(false))]);
        assert_eq!(w.gone().count(), 0);
        // Another pattern's address moves nothing here.
        let t = p.token(&instance("h1/gui", 9), true, at(5));
        assert!(w.fold(&t).is_empty());
        assert_eq!(w.said(&addr("h1/gui")), None);
    }

    /// A wildcard that never matched calls nothing gone, under a complete
    /// read; an exact address absent from one is gone at the first look.
    #[test]
    fn nothing_seen_is_nothing_gone_but_an_exact_address_is_watched_from_the_start() {
        let mut p = LivePresence::new(Scope::all());
        let mut wild = AddressWatch::new(target("*/tc"));
        let mut exact = AddressWatch::new(target("h9/tc"));
        wild.track(&mut p, at(0));
        exact.track(&mut p, at(0));
        p.seed(&seed(&[instance("h1/gui", 1)], true), at(1));
        assert!(wild.first_look(&p).is_empty());
        assert_eq!(wild.gone().count(), 0);
        assert_eq!(
            exact.first_look(&p),
            BTreeMap::from([(addr("h9/tc"), false)])
        );
        // The wildcard does not claim the exact watch's record.
        assert_eq!(wild.said(&addr("h9/tc")), None);
        // Its start is held, its going gone again.
        let t = p.token(&instance("h9/tc", 1), true, at(2));
        assert_eq!(
            moves(&exact.fold(&t)),
            [("h9/tc".into(), true, Some(false))]
        );
        assert!(wild.fold(&t).is_empty(), "first sight under the wildcard");
    }

    /// An exact address the first look could not see (a seed possibly
    /// incomplete) is said nothing of; the complete re-read that calls it
    /// down is its first move, with no prior.
    #[test]
    fn an_exact_address_first_seen_down_by_a_reread_is_a_move() {
        let mut p = LivePresence::new(Scope::all());
        let mut w = AddressWatch::new(target("h9/tc"));
        w.track(&mut p, at(0));
        p.seed(&seed(&[], false), at(1));
        assert!(w.first_look(&p).is_empty(), "unobservable: nothing said");
        let t = p.seed(&seed(&[], true), at(2));
        assert_eq!(moves(&w.fold(&t)), [("h9/tc".into(), false, None)]);
    }

    /// A loss moves nothing: what was said stands across it, and the
    /// re-read that restores the basis finds what the loss hid.
    #[test]
    fn a_loss_moves_nothing_and_the_reread_finds_what_it_hid() {
        let mut p = LivePresence::new(Scope::all());
        let mut w = AddressWatch::new(target("*/tc"));
        w.track(&mut p, at(0));
        p.seed(
            &seed(&[instance("h1/tc", 1), instance("h2/tc", 2)], true),
            at(1),
        );
        w.first_look(&p);
        let t = p.token(&instance("h2/tc", 2), false, at(2));
        assert_eq!(moves(&w.fold(&t)), [("h2/tc".into(), false, Some(true))]);
        let t = p.dropped(4, at(3));
        assert!(w.fold(&t).is_empty(), "unobservable is no move");
        assert_eq!(w.gone().collect::<Vec<_>>(), [&addr("h2/tc")]);
        let t = p.seed(&seed(&[instance("h2/tc", 5)], true), at(4));
        assert_eq!(
            moves(&w.fold(&t)),
            [
                ("h1/tc".into(), false, Some(true)),
                ("h2/tc".into(), true, Some(false)),
            ]
        );
    }

    /// The scope a set of patterns needs: one service, one system, or all.
    #[test]
    fn the_covering_scope_is_the_narrowest_that_holds_every_pattern() {
        let scope = |ts: &[&str]| {
            let ts: Vec<Target> = ts.iter().map(|t| target(t)).collect();
            covering_scope(&ts).selector()
        };
        assert_eq!(scope(&["h1/tc"]), "zk2/h1/tc/@zk/**");
        assert_eq!(scope(&["h1/tc", "h1/tc"]), "zk2/h1/tc/@zk/**");
        assert_eq!(scope(&["h1/tc", "h1/gui"]), "zk2/h1/*/@zk/**");
        assert_eq!(scope(&["h1/*"]), "zk2/h1/*/@zk/**");
        assert_eq!(scope(&["*/tc"]), "zk2/*/*/@zk/**");
        assert_eq!(scope(&["h1/tc", "h2/tc"]), "zk2/*/*/@zk/**");
        assert_eq!(scope(&[]), "zk2/*/*/@zk/**");
    }
}
