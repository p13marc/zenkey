//! `topic pub` — publish through the write facade (issue #47): a declared
//! publisher, never an ad-hoc put (P7); and since #97, a body that actually
//! ships in the encoding the subject declares.

use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::{BodySource, PrepareMode};

use crate::BusArgs;

/// Which of the engine's three preparation modes the flags select. Shared
/// with `service call` so both write paths read the same flags the same way.
pub fn mode(raw: bool, no_validate: bool) -> PrepareMode {
    match (raw, no_validate) {
        (true, _) => PrepareMode::Raw,
        (false, true) => PrepareMode::Lenient,
        (false, false) => PrepareMode::Encode,
    }
}

/// Parse an explicit `--qos` flag; the error names the closed vocabulary.
fn parse_qos(name: &str) -> Result<zenkey::qos::QosProfile> {
    zenkey::qos::QosProfile::from_name(name).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown QoS profile {name:?} — sampled|refreshed|transition|alert|frame (RFC 04 §3)"
        )
    })
}

/// The subject's declared profile, when the key refines to one (#158).
/// Returns the profile and the declared path for the source note.
fn declared_qos(
    base: &str,
    key: &str,
    slices: Option<&zenkey_fleet::SliceSet>,
) -> Option<(zenkey::qos::QosProfile, String)> {
    match zenkey_fleet::describe_key(base, key, slices)
        .facts
        .registration
    {
        zenkey_fleet::facts::Registration::Registered(s) => {
            s.declared_qos().map(|q| (q, s.path.clone()))
        }
        _ => None,
    }
}

