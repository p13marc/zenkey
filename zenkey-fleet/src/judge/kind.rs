//! Declared versus observed `kind` (#422): the sensor that publishes
//! `oom_kills_total` as a gauge, and nothing could have caught it.
//!
//! RFC 08 §2 (v1.32) lets a subject declare what its leaf value *is* —
//! `counter | gauge | text | bool` — and RFC 13 §3 says what a judge owes
//! that declaration:
//!
//! - **Not asked** when the entry declares no `kind`. Nothing here runs for
//!   such a key; the doctor's listen phase never calls
//!   [`KindObservation::observe`] for it.
//! - **Established(no)** — `kind-mismatch`, severity Error — when a
//!   self-describing payload's `type` tag disagrees with the declared kind,
//!   or when a `counter` decreased between two samples of one origin with
//!   no `alive` cycle of that origin in between. The restart is the one
//!   sanctioned reset, and it is on the wire (RFC 04 §5): the listen phase
//!   watches the liveliness planes and reports a down-then-up through
//!   [`KindObservation::alive_cycled`].
//! - **Unobservable** when the payload could not be decoded, said with the
//!   reason — one Warning per key, never folded into a pass.
//!
//! Per origin, never pooled: a key already names its origin, so the table
//! is per key and the counter baseline is that key's own series. The window
//! rides in every finding (O5/O6) — a window in which a counter did not
//! decrease has not established that it is one, which is why there is no
//! `Established(yes)` here at all.
//!
//! Pure, the house pattern of [`crate::judge::field`] and
//! [`crate::judge::budget`]: nothing here takes a session, so the same
//! observation judges a `.zrec` replay.

use std::collections::BTreeMap;

use serde_json::Value;
use zenkey::{SliceToken, SubjectKind};

use crate::judge::common::{EXPANSION_CAP, FINDING_CAP};
use crate::model::examples::Examples;
use crate::report::{CheckId, DoctorFinding, DoctorSeverity};

/// The producer identity a liveliness token and a data key share:
/// `(origin, producer)`, with a service origin's producer being the origin
/// minus its `@` — exactly [`crate::bus::roster::token_identity`]'s reading
/// of `v1/@catalog/state/alive`, so a cycle seen on the token plane lands on
/// the keys it restarted.
pub type ProducerId = (String, String);

/// What one key's series has shown against its declared kind.
#[derive(Debug)]
pub struct KeyKind {
    /// Whose series this is — the cycle bookkeeping is per producer.
    pub producer: ProducerId,
    pub declared: SubjectKind,
    /// Samples that reached a verdict (decoded and read).
    pub judged: u64,
    /// Samples with a self-describing tag that disagreed with `declared`.
    pub tag_mismatches: u64,
    /// Samples whose value was not of the declared kind — a negative or
    /// non-numeric `counter`, a non-boolean `bool`, a non-string `text`.
    pub value_mismatches: u64,
    /// `counter` only: decreases with no `alive` cycle in between.
    pub decreases: u64,
    /// Samples that could not be decoded at all — unobservable, said so.
    pub undecoded: u64,
    /// Up to [`EXPANSION_CAP`] mismatches, spelled out.
    examples: Examples<String>,
    /// `counter` only: the last value read, the next sample's baseline.
    last: Option<f64>,
    /// The producer's cycle generation this key's baseline belongs to.
    generation: u64,
}

/// The per-key observation the listen phase feeds and [`judge_kind`] reads.
#[derive(Debug, Default)]
pub struct KindObservation {
    keys: BTreeMap<String, KeyKind>,
    /// How many `alive` cycles each producer has shown. A key whose baseline
    /// is from an older generation resets on its next sample instead of
    /// being judged against a series the restart ended — kept per producer
    /// rather than as a one-shot mark so every key of a restarted producer
    /// resets, not just the first one to publish (#422).
    cycles: BTreeMap<ProducerId, u64>,
}

/// The tag a self-describing payload carries, when it does
/// (`{"type": "<kind>", "value": …}`, RFC 08 §2 / RFC 11 §4). A tag outside
/// the four is not a disagreement — it is a payload this rule does not read.
fn payload_tag(doc: &Value) -> Option<SubjectKind> {
    doc.get("type")
        .and_then(Value::as_str)
        .and_then(SubjectKind::from_payload_tag)
}

/// The value the declaration is about: `value` when the payload is an
/// object carrying one, the payload itself otherwise.
fn leaf(doc: &Value) -> &Value {
    match doc.get("value") {
        Some(v) if doc.is_object() => v,
        _ => doc,
    }
}

