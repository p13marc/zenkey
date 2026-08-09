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

#[allow(clippy::too_many_arguments)]
pub async fn run(
    key: &str,
    body: &str,
    qos: &str,
    encoding: Option<&str>,
    repeat: usize,
    interval: f64,
    no_validate: bool,
    raw: bool,
    args: &BusArgs,
) -> Result<()> {
    let qos = zenkey::qos::QosProfile::from_name(qos).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown QoS profile {qos:?} — sampled|refreshed|transition|alert|frame (RFC 04 §3)"
        )
    })?;
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
        publication.send(prepared.bytes.clone()).await?;
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
}
