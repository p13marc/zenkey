# zk2 codegen (#611, chunk FI): design

Status: design for FI, 2026-10-08, revised as built (the last section lists
what the implementation changed, and why). It turns r4 §3.14's sketch into
the generated API, on the runtime as it landed in FE–FH (`zenkey` 0.20.0).
The spec (`spec/core.md`) stays the source of every rule; this document only
says how generated code reaches the runtime.

## The build script

```rust
// build.rs
zenkey_build::Config::new()
    .contracts_dir("contracts")              // *.toml, the authoring format (spec §9.1)
    .history_dir("contracts/.history")       // optional: the compatibility gate (§9.7, §9.8)
    .json_type("json:Status", "crate::model::Status") // optional name hints (G23)
    .generate()?;
```

```rust
// src/zk2.rs
include!(concat!(env!("OUT_DIR"), "/zk2.rs"));
```

`contracts_dir` is not recursive and may be called once per directory
(`*.bindings.toml` deployment files are skipped); `out_file` moves the
generated file from `$OUT_DIR/zk2.rs`. The embedded bundles and the
generated types go beside it, in `zk2.d/`.

`generate()` fails the consumer's build, like v1's `zenkey_build`:
- **every lint error** of `zenkey-model` (stable codes; warnings go to
  `cargo:warning`), and the set's checks (E035, E036);
- **with a history:** a candidate `breaking` against any published revision
  (`zenkey_model::compat::check_history`). `review` is a `cargo:warning`
  naming each finding; the CI binary (`zk2 contract compat`) is where review
  is accepted. A history that does not verify fails too.

It emits `cargo:rerun-if-changed` for every contract, every schema file and
proto include root it names, the contracts directories and the history.

## What is generated, per interface

One module per interface, named after it (`thruster.v1` → `thruster_v1`):

```rust
pub mod thruster_v1 {
    pub const IFACE: &str = "thruster.v1";
    pub const FINGERPRINT: &str = "sha256:…";
    /// The bundle (§9.6), built once at build time, byte for byte what
    /// `zk2 contract bundle` builds; never rebuilt at run time.
    pub static BUNDLE: &[u8] = include_bytes!("…/zk2.d/thruster.v1.bundle.json");
    pub fn iface() -> zenkey::model::grammar::IfaceId;
    pub fn implementation() -> zenkey::Implementation;   // Implementation::from_bundle(BUNDLE)
    pub fn contract() -> std::sync::Arc<zenkey::model::contract::Contract>;

    pub mod types { /* protobuf: prost; JSON Schema: generated serde types or name hints */ }
    pub mod resource { pub const ARM: &str = "@op/arm"; /* … */ }   // descriptor names
    pub mod role { pub const CMD: &str = "cmd"; }                 // the contract's requirements

    // Serving (an owner)
    pub trait Handlers: Send + Sync + 'static { /* one async fn per operation */ }
    pub struct Server { /* one typed writer per data resource */ }
    impl Server { pub async fn declare<H: Handlers>(b: &mut zenkey::ServiceBuilder, h: std::sync::Arc<H>) -> zenkey::Result<Self>; }

    // Consuming and calling (through a role, or a tool's explicit address)
    pub struct Consumer { /* typed subscribe / get / replay / last_known per data resource */ }
    pub struct Client { /* implements Api */ }
    pub trait Api { /* the Client's operations, for test doubles */ }
    pub struct Fleet { /* only when an operation allows fan-out */ }
}
```

`Handlers`, `Api`, `Client` and `Fleet` exist only when the contract has
operations (`Fleet` only when one is `fanout = "allowed"`); an interface with
none has `Server::declare(b)`.

### Types

- **protobuf:** prost types, compiled from the bundle's own
  `FileDescriptorSet` (`prost_build::Config::compile_fds`), so the types and
  the bundle cannot disagree. No `protoc`. Messages derive through
  `zenkey::prost`, so their `prost` is the codec's; the well-known types are
  prost's (`google.protobuf.Empty` is `()`, the others `zenkey::prost_types`).
  One `__proto` tree holds every package; two contracts carrying one `.proto`
  file name with different content cannot share it, and fail the build.
