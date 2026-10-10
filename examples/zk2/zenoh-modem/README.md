# zenoh-modem, mapped onto zk2 (#623)

zenoh-modem carries stock Zenoh 1.10 over non-IP modems:
- RF at 220 B MTU and 2,400 bit/s;
- LoRa at a 1% duty cycle;
- Iridium SBD at about 96 messages a day.

**The contract.** A per-host management plane serves one contract for every backend: SBD,
SX1262, netdev, DLEP, ModemManager. That contract is `modem-contract/registry/modem.toml` v2.14:
- 87 subjects, 10 procedures, 3 errors and 26 deprecated paths;
- 88 of the 97 entries gated by 31 `capability:` predicates;
- RFC 05 §5.1 configuration with per-device confirm floors;
- `exposure` markers that keep the plane off the radio.

Its seam crates are on crates.io for out-of-tree backends.

**The mapping.** This directory maps the whole contract onto r3.2.
- One **service per device** (`rf0`, `sat0`, `wwan0`) on a **system that is the host id**.
- [`modem.v3.toml`](modem.v3.toml) holds 86 resources, 80 of them optional with their v1 gates.
  Their payloads are in [`schemas/modem.json`](schemas/modem.json).
- The configuration plane's 8 gated entries are `config.v1`, bound in
  [`config.v1-binding.md`](config.v1-binding.md). That file also explains why the far side of a
  radio can arm a reach change today but never keep it, and gives the zk2 grants that fix it.
- The radio faces, the rules a generator would emit, and the declaration cost of zk2 keys against
  v1's are in [`link.md`](link.md).
- What `modem-contract` becomes is in [`contract-crate.md`](contract-crate.md).

**The verdict.** Every v1 entry maps or is deleted for a stated reason, and the constrained-face
story needs no `@zk` traffic across the radio. Of r3.2's nine claims, six hold, some needing a
format or profile addition. Three hold only in part:
- device-as-service fails for faces;
- the continuity epoch is invisible across a constrained face;
- system = host holds only per application.

The 22 gaps below each come with their evidence and a proposed correction.

