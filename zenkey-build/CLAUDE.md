# zenkey-build

Claude Code loads this file when working in `zenkey-build/`; the root `CLAUDE.md` is the index.

## v1

v1's `zenkey-build` (0.11.x on crates.io) is the `v1` branch's, and that branch's
`CLAUDE.md` describes it. Nothing below applies to it.

## zk2

- **`zenkey-build/`** 0.20.0 (MIT, unpublished; v1's `zenkey-build` 0.11.x
  is the `v1` branch's, and no member of `main` uses it) is zk2's codegen
  (#611, FI; the design is `docs/zk2/codegen.md`). A consumer's build
  script runs `zenkey-model`'s lints and the history's compatibility gate
  (breaking fails, review warns), embeds each bundle as `zenkey-model` builds
  it, and generates one module per interface: prost types from the bundles'
  own descriptor sets (no `protoc`), typify types or a name hint for JSON
  Schema (`check_schema` keeps a hinted type and its schema in step), and
  `Handlers`/`Api` traits with `Server`/`Consumer`/`Client`/`Fleet` over
  `zenkey::typed`. `contract_crate(true)` puts the runtime items behind a
  `zenoh` feature. Its fixtures are workspace members:
  `zenkey-build/fixtures/codegen-test` (the walkthrough and tcgui contracts,
  on a bus and with an `Api` double, and `packed.v1` from
  `zenkey/tests/contracts/packed/`) and `fixtures/contract-crate` (no zenoh
  by default).
- **Packed types** (#979 B2, core §7.6, `src/packed.rs`): every type of
  each listed packed file is a struct under `__packed::<stem>` (an `int`
  `i64`, a `fixed` an exact `Decimal`, an `enum` a generated enum,
  `Coded<T>` with reserved codes, `Open<E>` with an escape, `Option` when
  optional or behind a marker), and a module per type holding its enums,
  its fixed-point statics and its `zenkey::packed::PackedRecord` impl:
  straight-line code, a statement per field in the layout's order, over
  the model's helpers, never the interpreted codec. The codec is
  `zenkey::codec::Packed<T>`. `codegen-test/tests/packed.rs` runs every
  vector of `spec/conformance/packed/vectors.json` through it (`Wide64`
  aside: E028 keeps it out of every contract).
- **Frame resources** (#981 K3, core §7.7): a frame resource's field on
  the generated `Server` is a `zenkey::typed::FrameWriter<Packed<T>>`
  (`emit::is_frame`), each record a value of its packed type, and the
  generated `Consumer`'s `subscribe_<name>` hands over each
  `zenkey::consumer::Delivery`, its `frame` the reader's reading, never a
  typed record. `codegen-test` generates `linkframes.v1`
  (`zenkey/tests/contracts/frames/`), and `tests/bus.rs`'s
  `frames_are_written_through_the_generated_code` runs it on a bus.
- **Packed operation types** (#985 C4c, core §5.1, §7.6): an operation's
  packed request, response, summary or error type is `Packed<T>` like any
  other (its `Handlers` method and `Api` call typed by it), carried in
  the forms behind its tag; the runtime reads and writes its `time`
  fields under the service's or client's deployment epoch. `codegen-test`
  generates `packedops.v1` (`zenkey/tests/contracts/packed-ops/`), and
  `tests/bus.rs`'s `packed_operations_are_served_and_called_in_the_forms`
  calls it under a catalog's epoch, its `app` refusal a packed `Mode`.
