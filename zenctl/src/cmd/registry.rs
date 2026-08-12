//! `zenctl registry export|diff|lint` (issue #50) — the registry as a
//! document, as a comparison, and as a checkable artifact.
//!
//! The three sit together because they answer the three questions an operator
//! has about a registry they did not write: *what does it say* (export),
//! *does the fleet agree with the checkout* (diff), and *would this pass a
//! build* (lint).
//!
//! `lint` deliberately runs the real thing — `zenkey_build::Config::lint`, the
//! same lints a consumer's `build.rs` runs, stopping where a build would. A
//! lint that reported more than the build does would be a different tool
//! wearing the same name.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use zenkey::RegistrySlice;

use crate::{BusArgs, output};

/// What `registry export` emits.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ExportAs {
    /// Registry TOML — round-trippable through `SliceSet::from_dirs`.
    Toml,
    /// A JSON Schema bundle built from the producers' served `describe`
    /// replies (RFC 08 §7).
    Jsonschema,
    /// An AsyncAPI 3.0 document: channels from subjects, operations from
    /// procedures.
    Asyncapi,
}

pub async fn export(target: ExportAs, producer: Option<&str>, args: &BusArgs) -> Result<()> {
    let slices = args.slice_set().await?;
    let selected: Vec<&RegistrySlice> = slices
        .slices()
        .iter()
        .filter(|s| producer.is_none_or(|p| p == s.name))
        .collect();
    if selected.is_empty() {
        return Err(anyhow!(
            "no slices to export{} — an empty set is not a verdict (RFC 05 §3.1); \
             try --registry <dir> or check --base",
            producer
                .map(|p| format!(" for producer {p:?}"))
                .unwrap_or_default()
        ));
    }

    match target {
        ExportAs::Toml => {
            // One document per producer, separated by the file boundary a
            // registry dir would have. A single concatenated TOML would not
            // re-parse: the format is one slice per file by construction.
            for (i, slice) in selected.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                println!("# ── {}.toml ─────────────────────────────", slice.name);
                print!("{}", zenkey::slice_to_toml(slice));
            }
        }
        ExportAs::Jsonschema => {
            let session = args.session().await?;
            let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
            let mut defs = serde_json::Map::new();
            let mut undescribed: Vec<String> = Vec::new();
            for slice in &selected {
                match store.set_for(&session, &slice.name).await {
                    Some(set) => {
                        for (name, schema) in set.iter() {
                            match schema.json_document() {
                                Some(doc) => {
                                    defs.insert(name.to_string(), doc.clone());
                                }
                                // A non-`json-schema` kind is recorded as what
                                // it is rather than translated into a JSON
                                // Schema it is not (O4 for a document).
                                None => {
                                    defs.insert(
                                        name.to_string(),
                                        serde_json::json!({
                                            "$comment": format!(
                                                "served as kind {:?} ({}), which this bundle \
                                                 does not translate",
                                                schema.kind().as_str(),
                                                schema.hash()
                                            ),
                                        }),
                                    );
                                }
                            }
                        }
                    }
                    None => undescribed.push(slice.name.clone()),
                }
            }
            let mut doc = serde_json::Map::new();
            doc.insert(
                "$schema".into(),
                serde_json::Value::String("https://json-schema.org/draft/2020-12/schema".into()),
            );
            doc.insert("$defs".into(), serde_json::Value::Object(defs));
            if !undescribed.is_empty() {
                doc.insert(
                    "$comment".into(),
                    serde_json::Value::String(format!(
                        "producers serving no `describe`, therefore absent here: {}",
                        undescribed.join(", ")
                    )),
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::Value::Object(doc))?
            );
        }
        ExportAs::Asyncapi => {
            println!("{}", serde_json::to_string_pretty(&asyncapi(&selected))?);
        }
    }
    Ok(())
}

