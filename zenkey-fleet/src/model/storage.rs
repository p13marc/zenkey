//! The storage planner (RFC 09 §2, #393): a deployment file in, the router's
//! `storage_manager` block out — with every derived number shown, every
//! caveat cited, and every refusal named.
//!
//! Each storage names its selector, relative to the deployment namespace; a
//! literal `strip_prefix` is derived from it and a volume's capability pair
//! comes from the §2.1 table. The two things a human types wrong here — a
//! `strip_prefix` that is not a literal prefix of its selector, and a
//! history mode the backend does not offer — produce a router that starts
//! happily and stores nothing, or refuses to start at all.
//!
//! **What left at FJ9** (#612): v1's class table (`state`, `telemetry`,
//! `events`, `catalog`, `catalog-pdns`, each a v1 key family) and the
//! lifespan v1 derived from the registry's longest `ttl_s`. A zk2 contract
//! declares no tombstone lifetime to derive one from, so a lifespan is the
//! file's, or zenoh's default, and says which. A storage on an owner's
//! state keys is what spec §4.2 S4 forbids; the doctor's
//! `storage-on-state` judges a running one.
//!
//! Pure, like everything in [`crate::model`]: values in hand, no session.
//! [`check_storages`] compares a plan against storages somebody else read
//! off the admin space; [`explain`] answers "which storage takes this key"
//! over the plan alone. [`to_json5`] is the one rendering of the plan that
//! is not zenkey's — it is `zenohd`'s.

use std::collections::BTreeMap;

use zenoh::key_expr::keyexpr;

use crate::report::{
    CheckFinding, CheckKind, Deployment, GarbageCollection, HistoryMode, Judgement, Persistence,
    PlanWarning, PlannedStorage, PlannedVolume, Refusal, Replication, StorageCheck, StorageExplain,
    StorageInfo, StoragePlan, Taker, TakerRelation, WarningKind,
};

/// Zenoh's own default `garbage_collection.lifespan`, seconds (RFC 09 §2.3:
/// "default 24 h").
pub const DEFAULT_LIFESPAN_S: i64 = 86_400;
/// Zenoh's own default `garbage_collection.period`, seconds.
pub const DEFAULT_GC_PERIOD_S: u64 = 30;

/// The admin selector `--check` reads storages from — the same one
/// [`crate::storages`] sweeps, restated here so the report can cite it.
pub const CHECK_ASKED: &str = "@/*/router/**/storage_manager/storages/**";

/// RFC 09 §2.1's capability table: persistence, and the history mode when
/// the plugin fixes it. `None` history = per volume (`redb`); `None` overall
/// = a plugin this tool does not know.
fn capability(plugin: &str) -> Option<(Persistence, Option<HistoryMode>)> {
    match plugin {
        "memory" => Some((Persistence::Volatile, Some(HistoryMode::Latest))),
        "fs" | "rocksdb" => Some((Persistence::Durable, Some(HistoryMode::Latest))),
        "influxdb" => Some((Persistence::Durable, Some(HistoryMode::All))),
        "redb" => Some((Persistence::Durable, None)),
        _ => None,
    }
}

