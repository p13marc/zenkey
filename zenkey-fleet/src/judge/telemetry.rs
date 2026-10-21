//! `telemetry.v1` (#819, TC): a reading of metric members judged, every
//! verdict decided from values (`spec/profiles/telemetry/v1.md` §5).
//!
//! **Session-free.** [`judge`] reads a [`TelemetryObservation`] that
//! [`crate::bus::telemetry`] gathered and hands back a [`TelemetryReport`].
//! Every answer is `zenkey_model::telemetry`'s, through the book
//! ([`crate::model::telemetry`]): `declaration_of` per resource, `check` per
//! sample, `pair` per pair.
//!
//! **§5's five questions as judgements** (RFC 13 §1.2, through
//! [`crate::report::judgement_exit_code`]), each named so that its finding
//! is the yes:
//! - "is this resource's declaration invalid?" ([`declaration_invalid`]):
//!   `invalid` is `Established` — a contract linter's warning (no core
//!   code), which the doctor and `check conform` report as one;
//! - "what is this member's kind, and its unit?" ([`kind_and_unit`]):
//!   informative, never a finding;
//! - "does this sample fail to conform?" ([`does_not_conform`]): `does_not`
//!   is `Established`;
//! - "did this counter or histogram go backwards within one epoch?"
//!   ([`decreased`]): `decreased` is `Established`, and only within an
//!   established epoch — a decrease read as a reset is no;
//! - "what is this counter's rate over the pair?" ([`rate`]): informative.
//!
//! The two unestablished kinds stay apart in every one (core O5). Of the
//! five, only a sample that does not conform and a decrease are the run's
//! findings ([`TelemetryReport::judgement`]).

use zenkey_model::telemetry::{
    Basis, Checked, Conformance, Continuity, Declaration, Measure, NoRate, Pair, Via,
};

use crate::bus::telemetry::TelemetryObservation;
use crate::model::presence_live::PresenceBasis;
use crate::model::telemetry::{MemberBook, MetricResource, PairPresence, PairSeen, invalid_words};
use crate::report::{
    ContinuityToken, DeclarationToken, EpochToken, Judgement, PairPresenceToken,
    TelemetryContinuity, TelemetryDeclaration, TelemetryMember, TelemetryPair, TelemetryPresence,
    TelemetryRate, TelemetryReport, TelemetryResource, TelemetrySamples, TelemetryTotals,
};

/// "Is this resource's declaration invalid?" (§2.2, §2.4): `invalid` is the
/// yes, a linter's warning; `declared` the no; a resource that is not this
/// profile's, or an event or an operation, not asked.
pub fn declaration_invalid(d: &Declaration) -> Judgement {
    match d {
        Declaration::Invalid(_) => Judgement::Established,
        Declaration::Declared(_) => Judgement::NotEstablished {
            reason: declaration_says(d),
        },
        Declaration::Ignored | Declaration::NotThisProfiles => Judgement::NotAsked,
    }
}

/// "What is this member's kind, and its unit?" (§2.5), as one sample gives
/// them: informative, so never a finding. Known is no finding; unobservable
/// when the declaration is invalid, or no kind is declared and the sample
/// is no Point; not asked when the resource is not this profile's.
pub fn kind_and_unit(c: &Checked) -> Judgement {
    match (&c.conformance, c.kind) {
        (Conformance::NotAsked(_), _) => Judgement::NotAsked,
        (_, Some((k, from))) => Judgement::NotEstablished {
            reason: format!(
                "{k} ({}), unit {}",
                from.as_str(),
                c.unit.as_ref().map_or(
                    "unknown, never dimensionless".to_owned(),
                    |(u, f)| format!("{u} ({})", f.as_str())
                )
            ),
        },
        (Conformance::Unobservable(why), None) => Judgement::Unobservable {
            reason: why.as_str().to_owned(),
        },
        (_, None) => Judgement::Unobservable {
            reason: "no kind is declared, and the sample is no Point".to_owned(),
        },
    }
}

/// "Does this sample fail to conform?" (§2.6): `does_not` is the yes, the
/// finding, with its reasons.
pub fn does_not_conform(c: &Conformance) -> Judgement {
    match c {
        Conformance::DoesNot(_) => Judgement::Established,
        Conformance::Conforms => Judgement::NotEstablished {
            reason: "it conforms".to_owned(),
        },
        Conformance::Unobservable(why) => Judgement::Unobservable {
            reason: why.as_str().to_owned(),
        },
        Conformance::NotAsked(_) => Judgement::NotAsked,
    }
}

