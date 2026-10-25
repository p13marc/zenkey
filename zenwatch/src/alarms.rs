//! `alerts <SEL>` (#800, AD): the alarms a selector reaches, read from the
//! fleet's live `alarms.v1` reader ([`AlarmFeed`], in the deployment's
//! namespace), one notice per alarm key as §5's first question moves.
//!
//! **One state per alarm key**, as `instance-gone` is one per address: the
//! notice's identity is `<rule_id>:<key>`, the key base-relative, the
//! alarm's identity from its raise to its resolve (§2.1). Its state is "is
//! this alarm firing?" (§2.14), read off the fleet's book, never the
//! sample's kind (§2.6):
//! - **firing** while it reads *yes*: a fresh firing document of the
//!   present instance;
//! - **ok** on *no*: put resolved, deleted, or not held by its owner's
//!   alarms, known whole (§2.14 step 5);
//! - **unobservable** on every other answer — stale, its owner absent (its
//!   alarms last-known), an earlier instance's put, a put that does not
//!   read — never ok.
//!
//! **What notifies** (§2.16, text 0.4, which this rule settled). A move of
//! that state, and, while firing, a move of the document's severity (an
//! escalation, either way: §2.5). A refresh and a confirmation are not
//! transitions (§0): at the state and severity held they say nothing, and
//! nor does a raise of an alarm already said firing at its severity. The
//! severity is the document's, and the notification carries the fleet's
//! pinned [`AlarmTransition`], the document rendered through the contract
//! (`rendering: schema`).
//!
//! **What the reader has not established moves nothing.** An answer whose
//! reason is this reader's own sight ([`NOT_YET`]: a loss no GET has healed
//! yet, §2.14 step 4; a clock not yet trusted, `freshness.v1` §2.6; a
//! subscription not yet listened 900 s; presence or a descriptor not read
//! whole; nothing read yet, here or across a face; a GET that drew nothing
//! for the key — alarms.v1 §2.16's closed list, text 0.8) leaves the key
//! at its last word, and time or the next read settles it. A loss is the
//! reader's, not the keys': the rule's **observer** (`<rule_id>:observer`)
//! says it once — unobservable on a loss (deliveries refused, the feed's
//! stall), on a presence read or a GET that ran to its timeout, on a reader
//! that stopped — and `observable_again` when a complete GET after it
//! stands. A key firing stays firing through the loss, and the GET that
//! heals it decides it, as an address stays gone through a presence loss
//! ([`crate::presence`]).
//!
//! **An owner's absence is a read's.** §2.14 step 1's *absent* is "a
//! complete read without its instance token": the rule takes it from a
//! refresh's presence read, never from a live token's withdrawal alone,
//! which a router's restart makes of every owner at once (core §8.1, 0.37,
//! which the fleet's book now follows too: an owner heard withdrawn is
//! possibly incomplete until the next presence read). Each refresh judges
//! every key the book holds again, so an owner that did go is said a tick
//! later. A refresh through a session cut off from every router and peer
//! makes no read at all ([`zenkey_fleet::cut_off`]). The readers' instance
//! tokens are followed on a watch session of their own, the reads made on
//! the namespaced session (#830).
//!
//! **The first look** (decided against `presence.rs`'s baseline and v1's
//! `alerts`). An alarm already firing when zenwatch starts is **told**, as
//! v1's seeded watch told it: it is the owner's own positive word, read
//! fresh, with its document's `timestamp` saying when it was raised (§2.6, a
//! late reader's raise), and a notifier started at hour three of an outage
//! must say so. An address already down is assumed instead because nothing
//! can be gone that was never seen held; an alarm needs no such premise.
//! The state file dedups it across a restart (a duplicate, not news), and
//! reconciles the rest: a remembered key read resolved is a `resolved`; one
//! the reader holds no sample of is §2.14 step 5's *none* — resolved
//! (`not_held`) once its owner's alarms are known whole, unobservable when
//! its owner is absent, kept until then. What the first look could not
//! establish is said once, in a passthrough `baseline` notification per
//! rule (`info`), with the observer's sight and the owners counted by their
//! verdict, and each such alarm is told when it settles.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime};

use zenkey_fleet::model::alarms::answer_of;
use zenkey_fleet::report::{AlarmAnswer, AlarmAnswerToken, AlarmChange};
use zenkey_fleet::{
    AlarmBook, AlarmFeed, AlarmReadLog, AlarmRefresh, AlarmRefreshed, AlarmScope, AlarmSpec,
    AlarmTransition, CondState,
};
use zenkey_model::alarms::{Alarm, AlarmRef, Answer, Judged, Reason, SetVerdict, Severity, State};
use zenkey_model::grammar::Addr;

use crate::engine::RuleNotice;
use crate::publish::Observer;
use crate::rules::Rule;
use crate::sinks::NoticeKind;

/// The identity of the rule's own sight.
pub const OBSERVER: &str = "observer";
/// The identity of an `alerts` rule's baseline notification (passthrough).
pub const BASELINE: &str = "baseline";

/// §2.14 step 1's absent: taken from a presence read only (see
/// `AlarmRule::judge`).
const ABSENT: &str = "absent";

/// The reasons that say only what this reader has not established yet, a
/// loss included: a key's answer with one of them leaves the key at its
/// last word (see the module doc). alarms.v1 §2.16 closes the list (text
/// 0.7, F-140; `silent` since 0.8, F-141): every other answer — yes, no, and an unobservable or not
/// asked that says something of the alarm, its owner or the deployment
/// (`stale`, `absent`, `previous_instance`, `undecodable`, `face_closed`,
/// `no_stamp`, `clock_disagrees`, `not_a_horizon`, …) — is the key's.
pub const NOT_YET: [&str; 10] = [
    "incomplete",
    "presence_incomplete",
    "no_descriptor",
    "not_read",
    "nothing_crossed",
    "silent",
    "clock_untrusted",
    "listened_too_short",
    "near_horizon",
    "no_observation",
];

/// What an `alerts` notice carries beside its words (#800, AD).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertFacts {
    /// The fleet's transition, as pinned: what the notification carries.
    pub transition: AlarmTransition,
    /// The latest document this reader holds of the key, for the
    /// rendering: the transition's, or the book's latest when the change
    /// carried none (a verdict moved with no sample), or the last the rule
    /// saw (a delete).
    pub document: Option<Alarm>,
}

