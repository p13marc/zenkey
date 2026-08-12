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
| [`zenctl/`](zenctl/) | *(binaries only)* | The **bus explorer CLI** — the `busctl`/`d-feet` equivalent for a convention-conformant bus: contexts, `get` (any selector, fleet-disciplined), `topic list/info/echo/hz/bw/pub/retire` (+ `pub --from ndjson`), `node list`, `service list/call`, `scout`, `serve` (mock queryable), `key includes/intersects/canon`, `admin routers/graph` (the mesh, `--dot` for Graphviz), `record`/`replay` (`.zrec` capture with in-file drop ledger, etiquette-enforced replay — RFC 09 §5.2), `doctor`, attachments on echo/pub, `--zenoh-config` passthrough for secured buses, shell completions, `--format json/ndjson`. Discovers any conformant fleet live via `introspect` (RFC 08 §6), or reads local registry TOMLs via `--registry`. Ships as GitHub release binaries / `cargo install --git`; not published to crates.io (0.1.x remains there un-yanked). |

| [`zengui/`](zengui/) | *(binaries only)* | The **graphical bus explorer** (Iced): a live key tree with per-key counts, rates and byte totals, and a filtered echo pane. **Key-agnostic** — it is useful on *any* Zenoh bus, and keyspace-v2 awareness is an enrichment overlay that labels origin/class/producer and resolves payload types only when a key parses. Non-conforming keys are first-class, not errors. Same distribution as `zenctl`. |

(`fixture-tests/` is an unpublished workspace member: the ZenSight registry as
codegen regression corpus.)

## Why a convention and not just keys?

Because the key layout is what turns a *pipe* into a *data platform*: a fixed
origin/class position is what makes per-host ACLs, storage selection, and
router-pinned QoS expressible at all. If every subsystem invents its own
layout, none of those can be written down.

## Adopting it

```rust
// One static per application: name + origin salt (RFC 06 §1).
static PROFILE: zenkey::AppProfile = zenkey::AppProfile::new("acme-fleet", "acme-fleet-host-id-v1");
```

```rust
// build.rs — your registry TOMLs live in YOUR repo (RFC 08 §5).
zenkey_build::Config::new().registry_dir("registry").generate().unwrap();
```

See the [zenkey crate README](zenkey/README.md) for the full adoption story.

## Status

The convention is at **v1.5 (proposed — ratifies on merge of the 0.3
redesign)**; see the RFC index for the amendment ledger. Deployed by
[ZenSight](https://github.com/p13marc/zensight) (reference profile, ch. 11)
and [tcgui](https://github.com/p13marc/tcgui). The registry is fully
**consumer-supplied** since 0.2.0: each application owns its `registry/*.toml`
and compiles it via `zenkey-build`. This repo graduated from the ZenSight
monorepo in 2026-07 with history preserved.

## License

`zenkey`, `zenkey-build` (libs): MIT. `zenkey-fleet` (lib), `zenctl` (CLI):
Apache-2.0. crates.io receives the lib crates only (publish order: zenkey →
zenkey-build → zenkey-fleet); binaries ride GitHub releases.