/// "Did this counter or histogram go backwards within one epoch?" (§2.10):
/// `decreased` — a decrease within an established epoch — is the yes, the
/// finding; `same_epoch` and `reset` the no, a decrease read as a reset
/// included; incomparable unobservable; neither a counter nor a histogram
/// not asked.
pub fn decreased(p: &Pair) -> Judgement {
    match p.continuity {
        Continuity::Decreased => Judgement::Established,
        Continuity::SameEpoch | Continuity::Reset => Judgement::NotEstablished {
            reason: format!("{} ({})", p.continuity.as_str(), basis_words(p.basis)),
        },
        Continuity::Unobservable => Judgement::Unobservable {
            reason: "incomparable: the kinds, or a histogram's bounds or length, differ".to_owned(),
        },
        Continuity::NotAsked => Judgement::NotAsked,
    }
}

/// "What is this counter's rate over the pair?" (§2.11): informative. A
/// rate, or none with why, is no finding; a pair not of two counters is not
/// asked.
pub fn rate(p: &Pair) -> Judgement {
    match &p.rate {
        Ok(r) => Judgement::NotEstablished {
            reason: format!(
                "{} {}",
                r.per_s,
                r.unit.as_deref().unwrap_or("(unit unknown)")
            ),
        },
        Err(NoRate::Kind) => Judgement::NotAsked,
        Err(why) => Judgement::NotEstablished {
            reason: format!("no rate: {}", why.as_str()),
        },
    }
}

fn basis_words(b: Option<Basis>) -> &'static str {
    match b {
        Some(Basis::Established(Via::InBand)) => "the epoch established in-band",
        Some(Basis::Established(Via::Presence)) => "the epoch established by presence",
        Some(Basis::Assumed) => "the epoch assumed",
        None => "the epoch not asked",
    }
}

/// A declaration in words (§2.2).
pub fn declaration_says(d: &Declaration) -> String {
    use zenkey_model::telemetry::{Carrier, EpochIn};
    match d {
        Declaration::Declared(d) => {
            let what = match (d.carrier, d.kind) {
                (Carrier::Point, Some(k)) => format!("a Point declared {k}"),
                (Carrier::Point, None) => "a Point that says its own kind".to_owned(),
                (Carrier::Bare, Some(k)) => format!("a bare {k}"),
                (Carrier::Bare, None) => "a bare value".to_owned(),
            };
            let unit = match (&d.unit, d.carrier) {
                (Some(u), _) => format!("unit {u}"),
                (None, Carrier::Point) => "unit the Point's, else unknown".to_owned(),
                (None, Carrier::Bare) => "unit unknown".to_owned(),
            };
            let cumulative = d.kind.is_none_or(|k| k.is_cumulative());
            let epoch = match d.epoch {
                Some(EpochIn::Point) if cumulative => "its epoch in-band, in the Point",
                Some(EpochIn::Attachment) => "its epoch in-band, in its Epoch attachment",
                _ if cumulative => "no in-band epoch: presence is its epoch's evidence",
                _ => "no epoch: a gauge, text, boolean or binary has none",
            };
            format!("declared: {what}, {unit}; {epoch}")
        }
        Declaration::Invalid(why) => format!(
            "invalid: {} — its members' kind, conformance and continuity are unobservable \
             (telemetry.v1 §2.2)",
            invalid_words(why)
        ),
        Declaration::Ignored => "ignored: an event or an operation".to_owned(),
        Declaration::NotThisProfiles => "not this profile's".to_owned(),
    }
}

/// One resource's declaration, as the report spells it.
pub fn declaration_row(d: &Declaration) -> TelemetryDeclaration {
    let says = declaration_says(d);
    match d {
        Declaration::Declared(x) => TelemetryDeclaration {
            answer: DeclarationToken::Declared,
            reasons: Vec::new(),
            carrier: Some(x.carrier.as_str().to_owned()),
            kind: x.kind.map(|k| k.as_str().to_owned()),
            unit: x.unit.clone(),
            buckets: x.buckets.clone(),
            semantic: x.semantic.map(|s| s.as_str().to_owned()),
            epoch_in: x.epoch.map(|e| e.as_str().to_owned()),
            says,
        },
        Declaration::Invalid(why) => TelemetryDeclaration {
            answer: DeclarationToken::Invalid,
            reasons: why.iter().map(|w| w.as_str().to_owned()).collect(),
            carrier: None,
            kind: None,
            unit: None,
            buckets: None,
            semantic: None,
            epoch_in: None,
            says,
        },
        // No resource that is neither reaches a reader's rows.
        Declaration::Ignored | Declaration::NotThisProfiles => TelemetryDeclaration {
            answer: DeclarationToken::Invalid,
            reasons: Vec::new(),
            carrier: None,
            kind: None,
            unit: None,
            buckets: None,
            semantic: None,
            epoch_in: None,
            says,
        },
    }
}