/// An AsyncAPI 3.0 document from the slice set — the prior-art direction
/// RFC 10 records, made concrete.
///
/// The mapping is deliberately literal: a subject is a channel whose address
/// is its **key pattern**, and a procedure is an operation. Nothing is
/// invented — an undeclared field is absent, not defaulted, and the `x-zenkey`
/// extensions carry the convention's own vocabulary rather than bending it
/// into AsyncAPI words that mean something else.
fn asyncapi(slices: &[&RegistrySlice]) -> serde_json::Value {
    let mut channels = serde_json::Map::new();
    let mut operations = serde_json::Map::new();

    for slice in slices {
        let origin = slice.service_origin.as_deref().unwrap_or("{origin}");
        for d in &slice.subjects {
            let address = match &slice.service_origin {
                // A service origin has no producer chunk (RFC 06 §5).
                Some(_) => format!("v1/{origin}/{}/{}", d.class, d.path),
                None => format!("v1/{origin}/{}/{}/{}", d.class, slice.name, d.path),
            };
            let id = format!("{}.{}", slice.name, d.path.replace('/', "."));
            let mut channel = serde_json::Map::new();
            channel.insert("address".into(), serde_json::Value::String(address));
            if let Some(desc) = &d.description {
                channel.insert(
                    "description".into(),
                    serde_json::Value::String(desc.clone()),
                );
            }
            if !d.type_name.is_empty() {
                channel.insert(
                    "messages".into(),
                    serde_json::json!({
                        d.type_name.clone(): { "name": d.type_name.clone() }
                    }),
                );
            }
            let mut ext = serde_json::Map::new();
            ext.insert("class".into(), serde_json::Value::String(d.class.clone()));
            for (k, v) in [
                ("qos", d.qos.clone()),
                ("unit", d.unit.clone()),
                ("rate", d.rate.clone()),
                ("encoding", d.encoding.clone()),
                ("since", d.since.clone()),
            ] {
                if let Some(v) = v {
                    ext.insert(k.into(), serde_json::Value::String(v));
                }
            }
            if let Some(t) = d.ttl_s {
                ext.insert("ttl_s".into(), serde_json::Value::from(t));
            }
            if let Some(c) = d.cardinality {
                ext.insert("cardinality".into(), serde_json::Value::from(c));
            }
            channel.insert("x-zenkey".into(), serde_json::Value::Object(ext));
            channels.insert(id, serde_json::Value::Object(channel));
        }

        for p in &slice.procedures {
            let address = match &slice.service_origin {
                Some(_) => format!("v1/{origin}/@rpc/{}", p.path),
                None => format!("v1/{origin}/@rpc/{}/{}", slice.name, p.path),
            };
            let id = format!("{}.@rpc.{}", slice.name, p.path.replace('/', "."));
            channels.insert(
                id.clone(),
                serde_json::json!({
                    "address": address,
                    "x-zenkey": { "plane": "@rpc", "kind": p.kind },
                }),
            );
            let mut op = serde_json::Map::new();
            // RFC 05: a `read` procedure is a GET the caller sends, a `write`
            // likewise — both are `send` from the explorer's side.
            op.insert("action".into(), serde_json::Value::String("send".into()));
            op.insert(
                "channel".into(),
                serde_json::json!({ "$ref": format!("#/channels/{id}") }),
            );
            if let Some(desc) = &p.description {
                op.insert(
                    "description".into(),
                    serde_json::Value::String(desc.clone()),
                );
            }
            let mut ext = serde_json::Map::new();
            ext.insert("kind".into(), serde_json::Value::String(p.kind.clone()));
            for (k, v) in [
                ("request", p.request.clone()),
                ("reply", p.reply.clone()),
                ("fanout", p.fanout.clone()),
                ("encoding", p.encoding.clone()),
                ("since", p.since.clone()),
            ] {
                if let Some(v) = v {
                    ext.insert(k.into(), serde_json::Value::String(v));
                }
            }
            if let Some(i) = p.idempotent {
                ext.insert("idempotent".into(), serde_json::Value::Bool(i));
            }
            op.insert("x-zenkey".into(), serde_json::Value::Object(ext));
            operations.insert(
                format!("{}.{}", slice.name, p.path.replace('/', ".")),
                serde_json::Value::Object(op),
            );
        }
    }

    let app = slices.first().map(|s| s.app.as_str()).unwrap_or("zenkey");
    serde_json::json!({
        "asyncapi": "3.0.0",
        "info": {
            "title": format!("{app} — keyspace-v2 registry"),
            "version": slices.first().map(|s| s.version.clone()).unwrap_or_default(),
            "description":
                "Generated by `zenctl registry export --as asyncapi` from RFC 08 registry \
                 slices. Channel addresses are base-relative key patterns; `{origin}` is the \
                 publishing identity (RFC 06 §1), not an AsyncAPI parameter.",
        },
        "channels": channels,
        "operations": operations,
    })
}

