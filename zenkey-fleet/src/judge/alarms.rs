//! `alarms.v1` (#800, AC): an alarms reading judged, every verdict decided
//! from values (`spec/profiles/alarms/v1.md` §5).
//!
//! **Session-free.** [`judge`] reads an [`AlarmObservation`] that
//! [`crate::bus::alarms`] gathered and hands back an [`AlarmsReport`]. Every
//! answer is `zenkey_model::alarms`'s, through the book
//! ([`crate::model::alarms`]): `firing`, `stale_firing` and `rederives` per
//! key, `owner_set` per owner and `any_firing` over the scope.
//!
//! **§5's four questions as judgements** (the tooling guide's exit codes, §1, through
//! [`crate::report::judgement_exit_code`]): each is named so that its
//! finding is the yes —
//! - "is this alarm firing?" ([`firing`]): yes is `Established`;
//! - "is anything firing at or above *S*?" ([`any_firing`]): yes is
//!   `Established`;
//! - "does this key fail to re-derive from its document?"
//!   ([`mismatch`]): §5's *no* (`mismatch`) is `Established`, and
//!   `sanitized` is `NotEstablished` with the SHOULD named in its reason;
//! - "is this firing alarm past its horizon, its owner present?"
//!   ([`stale`]): yes is `Established`, about the owner.
//!
//! The two unestablished kinds stay apart in every one (core O5).

use zenkey_model::alarms::{Answer, Judged, Reason, Severity};

use crate::bus::alarms::AlarmObservation;
use crate::model::alarms::{AlarmBook, answer_of, set_token, stamp_report, token};
use crate::report::{
    AlarmAnswer, AlarmOwnerRow, AlarmRow, AlarmScopeAnswer, AlarmsFace, AlarmsPresence,
    AlarmsReport, ClockGround, Judgement, LastKnownAlarm,
};

/// A yes-finding question's answer as a [`Judgement`].
fn yes_finding(j: Judged) -> Judgement {
    match j.answer {
        Answer::Yes => Judgement::Established,
        Answer::No => Judgement::NotEstablished {
            reason: format!("{} ({})", j.reason.says(), j.reason.as_str()),
        },
        Answer::Unobservable => Judgement::Unobservable {
            reason: format!("{} ({})", j.reason.says(), j.reason.as_str()),
        },
        Answer::NotAsked => Judgement::NotAsked,
    }
}

/// "Is this alarm firing?" (§2.14): the finding is the yes.
pub fn firing(j: Judged) -> Judgement {
    yes_finding(j)
}

/// "Is anything firing at or above *S*?" (§2.15): the finding is the yes.
pub fn any_firing(a: &zenkey_model::alarms::AnyFiring) -> Judgement {
    yes_finding(Judged {
        answer: a.answer,
        reason: a.reason,
    })
}

/// "Is this firing alarm past its horizon, its owner present?" (§2.7,
/// §2.14): the finding is the yes, about the owner.
pub fn stale(j: Judged) -> Judgement {
    yes_finding(j)
}

/// §5's "does this key re-derive from its document?", asked the other way
/// round so that its finding is the yes: `mismatch` is `Established`;
/// `strict` is clean, and `sanitized` is clean with the SHOULD of §2.3
/// named, which a tool MAY report.
pub fn mismatch(j: Judged) -> Judgement {
    match (j.answer, j.reason) {
        (Answer::No, _) => Judgement::Established,
        (Answer::Yes, Reason::Sanitized) => Judgement::NotEstablished {
            reason: format!(
                "{}: a break of §2.3's SHOULD to put framed input",
                j.reason.says()
            ),
        },
        (Answer::Yes, _) => Judgement::NotEstablished {
            reason: j.reason.says().to_owned(),
        },
        (Answer::Unobservable, _) => Judgement::Unobservable {
            reason: format!("{} ({})", j.reason.says(), j.reason.as_str()),
        },
        (Answer::NotAsked, _) => Judgement::NotAsked,
    }
}

/// One key's row: its §5 answers and what was read of it.
pub fn row(
    book: &AlarmBook,
    key: &str,
    at: std::time::Instant,
    wall: std::time::SystemTime,
) -> Option<AlarmRow> {
    let r = book.reading(key, at, wall)?;
    let seen = book.seen(key);
    let alarm = zenkey_model::alarms::AlarmRef::from_key(key)?;
    Some(AlarmRow {
        key: key.to_owned(),
        alarm,
        firing: answer_of(r.firing()),
        stale: answer_of(r.stale()),
        rederives: answer_of(r.rederives()),
        severity: r.severity,
        latest: seen,
        problems: r.problems(),
        foreign_stamp: book.foreign_stamp(key, at, wall),
        last_known: book.last_known(key).map(last_known),
    })
}