/// RFC 09 §2.2's example replication block — what `replication = true`
/// means.
fn default_replication() -> BTreeMap<String, serde_json::Value> {
    [
        ("interval", serde_json::json!(10.0)),
        ("sub_intervals", serde_json::json!(5)),
        ("hot", serde_json::json!(6)),
        ("warm", serde_json::json!(30)),
        ("propagation_delay", serde_json::json!(250)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// The literal leftmost run of a key expression — the one `strip_prefix`
/// Zenoh accepts (string prefix, no wildcards). `fleet-a/zk2/*/*/*/events/**`
/// → `fleet-a/zk2`; `fleet-a/zk2/host-a/tc/@stream/**` →
/// `fleet-a/zk2/host-a/tc/@stream`. Empty when the expression opens on a
/// wildcard.
pub fn literal_prefix(key_expr: &str) -> String {
    key_expr
        .split('/')
        .take_while(|c| !c.contains('*') && !c.contains('$'))
        .collect::<Vec<_>>()
        .join("/")
}

fn warn(kind: WarningKind, cite: &str, text: impl Into<String>) -> PlanWarning {
    PlanWarning {
        kind,
        text: text.into(),
        cite: cite.to_string(),
    }
}

fn refuse_storage(
    name: &str,
    key_expr: Option<String>,
    cite: &str,
    reason: impl Into<String>,
) -> Refusal {
    Refusal {
        storage: Some(name.to_string()),
        volume: None,
        key_expr,
        reason: reason.into(),
        cite: cite.to_string(),
    }
}

/// Plan the storages of a deployment (#393).
///
/// `fallback_base` is the observer's resolved namespace, used when the
/// deployment file names none.
pub fn plan_storages(fallback_base: &str, deployment: &Deployment) -> StoragePlan {
    let base = deployment
        .base
        .clone()
        .unwrap_or_else(|| fallback_base.to_string());

    let mut refusals = Vec::new();

    // ── Volumes: the capability pair, per volume (RFC 09 §2.1 v1.28). ──
    let mut volumes = Vec::new();
    let mut refused_volumes: Vec<String> = Vec::new();
    for (id, spec) in &deployment.volumes {
        let mut warnings = Vec::new();
        let (persistence, history) = match capability(&spec.plugin) {
            Some((p, Some(fixed))) => match spec.history {
                Some(h) if h != fixed => {
                    refusals.push(Refusal {
                        storage: None,
                        volume: Some(id.clone()),
                        key_expr: None,
                        reason: format!(
                            "plugin {:?} offers {} history only; it cannot be declared \
                             history = {:?}",
                            spec.plugin,
                            fixed.as_str(),
                            h.as_str()
                        ),
                        cite: "RFC 09 §2.1".into(),
                    });
                    refused_volumes.push(id.clone());
                    continue;
                }
                _ => (Some(p), fixed),
            },
            Some((p, None)) => match spec.history {
                Some(h) => (Some(p), h),
                None => {
                    refusals.push(Refusal {
                        storage: None,
                        volume: Some(id.clone()),
                        key_expr: None,
                        reason: format!(
                            "plugin {:?} offers both history modes and the choice is per \
                             volume: declare history = \"latest\" or \"all\" (one volume \
                             per mode from the same plugin)",
                            spec.plugin
                        ),
                        cite: "RFC 09 §2.1".into(),
                    });
                    refused_volumes.push(id.clone());
                    continue;
                }
            },
            None => match spec.history {
                Some(h) => {
                    warnings.push(warn(
                        WarningKind::UnknownPlugin,
                        "RFC 09 §2.1",
                        format!(
                            "plugin {:?} is not in the capability table; its history = \
                             {:?} is taken as declared, not verified",
                            spec.plugin,
                            h.as_str()
                        ),
                    ));
                    (None, h)
                }
                None => {
                    refusals.push(Refusal {
                        storage: None,
                        volume: Some(id.clone()),
                        key_expr: None,
                        reason: format!(
                            "plugin {:?} is not in the capability table, so its history \
                             mode cannot be inferred: declare history = \"latest\" or \"all\"",
                            spec.plugin
                        ),
                        cite: "RFC 09 §2.1".into(),
                    });
                    refused_volumes.push(id.clone());
                    continue;
                }
            },
        };
        volumes.push(PlannedVolume {
            id: id.clone(),
            plugin: spec.plugin.clone(),
            history,
            persistence,
            params: spec.params.clone(),
            warnings,
        });
    }

    // ── Storages. ──
    let mut storages: Vec<PlannedStorage> = Vec::new();
    for (name, spec) in &deployment.storages {
        // The selector, joined to the namespace.
        let key_expr = crate::model::namespace::join(&base, &spec.selector);
        if keyexpr::new(key_expr.as_str()).is_err() {
            refusals.push(refuse_storage(
                name,
                Some(key_expr.clone()),
                "RFC 03 §2",
                format!("{key_expr:?} is not a valid key expression"),
            ));
            continue;
        }

        // The volume, and its mode — the fact every §2.2 decision reads.
        let Some(volume) = volumes.iter().find(|v| v.id == spec.volume) else {
            let reason = if refused_volumes.contains(&spec.volume) {
                format!("its volume {:?} was refused (see above)", spec.volume)
            } else {
                format!(
                    "names volume {:?}, which [volumes] does not declare",
                    spec.volume
                )
            };
            refusals.push(refuse_storage(name, Some(key_expr), "RFC 09 §2", reason));
            continue;
        };
        let history = volume.history;

        // Replication — refused, not discovered, on an all-mode volume.
        let replication = match &spec.replication {
            Replication::Enabled(false) => None,
            Replication::Enabled(true) => Some(default_replication()),
            Replication::Params(p) => Some(p.clone()),
        };
        if replication.is_some() && history == HistoryMode::All {
            refusals.push(refuse_storage(
                name,
                Some(key_expr),
                "RFC 09 §2.2",
                format!(
                    "declares replication on volume {:?}, whose history mode is all — \
                     the storage manager refuses to start such a storage, so this plan \
                     refuses it first (anti-entropy aligns one value per key; an all-mode \
                     volume cannot participate)",
                    volume.id
                ),
            ));
            continue;
        }

        let mut warnings = Vec::new();
        if let Some(p) = &replication
            && let (Some(interval), Some(delay)) = (
                p.get("interval").and_then(serde_json::Value::as_f64),
                p.get("propagation_delay")
                    .and_then(serde_json::Value::as_f64),
            )
            && delay / 1000.0 >= interval / 2.0
        {
            warnings.push(warn(
                WarningKind::ReplicationParams,
                "RFC 09 §2.2",
                format!(
                    "propagation_delay {delay} ms is not below interval/2 = {} ms — \
                     divergent or inconsistent replication parameters cause digest \
                     storms, not errors",
                    interval * 500.0
                ),
            ));
        }

        // The tombstone lifetime (RFC 09 §2.3): the file's, or zenoh's own.
        let (lifespan_s, derivation) = match spec.gc_lifespan_s {
            Some(explicit) => (explicit, format!("declared gc_lifespan_s {explicit}")),
            None => (
                DEFAULT_LIFESPAN_S,
                format!(
                    "zenoh's default {DEFAULT_LIFESPAN_S} s — no contract declares a tombstone \
                     lifetime to derive one from"
                ),
            ),
        };

        // `complete: true` — right in exactly one place (RFC 09 §2.2).
        let mut complete = spec.complete;
        if complete && !(replication.is_some() && history == HistoryMode::Latest) {
            complete = false;
            let why = if history != HistoryMode::Latest {
                "its volume is not latest-mode"
            } else {
                "it is not replicated"
            };
            warnings.push(warn(
                WarningKind::CompleteRefused,
                "RFC 09 §2.2",
                format!(
                    "complete = true refused: {why} — complete is right only on a \
                     replicated latest-mode storage, where it lets the router answer a GET \
                     from the nearest replica; emitted as false"
                ),
            ));
        }

        // The volume's row of the §2.1 table, and its caveats.
        match volume.plugin.as_str() {
            "influxdb" => warnings.push(warn(
                WarningKind::RetentionIsTheDatabases,
                "RFC 09 §2.3",
                "retention is the database's policy (an InfluxDB retention policy), not \
                 zenoh config: garbage_collection prunes metadata and never drops a value, \
                 so this storage's data grows until the database prunes it — size the \
                 volume against the write rate",
            )),
            "redb" => match (history, &spec.retention) {
                (HistoryMode::All, None) => warnings.push(warn(
                    WarningKind::RetentionRequired,
                    "RFC 09 §2.1",
                    "an all-mode redb storage that declares no retention policy refuses to \
                     start — add a retention block bounding age, bytes or per-key samples \
                     (a loud config error is recoverable in seconds; a full disk is not)",
                )),
                (HistoryMode::Latest, Some(_)) => warnings.push(warn(
                    WarningKind::RetentionPointless,
                    "RFC 09 §2.1",
                    "a retention block on a latest-mode volume is refused at startup: there \
                     is no history to prune, and it would report passes while reclaiming \
                     nothing",
                )),
                _ => {}
            },
            _ => {}
        }

        storages.push(PlannedStorage {
            name: name.clone(),
            strip_prefix: literal_prefix(&key_expr),
            key_expr,
            volume: volume.id.clone(),
            history,
            replication,
            complete,
            garbage_collection: GarbageCollection {
                period_s: spec.gc_period_s.unwrap_or(DEFAULT_GC_PERIOD_S),
                lifespan_s,
                derivation,
            },
            retention: spec.retention.clone(),
            params: spec.params.clone(),
            warnings,
        });
    }

    // ── Overlaps (RFC 09 §2). ──
    let mut overlaps: Vec<(usize, usize)> = Vec::new();
    for i in 0..storages.len() {
        for j in (i + 1)..storages.len() {
            let (a, b) = (&storages[i], &storages[j]);
            if let (Ok(ka), Ok(kb)) = (
                keyexpr::new(a.key_expr.as_str()),
                keyexpr::new(b.key_expr.as_str()),
            ) && ka.intersects(kb)
            {
                overlaps.push((i, j));
            }
        }
    }
    for (i, j) in overlaps {
        for (this, other) in [(i, j), (j, i)] {
            let text = format!(
                "overlaps {} ({}): a GET under both selectors is answered by both, \
                 duplicate and possibly divergent — accept it (subscribers are unaffected; \
                 GET consumers consolidate) or carve one selector out of the other",
                storages[other].name, storages[other].key_expr
            );
            storages[this]
                .warnings
                .push(warn(WarningKind::Overlap, "RFC 09 §2", text));
        }
    }

    StoragePlan {
        base,
        volumes,
        storages,
        refusals,
    }
}

// ── The zenohd rendering ──────────────────────────────────────────────────

/// A JSON5 object key: bare where JSON5 allows it, quoted otherwise.
fn json5_key(k: &str) -> String {
    let bare = !k.is_empty()
        && !k.starts_with(|c: char| c.is_ascii_digit())
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if bare {
        k.to_string()
    } else {
        serde_json::to_string(k).expect("a string serializes")
    }
}

/// A JSON value on one line — JSON is JSON5.
fn json5_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(m) => {
            let inner: Vec<String> = m
                .iter()
                .map(|(k, v)| format!("{}: {}", json5_key(k), json5_value(v)))
                .collect();
            format!("{{ {} }}", inner.join(", "))
        }
        other => serde_json::to_string(other).expect("a value serializes"),
    }
}

/// One `{ id: "fs", dir: "latest" }`-shaped object from an id and params.
fn json5_object<'a>(
    head: impl IntoIterator<Item = (&'a str, serde_json::Value)>,
    params: &BTreeMap<String, serde_json::Value>,
) -> String {
    let mut parts: Vec<String> = head
        .into_iter()
        .map(|(k, v)| format!("{}: {}", json5_key(k), json5_value(&v)))
        .collect();
    parts.extend(
        params
            .iter()
            .map(|(k, v)| format!("{}: {}", json5_key(k), json5_value(v))),
    );
    if parts.is_empty() {
        "{}".to_string()
    } else {
        format!("{{ {} }}", parts.join(", "))
    }
}

/// The plan as the `plugins.storage_manager` block `zenohd` reads — JSON5,
/// with the derivations and every warning as comments beside the storage
/// they concern, and the refusals where the refused storage would have been.
///
/// Merge it under the router config's `plugins`; each non-memory volume needs
/// its backend plugin installed and version-matched to the router (RFC 09
/// §2).
pub fn to_json5(plan: &StoragePlan) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// zenohd storage_manager block — generated by `zenctl storage gen` (RFC 09 §2)."
    );
    let _ = writeln!(out, "// namespace {:?}.", plan.base);
    let _ = writeln!(
        out,
        "// Merge under the router config's `plugins`; every non-memory volume needs its \
         backend plugin installed, version-matched to the router."
    );
    let _ = writeln!(out, "plugins: {{");
    let _ = writeln!(out, "  storage_manager: {{");

    // Volumes.
    let _ = writeln!(out, "    volumes: {{");
    for v in &plan.volumes {
        for w in &v.warnings {
            let _ = writeln!(out, "      // ! {}: {} ({})", w.kind_str(), w.text, w.cite);
        }
        let mut head: Vec<(&str, serde_json::Value)> = Vec::new();
        if v.id != v.plugin {
            head.push(("backend", serde_json::json!(v.plugin)));
        }
        // `history` is a volume knob only where the plugin offers both modes;
        // the fixed rows would refuse an unknown key.
        if capability(&v.plugin).is_some_and(|(_, fixed)| fixed.is_none()) {
            head.push(("history", serde_json::json!(v.history.as_str())));
        }
        let pair = match v.persistence {
            Some(p) => format!("{} · {}", p.as_str(), v.history.as_str()),
            None => format!(
                "? · {} (plugin not in the capability table)",
                v.history.as_str()
            ),
        };
        let _ = writeln!(
            out,
            "      {}: {},  // {pair} (RFC 09 §2.1)",
            json5_key(&v.id),
            json5_object(head, &v.params)
        );
    }
    let _ = writeln!(out, "    }},");

    // Storages.
    let _ = writeln!(out, "    storages: {{");
    for r in &plan.refusals {
        let what = match (&r.storage, &r.volume) {
            (Some(s), _) => format!("storage {s}"),
            (None, Some(v)) => format!("volume {v}"),
            (None, None) => "entry".to_string(),
        };
        let _ = writeln!(out, "      // REFUSED {what}: {} ({})", r.reason, r.cite);
    }
    for s in &plan.storages {
        let _ = writeln!(out, "      // {}", s.name);
        for w in &s.warnings {
            let _ = writeln!(out, "      // ! {}: {} ({})", w.kind_str(), w.text, w.cite);
        }
        let _ = writeln!(out, "      {}: {{", json5_key(&s.name));
        let _ = writeln!(
            out,
            "        key_expr: {},",
            json5_value(&serde_json::json!(s.key_expr))
        );
        let _ = writeln!(
            out,
            "        strip_prefix: {},  // derived: the literal leftmost run of key_expr",
            json5_value(&serde_json::json!(s.strip_prefix))
        );
        let _ = writeln!(
            out,
            "        volume: {},",
            json5_object([("id", serde_json::json!(s.volume))], &s.params)
        );
        if let Some(rep) = &s.replication {
            let _ = writeln!(
                out,
                "        replication: {},  // identical on every replica (RFC 09 §2.2)",
                json5_object([], rep)
            );
        }
        if s.complete {
            let _ = writeln!(
                out,
                "        complete: true,  // replicated latest-mode storage (RFC 09 §2.2)"
            );
        }
        if let Some(ret) = &s.retention {
            let _ = writeln!(
                out,
                "        retention: {},  // the backend's own policy (RFC 09 §2.1)",
                json5_value(ret)
            );
        }
        let _ = writeln!(
            out,
            "        garbage_collection: {{ period: {}, lifespan: {} }},  // {} (RFC 09 §2.3)",
            s.garbage_collection.period_s,
            s.garbage_collection.lifespan_s,
            s.garbage_collection.derivation
        );
        let _ = writeln!(out, "      }},");
    }
    let _ = writeln!(out, "    }},");
    let _ = writeln!(out, "  }},");
    let _ = writeln!(out, "}}");
    out
}

