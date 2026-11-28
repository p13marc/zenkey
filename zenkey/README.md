# zenkey

The **zk2 runtime**: the session half of
[zk2](https://git.marcpardo.eu/marcpardo/zenkey), typed interface
contracts over [Zenoh](https://zenoh.io). A service at an address
`<system>/<service>` implements interfaces, each declared by a contract.
This crate brings the service up on a Zenoh session in the spec's order
(its resources, then its descriptor and bundles, then its liveliness
tokens), writes every sample with its contract's QoS,
reads data through a role's bindings, serves and calls operations, and
runs the owner's half of each profile that has one (`hostid.v1`,
`health.v1`, `freshness.v2`, `alarms.v1`, `config.v1`, `telemetry.v1`,
`desired.v1` and `desired.v2`, …). `health.v2`, the same health in packed
records with its words as indices into tables, is opt-in
(`ServiceBuilder::health_v2`): a service implements one of the two majors,
and `health.v1` stays the default. So is `alarms.v2`, each alarm one packed
record on a slot, named by the service's own table of rules or by words
(`ServiceBuilder::alarms_v2`), beside the default `alarms.v1`.

What needs no session (keys, contracts, bundles) is
[`zenkey-model`](https://crates.io/crates/zenkey-model)'s, re-exported as
`zenkey::model`. The typed module of each interface comes from
[`zenkey-build`](https://crates.io/crates/zenkey-build). Without its
default `zenoh` feature, this crate builds with no zenoh crate but
`zenoh-keyexpr`: what a contract crate's types and traits need.

## Starting

```toml
[dependencies]
zenkey = "0.23"
zenoh = "1.10"
tokio = { version = "1", features = ["full"] }

[build-dependencies]
zenkey-build = "0.23"
```

Take `zenkey` and `zenkey-build` at the same 0.x: the generated code is
written against this crate's `zenkey::typed`.

1. **A contract** per interface major, a TOML file in the authoring format
   ([`examples/zk2/README.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/examples/zk2/README.md)),
   say `contracts/thruster.v1.toml`. The contract CI checks it: `cargo
   install zenkey-model` gives `zk2 contract lint`.
2. **`zenkey-build` in `build.rs`** lints the contracts, gates them against
   their `.history`, embeds their bundles and generates a typed module per
   interface:

   ```rust
   // build.rs
   fn main() {
       zenkey_build::Config::new()
           .contracts_dir("contracts")
           .history_dir("contracts/.history")
           .generate()
           .unwrap_or_else(|e| panic!("{e}"));
   }
   ```

   ```rust
   // src/zk2.rs
   include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
   ```

3. **A service**, configured rather than coded, and brought up by
   `ServiceBuilder::new`:

   ```rust
   use std::sync::Arc;
   use zenkey::{ServiceBuilder, ServiceConfig};
   use crate::zk2::thruster_v1;

   let session = zenoh::open(zenoh::Config::default()).await?;
   // A literal address; `ServiceConfig::minted` asks hostid.v1 for the system.
   let config = ServiceConfig::new("vehicle-01/thruster-l".parse()?)
       .bind("cmd", &["vehicle-01/teleop"]);
   let mut b = ServiceBuilder::new(&session, config);
   // `MyThruster` implements `thruster_v1::Handlers`: `arm` and `disarm`.
   let thruster = thruster_v1::Server::declare(&mut b, Arc::new(MyThruster)).await?;
   let _service = b.start().await?; // resources, descriptor and bundles, then tokens
   thruster
       .telemetry
       .put(&thruster_v1::types::Telemetry { rpm: 1200.0, ..Default::default() })
       .await?;
   ```

The walkthrough is the worked example: its contracts are in
[`examples/zk2/walkthrough/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/examples/zk2/walkthrough),
and `examples/owner.rs` is a service to interoperate with, driven from
its standard input.

## Changes since 0.22

- **The wire breaks 0.22's: a 0.22 reader does not read a 0.23 owner's
  descriptor.** Move a deployment's readers first, then its owners: a
  0.23 reader reads a 0.22 owner's descriptor, which states minor 0.
  - An entry states its revision's minor only where the owner holds the
    authoring file, and none for a revision from its bundle (core 0.107,
    [#1127](https://git.marcpardo.eu/marcpardo/zenkey/issues/1127)):
    every generated `implementation()` and every standard profile's. A
    0.22 reader requires the member, and refuses the whole descriptor.
  - **The compact descriptor** (core 0.120,
    [#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s C5): an
    owner on a first hop given a catalog
    (`ServiceConfig::first_hop`, `ServiceBuilder::catalog`) writes its
    descriptor in a few tens of bytes, under `zenoh/bytes`, where the JSON
    form takes some hundreds. `ServiceConfig::compact_descriptor` asks for
    either form; `None`, the default, writes this one there and the JSON
    form elsewhere, and a role keeps the JSON form. Its readers need the
    catalog and every revision it names, held or retrieved, the standard
    contracts this crate embeds always held (`zenkey::standard`, new), so
    a consumer of a first-hop device's own interface reads it with its own
    contract and the catalog: one without reads `presence::Found::Unread`,
    never absent. A 0.22 reader reads
    none, so a first-hop owner moves after its readers, or runs
    `compact_descriptor(false)`.
  - **An archive's attachment** may be compact too (core 0.120, #980's
    C3): `Archive::start_under` writes it where its catalog lists the
    value's revision, and `archive::LastKnown::unresolved` says why one
    was not read.
  - **The ahead time** (core 0.113): an owner whose clock guard holds
    writes its heartbeat's time as ahead, which a reader before 0.113
    refuses.
- **Added: the owner heartbeat**
  ([#1110](https://git.marcpardo.eu/marcpardo/zenkey/issues/1110),
  `freshness.v2` 0.9 to 0.14). An owner given a catalog may confirm
  several interfaces in one put per class of exposure, on
  `…/freshness.v2/stream/heartbeats/<class>`, in place of one heartbeat
  per interface.
  - **`ServiceConfig::owner_heartbeat`**, off unless the deployment sets
    it (`owner_heartbeat: true` in a config file). It needs
    `ServiceBuilder::catalog`: a service with the option and no catalog
    does not start. `Service::owner_heartbeat_cover` lists the interfaces
    covered.
  - **Turn it on once the readers read it.** A consumer subscribes to an
    owner's class keys only when it holds a catalog: `Consumer::with_catalog`
    now holds the one given, and a consumer with none declares no class
    key, so to it every member of a covered interface reads stale.
    `Consumer::with_owner_catalog_timeout` bounds the read that learns the
    catalog an owner names.
  - A descriptor of a `freshness.v2` owner lists the standard contract's
    revision 1.1 (`9e07704d…`).
  - `consumer::HeartbeatRead` gains `group`: the class and the index a
    heartbeat read names.
- **Added: four profiles' second majors,** each opt-in beside its first:
  `health.v2` (`ServiceBuilder::health_v2`, `health_v2_with`;
  `health::Effective::word`), `alarms.v2` (`ServiceBuilder::alarms_v2`),
  `desired.v2` (`desired_author` and `desired_target` over a contract
  that lists it) and `redundancy.v2`, a follower that fences by run
  (core 0.119). `ServiceBuilder::health_with` and `config_v1_with` take
  an implementation of your own.
- **Added: a conditional read** (core 0.110, #986's J1):
  `Consumer::get_conditional` asks with the stamps it holds, and an
  unchanged member is answered by its held stamp alone.
- **zenoh and zenoh-ext 1.10.2**
  ([#1165](https://git.marcpardo.eu/marcpardo/zenkey/issues/1165)), which
  `zenoh = "1.10"` already resolves.
- **Grown or changed, and what to write instead:**
  - `ServiceConfig` gains `owner_heartbeat` and `compact_descriptor`:
    build it with `ServiceConfig::new` and its methods, never a literal.
  - `desired::Written` gains `PutV2` and `DeletedV2`, `desired::Kept`
    `major`, `desired::Document` `inband` and `desired::Seeded` `revision`
    ([#1156](https://git.marcpardo.eu/marcpardo/zenkey/issues/1156)).
    `desired::TargetStartup` gains `word_alone`, `across_face` and
    `hold`: a target holds a new run's versions until its descriptor
    states the run, its author's token appears, or `hold` (10 s) passes
    (PR #1176).
  - `desired::Target::set_reread` returns a `Result`: a period below a
    `desired.v2` read's timeout is refused
    ([#1166](https://git.marcpardo.eu/marcpardo/zenkey/issues/1166)).
  - `presence::Found` gains `Unread`, a descriptor not read (above).
  - `archive::LastKnown` gains `unresolved`.
- **A subscription across a face that re-declaring cannot repair is repaired** (core 0.121 §8.5, [#1185](https://git.marcpardo.eu/marcpardo/zenkey/issues/1185)). Where another reader of the key shares the far router, a re-declaration sends nothing across. The consumer declares a repair subscription on a key new at that router, the last literal chunk with `$*`, at most three for a subscription's life, and filters its samples locally. An event's subscription is re-declared with the others. `Subscription::repaired()` counts them.

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
  over. A contract that builds under 0.23 may need editing under 0.24,
  and its fingerprint may change.

A 0.23.x patch release changes none of them. The rules are the spec's
([`spec/core.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/core.md),
its version in its header), and a release implements the spec as it
stood when it was cut.

## Zenoh's unstable API

This crate takes `zenoh` and `zenoh-ext` 1.10 with their `unstable`
feature, so your build's zenoh has it on too. The core (§0) has a
participant use only stable Zenoh API, and names the few places where
one opts into more (§2.4, §2.5); an opt-in never changes a key's shape.
This crate opts into each of them, and into nothing else:

| Unstable in zenoh 1.10.1 and 1.10.2 | Where | Why |
|---|---|---|
| `zenoh_ext::AdvancedPublisher`, `AdvancedPublisherBuilderExt`, `CacheConfig`, `MissDetectionConfig` | `writer` | A resource whose contract declares `history` (core §2.5): the publisher's cache and its sample-miss detection. A resource without `history` is written by a plain publisher. |
| `PublisherBuilder::reliability`, `PublicationBuilder::reliability`, `zenoh::qos::Reliability` | `writer`, `service`, `qos` | The contract's declared `reliability`, on every publisher and every event's put (core §2.4). Without it every sample would go reliable, which §2.4 allows, and a `best_effort` stream would cost what a reliable one does. |
| `SessionInfo::transport_events_listener`, `TransportEventsListener` | `presence` (the watch barrier), `redeclare` | When the session's transport closed or opened: a barrier's round trip stands only for the transport it was asked on (core §8.1), and a silence across a reopen says nothing of a lost declaration (core §8.5). |
| `SessionInfo::links` | `presence::cut_off` | Whether the session still reaches another node (observer §3.5): a presence read, a barrier's round trip or a subscription declared again would go nowhere through a session cut off, so none is made. |

`zenkey-model` and `zenkey-build` use no zenoh API but `zenoh-keyexpr`'s,
which is stable.

## Documentation

- [`docs/zk2/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/README.md):
  the design record, what the code generator gives you
  ([`codegen.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/codegen.md)),
  and, for a team on zenkey 0.11, the
  [migration note](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/docs/zk2/migration.md).
- [`spec/`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/README.md):
  the rules, for implementers.
- Issues: [git.marcpardo.eu/marcpardo/zenkey](https://git.marcpardo.eu/marcpardo/zenkey/issues).
  The GitHub repository is a mirror.

zenkey 0.11.x on crates.io is v1, the keyspace-v2 convention, maintained
on the `v1` branch. 0.20 and later share its name and nothing else.

MIT.
