# Zenkey v2 — Redesign Brief

## Purpose

This document is an architectural brief for a **breaking redesign of Zenkey**.

Treat the current Zenkey implementation as a successful prototype that proved an important idea:

> Zenoh is an excellent communication substrate, but applications need a small, explicit semantic layer above it for typed API contracts, discovery, runtime introspection, and tooling.

The goal of Zenkey v2 is **not** to preserve the current API, keyspace, crate structure, registry format, or backward compatibility.

Backward compatibility may be broken freely.

The redesign should be driven by the architecture we want to maintain for the next decade, not by the shape of the existing implementation.

---

# 1. Strategic goal

Zenkey should become:

> **A small, implementation-independent API contract and reflection layer for Zenoh, with a first-class Rust implementation and tooling.**

Zenkey should **not** try to become another ROS 2.

Zenkey should **not** define every useful distributed-systems convention.

Zenkey should **not** make the whole company depend on one large framework.

The core should answer only a few questions:

1. What API does this component/service expose?
2. Which Zenoh resources implement that API?
3. What are the types of the exchanged data?
4. How can a generic tool discover and decode them at runtime?
5. How is compatibility between API versions determined?
6. How can Rust code use the API without constructing raw key strings manually?

Everything else should either:

- live in an optional profile,
- live in tooling,
- or remain an application concern.

---

# 2. Mental model

The target architecture should look approximately like this:

```text
┌───────────────────────────────────────────────┐
│               Application code                │
├───────────────────────────────────────────────┤
│       Generated / typed Zenkey bindings        │
├───────────────────────────────────────────────┤
│                                               │
│      Zenkey core application contract          │
│                                               │
│  - API description                             │
│  - resource identity                           │
│  - type/schema identity                        │
│  - runtime reflection                          │
│  - compatibility rules                         │
│                                               │
├───────────────────────────────────────────────┤
│                   Zenoh                        │
│ pub/sub · query/reply · liveliness · routing   │
├───────────────────────────────────────────────┤
│          QUIC / TCP / UDP / serial / ...       │
└───────────────────────────────────────────────┘
```

Optional functionality should sit beside the core rather than expanding it indefinitely:

```text
                    Zenkey Core
                        │
          ┌─────────────┼─────────────┐
          │             │             │
          ▼             ▼             ▼
      Telemetry       Media        Bulk/blob
       profile        profile        profile

                        │
                        ▼
                     Tooling
             zenctl / GUI / doctor /
             recording / diagnostics
```

---

# 3. The most important redesign principle

## Specification first, Rust crate second

The current project must stop being thought of primarily as:

> "a Rust crate that defines how applications use Zenoh"

and instead become:

> "a protocol/application-contract specification that happens to have an excellent Rust implementation"

A competent engineer should be able to implement the Zenkey protocol/convention in another language **without depending on the Rust crate**.

This is a hard design criterion.

For every core feature, ask:

> Could somebody implement this from the specification alone?

If the answer is no, the behavior is underspecified.

This matters even if Rust remains the dominant language.

---

# 4. Core vs optional functionality

The redesign should aggressively reduce the mandatory core.

## 4.1 Mandatory Zenkey core

The initial v2 core should contain only:

### A. Resource/API model

A formal model describing resources exposed over Zenoh.

At minimum, support concepts equivalent to:

- published stream/topic/resource;
- state/resource that can be read;
- queryable/request-response operation;
- runtime service/component presence;
- type/schema references.

Do **not** inherit the existing terminology blindly.

First determine the minimal semantic model.

### B. Registry / API definition

A machine-readable API definition from which we can derive:

- typed Rust bindings;
- runtime introspection data;
- documentation;
- compatibility checks;
- generic tooling.

The registry is the source of truth.

### C. Typed Rust generation

Applications should not manually concatenate key strings.

Generated APIs should provide strongly typed:

- resource identities;
- publishers/subscribers;
- query/request operations;
- parsers/matchers where needed.

Use the type system to prevent obvious addressing mistakes.

### D. Runtime introspection

A generic program should be able to discover:

- which components/services are alive;
- which APIs they expose;
- which resources exist;
- the kind of each resource;
- the type/schema associated with it;
- enough metadata to interact with it safely.

### E. Schema discovery

A generic tool should be able to move from:

