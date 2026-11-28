//! Core §3.3's compact descriptor, read beside the JSON one (#980's C5b):
//! the runtime's readers, `presence::serving` through a consumer,
//! `answer::owner_revision_held` and the retrieval of a revision a compact
//! descriptor names (§8.4). Each owner is `common::compact_owner`'s, a raw
//! session, so both forms answer one descriptor; the runtime's own writer
//! (C5c) is `constrained.rs`' §10 step 7.
//!
//! A reader tells the forms apart by the reply's `Encoding` alone, reads a
//! compact one through the catalogs it holds, and without one reads none
//! of it (`unknown_catalog`), never a guess (§8.6).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::compact_owner::{self, RUN};
use common::{T, client, contract, eventually, router};
use zenkey::answer::{OwnerRevision, owner_revision, owner_revision_held};
use zenkey::consumer::Consumer;
use zenkey::model::canonical::Fingerprint;
use zenkey::model::catalog::Catalog;
use zenkey::model::contract::Contract;
use zenkey::model::descriptor::Form;
use zenkey::model::grammar::{Addr, InstanceId};
use zenkey::model::wire::Run;
use zenkey::presence::{self, Found, Held};

const JSON: &str = "lab/json";
const COMPACT: &str = "lab/compact";
const INSTANCE: &str = "00000000000000c5";

fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

fn instance() -> InstanceId {
    INSTANCE.parse().expect("an instance id")
}

/// `nav.v2` (one capability gate, `imu`) and `ping.v1` (none).
fn revisions() -> (Contract, Contract) {
    (contract("nav.v2"), contract("ping.v1"))
}

/// The two owners, each listing `revisions` tokenless with `imu` held: one
/// answering the JSON form, one the compact form. The catalog lists both
/// revisions.
async fn owners(
    s: &zenoh::Session,
    revisions: &[&Contract],
) -> (Catalog, compact_owner::Owner, compact_owner::Owner) {
    let catalog = compact_owner::catalog(revisions);
    let d = |a| compact_owner::descriptor(a, INSTANCE, &catalog, revisions, false, &["imu"], None);
    let json = compact_owner::serve(s, &d(JSON), Form::Json, Vec::new()).await;
    let bytes = compact_owner::compact(&d(COMPACT), &catalog, revisions);
    let compact = compact_owner::serve(s, &d(COMPACT), Form::Compact, bytes).await;
    (catalog, json, compact)
}

/// One GET each: the JSON form reads with nothing held; the compact one
/// reads through the catalog and its revisions to the same members, and
/// holding no catalog it is not read, `unknown_catalog`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_compact_descriptor_reads_through_its_catalog_beside_a_json_one() {
    let (_r, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (nav, ping) = revisions();
    let (catalog, _json, _compact) = owners(&owner, &[&nav, &ping]).await;
    let held = Held::new(
        Arc::new(vec![catalog.clone()]),
        vec![Arc::new(nav.clone()), Arc::new(ping.clone())],
    );
    let json = eventually_found(&tool, JSON, &Held::default()).await;
    let compact = eventually_found(&tool, COMPACT, &held).await;
    let (Found::Descriptor(j, _), Found::Descriptor(c, bytes)) = (json, compact) else {
        panic!("both read");
    };
    assert!(
        bytes.len() < 30,
        "the compact form's bytes: {}",
        bytes.len()
    );
    assert_eq!(c.service, COMPACT);
    assert_eq!(c.instance, INSTANCE);
    assert_eq!(c.run.as_deref(), Some(RUN));
    assert_eq!(c.catalog, Some(catalog.id().hex()));
    assert_eq!(c.capabilities, j.capabilities);
    assert_eq!(c.interfaces.len(), 2);
    for (c, j) in c.interfaces.iter().zip(&j.interfaces) {
        assert_eq!(
            (&c.iface, &c.contract, c.token, c.standby),
            (&j.iface, &j.contract, j.token, j.standby)
        );
    }
    // Holding no catalog: the JSON form still reads, the compact one does not.
    let none = presence::descriptor(&tool, &addr(COMPACT), &instance(), T)
        .await
        .unwrap();
    match none {
        Found::Unread(why) => assert!(why.starts_with("unknown_catalog"), "{why}"),
        other => panic!("not read without a catalog: {other:?}"),
    }
    // Holding the catalog and not `ping.v1`, retrieving nothing: its bitmap
    // has no width to read by (`unknown_revision`).
    let lacking = Held::new(Arc::new(vec![catalog]), vec![Arc::new(nav)]);
    match presence::descriptor_held(&tool, &addr(COMPACT), &instance(), &lacking, T)
        .await
        .unwrap()
    {
        Found::Unread(why) => assert!(why.starts_with("unknown_revision"), "{why}"),
        other => panic!("a revision not held: {other:?}"),
    }
}

