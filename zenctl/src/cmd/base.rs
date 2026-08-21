//! `zenctl base list` — which deployment bases are actually on this bus.
//!
//! The one verb that deliberately never asks [`Bus::base`]: it exists to
//! answer *"what would I even pass as `--base`?"*, and filtering the answer by
//! the base you already guessed would make it useless.
//!
//! Lived inline in the dispatch until #209. The one-shot and the `--watch`
//! form now sit in one file, which is how the two stay the same command.

use anyhow::Result;

use crate::Bus;
use crate::report;

/// The bases in use, once.
pub async fn list(args: &Bus) -> Result<()> {
    let session = args.session().await?;
    let bases = zenkey_fleet::discover_bases(&session, args.timeout()).await?;
    crate::render::emit_with(
        &mut std::io::stdout(),
        &report::BaseList { bases },
        args.format(),
        args.color(),
    )
}
