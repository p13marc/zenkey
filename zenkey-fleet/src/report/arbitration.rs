//! `arbitration.v1` (#909, BC): what a tool reads of a consumer's
//! arbitrated role (`spec/profiles/arbitration/v1.md` §2.14, §5).
//!
//! **The profile's own tokens.** Each of §5's three questions answers in
//! the tokens `conclusions.json` states — `yes`, `no`, `unobservable`,
//! `not_asked`, `not_this_profiles`, each with its reason — so a script
//! reads `dead_man` where the text says dead-man:
//! - "is the role's declaration valid?" — `declaration`, no the finding,
//!   with the invalid reasons in §2.2's order;
//! - "which provider does the tool's view select?" — `selection`, no the
//!   finding (`dead_man`; `unbound` is informative), **always the tool's
//!   own view** ([`ArbitrationSelection::whose`]), never the consumer's;
//! - "does the binding leave a priority to an order nobody chose?" —
//!   `binding`, yes the finding (a warning), naming the wildcard and the
//!   shadowed entries.
//!
//! Each is asked of one consumer's role ([`ArbitrationRole`]), and
//! [`ArbitrationReport::judgement`] reads every role's three by their
//! findings into the one [`Judgement`] a frontend exits through. Computed by
//! `crate::judge::arbitration`, pinned here.

use std::collections::BTreeMap;

use serde::Serialize;

use super::judgement::Judgement;

/// What every selection says it is: the tool's own view (§2.14), which a
/// script can read without knowing the rule.
pub const TOOLS_OWN_VIEW: &str = "the tool's own view";

/// One of §5's answers, as `conclusions.json` spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArbitrationAnswerToken {
    Yes,
    No,
    Unobservable,
    NotAsked,
    NotThisProfiles,
}

impl ArbitrationAnswerToken {
    /// The token, as serialized.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unobservable => "unobservable",
            Self::NotAsked => "not_asked",
            Self::NotThisProfiles => "not_this_profiles",
        }
    }

    /// Whether the question was put and answered something, a finding or
    /// not: yes, no or unobservable. Not asked and not this profile's are
    /// set aside.
    pub fn asked(self) -> bool {
        matches!(self, Self::Yes | Self::No | Self::Unobservable)
    }
}

/// §5's first answer: is the role's declaration valid?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArbitrationDeclaration {
    pub answer: ArbitrationAnswerToken,
    /// `valid`, `invalid`, `no_contract`, `not_arbitrated`,
    /// `revisions_disagree` or `no_provider_contract`.
    pub reason: String,
    /// The answer in words.
    pub says: String,
    /// An invalid declaration's reasons, in §2.2's order: the fixture's
    /// tokens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    /// A valid one's policy (`priority`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    /// A valid one's stream, `<token>/<template>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// A valid one's deadline, from the revisions held (§2.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u64>,
    /// Whether this answer is the question's finding: an invalid
    /// declaration, which its consumer refuses to start on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finding: bool,
}

/// §5's second answer: which provider does the tool's view select?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArbitrationSelection {
    pub answer: ArbitrationAnswerToken,
    /// `selected`, `dead_man`, `unbound`, `listening`, `lost`, `invalid`,
    /// `not_listened`, `no_descriptor`, or a premise's.
    pub reason: String,
    pub says: String,
    /// The provider the view selects, when it selects one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Always [`TOOLS_OWN_VIEW`]: a consumer's switches are its own to
    /// tell (§2.10, §2.14).
    pub whose: &'static str,
    /// The view in the dead-man after listening one deadline, where the
    /// deployment expects a commander.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finding: bool,
}

/// §5's third answer: does the binding leave a priority to an order nobody
/// chose?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArbitrationBinding {
    pub answer: ArbitrationAnswerToken,
    /// `wildcard`, `shadowed`, `exact`, `no_descriptor`, or a premise's.
    pub reason: String,
    pub says: String,
    /// The wildcard entries' 0-based positions, as `conclusions.json`.
    pub wildcards: Vec<usize>,
    /// The shadowed entries' positions.
    pub shadowed: Vec<usize>,
    /// The wildcard entries, as the descriptor lists them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wildcard_entries: Vec<String>,
    /// The shadowed entries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shadowed_entries: Vec<String>,
    /// A wildcard or a shadowed entry: a warning.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub finding: bool,
}

/// One switch of the tool's view (§2.10), in ms since the view started.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArbitrationSwitch {
    /// An address, `None` the dead-man.
    pub from: Option<String>,
    pub to: Option<String>,
    /// `start`, `sample`, `deadline` or `withdrawn`.
    pub cause: String,
    pub at_ms: f64,
    /// `from`'s last arrival; `None` when `from` is.
    pub last_ms: Option<f64>,
}