impl AlertFacts {
    /// The document's discriminating labels (`alarms.v1` §2.2): what an
    /// alert adds to its rule's labels. `host` and `host.*` are the
    /// system's to say.
    pub fn labels(&self) -> BTreeMap<String, String> {
        self.document
            .iter()
            .flat_map(|d| d.labels.iter())
            .filter(|(k, _)| !zenkey_model::alarms::is_host_scoped(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// The owner's stamp of the sample, when it carried one: the message's
    /// `timestamp:` line.
    pub fn timestamp(&self) -> Option<&str> {
        self.transition.stamp.as_ref().map(|s| s.time.as_str())
    }
}

/// A rule's state of an answer (§5): yes is firing, no is ok, anything
/// else unobservable — never ok.
pub fn state_of(a: AlarmAnswerToken) -> CondState {
    match a {
        AlarmAnswerToken::Yes => CondState::Firing,
        AlarmAnswerToken::No => CondState::Ok,
        AlarmAnswerToken::Unobservable | AlarmAnswerToken::NotAsked => CondState::Unobservable,
    }
}

/// Whether an answer is the key's, rather than this reader's sight not yet
/// established ([`NOT_YET`]).
pub fn settled(a: &AlarmAnswer) -> bool {
    matches!(a.answer, AlarmAnswerToken::Yes | AlarmAnswerToken::No)
        || !NOT_YET.contains(&a.reason.as_str())
}

/// What a reader's reads let its rules conclude: the observer's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sight {
    /// No GET has been folded yet.
    Unseeded,
    /// The last refresh's presence read or GET ran to its timeout, or could
    /// not be made, with why.
    Incomplete(String),
    /// Deliveries refused, or stalls, since the sight was last whole, and
    /// no complete GET asked after the latest (§2.14 step 4).
    Lost { lost: u64 },
    /// A complete presence read and GET stand, and nothing was lost since.
    Complete,
    /// The reader is not reading, with why.
    Stopped(String),
}

impl Sight {
    pub fn state(&self) -> CondState {
        match self {
            Sight::Complete => CondState::Ok,
            _ => CondState::Unobservable,
        }
    }

    /// The `alarms` health check's reading of it.
    pub fn observer(&self) -> Observer {
        match self {
            Sight::Unseeded => Observer::Unseeded,
            Sight::Incomplete(_) => Observer::Incomplete {
                cut_off_during: false,
            },
            Sight::Lost { lost } => Observer::Lost { dropped: *lost },
            Sight::Complete => Observer::Complete,
            Sight::Stopped(_) => Observer::Stopped,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Sight::Complete => 0,
            Sight::Unseeded => 1,
            Sight::Incomplete(_) => 2,
            Sight::Lost { .. } => 3,
            Sight::Stopped(_) => 4,
        }
    }

    /// The observer's words.
    pub fn evidence(&self, selector: &str) -> String {
        match self {
            Sight::Complete => format!(
                "the alarms under {selector} are read whole: a complete presence read and GET \
                 stand, and nothing was lost since (alarms.v1 §2.15)"
            ),
            Sight::Unseeded => {
                format!(
                    "the alarms under {selector} have not been read yet: nothing is established"
                )
            }
            Sight::Incomplete(why) => format!(
                "the alarms under {selector} were not read whole — {why} — so an alarm the \
                 read did not show may fire: unobservable, never ok; re-reading every tick"
            ),
            Sight::Lost { lost } => format!(
                "the reader of the alarms under {selector} lost {lost} delivery(ies) or stalled: \
                 every alarm under it is unobservable until a GET answers it (alarms.v1 §2.14 \
                 step 4), and each keeps its last word until then (RFC 13 §3 O6)"
            ),
            Sight::Stopped(why) => format!(
                "the alarms under {selector} are not being read: {why} — every alarm under it \
                 is unobservable, never ok"
            ),
        }
    }
}

/// One live reader, per distinct selector.
struct Reader {
    selector: String,
    feed: Result<AlarmFeed, String>,
    /// The book's loss count when the sight was last whole: a loss's count
    /// is since then.
    lost_whole: u64,
}

impl Reader {
    /// The sight, read off the feed: a reader not reading, an unhealed
    /// loss, the last refresh's reads, in that order.
    fn look(&mut self) -> Sight {
        let sight = match &self.feed {
            Err(why) => Sight::Stopped(why.clone()),
            Ok(f) => sight(f.book(), f.log(), self.lost_whole),
        };
        if sight == Sight::Complete
            && let Ok(f) = &self.feed
        {
            self.lost_whole = f.book().lost();
        }
        sight
    }
}

/// What a reader's book and its last refresh's reads let its rules
/// conclude: an unhealed loss first (counted since `lost_whole`, the loss
/// count when the sight was last whole), then the keys its bound refused,
/// then the presence read, then the GET.
pub fn sight(book: &AlarmBook, log: &AlarmReadLog, lost_whole: u64) -> Sight {
    if book.loss_unhealed() {
        return Sight::Lost {
            lost: book.lost().saturating_sub(lost_whole).max(1),
        };
    }
    if book.refused() > 0 {
        return Sight::Incomplete(format!(
            "{} alarm key(s) refused at the reader's bound of {} keys: an alarm past it is \
             never read",
            book.refused(),
            zenkey_fleet::model::alarms::MAX_KEYS
        ));
    }
    match &log.presence {
        Some(Err(e)) => {
            return Sight::Incomplete(format!("the presence read could not be made: {e}"));
        }
        Some(Ok(p)) if !p.complete => {
            return Sight::Incomplete(format!(
                "the presence read {}, possibly incomplete (core §8.1)",
                zenkey_fleet::model::catalog::incomplete_reason(
                    false,
                    p.cut_off_during,
                    "ended at its timeout"
                )
                .unwrap_or_default()
            ));
        }
        _ => {}
    }
    match &log.seed {
        None => return Sight::Unseeded,
        Some(Err(e)) => {
            return Sight::Incomplete(format!("the owners' GET could not be made: {e}"));
        }
        Some(Ok(false)) => {
            return Sight::Incomplete(
                "the owners' GET ended at its timeout, or with an error reply".into(),
            );
        }
        Some(Ok(true)) => {}
    }
    Sight::Complete
}

/// What a rule last said of one key.
#[derive(Debug, Clone, PartialEq)]
struct Said {
    state: CondState,
    severity: Option<String>,
    document: Option<Alarm>,
}

/// One `alerts` rule and what it last said.
#[derive(Debug)]
struct AlarmRule {
    name: String,
    id: String,
    reader: usize,
    said: BTreeMap<String, Said>,
    observer: Option<CondState>,
}

/// An entry the state file remembered announced, as the first look reads
/// it.
#[derive(Debug, Clone, PartialEq)]
pub struct Remembered {
    /// The notice identity, `<rule_id>:<what>`.
    pub id: String,
    pub state: CondState,
    pub severity: String,
}

/// Every `alerts` rule of a config, over one reader per distinct selector.
pub struct AlarmRules {
    readers: Vec<Reader>,
    rules: Vec<AlarmRule>,
}

impl std::fmt::Debug for AlarmRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlarmRules")
            .field(
                "selectors",
                &self.readers.iter().map(|r| &r.selector).collect::<Vec<_>>(),
            )
            .field("rules", &self.rules)
            .finish()
    }
}

