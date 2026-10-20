# zenkey — a semantic convention for Zenoh keyspaces

Zenoh gives you a wildly capable pub/sub/query fabric — and [three rules of
thumb](https://zenoh.io/docs/manual/abstractions/) for naming keys. Everything
that makes a *fleet* manageable — who published this, what class of data is it,
which subjects exist, what QoS does each deserve, how do admin tools discover
any of it — is left to every application to invent.

**zenkey is that missing layer, made explicit and executable:**

| Part | crates.io | What it is |
|------|-----------|-----------|
| [`rfcs/`](rfcs/00-index.md) | — | The **keyspace-v2 convention**, v1's normative RFC set, frozen at v1.50. Grammar `<base>/v1/<origin>/<class>/<producer>/<subject...>`, planes (`@rpc`/`@media`/`@blob`), identity/`@catalog`, registry + introspection, operations. Chapters 02–10 are application-neutral; chapter 11 is a reference application profile ([ZenSight](https://github.com/p13marc/zensight)). |
| [`spec/`](spec/README.md) *(zk2)* | — | The **zk2 specification**: the normative core ([`core.md`](spec/core.md), 0.27), the profiles (`hostid.v1`, `freshness.v1`, `health.v1`), the contract and descriptor schemas, conformance fixtures and network scenarios. It is written so that any language can implement it from `spec/` alone; [`impl/python/`](impl/python/README.md) is the second implementation. |
| [`zenkey/`](zenkey/) | [zenkey](https://crates.io/crates/zenkey) *(v1: 0.11.x)* | The **zk2 runtime** (0.20.0, unpublished until #606): `ServiceBuilder` brings a service up in the spec's order (§8.2), typed writers apply the contract's QoS and stamp state, consumers read through a role's bindings, and operations come with their clients; presence, contract retrieval, archives, and the owner halves of `hostid.v1`, `freshness.v1` and `health.v1`. What needs no session is `zenkey-model`'s, re-exported as `zenkey::model`. crates.io holds v1's runtime (typed key grammar, origin minting, `AppProfile`, registry slices), released from the `v1` branch. |
| [`zenkey-build/`](zenkey-build/) | [zenkey-build](https://crates.io/crates/zenkey-build) *(v1: 0.11.x)* | **zk2's codegen** (0.20.0, unpublished): from an application's build script it lints the contracts, gates each against its `.history` (a breaking revision fails the build), embeds the bundles, and generates one module per interface, with its types, a `Server` and a `Consumer`, and `Handlers`, `Client` and `Api` for its operations ([design](docs/zk2/codegen.md)). crates.io holds v1's codegen over `registry/*.{toml,kdl}`, released from the `v1` branch. |
| [`zenkey-fleet/`](zenkey-fleet/) | [zenkey-fleet](https://crates.io/crates/zenkey-fleet) *(v1: 0.18.x)* | The **fleet engine**, zk2's since #612 (0.20.0, unpublished): disciplined fan-in queries, presence, contract retrieval, the lens a raw observer resolves keys through, contract-resolved rendering, the live key-tree monitor, recording and replay — the shared core of `zenctl`, `zengui` and `zenwatch`. crates.io holds the v1 engine, released from the `v1` branch. |
| [`zenctl/`](zenctl/) | *(binaries only)* | The **bus explorer CLI**, the `busctl`/`d-feet` equivalent for a zk2 deployment, and useful on any Zenoh bus. **Nouns:** `service list/show`, `iface list/show`, `schema show`, `namespace list`, `storage list/gen`, `acl gen` (spec §11's grants, from contracts and an enrollment), `admin routers/graph` (the mesh, `--dot` for Graphviz), `key includes/intersects/canon`, `hostid`, `bench call`, contexts. **Wire verbs:** `get` (any selector, fleet-disciplined) and `get state` (the owner's answer, or an archive's last-known, never shown as current), `call` (an operation through its contract; a fan-out only where the contract allows one), `watch` (a resource, decoded through its contract), `echo`/`pub`/`rate`/`field` (+ `pub --from ndjson`; `pub` refuses a key a zk2 service owns), `record`/`replay` (`.zrec` capture with an in-file drop ledger; `record --on <rule> --pre 30` captures the thirty seconds before a condition fired; `replay --namespace` stands in for the owners it recorded), `snapshot` (+ `snapshot diff`, aligned by key), `timeline` (a lane per address and resource, on a stated clock), `graph` (the binding graph, from descriptors), `serve` and `gen` (mock owners), `scout`. **Judgements**, with one exit contract (0 clean, 1 a finding, 2 no verdict): `doctor`, `health`, `why`, `check expect/probe/schema/conform`, `watchdog`, and `compat` between two contract revisions. Every key resolves through presence and the contracts (the bundles the bus serves, or `--contracts <PATH>`) in the deployment's `--namespace` (alias `--base`); a payload renders as its declared type, else structurally, naming where resolution stopped. Attachments on echo/pub, `--zenoh-config` passthrough for secured buses, shell completions, `--format json/ndjson`. v1's spellings and where each went: [`zenctl/CHANGELOG.md`](zenctl/CHANGELOG.md). Ships as Forgejo release binaries for **linux x86_64 only** (the one platform this project stands behind — zenkey #212); every other architecture and OS builds from source with `cargo install --git`. The zk2 build is not released yet; the `v1` branch releases the v1 build as 0.14.x. Not published to crates.io (0.1.x remains there un-yanked). |
| [`zengui/`](zengui/) | *(binaries only)* | The **graphical bus explorer** (Iced): a live key tree with per-key counts, rates and byte totals, a filtered echo pane, the Inspector and the Workbench. **Key-agnostic** — it is useful on *any* Zenoh bus, and zk2 awareness is an overlay: a key resolved through presence and the contracts gains its address, interface and resource, a contract-typed decode and a conformance badge; any other key says where resolution stopped. Non-conforming keys are first-class, not errors. Same distribution as `zenctl`. |
| [`zenwatch/`](zenwatch/) | *(binaries only)* | The **notifier**: one process, one JSON5 config, N rules, M sinks (`ntfy`, `smtp`, `webhook`, `exec`). A rule is one spelling of a closed vocabulary: `zenctl watchdog`'s (rates, silence, invalid payloads, QoS mismatches, the doctor, `alert-firing`, drops), plus `instance-gone <SYSTEM/SERVICE>` (an address gone, read from presence; it replaces v1's `origin-down`), `liveliness-gone <SEL>` (the dead-man's switch) and `alerts <SEL>` (each `alarms.v1` alarm firing, resolved or unobservable on its own, #800). Three states ride into every sink payload — `unobservable` is never folded into `ok` — and secrets are env-var names or file paths, never inline. `check-config` refuses what `run` would refuse; `--dry-run` prints what it would send and sends nothing. A running daemon is a zk2 service of its own (`health.v1` and `zenwatch.v1`). v1's spellings and where each went: [`zenwatch/CHANGELOG.md`](zenwatch/CHANGELOG.md). Same distribution as `zenctl`. |
| [`zenkey-model/`](zenkey-model/) *(zk2)* | *(unpublished until the zk2 spec, #606)* | The **session-free half of zk2**: the `zk2/` key grammar, resource templates (most-literal-first), the contract authoring format (draft 1) with stable-coded lints, the canonical form, fingerprints, and bundles. No zenoh dependency beyond `zenoh-keyexpr`. It validates every contract in [`examples/zk2/`](examples/zk2/) and seeds the conformance fixtures in [`spec/`](spec/README.md). |
| [`zenkey-explorer-config/`](zenkey-explorer-config/) | *(unpublished)* | The **shared explorer config**: named connection contexts, the config path policy and the completion cache dir — one file, two explorers. A sibling of the frontends, not a layer of the engine: it touches no bus, and living in `zenkey-fleet` forced `dirs` and `toml` onto every library consumer. |

(`fixture-tests/`, the ZenSight v1 registry that v1's tests read and its
codegen-corpus crate, lives on the `v1` branch.)

## Why a convention and not just keys?

Because the key layout is what turns a *pipe* into a *data platform*: a fixed
system, service, interface and kind position is what makes per-service ACLs,
storage selection by kind, and router-pinned QoS expressible at all. If every
subsystem invents its own layout, none of those can be written down.

## Adopting it

On `main`, an application adopts zk2 through a contract, `zenkey-build` and
the runtime's `ServiceBuilder`. The crates are 0.20.0 and unpublished until
#606, so a port builds against `main` from source.

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

The v1 adoption path (`AppProfile`, a `registry/` and v1's `zenkey-build`)
lives on the `v1` branch, in its
[crate README](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/v1/zenkey/README.md).

## Status

**v1 is frozen. zk2 is built on `main`, unpublished until #606.**
- **The redesign.** zenkey's next major is zk2: typed, introspectable
  interface contracts over Zenoh, for application architectures and not
  only observability. Its design record is in [`docs/zk2/`](docs/zk2/README.md)
  (epic [#585](https://git.marcpardo.eu/marcpardo/zenkey/issues/585)).
- **Moving from v1.** [`docs/zk2/migration.md`](docs/zk2/migration.md) is
  the note for a team on v1 weighing zk2: what maps to what, what a port
  involves, and what zk2 does not have yet.
- **v1 stays maintained.** The v1 convention below is frozen at RFC v1.50 and
  maintained on the `v1` branch as 0.14.x patch releases. Existing adopters
  stay on zenkey 0.11.x and zenkey-fleet 0.18.x until they port.

The convention is at **v1.50 (ratified at v1.18, 2026-08-15)**; see the
[RFC index](rfcs/00-index.md) for the amendment ledger. Deployed by
[ZenSight](https://github.com/p13marc/zensight) (reference profile, ch. 11)
and [tcgui](https://github.com/p13marc/tcgui). The registry is fully
**consumer-supplied** since 0.2.0: each application owns its `registry/*.toml`
and compiles it via `zenkey-build`. This repo graduated from the ZenSight
monorepo in 2026-07 with history preserved.

## License

`zenkey`, `zenkey-build`, `zenkey-model` (libs): MIT. `zenkey-fleet` (lib),
`zenctl` (CLI), `zengui` (GUI), `zenwatch` (notifier), `zenkey-explorer-config`:
Apache-2.0. crates.io receives the lib crates only (publish order: zenkey →
zenkey-build → zenkey-fleet); binaries ride Forgejo releases (linux x86_64).
