//! `zenctl pub` — publish through the write facade (issue #47): a declared
//! publisher, never an ad-hoc put (P7), with the QoS axes and the encoding
//! the operator states, or zenoh's defaults and none.
//!
//! Top-level since #307 — publishing is an act on the wire, not a verb of
//! the `topic` noun, which is what the registry declares.
//!
//! ## A zk2 service's own key is refused (#612, FJ5)
//!
//! A key is written only by the service that owns it (P3, spec §6, decided
//! 2026-10-08): `pub` refuses a key a zk2 service owns, with exit 2 and no
//! flag that moves it, and still writes every foreign key. A tool acts on a
//! service through its operations — `zenctl call`. v1's `retire` went with
//! it: a tombstone has no zk2 meaning a tool may send.
//!
//! ## Bytes, as typed (#612, FJ9)
//!
//! A foreign key has no contract to type it, so `pub` writes the body as
//! typed: no schema lookup, no encoding, no refusal, and the QoS the
//! operator gives (`--qos AXES`) or zenoh's own default. v1's registry
//! ladder — the declared profile, the served schema's encoding, `--raw` and
//! `--no-validate` to opt out of it — left with the v1 registry.
//!
//! ## An empty stdout is the contract (#242)
//!
//! Every sentence this module prints goes to **stderr**, and every one of the
//! sentences is deliberate. `pub` has no document to emit: its
//! answer is "it went out", and inventing a wire shape for that would be a
//! shape with no reader. What the empty stdout buys is composition — `zenctl
//! echo --format ndjson | zenctl pub --from ndjson` is the same row shape in
//! both directions (#235, RFC 09 §5.2), and a report on pub's stdout would put
//! something in the pipe that the next stage did not ask for.
//!
//! So `--format json` here is an empty stdout on purpose. It is not a verb
//! that was missed when the renderer seam went in; it is the one place the
//! seam's answer is "nothing".

use std::time::Duration;

use anyhow::Result;
use zenkey_fleet::WireQos;

use crate::bus::Link;
use crate::input::Source;

/// `zenctl pub`'s two shapes, told apart once (#209).
///
/// The `ArgGroup` on `PubArgs` has already refused every other combination:
/// exactly one of `--from`/`<KEY>`, and `<KEY>` requires `<BODY>`. What is
/// left is the two real shapes.
pub async fn dispatch(cli: crate::cli::PubArgs) -> Result<()> {
    let link = Link::resolve(&cli.session)?;
    match (cli.from, cli.key, cli.body) {
        (Some(crate::cli::PubSource::Ndjson), _, _) => {
            run_from_ndjson(cli.qos, cli.every, cli.i_know, &link).await
        }
        (None, Some(key), Some(body)) => {
            run(
                OneShot {
                    key: &key,
                    body: &body,
                    qos: cli.qos,
                    encoding: cli.encoding.as_deref(),
                    times: cli.times,
                    every: cli.every,
                    attachment: cli.attachment.as_ref(),
                },
                &link,
            )
            .await
        }
        (None, _, _) => unreachable!(
            "the `source` ArgGroup requires --from or <KEY>, and <KEY> requires <BODY>"
        ),
    }
}

/// One `pub <KEY> <BODY>`, named (#354): nine parameters of which four were
/// adjacent `Option<&str>`/`&str`, and `dispatch` filled them positionally.
struct OneShot<'a> {
    key: &'a str,
    body: &'a Source,
    qos: Option<WireQos>,
    encoding: Option<&'a str>,
    times: usize,
    every: f64,
    attachment: Option<&'a Source>,
}

/// The QoS a publication goes out with: the flag's, or zenoh's default —
/// said either way, because a publisher that picked its own QoS silently
/// would be the write-side O4 mistake (#158).
fn resolve_qos(explicit: Option<WireQos>) -> WireQos {
    match explicit {
        Some(q) => {
            eprintln!("qos: {}", q.token());
            q
        }
        None => {
            eprintln!(
                "qos: {} (zenoh's default — no contract types a foreign key)",
                WireQos::DEFAULT.token()
            );
            WireQos::DEFAULT
        }
    }
}