impl AlarmRules {
    /// The `alerts` rules among `rules`, their readers not yet opened.
    pub fn new(rules: &[Rule]) -> AlarmRules {
        let mut readers: Vec<Reader> = Vec::new();
        let mut out = Vec::new();
        for r in rules {
            let Some(selector) = r.kind.alarm_selector() else {
                continue;
            };
            let reader = match readers.iter().position(|x| x.selector == selector) {
                Some(i) => i,
                None => {
                    readers.push(Reader {
                        selector: selector.to_string(),
                        feed: Err("not opened yet".into()),
                        lost_whole: 0,
                    });
                    readers.len() - 1
                }
            };
            out.push(AlarmRule {
                name: r.name.clone(),
                id: r.id.clone(),
                reader,
                said: BTreeMap::new(),
                observer: None,
            });
        }
        AlarmRules {
            readers,
            rules: out,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules' names.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.rules.iter().map(|r| r.name.as_str())
    }

    /// Opens every reader on `session` (in the deployment's namespace):
    /// subscribed, then seeded by S4's GET (core §4.2), the owners' instance
    /// tokens followed on `watch`, a session of its own (#830). Returns, per
    /// reader, the transitions its first reading made — each alarm found,
    /// raised or resolved as a late reader reads it (§2.6) — for
    /// [`Self::baseline`].
    pub async fn open(
        &mut self,
        session: &zenoh::Session,
        watch: Option<&zenoh::Session>,
        timeout: Duration,
        clocks_synced: bool,
    ) -> Vec<Vec<AlarmTransition>> {
        let mut out = Vec::new();
        for r in &mut self.readers {
            let mut spec = AlarmSpec::new(timeout);
            spec.clocks_synced = clocks_synced;
            // What fires is the owners' word; an archive's last-known is
            // never part of a verdict (core S6).
            spec.archives = false;
            let opened = match AlarmScope::parse(&r.selector) {
                Err(why) => Err(why),
                Ok(scope) => AlarmFeed::open(session, watch, scope, spec, true)
                    .await
                    .map_err(|e| zenkey_fleet::one_line(&e)),
            };
            match opened {
                Ok((feed, initial)) => {
                    r.feed = Ok(feed);
                    out.push(initial);
                }
                Err(why) => {
                    r.feed = Err(format!("the reader could not be opened: {why}"));
                    out.push(Vec::new());
                }
            }
        }
        out
    }

    /// The first look (see the module doc), per rule: the state file's
    /// memory (`remembered`, by rule name), each alarm the first reading
    /// found, the remembered keys it holds no sample of, the observer, and
    /// a baseline notification when there is something to say.
    pub fn baseline(
        &mut self,
        initial: &[Vec<AlarmTransition>],
        remembered: &BTreeMap<String, Vec<Remembered>>,
        at: &str,
    ) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        let Self { readers, rules } = self;
        for rule in rules.iter_mut() {
            let reader = &mut readers[rule.reader];
            let sight = reader.look();
            let book = reader.feed.as_ref().ok().map(AlarmFeed::book);
            let ts = initial
                .get(rule.reader)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mine = remembered
                .get(&rule.name)
                .map(Vec::as_slice)
                .unwrap_or_default();
            out.extend(rule.first_look(book, ts, mine, &sight, &reader.selector, at));
        }
        out
    }

    /// Some reader listens: the drain owes it its deliveries.
    pub fn listening(&self) -> bool {
        self.readers.iter().any(|r| r.feed.is_ok())
    }

    /// The next delivery of any reader, folded into its book, and which
    /// reader it was; `None` when its subscriptions closed. Cancel-safe, as
    /// [`AlarmFeed::next`] is: a drain that drops it loses nothing.
    pub async fn next(&mut self) -> (usize, Option<Vec<AlarmTransition>>) {
        let mut live: Vec<(usize, &mut AlarmFeed)> = self
            .readers
            .iter_mut()
            .enumerate()
            .filter_map(|(i, r)| r.feed.as_mut().ok().map(|f| (i, f)))
            .collect();
        if live.is_empty() {
            return std::future::pending().await;
        }
        let nexts = live
            .iter_mut()
            .map(|(i, f)| {
                let i = *i;
                Box::pin(async move { (i, f.next().await) })
            })
            .collect::<Vec<_>>();
        let ((i, got), _, _) = futures_util::future::select_all(nexts).await;
        (i, got)
    }

    /// What [`Self::next`] got, as notices: the keys it moved, the rules'
    /// sight after it. A reader whose subscriptions closed reads
    /// unobservable from then on.
    pub fn delivered(
        &mut self,
        (i, got): (usize, Option<Vec<AlarmTransition>>),
        at: &str,
    ) -> Vec<RuleNotice> {
        let ts = match got {
            Some(ts) => ts,
            None => {
                if let Some(r) = self.readers.get_mut(i) {
                    r.feed = Err("its subscriptions closed: no delivery will come".into());
                }
                Vec::new()
            }
        };
        self.absorb(i, &ts, false, at)
    }

    /// Each reader's refresh — presence, S4's GET — owned, to run beside
    /// the drain ([`refresh`]), folded after it ([`Self::folded`]).
    pub fn plans(&self) -> Vec<(usize, AlarmRefresh)> {
        self.readers
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.feed.as_ref().ok().map(|f| (i, f.refresh_plan())))
            .collect()
    }

    /// What the refreshes read, folded into their books, as notices.
    pub fn folded(&mut self, done: Vec<(usize, AlarmRefreshed)>, at: &str) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        for (i, r) in done {
            let ts = match self.readers.get_mut(i).map(|x| &mut x.feed) {
                Some(Ok(f)) => f.fold(r),
                _ => continue,
            };
            out.extend(self.absorb(i, &ts, true, at));
        }
        out
    }

    /// Every key's verdict now — what moved with no sample: stale, its
    /// owner gone or back — as notices, on the tick.
    pub fn sweep(&mut self, at: &str) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        for i in 0..self.readers.len() {
            let ts = match &mut self.readers[i].feed {
                Ok(f) => f.sweep(),
                Err(_) => Vec::new(),
            };
            out.extend(self.absorb(i, &ts, false, at));
        }
        out
    }

    /// The worst sight over the readers, for the `alarms` health check;
    /// `None` when no rule reads alarms.
    pub fn sight(&mut self) -> Option<Observer> {
        self.readers
            .iter_mut()
            .map(Reader::look)
            .max_by_key(Sight::rank)
            .map(|s| s.observer())
    }

    /// Deliveries the readers lost, and stalls, so far.
    pub fn lost(&self) -> u64 {
        self.readers
            .iter()
            .filter_map(|r| r.feed.as_ref().ok())
            .map(|f| f.book().lost())
            .sum()
    }

    /// One batch of reader `i`'s transitions, for every rule on it: each
    /// key's notice, the remembered keys its book does not hold, then the
    /// rule's sight. `read` when the batch folds a refresh, whose presence
    /// read is what an owner's absence stands on: then every key the book
    /// holds is judged again too.
    fn absorb(
        &mut self,
        i: usize,
        ts: &[AlarmTransition],
        read: bool,
        at: &str,
    ) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        let Self { readers, rules } = self;
        let Some(reader) = readers.get_mut(i) else {
            return out;
        };
        for rule in rules.iter_mut().filter(|r| r.reader == i) {
            if let Ok(feed) = &reader.feed {
                let book = feed.book();
                for t in ts {
                    out.extend(rule.judge(t, book, read, at));
                }
                if read {
                    out.extend(rule.recheck(book, at));
                }
                out.extend(rule.reconcile(book, read, at).0);
            }
            let sight = reader.look();
            let state = sight.state();
            if rule.observer != Some(state) {
                let prior = rule.observer.replace(state);
                out.push(rule.observer_notice(&sight, &reader.selector, prior, false, at));
            }
        }
        out
    }
}

