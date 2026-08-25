//! `service info` — one producer's declared procedures.
//!
//! `service list` and `service call` live in `run()` and `cmd::call`
//! respectively; this module exists for the one verb that had a body and no
//! home (#354).

use anyhow::Result;

use crate::Bus;

pub async fn info(cli: crate::cli::ServiceInfoArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let crate::cli::ServiceInfoArgs {
        producer,
        procedure,
        bus: _,
    } = cli;
    let report = bus
        .slice_set()
        .await?
        .service_info(&producer, procedure.as_deref())?;
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
}
