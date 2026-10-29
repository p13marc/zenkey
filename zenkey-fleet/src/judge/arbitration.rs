//! `arbitration.v1` (#909, BC): a tool's reading of arbitrated roles
//! judged, every answer decided from values
//! (`spec/profiles/arbitration/v1.md` §2.14, §5).
//!
//! **Session-free.** [`judge`] reads an [`ArbitrationObservation`] that
//! [`crate::bus::arbitration`] gathered and hands back an
//! [`ArbitrationReport`]. Every answer is `zenkey_model::arbitration`'s:
//! `judge_declaration`, `judge_selection` and `judge_binding` over the
//! model's `Reading`, its view the tool's own `Arbiter`'s
//! ([`crate::model::arbitration::ViewBook`]). Nothing is re-derived here:
//! this module maps the model's answers onto the report's tokens, and says
//! them in words.
//!
//! **The run's verdict** ([`ArbitrationReport::judgement`]): a finding of
//! any question of any role is the finding; every question asked answered
//! on its other pole is clean; anything else is no verdict.

use std::collections::BTreeMap;
use std::time::Instant;

use zenkey_model::arbitration::{
    Answer, Declaration, Held, Judged, Reading, Reason, judge_binding, judge_declaration,
    judge_selection,
};

use crate::bus::arbitration::{ArbitrationObservation, Listened};
use crate::model::arbitration::{RoleRead, RoleSubject, ViewBook};
use crate::report::{
    ArbitrationAnswerToken, ArbitrationBinding, ArbitrationDeclaration, ArbitrationProvider,
    ArbitrationReport, ArbitrationRole, ArbitrationSelection, ArbitrationSwitch, ArbitrationView,
    TOOLS_OWN_VIEW,
};

/// The report's token for one of the model's answers.
pub fn token(a: Answer) -> ArbitrationAnswerToken {
    match a {
        Answer::Yes => ArbitrationAnswerToken::Yes,
        Answer::No => ArbitrationAnswerToken::No,
        Answer::Unobservable => ArbitrationAnswerToken::Unobservable,
        Answer::NotAsked => ArbitrationAnswerToken::NotAsked,
        Answer::NotThisProfiles => ArbitrationAnswerToken::NotThisProfiles,
    }
}

/// A reason in words, for a line.
pub fn says(r: Reason) -> &'static str {
    match r {
        Reason::NoContract => {
            "the contract declaring the role, at the revision the descriptor lists, is not in hand"
        }
        Reason::NotArbitrated => {
            "the role carries no arbitration.policy (or a manifest declares it, whose policy no \
             tool sees): not this profile's"
        }
        Reason::RevisionsDisagree => {
            "the revisions of the required interface in hand give two declarations"
        }
        Reason::NoProviderContract => "no revision of the required interface is in hand",
        Reason::Invalid => "the declaration is invalid: its consumer refuses to start",
        Reason::Valid => "arbitrated, with its policy and its deadline",
        Reason::Unbound => "the role is bound to nothing: its consumer holds the dead-man for good",
        Reason::NotListened => "the tool did not listen",
        Reason::Lost => {
            "the tool lost samples within the last deadline: a lost one may have been a higher provider's"
        }
        Reason::Selected => "the tool's own view follows this provider",
        Reason::Listening => "listened less than one deadline, nothing eligible yet",
        Reason::DeadMan => {
            "the tool's own view holds the dead-man after listening one deadline: no provider eligible"
        }
        Reason::Wildcard => {
            "a wildcard entry: the byte order among its addresses is a priority nobody chose"
        }
        Reason::Shadowed => "an entry that ranks no address of its own",
        Reason::Exact => "exact entries, none shadowed",
        Reason::NoDescriptor => "no consumer's descriptor read: no binding, and no view",
    }
}

