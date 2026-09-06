# 11 — Reference Application Profile: ZenSight

**Status: v1.0 (ratified)** · informative chapter · *amended in v1.25, v1.26, v1.29, v1.30, v1.31 and v1.32 — see [CHANGELOG.md](CHANGELOG.md)*

> **Registry location note (2026-07).** The registry *data* this profile
> describes (`registry/*.toml` for the ten producers and `@catalog`, plus
> the deprecation ledger) lives in the
> [ZenSight repository](https://github.com/p13marc/zensight) and is
> compiled there via `zenkey-build`. The `zenkey` crate no longer bundles
> it; a snapshot remains in this repo only as the codegen regression
> corpus (`fixture-tests/registry/`).

The convention chapters (02–10) are application-neutral. This chapter binds
them to ZenSight: the concrete base, producers, service origins, and a
conceptual mapping from every shipped key family to its home under the
convention. It is a *profile*, not a migration plan — sequencing,
coexistence, and code changes are explicitly out of scope
([01-motivation.md §5](01-motivation.md)).

---

## 1. Profile constants

| Convention slot | ZenSight binding |
|---|---|
| `<base>` | **empty by default** (*v1.6* — the base-less bus-root deployment; no session `namespace` is set). A deployment opts into isolation by setting the base (`zenoh.namespace` in the shared config block / `ZENSIGHT_ZENOH_NAMESPACE`); `zensight` is the conventional example. Either way no crate ever concatenates it ([03-grammar.md §1.1](03-grammar.md)) |
| version | `v1` (plain, not verbatim — [03-grammar.md §1.2](03-grammar.md)) |
| host origin | `h-<12hex>` from `sha256(machine-id + app salt)` — the same value the correlator uses today as `host_id`/`entity_id` (currently spelled `h_<12hex>`; the profile normalizes the separator to `-`) |
| service origins | `@catalog` (implemented by `zensight-correlator`) |
| producers | `snmp`, `logs`, `netflow`, `modbus`, `sysinfo`, `gnmi`, `netlink`, `netring`, `systemd`, `parallax` (+ `-<instance>` when doubled on one host) |
| payload default | CBOR, first-byte sniff for JSON interop (unchanged) |

Proxy producers (`snmp`, `modbus`, `gnmi`, `netflow`) put the observed
device as the first subject chunk; host-local producers (`sysinfo`,
`netlink`, `netring`, `systemd`, `logs`, `parallax`) start the subject at
the metric directly — the origin already names the host, so the incumbent
`<source>` hostname chunk disappears for them.

**Delivery tier, honestly.** The shipped sensors currently run the
advanced tier per-key across the *whole telemetry fan* (cache 10 +
heartbeat 5 s + publisher detection on every telemetry key) — a default
that predates the convention's cost analysis and lands squarely in
[04-planes.md §3.3](04-planes.md)'s "what the tier is NOT for" (wide fans:
4 entities and 2 network-wide declarations per key, heartbeat load
proportional to key count). Under this profile the target posture is the
baseline ([04 §3.2](04-planes.md)) for telemetry — the deployment already
runs the storages — with the tier retained only where its decision rule
holds: alerts and expectation/config echoes (`detect_s ≪ ttl_s`,
`sporadic_heartbeat`), and evidence documents keep their shipped
cache-only depth 1.

## 2. Worked examples per sensor

