# Zenkey v2: architecture proposal r3.2, a redesign driven by use cases

*Proposal r3, 2026-10-07; **r3.1** after the issue review of the same day (§0.1), and **r3.2** after checking it against the three adopters (§0.2). r3.x keeps r3's section numbers, so citations of the form `r3 §x` stay valid. It supersedes r2 and §8 of the r1 analysis, and
it is self-contained. It is not code. Zenoh facts were read in the zenoh
1.10.1 source (Appendix B).*

**Objective.** The smallest durable, language-independent interface-contract
and reflection layer that makes Zenoh the foundation of an *application
architecture*: control, perception, mission, payloads, hardware
abstraction, simulation, ground segment, *and* supervision.

**Standing rule.** When we can either add a Zenkey abstraction or use a
Zenoh primitive with a small semantic rule, we use the primitive.

---

## 0. Why r3

r1 and r2 were designed from a **supervisor's chair**. Services were things
you observe and occasionally call, data flowed only *out* of them, and the
validation examples were fleet, health and operator. Six symptoms follow
from that:

| Symptom in r2 | Why it matters outside supervision |
|---|---|
| The only way to send something *into* a component was an RPC | A control loop publishes setpoints at 50–1000 Hz. A fleet commands vehicles that are offline. Neither is request/reply. |
| Consumers addressed a *provider* by name, and nothing modelled the wiring | Perception pipelines, simulation and replay all need to swap a source without touching code. The data-flow graph should be inspectable. |
| QoS and timing were add-on profiles | For control, reliability, priority, express delivery and deadlines *are* the interface semantics. |
| High-rate and large data were a special "media" profile | Images and point clouds are the normal case in payload and perception software. |
| Every payload had to be decodable by generic tools, which pushes protobuf everywhere | Large zero-copy payloads (flatbuffers, raw buffers, CDR) are legitimate. Tools must be *honest* about them, not able to decode everything. |
| Time and simulation were absent | Simulated time, replay and determinism are daily concerns in robotics. |

r3 starts from the use cases (§1) and derives the core again. Much of r2
survives unchanged, because it was right for every use case: state
semantics, operation rules, contracts, bundles, compatibility, presence.
The data-plane model and the framing change.

| r2 element | r3 |
|---|---|
| `zk2/` grammar major, `system/service/api/kind`, `@op`, `@zk`, location-free contracts | **Kept** (§3.1) |
| State rules S1–S7, operation rules O1–O5, bundles, fingerprints, FULL_TRANSITIVE | **Kept**; O6 added for operations with many replies (§3.7) |
| "API" | Renamed **interface**: it now has *requirements* as well as resources |
| Data flows out of a service only | **Kept as a strengthened invariant**: every key under a service is written only by that service. Data *into* a component arrives through **requirements bound to providers** (§2, §3.4). |
| QoS left to a profile | **QoS is part of the resource definition**, mapped 1:1 onto Zenoh QoS (§3.3) |
| `@media` profile | Generalized to **explicit-only streams** (`@stream`) in the core (§3.2) |
| Protobuf and jsonschema as the only kinds | Two kinds **blessed for generic decoding**, others **allowed**; type identity is mandatory, decodability is not (§3.9) |
| Operations: one reply | Operations may declare **many replies** (listings, partial results) (§3.7) |
| Presence per (instance × API) | Plus **one token per instance**, so pure consumers are visible too (§3.10) |
| Profiles: health, config, alarms, jobs, redundancy, freshness, media, bulk | Re-cut by use case. Added `timing`, `clock`, `arbitration`, `desired`. Supervision is three profiles among many (§3.12). |

---

## 0.1 What r3.1 changed (the issue review, 2026-10-07)

The review was sourced against zenoh 1.10.1 (still the latest release), zenoh-pico, zenoh-python, the storage manager, and the Rust and Python libraries the model crate would use. It produced decisions and corrections, not new features.

**Decided by the maintainer:**
- **Paradigm:** P3 (U-A).
- **Pilot:** tcgui, which meets every criterion in §6 decision 7.
- **Workspace:** the **strangler layout**. v1 lives on a `v1` branch for patches. On `main`, the v1 tools pin zenkey/zenkey-build 0.11.x from crates.io, while zk2 reuses the crate names at 0.20.0. Milestone 0.15.0 is closed.

**Order.** The model crate's static half (keys, contracts, canonical form, fingerprints, bundles) is built *before* the spike, against fixtures. The spike harness uses it instead of a throwaway hand-written layer. Only the network half waits for measurement.

**Corrections:**

