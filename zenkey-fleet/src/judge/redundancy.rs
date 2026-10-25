//! `redundancy.v1` (#841, RC): §5's four questions of each governed
//! interface, and `hostid.v1` §2.12's recognised standby
//! (`spec/profiles/redundancy/v1.md` §2.15).
//!
//! Every answer is `zenkey_model::redundancy`'s ([`model::one_incumbent`],
//! [`model::vacancy`], [`model::standbys`], [`model::twin`]) over the
//! projection [`crate::model::redundancy`] makes; nothing is re-derived.
//! An instance's [`RedundancyRole`] is read off its fourth answer.

use zenkey_model::grammar::{Addr, IfaceId, InstanceId};
use zenkey_model::redundancy::{self as model, Answer, Governance, Judged, Reading};

use crate::bus::redundancy::{RedundancyObservation, RedundancySpec};
use crate::model::catalog::Observed;
use crate::model::redundancy::{self as projection, Contracts, Governed};
use crate::report::{
    RedundancyAnswer, RedundancyAnswerToken, RedundancyInstance, RedundancyRead, RedundancyReport,
    RedundancyRole, RedundancyRow, RedundancySeen, RedundancyStandbys,
};

fn token(a: Answer) -> RedundancyAnswerToken {
    match a {
        Answer::Yes => RedundancyAnswerToken::Yes,
        Answer::No => RedundancyAnswerToken::No,
        Answer::Unobservable => RedundancyAnswerToken::Unobservable,
        Answer::NotAsked => RedundancyAnswerToken::NotAsked,
        Answer::NotThisProfiles => RedundancyAnswerToken::NotThisProfiles,
    }
}

/// An answer's reason in words, for a person.
fn says(j: Judged, governed: &Governed) -> String {
    use model::Reason as R;
    match j.reason {
        R::AcrossFace => "across a constrained face presence does not cross: the deployment's \
                          word names the address, never which instance serves it (§2.13)"
            .into(),
        R::NotGoverned | R::NoContract | R::RevisionsDisagree => governed.says.clone(),
        R::Absent => "nothing of it in a complete read".into(),
        R::PresenceIncomplete => "a read that decides it is not complete: it can miss a \
                                  token, never invent one (core §8.1)"
            .into(),
        R::Split => "two or more holders of its token in both reads a grace apart (core §6)".into(),
        R::InTransition => "a shape seen in one read only: a re-mint, a takeover or a split \
                            not yet established"
            .into(),
        R::One => "one holder of its token, the read complete".into(),
        R::NoDescriptor => "no candidate seen, a descriptor unread".into(),
        R::NoCandidate => "no instance lists the interface".into(),
        R::Vacant => "no holder, a candidate present".into(),
        R::Held => "a holder of its token in the second read".into(),
        R::Lasting => "no holder in either read, both complete, a candidate present: the \
                       election did not end, or no instance holds the lock"
            .into(),
        R::StandingBy => "marked standby, holding none of its tokens".into(),
        R::NoneStanding => "no instance marked standby and holding none of its tokens".into(),
        R::SplitLowest => "a holder of a split, the lowest, which the heal keeps".into(),
        R::NotACandidate => "it lists the interface nowhere and holds no token of it".into(),
        R::Unlisted => "it holds the token, and its descriptor does not list the interface, in \
                        both reads"
            .into(),
        R::Incumbent => "the sole incumbent, unmarked, the read complete".into(),
        R::MarkedHolding => "it holds the token while its descriptor marks it standby, in both \
                             reads"
            .into(),
        R::Standby => "marked standby, no token, the read complete".into(),
        R::Unmarked => "it lists the interface unmarked and holds no token, in both reads: an \
                        exposure no token covers"
            .into(),
    }
}

fn answer(j: Judged, governed: &Governed) -> RedundancyAnswer {
    RedundancyAnswer {
        answer: token(j.answer),
        reason: j.reason.as_str().to_owned(),
        says: says(j, governed),
    }
}

