# Moving from v1 to zk2

This note is for a team that runs keyspace-v2 v1 (zenkey 0.11.x, the RFCs
at v1.50) and is weighing zk2. It maps what v1 had onto what zk2 has, says
what a port involves and what zk2 does not have yet, and takes the three
adopters one by one. Issue #614 owes it.

It is not normative. The rules are [`spec/core.md`](../../spec/core.md),
cited "core §n", and the profiles in
[`spec/profiles/`](../../spec/profiles/README.md). The design and its
reasons are [`architecture.md`](architecture.md), cited "r4 §n" (r4 keeps
r3's section numbers).

## 1. Who this is for, and the two lines

You run v1 if your application keeps a `registry/` of TOML or KDL files,
calls v1's `zenkey_build` from its build script, and declares an
`AppProfile` static. Two lines live in this repository now:

| | The v1 line | The zk2 line |
|---|---|---|
| Branch | `v1` | `main` |
| Rules | `rfcs/`, frozen at v1.50: errata only | `spec/core.md` 0.27, and three draft profiles |
| Libraries | `zenkey` and `zenkey-build` 0.11.x, `zenkey-fleet` 0.18.x, on crates.io | `zenkey`, `zenkey-build` and `zenkey-fleet` 0.20.0, and the new `zenkey-model`: all `publish = false` until the spec issue, #606 |
| Tools | zenctl, zengui and zenwatch, released as 0.14.x | the same three, ported. No release carries them yet: the last tag, 0.14.0, predates the port. |
| Also | v1's sources and the `fixture-tests` corpus | a second implementation of the spec, [`impl/python/`](../../impl/python/README.md) |

- **One line per consumer.** zk2 reuses the crate names. `zenkey = "0.11"`
  from crates.io is v1, and zk2 builds from `main`. A finished port links
  one line. While a port is under way, Cargo's `package =` rename can hold
  both: zengui ported pane by pane beside
  `zenkey-fleet-v1 = { package = "zenkey-fleet", version = "=0.18.0" }`,
  and removed it once nothing read it (`CLAUDE.md`, the FL decisions).
- **v1 stays maintained.** Fixes land on the `v1` branch and ship as
  0.14.N tags. A production source build tracks `--branch v1`, or pins the
  latest tag ([`README.md`](README.md), "v1").
- **Both lines share a bus.** `v1/…` keys and `zk2/…` keys never match each
  other's selectors (core §1.7), so v1 and zk2 services can run side by side
  while a port moves. zk2's tools show a v1 key as a foreign key, `not zk2`,
  with no v1 overlay.

## 2. The concept map

| Before (v1) | Now (zk2) | Where |
|---|---|---|
| `<base>/v1/<origin>/<class>/<producer>/<subject…>` (RFC 03 §1) | `[<namespace>/]zk2/<system>/<service>/<iface>.v<major>/<kind>/<resource…>`. Positions 1–5 have fixed arity, so `zk2/*/*/*/state/**` is exact, and "every provider of X" is the interface chunk. | core §1.1, §1.3 |
| `<base>`, the session namespace | Unchanged: the namespace, added on egress and stripped on ingress. Keys are base-relative. | core §1.6 |
| Classes `telemetry`, `state`, `events`; planes `@rpc`, `@media`, `@blob` | Kind tokens `stream`, `state` and `events`, and the verbatim `@stream`, `@state` and `@op`, which no ambient selector reaches. A profile's verbatim kind, such as `@blob`, is reserved and refused for now. | core §1.2, §1.3 |
| An origin: a host (`h-<12hex>`) or a service origin (`@catalog`, `@desired`) | An address, `<system>/<service>`, plus an instance id per run, which is the continuity epoch. A service origin becomes a plain system: `fleet/catalog`. | core §1.5; [zensight](../../examples/zk2/zensight/README.md), "Shape" |
| A producer and its subjects; a device as a key chunk | A service that implements interfaces. Resources are templates with typed `{param}` and rest `{param...}` chunks. A device becomes a service of its own (device-as-service), or a template parameter. | core §1.5, §2.2 |
| The registry (RFC 08 §2), its slices, and `deprecated.lock` | Contracts in the authoring format (draft 1), one file per interface major. Each has a canonical form, whose sha256 is its fingerprint, and ships as a bundle. Published bundles go in an append-only `.history`. Revisions inside a major are FULL_TRANSITIVE compatible, and a resource can be marked `deprecated`. | core §9; [`examples/zk2/README.md`](../../examples/zk2/README.md) |
| `AppProfile` and its salt, one per application (RFC 06 §1) | `hostid.v1`: one salt for every application, `zk2-hostid-v1`. A system is minted only when the configuration asks for one (`address = "@hostid.v1/<service>"`). The construction is v1's, but the salt is not, so every minted name changes. | hostid.v1 §2.1–§2.3, Appendix A |
| Five named QoS profiles: `sampled`, `refreshed`, `transition`, `alert`, `frame` (RFC 04 §3) | Four wire axes on each resource (`reliability`, `congestion`, `priority`, `express`), with defaults per kind. They are fingerprinted, and the generated writer applies them. | core §2.4 |
| `ttl_s` required on every state subject | `freshness.ttl_s`, optional, on streams too. A re-put is a mutation with a fresh stamp. | freshness.v1 §2.3, §2.4, Appendix A |
| `cardinality`, judged by tools | `cardinality` is required on every templated resource. The population budget is core, and the runtime refuses a new member past the bound. | core §2.2, §2.7 |
| The framework state set (`common = "…"`, RFC 04 §1.4): health, alerts, evidence | Profiles' standard contracts (`health.v1` today) and the application's own interfaces, found through descriptors. A deployment may leave them tokenless. | core §8.1; health.v1 Appendix A |
| `@rpc` procedures and `error/*` replies (RFC 05) | Operations on `@op`, under rules O1–O7: a concrete, `complete` queryable, and fan-out only where `fanout = "allowed"` (the default is forbidden; v1's reads defaulted to allowed). Failures carry the error envelope, whose `unavailable` names its cause: `build`, `config` or `capability`. A read that returns a document becomes state. | core §5; [tcgui](../../examples/zk2/tcgui/README.md), "Decisions recorded here" |
| Configuration (RFC 05 §5.1) | `config.v1`, being specified under #613, and not yet in `spec/profiles/`. zenoh-modem's binding is its reference. | [`config.v1-binding.md`](../../examples/zk2/zenoh-modem/config.v1-binding.md) |
| A storage on `state/**` as the late joiner's seed (RFC 04 §3.5) | The owner answers, with S4's GET (target `All`, consolidation `Latest`). A deployment MUST NOT run a storage that answers on an owner's `state/**` or `@state/**` keys. A late joiner subscribes first, then GETs, and merges by the owner's stamp. Last-known state comes from an `archive.v1`, read explicitly and never shown as current. Union storages on `events/**` stay. | core §4.2, §4.4, §2.6 |
| `alive` tokens (RFC 04 §5), the `sensor` document, and the roster built from them and `@catalog` | An instance token for every instance, pure consumers included, and an interface token per interface it exposes something of. The descriptor lists exposure, capabilities, bindings and profiles. Bring-up follows a fixed order, so alive means callable. | core §3.3, §8.1, §8.2 |
| `introspect` and `describe` (RFC 08 §6, §7) | The descriptor, and each contract's bundle, retrieved by fingerprint and verified by its hash. | core §3.3, §8.4 |
| Registration (`registered`, `unregistered`, no slice) and a three-pole payload verdict (RFC 13 §3) | Identity rungs, from the namespace and the key's form through provider, revision, contract and resource to the decode. Conformance has four poles: valid, invalid, undecodable, not checked. | [`tooling-guide.md`](tooling-guide.md) §3, §6 |
| Two publishers MUST NOT share a subject (RFC 03 §1.5), but a tool wrote any key: `zenctl pub` and `retire` did | P3: a key is written only by the service whose address prefixes it. Data reaches a component through roles bound to providers (R1–R7). The tools refuse to write a zk2 key. | core §6, §3.2; r4 §2 |

## 3. What a port involves

1. **Write the contracts from the registry.**
   - One file per interface major. Split by what a consumer binds
     separately: tcgui's one `tc.toml` became three interfaces.
   - Mutations stay operations, and their request types drop fields that
     repeat a template parameter (lint W102). Reads that return a document
     become state.
   - `introspect`, `describe` and `sensor` are deleted: the descriptor and
     the bundle replace them.
   - Pick a schema kind. JSON Schema, Rust-first, means schemars output,
     committed (tcgui, zenwatch). Protobuf is the walkthrough's choice.
   - Check and publish with the contract CI, the `zk2` binary in
     `zenkey-model`. Its exits are 0, 1 and 2, as zenctl's.

     ```bash
     cargo run -p zenkey-model --bin zk2 -- contract lint contracts/*.toml
     cargo run -p zenkey-model --bin zk2 -- contract fingerprint contracts/tc.netem.v1.toml
     cargo run -p zenkey-model --bin zk2 -- contract bundle contracts/tc.netem.v1.toml --history contracts/.history
     ```

     `contract compat` and `contract check-history` gate later revisions
     against the history.
2. **Generate code with `zenkey-build` 0.20** ([`codegen.md`](codegen.md)).

   ```rust
   // build.rs
   zenkey_build::Config::new()
       .contracts_dir("contracts")
       .history_dir("contracts/.history")
       .json_type("json:Status", "crate::model::Status") // a Rust-first type
       .generate()?;
   ```

   - It generates one module per interface: its types, `BUNDLE` and
     `FINGERPRINT`, a `Server` to serve it and a `Consumer` to read it.
     With operations come `Handlers`, `Client` and `Api`, and `Fleet` where
     one allows fan-out.
   - A lint error, or a revision `breaking` against the history, fails the
     build. A `review` finding is a warning.
   - `contract_crate(true)` puts the runtime behind a `zenoh` feature, for
     a crate that out-of-tree implementers depend on without zenoh.
3. **Move writers onto `ServiceBuilder`, and readers onto `Consumer`.**
   - A service is configured, never coded: its address (literal, or
     `@hostid.v1/<service>`) and its bindings.
   - `Server::declare` on the builder, then `start()`, which brings it up
     in core §8.2's order. The typed writers apply the contract's QoS and
     `Encoding`, and state writers stamp every put and delete (S1).
     `ServiceBuilder::health()` gives `health.v1`.
   - A reader binds a role (`Consumer::bind(&service, role)`) and gets
     typed `subscribe_…`, `get_…` (S4) and `last_known_…` (an archive).
   - A pure consumer is still a service, so that it appears in the graph:
     tcgui's frontend holds an instance token and no interface token. A
     tool without a service uses `Consumer::for_tool` and `Client::new`.
4. **Map the hand-built keys.**
   - List every key built with `format!`, every positional parser, and
     every key string inside a payload or a configuration file.
   - Generated handles cover the keys a contract declares. For the rest,
     use `zenkey::model::grammar` (`data_key`, `kind_selector`, `parse`)
     and `zenkey::model::template::Template::build`, which slugs values,
     never `format!`.
   - Payloads carry identities as structured fields (system, service,
     resource), never as key strings (r4 §0.2).
   - On each host, `zenctl hostid --v1-salt <salt>` prints the zk2 system
     beside the v1 origin it replaces. The hash is one-way, so only the
     host can compute the pair (hostid.v1 Appendix B).
5. **Change the deployment files.**
   - Remove every storage that answers on an owner's `state/**` or
     `@state/**` (S4). `zenctl storage gen --enrollment … --contracts …`
     plans the union storages for events, and refuses any selector over an
     owner's state.
   - Run an `archive.v1` where last-known state is needed (core §4.4).
   - Regenerate access control with `zenctl acl gen`, from the contracts
     and a zk2 enrollment ([`examples/zk2/acl/`](../../examples/zk2/acl/)).
   - For the tools to check S4, enable the routers' admin space,
     read-only, for the tools' principals (core §4.2).
6. **Check it.**
   - `zenctl service list` and `service show <system>/<service>` show what
     presence and the descriptors say.
   - `zenctl doctor --namespace <ns>` judges the deployment: nineteen
     checks, among them `split-brain`, `binding-unsatisfied`,
     `token-missing` and `storage-on-state`.
   - `zenctl check conform <system>/<service> <iface>` judges one service
     against its contract: the contract and resources served, payload
     types, QoS, operations, fan-out refused, state stamps and GETs,
     freshness, budget and health.
   - `zenctl watch <system>/<service> <iface> <resource>` prints each
     sample, decoded through the contract.
   - The judgements exit 0 when clean, 1 on a finding, and 2 when they
     cannot say.
   - The proof that a cutover is done is still RFC 13 §6's: the old root
     silent while the new keys speak, and a probe that builds concrete keys
     as the product does. `check cutover` stayed on the `v1` branch
     ([`zenctl/CHANGELOG.md`](../../zenctl/CHANGELOG.md), FJ9).

**Worked examples.**
- **The walkthrough:** [`examples/zk2/walkthrough/`](../../examples/zk2/walkthrough/),
  narrated in r4 §4.1–§4.3, in protobuf, with operations, arbitration and
  a durable command. `zenkey-build/fixtures/codegen-test` generates and
  compiles it.
- **tcgui:** [`examples/zk2/tcgui/`](../../examples/zk2/tcgui/README.md)
  maps every entry of a v1 registry, in Rust-first JSON Schema, with a
  bindings file and an access-control enrollment.
- **zenwatch's own state:** a v1 producer ported in this repository
  (FL3). [`zenwatch/contracts/zenwatch.v1.toml`](../../zenwatch/contracts/zenwatch.v1.toml)
  and its `build.rs` are the result, and
  [`zenwatch/CHANGELOG.md`](../../zenwatch/CHANGELOG.md) ("What it
  publishes") maps each v1 key.

## 4. The tools

zenctl, zengui and zenwatch are zk2's on `main`, and none of them links a
v1 crate. The `v1` branch keeps building and releasing their v1 versions.

- **`--namespace`** names the deployment. `--base` stays as its alias,
  and the `ZENCTL_BASE`, `ZENGUI_BASE` and `ZENWATCH_BASE` variables stay
  as rungs. Resolved reads use a session in the namespace, and raw wire
  selectors a session in none.
- **`--contracts`** replaces `--registry`. It takes an authoring file, a
  directory of them, or a `.history` root, and contracts not held there
  are retrieved from the bus. zenctl no longer has `--registry`. zengui
  refuses it by name (exit 2), and zenwatch refuses `bus.registry` the same
  way: its key is `contracts` now.
- **`--origin`, `--class` and `--producer`** give way to a wire selector,
  or an address `<system>/<service>`.

Each tool's CHANGELOG has the old→new tables, one row per spelling, each
naming where its question went:
- [`zenctl/CHANGELOG.md`](../../zenctl/CHANGELOG.md): FJ4 (the inspection
  nouns), FJ5 (`call`, `get state`, `watch`), FJ9 (what left with v1), FK1
  (`why` and `check conform` in zk2's terms), then each profile's entry;
- [`zengui/CHANGELOG.md`](../../zengui/CHANGELOG.md): "zengui on zk2
  (#614)", surface by surface;
- [`zenwatch/CHANGELOG.md`](../../zenwatch/CHANGELOG.md): FL2 (the rules,
  the doctor's check ids, the state file) and FL3 (what it publishes).

Two changes need a hand during the move. zenwatch refuses a version-1
state file rather than migrating it, so move it aside. `origin-down`
rules become `instance-gone <system>/<service>`.

## 5. What zk2 does not have yet

zk2 has three profiles, all drafts: `hostid.v1`, `freshness.v1` and
`health.v1`. Archives have their core rules (core §4.4) and a minimal
runtime (`zenkey::archive`), while the `archive.v1` text is #613's. The
rest of #613 is not written:

| Profile | What waits on it |
|---|---|
| `config.v1` | zenctl's `config`, zengui's Config tool, zenoh-modem's configuration plane |
| `blob.v1` | zenctl's `blob`, zengui's Blob browser, ZenSight's artifacts |
| `media.v1` | zengui's Media viewer, ZenSight's parallax. An explicit `@stream` carries frames meanwhile. |
| `alarms.v1` | Specified (#800, text 0.8): the runtime's owner (`zenkey::alarms`), zenctl's `alarms`, the watchdog's and `record --on`'s `alert-firing`, the doctor's `alarm-key` and `alarm-stale`, zenwatch's `alerts` and `alert-firing` (#800's AD). Still waiting: ZenSight's alerts |
| `redundancy.v1` | Specified (#841, text 0.1, core 0.39): a governed interface's claim election, its standby marked in the descriptor, a vacancy, the heal of a split, consumers following the incumbent, `external` for an actuator's own lock, and the session-free reading of each (`zenkey_model::redundancy`). Still waiting: the runtime's `ServiceBuilder::redundancy()` (#841's RB), the doctor reading the mark, `hostid.v1` §2.12's standby recognised and zenctl's questions (its RC); then ZenSight's catalog and desired singletons |
| `desired.v1` | durable commands keyed by their target; ZenSight's desired state |
| `link.v1` | per-resource exposure across a constrained face; zenoh-modem's RF and SBD faces |
| `telemetry.v1` | Specified (#819, text 0.5): its vocabulary and standard schema, `telemetry.v1.Point`, and the session-free reading of a declaration, a sample's bytes, and a pair's continuity and rate (`zenkey_model::telemetry`); the runtime's writers refuse what a metric's declaration refuses and write the epoch, the instance id (`zenkey::telemetry`, #819's TB); the fleet's continuity reader (`zenkey_fleet::model::telemetry`), `zenctl telemetry`, `check conform`'s `telemetry-declaration`, `telemetry-sample` and `telemetry-continuity`, and the doctor's `telemetry-declaration` and `telemetry-sample`, in place of `kind-mismatch`, with every payload check reading a metric sample by the profile (#819's TC); zengui reads a metric's kind and unit, and plots its value and a counter as its rate (#819's TD); a counter or histogram that formed no pair, or only assumed ones, could not tell (text 0.4, #819's TE), and so could a window holding an undecodable sample or a member under an invalid declaration (text 0.5, #819's TF) |
| `views.v1` | ZenSight's GUI's presentation documents |
| `timing.v1` | Specified (#908, text 0.2, core 0.55): a stream's period, its deadline judged per member on the reader's monotonic clock (missed, met and ended, seen within max(deadline/50, 1 ms)), its lifespan on a clock bound the deployment states, and the session-free reading of each (`zenkey_model::timing`). The runtime (#908's MB, text 0.2, core 0.55): a consumer's deadline events and lifespan on its binding's `clock_bound_ms`, every `Delivery`'s arrival and stamp, an owner's stamps (`zenkey::timing`), and the owner example's `--timing` with `consume --timing`. Still waiting: the doctor's `timing-declaration`, `check conform`'s `timing-period` and `timing-deadline`, `zenctl timing` and zengui's Inspector (its MC) |
| `arbitration.v1`, `clock.v1` | arbitration between commanders and the dead-man value (#909), simulated time |
| `jobs.v1` | work that outlives a query timeout |

- **Writing against them already works.** A contract may list one of these
  in `uses` and use draft 1's interim vocabulary for it (core §10,
  Appendix D), so tcgui's, zenoh-modem's and ZenSight's contracts load
  today. What the profile itself defines, its rules and its standard
  contract, is not there to rely on.
- **Dark in the tools until their profiles exist:** `blob` and `export`.
  `config` came back with `config.v1` (#768), the alert rules with
  `alarms.v1` (#800): the watchdog's `alert-firing`, and zenwatch's
  `alerts` and `alert-firing` (one notification per alarm, its selector
  base-relative: `zk2/…`), and the kind checks with `telemetry.v1` (#819):
  `zenctl telemetry`, the `telemetry-*` checks and cases, and a metric
  sample read by the profile wherever a payload is checked.
  `field-stuck` has its profile in `freshness.v1`, and waits to be ported
  onto it (tooling guide §6).
- **The population budget is not dark.** It went dark at FJ9 with v1's
  other profile-backed checks, and came back as core in 0.24 (#735): `check conform`'s `budget` cases and
  the doctor's `population-over-bound` ask it.
- **Removed, not dark:** zenwatch's inhibition read v1's catalog, and no
  profile brings it back.

A team that needs one of these stays on the v1 line until it exists. The
`v1` branch keeps the tool features that went dark, and inhibition.

## 6. Per adopter

**tcgui, the pilot** (decided 2026-10-07). Its whole v1 registry (11
subjects, 8 procedures) is mapped: three interfaces, `tc.netif.v1`,
`tc.netem.v1` and `tc.scenario.v1`, plus `health.v1`, on the service `tc`
of a system minted by `hostid.v1`. The frontend is a pure consumer that
binds each interface to `*/tc`. It needs only the three published
profiles. Its planned blob tier is out of the pilot. Its contracts already
generate in `zenkey-build/fixtures/codegen-test`, and zengui's test scenes
and demo traffic run them. The port is tcgui's own, in its repository; the
mapping's gap 4 asks it to drop the identity fields its payloads duplicate
([README](../../examples/zk2/tcgui/README.md)).

**zenoh-modem.** One service per device (`rf0`, `sat0`, `wwan0`) on a
system that is the host id, implementing `modem.v3`: 86 resources, 80 of
them optional with their v1 gates. `modem-contract` becomes a zk2 contract
crate, without zenoh by default, and out-of-tree backends keep depending
on `modem-device` unchanged ([`contract-crate.md`](../../examples/zk2/zenoh-modem/contract-crate.md)).
Two profiles it needs are not written: `config.v1` for its configuration
plane, whose types the crate depends on, and `link.v1` for its faces
([`link.md`](../../examples/zk2/zenoh-modem/link.md)). Whether to split the
SDU plane out of `modem.v3` must be decided before it freezes (the
mapping's gap 6). Its constrained faces are core §8.5 and the constrained
level of core §12.

**ZenSight, the largest.** Its 22 registries (813 subjects, 200
procedures) are mapped on paper, with twelve representative contracts and
24 gaps ([README](../../examples/zk2/zensight/README.md)). The migration
debt is in the mapping's "Migration debt": identities inside payloads
(artifact prefixes, alert references, historian series and the persisted
timeline uids), 23 production sites that build keys by hand, and a web
client that spells its keys by hand. Most of its planes need profiles
that do not exist yet: alarms, desired state, redundancy, views, media,
blobs and telemetry. Its presence also needs the tokenless set to stay
within the budget (r4 §4.9).

**ZenSight's path is undecided.** There are three options, and this note
recommends none:
1. **Stay on v1.** The v1 line keeps releasing 0.14.x, every feature
   ZenSight uses stays there, and the RFCs take errata only.
2. **Migrate incrementally.** v1 and zk2 keys share a bus without seeing
   each other (core §1.7), so services can move one at a time. A consumer
   that spans both reads both, and each host's origins map to its system
   through hostid.v1's table (Appendix B), which ZenSight's catalog could
   publish as aliases (r4 §0.3, D26).
3. **Rewrite on profiles.** Build ZenSight's planes on #613's profiles as
   they land, as the mapping sketches them.
