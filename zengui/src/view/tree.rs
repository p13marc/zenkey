//! The key tree.
//!
//! The snapshot it walks is grammar-blind — `KeyTreeSnapshot` groups on a plain
//! `split('/')` — so an arbitrary bus renders as an ordinary chunk tree with
//! counts and rates. On top of that this module lays the *positional* overlay:
//! when a subtree turns out to be keyspace-v2, its chunks are labelled origin /
//! class / producer / subject.
//!
//! The overlay is resolved **relative to the base, never by absolute index**
//! (RFC 03 §1.1) — a multi-chunk base like `acme/fleet-a` is legal — and the
//! producer-or-subject question at position 5 is decided by the origin chunk
//! alone (RFC 03 §1.5). If the chunk where `v1` should be is not `v1`, the
//! whole subtree is left unlabelled rather than mislabelled.

use std::collections::BTreeSet;

use iced::widget::{Column, button, column, row, text};
use iced::{Element, Length};
use zenkey_fleet::skeleton::{MergedNode, NodeStatus};

use crate::keyfacts::{KeyFacts, Registration};
use crate::message::Message;
use crate::view::kit::{self, human_bytes, human_rate};
use crate::view::theme::{RegistrationTone, colors};
use crate::view::tokens::{font, space};

/// What a chunk means, when the subtree is conforming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Version,
    Origin,
    Class,
    Producer,
    BlobTier,
    Subject,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Version => "version",
            Role::Origin => "origin",
            Role::Class => "class",
            Role::Producer => "producer",
            Role::BlobTier => "tier",
            Role::Subject => "subject",
        }
    }
}

/// One rendered line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub depth: usize,
    /// Display chunk — a symbolic skeleton position renders as `{var}`.
    pub chunk: String,
    /// The full display-path prefix this row stands for.
    pub path: String,
    pub has_children: bool,
    pub expanded: bool,
    /// The declared/observed state (issue #85's typed acceptance criterion).
    pub status: NodeStatus,
    /// Traffic landed on exactly this key.
    pub is_leaf: bool,
    pub count: u64,
    pub bytes: u64,
    pub rate_hz: f64,
    pub subtree_count: u64,
    pub subtree_bytes: u64,
    pub subtree_keys: usize,
    pub subtree_rate_hz: f64,
    /// `None` for any chunk outside a recognised v1 subtree.
    pub role: Option<Role>,
    /// The registry-declared payload type, when the skeleton knows one.
    pub decl_type: Option<String>,
}

/// The flattened tree, plus what was left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Flattened {
    pub rows: Vec<TreeRow>,
    /// Rows beyond the cap. Reported, never silently dropped.
    pub truncated: usize,
}

/// Flatten a snapshot into display rows.
///
/// `base` is the active deployment base, used only to know how many leading
/// chunks to skip before looking for `v1`. `expanded` holds the paths the user
/// has opened; everything else collapses.
pub fn flatten(
    merged: &MergedNode,
    base: &str,
    expanded: &BTreeSet<String>,
    max_rows: usize,
) -> Flattened {
    let base_chunks = if base.is_empty() {
        0
    } else {
        base.split('/').count()
    };
    let mut rows = Vec::new();
    let mut truncated = 0;
    walk(
        merged,
        &mut Ctx {
            expanded,
            max_rows,
            rows: &mut rows,
            truncated: &mut truncated,
        },
        String::new(),
        0,
        if base_chunks == 0 {
            Expect::Version
        } else {
            Expect::Base(base_chunks)
        },
    );
    Flattened { rows, truncated }
}

struct Ctx<'a> {
    expanded: &'a BTreeSet<String>,
    max_rows: usize,
    rows: &'a mut Vec<TreeRow>,
    truncated: &'a mut usize,
}

/// What the *next* chunk down this branch is expected to be.
///
/// A state machine rather than arithmetic on depth, because depth alone cannot
/// distinguish "we have not reached the origin position yet" from "this branch
/// is not conforming at all" — and conflating them labels a foreign chunk
/// `origin`, which is worse than saying nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// `n` base chunks still to consume before the convention starts.
    Base(usize),
    Version,
    Origin,
    /// `host` is what decides whether position 5 is a producer (RFC 03 §1.5).
    Class {
        host: bool,
    },
    Producer {
        host: bool,
    },
    BlobTier,
    Subject,
    /// This branch is not keyspace-v2. Nothing below it is labelled.
    Foreign,
}

