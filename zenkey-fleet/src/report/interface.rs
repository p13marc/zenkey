//! The interface plane: payload types and the carriers that move them —
//! one type's producers, subjects and media streams in one document.

use super::asked::Asked;
use super::schema::SchemaRow;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceTypeRow {
    pub name: String,
    pub carriers: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceList {
    pub types: Vec<InterfaceTypeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CarrierRow {
    pub producer: String,
    pub class: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterfaceShow {
    pub type_name: String,
    pub carriers: Vec<CarrierRow>,
    /// What each producer serving this type name says its schema is
    /// (issue #51). `NotAsked` = `--schema` was not passed, so the bus was
    /// never asked; `Asked(vec![])` = asked and no carrier served one — the
    /// empty `Vec` used to conflate the two (RFC 09 §5.1 O4, review finding
    /// R4). Two rows with different hashes *is* the RFC 08 §7 drift finding,
    /// visible right here rather than only in `doctor`.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub schemas: Asked<Vec<SchemaRow>>,
}
