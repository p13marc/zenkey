//! The node plane (RFC 04 §5): who is up, what they run, and which
//! deployment bases were seen at all.

use serde::Serialize;

/// One producer on one origin — row-shaped so a `--watch` loop can diff it
/// and a GUI can select it (`origin/producer` is the stable row identity).
#[derive(Debug, Clone, Serialize)]
pub struct NodeRow {
    pub origin: String,
    /// Live producer name (from the liveliness token; zero payload).
    pub producer: String,
    /// The producing app, when a registry slice joined (`--verbose`).
    /// `None` = not asked / no slice — never a default (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Registry version from the joined slice, same provenance rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeList {
    pub nodes: Vec<NodeRow>,
    /// Whether a slice join was even attempted (`--verbose`) — keeps "asked,
    /// no slice served" distinguishable from "not asked" in rows whose
    /// `app`/`registry_version` are `None` (O4).
    pub slices_joined: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseList {
    /// Discovered bases; `base` is a plain string, `""` for the empty base.
    pub bases: Vec<crate::DiscoveredBase>,
}

/// One producer's story on one node — the enrichment §6.3 promised
/// (issue #40). Every field is honest about its provenance: absent
/// introspection is `None`, never a default (RFC 09 §5.1 O4).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProducerInfo {
    pub name: String,
    /// A liveliness token stands (RFC 04 §5 — the only presence signal).
    pub alive: bool,
    /// From this origin's served introspect slice, when it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_version: Option<String>,
    pub subjects: usize,
    pub procedures: usize,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub blob_tiers: Vec<String>,
    /// Declared `@media` streams (RFC 08 §2/§6, v1.16 — the slice finally
    /// carries what §6 claimed through v1.7): discoverable off the bus, so
    /// a viewer can enumerate streams without a compiled-in registry.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub media: Vec<MediaStreamInfo>,
    /// Deprecated subjects this build still serves — RFC 08 §6's headline
    /// buy ("which hosts still serve a deprecated subject").
    pub deprecated_served: usize,
    /// It answered `introspect`, and the slice could not be read (#491).
    ///
    /// The third pole beside the other two this row already spells: a
    /// served slice is `app` + `registry_version` present, no reply is both
    /// absent *and* this absent, and this present is "answered, unreadable"
    /// — which RFC 08 §6 (v1.44) says a consumer reports as such, "never
    /// that there was none", because "could not read what it said" and "it
    /// said nothing" are two answers (RFC 13 §3 O4). Absent in every other
    /// case, so a document without it is byte-identical to the one before.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub unreadable: Option<UnreadableSlice>,
}

/// An `introspect` reply that arrived and could not be read as a registry
/// slice (RFC 08 §6, v1.44; #491): what it declared, and the first line of
/// why it did not read.
///
/// Three causes land here, and the shape does not tell them apart because
/// the reader does not need it to — the error line does: a declared
/// `Encoding` that is neither spelling (`application/json`), a document
/// malformed in the spelling it declared (a TOML file declared
/// `application/kdl` is read *as KDL*, never sniffed back), or one that
/// parses but is not a slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadableSlice {
    /// The reply's `Encoding`, verbatim — `None` when it declared none,
    /// which is itself the fact (the undeclared row of RFC 08 §6's table
    /// reads as TOML), not an unasked question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The parse error's first line. A TOML error's later lines are a
    /// caret diagram of the source, which a table row cannot carry and a
    /// script does not want.
    pub error: String,
}

impl UnreadableSlice {
    /// From the reply's declared encoding and whatever refused it — the
    /// first line of its `Display`, trimmed.
    pub fn new(encoding: Option<String>, error: &dyn std::fmt::Display) -> UnreadableSlice {
        let error = error.to_string();
        UnreadableSlice {
            encoding,
            error: error.lines().next().unwrap_or_default().trim().to_string(),
        }
    }

    /// The one sentence both explorers draw (#491): "introspect answered,
    /// slice unreadable (`application/kdl`: …)" — the house counterpart of
    /// "no introspect reply — capabilities unknown, not absent", and like
    /// it a statement about the fetch, never about the producer's
    /// capabilities.
    pub fn sentence(&self) -> String {
        let declared = match &self.encoding {
            Some(e) => format!("`{e}`"),
            None => "no encoding declared".to_string(),
        };
        format!(
            "introspect answered, slice unreadable ({declared}: {})",
            self.error
        )
    }
}

/// One declared media stream, as the slice states it (RFC 08 §2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MediaStreamInfo {
    /// The stream pattern after `@media/<producer>/`.
    pub path: String,
    /// The declared wire encoding (`image/jpeg`, `video/*`).
    pub encoding: String,
}

/// Freshness of one declared state subject on this node (RFC 04 §1.2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Freshness {
    pub producer: String,
    pub path: String,
    pub ttl_s: i64,
    /// Seconds since the newest matching sample's HLC stamp; `None` when no
    /// sample answered — which is "not seen", not "fresh" (O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_s: Option<i64>,
    /// `age > ttl`, or no sample at all for a declared live subject.
    pub stale: bool,
}

/// One node, joined: liveliness × introspect × state freshness.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeInfo {
    pub origin: String,
    pub producers: Vec<ProducerInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub freshness: Vec<Freshness>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The third pole, pinned (#491): present only when the reply answered
    /// unreadably, carrying the declaration verbatim and one line of error —
    /// and an undeclared reply's absent `encoding` is absence, not `null`.
    #[test]
    fn an_unreadable_slice_is_its_declaration_and_one_line() {
        let toml_err =
            "malformed registry slice: TOML parse error at line 1, column 1\n  |\n1 | {\n  | ^";
        let u = UnreadableSlice::new(Some("application/kdl".into()), &toml_err);
        assert_eq!(
            serde_json::to_value(&u).unwrap(),
            json!({
                "encoding": "application/kdl",
                "error": "malformed registry slice: TOML parse error at line 1, column 1",
            })
        );
        assert_eq!(
            u.sentence(),
            "introspect answered, slice unreadable (`application/kdl`: malformed registry \
             slice: TOML parse error at line 1, column 1)"
        );
        let undeclared = UnreadableSlice::new(None, &"malformed registry slice: no [registry]");
        assert_eq!(
            serde_json::to_value(&undeclared).unwrap(),
            json!({"error": "malformed registry slice: no [registry]"})
        );
        assert!(undeclared.sentence().contains("(no encoding declared: "));
    }
}
