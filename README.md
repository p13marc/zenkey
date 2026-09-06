# zenkey — a semantic convention for Zenoh keyspaces

Zenoh gives you a wildly capable pub/sub/query fabric — and [three rules of
thumb](https://zenoh.io/docs/manual/abstractions/) for naming keys. Everything
that makes a *fleet* manageable — who published this, what class of data is it,
which subjects exist, what QoS does each deserve, how do admin tools discover
any of it — is left to every application to invent.

**zenkey is that missing layer, made explicit and executable:**

| Part | crates.io | What it is |
|------|-----------|-----------|
| [`rfcs/`](rfcs/00-index.md) | — | The **keyspace-v2 convention** — a normative RFC set. Grammar `<base>/v1/<origin>/<class>/<producer>/<subject...>`, planes (`@rpc`/`@media`/`@blob`), identity/`@catalog`, registry + introspection, operations. Chapters 02–10 are application-neutral; chapter 11 is a reference application profile ([ZenSight](https://github.com/p13marc/zensight)). |
| [`zenkey/`](zenkey/) | [zenkey](https://crates.io/crates/zenkey) | The **runtime crate**: typed key grammar, origin minting, application profiles, slugs, QoS profiles, registry slices. Keys are built through types, not `format!`. App-neutral — ships no registry. |
| [`zenkey-build/`](zenkey-build/) | [zenkey-build](https://crates.io/crates/zenkey-build) | The **codegen crate**: lints your application's `registry/*.toml` (RFC 08 §5, build errors in *your* build) and generates typed subject/procedure builders + parsers from your build script. |
| [`zenkey-fleet/`](zenkey-fleet/) | [zenkey-fleet](https://crates.io/crates/zenkey-fleet) | The **fleet engine**: disciplined fan-in queries, liveliness roster, registry-slice sets, the RFC 08 §7 schema-decode pipeline, and the live key-tree monitor — the shared core of `zenctl` and `zengui`. |
| [`zenctl/`](zenctl/) | *(binaries only)* | The **bus explorer CLI** — the `busctl`/`d-feet` equivalent for a convention-conformant bus: contexts, `get` (any selector, fleet-disciplined), `topic list/info`, `echo`/`pub`/`retire`/`rate` (+ `pub --from ndjson`), `node list`, `service list/call`, `scout`, `serve` (mock queryable), `key includes/intersects/canon`, `admin routers/graph` (the mesh, `--dot` for Graphviz), `record`/`replay` (`.zrec` capture with in-file drop ledger, etiquette-enforced replay — RFC 09 §5.2), `doctor`, `check expect/cutover/retired/probe/schema` (one exit contract: 0 clean, 1 a finding, 2 no verdict), attachments on echo/pub, `--zenoh-config` passthrough for secured buses, shell completions, `--format json/ndjson`. Discovers any conformant fleet live via `introspect` (RFC 08 §6), or reads local registry TOMLs via `--registry`. Ships as Forgejo release binaries for **linux x86_64 only** (the one platform this project stands behind — zenkey #212); every other architecture and OS builds from source with `cargo install --git`. Not published to crates.io (0.1.x remains there un-yanked). |
| [`zengui/`](zengui/) | *(binaries only)* | The **graphical bus explorer** (Iced): a live key tree with per-key counts, rates and byte totals, and a filtered echo pane. **Key-agnostic** — it is useful on *any* Zenoh bus, and keyspace-v2 awareness is an enrichment overlay that labels origin/class/producer and resolves payload types only when a key parses. Non-conforming keys are first-class, not errors. Same distribution as `zenctl`. |
| [`zenwatch/`](zenwatch/) | *(binaries only)* | The **notifier**: one process, one JSON5 config, N rules over the closed `zenctl watchdog` vocabulary plus `alerts <SEL>` (the producers' own alert documents — a put is firing, a delete is resolved) and `liveliness-gone <SEL>` (the dead-man's switch), M sinks (`ntfy`, `smtp`, `webhook`, `exec`). Three states ride into every sink payload — `unobservable` is never folded into `ok` — and secrets are env-var names or file paths, never inline. `check-config` refuses what `run` would refuse; `--dry-run` prints what it would send and sends nothing. Same distribution as `zenctl`. |
| [`zenkey-explorer-config/`](zenkey-explorer-config/) | *(unpublished)* | The **shared explorer config**: named connection contexts, the config path policy and the completion cache dir — one file, two explorers. A sibling of the frontends, not a layer of the engine: it touches no bus, and living in `zenkey-fleet` forced `dirs` and `toml` onto every library consumer. |

(`fixture-tests/` is an unpublished workspace member: the ZenSight registry as
codegen regression corpus.)

## Why a convention and not just keys?

Because the key layout is what turns a *pipe* into a *data platform*: a fixed
origin/class position is what makes per-host ACLs, storage selection, and
router-pinned QoS expressible at all. If every subsystem invents its own
layout, none of those can be written down.

## Adopting it

```rust
// One static per application: name + origin salt (RFC 06 §1). Each half is
// named at the call site, because two adjacent `&'static str`s in the wrong
// order used to compile into a working profile with the wrong salt.
use zenkey::{AppName, AppProfile, OriginSalt};

static PROFILE: AppProfile = AppProfile::new(
    AppName::new("acme-fleet"),
    OriginSalt::new("acme-fleet-host-id-v1"),
);
```

```rust
// build.rs — your registry TOMLs live in YOUR repo (RFC 08 §5).
zenkey_build::Config::new().registry_dir("registry").generate().unwrap();
```

See the [zenkey crate README](zenkey/README.md) for the full adoption story.

## Status

The convention is at **v1.34 (ratified at v1.18, 2026-08-15)**; see the
[RFC index](rfcs/00-index.md) for the amendment ledger. Deployed by
[ZenSight](https://github.com/p13marc/zensight) (reference profile, ch. 11)
and [tcgui](https://github.com/p13marc/tcgui). The registry is fully
**consumer-supplied** since 0.2.0: each application owns its `registry/*.toml`
and compiles it via `zenkey-build`. This repo graduated from the ZenSight
monorepo in 2026-07 with history preserved.

## License

`zenkey`, `zenkey-build` (libs): MIT. `zenkey-fleet` (lib), `zenctl` (CLI),
`zengui` (GUI), `zenwatch` (notifier), `zenkey-explorer-config`: Apache-2.0. crates.io receives the
lib crates only (publish order: zenkey → zenkey-build → zenkey-fleet);
binaries ride Forgejo releases (linux x86_64).
