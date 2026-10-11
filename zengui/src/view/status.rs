//! The status strip: what zengui is actually watching, stated rather than
//! implied.
//!
//! This pane exists because of RFC 05 §3.1 and RFC 12 §9. An explorer showing
//! an empty tree looks identical whether the bus is quiet, the scope excludes
//! the traffic, the session reaches nothing, or no contract resolves a key.
//! Every one of those is stated here so the user never has to infer it.

use iced::Element;
use iced::widget::row;

use crate::message::{LinkState, Message};
use crate::state::deployment::ContractCounts;
use crate::view::kit::{self, human_bytes, human_rate};
use crate::view::theme::Tone;
use crate::view::tokens::space;

/// What the window holds of zk2's contracts (#614, FL6a): the last load,
/// and what the store has counted since.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContractsStatus<'a> {
    /// The last load, `None` until one landed — "not loaded", never
    /// "none" (O4).
    pub loaded: Option<&'a crate::services::contracts::LoadedContracts>,
    pub counts: ContractCounts,
}

impl ContractsStatus<'_> {
    /// The strip's contracts cell: where the revisions in hand came from,
    /// what retrieval found missing, and what a load could not read — each
    /// its own number, never folded into another (O6's posture, applied to
    /// a store). Split from the widget so it is testable.
    pub fn label(&self) -> String {
        let Some(loaded) = self.loaded else {
            return "contracts: not loaded".to_string();
        };
        let ContractCounts {
            offline,
            retrieved,
            unavailable,
            unreadable,
        } = self.counts;
        let mut label = if loaded.paths.is_empty() {
            format!("contracts: from the bus · {retrieved} retrieved")
        } else {
            format!("contracts: {offline} offline · {retrieved} retrieved")
        };
        label.push_str(&format!(" · {unavailable} unavailable"));
        if unreadable > 0 {
            label.push_str(&format!(" · {unreadable} unreadable"));
        }
        if !loaded.problems.is_empty() {
            label.push_str(&format!(
                " · {}",
                kit::plural(loaded.problems.len(), "load problem")
            ));
        }
        label
    }

    /// Whether the cell is worth a caution chip: a load that could not read
    /// something, or a revision that verified and does not read.
    pub fn troubled(&self) -> bool {
        self.loaded.is_some_and(|l| !l.problems.is_empty()) || self.counts.unreadable > 0
    }
}

/// The presence cell (#614, FL6a): how many addresses the projection holds
/// up, down and unobservable — on the basis its seed allows, which the cell
/// names when it is not complete. "Down" exists only under a complete seed
/// (§8.1), and a loss since makes every address unobservable at once: the
/// cell says so as a caution, never as everything down. `None` presence is
/// "not read", which is not "nobody there" (O4).
///
/// Returns the words, and whether they are worth a caution chip.
pub fn presence_text(p: Option<&crate::presence_link::PresenceUpdate>) -> (String, bool) {
    use zenkey_fleet::{AddressStatus, PresenceBasis};
    let Some(p) = p else {
        return ("presence: not read".to_string(), false);
    };
    let (mut up, mut down, mut unobservable) = (0usize, 0usize, 0usize);
    for (_, a) in p.presence.addresses() {
        match a.status {
            AddressStatus::Up => up += 1,
            AddressStatus::Down => down += 1,
            AddressStatus::Unobservable => unobservable += 1,
        }
    }
    let counts = format!("{up} up · {down} down · {unobservable} unobservable");
    match p.presence.basis() {
        PresenceBasis::Unseeded => ("presence: not seeded yet".to_string(), false),
        PresenceBasis::Incomplete => (
            format!("presence: {counts} — the seed was possibly incomplete (§8.1)"),
            false,
        ),
        PresenceBasis::Complete => (format!("presence: {counts}"), false),
        PresenceBasis::Lost { dropped } => (
            format!(
                "presence: {dropped} events lost — every address unobservable until a \
                 complete re-read"
            ),
            true,
        ),
    }
}

