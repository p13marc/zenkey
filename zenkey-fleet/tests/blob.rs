//! The `@blob` plane against real zenoh and a real blob server (issues #58,
//! #68).
//!
//! Three claims RFC 07 §2 makes that only a bus can check:
//!
//! 1. a probe **names every holder** and a fetch names **one** (§2.5);
//! 2. a disagreeing content root is named, never averaged (§2.1) — and a fetch
//!    from the disagreeing origin says *which* origin failed it;
//! 3. every `@blob` GET rides at **data-low** (§2.6) — asserted off the wire,
//!    by a queryable that reads `Query::priority()`, rather than by trusting
//!    the caller's own report of itself.
//!
//! Self-contained like `querier.rs`: in-process peers, explicit endpoints, no
//! scouting, no external router. Ports 7509-7512 (disjoint from every other
//! test binary).
//!
//! Needs the `blob` feature. `cargo test --workspace` unifies it in through
//! zenctl/zengui; `cargo test -p zenkey-fleet --features blob` runs it alone.

#![cfg(feature = "blob")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey_fleet::zblob::{self, BlobServer, BlobSpec, MemoryBlobSource};
use zenkey_fleet::{BlobFetchSpec, BlobTarget, blob_fetch, blob_probe};
use zenoh::qos::Priority;

/// The demo id, lowercase — a canonical ULID is uppercase Crockford base32 and
/// has no spelling in a key chunk (RFC 03 §2, RFC 07 §2.2 as amended in v1.11).
const ID: &str = "01jqz3demo0001";
const BASE: &str = "";
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

/// Deterministic bytes, so a byte-identical assertion means something.
fn payload(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) as u8
        })
        .collect()
}

fn target() -> BlobTarget {
    BlobTarget::parse(ID).expect("the demo id is a plain chunk")
}

/// The tier-1 prefix under one origin, spelled the way the engine spells it.
fn prefix_at(origin: &str) -> String {
    let origin = zenkey::grammar::Origin::Host(zenkey::HostId::parse(origin).unwrap());
    zenkey::grammar::blob_tier_prefix(&origin, zenkey::grammar::BlobTier::Artifact)
        .as_str()
        .to_string()
}

/// Wait until `key` answers, or give up after ~5s.
///
/// A peer pair takes a moment to connect, and a probe issued before it has
/// answers nothing — which is indistinguishable from "nobody holds it" and
/// would make every assertion below pass or fail on the scheduler's mood. So
/// the fixture proves itself routable, with a plain `fleet_get`, before any
/// claim is made about it.
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

/// Serve `data` as artifact [`ID`] under `origin`, and return the handle plus
/// the manifest (whose root is what a downloader should pin).
async fn serve(
    session: &zenoh::Session,
    origin: &str,
    data: Vec<u8>,
) -> (zblob::ServerHandle, zblob::Manifest) {
    let server = BlobServer::new(
        session,
        zblob::ServePrefix::new(prefix_at(origin)).expect("concrete prefix"),
    );
    let manifest = server
        .register_source(
            BlobSpec::new(ID).filename("demo-bundle.bin"),
            Arc::new(MemoryBlobSource::new(data)),
        )
        .await
        .expect("register");
    (server.spawn().await.expect("spawn"), manifest)
}