/// `registry diff` — local dirs against the live bus, per producer.
pub async fn diff(args: &BusArgs) -> Result<()> {
    let dirs = args.registry_dirs();
    if dirs.is_empty() {
        return Err(anyhow!(
            "registry diff compares local files against the bus — pass --registry <dir> \
             (or set one on the active context)"
        ));
    }
    let local = zenkey_fleet::SliceSet::from_dirs(&dirs)?;
    let session = args.session().await?;
    let served = zenkey_fleet::SliceSet::from_bus(&session, args.base(), args.timeout()).await?;
    let report = build_diff(&served, &local);
    output::registry_diff(&report, args.format)
}

/// Pure half, so the comparison is testable without a bus.
pub fn build_diff(
    served: &zenkey_fleet::SliceSet,
    local: &zenkey_fleet::SliceSet,
) -> crate::report::RegistryDiff {
    use crate::report::{ProducerDiff, RegistryDiff};
    let mut names: Vec<&str> = served
        .slices()
        .iter()
        .chain(local.slices())
        .map(|s| s.name.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();

    let mut producers = Vec::new();
    for name in names {
        let s = served.get(name);
        let l = local.get(name);
        producers.push(match (s, l) {
            (Some(s), Some(l)) => ProducerDiff {
                producer: name.to_string(),
                served_version: Some(s.version.clone()),
                local_version: Some(l.version.clone()),
                findings: zenkey::slice::diff(s, l)
                    .iter()
                    .map(|f| f.summary())
                    .collect(),
            },
            // Present on one side only. Neither is an error: a producer the
            // bus serves and the checkout does not know may simply be newer,
            // and one the checkout declares that nothing serves may simply be
            // down (RFC 05 §3.1 — silence is not a verdict).
            (Some(s), None) => ProducerDiff {
                producer: name.to_string(),
                served_version: Some(s.version.clone()),
                local_version: None,
                findings: vec!["served by the fleet, absent from the local registry".into()],
            },
            (None, Some(l)) => ProducerDiff {
                producer: name.to_string(),
                served_version: None,
                local_version: Some(l.version.clone()),
                findings: vec![
                    "declared locally, not served by any origin — down, or not deployed \
                     (silence is not a verdict, RFC 05 §3.1)"
                        .into(),
                ],
            },
            (None, None) => unreachable!("name came from one of the two sets"),
        });
    }
    RegistryDiff { producers }
}

/// `registry lint <dir>` — the consumer's build lints, without the build.
pub fn lint(dir: &Path, ledger: Option<&PathBuf>) -> Result<()> {
    let mut config = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed();
    if let Some(l) = ledger {
        config = config.ledger(l);
    }
    match config.lint() {
        Ok(()) => {
            println!("{}: registry lints pass (RFC 08 §5).", dir.display());
            Ok(())
        }
        // The build's own wording, verbatim — the value of this command is
        // that it says exactly what the build would.
        Err(e) => Err(anyhow!("{e}")),
    }
}

/// `registry lock <dir>` — regenerate the RFC 08 §3.1 compatibility lock.
/// Refuses an incompatible rewrite without `--force`; a forced break prints
/// every broken pin (loud by contract).
pub fn lock(dir: &Path, force: bool) -> Result<()> {
    let update = zenkey_build::Config::new()
        .registry_dir(dir)
        .no_rerun_if_changed()
        .write_compat_lock(force)
        .map_err(|e| anyhow!("{e}"))?;
    println!(
        "{}: {} ({} pinned entr{} added, {} released)",
        update.path.display(),
        if update.created { "created" } else { "updated" },
        update.added,
        if update.added == 1 { "y" } else { "ies" },
        update.retired,
    );
    for broken in &update.forced {
        eprintln!("FORCED BREAK (RFC 08 §3.1):\n{broken}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(toml: &str) -> zenkey_fleet::SliceSet {
        zenkey_fleet::SliceSet::from_slices(vec![zenkey::parse_slice(toml).unwrap()])
    }

    const SERVED: &str = r#"
[registry]
version = "2.0"
app = "t"
convention = 1
[producer]
name = "netring"
[[subject]]
path = "flows"
class = "telemetry"
type = "TelemetryPoint"
[[subject]]
path = "brand/new"
class = "telemetry"
type = "TelemetryPoint"
"#;

    const LOCAL: &str = r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[producer]
name = "netring"
[[subject]]
path = "flows"
class = "telemetry"
type = "TelemetryPoint"
"#;

    /// The diff reports exactly the edited subject, plus the version skew —
    /// #50's acceptance, without a bus.
    #[test]
    fn the_diff_names_the_one_subject_that_moved() {
        let report = build_diff(&set(SERVED), &set(LOCAL));
        assert_eq!(report.producers.len(), 1);
        let p = &report.producers[0];
        assert_eq!(p.served_version.as_deref(), Some("2.0"));
        assert_eq!(p.local_version.as_deref(), Some("1.0"));
        assert!(
            p.findings.iter().any(|f| f.contains("brand/new")),
            "{:?}",
            p.findings
        );
        assert!(
            p.findings.iter().any(|f| f.contains("2.0")),
            "the version skew is a finding too: {:?}",
            p.findings
        );
    }

    /// One-sided presence is a fact with a reason, never an error — and the
    /// two sides read differently.
    #[test]
    fn one_sided_producers_explain_themselves() {
        let empty = zenkey_fleet::SliceSet::from_slices(vec![]);
        let served_only = build_diff(&set(SERVED), &empty);
        assert!(served_only.producers[0].findings[0].contains("absent from the local registry"));
        assert!(served_only.producers[0].local_version.is_none());

        let local_only = build_diff(&empty, &set(LOCAL));
        assert!(local_only.producers[0].findings[0].contains("silence is not a verdict"));
        assert!(local_only.producers[0].served_version.is_none());
    }

    /// The AsyncAPI mapping places a producer chunk for a host producer and
    /// omits it for a service origin (RFC 06 §5) — the one place the mapping
    /// could quietly emit an unreachable address.
    #[test]
    fn asyncapi_addresses_follow_the_grammar() {
        let host = zenkey::parse_slice(SERVED).unwrap();
        let service = zenkey::parse_slice(
            r#"
[registry]
version = "1.0"
app = "t"
convention = 1
[service]
name = "catalog"
origin = "@catalog"
[[subject]]
path = "entity/{id}"
class = "state"
type = "Entity"
"#,
        )
        .unwrap();
        let doc = asyncapi(&[&host, &service]);
        let channels = doc["channels"].as_object().unwrap();
        assert_eq!(
            channels["netring.flows"]["address"],
            "v1/{origin}/telemetry/netring/flows"
        );
        assert_eq!(
            channels["catalog.entity.{id}"]["address"],
            "v1/@catalog/state/entity/{id}"
        );
    }
}