/// Every refresh in `plans`, together, beside the drain.
pub async fn refresh(plans: Vec<(usize, AlarmRefresh)>) -> Vec<(usize, AlarmRefreshed)> {
    futures_util::future::join_all(
        plans
            .into_iter()
            .map(|(i, p)| async move { (i, p.run().await) }),
    )
    .await
}

/// What the first look read, for the baseline's words.
#[derive(Debug, Default)]
struct Counts {
    firing: usize,
    not_firing: usize,
    unobservable: usize,
    /// Each key whose answer is not yet established, with why.
    not_yet: Vec<String>,
    /// Each remembered key the first look did not decide.
    kept: Vec<String>,
    owners: Option<zenkey_model::alarms::AnyFiring>,
}

impl Counts {
    /// Each key the first reading made a transition of, by its last one:
    /// counted by its state when settled, else not yet — or kept, when the
    /// state file remembers it (`remembered`, before the first look judged
    /// anything).
    fn read(&mut self, ts: &[AlarmTransition], remembered: &BTreeMap<String, Said>) {
        let mut last: BTreeMap<&str, &AlarmTransition> = BTreeMap::new();
        for t in ts.iter().filter(|t| t.change != AlarmChange::Lost) {
            last.insert(t.key.as_str(), t);
        }
        for (key, t) in last {
            let why = format!("{key} ({}: {})", t.firing.reason, t.firing.says);
            match (settled(&t.firing), remembered.get(key)) {
                (false, None) => self.not_yet.push(why),
                (false, Some(s)) => self.kept.push(format!(
                    "{why}; remembered {}",
                    crate::render::state_word(s.state)
                )),
                (true, _) => match state_of(t.firing.answer) {
                    CondState::Firing => self.firing += 1,
                    CondState::Ok => self.not_firing += 1,
                    CondState::Unobservable => self.unobservable += 1,
                },
            }
        }
    }
}

impl AlarmRule {
    /// The first look (see the module doc): the state file's memory of this
    /// rule (`remembered`), each alarm the first reading found (`ts`), the
    /// remembered keys the book holds no sample of, the observer, and a
    /// baseline notification when there is something to say. `book` is
    /// `None` when the reader could not be opened.
    fn first_look(
        &mut self,
        book: Option<&AlarmBook>,
        ts: &[AlarmTransition],
        remembered: &[Remembered],
        sight: &Sight,
        selector: &str,
        at: &str,
    ) -> Vec<RuleNotice> {
        let mut out = Vec::new();
        let mut remembered_observer = None;
        for m in remembered {
            let Some(what) =
                m.id.strip_prefix(&self.id)
                    .and_then(|s| s.strip_prefix(':'))
            else {
                continue;
            };
            if what == OBSERVER {
                remembered_observer = Some(m.state);
                continue;
            }
            if AlarmRef::from_key(what).is_some() {
                self.said.insert(
                    what.to_string(),
                    Said {
                        state: m.state,
                        severity: Some(m.severity.clone()),
                        document: None,
                    },
                );
            }
        }
        let mut counts = Counts::default();
        if let Some(book) = book {
            counts.read(ts, &self.said);
            for t in ts {
                out.extend(self.judge(t, book, true, at));
            }
            let (decided, kept) = self.reconcile(book, true, at);
            out.extend(decided);
            counts.kept.extend(kept);
            counts.owners =
                Some(book.any_firing(Severity::Info, Instant::now(), SystemTime::now()));
        }
        let state = sight.state();
        self.observer = Some(state);
        match (state, remembered_observer) {
            (CondState::Unobservable, was) => {
                out.push(self.observer_notice(sight, selector, was, true, at));
            }
            (CondState::Ok, Some(was)) if was != CondState::Ok => {
                out.push(self.observer_notice(sight, selector, Some(was), false, at));
            }
            _ => {}
        }
        let no_owner = counts
            .owners
            .as_ref()
            .is_some_and(|o| o.established + o.unobservable + o.not_asked == 0);
        if !counts.not_yet.is_empty()
            || !counts.kept.is_empty()
            || state != CondState::Ok
            || no_owner
        {
            out.push(self.baseline_notice(&counts, sight, selector, at));
        }
        out
    }

    /// One transition of a key, as this rule says it: `None` when the
    /// change is the reader's (a loss), not established ([`NOT_YET`]), or
    /// moves neither the state nor, while firing, the severity. An owner's
    /// absence is taken only from a presence read (`read`): §2.14 step 1's
    /// "a complete read without its instance token", never a live token's
    /// withdrawal alone, which a router's restart makes of every owner at
    /// once.
    fn judge(
        &mut self,
        t: &AlarmTransition,
        book: &AlarmBook,
        read: bool,
        at: &str,
    ) -> Option<RuleNotice> {
        if t.change == AlarmChange::Lost
            || !settled(&t.firing)
            || (!read && t.firing.reason == ABSENT)
        {
            return None;
        }
        let state = state_of(t.firing.answer);
        let prior = self.said.get(&t.key).cloned();
        let document = t
            .document
            .clone()
            .or_else(|| book.seen(&t.key).and_then(|s| s.document))
            .or_else(|| prior.as_ref().and_then(|p| p.document.clone()));
        let severity = t
            .severity
            .map(|s| s.as_str().to_owned())
            .or_else(|| prior.as_ref().and_then(|p| p.severity.clone()));
        let moved = match &prior {
            // A first word of ok is not news, and enters no ledger.
            None => state != CondState::Ok,
            Some(p) => p.state != state || (state == CondState::Firing && p.severity != severity),
        };
        self.said.insert(
            t.key.clone(),
            Said {
                state,
                severity: severity.clone(),
                document: document.clone(),
            },
        );
        if !moved {
            return None;
        }
        Some(self.notice(t, state, prior.as_ref(), document, severity, at))
    }

