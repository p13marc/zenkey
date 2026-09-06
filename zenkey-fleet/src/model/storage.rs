//! The storage planner (RFC 09 §2, #393): a deployment file plus the registry
//! in, the router's `storage_manager` block out — with every derived number
//! shown, every caveat cited, and every refusal named.
//!
//! RFC 09 §2 specifies the class-driven storages, each with a selector, a
//! literal `strip_prefix`, a volume from the §2.1 capability table, and a
//! `garbage_collection.lifespan` that must be ≥ the longest `ttl_s` in the
//! registry (§2.3, the tombstone-visibility row of RFC 04 §1.2). The registry
//! knows that number; until now nobody computed it, and the two things a
//! human types wrong here — a `strip_prefix` that is not a literal prefix of
//! its selector, and a lifespan chosen by feel — produce a router that starts
//! happily and stores nothing, or resurrects a retired key from a slow
//! replica.
//!
//! Pure, like everything in [`crate::model`]: values in hand, no session.
//! [`plan_storages`] takes an *optional* registry and says what it could not
//! verify without one rather than inventing a number (RFC 13 §3 O4);
//! [`check_storages`] compares a plan against storages somebody else read
//! off the admin space; [`explain`] answers "which storage takes this key"
//! over the plan alone. [`to_json5`] is the one rendering of the plan that
//! is not zenkey's — it is `zenohd`'s.

use std::collections::BTreeMap;

use zenoh::key_expr::keyexpr;

use crate::model::registry::SliceSet;
use crate::report::{
    Asked, CheckFinding, CheckKind, Deployment, GarbageCollection, HistoryMode, Judgement,
    Persistence, PlanWarning, PlannedStorage, PlannedVolume, Refusal, RegistryFacts, Replication,
    StorageCheck, StorageClass, StorageExplain, StorageInfo, StoragePlan, Taker, TakerRelation,
    WarningKind,
};

/// Zenoh's own default `garbage_collection.lifespan`, seconds (RFC 09 §2.3:
/// "default 24 h").
pub const DEFAULT_LIFESPAN_S: i64 = 86_400;
/// Zenoh's own default `garbage_collection.period`, seconds.
pub const DEFAULT_GC_PERIOD_S: u64 = 30;
/// The default margin over the longest covered `ttl_s`.
pub const DEFAULT_GC_MARGIN: f64 = 2.0;

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
/// Zenoh accepts (string prefix, no wildcards). `zensight/v1/*/state/**`
/// → `zensight/v1`; `zensight/v1/@catalog/state/pdns/**` →
/// `zensight/v1/@catalog/state/pdns`. Empty when the expression opens on a
/// wildcard.
pub fn literal_prefix(key_expr: &str) -> String {
    key_expr
        .split('/')
        .take_while(|c| !c.contains('*') && !c.contains('$'))
        .collect::<Vec<_>>()
        .join("/")
}

/// One declared subject, as a wire family under the base.
struct Family {
    producer: String,
    path: String,
    is_state: bool,
    ttl_s: Option<i64>,
    selector: String,
}

/// Every declared subject of every class as the selector it occupies on the
/// wire — the same composition `state_coverage` uses, all classes.
fn families(slices: &SliceSet, base: &str) -> Vec<Family> {
    let mut out = Vec::new();
    for slice in slices.slices() {
        for subject in &slice.subjects {
            let Ok(pattern) = zenkey::pattern::SubjectPattern::parse(&subject.path) else {
                continue;
            };
            let class = subject.class.token();
            let selector = match &slice.service_origin {
                Some(origin) => zenkey::grammar::with_base(
                    base,
                    format!("v1/{origin}/{class}/{}", pattern.selector_tail()),
                ),
                None => zenkey::grammar::with_base(
                    base,
                    format!("v1/*/{class}/{}/{}", slice.name, pattern.selector_tail()),
                ),
            };
            out.push(Family {
                producer: slice.name.clone(),
                path: subject.path.clone(),
                is_state: subject.class.is(&zenkey::Class::State),
                ttl_s: subject.ttl_s,
                selector,
            });
        }
    }
    out
}

