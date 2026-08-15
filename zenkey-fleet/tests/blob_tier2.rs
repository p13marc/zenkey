//! Tier 2 against real zenoh and a real tree server (RFC 07 §§2.3–2.5 as
//! amended in v1.17; zenkey #111).
//!
//! Three claims only a bus can check:
//!
//! 1. the Tier-2 probes are **possession verdicts**: a holder's bitfield and
//!    tree counts match what it actually has, attributed by its own prefix;
//! 2. a store fetch is **verified against the address** — a holder serving
//!    the wrong bytes is rejected, and the rejection names the origin;
//! 3. a tree target is **inspectable without a content store**: the summary
//!    validates against the root the caller asked for.
//!
//! Self-contained like `blob.rs`: in-process peers, explicit endpoints, no
//! scouting. Ports 7530-7531 (disjoint from every other test binary).

#![cfg(feature = "blob")]

use std::sync::Arc;
use std::time::Duration;

use zenkey_fleet::zblob::{self, ContentStore, MemoryStore, TreeServer, build_tree};
use zenkey_fleet::{BlobFetchSpec, BlobTarget, blob_fetch, blob_probe, blob_tree_index};

const BASE: &str = "";
const ORIGIN: &str = "h-aaaaaaaaaaaa";
const TIMEOUT: Duration = Duration::from_secs(5);

async fn peer_pair(port: u16) -> (zenoh::Session, zenoh::Session) {
    let listen = zenkey_fleet::session::open(&[], &[format!("tcp/127.0.0.1:{port}")], false)
        .await
        .expect("listener session");
    let connect = zenkey_fleet::session::open(&[format!("tcp/127.0.0.1:{port}")], &[], false)
        .await
        .expect("connector session");
    (listen, connect)
}

fn tier_prefix(origin: &str, tier: zenkey::grammar::BlobTier) -> String {
    let origin = zenkey::grammar::Origin::Host(zenkey::HostId::parse(origin).unwrap());
    zenkey::grammar::blob_tier_prefix(&origin, tier)
        .as_str()
        .to_string()
}

/// Same settle discipline as `blob.rs`: prove the fixture routable before
/// asserting anything, or "nobody answered" and "not connected yet" are
/// indistinguishable and the test rides the scheduler's mood.
async fn wait_routable(session: &zenoh::Session, key: &str) {
    for _ in 0..50 {
        let answers = zenkey_fleet::fleet_get(session, BASE, key, None, TIMEOUT)
            .await
            .expect("settle get");
        if !answers.is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("fixture never became routable: {key}");
}

/// A small on-disk tree with enough content to span several chunks.
fn make_tree(dir: &std::path::Path) -> Vec<u8> {
    let mut big = Vec::with_capacity(96 * 1024);
    let mut state = 11u64;
    for _ in 0..96 * 1024 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        big.push((state >> 33) as u8);
    }
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("big.bin"), &big).unwrap();
    std::fs::write(dir.join("sub/note.txt"), b"a small file").unwrap();
    big
}