impl PlanWarning {
    /// The kind as its wire token.
    pub fn kind_str(&self) -> String {
        match serde_json::to_value(self.kind).expect("a kind serializes") {
            serde_json::Value::String(s) => s,
            _ => unreachable!("a unit variant serializes to a string"),
        }
    }
}

// ── --check ───────────────────────────────────────────────────────────────

/// The running `garbage_collection.lifespan` of a storage, seconds, as far
/// as the admin document says. Layouts vary by version — a bare number, or
/// serde's `{ secs, nanos }` for a `Duration` — and an absent field is
/// `None`, which is *unjudged* and not *agreeing*.
fn observed_lifespan(raw: &serde_json::Value) -> Option<f64> {
    let gc = raw
        .get("garbage_collection")
        .or_else(|| raw.get("garbage_collection_config"))?;
    let lifespan = gc.get("lifespan")?;
    lifespan
        .as_f64()
        .or_else(|| lifespan.get("secs").and_then(serde_json::Value::as_f64))
}

/// Compare a plan against the storages a router admits to running (#393).
///
/// `observed` is what [`crate::storages`] read off the admin space. Empty is
/// **unobservable**, not clean: a peer-only mesh, a router without the
/// storage manager and a disabled admin space all answer nothing, and none
/// of them is a router running the plan.
pub fn check_storages(plan: &StoragePlan, observed: &[StorageInfo]) -> StorageCheck {
    let mut findings = Vec::new();
    let mut unjudged = Vec::new();
    if observed.is_empty() {
        return StorageCheck {
            base: plan.base.clone(),
            asked: CHECK_ASKED.into(),
            planned: plan.storages.len(),
            observed: 0,
            findings,
            unjudged,
            judgement: Judgement::Unobservable {
                reason: "the admin space answered no storages — a peer-only mesh, a router \
                         without the storage manager, or the admin space is disabled; there \
                         is nothing to compare the plan against"
                    .into(),
            },
        };
    }
    for p in &plan.storages {
        let rows: Vec<&StorageInfo> = observed.iter().filter(|o| o.name == p.name).collect();
        if rows.is_empty() {
            findings.push(CheckFinding {
                kind: CheckKind::Missing,
                storage: p.name.clone(),
                zid: None,
                planned: Some(p.key_expr.clone()),
                observed: None,
            });
            continue;
        }
        for o in rows {
            let mut differs =
                |kind: CheckKind, planned: &str, observed: Option<&str>, field| match observed {
                    Some(v) if v == planned => {}
                    Some(v) => findings.push(CheckFinding {
                        kind,
                        storage: p.name.clone(),
                        zid: Some(o.zid.clone()),
                        planned: Some(planned.to_string()),
                        observed: Some(v.to_string()),
                    }),
                    None => unjudged.push(format!(
                        "{}@{}: the admin document does not carry {field}",
                        p.name, o.zid
                    )),
                };
            differs(
                CheckKind::KeyExprDiffers,
                &p.key_expr,
                o.key_expr.as_deref(),
                "key_expr",
            );
            differs(
                CheckKind::StripPrefixDiffers,
                &p.strip_prefix,
                o.strip_prefix.as_deref(),
                "strip_prefix",
            );
            differs(
                CheckKind::VolumeDiffers,
                &p.volume,
                o.volume.as_deref(),
                "volume",
            );
            match observed_lifespan(&o.raw) {
                Some(l) if l < p.garbage_collection.lifespan_s as f64 => {
                    findings.push(CheckFinding {
                        kind: CheckKind::LifespanBelowMinimum,
                        storage: p.name.clone(),
                        zid: Some(o.zid.clone()),
                        planned: Some(p.garbage_collection.lifespan_s.to_string()),
                        observed: Some(l.to_string()),
                    });
                }
                Some(_) => {}
                None => unjudged.push(format!(
                    "{}@{}: the admin document does not carry garbage_collection.lifespan",
                    p.name, o.zid
                )),
            }
        }
    }
    for o in observed {
        if !plan.storages.iter().any(|p| p.name == o.name) {
            findings.push(CheckFinding {
                kind: CheckKind::Extra,
                storage: o.name.clone(),
                zid: Some(o.zid.clone()),
                planned: None,
                observed: o.key_expr.clone(),
            });
        }
    }
    let judgement = if findings.is_empty() {
        Judgement::NotEstablished {
            reason: format!(
                "every planned storage runs as planned on {} observed row(s)",
                observed.len()
            ),
        }
    } else {
        Judgement::Established
    };
    StorageCheck {
        base: plan.base.clone(),
        asked: CHECK_ASKED.into(),
        planned: plan.storages.len(),
        observed: observed.len(),
        findings,
        unjudged,
        judgement,
    }
}

