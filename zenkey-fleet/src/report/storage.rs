//! The storage plan (RFC 09 §2, #393): what a deployment file asks for, what
//! a router's storage manager makes of it, and how a live router compares.
//!
//! Three documents cross the wire here. [`Deployment`] comes *in* — the small
//! TOML an operator writes naming volumes and the selector each storage
//! takes — and it is here rather than beside the planner because a `Deserialize`
//! shape is somebody else's file format, which is the placement rule's whole
//! test. [`StoragePlan`] goes *out* as the plan, [`StorageCheck`] as the
//! verdict of `--check`, and [`StorageExplain`] as `--explain`'s answer.
//!
//! The planner itself is [`crate::model::storage`]; nothing here computes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::judgement::Judgement;

// ── The deployment file ───────────────────────────────────────────────────

/// The deployment file `zenctl storage gen --deployment` reads (#393).
///
/// Deliberately small: a namespace, the volumes, and one entry per storage
/// naming its **selector** — the `strip_prefix` is derived, because typing it
/// is where a router that starts happily and stores nothing comes from
/// (RFC 09 §2). Backend parameters pass through verbatim; this type validates
/// the storage manager's part and not the backend's.
///
/// v1's class table (`class = "state"` and its siblings, each a v1 key
/// family) and the registry's `ttl_s` the lifespan was derived from left
/// with the v1 registry (#612, FJ9): a file that still names a class is
/// refused, naming the field.
///
/// ```toml
/// base = "fleet-a"                  # the deployment namespace; default = --namespace / context / ""
///
/// [volumes.fs]
/// plugin = "fs"                     # memory | fs | rocksdb | influxdb | redb | <other>
/// # history = "latest"              # the plugin fixes it, except redb (per volume, RFC 09 §2.1)
/// dir = "/var/lib/zenoh/fs"         # every other key passes through to the volume block
///
/// [volumes.influxdb]
/// plugin = "influxdb"
/// url = "http://localhost:8086"
///
/// [storages.events]
/// selector = "zk2/*/*/*/events/**"  # relative to the namespace
/// volume = "fs"
/// replication = true                # or a table of RFC 09 §2.2 parameters
/// params = { dir = "events" }       # merged into `volume: { id: …, … }`
///
/// [storages.timeseries]
/// selector = "plant/line-1/**"      # a foreign key family is a selector too
/// volume = "influxdb"
/// params = { db = "telemetry" }
/// gc_lifespan_s = 86400             # garbage_collection.lifespan; default zenoh's 24 h
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deployment {
    /// The deployment namespace every selector is joined to. `None` = take
    /// the observer's resolved `--namespace` (alias `--base`), the empty one
    /// being the bus-root deployment.
    pub base: Option<String>,
    #[serde(default)]
    pub volumes: BTreeMap<String, VolumeSpec>,
    #[serde(default)]
    pub storages: BTreeMap<String, StorageSpec>,
}

/// One volume of the deployment file.
///
/// No `deny_unknown_fields`, on purpose: everything but `plugin` and
/// `history` is the backend's own vocabulary (`dir`, `url`, `org`, `token`,
/// `path`…) and rides through to the emitted volume block untouched.
#[derive(Debug, Clone, Deserialize)]
pub struct VolumeSpec {
    /// The backend plugin: `memory`, `fs`, `rocksdb`, `influxdb`, `redb`, or
    /// an out-of-tree name this tool does not know the capability of.
    pub plugin: String,
    /// The history mode (RFC 09 §2.1). Fixed by the plugin for the four
    /// known rows; **per volume**, and required, for `redb` and for any
    /// plugin this tool does not know.
    pub history: Option<HistoryMode>,
    /// Backend parameters, verbatim.
    #[serde(flatten)]
    pub params: BTreeMap<String, serde_json::Value>,
}

/// The history half of a volume's capability pair (RFC 09 §2.1, v1.28):
/// whether the backend keeps one value per key or every sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryMode {
    Latest,
    All,
}

impl HistoryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryMode::Latest => "latest",
            HistoryMode::All => "all",
        }
    }
}

/// The persistence half of the capability pair (RFC 09 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Persistence {
    Volatile,
    Durable,
}

impl Persistence {
    pub fn as_str(self) -> &'static str {
        match self {
            Persistence::Volatile => "volatile",
            Persistence::Durable => "durable",
        }
    }
}

