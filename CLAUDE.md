# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

The **keyspace-v2 convention** for Zenoh keyspaces, in four parts:

- `rfcs/` — the **normative RFC set** (v1.18, ratified 2026-08-15). Chapters 02–10 are
  application-neutral; chapter 11 is the ZenSight reference profile. Wire-contract
  changes go through these RFCs, amendment-style (see the changelog in
  `rfcs/00-index.md` — each amendment records what changed *and* what deliberately
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
- `zenctl/` — the **bus explorer CLI** (Apache-2.0, **not published**:
  Forgejo release binaries via `release.yml` / `cargo install --git`; 0.1.x
  stays on crates.io un-yanked): app-neutral; registry knowledge comes from the live bus
  (RFC 08 §6 introspection) or `--registry <dir>` TOMLs. `--base` resolves
  flag > env `ZENCTL_BASE` > the active named context
  (`zenctl context create …`, `~/.config/zenctl/config.toml`) > **empty**
  (the base-less bus-root deployment, the RFC v1.6 default).

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
`release.yml` (tag push `vX.Y.Z`) and `publish-crates.yml` (manual dispatch).

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
(RFC 05, `zenkey-fleet/src/query.rs`): every fleet GET goes through
`zenkey_fleet::fleet_get`
(target `All`, consolidation `None`, attribution by reply key). Silence is
never a verdict. Scouting is opt-in.

## Conventions

- Conventional commits (`feat:`/`fix:`/`docs:`/`chore:`), scope by crate
  (`feat(zenctl): …`, `docs(rfc): …`).
- RFC text is normative: when code and RFC disagree, either fix the code or amend
  the RFC explicitly (with a changelog entry in `00-index.md`) — never silently
  drift. Doc comments cite RFC sections (`RFC 03 §2`) and issues; keep that habit.
- Publishing (crates.io, LIB CRATES ONLY): `zenkey` → `zenkey-build` →
  `zenkey-fleet` (in that order; zenkey-build version-locks to zenkey 0.x).
  Binaries (zenctl, zengui) ship via the `release.yml` binary lane.
