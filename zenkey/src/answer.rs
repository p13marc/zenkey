//! A state GET's answer read whole (core §4.2; #977's E3): what the
//! consumer's GET, its re-read and an archive's alignment share.
//!
//! - **The owner's revision maps the index** (F-497): a reader applies the
//!   omission rule over a complete snapshot with the revision of its
//!   interface that the owner's descriptor names (§3.3), never with
//!   another. A runtime reader holds one revision, its contract, so it
//!   asks the owner's descriptor stating the snapshot's run before it
//!   drops anything ([`runs_revision`]), and drops nothing when that names
//!   another revision. Where no descriptor answers, as across a
//!   constrained face, the revision the reader was configured with is the
//!   deployment's word (§8.6, R7).
//! - **A GET's selector** ([`collection_read`]): a reader that wants a
//!   collection whole asks its snapshot key followed by `/**`, so that the
//!   owner answers by snapshot and its absences are evidence; a member
//!   read, concrete or wildcard, is answered member by member.

pub(crate) use zenkey_model::collection::collection_read;

use std::time::Duration;

use zenkey_model::canonical::Fingerprint;
use zenkey_model::contract::Contract;
use zenkey_model::grammar::{ZkKey, parse};
use zenkey_model::wire::Run;
use zenoh::query::{ConsolidationMode, Reply};

/// Whether the owner of `key` runs `contract`'s revision under `run` (core
/// §4.2, "The owner's revision maps the index"): its descriptor stating
/// `run` names `contract`'s fingerprint for its interface. A descriptor
/// that states no run stands for its instance where it is the only one.
/// Where none answers, or none states the run, the reader's own revision
/// is the deployment's word, and this is `true`.
pub(crate) async fn runs_revision(
    session: &zenoh::Session,
    key: &str,
    run: Run,
    contract: &Contract,
    timeout: Duration,
) -> bool {
    let Some(addr) = owner_of(key) else {
        return true;
    };
    let selector = format!("zk2/{}/{}/@zk/instance/*", addr.system, addr.service);
    let Ok(rx) = session
        .get(selector)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await
    else {
        return true;
    };
    let iface = contract.iface.to_string();
    let mine = Fingerprint::of(contract).to_string();
    let run = run.to_string();
    let mut named: Vec<(Option<String>, String)> = Vec::new();
    while let Ok(reply) = rx.recv_async().await {
        let Ok(sample) = reply.result() else { continue };
        let bytes = sample.payload().to_bytes();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let (Some(d), _) = zenkey_model::descriptor::check(text, &[]) else {
            continue;
        };
        if let Some(e) = d.interfaces.iter().find(|e| e.iface == iface) {
            named.push((d.run.clone(), e.contract.clone()));
        }
    }
    match named
        .iter()
        .find(|(r, _)| r.as_deref() == Some(run.as_str()))
    {
        Some((_, fp)) => *fp == mine,
        None => match named.as_slice() {
            [(None, fp)] => *fp == mine,
            _ => true,
        },
    }
}

/// The owner's address of a zk2 data key, a namespace in front skipped.
fn owner_of(key: &str) -> Option<zenkey_model::grammar::Addr> {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/")
            && let Ok(ZkKey::Data { addr, .. }) = parse(k)
        {
            return Some(addr);
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    None
}