```text
Zenoh resource
      ↓
API entry
      ↓
type identity
      ↓
schema / descriptor
      ↓
decoded value
```

### F. Compatibility

The API definition must support deterministic compatibility checks.

We need to distinguish at least:

- compatible additive change;
- breaking change;
- semantic change requiring explicit review;
- removal/deprecation.

Do not tie compatibility solely to a manually incremented version number.

---

## 4.2 Optional profiles

The following concepts should **not automatically belong to the core**:

- telemetry semantics;
- state aging / TTL conventions;
- event conventions;
- predefined QoS profiles;
- media streaming;
- blob transfer;
- record/replay;
- persistence/storage policy;
- alerting;
- diagnostics;
- authorization policy;
- fleet broadcast/fan-out behavior;
- observability-specific conventions.

They may remain in the repository, but move them into clearly separated profiles or libraries.

For example:

```text
zenkey-core
zenkey-schema
zenkey-codegen
zenkey-runtime

zenkey-profile-telemetry
zenkey-profile-media
zenkey-profile-bulk
```

The exact crate split is open to redesign.

The key point is conceptual separation.

---

# 5. Reconsider the keyspace from first principles

Do **not** preserve the current canonical key format merely because applications already use it.

Re-evaluate the key structure from zero.

The current design grew partly from observability / ZenSight-style requirements. The replacement must be suitable for general company software such as:

- navigation;
- control;
- mission management;
- payloads;
- health management;
- configuration;
- simulation;
- gateways;
- fleet-level services;
- hardware abstraction;
- monitoring.

## 5.1 Do not overfit physical deployment

A critical question:

> Is the primary identity of an API resource the machine/process that hosts it, or the logical service that provides it?

The new design should strongly consider **logical service identity as a first-class concept**.

Example conceptual distinction:

```text
physical identity:
vehicle-42 / computer-2 / process-731

logical identity:
vehicle-42 / navigation
```

Clients often care about:

> "the navigation service for vehicle 42"

rather than:

> "the process currently running on computer 2"

Physical placement may change without changing the API.

Do not encode deployment details into stable application APIs unless required.

## 5.2 Separate addressing concerns

Consider explicitly separating:

- domain/fleet/vehicle identity;
- logical service identity;
- API/interface identity;
- resource/operation identity;
- runtime instance identity.

Do not necessarily put every dimension into every Zenoh key.

Some information may belong in liveliness/introspection metadata instead.

## 5.3 Optimize for dominant queries

For every candidate key structure, test common selectors:

```text
everything exposed by one vehicle
all instances of one service
one resource across the fleet
everything exposed by one service instance
all implementations of API X
all resources of type Y
```

Zenoh key ordering has operational consequences because wildcard subscriptions and queries are important.

The chosen hierarchy should support the common queries naturally.

## 5.4 Separate stable API identity from runtime discovery identity

Avoid making ephemeral process IDs, host names, boot IDs, or deployment IDs part of a stable API path unless their presence is semantically meaningful.

A service may have:

```text
stable logical identity
+
ephemeral runtime instance identity
```

Treat those differently.

---

# 6. Define a small semantic resource model

Before designing syntax, define an abstract model.

One possible starting point is:

```text
Api
 ├── Stream
 ├── State
 ├── Operation
 └── Presence
```

But do not accept these categories without scrutiny.

The redesign task should ask whether these primitives are sufficient and orthogonal.

## 6.1 Stream

A value or sequence of values pushed by a producer.

Potential characteristics:

- payload type;
- reliability expectation;
- ordering expectation;
- retained/not retained;
- optional semantic annotations.

## 6.2 State

A value representing current state.

Potential behavior:

- subscribable;
- queryable;
- optionally stored;
- supersedes prior state.

Do not require Zenoh Storage for "state" to exist.

## 6.3 Operation

Request/reply semantics implemented with Zenoh queryables.

Potential metadata:

- request type;
- response type;
- error type;
- idempotency;
- timeout recommendation.

Avoid reinventing a huge RPC framework.

## 6.4 Presence

Runtime discovery should use Zenoh liveliness where appropriate rather than inventing periodic heartbeat topics unless there is a demonstrated reason.

Presence should allow tools to find:

- a running service/component;
- its runtime instance;
- the API contract it implements;
- where its introspection information can be retrieved.

---

# 7. Registry redesign

Treat the existing registry syntax as replaceable.

