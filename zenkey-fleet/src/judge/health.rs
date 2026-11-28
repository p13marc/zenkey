//! `health.v1` (#721, PF) and `health.v2` (#986's Q2): a health reading
//! judged, every verdict decided from values (`spec/profiles/health/v1.md`,
//! `v2.md`).
//!
//! **Which major.** Each service is read by the major its descriptor lists,
//! `health.v2` where it lists both (v2 §2.11's step 2, §2.12); across a
//! face, by the deployment's word; with no descriptor in hand, by the GET
//! that answered for it. Under `health.v2`, §5's clock question is the last
//! read heartbeat's (`zenkey_model::health::v2::clock_ahead`), a break of
//! §2.2 is reported from one reading that is one instant, and a word is
//! shown by the tables of the revisions its descriptor names.
//!
//! **Session-free.** [`judge`] reads a [`HealthObservation`] that
//! [`crate::bus::health`] gathered and hands back a [`HealthReport`]. Each
//! service's two readings are built by [`crate::model::health::reading`];
//! the verdicts are `zenkey_model::health`'s: `judge` for "is this service
//! healthy?" (§2.11), `agrees` for "does its status agree with its checks?",
//! `clock_ahead` for "is its clock ahead?" and `rollup` for the roll-up
//! (§5).
//!
//! **Who is asked.** One service named, or every service presence shows and
//! every one a reading's GET answered for (absent from a complete read, so
//! not asked). Across a constrained face, the one service, on the
//! deployment's word (§2.8).
//!
//! **Two readings** (§2.2). A break of the aggregation rule is a finding
//! about the owner only when both readings show it; one reading's is
//! unobservable, never clean and never the finding. Every other verdict is
//! the last reading's.

use std::collections::{BTreeMap, BTreeSet};

use zenkey_model::freshness::Observation;
use zenkey_model::grammar::Addr;
use zenkey_model::health::v2 as hv2;
use zenkey_model::health::{self as hm, Agreement, Judged, Major, Presence};

use crate::bus::health::{HealthObservation, HealthTarget, ServiceHeard};
use crate::model::catalog::Catalog;
use crate::model::health::{
    Got, ServiceReading, Trust, agreement_over, agreement_over_v2, answer, level_read_of,
    level_token, major_of, presence_in, reading, token, verdict_token, word_seen,
};
use crate::report::{
    CheckSeen, FaultSeen, HealthAnswer, HealthAnswerToken, HealthFace, HealthPresence,
    HealthReadingVerdict, HealthReport, HealthRollup, HealthRow, LastKnownStatus, LevelRead,
    StatusSeen, StatusVia,
};

