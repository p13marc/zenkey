//! The generated code on a bus (#611): in-process routers and clients, as in
//! `zenkey/tests/common`. Owners serve through `Handlers` and publish typed
//! data through `Server`; consumers receive it decoded through `Consumer`;
//! callers call through `Client` and `Fleet`.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zenkey::call::Outcome;
use zenkey::codec::Json;
use zenkey::model::envelope::Detail;
use zenkey::model::grammar::Addr;
use zenkey::typed::StateGet;
use zenkey::{CallInfo, CallMetadata, OpError, ServiceBuilder, ServiceConfig, Sink};
use zenkey_codegen_test::model::{TcError, TcErrorKind};
use zenkey_codegen_test::zk2::{mission_plan_v1, nav_v2, tc_netif_v1, thruster_v1, twist_cmd_v1};

/// The reply timeout.
const T: Duration = Duration::from_secs(1);

fn addr(s: &str) -> Addr {
    s.parse().unwrap()
}

fn base() -> zenkey::zenoh::Config {
    let mut c = zenkey::zenoh::Config::default();
    c.insert_json5("scouting/multicast/enabled", "false")
        .unwrap();
    c.insert_json5("scouting/gossip/enabled", "false").unwrap();
    c
}

/// A router on an ephemeral loopback port, and its endpoint.
async fn router() -> (zenkey::zenoh::Session, String) {
    let mut c = base();
    c.insert_json5("mode", "\"router\"").unwrap();
    c.insert_json5("listen/endpoints", "[\"tcp/127.0.0.1:0\"]")
        .unwrap();
    let r = zenkey::zenoh::open(c).await.unwrap();
    let ep = r
        .info()
        .locators()
        .await
        .into_iter()
        .map(|l| l.to_string())
        .find(|l| l.starts_with("tcp/127.0.0.1:"))
        .unwrap();
    (r, ep)
}

async fn client(endpoint: &str) -> zenkey::zenoh::Session {
    let mut c = base();
    c.insert_json5("mode", "\"client\"").unwrap();
    c.insert_json5("connect/endpoints", &format!("[\"{endpoint}\"]"))
        .unwrap();
    zenkey::zenoh::open(c).await.unwrap()
}