/// A short spelling of a JSON value for the evidence line.
fn describe(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => format!("boolean {b}"),
        Value::Number(n) => format!("number {n}"),
        Value::String(s) if s.len() > 24 => format!("string {:?}…", &s[..24]),
        Value::String(s) => format!("string {s:?}"),
        Value::Array(a) => format!("array of {}", a.len()),
        Value::Object(o) => format!("object with {} field(s)", o.len()),
    }
}

impl KindObservation {
    pub fn new() -> KindObservation {
        KindObservation::default()
    }

    /// The producer `(origin, producer)` went down and came back — the
    /// sanctioned counter reset (RFC 08 §2). A lone `NodeUp` at window start
    /// is a token seen, not a cycle; the caller decides that.
    pub fn alive_cycled(&mut self, origin: &str, producer: &str) {
        *self
            .cycles
            .entry((origin.to_string(), producer.to_string()))
            .or_default() += 1;
    }

    /// One `Put` sample on a key whose registry entry declares `kind`.
    /// `doc` is the structural document, `None` when the payload could not
    /// be decoded (counted, reported as unobservable, never judged).
    pub fn observe(
        &mut self,
        key: &str,
        origin: &str,
        producer: &str,
        declared: SubjectKind,
        doc: Option<&Value>,
    ) {
        let producer_id = (origin.to_string(), producer.to_string());
        let generation = self.cycles.get(&producer_id).copied().unwrap_or(0);
        let entry = self.keys.entry(key.to_string()).or_insert_with(|| KeyKind {
            producer: producer_id,
            declared,
            judged: 0,
            tag_mismatches: 0,
            value_mismatches: 0,
            decreases: 0,
            undecoded: 0,
            examples: Examples::new(EXPANSION_CAP),
            last: None,
            generation,
        });
        let Some(doc) = doc else {
            entry.undecoded += 1;
            return;
        };
        entry.judged += 1;

        // (a) The tag, when the payload self-describes. A disagreeing tag is
        // the finding on its own; the value is not judged twice.
        if let Some(tag) = payload_tag(doc)
            && tag != declared
        {
            entry.tag_mismatches += 1;
            entry.examples.push_with(|| {
                format!(
                    "payload tags itself `{}`, registry declares `{}`",
                    tag.payload_tag(),
                    declared.token()
                )
            });
            return;
        }

        // (b) The value.
        let v = leaf(doc);
        match declared {
            SubjectKind::Gauge => {
                if v.as_f64().is_none() {
                    entry.value_mismatches += 1;
                    entry
                        .examples
                        .push_with(|| format!("gauge value is {}", describe(v)));
                }
            }
            SubjectKind::Bool => {
                if !v.is_boolean() {
                    entry.value_mismatches += 1;
                    entry
                        .examples
                        .push_with(|| format!("bool value is {}", describe(v)));
                }
            }
            SubjectKind::Text => {
                if !v.is_string() {
                    entry.value_mismatches += 1;
                    entry
                        .examples
                        .push_with(|| format!("text value is {}", describe(v)));
                }
            }
            SubjectKind::Counter => {
                let Some(n) = v.as_f64() else {
                    entry.value_mismatches += 1;
                    entry
                        .examples
                        .push_with(|| format!("counter value is {}", describe(v)));
                    return;
                };
                if n < 0.0 {
                    entry.value_mismatches += 1;
                    entry
                        .examples
                        .push_with(|| format!("counter value is negative ({n})"));
                }
                if entry.generation != generation {
                    // The producer cycled its `alive` token since this
                    // key's baseline: the series restarted, and this sample
                    // is the new baseline (RFC 08 §2, RFC 13 §3).
                    entry.generation = generation;
                    entry.last = Some(n);
                    return;
                }
                if let Some(prev) = entry.last
                    && n < prev
                {
                    entry.decreases += 1;
                    entry.examples.push_with(|| {
                        format!("counter decreased {prev} → {n} with no `alive` cycle in between")
                    });
                }
                entry.last = Some(n);
            }
        }
    }
}

impl KeyKind {
    /// The mismatches spelled out, up to [`EXPANSION_CAP`] of them.
    pub fn examples(&self) -> &[String] {
        self.examples.as_slice()
    }
}

impl KindObservation {
    /// Every key observed, with what its series showed.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &KeyKind)> {
        self.keys.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys_seen(&self) -> usize {
        self.keys.len()
    }
}

