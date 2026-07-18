# zenkey

The executable form of the **keyspace-v2 convention**
([`rfcs/`](../rfcs/00-index.md), v1) for Zenoh keyspaces.
Producers and consumers emit and parse conforming keys through this crate and
never spell raw key strings. Application-neutral: nothing app-specific is
compiled in.

```text
v1/<origin>/<class>/<producer>/<subject...>        (base-relative; the base
                                                     is the session namespace)
```

| Module | Enforces | Mechanism |
|---|---|---|
| `grammar` | RFC 03 — chunk charset, reserved tokens, structural assembly/parse, wire-key helpers | validation + typed builders: an invalid key does not construct |
| `origin` | RFC 06 §1 — `h-<12hex>` minting, fallbacks | pinned to the RFC test vector |
| `profile` | RFC 06 §1 / RFC 11 §4 — the app name + origin salt an adopter declares | `AppProfile`, one static per application |
| `slug` | RFC 03 §2 — RFC-5952 IP canon, lossless `_xNN_` escape | pure functions, injectivity-tested |
| `qos` | RFC 04 §3 — the five named profiles | closed enum → zenoh QoS triple (behind the default `zenoh` feature) |
| `context` | RFC 03/04/05/07 framework keys | `V1Context` — origin + producer, every framework key built through it |
| `slice` | RFC 08 §6 — `RegistrySlice`, the `introspect` reply type + the diff | parse a served slice, diff it against ours; a disagreement is a *finding* |
| `tests/guard.rs` | RFC 03 §4 — design properties D1–D6, ACL inclusion | key algebra pinned as CI tests |

## Adopting the convention

An application declares its **profile** — its name and origin salt, the two
constants RFC 06 §1 leaves to the application:

```rust
use zenkey::{AppProfile, V1Context};

static PROFILE: AppProfile = AppProfile::new("acme-fleet", "acme-fleet-host-id-v1");

let ctx = V1Context::for_producer(&PROFILE, "sysinfo");
let health = ctx.health_key();     // "v1/h-3fa9c2d41b7e/state/sysinfo/health"
```

The subject vocabulary is governed by the registry (RFC 08) and is
**application-owned**: check `registry/*.toml` into your repo and generate the
typed subject/procedure builders with the
[`zenkey-build`](https://crates.io/crates/zenkey-build) crate from your build
script. This crate ships no registry.

**Build** — an unregistered subject does not construct:

```rust
let key = sysinfo::key(ctx.origin(), &sysinfo::Subject::DiskUsed { mount: "_".into() })?;
```

**Parse** — the direction that deletes positional `split('/')` from consumers:
a metric name refines straight into a typed subject with its variables named:

```rust
match sysinfo::Subject::parse_metric(&metric) {
    Some(sysinfo::Subject::DiskUsed { mount }) => …,     // not parts[1]
    None => { /* unregistered — drop it, loudly */ }
}
```

A consumer that cannot parse a subject **drops it** — there is deliberately no
string-parsing fallback: a fallback silently masks an unregistered subject,
which is precisely the defect this crate exists to prevent ("a subject that is
not registered does not exist").

## The deployment base

There is deliberately **no base constant** in this crate. The base is the
value a deployment sets as its Zenoh session `namespace` — an isolation
boundary, not a string convention (RFC 09 §0). Only session config,
router-side artifacts, and un-namespaced debug tools
([`zenctl`](https://crates.io/crates/zenctl)) ever see full keys;
`grammar::with_base` / `strip_base` / `parse_full` serve exactly those.

## Features

- `zenoh` *(default)* — the `QosProfile → zenoh::qos` mappings. Disable
  (`default-features = false`) where the zenoh stack is unwanted, e.g. in
  build scripts; `zenkey-build` does this for you.