/// One storage of the deployment file. `deny_unknown_fields`, unlike the
/// volume: every key here is the convention's, so a typo is a refusal rather
/// than a silently ignored intent.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageSpec {
    /// The key expression the storage takes, relative to the namespace
    /// (`zk2/*/*/*/events/**`).
    pub selector: String,
    /// The volume id, declared under `[volumes]`.
    pub volume: String,
    /// Backend parameters merged into the storage's `volume: { id: …, … }`
    /// block (`dir`, `db`…), verbatim.
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    /// `true` for RFC 09 §2.2's example parameters, or a table of your own.
    #[serde(default)]
    pub replication: Replication,
    /// Ask for `complete: true`. Honoured only on a replicated latest-mode
    /// storage (RFC 09 §2.2); refused, and said so, elsewhere.
    #[serde(default)]
    pub complete: bool,
    /// A backend retention block (`redb`, RFC 09 §2.1), verbatim.
    pub retention: Option<serde_json::Value>,
    /// `garbage_collection.period`, seconds. Default 30, Zenoh's own.
    pub gc_period_s: Option<u64>,
    /// An explicit `garbage_collection.lifespan`, seconds. Default zenoh's
    /// own 24 h (RFC 09 §2.3).
    pub gc_lifespan_s: Option<i64>,
}

/// `replication = true | false | { interval = 10.0, … }`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Replication {
    Enabled(bool),
    Params(BTreeMap<String, serde_json::Value>),
}

impl Default for Replication {
    fn default() -> Self {
        Replication::Enabled(false)
    }
}

// ── The plan ──────────────────────────────────────────────────────────────

/// What `zenctl storage gen` planned (#393).
#[derive(Debug, Clone, Serialize)]
pub struct StoragePlan {
    /// The namespace every selector below was joined to.
    pub base: String,
    pub volumes: Vec<PlannedVolume>,
    pub storages: Vec<PlannedStorage>,
    /// What the plan left out, and why. A refused storage is **omitted** from
    /// `storages` and named here — the plan is still emitted around it.
    pub refusals: Vec<Refusal>,
}

impl StoragePlan {
    /// Every warning, with the storage or volume it concerns.
    pub fn warnings(&self) -> impl Iterator<Item = (&str, &PlanWarning)> {
        self.volumes
            .iter()
            .flat_map(|v| v.warnings.iter().map(move |w| (v.id.as_str(), w)))
            .chain(
                self.storages
                    .iter()
                    .flat_map(|s| s.warnings.iter().map(move |w| (s.name.as_str(), w))),
            )
    }
}

/// One volume, as the plan will emit it.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedVolume {
    pub id: String,
    pub plugin: String,
    pub history: HistoryMode,
    /// `None` = a plugin this tool does not know the capability of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistence: Option<Persistence>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<PlanWarning>,
}

/// One storage, as the plan will emit it.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedStorage {
    pub name: String,
    /// The full wire selector, namespace included.
    pub key_expr: String,
    /// Derived: the literal leftmost run of `key_expr`.
    pub strip_prefix: String,
    pub volume: String,
    /// The volume's history mode — the fact every §2.2 decision reads.
    pub history: HistoryMode,
    /// The replication block, when replicated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replication: Option<BTreeMap<String, serde_json::Value>>,
    /// As it will be emitted — `false` where the request was refused.
    pub complete: bool,
    pub garbage_collection: GarbageCollection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<PlanWarning>,
}

/// `garbage_collection: { period, lifespan }` with the computation shown
/// (RFC 09 §2.3).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GarbageCollection {
    pub period_s: u64,
    pub lifespan_s: i64,
    /// How `lifespan_s` came about, in one line a reader can check.
    pub derivation: String,
}

/// One thing the plan wants said beside a storage or a volume.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanWarning {
    pub kind: WarningKind,
    pub text: String,
    /// The clause it cites.
    pub cite: String,
}

