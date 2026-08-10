//! The two bus operations RFC 07 §2.5 sanctions, in the order it sanctions
//! them: probe across origins with a tiny reply, then fetch from the one origin
//! you chose. Behind the `blob` feature, which is what pulls in the reference
//! client ([`zblob`]).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use zenkey::grammar::{self, ContentHash, Origin};
use zenkey::{RegistrySlice, RemoteOrigin, ServiceOrigin};
use zenoh::Session;
use zenoh::qos::Priority;

use super::{BlobTarget, declared_by};
use crate::query::{Answer, FleetAnswer, fleet_get_at};
use crate::report::{
    BlobAvailability, BlobFetchReport, BlobHolder, BlobManifest, BlobProbeReport, BlobProgress,
    CallError,
};

/// The priority every `@blob` GET this crate issues rides at (RFC 07 §2.6).
///
/// One constant, read by both the probe (which sets it on `fleet_get_at`) and
/// the fetch report (which names it) — so what the report says and what the
/// wire carried cannot drift apart. The fetch itself does not read it: the
/// reference client already defaults to `DataLow`, and re-setting it here would
/// mean two places to change and one of them silently winning.
pub const FETCH_PRIORITY: Priority = Priority::DataLow;

/// How a fetch should behave (RFC 07 §2.1, §2.5).
pub struct BlobFetchSpec {
    /// Per-query timeout. A transfer spans many queries, so this bounds a
    /// *stall*, not the transfer.
    pub timeout: Duration,
    /// Replace an existing destination file rather than refusing.
    pub overwrite: bool,
    /// The pinned content root (RFC 07 §2.1). `None` is trust-on-first-use,
    /// which the caller had to ask for out loud — [`BlobFetchReport::root_pinned`]
    /// reports which it was.
    pub root: Option<ContentHash>,
    /// Cooperative cancellation, so a GUI's stop button is not a lie.
    pub cancel: zblob::CancelToken,
}

impl Default for BlobFetchSpec {
    fn default() -> Self {
        BlobFetchSpec {
            timeout: Duration::from_secs(30),
            overwrite: false,
            root: None,
            cancel: zblob::CancelToken::new(),
        }
    }
}

/// RFC 07 §2.5, discharged: probe across origins with a tiny reply, attribute
/// by each reply's own key, and hand back one **concrete** key per holder.
///
/// The selector comes from [`zenkey::BlobProbePrefix`], which is not
/// convertible to a `Key` — so this function is the only shape a `*`-origin
/// `@blob` GET can take in this crate, and it can only ever ask for the two
/// tiny endpoints. Both GETs ride at [`FETCH_PRIORITY`].
///
/// **Tier 2 has no probe endpoint, and that is not an oversight.** A `tree` or
/// `store` key carries the object itself, so a `*`-origin GET on one *is* the
/// bulk fan-out §2.5 forbids — there is no small thing to ask for. Such a
/// target comes back with `not_probed` set and `declared_by` filled from the
/// slices: a capability claim, honestly labelled, never a possession verdict.
pub async fn blob_probe(
    session: &Session,
    base: &str,
    target: &BlobTarget,
    slices: &[RegistrySlice],
    timeout: Duration,
) -> Result<BlobProbeReport> {
    let tier = target.tier();
    let declared = declared_by(slices, tier);

    let Some(id) = target.artifact_id() else {
        return Ok(BlobProbeReport {
            target: target.spelling(),
            tier: tier.chunk().to_string(),
            asked: Vec::new(),
            not_probed: Some(format!(
                "the `{}` tier has no probe endpoint: its key carries the object itself, so a *-origin GET would be the wildcard bulk fetch RFC 07 §2.5 forbids. Fetch it from one origin's concrete key, or probe the tier-1 artifact that references it.",
                tier.chunk()
            )),
            holders: Vec::new(),
            answered: 0,
            roots: Vec::new(),
            declared_by: declared,
        });
    };

    // The wide form: `<base>/v1/*/@blob/artifact/<id>/{have,manifest}`. The
    // prefix is zenkey's probe type; the endpoint tails come from the reference
    // client, which is where RFC 07 §2.2's table is spelled out in code.
    let prefix = target.probe_prefix();
    let have = grammar::with_base(base, zblob::availability_key(prefix.as_str(), id));
    let manifest = grammar::with_base(base, zblob::manifest_key(prefix.as_str(), id));
    let asked = vec![have.clone(), manifest.clone()];

    let mut holders: Vec<BlobHolder> = Vec::new();
    for (key, kind) in [(&have, Endpoint::Have), (&manifest, Endpoint::Manifest)] {
        let answers = fleet_get_at(session, base, key, None, timeout, FETCH_PRIORITY).await?;
        for answer in answers {
            fold(&mut holders, base, kind, answer);
        }
    }
    holders.sort_by(|a, b| a.origin.cmp(&b.origin));

    let mut roots: Vec<String> = holders
        .iter()
        .filter_map(|h| h.manifest.as_ref().map(|m| m.root.clone()))
        .collect();
    roots.sort();
    roots.dedup();

    Ok(BlobProbeReport {
        target: target.spelling(),
        tier: tier.chunk().to_string(),
        asked,
        not_probed: None,
        answered: holders.len(),
        holders,
        roots,
        declared_by: declared,
    })
}

