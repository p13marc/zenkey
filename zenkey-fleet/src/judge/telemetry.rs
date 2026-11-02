//! `telemetry.v1` (#819, TC): a reading of metric members judged, every
//! verdict decided from values (`spec/profiles/telemetry/v1.md` §5).
//!
//! **Session-free.** [`judge`] reads a [`TelemetryObservation`] that
//! [`crate::bus::telemetry`] gathered and hands back a [`TelemetryReport`].
//! Every answer is `zenkey_model::telemetry`'s, through the book
//! ([`crate::model::telemetry`]): `declaration_of` per resource, `check` per
//! sample, `pair` per pair.
//!
//! **§5's five questions as judgements** (the tooling guide's exit codes, §1, through
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
//!   established epoch — a decrease read as a reset is no. Over one
//!   member's pairs ([`member_decreased`], §5, 0.4), the no needs an
//!   established pair: a counter or histogram that formed no pair, or only
//!   pairs whose epoch was assumed, could not tell;
//! - "what is this counter's rate over the pair?" ([`rate`]): informative.
//!
//! A frame resource's series (`telemetry.v2`, #983 T2) answer the same
//! questions, each its own member: its declaration is its
//! `telemetry.series` ([`frame_declaration_row`]), a sample an included
//! value, which conforms, and a narrow counter's pair outside its serial
//! window `wrap_or_decrease`, unobservable ([`unobservable_words`]), so the
//! series could not tell ([`member_decreased`], v2 §5).
//!
//! The two unestablished kinds stay apart in every one (core O5). Of the
//! five, only a sample that does not conform and a decrease are the run's
//! findings, and a member whose continuity could not be told, or a sample
//! whose conformance could not (undecodable, or under an invalid
//! declaration, 0.5), leaves the run without a verdict
//! ([`TelemetryReport::judgement`]).

use zenkey_model::telemetry::{
    Basis, Checked, Conformance, Continuity, Declaration, Measure, NoRate, Pair, TimeBasis, Via,
};

use crate::bus::telemetry::TelemetryObservation;
use crate::model::presence_live::PresenceBasis;
use crate::model::telemetry::{
    FrameMetric, MemberBook, MetricResource, PairPresence, PairSeen, invalid_words,
};
use crate::report::{
    ContinuityToken, DeclarationToken, EpochToken, Judgement, PairPresenceToken,
    TelemetryContinuity, TelemetryDeclaration, TelemetryMember, TelemetryPair, TelemetryPresence,
    TelemetryRate, TelemetryRateBound, TelemetryReport, TelemetryResource, TelemetrySamples,
    TelemetrySeries, TelemetrySeriesProblem, TelemetryTotals,
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
            reason: unobservable_words(p).to_owned(),
        },
        Continuity::NotAsked => Judgement::NotAsked,
    }
}

/// Why an unobservable pair is (`Pair::unobservable`), in words: a narrow
/// counter's change outside its serial window (`telemetry.v2` §2.20), or
/// two samples that cannot be compared (§2.10).
pub fn unobservable_words(p: &Pair) -> &'static str {
    match p.unobservable() {
        Some("wrap_or_decrease") => {
            "wrap_or_decrease: a narrow counter's change outside its serial window, a wrap or a \
             decrease its two values cannot tell apart (telemetry.v2 §2.20)"
        }
        _ => {
            "incomparable: the kinds, a counter's widths, or a histogram's bounds or length, differ"
        }
    }
}