/// A revision the compact descriptor names and the reader lacks is
/// retrieved from its holder (§8.4) before the rest is read; with no
/// holder it cannot be had, and the descriptor is not read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revision_the_reader_lacks_is_retrieved_first() {
    let (_r, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (nav, ping) = revisions();
    let (catalog, _json, mut compact) = owners(&owner, &[&nav, &ping]).await;
    let held = Held::new(Arc::new(vec![catalog]), vec![Arc::new(nav.clone())]).retrieving();
    eventually("the instance's descriptor answers", || async {
        !matches!(
            presence::descriptor_held(&tool, &addr(COMPACT), &instance(), &held, T).await,
            Ok(Found::Nothing)
        )
    })
    .await;
    match presence::descriptor_held(&tool, &addr(COMPACT), &instance(), &held, T)
        .await
        .unwrap()
    {
        Found::Unread(why) => assert!(why.starts_with("unknown_revision"), "{why}"),
        other => panic!("no holder of ping.v1: {other:?}"),
    }
    compact.hold(&ping).await;
    let named = vec![(ping.iface.clone(), Fingerprint::of(&ping))];
    eventually("ping.v1 is retrieved", || async {
        zenkey::retrieval::revisions(&tool, &named, &[Arc::new(nav.clone())], T)
            .await
            .len()
            == 1
    })
    .await;
    assert!(
        zenkey::retrieval::revisions(&tool, &named, &[Arc::new(ping.clone())], T)
            .await
            .is_empty(),
        "a revision held is not retrieved"
    );
    match presence::descriptor_held(&tool, &addr(COMPACT), &instance(), &held, T)
        .await
        .unwrap()
    {
        Found::Descriptor(d, _) => assert_eq!(d.interfaces[1].iface, "ping.v1"),
        other => panic!("read once ping.v1 is retrieved: {other:?}"),
    }
}

/// `presence::serving`, through a consumer of `nav.v2` bound to both
/// owners, each tokenless: holding the catalog, both serve; holding none,
/// the compact one's address is undescribed, `unknown_catalog` its reason,
/// and the read not complete (observer §3.6), never absent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serving_reads_a_tokenless_provider_by_its_compact_descriptor() {
    let (_r, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (nav, ping) = revisions();
    let (catalog, _json, mut compact) = owners(&owner, &[&nav, &ping]).await;
    compact.hold(&ping).await;
    let consumer = |c: Option<&Catalog>| {
        let k =
            Consumer::for_tool(&tool, Arc::new(nav.clone()), &["lab/*"], &BTreeMap::new()).unwrap();
        match c {
            Some(c) => k.with_catalogs([c.clone()]),
            None => k,
        }
    };
    let with = consumer(Some(&catalog));
    eventually(
        "both serve, the compact one read through the catalog",
        || async {
            with.presence(T)
                .await
                .is_ok_and(|p| p.complete && p.providers == vec![addr(COMPACT), addr(JSON)])
        },
    )
    .await;
    let without = consumer(None);
    let p = without.presence(T).await.unwrap();
    assert_eq!(p.providers, vec![addr(JSON)], "{p:?}");
    assert_eq!(p.undescribed, vec![addr(COMPACT)], "{p:?}");
    assert!(
        !p.complete,
        "a descriptor not read leaves the read not complete"
    );
    assert!(
        p.errors
            .iter()
            .any(|e| e.contains(COMPACT) && e.contains("not read: unknown_catalog")),
        "{:?}",
        p.errors
    );
}