    /// The remembered keys the book does not hold — a key its owner deleted
    /// while this reader did not hear it, or one the state file remembers
    /// from before a restart — judged as §2.14 step 5 judges *none*: no
    /// (`not_held`) once the owner's alarms are known whole, the owner's
    /// not-asked answer when it is absent or does not list `alarms.v1`, and
    /// kept at their last word while its alarms are unobservable. A key the
    /// selector no longer reaches is left as remembered; one said ok is
    /// forgotten. Returns the notices, and the keys kept, with why.
    fn reconcile(
        &mut self,
        book: &AlarmBook,
        read: bool,
        at: &str,
    ) -> (Vec<RuleNotice>, Vec<String>) {
        let held: BTreeSet<&str> = book.keys().collect();
        let absent: Vec<String> = self
            .said
            .keys()
            .filter(|k| !held.contains(k.as_str()))
            .cloned()
            .collect();
        let now = Instant::now();
        let mut out = Vec::new();
        let mut kept = Vec::new();
        for key in absent {
            let Some(said) = self.said.get(&key).cloned() else {
                continue;
            };
            if said.state == CondState::Ok {
                self.said.remove(&key);
                continue;
            }
            if !book.scope().selects_key(&key) {
                kept.push(format!("{key} (no longer under the rule's selector)"));
                continue;
            }
            let Some(alarm) = AlarmRef::from_key(&key) else {
                continue;
            };
            let Ok(owner) = format!("{}/{}", alarm.system, alarm.service).parse::<Addr>() else {
                continue;
            };
            let set = book.owner_set(&owner, now);
            let judged = match set.verdict {
                SetVerdict::Established => Judged {
                    answer: Answer::No,
                    reason: Reason::NotHeld,
                },
                SetVerdict::NotAsked => Judged {
                    answer: Answer::NotAsked,
                    reason: set.reason,
                },
                SetVerdict::Unobservable => {
                    kept.push(format!(
                        "{key} (not read; its owner's alarms are not known whole: {}; \
                         remembered {})",
                        set.reason.says(),
                        crate::render::state_word(said.state)
                    ));
                    continue;
                }
            };
            let t = AlarmTransition {
                key: key.clone(),
                alarm,
                change: AlarmChange::Judged,
                firing: answer_of(judged),
                severity: said.severity.as_deref().and_then(Severity::parse),
                document: None,
                stamp: None,
                via: None,
                lost: None,
            };
            out.extend(self.judge(&t, book, read, at));
        }
        (out, kept)
    }

    /// Every key the book holds, judged again at a refresh: what a
    /// transition deferred to a read (an owner's absence) is decided here,
    /// as a `judged` change.
    fn recheck(&mut self, book: &AlarmBook, at: &str) -> Vec<RuleNotice> {
        let (now, wall) = (Instant::now(), SystemTime::now());
        let keys: Vec<String> = book.keys().map(str::to_owned).collect();
        let mut out = Vec::new();
        for key in keys {
            let (Some(r), Some(alarm)) = (book.reading(&key, now, wall), AlarmRef::from_key(&key))
            else {
                continue;
            };
            let t = AlarmTransition {
                stamp: book.seen(&key).and_then(|s| s.stamp),
                key,
                alarm,
                change: AlarmChange::Judged,
                firing: answer_of(r.firing()),
                severity: r.severity,
                document: None,
                via: None,
                lost: None,
            };
            out.extend(self.judge(&t, book, true, at));
        }
        out
    }

    fn notice(
        &self,
        t: &AlarmTransition,
        state: CondState,
        prior: Option<&Said>,
        document: Option<Alarm>,
        severity: Option<String>,
        at: &str,
    ) -> RuleNotice {
        let a = &t.alarm;
        let mut lines = vec![format!(
            "alarm: system {} · service {} · alert key {} (alarms.v1 §2.12)",
            a.system, a.service, a.alert_key
        )];
        if let Some(d) = &document {
            lines.push(format!("alarm rule: {}", d.rule));
            if let Some(c) = &d.category {
                lines.push(format!("category: {c}"));
            }
            lines.push(format!(
                "{} {} (the document's timestamp: its latest transition, §2.4)",
                match d.state {
                    State::Firing => "firing since",
                    State::Resolved => "resolved at",
                },
                zenkey_fleet::rfc3339_from_unix(d.timestamp / 1000)
            ));
        }
        lines.push(format!(
            "change: {}{}",
            t.change.as_str(),
            match t.via {
                Some(zenkey_fleet::report::AlarmVia::Subscription) => ", heard live",
                Some(zenkey_fleet::report::AlarmVia::Get) => ", read by the owner's GET",
                None => "",
            }
        ));
        RuleNotice {
            rule: self.name.clone(),
            what: t.key.clone(),
            kind: NoticeKind::Alert,
            state,
            prior: prior.map(|p| p.state),
            evidence: evidence(t, state, prior, document.as_ref(), severity.as_deref()),
            severity,
            lines,
            key: Some(t.key.clone()),
            address: Some(format!("{}/{}", a.system, a.service)),
            at: at.to_string(),
            presence: None,
            assume: false,
            passthrough: false,
            reason: if t.lost.is_some() || t.change == AlarmChange::Judged {
                "the verdict moved with no sample"
            } else {
                "a delete carries no document"
            },
            alert: Some(Box::new(AlertFacts {
                transition: t.clone(),
                document,
            })),
        }
    }

    fn observer_notice(
        &self,
        sight: &Sight,
        selector: &str,
        prior: Option<CondState>,
        assume: bool,
        at: &str,
    ) -> RuleNotice {
        RuleNotice {
            rule: self.name.clone(),
            what: OBSERVER.to_string(),
            kind: NoticeKind::Unobservable,
            state: sight.state(),
            prior,
            severity: None,
            evidence: sight.evidence(selector),
            lines: Vec::new(),
            key: None,
            address: None,
            at: at.to_string(),
            presence: None,
            assume,
            passthrough: false,
            reason: "nothing was observed",
            alert: None,
        }
    }

