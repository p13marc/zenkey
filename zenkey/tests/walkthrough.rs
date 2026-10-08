//! The design's walkthroughs on their own contracts (`examples/zk2/`), and
//! the tcgui pilot's frontend binding (#619's done-when).

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use common::{T, client, config, eventually, example, router};
use serde_json::{Value, json};
use zenkey::consumer::Delivery;
use zenkey::model::authoring::Kind;
use zenkey::model::envelope::Detail;
use zenkey::model::grammar::{Addr, IfaceId, ZkKey};
use zenkey::model::template::Bindings;
use zenkey::operation::{Call, OpError, OperationServer};
use zenkey::presence::{self, Edge};
use zenkey::{Implementation, Outcome, Service, ServiceBuilder};

fn iface(s: &str) -> IfaceId {
    s.parse().unwrap()
}

/// A camera: `camera.v1`'s `@stream image` with its `FrameMeta`
/// attachment, and `state info`.
async fn camera(s: &zenoh::Session, address: &str) -> (Service, zenkey::writer::Writer) {
    let cam = iface("camera.v1");
    let mut b = ServiceBuilder::new(s, config(address));
    b.implement(Implementation::new(example("walkthrough/camera.v1")))
        .unwrap();
    let w = b
        .declare_writer(&cam, "@stream/image", &Bindings::new())
        .await
        .unwrap();
    b.expose(&cam, "state/info").unwrap();
    (b.start().await.unwrap(), w)
}

/// A detector: `detections.v1`, whose contract requires `camera.v1` as
/// `input`; the configuration binds it. Every image it receives becomes an
/// `objects` sample. The code is the same whatever the binding says.
async fn detector(
    s: &zenoh::Session,
    address: &str,
    input: &str,
) -> (
    Service,
    zenkey::consumer::Subscription,
    Arc<Mutex<Vec<String>>>,
) {
    let det = iface("detections.v1");
    let mut b = ServiceBuilder::new(s, config(address).bind("input", &[input]));
    b.implement(Implementation::new(example("walkthrough/detections.v1")))
        .unwrap();
    let out = Arc::new(
        b.declare_writer(&det, "stream/objects", &Bindings::new())
            .await
            .unwrap(),
    );
    let svc = b.start().await.unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let (o, sn) = (Arc::clone(&out), Arc::clone(&seen));
    let handle = tokio::runtime::Handle::current();
    let sub = svc
        .consumer("input", Arc::new(example("walkthrough/camera.v1")))
        .unwrap()
        .subscribe("@stream/image", move |d: Delivery| {
            assert!(d.sample.attachment().is_some(), "FrameMeta rides along");
            sn.lock().unwrap().push(d.provider.to_string());
            let o = Arc::clone(&o);
            // An empty `detections.v1.Objects` is a valid message.
            handle.spawn(async move { o.put(Vec::<u8>::new()).await.unwrap() });
        })
        .await
        .unwrap();
    (svc, sub, seen)
}