```
# sysinfo (host-local)
zensight/v1/h-3fa9c2d41b7e/telemetry/sysinfo/cpu/usage
zensight/v1/h-3fa9c2d41b7e/telemetry/sysinfo/memory/usage_percent
zensight/v1/h-3fa9c2d41b7e/@rpc/sysinfo/processes?sort=cpu;top=20

# snmp (proxy: device first subject chunk)
zensight/v1/h-3fa9c2d41b7e/telemetry/snmp/router01/system/sys_uptime
zensight/v1/h-3fa9c2d41b7e/state/snmp/device/router01/liveness
zensight/v1/h-3fa9c2d41b7e/state/snmp/device/router01/alive          (liveliness token)

# netlink
zensight/v1/h-3fa9c2d41b7e/telemetry/netlink/sockets/tcp/established
zensight/v1/h-3fa9c2d41b7e/state/netlink/alert/9f2c81ab04d7e3f1
zensight/v1/h-3fa9c2d41b7e/@rpc/netlink/sockets?ip=10.0.0.7
zensight/v1/h-3fa9c2d41b7e/@rpc/netlink/expectations/set

# netring
zensight/v1/h-3fa9c2d41b7e/telemetry/netring/flow/red/p95_ms
zensight/v1/h-3fa9c2d41b7e/state/netring/evidence/names/10-0-0-7
zensight/v1/h-3fa9c2d41b7e/events/netring/capture/01jgxqz4yqk8v6txw3m9f2a7cd
zensight/v1/h-3fa9c2d41b7e/@rpc/netring/capture/trigger

# netflow (proxy; REDESIGNED, not migrated as-is — see §3)
zensight/v1/h-3fa9c2d41b7e/telemetry/netflow/exporter01/flows_per_second
zensight/v1/h-3fa9c2d41b7e/telemetry/netflow/exporter01/top/talkers/1
zensight/v1/h-3fa9c2d41b7e/@rpc/netflow/flows?src=10.0.0.1;dst=10.0.0.2;max=500

# modbus (proxy)
zensight/v1/h-3fa9c2d41b7e/telemetry/modbus/plc01/holding/40001
zensight/v1/h-3fa9c2d41b7e/state/modbus/device/plc01/liveness

# gnmi (proxy; open-depth subject via the {path...} rest-variable, 08 §2;
# shipped bracketed path-elements are slugged per 03 §2:
# interfaces/interface[name=eth0]/state → interfaces/interface/eth0/state —
# the gNMI key list collapses into a chunk, original path in the payload)
zensight/v1/h-3fa9c2d41b7e/telemetry/gnmi/router01/interfaces/interface/eth0/state/counters/in_octets

# systemd
zensight/v1/h-3fa9c2d41b7e/telemetry/systemd/unit/sshd.service/active
zensight/v1/h-3fa9c2d41b7e/@rpc/systemd/action                       (gated write)

# logs (per-line detail is pull-only, P9)
zensight/v1/h-3fa9c2d41b7e/telemetry/logs/by_severity/error
zensight/v1/h-3fa9c2d41b7e/@rpc/logs/events?since=1720000000000;max=500

# historian (host-origin; a computed history application, 04 §4 — see below)
zensight/v1/h-3fa9c2d41b7e/@rpc/historian/range?series=h-9d02aa17c44f/sysinfo/cpu/usage;from=…;to=…;step=60
zensight/v1/h-3fa9c2d41b7e/@rpc/historian/series?prefix=sysinfo
zensight/v1/h-3fa9c2d41b7e/@rpc/historian/timeline?after_uid=01jgxqz4yqk8v6txw3m9f2a7cd;max=200
zensight/v1/h-3fa9c2d41b7e/@rpc/historian/stats
zensight/v1/h-3fa9c2d41b7e/state/historian/health                    (framework state only)

# parallax (media)
zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/video/h264/high
zensight/v1/h-3fa9c2d41b7e/@media/parallax/cam0/preview/jpeg
zensight/v1/h-3fa9c2d41b7e/state/parallax/stream/cam0                (catalogue+status doc)
zensight/v1/h-3fa9c2d41b7e/telemetry/parallax/cam0/stats/fps

# framework (every sensor, via sensor-core)
zensight/v1/h-3fa9c2d41b7e/state/<producer>/health
zensight/v1/h-3fa9c2d41b7e/state/<producer>/errors
zensight/v1/h-3fa9c2d41b7e/state/<producer>/sensor                   (registration doc)
zensight/v1/h-3fa9c2d41b7e/state/<producer>/alive                    (liveliness token)
zensight/v1/h-3fa9c2d41b7e/state/<producer>/evidence/self

# catalog (zensight-correlator)
zensight/v1/@catalog/state/entity/h-3fa9c2d41b7e
zensight/v1/@catalog/state/alias/h-9d02aa17c44f
zensight/v1/@catalog/state/pdns/93-184-216-34
zensight/v1/@catalog/state/edge/e-2879d4667f9d946d                   (§3.3)
zensight/v1/@catalog/@rpc/names?ip=93.184.216.34

# relationship claims (pve, container, probe, netlink — the inputs to the above)
zensight/v1/h-3fa9c2d41b7e/state/pve/evidence/relation/r-f5f9a2edb9601155
```

