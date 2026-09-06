//! A snapshot's rows from values in hand (RFC 13 §4.4; #219): the per-key
//! last-writer-wins fold, and the projections that turn one kept reply into
//! the row's facets — holder, registration, verdict, stamper.
//!
//! Nothing here takes a session. [`crate::bus::query::snapshot_get`] brings
//! the replies and the roster back; this module says what they mean, which
//! is what lets the same projections be unit-tested against hand-built
//! views and, later, run over a file.

use std::collections::BTreeMap;

use crate::bus::monitor::{SampleView, StampProvenance};
use crate::model::facts::{KeyFacts, KeyShape, Registration};
use crate::report::{AnsweredBy, Holder, RegistrationWire, StamperWire};

/// The replier's zenoh id, when the reply named one.
pub type Replier = Option<zenoh::config::ZenohId>;

/// Fold every reply to one kept value per key, last-writer-wins, and count
/// what lost.
///
/// **The rule is `pick_latest`'s** (the fetch ladder's,
/// [`crate::fetch_value`]), restated: a newer HLC wins; a stamped value beats
/// an unstamped one; between two unstamped values, or two carrying the same
/// stamp, the first seen is kept. RFC 04 §1.2's reconciliation is by HLC,
/// and an untimestamped reply cannot be reconciled at all — keeping the
/// first is the honest tie-break, and the loser is counted in `superseded`
/// rather than silently forgotten (O6 applied to a fold).
pub fn fold_latest(
    values: Vec<(SampleView, Replier)>,
) -> (BTreeMap<String, (SampleView, Replier)>, u64) {
    let mut kept: BTreeMap<String, (SampleView, Replier)> = BTreeMap::new();
    let mut superseded = 0u64;
    for (view, replier) in values {
        match kept.get(&view.key) {
            None => {
                kept.insert(view.key.clone(), (view, replier));
            }
            Some((cur, _)) => {
                let newer = match (cur.timestamp, view.timestamp) {
                    (Some(a), Some(b)) => b > a,
                    (None, Some(_)) => true,
                    _ => false,
                };
                superseded += 1;
                if newer {
                    kept.insert(view.key.clone(), (view, replier));
                }
            }
        }
    }
    (kept, superseded)
}

/// Who holds a value — evidence, not inference (RFC 13 §4.4).
///
/// `roster` is the liveliness roster as [`crate::roster`] returns it
/// (origin → producers), or `None` when it was not asked. The order of the
/// tests is the order of the questions: was the roster asked at all; does
/// the key name an origin; did that origin hold a token; and, only for a
/// live origin, whether the replier was the stamping entity.
pub fn holder_of(
    base: &str,
    key: &str,
    view: &SampleView,
    replier: Replier,
    roster: Option<&BTreeMap<String, Vec<String>>>,
) -> Holder {
    let Some(roster) = roster else {
        return Holder::Unattributed {
            reason: "roster not asked".into(),
        };
    };
    let facts = KeyFacts::project(base, key);
    let origin = match &facts.shape {
        KeyShape::V1(f) => f.origin.clone(),
        KeyShape::NotUnderBase => {
            return Holder::Unattributed {
                reason: "the key is not under the stated base, so it names no origin here".into(),
            };
        }
        KeyShape::Unparsed { reason } => {
            return Holder::Unattributed {
                reason: format!("the key names no origin: {reason}"),
            };
        }
    };
    if !roster.contains_key(&origin) {
        return Holder::StorageOnly { origin };
    }
    Holder::Live {
        origin,
        answered_by: answered_by(view, replier),
    }
}

/// Whether the replier was the stamping entity: both ids known and equal is
/// `Stamper`, both known and different is `Other`, anything less is
/// `Unknown` (O4 — a missing id is not a mismatch).
fn answered_by(view: &SampleView, replier: Replier) -> AnsweredBy {
    let stamper = match view.stamped_by {
        Some(StampProvenance::SelfStamped) => {
            view.source.map(|s| zenoh::time::TimestampId::from(s.zid))
        }
        Some(StampProvenance::Foreign { stamper })
        | Some(StampProvenance::Unattributable { stamper }) => Some(stamper),
        None => None,
    };
    match (stamper, replier) {
        (Some(s), Some(r)) if s == zenoh::time::TimestampId::from(r) => AnsweredBy::Stamper,
        (Some(_), Some(_)) => AnsweredBy::Other,
        _ => AnsweredBy::Unknown,
    }
}

