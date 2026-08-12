---
name: registry-conventions
description: RFC 08 §5 registry TOML conventions and zenkey-build codegen rules — read before editing any registry/*.toml, deprecated.lock, or the codegen in zenkey-build.
---

# Registry codegen and TOML conventions

## Registry codegen (zenkey-build — the load-bearing piece)

`Config::generate()` reads every `registry/*.toml` in the consumer's dir,
**lints it against RFC 08 §5** (returned as `Error`, surfaced by the consumer's
build.rs `unwrap()`), checks the append-only `deprecated.lock` ledger, and
emits the module. Codegen is normative in **both** directions (RFC 08 §1):
an unregistered subject does not construct; a metric name refines into a typed
subject with named variables (never `parts[1]`). Consumers that cannot parse a
subject **drop it** — no string-parsing fallback.

## Conventions to preserve when editing registry TOMLs

(Fixtures live here in `fixture-tests/registry`; the live files are in the
application repos.)

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
- **`registry.lock` is the RFC 08 §3.1 compatibility snapshot** (default
  `compat = "backward"` per file): an existing subject's class/type or a
  procedure's kind/request/reply may never change in place — retire through
  `[[deprecated]]` and add a suffixed sibling. Additive edits need
  `zenctl registry lock <dir>` regenerated and committed; `compat = "none"`
  opts a file out, loudly (build warning). Never hand-edit the lock.
