//! `zenctl schema show <producer>` and `interface show --schema` (issue #51)
//! — the served payload shapes, shown. Plus [`check`](check), which answers
//! under `check schema` (#307) and reads the same served documents.
//!
//! `schema <producer>` used to be its own spelling: a noun that was also a
//! verb, with `schema check` hanging off it and a bare `zenctl schema`
//! exiting **1** through an `anyhow` message where every other missing
//! argument in this tool exits 2. `show` is the verb it always was.
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

use crate::Bus;

/// The verdict verb's name, spelled once (#355) — the dispatcher
/// uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check schema");

/// `zenctl schema show <producer> [--type X] [--full]`.
pub async fn show(cli: crate::cli::SchemaShowArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::SchemaShowArgs {
        producer,
        type_name: type_filter,
        full,
        bus: _,
    } = cli;
    let (producer, type_filter) = (producer.as_str(), type_filter.as_deref());
    let session = args.session().await?;
    // Slices enrich the dump — the *types* come from the producer's served
    // `describe` (`zenkey_fleet::schema_dump`); slices only compute
    // which declared ones are missing. `None` here is the honest degradation
    // (`slices_optional` says so out loud), and it stays `None` into the dump
    // so "totality not checked" cannot render as "nothing missing"
    // (RFC 09 §5.1 O4; #246).
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
    let report = zenkey_fleet::schema_dump(
        &store,
        &session,
        slices.as_ref(),
        producer,
        type_filter,
        full,
    )
    .await;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

/// `zenctl check schema` (#159): one payload against one schema, exit-coded
/// for CI. 0 = valid; 1 = the payload does not conform (schema violations,
/// or bytes that do not decode as the kind at all); 2 = could not check
/// (no schema found, unknown kind) — "could not check" must never exit like
/// either verdict, for the same reason `check cutover` reserves its 2.
///
/// This checks; it never publishes and never encodes.
pub async fn check(cli: crate::cli::CheckSchemaArgs) -> Result<()> {
    let bus = ASKING.ask(Bus::resolve(&cli.bus));
    let args = &bus;
    let crate::cli::CheckSchemaArgs {
        type_name,
        from,
        producer,
        schema_set,
        encoding,
        bus: _,
    } = cli;
    let (type_name, from, producer, schema_set, encoding) = (
        type_name.as_str(),
        &from,
        producer.as_deref(),
        schema_set.as_deref(),
        encoding.as_deref(),
    );
    use zenkey::schema::WireEncoding;
    use zenkey_fleet::Verdict;

    // A payload that cannot be read is *unobservable*, not nonconformant
    // (#244). `?` here would exit 1 — this verb's "does not conform" — and
    // tell CI that a typo'd path is a schema violation.
    let bytes = match from.read() {
        Ok(bytes) => bytes,
        Err(e) => not_checked(&format!("{e:#}")),
    };

    // Offline (--schema-set) needs no session at all — that is the whole
    // point: an app repo checks its golden payloads in CI with no bus.
    // Every schema-acquisition failure below is `not_checked`, never `?`: a
    // `?` exits 1, this verb's "does not conform", and an unreadable file, a
    // dead bus or a missing flag is not a claim about the payload (the same
    // split #244 fixed for the payload itself).
    let schema = match (schema_set, producer) {
        (Some(path), _) => {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => not_checked(&format!("{}: {e}", path.display())),
            };
            let set = match zenkey::schema::SchemaSet::parse(&text) {
                Ok(s) => s,
                Err(e) => not_checked(&format!(
                    "{}: not a SchemaSet document: {e}",
                    path.display()
                )),
            };
            match set.get(type_name) {
                Some(s) => s.clone(),
                None => not_checked(&format!("{} carries no type {type_name:?}", path.display())),
            }
        }
        (None, Some(p)) => {
            let session = match args.session().await {
                Ok(s) => s,
                Err(e) => not_checked(&format!(
                    "no session, so {p}'s served describe is out of reach: {}",
                    crate::errors::without_source_locations(&format!("{e:#}"))
                )),
            };
            let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
            match store.schema_for(&session, p, type_name).await {
                Some(s) => s,
                None => not_checked(&format!(
                    "{p} serves no schema for {type_name:?} (RFC 08 §7 is a SHOULD — \
                     this is silence about the type, not a claim about the payload)"
                )),
            }
        }
        (None, None) => not_checked(
            "give --producer (live describe) or --schema-set FILE — the registry \
             TOMLs carry type names, not shapes (RFC 08 §7)",
        ),
    };

    let wire = match encoding
        .map(str::to_string)
        .or_else(|| zenkey_fleet::encode_encoding(None, None, Some(&schema)))
    {
        Some(name) => WireEncoding::from_encoding_str(&name),
        None => not_checked(&format!(
            "schema kind {:?} has no known framing — pass --encoding",
            schema.kind_str()
        )),
    };

    // A session-less store still decodes (it only needs one for fetching).
    let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
    use crate::render::SchemaCheckVerdict as V;
    let (verdict, detail): (V, Vec<String>) = match store.decode(&schema, &wire, &bytes) {
        Ok(decoded) => match decoded.verdict {
            Verdict::Valid => (V::Valid, decoded.notes),
            Verdict::Invalid(errors) => (V::Invalid, errors),
            Verdict::NotValidated(reason) => not_checked(&reason.to_string()),
        },
        Err(e) => match &e {
            zenkey::schema::decode::DecodeError::Malformed { .. }
            | zenkey::schema::decode::DecodeError::WrongEncoding(_) => {
                (V::Undecodable, vec![e.to_string()])
            }
            _ => not_checked(&e.to_string()),
        },
    };

    let report = crate::render::SchemaCheck {
        type_name: type_name.to_string(),
        kind: schema.kind_str().to_string(),
        verdict,
        detail,
    };
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    // Off a `match` on the verdict, not a string comparison — which is the
    // one thing `crate::exit` exists to stop being spelled twice (#356).
    match report.verdict.exit_code() {
        crate::exit::CLEAN => Ok(()),
        code => std::process::exit(code),
    }
}

/// Exit 2: the check never happened — reserved so CI can tell "nonconformant"
/// from "unobservable", the same split `check cutover` and `check probe`
/// guard (`crate::exit`).
fn not_checked(reason: &str) -> ! {
    // Through the same seam as every other exit-2 (#355): this used to spell
    // the code and the message itself, so it was a fourth statement of a
    // contract `crate::exit` exists to state once.
    ASKING.unobservable(format_args!("not checked: {reason}"))
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
        let mut subject = SubjectDecl::new("p", zenkey::Class::Telemetry);
        subject.type_name = subject_type.into();
        let mut blob = BlobDecl::new(zenkey::BlobTier::Artifact);
        blob.reference = Some("BlobRef".into());
        let mut slice = RegistrySlice::new("1.0", "t", name);
        slice.subjects = vec![subject];
        slice.blob = vec![blob];
        slice.procedures = reply
            .map(|r| {
                let mut p = ProcedureDecl::new("proc");
                p.kind = Some(zenkey::ProcedureKind::Read.into());
                p.reply = Some(r.into());
                p.idempotent = Some(true);
                vec![p]
            })
            .unwrap_or_default();
        slice
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
