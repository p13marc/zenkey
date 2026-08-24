//! The `@blob` plane (RFC 07 §2, issues #58/#68).
//!
//! Deliberately **not** feature-gated, and deliberately carrying no `zblob`
//! type. `zenkey-fleet`'s blob *transport* is optional (the `blob` feature);
//! its blob *output shape* is not, because a report is a contract:
//! `zenctl blob locate --format json` must serialize the same document
//! whether or not the binary was built with the transport, and a frontend
//! must be able to render a probe it deserialized from somewhere else
//! entirely.

use super::asked::Asked;
use super::call::CallError;
use serde::Serialize;

// ─── the @blob plane (RFC 07 §2, issues #58/#68) ────────────────────────────
//
// These are deliberately **not** feature-gated, and deliberately carry no
// `zblob` type. `zenkey-fleet`'s blob *transport* is optional (the `blob`
// feature); its blob *output shape* is not, because a report is a contract:
// `zenctl blob locate --format json` must serialize the same document whether
// or not the binary was built with the transport, and a frontend must be able
// to render a probe it deserialized from somewhere else entirely.

/// Where a [`BlobList`]'s rows came from — the O5 provenance line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlobListSource {
    /// Introspect slices served by live producers (RFC 08 §6).
    Bus,
    /// `--registry <dir>` TOMLs.
    RegistryDirs,
    /// Both, unioned.
    Union,
}

/// One producer's declaration that it serves one `@blob` tier.
#[derive(Debug, Clone, Serialize)]
pub struct BlobTierRow {
    pub producer: String,
    pub registry_version: String,
    /// The tier token **as declared**. A token RFC 07 §2 does not reserve is
    /// carried verbatim and flagged by `known_tier` rather than dropped: a
    /// declaration we do not understand is a fact about the fleet (RFC 09
    /// §5.1 O1), not noise.
    pub tier: String,
    /// Whether `tier` is one of the three reserved tokens.
    pub known_tier: bool,
    /// Declared endpoints (`artifact` only, RFC 07 §2.2).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<String>,
    /// Content-hash algorithm (`store` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algo: Option<String>,
    /// The type whose payload carries the content root (RFC 07 §2.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The blob *content*'s encoding, when declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Origins whose liveliness roster names this producer.
    ///
    /// `NotAsked` means the roster was never asked — an offline `--registry`
    /// read learns nothing about who is up, and rendering that as "no origin
    /// serves this tier" would report a verdict nobody obtained (RFC 09 §5.1
    /// O4). Even when asked it is a *capability* claim: a producer that
    /// declares a tier is saying it serves the endpoints, never that it holds
    /// any particular blob. Only a probe answers that.
    #[serde(skip_serializing_if = "Asked::is_not_asked", default)]
    pub origins: Asked<Vec<String>>,
}

/// Which producers declare which `@blob` tiers (RFC 07 §2.7 / 08 §2).
#[derive(Debug, Clone, Serialize)]
pub struct BlobList {
    pub tiers: Vec<BlobTierRow>,
    pub source: BlobListSource,
    /// How many slices were read. Without it an empty `tiers` reads as "nobody
    /// serves blobs" when it may mean "nothing was asked" (RFC 09 §5.1 O6).
    pub slices_considered: usize,
    /// How many of those declared no `@blob` tier at all.
    pub slices_without_blob: usize,
}

/// A holder's chunk availability for one artifact (RFC 07 §2.5's `have`).
#[derive(Debug, Clone, Serialize)]
pub struct BlobAvailability {
    pub chunk_count: u32,
    /// Chunks this holder can serve right now.
    pub have: u32,
    pub complete: bool,
}

/// A holder's manifest for one artifact (RFC 07 §2.2's `manifest`).
#[derive(Debug, Clone, Serialize)]
pub struct BlobManifest {
    pub id: String,
    /// Advisory only. It is never joined to any path — a remote party does not
    /// choose where bytes land.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub total_len: u64,
    pub chunk_size: u32,
    pub chunk_count: u32,
    /// The content root, hex — RFC 07 §2.1's integrity anchor.
    pub root: String,
    pub created_ms: i64,
}

/// One origin that answered a probe, and what it said.
#[derive(Debug, Clone, Serialize)]
pub struct BlobHolder {
    /// From the reply's **own** key. `"?"` only when that key neither parsed
    /// under the base nor had an origin in position 1.
    pub origin: String,
    /// The concrete key this origin answered on — the only fetchable form
    /// (RFC 07 §2.5: probe wide, fetch one).
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability: Option<BlobAvailability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<BlobManifest>,
    /// A per-holder observation the counters cannot carry — e.g. a tree
    /// holder with every chunk but no index (v1.17), which an index fetch
    /// will fail against despite a full-looking count. Rendered verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// It answered, and we could not read it: the encoding it declared and
    /// why. Answering unreadably is not not answering (RFC 09 §5.1 O4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
    /// An RFC 05 §3 error envelope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CallError>,
}

