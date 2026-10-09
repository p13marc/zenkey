//! `zenctl schema show` (#612, FJ4) — one zk2 revision's schema artifacts,
//! as its bundle carries them — and [`check`](check), which answers under
//! `check schema` (#307): a payload against one type of a revision (FJ8b).
//!
//! v1's `schema show <producer>` read a producer's served `describe`. In zk2
//! the shapes travel *with the contract*: a bundle carries every schema
//! artifact its types live in (spec §9.5), verified against the revision's
//! fingerprint, so a tool that was never compiled against the contract reads
//! them from there — offline from `--contracts`, or retrieved from the
//! revision's holders (spec §8.4).

use anyhow::Result;

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

/// `zenctl check schema` (#159; zk2's since #612, FJ8b): one payload, from
/// a file or stdin, against one type of a zk2 contract, exit-coded for CI.
/// 0 = it conforms; 1 = it does not (violations of its JSON Schema type,
/// or bytes that do not decode as the declared type at all); 2 = it could
/// not be checked (no revision, no such resource or member, a raw type that
/// declares no structure, an unreadable payload) — "could not check" must
/// never exit like either verdict.
///
/// The type is the revision's own, from `--contracts` or retrieved from its
/// holders (spec §8.4): a JSON Schema type decodes as JSON or CBOR and is
/// validated with `zenkey_model::validate` (§7.3); a protobuf type decodes
/// through the bundle's descriptor set (§7.2). This checks; it never
/// publishes and never encodes.
pub async fn check(cli: crate::cli::CheckSchemaArgs) -> Result<()> {
    let dep = ASKING.ask(Deployment::resolve(&cli.ns));
    let contracts = ASKING.ask(zk2::load_contracts(&cli.contracts));
    let crate::cli::CheckSchemaArgs {
        target,
        resource,
        member,
        from,
        encoding,
        contracts: _,
        ns: _,
    } = cli;
    // A payload that cannot be read is *unobservable*, not nonconformant
    // (#244): a typo'd path is not a schema violation.
    let bytes = match from.read() {
        Ok(bytes) => bytes,
        Err(e) => not_checked(&format!("{e:#}")),
    };
    let mut session = None;
    let revision = match zk2::revision(&dep, &contracts, &target, &mut session).await {
        Ok(r) => r,
        Err(e) => not_checked(&crate::errors::without_source_locations(&format!("{e:#}"))),
    };
    use zenkey_model::authoring::Kind;
    let r = match revision.resource(
        &resource,
        &[Kind::Stream, Kind::State, Kind::Event, Kind::Operation],
    ) {
        Ok(r) => r,
        Err(e) => not_checked(&e),
    };
    let member = member.into();
    let report =
        match zenkey_fleet::check_payload(&revision, r, member, encoding.as_deref(), &bytes) {
            Ok(report) => report,
            Err(why) => not_checked(&why),
        };
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())?;
    // Off a `match` on the conformance, never a string (#356).
    match report.conformance {
        zenkey_fleet::report::Conformance::Valid => Ok(()),
        zenkey_fleet::report::Conformance::Invalid { .. }
        | zenkey_fleet::report::Conformance::Undecodable { .. } => {
            std::process::exit(crate::exit::FINDING)
        }
        zenkey_fleet::report::Conformance::NotChecked { reason } => not_checked(&reason),
    }
}

/// Exit 2: the check never happened — reserved so CI can tell "nonconformant"
/// from "unobservable", through the same seam as every other exit-2 (#355).
fn not_checked(reason: &str) -> ! {
    ASKING.unobservable(format_args!("not checked: {reason}"))
}
