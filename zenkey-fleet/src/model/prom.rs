//! The Prometheus text exposition of an [`ExportSnapshot`] (#228, RFC 13
//! §3 *Exporter obligations*).
//!
//! A pure function of the snapshot, so the text a scraper reads and the
//! `--format json` document can never disagree — and deterministic in every
//! byte, so two scrapes with no traffic between them are identical (the
//! snapshot's `taken_at` is deliberately not here).
//!
//! **Names and units come from the registry, never from the leaf.** A
//! series is `zenkey_subject_<producer>_<literal chunks>` with the declared
//! `unit` normalised into the conventional suffix and `_total` appended for a
//! declared `counter`; a `{var}` chunk becomes a label named by its declared
//! name. The one thing this module does *not* do is guess: a subject with no
//! `unit` gets no suffix and a subject with no `kind` is `untyped`.
//!
//! What the exposition refuses is recorded in `zenctl export --help`, not
//! here — histograms, summaries, remote write and push are tool decisions
//! this chapter records rather than makes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::report::{ExportSnapshot, SeriesRow, SeriesState};

/// The metric name for a registry subject: `zenkey_subject_<producer>_<literal
/// chunks>[_<unit>][_total]`.
///
/// `unit` and `kind` are the registry's tokens (`kind` is `counter`,
/// `gauge`, `bool`, `text` or a token this build does not know). A suffix
/// already spelled by the leaf's last chunk (`rx_bytes` with `unit =
/// "bytes"`, `messages_total` with `kind = "counter"`) is not doubled — that
/// is a comparison against the declaration, not a sniff of the leaf.
pub fn metric_name(
    producer: &str,
    pattern: &str,
    unit: Option<&str>,
    kind: Option<&str>,
) -> String {
    let mut name = String::from("zenkey_subject_");
    name.push_str(&sanitize(producer));
    for chunk in pattern.split('/') {
        if chunk.starts_with('{') {
            continue;
        }
        name.push('_');
        name.push_str(&sanitize(chunk));
    }
    if let Some(unit) = unit {
        let suffix = unit_suffix(unit);
        if !suffix.is_empty() && !name.ends_with(&format!("_{suffix}")) {
            name.push('_');
            name.push_str(&suffix);
        }
    }
    if kind == Some("counter") && !name.ends_with("_total") {
        name.push_str("_total");
    }
    name
}

/// The registry `unit` as a Prometheus suffix: the base-unit spellings the
/// convention uses, anything else verbatim (sanitised).
pub fn unit_suffix(unit: &str) -> String {
    match unit {
        "ms" => "milliseconds".into(),
        "us" => "microseconds".into(),
        "ns" => "nanoseconds".into(),
        "s" => "seconds".into(),
        "bytes" | "B" => "bytes".into(),
        "percent" | "%" => "percent".into(),
        "ratio" => "ratio".into(),
        other => sanitize(other),
    }
}

/// The `# TYPE` a declared kind earns: a counter is a counter, a gauge or
/// a bool is a gauge, anything else — including no declaration — is
/// `untyped`, which is Prometheus's spelling of *not asked*.
pub fn prom_type(kind: Option<&str>) -> &'static str {
    match kind {
        Some("counter") => "counter",
        Some("gauge" | "bool") => "gauge",
        _ => "untyped",
    }
}

/// A metric or label name: `[a-zA-Z0-9_]`, everything else `_`.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A label value, escaped as the text format requires.
fn escape_label(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// A `# HELP` line's text, escaped.
fn escape_help(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

/// A sample value in the text format's spelling: `+Inf`, `-Inf`, `NaN`,
/// and no exponent for anything finite.
fn number(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v == f64::INFINITY {
        "+Inf".into()
    } else if v == f64::NEG_INFINITY {
        "-Inf".into()
    } else {
        format!("{v}")
    }
}

/// The fixed label names every subject series carries; a `{var}` whose
/// declared name collides with one is exposed as `var_<name>`.
const FIXED_LABELS: [&str; 6] = ["origin", "producer", "class", "subject", "field", "state"];

fn labels_of(row: &SeriesRow) -> Vec<(String, String)> {
    let mut labels = vec![
        ("origin".to_string(), row.origin.clone()),
        ("producer".to_string(), row.producer.clone()),
        ("class".to_string(), row.class.clone()),
        ("subject".to_string(), row.subject.clone()),
    ];
    for (name, value) in &row.labels {
        let mut name = sanitize(name);
        if FIXED_LABELS.contains(&name.as_str()) {
            name = format!("var_{name}");
        }
        labels.push((name, value.clone()));
    }
    if let Some(field) = &row.field {
        labels.push(("field".to_string(), field.clone()));
    }
    labels
}

fn label_set(labels: &[(String, String)]) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{k}=\"{}\"", escape_label(v));
    }
    out.push('}');
    out
}

