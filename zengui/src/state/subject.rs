//! The subject **slots** (#181, #257): what the workspace looks at, and
//! everything derived from each subject.
//!
//! Since #257 the workspace holds N subjects, not one. [`SubjectSlot`] is the
//! old singular `SubjectState`: one subject plus every field derived from it
//! — the recorder, the rate sampler, the chart, the fetch, the decode, the
//! Fields and Why sections. [`SubjectState`] is the collection: slot 0
//! ([`SlotId::FOLLOW`]) is the subject the tree and the location bar drive,
//! and every further slot is a *pin* — a subject a torn-off Inspector holds
//! while the selection moves on.
//!
//! The issue's finding is why this is a slot and not an `Option<Subject>`
//! beside the current one: six fields here are rebuilt when the subject
//! moves, so freezing only the identity would title a pane with one key
//! while its chart described another — the O4 failure #181 removed from the
//! fetch path, reintroduced one panel over. A pin that is a special case is
//! a pin that is wrong in the tear-off window; a pin that is a slot is just
//! another subject the tick already feeds.
//!
//! No `forget`, and that is the finding rather than an omission: a subject
//! follows the *user*, not the fleet. Switching base leaves the same keys
//! selected and pinned, and the panes then say honestly that they have no
//! value for them yet — which is what "not asked" means (O4).

use std::sync::Arc;

use zenkey_fleet::FetchOutcome;

use super::deployment::Deployment;
use crate::message::{SlotId, Subject};
use crate::view;

sub_state! {
    /// One subject and everything derived from it — per slot since #257.
    pub(crate) struct SubjectSlot {
        /// Which slot this is. [`SlotId::FOLLOW`] is the tree's; anything
        /// else is a pin, held by the window bound to it.
        pub(crate) id: SlotId,
        /// What this slot is looking at (#181).
        pub(crate) current: Subject,
        /// The slot key's observed skewed-latency summary, refreshed on the
        /// bus tick (#119) — never computed on the render path, and cleared
        /// with the selection.
        pub(crate) selected_latency: Option<(zenkey_fleet::LatencyReport, u64)>,
        /// The last on-demand fetch: (key, outcome-or-error).
        pub(crate) fetched: Option<(String, Result<Arc<FetchOutcome>, crate::services::ServiceError>)>,
        /// The decode of the last fetched value — the whole
        /// [`crate::value::DecodedValue`]: the decode, verdict included
        /// (#164), plus the document rendered from it once (#345).
        pub(crate) decoded: Option<Arc<crate::value::DecodedValue>>,
        /// The timeline's scroll position + viewport height, driving its
        /// virtual window (#183).
        ///
        /// It lives with the recorder rather than in the workspace's chrome,
        /// and that placement is the behaviour: a new subject is a new
        /// timeline, so it starts at the top rather than wherever the last
        /// key's list happened to be.
        pub(crate) history_scroll: crate::view::kit::Viewport,
        /// The slot key's history recording (issue #63). Created on selection
        /// (or cloned at pin time, #257), dropped with the slot — which is
        /// what makes unpinning stop the cost, since there is then nothing
        /// left to feed.
        pub(crate) history: Option<crate::history::HistoryRecorder>,
        /// The slot key's rate series (issue #64), sampled once per stats
        /// tick. Reset with the selection, like the history it sits beside.
        pub(crate) rate_series: crate::series::RateSampler,
        /// Which numeric leaf the value sparkline plots; `None` follows the first
        /// leaf the payload offers.
        pub(crate) series_leaf: Option<String>,
        /// The detail section's chart data, rebuilt when its **inputs** change
        /// rather than on every frame (#178).
        ///
        /// It was computed inside `view()`, which meant walking the history ring
        /// twice and cloning the whole rate series ~60 times a second for a
        /// picture that changes on the 250 ms tick. `refresh_series` is the one
        /// rebuild point; everything that can change the chart calls it, and
        /// nothing else may write this field.
        pub(crate) series: Option<view::detail::SeriesData>,
        /// The bounded field observation on the slot key (#223): run on
        /// demand, dropped with the subject — its report is evidence about
        /// one key's window.
        pub(crate) fields: view::fields::FieldsState,
        /// The why ladder's state for the slot key (#214): run on demand
        /// at the frugal default, dropped with the subject.
        pub(crate) why: view::why::WhyState,
        /// The declared readers of the slot's subject (#224): one admin
        /// sweep per click, never ambient, dropped with the subject.
        pub(crate) consumers: view::consumers::ConsumersState,
        /// Compare the key's newest sample against the loaded `.zsnap`'s
        /// row for it (#219), when one is loaded and carries the key.
        pub(crate) compare_snapshot: bool,
    }
}