/// The longest `ttl_s` among state families, and which one carries it —
/// the alphabetically first of a tie, so the derivation is stable.
fn longest_ttl<'f>(fams: impl Iterator<Item = &'f Family>) -> Option<(i64, String)> {
    fams.filter(|f| f.is_state)
        .filter_map(|f| f.ttl_s.map(|t| (t, format!("{}/{}", f.producer, f.path))))
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)))
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
/// `slices` is the registry when one was asked — `None` degrades every
/// lifespan to RFC 09 §2.3's default and says so in the derivation, and
/// skips the coverage refusal (no registry is not an empty registry, RFC 13
/// §3 O4). `fallback_base` is the observer's resolved base, used when the
/// deployment file names none.
pub fn plan_storages(
    slices: Option<&SliceSet>,
    fallback_base: &str,
    deployment: &Deployment,
) -> StoragePlan {
    let base = deployment
        .base
        .clone()
        .unwrap_or_else(|| fallback_base.to_string());
    let fams: Option<Vec<Family>> = slices.map(|s| families(s, &base));
    let registry = match (slices, &fams) {
        (Some(s), Some(f)) => {
            let longest = longest_ttl(f.iter());
            Asked::Asked(RegistryFacts {
                slices: s.slices().len(),
                max_ttl_s: longest.as_ref().map(|l| l.0),
                ttl_source: longest.map(|l| l.1),
            })
        }
        _ => Asked::NotAsked,
    };

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
        // The selector: one of class and override, never both, never neither.
        let (class, key_expr) = match (spec.class, spec.selector.as_deref()) {
            (Some(c), None) => (Some(c), zenkey::grammar::with_base(&base, c.selector())),
            (None, Some(sel)) => (None, zenkey::grammar::with_base(&base, sel)),
            (Some(_), Some(_)) => {
                refusals.push(refuse_storage(
                    name,
                    None,
                    "RFC 04 §4",
                    "declares both class and selector — the class derives the selector, so \
                     name one or the other",
                ));
                continue;
            }
            (None, None) => {
                refusals.push(refuse_storage(
                    name,
                    None,
                    "RFC 04 §4",
                    "declares neither class nor selector — nothing says what it stores",
                ));
                continue;
            }
        };
        let Ok(ke) = keyexpr::new(key_expr.as_str()) else {
            refusals.push(refuse_storage(
                name,
                Some(key_expr.clone()),
                "RFC 03 §2",
                format!("{key_expr:?} is not a valid key expression"),
            ));
            continue;
        };

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

        // Coverage: what the registry declares under this selector.
        let covered: Option<Vec<&Family>> = fams.as_ref().map(|f| {
            f.iter()
                .filter(|fam| keyexpr::new(fam.selector.as_str()).is_ok_and(|fk| ke.intersects(fk)))
                .collect()
        });
        if let Some(c) = &covered
            && c.is_empty()
        {
            let reason = format!(
                "the registry declares no subject under {key_expr:?} — empty coverage is \
                 a finding, not a plan (a storage that captures nothing is either a \
                 typo or a registry gap; either way it is not this deployment's)"
            );
            refusals.push(refuse_storage(name, Some(key_expr), "RFC 13 §3", reason));
            continue;
        }

        // The tombstone lifetime, derived (RFC 09 §2.3).
        let margin = spec.gc_margin.unwrap_or(DEFAULT_GC_MARGIN);
        let longest = covered
            .as_ref()
            .and_then(|c| longest_ttl(c.iter().copied()));
        let (lifespan_s, derivation) = match (spec.gc_lifespan_s, &covered, &longest) {
            (Some(explicit), _, Some((ttl, src))) => {
                if explicit < *ttl {
                    warnings.push(warn(
                        WarningKind::LifespanBelowTtl,
                        "RFC 09 §2.3",
                        format!(
                            "gc_lifespan_s {explicit} is below the longest covered ttl_s \
                             {ttl} ({src}): a delete must stay observable ≥ ttl_s (RFC 04 \
                             §1.2), else a slow replica may resurrect a retired key"
                        ),
                    ));
                }
                (
                    explicit,
                    format!(
                        "declared gc_lifespan_s {explicit} (longest covered ttl_s {ttl}, {src})"
                    ),
                )
            }
            (Some(explicit), Some(_), None) => (
                explicit,
                format!("declared gc_lifespan_s {explicit} (no state subject under this selector)"),
            ),
            (Some(explicit), None, _) => (
                explicit,
                format!("declared gc_lifespan_s {explicit} (no registry: unverified)"),
            ),
            (None, Some(_), Some((ttl, src))) => {
                let lifespan = (*ttl as f64 * margin).ceil() as i64;
                (
                    lifespan,
                    format!("max ttl_s {ttl} ({src}) × {margin:?} = {lifespan} s"),
                )
            }
            (None, Some(_), None) => (
                DEFAULT_LIFESPAN_S,
                format!(
                    "no state subject under this selector: RFC 09 §2.3 default \
                     {DEFAULT_LIFESPAN_S} s"
                ),
            ),
            (None, None, _) => (
                DEFAULT_LIFESPAN_S,
                format!("no registry: RFC 09 §2.3 default {DEFAULT_LIFESPAN_S} s, unverified"),
            ),
        };

        // `complete: true` — right in exactly one place (RFC 09 §2.2).
        let mut complete = spec.complete;
        if complete
            && !(replication.is_some()
                && history == HistoryMode::Latest
                && class == Some(StorageClass::State))
        {
            complete = false;
            let why = if class != Some(StorageClass::State) {
                "it is not the fully covering latest storage (class state)"
            } else if history != HistoryMode::Latest {
                "its volume is not latest-mode"
            } else {
                "it is not replicated"
            };
            warnings.push(warn(
                WarningKind::CompleteRefused,
                "RFC 09 §2.2",
                format!(
                    "complete = true refused: {why} — complete is right only on a \
                     replicated, fully covering latest storage, where it lets the router \
                     answer any state GET from the nearest replica; emitted as false"
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
            "memory" if class.is_some_and(StorageClass::seeds) => warnings.push(warn(
                WarningKind::VolatileSeed,
                "RFC 09 §2.1",
                "a volatile volume under a seed-bearing class: gone on router restart, \
                 so late joiners lose their seed until state refreshes",
            )),
            _ => {}
        }

        storages.push(PlannedStorage {
            name: name.clone(),
            class,
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
            covers: match &covered {
                Some(c) => Asked::Asked(c.len()),
                None => Asked::NotAsked,
            },
            warnings,
        });
    }

    // ── Overlaps (RFC 09 §2's documented one is catalog vs pdns_history). ──
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
        registry,
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
    let _ = write!(out, "// base {:?}; ", plan.base);
    match plan.registry.as_option() {
        Some(r) => {
            let _ = write!(out, "registry: {} slice(s)", r.slices);
            match (&r.max_ttl_s, &r.ttl_source) {
                (Some(t), Some(src)) => {
                    let _ = write!(out, ", longest state ttl_s {t} ({src})");
                }
                _ => {
                    let _ = write!(out, ", no state subject declared");
                }
            }
            let _ = writeln!(out, ".");
        }
        None => {
            let _ = writeln!(
                out,
                "registry: not asked — every lifespan below is RFC 09 §2.3's default \
                 {DEFAULT_LIFESPAN_S} s, unverified against any ttl_s."
            );
        }
    }
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
        let what = match s.class {
            Some(c) => format!("class {}", c.as_str()),
            None => "selector override".to_string(),
        };
        let covers = match s.covers.as_option() {
            Some(n) => format!("; {n} declared subject(s) under it"),
            None => String::new(),
        };
        let _ = writeln!(out, "      // {}: {what}{covers}", s.name);
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
                "        complete: true,  // replicated, fully covering latest storage (RFC 09 §2.2)"
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

/// Compare a plan against the storages a router admits to running (#393) —
/// the configuration half of `storage list`'s coverage question.
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
        let basis = match s.class {
            Some(c) => format!("class {} under base {:?}", c.as_str(), plan.base),
            None => format!("a selector override under base {:?}", plan.base),
        };
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
            class: s.class,
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
        let at_origin = zenkey::grammar::strip_base(&plan.base, key)
            .and_then(|rel| rel.split('/').nth(1).map(|c| c.starts_with('@')))
            .unwrap_or(false);
        let mut reason = String::from("no planned storage's selector includes it");
        if at_origin {
            reason.push_str(
                " — its origin is an @-chunk, and `*` never matches one: a service's state \
                 needs its own explicit storage (RFC 03 §4 D4)",
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

    fn fixture_registry() -> SliceSet {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixture-tests/registry");
        SliceSet::from_dirs(&[dir]).expect("the fixture registry reads")
    }

    fn volume(plugin: &str, history: Option<HistoryMode>) -> VolumeSpec {
        VolumeSpec {
            plugin: plugin.into(),
            history,
            params: BTreeMap::new(),
        }
    }

    fn storage(class: StorageClass, volume: &str) -> StorageSpec {
        StorageSpec {
            class: Some(class),
            volume: volume.into(),
            ..Default::default()
        }
    }

    /// The RFC 09 §2 sketch, as a deployment.
    fn reference() -> Deployment {
        let mut d = Deployment {
            base: Some("zensight".into()),
            ..Default::default()
        };
        d.volumes.insert("fs".into(), volume("fs", None));
        d.volumes
            .insert("influxdb".into(), volume("influxdb", None));
        d.storages.insert("latest".into(), {
            let mut s = storage(StorageClass::State, "fs");
            s.replication = Replication::Enabled(true);
            s.complete = true;
            s
        });
        d.storages.insert(
            "timeseries".into(),
            storage(StorageClass::Telemetry, "influxdb"),
        );
        d.storages
            .insert("catalog".into(), storage(StorageClass::Catalog, "fs"));
        d.storages.insert(
            "pdns_history".into(),
            storage(StorageClass::CatalogPdns, "influxdb"),
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
        assert_eq!(literal_prefix("zensight/v1/*/state/**"), "zensight/v1");
        assert_eq!(
            literal_prefix("zensight/v1/@catalog/state/pdns/**"),
            "zensight/v1/@catalog/state/pdns"
        );
        assert_eq!(literal_prefix("v1/*/state/**"), "v1");
        assert_eq!(literal_prefix("**"), "");
        assert_eq!(literal_prefix("a/b$*/c"), "a");
    }

    /// The headline: lifespans come from the registry, per storage, with the
    /// computation shown — and `strip_prefix` is derived.
    #[test]
    fn lifespans_are_derived_from_the_registry_and_shown() {
        let plan = plan_storages(Some(&fixture_registry()), "", &reference());
        assert!(plan.refusals.is_empty(), "{:?}", plan.refusals);
        let facts = plan.registry.as_option().expect("asked");
        assert_eq!(facts.max_ttl_s, Some(31_536_000));
        assert_eq!(facts.ttl_source.as_deref(), Some("catalog/alias/{old_id}"));

        let latest = by_name(&plan, "latest");
        assert_eq!(latest.key_expr, "zensight/v1/*/state/**");
        assert_eq!(latest.strip_prefix, "zensight/v1");
        assert_eq!(latest.garbage_collection.lifespan_s, 1800);
        assert_eq!(
            latest.garbage_collection.derivation,
            // Several subjects tie at 900; the alphabetically first names it.
            "max ttl_s 900 (gnmi/artifact/{kind}) × 2.0 = 1800 s"
        );
        assert!(latest.complete, "replicated, latest-mode, class state");
        assert!(latest.replication.is_some());

        let catalog = by_name(&plan, "catalog");
        assert_eq!(catalog.strip_prefix, "zensight/v1/@catalog/state");
        assert_eq!(catalog.garbage_collection.lifespan_s, 63_072_000);

        let pdns = by_name(&plan, "pdns_history");
        assert_eq!(pdns.strip_prefix, "zensight/v1/@catalog/state/pdns");
        assert_eq!(pdns.garbage_collection.lifespan_s, 172_800);
        assert!(
            pdns.garbage_collection
                .derivation
                .contains("catalog/pdns/{ip_slug}")
        );

        let ts = by_name(&plan, "timeseries");
        assert_eq!(ts.garbage_collection.lifespan_s, DEFAULT_LIFESPAN_S);
        assert!(
            ts.garbage_collection
                .derivation
                .contains("no state subject")
        );
        assert!(
            ts.warnings
                .iter()
                .any(|w| w.kind == WarningKind::RetentionIsTheDatabases && w.cite == "RFC 09 §2.3")
        );
    }

    /// The documented overlap, warned on both sides; and `*` versus
    /// `@catalog` is *not* one.
    #[test]
    fn catalog_and_pdns_history_overlap_and_state_does_not() {
        let plan = plan_storages(Some(&fixture_registry()), "", &reference());
        let overlaps = |name: &str| -> Vec<String> {
            by_name(&plan, name)
                .warnings
                .iter()
                .filter(|w| w.kind == WarningKind::Overlap)
                .map(|w| w.text.clone())
                .collect()
        };
        assert!(overlaps("catalog")[0].contains("overlaps pdns_history"));
        assert!(overlaps("pdns_history")[0].contains("overlaps catalog"));
        assert!(overlaps("latest").is_empty(), "`*` never matches @catalog");
    }

    /// RFC 09 §2.2: replication on an all-mode volume is a startup refusal,
    /// so the plan refuses first — and names the storage.
    #[test]
    fn replication_on_an_all_mode_volume_is_refused() {
        let mut d = reference();
        d.storages.get_mut("timeseries").unwrap().replication = Replication::Enabled(true);
        let plan = plan_storages(Some(&fixture_registry()), "", &d);
        assert!(plan.storages.iter().all(|s| s.name != "timeseries"));
        let r = plan
            .refusals
            .iter()
            .find(|r| r.storage.as_deref() == Some("timeseries"))
            .expect("refused");
        assert_eq!(r.cite, "RFC 09 §2.2");
        assert_eq!(r.key_expr.as_deref(), Some("zensight/v1/*/telemetry/**"));
    }

    /// `complete = true` anywhere but the one right place is emitted as
    /// `false` and said so.
    #[test]
    fn complete_is_refused_off_the_replicated_latest_storage() {
        let mut d = reference();
        d.storages.get_mut("catalog").unwrap().complete = true;
        d.storages.get_mut("latest").unwrap().replication = Replication::Enabled(false);
        let plan = plan_storages(Some(&fixture_registry()), "", &d);
        for name in ["catalog", "latest"] {
            let s = by_name(&plan, name);
            assert!(!s.complete);
            assert!(
                s.warnings
                    .iter()
                    .any(|w| w.kind == WarningKind::CompleteRefused && w.cite == "RFC 09 §2.2"),
                "{name}: {:?}",
                s.warnings
            );
        }
    }

    /// The other refusals: an undeclared volume, a class the registry has no
    /// subject for, a redb volume without a mode.
    #[test]
    fn undeclared_volumes_and_empty_coverage_are_refused() {
        let mut d = reference();
        d.storages
            .insert("events".into(), storage(StorageClass::Events, "influxdb"));
        d.storages
            .insert("stray".into(), storage(StorageClass::State, "nope"));
        d.volumes.insert("redb".into(), volume("redb", None));
        let plan = plan_storages(Some(&fixture_registry()), "", &d);
        let refused = |name: &str| {
            plan.refusals
                .iter()
                .find(|r| r.storage.as_deref() == Some(name) || r.volume.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name} refused: {:?}", plan.refusals))
        };
        assert!(refused("events").reason.contains("declares no subject"));
        assert!(refused("stray").reason.contains("does not declare"));
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
            storage(StorageClass::Telemetry, "redb-history"),
        );
        d.storages.insert("catalog".into(), {
            let mut s = storage(StorageClass::Catalog, "redb");
            s.retention = Some(serde_json::json!({"max_age_s": 60}));
            s
        });
        let plan = plan_storages(Some(&fixture_registry()), "", &d);
        assert!(
            by_name(&plan, "timeseries")
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::RetentionRequired)
        );
        assert!(
            by_name(&plan, "catalog")
                .warnings
                .iter()
                .any(|w| w.kind == WarningKind::RetentionPointless)
        );
        // And an `fs` volume declared all-mode is a refused volume, taking
        // its storages with it.
        d.volumes
            .insert("fs".into(), volume("fs", Some(HistoryMode::All)));
        let plan = plan_storages(Some(&fixture_registry()), "", &d);
        assert!(
            plan.refusals
                .iter()
                .any(|r| r.volume.as_deref() == Some("fs"))
        );
        assert!(
            plan.refusals
                .iter()
                .any(|r| r.storage.as_deref() == Some("latest") && r.reason.contains("refused"))
        );
    }

    /// No registry: the default lifespan, said to be unverified, and no
    /// coverage refusal — not asked is not empty (RFC 13 §3 O4).
    #[test]
    fn without_a_registry_the_plan_degrades_and_says_so() {
        let mut d = reference();
        d.storages
            .insert("events".into(), storage(StorageClass::Events, "influxdb"));
        let plan = plan_storages(None, "", &d);
        assert!(plan.registry.is_not_asked());
        assert!(plan.refusals.is_empty());
        let latest = by_name(&plan, "latest");
        assert_eq!(latest.garbage_collection.lifespan_s, DEFAULT_LIFESPAN_S);
        assert_eq!(
            latest.garbage_collection.derivation,
            "no registry: RFC 09 §2.3 default 86400 s, unverified"
        );
        assert!(latest.covers.is_not_asked());
        assert!(to_json5(&plan).contains("registry: not asked"));
    }

    /// The base falls back to the observer's when the file names none; an
    /// empty base composes to the bus-root deployment.
    #[test]
    fn the_base_falls_back_and_the_empty_base_is_the_identity() {
        let mut d = reference();
        d.base = None;
        let plan = plan_storages(None, "", &d);
        assert_eq!(by_name(&plan, "latest").key_expr, "v1/*/state/**");
        assert_eq!(by_name(&plan, "latest").strip_prefix, "v1");
        let plan = plan_storages(None, "acme", &d);
        assert_eq!(by_name(&plan, "latest").key_expr, "acme/v1/*/state/**");
    }

    /// The JSON5 is the RFC 09 §2 sketch's shape, with the numbers filled in
    /// and the caveats beside the storage they concern.
    #[test]
    fn the_json5_carries_the_block_and_its_comments() {
        let plan = plan_storages(Some(&fixture_registry()), "", &reference());
        let doc = to_json5(&plan);
        assert!(doc.contains("plugins: {\n  storage_manager: {\n    volumes: {"));
        assert!(doc.contains("      fs: {},  // durable · latest (RFC 09 §2.1)"));
        assert!(doc.contains("        key_expr: \"zensight/v1/*/state/**\","));
        assert!(doc.contains("        strip_prefix: \"zensight/v1\","));
        assert!(
            doc.contains("garbage_collection: { period: 30, lifespan: 1800 },  // max ttl_s 900")
        );
        assert!(doc.contains("replication: { hot: 6, interval: 10.0, propagation_delay: 250, sub_intervals: 5, warm: 30 }"));
        assert!(doc.contains("        complete: true,"));
        assert!(doc.contains("      // ! overlap: overlaps pdns_history"));
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
            d.storages
                .insert("events".into(), storage(StorageClass::Events, "influxdb"));
            to_json5(&plan_storages(Some(&fixture_registry()), "", &d))
        };
        assert!(refused.contains("      // REFUSED storage events:"));
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
        let plan = plan_storages(Some(&fixture_registry()), "", &reference());

        let empty = check_storages(&plan, &[]);
        assert!(empty.judgement.is_unobservable());
        assert_eq!(crate::judgement_exit_code(&empty.judgement), 2);

        let clean = vec![
            observed(
                "latest",
                "zensight/v1/*/state/**",
                "zensight/v1",
                "fs",
                Some(1800),
            ),
            observed(
                "timeseries",
                "zensight/v1/*/telemetry/**",
                "zensight/v1",
                "influxdb",
                Some(86400),
            ),
            observed(
                "catalog",
                "zensight/v1/@catalog/state/**",
                "zensight/v1/@catalog/state",
                "fs",
                Some(63_072_000),
            ),
            observed(
                "pdns_history",
                "zensight/v1/@catalog/state/pdns/**",
                "zensight/v1/@catalog/state/pdns",
                "influxdb",
                Some(172_800),
            ),
        ];
        let c = check_storages(&plan, &clean);
        assert!(c.findings.is_empty(), "{:?}", c.findings);
        assert_eq!(crate::judgement_exit_code(&c.judgement), 0);
        assert_eq!(c.asked, CHECK_ASKED);

        let drifted = vec![
            // Too short a lifespan, and the wrong prefix.
            observed(
                "latest",
                "zensight/v1/*/state/**",
                "zensight",
                "fs",
                Some(600),
            ),
            // Wrong selector, wrong volume.
            observed(
                "timeseries",
                "zensight/v1/**/telemetry/**",
                "zensight/v1",
                "memory",
                Some(86400),
            ),
            // Layout omits the gc block.
            observed(
                "catalog",
                "zensight/v1/@catalog/state/**",
                "zensight/v1/@catalog/state",
                "fs",
                None,
            ),
            // Not planned at all.
            observed("blobs", "zensight/v1/*/@blob/**", "zensight/v1", "fs", None),
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
                (CheckKind::StripPrefixDiffers, "latest"),
                (CheckKind::LifespanBelowMinimum, "latest"),
                (CheckKind::Missing, "pdns_history"),
                (CheckKind::KeyExprDiffers, "timeseries"),
                (CheckKind::VolumeDiffers, "timeseries"),
                (CheckKind::Extra, "blobs"),
            ]
        );
        assert_eq!(
            c.unjudged,
            ["catalog@aabbccdd: the admin document does not carry garbage_collection.lifespan"]
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

    /// `--explain`: the taker and its reason; the @-origin key nobody takes;
    /// the key a refused storage would have taken.
    #[test]
    fn explain_names_the_taker_or_the_reason_there_is_none() {
        let mut d = reference();
        d.storages.remove("catalog");
        d.storages
            .insert("events".into(), storage(StorageClass::Events, "influxdb"));
        let plan = plan_storages(Some(&fixture_registry()), "", &d);

        let e = explain(&plan, "zensight/v1/h-3fa9c2d41b7e/state/sysinfo/health");
        assert_eq!(e.takers.len(), 1);
        assert_eq!(e.takers[0].storage, "latest");
        assert_eq!(e.takers[0].relation, TakerRelation::Includes);
        assert!(
            e.takers[0]
                .why
                .contains("class state under base \"zensight\"")
        );
        assert!(e.none_reason.is_none());

        let e = explain(&plan, "zensight/v1/@catalog/state/entity/x");
        assert!(e.takers.is_empty());
        assert!(e.none_reason.as_deref().unwrap().contains("RFC 03 §4 D4"));

        let e = explain(
            &plan,
            "zensight/v1/h-3fa9c2d41b7e/events/netring/capture/01J",
        );
        assert!(e.takers.is_empty());
        assert_eq!(e.refused_takers, ["events"]);
        assert!(
            e.none_reason
                .as_deref()
                .unwrap()
                .contains("refused storage(s) events would have")
        );

        let e = explain(&plan, "zensight/v1/*/state/**");
        assert_eq!(e.takers[0].relation, TakerRelation::Includes);
        let e = explain(&plan, "zensight/v1/**");
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
