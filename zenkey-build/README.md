# zenkey-build

Registry codegen for the [zenkey](https://crates.io/crates/zenkey) keyspace-v2
convention (RFC 08): lint your application's `registry/*.toml` (or
`*.kdl`, RFC 08 §5.1 — one directory may mix the two, file by file) subject
vocabulary and generate typed subject/procedure builders and parsers, from
your build script.

```rust
// build.rs
fn main() {
    zenkey_build::Config::new()
        .registry_dir("registry")
        .generate()
        .unwrap();
}
```

```rust
// src/registry.rs
include!(concat!(env!("OUT_DIR"), "/zenkey_registry.rs"));
```

The generated module gives you, per producer/service:

- a `Subject` enum with typed constructors (an unregistered subject does not
  construct) and a precedence-ordered parser (`Subject::parse`,
  `Subject::parse_metric`) — the RFC 08 §1 contract, in both directions;
- a `ProcedureId` enum with `@rpc` key builders;
- the raw registry file (`REGISTRY_SOURCE`) served verbatim by the
  `introspect` procedure, and the `Encoding` that reply declares
  (`REGISTRY_ENCODING`: `application/toml` or `application/kdl`, RFC 08 §6);

plus the cross-producer `AnySubject` dispatch (`parse_subject`,
`AnySubject::common_state()` driven by `common = "..."` registry fields),
`REGISTRIES`, `registry_source()`, `registry_encoding()`, and
`is_registered_telemetry()`. (`REGISTRY_TOML` and `registry_toml()` remain
as deprecated aliases.)

Lints (RFC 08 §5) and the append-only deprecation ledger check (RFC 08 §3)
fail your build with the offending file and rule named.

The convention itself — grammar, planes, identity, registry, operations — is
specified in the [zenkey repository](https://github.com/p13marc/zenkey)'s
`rfcs/`.

## License

MIT.