/// What an instance reads as, from its fourth answer.
pub fn role_of(j: Judged) -> RedundancyRole {
    use model::Reason as R;
    match (j.answer, j.reason) {
        (Answer::No, R::Incumbent) => RedundancyRole::Incumbent,
        (Answer::No, R::Standby) => RedundancyRole::Standby,
        (Answer::Yes, _) => RedundancyRole::Twin,
        (Answer::NotAsked, _) => RedundancyRole::NotACandidate,
        _ => RedundancyRole::Unknown,
    }
}

fn seen_in(r: &model::Read, id: &InstanceId) -> Option<RedundancySeen> {
    r.get(id).map(|s| RedundancySeen {
        token: s.token,
        listing: projection::listing_token(s.descriptor).to_owned(),
    })
}

/// One address and interface (§2.15): §5's four questions, each instance
/// of either read asked the fourth.
pub fn row(
    addr: &Addr,
    iface: &IfaceId,
    governed: &Governed,
    reading: &Reading,
    zid: impl Fn(&InstanceId) -> Option<String>,
) -> RedundancyRow {
    let (sb, count) = model::standbys(reading);
    let mut ids: Vec<InstanceId> = reading
        .first
        .instances
        .iter()
        .chain(&reading.second.instances)
        .map(|s| s.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    let instances = ids
        .into_iter()
        .map(|id| {
            let t = model::twin(reading, &id);
            RedundancyInstance {
                zid: zid(&id),
                first: seen_in(&reading.first, &id),
                second: seen_in(&reading.second, &id),
                twin: answer(t, governed),
                role: role_of(t),
                instance: id.to_string(),
            }
        })
        .collect();
    RedundancyRow {
        address: addr.to_string(),
        iface: iface.to_string(),
        governance: governed.governance.as_str().to_owned(),
        governance_says: governed.says.clone(),
        holders: reading
            .second
            .holders()
            .into_iter()
            .map(ToString::to_string)
            .collect(),
        one_incumbent: answer(model::one_incumbent(reading), governed),
        vacancy: answer(model::vacancy(reading), governed),
        standbys: RedundancyStandbys {
            answer: token(sb.answer),
            reason: sb.reason.as_str().to_owned(),
            says: says(sb, governed),
            count: count.count,
            exact: count.exact,
        },
        instances,
    }
}

/// The rows of two reads: every (address, interface) in scope that is
/// governed, or whose governance cannot be told, or the one asked; an
/// interface known not governed is left out unless asked by name.
pub fn rows(
    first: Option<&Observed>,
    second: Option<&Observed>,
    contracts: &Contracts,
    addr: Option<&Addr>,
    iface: Option<&IfaceId>,
    across_face: bool,
) -> Vec<RedundancyRow> {
    let reads = [first, second];
    projection::subjects(&reads, addr, iface)
        .into_iter()
        .filter_map(|(a, i)| {
            let governed = if across_face {
                Governed {
                    governance: Governance::NoContract,
                    says: "across a constrained face no descriptor crosses (§2.13)".into(),
                }
            } else {
                projection::governance(&reads, &a, &i, contracts)
            };
            if governed.governance == Governance::NotGoverned && iface.is_none() {
                return None;
            }
            let reading =
                projection::reading(first, second, &a, &i, governed.governance, across_face);
            Some(row(&a, &i, &governed, &reading, |id| {
                projection::zid_of(&reads, &a, id)
            }))
        })
        .collect()
}

/// `zenctl redundancy`'s report (#841, RC), from what
/// [`crate::bus::redundancy::observe`] read.
pub fn judge(
    namespace: &str,
    target: Option<&Addr>,
    spec: &RedundancySpec,
    obs: &RedundancyObservation,
) -> RedundancyReport {
    let first = obs.first.as_ref().and_then(|r| r.as_ref().ok());
    let second = obs.second.as_ref().and_then(|r| r.as_ref().ok());
    let read = |r: &Option<Result<Observed, String>>| match r {
        Some(Ok(o)) => RedundancyRead {
            complete: o.complete,
            reason: o
                .incomplete_reason("ended at its timeout")
                .map(str::to_owned),
            instances: o.instances().len(),
        },
        Some(Err(e)) => RedundancyRead {
            complete: false,
            reason: Some(format!("not made: {e}")),
            instances: 0,
        },
        None => RedundancyRead {
            complete: false,
            reason: Some(
                obs.cut_off
                    .clone()
                    .map_or_else(|| "not made".to_owned(), |w| format!("not made: {w}")),
            ),
            instances: 0,
        },
    };
    let reads = if spec.across_face {
        Vec::new()
    } else {
        vec![read(&obs.first), read(&obs.second)]
    };
    let interfaces = rows(
        first,
        second,
        &obs.contracts,
        target,
        spec.iface.as_ref(),
        spec.across_face,
    );
    let unobservable = if let Some(why) = &obs.cut_off {
        Some(format!("no read was made: {why}"))
    } else if spec.across_face && interfaces.is_empty() {
        Some(
            "across a constrained face presence does not cross: name the address and --iface, \
             the deployment's word (§2.13)"
                .to_owned(),
        )
    } else if !spec.across_face && first.is_none() && second.is_none() {
        Some("neither presence read could be made".to_owned())
    } else {
        None
    };
    RedundancyReport {
        namespace: namespace.to_owned(),
        service: target.map(ToString::to_string),
        iface: spec.iface.as_ref().map(ToString::to_string),
        apart_s: obs.apart.as_secs_f64(),
        across_face: spec.across_face,
        asked: obs.asked.clone(),
        reads,
        interfaces,
        unobservable,
    }
}

/// What `hostid.v1` §2.12 makes of one instance (its text 0.6, 0.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standby {
    /// A recognised standby: its second-read descriptor lists only
    /// interfaces whose contracts use `redundancy.v1`, at least one, and
    /// the fourth question answers no, `standby`, of each, from the same
    /// two reads. Not counted.
    Recognised,
    /// Counted, and why it is not a recognised standby: its descriptor
    /// lists no interface or one not governed, or a twin, or its
    /// descriptor did not read (which the caller reads as it reads any
    /// unread descriptor).
    Counted(String),
    /// It lists only interfaces that are governed or whose governance
    /// cannot be read, and for one of them the question cannot be
    /// answered: hostid's §5 reads that unobservable, the finding not
    /// established (0.6).
    Unobservable(String),
}

