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
pub async fn show(type_name: &str, schema: bool, full: bool, args: &Bus) -> Result<()> {
    let slices = args.slices().await?;
    let mut report =
        zenkey_fleet::SliceSet::from_slices(slices.clone()).interface_show(type_name)?;
    if schema {
        // Only the producers that carry the type are asked — the registry
        // already says who, so this is never a fleet fan-out.
        let producers = super::schema::carriers_of(&slices, type_name);
        let session = args.session().await?;
        let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
        report.schemas =
            zenkey_fleet::schemas_for_type(&store, &session, &producers, type_name, full).await;
    }
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}
