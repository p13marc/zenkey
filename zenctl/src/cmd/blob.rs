//! `blob list / probe / fetch` — the `@blob` plane (RFC 07 §2, issue #58).
//!
//! Three commands with three different costs, and the CLI makes the difference
//! visible rather than hiding it behind one verb:
//!
//! - `list` reads registry slices and touches no data plane at all;
//! - `probe` fans two *tiny* GETs across origins (§2.5's sanctioned form);
//! - `fetch` moves bytes, from exactly one origin, at data-low (§2.6).
//!
//! The shape of the plane is enforced by types, not by checks here: the wide
//! form is a `BlobProbePrefix` that does not convert into a key, and `--from`
//! goes through `RemoteOrigin::parse`, which rejects `*`. So "wildcard bulk
//! fetch is unspellable through the CLI" is a property of the grammar crate
//! that this module inherits, rather than a validation it could forget.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use zenkey_fleet::report::{BlobListSource, BlobProgress};
use zenkey_fleet::{BlobFetchSpec, BlobTarget, blob_fetch, blob_list, blob_probe};

use crate::{BusArgs, output};

/// `blob list` — which producers declare which tiers.
///
/// Answers offline from `--registry` alone. The liveliness join is attempted
/// only when a session is available; when it is not, `origins` stays `None` and
/// renders as "not asked" rather than as an empty set (RFC 09 §5.1 O4).
pub async fn list(producer: Option<&str>, tier: Option<&str>, args: &BusArgs) -> Result<()> {
    let has_dirs = !args.registry_dirs().is_empty();
    let slices = args.slices().await?;

    // The roster is a separate, cheap sweep — and skipping it is not a defeat:
    // the report distinguishes "nobody asked" from "nobody answered".
    let roster = match args.session().await {
        Ok(session) => zenkey_fleet::roster(&session, args.base(), args.timeout())
            .await
            .ok(),
        Err(_) => None,
    };

    // `--registry` does not exclude the bus (#43): the slice set is a union,
    // so the report says so rather than claiming a purity it does not have.
    let source = if has_dirs {
        BlobListSource::Union
    } else {
        BlobListSource::Bus
    };
    let mut report = blob_list(&slices, roster.as_ref(), source);
    if let Some(p) = producer {
        report.tiers.retain(|r| r.producer == p);
    }
    if let Some(t) = tier {
        report.tiers.retain(|r| r.tier == t);
    }
    output::blob_list(&report, args.format)
}

/// `blob probe <target>` — who holds it, and at which root.
pub async fn probe(target: &str, args: &BusArgs) -> Result<()> {
    let target = BlobTarget::parse(target)?;
    let session = args.session().await?;
    // Slices are best-effort here: they only fill `declared_by`, the capability
    // claim that makes a *silent* probe legible. A fleet with no registry
    // still probes.
    let slices = args.slices().await.unwrap_or_default();
    let report = blob_probe(&session, args.base(), &target, &slices, args.timeout()).await?;
    output::blob_probe(&report, args.format)
}

/// `blob fetch <target> --from <origin> -o <path>` — one origin, verified.
#[allow(clippy::too_many_arguments)]
pub async fn fetch(
    target: &str,
    from: &str,
    out: &Path,
    root: Option<&str>,
    allow_unpinned: bool,
    overwrite: bool,
    quiet: bool,
    args: &BusArgs,
) -> Result<()> {
    let target = BlobTarget::parse(target)?;

    // RFC 07 §2.1: every reference handed to a consumer MUST carry the identity
    // of the bytes it names. An operator typing an id on a command line has no
    // reference, so trust-on-first-use has to be *asked for* — and the report
    // then says which it was. Tier 2 needs neither flag: the key is the root.
    let pinned = match (root, matches!(target, BlobTarget::Artifact { .. })) {
        (Some(hex), _) => Some(zenkey::ContentHash::parse(hex).map_err(|e| {
            anyhow::anyhow!("--root {hex}: {e} (RFC 07 §2.1 — the anchor is the content root)")
        })?),
        (None, true) if !allow_unpinned => bail!(
            "refusing a trust-on-first-use fetch: pass --root <hex> (the content root the \
             reference carried), or --allow-unpinned to accept whatever this origin serves. \
             RFC 07 §2.1 requires a reference to carry the identity of the bytes it names; \
             `zenctl blob probe {}` reports the roots on offer.",
            target.spelling()
        ),
        (None, _) => None,
    };

    let session = args.session().await?;
    let spec = BlobFetchSpec {
        timeout: args.timeout(),
        overwrite,
        root: pinned,
        ..Default::default()
    };

    // Progress goes to stderr so `--format ndjson | jq` stays clean.
    let on_progress = move |p: BlobProgress| {
        if quiet {
            return;
        }
        if let Some(line) = progress_line(&p) {
            eprintln!("{line}");
        }
    };

    if !quiet {
        eprintln!(
            "fetching {} from {from} at data-low (RFC 07 §2.6)",
            target.spelling()
        );
    }
    let report = blob_fetch(
        &session,
        args.base(),
        from,
        &target,
        out,
        &spec,
        &on_progress,
    )
    .await?;
    output::blob_fetch(&report, args.format)
}

fn progress_line(p: &BlobProgress) -> Option<String> {
    match p {
        BlobProgress::Started {
            total_len,
            chunk_count,
        } => Some(format!(
            "  manifest: {total_len} bytes in {chunk_count} chunks"
        )),
        BlobProgress::Resumed { received, total } => Some(format!(
            "  resumed: {received}/{total} chunks already banked"
        )),
        BlobProgress::Chunk {
            received, total, ..
        } => Some(format!("  {received}/{total} chunks verified")),
        BlobProgress::Verifying => Some("  verifying".to_string()),
        BlobProgress::Cancelled { received, total } => {
            Some(format!("  cancelled at {received}/{total} chunks"))
        }
        // Completion and failure are the report's job, not a progress line's.
        BlobProgress::Completed { .. } | BlobProgress::Failed { .. } => None,
    }
}

/// Default destination when `-o` is omitted: the target's own spelling, so a
/// hash-named tier-2 object lands under its hash and an artifact under its id.
///
/// The server's advisory filename is deliberately *not* used. A remote party
/// must not pick where our bytes land — that is the path-traversal vector the
/// reference client's docs call out, and a CLI is exactly where it would bite.
pub fn default_dest(target: &str) -> PathBuf {
    PathBuf::from(
        target
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("blob"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_destination_never_comes_from_the_far_end() {
        assert_eq!(
            default_dest("01jqz3demo0001"),
            PathBuf::from("01jqz3demo0001")
        );
        assert_eq!(
            default_dest("artifact/01jqz3demo0001"),
            PathBuf::from("01jqz3demo0001")
        );
        assert_eq!(default_dest("store/blake3/abcd"), PathBuf::from("abcd"));
        // No leading separator can survive, whatever was typed.
        assert!(!default_dest("a/b/").is_absolute());
    }

    #[test]
    fn a_progress_line_never_claims_an_outcome() {
        // Completion and failure are the report's, so a `--quiet` run and a
        // loud one agree about what happened.
        assert!(
            progress_line(&BlobProgress::Completed {
                path: "/tmp/x".into()
            })
            .is_none()
        );
        assert!(
            progress_line(&BlobProgress::Failed {
                error: "boom".into()
            })
            .is_none()
        );
        assert!(progress_line(&BlobProgress::Verifying).is_some());
    }
}