| Topic | Correction | Where |
|---|---|---|
| Bundle container | A JSON document `{contract, schemas: {"sha256:…": {kind, data}}}`. Integrity is Merkle-style: the canonical contract embeds each schema's sha256, so the bundle needs no hash of its own. Published bundles are kept, append-only, in `contracts/.history/<iface>.v<N>/`. | §3.11 |
| Canonical form | Restricted to ASCII identifiers and integers within ±(2^53−1), with duplicate keys rejected. These are the only conditions under which every JCS implementation agrees: Rust crates round larger integers, Python's `rfc8785` raises on them. | §3.11 |
| Protobuf | **proto2/proto3 only** for now. protox, prost and prost-reflect do not support editions (prost#1031 is open). | §3.9 |
| jsonschema | Restricted to **a zk2 subset of JSON Schema 2020-12**, so that compatibility is decidable. The existing checkers are young: `json-schema-diff` is best-effort draft-07, `jsoncompat` is alpha. | §3.9 |
| The classifier | No Rust equivalent of `buf breaking` exists. The protobuf rules become a WIRE_JSON subset on `prost-reflect`, evaluated per direction, with `buf` as an optional cross-check. Bytes + the retention rule stand, because no protobuf compiler guarantees byte-stable descriptors. | §3.11 |
| Wildcard puts | Verified live: under `default_permission: allow`, a put on `t/*` reaches a `t/a` subscriber even though a deny rule on `t/a` exists. P3's ACL guarantees therefore need `default_permission: deny`, and consumers discard samples whose key is not concrete. | §3.4, §3.13 |
| Storage manager | It keeps timestamps, but never replies with tombstones. It has a suspected garbage-collection bug that drops *recent* tombstones, and a known wildcard-delete resurrection issue (zenoh#2649). Spike S5 tests both; they weigh on U1. | §3.6, §5 |
| Constrained devices | zenoh-pico supports liveliness, `complete`, `reply_del`, timestamps and the querier. It has **no namespace, no real HLC** (wall clock with a per-session bump), and 4 KB default fragments. The location-free bundle key lets a gateway serve bundles for it. A constrained conformance level is a new open question (U13), and the clock rule (U2) must allow for it. | §3.10, §5 |
| Spikes | New: **S14** ACL on a live router, **S15** constrained devices. S7 becomes "bundle stability and classifier feasibility"; the compatibility matrix itself belongs to the conformance suite. | §7 |

## 0.2 What r3.2 changed (the adopters, 2026-10-07)

r3.1 was checked against the three repositories that use zenkey v1 today:

- **ZenSight** (fleet observability):
  - 22 registries: 813 subjects, 200 procedures, 74 `when` gates;
  - proxy pollers of other devices;
  - a single-writer catalog, video, content-addressed artifacts, desired state.
- **zenoh-modem** (Zenoh over non-IP radio and satellite links):
  - one management contract served by every modem backend, published on crates.io for out-of-tree implementers;
  - 31 capability predicates over 88 entries;
  - commit-confirm configuration of the link the call travels through;
  - links of 220 B MTU at 2,400 bit/s, LoRa duty cycles, and Iridium SBD at about 96 messages a day.
- **tcgui** (network emulation; the pilot).

Two shapes, both legal in r3.1 but unnamed there, carry most of the weight. Most of what follows is about naming them, plus a few small core additions.

**System = host.** All three adopters mint a host identity from the machine id, with no coordination (v1's `h-<12hex>` origin), and scope everything by it.
- Under zk2 that host id *is* the system: `zk2/h-3fa9c2d41b7e/sysinfo/sysinfo.v1/stream/cpu/usage`.
- v1's derivation becomes a profile, `hostid.v1`, so tools recognize minted names.
- Per-host views (`zk2/<host>/**`), series identity (system, service, interface, resource), and host-keyed desired state all carry over position for position.
- "All providers of X" is the interface chunk, not the service name. N hosts each running `sysinfo` do not collide.

**Device-as-service.**
- **What it is.** A process that serves several devices hosts **one service per device**. That covers a modem driver (rf0, sat0, wwan0), an SNMP poller (one per polled device, `snmp.router01`), or a container sensor.
- **What it replaces.** v1's device tokens and its per-device capability predicates become ordinary per-service presence and exposure.
- **The instance id is a continuity epoch.** An instance MUST mint a new id whenever its counters reset (device re-enumerated, sysUpTime rewound, container restarted), even without a process restart. That is exactly v1's "a counter resets only when its device token cycles".
- **Cost.** Device-as-service multiplies presence tokens. ZenSight's SNMP scale is the stress case for S2 (U18).

**Core additions and corrections:**

| Finding (adopter) | r3.2 | Where |
|---|---|---|
| Device-defined trees `{device}/{metric...}` (snmp, modbus, gnmi, netflow) | **Rest parameters** `{name...}`, allowed only as the last template segment, each chunk slugged; one type for the whole family | §3.2 |
| SNMP traps keyed by ULID, a storage keeping the union, consumers backfilling by GET | **Occurrence-keyed streams**: a stream template may end in an `{occurrence}` parameter (ULID), published once per sample; storages keep the union; replay is a bounded wildcard GET (U16) | §3.2 |
| Video frames carry a CBOR `FrameMeta` attachment (parallax) | **Typed attachments**: a resource may declare `attachment = "<type>"`, fingerprinted and encoded like the payload | §3.2 |
| Population state with budgets (pdns 100k, edges 50k), judged by conformance | **`cardinality`** is a core attribute of templated resources | §3.2 |
| 74 + 88 capability-gated entries; `error/gated` versus `error/unsupported` | Optional resources may name their **gate** (`gate = "capability:rssi"`). The descriptor lists the capabilities held and the unavailable resources with a **cause** (`build` / `config` / `capability`). The error code `unavailable` carries the same cause. | §3.7, §3.10 |
| A single-writer catalog with standbys (ZenSight's claim protocol) | A standby holds its **instance token only**: interface tokens exist only while the instance exposes a resource of that interface, so alive ⇒ callable holds. ZenSight's `service_guard` is the reference for `redundancy.v1`. | §3.8, §3.10 |
| Management kept off the radio; liveliness denied on constrained faces; an SBD dead peer looks alive for an hour | **No `@zk` traffic is ever required across a constrained face.** Bindings work statically. Presence is an optimization. Bundles come from local holders or are pre-provisioned. The descriptor stays small. | §3.4, §3.10 |
| `default_permission` is node-global; zenoh-modem runs `allow` with face-scoped denies | P3's guarantees rest on **O2 (server refusal) and R6 (consumer filter)**. ACL deny is defense in depth; U14 becomes a SHOULD. | §3.13 |
| AdvancedSubscriber cannot parse `@adv` across a verbatim chunk (v1's KEYSPACE note) | Advanced pub/sub (history, recovery) is opt-in on **`stream` and `state` only**, never on `@stream` | §3.2 |
| Both adopters pass a claimed `actor` and `request_id` for audit (`?actor=`) | An optional **call-metadata attachment** on operation requests: claimed, never authentication (U17) | §3.7 |
| `modem-contract` (no zenoh dependency) on crates.io for out-of-tree backends | **Contract crates**: codegen can emit a publishable crate (types + bundle + traits, zenoh glue behind a feature) | §3.14 |
| ZenSight embeds key prefixes and origin lists in payloads | Guidance: payloads carry identities as structured fields (system, service, resource), never as key strings | §3.11 |
| Fleet selectors spanning majors (`alarms.v$*`) | Still no `$*`: tools subscribe once per supported major | §3.1 |

**New or reshaped profiles:**
- `hostid.v1`;
- `media.v1` (back from r2): codec tiers, the frame-age clock, receiver-driven adaptation, for ZenSight's parallax;
- `link.v1`: per-resource exposure on constrained faces and downsampling, read by the face/ACL generator;
- `views.v1`: ZenSight's GUI presentation documents.

Three profiles now have a reference implementation:
- `alarms.v1` lets an application fix its alert-key derivation byte-precisely, as ZenSight does;
- `config.v1` takes zenoh-modem's semantics whole (reach groups, `{token, apply_at}` before the apply, per-device confirm floors, separately grantable confirm and persist, the echo visible over the face, desired state never for reach);
- `redundancy.v1` takes ZenSight's claim protocol.

**The adopters map** (§4.9–§4.11). Each adopter gets a mapping issue before fixtures are written.

## 1. Use cases a foundation must carry

| # | Use case | Needs | Typical load |
|---|---|---|---|
| A | **Sensor and perception data flow** | Producer → consumers; swap the source (sensor, simulator, replay); fan-in from many producers; large payloads, zero-copy | 10–1000 Hz; up to MB per sample |
| B | **Closed-loop control** | Setpoints into actuators; priority and express delivery; deadlines and freshness; arbitration between commanders (autopilot, teleop, safety); emergency stop | 50–1000 Hz, small |
| C | **Services and jobs** (mission, planning, gateways) | Request/reply; typed errors; long-running work with progress and cancel; listings | Low rate |
| D | **Shared state and configuration** (world model, mode, parameters) | Latest value, late joiners, collections, change notification, consistent ordering, commit-confirm changes | Low to medium |
| E | **Events and alarms** | Reliable delivery; "what is active now"; optional retention | Bursty, low |
| F | **Hardware abstraction** | Many devices per driver, hot-plug, standard device interfaces | Mixed |
| G | **Simulation, test, replay** | Same code against simulated or recorded sources; simulated clock; mocks | Like A–B |
| H | **Remote commanding over intermittent links** (ground ↔ vehicle) | Durable commands (store and forward), link budgets, honest offline behaviour | Low rate, high latency |
| I | **Operator interfaces and generic tools** | Discover, inspect, decode, call, all without application source | Low |
| J | **Supervision** | Health, telemetry, alarms, dependency impact | Medium |
| K | **Bridges** (ROS 2, MQTT, HTTP) | A mechanical mapping in both directions | Mixed |
| L | **Components in several languages** (Python ML, legacy C++) | A spec plus artifacts, with no Rust dependency | n/a |

Every rule in §3 is justified by at least one of these. §4 walks eight of
them through the design.

---

## 2. Choosing the paradigm: the decision everything else follows

Three ways to organize application communication over Zenoh:

| | **P1 Data-centric topics** (DDS, ROS 2, dora-rs) | **P2 Service-centric with sink-addressed inputs** (AUTOSAR ara::com / SOME/IP fields and methods, Sparkplug DCMD, AWS/Azure shadow "desired") | **P3 Service-owned keys + consumer bindings** (r3) |
|---|---|---|---|
| Who writes a key | Anyone | The owner for outputs; *others* for inputs | **Only the owner**, ever |
| Data into a component | It subscribes to a topic name | Others write into the component's input keys | It **binds a role** to the providers it consumes |
| Swapping a source (sim, replay) | Remap the topic | Impersonate the provider, or reconfigure every writer | **Rebind the role** |
| Fan-in (many producers) | Many writers on one topic | Many writers on one input | A wildcard binding over every implementer of an interface |
| Operations and state | Bolted on (ROS services, parameters) | Native | Native |
| Cohesive versioning | Per topic | Per interface | Per interface |
| ACL | Per topic; writers are unbounded | Cross-principal write grants on inputs | **A principal writes only its own prefix**; others get read and call grants only |
| Wildcard-write hazard | Yes | Yes (a put on `zk2/*/*/thruster.v1/@in/…` reaches every thruster) | None across principals |
| Data-flow graph | From endpoint discovery | Partial | **Declared bindings in every descriptor** |
| Fit with Zenoh | Good | Good | Best: a key *is* its publisher's address |

**r3 chooses P3.** The ownership invariant (*a key is written only by the
service whose address prefixes it*) gives one-rule ACLs, clear provenance
for stored data, no cross-principal wildcard writes, and a data-flow graph
that is declared rather than inferred. P1's decoupling comes back through
bindings. P2's cohesive interfaces come back through contracts.

**What P3 costs, stated plainly:**

1. **Consumers need binding configuration.** A consumer does not just say
   "subscribe to `/cmd_vel`"; it binds a role to a provider.
2. **Arbitration moves to the consumer.** An actuator chooses among its
   commanders, which is the `twist_mux` pattern, and is standardized by
   `arbitration.v1`.
3. **Durable commands live with their author.** They are published by the
   commanding service, keyed by target. This is v1's own desired-state
   escape hatch, promoted to a pattern.

If the spike or the walkthroughs show these costs are wrong for control or
commanding, the fallback is to add *one* sink kind (U-A).

---

## 3. The architecture

### 3.1 Grammar

```
[<namespace>/]                                                        optional deployment prefix
zk2/<system>/<service>/<iface>.v<major>/stream/<resource…>            output stream, ambient
zk2/<system>/<service>/<iface>.v<major>/@stream/<resource…>           output stream, explicit-only (high-rate, large)
zk2/<system>/<service>/<iface>.v<major>/state/<resource…>             output state
zk2/<system>/<service>/<iface>.v<major>/@op/<operation…>              operation
zk2/<system>/<service>/@zk/instance/<instance>                        instance token (liveliness) and descriptor (queryable)
zk2/<system>/<service>/@zk/alive/<iface>.v<major>/<instance>/<fp16>   interface tokens (liveliness)
zk2/@zk/contract/<iface>.v<major>/<sha256>                            contract bundles, location-free
```

| Pos | Chunk | Rule |
|---|---|---|
| 1 | `zk2` | Zenkey grammar major. A **plain** chunk: never verbatim, because v1's verbatim `@v1` broke zenoh-ext's `@adv` parsing. |
| 2 | `<system>` | One plain chunk (§3.5). Under `zk2/@zk/…`, position 2 is the control token instead. |
| 3 | `<service>` | One plain chunk. |
| 4 | `<iface>.v<major>` \| `@zk` | Interface identity, or the control token. |
| 5 | Kind | `stream` \| `@stream` \| `state` \| `@op`, or a verbatim kind registered by a profile (`@blob`). |
| 6+ | Resource path | Literal and `{param}` chunks from the contract template. Values are slugged injectively (v1's `x-` rule, with a decoder and fixtures). |

**Kind tokens and the infrastructure that selects on them:**

| Token | Plain or verbatim | Selected by |
|---|---|---|
| `stream` | Plain: rides `zk2/<system>/**` | Recorders, link budgets, QoS overwrite |
| `@stream` | Verbatim: must be named | Explicit consumers only. A system subscription or a link filter never pulls 100 MB/s by accident. |
| `state` | Plain | Storage (`zk2/*/*/*/state/**`) for last-known values and store-and-forward |
| `@op` | Verbatim | Calls only. No snapshot GET or `**` selector can invoke an operation. |

**Properties by key algebra:**

- **Exact kind selectors.** Positions 1–5 have fixed arity, so
  `zk2/*/*/*/state/**` is exact.
- **The system firehose.** `zk2/<system>/**` is exactly the system's
  ambient streams and states.
- **Majors are mutually invisible.** `zk2` versus `zk3`, and `nav.v2`
  versus `nav.v3`, never match each other.
- **Coexistence.** v1 (`v1/…`), rmw_zenoh (`<domain>/…`) and raw
  applications can share the bus.

### 3.2 Resources

An interface **provides** resources of three patterns:

| Pattern | Meaning | Zenoh mapping |
|---|---|---|
| **stream** | A sequence of values pushed by the owner. Each sample stands alone. | A declared publisher. `explicit = true` selects the `@stream` token. |
| **state** | The latest value is the truth; it may be deleted | A declared publisher + a non-`complete` queryable over the interface's `state/**` (§3.6) |
| **operation** | Request → one reply, or many | A `complete` queryable on the concrete `@op` key (§3.7) |

- **Templates** with typed parameters (`devices/{device}/sample`,
  `tracks/{track}`) apply to all three. A wildcard GET over a state
  template lists the collection.
- **Rest parameters (r3.2).** The last segment of a template may be `{name...}`. It matches one or more chunks, each slugged, for device-defined trees such as `devices/{device}/metrics/{metric...}`. The whole family has one type, which is why it is the last segment only.
- **Occurrence-keyed streams (r3.2).** A stream template may end in `{occurrence}` (a lowercase ULID). Each sample is published once, with a one-shot put, on a fresh key. A storage then keeps the union, and a consumer replays with a wildcard GET bounded by a `retention` annotation. This is v1's sanctioned events exception, for logs that must survive. Everything else stays on stable keys (U16).
- **Typed attachments (r3.2).** A resource may declare `attachment = "<type>"`, for example video frame metadata. The attachment is fingerprinted with the contract and encoded like the payload.
- **Cardinality (r3.2).** A templated resource declares `cardinality`, its expected population bound. Tools and conformance check it. This is the bus's low-cardinality discipline, kept from v1.
- **Advanced pub/sub (r3.2).** zenoh-ext history and recovery are opt-in, on `stream` and `state` resources only. On a key under a verbatim chunk (`@stream`), zenoh-ext's `@adv` token parser cannot work.
- **No input kind** (by §2).
- **No event kind.** An event is a reliable stream, or a state per
  occurrence (`alarms.v1`).
- **No presence kind.** Presence belongs to instances (§3.10).

### 3.3 QoS is part of the resource

Each stream and state resource carries Zenoh QoS **in the contract**, and
the generated publisher applies it. Correct by construction, fingerprinted,
and mapped 1:1 onto stable Zenoh QoS:

| Field | Values | Default: stream | Default: `@stream` | Default: state |
|---|---|---|---|---|
| `reliability` | `best_effort` \| `reliable` | best_effort | best_effort | reliable |
| `congestion` | `drop` \| `block` | drop | drop | block |
| `priority` | Zenoh's seven levels, by name | data | data_low | data |
| `express` | bool | false | false | false |

Operations carry a *recommended* `priority`. Zenoh replies inherit the
caller's QoS, so the caller applies it.

Timing (period, deadline, lifespan) is `timing.v1` (§3.12). It stays a
profile only because it needs synchronized clocks and runtime support that
not every deployment has. For control it is expected, not exotic.

### 3.4 Requirements and bindings: how data reaches a component

An interface, or a component's own manifest, declares **requirements**:
named roles typed by an interface.

```toml
[requires.cmd]
interface   = "twist_cmd.v1"
resources   = ["cmd"]           # what is consumed (optional: default all)
cardinality = "many"            # one | many
optional    = false
```

| # | Rule |
|---|---|
| R1 | A role is **bound at deployment**, never in code. The binding is a list of service addresses: exact (`vehicle-01/teleop`) or wildcard (`vehicle-01/*` means every service on vehicle-01 that implements the interface). Only the binding is exchanged; the configuration format itself is recommended, not normative (U-B). |
| R2 | Template parameters may be bound too: `{vehicle} = self` binds the consumer to *its own* slice of a provider's collection (§4.3). |
| R3 | The instance descriptor lists every requirement with its bindings as declared. **The data-flow graph is read from descriptors and never inferred**, which serves debugging, safety review and supervision impact analysis alike. |
| R4 | A consumer compiled against interface `X.vN` binds to providers of *any* revision of `X.vN`, because FULL_TRANSITIVE compatibility guarantees it (§3.11). |
| R5 | Presence lets a consumer wait for its bound providers (start-up ordering) and notice when they leave. |
| R6 *(r3.1)* | A consumer discards any sample whose key expression is not concrete. Zenoh accepts puts on wildcard keys and delivers them with the publisher's key, so this filter costs one check per sample and stops injection by an over-granted principal. |
| R7 *(r3.2)* | **Bindings never require presence.** A binding resolves statically from configuration; presence is an optimization (R5). Across a constrained face, where a deployment denies `@zk` traffic, a consumer binds statically and reads liveness from the provider's own state freshness (`health.v1`). |

When several providers are bound, choosing between them belongs to the
consumer. `arbitration.v1` standardizes policies such as priority with
deadline (`twist_mux`), freshest wins, or an explicit lease.

### 3.5 Identity

| Concept | Example | Chosen by | Where it lives |
|---|---|---|---|
| Deployment | `prod`, `sim-42`, `hil-bench` | Operator | Optional session namespace |
| **System**: the logical deployment and ownership unit that is the outer policy boundary | `vehicle-01`, `robot`, `ground`, `cloud`, `sim-1` | Deployment | Key, position 2 |
| **Service**: logical, stable, owns its keys | `navigation`, `thruster-l`, `cam-front`, `imu0` | Deployment | Key, position 3 |
| **Interface**: name + major | `nav.v2`, `camera.v1` | Contract author | Key, position 4 |
| **Instance**: one run of a service | random 64-bit, minted at start | Runtime | Tokens, descriptor |
| Host, process, build, zid | | Runtime | Descriptor only |

- **A single robot sets one constant system.** A fleet uses many. The
  grammar is the same in both cases.
- **System = host (r3.2)** is a first-class shape, used by all three
  adopters. The system name is minted from the machine id by
  `hostid.v1`: v1's `h-<12hex>` derivation, byte-precise, with test
  vectors. A deployment can also mix shapes. In a vehicle with several
  computers, system = vehicle, and per-computer services carry the host
  in their name (`sysinfo.c2`), so "all sysinfo" stays the interface
  chunk (S8).
- **Device-as-service (r3.2).** A process serving several devices hosts
  one service per device: `zk2/node-1/rf0/modem.v3/…`,
  `zk2/h-…/snmp.router01/snmp.v1/…`. Each device then has its own
  presence, exposure, capabilities and ACL prefix.
- **The instance id is a continuity epoch (r3.2).** An instance MUST mint
  a new id whenever its counters reset, even without a process restart:
  a device re-enumerated, a polled device's uptime rewound, a container
  restarted. Consumers treat an instance change as the only legitimate
  counter discontinuity.
- **A process may host several services**, for example a driver hosting
  `imu0` and `imu1`.
- **Moving a service to another host changes no key.**
- **No implicit identity.** Every client targets an address or a binding;
  fleet-wide selection is spelled by name.

### 3.6 State semantics (unchanged from r2)

**Facts** (Appendix B):

- Routers stamp puts only, where timestamping is enabled (routers yes;
  peers and clients no, by default).
- Deletes and query replies are never stamped by the network.
- `Latest` ranks an unstamped reply below any stamped one, and delivers at
  query completion.
- `reply_del` is stable.
- `new_timestamp()` uses the HLC if enabled, otherwise wall clock + zid.
- Routers re-stamp future-dated puts by default.

| # | Rule |
|---|---|
| S1 | Every state mutation (put **and** delete) carries a producer-set timestamp. |
| S2 | The owner's GET reply carries the timestamp of the mutation it represents. |
| S3 | The owner answers GETs for keys it deleted within the tombstone window with `reply_del` and the deletion timestamp. |
| S4 | A state GET sets target `All` and consolidation `Latest` explicitly. |
| S5 | **Deployment:** storages covering state preserve timestamps and keep tombstones at least as long as the window. |
| S6 | Consumers distinguish *current* state (owner present) from *last-known* state (owner absent), through presence. |
| S7 | Serving sessions SHOULD enable HLC (open: MUST? U2). |

Under P3, S1–S6 cover shared state (D), digital twins, store-and-forward
commands (H, through `desired.v1`) and supervision (J) with one mechanism.

### 3.7 Operation calling rules

| # | Rule |
|---|---|
| O1 | Operation queryables are concrete-keyed and `complete`. A concrete call with `BestMatching` executes on **at most one** instance, even under split-brain. |
| O2 | A wildcard call is legal only to `fanout = "allowed"` operations, with target `All` and consolidation `None`. The server refuses any non-concrete query to other operations (`fanout_forbidden`), whatever the ACL says. |
| O3 | *(r3.2: `unavailable` carries `cause = build \| config \| capability` plus a reason, which restores v1's actionable split between `unsupported` (rebuild) and `gated` (reconfigure).)* The reply goes on the operation's own concrete key. Success is a value reply; failure is `reply_err` with the core envelope (`invalid_request`, `not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`, `internal`, `app`). |
| O4 | Only idempotent operations may be retried. |
| O5 | An empty reply set is not a verdict. ACL refusals return empty since zenoh 1.3. Attribute silence through presence. |
| **O6** | An operation may declare `replies = "many"`: zero or more value replies, then completion. These are native Zenoh semantics. Callers MUST use consolidation `None`. This covers listings, partial results and streamed answers. Work that outlives a query timeout belongs to `jobs.v1`. |
| O7 *(r3.2)* | **Call metadata:** a request MAY carry a small attachment `{actor, request_id}`, which servers MAY record for audit. It is **claimed, never authentication**. Both adopters already pass these as selector parameters (U17). |

### 3.8 Ownership and serving

| `serving` | Applies to | Meaning |
|---|---|---|
| `exclusive` *(default)* | All | At most one instance at a time exposes the resource. |
| `replicated` | Operations, in v2 | Any number of instances expose it. Lint: `idempotent = true`, unless a profile adds fencing. |

The core has **no role concept**:

- a standby instance exposes no exclusive resources, and declares **no interface token** for an interface it exposes nothing of (r3.2). It is visible through its instance token only, so alive ⇒ callable holds;
- two instances exposing the same exclusive resource is a finding,
  diagnosed from descriptors (Zenoh's `SourceInfo` is unstable and
  unvalidated);
- election and fencing belong to `redundancy.v1`;
- actuators keep an exclusivity lock outside the bus.

### 3.9 Types and schemas: identity is mandatory, decodability is not

Every resource declares a **type identity**: a logical name, a schema kind,
and the sha256 of its schema artifact. Kinds use MCAP's well-known
spellings:

| Kind | Generic tools | Use |
|---|---|---|
| `protobuf` | **MUST decode** | The default for control, state, operations and inter-team interfaces |
| `jsonschema` | **MUST decode** | Documents, configuration, Rust-first teams (schemars output committed) |
| `flatbuffer` | MAY decode | Large payloads read zero-copy |
| `ros2msg` / `cdr` | MAY decode | The ROS 2 bridge; RIHS01 serves as the schema hash |
| `raw` | Media type only (`image/jpeg`, `application/octet-stream` plus a layout document) | Images, audio, opaque buffers |

- **Restrictions (r3.1).** `protobuf` means proto2/proto3, until the Rust stack (prost, prost-reflect, protox) supports editions. `jsonschema` means the zk2 subset of JSON Schema 2020-12 defined by spike S7, so that compatibility stays decidable.
- **Honest rendering.** Generic tools MUST render a kind they cannot
  decode as a declared type and size (or through a plugin), never as
  garbage and never silently.
- **Encoding id only on samples.** Samples carry the predefined Zenoh
  `Encoding` id, never a schema suffix (it is sent with every put, and
  capped at 255 bytes).
- **Shared memory stays possible.** Zenoh SHM is transparent to keys, and
  generated publishers for `@stream` resources accept SHM buffers.

### 3.10 Presence, descriptors, contract retrieval

**Two kinds of liveliness token.** All information is in the key, since
tokens carry no payload.

- **Instance token:** `zk2/<system>/<service>/@zk/instance/<instance>`.
  It is the same key as the descriptor's queryable; liveliness and data
  key spaces do not collide. Every instance has one, *including pure
  consumers* (loggers, operator interfaces, controllers that only
  consume), so the data-flow graph has no invisible nodes.
- **Interface tokens:** `…/@zk/alive/<iface>.v<major>/<instance>/<fp16>`,
  one per implemented interface *that the instance currently exposes at
  least one resource of* (r3.2). "Who implements `nav.v2`, at which
  revisions" becomes the selector `zk2/*/*/@zk/alive/nav.v2/**`, answered
  with zero payload.

**Descriptor.** It is put on every change and answered on GET. It lists:

- the interfaces, each with its contract sha256 and minor, its exposed
  resources, and its unavailable optional resources with a cause
  (`build` / `config` / `capability`) and a reason;
- the capabilities this instance holds (r3.2), naming the gates its
  contracts declare;
- **the requirements with their bindings** (R3);
- the profiles;
- metadata (host, build, zid).

**Constrained faces (r3.2).** No `@zk` traffic is ever *required* across a
face a deployment marks constrained (`link.v1`). Presence, descriptors and
bundles all stay local:
- bundles come from holders on the same side, or are pre-provisioned;
- across the face, only the resources `link.v1` exposes travel, downsampled as declared;
- the descriptor targets ≤ 1 KB, because v1's 50 KB `introspect` reply would cost about 185 SBD messages (two days of budget).

**Contract retrieval** uses the key `zk2/@zk/contract/<iface>.v<major>/<sha256>`.
Every holder declares a `complete` queryable on it. The client:

1. GETs with `BestMatching`, which routes to the nearest holder;
2. verifies the sha256 of each reply and accepts the first valid one;
3. on no valid reply, retries once with target `All`;
4. if still nothing, reports the contract as *unavailable*.

Dynamic-language clients (Python scripts, notebooks, gateways) use bundles
to build typed clients **at run time, with no code generation**. This is a
use-case-L feature as much as a tooling one.

**Start-up order:**

1. Declare the resources.
2. Validate required exposure.
3. Declare the descriptor and contract queryables.
4. Declare the instance token, then the interface tokens.

So alive ⇒ callable. Tools query liveliness with explicit large handlers
(zenoh#2678).

### 3.11 Contracts and compatibility

**Contracts:**

- **Authoring and canonical form.** TOML authoring; a canonical JSON form
  serialized with JCS. It includes every machine-relevant field:
  resources, templates, QoS, serving, idempotency, fanout, `replies`,
  requirements, annotations, the profiles used. **Documentation is
  excluded.**
- **The fingerprint** is the sha256 of the canonical bytes.
- **Bundles are built once by the contract's CI and embedded by every
  implementation**, which only verifies them.
- **The bundle container (r3.1).** A JSON document
  `{contract, schemas: {"sha256:…": {kind, data}}}`. Integrity is Merkle-style:
  verify the fingerprint over the canonical contract, then each schema's
  sha256 as the contract lists it. Published bundles are kept, append-only,
  in `contracts/.history/<iface>.v<N>/`.
- **Canonical-form restrictions (r3.1).** ASCII identifiers only,
  integers within ±(2^53−1), duplicate keys rejected. Every JCS
  implementation (`serde_json_canonicalizer`, Python `rfc8785`) agrees on
  that domain.
- **Retention rule:** a rebuild that the classifier judges identical keeps
  the old bundle. Fingerprints may over-detect, never under-detect.

**Compatibility.** Every revision inside a major is FULL_TRANSITIVE
compatible: both directions, against every earlier revision. Payload types
are delegated to the schema kind's rules (protobuf: buf WIRE_JSON).

**Metadata changes are directional.** The r2 table is kept:

- `idempotent` true → false: breaking;
- `fanout` allowed → forbidden: breaking;
- `delivery`/`reliability` reliable → best_effort: review;
- optional → required: breaking for implementers;
- enum value added: review.

New rows in r3:

| Change | Class |
|---|---|
| QoS `priority` changed, `express` toggled | review |
| `congestion` drop → block | review (it can stall the producer) |
| `explicit` false → true (stream leaves the ambient firehose) | breaking for ambient consumers |
| `explicit` true → false | review (link budgets) |
| `replies` one → many | breaking (callers' consolidation) |
| Required role added to an interface | breaking (deployments must bind it) |
| Optional role added | compatible |
| Role's interface or cardinality changed | breaking |

### 3.12 Profiles

A profile is an independently versioned spec: a few pages of normative
behaviour plus scenario tests. The core never depends on one. Profiles
contribute through **four core extension points**:

- a standard contract;
- an annotation vocabulary, declared with `uses = [...]` (fingerprinted);
- a registered verbatim kind;
- the descriptor's `profiles` list.

Re-cut by use case:

| Profile | Use cases | Contributes |
|---|---|---|
| `timing.v1` | A, B, G | `period_ms`, `deadline_ms`, `lifespan_ms` annotations. Runtime: staleness flags, deadline-missed events, dead-man stops. |
| `arbitration.v1` | B | Consumer policies over many bound providers (priority + deadline, freshest, lease); optional `acquire`/`release` operations for explicit control authority. |
| `clock.v1` | G | A standard `clock.v1` interface (simulated, paused or real time); runtime rule that components read time through the bound clock. |
| `desired.v1` | H, D | Desired documents as **state of the authoring service, keyed by target**. Targets bind with `{target} = self`, converge, and report `observed_revision` in their own state (Kubernetes spec/status, device shadows). Store-and-forward rides S1–S6 plus a storage. |
| `config.v1` | D | Commit-confirm configuration (set → apply → confirm window → confirm or roll back; hot versus reach groups), as a standard interface. |
| `jobs.v1` | C | Start → id, state `jobs/{job}` lifecycle with terminal states and retention, cancel; annotations mark an interface's own job resources. |
| `redundancy.v1` | C, B | Election, fencing epochs, role naming (§3.8). |
| `blob.v1` | C, H | `@blob` kind: content-addressed bulk transfer with integrity rules. Delivery references carry (holder address, hash), never key prefixes. |
| `hostid.v1` *(r3.2)* | J, all adopters | System names minted from the machine id: v1's `h-<12hex>` derivation, with test vectors |
| `media.v1` *(r3.2)* | A | Codec tiers, the frame-age clock, receiver-driven adaptation, frame-metadata attachment (ZenSight parallax) |
| `link.v1` *(r3.2)* | H | Per-resource exposure on constrained faces and downsampling, read by the face/ACL generator (zenoh-modem) |
| `views.v1` *(r3.2)* | I | Presentation documents for generic UIs (ZenSight's `views`) |
| `health.v1`, `telemetry.v1`, `alarms.v1` | J, E | Health interface; metric annotations (unit, counter/gauge, histogram); active alarms as state plus transitions. |
| `lifecycle.v1` *(only if a use case proves it)* | B, C | Armed/disarmed or managed-component states. Not adopted by default. |

### 3.13 Security

The ownership invariant reduces ACL to three grant shapes:

| Grant | Rule |
|---|---|
| **Own** | A service principal publishes, deletes, declares queryables and declares tokens under `zk2/<system>/<service>/**`, plus its verbatim subtrees (`…/@stream/**`, `…/@op/**`, `…/@zk/**`), plus `zk2/@zk/contract/<iface>/*` for the interfaces it implements. |
| **Consume** | Subscribe or GET on the prefixes a principal's bindings name. |
| **Call** | Query on specific `…/@op/<op>` keys. Privileged operations are listed one by one. |

There are **no cross-principal write grants**: no command reaches a
component except as a call or through its own bindings.

Plain facts the spec states:

- **under `default_permission: allow`, a put on a wildcard key bypasses a
  deny rule on a concrete key it covers** (verified live, r3.1). P3's
  guarantees therefore rest on O2 (server refusal) and R6 (consumer
  filter). `default_permission: deny` is recommended where practical. It
  is node-global, and zenoh-modem's constrained faces run `allow` with
  face-scoped denies (U14, r3.2);

- deny works by inclusion, so deny with the widest pattern (`**/@op/**`);
- multicast transports bypass ACL;
- `zids` subjects are unauthenticated, so bind principals by certificate
  CN;
- ACL is enforced per hop, and the running ACL is not observable.

The certificate scope (one per system or one per service) is a deployment
choice. ACL generation from contracts plus bindings plus a role file is a
tool.

### 3.14 Rust shape

This is a sketch: the names and signatures show the shape, not a settled
API.

```rust
// an actuator: provides thruster.v1, consumes twist_cmd.v1 from bound commanders
let svc = zenkey::Service::new(&session, Addr::parse("vehicle-01/thruster-l")?);

let thr = thruster_v1::Server::declare(&svc, thruster_v1::Handlers {
    arm:    |req| async move { arm(req) },
    disarm: |req| async move { disarm(req) },
}).await?;

let mut cmds = twist_cmd_v1::Consumer::bind(&svc, bindings.role("cmd")?)   // recorded in the descriptor (R3)
    .arbitrate(Priority::with_deadline(Duration::from_millis(100)))        // arbitration.v1 + timing.v1
    .cmd()
    .await?;

let svc = svc.start().await?;           // resources → descriptor/contracts → instance token → interface tokens

while let Some((source, cmd)) = cmds.next().await {
    thr.telemetry.put(&apply(cmd)).await?;   // StreamWriter<Telemetry>, QoS from the contract
}
```

- **Tools and scripts** use `Client`s without a `Service`. A component
  that should appear in the graph declares a `Service`, even if it only
  consumes.
- **Zenoh stays reachable:** `key_expr()` on every handle, closures over
  the zenoh builders, and the session is always the caller's.
- **Test mocks:** generated `Server` and `Client` traits allow in-process
  test doubles.

---

## 4. Walkthroughs

**4.1 Perception pipeline (A, G).**

- **Wiring:**
  - `vehicle-01/cam-front` implements `camera.v1`: `@stream image`
    (`raw`, `image/jpeg`, or `flatbuffer`) and `state info`.
  - `vehicle-01/detector` implements `detections.v1` (`stream objects`)
    and requires `camera.v1` as `input`, bound to `vehicle-01/cam-front`.
  - `vehicle-01/tracker` requires `detections.v1` as `sources`, bound to
    `vehicle-01/*`, which means every detector.
- **Replay:** the recorder republishes as `replay/cam-front` in a
  `replay` namespace, or the detector's `input` is rebound to
  `vehicle-01/replay-cam`. No code changes.
- **Simulation:** `sim-1/cam-front` implements the same `camera.v1`.
- **The graph:** detector ← cam-front, tracker ← {detectors}, read from
  descriptors.

**4.2 Control loop with arbitration and emergency stop (B).**

- **Wiring:**
  - `thruster-l` and `thruster-r` implement `thruster.v1`: `state status`,
    `stream telemetry`, `@op arm`/`disarm`.
  - Each requires `twist_cmd.v1` as `cmd`, cardinality `many`, bound to
    `[vehicle-01/safety, vehicle-01/teleop, vehicle-01/autopilot]` in
    priority order (`arbitration.v1`, deadline 100 ms via `timing.v1`).
  - The commanders implement `twist_cmd.v1`: `stream cmd`, best_effort,
    drop, `real_time` priority, express.
- **Fail-safe:** a commander falling silent past its deadline drops out
  of arbitration; all silent means a dead-man stop.
- **Emergency stop:** `safety.v1` `@op estop`, `fanout = "allowed"`,
  called as `zk2/vehicle-01/*/safety.v1/@op/estop` with target `All`.
- **No cross-principal writes anywhere.**

**4.3 Commanding an intermittently connected vehicle (H).**

- **The durable command:** `ground/fleet-mgr` implements
  `mission_plan.v1` with `state plans/{vehicle}` (`desired.v1`).
- **The vehicle side:** `vehicle-01/executor` requires `mission_plan.v1`
  as `plan`, bound to `ground/fleet-mgr` with `{vehicle} = self`.
- **Store and forward:** a storage keeps `plans/*`. On reconnect, the
  executor GETs (S4, so the newest stamped value wins), converges, and
  reports `state/plan_status` with `observed_revision`.
- **Immediate actions:** `vehicle-01/executor` `@op abort` *fails fast
  when offline*. That is honest, and the operator sees it.

**4.4 Hardware driver with N devices (F).**

- **Default:** a service per device (`vehicle-01/imu0`, `imu1`) in one
  process. Instance tokens make hot-plug visible, and standard device
  interfaces (`imu.v1`) make drivers interchangeable.
- **Dynamic or large populations:** a template (`devices/{device}/sample`)
  inside one service.

**4.5 Mission service (C).** `jobs.v1` (start → id, `state jobs/{job}`,
cancel). Listings use an operation with `replies = "many"` (O6).

**4.6 Operator interface or Python notebook (I, L).**

1. Interface tokens show who is alive.
2. A descriptor GET shows the exposure and the graph.
3. A bundle fetch, verified by hash.
4. A dynamic client built from the protobuf descriptors: render state,
   subscribe, call.
5. A `raw` or `flatbuffer` stream is shown as type plus rate, unless a
   plugin decodes it.

**4.7 Supervision (J).**

- `health.v1` on every service; `telemetry.v1` annotations on metric
  streams; `alarms.v1` for active alarms.
- Impact analysis walks the declared bindings: "if `navigation` dies, the
  services bound to it are `planner` and `executor`". v1 needed a catalog
  service and evidence fusion to get this.

**4.9 ZenSight (r3.2).**

**Identity:** `system` = host id (`hostid.v1`).

**Services:**
- the host sensors (`sysinfo`, `netlink`, `netring`, `logs`, `systemd`, `hostspec`);
- per-device services for proxied devices (`snmp.router01`, `modbus.plc3`, `bmc.<chassis>`), each a continuity epoch;
- the deployment singletons on a logical system: `zk2/fleet/catalog/zs.catalog.v1/…` and `zk2/fleet/desired/…`, under `redundancy.v1` with the claim protocol as reference.

**Framework set:** v1's set becomes interfaces every producer implements, each with a fleet-wide selector:

| v1 | zk2 interface | Fleet-wide selector |
|---|---|---|
| `health` | `health.v1` | |
| `alert/{key}` | `alarms.v1` (ZenSight's alert-key recipe kept byte-exact) | `zk2/*/*/alarms.v1/state/**` |
| `evidence/**` | `zs.evidence.v1` | `zk2/*/*/zs.evidence.v1/state/**` |
| thresholds, `<topic>/set`, `applied/{topic}` | `zs.thresholds.v1` + `desired.v1` | |
| `views` | `views.v1` | |

**Telemetry:** device-defined trees use rest parameters. `telemetry.v1.Point` keeps the self-describing value (counter / gauge / histogram as a protobuf oneof), so the ~600 subjects without a declared `kind` need no back-fill.

**Traps:** occurrence-keyed streams.

**Historian:** `range` becomes a `fanout = "allowed"`, `replies = "many"` operation. Its series are named by (system, service, interface, resource).

**Parallax:** `media.v1` on `@stream`, with a `FrameMeta` attachment.

**Artifacts:** `blob.v1`.

**Migration debt to plan:** key prefixes and origin lists inside payloads (artifact delivery, alert refs, historian series); about 20 hand-built keys; the web client's hand-spelled keys.

**4.10 zenoh-modem (r3.2).**

**Identity:** `system` = node (host id).

**Services:** one per device (`rf0`, `sat0`, `wwan0`), each driver process hosting its device's service.

**Interfaces:** the management contract becomes `modem.v3` (the 2.x line was v1's registry stream). Its 88 gated entries become `optional` resources with `gate = "capability:…"`. Devices differ through their per-service exposure.

**Configuration:** `config.v1` with zenoh-modem's semantics whole:
- each of set / confirm / cancel / extend / persist is its own `@op` key, so each is separately grantable;
- the device is the service, so it is in the path.

**Links:** `link.v1` marks the four `link` resources that may cross an RF face (downsampled to 1/min) and none for SBD. No `@zk` traffic crosses.

**Contract crate:** `modem-contract` becomes a zk2 contract crate, with its bundle and generated traits, and zenoh glue behind a feature. Out-of-tree backends depend on it as they do today.

**Mixed versions:** FULL_TRANSITIVE inside `modem.v3` matches its own observation that "a field terminal and its ground gateway are upgraded months apart".

**4.11 tcgui (r3.2, the pilot).**
- **Identity:** `system` = host id; `service` = `tc`.
- **Requirement:** the frontend binds `tc.v1` as `backends` to `*/tc`.
- **Operations:** templated, exclusive, fan-out forbidden; `diagnostics` is fan-out allowed.
- **Full mapping:** `examples/zk2/tcgui/`.

**4.8 ROS 2 bridge (K).** The mapping:

| ROS 2 | Zenkey |
|---|---|
| node | service |
| publisher | stream (or state, for latched topics) |
| subscriber | requirement |
| service | operation |
| parameters | `config.v1` |
| actions | `jobs.v1` |
| message types | the `ros2msg` kind, with RIHS01 as the hash |

The bridge publishes under its own service addresses (the ownership
invariant holds).

---

## 5. Unresolved design decisions

The ones that decide the paradigm come first.

| # | Question | Lean | Decided by | Fallback |
|---|---|---|---|---|
| **U-A** | P3 (consumer bindings) or P2 (sink-addressed inputs) for control and commanding | **Decided 2026-10-07: P3** | Walkthroughs 4.2/4.3 under review; spikes S11, S12 | Add one sink kind `@in` (verbatim; the owner discards samples whose key is not concrete, which works because subscribers see the publication key) |
| **U-B** | Is the binding configuration format normative? | Descriptor field normative, configuration shape recommended | Multi-language pilot (L) | A normative minimal TOML/JSON shape |
| **U-C** | Requirements in contracts, or only in the component manifest? | Both allowed; interface-level ones are fingerprinted | Walkthroughs | Manifest only (simpler compatibility, weaker contracts) |
| **U-D** | `@stream` as a kind token, or a contract attribute only | Token (infrastructure must see it) | S3, S9 | Attribute + a recommended separate interface |
| **U-E** | QoS defaults per pattern; `priority` in the core | As in §3.3 | S9, S11 | Fewer fields; the rest to `timing.v1` |
| **U-F** | Decodability: two blessed kinds + allowed others | As in §3.9 | Pilot, S9 | Narrow to protobuf, jsonschema and raw |
| **U-G** | Operations with many replies in the core | Yes (O6) | S6 | `jobs.v1` only |
| U1 | State: producer + storage merged by timestamp, or owner-only + an explicit archive interface | Merge | S5 | Owner-only + `archive.v1` |
| U2 | Clock discipline: HLC MUST? Restart regression strategy | HLC MUST + hold writes until the clock passes the last stored timestamp | S5 | Epoch in the value |
| U3 | Tombstone window | Core default, annotation override | S5 | n/a |
| U4 | `complete` operation queryables (O1) | Keep | S6 | Non-complete; lose at-most-once |
| U5 | Token layout | Instance token + API-first interface tokens | S2 | Instance tokens only + descriptors |
| U6 | Mandatory one-chunk `system` | Keep | S8 | Revisit only on fake names |
| U7 | Bundle stability | Build once, embed, retention rule | S7 | Normalized description per kind |
| U8 | Error envelope encoding | Core `Error` in protobuf and jsonschema, `detail` in the interface's encoding | Spec draft | Always JSON |
| U10 | Large collections | No paging in the core; large collections become streams or operations with many replies | S5 | Paging on GET parameters |
| U11 | Descriptor dynamics | Put + GET | S2 | GET only |
| U12 | Redundancy: diagnose or delegate | Diagnose only | S6 | n/a |
| U13 *(r3.1)* | A constrained conformance level for zenoh-pico participants (no namespace, no HLC, 4 KB fragments) | Define it; bundles served by a gateway | S15 | zk2 requires the full Rust-core feature set |
| U14 *(r3.1, r3.2)* | Is `default_permission: deny` a deployment MUST for P3's guarantees? | **SHOULD**: guarantees rest on O2 + R6, because the setting is node-global (zenoh-modem) | S14 | n/a |
| U15 *(r3.1)* | The storage-manager position: depend on it, require a fixed version, or no storage on `state/**` | Decided with U1 | S5 | `archive.v1` |
| U16 *(r3.2)* | Occurrence-keyed streams in the core, or an `events.v1` profile | Core attribute (it changes key semantics and storage behaviour) | S5 (union storage, bounded replay GET) | Profile |
| U17 *(r3.2)* | Call metadata (`actor`, `request_id`) as a core convention | Optional core attachment, claimed only | Spec draft | Left to `config.v1` and applications |
| U18 *(r3.2)* | Device-as-service at ZenSight's SNMP scale (thousands of devices per poller): token and descriptor cost | Device-as-service, with interface tokens only where exposed | S2 (ZenSight shape) | Per-device presence as a template-scoped liveliness token under the parent service |

(r2's U9, profile binding, is now settled: `uses` in the contract plus
`profiles` in the descriptor.)

---

## 6. Maintainer decisions

| # | Decision | Recommendation |
|---|---|---|
| **0** | **Paradigm** (new) | **Decided 2026-10-07: P3**, service-owned keys + consumer bindings. |
| 1 | ZenSight | Freeze v1 at 1.50 / 0.14.x. Port tcgui first. Decide ZenSight after the tooling phase. |
| 2 | Mandatory `system` | Yes, provisionally (S8) |
| 3 | Kind in the key | Yes: `stream`, `@stream`, `state`, `@op` |
| 4 | Schema kinds | Protobuf by default; jsonschema first-class; flatbuffer, ros2msg and raw allowed and rendered honestly |
| 5 | Ownership | Exclusive per resource by default; replicated operations explicit; no core role |
| 6 | Repository and naming | Same repository. A `v1` branch for the frozen line; main takes the new architecture; 0.x until the core spec is frozen. Move production zenctl builds to `v1` first, and decide the open 0.15.0 milestone. |
| 7 | Pilot | **Decided 2026-10-07: tcgui.** The criteria it meets: a real, non-safety-critical company component with **a data-flow or control path** (at least one bound requirement), a state, an operation and a parametric resource, restart and reconnect behaviour, and preferably two machines. A pure supervision pilot would not validate r3. |

---

## 7. Spike plan

The spike is throwaway code that produces measured numbers. **The paradigm
tests (S9–S13) run first.**

| # | Group | Matrix | Measure | Decides |
|---|---|---|---|---|
| **S9** | Typed-layer overhead | Small control messages at 1 kHz; 4 MB frames at 30 Hz, with and without SHM; typed handles versus raw zenoh | Latency, p99 jitter, throughput, CPU, allocations per sample | U-D, U-E, U-F |
| **S10** | Wildcard bindings | 1 → 100 producers bound by `vehicle-01/*`; join and leave through presence | Fan-in correctness, join latency, memory | U-A, R1, R5 |
| **S11** | Control arbitration | Teleop / autopilot / safety on priority + deadline; commander crash; dead-man stop | Switch-over latency, missed-deadline detection, false stops | U-A, U-E |
| **S12** | Store-and-forward | `desired.v1` across a disconnect, router restart, storage on ground and on vehicle; clock skew | Convergence time, stale-command rejection, wrong answers | U-A, U1, U2 |
| **S13** | Simulation and replay | Rebind sources to simulator and replay without code change; `clock.v1` simulated time | Pass/fail; timing-profile behaviour under simulated time | R1, `clock.v1` |
| S1 | Grammar basics | Fixtures; namespace × verbatim chunks; an advanced publisher on `zk2/…` | Pass/fail | Grammar |
| S2 | Liveliness scaling | 100 / 1k / 10k / 50k tokens; layouts; 2+ routers; *r3.2:* the ZenSight shape (1,000 hosts × 6 sensors × ~5 framework interfaces, plus SNMP devices as services) | RSS, CPU, bytes, discovery latency, restart and reconnect convergence, churn, backpressure | U5, U11 |
| S3 | Constrained links | RTT, bandwidth, loss, reconnect (netem / tcgui); *r3.2:* zenoh-modem's classes (220 B MTU at 2,400 bit/s; LoRa 1 % duty; SBD with a 1 h lease); `@zk` denied on the face with static bindings; the declaration cost of zk2 keys versus v1's | Presence replay, descriptor and bundle fetch, time to first view | Link guidance, U-D |
| S4 | Contract serving | One, many, slow, unreachable and corrupt holders | Correctness, latency, retries | §3.10 |
| S5 | State correctness | Producer / storage / both / stale; restart with a clock behind; re-stamping; `reply_del`; wildcard GET at 1k and 10k; HLC on and off; *r3.2:* occurrence-keyed traps (union storage, bounded replay); advanced pub/sub on `state` alongside a storage | Wrong-answer count, latency, memory | U1–U3, U10 |
| S6 | Operations and ownership | Two owners (at-most-once?); replicated; failover; partition; fan-in over `complete` queryables; replies = many | Duplicates, failover time, findings | U4, U12, U-G |
| S7 | Bundle stability and classifier feasibility *(reworded in r3.1)* | protox/protoc/buf determinism; the retention-rule identity check; a WIRE_JSON subset on prost-reflect cross-checked with `buf`; the zk2 JSON Schema subset cross-checked with `jsoncompat`; Python `rfc8785` verifying Rust canonical bytes; the directional matrix drafted for the conformance suite | Byte drift; disagreements with buf | §3.11, U7 |
| S8 | Deployment shapes | Single robot, vehicle + ground, fleet-global, simulator, multi-device driver; *r3.2:* ZenSight (system = host, proxied devices, fleet singletons), zenoh-modem (node + device services), a vehicle with per-computer host services | Do natural `system`/`service` names exist? | U6 |
| **S14** *(r3.1)* | ACL on a live router | Pilot and walkthrough rule sets under `default_permission: deny`, with mTLS-CN and usrpwd subjects; wildcard-put injection; fan-out refusal; interest propagation | Exact grants per principal; leaks under `allow` | §3.13, U14 |
| **S15** *(r3.1)* | Constrained devices | A zenoh-pico participant as owner (health, stream, state, operation) against a router | What works; what the constrained level must relax | U13, U2 |

---

## 8. Not in the v2 core

Listed so later rounds can refuse them quickly:

- input (sink) kinds, unless U-A falls back;
- replicated streams and state;
- load balancing beyond the nearest replica;
- paging;
- per-sample instance attribution;
- durable RPC (use `desired.v1`);
- several encodings per resource;
- authorization policy;
- central caches or indexers;
- `lifecycle.v1`, unless a pilot proves it;
- recording formats and observer-honesty rules (tooling);
- everything ZenSight-specific.

---

## Appendix A: example contracts

```toml
# contracts/twist_cmd.v1.toml: what a commander provides
[interface]
name  = "twist_cmd"
major = 1
minor = 0
schemas = { protobuf = "proto/twist_cmd/v1/cmd.proto" }
uses    = ["timing.v1"]

[resources.cmd]
kind        = "stream"
type        = "twist_cmd.v1.Twist"
reliability = "best_effort"
congestion  = "drop"
priority    = "real_time"
express     = true
annotations = { "timing.period_ms" = 20, "timing.deadline_ms" = 100, "timing.lifespan_ms" = 100 }
```

```toml
# contracts/thruster.v1.toml: what an actuator provides, and what it requires
[interface]
name  = "thruster"
major = 1
minor = 0
schemas = { protobuf = "proto/thruster/v1/thruster.proto" }
uses    = ["arbitration.v1"]

[resources.status]
kind = "state"
type = "thruster.v1.Status"

[resources.telemetry]
kind = "stream"
type = "thruster.v1.Telemetry"

[resources.arm]
kind       = "operation"
request    = "thruster.v1.ArmRequest"
response   = "thruster.v1.Status"
idempotent = true

[requires.cmd]
interface   = "twist_cmd.v1"
cardinality = "many"
annotations = { "arbitration.policy" = "priority" }
```

```toml
# contracts/camera.v1.toml: a high-rate stream kept off ambient selectors
[interface]
name  = "camera"
major = 1
minor = 0
schemas = { protobuf = "proto/camera/v1/camera.proto" }

[resources.image]
kind     = "stream"
explicit = true                      # key token @stream
type     = { kind = "raw", media_type = "image/jpeg" }
priority = "data_low"

[resources.info]
kind = "state"
type = "camera.v1.Info"
```

```toml
# contracts/mission_plan.v1.toml: a durable command, owned by its author and keyed by target
[interface]
name  = "mission_plan"
major = 1
minor = 0
schemas = { protobuf = "proto/mission_plan/v1/plan.proto" }
uses    = ["desired.v1"]

[resources."plans/{vehicle}"]
kind   = "state"
type   = "mission_plan.v1.Plan"
params = { vehicle = "string" }

[resources.list]
kind     = "operation"
request  = "mission_plan.v1.ListRequest"
response = "mission_plan.v1.PlanSummary"
replies  = "many"
```

## Appendix B: Zenoh facts relied on (1.10.1 source)

| Fact | Source |
|---|---|
| Liveliness tokens carry no payload | `commons/zenoh-protocol/src/network/declare.rs` |
| Liveliness and data are separate key spaces | `src/api/liveliness.rs`, `src/net/routing/hat/*/token.rs` |
| Routers hold every token; clients and peers receive tokens on interest | `src/net/routing/hat/{router,client,peer,broker}/token.rs` |
| Verbatim = a chunk starting with `@`; `*`/`**` never match it | `commons/zenoh-keyexpr/src/key_expr/borrowed.rs` |
| **A put on a wildcard key is legal, and subscribers receive the publication's key, not their own** | `src/api/session.rs` (put resolution has no wildcard check; `subscriber_callbacks` passes the incoming key, ~l.498–520) |
| `BestMatching` = the nearest `complete` queryable whose key includes the query's key, else `All` | `src/net/routing/dispatcher/queries.rs` |
| `Latest`: per key, greatest timestamp (`None` lowest), delivered at completion; `None` consolidation delivers every reply | `src/api/session.rs` (~l.3540–3605) |
| Routers stamp puts only, where timestamping is enabled; future-dated puts are re-stamped | `src/net/routing/dispatcher/pubsub.rs` (`treat_timestamp!`); `DEFAULT_CONFIG.json5` |
| `Query::reply_del` is stable; `Session::new_timestamp()` = HLC, else wall clock + zid | `src/api/queryable.rs:437`; `src/api/session.rs:1038` |
| Replies inherit the query's QoS | v1 RFC 04 §3 (verified there against zenoh's no-op reply-QoS setters) |
| Encoding schema suffix ≤ 255 bytes, sent per put | `commons/zenoh-codec/src/core/encoding.rs` |
| ACL by inclusion; multicast bypasses ACL; `zids` unauthenticated | `src/net/routing/interceptor/{authorization,access_control}.rs`; `DEFAULT_CONFIG.json5` |
| `SourceInfo` unstable and unvalidated; matching status is a boolean | `src/api/{sample,matching}.rs` |
| `liveliness_query` deadlock past ~256 tokens on the default handler (open) | eclipse-zenoh/zenoh#2678 |
