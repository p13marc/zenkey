//! Self-publication (#389): the daemon as a producer, so `zengui` and
//! `zenctl` see the notifier without SSH and something else can watch the
//! watcher.
//!
//! The same bring-up every producer does (RFC 04 §5, through the engine's
//! [`BringUp`]): the two `@rpc` queryables first — `introspect` answers
//! the generated registry slice verbatim, `describe` the served
//! [`SchemaSet`] over the three registered types — and the `alive` token
//! last, so "alive ⇒ callable" holds from the first moment the roster
//! lists this process. Then, as state:
//!
//! - `state/zenwatch/health` — [`ZenwatchHealth`], every
//!   [`HEALTH_PERIOD`] (the registry's `ttl_s = 60`, refreshed at half of
//!   it) and carrying every counter the discipline keeps, because a
//!   bounded observer reports what its bounds cost (RFC 13 §3 O6);
//! - `state/zenwatch/firing/{rule_id}` — one [`FiringRule`] per rule with
//!   something announced, the most severe (then oldest) notice under it
//!   named and the rest counted; tombstoned when the rule goes quiet;
//!   never more than the registry's cardinality, the refusals counted;
//! - `state/zenwatch/doctor` — [`ZenwatchDoctor`], after every scheduled
//!   doctor run (#390, [`crate::doctor`]): the outcome, the counts, and the
//!   report exactly as `zenctl doctor --format json` prints it. The
//!   registry's `ttl_s = 0` — retained, last writer wins — and it is not
//!   tombstoned at shutdown: the last report stays the last report, and a
//!   `next_at` in the past is a schedule that lapsed.
//!
//! Keys are base-relative from the [`V1Context`] and wired with
//! [`Fleet::wire`]; the profile's origin salt is a compile-time constant,
//! as every producer's is (`zenkey/src/profile.rs`).

use std::collections::BTreeMap;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use zenkey::qos::QosProfile;
use zenkey::schema::SchemaSet;
use zenkey::{AppName, AppProfile, OriginSalt, V1Context};
use zenkey_fleet::{BringUp, Fleet, Publication, declare_publication};

use crate::discipline::{Discipline, severity_rank};
use crate::render::state_word;

/// The application profile: the name (the host-id fallback path) and the
/// RFC 06 §1 origin salt. Changing the salt re-keys the notifier's origin.
pub static PROFILE: AppProfile = AppProfile::new(
    AppName::new("zenwatch"),
    OriginSalt::new("zenwatch-host-id-v1"),
);

/// The producer name — the registry's `[producer] name`.
pub const PRODUCER: &str = "zenwatch";

/// How often the health document is refreshed: half the registry's
/// `ttl_s = 60`, so a reader never sees it expire while the daemon is up.
pub const HEALTH_PERIOD: Duration = Duration::from_secs(30);

/// The registry's `cardinality` on `firing/{rule_id}`: the most
/// `firing/*` keys this producer will ever hold at once.
pub const FIRING_CARDINALITY: usize = 256;

/// The three registered types, in registry order — what `describe` must
/// cover (RFC 08 §6.1).
pub const TYPE_NAMES: [&str; 3] = ["ZenwatchHealth", "FiringRule", "ZenwatchDoctor"];

/// This process's producer context.
pub fn context() -> V1Context {
    V1Context::for_producer(&PROFILE, PRODUCER).expect("a literal producer name is a valid chunk")
}

/// `ok` or `degraded`: degraded when the last persist failed or a delivery
/// failed since the last health document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
}

/// Where the scheduled doctor stands, on the health document (#390).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum DoctorStatus {
    /// No `doctor` block: nothing runs, nothing is published.
    #[serde(rename = "not scheduled")]
    NotScheduled,
    /// Scheduled; the first run has not happened yet.
    #[serde(rename = "pending")]
    Pending,
    /// The last run succeeded.
    #[serde(rename = "ok")]
    Ok,
    /// The last run could not happen; the report before it is retained.
    #[serde(rename = "failed")]
    Failed,
}

