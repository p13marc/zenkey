# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

The **keyspace-v2 convention** for Zenoh keyspaces, in four parts:

- `rfcs/` — the **normative RFC set** (v1.8 proposed; v1.4 ratified). Chapters 02–10 are
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
  shared core of zenctl and the future zengui — `fleet_get` (the RFC 05 §2.1
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

```bash
cargo build --workspace --all-targets
cargo test --workspace                  # includes fixture-tests (codegen round-trip)
cargo test -p zenkey --test guard       # single integration-test file
cargo test -p zenkey slug::             # tests matching a module path
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # zero-warnings gate (CI)

cargo run -p zenctl -- node list --base zensight -c tcp/127.0.0.1:7447
cargo run -p zenctl -- topic list --base zensight --registry ../zensight/zensight-common/registry
```

Plain cargo is still the build system; the `justfile` only covers what needs
more than one command — chiefly running the GUI against traffic to look at.
The demo is self-contained (no `zenohd`): `spray` listens and zengui connects
straight to it.

```bash
just gui-demo               # zengui + generated conforming/foreign traffic
just gui-demo-bounded       # the same, with the key bound tripping immediately
just gui-demo-no-registry   # the same, with badges reading "not asked"
just spray                  # traffic only; then `just test-live` elsewhere
just ci                     # everything CI runs, in the same order
```

CI (`.forgejo/workflows/ci.yml`, Forgejo Actions) runs: fmt check, clippy
`-D warnings` (all features), build + test (workspace, plus `-p zenkey
--all-features`), an MSRV job (rustc pinned), and rustdoc with `-D warnings`.
`release.yml` (tag push `vX.Y.Z`) builds the zenctl binary + source tarball as
a Forgejo release; `publish-crates.yml` (manual dispatch) publishes the lib
crates to crates.io.

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

### zenkey crate layers (each module maps to an RFC section)

- `grammar` — chunk charset, reserved tokens, structural assembly/parse
  (RFC 03), plus the wire-observer helpers (`parse_full`, `fleet_rpc_key`,
  `service_rpc_key`, `all_liveliness_wildcard`, `service_alive_key`).
- `origin` — `h-<12hex>` minting from machine-id + app salt (RFC 06 §1);
  `profile` supplies the salt and fallback persistence.
- `slug` — canonical, injective slugging of foreign values, `_xNN_` escape (RFC 03 §2).
- `qos` — the five named QoS profiles as a closed enum (RFC 04 §3); the zenoh
  mappings sit behind the default `zenoh` feature (zenkey-build depends on
  zenkey with `default-features = false`).
- `context` — `V1Context` bundles origin + producer; producers build all keys
  through it.
- `slice` — `RegistrySlice`, the `introspect` reply type + diff (RFC 08 §6),
  including per-producer `[[blob]]` tier declarations (v1.8). Optional
  metadata fields (qos/ttl/unit/rate/cardinality) must stay **optional** —
  forward-compat is pinned by zenctl's foreign-slice test.
- `tests/guard.rs` — RFC 03 §4 design properties D1–D6 pinned as executable
  tests. If a grammar change breaks these, the change is wrong (or needs an RFC
  amendment).

### Registry codegen (zenkey-build — the load-bearing piece)

`Config::generate()` reads every `registry/*.toml` in the consumer's dir,
**lints it against RFC 08 §5** (returned as `Error`, surfaced by the consumer's
build.rs `unwrap()`), checks the append-only `deprecated.lock` ledger, and
emits the module. Codegen is normative in **both** directions (RFC 08 §1):
an unregistered subject does not construct; a metric name refines into a typed
subject with named variables (never `parts[1]`). Consumers that cannot parse a
subject **drop it** — no string-parsing fallback.

Registry conventions to preserve when editing registry TOMLs (fixtures here,
live files in the application repos):

- Telemetry is registered as **real subject families**, not a `{metric...}`
  catch-all. Exception: device-defined trees (snmp/modbus/gnmi/netflow style)
  keep a trailing rest-var by design.
- A leaf naming a distinct measurement is a **literal**; a leaf that is a value
  of a dimension is a **`{var}`**.
- `deprecated.lock` is an **append-only ledger**: one `<producer>\t<path>` line
  per retired subject, never removed; codegen fails if ledger and
  `[[deprecated]]` entries disagree.
- `common = "health|errors|sensor|alert|evidence_self|evidence_device|evidence_names|entity|alias|pdns"`
  marks a state subject as one of the RFC framework set; the lint checks the
  pattern's variable names against the CommonState variant fields.

### zenctl: source-parameterized, app-neutral

Registry slices come from the live bus (`bus::fleet_registry`, RFC 08 §6) or
`--registry <dir>` (offline TOMLs) — every renderer takes `&[RegistrySlice]`
and is source-agnostic. Payloads render generically (JSON / CBOR→JSON
diagnostic / text / hex, tagged with the slice-declared type). Bus discipline
(RFC 05, `zenctl/src/bus.rs`): every fleet GET goes through `bus::fleet_get`
(target `All`, consolidation `None`, attribution by reply key). Silence is
never a verdict. Scouting is opt-in.

## Conventions

- Conventional commits (`feat:`/`fix:`/`docs:`/`chore:`), scope by crate
  (`feat(zenctl): …`, `docs(rfc): …`).
- RFC text is normative: when code and RFC disagree, either fix the code or amend
  the RFC explicitly (with a changelog entry in `00-index.md`) — never silently
  drift. Doc comments cite RFC sections (`RFC 03 §2`) and issues; keep that habit.
- Rust edition 2024, Zenoh 1.9 (`unstable` feature), tokio.
- Publishing (crates.io, LIB CRATES ONLY): `zenkey` → `zenkey-build` →
  `zenkey-fleet` (in that order; zenkey-build version-locks to zenkey 0.x).
  Binaries (zenctl, zengui) ship via the `release.yml` binary lane.
