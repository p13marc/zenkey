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
pub(crate) mod call;
pub(crate) mod context;
pub(crate) mod detail;
pub(crate) mod doctor;
pub(crate) mod echo;
pub(crate) mod media;
pub(crate) mod nodes;
pub(crate) mod publish;
pub(crate) mod replay;

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
        PaneMsg::Call(msg) => call::update(&mut work.bench.call_form, msg, cx),
        PaneMsg::Nodes(msg) => nodes::update(&mut work.verdicts, msg, cx),
        PaneMsg::Doctor(msg) => doctor::update(&mut work.verdicts, &mut work.right_pane, msg, cx),
        PaneMsg::Blob(msg) => blob::update(&mut work.verdicts.blob, msg, cx),
        PaneMsg::Media(msg) => media::update(&mut work.bench.media, &work.verdicts.roster, msg, cx),
        PaneMsg::Admin(msg) => admin::update(&mut work.verdicts.admin, msg, cx),
        PaneMsg::Detail(msg) => detail::update(sub, dep, msg),
        PaneMsg::History(msg) => detail::history(sub, msg),
        PaneMsg::Publish(msg) => publish::update(&mut work.bench, msg, cx),
        PaneMsg::Echo(msg) => echo::update(&mut work.echo, &mut work.right_pane, msg, cx),
        PaneMsg::Context(msg) => context::update(&mut work.bench.context_form, msg),
    }
}
