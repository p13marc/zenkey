//! The status strip: what zengui is actually watching, stated rather than
//! implied.
//!
//! This pane exists because of RFC 05 §3.1 and RFC 12 §9. An explorer showing
//! an empty tree looks identical whether the bus is quiet, the scope excludes
//! the traffic, the session reaches nothing, or the registry has not loaded.
//! Every one of those is stated here so the user never has to infer it.

use iced::Element;
use iced::widget::{row, text};

use crate::message::{LinkState, Message};
use crate::view::kit::{self, human_bytes, human_rate};
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// Where registry slices came from, if anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SliceSource {
    /// Not loaded. Every registration badge reads "—" until this changes.
    None,
    Bus {
        count: usize,
    },
    Dirs {
        count: usize,
    },
    /// The §6.1 union: served wins, dirs fill, disagreement is data.
    Union {
        from_bus: usize,
        dirs_only: usize,
        disagreements: usize,
    },
    Failed(String),
}

impl SliceSource {
    pub fn label(&self) -> String {
        match self {
            SliceSource::None => "registry: not loaded".to_string(),
            SliceSource::Bus { count } => format!("registry: bus · {count} slices"),
            SliceSource::Dirs { count } => format!("registry: dirs · {count} slices"),
            SliceSource::Union {
                from_bus,
                dirs_only,
                disagreements,
            } => {
                let mut label =
                    format!("registry: union · {from_bus} served + {dirs_only} dirs-only");
                if *disagreements > 0 {
                    label.push_str(&format!(" · {disagreements} DISAGREE"));
                }
                label
            }
            SliceSource::Failed(e) => format!("registry: failed — {e}"),
        }
    }
}

/// Everything the strip reports.
pub struct Status<'a> {
    pub link: &'a LinkState,
    pub base_label: &'a str,
    /// Active watch selectors (coverage, O5).
    pub watched: &'a [String],
    /// What fed the skeleton, when one was built.
    pub skeleton: Option<zenkey_fleet::skeleton::SkeletonCoverage>,
    /// Keys retired because their watch was released (O6's third counter).
    pub keys_unwatched: u64,
    /// The last on-demand value fetch, for the selection.
    pub fetched: Option<&'a (
        String,
        Result<std::sync::Arc<zenkey_fleet::FetchOutcome>, String>,
    )>,
    /// The scope's short name. The long explanation lives in the toolbar; the
    /// strip must stay narrow enough to fit, or its right-hand end — which is
    /// where the registry state and the reachability warning live — clips off
    /// screen exactly when something is wrong.
    pub scope_label: &'a str,
    pub keys: usize,
    /// Keys retired to stay within the table's bound.
    pub keys_evicted: u64,
    pub totals: (u64, u64, f64),
    pub slices: &'a SliceSource,
    /// Watches whose seed phase has not resolved yet (issue #92).
    pub seeding: usize,
    /// Seed phases that have completed since connect.
    pub seeded_watches: usize,
    /// Cumulative seed coverage since connect: (cache replies, storage
    /// replies, superseded).
    pub seed_totals: (usize, usize, u64),
    pub unreachable: bool,
}

/// The key count, and — if the table has evicted — what that count omits.
///
/// A bounded observer must report what its bound cost (RFC 09 §5.1 O6): a key
/// set that stops growing, or shrinks, is otherwise indistinguishable from a
/// bus that went quiet.
fn keys_label<'a>(keys: usize, evicted: u64) -> Element<'a, Message> {
    let label = keys_text(keys, evicted);
    if evicted == 0 {
        return kit::muted(label);
    }
    text(label)
        .size(font::CAPTION)
        .style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).warning()),
        })
        .into()
}

/// The wording of the key count. Split from the widget so it is testable.
pub fn keys_text(keys: usize, evicted: u64) -> String {
    if evicted == 0 {
        kit::plural(keys, "key")
    } else {
        format!(
            "{} (+{evicted} retired — bound reached)",
            kit::plural(keys, "key")
        )
    }
}