**The historian, stated as what shipped (v1.31).** Telemetry history in
this application is a **computed answer**, not a storage: `zensight-historian`
subscribes `zensight/v1/*/telemetry/**` through the advanced subscriber
(history + recovery), keeps tiered downsampled series, and serves four read
procedures. Four things about it are worth saying out loud because the
instinct runs the other way on each:

- **It is a host-origin producer, not a service origin.** Service origins
  exist for single-writer fleet *state* — the reason `@catalog` and
  `@desired` are ones — and a history application writes none; it only
  answers RPC. Two historians (one per site, or one per host) are then
  ordinary [05 §2.1](05-control-rpc.md) fan-in with no claim protocol,
  each answering `range` from its own ring.
- **Series identity is `(origin, producer, subject)`** — the wire key minus
  the class chunk. It is derivable from a sample alone, so it survives a
  catalog merge and a correlator outage; entity resolution is a query-time
  join, never a storage key.
- **It publishes no telemetry.** [04 §1.1](04-planes.md) already forbids
  re-publishing on a data key; it is restated here because a history
  application is exactly the thing tempted to re-emit. Its own numbers ride
  its health document and `stats`.
- **`range` answers through the [05 §3.2](05-control-rpc.md) envelope**:
  `step` is clamped to a tier and the *served* `step_s` is stated in the
  reply; `covers_from` states the oldest instant that tier could answer
  for, so a sub-minute query over a hot ring cannot pass ten minutes off as
  the day; `agg` defaults by the subject's kind — counter → rate, gauge →
  avg, bool → max. `timeline` (events and alert transitions, newest first)
  carries the `@rpc/logs/events` cursor contract, `after_uid`.

