//! The `@rpc` plane: producers, their procedures, and what a procedure
//! declares (RFC 05 §3, RFC 08 §6.1).

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ServiceRow {
    pub producer: String,
    pub registry_version: String,
    pub kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceList {
    pub procedures: Vec<ServiceRow>,
}

/// One procedure, with everything the registry declares about it and the key
/// shape a caller would actually use (issue #211).
#[derive(Debug, Clone, Serialize)]
pub struct ServiceProcedure {
    pub path: String,
    pub kind: String,
    /// The base-relative `@rpc` key this procedure answers on, with `{origin}`
    /// standing for the publishing identity. A service origin has no producer
    /// chunk (RFC 06 §5), so the two shapes genuinely differ and the reader
    /// should not have to reconstruct which applies.
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    /// `forbidden` means the fleet spelling does not exist for this procedure
    /// (RFC 08 §1.1 G2) — worth seeing *before* reaching for `*`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fanout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One producer's `@rpc` surface — the verb a user reaches for immediately
/// before `service call` (issue #211).
///
/// `service list` says *what exists* across the fleet; this says *what one
/// producer offers and how to call it*, which is the question left over.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceInfo {
    pub producer: String,
    pub registry_version: String,
    /// Present when this producer is a service origin (RFC 06 §5) — its keys
    /// carry no producer chunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub procedures: Vec<ServiceProcedure>,
}
