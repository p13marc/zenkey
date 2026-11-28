//! An archive reply's attachment in its two forms (core §4.4; #980 C3), as
//! the fleet's readers read it: the doctor's `archive-unaligned`, the
//! last-known state report, and `why`'s last-known rung.
//!
//! Two runtime archives record one owner's state on one router: one under
//! the deployment catalog (`Archive::start_under`), whose replies carry the
//! compact attachment, 3 bytes, and one given none, whose replies carry the
//! JSON form. Each reader reads `confirmed` from either; the state report
//! reads the compact one's identity through the catalog given, and given
//! none states `confirmed` and no identity, never a guess.

use std::time::Duration;

mod util;
use util::zk2::{T, addr, client, config, eventually, example, iface, router};

use zenkey::archive::{Archive, ArchiveConfig, Recorded};
use zenkey::model::template::Bindings;
use zenkey::{Implementation, ServiceBuilder};
use zenkey_fleet::model::catalog::Revision;
use zenkey_fleet::report::{CheckId, ContractSource, Judgement};
use zenkey_fleet::{BundleStore, Catalogs, DoctorBus, DoctorSpec, StateRead, run_doctor};

const ORIGIN: &str = "zk2/host-a/tc/tc.netif.v1/state/interfaces/default/eth0";

/// An archive at `address` recording `host-a/tc`'s interfaces. Its watch
/// session is `quiet`, a client of a router no owner reaches, so it aligns
/// only when asked: an alignment of its own at an owner's token, still
/// retrying when the owner goes, would read nothing and leave its key
/// unconfirmed (core §4.4, "Retrying").
fn archive_config(address: &str, quiet: &zenoh::Session) -> ArchiveConfig {
    ArchiveConfig {
        service: config(address),
        records: vec![Recorded {
            owner: addr("host-a/tc"),
            selector: "zk2/host-a/tc/tc.netif.v1/state/interfaces/*/*".to_owned(),
            implementation: Implementation::new(example("tcgui/tc.netif.v1")),
        }],
        peers: Vec::new(),
        unconfirmed_horizon: None,
        watch: Some(quiet.clone()),
    }
}

fn member() -> Bindings {
    [
        ("ns".to_owned(), vec!["default".to_owned()]),
        ("iface".to_owned(), vec!["eth0".to_owned()]),
    ]
    .into()
}