/// `state/zenwatch/health` (RFC 04 §1.2; the identity bridge rides it,
/// RFC 06 §6.2 — `host_id` is the origin).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ZenwatchHealth {
    pub status: HealthStatus,
    pub host_id: String,
    pub started_at: String,
    pub rules: usize,
    pub sinks: usize,
    /// Notice identities currently announced firing.
    pub firing: usize,
    /// Notice identities currently announced unobservable.
    pub unobservable: usize,
    pub notifications_sent: u64,
    pub deliveries_failed: u64,
    /// Symptoms held back by inhibition (RFC 06 §5.6).
    pub inhibited: u64,
    /// Events the observer's own broadcast dropped (O6).
    pub dropped_total: u64,
    pub state_entries: usize,
    pub state_evicted: u64,
    /// `firing/*` documents refused at the registry's cardinality.
    pub firing_refused: u64,
    pub last_persist_error: Option<String>,
    /// The scheduled doctor (#390): `not scheduled`, `pending`, `ok`,
    /// `failed`.
    pub doctor: DoctorStatus,
    /// RFC 3339: when the doctor last ran (or last tried to).
    pub doctor_last: Option<String>,
    /// RFC 3339: when it runs next — in the past, the schedule has lapsed.
    pub doctor_next: Option<String>,
}

/// `state/zenwatch/firing/{rule_id}`: what a rule currently has announced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FiringRule {
    pub rule: String,
    /// The notice identity named here: the most severe, then the oldest.
    pub id: String,
    /// `firing` or `unobservable`.
    pub state: String,
    pub severity: String,
    /// RFC 3339: when the named notice's condition began.
    pub since: String,
    pub labels: BTreeMap<String, String>,
    /// How many notice identities are announced under this rule, this
    /// one included.
    pub count: usize,
}

/// Whether a doctor run happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DoctorOutcome {
    Ok,
    Failed,
}

/// `state/zenwatch/doctor`: the scheduled doctor's last run (#390,
/// [`crate::doctor`]). Published after every run, successful or not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZenwatchDoctor {
    /// RFC 3339: when this run happened (or was attempted).
    pub ran_at: String,
    /// RFC 3339: when the next one is due. In the past: the schedule lapsed.
    pub next_at: String,
    /// The interval, seconds.
    pub every_s: f64,
    pub outcome: DoctorOutcome,
    /// Why a `failed` run could not happen.
    pub error: Option<String>,
    /// Findings in `report` — after a failed run, the retained report's.
    pub findings: usize,
    /// Findings new since the previous run (`0` on the baseline).
    pub new: usize,
    /// Findings gone since the previous run.
    pub fixed: usize,
    /// The `DoctorReport`, exactly as `zenctl doctor --format json` prints
    /// it — `synced` absent when the registry diff was not asked (RFC 13
    /// §1). After a failed run, the last report that succeeded, retained;
    /// `null` when none has yet.
    pub report: serde_json::Value,
    /// RFC 3339: the run that produced `report` — `ran_at` on a successful
    /// run, older after a failed one, absent when there is no report.
    pub report_at: Option<String>,
    /// The `DoctorDelta` against the previous run; absent on the baseline
    /// and after a failed run.
    pub delta: Option<serde_json::Value>,
}

/// The served schema set: every registered subject's type described
/// (RFC 08 §6.1 / §7), verified at build so a gap is a startup panic.
pub fn schema_set() -> SchemaSet {
    SchemaSet::builder(PRODUCER)
        .json::<ZenwatchHealth>("ZenwatchHealth")
        .json::<FiringRule>("FiringRule")
        .json::<ZenwatchDoctor>("ZenwatchDoctor")
        .build_verified(&TYPE_NAMES)
}