/// Retries `f` until it gives `Some`, failing after 20 s: a net for routing
/// to settle, never an assertion.
async fn eventually<T, F, Fut>(what: &str, mut f: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(v) = f().await {
            return v;
        }
        assert!(tokio::time::Instant::now() < deadline, "never: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A thruster: `arm` arms it, unless the reason is empty.
#[derive(Default)]
struct Thruster {
    seen: Mutex<Vec<Option<CallMetadata>>>,
}

impl thruster_v1::Handlers for Thruster {
    async fn arm(
        &self,
        request: thruster_v1::types::ArmRequest,
        call: CallInfo,
    ) -> Result<thruster_v1::types::Status, OpError> {
        self.seen.lock().unwrap().push(call.metadata().cloned());
        if request.reason.is_empty() {
            return Err(OpError::invalid_request("a reason is required"));
        }
        Ok(thruster_v1::types::Status {
            armed: true,
            active_commander: "vehicle-01/teleop".into(),
        })
    }

    async fn disarm(
        &self,
        _request: (),
        _call: CallInfo,
    ) -> Result<thruster_v1::types::Status, OpError> {
        Ok(thruster_v1::types::Status::default())
    }
}

/// Walkthrough §4.2 on the generated code: a thruster serving `arm` and
/// `disarm` through its `Handlers` and publishing typed telemetry and
/// status; a monitor receiving them decoded through `Consumer`; a tool
/// calling through `Client`; the thruster's `cmd` role fed by a commander
/// whose `Server` has no operations.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_thruster_serves_publishes_and_is_consumed_and_called() {
    let (_r, ep) = router().await;
    let veh = client(&ep).await;
    let ground = client(&ep).await;

    // The commander: twist_cmd.v1, data only.
    let mut b = ServiceBuilder::new(&veh, ServiceConfig::new(addr("vehicle-01/teleop")));
    let teleop = twist_cmd_v1::Server::declare(&mut b).await.unwrap();
    let _teleop = b.start().await.unwrap();

    // The thruster: thruster.v1, its `cmd` role bound to the commander.
    let handlers = Arc::new(Thruster::default());
    let mut b = ServiceBuilder::new(
        &veh,
        ServiceConfig::new(addr("vehicle-01/thruster-l")).bind("cmd", &["vehicle-01/teleop"]),
    );
    let thr = thruster_v1::Server::declare(&mut b, Arc::clone(&handlers))
        .await
        .unwrap();
    assert_eq!(thr.operations().len(), 2);
    let thr_svc = b.start().await.unwrap();
    let cmds: Arc<Mutex<Vec<f64>>> = Arc::default();
    let c = Arc::clone(&cmds);
    let _cmd_sub = twist_cmd_v1::Consumer::bind(&thr_svc, thruster_v1::role::CMD)
        .unwrap()
        .subscribe_cmd(move |r| c.lock().unwrap().push(r.value.unwrap().linear_x_mps))
        .await
        .unwrap();

    // The monitor: a service whose manifest requires thruster.v1.
    let mut b = ServiceBuilder::new(
        &ground,
        ServiceConfig::new(addr("ground/monitor")).bind("thrusters", &["vehicle-01/*"]),
    );
    b.require("thrusters", thruster_v1::iface(), false);
    let monitor = b.start().await.unwrap();
    let thrusters = thruster_v1::Consumer::bind(&monitor, "thrusters").unwrap();
    let telemetry: Arc<Mutex<Vec<(String, f64)>>> = Arc::default();
    let t = Arc::clone(&telemetry);
    let _sub = thrusters
        .subscribe_telemetry(move |r| {
            t.lock()
                .unwrap()
                .push((r.provider.to_string(), r.value.unwrap().rpm));
        })
        .await
        .unwrap();

    eventually("the commander's and the thruster's data cross", || async {
        teleop
            .cmd
            .put(&twist_cmd_v1::types::Twist {
                linear_x_mps: 0.5,
                ..Default::default()
            })
            .await
            .unwrap();
        thr.telemetry
            .put(&thruster_v1::types::Telemetry {
                rpm: 1200.0,
                current_a: 3.5,
                temperature_c: 41.0,
            })
            .await
            .unwrap();
        (!telemetry.lock().unwrap().is_empty() && !cmds.lock().unwrap().is_empty()).then_some(())
    })
    .await;
    assert_eq!(
        telemetry.lock().unwrap()[0],
        ("vehicle-01/thruster-l".to_owned(), 1200.0)
    );
    assert_eq!(cmds.lock().unwrap()[0], 0.5);

    // State: stamped by the owner, read from it decoded (S1, S4).
    thr.status
        .put(&thruster_v1::types::Status {
            armed: false,
            active_commander: String::new(),
        })
        .await
        .unwrap();
    let got = eventually("the owner answers its state", || async {
        match thrusters.get_status(T).await.unwrap() {
            StateGet::Answered(v) => Some(v),
            StateGet::Silent => None,
        }
    })
    .await;
    let zenkey::typed::Current::Value { value, .. } = &got[0] else {
        panic!("{got:?}")
    };
    assert!(!value.as_ref().unwrap().armed);

    // A tool's typed call; the metadata reaches the handler (O7).
    let tool = thruster_v1::Client::new(&ground, &["vehicle-01/*"])
        .unwrap()
        .with_metadata(CallMetadata::new("op@ws-01", "r-1"));
    let at = addr("vehicle-01/thruster-l");
    let out = eventually("the thruster answers", || async {
        let out = thruster_v1::Api::arm(
            &tool,
            &at,
            thruster_v1::types::ArmRequest {
                reason: "mission start".into(),
            },
        )
        .await
        .unwrap();
        out.answer().is_some().then_some(out)
    })
    .await;
    let Outcome::Value(status) = out else {
        unreachable!()
    };
    assert!(status.armed);
    assert_eq!(status.active_commander, "vehicle-01/teleop");
    assert!(
        handlers
            .seen
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.as_ref().and_then(|m| m.request_id.as_deref()) == Some("r-1"))
    );
    // A handler's refusal is the error envelope (O3).
    let refused = thruster_v1::Api::arm(&tool, &at, thruster_v1::types::ArmRequest::default())
        .await
        .unwrap();
    assert_eq!(refused.refusal().unwrap().code, "invalid_request");
    let disarmed = thruster_v1::Api::disarm(&tool, &at, ()).await.unwrap();
    assert!(!disarmed.answer().unwrap().armed);
}

/// The tcgui backend: interface control and diagnostics, JSON Schema types.
struct Netif;

impl tc_netif_v1::Handlers for Netif {
    async fn interfaces_set(
        &self,
        request: tc_netif_v1::types::InterfaceControlRequest,
        call: CallInfo,
    ) -> Result<tc_netif_v1::types::InterfaceControlResponse, OpError> {
        if call.value("iface") == Some("eth9") {
            return Err(OpError::app_as::<Json<TcError>>(
                "the kernel refused",
                &TcError {
                    kind: TcErrorKind::NotFound,
                    message: "no such device".into(),
                },
            ));
        }
        Ok(tc_netif_v1::types::InterfaceControlResponse {
            message: Some(format!(
                "{} {}",
                call.value("ns").unwrap(),
                call.value("iface").unwrap()
            )),
            new_state: matches!(
                request.operation,
                tc_netif_v1::types::InterfaceControlRequestOperation::Enable
            ),
        })
    }