impl SubjectSlot {
    pub(crate) fn new(id: SlotId) -> SubjectSlot {
        SubjectSlot {
            id,
            current: Subject::None,
            selected_latency: None,
            fetched: None,
            decoded: None,
            history_scroll: crate::view::kit::Viewport::default(),
            history: None,
            rate_series: crate::series::RateSampler::new(),
            series_leaf: None,
            series: None,
            fields: view::fields::FieldsState::default(),
            why: view::why::WhyState::default(),
            consumers: view::consumers::ConsumersState::default(),
            compare_snapshot: false,
        }
    }

    /// Rebuild the detail section's chart data.
    ///
    /// The one rebuild point (#178): everything that can change the chart
    /// calls this, and nothing else writes `series`. It takes the deployment
    /// because a registered unit is a *registry* fact about the key, not
    /// something the history ring knows.
    pub(crate) fn refresh_series(&mut self, dep: &Deployment) {
        self.series = self.series_data(dep);
    }

    /// Derive the detail section's sparkline data from the recorded history
    /// (issue #64).
    ///
    /// The ring is the single source, so nothing here is cached beyond the
    /// `SeriesData` this returns — a second cache would be one more thing to
    /// invalidate on every eviction. The leaves come from the newest payload,
    /// so a producer that starts emitting a new field offers it without a
    /// restart.
    fn series_data(&self, dep: &Deployment) -> Option<view::detail::SeriesData> {
        let rec = self.history.as_ref()?;
        // The most recent entry that *is* a document, not simply the most
        // recent one: a tombstone carries no fields, and letting it empty the
        // picker would make the chart vanish on every retirement and come
        // back on the next put.
        let leaves = rec
            .ring
            .iter()
            .find_map(|e| e.value.as_ref())
            .map(crate::series::numeric_leaves)
            .unwrap_or_default();
        // The chosen leaf, if the newest payload still carries it — a field
        // that disappeared should not silently keep plotting its own gaps.
        let leaf = self
            .series_leaf
            .as_ref()
            .filter(|p| leaves.leaves.iter().any(|(k, _)| k == *p))
            .cloned()
            .or_else(|| leaves.leaves.first().map(|(k, _)| k.clone()));
        let value = match &leaf {
            Some(p) => crate::series::value_series(&rec.ring, p),
            None => crate::series::Series::new(),
        };
        let unit = match dep.facts.get(&rec.key).map(|f| &f.registration) {
            Some(zenkey_fleet::Registration::Registered(s)) => s.unit.clone(),
            _ => None,
        };
        Some(view::detail::SeriesData {
            leaves,
            leaf,
            value,
            // A fresh `SeriesData` is a cleared cache: this function is
            // called exactly when the chart's inputs moved, which is exactly
            // when the retained geometry stopped being valid (#178).
            caches: view::detail::SeriesCaches::default(),
            rate: self.rate_series.series().clone(),
            unit,
        })
    }
}

sub_state! {
    /// The workspace's subjects (#257): the follow slot the tree drives,
    /// plus one slot per pin.
    pub(crate) struct SubjectState {
        /// The slot the tree and the location bar drive.
        ///
        /// A **field**, not index 0 of a `Vec` (#358). It was the latter, with
        /// a doc comment saying `slots[0]` is always the follow slot and three
        /// methods defending that: two indexing `[0]` and panicking if it were
        /// ever untrue, and `unpin`/`drop_pins` retaining `!is_pin()` to keep
        /// it there. The invariant they defended is now the shape.
        pub(crate) follow: SubjectSlot,
        /// The pins, in pin order. The follow slot is structurally not among
        /// them, so "the follow slot refuses to be unpinned" needs no guard.
        pub(crate) pins: Vec<SubjectSlot>,
        /// The id the next pin gets. Never reused within a session, so a
        /// message routed to a dropped slot misses instead of landing in a
        /// stranger.
        pub(crate) next_slot: SlotId,
    }
}

impl Default for SubjectState {
    fn default() -> SubjectState {
        SubjectState {
            follow: SubjectSlot::new(SlotId::FOLLOW),
            pins: Vec::new(),
            next_slot: SlotId::FOLLOW.next(),
        }
    }
}

impl SubjectState {
    /// Every slot: the follow slot first, then the pins in pin order — the
    /// order `slots` used to hold them in, so every sweep over "all subjects"
    /// reads the same as before.
    pub(crate) fn all(&self) -> impl Iterator<Item = &SubjectSlot> {
        std::iter::once(&self.follow).chain(self.pins.iter())
    }

    pub(crate) fn all_mut(&mut self) -> impl Iterator<Item = &mut SubjectSlot> {
        std::iter::once(&mut self.follow).chain(self.pins.iter_mut())
    }

