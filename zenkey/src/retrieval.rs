//! Fetching a contract bundle by its fingerprint (spec §8.4).
//!
//! 1. GET with target `BestMatching`: the nearest holder on each router the
//!    query visits.
//! 2. Verify each reply **as it arrives** (§9.6) and accept the first valid
//!    one, without waiting for the GET to complete (spike S4: 1.1–1.5 ms,
//!    against 1,001 ms for waiting).
//! 3. If none was valid, retry once with target `All`.
//! 4. If still none, the contract is unavailable. An unverified bundle is
//!    never accepted.
//!
//! Replies are taken unconsolidated, because zenoh's `Latest` consolidation
//! holds every reply until the query finalizes.

use std::time::Duration;

use zenkey_model::bundle::Bundle;
use zenkey_model::canonical::Fingerprint;
use zenkey_model::grammar::{IfaceId, contract_key};
use zenoh::query::{ConsolidationMode, QueryTarget, Reply};

use crate::error::{Result, zenoh};

/// What a retrieval found.
#[derive(Debug, Clone)]
pub enum Retrieved {
    /// The first reply that verified, with its bytes.
    Bundle(Box<Bundle>, Vec<u8>),
    /// No reply verified, after the retry with `All`. `refused` holds the
    /// verification tag of every reply refused, in arrival order (§9.6).
    Unavailable { refused: Vec<String> },
}

/// Retrieves the bundle of `iface` with fingerprint `fp`, per §8.4. Each GET
/// waits at most `timeout`.
pub async fn fetch_bundle(
    session: &zenoh::Session,
    iface: &IfaceId,
    fp: &Fingerprint,
    timeout: Duration,
) -> Result<Retrieved> {
    let key = contract_key(iface, fp.hex())?.into_keyexpr();
    let mut refused = Vec::new();
    for target in [QueryTarget::BestMatching, QueryTarget::All] {
        let rx = session
            .get(key.clone())
            .target(target)
            .consolidation(ConsolidationMode::None)
            .timeout(timeout)
            .with(flume::unbounded::<Reply>())
            .await
            .map_err(zenoh)?;
        while let Ok(reply) = rx.recv_async().await {
            let Ok(sample) = reply.result() else {
                refused.push("error_reply".to_owned());
                continue;
            };
            let bytes = sample.payload().to_bytes().into_owned();
            match Bundle::verify_expecting(&bytes, fp) {
                Ok(b) => return Ok(Retrieved::Bundle(Box::new(b), bytes)),
                Err(e) => refused.push(e.tag().to_owned()),
            }
        }
    }
    Ok(Retrieved::Unavailable { refused })
}
