//! The blob browser's model (#68): the RFC 07 §2.5 sequence as state.
//!
//! Probe wide, choose one holder, fetch from that holder — and the type here
//! is shaped so the middle step cannot be skipped. There is no origin *field*:
//! the only origin a fetch can name is `holders[selected].origin`, which came
//! off a reply key. A wildcard fetch is not refused by a check in this module;
//! it has nowhere to be typed.
//!
//! Pure logic, no widgets, so the pane renders standalone in a test.

use std::sync::Arc;

use zenkey_fleet::BlobTarget;
use zenkey_fleet::report::{BlobFetchReport, BlobList, BlobProbeReport};

/// A probe's state. `NotAsked` is a distinct variant on purpose: "never
/// probed" and "probed, nobody answered" are different facts and must not
/// render alike (RFC 09 §5.1 O4).
#[derive(Debug, Default)]
pub enum Probe {
    #[default]
    NotAsked,
    InFlight,
    Done(Arc<BlobProbeReport>),
    Failed(String),
}

/// A fetch's state, same discipline.
#[derive(Debug, Default)]
pub enum Fetch {
    #[default]
    NotAsked,
    InFlight {
        received: u32,
        total: u32,
        bytes: u64,
    },
    Done(Arc<BlobFetchReport>),
    /// A `tree/<root>` target: inspected, not downloaded (RFC 07 §2.3,
    /// v1.17) — the validated index summary, no content store involved.
    Inspected(Arc<zenkey_fleet::report::BlobTreeIndexReport>),
    Failed(String),
}

/// The pane's whole state.
///
/// Not `Debug`-derived: the reference client's cancel token is not `Debug`, and
/// a hand-written impl to print `Some(..)` for it would buy nothing.
#[derive(Default)]
pub struct BlobState {
    /// The registry projection. Costs no traffic — it is what the slices the
    /// app already loaded happen to say — so it is not gated behind a button.
    /// The laziness rule (#84/#85) is about *fetching*, not about rendering
    /// what is already in hand.
    pub list: Option<BlobList>,
    pub target_input: String,
    pub root_input: String,
    pub dest_input: String,
    /// The parse verdict for `target_input`, recomputed per keystroke. Shown,
    /// never auto-corrected: an id we silently rewrote would be an id the user
    /// did not ask about.
    pub target: Option<Result<BlobTarget, String>>,
    pub probe: Probe,
    /// Index into the probe's holders — **the only way an origin is chosen**.
    pub holder: Option<usize>,
    /// Trust-on-first-use, asked for out loud (RFC 07 §2.1).
    pub allow_unpinned: bool,
    pub fetch: Fetch,
    /// Live while a fetch runs, so the stop button is not a lie.
    pub cancel: Option<zenkey_fleet::zblob::CancelToken>,
}

impl BlobState {
    /// Re-parse the target field. Called on every edit.
    pub fn set_target(&mut self, input: String) {
        self.target = if input.trim().is_empty() {
            None
        } else {
            Some(BlobTarget::parse(&input).map_err(|e| e.to_string()))
        };
        self.target_input = input;
        // A new target invalidates the old answers: holders of *that* id say
        // nothing about this one.
        self.probe = Probe::NotAsked;
        self.holder = None;
        self.fetch = Fetch::NotAsked;
    }

    /// The chosen holder, if the probe produced one and it was selected.
    pub fn selected(&self) -> Option<&zenkey_fleet::report::BlobHolder> {
        let Probe::Done(report) = &self.probe else {
            return None;
        };
        self.holder.and_then(|i| report.holders.get(i))
    }

    /// Distinct content roots the probe saw. More than one is a finding
    /// (RFC 07 §2.1), and the pane says so rather than picking.
    pub fn roots(&self) -> &[String] {
        match &self.probe {
            Probe::Done(r) => &r.roots,
            _ => &[],
        }
    }

