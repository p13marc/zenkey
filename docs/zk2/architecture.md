# Zenkey v2: architecture proposal r4, a redesign driven by use cases

*Proposal r3, 2026-10-07; **r3.1** after the issue review of the same day (§0.1), **r3.2** after checking it against the three adopters (§0.2), **r3.3** after the adopters' contract mappings (§0.3), and **r4** after the spike (§0.4, 2026-10-08). r3.x and r4 keep r3's section numbers, so citations of the form `r3 §x` stay valid. It supersedes r2 and §8 of the r1 analysis, and
it is self-contained. It is not code. Zenoh facts were read in the zenoh
1.10.1 source, and r4's were measured by the spike (Appendix B).*

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
| The classifier | No Rust equivalent of `buf breaking` exists. The protobuf rules become a WIRE_JSON subset on `prost-reflect`, evaluated per direction, with `buf` as an optional cross-check. Bytes + the retention rule stand, because no protobuf compiler guarantees byte-stable descriptors. *(r4: superseded by WIRE with renumber detection, §3.11.)* | §3.11 |
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

## 0.3 What r3.3 changed (the contract mappings, 2026-10-07)

**The input.** The three adopters were mapped onto zk2 in the authoring format (`examples/zk2/`):
- **tcgui:** #589; 4 gaps.
- **zenoh-modem:** #623; 22 gaps, cited below as **Z1–Z22**.
- **ZenSight:** #622; 24 gaps, cited as **G1–G24**. Their lists are in each mapping's README.

