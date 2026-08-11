//! The admin & storage panel's model (#70): one sweep of the Zenoh admin
//! space, assembled off the UI thread, plus the staleness guard that keeps a
//! sweep about one deployment from being displayed as evidence about another.
//!
//! Pure logic, no widgets — the pane renders standalone in a test.
//!
//! **No key expression is spelled here.** Every admin selector lives inside
//! `zenkey_fleet::admin`; this module holds what came back. That is the right
//! posture for a plane whose shape is the middleware's rather than this
//! convention's, and it is why `scope.rs` needs no new entry.

use std::collections::BTreeSet;
use std::sync::Arc;

use zenkey_fleet::report::StorageList;
use zenkey_fleet::{DeclaredEntities, RouterInfo};

/// One sweep's worth of admin space.
#[derive(Debug)]
pub struct AdminSweep {
    pub routers: Vec<RouterInfo>,
    /// Storages *and* their coverage join — the exact struct
    /// `zenctl storage list --format json` serializes, so the GUI table and
    /// the CLI cannot disagree about what covers what.
    pub storage: StorageList,
    /// `None` = **nothing answered the admin space at all**. Zenoh's
    /// `adminspace.enabled` defaults to false and a peer mesh has none, so
    /// this is "not available" and never "nothing declared" (RFC 09 §5.1 O4).
    pub declared: Option<DeclaredEntities>,
    /// Why the coverage join was not attempted, when it was not — the GUI form
    /// of the `note:` zenctl prints to stderr. An absent table and an empty
    /// one are different facts, and the pane renders them differently.
    pub coverage_note: Option<String>,
    /// The mesh as the admin space answered it (#118) — nodes, edges, and
    /// who only got mentioned. Same struct `zenctl admin graph` renders.
    pub topology: zenkey_fleet::TopologyReport,
    /// The base the coverage was judged against — the staleness guard.
    pub base: String,
}

/// The panel's whole state.
#[derive(Debug, Default)]
pub struct AdminState {
    pub in_flight: bool,
    pub sweep: Option<Arc<AdminSweep>>,
    pub error: Option<String>,
    /// Sweeps completed this session. A sweep that ran and found nothing is an
    /// observation; never-run is not — this is what tells them apart.
    pub runs: usize,
    /// Rows whose raw admin document is expanded. Admin layouts vary by
    /// version, so `raw` is the honest fallback and the pane can show it.
    pub expanded_raw: BTreeSet<String>,
}

impl AdminState {
    /// A sweep landed.
    ///
    /// A sweep judged against another base is evidence about a deployment this
    /// window is no longer looking at, so it is dropped rather than shown. A
    /// failure keeps the last good sweep: deltas against nothing would be
    /// worse than a stale table with an error beside it.
    pub fn finish(&mut self, outcome: Result<Arc<AdminSweep>, String>, base: &str) {
        self.in_flight = false;
        match outcome {
            Ok(sweep) if sweep.base == base => {
                self.runs += 1;
                self.sweep = Some(sweep);
                self.error = None;
            }
            Ok(_) => {
                // Silently dropped, deliberately: it is not an error, and
                // reporting it would explain a deployment the user has left.
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Base change / reconnect / context switch.
    pub fn clear(&mut self) {
        *self = AdminState::default();
    }

    /// Toggle one row's raw-document disclosure.
    pub fn toggle_raw(&mut self, row: String) {
        if !self.expanded_raw.remove(&row) {
            self.expanded_raw.insert(row);
        }
    }
}

/// Stable row identities, spelled exactly as `zenctl … --watch` spells them
/// (`cmd/watch.rs`), so both explorers share one vocabulary for "which row is
/// this".
pub fn storage_row_id(name: &str, zid: &str) -> String {
    format!("storage:{name}@{zid}")
}

pub fn router_row_id(zid: &str) -> String {
    format!("router:{zid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sweep(base: &str) -> Arc<AdminSweep> {
        Arc::new(AdminSweep {
            routers: vec![],
            storage: StorageList {
                storages: vec![],
                coverage: vec![],
            },
            declared: None,
            coverage_note: None,
            topology: zenkey_fleet::TopologyReport {
                nodes: vec![],
                edges: vec![],
                asked: "@/*/*".into(),
                answered: 0,
                self_zid: String::new(),
            },
            base: base.to_string(),
        })
    }

    #[test]
    fn a_sweep_from_another_base_is_not_evidence_here() {
        let mut state = AdminState::default();
        state.finish(Ok(sweep("acme")), "acme");
        assert_eq!(state.runs, 1);

        // The user switched deployment while the sweep was in flight.
        state.finish(Ok(sweep("acme")), "other");
        assert_eq!(state.runs, 1, "a sweep about acme says nothing about other");
        assert!(state.error.is_none(), "and it is not an error either");
    }

    #[test]
    fn a_failed_sweep_keeps_the_last_good_one() {
        let mut state = AdminState::default();
        state.finish(Ok(sweep("acme")), "acme");
        state.finish(Err("admin space unreachable".into()), "acme");
        assert!(state.sweep.is_some(), "the baseline survives a failure");
        assert!(state.error.is_some());
        assert_eq!(state.runs, 1);
    }

    #[test]
    fn clearing_returns_to_never_run() {
        let mut state = AdminState::default();
        state.finish(Ok(sweep("acme")), "acme");
        state.toggle_raw(storage_row_id("latest", "z1"));
        state.clear();
        assert!(state.sweep.is_none());
        assert_eq!(state.runs, 0, "never-run, not run-and-found-nothing");
        assert!(state.expanded_raw.is_empty());
    }

    #[test]
    fn raw_disclosure_toggles() {
        let mut state = AdminState::default();
        let id = router_row_id("z1");
        state.toggle_raw(id.clone());
        assert!(state.expanded_raw.contains(&id));
        state.toggle_raw(id.clone());
        assert!(!state.expanded_raw.contains(&id));
    }
}
