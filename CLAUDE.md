# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

The **keyspace-v2 convention** for Zenoh keyspaces, in four parts:

- `rfcs/` — the **normative RFC set** (v1.32; ratified at v1.18, 2026-08-15). Chapters 02–10
  and 13 (observer conformance, v1.24) are application-neutral; chapter 11 is the
  ZenSight reference profile. Wire-contract
  changes go through these RFCs, amendment-style (see `rfcs/CHANGELOG.md` —
  each amendment records what changed *and* what deliberately
  did not).
- `zenkey/` — the **runtime crate** (MIT, crates.io): typed key grammar, origin
  minting, `AppProfile`, slugs, QoS profiles, registry slices. Keys are built
  through types, never `format!`. **App-neutral: no bundled registry, no app
  constants.**
- `zenkey-build/` — the **codegen crate** (MIT, crates.io): consumers call
  `zenkey_build::Config::new().registry_dir("registry").generate()` from their
  build script; registry lints (RFC 08 §5) and the deprecation-ledger check fail
  the *consumer's* build. Generates `Subject`/`ProcedureId` enums, parsers,
  `AnySubject` dispatch, `REGISTRIES`, `is_registered_telemetry`, and — when
  the registry declares `[[blob]]` entries (v1.8) — an app-level `blob`
  module (deduped `Tier` enum across all declaring producers, typed key
  builders, the probe form).
- `zenkey-fleet/` — the **fleet engine crate** (Apache-2.0, crates.io): the
  shared core of zenctl and zengui — `fleet_get` (the RFC 05 §2.1
  chokepoint, moved verbatim from zenctl), `SliceSet`, the RFC 08 §7
  schema-decode pipeline (`SchemaStore`/`decode_sample`), `Monitor` with
  bounded broadcast + `Dropped(n)` honesty and ArcSwap key-tree snapshots.
  **Five layers**, and `lib.rs`'s doc-map is the normative statement of
  them: `bus/` (everything holding a session → observations), `model/`
  (values in hand → meaning; nothing here takes a session, which is what
  lets a `.zrec` replay through the same projections as live traffic),
  `judge/` (meaning → verdicts, plus `judge/common.rs` for the vocabulary
  the judges share), `report/` (every serde-pinned wire shape), `tape/`
  (`record`/`ingest`/`generate`/`synth`/`bench`). Placing a module is one
  ordered question — session? values in hand? says something is *wrong*?
  turns a stream into a recording? — and placing a **type** is not a module
  question at all: **every serde-pinned wire shape lives under `report/`,
  split by domain, with its pinned-shape test beside it; a type the wire
  never sees stays in the module that computes it.** That rule has no
  exceptions (the judgement core is under `report/` because
  `{"answer": "not_asked"}` reaches a script). `report/`'s domain files are
  private and re-exported flat, so `zenkey_fleet::report::Thing` stays the
  one path and the split can be re-cut without a call site moving.
