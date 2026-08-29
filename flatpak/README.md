# Flatpak packaging

`com.github.p13marc.ZenGui` — the `zengui` workspace member as a flatpak, on
freedesktop Platform/Sdk 25.08 with the rust-stable SDK extension
(fleet-standard manifest shape, mirroring tcgui/parallax/zensight).

Built by the `flatpak` job in `.forgejo/workflows/release.yml` on every
release tag: an unsigned export is force-pushed to the `flatpak-export`
branch (vm-edge signs and publishes it to https://flatpak.marcpardo.eu), and
a versioned `zenkey-zengui-<tag>.flatpak` bundle is attached to the release.

**CI builds with software rendering only** — the runner has no GPU. Iced's
wgpu renderer falls back to Mesa llvmpipe from the runtime (zengui's own CI
runs with `WGPU_BACKEND=gl` for the same reason); a "no suitable adapter"
symptom in a GPU-less environment is expected, not a bug.

Sandbox permissions: display (wayland + fallback-x11 + ipc + dri),
`--share=network` (zengui is a Zenoh *client* — unlike a pure-local GUI it
dials routers/peers at runtime), and the shared explorer XDG dirs:
`~/.config/zenkey-explorer/` (contexts `config.toml` shared with zenctl,
plus zengui's `zengui.toml` prefs), read-only legacy `~/.config/zenctl/`,
and the `~/.cache/zenkey-explorer/` completion/slice cache.

Local build (needs flatpak-builder + the 25.08 runtime/Sdk + rust-stable
extension):

    flatpak-builder --user --force-clean --install-deps-from=flathub \
      builddir flatpak/com.github.p13marc.ZenGui.yml