fn last_known(a: &crate::model::alarms::ArchivedAlarm) -> LastKnownAlarm {
    LastKnownAlarm {
        archive: a.archive.to_string(),
        document: a
            .value
            .as_deref()
            .and_then(|v| zenkey_model::alarms::decode(v).ok()),
        deleted: a.value.is_none(),
        stamp: a.stamp.as_ref().map(stamp_report),
        confirmed: a.confirmed,
    }
}

/// A key the archive holds whose owner holds no key of it here: its row is
/// the archive's alone, every question not asked (§2.14 step 1 or 3).
fn archived_row(book: &AlarmBook, key: &str) -> Option<AlarmRow> {
    let a = book.last_known(key)?;
    let alarm = zenkey_model::alarms::AlarmRef::from_key(key)?;
    let owner: zenkey_model::grammar::Addr =
        format!("{}/{}", alarm.system, alarm.service).parse().ok()?;
    let premise = match book.presence_of(&owner) {
        zenkey_model::alarms::Presence::Absent => Reason::Absent,
        _ => Reason::LastKnown,
    };
    let not_asked = answer_of(Judged {
        answer: Answer::NotAsked,
        reason: premise,
    });
    Some(AlarmRow {
        key: key.to_owned(),
        alarm,
        firing: not_asked.clone(),
        stale: not_asked.clone(),
        rederives: not_asked,
        severity: None,
        latest: None,
        problems: Vec::new(),
        foreign_stamp: false,
        last_known: Some(last_known(a)),
    })
}

/// Every verdict of one alarms reading, at `at_least`.
pub fn judge(obs: &AlarmObservation, at_least: Severity) -> AlarmsReport {
    let unread = |reason: String| AlarmScopeAnswer {
        at_least,
        answer: crate::report::AlarmAnswerToken::Unobservable,
        reason: "not_read".into(),
        says: reason,
        firing: 0,
        established: 0,
        unobservable: 0,
        not_asked: 0,
    };
    let face = obs.spec.face.map(|reachable| AlarmsFace { reachable });
    let window_s = obs.listened.map(|d| d.as_secs_f64());
    let book = match &obs.book {
        Ok(b) => b,
        Err(e) => {
            let why = format!("the alarms reader could not be opened: {e}");
            return AlarmsReport {
                namespace: obs.namespace.clone(),
                selector: obs.selector.clone(),
                presence: None,
                face,
                window_s,
                clock: ClockGround::None,
                asked: obs.asked.clone(),
                any_firing: unread(why.clone()),
                owners: Vec::new(),
                alarms: Vec::new(),
                lost: 0,
                unobservable: Some(why),
            };
        }
    };
    let (at, wall) = (obs.at, obs.wall);
    let mut unobservable = None;
    let presence = match &obs.log.presence {
        Some(Ok(p)) => Some(AlarmsPresence {
            selector: p.selector.clone(),
            complete: p.complete,
            cut_off_during: p.cut_off_during,
            services: p.services,
            listing: p.listing,
        }),
        Some(Err(e)) => {
            unobservable = Some(format!("the presence read could not be made: {e}"));
            None
        }
        None => None,
    };
    if let Some(Err(e)) = &obs.log.seed
        && unobservable.is_none()
    {
        unobservable = Some(format!("the GET of the alarms could not be made: {e}"));
    }
    if let Some(Err(e)) = &obs.log.archives
        && unobservable.is_none()
    {
        unobservable = Some(format!(
            "the archives' last-known could not all be read, and one not read could know an \
             absent owner (alarms.v1 §2.15, F-131): {e}"
        ));
    }
    let mut alarms: Vec<AlarmRow> = book.keys().filter_map(|k| row(book, k, at, wall)).collect();
    for k in book.archived_keys() {
        if alarms.iter().all(|r| r.key != k)
            && let Some(r) = archived_row(book, k)
        {
            alarms.push(r);
        }
    }
    alarms.sort_by(|a, b| a.key.cmp(&b.key));
    let owners: Vec<AlarmOwnerRow> = book
        .owners()
        .map(|a| {
            let set = book.owner_set(a, at);
            AlarmOwnerRow {
                address: a.to_string(),
                known_by: book.known_by(a),
                verdict: set_token(set.verdict),
                reason: set.reason.as_str().to_owned(),
                says: set.reason.says().to_owned(),
                alarms: book.keys_of(a).count(),
                window_s: book.window(a, at).map(|w| w.listened.as_secs_f64()),
            }
        })
        .collect();
    let any = book.any_firing(at_least, at, wall);
    let mut says = any.reason.says().to_owned();
    if any.reason == Reason::ScopeIncomplete {
        // Which read fell short (§2.15, text 0.5, F-137), and the read's
        // own reason beside the profile's (core §8.1, 0.38, F-191).
        says.push_str(&match &obs.log.presence {
            Some(Ok(p)) if !p.complete => format!(
                ": the presence read {}, so it is possibly incomplete (spec §8.1)",
                p.incomplete_reason().unwrap_or_default()
            ),
            Some(Err(e)) => format!(": the presence read could not be made: {e}"),
            _ if book.lost() > 0 => ": deliveries were lost since the last complete presence \
                                     read, and an instance token's appearance may be among them"
                .to_owned(),
            _ => ": no presence read was made".to_owned(),
        });
    }
    if book.refused() > 0 {
        says.push_str(&format!(
            "; {} alarm key(s) refused at the reader's bound could be firing",
            book.refused()
        ));
    }
    if unobservable.is_none() && owners.is_empty() {
        unobservable = Some(format!(
            "no owner of {} is known to this reader in {}: a reading over an empty scope \
             judged nothing, which is not \"nothing fires\"",
            obs.selector,
            if obs.namespace.is_empty() {
                "the bus root".to_owned()
            } else {
                format!("namespace {:?}", obs.namespace)
            }
        ));
    }
    AlarmsReport {
        namespace: obs.namespace.clone(),
        selector: obs.selector.clone(),
        presence,
        face,
        window_s,
        clock: book.clock_ground(),
        asked: obs.asked.clone(),
        any_firing: AlarmScopeAnswer {
            at_least,
            answer: token(any.answer),
            reason: any.reason.as_str().to_owned(),
            says,
            firing: any.firing,
            established: any.established,
            unobservable: any.unobservable,
            not_asked: any.not_asked,
        },
        owners,
        alarms,
        lost: book.lost(),
        unobservable,
    }
}