    /// Why a fetch cannot start yet, or `Ok(())`.
    ///
    /// Every reason is a sentence the pane shows: a disabled button that does
    /// not say why is a worse answer than an error.
    pub fn fetch_ready(&self) -> Result<(), &'static str> {
        match &self.target {
            None => return Err("type an artifact id or a tier-2 address first"),
            Some(Err(_)) => return Err("the target does not parse"),
            Some(Ok(_)) => {}
        }
        if self.selected().is_none() {
            return Err("choose one holder — a fetch names exactly one origin (RFC 07 §2.5)");
        }
        // A tree target is inspected, not downloaded (RFC 07 §2.3, v1.17):
        // the summary writes nothing, so it needs no destination.
        let tree = matches!(self.target, Some(Ok(BlobTarget::Tree { .. })));
        if !tree && self.dest_input.trim().is_empty() {
            return Err("choose where to write it");
        }
        if matches!(self.fetch, Fetch::InFlight { .. }) {
            return Err("a fetch is already running");
        }
        // RFC 07 §2.1: an anchor, or an explicit decision to go without one.
        let tier_one = matches!(self.target, Some(Ok(BlobTarget::Artifact { .. })));
        if tier_one && self.root_input.trim().is_empty() && !self.allow_unpinned {
            return Err(
                "pin a content root, or tick 'accept unpinned' — RFC 07 §2.1 wants the \
                 reference to carry the identity of the bytes it names",
            );
        }
        Ok(())
    }

    /// A probe finished. Judged against the base it *ran* against (#109):
    /// a probe from another base is dropped whole — Ok and Err alike, unlike
    /// doctor/admin, because here the base rides beside the `Result` and a
    /// stale error would misexplain the deployment the window now shows.
    /// `clear()` already reset the pane; there is nothing true to add.
    pub fn probe_finished(
        &mut self,
        outcome: Result<Arc<BlobProbeReport>, String>,
        ran_against: &str,
        base: &str,
    ) {
        if ran_against != base {
            return;
        }
        self.probe = match outcome {
            Ok(report) => Probe::Done(report),
            Err(e) => Probe::Failed(e),
        };
    }

    /// A fetch finished, same judgment — with one more reason to drop a
    /// mismatch whole: a base change *cancels* the fetch via [`Self::clear`],
    /// so its `Err("cancelled")` completion is a direct product of the
    /// switch. Landing it would make every base change manufacture an error,
    /// and it would also null the cancel token out from under a fetch the
    /// user started on the new base.
    pub fn fetch_finished(
        &mut self,
        outcome: Result<Arc<BlobFetchReport>, String>,
        ran_against: &str,
        base: &str,
    ) {
        if ran_against != base {
            return;
        }
        self.cancel = None;
        self.fetch = match outcome {
            Ok(report) => Fetch::Done(report),
            Err(e) => Fetch::Failed(e),
        };
    }

    /// A tree inspection finished — [`Self::fetch_finished`]'s judgment, for
    /// the summary a `tree/<root>` target produces instead of a file.
    pub fn inspect_finished(
        &mut self,
        outcome: Result<Arc<zenkey_fleet::report::BlobTreeIndexReport>, String>,
        ran_against: &str,
        base: &str,
    ) {
        if ran_against != base {
            return;
        }
        self.cancel = None;
        self.fetch = match outcome {
            Ok(report) => Fetch::Inspected(report),
            Err(e) => Fetch::Failed(e),
        };
    }

    /// Base change / reconnect / context switch: answers about another
    /// deployment are not evidence here.
    pub fn clear(&mut self) {
        if let Some(c) = self.cancel.take() {
            c.cancel();
        }
        *self = BlobState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey_fleet::report::{BlobHolder, BlobManifest};

    fn holder(origin: &str, root: &str) -> BlobHolder {
        BlobHolder {
            origin: origin.into(),
            key: format!("v1/{origin}/@blob/artifact/01jqz3demo0001"),
            availability: None,
            note: None,
            manifest: Some(BlobManifest {
                id: "01jqz3demo0001".into(),
                filename: None,
                total_len: 10,
                chunk_size: 65536,
                chunk_count: 1,
                root: root.into(),
                created_ms: 0,
            }),
            unreadable: None,
            error: None,
        }
    }

    fn probed(state: &mut BlobState, holders: Vec<BlobHolder>, roots: Vec<String>) {
        state.probe = Probe::Done(Arc::new(BlobProbeReport {
            target: "artifact/01jqz3demo0001".into(),
            tier: "artifact".into(),
            asked: vec!["v1/*/@blob/artifact/01jqz3demo0001/have".into()],
            not_probed: None,
            answered: holders.len(),
            holders,
            roots,
            declared_by: vec![],
        }));
    }

    fn probe_report(holders: Vec<BlobHolder>) -> Arc<BlobProbeReport> {
        Arc::new(BlobProbeReport {
            target: "artifact/01jqz3demo0001".into(),
            tier: "artifact".into(),
            asked: vec!["v1/*/@blob/artifact/01jqz3demo0001/have".into()],
            not_probed: None,
            answered: holders.len(),
            holders,
            roots: vec![],
            declared_by: vec![],
        })
    }

    /// #109: the user switched deployment while a probe was in flight. Its
    /// holders are holders on a bus this window no longer shows — dropped
    /// whole, Ok and Err alike, and no error appears (clear() already reset
    /// the pane; a stale error would misexplain the new deployment).
    #[test]
    fn a_probe_from_another_base_is_not_evidence_here() {
        let mut state = BlobState::default();
        state.probe_finished(Ok(probe_report(vec![])), "acme", "acme");
        assert!(matches!(state.probe, Probe::Done(_)), "matched base lands");

        state.clear();
        state.probe_finished(
            Ok(probe_report(vec![holder("h-aaaaaaaaaaaa", "ab")])),
            "acme",
            "other",
        );
        assert!(
            matches!(state.probe, Probe::NotAsked),
            "holders on acme say nothing about other"
        );
        state.probe_finished(Err("timeout".into()), "acme", "other");
        assert!(
            matches!(state.probe, Probe::NotAsked),
            "a stale error is not an error about this deployment either"
        );
    }

    /// #109: a base change cancels the running fetch, so its cancellation
    /// lands *after* the clear — and must touch neither the fresh fetch
    /// state nor the fresh cancel token.
    #[test]
    fn a_stale_fetch_completion_does_not_touch_the_new_fetch() {
        // The user has already started a new fetch on the new base.
        let mut state = BlobState {
            fetch: Fetch::InFlight {
                received: 1,
                total: 4,
                bytes: 65536,
            },
            cancel: Some(zenkey_fleet::zblob::CancelToken::new()),
            ..BlobState::default()
        };

        state.fetch_finished(Err("cancelled".into()), "old", "new");
        assert!(
            matches!(state.fetch, Fetch::InFlight { .. }),
            "the old base's cancellation must not overwrite the new fetch"
        );
        assert!(
            state.cancel.is_some(),
            "and must not null the new fetch's cancel token"
        );

        // The new base's own completion still lands and clears the token.
        state.fetch_finished(Err("io: disk full".into()), "new", "new");
        assert!(matches!(state.fetch, Fetch::Failed(_)));
        assert!(state.cancel.is_none());
    }

    #[test]
    fn a_new_target_forgets_the_old_answers() {
        let mut state = BlobState::default();
        state.set_target("01jqz3demo0001".into());
        probed(
            &mut state,
            vec![holder("h-aaaaaaaaaaaa", "ab")],
            vec!["ab".into()],
        );
        state.holder = Some(0);

        state.set_target("01jqz3demo0002".into());
        assert!(matches!(state.probe, Probe::NotAsked));
        assert!(
            state.holder.is_none(),
            "holders of another id say nothing here"
        );
    }

    #[test]
    fn an_uppercase_id_is_reported_not_corrected() {
        let mut state = BlobState::default();
        state.set_target("01HQXK8F9C2N4PZQ".into());
        let err = state.target.as_ref().unwrap().as_ref().unwrap_err();
        assert!(err.contains("lowercase"), "{err}");
        // The field still holds what was typed: the pane shows a verdict, it
        // does not rewrite the user's input.
        assert_eq!(state.target_input, "01HQXK8F9C2N4PZQ");
    }

    #[test]
    fn a_fetch_needs_one_chosen_holder_and_an_anchor() {
        let mut state = BlobState::default();
        assert!(state.fetch_ready().is_err(), "nothing typed yet");

        state.set_target("01jqz3demo0001".into());
        state.dest_input = "/tmp/demo.bin".into();
        assert!(
            state.fetch_ready().unwrap_err().contains("one holder"),
            "a probe with no selection cannot fetch"
        );

        probed(
            &mut state,
            vec![
                holder("h-aaaaaaaaaaaa", "ab"),
                holder("h-bbbbbbbbbbbb", "ab"),
            ],
            vec!["ab".into()],
        );
        state.holder = Some(1);
        assert!(
            state.fetch_ready().unwrap_err().contains("2.1"),
            "tier-1 needs a pinned root or an explicit decision to go without"
        );

        state.allow_unpinned = true;
        assert!(state.fetch_ready().is_ok());
        assert_eq!(state.selected().unwrap().origin, "h-bbbbbbbbbbbb");
    }

    #[test]
    fn tier_two_needs_no_anchor_because_the_key_is_one() {
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut state = BlobState::default();
        state.set_target(format!("store/blake3/{hash}"));
        state.dest_input = "/tmp/chunk".into();
        probed(&mut state, vec![holder("h-aaaaaaaaaaaa", hash)], vec![]);
        state.holder = Some(0);
        assert!(
            state.fetch_ready().is_ok(),
            "no --root needed: the key is the root"
        );
    }

    #[test]
    fn disagreeing_roots_are_surfaced_not_resolved() {
        let mut state = BlobState::default();
        state.set_target("01jqz3demo0001".into());
        probed(
            &mut state,
            vec![
                holder("h-aaaaaaaaaaaa", "ab"),
                holder("h-cccccccccccc", "cd"),
            ],
            vec!["ab".into(), "cd".into()],
        );
        assert_eq!(
            state.roots().len(),
            2,
            "the pane reports both, and picks neither"
        );
    }
}
