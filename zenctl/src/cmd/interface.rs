//! `zenctl interface list|show` — the registry's type vocabulary, and the
//! schemas behind it (RFC 08 §7).
//!
//! Lived inline in the dispatch until #209. `show --schema` is the reason it
//! could not stay there: it is the one verb here that reaches the bus, and it
//! does so *narrowly* — the registry already names which producers carry a
//! type, so asking only those is not a fleet fan-out.

use anyhow::Result;

use crate::Bus;

/// The declared types, from whichever slice source the flags select.
pub async fn list(args: &Bus) -> Result<()> {
    let report = args.slice_set().await?.interface_list();
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

/// One type: its shape, and optionally the schemas its carriers actually
/// serve.
pub async fn show(cli: crate::cli::InterfaceShowArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let crate::cli::InterfaceShowArgs {
        type_name,
        schema,
        full,
        bus: _,
    } = cli;
    let type_name = type_name.as_str();
    let slices = args.slices().await?;
    let mut report =
        zenkey_fleet::SliceSet::from_slices(slices.clone()).interface_show(type_name)?;
    if schema {
        // Only the producers that carry the type are asked — the registry
        // already says who, so this is never a fleet fan-out.
        let producers = super::schema::carriers_of(&slices, type_name);
        let carriers = zenkey_fleet::SliceSet::from_slices(
            slices
                .into_iter()
                .filter(|s| producers.contains(&s.name))
                .collect(),
        );
        let session = args.session().await?;
        let fleet = args.fleet(&session);
        // The engine's sweep, not a `SchemaStore` (#410): the store keeps
        // the first parseable reply per producer and no origin, so two hosts
        // of one producer could never disagree in it, and this page's drift
        // note was a second, worse copy of the doctor's check. The sweep
        // keeps every answer attributed, the rows come off its first-per-
        // producer fold, and the verdict is `schema_drift` — the one
        // implementation — filtered to this type.
        let sweep = zenkey_fleet::describe_sweep(&fleet, &carriers, args.timeout()).await?;
        // `Asked` even when nothing answered: asked-and-unserved is a
        // different fact from never-asked (O4, R4).
        report.schemas = zenkey_fleet::report::Asked::Asked(zenkey_fleet::schema_rows_for_type(
            &sweep.first_per_producer(),
            type_name,
            full,
        ));
        report.drift = zenkey_fleet::schema_drift(&sweep.answers)
            .into_iter()
            .filter(|d| d.type_name == type_name)
            .collect();
    }
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}
