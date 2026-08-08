//! zengui — the graphical bus explorer (issue #18).
//!
//! The GUI sibling of `zenctl`, over the same engine: everything that is not
//! presentation lives in `zenkey-fleet` (`docs/redesign-2026-07.md` §6.3 — "a
//! missing type is a zenkey-fleet issue, not a zengui workaround").
//!
//! **Key-agnostic by construction.** zengui is a useful explorer on *any*
//! Zenoh bus; the keyspace-v2 convention is an enrichment overlay that lights
//! up when a key parses, never a precondition. That is the grain of the engine
//! already: [`zenkey_fleet::KeyTreeSnapshot`] groups on a plain `split('/')`
//! with no grammar knowledge, and `zenkey::grammar::parse_full` returns
//! `Option` rather than `Result` because for an observer "does not parse" is an
//! answer, not an error.
//!
//! Like every explorer, the session is **un-namespaced** (RFC 09 §5): zengui
//! spells full wire keys and does its own base handling, which is what lets it
//! see traffic a namespaced application is blind to.

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,zenoh=warn".into()
            }),
        )
        .init();

    iced::application(Zengui::boot, Zengui::update, Zengui::view)
        .title("zengui")
        .run()
}

/// Placeholder shell. The real state lands with the connection flow.
#[derive(Default)]
struct Zengui;

#[derive(Debug, Clone)]
enum Message {}

impl Zengui {
    fn boot() -> (Self, iced::Task<Message>) {
        (Zengui, iced::Task::none())
    }

    fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {}
    }

    fn view(&self) -> iced::Element<'_, Message> {
        iced::widget::center(iced::widget::text("zengui").size(24.0)).into()
    }
}