The registry should describe **logical API contracts**, not implementation boilerplate.

A good registry entry should be able to express something conceptually similar to:

```text
api: navigation
version: 2

resources:
  position:
    kind: stream
    type: nav.Position

  status:
    kind: state
    type: nav.Status

  set_origin:
    kind: operation
    request: nav.SetOriginRequest
    response: nav.SetOriginResponse
    error: common.Error
```

This is illustrative only.

Do not copy this syntax mechanically.

## 7.1 Registry requirements

The format must be:

- deterministic;
- human-reviewable;
- friendly to version control;
- easy to generate code from;
- easy to validate;
- independent from Rust;
- capable of referencing external schemas;
- canonicalizable for hashing/signatures if needed later.

Avoid unnecessary expressiveness.

Prefer a declarative data model over a mini programming language.

## 7.2 Multiple API definitions

A process may implement multiple APIs.

For example:

```text
navigation
health
diagnostics
```

Do not equate:

```text
one process == one API
```

Likewise, multiple runtime instances may implement the same API.

## 7.3 API composition

Investigate whether API definitions should support composition/imports.

For example:

```text
navigation-service
  implements common.health
  implements common.lifecycle
  implements navigation.v2
```

Keep this simple.

Do not create inheritance machinery unless needed.

---

# 8. Type and schema model

Zenkey should not invent its own general-purpose serialization format.

It should provide a generic way to identify and discover types.

## 8.1 Separate type identity from encoding

These are different questions:

```text
What does this value mean?
```

versus:

```text
How are the bytes encoded?
```

The model should distinguish at least:

- logical type identity;
- wire encoding;
- schema/descriptor;
- schema version or content hash.

For example, a logical `nav.Position` could theoretically be encoded as Protobuf, CDR, JSON, etc.

Whether multiple encodings for one logical type should be supported simultaneously is an open design question.

## 8.2 Content-address schemas where practical

Consider identifying exact schemas using a content hash.

Conceptually:

```text
type: nav.Position
encoding: protobuf
schema: sha256:...
```

This gives tooling an unambiguous descriptor.

The human-readable type name is not enough for exact compatibility.

## 8.3 Schema providers

Design schema discovery through a provider/plugin model where possible.

Potential providers:

- Protobuf descriptor set;
- JSON Schema;
- CDR/IDL metadata;
- custom application descriptors.

The core should not become coupled to every serialization ecosystem.

## 8.4 Rust types are not the protocol

Do not use Rust type names, `TypeId`, module paths, Serde implementation details, or crate version numbers as portable wire-level type identity.

Rust bindings map onto the protocol; they do not define it.

---

# 9. Runtime introspection redesign

Runtime introspection is the main differentiator from "raw Zenoh + protobuf".

It needs to be extremely clean.

## 9.1 Desired user experience

A generic tool should eventually be able to do something like:

```text
$ zenctl service list

vehicle-01/navigation
vehicle-01/health
vehicle-02/navigation
```

Then:

```text
$ zenctl api show vehicle-01/navigation

API: navigation
Version: 2.1
Instance: ...
Resources:
  position      stream     nav.Position
  status        state      nav.Status
  set_origin    operation  nav.SetOriginRequest -> nav.SetOriginResponse
```

Then:

```text
$ zenctl watch vehicle-01/navigation/position

{
  "latitude": ...,
  "longitude": ...,
  ...
}
```

The exact CLI commands may differ.

The architecture must make this flow naturally possible.

## 9.2 Introspection must describe reality

This is crucial.

A static API document that claims a resource exists while the program never registered it is dangerous.

Avoid introspection that can silently lie.

Investigate mechanisms to make the runtime exposure and introspection derive from the same registrations.

For example, prefer APIs conceptually like:

```rust
service.serve(api::SET_ORIGIN, handler);
service.publish(api::POSITION, publisher);
```

where runtime registration can automatically populate reflection state.

Avoid:

```rust
start_handler_manually();
publish_static_registry_separately();
```

when these can diverge.

## 9.3 Distinguish contract from runtime status

The introspection model should clearly differentiate:

```text
API contract:
"navigation v2 defines operation X"
```

from:

```text
runtime state:
"this instance currently exposes operation X"
```

This becomes important for:

- optional features;
- degraded modes;
- dynamically loaded modules;
- permissions;
- hardware-dependent capabilities.