/// "Did this counter or histogram go backwards within one epoch?" over every
/// pair of one member in the window (§5, 0.4; zk2py round 47's F-152). A
/// decrease within an established epoch is the yes, the finding. The no
/// needs evidence: at least one pair whose epoch was established, in-band
/// or by presence, and none decreased. A counter or histogram that formed
/// no pair, or only pairs whose epoch was assumed or that were
/// incomparable, could not tell: no decrease could have been found.
/// Neither a counter nor a histogram, by the member's kind and its pairs,
/// is not asked. A member whose declaration is invalid could not tell
/// either (§2.2, 0.5; zk2py round 48's F-157): no question of its members
/// is answered. `reads_presence` says why an assumed epoch was: no presence
/// read (across a face), or none established.
pub fn member_decreased(m: &MemberBook, reads_presence: bool) -> Judgement {
    let t = &m.tally;
    if t.decreased > 0 {
        return Judgement::Established;
    }
    if let Some(reason) = m.metric.invalid_words() {
        return Judgement::Unobservable { reason };
    }
    let cumulative =
        m.kind.is_some_and(|(k, _)| k.is_cumulative()) || t.asked() + t.unobservable > 0;
    if !cumulative {
        return Judgement::NotAsked;
    }
    // A wrap or a decrease within one run, which a narrow counter's values
    // cannot tell apart: an established pair beside it answers nothing of
    // it (telemetry.v2 §5).
    if t.wrap_or_decrease > 0 {
        let width = m
            .field
            .as_ref()
            .and_then(|f| f.width)
            .map_or(String::new(), |w| format!(" {w}-bit"));
        return Judgement::Unobservable {
            reason: format!(
                "{} of its {} pair(s) wrap_or_decrease: the{width} counter's change lay outside \
                 its serial window, a wrap or a decrease its values cannot tell apart, and none \
                 decreased (telemetry.v2 §2.20)",
                t.wrap_or_decrease,
                t.total()
            ),
        };
    }
    let established = t.in_band + t.by_presence;
    if established > 0 {
        return Judgement::NotEstablished {
            reason: format!(
                "{established} pair(s) whose epoch was established ({} in-band, {} by presence), \
                 none decreased; {} assumed, {} reset",
                t.in_band, t.by_presence, t.assumed, t.reset
            ),
        };
    }
    let reason = if t.total() == 0 && m.field.is_some() {
        // A frame's series (telemetry.v2 §5): fewer than two samples, or
        // unknown throughout.
        format!(
            "no pair formed: {} sample(s) of it in the window, and a pair needs two{}",
            m.conforming,
            no_sample_words(m)
        )
    } else if t.total() == 0 {
        format!(
            "no pair formed: {} conforming sample(s) of it in the window, and a pair needs two",
            m.conforming
        )
    } else if t.assumed > 0 {
        format!(
            "every one of its {} pair(s) had its epoch assumed: no in-band epoch, and {}",
            t.assumed,
            if reads_presence {
                "presence established none"
            } else {
                "no presence read"
            }
        )
    } else {
        format!(
            "its {} pair(s) incomparable: the kinds, a counter's widths, or a histogram's bounds \
             or length, differ",
            t.unobservable
        )
    };
    Judgement::Unobservable { reason }
}