/// Everything the strip reports.
pub struct Status<'a> {
    pub link: &'a LinkState,
    pub namespace_label: &'a str,
    /// Active watch selectors (coverage, O5).
    pub watched: &'a [String],
    /// What fed the skeleton, when one was built.
    pub skeleton: Option<&'a zenkey_fleet::SkeletonCoverage>,
    /// Keys retired because their watch was released (O6's third counter).
    pub keys_unwatched: u64,
    /// The last on-demand value fetch, for the selection.
    pub fetched: Option<&'a (
        String,
        Result<std::sync::Arc<zenkey_fleet_v1::FetchOutcome>, crate::services::ServiceError>,
    )>,
    /// The scope's short name.
    pub scope_label: &'a str,
    /// What the scope covers, in words — "everything" is not everything
    /// (core §1.3), and this is where that is said. It moved here from the
    /// location bar with #536; the strip wraps rather than clipping, so a
    /// long statement no longer pushes the contracts state and the
    /// reachability warning off screen.
    pub scope_coverage: &'a str,
    pub keys: usize,
    /// Keys retired to stay within the table's bound.
    pub keys_evicted: u64,
    /// Key projections held by the facts cache (issue #107).
    pub facts_cached: usize,
    /// Projections retired to stay within the *cache's* bound — a separate
    /// number from `keys_evicted` on purpose, see [`facts_text`].
    pub facts_evicted: u64,
    /// zk2 identities held by the identity cache, and those its bound
    /// retired (#614, FL6a) — its own pair, beside the facts cache's.
    pub identity: (usize, u64),
    pub totals: crate::message::WatchedTotals,
    /// The contracts in hand (#614, FL6a).
    pub contracts: ContractsStatus<'a>,
    /// The last presence landing (#614, FL6a); `None` = not read.
    pub presence: Option<&'a crate::presence_link::PresenceUpdate>,
    /// Watches whose seed phase has not resolved yet (issue #92).
    pub seeding: usize,
    /// Seed phases that have completed since connect.
    pub seeded_watches: usize,
    /// Cumulative seed coverage since connect: (cache replies, storage
    /// replies, superseded).
    pub seed_totals: crate::state::observation::SeedTotals,
    pub unreachable: bool,
    /// Why the persisted preferences are not in force, when a file could not
    /// be read (issue #73). `None` on the ordinary path, including first run.
    pub prefs_note: Option<&'a str>,
    /// Replay mode (issue #74): the panes are fed from a file and the live
    /// link is off — the strip must not describe a link nobody is pumping.
    pub replaying: bool,
    /// The monitor's retained window (#217): the budget is deliberately
    /// visible here, in the resting state — an operator should not discover
    /// the bound by hitting it. `None` before a monitor exists (O4), and
    /// suppressed while replaying (the ring describes a bus the panes are
    /// not showing).
    pub retention: Option<zenkey_fleet::RetentionStats>,
}

