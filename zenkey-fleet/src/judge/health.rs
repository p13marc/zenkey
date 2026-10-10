//! `health.v1` (#721, PF): a health reading judged, every verdict decided
//! from values (`spec/profiles/health/v1.md`).
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
use zenkey_model::health::{self as hm, Agreement, Judged, Presence};

use crate::bus::health::{HealthObservation, HealthTarget, ServiceHeard};
use crate::model::catalog::Catalog;
use crate::model::health::{
    Got, ServiceReading, Trust, agreement_over, answer, level_read, level_token, presence_in,
    reading, token, verdict_token,
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
    let mut presence_doc = None;
    match (&obs.spec.face, &obs.presence, &obs.target) {
        (Some(face), _, HealthTarget::One(a)) => {
            asked.insert(
                a.clone(),
                Presence::AcrossFace {
                    status_crosses: face.status_crosses,
                },
            );
        }
        (_, Some(Ok(o)), target) => {
            let catalog = Catalog::new(o);
            presence_doc = Some(HealthPresence {
                selector: o.selector.clone(),
                complete: o.complete,
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
                    addrs.extend(g.services.keys().cloned());
                }
            }
            for a in addrs {
                let p = presence_in(o, &a);
                asked.insert(a, p);
            }
            if asked.is_empty() {
                unobservable = Some(format!(
                    "no zk2 service visible to this reader in {} (`{}`){}: a reading over an \
                     empty scope judged nothing, which is not a healthy deployment",
                    namespace_words(&obs.namespace),
                    o.selector,
                    if o.complete {
                        ""
                    } else {
                        " (and the read ended at its timeout)"
                    }
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
            let heard = window.and_then(|w| w.heard.get(addr));
            let subscribed = window.map(|w| {
                (
                    w.status
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
            let r1 = reading(*presence, Got::of(first, addr), None, &trust, false);
            let r2 = reading(
                *presence,
                Got::of(second, addr),
                subscribed,
                &trust,
                archived.is_some(),
            );
            let last_known = archived.map(|a| LastKnownStatus {
                archive: a.archive.to_string(),
                level: a
                    .value
                    .as_ref()
                    .map_or(LevelRead::Token("deleted"), |v| level_read(v.level)),
                reason: a
                    .value
                    .as_ref()
                    .map(|v| v.reason.clone())
                    .unwrap_or_default(),
                since_ns: a.value.as_ref().map_or(0, |v| v.since_ns),
                stamp: a.stamp.as_ref().map(|s| s.report()),
                confirmed: a.confirmed,
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
                    let sequence = heard.map(|h| h.sequence.as_slice()).unwrap_or_default();
                    let c = hm::clock_ahead(*presence, sequence);
                    answer(token(c.answer), c.reason.as_str(), c.reason.says())
                }
            };
            row(
                addr,
                &[r1, r2],
                Got::of(second, addr),
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

/// One service's row: the last reading's verdict, every reading's, §5's
/// other questions, and what was read as it was read.
fn row(
    addr: &Addr,
    readings: &[ServiceReading],
    last_got: Got<'_>,
    heard: Option<&ServiceHeard>,
    last_known: Option<LastKnownStatus>,
    clock: HealthAnswer,
) -> (HealthRow, Judged) {
    let judged: Vec<Judged> = readings.iter().map(ServiceReading::judged).collect();
    let agreements: Vec<Agreement> = readings.iter().map(ServiceReading::agreement).collect();
    let last = *judged.last().expect("two readings");
    let (status, checks) = match last_got {
        Got::Answered { got, .. } => {
            let status = got.status.as_ref().and_then(|r| {
                r.value.as_ref().map(|v| StatusSeen {
                    level: level_read(v.level),
                    reason: v.reason.clone(),
                    since_ns: v.since_ns,
                    stamp: r.stamp.as_ref().map(|s| s.report()),
                    via: StatusVia::Get,
                })
            });
            let checks = got
                .checks
                .iter()
                .filter_map(|(name, c)| {
                    c.value.as_ref().map(|v| CheckSeen {
                        check: zenkey_model::slug::chunk_unslug(name)
                            .unwrap_or_else(|| name.clone()),
                        level: level_read(v.level),
                        detail: v.detail.clone(),
                    })
                })
                .collect();
            (status, checks)
        }
        _ => (None, Vec::new()),
    };
    let status = status.or_else(|| {
        heard.and_then(|h| h.last.as_ref()).and_then(|r| {
            r.value.as_ref().map(|v| StatusSeen {
                level: level_read(v.level),
                reason: v.reason.clone(),
                since_ns: v.since_ns,
                stamp: r.stamp.as_ref().map(|s| s.report()),
                via: StatusVia::Subscription,
            })
        })
    });
    let row = HealthRow {
        address: addr.to_string(),
        verdict: verdict_token(last.verdict),
        reason: last.reason.as_str().to_owned(),
        level: last.level.map(level_token),
        says: last.reason.says().to_owned(),
        readings: judged
            .iter()
            .map(|j| HealthReadingVerdict {
                verdict: verdict_token(j.verdict),
                reason: j.reason.as_str().to_owned(),
                level: j.level.map(level_token),
            })
            .collect(),
        agrees: agreement_over(&agreements),
        clock_ahead: clock,
        status,
        checks,
        last_known,
        faults: heard.map_or(0, |h| h.faults),
        last_fault: heard
            .and_then(|h| h.last_fault.as_ref())
            .map(|f| FaultSeen {
                class: hm::code(&f.code).as_str(),
                code: f.code.clone(),
                level: level_read(f.level),
                detail: f.detail.clone(),
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
                level,
                detail: "d".into(),
            }),
            stamp: None,
        }
    }

    fn got(s: Option<Replied<StatusValue>>, checks: &[(&str, Read)]) -> ServiceGot {
        ServiceGot {
            status: s,
            checks: checks
                .iter()
                .map(|(n, l)| ((*n).to_owned(), check(*l)))
                .collect(),
        }
    }

    fn get(services: Vec<(&str, ServiceGot)>) -> HealthGet {
        HealthGet {
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
            timeout: Duration::from_secs(1),
            window: None,
            grace: Duration::from_secs(2),
            clocks_synced: synced,
            face: None,
        }
    }

    fn obs(target: HealthTarget, p: Observed, both: HealthGet) -> HealthObservation {
        HealthObservation {
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
            archive: addr("lab/archive"),
            value: Some(StatusValue {
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
            namespace: String::new(),
            target: HealthTarget::One(addr(SVC)),
            spec: HealthSpec {
                window: Some(Duration::from_secs(65)),
                face: Some(Face {
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
            w(Some(Duration::from_secs(10)), vec![hm::Heard::Status]),
        ));
        assert_eq!(one(&r).verdict, HealthVerdict::Healthy);
        assert_eq!(
            one(&r).status.as_ref().unwrap().via,
            StatusVia::Subscription
        );
        assert_eq!(one(&r).clock_ahead.answer, HealthAnswerToken::No);
        let r = judge(&face(
            true,
            w(Some(Duration::from_secs(64)), vec![hm::Heard::Status]),
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
}