/// A provider the view knows, in rank order, with its counts (§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArbitrationProvider {
    pub address: String,
    pub delivered: u64,
    pub not_active: u64,
    pub deleted: u64,
    pub expired: u64,
    pub not_incumbent: u64,
    /// Withdrawn at the view's end (§2.7).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub withdrawn: bool,
}

/// The tool's own view of a role (§2.14), never the consumer's.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArbitrationView {
    /// Always [`TOOLS_OWN_VIEW`].
    pub whose: &'static str,
    pub listened_ms: f64,
    /// It knows it lost a sample within the last deadline: its queue
    /// refused one, or its receive path stalled.
    pub lost: bool,
    /// The provider it selects at its end; `None` the dead-man.
    pub active: Option<String>,
    /// Whether withdrawals were heard: a watch session followed the
    /// providers' interface tokens (§2.7). Without, the deadline alone.
    pub watching: bool,
    /// §2.5's outcome 6 read: the required interface is governed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub governed: bool,
    pub switches: Vec<ArbitrationSwitch>,
    /// Switches past the ones kept, counted.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub switches_dropped: u64,
    pub providers: Vec<ArbitrationProvider>,
    /// The samples with no provider (§2.5): `wild`, `unresolved`,
    /// `unbound`.
    pub discards: BTreeMap<String, u64>,
    /// The reads again made (§2.7).
    pub reads: u64,
    /// The watch session's round trips: at its start, and after each move
    /// (§2.8).
    pub round_trips: u64,
    /// The losses it was told of: its queue's refusals and its stalls.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub losses: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// One consumer's role, as the tool read it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArbitrationRole {
    /// The consumer, `<system>/<service>`; absent for a role read from a
    /// contract in hand with no consumer's descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumer: Option<String>,
    /// The instance whose descriptor was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub role: String,
    /// The required interface.
    pub iface: String,
    /// The interface whose contract declares the role; `null` for a
    /// manifest's.
    pub declared_by: Option<String>,
    /// The revision of it read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declaring: Option<String>,
    /// The descriptor's bindings in order; `null` with no descriptor.
    pub bindings: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
    /// The revisions of the required interface the declaration was read
    /// over (§2.14).
    pub revisions: Vec<String>,
    pub declaration: ArbitrationDeclaration,
    pub selection: ArbitrationSelection,
    pub binding: ArbitrationBinding,
    /// The tool's view, when it listened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ArbitrationView>,
    /// What could not be read, one line each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl ArbitrationRole {
    /// The three answers, with their names.
    pub fn answers(&self) -> [(&'static str, ArbitrationAnswerToken, &str, bool); 3] {
        [
            (
                "declaration",
                self.declaration.answer,
                self.declaration.reason.as_str(),
                self.declaration.finding,
            ),
            (
                "selection",
                self.selection.answer,
                self.selection.reason.as_str(),
                self.selection.finding,
            ),
            (
                "binding",
                self.binding.answer,
                self.binding.reason.as_str(),
                self.binding.finding,
            ),
        ]
    }

    /// The role's subject in words: `<consumer> <role>`, or the contract.
    pub fn subject(&self) -> String {
        match &self.consumer {
            Some(c) => format!("{c} {}", self.role),
            None => format!(
                "{} {} (no consumer)",
                self.declared_by.as_deref().unwrap_or("?"),
                self.role
            ),
        }
    }
}

/// One reading of a scope's arbitrated roles (`zenctl arbitration`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArbitrationReport {
    /// The namespace the session is in; empty for the bus root.
    pub namespace: String,
    /// The consumer asked of, `<system>/<service>`; absent: every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The role asked of; absent: every role carrying a policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// How long each view listened for, at most.
    pub listen_s: f64,
    /// The selectors put to the bus, base-relative.
    pub asked: Vec<String>,
    pub roles: Vec<ArbitrationRole>,
    /// What in scope could not be read, one line each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// Set when the reading as a whole answered nothing, with why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unobservable: Option<String>,
}