#[derive(Clone, Copy)]
enum Endpoint {
    Have,
    Manifest,
}

/// Merge one reply into the holder list, keyed by origin: `have` and
/// `manifest` are two GETs, and one origin answering both is one holder.
fn fold(holders: &mut Vec<BlobHolder>, base: &str, kind: Endpoint, answer: FleetAnswer) {
    let origin = attribute(base, &answer);
    let idx = match holders.iter().position(|h| h.origin == origin) {
        Some(i) => i,
        None => {
            holders.push(BlobHolder {
                origin: origin.clone(),
                key: answer.key.clone(),
                availability: None,
                manifest: None,
                unreadable: None,
                error: None,
            });
            holders.len() - 1
        }
    };
    let holder = &mut holders[idx];
    if holder.key.is_empty() {
        holder.key = answer.key.clone();
    }

    match answer.answer {
        Answer::Error { name, message } => {
            holder.error = Some(CallError { name, message });
        }
        Answer::Value(payload) => {
            let bytes = payload.to_bytes();
            let (want, decoded) = match kind {
                Endpoint::Have => (
                    zblob::wire::ENC_AVAIL,
                    decode_have(&bytes).map(|a| holder.availability = Some(a)),
                ),
                Endpoint::Manifest => (
                    zblob::wire::ENC_MANIFEST,
                    decode_manifest(&bytes).map(|m| holder.manifest = Some(m)),
                ),
            };
            if let Err(why) = decoded {
                // It answered; we could not read it. That is an observation
                // about this origin, not silence (RFC 09 §5.1 O4) — so it is
                // recorded rather than dropped, with what it claimed to be.
                let declared = answer.encoding.as_deref().unwrap_or("(none)");
                holder.unreadable =
                    Some(format!("{why} (encoding `{declared}`, expected `{want}`)"));
            }
        }
    }
}

/// The responder's origin. `FleetAnswer::origin` is already the grammar's
/// answer; the fallback reads position 1 off the reply's own key, so a key that
/// does not parse under this base still *names* its holder (RFC 09 §5.1 O1) —
/// which is the whole point of a probe.
fn attribute(base: &str, answer: &FleetAnswer) -> String {
    if answer.origin != "?" {
        return answer.origin.clone();
    }
    let stripped = answer
        .key
        .strip_prefix(base)
        .map(|s| s.trim_start_matches('/'))
        .unwrap_or(&answer.key);
    stripped
        .split('/')
        .nth(1)
        .filter(|c| !c.is_empty())
        .unwrap_or("?")
        .to_string()
}

fn decode_have(bytes: &[u8]) -> Result<BlobAvailability, String> {
    let avail: zblob::wire::Availability =
        zblob::wire::decode(bytes).map_err(|e| format!("undecodable availability: {e}"))?;
    Ok(BlobAvailability {
        chunk_count: avail.chunk_count,
        have: avail.count(),
        complete: avail.count() == avail.chunk_count,
    })
}