    async fn diagnostics(
        &self,
        request: tc_netif_v1::types::DiagnosticsRequest,
        _call: CallInfo,
    ) -> Result<tc_netif_v1::types::DiagnosticsResponse, OpError> {
        let mut results = serde_json::Map::new();
        results.insert("link".into(), serde_json::json!("up"));
        Ok(tc_netif_v1::types::DiagnosticsResponse {
            message: Some(request.interface),
            results,
        })
    }
}

/// The tcgui pilot on the generated code: a `tc` backend per host serving
/// a templated operation and a fan-out one, with typed state members
/// written after start; the frontend calling, fanning out and reading.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tcgui_backend_is_called_fanned_out_and_read() {
    let (_r, ep) = router().await;
    let hosts = client(&ep).await;
    let ui = client(&ep).await;

    let mut servers = Vec::new();
    for h in ["h-1", "h-2"] {
        let mut b = ServiceBuilder::new(&hosts, ServiceConfig::new(addr(&format!("{h}/tc"))));
        let server = tc_netif_v1::Server::declare(&mut b, Arc::new(Netif))
            .await
            .unwrap();
        let mut svc = b.start().await.unwrap();
        server
            .namespaces
            .put(&tc_netif_v1::types::Namespaces {
                namespaces: vec!["default".into()],
            })
            .await
            .unwrap();
        // A templated state member, after start (§8.2: exposed by declare).
        let eth0 = server
            .interfaces(&mut svc, "default", "eth0")
            .await
            .unwrap();
        let nic: tc_netif_v1::types::NetworkInterface = serde_json::from_value(
            serde_json::json!({"name": "eth0", "index": 2, "namespace": "default", "is_up": true}),
        )
        .unwrap();
        eth0.put(&nic).await.unwrap();
        servers.push((server, svc, eth0));
    }

    let tool = tc_netif_v1::Client::new(&ui, &["*/tc"]).unwrap();
    let h1 = addr("h-1/tc");
    let enable = || tc_netif_v1::types::InterfaceControlRequest {
        operation: tc_netif_v1::types::InterfaceControlRequestOperation::Enable,
    };
    let out = eventually("h-1 answers", || async {
        let out = tc_netif_v1::Api::interfaces_set(&tool, &h1, "default", "eth0", enable())
            .await
            .unwrap();
        out.answer().is_some().then_some(out)
    })
    .await;
    let resp = out.answer().unwrap();
    assert!(resp.new_state);
    assert_eq!(resp.message.as_deref(), Some("default eth0"));

    // The app detail rides the JSON envelope as the hinted type (§5.2).
    let refused = tc_netif_v1::Api::interfaces_set(&tool, &h1, "default", "eth9", enable())
        .await
        .unwrap();
    let env = refused.refusal().unwrap();
    assert_eq!(env.code, "app");
    let Some(Detail::Value(v)) = &env.detail else {
        panic!("{env:?}")
    };
    let detail: TcError = serde_json::from_value(v.clone()).unwrap();
    assert_eq!(detail.kind, TcErrorKind::NotFound);

    // A fan-out (O2): every host answers, attributed.
    let fleet = tool.fleet().with_timeout(T);
    let replies = eventually("both hosts answer the fan-out", || async {
        let r = fleet
            .diagnostics(tc_netif_v1::types::DiagnosticsRequest {
                interface: "eth0".into(),
                namespace: "default".into(),
                target: None,
                timeout_ms: None,
            })
            .await
            .unwrap();
        (r.repliers.len() == 2).then_some(r)
    })
    .await;
    let mut who: Vec<String> = replies
        .repliers
        .iter()
        .map(|r| r.addr.to_string())
        .collect();
    who.sort();
    assert_eq!(who, ["h-1/tc", "h-2/tc"]);
    assert!(replies.values().all(|v| v.results["link"] == "up"));

    // Current state, typed, from every bound provider (S4).
    let mut b = ServiceBuilder::new(
        &ui,
        ServiceConfig::new(addr("ws-01/tcgui")).bind("netif", &["*/tc"]),
    );
    b.require("netif", tc_netif_v1::iface(), false);
    let frontend = b.start().await.unwrap();
    let netif = tc_netif_v1::Consumer::bind(&frontend, "netif").unwrap();
    let members = eventually("both hosts answer for eth0", || async {
        match netif
            .get_interfaces_member("default", "eth0", T)
            .await
            .unwrap()
        {
            StateGet::Answered(v) if v.len() == 2 => Some(v),
            _ => None,
        }
    })
    .await;
    for m in members {
        let zenkey::typed::Current::Value { value, .. } = m else {
            panic!("a value")
        };
        assert_eq!(value.unwrap().index, 2);
    }
}