fn whole_ms(d: Option<std::time::Duration>) -> Option<u64> {
    d.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// §5's first answer, as the report carries it.
pub fn declaration_row(r: &Reading) -> ArbitrationDeclaration {
    let (j, why): (Judged, _) = judge_declaration(r);
    let a = match &r.declaration {
        Held::Read(Declaration::Arbitrated(a)) => Some(a),
        _ => None,
    };
    let mut words = says(j.reason).to_owned();
    if !why.is_empty() {
        words = format!(
            "invalid: {}",
            why.iter()
                .map(|w| format!("{} ({})", w.says(), w.as_str()))
                .collect::<Vec<_>>()
                .join("; ")
        );
    } else if let Some(a) = a {
        words = format!(
            "arbitrated, {} over {}, deadline {} ms{}",
            a.policy,
            a.resource(),
            whole_ms(Some(a.deadline)).unwrap_or(0),
            whole_ms(a.period)
                .map(|p| format!(", period {p} ms"))
                .unwrap_or_default()
        );
    }
    ArbitrationDeclaration {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: words,
        reasons: why.iter().map(|w| w.as_str().to_owned()).collect(),
        policy: a.map(|a| a.policy.as_str().to_owned()),
        resource: a.map(zenkey_model::arbitration::Arbitrated::resource),
        deadline_ms: a.and_then(|a| whole_ms(Some(a.deadline))),
        period_ms: a.and_then(|a| whole_ms(a.period)),
        finding: j.answer == Answer::No,
    }
}

/// §5's second answer, the tool's own view.
pub fn selection_row(r: &Reading) -> ArbitrationSelection {
    let (j, provider) = judge_selection(r);
    ArbitrationSelection {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: says(j.reason).to_owned(),
        provider: provider.map(|a| a.to_string()),
        whose: TOOLS_OWN_VIEW,
        finding: j.answer == Answer::No && j.reason == Reason::DeadMan,
    }
}

/// §5's third answer, naming the wildcard and the shadowed entries.
pub fn binding_row(r: &Reading) -> ArbitrationBinding {
    let (j, wildcards, shadowed) = judge_binding(r);
    let entries = r
        .binding
        .as_ref()
        .map(|b| {
            b.entries()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let named = |at: &[usize]| -> Vec<String> {
        at.iter().filter_map(|i| entries.get(*i).cloned()).collect()
    };
    ArbitrationBinding {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: says(j.reason).to_owned(),
        wildcard_entries: named(&wildcards),
        shadowed_entries: named(&shadowed),
        wildcards,
        shadowed,
        finding: j.answer == Answer::Yes,
    }
}

/// The view's row.
pub fn view_row(book: &ViewBook, l: &Listened) -> ArbitrationView {
    let v = book.view(l.end);
    let ms = |at: Instant| book.ms(at);
    let d = book.discards();
    ArbitrationView {
        whose: TOOLS_OWN_VIEW,
        listened_ms: v.listened.as_secs_f64() * 1e3,
        lost: v.lost,
        active: v.active.map(|a| a.to_string()),
        watching: l.watching,
        governed: book.governed(),
        switches: book
            .switches()
            .iter()
            .map(|s| ArbitrationSwitch {
                from: s.from.as_ref().map(ToString::to_string),
                to: s.to.as_ref().map(ToString::to_string),
                cause: s.cause.as_str().to_owned(),
                at_ms: ms(s.at),
                last_ms: s.last.map(ms),
            })
            .collect(),
        switches_dropped: book.switches_dropped(),
        providers: book
            .providers()
            .into_iter()
            .map(|(a, c, withdrawn)| ArbitrationProvider {
                address: a.to_string(),
                delivered: c.delivered,
                not_active: c.not_active,
                deleted: c.deleted,
                expired: c.expired,
                not_incumbent: c.not_incumbent,
                withdrawn,
            })
            .collect(),
        discards: BTreeMap::from([
            ("wild".to_owned(), d.wild),
            ("unresolved".to_owned(), d.unresolved),
            ("unbound".to_owned(), d.unbound),
        ]),
        reads: l.reads,
        round_trips: l.round_trips,
        losses: book.losses(),
    }
}

/// One role's row: its reading, with the tool's view when it listened.
pub fn role_row(
    s: &RoleSubject,
    read: &RoleRead,
    listened: Option<&(ViewBook, Listened)>,
) -> ArbitrationRole {
    let mut reading = read.reading.clone();
    reading.view = listened.map(|(b, l)| b.view(l.end));
    let mut notes = read.notes.clone();
    if let Some((_, l)) = listened {
        notes.extend(l.notes.iter().cloned());
    }
    ArbitrationRole {
        consumer: s.consumer.as_ref().map(ToString::to_string),
        instance: s.instance.as_ref().map(ToString::to_string),
        role: s.role.clone(),
        iface: s.required.to_string(),
        declared_by: s.declared_by.as_ref().map(ToString::to_string),
        declaring: s.declaring.as_ref().map(ToString::to_string),
        bindings: s.bindings.clone(),
        params: s.params.clone(),
        revisions: read.revisions.iter().map(ToString::to_string).collect(),
        declaration: declaration_row(&reading),
        selection: selection_row(&reading),
        binding: binding_row(&reading),
        view: listened.map(|(b, l)| view_row(b, l)),
        notes,
    }
}

/// One reading judged: see the module doc.
pub fn judge(obs: &ArbitrationObservation) -> ArbitrationReport {
    ArbitrationReport {
        namespace: obs.namespace.clone(),
        target: obs.target.as_ref().map(ToString::to_string),
        role: obs.role.clone(),
        listen_s: obs.listen.as_secs_f64(),
        asked: obs.asked.clone(),
        roles: obs
            .roles
            .iter()
            .map(|r| role_row(&r.subject, &r.read, r.view.as_ref()))
            .collect(),
        skipped: obs.skipped.clone(),
        unobservable: obs.unobservable.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use serde_json::Value;
    use zenkey_model::arbitration::{Binding, View};

    use crate::model::arbitration::{Declaring, read_role, subjects};

    /// `conclusions.json`'s reading, as the model reads it from the fixture.
    fn reading(v: &Value) -> Reading {
        let d = &v["declaration"];
        let declaration = match d["declaration"].as_str().unwrap() {
            "no_contract" => Held::NoContract,
            "revisions_disagree" => Held::RevisionsDisagree,
            "not_arbitrated" => Held::Read(Declaration::NotArbitrated),
            "unread" => Held::Read(Declaration::Unread),
            "invalid" => Held::Read(Declaration::Invalid(
                d["reasons"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        use zenkey_model::arbitration::Invalid::*;
                        match r.as_str().unwrap() {
                            "not_used" => NotUsed,
                            "manifest_role" => ManifestRole,
                            "policy_unsupported" => PolicyUnsupported,
                            "value" => Value,
                            "cardinality" => Cardinality,
                            "resource" => Resource,
                            "no_deadline" => NoDeadline,
                            "params_unbound" => ParamsUnbound,
                            o => panic!("{o}"),
                        }
                    })
                    .collect(),
            )),
            "arbitrated" => Held::Read(Declaration::Arbitrated(
                zenkey_model::arbitration::Arbitrated {
                    policy: zenkey_model::arbitration::Policy::Priority,
                    iface: "steer.v1".parse().unwrap(),
                    token: zenkey_model::grammar::KindToken::Stream,
                    template: zenkey_model::template::Template::parse("cmd").unwrap(),
                    deadline: std::time::Duration::from_millis(d["deadline_ms"].as_u64().unwrap()),
                    period: d["period_ms"]
                        .as_u64()
                        .map(std::time::Duration::from_millis),
                },
            )),
            o => panic!("{o}"),
        };
        let binding = v["bindings"].as_array().map(|b| {
            let e: Vec<&str> = b.iter().map(|x| x.as_str().unwrap()).collect();
            Binding::parse(&e).unwrap()
        });
        let view = v.get("view").map(|w| View {
            listened: std::time::Duration::from_millis(w["listened_ms"].as_u64().unwrap()),
            lost: w["lost"].as_bool().unwrap(),
            active: w["active"].as_str().map(|a| a.parse().unwrap()),
        });
        Reading {
            declaration,
            binding,
            view,
        }
    }

    /// Every case of `conclusions.json` through the report's rows: the
    /// tokens, the reasons, the provider and the entries' positions the
    /// fixture expects, and a finding exactly where §5's last column says.
    #[test]
    fn every_conclusion_reaches_the_report_as_the_fixture_says() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/arbitration/conformance/conclusions.json");
        let fixture: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 24);
        for c in cases {
            let r = reading(&c["reading"]);
            let (d, s, b) = (declaration_row(&r), selection_row(&r), binding_row(&r));
            let e = &c["expect"];
            let note = c["note"].as_str().unwrap();
            assert_eq!(d.answer.as_str(), e["declaration"]["answer"], "{note}");
            assert_eq!(d.reason, e["declaration"]["reason"], "{note}");
            if let Some(why) = e["declaration"].get("reasons") {
                assert_eq!(serde_json::to_value(&d.reasons).unwrap(), *why, "{note}");
            }
            assert_eq!(s.answer.as_str(), e["selection"]["answer"], "{note}");
            assert_eq!(s.reason, e["selection"]["reason"], "{note}");
            assert_eq!(
                s.provider.as_deref(),
                e["selection"]["provider"].as_str(),
                "{note}"
            );
            assert_eq!(s.whose, TOOLS_OWN_VIEW);
            assert_eq!(b.answer.as_str(), e["binding"]["answer"], "{note}");
            assert_eq!(b.reason, e["binding"]["reason"], "{note}");
            assert_eq!(
                serde_json::to_value(&b.wildcards).unwrap(),
                e["binding"]["wildcards"],
                "{note}"
            );
            assert_eq!(
                serde_json::to_value(&b.shadowed).unwrap(),
                e["binding"]["shadowed"],
                "{note}"
            );
            assert_eq!(d.finding, d.answer == ArbitrationAnswerToken::No, "{note}");
            assert_eq!(s.finding, s.reason == "dead_man", "{note}");
            assert_eq!(b.finding, b.answer == ArbitrationAnswerToken::Yes, "{note}");
        }
    }

    fn contract(name: &str) -> zenkey_model::contract::Contract {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts")
            .join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&path).unwrap();
        let l = zenkey_model::contract::load_str(&text, path.parent().unwrap(), None);
        l.contract.unwrap_or_else(|| panic!("{}", l.report))
    }

    /// A descriptor's role read over the scenarios' contracts: valid with
    /// `steer.v1`; two revisions of it with two deadlines disagree
    /// (scenarios §10 step 5); none held is `no_provider_contract`; the
    /// declaring contract missing is `no_contract`.
    #[test]
    fn a_role_reads_over_every_revision_held() {
        let rudder = contract("rudder.v1");
        let steer = contract("steer.v1");
        let steer_200 = {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../zenkey/tests/contracts");
            let text = std::fs::read_to_string(path.join("steer.v1.toml"))
                .unwrap()
                .replace(
                    "\"timing.deadline_ms\" = 300",
                    "\"timing.deadline_ms\" = 200",
                );
            zenkey_model::contract::load_str(&text, &path, None)
                .contract
                .unwrap()
        };
        let d: zenkey_model::descriptor::Descriptor = serde_json::from_value(serde_json::json!({
            "format": "zk2-descriptor/0.1",
            "service": "lab/rudder-l",
            "instance": "000000000000000a",
            "interfaces": [{"iface": "rudder.v1", "contract": format!("sha256:{}", "a".repeat(64)), "minor": 0}],
            "requires": [{"role": "cmd", "interface": "steer.v1", "declared_by": "rudder.v1",
                          "bindings": ["lab/safety", "lab/teleop", "lab/autopilot"]}]
        }))
        .unwrap();
        let a: zenkey_model::grammar::Addr = "lab/rudder-l".parse().unwrap();
        let i: zenkey_model::grammar::InstanceId = "000000000000000a".parse().unwrap();
        let s = &subjects([(&a, &i, &d)], None)[0];
        let fp = |n: u8| {
            zenkey_model::canonical::Fingerprint::parse(&format!(
                "sha256:{}",
                format!("{n:x}").repeat(64)
            ))
            .unwrap()
        };
        let one = read_role(s, Declaring::Held(&rudder), &[(fp(1), &steer)]);
        let row = role_row(s, &one, None);
        assert_eq!(row.declaration.reason, "valid");
        assert_eq!(row.declaration.deadline_ms, Some(300));
        assert_eq!(row.selection.reason, "not_listened");
        assert_eq!(row.binding.reason, "exact");
        let two = read_role(
            s,
            Declaring::Held(&rudder),
            &[(fp(1), &steer), (fp(2), &steer_200)],
        );
        assert_eq!(
            role_row(s, &two, None).declaration.reason,
            "revisions_disagree"
        );
        let none = read_role(s, Declaring::Held(&rudder), &[]);
        assert_eq!(
            role_row(s, &none, None).declaration.reason,
            "no_provider_contract"
        );
        let missing = read_role(s, Declaring::Missing("not retrieved"), &[(fp(1), &steer)]);
        assert_eq!(
            role_row(s, &missing, None).declaration.reason,
            "no_contract"
        );
        assert!(crate::model::arbitration::in_scope(&missing));
    }
}
