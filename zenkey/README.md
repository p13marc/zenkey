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
zenkey = "0.21"
zenoh = "1.10"
tokio = { version = "1", features = ["full"] }

[build-dependencies]
zenkey-build = "0.21"
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

## Changes since 0.20

- **The wire breaks 0.20's: a 0.20 service and a 0.21 one do not
  understand each other's state GETs.** Move a deployment's owners and
  readers together.
  - An owner answers a state GET by snapshot (core 0.100,
    [#977](https://git.marcpardo.eu/marcpardo/zenkey/issues/977)): one per
    collection, in pages of `ServiceConfig::snapshot_page_bytes` (1,024 B
    unless set), a live entry with a horizon carrying its confirmation.
    Only a per-key resource still answers a reply per key.
  - Every state GET, the generated `Consumer`'s, `last_known`, the
    freshness re-read and an archive's alignment among them, is read under
    consolidation `None`. Its replies merge per key, and the omission rule
    applies over a complete snapshot (`state::StateAnswer::{snapshots,
    confirmations, omitted}`). A complete snapshot of no entry is
    `StateGet::Answered` with no key, never `Silent`. A `reply_del` on a
    member of a collection answered by snapshot is undecodable, never a
    deletion.
  - A call is read in its forms alone (core 0.99,
    [#985](https://git.marcpardo.eu/marcpardo/zenkey/issues/985)'s C4d). An
    owner refuses a request under any `Encoding` but `zenoh/bytes` with
    `invalid_request`, before any handler runs. A caller reads a value
    under another `Encoding` as malformed, and an error under one as the
    transport's, which is silence.
  - A `freshness.v2` heartbeat's time counts from the catalog's epoch, as
    a frame's does (`freshness.v2` 0.6,
    [#1080](https://git.marcpardo.eu/marcpardo/zenkey/issues/1080)).
- **Removed, and what to use instead:**
  - `call::Malformed`, `Outcome::Malformed`, `ReplyKind::Malformed`,
    `client::ErrorReply::Malformed` and `Replies::malformed`: a reply
    error that is no error form is `Transport`, silence.
  - `OpError::envelope`, `OpError::into_envelope`, `From<Envelope> for
    OpError` and `Refusal::of_envelope`: `OpError::{code, message, cause,
    detail}`. `Refusal` holds `code`, `message`, `cause` and `detail` as
    its own fields, where it dereferenced to an envelope.
  - `Answer::record_encoding` and `Call::record_encoding`: a record has
    no `Encoding` of its own now. `Answer::record` fails with a
    `FormRefusal`, `ReplyForm::Bare` is `ReplyForm::NoForm`, and
    `Call::request_tag` is a `Tag`, no longer an `Option`.
  - `heartbeat::EPOCH`: the epoch is the catalog's, given by
    `ServiceBuilder::catalog` (`Service::deployment_epoch`), and 0 with
    none.

## What 0.21 does not promise

zk2c, the bandwidth-first wire (epic
[#975](https://git.marcpardo.eu/marcpardo/zenkey/issues/975)), is not
finished. Two things under 0.21 are still moving, and a 0.22 may break
either:

- **The wire.** An event's key is to end in its run and seq in place of
  a ULID ([#980](https://git.marcpardo.eu/marcpardo/zenkey/issues/980)'s
  C2), and the rest of #975 follows. A 0.21 service and a 0.22 one may
  not understand each other.
- **The authoring format** and the canonical form a fingerprint is taken
  over. Neither moved from 0.20 to 0.21, but a contract that builds under
  0.21 may need editing under 0.22, and its fingerprint may change.

A 0.21.x patch release changes neither. The rules are the spec's
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
