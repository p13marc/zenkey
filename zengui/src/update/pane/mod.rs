//! One module per pane-shaped surface (#175), each taking the state it can
//! move — the right-hand panes, the Inspector's sections (#182), the dock's
//! streams (#183), and the Connect overlay (#185).
//!
//! Most surfaces have state of their own. Detail and History do not: they
//! are windows onto the selected key, so they take `&mut Subject` and
//! nothing else. That is worth knowing before #180 docks the panes — two of
//! the docks have nothing to dock.

use iced::Task;

use crate::message::{Message, PaneMsg};
use crate::state::{Deployment, Observation, SubjectState, Workspace};
use crate::update::Ctx;

pub(crate) mod admin;
pub(crate) mod blob;
pub(crate) mod context;
pub(crate) mod detail;
pub(crate) mod doctor;
pub(crate) mod echo;
pub(crate) mod fields;
pub(crate) mod media;
pub(crate) mod nodes;
pub(crate) mod replay;
pub(crate) mod scope_editor;
pub(crate) mod send;
pub(crate) mod settings;
pub(crate) mod why;

/// One pane-shaped surface.
pub(crate) fn update(
    dep: &mut Deployment,
    obs: &Observation,
    sub: &mut SubjectState,
    work: &mut Workspace,
    msg: PaneMsg,
) -> Task<Message> {
    // Built once: `Ctx` is `Copy`, and every arm that wants it wants the same
    // one. What differs between the arms is only what each pane may *move*.
    let cx = Ctx { dep, obs, sub };
    match msg {
        PaneMsg::Send(msg) => send::update(&mut work.bench, msg, cx),
        PaneMsg::Nodes(msg) => nodes::update(&mut work.verdicts, msg, cx),
        PaneMsg::Doctor(msg) => doctor::update(&mut work.verdicts, msg, cx),
        PaneMsg::Blob(msg) => blob::update(&mut work.verdicts.blob, msg, cx),
        PaneMsg::Media(msg) => media::update(&mut work.bench.media, &work.verdicts.roster, msg, cx),
        PaneMsg::Admin(msg) => admin::update(&mut work.verdicts.admin, msg, cx),
        // The four subject-slot sections (#257): the SlotId names which
        // Inspector spoke — the docked one is the follow slot, a pinned
        // window its own. This is the one routing point; the handlers below
        // take the resolved slot and never know which surface they serve.
        // A message for a dropped slot lands nowhere: its window is gone,
        // and with it the only surface that could display the result.
        PaneMsg::Detail(id, msg) => match sub.slot_mut(id) {
            Some(slot) => detail::update(slot, dep, msg),
            None => Task::none(),
        },
        PaneMsg::History(id, msg) => match sub.slot_mut(id) {
            Some(slot) => detail::history(slot, msg),
            None => Task::none(),
        },
        // Two more windows onto a slot (#223, #214) — like Detail and
        // History they take the slot and the deployment, not `cx`.
        PaneMsg::Fields(id, msg) => match sub.slot_mut(id) {
            Some(slot) => fields::update(slot, dep, msg),
            None => Task::none(),
        },
        PaneMsg::Why(id, msg) => match sub.slot_mut(id) {
            Some(slot) => why::update(slot, dep, msg),
            None => Task::none(),
        },
        PaneMsg::Echo(msg) => echo::update(&mut work.echo, msg, cx),
        PaneMsg::Context(msg) => context::update(&mut work.bench.context_form, msg),
        PaneMsg::Scope(msg) => scope_editor::update(&mut work.bench.scope_form, msg, cx),
        PaneMsg::Settings(msg) => settings::update(&mut work.bench.settings_form, msg),
    }
}
