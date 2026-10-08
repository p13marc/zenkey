# zk2 codegen (#611, chunk FI): design

Status: design for FI, 2026-10-08. It turns r4 §3.14's sketch into the
generated API, on the runtime as it landed in FE–FH (`zenkey` 0.20.0). The
spec (`spec/core.md`) stays the source of every rule; this document only says
how generated code reaches the runtime.

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

`generate()` fails the consumer's build, like v1's `zenkey_build`:
- **every lint error** of `zenkey-model` (stable codes; warnings go to
  `cargo:warning`);
- **with a history:** a candidate `breaking` against any published revision
  (`zenkey_model::compat::check_history`). `review` is a `cargo:warning`
  naming each finding; the CI binary (`zk2 contract compat`) is where review
  is accepted.

It emits `cargo:rerun-if-changed` for every contract and every schema file it
read.

## What is generated, per interface

One module per interface, named after it (`thruster.v1` → `thruster_v1`):

```rust
pub mod thruster_v1 {
    pub const IFACE: &str = "thruster.v1";
    pub const FINGERPRINT: &str = "sha256:…";
    /// The bundle (§9.6), built once at build time, byte for byte what
    /// `zk2 contract bundle` builds; never rebuilt at run time.
    pub static BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/thruster.v1.bundle.json"));
    pub fn implementation() -> zenkey::Implementation;   // from BUNDLE
    pub fn contract() -> std::sync::Arc<zenkey::model::contract::Contract>;

    pub mod types { /* protobuf: prost; JSON Schema: generated serde types or name hints */ }

    // Serving (an owner)
    pub trait Handlers: Send + Sync + 'static { /* one async fn per operation */ }
    pub struct Server { /* one typed writer per data resource */ }
    impl Server { pub async fn declare<H: Handlers>(b: &mut zenkey::ServiceBuilder, h: std::sync::Arc<H>) -> zenkey::Result<Self>; }

    // Consuming and calling (through a role, or a tool's explicit address)
    pub struct Consumer { /* typed subscribe / get per data resource */ }
    pub struct Client { /* one typed async fn per operation */ }
    pub trait Api { /* the Client's operations, for test doubles */ }
}
```

### Types

- **protobuf:** prost types, compiled from the bundle's own
  `FileDescriptorSet` (`prost_build::Config::compile_fds`), so the types and
  the bundle cannot disagree. No `protoc`.
- **JSON Schema:** a name hint (`json_type`) maps a type reference to an
  existing Rust type, which must be `Serialize + DeserializeOwned`; this is
  the Rust-first path (tcgui), where the schema was produced by schemars.
  `zenkey_build::check_schema::<T>(file, name)` is a test helper that fails
  when the committed schema no longer matches `T`'s schemars output, read
  through the subset (§7.3). Without a hint, a type is generated from the
  schema (`typify`).
- **raw:** `zenkey::bytes::ZBytes` (or `Vec<u8>`); the `Encoding` comes from
  the contract (§7.2), a raw family's from its `media_param` value.

The wire encoding is never the caller's choice: generated code encodes
protobuf with prost and JSON Schema types as the contract's `encoding` says
(JSON or CBOR, §7.2), through the runtime's writers.

### Serving

- **`Handlers`:** one `async fn` per operation, taking the decoded request
  and the runtime's `Call` (metadata, template values), returning the
  decoded response or an `OpError`. **Required operations are required trait
  methods**, so a missing one is a compile error. **Optional operations have a
  default method** that answers `OpError::unavailable(Cause::Build, …)`; the
  owner overrides it, or lists the resource unavailable with its cause.
  `replies = "many"` operations get a `Replies` sink (values, then the
  summary, O6).
- **`Server::declare`** declares, on the builder (before `start`, §8.2):
  - one typed writer per data resource of the contract that is required, or
    optional and not gated off: `Writer<T>` for streams, `StateWriter<T>`
    for state, `EventWriter<T>` for events; a templated resource becomes a
    method taking its parameters (`plans(vehicle)`), served as a template
    (`serve_state`, exposure recorded once);
  - one operation server per operation, wired to the handler.
  QoS and `Encoding` come from the contract through the runtime; nothing is
  configurable per call.
- `[defaults]` are already resolved in the contract model; `summary`
  replies follow `replies = "many"`.

### Consuming and calling

- **`Consumer::bind(&service, role)`:** the role's `zenkey::Consumer`
  (bindings from configuration, R1–R7), with typed `subscribe_<resource>`
  (decoded values, attributed to their provider) and typed `get_<state>`
  (S4; `StateGet` with decoded values) and `replay_<event>`.
- **`Client`:** from a role (`Client::bind(&service, role)`) or explicit
  providers (tools, `Client::new(&session, providers)`); one typed method per
  operation returning `Outcome<Resp>` (O5: value, refusal, malformed,
  silence); `Fleet` methods only for `fanout = "allowed"` operations.
- **`Api`:** a trait with the Client's operation methods. The generated
  `Client` implements it; a test implements it with a double. A component
  written against `impl Api` is testable without a bus.

## Contract crates (no zenoh by default)

`Config::contract_crate(true)` emits the same modules with every runtime
item behind `#[cfg(feature = "zenoh")]`: without the feature, a crate holds
the types, `BUNDLE`, `FINGERPRINT`, `IFACE` and the `Api`/`Handlers`
signatures in terms of its own types only, so an out-of-tree implementation
(zenoh-modem's `modem-contract` pattern) depends on it without zenoh.

## Runtime support this needs

- `zenkey_model::contract::Contract::from_bundle(&Bundle)`: the model from a
  bundle's canonical contract, so generated code (and tools, #612) build an
  `Implementation` from bytes alone. `Implementation::from_bundle(&[u8])`.
- Typed handles in `zenkey`: thin generic wrappers over `Writer`,
  `StateWriter`, `EventWriter`, `Consumer`, `Client` taking a codec
  (`Protobuf<T: prost::Message>`, `Json<T: Serialize + DeserializeOwned>`
  per the contract's encoding, `Raw`), so generated code stays small and the
  encoding logic lives in one place.

## Done when (#611)

- The walkthrough's and the tcgui pilot's contracts (`examples/zk2/`)
  generate and compile, in a test crate that uses them.
- The generated `Api` drives a unit test with a double.
- A contract crate builds with no zenoh dependency (`cargo tree` shows none
  without the feature).
- A breaking revision against a history fails the build; a review revision
  warns.
