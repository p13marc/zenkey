//! The exporter's ledger (#228, RFC 13 §3 *Exporter obligations*): samples
//! in, an [`ExportSnapshot`] out, and every blind spot counted on the way.
//!
//! Pure by the layer's rule — nothing here takes a session. The frontend
//! (`zenctl export`) decodes a sample structurally, describes its key, and
//! hands both to [`ExportLedger::ingest`]; a scrape is one
//! [`ExportLedger::fold`] over the monitor's counters, the statistics table,
//! the roster's departures and the last doctor run. A `.zrec` replayed
//! through the same two calls would fold to the same series.
//!
//! **A series is the contract, not the wire.** Its identity is `(origin,
//! producer, declared pattern, `{var}` bindings, field)`; its name and unit
//! are the registry's; a key that does not refine is counted under
//! `unregistered_keys` and never exported. Everything the ledger refuses —
//! a population past its declared `cardinality`, a series past
//! `max_series`, a field past the per-subject cap, a `text` kind, a payload
//! with no number in it — is counted by reason, because to a scraper a
//! series that was never made and a series that stopped look the same, and
//! only a count tells them apart.
//!
//! **Coalescing is the third O6 kind.** Between two folds only the newest
//! value per series survives; the samples folded into it are counted, and
//! the count rides the exposition beside the drops and the evictions.

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use serde_json::Value;

use crate::model::facts::{KeyFacts, KeyShape, Registration};
use crate::model::prom::metric_name;
use crate::model::retain::RetentionStats;
use crate::model::stats::StatsTable;
use crate::report::{
    Asked, ContractCounters, DoctorFindingRef, DoctorReport, DoctorSummary, ExportSnapshot,
    ObserverCounters, QosMismatchRow, RegistryInfo, SeriesRow, SeriesState,
};

/// How many top-level numeric fields one subject family may fan out into
/// when its payload is an object rather than one leaf value. Past it, the
/// overflow is counted under `suppressed["fields"]`.
pub const FIELD_CAP: usize = 16;

/// The default `--max-series` bound.
pub const DEFAULT_MAX_SERIES: usize = 10_000;

/// A payload verdict as the ledger counts it — the three populations, with
/// every reason a payload was *not* validated folded into the third
/// (RFC 13 §3: never a ratio that hides it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadVerdict {
    Valid,
    Invalid,
    NotValidated,
}

/// One sample, as the frontend hands it over.
#[derive(Debug, Clone, Copy)]
pub struct Observed<'a> {
    /// The full wire key.
    pub key: &'a str,
    /// `SampleKind::Delete` — a retirement tombstone (RFC 04 §1.2), never a
    /// value.
    pub delete: bool,
    /// The structural document, when the bytes carried one. `None` is
    /// *undecodable* and is counted as such.
    pub doc: Option<&'a Value>,
    /// Whether the wire's QoS axes matched the declared profile; `None`
    /// when the subject declares none this build knows (not judged).
    pub qos_matches: Option<bool>,
    pub verdict: PayloadVerdict,
    /// Arrival, unix seconds on the observer's clock.
    pub wall_unix_s: u64,
}

/// What one fold reads besides the ledger.
pub struct FoldInputs<'a> {
    pub stats: &'a StatsTable,
    pub retention: RetentionStats,
    /// The monitor's cumulative broadcast drops.
    pub dropped: u64,
    /// `(origin, producer)` pairs whose `alive` token the roster saw leave.
    pub down: &'a BTreeSet<(String, String)>,
    /// The last doctor run and when it finished, if one was asked for.
    pub doctor: Option<DoctorRun<'a>>,
    pub now: SystemTime,
}

/// A doctor report with the time it finished.
#[derive(Debug, Clone, Copy)]
pub struct DoctorRun<'a> {
    pub report: &'a DoctorReport,
    pub ran_at_unix_s: u64,
}

/// The contract identity of one series. Ordered, so the snapshot is
/// deterministic.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SeriesKey {
    producer: String,
    pattern: String,
    origin: String,
    bindings: Vec<(String, String)>,
    field: Option<String>,
}

