//! The nodes pane on zk2 presence (#614, FL9): addresses up, down and
//! unobservable, an instance's descriptor, and health on selection.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use zengui::nodes::NodesState;
use zengui::presence_link::PresenceUpdate;
use zengui::view::nodes::{NodesData, pane};
use zenkey_fleet::model::catalog::{DescriptorRead, Observed};
use zenkey_fleet::{Catalog, LivePresence, PresenceScope, SeedOutcome};
use zenkey_model::descriptor::Descriptor;
use zenkey_model::grammar::Addr;

use super::*;

const NAV1: &str = "zk2/robot1/nav/@zk/instance/0000000000000001";
const NAV2: &str = "zk2/robot2/nav/@zk/instance/0000000000000002";

fn addr(s: &str) -> Addr {
    s.parse().expect("an address")
}

fn observed(keys: &[&str], complete: bool) -> Observed {
    let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
    Observed::from_keys("zk2/*/*/@zk/**", &keys, complete)
}

fn update(live: LivePresence) -> PresenceUpdate {
    PresenceUpdate {
        presence: Arc::new(live),
        moved: Vec::new(),
        catalog: None,
        generation: 0,
        seed: SeedOutcome::Complete,
    }
}

fn render<'a>(
    presence: Option<&'a PresenceUpdate>,
    nodes: &'a NodesState,
    selected: Option<&'a Addr>,
) -> iced_test::Simulator<'a, Message> {
    iced_test::Simulator::with_size(
        iced::Settings::default(),
        (1600.0, 1200.0),
        pane(NodesData {
            presence,
            nodes,
            selected,
            sp: sp(),
        }),
    )
}

/// Up, down and unobservable are three states, each in its own words and
/// glyph — and unobservable is never down: an address whose token was not
/// seen under a basis that cannot prove absence reads unobservable, and
/// after a loss *every* address does, whatever it last showed.
#[test]
fn up_down_and_unobservable_are_kept_apart_and_unobservable_is_never_down() {
    let now = SystemTime::now();
    let nodes = NodesState::default();

    // A complete read, then robot2's instance token is heard going:
    // unobservable, awaiting the read it owes (§8.1, 0.37), never down.
    let mut live = LivePresence::new(PresenceScope::all());
    live.seed(&observed(&[NAV1, NAV2], true), now);
    live.token(NAV2, false, now);
    let withdrawn = update(live.clone());
    let mut ui = render(Some(&withdrawn), &nodes, None);
    assert!(ui.find("robot2/nav: unobservable").is_ok());
    assert!(
        ui.find("heard withdrawn, awaiting a read — not down (§8.1)")
            .is_ok()
    );
    assert!(ui.find("robot2/nav: down").is_err(), "not on a withdrawal");
    // The read that shows none: robot2 is down.
    live.seed(&observed(&[NAV1], true), now);
    let complete = update(live.clone());
    let mut ui = render(Some(&complete), &nodes, None);
    assert!(ui.find("robot1/nav: up").is_ok());
    assert!(
        ui.find("robot2/nav: down").is_ok(),
        "no instance token under a complete read is down"
    );
    assert!(ui.find("presence: 1 up · 1 down · 0 unobservable").is_ok());

    // Events lost: nothing is down any more, robot2 included.
    live.dropped(3, now);
    let lost = update(live);
    let mut ui = render(Some(&lost), &nodes, None);
    assert!(ui.find("robot1/nav: unobservable").is_ok());
    assert!(
        ui.find("robot2/nav: unobservable").is_ok(),
        "a loss makes the down address unobservable too"
    );
    assert!(
        ui.find("robot2/nav: down").is_err(),
        "unobservable is never down"
    );
    assert!(
        ui.find("presence: 3 events lost — every address unobservable until a complete re-read")
            .is_ok()
    );

    // No seed yet: a token heard is up, an address tracked with none is
    // unobservable — not down, and said why.
    let mut live = LivePresence::new(PresenceScope::all());
    live.token(NAV1, true, now);
    live.track(&addr("robot2/nav"), now);
    let unseeded = update(live);
    let mut ui = render(Some(&unseeded), &nodes, None);
    assert!(ui.find("robot1/nav: up").is_ok());
    assert!(ui.find("robot2/nav: unobservable").is_ok());
    assert!(
        ui.find("no complete read behind it — not down (§8.1)")
            .is_ok()
    );
}

/// Not asked is not empty, and an empty answer says which kind it is: a
/// complete read that found nobody, or one that may have missed someone.
#[test]
fn not_asked_is_not_empty() {
    let nodes = NodesState::default();
    let mut ui = render(None, &nodes, None);
    assert!(
        ui.find("presence not read").is_ok(),
        "no presence landing is not an empty deployment (O4)"
    );

    let mut complete = LivePresence::new(PresenceScope::all());
    complete.seed(&observed(&[], true), SystemTime::now());
    let complete = update(complete);
    let mut ui = render(Some(&complete), &nodes, None);
    assert!(ui.find("no zk2 service holds a token here").is_ok());

    let mut partial = LivePresence::new(PresenceScope::all());
    partial.seed(&observed(&[], false), SystemTime::now());
    let partial = update(partial);
    let mut ui = render(Some(&partial), &nodes, None);
    assert!(
        ui.find("no token observed").is_ok(),
        "an incomplete read proves no absence"
    );
    assert!(ui.find("no zk2 service holds a token here").is_err());
}

