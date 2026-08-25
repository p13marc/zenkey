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
use crate::services;
use crate::state::workspace::ClosedWindow;
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
        ChromeMsg::WindowResized(id, w, h) => {
            // Not on every pixel of a drag — the prefs file would be
            // rewritten hundreds of times per resize. Recorded here and
            // marked dirty; a settle timer writes it once the drag stops
            // (issue #189). "Written on the next real change" meant a
            // resize-then-quit lost the geometry entirely.
            //
            // Window-aware since #186: a torn-off dock's size belongs to
            // the named layout its window is part of, the main window's to
            // the next launch as before. An id naming neither is a close
            // racing its last resize, and owes nothing.
            match work.windows.classify(id) {
                ClosedWindow::Torn(_) => {
                    if let Some(t) = work.windows.torn.iter_mut().find(|t| t.id == id) {
                        t.size = Some((w, h));
                    }
                    super::workspace::persist_custom(chrome, work);
                }
                ClosedWindow::Main => {
                    chrome.prefs.window = Some((w, h));
                    chrome.prefs_dirty = true;
                }
                // A resize racing a close: the window is gone, and writing
                // its size over the main window's would be the bug.
                ClosedWindow::Unknown => {}
            }
            Task::none()
        }
        ChromeMsg::WindowMoved(id, x, y) => {
            // Only a torn-off dock's position is remembered (#186): "echo
            // on the second monitor" should reopen there. The main window
            // keeps its platform-default placement, as it always has — and
            // platforms that never report a move (Wayland) simply never
            // send this.
            if work.windows.role_of(id).is_some() {
                if let Some(t) = work.windows.torn.iter_mut().find(|t| t.id == id) {
                    t.position = Some((x, y));
                }
                super::workspace::persist_custom(chrome, work);
            }
            Task::none()
        }
        ChromeMsg::WindowSettled => {
            if chrome.prefs_dirty {
                remember(chrome, dep, work);
                chrome.prefs_dirty = false;
                // The write itself is a task (#255): the settle timer
                // decides *when* a write is owed; the disk never runs on
                // the update thread.
                return services::prefs::save(chrome.prefs.clone());
            }
            Task::none()
        }
        ChromeMsg::PrefsSaved(Ok(())) => Task::none(),
        ChromeMsg::PrefsSaved(Err(e)) => {
            // Best-effort, but not silent (#255): the note the status strip
            // already shows for an unreadable prefs file is the surface for
            // an unwritable one too.
            tracing::warn!("preferences not saved: {e}");
            chrome.prefs_note = Some(format!("preferences not saved: {e}"));
            Task::none()
        }
        ChromeMsg::Key(key, modifiers) => update_key(chrome, dep, sub, work, &key, modifiers),
        ChromeMsg::Palette(msg) => update_palette(chrome, dep, work, msg),
        ChromeMsg::Prefs(msg) => {
            use crate::message::PrefsMsg;
            match msg {
                PrefsMsg::ThemeToggled => chrome.prefs.theme = chrome.prefs.theme.toggled(),
                PrefsMsg::DensityToggled => {
                    chrome.prefs.density = chrome.prefs.density.toggled();
                }
                PrefsMsg::ZoomIn => chrome.prefs.zoom_in(),
                PrefsMsg::ZoomOut => chrome.prefs.zoom_out(),
                PrefsMsg::ZoomReset => chrome.prefs.zoom_reset(),
            }
            // On the settle timer rather than per change (#255): a held
            // zoom key wrote the file once per keypress, which is #189's
            // per-pixel lesson wearing a keyboard. The 700 ms window is the
            // exposure a kill -9 already had between change and write.
            remember(chrome, dep, work);
            Task::none()
        }
    }
}

/// One key press, in context.
///
/// The modifier-less keys arrive as the [`Chord`](crate::shortcuts::Chord)
/// the shortcut table names (#190) — never the raw key, so a bare key the
/// app answers cannot exist outside `shortcuts.rs`. What each chord *means*
/// is still decided here, because it depends on what is open:
///
/// **Esc layering** (#75): palette first, then a tree selection, then
/// nothing — one layer per press, so Esc never does two things at once.
/// The arrows and Enter drive the overlay only while one is open, which is
/// what keeps them available to the panes the rest of the time.
fn update_key(
    chrome: &mut Chrome,
    dep: &Deployment,
    sub: &SubjectState,
    work: &mut Workspace,
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Task<Message> {
    use crate::shortcuts::Chord;
    use view::palette::{Overlay, PaletteMsg};

    match crate::shortcuts::chord(key) {
        Some(Chord::Escape) => {
            if chrome.palette.is_open() {
                chrome.palette.close();
            } else if sub.follow.current != Subject::None {
                return Task::done(Message::Subject(SubjectMsg::Select(Subject::None)));
            }
            return Task::none();
        }
        Some(Chord::Down) if chrome.palette.is_open() => {
            return update_palette(chrome, dep, work, PaletteMsg::CursorDown);
        }
        Some(Chord::Up) if chrome.palette.is_open() => {
            return update_palette(chrome, dep, work, PaletteMsg::CursorUp);
        }
        Some(Chord::Enter) if chrome.palette.is_open() => {
            return update_palette(chrome, dep, work, PaletteMsg::Activate);
        }
        // `?` reaches here only when no text input consumed it, so it is
        // safe bare — the one key every tool in this family answers.
        Some(Chord::Help) => {
            return Task::done(Message::Chrome(ChromeMsg::Palette(PaletteMsg::Open(
                Overlay::Help,
            ))));
        }
        _ => {}
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
            // The modal editors open on the deployment's current truth — a
            // draft left from last time must not masquerade as the active
            // scope (#187) or the settings in force (#188).
            if overlay == view::palette::Overlay::Selectors {
                work.bench
                    .scope_form
                    .seed(dep.settings.scope, &dep.settings.selectors);
            }
            if overlay == view::palette::Overlay::Settings {
                work.bench.settings_form.seed(&dep.settings);
            }
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
    chrome.palette.close();
    // Jumping to a key also shows it: selecting without revealing the
    // Inspector dock would look like nothing happened. Spoken as the same
    // `PaneSelected` every other reveal uses (#180), so a closed dock is
    // restored — and persisted — by the one handler that owns the layout.
    if matches!(chrome.palette.overlay, view::palette::Overlay::Keys) {
        return Task::done(Message::Workspace(
            crate::message::WorkspaceMsg::PaneSelected(RightPane::Inspector),
        ))
        .chain(Task::done(message));
    }
    Task::done(message)
}

/// Record what the window looks like now, and mark the prefs dirty — the
/// settle timer's `WindowSettled` does the actual write, as a task (#255).
///
/// No disk here, deliberately: `remember` is called from the update thread
/// on every theme toggle, zoom step, scope change and context switch, and
/// each used to be a synchronous file write on a frame. The debounce seam
/// is `prefs_dirty` (#189), and now every preference rides it.
pub(crate) fn remember(chrome: &mut Chrome, dep: &Deployment, work: &Workspace) {
    chrome.prefs.scope = dep.settings.scope;
    // The selectors that give a custom scope its meaning travel with it
    // (#187): a remembered `custom` used to be dropped on the next launch
    // because these were session state.
    chrome.prefs.selectors = dep.settings.selectors.clone();
    chrome.prefs.context = work
        .bench
        .context_form
        .active
        .clone()
        .or(chrome.prefs.context.take());
    chrome.prefs_dirty = true;
}