/// Walkthrough §4.1: camera → detector → tracker over the walkthrough's
/// contracts; replay by rebinding the detector's input, with no code
/// change; and the graph, read from descriptors.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w4_1_perception_pipeline() {
    let (_r1, ep) = router(None).await;
    let veh = client(&ep).await;
    let tool = client(&ep).await;

    let (_cam, frames) = camera(&veh, "vehicle-01/cam-front").await;
    let (_replay, replay_frames) = camera(&veh, "vehicle-01/replay-cam").await;
    let (_det, _dsub, det_seen) =
        detector(&veh, "vehicle-01/detector", "vehicle-01/cam-front").await;
    // Replay: the same detector code, its input rebound by configuration.
    let (_det2, _d2sub, det2_seen) =
        detector(&veh, "vehicle-01/detector-replay", "vehicle-01/replay-cam").await;

    let mut b = ServiceBuilder::new(
        &veh,
        config("vehicle-01/tracker").bind("sources", &["vehicle-01/*"]),
    );
    b.require("sources", iface("detections.v1"), false);
    let tracker = b.start().await.unwrap();
    let tracked: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let t = Arc::clone(&tracked);
    let _tsub = tracker
        .consumer("sources", Arc::new(example("walkthrough/detections.v1")))
        .unwrap()
        .subscribe("stream/objects", move |d: Delivery| {
            t.lock().unwrap().insert(d.provider.to_string());
        })
        .await
        .unwrap();

    // A camera frame: JPEG bytes, an (empty) FrameMeta attachment.
    eventually(
        "both detectors see their camera and the tracker sees both",
        || async {
            frames
                .put_with(vec![0xff, 0xd8], Some(Vec::<u8>::new()))
                .await
                .unwrap();
            replay_frames
                .put_with(vec![0xff, 0xd8], Some(Vec::<u8>::new()))
                .await
                .unwrap();
            tracked.lock().unwrap().len() == 2
        },
    )
    .await;
    assert!(
        det_seen
            .lock()
            .unwrap()
            .iter()
            .all(|p| p == "vehicle-01/cam-front")
    );
    assert!(
        det2_seen
            .lock()
            .unwrap()
            .iter()
            .all(|p| p == "vehicle-01/replay-cam")
    );

    // The graph: detector ← cam-front, tracker ← {detectors}, from
    // descriptors and tokens alone.
    let tokens = presence::tokens(&tool, "zk2/vehicle-01/*/@zk/**", T)
        .await
        .unwrap();
    let mut descriptors = Vec::new();
    for k in &tokens {
        if let ZkKey::Instance { addr, instance } = k {
            descriptors.push(
                presence::descriptor(&tool, addr, instance, T)
                    .await
                    .unwrap()
                    .into_descriptor()
                    .unwrap(),
            );
        }
    }
    let e = |c: &str, r: &str, p: &str| Edge {
        consumer: c.to_owned(),
        role: r.to_owned(),
        provider: p.to_owned(),
    };
    let drawn: BTreeSet<Edge> = presence::edges(&descriptors, &tokens).into_iter().collect();
    assert_eq!(
        drawn,
        BTreeSet::from([
            e("vehicle-01/detector", "input", "vehicle-01/cam-front"),
            e(
                "vehicle-01/detector-replay",
                "input",
                "vehicle-01/replay-cam"
            ),
            e("vehicle-01/tracker", "sources", "vehicle-01/detector"),
            e(
                "vehicle-01/tracker",
                "sources",
                "vehicle-01/detector-replay"
            ),
        ])
    );
}

/// The tcgui frontend (`examples/zk2/tcgui/frontend.bindings.toml`): a pure
/// consumer binding `tc.netif.v1` to `*/tc`, every system's tc service.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcgui_frontend_binding() {
    let (_r1, ep) = router(None).await;
    let hosts = client(&ep).await;
    let ws = client(&ep).await;
    let netif = iface("tc.netif.v1");
    let bw = |ns: &str, i: &str| -> Bindings {
        [
            ("ns".to_owned(), vec![ns.to_owned()]),
            ("iface".to_owned(), vec![i.to_owned()]),
        ]
        .into()
    };
    let mut writers = Vec::new();
    let mut tcs = Vec::new();
    for host in ["host-a", "host-b"] {
        let c = example("tcgui/tc.netif.v1");
        let names: Vec<String> = c
            .resources
            .iter()
            .map(zenkey::implementation::resource_name)
            .collect();
        let mut b = ServiceBuilder::new(&hosts, config(&format!("{host}/tc")));
        b.implement(Implementation::new(c)).unwrap();
        writers.push(
            b.declare_writer(
                &netif,
                "stream/bandwidth/{ns}/{iface}",
                &bw("default", "eth0"),
            )
            .await
            .unwrap(),
        );
        for n in names {
            b.expose(&netif, &n).unwrap();
        }
        tcs.push(b.start().await.unwrap());
    }

    let mut b = ServiceBuilder::new(&ws, config("ws-01/tcgui-frontend").bind("netif", &["*/tc"]));
    b.require("netif", netif.clone(), false);
    let frontend = b.start().await.unwrap();
    let from: Arc<Mutex<BTreeSet<String>>> = Arc::default();
    let f = Arc::clone(&from);
    let _sub = frontend
        .consumer("netif", Arc::new(example("tcgui/tc.netif.v1")))
        .unwrap()
        .subscribe("stream/bandwidth/{ns}/{iface}", move |d: Delivery| {
            assert_eq!(d.values["iface"], ["eth0"]);
            f.lock().unwrap().insert(d.provider.to_string());
        })
        .await
        .unwrap();
    eventually("both hosts' tc services deliver", || async {
        for w in &writers {
            w.put_value(&serde_json::json!({"rx_bps": 1, "tx_bps": 2}))
                .await
                .ok();
        }
        from.lock().unwrap().len() == 2
    })
    .await;
    assert_eq!(
        *from.lock().unwrap(),
        BTreeSet::from(["host-a/tc".to_owned(), "host-b/tc".to_owned()])
    );
}