impl<'a> Status<'a> {
    /// Read the strip's facts off five of the six sub-states (#175).
    ///
    /// This looks like [`Ctx`](crate::update::Ctx) and is its opposite. `Ctx`
    /// is an update-side type whose point is being *narrow* — what a handler
    /// may consult but not move. Nothing here is narrow, and nothing should
    /// be: a status strip that read fewer sub-states would be a status strip
    /// that told the user less. That it names five of six is what a status
    /// strip *is*.
    pub(crate) fn of(
        chrome: &'a crate::state::Chrome,
        dep: &'a crate::state::Deployment,
        obs: &'a crate::state::Observation,
        sub: &'a crate::state::SubjectState,
        work: &'a crate::state::Workspace,
    ) -> Status<'a> {
        Status {
            link: &obs.link,
            namespace_label: dep.namespace_label(),
            scope_label: dep.settings.scope.short(),
            scope_coverage: dep.settings.scope.label(),
            keys: obs.keys,
            keys_evicted: obs.keys_evicted,
            keys_unwatched: obs.keys_unwatched,
            facts_cached: dep.facts.len(),
            facts_evicted: dep.facts.evicted(),
            identity: (dep.identity.len(), dep.identity.evicted()),
            watched: &obs.watched,
            skeleton: dep.skeleton.as_deref().map(|s| &s.coverage),
            fetched: sub.follow.fetched.as_ref(),
            totals: obs.totals,
            contracts: ContractsStatus {
                loaded: dep.contracts.as_deref(),
                counts: dep.contract_counts,
            },
            presence: dep.presence.as_deref(),
            seeding: obs.seeding.len(),
            seeded_watches: obs.seeded_watches,
            seed_totals: obs.seed_totals,
            unreachable: dep.settings.is_unreachable(),
            prefs_note: chrome.prefs_note.as_deref(),
            replaying: work.replay.replay.is_some(),
            retention: obs.retention,
        }
    }
}
/// What the tree's declared half was built from (#614, FL6b): zk2's
/// skeleton, the O5 coverage statement typed — how many revisions placed
/// resources, at how many addresses presence named, what was named and not
/// held, and whether the admin space answered. A zero is an observation;
/// "n/a" is an admin space not asked or not answering, never a zero (O4).
pub fn skeleton_text(skeleton: Option<&zenkey_fleet::SkeletonCoverage>) -> String {
    let Some(c) = skeleton else {
        return "skeleton: not built".to_string();
    };
    let placed = match &c.presence {
        // Without a presence read the revisions sit under a symbolic
        // `{system}/{service}`: declared, not placed.
        None => "no presence read".to_string(),
        Some(p) if p.complete => kit::plural(c.addresses, "address"),
        Some(_) => format!(
            "{} (presence possibly incomplete)",
            kit::plural(c.addresses, "address")
        ),
    };
    let mut text = format!(
        "skeleton: {} · {placed}",
        kit::plural(c.revisions, "revision")
    );
    if c.unheld > 0 {
        text.push_str(&format!(" · {} not held", c.unheld));
    }
    if c.ambiguous > 0 {
        text.push_str(&format!(" · {} ambiguous", c.ambiguous));
    }
    text.push_str(" · admin ");
    match c.admin_entities {
        // Not asked / not available is not a zero (O4).
        None => text.push_str("n/a"),
        Some(n) => text.push_str(&n.to_string()),
    }
    text
}

/// The key count, and — if the table has evicted — what that count omits.
///
/// A bounded observer must report what its bound cost (RFC 09 §5.1 O6): a key
/// set that stops growing, or shrinks, is otherwise indistinguishable from a
/// bus that went quiet.
fn keys_label<'a>(keys: usize, evicted: u64) -> Element<'a, Message> {
    let label = keys_text(keys, evicted);
    if evicted == 0 {
        return kit::muted(label);
    }
    // A bound that has cost something is a caution chip (#537): the quiet
    // cells are words, so the one that matters stands out.
    kit::status_chip(Tone::Caution, label)
}

/// The wording of the key count. Split from the widget so it is testable.
pub fn keys_text(keys: usize, evicted: u64) -> String {
    if evicted == 0 {
        kit::plural(keys, "key")
    } else {
        format!(
            "{} (+{evicted} retired — bound reached)",
            kit::plural(keys, "key")
        )
    }
}

/// The projection cache's population, and what its bound cost (issue #107).
///
/// A **second** number beside [`keys_text`], never folded into it. The engine's
/// key table and the projection cache have different bounds and different
/// populations — the cache also holds liveliness-token keys, which never enter
/// the table — so one figure over both would answer neither question. O6 asks
/// what each bound cost, not what the bounds cost between them.
pub fn facts_text(cached: usize, evicted: u64) -> String {
    format!("facts: {cached} cached (+{evicted} projections retired — cache bound reached)")
}

/// The retained window's strip line (#217): the budget stated in the
/// resting state, the span actually held, and — only once the byte bound
/// has bitten — its own eviction count. A **fourth** number beside
/// [`keys_text`], `facts_text` and the unwatch line, never folded into any
/// of them: each bound reports its own cost (RFC 09 §5.1 O6; v1.18 R1).
pub fn retention_text(r: &zenkey_fleet::RetentionStats) -> String {
    let mut label = format!(
        "retained: {:.0}s held · budget {}",
        r.span.as_secs_f64(),
        crate::view::replay::budget_label(r.budget),
    );
    if r.evicted > 0 {
        label.push_str(&format!(" (+{} evicted by the byte budget)", r.evicted));
    }
    label
}

