//! `zenctl schema show` (#612, FJ4) — one zk2 revision's schema artifacts,
//! as its bundle carries them — and [`check`](check), which answers under
//! `check schema` (#307) and still checks a payload against a v1 producer's
//! served `describe` or a SchemaSet file until FJ8 re-cuts it.
//!
//! v1's `schema show <producer>` read a producer's served `describe`. In zk2
//! the shapes travel *with the contract*: a bundle carries every schema
//! artifact its types live in (spec §9.5), verified against the revision's
//! fingerprint, so a tool that was never compiled against the contract reads
//! them from there — offline from `--contracts`, or retrieved from the
//! revision's holders (spec §8.4).

use anyhow::Result;

use crate::Bus;
use crate::bus::Deployment;
use crate::cmd::zk2;
use crate::exit::unaskable;

/// The verdict verb's name, spelled once (#355) — the dispatcher
/// uses it too.
pub const ASKING: crate::exit::Asking = crate::exit::Asking::new("check schema");

/// `schema show <iface>[@<fingerprint>] [resource] [--full]`.
///
/// No session when `--contracts` settles which revision is meant; exit 2
/// when no revision can be had (`crate::exit::Unanswered`), and when the
/// revision declares no such resource (the input, refused).
pub async fn show(cli: crate::cli::SchemaShowArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let mut session = None;
    let revision = zk2::revision(&dep, &contracts, &cli.target, &mut session).await?;
    let documents = cli.full || cli.resource.is_some();
    let view = revision
        .schema_view(cli.resource.as_deref(), documents)
        .map_err(|e| unaskable!("{e}"))?;
    crate::render::emit_with(&mut std::io::stdout(), &view, dep.format(), dep.color())
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
