//! The mesh tool's handler (#541).

use iced::Task;

use crate::admin::AdminState;
use crate::message::Message;
use crate::view::mesh::MeshMsg;

/// The mesh reads the admin sweep and moves nothing of it: its one message
/// copies the drawing out.
pub(crate) fn update(admin: &AdminState, msg: MeshMsg) -> Task<Message> {
    match msg {
        MeshMsg::CopyDot => {
            // The Graphviz export (#234): the engine's `render_dot` over the
            // sweep already in hand — no bus cost, clipboard-bound exactly
            // like echo's ndjson export. No sweep, nothing to copy: the
            // button only renders inside a swept topology.
            let Some(sweep) = admin.sweep.as_deref() else {
                return Task::none();
            };
            iced::clipboard::write(zenkey_fleet::render_dot(&sweep.topology, &sweep.origins))
        }
    }
}
