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
//!   deployment's word (§8.6, R7). A read that failed, its timeout or a
//!   descriptor that does not read, is no such word: the reader drops
//!   nothing, and the entries still merge.
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
///
/// - **No descriptor reaches the reader**: the GET of its instances' keys
///   ran to its final reply and none answered, as across a constrained
///   face. The reader's own revision is then the deployment's word (§8.6,
///   R7), and this is `true`.
/// - **A read that failed** is no such word (review-bot on #1086): a GET
///   that could not be asked, an error reply (zenoh's at its timeout among
///   them), a descriptor that does not read, or descriptors that answered
///   and of which none names the interface under `run`. The reader then
///   cannot tell the revision that maps the index, so this is `false`: it
///   applies no omission rule over the snapshot, whose entries still
///   merge, as for an index naming no collection. The first such read is
///   logged at warning level, the rest at debug.
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
    let rx = match session
        .get(&selector)
        .consolidation(ConsolidationMode::None)
        .timeout(timeout)
        .with(flume::unbounded::<Reply>())
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            unread(
                key,
                &format!("the GET of {selector} could not be asked: {e}"),
            );
            return false;
        }
    };
    let iface = contract.iface.to_string();
    let mine = Fingerprint::of(contract).to_string();
    let run = run.to_string();
    let mut named: Vec<(Option<String>, String)> = Vec::new();
    let mut heard = false;
    let mut failed: Option<&str> = None;
    while let Ok(reply) = rx.recv_async().await {
        let Ok(sample) = reply.result() else {
            failed = Some("an error reply, as at its timeout");
            continue;
        };
        heard = true;
        let bytes = sample.payload().to_bytes();
        let d = std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| zenkey_model::descriptor::check(text, &[]).0);
        let Some(d) = d else {
            failed = Some("a descriptor that does not read");
            continue;
        };
        if let Some(e) = d.interfaces.iter().find(|e| e.iface == iface) {
            named.push((d.run.clone(), e.contract.clone()));
        }
    }
    if let Some((_, fp)) = named
        .iter()
        .find(|(r, _)| r.as_deref() == Some(run.as_str()))
    {
        return *fp == mine;
    }
    match (named.as_slice(), failed) {
        ([(None, fp)], None) => *fp == mine,
        (_, Some(why)) => {
            unread(key, why);
            false
        }
        ([], None) if !heard => true,
        _ => {
            unread(key, &format!("no descriptor names {iface} under run {run}"));
            false
        }
    }
}

/// Logs a descriptor read that failed: at warning level the first time,
/// then at debug.
fn unread(key: &str, why: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if LOGGED.swap(true, Ordering::Relaxed) {
        tracing::debug!(
            key,
            why,
            "the owner's revision is not known: no omission rule"
        );
    } else {
        tracing::warn!(
            key,
            why,
            "the owner's revision is not known: no omission rule applies over its snapshot (core §4.2)"
        );
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