    fn baseline_notice(&self, c: &Counts, sight: &Sight, selector: &str, at: &str) -> RuleNotice {
        let owners = match &c.owners {
            Some(o) => format!(
                "owners in scope: {} established, {} unobservable, {} not asked",
                o.established, o.unobservable, o.not_asked
            ),
            None => "no owner read".to_string(),
        };
        let mut evidence = format!(
            "alerts {selector}: the first look read {} alarm(s) — {} firing, {} not firing, {} \
             unobservable, {} not yet established; {owners} — {}",
            c.firing + c.not_firing + c.unobservable + c.not_yet.len(),
            c.firing,
            c.not_firing,
            c.unobservable,
            c.not_yet.len(),
            sight.evidence(selector)
        );
        if c.owners
            .as_ref()
            .is_some_and(|o| o.established + o.unobservable + o.not_asked == 0)
        {
            evidence.push_str(&format!(
                "; no owner of alarms.v1 under {selector} is known yet, so the first one to \
                 appear is the first one watched"
            ));
        }
        let mut lines = vec![
            "the first look: an alarm read firing is told on its own; one this reader cannot \
             judge yet (its clock not trusted, a loss not healed, its subscription not listened \
             900 s) is told when it settles, and one the state file remembers keeps its last \
             word until then"
                .to_string(),
        ];
        lines.extend(c.not_yet.iter().map(|k| format!("- not yet: {k}")));
        lines.extend(c.kept.iter().map(|k| format!("- kept: {k}")));
        let state = if c.not_yet.is_empty() && c.kept.is_empty() && sight.state() == CondState::Ok {
            CondState::Ok
        } else {
            CondState::Unobservable
        };
        RuleNotice {
            rule: self.name.clone(),
            what: BASELINE.to_string(),
            kind: NoticeKind::Baseline,
            state,
            prior: None,
            severity: Some("info".into()),
            evidence,
            lines,
            key: None,
            address: None,
            at: at.to_string(),
            presence: None,
            assume: false,
            passthrough: true,
            reason: "a baseline is a count, not a payload",
            alert: None,
        }
    }
}

/// What a key's notice says: the alarm, what happened to it, its summary,
/// and §5's first answer with its reason.
fn evidence(
    t: &AlarmTransition,
    state: CondState,
    prior: Option<&Said>,
    doc: Option<&Alarm>,
    severity: Option<&str>,
) -> String {
    let a = &t.alarm;
    let what = match state {
        CondState::Firing => {
            let sev = severity.unwrap_or("an unknown severity");
            let was = prior
                .filter(|p| p.state == CondState::Firing)
                .and_then(|p| p.severity.as_deref());
            match (was, t.change) {
                (Some(was), _) if was != sev => format!("escalated from {was} to {sev}"),
                (_, AlarmChange::Raised) => format!("raised at {sev}"),
                (_, AlarmChange::Escalated) => format!("escalated to {sev}"),
                _ => format!("firing at {sev}"),
            }
        }
        CondState::Ok => match t.firing.reason.as_str() {
            "resolved" => "resolved".to_string(),
            "deleted" => "deleted, which reads as resolved (§2.6)".to_string(),
            "not_held" => {
                "gone from its owner's alarms, known whole: resolved and deleted while this \
                 reader did not hear it"
                    .to_string()
            }
            other => format!("not firing ({other})"),
        },
        CondState::Unobservable => match t.firing.answer {
            AlarmAnswerToken::NotAsked => "not asked".to_string(),
            _ => "unobservable".to_string(),
        },
    };
    let summary = doc.map(|d| format!(" — {}", d.summary)).unwrap_or_default();
    format!(
        "alarm {} of {}/{} {what}{summary}: {} ({}, {}; alarms.v1 §2.14)",
        a.alert_key,
        a.system,
        a.service,
        t.firing.says,
        t.firing.answer.as_str(),
        t.firing.reason
    )
}

#[cfg(test)]
mod tests {
    //! The rule over a real book (the fleet's, session-free): §2.6's
    //! transitions as notices, a loss said once by the observer, an owner
    //! gone, and the first look.

    use super::*;
    use std::time::Duration;
    use zenkey_fleet::report::AlarmVia;
    use zenkey_fleet::{AlarmBookSpec, AlarmPresenceSeen, AlarmSample, AlarmSeedEnd};
    use zenkey_fleet::{DescriptorRead, ObservedPresence};
    use zenoh::time::{NTP64, Timestamp, TimestampId};

    const A: &str = "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da";
    const ZID: &str = "a1";

    fn stamp(at: SystemTime) -> Timestamp {
        Timestamp::new(
            NTP64::from(at.duration_since(SystemTime::UNIX_EPOCH).unwrap()),
            ZID.parse::<TimestampId>().unwrap(),
        )
    }

