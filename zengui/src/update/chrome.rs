//! The window, and what floats over it (#175).
//!
//! `(&mut Chrome, &Deployment, &Subject, &mut Workspace, ChromeMsg)`. The two
//! it does not name are the tree and the observation, which is the sharpest
//! statement in the file: the theme, the zoom, the geometry and the palette
//! cannot move a row or a watch.
//!
//! `&mut Workspace` is here for the palette's jump-to-key, which lands the
//! detail pane — selecting without switching panes would look like nothing
//! happened.

use iced::Task;

use crate::message::{ChromeMsg, Message, RightPane, Subject, SubjectMsg};
use crate::state::{Chrome, Deployment, SubjectState, Workspace};
use crate::view;

/// The window, and what floats over it.
pub(crate) fn update(
    chrome: &mut Chrome,
    dep: &Deployment,
    sub: &SubjectState,
    work: &mut Workspace,
    msg: ChromeMsg,
) -> Task<Message> {
    match msg {
        ChromeMsg::WindowResized(w, h) => {
            // Not on every pixel of a drag — the prefs file would be
            // rewritten hundreds of times per resize. Recorded here and
            // marked dirty; a settle timer writes it once the drag stops
            // (issue #189). "Written on the next real change" meant a
            // resize-then-quit lost the geometry entirely.
            chrome.prefs.window = Some((w, h));
            chrome.window_dirty = true;
            Task::none()
        }
        ChromeMsg::WindowSettled => {
            if chrome.window_dirty {
                remember(chrome, dep, work);
            }
            Task::none()
        }
        ChromeMsg::Key(key, modifiers) => update_key(chrome, dep, sub, work, &key, modifiers),
        ChromeMsg::Palette(msg) => update_palette(chrome, dep, work, msg),
        ChromeMsg::Prefs(msg) => {
            use crate::message::PrefsMsg;
            match msg {
                PrefsMsg::ThemeToggled => chrome.prefs.theme = chrome.prefs.theme.toggled(),
                PrefsMsg::ZoomIn => chrome.prefs.zoom_in(),
                PrefsMsg::ZoomOut => chrome.prefs.zoom_out(),
                PrefsMsg::ZoomReset => chrome.prefs.zoom_reset(),
            }
            // Saved on every change rather than at exit: a GUI is killed,
            // not quit, more often than anyone admits.
            remember(chrome, dep, work);
            Task::none()
        }
    }
}

