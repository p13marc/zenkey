# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Keeping this file small

This file is the index. Each crate's detail lives in the nested `CLAUDE.md`
of its directory (`zenkey-fleet/CLAUDE.md`, `spec/CLAUDE.md`, …), which
Claude Code loads when it reads files there. A change adds its detail to the
nested `CLAUDE.md` of the crate it touches, and adds to this index only when
a crate or a decision is new. This file stays under 30 KB.

## What this repo is

The **keyspace-v2 convention** for Zenoh keyspaces, in four parts:

- v1's **RFC set** (v1.50; ratified at v1.18, 2026-08-15) is **not on
  `main`** since #925: it is frozen on the `v1` branch's `rfcs/`, with its
  amendment ledger (`rfcs/CHANGELOG.md`) and `check-rfc-status.sh`. zk2's
  normative texts are `spec/core.md`, `spec/observer.md` and the profiles
  under `spec/profiles/`, each with its CHANGELOG; the tooling guide
  (`docs/zk2/tooling-guide.md`) is guidance, and its §8 holds the v1
  practices the reference tools keep (session posture, `$*`, matching
  status, router storages).
- `zenkey-model/` — zk2's **session-free half** (MIT, unpublished): keys,
  the authoring format and its lints, bundles, `compat`, the late joiner's
  merge, each profile's session-free half, the zk2c envelope, and the `zk2`
  contract CI. Detail: `zenkey-model/CLAUDE.md`.
- `zenkey/` — the **runtime crate**: on `main`, 0.20.0 is zk2's session
  half (MIT, unpublished): `ServiceBuilder`, presence, writers, state,
  consumers, operations and each profile's runtime. v1's 0.11.x stays on
  crates.io, from the `v1` branch. Detail: `zenkey/CLAUDE.md`.
- `zenkey-build/` — the **codegen crate**: on `main`, 0.20.0 is zk2's (MIT,
  unpublished): a consumer's build script lints its contracts, gates their
  history, embeds the bundles and generates one module per interface. v1's
  0.11.x is the `v1` branch's. Detail: `zenkey-build/CLAUDE.md`.
- `zenkey-fleet/` — the **fleet engine crate** (Apache-2.0; the in-tree
  0.20.0 is zk2's and `publish = false`, 0.18.x on crates.io is v1's): the
  shared core of zenctl, zengui and zenwatch, in five layers (`bus/`,
  `model/`, `judge/`, `report/`, `tape/`). Detail: `zenkey-fleet/CLAUDE.md`.
- `zenkey-explorer-config/` — the **shared explorer config crate**
  (Apache-2.0, **not published**): named connection contexts, the config
  path policy and the completion cache dir; it touches no bus. Detail:
  `zenkey-explorer-config/CLAUDE.md`.
- `zengui/` — the **graphical bus explorer** (Apache-2.0, **not published**;
  Forgejo release binaries), in Iced 0.14 over the fleet: a key-agnostic
  core with zk2 as overlay. `just shots` is how to see it on a host with no
  display. Detail: `zengui/CLAUDE.md`.
- `zenctl/` — the **bus explorer CLI** (Apache-2.0, **not published**:
  Forgejo release binaries; 0.1.x stays on crates.io), app-neutral and
  zk2's: nouns, wire verbs and exit-coded judgements over the fleet, its
  exit contract in `zenctl/src/exit.rs`. Detail: `zenctl/CLAUDE.md`.
- `zenwatch/` — the **notifier** (Apache-2.0, **not published**; Forgejo
  release binaries): one process, one JSON5 config, N rules, M sinks, over
  the fleet, and itself a zk2 service (`zenwatch.v1`, `health.v1`). Detail:
  `zenwatch/CLAUDE.md`.
- `spec/` — what every implementation must agree on: the normative
  `core.md` and `observer.md` and the profiles under `spec/profiles/`, each
  with its changelog, beside the generated `RULES.md`, the contract schema
  and the conformance fixtures. Detail: `spec/CLAUDE.md`.

