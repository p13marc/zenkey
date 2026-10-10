//! `liveliness-gone` on tokens that are not the deployment's zk2 presence
//! (#614, FL2): another application's on the shared bus (spec §1.7),
//! another namespace's, a shape the projection's reading does not cover.
//! The daemon keeps its own ledger of them: one state per token key, a
//! token that disappears is firing, one that comes back is ok — the
//! dead-man's switch, as v1 had it.
//!
//! **Seeded, and honest about the seed** (spec §8.1). The ledger is seeded
//! by one liveliness read per selector, through the fleet's chokepoint
//! ([`zenkey_fleet::liveliness_read`], the unbounded handler), and a read
//! that ran to its timeout is **possibly incomplete**: what it showed is
//! held, what it did not is not known to be absent, and the rule's observer
//! says `unobservable` until a complete read lands. The roster ask v1's
//! engine made here never noticed it had run to its timeout, and took a
//! partial roster for a whole one; this is the bug [`TokenFeed`] replaces,
//! in [`zenkey_fleet::PresenceFeed`]'s order: the event stream is taken
//! before the seed is read, and what it delivered while the read was in
//! flight is folded into the seed in one step.
//!
//! **A loss is unobservable, never ok.** A `Dropped(n)` from the monitor
//! may have carried a token's going or its return, so every rule's observer
//! goes `unobservable` and the ledger is read again; a token that went
//! inside the loss is found gone by the complete re-read, one that came
//! back found held. Cancel-safe like the presence feed: transitions are
//! held in the feed until a call hands them out, and a re-read is owed
//! before it is read.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use zenkey_fleet::{
    CondState, EventStream, FleetEvent, PresenceSource, RESEED_BACKOFF, RESEED_BACKOFF_MAX,
    StreamItem,
};
use zenoh::Session;
use zenoh::key_expr::KeyExpr;

use crate::engine::RuleNotice;
use crate::presence::{BASELINE, OBSERVER, presence_form};
use crate::rules::{Rule, RuleKind};
use crate::sinks::NoticeKind;

/// What the ledger's answers stand on, as the projection's basis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenBasis {
    /// Not read yet.
    Unseeded,
    /// The last read ended at its timeout (or failed), and no complete one
    /// stands: a token it did not show may be held.
    Incomplete,
    /// A complete read stands, and nothing was lost since.
    Complete,
    /// Events lost since the last complete read: what is held is the last
    /// known, and a token may have gone or come back unseen.
    Lost { dropped: u64 },
}

/// One token's move, as the ledger folded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenMove {
    pub at: SystemTime,
    pub source: PresenceSource,
    /// The token's wire key.
    pub key: String,
    /// Held now (it appeared or came back) or not (it went).
    pub held: bool,
}

/// How many reads one re-read makes while each one loses events.
const RESEED_ATTEMPTS: usize = 3;

/// The daemon's own token ledger, kept current from a monitor watching
/// `selectors` on a session in no namespace (see the module doc).
pub struct TokenFeed {
    session: Session,
    selectors: Vec<String>,
    timeout: Duration,
    events: EventStream,
    held: BTreeSet<String>,
    basis: TokenBasis,
    retry: Option<(tokio::time::Instant, Duration)>,
    pending: Vec<TokenMove>,
}

impl std::fmt::Debug for TokenFeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenFeed")
            .field("selectors", &self.selectors)
            .field("held", &self.held.len())
            .field("basis", &self.basis)
            .finish_non_exhaustive()
    }
}

enum Wake {
    Item(Option<StreamItem>),
    Retry,
}

impl TokenFeed {
    /// Seed the ledger and start folding `events` (taken from the monitor
    /// before this is called). Returns the feed; its first read's moves
    /// are not news — [`TokenFeed::held`] and [`TokenFeed::basis`] are the
    /// baseline.
    pub async fn open(
        session: &Session,
        events: EventStream,
        selectors: Vec<String>,
        timeout: Duration,
    ) -> TokenFeed {
        let mut feed = TokenFeed {
            session: session.clone(),
            selectors,
            timeout,
            events,
            held: BTreeSet::new(),
            basis: TokenBasis::Unseeded,
            retry: None,
            pending: Vec::new(),
        };
        feed.reseed().await;
        feed.pending.clear();
        feed
    }

    /// The tokens held, by wire key.
    pub fn held(&self) -> &BTreeSet<String> {
        &self.held
    }

    pub fn basis(&self) -> TokenBasis {
        self.basis
    }

