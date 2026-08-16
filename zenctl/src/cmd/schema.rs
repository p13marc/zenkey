//! `zenctl schema <producer>` and `interface show --schema` (issue #51) —
//! the served payload shapes, shown.
//!
//! zenctl's README used to *decline* to show schemas ("maps the type
//! vocabulary rather than pretending to reproduce the shapes"). That stance
//! predates RFC 08 §7: since `describe` and `SchemaStore` shipped, the shapes
//! are **served data**, not something a tool would be inventing. Refusing to
//! print them only sent people to `curl`.
//!
//! Two honesty rules the whole command hangs on:
//!
//! - a producer serving no `describe` is a *degradation*, not an error — §7 is
//!   a SHOULD, and silence about a type is not a claim about it;
//! - the same type name served with different hashes is RFC 08 §7's **drift**
//!   finding, and `interface show --schema` is where a user is already looking
//!   at that type, so it surfaces there rather than only in `doctor`.

use anyhow::Result;

use crate::{BusArgs, output};

/// `zenctl schema <producer> [--type X] [--full]`.
pub async fn dump(
    producer: &str,
    type_filter: Option<&str>,
    full: bool,
    args: &BusArgs,
) -> Result<()> {
    let session = args.session().await?;
    let slices = args.slice_set().await.unwrap_or_default();
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let report =
        zenkey_fleet::schema_dump(&store, &session, &slices, producer, type_filter, full).await;
    output::schema_dump(&report, args.format)
}

/// `zenctl schema check` (#159): one payload against one schema, exit-coded
/// for CI. 0 = valid; 1 = the payload does not conform (schema violations,
/// or bytes that do not decode as the kind at all); 2 = could not check
/// (no schema found, unknown kind) — "could not check" must never exit like
/// either verdict, for the same reason `cutover` reserves its 2.
///
/// This checks; it never publishes and never encodes.
pub async fn check(
    type_name: &str,
    from: &str,
    producer: Option<&str>,
    schema_set: Option<&std::path::Path>,
    encoding: Option<&str>,
    args: &BusArgs,
) -> Result<()> {
    use zenkey::schema::WireEncoding;
    use zenkey_fleet::Verdict;

    let bytes = match from {
        "-" => {
            use std::io::Read as _;
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
        b => match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        },
    };

    // Offline (--schema-set) needs no session at all — that is the whole
    // point: an app repo checks its golden payloads in CI with no bus.
    let schema = match (schema_set, producer) {
        (Some(path), _) => {
            let text = std::fs::read_to_string(path)?;
            let set = zenkey::schema::SchemaSet::parse(&text).map_err(|e| {
                anyhow::anyhow!("{}: not a SchemaSet document: {e}", path.display())
            })?;
            match set.get(type_name) {
                Some(s) => s.clone(),
                None => not_checked(&format!("{} carries no type {type_name:?}", path.display())),
            }
        }
        (None, Some(p)) => {
            let session = args.session().await?;
            let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
            match store.schema_for(&session, p, type_name).await {
                Some(s) => s,
                None => not_checked(&format!(
                    "{p} serves no schema for {type_name:?} (RFC 08 §7 is a SHOULD — \
                     this is silence about the type, not a claim about the payload)"
                )),
            }
        }
        (None, None) => anyhow::bail!(
            "give --producer (live describe) or --schema-set FILE — the registry \
             TOMLs carry type names, not shapes (RFC 08 §7)"
        ),
    };

    let wire = match encoding
        .map(str::to_string)
        .or_else(|| zenkey_fleet::encode_encoding(None, None, Some(&schema)))
    {
        Some(name) => WireEncoding::from_encoding_str(&name),
        None => not_checked(&format!(
            "schema kind {:?} has no known framing — pass --encoding",
            schema.kind().as_str()
        )),
    };

    // A session-less store still decodes (it only needs one for fetching).
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let (verdict, detail): (&str, Vec<String>) = match store.decode(&schema, &wire, &bytes) {
        Ok(decoded) => match decoded.verdict {
            Verdict::Valid => ("valid", decoded.notes),
            Verdict::Invalid(errors) => ("invalid", errors),
            Verdict::NotValidated(reason) => not_checked(&reason.to_string()),
        },
        Err(e) => match &e {
            zenkey::schema::decode::DecodeError::Malformed(..)
            | zenkey::schema::decode::DecodeError::WrongEncoding(_) => {
                ("undecodable", vec![e.to_string()])
            }
            _ => not_checked(&e.to_string()),
        },
    };

    if matches!(
        args.format.resolved(),
        crate::output::Format::Json | crate::output::Format::Ndjson
    ) {
        let obj = serde_json::json!({
            "type": type_name,
            "kind": schema.kind().as_str(),
            "verdict": verdict,
            "detail": detail,
        });
        println!("{obj}");
    } else {
        println!("{type_name} ({}): {verdict}", schema.kind().as_str());
        for line in &detail {
            println!("  {line}");
        }
    }
    if verdict != "valid" {
        std::process::exit(1);
    }
    Ok(())
}