async fn run(p: OneShot<'_>, link: &Link) -> Result<()> {
    let OneShot {
        key,
        body,
        qos,
        encoding,
        times,
        every,
        attachment,
    } = p;
    // A wildcard key is a blast radius, not a publication (#504) — refused
    // first, before the body or the bus, and no flag moves it. Nor does one
    // move P3: a zk2 service's own key is its owner's alone to write.
    zenkey_fleet::check_concrete(key, zenkey_fleet::WriteAct::Put)?;
    refuse_owned(key)?;
    let bytes = body.read()?;
    // The attachment ships verbatim (#117).
    let attachment: Option<Vec<u8>> = attachment.map(Source::read).transpose()?;

    let session = link.session().await?;
    let qos = resolve_qos(qos);
    let publication = zenkey_fleet::declare_publication(&session, key, qos, encoding).await?;
    if let Some(note) = matching_note(&publication, key).await {
        eprintln!("{}", note.to_line());
    }
    // `--times` is a count, so 0 means zero and 1 means once (#307). The old
    // `--repeat` made 0 and 1 the same number, which is one spelling too many
    // — and the one people typed for "none" was the one that published.
    for n in 0..times {
        publication.send(bytes.clone(), attachment.clone()).await?;
        eprintln!(
            "published {key} ({} bytes) [{}/{times}]",
            bytes.len(),
            n + 1
        );
        if n + 1 < times {
            tokio::time::sleep(Duration::from_secs_f64(every.max(0.0))).await;
        }
    }
    publication.undeclare().await?;
    Ok(())
}

/// Refuses a key a zk2 service owns (P3, spec §6): exit 2, and no flag moves
/// it. The prefix before its `zk2` chunk is not read as a namespace (the
/// tooling guide's O3): an owner exists, wherever it runs.
fn refuse_owned(key: &str) -> Result<()> {
    match owned_refusal(key) {
        Some(why) => Err(crate::exit::unaskable!("{key} {why}")),
        None => Ok(()),
    }
}

/// Why `pub` will not write `key`, when it is a zk2 service's own.
fn owned_refusal(key: &str) -> Option<String> {
    zenkey_fleet::owned_key(key).map(|o| {
        format!(
            "is a key {} owns, and only the owner writes its keys (P3, spec §6) — act on \
             the service through its operations: zenctl call",
            o.owner
        )
    })
}

/// The #38 matching note: a routing fact about **this** publisher.
///
/// Informative, never gating, and never a fleet verdict — a zero here means
/// this session sees no matching subscriber, which is not the same claim as
/// "nobody is listening" (RFC 05 §3.1). `None` when the status could not be
/// read at all: an unanswerable question earns no sentence.
///
/// One spelling, once `retire` printed the same fact beside a verbatim copy
/// of it (#210); `retire` is gone (FJ5) and the one spelling stays.
async fn matching_note(
    publication: &zenkey_fleet::Publication,
    key: &str,
) -> Option<crate::render::Note> {
    match publication.matching_status().await {
        Ok(true) => Some(crate::render::Note::caveat(format!(
            "matching: a subscriber currently matches {key}"
        ))),
        Ok(false) => Some(crate::render::Note::silence(format!(
            "matching: no subscriber currently matches {key} — a routing fact \
             about this publisher, not a fleet verdict"
        ))),
        Err(_) => None,
    }
}

