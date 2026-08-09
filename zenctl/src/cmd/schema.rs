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