/// One key press, in context.
///
/// **Esc layering** (#75): palette first, then a tree selection, then
/// nothing — one layer per press, so Esc never does two things at once.
/// The arrows drive the overlay only while one is open, which is what
/// keeps them available to the panes the rest of the time.
fn update_key(
    chrome: &mut Chrome,
    dep: &Deployment,
    sub: &SubjectState,
    work: &mut Workspace,
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Task<Message> {
    use iced::keyboard::{Key, key::Named};
    use view::palette::PaletteMsg;

    if crate::shortcuts::is_escape(key) {
        if chrome.palette.is_open() {
            chrome.palette.close();
        } else if sub.current != Subject::None {
            return Task::done(Message::Subject(SubjectMsg::Select(Subject::None)));
        }
        return Task::none();
    }
    if chrome.palette.is_open() {
        match key {
            Key::Named(Named::ArrowDown) => {
                return update_palette(chrome, dep, work, PaletteMsg::CursorDown);
            }
            Key::Named(Named::ArrowUp) => {
                return update_palette(chrome, dep, work, PaletteMsg::CursorUp);
            }
            Key::Named(Named::Enter) => {
                return update_palette(chrome, dep, work, PaletteMsg::Activate);
            }
            _ => {}
        }
    }
    match crate::shortcuts::resolve(key, modifiers) {
        Some(message) => Task::done(message),
        None => Task::none(),
    }
}

/// The command palette (#75).
///
/// Every activation *returns* the action's own message as a `Task::done`,
/// which is what keeps the palette from being a second implementation of
/// anything: it is a faster way to send a message the UI already sends,
/// and nothing more. It used to re-enter `update` directly; the message
/// goes back out to iced now, which changes nothing about ordering — a
/// `Task::done` resolves immediately — and everything about what a
/// handler is allowed to reach.
fn update_palette(
    chrome: &mut Chrome,
    dep: &Deployment,
    work: &mut Workspace,
    msg: view::palette::PaletteMsg,
) -> Task<Message> {
    use view::palette::PaletteMsg;
    match msg {
        PaletteMsg::Open(overlay) => {
            chrome.palette.open(overlay);
            Task::none()
        }
        PaletteMsg::Close => {
            chrome.palette.close();
            Task::none()
        }
        PaletteMsg::QueryChanged(q) => {
            chrome.palette.query = q;
            // A new query re-ranks the list, so the old cursor points at a
            // different row — start from the best match again.
            chrome.palette.cursor = 0;
            Task::none()
        }
        PaletteMsg::CursorUp => {
            chrome.palette.cursor = chrome.palette.cursor.saturating_sub(1);
            Task::none()
        }
        PaletteMsg::CursorDown => {
            chrome.palette.cursor = chrome
                .palette
                .cursor
                .saturating_add(1)
                .min(palette_row_count(chrome, dep, work).saturating_sub(1));
            Task::none()
        }
        PaletteMsg::Activate => run_palette_row(chrome, dep, work, chrome.palette.cursor),
        PaletteMsg::Pick(i) => run_palette_row(chrome, dep, work, i),
    }
}

/// How many rows the open overlay currently shows. Ranks over borrowed
/// keys (fat pointers, no string bytes) — runs per keypress, not per
/// frame (#110).
fn palette_row_count(chrome: &Chrome, dep: &Deployment, work: &Workspace) -> usize {
    use view::palette::{Overlay, actions, rank};
    match chrome.palette.overlay {
        Overlay::Commands => {
            let items = actions(&work.bench.context_form.known);
            rank(&items, &chrome.palette.query, |a| a.label.as_str()).len()
        }
        Overlay::Keys => {
            // Observed keys only — never a guess (O4): the jump-to
            // overlay offers what is on the bus, not what a registry
            // says could be.
            let keys: Vec<&str> = dep.facts.keys().collect();
            rank(&keys, &chrome.palette.query, |k| *k).len()
        }
        _ => 0,
    }
}

/// The message behind row `index` — on the Keys overlay, the one place
/// the palette ever clones a key `String` (#110): the activated row.
fn palette_row(
    chrome: &Chrome,
    dep: &Deployment,
    work: &Workspace,
    index: usize,
) -> Option<Message> {
    use view::palette::{Overlay, actions, rank};
    match chrome.palette.overlay {
        Overlay::Commands => {
            let items = actions(&work.bench.context_form.known);
            let order = rank(&items, &chrome.palette.query, |a| a.label.as_str());
            order.get(index).map(|i| items[*i].message.clone())
        }
        Overlay::Keys => {
            let keys: Vec<&str> = dep.facts.keys().collect();
            let order = rank(&keys, &chrome.palette.query, |k| *k);
            order
                .get(index)
                .map(|i| Message::Subject(SubjectMsg::Select(Subject::Key(keys[*i].to_string()))))
        }
        _ => None,
    }
}

fn run_palette_row(
    chrome: &mut Chrome,
    dep: &Deployment,
    work: &mut Workspace,
    index: usize,
) -> Task<Message> {
    let Some(message) = palette_row(chrome, dep, work, index) else {
        return Task::none();
    };
    // Jumping to a key also shows it: selecting without switching panes
    // would look like nothing happened.
    if matches!(chrome.palette.overlay, view::palette::Overlay::Keys) {
        work.right_pane = RightPane::Detail;
    }
    chrome.palette.close();
    Task::done(message)
}

/// Persist what the window looks like now. Best-effort by construction
/// (see `Prefs::save`) — a preference that cannot be written must not fail
/// whatever the user was actually doing.
pub(crate) fn remember(chrome: &mut Chrome, dep: &Deployment, work: &Workspace) {
    chrome.prefs.scope = dep.settings.scope;
    chrome.prefs.context = work
        .bench
        .context_form
        .active
        .clone()
        .or(chrome.prefs.context.take());
    chrome.window_dirty = false;
    chrome.prefs.save();
}