fn walk(node: &MergedNode, ctx: &mut Ctx<'_>, path: String, depth: usize, expect: Expect) {
    for (chunk, child) in &node.children {
        let child_path = if path.is_empty() {
            chunk.clone()
        } else {
            format!("{path}/{chunk}")
        };
        let (role, next_expect) = classify(chunk, expect);

        if ctx.rows.len() >= ctx.max_rows {
            *ctx.truncated += 1;
        } else {
            let expanded = ctx.expanded.contains(&child_path);
            let stats = child.stats;
            ctx.rows.push(TreeRow {
                depth,
                chunk: chunk.clone(),
                path: child_path.clone(),
                has_children: !child.children.is_empty(),
                expanded,
                status: child.status,
                is_leaf: stats.map(|s| s.count > 0).unwrap_or(false),
                count: stats.map(|s| s.count).unwrap_or(0),
                bytes: stats.map(|s| s.bytes).unwrap_or(0),
                rate_hz: stats.map(|s| s.rate_hz).unwrap_or(0.0),
                subtree_count: stats.map(|s| s.subtree_count).unwrap_or(0),
                subtree_bytes: stats.map(|s| s.subtree_bytes).unwrap_or(0),
                subtree_keys: stats.map(|s| s.subtree_keys).unwrap_or(0),
                subtree_rate_hz: stats.map(|s| s.subtree_rate_hz).unwrap_or(0.0),
                role,
                decl_type: child.decl.as_ref().map(|d| d.type_name.clone()),
            });
            if expanded {
                walk(child, ctx, child_path, depth + 1, next_expect);
            }
        }
    }
}

/// The role of one chunk, and what to expect below it.
fn classify(chunk: &str, expect: Expect) -> (Option<Role>, Expect) {
    match expect {
        // The convention says nothing about how a deployment spells its own
        // base, so base chunks carry no role.
        Expect::Base(n) if n > 1 => (None, Expect::Base(n - 1)),
        Expect::Base(_) => (None, Expect::Version),
        Expect::Version => {
            if chunk == zenkey::grammar::VERSION_CHUNK {
                (Some(Role::Version), Expect::Origin)
            } else {
                // Not a v1 subtree. Leave the whole branch unlabelled rather
                // than guess — mislabelling is worse than not labelling.
                (None, Expect::Foreign)
            }
        }
        Expect::Origin => {
            // RFC 03 §1.3 licenses tooling to use this shape, and §1.5 makes it
            // the sole discriminator for the producer position. A *symbolic*
            // origin (`{origin}`) exists only for host slices — the skeleton
            // never symbolizes a service origin (those are literal `@x`) — so
            // it classifies as host-like.
            let host = zenkey::grammar::is_valid_host_origin(chunk) || chunk.starts_with('{');
            (Some(Role::Origin), Expect::Class { host })
        }
        Expect::Class { host } => {
            // Under `@blob` position 5 is a tier token, not a producer.
            let next = if chunk == zenkey::grammar::PLANE_BLOB {
                Expect::BlobTier
            } else {
                Expect::Producer { host }
            };
            (Some(Role::Class), next)
        }
        // A service origin omits the producer chunk entirely; position 5 is
        // already subject (RFC 03 §1.5).
        Expect::Producer { host: true } => (Some(Role::Producer), Expect::Subject),
        Expect::Producer { host: false } => (Some(Role::Subject), Expect::Subject),
        Expect::BlobTier => (Some(Role::BlobTier), Expect::Subject),
        Expect::Subject => (Some(Role::Subject), Expect::Subject),
        Expect::Foreign => (None, Expect::Foreign),
    }
}

/// Which tone a leaf's registration should read as.
pub fn tone(reg: &Registration) -> RegistrationTone {
    match reg {
        Registration::Registered(_) => RegistrationTone::Registered,
        Registration::Unregistered => RegistrationTone::Unregistered,
        Registration::NoSliceForProducer => RegistrationTone::NoSlice,
        Registration::Unknown => RegistrationTone::Unknown,
        Registration::NotApplicable => RegistrationTone::NotApplicable,
    }
}

/// The badge text for a registration state.
///
/// The tri-state is only useful if the words differ: "—" (not asked) must not
/// read like "unregistered" (asked, and the answer was no).
pub fn registration_label(reg: &Registration) -> &'static str {
    match reg {
        Registration::Registered(_) => "registered",
        Registration::Unregistered => "unregistered",
        Registration::NoSliceForProducer => "no slice",
        Registration::Unknown => "—",
        Registration::NotApplicable => "",
    }
}

