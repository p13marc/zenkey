//! A picture of every surface, in both themes (#532).
//!
//! Two tests over one scene list ([`common::scenes::ALL`]):
//!
//! * `every_scene_renders` runs everywhere: each scene builds, lays out, and
//!   shows its landmark. It keeps the scenes honest as the GUI moves, so a
//!   screenshot named "diagnose" is still a picture of the doctor.
//! * `shots` is `#[ignore]`d — it writes files. `just shots` runs it with
//!   `ICED_TEST_BACKEND=tiny-skia`, the CPU renderer, which never probes a
//!   GPU (so it also sidesteps #229) and draws the same pixels on every
//!   host. The PNGs land in `$SHOTS_DIR` (default `target/shots/current`),
//!   named `<scene>-<theme>-tiny-skia.png`, at twice the scene's logical
//!   size; `just shots-base <rev>` renders the same list from another
//!   revision, so a visual change is reviewed as a pair.
//!
//! The shots are not compared against stored images: a picture is for a
//! reviewer's eyes, and pixel equality across font rasterizer versions is
//! a flake with extra steps. What is asserted is what `panes.rs` asserts —
//! the words on screen.

mod common;

use std::path::PathBuf;

use common::scenes::{ALL, Scene};
use zengui::message::Message;
use zengui::prefs::ThemeChoice;

fn settings() -> iced::Settings {
    iced::Settings::default()
}

fn simulate(
    scene: &Scene,
    theme: ThemeChoice,
    f: impl FnOnce(&mut iced_test::Simulator<'_, Message>),
) {
    let app = (scene.build)(theme);
    // Any id renders the main workspace: a headless app opened no window,
    // and the main window is `view`'s fallback, not a named case.
    let mut ui = iced_test::Simulator::with_size(
        settings(),
        scene.size,
        app.view(iced::window::Id::unique()),
    );
    f(&mut ui);
}

#[test]
fn every_scene_renders() {
    for scene in ALL {
        for theme in ThemeChoice::ALL {
            simulate(scene, theme, |ui| {
                assert!(
                    ui.find(scene.landmark).is_ok(),
                    "scene `{}` ({}) lost its landmark {:?}",
                    scene.name,
                    theme.label(),
                    scene.landmark
                );
            });
        }
    }
}

#[test]
#[ignore = "writes PNGs — run through `just shots`"]
fn shots() {
    assert_eq!(
        std::env::var("ICED_TEST_BACKEND").as_deref(),
        Ok("tiny-skia"),
        "shots are drawn by the CPU renderer, so every host draws the same \
         pixels — run them through `just shots`"
    );
    let dir = std::env::var_os("SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/shots/current")
        });
    std::fs::create_dir_all(&dir).expect("shots dir");
    for scene in ALL {
        for theme in ThemeChoice::ALL {
            let stem = dir.join(format!("{}-{}", scene.name, theme.label()));
            // `matches_image` writes only when the file is missing, and
            // appends the renderer's name — so clear exactly that file.
            let _ = std::fs::remove_file(stem.with_file_name(format!(
                "{}-{}-tiny-skia.png",
                scene.name,
                theme.label()
            )));
            simulate(scene, theme, |ui| {
                let snap = ui.snapshot(&theme.theme()).expect("snapshot");
                assert!(snap.matches_image(&stem).expect("write png"));
            });
            println!("{}", stem.display());
        }
    }
}