fn decode_manifest(bytes: &[u8]) -> Result<BlobManifest, String> {
    let m: zblob::Manifest =
        zblob::wire::decode(bytes).map_err(|e| format!("undecodable manifest: {e}"))?;
    Ok(BlobManifest {
        chunk_count: chunk_count(m.total_len, m.chunk_size),
        id: m.id,
        filename: m.filename,
        total_len: m.total_len,
        chunk_size: m.chunk_size,
        root: m.root.to_string(),
        created_ms: m.created_ms,
    })
}

fn chunk_count(total_len: u64, chunk_size: u32) -> u32 {
    if chunk_size == 0 {
        return 0;
    }
    total_len.div_ceil(chunk_size as u64) as u32
}

/// Fetch from **one** origin's concrete key, at data-low, verifying every reply
/// against the content root before disk (RFC 07 §2.1, §2.5, §2.6).
///
/// `origin` is parsed through [`RemoteOrigin::parse`] / [`ServiceOrigin::new`],
/// both of which reject `*` — so a wildcard fetch is refused here *and*
/// unspellable upstream, which is the layering the plane's whole design rests
/// on. The transfer itself is the reference client's: every slice is verified
/// against the root as it arrives, and a rejected reply never reaches the
/// destination file.
pub async fn blob_fetch(
    session: &Session,
    base: &str,
    origin: &str,
    target: &BlobTarget,
    dest: &Path,
    spec: &BlobFetchSpec,
    on_progress: &(dyn Fn(BlobProgress) + Send + Sync),
) -> Result<BlobFetchReport> {
    let origin = parse_origin(origin)?;
    let Some(id) = target.artifact_id() else {
        bail!(
            "`{}` cannot be fetched by this build: the reference client exposes no single-chunk fetch for the content-addressed tiers, so `{}` under {} is reachable only by a client that speaks tier 2 directly (RFC 07 §2.3/§2.4)",
            target.spelling(),
            target
                .key_at(&origin)
                .map(|k| grammar::with_base(base, k.as_str()))
                .unwrap_or_else(|_| target.spelling()),
            origin.chunk()
        );
    };

    let prefix = grammar::with_base(base, target.prefix_at(&origin).as_str());
    let key = grammar::with_base(
        base,
        target
            .key_at(&origin)
            .context("building the concrete blob key")?
            .as_str(),
    );

    let client = zblob::BlobClient::builder(Arc::new(session.clone()), prefix)
        // Priority is deliberately not set: the reference client already
        // defaults to DataLow, which is how RFC 07 §2.6 says a conformant
        // caller behaves without touching the setting. Setting it again here
        // would create a second source of truth for `FETCH_PRIORITY`.
        .query_timeout(spec.timeout)
        .overwrite(if spec.overwrite {
            zblob::Overwrite::Replace
        } else {
            zblob::Overwrite::Refuse
        })
        .build();

    let request = match &spec.root {
        Some(root) => {
            let parsed: zblob::Hash = root.as_str().parse().map_err(|e| {
                anyhow!("`{root}` is not a content root the reference client accepts: {e}")
            })?;
            zblob::DownloadRequest::pinned(id, parsed)
        }
        None => zblob::DownloadRequest::new(id),
    };
    let root_pinned = request.expected_root.is_some();

    let sink = move |p: zblob::Progress| on_progress(translate(p));
    let stats = client
        .download_to(&request, dest, &sink, &spec.cancel)
        .await
        // The origin is named here, once, so every failure this fetch can
        // produce — a hash mismatch above all — says which origin produced it.
        // A verification failure that does not name its source is an
        // unactionable one.
        .map_err(|e| anyhow!("{}: {e}", origin.chunk()))?;

    Ok(BlobFetchReport {
        origin: origin.chunk().to_string(),
        key,
        dest: dest.display().to_string(),
        bytes: stats.bytes_fetched,
        chunks: stats.chunks_fetched,
        chunks_resumed: stats.chunks_resumed,
        rejected: stats.rejected,
        retries: stats.retries,
        elapsed_ms: stats.elapsed.as_millis() as u64,
        root: request
            .expected_root
            .map(|r| r.to_string())
            .unwrap_or_default(),
        root_pinned,
        priority: priority_name(FETCH_PRIORITY).to_string(),
    })
}

