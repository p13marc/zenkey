//! What zengui subscribes to, and how it says so honestly.
//!
//! Selectors are **never** built with `format!`. A preset's selectors come
//! from `zenkey_model::grammar`'s typed builders — [`AMBIENT`] and
//! [`kind_selector`] — and are lifted to the wire with
//! `zenkey_fleet::with_namespace`, the same path zenctl's raw observers take.
//! The data pump watches on the session in **no** namespace (#614, FL6a), so
//! every selector here is a wire selector, the deployment's namespace in
//! front: that is what lets a key outside the deployment show up at all.
//!
//! # What `**` actually reaches
//!
//! Zenoh's `*` and `**` **never match a verbatim chunk** — one beginning with
//! `@` (core §1.3). It cuts both ways:
//!
//! - A `**` subscription **cannot** pull an explicit `@stream` (high rate,
//!   large: camera frames), an `@state` population, an `@op` reply, the `@zk`
//!   control keys or the admin space. The raw scope is safe by key algebra,
//!   not by policy — no defensive narrowing needed.
//! - `**` is therefore **not "everything"**, and calling it that would be a
//!   lie. A preset that wants an explicit kind names it ([`ScopePreset::Explicit`]),
//!   and every selector carries a line saying which verbatim chunks it cannot
//!   see ([`blind_spot`], from `zenkey_model::grammar::blind_spots`).

use anyhow::{Result, bail};
use zenkey_model::grammar::{AMBIENT, KindToken, kind_selector};

/// The raw scope: every plain-chunk key under any prefix, every namespace's
/// and none.
///
/// `**` spans any number of chunks *including zero*, so one selector covers
/// every namespace and the bus root alike.
pub const EVERYTHING: &str = "**";

/// What to watch.
///
/// `Serialize`/`Deserialize` because the preset is remembered across launches
/// (issue #73) — it is a user's choice about their window, not runtime state.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, clap::ValueEnum, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ScopePreset {
    /// Every plain-chunk key on the bus. The key-agnostic default.
    Everything,
    /// The deployment's ambient keys: `<namespace>/zk2/**`.
    Deployment,
    /// The deployment's ambient streams. v1 called the preset `telemetry`,
    /// and a remembered or typed `telemetry` still reads as this one.
    #[serde(alias = "telemetry")]
    #[value(alias = "telemetry")]
    Streams,
    State,
    Events,
    /// The explicit-only kinds, named: `@stream` and `@state` (core §1.3).
    /// No ambient selector ever delivers one.
    Explicit,
    /// User-supplied key expressions.
    Custom,
}

impl std::fmt::Display for ScopePreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.short())
    }
}

impl ScopePreset {
    /// Every preset, in menu order — the scope picker and the palette both
    /// iterate this, so the option list can never exclude a variant. That is
    /// the #187 defect: the picker hardcoded five presets, so launching with
    /// `--scope custom` produced a `pick_list` whose selected value was not
    /// among its own options.
    pub const ALL: [ScopePreset; 7] = [
        ScopePreset::Everything,
        ScopePreset::Deployment,
        ScopePreset::Streams,
        ScopePreset::State,
        ScopePreset::Events,
        ScopePreset::Explicit,
        ScopePreset::Custom,
    ];

    /// The short name, for places too narrow for [`Self::label`].
    pub fn short(self) -> &'static str {
        match self {
            ScopePreset::Everything => "everything",
            ScopePreset::Deployment => "deployment",
            ScopePreset::Streams => "streams",
            ScopePreset::State => "state",
            ScopePreset::Events => "events",
            ScopePreset::Explicit => "explicit",
            ScopePreset::Custom => "custom",
        }
    }

    /// A description precise enough that the user is not misled about coverage.
    pub fn label(self) -> &'static str {
        match self {
            ScopePreset::Everything => {
                "all plain-chunk traffic — @stream/@state/@op/@zk and the admin space excluded by key algebra (core §1.3)"
            }
            ScopePreset::Deployment => {
                "the deployment's ambient keys (zk2/**) — its explicit @stream/@state, @op and @zk excluded"
            }
            ScopePreset::Streams => "the deployment's ambient streams",
            ScopePreset::State => "the deployment's ambient state",
            ScopePreset::Events => "the deployment's events",
            ScopePreset::Explicit => {
                "the deployment's explicit @stream and @state, named — high rate or large"
            }
            ScopePreset::Custom => "user-supplied key expressions",
        }
    }

    /// The wire selectors this preset subscribes to.
    ///
    /// `namespace` is the active deployment namespace; the empty one is legal
    /// and means the bus root, for which `with_namespace` is the identity.
    pub fn selectors(self, namespace: &str, custom: &[String]) -> Vec<String> {
        let kind = |k: KindToken| {
            zenkey_fleet::with_namespace(namespace, kind_selector(None, None, None, k))
        };
        match self {
            ScopePreset::Everything => vec![EVERYTHING.to_string()],
            ScopePreset::Deployment => vec![zenkey_fleet::with_namespace(namespace, AMBIENT)],
            ScopePreset::Streams => vec![kind(KindToken::Stream)],
            ScopePreset::State => vec![kind(KindToken::State)],
            ScopePreset::Events => vec![kind(KindToken::Events)],
            // Named one by one: a verbatim kind is reached only by being
            // spelled, and spelling one lends nothing to the other.
            ScopePreset::Explicit => vec![
                kind(KindToken::ExplicitStream),
                kind(KindToken::ExplicitState),
            ],
            ScopePreset::Custom => custom.to_vec(),
        }
    }
}

