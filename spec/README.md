# zk2 spec

The language-neutral half of zk2: what any implementation, in any language,
reads and must agree on. The design of record is
[`docs/zk2/architecture.md`](../docs/zk2/architecture.md) (r4), and the
measurements behind it are [`docs/zk2/spike-report.md`](../docs/zk2/spike-report.md).

## Glossary

The words the texts lean on, each with the section that defines it. A
section number alone is the core's.

- **System**: the deployment's unit of ownership and outer policy
  boundary, a key's second chunk ([§1.5](core.md#15-identity)).
- **Service**: a logical, stable unit that owns its keys, a key's third
  chunk ([§1.5](core.md#15-identity)).
- **Interface**: a contract's name and major, such as `nav.v2`, a key's
  fourth chunk ([§1.5](core.md#15-identity)).
- **Instance**: one run of a service, named by a random 64-bit id
  ([§1.5](core.md#15-identity)).
- **Epoch**: a span of continuity, which an instance id names. An owner
  starts a new one whenever its counters reset ([§1.5](core.md#15-identity),
  [§4.3](core.md#43-clocks-s7)).
- **Resource**: what an interface provides: a stream, a state, an event or
  an operation ([§2.1](core.md#21-patterns)).
- **Kind**: which of the four a resource is, and the key token that says
  so ([§1.3](core.md#13-kind-tokens)).
- **Explicit**: a stream or state under a verbatim token, `@stream` or
  `@state`, which no ambient selector reaches ([§1.3](core.md#13-kind-tokens)).
- **Ambient selector**: a selector with no verbatim chunk, such as
  `zk2/**` ([§1.3](core.md#13-kind-tokens)).
- **Template**: a resource's path below its kind chunk, with parameters
  such as `{axis}` ([§2.2](core.md#22-templates)).
- **Member**: one value of a template's parameters, which is one key for a
  stream or a state ([§2.7](core.md#27-the-population-budget)).
- **Cardinality**: a template's bound on its members in one instance, or
  `"unbounded"` ([§2.2](core.md#22-templates)).
- **Contract**: a TOML file declaring one interface major: its resources,
  types and requirements ([§9.1](core.md#91-the-authoring-format)).
- **Fingerprint**: `sha256:` and the hash of a contract's canonical form
  ([§9.5](core.md#95-the-canonical-form-and-the-fingerprint)).
- **Revision**: one published contract within its major, named by its
  fingerprint ([§9.7](core.md#97-history-and-retention),
  [§9.8](core.md#98-compatibility)).
- **Bundle**: a canonical contract with its schema artifacts. A tool
  retrieves it from the bus and checks its fingerprint
  ([§9.6](core.md#96-bundles), [§8.4](core.md#84-contract-retrieval)).
- **Descriptor**: the JSON record every instance serves at its instance key
  ([§3.3](core.md#33-the-descriptor-record)).
- **Requirement**: a named role, typed by an interface, through which data
  reaches a consumer ([§3.1](core.md#31-requirements)).
- **Role**: a requirement's name, which a binding binds
  ([§3.1](core.md#31-requirements)). The roles a rule binds, owner to tool,
  are another sense ([§0](core.md#0-conventions)).
- **Binding**: the ordered list of providers a deployment configures for a
  role ([§3.2](core.md#32-binding-rules)).
- **Provider**: a service a binding names, implementing the required
  interface ([§3.2](core.md#32-binding-rules)).
- **Token**: a liveliness token, which carries no payload. Its key says who
  is alive ([§8.1](core.md#81-tokens)).
- **Presence**: the tokens and descriptors that say which instances run,
  and what they implement
  ([§8](core.md#8-presence-descriptors-contract-retrieval)).
- **Re-mint**: an owner taking a new instance id while running, new tokens
  before the old go ([§8.1](core.md#81-tokens)).
- **Tokenless**: an interface in the deployment's tokenless set, for which
  an owner holds no interface token ([§8.1](core.md#81-tokens)).
- **Standby**: an instance that implements an interface and exposes none of
  it, its descriptor saying so ([§6](core.md#6-ownership-and-serving)).
- **Stamp**: the timestamp an owner sets on every state mutation, from
  Zenoh's HLC ([§4.3](core.md#43-clocks-s7)).
- **Late joiner**: a consumer that subscribes to a state, then asks its
  owner, and merges both by stamp
  ([§4.2](core.md#42-rules-for-owners-and-consumers)).
- **Tombstone window**: how long an owner answers a GET of a key it deleted
  with the delete, 60 s by default
  ([§4.2](core.md#42-rules-for-owners-and-consumers)).
- **Archive**: a service that records owners' state and serves it as
  last-known, never as current ([§4.4](core.md#44-archives)).
- **Face**: a router's link to a part of the network. A constrained one
  carries little ([§8.5](core.md#85-constrained-faces)).
- **Watch session**: a reader's session that holds its liveliness
  subscribers and makes no read
  ([observer §3.7](observer.md#37-the-watch-session)).
- **Round trip**: a GET on the watch session whose answer is discarded. It
  follows the subscribers' declarations
  ([observer §3.7](observer.md#37-the-watch-session)).
- **Complete read**: a presence read that ended at its routers' final reply,
  with no error reply. Only a complete read shows absence
  ([observer §3.3](observer.md#33-possibly-incomplete),
  [§3.6](observer.md#36-absence)).
- **Profile**: a specification versioned on its own that extends the core,
  such as `timing.v1` ([§10](core.md#10-extension-points-for-profiles)).
- **Class**: the rules of one role, with the static rules. An
  implementation claims one per role it plays
  ([§12](core.md#12-conformance-classes)).

## What is here

| Path | What it is | Source |
|---|---|---|
| [`core.md`](core.md) | **The normative core.** Its header gives its version, the date its first version was accepted, and a link to [`CHANGELOG.md`](CHANGELOG.md), which records every amendment since. Every MUST cites a fixture or a scenario. | written from r4 |
| [`observer.md`](observer.md) | **The observer rules**, normative beside the core: what a tool, or any participant reading presence, state or the admin space, does with what it reads, when a read is complete, and what it may conclude and tell a person. Its header gives its text version, its status and the core version it is written against, and links [`observer-CHANGELOG.md`](observer-CHANGELOG.md). It holds the reader's discipline the core moved out, and the judgement shape and observer obligations that [`docs/zk2/tooling-guide.md`](../docs/zk2/tooling-guide.md) carried, promoted where they cite evidence. Its evidence is the core's fixtures and scenarios, and the profiles'. | moved from the core, and the tooling guide |
| [`RULES.md`](RULES.md) | **The rule index**, generated from `core.md` and `observer.md`: every MUST, MUST NOT, SHOULD and SHOULD NOT, by the role it binds, then by section, with its rule id and its evidence. A stream-only owner's minimum path is its owner and implementation entries. CI fails when it is stale. | `scripts/gen-rules-index.py` |
| [`contract.schema.json`](contract.schema.json) | JSON Schema (2020-12) of the contract authoring format, draft 2. It is generated from `zenkey-model`'s authoring types and checked in CI. | `zenkey-model/src/authoring.rs` |
| [`descriptor.schema.json`](descriptor.schema.json) | JSON Schema of the descriptor record (core §3.3), generated the same way | `zenkey-model/src/descriptor.rs` |
| [`core/`](core/) | The error envelope's definitions: `error.proto`, `error.schema.json` (core §5.2) | |
| [`conformance/`](conformance/) | Fixtures: keys, slugs, templates, contract lints, canonical forms, fingerprints, bundles, set checks, history, descriptors, error envelopes, the compatibility matrix, the population budget, a late joiner's merge. | `zenkey-model/tests/conformance.rs` |
| [`scenarios/`](scenarios/) | Network rules that a fixture cannot check: setup, steps, expected observations. | the spike's runs |
| [`profiles/`](profiles/) | The profiles (core §10), one directory each, with its own text version, changelog, fixtures and scenarios. They are listed, each with its text version, status, who uses it and the core version it is written against, in [the index](profiles/README.md#index), those nobody uses yet (`experimental`) in a table of their own, beside the template and the process. | `zenkey-model/tests/profiles.rs` |
| [`CHANGELOG.md`](CHANGELOG.md) | The amendments to `core.md` since its first accepted version, newest first | |
| [`observer-CHANGELOG.md`](observer-CHANGELOG.md) | The versions of `observer.md`'s text, newest first | |

**License.** The spec, its fixtures and its scenarios ship under the
repository's MIT license, like `zenkey-model`.

**Process.**
- **Since the core was accepted (2026-10-08),** every change goes through
  `CHANGELOG.md`, amendment-style. Each entry records what changed, and what
  deliberately did not. `observer.md`'s changes go through
  `observer-CHANGELOG.md` the same way.
- **Fixtures land with their rule.** A machine-testable normative rule
  arrives with its fixture or scenario in the same change.
- **Lessons go to guides, not MUSTs.**

To regenerate the schema and fixtures after an intended change, run the
command below, then read the diff: a bless claims that every changed
expectation is right.

```bash
ZK2_BLESS=1 cargo test -p zenkey-model --test conformance
```