/// `hostid.v1` §2.12's recognised standby (its text 0.6, 0.7), from the
/// two reads a doctor or a reader holds.
pub fn recognised_standby(
    first: Option<&Observed>,
    second: Option<&Observed>,
    contracts: &Contracts,
    addr: &Addr,
    instance: &InstanceId,
) -> Standby {
    let reads = [first, second];
    let Some(d) = second
        .and_then(|o| o.descriptors.as_ref())
        .and_then(|ds| ds.get(&(addr.clone(), instance.clone())))
        .and_then(|r| r.descriptor())
    else {
        return Standby::Counted("its descriptor in the second read was not read".to_owned());
    };
    if d.interfaces.is_empty() {
        return Standby::Counted("it lists no interface".to_owned());
    }
    let mut unobservable = Vec::new();
    for e in &d.interfaces {
        let Ok(iface) = e.iface.parse::<IfaceId>() else {
            return Standby::Counted(format!("{:?} is not an interface id", e.iface));
        };
        let governed = projection::governance(&reads, addr, &iface, contracts);
        match governed.governance {
            Governance::NotGoverned => {
                return Standby::Counted(format!("{iface} is not governed: {}", governed.says));
            }
            Governance::NoContract | Governance::Disagree => {
                unobservable.push(format!(
                    "whether {iface} is governed cannot be read: {}",
                    governed.says
                ));
                continue;
            }
            Governance::Governed(_) => {}
        }
        let reading = projection::reading(first, second, addr, &iface, governed.governance, false);
        let t = model::twin(&reading, instance);
        match role_of(t) {
            RedundancyRole::Standby => {}
            RedundancyRole::Twin => {
                return Standby::Counted(format!(
                    "of {iface}, a twin: \"is this instance a standby, or a twin?\" answers yes, \
                     {}",
                    t.reason.as_str()
                ));
            }
            RedundancyRole::Incumbent | RedundancyRole::NotACandidate => {
                return Standby::Counted(format!(
                    "of {iface}, not a standby: the question answers {}, {}",
                    t.answer.as_str(),
                    t.reason.as_str()
                ));
            }
            RedundancyRole::Unknown => unobservable.push(format!(
                "of {iface}, \"is this instance a standby, or a twin?\" answers {}, {}",
                t.answer.as_str(),
                t.reason.as_str()
            )),
        }
    }
    if unobservable.is_empty() {
        Standby::Recognised
    } else {
        Standby::Unobservable(unobservable.join("; "))
    }
}