/// A §5 answer, as the report spells it, for a caller holding a
/// [`Judged`].
pub fn answer(j: Judged) -> AlarmAnswer {
    answer_of(j)
}

#[cfg(test)]
mod tests {
    //! The judgements' polarity, and a reading judged from a book: a firing
    //! alarm the finding, an absent owner's archived alarm last-known, an
    //! unreadable scope no verdict.

    use super::*;
    use crate::bus::alarms::{AlarmSpec, PresenceSeen, ReadLog};
    use crate::model::alarms::{AlarmSample, AlarmScope, ArchivedAlarm, BookSpec, SeedRead};
    use crate::report::{AlarmAnswerToken, AlarmVia, judgement_exit_code};
    use std::time::{Duration, Instant, SystemTime};
    use zenoh::time::{NTP64, Timestamp, TimestampId};

    const A: &str = "zk2/lab/svc/alarms.v1/state/alarms/a659f813308ad1da";

    fn j(answer: Answer, reason: Reason) -> Judged {
        Judged { answer, reason }
    }

    #[test]
    fn each_question_names_its_finding_the_yes() {
        let code = |x: Judgement| judgement_exit_code(&x);
        assert_eq!(code(firing(j(Answer::Yes, Reason::Firing))), 1);
        assert_eq!(code(firing(j(Answer::No, Reason::Resolved))), 0);
        assert_eq!(code(firing(j(Answer::Unobservable, Reason::Stale))), 2);
        assert_eq!(code(firing(j(Answer::NotAsked, Reason::Absent))), 2);
        assert_eq!(code(mismatch(j(Answer::No, Reason::Mismatch))), 1);
        assert_eq!(code(mismatch(j(Answer::Yes, Reason::Strict))), 0);
        let sanitized = mismatch(j(Answer::Yes, Reason::Sanitized));
        assert!(
            matches!(&sanitized, Judgement::NotEstablished { reason } if reason.contains("SHOULD")),
            "{sanitized:?}"
        );
        assert_eq!(code(mismatch(j(Answer::NotAsked, Reason::NoDocument))), 2);
        assert_eq!(code(stale(j(Answer::Yes, Reason::Stale))), 1);
        assert_eq!(code(stale(j(Answer::No, Reason::Fresh))), 0);
    }

    fn wall() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn stamp(age_s: u64) -> Timestamp {
        Timestamp::new(
            NTP64::from(
                (wall() - Duration::from_secs(age_s))
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap(),
            ),
            "a1".parse::<TimestampId>().unwrap(),
        )
    }

