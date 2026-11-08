//! A state GET's answer read whole (core §4.2; #977's E3): what the
//! consumer's GET, its re-read and an archive's alignment share.
//!
//! - **The owner's revision maps the index** (F-497): a reader applies the
//!   omission rule over a complete snapshot with the revision of its
//!   interface that the owner's descriptor names (§3.3), never with
//!   another. A runtime reader holds one revision, its contract, so it
//!   asks the owner's descriptor stating the snapshot's run before it
//!   drops anything (`runs_revision`), and drops nothing when that names
//!   another revision. Where no descriptor answers, as across a
//!   constrained face, the revision the reader was configured with is the
//!   deployment's word (§8.6, R7). A read that failed, its timeout or a
//!   descriptor that does not read, is no such word: the reader drops
//!   nothing, and the entries still merge. A reader that holds several
//!   revisions, as a tool does, asks [`owner_revision`] which one.
//! - **A GET's selector** (`collection_read`): a reader that wants a
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
/// §4.2, "The owner's revision maps the index"), as [`owner_revision`]
/// reads its descriptors: `true` where one names `contract`'s fingerprint,
/// and where no descriptor reaches the reader, the deployment's word;
/// `false` where one names another, and where the read failed, so that no
/// omission rule applies and the entries still merge.
pub(crate) async fn runs_revision(
    session: &zenoh::Session,
    key: &str,
    run: Run,
    contract: &Contract,
    timeout: Duration,
) -> bool {
    match owner_revision(session, key, run, &contract.iface, timeout).await {
        OwnerRevision::Named(fp) => fp == Fingerprint::of(contract).to_string(),
        OwnerRevision::Unreached => true,
        OwnerRevision::Unread => false,
    }
}

/// Whether a `reply_del` on `key` under `run` is a stray one (core §4.2,
/// 0.102): a deletion on a member of a collection that the owner's revision
/// answers by snapshot ([`zenkey_model::collection::stray_reply_del`]).
/// Whether a resource is per key is read from the owner's revision, the one
/// its descriptor stating `run` names ([`owner_revision`]): `contract`
/// itself where it names it, else that revision retrieved by its
/// fingerprint (§8.4); where no descriptor reaches the reader, `contract`,
/// the deployment's word. Never from `contract` alone where the owner names
/// another: an owner on an earlier revision can answer per key a member a
/// later revision answers by snapshot, and its real `reply_del` is no
/// stray. A read that failed, or a revision that cannot be had, judges
/// nothing, so the `reply_del` merges as a deletion. `asked` holds each
/// owner's answer under each run, so a GET asks once.
pub(crate) async fn stray_reply_del(
    session: &zenoh::Session,
    key: &str,
    run: Run,
    contract: &Contract,
    timeout: Duration,
    asked: &mut std::collections::BTreeMap<(String, String), Option<std::sync::Arc<Contract>>>,
) -> bool {
    let owner = owner_of(key).map_or_else(String::new, |(prefix, addr)| format!("{prefix}{addr}"));
    let at = (owner, run.to_string());
    let theirs = match asked.get(&at) {
        Some(c) => c.clone(),
        None => {
            let c = match owner_revision(session, key, run, &contract.iface, timeout).await {
                OwnerRevision::Named(fp) if fp == Fingerprint::of(contract).to_string() => {
                    Some(std::sync::Arc::new(contract.clone()))
                }
                OwnerRevision::Named(fp) => retrieved(session, &contract.iface, &fp, timeout).await,
                OwnerRevision::Unreached => Some(std::sync::Arc::new(contract.clone())),
                OwnerRevision::Unread => None,
            };
            asked.insert(at, c.clone());
            c
        }
    };
    theirs.is_some_and(|c| zenkey_model::collection::stray_reply_del(&c, key))
}

/// The revision of `iface` whose fingerprint is `fp`, retrieved (§8.4);
/// `None` where it cannot be had.
async fn retrieved(
    session: &zenoh::Session,
    iface: &zenkey_model::grammar::IfaceId,
    fp: &str,
    timeout: Duration,
) -> Option<std::sync::Arc<Contract>> {
    let fp = Fingerprint::parse(fp).ok()?;
    match crate::retrieval::fetch_bundle(session, iface, &fp, timeout).await {
        Ok(crate::retrieval::Retrieved::Bundle(b, _)) => {
            Contract::from_bundle(&b).ok().map(std::sync::Arc::new)
        }
        _ => None,
    }
}

/// What the owner of a key's descriptors say of the revision of its
/// interface it runs under a run (core §4.2, "The owner's revision maps
/// the index"; §3.3): [`owner_revision`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerRevision {
    /// The fingerprint its descriptor stating the run names: the
    /// descriptor of the instance whose `run` is the snapshot's (core
    /// §4.2, 0.102; F-507).
    Named(String),
    /// No descriptor reaches the reader: the GET of its instances' keys ran
    /// to its final reply and none answered, as across a constrained face.
    /// The deployment's word then decides (§8.6, R7).
    Unreached,
    /// A read that failed (review-bot on #1086): a GET that could not be
    /// asked, an error reply (zenoh's at its timeout among them), a
    /// descriptor that does not read, or descriptors none of which names
    /// the interface under the run, a lone descriptor stating no run among
    /// them (core 0.102, F-507: §3.3 asks an owner to state one). No word:
    /// the reader cannot tell the revision, and applies no omission rule.
    /// The first is logged at warning level, the rest at debug.
    Unread,
}

/// The revision of `iface` the owner of `key` runs under `run`, as its
/// descriptors say ([`OwnerRevision`]). A key with a namespace in front,
/// as a session in no namespace hears one, asks its descriptors under that
/// namespace. What the consumer's GET and an archive's alignment map a
/// snapshot's index with, and a tool that holds several revisions picks
/// one by (#977's E4: the fleet's seed).
pub async fn owner_revision(
    session: &zenoh::Session,
    key: &str,
    run: Run,
    iface: &zenkey_model::grammar::IfaceId,
    timeout: Duration,
) -> OwnerRevision {
    let Some((prefix, addr)) = owner_of(key) else {
        return OwnerRevision::Unreached;
    };
    let selector = format!(
        "{prefix}zk2/{}/{}/@zk/instance/*",
        addr.system, addr.service
    );
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
            return OwnerRevision::Unread;
        }
    };
    let iface = iface.to_string();
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
        return OwnerRevision::Named(fp.clone());
    }
    // Descriptors reach the reader and none states the run: no omission
    // rule applies over that snapshot (core §4.2, 0.102).
    match (named.as_slice(), failed) {
        (_, Some(why)) => {
            unread(key, why);
            OwnerRevision::Unread
        }
        ([], None) if !heard => OwnerRevision::Unreached,
        _ => {
            unread(key, &format!("no descriptor names {iface} under run {run}"));
            OwnerRevision::Unread
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

/// The owner's address of a zk2 data key, and the namespace in front of
/// it, skipped: empty for a base-relative key, else ending in `/`.
fn owner_of(key: &str) -> Option<(&str, zenkey_model::grammar::Addr)> {
    let mut rest = Some(key);
    while let Some(k) = rest {
        if k.starts_with("zk2/")
            && let Ok(ZkKey::Data { addr, .. }) = parse(k)
        {
            return Some((&key[..key.len() - k.len()], addr));
        }
        rest = k.split_once('/').map(|(_, r)| r);
    }
    None
}