---

# 10. Discovery and liveliness

Use Zenoh-native mechanisms wherever possible.

Zenkey should add semantics, not duplicate Zenoh.

Prefer Zenoh liveliness for runtime presence/discovery when it fits.

A discovered instance should expose enough metadata to answer:

```text
Who are you?
What logical service are you?
Which API(s) do you implement?
Which API revision/contract hash?
Where is your reflection endpoint?
What runtime instance is this?
```

Do not stuff large API registries into liveliness tokens if that creates unnecessary discovery traffic.

A small presence record pointing toward introspection may be better.

Consider large systems with hundreds of processes/services.

Discovery must remain efficient.

---

# 11. Compatibility model

This area deserves more rigor than ordinary semantic versioning.

## 11.1 Structural compatibility

The toolchain should be able to classify changes automatically where possible.

Examples:

Potentially compatible:

```text
+ add a new independent resource
+ add an optional field to a compatible schema format
```

Potentially breaking:

```text
- remove a resource
- change resource kind
- change request type
- change response type
- incompatible schema mutation
- change addressing semantics
```

Potentially semantic/manual-review:

```text
same type, but units changed
same operation, but behavior changed
same field, but valid range changed
```

## 11.2 Contract fingerprint

Strongly consider generating a deterministic fingerprint of an API contract.

For example:

```text
navigation v2
contract hash: sha256:...
```

This makes runtime diagnostics and reproducibility much better.

The human version is useful.

The exact contract fingerprint is authoritative.

## 11.3 Versioning policy

Do not automatically put a version component into every key unless there is a compelling reason.

Evaluate at least these models:

### Model A — version in path

```text
.../navigation/v2/...
```

### Model B — stable path, version negotiated/discovered via metadata

```text
.../navigation/...
```

with the contract version exposed through introspection.

### Model C — API/interface version in path but schema revisions external

Choose intentionally.

Do not inherit the current answer without analysis.

---

# 12. Generated Rust API

Rust is a major reason to build this architecture.

The generated bindings should feel native and make incorrect usage difficult.

## 12.1 Goals

Aim for:

- no hand-written key formatting in applications;
- compile-time distinction between resource kinds where practical;
- typed payloads;
- typed operation request/response/error;
- explicit logical service targeting;
- minimal boilerplate;
- very small runtime overhead;
- async support matching Zenoh naturally.

## 12.2 Avoid hiding Zenoh completely

Zenkey should not become a giant abstraction that prevents access to Zenoh functionality.

Applications may legitimately need:

- selectors;
- congestion control;
- priority;
- locality;
- query targets;
- advanced publishers;
- matching listeners;
- custom encodings.

The API can provide safe defaults while preserving an escape hatch to underlying Zenoh concepts.

Prefer:

```text
Zenkey = semantic layer on Zenoh
```

not:

```text
Zenkey = replacement API that happens to use Zenoh internally
```

## 12.3 Runtime registrations should drive introspection

As noted earlier, strongly consider generated APIs where using a resource registers it with a runtime service object.

Conceptual example only:

```rust
let service = Navigation::serve(session, identity).await?;

let position = service.publisher(api::POSITION).await?;

service
    .operation(api::SET_ORIGIN)
    .serve(|request| async move {
        ...
    })
    .await?;
```

The runtime should then know exactly what this instance actually exposes.

Do not copy this API literally without evaluating ergonomics.

---

# 13. Do not centralize the architecture unnecessarily

The registry is a source of truth for an API definition, but the runtime system should remain decentralized.

Avoid creating a mandatory central:

- schema server;
- API registry server;
- discovery daemon;
- broker;
- controller.

Zenoh already gives us decentralized communication/discovery capabilities.

A generic tool should be able to introspect a live system even if no central management service exists.

Central indexing/cache services may exist as optional tools later.

---

# 14. Security boundary

Zenkey should expose information in a way that can be protected with Zenoh ACLs.

Do not assume introspection is always world-readable.

The design should allow policies such as:

```text
ordinary application:
  can access its required API resources

operator:
  can read introspection

administrator:
  can execute privileged operations
```

The core does not need to invent authorization.

It needs to ensure its keyspace and introspection architecture can be secured cleanly using Zenoh's mechanisms.

Avoid one giant introspection endpoint that accidentally leaks every sensitive operation or schema if access control cannot be expressed around it.

