//! The zengui binary. Everything of substance lives in the library crate so it
//! can be unit-tested and driven by `iced_test` without opening a window.

use clap::Parser;
use zengui::app::Zengui;
use zengui::config::Cli;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,zenoh=warn".into()
            }),
        )
        .init();

    let settings = match Cli::parse().settings() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zengui: {e:#}");
            std::process::exit(2);
        }
    };

    iced::application(
        move || Zengui::new(settings.clone()),
        Zengui::update,
        Zengui::view,
    )
    .title(Zengui::title)
    .theme(Zengui::theme)
    .subscription(Zengui::subscription)
    .run()
}
