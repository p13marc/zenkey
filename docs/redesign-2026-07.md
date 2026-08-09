# zenkey redesign 2026-07 — analysis, generalization, and roadmap

Status: **largely implemented** (historical design record). Written against RFC v1.4; the RFC set is at v1.10. P0–P5 shipped through the 0.3/0.4 releases and the zengui bootstrap (PR #32); remaining rows are tracked by epic #33 ("Explorer Suite 1.0"). Kept for the rationale; where it and shipped code disagree, the code and the RFCs win.
Scope: `zenkey`, `zenkey-build`, `zenctl`, the RFC set, and the groundwork for a future
Iced GUI bus explorer. Backward compatibility is explicitly on the table — we are the
only consumers today (zensight, tcgui).

This report was produced from a deep pass over the zenkey workspace, both consumer
repos, and external research on the Zenoh ecosystem and comparable systems
(ROS 2, MQTT/Sparkplug, Kafka/Confluent, DDS XTypes, NATS, AsyncAPI). Sources are
cited in the appendix.

---

## 1. Executive summary

zenkey today is a sound, unusually well-specified convention layer: a normative RFC
set with an amendment discipline, a typed key grammar with executable design-property
guards (D1–D6), consumer-owned registries compiled to typed builders, and an honest
app-neutral CLI. The extraction from the zensight monorepo is essentially complete
(issue #1 is done in substance).

But looking at how the two real consumers actually use it, the typed surface stops
too early, in four places:

1. **Only the concrete build+parse direction is typed.** Selectors/wildcards — the
   *subscription* half of a bus — are raw strings: zensight hand-formats ~38 wildcard
   helpers in `zensight-common/src/keyexpr.rs`, tcgui hardcodes `SEL_*` consts.
2. **Builders return `String`.** Both consumers re-parse into `OwnedKeyExpr` with
   panicking wrappers; typed origins (RFC 08 §1.1, G5) are specified but implemented
   only in tcgui's app code, not in zenkey.
3. **Codegen emits enums, not ergonomics.** tcgui wrote a ~400-line `topics` wrapper
   module plus a parallel `StateFamily` enum re-encoding what the registry already
   knows; manual `slug_key_chunk()` at every call site is a latent-bug source.
4. **Payload *types* are named but not described.** The registry binds a type name
   per subject, but there is no schema anywhere a generic tool can fetch — so zenctl
   (and any future GUI) can only render anonymous CBOR/JSON structure, never named
   fields, and protobuf would be fully opaque.

The proposal is three pillars, each closing one of these gaps and each feeding the
future GUI:

- **Pillar A — core API v2**: validated `Key`/`Selector`/`Chunk` newtypes, typed
  origins hoisted into zenkey, a first-class selector + pattern-matching API, and
  codegen v2 that makes the consumer wrapper layers deletable.
- **Pillar B — payload self-description (RFC 08 §7)**: a per-producer `describe`
  RPC serving schema documents (JSON Schema for serde types, FileDescriptorSet for
  protobuf), plus a decode/encode registry *in zenkey* so every tool — zenctl, GUI,
  scripts — can turn any registered payload into named fields, and build request
  bodies from schemas.
- **Pillar C — zenctl v2 + `zenkey-fleet`**: nats-style contexts, shell completions,
  `--format json/ndjson`, `hz`/`bw`/`bench`, Zenoh admin-space integration, and the
  extraction of a `zenkey-fleet` library crate that becomes the shared engine of the
  CLI and the Iced GUI.

End state: a registry-aware bus explorer that is categorically ahead of
zenoh-explorer and zenoh-hammer — both of which treat keys as raw strings and
payloads as opaque bytes.

---

## 2. Current-state assessment

### 2.1 What is working well (keep)

- **The RFC discipline.** Normative text, amendment changelog recording what
  deliberately did *not* change, decision record (RFC 12) with revisit triggers,
  and D1–D6 pinned as executable guard tests (`zenkey/tests/guard.rs`). Nothing in
  this proposal touches the grammar; D1–D6 survive untouched.
- **Consumer-owned registries + build-time lints.** `zenkey-build` failing the
  *consumer's* build on RFC 08 §5 violations is the right enforcement point, and the
  append-only `deprecated.lock` ledger works.
- **App neutrality.** No base constant, no bundled registry, profile/salt as the
  only app seam. Issue #1's demands are met in substance.
- **zenctl's honesty posture.** Single fan-in chokepoint (`bus::fleet_get`, RFC 05
  §2.1), "silence is never a verdict", scouting opt-in, forward-compatible slice
  parsing pinned by the foreign-slice test. All preserved (and moved, not rewritten,
  in Pillar C).

### 2.2 The gaps, with evidence

| # | Gap | Evidence |
|---|-----|----------|
| G-a | Typed origins specified, unimplemented | RFC 08 §1.1 + v1.4 G5 (MUST for writes); generated builders take plain `&Origin`. The reference implementation exists in **tcgui**, `tcgui-shared/src/identity.rs` (`LocalOrigin`/`RemoteOrigin`/`ConcreteOrigin`) — app code doing convention work. |
| G-b | Untyped selectors | `zensight-common/src/keyexpr.rs`: ~38 `format!`-built wildcard fns (`all_telemetry_wildcard`, `fleet_rpc_key`, focus-mode selectors…); tcgui `SEL_STATE`/`SEL_TELEMETRY`/`SEL_ALIVE` string consts. |
| G-c | Builders return `String` | tcgui `ke()` helper (`tcgui-shared/src/lib.rs:72`): `OwnedKeyExpr::try_from(key).expect("registry-built keys are valid keyexprs")`. |
| G-d | Consumer wrapper layers | tcgui `topics` module (~400 lines, `tcgui-shared/src/lib.rs:64–450`) + hand-written `StateFamily` mirror of `tc::Subject`; zensight `command.rs` wrappers over `V1Context`. |
| G-e | Manual slugging | Every tcgui builder call slugs by hand (`slug_key_chunk(ns)`); forgetting is a latent bug (guarded only by an app test). |
| G-f | Pattern matching duplicated | Generated `parse` if-chains (`zenkey-build/src/emit.rs`) vs interpretive `zenctl/src/offline.rs::match_subject` — same precedence rules, two implementations, no parity test. |
| G-g | `[[media]]` codegen missing | Linted, generates nothing; the RFC v1.3 changelog *promised* generated media builders. |
| G-h | `fanout`/`idempotent` unenforced | Fields exist in registry TOMLs (tc.toml, catalog.toml); `zenkey-build` does not even parse them (zero hits in lib.rs/emit.rs). RFC prefers builder-level refusal of `*`-origin writes. |
| G-i | No payload schemas | Registry binds type *names* only. zensight hand-wrote the decode table (`zensight-common/src/payload.rs`, `decode_payload` + `PAYLOAD_TYPES` + `types_are_total` test); **tcgui has none** — its payloads are undecodable by any generic tool. This is also the blocker in issue #2. |
| G-j | Consumer-side string re-parsing survives | Positional `split('/')` parsing in zensight views for proxy producers (snmp/modbus/gnmi/syslog) — partially inherent to rest-var families, but `refine_key` should at least be generated, not hand-copied. |
| G-k | zenctl UX debt | `--base` required on every call, no config/contexts, no completions, no `--format json`, stringly `service call` params, no `hz`/`bw`/bench, no admin-space access, `--registry` and bus sources mutually exclusive. |
| G-l | CI/release gaps | No release automation, no MSRV check despite `rust-version = 1.88`, single-platform CI. |

### 2.3 Existing GitHub issues

- **#1 "De-ZenSight the engine"** — done in substance: `zenkey-build` exists, the
  crate ships no registry, no `DEFAULT_BASE`, zensight owns its TOMLs. The ZenSight
  registry snapshot in `fixture-tests/` is deliberate (regression corpus, unpublished).
  → close, pointing here.
- **#2 "zenctl: sever the zensight-common dependency"** — the discovery half is done
  (in-repo zenctl is app-neutral, bus/`--registry` sourced). The payload-decode half
  is exactly Pillar B; the "move generic keyexpr wrappers into zenkey" half is
  Pillar A §4.4. → superseded by the new issues.

---

## 3. Landscape research (what to borrow)

### 3.1 Zenoh 1.8/1.9 native introspection — stop duplicating it

- The **admin space `@/**`** exposes runtime state (config, metrics, plugins,
  storages) over ordinary GET — fleet/node discovery is a wildcard GET away.
- Since 1.8 ("Kiyohime"), **liveliness tokens are enumerable via the admin space**
  (`@/<zid>/…/token/**`) — a native complement to our liveliness roster.
- **MatchingListener / matching status**: a publisher (and querier) can know whether
  anyone is listening. Per-key "has consumers" badges become possible.
- **Storage manager admin keys** let a tool list storages and their key expressions —
  enabling a *storage coverage* check against declared `state` subjects (broken
  late-joiner seeds are today silent).
- 1.9 ("Longwang") **Regions** replace the fixed router/peer/client hierarchy —
  tooling must not hardcode a 3-tier topology.

Implication: the convention's introspection (`@catalog`, RFC 08 §6) should treat the
admin space as a first-class complementary source, not reinvent it. Caveat: exact
admin key layouts vary between docs versions — verify against a live `zenohd`
before hardcoding paths in zenctl.

### 3.2 Existing Zenoh GUIs — the bar to clear

- **zenoh-explorer** (dad-io): live message monitor, wildcard subscriptions, publish
  panel with encodings, query panel, hierarchical topic tree with payload counts.
- **zenoh-hammer** (sanri): get/put/sub panels, hex viewer, image rendering, session
  config persistence.

Both are **untyped**: raw keys, opaque payloads, no schema/registry awareness, no
liveliness/QoS semantics, no rate metrics. A registry-aware explorer with typed key
trees, schema-decoded payloads, QoS/ttl/rate badges, liveliness and matching status
is a different category of tool.

### 3.3 Prior art in comparable ecosystems

| System | Lesson taken |
|--------|--------------|
| **ROS 2** `ros2 topic list/echo/hz/bw/info` | The expected verb set for a bus CLI; runtime type lookup so `echo` decodes without naming the type; a daemon caches discovery (we don't need the daemon — Zenoh discovery is fast — but we take the *cache-for-completions* idea). |
| **MQTT Sparkplug B** | Birth certificates: producers self-describe their full metric/schema inventory on a well-known address at startup. Validates our per-producer `introspect`, and motivates the `describe` schema endpoint. |
| **Kafka / Confluent Schema Registry** | Schema decoupled from wire bytes and *fetchable*; per-subject compatibility policies (BACKWARD/FORWARD/FULL) enforced at registration → the model for registry compatibility levels. We deliberately do **not** take the schema-id-per-sample wire prefix. |
| **DDS XTypes** | Ship a type hash in discovery, fetch the full type on demand → our `SchemaSet` entries carry a sha256 hash for caching and drift detection. |
| **NATS CLI** | Named contexts (`nats context`), clean verbs, built-in `bench`, schema subcommand → the zenctl v2 UX model. |
| **kcat** | `-L` metadata mode, composable `-f` output format strings → `zenctl topic echo --fmt`. |
| **CloudEvents / AsyncAPI 3.x** | Classification metadata belongs in the addressing layer (validates our grammar); AsyncAPI as a machine-readable bus description → `zenctl registry export --format asyncapi` gives browsable docs, diagrams and diffs from the registry for free. |
| **prost-reflect / schemars / CDDL** | The Rust machinery for generic decode: `DynamicMessage::decode` against a served FileDescriptorSet; JSON Schema derived from serde types honoring `#[serde]` attrs; CDDL held in reserve as a future schema kind. |

---

## 4. Pillar A — core API v2 (zenkey + zenkey-build)

No grammar change. D1–D6 untouched. This pillar codifies MUSTs the RFCs already
state (G2, G5, the v1.3 media promise) and types the surfaces consumers proved they
need by writing wrappers.

### 4.1 Validated key types: `Key`, `Selector`, `Chunk`

New `zenkey::key` module:

```rust
/// Validated, canonical, base-relative concrete v1 key. No wildcards.
pub struct Key(String);        // Deref<str>, Display, AsRef<str>, Into<String>
/// Validated base-relative key expression; MAY contain * / **.
pub struct Selector(String);   // Key: Into<Selector>

#[cfg(feature = "keyexpr")]    // zenoh-keyexpr only — far lighter than full zenoh
impl From<Key> for zenoh::key_expr::OwnedKeyExpr { /* infallible by construction */ }

/// One validated plain chunk (subsumes today's free validator fns).
pub struct Chunk(String);
impl Chunk {
    pub fn slug(v: impl AsRef<str>) -> Chunk;              // always succeeds
    pub fn parse(v: &str) -> Result<Chunk, KeyError>;      // must already be legal
}
```

Every builder in `grammar`, `V1Context`, and generated code returns `Key` (or
`Selector`). The infallible `From<Key> for OwnedKeyExpr` is pinned by a property
test: every grammar-legal key is a canonical zenoh keyexpr (we already have
`zenoh-keyexpr` as a dev-dep; it becomes an optional real dep).

Rejected alternative: returning `OwnedKeyExpr` directly — zenkey-build consumes
zenkey with `default-features = false`, and `OwnedKeyExpr` cannot encode
"base-relative, concrete, v1".

Kills: tcgui's `ke()` panicking wrapper; zensight's untyped `String` flow into zenoh.

### 4.2 Typed origins (hoisted from tcgui)

`zenkey::origin` grows the design tcgui already proved (`tcgui-shared/src/identity.rs`):

```rust
pub struct LocalOrigin(HostId);      // minted only via AppProfile::local_origin()
pub struct RemoteOrigin(HostId);     // only from wire data; never wildcard/@service
pub struct ServiceOrigin(String);    // validated @-verbatim; ::catalog()

/// Sealed: LocalOrigin | RemoteOrigin | ServiceOrigin. Never a fleet value.
pub trait ConcreteOrigin { fn chunk(&self) -> &str; }

/// The deliberate `*` — accepted only by selector builders and fanout-allowed RPC.
pub struct Fleet;
```

- Publish/serve builders take `&LocalOrigin` (or `&ServiceOrigin`).
- Call/address builders take `&impl ConcreteOrigin`.
- **Write builders have no fleet form at all** — G5's MUST becomes a type-level fact.
- The existing `enum Origin` stays as the parse-side representation inside
  `StructuralKey` (with `StructuralKey::remote_origin() -> Option<RemoteOrigin>` as
  the bridge). Optional `serde` feature for `HostId`/`RemoteOrigin` (tcgui carries
  origins in health documents).

### 4.3 First-class selectors + shared pattern matching

`zenkey::selector` — typed scope + the canned framework selectors that absorb
zensight's `keyexpr.rs` and tcgui's `SEL_*`:

```rust
pub enum Scope { Fleet, Origin(Chunk) }   // Scope::origin(&impl ConcreteOrigin)

pub fn all_state(scope: Scope) -> Selector;        // v1/*/state/**  | v1/h-x/state/**
pub fn all_telemetry(scope: Scope) -> Selector;
pub fn all_events(scope: Scope) -> Selector;
pub fn all_liveliness(scope: Scope) -> Selector;   // v1/*/state/*/alive
pub fn producer_state(scope: Scope, producer: &str, prefix: &[&str]) -> Selector;
pub fn fleet_rpc(producer: &str, procedure: &[&str]) -> Selector;
pub fn rpc_at(o: &impl ConcreteOrigin, producer: &str, procedure: &[&str]) -> Key;
```

`zenkey::pattern` — the single match+bind primitive ending the G-f duplication:

```rust
pub struct SubjectPattern { /* Literal | Var | Rest chunks */ }
impl SubjectPattern {
    pub fn parse(pattern: &str) -> Result<Self, PatternError>;
    pub fn matches(&self, tail: &[&str]) -> Option<Vec<(&str, String)>>;  // named binds
    pub fn precedence(&self) -> impl Ord;    // literal < var < rest, per position
    pub fn selector_tail(&self) -> String;   // {var}→*, {var...}→**
}
pub fn best_match(...) -> Option<...>;        // most-literal-first across a set
```

zenctl's `match_subject` delegates here; zenkey-build orders its compiled if-chains
by `precedence()`; and `fixture-tests` gains a **parity test** (generated
`Subject::parse` ≡ interpretive `best_match` over the whole ZenSight snapshot
corpus) so the two can never drift again.

### 4.4 Wrapper migration into zenkey (the issue #2 half)

`fleet_rpc_key` / `all_liveliness_wildcard` etc. move under `selector` returning
typed `Selector`. `is_telemetry_key` generalizes to
`grammar::is_class_key(key, Class)`. `refine_key`/`refine_full_key` need a registry,
so they become **generated** (§4.5), deleting zensight's hand copies. App-specific
catalog helpers fall out of generated per-family builders (catalog is a `[service]`
registry).

### 4.5 Codegen v2 (zenkey-build emit)

The test of success: **tcgui deletes its `topics` module; zensight deletes the bulk
of `keyexpr.rs` and `command.rs`.** Sketch, for tcgui's `tc` registry:

```rust
pub mod tc {
    pub enum Subject {
        Health, Sensor,
        Interface { ns: Chunk, iface: Chunk },
        Config    { ns: Chunk, iface: Chunk },
        Scenario  { id: Chunk },
        Bandwidth { ns: Chunk, iface: Chunk },
        Applied   { ulid: Chunk },
        /* … */
    }
    impl Subject {
        // slug-at-the-boundary constructors — manual slug_key_chunk() dies here:
        pub fn interface(ns: impl AsRef<str>, iface: impl AsRef<str>) -> Self;
        pub fn family(&self) -> Family;
        /* class(), qos(), ttl_s(), payload_type(), … unchanged */
    }

    /// Fieldless mirror — deletes tcgui's hand-written StateFamily.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Family { Health, Sensor, Interface, Config, Scenario, Bandwidth, Applied, /*…*/ }
    impl Family {
        pub const ALL: &[Family];
        pub fn class(self) -> Class;
        pub fn selector(self, scope: Scope) -> Selector;  // {var}→*, {var...}→**
    }

    /// Publishing is Local by type (G5); key() is infallible (Chunk fields).
    pub fn key(o: &LocalOrigin, s: &Subject) -> Key;
    pub fn key_at(o: &impl ConcreteOrigin, s: &Subject) -> Key;   // drill-down GETs
    pub fn interface_key(o: &LocalOrigin, ns: impl AsRef<str>, iface: impl AsRef<str>) -> Key;

    pub enum ProcedureId { ConfigNsIfaceSet, Diagnostics, Introspect, /*…*/ }
    /// The fanout = "allowed" subset — a fleet WRITE is unspellable (G2):
    pub enum FleetProcedureId { Diagnostics, Introspect }
    pub fn rpc_fleet_selector(p: FleetProcedureId) -> Selector;
    pub fn config_set_key(o: &impl ConcreteOrigin, ns: ..., iface: ...) -> Key;
    pub fn config_set_serve(o: &LocalOrigin) -> Selector;
}

// per build, top level:
pub struct Refined { pub key: StructuralKey, pub producer: String, pub subject: AnySubject }
pub fn refine_key(key: &str) -> Option<Refined>;
pub fn refine_full_key(base: &str, key: &str) -> Option<Refined>;
/// Decode-dispatch hook (Pillar B consumes this):
#[macro_export] macro_rules! zenkey_for_each_payload_type { ... }
```

Also in codegen v2:

- **`[[media]]` builders** (closes the v1.3 promise): a `Media` enum with
  Chunk-typed vars, `media_key(o: &LocalOrigin, m)` / `media_key_at(o: &RemoteOrigin, m)`;
  deliberately **no** wildcard selector for media (the v1.3 tier-wildcard revocation).
- **`fanout`/`idempotent` parsed and enforced** (G-h): `kind="write"` defaults
  `fanout="forbidden"`; `fanout="allowed"` on a write must be explicit; fleet
  builders exist only for the allowed subset.
- **G1 typing**: a `[service]` subject whose first chunk is `{host}` gets a
  `HostId`-typed constructor arg, plus a lint that desired-state families lead with
  `{host}` (implements ratified 07 §3).
- Note: `Chunk::slug` preserves the G4 case-injectivity exemption (escaping, never
  lossy lowercasing).

### 4.6 Registry compatibility levels (new normative text — H3)

Generalize the deprecation ledger into a Confluent-inspired evolution gate: a
committed, generated `registry.lock` snapshot (one line per subject:
`producer\tpattern\tclass\ttype`) plus

```rust
Config::new().compatibility(Compat::Backward)   // default
```

- `Backward` (default): a previously-locked pattern may not disappear without a
  `[[deprecated]]` entry, nor change class/type in place.
- `Full`: additionally forbids narrowing a `{var...}` family.
- `None`: opt out.

This is the one Pillar-A item requiring genuinely new RFC text; held for your
review before an issue is filed.

---

## 5. Pillar B — payload self-description (RFC 08 §7)

The crux for generic tooling. Design principle: **schemas are fetched, not shipped
per-sample**; zenkey-core stays format-agnostic; unknown kinds degrade gracefully.

### 5.1 The `describe` procedure

Every producer serves `@rpc/<producer>/describe` (read, idempotent) returning a
`SchemaSet` JSON document, fetched lazily on first decode miss and cached by hash.
`introspect` is unchanged (small TOML fleet inventory).

```json
{
  "schema_version": 1,
  "app": "zensight",
  "types": {
    "TelemetryPoint": {
      "kind": "json-schema",
      "hash": "sha256:ab12…",
      "schema": { "$schema": "https://json-schema.org/draft/2020-12/schema", "...": "..." }
    },
    "FlowRecord": {
      "kind": "protobuf",
      "hash": "sha256:cd34…",
      "message": "zensight.flow.FlowRecord",
      "descriptor_b64": "<base64 FileDescriptorSet>"
    }
  }
}
```

- **Two kinds registered now.** `json-schema` (schemars, honors `#[serde]` attrs)
  covers every existing payload — CBOR needs no separate kind, since CBOR and JSON
  share the serde data model; *encoding* is a separate axis (§5.4). `protobuf`
  carries exactly what prost-reflect's `DescriptorPool` + `DynamicMessage::decode`
  needs. `kind` is an open string: consumers MUST skip unknown kinds without
  dropping the set (room for CDDL/Avro later).
- **Totality**: `describe` MUST cover every type name the producer's `introspect`
  slice references.
- **Hash** (`sha256:` over canonical schema bytes): client caching + a new doctor
  finding (same type name, different hash across producers).
- **No per-sample schema ids** — reaffirms RFC 08 §3's out-of-band posture and
  RFC 03 §6.4's rejection of per-sample type hashing. Evolution stays additive-only
  under one name; incompatible = new suffixed sibling name.

Rejected alternatives (one line each): schemas embedded in introspect TOML (bloats
fleet inventory); Sparkplug-style retained birth key (needs storage guarantees;
RPC is our established idiom); central schema-registry service (new singleton in a
deliberately peer-served design); Kafka-style payload envelope (wire-breaking,
violates RFC 07 §1).

### 5.2 zenkey/zenkey-build support

- **zenkey `schema` module** (adds serde/serde_json): `SchemaKind` (open),
  `TypeSchema`, `SchemaSet` with tolerant `parse` (same posture as `parse_slice`),
  `verify_covers(&[&str])` — the generic replacement for zensight's hand-written
  `types_are_total` test. Feature `schemars`:
  `TypeSchema::json_schema_of::<T: JsonSchema>(name)`. Protobuf serving needs no
  prost dep (apps wrap the FDS bytes from their own `prost-build`).
- **Decode lives in zenkey, not zenctl** (the GUI must reuse it). Feature `decode`
  (serde_json + ciborium): `DecoderRegistry` + json-schema decoder. Feature
  `decode-protobuf` (prost-reflect).

  ```rust
  pub trait PayloadDecoder: Send + Sync {
      fn kind(&self) -> &str;
      fn decode(&self, schema: &TypeSchema, enc: WireEncoding, bytes: &[u8]) -> Result<DecodedPayload>;
      fn encode(&self, schema: &TypeSchema, value: &serde_json::Value) -> Result<(Vec<u8>, WireEncoding)>;
  }
  ```

  `encode` is the serialize half: schema-driven request forms in the GUI and
  validated `zenctl service call --body`.
- **zenkey-build**: materialize RFC 08 §5's "shared type table" (which never
  existed as an artifact) as **`registry/types.toml`**
  (`[types.TelemetryPoint] kind = "json-schema" rust = "zensight_common::telemetry::TelemetryPoint"`);
  new lint: every `type`/`request`/`reply`/`attachment` name must resolve; emit
  `pub const TYPE_NAMES: &[&str]` so `SchemaSet::verify_covers(TYPE_NAMES)` binds
  schema table to registry mechanically.
- **Consumer cost** (this is the whole point): `#[derive(JsonSchema)]` on payload
  types + one static:

  ```rust
  static SCHEMAS: LazyLock<SchemaSet> = LazyLock::new(|| SchemaSet::builder("tcgui")
      .json::<NetworkInterface>("NetworkInterface")
      .json::<Ack>("Ack")
      .build_verified(registry::TYPE_NAMES));
  ```

  plus a 5-line `describe` queryable. tcgui becomes fully decodable by generic
  tools with ~a page of diff.

### 5.3 The generic decode pipeline (zenctl + GUI)

```
sample → parse_full → slice match → type_name (+unit, +encoding)
       → SchemaStore.get(producer, type_name)     # cache by hash; miss → GET describe
       → DecoderRegistry.decode(schema, encoding, bytes)
       → DecodedPayload { value, field docs/units, unknown-field flags }
       → render (zenctl line / GUI tree)
```

Fallbacks stay honest: no schema → today's sniff render tagged `<TypeName?>`;
unknown kind → `<undecoded TypeName: N bytes>` (issue #2's wording).

### 5.4 Encoding declaration

Registry `[[subject]]`/`[[procedure]]` gain optional `encoding` (RECOMMENDED —
media already has one; this is symmetry). The RFC also RECOMMENDS producers set the
Zenoh sample `Encoding`. Resolution: sample > registry > sniff; the sniff never
goes away (mixed-fleet tolerance).

### 5.5 RFC v1.5 amendments for this pillar

New **RFC 08 §7** (describe contract, kinds, hash, MUST/SHOULD levels — MUST serve
describe if any referenced type is not self-describing, e.g. protobuf); 08 §5
amendment (types.toml materializes the type table); 08 §2 amendment (`encoding`
field); RFC 04 note (set sample Encoding); 00-index changelog + the Scope sentence
amended: payload *contents* stay out of scope, the *transport of schema documents*
is in scope.

---

## 6. Pillar C — zenctl v2 + `zenkey-fleet`

### 6.1 UX decisions (opinionated)

| Decision | Choice |
|---|---|
| Profiles | Named **contexts** in `~/.config/zenctl/config.toml` (nats-style); `--base` becomes optional: flag > env > context |
| Scripting | Global `--format auto\|table\|json\|ndjson` over **typed report structs** (serde) — one refactor unlocking scripting, tests, GUI parity |
| Daemon | **No** (Zenoh discovery is fast; ros2's daemon solves a DDS problem we don't have). On-disk slice cache `~/.cache/zenctl/<context>/slices/` gives instant dynamic completions |
| Completions | `clap_complete` static + dynamic (subject/producer/type names from the cache) |
| Watch | `--watch` flags on list commands; the TUI is the GUI's job |
| Registry sources | `--registry` and bus stop being exclusive: union, served wins per producer; disagreement is reported by `doctor`/`registry diff` |

### 6.2 Command surface v2 (T = table stakes, D = differentiator, L = later)

```
zenctl context create|list|show|select|rm|edit          [T]
zenctl completions <shell>                               [T]

zenctl topic list [--producer] [--class] [--type] [--watch]          [T]
zenctl topic info <key>                                              [T]
zenctl topic echo [SEL] [--origin|--class|--producer|--subject GLOB]
                  [--fmt FMT] [--hex|--raw|--no-decode] [--count] [--rate]  [T/D]
zenctl topic hz|bw <SEL> [--window 10s] [--per-key]                  [T]
zenctl topic pub <key> [BODY|@file|-] [--qos PROFILE] [--repeat] [--interval]
                       [--encoding MIME] [--no-validate] [--raw]     [D]
       # the body is ENCODED for the wire against the served schema (#97)

zenctl node list [--verbose] [--watch]    # liveliness + introspect + admin join  [T]
zenctl node info <origin>                                            [D]

zenctl service list|info                                             [T]
zenctl service call <origin|*|@svc> <producer|-> <proc>
                    [--param k=v] [--body JSON|@file|-] [--no-validate] [--raw]  [T]
       # exit 1 = an error reply; exit 2 = zero replies (silence stays non-verdict)

zenctl interface list | show <Type> [--schema] [--full]              [T]
zenctl schema <producer> [--type T] [--full]   # dump served SchemaSet  [T]

zenctl registry export --format asyncapi|jsonschema|toml             [D]
zenctl registry diff | lint <dir>                                    [D/L]

zenctl admin get [SEL=@/**] | admin routers                          [T/D]
zenctl storage list      # declared state subjects vs storage coverage  [D]

zenctl doctor [--sample N] [--deep]   # + freshness-vs-ttl, storage coverage,
                                      #   admin reachability, schema conformance  [T→D]
zenctl bench rpc <origin|*> <producer> [--count]                     [D]
zenctl bench pub|sub …                                               [L]
zenctl record <SEL> -o f.zrec | replay f.zrec [--speed] [--dry-run]  [L]
```

echo v2 notes: `--origin/--class/--producer` compose **server-side** into the
selector (never client-filter what the grammar can express positionally);
kcat-style `--fmt` (`%k %o %c %p %s %t %v %{a.b.c} %T %e %l %n`); typed decode via
Pillar B with sniff fallback; ndjson mode emits one object per sample — value
filtering is `| jq`, deliberately not reimplemented. Matching-status doctor checks
are **deferred honestly** (publisher-side probing can't be done without publishing)
and logged in RFC 12.

### 6.3 The `zenkey-fleet` crate (GUI groundwork)

New workspace member (Apache-2.0). The RFC 05 §2.1 chokepoint (`fleet_get`) moves
**verbatim** from `zenctl/src/bus.rs` and stays singular — now for CLI *and* GUI.

```
zenkey-fleet/src/
  session.rs   # un-namespaced open(), FleetConfig
  query.rs     # fleet_get discipline, FleetAnswer
  roster.rs    # roster() + NodeInfo admin-space enrichment
  registry.rs  # SliceSource {Bus, Dirs, Union}, SliceSet, match via zenkey::pattern, disk cache
  decode.rs    # PayloadDecoder seam (Pillar B plugs in), SampleView (lazy decode)
  stats.rs     # windowed counters: msg/byte rate, EWMA (hz/bw/--rate/GUI)
  tree.rs      # KeyTree + immutable ArcSwap snapshots, GroupBy, per-node NodeStats
  sub.rs       # Monitor: subscription multiplexing + liveliness → FleetEvent
  admin.rs     # @/** queries, RouterInfo, StorageInfo
  record.rs    # [later] .zrec read/write/replay
```

Async model: tokio throughout. `Monitor::events()` is a bounded broadcast
(overflow surfaces as an explicit `Dropped(n)` event — dropped samples are never
invisible); the tree view redraws on `StatsTick` (default 250 ms) by pulling an
`ArcSwap<KeyTreeSnapshot>` — so a hot bus cannot melt a render loop. Iced consumes
via `Subscription::run` over a `BroadcastStream`. The CLI's `echo`/`hz`/`bw`/
`--watch` use the same `Monitor`, which validates the API before the GUI exists.

zenctl afterwards is a thin frontend: `main.rs` (clap tree), `context.rs`,
`output.rs` (table|json|ndjson renderers), `report.rs` (serde structs),
`cmd/*.rs`. The foreign-slice forward-compat test moves with `SliceSet`.

### 6.4 GUI MVP checklist (validates the API — nothing needs a type not listed above)

1. Tree view — `Monitor::tree()` + `GroupBy` + `NodeStats` (count, rate, last
   sample, registered-vs-wild flag).
2. Live echo pane — filtered `events()`, ring buffer, `Dropped` indicator.
3. Publish/call pane — `Fleet::publish` (QoS picker = the closed enum),
   `Fleet::call` with attributed answers, request forms scaffolded from registry +
   schemas (Pillar B `encode`).
4. Node dashboard — enriched `NodeInfo`, `NodeUp/NodeDown`, per-node freshness.
5. Schema-aware payload inspector — raw hex + lazy named-field JSON side by side.

---

## 7. RFC v1.5 amendment slate (summary)

| ID | Chapter | Content | Nature |
|----|---------|---------|--------|
| H1 | 08 §1.1/§5 | Generated-surface v2: `Key` returns, Chunk-typed slug-at-boundary vars, `Family` mirror + selectors, G2 as "no fleet variant generated", G5 as typed origins in zenkey | codifies existing MUSTs |
| H2 | 08 §2 | `[[media]]` builder codegen delivered; `variant` legal on media | closes v1.3 promise |
| H3 | 08 §3 (new subsection) | Compatibility levels: `registry.lock`, `backward` default | **new normative** — awaiting review |
| H4 | 08 §5 | Lint: desired-state service subjects lead with `{host}`, typed | additive (implements G1) |
| H5 | 08 §7 (new) | Payload self-description: `describe`, SchemaSet, kinds, hash, no per-sample ids | **new normative** |
| H6 | 08 §2/§5, 04 | `encoding` field + sample-Encoding recommendation; types.toml materializes the type table | additive |
| H7 | 09, 12 | Cookbook: contexts, record/replay etiquette; open question: matching-status introspection | informative |

D1–D6 and the version-chunk/base-as-namespace decisions are untouched.

---

## 8. Migration impact (zensight, tcgui)

The deletions are the payoff:

**tcgui** — delete `topics` (~400 lines) and `StateFamily`; delete
`identity.rs`'s origin types in favor of zenkey's (keep `PROFILE`); delete all
key-boundary `slug_key_chunk` calls; replace `SEL_*` with `selector::*`. Add:
`#[derive(JsonSchema)]` + SchemaSet + `describe` (becomes decodable by generic
tools for the first time).

**zensight** — `keyexpr.rs` shrinks to near-nothing (framework selectors →
`zenkey::selector`; catalog helpers → generated; refine fns → generated);
`command.rs` rewritten onto generated per-procedure builders (this flushes out any
procedure missing from registry TOMLs — RFC "the registry must not lie");
`payload.rs` decode table → one `zenkey_for_each_payload_type!` invocation, later
replaced by SchemaSet; `String`→`Key` ripple is mostly absorbed by `Deref<str>` +
`Into<OwnedKeyExpr>`.

**zenctl** — `match_subject` → `zenkey::pattern`; then the Pillar-C restructuring.

Versions: breaking → `zenkey` 0.3.0, `zenkey-build` 0.3.0, `zenctl` 0.2.0.
Publish order becomes zenkey → zenkey-build → zenkey-fleet → zenctl. Cross-repo
development via temporary `[patch.crates-io]` as usual.

---

## 9. Roadmap (merged, effort-sized)

| Phase | Content | Size |
|-------|---------|------|
| P0 | RFC v1.5 text (H1–H7) + changelog — amendments first, "RFC text is normative" | ~1 d |
| P1 | zenkey core: Key/Selector/Chunk, typed origins (+serde feature), Scope/selector, pattern, V1Context return types, guard-test update | 2–3 d |
| P2 | zenkey-build emit v2: Chunk fields + constructors, Family, per-variant + named-arg builders, FleetProcedureId, media, refine_key, payload macro, {host} lint; fixture-tests rewrite + parity test | 3–4 d |
| P3 | zenkey `schema` module + schemars feature; types.toml lint + TYPE_NAMES; `encoding` field | 2–3 d |
| P4 | zenctl quick wins: contexts, `--format` + report structs, completions, echo selector composition, exit codes | ~2 d |
| P5 | zenkey-fleet extraction; decode/decode-protobuf features + SchemaStore; echo v2 --fmt; hz/bw; admin get/routers; node enrichment; slice cache; doctor v2; bench rpc; storage list | 4–5 d |
| P6 | Consumer migrations: tcgui (1–2 d), zensight (2–3 d); both serve `describe` | 3–5 d |
| P7 | Post-review items: compatibility levels (H3), registry export (AsyncAPI), bench pub/sub, topic pub, record/replay | as approved |
| P8 | The Iced GUI (separate effort, consuming zenkey-fleet per the MVP checklist) | — |

P1→P2 are strictly ordered; P3 can run parallel to P2; P4 parallel to anything;
P6 gates on P2/P3.

### Open decisions awaiting your review

1. **Compatibility levels (H3)** — new normative machinery; worth it now, or after
   the schema story lands?
2. **`Chunk` fields in generated enums** — maximal safety, but pattern-matching
   ergonomics change slightly (`ns.as_str()`); the alternative (String fields +
   slugging constructors only) keeps the latent bypass.
3. **record/replay and bench pub/sub** — scope and posture (writing into a fleet).
4. **GUI timing** — after P5 (zenkey-fleet + decode) it can start; before P6 it
   would only decode zensight (the only app with schemas until migration).
5. **crates.io naming** — `zenkey-fleet` vs alternatives for the shared engine.

---

## 10. Appendix

### 10.1 Issue map

Existing: **#1 closed** (done in substance); **#2** kept open with a status
comment — decode half superseded by #11, wrapper half by #7, publishing by #16.

Filed with this report:

| Issue | Title | Report § |
|-------|-------|----------|
| #5 | validated Key/Selector/Chunk types | §4.1 |
| #6 | typed origins hoisted from tcgui (RFC 08 §1.1, G5) | §4.2 |
| #7 | selector API + shared subject-pattern match/bind | §4.3–4.4 |
| #8 | codegen v2 (slug-at-boundary, Family, per-variant builders) | §4.5 |
| #9 | fanout/idempotent enforcement — fleet writes unspellable (G2) | §4.5 |
| #10 | `[[media]]` builder codegen (v1.3 promise) | §4.5 |
| #11 | payload self-description (describe/SchemaSet/types.toml, RFC 08 §7) | §5 |
| #12 | zenctl contexts, completions, --format | §6.1–6.2 |
| #13 | topic hz/bw + echo v2 | §6.2 |
| #14 | admin-space integration + doctor v2 + storage coverage | §6.2, §3.1 |
| #15 | zenkey-fleet extraction (GUI engine) | §6.3–6.4 |
| #16 | CI/release hygiene (release automation, MSRV) | §2.2 |

Held for review (not filed): registry compatibility levels (H3, §4.6),
record/replay + bench pub/sub (§6.2 [L] items), GUI timing (§9).

### 10.2 External references

- Zenoh releases and blogs: https://github.com/eclipse-zenoh/zenoh/releases ,
  https://zenoh.io/blog/2026-04-16-zenoh-longwang/ ,
  https://zenoh.io/blog/2026-03-18-zenoh-kiyohime/
- Zenoh manuals: https://zenoh.io/docs/manual/configuration/ ,
  https://zenoh.io/docs/manual/abstractions/ ,
  https://zenoh.io/docs/manual/plugin-storage-manager/
- Existing GUIs: https://github.com/dad-io/zenoh-explorer ,
  https://github.com/sanri/zenoh-hammer
- ROS 2 CLI: https://ros2-tutorial.readthedocs.io/en/latest/inspecting_topics.html ,
  https://github.com/ros2/ros2cli/issues/488
- Sparkplug B: https://sparkplug.eclipse.org/specification/version/2.2/documents/sparkplug-specification-2.2.pdf
- Confluent Schema Registry: https://docs.confluent.io/platform/current/schema-registry/fundamentals/serdes-develop/index.html ,
  https://www.infoq.com/news/2026/05/confluent-kafka-header-schema-id/
- DDS XTypes: https://fast-dds.docs.eprosima.com/en/2.14.x/fastdds/dynamic_types/discovery.html
- CloudEvents: https://github.com/cloudevents/spec/blob/main/cloudevents/spec.md
- AsyncAPI: https://www.asyncapi.com/docs/concepts/asyncapi-document/structure ,
  https://docs.confluent.io/cloud/current/stream-governance/async-api.html
- Rust machinery: https://docs.rs/prost-reflect , https://docs.rs/schemars ,
  https://github.com/ericseppanen/cddl-cat
- CLI UX: https://github.com/nats-io/natscli , https://github.com/edenhill/kcat

---

# Round 2 addendum (2026-07-19) — owner directives and zenoh-native refinement

## 11. Owner directives

Marc reviewed rounds 1's report and issues #5–#16 and directed:

1. **Require the most recent zenoh** — deprecate convention machinery zenoh handles natively.
2. **Strongly typed** (String → `OwnedKeyExpr`-class types), Rust-idiomatic, async-friendly.
3. **zenoh `unstable` and zenoh-ext `unstable` are allowed** everywhere.
4. The GUI is named **`zengui`**.
5. **crates.io receives reusable lib crates only** — binaries (zenctl, zengui) are not published.
6. **Performance is a requirement**: CPU, memory, latency, bandwidth.

A second research pass (web + local audit) followed. Version baseline: zenoh 1.9.0
"Longwang" is the latest release (no 1.10 as of 2026-07); zenoh-ext 1.9.0
(2026-07-08). All three repos already pin 1.9 — this round is "build on what is now
native", not "catch up".

## 12. zenoh 1.9 capability adoption matrix

| Native capability | Convention machinery it deprecates/changes | Tracked in |
|---|---|---|
| **Session namespace** auto-prefixes *all* egress (pubs, subs, queries, replies, declarations, liveliness tokens) | App-side manual base handling; `with_base`/`strip_base` become **explorer-only** (zenctl/zengui, router artifacts) | #19 |
| **AdvancedPublisher cache + AdvancedSubscriber history/recovery** (zenoh-ext; the legacy `PublicationCache`/`QueryingSubscriber`/`FetchingSubscriber` are now *deprecated upstream*) | GET-then-subscribe seeding for **volatile** state (alerts/entities/liveliness seeds in zensight); storage-manager remains for durable at-rest data only | #20 |
| **Liveliness subscriber `history(true)`** | The separate `liveliness().get()` roster seed | #14 |
| **`Querier`** (stable declared repeated queries; network optimizes) | Fresh `session.get()` per recurring `fleet_get` (hz/bw/--watch/zengui polling) | #15 |
| **MatchingListener / matching_status** on Publisher *and* Querier | "does anyone consume this?" guesswork; round-1's deferral is lifted for entities we declare ourselves | #15 |
| **`Encoding` constants** (`APPLICATION_CBOR`/`JSON`/`PROTOBUF`; pure metadata) | Sniff-first decoding; sample > registry > sniff resolution becomes normative with named constants | #11 |
| **`ZBytes::reader()`/`slices()`** zero-copy, borrowed `&keyexpr`, alloc-free `intersects`/`includes` | `to_bytes().to_vec()` double copies; `String` key currency in hot loops | #17, #15 |
| **`kedefine!`/`keformat!`/`KeFormat`** typed key formats (format + parse, named fields) | Hand-rolled pattern matcher duplication (backs `SubjectPattern`; zenkey keeps precedence + verbatim handling) | #7 |
| **`sample.timestamp()` + `SourceInfo`** (source_id/source_sn) | Client-clock rate measurement; enables `--loss` gap detection | #13 |
| **`express` flag** | Missing latency axis in the QoS profiles (Alert/Frame → express) | #21 |

**Explicit rejections** (recorded so they aren't relitigated):
- **`z_serialize`/`z_deserialize`** for registry-described payloads: compact but *not
  self-describing* — hostile to the generic decode pipeline (Pillar B). At most for
  internal control frames. CBOR/JSON stay.
- **SHM**: only pays for large co-located payloads — a possible same-host `@media`
  optimization someday, out of zenkey scope.
- **Deprecated zenoh-ext APIs** (`PublicationCache`, `QueryingSubscriber`,
  `FetchingSubscriber`): never adopt.

**Hard constraint on everything above**: zenoh-ext's `@adv` liveliness-token key must
remain structurally parseable through our grammar — the reason the version chunk is
plain, pinned by `zenkey/tests/adv_token.rs` and zensight's
`sensor-core/tests/adv_publisher_detection.rs`. Every grammar/codegen change keeps
those green.

## 13. Key representation — decision record (supersedes §4.1's rejection)

**Decision: `Key` and `Selector` are newtypes wrapping `zenoh_keyexpr::OwnedKeyExpr`,
unconditionally.**

Round 1 rejected `OwnedKeyExpr` for two reasons; both are invalidated:
(a) *"zenkey-build consumes zenkey with default-features = false"* — but
`zenoh-keyexpr` 1.9 is standalone (no tokio, no networking; deps are
hashbrown/keyed-set/serde-class), light enough to be an **unconditional** dependency
usable in build scripts. (b) *"OwnedKeyExpr can't encode base-relative/concrete"* —
true of a *bare* `OwnedKeyExpr`; the **newtype** carries those invariants exactly as
`Key(String)` would, over a better representation.

Consequences:
- `impl Deref for Key { type Target = keyexpr }` → alloc-free `intersects`/`includes`
  and `&keyexpr` borrows for free.
- `From<Key> for zenoh::key_expr::OwnedKeyExpr` is a **zero-cost field move** —
  verified: the workspace lockfile has exactly one `zenoh-keyexpr 1.9.0` and zenoh
  re-exports it; no unsafe, no re-validation, no `autocanonize`.
- Construction: assemble the string once (single-pass write), validate once via
  `try_from`; a property test pins that grammar output is *already canonical*.
- tcgui's ~15 `ke()` re-parse sites become moves; zensight's `TryFrom<String>` →
  `KeyExpr<'static>` dance disappears. Owned + `Send + Sync` → async-friendly.
- The round-1 optional `keyexpr` feature is dropped (dep is unconditional); the
  `zenoh` feature keeps gating only session/QoS mappings.
- Implementation-time check: confirm which feature flags the standalone crate needs
  for the `format` (KeFormat) module.

`Chunk` stays a small validated-string newtype (a single chunk is not a keyexpr
concern).

## 14. Performance program

No `benches/` or criterion exist in any of the three repos today — directive 6 starts
with a baseline. Verified alloc/copy hotspots:

| Site | Problem |
|---|---|
| `zenkey/src/grammar.rs:580` | parse collects `Vec<String>` (one `String` per chunk) — the allocation floor of every parse |
| `zenkey/src/context.rs:82-83`, `:145-147` | double `Vec` (`Vec<String>` + `Vec<&str>`) per `state_key`/`media_key` build |
| `zenkey-build/src/emit.rs:308-313`, `:342` | generated `parse` allocates per bound var; `parse_metric` does `split('/').collect()` per decode |
| `zenctl/src/bus.rs:111`, `:119` | `to_bytes().to_vec()` — `Cow` materialization + owned copy per reply |
| zensight `subscription.rs:610` | full key re-parse (→ `Vec<String>`) per sample in the telemetry hot loop (consumer-side; fixed by borrowed parse views) |
| zensight `sensor-core/advanced_publisher.rs:167-175,180,238` | `format!` key + double `RwLock` acquisition per publish (fixed by prebuilt `Key` + one lock/ArcSwap once 0.3 lands) |

Program (tracked in #17): criterion benches for key build / parse / slug / selector
match / codegen over the fixture corpus, **measured before** the 0.3 rewrite; hotspot
fixes ride with #5/#8/#15; a short zero-copy discipline doc (ZBytes rules, borrowed
`&keyexpr` currency, `intersects` over string ops); `zenkey-fleet` gets a soak bench
proving a hot bus cannot melt a zengui render loop (bounded channels + `Dropped(n)`).

## 15. Publication & naming policy

- **crates.io: lib crates only** — `zenkey` 0.3.0, `zenkey-build` 0.3.0,
  `zenkey-fleet` 0.1.0 (publish order in that sequence).
- **zenctl is no longer published** (0.1.x stays, not yanked; crates.io README gains
  a pointer to GitHub releases). The crate and its Apache-2.0 license remain in-repo.
  Distribution: GitHub release binaries (cargo-dist or equivalent) + `cargo install
  --git`.
- **The GUI is `zengui`** (repo `p13marc/zengui`), Apache-2.0, binaries only —
  never on crates.io. Gated on #15 (zenkey-fleet); MVP scope is §6.4's checklist,
  built strictly against the zenkey-fleet API (a missing type is a zenkey-fleet
  issue, not a zengui workaround).

  > **Amended on implementation (2026-08-08).** zengui was built as a member of
  > *this* workspace (`zenkey/zengui/`), not a separate `p13marc/zengui` repo —
  > the owner's call, for the tighter iteration loop against zenkey-fleet.
  > Everything else in this bullet held: Apache-2.0, `publish = false`, the
  > binary lane (`release.yml` now builds `-p zenctl -p zengui`), and the
  > "a missing type is a zenkey-fleet issue" rule, which produced
  > `zenkey-fleet` 0.3.0 (Monitor `Drop`, `liveliness: Vec<String>`, subtree
  > stats on `TreeNode`) rather than GUI workarounds.
  >
  > Two §6.4 items were amended by contact with the RFCs:
  > - the "registered-vs-wild **flag**" is a **tri-state**, not a bool — a bool
  >   renders "not asked yet" identically to "not registered", which is the
  >   false verdict RFC 05 §3.1 / RFC 12 §9 forbid;
  > - it lives in zengui, not on `TreeNode`. The key tree is rebuilt every
  >   250 ms and its grammar-blindness is what makes it work on an arbitrary
  >   bus; coupling it to the registry (and to user-mutable base/slice state)
  >   would put explorer policy in the engine.
  >
  > Also recorded because it inverts an assumption easy to make twice:
  > `*`/`**` **never cross a chunk beginning with `@`** (RFC 03 §4 D2, verified
  > against zenoh 1.9). So a `**` scope cannot pull `@media` frames — it is
  > media-safe by key algebra, not by policy — but it equally cannot see
  > `@catalog`, so `**` is *not* "everything" and must never be labelled as
  > such.
- Dependency floor: zenoh **1.9** with `unstable`; zenoh-ext 1.9 `unstable` permitted
  in any crate that needs it (zenkey core gains an optional `zenoh-ext` feature only
  where #20's helpers land).

## 16. Amended issue map (round 2)

| Issue | Round-2 action |
|---|---|
| #5 key types | **Amended** — Key/Selector wrap `zenoh_keyexpr::OwnedKeyExpr` (decision record §13) |
| #6 typed origins | unchanged |
| #7 selector/pattern | **Amended** — KeFormat backs parse/format; `intersects`/`includes` for matching; parity test; verbatim-chunk limits noted |
| #8 codegen v2 | **Amended** — builders return `Key`; single-pass construction; parse must not allocate per chunk; `@adv` regression test |
| #9 fanout | unchanged |
| #10 media | unchanged (SHM note: out of scope) |
| #11 self-description | **Amended** — Encoding constants normative; z_serialize rejected; decode via `reader()`/`slices()` |
| #12 zenctl UX | unchanged |
| #13 hz/bw/echo | **Amended** — `sample.timestamp()` for hz; `SourceInfo` `--loss`; borrowed-key bucketing; `ZBytes::len()` for bw |
| #14 admin space | **Amended** — roster via liveliness `history(true)`; admin token endpoint = cross-check; storage-coverage scope narrows per #20 |
| #15 zenkey-fleet | **Amended** — consumer is zengui; Querier; matching badges; ZBytes rules; soak bench; published to crates.io |
| #16 CI/release | **Amended** — libs-only publication; two release lanes; zenoh floor enforced; zenkey-fleet 0.1.0 |
| #17 (new) | Performance program: criterion baseline + hotspot fixes + zero-copy discipline |
| #18 (new) | zengui bootstrap (named, not published, gated on #15) |
| #19 (new) | Session-namespace audit: `with_base`/`strip_base` explorer-only (RFC 09) |
| #20 (new) | Late-joiner seeding delegated to advanced pub/sub (RFC 04) + linked zensight issue |
| #21 (new) | QosProfile `express` axis (RFC 04 §3) |

(Issue numbers #17–#21 verified against the tracker 2026-08-08: all five match. #19/#20/#21 closed as RFC amendments; #17 continues in epic #33's #44/#45; #18 delivered by PR #32.)
