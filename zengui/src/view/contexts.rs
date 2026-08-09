//! The connection pane (issue #67) — contexts, endpoints, and what scouting
//! actually means.
//!
//! The connect flow was command-line-only: endpoints, scouting and registry
//! dirs were fixed at launch, and the only in-app control was the base picker.
//! With the context store in the engine (#35) both explorers read and write the
//! **same** named contexts, so a context created in `zenctl` is one this pane
//! can select, and vice versa — that reciprocity is the point, and it is what
//! the acceptance test pins.
//!
//! ## Why the scouting help text is this long
//!
//! RFC 09 §0.1: multicast scouting and gossip are **independent** switches, and
//! "scouting off" in every explorer here means the multicast half only. That
//! distinction is invisible in a checkbox labelled "scouting", and getting it
//! wrong is how a throwaway session ends up in a production fleet — or, in the
//! other direction, how someone concludes a fleet is down when they have merely
//! isolated themselves from it. The pane spends the words.

use iced::widget::{Column, button, checkbox, column, pick_list, row, text, text_input};
use iced::{Element, Length};
use zenkey_fleet::StoredContext;

use crate::message::Message;
use crate::view::kit;
use crate::view::theme::colors;
use crate::view::tokens::{font, space};

/// The pane's editable state (owned by the app).
#[derive(Debug, Clone, Default)]
pub struct ContextForm {
    /// Contexts as the shared config file has them, newest read.
    pub known: Vec<String>,
    /// The context in force, when one is.
    pub active: Option<String>,
    /// Name being created or edited.
    pub name: String,
    /// Comma- or space-separated endpoints.
    pub connect: String,
    pub listen: String,
    pub base: String,
    pub scouting: bool,
    /// The last action's outcome, rendered verbatim.
    pub status: Option<Result<String, String>>,
}

/// Messages the pane emits.
#[derive(Debug, Clone)]
pub enum ContextMsg {
    Selected(String),
    NameChanged(String),
    ConnectChanged(String),
    ListenChanged(String),
    BaseChanged(String),
    ScoutingToggled(bool),
    /// Load the named context's values into the editor.
    Load,
    /// Write the editor's values to the shared config.
    Save,
    /// Save, then switch to it.
    SaveAndSelect,
    /// The isolated-verification recipe, one click (RFC 09 §0.1): multicast
    /// off, explicit endpoints, nothing discovered by accident.
    Isolate,
}

fn msg(m: ContextMsg) -> Message {
    Message::Context(m)
}

/// Split a user-typed endpoint list. Commas, spaces and newlines all work —
/// this is a text box, not a parser exercise.
pub fn endpoints(text: &str) -> Vec<String> {
    text.split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Catch the one endpoint mistake worth catching in a text box: a bare
/// `host:port` with no protocol, which zenoh accepts into a config and then
/// silently never connects.
///
/// Deliberately **not** a protocol allow-list. zenoh's transport set grows,
/// and a pane that refused `quic/…` because this build had not heard of it
/// would be worse than useless — the shape check is the part that is safe to
/// assert without pretending to know every protocol.
pub fn validate_endpoints(list: &[String]) -> Result<(), String> {
    for e in list {
        if !e.contains('/') {
            return Err(format!(
                "{e:?} is not an endpoint — expected <protocol>/<address>, \
                 e.g. tcp/127.0.0.1:7447"
            ));
        }
    }
    Ok(())
}

impl ContextForm {
    /// The editor's values as a storable context.
    pub fn to_stored(&self) -> Result<StoredContext, String> {
        let connect = endpoints(&self.connect);
        let listen = endpoints(&self.listen);
        validate_endpoints(&connect)?;
        validate_endpoints(&listen)?;
        if self.name.trim().is_empty() {
            return Err("a context needs a name".into());
        }
        Ok(StoredContext {
            // An explicitly empty base is a real deployment (the bus root,
            // RFC v1.6) — stored as `Some("")` rather than dropped, so it
            // overrides a different default rather than falling through it.
            base: Some(self.base.clone()),
            connect,
            listen,
            registry: Vec::new(),
            scouting: Some(self.scouting),
            timeout: None,
        })
    }

    /// Fill the editor from a stored context.
    pub fn load_from(&mut self, name: &str, stored: &StoredContext) {
        self.name = name.to_string();
        self.base = stored.base.clone().unwrap_or_default();
        self.connect = stored.connect.join(" ");
        self.listen = stored.listen.join(" ");
        self.scouting = stored.scouting.unwrap_or(false);
    }
}

pub fn pane<'a>(form: &'a ContextForm, unreachable: bool) -> Element<'a, Message> {
    let picker = pick_list(form.known.clone(), form.active.clone(), |n| {
        msg(ContextMsg::Selected(n))
    })
    .placeholder("context")
    .text_size(font::CAPTION);

    let mut col = column![
        kit::section_header("Connection", None),
        row![
            text("context").size(font::CAPTION),
            picker,
            button(text("load into editor").size(font::CAPTION))
                .on_press(msg(ContextMsg::Load))
                .padding(4),
        ]
        .spacing(space::SM)
        .align_y(iced::Alignment::Center),
        kit::muted(
            "Contexts are shared with zenctl — one file, two explorers \
             (~/.config/zenkey-explorer/config.toml)."
        ),
    ]
    .spacing(space::SM);

    if unreachable {
        // The same warning the status strip carries, repeated where the fix
        // is: an empty tree and a session that reaches nothing look identical.
        col = col.push(
            text(
                "this session has no endpoints and multicast scouting is off — \
                 it reaches nothing",
            )
            .size(font::CAPTION)
            .style(|theme: &iced::Theme| text::Style {
                color: Some(colors(theme).danger()),
            }),
        );
    }

    col = col.push(kit::section_header("Edit", None));
    col = col.push(
        text_input("name", &form.name)
            .on_input(|t| msg(ContextMsg::NameChanged(t)))
            .size(font::CAPTION),
    );
    col = col.push(
        text_input("base (empty = the bus root, keys start at v1/)", &form.base)
            .on_input(|t| msg(ContextMsg::BaseChanged(t)))
            .size(font::CAPTION),
    );
    col = col.push(
        text_input("connect: tcp/127.0.0.1:7447 …", &form.connect)
            .on_input(|t| msg(ContextMsg::ConnectChanged(t)))
            .size(font::CAPTION),
    );
    col = col.push(
        text_input("listen: tcp/0.0.0.0:7448 …", &form.listen)
            .on_input(|t| msg(ContextMsg::ListenChanged(t)))
            .size(font::CAPTION),
    );
    col = col.push(
        checkbox(form.scouting)
            .label("multicast scouting")
            .on_toggle(|b| msg(ContextMsg::ScoutingToggled(b)))
            .text_size(font::CAPTION),
    );
    col = col.push(scouting_help());
    col = col.push(
        row![
            button(text("save").size(font::CAPTION))
                .on_press(msg(ContextMsg::Save))
                .padding(4),
            button(text("save + switch to it").size(font::CAPTION))
                .on_press(msg(ContextMsg::SaveAndSelect))
                .padding(4),
            button(text("isolated-verification preset").size(font::CAPTION))
                .on_press(msg(ContextMsg::Isolate))
                .padding(4),
        ]
        .spacing(space::SM),
    );

    if let Some(status) = &form.status {
        col = col.push(match status {
            Ok(s) => kit::muted(s.clone()),
            Err(e) => text(e.clone())
                .size(font::CAPTION)
                .style(|theme: &iced::Theme| text::Style {
                    color: Some(colors(theme).danger()),
                })
                .into(),
        });
    }

    iced::widget::scrollable(col).height(Length::Fill).into()
}