#[cfg(test)]
mod tests {
    //! `conclusions.json`'s readings, built as the values two reads would
    //! hold, through the fleet's projection: every one answers as the
    //! fixture does, so the fleet decides nothing the model does not.

    use super::*;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use zenkey_model::grammar::{Fp16, ZkKey};
    use zenkey_model::redundancy::Election;

    use crate::model::catalog::DescriptorRead;

    fn ledger() -> IfaceId {
        "ledger.v1".parse().unwrap()
    }

    fn addr() -> Addr {
        "lab/ledger".parse().unwrap()
    }

    fn descriptor(id: &str, listing: &str) -> DescriptorRead {
        let interfaces = match listing {
            "not_listed" => serde_json::json!([]),
            "standby" | "serving" => serde_json::json!([{
                "iface": "ledger.v1",
                "contract": format!("sha256:{}", "0".repeat(64)),
                "minor": 0,
                "standby": listing == "standby",
            }]),
            other => panic!("{other}"),
        };
        DescriptorRead::Served(Box::new(
            serde_json::from_value(serde_json::json!({
                "format": "zk2-descriptor/0.1",
                "service": "lab/ledger",
                "instance": id,
                "interfaces": interfaces,
                "meta": {"zid": format!("z{id}")},
            }))
            .unwrap(),
        ))
    }

    /// A read as `elections.json` writes one, as the bus would hand it.
    fn observed(v: &Value) -> Observed {
        let mut tokens = Vec::new();
        let mut descriptors = BTreeMap::new();
        for i in v["instances"].as_array().unwrap() {
            let id: InstanceId = i["id"].as_str().unwrap().parse().unwrap();
            tokens.push(ZkKey::Instance {
                addr: addr(),
                instance: id.clone(),
            });
            if i["token"] == true {
                tokens.push(ZkKey::Alive {
                    addr: addr(),
                    iface: ledger(),
                    instance: id.clone(),
                    fp: Fp16::new("0000000000000000").unwrap(),
                });
            }
            let d = match i["descriptor"].as_str().unwrap() {
                "unread" => DescriptorRead::Silent,
                l => descriptor(id.as_str(), l),
            };
            descriptors.insert((addr(), id), d);
        }
        Observed {
            selector: "zk2/lab/ledger/@zk/**".into(),
            complete: v["complete"] == true,
            cut_off_during: false,
            at_interests_timeout: false,
            tokens,
            unparsed: vec![],
            descriptors: Some(descriptors),
        }
    }

