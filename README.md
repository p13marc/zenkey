# zenkey — zk2, typed interface contracts over Zenoh

**zk2** is a small, typed and introspectable interface-contract layer over
[Zenoh](https://zenoh.io). A service, at an address `<system>/<service>`,
implements interfaces. Each interface major is a contract: a TOML file
that declares its resources (streams, state, events and operations), their
types and their QoS, and that has a fingerprint, so every revision has one
name. Every key a service writes follows one grammar,
`zk2/<system>/<service>/<iface>.v<major>/<kind>/<resource…>`, and only the
service that owns a key writes it. A consumer binds the roles it requires
to the providers its deployment names. Every running instance announces
itself with liveliness tokens and serves a descriptor of what it
implements, so a tool can resolve any key it sees to its service,
interface, contract and type. The rules are a language-neutral
specification in [`spec/`](spec/README.md). This repository holds the
Rust reference implementation, the tools built on it, and a second
implementation in Python.

## Start here

Two pages are coming, and a newcomer will start from them: the
**quickstart**, with the first runnable example
([#923](https://git.marcpardo.eu/marcpardo/zenkey/issues/923)), and the
**user guide**, every concept a user meets in plain words, with how-tos
([#933](https://git.marcpardo.eu/marcpardo/zenkey/issues/933)). Until
they land:

- **A worked example:** [Adopting zk2](#adopting-zk2), below, and the
  walkthrough's contracts in [`examples/zk2/walkthrough/`](examples/zk2/walkthrough/),
  narrated in [`docs/zk2/architecture.md`](docs/zk2/architecture.md) §4.
- **Writing a contract:** the authoring format,
  [`examples/zk2/README.md`](examples/zk2/README.md).
- **What the code generator gives you:** [`docs/zk2/codegen.md`](docs/zk2/codegen.md).
- **What the tools owe the person reading them:** the rules in
  [`spec/observer.md`](spec/observer.md), and how the reference tools
  meet them in [`docs/zk2/tooling-guide.md`](docs/zk2/tooling-guide.md).
- **Coming from v1:** [`docs/zk2/migration.md`](docs/zk2/migration.md).
- **The rules, for implementers:** [`spec/README.md`](spec/README.md): the
  core, the [profiles](spec/profiles/README.md#index), the schemas, the
  fixtures and the scenarios.
- **The design and its reasons:** [`docs/zk2/README.md`](docs/zk2/README.md),
  the design record and the spike that measured it.

## Components

The zk2 crates are unpublished (`publish = false`). Publishing waits on
the maintainer's call ([#585](https://git.marcpardo.eu/marcpardo/zenkey/issues/585),
[#929](https://git.marcpardo.eu/marcpardo/zenkey/issues/929)), so a port
builds against `main` from source. The crates on crates.io under the same
names are v1's ([below](#v1)).

| Part | What it is |
|------|-----------|
| [`spec/`](spec/README.md) | The **zk2 specification**, for implementers: the normative core ([`core.md`](spec/core.md): its version is in its header, and every amendment in [`spec/CHANGELOG.md`](spec/CHANGELOG.md)), the observer rules a tool and any reader follow ([`observer.md`](spec/observer.md), versioned the same way in its header and [`spec/observer-CHANGELOG.md`](spec/observer-CHANGELOG.md)), the profiles, each a specification of its own built on the core (listed in [the profiles index](spec/profiles/README.md#index)), the contract and descriptor schemas, conformance fixtures and network scenarios. It is written so that any language can implement it from `spec/` alone. |
| [`zenkey-model/`](zenkey-model/) | The **session-free half of zk2**: the `zk2/` key grammar and its kind selectors, resource templates (most-literal-first) and slugs, the contract authoring format (draft 2) with stable-coded lints, the canonical form, fingerprints, bundles and the append-only `.history`, the compatibility classifier, a late joiner's merge, and the session-free half of each profile. It depends on no zenoh crate but `zenoh-keyexpr`. Its `zk2` binary is the contract CI (`zk2 contract lint\|fingerprint\|bundle\|compat\|check-history`). It loads every contract in [`examples/zk2/`](examples/zk2/), generates `spec/`'s schemas and runs `spec/`'s fixtures. |
| [`zenkey/`](zenkey/) | The **zk2 runtime**: `ServiceBuilder` brings a service up in the spec's order (resources, then the descriptor and bundles, then the tokens), typed writers apply the contract's QoS and stamp state, consumers read through a role's bindings, and operations come with their clients and their fan-out rules; presence, contract retrieval, a minimal archive, and the runtime half of each profile that has one. What needs no session is `zenkey-model`'s, re-exported as `zenkey::model`; without its default `zenoh` feature the crate builds with no zenoh, so a contract crate's traits do too. `examples/owner.rs` is a service to interoperate with, driven from stdin. |
| [`zenkey-build/`](zenkey-build/) | **zk2's codegen**: from an application's build script it lints the contracts, gates each against its `.history` (a breaking revision fails the build), embeds the bundles, and generates one module per interface, with its types, a `Server` and a `Consumer`, and `Handlers`, `Client` and `Api` for its operations ([design](docs/zk2/codegen.md)). |
| [`zenkey-fleet/`](zenkey-fleet/) | The **fleet engine**, the shared core of `zenctl`, `zengui` and `zenwatch`: disciplined fan-in queries, presence, contract retrieval, the lens a raw observer resolves keys through, contract-resolved rendering, the live key-tree monitor, the readers and judges behind every judgement the tools make, and recording and replay. |
| [`zenctl/`](zenctl/) | The **bus explorer CLI**, the `busctl`/`d-feet` equivalent for a zk2 deployment, and useful on any Zenoh bus. **Nouns:** `service list/show`, `iface list/show`, `schema show`, `namespace list`, `storage list/gen`, `acl gen` (spec §11's grants, from contracts and an enrollment; with `--face-policy`, a constrained face's denies, `link.v1`), `admin routers/graph` (the mesh, `--dot` for Graphviz), `key includes/intersects/canon`, `hostid`, `config get/set/confirm/cancel/extend/persist` (a running service's configuration, `config.v1`), `bench call`, `context`. **Wire verbs:** `get` (any selector, fleet-disciplined) and `get state` (the owner's answer, or an archive's last-known, never shown as current), `call` (an operation through its contract; a fan-out only where the contract allows one), `watch` (a resource, decoded through its contract), `echo`/`pub`/`rate`/`field` (+ `pub --from ndjson`; `pub` refuses a key a zk2 service owns), `record`/`replay` (`.zrec` capture with an in-file drop ledger; `record --on <rule> --pre 30` captures the thirty seconds before a condition fired; `replay --namespace` stands in for the owners it recorded), `snapshot` (+ `snapshot diff`, aligned by key), `timeline` (a lane per address and resource, on a stated clock), `graph` (the binding graph, from descriptors), `serve` and `gen` (mock owners), `scout`. **Judgements**, with one exit contract (0 clean, 1 a finding, 2 no verdict): `doctor`, `health` (`health.v1`), `alarms` (`alarms.v1`), `telemetry` (`telemetry.v1`), `timing` (`timing.v1`), `desired` (`desired.v1`), `redundancy` (`redundancy.v1`), `arbitration` (`arbitration.v1`), `why`, `check expect/probe/schema/conform/face` (`check face`: a router's access-control block against a `link.v1` face policy), `watchdog`, and `compat` between two contract revisions. Every key resolves through presence and the contracts (the bundles the bus serves, or `--contracts <PATH>`) in the deployment's `--namespace` (alias `--base`); a payload renders as its declared type, else structurally, naming where resolution stopped. Attachments on echo/pub, `--zenoh-config` passthrough for secured buses, shell completions, `--format json/ndjson`. v1's spellings and where each went: [`zenctl/CHANGELOG.md`](zenctl/CHANGELOG.md). Ships as Forgejo release binaries for **linux x86_64 only** (the one platform this project stands behind — zenkey #212); every other architecture and OS builds from source with `cargo install --git`. The zk2 build is not released yet; the `v1` branch releases the v1 build as 0.14.x. Not published to crates.io (0.1.x remains there un-yanked). |
| [`zengui/`](zengui/) | The **graphical bus explorer** (Iced): a live key tree, built from the contracts' declared keyspace and what is observed, with per-key counts, rates and byte totals; a filtered echo pane; the Inspector, which reads a value by what its key is (the owner's state, an archive's last-known behind a button, a stream's history) and shows a metric's kind, unit and rate (`telemetry.v1`) and a timed stream's deadline and period (`timing.v1`); and the Workbench: the nodes per address with their health, redundancy roles (`redundancy.v1`) and arbitration (`arbitration.v1`), the doctor, the admin pane, the mesh, publish and call, and the Config tool (`config.v1`). A `.zrec` capture replays through the same lens, and panes tear off into windows of their own. **Key-agnostic**: it is useful on *any* Zenoh bus, and zk2 awareness is an overlay: a key resolved through presence and the contracts gains its address, interface and resource, a contract-typed decode and a conformance badge; any other key says where resolution stopped. Non-conforming keys are first-class, not errors. Same distribution as `zenctl`. |
| [`zenwatch/`](zenwatch/) | The **notifier**: one process, one JSON5 config, N rules, M sinks (`ntfy`, `smtp`, `webhook`, `exec`). A rule is one spelling of a closed vocabulary: `zenctl watchdog`'s (rates, silence, invalid payloads, QoS mismatches, the doctor, `alert-firing`, drops), plus `instance-gone <SYSTEM/SERVICE>` (an address gone, read from presence; it replaces v1's `origin-down`), `liveliness-gone <SEL>` (the dead-man's switch) and `alerts <SEL>` (each `alarms.v1` alarm firing, resolved or unobservable on its own); a `doctor` block runs the doctor every few hours and notifies what changed. Three states ride into every sink payload — `unobservable` is never folded into `ok` — and secrets are env-var names or file paths, never inline. `check-config` refuses what `run` would refuse; `--dry-run` prints what it would send and sends nothing. A running daemon is a zk2 service of its own (`health.v1` and `zenwatch.v1`). v1's spellings and where each went: [`zenwatch/CHANGELOG.md`](zenwatch/CHANGELOG.md). Same distribution as `zenctl`. |
| [`zenkey-explorer-config/`](zenkey-explorer-config/) | The **shared explorer config**: named connection contexts, the config path policy and the completion cache dir — one file, two explorers. A sibling of the frontends, not a layer of the engine: it touches no bus, and living in `zenkey-fleet` forced `dirs` and `toml` onto every library consumer. Not published. |
| [`impl/python/`](impl/python/README.md) | **zk2py**, the second implementation, written from `spec/` and `examples/zk2/` alone, never from the Rust: it tests the spec's claim that any language can implement it, and the gaps it finds are recorded in its `SPEC-FINDINGS.md`. |

(`report-fixtures/` and `zenkey-build/fixtures/` are test crates: the
report values the fleet's and zenctl's tests share, and two crates built
from generated code. `fixture-tests/`, the ZenSight v1 registry that v1's
tests read and its codegen-corpus crate, lives on the `v1` branch.)

## Why a convention and not just keys?

Because the key layout is what turns a *pipe* into a *data platform*: a fixed
system, service, interface and kind position is what makes per-service ACLs,
storage selection by kind, and router-pinned QoS expressible at all. If every
subsystem invents its own layout, none of those can be written down.

## Adopting zk2

An application adopts zk2 through a contract, `zenkey-build` and the
runtime's `ServiceBuilder`. The crates are unpublished, so a port builds
against `main` from source.

1. **A contract** per interface major, in the authoring format
   ([`examples/zk2/README.md`](examples/zk2/README.md)), checked by the
   contract CI:

   ```bash
   cargo run -p zenkey-model --bin zk2 -- contract lint examples/zk2/walkthrough/thruster.v1.toml
   ```

2. **`zenkey-build`** in the build script lints the contracts, gates them
   against their `.history`, and generates a typed module per interface:

   ```rust
   // build.rs
   zenkey_build::Config::new()
       .contracts_dir("contracts")
       .history_dir("contracts/.history")
       .generate()
       .unwrap_or_else(|e| panic!("{e}"));
   ```

   ```rust
   // src/zk2.rs
   include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
   ```

3. **A service**, configured rather than coded, and brought up by
   `ServiceBuilder`:

   ```rust
   use std::sync::Arc;
   use zenkey::{ServiceBuilder, ServiceConfig};
   use crate::zk2::thruster_v1;

   // A literal address; `ServiceConfig::minted` asks hostid.v1 for the system.
   let config = ServiceConfig::new("vehicle-01/thruster-l".parse()?)
       .bind("cmd", &["vehicle-01/teleop"]);
   let mut b = ServiceBuilder::new(&session, config);
   // `MyThruster` implements `thruster_v1::Handlers`: `arm` and `disarm`.
   let thruster = thruster_v1::Server::declare(&mut b, Arc::new(MyThruster)).await?;
   let _service = b.start().await?; // spec §8.2: resources, descriptor and bundles, then tokens
   thruster
       .telemetry
       .put(&thruster_v1::types::Telemetry { rpm: 1200.0, ..Default::default() })
       .await?;
   ```

The walkthrough is the worked example: its contracts are in
[`examples/zk2/walkthrough/`](examples/zk2/walkthrough/), narrated in
[`docs/zk2/architecture.md`](docs/zk2/architecture.md) §4, and generated in
`zenkey-build/fixtures/codegen-test`. [`docs/zk2/codegen.md`](docs/zk2/codegen.md)
says what is generated. A team on v1 starts from
[`docs/zk2/migration.md`](docs/zk2/migration.md).

## Status

zk2 is built on `main` and unpublished. Its design record is in
[`docs/zk2/`](docs/zk2/README.md) (epic
[#585](https://git.marcpardo.eu/marcpardo/zenkey/issues/585)). No adopter
runs it yet: ZenSight, tcgui and zenoh-modem are mapped onto it in
[`examples/zk2/`](examples/zk2/README.md), and
[`docs/zk2/migration.md`](docs/zk2/migration.md) says what a port
involves and what zk2 does not have yet.

## v1

v1, the keyspace-v2 convention (`<base>/v1/<origin>/<class>/<producer>/<subject...>`),
is frozen: errata only. The convention is at **v1.50 (ratified at v1.18, 2026-08-15)**,
and is maintained on the [`v1` branch](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/v1)
as 0.14.x patch releases. crates.io's `zenkey` and `zenkey-build` 0.11.x
and `zenkey-fleet` 0.18.x are v1's, and existing adopters
([ZenSight](https://github.com/p13marc/zensight), whose reference profile
is the RFCs' chapter 11, and [tcgui](https://github.com/p13marc/tcgui))
stay on them until they port. Its frozen RFC set, with its amendment ledger,
is in the `v1` branch's [`rfcs/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/v1/rfcs) (#925 took it off `main`). The v1 adoption
path (`AppProfile`, a `registry/` and v1's `zenkey-build`) is in its
[crate README](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/v1/zenkey/README.md).
This repository graduated from the ZenSight monorepo in 2026-07, with its
history.

## License

`zenkey`, `zenkey-build`, `zenkey-model` (libs): MIT. `zenkey-fleet` (lib),
`zenctl` (CLI), `zengui` (GUI), `zenwatch` (notifier), `zenkey-explorer-config`:
Apache-2.0. crates.io receives the lib crates only (the `v1` branch
publishes zenkey → zenkey-build → zenkey-fleet, in that order); binaries
ride Forgejo releases (linux x86_64).
