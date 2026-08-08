//! What zengui subscribes to, and how it says so honestly.
//!
//! Selectors are **never** built with `format!`. Base-relative selectors come
//! from `zenkey::selector`'s typed builders and are lifted to the wire with
//! `grammar::with_base` — the same path `zenctl` takes. That is a convention of
//! this repo, and here it is also load-bearing: the D4 trap below is exactly
//! the kind of thing a hand-written string gets wrong.
//!
//! # What `**` actually reaches
//!
//! Verified against zenoh 1.9: `*` and `**` **never match a chunk beginning
//! with `@`**. This is RFC 03 §4 **D2**, pinned executably in
//! `zenkey/tests/guard.rs`, and it cuts both ways:
//!
//! - A `**` subscription **cannot** pull `@media` video frames, `@blob` bulk,
//!   `@rpc` replies, `@adv` sidecars, or the admin space. The raw scope is
//!   media-safe by key algebra, not by policy — no defensive narrowing needed.
//! - `**` is therefore **not "everything"**, and calling it that would be a
//!   lie. It silently omits every verbatim plane, most importantly `@catalog`.
//!   An explorer that shows `**` and reports "no entities" when the catalog is
//!   merely unmatched has produced the false verdict RFC 05 §3.1 and D4's
//!   corollary forbid. Hence [`ScopePreset::label`], and hence `@catalog`'s
//!   liveliness token being named explicitly in every roster.

use anyhow::{Result, bail};
use zenkey::grammar::with_base;
use zenkey::origin::ServiceOrigin;
use zenkey::selector::{self, Scope};

/// The base placeholder used before the deployment base is known.
///
/// `**` spans any number of chunks *including zero*, so a single sweep covers
/// every base and the base-less bus root alike (RFC 09 §5's discovery recipe).
const ANY_BASE: &str = "**";

/// What to watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum)]
pub enum ScopePreset {
    /// Every plain-chunk key on the bus. The key-agnostic default.
    Everything,
    /// The three data classes of one deployment, plus `@catalog`.
    Deployment,
    Telemetry,
    State,
    Events,
    /// User-supplied key expressions.
    Custom,
}

impl std::fmt::Display for ScopePreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ScopePreset::Everything => "everything",
            ScopePreset::Deployment => "deployment",
            ScopePreset::Telemetry => "telemetry",
            ScopePreset::State => "state",
            ScopePreset::Events => "events",
            ScopePreset::Custom => "custom",
        })
    }
}

impl ScopePreset {
    /// A description precise enough that the user is not misled about coverage.
    pub fn label(self) -> &'static str {
        match self {
            ScopePreset::Everything => {
                "all plain-chunk traffic — @rpc/@media/@blob/@catalog excluded by key algebra (RFC 03 §4 D2)"
            }
            ScopePreset::Deployment => "the deployment's data classes, plus @catalog",
            ScopePreset::Telemetry => "one deployment's telemetry",
            ScopePreset::State => "one deployment's state",
            ScopePreset::Events => "one deployment's events",
            ScopePreset::Custom => "user-supplied key expressions",
        }
    }

    /// The wire selectors this preset subscribes to.
    ///
    /// `base` is the active deployment base; the empty base is legal and means
    /// the bus root, for which `with_base` is the identity.
    pub fn selectors(self, base: &str, custom: &[String]) -> Vec<String> {
        match self {
            ScopePreset::Everything => vec![ANY_BASE.to_string()],
            ScopePreset::Deployment => vec![
                with_base(base, selector::all_telemetry(Scope::fleet())),
                with_base(base, selector::all_state(Scope::fleet())),
                with_base(base, selector::all_events(Scope::fleet())),
                // Named explicitly: `*` in the origin position never matches a
                // verbatim service origin (RFC 03 §4 D4). Without this line the
                // catalog is invisible and its absence reads as "no entities".
                with_base(base, catalog_subtree()),
            ],
            ScopePreset::Telemetry => {
                vec![with_base(base, selector::all_telemetry(Scope::fleet()))]
            }
            ScopePreset::State => vec![with_base(base, selector::all_state(Scope::fleet()))],
            ScopePreset::Events => vec![with_base(base, selector::all_events(Scope::fleet()))],
            ScopePreset::Custom => custom.to_vec(),
        }
    }
}

/// The `@catalog` state subtree, base-relative.
///
/// `zenkey::selector` has no builder for a service origin's whole state subtree
/// (only `service_alive` and `service_rpc`), so this is assembled from the
/// crate's own reserved tokens rather than a literal.
fn catalog_subtree() -> String {
    format!(
        "{}/{}/{}/**",
        zenkey::grammar::VERSION_CHUNK,
        ServiceOrigin::catalog().as_str(),
        zenkey::grammar::CLASS_STATE,
    )
}

/// The liveliness selectors for a roster.
///
/// Always two, never one. RFC 04 §5 makes the per-producer `alive` token the
/// only presence signal a dashboard may read, and RFC 03 §4 D4 means the fleet
/// sweep cannot reach `@catalog` — so "catalog dead" and "no entities" are
/// indistinguishable unless the service token is asked for by name (RFC 09 §1).
///
/// `base` may be [`ANY_BASE`] while the deployment base is still unknown.
pub fn liveliness_selectors(base: &str) -> Vec<String> {
    vec![
        with_base(base, selector::all_liveliness(Scope::fleet())),
        with_base(base, selector::service_alive(&ServiceOrigin::catalog())),
    ]
}

/// The base-agnostic roster sweep, for use before a base has been chosen.
pub fn liveliness_any_base() -> Vec<String> {
    liveliness_selectors(ANY_BASE)
}