/// O2's rung for a projected (and, when a registry was loaded, resolved)
/// key. [`Registration::Unknown`] is `registry_not_loaded`, not
/// `unregistered` — "not asked" is not "answered no" (O4).
pub fn registration_of(facts: &KeyFacts) -> RegistrationWire {
    match &facts.shape {
        KeyShape::NotUnderBase => RegistrationWire::NotUnderBase,
        KeyShape::Unparsed { .. } => RegistrationWire::NotV1,
        KeyShape::V1(_) => match &facts.registration {
            Registration::Unknown => RegistrationWire::RegistryNotLoaded,
            Registration::NoSliceForProducer => RegistrationWire::NoSliceForProducer,
            Registration::Unregistered => RegistrationWire::Unregistered,
            Registration::Registered(_) => RegistrationWire::Registered,
            Registration::NotApplicable => RegistrationWire::NotADataClass,
        },
    }
}

/// The three-valued verdict on the wire, with the not-validated reason as a
/// stable token rather than its prose.
#[cfg(feature = "decode")]
pub fn verdict_of(verdict: &zenkey::schema::validate::Verdict) -> crate::report::VerdictWire {
    use crate::report::VerdictWire;
    use zenkey::schema::validate::{NotValidated, Verdict};
    match verdict {
        Verdict::Valid => VerdictWire::Valid,
        Verdict::Invalid(violations) => VerdictWire::Invalid {
            violations: violations.clone(),
        },
        Verdict::NotValidated(reason) => VerdictWire::NotValidated {
            reason: match reason {
                NotValidated::NoSchema => "no_schema",
                NotValidated::NoRegistry => "no_registry",
                NotValidated::FeatureOff => "feature_off",
                NotValidated::KindUnsupported => "kind_unsupported",
                NotValidated::Undecodable => "undecodable",
                NotValidated::BadSchema => "bad_schema",
            }
            .into(),
        },
    }
}

