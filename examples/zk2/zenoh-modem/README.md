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
- Since revision 3.2 (#983 T4), it also holds frame resources (core §7.7), which carry the 73
  readings in packed records across the radios, each field a
  [`telemetry.v2`](../../../spec/profiles/telemetry/v2.md) series. Since revision 3.3 (#991) three
  of them carry all 73 in one record at zenoh-modem's own ranges
  ([`schemas/modem-readings.json`](schemas/modem-readings.json)), one per horizon a medium can
  carry, and 3.2's nine are deprecated. "Frames (12)", below, lists them, where each range comes
  from, and what each face spends.
- Since revision 3.3, the 7 states are also packed records, under `packed/`
  ([`schemas/modem-states.json`](schemas/modem-states.json)), beside the JSON documents. "State",
  below, says what each keeps.
- The configuration plane's 8 gated entries are `config.v1`, bound in
  [`config.v1-binding.md`](config.v1-binding.md). That file also explains why the far side of a
  radio can arm a reach change today but never keep it, and gives the zk2 grants that fix it.
- Since #985 G4, the SX1262's configuration is a contract of its own under
  [`config.v2`](../../../spec/profiles/config/v2.md):
  [`config/sx1262_config.v1.toml`](config/sx1262_config.v1.toml), its types in
  [`config/schemas/sx1262_config.json`](config/schemas/sx1262_config.json). One packed record
  replaces the view and the status. [`config.v2-binding.md`](config.v2-binding.md) binds it, and
  gives the far-side reach change at one radio frame a step. A backend whose configuration holds
  text stays on `config.v1` until a packed format carries text.
- The radio faces, the rules a generator would emit, and the declaration cost of zk2 keys against
  v1's are in [`link.md`](link.md), a sketch now superseded by the profile
  [`link.v1`](../../../spec/profiles/link/v1.md) (#769). Since #988 L5 the routers run
  [`link.v2`](../../../spec/profiles/link/v2.md) policies, one router per medium, in
  [`faces/`](faces/): RF, LoRa (a policy per spreading factor, SF7, SF9 and SF12), and SBD.
  `link.md`'s first note gives each face's transport, the frames that cross it and its plan's
  warnings.
- What `modem-contract` becomes is in [`contract-crate.md`](contract-crate.md).
- How to move zenoh-modem onto zk2c, step by step, against what zenkey's `main` provides today,
  is [`MIGRATION.md`](MIGRATION.md): what is landed, drafted and missing, the order, the
  routers, the ground side, and what is still to decide.
- [`catalog.json`](catalog.json) is the deployment catalog ([core §8.6](../../../spec/core.md#86-the-deployment-catalog)),
  provisioned to rf0's host and to the ground. It indexes `modem.v3`, `config.v1` and
  `health.v1` at their current revisions (`modem.v3` 3.3 at index 8, catalog `e3a529fe`), and
  `sx1262_config.v1` (index 7, #985 G4), and dates times from 2026-01-01. A new revision goes in
  with `zk2 catalog add`, which writes the catalog's successor.

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
| interfaces | `modem.v3` on every device service; where the device has a configuration plane (v1 `capability:config`), its backend's config contract under `config.v2` (`sx1262_config.v1` on an SX1262, [binding](config.v2-binding.md)), or `config.v1` while that configuration holds text; `health.v1` ([`spec/profiles/health/health.v1.toml`](../../../spec/profiles/health/health.v1.toml), the profile [`health.v1`](../../../spec/profiles/health/v1.md)) recommended, though none of it crosses the RF face ([`health.v1`](../../../spec/profiles/health/v1.md) §2.8), its status derived from `oper` (`up` → ok; `down` and recoverable → degraded; `down` and not recoverable → failed) |
| presence | per device service: an instance token, plus one interface token per interface the device exposes something of. A netdev holds no SDU resource, but it still implements `modem.v3`, because its `oper`, `identity` and `security` are always exposed. |
| continuity epoch | a new instance id whenever a device's counters reset: re-enumeration, ifindex change, restart; a frame resource's is its run, in band, which a new instance renews (`telemetry.v2` §2.19) |
| ownership | every resource `exclusive`; both operations `fanout = "forbidden"`; config writes serialized per device by the server, which is v1's single-writer rule |
| schema kind | **jsonschema**, Rust-first: schemars output committed ([crate](contract-crate.md) §2) |

Example keys, for host `h-3fa9c2d41b7e` and device `rf0`:

```text
zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/sdu/dropped_total           a link-exposed counter
zk2/h-3fa9c2d41b7e/rf0/modem.v3/state/sdu/endpoint/peer-b          a templated state, cardinality 64
zk2/h-3fa9c2d41b7e/rf0/modem.v3/events/oper_change/01j9x3k7q2m4n5p6r7s8t9v0wx
zk2/h-3fa9c2d41b7e/sat0/modem.v3/@op/action/mailbox_check
zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/set/radio                      a reach group: {token, apply_at} first
zk2/h-3fa9c2d41b7e/rf0/config.v1/state/view                         the echo; GET is the read
zk2/h-c2a95e41d7f0/lora0/sx1262_config.v1/@op/set/radio             config.v2: the record, pending, before the apply
zk2/h-c2a95e41d7f0/lora0/sx1262_config.v1/state/config              config.v2's record, one packed state; GET is the read
zk2/h-3fa9c2d41b7e/rf0/@zk/alive/modem.v3/<instance>/<fp16>         never across a constrained face
zk2/@zk/contract/modem.v3/<sha256>
zk2/*/*/@zk/alive/config.v1/**                                     "which devices can be configured"
```

## Decisions recorded here

- **v1's events are zk2 events, all five:** `kind = "event"`, one key per occurrence,
  `…/events/<template>/<ulid>` (draft 1, D3). Each exists so that a reader arriving after the
  fact still learns what happened (v1 RFC 04 §1.3), which is the case r3.2 called
  occurrence-keyed and r3.3 made the `events` kind. A reliable stream would lose them to a
  reader that was not connected.
  - The reason and retention for each are on each resource in `modem.v3.toml`.
  - `device_fault` is edge-triggered raised/cleared, which is `alarms.v1`'s shape
    ([`spec/profiles/alarms/v1.md`](../../../spec/profiles/alarms/v1.md), Appendix B): a later
    option, not taken in a 1:1 mapping.
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
- **Exposure defaults to `host`.** Until revision 3.1, only the four v1 `link` streams carried
  `link.exposure = "link"`. Since 3.2 they are `host` again, written out, and only frame resources
  are `link`: since 3.3, `frames/fast`, `frames/slow` and `frames/paid`, 3.2's nine being deprecated
  and `host` ([link](link.md), the note at its top).

## Every v1 entry, mapped

Generated from `modem.toml` v2.14 against `modem.v3.toml`. The zk2 column is relative to
`zk2/<host-id>/<device>/`.

### Telemetry (73) → `modem.v3` streams

Key: `zk2/<host-id>/<device>/modem.v3/stream/<resource>`; every one keeps v1's `sampled` QoS (`priority = "data_low"`) and carries `telemetry.kind` (and `telemetry.unit` where v1 had one). The four v1 `link` streams are `host` since revision 3.2: their readings cross in a frame (below).

| # | v1 `telemetry/modem/…` | Gate (`capability:`) | zk2 resource | Type · kind · unit |
|---|---|---|---|---|
| 1 | `device/{device}/sdu/tx_sdus_total` | `sdu` | `sdu/tx_sdus_total` | `Counter` · counter |
| 2 | `device/{device}/sdu/tx_bytes_total` | `sdu` | `sdu/tx_bytes_total` | `Counter` · counter · `By` |
| 3 | `device/{device}/sdu/rx_sdus_total` | `sdu` | `sdu/rx_sdus_total` | `Counter` · counter |
| 4 | `device/{device}/sdu/rx_bytes_total` | `sdu` | `sdu/rx_bytes_total` | `Counter` · counter · `By` |
| 5 | `device/{device}/sdu/dropped_total` | `sdu` | `sdu/dropped_total` | `Counter` · counter · **`link.exposure = "link"`** until 3.1, `host` since 3.2 |
| 6 | `device/{device}/sdu/tx_failed_total` | `sdu` | `sdu/tx_failed_total` | `Counter` · counter |
| 7 | `device/{device}/sdu/rx_unroutable_total` | `sdu` | `sdu/rx_unroutable_total` | `Counter` · counter |
| 8 | `device/{device}/sdu/rx_oversize_total` | `sdu` | `sdu/rx_oversize_total` | `Counter` · counter |
| 9 | `device/{device}/sdu/tx_oversize_total` | `sdu` | `sdu/tx_oversize_total` | `Counter` · counter |
| 10 | `device/{device}/sched/dropped_total` | `sdu` | `sched/dropped_total` | `Counter` · counter |
| 11 | `device/{device}/sched/stale_total` | `sdu` | `sched/stale_total` | `Counter` · counter |
| 12 | `device/{device}/sched/peerless_total` | `sdu` | `sched/peerless_total` | `Counter` · counter |
| 13 | `device/{device}/sched/queue_depth` | `sdu` | `sched/queue_depth` | `Gauge` · gauge · **`link.exposure = "link"`** until 3.1, `host` since 3.2 |
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
| 47 | `device/{device}/air/duty_cycle_available` | `duty_cycle` | `air/duty_cycle_available` | `Gauge` · gauge · **`link.exposure = "link"`** until 3.1, `host` since 3.2 |
| 48 | `device/{device}/air/duty_cycle_wait_ms` | `duty_cycle` | `air/duty_cycle_wait_ms` | `Gauge` · gauge · `ms` · **`link.exposure = "link"`** until 3.1, `host` since 3.2 |
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

### Frames (12) → `modem.v3` frame resources, since revisions 3.2 and 3.3

No v1 entry: v1 had no frame. Since revision 3.2 (#983 T4), `modem.v3` lists
[`telemetry.v2`](../../../spec/profiles/telemetry/v2.md) in place of `telemetry.v1`, and carries the
73 readings above in packed records as well (core §7.6, §7.7). Since revision 3.3 (#991) they cross
in three frame resources over one record, `Readings`
([`schemas/modem-readings.json`](schemas/modem-readings.json)), at zenoh-modem's own ranges (below).
Each of the three:
- is a plain stream, `priority = "data"`, exposed `link`;
- holds one record a frame, its time measured, so its cadence is its owner's configuration: a full
  record, or a delta record of a bit per reading and then the readings that moved;
- is optional, gated `config:device.frames.<name>`, a `[[device]]`'s `frames` table in
  zenoh-modem's file: a device publishes the frame its face lists, at the face's cadence, and lists
  the others unavailable;
- declares `telemetry.series`, each field a mirror of its JSON stream, which lends the series its
  kind and unit. The series is the frame key and the field, never the mirrored stream
  (`telemetry.v2` §2.17), and its epoch is the frame's run (§2.19);
- declares a horizon its medium can carry, so it is liveness evidence across its face (`link.v1`
  §2.12, `link.v2` §2.12).

| Frame resource | Horizon | Full record | Deadbands | Faces, at their cadence |
|---|---:|---|---|---|
| `frames/fast` | 300 s | at least every 15 | RSSI and SNR (1 dB), the scheduler's queue (4) and lane backlog (1,024 B), the qdisc backlog (4,096 B, 4 packets), each in at least 1 record of 5 | RF and LoRa SF7, 60 s; LoRa SF9, 120 s, a full record every 15 minutes (`keyframe_s`) |
| `frames/slow` | 1,800 s | at least every 4 | the same, in at least 1 record of 4 | LoRa SF12, 900 s: a full record hourly |
| `frames/paid` | 3,600 s | every frame | none | SBD, 1,800 s: one paid message |

A full record with every reading present is 1,542 bits, 199 B of body: 207 B of payload after an
envelope with a 32-bit run, 205 B with a 16-bit run. That is `frames/paid`'s largest frame
(`link.v1` §2.8), a full record every frame. `frames/fast`'s and `frames/slow`'s may hold a delta
record that includes every reading, a bit per reading longer (`link.v1` 0.16): 216 B, within RF's
220 B batch and LoRa's 224 B, and above SF9's 117 B alone (`frame_over_batch`). A device writes a
reading it lacks absent, one bit, so rf0's full record, 18 readings present, is 66 B of payload,
and a delta 39 B.

**3.2's nine, deprecated.** `frames/link`, the seven group frames (`sdu`, `sched`, `air`, `layer`,
`if`, `radio`, `bearer`) and `frames/message`, typed in
[`schemas/modem-frames.json`](schemas/modem-frames.json), stay until the next major, `deprecated` and
`host` since 3.3:
- their ranges and widths were illustrative, and a frame's record has no extension marker, so any
  change to its layout is breaking (core §9.8: `bound_changed`, `width_changed`);
- one frame per group spent about 15 B of envelope, time and put on each of eight frames a minute:
  401 KB a day on RF with all 73 present, and 164 % and 241 % of LoRa's duty at SF9 and SF12
  (below).

An owner lists them unavailable, and no face lets them cross.

#### The ranges, from zenoh-modem's code

Every field is optional, as `modem_device::Report` is all `Option` (`modem-device/src/lib.rs:674`):
absent is not zero. Where zenoh-modem's code bounds a reading, the range is the code's. Where it does
not, the bound is physical, and *chosen*. `out_of_range` is reserved wherever a value past the bound
can occur, and the owner then writes that name, never a clamped value (core §7.6). zenoh-modem was
read at `d8c62df`.

| Reading | Packed | Bits | From |
|---|---|---:|---|
| `sched/queue_depth` | int 0–1,022, `out_of_range` | 10 | code: the lanes' room, `DEFAULT_LANE_DEPTH` + 1 each (`modem-driver/src/sched.rs:49, 556`); *chosen*: 64 endpoints (the contract's cardinality) × 8 lanes × 2 = 1,024 |
| `sched/lane_backlog_bytes` | int 0–1,048,574, `out_of_range` | 20 | code: what each lane's Zenoh wrote and the driver has not read (`lanes.rs:613`), about 4.6 KB under the shipped `so_sndbuf=1` (`TCP_FLOOR_BYTES`); *chosen*: four lanes at the kernel's default 212,992 B send buffer |
| `sched/lane_backlog_drain_ms` | fixed 0–3,600,000 at 100, `out_of_range` | 16 | code: the backlog × `sdu_time` / batch (`modem-mgmt/src/telemetry.rs:112`); *chosen*: an hour, at 100 ms |
| `layer/aead/send_key_id` | int 0–15 | 4 | code: `key_id` is 0–15 (`layer-aead/src/config.rs:58`), the header's nibble |
| `air/duty_cycle_available` | int 0–1 | 1 | code: `DutyCycle::available`, a bool |
| `air/duty_cycle_wait_ms` | fixed 0–3,600,000 at 100, `out_of_range` | 16 | code: the SX1262 waits within its duty period, EU868's 3,600 s (`DutyBudget::eu868`), SBD within `retry_max_ms`, 180 s by default; *chosen*: 100 ms. A `period_s` above an hour writes `out_of_range` |
| `radio/signal_bars` | int 0–5 | 3 | code: `Report::signal_bars`, the terminal's own 0–5 (`AT+CSQF`) |
| `if/qdisc_backlog_bytes` | int 0–16,777,214, `out_of_range` | 24 | code: the sum of the kernel's u32 backlogs (`netdev-modem/src/lib.rs:160`); *chosen*: 16 MiB, half fq_codel's default `memory_limit` |
| `if/qdisc_backlog_pkts` | int 0–65,534, `out_of_range` | 16 | code: the sum of u32 `qlen`s; *chosen*: above fq_codel's default `limit` of 10,240 packets |
| `radio/rssi` | fixed −160–0 at 0.5, `out_of_range` | 9 | code: the SX1262's −p/2 dBm, −127.5–0 at 0.5 (`sx1262-medium/src/chip.rs:400`), and ModemManager's double, unbounded (`mm-modem/src/lib.rs:284`); *chosen*: −160 dBm, below any receiver's floor |
| `radio/rsrp` | fixed −156–−31 at 1, `out_of_range` | 7 | ModemManager's double, unbounded; *chosen*: 3GPP's reporting ranges, LTE −140–−44 (TS 36.133), NR −156–−31 (TS 38.133), at 1 dB |
| `radio/rsrq` | fixed −43–20 at 0.5, `out_of_range` | 7 | ModemManager's double, unbounded; *chosen*: LTE −19.5–−3 (extended −34–2.5), NR −43–20, at 0.5 dB |
| `radio/snr` | fixed −32–40 at 0.25, `out_of_range` | 9 | code: the SX1262's i8 / 4 dB, −32–31.75 at 0.25 (`chip.rs:401`), and ModemManager's double; *chosen*: NR's SINR, to 40 dB |
| `radio/error_rate` | fixed 0–100 at 0.01 | 14 | ModemManager's percent, unbounded; *chosen*: 0–100 %, at 0.01, which holds GSM's RXQUAL classes (0.14 %, 0.28 %, …) |
| `link/rx_rate`, `tx_rate`, `rx_max_rate`, `tx_max_rate` | fixed 0–10,000,000,000 at 100, `out_of_range` | 27 each | code: DLEP's 64-bit bit/s (`dlep-modem/src/session.rs:84`); *chosen*: 10 Gbit/s, at 100 bit/s, so a 9,600 or 4,800 bit/s radio reads exact |
| `link/latency` | fixed 0–60,000,000 at 100, `out_of_range` | 20 | code: DLEP's 64-bit µs; *chosen*: 60 s, at 100 µs |
| `link/resources`, `rx_quality`, `tx_quality` | int 0–100, `out_of_range` | 7 each | code: DLEP's one-octet percent, 0–100 (RFC 8175 §13.10–13.12; `session.rs:94`) |

The 22 gauges take 305 bits. The 51 counters are each the code's u64 (`Report`, `Metrics`,
`NetdevStats`, the layers' `Counter`s), carried modulo 2^w, a declared loss (`telemetry.v2`
§2.20). Each width holds its counters' worst rate between two frames of the slowest cadence a face
lists, 900 s, within the serial window 2^(w−1):

| Counters | Width | Worst rate | Window |
|---|---:|---|---:|
| the 4 byte counters, `sdu/tx_bytes_total`, `sdu/rx_bytes_total`, `air/bytes_wasted_total` and `layer/compress/bytes_saved_total` | 24 | 2,734 B/s: the fastest SDU medium in the tree, an SX1262 at SF7 and 500 kHz, 21.9 kbit/s (`sx1262-medium/src/lib.rs:164`) | 51 min |
| `air/airtime_ms_total` | 24 | 1,000 ms/s: occupancy cannot outrun the clock | 2.3 h |
| the other 37 of the SDU plane and its layers: SDUs, batches, frames, tiles and refusals | 20 | 300/s: one a byte at RF's 2,400 bit/s, more than any SX1262 setting's frames | 29 min |
| `if/in_octets` and `if/out_octets` | 40 | 1.25 GB/s: a 10 Gbit/s bearer | 7.3 min; 73 min at 1 Gbit/s |
| the 7 kernel packet, error and discard counters | 32 | 14.88 Mpps: 10 GbE at its smallest frame | 2.4 min; 24 min at 1 GbE |

The counters take 1,164 bits. A pair further apart than its window reads `wrap_or_decrease`, never a
wrong rate (`telemetry.v2` §2.20). Across a 3,600 s outage at RF's rates every SDU-plane counter still
reads. The kernel's are a netdev's, which no shipped face carries; `udp-medium` is test-only, and the
simulated medium's `bitrate_bps` is a lab's to raise.

#### What each face spends

The wire model is the report's Appendix A (`docs/zk2/constrained-overhead-review.md`), read on each
face's policy: a put on a scope its far reader declared, priority `data` (no QoS extension), a 16-bit
run, the face's 16-bit sequence and batch, the lean stack (16 B a SDU), Semtech's time on air, and
SF12's 51 B tiles. **All 73** is every reading present, 16 counters and 8 gauges moving every frame
(the report's W1). **Its own** is the face's device, its readings as zenoh-modem's code fills them.

| Face | Frame, cadence | All 73 | Its own | Against |
|---|---|---:|---:|---|
| RF, rf0 | `fast`, 60 s | 177 KB a day | 92 KB (18 readings) | 25.9 MB a day: 0.7 %. The report's model: 200 KB, with its 203 B keyframe |
| LoRa SF7, lora0 | `fast`, 60 s | 302 s + 74 s of keep-alives = 376 s | 174 s + 74 s = 248 s (22 readings) | 864 s of air a day: 43 % and 29 % |
| LoRa SF9 | `fast`, 120 s, full every 15 min | 534 s | 281 s | 62 % and 33 % |
| LoRa SF12 | `slow`, 900 s, full hourly | 785 s | 434 s (28, the fragment layer's among them) | 91 % and 50 % |
| SBD, sat0 | `paid`, 1,800 s | 48 messages up, each one frame of 229 B, and 48 keep-alives down | the same, each 94 B | 96 messages a day: as idle |

The downlink's keep-alives are the ground's air: 148 s, 119 s and 142 s at SF7, SF9 and SF12. Under
3.2's nine frames the same readings cost 401 KB a day on RF, and 736 s, 1,420 s and 2,081 s of air at
SF7, SF9 and SF12. SF9's and SF12's faces now run a lease of four frame intervals, 480 s and 3,600 s,
so their frames replace every uplink keep-alive ([link](link.md), the note at its top).

The 73 JSON streams stay, at 5 s on the host bus, for host tools: one reading, two series, which
a tool labels apart ("`sdu/dropped_total` (`frames/fast`)").

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

**Since revision 3.3, the 7 again, packed** (#991). A resource's type kind cannot change within the
major (core §9.8, `type_kind_changed`), so the JSON states stay, and each has a packed twin under
`packed/`, typed in [`schemas/modem-states.json`](schemas/modem-states.json): optional, with its
twin's gate, `freshness.ttl_s = 60`, `host`, and confirmed by the same heartbeat, one more
collection each (`freshness.v2` §2.4). A packed field holds an enum, a number, a bit or a time
(core §7.6), so each record keeps what is not text, and the text stays in the JSON state:

| Packed state | Record, bits | What it keeps | What stays JSON |
|---|---|---|---|
| `packed/oper` | `OperRecord`, 83 | `status` and `admin` (enums with an escape), `recoverable`, `last_change` (a time to the ms), `uptime_s`, `clock_synced` | `reason`, the device's words; `boot_id`, since the record's run tells driver starts apart |
| `packed/identity` | `IdentityRecord`, 52 | the IMEI's 15 digits | the RFC 8348 names and revisions, all text |
| `packed/security` | `SecurityRecord`, 10 | `provider` (`none`, `zenoh_modem_aead`, an escape), `scope`, each of the four guarantees as a bit, and whether a principal is named | the principal's name, the note |
| `packed/radio` | `RadioRecord`, 9 | `registration`, `access_technology` (ModemManager's names, `-` as `_`, `5gnr` as `nr5g`) | nothing |
| `packed/sdu/link` | `LinkRecord`, 145 | the capabilities, flattened, framing's ARQ as an enum and its attempts; how many lanes, and how many connected | each lane (`@op/sdu/lanes`'s), `ttl_s` (the contract's) |
| `packed/sdu/endpoint/{endpoint}` | `EndpointRecord`, 17 | how many lanes reach the remote, and how many are connected | each lane's name |
| `packed/destination/{destination}` | `DestinationRecord`, 158 | the eight DLEP metrics, at `Readings`' ranges | the addresses |

Each record ends with an extension marker, so a later minor appends fields. Put with a 16-bit run,
each is 8 to 21 B, against 111 to 585 B of JSON (`modem-contract/src/documents.rs`), so a GET across
a face by grant (`link.v1` §2.9), and #977's snapshots, carry a record's size. None is exposed: a
state exposed `link` crosses every face that exposes, since a `link.v2` face lists frames only, and
its heartbeat with it every 30 s, which SBD's messages and LoRa's duty cannot afford.

### Events (5) → events, one key per occurrence

| # | v1 `events/modem/…` | Gate | v1 rate | zk2 | Retention · bound |
|---|---|---|---|---|---|
| 1 | `device/{device}/oper_change/{event_id}` | — | `rare` | `modem.v3/events/oper_change/<ulid>` | `7d` · 168 |
| 2 | `device/{device}/device_fault/{event_id}` | — | `low` | `modem.v3/events/device_fault/<ulid>` | `1d` · 1,440 |
| 3 | `device/{device}/identity_change/{event_id}` | — | `rare` | `modem.v3/events/identity_change/<ulid>` | `7d` · 168 |
| 4 | `device/{device}/config_change/{event_id}` | `config` | `burst(600/h)` | `config.v1/stream/changes/{occurrence}` | `7d` · 100,800 (worst case) |
| 5 | `device/{device}/action/{event_id}` | `actions` | `burst(600/h)` | `modem.v3/events/action_event/<ulid>` | `1d` · 14,400 |

Revision 3.3 changes none of them (#991). Each template is a literal, so a far reader subscribed to
`…/events/<template>/*` declares `…/events/<template>`, and each put carries its ULID chunk alone:
the shortest suffix core §8.5 allows (#980 C1b). None is exposed, so none crosses a face.

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
| `link_change/{event_id}` | `device/{device}/oper_change/{event_id}` | not carried; the successor is `modem.v3/events/oper_change/<ulid>` |
| `device_fault/{event_id}` | `device/{device}/device_fault/{event_id}` | not carried; the successor is `modem.v3/events/device_fault/<ulid>` |
| `{device}/lanes` | `device/{device}/sdu/lanes` | not carried; the successor is `modem.v3/@op/sdu/lanes` |

**Count.** 73 + 9 + 5 + 10 entries:
- 86 → `modem.v3`, 80 of them gated;
- 8 → `config.v1`, all gated `config`, so 80 + 8 = v1's 88;
- 3 deleted (`sensor`, `describe`, `introspect`).

Plus 3 errors, 2 tokens and 26 retired paths. Nothing in `modem.toml` is left unmapped. The 12
frame resources of revisions 3.2 and 3.3 map no v1 entry: they carry the 73 readings again, packed.
Nor do 3.3's 7 packed states: they carry the 7 states again.

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
   - *Status:* [`telemetry.v1`](../../../spec/profiles/telemetry/v1.md) 0.1 states both: a
     decrease with no epoch evidence reads as a reset (§2.10, §2.14), and a bare counter carries its
     epoch in an attachment typed `telemetry.v1.Epoch` (§2.4, §2.9), which `link.v1` §2.13 cites.
     `modem.v3` declares none yet: adding it to `sdu/dropped_total` is a revision, review under
     core §9.8.
   - *Status since `modem.v3` 3.2 and the `link.v2` faces (#983 T4, #988 L5):* **closed on every
     face.** No JSON stream crosses. Each counter crosses as a frame's field: since 3.3 (#991),
     `frames/fast` over RF, SF7 and SF9, `frames/slow` at SF12, `frames/paid` over SBD. Its epoch
     is the frame's run, in band ([`telemetry.v2`](../../../spec/profiles/telemetry/v2.md) §2.19),
     at no byte, and [`link.v2`](../../../spec/profiles/link/v2.md) §2.13 reads it so across a
     face. A decrease within one run is not a reset there: at its width, 20 to 40 bits, it is
     `wrap_or_decrease` (§2.20). The JSON stream on the host bus reads as before, by presence.
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
   - *Status since #985 G4:* **closed for the SX1262.** Under `config.v2` every operation answers
     one type, the record, which a reach `set` returns with the change pending and its `apply_at`
     ([binding](config.v2-binding.md) §5). No union is needed.
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
   - *Status since #985 G4:* **closed for the SX1262, open where a configuration holds text.**
     `config.v2`'s record holds the values, the pending and the last change, `floor_s` and a
     `writable` bit per group: 30 to 50 B with its envelope, one frame, put on a change and
     confirmed by the heartbeat. Every control operation answers it
     ([binding](config.v2-binding.md) §7). `fake-medium` and `udp-medium` can follow at 0.1.
     `sbd-medium`, `netdev-modem`, `dlep-modem` and `mm-modem` cannot: `config.v2` 0.1 has `int`
     and `bool` alone, and their `imei`, `port`, `interface`, `apn` and `pin` are text, which
     waits for #979's B3. They stay on `config.v1`.
10. **Per-face exposure fails on a node with two radios.** Device-as-service gives per-device keys,
    but the face selector cannot tell rf0's lane from sat0's: `unixsock-stream` reports no
    interfaces. RF's carve-out and SBD's "nothing" cannot both hold on one router.
    - *Evidence:* zenoh-modem `architecture.md:274, 902-906`; r3.2 l.101-105.
    - *Correction:* `link.v1` states that face policy is per router transport. A two-radio node
      uses `zids` subjects per lane peer (unauthenticated, but the peer is configured) or a router
      per radio. S3 and S14 test `zids` subjects.
    - *Status since #988 L5:* **narrowed.** A `link.v2` policy states the router's transport,
      which zenoh reads once per router, so a router per medium is the rule (`link.v2` §2.16):
      RF's 240 s lease and SBD's 3,600 s cannot share one. [`faces/`](faces/) gives each medium
      its router, each carrying one device. A node with two radios stays open: a face's `frames`
      names an interface's frame resource, not a service's, so a router that also carried the
      other device's traffic would let its listed frames across too.
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
    - *Status since `modem.v3` 3.2:* **narrowed, still open.** `sdu/`, `sched/` and `air/` hold no
      crossing resource now, so each is denied whole. Only `frames/` mixes: since 3.3 it holds 3.2's
      nine, deprecated and `host`, and the three that cross, each face listing one and denying the
      others by name (`link.v2` §2.7). A resource a later minor added there would cross until the
      plan is derived again.
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
    - *Status since `modem.v3` 3.3 and the `link.v2` faces (#991):* **closed on every face.** Each
      face carries a frame whose horizon its cadence keeps, so each plan lists it as liveness
      evidence (`link.v1` §2.12): `frames/fast`, 300 s, on RF and SF7 every minute and SF9 every
      2; `frames/slow`, 1,800 s, at SF12 every 15 minutes; `frames/paid`, 3,600 s, over SBD every
      30. Under 3.2, SF12's frames/link drew `interval_over_horizon` and `horizon_under_face`, and
      SBD's frames/message declared no horizon.
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
    - *Status since #985 G4:* **stands.** `config.v2` takes duplication: each config contract
      carries its own derived `Control` and `Error`, which its lint checks against the derivation.
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
    - *Status since #988 L5:* **open.** A `link.v2` face lists which of a contract's exposed frames
      cross it (`link.v2` §2.7): exposure chosen by the deployment, but per face, among frames the
      contract already exposes. It is not per principal, and reaches no standard profile's
      resource: `config.v1`'s view still crosses only as a GET, by grant (`link.v1` §2.9).
    - *Status since #985 G4:* **closed for configuration.** Under `config.v2` the config contract is
      the adopter's own, so `sx1262_config.v1` exposes its record `link`, and it crosses RF and
      LoRa as a push ([binding](config.v2-binding.md) §11). `config.v1`'s view still cannot be.
21. **`claimed_source` has no zk2 source.** O7 carries `{actor, request_id}` only, and zenoh's
    `SourceInfo` is unstable. (Minor.)
    - *Evidence:* `zenkey/src/config.rs:780-784`; r3.2 l.384, §3.8.
    - *Correction:* keep it optional, filled from the runtime's best effort and documented as
      unauthenticated.
    - *Status since #985 G4:* **unchanged.** `config.v2`'s change event keeps `claimed_source`
      optional, and stays on the host.
22. **"System = host" holds only per application.** v1 salts the host id per application. RFC 06
    §1 makes the salt an application constant, so one machine mints three different `h-<12hex>`
    ids for the three adopters, and `zk2/<host>/**` would show one application's services.
    - *Evidence:* [`rfcs/06-identity.md:49-58`](https://git.marcpardo.eu/marcpardo/zenkey/src/branch/v1/rfcs/06-identity.md#L49-L58) (`v1` branch); zenoh-modem `modem-contract/src/lib.rs:50-53`
      (`zenoh-modem-host-id-v1`); ZenSight's `zensight-host-id-v1`; tcgui's `tcgui-host-id-v1`;
      r3.2 l.95-98 ("per-host views … carry over position for position").
    - *Correction:* `hostid.v1` fixes one zk2-wide salt, so the system chunk agrees across
      adopters, while v1 origins keep their salts. The re-key costs nothing extra at the major
      boundary, but any far side that binds statically must be re-provisioned with the new name.
      zenoh-modem's RF peers do.

## v1 findings (zenoh-modem's own, made while mapping)

These are not zk2 gaps. They are drift between zenoh-modem's v1 registry, its implementation and
RFC 05 §5.1, and they are listed so that the port fixes them on purpose. Details are in
[`config.v1-binding.md`](config.v1-binding.md) §10 and [`link.md`](link.md) §3. For the SX1262,
`config.v2` closes the first five ([`config.v2-binding.md`](config.v2-binding.md) §10).
- `set` declares the wrong reply type for reach groups.
- The reach reply carries a `deadline` field that no schema describes.
- `persist` replies `{}` rather than the read-back.
- RFC v1.50's joining `set` and `last_change` are not implemented.
- `APPLY_AFTER` is a fixed 1 s, which is too short for a 300 bit/s reply.
- The SBD confirm floor protects a path that the shipped faces close.
- The `ops` principal reaches every `@rpc` read.
- v1's downsampling rules share one slot across devices.