/// A conforming value as its kind carries it, for a report.
pub fn measure_json(m: &Measure) -> serde_json::Value {
    use serde_json::json;
    match m {
        Measure::Counter(n) => json!(n),
        Measure::Gauge(f) => json!(f),
        Measure::Histogram(h) => json!({
            "buckets": h.bounds, "counts": h.counts, "count": h.count, "sum": h.sum,
        }),
        Measure::Text(s) => json!(s),
        Measure::Boolean(b) => json!(b),
        Measure::Binary(b) => json!(b),
    }
}

/// One pair, as the report spells it.
pub fn pair_row(p: &PairSeen) -> TelemetryPair {
    let (epoch, via) = match p.pair.basis {
        None => (None, None),
        Some(Basis::Assumed) => (Some(EpochToken::Assumed), None),
        Some(Basis::Established(v)) => (Some(EpochToken::Established), Some(v.as_str().to_owned())),
    };
    TelemetryPair {
        earlier: measure_json(&p.earlier),
        later: measure_json(&p.later),
        continuity: match p.pair.continuity {
            Continuity::SameEpoch => ContinuityToken::SameEpoch,
            Continuity::Reset => ContinuityToken::Reset,
            Continuity::Decreased => ContinuityToken::Decreased,
            Continuity::Unobservable => ContinuityToken::Unobservable,
            Continuity::NotAsked => ContinuityToken::NotAsked,
        },
        epoch,
        via,
        presence: match p.presence {
            PairPresence::Observed { .. } => PairPresenceToken::Observed,
            PairPresence::SpansLoss => PairPresenceToken::SpansLoss,
            PairPresence::None => PairPresenceToken::None,
        },
        rate: p.pair.rate.as_ref().ok().map(|r| r.per_s),
        rate_unit: p.pair.rate.as_ref().ok().and_then(|r| r.unit.clone()),
        no_rate: p.pair.rate.as_ref().err().map(|n| n.as_str().to_owned()),
        base: p.base.to_owned(),
        elapsed_s: p.elapsed_ns as f64 / 1e9,
    }
}

/// One member, as the report spells it.
pub fn member_row(key: &str, m: &MemberBook) -> TelemetryMember {
    let r = &m.metric;
    TelemetryMember {
        key: key.to_owned(),
        address: r.address.to_string(),
        iface: r.iface.to_string(),
        resource: r.resource.clone(),
        kind: m.kind.map(|(k, _)| k.as_str().to_owned()),
        kind_from: m.kind.map(|(_, f)| f.as_str().to_owned()),
        unit: m.unit.as_ref().map(|(u, _)| u.clone()),
        unit_from: m.unit.as_ref().map(|(_, f)| f.as_str().to_owned()),
        samples: TelemetrySamples {
            received: m.samples,
            conforms: m.conforming,
            does_not: m.does_not,
            unobservable: m.unobservable,
            unobservable_reason: m.unobservable_why.map(|w| w.as_str().to_owned()),
            not_kept: m.not_kept,
            deletes: m.deletes,
            problems: m
                .problems
                .iter()
                .map(|(p, n)| (p.as_str().to_owned(), *n))
                .collect(),
            latest_does_not: m
                .last_does_not
                .iter()
                .flatten()
                .map(|p| p.as_str().to_owned())
                .collect(),
        },
        latest: m.latest.as_ref().map(|r| measure_json(&r.value)),
        continuity: TelemetryContinuity {
            pairs: m.tally.total(),
            same_epoch: m.tally.same_epoch,
            reset: m.tally.reset,
            decreased: m.tally.decreased,
            unobservable: m.tally.unobservable,
            not_asked: m.tally.not_asked,
            in_band: m.tally.in_band,
            by_presence: m.tally.by_presence,
            assumed: m.tally.assumed,
        },
        pairs: m.pairs.iter().map(pair_row).collect(),
        rate: m.rate.as_ref().map(|r| TelemetryRate {
            per_s: r.per_s,
            unit: r.unit.clone(),
        }),
    }
}

fn same_resource(a: &MetricResource, b: &MetricResource) -> bool {
    a.address == b.address && a.iface == b.iface && a.resource == b.resource
}

