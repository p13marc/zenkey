# ZenSight, mapped onto zk2 (#622)

ZenSight is the reference adopter of v1. This directory maps it onto zk2 r3.2
(`docs/zk2/architecture.md`), first in the draft-0 authoring format and now in
draft 1 ([`../README.md`](../README.md)). It is a paper exercise: nothing here
is built, and `zenkey-model` (#608) validates every file.

**Verdict.** r3.2 carries ZenSight. All 22 registries map, family by family,
onto two shapes: *system = host* (`hostid.v1`) and *device-as-service*. Every
v1 subject, procedure, gate, blob tier and media entry has a home or a stated
deletion, and the representative contracts use only draft-0 fields. The
exercise also found **24 gaps** (below). Five of them break something
ZenSight does today:

- template overlap (G1);
- dot-packed alert references against dotted service names (G4);
- no key-level selector for occurrence streams (G6);
- fleet populations reaching every state subscriber (G7);
- no per-replier completion for many-reply fan-out (G14).

**Draft 1 (#608).** #608 migrated the contracts to draft 1, and `zenkey-model`
now validates all twelve with no finding:
- `zs.snmp`'s `traps` and `zs.snmp_poller`'s `strays/{sender}` are
  `kind = "event"` (D3, G6), with v1's declared `burst(1000/h)`;
- `zs.sysinfo`'s `feature:nvml` gates are `build:nvml` (D4, G17);
- `zs.parallax`'s video is the raw family `video/*`, tied to `{codec}` by
  `media_param` (D6, G10), which replaces the `media.codec_param` annotation;
- FrameMeta is `attachment_encoding = "cbor"` (D5, G8).

The interim `media.v1` vocabulary now includes the keys this mapping needed
(`tier_param`, `control`, `receiver_report`). The self-check and the gaps
below are the draft-0 record, kept as written; r3.3 §0.3 says where each gap
went.

The rest are missing vocabulary or ergonomics.

The v1 surface, re-counted from
`zensight/zensight-common/registry/*.toml` (it matches the issue):

- 22 registries, 813 subjects (622 telemetry, 190 state, 1 events);
- 200 procedures, 25 errors, 74 `when` gates (7 on subjects, 67 on procedures);
- 30 `[[blob]]` tiers, 2 `[[media]]` entries, 20 `[[deprecated]]` paths.

## Shape

| zk2 concept | ZenSight |
|---|---|
| `system` | the host id, `h-<12hex>`, minted by `hostid.v1` (v1's origin derivation, unchanged) |
| host services | v1's producer names: `sysinfo`, `netlink`, `netring`, `systemd`, `logs`, `hostspec`, `parallax`, `probe`, `pve`, `historian`, plus the process identities `correlator`, `policy-compiler`, `exporter-prometheus`, `exporter-otel`. A second instance is just another service (`netring-2`) implementing the same interface. |
| device services | one per polled device: `snmp.<device>`, `modbus.<device>`, `gnmi.<device>`, `netflow.<exporter>`, `bmc.<chassis>`, `container.<name>`. Each poller keeps its own service (`snmp`, …) for what belongs to the process. The instance id is the device's continuity epoch. |
| fleet singletons | `zk2/fleet/catalog` (`zs.catalog.v1`) and `zk2/fleet/desired` (`zs.desired.v1`), single-writer under `redundancy.v1` with ZenSight's claim protocol as the reference. They replace v1's verbatim service origins `@catalog` and `@desired` ([`../shapes.md`](../shapes.md) row 4). |
| framework set | interfaces every service implements: `health.v1` ([`../walkthrough/health.v1.toml`](../walkthrough/health.v1.toml)), `alarms.v1`, `zs.evidence.v1`, `zs.thresholds.v1`, `views.v1`; also `zs.errors.v1` and `zs.artifacts.v1` (named, not written in this cut) |
| schema kind | **jsonschema**: ZenSight's types are serde structs on a CBOR wire. The shared telemetry point is `schemas/telemetry.v1.json`. |

Example keys, for host `h-3fa9c2d41b7e`:

```text
zk2/h-3fa9c2d41b7e/sysinfo/zs.sysinfo.v1/stream/memory/usage_percent
zk2/h-3fa9c2d41b7e/snmp.router01/zs.snmp.v1/stream/if/3/in_octets.rate
zk2/h-3fa9c2d41b7e/snmp.router01/zs.snmp.v1/stream/metrics/system/sys_uptime
zk2/h-3fa9c2d41b7e/snmp.router01/zs.snmp.v1/stream/traps/01jgxqz4yqk8v6txw3m9f2a7cd
zk2/h-3fa9c2d41b7e/netlink/alarms.v1/state/alarms/a659f813308ad1da
zk2/h-3fa9c2d41b7e/pve/zs.evidence.v1/state/relation/r-f5f9a2edb9601155
zk2/h-3fa9c2d41b7e/parallax/zs.parallax.v1/@stream/streams/cam0/video/h264/high
zk2/fleet/catalog/zs.catalog.v1/state/acks/h-3fa9c2d41b7e/netlink/a659f813308ad1da
zk2/fleet/desired/zs.desired.v1/state/desired/h-3fa9c2d41b7e/systemd/expectations.systemd
zk2/*/*/zs.historian.v1/@op/range                                   (fan-out, target All)
zk2/h-3fa9c2d41b7e/snmp.router01/@zk/alive/zs.snmp.v1/<instance>/<fp16>
```

## Files

| Contract | What | Resources |
|---|---|---|
| [`alarms.v1.toml`](alarms.v1.toml) | The generic alarm profile, with ZenSight's alert-key recipe byte-exact (RFC 11 §3.1, test vector `a659f813308ad1da`) | 1 |
| [`zs.evidence.v1.toml`](zs.evidence.v1.toml) | Identity evidence: `self`, `device/{device}`, `names/{ip}`, `relation/{relation_id}` | 4 |
| [`zs.thresholds.v1.toml`](zs.thresholds.v1.toml) | Threshold rules: state, `set`, `applied/{topic}`, the desired requirement | 3 |
| [`views.v1.toml`](views.v1.toml) | The ViewSet presentation document | 1 |
| [`zs.sysinfo.v1.toml`](zs.sysinfo.v1.toml) | Literal telemetry: all 136 v1 telemetry subjects, plus state and two operations | 140 |
| [`zs.snmp.v1.toml`](zs.snmp.v1.toml) | One polled device: rest-parameter metric tree, 126 typed columns, interface table, traps, gated outlet control | 133 |
| [`zs.snmp_poller.v1.toml`](zs.snmp_poller.v1.toml) | The poller: targets, discovery, traps from unconfigured senders | 4 |
| [`zs.netring.v1.toml`](zs.netring.v1.toml) | 75 telemetry streams, config topics, many-reply listings, captures via artifacts | 86 |
| [`zs.catalog.v1.toml`](zs.catalog.v1.toml) | Fusion outputs and operator intent, single writer, actor from O7 | 15 |
| [`zs.historian.v1.toml`](zs.historian.v1.toml) | `range`, `series`, `timeline` as fan-out, many-reply operations; `stats` | 4 |
| [`zs.parallax.v1.toml`](zs.parallax.v1.toml) | `@stream` video + `FrameMeta` attachment, `media.v1`, stream control | 24 |
| [`zs.desired.v1.toml`](zs.desired.v1.toml) | The desired author, keyed by target host and service | 8 |

The schemas are in [`schemas/`](schemas/): one file per contract, plus
`telemetry.v1.json` and `zs.common.json`, which several contracts list
unchanged so that a shared type keeps one identity (G12). The large telemetry
families of sysinfo, snmp, netring and parallax were transcribed from the v1
TOMLs by a throwaway script: one resource per subject, with the v1
description as `doc`, `when` mapped to `optional` + `gate`, and v1's
`cardinality`, `unit`, `kind` and `buckets` carried over.

**Self-check** (a throwaway script, not `zenkey-model`):

- every TOML and JSON file parses;
- every `json:` reference resolves in the `$defs` of a listed schema file, and every `$ref` resolves;
- no protobuf is referenced;
- every templated resource has `params` and a `cardinality`;
- rest and `{occurrence}` parameters come last; a `gate` only comes with `optional`;
- every annotation's profile is in `uses`; every field is a draft-0 field for its kind;
- no two templates of one kind token overlap.

## Mapping: all 22 registries

### The framework rows (once, for every registry that has them)

| v1 family | Registries | zk2 |
|---|---|---|
| state `health` | 20 | `health.v1` (`status`, `checks/{check}`, `faults`). ZenSight's HealthSnapshot fields (devices_total/responding, self_stats) need checks or a profile extension. |
| state `errors` (ZenSight's profile token) | 20 | `zs.errors.v1` state `errors`: the rolling error window (not written; one resource) |
| state `sensor` (registration document) | 20 | **deleted**: the instance descriptor (host, build, capabilities, profiles, r3.2 §3.10) |
| state `evidence/self` | 20 | `zs.evidence.v1` `self` |
| state `evidence/device/{device}`, `evidence/names/{ip_slug}`, `evidence/relation/{relation_id}` | 8, 1, 4 | `zs.evidence.v1` `device/{device}`, `names/{ip}`, `relation/{relation_id}` |
| state `alert/{alert_key}` | 20 | `alarms.v1` `alarms/{alert_key}` |
| state `applied/{topic}` | 15 | `zs.thresholds.v1` `applied/{topic}` |
| state `artifact/{kind}`, procedures `artifact/{request,cancel,status}`, `[[blob]]` artifact/tree/store | 10, 30, 30 | `zs.artifacts.v1`: a `jobs.v1` job whose Ready state carries a `blob.v1` reference (holder system, holder service, hash). Not written. |
| procedures `thresholds`, `thresholds/set` | 15 × 2 | `zs.thresholds.v1` state `thresholds`, `@op set` |
| procedure `views` | 8 | `views.v1` state `views` |
| procedures `introspect`, `describe` | 22 × 2 | **deleted**: presence + descriptor + contract bundles (§3.10, §3.11) |
| producer `alive` token | 20 + 2 service origins | the instance token, plus one interface token per exposed interface |
| `device/<d>/alive` tokens (snmp, modbus, container, parallax) | 4 | a device service's instance (the continuity epoch); parallax stays templated (G16) |

### Per registry

Counts are v1's: telemetry / state / events · procedures · errors · `when`.

| Registry | v1 counts | zk2 home | Notes |
|---|---|---|---|
| bmc | 10/14/0 · 5 · 1 · 0 | `bmc.<chassis>` services, `zs.bmc.v1` | `{chassis}/` leaves every template: streams `psu/{psu}/…`, `fan/{fan}/…`, `thermal/{sensor}/…`, `reachable`; state `chassis`, `psu/{psu}`, `fan/{fan}`, `thermal/{sensor}`, `drive/{drive}`, `memory/{dimm}`, `redundancy/{group}`. Its view's `group_by = "{chassis}"` becomes `group_by = "service"`. |
| catalog | 0/8/0 · 9 · 3 · 6 | `zk2/fleet/catalog`, **[`zs.catalog.v1`](zs.catalog.v1.toml)** | written; `ack/{alert_ref}` → `acks/{system}/{service}/{alert_key}` (G4) |
| container | 21/9/0 · 5 · 1 · 1 | `container.<name>` services, `zs.container.v1`; host aggregates on `container` | The 18 per-container streams lose `{name}/`; state `container/{name}` → `container`; `containers/{total,running,unhealthy}` stay on the host service. One gate (`config:upstream.enabled`) → optional. 512 per host is the device-as-service stress case (G16, U18). |
| correlator | 0/5/0 · 2 · 0 · 0 | host service `correlator` | framework only; the catalog it computes is `fleet/catalog` |
| desired | 0/21/0 · 3 · 1 · 1 | `zk2/fleet/desired`, **[`zs.desired.v1`](zs.desired.v1.toml)** | written; `{host}/<producer>/<topic>` → `desired/{host}/{service}/<document>`, one template per type (G5) |
| exporter-otel | 0/5/0 · 2 · 0 · 0 | host service `exporter-otel` | framework only |
| exporter-prometheus | 0/5/0 · 2 · 0 · 0 | host service `exporter-prometheus` | framework only |
| gnmi | 1/7/0 · 8 · 1 · 3 | `gnmi.<device>` services, `zs.gnmi.v1` | `{device}/{path...}` → `paths/{path...}`; the gNMI key-list collapse (`[name=eth0]` → `eth0`) is not slugging, so the original path stays a point label. Plus 3 blob tiers. |
| historian | 0/5/0 · 6 · 4 · 0 | host service `historian`, **[`zs.historian.v1`](zs.historian.v1.toml)** | written |
| hostspec | 1/6/0 · 7 · 1 · 0 | `zs.hostspec.v1` | stream `assertions/failing`; `expectations` state + `expectations/set` (desired `expectations.hostspec`); `spec` → state |
| logs | 23/8/0 · 13 · 1 · 5 | `zs.logs.v1` | streams `ingest/…`, `store/…`, `journald/…`, `by_severity/{severity}`, `by_unit/{unit}/…`, `by_template/{template}/…`. `events` + `events/page` → one many-reply op `events`, fan-out allowed. `filter` pair gated `config:enable_dynamic_filters`; `rules` pair + desired `rules.logs`; evidence/device 4096. 3 blob tiers. 18 retired paths do not migrate (G19). |
| modbus | 1/7/0 · 8 · 1 · 3 | `modbus.<device>` services, `zs.modbus.v1` | `{device}/{metric...}` → `metrics/{metric...}`; 3 blob tiers |
| netflow | 1/7/0 · 10 · 1 · 3 | `netflow.<exporter>` services, `zs.netflow.v1`; receiver service `netflow` | `{exporter}/{metric...}` → `metrics/{metric...}`; `flows` + `flows/page` → many-reply op on the receiver; exporters that push without being configured are G15's case. 3 blob tiers. |
| netlink | 106/9/0 · 23 · 2 · 7 | `zs.netlink.v1` | 106 streams (iface, sockets, routes, neighbors, addresses, conntrack, diagnostics, ethtool, tc, xfrm, nft, wireguard, events). `expectations` (+ desired `expectations.netlink`) and `collection` pairs; 12 many-reply listings. eBPF gates `["feature:ebpf", "config:collect.ebpf", "capability:CAP_BPF"]`. Error `no-route-socket` → `unavailable`, cause capability. 3 blob tiers. |
| netring | 75/9/0 · 30 · 1 · 22 | **[`zs.netring.v1`](zs.netring.v1.toml)** | written (3 of 15 listings spelled out) |
| parallax | 17/10/0 · 10 · 1 · 3 (+2 media) | **[`zs.parallax.v1`](zs.parallax.v1.toml)** | written |
| policy-compiler | 0/5/0 · 2 · 0 · 0 | host service `policy-compiler` | framework only; the documents it compiles are `fleet/desired` |
| probe | 21/8/0 · 7 · 1 · 0 | `zs.probe.v1` | `{target}/…` → `targets/{target}/…`; `duration_seconds` is v1's one histogram (`telemetry.kind = "histogram"` + `telemetry.buckets`); state `target/{target}` → `results/{target}`; `targets` pair (fan-out forbidden) + desired `targets.probe` |
| pve | 45/16/0 · 5 · 1 · 0 | `zs.pve.v1` (one service) | guests, storage, nodes and backups are facets read from one API, not devices with counters, so they stay templates (`guest/{vmid}/…`, `node/{node}/…`, `backup/job/{node}/…`, `ceph`, `cluster`) |
| snmp | 127/10/1 · 14 · 1 · 4 | `snmp.<device>` services, **[`zs.snmp.v1`](zs.snmp.v1.toml)**; poller `snmp`, **[`zs.snmp_poller.v1`](zs.snmp_poller.v1.toml)** | written |
| sysinfo | 136/9/0 · 9 · 1 · 7 | **[`zs.sysinfo.v1`](zs.sysinfo.v1.toml)** | written; 4 templates renamed to remove overlap (G1) |
| systemd | 37/7/0 · 20 · 2 · 9 | `zs.systemd.v1` | streams `unit/{unit}/…`, `manager/…`, `units/…`, `boot/{phase}`, `mounts/…`, `journal/…`, `events/{kind}`. `expectations` pair gated `["config:alerts.enabled", "capability:system-bus"]` + desired `expectations.systemd`; `action/set` gated `config:actions.enabled` + `capability:system-bus`; `action/capability` → state; `units`, `failed`, `timers`, `events`, `actions` → many-reply; `unit`, `unit/file` (gated), `cgroups` → operations with typed requests. Error `no-system-bus` → `unavailable`, cause capability. 3 blob tiers. |
| **total** | **622/190/1 · 200 · 25 · 74** | | + 30 blob tiers, 2 media entries, 20 retired paths |

### The 200 procedures, by family

| Family | Count | zk2 |
|---|---|---|
| `introspect` | 22 | deleted (descriptor + bundle) |
| `describe` | 22 | deleted (bundle schemas) |
| `views` | 8 | `views.v1` state |
| `thresholds` + `thresholds/set` | 30 | `zs.thresholds.v1` state + `@op set` |
| `artifact/{request,cancel,status}` | 30 | `zs.artifacts.v1` (jobs.v1 + blob.v1) |
| config pairs `<topic>` + `<topic>/set` (expectations, collection, detectors, capture_filter, threat_intel, capture_disk, filter, rules, targets, and the snmp/systemd `action` pairs) | 28 | the read becomes **state**, the write an `@op <topic>/set`, so a read needs no call grant (the tcgui pilot's split). Actions stay gated operations. |
| listings (`Vec<…>` / `Page<…>` replies) | 41 | `replies = "many"` operations (O6), with the selector parameters as a typed request |
| `action/capability` | 2 | state |
| other | 17 | catalog link/unlink/ack/unack/silence/unsilence (6); desired `override/set`; historian range/timeline/stats; hostspec `spec`; parallax `stream/set` → `control`, `stream/report` → `report`; sysinfo `latency`; systemd `unit`, `unit/file`, `cgroups` |
| **total** | **200** | |

### The other v1 constructs

| v1 | zk2 |
|---|---|
| class `telemetry` | `stream` typed `json:telemetry.v1.Point` |
| class `events` (1: snmp traps) | occurrence-keyed `stream` `traps/{occurrence}`, reliable, `retention` |
| `qos` profiles | explicit fields: `alert` → reliable, block, interactive_high, express; `transition` → reliable, block; `frame` → best_effort, drop, interactive_high; `sampled` → the zk2 stream defaults (priority `data`, not v1's `data_low`; G11) |
| `ttl_s` (every state subject) | `freshness.ttl_s`, with 0 meaning never (G18) |
| `when` (74) | `optional = true` + `gate` (the same ANDed list) |
| `common` tokens | the framework interfaces |
| `kind` (only 22 subjects: 16 gauge, 5 counter, 1 histogram), `unit`, `buckets`, `semantic` | `telemetry.kind`, `.unit`, `.buckets`, `.semantic` annotations; the ~600 unkinded subjects self-describe through the point's tag (r3.2 §4.9) |
| `cardinality` | `cardinality` (G2) |
| `rate` (the trap events; parallax's report procedure) | no field (G6) |
| `variant` | no field (G23) |
| `[[media]]` (2) | `explicit = true` streams with a `raw` type and a typed attachment |
| `[[blob]]` (30) | `blob.v1`'s `@blob` kind |
| `[[error]]` (25) | `serialize` (16) → core `internal`; `no-route-socket`, `no-system-bus` → `unavailable` cause capability; catalog `not-firing`/`publish`, desired `not-durable`, the historian's four → `app` error types (`CatalogError`, `DesiredError`, `HistorianError`) |
| `[[deprecated]]` (20) | not migrated: a new major starts clean (G19 is about the next retirements) |
| `seed`, the advanced tier | S4 state GET to the owner, and an `archive.v1` read for last-known (r4); `history = true` where v1 ran an AdvancedPublisher on state or evidence |
| `?actor=` selector parameter | O7 call metadata `{actor, request_id}`, recorded as `by` |

## Consumer side

One row per v1 fleet-wide selector, with where it is declared (`zensight/`
paths unless noted). "Introspection-driven" means the consumer discovers
interfaces from presence and contracts, then subscribes per interface.

| v1 selector (consumers) | zk2 |
|---|---|
| `v1/*/telemetry/**`: Prometheus exporter `subscriber.rs:178`, OTel exporter `subscriber.rs:122`, historian `ingest.rs:97`, GUI fallback `subscription.rs:519-563`, rerun | **Introspection-driven.** Liveliness `zk2/*/*/@zk/alive/**` → fetch each new interface's contract (`zk2/@zk/contract/<iface>/<sha256>`) → keep those whose contract `uses = ["telemetry.v1"]` → subscribe `zk2/*/*/<iface>/stream/**` per such interface, keeping the resources typed `telemetry.v1.Point` (which drops traps). `zk2/*/*/*/stream/**` is exact for "every ambient stream", but it mixes occurrence streams in (G6). Exporter metric names become `<interface>_<template literals>`. |
| GUI derived plan `v1/*/telemetry/<producer>/<path>` (`view/plan.rs:86-206`) | `zk2/*/*/<iface>/stream/<template with *>`. The interface chunk also fixes v1's miss of `netring-2`: the plan spells the producer's base name. |
| `v1/*/state/**` (GUI `subscription.rs:146`) | `zk2/*/*/*/state/**`, exact, but it now also carries the fleet catalog and desired populations (G7) |
| `v1/*/events/**` (GUI `subscription.rs:160`, OTel `subscriber.rs:180`, historian `timeline.rs:53`, storage `configs/router-events-storage.json5:95`) | **No exact selector** (G6). Per interface: `zk2/*/*/zs.snmp.v1/stream/traps/*` and `zk2/*/*/zs.snmp_poller.v1/stream/strays/**`. |
| `v1/*/state/*/alert/*` (GUI `subscription.rs:359`, Prometheus `subscriber.rs:186`, OTel `subscriber.rs:158`, historian `timeline.rs:123`, correlator `subscriber.rs:84`, zenwatch `rules.rs:244`) | `zk2/*/*/alarms.v1/state/alarms/*`, exact, one major per subscription |
| `v1/*/state/*/alive` liveliness (GUI `subscription.rs:281`, Prometheus `:235`, OTel `:204`, correlator `:122`, zenwatch `engine.rs:610`, `rules.rs:245`) | every instance: `zk2/*/*/@zk/instance/*`. "Is the owner of these alarms present" (S6): `zk2/*/*/@zk/alive/alarms.v1/**`. |
| `v1/*/state/*/device/*/alive` (GUI `subscription.rs:333`; hand-spelled at `zensight-common/src/keyexpr.rs:875`) | `zk2/*/*/@zk/alive/zs.snmp.v1/**`, and the same for zs.modbus.v1, zs.gnmi.v1, zs.netflow.v1, zs.bmc.v1, zs.container.v1. An instance change is the counter discontinuity. |
| `v1/*/state/*/evidence/**` + `…/evidence/names/*` (correlator `subscriber.rs:45`, `:55`; hand-spelled `keyexpr.rs:373`) | `zk2/*/*/zs.evidence.v1/state/**` and `…/state/names/*`. The correlator's two overlapping subscriptions collapse into one. Storage `router-evidence-storage.json5` → the same selector. |
| `v1/*/state/*/health` (rerun; `keyexpr.rs:336`) | `zk2/*/*/health.v1/state/**` |
| `v1/*/@rpc/*/introspect` + `v1/@catalog/@rpc/introspect` (GUI `app.rs:8446`, `:8451`) | liveliness `zk2/*/*/@zk/instance/*`, then a descriptor GET (on `zk2/*/*/@zk/instance/*`), then contracts by sha256. The catalog needs no special case, because `fleet` is a plain system. |
| `v1/*/@rpc/<p>/describe` (GUI `app.rs:6602`) | deleted: the schemas are in the bundles |
| `v1/*/@rpc/*/views` (GUI `app.rs:8455`) | GET `zk2/*/*/views.v1/state/views` |
| `v1/@catalog/state/{entity,edge,incident,ack,silence,alias,assertion}/*` (GUI `subscription.rs:220-475`, Prometheus `:203-217`, OTel `:129`, desired `main.rs:361`, `fleet.rs:38`; zenwatch `discipline/inhibit.rs:31-33`) | `zk2/fleet/catalog/zs.catalog.v1/state/{entities,edges,incidents,silences,aliases,assertions}/*` and `…/state/acks/*/*/*` |
| `v1/@catalog/state/alive`, `v1/@desired/state/alive` (GUI `subscription.rs:311`) | `zk2/fleet/catalog/@zk/alive/zs.catalog.v1/**`, `zk2/fleet/desired/@zk/alive/zs.desired.v1/**` |
| `v1/@catalog/state/claim/*`, `v1/@desired/state/claim/*` (`service_guard.rs:228`; hand-built `keyexpr.rs:724-745`) | `zk2/fleet/catalog/@zk/instance/*`, `zk2/fleet/desired/@zk/instance/*`: the instance tokens are the claim set |
| `v1/*/@rpc/historian/range`, `…/timeline` (GUI `app.rs:8047`, `:1972`) | `zk2/*/*/zs.historian.v1/@op/range`, `…/timeline` (target All, consolidation None) |
| `v1/*/@rpc/logs/events/page`, netring/netlink listings, the generic `v1/*/@rpc/<p>/<proc>` fallback (`call.rs:455-470`, `view/specialized/netring_detail.rs:86-91`, `bandwidth.rs:157`) | `zk2/*/*/<iface>/@op/<op>`, legal only where `fanout = "allowed"` is declared (O2). v1 reads defaulted to fan-out allowed; draft 0 defaults to forbidden, so every fleet-called read declares it. |
| `v1/*/@rpc/<p>/artifact/request` and `…/cancel` fleet writes (`view/artifact_fetch.rs:516`, `app.rs:4048`) | `zk2/*/*/zs.artifacts.v1/@op/request`, `…/cancel` (fan-out allowed, as v1 declared) |
| `v1/@desired/state/**` seed (desired `publish.rs:62`, hand-built) | `zk2/fleet/desired/zs.desired.v1/state/**`. The sensor side reads its exact key, binding `{host}`/`{service}` to itself. |
| `v1/@catalog/state/pdns/**` storage (`router-pdns-influxdb-storage.json5:76`) | `zk2/fleet/catalog/zs.catalog.v1/state/pdns/**`; under r4 the history capture is an archive, recording, never answering on the catalog's keys |
| web `v1/*/state/parallax/alive` (`web/src/origins.ts:28`, `keys.ts:68-74`) | `zk2/*/*/@zk/alive/zs.parallax.v1/**` |
| web `v1/<origin>/state/parallax/stream/*` (`catalogue.ts:45`) | `zk2/<host>/*/zs.parallax.v1/state/streams/*` |
| zenwatch's own state (`zenwatch/src/publish.rs:557-565`: health, `firing/{rule}`, describe) | `health.v1` + a zenwatch interface of its own; describe is deleted |

## Migration debt

**Identities inside payloads** (r3.2 §3.11 wants structured fields, never
key strings):

- full keys and key prefixes:
  - `Delivery::Blob.blob_prefix`, `Delivery::Tree.store_prefix`/`tree_prefix` (`zensight-common/src/artifact.rs:210, 226, 228`) and `CaptureRecord.artifact_prefix` (`query_detail.rs:292`). `blob.v1` references carry (holder system, holder service, hash). The GUI re-splits the prefix by position (`view/artifact_fetch.rs:759-761`).
  - `TimelineEntry.key` (`history.rs:308`). The **persisted** timeline uid is a hash of that key string (`zensight-store/src/timeline.rs:74-76`), so every stored uid and cursor changes.
  - `RangeReply.next_cursor` = `"<n>:<origin>/<producer>/<subject>"` (`zensight-historian/src/query/range.rs:187`).
  - The historian's persisted series key `series_path()` = `"<origin>/<producer>/<subject>"` (`zensight-store/src/lib.rs:777`), and `device_prefix()` (`:782`).
- alarm identity:
  - `AlertRef` is one dot-packed string (`alert.rs:317-409`), inside `AlertAck.alert_ref` and `Incident.alerts`.
  - `AlertSite` has no producer (`impact.rs:77-81`).
  - AlertRefs are built with `format!` in the correlator (`subscriber.rs:431`), the GUI (`view/alerts.rs:364`) and the Prometheus exporter (`alerts.rs:142`). The last two use `alert.protocol`, which loses the instance suffix.
  - `Alert.source`/`.protocol` (`alert.rs:132-135`).
- origins and producers:
  - `HostEntity.origins` (`entity.rs:207`), `MemberClaim.(sensor, source)`, `Incident.origins` (`incident.rs:50`), `Edge.observers[].origin` (`relation.rs:310`).
  - `OperatorAssertion.old`/`new`/`id` (`"{kind}-{old}-{new}"`, `entity.rs:117`).
  - Silence matchers on `origin`/`producer`/`source`, possibly as regexes over origins (`silence.rs:56-117`).
  - `HostEvidence.(sensor, source, host_id)`, `NameObservation.observer`, `EndpointClaim.host_id`.
  - `HealthSnapshot.(host_id, source)`, and `SensorInfo.(producer, source, capabilities keyed by device chunk)`.
  - `DesiredOverride.(host, producer, topic)` (`desired.rs:609-611`), and the controller's overrides and policy files keyed by host and `"producer/topic"` (`zensight-desired/src/overrides.rs:39`, `compile.rs:111`, `policy.rs:245`).
- subject paths:
  - `TelemetryPoint.source`/`.metric` (`telemetry.rs:20, 29`), dropped from `telemetry.v1.Point`.
  - `ThresholdRule.metric` globs and the `source` label convention (`threshold.rs:59-96`), and the netlink expectations' `metric` fields. Every v1 rule set has to be rewritten, because the paths change (the device moves to the service, rest families move under `metrics/`, four sysinfo and the netring per-source templates are renamed).
  - `Panel.scope`/`join` as v1 subject patterns (`views.rs:146-151`).
  - The stream chunks repeated in `StreamControl`, `StreamStatus` and `MediaReceiverReport`.
  - Fields that repeat a template parameter (`AppliedConfig.topic`, `GpuInfo.card`, `InterfaceTable.device`, `OutletStatus.device`): the tcgui pilot's gap 1.
- selectors in configuration:
  - historian, exporter and rerun `key_expr`; the GUI's persisted `subscription_scope` (`view/settings.rs:94`);
  - the router storage configs: `router-evidence-storage.json5:56-83`, `router-events-storage.json5:95`, `router-blob-storage.json5:51-57`, `router-pdns-influxdb-storage.json5:76-77`.

**Hand-built keys.** There are 23 production sites:

- `zensight-common/src/keyexpr.rs:206, 373, 625, 724, 730, 740, 745, 799, 807, 815, 823, 831, 844, 875, 1019`;
- `served.rs:732`, `subscribe.rs:35, 128`;
- both exporters' `subscriber.rs:17`;
- `zensight-desired/src/publish.rs:62` and `lib.rs:55`;
- `zensight-sensor-core/src/alert.rs:445`.

On top of those:

- the GUI's `view/plan.rs:114-201` and `detection_tuning.rs:67`;
- entity ids minted in origin shape (`zensight-correlator/src/merge.rs:745, 765`);
- literal service-origin comparisons (`subscription.rs:873, 885`, `view/explorer/core.rs:164`);
- selector parameters appended with `format!` (`app.rs:1692, 4449, 4773, 7645-7681`, `call.rs:469`, `netring_detail.rs:125-157`);
- positional key parsers: desired `publish.rs:215`, `audit.rs:466`, `advanced_publisher.rs:460`, `artifact_fetch.rs:759`, `service_guard.rs:238`, `subscription.rs:874-922`, `zensight-store/src/lib.rs:633`;
- literal subject-chunk arrays handed to `V1Context` by the bmc, container, probe, pve, snmp, parallax and sysinfo pollers.

Tests spell many more keys; bmc's e2e alone spells 61.

**The web client spells its keys by hand:**

- `web/src/keys.ts:10, 12, 44, 49, 57, 69, 74` (a regex parse), `80, 94, 99`;
- `report.ts:13`, which bypasses `keys.ts`;
- `main.ts:334, 347`, which repeat the media keys;
- `types.gen.ts`, generated from `web/schemas/*.json`.

**zenoh-ext.** All of it is on plain kinds, which zk2 allows.

- AdvancedPublisher `default()` runs on **telemetry**:
  - Configuration: cache 10 + 5 s heartbeat + publisher detection (`zensight-sensor-core/src/advanced_publisher.rs:63-71`).
  - Users: netlink (`collector.rs:194`, which also publishes neighbour evidence through that telemetry-QoS registry, `:711`) and netring (`publish.rs:72`).
- `cache_only(1)` runs on **evidence** everywhere (`runner.rs:739`, `relation.rs:107`, snmp, bmc, container, pve, parallax, logs, netring), and on **state**: snmp interfaces/discovery, bmc chassis, probe targets, container, pve, parallax discovery.
- AdvancedSubscribers:
  - on telemetry: historian, exporters, GUI;
  - on events and alerts: historian `timeline.rs:54, 125`;
  - on evidence, assertions, alerts, acks and silences: correlator `subscriber.rs:47-114`. The catalog documents there are published by plain publishers, so their history reads nothing.
  - on **desired**, sensor side only (`zensight-sensor-core/src/desired.rs:274-279`).
- The contracts declare `history = true` on evidence and on that state, and nowhere on telemetry (RFC 11 §1's target posture) or desired (S4 + an archive, r4). Depth and heartbeat cannot be declared (G24).

**Other:**

- 18 types v1 declares without a schema (`zensight-common/src/schema.rs:213-232`: Ack, ArtifactAck, TopicStatus, TopicConfig, DetectorConfig, ThreatIntelConfig, FilterConfig, CollectionConfig, CaptureDisk*, ExpectationCommand, RulesStatus, seven `Vec<…Record>`). Each needs a schema, or an honest `{ raw = "application/json" }`; `zs.netring.v1` shows both.
- The exporters' `device="…"` label becomes the `service` label (dashboards break).
- v1 reads defaulted to fan-out allowed; draft 0 defaults to forbidden.

## Decisions recorded here

- **Device-as-service for pollers with per-device counters** (snmp, modbus, gnmi, netflow, bmc, container). **Templates for facets** read from one API (pve guests, probe targets, parallax streams). A typed column and the rest-parameter catch-all never share keys: a metric goes to exactly one of `if/{index}/…` and `metrics/{metric...}`.
- **Reads become state where they return a document** (`thresholds`, `targets`, the config topics, `action/capability`, the snmp `action` read). Listings become many-reply operations.
- **Write operations do not repeat template parameters** (tcgui gap 1). The snmp outlet action becomes `outlets/{outlet}/cycle`, with fan-out now forbidden: the device is in the key.
- **`alarms.v1` carries no producer fields.** ZenSight's `kind` rides an unhashed `category`, so it cannot change the key.
- **The catalog's identity-shaped fields are renamed** (`origins` → `systems`, `MemberClaim.sensor` → `service`, AlertRef structured, `AlertSite` gains `service`). The other schemas keep v1's fields and flag the duplicates as debt.
- **Occurrence retention is set to 7 days**; v1's events storage had no bound.

## Gaps found

Each gap: what it is · the evidence · the proposed correction.

1. **G1. Template overlap has no rule.** Draft 0 makes template *strings*
   unique, but says nothing when two templates match one concrete key.
   - *Evidence:* v1 has 187 same-class overlapping pairs. sysinfo has 56 (`sysinfo.toml:171/189/204`, `cpu/times/user` matches three templates). snmp has 129 (`snmp.toml:15`, the catch-all against every typed column). netring has 1 (`netring.toml:124` vs `:236`) and container 1. v1 resolves them by most-literal-first precedence (`rfcs/08-registry.md:33`). Here, four sysinfo templates and the netring per-source family were renamed to get a clean check.
   - *Correction:* adopt v1's precedence (literal beats `{x}` beats `{x...}`, position by position) in draft 0 and the model's parser, and lint overlapping templates whose types differ. Or forbid overlap outright; ZenSight can live with either.
2. **G2. Cardinality is one number per contract, but bounds belong to implementations.**
   - *Evidence:* alarms run 32 (correlator) to 256 (bmc); `evidence/device` runs 64 (snmp) to 4096 (logs, netring). Device services re-base every v1 poller-wide bound (`zs.snmp.v1` keeps v1's).
   - *Correction:* the contract value is the interface-wide ceiling. The descriptor MAY declare a lower bound per instance and resource, and conformance checks the descriptor's.
3. **G3. A profile's per-implementer parameters have no home.**
   - *Evidence:* the alarm-key recipe excludes "the label named `host`, *and any label the producer documents as host-scoped*" (RFC 11 §3.1). ZenSight's set is `host.*` (`alert.rs:285-305`). A standard contract cannot carry implementer annotations, and the descriptor has no slot.
   - *Correction:* `alarms.v1` fixes the excluded set normatively (`host` and `host.*`), as `alarms.v1.toml` does here. More generally, the descriptor's `profiles` entries may carry parameters.
4. **G4. Dot-packed references break on dotted service names, and device-service names have no derivation.**
   - *Evidence:* AlertRef `<origin>.<producer>.<alert_key>` splits on the first two dots (`alert.rs:378-408`, RFC 11 §3.2), so `h-….snmp.router01.<key>` parses wrong. Target names allow `.` and capitals (`zensight-common/src/targets.rs:179`). r3.2 §3.5 and `../shapes.md` bless `snmp.router01` without saying how a device name becomes a chunk.
   - *Correction:* r3.x states the derivation `<parent>.<slug(device)>` (the parent dot-free, split on the first dot, §3.1's slug rule). Payload references become structured fields keyed by multi-parameter templates (`acks/{system}/{service}/{alert_key}`), never dot-packed.
5. **G5. Heterogeneous documents per target.**
   - *Evidence:* `@desired`'s `{host}/<producer>/<topic>` carries 7 types over 21 subjects (`desired.toml`, `desired.rs` `topics()`). A template has one type, and the target service is a deployment fact. R2's `{x} = self` cannot say whether `self` is the system or the service when a template has both. The walkthrough's `desired.target_param` names one parameter.
   - *Correction:* `desired.v1` writes the pattern down: one template per document type, ending in a literal type chunk (`thresholds`, `expectations.systemd`). R2 spells `self.system` / `self.service`, and `desired.target_param` accepts a list.
6. **G6. Occurrence streams have no key-level selector, and no rate budget.**
   - *Evidence:* the events storage selects `v1/*/events/**` (`configs/router-events-storage.json5:95`), and the GUI, OTel and the historian subscribe it. In zk2, occurrence streams share the `stream` token with telemetry, so the union storage r3.2 §3.2 relies on has to enumerate every (interface, template). v1's `rate = "burst(1000/h)"` (snmp traps) and the parallax report procedure's rate have no field. `cardinality` on an `{occurrence}` template has no defined meaning.
   - *Correction:* give occurrence streams their own plain kind token (`events`), so that `zk2/*/*/*/events/**` is exact, as in v1. Add a `rate` field (streams and operations). Define the occurrence cardinality as rate × retention.
7. **G7. Fleet populations reach every fleet-wide state subscriber.**
   - *Evidence:* v1 kept `@catalog` and `@desired` off `v1/*/state/**` with verbatim origins (`entity.rs:44-48`, "D4"). The GUI subscribes the whole state plane (`subscription.rs:146`). On zk2, `zk2/*/*/*/state/**` would carry pdns (100k), edges (50k) and desired documents (≈210k).
   - *Correction:* allow `explicit = true` on state (an `@state` token) for population-budgeted resources meant for named consumers, or a verbatim system token for singletons.
8. **G8. Attachment encoding when the payload is raw.**
   - *Evidence:* r3.2 §3.2 says an attachment is "encoded like the payload", which is undefined for a `raw` payload. FrameMeta is canonical CBOR, pinned byte for byte by a corpus shared with another repository (`stream.rs:97-131`).
   - *Correction:* an attachment declares its own encoding.
9. **G9. The wire encoding of a jsonschema type is unspecified.**
   - *Evidence:* ZenSight payloads are CBOR, with JSON accepted by sniffing the first byte (RFC 11 §1; `zensight-common/src/config.rs:524` makes it configurable). §8 rules out several encodings per resource, and §3.9 only says samples carry the Zenoh `Encoding` id. serde's `Vec<u8>` is an array of numbers in both encodings.
   - *Correction:* a jsonschema resource fixes its encoding (`cbor` | `json`) in the contract; the data model is JSON's; bytes have a stated rule.
10. **G10. Raw type families.**
    - *Evidence:* parallax's `[[media]] encoding = "video/*"`, with the codec as a key chunk (`parallax.toml:349-360`). Draft 0 shows only concrete media types.
    - *Correction:* allow `{ raw = "video/*" }`, the subtype carried by the sample's `Encoding`, optionally tied to a template parameter.
11. **G11. No interface-level annotations or defaults.**
    - *Evidence:* `redundancy.v1` governs the whole catalog interface but can only be listed in `uses`. 136 sysinfo streams would repeat v1's `sampled` priority, and every state resource repeats `freshness.ttl_s` (tcgui gap 2).
    - *Correction:* `[interface] annotations` as defaults, plus `[defaults.stream]` / `[defaults.state]` QoS, both expanded into the canonical form.
12. **G12. Shared and profile types across contracts.**
    - *Evidence:* a type's identity is the sha256 of the schema artifact holding it (§3.9), so `telemetry.v1.Point` and `ThresholdsConfig` stay one type only if every contract lists a byte-identical file. `json:` references are unqualified, so two listed files defining one name collide. A `$ref` across listed files (`zs.desired.v1.json` → `zs.common.json`) is unspecified. So are `prefixItems` (the historian's `(ts, value)` tuples) and an open schema (`DesiredOverride.doc`) in the zk2 subset.
    - *Correction:* a profile-type reference form (`"telemetry.v1:Point"`) resolved against the profile's published schema with a pinned sha256; a rule that a `json:` name is unique per contract; `$ref` allowed to listed files only; S7 to settle tuples and open schemas.
13. **G13. The schema kind of `telemetry.v1.Point`.**
    - *Evidence:* r3.2 §4.9 says "a protobuf oneof". ZenSight's wire is the CBOR of serde's internally tagged `{"type", "value"}` (`telemetry.rs:105-134`, RFC 11 §4). A protobuf Point re-encodes every telemetry sample in the fleet.
    - *Correction:* `telemetry.v1` defines Point in jsonschema with the tagged shape as normative, and optionally a protobuf twin with a normative mapping.
14. **G14. A many-reply fan-out cannot tell "finished" from "timed out" per replier.**
    - *Evidence:* O2 + O6: Zenoh completes the whole query, not each replier. The historian exists to be honest about short answers (`partial`, `covers_from`, `next_cursor`, `history.rs:185-240`), and the GUI merges per series across historians.
    - *Correction:* O6 adds a terminal per-replier marker (a final reply flag in the core envelope or attachment), or requires `partial` on every reply of a fan-out, many-reply operation.
15. **G15. Data about devices a process does not serve.**
    - *Evidence:* traps are keyed by the slugged sender IP (`zensight-sensor-snmp/src/trap.rs:298-299`), and netflow exporters push unannounced.
    - *Correction:* r3.x states that device services come from configuration only, and the parent service keeps unmatched devices (`strays/{sender}/…`, as in `zs.snmp_poller.v1`).
16. **G16. No continuity epoch per member of a templated population.**
    - *Evidence:* v1 cycles a device token per device for snmp, modbus, container and parallax streams (`docs/KEYSPACE.md:47-53`, RFC 08 v1.39). In zk2 the epoch is the whole service's instance. Containers reach 512 per host (`container.toml`).
    - *Correction:* promote U18's fallback to a core option: `epoch = "<param>"` on a template, with a per-member token under `@zk`. Or size S2 for 512 services per host.
17. **G17. The gate vocabulary does not line up.**
    - *Evidence:* draft 0's `feature:` against r3.2's cause `build`. v1 names such as `capability:CAP_BPF` (netlink) and dotted config keys.
    - *Correction:* align the spellings (feature ↔ build) and define the name charset (v1: free text without whitespace).
18. **G18. `freshness.v1` is missing from r3.2 §3.12, and state expiry has no field.**
    - *Evidence:* the walkthrough and tcgui use `freshness.ttl_s`, which §3.12 does not list. v1's `ttl_s` on all 190 state subjects mixes a refresh cadence (900) with real expiry: pdns 86400, alias 31536000, ack/silence/desired 0 = never.
    - *Correction:* list `freshness.v1` with `ttl_s = 0` meaning never.
19. **G19. No deprecation marker.**
    - *Evidence:* ZenSight retired 20 paths during v1 (logs 18, sysinfo 2). Within a zk2 major a retirement can only be optional-and-never-exposed.
    - *Correction:* a documentation-only `deprecated = "<reason>"` field, and a lint that a deprecated resource is optional.
20. **G20. A view belongs to an interface, not to an instance.**
    - *Evidence:* views are bundled per registry, static per build, and served by every producer (`views.rs:19-24`). Scopes span several interfaces of one service.
    - *Correction:* bundles may carry presentation documents excluded from the fingerprint. Until then, `views.v1` is served as state, scoped by (interface, template).
21. **G21. Profile annotation vocabularies are unwritten.**
    - *Evidence:* this mapping invents `telemetry.kind/unit/buckets/semantic`, `media.codec_param/tier_param/frame_clock/control/receiver_report` and the list form of `desired.target_param`. The draft-0 example's `freshness.v1` is G18.
    - *Correction:* each profile ships its annotation table before #608 lints `uses`.
22. **G22. S5 and U10 have to cover ZenSight's sizes.**
    - *Evidence:* ZenSight seeds 10k entities and 50k edges by wildcard GET (`subscription.rs:388, 472`), and pdns reaches 100k.
    - *Correction:* S5's wildcard-GET matrix adds 50k and 100k. If U10's "collections become streams or operations" wins, the catalog's seed path changes.
23. **G23. Minor format gaps.**
    - *Evidence:* no codegen name hint (v1 `variant`, used by sysinfo and netlink to avoid generated-name collisions), and no `doc` on a `[requires.<role>]`.
    - *Correction:* both are optional, documentation-only fields.
24. **G24. Advanced pub/sub parameters cannot be declared.**
    - *Evidence:* v1 runs two configurations: cache 10 + 5 s heartbeat + publisher detection on telemetry, and cache 1 on evidence and state (`advanced_publisher.rs:63-83`). `history = true` is one bit.
    - *Correction:* `history = { depth = n, miss_detection = "<period>" }`, fingerprinted, or a profile.

## What fits r3.2 as written

- **The claim protocol fits core presence** (`zs.catalog.v1` header):
  - the instance tokens are the claim set;
  - the interface token marks the incumbent, and a standby has none (§3.8, §3.10);
  - election and `alive` stay two steps.
- **Rest parameters** fit snmp, modbus, gnmi and netflow; `{occurrence}` fits traps.
- **`cardinality`** expresses every v1 bound, including the 100k pdns.
- **O7 call metadata** replaces `?actor=`.
- **Selecting by interface chunk** removes v1's instance-suffix bugs (the GUI plan, the web client's parallax regex).
- **`explicit = true`** keeps video off ambient selectors, as `@media` did.
- **The fleet singletons** need no special casing anywhere a consumer looks.
