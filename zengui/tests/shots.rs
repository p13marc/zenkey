//! A picture of every surface, in both themes (#532).
//!
//! Two tests over one scene list ([`common::scenes::ALL`]):
//!
//! * `every_scene_renders` runs everywhere: each scene builds, lays out, and
//!   shows its landmark. It keeps the scenes honest as the GUI moves, so a
//!   screenshot named "diagnose" is still a picture of the doctor.
//! * `shots` is `#[ignore]`d — it writes files. `just shots` runs it with
//!   `ICED_TEST_BACKEND=wgpu` over `WGPU_BACKEND=gl` — the renderer the app
//!   itself uses, on mesa's software GL where there is no GPU. Not
//!   tiny-skia, though it needs no GPU at all: iced_test 0.14's tiny-skia
//!   path drops a canvas's fills and strokes and draws its text at the
//!   wrong offset, so the mesh and every sparkline came out blank (#533) —
//!   a screenshot that silently omits the charts is worse than none. The
//!   PNGs land in `$SHOTS_DIR` (default `target/shots/current`), named
//!   `<scene>-<theme>-wgpu.png`, at twice the scene's logical size;
//!   `just shots-base <rev>` renders the same list from another revision,
//!   so a visual change is reviewed as a pair.
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

/// The app's own renderer settings — its bundled faces (#533) — loaded into
/// the process's font system once: `Simulator::with_size` loads whatever
/// `fonts` it is handed on every call, and 48 scenes would load each face
/// 48 times.
fn settings() -> iced::Settings {
    static LOADED: std::sync::Once = std::sync::Once::new();
    let mut settings = zengui::view::fonts::settings();
    let mut first = false;
    LOADED.call_once(|| first = true);
    if !first {
        settings.fonts.clear();
    }
    settings
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
        Ok("wgpu"),
        "shots are drawn by the app's own renderer (tiny-skia drops canvases) \
         — run them through `just shots`"
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
                "{}-{}-wgpu.png",
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