---

# 15. Things to remove from the core

During the redesign, actively search for features that entered Zenkey because they were useful to one project.

Candidates to remove from the core include:

- ZenSight-specific semantics;
- assumptions that every producer is a "sensor";
- assumptions that telemetry/state/events are universal top-level classes;
- fleet-specific fan-out policy;
- media protocol details;
- blob transport;
- recording/replay policy;
- application monitoring;
- storage topology;
- deployment topology;
- hard-coded operational QoS profiles;
- UI-specific metadata.

They can survive elsewhere.

Removing something from `zenkey-core` does **not** mean deleting the functionality.

It means putting the responsibility at the correct layer.

---

# 16. Proposed repository shape

Do not treat this as mandatory, but evaluate a structure approximately like:

```text
zenkey/
├── spec/
│   ├── core.md
│   ├── resource-model.md
│   ├── addressing.md
│   ├── registry.md
│   ├── introspection.md
│   ├── schema.md
│   └── compatibility.md
│
├── crates/
│   ├── zenkey-core/
│   ├── zenkey-registry/
│   ├── zenkey-codegen/
│   ├── zenkey-runtime/
│   ├── zenkey-schema/
│   └── zenctl/
│
├── profiles/
│   ├── telemetry/
│   ├── media/
│   └── bulk/
│
├── examples/
│   ├── navigation/
│   ├── health/
│   └── multi-api-process/
│
└── tests/
    ├── compatibility/
    ├── conformance/
    └── interoperability/
```

An even smaller split is preferable if it remains clean.

Do not create crates merely for aesthetic architecture.

---

# 17. Add conformance tests

Because Zenkey should become a specification, add tests that validate implementations against the specification rather than merely unit-testing the Rust code.

Examples:

```text
- key/resource identity canonicalization
- registry canonicalization
- contract fingerprint generation
- runtime introspection format
- schema lookup
- compatibility classification
- wildcard/discovery behavior
```

Where possible, fixtures should be language-independent.

For example:

```text
tests/conformance/*.json
```

containing:

```text
input
expected canonical representation
expected key(s)
expected hash
expected compatibility result
```

This will make a future Python/C++ implementation possible without reverse-engineering Rust behavior.

---

# 18. Performance and scaling tests

The design must be tested against realistic company scale.

Do not optimize for only 3 demo processes.

Test at least conceptually:

```text
100+ machines/services
hundreds of API instances
thousands of resources
many simultaneous liveliness tokens
introspection after reconnect
router restart
service restart
slow/constrained links
```

Measure:

- startup discovery traffic;
- steady-state discovery traffic;
- memory;
- number of Zenoh resources;
- introspection payload sizes;
- reconnection behavior.

The reflection system must not recreate the sort of large discovery overhead that we are trying to avoid by not using ROS 2 everywhere.

---

# 19. Example domain used to validate the redesign

Use a neutral domain rather than ZenSight as the main design driver.

A good validation example is a navigation service.

Example requirements:

```text
Navigation API

stream:
  position

state:
  status
  origin

operations:
  set_origin
  reset
```

Deploy it as:

```text
vehicle-01/navigation
vehicle-02/navigation
```

Then test:

```text
- discover every navigation service
- select navigation for vehicle-01
- inspect its contract
- decode position generically
- call set_origin generically
- start a second navigation instance
- represent failover/redundancy
- move the service to another machine without changing its logical API
```

Then use a second example that challenges the model:

```text
health service
```

and a third:

```text
hardware driver with multiple devices
```

If the key/addressing model only looks elegant for the first example, redesign it.

---

# 20. Relationship with ROS 2

Zenkey should not aim for ROS 2 API compatibility.

However, the architecture should remain bridgeable.

It should be possible to build an adapter roughly like:

```text
ROS 2 topic/service
       ↕
bridge
       ↕
Zenkey resource/operation
```

without polluting the Zenkey core with ROS concepts.

Do not reproduce:

- ROS graph semantics;
- node semantics;
- ROS package semantics;
- actions;
- parameter server;
- lifecycle nodes;

unless a use case independently proves that one of these concepts belongs in Zenkey.

---

# 21. Relationship with raw Zenoh

A key design benchmark is:

> How much value do we add over "Zenoh + Protobuf + documented key names"?

