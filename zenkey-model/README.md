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

## Changes since 0.21

- **The wire breaks 0.21's: an event's key ends in its owner's run and
  seq** (core 0.101 and 0.103,
  [#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s C2). An
  events key's last chunk is an occurrence chunk, the run and the seq in
  lowercase Crockford base32, 8 characters for a 16-bit run and 12 for a
  32-bit one, where a ULID took 26. The grammar refuses a ULID there, so
  a 0.21 owner's event keys are no zk2 keys to 0.22.
  - `chunk::occurrence` reads the stamp a chunk names, `is_occurrence`
    asks whether it names one, and `write_occurrence` writes it.
  - The put's payload is form 4, `CH · t32 · ms16 · record`: the
    occurrence's time from the deployment epoch, known or unknown, and its
    stamp its key's. `wire::Event` writes it, and `wire::read_event` and
    `read_put` (`Reading::Event`) read it. Its refusals are `key_stamp`
    and `time` (`Refusal::{KeyStamp, Time}`). A 0.21 reader refuses form
    4.
  - A put or a delete on an events key that carries no event is never
    delivered (core 0.104 and 0.105).
  - A `reply_del` is a stray by the owner's revision under its run, never
    the reader's (core 0.102,
    [#1094](https://git.marcpardo.eu/marcpardo/zenkey/issues/1094)): give
    `collection::stray_reply_del` the revision the owner's descriptor
    names.
- **`link.v2` 0.12 to 0.14**
  ([#1098](https://git.marcpardo.eu/marcpardo/zenkey/issues/1098),
  [#1106](https://git.marcpardo.eu/marcpardo/zenkey/issues/1106)):
  - A face lists the events it carries, `events = [...]`, as it lists its
    `frames` (`Face::events`). On a face with `events`, each other exposed
    event is denied by name, and an entry that names no event of a listed
    interface is `event_unknown`.
  - A face router stamps nothing. `TIMESTAMPING_OFF` is the
    `timestamping` block the plan writes, `StatedTransport::stamps` reads
    a router's at its `mode`, and the check warns `face_timestamping`.
- **Removed:** `chunk::is_lower_ulid`. Use `chunk::is_occurrence`, or
  `chunk::occurrence` for the stamp.
- **Grown:** `wire::Reading::Event`, `wire::Refusal::{KeyStamp, Time}`,
  `link::Code::{FaceTimestamping, EventUnknown}` and the field
  `link::Face::events`. A `match` over those enums, or a `Face` written
  field by field, needs them.
- **The authoring format and the canonical form did not move:** a
  contract that lints under 0.21 lints under 0.22, at the same
  fingerprint.

## What 0.22 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. Two things under 0.22 are still moving, and a 0.23 may break
either:

- **The authoring format** and the canonical form a fingerprint is taken
  over. A contract that lints under 0.22 may need editing under
  0.23, and its fingerprint may change.
- **The wire and the profiles.** These are still open:
  - `freshness.v2`'s heartbeat, one put per interface, which the
    maintainer is deciding
    ([#1110](https://git.marcpardo.eu/marcpardo/zenkey/issues/1110));
  - `link.v2`'s subjects by user name, in progress
    ([#1107](https://git.marcpardo.eu/marcpardo/zenkey/issues/1107));
  - #980's C3, the archive attachment, and C5, the compact descriptor.

  The keys, the envelope, the forms and the policies this crate reads
  and writes move with them.

A 0.22.x patch release changes neither.

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