/// The `firing/{rule_id}` documents the discipline's ledger implies.
pub fn firing_docs(discipline: &Discipline) -> BTreeMap<String, FiringRule> {
    let mut out: BTreeMap<String, FiringRule> = BTreeMap::new();
    for e in discipline.announced() {
        let doc = out.entry(e.rule_id.clone()).or_insert_with(|| FiringRule {
            rule: e.rule.clone(),
            id: e.id.clone(),
            state: state_word(e.state).into(),
            severity: e.severity.clone(),
            since: zenkey_fleet::rfc3339_from_unix(e.since.max(0.0) as u64),
            labels: e.labels.clone(),
            count: 0,
        });
        doc.count += 1;
        let more_severe = severity_rank(&e.severity) > severity_rank(&doc.severity);
        let older_at_same = severity_rank(&e.severity) == severity_rank(&doc.severity)
            && zenkey_fleet::rfc3339_from_unix(e.since.max(0.0) as u64) < doc.since;
        if more_severe || older_at_same {
            doc.id = e.id.clone();
            doc.state = state_word(e.state).into();
            doc.severity = e.severity.clone();
            doc.since = zenkey_fleet::rfc3339_from_unix(e.since.max(0.0) as u64);
            doc.labels = e.labels.clone();
        }
    }
    out
}

/// The daemon, up as a producer: queryables served on their own task, the
/// token held, the state publications declared.
pub struct SelfProducer {
    session: zenoh::Session,
    base: String,
    ctx: V1Context,
    health: Publication,
    doctor: Publication,
    firing: BTreeMap<String, Publication>,
    published: BTreeMap<String, FiringRule>,
    refused: u64,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<zenkey_fleet::Result<()>>,
}

