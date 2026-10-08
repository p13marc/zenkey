//! zenkey-build's test crate (#611): the generated modules of the
//! walkthrough's and the tcgui pilot's contracts, and the hand-written
//! types a name hint binds.

/// The generated code.
pub mod zk2 {
    include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
}

/// Hand-written types, bound by name hints (G23): the Rust-first path.
pub mod model {
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    /// The tcgui pilot's error detail (`json:TcError`, `schemas/tc.json`).
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    pub struct TcError {
        pub kind: TcErrorKind,
        pub message: String,
    }

    /// What went wrong.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    pub enum TcErrorKind {
        Validation,
        Kernel,
        NotFound,
        Busy,
    }
}

/// A component written against the generated `Api`: it arms every thruster
/// it is given and reports which ones are armed. It needs no bus; its test
/// drives it with a double.
pub async fn arm_all<A: zk2::thruster_v1::Api>(
    api: &A,
    thrusters: &[zenkey::model::grammar::Addr],
    reason: &str,
) -> zenkey::Result<Vec<bool>> {
    use zk2::thruster_v1::types::ArmRequest;
    let mut armed = Vec::new();
    for t in thrusters {
        let out = api
            .arm(
                t,
                ArmRequest {
                    reason: reason.to_owned(),
                },
            )
            .await?;
        armed.push(out.answer().is_some_and(|s| s.armed));
    }
    Ok(armed)
}