/// The `kind-mismatch` findings for one listen window: one Error per key
/// whose series disagreed with its declaration, one Warning per key whose
/// payloads could not be judged, both capped at the doctor's per-check
/// finding cap with the remainder counted. Filter **then** cap, like every
/// listen check.
pub fn judge_kind(observation: &KindObservation, window_s: f64) -> Vec<DoctorFinding> {
    let mut findings = Vec::new();
    let mut bad: Examples<DoctorFinding> = Examples::new(FINDING_CAP);
    let mut unjudged: Examples<DoctorFinding> = Examples::new(FINDING_CAP);
    for (key, k) in observation.iter() {
        let mismatches = k.tag_mismatches + k.value_mismatches + k.decreases;
        if mismatches > 0 {
            bad.push_with(|| {
                let mut parts = Vec::new();
                if k.tag_mismatches > 0 {
                    parts.push(format!("{} tag disagreement(s)", k.tag_mismatches));
                }
                if k.decreases > 0 {
                    parts.push(format!("{} decrease(s)", k.decreases));
                }
                if k.value_mismatches > 0 {
                    parts.push(format!("{} value(s) not of that kind", k.value_mismatches));
                }
                let examples = k.examples.as_slice().join("; ");
                DoctorFinding {
                    severity: DoctorSeverity::Error,
                    check: CheckId::KindMismatch,
                    subject: key.to_string(),
                    evidence: format!(
                        "declared `{}`, and {} of {} sample(s) from origin {} in {window_s:.0}s \
                         disagree: {} — e.g. {examples}",
                        k.declared.token(),
                        mismatches,
                        k.judged,
                        k.producer.0,
                        parts.join(", "),
                    ),
                    citation: Some("RFC 08 §2".into()),
                }
            });
        }
        if k.undecoded > 0 {
            unjudged.push_with(|| DoctorFinding {
                severity: DoctorSeverity::Warning,
                check: CheckId::KindMismatch,
                subject: key.to_string(),
                evidence: format!(
                    "kind not judged: {} payload(s) from origin {} in {window_s:.0}s could \
                     not be decoded, so the declared `{}` is unobservable for them",
                    k.undecoded,
                    k.producer.0,
                    k.declared.token(),
                ),
                citation: Some("RFC 13 §3".into()),
            });
        }
    }
    for (ex, tail) in [
        (bad, "more key(s) with the same finding"),
        (unjudged, "more key(s) whose kind was not judged"),
    ] {
        let more = ex.more(tail);
        findings.extend(ex.into_vec());
        if let Some(evidence) = more {
            findings.push(DoctorFinding {
                severity: DoctorSeverity::Info,
                check: CheckId::KindMismatch,
                subject: "fleet".into(),
                evidence,
                citation: None,
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const KEY: &str = "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/memory/oom_kills_total";

    fn observe(obs: &mut KindObservation, declared: SubjectKind, doc: Value) {
        obs.observe(KEY, "h-aaaaaaaaaaaa", "sysinfo", declared, Some(&doc));
    }

    fn mismatches(obs: &KindObservation) -> Vec<DoctorFinding> {
        judge_kind(obs, 10.0)
            .into_iter()
            .filter(|f| f.severity == DoctorSeverity::Error)
            .collect()
    }

    /// A counter that goes down is the finding, named per origin with the
    /// window; a counter that only goes up is not a pass — it is nothing.
    #[test]
    fn a_decreasing_counter_is_a_finding_and_a_rising_one_is_nothing() {
        let mut obs = KindObservation::new();
        for n in [10, 20, 30] {
            observe(&mut obs, SubjectKind::Counter, json!(n));
        }
        assert!(
            judge_kind(&obs, 10.0).is_empty(),
            "no Established(yes) exists"
        );

        observe(&mut obs, SubjectKind::Counter, json!(5));
        let f = mismatches(&obs);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].check, CheckId::KindMismatch);
        assert_eq!(f[0].subject, KEY);
        assert!(
            f[0].evidence.contains("h-aaaaaaaaaaaa"),
            "{}",
            f[0].evidence
        );
        assert!(f[0].evidence.contains("10s"), "the window is stated");
        assert!(f[0].evidence.contains("30 → 5"), "{}", f[0].evidence);
        assert_eq!(f[0].citation.as_deref(), Some("RFC 08 §2"));
    }

    /// The restart is the one sanctioned reset (RFC 08 §2): a cycle of the
    /// producer's `alive` token between the samples makes the drop a new
    /// baseline, for every key of that producer, and a cycle of *another*
    /// producer excuses nothing.
    #[test]
    fn a_reset_across_an_alive_cycle_is_not_a_finding() {
        let mut obs = KindObservation::new();
        let other = "v1/h-aaaaaaaaaaaa/telemetry/sysinfo/memory/page_faults_total";
        observe(&mut obs, SubjectKind::Counter, json!(10));
        obs.observe(
            other,
            "h-aaaaaaaaaaaa",
            "sysinfo",
            SubjectKind::Counter,
            Some(&json!(7)),
        );
        obs.alive_cycled("h-aaaaaaaaaaaa", "sysinfo");
        observe(&mut obs, SubjectKind::Counter, json!(0));
        obs.observe(
            other,
            "h-aaaaaaaaaaaa",
            "sysinfo",
            SubjectKind::Counter,
            Some(&json!(0)),
        );
        assert!(mismatches(&obs).is_empty(), "{:?}", judge_kind(&obs, 10.0));

        // After the reset the new baseline holds: a later drop is judged.
        observe(&mut obs, SubjectKind::Counter, json!(3));
        observe(&mut obs, SubjectKind::Counter, json!(1));
        assert_eq!(mismatches(&obs).len(), 1);

        // Another producer's cycle is not this one's restart.
        let mut obs = KindObservation::new();
        observe(&mut obs, SubjectKind::Counter, json!(10));
        obs.alive_cycled("h-aaaaaaaaaaaa", "netring");
        observe(&mut obs, SubjectKind::Counter, json!(0));
        assert_eq!(mismatches(&obs).len(), 1);
    }

    /// A self-describing payload's tag must agree (RFC 08 §2); `boolean` is
    /// how `bool` is spelled on the wire (RFC 11 §4), and an unknown tag is
    /// not a disagreement.
    #[test]
    fn a_disagreeing_tag_is_a_finding_and_an_agreeing_or_foreign_one_is_not() {
        let mut obs = KindObservation::new();
        observe(
            &mut obs,
            SubjectKind::Counter,
            json!({"type": "gauge", "value": 1}),
        );
        let f = mismatches(&obs);
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(
            f[0].evidence.contains("tags itself `gauge`"),
            "{}",
            f[0].evidence
        );

        let mut obs = KindObservation::new();
        observe(
            &mut obs,
            SubjectKind::Bool,
            json!({"type": "boolean", "value": true}),
        );
        observe(
            &mut obs,
            SubjectKind::Bool,
            json!({"type": "histogram", "value": true}),
        );
        assert!(mismatches(&obs).is_empty());
    }

    /// The value itself: a `text` that is a number, a `bool` that is a
    /// string, a negative `counter` — bare or under `value`.
    #[test]
    fn a_value_not_of_the_declared_kind_is_a_finding() {
        let mut obs = KindObservation::new();
        observe(&mut obs, SubjectKind::Text, json!(3));
        observe(&mut obs, SubjectKind::Text, json!({"value": "ok"}));
        assert_eq!(mismatches(&obs).len(), 1);
        let mut obs = KindObservation::new();
        observe(&mut obs, SubjectKind::Bool, json!({"value": "true"}));
        assert_eq!(mismatches(&obs).len(), 1);
        let mut obs = KindObservation::new();
        observe(&mut obs, SubjectKind::Counter, json!(-1));
        assert_eq!(mismatches(&obs).len(), 1);
        let mut obs = KindObservation::new();
        observe(&mut obs, SubjectKind::Gauge, json!(-1.5));
        observe(&mut obs, SubjectKind::Gauge, json!({"value": 2}));
        assert!(mismatches(&obs).is_empty(), "a gauge is any number");
    }

    /// An undecodable payload is unobservable, said so per key as a Warning
    /// — never a pass, never an Error (RFC 13 §3).
    #[test]
    fn an_undecodable_payload_is_reported_unobservable_not_passed() {
        let mut obs = KindObservation::new();
        obs.observe(KEY, "h-aaaaaaaaaaaa", "sysinfo", SubjectKind::Counter, None);
        obs.observe(KEY, "h-aaaaaaaaaaaa", "sysinfo", SubjectKind::Counter, None);
        let f = judge_kind(&obs, 10.0);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].severity, DoctorSeverity::Warning);
        assert_eq!(f[0].check, CheckId::KindMismatch);
        assert!(
            f[0].evidence.contains("kind not judged: 2 payload(s)"),
            "{}",
            f[0].evidence
        );
        assert!(f[0].evidence.contains("10s"));
    }

    /// The cap bounds the findings and the remainder counts what it hid —
    /// the note cannot disagree with the population it summarises.
    #[test]
    fn the_cap_bites_with_a_counted_remainder() {
        let mut obs = KindObservation::new();
        for i in 0..(FINDING_CAP + 3) {
            let key = format!("v1/h-aaaaaaaaaaaa/telemetry/sysinfo/k{i}");
            obs.observe(
                &key,
                "h-aaaaaaaaaaaa",
                "sysinfo",
                SubjectKind::Text,
                Some(&json!(1)),
            );
        }
        let f = judge_kind(&obs, 10.0);
        assert_eq!(f.len(), FINDING_CAP + 1, "{f:?}");
        let tail = f.last().unwrap();
        assert_eq!(tail.severity, DoctorSeverity::Info);
        assert!(tail.evidence.contains("… and 3 more"), "{}", tail.evidence);
    }
}