/// A frame series' records that were no sample, as a clause: `""` when
/// none, else "; 4 of its records were no sample: 1 held, 3 unknown"
/// (`telemetry.v2` §2.18), the readings' tokens.
fn no_sample_words(m: &MemberBook) -> String {
    if m.no_sample.is_empty() {
        return String::new();
    }
    format!(
        "; {} of its records were no sample: {}",
        m.no_sample.values().sum::<u64>(),
        m.no_sample
            .iter()
            .map(|(t, n)| format!("{n} {t}"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A `scheduled` rate's bound (`telemetry.v2` §2.21): the rate over the
/// elapsed time plus its jitter, and over it less its jitter, `None` past
/// it. `None` for any other basis.
pub fn rate_bound(r: &zenkey_model::telemetry::Rate) -> Option<TelemetryRateBound> {
    let Some(TimeBasis::Scheduled { jitter_ms }) = r.basis else {
        return None;
    };
    let e = r.elapsed_ns as f64;
    let j = jitter_ms as f64 * 1e6;
    Some(TelemetryRateBound {
        low: r.per_s * e / (e + j),
        high: (e > j).then(|| r.per_s * e / (e - j)),
    })
}

/// A time basis' token and a `scheduled` one's jitter (v2 §2.21).
fn basis_row(b: Option<TimeBasis>) -> (Option<String>, Option<u64>) {
    match b {
        None => (None, None),
        Some(b) => (
            Some(b.as_str().to_owned()),
            match b {
                TimeBasis::Scheduled { jitter_ms } => Some(jitter_ms),
                _ => None,
            },
        ),
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

/// A frame resource's `telemetry.series` in words (v2 §2.16).
pub fn frame_declaration_says(f: &FrameMetric) -> String {
    match (&f.series, f.declared()) {
        (_, Some(series)) => format!(
            "declared (telemetry.v2): a frame's {} series, {}; each epoch its run, in band",
            series.len(),
            series
                .iter()
                .map(|s| {
                    let mut w = format!("{} a {}", s.field, s.kind);
                    if let Some(b) = s.width.filter(|b| *b < 64) {
                        w.push_str(&format!(" of {b} bits"));
                    }
                    match &s.unit {
                        Some(u) => w.push_str(&format!(" in {u}")),
                        None => w.push_str(", unit unknown"),
                    }
                    if let Some(m) = &s.mirror {
                        w.push_str(&format!(", mirroring {m}"));
                    }
                    w
                })
                .collect::<Vec<_>>()
                .join("; ")
        ),
        _ => format!(
            "invalid: {} — no field of it is read, so its series' samples and continuity are \
             unobservable (telemetry.v2 §2.16)",
            f.problems()
                .map(crate::model::telemetry::series_problem_words)
                .unwrap_or_default()
        ),
    }
}

/// A frame resource's declaration, as the report spells it (v2 §2.16).
pub fn frame_declaration_row(f: &FrameMetric) -> TelemetryDeclaration {
    let says = frame_declaration_says(f);
    let problems: Vec<TelemetrySeriesProblem> = f
        .problems()
        .unwrap_or_default()
        .iter()
        .map(|p| TelemetrySeriesProblem {
            entry: p.entry.clone(),
            reason: p.reason.as_str().to_owned(),
        })
        .collect();
    TelemetryDeclaration {
        answer: if f.declared().is_some() {
            DeclarationToken::Declared
        } else {
            DeclarationToken::Invalid
        },
        reasons: problems.iter().map(|p| p.reason.clone()).collect(),
        carrier: f.declared().map(|_| "frame".to_owned()),
        kind: None,
        unit: None,
        buckets: None,
        semantic: None,
        epoch_in: None,
        series: f
            .declared()
            .unwrap_or_default()
            .iter()
            .map(|s| TelemetrySeries {
                field: s.field.clone(),
                kind: s.kind.as_str().to_owned(),
                unit: s.unit.clone(),
                semantic: s.semantic.map(|x| x.as_str().to_owned()),
                mirrors: s.mirror.clone(),
                width: s.width,
            })
            .collect(),
        problems,
        says,
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
            series: Vec::new(),
            problems: Vec::new(),
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
            series: Vec::new(),
            problems: Vec::new(),
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
            series: Vec::new(),
            problems: Vec::new(),
            says,
        },
    }
}

/// A conforming value as its kind carries it, for a report.
pub fn measure_json(m: &Measure) -> serde_json::Value {
    use serde_json::json;
    match m {
        Measure::Counter { value, .. } => json!(value),
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
        time_basis: basis_row(p.basis).0,
        jitter_ms: basis_row(p.basis).1,
    }
}

/// One member, as the report spells it. `reads_presence` as
/// [`member_decreased`] takes it.
pub fn member_row(key: &str, m: &MemberBook, reads_presence: bool) -> TelemetryMember {
    let r = &m.metric;
    TelemetryMember {
        key: key.to_owned(),
        address: r.address.to_string(),
        iface: r.iface.to_string(),
        resource: r.resource.clone(),
        field: m.field.as_ref().map(|f| f.field.clone()),
        mirrors: m.field.as_ref().and_then(|f| f.mirror.clone()),
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
            no_sample: m
                .no_sample
                .iter()
                .map(|(t, n)| ((*t).to_owned(), *n))
                .collect(),
        },
        latest: m
            .latest_json
            .clone()
            .or_else(|| m.latest.as_ref().map(|r| measure_json(&r.value))),
        continuity: TelemetryContinuity {
            answer: member_decreased(m, reads_presence),
            pairs: m.tally.total(),
            pairs_listed: m.pairs.len() as u64,
            same_epoch: m.tally.same_epoch,
            reset: m.tally.reset,
            decreased: m.tally.decreased,
            unobservable: m.tally.unobservable,
            wrap_or_decrease: m.tally.wrap_or_decrease,
            not_asked: m.tally.not_asked,
            in_band: m.tally.in_band,
            by_presence: m.tally.by_presence,
            assumed: m.tally.assumed,
        },
        pairs: m.pairs.iter().map(pair_row).collect(),
        rate: m.rate.as_ref().map(|r| TelemetryRate {
            per_s: r.per_s,
            unit: r.unit.clone(),
            time_basis: basis_row(r.basis).0,
            jitter_ms: basis_row(r.basis).1,
            bound: rate_bound(r),
        }),
    }
}

fn same_resource(a: &MetricResource, b: &MetricResource) -> bool {
    a.address == b.address && a.iface == b.iface && a.resource == b.resource
}

/// Every verdict of one reading.
pub fn judge(obs: &TelemetryObservation) -> TelemetryReport {
    let book = &obs.book;
    let members: Vec<TelemetryMember> = book
        .members()
        .map(|(k, m)| member_row(k, m, book.reads_presence()))
        .collect();
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
            declaration: match &r.frame {
                Some(f) => frame_declaration_row(f),
                None => declaration_row(&r.ty.declaration),
            },
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
    totals.unanswered = members
        .iter()
        .filter(|m| m.continuity.answer.is_unobservable())
        .count() as u64;
    let mut skipped = obs.skipped.clone();
    if let Some(Ok(s)) = &obs.seed
        && s.unsettled
    {
        skipped.push(format!(
            "the last presence seed {}; the pairs since it have no presence",
            crate::bus::presence::BARRIER_NOT_ESTABLISHED
        ));
    }
    let presence = match &obs.seed {
        Some(Ok(s)) => Some(TelemetryPresence {
            selector: s.selector.clone(),
            complete: s.complete,
            cut_off_during: s.cut_off_during,
            services: s.services,
            basis: match book.presence_basis() {
                Some(PresenceBasis::Complete) => "complete",
                Some(PresenceBasis::Incomplete { .. }) => "incomplete",
                Some(PresenceBasis::Lost { .. }) => "lost",
                Some(PresenceBasis::CutOff) => "cut_off",
                Some(PresenceBasis::Moved) => "moved",
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
        scope: obs.reach.clone(),
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

    /// zk2py round 47's F-152 (§5, 0.4), through the book over `meter.v1`:
    /// the continuity question is each counter's or histogram's. One that
    /// formed no pair, or only pairs whose epoch was assumed, could not
    /// tell, and the run says so (exit 2) even beside another member's
    /// established pair; a decrease or a sample that does not conform still
    /// wins (exit 1); a gauge is not asked.
    #[test]
    fn a_counter_that_formed_no_pair_could_not_tell() {
        use crate::bus::telemetry::{TelemetryObservation, TelemetrySpec};
        use crate::model::catalog::{ContractSet, Observed};
        use crate::model::telemetry::{
            MetricResource, MetricSample, SampleVia, TelemetryBook, TelemetryScope,
        };
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/meter.v1.toml");
        let (set, _) = ContractSet::load_path(&path);
        let rev = Arc::clone(set.iter().next().expect("meter.v1"));
        let addr: zenkey_model::grammar::Addr = "lab/meter".parse().unwrap();
        let resources: Vec<Arc<MetricResource>> = (0..rev.contract().resources.len())
            .filter_map(|i| MetricResource::of(&addr, &rev, i))
            .map(Arc::new)
            .collect();
        let of = |r: &str| Arc::clone(resources.iter().find(|m| m.resource == r).unwrap());
        let t0 = Instant::now();
        let put = |r: &str, payload: &str, epoch: Option<&str>, ms: u64| MetricSample {
            key: format!("zk2/lab/meter/meter.v1/{r}"),
            delete: false,
            payload: payload.as_bytes().to_vec(),
            attachment: epoch.map(|e| format!("{e:?}").into_bytes()),
            stamp: None,
            at: t0 + Duration::from_millis(ms),
            via: SampleVia::Subscription,
            frame: None,
        };
        let book = |presence: bool| {
            let mut b = TelemetryBook::new(
                presence.then(|| crate::bus::presence::Scope::service(&addr)),
                t0,
            );
            b.seed(
                &Observed::from_keys(
                    "zk2/lab/meter/@zk/**",
                    &["zk2/lab/meter/@zk/instance/00000000000000a1".to_owned()],
                    true,
                ),
                std::iter::empty::<(&str, bool)>(),
            );
            b
        };
        let judged = |book: TelemetryBook, across_face: bool| {
            let mut spec = TelemetrySpec::new(Duration::from_secs(1), Duration::from_secs(2));
            spec.across_face = across_face;
            judge(&TelemetryObservation {
                namespace: "acme".into(),
                spec,
                scope: TelemetryScope::of(&addr),
                resources: resources.clone(),
                seed: None,
                book,
                asked: vec![],
                listened: Duration::from_secs(2),
                unresolved: 0,
                skipped: vec![],
                reach: None,
                unobservable: None,
                opening: None,
            })
        };
        let answer = |r: &TelemetryReport, key: &str| {
            r.members
                .iter()
                .find(|m| m.key.ends_with(key))
                .unwrap_or_else(|| panic!("{key}: {r:#?}"))
                .continuity
                .answer
                .clone()
        };
        let code = |r: &TelemetryReport| judgement_exit_code(&r.judgement());

        // One conforming sample of `drops` and no pair: could not tell.
        let mut b = book(true);
        b.sample(&of("stream/drops"), put("stream/drops", "10", None, 0));
        let r = judged(b.clone(), false);
        assert!(
            matches!(answer(&r, "drops"), Judgement::Unobservable { reason }
                if reason.starts_with("no pair formed: 1 conforming sample(s)")),
            "{r:#?}"
        );
        assert_eq!(
            (r.totals.unanswered, code(&r)),
            (1, 2),
            "{:?}",
            r.judgement()
        );

        // A gauge beside it is not asked, and changes nothing.
        b.sample(&of("stream/level"), put("stream/level", "-70.5", None, 0));
        b.sample(&of("stream/level"), put("stream/level", "-70.0", None, 300));
        let r = judged(b.clone(), false);
        assert_eq!(answer(&r, "level"), Judgement::NotAsked);
        assert_eq!(code(&r), 2);

        // `bytes`, established in-band, answers for itself and not for
        // `drops`.
        b.sample(&of("stream/bytes"), put("stream/bytes", "1", Some("e1"), 0));
        b.sample(
            &of("stream/bytes"),
            put("stream/bytes", "5", Some("e1"), 300),
        );
        let r = judged(b.clone(), false);
        assert!(
            matches!(answer(&r, "bytes"), Judgement::NotEstablished { .. }),
            "{r:#?}"
        );
        assert_eq!((r.totals.unanswered, code(&r)), (1, 2));

        // `drops`' second sample, its epoch by presence: every counter
        // answered, and nothing went backwards.
        b.sample(&of("stream/drops"), put("stream/drops", "12", None, 300));
        let r = judged(b.clone(), false);
        assert!(matches!(
            answer(&r, "drops"),
            Judgement::NotEstablished { .. }
        ));
        assert_eq!(
            (r.totals.unanswered, code(&r)),
            (0, 0),
            "{:?}",
            r.judgement()
        );

        // Across a face, with no presence read, `drops`' pair is assumed:
        // could not tell, whatever `bytes` established in-band.
        let mut b = book(false);
        for (n, ms) in [("10", 0), ("12", 300)] {
            b.sample(&of("stream/drops"), put("stream/drops", n, None, ms));
        }
        for (n, ms) in [("1", 0), ("5", 300)] {
            b.sample(&of("stream/bytes"), put("stream/bytes", n, Some("e1"), ms));
        }
        let r = judged(b.clone(), true);
        assert!(
            matches!(answer(&r, "drops"), Judgement::Unobservable { reason }
                if reason.contains("assumed") && reason.contains("no presence read")),
            "{r:#?}"
        );
        assert_eq!(code(&r), 2);

        // A decrease within an established epoch is the finding, beside a
        // counter that could not tell.
        b.sample(
            &of("stream/bytes"),
            put("stream/bytes", "3", Some("e1"), 600),
        );
        let r = judged(b, true);
        assert_eq!(answer(&r, "bytes"), Judgement::Established);
        assert_eq!(code(&r), 1);
    }
    /// zk2py round 48's F-157 (§5, 0.5): a sample that could not be read
    /// leaves "did every sample conform?" unanswered. An undecodable sample
    /// of `level` beside its conforming ones, or a member under an invalid
    /// declaration, makes the run say it could not tell (exit 2), naming the
    /// member, or the resource with its declaration's reasons; a sample that
    /// does not conform still wins (exit 1).
    #[test]
    fn an_unreadable_sample_could_not_tell() {
        use crate::bus::telemetry::{TelemetryObservation, TelemetrySpec};
        use crate::model::catalog::{ContractSet, Revision};
        use crate::model::telemetry::{
            MetricResource, MetricSample, SampleVia, TelemetryBook, TelemetryScope,
        };
        use crate::report::ContractSource;
        use std::sync::Arc;
        use std::time::{Duration, Instant};
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../zenkey/tests/contracts/meter.v1.toml");
        let (set, _) = ContractSet::load_path(&path);
        let meter = Arc::clone(set.iter().next().expect("meter.v1"));
        let odd = zenkey_model::contract::load_str(
            "[interface]\nname = \"odd\"\nmajor = 1\nuses = [\"telemetry.v1\"]\n\
             [resources.blob]\nkind = \"stream\"\ntype = { raw = \"application/octet-stream\" }\n\
             annotations = { \"telemetry.kind\" = \"counter\" }\n",
            std::path::Path::new("."),
            None,
        );
        let odd = Arc::new(Revision::from_contract(
            odd.contract.expect("odd.v1 loads"),
            ContractSource::File,
        ));
        let addr: zenkey_model::grammar::Addr = "lab/meter".parse().unwrap();
        let resources: Vec<Arc<MetricResource>> = [&meter, &odd]
            .into_iter()
            .flat_map(|rev| {
                (0..rev.contract().resources.len())
                    .filter_map(|i| MetricResource::of(&addr, rev, i))
                    .collect::<Vec<_>>()
            })
            .map(Arc::new)
            .collect();
        let of = |r: &str| Arc::clone(resources.iter().find(|m| m.resource == r).unwrap());
        let t0 = Instant::now();
        let put = |iface: &str, r: &str, payload: &str, ms: u64| MetricSample {
            key: format!("zk2/lab/meter/{iface}/{r}"),
            delete: false,
            payload: payload.as_bytes().to_vec(),
            attachment: None,
            stamp: None,
            at: t0 + Duration::from_millis(ms),
            via: SampleVia::Subscription,
            frame: None,
        };
        let judged = |book: TelemetryBook| {
            let mut spec = TelemetrySpec::new(Duration::from_secs(1), Duration::from_secs(2));
            spec.across_face = true;
            judge(&TelemetryObservation {
                namespace: "acme".into(),
                spec,
                scope: TelemetryScope::of(&addr),
                resources: resources.clone(),
                seed: None,
                book,
                asked: vec![],
                listened: Duration::from_secs(2),
                unresolved: 0,
                skipped: vec![],
                reach: None,
                unobservable: None,
                opening: None,
            })
        };
        let reason = |r: &TelemetryReport| match r.judgement() {
            Judgement::Unobservable { reason } => reason,
            j => panic!("not unobservable: {j:?}"),
        };

        // A gauge's conforming samples alone answer every question asked.
        let mut b = TelemetryBook::new(None, t0);
        for (n, ms) in [("-70.5", 0), ("-70.0", 300)] {
            b.sample(&of("stream/level"), put("meter.v1", "stream/level", n, ms));
        }
        assert_eq!(judgement_exit_code(&judged(b.clone()).judgement()), 0);

        // An undecodable one beside them could not tell.
        b.sample(
            &of("stream/level"),
            put("meter.v1", "stream/level", "1e400", 600),
        );
        let r = judged(b.clone());
        assert_eq!(r.totals.unobservable, 1);
        assert!(
            reason(&r).contains("zk2/lab/meter/meter.v1/stream/level: 1 sample(s) undecodable"),
            "{r:#?}"
        );

        // A member under an invalid declaration could not tell either: its
        // samples and its continuity are unobservable.
        let mut b = TelemetryBook::new(None, t0);
        b.sample(
            &of("stream/level"),
            put("meter.v1", "stream/level", "-70.5", 0),
        );
        b.sample(&of("stream/blob"), put("odd.v1", "stream/blob", "x", 0));
        let r = judged(b.clone());
        let blob = r
            .members
            .iter()
            .find(|m| m.resource == "stream/blob")
            .unwrap();
        assert!(blob.continuity.answer.is_unobservable(), "{blob:#?}");
        assert!(
            reason(&r).contains("lab/meter odd.v1 stream/blob: its declaration is invalid"),
            "{r:#?}"
        );

        // A sample that does not conform is still the finding.
        b.sample(
            &of("stream/level"),
            put("meter.v1", "stream/level", "\"x\"", 300),
        );
        assert_eq!(judgement_exit_code(&judged(b).judgement()), 1);
    }
}