Every core Zenkey concept must have a strong answer.

Expected core value:

```text
raw Zenoh + schema
    +
canonical resource identity
    +
machine-readable API contract
    +
runtime discovery
    +
reflection
    +
typed bindings
    +
compatibility validation
```

Anything beyond that deserves scrutiny.

---

# 22. Relationship with Keelson and other prior art

Review prior art again during the redesign.

Do not copy it blindly, but explicitly compare the new architecture with:

- Keelson;
- ROS 2 / RMW;
- D-Bus introspection;
- NATS subject conventions;
- MQTT/Sparkplug;
- uProtocol;
- VSS;
- OpenAPI / AsyncAPI concepts;
- Protobuf service descriptors;
- DDS type discovery;
- Zenoh admin-space conventions.

The objective is not academic completeness.

For every relevant idea, answer:

```text
Does this solve something Zenkey needs?
If yes, should we reuse the concept?
If no, why not?
```

---

# 23. Documentation structure

The redesigned project should have a very short architectural entry point.

A new engineer should be able to read roughly:

```text
README
  ↓
Core concepts
  ↓
10-minute example
  ↓
Specification
```

The README should explain Zenkey in one sentence similar to:

> Zenkey defines typed, introspectable application APIs over Zenoh.

Avoid describing Zenkey primarily as an observability framework, fleet framework, or keyspace convention.

The specification should separate:

```text
MUST
SHOULD
MAY
```

where useful.

---

# 24. Governance-ready design

Even though Zenkey is currently a personal project, design v2 so an organization could own the specification.

That means:

- specification changes are explicit;
- breaking changes are obvious;
- registry compatibility can be reviewed automatically;
- core behavior is not hidden in implementation details;
- normative documents exist;
- experimental profiles do not silently modify core semantics.

Consider an RFC process later, but do not let process overwhelm the engineering now.

The immediate priority is a small, stable core.

---

# 25. Concrete implementation plan

Do **not** start by patching the current API incrementally.

Follow this sequence.

## Phase 1 — Inventory

Analyze the current repository.

Produce a table with every current major concept:

```text
concept
current purpose
keep in core?
move to optional profile?
move to tooling?
delete?
redesign?
```

Include at least:

- key grammar;
- origin types;
- registry;
- introspect;
- describe;
- RPC;
- telemetry;
- state;
- events;
- liveliness;
- fleet behavior;
- schemas;
- serialization;
- media;
- blobs;
- QoS;
- storage;
- codegen;
- zenctl;
- GUI integration if present;
- compatibility/lock files.

Do not modify code until this inventory is complete.

## Phase 2 — New architecture document

Create:

```text
docs/v2-architecture.md
```

It must define:

- scope;
- non-goals;
- abstract resource model;
- identity/addressing model;
- registry model;
- introspection model;
- type/schema model;
- compatibility model;
- runtime discovery model;
- optional-profile boundary.

Do not use existing implementation names unless they survive the redesign deliberately.

## Phase 3 — Keyspace experiments

Propose at least **three** candidate addressing/keyspace designs.

For each, show how it handles:

```text
vehicle-01 navigation
vehicle-02 navigation
multiple navigation instances
one process exposing navigation + health
fleet-wide navigation subscription
hardware device resources
logical service moved to another host
```

Evaluate:

- wildcard ergonomics;
- stability;
- coupling to deployment;
- ACL ergonomics;
- readability;
- resource count;
- compatibility/versioning.

Pick one and document why.

## Phase 4 — Registry prototype

Create a new v2 registry format.

Use it for at least:

```text
navigation
health
```

Generate a canonical representation and contract hash.

Do not reuse the old format just to save implementation effort.

## Phase 5 — Minimal runtime

Implement only enough to support:

```text
discovery
introspection
schema lookup
one stream
one state
one operation
```

Nothing more.

Get this architecture clean before reintroducing extra features.

## Phase 6 — New Rust binding generation

Generate ergonomic Rust APIs from the new registry.

Validate that an application never needs to construct ordinary Zenkey resource strings manually.

Ensure runtime registrations and reflection cannot silently diverge.

## Phase 7 — zenctl

Implement only the generic commands needed to prove the architecture.

For example:

```text
zenctl service list
zenctl api show <service>
zenctl watch <resource>
zenctl get <state>
zenctl call <operation>
zenctl schema show <type>
zenctl compatibility <old> <new>
```

