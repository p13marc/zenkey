//! `zenctl snapshot` (#219, RFC 13 §4.4): a fleet's current values kept
//! on disk as a `.zsnap`, and two of them compared.
//!
//! The take is the engine's [`take_snapshot`] — one fan-in GET per
//! selector, the roster joined beside it, every reply folded per key
//! last-writer-wins — written through the same `tokio::fs` → `BufWriter`
//! path `record` uses. **Silence is not a snapshot**: a fan-out nobody
//! answered exits 2 through [`crate::exit`]'s reserved non-verdict and
//! writes nothing, because a file of zero rows would read as an empty
//! fleet the next time somebody diffed against it (RFC 05 §3.1).
//!
//! The diff opens no session. Its exit is the RFC 13 §1.2 projection over
//! the diff's own judgement — a difference *is* the finding (1), identity
//! is clean (0) — and a file that could not be read is the reserved 2,
//! never a claim that the two agree.
//!
//! `--normalize-origins` (#220) is two deployments, one diff: the engine
//! profiles every host on both sides, plans the alignment (explicit
//! `--map`s, then verified unique labels, then unique producer sets —
//! `zenkey_fleet::plan_map`), and compares `b` read through it. An origin
//! the plan cannot place is the third exit path: the report goes out with
//! every unpaired origin listed, no comparison is made over them, and the
//! judgement is the reserved 2 — "I cannot map these" is the finding a
//! script can act on, and a diff that compared around them would not be.

use std::io::{BufReader, BufWriter};

use anyhow::{Context, Result};
use zenkey::origin::HostId;
use zenkey_fleet::{DiffOpts, SnapshotSpec, ZsnapReader, ZsnapWriter, take_snapshot};

use crate::Bus;
use crate::cli::{SnapshotArgs, SnapshotDiffArgs};
use crate::exit::{self, Asking};

/// The take's name on its one exit-2 path (silence under fan-out).
const TAKING: Asking = Asking::new("snapshot");
/// The diff's name on its pre-run guard (a file that would not read).
const DIFFING: Asking = Asking::new("snapshot diff");

pub async fn take(cli: SnapshotArgs) -> Result<()> {
    let bus = Bus::resolve(&cli.bus)?;
    let args = &bus;
    let SnapshotArgs {
        selector,
        out,
        max_replies,
        no_roster,
        cmd: _,
        bus: _,
    } = cli;
    // `required = true` under `subcommand_negates_reqs`: clap has already
    // refused a take without `--out`, so this is the type's shape, not a
    // second check.
    let Some(out) = out else {
        return Err(exit::unaskable!(
            "--out <FILE> is required to take a snapshot"
        ));
    };
    let selector = super::selector_of(&selector, args)?;

    let session = args.session().await?;
    // Slices enrich: registration and the verdict judge against the
    // registry; with none loaded every row says `registry_not_loaded` and
    // `no_registry` — not asked, not answered no (O4).
    let slices = args.slices_optional().await?;
    let store = zenkey_fleet::SchemaStore::new(args.base(), args.timeout());
    let fleet = args.fleet(&session);
    let spec = SnapshotSpec {
        selectors: vec![selector.clone()],
        timeout: args.timeout(),
        max_replies,
        roster: !no_roster,
    };

    // A `**` selector never crosses an `@`-chunk: say what the snapshot
    // cannot contain up front, not after someone diffs it (O5).
    eprintln!(
        "snapshot of {selector} to {out} — collected over a span, not at an instant{}",
        if selector.contains("**") {
            " — `**` cannot cross `@`-planes; they are excluded, not empty"
        } else {
            ""
        }
    );

    let taken = take_snapshot(&fleet, slices.as_ref(), &store, &spec).await?;
    if taken.snapshot.header.answered == 0 {
        // Silence under fan-out is the reserved 2 (`exit.rs`), and the file
        // is deliberately not written: nothing answered is not "nothing
        // there", and a `.zsnap` of zero rows would claim exactly that.
        TAKING.unobservable(format!(
            "nobody answered {selector} within {}s — nothing written; silence is \
             not a snapshot (RFC 05 §3.1)",
            args.timeout().as_secs_f64()
        ));
    }

    // The create through `tokio::fs`, like `record` (#332); the rows are
    // bounded and already in hand, so the write itself is one buffered pass.
    let file = tokio::fs::File::create(&out)
        .await
        .with_context(|| format!("create {out}"))?
        .into_std()
        .await;
    let mut writer = ZsnapWriter::new(BufWriter::new(file), &taken.snapshot.header)?;
    for row in &taken.snapshot.rows {
        writer.write_row(row)?;
    }
    writer.finish()?;

    let report = zenkey_fleet::snapshot_report(&taken.snapshot, Some(out), taken.incomplete);
    crate::render::emit_with(&mut std::io::stdout(), &report, args.format(), args.color())
}

pub fn diff(cli: SnapshotDiffArgs) -> Result<()> {
    let SnapshotDiffArgs {
        a,
        b,
        normalize_origins,
        maps,
        max_changes,
        out,
    } = cli;
    // Parsed at the edge, before either file is opened: a `--map` this tool
    // cannot read is an input it refuses (exit 2), not a pre-run failure.
    let maps = maps
        .iter()
        .map(|m| parse_map(m))
        .collect::<Result<Vec<_>>>()?;
    let read = |path: &str| -> Result<zenkey_fleet::Snapshot> {
        let file = std::fs::File::open(path).with_context(|| format!("open {path}"))?;
        let snapshot = ZsnapReader::new(BufReader::new(file))
            .and_then(ZsnapReader::read_all)
            .with_context(|| format!("read {path}"))?;
        Ok(snapshot)
    };
    // Both reads are pre-run: a file that will not read is the reserved 2,
    // not a 1 claiming the two differ.
    let a = DIFFING.ask(read(&a));
    let b = DIFFING.ask(read(&b));
    let opts = DiffOpts {
        max_changes,
        ..DiffOpts::default()
    };
    let diff = if normalize_origins {
        let plan = zenkey_fleet::plan_map(
            &zenkey_fleet::origin_profiles(&a),
            &zenkey_fleet::origin_profiles(&b),
            &maps,
        )
        // An explicit pair naming an origin neither file holds is the
        // operator's input, refused whole (exit 2) — a plan that quietly
        // dropped it would compare the wrong keys.
        .map_err(|e| exit::unaskable!("{e}"))?;
        zenkey_fleet::diff_normalized(&a, &b, &plan, opts)
    } else {
        zenkey_fleet::diff_snapshots(&a, &b, opts)
    };
    crate::render::emit_with(&mut std::io::stdout(), &diff, out.format, out.color)?;
    // Refused alignments exit 2 here too: `SnapshotDiff::to_judgement` is
    // `Unobservable` over an unpaired origin, and the projection is the one
    // seam (`exit.rs`).
    exit::verdict(&diff.to_judgement())
}

/// `A=B`, both sides `h-<12hex>` (RFC 03 §1.3) — anything else is refused
/// at the edge.
fn parse_map(spec: &str) -> Result<(HostId, HostId)> {
    let Some((x, y)) = spec.split_once('=') else {
        return Err(exit::unaskable!(
            "--map {spec:?}: expected A=B, a's origin = b's origin"
        ));
    };
    let parse = |side: &str, s: &str| {
        HostId::parse(s)
            .map_err(|e| exit::unaskable!("--map {spec:?}: {side} is not a host origin ({e})"))
    };
    Ok((parse("a", x)?, parse("b", y)?))
}