#[derive(Debug, Clone)]
struct Series {
    name: String,
    key: String,
    class: String,
    kind: Option<String>,
    unit: Option<String>,
    ttl_s: Option<i64>,
    state_class: bool,
    last_value: f64,
    last_seen_unix_s: u64,
    samples: u64,
    since_fold: u64,
    drop_exposed: u64,
    retired: bool,
}

/// One `(origin, producer, pattern)` family's populations, for the two caps.
#[derive(Debug, Default)]
struct Family {
    bindings: BTreeSet<Vec<(String, String)>>,
    fields: BTreeSet<String>,
}

/// The ledger.
#[derive(Debug)]
pub struct ExportLedger {
    max_series: usize,
    series: BTreeMap<SeriesKey, Series>,
    families: BTreeMap<(String, String, String), Family>,
    unregistered: BTreeSet<String>,
    suppressed: BTreeMap<&'static str, u64>,
    contract: ContractCounters,
    qos_by_subject: BTreeMap<(String, String), u64>,
    last_dropped: u64,
    coalesced: u64,
    scopes: Vec<String>,
    excluded: Vec<String>,
    registry_producers: Option<usize>,
    started_at_unix_s: u64,
}

/// The planes a `*`/`**` selector cannot reach, named verbatim (RFC 03 §4
/// D2/D4; RFC 13 §3 O5).
pub const WILDCARD_EXCLUDES: [&str; 5] = ["@rpc", "@media", "@blob", "@adv", "service origins"];

/// What a wildcard scope leaves out: the verbatim planes when any scope
/// carries a wildcard, nothing when every scope is concrete.
pub fn excluded_by(scopes: &[String]) -> Vec<String> {
    if scopes.iter().any(|s| s.contains('*')) {
        WILDCARD_EXCLUDES.iter().map(|s| (*s).to_string()).collect()
    } else {
        Vec::new()
    }
}

