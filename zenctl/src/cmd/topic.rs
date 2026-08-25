//! `topic list` — what the registry declares, filtered.
//!
//! `topic info` stays a two-line arm in `run()`: it has no decision to move
//! here (#354).

use anyhow::Result;

use crate::Bus;

/// `topic list`, with its two mode decisions where the verb is (#354).
///
/// `--watch` and `--budget` used to be chosen by the dispatcher, which is the
/// same shape as the `--latency implies --per-key` rule one verb over: a
/// decision about what this verb *means*, taken by the code that only routes.
pub async fn list(cli: crate::cli::TopicListArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let crate::cli::TopicListArgs {
        producer,
        class,
        r#type,
        deprecated,
        watch,
        every,
        budget,
        for_secs,
        bus: _,
    } = cli;
    let filter = crate::cmd::watch::TopicFilter {
        producer,
        class,
        type_name: r#type,
        deprecated,
    };
    if watch {
        return crate::cmd::watch::topic_list(every, &filter, &bus).await;
    }
    if budget {
        return crate::cmd::budget::topic_list_budget(&filter, for_secs, &bus).await;
    }
    let report = filter.apply(&bus.slice_set().await?)?;
    crate::render::emit_with(&mut std::io::stdout(), &report, bus.format(), bus.color())
}