fn descriptor(service: &str, instance: &str, profiles: &[&str]) -> Descriptor {
    let mut meta = BTreeMap::new();
    meta.insert("host".to_owned(), serde_json::json!("rover-01"));
    meta.insert("process".to_owned(), serde_json::json!("navd"));
    meta.insert("zid".to_owned(), serde_json::json!("a1b2c3"));
    Descriptor {
        format: "zk2-descriptor/0.1".into(),
        service: service.into(),
        instance: instance.into(),
        interfaces: Vec::new(),
        capabilities: Vec::new(),
        requires: Vec::new(),
        profiles: profiles.iter().map(|p| p.to_string()).collect(),
        meta,
    }
}

/// The hostid chip comes from the descriptor's `profiles` and nothing else
/// (hostid.v1 §2.8, §2.11): a system *named* in the minted shape, whose
/// descriptor does not list `hostid.v1`, gets no chip.
#[test]
fn a_hostid_chip_only_from_profiles() {
    const MINTED: &str = "zk2/h-0123456789ab/nav/@zk/instance/0000000000000003";
    const LOOKALIKE: &str = "zk2/h-3fa9c2d41b7e/nav/@zk/instance/0000000000000004";
    let now = SystemTime::now();
    let mut seen = observed(&[MINTED, LOOKALIKE], true);
    seen.descriptors = Some(BTreeMap::from([
        (
            (
                addr("h-0123456789ab/nav"),
                "0000000000000003".parse().expect("id"),
            ),
            DescriptorRead::Served(Box::new(descriptor(
                "h-0123456789ab/nav",
                "0000000000000003",
                &["hostid.v1"],
            ))),
        ),
        (
            (
                addr("h-3fa9c2d41b7e/nav"),
                "0000000000000004".parse().expect("id"),
            ),
            DescriptorRead::Served(Box::new(descriptor(
                "h-3fa9c2d41b7e/nav",
                "0000000000000004",
                &[],
            ))),
        ),
    ]));
    let catalog = Arc::new(Catalog::new(&seen));
    let mut live = LivePresence::new(PresenceScope::all());
    live.seed(&seen, now);
    let presence = update(live);
    let mut nodes = NodesState::default();
    nodes.describe(Some(&catalog));

    let minted = addr("h-0123456789ab/nav");
    let mut ui = render(Some(&presence), &nodes, Some(&minted));
    assert!(
        ui.find("hostid.v1 — listed in its profiles").is_ok(),
        "its descriptor lists hostid.v1"
    );
    assert!(
        ui.find("host rover-01 · process navd · build — · zid a1b2c3")
            .is_ok(),
        "the descriptor's meta as written, an absent field as a dash"
    );

    let lookalike = addr("h-3fa9c2d41b7e/nav");
    let mut ui = render(Some(&presence), &nodes, Some(&lookalike));
    assert!(
        ui.find("hostid.v1 — listed in its profiles").is_err(),
        "a system named like a minted one is no minted system (§2.11)"
    );
}

/// `lab/svc`'s presence and descriptor, listing `health.v1`.
fn health_presence() -> Observed {
    const INST: &str = "zk2/lab/svc/@zk/instance/000000000000000a";
    let mut o = observed(&[INST], true);
    let d: Descriptor = serde_json::from_value(serde_json::json!({
        "format": "zk2-descriptor/0.1",
        "service": "lab/svc",
        "instance": "000000000000000a",
        "interfaces": [{
            "iface": "health.v1",
            "contract": format!("sha256:{}", zenkey::health::FINGERPRINT),
            "minor": 0,
            "token": false,
        }],
        "meta": {"zid": "ab12"},
    }))
    .expect("a descriptor");
    o.descriptors = Some(BTreeMap::from([(
        (addr("lab/svc"), "000000000000000a".parse().expect("id")),
        DescriptorRead::Served(Box::new(d)),
    )]));
    o
}