- **JSON Schema:** a name hint (`json_type`) maps a type reference
  (`json:<Name>`, or `json:<stem>#<Name>` for one file) to an existing Rust
  type, which must be `Serialize + DeserializeOwned`; this is the Rust-first
  path (tcgui), where the schema was produced by schemars.
  `zenkey_build::check_schema::<T>(file, name)` is a test helper that fails
  when the committed schema no longer matches `T`'s schemars output, read
  through the subset (§7.3): annotations ignored, local `$ref`s followed, a
  `type` that an `enum` implies dropped, and a keyword schemars emits outside
  the subset refused. Without a hint, a type is generated from the schema
  (`typify`, whose type replacement is the hint). The files a contract lists
  form one group and one module (`__json::<stems>`), with cross-file `$ref`s
  resolved by stem as in a bundle; contracts listing the same files share
  it, so the tcgui pilot's interfaces share one `TcError`. A `$defs` name two
  files of a group define is qualified by its stem. `format`, an annotation
  in the subset, is not read: a string stays a `String`. A hint that names
  no definition is a `cargo:warning`.
- **raw:** `Vec<u8>` in the typed API; the untyped writer stays reachable
  (`inner()`) for a zero-copy `ZBytes` (SHM, §7.4). The `Encoding` comes
  from the contract (§7.2), a raw family's from its `media_param` value.

The wire encoding is never the caller's choice: generated code encodes
through a codec (`zenkey::codec::{Protobuf<T>, Json<T>, Raw, Nothing}`):
protobuf with prost, JSON Schema types as the contract's `encoding` says (JSON
or CBOR, §7.2), through the runtime's writers. Decoding follows §7.2's order:
the sample's `Encoding`, then the contract's.

### Serving

- **`Handlers`:** one `async fn` per operation, taking the decoded request
  and a `zenkey::CallInfo` (the template values, the claimed metadata of O7,
  whether the call was a fan-out; with the `zenoh` feature, the runtime's
  `Call` through `CallInfo::call()`), returning the decoded response or an
  `OpError`. **Required operations are required trait methods**, so a
  missing one is a compile error. **Optional operations have a default
  method** that answers `OpError::unavailable(Cause::Build, …)`; the owner
  overrides it, or lists the resource unavailable with its cause before
  `Server::declare`. `replies = "many"` operations get a
  `zenkey::Sink<Resp, Summary>` (values, then the summary, O6; `()` when no
  summary is declared). An `app` error carries a value of the declared
  `error` type through its codec: `OpError::app_as::<Protobuf<NavError>>(…)`.
- **`Server::declare`** declares, on the builder (before `start`, §8.2),
  after implementing the interface if the builder does not yet:
  - one typed writer per data resource without parameters that is
    required, or optional and not absent here (`ServiceBuilder::is_absent`:
    gated on a capability not held, or listed unavailable), as a field:
    `zenkey::typed::Writer<C, A>` for streams (`A` the declared attachment),
    `StateWriter<C>` for state, `EventWriter<C>` for events; an optional one
    is an `Option`;
  - for a templated resource, exposure (and `serve_state` for state)
    recorded once, and a method taking its parameters that declares a
    member on the running service: `server.plans(&mut svc, "vehicle-01")`
    for state (`Service::state_writer`), `&svc` for streams and events;
  - one operation server per operation not absent here, wired to the
    handler over the whole template (`zenkey::typed::serve_one`,
    `serve_many`).
  QoS and `Encoding` come from the contract through the runtime; nothing is
  configurable per call.
- `[defaults]` are already resolved in the contract model; `summary`
  replies follow `replies = "many"`.

### Consuming and calling

- **`Consumer::bind(&service, role)`:** the role's `zenkey::Consumer`
  (bindings from configuration, R1–R7), with typed `subscribe_<resource>`
  (decoded values, attributed to their provider; a sample that does not
  decode is delivered as such, with the reason), `get_<state>` (S4; a typed
  `StateGet`) and `get_<state>_member(<params>)` for a templated state,
  `last_known_<state>(archive, provider, <params>)` (the archive client,
  S5, #620), and `replay_<event>`.
- **`Client`:** from a role (`Client::bind(&service, role)`) or explicit
  providers (tools, `Client::new(&session, providers)`, the session named
  through `zenkey::zenoh`); one typed method per operation, the `Api`
  trait's, returning `zenkey::Result<Outcome<Resp>>` (O5: value, refusal,
  malformed, silence) or, for `replies = "many"`, `Result<Replies<Resp,
  Summary>>`. A value reply that does not decode as its type is the
  replier's bug: `Error::Payload`, never silence. `Fleet` methods, only for
  `fanout = "allowed"` operations, take each parameter as an `Option`
  (`None` selects every member).
