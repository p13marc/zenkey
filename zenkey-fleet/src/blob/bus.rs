//! The two bus operations RFC 07 §2.5 sanctions, in the order it sanctions
//! them: probe across origins with a tiny reply, then fetch from the one origin
//! you chose. Behind the `blob` feature, which is what pulls in the reference
//! client ([`zblob`]).

use std::path::Path;
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
/// `@blob` GET can take in this crate, and it can only ever ask for the tiny
/// endpoints. Every probe GET rides at [`FETCH_PRIORITY`].
///
/// **Tier 2 is probed through its v1.17 endpoints** (RFC 07 §2.4/§2.5):
/// `store/<algo>/have` answers a bitfield over exactly the asked addresses,
/// `tree/<root>/have` answers has-index plus chunks present/total — replies
/// that are O(question) by construction, which is what makes the wildcard
/// origin as legitimate there as it always was on Tier 1, and what turns the
/// old `not_probed` apology into a **possession verdict**. The one honest
/// refusal left is a store algorithm the reference client does not speak;
/// that still comes back as `not_probed`, with `declared_by` filled from the
/// slices.
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
        return probe_tier2(session, base, target, declared, timeout).await;
    };

    // The wide form: `<base>/v1/*/@blob/artifact/<id>/{have,manifest}`. The
    // prefix is zenkey's probe type; the endpoint tails come from the reference
    // client, which is where RFC 07 §2.2's table is spelled out in code.
    let prefix = target.probe_prefix();
    let have = grammar::with_base(base, zblob::keys::availability_key(prefix.as_str(), id));
    let manifest = grammar::with_base(base, zblob::keys::manifest_key(prefix.as_str(), id));
    let asked = vec![have.clone(), manifest.clone()];

    // Two independent questions to the same fleet, asked concurrently: a
    // probe costs one timeout window, not two. Folding stays sequential and
    // ordered (have, then manifest), so the merge is deterministic.
    let (have_answers, manifest_answers) = tokio::join!(
        fleet_get_at(session, base, &have, None, timeout, FETCH_PRIORITY),
        fleet_get_at(session, base, &manifest, None, timeout, FETCH_PRIORITY),
    );
    let mut holders: Vec<BlobHolder> = Vec::new();
    for (answers, kind) in [
        (have_answers?, Endpoint::Have),
        (manifest_answers?, Endpoint::Manifest),
    ] {
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

/// The Tier-2 half of [`blob_probe`] (RFC 07 §2.4/§2.5, v1.17): ask the tiny
/// endpoint whose reply size is a function of the question, and report what
/// each holder *has* — a possession verdict, attributed by the reply's own
/// key exactly as the Tier-1 probe attributes its holders.
async fn probe_tier2(
    session: &Session,
    base: &str,
    target: &BlobTarget,
    declared: Vec<String>,
    timeout: Duration,
) -> Result<BlobProbeReport> {
    let tier = target.tier();
    let probe_prefix = grammar::with_base(base, target.probe_prefix().as_str());
    let report = |asked: Vec<String>, not_probed: Option<String>, holders: Vec<BlobHolder>| {
        BlobProbeReport {
            target: target.spelling(),
            tier: tier.chunk().to_string(),
            asked,
            not_probed,
            answered: holders.len(),
            holders,
            roots: Vec::new(),
            declared_by: declared.clone(),
        }
    };

    // Both tier-2 probes ride the same fleet chokepoint as tier 1 (RFC 05
    // §2.1: consolidation None, attribution by each reply's own key), and
    // fold with the same posture: an errored or undecodable holder is
    // *recorded*, never dropped — answered-but-unreadable is an observation
    // about an origin, not silence (RFC 09 §5.1 O4). The reference client's
    // own probe helpers skip such replies, which is right for a transfer
    // client choosing a source and wrong for an explorer reporting a fleet.
    match target {
        BlobTarget::Store { algo, hash } => {
            // Probing is per-algorithm like everything else on this tier
            // (RFC 07 §2.4). The reference client speaks one; a foreign algo
            // is the one honest `not_probed` left, and it must say so rather
            // than answer "no holders" for a question it never asked.
            if algo != zblob::Hash::ALGO {
                return Ok(report(
                    Vec::new(),
                    Some(format!(
                        "the reference client speaks `{}` only, so a `{algo}` chunk cannot be probed by this build (RFC 07 §2.4 — dedup and probing are per-algorithm)",
                        zblob::Hash::ALGO
                    )),
                    Vec::new(),
                ));
            }
            let parsed: zblob::Hash = hash.as_str().parse().map_err(|e| {
                anyhow!("`{hash}` is not a content address the reference client accepts: {e}")
            })?;
            let have_key = zblob::keys::store_have_key(&probe_prefix, zblob::HashAlgo::Blake3);
            let want = zblob::wire::encode(&zblob::wire::WantList::new(vec![parsed]))
                .map_err(|e| anyhow!("encoding the want-list: {e}"))?;
            let answers = fleet_get_at(
                session,
                base,
                &have_key,
                Some(want),
                timeout,
                FETCH_PRIORITY,
            )
            .await?;
            let holders = fold_tier2(base, answers, |bytes| {
                let bits: zblob::wire::HaveBits = zblob::wire::decode(bytes)
                    .map_err(|e| format!("undecodable have bitfield: {e}"))?;
                bits.validate(1)
                    .map_err(|e| format!("invalid have bitfield: {e}"))?;
                let held = bits.is_set(0);
                Ok((
                    BlobAvailability {
                        chunk_count: 1,
                        have: u32::from(held),
                        complete: held,
                    },
                    None,
                ))
            });
            Ok(report(vec![have_key], None, holders))
        }
        BlobTarget::Tree { root } => {
            // The probe key must be an address the reference client could
            // serve: `ContentHash` admits any even-length hex, `zblob::Hash`
            // exactly one digest size — validating here keeps the probe and
            // the fetch agreeing about what is askable, instead of the probe
            // returning an honest-looking "nobody holds it" for a root no
            // holder could ever have.
            let parsed: zblob::Hash = root.as_str().parse().map_err(|e| {
                anyhow!("`{root}` is not a content address the reference client accepts: {e}")
            })?;
            let have_key = zblob::keys::tree_have_key(&probe_prefix, &parsed.to_string());
            let answers =
                fleet_get_at(session, base, &have_key, None, timeout, FETCH_PRIORITY).await?;
            let holders = fold_tier2(base, answers, |bytes| {
                let probe: zblob::wire::TreeProbe = zblob::wire::decode(bytes)
                    .map_err(|e| format!("undecodable tree probe: {e}"))?;
                probe
                    .validate()
                    .map_err(|e| format!("invalid tree probe: {e}"))?;
                // A full-looking chunk count with no index is the one verdict
                // the counters cannot express, and it predicts exactly how a
                // fetch from this holder fails — say it.
                let note = (!probe.have_index && probe.chunks_present > 0).then(|| {
                    "holds chunks but not the index — an index fetch from this origin will fail"
                        .to_string()
                });
                Ok((
                    BlobAvailability {
                        chunk_count: probe.chunks_total,
                        have: probe.chunks_present,
                        complete: probe.have_index && probe.chunks_present == probe.chunks_total,
                    },
                    note,
                ))
            });
            Ok(report(vec![have_key], None, holders))
        }
        BlobTarget::Artifact { .. } => {
            bail!("tier-1 target reached the tier-2 probe path — a bug in blob_probe")
        }
    }
}

/// One tier-2 reply becomes one holder, with [`fold`]'s O4 posture: errors
/// and unreadable payloads are recorded against the origin that produced
/// them. The holder's `key` is the reply's own key — the same attribution
/// evidence tier 1 keeps — and duplicate replies from one origin keep the
/// first, exactly as the tier-1 merge does.
fn fold_tier2(
    base: &str,
    answers: Vec<FleetAnswer>,
    decode: impl Fn(&[u8]) -> Result<(BlobAvailability, Option<String>), String>,
) -> Vec<BlobHolder> {
    let mut holders: Vec<BlobHolder> = Vec::new();
    for answer in answers {
        let origin = attribute(base, &answer);
        if holders.iter().any(|h| h.origin == origin) {
            continue;
        }
        let mut holder = BlobHolder {
            origin,
            key: answer.key.clone(),
            availability: None,
            manifest: None,
            note: None,
            unreadable: None,
            error: None,
        };
        match answer.answer {
            Answer::Error { name, message } => {
                holder.error = Some(CallError { name, message });
            }
            Answer::Value(payload) => match decode(&payload.to_bytes()) {
                Ok((availability, note)) => {
                    holder.availability = Some(availability);
                    holder.note = note;
                }
                Err(why) => {
                    let declared = answer.encoding.as_deref().unwrap_or("(none)");
                    holder.unreadable = Some(format!("{why} (encoding `{declared}`)"));
                }
            },
        }
        holders.push(holder);
    }
    holders.sort_by(|a, b| a.origin.cmp(&b.origin));
    holders
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
                note: None,
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
                    &zblob::wire::ENC_AVAIL,
                    decode_have(&bytes).map(|a| holder.availability = Some(a)),
                ),
                Endpoint::Manifest => (
                    &zblob::wire::ENC_MANIFEST,
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
    // The chunk count is the reference client's own arithmetic now (v3) —
    // but a manifest whose sizing does not divide still *names a root*, and
    // the root is what the §2.1 disagreement check feeds on. So the manifest
    // is kept and the count degrades to zero: a 0-chunk row renders oddly, a
    // discarded root renders as *agreement*, and only one of those is a lie.
    let chunk_count = m.chunk_count().unwrap_or(0);
    Ok(BlobManifest {
        chunk_count,
        id: m.id.to_string(),
        filename: m.filename,
        total_len: m.total_len,
        chunk_size: m.chunk_size,
        root: m.root.to_string(),
        created_ms: m.created_ms,
    })
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
        return fetch_tier2(session, base, &origin, target, dest, spec).await;
    };

    let prefix = grammar::with_base(base, target.prefix_at(&origin).as_str());
    let key = grammar::with_base(
        base,
        target
            .key_at(&origin)
            .context("building the concrete blob key")?
            .as_str(),
    );

    let prefix = zblob::QueryPrefix::new(prefix).map_err(|e| {
        anyhow!(
            "`{origin_chunk}`'s artifact prefix is not queryable: {e}",
            origin_chunk = origin.chunk()
        )
    })?;
    let client = zblob::BlobClient::builder(session, prefix)
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
        .download_to(&request, dest)
        .progress(&sink)
        .cancel(&spec.cancel)
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

/// The Tier-2 half of [`blob_fetch`] (RFC 07 §2.4, v1.17): one verified,
/// content-addressed chunk from one origin. The address *is* the pin — a
/// reply that unframes to anything else is rejected naming the origin, so
/// trust-on-first-use is unspellable on this path by construction.
async fn fetch_tier2(
    session: &Session,
    base: &str,
    origin: &Origin,
    target: &BlobTarget,
    dest: &Path,
    spec: &BlobFetchSpec,
) -> Result<BlobFetchReport> {
    let started = std::time::Instant::now();
    match target {
        BlobTarget::Store { algo, hash } => {
            if algo != zblob::Hash::ALGO {
                bail!(
                    "`{}` cannot be fetched by this build: the reference client speaks `{}` only (RFC 07 §2.4 — addressing is per-algorithm)",
                    target.spelling(),
                    zblob::Hash::ALGO
                );
            }
            let parsed: zblob::Hash = hash.as_str().parse().map_err(|e| {
                anyhow!("`{hash}` is not a content address the reference client accepts: {e}")
            })?;
            let prefix_str = grammar::with_base(
                base,
                grammar::blob_tier_prefix(origin, grammar::BlobTier::Store).as_str(),
            );
            let prefix = zblob::QueryPrefix::new(prefix_str.clone())
                .map_err(|e| anyhow!("`{prefix_str}` is not a queryable prefix: {e}"))?;
            let key = zblob::keys::store_key(prefix.as_str(), zblob::HashAlgo::Blake3, &parsed);
            let client = zblob::StoreClient::builder(session, prefix)
                .query_timeout(spec.timeout)
                .priority(FETCH_PRIORITY)
                .build();
            let bytes = client
                .fetch_chunk(&parsed)
                // The origin is named here for the same reason blob_fetch
                // names it: a verification failure that does not say which
                // origin produced it is unactionable.
                .await
                .map_err(|e| anyhow!("{}: {e}", origin.chunk()))?;
            if dest.exists() && !spec.overwrite {
                bail!(
                    "`{}` already exists — pass overwrite to replace it",
                    dest.display()
                );
            }
            std::fs::write(dest, &bytes).with_context(|| format!("writing {}", dest.display()))?;
            Ok(BlobFetchReport {
                origin: origin.chunk().to_string(),
                key,
                dest: dest.display().to_string(),
                bytes: bytes.len() as u64,
                chunks: 1,
                chunks_resumed: 0,
                rejected: 0,
                retries: 0,
                elapsed_ms: started.elapsed().as_millis() as u64,
                root: hash.to_string(),
                // The key is the root (RFC 07 §2.1): a store fetch cannot be
                // trust-on-first-use, so this is true by construction.
                root_pinned: true,
                priority: priority_name(FETCH_PRIORITY).to_string(),
            })
        }
        BlobTarget::Tree { .. } => bail!(
            "`{}` is inspected, not downloaded, by this explorer: a validated index summary needs no content store (RFC 07 §2.3, v1.17) — the frontends route tree targets to the tree-index report; materializing a tree is the reference client's `download_tree`, which needs a store this build deliberately does not keep",
            target.spelling()
        ),
        BlobTarget::Artifact { .. } => {
            bail!("tier-1 target reached the tier-2 fetch path — a bug in blob_fetch")
        }
    }
}

/// Fetch and fully validate one origin's index for `tree/<root>`, returning
/// the summary an explorer renders (RFC 07 §2.3, v1.17) — **no content store
/// involved**: the stats make inspecting a huge tree cheap, which is the
/// difference between browsing a snapshot and downloading one.
pub async fn blob_tree_index(
    session: &Session,
    base: &str,
    origin: &str,
    root: &ContentHash,
    timeout: Duration,
) -> Result<crate::report::BlobTreeIndexReport> {
    let started = std::time::Instant::now();
    let origin = parse_origin(origin)?;
    let tree_str = grammar::with_base(
        base,
        grammar::blob_tier_prefix(&origin, grammar::BlobTier::Tree).as_str(),
    );
    let store_str = grammar::with_base(
        base,
        grammar::blob_tier_prefix(&origin, grammar::BlobTier::Store).as_str(),
    );
    let tree_prefix = zblob::QueryPrefix::new(tree_str.clone())
        .map_err(|e| anyhow!("`{tree_str}` is not a queryable prefix: {e}"))?;
    let store_prefix = zblob::QueryPrefix::new(store_str.clone())
        .map_err(|e| anyhow!("`{store_str}` is not a queryable prefix: {e}"))?;
    let parsed: zblob::Hash = root.as_str().parse().map_err(|e| {
        anyhow!("`{root}` is not a content address the reference client accepts: {e}")
    })?;
    let key = zblob::keys::tree_key(tree_prefix.as_str(), root.as_str());
    // No priority setter: the reference client defaults to data-low, which is
    // FETCH_PRIORITY — the §2.6 conformant untouched default.
    let client = zblob::TreeClient::builder(session, store_prefix, tree_prefix)
        .query_timeout(timeout)
        .build();
    let index = client
        .fetch_index_by_root(&parsed)
        .await
        .map_err(|e| anyhow!("{}: {e}", origin.chunk()))?;
    Ok(crate::report::BlobTreeIndexReport {
        origin: origin.chunk().to_string(),
        key,
        root: root.to_string(),
        entries: index.entries().len(),
        files: index.file_count(),
        total_size: index.total_size(),
        chunks: index.needed_chunk_refs().len(),
        elapsed_ms: started.elapsed().as_millis() as u64,
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
    fn an_unparseable_key_still_names_its_holder() {
        // O1: the grammar could not classify this key, and the probe must
        // still say who answered — that is what a probe is for.
        let answer = FleetAnswer {
            origin: "?".to_string(),
            key: "zensight/v1/h-3fa9c2d41b7e/@blob/artifact/NOPE/have".to_string(),
            encoding: None,
            attachment: None,
            answer: Answer::Error {
                name: "error/x".into(),
                message: String::new(),
            },
        };
        assert_eq!(attribute("zensight", &answer), "h-3fa9c2d41b7e");
    }
}