    fn doc() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "timestamp": 1, "rule": "link_down", "severity": "critical",
            "state": "firing", "summary": "link to r2 down on eth0",
            "labels": {"peer": "r2", "port": "eth0"},
        }))
        .unwrap()
    }

    fn observed(present: bool) -> crate::model::catalog::Observed {
        let keys: Vec<String> = if present {
            vec!["zk2/lab/svc/@zk/instance/000000000000000a".into()]
        } else {
            vec![]
        };
        let mut o = crate::model::catalog::Observed::from_keys("zk2/lab/*/@zk/**", &keys, true);
        let mut ds = std::collections::BTreeMap::new();
        if present {
            let d: zenkey_model::descriptor::Descriptor =
                serde_json::from_value(serde_json::json!({
                    "format": "zk2-descriptor/0.1", "service": "lab/svc",
                    "instance": "000000000000000a",
                    "interfaces": [{"iface": "alarms.v1",
                        "contract": format!("sha256:{}", zenkey::alarms::FINGERPRINT),
                        "minor": 0, "token": true}],
                    "meta": {"zid": "a1"},
                }))
                .unwrap();
            ds.insert(
                (
                    "lab/svc".parse().unwrap(),
                    "000000000000000a".parse().unwrap(),
                ),
                crate::model::catalog::DescriptorRead::Served(Box::new(d)),
            );
        }
        o.descriptors = Some(ds);
        o
    }

    fn obs(present: bool) -> AlarmObservation {
        let t0 = Instant::now();
        let mut spec = BookSpec::new(Duration::from_secs(1));
        spec.clocks_synced = true;
        let mut book = AlarmBook::new(
            AlarmScope::parse("zk2/lab/*/alarms.v1/state/alarms/*").unwrap(),
            spec,
        );
        book.presence_read(&observed(present), t0);
        book.archived(vec![ArchivedAlarm {
            archive: "lab/archive".parse().unwrap(),
            key: A.into(),
            value: Some(doc()),
            stamp: Some(stamp(30)),
            confirmed: true,
        }]);
        let replies = if present {
            vec![AlarmSample {
                key: A.into(),
                delete: false,
                payload: doc(),
                stamp: Some(stamp(10)),
                at: t0,
                wall: wall(),
                via: AlarmVia::Get,
            }]
        } else {
            vec![]
        };
        book.seed(
            SeedRead {
                asked_at: t0,
                complete: true,
                replies,
                discarded: 0,
            },
            t0,
            wall(),
        );
        let mut spec = AlarmSpec::new(Duration::from_secs(1));
        spec.clocks_synced = true;
        AlarmObservation {
            namespace: "acme".into(),
            spec,
            selector: "zk2/lab/*/alarms.v1/state/alarms/*".into(),
            book: Ok(book),
            log: ReadLog {
                presence: Some(Ok(PresenceSeen {
                    selector: "zk2/lab/*/@zk/**".into(),
                    complete: true,
                    cut_off_during: false,
                    services: usize::from(present),
                    listing: usize::from(present),
                    unsettled: false,
                })),
                seed: Some(Ok(true)),
                archives: Some(Ok(vec!["lab/archive".into()])),
                discarded: 0,
            },
            asked: vec!["zk2/lab/*/alarms.v1/state/alarms/*".into()],
            listened: None,
            at: t0,
            wall: wall(),
        }
    }

    #[test]
    fn a_firing_alarm_is_the_finding_and_an_absent_one_last_known() {
        let r = judge(&obs(true), Severity::Warning);
        assert_eq!(r.any_firing.answer, AlarmAnswerToken::Yes);
        assert_eq!(r.any_firing.firing, 1);
        assert_eq!(judgement_exit_code(&r.judgement()), 1);
        assert_eq!(r.alarms.len(), 1);
        assert_eq!(r.alarms[0].firing.reason, "firing");
        assert_eq!(r.alarms[0].stale.reason, "fresh");
        assert_eq!(r.alarms[0].rederives.reason, "strict");
        assert_eq!(r.owners[0].reason, "answered");
        assert_eq!(r.clock, ClockGround::DeploymentWord);

        let r = judge(&obs(false), Severity::Warning);
        assert_eq!(r.any_firing.answer, AlarmAnswerToken::NotAsked, "{r:#?}");
        assert_eq!(judgement_exit_code(&r.judgement()), 2);
        let row = &r.alarms[0];
        assert_eq!(row.firing.reason, "absent");
        let lk = row.last_known.as_ref().expect("the archive's document");
        assert_eq!(lk.archive, "lab/archive");
        assert!(lk.document.is_some() && lk.confirmed);
        assert!(row.latest.is_none(), "never shown as current");
        assert_eq!(r.owners[0].known_by, [crate::report::KnownBy::Archive]);
    }

    #[test]
    fn an_archive_not_read_is_no_verdict_over_nothing_firing() {
        let mut o = obs(true);
        o.log.archives = Some(Err("archive lab/archive: timed out".into()));
        let mut r = judge(&o, Severity::Warning);
        let why = r.unobservable.clone().expect("an archive not read is said");
        assert!(why.contains("lab/archive: timed out"), "{why}");
        assert_eq!(
            judgement_exit_code(&r.judgement()),
            1,
            "a firing alarm is still the finding"
        );
        r.any_firing.answer = AlarmAnswerToken::No;
        assert_eq!(
            judgement_exit_code(&r.judgement()),
            2,
            "never \"nothing fires\""
        );
    }

    #[test]
    fn a_reader_that_could_not_open_is_no_verdict() {
        let mut o = obs(true);
        o.book = Err("no session".into());
        let r = judge(&o, Severity::Info);
        assert_eq!(judgement_exit_code(&r.judgement()), 2);
        assert!(r.unobservable.unwrap().contains("no session"));
    }

    /// F-137 (alarms.v1 text 0.5): a presence read that ran to its timeout
    /// may have missed an owner, so its one owner established and quiet is
    /// no "nothing fires": `scope_incomplete`, the read that fell short
    /// said, exit 2.
    #[test]
    fn a_presence_read_not_whole_is_no_verdict_over_nothing_firing() {
        let mut o = obs(true);
        let t0 = o.at;
        let book = o.book.as_mut().unwrap();
        let resolved = serde_json::to_vec(&serde_json::json!({
            "timestamp": 2, "rule": "link_down", "severity": "critical",
            "state": "resolved", "summary": "link to r2 up again",
            "labels": {"peer": "r2", "port": "eth0"},
        }))
        .unwrap();
        book.sample(AlarmSample {
            key: A.into(),
            delete: false,
            payload: resolved,
            stamp: Some(stamp(5)),
            at: t0,
            wall: wall(),
            via: AlarmVia::Subscription,
        });
        let r = judge(&o, Severity::Warning);
        assert_eq!(r.any_firing.answer, AlarmAnswerToken::No, "{r:#?}");
        assert_eq!(judgement_exit_code(&r.judgement()), 0);

        let mut partial = observed(true);
        partial.complete = false;
        o.book.as_mut().unwrap().presence_read(&partial, t0);
        if let Some(Ok(p)) = &mut o.log.presence {
            p.complete = false;
        }
        let r = judge(&o, Severity::Warning);
        assert_eq!(r.any_firing.answer, AlarmAnswerToken::Unobservable);
        assert_eq!(r.any_firing.reason, "scope_incomplete");
        assert!(
            r.any_firing.says.contains("timeout"),
            "{}",
            r.any_firing.says
        );
        assert_eq!(
            r.any_firing.established, 1,
            "the owner found is still known whole"
        );
        assert_eq!(judgement_exit_code(&r.judgement()), 2);

        // A read cut off while it ran is possibly incomplete too, and says
        // so (core §8.1, 0.38, F-185).
        if let Some(Ok(p)) = &mut o.log.presence {
            p.cut_off_during = true;
        }
        let r = judge(&o, Severity::Warning);
        assert_eq!(r.any_firing.reason, "scope_incomplete");
        assert!(
            r.any_firing.says.contains("cut off while it ran"),
            "{}",
            r.any_firing.says
        );
        assert!(r.presence.as_ref().is_some_and(|p| p.cut_off_during));
        assert_eq!(judgement_exit_code(&r.judgement()), 2);

        // A read not made: the profile's reason stands, with the read's own
        // beside it (core §8.1, 0.38, F-191).
        o.log.presence = Some(Err(crate::error::NotAsked::CutOff.to_string()));
        let r = judge(&o, Severity::Warning);
        assert_eq!(r.any_firing.reason, "scope_incomplete");
        assert!(
            r.any_firing.says.contains("no router and no peer"),
            "{}",
            r.any_firing.says
        );
        assert!(
            r.unobservable
                .as_deref()
                .is_some_and(|u| u.contains("no router and no peer")),
            "{:?}",
            r.unobservable
        );
        assert_eq!(judgement_exit_code(&r.judgement()), 2);
    }
}