/// The closed vocabulary of plan warnings — a new way for a deployment to be
/// wrong is a new variant, not a free-text note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// Two storages' selectors intersect: a GET under both is answered twice
    /// (RFC 09 §2).
    Overlap,
    /// `complete = true` asked for where RFC 09 §2.2 does not allow it; the
    /// plan emits `false`.
    CompleteRefused,
    /// Retention lives in the database; `garbage_collection` is not it
    /// (RFC 09 §2.3).
    RetentionIsTheDatabases,
    /// An all-mode `redb` storage without a retention block refuses to start
    /// (RFC 09 §2.1).
    RetentionRequired,
    /// A retention block on a latest-mode volume is refused at startup
    /// (RFC 09 §2.1).
    RetentionPointless,
    /// Replication parameters that break RFC 09 §2.2's rule.
    ReplicationParams,
    /// A plugin this tool does not know; the declared capability is taken on
    /// trust.
    UnknownPlugin,
}

/// One storage or volume the plan refused to emit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Refusal {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume: Option<String>,
    /// The selector the refused storage would have taken — so `--explain`
    /// can say "a refused storage would have taken this key".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_expr: Option<String>,
    pub reason: String,
    pub cite: String,
}

// ── --check ───────────────────────────────────────────────────────────────

/// `zenctl storage gen --check`: the plan against what a live router runs.
#[derive(Debug, Clone, Serialize)]
pub struct StorageCheck {
    pub base: String,
    /// The admin selector put to the bus (RFC 13 §3 O5).
    pub asked: String,
    pub planned: usize,
    pub observed: usize,
    pub findings: Vec<CheckFinding>,
    /// Comparisons the admin document could not carry — a field the layout
    /// omits is not a field that agrees.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unjudged: Vec<String>,
    pub judgement: Judgement,
}

/// One way the running configuration differs from the plan.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckFinding {
    pub kind: CheckKind,
    pub storage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// Planned, and no router runs it.
    Missing,
    /// Running, and the plan does not name it.
    Extra,
    KeyExprDiffers,
    StripPrefixDiffers,
    VolumeDiffers,
    /// The running `garbage_collection.lifespan` is below the plan's — a
    /// slow replica may resurrect a retired key (RFC 09 §2.3).
    LifespanBelowMinimum,
}

// ── --explain ─────────────────────────────────────────────────────────────

/// `zenctl storage gen --explain <key>`: which planned storage takes a key.
#[derive(Debug, Clone, Serialize)]
pub struct StorageExplain {
    pub key: String,
    pub base: String,
    pub takers: Vec<Taker>,
    /// Refused storages whose selector would have included the key.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refused_takers: Vec<String>,
    /// Why nothing takes it, when nothing does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub none_reason: Option<String>,
}

