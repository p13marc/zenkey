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
- **Instance**: one run of a service ([§1.5](core.md#15-identity)),
  named by an instance id of 64 random bits
  ([§1.2](core.md#12-lexical-rules)).
- **Epoch**: a span of continuity, which an instance id names. An owner
  starts a new one whenever its counters reset or its state's ordering
  restarts ([§1.5](core.md#15-identity)). §1.5 asks for a new session, so
  that its stamps' id changes too. With no record of its last stamp, §4.3
  requires a fresh zid ([§4.3](core.md#43-clocks-s7)).
- **Resource**: what an interface provides: a stream, a state, an event or
  an operation ([§2.1](core.md#21-patterns)).
- **Kind**: which of the four a resource is, and the key token that says
  so ([§1.3](core.md#13-kind-tokens)).
- **Explicit**: a stream or state under a verbatim token, `@stream` or
  `@state`, which no ambient selector reaches ([§1.3](core.md#13-kind-tokens)).
- **Ambient selector**: a selector with no verbatim chunk, such as
  `zk2/**`. It reaches no `@stream`, `@state` or `@op` key, and no control
  key ([§1.3](core.md#13-kind-tokens)).
- **Template**: a resource's path below its kind chunk, with parameters
  such as `{axis}` ([§2.2](core.md#22-templates)).
- **Member**: one value of a template's parameters. That is a key, for a
  stream or a state; for an event, its key without the ULID chunk
  ([§2.7](core.md#27-the-population-budget)).
- **Cardinality**: a template's bound on its members in one instance, or
  `"unbounded"` ([§2.2](core.md#22-templates)).
- **Contract**: a TOML file declaring one interface major: its resources,
  types and requirements ([§9.1](core.md#91-the-authoring-format)).
- **Fingerprint**: `sha256:` and the hash of a contract's canonical form
  ([§9.5](core.md#95-the-canonical-form-and-the-fingerprint)).
- **Revision**: one version of a contract within its major, named by its
  fingerprint. A breaking one is never published
  ([§9.7](core.md#97-history-and-retention),
  [§9.8](core.md#98-compatibility)).
- **Bundle**: a canonical contract with its schema artifacts and the extra
  documents it references. A tool retrieves it from the bus and checks its
  fingerprint ([§9.6](core.md#96-bundles),
  [§8.4](core.md#84-contract-retrieval)).
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
- **Presence**: the liveliness tokens that say which instances run, and
  what they expose ([§8.1](core.md#81-tokens)). An instance's descriptor
  says what it implements ([§3.3](core.md#33-the-descriptor-record)).
- **Re-mint**: an owner taking a new instance id while running, new tokens
  before the old go ([§8.1](core.md#81-tokens)). A restart is a re-mint
  too ([§2.7](core.md#27-the-population-budget)).
- **Tokenless**: an interface in the deployment's tokenless set, for which
  an owner holds no interface token ([§8.1](core.md#81-tokens)).
- **Standby**: an instance that implements an interface and exposes none of
  it, its descriptor saying so ([§6](core.md#6-ownership-and-serving)).
- **Stamp**: the timestamp an owner sets on every state mutation. It is
  the greater of `Session::new_timestamp()` and its last stamp plus one
  tick. At the constrained level, a wall clock with a per-session bump
  replaces the HLC ([§4.3](core.md#43-clocks-s7)).
- **Late joiner**: a consumer or a tool that subscribes to a state, then
  asks its owner, and merges both by stamp. Across two stamp ids, a
  consumer keeps what §4.3's consumer rule accepts
  ([§4.2](core.md#42-rules-for-owners-and-consumers)). A tool keeps the
  merge for as long as it follows ([observer §4.1](observer.md#41-a-seed)).
- **Tombstone window**: how long an owner answers a GET of a key it deleted
  with the delete, 60 s by default
  ([§4.2](core.md#42-rules-for-owners-and-consumers)).
- **Archive**: a service that records owners' state and serves it as
  last-known, never as current ([§4.4](core.md#44-archives)).
- **Face**: a router's link to a part of the network. A deployment marks
  one constrained, and gives its maximum outage
  ([§8.5](core.md#85-constrained-faces)).
- **Watch session**: a reader's session that holds its liveliness
  subscribers and makes no read
  ([observer §3.7](observer.md#37-the-watch-session)).
- **Round trip**: a liveliness GET on the watch session whose answer is
  discarded. It follows the subscribers' declarations
  ([observer §3.7](observer.md#37-the-watch-session)).
- **Complete read**: a presence read that ended at its routers' final reply,
  with no error reply, before its session's interests timeout
  ([observer §3.3](observer.md#33-possibly-incomplete)). Its session held a
  link throughout ([§3.5](observer.md#35-a-session-cut-off)). A reader is
  asked to make it on a session that never held a liveliness subscriber.
  A reader with a watch session made it while a round trip stood
  ([§3.7](observer.md#37-the-watch-session)). It covers the
  tokens of its routers' clients and its linked peers
  ([§3.9](observer.md#39-what-a-complete-read-covers)). Only a complete read
  shows absence ([§3.6](observer.md#36-absence)).
- **Profile**: a specification versioned on its own that extends the core,
  such as `timing.v1` ([§10](core.md#10-extension-points-for-profiles)).
- **Class**: the rules of one role, with the static rules. An
  implementation claims one per role it plays. A profile is a class too
  ([§12](core.md#12-conformance-classes)).
- **Reader**: any participant that reads presence, state or the admin
  space, and concludes from it. A tool is one, and so is a consumer, a
  caller or an owner where it reads
  ([observer §0](observer.md#0-conventions)).
- **Static rule**: a rule that binds every implementation where it does
  what the rule governs. Those are §1's, §9's, those on what a contract
  states in the sections §12 lists, any naming its lint code, and §3.3's
  checker's. A rule of those sections that names a role is that role's
  ([§12](core.md#12-conformance-classes)).
- **Expose**: an instance exposes a resource when it serves it, as its
  descriptor says ([§8.2](core.md#82-start-up-order)). It is **active** on
  an interface when it exposes one of its resources
  ([§5.1](core.md#51-rules)). An active instance holds the interface's
  token, unless the interface is tokenless ([§8.1](core.md#81-tokens)).
- **Possibly incomplete**: a read that may have missed a token. It had an
  error reply, or ran to its timeout or its session's interests timeout
  ([observer §3.3](observer.md#33-possibly-incomplete)). So is one
  cut off while it ran ([§3.5](observer.md#35-a-session-cut-off)), and one
  made while no round trip stood ([§3.7](observer.md#37-the-watch-session)).
- **Interests timeout**: a session's `routing/interests/timeout`, 10 s by
  default. zenoh ends an unfinished liveliness GET there, with no error
  reply ([observer §3.3](observer.md#33-possibly-incomplete)).
- **Move**: a watch session's transport closing or opening. It unconfirms
  what the reader holds ([observer §3.7](observer.md#37-the-watch-session)).
- **Seed**: a reader's first read of what it then follows. A late
  joiner's is its S4 GET, after its subscription
  ([§4.2](core.md#42-rules-for-owners-and-consumers),
  [observer §4.1](observer.md#41-a-seed)). A presence reader's is a
  liveliness read ([observer §3.7](observer.md#37-the-watch-session)).
- **History**: three senses. Advanced publication's cache of a stream or
  a state ([§2.5](core.md#25-advanced-publication)). The history root of
  published bundles ([§9.7](core.md#97-history-and-retention)). A
  liveliness subscriber's, the tokens it is told were held
  ([observer §3.7](observer.md#37-the-watch-session)).
- **Envelope**: what a failed call's `reply_err` carries: a code and a
  message. `unavailable` adds a cause, and `app` may add a detail
  ([§5.2](core.md#52-the-error-envelope)).
- **Holder**: any participant that holds a bundle and serves it on its key
  ([§8.4](core.md#84-contract-retrieval)).
- **Address**: a service's `<system>/<service>`, as a binding names it
  ([§3.2](core.md#32-binding-rules),
  [observer §3.6](observer.md#36-absence)).
- **Plain chunk**: `[a-z0-9]([a-z0-9._-]*[a-z0-9])?`
  ([§1.2](core.md#12-lexical-rules)).
- **Verbatim chunk**: a chunk that starts with `@`. In zk2, only the
  reserved tokens use one ([§1.2](core.md#12-lexical-rules)).
- **Established**: the question was put, and the bus answered, yes or no
  ([observer §1](observer.md#1-the-judgement-shape)).
- **NotAsked**: nobody put the question, so the tool renders no answer. A
  profile also spells not this profile's so, with its reason. There the
  question was put ([observer §1](observer.md#1-the-judgement-shape)).
- **Unobservable**: the question was put, and the observation could not be
  had. Its reason names why ([observer §1](observer.md#1-the-judgement-shape)).

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