    /// The next moves: an event that moves a token, or a re-read that is
    /// owed. `None` only when the monitor stopped. Cancel-safe.
    pub async fn next(&mut self) -> Option<Vec<TokenMove>> {
        loop {
            if !self.pending.is_empty() {
                return Some(std::mem::take(&mut self.pending));
            }
            let wake = match self.retry {
                Some((when, _)) => tokio::select! {
                    biased;
                    () = tokio::time::sleep_until(when) => Wake::Retry,
                    item = self.events.recv() => Wake::Item(item),
                },
                None => Wake::Item(self.events.recv().await),
            };
            match wake {
                Wake::Retry => self.reseed().await,
                Wake::Item(None) => return None,
                Wake::Item(Some(StreamItem::Event(FleetEvent::NodeUp(key)))) => {
                    self.fold(key, true, PresenceSource::Event, SystemTime::now());
                }
                Wake::Item(Some(StreamItem::Event(FleetEvent::NodeDown(key)))) => {
                    self.fold(key, false, PresenceSource::Event, SystemTime::now());
                }
                Wake::Item(Some(StreamItem::Event(_))) => {}
                Wake::Item(Some(StreamItem::Dropped(n))) => {
                    self.lost(n);
                    // A basis change is a move too: hand it out, so the
                    // rules' observers say so before the re-read lands.
                    return Some(Vec::new());
                }
            }
        }
    }

    fn lost(&mut self, n: u64) {
        self.basis = match self.basis {
            TokenBasis::Lost { dropped } => TokenBasis::Lost {
                dropped: dropped.saturating_add(n),
            },
            _ => TokenBasis::Lost { dropped: n },
        };
        self.owe_now();
    }

    fn fold(&mut self, key: String, held: bool, source: PresenceSource, at: SystemTime) {
        let changed = if held {
            self.held.insert(key.clone())
        } else {
            // A delete for a token never seen held is still positive
            // evidence that it went.
            self.held.remove(&key) || source == PresenceSource::Event
        };
        if changed {
            self.pending.push(TokenMove {
                at,
                source,
                key,
                held,
            });
        }
    }

    /// Read every selector again and fold the reads, with what the
    /// subscription delivered meanwhile. Owed until a complete read lands.
    async fn reseed(&mut self) {
        if self.retry.is_none() {
            self.owe_now();
        }
        for _ in 0..RESEED_ATTEMPTS {
            let mut keys = BTreeSet::new();
            let mut complete = true;
            for sel in &self.selectors {
                match zenkey_fleet::liveliness_read(&self.session, sel, self.timeout).await {
                    Ok(read) => {
                        complete &= read.complete;
                        keys.extend(read.keys);
                    }
                    Err(e) => {
                        tracing::warn!(
                            selector = sel.as_str(),
                            "liveliness read failed ({}); possibly incomplete",
                            zenkey_fleet::one_line(&e)
                        );
                        complete = false;
                    }
                }
            }
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
            if lost > 0 {
                self.lost(lost);
                continue;
            }
            let at = SystemTime::now();
            let mut next: BTreeSet<String> = if complete {
                keys
            } else {
                // Absence from a possibly incomplete read is not evidence.
                self.held.iter().cloned().chain(keys).collect()
            };
            for (key, held) in then {
                if held {
                    next.insert(key);
                } else {
                    next.remove(&key);
                }
            }
            let gone: Vec<String> = self.held.difference(&next).cloned().collect();
            let back: Vec<String> = next.difference(&self.held).cloned().collect();
            for key in gone {
                self.fold(key, false, PresenceSource::Seed, at);
            }
            for key in back {
                self.fold(key, true, PresenceSource::Seed, at);
            }
            self.basis = match (complete, self.basis) {
                (true, _) => TokenBasis::Complete,
                (false, b @ (TokenBasis::Lost { .. } | TokenBasis::Complete)) => b,
                (false, _) => TokenBasis::Incomplete,
            };
            if complete {
                self.retry = None;
            } else {
                self.back_off();
            }
            return;
        }
        self.back_off();
    }

    fn owe_now(&mut self) {
        let last = self.retry.map_or(Duration::ZERO, |(_, last)| last);
        self.retry = Some((tokio::time::Instant::now(), last));
    }

    fn back_off(&mut self) {
        let wait = match self.retry {
            Some((_, last)) if !last.is_zero() => (last * 2).min(RESEED_BACKOFF_MAX),
            _ => RESEED_BACKOFF,
        };
        self.retry = Some((tokio::time::Instant::now() + wait, wait));
    }
}