/// Every verdict of one health reading.
pub fn judge(obs: &HealthObservation) -> HealthReport {
    let window = match &obs.window {
        Some(Ok(w)) => Some(w),
        _ => None,
    };
    let trust = if obs.spec.clocks_synced {
        Trust::Word
    } else if let Some(w) = window {
        Trust::Measured(&w.clocks)
    } else {
        Trust::None
    };
    let first = obs.first.as_ref().ok();
    let second = obs.second.as_ref().ok();
    let mut unobservable = None;
    // Who is asked, and how present each one is.
    let mut asked: BTreeMap<Addr, Presence> = BTreeMap::new();
    // The major each one is read by, where one is (v2 §2.11, §2.12).
    let mut majors: BTreeMap<Addr, Major> = BTreeMap::new();
    let mut presence_doc = None;
    match (&obs.spec.face, &obs.presence, &obs.target) {
        (Some(face), _, HealthTarget::One(a)) => {
            asked.insert(
                a.clone(),
                Presence::AcrossFace {
                    status_crosses: face.status_crosses,
                },
            );
            majors.insert(a.clone(), face.major);
        }
        (_, Some(Ok(o)), target) => {
            let catalog = Catalog::new(o);
            presence_doc = Some(HealthPresence {
                selector: o.selector.clone(),
                complete: o.complete,
                cut_off_during: o.cut_off_during,
                services: catalog
                    .addresses()
                    .filter(|a| catalog.has_instance(a))
                    .count(),
            });
            let mut addrs: BTreeSet<Addr> = match target {
                HealthTarget::One(a) => BTreeSet::from([a.clone()]),
                HealthTarget::All => catalog.addresses().cloned().collect(),
            };
            if matches!(target, HealthTarget::All) {
                // A status answered for an address presence did not show is
                // read too: absent from a complete read, so not asked.
                for g in [first, second].into_iter().flatten() {
                    addrs.extend(g.addresses().cloned());
                }
            }
            for a in addrs {
                let p = presence_in(o, &a);
                if let Some(m) = major_of(o, &a) {
                    majors.insert(a.clone(), m);
                }
                asked.insert(a, p);
            }
            if asked.is_empty() {
                unobservable = Some(format!(
                    "no zk2 service visible to this reader in {} (`{}`){}: a reading over an \
                     empty scope judged nothing, which is not a healthy deployment",
                    namespace_words(&obs.namespace),
                    o.selector,
                    o.incomplete_reason("ended at its timeout")
                        .map(|why| format!(" (and the read {why})"))
                        .unwrap_or_default()
                ));
            }
        }
        (_, Some(Err(e)), target) => {
            unobservable = Some(format!("the presence read could not be made: {e}"));
            if let HealthTarget::One(a) = target {
                asked.insert(a.clone(), Presence::Incomplete);
            }
        }
        (_, None, target) => {
            if let HealthTarget::One(a) = target {
                asked.insert(a.clone(), Presence::Incomplete);
            }
        }
    }
    if let Err(e) = &obs.second
        && unobservable.is_none()
    {
        unobservable = Some(format!("the reading's GET could not be made: {e}"));
    }
    let archived = match &obs.archived {
        Some(Ok(Some(a))) => Some(a),
        _ => None,
    };
    let rows: Vec<(HealthRow, Judged)> = asked
        .iter()
        .map(|(addr, presence)| {
            // Read by the major listed; with none listed, by the GET that
            // answered for it, else `health.v1`. The row names a major only
            // where one was listed, or across a face the deployment's word.
            let listed = majors.get(addr).copied();
            let major = listed
                .or_else(|| second.and_then(|g| g.answered_major(addr)))
                .or_else(|| first.and_then(|g| g.answered_major(addr)))
                .or_else(|| archived.map(|a| a.major))
                .unwrap_or(Major::V1);
            let heard = window.and_then(|w| w.heard_of(major).get(addr));
            let subscribed = window.map(|w| {
                (
                    w.status_of(major)
                        .get(addr)
                        .copied()
                        .unwrap_or(Observation::Subscribed {
                            last: None,
                            listened: w.listened,
                            complete: true,
                        }),
                    heard.and_then(|h| h.last.as_ref()),
                )
            });
            let tables = obs.tables.get(addr);
            // At the horizon of the revision read: the standard's, or
            // across a face the deployment's word (#1151).
            let horizon = obs.spec.status_horizon(major);
            let r1 = reading(
                *presence,
                Got::of_major(first, addr, major),
                None,
                &trust,
                false,
            )
            .within(horizon.clone());
            let r2 = reading(
                *presence,
                Got::of_major(second, addr, major),
                subscribed,
                &trust,
                archived.is_some(),
            )
            .within(horizon);
            let last_known = archived.map(|a| {
                let words = a.value.as_ref().map(|v| status_words(v, tables));
                LastKnownStatus {
                    archive: a.archive.to_string(),
                    level: a.value.as_ref().map_or(LevelRead::Token("deleted"), |v| {
                        level_read_of(v.level, a.major)
                    }),
                    reason: words.as_ref().map(|w| w.0.clone()).unwrap_or_default(),
                    since_ns: match a.major {
                        Major::V1 => Some(a.value.as_ref().map_or(0, |v| v.since_ns)),
                        Major::V2 => None,
                    },
                    word: words.and_then(|w| w.1),
                    since_s: a.value.as_ref().and_then(|v| v.since_s),
                    stamp: a.stamp.as_ref().map(|s| s.report()),
                    confirmed: a.confirmed,
                }
            });
            let clock = match &obs.window {
                None => answer(
                    HealthAnswerToken::NotAsked,
                    "no_window",
                    "no subscription listened: the question rests on what a subscriber hears \
                     (pass --for)",
                ),
                Some(Err(e)) => answer(
                    HealthAnswerToken::Unobservable,
                    "subscription_failed",
                    &format!("the window's subscription could not be made: {e}"),
                ),
                Some(Ok(_)) => {
                    let c = match major {
                        Major::V1 => hm::clock_ahead(
                            *presence,
                            heard.map(|h| h.sequence.as_slice()).unwrap_or_default(),
                        ),
                        Major::V2 => hv2::clock_ahead(
                            *presence,
                            heard.map(|h| h.clock.as_slice()).unwrap_or_default(),
                        ),
                    };
                    let says = match (major, c.reason) {
                        (Major::V2, hm::ClockReason::NotListed) => {
                            "its descriptor lists neither health.v1 nor health.v2"
                        }
                        (Major::V2, hm::ClockReason::TimeUnknown) => {
                            "its last read heartbeat's time was unknown: its clock reads no \
                             instant, which says nothing of ahead"
                        }
                        (Major::V2, hm::ClockReason::NothingHeard) => {
                            "no heartbeat was read in the window, whatever fault was heard: a \
                             fault is an occurrence, and answers nothing"
                        }
                        (Major::V2, hm::ClockReason::Confirmed) => {
                            "its last read heartbeat's time was an instant: its clock guard did \
                             not hold when it wrote it"
                        }
                        _ => c.reason.says(),
                    };
                    answer(token(c.answer), c.reason.as_str(), says)
                }
            };
            let got_v2 = |g: Option<&crate::bus::health::HealthGet>| {
                g.and_then(|g| g.v2.get(addr))
                    .and_then(|s| s.reading_v2.as_ref())
                    .is_some_and(hv2::Reading::is_one_instant)
            };
            let one_instant = (major == Major::V2).then(|| [got_v2(first), got_v2(second)]);
            row(
                addr,
                HowRead {
                    profile: listed.map(Major::iface),
                    major,
                    tables,
                    one_instant,
                },
                &[r1, r2],
                Got::of_major(second, addr, major),
                heard,
                last_known,
                clock,
            )
        })
        .collect();
    let (services, judged): (Vec<HealthRow>, Vec<Judged>) = rows.into_iter().unzip();
    let rollup = {
        let r = hm::rollup(judged);
        HealthRollup {
            worst: r.worst.map(level_token),
            healthy: r.healthy,
            unhealthy: r.unhealthy,
            stale: r.stale,
            unobservable: r.unobservable,
            not_asked: r.not_asked,
        }
    };
    let apart_s = match (first, second) {
        (Some(a), Some(b)) => b
            .read_at
            .duration_since(a.read_at)
            .unwrap_or_default()
            .as_secs_f64(),
        _ => obs.spec.grace.as_secs_f64(),
    };
    HealthReport {
        namespace: obs.namespace.clone(),
        service: match &obs.target {
            HealthTarget::One(a) => Some(a.to_string()),
            HealthTarget::All => None,
        },
        presence: presence_doc,
        face: obs.spec.face.map(|f| HealthFace {
            status_crosses: f.status_crosses,
        }),
        window_s: window.map(|w| w.listened.as_secs_f64()),
        apart_s,
        clock: trust.ground(),
        asked: obs.asked(),
        services,
        rollup,
        unobservable,
    }
}

