# zenkey — a semantic convention for Zenoh keyspaces

Zenoh gives you a wildly capable pub/sub/query fabric — and [three rules of
thumb](https://zenoh.io/docs/manual/abstractions/) for naming keys. Everything
that makes a *fleet* manageable — who published this, what class of data is it,
which subjects exist, what QoS does each deserve, how do admin tools discover
any of it — is left to every application to invent.

**zenkey is that missing layer, made explicit and executable:**

| Part | What it is |
|------|-----------|
| [`rfcs/`](rfcs/00-index.md) | The **keyspace-v2 convention** — a normative RFC set. Grammar `<base>/v1/<origin>/<class>/<producer>/<subject...>`, planes (`@rpc`/`@media`/`@blob`), identity/`@catalog`, registry + introspection, operations. Chapters 02–10 are application-neutral; chapter 11 is a reference application profile ([ZenSight](https://github.com/p13marc/zensight)). |
| [`zenkey/`](zenkey/) | The **enforcement crate**: typed key grammar, origin minting, slugs, QoS profiles, and a registry codegen (`registry/*.toml` → typed subject/procedure builders, linted at build time). Keys are built through types, not `format!`. |
| [`zenctl/`](zenctl/) | The **bus explorer CLI** — the `busctl`/`d-feet` equivalent for a convention-conformant bus: `topic list/info/echo`, `node list`, `service list/call`, `doctor`. Discovers foreign apps live via `introspect` (RFC 08 §6). |

## Why a convention and not just keys?

Because the key layout is what turns a *pipe* into a *data platform*: a fixed
origin/class position is what makes per-host ACLs, storage selection, and
router-pinned QoS expressible at all. If every subsystem invents its own
layout, none of those can be written down.

## Status

The convention is ratified as **v1** and deployed by
[ZenSight](https://github.com/p13marc/zensight) (see the RFC index for the
current amendment level). This repo graduated from the ZenSight monorepo in
2026-07 with history preserved; the engine crate currently still bundles
ZenSight's subject registry as its compiled-in default — making the registry
fully consumer-supplied is the next step (see issues).

## License

`zenkey` (lib): MIT. `zenctl` (CLI): Apache-2.0.
