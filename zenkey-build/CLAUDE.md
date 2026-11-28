# zenkey-build

Claude Code loads this file when working in `zenkey-build/`; the root `CLAUDE.md` is the index.

## v1

v1's `zenkey-build` (0.11.x on crates.io) is the `v1` branch's, and that branch's
`CLAUDE.md` describes it. Nothing below applies to it.

## zk2

- **`zenkey-build/`** 0.22.0 (MIT, on crates.io from 0.20.0, #929; v1's `zenkey-build` 0.11.x
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
  aside: E028 keeps it out of every contract). A packed type whose layout
  tag is 0 is `zenkey-model`'s E051 (core 0.117, #1152), a lint error the
  build fails on like any other (`tests/packed_tag.rs`); `Grow` moved to
  a maximum of 100 for it. A type config.v2 derives is set aside (core
  §7.6, F-619): `derive.json` case 2's contract, its `Record` at 0,
  generates.
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
- **config.v2** (#985 G2): `generate` runs `config::v2::lint_contract` on
  every contract that uses `config.v2`, a finding a lint error (v2.md
  §2.2); the derived types (`Record`, `Set<T>`, `Control`, `Error`) live in
  the contract's packed file, so they are generated as any packed type:
  a field keeps its packed name, `duty__writable` included (the
  `__packed` module allows `non_snake_case`), and a type's module never
  takes `error`, typify's own module in every JSON Schema module, which
  `types` re-exports beside it (`Error`'s is `error_2`).
  `codegen-test` generates the spec's `rfcfg.v1`, and `tests/config_v2.rs`
  reads `records.json`'s records and `validation.json`'s requests through
  them, agreeing with the model's codec, refusals included.
- **health.v2** (#986 Q2): `generate` runs `health::v2::lint_contract`
  beside desired.v2's (`table_lints`, both `zenkey_model::words::Lint`,
  printed by its `Display`): an invalid table of words in a contract
  that lists `health.v2` (`value`, `disagrees`; v2.md §2.10) is a lint
  error naming the resource and the key, and nothing is generated. No
  codegen moved: the tables are annotations, read at run time by the
  runtime (`Health::tables`). `tests/health_v2.rs` generates the
  scenarios' `tc.v1` and fails on a table that is no list of names, one
  with a name twice and two lists that differ.
- **alarms.v2** (#984 S1): `table_lints` runs `alarms::v2::lint_contract`
  too: an invalid `alarms.rules` in a contract that lists `alarms.v2`
  (§4's findings, `not_used` aside) is a lint error naming the resource
  and the key, and nothing is generated. Under `alarms.v1` the table is
  core W105, a warning, and the build goes on. No codegen moved: the
  table is an annotation the runtime reads. `tests/alarms_v2.rs`
  generates §4's example contract and fails on `rule_name`,
  `placeholder`, `labels_width`, `host_label`, `value` and `disagrees`.
- **desired.v2** (#986 J2): `generate` runs
  `desired::v2::lint_contract` on every contract, a finding a lint error
  (an invalid declaration of a desired resource, `horizon_v1` among its
  reasons, or of a report resource; v2.md §2.2, §2.10). No codegen
  moved: a target's contract lists the profile's `desired.v2.json` in
  `[schemas] packed`, so its `Report` is generated as any packed type
  (`types::Report`, `Open<report::Source>`, `Open<report::Outcome>`,
  `report::InForce`, eight `Option<i64>`, `Option<Open<report::Code>>`,
  two `Option<Coded<i64>>`, tag 12), its report resource a
  `StateWriter<Packed<Report>>` on the `Server`. It carries the record's
  fields; §2.10's consistency and the two tables are the model's.
  `tests/desired_v2.rs` generates the scenarios' `tasking.v2` and
  `worker.v2` from a scratch copy; `codegen-test` generates them from a
  mirror its `build.rs` makes under `OUT_DIR` (a `contracts_dir` takes a
  whole directory, and they live among the runtime's test contracts),
  and its `tests/desired_v2.rs` puts every record of
  `conformance/v2/reports.json` through the generated type, as the
  fixture says and as the model's codec reads it.
- **redundancy** (#987's N1): `generate` runs `redundancy::lint_contract`
  on every contract that loads, and each `Warning` is a `cargo:warning`
  (`<file>: warning[redundancy.vN <token>] <at>: <says>`), never a
  failure: a governed interface's invalid declaration of either major,
  `value` (each resource), `disagrees` (the interface) and, under
  `redundancy.v2`, `explicit_stream` (each `@stream`; v2.md §2.2). The
  runtime refuses to start such an interface, and the core has no code
  for any of them, so the text asks a linter to warn (v1 §2.2's SHOULD,
  unmet before N1). No codegen moved. `tests/redundancy_v2.rs` builds a
  clean `redundancy.v2` contract without a word, warns of its explicit
  stream (none under `redundancy.v1`), and of `value` under either major.