fn namespace_words(ns: &str) -> String {
    if ns.is_empty() {
        "the bus root".to_owned()
    } else {
        format!("namespace {ns:?}")
    }
}

/// How one row was read: the major listed (the row's `profile`), the
/// major read by, the instance's tables, and under `health.v2` whether
/// each reading was one instant.
struct HowRead<'a> {
    profile: Option<&'static str>,
    major: Major,
    tables: Option<&'a hv2::InstanceTables>,
    one_instant: Option<[bool; 2]>,
}

/// A status's reason as the report shows it: `health.v1`'s text, or a
/// `health.v2` word's name, its number where no table in hand names it,
/// and nothing for no word or a word no table names, with the word.
fn status_words(
    v: &crate::bus::health::StatusValue,
    tables: Option<&hv2::InstanceTables>,
) -> (String, Option<crate::report::WordSeen>) {
    word_text(&v.reason, v.word, hv2::Kind::Reason, tables)
}

fn word_text(
    text: &str,
    word: Option<hv2::Word>,
    kind: hv2::Kind,
    tables: Option<&hv2::InstanceTables>,
) -> (String, Option<crate::report::WordSeen>) {
    match word {
        None => (text.to_owned(), None),
        Some(w) => {
            let seen = word_seen(kind, w, tables);
            let text = match (&seen.name, seen.read) {
                (Some(n), _) => n.clone(),
                (None, "number") => seen.number.to_string(),
                _ => String::new(),
            };
            (text, Some(seen))
        }
    }
}

/// One service's row: the last reading's verdict, every reading's, §5's
/// other questions, and what was read as it was read.
fn row(
    addr: &Addr,
    how: HowRead<'_>,
    readings: &[ServiceReading],
    last_got: Got<'_>,
    heard: Option<&ServiceHeard>,
    last_known: Option<LastKnownStatus>,
    clock: HealthAnswer,
) -> (HealthRow, Judged) {
    let HowRead {
        profile,
        major,
        tables,
        one_instant,
    } = how;
    let judged: Vec<Judged> = readings.iter().map(ServiceReading::judged).collect();
    let agreements: Vec<Agreement> = readings.iter().map(ServiceReading::agreement).collect();
    let last = *judged.last().expect("two readings");
    let since_ns = |v: &crate::bus::health::StatusValue| match major {
        Major::V1 => Some(v.since_ns),
        Major::V2 => None,
    };
    let seen = |r: &crate::bus::health::Replied<crate::bus::health::StatusValue>, via| {
        r.value.as_ref().map(|v| {
            let (reason, word) = status_words(v, tables);
            StatusSeen {
                level: level_read_of(v.level, major),
                reason,
                since_ns: since_ns(v),
                word,
                since_s: v.since_s,
                stamp: r.stamp.as_ref().map(|s| s.report()),
                via,
            }
        })
    };
    let (status, checks) = match last_got {
        Got::Answered { got, .. } => {
            let status = got.status.as_ref().and_then(|r| seen(r, StatusVia::Get));
            let checks = got
                .checks
                .iter()
                .filter_map(|(name, c)| {
                    c.value.as_ref().map(|v| {
                        let (detail, word) =
                            word_text(&v.detail, v.word, hv2::Kind::Detail, tables);
                        CheckSeen {
                            check: zenkey_model::slug::chunk_unslug(name)
                                .unwrap_or_else(|| name.clone()),
                            level: level_read_of(v.level, major),
                            detail,
                            word,
                            at_s: v.at_s,
                        }
                    })
                })
                .collect();
            (status, checks)
        }
        _ => (None, Vec::new()),
    };
    let status = status.or_else(|| {
        heard
            .and_then(|h| h.last.as_ref())
            .and_then(|r| seen(r, StatusVia::Subscription))
    });
    let agrees = match one_instant {
        Some(instants) => agreement_over_v2(&agreements, &instants[..agreements.len()]),
        None => agreement_over(&agreements),
    };
    let row = HealthRow {
        address: addr.to_string(),
        profile,
        verdict: verdict_token(last.verdict),
        reason: last.reason.as_str().to_owned(),
        level: last.level.map(level_token),
        // The reasons are one set for both majors (v2 §2.11); a rule's
        // place is the major's, and "not listed" is neither's.
        says: match (last.reason, major) {
            (hm::Reason::NotListed, _) => {
                "its descriptor lists neither health.v1 nor health.v2".to_owned()
            }
            (r, Major::V2) => r.says().replace("health.v1 §", "health.v2 §"),
            (r, Major::V1) => r.says().to_owned(),
        },
        readings: judged
            .iter()
            .enumerate()
            .map(|(i, j)| HealthReadingVerdict {
                verdict: verdict_token(j.verdict),
                reason: j.reason.as_str().to_owned(),
                level: j.level.map(level_token),
                one_instant: one_instant.map(|o| o[i]),
            })
            .collect(),
        agrees,
        clock_ahead: clock,
        status,
        checks,
        last_known,
        faults: heard.map_or(0, |h| h.faults),
        last_fault: heard
            .and_then(|h| h.last_fault.as_ref())
            .map(|f| match f.word {
                None => FaultSeen {
                    class: hm::code(&f.code).as_str(),
                    code: f.code.clone(),
                    level: level_read_of(f.level, major),
                    detail: f.detail.clone(),
                    word: None,
                    argument: None,
                    at_s: None,
                },
                Some(_) => {
                    let (code, word) = word_text(&f.code, f.word, hv2::Kind::Code, tables);
                    FaultSeen {
                        class: word.as_ref().map_or("undecodable", |w| w.read),
                        code,
                        level: level_read_of(f.level, major),
                        detail: String::new(),
                        word,
                        argument: f.argument.map(hv2::Argument::to_json),
                        at_s: f.at_s,
                    }
                }
            }),
    };
    (row, last)
}