/// A probe must name **every** origin that answered, and a fetch must name the
/// one it was pointed at — RFC 07 §2.5's whole shape, and RFC 05 §2.1's
/// attribution-by-reply-key underneath it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_probe_names_every_holder_and_a_fetch_names_one() {
    let (serving, asking) = peer_pair(7509).await;
    let data = payload(200_000, 7);
    let (a, manifest) = serve(&serving, "h-aaaaaaaaaaaa", data.clone()).await;
    let (b, _) = serve(&serving, "h-bbbbbbbbbbbb", data.clone()).await;
    for origin in ["h-aaaaaaaaaaaa", "h-bbbbbbbbbbbb"] {
        wait_routable(&asking, &zblob::keys::manifest_key(&prefix_at(origin), ID)).await;
    }

    let report = blob_probe(&asking, BASE, &target(), &[], TIMEOUT)
        .await
        .expect("probe");

    // Two holders, distinct origins — attributed by each reply's own key, not
    // by the `*` we asked on.
    let origins: Vec<&str> = report.holders.iter().map(|h| h.origin.as_str()).collect();
    assert_eq!(
        origins,
        vec!["h-aaaaaaaaaaaa", "h-bbbbbbbbbbbb"],
        "a probe names every holder: {report:?}"
    );
    // O5: the coverage claim is exactly the selectors asked, and no wider.
    assert_eq!(
        report.asked.len(),
        2,
        "have and manifest: {:?}",
        report.asked
    );
    assert!(report.asked.iter().any(|s| s.ends_with("/have")));
    assert!(report.asked.iter().any(|s| s.ends_with("/manifest")));
    assert!(report.not_probed.is_none());

    // Both hold the same bytes, so there is one root and nothing to choose
    // between.
    assert_eq!(report.roots, vec![manifest.root.to_string()]);
    for holder in &report.holders {
        let avail = holder.availability.as_ref().expect("a have reply");
        assert!(avail.complete, "a full server answers all-ones: {avail:?}");
        assert_eq!(
            holder.manifest.as_ref().map(|m| m.total_len),
            Some(data.len() as u64)
        );
        assert!(holder.unreadable.is_none(), "{:?}", holder.unreadable);
        // The concrete key is what makes the next step addressable.
        assert!(holder.key.contains(&holder.origin), "{}", holder.key);
    }

    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("demo.bin");
    let spec = BlobFetchSpec {
        timeout: TIMEOUT,
        root: Some(zenkey::ContentHash::parse(&manifest.root.to_string()).unwrap()),
        ..Default::default()
    };
    let fetched = blob_fetch(
        &asking,
        BASE,
        "h-bbbbbbbbbbbb",
        &target(),
        &dest,
        &spec,
        &|_| {},
    )
    .await
    .expect("fetch");

    assert_eq!(
        fetched.origin, "h-bbbbbbbbbbbb",
        "one origin, the chosen one"
    );
    assert_eq!(fetched.rejected, 0, "nothing failed verification");
    assert!(fetched.root_pinned, "the root was pinned, not TOFU");
    assert_eq!(fetched.priority, "data-low");
    assert_eq!(
        std::fs::read(&dest).unwrap(),
        data,
        "the bytes on disk are the bytes served"
    );

    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

/// Two origins claiming one id at two different roots is a **finding**, not a
/// tie-break (RFC 07 §2.1: the id is a name, the root is the anchor) — and a
/// fetch from the wrong one must name it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disagreeing_root_is_named_not_averaged() {
    let (serving, asking) = peer_pair(7510).await;
    let honest = payload(120_000, 11);
    let rogue = payload(120_000, 12);
    let (a, manifest) = serve(&serving, "h-aaaaaaaaaaaa", honest.clone()).await;
    let (c, rogue_manifest) = serve(&serving, "h-cccccccccccc", rogue).await;
    for origin in ["h-aaaaaaaaaaaa", "h-cccccccccccc"] {
        wait_routable(&asking, &zblob::keys::manifest_key(&prefix_at(origin), ID)).await;
    }
    assert_ne!(
        manifest.root, rogue_manifest.root,
        "the fixture must disagree"
    );

    let report = blob_probe(&asking, BASE, &target(), &[], TIMEOUT)
        .await
        .expect("probe");
    assert_eq!(report.holders.len(), 2);
    assert_eq!(
        report.roots.len(),
        2,
        "two roots under one id, both reported: {:?}",
        report.roots
    );

    // Fetch from the rogue, pinning the honest root: every slice is verified
    // against that root before disk, so this must fail — naming the origin.
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("demo.bin");
    let spec = BlobFetchSpec {
        timeout: TIMEOUT,
        root: Some(zenkey::ContentHash::parse(&manifest.root.to_string()).unwrap()),
        ..Default::default()
    };
    let err = blob_fetch(
        &asking,
        BASE,
        "h-cccccccccccc",
        &target(),
        &dest,
        &spec,
        &|_| {},
    )
    .await
    .expect_err("a pinned root must refuse the wrong content")
    .to_string();

    assert!(
        err.contains("h-cccccccccccc"),
        "the failure must name the origin that produced it: {err}"
    );
    assert!(
        !dest.exists(),
        "verification happens before disk (RFC 07 §2.1), so nothing was written"
    );

    a.shutdown().await.unwrap();
    c.shutdown().await.unwrap();
}

