//! The zengui binary. Everything of substance lives in the library crate so it
//! can be unit-tested and driven by `iced_test` without opening a window.

use clap::Parser;
use zengui::app::Zengui;
use zengui::config::Cli;
use zengui::prefs::Prefs;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "info,wgpu_core=warn,wgpu_hal=warn,naga=warn,zenoh=warn".into()
            }),
        )
        .init();

    // Preferences are read once, here, so the app can be constructed from an
    // injected set in tests (issue #73). A note about an unreadable file rides
    // into the window rather than to a terminal nobody is watching.
    //
    // Before the settings, not after: the remembered context is one of the
    // defaults `settings()` resolves over, so it has to exist first (#189).
    let (prefs, prefs_note) = Prefs::load();
    if let Some(note) = &prefs_note {
        tracing::warn!("{note}");
    }

    let settings = match Cli::parse().settings(&prefs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zengui: {e:#}");
            std::process::exit(2);
        }
    };
    let window = iced::window::Settings {
        size: prefs
            .window
            .map(|(w, h)| iced::Size::new(w, h))
            .unwrap_or(iced::Size::new(1280.0, 800.0)),
        ..Default::default()
    };

    iced::application(
        move || Zengui::with_prefs(settings.clone(), prefs.clone(), prefs_note.clone()),
        Zengui::update,
        Zengui::view,
    )
    .title(Zengui::title)
    .theme(Zengui::theme)
    .scale_factor(Zengui::scale_factor)
    .subscription(Zengui::subscription)
    .window(window)
    .run()
}