// ── --explain ─────────────────────────────────────────────────────────────

/// Which planned storage(s) take `key`, and why (#393). Pure over the plan.
pub fn explain(plan: &StoragePlan, key: &str) -> StorageExplain {
    let mut takers = Vec::new();
    let mut refused_takers = Vec::new();
    let Ok(k) = keyexpr::new(key) else {
        return StorageExplain {
            key: key.to_string(),
            base: plan.base.clone(),
            takers,
            refused_takers,
            none_reason: Some(format!("{key:?} is not a valid key expression (RFC 03 §2)")),
        };
    };
    for s in &plan.storages {
        let Ok(ke) = keyexpr::new(s.key_expr.as_str()) else {
            continue;
        };
        let relation = if ke.includes(k) {
            TakerRelation::Includes
        } else if ke.intersects(k) {
            TakerRelation::Intersects
        } else {
            continue;
        };
        let basis = format!("its selector under namespace {:?}", plan.base);
        let why = match relation {
            TakerRelation::Includes => format!(
                "{basis}: {} includes every key {key} names; stored under strip_prefix {:?} on volume {} ({})",
                s.key_expr,
                s.strip_prefix,
                s.volume,
                s.history.as_str()
            ),
            TakerRelation::Intersects => format!(
                "{basis}: {} intersects {key} — some keys under it land here, not all",
                s.key_expr
            ),
        };
        takers.push(Taker {
            storage: s.name.clone(),
            key_expr: s.key_expr.clone(),
            relation,
            why,
        });
    }
    for r in &plan.refusals {
        if let (Some(name), Some(ke)) = (&r.storage, &r.key_expr)
            && keyexpr::new(ke.as_str()).is_ok_and(|ke| ke.includes(k))
        {
            refused_takers.push(name.clone());
        }
    }
    let none_reason = takers.is_empty().then(|| {
        let verbatim = crate::model::namespace::strip(&plan.base, key)
            .unwrap_or(key)
            .split('/')
            .any(|c| c.starts_with('@'));
        let mut reason = String::from("no planned storage's selector includes it");
        if verbatim {
            reason.push_str(
                " — it holds a verbatim chunk (`@…`), and `*`/`**` never match one: a \
                 storage must name it",
            );
        }
        if !refused_takers.is_empty() {
            reason.push_str(&format!(
                "; refused storage(s) {} would have",
                refused_takers.join(", ")
            ));
        }
        reason
    });
    StorageExplain {
        key: key.to_string(),
        base: plan.base.clone(),
        takers,
        refused_takers,
        none_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{StorageSpec, VolumeSpec};

    fn volume(plugin: &str, history: Option<HistoryMode>) -> VolumeSpec {
        VolumeSpec {
            plugin: plugin.into(),
            history,
            params: BTreeMap::new(),
        }
    }

    fn storage(selector: &str, volume: &str) -> StorageSpec {
        StorageSpec {
            selector: selector.into(),
            volume: volume.into(),
            ..Default::default()
        }
    }

    /// A zk2 deployment's storages, by selector.
    fn reference() -> Deployment {
        let mut d = Deployment {
            base: Some("fleet-a".into()),
            ..Default::default()
        };
        d.volumes.insert("fs".into(), volume("fs", None));
        d.volumes
            .insert("influxdb".into(), volume("influxdb", None));
        d.storages.insert("events".into(), {
            let mut s = storage("zk2/*/*/*/events/**", "fs");
            s.replication = Replication::Enabled(true);
            s.complete = true;
            s
        });
        d.storages.insert(
            "timeseries".into(),
            storage("zk2/*/*/*/stream/**", "influxdb"),
        );
        d.storages.insert("links".into(), {
            let mut s = storage("zk2/*/*/*/events/link/**", "fs");
            s.gc_lifespan_s = Some(3600);
            s
        });
        d.storages.insert(
            "frames".into(),
            storage("zk2/*/*/*/@stream/frames/**", "influxdb"),
        );
        d
    }

    fn by_name<'p>(plan: &'p StoragePlan, name: &str) -> &'p PlannedStorage {
        plan.storages
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("{name} planned; refusals: {:?}", plan.refusals))
    }

    #[test]
    fn the_literal_prefix_stops_at_the_first_wildcard() {
        assert_eq!(literal_prefix("fleet-a/zk2/*/*/*/events/**"), "fleet-a/zk2");
        assert_eq!(
            literal_prefix("fleet-a/zk2/host-a/tc/@stream/**"),
            "fleet-a/zk2/host-a/tc/@stream"
        );
        assert_eq!(literal_prefix("zk2/*/*/*/events/**"), "zk2");
        assert_eq!(literal_prefix("**"), "");
        assert_eq!(literal_prefix("a/b$*/c"), "a");
    }

    /// The headline: the selector joined to the namespace, `strip_prefix`
    /// derived, and a lifespan that says where it came from.
    #[test]
    fn selectors_are_joined_and_lifespans_say_where_they_came_from() {
        let plan = plan_storages("", &reference());
        assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);

        let events = by_name(&plan, "events");
        assert_eq!(events.key_expr, "fleet-a/zk2/*/*/*/events/**");
        assert_eq!(events.strip_prefix, "fleet-a/zk2");
        assert_eq!(events.garbage_collection.lifespan_s, DEFAULT_LIFESPAN_S);
        assert!(
            events
                .garbage_collection
                .derivation
                .contains("no contract declares a tombstone lifetime"),
            "{}",
            events.garbage_collection.derivation
        );
        assert!(events.complete, "replicated, latest-mode");
        assert!(events.replication.is_some());

        let links = by_name(&plan, "links");
        assert_eq!(links.garbage_collection.lifespan_s, 3600);
        assert_eq!(
            links.garbage_collection.derivation,
            "declared gc_lifespan_s 3600"
        );

        let frames = by_name(&plan, "frames");
        assert_eq!(frames.strip_prefix, "fleet-a/zk2");

        let ts = by_name(&plan, "timeseries");
        assert!(
            ts.warnings
                .iter()
                .any(|w| w.kind == WarningKind::RetentionIsTheDatabases && w.cite == "RFC 09 §2.3")
        );
    }

    /// Two selectors that intersect are an overlap, warned on both sides; a
    /// verbatim chunk is not reached by a `*` and makes none.
    #[test]
    fn intersecting_selectors_overlap_and_a_verbatim_chunk_does_not() {
        let plan = plan_storages("", &reference());
        let overlaps = |name: &str| -> Vec<String> {
            by_name(&plan, name)
                .warnings
                .iter()
                .filter(|w| w.kind == WarningKind::Overlap)
                .map(|w| w.text.clone())
                .collect()
        };
        assert!(overlaps("events")[0].contains("overlaps links"));
        assert!(overlaps("links")[0].contains("overlaps events"));
        assert!(
            overlaps("timeseries").is_empty(),
            "`*` never matches @stream: {:?}",
            overlaps("timeseries")
        );
    }

    /// RFC 09 §2.2: replication on an all-mode volume is a startup refusal,
    /// so the plan refuses first — and names the storage.
    #[test]
    fn replication_on_an_all_mode_volume_is_refused() {
        let mut d = reference();
        d.storages.get_mut("timeseries").unwrap().replication = Replication::Enabled(true);
        let plan = plan_storages("", &d);
        assert!(plan.storages.iter().all(|s| s.name != "timeseries"));
        let r = plan
            .refusals
            .iter()
            .find(|r| r.storage.as_deref() == Some("timeseries"))
            .expect("refused");
        assert_eq!(r.cite, "RFC 09 §2.2");
        assert_eq!(r.key_expr.as_deref(), Some("fleet-a/zk2/*/*/*/stream/**"));
    }

    /// `complete = true` off a replicated latest-mode storage is emitted as
    /// `false` and said so.
    #[test]
    fn complete_is_refused_off_a_replicated_latest_storage() {
        let mut d = reference();
        d.storages.get_mut("links").unwrap().complete = true;
        d.storages.get_mut("timeseries").unwrap().complete = true;
        let plan = plan_storages("", &d);
        for (name, why) in [
            ("links", "it is not replicated"),
            ("timeseries", "its volume is not latest-mode"),
        ] {
            let s = by_name(&plan, name);
            assert!(!s.complete);
            assert!(
                s.warnings
                    .iter()
                    .any(|w| w.kind == WarningKind::CompleteRefused
                        && w.cite == "RFC 09 §2.2"
                        && w.text.contains(why)),
                "{name}: {:?}",
                s.warnings
            );
        }
    }

    /// The other refusals: an undeclared volume, a bad key expression, a
    /// redb volume without a mode.
    #[test]
    fn undeclared_volumes_and_bad_selectors_are_refused() {
        let mut d = reference();
        d.storages.insert("stray".into(), storage("zk2/**", "nope"));
        d.storages.insert("typo".into(), storage("zk2//x", "fs"));
        d.volumes.insert("redb".into(), volume("redb", None));
        let plan = plan_storages("", &d);
        let refused = |name: &str| {
            plan.refusals
                .iter()
                .find(|r| r.storage.as_deref() == Some(name) || r.volume.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name} refused: {:?}", plan.refusals))
        };
        assert!(refused("stray").reason.contains("does not declare"));
        assert!(
            refused("typo")
                .reason
                .contains("not a valid key expression")
        );
        assert!(refused("redb").reason.contains("per volume"));
        assert_eq!(plan.storages.len(), 4);
    }

    /// redb's per-volume mode: retention is mandatory in all mode and refused
    /// in latest mode (RFC 09 §2.1).
    #[test]
    fn redb_retention_is_judged_by_the_volumes_mode() {
        let mut d = reference();
        d.volumes.insert(
            "redb-history".into(),
            volume("redb", Some(HistoryMode::All)),
        );
        d.volumes
            .insert("redb".into(), volume("redb", Some(HistoryMode::Latest)));
        d.storages.insert(
            "timeseries".into(),
            storage("zk2/*/*/*/stream/**", "redb-history"),
        );
        d.storages.insert("links".into(), {
            let mut s = storage("zk2/*/*/*/events/link/**", "redb");
            s.retention = Some(serde_json::json!({"max_age_s": 60}));
            s
        });
        let plan = plan_storages("", &d);
        assert!(
            by_name(&plan, "timeseries")
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::RetentionRequired)
        );
        assert!(
            by_name(&plan, "links")
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::RetentionPointless)
        );
        // And an `fs` volume declared all-mode is a refused volume, taking
        // its storages with it.
        d.volumes
            .insert("fs".into(), volume("fs", Some(HistoryMode::All)));
        let plan = plan_storages("", &d);
        assert!(
            plan.refusals
                .iter()
                .any(|r| r.volume.as_deref() == Some("fs"))
        );
        assert!(
            plan.refusals
                .iter()
                .any(|r| r.storage.as_deref() == Some("events") && r.reason.contains("refused"))
        );
    }

    /// The namespace falls back to the observer's when the file names none;
    /// the bus root prefixes nothing.
    #[test]
    fn the_namespace_falls_back_and_the_bus_root_is_the_identity() {
        let mut d = reference();
        d.base = None;
        let plan = plan_storages("", &d);
        assert_eq!(by_name(&plan, "events").key_expr, "zk2/*/*/*/events/**");
        assert_eq!(by_name(&plan, "events").strip_prefix, "zk2");
        let plan = plan_storages("acme", &d);
        assert_eq!(
            by_name(&plan, "events").key_expr,
            "acme/zk2/*/*/*/events/**"
        );
    }

    /// The JSON5 is RFC 09 §2's shape, with the numbers filled in and the
    /// caveats beside the storage they concern.
    #[test]
    fn the_json5_carries_the_block_and_its_comments() {
        let plan = plan_storages("", &reference());
        let doc = to_json5(&plan);
        assert!(doc.contains("// namespace \"fleet-a\"."));
        assert!(doc.contains("plugins: {\n  storage_manager: {\n    volumes: {"));
        assert!(doc.contains("      fs: {},  // durable · latest (RFC 09 §2.1)"));
        assert!(doc.contains("        key_expr: \"fleet-a/zk2/*/*/*/events/**\","));
        assert!(doc.contains("        strip_prefix: \"fleet-a/zk2\","));
        assert!(doc.contains(
            "garbage_collection: { period: 30, lifespan: 3600 },  // declared gc_lifespan_s 3600"
        ));
        assert!(doc.contains("replication: { hot: 6, interval: 10.0, propagation_delay: 250, sub_intervals: 5, warm: 30 }"));
        assert!(doc.contains("        complete: true,"));
        assert!(doc.contains("      // ! overlap: overlaps links"));
        assert!(doc.contains("      // ! retention_is_the_databases:"));
        assert!(
            !doc.contains("redb"),
            "nothing said of redb's retention on other rows"
        );
        // A quoted key where JSON5 needs one, a bare one where it does not.
        assert_eq!(json5_key("redb-history"), "\"redb-history\"");
        assert_eq!(json5_key("fs"), "fs");
        let refused = {
            let mut d = reference();
            d.storages.get_mut("timeseries").unwrap().replication = Replication::Enabled(true);
            to_json5(&plan_storages("", &d))
        };
        assert!(refused.contains("      // REFUSED storage timeseries:"));
    }

    fn observed(
        name: &str,
        key_expr: &str,
        strip: &str,
        volume: &str,
        lifespan: Option<i64>,
    ) -> StorageInfo {
        let mut raw = serde_json::json!({"key_expr": key_expr});
        if let Some(l) = lifespan {
            raw["garbage_collection"] = serde_json::json!({"period": 30, "lifespan": l});
        }
        StorageInfo {
            zid: "aabbccdd".into(),
            name: name.into(),
            key_expr: Some(key_expr.into()),
            strip_prefix: Some(strip.into()),
            volume: Some(volume.into()),
            raw,
        }
    }

    /// `--check` over a hand-built admin reading: every finding kind, and the
    /// two non-verdicts (empty admin space; a field the layout omits).
    #[test]
    fn the_check_diffs_the_plan_against_what_runs() {
        let plan = plan_storages("", &reference());

        let empty = check_storages(&plan, &[]);
        assert!(empty.judgement.is_unobservable());
        assert_eq!(crate::judgement_exit_code(&empty.judgement), 2);

        let clean = vec![
            observed(
                "events",
                "fleet-a/zk2/*/*/*/events/**",
                "fleet-a/zk2",
                "fs",
                Some(86400),
            ),
            observed(
                "timeseries",
                "fleet-a/zk2/*/*/*/stream/**",
                "fleet-a/zk2",
                "influxdb",
                Some(86400),
            ),
            observed(
                "links",
                "fleet-a/zk2/*/*/*/events/link/**",
                "fleet-a/zk2",
                "fs",
                Some(3600),
            ),
            observed(
                "frames",
                "fleet-a/zk2/*/*/*/@stream/frames/**",
                "fleet-a/zk2",
                "influxdb",
                Some(86400),
            ),
        ];
        let c = check_storages(&plan, &clean);
        assert!(c.findings.is_empty(), "{:?}", c.findings);
        assert_eq!(crate::judgement_exit_code(&c.judgement), 0);
        assert_eq!(c.asked, CHECK_ASKED);

        let drifted = vec![
            // Too short a lifespan, and the wrong prefix.
            observed(
                "events",
                "fleet-a/zk2/*/*/*/events/**",
                "fleet-a",
                "fs",
                Some(600),
            ),
            // Wrong selector, wrong volume.
            observed(
                "timeseries",
                "fleet-a/zk2/**",
                "fleet-a/zk2",
                "memory",
                Some(86400),
            ),
            // Layout omits the gc block.
            observed(
                "links",
                "fleet-a/zk2/*/*/*/events/link/**",
                "fleet-a/zk2",
                "fs",
                None,
            ),
            // Not planned at all.
            observed(
                "state",
                "fleet-a/zk2/*/*/*/state/**",
                "fleet-a/zk2",
                "fs",
                None,
            ),
        ];
        let c = check_storages(&plan, &drifted);
        let kinds: Vec<(CheckKind, &str)> = c
            .findings
            .iter()
            .map(|f| (f.kind, f.storage.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (CheckKind::StripPrefixDiffers, "events"),
                (CheckKind::LifespanBelowMinimum, "events"),
                (CheckKind::Missing, "frames"),
                (CheckKind::KeyExprDiffers, "timeseries"),
                (CheckKind::VolumeDiffers, "timeseries"),
                (CheckKind::Extra, "state"),
            ]
        );
        assert_eq!(
            c.unjudged,
            ["links@aabbccdd: the admin document does not carry garbage_collection.lifespan"]
        );
        assert_eq!(crate::judgement_exit_code(&c.judgement), 1);

        // serde's Duration shape for the lifespan is read too.
        assert_eq!(
            observed_lifespan(
                &serde_json::json!({"garbage_collection_config": {"lifespan": {"secs": 7, "nanos": 0}}})
            ),
            Some(7.0)
        );
    }

    /// `--explain`: the taker and its reason; the verbatim key nobody takes;
    /// the key a refused storage would have taken.
    #[test]
    fn explain_names_the_taker_or_the_reason_there_is_none() {
        let mut d = reference();
        d.storages.remove("frames");
        d.storages.remove("links");
        d.storages.get_mut("timeseries").unwrap().replication = Replication::Enabled(true);
        let plan = plan_storages("", &d);

        let e = explain(&plan, "fleet-a/zk2/host-a/tc/tc.netif.v1/events/link/01k0");
        assert_eq!(e.takers.len(), 1);
        assert_eq!(e.takers[0].storage, "events");
        assert_eq!(e.takers[0].relation, TakerRelation::Includes);
        assert!(
            e.takers[0]
                .why
                .contains("its selector under namespace \"fleet-a\""),
            "{}",
            e.takers[0].why
        );
        assert!(e.none_reason.is_none());

        let e = explain(&plan, "fleet-a/zk2/host-a/tc/tc.netif.v1/@stream/frames");
        assert!(e.takers.is_empty());
        assert!(
            e.none_reason.as_deref().unwrap().contains("verbatim chunk"),
            "{:?}",
            e.none_reason
        );

        let e = explain(&plan, "fleet-a/zk2/host-a/tc/tc.netif.v1/stream/bandwidth");
        assert!(e.takers.is_empty());
        assert_eq!(e.refused_takers, ["timeseries"]);
        assert!(
            e.none_reason
                .as_deref()
                .unwrap()
                .contains("refused storage(s) timeseries would have")
        );

        let e = explain(&plan, "fleet-a/zk2/*/*/*/events/**");
        assert_eq!(e.takers[0].relation, TakerRelation::Includes);
        let e = explain(&plan, "fleet-a/zk2/**");
        assert!(
            e.takers
                .iter()
                .all(|t| t.relation == TakerRelation::Intersects)
        );

        let e = explain(&plan, "a//b");
        assert!(
            e.none_reason
                .as_deref()
                .unwrap()
                .contains("not a valid key expression")
        );
    }
}