/// One foreign `liveliness-gone` rule and what it last said.
#[derive(Debug)]
struct ForeignRule {
    name: String,
    selector: String,
    keyexpr: KeyExpr<'static>,
    /// Per token key: `true` held, `false` gone.
    known: BTreeMap<String, bool>,
    observer: Option<CondState>,
}

/// Every foreign `liveliness-gone` rule of a config, over one ledger.
#[derive(Debug)]
pub struct LivelinessRules {
    rules: Vec<ForeignRule>,
}

impl LivelinessRules {
    /// The `liveliness-gone` rules whose selector is not the deployment's
    /// own zk2 presence in `namespace` ([`presence_form`]).
    pub fn new(rules: &[Rule], namespace: &str) -> LivelinessRules {
        LivelinessRules {
            rules: rules
                .iter()
                .filter_map(|r| match (&r.kind, &r.keyexpr) {
                    (RuleKind::LivelinessGone { selector }, Some(k))
                        if presence_form(selector, namespace).is_none() =>
                    {
                        Some(ForeignRule {
                            name: r.name.clone(),
                            selector: selector.clone(),
                            keyexpr: k.clone(),
                            known: BTreeMap::new(),
                            observer: None,
                        })
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The distinct selectors to watch and seed, in rule order.
    pub fn selectors(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for r in &self.rules {
            if !out.contains(&r.selector) {
                out.push(r.selector.clone());
            }
        }
        out
    }

    /// The first look: the tokens held are the baseline (never "came
    /// back"), reconciled against what the ledger remembers announced —
    /// held now is a resolve, still absent under a complete read is a
    /// duplicate — and an observer that cannot see says so once.
    pub fn baseline(
        &mut self,
        held: &BTreeSet<String>,
        basis: TokenBasis,
        announced: &BTreeMap<String, Vec<String>>,
        rule_ids: &BTreeMap<String, String>,
        at: &str,
    ) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        for r in &mut self.rules {
            for key in held {
                if covers(&r.keyexpr, key) {
                    r.known.insert(key.clone(), true);
                }
            }
            let id = rule_ids.get(&r.name).cloned().unwrap_or_default();
            for remembered in announced.get(&r.name).into_iter().flatten() {
                let Some(key) = remembered
                    .strip_prefix(&id)
                    .and_then(|s| s.strip_prefix(':'))
                else {
                    continue;
                };
                if key == OBSERVER {
                    continue;
                }
                if held.contains(key) {
                    out.push(r.notice(key, CondState::Ok, None, false, None, at));
                    continue;
                }
                // Remembered gone either way, so that its return is the
                // resolve it owes; assumed gone when a complete read says so.
                r.known.insert(key.to_string(), false);
                if basis == TokenBasis::Complete {
                    out.push(r.notice(key, CondState::Firing, None, true, None, at));
                }
            }
            let observer = observer_state(basis);
            r.observer = Some(observer);
            if observer == CondState::Unobservable {
                out.push(r.observer_notice(observer, None, basis, true, at));
                out.push(r.baseline_notice(held, basis, at));
            }
        }
        out
    }

    /// Fold one batch of the ledger's moves into notices, the rules'
    /// sight read off `basis` after it.
    pub fn fold(&mut self, moved: &[TokenMove], basis: TokenBasis, at: &str) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        for r in &mut self.rules {
            for m in moved {
                if !covers(&r.keyexpr, &m.key) {
                    continue;
                }
                let prior = r.known.get(&m.key).copied();
                if prior == Some(m.held) {
                    continue;
                }
                r.known.insert(m.key.clone(), m.held);
                // A token first seen held is not news: remembered, so that
                // its going can say it went.
                if prior.is_none() && m.held {
                    continue;
                }
                let state = if m.held {
                    CondState::Ok
                } else {
                    CondState::Firing
                };
                let prior = prior.map(|h| if h { CondState::Ok } else { CondState::Firing });
                out.push(r.notice(&m.key, state, prior, false, Some(m.source), at));
            }
            let observer = observer_state(basis);
            if r.observer != Some(observer) {
                let prior = r.observer;
                r.observer = Some(observer);
                out.push(r.observer_notice(observer, prior, basis, false, at));
            }
        }
        out
    }
}

fn covers(keyexpr: &KeyExpr<'static>, key: &str) -> bool {
    KeyExpr::try_from(key).is_ok_and(|k| keyexpr.intersects(&k))
}

fn observer_state(basis: TokenBasis) -> CondState {
    match basis {
        TokenBasis::Complete => CondState::Ok,
        _ => CondState::Unobservable,
    }
}

impl ForeignRule {
    fn notice(
        &self,
        key: &str,
        state: CondState,
        prior: Option<CondState>,
        assume: bool,
        source: Option<PresenceSource>,
        at: &str,
    ) -> RuleNotice {
        let how = match source {
            Some(PresenceSource::Event) => " — heard as it happened",
            Some(PresenceSource::Seed) => " — found by a re-read: it moved at or before then",
            _ => "",
        };
        let evidence = if state == CondState::Ok {
            format!("the liveliness token {key} is held{how}")
        } else {
            format!("the liveliness token {key} is gone{how}")
        };
        RuleNotice {
            rule: self.name.clone(),
            what: key.to_string(),
            kind: NoticeKind::Liveliness,
            state,
            prior,
            severity: None,
            evidence,
            lines: Vec::new(),
            key: Some(key.to_string()),
            address: None,
            at: at.to_string(),
            presence: None,
            assume,
            passthrough: false,
            reason: "a liveliness token carries no payload",
        }
    }

    fn observer_notice(
        &self,
        state: CondState,
        prior: Option<CondState>,
        basis: TokenBasis,
        assume: bool,
        at: &str,
    ) -> RuleNotice {
        let sel = &self.selector;
        let evidence = match basis {
            TokenBasis::Complete => {
                format!("a complete liveliness read of {sel} stands: observable again")
            }
            TokenBasis::Lost { dropped } => format!(
                "the observer lost {dropped} liveliness event(s) on {sel}: a token's going or \
                 return inside that span was not seen — unobservable, not ok (RFC 13 §3 O6); \
                 re-reading"
            ),
            TokenBasis::Incomplete => format!(
                "the liveliness read of {sel} ended at its timeout, possibly incomplete (spec \
                 §8.1): a token it did not show is not known to be absent; re-reading"
            ),
            TokenBasis::Unseeded => format!("{sel} has not been read yet"),
        };
        RuleNotice {
            rule: self.name.clone(),
            what: OBSERVER.to_string(),
            kind: NoticeKind::Unobservable,
            state,
            prior,
            severity: None,
            evidence,
            lines: Vec::new(),
            key: None,
            address: None,
            at: at.to_string(),
            presence: None,
            assume,
            passthrough: false,
            reason: "nothing was observed",
        }
    }

    fn baseline_notice(&self, held: &BTreeSet<String>, basis: TokenBasis, at: &str) -> RuleNotice {
        let n = held.iter().filter(|k| covers(&self.keyexpr, k)).count();
        RuleNotice {
            rule: self.name.clone(),
            what: BASELINE.to_string(),
            kind: NoticeKind::Baseline,
            state: observer_state(basis),
            prior: None,
            severity: Some("info".into()),
            evidence: format!(
                "liveliness-gone {}: {n} token(s) held at the first look, which could not be \
                 completed — a token it did not show is unobservable until a complete read",
                self.selector
            ),
            lines: Vec::new(),
            key: None,
            address: None,
            at: at.to_string(),
            presence: None,
            assume: false,
            passthrough: true,
            reason: "a baseline is a count, not a payload",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Vec<Rule> {
        ["demo/held/*/alive", "prod/zk2/*/tc/@zk/instance/*"]
            .iter()
            .enumerate()
            .map(|(i, sel)| {
                Rule::from_config(&crate::config::RuleConfig {
                    name: format!("r{i}"),
                    rule: format!("liveliness-gone {sel}"),
                    severity: None,
                    labels: Default::default(),
                    sinks: vec!["ops".into()],
                    for_s: None,
                })
                .unwrap()
            })
            .collect()
    }

    fn ids(rules: &[Rule]) -> BTreeMap<String, String> {
        rules
            .iter()
            .map(|r| (r.name.clone(), r.id.clone()))
            .collect()
    }

    fn mv(key: &str, held: bool) -> TokenMove {
        TokenMove {
            at: SystemTime::UNIX_EPOCH,
            source: PresenceSource::Event,
            key: key.into(),
            held,
        }
    }

    fn summary(out: &[RuleNotice]) -> Vec<(String, CondState, Option<CondState>, bool)> {
        out.iter()
            .map(|n| (n.what.clone(), n.state, n.prior, n.assume))
            .collect()
    }

    /// Which rules are foreign: the deployment's own zk2 presence rides the
    /// projection, everything else (here, under `prod`) is this ledger's;
    /// under another namespace, the zk2 selector is foreign too.
    #[test]
    fn foreign_rules_are_what_the_projection_does_not_read() {
        let r = rules();
        assert_eq!(
            LivelinessRules::new(&r, "prod").selectors(),
            ["demo/held/*/alive"]
        );
        assert_eq!(
            LivelinessRules::new(&r, "staging").selectors(),
            ["demo/held/*/alive", "prod/zk2/*/tc/@zk/instance/*"]
        );
    }

    /// The poles of a foreign token: held at the first look is the
    /// baseline (no notice); gone is firing from ok; back is ok from
    /// firing; a token the selector does not cover moves nothing; a loss
    /// is the observer unobservable — never an ok — and the complete
    /// re-read's moves and observer follow.
    #[test]
    fn a_foreign_token_goes_comes_back_and_a_loss_is_unobservable() {
        let r = rules();
        let mut watch = LivelinessRules::new(&r, "prod");
        let held = BTreeSet::from(["demo/held/p1/alive".to_string()]);
        let out = watch.baseline(&held, TokenBasis::Complete, &BTreeMap::new(), &ids(&r), "t");
        assert!(
            out.is_empty(),
            "held at the first look is the baseline: {out:?}"
        );

        let out = watch.fold(
            &[mv("demo/held/p1/alive", false), mv("other/x/alive", false)],
            TokenBasis::Complete,
            "t",
        );
        assert_eq!(
            summary(&out),
            [(
                "demo/held/p1/alive".into(),
                CondState::Firing,
                Some(CondState::Ok),
                false
            )]
        );
        assert_eq!(out[0].key.as_deref(), Some("demo/held/p1/alive"));
        assert!(out[0].evidence.contains("is gone"), "{}", out[0].evidence);
        let out = watch.fold(&[mv("demo/held/p1/alive", true)], TokenBasis::Complete, "t");
        assert_eq!(
            summary(&out),
            [(
                "demo/held/p1/alive".into(),
                CondState::Ok,
                Some(CondState::Firing),
                false
            )]
        );
        let out = watch.fold(&[], TokenBasis::Lost { dropped: 4 }, "t");
        assert_eq!(
            summary(&out),
            [(
                "observer".into(),
                CondState::Unobservable,
                Some(CondState::Ok),
                false
            )]
        );
        assert!(out[0].evidence.contains("lost 4"), "{}", out[0].evidence);
        let gone = TokenMove {
            source: PresenceSource::Seed,
            ..mv("demo/held/p1/alive", false)
        };
        let out = watch.fold(&[gone], TokenBasis::Complete, "t");
        assert_eq!(
            summary(&out),
            [
                (
                    "demo/held/p1/alive".into(),
                    CondState::Firing,
                    Some(CondState::Ok),
                    false
                ),
                (
                    "observer".into(),
                    CondState::Ok,
                    Some(CondState::Unobservable),
                    false
                ),
            ]
        );
        assert!(out[0].evidence.contains("re-read"), "{}", out[0].evidence);
    }

    /// The first look against the ledger after a restart, and a first read
    /// that ran to its timeout: a remembered-gone token held now is a
    /// resolve, still absent under a complete read is assumed; possibly
    /// incomplete, nothing remembered is decided, and the observer and a
    /// baseline say why.
    #[test]
    fn the_first_look_is_honest_about_its_read() {
        let r = rules();
        let announced = BTreeMap::from([(
            "r0".to_string(),
            vec![
                "r0:demo/held/p1/alive".to_string(),
                "r0:demo/held/p2/alive".to_string(),
            ],
        )]);
        let held = BTreeSet::from(["demo/held/p1/alive".to_string()]);
        let mut watch = LivelinessRules::new(&r, "prod");
        let out = watch.baseline(&held, TokenBasis::Complete, &announced, &ids(&r), "t");
        assert_eq!(
            summary(&out),
            [
                ("demo/held/p1/alive".into(), CondState::Ok, None, false),
                ("demo/held/p2/alive".into(), CondState::Firing, None, true),
            ]
        );
        let mut watch = LivelinessRules::new(&r, "prod");
        let out = watch.baseline(&held, TokenBasis::Incomplete, &announced, &ids(&r), "t");
        assert_eq!(
            summary(&out),
            [
                ("demo/held/p1/alive".into(), CondState::Ok, None, false),
                ("observer".into(), CondState::Unobservable, None, true),
                ("baseline".into(), CondState::Unobservable, None, false),
            ]
        );
        assert!(out[2].passthrough);
        assert!(
            out[2].evidence.contains("could not be completed"),
            "{}",
            out[2].evidence
        );
    }
}
