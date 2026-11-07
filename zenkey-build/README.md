# zenkey-build

**zk2's codegen**: typed servers, consumers and clients from
[zk2](https://git.marcpardo.eu/marcpardo/zenkey) interface contracts,
generated from an application's build script for the
[`zenkey`](https://crates.io/crates/zenkey) runtime. For each set of
contracts it:

- runs every lint of [`zenkey-model`](https://crates.io/crates/zenkey-model)
  (stable codes; an error fails the build, a warning is a
  `cargo:warning`);
- with a history, gates each contract against its published revisions: a
  breaking revision fails the build, one to review warns;
- embeds each contract's bundle, built once here and served byte for byte
  at run time;
- generates one module per interface (`thruster.v1` → `thruster_v1`): its
  `IFACE`, `FINGERPRINT` and `BUNDLE`, its types (prost's for protobuf,
  compiled from the bundle's own descriptor sets with no `protoc`;
  typify's for JSON Schema; a struct with a straight-line codec for a
  packed type), and, as the contract has them, the `Handlers` and `Api`
  traits, `Server`, `Consumer`, `Client` and `Fleet`.

## Starting

```toml
[dependencies]
zenkey = "0.20"

[build-dependencies]
zenkey-build = "0.20"
```

Take `zenkey` and `zenkey-build` at the same 0.x: the generated code names
`zenkey` (and, through it, `prost`). JSON Schema types also need `serde`
and `serde_json` among your dependencies.

```rust
// build.rs
fn main() {
    zenkey_build::Config::new()
        .contracts_dir("contracts")              // *.toml, the authoring format
        .history_dir("contracts/.history")       // optional: the compatibility gate
        .generate()
        .unwrap_or_else(|e| panic!("{e}"));
}
```

```rust
// src/zk2.rs
include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
```

A service then declares each interface on `zenkey::ServiceBuilder::new`'s
builder with its generated `Server::declare`, and starts
([`zenkey`'s README](https://crates.io/crates/zenkey) has the whole
example). `Config::contract_crate(true)` puts every runtime item behind a
`zenoh` feature, for a crate of contracts whose default build has no
zenoh dependency (`zenkey` with `default-features = false`).

## What 0.20 does not promise

0.20 is the first zk2 release. Two things under it are still moving, and
a 0.21 may break either:

- **The authoring format** and the canonical form a fingerprint is taken
  over. A contract that builds under 0.20 may need editing under
  0.21, its fingerprint may change, and so may the generated API.
- **The wire.** zk2c, the bandwidth-first wire (epic
  [#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is being
  specified: one snapshot per collection
  ([#977](https://git.marcpardo.eu/marcpardo/zenkey/issues/977)), and
  config.v2's calls without the bare wire (C4d,
  [#985](https://git.marcpardo.eu/marcpardo/zenkey/issues/985)).

A 0.20.x patch release changes neither.

**Zenoh's unstable API:** none. This crate depends on no zenoh crate but
`zenoh-keyexpr`, through `zenkey-model`.

## Documentation

- [`docs/zk2/codegen.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/codegen.md):
  what is generated, and why.
- [`docs/zk2/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/README.md):
  the design record; the authoring format is
  [`examples/zk2/README.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/examples/zk2/README.md).
- Issues: [git.marcpardo.eu/marcpardo/zenkey](https://git.marcpardo.eu/marcpardo/zenkey/issues).
  The GitHub repository is a mirror.

zenkey-build 0.11.x on crates.io is v1's registry codegen, maintained on
the `v1` branch. 0.20 shares its name and nothing else.

MIT.