/// The tree display path of one address's subtree (#61 click-through; zk2's
/// since #614, FL9): `zk2/<system>/<service>`, lifted with the namespace —
/// where every key the address owns sits, whatever its interface or kind.
/// Built from the typed [`Addr`](zenkey_model::grammar::Addr), whose chunks
/// the grammar validated, never from typed text.
pub fn address_display_path(namespace: &str, addr: &zenkey_model::grammar::Addr) -> String {
    zenkey_fleet::with_namespace(
        namespace,
        [
            zenkey_model::grammar::GRAMMAR,
            addr.system.as_str(),
            addr.service.as_str(),
        ]
        .join("/"),
    )
}

/// The watch selector for one tree subtree: symbolic skeleton chunks widen
/// (`{var}` → `*`, `{rest...}` → `**`), and the subtree is covered with a
/// trailing `/**`.
///
/// Verbatim chunks in the path are spelled literally, so watching an
/// `@stream` subtree works; verbatim chunks *below* the subtree stay
/// excluded by key algebra (core §1.3) — which the coverage label states
/// rather than hides.
pub fn subtree_selector(display_path: &str) -> String {
    let mut chunks: Vec<String> = Vec::new();
    for chunk in display_path.split('/') {
        if chunk.starts_with('{') && chunk.ends_with("...}") {
            chunks.push("**".to_string());
            break; // a rest consumes the remainder
        } else if chunk.starts_with('{') && chunk.ends_with('}') {
            chunks.push("*".to_string());
        } else {
            chunks.push(chunk.to_string());
        }
    }
    if chunks.last().map(|c| c.as_str()) != Some("**") {
        chunks.push("**".to_string());
    }
    chunks.join("/")
}

/// Whether a key or selector names the `@op` chunk — an operation's key,
/// whose only GET is a call (spec §5; #614, FL6b). The chunk is verbatim
/// (core §1.3): no wildcard reaches it and no other key carries it, so
/// naming it is the one way in, and this is the one test of it.
pub fn names_operation(key_or_selector: &str) -> bool {
    key_or_selector
        .split(['/', '?'])
        .any(|c| c == KindToken::Op.as_str())
}

/// What one selector **cannot** see — the per-selector honesty line the
/// key-expression editor renders (#187), in zk2's verbatim chunks (#614,
/// FL6a).
///
/// The rule users get wrong is core §1.3's: `*` and `**` never cross a
/// verbatim chunk. So a wildcarded selector is safe by key algebra — and
/// equally blind to every verbatim chunk it does not spell; it is not
/// "everything" and must never be labelled so. Which of `@stream`,
/// `@state`, `@op` and `@zk` it cannot reach is
/// `zenkey_model::grammar::blind_spots`'s answer, and any other `@` chunk
/// (the admin space, an advanced publisher's `@adv`) is out of reach the
/// same way.
pub fn blind_spot(sel: &str) -> String {
    let has_wildcard = sel.split('/').any(|c| c.contains('*'));
    if !has_wildcard {
        return "literal — matches exactly the keys it spells, and nothing else".to_string();
    }
    match zenkey_model::grammar::blind_spots(sel) {
        Ok(hidden) if hidden.is_empty() => {
            "names every zk2 verbatim chunk it could reach; its wildcards still \
             cannot cross any other @ chunk (core §1.3)"
                .to_string()
        }
        Ok(hidden) => format!(
            "wildcards never cross a verbatim chunk (core §1.3) — cannot see {}, \
             nor any other @ chunk (the admin space, @adv)",
            hidden.join(", ")
        ),
        Err(e) => format!("not a key expression: {e}"),
    }
}