/// One concrete origin, host or service. Both constructors reject `*`, which is
/// what makes "fetch from one origin" a type-level guarantee rather than a
/// convention.
fn parse_origin(origin: &str) -> Result<Origin> {
    if let Some(service) = origin.strip_prefix('@') {
        let _ = service;
        let svc = ServiceOrigin::new(origin)
            .map_err(|e| anyhow!("`{origin}` is not a service origin: {e}"))?;
        return Ok(Origin::Service(svc.as_str().to_string()));
    }
    let host = RemoteOrigin::parse(origin).map_err(|e| {
        anyhow!(
            "`{origin}` is not one concrete origin: {e}. A fetch names exactly one holder (RFC 07 §2.5) — probe first, then fetch from an origin the probe reported."
        )
    })?;
    Ok(Origin::Host(host.host_id().clone()))
}

fn translate(p: zblob::Progress) -> BlobProgress {
    match p {
        zblob::Progress::Started {
            total_len,
            chunk_count,
        } => BlobProgress::Started {
            total_len,
            chunk_count,
        },
        zblob::Progress::Resumed { received, total } => BlobProgress::Resumed { received, total },
        zblob::Progress::Chunk {
            index,
            received,
            total,
            bytes_received,
        } => BlobProgress::Chunk {
            index,
            received,
            total,
            bytes_received,
        },
        zblob::Progress::Verifying => BlobProgress::Verifying,
        zblob::Progress::Completed { path } => BlobProgress::Completed {
            path: path.display().to_string(),
        },
        zblob::Progress::Cancelled { received, total } => {
            BlobProgress::Cancelled { received, total }
        }
        zblob::Progress::Failed { error } => BlobProgress::Failed { error },
        // The reference client's progress type is #[non_exhaustive]; a variant
        // added upstream must not be silently swallowed, so it surfaces as what
        // it is — an event this build does not understand.
        other => BlobProgress::Failed {
            error: format!("unrecognised progress event from the reference client: {other:?}"),
        },
    }
}

fn priority_name(p: Priority) -> &'static str {
    match p {
        Priority::RealTime => "real-time",
        Priority::InteractiveHigh => "interactive-high",
        Priority::InteractiveLow => "interactive-low",
        Priority::DataHigh => "data-high",
        Priority::Data => "data",
        Priority::DataLow => "data-low",
        Priority::Background => "background",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wildcard_origin_is_not_an_origin() {
        for spelled in ["*", "**", "h-*", "", "not-a-host"] {
            assert!(
                parse_origin(spelled).is_err(),
                "`{spelled}` must not parse as a fetch origin"
            );
        }
        assert!(parse_origin("h-3fa9c2d41b7e").is_ok());
        assert!(parse_origin("@catalog").is_ok());
    }

    #[test]
    fn the_reported_priority_is_the_one_the_client_uses() {
        // The report's sentence and the wire's behaviour come from one
        // constant; this pins the rendering of it.
        assert_eq!(priority_name(FETCH_PRIORITY), "data-low");
    }

    #[test]
    fn chunk_counts_round_up() {
        assert_eq!(chunk_count(0, 65536), 0);
        assert_eq!(chunk_count(1, 65536), 1);
        assert_eq!(chunk_count(65536, 65536), 1);
        assert_eq!(chunk_count(65537, 65536), 2);
        assert_eq!(chunk_count(10, 0), 0);
    }

    #[test]
    fn an_unparseable_key_still_names_its_holder() {
        // O1: the grammar could not classify this key, and the probe must
        // still say who answered — that is what a probe is for.
        let answer = FleetAnswer {
            origin: "?".to_string(),
            key: "zensight/v1/h-3fa9c2d41b7e/@blob/artifact/NOPE/have".to_string(),
            encoding: None,
            answer: Answer::Error {
                name: "error/x".into(),
                message: String::new(),
            },
        };
        assert_eq!(attribute("zensight", &answer), "h-3fa9c2d41b7e");
    }
}
