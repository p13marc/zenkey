# zenkey-model

The session-free half of **zk2** (epic #585, issue #608): everything about
keys and contracts that can be decided from files, with no Zenoh session.
Build scripts, CI tools and the conformance runners stand on it.

- **Keys** (`grammar`, `template`, `slug`): the `zk2/` grammar as typed
  values over `zenoh_keyexpr::OwnedKeyExpr`, resource templates with
  most-literal-first precedence (r3.3 D1), and v1's injective slugging.
- **Contracts** (`authoring`, `contract`, `diag`): the TOML authoring format,
  draft 1 (`examples/zk2/README.md`), with defaults expanded, types resolved
  (protobuf through `protox`, JSON Schema, raw media types), and every lint
  reported with a stable code.
- **Identity** (`canonical`, `bundle`, `history`): the canonical JSON form,
  its fingerprint, the bundle container verified Merkle-style, and the
  append-only `.history` of published revisions.

The design of record is `docs/zk2/architecture.md` (r3.3).
`spec/contract.schema.json` and the `spec/conformance/` fixtures come from
this crate.

```bash
cargo run -p zenkey-model --example zk2-check -- examples/zk2/walkthrough/nav.v2.toml
```

MIT. Not on crates.io until the zk2 core spec is accepted (#606).
