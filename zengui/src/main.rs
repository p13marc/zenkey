//! The zengui binary. Everything of substance lives in the library crate so it
//! can be unit-tested and driven by `iced_test` without opening a window.

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
