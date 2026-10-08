//! What the generated code gives without a bus (#611): a component driven
//! through the generated `Api` with a double, handlers driven directly, the
//! embedded bundles, and a hinted type against its committed schema.

use std::path::Path;
use std::sync::Mutex;

use zenkey::call::Outcome;
use zenkey::model::bundle::Bundle;
use zenkey::model::contract::load_path;
use zenkey::model::grammar::Addr;
use zenkey::{CallInfo, OpError, Sink};
use zenkey_codegen_test::zk2::{mission_plan_v1, nav_v2, tc_netif_v1, thruster_v1};
use zenkey_codegen_test::{arm_all, model};

fn addr(s: &str) -> Addr {
    s.parse().unwrap()
}

fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/zk2")
}

/// A double of the thruster's operations: `thruster-l` arms, `thruster-r`
/// refuses, anything else is silent.
#[derive(Default)]
struct Thrusters {
    calls: Mutex<Vec<(String, String)>>,
}

impl thruster_v1::Api for Thrusters {
    async fn arm(
        &self,
        at: &Addr,
        request: thruster_v1::types::ArmRequest,
    ) -> zenkey::Result<Outcome<thruster_v1::types::Status>> {
        self.calls
            .lock()
            .unwrap()
            .push((at.to_string(), request.reason));
        Ok(match at.service.as_str() {
            "thruster-l" => Outcome::Value(thruster_v1::types::Status {
                armed: true,
                active_commander: String::new(),
            }),
            "thruster-r" => Outcome::Refused(OpError::busy("interlocked").into_envelope()),
            _ => Outcome::NoAnswer(zenkey::call::Silence {
                attempts: 1,
                transport: None,
                presence: zenkey::call::Attribution::Absent,
            }),
        })
    }

    async fn disarm(
        &self,
        _at: &Addr,
        _request: (),
    ) -> zenkey::Result<Outcome<thruster_v1::types::Status>> {
        unreachable!("the component never disarms")
    }
}

/// The done-when: the generated `Api` drives a unit test with a double.
#[tokio::test]
async fn a_component_is_tested_through_the_api_with_a_double() {
    let double = Thrusters::default();
    let armed = arm_all(
        &double,
        &[
            addr("vehicle-01/thruster-l"),
            addr("vehicle-01/thruster-r"),
            addr("vehicle-01/thruster-x"),
        ],
        "mission start",
    )
    .await
    .unwrap();
    assert_eq!(armed, [true, false, false]);
    let calls = double.calls.lock().unwrap();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|(_, r)| r == "mission start"));
}

/// The owner's mission-plan listing, a `replies = "many"` operation.
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

/// The navigator implements the required operation and leaves the optional
/// `reset` to its default.
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

#[tokio::test]
async fn handlers_are_driven_without_a_bus() {
    use mission_plan_v1::Handlers as _;
    use nav_v2::Handlers as _;

    let (sink, got) = Sink::memory();
    Plans
        .list(
            mission_plan_v1::types::ListRequest {
                vehicle_prefix: "vehicle-".into(),
            },
            CallInfo::default(),
            sink,
        )
        .await
        .unwrap();
    let names: Vec<String> = got
        .lock()
        .unwrap()
        .values
        .iter()
        .map(|p| p.vehicle.clone())
        .collect();
    assert_eq!(names, ["vehicle-01", "vehicle-02"]);

    let e = Navigator
        .reset((), CallInfo::default())
        .await
        .expect_err("an optional operation answers unavailable by default");
    assert_eq!(e.code(), "unavailable");
    assert_eq!(e.envelope().cause.as_deref(), Some("build"));
    let o = nav_v2::types::Origin {
        latitude_deg: 48.0,
        longitude_deg: 2.0,
        altitude_m: 35.0,
    };
    let set = nav_v2::types::SetOriginRequest { origin: Some(o) };
    assert_eq!(
        Navigator
            .set_origin(set, CallInfo::default())
            .await
            .unwrap(),
        o
    );
}

/// A double of `tc.netif.v1`: the `Api` of a templated operation takes its
/// parameters as arguments.
struct Netif;

impl tc_netif_v1::Api for Netif {
    async fn interfaces_set(
        &self,
        _at: &Addr,
        ns: &str,
        iface: &str,
        request: tc_netif_v1::types::InterfaceControlRequest,
    ) -> zenkey::Result<Outcome<tc_netif_v1::types::InterfaceControlResponse>> {
        assert_eq!((ns, iface), ("default", "eth0"));
        Ok(Outcome::Value(
            tc_netif_v1::types::InterfaceControlResponse {
                message: None,
                new_state: matches!(
                    request.operation,
                    tc_netif_v1::types::InterfaceControlRequestOperation::Enable
                ),
            },
        ))
    }

    async fn diagnostics(
        &self,
        _at: &Addr,
        _request: tc_netif_v1::types::DiagnosticsRequest,
    ) -> zenkey::Result<Outcome<tc_netif_v1::types::DiagnosticsResponse>> {
        Ok(Outcome::Value(tc_netif_v1::types::DiagnosticsResponse {
            message: None,
            results: Default::default(),
        }))
    }
}

#[tokio::test]
async fn a_templated_operation_takes_its_parameters() {
    use tc_netif_v1::Api as _;
    let out = Netif
        .interfaces_set(
            &addr("h-1/tc"),
            "default",
            "eth0",
            tc_netif_v1::types::InterfaceControlRequest {
                operation: tc_netif_v1::types::InterfaceControlRequestOperation::Enable,
            },
        )
        .await
        .unwrap();
    assert!(out.answer().unwrap().new_state);
}

/// The embedded bundle is byte for byte the one `zenkey-model` builds from
/// the source, and the one published in the history.
#[test]
fn the_embedded_bundle_is_the_published_one() {
    let source = load_path(&examples().join("walkthrough/thruster.v1.toml"))
        .contract
        .unwrap();
    assert_eq!(
        thruster_v1::BUNDLE,
        Bundle::build(&source).to_bytes().as_slice()
    );
    let imp = thruster_v1::implementation();
    assert_eq!(imp.fingerprint().to_string(), thruster_v1::FINGERPRINT);
    assert_eq!(imp.iface(), &thruster_v1::iface());
    let published = examples().join(format!(
        ".history/thruster.v1/{}.bundle.json",
        thruster_v1::FINGERPRINT.trim_start_matches("sha256:")
    ));
    assert_eq!(std::fs::read(published).unwrap(), thruster_v1::BUNDLE);
    assert!(
        thruster_v1::contract()
            .resource(zenkey::model::grammar::KindToken::Op, "arm")
            .is_some()
    );
}

/// The Rust-first path: the hinted `TcError` and the committed schema agree,
/// read through the zk2 subset.
#[test]
fn the_hinted_type_matches_its_committed_schema() {
    zenkey_build::check_schema::<model::TcError>(
        examples().join("tcgui/schemas/tc.json"),
        "TcError",
    )
    .unwrap();
    // The generated modules name the hinted type, not a generated one.
    let e: tc_netif_v1::types::TcError = model::TcError {
        kind: model::TcErrorKind::Busy,
        message: "locked".into(),
    };
    assert_eq!(e.kind, model::TcErrorKind::Busy);
}