The actual command hierarchy may change.

## Phase 8 — Optional profiles

Only after the core is stable, reintroduce useful old functionality deliberately.

For every old feature ask:

```text
core?
generic optional profile?
application-specific?
delete?
```

---

# 26. Explicit permission to break things

For this redesign:

- backward compatibility is not required;
- existing key expressions may change;
- crate names may change;
- registry files may change;
- lock files may change;
- Rust API may change completely;
- CLI commands may change;
- introspection wire format may change;
- RFC numbering/history may be reorganized;
- deprecated code may be deleted instead of shimmed.

Do not build compatibility adapters unless they are genuinely useful for evaluating/migrating existing examples.

The objective is architectural quality.

---

# 27. Things the redesign must avoid

Do not:

1. Turn Zenkey into ROS 2 implemented in Rust.
2. Hide Zenoh so deeply that advanced Zenoh usage becomes impossible.
3. Create a mandatory central discovery/schema service.
4. Make physical machine/process identity part of every stable API without justification.
5. Treat Rust-specific implementation details as the protocol.
6. Put every useful company convention into `zenkey-core`.
7. Let introspection claim resources that are not actually exposed.
8. Invent a new serialization format without a compelling reason.
9. Optimize the design only for ZenSight.
10. Preserve a bad abstraction for backward compatibility.
11. Create dozens of crates/modules merely to look modular.
12. Encode version numbers everywhere without first defining what versioning means.
13. Reimplement Zenoh features that Zenoh already provides well.
14. Make generic tooling depend on application source code.
15. Require applications to manually duplicate their API definition in code and registry files.

---

# 28. Success criteria

Zenkey v2 is successful if all of the following are true.

## Architectural

A new engineer can explain it as:

> "Zenkey is a small typed API-contract and reflection layer over Zenoh."

Not:

> "Zenkey is a large framework containing telemetry, RPC, media, blob transfer, fleet discovery, storage, monitoring..."

## Implementation independence

A non-Rust implementation can be written from the specification.

## Rust ergonomics

A Rust service can expose a complete typed API with little boilerplate and no manual key formatting.

## Introspection

Without knowing the application beforehand, a generic tool can:

```text
discover service
→ inspect API
→ obtain schema
→ decode data
→ invoke an operation
```

subject to permissions.

## Truthfulness

Runtime introspection reflects what is actually exposed.

## Compatibility

CI can compare two registry/API versions and identify structural breaking changes.

## Deployment independence

A logical service can move between hosts/processes without forcing an API rename.

## Zenoh-native

The implementation uses Zenoh capabilities rather than rebuilding its own broker/discovery/runtime.

## Small core

The minimal Zenkey protocol can be understood without reading media, telemetry, storage, or GUI code.

## Scalability

Discovery/reflection remains reasonable with hundreds of services and thousands of resources.

---

# 29. Deliverables expected from this redesign task

Before large-scale implementation, produce the following artifacts:

```text
1. CURRENT_ARCHITECTURE_INVENTORY.md
2. docs/v2-architecture.md
3. docs/v2-keyspace-alternatives.md
4. docs/v2-registry.md
5. docs/v2-introspection.md
6. docs/v2-compatibility.md
7. examples/v2/navigation/...
8. examples/v2/health/...
9. an implementation roadmap / issue list
```

The architecture documents should be reviewed for internal consistency before doing a full rewrite.

---

# 30. Final instruction to the implementing LLM

Do not interpret this task as:

> "refactor Zenkey while preserving its current concepts."

Interpret it as:

> **"Use the current Zenkey repository as research material and redesign the smallest durable semantic/API layer that should exist above Zenoh."**

Challenge every existing abstraction.

Keep the ideas that remain strong after scrutiny:

- registry as source of truth;
- generated typed APIs;
- runtime introspection;
- schema discovery;
- compatibility validation;
- generic tooling.

Everything else is negotiable.

When uncertain, prefer:

```text
smaller core
+
clear specification
+
Zenoh-native primitive
+
optional extension
```

over:

```text
another mandatory Zenkey abstraction
```

The desired endpoint is not merely **Zenkey 2.0**.

The desired endpoint is a protocol/application-contract layer that we could confidently propose as the long-term foundation for a large Rust- and Zenoh-based software architecture.
