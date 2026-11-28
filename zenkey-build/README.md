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
zenkey = "0.23"

[build-dependencies]
zenkey-build = "0.23"
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

## Changes since 0.22

- **This crate's API and the code it generates did not change**
  (`cargo semver-checks` against 0.22.0 finds nothing). What the
  generated code runs on did: take `zenkey` and `zenkey-build` at 0.23
  together. [`zenkey`'s README](https://crates.io/crates/zenkey),
  "Changes since 0.22", has the rest.
- **The wire breaks 0.22's, through the generated code.** A generated
  `implementation()` holds its revision's bundle and not its authoring
  file, so the owner's descriptor states no `minor` for it (core 0.107,
  [#1127](https://git.marcpardo.eu/marcpardo/zenkey/issues/1127)), and a
  0.22 reader refuses that descriptor. On a first hop given a catalog,
  the descriptor is the compact form (core 0.120,
  [#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s C5),
  which a 0.22 reader does not read either. Move the readers first.
- **The build refuses more:**
  - E051, a packed type declared at layout tag 0, a profile-derived type
    aside ([#1152](https://git.marcpardo.eu/marcpardo/zenkey/issues/1152));
  - in a contract that lists `desired.v2`, an invalid declaration of a
    desired or report resource; in one that lists `health.v2`, an
    invalid table of words; in one that lists `alarms.v2`, an invalid
    table of rules. Each is an error, since an owner would write no word
    of it.
  - A governed interface's invalid declaration, under either major of
    `redundancy` (`value`, `disagrees`, and `explicit_stream` under
    `redundancy.v2`), is a `cargo:warning`: its runtime refuses to start
    it.
- **The authoring format and the canonical form did not move:** a
  contract that builds under 0.22 builds under 0.23, at the same
  fingerprint, unless one of the checks above refuses it.

## What 0.23 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. These are still moving under 0.23, and a 0.24 may break them:

- **The texts zk2py has not read yet.** The independent implementation's
  rounds ([#609](https://git.marcpardo.eu/marcpardo/zenkey/issues/609))
  were paused for zenoh-modem's needs. Core 0.120's compact descriptor and
  archive attachment ([#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980))
  and link.v2 0.30's batch allowance
  ([#1136](https://git.marcpardo.eu/marcpardo/zenkey/issues/1136)) are the
  reference's reading alone, and a round may amend them.
- **Owners' defaults that wait for these readers.** #986's Q3 and #984's
  S5 flip an owner's default once 0.23's readers are deployed: readers
  move first.
- **#980's rest:** the near side's descriptor size, and the minor.
- **The generated API.** A packed type's struct may gain a constructor
  or a `Default`
  ([#1182](https://git.marcpardo.eu/marcpardo/zenkey/issues/1182)), and
  a contract that builds under 0.23 may need editing under 0.24.

A 0.23.x patch release changes none of them. The rules are the spec's
([`spec/core.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/core.md),
its version in its header), and a release implements the spec as it
stood when it was cut.

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
the `v1` branch. 0.20 and later share its name and nothing else.

MIT.
