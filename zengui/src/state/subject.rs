//! The selected key, and everything derived from it.
//!
//! No `forget`, and that is the finding rather than an omission: a selection
//! follows the *user*, not the fleet. Switching base leaves the same key
//! selected, and the panes then say honestly that they have no value for it
//! yet — which is what "not asked" means (O4).
//!
//! Two panes have no state of their own and mutate this group instead: Detail
//! and History are windows onto the selection. That is worth knowing before
//! #180 docks the eleven panes — two of them have nothing to dock.

use std::sync::Arc;

use zenkey_fleet::FetchOutcome;

use crate::view;

sub_state! {
    #[derive(Default)]
    pub(crate) struct Subject {
        pub(crate) selected: Option<String>,
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
