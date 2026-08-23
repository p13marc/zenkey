//! The key-expression editor (#187) — behind the location bar's scope chip.
//!
//! Custom selectors were CLI-only, and the picker lied about it: `--scope
//! custom --selector …` produced a `pick_list` whose selected value was not
//! among its own options, and nothing in the UI could see, edit or add a key
//! expression — the single most important thing an explorer lets you type.
//!
//! Two modes, one surface. On a preset, it shows the **resolved** selectors
//! read-only — via [`ScopePreset::selectors`], the single place selectors are
//! built, so the `Deployment` preset keeps naming `catalog_subtree()`
//! explicitly (RFC 03 §4 D4) and no `format!` of a key expression appears
//! here. Forking copies that resolved set into an editable draft; applying it
//! makes the scope [`ScopePreset::Custom`].
//!
//! Every row is validated as typed with [`crate::scope::validate_selector`]
//! (empty and `$*` are refused — RFC 03 §2 forbids `$*` in selectors, not
//! merely in published keys), and every row carries its
//! [`crate::scope::blind_spot`]: the D2 rule is the one users get wrong, so
//! the editor states it inline rather than trusting a tooltip.

use iced::Element;
use iced::widget::{Column, column, row, text};

use crate::message::{Message, PaneMsg};
use crate::scope::ScopePreset;
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// The editor's draft state (owned by the app, like the Connect form).
#[derive(Debug, Clone, Default)]
pub struct ScopeForm {
    /// Whether the draft rows are being edited. `false` renders the current
    /// scope's resolved selectors read-only, with the fork as the way in.
    pub editing: bool,
    /// The draft selector strings — the user's own text, validated per
    /// keystroke and applied only as a set.
    pub rows: Vec<String>,
    /// The last apply's outcome, rendered verbatim.
    pub status: Option<Result<String, String>>,
}

impl ScopeForm {
    /// Reset the draft to the deployment's current truth — what opening the
    /// overlay does, so a stale draft never masquerades as the active scope.
    pub fn seed(&mut self, scope: ScopePreset, selectors: &[String]) {
        self.editing = scope == ScopePreset::Custom;
        self.rows = selectors.to_vec();
        self.status = None;
    }
}

/// Messages the editor emits.
#[derive(Debug, Clone)]
pub enum ScopeMsg {
    /// Copy the resolved selectors of the current scope into the draft and
    /// start editing.
    Fork,
    RowChanged(usize, String),
    RowAdded,
    RowRemoved(usize),
    /// Validate the whole draft; a valid set becomes the custom scope.
    Apply,
}

fn msg(m: ScopeMsg) -> Message {
    Message::Pane(PaneMsg::Scope(m))
}

/// Everything the editor shows, as plain data.
pub struct ScopeEditorData<'a> {
    pub scope: ScopePreset,
    /// The deployment base in force; `""` is the bus root.
    pub base: &'a str,
    /// The custom selectors in force (empty unless the scope is custom).
    pub selectors: &'a [String],
    pub form: &'a ScopeForm,
}

/// The D2 statement the whole editor hangs off — inline, because it is the
/// rule users get wrong (`scope.rs`'s module doc, on screen). `pub` so the
/// pane test pins the exact wording that reaches the screen.
pub const D2_RULE: &str = "* and ** never cross a chunk beginning with @ (RFC 03 §4 D2). \
     A ** scope is media-safe by key algebra — and equally cannot see @catalog. \
     It is not \"everything\", and each selector below names what it cannot see.";

pub fn pane(d: ScopeEditorData<'_>) -> Element<'_, Message> {
    let mut col = column![
        kit::section_header("Scope selectors", None),
        kit::caption(format!("scope: {} — {}", d.scope.short(), d.scope.label())),
        kit::body(D2_RULE).style(|theme: &iced::Theme| text::Style {
            color: Some(colors(theme).text_muted()),
        }),
    ]
    .spacing(space::SM);

    if d.form.editing {
        col = editing(col, d.form);
    } else {
        col = resolved(col, &d);
    }

    if let Some(status) = &d.form.status {
        col = col.push(match status {
            Ok(s) => kit::muted(s.clone()),
            Err(e) => kit::body(e.clone())
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                })
                .into(),
        });
    }
    col.into()
}

/// The read-only face: what the preset actually subscribes to, resolved by
/// the one place selectors are built.
fn resolved<'a>(mut col: Column<'a, Message>, d: &ScopeEditorData<'_>) -> Column<'a, Message> {
    col = col.push(kit::muted(
        "resolved read-only — fork to edit them as a custom scope",
    ));
    for sel in d.scope.selectors(d.base, d.selectors) {
        col = col.push(selector_row(sel));
    }
    col.push(
        kit::action(kit::caption("fork into custom and edit"))
            .on_press(msg(ScopeMsg::Fork))
            .padding(4),
    )
}

/// One resolved selector and its blind spot.
fn selector_row<'a>(sel: String) -> Element<'a, Message> {
    let spot = crate::scope::blind_spot(&sel);
    column![
        kit::caption(sel).font(iced::Font::MONOSPACE),
        kit::muted(format!("  cannot see: {spot}")),
    ]
    .spacing(1)
    .into()
}

/// The editable face: one text box per selector, validated per keystroke.
fn editing<'a>(mut col: Column<'a, Message>, form: &'a ScopeForm) -> Column<'a, Message> {
    for (i, sel) in form.rows.iter().enumerate() {
        col = col.push(
            row![
                kit::input("key expression, e.g. demo/**", sel)
                    .on_input(move |t| msg(ScopeMsg::RowChanged(i, t)))
                    .on_submit(msg(ScopeMsg::Apply))
                    .font(iced::Font::MONOSPACE)
                    .size(font::CAPTION),
                kit::action(kit::caption("remove"))
                    .on_press(msg(ScopeMsg::RowRemoved(i)))
                    .padding(4),
            ]
            .spacing(space::SM)
            .align_y(iced::Alignment::Center),
        );
        // The per-keystroke verdict: an invalid row says why, a valid one
        // says what it cannot see — never both, never neither.
        col = col.push(match crate::scope::validate_selector(sel) {
            Err(e) => Element::from(kit::body(format!("  {e}")).style(|theme: &iced::Theme| {
                text::Style {
                    color: Some(colors(theme).danger()),
                }
            })),
            Ok(()) => kit::muted(format!("  cannot see: {}", crate::scope::blind_spot(sel))),
        });
    }
    col.push(
        row![
            kit::action(kit::caption("add selector"))
                .on_press(msg(ScopeMsg::RowAdded))
                .padding(4),
            kit::action(kit::caption("apply — the scope becomes custom"))
                .on_press(msg(ScopeMsg::Apply))
                .padding(4),
        ]
        .spacing(space::SM),
    )
}