/// The fleet manager's listing (`replies = "many"`) and its plans, a
/// templated state resource.
struct Plans;

impl mission_plan_v1::Handlers for Plans {
    async fn list(
        &self,
        request: mission_plan_v1::types::ListRequest,
        _call: CallInfo,
        out: Sink<mission_plan_v1::types::PlanSummary, ()>,
    ) -> Result<(), OpError> {
        for v in ["vehicle-01", "vehicle-02", "boat-01"] {
            if v.starts_with(&request.vehicle_prefix) {
                out.send(mission_plan_v1::types::PlanSummary {
                    vehicle: v.to_owned(),
                    revision: 3,
                    waypoint_count: 2,
                })
                .await?;
            }
        }
        Ok(())
    }
}

/// A navigator without the optional `reset`.
struct Navigator;

impl nav_v2::Handlers for Navigator {
    async fn set_origin(
        &self,
        request: nav_v2::types::SetOriginRequest,
        _call: CallInfo,
    ) -> Result<nav_v2::types::Origin, OpError> {
        request
            .origin
            .ok_or_else(|| OpError::invalid_request("no origin"))
    }
}

/// Many replies through a `Sink` and back as typed `Replies` (O6); a
/// templated state member read by its parameters; an optional operation
/// gated on a capability not held, which `Server::declare` skips and the
/// runtime answers `unavailable` (O3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_replies_templated_state_and_an_absent_option() {
    let (_r, ep) = router().await;
    let s = client(&ep).await;

    let mut b = ServiceBuilder::new(&s, ServiceConfig::new(addr("ground/fleet-mgr")));
    let plans = mission_plan_v1::Server::declare(&mut b, Arc::new(Plans))
        .await
        .unwrap();
    let mut mgr = b.start().await.unwrap();
    let v1 = plans.plans(&mut mgr, "vehicle-01").await.unwrap();
    v1.put(&mission_plan_v1::types::Plan {
        revision: 3,
        waypoints: vec![mission_plan_v1::types::Waypoint {
            latitude_deg: 48.1,
            longitude_deg: 2.3,
            speed_mps: 1.5,
        }],
        valid_until_ns: 0,
    })
    .await
    .unwrap();

    let mut b = ServiceBuilder::new(&s, ServiceConfig::new(addr("vehicle-01/nav")));
    let nav = nav_v2::Server::declare(&mut b, Arc::new(Navigator))
        .await
        .unwrap();
    assert_eq!(nav.operations().len(), 1, "reset is absent: no reset_line");
    let _nav = b.start().await.unwrap();

    let fm = mission_plan_v1::Client::new(&s, &["ground/fleet-mgr"]).unwrap();
    let at = addr("ground/fleet-mgr");
    let listing = eventually("the fleet manager lists", || async {
        let r = mission_plan_v1::Api::list(
            &fm,
            &at,
            mission_plan_v1::types::ListRequest {
                vehicle_prefix: "vehicle-".into(),
            },
        )
        .await
        .unwrap();
        (!r.is_silent()).then_some(r)
    })
    .await;
    let names: Vec<&str> = listing.values().map(|p| p.vehicle.as_str()).collect();
    assert_eq!(names, ["vehicle-01", "vehicle-02"]);

    let mut b = ServiceBuilder::new(
        &s,
        ServiceConfig::new(addr("vehicle-01/executor")).bind("plan", &["ground/fleet-mgr"]),
    );
    b.require("plan", mission_plan_v1::iface(), false);
    let exec = b.start().await.unwrap();
    let plan = mission_plan_v1::Consumer::bind(&exec, "plan").unwrap();
    let got = eventually("the owner answers for vehicle-01", || async {
        match plan.get_plans_member("vehicle-01", T).await.unwrap() {
            StateGet::Answered(v) => Some(v),
            StateGet::Silent => None,
        }
    })
    .await;
    let zenkey::typed::Current::Value { value, .. } = &got[0] else {
        panic!("{got:?}")
    };
    assert_eq!(value.as_ref().unwrap().revision, 3);

    let navc = nav_v2::Client::new(&s, &["vehicle-01/nav"]).unwrap();
    let nav_at = addr("vehicle-01/nav");
    let out = eventually("the navigator answers reset", || async {
        let out = nav_v2::Api::reset(&navc, &nav_at, ()).await.unwrap();
        out.refusal().is_some().then_some(out)
    })
    .await;
    let env = out.refusal().unwrap();
    assert_eq!(
        (env.code.as_str(), env.cause.as_deref()),
        ("unavailable", Some("capability"))
    );
}
