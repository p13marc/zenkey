# zenkey-explorer-config

Claude Code loads this file when working in `zenkey-explorer-config/`; the root `CLAUDE.md` is the index.

- `zenkey-explorer-config/` — the **shared explorer config crate**
  (Apache-2.0, **not published**): named connection contexts, the path
  policy (`~/.config/zenkey-explorer/config.toml`, legacy zenctl read
  fallback, `ZENKEY_EXPLORER_CONFIG_DIR`), and the completion cache dir. One
  file, two explorers (issue #35). It is a sibling of the frontends, not a
  layer of the engine: it touches no bus, and living in `zenkey-fleet`
  forced `dirs` and `toml` onto every library consumer.