/// Every verdict of one reading.
pub fn judge(obs: &TelemetryObservation) -> TelemetryReport {
    let book = &obs.book;
    let members: Vec<TelemetryMember> = book.members().map(|(k, m)| member_row(k, m)).collect();
    let resources: Vec<TelemetryResource> = obs
        .resources
        .iter()
        .map(|r| TelemetryResource {
            address: r.address.to_string(),
            iface: r.iface.to_string(),
            fingerprint: if obs.spec.across_face {
                String::new()
            } else {
                r.fingerprint.clone()
            },
            resource: r.resource.clone(),
            selector: obs
                .scope
                .subscription(r)
                .unwrap_or_else(|| r.selector.clone()),
            declaration: declaration_row(&r.ty.declaration),
            members: book
                .members()
                .filter(|(_, m)| same_resource(&m.metric, r))
                .count(),
        })
        .collect();
    let mut totals = TelemetryTotals {
        resources: resources.len(),
        invalid: obs.resources.iter().filter(|r| !r.declared()).count(),
        members: members.len(),
        ..TelemetryTotals::default()
    };
    for (_, m) in book.members() {
        totals.samples += m.samples;
        totals.conforms += m.conforming;
        totals.does_not += m.does_not;
        totals.unobservable += m.unobservable;
        totals.pairs += m.tally.total();
        totals.decreased += m.tally.decreased;
        totals.established += m.tally.in_band + m.tally.by_presence;
        totals.assumed += m.tally.assumed;
    }
    let mut skipped = obs.skipped.clone();
    let presence = match &obs.seed {
        Some(Ok(s)) => Some(TelemetryPresence {
            selector: s.selector.clone(),
            complete: s.complete,
            services: s.services,
            basis: match book.presence_basis() {
                Some(PresenceBasis::Complete) => "complete",
                Some(PresenceBasis::Incomplete) => "incomplete",
                Some(PresenceBasis::Lost { .. }) => "lost",
                Some(PresenceBasis::Unseeded) | None => "unseeded",
            }
            .to_owned(),
        }),
        Some(Err(e)) => {
            skipped.push(format!(
                "the last presence seed did not stand ({e}): the pairs since it was owed have \
                 no presence"
            ));
            None
        }
        None => None,
    };
    TelemetryReport {
        namespace: obs.namespace.clone(),
        selector: obs.scope.selector().map(str::to_owned),
        presence,
        across_face: obs.spec.across_face,
        window_s: obs.listened.as_secs_f64(),
        asked: obs.asked.clone(),
        totals,
        resources,
        members,
        lost: book.lost(),
        refused: book.refused(),
        unresolved: obs.unresolved,
        skipped,
        unobservable: obs.unobservable.clone(),
    }
}

#[cfg(test)]
mod tests {
    //! The judgements' polarity: each question names its finding the yes,
    //! and only `does_not` and `decreased` are ever the 1.

    use super::*;
    use crate::report::judgement_exit_code;
    use zenkey_model::telemetry::{Invalid, Rate, Unasked};

    fn pair(continuity: Continuity, basis: Option<Basis>, rate: Result<Rate, NoRate>) -> Pair {
        Pair {
            continuity,
            basis,
            rate,
        }
    }

    #[test]
    fn each_question_names_its_finding_the_yes() {
        let code = |j: Judgement| judgement_exit_code(&j);
        assert_eq!(
            code(declaration_invalid(&Declaration::Invalid(vec![
                Invalid::KindMissing
            ]))),
            1
        );
        assert_eq!(code(declaration_invalid(&Declaration::NotThisProfiles)), 2);
        assert_eq!(
            code(does_not_conform(&Conformance::DoesNot(vec![]))),
            1,
            "does_not"
        );
        assert_eq!(code(does_not_conform(&Conformance::Conforms)), 0);
        assert_eq!(
            code(does_not_conform(&Conformance::Unobservable(
                Unasked::Undecodable
            ))),
            2
        );
        let est = Some(Basis::Established(Via::InBand));
        assert_eq!(
            code(decreased(&pair(
                Continuity::Decreased,
                est,
                Err(NoRate::Decreased)
            ))),
            1
        );
        let assumed_reset = pair(Continuity::Reset, Some(Basis::Assumed), Err(NoRate::Reset));
        assert_eq!(code(decreased(&assumed_reset)), 0, "a reset is no finding");
        assert_eq!(code(rate(&assumed_reset)), 0, "no rate, with why");
        assert_eq!(
            code(decreased(&pair(
                Continuity::Unobservable,
                None,
                Err(NoRate::Kind)
            ))),
            2
        );
        let gauges = pair(Continuity::NotAsked, None, Err(NoRate::Kind));
        assert_eq!(code(decreased(&gauges)), 2);
        assert_eq!(code(rate(&gauges)), 2, "not asked");
    }
}