- `zenkey-explorer-config/` — the **shared explorer config crate**
  (Apache-2.0, **not published**): named connection contexts, the path
  policy (`~/.config/zenkey-explorer/config.toml`, legacy zenctl read
  fallback, `ZENKEY_EXPLORER_CONFIG_DIR`), and the completion cache dir. One
  file, two explorers (issue #35). It is a sibling of the frontends, not a
  layer of the engine: it touches no bus, and living in `zenkey-fleet`
  forced `dirs` and `toml` onto every library consumer.
- `zengui/` — the **graphical bus explorer** (Apache-2.0, **not published**;
  Forgejo release binaries, like zenctl). The GUI sibling of zenctl over the
  same engine, in Iced 0.14. **Key-agnostic core, RFC as overlay**: it is a
  useful explorer on *any* Zenoh bus, and keyspace-v2 awareness lights up only
  when a key parses — `keyfacts.rs` is the single seam where that happens, and
  `scope.rs` the single place selectors are built (typed builders +
  `with_base`, never `format!`). Note `**` never crosses an `@`-chunk
  (RFC 03 §4 D2), so the raw scope is media-safe *and* cannot see `@catalog` —
  which is why the roster always names the service token explicitly.
  Deviation from `docs/redesign-2026-07.md` §15, deliberate: it lives here
  rather than in a separate `p13marc/zengui` repo.
  **Layout (#175)**: `app.rs` is the shell only — `new`/`update`/`view`/
  `subscription`. The state lives in `state/` as six sub-states named for what
  invalidates them; the handlers in `update/` (one module per message group,
  one per pane), where a signature is the exhaustive list of sub-states it can
  move; every bus call in `services/` as a free `fn -> Task<Message>`. Nothing
  but `update` takes `&mut Zengui`, and nothing under `view/` names it at all.
  The workspace holds **subject slots** (#181, #257, `message.rs`): each slot
  is one `Subject` — a key, a subtree prefix, an origin, or nothing — plus
  everything derived from it (`state/subject.rs`, `SubjectSlot`); slot 0
  (`SlotId::FOLLOW`) is the one the tree and location bar drive, and every
  further slot is a *pin*. The one bus tick fans out to every slot's recorder
  — a pin costs a bounded ring, never a second subscription — and the four
  slot sections (Detail, History, Fields, Why) carry a `SlotId` on their
  `PaneMsg` so two Inspectors never write into each other's state.
  `view/inspector.rs` is the one surface that dispatches on a subject
  (#182). Under `view/`, a `pane` returns an `Element`
  and owns its scroll; a `section` returns a `Column` and is a piece of one; a
  `dock` (`view/activity.rs`, #183) is a region holding the session's parallel
  streams — echo, the publish log, doctor verdicts, replay transport — with its
  own tab strip. The three virtualized lists (tree, timeline, echo) share
  `kit::window`. **Spacing (#192)** comes off the 8pt grid in `view/tokens.rs`
  by role (SM inside a card, MD between cards / dock padding, LG between
  sections, XL page-level); inside a dock it is a `Spacing` resolved once per
  dock (`panes::grid`) from the persisted density — Ctrl+Shift+D, Compact
  default in the Locator — which multiplies the grid and row heights, never a
  font size. Gated by `scripts/check-spacing.sh`, like the type scale (#191)
  and the interactive seam (#193). **Windows (#186)**: `main.rs` runs
  `iced::daemon`, so the process owns N windows and `view`/`title`/`theme`/
  `scale_factor` take a `window::Id`. Inspector, Activity and Workbench tear
  off into windows of their own (`WorkspaceMsg::TearOff`, rendered through
  the same free pane functions via `panes::solo`); the Locator never does —
  it *is* the navigation. A role has one home: a torn dock leaves the grid,
  every reveal path focuses its window, closing the window re-docks it, and
  closing the **main** window exits explicitly (a daemon never stops on its
  own). **A torn Inspector IS a pin (#257)**: tearing it off pins the current
  subject into a slot of its own (evidence carried whole), the docked
  Inspector goes on following the selection, and the one-home rule reads on
  the claim — reveal paths ask `WindowSet::follow_window_of`, so a pinned
  window is never focused as the selection's home; closing it unpins,
  dropping exactly its slot. Pins are session-only: only follow-bound torn
  windows persist, because a pin's evidence cannot (persisting the identity
  alone would be the freeze the issue rejects). The torn set and each
  window's geometry ride the named layout (`WorkspaceLayout::torn`); the
  replay locks are per-application and hold across every window.
- `zenctl/` — the **bus explorer CLI** (Apache-2.0, **not published**:
  Forgejo release binaries via `release.yml` / `cargo install --git`; 0.1.x
  stays on crates.io un-yanked): app-neutral; registry knowledge comes from the live bus
  (RFC 08 §6 introspection) or `--registry <dir>` TOMLs. `--base` resolves
  flag > env `ZENCTL_BASE` > the active named context
  (`zenctl context create …`, `~/.config/zenctl/config.toml`) > **empty**
  (the base-less bus-root deployment, the RFC v1.6 default).
  **Tree (#307)**, and the depth carries meaning: a **noun** is something
  declared, alive or persisted and gets verbs under it (`topic list|info`,
  `node`, `base`, `service`, `interface`, `schema show`, `registry`,
  `storage`, `blob list|locate|fetch`, `admin`, `key`, `bench rpc`); a **wire
  verb** is an act or observation on live traffic and hangs off the root
  (`get`, `echo`, `pub`, `retire`, `rate`, `field`, `record`, `replay`,
  `serve`, `gen`, `scout`); a **judgement** is exit-coded (`check
  expect|cutover|retired|probe|schema`, `doctor`, `why`, `watchdog`).
  **Flag vocabulary**: `--for` is every passive window (f64 seconds),
  `--timeout` is reply-wait only, `--duration` bounds generated output (`gen`
  alone), `--watch` is a bare bool with `--every` as the one period,
  `--count` is a stop bound (`--at-least` asserts, `--calls` sizes a bench,
  `--times` repeats a publish), `--from` names an input source, `--i-know` is
  one guard per verb. **Exit contract**, written once in `zenctl/src/exit.rs`
  and cited by every verb: 0 asked-and-clean, 1 asked-and-a-finding (`why`
  included — a cause is the finding), 2 no verdict — which covers clap's
  usage errors, everything else this tool refuses of your input
  (`exit::Unaskable`), silence under fan-out, and any pre-run failure of a
  verdict verb (`exit::asked`). Verdicts exit through
  `zenkey_fleet::judgement_exit_code` over the RFC 13 §1.2 `Judgement`, never
  a hand-rolled match. `zenctl/CHANGELOG.md` carries the old→new table.

Plus `fixture-tests/` (unpublished): the ZenSight registry snapshot compiled
through zenkey-build — the codegen regression corpus. **Do not add features
there**; it exists so a codegen change that breaks generated code fails here,
not downstream.

Graduated from the ZenSight monorepo in 2026-07; issue references like `#453`/`#475`
point at `p13marc/zensight` issues, `tcgui#43` at `p13marc/tcgui`.

## Commands

Zero warnings is a CI gate. `cargo test --workspace` includes fixture-tests
(the codegen round-trip).

```bash
cargo run -p zenctl -- node list --base zensight -c tcp/127.0.0.1:7447
cargo run -p zenctl -- topic list --base zensight --registry ../zensight/zensight-common/registry
```

Plain cargo is still the build system; the `justfile` only covers what needs
more than one command — chiefly running the GUI against traffic to look at.
The demo is self-contained (no `zenohd`): `spray` listens and zengui connects
straight to it (`just --list` for the recipes).

CI gates live in `.forgejo/workflows/ci.yml`; the release and publish lanes in
`release.yml` (tag push `X.Y.Z` — **bare, no `v`**, from 0.7.1 on; the
`v`-prefixed tags up to `v0.7.0` are history and were not retagged) and
`publish-crates.yml` (manual dispatch). Both also take a
`workflow_dispatch`; publishing skips any crate whose version is already on
crates.io, so a point release that moves one crate is a normal thing here.

## Architecture

### The grammar everything hangs off

```
v1/<origin>/<class>/<producer>/<subject...>     (base-relative)
```

Keys built by this crate are **base-relative** — they start at `v1`. The
deployment base is the Zenoh session **namespace**, added on egress and stripped
on ingress; there is deliberately **no base constant in zenkey** — application
code has no way to spell it. Only session config, router artifacts, and
un-namespaced debug tools (`zenctl`) ever see full keys
(`grammar::with_base`/`strip_base`/`parse_full`).

### Applications adopt via two seams

1. **`AppProfile`** (`zenkey/src/profile.rs`): `AppProfile::new(app, salt)` as a
   static — the app name (drives the host-id fallback path) and the RFC 06 §1
   origin salt. `V1Context::for_producer(&PROFILE, name)` builds all framework
   keys. Changing a salt re-keys the fleet; profiles are compile-time constants.
2. **Consumer-owned registry** (RFC 08 §5): `registry/*.toml` live in the
   application repo; `zenkey-build` generates the typed builders into the
   consumer's `OUT_DIR` (`include!` in e.g. their `src/registry.rs`). The
   `common = "..."` field on state subjects drives the generated
   `AnySubject::common_state()` (the RFC-defined framework set in
   `zenkey/src/common_state.rs`); app-specific groupings are wrappers the
   consumer writes over its own `AnySubject`.

### Where the detail lives

- `zenkey` crate module-by-module layering (each module ↔ its RFC section):
  `zenkey/CLAUDE.md`, loaded when working in the crate.
- Registry codegen and the registry-TOML conventions (RFC 08 §5 lints, the
  append-only `deprecated.lock` ledger, literal-vs-`{var}` leaves, the
  `common = "…"` framework set): the `registry-conventions` skill — read it
  before editing any `registry/*.toml` or touching zenkey-build's codegen.

### zenctl: source-parameterized, app-neutral

Registry slices come from the live bus (`zenkey_fleet::fleet_registry`, RFC 08 §6) or
`--registry <dir>` (offline TOMLs) — every renderer takes `&[RegistrySlice]`
and is source-agnostic. Payloads render generically (JSON / CBOR→JSON
diagnostic / text / hex, tagged with the slice-declared type). Bus discipline
(RFC 05, `zenkey-fleet/src/bus/query.rs`): every fleet GET goes through
`zenkey_fleet::fleet_get`
(target `All`, consolidation `None`, attribution by reply key). Silence is
never a verdict. Scouting is opt-in.

## Conventions

- Conventional commits (`feat:`/`fix:`/`docs:`/`chore:`), scope by crate
  (`feat(zenctl): …`, `docs(rfc): …`).
- RFC text is normative: when code and RFC disagree, either fix the code or amend
  the RFC explicitly (with a changelog entry in `rfcs/CHANGELOG.md`) — never silently
  drift. Doc comments cite RFC sections (`RFC 03 §2`) and issues; keep that habit.
- Publishing (crates.io, LIB CRATES ONLY): `zenkey` → `zenkey-build` →
  `zenkey-fleet` (in that order; zenkey-build version-locks to zenkey 0.x).
  Binaries (zenctl, zengui) ship via the `release.yml` binary lane.