impl ArbitrationReport {
    /// The run as one [`Judgement`]: any question's finding over any role
    /// (§5's last column: a declaration invalid, the view in the dead-man,
    /// a wildcard or shadowed entry) is the finding, exit 1. Clean, exit 0,
    /// when every question asked of every role answers its other pole
    /// (`unbound` among them: informative, never a finding), and at least
    /// one was asked. No verdict otherwise, exit 2.
    pub fn judgement(&self) -> Judgement {
        if self.roles.iter().any(|r| r.answers().iter().any(|a| a.3)) {
            return Judgement::Established;
        }
        if let Some(why) = &self.unobservable {
            return Judgement::Unobservable {
                reason: why.clone(),
            };
        }
        let ns = if self.namespace.is_empty() {
            "the bus root".to_owned()
        } else {
            format!("namespace {:?}", self.namespace)
        };
        if self.roles.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no arbitrated role in scope in {ns}: no consumer's descriptor lists a role \
                     carrying arbitration.policy{}",
                    match &self.role {
                        Some(r) => format!(" named {r:?}, nor does a contract in hand"),
                        None => String::new(),
                    }
                ),
            };
        }
        let asked: Vec<(String, &'static str, ArbitrationAnswerToken, &str)> = self
            .roles
            .iter()
            .flat_map(|r| {
                r.answers()
                    .into_iter()
                    .filter(|a| a.1.asked())
                    .map(move |a| (r.subject(), a.0, a.1, a.2))
            })
            .collect();
        if asked.is_empty() {
            return Judgement::Unobservable {
                reason: format!(
                    "no question was asked of the {} role(s) read: none is this profile's",
                    self.roles.len()
                ),
            };
        }
        if let Some((s, q, _, why)) = asked
            .iter()
            .find(|a| a.2 == ArbitrationAnswerToken::Unobservable)
        {
            return Judgement::Unobservable {
                reason: format!(
                    "{s}: the {q} question could not be answered, {why} (arbitration.v1 §5)"
                ),
            };
        }
        Judgement::NotEstablished {
            reason: format!(
                "every question asked of the {} role(s) answered on its other pole",
                self.roles.len()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::judgement_exit_code;
    use serde_json::json;

    fn role(
        decl: (ArbitrationAnswerToken, &str, bool),
        sel: (ArbitrationAnswerToken, &str, bool),
        bind: (ArbitrationAnswerToken, &str, bool),
    ) -> ArbitrationRole {
        ArbitrationRole {
            consumer: Some("lab/rudder-l".into()),
            instance: Some("000000000000000a".into()),
            role: "cmd".into(),
            iface: "steer.v1".into(),
            declared_by: Some("rudder.v1".into()),
            declaring: None,
            bindings: Some(vec!["lab/safety".into(), "lab/*".into()]),
            params: BTreeMap::new(),
            revisions: vec![],
            declaration: ArbitrationDeclaration {
                answer: decl.0,
                reason: decl.1.into(),
                says: "words".into(),
                reasons: vec![],
                policy: None,
                resource: None,
                deadline_ms: None,
                period_ms: None,
                finding: decl.2,
            },
            selection: ArbitrationSelection {
                answer: sel.0,
                reason: sel.1.into(),
                says: "words".into(),
                provider: None,
                whose: TOOLS_OWN_VIEW,
                finding: sel.2,
            },
            binding: ArbitrationBinding {
                answer: bind.0,
                reason: bind.1.into(),
                says: "words".into(),
                wildcards: vec![],
                shadowed: vec![],
                wildcard_entries: vec![],
                shadowed_entries: vec![],
                finding: bind.2,
            },
            view: None,
            notes: vec![],
        }
    }

    fn report(roles: Vec<ArbitrationRole>) -> ArbitrationReport {
        ArbitrationReport {
            namespace: "acme".into(),
            target: Some("lab/rudder-l".into()),
            role: None,
            listen_s: 2.0,
            asked: vec!["zk2/lab/rudder-l/@zk/**".into()],
            roles,
            skipped: vec![],
            unobservable: None,
        }
    }

    use ArbitrationAnswerToken as T;

    fn clean() -> ArbitrationRole {
        role(
            (T::Yes, "valid", false),
            (T::Yes, "selected", false),
            (T::No, "exact", false),
        )
    }

    /// The shapes a script reads, pinned: the three answers in the
    /// fixture's tokens, the selection and the view labelled the tool's
    /// own, a switch's instants, a provider's five counts.
    #[test]
    fn the_arbitration_report_pins_its_shape() {
        let mut r = clean();
        r.declaration.policy = Some("priority".into());
        r.declaration.resource = Some("stream/cmd".into());
        r.declaration.deadline_ms = Some(300);
        r.declaration.period_ms = Some(50);
        r.selection.provider = Some("lab/safety".into());
        r.binding = ArbitrationBinding {
            answer: T::Yes,
            reason: "wildcard".into(),
            says: "words".into(),
            wildcards: vec![1],
            shadowed: vec![],
            wildcard_entries: vec!["lab/*".into()],
            shadowed_entries: vec![],
            finding: true,
        };
        r.view = Some(ArbitrationView {
            whose: TOOLS_OWN_VIEW,
            listened_ms: 2000.5,
            lost: false,
            active: Some("lab/safety".into()),
            watching: true,
            governed: false,
            switches: vec![ArbitrationSwitch {
                from: None,
                to: None,
                cause: "start".into(),
                at_ms: 0.0,
                last_ms: None,
            }],
            switches_dropped: 0,
            providers: vec![ArbitrationProvider {
                address: "lab/safety".into(),
                delivered: 40,
                not_active: 0,
                deleted: 0,
                expired: 0,
                not_incumbent: 0,
                withdrawn: false,
            }],
            discards: BTreeMap::from([("wild".to_owned(), 0)]),
            reads: 0,
            round_trips: 1,
            losses: 0,
        });
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v["declaration"],
            json!({"answer": "yes", "reason": "valid", "says": "words", "policy": "priority",
                   "resource": "stream/cmd", "deadline_ms": 300, "period_ms": 50})
        );
        assert_eq!(
            v["selection"],
            json!({"answer": "yes", "reason": "selected", "says": "words",
                   "provider": "lab/safety", "whose": "the tool's own view"})
        );
        assert_eq!(
            v["binding"],
            json!({"answer": "yes", "reason": "wildcard", "says": "words", "wildcards": [1],
                   "shadowed": [], "wildcard_entries": ["lab/*"], "finding": true})
        );
        assert_eq!(
            v["view"],
            json!({"whose": "the tool's own view", "listened_ms": 2000.5, "lost": false,
                   "active": "lab/safety", "watching": true,
                   "switches": [{"from": null, "to": null, "cause": "start", "at_ms": 0.0,
                                 "last_ms": null}],
                   "providers": [{"address": "lab/safety", "delivered": 40, "not_active": 0,
                                  "deleted": 0, "expired": 0, "not_incumbent": 0}],
                   "discards": {"wild": 0}, "reads": 0, "round_trips": 1})
        );
        assert_eq!(v["declared_by"], "rudder.v1");
        assert_eq!(v["bindings"], json!(["lab/safety", "lab/*"]));
        let offline = ArbitrationRole {
            consumer: None,
            instance: None,
            bindings: None,
            ..clean()
        };
        let v = serde_json::to_value(&offline).unwrap();
        assert!(v.get("consumer").is_none());
        assert_eq!(
            v["bindings"],
            json!(null),
            "no descriptor read is null, never []"
        );
        let whole = serde_json::to_value(report(vec![clean()])).unwrap();
        assert_eq!(
            whole.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["asked", "listen_s", "namespace", "roles", "target"]
        );
    }

    /// The exit: a finding of any question is 1, a warning's included;
    /// every question asked on its other pole, `unbound` among them, 0;
    /// an unobservable answer, or nothing asked, 2.
    #[test]
    fn the_run_exits_by_its_findings_then_its_poles() {
        let code = |r: ArbitrationReport| judgement_exit_code(&r.judgement());
        assert_eq!(code(report(vec![clean()])), 0);
        let unbound = role(
            (T::Yes, "valid", false),
            (T::No, "unbound", false),
            (T::No, "exact", false),
        );
        assert_eq!(code(report(vec![unbound])), 0, "unbound is informative");
        let wildcard = role(
            (T::Yes, "valid", false),
            (T::Yes, "selected", false),
            (T::Yes, "wildcard", true),
        );
        assert_eq!(code(report(vec![wildcard])), 1, "a warning is a finding");
        let dead = role(
            (T::Yes, "valid", false),
            (T::No, "dead_man", true),
            (T::No, "exact", false),
        );
        assert_eq!(code(report(vec![clean(), dead])), 1);
        let offline_invalid = role(
            (T::No, "invalid", true),
            (T::NotAsked, "no_descriptor", false),
            (T::NotAsked, "no_descriptor", false),
        );
        assert_eq!(code(report(vec![offline_invalid])), 1);
        let listening = role(
            (T::Yes, "valid", false),
            (T::Unobservable, "listening", false),
            (T::No, "exact", false),
        );
        assert_eq!(code(report(vec![clean(), listening])), 2);
        let none = role(
            (T::NotThisProfiles, "not_arbitrated", false),
            (T::NotThisProfiles, "not_arbitrated", false),
            (T::NotThisProfiles, "not_arbitrated", false),
        );
        assert_eq!(code(report(vec![none])), 2, "nothing asked");
        assert_eq!(code(report(vec![])), 2, "an empty scope");
    }
}
