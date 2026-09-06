//! The notification discipline (#389): everything between [`route`] and a
//! delivery that is the difference between a notifier someone keeps
//! enabled and one they mute in week two.
//!
//! [`route`]: crate::engine::route
//!
//! One notice at a time goes in ([`Discipline::observe`]); once per tick
//! ([`Discipline::tick`]) what is *due* comes out as the notifications to
//! deliver. Between the two, per notice identity:
//!
//! - **`for`** — a firing notice is announced only if it is still firing
//!   `for_s` after it started; a return to `ok` before then cancels
//!   silently (nothing was announced, so nothing resolves); `unobservable`
//!   mid-wait **pauses** the timer rather than cancelling it, and is itself
//!   announced only if it persists past `for` (silence is not evidence,
//!   RFC 13 §3 O6).
//! - **dedup** — `(id, state, severity)` unchanged is suppressed; a
//!   severity change is announced.
//! - **repeat** — still firing `repeat_s` after the last delivery is sent
//!   again, once per interval, with `repeat` counting up.
//! - **grouping** — notices sharing a group key (a configured label set;
//!   by origin unless said otherwise) within one window are one
//!   [`NoticeKind::Group`]; a group of one goes plain.
//! - **the resolved family** — `firing → ok` is `resolved`, `unobservable
//!   → ok` is `observable_again`, `firing → unobservable` is `lost_sight`,
//!   `ok → unobservable` is `unobservable`; each is one notification, and
//!   `resolved_notice: false` drops the first two.
//! - **inhibition** — on the tick, the engine's impact attribution (RFC 06
//!   §5.6) runs over the catalog's edges, the entities of everything
//!   firing, and the entities this daemon judged down. A notice whose
//!   entity is a *symptom* carries `inhibited_by: <root>`; it is delivered
//!   inside the root's own group or when its severity is `error` or
//!   worse, and otherwise held — counted and logged, never dropped
//!   silently.
//!
//! **Time is an argument.** Every entry point takes `now` as Unix seconds,
//! so the whole table above is pinned by unit tests with an injected clock
//! and no `sleep`; the engine passes the wall clock. The same seconds are
//! what the state file stores ([`state`]), which is what lets a restart
//! pick the timers up where they were.
//!
//! **Bounded.** At most `max_entries` identities are remembered; the least
//! recently changed go first, and the count rides the health document and
//! the state file.

pub mod inhibit;
pub mod state;

use std::collections::{BTreeMap, BTreeSet};

use zenkey_fleet::{CondState, ImpactInputs, ImpactReport, RenderSource, attribute};

use crate::config::DisciplineConfig;
use crate::render::{RenderConfig, state_word, truncate_bytes};
use crate::rules::Rule;
use crate::sinks::{NoticeKind, Notification, Outgoing};

use self::inhibit::Catalog;
use self::state::{PersistedEntry, PersistedState};

/// What the engine knows about a notice beyond its rendering.
#[derive(Debug, Clone, Default)]
pub struct NoticeMeta {
    /// The origin chunk of the key the notice came from, if it had one.
    pub origin: Option<String>,
    /// An event with no identity to remember — the observer's own
    /// `Dropped(n)` — delivered every time, never deduplicated.
    pub passthrough: bool,
}

/// A firing notice waiting out its `for` window.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pending {
    state: CondState,
    /// When the firing began, shifted forward by any pause.
    started: f64,
    /// Set while the wait is paused by `unobservable`.
    unobservable_since: Option<f64>,
}

/// One remembered notice identity.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub rule: String,
    pub rule_id: String,
    /// The announced state; `Ok` when nothing is announced.
    pub state: CondState,
    pub severity: String,
    /// Unix seconds when the announced condition began.
    pub since: f64,
    pub last_sent: Option<f64>,
    pub repeat: u32,
    pub labels: BTreeMap<String, String>,
    pub origin: Option<String>,
    /// Something was announced (or held as a symptom) for the current
    /// condition — the flag a resolve needs.
    pub announced: bool,
    /// The root that explains this notice, when attribution said so.
    pub inhibited_by: Option<String>,
    /// The announcement was held as a low-severity symptom: nothing went
    /// out, so its resolve stays in too.
    held: bool,
    pending: Option<Pending>,
    /// The last rendering, to send at the flush or on a repeat.
    latest: Option<Box<Outgoing>>,
    /// For the eviction order.
    changed: f64,
}

/// A notification waiting for the flush.
#[derive(Debug, Clone)]
struct Queued {
    id: String,
    outgoing: Outgoing,
    origin: Option<String>,
    queued_at: f64,
    passthrough: bool,
}

/// What the discipline did, counted (RFC 13 §3 O6: every bound reports
/// its cost, and every suppression is a number somewhere).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Counters {
    /// Notifications handed out for delivery (a group counts once).
    pub sent: u64,
    /// Same identity, state and severity again.
    pub deduped: u64,
    /// A `for` wait that returned to `ok` before it was announced.
    pub cancelled: u64,
    /// A first-observation `ok` — a baseline, not news.
    pub baseline: u64,
    /// Notices folded into a group (members, not groups).
    pub grouped: u64,
    pub repeats: u64,
    /// Symptoms held back, plus their resolves.
    pub inhibited: u64,
    /// Identities evicted at the bound, over the ledger's life.
    pub evicted: u64,
}

/// One tick's output.
#[derive(Debug, Default)]
pub struct Flush {
    pub outgoing: Vec<Outgoing>,
    /// The attribution that ran, when inhibition had a graph to run on.
    pub impact: Option<ImpactReport>,
}