/// #158's resolution ladder: an explicit flag wins, the declared profile is
/// the default, `sampled` is the stated last resort. The choice and its
/// source are printed either way — a publisher that picked its own QoS
/// silently would be the write-side O4 mistake.
fn resolve_qos(
    explicit: Option<zenkey::qos::QosProfile>,
    base: &str,
    key: &str,
    slices: Option<&zenkey_fleet::SliceSet>,
) -> zenkey::qos::QosProfile {
    if let Some(q) = explicit {
        return q;
    }
    match declared_qos(base, key, slices) {
        Some((q, path)) => {
            eprintln!("qos: {} (declared for {path})", q.name());
            q
        }
        None => {
            eprintln!("qos: sampled (default — no declared profile for this key)");
            zenkey::qos::QosProfile::Sampled
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    key: &str,
    body: &str,
    qos: Option<&str>,
    encoding: Option<&str>,
    repeat: usize,
    interval: f64,
    no_validate: bool,
    raw: bool,
    attachment: Option<&str>,
    args: &BusArgs,
) -> Result<()> {
    // An explicit --qos fails fast, before the body or the bus.
    let explicit_qos = qos.map(parse_qos).transpose()?;
    let typed = match body {
        "-" => {
            use std::io::Read as _;
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
        b => match b.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => b.as_bytes().to_vec(),
        },
    };
    // The attachment ships verbatim (#117): never schema-encoded, the
    // registry's vocabulary ends at the payload.
    let attachment: Option<Vec<u8>> = match attachment {
        None => None,
        Some(a) => Some(match a.strip_prefix('@') {
            Some(path) => std::fs::read(path)?,
            None => a.as_bytes().to_vec(),
        }),
    };

    let session = args.session().await?;
    // Registry awareness: when the key refines to a registered subject, the
    // producer's served schema **encodes** the body — those bytes are what
    // rides the wire, and the declared encoding labels them. A body the schema
    // cannot encode is refused before it touches the bus. An unregistered key,
    // an untyped subject, or a producer serving no describe publishes as
    // typed, and the engine's note says which happened (--raw opts out
    // entirely, --no-validate opts out of the refusal but not the labelling).
    let slices = if raw {
        None
    } else {
        args.slice_set().await.ok()
    };
    let store = zenkey_fleet::decode::SchemaStore::new(args.base(), args.timeout());
    let prepared = zenkey_fleet::prepare_publish(
        &session,
        &store,
        slices.as_ref(),
        args.base(),
        key,
        encoding,
        &typed,
        mode(raw, no_validate),
    )
    .await?;
    if let Some(note) = &prepared.note {
        eprintln!("note: {note}");
    }
    if let BodySource::Encoded { type_name } = &prepared.source {
        eprintln!(
            "encoded {} bytes as {type_name} → {}",
            prepared.bytes.len(),
            prepared.encoding.as_deref().unwrap_or("(no encoding set)")
        );
    }
    let qos = resolve_qos(explicit_qos, args.base(), key, slices.as_ref());

    let publication =
        zenkey_fleet::declare_publication(&session, key, qos, prepared.encoding.as_deref()).await?;
    // Matching note (#38): a routing fact about THIS publisher — informative,
    // never gating, and never a fleet verdict (RFC 05 §3.1).
    match publication.matching_status().await {
        Ok(true) => eprintln!("matching: a subscriber currently matches {key}"),
        Ok(false) => eprintln!(
            "matching: no subscriber currently matches {key} — a routing fact about \
             this publisher, not a fleet verdict (RFC 05 §3.1)"
        ),
        Err(_) => {}
    }
    let times = repeat.max(1);
    for n in 0..times {
        publication
            .send(prepared.bytes.clone(), attachment.clone())
            .await?;
        eprintln!(
            "published {key} ({} bytes) [{}/{times}]",
            prepared.bytes.len(),
            n + 1
        );
        if n + 1 < times {
            tokio::time::sleep(Duration::from_secs_f64(interval.max(0.0))).await;
        }
    }
    publication.undeclare().await?;
    Ok(())
}

/// `topic retire` — the RFC 04 §1.2 tombstone, class-guarded (#115).
pub async fn retire(key: &str, qos: &str, i_know: bool, args: &BusArgs) -> Result<()> {
    let qos = parse_qos(qos)?;
    // Slices are best-effort, like pub: the guard is honest about a missing
    // registry (a state key still passes — the class is in the key).
    let slices = args.slice_set().await.ok();
    let verdict = zenkey_fleet::check_retire(args.base(), key, slices.as_ref(), i_know)?;
    match &verdict {
        zenkey_fleet::RetireClass::State { registered, ttl_s } => match (registered, ttl_s) {
            (true, Some(ttl)) => eprintln!(
                "retiring a state key — the tombstone stays observable ≥ {ttl}s \
                 where storages enforce gc.lifespan (RFC 04 §1.2)"
            ),
            (true, None) => eprintln!("retiring a state key (no ttl_s declared)"),
            (false, _) => eprintln!(
                "retiring an unregistered state key — the tombstone is still \
                 authoritative; no ttl_s bounds its observability"
            ),
        },
        zenkey_fleet::RetireClass::NonState { class } => eprintln!(
            "retiring a {class} key as an operator cleanup (RFC 04 §1.2, v1.12) — \
             --i-know acknowledged"
        ),
        zenkey_fleet::RetireClass::Unclassified { reason } => {
            eprintln!("retiring an unclassified key ({reason}) — --i-know acknowledged")
        }
    }

    let session = args.session().await?;
    let publication = zenkey_fleet::declare_publication(&session, key, qos, None).await?;
    // The same routing fact pub prints, with the same bounds (RFC 05 §3.1).
    match publication.matching_status().await {
        Ok(true) => eprintln!("matching: a subscriber currently matches {key}"),
        Ok(false) => eprintln!(
            "matching: no subscriber currently matches {key} — a routing fact about \
             this publisher, not a fleet verdict (RFC 05 §3.1)"
        ),
        Err(_) => {}
    }
    publication.retire().await?;
    eprintln!("retired {key}");
    publication.undeclare().await?;
    Ok(())
}

/// Where `topic pub` reads from, besides its arguments.
/// `topic pub --from ndjson` (#125): the pipe made symmetric. Reads the
/// exact row shape `topic echo --format ndjson` emits, publishes each row
/// through a declared publisher — one per distinct key, reusing the write
/// facade, never ad-hoc puts — and counts what it could not publish
/// instead of silently skipping it.
pub async fn run_from_ndjson(
    default_qos: Option<&str>,
    interval: f64,
    i_know: bool,
    args: &BusArgs,
) -> Result<()> {
    use std::io::BufRead as _;

    // An explicit --qos fails fast; otherwise each key falls back to its
    // declared profile, then sampled — the same ladder as `topic pub` (#158).
    let explicit_qos = default_qos.map(parse_qos).transpose()?;
    let session = args.session().await?;
    let slices = args.slice_set().await.ok();
    let base = args.base().to_string();

    let mut publications: std::collections::HashMap<String, zenkey_fleet::Publication> =
        std::collections::HashMap::new();
    let mut published = 0usize;
    let mut tombstones = 0usize;
    let mut malformed = 0usize;
    let mut refused = 0usize;
    let mut first_errors: Vec<String> = Vec::new();
    let mut record_err = |line_no: usize, reason: String, count: &mut usize| {
        *count += 1;
        if first_errors.len() < 3 {
            first_errors.push(format!("line {line_no}: {reason}"));
        }
    };

    let stdin = std::io::stdin();
    for (i, line) in stdin.lock().lines().enumerate() {
        let line_no = i + 1;
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row = match zenkey_fleet::parse_row(&line) {
            Ok(r) => r,
            Err(e) => {
                record_err(line_no, e, &mut malformed);
                continue;
            }
        };
        // A delete row is a tombstone (RFC 04 §1.2): even in a pipe, the
        // off-state operator act keeps its price (v1.12) — refused rows are
        // counted, never silently dropped.
        if row.delete
            && let Err(e) = zenkey_fleet::check_retire(&base, &row.key, slices.as_ref(), i_know)
        {
            record_err(line_no, e.to_string(), &mut refused);
            continue;
        }
        let publication = match publications.entry(row.key.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                // Row > flag > declared > sampled. Only the row's own name can
                // be malformed; the declared fallback resolves silently per
                // key (a per-row note would drown the pipe's real output).
                let qos = match row.qos.as_deref() {
                    Some(name) => match zenkey::qos::QosProfile::from_name(name) {
                        Some(q) => q,
                        None => {
                            record_err(
                                line_no,
                                format!("unknown QoS profile {name:?}"),
                                &mut malformed,
                            );
                            continue;
                        }
                    },
                    None => explicit_qos
                        .or_else(|| declared_qos(&base, &row.key, slices.as_ref()).map(|(q, _)| q))
                        .unwrap_or(zenkey::qos::QosProfile::Sampled),
                };
                let publication = zenkey_fleet::declare_publication(
                    &session,
                    &row.key,
                    qos,
                    row.encoding.as_deref(),
                )
                .await?;
                e.insert(publication)
            }
        };
        if row.delete {
            publication.retire().await?;
            tombstones += 1;
        } else {
            publication.send(row.payload, row.attachment).await?;
            published += 1;
        }
        if interval > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(interval)).await;
        }
    }

    for (_, publication) in publications.drain() {
        publication.undeclare().await?;
    }
    let keys = published + tombstones;
    eprintln!(
        "published {published} row(s) and {tombstones} tombstone(s){}",
        if keys > 0 {
            ""
        } else {
            " — stdin held no publishable rows"
        }
    );
    if malformed > 0 || refused > 0 {
        eprintln!(
            "{malformed} malformed row(s), {refused} refused delete row(s) — counted, \
             not silently skipped:"
        );
        for e in &first_errors {
            eprintln!("  {e}");
        }
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `--raw` wins over everything: it is the escape hatch, not a preference.
    #[test]
    fn flag_pairs_map_onto_the_three_modes() {
        assert_eq!(mode(false, false), PrepareMode::Encode);
        assert_eq!(mode(false, true), PrepareMode::Lenient);
        assert_eq!(mode(true, false), PrepareMode::Raw);
        assert_eq!(mode(true, true), PrepareMode::Raw);
    }

    /// #158: flag > declared > sampled, and each rung is reachable.
    #[test]
    fn qos_resolves_flag_then_declared_then_sampled() {
        use zenkey::qos::QosProfile;
        use zenkey::slice::{RegistrySlice, SubjectDecl};
        let slice = RegistrySlice {
            version: "1.0".into(),
            app: "test".into(),
            convention: 1,
            name: "sysinfo".into(),
            service_origin: None,
            description: None,
            subjects: vec![SubjectDecl {
                path: "health".into(),
                class: "state".into(),
                type_name: "Health".into(),
                common: None,
                since: None,
                description: None,
                qos: Some("transition".into()),
                ttl_s: None,
                unit: None,
                rate: None,
                cardinality: None,
                encoding: None,
            }],
            procedures: vec![],
            blob: vec![],
            media: vec![],
            deprecated: vec![],
        };
        let slices = zenkey_fleet::SliceSet::from_slices(vec![slice]);
        let key = "v1/h-3fa9c2d41b7e/state/sysinfo/health";

        // The declared profile is found, with its path for the note.
        assert_eq!(
            declared_qos("", key, Some(&slices)),
            Some((QosProfile::Transition, "health".to_string()))
        );
        // An explicit flag wins over it.
        assert_eq!(
            resolve_qos(Some(QosProfile::Alert), "", key, Some(&slices)),
            QosProfile::Alert
        );
        // No flag: the declared profile drives.
        assert_eq!(
            resolve_qos(None, "", key, Some(&slices)),
            QosProfile::Transition
        );
        // Unregistered key, no flag: the stated last resort.
        let unregistered = "v1/h-3fa9c2d41b7e/state/sysinfo/other";
        assert_eq!(
            resolve_qos(None, "", unregistered, Some(&slices)),
            QosProfile::Sampled
        );
        // No registry at all: not asked is not "declared nothing" — but the
        // publisher still needs a profile, and sampled is the stated one.
        assert_eq!(resolve_qos(None, "", key, None), QosProfile::Sampled);
    }
}