A declared-but-unbuilt procedure answers `error/unsupported`
([05 §3](05-control-rpc.md)) rather than going undeclared: an undeclared key
times out, and a timeout is indistinguishable from a slow fleet, a dropped
reply or a wrong key. [08 §6.1](08-registry.md)'s coverage check caught a
real bug the first time that was got wrong. On cost, this chapter asserts
no figure: at 10 000 series the reference implementation misses two of its
own storage targets, measured and recorded in `zensight-historian/docs/storage.md`
(zensight#911).

**The profile's framework extension (v1.25).** The framework block above
is the neutral set of [04-planes.md §1.4](04-planes.md) plus one
profile-defined token:

| Subject (under `state/<producer>/`) | `common` token | What |
|---|---|---|
| `errors` | `errors` | the rolling error window — the last N producer-side errors as one LWW document, refreshed like any state |

`errors` appears in no neutral chapter — it is ZenSight's `sensor-core`
convention, registered per producer with `common = "errors"` under
04 §1.4's profile-extension rule. Another adopter neither inherits it nor
collides with it.

## 3. Mapping: every shipped family → its convention home

Conceptual correspondence (shipped grammar per
[`docs/KEYSPACE.md`](https://github.com/p13marc/zensight/blob/master/docs/KEYSPACE.md)):

| Shipped family | Convention home | Notes |
|---|---|---|
| `zensight/<proto>/<source>/<metric>` | `…/<origin>/telemetry/<proto>/[<device>/]<metric>` | `<source>` → origin (host-local) or first subject chunk (proxy) |
| `…/<source>/@/health` `@/errors` `@/status` | `…/<origin>/state/<proto>/health` etc. | status doc merges into health/registration |
| `…/<source>/@/alive` (+ devices) | `…/state/<proto>/alive`, `…/device/<d>/alive` | token keys mirror state grammar ([04-planes.md §5](04-planes.md)) |
| `…/<proto>/@/alerts/<key>` | `…/<origin>/state/<proto>/alert/<key>` | key function **changes**: shipped = `<rule>-<16hex>` of FNV-1a(source+rule+labels), case-preserving; convention = 16 lowercase hex of FNV-1a(rule+labels) — source dropped (origin+producer are in the key), rule prefix dropped (uppercase rules violate the charset). The neutral requirement (stable, origin-excluded, byte-precise per profile) is [04-planes.md §1.2](04-planes.md); the byte-precise recipe is §3.1 below |
| `…/<proto>/@/query/alerts` | GET `…/*/state/*/alert/*` | seed = state itself ([05 §4](05-control-rpc.md)) |
| `…/@/commands/<t>` + `@/status/<t>` + `@/query/<t>` | `…/<origin>/@rpc/<proto>/…` | mapping pattern in [05-control-rpc.md §5](05-control-rpc.md); full table in §5 below |
| `…/@/artifact/{request,status,cancel}` | `@rpc` + `state/<proto>/artifact/<kind>` | long-running pattern ([05 §3](05-control-rpc.md)) |
| `…/@/artifact/blob/<id>/**`, `…/@/store/**`, `…/@/tree/**` | `…/<origin>/@blob/{artifact,store,tree}/…` | one plane ([07-bulk-planes.md](07-bulk-planes.md)) |
| `…/<source>/@media/<stream>/…` | `…/<origin>/@media/parallax/<stream>/…` | producer chunk added |
| `_meta/sensors/<name>/<source>` | `…/<origin>/state/<proto>/sensor` | |
| `_meta/evidence/host/<sensor>/<source>` | `…/<origin>/state/<proto>/evidence/self` (or `…/evidence/device/<d>`) | observer split by subject, not payload flag |
| `_meta/evidence/names/<sensor>/<ip>` | `…/<origin>/state/<proto>/evidence/names/<ip>` | |
| `_meta/entity/host/<id>` | `@catalog/state/entity/<id>` | |
| *(none — GUI-local `EdgeKind` derived from `@rpc` replies)* | `@catalog/state/edge/<edge-id>` | **new in v1.30**: the topology graph was derived inside one GUI from netring's matrix, netlink's neighbours and gateways, and never left it. Sensors now claim (`evidence/relation/<relation-id>`) and the catalog concludes ([06 §5.6](06-identity.md)); §3.3 below binds both ids |
| *(none — flow adjacency, GUI-local)* | **`@rpc/netring/matrix`, deliberately not an edge** | per-observed-peer and unbounded; [06 §5.6](06-identity.md) makes the exclusion normative |
| *(none — GUI-local charts over the influx storage)* | `…/<origin>/@rpc/historian/range` | **new in v1.31**: telemetry history as a computed answer ([04 §4](04-planes.md)); the influx capture stays the raw alternative and is optional ([09 §2](09-operations.md)) |
| `_meta/query/{entities,names}` | GET on entity state / `@catalog/@rpc/names` | |
| `_meta/correlator/@/alive` | `@catalog/state/alive` | |
| `zensight/@pdns/<ip>` | `@catalog/state/pdns/<ip>` | historical tier = storage choice ([06 §5.2](06-identity.md)) |
| `…/<source>/@/devices/<d>/liveness` | `…/state/<proto>/device/<d>/liveness` (doc) + `…/device/<d>/alive` (token) | |
| netflow `zensight/netflow/<exp>/<src>/<dst>` | **no as-is home — redesigned** | per-flow-pair keys are unbounded-cardinality per-message data ([03 §2](03-grammar.md), [04 R3](04-planes.md)): the family becomes bounded rollups on `telemetry` + on-demand `@rpc/netflow/flows` (§2) |
| gnmi bracketed paths | slugged `{path...}` subject | shipped `[name=eth0]` charset is illegal under [03 §2](03-grammar.md); see §2's slug rule |

Every shipped family has a mapped home, with two honest asymmetries.
*Forward*: netflow's per-pair telemetry and gNMI's bracketed paths do
**not** migrate as-is — their shipped shapes violate the grammar's
cardinality/charset rules and are redesigned above (the
[01 §5](01-motivation.md) "vocabulary migrates as-is" non-goal is
qualified accordingly). *Reverse*: a few convention mechanisms have no
shipped counterpart and are marked as new — `alias/<old-id>` records as
keys (shipped aliases are a payload field on `HostEntity`), the `events`
class's budget machinery, the ownership-claim keys of
[06 §5.3](06-identity.md), and since v1.30 the relationship pair
(`evidence/relation/<relation-id>` + `edge/<edge-id>`), whose shipped
counterpart was not a key at all but a derivation living in one GUI
process. (The one deliberate deletion: protocol-scoped
shared channels have no successor; their two uses — fan-in queries and
fleet-wide commands — are both expressed by `*`-origin RPC selectors.
The `@/status` running/offline flag lands in the `health` document's
status field.)

### 3.1 The alert key, byte-precise (moved from 04 §1.2 in v1.25)

[04-planes.md §1.2](04-planes.md) requires an alert key to be a stable,
origin-excluded hash of rule identity + discriminating labels, and leaves
the bytes to the profile. This is ZenSight's binding — byte-precise in the
manner of the origin derivation ([06-identity.md §1](06-identity.md)):
two independent implementations MUST mint the same key for the same
alert.

```
input      = rule_name
             ++ ( "\n" ++ label_name ++ "=" ++ label_value )*   for each
             discriminating label, ascending by label_name (byte order)
alert_key  = lowercase_hex(fnv1a_64(utf8(input)))               16 chars, all 64 bits
```

- **Hash.** FNV-1a, 64-bit: offset basis `0xcbf29ce484222325`, prime
  `0x100000001b3`, over the UTF-8 bytes of `input`.
- **Discriminating labels** are the labels that distinguish instances of
  one rule (`peer`, `port`, `unit`…). Host-scoped labels — the label
  named `host`, and any label the producer documents as host-scoped —
  are excluded *before* sorting: the origin already scopes the key
  ([04-planes.md §1.2](04-planes.md)), and hashing the host in would
  break the one property the exclusion exists for (the same alert on two
  hosts is the same key under two origins).
- **Framing.** Labels sort by `label_name` under ascending byte
  (memcmp) collation; each contributes `\n` (0x0a) + name + `=` (0x3d)
  + value. The framing is injective because label names are
  `snake_case` identifiers (no `\n`, no `=` can appear) and label values
  MUST NOT contain `\n`; a rule with no discriminating labels hashes the
  bare rule name.
- **Test vector** (implementations MUST reproduce this). Rule
  `link_down`, labels `{peer: "r2", port: "eth0",
  host: "h-3fa9c2d41b7e"}`. The `host` label is host-scoped and drops
  out; `peer` sorts before `port`. The input, byte for byte:

  ```
  link_down\npeer=r2\nport=eth0
  (27 bytes: 6c696e6b5f646f776e 0a 706565723d7232 0a 706f72743d65746830)
  ```

  FNV-1a-64 of those bytes is `0xa659f813308ad1da`, so
  `alert_key = a659f813308ad1da` and the full key is
  `…/state/netlink/alert/a659f813308ad1da`.

(This deliberately differs from the incumbent `alert_key`, which prefixes
the rule name and hashes the source — the §3 table row above records both
halves of the change and why.)

### 3.2 The alert ref, byte-precise (v1.29)

[06-identity.md §5.5](06-identity.md) keys an acknowledgement by
`ack/<alert-ref>`. An **alert ref** names one firing alert as a single key
chunk:

```
alert_ref = origin ++ "." ++ producer ++ "." ++ alert_key
```

- `origin` is the publishing host's origin chunk (`h-<12hex>`, §1 of
  [06](06-identity.md)); `producer` the producer chunk of the key the
  alert was published on; `alert_key` the §3.1 hash. All three are read
  from the **key**, never from the payload — an `Alert`'s `source` is the
  polled device for a proxy producer, so the document alone cannot say
  which host published it.
- **Parsing splits on the first two separators** (`splitn(3, '.')`) and
  keeps the remainder as `alert_key`. This is unambiguous rather than
  merely conventional: `origin` and `producer` are grammar chunks whose
  alphabet excludes `.`, while a `alert_key` may legitimately contain one
  (an application whose §3.1 binding differs, or a rule slug carrying a
  dotted metric name).
- **Why `.`.** It is the one separator already legal *inside* a chunk and
  already used there (`in_errors.rate`,
  [03-grammar.md §5](03-grammar.md)), so no component needs escaping and
  the key grammar is untouched. `/` would make three chunks and defeat
  the purpose; `:` and `@` are reserved elsewhere in the grammar.
- **Why not a hash of the triple.** It would be shorter and equally
  unique, and opaque in an explorer's output, in a storage listing, and
  in whatever an on-call tool renders — to exactly the operator who needs
  to know *which host's producer* is being acknowledged. There is no
  collision benefit: the triple is already the alert's full identity.
- A ref whose components would not survive as a key chunk MUST be
  refused rather than truncated or escaped: a ref that needs escaping is
  a ref that will be wrong somewhere.

**Test vector.** The §3.1 vector's alert, published by `netlink` on origin
`h-3fa9c2d41b7e`:

```
h-3fa9c2d41b7e.netlink.a659f813308ad1da
```

and the acknowledgement lives at
`…/@catalog/state/ack/h-3fa9c2d41b7e.netlink.a659f813308ad1da`.

### 3.3 The edge id and the relation id, byte-precise (v1.30)

[06-identity.md §5.6](06-identity.md) requires both ids to be a function
of `(kind, from, to)` and of nothing else, and requires two
implementations to agree. This is ZenSight's binding, in the manner of
§3.1.

```
US         = U+001F                                    (unit separator, 1 byte 0x1f)
triple     = kind_token ++ US ++ repr(from) ++ US ++ repr(to)
edge_id    = "e-" ++ lowercase_hex(fnv1a_64(utf8(triple)))    18 chars total
relation_id= "r-" ++ lowercase_hex(fnv1a_64(utf8(triple)))    18 chars total
```

Same hash as §3.1 — FNV-1a, 64-bit, offset basis `0xcbf29ce484222325`,
prime `0x100000001b3`. The two ids differ only in what `repr` produces,
because one hashes **resolved** ends and the other hashes **claims**:

```
# edge_id — resolved endpoints (06 §5.6)
repr(Entity)   = "e:" ++ entity_id
repr(External) = "x:" ++ ip ++ "|" ++ mac ++ "|" ++ name     absent field = empty

# relation_id — an unresolved claim
repr(Claim)    = host_id ++ "|" ++ device ++ "|" ++ ips ++ "|" ++ macs ++ "|" ++ name
                 ips and macs joined with "," in ascending byte order;
                 absent field = empty
```

**Kind tokens** (the closed vocabulary of 06 §5.6, as this profile binds
it). The token is part of the key, so none of them may ever change
spelling:

| Token | Meaning (`from` → `to`) | Containment |
|---|---|---|
| `hosts` | a hypervisor node hosts a guest | yes |
| `runs` | a host runs a container | yes |
| `gateway_of` | `from` is the gateway for `to` | yes |
| `probes` | `from` is a vantage point checking `to` | yes |
| `l2_adjacent` | the two share a link-layer segment | **no** — symmetric, no causal direction |

Only the containment kinds propagate failure in impact attribution
(06 §5.6); `l2_adjacent` never does, which is the whole reason the column
exists rather than being inferred.

- **Why the separators are what they are.** `US` cannot appear in a
  grammar chunk, an entity id or a slug, so the triple framing is
  injective without escaping; `|` inside a `repr` is likewise excluded
  from every field it separates. Neither byte ever reaches a key — only
  the hex does.
- **Why sort before hashing.** `ips` and `macs` arrive in whatever order
  a sensor observed them. Sorting is what makes a claim's id independent
  of observation order, which is the property 06 §5.6 rule 1 needs from a
  restart.
- **Why a hash here and not the readable triple of §3.2.** An
  `alert_ref` names one alert on one host and stays legible as a chunk;
  an edge names *two* endpoints, either of which may be an
  `External` carrying an IP, a MAC and a name. Spelling that out would
  produce keys hundreds of bytes long, would embed structure a consumer
  could be tempted to parse ([03-grammar.md §6.2](03-grammar.md)), and
  would change the key whenever a display name changed. The payload
  carries both endpoints in full; the key only has to be stable and
  unique.
- **Not cryptographic, and does not need to be.** The id names a key, it
  does not authenticate one. FNV-1a rather than `std`'s default hasher
  because that one is explicitly not stable across releases, and a key
  that moved on a toolchain upgrade would tombstone and republish the
  entire edge set.

**Test vectors** (implementations MUST reproduce these).

An edge: the pve node on origin `h-3fa9c2d41b7e` hosts the guest that
resolved to entity `h-9d02aa17c44f`.

```
triple  = "hosts" 1f "e:h-3fa9c2d41b7e" 1f "e:h-9d02aa17c44f"
edge_id = e-2879d4667f9d946d
```

Direction and kind both matter, which these pin:

```
hosts, ends swapped        → e-37f51d33d897f9b3
runs, same ends            → e-f5a68bdecd893aa2
```

The claim that produced it — pve self-claims its own `host_id` and names
the guest by device slug `vm-101` and name `db01`:

```
repr(from)  = "h-3fa9c2d41b7e||||"
repr(to)    = "|vm-101|||db01"
relation_id = r-f5f9a2edb9601155
```

published at
`zensight/v1/h-3fa9c2d41b7e/state/pve/evidence/relation/r-f5f9a2edb9601155`,
and the edge at
`zensight/v1/@catalog/state/edge/e-2879d4667f9d946d`.

## 4. What ZenSight-specific knowledge remains

For other adopters, the checklist of what they would replace: the base
chunk; the producer vocabulary and their registry files
([08-registry.md](08-registry.md)); the payload types (`TelemetryPoint`,
`Alert`, `HealthSnapshot`, `HostEntity`, `FrameMeta`…); the catalog
implementation behind `@catalog`; and the application salt constant of the
origin derivation (ZenSight's is `"zensight-host-id-v1"`, compiled-in and
non-configurable — [06-identity.md §1](06-identity.md)). Everything else
in chapters 02–10 transfers unchanged.

One binding worth stating (v1.32): `TelemetryPoint.value` is the
internally tagged `TelemetryValue` — `{"type": "counter" | "gauge" |
"text" | "boolean", "value": …}` — and that tag is the registry's `kind`
([08-registry.md §2](08-registry.md)) spelled on the wire (`boolean` for
`bool`). The profile's `checked_point` guard therefore asserts the variant
against the declared kind at build time, and the exporters derive
Prometheus `TYPE` and the OTLP instrument from `kind`, not from the
variant — which is what turns "a sensor published `oom_kills_total` as a
gauge and nothing could have caught it" into a build failure.

## 5. Mapping the incumbent control channels (moved from 05 §5 in v1.25)

The row-by-row mapping of every shipped ZenSight control channel onto the
`@rpc` plane — normative for ZenSight's migration, illustrative for other
adopters. Each row instantiates the neutral pattern of
[05-control-rpc.md §5](05-control-rpc.md). `P` = the producer chunk.

| Incumbent key (protocol- or host-scoped) | Convention location |
|---|---|
| `…/@/commands/<topic>` + `…/@/status/<topic>` | `@rpc/P/<topic>/set` (write, ack reply) + `@rpc/P/<topic>` (read current) |
| `…/@/query/<topic>` | `@rpc/P/<topic>` (read) |
| `…/@/query/alerts` (firing seed) | GET on `state/*/alert/*` selector ([05 §4](05-control-rpc.md)) |
| `…/@/artifact/request` (pub/sub) | `@rpc/P/artifact/request` (write → `{id}` or error reply) |
| `…/@/artifact/status` (queryable) | `state/P/artifact/<kind>` (observable LWW status) |
| `…/@/artifact/cancel` (pub/sub) | `@rpc/P/artifact/cancel?id=` (write) |
| `…/@/artifact/blob/<id>/**`, `…/@/store/**`, `…/@/tree/**` | `@blob/…` ([07-bulk-planes.md](07-bulk-planes.md)) |
| logs `@/query/events?since=;max=;host=` | `@rpc/logs/events?since=;max=;source=` (`source=` filters the *observed* device — a centralized syslog receiver holds many sources' lines; origin targeting selects the receiver, not the line's source) |
| netlink `@/commands/expectations` | `@rpc/netlink/expectations/set` + read at `@rpc/netlink/expectations` |
| netring `@/commands/capture_disk` (`capture_now`) | `@rpc/netring/capture/trigger` (write) + `state/netring/capture` (mode/occupancy) + `events/netring/capture/<ulid>` |
| systemd `@/commands/action` (gated) | `@rpc/systemd/action` (write; gate unchanged, plus per-key ACL) |
| parallax `@/commands/stream` (`OpenStream`…) | `@rpc/parallax/stream/set` (write, `Command<StreamControl>` → `Ack`: `OpenStream`, `CloseStream`, `RequestKeyframe`), plus `@rpc/parallax/stream/report` for receiver feedback ([07 §1.1](07-bulk-planes.md)) |
| parallax `@/query/streams`, `@/status/streams` | `state/parallax/stream/<stream>` (catalogue + status as LWW docs; a closed stream keeps its doc with `open: false` — tombstone on *removal from config*, not on close, or the UI loses the "openable streams" catalogue) |

*Corrected in v1.26.* The stream row named three write procedures —
`stream/open`, `stream/close`, `stream/keyframe` — that nothing has ever
served; the profile ships one, `stream/set`, carrying a tagged union. See
[07 §1.1](07-bulk-planes.md), where the convention-level shape is now
stated and the reasoning recorded.

### 5.1 The `streams` procedure is profile-local (v1.26)

`parallax.toml` registers, and the reference sensor serves, a read
procedure `@rpc/parallax/streams` replying with the whole stream catalogue
in one round trip. The v1.22 changelog called it "phantom" on the grounds
that 07 §1 advertised tiers via a procedure the convention never defined;
that correction was right about the *convention* and wrong about the
*deployment*, because the procedure is live and consumers call it.

It is settled here rather than in a normative chapter, and it stays:

- **The normative catalogue is the per-stream `state` documents.** A
  consumer that follows the convention and nothing else subscribes or GETs
  `state/parallax/stream/*` and is complete — the seed rules of
  [05 §4](05-control-rpc.md) make that its own late-joiner path. Nothing
  about tier discovery depends on this procedure.
- **`streams` is a profile-local convenience over that same data**: one
  round trip for N streams instead of a subscription that stays live. It is
  legitimate exactly as [08 §6.1](08-registry.md) requires — registered and
  served, never registered-and-absent — and it is named here so that
  another adopter neither inherits it nor collides with the name.
- **It is not promoted to the convention.** A catalogue-in-one-call
  procedure is a shape any producer of a many-instance `state` family
  might want; blessing this one would bless a general mechanism on the
  strength of a single profile's convenience. If that mechanism is wanted,
  it is its own amendment to [05 §5](05-control-rpc.md), not a parallax
  entry read as precedent.
- **It is not removed.** Removal would be a retirement
  ([08 §3](08-registry.md)) that costs the reference sensor and its viewers
  a working path to buy a contradiction that this subsection resolves for
  free.

The generic first row covers, by name, every shipped config-style topic not
listed individually: logs `filter` → `@rpc/logs/filter/set` + read at
`@rpc/logs/filter`; netlink `collection` → `@rpc/netlink/collection/set`;
netring `detectors`, `capture_filter`, `threat_intel` →
`@rpc/netring/<topic>/set` — each with the read procedure at the same key
minus `/set`.
