# zenkey-model

The session-free half of [zk2](https://git.marcpardo.eu/marcpardo/zenkey),
typed interface contracts over [Zenoh](https://zenoh.io): everything about
keys and contracts that can be decided from files, with no Zenoh session.
Build scripts, CI tools, the [`zenkey`](https://crates.io/crates/zenkey)
runtime and the conformance runners stand on it.

- **Keys** (`grammar`, `template`, `slug`): the `zk2/` grammar as typed
  values over `zenoh_keyexpr::OwnedKeyExpr`, resource templates with
  most-literal-first precedence, and injective slugging.
- **Contracts** (`authoring`, `contract`, `diag`): the TOML authoring
  format, with defaults expanded, types resolved (protobuf
  through `protox`, JSON Schema, packed types, raw media types), and every
  lint reported with a stable code.
- **Identity** (`canonical`, `bundle`, `history`, `compat`): the canonical
  JSON form, its fingerprint, the bundle container verified Merkle-style,
  the append-only `.history` of published revisions, and the
  compatibility classifier between two revisions.
- **The rest of the spec that needs no session:** a late joiner's merge,
  the zk2c envelope, and the session-free half of each profile.

## Starting

An application rarely depends on this crate directly: it writes a
contract, generates its typed module with
[`zenkey-build`](https://crates.io/crates/zenkey-build) in `build.rs`,
and brings a service up with `zenkey::ServiceBuilder::new`
([`zenkey`'s README](https://crates.io/crates/zenkey) has the example).
This crate is what both stand on, re-exported as `zenkey::model`.

Its `zk2` binary is the contract CI:

```bash
cargo install zenkey-model
zk2 contract lint contracts/thruster.v1.toml
zk2 contract fingerprint contracts/thruster.v1.toml
zk2 contract compat --history contracts/.history contracts/thruster.v1.toml
zk2 contract bundle contracts/thruster.v1.toml --history contracts/.history
```

## What 0.20 does not promise

0.20 is the first zk2 release. Two things under it are still moving, and
a 0.21 may break either:

- **The authoring format** and the canonical form a fingerprint is taken
  over. A contract that lints under 0.20 may need editing under
  0.21, and its fingerprint may change.
- **The wire.** zk2c, the bandwidth-first wire (epic
  [#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is being
  specified: one snapshot per collection
  ([#977](https://git.marcpardo.eu/marcpardo/zenkey/issues/977)). The
  envelope and the forms this crate reads and writes move with it.

A 0.20.x patch release changes neither.

**Zenoh's unstable API:** none. The only zenoh crate this one depends on
is `zenoh-keyexpr`.

## Documentation

- [`docs/zk2/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/README.md):
  the design record (`architecture.md`, r4).
- [`examples/zk2/README.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/examples/zk2/README.md):
  the authoring format.
- [`spec/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/README.md):
  the rules. `spec/contract.schema.json` and the `spec/conformance/`
  fixtures come from this crate.
- Issues: [git.marcpardo.eu/marcpardo/zenkey](https://git.marcpardo.eu/marcpardo/zenkey/issues).
  The GitHub repository is a mirror.

MIT.