/// Who holds an artifact, and at which root (RFC 07 §2.5).
#[derive(Debug, Clone, Serialize)]
pub struct BlobProbeReport {
    /// The target as spelled back: `artifact/<id>`, `tree/<hex>`, …
    pub target: String,
    pub tier: String,
    /// The selectors actually asked. A probe's coverage claim is exactly this
    /// list and no wider (RFC 09 §5.1 O5).
    pub asked: Vec<String>,
    /// Why nothing was asked, when nothing was — a store algorithm the
    /// reference client does not speak, chiefly, now that every tier has a
    /// probe endpoint (RFC 07 §2.5, v1.17). Renders instead of a holder
    /// list; an unasked probe must never read as "no holders".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_probed: Option<String>,
    pub holders: Vec<BlobHolder>,
    pub answered: usize,
    /// Distinct content roots across holders.
    ///
    /// More than one is a **finding, not a tie-break**: the id is a name and
    /// RFC 07 §2.1's root is what disambiguates it, so a caller facing two
    /// roots must pin one rather than trust whoever answered first.
    pub roots: Vec<String>,
    /// Producers whose slice declares this tier — a capability claim, carried
    /// so a silent probe stays legible (RFC 05 §3.1: silence is not a verdict,
    /// and "nobody declares this" and "the declarers are down" are different
    /// silences).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub declared_by: Vec<String>,
    /// How many registry slices the `declared_by` sweep read —
    /// [`BlobList`]'s own solution, applied here (review finding R7). Without
    /// it an empty `declared_by` conflates "no slice declares this tier" with
    /// "no registry was loaded, so nobody was asked" (RFC 09 §5.1 O4) — the
    /// third silence, beside the two above. Additive.
    pub slices_considered: usize,
}

/// A fetch's progress, as the caller may render it.
///
/// Engine-owned rather than a re-export of the reference client's progress
/// type: that one is `#[non_exhaustive]`, and a GUI message enum cannot carry
/// a non-exhaustive payload without a wildcard arm in every match — which is
/// how a new variant becomes invisible instead of a compile error.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum BlobProgress {
    Started {
        total_len: u64,
        chunk_count: u32,
    },
    /// A partial download resumed from its persisted chunk bitfield.
    Resumed {
        received: u32,
        total: u32,
    },
    Chunk {
        index: u32,
        received: u32,
        total: u32,
        bytes_received: u64,
    },
    Verifying,
    Completed {
        path: String,
    },
    Cancelled {
        received: u32,
        total: u32,
    },
    Failed {
        error: String,
    },
}

/// What one fetch from one origin cost and proved (RFC 07 §2.1, §2.5, §2.6).
#[derive(Debug, Clone, Serialize)]
pub struct BlobFetchReport {
    pub origin: String,
    /// The one concrete key fetched from.
    pub key: String,
    pub dest: String,
    pub bytes: u64,
    pub chunks: u32,
    /// Chunks a previous attempt had already banked.
    pub chunks_resumed: u32,
    /// Replies verification rejected **before disk** (RFC 07 §2.1).
    pub rejected: u32,
    pub retries: u32,
    pub elapsed_ms: u64,
    pub root: String,
    /// `false` = trust-on-first-use, which the caller had to ask for out loud.
    /// RFC 07 §2.1 requires a reference to carry the root; an operator typing
    /// an id by hand has no reference, so the report says which it was.
    pub root_pinned: bool,
    /// The priority the GETs actually rode at (RFC 07 §2.6) — reported rather
    /// than assumed, and filled from the same constant the client is built
    /// with, so the sentence cannot drift from the behaviour.
    pub priority: String,
}

/// A validated tree-index summary from one origin (RFC 07 §2.3, v1.17):
/// inspection **without a content store**. The reply chain is untrusted at
/// every step — index chunks verify against their own addresses and the
/// reassembled index verifies against the root the caller asked for — so this
/// is pinned by construction, and browsing a tree costs its index, never its
/// content.
#[derive(Debug, Clone, Serialize)]
pub struct BlobTreeIndexReport {
    pub origin: String,
    /// The one concrete key asked.
    pub key: String,
    /// The identity fetched — also the pin.
    pub root: String,
    /// Directory entries of every kind.
    pub entries: usize,
    /// Files among them.
    pub files: usize,
    /// Total content bytes the snapshot references.
    pub total_size: u64,
    /// Distinct content chunks the snapshot references.
    pub chunks: usize,
    pub elapsed_ms: u64,
    /// See [`BlobFetchReport::priority`].
    pub priority: String,
}