/// A tcgui backend at `address`: `tc.netif.v1` and `tc.netem.v1`, serving
/// their operations over the templates and exposing the rest. Each
/// execution is recorded as `<host> <op> <values>`.
async fn tc_backend(
    s: &zenoh::Session,
    address: &str,
    ran: Arc<Mutex<Vec<String>>>,
) -> (Service, Vec<OperationServer>) {
    let (netif, netem) = (iface("tc.netif.v1"), iface("tc.netem.v1"));
    let mut b = ServiceBuilder::new(s, config(address));
    for (i, path) in [(&netif, "tcgui/tc.netif.v1"), (&netem, "tcgui/tc.netem.v1")] {
        let c = example(path);
        let names: Vec<String> = c
            .resources
            .iter()
            .filter(|r| r.kind != Kind::Operation)
            .map(zenkey::implementation::resource_name)
            .collect();
        b.implement(Implementation::new(c)).unwrap();
        for n in names {
            b.expose(i, &n).unwrap();
        }
    }
    let record = |op: &'static str| {
        let ran = Arc::clone(&ran);
        let host = address.to_owned();
        move |call: &Call| {
            let v: Vec<String> = call
                .values()
                .map(|v| v.values().flatten().cloned().collect())
                .unwrap_or_default();
            ran.lock()
                .unwrap()
                .push(format!("{host} {op} {}", v.join("/")));
        }
    };

    let mut servers = Vec::new();
    let rec = record("interfaces-set");
    servers.push(
        b.serve_value(
            &netif,
            "@op/interfaces/{ns}/{iface}/set",
            None,
            move |call: Call, req: Value| {
                rec(&call);
                async move { Ok(json!({"new_state": req["operation"] == "Enable"})) }
            },
        )
        .await
        .unwrap(),
    );
    let rec = record("diagnostics");
    let host = address.to_owned();
    servers.push(
        b.serve_value(
            &netif,
            "@op/diagnostics",
            None,
            move |call: Call, req: Value| {
                rec(&call);
                let host = host.clone();
                async move { Ok(json!({"results": {"host": host, "interface": req["interface"]}})) }
            },
        )
        .await
        .unwrap(),
    );
    let rec = record("config-set");
    servers.push(
        b.serve_value(
            &netem,
            "@op/config/{ns}/{iface}/set",
            None,
            move |call: Call, _req: Value| {
                rec(&call);
                let lo = call.values().is_some_and(|v| v["iface"] == ["lo"]);
                async move {
                    if lo {
                        return Err(OpError::app(
                            "netem refused",
                            &json!({"kind": "kernel", "message": "Operation not permitted"}),
                        ));
                    }
                    Ok(json!({"message": "applied"}))
                }
            },
        )
        .await
        .unwrap(),
    );
    let rec = record("plug-set");
    servers.push(
        b.serve_value(
            &netem,
            "@op/plug/{ns}/{iface}/set",
            None,
            move |call: Call, _req: Value| {
                rec(&call);
                async move { Ok(json!({"message": "plugged"})) }
            },
        )
        .await
        .unwrap(),
    );
    (b.start().await.unwrap(), servers)
}