    /// How many slots the workspace holds — the follow slot plus its pins.
    ///
    /// Only the tear-off tests count slots; the code paths that need them all
    /// iterate `all()`.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        1 + self.pins.len()
    }

    pub(crate) fn slot(&self, id: SlotId) -> Option<&SubjectSlot> {
        self.all().find(|s| s.id == id)
    }

    pub(crate) fn slot_mut(&mut self, id: SlotId) -> Option<&mut SubjectSlot> {
        self.all_mut().find(|s| s.id == id)
    }

    /// Pin the follow slot's subject (#257): mint a new slot carrying the
    /// whole derivation, so the pinned pane keeps showing exactly what it
    /// showed — the history recorded so far, the fetch, the decode, the
    /// chart — and evolves independently from here on.
    ///
    /// The recorder is *cloned*, not shared: from this instant the two slots
    /// are two observers, and each ring counts its own retention and its own
    /// evictions. The Fields and Why sections start over instead — their
    /// landings route by slot id, so a report in flight for the follow slot
    /// must not be awaited by the pin (only the window *input* carries over;
    /// it is the user's, not the fleet's).
    pub(crate) fn pin_current(&mut self, dep: &Deployment) -> SlotId {
        let id = self.next_slot;
        self.next_slot = self.next_slot.next();
        let follow = &self.follow;
        let fields = view::fields::FieldsState {
            window: follow.fields.window.clone(),
            ..Default::default()
        };
        let mut slot = SubjectSlot {
            id,
            current: follow.current.clone(),
            selected_latency: follow.selected_latency.clone(),
            fetched: follow.fetched.clone(),
            decoded: follow.decoded.clone(),
            history_scroll: follow.history_scroll,
            history: follow.history.clone(),
            rate_series: follow.rate_series.clone(),
            series_leaf: follow.series_leaf.clone(),
            // Rebuilt below: a `SeriesData` carries retained canvas geometry,
            // which is per-surface by nature.
            series: None,
            fields,
            why: view::why::WhyState::default(),
            consumers: view::consumers::ConsumersState::default(),
            // A view toggle carries over like the scroll offset: the pin
            // keeps showing what the follow slot was showing.
            compare_snapshot: follow.compare_snapshot,
        };
        slot.refresh_series(dep);
        self.pins.push(slot);
        id
    }

    /// Drop one pinned slot — its recorder, its chart, its fetch — and
    /// nothing of any other slot's. The follow slot refuses: it is the
    /// workspace's, not any pane's.
    pub(crate) fn unpin(&mut self, id: SlotId) -> bool {
        // No `is_pin` guard: `pins` cannot contain the follow slot, so an id
        // that is not a pin simply matches nothing (#358).
        let before = self.pins.len();
        self.pins.retain(|s| s.id != id);
        self.pins.len() != before
    }

    /// Drop every pin — the layout presets are fully docked, and a pinned
    /// window they close takes its slot with it (#186, #257).
    pub(crate) fn drop_pins(&mut self) {
        self.pins.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep() -> Deployment {
        Deployment::new(crate::app::test_settings())
    }

    /// The pin carries the derivation, not just the identity — the issue's
    /// whole point. And dropping it drops exactly its own evidence.
    #[test]
    fn a_pin_carries_the_evidence_and_unpinning_drops_only_its_own() {
        let dep = dep();
        let mut sub = SubjectState::default();
        let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";
        sub.follow.current = Subject::Key(key.into());
        sub.follow.history = Some(crate::history::HistoryRecorder::new(key, 10));

        let pin = sub.pin_current(&dep);
        assert!(pin.is_pin());
        let pinned = sub.slot(pin).expect("just minted");
        assert_eq!(pinned.current.key(), Some(key));
        assert!(
            pinned.history.is_some(),
            "the pin keeps recording what the follow slot was recording"
        );

        assert!(!sub.unpin(SlotId::FOLLOW), "the follow slot is not a pin");
        assert!(sub.unpin(pin));
        assert!(sub.slot(pin).is_none());
        assert!(
            sub.follow.history.is_some(),
            "unpinning must not touch the follow slot's recorder"
        );
        assert!(!sub.unpin(pin), "a dropped slot stays dropped");
    }

    /// Ids are never reused within a session: a message routed to a dropped
    /// slot must miss, not land in whatever slot was minted after it.
    #[test]
    fn slot_ids_are_not_reused() {
        let dep = dep();
        let mut sub = SubjectState::default();
        let first = sub.pin_current(&dep);
        assert!(sub.unpin(first));
        let second = sub.pin_current(&dep);
        assert_ne!(first, second);
        assert!(sub.slot(first).is_none());
    }
}