/// The doctor, the state report and `why` read both forms.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_fleets_readers_read_the_compact_attachment_beside_the_json_one() {
    let contract = example("tcgui/tc.netif.v1");
    let mut catalog = zenkey_model::catalog::Catalog::new(0).expect("a catalog");
    catalog
        .push(
            contract.iface.clone(),
            zenkey_model::canonical::Fingerprint::of(&contract),
        )
        .expect("an entry");
    let (_r, ep) = router(None).await;
    let (owners, tool, archiving) = (client(&ep).await, client(&ep).await, client(&ep).await);
    let (_quiet_router, quiet_ep) = router(None).await;
    let quiet = client(&quiet_ep).await;
    let compact = Archive::start_under(
        &archiving,
        archive_config("ground/archive", &quiet),
        catalog.clone(),
    )
    .await
    .expect("an archive under the catalog");
    let json = Archive::start(&archiving, archive_config("ground/json-archive", &quiet))
        .await
        .expect("an archive given none");
    let mut b = ServiceBuilder::new(&owners, config("host-a/tc"));
    b.implement(Implementation::new(contract.clone()))
        .expect("implement");
    for r in &contract.resources {
        let _ = b.expose(
            &iface("tc.netif.v1"),
            &zenkey::implementation::resource_name(r),
        );
    }
    b.serve_state(&iface("tc.netif.v1")).expect("serve state");
    let mut owner = b.start().await.expect("the owner");
    let writer = owner
        .state_writer(
            &iface("tc.netif.v1"),
            "state/interfaces/{ns}/{iface}",
            &member(),
        )
        .await
        .expect("a state writer");
    eventually("both archives record it, confirmed", || async {
        writer
            .put(br#"{"name":"eth0","index":2,"namespace":"default","is_up":true}"#.to_vec())
            .await
            .expect("a put");
        compact.confirmed(ORIGIN) == Some(true) && json.confirmed(ORIGIN) == Some(true)
    })
    .await;
    // The compact archive's reply carries 3 bytes.
    let got = tool
        .get(compact.key(ORIGIN))
        .timeout(T)
        .await
        .expect("a GET");
    let reply = got.recv_async().await.expect("a reply");
    let att = reply
        .result()
        .expect("a value")
        .attachment()
        .expect("an attachment")
        .to_bytes()
        .into_owned();
    assert_eq!(att.len(), 3, "the compact form");

    // archive-unaligned: every value confirmed, in either form.
    let bus = DoctorBus {
        session: tool.clone(),
        raw: tool.clone(),
        namespace: String::new(),
        watch: None,
    };
    let mut only = DoctorSpec::new(T);
    only.grace = Duration::from_millis(300);
    only.checks = [CheckId::ArchiveUnaligned].into_iter().collect();
    let mut clean = String::new();
    for _ in 0..50 {
        let r = run_doctor(&bus, &BundleStore::new(T), &only).await;
        match &r
            .check(CheckId::ArchiveUnaligned)
            .expect("reported")
            .verdict
        {
            Judgement::NotEstablished { reason } if reason.contains("2 value(s)") => {
                clean = reason.clone();
                break;
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    assert!(clean.contains("every one confirmed"), "{clean}");
    // The compact archive's alignment refused: its bit says unconfirmed.
    compact.refuse_next_reads(100);
    let a = compact.align(T).await.expect("an alignment");
    assert_eq!(a.unconfirmed, 1);
    let r = run_doctor(&bus, &BundleStore::new(T), &only).await;
    let c = r.check(CheckId::ArchiveUnaligned).expect("reported");
    assert_eq!(c.verdict, Judgement::Established, "{c:#?}");
    assert!(
        c.findings
            .iter()
            .any(|f| f.subject.contains("ground/archive") && f.evidence.contains("1 of 1")),
        "{c:#?}"
    );
    compact.refuse_next_reads(0);
    eventually("confirmed again", || async {
        let _ = compact.align(T).await;
        compact.confirmed(ORIGIN) == Some(true)
    })
    .await;

    // The state report: last-known, the identity through the catalog given
    // as the JSON form states it; given none, `confirmed` alone.
    let revision = Revision::from_contract(contract.clone(), ContractSource::File);
    let resource = revision
        .resource(
            "state/interfaces/{ns}/{iface}",
            &[zenkey_model::authoring::Kind::State],
        )
        .expect("the resource");
    let (owner_addr, values) = (addr("host-a/tc"), member());
    let read = || StateRead {
        revision: &revision,
        owner: &owner_addr,
        resource,
        values: &values,
        timeout: T,
    };
    let catalogs = Catalogs::new(vec![catalog]);
    let through = zenkey_fleet::last_known_state_through(
        &tool,
        read(),
        &addr("ground/archive"),
        zenkey_fleet::DeploymentEpoch::Assumed,
        &catalogs,
    )
    .await
    .expect("a last-known read");
    let twin = zenkey_fleet::last_known_state(&tool, read(), &addr("ground/json-archive"))
        .await
        .expect("a last-known read");
    let blind = zenkey_fleet::last_known_state(&tool, read(), &addr("ground/archive"))
        .await
        .expect("a last-known read");
    let (through, twin, blind) = (&through.rows[0], &twin.rows[0], &blind.rows[0]);
    assert_eq!(through.confirmed, Some(true));
    assert_eq!(through.identity, twin.identity, "the JSON form's identity");
    assert_eq!(
        through.identity.as_ref().map(|i| i["iface"].clone()),
        Some(serde_json::json!("tc.netif.v1"))
    );
    assert_eq!(blind.confirmed, Some(true), "the bit needs no catalog");
    assert_eq!(
        blind.identity.as_ref().map(|i| i["iface"].clone()),
        Some(serde_json::Value::Null),
        "never a guessed identity"
    );

    // why: an owner present and silent sends the ladder to its last-known
    // rung (S6). A twin holds the owner's presence under another instance
    // (its tokens and its descriptor, no state queryable), up before the
    // owner goes. The archive aligns only when asked, so it keeps its key
    // confirmed; `why` reads that bit from the compact attachment.
    drop(json);
    let twin = client(&ep).await;
    let instance: zenkey_model::grammar::InstanceId = "00000000000000c3".parse().expect("an id");
    let at = addr("host-a/tc");
    let mut d: serde_json::Value =
        serde_json::from_slice(&owner.descriptor_bytes()).expect("the owner's descriptor");
    d["instance"] = serde_json::json!(instance.to_string());
    let body = serde_json::to_vec(&d).expect("JSON");
    let ikey = zenkey_model::grammar::instance_key(&at, &instance)
        .expect("a key")
        .to_string();
    let fp = zenkey_model::canonical::Fingerprint::of(&contract);
    let akey = zenkey_model::grammar::alive_key(&at, &contract.iface, &instance, &fp.hex().fp16())
        .expect("a key")
        .to_string();
    let reply_key = ikey.clone();
    let _descriptor = twin
        .declare_queryable(ikey.as_str())
        .complete(true)
        .callback(move |q| {
            use zenoh::Wait;
            let _ = q
                .reply(reply_key.as_str(), body.clone())
                .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                .wait();
        })
        .await
        .expect("a descriptor queryable");
    let _tokens = (
        twin.liveliness()
            .declare_token(ikey.as_str())
            .await
            .expect("a token"),
        twin.liveliness()
            .declare_token(akey.as_str())
            .await
            .expect("a token"),
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    drop(writer);
    owner.close().await.expect("closed");
    assert_eq!(compact.confirmed(ORIGIN), Some(true));
    let mut offline = zenkey_fleet::ContractSet::new();
    offline.insert(Revision::from_contract(
        contract.clone(),
        ContractSource::File,
    ));
    let store = BundleStore::new(T);
    store.seed(&offline);
    let spec = zenkey_fleet::WhySpec {
        timeout: T,
        window: Duration::from_secs(1),
    };
    let mut last_known = serde_json::Value::Null;
    let mut seen = serde_json::Value::Null;
    for _ in 0..50 {
        let target = zenkey_fleet::WhyTarget::parse(ORIGIN).expect("a key");
        let report =
            zenkey_fleet::run_why_through(&tool, &store, &catalogs, "", target, spec).await;
        let v = serde_json::to_value(&report).expect("a report");
        seen = v.clone();
        if !v["last_known"].is_null() {
            last_known = v["last_known"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(last_known["archive"], "ground/archive", "{seen:#}");
    assert_eq!(
        last_known["confirmed"], true,
        "the compact attachment's bit: {last_known}"
    );
}