- **`Api`:** a trait with the Client's operation methods (`at` the concrete
  address, then the template's parameters, then the request). The generated
  `Client` implements it; a test implements it with a double. A component
  written against `impl Api` is testable without a bus.

## Contract crates (no zenoh by default)

`Config::contract_crate(true)` emits the same modules with every runtime
item (`Server`, `Consumer`, `Client`, `Fleet`) behind
`#[cfg(feature = "zenoh")]`. The crate depends on `zenkey` with
`default-features = false` and forwards `zenoh = ["zenkey/zenoh"]`. Without
the feature, it holds the types, `BUNDLE`, `FINGERPRINT`, `IFACE`,
`implementation()`, `contract()` and the `Api`/`Handlers` signatures in terms
of its own types and `zenkey`'s session-free vocabulary (`zenkey::call`), so
an out-of-tree implementation (zenoh-modem's `modem-contract` pattern)
depends on it without zenoh: its normal tree holds `zenoh-keyexpr` (and
`zenoh-result`), as `zenkey-model`'s does, and no other zenoh crate.

## Runtime support this needs

- `zenkey_model::contract::Contract::from_bundle(&Bundle)`: the model from a
  bundle's canonical contract, so generated code (and tools, #612) build an
  `Implementation` from bytes alone; `Bundle::build` of the result is the
  bundle, byte for byte. `Implementation::from_bundle(&[u8])` verifies the
  bytes and requires them in JCS form, so they are served as built.
- `zenkey`'s **`zenoh` feature** (default): everything holding a session.
  Without it, `zenkey` is the model, `config`, `Implementation`, `call` (the
  session-free vocabulary: `OpError`, `CallMetadata`, `CallInfo`, `Sink`,
  `Outcome<T>`, `Replies<V, S>`) and `codec`.
- **Typed handles in `zenkey::typed`:** thin generic wrappers over
  `Writer`, `StateWriter`, `EventWriter`, the consumer's reads, `Client`,
  `Fleet` and `ServiceBuilder::serve`, taking a codec, so generated code
  stays small and the encoding logic lives in one place. Each checks its
  codec against the contract's type kind at declaration.

## Done when (#611)

- The walkthrough's and the tcgui pilot's contracts (`examples/zk2/`)
  generate and compile, in a test crate that uses them
  (`zenkey-build/fixtures/codegen-test`).
- The generated `Api` drives a unit test with a double.
- A contract crate builds with no zenoh dependency (`cargo tree` shows none
  without the feature; `zenkey-build/fixtures/contract-crate`).
- A breaking revision against a history fails the build; a review revision
  warns (`zenkey-build/tests/gate.rs`).

## What the implementation changed (2026-10-08)

| Design said | Built | Why |
|---|---|---|
| Handlers take the runtime's `Call` | they take `zenkey::CallInfo`; the `Call` is `CallInfo::call()` with the `zenoh` feature | `Call` holds a zenoh `Query`, and a contract crate's `Handlers` must build without zenoh |
| `OpError`, `Outcome`, `Replies` are the runtime's | they moved to `zenkey::call`, session-free and generic (`Outcome<T>`, `Replies<V, S>`); `client::Outcome`, `Replies`, `Replier` are aliases of their `Answer` instantiations | the `Api` signatures need them without zenoh; callers of the runtime are unchanged |
| a contract crate without zenoh | `zenkey` gained a default `zenoh` feature | the session-free vocabulary has one home, shared by every contract crate, as v1's `zenkey` did with `default-features = false` |
| raw is `ZBytes` (or `Vec<u8>`) | `Vec<u8>`, the untyped writer reachable | `ZBytes` is zenoh's; the zero-copy path stays through `inner()` |
| `Server` holds a writer per data resource; a templated one is a method | a templated resource's method takes the running `Service` | its members appear after start, and the runtime's member writers are the `Service`'s |
| `Consumer` has `subscribe`, `get`, `replay` | also `get_<state>_member` and `last_known_<state>` | a templated state is read by member, and the issue's re-plan asks for the archive client |
| generated code names `prost`, `zenoh` | through `zenkey::prost`, `zenkey::prost_types`, `zenkey::zenoh` | the codec's prost and the generated messages' are then the same crate; a consumer needs no zenoh dependency of its own |
