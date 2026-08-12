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
use zenkey_fleet::{ReplayEvent, ReplayTarget, ZrecReader};

use crate::BusArgs;
use crate::output;

#[allow(clippy::too_many_arguments)] // clap surface
pub async fn run(
    file: &str,
    speed: f64,
    dry_run: bool,
    force_base: bool,
    i_know: bool,
    qos: &str,
    args: &BusArgs,
) -> Result<()> {
    let source = std::fs::File::open(file).with_context(|| format!("open {file}"))?;
    let mut reader = ZrecReader::new(BufReader::new(source))?;
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

    let ndjson = matches!(args.format.resolved(), output::Format::Ndjson);
    let mut on_event = |ev: ReplayEvent<'_>| match ev {
        ReplayEvent::WouldPut {
            key,
            bytes,
            encoding,
        } => {
            if ndjson {
                println!(
                    "{}",
                    serde_json::json!({
                        "would": "put", "key": key, "bytes": bytes,
                        "encoding": encoding,
                    })
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
                println!("{}", serde_json::json!({"would": "retire", "key": key}));
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
            ReplayTarget::DryRun,
            speed,
            i_know,
            qos,
            &mut on_event,
        )
        .await?
    } else {
        let session = args.session().await?;
        let slices = args.slice_set().await.ok();
        zenkey_fleet::replay(
            &mut reader,
            ReplayTarget::Bus {
                session: &session,
                slices: slices.as_ref(),
            },
            speed,
            i_know,
            qos,
            &mut on_event,
        )
        .await?
    };

    let failed = report.malformed > 0 || report.refused > 0;
    output::replay(&report, args.format);
    if failed {
        std::process::exit(1);
    }
    Ok(())
}
