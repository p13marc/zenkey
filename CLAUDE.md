# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repo is

The **keyspace-v2 convention** for Zenoh keyspaces, in three parts:

- `rfcs/` — the **normative RFC set** (v1.4, ratified). Chapters 02–10 are
  application-neutral; chapter 11 is the ZenSight reference profile. Wire-contract
  changes go through these RFCs, amendment-style (see the changelog in
  `rfcs/00-index.md` — each amendment records what changed *and* what deliberately
  did not).
- `zenkey/` — the **enforcement crate** (MIT): typed key grammar, origin minting,
  slugs, QoS profiles, and registry codegen. Keys are built through types, never
  `format!` — an invalid or unregistered key does not construct.
- `zenctl/` — the **bus explorer CLI** (Apache-2.0): `busctl`/`d-feet` equivalent.

Graduated from the ZenSight monorepo in 2026-07; issue references like `#453`/`#475`
point at `p13marc/zensight` issues, `tcgui#43` at `p13marc/tcgui`.

## Commands

```bash
cargo build --workspace --all-targets
cargo test --workspace                  # includes zenkey/tests/{guard,registry,adv_token}.rs
cargo test -p zenkey --test guard       # single integration-test file
cargo test -p zenkey slug::            # tests matching a module path
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # zero-warnings gate (CI)

cargo run -p zenctl -- node list -c tcp/127.0.0.1:7447  # on-bus commands need a running fleet
cargo run -p zenctl -- topic list                        # offline commands need nothing
```

CI (`.github/workflows/ci.yml`) runs exactly: build, test, fmt check, clippy `-D warnings`.

## Architecture

### The grammar everything hangs off

```
v1/<origin>/<class>/<producer>/<subject...>     (base-relative)
```

Keys built by this crate are **base-relative** — they start at `v1`. The deployment
base (`DEFAULT_BASE = "zensight"`) is the Zenoh session **namespace**, added on
egress and stripped on ingress; application code deliberately has no way to spell
it. Only session config, router artifacts, and un-namespaced debug tools (`zenctl`)
ever see full keys.

### zenkey crate layers (each module maps to an RFC section)

- `grammar` — chunk charset, reserved tokens, structural assembly/parse (RFC 03).
  Typed builders: `Origin`, `Class`, `Plane`, `Producer`, `StructuralKey`.
- `origin` — `h-<12hex>` host-id minting from `/etc/machine-id` + app salt, with
  persisted-random fallback (RFC 06 §1).
- `slug` — canonical, injective slugging of foreign values, `_xNN_` escape (RFC 03 §2).
- `qos` — the five named QoS profiles as a closed enum (RFC 04 §3).
- `context` — `V1Context` bundles origin + producer; sensors build all keys through it.
- `slice` — `RegistrySlice`, the `introspect` reply type + diff (RFC 08 §6). A
  disagreement between a served slice and ours is a *finding*, not an ambiguity.
- `tests/guard.rs` — RFC 03 §4 design properties D1–D6 pinned as executable tests.
  If a grammar change breaks these, the change is wrong (or needs an RFC amendment).

### Registry codegen (build.rs — the load-bearing piece)

`zenkey/build.rs` reads every `registry/*.toml`, **lints it against RFC 08 §5 as
build errors**, and generates `$OUT_DIR/registry_gen.rs`: per producer, a `Subject`
enum with typed constructors and a precedence-ordered parser, a `ProcedureId` enum
with `@rpc` key builders, and the raw slice served by `introspect`. Codegen is
normative in **both** directions:

- **Build**: an unregistered subject does not construct.
- **Parse**: a metric name refines into a typed subject with named variables
  (`Subject::DiskUsed { mount }`, never `parts[1]`). Consumers that cannot parse a
  subject **drop it** — there is deliberately no string-parsing fallback; gaps are
  made loud at publish time instead.

Registry conventions to preserve when editing `registry/*.toml`:

- Telemetry is registered as **real subject families**, not a `{metric...}`
  catch-all (a catch-all makes the §5 lint vacuous). Exception: `snmp`/`modbus`/
  `gnmi`/`netflow` keep a trailing rest-var *by design* — their metric tree belongs
  to the polled device.
- A leaf naming a distinct measurement is a **literal**; a leaf that is a value of
  a dimension is a **`{var}`**. `sysinfo.toml` is the reference file.
- `deprecated.lock` is an **append-only ledger**: one `<producer>\t<path>` line per
  retired subject, never removed. build.rs fails if the ledger and the
  `[[deprecated]]` TOML entries disagree.
- The crate currently bundles ZenSight's registry as its compiled-in default;
  making it fully consumer-supplied is planned work.

### zenctl: two halves, kept visibly apart

**Offline** commands (`topic list/info`, `service list`, `interface list/show`)
answer from the compiled-in registry — what *may* exist. **On-bus** commands
(`node list`, `topic echo`, `service call`, `doctor`) ask the live fleet — what
*does* exist. The gap between them is drift, and `doctor` reports it. Do not blend
the halves.

Bus discipline (RFC 05, encoded in `zenctl/src/bus.rs`): every fleet GET goes
through `bus::fleet_get`, which sets query target `All`, consolidation `None`, and
attributes replies by the reply's own concrete key. Silence is never a verdict;
errors ride `reply_err`, never a value reply. Scouting is opt-in (`--scouting`).

### Cross-repo circularity (why the `[patch]` exists)

`zenctl` depends on `zensight-common` (git, from the zensight repo), whose own
`zenkey` dependency is *this repo* (git). The root `Cargo.toml` patches that git
source to the local path so exactly one copy of `zenkey` exists — types cross the
`zensight-common` API, so two copies would mismatch. Consequences:

- Changes here are **not** seen by a zensight build until pushed (zensight consumes
  zenkey as a git dep).
- Changes to `zensight-common`'s API can break `zenctl` here; `zenctl` builds
  against zensight's `master` branch.

## Conventions

- Conventional commits (`feat:`/`fix:`/`docs:`/`chore:`), scope by crate
  (`feat(zenctl): …`, `docs(rfc): …`).
- RFC text is normative: when code and RFC disagree, either fix the code or amend
  the RFC explicitly (with a changelog entry in `00-index.md`) — never silently
  drift. Doc comments cite RFC sections (`RFC 03 §2`) and issues; keep that habit.
- Rust edition 2024, Zenoh 1.9 (`unstable` feature), tokio.