/// RFC 07 §2.6 says the **caller** must issue `@blob` GETs at data-low, because
/// replies inherit the query's QoS and a server-side setter is a no-op. This
/// reads the priority off the wire — the only place the claim is checkable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_blob_get_rides_at_data_low() {
    let (serving, asking) = peer_pair(7511).await;
    let seen: Arc<Mutex<Vec<Priority>>> = Arc::new(Mutex::new(Vec::new()));

    // A manifest for a blob nobody serves: enough for the client to move on to
    // slice queries, which is the second GET shape we want to observe.
    let manifest = zblob::Manifest {
        version: zblob::wire::WIRE_VERSION,
        id: zblob::BlobId::new(ID).expect("valid id"),
        filename: None,
        total_len: 65_536 * 4,
        chunk_size: 65_536,
        root: zblob::Hash::of(b"not the content"),
        created_ms: 0,
        ext: zblob::wire::Ext::default(),
    };
    let manifest_bytes = zblob::wire::encode(&manifest).unwrap();

    let prefix = prefix_at("h-dddddddddddd");
    // Replies go on the responder's own **concrete** key, exactly as a real
    // blob server does — which is what makes attribution-by-reply-key work
    // under a `*`-origin probe.
    let manifest_key = zblob::keys::manifest_key(&prefix, ID);

    let cancel = zblob::CancelToken::new();
    let recorder = {
        let seen = seen.clone();
        let cancel = cancel.clone();
        let manifest_key = manifest_key.clone();
        serving
            .declare_queryable(format!("{prefix}/**"))
            .callback(move |query| {
                seen.lock().unwrap().push(query.priority());
                if query.key_expr().as_str().ends_with("/manifest")
                    || query.key_expr().as_str().ends_with("/**")
                {
                    let bytes = manifest_bytes.clone();
                    let key = manifest_key.clone();
                    tokio::spawn(async move {
                        let _ = query
                            .reply(key, bytes)
                            .encoding(&zblob::wire::ENC_MANIFEST)
                            .await;
                    });
                } else {
                    // A slice query observed is all this test needs; stop the
                    // transfer rather than waiting out the retry budget.
                    cancel.cancel();
                }
            })
            .await
            .expect("recording queryable")
    };

    // Here the settle matters twice over: "nothing answered" would pass an
    // all-data-low assertion vacuously.
    wait_routable(&asking, &manifest_key).await;
    // Those settle GETs rode at the default priority, on purpose — they are
    // not @blob traffic under test. Forget them before the real assertions.
    seen.lock().unwrap().clear();

    let probe = blob_probe(&asking, BASE, &target(), &[], Duration::from_secs(2))
        .await
        .expect("probe");
    assert_eq!(probe.answered, 1, "the recorder answered the manifest GET");

    let dir = tempfile::tempdir().unwrap();
    let spec = BlobFetchSpec {
        timeout: Duration::from_secs(2),
        cancel,
        ..Default::default()
    };
    // The fetch cannot succeed — nobody serves the slices — and that is fine:
    // the assertion is about the QoS of the GETs it issued on the way.
    let _ = tokio::time::timeout(
        Duration::from_secs(30),
        blob_fetch(
            &asking,
            BASE,
            "h-dddddddddddd",
            &target(),
            &dir.path().join("demo.bin"),
            &spec,
            &|_| {},
        ),
    )
    .await;

    let priorities = seen.lock().unwrap().clone();
    assert!(
        priorities.len() >= 3,
        "expected have + manifest + at least one slice query, saw {}",
        priorities.len()
    );
    assert!(
        priorities.iter().all(|p| *p == Priority::DataLow),
        "every @blob GET must ride at data-low (RFC 07 §2.6): {priorities:?}"
    );

    recorder.undeclare().await.unwrap();
}

/// RFC 07 §2.4/§2.5 (v1.17): the content-addressed tiers have a probe now —
/// `store/<algo>/have` — so a store target is *asked*, and an empty bus is
/// honestly "asked, nobody answered", not "not probed". The one refusal left
/// is an algorithm this build's reference client does not speak, and that one
/// must still say so rather than report zero holders.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tier_two_is_probed_and_a_foreign_algo_says_why_not() {
    let (_serving, asking) = peer_pair(7512).await;
    let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    // blake3 is speakable: the probe asks the §2.4 have endpoint. Nobody
    // serves it on this bus, so the verdict is an honest empty holder list —
    // with the asked selector recorded, per O5.
    let target = BlobTarget::parse(&format!("store/blake3/{hash}")).unwrap();
    let report = blob_probe(&asking, BASE, &target, &[], Duration::from_secs(1))
        .await
        .expect("probe");
    assert!(
        report.not_probed.is_none(),
        "blake3 must be probed: {report:?}"
    );
    assert_eq!(
        report.asked,
        vec!["v1/*/@blob/store/blake3/have".to_string()],
        "the §2.4 probe key under the empty base, and no wider"
    );
    assert!(report.holders.is_empty());

    // A foreign algo cannot be asked by this build, and must say so — O4:
    // "not asked" must never be readable as "nobody holds it".
    let target = BlobTarget::parse(&format!("store/sha256/{hash}")).unwrap();
    let report = blob_probe(&asking, BASE, &target, &[], Duration::from_secs(1))
        .await
        .expect("probe");
    assert!(report.asked.is_empty(), "nothing was asked");
    assert!(report.holders.is_empty());
    let why = report.not_probed.expect("an unasked probe must say why");
    assert!(why.contains("blake3"), "{why}");
    assert!(why.contains("2.4"), "{why}");
}
