//! The contract crate's default build, with no zenoh (#611): the types, the
//! embedded bundle and the traits are there, and a component is tested
//! through the `Api` with a double.

use std::future::Future;
use std::task::{Context, Poll, Waker};

use zenkey::call::Outcome;
use zenkey::model::grammar::Addr;
use zenkey::{CallInfo, OpError};
use zk2_walkthrough_contract::thruster_v1::{self, Api, Handlers, types};

/// These futures never wait: polled once, they are done.
fn now<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("a double's future waited"),
    }
}

/// An out-of-tree implementation of the thruster's operations.
struct Hardware;

impl Handlers for Hardware {
    async fn arm(
        &self,
        request: types::ArmRequest,
        _call: CallInfo,
    ) -> Result<types::Status, OpError> {
        if request.reason.is_empty() {
            return Err(OpError::invalid_request("a reason is required"));
        }
        Ok(types::Status {
            armed: true,
            active_commander: String::new(),
        })
    }

    async fn disarm(&self, _request: (), _call: CallInfo) -> Result<types::Status, OpError> {
        Ok(types::Status::default())
    }
}

/// A double of the calls, answering through the implementation above.
struct Loopback(Hardware);

impl Api for Loopback {
    async fn arm(
        &self,
        _at: &Addr,
        request: types::ArmRequest,
    ) -> zenkey::Result<Outcome<types::Status>> {
        Ok(match self.0.arm(request, CallInfo::default()).await {
            Ok(s) => Outcome::Value(s),
            Err(e) => Outcome::Refused(e.into_envelope()),
        })
    }

    async fn disarm(&self, _at: &Addr, request: ()) -> zenkey::Result<Outcome<types::Status>> {
        Ok(Outcome::Value(
            self.0.disarm(request, CallInfo::default()).await.unwrap(),
        ))
    }
}

#[test]
fn the_traits_and_the_bundle_need_no_zenoh() {
    let api = Loopback(Hardware);
    let at: Addr = "vehicle-01/thruster-l".parse().unwrap();
    let armed = now(api.arm(
        &at,
        types::ArmRequest {
            reason: "go".into(),
        },
    ))
    .unwrap();
    assert!(armed.answer().unwrap().armed);
    let refused = now(api.arm(&at, types::ArmRequest::default())).unwrap();
    assert_eq!(refused.refusal().unwrap().code, "invalid_request");

    let imp = thruster_v1::implementation();
    assert_eq!(imp.fingerprint().to_string(), thruster_v1::FINGERPRINT);
    assert_eq!(imp.bundle_bytes().as_ref(), thruster_v1::BUNDLE);
    assert_eq!(thruster_v1::resource::ARM, "@op/arm");
}
