# zk2 spec

The language-neutral half of zk2: what any implementation, in any language,
reads and must agree on. The design of record is
[`docs/zk2/architecture.md`](../docs/zk2/architecture.md) (r4), and the
measurements behind it are [`docs/zk2/spike-report.md`](../docs/zk2/spike-report.md).

| Path | What it is | Source |
|---|---|---|
| [`core.md`](core.md) | **The normative core**, version 0.23 (0.1 accepted on 2026-10-08, #606; amended for U23, the classifier's rule set, the TOML 1.0 lint, the second implementation's findings, its findings against 0.5 with the archive's gaps, its live findings with the operations runtime's and the codegen's gaps, a refused presence read with what implementing 0.7 found, the order of an owner's refusals, what a doctor can and cannot decide, how a zid compares, who may answer the admin space, how a far router is verified, what access control measured, what §11 needs to be built from, what the tools' last verbs could not decide, what a tool needs that it cannot read off the bus, two words 0.17 left loose, profiles that only derive, a provider on the service's own system with the order of `profiles`, the first published vocabulary with a re-put, one major of a profile per contract, and a clock ahead reported by a stream with the first standard contract a profile publishes). Every MUST cites a fixture or a scenario. | written from r4 |
| [`contract.schema.json`](contract.schema.json) | JSON Schema (2020-12) of the contract authoring format, draft 1. It is generated from `zenkey-model`'s authoring types and checked in CI. | `zenkey-model/src/authoring.rs` |
| [`descriptor.schema.json`](descriptor.schema.json) | JSON Schema of the descriptor record (core §3.3), generated the same way | `zenkey-model/src/descriptor.rs` |
| [`core/`](core/) | The error envelope's definitions: `error.proto`, `error.schema.json` (core §5.2) | |
| [`conformance/`](conformance/) | Fixtures: keys, slugs, templates, contract lints, canonical forms, fingerprints, bundles, set checks, history, descriptors, error envelopes, the compatibility matrix. Seeded by `zenkey-model` (#608), completed by #607. | `zenkey-model/tests/conformance.rs` |
| [`scenarios/`](scenarios/) | Network rules that a fixture cannot check: setup, steps, expected observations. | the spike's runs |
| [`profiles/`](profiles/) | The profiles (core §10), one directory each, with its own text version, changelog, fixtures and scenarios. The index, the template and the process are in [`profiles/README.md`](profiles/README.md). First: `hostid.v1`, text 0.3, draft; then `freshness.v1`, text 0.2, draft; then `health.v1`, text 0.3, draft, with its standard contract and `.history/`. | `zenkey-model/tests/profiles.rs` |
| [`CHANGELOG.md`](CHANGELOG.md) | Amendments after version 0.1 is accepted | |

**License.** The spec, its fixtures and its scenarios ship under the
repository's MIT license, like `zenkey-model`.

**Process.**
- **Since acceptance (0.1, 2026-10-08),** every change goes through
  `CHANGELOG.md`, amendment-style. Each entry records what changed, and what
  deliberately did not.
- **Fixtures land with their rule.** A machine-testable normative rule
  arrives with its fixture or scenario in the same change.
- **Lessons go to guides, not MUSTs.**

To regenerate the schema and fixtures after an intended change, run the
command below, then read the diff: a bless claims that every changed
expectation is right.

```bash
ZK2_BLESS=1 cargo test -p zenkey-model --test conformance
```