/// Where `zenctl pub` reads from, besides its arguments.
/// `zenctl pub --from ndjson` (#125): the pipe made symmetric. Reads the
/// exact row shape `zenctl echo --format ndjson` emits, publishes each row
/// through a declared publisher — one per distinct key, reusing the write
/// facade, never ad-hoc puts — and counts what it could not publish
/// instead of silently skipping it.
///
/// **Re-cut for zk2** (#612, FJ8a, FJ9). A row whose key a zk2 service owns
/// is refused exactly as `pub` refuses one (P3), and counted; a foreign row
/// is written as bytes, with the QoS axes it recorded (`qos_axes`), so
/// `echo --format ndjson | pub --from ndjson` keeps a foreign key's QoS. A
/// row that recorded none goes out with `--qos`, or zenoh's default. The
/// session opens on the first row there is to write: a pipe of refused rows
/// asks nothing of the bus.
pub async fn run_from_ndjson(
    default_qos: Option<WireQos>,
    every: f64,
    i_know: bool,
    link: &Link,
) -> Result<()> {
    use std::io::BufRead as _;

    let default_qos = default_qos.unwrap_or(WireQos::DEFAULT);
    let mut session: Option<zenoh::Session> = None;

    let mut publications: std::collections::HashMap<String, zenkey_fleet::Publication> =
        std::collections::HashMap::new();
    let mut published = 0usize;
    let mut tombstones = 0usize;
    let mut malformed = 0usize;
    let mut refused = 0usize;
    let mut meta = 0usize;
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
        // Through the stream reader, not the bare row parser: an echo stream
        // interleaves tagged meta rows (`"row":"dropped"`, `"row":"seed"`)
        // with its samples, and those are skipped-and-counted-as-skipped —
        // stream metadata is not a malformed row (#235's symmetry, kept).
        let row = match zenkey_fleet::parse_stream_line(&line) {
            Ok(zenkey_fleet::StreamLine::Sample(r)) => r,
            Ok(zenkey_fleet::StreamLine::Meta(_)) => {
                meta += 1;
                continue;
            }
            Err(e) => {
                record_err(line_no, e, &mut malformed);
                continue;
            }
        };
        // A delete row is a tombstone: even in a pipe, the operator act keeps
        // its price — refused rows are counted, never silently dropped. A put
        // row on a wildcard is the same blast radius as a wildcard delete
        // (#504), and a zk2 service's own key is its owner's alone (P3);
        // `--i-know` moves neither. Every refusal is decided before anything
        // is asked of the bus.
        if let Some(why) = owned_refusal(&row.key) {
            record_err(line_no, format!("{}: {why}", row.key), &mut refused);
            continue;
        }
        let refusal = if row.delete {
            zenkey_fleet::check_retire(&row.key, i_know)
                .err()
                .map(|e| e.to_string())
        } else {
            zenkey_fleet::check_concrete(&row.key, zenkey_fleet::WriteAct::Put)
                .err()
                .map(|e| e.to_string())
        };
        if let Some(e) = refusal {
            record_err(line_no, e, &mut refused);
            continue;
        }
        let publication = match publications.entry(row.key.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                // The row's axes > the flag > zenoh's default.
                let qos = row.qos_axes.unwrap_or(default_qos);
                if session.is_none() {
                    session = Some(link.session().await?);
                }
                let s = session.as_ref().expect("opened above");
                let publication =
                    zenkey_fleet::declare_publication(s, &row.key, qos, row.encoding.as_deref())
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
        if every > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(every)).await;
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
    if meta > 0 {
        eprintln!(
            "{meta} tagged meta line(s) skipped (dropped/seed markers — stream \
             metadata, not rows)"
        );
    }
    if malformed > 0 || refused > 0 {
        eprintln!(
            "{malformed} malformed row(s), {refused} refused row(s) — counted, \
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

    /// A zk2 service's own key is refused by name, whatever namespace it
    /// sits under; a foreign key is not.
    #[test]
    fn an_owned_key_is_refused_and_a_foreign_one_is_not() {
        let why = owned_refusal("prod/zk2/host-a/tc/tc.netif.v1/state/namespaces")
            .expect("an owner's key");
        assert!(why.contains("host-a/tc"), "{why}");
        assert!(why.contains("zenctl call"), "{why}");
        assert_eq!(owned_refusal("plant/line-1/temp"), None);
    }
}