/// One storage that takes the key, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Taker {
    pub storage: String,
    pub key_expr: String,
    /// `includes` — every key the expression names lands here; `intersects`
    /// — the expression is itself a selector and only some of it does.
    pub relation: TakerRelation,
    pub why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TakerRelation {
    Includes,
    Intersects,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gc() -> GarbageCollection {
        GarbageCollection {
            period_s: 30,
            lifespan_s: 1800,
            derivation: "declared gc_lifespan_s 1800".into(),
        }
    }

    /// The deployment file's shape, pinned from the TOML side: the history
    /// token is lower-case, `replication` takes a bool or a table, and a
    /// volume's unknown keys ride through while a storage's are refused — v1's
    /// `class` among them.
    #[test]
    fn the_deployment_file_parses_as_documented() {
        let d: Deployment = serde_json::from_value(json!({
            "base": "fleet-a",
            "volumes": {
                "fs": {"plugin": "fs", "dir": "/var/lib/zenoh"},
                "redb-history": {"plugin": "redb", "history": "all"}
            },
            "storages": {
                "events": {"selector": "zk2/*/*/*/events/**", "volume": "fs",
                           "replication": true, "complete": true},
                "history": {"selector": "plant/**", "volume": "redb-history",
                            "replication": {"interval": 10.0}, "retention": {"max_age_s": 86400}}
            }
        }))
        .unwrap();
        assert_eq!(d.base.as_deref(), Some("fleet-a"));
        assert_eq!(d.volumes["fs"].params["dir"], json!("/var/lib/zenoh"));
        assert_eq!(d.volumes["redb-history"].history, Some(HistoryMode::All));
        assert_eq!(d.storages["events"].selector, "zk2/*/*/*/events/**");
        assert_eq!(d.storages["events"].replication, Replication::Enabled(true));
        assert!(matches!(
            d.storages["history"].replication,
            Replication::Params(ref p) if p["interval"] == json!(10.0)
        ));

        let typo: Result<Deployment, _> = serde_json::from_value(json!({
            "storages": {"e": {"selector": "x/**", "volume": "fs", "replicaton": true}}
        }));
        assert!(
            typo.is_err(),
            "a storage key this tool does not know is refused"
        );
        let v1: Result<Deployment, _> = serde_json::from_value(json!({
            "storages": {"latest": {"class": "state", "volume": "fs"}}
        }));
        assert!(
            v1.is_err(),
            "v1's class table is gone: a file naming one is refused"
        );
    }

    /// The plan's wire shape: a refused `complete` is emitted as `false`,
    /// and every vocabulary is snake_case.
    #[test]
    fn the_plan_pins_its_shape() {
        let plan = StoragePlan {
            base: "fleet-a".into(),
            volumes: vec![PlannedVolume {
                id: "fs".into(),
                plugin: "fs".into(),
                history: HistoryMode::Latest,
                persistence: Some(Persistence::Durable),
                params: BTreeMap::new(),
                warnings: vec![],
            }],
            storages: vec![PlannedStorage {
                name: "events".into(),
                key_expr: "fleet-a/zk2/*/*/*/events/**".into(),
                strip_prefix: "fleet-a/zk2".into(),
                volume: "fs".into(),
                history: HistoryMode::Latest,
                replication: None,
                complete: false,
                garbage_collection: gc(),
                retention: None,
                params: BTreeMap::new(),
                warnings: vec![PlanWarning {
                    kind: WarningKind::CompleteRefused,
                    text: "t".into(),
                    cite: "RFC 09 §2.2".into(),
                }],
            }],
            refusals: vec![Refusal {
                storage: Some("stray".into()),
                volume: None,
                key_expr: Some("fleet-a/plant/**".into()),
                reason: "r".into(),
                cite: "RFC 09 §2".into(),
            }],
        };
        let v = serde_json::to_value(&plan).unwrap();
        assert_eq!(v["volumes"][0]["persistence"], json!("durable"));
        assert_eq!(v["volumes"][0]["history"], json!("latest"));
        assert!(v["volumes"][0].get("params").is_none());
        let s = &v["storages"][0];
        assert!(s.get("class").is_none(), "v1's class table is gone");
        assert_eq!(s["complete"], json!(false));
        assert!(s.get("replication").is_none());
        assert_eq!(s["garbage_collection"]["lifespan_s"], json!(1800));
        assert_eq!(s["warnings"][0]["kind"], json!("complete_refused"));
        assert_eq!(v["refusals"][0]["storage"], json!("stray"));
        assert!(v["refusals"][0].get("volume").is_none());
    }

    /// `--check` carries its selector, its judgement and the finding
    /// vocabulary on the wire.
    #[test]
    fn the_check_pins_its_shape() {
        let check = StorageCheck {
            base: "".into(),
            asked: "@/*/router/**/storage_manager/storages/**".into(),
            planned: 1,
            observed: 1,
            findings: vec![CheckFinding {
                kind: CheckKind::LifespanBelowMinimum,
                storage: "latest".into(),
                zid: Some("aabb".into()),
                planned: Some("1800".into()),
                observed: Some("600".into()),
            }],
            unjudged: vec![],
            judgement: Judgement::Established,
        };
        let v = serde_json::to_value(&check).unwrap();
        assert_eq!(v["findings"][0]["kind"], json!("lifespan_below_minimum"));
        assert_eq!(v["judgement"], json!({"answer": "established"}));
        assert!(v.get("unjudged").is_none(), "empty unjudged is absence");
    }

    /// `--explain` distinguishes a key nothing takes from one a refused
    /// storage would have.
    #[test]
    fn the_explain_pins_its_shape() {
        let e = StorageExplain {
            key: "fleet-a/zk2/host-a/tc/tc.netif.v1/events/link/01k0".into(),
            base: "fleet-a".into(),
            takers: vec![Taker {
                storage: "events".into(),
                key_expr: "fleet-a/zk2/*/*/*/events/**".into(),
                relation: TakerRelation::Includes,
                why: "w".into(),
            }],
            refused_takers: vec![],
            none_reason: None,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["takers"][0]["relation"], json!("includes"));
        assert!(v.get("none_reason").is_none());
        assert!(v.get("refused_takers").is_none());
    }
}
