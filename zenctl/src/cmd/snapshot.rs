//! `zenctl snapshot` (#219, RFC 13 §4.4; zk2's since #612, FJ8b): a
//! deployment's current state kept on disk as a `.zsnap`, and two of them
//! compared by zk2 key.
//!
//! The take is the engine's [`take_snapshot`]: one state GET per selector,
//! the owners answering (spec §4.2 S4: target All, consolidation Latest),
//! every reply folded per key and resolved through the raw observers' lens
//! — the presence read made first, in the namespace, naming each address's
//! instances and its owner's session zid, and the revisions its descriptors
//! name. Written through the same `tokio::fs` → `BufWriter` path `record`
//! uses. **Silence is not a snapshot**: a GET nobody answered exits 2
//! through [`crate::exit`]'s reserved non-verdict and writes nothing,
//! because a file of zero rows would read as an empty deployment the next
//! time somebody diffed against it.
//!
//! The diff opens no session. Its exit is the RFC 13 §1.2 projection over
//! the diff's own judgement — a difference *is* the finding (1), identity
//! is clean (0) — and a file that could not be read is the reserved 2,
//! never a claim that the two agree. It compares by zk2 key: each row's key
//! relative to its own file's namespace, so two deployments line up on the
//! keys their services publish, which v1's origin alignment existed to
//! approximate.

use std::io::{BufReader, BufWriter};

use anyhow::{Context, Result};
use zenkey_fleet::{DiffOpts, Lens, SnapshotSpec, ZsnapReader, ZsnapWriter, take_snapshot};

use crate::bus::Deployment;
use crate::cli::{SnapshotArgs, SnapshotDiffArgs};
use crate::cmd::zk2;
use crate::exit::{self, Asking};

/// The take's name on its one exit-2 path (silence under fan-out).
const TAKING: Asking = Asking::new("snapshot");
/// The diff's name on its pre-run guard (a file that would not read).
const DIFFING: Asking = Asking::new("snapshot diff");

pub async fn take(cli: SnapshotArgs) -> Result<()> {
    let dep = Deployment::resolve(&cli.ns)?;
    let contracts = zk2::load_contracts(&cli.contracts)?;
    let SnapshotArgs {
        selectors,
        out,
        max_replies,
        no_presence,
        overwrite,
        cmd: _,
        contracts: _,
        ns: _,
    } = cli;
    // `required = true` under `subcommand_negates_reqs`: clap has already
    // refused a take without `--out`, so this is the type's shape, not a
    // second check.
    let Some(out) = out else {
        return Err(exit::unaskable!(
            "--out <FILE> is required to take a snapshot"
        ));
    };
    // An existing snapshot is refused unless --overwrite (#514) — here,
    // before the session, because nothing the fleet answers changes it.
    let mode = super::output_mode(&out, overwrite)?;
    let namespace = dep.namespace().to_owned();
    let asked: Vec<String> = if selectors.is_empty() {
        vec![zk2::default_selector(&namespace)]
    } else {
        selectors
            .iter()
            .map(|s| zk2::wire_selector(Some(s), &namespace))
            .collect::<Result<_>>()?
    };
    // Only state answers a GET (spec §1.3): each selector narrowed to its
    // state keys, and one that reaches none named, never silently dropped.
    let mut gets = Vec::new();
    let mut unreachable = Vec::new();
    for s in &asked {
        match zenkey_fleet::state_projection(&namespace, s) {
            Some(p) if !gets.contains(&p) => gets.push(p),
            Some(_) => {}
            None => unreachable.push(s.clone()),
        }
    }
    for s in &unreachable {
        eprintln!(
            "snapshot: {s} reaches no state key of namespace {namespace:?} — another kind, a \
             control key, or outside the namespace: not asked"
        );
    }
    if gets.is_empty() {
        return Err(exit::unaskable!(
            "no selector reaches a state key in {}: a snapshot is of state (spec §4.2)",
            zk2::namespace_phrase(&namespace)
        ));
    }

    let store = zk2::store(&dep, &contracts);
    let catalog = if no_presence {
        None
    } else {
        let ns_session = dep.session().await?;
        zk2::read_lens(&dep, &ns_session, &store).await
    };
    let held = store.held().len();
    let lens = Lens::new(&namespace, catalog.as_ref(), &store)
        .offline(&contracts)
        .held(held);
    let session = dep.link().session().await?;
    let spec = SnapshotSpec {
        selectors: gets.clone(),
        timeout: dep.timeout(),
        max_replies,
    };

    eprintln!(
        "snapshot of {} to {out} — collected over a span, not at an instant",
        gets.join(" + ")
    );

    let taken = take_snapshot(&session, &lens, &spec).await?;
    if taken.snapshot.header.answered == 0 {
        // Silence under fan-out is the reserved 2 (`exit.rs`), and the file
        // is deliberately not written: nothing answered is not "nothing
        // there", and a `.zsnap` of zero rows would claim exactly that.
        TAKING.unobservable(format!(
            "nobody answered {} within {}s — nothing written; silence is not a \
             snapshot (S6)",
            gets.join(" + "),
            dep.timeout().as_secs_f64()
        ));
    }

    // The create through `tokio::fs`, like `record` (#332); the rows are
    // bounded and already in hand, so the write itself is one buffered pass.
    // `create_new` (#514): a file that appeared during the collection is
    // refused like one that was there before it.
    let file = super::open_output_or_refuse(&out, mode).await?;
    let mut writer = ZsnapWriter::new(BufWriter::new(file), &taken.snapshot.header)?;
    for row in &taken.snapshot.rows {
        writer.write_row(row)?;
    }
    writer.finish()?;

    let mut incomplete = taken.incomplete;
    incomplete.extend(unreachable);
    let report = zenkey_fleet::snapshot_report(&taken.snapshot, Some(out), incomplete);
    crate::render::emit_with(&mut std::io::stdout(), &report, dep.format(), dep.color())
}

pub fn diff(cli: SnapshotDiffArgs) -> Result<()> {
    let SnapshotDiffArgs {
        a,
        b,
        max_changes,
        out,
    } = cli;
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
    let diff = zenkey_fleet::diff_snapshots(&a, &b, opts);
    crate::render::emit_with(&mut std::io::stdout(), &diff, out.format, out.color)?;
    exit::verdict(&diff.to_judgement())
}