/// Registration lookup by wire key. A plain map rather than a closure so the
/// pane borrows app state directly instead of a temporary.
pub type FactsIndex = std::collections::HashMap<String, KeyFacts>;

/// Render the tree pane.
pub fn tree_view<'a>(
    flat: &'a Flattened,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watched_paths: &'a BTreeSet<String>,
) -> Element<'a, Message> {
    if flat.rows.is_empty() {
        return kit::empty_state(
            "Nothing observed yet",
            "An empty tree is not a verdict about the bus (RFC 05 §3.1) — \
             it may simply mean nothing has published on the current scope.",
        );
    }

    let mut col = Column::new().spacing(1);
    for r in &flat.rows {
        col = col.push(row_view(r, facts, selected, watched_paths));
    }
    if flat.truncated > 0 {
        col = col.push(kit::muted(format!(
            "… {} more rows not shown (row cap)",
            flat.truncated
        )));
    }
    iced::widget::scrollable(col).height(Length::Fill).into()
}

fn row_view<'a>(
    r: &'a TreeRow,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watched_paths: &'a BTreeSet<String>,
) -> Element<'a, Message> {
    let indent = iced::widget::Space::new().width(Length::Fixed(r.depth as f32 * 14.0));

    let marker = if r.has_children {
        if r.expanded { "▾" } else { "▸" }
    } else {
        " "
    };

    let is_selected = selected == Some(r.path.as_str());
    let name = text(format!("{marker} {}", r.chunk))
        .size(font::CAPTION)
        .font(iced::Font::MONOSPACE)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if is_selected {
                colors(theme).primary()
            } else {
                colors(theme).text()
            }),
        });

    let mut line = row![indent, name]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center);

    if let Some(role) = r.role {
        line = line.push(kit::muted(role.label()));
    }

    // The declared/observed state (issue #85): "declared" must never read
    // like "quiet", and "unwatched" like neither.
    match r.status {
        NodeStatus::DeclaredOnly(_) => {
            line = line.push(kit::tone_badge(
                crate::view::theme::RegistrationTone::Unknown,
                "declared",
            ));
        }
        NodeStatus::WatchedQuiet(_) => {
            line = line.push(kit::muted("quiet"));
        }
        NodeStatus::Unwatched(_) => {
            line = line.push(kit::tone_badge(
                crate::view::theme::RegistrationTone::Unregistered,
                "unwatched",
            ));
        }
        NodeStatus::Observed(_) => {}
    }
    if let Some(ty) = &r.decl_type {
        line = line.push(kit::muted(ty.clone()));
    }

    line = line.push(iced::widget::space::horizontal());

    // A collapsed node reports its whole subtree; an expanded leaf reports
    // itself. Showing only leaf traffic on a collapsed node would understate
    // it by exactly the part the user cannot see.
    if r.is_leaf {
        line = line.push(kit::muted(format!(
            "{} · {} · {}",
            r.count,
            human_bytes(r.bytes),
            human_rate(r.rate_hz)
        )));
        if let Some(f) = facts.get(&r.path) {
            let label = registration_label(&f.registration);
            if !label.is_empty() {
                line = line.push(kit::tone_badge(tone(&f.registration), label));
            }
            if let Some(ty) = f.type_name() {
                line = line.push(kit::muted(ty.to_string()));
            }
        }
    } else if !r.expanded && r.subtree_count > 0 {
        line = line.push(kit::muted(format!(
            "{} · {} · {} · {}",
            kit::plural(r.subtree_keys, "key"),
            r.subtree_count,
            human_bytes(r.subtree_bytes),
            human_rate(r.subtree_rate_hz)
        )));
    }

    // Observation is opt-in, per subtree (issue #85). The toggle reflects
    // *this app's* watches, not global coverage.
    let watch_label = if watched_paths.contains(&r.path) {
        "◉"
    } else {
        "○"
    };
    let watch = button(text(watch_label).size(font::CAPTION))
        .padding(2)
        .style(button::text)
        .on_press(Message::WatchToggled(r.path.clone()));

    let body = button(line)
        .width(Length::Fill)
        .padding(2)
        .style(|_theme: &iced::Theme, _status| button::Style {
            background: None,
            text_color: iced::Color::WHITE,
            ..Default::default()
        })
        .on_press(if r.has_children {
            Message::ToggleNode(r.path.clone())
        } else {
            Message::SelectKey(Some(r.path.clone()))
        });
    row![watch, body]
        .spacing(space::XS)
        .align_y(iced::Alignment::Center)
        .into()
}