/// The tcgui pilot's operations (#621's done-when): the frontend calls the
/// templated `set` operations at one host's explicit address through its
/// roles, gets the backend's typed `app` error, and fans `diagnostics` out
/// across hosts, each reply attributed to its host.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tcgui_set_and_diagnostics() {
    let (_r1, ep) = router(None).await;
    let ran: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut backends = Vec::new();
    for host in ["host-a", "host-b"] {
        let s = client(&ep).await;
        backends.push(tc_backend(&s, &format!("{host}/tc"), Arc::clone(&ran)).await);
    }

    let ws = client(&ep).await;
    let mut b = ServiceBuilder::new(
        &ws,
        config("ws-01/tcgui-frontend")
            .bind("netif", &["*/tc"])
            .bind("netem", &["*/tc"]),
    );
    b.require("netif", iface("tc.netif.v1"), false);
    b.require("netem", iface("tc.netem.v1"), false);
    let frontend = b.start().await.unwrap();
    let netif = frontend
        .client("netif", Arc::new(example("tcgui/tc.netif.v1")))
        .unwrap();
    let netem = frontend
        .client("netem", Arc::new(example("tcgui/tc.netem.v1")))
        .unwrap();
    // A tool acts on presence, never on an owner's start() returning.
    eventually("both backends are present", || async {
        netif.present(T).await.unwrap().len() == 2 && netem.present(T).await.unwrap().len() == 2
    })
    .await;
    let (host_a, host_b): (Addr, Addr) =
        ("host-a/tc".parse().unwrap(), "host-b/tc".parse().unwrap());
    let member = |ns: &str, i: &str| -> Bindings {
        [
            ("ns".to_owned(), vec![ns.to_owned()]),
            ("iface".to_owned(), vec![i.to_owned()]),
        ]
        .into()
    };

    // A templated set at one explicit address: the value is slugged into
    // its chunk, and the server reads it back raw.
    let out = netif
        .call_value(
            &host_a,
            "@op/interfaces/{ns}/{iface}/set",
            &member("default", "veth@1"),
            &json!({"operation": "Enable"}),
        )
        .await
        .unwrap();
    let Outcome::Value(answer) = out else {
        panic!("a value reply: {out:?}")
    };
    assert_eq!(
        answer.key(),
        "zk2/host-a/tc/tc.netif.v1/@op/interfaces/default/x-veth_x401/set"
    );
    assert_eq!(answer.value::<Value>().unwrap()["new_state"], true);

    // A netem apply the kernel refuses: `app`, with the TcError inline.
    let out = netem
        .call_value(
            &host_b,
            "@op/config/{ns}/{iface}/set",
            &member("default", "lo"),
            &json!({"operation": {"Apply": {}}}),
        )
        .await
        .unwrap();
    let env = out.refusal().expect("refused");
    assert_eq!(env.code, "app");
    let Some(Detail::Value(detail)) = &env.detail else {
        panic!("an inline detail: {env:?}")
    };
    assert_eq!(detail["kind"], "kernel");
    let plug = netem
        .call_value(
            &host_b,
            "@op/plug/{ns}/{iface}/set",
            &member("default", "eth0"),
            &json!({"operation": "Buffer"}),
        )
        .await
        .unwrap();
    assert!(matches!(plug, Outcome::Value(_)), "{plug:?}");

    // Cross-host diagnostics: one fan-out, one reply per host, attributed.
    let replies = netif
        .fleet()
        .call_value(
            "@op/diagnostics",
            &Bindings::new(),
            &json!({"namespace": "default", "interface": "eth0"}),
        )
        .await
        .unwrap();
    let mut by: Vec<(String, String)> = replies
        .repliers
        .iter()
        .map(|r| {
            let v = r.values[0].value::<Value>().unwrap();
            let host = v["results"]["host"].as_str().unwrap().to_owned();
            (r.addr.to_string(), host)
        })
        .collect();
    by.sort();
    let pair = |h: &str| (h.to_owned(), h.to_owned());
    assert_eq!(by, [pair("host-a/tc"), pair("host-b/tc")]);
    // A set is never fanned out (O2).
    assert!(
        netif
            .fleet()
            .call_value(
                "@op/interfaces/{ns}/{iface}/set",
                &Bindings::new(),
                &json!({"operation": "Disable"}),
            )
            .await
            .is_err()
    );

    let mut ran = ran.lock().unwrap().clone();
    ran.sort();
    assert_eq!(
        ran,
        [
            "host-a/tc diagnostics ",
            "host-a/tc interfaces-set veth@1/default",
            "host-b/tc config-set lo/default",
            "host-b/tc diagnostics ",
            "host-b/tc plug-set eth0/default",
        ]
    );
}

