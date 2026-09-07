# Zenoh Semantic Convention RFC — Index

A key-space convention for Zenoh applications: how to shape key
expressions so that routing, subscriptions, storage selection, access
control, and bandwidth policy all fall out of the grammar instead of being
re-implemented per consumer. Written application-neutrally; **ZenSight** is
the reference application and supplies the worked examples.

**Status: v1.35** (2026-09-06; ratified at v1.18, 2026-08-15; v1.0
2026-07-12; adopted for ZenSight, migration tracked in
[#453](https://github.com/p13marc/zensight/issues/453) with the
enforcement crate `zenkey`). The full amendment ledger — every version,
what changed and what deliberately did not — is
[CHANGELOG.md](CHANGELOG.md). The last three amendments, one line each:

- **v1.35** (2026-09-06) — the conditional field: v1.20's deferral ends;
  a subject or procedure declares `when = ["feature:…", "config:…",
  "capability:…"]` (ANDed, three kinds, a `gate_note` for the human), the
  kinds bind the error a gated procedure answers, `introspect` carries the
  condition, the `conditional.lock` ledger becomes the legacy spelling for
  one cycle, a gate is not a shape in the lock, and 13 §3 gains the `when`
  judgement plus the conformance-suite obligation (three states, unknowable
  never failed) (08 §2, §3.1, §6.1; 13 §3).
- **v1.34** (2026-09-06) — the state artifacts: `.zrec` version 2 carries a
  state preamble (rows marked, at `t: 0`, original HLCs kept as provenance)
  and a trigger record, with replay skipping the preamble unless told to seed
  state; a `.zsnap` snapshot is a fan-in GET on disk that states the span it
  was collected *over* and holds per-key evidence (stamper, registration
  rung, verdict, holder); exporters owe the observer rules in their own
  medium — bounds as separate series, a stopped series named, provenance on
  the surface; and an observation-derived registry draft carries a marker
  the build refuses unless admitted (13 §4.1, §4.4, §3; 08 §6.1).
- **v1.33** (2026-09-06) — the generators: a fifth ACL fact — interest is
  evaluated on egress against the responding face's subject, so own-origin
  grants starve a peer-mode publisher of interest and it publishes to nobody;
  a shared egress-only `interest-prop` row fixes it — and the two 09 recipes
  become `zenctl storage gen` and `zenctl acl gen`, with the running ACL
  recorded as unobservable in zenoh 1.10 (09 §2, §3).

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
| **plane** | a verbatim-isolated subtree no data wildcard can reach: `@rpc`, `@media`, `@blob` (the version chunk used the same verbatim mechanism in v1.0 but was never a plane; it is plain `v<int>` since v1.1 — [03 §1.2](03-grammar.md)) |
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
- **Building a tool that reads or judges the bus**: 13, with
  05 §3.1 for the reply-set instance of its silence rule and 08 §6–§7 for
  the registry surface a tool renders.

## Chapters

| # | File | What it holds |
|---|---|---|
| 00 | this file | grammar-on-a-page, glossary, reading order; the amendment ledger is [CHANGELOG.md](CHANGELOG.md) |
| 01 | [01-motivation.md](01-motivation.md) | the shipped keyspace, its eight structural pain points, goals and non-goals |
| 02 | [02-principles.md](02-principles.md) | the eleven design principles, each with provenance |
| 03 | [03-grammar.md](03-grammar.md) | **normative core**: conformance model, chunk-by-chunk grammar, lexical rules, reserved tokens, design properties D1–D6 (theorems + preconditions), alternatives considered |
| 04 | [04-planes.md](04-planes.md) | class semantics (telemetry/state/events), placement rules, QoS profiles, delivery contracts + baseline + opt-in advanced tier, storage mapping, liveliness |
| 05 | [05-control-rpc.md](05-control-rpc.md) | the `@rpc` plane: targeting, read/write/long-running idioms, the incumbent-channel mapping pattern (row-by-row table: 11 §5, since v1.25) |
| 06 | [06-identity.md](06-identity.md) | origin minting, observed devices, evidence, the `@catalog` contract |
| 07 | [07-bulk-planes.md](07-bulk-planes.md) | `@media` (live frames) and `@blob` (bulk/content-addressed transfer) |
| 08 | [08-registry.md](08-registry.md) | the subject registry: format, versioning policy + compatibility lock (§3.1), naming rules, ownership |
| 09 | [09-operations.md](09-operations.md) | cookbook: session/namespace config, selectors, storage (volumes, replication, GC), ACL recipes (rules/subjects/policies, per-plane), constrained-link policy, base discovery; tombstones for the tool material moved to 13 (v1.24) |
| 10 | [10-prior-art.md](10-prior-art.md) | Keelson, uProtocol/automotive, rmw_zenoh, Sparkplug, OTel, NATS, Zenoh guidance, D-Bus, Homie, OPC UA — took/rejected per system |
| 11 | [11-zensight-profile.md](11-zensight-profile.md) | the reference application: profile constants, worked keys per sensor, the byte-precise alert key (§3.1, v1.25), full shipped-family and incumbent-channel mapping (§5, v1.25) |
| 12 | [12-open-questions.md](12-open-questions.md) | the decision record: all six former open questions decided, each with its alternatives and revisit trigger; §9 carries the matching-badge adoption note (v1.14) |
| 13 | [13-observer-conformance.md](13-observer-conformance.md) | **observer conformance** (normative, v1.24): the judgment shape and its exit-code projection, the general silence rule, observer obligations O1–O7 with per-medium consequences and conformance-test shapes, `.zrec` capture/replay (format minimum + etiquette), the synthetic marker, cutover acceptance |

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