**Draft 1 (#608).** The mapping was written in draft 0. #608 migrated it to draft 1, and
`zenkey-model` now validates it with no finding:
- the four `{occurrence}` streams are `kind = "event"` (D3), with v1's declared rates: `oper_change`
  and `identity_change` are `rare`, `device_fault` is `low`, and `action_event` is `burst(600/h)`;
- the QoS written on every resource moved into `[defaults.stream]` and `[defaults.state]` (D2).
  That removed 87 lines, and the fingerprint did not change: D2's promise, checked on a real contract.

The gaps below are the draft-0 record, kept as written; r3.3 §0.3 says where each one went.

## r3.2's claims, checked

| Claim (`docs/zk2/architecture.md`) | Verdict | Where |
|---|---|---|
| Device-as-service: one service per device, so per-device exposure, capabilities and ACL prefix (l.101-105, 333-336) | **Holds for keys, grants and descriptors.** The ACL can grant `rf0` and withhold `sat0`, which v1's generator could not (`grant_cannot_carve`). **Fails for faces**: zenoh cannot tell rf0's lane from sat0's (gap 10). | [binding](config.v1-binding.md) §12, [link](link.md) §3 |
| The instance id is the continuity epoch (l.337-341) | **Holds on the host bus. Invisible across a constrained face**, where a counter does cross (gap 1). The runtime also lacks an epoch-change sequence (gap 16). | [link](link.md) §5 |
| Optional resources with `gate = "capability:…"` (l.115) | **Holds mechanically**: 80 gates, one of them a conjunction. But the descriptor's unavailable list is 4.5–5.4 KB per device (gap 15), the vocabulary is split (gap 5), and plane-sized gates want a rule (gap 6). | `modem.v3.toml` |
| `config.v1` with its semantics whole (l.133) | **Holds**, with three profile additions: a union reply for `set` (gap 8), a compact status and visible floor (gap 9), and companion declaration (gap 7). | [binding](config.v1-binding.md) |
| `link.v1` (l.128) | **Holds** as a contract annotation plus deployment face files. It needs face-side exposure for standard interfaces (gap 20) and per-key downsampling rules (gap 11). It inherits the fail-open carve-out (gap 12). | [link](link.md) |
| No `@zk` traffic ever required across a constrained face (l.117, 453-458) | **Proved**: static binding, compiled contract, FULL_TRANSITIVE. Caveats are the epoch (gap 1) and liveness evidence (gap 14). `face-deny-zk` is mandatory, not defense in depth: presence would cost 277 B per device per reconnect. | [link](link.md) §5 |
| `default_permission` node-global; guarantees on O2 + R6; deny as defense in depth (l.118) | **Holds**, and two facts sharpen it: allow rules are inert under `allow` (gap 13), and a fail-closed shape exists when the host face is a distinct protocol (gap 12). | [link](link.md) §4 |
| Contract crates, no zenoh by default (l.121) | **Holds**, with a Rust-first schema path (gap 19) and a cross-interface type (gap 18). | [crate](contract-crate.md) |
| System = host, by `hostid.v1` (l.95-99) | **Holds only per application**: v1 salts the host id per application, so one machine has three host ids across the three adopters (gap 22). | below |

## Shape (for #604)

| zk2 concept | zenoh-modem |
|---|---|
| `system` | the node's host id, `h-<12hex>` (`hostid.v1`; see gap 22 for its salt) |
| `service` | one per device the host serves: `rf0`, `sat0`, `wwan0`, `lora0`, `dlep0`. They are hosted by one driver process (`modem-mgmt`'s `Host` with N `Node`s), one session, one instance id per service. |
| interfaces | `modem.v3` on every device service; `config.v1` where the device has a configuration plane (v1 `capability:config`); `health.v1` ([`spec/profiles/health/health.v1.toml`](../../../spec/profiles/health/health.v1.toml), the profile [`health.v1`](../../../spec/profiles/health/v1.md)) recommended, though none of it crosses the RF face ([`health.v1`](../../../spec/profiles/health/v1.md) §2.8), its status derived from `oper` (`up` → ok; `down` and recoverable → degraded; `down` and not recoverable → failed) |
| presence | per device service: an instance token, plus one interface token per interface the device exposes something of. A netdev holds no SDU resource, but it still implements `modem.v3`, because its `oper`, `identity` and `security` are always exposed. |
| continuity epoch | a new instance id whenever a device's counters reset: re-enumeration, ifindex change, restart |
| ownership | every resource `exclusive`; both operations `fanout = "forbidden"`; config writes serialized per device by the server, which is v1's single-writer rule |
| schema kind | **jsonschema**, Rust-first: schemars output committed ([crate](contract-crate.md) §2) |

Example keys, for host `h-3fa9c2d41b7e` and device `rf0`:

```text
zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/sdu/dropped_total           a link-exposed counter
zk2/h-3fa9c2d41b7e/rf0/modem.v3/state/sdu/endpoint/peer-b          a templated state, cardinality 64
zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/oper_change/01j9x3k7q2m4n5p6r7s8t9v0wx
zk2/h-3fa9c2d41b7e/sat0/modem.v3/@op/action/mailbox_check
zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/set/radio                      a reach group: {token, apply_at} first
zk2/h-3fa9c2d41b7e/rf0/config.v1/state/view                         the echo; GET is the read
zk2/h-3fa9c2d41b7e/rf0/@zk/alive/modem.v3/<instance>/<fp16>         never across a constrained face
zk2/@zk/contract/modem.v3/<sha256>
zk2/*/*/@zk/alive/config.v1/**                                     "which devices can be configured"
```

## Decisions recorded here

- **v1's events are occurrence-keyed streams, all five.** Each exists so that a reader arriving
  after the fact still learns what happened (v1 RFC 04 §1.3), which is r3.2's definition of the
  occurrence-keyed case. A reliable stream would lose them to a reader that was not connected.
  - The reason and retention for each are on each resource in `modem.v3.toml`.
  - `device_fault` is edge-triggered raised/cleared, which is `alarms.v1`'s shape: a later option,
    not taken in a 1:1 mapping.
  - The v1 ULIDs are already lowercase (`modem-mgmt/src/clock.rs:54-69`), so the keys are the
    same shape.
- **The action event is renamed `action_event/{occurrence}`.** `action/{occurrence}` would have
  the same shape as the operation `action/{name}` (gap 3).
- **QoS keeps v1's three profiles** explicitly: `sampled` for telemetry, `refreshed` for states,
  `transition` for occurrence streams and the config echo. The refreshed states stay best-effort:
  their re-put every `ttl_s/2` heals a loss, and the owner's GET is the truth.
- **One interface, not a split by plane.** `modem.v3` maps v1 one-to-one, with its gates. The
  case for `modem_sdu.v1` and `modem_netdev.v1` is gap 6. It must be decided before `modem.v3`
  freezes, because moving resources out later is breaking.
- **`sensor`, `describe`, `introspect` and the two `alive` tokens are deleted.**
  - Their content goes to the descriptor, the bundle and the instance and interface tokens.
  - The capabilities that `sensor` listed per device are each device service's descriptor's.
  - Its `config_hash` belongs to `config.v1`.
- **Exposure defaults to `host`.** Only the four v1 `link` streams carry `link.exposure`.

## Every v1 entry, mapped

Generated from `modem.toml` v2.14 against `modem.v3.toml`. The zk2 column is relative to
`zk2/<host-id>/<device>/`.

### Telemetry (73) → `modem.v3` streams

Key: `zk2/<host-id>/<device>/modem.v3/stream/<resource>`; every one keeps v1's `sampled` QoS (`priority = "data_low"`) and carries `telemetry.kind` (and `telemetry.unit` where v1 had one).

| # | v1 `telemetry/modem/…` | Gate (`capability:`) | zk2 resource | Type · kind · unit |
|---|---|---|---|---|
| 1 | `device/{device}/sdu/tx_sdus_total` | `sdu` | `sdu/tx_sdus_total` | `Counter` · counter |
| 2 | `device/{device}/sdu/tx_bytes_total` | `sdu` | `sdu/tx_bytes_total` | `Counter` · counter · `By` |
| 3 | `device/{device}/sdu/rx_sdus_total` | `sdu` | `sdu/rx_sdus_total` | `Counter` · counter |
| 4 | `device/{device}/sdu/rx_bytes_total` | `sdu` | `sdu/rx_bytes_total` | `Counter` · counter · `By` |
| 5 | `device/{device}/sdu/dropped_total` | `sdu` | `sdu/dropped_total` | `Counter` · counter · **`link.exposure = "link"`** |
| 6 | `device/{device}/sdu/tx_failed_total` | `sdu` | `sdu/tx_failed_total` | `Counter` · counter |
| 7 | `device/{device}/sdu/rx_unroutable_total` | `sdu` | `sdu/rx_unroutable_total` | `Counter` · counter |
| 8 | `device/{device}/sdu/rx_oversize_total` | `sdu` | `sdu/rx_oversize_total` | `Counter` · counter |
| 9 | `device/{device}/sdu/tx_oversize_total` | `sdu` | `sdu/tx_oversize_total` | `Counter` · counter |
| 10 | `device/{device}/sched/dropped_total` | `sdu` | `sched/dropped_total` | `Counter` · counter |
| 11 | `device/{device}/sched/stale_total` | `sdu` | `sched/stale_total` | `Counter` · counter |
| 12 | `device/{device}/sched/peerless_total` | `sdu` | `sched/peerless_total` | `Counter` · counter |
| 13 | `device/{device}/sched/queue_depth` | `sdu` | `sched/queue_depth` | `Gauge` · gauge · **`link.exposure = "link"`** |
| 14 | `device/{device}/sched/lane_backlog_bytes` | `lane_backlog` | `sched/lane_backlog_bytes` | `Gauge` · gauge · `By` |
| 15 | `device/{device}/sched/lane_backlog_drain_ms` | `lane_backlog` ∧ `sdu_time` | `sched/lane_backlog_drain_ms` | `Gauge` · gauge · `ms` |
| 16 | `device/{device}/air/tx_frames_total` | `framing` | `air/tx_frames_total` | `Counter` · counter |
| 17 | `device/{device}/air/rx_frames_total` | `framing` | `air/rx_frames_total` | `Counter` · counter |
| 18 | `device/{device}/air/cad_deferrals_total` | `lbt` | `air/cad_deferrals_total` | `Counter` · counter |
| 19 | `device/{device}/air/cad_forced_total` | `lbt` | `air/cad_forced_total` | `Counter` · counter |
| 20 | `device/{device}/layer/aead/auth_failures_total` | `layer_aead` | `layer/aead/auth_failures_total` | `Counter` · counter |
| 21 | `device/{device}/layer/aead/replays_total` | `layer_aead` | `layer/aead/replays_total` | `Counter` · counter |
| 22 | `device/{device}/layer/aead/unknown_key_total` | `layer_aead` | `layer/aead/unknown_key_total` | `Counter` · counter |
| 23 | `device/{device}/layer/aead/send_key_id` | `layer_aead` | `layer/aead/send_key_id` | `Gauge` · gauge |
| 24 | `device/{device}/layer/address/foreign_total` | `layer_address` | `layer/address/foreign_total` | `Counter` · counter |
| 25 | `device/{device}/layer/address/unknown_source_total` | `layer_address` | `layer/address/unknown_source_total` | `Counter` · counter |
| 26 | `device/{device}/layer/fec/repair_frames_total` | `layer_fec` | `layer/fec/repair_frames_total` | `Counter` · counter |
| 27 | `device/{device}/layer/fec/repaired_total` | `layer_fec` | `layer/fec/repaired_total` | `Counter` · counter |
| 28 | `device/{device}/layer/fec/unrecoverable_total` | `layer_fec` | `layer/fec/unrecoverable_total` | `Counter` · counter |
| 29 | `device/{device}/layer/fec/late_repair_total` | `layer_fec` | `layer/fec/late_repair_total` | `Counter` · counter |
| 30 | `device/{device}/layer/fec/unused_repair_total` | `layer_fec` | `layer/fec/unused_repair_total` | `Counter` · counter |
| 31 | `device/{device}/layer/fec/malformed_total` | `layer_fec` | `layer/fec/malformed_total` | `Counter` · counter |
| 32 | `device/{device}/layer/fragment/fragments_total` | `layer_fragment` | `layer/fragment/fragments_total` | `Counter` · counter |
| 33 | `device/{device}/layer/fragment/reassembled_total` | `layer_fragment` | `layer/fragment/reassembled_total` | `Counter` · counter |
| 34 | `device/{device}/layer/fragment/rcs_failures_total` | `layer_fragment` | `layer/fragment/rcs_failures_total` | `Counter` · counter |
| 35 | `device/{device}/layer/fragment/timeouts_total` | `layer_fragment` | `layer/fragment/timeouts_total` | `Counter` · counter |
| 36 | `device/{device}/layer/fragment/oversize_total` | `layer_fragment` | `layer/fragment/oversize_total` | `Counter` · counter |
| 37 | `device/{device}/layer/fragment/malformed_total` | `layer_fragment` | `layer/fragment/malformed_total` | `Counter` · counter |
| 38 | `device/{device}/layer/compress/compressed_total` | `layer_compress` | `layer/compress/compressed_total` | `Counter` · counter |
| 39 | `device/{device}/layer/compress/stored_total` | `layer_compress` | `layer/compress/stored_total` | `Counter` · counter |
| 40 | `device/{device}/layer/compress/bytes_saved_total` | `layer_compress` | `layer/compress/bytes_saved_total` | `Counter` · counter |
| 41 | `device/{device}/layer/compress/dictionary_mismatch_total` | `layer_compress` | `layer/compress/dictionary_mismatch_total` | `Counter` · counter |
| 42 | `device/{device}/layer/compress/malformed_total` | `layer_compress` | `layer/compress/malformed_total` | `Counter` · counter |
| 43 | `device/{device}/air/airtime_ms_total` | `airtime` | `air/airtime_ms_total` | `Counter` · counter · `ms` |
| 44 | `device/{device}/air/retransmits_total` | `retransmits` | `air/retransmits_total` | `Counter` · counter |
| 45 | `device/{device}/air/bytes_wasted_total` | `slot_accounting` | `air/bytes_wasted_total` | `Counter` · counter · `By` |
| 46 | `device/{device}/air/rx_errors_total` | `rx_errors` | `air/rx_errors_total` | `Counter` · counter |
| 47 | `device/{device}/air/duty_cycle_available` | `duty_cycle` | `air/duty_cycle_available` | `Gauge` · gauge · **`link.exposure = "link"`** |
| 48 | `device/{device}/air/duty_cycle_wait_ms` | `duty_cycle` | `air/duty_cycle_wait_ms` | `Gauge` · gauge · `ms` · **`link.exposure = "link"`** |
| 49 | `device/{device}/radio/signal_bars` | `signal_bars` | `radio/signal_bars` | `Gauge` · gauge |
| 50 | `device/{device}/if/in_octets` | `netdev` | `if/in_octets` | `Counter` · counter · `By` |
| 51 | `device/{device}/if/out_octets` | `netdev` | `if/out_octets` | `Counter` · counter · `By` |
| 52 | `device/{device}/if/in_errors` | `netdev` | `if/in_errors` | `Counter` · counter |
| 53 | `device/{device}/if/out_errors` | `netdev` | `if/out_errors` | `Counter` · counter |
| 54 | `device/{device}/if/in_discards` | `netdev` | `if/in_discards` | `Counter` · counter |
| 55 | `device/{device}/if/out_discards` | `netdev` | `if/out_discards` | `Counter` · counter |
| 56 | `device/{device}/if/in_multicast_pkts` | `netdev` | `if/in_multicast_pkts` | `Counter` · counter |
| 57 | `device/{device}/if/in_pkts` | `netdev` | `if/in_pkts` | `Counter` · counter |
| 58 | `device/{device}/if/out_pkts` | `netdev` | `if/out_pkts` | `Counter` · counter |
| 59 | `device/{device}/if/qdisc_backlog_bytes` | `qdisc` | `if/qdisc_backlog_bytes` | `Gauge` · gauge · `By` |
| 60 | `device/{device}/if/qdisc_backlog_pkts` | `qdisc` | `if/qdisc_backlog_pkts` | `Gauge` · gauge |
| 61 | `device/{device}/radio/rssi` | `rssi` | `radio/rssi` | `Measure` · gauge · `dBm` |
| 62 | `device/{device}/radio/rsrp` | `rsrp` | `radio/rsrp` | `Measure` · gauge · `dBm` |
| 63 | `device/{device}/radio/rsrq` | `rsrq` | `radio/rsrq` | `Measure` · gauge · `dB` |
| 64 | `device/{device}/radio/snr` | `snr` | `radio/snr` | `Measure` · gauge · `dB` |
| 65 | `device/{device}/radio/error_rate` | `error_rate` | `radio/error_rate` | `Measure` · gauge · `%` |
| 66 | `device/{device}/link/rx_rate` | `data_rate` | `link/rx_rate` | `Gauge` · gauge · `bit/s` |
| 67 | `device/{device}/link/tx_rate` | `data_rate` | `link/tx_rate` | `Gauge` · gauge · `bit/s` |
| 68 | `device/{device}/link/rx_max_rate` | `data_rate` | `link/rx_max_rate` | `Gauge` · gauge · `bit/s` |
| 69 | `device/{device}/link/tx_max_rate` | `data_rate` | `link/tx_max_rate` | `Gauge` · gauge · `bit/s` |
| 70 | `device/{device}/link/latency` | `latency` | `link/latency` | `Gauge` · gauge · `us` |
| 71 | `device/{device}/link/resources` | `resources` | `link/resources` | `Measure` · gauge · `%` |
| 72 | `device/{device}/link/rx_quality` | `link_quality` | `link/rx_quality` | `Measure` · gauge · `%` |
| 73 | `device/{device}/link/tx_quality` | `link_quality` | `link/tx_quality` | `Measure` · gauge · `%` |

### State (9)

| # | v1 `state/modem/…` | Gate | zk2 | Notes |
|---|---|---|---|---|
| 1 | `sensor` | — | **deleted** | `version`, `build`, `zenkey` → the descriptor's metadata; `capabilities{device → [name]}` → each device service's descriptor ("the capabilities this instance holds", r3.2 §3.10); `config_hash` → `config.v1` |
| 2 | `device/{device}/sdu/link` | `sdu` | `modem.v3/state/sdu/link` | `LinkState`, `freshness.ttl_s = 60`, v1 `refreshed` QoS |
| 3 | `device/{device}/sdu/endpoint/{endpoint}` | `sdu` | `modem.v3/state/sdu/endpoint/{endpoint}` | `EndpointState`, `freshness.ttl_s = 60`, v1 `refreshed` QoS, cardinality 64 |
| 4 | `device/{device}/destination/{destination}` | `destinations` | `modem.v3/state/destination/{destination}` | `DestinationState`, `freshness.ttl_s = 60`, v1 `refreshed` QoS, cardinality 64 |
| 5 | `device/{device}/security` | — | `modem.v3/state/security` | `SecurityState`, `freshness.ttl_s = 60`, v1 `refreshed` QoS |
| 6 | `device/{device}/oper` | — | `modem.v3/state/oper` | `Oper`, `freshness.ttl_s = 60`, v1 `refreshed` QoS |
| 7 | `device/{device}/identity` | — | `modem.v3/state/identity` | `Identity`, `freshness.ttl_s = 60`, v1 `refreshed` QoS |
| 8 | `device/{device}/radio` | `registration` | `modem.v3/state/radio` | `RadioState`, `freshness.ttl_s = 60`, v1 `refreshed` QoS |
| 9 | `config/{device}` | `config` | `config.v1/state/view` | the echo: v1 `transition` QoS = the zk2 state default ([binding](config.v1-binding.md) §7) |

### Events (5) → occurrence-keyed streams

| # | v1 `events/modem/…` | Gate | v1 rate | zk2 | Retention · bound |
|---|---|---|---|---|---|
| 1 | `device/{device}/oper_change/{event_id}` | — | `rare` | `modem.v3/stream/oper_change/{occurrence}` | `7d` · 168 |
| 2 | `device/{device}/device_fault/{event_id}` | — | `low` | `modem.v3/stream/device_fault/{occurrence}` | `1d` · 1,440 |
| 3 | `device/{device}/identity_change/{event_id}` | — | `rare` | `modem.v3/stream/identity_change/{occurrence}` | `7d` · 168 |
| 4 | `device/{device}/config_change/{event_id}` | `config` | `burst(600/h)` | `config.v1/stream/changes/{occurrence}` | `7d` · 100,800 (worst case) |
| 5 | `device/{device}/action/{event_id}` | `actions` | `burst(600/h)` | `modem.v3/stream/action_event/{occurrence}` | `1d` · 14,400 |

### Procedures (10)

| # | v1 `@rpc/modem/…` | Gate | zk2 | Notes |
|---|---|---|---|---|
| 1 | `describe` | — | **deleted** | the schemas ride in the bundle `zk2/@zk/contract/modem.v3/<sha256>` (r3 §3.10) |
| 2 | `introspect` | — | **deleted** | the canonical contract is the bundle; the descriptor (≤ 1 KB target) says what is exposed |
| 3 | `device/{device}/sdu/lanes` | `sdu` | `modem.v3/@op/sdu/lanes` | `Empty` → `LaneTable`, idempotent |
| 4 | `config/{device}` | `config` | **deleted** → GET `config.v1/state/view` | a state GET is the read (S2) |
| 5 | `config/{device}/{group}/set` | `config` | `config.v1/@op/set/{group}` | hot: the view; reach: `{token, apply_at}` before the apply (gap 8) |
| 6 | `config/{device}/confirm` | `config` | `config.v1/@op/confirm` | its own key |
| 7 | `config/{device}/cancel` | `config` | `config.v1/@op/cancel` | its own key |
| 8 | `config/{device}/extend` | `config` | `config.v1/@op/extend` | its own key |
| 9 | `config/{device}/persist` | `config` | `config.v1/@op/persist` | its own key; replies the view (RFC v1.47), not v1's `Ack` |
| 10 | `device/{device}/action/{name}` | `actions` | `modem.v3/@op/action/{name}` | `ActionRequest` → `ActionOutcome`, fanout forbidden, error `ModemError` |

### Errors (3)

| v1 `[[error]]` | On | zk2 |
|---|---|---|
| `restart-required` | `config/{device}/{group}/set` | `config.v1` `ConfigError{code: "restart-required"}`, the `app` detail of `@op/set/{group}` |
| `device-refused` | `config/{device}/{group}/set` | `config.v1` `ConfigError{code: "device-refused"}`, the `app` detail of `@op/set/{group}` |
| `action-refused` | `device/{device}/action/{name}` | `modem.v3` `ModemError{code: "action-refused"}`, the `app` detail of `@op/action/{name}` |

### Tokens (2)

| v1 | zk2 |
|---|---|
| `state/modem/alive` (one per host, the producer) | **deleted**: there is no host-level service. Each device service has its own instance token `zk2/<host>/<device>/@zk/instance/<instance>`; v1's twin guard (`modem-mgmt/src/lib.rs:544-560`) becomes a liveliness query on that key before declaring. |
| `state/modem/device/{device}/alive` (cycles when counters reset) | the instance token plus one interface token per exposed interface (`…/@zk/alive/modem.v3/<instance>/<fp16>`). **The instance id is the continuity epoch**: a re-enumerated netdev mints a new one (gap 16). |

### Deprecated (26)

v1's `[[deprecated]]` ledger retired the 1.x paths in 2.0 (#120). **None is carried**: `modem.v3` is a new major, mutually invisible to v1, and a 1.x or 2.x reader is served by v1 until it is retired. Each row is listed so that nothing is left unmapped.

| v1 1.x path (retired in 2.0) | v1 2.x successor | zk2 |
|---|---|---|
| `{device}/tx_sdus_total` | `device/{device}/sdu/tx_sdus_total` | not carried; the successor is `modem.v3/stream/sdu/tx_sdus_total` |
| `{device}/tx_bytes_total` | `device/{device}/sdu/tx_bytes_total` | not carried; the successor is `modem.v3/stream/sdu/tx_bytes_total` |
| `{device}/rx_sdus_total` | `device/{device}/sdu/rx_sdus_total` | not carried; the successor is `modem.v3/stream/sdu/rx_sdus_total` |
| `{device}/rx_bytes_total` | `device/{device}/sdu/rx_bytes_total` | not carried; the successor is `modem.v3/stream/sdu/rx_bytes_total` |
| `{device}/dropped_total` | `device/{device}/sdu/dropped_total` | not carried; the successor is `modem.v3/stream/sdu/dropped_total` |
| `{device}/sched_dropped_total` | `device/{device}/sched/dropped_total` | not carried; the successor is `modem.v3/stream/sched/dropped_total` |
| `{device}/sched_stale_total` | `device/{device}/sched/stale_total` | not carried; the successor is `modem.v3/stream/sched/stale_total` |
| `{device}/sched_peerless_total` | `device/{device}/sched/peerless_total` | not carried; the successor is `modem.v3/stream/sched/peerless_total` |
| `{device}/tx_failed_total` | `device/{device}/sdu/tx_failed_total` | not carried; the successor is `modem.v3/stream/sdu/tx_failed_total` |
| `{device}/rx_unroutable_total` | `device/{device}/sdu/rx_unroutable_total` | not carried; the successor is `modem.v3/stream/sdu/rx_unroutable_total` |
| `{device}/rx_oversize_total` | `device/{device}/sdu/rx_oversize_total` | not carried; the successor is `modem.v3/stream/sdu/rx_oversize_total` |
| `{device}/tx_oversize_total` | `device/{device}/sdu/tx_oversize_total` | not carried; the successor is `modem.v3/stream/sdu/tx_oversize_total` |
| `{device}/airtime_ms_total` | `device/{device}/air/airtime_ms_total` | not carried; the successor is `modem.v3/stream/air/airtime_ms_total` |
| `{device}/retransmits_total` | `device/{device}/air/retransmits_total` | not carried; the successor is `modem.v3/stream/air/retransmits_total` |
| `{device}/bytes_wasted_total` | `device/{device}/air/bytes_wasted_total` | not carried; the successor is `modem.v3/stream/air/bytes_wasted_total` |
| `{device}/rx_errors_total` | `device/{device}/air/rx_errors_total` | not carried; the successor is `modem.v3/stream/air/rx_errors_total` |
| `{device}/duty_cycle_available` | `device/{device}/air/duty_cycle_available` | not carried; the successor is `modem.v3/stream/air/duty_cycle_available` |
| `{device}/duty_cycle_wait_ms` | `device/{device}/air/duty_cycle_wait_ms` | not carried; the successor is `modem.v3/stream/air/duty_cycle_wait_ms` |
| `{device}/signal_bars` | `device/{device}/radio/signal_bars` | not carried; the successor is `modem.v3/stream/radio/signal_bars` |
| `{device}/queue_depth` | `device/{device}/sched/queue_depth` | not carried; the successor is `modem.v3/stream/sched/queue_depth` |
| `link/{device}` | `device/{device}/sdu/link` | not carried; the successor is `modem.v3/state/sdu/link` |
| `security/{device}` | `device/{device}/security` | not carried; the successor is `modem.v3/state/security` |
| `endpoint/{device}/{endpoint}` | `device/{device}/sdu/endpoint/{endpoint}` | not carried; the successor is `modem.v3/state/sdu/endpoint/{endpoint}` |
| `link_change/{event_id}` | `device/{device}/oper_change/{event_id}` | not carried; the successor is `modem.v3/stream/oper_change/{occurrence}` |
| `device_fault/{event_id}` | `device/{device}/device_fault/{event_id}` | not carried; the successor is `modem.v3/stream/device_fault/{occurrence}` |
| `{device}/lanes` | `device/{device}/sdu/lanes` | not carried; the successor is `modem.v3/@op/sdu/lanes` |

**Count.** 73 + 9 + 5 + 10 entries:
- 86 → `modem.v3`, 80 of them gated;
- 8 → `config.v1`, all gated `config`, so 80 + 8 = v1's 88;
- 3 deleted (`sensor`, `describe`, `introspect`).

Plus 3 errors, 2 tokens and 26 retired paths. Nothing in `modem.toml` is left unmapped.

## Gaps found

Each gap gives what is missing, the evidence, and a proposed correction. "r3.2" is
`docs/zk2/architecture.md`, and "draft 0" is `../README.md`.

1. **A counter's continuity epoch is invisible across a constrained face.** The instance id, which
   r3.2 makes the epoch, lives only in `@zk` tokens and descriptors, and those never cross. Yet
   `sdu/dropped_total` is a counter, and it crosses RF.
   - *Evidence:* r3.2 l.337-341 against l.453-458. v1 is equally blind: `zenohd-rf0.json5:80-82`
     denies the device token, which zenoh-modem's `interfaces.md:298` calls "the sanctioned reset".
   - *Correction:* `telemetry.v1` states that a counter's decrease is a reset where the epoch is
     unobservable. `link.v1` may require a typed attachment carrying the epoch (r3.2 §3.2) on every
     counter it exposes.
2. **Occurrence-keyed streams cannot state a rate, so their population bound is uncheckable.** v1's
   `rate = rare | low | burst(600/h)` is enforced by the publisher, which folds the excess into
   `occurrences`. The cardinalities in `modem.v3.toml` (168 to 14,400; 100,800 for `config.v1`) are
   rate × retention, worked out by hand.
   - *Evidence:* `modem.toml:1127, 1141, 1151, 1168, 1183`; `modem-mgmt/src/events.rs`
     `FAULT_FLOOR`; `config.rs` `Bucket`; draft 0 l.81-82, 99.
   - *Correction:* a core `rate` attribute on occurrence streams, or `timing.v1` `max_rate` with a
     burst. Conformance checks rate × retention ≤ cardinality. Name the `occurrences` folding
     convention: summarized, never dropped.
3. **Template uniqueness is undefined for templates of the same shape.** `action/{name}` (an
   operation) and `action/{event_id}` (an event) differ in text but not in shape.
   - *Evidence:* draft 0 l.72-73 says "unique across the whole interface, whatever its kind";
     `modem.toml:1180, 1311`. Renamed here to `action_event/{occurrence}`.
   - *Correction:* define uniqueness structurally (literal chunks and parameter positions, names
     ignored) and lint it.
4. **No interface-level defaults.** `modem.v3.toml` repeats `priority = "data_low"` 73 times,
   `reliability` and `congestion` 7 times, and `freshness.ttl_s` 7 times. This is tcgui's gap 2,
   extended to QoS.
   - *Evidence:* draft 0 l.94-97. v1 gave QoS once per class (`zenkey/src/qos.rs:74-92`).
   - *Correction:* `[defaults.stream]`, `[defaults.state]` and interface annotations, expanded into
     each resource before canonicalization, so the sugar does not move the fingerprint.
5. **Two vocabularies for why a resource is absent, and the gate kind is not the cause.**
   - *Evidence:*
     - draft 0's gate kinds are `capability | config | feature` (l.84); r3.2's causes are
       `build | config | capability` (l.115, 380);
     - modem's `layer_*` gates are config-caused (`modem.toml:292`, "a device with an aead layer
       configured"), and `lane_backlog` is driver-caused (`modem.toml:205`).
   - *Correction:* one vocabulary, `build | config | capability`. The gate names the predicate; the
     descriptor's cause is per instance and may differ from the gate's kind.
6. **No rule for "a plane as a gate" against "a plane as an interface".**
   - *Evidence:*
     - r3.2 splits configuration out as `config.v1` (8 entries), but leaves `capability:sdu` on
       16 `modem.v3` resources, `netdev` and `qdisc` on 11, and `layer_*` on 23;
     - a netdev exposes none of the SDU ones;
     - interface tokens exist only where something is exposed (r3.2 l.116), which would make
       "every SDU device" a zero-payload selector.
   - *Correction:* guidance in r3 §3.2 and §3.12: a gate covering a coherent plane with its own
     consumers becomes an interface. For zenoh-modem, decide `modem_sdu.v1` and `modem_netdev.v1`
     before `modem.v3` freezes.
7. **Companion interfaces are unexpressed.** `capability:config` gated 8 v1 entries, which are now
   `config.v1`. A `modem.v3` contract cannot say that the capability means "also implements
   `config.v1`", and declared − gated = served now spans two interfaces.
   - *Evidence:* `modem.toml:1105, 1171, 1238-1301`.
   - *Correction:* an optional `[companions]` table (interface plus gate), fingerprinted; or state
     that interface presence is the gate, and drop the capability name.
8. **A response type that depends on the data.** `config.v1`'s `set` replies with the view (hot
   groups, dry runs) or with `{token, apply_at, deadline}` (reach groups). v1 declares the view
   only, which is wrong for the case the reach class exists for.
   - *Evidence:* `modem.toml:1248` against `modem-mgmt/src/config.rs:373-378, 396-409`; draft 0
     allows one `response` (l.105).
   - *Correction:* a discriminated `SetReply` union, which needs S7's subset to admit `oneOf` with
     a discriminator. Splitting `set` and `arm` is the alternative, and is rejected: the class is
     the device's, not the caller's.
9. **The `config.v1` echo does not fit a constrained face, and the floor is not visible.**
   - *Evidence:*
     - the view is about 1,286 B: six SDUs, 4.4 s at 2,400 bit/s per read, and about 15% of the
       channel if subscribed at the 30 s refresh;
     - `confirm`, `cancel` and `extend` reply with the same view;
     - the floor appears only in a reply's `deadline`;
     - r3.2 l.133 claims "the echo visible over the face".
   - *Correction:* a `config.v1` `state/status` of about 319 B (revision, pending, last change,
     `confirm_floor_s`, values), and a per-group `writable` in the view. Control operations answer
     the status when asked to.
10. **Per-face exposure fails on a node with two radios.** Device-as-service gives per-device keys,
    but the face selector cannot tell rf0's lane from sat0's: `unixsock-stream` reports no
    interfaces. RF's carve-out and SBD's "nothing" cannot both hold on one router.
    - *Evidence:* zenoh-modem `architecture.md:274, 902-906`; r3.2 l.101-105.
    - *Correction:* `link.v1` states that face policy is per router transport. A two-radio node
      uses `zids` subjects per lane peer (unauthenticated, but the peer is configured) or a router
      per radio. S3 and S14 test `zids` subjects.
11. **Every key under one downsampling rule shares its timer.**
    - *Evidence:* zenoh 1.10.1 `interceptor/downsampling.rs:208-251, 281-304`. v1's
      `zenohd-rf0.json5:118-121` rules span `*` devices: a latent v1 bug on a two-device node.
    - *Correction:* the generator emits one rule per concrete (system, service, resource).
12. **The carve-out fails open for resources a later minor adds.** Under `allow` a carve-out is an
    absence. FULL_TRANSITIVE minors add resources, and the running ACL is not observable. Subtree
    denies narrow it ([link](link.md) §3), but a new resource under `sdu/`, `sched/` or `air/`
    crosses the radio until the block is regenerated.
    - *Evidence:* `zenohd-rf0.json5:83`; zenoh 1.10.1 `authorization.rs:604-614`; r3 §3.11,
      §3.13.
    - *Correction:* the generator offers the fail-closed shape ([link](link.md) §4): default deny
      plus a `tcp` host-face subject. That contradicts zenoh-modem's "only shape"
      (`architecture.md:286-296`) whenever the host face is a distinct protocol. S14 runs it.
13. **§3.13's grants are written as allows, and allow rules are inert under `allow`.** A grant is
    the absence of a deny in the principal's policy, plus a deny on the enumerated complement.
    - *Evidence:*
      - zenoh 1.10.1 `authorization.rs:613-614`;
      - the v1 fixture's `writes-ops` (`zenohd-rf0-ops.json5:49-51`) does nothing, and the `ops`
        principal thereby reaches every v1 `@rpc` read, `introspect` (about 50 KB) included.
    - *Correction:* §3.13 states how each grant shape compiles under each default. The generator
      enumerates every declared operation not granted, reads included, from the contracts and the
      deployment.
14. **R7's liveness evidence assumes a state crosses.** It reads "state freshness (`health.v1`)",
    but RF carries four streams and SBD carries nothing.
    - *Evidence:* r3.2 l.307; `zenohd-sat0-gateway.json5:99-102`.
    - *Correction:* R7 reads "the freshness of whatever `link.v1` exposes". Over SBD, liveness
      across the face is unobservable by design.
15. **The descriptor's ≤ 1 KB target fails for a union contract.** Listing unavailable optional
    resources one by one costs 4.5–5.4 KB per device (wwan0: 69 entries; rf0 and sat0: 57).
    - *Evidence:* r3.2 l.444-447, 458.
    - *Correction:* the descriptor lists the capabilities held, which takes 30–100 B. "Unavailable"
      is then every resource whose gate is not held, derivable from the contract. Only exceptions
      are listed per resource: a cause other than the gate's.
16. **No sequence and no API for a new epoch on a running service.** r3.2 §3.5 says MUST, but
    §3.10's start-up order and §3.14's Rust shape describe only a start. modem-mgmt cycles a
    netdev's token in place.
    - *Evidence:* r3.2 l.337-341, 472-477; `modem-mgmt/src/lib.rs:644-680`.
    - *Correction:* §3.10 gains an epoch-change sequence: withdraw the interface tokens, then the
      instance token; mint a new id; re-put the descriptor; declare the tokens. §3.14 gains
      `Service::new_epoch()`. The spec must say what consumers do with samples in flight, since
      there is no per-sample attribution (§8).
17. **The bound on devices per host is lost.** v1's `cardinality = 4` sat on every
    `device/{device}/…` template. Device-as-service removes the template, and the bound has no
    home. (Minor.)
    - *Evidence:* `modem.toml:38-39, 46`.
    - *Correction:* an optional interface annotation (`instances_per_system`) that tools check
      (U18), or drop the claim.
18. **No cross-interface type reference.** `ActionRequest.arguments` uses `config.v1`'s
    `ParamValue`, which is duplicated in `schemas/modem.json`.
    - *Evidence:* `documents.rs:696-703`. In draft 0, `json:Name` reaches only the interface's own
      schema files (l.59).
    - *Correction:* `json:<iface>.v<N>/<Name>`, resolved in that contract's bundle and fingerprinted
      by its sha256; or accept duplication, each copy checked on its own.
19. **Rust-first schemas need a type binding and a schema subset that fits.** modem-contract
    derives its schemas with schemars 1. v1 bound types to Rust in `types.toml`'s `rust =` field,
    and draft 0 has no place for that.
    - *Evidence:* `modem-contract/Cargo.toml`; `zenkey/src/config.rs:30, 110, 305` (an internally
      tagged enum, `flatten`, an untagged union).
    - *Correction:* S7's corpus includes schemars 1.x output for `Option`, unit enums, untagged
      unions and flattened tagged enums. zenkey-build gains `.rust_type("json:X", path)`, outside
      the fingerprint.
20. **`link.v1` annotations reach only resources the contract's author owns.** `config.v1`'s view
    and operations, exposed to an authenticated operator, cannot be annotated in a standard
    profile's contract.
    - *Evidence:* r3.2 l.128; draft 0 l.128-132.
    - *Correction:* face and role files may expose resources per principal. `link.v1` defines that
      vocabulary beside the contract annotation.
21. **`claimed_source` has no zk2 source.** O7 carries `{actor, request_id}` only, and zenoh's
    `SourceInfo` is unstable. (Minor.)
    - *Evidence:* `zenkey/src/config.rs:780-784`; r3.2 l.384, §3.8.
    - *Correction:* keep it optional, filled from the runtime's best effort and documented as
      unauthenticated.
22. **"System = host" holds only per application.** v1 salts the host id per application. RFC 06
    §1 makes the salt an application constant, so one machine mints three different `h-<12hex>`
    ids for the three adopters, and `zk2/<host>/**` would show one application's services.
    - *Evidence:* `rfcs/06-identity.md:49-58`; zenoh-modem `modem-contract/src/lib.rs:50-53`
      (`zenoh-modem-host-id-v1`); ZenSight's `zensight-host-id-v1`; tcgui's `tcgui-host-id-v1`;
      r3.2 l.95-98 ("per-host views … carry over position for position").
    - *Correction:* `hostid.v1` fixes one zk2-wide salt, so the system chunk agrees across
      adopters, while v1 origins keep their salts. The re-key costs nothing extra at the major
      boundary, but any far side that binds statically must be re-provisioned with the new name.
      zenoh-modem's RF peers do.

## v1 findings (zenoh-modem's own, made while mapping)

These are not zk2 gaps. They are drift between zenoh-modem's v1 registry, its implementation and
RFC 05 §5.1, and they are listed so that the port fixes them on purpose. Details are in
[`config.v1-binding.md`](config.v1-binding.md) §10 and [`link.md`](link.md) §3.
- `set` declares the wrong reply type for reach groups.
- The reach reply carries a `deadline` field that no schema describes.
- `persist` replies `{}` rather than the read-back.
- RFC v1.50's joining `set` and `last_change` are not implemented.
- `APPLY_AFTER` is a fixed 1 s, which is too short for a 300 bit/s reply.
- The SBD confirm floor protects a path that the shipped faces close.
- The `ops` principal reaches every `@rpc` read.
- v1's downsampling rules share one slot across devices.