/// `answer::owner_revision_held`: the descriptor stating the snapshot's
/// run names the revision, in either form; a compact one read through no
/// catalog is no word (`Unread`), never a guess.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_owners_revision_is_read_from_either_form() {
    let (_r, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (nav, ping) = revisions();
    let (catalog, _json, mut compact) = owners(&owner, &[&nav, &ping]).await;
    compact.hold(&ping).await;
    let run = Run::from_hex(RUN).expect("a run");
    let fp = Fingerprint::of(&nav).to_string();
    let key = |a: &str| format!("zk2/{a}/nav.v2/state/pose");
    let held = Held::new(Arc::new(vec![catalog]), vec![Arc::new(nav.clone())]).retrieving();
    eventually("the JSON owner's revision is named", || async {
        owner_revision(&tool, &key(JSON), run, &nav.iface, T).await
            == OwnerRevision::Named(fp.clone())
    })
    .await;
    eventually("the compact owner's revision is named", || async {
        owner_revision_held(&tool, &key(COMPACT), run, &nav.iface, &held, T).await
            == OwnerRevision::Named(fp.clone())
    })
    .await;
    assert_eq!(
        owner_revision(&tool, &key(COMPACT), run, &nav.iface, T).await,
        OwnerRevision::Unread,
        "holding no catalog, the compact descriptor is no word"
    );
    let other = Run::from_hex("0008").expect("a run");
    assert_eq!(
        owner_revision_held(&tool, &key(COMPACT), other, &nav.iface, &held, T).await,
        OwnerRevision::Unread,
        "no descriptor states run 0008"
    );
}

/// The descriptor of `address`'s instance, once one answers.
async fn eventually_found(tool: &zenoh::Session, address: &str, held: &Held) -> Found {
    let (a, i) = (addr(address), instance());
    let get = || presence::descriptor_held(tool, &a, &i, held, T);
    eventually("the descriptor answers", || async {
        !matches!(get().await, Ok(Found::Nothing))
    })
    .await;
    get().await.unwrap()
}

/// The runtime's other readers (#980 C5b2): a `Client` given the catalog
/// finds a tokenless provider by its compact descriptor, one given none
/// leaves it undescribed; `presence::heard` reads a compact descriptor's
/// put by its form, and `Found::zid` its `meta.zid`, which `timing` and
/// `arbitration` attribute a stamp by.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_and_a_put_read_the_compact_form() {
    use zenkey::Client;
    let (_r, ep) = router(None).await;
    let (owner, tool) = (client(&ep).await, client(&ep).await);
    let (nav, ping) = revisions();
    let revs = [&nav, &ping];
    let catalog = compact_owner::catalog(&revs);
    let d = compact_owner::descriptor(
        COMPACT,
        INSTANCE,
        &catalog,
        &revs,
        false,
        &["imu"],
        Some("a1b2"),
    );
    let bytes = compact_owner::compact(&d, &catalog, &revs);
    // The put first, heard by a subscriber holding the catalog and both
    // revisions, and by one holding no catalog.
    let held = Held::new(
        Arc::new(vec![catalog.clone()]),
        vec![Arc::new(nav.clone()), Arc::new(ping.clone())],
    );
    type Heard = Vec<(bool, Option<u128>)>;
    let heard: Arc<std::sync::Mutex<Heard>> = Arc::default();
    let h = Arc::clone(&heard);
    let _sub = tool
        .declare_subscriber("zk2/lab/*/@zk/instance/*")
        .callback(move |s| {
            for (with, held) in [(true, &held), (false, &Held::default())] {
                if let Some((_, _, found)) = presence::heard(&s, held) {
                    h.lock().unwrap().push((with, found.zid()));
                }
            }
        })
        .await
        .unwrap();
    let key = format!("zk2/{COMPACT}/@zk/instance/{INSTANCE}");
    eventually("the put is heard both ways", || async {
        owner
            .put(key.as_str(), bytes.clone())
            .encoding(zenoh::bytes::Encoding::ZENOH_BYTES)
            .await
            .unwrap();
        heard.lock().unwrap().len() >= 2
    })
    .await;
    let got = heard.lock().unwrap().clone();
    assert!(got.contains(&(true, Some(0xa1b2))), "{got:?}");
    assert!(got.contains(&(false, None)), "never a guess: {got:?}");

    let mut compact = compact_owner::serve(&owner, &d, Form::Compact, bytes).await;
    compact.hold(&ping).await;
    let with = Client::new(&tool, Arc::new(nav.clone()), &["lab/*"])
        .unwrap()
        .with_catalogs([catalog]);
    eventually(
        "the client finds the provider by its compact descriptor",
        || async {
            with.presence(T)
                .await
                .is_ok_and(|p| p.complete && p.providers == vec![addr(COMPACT)])
        },
    )
    .await;
    let without = Client::new(&tool, Arc::new(nav), &["lab/*"]).unwrap();
    let p = without.presence(T).await.unwrap();
    assert!(p.providers.is_empty(), "{p:?}");
    assert_eq!(p.undescribed, vec![addr(COMPACT)], "{p:?}");
    let fleet = with.fleet();
    eventually("its fleet keeps the catalog", || async {
        fleet
            .presence(T)
            .await
            .is_ok_and(|p| p.providers == vec![addr(COMPACT)])
    })
    .await;
}