/// Validate a user-supplied key expression.
pub fn validate_selector(sel: &str) -> Result<()> {
    if sel.trim().is_empty() {
        bail!("empty key expression");
    }
    // The reference refuses `$*` in selectors, not merely in published keys
    // (tooling guide §8.2).
    if sel.contains("$*") {
        bail!("`$*` must not be used in selectors (tooling guide §8.2): {sel:?}");
    }
    zenoh::key_expr::KeyExpr::new(sel.to_string())
        .map_err(|e| anyhow::anyhow!("not a valid key expression {sel:?}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zenoh::key_expr::KeyExpr;

    /// #614 (FL6b): `@op` is named only by spelling it — a wildcard never
    /// reaches a verbatim chunk — so a key or selector names an operation
    /// exactly when one of its chunks is `@op`.
    #[test]
    fn only_a_spelled_op_chunk_names_an_operation() {
        assert!(names_operation("zk2/robot1/nav/nav.v2/@op/reset"));
        assert!(names_operation("spray/zk2/*/*/*/@op/**"));
        assert!(names_operation("zk2/robot1/nav/nav.v2/@op"));
        assert!(!names_operation("zk2/robot1/nav/nav.v2/**"));
        assert!(!names_operation("**"));
        assert!(!names_operation("demo/op/reset"));
        assert!(!names_operation("demo/@operator"));
    }

    fn intersects(a: &str, b: &str) -> bool {
        KeyExpr::new(a)
            .unwrap()
            .intersects(&KeyExpr::new(b).unwrap())
    }

    /// The claim the whole scope design rests on: a wildcard never crosses a
    /// verbatim chunk (core §1.3). Pinned here, in zengui's own tests,
    /// because if it ever stopped holding the raw scope would start pulling
    /// camera frames into an echo pane.
    #[test]
    fn a_wildcard_never_crosses_a_verbatim_chunk() {
        let raw = &ScopePreset::Everything.selectors("", &[])[0];
        assert!(intersects(
            raw,
            "spray/zk2/robot1/nav/nav.v2/stream/position"
        ));
        for hidden in [
            "spray/zk2/robot1/camera/camera.v1/@stream/frames",
            "spray/zk2/fleet/planner/mission_plan.v1/@state/plans/r1",
            "spray/zk2/robot1/nav/nav.v2/@op/set_origin",
            "spray/zk2/robot1/nav/@zk/instance/0123456789abcdef",
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
        assert!(label.contains("@stream"), "{label}");
        assert!(label.contains("§1.3"), "{label}");
    }

    /// The Deployment preset is the namespace's ambient selector — and an
    /// ambient selector is the guard: it never delivers an explicit kind,
    /// an `@op` reply or a control key (core §1.3).
    #[test]
    fn the_deployment_scope_is_the_ambient_selector_in_the_namespace() {
        assert_eq!(
            ScopePreset::Deployment.selectors("spray", &[]),
            ["spray/zk2/**"]
        );
        assert_eq!(ScopePreset::Deployment.selectors("", &[]), ["zk2/**"]);
        let sel = &ScopePreset::Deployment.selectors("spray", &[])[0];
        assert!(intersects(
            sel,
            "spray/zk2/host-a/tc/tc.netif.v1/state/default/eth0"
        ));
        assert!(!intersects(
            sel,
            "elsewhere/zk2/host-a/tc/tc.netif.v1/state/default/eth0"
        ));
        assert!(!intersects(
            sel,
            "spray/zk2/robot1/camera/camera.v1/@stream/frames"
        ));
    }

    /// The explicit preset names both explicit kinds, each on its own — and
    /// neither reaches the other's keys, nor an ambient one.
    #[test]
    fn the_explicit_scope_names_each_explicit_kind() {
        let sels = ScopePreset::Explicit.selectors("spray", &[]);
        assert_eq!(sels.len(), 2);
        let frames = "spray/zk2/robot1/camera/camera.v1/@stream/frames";
        let plans = "spray/zk2/fleet/planner/mission_plan.v1/@state/plans/r1";
        assert!(intersects(&sels[0], frames) && !intersects(&sels[0], plans));
        assert!(intersects(&sels[1], plans) && !intersects(&sels[1], frames));
        for s in &sels {
            assert!(!intersects(
                s,
                "spray/zk2/robot1/nav/nav.v2/stream/position"
            ));
        }
    }

    /// `telemetry` is v1's word for the ambient streams (#614, FL6a): a
    /// typed `--scope telemetry` still parses, and as `streams`.
    #[test]
    fn telemetry_is_an_alias_of_streams() {
        use clap::ValueEnum as _;
        assert_eq!(
            ScopePreset::from_str("telemetry", false),
            Ok(ScopePreset::Streams)
        );
        assert_eq!(
            ScopePreset::from_str("streams", false),
            Ok(ScopePreset::Streams)
        );
    }

    /// The empty namespace is the identity, so a bus-root deployment gets
    /// clean `zk2/...` selectors rather than a leading slash.
    #[test]
    fn the_empty_namespace_yields_bus_root_selectors() {
        for preset in ScopePreset::ALL {
            for s in preset.selectors("", &["demo/**".into()]) {
                assert!(!s.starts_with('/'), "{preset}: {s}");
            }
        }
        assert!(intersects(
            &ScopePreset::Streams.selectors("", &[])[0],
            "zk2/robot1/nav/nav.v2/stream/position"
        ));
    }

    /// A multi-chunk namespace is legal (core §1.6).
    #[test]
    fn multi_chunk_namespaces_compose() {
        let sels = ScopePreset::State.selectors("acme/fleet-a", &[]);
        assert!(intersects(
            &sels[0],
            "acme/fleet-a/zk2/host-a/tc/tc.netif.v1/state/default/eth0"
        ));
    }

    /// Positions 1–5 have fixed arity (core §1.3), so the kind presets are
    /// pairwise disjoint.
    #[test]
    fn kind_presets_are_disjoint() {
        let kinds = [
            ScopePreset::Streams,
            ScopePreset::State,
            ScopePreset::Events,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                let (sa, sb) = (a.selectors("zs", &[]), b.selectors("zs", &[]));
                assert_eq!(intersects(&sa[0], &sb[0]), i == j, "{a} vs {b}");
            }
        }
    }

    /// `ALL` is the whole vocabulary: a variant added to the enum must reach
    /// the picker and the palette without anyone remembering to add it (#187).
    #[test]
    fn every_preset_is_in_the_option_list() {
        for preset in [
            ScopePreset::Everything,
            ScopePreset::Deployment,
            ScopePreset::Streams,
            ScopePreset::State,
            ScopePreset::Events,
            ScopePreset::Explicit,
            ScopePreset::Custom,
        ] {
            // The match is the exhaustiveness proof: a new variant fails to
            // compile here until it is listed, and the assert then pins it
            // into `ALL`.
            match preset {
                ScopePreset::Everything
                | ScopePreset::Deployment
                | ScopePreset::Streams
                | ScopePreset::State
                | ScopePreset::Events
                | ScopePreset::Explicit
                | ScopePreset::Custom => {}
            }
            assert!(ScopePreset::ALL.contains(&preset), "{preset} not offered");
        }
    }

    /// The editor's per-selector honesty line (#187), in zk2's verbatim
    /// chunks (#614, FL6a): a wildcard scope names what it cannot see, a
    /// literal admits it sees only itself, and a spelled verbatim kind does
    /// not launder the others in.
    #[test]
    fn a_blind_spot_line_names_what_the_selector_cannot_see() {
        let raw = blind_spot("**");
        assert!(raw.contains("§1.3"), "{raw}");
        for chunk in ["@stream", "@state", "@op", "@zk"] {
            assert!(raw.contains(chunk), "{chunk}: {raw}");
        }

        let literal = blind_spot("spray/zk2/host-a/tc/tc.netif.v1/state/default/eth0");
        assert!(literal.contains("exactly"), "{literal}");
        assert!(!literal.contains("§1.3"), "{literal}");

        let named = blind_spot("spray/zk2/*/*/*/@stream/**");
        assert!(!named.contains("@stream,"), "it names @stream: {named}");
        assert!(named.contains("@state"), "{named}");
        assert!(named.contains("@op"), "{named}");
    }

    #[test]
    fn selector_validation_rejects_the_forbidden_forms() {
        assert!(validate_selector("demo/**").is_ok());
        assert!(validate_selector("zk2/*/*/*/state/**").is_ok());
        assert!(validate_selector("").is_err());
        assert!(validate_selector("   ").is_err());
        // The reference refuses `$*` in selectors, not only in published keys
        // (tooling guide §8.2).
        let err = validate_selector("demo/$*/x").unwrap_err().to_string();
        assert!(err.contains("tooling guide §8.2"), "{err}");
    }

    /// The subtree watch selector widens symbolic chunks and covers the tail.
    #[test]
    fn subtree_selectors_widen_symbols() {
        assert_eq!(
            subtree_selector("v1/h-abc/telemetry"),
            "v1/h-abc/telemetry/**"
        );
        assert_eq!(
            subtree_selector("v1/{origin}/telemetry/sysinfo"),
            "v1/*/telemetry/sysinfo/**"
        );
        assert_eq!(
            subtree_selector("v1/h-abc/telemetry/sysinfo/disk/{mount}"),
            "v1/h-abc/telemetry/sysinfo/disk/*/**"
        );
        assert_eq!(
            subtree_selector("v1/h-abc/state/x/{path...}"),
            "v1/h-abc/state/x/**"
        );
        // A verbatim kind is spelled literally.
        assert_eq!(
            subtree_selector("zk2/robot1/camera/camera.v1/@stream"),
            "zk2/robot1/camera/camera.v1/@stream/**"
        );
    }
}