#[cfg(test)]
mod tests {
    //! Each reading built from values, and every verdict's poles: healthy,
    //! unhealthy, stale, unobservable and not asked, the break of §2.2 from
    //! two readings, the clock, and an absent owner's archived status shown
    //! last-known.

    use super::*;
    use crate::bus::health::{
        AcrossFace as Face, ArchivedStatus, CheckValue, HealthGet, HealthSpec, HealthWindow,
        Replied, ServiceGot, Stamped, StatusValue,
    };
    use crate::model::catalog::Observed;
    use crate::report::{ClockGround, HealthLevel, HealthVerdict};
    use std::time::{Duration, SystemTime};
    use zenkey_model::freshness::Last;
    use zenkey_model::health::{Level, Read};

    const SVC: &str = "lab/svc";

    fn addr(s: &str) -> Addr {
        s.parse().unwrap()
    }

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn status(level: Read, age_s: u64) -> Replied<StatusValue> {
        Replied {
            value: Some(StatusValue {
                word: None,
                since_s: None,
                level,
                reason: "r".into(),
                since_ns: 1,
            }),
            stamp: Some(Stamped {
                time: now() - Duration::from_secs(age_s),
                clock: "ab12".into(),
            }),
        }
    }

    fn check(level: Read) -> Replied<CheckValue> {
        Replied {
            value: Some(CheckValue {
                word: None,
                at_s: None,
                level,
                detail: "d".into(),
            }),
            stamp: None,
        }
    }

    fn got(s: Option<Replied<StatusValue>>, checks: &[(&str, Read)]) -> ServiceGot {
        ServiceGot {
            reading_v2: None,
            status: s,
            checks: checks
                .iter()
                .map(|(n, l)| ((*n).to_owned(), check(*l)))
                .collect(),
        }
    }

    fn get(services: Vec<(&str, ServiceGot)>) -> HealthGet {
        HealthGet {
            v2: BTreeMap::new(),
            selector: "zk2/*/*/health.v1/state/**".into(),
            read_at: now(),
            complete: true,
            services: services.into_iter().map(|(a, g)| (addr(a), g)).collect(),
        }
    }

    fn presence(keys: &[&str], listed: Option<bool>) -> Observed {
        let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
        let mut o = Observed::from_keys("zk2/*/*/@zk/**", &keys, true);
        let mut ds = BTreeMap::new();
        for k in &o.tokens {
            if let zenkey_model::grammar::ZkKey::Instance { addr, instance } = k {
                let interfaces = match listed {
                    Some(token) => serde_json::json!([{
                        "iface": "health.v1",
                        "contract": format!("sha256:{}", zenkey::health::FINGERPRINT),
                        "minor": 0, "token": token,
                    }]),
                    None => serde_json::json!([]),
                };
                let d: zenkey_model::descriptor::Descriptor =
                    serde_json::from_value(serde_json::json!({
                        "format": "zk2-descriptor/0.1",
                        "service": addr.to_string(),
                        "instance": instance.to_string(),
                        "interfaces": interfaces,
                        "meta": {"zid": "ab12"},
                    }))
                    .unwrap();
                ds.insert(
                    (addr.clone(), instance.clone()),
                    crate::model::catalog::DescriptorRead::Served(Box::new(d)),
                );
            }
        }
        o.descriptors = Some(ds);
        o
    }

    fn spec(synced: bool) -> HealthSpec {
        HealthSpec {
            contracts: Default::default(),
            timeout: Duration::from_secs(1),
            window: None,
            grace: Duration::from_secs(2),
            clocks_synced: synced,
            face: None,
            catalogs: Default::default(),
        }
    }

    fn obs(target: HealthTarget, p: Observed, both: HealthGet) -> HealthObservation {
        HealthObservation {
            tables: BTreeMap::new(),
            namespace: "acme".into(),
            target,
            spec: spec(true),
            presence: Some(Ok(p)),
            first: Ok(both.clone()),
            second: Ok(both),
            window: None,
            archived: None,
        }
    }

    const INST: &str = "zk2/lab/svc/@zk/instance/000000000000000a";