/// One family: `# HELP`, `# TYPE`, then its samples, in the order given.
struct Family {
    name: String,
    help: String,
    kind: &'static str,
    lines: Vec<String>,
}

impl Family {
    fn new(name: impl Into<String>, help: impl Into<String>, kind: &'static str) -> Family {
        Family {
            name: name.into(),
            help: help.into(),
            kind,
            lines: Vec::new(),
        }
    }

    fn sample(&mut self, labels: &[(String, String)], value: impl Into<f64>) {
        let value: f64 = value.into();
        if labels.is_empty() {
            self.lines.push(format!("{} {}", self.name, number(value)));
        } else {
            self.lines.push(format!(
                "{}{} {}",
                self.name,
                label_set(labels),
                number(value)
            ));
        }
    }

    fn write(&self, out: &mut String) {
        let _ = writeln!(out, "# HELP {} {}", self.name, escape_help(&self.help));
        let _ = writeln!(out, "# TYPE {} {}", self.name, self.kind);
        for line in &self.lines {
            out.push_str(line);
            out.push('\n');
        }
    }
}

fn l(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// The whole exposition, `text/plain; version=0.0.4`.
pub fn exposition(s: &ExportSnapshot) -> String {
    let mut fams: Vec<Family> = Vec::new();

    // ── the observer's own bounds (O6) — four populations, never summed ──
    let mut f = Family::new(
        "zenkey_observer_dropped_total",
        "samples this observer missed while behind; while this moves every value below is a lower bound (RFC 13 §3 O6)",
        "counter",
    );
    f.sample(&[], s.observer.dropped as f64);
    fams.push(f);

    let mut f = Family::new(
        "zenkey_observer_evicted_total",
        "what the observer chose to forget at its bounds, by population — keys at the stats-table bound, retained samples at the byte budget, retained samples aged out, keys unwatched (never sum these; RFC 13 §3 O6)",
        "counter",
    );
    f.sample(
        &l(&[("population", "keys")]),
        s.observer.evicted_keys as f64,
    );
    f.sample(
        &l(&[("population", "retained_bytes")]),
        s.observer.evicted_bytes as f64,
    );
    f.sample(
        &l(&[("population", "retained_age")]),
        s.observer.expired as f64,
    );
    f.sample(
        &l(&[("population", "unwatched")]),
        s.observer.unwatched as f64,
    );
    fams.push(f);

    let mut f = Family::new(
        "zenkey_observer_coalesced_total",
        "samples folded into a newer one between two scrapes — only the newest value per series is exposed (RFC 13 §3 O6)",
        "counter",
    );
    f.sample(&[], s.observer.coalesced as f64);
    fams.push(f);

    let mut f = Family::new(
        "zenkey_observer_unstamped_total",
        "samples that carried no HLC timestamp — counted, never defaulted to arrival",
        "counter",
    );
    f.sample(&[], s.observer.unstamped as f64);
    fams.push(f);

    // ── the contract: declared versus observed ──
    let mut f = Family::new(
        "zenkey_qos_judged_total",
        "samples whose subject declares a QoS profile this build knows",
        "counter",
    );
    f.sample(&[], s.contract.qos_judged as f64);
    fams.push(f);

    let mut f = Family::new(
        "zenkey_qos_mismatch_total",
        "samples that did not ride their declared QoS profile, by declared subject (RFC 04 §3)",
        "counter",
    );
    for row in &s.contract.qos_mismatch_by_subject {
        f.sample(
            &l(&[("producer", &row.producer), ("subject", &row.subject)]),
            row.n as f64,
        );
    }
    fams.push(f);

    let mut f = Family::new(
        "zenkey_payload_verdict_total",
        "payload verdicts as three counted populations — valid, invalid, not_validated — never a ratio (RFC 13 §3); without --validate everything is not_validated",
        "counter",
    );
    f.sample(&l(&[("verdict", "valid")]), s.contract.payload_valid as f64);
    f.sample(
        &l(&[("verdict", "invalid")]),
        s.contract.payload_invalid as f64,
    );
    f.sample(
        &l(&[("verdict", "not_validated")]),
        s.contract.payload_not_validated as f64,
    );
    fams.push(f);

    // ── the doctor: not asked is a state of its own ──
    let mut info = Family::new(
        "zenkey_doctor_info",
        "whether the doctor was asked to run (--doctor-every): not_asked or ran",
        "gauge",
    );
    let mut findings = Family::new(
        "zenkey_doctor_finding",
        "one series per finding of the last doctor run, by check id and severity",
        "gauge",
    );
    let mut last_run = None;
    match s.doctor.as_option() {
        None => {
            info.sample(&l(&[("state", "not_asked")]), 1.0);
        }
        Some(d) => {
            info.sample(&l(&[("state", "ran")]), 1.0);
            let mut last = Family::new(
                "zenkey_doctor_last_run_timestamp_seconds",
                "when the last doctor run finished, unix seconds",
                "gauge",
            );
            last.sample(&[], d.ran_at_unix_s as f64);
            last_run = Some(last);
            for f in &d.findings {
                let sev = serde_json::to_value(f.severity)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                findings.sample(
                    &l(&[
                        ("check_id", f.check.as_str()),
                        ("severity", &sev),
                        ("subject", &f.subject),
                    ]),
                    1.0,
                );
            }
        }
    }
    fams.push(info);
    fams.extend(last_run);
    fams.push(findings);

    // ── scope and provenance (O5) ──
    let mut f = Family::new(
        "zenkey_scope_info",
        "one series per selector watched; `excluded` names the planes a wildcard cannot reach (RFC 13 §3 O5, RFC 03 §4 D2)",
        "gauge",
    );
    let excluded = s.excluded.join(",");
    for scope in &s.scopes {
        f.sample(&l(&[("selector", scope), ("excluded", &excluded)]), 1.0);
    }
    fams.push(f);

    let mut f = Family::new(
        "zenkey_observer_started_timestamp_seconds",
        "when this observer started watching, unix seconds — a claim about anything earlier is unobservable",
        "gauge",
    );
    f.sample(&[], s.started_at_unix_s as f64);
    fams.push(f);

    let mut f = Family::new(
        "zenkey_registry_info",
        "the contract every subject series derives from: loaded (with the producer count) or not_loaded, in which case no key refines and every key is unregistered",
        "gauge",
    );
    match s.registry.as_option() {
        Some(r) => f.sample(
            &l(&[("state", "loaded"), ("producers", &r.producers.to_string())]),
            1.0,
        ),
        None => f.sample(&l(&[("state", "not_loaded"), ("producers", "")]), 1.0),
    }
    fams.push(f);

    let mut f = Family::new(
        "zenkey_series_suppressed_total",
        "samples that produced no series, by reason: cardinality (over the declared budget), max_series, fields (per-subject field cap), text, non_numeric, undecodable, unparsed",
        "counter",
    );
    for reason in SUPPRESSION_REASONS {
        let n = s.suppressed.get(reason).copied().unwrap_or(0);
        f.sample(&l(&[("reason", reason)]), n as f64);
    }
    for (reason, n) in &s.suppressed {
        if !SUPPRESSION_REASONS.contains(&reason.as_str()) {
            f.sample(&l(&[("reason", reason)]), *n as f64);
        }
    }
    fams.push(f);

    let mut f = Family::new(
        "zenkey_unregistered_keys",
        "distinct keys the registry does not declare — counted, never exported (RFC 13 §3 O4)",
        "gauge",
    );
    f.sample(&[], s.unregistered_keys as f64);
    fams.push(f);

    let mut f = Family::new(
        "zenkey_series_count",
        "series the ledger holds, against its --max-series bound",
        "gauge",
    );
    f.sample(&[], s.series.len() as f64);
    fams.push(f);

    // ── the subject series, grouped by metric name ──
    let mut subjects: BTreeMap<&str, Family> = BTreeMap::new();
    let mut last_seen = Family::new(
        "zenkey_key_last_seen_timestamp_seconds",
        "when the newest sample of a series arrived, unix seconds on the observer's clock — constant between scrapes",
        "gauge",
    );
    let mut state = Family::new(
        "zenkey_series_state",
        "why a series is or is not current: live, quiet (a state subject past its declared ttl_s), evicted (forgotten at the observer's bound), origin_down (the producer's alive token went), retired (a tombstone) — a stopped series keeps this line and loses its value (RFC 13 §3)",
        "gauge",
    );
    let mut exposed = Family::new(
        "zenkey_series_drop_exposed_total",
        "scrape intervals in which this series was fed while zenkey_observer_dropped_total moved — its value may not have been the newest",
        "counter",
    );
    for row in &s.series {
        let labels = labels_of(row);
        if row.state.exposes_value()
            && let Some(v) = row.value
        {
            let fam = subjects.entry(row.name.as_str()).or_insert_with(|| {
                Family::new(
                    row.name.clone(),
                    format!(
                        "registry subject `{}` `{}`{}{} — name and unit from the declaration, not the leaf; may lag while zenkey_observer_dropped_total moves",
                        row.producer,
                        row.subject,
                        row.unit
                            .as_deref()
                            .map(|u| format!(", unit {u}"))
                            .unwrap_or_default(),
                        row.kind
                            .as_deref()
                            .map(|k| format!(", kind {k}"))
                            .unwrap_or_else(|| ", kind undeclared".into()),
                    ),
                    prom_type(row.kind.as_deref()),
                )
            });
            fam.sample(&labels, v);
        }
        last_seen.sample(&labels, row.last_seen_unix_s as f64);
        let mut with_state = labels.clone();
        with_state.push(("state".to_string(), row.state.as_str().to_string()));
        state.sample(&with_state, 1.0);
        exposed.sample(&labels, row.drop_exposed as f64);
    }
    for fam in subjects.into_values() {
        fams.push(fam);
    }
    fams.push(last_seen);
    fams.push(state);
    fams.push(exposed);

    let mut out = String::new();
    for fam in &fams {
        fam.write(&mut out);
    }
    out
}

/// The suppression reasons the ledger spells, so every one is exposed at
/// zero and a reason a scraper alerts on cannot be absent.
pub const SUPPRESSION_REASONS: [&str; 7] = [
    "cardinality",
    "max_series",
    "fields",
    "text",
    "non_numeric",
    "undecodable",
    "unparsed",
];

/// The states, for a consumer that wants the vocabulary without a snapshot.
pub const SERIES_STATES: [SeriesState; 5] = [
    SeriesState::Live,
    SeriesState::Quiet,
    SeriesState::Evicted,
    SeriesState::OriginDown,
    SeriesState::Retired,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_declaration_and_are_never_doubled() {
        assert_eq!(
            metric_name("sysinfo", "cpu/usage", Some("percent"), Some("gauge")),
            "zenkey_subject_sysinfo_cpu_usage_percent"
        );
        assert_eq!(
            metric_name(
                "netlink",
                "iface/{iface}/rx_bytes",
                Some("bytes"),
                Some("counter")
            ),
            "zenkey_subject_netlink_iface_rx_bytes_total"
        );
        assert_eq!(
            metric_name(
                "logs",
                "by_unit/{unit}/messages_total",
                None,
                Some("counter")
            ),
            "zenkey_subject_logs_by_unit_messages_total"
        );
        assert_eq!(
            metric_name("tc-gui", "latency/p95", Some("ms"), None),
            "zenkey_subject_tc_gui_latency_p95_milliseconds"
        );
        assert_eq!(prom_type(None), "untyped");
        assert_eq!(prom_type(Some("bool")), "gauge");
        assert_eq!(prom_type(Some("histogram")), "untyped");
    }

    #[test]
    fn label_values_are_escaped_and_numbers_spell_the_specials() {
        assert_eq!(escape_label("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(number(f64::INFINITY), "+Inf");
        assert_eq!(number(f64::NEG_INFINITY), "-Inf");
        assert_eq!(number(f64::NAN), "NaN");
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(3.0), "3");
    }
}