/// Walkthrough §4.3: a plan written while the vehicle was offline, after
/// which the fleet manager stopped, still reaches the executor, through the
/// ground's archive and the vehicle's (S5, S6). The executor applies a plan
/// only when it is newer than the last applied (§4.3's consumer rule), so it
/// converges with zero wrong answers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn w4_3_commanding_an_intermittently_connected_vehicle() {
    use common::{link_to, router_via};
    use zenkey::archive::{self, Archive, ArchiveConfig, Recorded};
    use zenkey::state::{StateGet, ValueOrder};

    let (_r1, ep1) = router(None).await;
    let link = link_to(&ep1).await;
    let (_r2, ep2) = router_via(&link).await;
    let ground = client(&ep1).await;
    let vehicle = client(&ep2).await;
    let plan = iface("mission_plan.v1");
    let plan_imp = || Implementation::new(example("walkthrough/mission_plan.v1"));
    let origin = "zk2/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01";

    let mut b = ServiceBuilder::new(&ground, config("ground/fleet-mgr"));
    b.implement(plan_imp()).unwrap();
    b.expose(&plan, "state/plans/{vehicle}").unwrap();
    b.expose(&plan, "@op/list").unwrap();
    b.serve_state(&plan).unwrap();
    let mut mgr = b.start().await.unwrap();
    let vals: Bindings = [("vehicle".to_owned(), vec!["vehicle-01".to_owned()])].into();
    let w = mgr
        .state_writer(&plan, "state/plans/{vehicle}", &vals)
        .await
        .unwrap();

    let record = |selector: &str| Recorded {
        owner: common::addr("ground/fleet-mgr"),
        selector: selector.to_owned(),
        implementation: plan_imp(),
    };
    let _ground_arch = Archive::start(
        &ground,
        ArchiveConfig {
            service: config("ground/archive"),
            records: vec![record("zk2/ground/fleet-mgr/mission_plan.v1/state/plans/*")],
            peers: vec![],
            unconfirmed_horizon: None,
        },
    )
    .await
    .unwrap();
    let veh_arch = Archive::start(
        &vehicle,
        ArchiveConfig {
            service: config("vehicle-01/archive"),
            records: vec![record(origin)],
            peers: vec![common::addr("ground/archive")],
            unconfirmed_horizon: None,
        },
    )
    .await
    .unwrap();

    let mut cfg = config("vehicle-01/executor").bind("plan", &["ground/fleet-mgr"]);
    cfg.bindings
        .get_mut("plan")
        .unwrap()
        .params
        .insert("vehicle".to_owned(), "self.system".to_owned());
    let mut b = ServiceBuilder::new(&vehicle, cfg);
    b.require("plan", plan.clone(), false);
    let exec = b.start().await.unwrap();
    let consumer = exec
        .consumer("plan", Arc::new(example("walkthrough/mission_plan.v1")))
        .unwrap();

    // The executor: the owner while it answers, else the vehicle's archive,
    // known to be last-known; it applies only what is newer.
    let mut order = ValueOrder::new();
    let mut applied: Vec<(Vec<u8>, bool)> = Vec::new();
    let read_and_apply = async |order: &mut ValueOrder, applied: &mut Vec<(Vec<u8>, bool)>| {
        let (bytes, ts, last_known) = match consumer
            .get("state/plans/{vehicle}", None, T)
            .await
            .unwrap()
        {
            StateGet::Answered(v) => match &v[0] {
                zenkey::state::Current::Value { sample, .. } => (
                    sample.payload().to_bytes().into_owned(),
                    *sample.timestamp().unwrap(),
                    false,
                ),
                zenkey::state::Current::Deleted { .. } => return,
            },
            StateGet::Silent => {
                let Some(l) =
                    archive::last_known(&vehicle, &common::addr("vehicle-01/archive"), origin, T)
                        .await
                        .unwrap()
                else {
                    return;
                };
                (l.value.unwrap_or_default(), l.timestamp.unwrap(), true)
            }
        };
        if order.accept(origin, &ts) {
            applied.push((bytes, last_known));
        }
    };

    // Rev 1, with the link up: current.
    eventually("the vehicle's archive records", || async {
        w.put("rev1").await.unwrap();
        veh_arch.confirmed(origin) == Some(true)
    })
    .await;
    read_and_apply(&mut order, &mut applied).await;
    assert_eq!(applied, [(b"rev1".to_vec(), false)]);

    // The vehicle goes offline; rev 2 is written; the fleet manager stops.
    link.cut();
    eventually("the vehicle is cut off", || async {
        zenkey::presence::liveliness_keys(&vehicle, "zk2/ground/*/@zk/**", T)
            .await
            .unwrap()
            .is_empty()
    })
    .await;
    w.put("rev2").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    drop(w);
    mgr.close().await.unwrap();

    // The link heals: the vehicle's archive aligns from the ground's.
    link.heal();
    eventually("rev 2 reached the vehicle's archive", || async {
        archive::last_known(&vehicle, &common::addr("vehicle-01/archive"), origin, T)
            .await
            .unwrap()
            .is_some_and(|l| l.value.as_deref() == Some(&b"rev2"[..]))
    })
    .await;
    read_and_apply(&mut order, &mut applied).await;
    // Reading again changes nothing: no stale plan is ever applied.
    read_and_apply(&mut order, &mut applied).await;
    assert_eq!(
        applied,
        [(b"rev1".to_vec(), false), (b"rev2".to_vec(), true)],
        "converged, last-known marked"
    );
}