**Strangler layout (#615), complete.** The v1 `zenkey/` and `zenkey-build/`
sources described above, and the `fixture-tests` crate with its
`registry/` and `registry-kdl/` corpus, live on the **`v1` branch**, which
releases 0.14.x patches. No member of `main` links v1: zenctl and the
in-tree fleet left it at FJ9 (#612), zenwatch at FL2–FL3 and zengui at
FL11b (#614), so `cargo tree -i zenkey@0.11.1` and `-i
zenkey-fleet@0.18.0` match no package, and `Cargo.lock` holds one
`zenkey` and one `zenkey-fleet`. zk2 reuses the crate names at 0.20.0
(`publish = false`), every member takes them by path, and the workspace
table pins none; plain `-p zenkey` and `-p zenkey-fleet` name them. The
`registry-conventions` skill describes the v1 registry format; its
fixtures are the `v1` branch's.

Graduated from the ZenSight monorepo in 2026-07; issue references like `#453`/`#475`
point at `p13marc/zensight` issues, `tcgui#43` at `p13marc/tcgui`.

## zk2 — the redesign in progress (epic #585)

- **The design of record** is `docs/zk2/architecture.md` (proposal r4, after the spike; contracts in `examples/zk2/`, authoring format draft 2).
  Its history (brief → r1 → review → r2) sits beside it, indexed by
  `docs/zk2/README.md`. Issues cite it as `r3 §x`; r3.x and r4 keep r3's
  section numbers. The r-number moves only through a reviewed revision carrying a
  "what changed since rN" table.
- **The v1 line.** v1 (its RFC set frozen at v1.50, in the `v1` branch's
  `rfcs/`, off `main` since #925) is maintained on the `v1` branch (0.14.x
  patches, #587).
- **`main` is the zk2 line, in the strangler layout (#615).** Every tool on
  it is ported, and none links v1's crates since #614 (FL11b). zk2 reuses
  the crate names at 0.20.0. `zenkey-model` is new.
- **Spike code** lives on branch `zk2-spike`, with its own `[workspace]`,
  and is never merged (tag `zk2-spike-final`). Only results and
  `docs/zk2/spike-report.md` land on `main`.
- **Decided 2026-10-07:**
  - paradigm P3: a key is written only by the service that owns it, and
    consumers bind roles to providers;
  - tcgui is the pilot;
  - adopter shapes are first-class: system = host (`hostid.v1`), and
    device-as-service.
- **Decided 2026-10-08 (r4):**
  - the owner is authoritative for state, and last-known state lives in
    `archive.v1`;
  - O1 holds only while one instance serves; `redundancy.v1` owns
    exclusivity;
  - the protobuf classifier uses WIRE semantics with renumber detection;
  - the draft upstream reports in `docs/zk2/upstream/` are not filed;
  - `spec/core.md` v0.1 is accepted; its current version is in its header
    and every amendment in `spec/CHANGELOG.md` (since #922 the header is
    the one place a version is stated), and `spec/observer.md`'s text
    version is in its own header, with `spec/observer-CHANGELOG.md` (#925;
    core 0.58 moved the reader's discipline there, no rule changed):
    - U22 is a deployment-configured tokenless set of interfaces (descriptor
      `"token": false`);
    - U23 is settled by measurement: a far site's router sits in a
      `gateway.south` region of the near router, with the `@zk` deny.

- **Decided 2026-10-08 (FJ, the tools' port, #612):**
  - zenctl's resolved verbs open a session in the deployment namespace
    (`--namespace`, `--base` an alias); raw verbs and the admin space stay
    un-namespaced;
  - `retire` is dropped; `pub` refuses zk2 data keys (exit 2) and still
    writes foreign keys;
  - profile-backed features (`config`, `blob`, `export`, the kind and budget
    checks, alerts) go dark on `main` until their profiles exist (#613); the
    `v1` branch keeps them. Config came back with #768, alerts with #800,
    the kind checks with telemetry.v1 (#819 TC), the budget as core 0.24;
  - no zenctl, zengui or zenwatch release from `main` until FJ9; fixes ship
    from `v1`.

- **Decided 2026-10-09 (FL, #614's in-repo half: zengui and zenwatch onto
  zk2, chunks FL1–FL12):**
  - `hostid.v1` and `health.v1` (#613's first tier) come before the port;
  - zenwatch ports with `alerts` dark until alarms.v1 (lit by #800 AD,
    2026-10-20). Inhibition is
    removed, not dark: it read v1's catalog, and no profile brings it back;
  - the population budget is core (§2.2, `cardinality`), lit up with a spec
    amendment (0.24, §2.7, #735). This reverses that part of FJ's "kind and
    budget go dark";
  - zengui moves pane by pane, beside v1's fleet under a renamed dependency
    (`zenkey-fleet-v1 = { package = "zenkey-fleet", version = "=0.18.0" }`),
    which FL11b removed with `zenkey-v1` once nothing read them;
    zenwatch requires `publish.address` until hostid.v1; contracts come by
    flag, never a named context (the config file is shared with the v1
    tools); no zengui or zenwatch release from `main` without the
    maintainer.

- **Decided 2026-10-29 (#938, epic #921 Track C): profiles by use.**
  - `spec/profiles/README.md`'s index has a **Used by** column: the
    adopter mappings (`examples/zk2/zensight/`, `zenoh-modem/`, `tcgui/`)
    and the shipped services (zenwatch) that use each profile, as a
    descriptor would list it (core §3.3). `zenkey-model/tests/profiles.rs`
    (`the_index_says_who_uses_each_profile`) computes it and fails when a
    row is stale, when the status disagrees with it, or when a row's text
    version, status or core version is not its text's header's;
  - `experimental` is a status beside `draft` and `accepted`: a profile no
    adopter's mapping and no shipped service uses, listed in the index's
    second table. `timing.v1` and `arbitration.v1` are experimental (the
    walkthrough alone uses them); their texts, runtimes and tools are
    unchanged. A profile leaves experimental when an adopter's mapping or a
    shipped service uses it, and a status moves without a text version;
  - no new tier before an adopter asks: #613's remaining profiles start
    only when an adopter's mapping needs one, `media.v1` first in line
    (`zs.parallax.v1` lists it in `uses`).

- **Decided 2026-10-29 (#937, epic #921 Track C): the scale machinery out
  of a newcomer's path.**
  - `cardinality = "unbounded"` is the authoring format's spelling of no
    ceiling (draft 2, core 0.57): `authoring::Cardinality`, resolved to
    4294967295 before the canonical form, so both spellings have one
    fingerprint (`ok-cardinality-unbounded` beside `ok-cardinality-no-ceiling`)
    and any other string is E000 (`e000-cardinality-string`). An omitted
    `cardinality` stays E013, never unbounded. `archive.v1`'s minimal
    contract writes `"unbounded"`;
  - the core's sections that matter only past some size open with
    "**At scale.** A deployment needs this … only once …; below that, …":
    §2.7, §3.3's `standby`, §8.1's tokenless set and member tokens, §8.3;
  - the runtime asks a newcomer nothing more: a templated service needs
    `ServiceBuilder::new`, `implement`, a writer and `start`
    (`zenkey/tests/budget.rs`,
    `a_templated_service_needs_nothing_beyond_its_contract`).

## Commands

Zero warnings is a CI gate. (The fixture-tests codegen round-trip runs on the
`v1` branch, where the corpus lives; see the strangler note above.)
CI's `test` job runs the suite with cargo-nextest (`.config/nextest.toml`):
a process per test, across binaries at once, because most of the suite waits
on real clocks. `cargo test --workspace` runs the same tests one binary at a
time; a test must not lean on sharing a process with another. A change only
under `impl/python/` skips `ci.yml` and runs `python.yml` (zk2py's
conformance) instead.

```bash
cargo run -p zenctl -- service list --namespace zensight -c tcp/127.0.0.1:7447
cargo run -p zenctl -- schema show zs.catalog.v1 --contracts examples/zk2/zensight   # no session
```

Plain cargo is still the build system; the `justfile` only covers what needs
more than one command — chiefly running the GUI against traffic to look at.
The demo is self-contained (no `zenohd`): `spray` listens and zengui connects
straight to it (`just --list` for the recipes). `just spray` and `just
gui-demo` run the zk2 spray.
`just shots` renders every zengui scene (`zengui/tests/common/scenes.rs`) to
PNGs in both themes headlessly (#532) — the only way to *see* the GUI on a
host with no display; `just shots-base main` draws the same scenes from `main`
for a before/after pair.

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

### zenctl: contract-parameterized, app-neutral

Contracts come from the bundles the bus serves (spec §8.4, retrieved by
fingerprint from their holders) or `--contracts <path>` (authoring files, a
directory, a `.history` root) — a revision held offline is never retrieved,
and every renderer is source-agnostic. A payload renders as its declared type
when a contract resolves one, else structurally (JSON / CBOR→JSON diagnostic
/ text / hex), with the rung where resolution stopped named. Bus discipline
(core §5.1, `zenkey-fleet/src/bus/query.rs`): every fleet GET goes through
`zenkey_fleet::fleet_get`
(target `All`, consolidation `None`, attribution by reply key). Silence is
never a verdict. Scouting is opt-in.

## Conventions

- Conventional commits (`feat:`/`fix:`/`docs:`/`chore:`), scope by crate
  (`feat(zenctl): …`, `docs(rfc): …`).
- Spec text is normative: when code and `spec/` disagree, either fix the code or
  amend the text explicitly (an entry in `spec/CHANGELOG.md`,
  `spec/observer-CHANGELOG.md` or the profile's CHANGELOG) — never silently
  drift. A new or changed rule in `spec/` carries no inline history, such
  as `(0.44, F-240)`: the version and the finding go in its changelog
  entry, and `scripts/check-spec-prose.py` fails on a tag that comes back
  (#935). Doc comments and report `cite` values cite zk2's texts (`core §1.4`,
  `observer §8.2`, `alarms.v1 §2.5`, `tooling guide O6`) and issues, never
  v1's RFC set: `scripts/check-v1-citations.sh` (in `just ci` and ci.yml)
  fails on `RFC 01`–`RFC 13` in any `.rs`, `.trycmd` or `tests/{cmd,fixtures}`
  file (#925). An IETF RFC (`RFC 3339`) is fine.
- Publishing (crates.io, LIB CRATES ONLY): `zenkey` → `zenkey-build` →
  `zenkey-fleet` (in that order; zenkey-build version-locks to zenkey 0.x).
  Binaries (zenctl, zengui, zenwatch) ship via the `release.yml` binary lane.