    fn doc(severity: &str, state: &str, timestamp: u64, summary: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "timestamp": timestamp, "rule": "link_down", "severity": severity,
            "state": state, "summary": summary,
            "labels": {"peer": "r2", "port": "eth0", "host": "h-3fa9c2d41b7e"},
        }))
        .unwrap()
    }

    /// One put of A, `n` ms after the epoch's stamp base, as the reader got
    /// it: stamped by the owner's clock now.
    fn put(payload: Vec<u8>, via: AlarmVia) -> AlarmSample {
        AlarmSample {
            key: A.into(),
            delete: false,
            payload,
            stamp: Some(stamp(SystemTime::now())),
            at: Instant::now(),
            wall: SystemTime::now(),
            via,
        }
    }

    fn delete() -> AlarmSample {
        AlarmSample {
            delete: true,
            payload: Vec::new(),
            ..put(Vec::new(), AlarmVia::Subscription)
        }
    }

    fn presence(held: bool) -> ObservedPresence {
        let keys = if held {
            vec!["zk2/lab/svc/@zk/instance/000000000000000a".to_owned()]
        } else {
            Vec::new()
        };
        let mut o = ObservedPresence::from_keys("zk2/lab/*/@zk/**", &keys, true);
        let d: zenkey_model::descriptor::Descriptor = serde_json::from_value(serde_json::json!({
            "format": "zk2-descriptor/0.1", "service": "lab/svc",
            "instance": "000000000000000a",
            "interfaces": [{"iface": "alarms.v1",
                "contract": format!("sha256:{}", zenkey::alarms::FINGERPRINT),
                "minor": 0, "token": false}],
            "meta": {"zid": ZID},
        }))
        .unwrap();
        o.descriptors = Some(BTreeMap::from([(
            (
                "lab/svc".parse().unwrap(),
                "000000000000000a".parse().unwrap(),
            ),
            DescriptorRead::Served(Box::new(d)),
        )]));
        o
    }

    /// A reader's book over `lab`: subscribed, its owner present and listing
    /// alarms.v1, seeded by a complete GET of nothing; clocks on the word
    /// when `synced`.
    fn book(synced: bool) -> AlarmBook {
        let mut spec = AlarmBookSpec::new(Duration::from_secs(1));
        spec.clocks_synced = synced;
        let mut b = AlarmBook::new(
            AlarmScope::parse("zk2/lab/*/alarms.v1/state/alarms/*").unwrap(),
            spec,
        );
        let t0 = Instant::now();
        b.subscribed(t0);
        b.presence_read(&presence(true), t0);
        b.seeding();
        b.seeded(
            AlarmSeedEnd {
                asked_at: t0,
                complete: true,
            },
            t0,
            SystemTime::now(),
        );
        b
    }

    /// The last refresh's reads, whole.
    fn whole() -> AlarmReadLog {
        AlarmReadLog {
            presence: Some(Ok(AlarmPresenceSeen {
                selector: "zk2/lab/*/@zk/**".into(),
                complete: true,
                cut_off_during: false,
                services: 1,
                listing: 1,
                unsettled: false,
            })),
            seed: Some(Ok(true)),
            ..AlarmReadLog::default()
        }
    }

    fn rule() -> AlarmRule {
        AlarmRule {
            name: "fleet".into(),
            id: "fleet".into(),
            reader: 0,
            said: BTreeMap::new(),
            observer: Some(CondState::Ok),
        }
    }

    /// One sample through the book, then the rule.
    fn feed(r: &mut AlarmRule, b: &mut AlarmBook, s: AlarmSample) -> Option<RuleNotice> {
        let t = b.sample(s)?;
        r.judge(&t, b, false, "t")
    }

    fn says(n: &RuleNotice) -> (CondState, Option<CondState>, Option<&str>) {
        (n.state, n.prior, n.severity.as_deref())
    }

    /// §2.6 as notices: a raise fires at its document's severity, a refresh
    /// and a confirmation say nothing, an escalation is news in its own
    /// words, a resolve resolves, and its delete says nothing more; a
    /// re-raise fires again.
    #[test]
    fn a_raise_fires_a_refresh_is_nothing_an_escalation_is_news_and_a_resolve_resolves() {
        let mut b = book(false);
        let mut r = rule();
        let sub = AlarmVia::Subscription;
        let n = feed(
            &mut r,
            &mut b,
            put(doc("warning", "firing", 1, "down"), sub),
        )
        .unwrap();
        assert_eq!(says(&n), (CondState::Firing, None, Some("warning")));
        assert_eq!(n.kind, NoticeKind::Alert);
        assert_eq!(n.what, A);
        assert_eq!(n.address.as_deref(), Some("lab/svc"));
        assert!(
            n.evidence.contains("raised at warning — down"),
            "{}",
            n.evidence
        );
        let facts = n.alert.as_ref().unwrap();
        assert_eq!(facts.transition.change, AlarmChange::Raised);
        assert_eq!(
            facts.labels(),
            BTreeMap::from([("peer".into(), "r2".into()), ("port".into(), "eth0".into())]),
            "host is the system's to say"
        );
        assert!(
            feed(
                &mut r,
                &mut b,
                put(doc("warning", "firing", 1, "still down"), sub)
            )
            .is_none(),
            "a refresh is no transition"
        );
        assert!(
            feed(
                &mut r,
                &mut b,
                put(doc("warning", "firing", 1, "still down"), sub)
            )
            .is_none(),
            "nor a confirmation"
        );
        let n = feed(
            &mut r,
            &mut b,
            put(doc("critical", "firing", 2, "down"), sub),
        )
        .unwrap();
        assert_eq!(
            says(&n),
            (CondState::Firing, Some(CondState::Firing), Some("critical"))
        );
        assert!(
            n.evidence.contains("escalated from warning to critical"),
            "{}",
            n.evidence
        );
        let n = feed(
            &mut r,
            &mut b,
            put(doc("critical", "resolved", 3, "up"), sub),
        )
        .unwrap();
        assert_eq!(
            says(&n),
            (CondState::Ok, Some(CondState::Firing), Some("critical"))
        );
        assert!(n.evidence.contains(" resolved — up"), "{}", n.evidence);
        assert!(feed(&mut r, &mut b, delete()).is_none(), "retired: no more");
        let n = feed(
            &mut r,
            &mut b,
            put(doc("warning", "firing", 4, "down"), sub),
        )
        .unwrap();
        assert_eq!(
            says(&n),
            (CondState::Firing, Some(CondState::Ok), Some("warning"))
        );
    }

    /// A loss is the observer's, once: the keys keep their last word —
    /// even a resolve heard during it moves nothing — and the GET that
    /// heals it decides each key and restores the sight.
    #[test]
    fn a_loss_is_said_once_and_the_get_after_it_decides() {
        let mut b = book(true);
        let mut r = rule();
        let sub = AlarmVia::Subscription;
        feed(
            &mut r,
            &mut b,
            put(doc("warning", "firing", 1, "down"), sub),
        )
        .unwrap();
        assert_eq!(sight(&b, &whole(), 0), Sight::Complete);
        let lost = b.lose(
            zenkey_fleet::report::AlarmLoss {
                dropped: 4,
                stalled: false,
            },
            Instant::now(),
            SystemTime::now(),
        );
        assert_eq!(lost.len(), 1);
        assert!(
            r.judge(&lost[0], &b, false, "t").is_none(),
            "the key keeps its word"
        );
        assert_eq!(sight(&b, &whole(), 0), Sight::Lost { lost: 4 });
        let resolved = put(doc("warning", "resolved", 2, "up"), sub);
        assert!(
            feed(&mut r, &mut b, resolved.clone()).is_none(),
            "a resolve after a loss is not established (§2.14 step 4)"
        );
        // The GET after it: the owner answers its delete.
        let asked = Instant::now();
        b.seeding();
        let mut reply = delete();
        reply.via = AlarmVia::Get;
        reply.at = asked;
        let mut out: Vec<RuleNotice> = b
            .sample(reply)
            .into_iter()
            .filter_map(|t| r.judge(&t, &b, true, "t"))
            .collect();
        let judged = b.seeded(
            AlarmSeedEnd {
                asked_at: asked,
                complete: true,
            },
            Instant::now(),
            SystemTime::now(),
        );
        out.extend(judged.iter().filter_map(|t| r.judge(t, &b, true, "t")));
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(
            says(&out[0]),
            (CondState::Ok, Some(CondState::Firing), Some("warning"))
        );
        assert_eq!(sight(&b, &whole(), 0), Sight::Complete);
    }

    /// An owner gone: its alarm is last-known, unobservable — never
    /// resolved, never ok. A live token's withdrawal alone moves nothing
    /// (a router's restart withdraws every token at once); the presence
    /// read after it decides, at the refresh.
    #[test]
    fn an_owner_gone_leaves_its_alarm_unobservable_never_resolved() {
        let mut b = book(true);
        let mut r = rule();
        feed(
            &mut r,
            &mut b,
            put(doc("warning", "firing", 1, "down"), AlarmVia::Subscription),
        )
        .unwrap();
        b.instance(
            &"lab/svc".parse().unwrap(),
            &"000000000000000a".parse().unwrap(),
            false,
            Instant::now(),
        );
        let moved = b.sweep(Instant::now(), SystemTime::now());
        assert_eq!(moved.len(), 1, "the book reads it absent");
        assert!(
            moved.iter().all(|t| r.judge(t, &b, false, "t").is_none()),
            "a token's withdrawal alone is no read"
        );
        b.presence_read(&presence(false), Instant::now());
        let out = r.recheck(&b, "t");
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(
            says(&out[0]),
            (
                CondState::Unobservable,
                Some(CondState::Firing),
                Some("warning")
            )
        );
        assert!(
            out[0].evidence.contains("not asked") && out[0].evidence.contains("last-known"),
            "{}",
            out[0].evidence
        );
        assert!(r.recheck(&b, "t").is_empty(), "said once");
    }

    /// The first look: an alarm read firing is told; one read only by a
    /// GET whose stamp no trusted clock can age is not yet, said in the
    /// baseline and told when a live confirmation settles it; a key the
    /// state file remembers firing that its owner, known whole, no longer
    /// holds is resolved; one remembered firing and read firing is no news.
    #[test]
    fn the_first_look_tells_what_fires_and_says_what_it_cannot_judge() {
        // Not synced: the GET's reply is not yet established.
        let mut b = book(false);
        b.seeding();
        let ts: Vec<AlarmTransition> = b
            .sample(put(doc("warning", "firing", 1, "down"), AlarmVia::Get))
            .into_iter()
            .collect();
        let mut r = rule();
        let out = r.first_look(Some(&b), &ts, &[], &Sight::Complete, "zk2/lab/**", "t");
        assert_eq!(out.len(), 1, "the baseline alone: {out:?}");
        assert_eq!(out[0].what, BASELINE);
        assert_eq!(out[0].state, CondState::Unobservable);
        assert_eq!(out[0].severity.as_deref(), Some("info"));
        assert!(out[0].passthrough);
        assert!(
            out[0]
                .lines
                .iter()
                .any(|l| l.starts_with(&format!("- not yet: {A} ("))),
            "{:?}",
            out[0].lines
        );
        let n = feed(
            &mut r,
            &mut b,
            put(doc("warning", "firing", 1, "down"), AlarmVia::Subscription),
        )
        .unwrap();
        assert_eq!(says(&n), (CondState::Firing, None, Some("warning")));

        // Synced: the firing alarm is told at once, and nothing else is said.
        // Its owner answered a complete GET: its alarms are known whole.
        let mut b = book(true);
        b.seeding();
        let mut ts: Vec<AlarmTransition> = b
            .sample(put(doc("critical", "firing", 1, "down"), AlarmVia::Get))
            .into_iter()
            .collect();
        ts.extend(b.seeded(
            AlarmSeedEnd {
                asked_at: Instant::now(),
                complete: true,
            },
            Instant::now(),
            SystemTime::now(),
        ));
        let mut r = rule();
        let out = r.first_look(Some(&b), &ts, &[], &Sight::Complete, "zk2/lab/**", "t");
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(says(&out[0]), (CondState::Firing, None, Some("critical")));

        // Remembered firing and read firing: no news. Remembered firing and
        // gone from the owner's answer: resolved.
        let gone = "zk2/lab/svc/alarms.v1/state/alarms/0000000000000001";
        let remembered = [
            Remembered {
                id: format!("fleet:{A}"),
                state: CondState::Firing,
                severity: "critical".into(),
            },
            Remembered {
                id: format!("fleet:{gone}"),
                state: CondState::Firing,
                severity: "warning".into(),
            },
            Remembered {
                id: "fleet:observer".into(),
                state: CondState::Unobservable,
                severity: "warning".into(),
            },
        ];
        let mut r = rule();
        r.observer = None;
        let out = r.first_look(
            Some(&b),
            &ts,
            &remembered,
            &Sight::Complete,
            "zk2/lab/**",
            "t",
        );
        let by: Vec<(&str, CondState, Option<CondState>)> = out
            .iter()
            .map(|n| (n.what.as_str(), n.state, n.prior))
            .collect();
        assert_eq!(
            by,
            [
                (gone, CondState::Ok, Some(CondState::Firing)),
                (OBSERVER, CondState::Ok, Some(CondState::Unobservable)),
            ],
            "{out:?}"
        );
        assert!(out[0].evidence.contains("not_held"), "{}", out[0].evidence);
    }

    fn answer(answer: AlarmAnswerToken, reason: &str) -> AlarmAnswer {
        AlarmAnswer {
            answer,
            reason: reason.into(),
            says: "s".into(),
        }
    }

    /// The three states, never two: yes firing, no ok, every other answer
    /// unobservable — not asked included; and only the reader's own sight
    /// is not yet the key's.
    #[test]
    fn an_answer_is_one_of_three_states_and_the_readers_sight_is_not_the_keys() {
        use AlarmAnswerToken::*;
        assert_eq!(state_of(Yes), CondState::Firing);
        assert_eq!(state_of(No), CondState::Ok);
        assert_eq!(state_of(Unobservable), CondState::Unobservable);
        assert_eq!(state_of(NotAsked), CondState::Unobservable, "never ok");
        // alarms.v1 §2.16's closed list of the reader's own sight (text 0.7,
        // F-140; 0.8, F-141), spelled out here so that dropping one from
        // `NOT_YET` fails.
        for r in [
            "incomplete",
            "presence_incomplete",
            "no_descriptor",
            "not_read",
            "nothing_crossed",
            "silent",
            "clock_untrusted",
            "listened_too_short",
            "near_horizon",
            "no_observation",
        ] {
            assert!(!settled(&answer(Unobservable, r)), "{r}");
        }
        assert_eq!(NOT_YET.len(), 10, "the text's ten, and no more");
        for (a, r) in [
            (Yes, "firing"),
            (No, "resolved"),
            (No, "deleted"),
            (No, "not_held"),
            (Unobservable, "stale"),
            (Unobservable, "undecodable"),
            (Unobservable, "face_closed"),
            (Unobservable, "no_stamp"),
            (Unobservable, "clock_disagrees"),
            (Unobservable, "not_a_horizon"),
            (NotAsked, "absent"),
            (NotAsked, "previous_instance"),
            (NotAsked, "not_listed"),
        ] {
            assert!(settled(&answer(a, r)), "{r}");
        }
    }

    /// The sight's words and its health check, and the worst wins.
    #[test]
    fn the_sight_says_why_and_ranks() {
        let lost = Sight::Lost { lost: 3 };
        assert_eq!(lost.state(), CondState::Unobservable);
        assert_eq!(lost.observer(), Observer::Lost { dropped: 3 });
        let e = lost.evidence("zk2/lab/**");
        assert!(e.contains("lost 3") && e.contains("§2.14 step 4"), "{e}");
        assert_eq!(Sight::Complete.state(), CondState::Ok);
        assert!(
            Sight::Incomplete("the GET ended".into())
                .evidence("zk2/**")
                .contains("never ok")
        );
        let worst = [Sight::Complete, lost.clone(), Sight::Unseeded]
            .into_iter()
            .max_by_key(Sight::rank);
        assert_eq!(worst, Some(lost));
    }
}