/// Per-rule facts the discipline needs that a notification does not carry.
#[derive(Debug, Clone)]
struct RuleFacts {
    id: String,
    head: &'static str,
    for_s: f64,
    sinks: Vec<String>,
}

pub struct Discipline {
    cfg: DisciplineConfig,
    rules: BTreeMap<String, RuleFacts>,
    max_message_bytes: usize,
    entries: BTreeMap<String, Entry>,
    queue: Vec<Queued>,
    counters: Counters,
    max_entries: usize,
    dirty: bool,
    groups_made: u64,
}

/// Severity rank: the config's closed vocabulary, `warning` for anything a
/// producer spelled outside it.
pub fn severity_rank(s: &str) -> u8 {
    match s {
        "info" => 0,
        "warning" => 1,
        "error" => 2,
        "critical" => 3,
        _ => 1,
    }
}

fn state_rank(s: CondState) -> u8 {
    match s {
        CondState::Ok => 0,
        CondState::Unobservable => 1,
        CondState::Firing => 2,
    }
}

fn rfc3339(now: f64) -> String {
    zenkey_fleet::rfc3339_from_unix(now.max(0.0) as u64)
}

impl Discipline {
    pub fn new(
        cfg: &DisciplineConfig,
        rules: &[Rule],
        render: &RenderConfig,
        max_entries: usize,
    ) -> Discipline {
        Discipline {
            cfg: cfg.clone(),
            rules: rules
                .iter()
                .map(|r| {
                    (
                        r.name.clone(),
                        RuleFacts {
                            id: r.id.clone(),
                            head: r.kind.head(),
                            for_s: r.for_s.or(cfg.for_s).unwrap_or(0.0).max(0.0),
                            sinks: r.sinks.clone(),
                        },
                    )
                })
                .collect(),
            max_message_bytes: render.max_message_bytes,
            entries: BTreeMap::new(),
            queue: Vec::new(),
            counters: Counters::default(),
            max_entries: max_entries.max(1),
            dirty: false,
            groups_made: 0,
        }
    }

    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// The entries something was announced for, in id order.
    pub fn announced(&self) -> impl Iterator<Item = &Entry> {
        self.entries
            .values()
            .filter(|e| e.announced && e.state != CondState::Ok)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the ledger changed since the last [`Discipline::snapshot`].
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// The ledger as the state file stores it — announced, non-`ok`
    /// entries only — and clears the dirty flag.
    pub fn snapshot(&mut self, now: f64) -> PersistedState {
        self.dirty = false;
        PersistedState {
            version: state::STATE_VERSION,
            saved_at: rfc3339(now),
            entries: self
                .announced()
                .map(|e| PersistedEntry {
                    id: e.id.clone(),
                    rule: e.rule.clone(),
                    rule_id: e.rule_id.clone(),
                    state: e.state,
                    severity: e.severity.clone(),
                    since: e.since,
                    last_sent: e.last_sent,
                    repeat: e.repeat,
                    labels: e.labels.clone(),
                    origin: e.origin.clone(),
                    inhibited_by: e.inhibited_by.clone(),
                })
                .collect(),
            evicted_total: self.counters.evicted,
        }
    }

    /// Load a saved ledger: every entry comes back announced, so a re-put
    /// of an alert already announced before the restart is a duplicate,
    /// not news. Entries for rules no longer configured are dropped.
    pub fn restore(&mut self, saved: PersistedState) -> usize {
        self.counters.evicted = saved.evicted_total;
        let mut restored = 0;
        for p in saved.entries {
            let Some(facts) = self.rules.get(&p.rule) else {
                continue;
            };
            let rule_id = if p.rule_id.is_empty() {
                facts.id.clone()
            } else {
                p.rule_id
            };
            let held = p.inhibited_by.is_some();
            self.entries.insert(
                p.id.clone(),
                Entry {
                    id: p.id,
                    rule: p.rule,
                    rule_id,
                    state: p.state,
                    severity: p.severity,
                    since: p.since,
                    last_sent: p.last_sent,
                    repeat: p.repeat,
                    labels: p.labels,
                    origin: p.origin,
                    announced: true,
                    inhibited_by: p.inhibited_by,
                    held,
                    pending: None,
                    latest: None,
                    changed: p.since,
                },
            );
            restored += 1;
        }
        self.evict();
        restored
    }

    /// One routed notification in.
    pub fn observe(&mut self, o: Outgoing, meta: &NoticeMeta, now: f64) {
        if meta.passthrough {
            self.queue.push(Queued {
                id: o.notification.id.clone(),
                outgoing: o,
                origin: meta.origin.clone(),
                queued_at: now,
                passthrough: true,
            });
            return;
        }
        let n = &o.notification;
        let Some(facts) = self.rules.get(&n.rule).cloned() else {
            return;
        };
        let id = n.id.clone();
        let (state, severity) = (n.state, n.severity.clone());
        let e = self.entries.entry(id.clone()).or_insert_with(|| Entry {
            id: id.clone(),
            rule: n.rule.clone(),
            rule_id: facts.id.clone(),
            state: CondState::Ok,
            severity: severity.clone(),
            since: now,
            last_sent: None,
            repeat: 0,
            labels: n.labels.clone(),
            origin: meta.origin.clone(),
            announced: false,
            inhibited_by: None,
            held: false,
            pending: None,
            latest: None,
            changed: now,
        });
        e.labels = n.labels.clone();
        e.origin = meta.origin.clone();
        e.latest = Some(Box::new(o.clone()));
        e.changed = now;
        self.dirty = true;

        let mut queue: Option<(NoticeKind, CondState)> = None;
        match state {
            CondState::Firing => {
                if e.announced && e.state == CondState::Firing {
                    if e.severity == severity {
                        self.counters.deduped += 1;
                    } else {
                        // A severity change is news; the timer is not reset.
                        e.severity = severity;
                        queue = Some((base_kind(facts.head), CondState::Firing));
                    }
                } else if let Some(p) = &mut e.pending {
                    match p.state {
                        CondState::Firing => self.counters.deduped += 1,
                        _ => {
                            // Back from the pause: credit what had elapsed.
                            let elapsed = p.unobservable_since.unwrap_or(now) - p.started;
                            p.started = now - elapsed.max(0.0);
                            p.unobservable_since = None;
                            p.state = CondState::Firing;
                        }
                    }
                } else if facts.for_s > 0.0 {
                    e.pending = Some(Pending {
                        state: CondState::Firing,
                        started: now,
                        unobservable_since: None,
                    });
                    e.severity = severity;
                    e.since = now;
                } else {
                    e.severity = severity;
                    queue = Some((base_kind(facts.head), CondState::Firing));
                }
            }
            CondState::Ok => {
                if e.pending.take().is_some() && !e.announced {
                    self.counters.cancelled += 1;
                } else if e.announced {
                    let kind = match e.state {
                        CondState::Firing => Some(NoticeKind::Resolved),
                        CondState::Unobservable => Some(NoticeKind::ObservableAgain),
                        CondState::Ok => None,
                    };
                    match kind {
                        Some(_) if e.held => {
                            self.counters.inhibited += 1;
                            tracing::info!(id = %e.id, root = ?e.inhibited_by,
                                "resolve held with its symptom");
                        }
                        Some(k) if self.cfg.resolved_notice => {
                            queue = Some((k, CondState::Ok));
                        }
                        Some(_) => {}
                        None => self.counters.deduped += 1,
                    }
                    e.announced = false;
                    e.state = CondState::Ok;
                    e.repeat = 0;
                    e.inhibited_by = None;
                    e.held = false;
                    e.last_sent = None;
                } else {
                    self.counters.baseline += 1;
                }
            }
            CondState::Unobservable => {
                if let Some(p) = &mut e.pending {
                    if p.state == CondState::Firing {
                        p.state = CondState::Unobservable;
                        p.unobservable_since = Some(now);
                    } else {
                        self.counters.deduped += 1;
                    }
                } else if e.announced {
                    match e.state {
                        CondState::Firing => {
                            queue = Some((NoticeKind::LostSight, CondState::Unobservable));
                        }
                        _ => self.counters.deduped += 1,
                    }
                } else if facts.for_s > 0.0 {
                    e.pending = Some(Pending {
                        state: CondState::Unobservable,
                        started: now,
                        unobservable_since: Some(now),
                    });
                    e.since = now;
                } else {
                    queue = Some((NoticeKind::Unobservable, CondState::Unobservable));
                }
            }
        }
        if let Some((kind, new_state)) = queue {
            self.announce(&id, kind, new_state, now);
        }
    }

    /// Mark an entry announced in `new_state` and queue its notification.
    fn announce(&mut self, id: &str, kind: NoticeKind, new_state: CondState, now: f64) {
        let Some(e) = self.entries.get_mut(id) else {
            return;
        };
        if e.state != new_state || !e.announced {
            e.since = now;
            e.repeat = 0;
        }
        e.state = new_state;
        e.announced = new_state != CondState::Ok;
        e.pending = None;
        e.changed = now;
        self.dirty = true;
        let Some(latest) = e.latest.as_deref() else {
            return;
        };
        let mut o = latest.clone();
        o.notification.kind = kind;
        o.notification.state = new_state;
        o.notification.repeat = e.repeat;
        self.queue.push(Queued {
            id: id.to_string(),
            outgoing: o,
            origin: e.origin.clone(),
            queued_at: now,
            passthrough: false,
        });
    }

    /// A repeat for a still-firing entry — from its last rendering, or
    /// from the entry itself after a restart left no rendering behind.
    fn repeat_of(&self, e: &Entry, now: f64) -> Outgoing {
        let facts = self.rules.get(&e.rule);
        let head = facts.map(|f| f.head).unwrap_or("rule");
        let mut o = e.latest.as_deref().cloned().unwrap_or_else(|| Outgoing {
            notification: Notification {
                id: e.id.clone(),
                rule: e.rule.clone(),
                rule_kind: head.to_string(),
                kind: base_kind(head),
                state: e.state,
                prior: None,
                severity: e.severity.clone(),
                title: format!("{} {} {}", e.severity, e.rule, state_word(e.state)),
                message: format!(
                    "state: {} (since {}, restored from the state file)",
                    state_word(e.state),
                    rfc3339(e.since)
                ),
                labels: e.labels.clone(),
                at: rfc3339(now),
                evidence: format!("still {} since {}", state_word(e.state), rfc3339(e.since)),
                rendering: RenderSource::KeyOnly,
                truncated: false,
                repeat: 0,
                group: None,
                inhibited_by: None,
            },
            sinks: facts.map(|f| f.sinks.clone()).unwrap_or_default(),
        });
        o.notification.kind = base_kind(head);
        o.notification.state = e.state;
        o.notification.at = rfc3339(now);
        o.notification.repeat = e.repeat;
        o.notification.title = format!(
            "{} {} {} (repeat {})",
            e.severity,
            e.rule,
            state_word(e.state),
            e.repeat
        );
        o
    }

    /// The tick: advance the `for` timers, issue repeats, flush the groups
    /// that are due (all of them with `force`, at shutdown), and run
    /// inhibition over what is about to go out.
    pub fn tick(
        &mut self,
        now: f64,
        catalog: Option<&Catalog>,
        gone_origins: &BTreeSet<String>,
        force: bool,
    ) -> Flush {
        // 1. Timers.
        let due_timers: Vec<(String, NoticeKind, CondState)> = self
            .entries
            .iter()
            .filter_map(|(id, e)| {
                let p = e.pending?;
                let for_s = self.rules.get(&e.rule)?.for_s;
                let head = self.rules.get(&e.rule)?.head;
                match p.state {
                    CondState::Firing if now >= p.started + for_s => {
                        Some((id.clone(), base_kind(head), CondState::Firing))
                    }
                    CondState::Unobservable
                        if now >= p.unobservable_since.unwrap_or(p.started) + for_s =>
                    {
                        Some((
                            id.clone(),
                            NoticeKind::Unobservable,
                            CondState::Unobservable,
                        ))
                    }
                    _ => None,
                }
            })
            .collect();
        for (id, kind, state) in due_timers {
            self.announce(&id, kind, state, now);
        }

        // 2. Repeats.
        if self.cfg.repeat_s > 0.0 {
            let due: Vec<String> = self
                .entries
                .values()
                .filter(|e| {
                    e.announced
                        && e.state == CondState::Firing
                        && !e.held
                        && e.last_sent
                            .is_some_and(|sent| now >= sent + self.cfg.repeat_s)
                })
                .map(|e| e.id.clone())
                .collect();
            for id in due {
                let snapshot = {
                    let e = self.entries.get_mut(&id).expect("listed");
                    e.repeat += 1;
                    e.changed = now;
                    e.clone()
                };
                let (o, origin) = (self.repeat_of(&snapshot, now), snapshot.origin.clone());
                self.counters.repeats += 1;
                self.dirty = true;
                self.queue.push(Queued {
                    id,
                    outgoing: o,
                    origin,
                    queued_at: now,
                    passthrough: false,
                });
            }
        }

        // 3. Which groups are due: a window opens with its first member.
        let mut groups: BTreeMap<String, (BTreeMap<String, String>, Vec<Queued>)> = BTreeMap::new();
        for (i, q) in std::mem::take(&mut self.queue).into_iter().enumerate() {
            let (mut key, labels) = self.group_key(&q);
            if q.passthrough {
                // No identity, no company: every one of these is its own.
                key = format!("{key}#{i}");
            }
            groups
                .entry(key)
                .or_insert_with(|| (labels, Vec::new()))
                .1
                .push(q);
        }
        let mut due: BTreeMap<String, (BTreeMap<String, String>, Vec<Queued>)> = BTreeMap::new();
        for (key, (labels, members)) in groups {
            let opened = members
                .iter()
                .map(|q| q.queued_at)
                .fold(f64::INFINITY, f64::min);
            if force || now >= opened + self.cfg.group_window_s {
                due.insert(key, (labels, members));
            } else {
                self.queue.extend(members);
            }
        }

        // 4. Inhibition over what is due.
        let mut flush = Flush::default();
        if let Some(cat) = catalog
            && self.cfg.inhibit.enabled
            && !cat.is_empty()
        {
            self.inhibit(cat, gone_origins, &mut due, &mut flush);
        }

        // 5. Emit.
        for (key, (labels, mut members)) in due {
            // The root of a folded group first, then by arrival, then id.
            members.sort_by(|a, b| {
                a.outgoing
                    .notification
                    .inhibited_by
                    .is_some()
                    .cmp(&b.outgoing.notification.inhibited_by.is_some())
                    .then_with(|| {
                        a.queued_at
                            .partial_cmp(&b.queued_at)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .then_with(|| a.id.cmp(&b.id))
            });
            let ids: Vec<String> = members.iter().map(|q| q.id.clone()).collect();
            let out = if members.len() == 1 {
                members.pop().expect("one").outgoing
            } else {
                self.counters.grouped += members.len() as u64;
                self.group_outgoing(&key, labels, &members, now)
            };
            for id in &ids {
                if let Some(e) = self.entries.get_mut(id) {
                    e.last_sent = Some(now);
                    self.dirty = true;
                }
            }
            self.counters.sent += 1;
            flush.outgoing.push(out);
        }

        self.evict();
        flush
    }

    /// Attribute, then decide per due firing notice: fold into the root's
    /// group, deliver alone as `error`+, or hold (counted).
    fn inhibit(
        &mut self,
        cat: &Catalog,
        gone_origins: &BTreeSet<String>,
        due: &mut BTreeMap<String, (BTreeMap<String, String>, Vec<Queued>)>,
        flush: &mut Flush,
    ) {
        let entity_of = |origin: Option<&str>| origin.and_then(|o| cat.entity_of(o));
        let mut firing: BTreeSet<String> = self
            .entries
            .values()
            .filter(|e| {
                (e.announced && e.state == CondState::Firing)
                    || e.pending.is_some_and(|p| p.state == CondState::Firing)
            })
            .filter_map(|e| entity_of(e.origin.as_deref()))
            .collect();
        for (_, members) in due.values() {
            for q in members {
                if q.outgoing.notification.state == CondState::Firing
                    && let Some(ent) = entity_of(q.origin.as_deref())
                {
                    firing.insert(ent);
                }
            }
        }
        let down = cat.down_entities(gone_origins);
        let edges = cat.edges();
        let report = attribute(
            &ImpactInputs {
                edges: &edges,
                firing: &firing,
                down: &down,
            },
            self.cfg.inhibit.depth,
        );
        let symptoms: BTreeMap<&str, &str> = report
            .symptoms
            .iter()
            .map(|s| (s.entity.as_str(), s.explained_by.as_str()))
            .collect();
        if !symptoms.is_empty() {
            // Which roots have a notice in this flush, and under which key.
            let mut root_keys: BTreeMap<String, String> = BTreeMap::new();
            for (key, (_, members)) in due.iter() {
                for q in members {
                    if let Some(ent) = entity_of(q.origin.as_deref())
                        && report.roots.iter().any(|r| r.entity == ent)
                    {
                        root_keys.entry(ent).or_insert_with(|| key.clone());
                    }
                }
            }
            let mut moved: Vec<(String, Queued)> = Vec::new();
            for (_, members) in due.values_mut() {
                let mut keep = Vec::new();
                for mut q in members.drain(..) {
                    let is_firing = q.outgoing.notification.state == CondState::Firing;
                    let symptom = entity_of(q.origin.as_deref())
                        .and_then(|ent| symptoms.get(ent.as_str()).map(|r| (*r).to_string()));
                    let (Some(root), true) = (symptom, is_firing) else {
                        keep.push(q);
                        continue;
                    };
                    q.outgoing.notification.inhibited_by = Some(root.clone());
                    if let Some(e) = self.entries.get_mut(&q.id) {
                        e.inhibited_by = Some(root.clone());
                    }
                    if let Some(root_key) = root_keys.get(&root) {
                        moved.push((root_key.clone(), q));
                    } else if severity_rank(&q.outgoing.notification.severity)
                        >= severity_rank("error")
                    {
                        keep.push(q);
                    } else {
                        self.counters.inhibited += 1;
                        if let Some(e) = self.entries.get_mut(&q.id) {
                            e.held = true;
                        }
                        tracing::info!(id = %q.id, root = %root,
                            "held: a symptom of a down entity (RFC 06 §5.6)");
                    }
                }
                *members = keep;
            }
            for (root_key, q) in moved {
                if let Some((_, members)) = due.get_mut(&root_key) {
                    members.push(q);
                }
            }
            due.retain(|_, (_, m)| !m.is_empty());
        }
        flush.impact = Some(report);
    }

    fn group_key(&self, q: &Queued) -> (String, BTreeMap<String, String>) {
        let n = &q.outgoing.notification;
        let mut labels = BTreeMap::new();
        for l in &self.cfg.group_by {
            let v = if l == "origin" {
                q.origin.clone()
            } else {
                n.labels.get(l).cloned()
            };
            if let Some(v) = v {
                labels.insert(l.clone(), v);
            }
        }
        if labels.is_empty() || q.passthrough {
            return (format!("id:{}", n.id), labels);
        }
        let key = labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        (key, labels)
    }

    fn group_outgoing(
        &mut self,
        key: &str,
        labels: BTreeMap<String, String>,
        members: &[Queued],
        now: f64,
    ) -> Outgoing {
        self.groups_made += 1;
        let ns: Vec<&Notification> = members.iter().map(|q| &q.outgoing.notification).collect();
        let state = ns
            .iter()
            .map(|n| n.state)
            .max_by_key(|s| state_rank(*s))
            .unwrap_or(CondState::Ok);
        let severity = ns
            .iter()
            .map(|n| n.severity.as_str())
            .max_by_key(|s| severity_rank(s))
            .unwrap_or("warning")
            .to_string();
        let mut sinks: Vec<String> = Vec::new();
        for q in members {
            for s in &q.outgoing.sinks {
                if !sinks.contains(s) {
                    sinks.push(s.clone());
                }
            }
        }
        let mut lines: Vec<String> = ns
            .iter()
            .map(|n| match &n.inhibited_by {
                Some(root) => format!("- {} (symptom of {root})", n.title),
                None => format!("- {}", n.title),
            })
            .collect();
        lines.insert(0, format!("{} notices sharing {key}:", ns.len()));
        let (message, truncated) = truncate_bytes(&lines.join("\n"), self.max_message_bytes);
        Outgoing {
            notification: Notification {
                id: format!("group:{key}:{}", self.groups_made),
                rule: "group".into(),
                rule_kind: "group".into(),
                kind: NoticeKind::Group,
                state,
                prior: None,
                severity: severity.clone(),
                title: format!("{severity} {} notices from {key}", ns.len()),
                message,
                labels,
                at: rfc3339(now),
                evidence: format!(
                    "{} notices within {}s sharing {key}",
                    ns.len(),
                    self.cfg.group_window_s
                ),
                rendering: RenderSource::KeyOnly,
                truncated,
                repeat: 0,
                group: Some(ns.iter().map(|n| n.id.clone()).collect()),
                inhibited_by: None,
            },
            sinks,
        }
    }

    /// Bound the ledger: least recently changed first; an entry with a
    /// notification still queued is never the one to go.
    fn evict(&mut self) {
        while self.entries.len() > self.max_entries {
            let queued: BTreeSet<&str> = self.queue.iter().map(|q| q.id.as_str()).collect();
            let victim = self
                .entries
                .values()
                .filter(|e| !queued.contains(e.id.as_str()))
                .min_by(|a, b| {
                    a.changed
                        .partial_cmp(&b.changed)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| a.id.cmp(&b.id))
                })
                .map(|e| e.id.clone());
            let Some(id) = victim else {
                break;
            };
            self.entries.remove(&id);
            self.counters.evicted += 1;
            self.dirty = true;
        }
    }
}

/// The kind a rule's own firing notice carries.
pub fn base_kind(head: &str) -> NoticeKind {
    match head {
        "alerts" => NoticeKind::Alert,
        "liveliness-gone" => NoticeKind::Liveliness,
        _ => NoticeKind::Transition,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{InhibitConfig, RuleConfig};
    use zenoh::sample::SampleKind;

    fn rule(name: &str, rule: &str, for_s: Option<f64>) -> Rule {
        Rule::from_config(&RuleConfig {
            name: name.into(),
            rule: rule.into(),
            severity: None,
            labels: BTreeMap::new(),
            sinks: vec!["ops".into()],
            for_s,
        })
        .unwrap()
    }

    fn rules() -> Vec<Rule> {
        vec![
            rule("fleet-alerts", "alerts v1/*/state/*/alert/*", None),
            rule("hosts-gone", "liveliness-gone v1/*/state/*/alive", None),
            rule("slow", "silent-for v1/** 30", Some(10.0)),
        ]
    }

    fn cfg(window: f64, repeat: f64) -> DisciplineConfig {
        DisciplineConfig {
            group_window_s: window,
            repeat_s: repeat,
            inhibit: InhibitConfig {
                enabled: true,
                depth: 4,
            },
            ..DisciplineConfig::default()
        }
    }

    fn outgoing(rule: &str, id: &str, state: CondState, severity: &str) -> Outgoing {
        Outgoing {
            notification: Notification {
                id: id.into(),
                rule: rule.into(),
                rule_kind: "x".into(),
                kind: NoticeKind::Alert,
                state,
                prior: None,
                severity: severity.into(),
                title: format!("{severity} {rule} {}", state_word(state)),
                message: "m".into(),
                labels: BTreeMap::new(),
                at: "t".into(),
                evidence: "e".into(),
                rendering: RenderSource::KeyOnly,
                truncated: false,
                repeat: 0,
                group: None,
                inhibited_by: None,
            },
            sinks: vec!["ops".into()],
        }
    }

    fn meta(origin: &str) -> NoticeMeta {
        NoticeMeta {
            origin: Some(origin.into()),
            passthrough: false,
        }
    }

    fn flush(d: &mut Discipline, now: f64) -> Vec<Outgoing> {
        d.tick(now, None, &BTreeSet::new(), false).outgoing
    }

    fn kinds(out: &[Outgoing]) -> Vec<NoticeKind> {
        out.iter().map(|o| o.notification.kind).collect()
    }

    /// `for`: announced only when still firing at `start + for`; an `ok`
    /// before then cancels silently; `unobservable` mid-wait pauses.
    #[test]
    fn for_s_announces_late_cancels_silently_and_pauses_on_unobservable() {
        let r = rules();
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        let fire = |d: &mut Discipline, t: f64| {
            d.observe(
                outgoing("slow", "slow:a", CondState::Firing, "warning"),
                &meta("h-a"),
                t,
            )
        };
        fire(&mut d, 0.0);
        assert!(flush(&mut d, 5.0).is_empty(), "not yet");
        fire(&mut d, 6.0);
        assert!(
            flush(&mut d, 9.9).is_empty(),
            "a re-fire does not reset the timer"
        );
        let out = flush(&mut d, 10.0);
        assert_eq!(kinds(&out), vec![NoticeKind::Transition]);
        assert_eq!(out[0].notification.state, CondState::Firing);

        // Cancelled: firing, then ok at 4s — nothing, ever.
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        fire(&mut d, 0.0);
        d.observe(
            outgoing("slow", "slow:a", CondState::Ok, "warning"),
            &meta("h-a"),
            4.0,
        );
        assert!(flush(&mut d, 20.0).is_empty());
        assert_eq!(d.counters().cancelled, 1);

        // Paused: firing at 0, unobservable at 5, firing again at 7 — the 5s
        // already elapsed are credited, so it announces at 12, not 17.
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        fire(&mut d, 0.0);
        d.observe(
            outgoing("slow", "slow:a", CondState::Unobservable, "warning"),
            &meta("h-a"),
            5.0,
        );
        fire(&mut d, 7.0);
        assert!(flush(&mut d, 11.9).is_empty());
        assert_eq!(kinds(&flush(&mut d, 12.0)), vec![NoticeKind::Transition]);

        // Unobservable that persists past `for` is announced as such.
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        fire(&mut d, 0.0);
        d.observe(
            outgoing("slow", "slow:a", CondState::Unobservable, "warning"),
            &meta("h-a"),
            5.0,
        );
        assert!(flush(&mut d, 14.9).is_empty());
        let out = flush(&mut d, 15.0);
        assert_eq!(kinds(&out), vec![NoticeKind::Unobservable]);
        assert_eq!(out[0].notification.state, CondState::Unobservable);
    }

    /// Dedup and repeat: the same `(id, state, severity)` is nothing; a
    /// severity change is news; still firing past `repeat_s` re-sends with
    /// `repeat` counting up; `0` never repeats.
    #[test]
    fn dedup_suppresses_and_repeat_resends_once_per_interval() {
        let r = rules();
        let mut d = Discipline::new(&cfg(0.0, 100.0), &r, &RenderConfig::default(), 100);
        let fire = |d: &mut Discipline, t: f64, sev: &str| {
            d.observe(
                outgoing("fleet-alerts", "fleet-alerts:x", CondState::Firing, sev),
                &meta("h-a"),
                t,
            )
        };
        fire(&mut d, 0.0, "warning");
        assert_eq!(kinds(&flush(&mut d, 0.0)), vec![NoticeKind::Alert]);
        fire(&mut d, 10.0, "warning");
        assert!(flush(&mut d, 10.0).is_empty());
        assert_eq!(d.counters().deduped, 1);
        assert!(flush(&mut d, 99.0).is_empty());
        let out = flush(&mut d, 100.0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].notification.repeat, 1);
        assert!(out[0].notification.title.contains("repeat 1"));
        assert!(flush(&mut d, 150.0).is_empty());
        assert_eq!(flush(&mut d, 200.0)[0].notification.repeat, 2);
        fire(&mut d, 201.0, "critical");
        let out = flush(&mut d, 201.0);
        assert_eq!(out.len(), 1, "a severity change is announced");
        assert_eq!(out[0].notification.severity, "critical");

        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        fire(&mut d, 0.0, "warning");
        flush(&mut d, 0.0);
        assert!(flush(&mut d, 1e6).is_empty(), "repeat_s 0 never repeats");
    }

    /// Grouping: three notices from one origin within the window are one
    /// group carrying their ids; one from another origin goes plain; the
    /// window opens with the first member and flushes on the tick after
    /// it closes.
    #[test]
    fn notices_sharing_a_group_key_within_the_window_are_one_group() {
        let r = rules();
        let mut d = Discipline::new(&cfg(5.0, 0.0), &r, &RenderConfig::default(), 100);
        for (i, t) in [0.0, 1.0, 2.0].into_iter().enumerate() {
            d.observe(
                outgoing(
                    "fleet-alerts",
                    &format!("fleet-alerts:a{i}"),
                    CondState::Firing,
                    "warning",
                ),
                &meta("h-a"),
                t,
            );
        }
        d.observe(
            outgoing("hosts-gone", "hosts-gone:h-b/p", CondState::Firing, "error"),
            &meta("h-b"),
            0.0,
        );
        assert!(flush(&mut d, 4.9).is_empty(), "the window is still open");
        let out = flush(&mut d, 5.0);
        assert_eq!(
            kinds(&out),
            vec![NoticeKind::Group, NoticeKind::Liveliness],
            "ordered by group key"
        );
        let g = &out[0].notification;
        assert_eq!(
            g.group.as_deref(),
            Some(&["fleet-alerts:a0", "fleet-alerts:a1", "fleet-alerts:a2"].map(String::from)[..])
        );
        assert_eq!(g.state, CondState::Firing);
        assert_eq!(g.title, "warning 3 notices from origin=h-a");
        assert_eq!(g.labels.get("origin").map(String::as_str), Some("h-a"));
        assert!(
            g.message.contains("- warning fleet-alerts firing"),
            "{}",
            g.message
        );
        assert_eq!(d.counters().grouped, 3);
        assert_eq!(d.counters().sent, 2);
    }

    /// The resolved family, one notification each — and none of the two
    /// resolves with `resolved_notice: false`.
    #[test]
    fn the_four_resolved_kinds_are_distinct() {
        let r = rules();
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        let see = |d: &mut Discipline, id: &str, s: CondState, t: f64| {
            d.observe(outgoing("fleet-alerts", id, s, "warning"), &meta("h-a"), t);
            flush(d, t)
        };
        // firing → ok
        see(&mut d, "a", CondState::Firing, 0.0);
        assert_eq!(
            kinds(&see(&mut d, "a", CondState::Ok, 1.0)),
            vec![NoticeKind::Resolved]
        );
        // ok → unobservable, then unobservable → ok
        assert_eq!(
            kinds(&see(&mut d, "b", CondState::Unobservable, 2.0)),
            vec![NoticeKind::Unobservable]
        );
        assert_eq!(
            kinds(&see(&mut d, "b", CondState::Ok, 3.0)),
            vec![NoticeKind::ObservableAgain]
        );
        // firing → unobservable
        see(&mut d, "c", CondState::Firing, 4.0);
        assert_eq!(
            kinds(&see(&mut d, "c", CondState::Unobservable, 5.0)),
            vec![NoticeKind::LostSight]
        );
        // A first-observation ok is a baseline, not news.
        assert!(see(&mut d, "d", CondState::Ok, 6.0).is_empty());
        assert_eq!(d.counters().baseline, 1);

        let mut quiet = cfg(0.0, 0.0);
        quiet.resolved_notice = false;
        let mut d = Discipline::new(&quiet, &r, &RenderConfig::default(), 100);
        d.observe(
            outgoing("fleet-alerts", "a", CondState::Firing, "warning"),
            &meta("h-a"),
            0.0,
        );
        flush(&mut d, 0.0);
        d.observe(
            outgoing("fleet-alerts", "a", CondState::Ok, "warning"),
            &meta("h-a"),
            1.0,
        );
        assert!(flush(&mut d, 1.0).is_empty());
    }

    fn catalog() -> Catalog {
        let mut c = Catalog::default();
        c.apply(
            inhibit::Family::Entity,
            "v1/@catalog/state/entity/ent-a",
            SampleKind::Put,
            br#"{"entity_id":"ent-a","origins":["h-a"]}"#,
        );
        c.apply(
            inhibit::Family::Entity,
            "v1/@catalog/state/entity/ent-b",
            SampleKind::Put,
            br#"{"entity_id":"ent-b","origins":["h-b"]}"#,
        );
        c.apply(
            inhibit::Family::Edge,
            "v1/@catalog/state/edge/e-1",
            SampleKind::Put,
            br#"{"edge_id":"e-1","kind":"hosts","from":{"entity":{"entity_id":"ent-a"}},"to":{"entity":{"entity_id":"ent-b"}}}"#,
        );
        c
    }

    /// Inhibition: the root's liveliness and the symptom's alert in one
    /// flush fold into the root's group; a symptom alone is delivered with
    /// `inhibited_by` at `error`, held (counted) below it; its resolve is
    /// held too; and an `l2_adjacent` edge inhibits nothing.
    #[test]
    fn a_symptom_rides_with_its_root_or_is_held_and_counted() {
        let r = rules();
        let cat = catalog();
        let gone: BTreeSet<String> = ["h-a".to_string()].into();
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        d.observe(
            outgoing("hosts-gone", "hosts-gone:h-a/p", CondState::Firing, "error"),
            &meta("h-a"),
            0.0,
        );
        d.observe(
            outgoing(
                "fleet-alerts",
                "fleet-alerts:b",
                CondState::Firing,
                "warning",
            ),
            &meta("h-b"),
            0.0,
        );
        let f = d.tick(0.0, Some(&cat), &gone, false);
        assert_eq!(kinds(&f.outgoing), vec![NoticeKind::Group]);
        let g = &f.outgoing[0].notification;
        assert_eq!(
            g.group.as_deref(),
            Some(&["hosts-gone:h-a/p", "fleet-alerts:b"].map(String::from)[..])
        );
        assert!(g.message.contains("(symptom of ent-a)"), "{}", g.message);
        let impact = f.impact.unwrap();
        assert_eq!(impact.roots[0].entity, "ent-a");
        assert_eq!(impact.symptoms[0].explained_by, "ent-a");

        // Alone, below error: held and counted; its resolve is held too.
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        d.observe(
            outgoing(
                "fleet-alerts",
                "fleet-alerts:b",
                CondState::Firing,
                "warning",
            ),
            &meta("h-b"),
            0.0,
        );
        assert!(d.tick(0.0, Some(&cat), &gone, false).outgoing.is_empty());
        assert_eq!(d.counters().inhibited, 1);
        assert!(
            d.announced()
                .any(|e| e.inhibited_by.as_deref() == Some("ent-a"))
        );
        d.observe(
            outgoing("fleet-alerts", "fleet-alerts:b", CondState::Ok, "warning"),
            &meta("h-b"),
            1.0,
        );
        assert!(d.tick(1.0, Some(&cat), &gone, false).outgoing.is_empty());
        assert_eq!(d.counters().inhibited, 2);

        // Alone, at error: delivered, saying what explains it.
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        d.observe(
            outgoing("fleet-alerts", "fleet-alerts:b", CondState::Firing, "error"),
            &meta("h-b"),
            0.0,
        );
        let f = d.tick(0.0, Some(&cat), &gone, false);
        assert_eq!(f.outgoing.len(), 1);
        assert_eq!(
            f.outgoing[0].notification.inhibited_by.as_deref(),
            Some("ent-a")
        );

        // A symmetric kind is inert.
        let mut l2 = Catalog::default();
        for (k, doc) in [
            (
                "v1/@catalog/state/entity/ent-a",
                r#"{"entity_id":"ent-a","origins":["h-a"]}"#,
            ),
            (
                "v1/@catalog/state/entity/ent-b",
                r#"{"entity_id":"ent-b","origins":["h-b"]}"#,
            ),
        ] {
            l2.apply(inhibit::Family::Entity, k, SampleKind::Put, doc.as_bytes());
        }
        l2.apply(
            inhibit::Family::Edge,
            "v1/@catalog/state/edge/e-2",
            SampleKind::Put,
            br#"{"edge_id":"e-2","kind":"l2_adjacent","from":{"entity":{"entity_id":"ent-a"}},"to":{"entity":{"entity_id":"ent-b"}}}"#,
        );
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        d.observe(
            outgoing(
                "fleet-alerts",
                "fleet-alerts:b",
                CondState::Firing,
                "warning",
            ),
            &meta("h-b"),
            0.0,
        );
        let f = d.tick(0.0, Some(&l2), &gone, false);
        assert_eq!(f.outgoing.len(), 1);
        assert_eq!(f.outgoing[0].notification.inhibited_by, None);
        assert_eq!(d.counters().inhibited, 0);
    }

    /// The state file: a snapshot restores as announced, so the same firing
    /// again is a duplicate; the bound evicts least-recently-changed and
    /// counts it across the round trip; a dropped notice passes through
    /// every time.
    #[test]
    fn a_restored_ledger_deduplicates_and_the_bound_is_counted() {
        let r = rules();
        let mut d = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 2);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            d.observe(
                outgoing("fleet-alerts", id, CondState::Firing, "warning"),
                &meta("h-a"),
                i as f64,
            );
            flush(&mut d, i as f64);
        }
        assert_eq!(d.len(), 2);
        assert_eq!(
            d.counters().evicted,
            1,
            "a, the least recently changed, went"
        );
        assert!(d.dirty());
        let saved = d.snapshot(3.0);
        assert!(!d.dirty());
        assert_eq!(saved.entries.len(), 2);
        assert_eq!(saved.evicted_total, 1);
        assert_eq!(saved.entries[0].state, CondState::Firing);

        let mut fresh = Discipline::new(&cfg(0.0, 0.0), &r, &RenderConfig::default(), 100);
        assert_eq!(fresh.restore(saved), 2);
        assert_eq!(fresh.counters().evicted, 1, "the cost survives the restart");
        fresh.observe(
            outgoing("fleet-alerts", "b", CondState::Firing, "warning"),
            &meta("h-a"),
            10.0,
        );
        assert!(
            flush(&mut fresh, 10.0).is_empty(),
            "already announced before the restart"
        );
        assert_eq!(fresh.counters().deduped, 1);
        fresh.observe(
            outgoing("fleet-alerts", "b", CondState::Ok, "warning"),
            &meta("h-a"),
            11.0,
        );
        assert_eq!(kinds(&flush(&mut fresh, 11.0)), vec![NoticeKind::Resolved]);

        let dropped = NoticeMeta {
            origin: None,
            passthrough: true,
        };
        let mut o = outgoing(
            "fleet-alerts",
            "fleet-alerts:dropped",
            CondState::Unobservable,
            "info",
        );
        o.notification.kind = NoticeKind::Unobservable;
        fresh.observe(o.clone(), &dropped, 12.0);
        fresh.observe(o, &dropped, 12.0);
        assert_eq!(flush(&mut fresh, 12.0).len(), 2, "no identity, no dedup");
    }
}