    #[test]
    fn every_conclusion_reads_as_the_fixture() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../spec/profiles/redundancy/conformance/conclusions.json");
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 39);
        for case in cases {
            let r = &case["reading"];
            let governance = Governance::parse(r["governance"].as_str().unwrap()).unwrap();
            let governed = Governed {
                governance,
                says: String::new(),
            };
            let (first, second) = (observed(&r["first"]), observed(&r["second"]));
            let across = r["across_face"] == true;
            let reading = projection::reading(
                Some(&first),
                Some(&second),
                &addr(),
                &ledger(),
                governance,
                across,
            );
            let row = row(&addr(), &ledger(), &governed, &reading, |_| None);
            let note = &case["note"];
            let e = &case["expect"];
            let pair = |a: &RedundancyAnswer| serde_json::json!({"answer": a.answer.as_str(), "reason": a.reason});
            assert_eq!(pair(&row.one_incumbent), e["one_incumbent"], "{note}");
            assert_eq!(pair(&row.vacancy), e["vacancy"], "{note}");
            assert_eq!(
                serde_json::json!({
                    "answer": row.standbys.answer.as_str(),
                    "reason": row.standbys.reason,
                    "count": row.standbys.count,
                    "exact": row.standbys.exact,
                }),
                e["standbys"],
                "{note}"
            );
            let twins: serde_json::Map<String, Value> = row
                .instances
                .iter()
                .map(|i| (i.instance.clone(), pair(&i.twin)))
                .collect();
            assert_eq!(Value::Object(twins), e["twins"], "{note}");
        }
    }

    /// The governance of an address, from the revisions its descriptors
    /// name (§2.2): governed by one election, not governed, a revision not
    /// in hand, two that disagree.
    #[test]
    fn governance_is_read_from_every_listing_revision() {
        use crate::model::catalog::{ContractState, Revision};
        use std::sync::Arc;
        let load = |name: &str, text: Option<&str>| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts")
                .join(format!("{name}.toml"));
            let body = text.map_or_else(|| std::fs::read_to_string(&path).unwrap(), str::to_owned);
            let l = zenkey_model::contract::load_str(&body, path.parent().unwrap(), None);
            Arc::new(Revision::from_contract(
                l.contract.unwrap(),
                crate::report::ContractSource::File,
            ))
        };
        let governed_rev = load("ledger.v1", None);
        let plain = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../zenkey/tests/contracts/ledger.v1.toml"),
        )
        .unwrap()
        .replace("uses  = [\"redundancy.v1\"]\n", "")
        .replace(
            "[defaults]\nannotations = { \"redundancy.election\" = \"claim\" }\n",
            "",
        );
        let plain_rev = load("ledger.v1", Some(&plain));
        let fp = |r: &Arc<Revision>| r.fingerprint().clone();
        let with = |entries: &[(&str, &Arc<Revision>)]| -> Observed {
            let mut descriptors = BTreeMap::new();
            for (id, rev) in entries {
                descriptors.insert(
                    (addr(), id.parse::<InstanceId>().unwrap()),
                    DescriptorRead::Served(Box::new(
                        serde_json::from_value(serde_json::json!({
                            "format": "zk2-descriptor/0.1",
                            "service": "lab/ledger",
                            "instance": id,
                            "interfaces": [{"iface": "ledger.v1", "contract": fp(rev).to_string(), "minor": 0}],
                        }))
                        .unwrap(),
                    )),
                );
            }
            Observed {
                selector: String::new(),
                complete: true,
                cut_off_during: false,
                at_interests_timeout: false,
                tokens: vec![],
                unparsed: vec![],
                descriptors: Some(descriptors),
            }
        };
        let mut contracts = Contracts::new();
        contracts.insert(
            (ledger(), fp(&governed_rev)),
            Ok(ContractState::Held(Arc::clone(&governed_rev))),
        );
        contracts.insert(
            (ledger(), fp(&plain_rev)),
            Ok(ContractState::Held(Arc::clone(&plain_rev))),
        );
        let g = |o: &Observed, c: &Contracts| {
            projection::governance(&[Some(o)], &addr(), &ledger(), c).governance
        };
        let a = "000000000000000a";
        let b = "000000000000000b";
        assert_eq!(
            g(&with(&[(a, &governed_rev)]), &contracts),
            Governance::Governed(Election::Claim)
        );
        assert_eq!(
            g(&with(&[(a, &plain_rev)]), &contracts),
            Governance::NotGoverned
        );
        assert_eq!(
            g(&with(&[(a, &governed_rev), (b, &plain_rev)]), &contracts),
            Governance::Disagree
        );
        assert_eq!(
            g(&with(&[(a, &governed_rev)]), &Contracts::new()),
            Governance::NoContract
        );
    }
}