/// Validate a user-supplied key expression.
pub fn validate_selector(sel: &str) -> Result<()> {
    if sel.trim().is_empty() {
        bail!("empty key expression");
    }
    // RFC 03 §2 forbids `$*` in selectors, not merely in published keys.
    if sel.contains("$*") {
        bail!("`$*` must not be used in selectors (RFC 03 §2): {sel:?}");
    }
    zenoh::key_expr::KeyExpr::new(sel.to_string())
        .map_err(|e| anyhow::anyhow!("not a valid key expression {sel:?}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenoh::key_expr::KeyExpr;

    fn intersects(a: &str, b: &str) -> bool {
        KeyExpr::new(a)
            .unwrap()
            .intersects(&KeyExpr::new(b).unwrap())
    }

    /// The claim the whole scope design rests on: a wildcard never crosses a
    /// verbatim chunk (RFC 03 §4 D2). Pinned here, in zengui's own tests,
    /// because if it ever stopped holding the raw scope would start pulling
    /// video frames into an echo pane.
    #[test]
    fn a_wildcard_never_crosses_a_verbatim_chunk() {
        let raw = &ScopePreset::Everything.selectors("", &[])[0];
        assert!(intersects(
            raw,
            "zensight/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage"
        ));
        for hidden in [
            "zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/hi",
            "zensight/v1/h-3fa9c2d41b7e/@blob/store/sha256/abcdef01",
            "zensight/v1/h-3fa9c2d41b7e/@rpc/sysinfo/introspect",
            "zensight/v1/@catalog/state/entity/x",
            "@/router/abc",
        ] {
            assert!(!intersects(raw, hidden), "{raw} must not reach {hidden}");
        }
    }

    /// …and because it does not, the raw scope must not claim to be everything.
    #[test]
    fn the_raw_scope_does_not_claim_to_be_everything() {
        let label = ScopePreset::Everything.label();
        assert!(label.contains("plain-chunk"), "{label}");
        assert!(label.contains("D2"), "{label}");
    }

    /// The D4 regression guard: dropping this selector is a silent coverage
    /// loss that reads to the user as a dead catalog.
    #[test]
    fn the_deployment_scope_names_the_catalog_explicitly() {
        let sels = ScopePreset::Deployment.selectors("zensight", &[]);
        let catalog_key = "zensight/v1/@catalog/state/entity/x";
        assert!(
            sels.iter().any(|s| intersects(s, catalog_key)),
            "no selector in {sels:?} reaches {catalog_key}"
        );
        // And it is a *distinct* selector — the fleet ones genuinely cannot.
        for s in sels.iter().filter(|s| !s.contains("@catalog")) {
            assert!(!intersects(s, catalog_key), "{s} should not reach it");
        }
    }

    /// Presence needs both tokens, for the same D4 reason.
    #[test]
    fn the_roster_watches_both_the_fleet_and_the_catalog() {
        let sels = liveliness_selectors("zensight");
        assert_eq!(sels.len(), 2);
        assert!(
            sels.iter()
                .any(|s| intersects(s, "zensight/v1/h-3fa9c2d41b7e/state/sysinfo/alive"))
        );
        assert!(
            sels.iter()
                .any(|s| intersects(s, "zensight/v1/@catalog/state/alive"))
        );
    }

    /// `**` spans zero chunks, so one sweep covers every base *and* the
    /// base-less bus root (RFC 09 §5).
    #[test]
    fn the_any_base_roster_covers_named_and_empty_bases() {
        let sels = liveliness_any_base();
        for key in [
            "v1/h-3fa9c2d41b7e/state/sysinfo/alive",
            "zensight/v1/h-3fa9c2d41b7e/state/sysinfo/alive",
            "acme/fleet-a/v1/h-3fa9c2d41b7e/state/sysinfo/alive",
        ] {
            assert!(sels.iter().any(|s| intersects(s, key)), "{key}");
        }
    }

    /// The empty base is the identity, so a base-less deployment gets clean
    /// `v1/...` selectors rather than a leading slash.
    #[test]
    fn the_empty_base_yields_bus_root_selectors() {
        let sels = ScopePreset::Deployment.selectors("", &[]);
        for s in &sels {
            assert!(s.starts_with("v1/"), "{s}");
        }
        assert!(
            sels.iter()
                .any(|s| intersects(s, "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage"))
        );
    }

    /// A multi-chunk base is legal (RFC 03 §1.1).
    #[test]
    fn multi_chunk_bases_compose() {
        let sels = ScopePreset::Telemetry.selectors("acme/fleet-a", &[]);
        assert!(intersects(
            &sels[0],
            "acme/fleet-a/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage"
        ));
    }

    /// RFC 03 §4 D3: the class presets are pairwise disjoint.
    #[test]
    fn class_presets_are_disjoint() {
        let classes = [
            ScopePreset::Telemetry,
            ScopePreset::State,
            ScopePreset::Events,
        ];
        for (i, a) in classes.iter().enumerate() {
            for (j, b) in classes.iter().enumerate() {
                let (sa, sb) = (a.selectors("zs", &[]), b.selectors("zs", &[]));
                assert_eq!(intersects(&sa[0], &sb[0]), i == j, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn selector_validation_rejects_the_forbidden_forms() {
        assert!(validate_selector("demo/**").is_ok());
        assert!(validate_selector("v1/*/state/**").is_ok());
        assert!(validate_selector("").is_err());
        assert!(validate_selector("   ").is_err());
        // RFC 03 §2 forbids `$*` in selectors, not only in published keys.
        let err = validate_selector("demo/$*/x").unwrap_err().to_string();
        assert!(err.contains("RFC 03 §2"), "{err}");
    }
}
