//! zk2's `why` (#702): a key's or a service's silence, explained rung by
//! rung, in the judgement shape.
//!
//! **Polarity** (`docs/zk2/tooling-guide.md` §1). Every rung asks "is the
//! silence caused here?", so a rung's finding is the *yes*: `Established`,
//! with the cause. A healthy rung is `NotEstablished`, with the evidence
//! that makes it healthy as its reason; a rung whose observation could not
//! be had is `Unobservable`, with what stood in the way; and a rung the
//! ladder never reached is `NotAsked`. The four stay apart in every medium.
//!
//! The report's own [`WhyReport::verdict`] reads the same way: a cause found
//! is the finding (exit 1, "a cause is the finding"), every rung healthy
//! and the key answering is clean (exit 0), and a rung left unobservable is
//! no verdict (exit 2). The ladder stops at the first rung that establishes
//! a cause, or that cannot be observed — save the owner's silence, after
//! which S6 sends a reader to an archive.

use std::fmt;

use serde::Serialize;

use super::judgement::Judgement;
use super::payload::PayloadRendering;
use super::state::Stamp;

/// The rungs of the ladder, in order. **Stable API**: scripts branch on
/// these through `--format json`; new rungs append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RungId {
    /// The wire key sits under the deployment's namespace (§1.6).
    Namespace,
    /// It is a zk2 key of one of the five forms (§1.1).
    Key,
    /// The address holds its tokens, visibly to this reader (§8.1).
    Presence,
    /// Its descriptor is served, implements the interface, and exposes the
    /// resource, or says why not (§3.3).
    Descriptor,
    /// The revision the descriptor names is retrievable and verifies, and
    /// declares the resource (§8.4).
    Contract,
    /// The key answers: the owner's state GET (S4), a stream's sample in
    /// the window, a union storage's occurrence (§2.6).
    Answer,
    /// An archive's last-known value, read only after the owner's silence
    /// (S6): last-known, never current.
    LastKnown,
}

impl RungId {
    /// Every rung, in ladder order.
    pub const ALL: [RungId; 7] = [
        RungId::Namespace,
        RungId::Key,
        RungId::Presence,
        RungId::Descriptor,
        RungId::Contract,
        RungId::Answer,
        RungId::LastKnown,
    ];

    /// The wire token, exactly as it serializes.
    pub fn as_str(self) -> &'static str {
        match self {
            RungId::Namespace => "namespace",
            RungId::Key => "key",
            RungId::Presence => "presence",
            RungId::Descriptor => "descriptor",
            RungId::Contract => "contract",
            RungId::Answer => "answer",
            RungId::LastKnown => "last-known",
        }
    }

    /// The rung's question, worded so that its finding is the **yes**.
    pub fn question(self) -> &'static str {
        match self {
            RungId::Namespace => "is the key outside this deployment's namespace?",
            RungId::Key => "is the key not a zk2 key, or not one this ladder reads?",
            RungId::Presence => "is no token of its service visible to this reader?",
            RungId::Descriptor => {
                "does its descriptor fail, not implement the interface, or leave the resource \
                 unexposed?"
            }
            RungId::Contract => {
                "is the revision unavailable or unreadable, or does it declare no such resource?"
            }
            RungId::Answer => "does the key answer with a deletion, rather than a value?",
            RungId::LastKnown => "does no archive hold a last-known value?",
        }
    }
}

impl fmt::Display for RungId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What `why` was asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhySubject {
    /// A wire key: the whole ladder.
    Key,
    /// A service address, or a control key naming one: the rungs up to its
    /// contracts, its descriptor's answer the service's.
    Service,
}

/// One rung's answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhyRung {
    pub rung: RungId,
    /// The core section the rung reads (`spec/core.md`).
    pub section: &'static str,
    /// The rung's question answered: the cause is the yes.
    pub verdict: Judgement,
    /// The cause, when the rung established one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

impl WhyRung {
    /// A rung the ladder never reached.
    pub fn not_asked(rung: RungId, section: &'static str) -> WhyRung {
        WhyRung {
            rung,
            section,
            verdict: Judgement::NotAsked,
            cause: None,
        }
    }

    /// A healthy rung, with its evidence.
    pub fn healthy(rung: RungId, section: &'static str, evidence: impl Into<String>) -> WhyRung {
        WhyRung {
            rung,
            section,
            verdict: Judgement::NotEstablished {
                reason: evidence.into(),
            },
            cause: None,
        }
    }

    /// A rung that established a cause.
    pub fn cause(rung: RungId, section: &'static str, cause: impl Into<String>) -> WhyRung {
        WhyRung {
            rung,
            section,
            verdict: Judgement::Established,
            cause: Some(cause.into()),
        }
    }

    /// A rung whose observation could not be had.
    pub fn unobservable(rung: RungId, section: &'static str, why: impl Into<String>) -> WhyRung {
        WhyRung {
            rung,
            section,
            verdict: Judgement::Unobservable { reason: why.into() },
            cause: None,
        }
    }
}