/// O7's classification of a stamp, on the wire.
pub fn stamper_of(provenance: &StampProvenance) -> StamperWire {
    match provenance {
        StampProvenance::SelfStamped => StamperWire::SelfStamped,
        StampProvenance::Foreign { stamper } => StamperWire::Foreign {
            id: stamper.to_string(),
        },
        StampProvenance::Unattributable { stamper } => StamperWire::Unattributable {
            id: stamper.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn stamp(secs: u64, id: zenoh::time::TimestampId) -> zenoh::time::Timestamp {
        zenoh::time::Timestamp::new(zenoh::time::NTP64::from(Duration::from_secs(secs)), id)
    }

    fn view(key: &str, payload: &[u8], timestamp: Option<zenoh::time::Timestamp>) -> SampleView {
        SampleView {
            key: key.to_string(),
            payload: zenoh::bytes::ZBytes::from(payload.to_vec()),
            encoding: String::new(),
            kind: zenoh::sample::SampleKind::Put,
            stamped_by: timestamp.map(|t| StampProvenance::Unattributable {
                stamper: *t.get_id(),
            }),
            timestamp,
            attachment: None,
            priority: zenoh::qos::Priority::DEFAULT,
            congestion_control: zenoh::qos::CongestionControl::DEFAULT,
            reliability: zenoh::qos::Reliability::DEFAULT,
            express: false,
            source: None,
            received: Instant::now(),
        }
    }

    const KEY: &str = "v1/h-3fa9c2d41b7e/state/sysinfo/health";

    /// The fold keeps the newest stamp, lets a stamp beat no stamp, keeps
    /// the first of two unstamped — and counts every loser.
    #[test]
    fn the_fold_is_last_writer_wins_and_counts_what_lost() {
        let id = zenoh::time::TimestampId::rand();
        let (kept, superseded) = fold_latest(vec![
            (view(KEY, b"old", Some(stamp(10, id))), None),
            (view(KEY, b"new", Some(stamp(20, id))), None),
            (view(KEY, b"stale", Some(stamp(5, id))), None),
            (view(KEY, b"unstamped", None), None),
        ]);
        assert_eq!(superseded, 3);
        assert_eq!(kept[KEY].0.payload.to_bytes().as_ref(), b"new");

        let (kept, superseded) = fold_latest(vec![
            (view(KEY, b"first", None), None),
            (view(KEY, b"second", None), None),
        ]);
        assert_eq!(superseded, 1);
        assert_eq!(
            kept[KEY].0.payload.to_bytes().as_ref(),
            b"first",
            "two unstamped values cannot be reconciled; the first seen stands"
        );

        let (kept, superseded) = fold_latest(vec![
            (view(KEY, b"unstamped", None), None),
            (view(KEY, b"stamped", Some(stamp(1, id))), None),
        ]);
        assert_eq!(superseded, 1);
        assert_eq!(kept[KEY].0.payload.to_bytes().as_ref(), b"stamped");
    }

    /// The holder ladder, every rung: roster not asked, no origin, origin
    /// not alive, alive with the three replier answers.
    #[test]
    fn the_holder_is_evidence_at_every_rung() {
        let v = view(KEY, b"{}", None);
        assert_eq!(
            holder_of("", KEY, &v, None, None),
            Holder::Unattributed {
                reason: "roster not asked".into()
            }
        );
        let roster: BTreeMap<String, Vec<String>> = BTreeMap::new();
        assert!(matches!(
            holder_of("", "not/a/v1/key", &v, None, Some(&roster)),
            Holder::Unattributed { .. }
        ));
        assert!(matches!(
            holder_of("acme", KEY, &v, None, Some(&roster)),
            Holder::Unattributed { reason } if reason.contains("not under the stated base")
        ));
        assert_eq!(
            holder_of("", KEY, &v, None, Some(&roster)),
            Holder::StorageOnly {
                origin: "h-3fa9c2d41b7e".into()
            }
        );

        let mut roster = roster;
        roster.insert("h-3fa9c2d41b7e".into(), vec!["sysinfo".into()]);
        assert_eq!(
            holder_of("", KEY, &v, None, Some(&roster)),
            Holder::Live {
                origin: "h-3fa9c2d41b7e".into(),
                answered_by: AnsweredBy::Unknown,
            },
            "unstamped and no replier: nothing to compare (O4)"
        );

        let stamper = zenoh::config::ZenohId::default();
        let stamped = view(KEY, b"{}", Some(stamp(1, stamper.into())));
        assert_eq!(
            holder_of("", KEY, &stamped, Some(stamper), Some(&roster)),
            Holder::Live {
                origin: "h-3fa9c2d41b7e".into(),
                answered_by: AnsweredBy::Stamper,
            }
        );
        let other = view(KEY, b"{}", Some(stamp(1, zenoh::time::TimestampId::rand())));
        assert_eq!(
            holder_of("", KEY, &other, Some(stamper), Some(&roster)),
            Holder::Live {
                origin: "h-3fa9c2d41b7e".into(),
                answered_by: AnsweredBy::Other,
            }
        );
        assert_eq!(
            holder_of("", KEY, &stamped, None, Some(&roster)),
            Holder::Live {
                origin: "h-3fa9c2d41b7e".into(),
                answered_by: AnsweredBy::Unknown,
            },
            "a stamp with no replier id is unknown, not other"
        );
    }

    /// `Registration::Unknown` is *registry not loaded*, never
    /// *unregistered* (O4); the two non-v1 shapes have their own rungs.
    #[test]
    fn registration_keeps_not_loaded_apart_from_unregistered() {
        assert_eq!(
            registration_of(&KeyFacts::project("", KEY)),
            RegistrationWire::RegistryNotLoaded
        );
        assert_eq!(
            registration_of(&KeyFacts::project("acme", KEY)),
            RegistrationWire::NotUnderBase
        );
        assert_eq!(
            registration_of(&KeyFacts::project("", "plain/zenoh/key")),
            RegistrationWire::NotV1
        );
        assert_eq!(
            registration_of(&KeyFacts::project(
                "",
                "v1/h-3fa9c2d41b7e/@rpc/sysinfo/ping"
            )),
            RegistrationWire::NotADataClass
        );
    }
}