/// A reading of `lab/svc` whose owner answered a status at OK, stamped five
/// seconds before the reading: judged by the engine, with or without the
/// operator's word for the clock.
fn health_report(clocks_synced: bool) -> zenkey_fleet::report::HealthReport {
    use zenkey_fleet::bus::health::{HealthGet, Replied, ServiceGot, Stamped, StatusValue};
    use zenkey_model::health::{Level, Read};
    let read_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let got = HealthGet {
        selector: "zk2/lab/svc/health.v1/state/**".into(),
        read_at,
        complete: true,
        services: BTreeMap::from([(
            addr("lab/svc"),
            ServiceGot {
                status: Some(Replied {
                    value: Some(StatusValue {
                        level: Read::Level(Level::Ok),
                        reason: "nominal".into(),
                        since_ns: 1,
                    }),
                    stamp: Some(Stamped {
                        time: read_at - Duration::from_secs(5),
                        clock: "ab12".into(),
                    }),
                }),
                checks: BTreeMap::new(),
            },
        )]),
    };
    zenkey_fleet::judge_health(&zenkey_fleet::HealthObservation {
        namespace: String::new(),
        target: zenkey_fleet::HealthTarget::One(addr("lab/svc")),
        spec: zenkey_fleet::HealthSpec {
            timeout: Duration::from_secs(1),
            window: None,
            grace: Duration::from_secs(1),
            clocks_synced,
            face: None,
        },
        presence: Some(Ok(health_presence())),
        first: Ok(got.clone()),
        second: Ok(got),
        window: None,
        archived: None,
    })
}

/// Health without the clock's ground reads "age unobservable" — never a
/// level, never stale — and says how to give it one; with the operator's
/// word, the same answer reads healthy at its level.
#[test]
fn health_without_clocks_reads_age_unobservable() {
    let svc = addr("lab/svc");
    let mut live = LivePresence::new(PresenceScope::all());
    live.seed(&health_presence(), SystemTime::now());
    let presence = update(live);

    let mut nodes = NodesState::default();
    nodes.asking(svc.clone());
    nodes.health_read(svc.clone(), Arc::new(health_report(false)));
    let mut ui = render(Some(&presence), &nodes, Some(&svc));
    assert!(
        ui.find(
            "age unobservable — this reader's clock is not trusted to the HLC delta against \
             the stamping clock; tick \"clocks synced\" if they agree"
        )
        .is_ok(),
        "no word and no window: the reply's age is unobservable"
    );
    assert!(
        ui.find("clock: no word and no window — a reply's age is unobservable (freshness.v1 §2.6)")
            .is_ok()
    );
    assert!(
        ui.find("status ok — nominal (the owner's reply, S4)")
            .is_ok()
    );
    drop(ui);

    nodes.clocks_synced = true;
    nodes.asking(svc.clone());
    nodes.health_read(svc.clone(), Arc::new(health_report(true)));
    let mut ui = render(Some(&presence), &nodes, Some(&svc));
    assert!(
        ui.find("healthy (ok) — its status is OK and confirmed, and no check is worse")
            .is_ok()
    );
    assert!(
        ui.find("clock: the deployment's word (clocks synced)")
            .is_ok()
    );

    // Nothing selected: the reading stays the selected address's, and no
    // card shows it.
    let mut ui = render(Some(&presence), &nodes, None);
    assert!(
        ui.find("healthy (ok) — its status is OK and confirmed, and no check is worse")
            .is_err()
    );
}

/// `arbitration.v1` on the nodes card (#909, BC): an arbitrated consumer's
/// role, its declaration, its binding naming the wildcard, and the
/// selection said to be the tool's own view, each in the engine's tokens
/// and words; nothing derived in the view.
#[test]
fn an_arbitrated_role_reads_as_the_tools_own_view() {
    let world = common::World::new();
    let rudder = addr(common::RUDDER);
    let mut live = LivePresence::new(PresenceScope::all());
    live.seed(&world.observed_with_rudder(), SystemTime::now());
    let presence = update(live);
    let mut nodes = NodesState::default();
    nodes.asking_arbitration(rudder.clone());
    let mut ui = render(Some(&presence), &nodes, Some(&rudder));
    assert!(ui.find("ARBITRATION (ARBITRATION.V1)").is_ok());
    assert!(
        ui.find(
            "reading arbitration.v1 — the descriptor, the contracts, and the tool's own view \
             listening two deadlines"
        )
        .is_ok()
    );
    drop(ui);
    nodes.arbitration_read(rudder.clone(), world.rudder_arbitration());
    let mut ui = render(Some(&presence), &nodes, Some(&rudder));
    assert!(ui.find("cmd → steer.v1: a warning").is_ok());
    assert!(
        ui.find(
            "declaration: yes (valid) — arbitrated, priority over stream/cmd, deadline 300 ms, \
             period 50 ms"
        )
        .is_ok()
    );
    assert!(
        ui.find("the tool's own view: follows lab/autopilot (selected), over 600 ms")
            .is_ok()
    );
    assert!(
        ui.find("binding: yes (wildcard lab/*) — bound [lab/safety, lab/*]")
            .is_ok()
    );
    drop(ui);
    // An address that requires no role shows no section at all.
    let nav = addr("robot1/nav");
    let mut ui = render(Some(&presence), &nodes, Some(&nav));
    assert!(ui.find("ARBITRATION (ARBITRATION.V1)").is_err());
}