pub fn strip<'a>(s: Status<'a>) -> Element<'a, Message> {
    let (link_text, link_is_bad) = match s.link {
        LinkState::Connecting => ("connecting…".to_string(), false),
        LinkState::Pumping if s.watched.is_empty() => (
            // The lazy resting state, stated as a posture, not an absence.
            "skeleton only — nothing observed by choice".to_string(),
            false,
        ),
        LinkState::Pumping => (
            format!("observing {}", kit::plural(s.watched.len(), "watch")),
            false,
        ),
        LinkState::Ended => ("link ended — retrying".to_string(), true),
        LinkState::Failed(e) => (format!("link failed: {e}"), true),
    };

    let link = text(link_text)
        .size(font::CAPTION)
        .style(move |theme: &iced::Theme| text::Style {
            color: Some(if link_is_bad {
                colors(theme).danger()
            } else {
                colors(theme).text_muted()
            }),
        });

    let (count, bytes, rate) = s.totals;

    let mut r = row![
        link,
        kit::muted(format!("base: {}", s.base_label)),
        kit::muted(format!("scope: {}", s.scope_label)),
        keys_label(s.keys, s.keys_evicted),
        kit::muted(format!(
            "{count} samples · {} · {}",
            human_bytes(bytes),
            human_rate(rate)
        )),
        kit::muted(s.slices.label()),
        kit::muted(match s.skeleton {
            None => "skeleton: not built".to_string(),
            Some(c) => format!(
                "skeleton: {} · {} · admin {}",
                kit::plural(c.slices, "slice"),
                kit::plural(c.roster_origins, "origin"),
                match c.admin_entities {
                    // Not asked / not available is not a zero (O4).
                    None => "n/a".to_string(),
                    Some(n) => n.to_string(),
                }
            ),
        }),
    ]
    .spacing(space::MD)
    .align_y(iced::Alignment::Center);

    // The seed phase is stated while it runs and summarized once done —
    // zeros are observations (O4): a seed that ran and found nothing says
    // so, it does not vanish.
    if s.seeding > 0 {
        r = r.push(kit::muted(format!(
            "seeding {}…",
            kit::plural(s.seeding, "watch")
        )));
    } else if s.seeded_watches > 0 {
        let (cache, storage, superseded) = s.seed_totals;
        r = r.push(kit::muted(format!(
            "seeded {}: {cache} cache · {storage} storage · {superseded} superseded",
            kit::plural(s.seeded_watches, "watch")
        )));
    }
    if s.keys_unwatched > 0 {
        r = r.push(kit::muted(format!(
            "{} keys retired by unwatch",
            s.keys_unwatched
        )));
    }
    if let Some((key, outcome)) = s.fetched {
        let label = match outcome {
            Ok(out) => match out.as_ref() {
                zenkey_fleet::FetchOutcome::Value(v) => {
                    format!(
                        "fetched {key}: {} bytes via {:?}",
                        v.payload.len(),
                        v.source
                    )
                }
                zenkey_fleet::FetchOutcome::None { attempted } => format!(
                    "no value for {key} — asked {} (not a verdict)",
                    attempted.join(", ")
                ),
            },
            Err(e) => format!("fetch {key} failed: {e}"),
        };
        r = r.push(kit::muted(label));
    }

    // The single most misleading state a bus explorer can be in: a healthy
    // window, an empty tree, and no way to tell that the session never reached
    // anything. Say it outright.
    if s.unreachable {
        r = r.push(
            text("no endpoints and scouting off — this session reaches nothing")
                .size(font::CAPTION)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                }),
        );
    }

    r.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Not loaded" and "loaded, and empty" are different facts and must read
    /// differently — otherwise a missing registry looks like a bus with no
    /// registered producers.
    #[test]
    fn slice_source_distinguishes_absent_from_empty() {
        assert_ne!(
            SliceSource::None.label(),
            SliceSource::Bus { count: 0 }.label()
        );
        assert!(SliceSource::None.label().contains("not loaded"));
        assert!(SliceSource::Bus { count: 3 }.label().contains('3'));
        assert!(SliceSource::Dirs { count: 7 }.label().contains("dirs"));
        assert!(SliceSource::Failed("boom".into()).label().contains("boom"));
    }

    /// RFC 09 §5.1 O6: a bounded observer reports what its bound cost. Without
    /// this, a key count that stops growing looks like a bus that went quiet.
    #[test]
    fn the_key_count_discloses_eviction() {
        assert_eq!(keys_text(120, 0), "120 keys");
        let noted = keys_text(120, 7);
        assert!(noted.contains("120 keys"), "{noted}");
        assert!(noted.contains('7'), "{noted}");
        assert!(noted.contains("retired"), "{noted}");
    }
}