/// Exit 2: the check never happened — reserved so CI can tell "nonconformant"
/// from "unobservable", the same split `cutover` and `probe` guard.
fn not_checked(reason: &str) -> ! {
    eprintln!("not checked: {reason}");
    std::process::exit(2);
}

/// The producers that carry a type name, from the loaded slices — who to ask
/// for its schema. A type carried nowhere is asked of nobody, which is why
/// `interface show` refuses an unknown name before this runs.
pub fn carriers_of(slices: &[zenkey::slice::RegistrySlice], type_name: &str) -> Vec<String> {
    let mut out: Vec<String> = slices
        .iter()
        .filter(|s| {
            s.subjects.iter().any(|d| d.type_name == type_name)
                || s.procedures.iter().any(|p| {
                    p.reply.as_deref() == Some(type_name) || p.request.as_deref() == Some(type_name)
                })
                || s.blob
                    .iter()
                    .any(|b| b.reference.as_deref() == Some(type_name))
        })
        .map(|s| s.name.clone())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenkey::slice::{BlobDecl, ProcedureDecl, RegistrySlice, SubjectDecl};

    fn slice(name: &str, subject_type: &str, reply: Option<&str>) -> RegistrySlice {
        RegistrySlice {
            version: "1.0".into(),
            app: "t".into(),
            convention: 1,
            name: name.into(),
            service_origin: None,
            description: None,
            subjects: vec![SubjectDecl {
                path: "p".into(),
                class: "telemetry".into(),
                type_name: subject_type.into(),
                common: None,
                since: None,
                description: None,
                qos: None,
                ttl_s: None,
                unit: None,
                rate: None,
                cardinality: None,
                encoding: None,
            }],
            procedures: reply
                .map(|r| {
                    vec![ProcedureDecl {
                        path: "proc".into(),
                        kind: "read".into(),
                        reply: Some(r.into()),
                        request: None,
                        encoding: None,
                        fanout: None,
                        idempotent: Some(true),
                        since: None,
                        description: None,
                    }]
                })
                .unwrap_or_default(),
            media: vec![],
            blob: vec![BlobDecl {
                tier: "artifact".into(),
                endpoints: vec![],
                algo: None,
                reference: Some("BlobRef".into()),
                encoding: None,
                since: None,
                description: None,
            }],
            deprecated: vec![],
        }
    }

    /// Every binding site counts as carrying the type — subject, procedure
    /// reply *and* request, and a blob reference. Asking only the producers
    /// that carry it is what keeps `--schema` from fanning out to the fleet.
    #[test]
    fn carriers_cover_every_binding_site() {
        let slices = vec![
            slice("a", "Point", None),
            slice("b", "Other", Some("Point")),
            slice("c", "Other", None),
        ];
        assert_eq!(carriers_of(&slices, "Point"), vec!["a", "b"]);
        assert_eq!(carriers_of(&slices, "BlobRef"), vec!["a", "b", "c"]);
        assert!(carriers_of(&slices, "Nothing").is_empty());
    }
}
