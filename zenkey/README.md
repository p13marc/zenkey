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
`health.v1`, `freshness.v2`, `alarms.v1`, `config.v1`, `telemetry.v1`, …).

What needs no session (keys, contracts, bundles) is
[`zenkey-model`](https://crates.io/crates/zenkey-model)'s, re-exported as
`zenkey::model`. The typed module of each interface comes from
[`zenkey-build`](https://crates.io/crates/zenkey-build). Without its
default `zenoh` feature, this crate builds with no zenoh crate but
`zenoh-keyexpr`: what a contract crate's types and traits need.

## Starting

```toml
[dependencies]
zenkey = "0.22"
zenoh = "1.10"
tokio = { version = "1", features = ["full"] }

[build-dependencies]
zenkey-build = "0.22"
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

## Changes since 0.21

- **The wire breaks 0.21's: a 0.21 service and a 0.22 one do not
  understand each other's events.** Move a deployment's owners and
  readers together.
  - An owner keys each occurrence by its run and its next seq (core
    0.103, [#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s
    C2), `…/events/<template>/<occurrence>`, 8 or 12 characters where a
    ULID took 26. Its payload is form 4: the occurrence's time, counted
    from the deployment epoch (the catalog's, given by
    `ServiceBuilder::catalog`, else 0), then the record. A 0.22 reader
    refuses a 0.21 owner's ULID keys, and a 0.21 reader refuses form 4.
  - `EventWriter::put` writes the clock's time, unknown while core §4.3's
    clock guard holds. `put_at` writes the time it is given, and its key
    still takes the next seq. `put_untimed` writes an unknown time, for an
    owner that does not trust its clock.
  - `Consumer::replay` orders by the event's time, then by run and seq,
    and the occurrences of unknown time after the rest (core §2.6).
    `Delivery::occurred` carries an event's time.
  - A put or a delete on an events key that carries no event is never
    delivered, live or replayed (core 0.104 and 0.105). It is counted
    undecodable, `form` (`inband::admit_event`, `Undecodable::NoEvent`),
    and a replay's refusals now reach `Consumer::with_refused`'s callback
    too.
  - A `reply_del` is a stray by the owner's revision under its run, never
    the reader's (core 0.102,
    [#1094](https://git.marcpardo.eu/marcpardo/zenkey/issues/1094)): the
    revision the owner's descriptor names, retrieved by its fingerprint
    when the reader holds another. A snapshot's omission rule applies only
    where a descriptor states the snapshot's run.
- **An archive keeps the watch session handed in**
  ([#1101](https://git.marcpardo.eu/marcpardo/zenkey/issues/1101)).
  Given its caller's only handle in `ArchiveConfig::watch`, it went deaf
  as `start` returned, and never re-aligned an owner whose token came
  back after a link cut.
- **Added:** `Consumer::subscribe_heartbeats` hands over each heartbeat
  its view reads, confirmed, diverged or absent, and whether one was lost
  before it (`consumer::{HeartbeatRead, HeartbeatSection}`;
  [#990](https://git.marcpardo.eu/marcpardo/zenkey/issues/990)'s V6).
  `inband::event_time` reads an event's time from its sample.
- **Removed, and what to use instead:**
  - `writer::ulid` and `writer::ulid_time`: an event's key ends in its
    occurrence chunk (`zenkey_model::chunk::occurrence` reads it), its
    time is `Delivery::occurred` or `inband::event_time`, and a call's
    request id is `call::request_id`.
  - Built field by field or matched whole, `Delivery` needs `occurred`
    and `Undecodable` its `NoEvent`.

## What 0.22 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. Two things under 0.22 are still moving, and a 0.23 may break
either:

- **The wire and the profiles.** These are still open, and a 0.22
  service and a 0.23 one may not understand each other:
  - `freshness.v2`'s heartbeat, one put per interface, which the
    maintainer is deciding
    ([#1110](https://git.marcpardo.eu/marcpardo/zenkey/issues/1110));
  - `link.v2`'s subjects by user name, in progress
    ([#1107](https://git.marcpardo.eu/marcpardo/zenkey/issues/1107));
  - #980's C3, the archive attachment, and C5, the compact descriptor.
- **The authoring format** and the canonical form a fingerprint is taken
  over. Neither moved from 0.21 to 0.22, but a contract that builds under
  0.22 may need editing under 0.23, and its fingerprint may change.

A 0.22.x patch release changes neither. The rules are the spec's
([`spec/core.md`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/main/spec/core.md),
its version in its header), and a release implements the spec as it
stood when it was cut.

## Zenoh's unstable API

This crate takes `zenoh` and `zenoh-ext` 1.10 with their `unstable`
feature, so your build's zenoh has it on too. The core (§0) has a
participant use only stable Zenoh API, and names the few places where
one opts into more (§2.4, §2.5); an opt-in never changes a key's shape.
This crate opts into each of them, and into nothing else:

| Unstable in zenoh 1.10.1 | Where | Why |
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