fn unix_s(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The numeric reading of a JSON value: a number, or a bool as `0`/`1`.
fn numeric(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

impl ExportLedger {
    /// An empty ledger over `scopes`, started at `started_at`.
    ///
    /// `registry_producers` is `None` when no registry was loaded — then
    /// nothing refines, every key is unregistered, and the snapshot says
    /// the registry was not asked rather than that it declares nothing.
    pub fn new(
        max_series: usize,
        scopes: Vec<String>,
        registry_producers: Option<usize>,
        started_at: SystemTime,
    ) -> ExportLedger {
        ExportLedger {
            max_series,
            series: BTreeMap::new(),
            families: BTreeMap::new(),
            unregistered: BTreeSet::new(),
            suppressed: BTreeMap::new(),
            contract: ContractCounters::default(),
            qos_by_subject: BTreeMap::new(),
            last_dropped: 0,
            coalesced: 0,
            excluded: excluded_by(&scopes),
            scopes,
            registry_producers,
            started_at_unix_s: unix_s(started_at),
        }
    }

    fn suppress(&mut self, reason: &'static str) {
        *self.suppressed.entry(reason).or_default() += 1;
    }

    /// Distinct series held.
    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// Feed one sample. `facts` is the key's projection under the active
    /// base and registry ([`crate::describe_key`] or a
    /// [`crate::FactsCache`] entry).
    pub fn ingest(&mut self, obs: &Observed<'_>, facts: &KeyFacts) {
        let KeyShape::V1(v) = &facts.shape else {
            self.suppress("unparsed");
            return;
        };
        let sf = match &facts.registration {
            Registration::Registered(sf) => sf,
            Registration::Unknown
            | Registration::NoSliceForProducer
            | Registration::Unregistered => {
                // Bounded like the series map: past the bound the key is
                // counted under `max_series`, which is the bound it hit.
                if !self.unregistered.contains(obs.key) {
                    if self.unregistered.len() >= self.max_series {
                        self.suppress("max_series");
                    } else {
                        self.unregistered.insert(obs.key.to_string());
                    }
                }
                return;
            }
            Registration::NotApplicable => {
                self.suppress("unparsed");
                return;
            }
        };
        // A service origin omits the producer chunk (RFC 03 §1.5); the
        // roster spells its producer as the origin without the `@`
        // (`token_identity`), and the `down` set is matched against that.
        let producer = v
            .producer
            .clone()
            .unwrap_or_else(|| v.origin.trim_start_matches('@').to_string());

        // The contract counters count every sample that refined, tombstone
        // or not: a mismatched QoS on a delete is still a mismatch.
        if let Some(matched) = obs.qos_matches {
            self.contract.qos_judged += 1;
            if !matched {
                self.contract.qos_mismatch += 1;
                *self
                    .qos_by_subject
                    .entry((producer.clone(), sf.path.clone()))
                    .or_default() += 1;
            }
        }
        match obs.verdict {
            PayloadVerdict::Valid => self.contract.payload_valid += 1,
            PayloadVerdict::Invalid => self.contract.payload_invalid += 1,
            PayloadVerdict::NotValidated => self.contract.payload_not_validated += 1,
        }

        if obs.delete {
            // The producer's own statement: every series fed from this key
            // is retired. Labels stay; the value line goes.
            for s in self.series.values_mut() {
                if s.key == obs.key {
                    s.retired = true;
                    s.last_seen_unix_s = obs.wall_unix_s;
                    s.samples += 1;
                    s.since_fold += 1;
                }
            }
            return;
        }

        let kind = sf.kind.as_ref().map(|k| k.token().to_string());
        if kind.as_deref() == Some("text") {
            self.suppress("text");
            return;
        }
        let Some(doc) = obs.doc else {
            self.suppress("undecodable");
            return;
        };

        // The values: one leaf, or one per top-level numeric field.
        let mut values: Vec<(Option<String>, f64)> = Vec::new();
        if let Some(n) = numeric(doc) {
            values.push((None, n));
        } else if let Some(map) = doc.as_object() {
            match map.get("value") {
                // The RFC 11 §4 self-describing tag: the leaf is `value`.
                Some(leaf) => {
                    if let Some(n) = numeric(leaf) {
                        values.push((None, n));
                    }
                }
                None => {
                    for (k, val) in map {
                        if let Some(n) = numeric(val) {
                            values.push((Some(k.clone()), n));
                        }
                    }
                }
            }
        }
        if values.is_empty() {
            self.suppress("non_numeric");
            return;
        }

        let family_key = (v.origin.clone(), producer.clone(), sf.path.clone());
        let bindings: Vec<(String, String)> = sf.vars.clone();
        let family = self.families.entry(family_key).or_default();
        if !family.bindings.contains(&bindings) {
            // RFC 04 §1 bounds a `{var}` population per producer; a
            // literal subject's population is 1 by construction.
            if let Some(card) = sf.cardinality
                && !bindings.is_empty()
                && family.bindings.len() as i64 >= card
            {
                self.suppress("cardinality");
                return;
            }
            family.bindings.insert(bindings.clone());
        }
        // The field cap, decided while the family is borrowed; the count is
        // charged once the borrow ends.
        let mut fields_over = 0u64;
        let values: Vec<(Option<String>, f64)> = values
            .into_iter()
            .filter(|(field, _)| match field {
                None => true,
                Some(f) if family.fields.contains(f) => true,
                Some(f) if family.fields.len() < FIELD_CAP => {
                    family.fields.insert(f.clone());
                    true
                }
                Some(_) => {
                    fields_over += 1;
                    false
                }
            })
            .collect();
        for _ in 0..fields_over {
            self.suppress("fields");
        }

        let name = metric_name(&producer, &sf.path, sf.unit.as_deref(), kind.as_deref());
        let ttl_s = sf.ttl_s;
        let unit = sf.unit.clone();
        let state_class = v.class == "state";
        for (field, value) in values {
            let key = SeriesKey {
                producer: producer.clone(),
                pattern: sf.path.clone(),
                origin: v.origin.clone(),
                bindings: bindings.clone(),
                field,
            };
            match self.series.get_mut(&key) {
                Some(s) => {
                    s.last_value = value;
                    s.last_seen_unix_s = obs.wall_unix_s;
                    s.samples += 1;
                    s.since_fold += 1;
                    s.retired = false;
                    s.key = obs.key.to_string();
                }
                None => {
                    if self.series.len() >= self.max_series {
                        self.suppress("max_series");
                        continue;
                    }
                    self.series.insert(
                        key,
                        Series {
                            name: name.clone(),
                            key: obs.key.to_string(),
                            class: v.class.clone(),
                            kind: kind.clone(),
                            unit: unit.clone(),
                            ttl_s,
                            state_class,
                            last_value: value,
                            last_seen_unix_s: obs.wall_unix_s,
                            samples: 1,
                            since_fold: 1,
                            drop_exposed: 0,
                            retired: false,
                        },
                    );
                }
            }
        }
    }

    /// One scrape: the snapshot as of `inputs.now`.
    ///
    /// Two folds with no ingest between them differ only in
    /// `taken_at_unix_s` (and in a `quiet` judgement a ttl may have crossed)
    /// — the exposition carries neither the scrape time nor anything else
    /// that moves without traffic, which is what makes an idle scrape
    /// byte-identical.
    pub fn fold(&mut self, inputs: &FoldInputs<'_>) -> ExportSnapshot {
        // Drops since the last fold taint every series fed in the interval:
        // its value may not be the newest, and the row says so.
        let drops_moved = inputs.dropped > self.last_dropped;
        self.last_dropped = inputs.dropped;
        let now = unix_s(inputs.now);

        let mut rows = Vec::with_capacity(self.series.len());
        for (key, s) in self.series.iter_mut() {
            if s.since_fold > 0 {
                if drops_moved {
                    s.drop_exposed += 1;
                }
                self.coalesced += s.since_fold - 1;
                s.since_fold = 0;
            }
            let state = if s.retired {
                SeriesState::Retired
            } else if inputs
                .down
                .contains(&(key.origin.clone(), key.producer.clone()))
            {
                SeriesState::OriginDown
            } else if inputs.stats.get(&s.key).is_none() {
                SeriesState::Evicted
            } else if s.state_class
                && let Some(ttl) = s.ttl_s
                && ttl > 0
                && now.saturating_sub(s.last_seen_unix_s) > ttl as u64
            {
                SeriesState::Quiet
            } else {
                SeriesState::Live
            };
            rows.push(SeriesRow {
                name: s.name.clone(),
                key: s.key.clone(),
                origin: key.origin.clone(),
                producer: key.producer.clone(),
                class: s.class.clone(),
                subject: key.pattern.clone(),
                labels: key.bindings.iter().cloned().collect(),
                field: key.field.clone(),
                kind: s.kind.clone(),
                unit: s.unit.clone(),
                value: state.exposes_value().then_some(s.last_value),
                last_seen_unix_s: s.last_seen_unix_s,
                state,
                samples: s.samples,
                drop_exposed: s.drop_exposed,
            });
        }

        let unstamped = inputs.stats.iter().map(|(_, k)| k.unstamped).sum();
        let mut contract = self.contract.clone();
        contract.qos_mismatch_by_subject = self
            .qos_by_subject
            .iter()
            .map(|((producer, subject), n)| QosMismatchRow {
                producer: producer.clone(),
                subject: subject.clone(),
                n: *n,
            })
            .collect();

        ExportSnapshot {
            scopes: self.scopes.clone(),
            excluded: self.excluded.clone(),
            registry: self
                .registry_producers
                .map(|producers| RegistryInfo { producers })
                .into(),
            max_series: self.max_series,
            started_at_unix_s: self.started_at_unix_s,
            taken_at_unix_s: now,
            series: rows,
            observer: ObserverCounters {
                dropped: inputs.dropped,
                evicted_keys: inputs.stats.evicted(),
                evicted_bytes: inputs.retention.evicted,
                expired: inputs.retention.expired,
                unwatched: inputs.stats.unwatched(),
                coalesced: self.coalesced,
                unstamped,
            },
            contract,
            suppressed: self
                .suppressed
                .iter()
                .map(|(k, v)| ((*k).to_string(), *v))
                .collect(),
            unregistered_keys: self.unregistered.len() as u64,
            doctor: match inputs.doctor {
                None => Asked::NotAsked,
                Some(run) => Asked::Asked(DoctorSummary {
                    ran_at_unix_s: run.ran_at_unix_s,
                    findings: run
                        .report
                        .findings
                        .iter()
                        .map(|f| DoctorFindingRef {
                            check: f.check,
                            severity: f.severity,
                            subject: f.subject.clone(),
                        })
                        .collect(),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::facts::describe_key;
    use crate::model::prom::exposition;
    use crate::model::registry::SliceSet;
    use serde_json::json;
    use std::time::{Duration, Instant};

    const ORIGIN: &str = "h-3fa9c2d41b7e";

    fn slices() -> SliceSet {
        SliceSet::from_slices(vec![
            zenkey::parse_slice(
                r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "sysinfo"
[[subject]]
path = "cpu/usage"
class = "telemetry"
type = "TelemetryPoint"
unit = "percent"
kind = "gauge"
[[subject]]
path = "disk/{mount}/used"
class = "telemetry"
type = "TelemetryPoint"
unit = "bytes"
cardinality = 2
[[subject]]
path = "health"
class = "state"
type = "HealthSnapshot"
ttl_s = 30
[[subject]]
path = "hostname"
class = "state"
type = "Text"
kind = "text"
"#,
            )
            .expect("fixture parses"),
        ])
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn key(rest: &str) -> String {
        format!("v1/{ORIGIN}/{rest}")
    }

    struct Rig {
        ledger: ExportLedger,
        slices: SliceSet,
        stats: StatsTable,
        down: BTreeSet<(String, String)>,
        dropped: u64,
        clock: Instant,
    }

    impl Rig {
        fn new() -> Rig {
            Rig {
                ledger: ExportLedger::new(16, vec!["v1/*/**".into()], Some(1), at(1_000)),
                slices: slices(),
                stats: StatsTable::new(),
                down: BTreeSet::new(),
                dropped: 0,
                clock: Instant::now(),
            }
        }

        fn put(&mut self, rest: &str, doc: Value, wall: u64) {
            let key = key(rest);
            self.stats.record(&key, 8, None, self.clock, None, None);
            let facts = describe_key("", &key, Some(&self.slices)).facts;
            self.ledger.ingest(
                &Observed {
                    key: &key,
                    delete: false,
                    doc: Some(&doc),
                    qos_matches: None,
                    verdict: PayloadVerdict::NotValidated,
                    wall_unix_s: wall,
                },
                &facts,
            );
        }

        fn delete(&mut self, rest: &str, wall: u64) {
            let key = key(rest);
            let facts = describe_key("", &key, Some(&self.slices)).facts;
            self.ledger.ingest(
                &Observed {
                    key: &key,
                    delete: true,
                    doc: None,
                    qos_matches: None,
                    verdict: PayloadVerdict::NotValidated,
                    wall_unix_s: wall,
                },
                &facts,
            );
        }

        fn fold(&mut self, now: u64) -> ExportSnapshot {
            self.ledger.fold(&FoldInputs {
                stats: &self.stats,
                retention: RetentionStats {
                    budget: Default::default(),
                    retained: 0,
                    retained_bytes: 0,
                    span: Duration::ZERO,
                    evicted: 0,
                    expired: 0,
                },
                dropped: self.dropped,
                down: &self.down,
                doctor: None,
                now: at(now),
            })
        }
    }

    fn row<'a>(s: &'a ExportSnapshot, name: &str) -> &'a SeriesRow {
        s.series
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("no series {name} in {:?}", s.series))
    }

    /// The acceptance clause: killing a producer makes its series go stale
    /// **explicitly** — the state names it and the value line is gone, the
    /// labels are not.
    #[test]
    fn a_producer_that_went_away_flips_its_series_to_origin_down_and_drops_the_value() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/cpu/usage", json!(12.5), 1_010);
        let s = rig.fold(1_011);
        let r = row(&s, "zenkey_subject_sysinfo_cpu_usage_percent");
        assert_eq!(r.state, SeriesState::Live);
        assert_eq!(r.value, Some(12.5));

        rig.down.insert((ORIGIN.to_string(), "sysinfo".to_string()));
        let s = rig.fold(1_012);
        let r = row(&s, "zenkey_subject_sysinfo_cpu_usage_percent");
        assert_eq!(r.state, SeriesState::OriginDown);
        assert_eq!(r.value, None, "a stopped series exposes no value");
        assert_eq!(r.origin, ORIGIN, "and keeps its labels");
        let text = exposition(&s);
        assert!(
            text.contains("zenkey_series_state{origin=\"h-3fa9c2d41b7e\",producer=\"sysinfo\",class=\"telemetry\",subject=\"cpu/usage\",state=\"origin_down\"} 1"),
            "{text}"
        );
        assert!(
            !text.contains("zenkey_subject_sysinfo_cpu_usage_percent{"),
            "no value line for a series whose origin is down:\n{text}"
        );
    }

    /// The second acceptance clause: forcing observer drops moves the
    /// counter and taints exactly the series fed in that interval.
    #[test]
    fn drops_move_the_counter_and_taint_only_the_series_fed_in_the_interval() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        rig.put("telemetry/sysinfo/disk/root/used", json!(100), 1_010);
        rig.fold(1_011);

        // Only cpu is fed while the observer drops.
        rig.put("telemetry/sysinfo/cpu/usage", json!(2), 1_012);
        rig.dropped = 7;
        let s = rig.fold(1_013);
        assert_eq!(s.observer.dropped, 7);
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_cpu_usage_percent").drop_exposed,
            1
        );
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_disk_used_bytes").drop_exposed,
            0,
            "a series not fed in the interval is not tainted by it"
        );
        let text = exposition(&s);
        assert!(text.contains("zenkey_observer_dropped_total 7\n"), "{text}");
    }

    /// The third acceptance clause: two scrapes with no traffic are
    /// byte-identical.
    #[test]
    fn two_folds_with_no_ingest_between_them_expose_identical_bytes() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        rig.put(
            "state/sysinfo/health",
            json!({"type": "gauge", "value": 1}),
            1_010,
        );
        let a = exposition(&rig.fold(1_011));
        let b = exposition(&rig.fold(1_020));
        assert_eq!(a, b);
        assert!(a.contains("zenkey_subject_sysinfo_health{"), "{a}");
    }

    /// Coalescing is counted, and only the newest value is exposed.
    #[test]
    fn samples_between_two_folds_coalesce_into_the_newest_and_are_counted() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        rig.put("telemetry/sysinfo/cpu/usage", json!(2), 1_011);
        rig.put("telemetry/sysinfo/cpu/usage", json!(3), 1_012);
        let s = rig.fold(1_013);
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_cpu_usage_percent").value,
            Some(3.0)
        );
        assert_eq!(s.observer.coalesced, 2);
    }

    /// A population past its declared `cardinality` is suppressed and
    /// counted — never a silent missing series.
    #[test]
    fn a_cardinality_overflow_is_suppressed_and_counted() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/disk/root/used", json!(1), 1_010);
        rig.put("telemetry/sysinfo/disk/var/used", json!(2), 1_010);
        rig.put("telemetry/sysinfo/disk/tmp/used", json!(3), 1_010);
        rig.put("telemetry/sysinfo/disk/tmp/used", json!(4), 1_011);
        let s = rig.fold(1_012);
        assert_eq!(
            s.series
                .iter()
                .filter(|r| r.subject == "disk/{mount}/used")
                .count(),
            2
        );
        assert_eq!(s.suppressed.get("cardinality"), Some(&2));
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_disk_used_bytes").labels["mount"],
            "root"
        );
    }

    /// The four O6 populations serialize distinctly and are never summed —
    /// asserted with all of them non-zero, because a renderer that adds
    /// them up passes any fixture where three are zero.
    #[test]
    fn the_evicted_populations_are_four_lines_and_never_one() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        let mut s = rig.fold(1_011);
        s.observer.evicted_keys = 3;
        s.observer.evicted_bytes = 5;
        s.observer.expired = 7;
        s.observer.unwatched = 11;
        s.observer.dropped = 13;
        s.observer.coalesced = 17;
        let text = exposition(&s);
        for line in [
            "zenkey_observer_evicted_total{population=\"keys\"} 3",
            "zenkey_observer_evicted_total{population=\"retained_bytes\"} 5",
            "zenkey_observer_evicted_total{population=\"retained_age\"} 7",
            "zenkey_observer_evicted_total{population=\"unwatched\"} 11",
            "zenkey_observer_dropped_total 13",
            "zenkey_observer_coalesced_total 17",
        ] {
            assert!(text.contains(line), "missing `{line}` in:\n{text}");
        }
        assert!(
            !text.contains(" 26") && !text.contains(" 56"),
            "no sum of the kinds:\n{text}"
        );
    }

    /// The three payload populations are always three lines, and without
    /// validation everything is the third.
    #[test]
    fn payload_verdicts_are_three_populations_never_a_ratio() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        let text = exposition(&rig.fold(1_011));
        assert!(text.contains("zenkey_payload_verdict_total{verdict=\"valid\"} 0"));
        assert!(text.contains("zenkey_payload_verdict_total{verdict=\"invalid\"} 0"));
        assert!(text.contains("zenkey_payload_verdict_total{verdict=\"not_validated\"} 1"));
    }

    /// A key the registry does not declare is counted, never exported; a
    /// declared `text` kind and a non-numeric payload are counted by reason.
    #[test]
    fn what_is_not_exported_is_counted_by_reason() {
        let mut rig = Rig::new();
        rig.put("telemetry/sysinfo/nope", json!(1), 1_010);
        rig.put("telemetry/other/cpu/usage", json!(1), 1_010);
        rig.put("state/sysinfo/hostname", json!("box"), 1_010);
        rig.put("telemetry/sysinfo/cpu/usage", json!("high"), 1_010);
        let s = rig.fold(1_011);
        assert_eq!(s.unregistered_keys, 2);
        assert_eq!(s.suppressed.get("text"), Some(&1));
        assert_eq!(s.suppressed.get("non_numeric"), Some(&1));
        assert!(s.series.is_empty());
        let text = exposition(&s);
        assert!(text.contains("zenkey_unregistered_keys 2"));
        assert!(text.contains("zenkey_series_suppressed_total{reason=\"text\"} 1"));
    }

    /// An object payload fans out into one series per top-level numeric
    /// field, and a tombstone retires them all.
    #[test]
    fn an_object_fans_out_by_field_and_a_tombstone_retires_the_key() {
        let mut rig = Rig::new();
        rig.put(
            "state/sysinfo/health",
            json!({"uptime_s": 42, "ok": true, "note": "fine"}),
            1_010,
        );
        let s = rig.fold(1_011);
        let fields: Vec<_> = s.series.iter().filter_map(|r| r.field.clone()).collect();
        assert_eq!(fields, ["ok", "uptime_s"]);
        let by_field = |f: &str| {
            s.series
                .iter()
                .find(|r| r.field.as_deref() == Some(f))
                .and_then(|r| r.value)
        };
        assert_eq!(by_field("ok"), Some(1.0), "a bool reads as 0/1");
        assert_eq!(by_field("uptime_s"), Some(42.0));
        assert!(
            s.series
                .iter()
                .all(|r| r.name == "zenkey_subject_sysinfo_health"),
            "fields share the subject's name and differ by label"
        );

        rig.delete("state/sysinfo/health", 1_012);
        let s = rig.fold(1_013);
        assert!(
            s.series
                .iter()
                .all(|r| r.state == SeriesState::Retired && r.value.is_none()),
            "{:?}",
            s.series
        );
    }

    /// A `state` subject past its declared `ttl_s` is `quiet`; telemetry
    /// declares no period and is never judged (O4).
    #[test]
    fn quiet_is_judged_only_against_a_declared_ttl() {
        let mut rig = Rig::new();
        rig.put("state/sysinfo/health", json!({"value": 1}), 1_010);
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        let s = rig.fold(1_100);
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_health").state,
            SeriesState::Quiet
        );
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_health").value,
            Some(1.0),
            "quiet still exposes its last value — it is silence, not a stop"
        );
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_cpu_usage_percent").state,
            SeriesState::Live
        );
    }

    /// A key the statistics table forgot is `evicted`, by name.
    #[test]
    fn a_key_the_observer_forgot_is_named_evicted() {
        let mut rig = Rig::new();
        rig.stats = StatsTable::with_capacity(1);
        rig.put("telemetry/sysinfo/cpu/usage", json!(1), 1_010);
        rig.put("telemetry/sysinfo/disk/root/used", json!(1), 1_010);
        let s = rig.fold(1_011);
        assert_eq!(
            row(&s, "zenkey_subject_sysinfo_cpu_usage_percent").state,
            SeriesState::Evicted
        );
        assert_eq!(s.observer.evicted_keys, 1);
    }

    #[test]
    fn a_wildcard_scope_names_what_it_cannot_reach() {
        assert_eq!(
            excluded_by(&["v1/*/**".to_string()]),
            WILDCARD_EXCLUDES.map(String::from)
        );
        assert!(
            excluded_by(&["v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage".to_string()]).is_empty()
        );
    }
}