    fn one(r: &HealthReport) -> &HealthRow {
        assert_eq!(r.services.len(), 1, "{r:#?}");
        &r.services[0]
    }

    #[test]
    fn a_fresh_status_is_healthy_or_unhealthy_at_its_level() {
        let ok = lv(Level::Ok);
        let o = obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(false)),
            get(vec![(SVC, got(Some(status(ok, 5)), &[("disk", ok)]))]),
        );
        let r = judge(&o);
        let row = one(&r);
        assert_eq!(row.verdict, HealthVerdict::Healthy);
        assert_eq!(row.level, Some(HealthLevel::Ok));
        assert_eq!(row.agrees.answer, HealthAnswerToken::Yes);
        assert_eq!(row.clock_ahead.answer, HealthAnswerToken::NotAsked);
        assert_eq!(row.status.as_ref().unwrap().via, StatusVia::Get);
        assert_eq!(row.checks.len(), 1);
        assert_eq!(r.clock, ClockGround::DeploymentWord);
        assert_eq!(r.rollup.worst, Some(HealthLevel::Ok));
        assert_eq!(crate::report::judgement_exit_code(&r.judgement()), 0);

        let degraded = lv(Level::Degraded);
        let o = obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(true)),
            get(vec![(SVC, got(Some(status(degraded, 5)), &[]))]),
        );
        let r = judge(&o);
        assert_eq!(
            (one(&r).verdict, one(&r).reason.as_str()),
            (HealthVerdict::Unhealthy, "degraded")
        );
        assert_eq!(crate::report::judgement_exit_code(&r.judgement()), 1);
    }

    #[test]
    fn a_status_better_than_a_check_is_a_break_only_in_both_readings() {
        let (ok, failed) = (lv(Level::Ok), lv(Level::Failed));
        let liar = get(vec![(SVC, got(Some(status(ok, 5)), &[("disk", failed)]))]);
        let o = obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(true)),
            liar.clone(),
        );
        let r = judge(&o);
        let row = one(&r);
        assert_eq!(
            (row.verdict, row.reason.as_str(), row.level),
            (
                HealthVerdict::Unhealthy,
                "inconsistent",
                Some(HealthLevel::Failed)
            )
        );
        assert!(row.readings.iter().all(|x| x.reason == "inconsistent"));
        assert_eq!(row.agrees.answer, HealthAnswerToken::No);

        // The first reading agreed: one reading's break is unobservable.
        let mut o = o;
        o.first = Ok(get(vec![(SVC, got(Some(status(ok, 5)), &[("disk", ok)]))]));
        let r = judge(&o);
        assert_eq!(one(&r).agrees.answer, HealthAnswerToken::Unobservable);
        assert_eq!(one(&r).agrees.reason, "one_reading");

        // Worse than its checks is allowed.
        let frank = get(vec![(SVC, got(Some(status(failed, 5)), &[("disk", ok)]))]);
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(true)),
            frank,
        ));
        assert_eq!(one(&r).reason, "failed");
        assert_eq!(one(&r).agrees.answer, HealthAnswerToken::Yes);
    }

    #[test]
    fn stale_and_unobservable_are_never_a_level() {
        let ok = lv(Level::Ok);
        let old = get(vec![(SVC, got(Some(status(ok, 70)), &[]))]);
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(true)),
            old.clone(),
        ));
        assert_eq!(
            (one(&r).verdict, one(&r).level),
            (HealthVerdict::Stale, None)
        );
        // No word and no window: the reply's age is unobservable.
        let mut o = obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(true)),
            old,
        );
        o.spec.clocks_synced = false;
        let r = judge(&o);
        assert_eq!(one(&r).verdict, HealthVerdict::Unobservable);
        assert_eq!(one(&r).reason, "clock_untrusted");
        assert_eq!(r.clock, ClockGround::None);
        assert_eq!(crate::report::judgement_exit_code(&r.judgement()), 2);
        // A GET that answered nothing for it: silent.
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence(&[INST], Some(true)),
            get(vec![]),
        ));
        assert_eq!(one(&r).reason, "silent");
        assert_eq!(one(&r).agrees.reason, "silent");
    }

    #[test]
    fn found_by_descriptor_and_never_by_token() {
        let ok = lv(Level::Ok);
        let both = get(vec![(SVC, got(Some(status(ok, 5)), &[]))]);
        let r = judge(&obs(
            HealthTarget::All,
            presence(&[INST], Some(false)),
            both.clone(),
        ));
        assert_eq!(one(&r).verdict, HealthVerdict::Healthy, "tokenless");
        let r = judge(&obs(HealthTarget::All, presence(&[INST], None), both));
        assert_eq!(
            (one(&r).verdict, one(&r).reason.as_str()),
            (HealthVerdict::NotAsked, "not_listed")
        );
        assert_eq!(crate::report::judgement_exit_code(&r.judgement()), 2);
        let r = judge(&obs(HealthTarget::All, presence(&[], None), get(vec![])));
        assert!(r.unobservable.as_deref().unwrap().contains("empty scope"));
    }

    #[test]
    fn an_absent_owner_is_not_asked_and_its_archive_last_known() {
        let mut o = obs(
            HealthTarget::One(addr(SVC)),
            presence(&[], None),
            get(vec![]),
        );
        o.archived = Some(Ok(Some(ArchivedStatus {
            major: Major::V1,
            archive: addr("lab/archive"),
            value: Some(StatusValue {
                word: None,
                since_s: None,
                level: lv(Level::Degraded),
                reason: "upstream lost".into(),
                since_ns: 3,
            }),
            stamp: None,
            confirmed: true,
        })));
        let r = judge(&o);
        let row = one(&r);
        assert_eq!(
            (row.verdict, row.reason.as_str(), row.level),
            (HealthVerdict::NotAsked, "absent", None)
        );
        let lk = row.last_known.as_ref().expect("the archived status");
        assert_eq!(lk.level, LevelRead::Token("degraded"));
        assert!(lk.confirmed);
        assert_eq!(r.rollup.not_asked, 1);
    }

    #[test]
    fn across_a_face_nothing_crossed_is_unobservable_and_a_closed_face_says_so() {
        let w = |last: Option<Duration>, sequence: Vec<hm::Heard>| HealthWindow {
            heard_v2: BTreeMap::new(),
            status_v2: BTreeMap::new(),
            selectors: vec![],
            listened: Duration::from_secs(65),
            heard: BTreeMap::from([(
                addr(SVC),
                ServiceHeard {
                    last: last.map(|_| status(lv(Level::Ok), 1)),
                    sequence,
                    ..ServiceHeard::default()
                },
            )]),
            status: BTreeMap::from([(
                addr(SVC),
                Observation::Subscribed {
                    last: last.map(Last::Put),
                    listened: Duration::from_secs(65),
                    complete: true,
                },
            )]),
            clocks: BTreeMap::new(),
        };
        let face = |crosses: bool, window: HealthWindow| HealthObservation {
            tables: BTreeMap::new(),
            namespace: String::new(),
            target: HealthTarget::One(addr(SVC)),
            spec: HealthSpec {
                window: Some(Duration::from_secs(65)),
                face: Some(Face {
                    major: Major::V1,
                    status_crosses: crosses,
                }),
                ..spec(false)
            },
            presence: None,
            first: Ok(get(vec![])),
            second: Ok(get(vec![])),
            window: Some(Ok(window)),
            archived: None,
        };
        let r = judge(&face(true, w(None, vec![])));
        assert_eq!(one(&r).reason, "nothing_crossed");
        assert_eq!(one(&r).clock_ahead.reason, "nothing_heard");
        let r = judge(&face(
            true,
            w(Some(Duration::from_secs(10)), vec![hm::Heard::Heartbeat]),
        ));
        assert_eq!(one(&r).verdict, HealthVerdict::Healthy);
        assert_eq!(
            one(&r).status.as_ref().unwrap().via,
            StatusVia::Subscription
        );
        assert_eq!(one(&r).clock_ahead.answer, HealthAnswerToken::No);
        let r = judge(&face(
            true,
            w(Some(Duration::from_secs(64)), vec![hm::Heard::Heartbeat]),
        ));
        assert_eq!(one(&r).verdict, HealthVerdict::Stale);
        let r = judge(&face(false, w(None, vec![hm::Heard::ClockAhead])));
        assert_eq!(one(&r).reason, "face_closed");
        assert_eq!(one(&r).clock_ahead.answer, HealthAnswerToken::Yes);
        assert!(r.presence.is_none() && r.face.is_some());
    }

    fn lv(l: Level) -> Read {
        Read::Level(l)
    }

    // ── health.v2 (#986's Q2) ───────────────────────────────────────────

    /// A presence read whose one instance lists `ifaces`, each by the
    /// standard contract's fingerprint of its major.
    fn presence_listing(ifaces: &[&str]) -> Observed {
        let mut o = presence(&[INST], Some(true));
        let entries: Vec<serde_json::Value> = ifaces
            .iter()
            .map(|i| {
                let fp = match *i {
                    "health.v2" => hv2::FINGERPRINT,
                    _ => zenkey::health::FINGERPRINT,
                };
                serde_json::json!({
                    "iface": i, "contract": format!("sha256:{fp}"), "minor": 0, "token": true,
                })
            })
            .collect();
        for read in o.descriptors.as_mut().unwrap().values_mut() {
            let crate::model::catalog::DescriptorRead::Served(d) = read else {
                unreachable!()
            };
            let mut v = serde_json::to_value(&**d).unwrap();
            v["interfaces"] = serde_json::Value::Array(entries.clone());
            **d = serde_json::from_value(v).unwrap();
        }
        o
    }

    fn word(n: u8) -> hv2::Word {
        hv2::Word::new(n).unwrap_or(hv2::Word::OUT_OF_RANGE)
    }

    /// A `health.v2` status at `level` with the reason `reason`, its
    /// confirmation `age_s` old.
    fn status_v2(level: Read, reason: u8, age_s: u64) -> Replied<StatusValue> {
        let mut r = status(level, age_s);
        let v = r.value.as_mut().unwrap();
        v.reason = String::new();
        v.since_ns = 0;
        v.word = Some(word(reason));
        v.since_s = Some(1_794_062_759);
        r
    }

    fn check_v2(level: Read, detail: u8) -> Replied<CheckValue> {
        let mut r = check(level);
        let v = r.value.as_mut().unwrap();
        v.detail = String::new();
        v.word = Some(word(detail));
        r
    }

    /// One owner's answer read as a late joiner reads it (v2 §2.2): a
    /// status at `status` and a check `disk` at `disk`, the checks' snapshot
    /// at `checks_asof`, the status's at 5.
    fn reading_v2(status: Level, disk: Level, checks_asof: u32) -> hv2::Reading {
        use hv2::Packed as _;
        use zenkey_model::collection::{Confirmation, Entry, Page, Time};
        use zenkey_model::wire::{Run, Seq, Tag};
        let page = |coll, asof, id: &str, seq, record: Vec<u8>, c| {
            Page {
                tag: Tag::new(3).unwrap(),
                run: Run::U16(7),
                asof: Seq::new(asof).unwrap(),
                time: Time::Known { s: 100, ms: 0 },
                index: 0,
                more: false,
                coll,
                live: vec![Entry {
                    id: id.to_owned(),
                    run: None,
                    seq: Seq::new(seq).unwrap(),
                    confirmation: c,
                    record,
                }],
                tombs: vec![],
            }
            .write()
            .unwrap()
        };
        let s = hv2::Status {
            level: Some(status),
            reason: word(2),
            since: None,
        };
        let c = hv2::Check {
            level: Some(disk),
            detail: word(0),
            at: None,
        };
        let sp = page(1, 5, "", 4, s.write(0).unwrap().bytes, Confirmation::AtTime);
        let cp = page(
            0,
            checks_asof,
            "disk",
            3,
            c.write(0).unwrap().bytes,
            Confirmation::None,
        );
        hv2::Reading::of(
            [
                ("zk2/lab/svc/health.v2/state/status", sp.as_slice()),
                ("zk2/lab/svc/health.v2/state/checks", cp.as_slice()),
            ],
            0,
        )
    }

    fn get_v2(services: Vec<(&str, ServiceGot)>) -> HealthGet {
        HealthGet {
            v2: services.into_iter().map(|(a, g)| (addr(a), g)).collect(),
            ..get(vec![])
        }
    }

    /// v2 §2.11, §2.12, §2.10: a descriptor that lists `health.v2` is read
    /// by it, and one that lists both too (`profile`); its words are shown
    /// by the instance's tables, its number where none in hand names it, a
    /// word no table names as that, and a word plays no part in the verdict.
    #[test]
    fn a_health_v2_service_is_read_by_its_major_and_its_words_by_its_tables() {
        let degraded = lv(Level::Degraded);
        let answered = get_v2(vec![(
            SVC,
            ServiceGot {
                status: Some(status_v2(degraded, 32, 5)),
                checks: [("qdisc".to_owned(), check_v2(degraded, 3))].into(),
                reading_v2: None,
            },
        )]);
        for listed in [&["health.v2"][..], &["health.v1", "health.v2"]] {
            let mut o = obs(
                HealthTarget::One(addr(SVC)),
                presence_listing(listed),
                answered.clone(),
            );
            o.tables.insert(
                addr(SVC),
                hv2::Tables::of_instance(&[hv2::Tables {
                    reasons: Some(vec!["tc_netns_gone".into()]),
                    ..Default::default()
                }]),
            );
            let r = judge(&o);
            let row = one(&r);
            assert_eq!(row.profile, Some("health.v2"), "{listed:?}");
            assert_eq!(
                (row.verdict, row.reason.as_str()),
                (HealthVerdict::Unhealthy, "degraded")
            );
            let st = row.status.as_ref().unwrap();
            assert_eq!(st.reason, "tc_netns_gone");
            let w = st.word.as_ref().unwrap();
            assert_eq!((w.number, w.read), (32, "service"));
            assert_eq!(st.since_ns, None, "no nanoseconds under health.v2");
            assert_eq!(st.since_s, Some(1_794_062_759));
            assert_eq!(row.checks[0].detail, "timeout", "the profile's detail 3");
            assert_eq!(row.readings[1].one_instant, Some(false));
        }
        // No table in hand: the number, never another revision's name.
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence_listing(&["health.v2"]),
            answered,
        ));
        let st = one(&r).status.clone().unwrap();
        assert_eq!(
            (st.reason.as_str(), st.word.unwrap().read),
            ("32", "number")
        );
        assert_eq!(one(&r).verdict, HealthVerdict::Unhealthy);
        // A word no table names: never a name, never no word.
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence_listing(&["health.v2"]),
            get_v2(vec![(SVC, got_v2(Some(status_v2(degraded, 63, 5)), None))]),
        ));
        let st = one(&r).status.clone().unwrap();
        assert_eq!(st.reason, "");
        assert_eq!(st.word.as_ref().unwrap().read, "out_of_range");
        assert_eq!(st.word.unwrap().words(), "a word no table names");
        // A `health.v1` owner beside it is read by its own major.
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence_listing(&["health.v1"]),
            get(vec![(SVC, got(Some(status(lv(Level::Ok), 5)), &[]))]),
        ));
        assert_eq!(one(&r).profile, Some("health.v1"));
        assert_eq!(one(&r).verdict, HealthVerdict::Healthy);
        // A `health.v2` reading of an unknown status reads `unknown`.
        let r = judge(&obs(
            HealthTarget::One(addr(SVC)),
            presence_listing(&["health.v2"]),
            get_v2(vec![(
                SVC,
                got_v2(Some(status_v2(Read::Unspecified, 1, 5)), None),
            )]),
        ));
        assert_eq!(one(&r).reason, "unknown_level");
        assert_eq!(
            one(&r).status.as_ref().unwrap().level,
            LevelRead::Token("unknown")
        );
    }

    fn got_v2(s: Option<Replied<StatusValue>>, reading: Option<hv2::Reading>) -> ServiceGot {
        ServiceGot {
            status: s,
            checks: BTreeMap::new(),
            reading_v2: reading,
        }
    }

    /// v2 §2.2, scenarios §7: a break of §2.2 is reported from one reading
    /// that is one instant (`lab/liar`); a reading that is not one instant
    /// reports none alone (`lab/two`'s first), and two such readings a grace
    /// apart report it, as v1 says.
    #[test]
    fn a_health_v2_break_is_reported_from_one_reading_that_is_one_instant() {
        let (ok, failed) = (lv(Level::Ok), lv(Level::Failed));
        let answer = |instant: bool, break_: bool| {
            let disk = if break_ { failed } else { ok };
            let asof = if instant { 5 } else { 7 };
            get_v2(vec![(
                SVC,
                ServiceGot {
                    status: Some(status_v2(ok, 2, 5)),
                    checks: [("disk".to_owned(), check_v2(disk, 0))].into(),
                    reading_v2: Some(reading_v2(
                        Level::Ok,
                        if break_ { Level::Failed } else { Level::Ok },
                        asof,
                    )),
                },
            )])
        };
        let p = || presence_listing(&["health.v2"]);
        let at = |first, second| {
            let mut o = obs(HealthTarget::One(addr(SVC)), p(), first);
            o.second = Ok(second);
            judge(&o)
        };
        // lab/liar: one instant, the break: the finding from one reading.
        let r = at(answer(true, false), answer(true, true));
        let row = one(&r);
        assert_eq!(
            (row.verdict, row.reason.as_str(), row.level),
            (
                HealthVerdict::Unhealthy,
                "inconsistent",
                Some(HealthLevel::Failed)
            )
        );
        assert_eq!(row.agrees.answer, HealthAnswerToken::No);
        assert!(
            row.agrees.says.contains("one instant"),
            "{}",
            row.agrees.says
        );
        assert_eq!(
            row.readings
                .iter()
                .map(|r| r.one_instant)
                .collect::<Vec<_>>(),
            [Some(true), Some(true)]
        );
        // lab/two: its last reading not one instant, reported from it
        // alone never; from two readings that each show it, as v1 says.
        let r = at(answer(true, false), answer(false, true));
        assert_eq!(one(&r).agrees.answer, HealthAnswerToken::Unobservable);
        assert_eq!(one(&r).agrees.reason, "one_reading");
        assert_eq!(one(&r).reason, "inconsistent", "read at the check's level");
        let r = at(answer(false, true), answer(false, true));
        assert_eq!(one(&r).agrees.answer, HealthAnswerToken::No);
        assert!(one(&r).agrees.says.contains("each of 2 readings"));
        assert!(one(&r).agrees.says.contains("health.v2 §2.2"));
        // lab/flip: one instant, no break.
        let r = at(answer(true, false), answer(true, false));
        assert_eq!(one(&r).agrees.answer, HealthAnswerToken::Yes);
    }

    /// v2 §2.5: the last read heartbeat answers the clock question, its
    /// time ahead yes, an instant no, unknown or none unobservable; a fault
    /// answers nothing, across a face too, where the deployment's word names
    /// the major.
    #[test]
    fn the_health_v2_clock_question_is_the_last_read_heartbeats() {
        use zenkey_model::heartbeat::Time;
        let window = |clock: Vec<hv2::ClockHeard>| HealthWindow {
            selectors: vec![],
            listened: Duration::from_secs(35),
            heard: BTreeMap::new(),
            status: BTreeMap::new(),
            heard_v2: BTreeMap::from([(
                addr(SVC),
                ServiceHeard {
                    clock,
                    ..ServiceHeard::default()
                },
            )]),
            status_v2: BTreeMap::new(),
            clocks: BTreeMap::new(),
        };
        let clock = |face: bool, clock: Vec<hv2::ClockHeard>| {
            let mut o = obs(
                HealthTarget::One(addr(SVC)),
                presence_listing(&["health.v2"]),
                get(vec![]),
            );
            o.window = Some(Ok(window(clock)));
            if face {
                o.presence = None;
                o.spec.face = Some(Face {
                    status_crosses: false,
                    major: Major::V2,
                });
            }
            let r = judge(&o);
            let c = one(&r).clock_ahead.clone();
            (c.answer, c.reason)
        };
        let at = Time::At { secs: 1, ms: 0 };
        for face in [false, true] {
            assert_eq!(
                clock(face, vec![hv2::ClockHeard::Heartbeat(Time::Ahead)]),
                (HealthAnswerToken::Yes, "ahead".to_owned())
            );
            assert_eq!(
                clock(face, vec![hv2::ClockHeard::ClockAhead]),
                (HealthAnswerToken::Unobservable, "nothing_heard".to_owned()),
                "a fault answers nothing"
            );
            assert_eq!(
                clock(
                    face,
                    vec![
                        hv2::ClockHeard::Heartbeat(Time::Ahead),
                        hv2::ClockHeard::ClockAhead,
                        hv2::ClockHeard::Heartbeat(at)
                    ]
                ),
                (HealthAnswerToken::No, "confirmed".to_owned())
            );
            assert_eq!(
                clock(face, vec![hv2::ClockHeard::Heartbeat(Time::Unknown)]),
                (HealthAnswerToken::Unobservable, "time_unknown".to_owned())
            );
        }
    }
}
