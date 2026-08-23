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
    // A daemon, not an application (#186): the process owns N windows — the
    // main workspace plus a window per torn-off dock — so `view`, `title`,
    // `theme` and `scale_factor` are window-aware, and `boot` opens the
    // windows the preferences describe (a daemon opens none by itself).
    // Closing the main window exits explicitly (`update`'s `WindowClosed`
    // arm); a daemon that relied on "last window closed" would never stop.
    iced::daemon(
        move || Zengui::boot(settings.clone(), prefs.clone(), prefs_note.clone()),
        Zengui::update,
        Zengui::view,
    )
    .title(Zengui::title)
    .theme(Zengui::theme)
    .scale_factor(Zengui::scale_factor)
    .subscription(Zengui::subscription)
    .run()
}
