//! The schema plane (RFC 08 §7): what a producer serves for a type name,
//! and the drift between what was served and what was declared.

use super::asked::Asked;
use serde::Serialize;

/// One type's schema entry as one producer serves it (issue #51).
#[derive(Debug, Clone, Serialize)]
pub struct SchemaRow {
    pub producer: String,
    pub type_name: String,
    pub kind: String,
    pub hash: String,
    /// The schema document, when the caller asked for the full form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

/// One producer's served `describe` reply, rendered (issue #51).
///
/// `served = false` is the honest degradation RFC 08 §7 leaves room for —
/// `describe` is a SHOULD, so a producer that serves none has said nothing
/// about its types, which is not the same as having no types.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaDump {
    pub producer: String,
    pub served: bool,
    /// The declaring app, as the served set names it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub types: Vec<SchemaRow>,
    /// Registry-declared type names this producer's set does **not** cover —
    /// RFC 08 §7's totality clause, checked where the user is already looking.
    /// `NotAsked` = no registry was loaded, so totality was never checked —
    /// not asked is not answered no (RFC 09 §5.1 O4); `Asked(vec![])` is the
    /// actual clean bill.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub missing: Asked<Vec<String>>,
}

/// Two producers serving one type name with different hashes — "a `doctor`
/// finding" by RFC 08 §7's own words (issue #41).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SchemaDrift {
    pub type_name: String,
    /// Every (producer, hash) pair observed for the name.
    pub servers: Vec<(String, String)>,
}

/// A type the producer's slice references that its served describe set does
/// not cover — a violation of RFC 08 §7's totality clause.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TotalityGap {
    pub producer: String,
    pub missing: Vec<String>,
}