/// RFC 09 §0.1, in the place it matters. Two switches, not one — and the
/// sentence that keeps somebody from misreading an isolated session as a dead
/// fleet.
fn scouting_help<'a>() -> Element<'a, Message> {
    Column::with_children(vec![
        kit::muted(
            "RFC 09 §0.1: multicast scouting and gossip are independent. This \
             toggle is the multicast half only — with it off, a peer still \
             learns about others through gossip over an established link.",
        ),
        kit::muted(
            "Off is the default on purpose: an explorer that multicast-scouts \
             joins whatever mesh it can reach. For isolated verification use \
             the preset — multicast off, explicit endpoints — and read an empty \
             result as \"nothing on these endpoints\", never as \"nothing on the \
             network\".",
        ),
    ])
    .spacing(2)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_split_on_anything_a_human_types() {
        assert_eq!(
            endpoints("tcp/a:1, tcp/b:2\ntcp/c:3  tcp/d:4"),
            ["tcp/a:1", "tcp/b:2", "tcp/c:3", "tcp/d:4"]
        );
        assert!(endpoints("   ").is_empty());
    }

    /// A typo is caught in the pane, not as a session that mysteriously
    /// reaches nothing.
    #[test]
    fn an_endpoint_without_a_protocol_is_refused() {
        let err = validate_endpoints(&["127.0.0.1:7447".into()]).unwrap_err();
        assert!(err.contains("<protocol>/<address>"), "{err}");
        assert!(validate_endpoints(&["tcp/127.0.0.1:7447".into()]).is_ok());
    }

    /// The editor round-trips a stored context, including the empty base —
    /// which is a real deployment, not an unset field (RFC v1.6).
    #[test]
    fn the_editor_round_trips_a_context_including_the_empty_base() {
        let stored = StoredContext {
            base: Some(String::new()),
            connect: vec!["tcp/127.0.0.1:7447".into()],
            listen: vec![],
            registry: vec![],
            scouting: Some(true),
            timeout: None,
        };
        let mut form = ContextForm::default();
        form.load_from("lab", &stored);
        assert_eq!(form.name, "lab");
        assert_eq!(form.base, "");
        assert!(form.scouting);

        let back = form.to_stored().unwrap();
        assert_eq!(back.base, Some(String::new()), "an empty base is stored");
        assert_eq!(back.connect, stored.connect);
        assert_eq!(back.scouting, Some(true));
    }

    #[test]
    fn a_nameless_context_is_refused() {
        let form = ContextForm {
            connect: "tcp/127.0.0.1:7447".into(),
            ..ContextForm::default()
        };
        assert!(form.to_stored().unwrap_err().contains("needs a name"));
    }
}