They consolidate into the decisions below (D1–D26). **The authoring format moves to draft 1** (`examples/zk2/README.md`), and `zenkey-model` (#608) implements draft 1 and migrates every example to it.

| # | Decision | Gaps | Where |
|---|---|---|---|
| D1 | **Template precedence.** Two templates under one kind token may not have the same *shape* (literal/parameter positions). Overlap is allowed, resolved most-literal-first chunk by chunk (literal > `{p}` > `{p...}`), which is v1's rule. A lint warns when overlapping templates have different types. The same shape under different kind tokens is allowed, because the keys differ. | G1, Z3 | §3.2 |
| D2 | **Defaults.** `[defaults]` and `[defaults.<kind>]` in a contract set annotations, QoS and `encoding` for every resource, which may override them. Defaults are expanded *before* canonicalization, so the fingerprint is the same whether a value is defaulted or written out. | G11, Z4, tcgui 2 | §3.11 |
| D3 | **Two kind tokens.** Events become their own **plain** token, `events`: a resource of `kind = "event"` publishes each occurrence on `…/events/<template>/<ulid>` (the ULID chunk is implicit), carries `rate` (`rare` \| `low` \| `burst(n/h)`) and `retention`, and its cardinality is rate × retention. Storages select `zk2/*/*/*/events/**`, as v1's events storage selected `v1/*/events/**`. Large populations get **`@state`** (`explicit = true` on state): they stay off ambient `state/**` selectors, and storages and consumers name them. The kind tokens are now `stream`, `@stream`, `state`, `@state`, `events`, `@op`. | G6, G7, Z2 | §3.1, §3.2 |
| D4 | **One gate vocabulary:** `build:<n>`, `config:<n>`, `capability:<n>` (names `[a-z0-9][a-z0-9_.-]*`; v1's `CAP_BPF` becomes `cap_bpf`). The descriptor's cause uses the same three words, independently: it says why the resource is absent *here*, whatever the gate's kind. | G17, Z5 | §3.2, §3.10 |
| D5 | **Encodings.** A jsonschema resource declares `encoding = "json"` (the default) or `"cbor"`; protobuf is binary. An attachment declares its own encoding, by the same rule (ZenSight's FrameMeta is pinned CBOR). Bytes are CBOR byte strings, or base64 in JSON. | G8, G9 | §3.9 |
| D6 | **Raw families.** `{ raw = "video/*" }` is allowed. The sample's `Encoding` carries the concrete subtype, and `media_param = "codec"` may tie it to a template parameter. | G10 | §3.9 |
| D7 | **Type references across files and interfaces.** `json:<Name>` resolves across the listed files, and names must be unique across them (lint). The qualified form is `json:<file-stem>#<Name>`. A cross-file `$ref` may point only at listed files, which are bundled. Profile types (`telemetry.v1.Point`, the alarm document) come from profile schema files listed like any other. A type's identity is (kind, name, artifact sha256). | G12, Z18 | §3.11 |
| D8 | **A compact descriptor.** It lists the **capabilities held**, and as `unavailable` only resources whose absence is *not* implied by a missing gate capability. Exposure is the contract, minus resources gated on a missing capability, minus the listed exceptions. An instance may declare a lower `cardinality` than the contract's ceiling. The ≤ 1 KB target holds for zenoh-modem's union contract. | Z15, G2 | §3.10 |
| D9 | **Continuity epochs.** (a) **Re-minting:** declare the new instance token, interface tokens and descriptor first, then undeclare the old ones (an overlap, never a gap); the runtime API is `Service::new_epoch()`. (b) **Per-member epochs:** a template may declare `epoch = "<param>"`, and the implementation then holds a member token `…/@zk/member/<iface>.v<N>/<value>/<epoch>` per member, cycled on discontinuity (containers, parallax streams, devices modelled as members). Device-as-service and member tokens are both legal; S2 sets the default (U18). (c) **Across constrained faces:** a counter that `link.v1` exposes carries its epoch in-band (`telemetry.v1`'s counter type has `epoch`), and where the epoch cannot be observed, a decrease is read as a reset. | Z1, Z16, G16 | §3.5 |
| D10 | **Per-replier completion.** A `replies = "many"` operation may declare `summary = "<type>"`. Each replier then ends with exactly one summary reply (partial, scanned, cursor, `covers_from`), so the caller knows which replier finished and how completely. | G14 | §3.7 |
| D11 | **Configuration** (recorded for `config.v1`): `set` replies with a discriminated union (the read-back, or `{token, apply_at}`), and a compact `status` resource serves constrained faces. The zk2 JSON Schema subset admits `oneOf` with a discriminator. | Z8, Z9 | §3.12 |
| D12 | **Gate or interface?** A coherent optional plane with its own semantics and consumers (configuration, SDU data) is its own interface, and the interface's presence is the gate: "capability X ⇒ implements `config.v1`" needs no new field. Scattered optional readings are gates. zenoh-modem decides whether to split out its SDU plane before `modem.v3` freezes. | Z6, Z7 | §3.2 |
| D13 | **ACL compilation per posture.** Under `default_permission: deny`, grants are allows: fail-closed, and recommended wherever the router can afford the enumeration. Under `allow` (zenoh-modem's faces), zenoh evaluates only denies, as read in its source (S14 to confirm). The generator therefore compiles each grant into denies of its complement: every declared operation or plane not granted, reads included. A later minor's new resources cross until regenerated (fail-open), so generation re-runs on every contract revision. | Z12, Z13 | §3.13 |
| D14 | **`link.v1`** is contract defaults plus a deployment face policy that may expose per principal (an operator's `config.v1` view). Two zenoh limits are recorded: a face cannot tell two lanes of one link protocol apart (use distinct protocols or zids, or a router per radio), and downsampling needs one concrete rule per key, because a wildcard rule shares one timer. | Z10, Z11, Z20 | §3.12 |
| D15 | **R7, restated.** Across a constrained face, liveness comes from the freshness of whatever `link.v1` exposes. Where nothing crosses (SBD), liveness is *unobservable*, and tools say so. | Z14 | §3.4 |
| D16 | **`freshness.v1`** joins §3.12: `ttl_s` is the staleness horizon, and `ttl_s = 0` means never stale (operator intent, desired state). | G18 | §3.12 |
| D17 | **Deprecation.** `deprecated = { since = <minor>, replaced_by = "<template>", reason = "…" }` is fingerprinted, and tools warn. A deprecated resource is still served until the next major (lint). | G19 | §3.11 |
| D18 | **`views.v1`.** A presentation document is a content-addressed artifact, referenced from the contract by annotation (`views.document = "sha256:…"`), carried in the bundle's `extras`, and verified by its hash. Its scopes are (interface, template). | G20 | §3.12 |
| D19 | **Profile vocabularies.** Each profile ships its annotation table. Until the profiles land, draft 1 of the format carries interim tables (freshness, timing, telemetry, link, arbitration, desired, alarms, media, views, redundancy). | G21 | #613 |
| D20 | **History parameters.** `history = { depth = <n>, miss_detection_ms = <ms> }`, fingerprinted; `true` means depth 1. | G24 | §3.2 |
| D21 | **No repeated parameters.** Operation request types SHOULD NOT repeat template parameters (lint). | tcgui 1 | §3.7 |
| D22 | **`desired.v1`.** One template per document type. The binding names its target explicitly: `self.system` or `self.service`. | G5 | §3.12 |
| D23 | **`alarms.v1`** fixes the host-scoped label exclusion normatively. References across services are structured fields (system, service, key), never dot-packed. Dotted service names are derived as `<parent>.<slug(device)>`, with a dot-free parent, split at the first dot. | G3, G4 | §3.5, §3.12 |
| D24 | **Strays.** A parent service keeps families for data about devices it does not serve as services (`traps/{sender}`). Device services exist only by configuration. | G15 | §3.5 |
| D26 | **One host-id salt for zk2.** v1 salts the machine-id hash per application, so one machine gets three different origins under tcgui, ZenSight and zenoh-modem. `hostid.v1` uses a single zk2-wide salt (`zk2-hostid-v1`), keeping the derivation and test-vector discipline, so a machine is **one system across every application**. Each adopter's v1 origin maps to its zk2 system name at port time (a migration table; ZenSight's catalog can publish it as aliases). | Z22 | §3.5, §3.12 |
| D25 | **Large populations.** S5 seeds 50k and 100k by wildcard GET. U10's lean changes: large populations are `@state`, with paged or many-reply operations, or storage-backed GETs, sized by measurement. *(r4: storage-backed GETs on an owner's keys are superseded by §3.6 rule S4; archives serve last-known state.)* | G22 | §5 |

**Deferred, minor:**
- codegen name hints (G23) → #611;
- a services-per-system bound (Z17): dropped, a deployment concern;
- `claimed_source` (Z21) → O7's `actor`;
- the Rust binding of jsonschema types, and schemars output versus the zk2 subset (Z19) → #611, with S7's corpus;
- a pure consumer's manifest (tcgui 3): unchanged, its requirements are code plus binding configuration;
- duplicated identity fields in payloads (tcgui 4): port time.

**Confirmed by the mappings, not changed:**
- the claim protocol fits instance tokens as the claim set, and the interface token as the incumbent (§3.8, §3.10);
- rest parameters fit snmp, modbus, gnmi and netflow;
- `cardinality` fits every v1 bound;
- interface-keyed selectors remove a ZenSight GUI bug: instance-suffixed producers (`netring-2`) were silently missed;
- device-as-service and capability gates express all 88 of zenoh-modem's gated entries;
- no `@zk` traffic needs to cross the radio.

## 0.4 What r4 changed (the spike, 2026-10-08)

**The input.** Every spike has run: S1–S15, with S8 on paper. The numbers
are in [`spike-report.md`](spike-report.md), and the code is tagged
`zk2-spike-final`. Each row below cites the spike behind it. r4 keeps r3's
section numbers. *In §0.4, "Sn" names a spike. §3.6's state rules are
written "rule Sn".*

**Reviewed.** An independent review checked r4 against the spike report,
its raw data, the spike code and the zenoh 1.10.1 sources. It made 40
findings (3 blockers), all integrated before r4 became the design of record
(#605).

**Decided by the maintainer on 2026-10-08:**
- **U1: the owner is authoritative.** A state GET is answered by the owner.
  Last-known state lives in an **archive**, a service implementing
  `archive.v1` that consumers read explicitly. This is r3's own fallback for
  U1, adopted because the merged GET gave 10 wrong answers in 42 cases (S5).
- **O1 is reworded.** A concrete call executes at most once **only while one
  instance serves it**. `BestMatching` reaches one `complete` queryable per
  router, so under a split-brain across routers every call ran on both sides
  (S6). Tools detect that from tokens, and `redundancy.v1` owns exclusivity.
- **The protobuf classifier uses WIRE semantics with renumber detection.**
  JSON-name and enum-name changes are *review*, and adding a value to an
  open enum is compatible. zk2 protobuf is binary on the wire, and tools
  decode with the writer's bundle (S7).
- **The two draft upstream reports** (the storage manager, S5; ACL denies on
  router links, S3) **are not filed.** They stay in `docs/zk2/upstream/` as
  records of zenoh 1.10.1's behaviour.

**What changed:**

| # | Change | Spike | Where |
|---|---|---|---|
| E1 | **State: owner-authoritative GET, and `archive.v1`** (U1, U15). A deployment MUST NOT run a storage that answers on an owner's `state/**` or `@state/**` keys; tools check the storage admin space. An archive records owners' mutations under its own address, with timestamps, type identity and tombstones, and answers explicit GETs only. Its backend MUST NOT accept a put older than a delete it holds; zenoh 1.10.1's storage manager does (`upstream/storage-manager-outdated-guard.md`, not filed). It **re-aligns** with an owner that becomes reachable again (or with the owner's side archive while the owner is down), and drops a key only on a `reply_del` (erratum). Archives are placed on the consumer's side against link loss, and on the owner's side against owner loss. | S5, S12 | §3.6, §3.12 |
| E2 | **Clocks bounded both ways** (U2). HLC MUST. **Catch-up:** an owner never stamps at or below the last timestamp it issued. It reads that from its own persistent record, else from an archive on its own side. With neither, it starts a new epoch: a new instance with a fresh zid. Every zenoh timestamp carries its HLC's id, so consumers order within an id and restart their ordering at a new one. **Ahead:** an owner never stamps beyond its router's HLC delta (500 ms by default). It SHOULD detect drift against a router-stamped reference it subscribes to, and stops writing state when beyond it. Routers re-stamp future-dated puts but not replies. The constrained level keeps the catch-up and the ahead bound on a wall clock. | S5, S12, S15 | §3.6 |
| E3 | **Tombstone windows** (U3): 60 s by default, a deployment setting of the owner (erratum: not an annotation). State an archive records across a constrained face carries a window of at least the face's maximum outage. The deployment raises it through `link.v1`'s face policy. This takes the place of S5's "replication required for storages on state". About 100 B per tombstone. | S5 | §3.6, §3.12 |
| E4 | **O1 reworded; split-brain diagnosed, exclusivity delegated** (U4, U12). `complete` stays: it routes concrete calls, captures templated ones, and makes fan-in work. The token check has a grace period longer than D9a's make-before-break overlap. Tools diagnose; the runtime does not fence. | S6, S2 | §3.7, §3.8 |
| E5 | **O6, measured:** `Latest` and `Auto` kept 1 reply of 10, so consolidation `None` stays a MUST (U-G). | S6 | §3.7 |
| E6 | **Contract retrieval accepts the first valid reply as it arrives.** In S4, the slow and the unreachable holders were the nearest, the valid reply came from behind a second router, and waiting for completion cost the whole timeout. | S4 | §3.10 |
| E7 | **A presence budget** (U5). Presence costs per token, not per layout. Discovery took 1.2–1.9 s at 10k tokens, 23.8 s at 36k, and 46–49 s or never at 50k. A presence domain, the routers that exchange declarations, SHOULD stay within about 10–15k tokens, which kept discovery within 2–4 s. S2 measured tokens only, so the budget may be optimistic. Cutting r3's per-service multiplier is U22. | S2 | §3.10, §5 |
| E8 | **Device-as-service holds to about 5,000 devices per domain** (15k tokens, 3.3 s), which is the whole budget. Above that, member tokens (D9b) are the shape, at a third of the tokens (U18). | S2 | §3.5 |
| E9 | **What crosses a constrained face depends on the attachment** (R7, D15). A router-to-router link carries every declaration. A `@zk` deny there hides presence, but the denied declarations still cross, key strings included (`upstream/acl-denied-declarations-cross.md`, not filed). A far-side session attached as a **client** carries only what it asks for: 17 B per bring-up of 50 services with the deny, and 322 B for 200 unrequested tokens without it, against 11.1 KB router to router. The rule covers one far-side session, or a gateway session that re-keys what it relays under its own address. A site with its own router, or a service commanding many vehicles, is **U23** (zenoh 1.10.1 clients connect to one endpoint). | S3 | §3.4, §3.10, §3.13, §4.10 |
| E10 | **The constrained level** (U13), in two halves. **Devices:** a wall clock with the catch-up, a literal deployment prefix, a one-fragment descriptor, gateway bundles where receive limits bite. **Links:** the far side attached as a client; batches that cross well within the lease (about 1 KB at 2,400 bit/s, where the 64 KB default made the link reconnect in a loop; a link's batch is node-wide on TCP, so the face needs its own router); `@stream` and `@zk` denied across the face; no presence across it, because on a router link every coverage gap re-declares everything (11.8 KB against 536 B over a client link). | S15, S3 | §3.10, §3.12 |
| E11 | **Advanced pub/sub under a verbatim chunk, corrected.** History from publishers already present works, but late-publisher detection and heartbeat recovery do not, because zenoh-ext parses `@adv` keys with a `**` that cannot cross `@stream`. History stays on plain `stream` and `state` only (lint E019 in `zenkey-model`'s `diag::CODES`). | S1 | §3.2 |
| E12 | **QoS defaults confirmed; `express` stays opt-in** (U-E). `real_time` + express lost 0.6–4 % of samples at 1–5 kHz on loopback, with no latency gain; the contract author opts in only on measurement. `@stream` costs nothing measurable as a token, and ambient selectors carried none of its frames across a link (U-D). | S9, S3 | §3.3 |
| E13 | **The typed-layer budget is a per-message cost:** at most 1 µs of CPU on the typed path for a control-size message (measured: 126 ns). The ≤ 5 % p99 target is deferred to a quiet host. **SHM conditions:** zenoh-shm locks the pool plus 1,280 KiB of metadata, so `RLIMIT_MEMLOCK` SHOULD be at least the pool plus about 2 MiB. The implicit 16 MiB pool needs 17.25 MiB; at 8 MiB it fell back to TCP silently. An unlockable metadata segment panics, and tools check the limit at start. Zero-copy flatbuffers need in-place building (#611). | S9 | §3.9 |
| E14 | **`arbitration.v1` and `timing.v1` gain their rules.** The binding order is the priority. The deadline runs on the **receiver's** monotonic clock, evaluated on receive and on a timer of at most deadline/50. A liveliness delete drops a provider at once. No fresh provider means the dead-man value. Identity comes from the key. A sender-stamped `lifespan` MUST NOT be used unless skew is bounded well below it, and uhlc's default 500 ms delta is not. Without that bound `lifespan` is not enforced, and control loops rely on the deadline. | S11 | §3.12, Appendix A |
| E15 | **`clock.v1` states its tick.** Replay into a deployment namespace needs a replayer in that namespace (zk2 `zenctl replay --namespace`, #612). | S13 | §3.12, §4.1 |
| E16 | **ACL generation rules** for #612. Egress is checked by inclusion against the query's or subscription's own key, so every consumer selector that intersects a provider joins that provider's egress grant. Replies to such selectors are granted on the provider's ingress. Own spells out `@state`; contract bundles are open to all. U21 is confirmed live, D13's complement denies work, and U14 stays a SHOULD. | S14 | §3.13 |
| E17 | **Bundles and compatibility:** protox matches protoc byte for byte. The retention rule's identity check is a normalized comparison (source info and default `json_name` dropped), which also recognizes `buf build`'s bytes (U7). The protobuf rules are WIRE with renumber detection, and adding to a proto2 closed enum is review. The jsonschema subset uses tolerant readers. S7's draft matrix is regenerated under the decision (#607). | S7 | §3.11 |
| E18 | **Events: `retention` needs a time-series backend**, or a consumer-side filter. The memory backend ignores `_time` (U16, U19). | S5 | §3.2 |
| E19 | **Runtime rules.** A liveliness GET on a session that holds a liveliness subscriber MUST use a callback or an unbounded handler: zenoh#2678 hung such a session at every measured size from 996 tokens. A silent loss is noticed only at the lease, 10 s by default. | S2, S10 | §3.10, §3.4 |
| E20 | **`desired.v1` cancels with a terminal document**, not a delete, so an archive that missed it cannot resurrect a plan. | S5 | §3.12, §4.3 |

**Errata found while writing the spec (#606).** Each is marked where it
applies:
- §3.2: events are the fourth pattern (D3), and the "no event kind" line is
  gone.
- §3.6 rule S3: the tombstone window is a deployment setting of the owner.
  No annotation key exists for it.
- §3.6 rule S5: archive alignment drops a key only on positive evidence (a
  `reply_del`). An empty reply set never does.
- §3.3: state QoS is a SHOULD, not a MUST. zenoh-modem's refreshed state is
  legitimate.
- §3.1: profile-registered verbatim kinds are reserved.
- **Minting** *(spec §4.3)*: zenoh 1.10.1 keeps `Session::hlc()` internal,
  so an owner mints state timestamps as the greater of `new_timestamp()`
  and the last one plus a tick.

**Not measured.** The spike did not cover these, and the rules above do not
claim them:
- **S3:**
  - loss (no netem on the host);
  - the 220 B MTU, the LoRa duty cycle, and SBD's one-hour lease;
  - several client sessions sharing one line;
  - `express` under link contention (S9 deferred it to S3, and S3 did not
    run it).
- **S2:** declarations other than tokens.
- **S9:** cross-host latency, and the ≤ 5 % p99 target on a quiet host.
- **S12:** outages longer than about 1.5 s.
- **S14:** certificate-CN subjects.
- **S5:** an owner answering a 100k collection alone, without a storage.

**Confirmed, not changed:**
- the grammar: 2,226 keys round-trip, and the guard holds under namespaces
  (S1);
- P3 (S10, S11, S12);
- R1, because sources switched by configuration alone (S13);
- R3, because descriptors alone drew the data-flow graph (S10);
- R5 (S10);
- R6, at about 10 ns per sample (S1);
- O2 and O7 (S6);
- U6 (S8);
- U10, with no paging up to 100k keys (S5);
- U11, because make-before-break never left a gap (S2);
- U17 (S6);
- U20 (S2, S5);
- U-F;
- zenoh-pico as a full owner (S15).

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
zk2/<system>/<service>/<iface>.v<major>/@state/<resource…>            output state, explicit-only (large populations; r3.3)
zk2/<system>/<service>/<iface>.v<major>/events/<resource…>/<ulid>     one key per occurrence (r3.3)
zk2/<system>/<service>/<iface>.v<major>/@op/<operation…>              operation
zk2/<system>/<service>/@zk/member/<iface>.v<major>/<value>/<epoch>    member tokens (liveliness; r3.3, D9b)
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
| 5 | Kind | `stream` \| `@stream` \| `state` \| `@state` \| `events` \| `@op` *(r3.3; r4: the block above updated to match)*. A verbatim kind registered by a profile (`@blob`) is reserved: the spec (#606) refuses it until a later version defines its form. |
| 6+ | Resource path | Literal and `{param}` chunks from the contract template. Values are slugged injectively (v1's `x-` rule, with a decoder and fixtures). |

**Kind tokens and the infrastructure that selects on them:**

| Token | Plain or verbatim | Selected by |
|---|---|---|
| `stream` | Plain: rides `zk2/<system>/**` | Recorders, link budgets, QoS overwrite |
| `@stream` | Verbatim: must be named | Explicit consumers only. A system subscription or a link filter never pulls 100 MB/s by accident. |
| `state` | Plain | Archives (`zk2/*/*/*/state/**`, subscribing; *r4*: never answering on the owner's keys; across a constrained face, only what `link.v1` exposes) for last-known values and store-and-forward |
| `@state` *(r3.3)* | Verbatim: must be named | Large populations (catalog edges, pdns, desired documents); archives *(r4)* and consumers name them explicitly |
| `events` *(r3.3)* | Plain | One key per occurrence (`…/events/<template>/<ulid>`); union storages (`zk2/*/*/*/events/**`) and bounded replay GETs |
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

An interface **provides** resources of four patterns *(r4 erratum: r3.3's D3 made events a kind, and this table had not followed)*:

| Pattern | Meaning | Zenoh mapping |
|---|---|---|
| **stream** | A sequence of values pushed by the owner. Each sample stands alone. | A declared publisher. `explicit = true` selects the `@stream` token. |
| **state** | The latest value is the truth; it may be deleted | A declared publisher + a non-`complete` queryable over the interface's `state/**` and `@state/**` (§3.6) |
| **event** | One key per occurrence, kept by union storages (D3) | A one-shot put on `…/events/<template>/<ulid>` |
| **operation** | Request → one reply, or many | A `complete` queryable on the concrete `@op` key (§3.7) |

- **Templates** with typed parameters (`devices/{device}/sample`,
  `tracks/{track}`) apply to all three. A wildcard GET over a state
  template lists the collection.
- **Rest parameters (r3.2).** The last segment of a template may be `{name...}`. It matches one or more chunks, each slugged, for device-defined trees such as `devices/{device}/metrics/{metric...}`. The whole family has one type, which is why it is the last segment only.
- **Occurrence-keyed streams (r3.2; r3.3: now the `events` kind, D3).** A stream template may end in `{occurrence}` (a lowercase ULID). Each sample is published once, with a one-shot put, on a fresh key. A storage then keeps the union, and a consumer replays with a wildcard GET bounded by a `retention` annotation. This is v1's sanctioned events exception, for logs that must survive. Everything else stays on stable keys (U16). *(r4)* Enforcing `retention` needs a time-series backend, or a consumer-side filter, because the storage manager's memory backend ignores `_time` (spike S5). A union storage on `events/**` is not an owner's state, and S4 of §3.6 does not forbid it.
- **Typed attachments (r3.2).** A resource may declare `attachment = "<type>"`, for example video frame metadata. The attachment is fingerprinted with the contract and encoded like the payload.
- **Cardinality (r3.2).** A templated resource declares `cardinality`, its expected population bound. Tools and conformance check it. This is the bus's low-cardinality discipline, kept from v1.
- **Advanced pub/sub (r3.2; corrected in r4).** zenoh-ext history and recovery are opt-in, on `stream` and `state` resources only. Under a verbatim chunk, history from publishers already present works, but late-publisher detection and heartbeat recovery do not: zenoh-ext parses `@adv` keys with a `**` that cannot cross `@stream` (spike S1). zk2 therefore allows history on plain `stream` and `state` only (lint E019 in `zenkey-model`'s `diag::CODES`).
- **No input kind** (by §2).
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

*(r4)* The defaults delivered every sample at 100 Hz–5 kHz (spike S9).
`express` stays opt-in: `real_time` + express lost 0.6–4 % of samples at
1–5 kHz on loopback, with no latency gain. It is a fingerprinted contract
field, so the contract author opts in, and SHOULD do so only on
measurement. The walkthrough's `twist_cmd.v1` keeps it as the example of
opting in. State writes SHOULD keep the state defaults (`reliable` +
`block`): with a dropping put, 10,917 of 100,000 values never reached a
storage (spike S5). *(r4 erratum, #606: r4 said MUST. zenoh-modem's
best-effort state, re-put every `ttl_s/2` and read by the owner's GET, is a
legitimate pattern. An archive recording it can miss values between
re-puts.)*

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
| R5 | Presence lets a consumer wait for its bound providers (start-up ordering) and notice when they leave. *(r4)* A binding resolves at once, without presence, and a consumer waiting on presence starts within one token propagation (spike S10: 11 ms). A silent loss is noticed only at the lease, 10 s by default. |
| R6 *(r3.1)* | A consumer discards any sample whose key expression is not concrete. Zenoh accepts puts on wildcard keys and delivers them with the publisher's key, so this filter costs one check per sample and stops injection by an over-granted principal. |
| R7 *(r3.2, r3.3, r4)* | **Bindings never require presence.** A binding resolves statically from configuration; presence is an optimization (R5). Across a constrained face, a consumer binds statically and judges liveness from the freshness of whatever `link.v1` exposes. Where nothing crosses (SBD), liveness is *unobservable*, and tools say so (D15). *(r4)* What crosses a constrained face is set by how the far side attaches (§3.10). A router-to-router link carries every declaration, and a `@zk` deny there hides presence without keeping it off the link (spike S3). A far-side session attached as a **client** of the near router carries only the declarations its interests ask for. That covers one far-side session or a gateway session; a far side with several sessions or its own router is U23. |

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
  presence, exposure, capabilities and ACL prefix. *(r4)* It holds to about
  5,000 devices per presence domain (15k tokens, discovered in 3.3 s; spike
  S2). That is the whole presence budget (§3.10), shared with everything
  else in the domain. Above it, member tokens (D9b) are the shape, at a
  third of the tokens (U18).
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

### 3.6 State semantics (r4: the owner is authoritative)

*In this section, S1–S7 are the state rules. Spikes are written "spike Sn".*

**Facts** (Appendix B):

- Routers stamp puts only, where timestamping is enabled (routers yes;
  peers and clients no, by default).
- Deletes and query replies are never stamped by the network.
- `Latest` ranks an unstamped reply below any stamped one, and delivers at
  query completion.
- `reply_del` is stable.
- `new_timestamp()` uses the HLC if enabled, otherwise wall clock + zid.
- Routers re-stamp future-dated puts by default, but not GET replies *(r4)*.
  A revision written by a clock that runs ahead therefore carries two
  timestamps (spike S12). uhlc's default maximum delta is 500 ms
  (`UHLC_MAX_DELTA_MS`). A router can drop future-dated puts instead
  (`timestamping.drop_future_timestamp`).

| # | Rule |
|---|---|
| S1 | Every state mutation (put **and** delete) carries a producer-set timestamp. |
| S2 | The owner's GET reply carries the timestamp of the mutation it represents. |
| S3 | The owner answers GETs for keys it deleted within the tombstone window with `reply_del` and the deletion timestamp. *(r4; erratum, #606)* The window is 60 s, unless the deployment configures the owner with another (U3). No annotation key exists for it. State recorded by an archive across a constrained face MUST carry a window of at least that face's maximum outage, which the deployment's face configuration gives. A delete the archive misses is otherwise resurrected: spike S5's "after the producer's window" case. |
| S4 *(r4)* | **The owner is authoritative.** A consumer's state GET is addressed to the owner's keys and answered by the owner, with target `All` and consolidation `Latest` set explicitly. A deployment MUST NOT run a storage that answers on an owner's `state/**` or `@state/**` keys. Under `Latest` a consumer cannot tell which replier answered, so tools check this against the routers' storage admin space (#612). |
| S5 *(r4)* | **Last-known state lives in an archive**: a service implementing `archive.v1`, read explicitly. It records owners' mutations under its own address with each mutation's timestamp and type identity, and keeps tombstones for at least the window (S3). Its backend MUST NOT accept a put older than a delete it holds. zenoh 1.10.1's storage manager does accept one (spike S5). **Alignment** *(erratum, #606)*: when an owner becomes reachable again, the archive MUST re-read the owner's recorded collection before serving it as confirmed. While the owner stays absent, it re-reads that collection from an archive on the owner's side instead. It drops a key **only on positive evidence**, a `reply_del` from the source. A key the source neither reports nor tombstones is kept and served unconfirmed, because an empty reply set is not a verdict (O5): an access-control refusal, or a route that has not crossed yet, returns empty too. The raised window of S3 is what makes every missed delete answerable. |
| S6 *(r4)* | Consumers distinguish *current* state (the owner's GET) from *last-known* state (an archive). A consumer turns to an archive only when the owner gave no reply within the GET's timeout, or when presence shows the owner absent. That silence is not a verdict about the key (O5): it only selects the archive, whose answer is last-known, never current. |
| S7 *(r4)* | Serving sessions MUST enable HLC (U2). **Catch-up:** an owner MUST NOT stamp at or below the last timestamp it issued for its keys. Before its first write, it takes that timestamp from its own persistent record of it, else from an explicit read of an archive **on its own side**. A consumer-side archive can be stale by a whole partition. With neither source, it MUST start a new epoch: a new instance (§3.5) whose session has a fresh, unpinned zid. Every zenoh timestamp carries its HLC's id, the zid, so each value already carries its epoch, with no per-sample attribution (§8). A consumer orders values by time **within** one timestamp id, and accepts the first value under a new id, restarting its ordering there. That is U2's "epoch in the value". A value from the old epoch still in transit when the new one starts is the residual risk, left to the spec (#606). **Ahead:** an owner MUST NOT stamp beyond its router's HLC delta (500 ms by default). It SHOULD detect drift against a reference it holds for the purpose: a subscription to a router-stamped key, such as a deployment heartbeat, because its own puts are never echoed back. When a detection shows it beyond the delta, it stops writing state and reports through `health.v1`. Without a reference, routers re-stamp its future-dated puts (or drop them, with `drop_future_timestamp`), and its GET replies keep its own stamps. The constrained level replaces HLC with a wall clock plus the catch-up, and the ahead bound still applies (U13). |

**Why r4 left the merged GET.** r3 merged owner and storage replies by
timestamp. Spike S5 measured 10 wrong answers in 42 cases with the stock
storage manager:
- A local patch to the storage manager removed 4 of them net. It fixed five
  rows caused by the defect (an older put after a delete resurrects the key)
  and turned one replicated case that had been right only by accident into
  a wrong one.
- Six remain, and they are design-level:
  - a stale storage answering alone;
  - a producer restarting with its clock behind;
  - a tombstone window shorter than the storage's staleness;
  - an unstamped producer;
  - the memory backend ignoring `_time` on events.
- A clock running *ahead* was spike S12's case, not spike S5's.

The owner-authoritative form keeps a stale storage, a short window and an
unstamped producer away from a consumer that asks for *current* state. It
does not fix a clock behind, which still breaks a consumer that remembers
what it applied (spike S12). The catch-up in S7 fixes that. An archive makes
the remaining risk explicit in the read.

**The archive** *(r4; a sketch, settled by the spec #606 and `archive.v1`,
#613)*:
- **It is a new zk2 service.** The zenoh storage manager answers only on
  the keys it subscribes to, so it cannot serve under another prefix. It can
  serve as the archive's store only once it stops accepting outdated puts.
- **Its keys:** the archived key's address under the archive's own `@state`
  token: `zk2/<system>/<archive>/archive.v1/@state/{origin...}`. Each origin
  chunk is slugged like any rest parameter, so a verbatim `@state` chunk of
  the origin becomes `x-_x40state`. For example,
  `zk2/vehicle-01/archive/archive.v1/@state/ground/fleet-mgr/mission_plan.v1/state/plans/vehicle-01`.
- **What it answers:** GETs only. It never puts, because it is not a second
  publisher of the data. Each reply carries the archived value's type
  identity (interface, contract fingerprint, type) in an attachment. The
  owner's descriptor is unreachable exactly when the archive is read.
- **Its `cardinality`:** the sum of the populations it records.
- **Ownership holds:** the archive writes only its own prefix. Ambient
  selectors never see archived copies.
- **Placement, by what each covers:**
  - an archive on the consumer's side covers losing the link;
  - an archive on the owner's side covers losing the owner's process, for
    example a plan written while the vehicle was offline, after which the
    fleet manager stopped.

  Store-and-forward deployments place one on each side.

Under P3, S1–S7 cover shared state (D), digital twins, store-and-forward
commands (H, through `desired.v1` and archives) and supervision (J) with one
mechanism.

### 3.7 Operation calling rules

| # | Rule |
|---|---|
| O1 *(r4)* | Operation queryables are concrete-keyed and `complete`. A concrete call with `BestMatching` executes on **at most one instance while one instance serves the operation**. `BestMatching` reaches one `complete` queryable per router, so under a split-brain across routers a call executes on each side: spike S6 measured 200 of 200 calls run twice. Tools diagnose that from tokens (§3.8), and exclusivity belongs to `redundancy.v1`. |
| O2 | A wildcard call is legal only to `fanout = "allowed"` operations, with target `All` and consolidation `None`. The server refuses any non-concrete query to other operations (`fanout_forbidden`), whatever the ACL says. |
| O3 | *(r3.2: `unavailable` carries `cause = build \| config \| capability` plus a reason, which restores v1's actionable split between `unsupported` (rebuild) and `gated` (reconfigure).)* The reply goes on the operation's own concrete key. Success is a value reply; failure is `reply_err` with the core envelope (`invalid_request`, `not_found`, `unavailable`, `forbidden`, `fanout_forbidden`, `busy`, `internal`, `app`). |
| O4 | Only idempotent operations may be retried. |
| O5 | An empty reply set is not a verdict. ACL refusals return empty since zenoh 1.3. Attribute silence through presence. |
| **O6** *(r3.3: + `summary`, D10)* | An operation may declare `replies = "many"`: zero or more value replies, then completion. These are native Zenoh semantics. Callers MUST use consolidation `None`. *(r4)* `Latest` and `Auto` kept 1 reply of 10 in spike S6. `Monotonic` kept all 10 there only because the replies were unstamped. This covers listings, partial results and streamed answers. Work that outlives a query timeout belongs to `jobs.v1`. |
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
  unvalidated). *(r4)* The token form of the check is two alive instances of
  one service holding an interface token for the same interface, for longer
  than a grace period. The grace period MUST exceed D9a's make-before-break
  overlap, which is that same shape for a moment. The check found every
  split-brain in spike S6 and flagged no standby. Tools diagnose (`doctor`,
  #612); the runtime does not fence;
- *(r4)* `serving = "replicated"` costs one execution per router per call
  (spike S6), which is why it requires idempotent operations;
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
  *(r4)* Raw frames over SHM were zero-copy end to end: 0.02 MB allocated
  per 4 MB frame (spike S9). Two conditions apply:
  - **Memory locking.** zenoh-shm `mlock`s every segment it creates or maps:
    the pool, plus 1,280 KiB of metadata on first use.
    - A deployment SHOULD set `RLIMIT_MEMLOCK` to at least the pool plus
      2 MiB; zenoh's implicit 16 MiB pool needs about 17.25 MiB. At the spike
      host's 8 MiB limit, the pool could not be created and SHM fell back to
      TCP silently.
    - A metadata segment that cannot be locked panics.
    - On 1.10.1 the fallback is visible only on the receiving side, through
      the unstable `ZBytes::as_shm()`. Tools SHOULD therefore check the
      limit at start.
  - **Copies.** Protobuf `bytes` costs a copy on each side. Flatbuffers are
    zero-copy only when built in place in the SHM buffer (#611).
- **The typed layer's budget** *(r4)* is a per-message cost: at most 1 µs of
  CPU on the typed path for a control-size message (spike S9: 126 ns). The
  ≤ 5 % p99 target from #592 is deferred to a quiet, CPU-isolated host: on a
  shared host, end-to-end p99 varied by up to 4× between repetitions.

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

**Descriptor** *(r3.3: compact form, D8: the capabilities held, plus exceptions only)*. It is put on every change and answered on GET. It lists:

- the interfaces, each with its contract sha256 and minor, its exposed
  resources, and its unavailable optional resources with a cause
  (`build` / `config` / `capability`) and a reason;
- the capabilities this instance holds (r3.2), naming the gates its
  contracts declare;
- **the requirements with their bindings** (R3);
- the profiles;
- metadata (host, build, zid).

**Constrained faces (r3.2; r4: the mechanism).** No `@zk` traffic is ever *required* across a
face a deployment marks constrained (`link.v1`). Presence, descriptors and
bundles all stay local:
- bundles come from holders on the same side, or are pre-provisioned;
- across the face, only the resources `link.v1` exposes travel, downsampled as declared;
- the descriptor targets ≤ 1 KB, because v1's 50 KB `introspect` reply would cost about 185 SBD messages (two days of budget);
- *(r4)* **How the far side attaches decides what crosses** (spike S3, at 2,400 bit/s):
  - **Router to router,** every declaration crosses. A bring-up of 50 services cost 11.1 KB, and each coverage gap re-declared everything (11.8 KB).
  - **A client** of the near router receives only the declarations its interests ask for. With the `@zk` deny on, a bring-up cost 17 B and a gap 536 B. Without the deny, 200 tokens nobody asked for cost 322 B.
  - **A deployment SHOULD attach a single far-side session, or a gateway session, as a client,** with the `@zk` deny on the face. A **gateway** is the only session on its side that talks across the face. It is a principal of its own, and it never answers or republishes on the near side's keys: what it relays to its side, it re-keys under its own address, like §4.8's bridge. Anything beyond that is U23.
  - **The scope of that rule:**
    - zenoh 1.10.1 clients connect to one endpoint at a time, so a site with its own router, or a ground service commanding many vehicles, does not fit it. That is U23, whose router-level candidate is zenoh 1.10.1's `gateway.south` regions.
    - The spike shaped each TCP connection separately, so it did not measure several client sessions sharing one line.
- *(r4)* **On a router-to-router link, a `@zk` deny hides presence but does not keep it off the link.** The denied declarations still cross, key strings included (zenoh 1.10.1; `upstream/acl-denied-declarations-cross.md`, not filed).
- *(r4)* **A batch MUST cross the link well within the lease.** At 2,400 bit/s, the 64 KB default outlasted the 10 s lease and the link reconnected in a loop. With 1 KB batches it ran at link speed (spike S3). A link's batch is the minimum of the configured size, the link's MTU and both ends' settings, so a TCP router configured for 1 KB caps its local links as well. The constrained face then needs a router of its own, or a link type whose MTU bounds the batch. The cost of 1 KB batches on high-rate local traffic was not measured.
- *(r4)* **`@stream` keys SHOULD be denied across the face** unless `link.v1` downsamples them. Nothing is dropped: one named key queued 4 s of frames for 29 s at 64 kbit/s and for more than 600 s at 2,400 bit/s, and every request waited behind them.

**The constrained level** *(r4, U13)* relaxes the full level in two halves:
- **Devices** (spike S15, a zenoh-pico participant as owner):
  - timestamps from a wall clock with a per-session bump, plus the catch-up (§3.6 S7);
  - a literal deployment prefix in keys instead of a session namespace;
  - a descriptor small enough for one fragment;
  - bundles from a gateway holder where the device's receive limit (`Z_FRAG_MAX_SIZE`, 4 KB by default) bites. A pico *sent* 100 KB replies.
- **Links** (spike S3): the far side attached as a client, batches within the lease, `@stream` and `@zk` denied across the face. No presence crosses: on a router link, every coverage gap re-declares all of it.

**The presence budget** *(r4, U5)*. Presence costs per token, whatever the
layout: a router holds 2.1–2.4 KiB per token, and a router link carries
63–85 B per declaration. Discovery stays fast up to about 10k tokens and
then degrades (spike S2):

| Tokens | Discovery |
|---|---|
| 10k | 1.2–1.9 s |
| 15k | 3.3 s |
| 36k (ZenSight's shape, as spike S2 modelled it) | 23.8 s |
| 50k | 46–49 s, or never |

At 50k, a liveliness GET returned almost nothing within 10 s.

**A presence domain** is the set of routers whose router-to-router links
carry each other's declarations: the whole routed network, unless client
attachment or disjoint networks split it. A deployment SHOULD keep a domain
within about **10–15k tokens**, which kept discovery within 2–4 s on the
spike's host. The budget is shared:
- r3's layout spends 1 + the number of interfaces per service;
- device-as-service at 5,000 devices spends all of it (§3.5);
- queryables and subscribers are routed the same way, but spike S2 measured
  tokens only, so the budget may be optimistic.

How a deployment above the budget cuts the multiplier is U22.

**Contract retrieval** uses the key `zk2/@zk/contract/<iface>.v<major>/<sha256>`.
Every holder declares a `complete` queryable on it. The client:

1. GETs with `BestMatching`. *(r4)* That reaches the nearest holder on each router the query visits, so replies may come from several routers (spikes S4, S6);
2. verifies the sha256 of each reply **as it arrives**, and accepts the first valid one without waiting for the GET to complete *(r4)*. In spike S4 the slow holder and the unreachable holder were the nearest, and the valid reply came from behind a second router: waiting for completion cost the whole 1 s timeout;
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
(zenoh#2678). *(r4)* A liveliness GET on a session that already holds a
liveliness subscriber MUST use a callback or an unbounded handler: with the
default handler, such a session hung at every measured size from 996 tokens
(spike S2; a fresh session read 9,996 tokens in 679 ms). Make-before-break
re-minting (D9a) never left a service without a live instance token, and
one re-mint per second over 10k tokens cost 0.7 KiB/s (spike S2).

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
  *(r4)* The identity check is a normalized comparison of descriptor sets
  (source info and default `json_name` dropped). protox matches protoc 3.21
  byte for byte, and the normalization also recognizes `buf build`'s bytes
  (spike S7).

**Compatibility.** Every revision inside a major is FULL_TRANSITIVE
compatible: both directions, against every earlier revision. Payload types
are delegated to the schema kind's rules *(r4: for protobuf, no longer buf's
WIRE_JSON; see below)*.

*(r4)* **Protobuf uses WIRE semantics with renumber detection**, judged per
direction: a reader reads a writer's data, ignoring unknown fields and
defaulting missing ones.
- **Breaking:**
  - a field's declared scalar type changes, including int32 → int64 and
    string → bytes, which share a wire type (spike S7's matrix);
  - its cardinality changes;
  - it moves into or out of a oneof;
  - it is **renumbered**, which silently drops the data both ways.
- **Review:** a field renamed (its JSON name), an enum value renamed or
  deleted, a `json_name` option. zk2 protobuf is binary on the wire, and
  tools decode with the writer's bundle, fetched by fingerprint, so a name
  change relabels a display and breaks nothing running.
- **Compatible:** adding an enum value to a proto3 (open) enum. For a proto2
  (closed) enum it is review, because an old reader reads the unknown value
  as an unknown field and falls back to the default. Spike S7 tested proto3
  only.
- **Warning:** a field deleted without reserving its number. Reuse is caught
  against the whole history.

`buf breaking` run in both orders cannot express this: it calls adding a
field breaking in the swapped order. Spike S7 re-implemented buf's WIRE_JSON
rules on prost-reflect, and they agreed with `buf breaking` on 14 of 14 cases
in both orders. The decided WIRE rules depart from buf by design, on renames
and enum names. Spike S7's draft compatibility matrix carries the superseded
WIRE_JSON verdicts, and #607 regenerates it under this decision. **jsonschema** uses the zk2 subset of
2020-12, with protobuf's discipline: readers tolerate unknown properties,
and writers send only what their schema declares. Adding an optional
property, even to a closed schema, and removing one are therefore
compatible. Keywords whose containment is undecidable (`pattern`, `allOf`,
`if`/`then`, …) are refused (spike S7).

**Metadata changes are directional.** The r2 table is kept:

- `idempotent` true → false: breaking;
- `fanout` allowed → forbidden: breaking;
- `delivery`/`reliability` reliable → best_effort: review;
- optional → required: breaking for implementers;
- enum value added: review. *(r4)* This row is about contract-level
  enumerations, such as a template parameter's allowed values. Payload enums
  follow the schema kind's rules above.

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
| `timing.v1` | A, B, G | `period_ms`, `deadline_ms`, `lifespan_ms` annotations. Runtime: staleness flags, deadline-missed events, dead-man stops. *(r4, spike S11)* The deadline runs on the **receiver's** monotonic clock. A `lifespan` check against the sender's stamp MUST NOT be used unless clock skew is bounded well below the lifespan. At ±250 ms of skew it rejected every sample of a healthy commander. uhlc's default maximum delta (500 ms) is larger than both that skew and `twist_cmd.v1`'s 100 ms lifespan. Without such a bound, `lifespan` is **not enforced**: a receive-clock check cannot see that a sample was already stale on arrival. Control loops rely on the deadline, and on a sequence check where staleness in transit matters. |
| `arbitration.v1` | B | Consumer policies over many bound providers (priority + deadline, freshest, lease); optional `acquire`/`release` operations for explicit control authority. *(r4, spike S11)* The binding's order is the priority. A role's deadline comes from `timing.v1`, and a binding may tighten it. Arbitration is evaluated on every receive and on a timer of at most deadline/50. A liveliness delete drops a provider at once. No fresh provider means the dead-man value. A source's identity comes from its key, never its payload. Measured: takeover in one sample, silence caught at 100–102 ms, a crash in 4–5 ms, no false stops, and 7.0 % of a core for two actuators on a 2 ms tick. |
| `clock.v1` | G | A standard `clock.v1` interface (simulated, paused or real time); runtime rule that components read time through the bound clock. *(r4, spike S13)* The profile states its tick, and consumers extrapolate between ticks or accept tick granularity. |
| `archive.v1` *(r4)* | D, H, J | Last-known state (§3.6, rules S3, S5 and S6). It records owners' mutations under the archive's own address, with timestamps, type identity and tombstones. It answers explicit GETs only, and re-aligns with an owner that becomes reachable again. Its backend MUST NOT accept a put older than a delete it holds. Placement: on the consumer's side against link loss, on the owner's side against owner loss (§3.6). |
| `desired.v1` | H, D | Desired documents as **state of the authoring service, keyed by target**. Targets bind with `{target} = self`, converge, and report `observed_revision` in their own state (Kubernetes spec/status, device shadows). Store-and-forward rides rules S1–S7 plus archives (§3.6). *(r4)* Cancellation SHOULD be a put of a terminal document, not a delete, so that an archive that missed it cannot resurrect the plan (spike S5's resurrection case, under long outages). |
| `config.v1` | D | Commit-confirm configuration (set → apply → confirm window → confirm or roll back; hot versus reach groups), as a standard interface. |
| `jobs.v1` | C | Start → id, state `jobs/{job}` lifecycle with terminal states and retention, cancel; annotations mark an interface's own job resources. |
| `redundancy.v1` | C, B | Election, fencing epochs, role naming (§3.8). *(r4)* It owns exclusivity: O1's at-most-once holds only while one instance serves (spike S6). ZenSight's claim protocol, where a standby holds its instance token only, raised no finding and took no calls. |
| `blob.v1` | C, H | `@blob` kind: content-addressed bulk transfer with integrity rules. Delivery references carry (holder address, hash), never key prefixes. |
| `hostid.v1` *(r3.2)* | J, all adopters | System names minted from the machine id: v1's `h-<12hex>` derivation, with test vectors |
| `media.v1` *(r3.2)* | A | Codec tiers, the frame-age clock, receiver-driven adaptation, frame-metadata attachment (ZenSight parallax) |
| `link.v1` *(r3.2; r4)* | H | Per-resource exposure on constrained faces and downsampling, read by the face/ACL generator (zenoh-modem). *(r4, spike S3)* It also records the face's attachment (the far side as a client, within U23's scope), the batch size for the link's rate (about 1 KB at 2,400 bit/s, crossing well within the lease), the face's maximum outage (the floor for state windows, §3.6 rule S3), and the `@zk` and `@stream` denies. Across the face, an archive records only what `link.v1` exposes. |
| `views.v1` *(r3.2, r3.3)* | I | Presentation documents for generic UIs (ZenSight's `views`): content-addressed artifacts referenced by annotation and carried in the bundle's `extras` (D18) |
| `freshness.v1` *(r3.3)* | D, J | `ttl_s` is the staleness horizon; `0` means never stale (D16) |
| `health.v1`, `telemetry.v1`, `alarms.v1` | J, E | Health interface; metric annotations (unit, counter/gauge, histogram); active alarms as state plus transitions. |
| `lifecycle.v1` *(only if a use case proves it)* | B, C | Armed/disarmed or managed-component states. Not adopted by default. |

### 3.13 Security

The ownership invariant reduces ACL to three grant shapes:

| Grant | Rule |
|---|---|
| **Own** | A service principal publishes, deletes, declares queryables and declares tokens under `zk2/<system>/<service>/**`, plus its verbatim subtrees, spelled out because `**` never crosses one: `…/*/@stream/**`, `…/*/@state/**`, `…/*/@op/**`, `…/@zk/**` *(r4: `@state` added, spike S14)*. Contract bundles are open to all: any principal may hold or fetch `zk2/@zk/contract/<iface>.v<major>/*`, because the hash is the check (spike S14). An archive principal (§3.6) has Own on its own prefix, plus Consume on the keys it records. |
| **Consume** | Subscribe or GET on the prefixes a principal's bindings name. |
| **Call** | Query on specific `…/@op/<op>` keys. Privileged operations are listed one by one. |

There are **no cross-principal write grants**: no command reaches a
component except as a call or through its own bindings.

Plain facts the spec states:

- **grants compile per posture (r3.3, D13).** Under `default_permission: deny`, a grant is an allow. Under `allow`, it is compiled into denies of its complement, and regenerated on every contract revision. *(r4, spike S14)* Measured on a live router: deny + allows passed 13 of 13 checks; under `allow`, allow rules are not evaluated (U21); the complement denies blocked every unauthorized action except wildcard puts, which R6 stops;
- *(r4, spike S14)* **egress is checked by inclusion** against the query's or subscription's own key expression. Every consumer selector that intersects a provider's keys therefore joins that provider's egress grant, and the same selectors are granted for the provider's ingress `reply` (a refusal included). These are rules for the generator (#612);
- *(r4, spike S3)* **an ACL controls what the far side sees, not what a router link carries.** On a router-to-router link, a denied declaration still crosses, key strings included, so an ACL is not a confidentiality boundary for key names there. Over a client link, declarations travel only toward interests;
- **under `default_permission: allow`, a put on a wildcard key bypasses a
  deny rule on a concrete key it covers** (verified live, r3.1). P3's
  guarantees therefore rest on O2 (server refusal) and R6 (consumer
  filter). `default_permission: deny` is recommended where practical. It
  is node-global, and zenoh-modem's constrained faces run `allow` with
  face-scoped denies (U14, r3.2);
- deny works by inclusion, so deny with the widest pattern (`**/@op/**`);
- multicast transports bypass ACL;
- `zids` subjects are unauthenticated, so bind principals by certificate
  CN. *(r4)* Spike S14 used usrpwd subjects; certificate-CN subjects were
  not run;
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
- **Replay:** the recorder republishes under the original address in a
  `replay` namespace, with the detector's binding unchanged. Alternatively,
  the detector's `input` is rebound to `vehicle-01/replay-cam`. No code
  changes either way. *(r4)* Spike S13 ran both. The first needs a replayer
  whose session carries the namespace: zk2 `zenctl replay --namespace`,
  #612.
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
- **Store and forward** *(r4: through archives, §3.6)*:
  - **Two archives.** An archive on the vehicle (`vehicle-01/archive`)
    records `plans/vehicle-01`, against link loss. One on the ground
    records `plans/*`, against the fleet manager stopping.
  - **Reading.** The executor reads the owner while it answers, and the
    vehicle's archive when the owner gives no reply within the timeout,
    knowing that read is last-known (rule S6). While the link is up and the
    fleet manager is down, the vehicle's archive aligns from the ground's
    archive (rule S5). A plan written while the vehicle was offline, after
    which the fleet manager stopped, therefore still reaches the executor.
  - **Applying.** It applies a plan only if its timestamp is newer than the
    last one applied, which rejects stale commands. It converges and
    reports `state/plan_status` with `observed_revision`.
  - **Cancellation** is a terminal document, not a delete (`desired.v1`).
    The plans carry a tombstone window of at least the link's maximum
    outage (rule S3).
- **Measured:** spike S12 converged in 1.4–1.6 s after the link healed, with
  zero stale plans, using a storage and outages of about 1.5 s. The spec
  (#606) re-runs spike S12's cases against the archive form, with outages
  longer than the window.
- **The link** *(r4)*. How the ground side attaches across the vehicle's
  constrained face is U23 when the fleet manager serves many vehicles: a
  zenoh 1.10.1 client connects to one router.
- **Immediate actions:** `vehicle-01/executor` `@op abort` fails once the
  route is gone. *(r4)* Until the lease expires, a call waits out its own
  timeout: spike S6 saw 9 timeouts within the 10 s lease, then immediate
  failures. Over a link with a long lease (SBD's is an hour), that is the
  whole lease. The operator sees each timeout.

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

**4.9 ZenSight (r3.2).**

**Identity:** `system` = host id (`hostid.v1`).

**Presence** *(r4)*:
- **As spike S2 modelled it:** 36k tokens (1,000 hosts × 6 sensors × (1
  instance + 5 interfaces)), discovered in 23.8 s. That is well over the
  presence budget (§3.10).
- **The mapping's own count is higher:** 5 framework interfaces plus each
  sensor's own interface, so about 7 tokens per service and about 42k in
  all.
- **Under U22's lean** (no interface token for the framework set), it is 2
  per service, about 12k, within the budget.

ZenSight is U22's case.

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

**Links:** `link.v1` marks the four `link` resources that may cross an RF face (downsampled to 1/min) and none for SBD. No `@zk` traffic crosses. *(r4)* The ground's gateway session (§3.10) attaches to the node's router as a client across the RF face, with the `@zk` deny on the face. Batches on the face stay within the lease. zenoh-modem's own radio link type bounds them by its MTU; on TCP, the face needs a router of its own, with batches of about 1 KB. At 2,400 bit/s, zenoh's default batches made the link reconnect in a loop (spike S3). A ground site with several sessions, or its own router, behind one radio is U23.

**Contract crate:** `modem-contract` becomes a zk2 contract crate, with its bundle and generated traits, and zenoh glue behind a feature. Out-of-tree backends depend on it as they do today.

**Mixed versions:** FULL_TRANSITIVE inside `modem.v3` matches its own observation that "a field terminal and its ground gateway are upgraded months apart".

**4.11 tcgui (r3.2, the pilot).**
- **Identity:** `system` = host id; `service` = `tc`.
- **Requirement:** the frontend binds `tc.v1` as `backends` to `*/tc`.
- **Operations:** templated, exclusive, fan-out forbidden; `diagnostics` is fan-out allowed.
- **Full mapping:** `examples/zk2/tcgui/`.

---

## 5. Unresolved design decisions

The ones that decide the paradigm come first. *(r4)* The spike settled most
of them. In this table, "Sn" names a spike. The Lean column now gives each item's status: **r4** marks a
verdict, with the spike that measured it (`spike-report.md`'s decisions
table).

| # | Question | Lean, or *r4* status | Decided by | Fallback |
|---|---|---|---|---|
| **U-A** | P3 (consumer bindings) or P2 (sink-addressed inputs) for control and commanding | **Decided 2026-10-07: P3**; confirmed by S10, S11, S12 | Walkthroughs 4.2/4.3; spikes S11, S12 | Add one sink kind `@in` (verbatim; the owner discards samples whose key is not concrete, which works because subscribers see the publication key) |
| **U-B** | Is the binding configuration format normative? | Descriptor field normative, configuration shape recommended | Multi-language pilot (L) | A normative minimal TOML/JSON shape |
| **U-C** | Requirements in contracts, or only in the component manifest? | Both allowed; interface-level ones are fingerprinted | Walkthroughs | Manifest only (simpler compatibility, weaker contracts) |
| **U-D** | `@stream` as a kind token, or a contract attribute only | **r4: token.** No measurable cost (S9); ambient selectors carried none of its frames across a link (S3) | S3, S9 | Attribute + a recommended separate interface |
| **U-E** | QoS defaults per pattern; `priority` in the core | **r4: as in §3.3**; `express` stays opt-in (S9) | S9, S11 | Fewer fields; the rest to `timing.v1` |
| **U-F** | Decodability: two blessed kinds + allowed others | **r4: as in §3.9** (protobuf 126 ns per message; JSON affordable for documents; S9) | Pilot, S9 | Narrow to protobuf, jsonschema and raw |
| **U-G** | Operations with many replies in the core | **r4: yes (O6)**; consolidation `None` is necessary (S6) | S6 | `jobs.v1` only |
| U1 | State: producer + storage merged by timestamp, or owner-only + an explicit archive interface | **Decided 2026-10-08: the owner is authoritative, with `archive.v1`** (§3.6). The merged GET gave 10 wrong answers in 42 cases (S5) | S5, S12 | Merge, with a storage manager that cannot resurrect a deleted key |
| U2 | Clock discipline: HLC MUST? Restart regression strategy | **r4: HLC MUST, the catch-up (from a persistent record or an archive, else a new instance id), and a bound on clocks ahead** (S5, S12); a wall clock + the catch-up at the constrained level (S15) | S5, S12, S15 | Epoch in the value, which the new-instance fallback already carries |
| U3 | Tombstone window | **r4: 60 s by default**, a deployment setting of the owner (erratum, #606). Archives keep tombstones at least that long. State archived across a constrained face carries at least the face's maximum outage (S5) | S5 | n/a |
| U4 | `complete` operation queryables (O1) | **r4: keep `complete`; O1 reworded** (decided 2026-10-08; S6) | S6 | Non-complete; lose at-most-once |
| U5 | Token layout | **r4: kept** (A = B per token), with a **presence budget** of about 10–15k tokens per domain (S2); the multiplier is U22 | S2 | Instance tokens only + descriptors |
| U6 | Mandatory one-chunk `system` | **Holds** on paper (S8) | S8 | Revisit only on fake names |
| U7 | Bundle stability | **r4: holds**; the identity check is a normalized comparison (S7) | S7 | Normalized description per kind |
| U8 | Error envelope encoding | Core `Error` in protobuf and jsonschema, `detail` in the interface's encoding | Spec draft | Always JSON |
| U10 | Large collections | **r4: no paging**. 100k keys came back in ≤ 366 ms with owner + storage and `Latest` (S5). The owner alone is measured in #606 | S5 | Paging on GET parameters |
| U11 | Descriptor dynamics | **r4: put + GET** (S2) | S2 | GET only |
| U12 | Redundancy: diagnose or delegate | **r4: diagnose** (the token check found every split-brain) **and delegate** to `redundancy.v1` (S6) | S6 | n/a |
| U13 *(r3.1)* | A constrained conformance level for zenoh-pico participants (no namespace, no HLC, 4 KB fragments) | **r4: defined in two halves**, devices (S15) and links (S3): §3.10 | S15, S3 | zk2 requires the full Rust-core feature set |
| U14 *(r3.1, r3.2)* | Is `default_permission: deny` a deployment MUST for P3's guarantees? | **r4: SHOULD**, confirmed live (S14) | S14 | n/a |
| U15 *(r3.1)* | The storage-manager position: depend on it, require a fixed version, or no storage on `state/**` | **r4: no storage on owners' state**; archives only, whose backend must not resurrect a deleted key. zenoh 1.10.1's storage manager does (the draft upstream report is not filed) | S5 | n/a |
| U16 *(r3.2)* | Occurrence-keyed streams in the core, or an `events.v1` profile | **r4: core** (D3). Union replay is right; `retention` needs a time-series backend (S5) | S5 | Profile |
| U17 *(r3.2)* | Call metadata (`actor`, `request_id`) as a core convention | Optional core attachment, claimed only. The attachment works (S6) | Spec draft | Left to `config.v1` and applications |
| U19 *(r3.3)* | The `events` kind token, with cardinality = rate × retention | **r4: yes** (S5) | S5 | Occurrence streams under `stream` plus an `events.v1` profile |
| U20 *(r3.3)* | `@state` for large populations | **r4: yes.** Per-entity tokens break down between 15k and 50k (S2); 100k `@state` keys read in ≤ 366 ms with owner + storage (S5) | S2, S5 | A verbatim system token for singleton services |
| U21 *(r3.3)* | Does zenoh evaluate allow rules under `default_permission: allow`? | **r4: it does not**, confirmed live; D13's complement denies work (S14) | S14 | n/a |
| U18 *(r3.2; r3.3: member tokens, D9)* | Device-as-service at ZenSight's SNMP scale (thousands of devices per poller): token and descriptor cost | **r4: device-as-service to about 5,000 devices per domain** (15k tokens, 3.3 s); member tokens above (S2) | S2 | Per-device presence as a template-scoped liveliness token under the parent service |
| **U22** *(r4)* | A deployment above the presence budget: how does it cut the per-service multiplier (1 instance + one token per interface)? | **Decided 2026-10-08, at the spec's acceptance:** a deployment-configured tokenless set, recommended for the framework interfaces every service implements. The descriptor marks them `"token": false`, and their providers are found through instance tokens and descriptors. A contract-level flag was rejected: it would have changed every fingerprint (spec §8.1) | Spec review (#606) | Instance tokens only, with interfaces read from descriptors |
| **U23** *(r4)* | The far side of a constrained face when it is more than one session: a ground site with its own router, or a service commanding many vehicles (§4.3) | **Decided 2026-10-08:** measure zenoh 1.10.1's `gateway.south` regions (a far router placed south by zid, interface or region name), before `link.v1` (#613) | A spike follow-up | One gateway session per link, re-keying what it relays under its own address |

(r2's U9, profile binding, is now settled: `uses` in the contract plus
`profiles` in the descriptor.)

---

## 6. Maintainer decisions

| # | Decision | Recommendation |
|---|---|---|
| **0** | **Paradigm** (new) | **Decided 2026-10-07: P3**, service-owned keys + consumer bindings. |
| 1 | ZenSight | Freeze v1 at 1.50 / 0.14.x. Port tcgui first. Decide ZenSight after the tooling phase. |
| 2 | Mandatory `system` | Yes, provisionally (S8) |
| 3 | Kind in the key | Yes: `stream`, `@stream`, `state`, `@op`; r3.3 added `@state` and `events` |
| 4 | Schema kinds | Protobuf by default; jsonschema first-class; flatbuffer, ros2msg and raw allowed and rendered honestly |
| 5 | Ownership | Exclusive per resource by default; replicated operations explicit; no core role |
| 6 | Repository and naming | Same repository. A `v1` branch for the frozen line; main takes the new architecture; 0.x until the core spec is frozen. Move production zenctl builds to `v1` first, and decide the open 0.15.0 milestone. |
| 7 | Pilot | **Decided 2026-10-07: tcgui.** The criteria it meets: a real, non-safety-critical company component with **a data-flow or control path** (at least one bound requirement), a state, an operation and a parametric resource, restart and reconnect behaviour, and preferably two machines. A pure supervision pilot would not validate r3. |
| 8 *(r4)* | State answers (U1) | **Decided 2026-10-08:** the owner is authoritative, and last-known state lives in `archive.v1` (§3.6). |
| 9 *(r4)* | Operation exclusivity (O1) | **Decided 2026-10-08:** at most once only while one instance serves; split-brain diagnosed from tokens; exclusivity in `redundancy.v1` (§3.7). |
| 10 *(r4)* | Protobuf compatibility | **Decided 2026-10-08:** WIRE semantics with renumber detection; JSON-name and enum-name changes are review (§3.11). |
| 11 *(r4)* | Upstream reports | **Decided 2026-10-08:** the two drafts in `docs/zk2/upstream/` are not filed. |
| 12 | The core spec | **Accepted 2026-10-08:** `spec/core.md` v0.1 (#606), with U22 decided and U23 to be measured. |

---

## 7. Spike plan

The spike is throwaway code that produces measured numbers. **The paradigm
tests (S9–S13) run first.**

*(r4)* Every spike has run. The results are in
[`spike-report.md`](spike-report.md), and the code is on branch `zk2-spike`,
tagged `zk2-spike-final`. The table is kept as the plan of record.

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
# r4: lifespan is not enforced unless clock skew is bounded well below 100 ms; the deadline is (§3.12)
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

## Appendix B: Zenoh facts relied on (1.10.1 source; r4: measured)

| Fact | Source |
|---|---|
| Liveliness tokens carry no payload | `commons/zenoh-protocol/src/network/declare.rs` |
| Liveliness and data are separate key spaces | `src/api/liveliness.rs`, `src/net/routing/hat/*/token.rs` |
| Routers hold every token; clients and peers receive tokens on interest | `src/net/routing/hat/{router,client,peer,broker}/token.rs` |
| Verbatim = a chunk starting with `@`; `*`/`**` never match it | `commons/zenoh-keyexpr/src/key_expr/borrowed.rs` |
| **A put on a wildcard key is legal, and subscribers receive the publication's key, not their own** | `src/api/session.rs` (put resolution has no wildcard check; `subscriber_callbacks` passes the incoming key, ~l.498–520) |
| `BestMatching` = the nearest `complete` queryable whose key includes the query's key, else `All`. *(r4, measured)* "Nearest" is per router: a query reaches one such queryable on each router it visits | `src/net/routing/dispatcher/queries.rs`; S4, S6 |
| `Latest`: per key, greatest timestamp (`None` lowest), delivered at completion; `None` consolidation delivers every reply | `src/api/session.rs` (~l.3540–3605) |
| Routers stamp puts only, where timestamping is enabled; future-dated puts are re-stamped | `src/net/routing/dispatcher/pubsub.rs` (`treat_timestamp!`); `DEFAULT_CONFIG.json5` |
| `Query::reply_del` is stable; `Session::new_timestamp()` = HLC, else wall clock + zid | `src/api/queryable.rs:437`; `src/api/session.rs:1038` |
| Replies inherit the query's QoS | v1 RFC 04 §3 (verified there against zenoh's no-op reply-QoS setters) |
| Encoding schema suffix ≤ 255 bytes, sent per put | `commons/zenoh-codec/src/core/encoding.rs` |
| ACL by inclusion; multicast bypasses ACL; `zids` unauthenticated | `src/net/routing/interceptor/{authorization,access_control}.rs`; `DEFAULT_CONFIG.json5` |
| `SourceInfo` unstable and unvalidated; matching status is a boolean | `src/api/{sample,matching}.rs` |
| `liveliness_query` deadlock past ~256 tokens on the default handler (open). *(r4)* A session already holding a liveliness subscriber hung at every measured size from 996 tokens (the first point measured above 96). A fresh session read 9,996 tokens in 679 ms | eclipse-zenoh/zenoh#2678; S2 |

**Measured by the spike (r4).** Each row cites its section of
[`spike-report.md`](spike-report.md).

| Fact | Spike |
|---|---|
| `BestMatching` reaches one `complete` queryable per router. Under a split-brain across routers, a concrete call executes on each side | S4, S6 |
| Routers re-stamp future-dated puts, but not GET replies | S12 |
| zenoh-ext parses advanced-publisher keys with `${remaining:**}/@adv/…` (`advanced_cache.rs:40`). It cannot cross a verbatim chunk, so late-publisher detection and heartbeat recovery fail under `@stream`, while the initial history query works | S1 |
| zenoh-shm `mlock`s every segment it creates or maps (`shm/unix.rs:291`). Under the memlock limit, SHM silently falls back to TCP, and a metadata segment that cannot be locked panics (`metadata/storage.rs:31`) | S9 |
| Discovery: 1.2–1.9 s at 10k tokens; 46–49 s or never at 50k. Routers hold 2.1–2.4 KiB per token, and router links carry 63–85 B per declaration | S2 |
| A router-to-router link carries every declaration. A client link carries only those its interests ask for | S3 |
| An ACL deny does not stop a denied declaration, key string included, from crossing a router-to-router link. It hides it from the far side | S3; `upstream/acl-denied-declarations-cross.md`, not filed |
| At 2,400 bit/s, the default 65,535 B batch outlasts the 10 s lease, and the link reconnects in a loop. `transport/link/tx/batch_size` = 1024, or a 60 s lease, restores link speed | S3 |
| The storage manager 1.10.1 accepts a put older than a delete it holds (`guard_cache_if_latest`), and its GC keeps the wrong side of the limit | S5; `upstream/storage-manager-outdated-guard.md`, not filed |
| The storage manager's memory backend ignores `_time` | S5 |
| zenoh-pico 1.10.1 stamps with a wall clock plus a per-session bump. `Z_FRAG_MAX_SIZE` = 4096 bounds what it receives, not what it sends | S15 |

**Read in the 1.10.1 sources for r4:**

| Fact | Source |
|---|---|
| uhlc's default maximum clock delta is 500 ms, overridable by `UHLC_MAX_DELTA_MS` | `uhlc-0.8.2/src/lib.rs:89` |
| A router can drop future-dated puts instead of re-stamping them (`timestamping.drop_future_timestamp`, default false) | `DEFAULT_CONFIG.json5:219` |
| A link's batch size is the minimum of the configured size, the link's MTU and the unicast bound, negotiated down to the smaller end's | `zenoh-transport` `unicast/establishment/open.rs:318`, `:652` |
| A client connects to a single endpoint at a time | `DEFAULT_CONFIG.json5:50` |
| `gateway.south` regions assign remotes to south-bound subregions by zid, interface or region name (`"auto"` by default) | `DEFAULT_CONFIG.json5:250–273`; `dispatcher/region.rs` |
| `ZBytes::as_shm()` exists only with the `unstable` and `shared-memory` features | `src/api/bytes.rs:256–260` |
| A query timeout is delivered as a reply error `"Timeout"` | `src/api/session.rs:2760` |