/// A standalone pane, for tests and for the `view/mod` composition.
pub fn pane<'a>(
    flat: &'a Flattened,
    facts: &'a FactsIndex,
    selected: Option<&'a str>,
    watched_paths: &'a BTreeSet<String>,
) -> Element<'a, Message> {
    column![
        kit::section_header("Keys", None),
        tree_view(flat, facts, selected, watched_paths)
    ]
    .spacing(space::SM)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use zenkey_fleet::stats::StatsTable;

    fn snapshot(keys: &[&str]) -> MergedNode {
        let mut stats = StatsTable::new();
        let now = Instant::now();
        for k in keys {
            stats.record(k, 8, None, now);
        }
        let observed = zenkey_fleet::KeyTreeSnapshot::build(&stats);
        // Tests watch everything: every observed node reads Observed.
        let skel = zenkey_fleet::Skeleton::build(
            "",
            &zenkey_fleet::SliceSet::default(),
            &std::collections::BTreeMap::new(),
            None,
        );
        zenkey_fleet::skeleton::merge(&skel, &observed, &["**".to_string()])
    }

    fn expand(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|s| s.to_string()).collect()
    }

    fn role_of(flat: &Flattened, path: &str) -> Option<Role> {
        flat.rows
            .iter()
            .find(|r| r.path == path)
            .and_then(|r| r.role)
    }

    #[test]
    fn collapsed_by_default_and_expands_on_demand() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage"]);
        let flat = flatten(&snap, "", &BTreeSet::new(), 100);
        assert_eq!(flat.rows.len(), 1, "only the root chunk shows");
        assert_eq!(flat.rows[0].chunk, "v1");
        assert!(flat.rows[0].has_children);

        let flat = flatten(&snap, "", &expand(&["v1"]), 100);
        assert_eq!(flat.rows.len(), 2);
        assert_eq!(flat.rows[1].chunk, "h-3fa9c2d41b7e");
    }

    /// RFC 03 §1.1: the overlay is resolved relative to the base. The same
    /// subject under a one-chunk and a two-chunk base must label identically.
    #[test]
    fn roles_are_resolved_relative_to_the_base() {
        let key = "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu";
        for (base, prefix) in [
            ("", ""),
            ("zensight", "zensight/"),
            ("acme/fleet-a", "acme/fleet-a/"),
        ] {
            let full = format!("{prefix}{key}");
            let snap = snapshot(&[&full]);
            // Expand the whole path.
            let mut paths = Vec::new();
            let mut acc = String::new();
            for chunk in full.split('/') {
                acc = if acc.is_empty() {
                    chunk.to_string()
                } else {
                    format!("{acc}/{chunk}")
                };
                paths.push(acc.clone());
            }
            let set: BTreeSet<String> = paths.into_iter().collect();
            let flat = flatten(&snap, base, &set, 100);

            assert_eq!(role_of(&flat, &format!("{prefix}v1")), Some(Role::Version));
            assert_eq!(
                role_of(&flat, &format!("{prefix}v1/h-3fa9c2d41b7e")),
                Some(Role::Origin)
            );
            assert_eq!(
                role_of(&flat, &format!("{prefix}v1/h-3fa9c2d41b7e/telemetry")),
                Some(Role::Class)
            );
            assert_eq!(
                role_of(
                    &flat,
                    &format!("{prefix}v1/h-3fa9c2d41b7e/telemetry/sysinfo")
                ),
                Some(Role::Producer),
                "base {base:?}"
            );
        }
    }

    /// RFC 03 §1.5: under a service origin position 5 is already subject.
    #[test]
    fn a_service_origin_has_no_producer_position() {
        let snap = snapshot(&["v1/@catalog/state/entity/x"]);
        let set = expand(&[
            "v1",
            "v1/@catalog",
            "v1/@catalog/state",
            "v1/@catalog/state/entity",
        ]);
        let flat = flatten(&snap, "", &set, 100);
        assert_eq!(role_of(&flat, "v1/@catalog"), Some(Role::Origin));
        assert_eq!(role_of(&flat, "v1/@catalog/state"), Some(Role::Class));
        assert_eq!(
            role_of(&flat, "v1/@catalog/state/entity"),
            Some(Role::Subject),
            "`entity` is subject, not a producer"
        );
    }

    /// Under `@blob` position 5 is a tier token, not a producer (RFC 03 §1.5).
    #[test]
    fn a_blob_tier_is_not_labelled_a_producer() {
        let snap = snapshot(&["v1/h-3fa9c2d41b7e/@blob/store/sha256/abcdef01"]);
        let set = expand(&[
            "v1",
            "v1/h-3fa9c2d41b7e",
            "v1/h-3fa9c2d41b7e/@blob",
            "v1/h-3fa9c2d41b7e/@blob/store",
        ]);
        let flat = flatten(&snap, "", &set, 100);
        assert_eq!(role_of(&flat, "v1/h-3fa9c2d41b7e/@blob"), Some(Role::Class));
        assert_eq!(
            role_of(&flat, "v1/h-3fa9c2d41b7e/@blob/store"),
            Some(Role::BlobTier)
        );
        assert_eq!(
            role_of(&flat, "v1/h-3fa9c2d41b7e/@blob/store/sha256"),
            Some(Role::Subject)
        );
    }

    /// A foreign key is a first-class citizen: it gets rows and stats, and is
    /// left unlabelled rather than mislabelled.
    #[test]
    fn foreign_keys_render_unlabelled_but_present() {
        let snap = snapshot(&["demo/example/foo"]);
        let set = expand(&["demo", "demo/example"]);
        let flat = flatten(&snap, "", &set, 100);
        assert_eq!(flat.rows.len(), 3);
        for r in &flat.rows {
            assert_eq!(r.role, None, "{} must not be labelled", r.path);
        }
        // …and it still carries its traffic.
        let leaf = flat
            .rows
            .iter()
            .find(|r| r.path == "demo/example/foo")
            .unwrap();
        assert!(leaf.is_leaf);
        assert_eq!(leaf.count, 1);
    }

    /// A subtree whose version chunk is not `v1` must not be labelled by
    /// position — that would attach convention meanings to foreign chunks.
    #[test]
    fn a_non_v1_subtree_is_left_unlabelled() {
        let snap = snapshot(&["v2/h-3fa9c2d41b7e/telemetry/sysinfo/cpu"]);
        let set = expand(&["v2", "v2/h-3fa9c2d41b7e", "v2/h-3fa9c2d41b7e/telemetry"]);
        let flat = flatten(&snap, "", &set, 100);
        for r in &flat.rows {
            assert_eq!(r.role, None, "{} must not be labelled", r.path);
        }
    }

    /// A collapsed node must carry its subtree's totals, or the number the
    /// user reads understates the traffic by exactly the invisible part.
    #[test]
    fn collapsed_rows_report_subtree_totals() {
        let snap = snapshot(&[
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu",
            "v1/h-3fa9c2d41b7e/telemetry/sysinfo/mem",
            "v1/h-3fa9c2d41b7e/state/sysinfo/health",
        ]);
        let flat = flatten(&snap, "", &BTreeSet::new(), 100);
        let root = &flat.rows[0];
        assert_eq!(root.chunk, "v1");
        assert!(!root.expanded);
        assert_eq!(root.subtree_count, 3);
        assert_eq!(root.subtree_keys, 3);
        assert_eq!(root.subtree_bytes, 24);
    }

    /// Truncation is reported, never silent — a capped list that looks
    /// complete is a lie about coverage.
    #[test]
    fn the_row_cap_reports_what_it_dropped() {
        let keys: Vec<String> = (0..50).map(|i| format!("demo/k{i}")).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let snap = snapshot(&refs);
        let flat = flatten(&snap, "", &expand(&["demo"]), 10);
        assert_eq!(flat.rows.len(), 10);
        assert_eq!(flat.truncated, 41, "1 root + 50 leaves, 10 shown");
    }

    /// The three "we know nothing" states must not read alike.
    #[test]
    fn registration_labels_distinguish_not_asked_from_not_registered() {
        assert_eq!(registration_label(&Registration::Unknown), "—");
        assert_eq!(
            registration_label(&Registration::Unregistered),
            "unregistered"
        );
        assert_eq!(
            registration_label(&Registration::NoSliceForProducer),
            "no slice"
        );
        assert_ne!(
            tone(&Registration::Unknown),
            tone(&Registration::Unregistered)
        );
    }
}