impl SelfProducer {
    /// Bring the producer up in RFC 04 §5 order: `introspect` and
    /// `describe` declared and served, then the token.
    pub async fn bring_up(fleet: &Fleet<'_>) -> zenkey_fleet::Result<SelfProducer> {
        let ctx = context();
        let session = fleet.session().clone();
        let base = fleet.base().to_string();
        let introspect = fleet.wire(ctx.rpc_key(&["introspect"])?.as_str());
        let describe = fleet.wire(ctx.rpc_key(&["describe"])?.as_str());
        let alive = fleet.wire(ctx.alive_key().as_str());
        let health_key = fleet.wire(ctx.health_key().as_str());
        let doctor_key = fleet.wire(ctx.state_key(&["doctor"])?.as_str());

        let mut up = BringUp::new(&session);
        up.serve(&introspect).await?;
        up.serve(&describe).await?;
        let live = up.alive(&alive).await?;

        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let introspect_body = crate::registry::zenwatch::REGISTRY_TOML.as_bytes().to_vec();
        let describe_body = schema_set().to_json().into_bytes();
        let task = tokio::spawn(async move {
            let (intro, desc) = (&live.responders[0], &live.responders[1]);
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    q = intro.next() => match q {
                        Some(q) => {
                            if let Err(e) = intro.reply(&q, introspect_body.clone(), Some("text/plain")).await {
                                tracing::warn!(key = %intro.key(), "introspect reply failed: {e}");
                            }
                        }
                        None => break,
                    },
                    q = desc.next() => match q {
                        Some(q) => {
                            if let Err(e) = desc.reply(&q, describe_body.clone(), Some("application/json")).await {
                                tracing::warn!(key = %desc.key(), "describe reply failed: {e}");
                            }
                        }
                        None => break,
                    },
                }
            }
            live.retire().await
        });

        let health = declare_publication(
            &session,
            &health_key,
            QosProfile::Refreshed,
            Some("application/json"),
        )
        .await?;
        // Written on rare transitions — one per scheduled run — that a
        // consumer cannot learn late: the transition profile (RFC 04 §3).
        let doctor = declare_publication(
            &session,
            &doctor_key,
            QosProfile::Transition,
            Some("application/json"),
        )
        .await?;
        tracing::info!(origin = %ctx.origin().chunk(), alive, "zenwatch is up as a producer");
        Ok(SelfProducer {
            session,
            base,
            ctx,
            health,
            doctor,
            firing: BTreeMap::new(),
            published: BTreeMap::new(),
            refused: 0,
            stop: Some(stop_tx),
            task,
        })
    }

    /// The origin chunk this producer publishes under.
    pub fn origin(&self) -> String {
        self.ctx.origin().chunk().to_string()
    }

    /// `firing/*` documents refused at the cardinality bound so far.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    pub async fn publish_health(&self, health: &ZenwatchHealth) -> zenkey_fleet::Result<()> {
        let body = serde_json::to_vec(health).expect("a health document serializes");
        self.health.send(body, None).await
    }

    /// The scheduled doctor's last run (#390) — after every run, whatever
    /// its outcome. Never tombstoned: the registry's `ttl_s = 0` is a
    /// retained document, and the last report stays the last report.
    pub async fn publish_doctor(&self, doc: &ZenwatchDoctor) -> zenkey_fleet::Result<()> {
        let body = serde_json::to_vec(doc).expect("a doctor document serializes");
        self.doctor.send(body, None).await
    }

    /// Bring the published `firing/*` set to `docs`: changed documents are
    /// put, vanished ones tombstoned, and a new key beyond the registry's
    /// cardinality is refused and counted rather than declared.
    pub async fn publish_firing(
        &mut self,
        docs: &BTreeMap<String, FiringRule>,
    ) -> zenkey_fleet::Result<()> {
        let gone: Vec<String> = self
            .published
            .keys()
            .filter(|id| !docs.contains_key(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(p) = self.firing.get(&id) {
                p.retire().await?;
            }
            self.published.remove(&id);
        }
        for (rule_id, doc) in docs {
            if self.published.get(rule_id) == Some(doc) {
                continue;
            }
            if !self.firing.contains_key(rule_id) {
                if self.firing.len() >= FIRING_CARDINALITY {
                    self.refused += 1;
                    tracing::warn!(
                        rule_id,
                        "firing/* at the registry's cardinality ({FIRING_CARDINALITY}); not published"
                    );
                    continue;
                }
                let key = zenkey::grammar::with_base(
                    &self.base,
                    self.ctx.state_key(&["firing", rule_id])?.as_str(),
                );
                let p = declare_publication(
                    &self.session,
                    &key,
                    QosProfile::Refreshed,
                    Some("application/json"),
                )
                .await?;
                self.firing.insert(rule_id.clone(), p);
            }
            let body = serde_json::to_vec(doc).expect("a firing document serializes");
            self.firing[rule_id].send(body, None).await?;
            self.published.insert(rule_id.clone(), doc.clone());
        }
        Ok(())
    }

    /// Retire in reverse: every `firing/*` document and the health
    /// tombstoned (the doctor report deliberately not — it is retained
    /// state, not a fact about a live process), then the token retracted
    /// and the queryables undeclared (the engine's
    /// [`LiveProducer::retire`](zenkey_fleet::bus::producer::LiveProducer::retire) order).
    pub async fn shutdown(mut self) -> zenkey_fleet::Result<()> {
        for id in self.published.keys() {
            if let Some(p) = self.firing.get(id) {
                p.retire().await?;
            }
        }
        self.published.clear();
        self.health.retire().await?;
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match self.task.await {
            Ok(r) => r,
            Err(e) => Err(zenkey_fleet::Error::bus(
                "retire self producer",
                "",
                std::io::Error::other(e.to_string()),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The served describe set names every registered type (RFC 08 §6.1),
    /// and the health document's JSON is what the schema says.
    #[test]
    fn the_schema_set_covers_the_registry_and_health_serializes() {
        let set = schema_set();
        for name in TYPE_NAMES {
            assert!(set.get(name).is_some(), "{name} described");
        }
        let parsed = SchemaSet::parse(&set.to_json()).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(crate::registry::zenwatch::REGISTRY_TOML.contains("firing/{rule_id}"));
        let h = ZenwatchHealth {
            status: HealthStatus::Degraded,
            host_id: "h-3fa9c2d41b7e".into(),
            started_at: "2026-09-06T00:00:00Z".into(),
            rules: 4,
            sinks: 2,
            firing: 1,
            unobservable: 0,
            notifications_sent: 7,
            deliveries_failed: 1,
            inhibited: 2,
            dropped_total: 0,
            state_entries: 1,
            state_evicted: 0,
            firing_refused: 0,
            last_persist_error: None,
            doctor: DoctorStatus::NotScheduled,
            doctor_last: None,
            doctor_next: None,
        };
        let j = serde_json::to_value(&h).unwrap();
        assert_eq!(j["status"], "degraded");
        assert_eq!(j["host_id"], "h-3fa9c2d41b7e");
        assert_eq!(j["last_persist_error"], serde_json::Value::Null);
        assert_eq!(j["doctor"], "not scheduled");
        assert_eq!(j["doctor_last"], serde_json::Value::Null);
        for (s, word) in [
            (DoctorStatus::Pending, "pending"),
            (DoctorStatus::Ok, "ok"),
            (DoctorStatus::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), word);
        }
    }

    /// The doctor document's wire shape (#390): the report rides verbatim,
    /// `not asked` as absence inside it, and the whole thing round-trips
    /// through the served schema's type — what an explorer parses.
    #[test]
    fn the_doctor_document_shape_is_pinned() {
        let d = ZenwatchDoctor {
            ran_at: "2026-09-06T01:00:00Z".into(),
            next_at: "2026-09-06T07:00:00Z".into(),
            every_s: 21600.0,
            outcome: DoctorOutcome::Failed,
            error: Some("timed out".into()),
            findings: 1,
            new: 0,
            fixed: 0,
            report: serde_json::json!({"findings": [{"severity": "error", "check": "slice-sync",
                "subject": "h-1/sysinfo", "evidence": "e", "citation": "RFC 08 §6"}],
                "introspect_answered": 1, "live_producers": 1, "describe_served": 0,
                "describe_missing": 1, "routers": 0, "deep": false}),
            report_at: Some("2026-09-05T19:00:00Z".into()),
            delta: None,
        };
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(
            j,
            serde_json::json!({
                "ran_at": "2026-09-06T01:00:00Z",
                "next_at": "2026-09-06T07:00:00Z",
                "every_s": 21600.0,
                "outcome": "failed",
                "error": "timed out",
                "findings": 1,
                "new": 0,
                "fixed": 0,
                "report": d.report,
                "report_at": "2026-09-05T19:00:00Z",
                "delta": null,
            })
        );
        assert!(!j["report"].as_object().unwrap().contains_key("synced"));
        let back: ZenwatchDoctor = serde_json::from_value(j).unwrap();
        assert_eq!(back, d);
        // The served describe names the document's fields — the shape an
        // explorer decodes through is the shape that is published.
        let served = schema_set().to_json();
        for field in ["ran_at", "next_at", "outcome", "report", "report_at", "delta"] {
            assert!(served.contains(&format!("\"{field}\"")), "{field} described");
        }
    }

    /// The profile's constants are checked where they are spelled; the
    /// context builds the keys the registry declares.
    #[test]
    fn the_context_spells_the_registered_keys() {
        let ctx = context();
        let origin = ctx.origin().chunk().to_string();
        assert!(zenkey::grammar::is_valid_host_origin(&origin), "{origin}");
        assert_eq!(
            ctx.health_key().as_str(),
            format!("v1/{origin}/state/zenwatch/health")
        );
        assert_eq!(
            ctx.state_key(&["firing", "fleet-alerts"]).unwrap().as_str(),
            format!("v1/{origin}/state/zenwatch/firing/fleet-alerts")
        );
        assert_eq!(
            ctx.rpc_key(&["describe"]).unwrap().as_str(),
            format!("v1/{origin}/@rpc/zenwatch/describe")
        );
    }
}
