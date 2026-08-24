//! `zenctl replay` (issue #53): publish a `.zrec` capture back onto a bus
//! — through declared publishers, at the capture's own pacing — or list
//! what doing so would put (`--dry-run`, no session at all). The etiquette
//! is RFC 09 §5.2's, enforced rather than suggested: the capture header's
//! base is a contract, and replaying under any other base is refused
//! without `--force-base`; a recorded delete keeps the retire gate's price
//! (`--i-know`); the capture's drop ledger is repeated, because a replay
//! of a partial view is a partial view.

use std::io::BufReader;

use anyhow::{Context, Result, bail};
use zenkey_fleet::{ReplayEvent, ReplayTarget, ZrecSource};

use crate::Bus;

#[allow(clippy::too_many_arguments)] // clap surface
pub async fn run(
    file: &str,
    speed: f64,
    dry_run: bool,
    force_base: bool,
    i_know: bool,
    qos: &str,
    args: &Bus,
) -> Result<()> {
    // The `--qos` name is checked here, before a byte is read: the engine
    // takes the closed enum, so an unknown profile is a refusal rather than a
    // per-row "malformed" event partway through a replay.
    let default_qos = super::publish::parse_qos(qos)?;
    // The read runs on the blocking pool (#332): a replay interleaves pacing
    // sleeps and network puts, and a blocking line read between them stalls
    // the runtime mid-pacing.
    let source = tokio::fs::File::open(file)
        .await
        .with_context(|| format!("open {file}"))?
        .into_std()
        .await;
    let mut reader = ZrecSource::spawn(BufReader::new(source)).await?;
    let header = reader.header().clone();

    // The header names what the capture asked and under which base; the
    // replay names where it is about to write. Both, up front, always.
    eprintln!(
        "{file}: captured {} under base {:?} ({})",
        header.selectors.join(" + "),
        header.base,
        header.captured_at,
    );
    let target_base = args.base();
    if !dry_run {
        eprintln!("replaying onto base {target_base:?} at speed {speed}");
    }
    if header.base != target_base && !force_base {
        bail!(
            "capture base {:?} != target base {:?} — recorded keys spell the \
             capture's deployment, and \"same keys, different deployment\" is \
             presumed a mistake (RFC 09 §5.2). Pass --force-base to mean it.",
            header.base,
            target_base,
        );
    }

    // One resolution for the whole run, and it happens in `Mode::of` (#198).
    // A streaming verb's question is only ever "is a program reading this" —
    // it has rows for one and prose for the other, and no third answer.
    let ndjson = crate::render::Mode::of(args.format()).machine();
    let mut on_event = |ev: ReplayEvent<'_>| match ev {
        ReplayEvent::WouldPut {
            key,
            bytes,
            encoding,
        } => {
            if ndjson {
                // Tagged (`"row":"would"`) like every non-sample line of an
                // explorer stream, so the preview cannot be mistaken for
                // publishable rows by a reader of the row dialect.
                println!(
                    "{}",
                    crate::render::Row::tagged(
                        "would",
                        serde_json::json!({
                            "would": "put", "key": key, "bytes": bytes,
                            "encoding": encoding,
                        })
                    )
                    .into_line()
                );
            } else {
                println!(
                    "would put {key}  ({bytes} B{})",
                    encoding.map(|e| format!(", {e}")).unwrap_or_default()
                );
            }
        }
        ReplayEvent::WouldRetire { key } => {
            if ndjson {
                println!(
                    "{}",
                    crate::render::Row::tagged(
                        "would",
                        serde_json::json!({"would": "retire", "key": key})
                    )
                    .into_line()
                );
            } else {
                println!("would retire {key}  (tombstone)");
            }
        }
        ReplayEvent::Malformed { reason } => eprintln!("malformed: {reason}"),
        ReplayEvent::Refused { key, reason } => eprintln!("refused {key}: {reason}"),
        ReplayEvent::CaptureDropped(n) => {
            eprintln!("-- the capture itself dropped {n} sample(s) here --");
        }
    };

    let report = if dry_run {
        zenkey_fleet::replay(
            &mut reader,
            zenkey_fleet::ReplaySpec {
                target: ReplayTarget::DryRun,
                speed,
                i_know,
                default_qos,
            },
            &mut on_event,
        )
        .await?
    } else {
        let session = args.session().await?;
        let slices = args.slices_optional().await?;
        zenkey_fleet::replay(
            &mut reader,
            zenkey_fleet::ReplaySpec {
                target: ReplayTarget::Bus {
                    session: &session,
                    slices: slices.as_ref(),
                },
                speed,
                i_know,
                default_qos,
            },
            &mut on_event,
        )
        .await?
    };

    let failed = report.malformed > 0 || report.refused > 0;
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())?;
    if failed {
        std::process::exit(1);
    }
    Ok(())
}