pub fn strip<'a>(s: Status<'a>) -> Element<'a, Message> {
    // `Some(tone)` = worth a chip. Replay is a mode, not a fault (#544):
    // its chip is the mode's, where a failed link's is danger's.
    let (link_text, chip) = match s.link {
        // Replay wins over whatever the link was doing: the panes are
        // showing a file, and the strip says so (#74).
        _ if s.replaying => ("REPLAY — live link off".to_string(), Some(Tone::Mode)),
        LinkState::Connecting => ("connecting…".to_string(), None),
        LinkState::Pumping if s.watched.is_empty() => (
            // The lazy resting state, stated as a posture, not an absence.
            "skeleton only — nothing observed by choice".to_string(),
            None,
        ),
        LinkState::Pumping => (
            format!("observing {}", kit::plural(s.watched.len(), "watch")),
            None,
        ),
        LinkState::Ended => ("link ended — retrying".to_string(), Some(Tone::Negative)),
        LinkState::Failed(e) => (format!("link failed: {e}"), Some(Tone::Negative)),
    };

    let link: Element<'a, Message> = match chip {
        Some(tone) => kit::status_chip(tone, link_text),
        None => kit::muted(link_text),
    };

    let crate::message::WatchedTotals {
        samples: count,
        bytes,
        rate_hz: rate,
    } = s.totals;

    let contracts: Element<'a, Message> = if s.contracts.troubled() {
        kit::status_chip(Tone::Caution, s.contracts.label())
    } else {
        kit::muted(s.contracts.label())
    };
    let presence: Element<'a, Message> = match presence_text(s.presence) {
        (text, true) => kit::status_chip(Tone::Caution, text),
        (text, false) => kit::muted(text),
    };
    let mut r = row![
        link,
        kit::muted(format!("namespace: {}", s.namespace_label)),
        kit::muted(if s.scope_coverage.is_empty() {
            format!("scope: {}", s.scope_label)
        } else {
            format!("scope: {} ({})", s.scope_label, s.scope_coverage)
        }),
        keys_label(s.keys, s.keys_evicted),
        kit::muted(format!(
            "{count} samples · {} · {}",
            human_bytes(bytes),
            human_rate(rate)
        )),
        contracts,
        presence,
        kit::muted(skeleton_text(s.skeleton)),
    ]
    .spacing(space::MD)
    .align_y(iced::Alignment::Center);

    // The seed phase is stated while it runs and summarized once done —
    // zeros are observations (O4): a seed that ran and found nothing says
    // so, it does not vanish.
    if s.seeding > 0 {
        r = r.push(kit::muted(format!(
            "seeding {}…",
            kit::plural(s.seeding, "watch")
        )));
    } else if s.seeded_watches > 0 {
        // `history` is the cache: same fact, the word the user reads.
        let crate::state::observation::SeedTotals {
            history: cache,
            storage,
            superseded,
        } = s.seed_totals;
        r = r.push(kit::muted(format!(
            "seeded {}: {cache} cache · {storage} storage · {superseded} superseded",
            kit::plural(s.seeded_watches, "watch")
        )));
    }
    if s.keys_unwatched > 0 {
        r = r.push(kit::muted(format!(
            "{} keys retired by unwatch",
            s.keys_unwatched
        )));
    }
    // The retained window (#217): stated while live, warning-toned once its
    // byte bound has cost something — like every other bound on this strip,
    // and always as its own number.
    if let Some(ret) = s.retention.as_ref().filter(|_| !s.replaying) {
        let label = retention_text(ret);
        r = r.push(if ret.evicted > 0 {
            kit::status_chip(Tone::Caution, label)
        } else {
            kit::muted(label)
        });
    }
    // Only once the bound has actually bitten: the key count already states
    // the population, so a quiet cache has nothing to disclose. Warning tone,
    // like every other bound that is costing something.
    if s.facts_evicted > 0 {
        r = r.push(kit::status_chip(
            Tone::Caution,
            facts_text(s.facts_cached, s.facts_evicted),
        ));
    }
    // The identity cache's bound (#614, FL6a), the same way: said once it
    // has cost something, as its own number.
    let (identities, identities_evicted) = s.identity;
    if identities_evicted > 0 {
        r = r.push(kit::status_chip(
            Tone::Caution,
            format!(
                "identities: {identities} cached (+{identities_evicted} retired — cache \
                 bound reached)"
            ),
        ));
    }
    if let Some((key, outcome)) = s.fetched {
        let label = match outcome {
            Ok(out) => match out.as_ref() {
                zenkey_fleet_v1::FetchOutcome::Value(v) => {
                    format!(
                        "fetched {key}: {} bytes via {}",
                        v.payload.len(),
                        crate::view::detail::source_word(v.source)
                    )
                }
                zenkey_fleet_v1::FetchOutcome::None { attempted } => format!(
                    "no value for {key} — asked {} (not a verdict)",
                    attempted.join(", ")
                ),
            },
            Err(e) => format!("fetch {key} failed: {e}"),
        };
        r = r.push(kit::muted(label));
    }

    // A preferences file that could not be read is worth one line: the window
    // looks reset, and "your settings did not load" is a much better
    // explanation than the user re-deriving it (issue #73).
    if let Some(note) = s.prefs_note {
        r = r.push(kit::status_chip(
            Tone::Caution,
            format!("preferences: {note}"),
        ));
    }

    // The single most misleading state a bus explorer can be in: a healthy
    // window, an empty tree, and no way to tell that the session never reached
    // anything. Say it outright.
    if s.unreachable {
        r = r.push(kit::status_chip(
            Tone::Negative,
            "no endpoints and scouting off — this session reaches nothing",
        ));
    }

    // It wraps (#536): a strip that clips loses its right-hand end — the
    // contracts state and the reachability warning — exactly when there is
    // most to say.
    r.wrap().vertical_spacing(space::XS).into()
}