/// What an archive still holds, read after the owner's silence (S6):
/// last-known, never current.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhyLastKnown {
    /// The archive that answered, `<system>/<service>`.
    pub archive: String,
    /// The origin key, base-relative.
    pub key: String,
    /// The value, rendered through the contract; absent for a deletion the
    /// archive holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Box<PayloadRendering>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Stamp>,
    /// Whether the archive's alignment confirmed it (§4.4).
    pub confirmed: bool,
}

/// `zenctl why` (#702): the ladder, and what it established.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WhyReport {
    /// The key or the address, as given.
    pub target: String,
    pub subject: WhySubject,
    /// The deployment's namespace; empty for the bus root.
    pub namespace: String,
    /// The selectors put to the bus, base-relative in the namespace: what
    /// the ladder read, and no wider (tooling guide O5). Empty when it
    /// stopped before asking anything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asked: Vec<String>,
    /// How long a stream key was listened to, seconds; absent when no
    /// window was opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_s: Option<f64>,
    /// Every rung, in ladder order: those past the stop are `not_asked`.
    pub rungs: Vec<WhyRung>,
    /// The ladder's verdict: a cause is the finding (`established`); every
    /// rung healthy and the key answering is clean; a rung left
    /// unobservable — or an answer this ladder does not ask for — is no
    /// verdict.
    pub verdict: Judgement,
    /// The rung that stopped the ladder, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<RungId>,
    /// The key's answer, rendered through its contract, when it gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Box<PayloadRendering>>,
    /// An archive's last-known value, when the owner was silent (S6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_known: Option<WhyLastKnown>,
}

impl WhyReport {
    /// One rung's answer.
    pub fn rung(&self, id: RungId) -> Option<&WhyRung> {
        self.rungs.iter().find(|r| r.rung == id)
    }

    /// The cause the ladder established, with its rung.
    pub fn cause(&self) -> Option<(RungId, &str)> {
        self.rungs
            .iter()
            .find_map(|r| r.cause.as_deref().map(|c| (r.rung, c)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    /// The rung vocabulary is API: additions append, nothing renames.
    #[test]
    fn rung_ids_are_stable_and_ask_for_the_yes() {
        assert_eq!(
            RungId::ALL.map(RungId::as_str),
            [
                "namespace",
                "key",
                "presence",
                "descriptor",
                "contract",
                "answer",
                "last-known"
            ]
        );
        for r in RungId::ALL {
            assert_eq!(
                serde_json::to_value(r).unwrap(),
                json!(r.as_str()),
                "as_str and serde are one vocabulary"
            );
            assert!(r.question().ends_with('?'), "{r}");
        }
    }

    /// The serialized report is a wire contract: a rung's four poles each
    /// with their own `answer`, the cause only on the established one, and
    /// what was not asked absent rather than empty.
    #[test]
    fn the_why_report_pins_its_shape() {
        let report = WhyReport {
            target: "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0".into(),
            subject: WhySubject::Key,
            namespace: "acme".into(),
            asked: vec!["zk2/host-a/tc/@zk/**".into()],
            window_s: None,
            rungs: vec![
                WhyRung::healthy(RungId::Namespace, "§1.6", "under acme"),
                WhyRung::cause(RungId::Presence, "§8.1", "no token visible to this reader"),
                WhyRung::unobservable(RungId::Answer, "§4.2 S4", "silent"),
                WhyRung::not_asked(RungId::LastKnown, "§4.2 S6"),
            ],
            verdict: Judgement::Established,
            stopped_at: Some(RungId::Presence),
            value: None,
            last_known: None,
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "target": "acme/zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
                "subject": "key",
                "namespace": "acme",
                "asked": ["zk2/host-a/tc/@zk/**"],
                "rungs": [
                    {"rung": "namespace", "section": "§1.6",
                     "verdict": {"answer": "not_established", "reason": "under acme"}},
                    {"rung": "presence", "section": "§8.1",
                     "verdict": {"answer": "established"},
                     "cause": "no token visible to this reader"},
                    {"rung": "answer", "section": "§4.2 S4",
                     "verdict": {"answer": "unobservable", "reason": "silent"}},
                    {"rung": "last-known", "section": "§4.2 S6",
                     "verdict": {"answer": "not_asked"}},
                ],
                "verdict": {"answer": "established"},
                "stopped_at": "presence",
            })
        );
        assert_eq!(
            report.cause(),
            Some((RungId::Presence, "no token visible to this reader"))
        );
        assert_eq!(
            judgement_exit_code(&report.verdict),
            1,
            "a cause is the finding"
        );
        let last = WhyLastKnown {
            archive: "ground/archive".into(),
            key: "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0".into(),
            value: None,
            timestamp: None,
            confirmed: false,
        };
        assert_eq!(
            serde_json::to_value(&last).unwrap(),
            json!({
                "archive": "ground/archive",
                "key": "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0",
                "confirmed": false,
            }),
            "a deletion held: no value, never a null one"
        );
    }
}
