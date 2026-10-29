//! The admin & storage panel's model (#70; zk2's since #614, FL9): one
//! sweep of the Zenoh admin space, assembled off the UI thread, plus the
//! staleness guard that keeps a sweep about one deployment from being
//! displayed as evidence about another.
//!
//! Pure logic, no widgets — the pane renders standalone in a test.
//!
//! **No admin selector is spelled here.** Every one lives inside
//! `zenkey_fleet::bus::admin`; this module holds what came back. That is the
//! right posture for a plane whose shape is the middleware's rather than
//! this convention's.
//!
//! **S4 inverts v1's table** (D14). v1 joined declared state families to the
//! storages that would seed them and read "covered" as good: the `latest`
//! storage was v1's late-joiner seed. zk2's owner is authoritative
//! for its state (core §4.2 S4): a late joiner seeds from the owner, and
//! last-known state is an `archive.v1`'s (§4.4) — so a router storage on an
//! owner's state keys is the finding, and each storage is judged against
//! `state/**` and `@state/**` of every owner in the namespace
//! ([`zenkey_fleet::on_owner_state`], the doctor's own test).

use std::collections::BTreeSet;
use std::sync::Arc;

use zenkey_fleet::report::{RouterInfo, StorageInfo, TopologyReport};

/// One storage judged against S4: does it answer on an owner's state?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnState {
    /// It does: the owner-state selectors its key expression intersects.
    Finding(Vec<String>),
    /// It does not: no owner's state in this namespace is under it.
    Clean,
    /// It could not be judged: its admin document names no key expression,
    /// or one that does not parse. Never clean.
    Unjudged(String),
}

/// One storage's S4 row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4Row {
    /// `<name>@<zid>`, as the doctor's `storage-on-state` names it.
    pub storage: String,
    pub key_expr: Option<String>,
    pub on_state: OnState,
}

/// Every storage judged against S4 in `namespace` — the doctor's
/// `storage-on-state`, storage by storage, from the sweep in hand.
pub fn s4_rows(namespace: &str, storages: &[StorageInfo]) -> Vec<S4Row> {
    storages
        .iter()
        .map(|s| {
            let on_state = match &s.key_expr {
                None => OnState::Unjudged("its admin document names no key_expr".to_string()),
                Some(ke) if zenoh::key_expr::keyexpr::new(ke.as_str()).is_err() => {
                    OnState::Unjudged(format!("{ke:?} is not a key expression"))
                }
                Some(ke) => match zenkey_fleet::on_owner_state(namespace, ke) {
                    hits if hits.is_empty() => OnState::Clean,
                    hits => OnState::Finding(hits),
                },
            };
            S4Row {
                storage: format!("{}@{}", s.name, s.zid),
                key_expr: s.key_expr.clone(),
                on_state,
            }
        })
        .collect()
}

/// One sweep's worth of admin space.
#[derive(Debug)]
pub struct AdminSweep {
    pub routers: Vec<RouterInfo>,
    /// The storages, as `zenctl storage list --format json` lists them.
    pub storages: Vec<StorageInfo>,
    /// Each storage judged against S4 in the namespace the sweep ran in.
    pub s4: Vec<S4Row>,
    /// The mesh as the admin space answered it (#118) — nodes, edges, who
    /// only got mentioned — and zk2's instances joined onto the routers by
    /// the session zid their descriptors state (#705): the struct `zenctl
    /// admin graph` renders. The join is "not asked" without a session in
    /// the namespace.
    pub topology: TopologyReport,
    /// The namespace the S4 rows and the instance join were judged in — the
    /// staleness guard.
    pub namespace: String,
}