/// One server holding a snapshot; probes report possession, a store fetch
/// verifies against the address, and the tree summary needs no store.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tier_two_probes_are_possession_verdicts_and_fetches_verify() {
    let (serving, asking) = peer_pair(7530).await;
    let src = tempfile::tempdir().unwrap();
    make_tree(src.path());

    let store: Arc<dyn ContentStore> = Arc::new(MemoryStore::new());
    // Small CDC bounds so the fixture spans several chunks; keyed by its own
    // root, which is what makes `tree/<root>` answer (RFC 07 §2.3).
    let cdc = zblob::CdcParams {
        min: 2048,
        avg: 8192,
        max: 32768,
        normalization: 2,
        gear_seed: 0,
    };
    let index = build_tree(src.path(), "snap", &cdc, &*store)
        .unwrap()
        .keyed_by_root();
    let root_hex = index.root_hash.to_string();
    let chunk = index.needed_chunk_refs()[0].clone();
    let (entries, files, total_size, chunks) = (
        index.entries().len(),
        index.file_count(),
        index.total_size(),
        index.needed_chunk_refs().len(),
    );

    let server = TreeServer::new(
        &serving,
        zblob::ServePrefix::new(tier_prefix(ORIGIN, zenkey::grammar::BlobTier::Store)).unwrap(),
        zblob::ServePrefix::new(tier_prefix(ORIGIN, zenkey::grammar::BlobTier::Tree)).unwrap(),
        store,
    );
    server.register(index).await.unwrap();
    let handle = server.spawn().await.unwrap();
    wait_routable(
        &asking,
        &zblob::keys::tree_have_key(
            &tier_prefix(ORIGIN, zenkey::grammar::BlobTier::Tree),
            &root_hex,
        ),
    )
    .await;

    // The tree probe: a possession verdict, not a capability claim.
    let target = BlobTarget::parse(&format!("tree/{root_hex}")).unwrap();
    let report = blob_probe(&asking, BASE, &target, &[], TIMEOUT)
        .await
        .expect("tree probe");
    assert!(report.not_probed.is_none(), "{report:?}");
    assert_eq!(report.holders.len(), 1, "{report:?}");
    let holder = &report.holders[0];
    assert_eq!(
        holder.origin, ORIGIN,
        "attributed by the holder's own prefix"
    );
    let avail = holder.availability.as_ref().expect("a tree-probe verdict");
    assert!(avail.complete, "index + every chunk: {avail:?}");
    assert_eq!(avail.chunk_count as usize, chunks);

    // The store probe: one bit per asked address, per holder.
    let target = BlobTarget::parse(&format!("store/blake3/{}", chunk.hash)).unwrap();
    let report = blob_probe(&asking, BASE, &target, &[], TIMEOUT)
        .await
        .expect("store probe");
    assert_eq!(report.holders.len(), 1, "{report:?}");
    let avail = report.holders[0].availability.as_ref().unwrap();
    assert!(avail.complete, "the holder holds the asked chunk");

    // An address nobody holds: the same holder answers, and the verdict is an
    // honest zero — "asked, and it does not have it", not silence.
    let absent = zblob::Hash::of(b"content nobody published");
    let target = BlobTarget::parse(&format!("store/blake3/{absent}")).unwrap();
    let report = blob_probe(&asking, BASE, &target, &[], TIMEOUT)
        .await
        .expect("absent-chunk probe");
    assert_eq!(report.holders.len(), 1, "{report:?}");
    let avail = report.holders[0].availability.as_ref().unwrap();
    assert!(!avail.complete && avail.have == 0, "{avail:?}");

    // The store fetch: verified bytes on disk, pinned by construction.
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("chunk.bin");
    let target = BlobTarget::parse(&format!("store/blake3/{}", chunk.hash)).unwrap();
    let fetched = blob_fetch(
        &asking,
        BASE,
        ORIGIN,
        &target,
        &dest,
        &BlobFetchSpec {
            timeout: TIMEOUT,
            ..Default::default()
        },
        &|_| {},
    )
    .await
    .expect("store fetch");
    assert!(fetched.root_pinned, "a store fetch cannot be TOFU");
    assert_eq!(fetched.chunks, 1);
    let bytes = std::fs::read(&dest).unwrap();
    assert_eq!(
        zblob::Hash::of(&bytes),
        chunk.hash,
        "the bytes on disk verify against the address"
    );

    // Overwrite::Refuse semantics, tier-2 edition: an existing destination is
    // refused before a byte is fetched, and saying so is the report's job.
    let err = blob_fetch(
        &asking,
        BASE,
        ORIGIN,
        &target,
        &dest,
        &BlobFetchSpec {
            timeout: TIMEOUT,
            ..Default::default()
        },
        &|_| {},
    )
    .await
    .expect_err("an existing destination is refused without overwrite");
    assert!(err.to_string().contains("already exists"), "{err}");

    // The tree summary: validated against the root, no content store, and
    // the numbers are the index's own.
    let root = zenkey::ContentHash::parse(&root_hex).unwrap();
    let summary = blob_tree_index(&asking, BASE, ORIGIN, &root, TIMEOUT)
        .await
        .expect("tree summary");
    assert_eq!(summary.origin, ORIGIN);
    assert_eq!(
        (
            summary.entries,
            summary.files,
            summary.total_size,
            summary.chunks
        ),
        (entries, files, total_size, chunks)
    );

    handle.shutdown().await.unwrap();
}

/// A holder serving the wrong bytes under a store key is rejected before the
/// destination file exists — and the rejection names the origin, because an
/// unattributed verification failure is unactionable (RFC 07 §2.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lying_store_holder_is_rejected_by_name() {
    let (serving, asking) = peer_pair(7531).await;
    let honest = zblob::Hash::of(b"the content the address names");
    let store_prefix = tier_prefix(ORIGIN, zenkey::grammar::BlobTier::Store);
    let key = zblob::keys::store_key(&store_prefix, zblob::HashAlgo::Blake3, &honest);

    // A well-formed container carrying the wrong content: it decodes and
    // unframes cleanly, so the only thing standing between it and the disk is
    // per-reply verification against the address.
    let wrong =
        zblob::frame_chunk(b"something else entirely", zblob::ChunkCompression::None).unwrap();
    let reply_key = key.clone();
    let _liar = serving
        .declare_queryable(key.clone())
        .callback(move |query| {
            let bytes = wrong.clone();
            let key = reply_key.clone();
            tokio::spawn(async move {
                let _ = query
                    .reply(key, bytes)
                    .encoding(&zblob::wire::ENC_CHUNK)
                    .await;
            });
        })
        .await
        .expect("liar queryable");
    wait_routable(&asking, &key).await;

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("chunk.bin");
    let target = BlobTarget::parse(&format!("store/blake3/{honest}")).unwrap();
    let err = blob_fetch(
        &asking,
        BASE,
        ORIGIN,
        &target,
        &dest,
        &BlobFetchSpec {
            timeout: Duration::from_secs(2),
            ..Default::default()
        },
        &|_| {},
    )
    .await
    .expect_err("wrong bytes must not verify");
    assert!(
        err.to_string().contains(ORIGIN),
        "the failure names the origin: {err}"
    );
    assert!(!dest.exists(), "nothing unverified reaches the destination");
}
