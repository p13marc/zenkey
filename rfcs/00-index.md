# Zenoh Semantic Convention RFC — Index

**Status: v1.8 — PROPOSED** (v1.0 2026-07-12; adopted for ZenSight, migration
tracked in [#453](https://github.com/p13marc/zensight/issues/453) with the
enforcement crate `zenkey`; v1.5 ratifies on merge of the 0.3 redesign
branch).

> **v1.8 (2026-07-29, the `@blob` registry amendment)** — v1.7 re-specified
> `@blob` and left it, by its own admission, the one plane with no registry
> entry kind. That is a strange place to stop: the plane that moves whole
> files was the only one no build-lint and no bus explorer could see. v1.8
> closes it. [08 §2](08-registry.md) gains a field table; nothing else in the
> convention moves.
>
> | | Chapter | What |
> |---|---|---|
> | **C1** | [08 §2](08-registry.md) *(new table)* | **`[[blob]]` is an entry kind.** An origin declares which **tier** it serves (`artifact` \| `tree` \| `store`) and, for `artifact`, which of [07 §2.2](07-bulk-planes.md)'s reserved endpoints (`manifest`, `slice`, `have`, `push`, `fanout`); plus `algo` on `store`, an optional `reference` type (the payload that must carry the content root, [07 §2.1](07-bulk-planes.md)), and an optional content `encoding`. Each producer file declares the tiers *that producer* serves, and several files MAY declare one tier — the introspect slice is per-producer truth — with all declarations of a tier agreeing in shape, deduped by codegen into one app-level surface that records every declarer. Deliberately **no `path`** — alone among the kinds, blob key shapes are fixed by chapter 07 and their variable chunks are content addresses, not registry vocabulary — and **no `cardinality`**, since [03 §3](03-grammar.md) already carves blob ids and tree roots out of the budget as sanctioned unbounded families. Asking for a number would have invited a fiction and then budget-reviewed it. |
> | **C2** | [08 §2](08-registry.md) | **Two chapter-07 rules become structural in the generated surface**, the move H1 made for forbidden-fanout writes. `tree`/`store` builders take a validated content-hash type, so [07 §2.3](07-bulk-planes.md)'s revoked caller-chosen name (`tree/nightly`) has *no spelling* in generated code; and the [§2.5](07-bulk-planes.md) probe form returns a distinct probe-prefix type, so a probe prefix cannot be passed where §3's prohibition forbids one. A rule the codegen refuses to spell is a rule nobody has to remember. |
> | **C3** | [08 §5](08-registry.md), [§7](08-registry.md) | **Build-time enforcement and describe totality.** The blob vocabularies are closed, so all of it is decidable: `tier` in range; `endpoints` present exactly on `artifact` and drawn from the reserved set; `algo` present exactly on `store`; `since`/`description` present; all declarations of one tier agree in shape (`endpoints` as a set, `reference`, `encoding`) across the registry set, with the same `(tier, algo)` at most once per *file*; a second concurrent `store` algo is a build diagnostic until codegen learns per-algo builders; `reference` resolves in the shared type table. §7's totality clause now names `reference`, so a declared blob type must be covered by `describe`. |
> | **C4** | [07 §2.7](07-bulk-planes.md) *(new)*, [08 §6](08-registry.md) | **Blob tiers reach runtime introspection** — the stated point of the exercise: an explorer can see which origins serve blobs, and of which tier. This exposes a **pre-existing** gap rather than creating one, and §6 is corrected instead of quietly widened: through v1.7 it claimed the introspect slice carried "media shapes", and it never has. Retrofitting `[[media]]` into the slice is separate work and is not bundled here. |
> | **C5** | [12 §8.2](12-open-questions.md) | **A revisit trigger fired and is recorded.** §8.2's condition — "a rejected amendment reopens only if the chapter it was rejected against changes" — was met by v1.7's rewrite of chapter 07. Re-read rather than recalled (§8.1's own lesson): the rewrite *strengthened* the rejection, and C2 now enforces it in the type system. The amendment stays rejected. An unexamined trigger that silently never fires is indistinguishable from one whose condition was met and ignored. |
>
> **What did *not* change.** No grammar change and no wire change: position
> counts, the tier tokens, chunk lexical rules and design properties D1–D6 are
> untouched (the D1–D6 guard tests and the `@adv`-token test pass unmodified),
> and [07 §2](07-bulk-planes.md)'s key shapes, endpoints, integrity anchoring
> and QoS obligation are exactly as v1.7 left them — §2.7 describes how they
> are *modelled*, and changes none of them. The registry TOML format is
> extended purely additively: no existing registry file needs an edit, and an
> application that serves no blobs writes nothing. `[[media]]`, `[[subject]]`
> and `[[procedure]]` field tables are untouched. Declaring `push` remains a
> statement of *capability*: [07 §2.2](07-bulk-planes.md)'s authorization gate
> and off-by-default posture are unaffected by anything a registry says.

> **v1.7 (2026-07-28, the `@blob` re-specification, zblob wire v2)** — the
> bulk-transfer plane is re-specified against the reference client's
> verified-streaming protocol. [07 §2](07-bulk-planes.md) is rewritten;
> nothing else in the convention moves.
>
> | | Chapter | What |
> |---|---|---|
> | **B1** | [07 §2.3](07-bulk-planes.md) | **Tier-2 keys become content-addressed.** A tree index is keyed by its own root hash (`@blob/tree/<root>`), and snapshot *names* move to `state/<producer>/snapshot/<name>` carrying that root. Both [03 §3](03-grammar.md) ("tree-index ids under `@blob/tree` — the root hash of the tree they index", one of the four sanctioned unbounded-cardinality exceptions) and 07 §2 v1.2 ("tree ids are root hashes", the premise under fleet-wide cacheability, the storage PUT exemption, and no-op last-writer-wins) already *said* this — neither made it a requirement, and the reference client allowed a caller-chosen name, which falsified all three conclusions at once. v1.7 does not invent the rule; it states it normatively where the keys are defined and makes the implementation obey it. Immutable content on `@blob`, mutable names on `state` — the objects/refs split. |
> | **B2** | [07 §2.1](07-bulk-planes.md) *(new)* | **Integrity is anchored, not assumed.** Every `@blob` reference MUST carry the identity of the bytes it names: Tier-1 references carry the content root, Tier-2 keys *are* the root (B1). Without an anchor a consumer can only trust whoever answers first — integrity holds within a transfer but not across it. Verification is per-reply and pre-disk, superseding v1.2's whole-blob check that could only fail *after* paying for everything. |
> | **B3** | [07 §2.2](07-bulk-planes.md) | **The per-artifact endpoints are named and normative** (`manifest`, `slice/<i>`, `have`, `push/offer`, `push/slice/<i>`, `fanout`), with two corrections: `@blob` is no longer uniformly queryables — `fanout` is a *publication* (one-to-many rollout, the case where N pulls is the amplification this plane avoids) — and `push/**` is a write path expressed as a query, which MUST be authorization-gated and off by default. Resume is a persisted chunk bitfield: a hole in the middle no longer re-streams the tail. |
> | **B4** | [07 §2.4](07-bulk-planes.md) *(new)* | **Chunk values are framed; the hash addresses the content.** A chunk value is a self-describing container (uncompressed or named compression) and `<hash>` is the hash of the *content*, so a holder MAY re-frame what it stores without changing the address — which is what keeps dedup intact across holders with different storage policies. An **encrypted** container MUST NOT appear under a fleet-reachable `store` key: encryption at rest is one disk's property, not the shared address space's. |
> | **B5** | [07 §2.5](07-bulk-planes.md) | **Probing is a named endpoint** (`have`, returning an availability bitfield), not the vague "tiny replies" of v1.2 — a client can now choose the best-stocked holder rather than the first to answer. Also: a resolved PUT is hand-off, not retention, so a producer that intends to exit MUST read back what it published. |
> | **B6** | [07 §2.6](07-bulk-planes.md) | **The bulk-QoS obligation is discharged by default** in the reference client rather than merely documented — a client that never touches the setting is conformant, and raising priority is the deliberate act. The `fanout` publication additionally uses blocking congestion control. |
>
> **What did *not* change.** No grammar change: position counts, the tier
> tokens (`artifact` | `tree` | `store`), chunk lexical rules and design
> properties D1–D6 are untouched (including [03 §3](03-grammar.md)'s
> unbounded-cardinality carve-out, which already named the root hash) —
> `tree/<root>` is still
> `<tier-token>/<one chunk>`, only the chunk's *provenance* is now
> normative. §1 (`@media`) is untouched. §3's wildcard rule is restated, not
> revised: B5 names the probing mechanism the rule already assumed, and the
> prohibition on wildcard-origin *bulk fetch* is unchanged. §4's rationale
> for planes-not-payloads stands. Tier-1 ids remain the RPC-minted ULID.
> The registry format is untouched — modelling `@blob` in the registry
> remains open (it is the one plane with no entry kind). *(Closed by v1.8,
> above.)* One editorial
> touch outside 07: [03 §2](03-grammar.md)'s plane table said `@blob` is
> "queryables"; it now reads "queryables (+ `fanout` pub)" to match B3.
>
> **Migration.** Wire-breaking for Tier-2 by construction: a named tree key
> becomes a root-hash key, and the name moves to `state`. Tier-1 keys are
> unchanged; what changes is that a reference without a root is no longer
> conformant. Both land together with the reference client's v2 wire, which
> is itself a clean break (BLAKE3 verified streaming, postcard control
> messages, chunk-range resume) — so there is one migration, not two.

> **v1.6 (2026-07-20, the empty-base amendment)** — the **empty base becomes a
> licensed deployment configuration**, superseding the v1.5 note below that
> kept it an observed wire condition. Rationale: Zenoh's own default is *no*
> session namespace, and a convention deployment must be able to ride a
> default-configured session as-is — the base is an isolation knob one opts
> *into*, not a toll every deployment pays.
>
> [03 §1.1](03-grammar.md): `<base>` MAY be **empty** (zero chunks). A
> base-less deployment sets no session namespace and its full wire keys start
> at `v1/…`; everything else in the convention is untouched (application keys
> were base-relative already, so for the empty base the two views of a key
> coincide). The base remains the isolation boundary for shared
> infrastructure: two deployments on one Zenoh network MUST still use
> distinct bases, and the empty base counts as one base value — a mismatch
> (empty vs. named, or two names) partitions exactly like any other. Observer
> tooling already treats the empty base as legal input (v1.5 N3:
> `with_base`/`strip_base` are identities for it; `zenctl --base ""`) —
> unchanged, except that a `v1/…` wire is now a legal deployment rather than
> an off-convention condition ([09 §5](09-operations.md)).

> **v1.5 (2026-07-19, the 0.3 redesign amendments, issues
> [#5](https://github.com/p13marc/zenkey/issues/5)–[#21](https://github.com/p13marc/zenkey/issues/21),
> design record in [docs/redesign-2026-07.md](../docs/redesign-2026-07.md))**
> — the convention's enforcement crate grew up: typed everywhere the RFCs
> already demanded it, delegating to the middleware everywhere Zenoh 1.9
> provides the mechanism natively.
>
> | | Chapter | What |
> |---|---|---|
> | **H1** | [08 §1.2](08-registry.md) *(new)* | **The generated surface.** Builders return a **validated key type** (canonical, concrete, base-relative), never `String`; generated constructors **slug at the API boundary** so key construction from a well-formed subject is infallible; the §1.1 typed origins (`LocalOrigin`/`RemoteOrigin`/service/fleet) live in the enforcement crate itself; G2 becomes **structural** — a forbidden-fanout write has *no fleet spelling in the generated surface*; each subject family generates a fieldless family id with per-family selector builders. |
> | **H2** | [08 §2](08-registry.md) | **Media codegen delivered.** The v1.3 promise ("generated key builders" for `[[media]]`) is implemented: media value type, slugging constructors, local-origin publish builder, remote-origin viewer builder — and deliberately **no** wildcard/family selector (the 07 §1 tier-wildcard revocation). `variant` becomes legal on media entries. |
> | **H4** | [08 §5](08-registry.md) | **Desired-state `{host}` lint.** In a service registry, a subject pattern containing `{host}` MUST lead with it (G1's proxy rule as CI), and generated constructors type it as a host id. |
> | **H5** | [08 §7](08-registry.md) *(new)* | **Payload self-description.** Producers serve `@rpc/<producer>/describe` — a SchemaSet JSON document (type name → kind + sha256 hash + schema; kinds registered now: `json-schema`, `protobuf` as a base64 FileDescriptorSet). SHOULD for self-describing encodings, **MUST** where a referenced type rides protobuf. No per-sample schema ids — evolution stays additive-only under §3's suffixed-sibling rule. |
> | **H6** | [08 §2/§5](08-registry.md), [04 §3](04-planes.md) | **Encoding + the materialized type table.** Optional `encoding` on `[[subject]]`/`[[procedure]]`; producers SHOULD set the sample `Encoding` (resolution: sample > registry > sniff — the sniff stays). The §5 "shared type table" becomes a concrete artifact: `registry/types.toml`, resolution-linted by codegen. |
> | **H7** | [04 §3.5](04-planes.md) *(new)* | **Late-joiner seeding delegated.** Volatile-state seeding moves to the middleware's advanced-tier cache + history/recovery (its legacy cache APIs are deprecated upstream); the storage-manager remains authoritative for durable at-rest data. Rests on the plain version chunk (`@adv` token parseability). |
> | **N1** | [09 §4/§5](09-operations.md) | **Base handling is the session's.** The session namespace prefixes/strips every egress/ingress; `with_base`/`strip_base`/`parse_full` are reclassified as observer-side tools (explorers, router artifacts, tests). |
> | **N2** | [12 §9](12-open-questions.md) *(new)* | **Matching-status introspection deferred** — tooling shows its own matches, never infers fleet verdicts from foreign publishers' silence. |
> | **N3** | [09 §5](09-operations.md) | **Base discovery + the empty-base observer.** An observer recovers the bases in use from the wire itself: a liveliness sweep with the base wildcarded (`**/v1/*/state/*/alive`, plus `@catalog` by name — D4) and the router storage configs' `key_expr`/`strip_prefix`. Observer tools MUST accept the *empty* base as input (`with_base`/`strip_base` are identities for it): an off-convention wire whose keys start at `v1/` is precisely what a debug tool must be able to see and name. |
> | **E1** | [04 §3](04-planes.md) | **The `express` axis.** Zenoh's per-message `express` flag joins the profile table as a fourth axis: `alert` and `frame` set it, the throughput-shaped profiles do not. Rejected alternative: a per-key `express` registry override — it would reopen the per-key QoS bikeshed the closed five-profile vocabulary exists to prevent. |
>
> **What did *not* change.** No grammar change — position count, chunk
> lexical rules, the plain `v1` version chunk, and design properties D1–D6
> are untouched (the D1–D6 guard tests and the `@adv`-token test pass
> unmodified). The registry TOML format is extended only additively
> (`variant` on media); no existing registry file needs an edit. Payloads
> are untouched by this half of v1.5 (payload self-description arrives in
> the H5–H7 rows, appended when that part of the branch lands). N3 does not
> touch [03 §1.1](03-grammar.md) either: a *deployment* still MUST set a
> ≥ 1-chunk base — the empty base is an observed wire condition, not a
> licensed configuration.

> **v1.4 (2026-07-18, actuator-adoption amendments, issue
> [tcgui#43](https://github.com/p13marc/tcgui/issues/43))** — six additive
> amendments from migrating a *side-effecting actuator* (a tc/netem
> traffic-control agent) onto v1.3: the first **writer of shared kernel state**
> to adopt the convention, which surfaced gaps every read-only sensor had left
> latent. Each was fact-checked against the ratified chapter text before it was
> kept.
>
> | | Chapter | What |
> |---|---|---|
> | **G1** | [07 §3](07-bulk-planes.md) (+ [12 §3](12-open-questions.md), [05 §3](05-control-rpc.md), [04 §2](04-planes.md), [09 §3](09-operations.md)) | **Service-origin escape hatch for desired-state.** A *registered service origin* (e.g. `@desired`) MAY publish durable desired-state on behalf of a target, and MUST place the **target host as the first subject chunk** — the proxy-producer rule ([03 §1.6](03-grammar.md)) applied to the origin position. Grammar-legal, zero new mechanism; the ratified escape hatch (12 §3) had never been spelled as a concrete key. |
> | **G2** | [08 §2](08-registry.md), [05 §2.1](05-control-rpc.md) | **Read vs write fan-out.** Procedures gain a `fanout = forbidden \| allowed` field (default `forbidden` for `kind = "write"`); a `*`-origin fan-out call to a forbidden-fanout write **MUST** be refused (builder, registry, or ACL). Fan-in was safe to broadcast for *reads*; broadcasting a *write* actuates the whole fleet. |
> | **G3** | [03 §1.5](03-grammar.md) | **Exclusivity is off-bus.** Liveliness/claim tokens are **presence, never a lock**; a side-effecting producer **MUST** enforce single-writer exclusivity *outside* the bus (file lock / systemd unit / netlink), because convergence can reconcile a state *document* but cannot undo a *side effect*. |
> | **G4** | [03 §2](03-grammar.md) | **Grammar erratum.** The `_xNN_` escape can itself emit a leading `_` (infinite regress on `_myns`); the slug MUST guarantee an **alphanumeric first character**. And case-sensitive identifier domains (Linux interface names, `eth0` ≠ `ETH0`) are **exempt** from the lowercasing MUST — the injectivity MUST in the same section wins. |
> | **G5** | [08 §1.1](08-registry.md) | **Typed origin MUST for writes.** The origin-kind SHOULD-be-a-type is upgraded to **MUST** for `kind = "write"` builders (a mis-typed origin actuates the wrong — or one's own — host); it stays SHOULD for read-only builders. |
> | **G6** | [09 §3](09-operations.md) | **Sub-host ACL needs the resource in the path.** ACL matches keyexpr *inclusion* and cannot match selector parameters, so sub-host authority ("may shape `eth1`, not the management NIC") requires the actuated resource to be a **path chunk** (`@rpc/<producer>/config/{ns}/{if}/set`), never a `?if=` selector. Bounded interface populations make this grammar-legal. |
>
> **What did *not* change.** No wire change and **no grammar change** — every
> amendment is prose that makes an existing rule bind a new caller class
> (writers/actuators). Position count, version chunk, and all payloads are
> untouched; all six are additive.

> **v1.3 (2026-07-15) — `@media` tiers; wire-breaking payloads, key grammar
> unchanged** ([#494](https://github.com/p13marc/zensight/issues/494)). The
> `@media` video key's last chunk changes meaning from an undefined `<profile>`
> to a normative **`<tier>`** — a named bandwidth rung the publisher offers
> concurrently and the viewer subscribes to *exactly*. Three linked changes:
> **(1)** [07 §1](07-bulk-planes.md) defines `<tier>` and **revokes** the
> `…/video/<codec>/*` wildcard (amendment F′ licensed it against "the viewer
> cannot know this chunk"; the catalogue now publishes the tiers, so the viewer
> *can* — and the `*` would match every tier at once, breaking simulcast).
> **(2)** [08 §2](08-registry.md) gives `[[media]]` a real contract — its own
> normative field table, `attachment` CI-resolved against the shared type
> table, `cardinality` required on `{var}` media paths, and generated key
> builders. **(3)** The stream-control payloads (`StreamControl`,
> `StreamStatus`, `StreamDescriptor`) move to a tier-oriented, capability-bearing
> shape. The **key grammar is unchanged** — position count and version chunk
> stay `v1`; only one chunk's *meaning* and the control payloads move — but the
> payload change is wire-breaking, and backward compatibility was explicitly not
> a constraint for this release.

> **v1.2 (2026-07-14) — six amendments, all additive, no wire change**
> ([#467](https://github.com/p13marc/zensight/issues/467)). Every one is a
> lesson from actually migrating a real application onto v1.0/v1.1 — and each
> was fact-checked against the ratified chapter text before it was kept. Two
> proposed amendments were **dropped** because the RFC already said it (see
> "what did *not* change", below); that is the more useful half of the record.
>
> | | Chapter | What |
> |---|---|---|
> | **A** | [06 §6](06-identity.md) *(new)* | **The consumer identity bridge.** The payload `host_id` **is** the origin id; a consumer holding a *hostname* MUST resolve it to an origin before building an origin-scoped key. `host_id` appeared **nowhere** in 06, and §5.1 only ran origin → entity — never "I have a box, what key do I build?". A UI built on the missing half took every drill-down in the reference product down at once. **This is the amendment that would have prevented the outage.** |
> | **B** | [08 §1.1](08-registry.md) *(new)* | **The origin is an argument too.** The codegen contract is build/parse × **local/remote**, and the origin's kind SHOULD be a *type*, so "I built a key for my own host by accident" is a compile error rather than a timeout. Shipped as a bug three times. |
> | **C** | [08 §5, §6.1](08-registry.md) | **The registry MUST NOT lie.** registry ⊆ served is upgraded from "a finding" to a **MUST**, with the reverse-direction lint. The reference registry advertised **seven** surfaces no build served; `introspect` was shipping them to the fleet as truth. Also: the forward lint is *vacuous* wherever a producer registers a catch-all subject. |
> | **D** | [09 §0.1](09-operations.md) *(new)* | **Discovery and scouting.** `scout`/`gossip`/`multicast` had **zero hits across all 13 chapters**. Multicast and gossip are *independent* switches; isolated verification is multicast **off**, gossip **on**; a gossip-less hub silently breaks spoke→spoke discovery. |
> | **F′** | [07 §1, §3](07-bulk-planes.md) *(new §3)* | **The wildcard rule.** *A publisher MUST always use its concrete origin; a subscriber MAY wildcard a chunk it cannot know* — and on the bulk planes, not even then. §1 licensed `*` for the *profile* chunk; nothing forbade wildcarding the **origin** on `@media`, which subscribes to every host's stream of that name. |
> | **G** | [09 §6](09-operations.md) *(new)* | **Cutover acceptance.** A cutover is not done until the retired family is provably **silent** *and* a **consumer-shaped, concrete-key** probe passes. A `*`-origin probe cannot catch a broken origin path — the reference smoke was green while the product was entirely broken. |
>
> **What did *not* change, deliberately.** [05 §2.1](05-control-rpc.md) (fan-in
> call discipline) was proposed for amendment and **left alone**: it already
> mandated query target `All` and replying on the producer's own concrete key,
> as bolded MUSTs, with the right reasoning. Both were hit as real bugs — not
> because the chapter was silent, but because it had not been read. An
> editorial note now says so in place, so nobody "fixes" a section that was
> right. Likewise a proposed `@blob` wildcard amendment was dropped: 07 §2
> already said the **opposite** of what was proposed, and correctly.
>
> Also fixed: 07 §1 cited `05 §5` twice for the stream-control RPC idiom; the
> normative home is **05 §3**.

> **v1.1 (2026-07-14) — one amendment.** The version chunk is a **plain** `v1`,
> not the verbatim `@v1` of v1.0. Verbatim made zenoh-ext's `@adv`
> publisher-detection tokens structurally unparseable (`**` cannot cross an `@`),
> silently killing late-publisher detection; the invisibility it bought was a
> *migration* property we no longer need, while cross-major isolation never
> depended on it. Wire-breaking. See [03 §1.2](03-grammar.md) and
> [12 §7](12-open-questions.md).

Drafting history: round 1
adversarial review + Zenoh 1.9 source verification + D-Bus/Homie/OPC-UA
research; round 2 base = session namespace + storage guidance; round 3
delivery re-grounded (stable baseline default, advanced pub/sub a priced
opt-in tier); round 4 all open questions decided
([12-open-questions.md](12-open-questions.md) is the decision record). ·
supersedes the exploratory drafts in `zensight-key-semantic/` (credited in
[03 §6.2](03-grammar.md)) · does **not** replace
[`docs/KEYSPACE.md`](https://github.com/p13marc/zensight/blob/master/docs/KEYSPACE.md), which remains authoritative for
the shipped keyspace.

A key-space convention for Zenoh applications: how to shape key
expressions so that routing, subscriptions, storage selection, access
control, and bandwidth policy all fall out of the grammar instead of being
re-implemented per consumer. Written application-neutrally; **ZenSight** is
the reference application and supplies the worked examples.

---

## The convention on one page

```
<base>/v1/<origin>/<class>/<producer>/<subject...>
```

| Position | Chunk | Example |
|---|---|---|
| 1 | **base** — deployment root (config; tenancy = deployment prefix; normally the session **namespace**, so app code never spells it; MAY be empty — the base-less bus-root deployment, v1.6) | `zensight` · *(empty)* |
| 2 | **version** — plain `v<int>`; majors are mutually invisible by key algebra | `v1` |
| 3 | **origin** — who publishes: self-minted stable host id, or verbatim service | `h-3fa9c2d41b7e` · `@catalog` |
| 4 | **class** — bus semantics: `telemetry` (superseded) · `state` (LWW+tombstone) · `events` (immutable) · verbatim planes `@rpc` · `@media` · `@blob` | `state` |
| 5 | **producer** — the component that produced it (`name[-instance]`; omitted under service origins) | `netlink` |
| 6+ | **subject** — open-ended, registry-governed meaning path | `alert/9f2c81ab04d7e3f1` |

Normative examples (base = `zensight`):

```
zensight/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage
zensight/v1/h-3fa9c2d41b7e/telemetry/snmp/router01/system/sys_uptime
zensight/v1/h-3fa9c2d41b7e/state/netring/health
zensight/v1/h-3fa9c2d41b7e/state/netlink/alert/9f2c81ab04d7e3f1
zensight/v1/h-3fa9c2d41b7e/state/netring/evidence/names/10-0-0-7
zensight/v1/h-3fa9c2d41b7e/events/netring/capture/01jgxqz4yqk8v6txw3m9f2a7cd
zensight/v1/h-3fa9c2d41b7e/@rpc/netlink/sockets
zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/high
zensight/v1/h-3fa9c2d41b7e/@blob/store/sha256/ab12cd34ef56
zensight/v1/@catalog/state/entity/h-3fa9c2d41b7e
zensight/v1/@catalog/state/pdns/93-184-216-34
```

The canonical selectors, and the properties that make them safe, are in
[03 §4–5](03-grammar.md); the headline property: a per-host subscription
`zensight/v1/h-xxx/**` delivers that host's complete data plane and can
never pull keys under `@rpc`/`@media`/`@blob` — by key algebra; that
frames and bulk actually live there is the registry's placement rule
(the theorem/precondition split of [03 §4](03-grammar.md)).

## Glossary

| Term | Meaning |
|---|---|
| **base** | the deployment's root chunk(s); everything the convention defines lives under it. MAY be empty (v1.6): the base-less deployment sets no session namespace and lives at the bus root |
| **origin** | the publishing identity in every key — a host id or a named service |
| **class** | the update semantics of a subtree: telemetry / state / events |
| **plane** | a verbatim-isolated subtree no data wildcard can reach: `@rpc`, `@media`, `@blob` (the version chunk uses the same verbatim mechanism but is not a plane) |
| **producer** | the component (sensor/agent/service) that emits the data |
| **subject** | the registry-governed meaning path — the open part of the key |
| **catalog** | the singleton service that fuses identity evidence into entities; the only author of identity *conclusions* |
| **registry** | the machine-readable inventory binding every subject/procedure to a payload type, QoS, and lifecycle |
| **sidecar** | the `@adv` machinery keys zenoh-ext parks under a data key (`<key>/@adv/…`: publisher cache, liveliness, heartbeat) — advanced-tier-only, verbatim-isolated, ACL-relevant, never application-published, never a presence roster |

## Reading order

Chapters are numbered for reference, not reading. Suggested paths:

- **Evaluating the design** (reviewers): 01 → 03 → 04 → 05 → 06 → 12,
  with 10 for the influences and 03 §6 for the roads not taken.
- **Adopting the convention** (other Zenoh apps): 02 → 03 → 04 (delivery
  contracts §3.1–3.4 especially) → 08 → 09, then 11 §4 for the
  replace-this checklist.
- **Operating a deployment**: 09, with 04 for the class semantics behind
  the recipes.

## Chapters

| # | File | What it holds |
|---|---|---|
| 00 | this file | grammar-on-a-page, glossary, reading order |
| 01 | [01-motivation.md](01-motivation.md) | the shipped keyspace, its eight structural pain points, goals and non-goals |
| 02 | [02-principles.md](02-principles.md) | the eleven design principles, each with provenance |
| 03 | [03-grammar.md](03-grammar.md) | **normative core**: conformance model, chunk-by-chunk grammar, lexical rules, reserved tokens, design properties D1–D6 (theorems + preconditions), alternatives considered |
| 04 | [04-planes.md](04-planes.md) | class semantics (telemetry/state/events), placement rules, QoS profiles, delivery contracts + baseline + opt-in advanced tier, storage mapping, liveliness |
| 05 | [05-control-rpc.md](05-control-rpc.md) | the `@rpc` plane: targeting, read/write/long-running idioms, mapping of every incumbent control channel |
| 06 | [06-identity.md](06-identity.md) | origin minting, observed devices, evidence, the `@catalog` contract |
| 07 | [07-bulk-planes.md](07-bulk-planes.md) | `@media` (live frames) and `@blob` (bulk/content-addressed transfer) |
| 08 | [08-registry.md](08-registry.md) | the subject registry: format, versioning policy, naming rules, ownership |
| 09 | [09-operations.md](09-operations.md) | cookbook: session/namespace config, selectors, storage (volumes, replication, GC), ACL recipes (rules/subjects/policies, per-plane), constrained-link policy |
| 10 | [10-prior-art.md](10-prior-art.md) | Keelson, uProtocol/automotive, rmw_zenoh, Sparkplug, OTel, NATS, Zenoh guidance, D-Bus, Homie, OPC UA — took/rejected per system |
| 11 | [11-zensight-profile.md](11-zensight-profile.md) | the reference application: profile constants, worked keys per sensor, full shipped-family mapping |
| 12 | [12-open-questions.md](12-open-questions.md) | the decision record: all six former open questions decided, each with its alternatives and revisit trigger |

## Scope

**In scope**: the key grammar and its semantics; the class/plane system;
the RPC, identity, media, blob, and registry contracts; operational
recipes.

**Out of scope** (by decision, see [01 §5](01-motivation.md)): metric
renaming, multi-tenancy machinery, payload schema *contents* (their
**transport** is in scope since v1.5 — [08 §7](08-registry.md)), and —
deliberately — any migration plan. Convention majors are mutually invisible by
key algebra (`v1` and `v2` are different literal chunks), so two majors can share
a network indefinitely; when and how to walk across is a separate decision.
(In v1.0 the version chunk was verbatim `@v1`, which additionally hid v1 from an
*un-versioned* selector. It no longer is — see [03 §1.2](03-grammar.md).)
