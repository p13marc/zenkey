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

## Changes since 0.20

- **The wire breaks 0.20's.** A state GET is answered by snapshot
  (core 0.100, [#977](https://git.marcpardo.eu/marcpardo/zenkey/issues/977)):
  one per collection, in pages of form 7, read under consolidation `None`,
  merged per key, with the omission rule over a complete snapshot.
  `collection` reads the pages (`Heard`, `hear`, `Page::members`, `omit`)
  and `seed::apply_snapshot` applies the rule to what a late joiner holds.
  A `reply_del` on a member of a collection answered by snapshot is no
  owner's answer (`collection::stray_reply_del`, `STRAY_REPLY_DEL`). A
  `freshness.v2` heartbeat's time counts from the catalog's epoch
  (`freshness.v2` 0.6,
  [#1080](https://git.marcpardo.eu/marcpardo/zenkey/issues/1080)).
- **The bare wire is gone** (core 0.99,
  [#985](https://git.marcpardo.eu/marcpardo/zenkey/issues/985)'s C4d): a
  call's reply or error under any `Encoding` but `zenoh/bytes` carries no
  form.
  - `envelope::{Envelope, EnvelopeError, decode, encode, JSON, CBOR,
    PROTOBUF}`, `Detail::raw`, `Detail::raw_bytes` and
    `ErrorReading::Envelope` are removed. Read a reply error with
    `envelope::read_error` or `ErrorForm::read`: one that is no error form
    is the transport's.
  - `wire::read_value` returns `Result<wire::Envelope, FormRefusal>` in
    place of a `Reading`, whose `Plain` was the bare wire:
    `FormRefusal::Encoding` under another `Encoding`, `FormRefusal::Form`
    for the parse's refusal, and `FormRefusal::reason` to name either.
- **The authoring format and the canonical form did not move:** a
  contract that lints under 0.20 lints under 0.21, at the same
  fingerprint.

## What 0.21 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. Two things under 0.21 are still moving, and a 0.22 may break
either:

- **The authoring format** and the canonical form a fingerprint is taken
  over. A contract that lints under 0.21 may need editing under
  0.22, and its fingerprint may change.
- **The wire.** An event's key is to end in its run and seq in place of
  a ULID ([#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s
  C2), and the rest of #975 follows. The keys, the envelope and the forms
  this crate reads and writes move with it.

A 0.21.x patch release changes neither.

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
