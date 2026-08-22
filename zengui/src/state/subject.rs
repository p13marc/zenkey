//! The [`Subject`] the whole workspace follows, and everything derived from it.
//!
//! The struct is `SubjectState` and the subject itself is
//! [`Subject`](crate::message::Subject), which lives in `message.rs` because
//! that is what carries it — and because `view` is public while `state` is
//! `pub(crate)`, so a pane function could not name it from here.
//! `SubjectState` follows the existing `TreeState` precedent: the sub-state
//! that *holds* a thing is named for the thing plus `State`.
//!
//! No `forget`, and that is the finding rather than an omission: a subject
//! follows the *user*, not the fleet. Switching base leaves the same key
//! selected, and the panes then say honestly that they have no value for it
//! yet — which is what "not asked" means (O4).
//!
//! Two panes have no state of their own and mutate this group instead: Detail
//! and History are windows onto the subject. That is worth knowing before #180
//! docks the eleven panes — two of them have nothing to dock.

use std::sync::Arc;

use zenkey_fleet::FetchOutcome;

use super::deployment::Deployment;
use crate::message::Subject;
use crate::view;

sub_state! {
    #[derive(Default)]
    pub(crate) struct SubjectState {
        /// What the workspace is looking at (#181).
        pub(crate) current: Subject,
        /// The selected key's observed skewed-latency summary, refreshed on the
        /// bus tick (#119) — never computed on the render path, and cleared
        /// with the selection.
        pub(crate) selected_latency: Option<(zenkey_fleet::LatencyReport, u64)>,
        /// The last on-demand fetch: (key, outcome-or-error).
        pub(crate) fetched: Option<(String, Result<Arc<FetchOutcome>, String>)>,
        /// The decode of the last fetched value.
        pub(crate) decoded: Option<(Option<String>, zenkey_fleet::decode::Rendering)>,
        /// The selected key's history recording (issue #63). Created on selection,
        /// dropped on the next one — which is what makes deselecting stop the
        /// cost, since there is then nothing left to feed.
        pub(crate) history: Option<crate::history::HistoryRecorder>,
        /// The selected key's rate series (issue #64), sampled once per stats
        /// tick. Reset with the selection, like the history it sits beside.
        pub(crate) rate_series: crate::series::RateSampler,
        /// Which numeric leaf the value sparkline plots; `None` follows the first
        /// leaf the payload offers.
        pub(crate) series_leaf: Option<String>,
        /// The detail pane's chart data, rebuilt when its **inputs** change rather
        /// than on every frame (#178).
        ///
        /// It was computed inside `view()`, which meant walking the history ring
        /// twice and cloning the whole rate series ~60 times a second for a
        /// picture that changes on the 250 ms tick. `refresh_series` is the one
        /// rebuild point; everything that can change the chart calls it, and
        /// nothing else may write this field.
        pub(crate) series: Option<view::detail::SeriesData>,
    }
}

impl SubjectState {
    /// Rebuild the detail pane's chart data.
    ///
    /// The one rebuild point (#178): everything that can change the chart
    /// calls this, and nothing else writes `series`. It takes the deployment
    /// because a registered unit is a *registry* fact about the key, not
    /// something the history ring knows.
    pub(crate) fn refresh_series(&mut self, dep: &Deployment) {
        self.series = self.series_data(dep);
    }

    /// Derive the detail pane's sparkline data from the recorded history
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