/// The panel's whole state.
#[derive(Debug, Default)]
pub struct AdminState {
    pub in_flight: bool,
    pub sweep: Option<Arc<AdminSweep>>,
    pub error: Option<crate::services::ServiceError>,
    /// Sweeps completed this session. A sweep that ran and found nothing is an
    /// observation; never-run is not — this is what tells them apart.
    pub runs: usize,
    /// Rows whose raw admin document is expanded. Admin layouts vary by
    /// version, so `raw` is the honest fallback and the pane can show it.
    pub expanded_raw: BTreeSet<String>,
    /// The topology drawing's retained geometry (#178).
    ///
    /// Here rather than on [`AdminSweep`], which crosses a task boundary in an
    /// `Arc` and therefore has to be `Send`: `canvas::Cache` is a `RefCell`
    /// and is not. It is cleared when a sweep lands (below) and when the pan
    /// or zoom moves (`view::mesh`, which is the only thing that can see the
    /// viewport); a namespace change replaces the whole state.
    pub mesh_cache: iced::widget::canvas::Cache,
}

impl AdminState {
    /// A sweep landed.
    ///
    /// A sweep judged in another namespace is evidence about a deployment
    /// this window is no longer looking at, so it is dropped rather than
    /// shown. A failure keeps the last good sweep: a stale table with an
    /// error beside it is better than none.
    pub fn finish(
        &mut self,
        outcome: Result<Arc<AdminSweep>, crate::services::ServiceError>,
        namespace: &str,
    ) {
        self.in_flight = false;
        match outcome {
            Ok(sweep) if sweep.namespace == namespace => {
                self.runs += 1;
                self.sweep = Some(sweep);
                self.error = None;
                // New nodes, new edges: the retained drawing is of the old
                // mesh (#178).
                self.mesh_cache.clear();
            }
            Ok(_) => {
                // Silently dropped, deliberately: it is not an error, and
                // reporting it would explain a deployment the user has left.
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Namespace change / reconnect / context switch.
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

    fn sweep(namespace: &str) -> Arc<AdminSweep> {
        Arc::new(AdminSweep {
            routers: vec![],
            storages: vec![],
            s4: vec![],
            topology: TopologyReport {
                nodes: vec![],
                edges: vec![],
                asked: "@/*/*".into(),
                answered: 0,
                self_zid: String::new(),
                instances: zenkey_fleet::report::Asked::NotAsked,
            },
            namespace: namespace.to_string(),
        })
    }

    fn storage(name: &str, key_expr: Option<&str>) -> StorageInfo {
        StorageInfo {
            zid: "z1".into(),
            name: name.into(),
            key_expr: key_expr.map(Into::into),
            strip_prefix: None,
            volume: Some("rocksdb".into()),
            raw: serde_json::json!({}),
        }
    }

    /// S4, storage by storage (D14): a storage on an owner's state is the
    /// finding — `**` reaches `state/**` but never `@state/**`, which only
    /// a selector naming it reaches — a storage beside the state is clean,
    /// and one whose document names no key expression is not judged,
    /// never clean.
    #[test]
    fn a_storage_on_owner_state_is_the_finding() {
        let rows = s4_rows(
            "acme",
            &[
                storage("everything", Some("**")),
                storage("explicit", Some("acme/zk2/*/*/*/@state/**")),
                storage("events", Some("acme/zk2/*/*/*/events/**")),
                storage("other", Some("elsewhere/zk2/*/*/*/state/**")),
                storage("silent", None),
            ],
        );
        assert_eq!(
            rows[0].on_state,
            OnState::Finding(vec!["acme/zk2/*/*/*/state/**".into()])
        );
        assert_eq!(
            rows[1].on_state,
            OnState::Finding(vec!["acme/zk2/*/*/*/@state/**".into()])
        );
        assert_eq!(rows[2].on_state, OnState::Clean, "an event log is no state");
        assert_eq!(
            rows[3].on_state,
            OnState::Clean,
            "another namespace's state is not this one's"
        );
        assert!(matches!(rows[4].on_state, OnState::Unjudged(_)));
        assert_eq!(rows[0].storage, "everything@z1");
    }

    #[test]
    fn a_sweep_from_another_namespace_is_not_evidence_here() {
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