/// The link's state as a tone and a word (#536) — the connection pill's
/// glance. The strip's first cell keeps the full posture sentence; this is
/// the word beside the dot, and the dot never stands without it.
///
/// Replay wins, as it does in the strip: the panes are showing a file — in
/// the mode's tone, never caution's (#544). A session that reaches nothing
/// says so even while its pump runs.
pub fn link_word(link: &LinkState, replaying: bool, unreachable: bool) -> (Tone, &'static str) {
    if replaying {
        return (Tone::Mode, "replay — live link off");
    }
    if unreachable {
        return (Tone::Negative, "reaches nothing");
    }
    match link {
        LinkState::Connecting => (Tone::Caution, "connecting…"),
        LinkState::Pumping => (Tone::Positive, "connected"),
        LinkState::Ended => (Tone::Caution, "link ended — retrying"),
        LinkState::Failed(_) => (Tone::Negative, "link failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pill's word (#536): every link state has one, the healthy and
    /// the failed states never share one, and the dot's tone agrees with the
    /// word — a failure is never a green dot.
    #[test]
    fn every_link_state_has_its_own_word() {
        let failed = LinkState::Failed(crate::services::ServiceError::from("router refused"));
        let states = [
            LinkState::Connecting,
            LinkState::Pumping,
            LinkState::Ended,
            failed,
        ];
        let words: Vec<_> = states.iter().map(|l| link_word(l, false, false)).collect();
        for (i, (_, a)) in words.iter().enumerate() {
            assert!(!a.is_empty());
            for (_, b) in &words[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(
            link_word(&LinkState::Pumping, false, false).0,
            Tone::Positive
        );
        assert_eq!(words[3].0, Tone::Negative);
        assert_eq!(
            link_word(&LinkState::Pumping, true, false),
            (Tone::Mode, "replay — live link off"),
            "replay wins, as it does in the strip — as a mode, not a caution (#544)"
        );
        assert_eq!(
            link_word(&LinkState::Pumping, false, true),
            (Tone::Negative, "reaches nothing"),
            "a session that reaches nothing is not connected, whatever the pump says"
        );
    }

    /// "Not loaded" and "loaded, and empty" are different facts and must read
    /// differently (#614, FL6a) — otherwise a load that has not landed looks
    /// like a deployment no contract describes. Every count is its own
    /// number, and a load problem is said beside what did load, never
    /// instead of it.
    #[test]
    fn the_contracts_cell_distinguishes_absent_from_empty() {
        use crate::services::contracts::LoadedContracts;
        let absent = ContractsStatus::default();
        assert_eq!(absent.label(), "contracts: not loaded");
        assert!(!absent.troubled());

        let bus_only = LoadedContracts::default();
        let s = ContractsStatus {
            loaded: Some(&bus_only),
            counts: ContractCounts {
                retrieved: 2,
                unavailable: 1,
                ..ContractCounts::default()
            },
        };
        assert_eq!(
            s.label(),
            "contracts: from the bus · 2 retrieved · 1 unavailable"
        );
        assert_ne!(s.label(), absent.label());

        let with_problem = LoadedContracts {
            paths: vec!["contracts".into(), "gone".into()],
            problems: vec![zenkey_fleet::LoadProblem {
                at: "gone".into(),
                message: "no such file or directory".into(),
            }],
            ..LoadedContracts::default()
        };
        let s = ContractsStatus {
            loaded: Some(&with_problem),
            counts: ContractCounts {
                offline: 7,
                ..ContractCounts::default()
            },
        };
        let label = s.label();
        assert!(
            label.starts_with("contracts: 7 offline · 0 retrieved"),
            "{label}"
        );
        assert!(label.contains("1 load problem"), "{label}");
        assert!(s.troubled(), "a problem is a caution chip");
    }
    /// The presence cell (#614, FL6a): "not read" is not "nobody there", a
    /// complete seed counts up and down, and a loss since is a caution that
    /// says every address is unobservable — never everything down.
    #[test]
    fn the_presence_cell_keeps_not_read_complete_and_lost_apart() {
        use std::sync::Arc;
        use zenkey_fleet::{LivePresence, ObservedPresence, PresenceScope, SeedOutcome};

        assert_eq!(
            presence_text(None),
            ("presence: not read".to_string(), false)
        );

        let keys = vec![
            "zk2/robot1/nav/@zk/instance/0123456789abcdef".to_string(),
            "zk2/host-a/tc/@zk/instance/fedcba9876543210".to_string(),
        ];
        let observed = ObservedPresence::from_keys("zk2/*/*/@zk/**", &keys, true);
        let mut live = LivePresence::new(PresenceScope::all());
        let at = std::time::SystemTime::now();
        let moved = live.seed(&observed, at);
        let update = |live: &LivePresence| crate::presence_link::PresenceUpdate {
            presence: Arc::new(live.clone()),
            moved: moved.clone(),
            catalog: None,
            generation: 0,
            seed: SeedOutcome::Complete,
        };
        assert_eq!(
            presence_text(Some(&update(&live))),
            (
                "presence: 2 up · 0 down · 0 unobservable".to_string(),
                false
            )
        );

        live.dropped(3, at);
        let (text, caution) = presence_text(Some(&update(&live)));
        assert!(caution, "a loss is a caution chip");
        assert!(text.contains("3 events lost"), "{text}");
        assert!(!text.contains("down"), "a loss is never 'down': {text}");
    }

    /// #217: the budget is visible in the resting state — an operator should
    /// not discover the bound by hitting it — and the ring's eviction count
    /// appears as its own number, never inside the key count's (O6).
    #[test]
    fn the_retained_window_states_its_budget_and_its_own_cost() {
        let mut r = zenkey_fleet::RetentionStats {
            budget: zenkey_fleet::RetentionBudget::default(),
            retained: 10,
            retained_bytes: 4096,
            span: std::time::Duration::from_secs(85),
            evicted: 0,
            expired: 3,
        };
        let quiet = retention_text(&r);
        assert!(quiet.contains("85s held"), "{quiet}");
        assert!(quiet.contains("2 min"), "{quiet}");
        assert!(!quiet.contains("evicted"), "no cost yet: {quiet}");

        r.evicted = 12;
        let costly = retention_text(&r);
        assert!(
            costly.contains("+12 evicted by the byte budget"),
            "{costly}"
        );
        assert!(
            !costly.contains("retired"),
            "the stats table's word stays the stats table's: {costly}"
        );
    }

    /// RFC 09 §5.1 O6: a bounded observer reports what its bound cost. Without
    /// this, a key count that stops growing looks like a bus that went quiet.
    #[test]
    fn the_key_count_discloses_eviction() {
        assert_eq!(keys_text(120, 0), "120 keys");
        let noted = keys_text(120, 7);
        assert!(noted.contains("120 keys"), "{noted}");
        assert!(noted.contains('7'), "{noted}");
        assert!(noted.contains("retired"), "{noted}");
    }
}
