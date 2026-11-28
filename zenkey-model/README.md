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

## Changes since 0.22

- **The wire breaks 0.22's: a 0.22 reader does not read a 0.23 owner's
  descriptor.** Move a deployment's readers first, then its owners: a
  0.23 reader reads a 0.22 owner's descriptor, which states minor 0.
  - `descriptor::InterfaceEntry::minor` is an `Option<u64>` (core 0.107,
    [#1127](https://git.marcpardo.eu/marcpardo/zenkey/issues/1127)). An
    entry states the minor of the revision it names, and none where its
    owner holds only the bundle, as generated code and the standard
    profiles do. 0.22 requires the member, and refuses the whole
    descriptor ("missing field `minor`").
  - The compact form (core 0.120,
    [#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s C5):
    `descriptor::compact` encodes and decodes it through a catalog and
    the revisions it names, and `descriptor::Form` tells the two forms
    apart by their `Encoding`. An owner on a first hop given a catalog
    writes it. 0.22 reads none.
  - A heartbeat's time may be the ahead time (core 0.113): the owner's
    clock guard holds (`heartbeat::Time::Ahead`,
    `health::ClockReason::Ahead`). A reader before 0.113 refuses it.
  - `freshness.v2` 0.8 to 0.14: the owner heartbeat, one put per class of
    exposure with a group per interface
    ([#1110](https://git.marcpardo.eu/marcpardo/zenkey/issues/1110);
    `heartbeat::Collection::class`, `freshness::SectionRead::OtherClass`),
    and the standard contract's revision 1.1.
  - A conditional read (core 0.110,
    [#986](https://git.marcpardo.eu/marcpardo/zenkey/issues/986)'s J1):
    an unchanged member is answered by its held stamp alone
    (`wire::SampleKind::Get`, `wire::Refusal::{Held, HeldTag}`;
    `SampleKind::ALL` has six entries).
- **Four profiles' second majors,** session-free: `health::v2`,
  `alarms::v2`, `desired::v2` and `redundancy::v2`, each its text's.
- **`link.v2` 0.14 to 0.30, `link.v1` 0.19 to 0.22:**
  - subjects by user name, the far end opening the face
    ([#1107](https://git.marcpardo.eu/marcpardo/zenkey/issues/1107);
    `Face::opens`, `Deployment::users`, `Code::NearOpensUser`);
  - the owner heartbeat in the plans
    ([#1145](https://git.marcpardo.eu/marcpardo/zenkey/issues/1145);
    `Deployment::owner_heartbeats`, `Code::OwnerHeartbeatDenied`);
  - zenoh 1.10.2 ([#1165](https://git.marcpardo.eu/marcpardo/zenkey/issues/1165);
    `Binding::Zenoh1_10_2`, and `Binding: FromStr`);
  - a frame weighed with what zenoh writes beside it
    ([#1136](https://git.marcpardo.eu/marcpardo/zenkey/issues/1136);
    `Res::{priority_data, express}`), and `Res::cardinality`.
- **One lint more: E051** refuses a contract that declares a packed type
  at layout tag 0, a profile-derived type aside
  ([#1152](https://git.marcpardo.eu/marcpardo/zenkey/issues/1152)). The
  authoring format and the canonical form did not move: a contract that
  lints under 0.22 lints under 0.23, at the same fingerprint, unless
  E051 refuses it.
- **Grown, so a `match` over them or a literal of them needs the new
  members:** `desired::Finding::{Escape, RunFields}`,
  `desired::Invalid::HorizonV1`, `redundancy::Invalid::{Majors,
  ExplicitStream}`, eight `alarms::Reason`s (`Rebound`, `Snapshot`,
  `SnapshotIncomplete`, `SnapshotRun`, `Table`, `NoWords`, `NoRecord`,
  `Unreadable`), `arbitration::Event::Read`'s `asked` and
  `redundancy::Holding::Read`'s fourth field. `link::Code`'s
  discriminants moved, which only an `as` cast sees.
- **Moved:** `desired::Ended<R = Refusal>` takes the refusal's type
  ([#1153](https://git.marcpardo.eu/marcpardo/zenkey/issues/1153)), and
  `desired::{in_force, last_known, refused}` take one more type
  parameter with it. `arbitration::Incumbents` is `redundancy::Fence`
  ([#987](https://git.marcpardo.eu/marcpardo/zenkey/issues/987)'s N1).
- **The repair key** (core 0.121 §8.5, [#1185](https://git.marcpardo.eu/marcpardo/zenkey/issues/1185)): `link::repair_key` and `REPAIRS`, read against `conformance/repair/keys.json`.

## What 0.23 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. These are still moving under 0.23, and a 0.24 may break them:

- **The texts zk2py has not read yet.** The independent implementation's
  rounds ([#609](https://git.marcpardo.eu/marcpardo/zenkey/issues/609))
  were paused for zenoh-modem's needs. Core 0.120's compact descriptor and
  archive attachment ([#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980))
  link.v2 0.30's batch allowance
  ([#1136](https://git.marcpardo.eu/marcpardo/zenkey/issues/1136)) and
  core 0.121's repair subscription
  ([#1185](https://git.marcpardo.eu/marcpardo/zenkey/issues/1185)) are the
  reference's reading alone, and a round may amend them.
- **Owners' defaults that wait for these readers.** #986's Q3 and #984's
  S5 flip an owner's default once 0.23's readers are deployed: readers
  move first.
- **#980's rest:** the near side's descriptor size, and the minor.
- **The authoring format** and the canonical form a fingerprint is taken
  over. A contract that lints under 0.23 may need editing under 0.24,
  and its fingerprint may change.

A 0.23.x patch release changes none of them. The rules are the spec's
([`spec/core.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/core.md),
its version in its header), and a release implements the spec as it
stood when it was cut.

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
