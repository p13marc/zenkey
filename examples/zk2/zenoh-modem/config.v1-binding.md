# `config.v1`, bound to zenoh-modem (#623)

zenoh-modem's configuration plane is zenkey v1's RFC 05 §5.1, implemented whole by
`modem-mgmt/src/config.rs`, with each device declaring its own schema (`modem_device::Configure`).
r3.2 claims `config.v1` takes those semantics whole (`docs/zk2/architecture.md:133`):
- reach groups;
- `{token, apply_at}` before the apply;
- per-device confirm floors;
- separately grantable confirm and persist;
- the echo visible over the face;
- desired state never for reach.

This file binds it. It sketches `config.v1`'s contract as zenoh-modem needs it, maps every v1
fact onto it, then answers the issue's question: why the far side of a radio can **arm** a reach
change today but can **neither confirm it nor see the echo**, and which zk2 grants fix that.

**Verdict:** the semantics carry over whole, and the zk2 keys make each act separately grantable
**per device**, which v1 could not do. Three parts need the profile to grow:
- a union reply for `set` (README gap 8);
- a compact status the radio can afford (gap 9);
- the confirm floor, visible before the call (gap 9).

## 1. Which service, which interface

Every device service whose device declares a configuration plane implements `config.v1` beside
`modem.v3`: v1's `capability:config`. A device without one (the UDP lab medium aside, every
shipped backend has one) simply does not implement it. It then holds no `config.v1` interface
token, so "who can be configured" is the selector `zk2/*/*/@zk/alive/config.v1/**`. The gate
becomes interface presence (README gap 7 records what that costs the contract).

## 2. The keys

For device `rf0` on host `h-3fa9c2d41b7e` (system = host id, `hostid.v1`):

| v1 (`v1/<origin>/…`) | zk2 | Kind | Grant |
|---|---|---|---|
| `@rpc/modem/config/rf0` (read) | `zk2/h-3fa9c2d41b7e/rf0/config.v1/state/view` | state GET | **deleted as an operation**: a GET on the state is the read (r3 §3.6 S2) |
| `@rpc/modem/config/rf0/{group}/set` | `zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/set/{group}` | `@op` | per group: `…/@op/set/duty` and `…/@op/set/radio` are distinct grants |
| `@rpc/modem/config/rf0/confirm` | `zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/confirm` | `@op` | its own key |
| `@rpc/modem/config/rf0/cancel` | `zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/cancel` | `@op` | its own key |
| `@rpc/modem/config/rf0/extend` | `zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/extend` | `@op` | its own key |
| `@rpc/modem/config/rf0/persist` | `zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/persist` | `@op` | its own key, so a change can be allowed while surviving a restart is denied |
| `state/modem/config/rf0` (the echo) | `zk2/h-3fa9c2d41b7e/rf0/config.v1/state/view` | state | GET and/or subscribe |
| — (proposed, gap 9) | `zk2/h-3fa9c2d41b7e/rf0/config.v1/state/status` | state | the small document a constrained face can afford |
| `events/modem/device/rf0/config_change/{ulid}` | `zk2/h-3fa9c2d41b7e/rf0/config.v1/stream/changes/{occurrence}` | occurrence-keyed stream | host only |

The device is the service, so it is in the path at position 3: `sat0`'s keys differ only there.
v1 had the device as a chunk too (`config/{device}/…`), but its generator worked from the
registry's templates and could only grant `config/*/*/set` (zenoh-modem `architecture.md:1258-1262`,
`grant_cannot_carve`). The zk2 generator reads the deployment's services, so it can grant `rf0`
and withhold `sat0` (§11).

## 3. `config.v1`'s contract, as zenoh-modem needs it

The profile owns this contract (r3 §3.12). Here is the shape zenoh-modem binds to. The types are
zenkey 0.11's `zenkey::config` (`zenkey/src/config.rs`), carried whole, plus the additions the
gaps propose (marked).

```toml
# config.v1: commit-confirm configuration of one service (zenkey v1 RFC 05 §5.1, whole).
[interface]
name    = "config"
major   = 1
minor   = 0
summary = "The configuration of one service: groups with classes, confirmed commit, persistence"
uses    = ["freshness.v1"]

[schemas]
jsonschema = ["schemas/config.json"]          # the profile's; zenkey::config's shapes

[resources.view]                               # v1 echo + read: transition QoS = the zk2 state default
kind        = "state"
type        = "json:ConfigView"
annotations = { "freshness.ttl_s" = 60 }

[resources.status]                             # PROPOSED (gap 9): what a constrained face can afford
kind        = "state"
type        = "json:ConfigStatus"
annotations = { "freshness.ttl_s" = 60 }

[resources."set/{group}"]
kind        = "operation"
params      = { group = "string" }
cardinality = 16
request     = "json:ConfigChange"
response    = "json:SetReply"                  # PROPOSED union (gap 8): ConfigView | PendingReply
error       = "json:ConfigError"
fanout      = "forbidden"

[resources.confirm]
kind     = "operation"
request  = "json:ControlRequest"
response = "json:ConfigView"
error    = "json:ConfigError"

[resources.cancel]
kind     = "operation"
request  = "json:ControlRequest"
response = "json:ConfigView"
error    = "json:ConfigError"

[resources.extend]
kind     = "operation"
request  = "json:ControlRequest"               # carries confirm_s, the new window from now
response = "json:ConfigView"
error    = "json:ConfigError"

[resources.persist]
kind     = "operation"
request  = "json:ControlRequest"               # the pending token, or view.last_change.token
response = "json:ConfigView"                   # RFC v1.47; zenoh-modem's v1 `Ack` retires (§9)
error    = "json:ConfigError"

[resources."changes/{occurrence}"]             # RFC 6470's netconf-config-change
kind        = "stream"
type        = "json:ConfigChangeEvent"
reliability = "reliable"
congestion  = "block"
retention   = "7d"
cardinality = 100800                           # v1 burst(600/h) x 7d: the worst case (gap 2)
```

None of the operations is `idempotent`: v1 marks none of them, and a confirm retried after a lost
reply answers `not_found`. Retry safety for `set` is per request, through `idempotency_key`: the
first answer comes back for 300 s (`modem-mgmt/src/config.rs:41`). On a 2,400 bit/s link a lost
reply is the common case, not the exception.

| Type | Fields | Source |
|---|---|---|
| `ConfigView` | `resource`, `revision`, `pending?{token, deadline?, groups}`, `last_change?{token, groups}`, `groups[]{name, class, description, parameters[]{name, kind (bool / integer{min?, max?, unit?} / text), sensitive?, description, value?, source?, startup?}}` | `config.rs:640-654`; **add** `confirm_floor_s` and, per group, `writable` (gap 9) |
| `ConfigChange` | `values{name → ParamValue}`, `expected_revision?`, `idempotency_key?`, `dry_run?`, `confirm_s?`, `token?` (join) | `config.rs:397-424` |
| `ControlRequest` | `token`, `confirm_s?` (extend) | `config.rs:452-459` |
| `PendingReply` | `token`, `apply_at?`; zenoh-modem also sends `deadline` (§9) | `config.rs:610-617` |
| `SetReply` *(proposed)* | `{applied: ConfigView}` or `{pending: PendingReply}`, discriminated | gap 8 |
| `ConfigStatus` *(proposed)* | `revision`, `pending?`, `last_change?`, `confirm_floor_s`, `values{group → {name → ParamValue}}` (sensitive values never present) | gap 9 |
| `ConfigChangeEvent` | `resource`, `revision`, `token?`, `outcome` (`applied` / `confirmed` / `rolled-back` / `partial`), `edits[]{parameter, old?, new?, redacted?}`, `actor?`, `request_id?`, `claimed_source?` | `config.rs:758-785` |
| `ConfigError` | `code` ∈ {`restart-required`, `device-refused`}, `detail` | v1 `[[error]]` entries (`modem.toml:1325-1335`); every producer of RFC 05 §5.1 needs both, so the profile standardizes them |

`resource` in the view and the event repeats the service name, as a structured field (r3.2
guidance). `actor` and `request_id` come from the O7 call-metadata attachment, which replaces v1's
`?actor=` and `?request_id=` selector parameters (`architecture.md:384`). `claimed_source` has no
stable source in zenoh 1.10.1 (gap 21).

## 4. Groups and classes, per device

The class belongs to the device's declaration, never to a parameter's name. An MTU is `contract` on
an SDU medium and `hot` on a netdev. Groups are runtime data in the view. They are not in the
contract, and the `set/{group}` values come from the device.

| Backend (shipped config) | Group | Class | Parameters | Writable in the shipped config |
|---|---|---|---|---|
| `sbd-medium` (`sat0-loopback.toml`) | `poll` | hot | `signal_poll_s` (s, ≥ 0), `mailbox_poll_s` (s, ≥ 0) | yes |
| | `alerts` | hot | `ring_alerts` (bool) | yes |
| | `terminal` | contract | `port` (text), `baud` (1200–115200 bit/s), `imei` (text) | — |
| `sx1262-medium` (`lora0-sx1262.toml`) | `duty` | hot | `fraction_ppm` (0–1,000,000 ppm), `period_s` (s, ≥ 1) | yes |
| | `radio` | **reach** | `frequency_hz` (150–960 MHz) | no |
| | `modulation` | contract | `spreading_factor` (7–9) | — |
| `fake-medium` simulator (`rf0-managed.toml`) | `duty` | hot | `fraction_ppm`, `period_s` | — |
| `fake-medium` radio cell (tests) | `radio` | **reach** | `channel` (0–255) | tests |
| `udp-medium` (lab) | `air` | hot | `rate_bps` (bit/s, ≥ 1) | — |
| `netdev-modem` (`wwan0-netdev.toml`), and `dlep-modem` through its kernel interface (`dlep0.toml`) | `link` | hot | `mtu` (68–65535 By) | wwan0 yes, dlep0 no |
| | `queue` | hot | `tx_queue_len` (packets, ≥ 0) | yes |
| | `interface` | contract | `interface` (text) | — |
| `mm-modem` (`wwan1-mm.toml`) | netdev's three, plus `bearer` | **reach** | `apn` (text) | no |
| | `sim` | hot | `pin` (text, **sensitive**) | no |

Sources: the `Configure::schema` impls in `sbd-medium/src/lib.rs`, `sx1262-medium/src/lib.rs`,
`fake-medium/src/{sim,cell}.rs`, `udp-medium/src/lib.rs:224-238`, `netdev-modem/src/lib.rs`, and
`mm-modem/src/lib.rs`. The `writable` lines come from `configs/*.toml`.

- **contract**: `set` answers `app` with `ConfigError{code: "restart-required"}`.
- **sensitive**: write-only. A sensitive value never appears in a reply, the view, the status or
  an event (its edit is `redacted`), and never in the overlay. `persist` refuses a change carrying
  one (`config.rs:772-786`).
- **No shipped config makes a reach group writable.** A reach change from anywhere is refused by
  the server's allowlist before any ACL is consulted, unless an operator names the group in
  `writable`. Every claim about the far side below assumes that line was written.

## 5. One change, step by step

The server's own guards run first, in v1's order (`modem-mgmt/src/config.rs:278-410`;
zenoh-modem `interfaces.md` §4.5). Each refusal maps onto the zk2 core envelope (r3 O3):

| Step | v1 refusal | zk2 |
|---|---|---|
| 1. The key must be this device's concrete key | `error/fanout-forbidden` | `fanout_forbidden` (O2, `architecture.md:379`), whatever the ACL says |
| 2. The group must be in the startup `writable` | `error/gated` | `unavailable`, cause `config` |
| 3. One writer, one pending change per device | `error/busy` naming the token | `busy`, detail names the token; a `set` carrying that token **joins** instead (RFC v1.50) |
| 4. `idempotency_key` seen in the last 300 s | the first answer | the first answer |
| 4. `expected_revision` stale | `error/invalid-args` | `invalid_request` |
| 5. The reference validator | `error/not-found` (unknown group or parameter), `error/invalid-args` (kind, bounds, reach without window or token, join with a window) | `not_found`, `invalid_request` |
| 5. A contract group | `error/modem/restart-required` | `app` + `ConfigError{restart-required}` |
| 6. The device refuses | `error/modem/device-refused` | `app` + `ConfigError{device-refused}`, the device's words in `detail` |
| `persist` with no state directory | `error/unsupported` | `unavailable`, cause `config` |
| A token that names nothing | `error/not-found` | `not_found` |

Then, by class:
- **hot, no window.** Apply, then reply with the read-back (`ConfigView`, values at `source:
  runtime`, `revision` + 1). The event is `applied`, and the change is `last_change`.
- **any class, with `confirm_s`.** The window is `max(confirm_s, confirm_floor)`
  (`config.rs:352`). Hot: apply, arm the deadline, reply with the read-back.
  - `confirm` keeps the change, `cancel` reverts it now, `extend` moves the deadline.
  - Each of the three answers with the read-back after the act.
  - At the deadline the undo runs **once**. Success is `rolled-back`. A failed undo is `partial`
    plus a `device_fault`: never retried, never escalated (zenoh-modem I1).
- **reach.** A window or a joining token is mandatory. The reply `{token, apply_at, deadline}`
  leaves **before** the apply, which follows `APPLY_AFTER` = 1 s later (`config.rs:44-46,
  354-378`), because the read-back would cross the link being changed.
  - The caller observes the echo over the new link, then confirms.
  - A `confirm` that arrives before the apply is `busy`: "confirm it once the echo shows it"
    (`config.rs:601-617`).
- **dry run.** Runs steps 1–5 and answers the would-be view, touching nothing.

## 6. The confirm floor, per device

`Configure::confirm_floor` (`modem-device/src/config.rs:392-400`) is the smallest window that
makes sense on the device's link. A shorter `confirm_s` is raised to it, and the reply's
`deadline` says so.

| Device | Floor | Why | Source |
|---|---|---|---|
| default | 60 s | — | `modem-device/src/config.rs:398-400` |
| SBD terminal (`sbd-medium`) | 2 × `session_timeout_ms` | one session to apply, one to read back, each of which may wait for a satellite | `sbd-medium/src/lib.rs:426-431` |
| SX1262 LoRa (`sx1262-medium`) | max(20 × airtime(max SDU), 10 s) | twenty full frames before the far end can have answered on the new channel | `sx1262-medium/src/lib.rs:878-883` |
| ModemManager (`mm-modem`) | 90 s | re-attaching to a cellular network takes tens of seconds | `mm-modem/src/lib.rs:555-558` |
| netdev (`netdev-modem`, `dlep-modem`) | 10 s | a local link | `netdev-modem/src/lib.rs:415-418` |
| simulators (`fake-medium`) | max(20 × latency, 1 s) | twenty round trips | `fake-medium/src/sim.rs:1247-1251`, `cell.rs:849` |

The floor is an instance fact: it depends on the device's configuration. The v1 view does not
carry it, so a caller learns it only from the `deadline` in a reply. `config.v1` should carry
`confirm_floor_s` in the view and in the status (gap 9).

## 7. The echo

`state/view` is v1's echo. v1 sent it on `transition` QoS (reliable, block, data), which is exactly
the zk2 state default, so the contract needs no override.
- **Cadence.** It is put the moment a change lands and refreshed every `ttl_s/2`, with
  `freshness.ttl_s = 60` (`config.rs:1086-1100`).
- **The read.** A GET on it is the read (S2), so v1's read procedure is gone. The owner's
  queryable answers with the mutation's timestamp.
- **Cost.** For the SX1262's three groups the view is about **1,286 B** of JSON, schema and
  descriptions included: six 220-byte SDUs, **4.4 s** of air at 2,400 bit/s.
  - Subscribing to it across the RF face would cost about 15% of the channel through its 30 s
    refresh alone.
  - The `{revision, pending, last_change, values}` status a remote operator needs is about
    **319 B**, two SDUs. That is gap 9.

## 8. Persistence, and the boot that checks it

- **Three layers.** *startup* is the operator's TOML, which the daemon never rewrites. *overlay*
  is `<state_dir>/<device>.overlay.toml`, mode 0600, written only by `persist`. *running* is
  what the device has.
- **What the overlay records.** Each entry keeps the startup value it overrode.
- **What boot does.** Boot reconciles the overlay against the device rather than assuming it. It
  drops, and reports as a `partial` change by actor `boot`:
  - an entry whose startup value has since moved;
  - an overlay written against another schema (dropped whole);
  - an entry outside the schema's bounds.
- **Provisional boot.** With `provisional_boot_s`, a device that is not `up` by the deadline under
  the overlay boots without it. The startup values return, the file is set aside as
  `.provisional`, and a `rolled-back` change by `boot` says so, once (`config.rs:951-1010`;
  zenoh-modem `architecture.md` §4.14).
- **Where it shows.** The view's per-value `source` (`default` / `file` / `overlay` / `runtime`)
  and `startup` make divergence visible without a second document.
- **zk2 adds one thing.** A restart during a pending change is a rollback by construction, and it
  is also a **new instance id**. A late `confirm` with the old token is `not_found`, and the
  caller can tell why from presence (O5).

`config.v1` keeps all of this as the producer's business. It names only the act: `persist` is
explicit, has its own key, and is separately grantable.

## 9. Desired state never carries a reach group

A group's class is runtime data in the view, not a contract fact. So "desired state never carries
reach" cannot be checked by codegen or by the ACL generator. **The `config.v1` server enforces it
when it converges**: a desired document that names a reach group, a contract group or a sensitive
parameter is refused, and the refusal is reported in the target's own state. The rule is still
whole, but it lives at the server.

zenoh-modem decided on 2026-10-05 not to build desired state yet (`docs/design/desired-state.md`).
When it does, it is `desired.v1`. The document is the authoring service's state keyed by target,
for example `zk2/ground/fleet-mgr/modem_desired.v1/state/configs/h-3fa9c2d41b7e.sat0`. The device
service binds it with `{target} = self`, converges through the same path as `set`, and reports
`observed_revision` in `config.v1`'s status.

## 10. v1 findings made while mapping

These are drifts between zenoh-modem's v1 registry and implementation and RFC 05 §5.1. They are
not zk2 gaps. Each is fixed by binding to `config.v1` as written above.

1. **`set` declares the wrong reply for reach.** The registry declares `reply = "ConfigView"`
   (`modem.toml:1248`), but a reach `set` answers `{token, apply_at, deadline}`
   (`config.rs:373-378`). The registry does not tell the truth for the one case the reach class
   exists for.
2. **The reach reply carries an extra field.** `deadline` is not in zenkey's `PendingReply`
   (`zenkey/src/config.rs:610-617`). It is additive, and the schemas are open, but no schema
   describes it.
3. **`persist` replies with `Ack`.** It sends `{}` (`modem.toml:1296`, `config.rs:827`) where RFC
   05 §5.1 (v1.47) answers with the read-back.
4. **No joining and no `last_change`.** RFC v1.50's joining `set` and `last_change` are not
   implemented. `not_busy` refuses every second writer (`config.rs:412-423`), and `persist` takes
   only a confirmed change (`config.rs:761-771`). So a change applied without a window cannot be
   persisted.
5. **`APPLY_AFTER` is a fixed 1 s** (`config.rs:44-46`). A 114 B reply is one SDU, 0.73 s at
   2,400 bit/s. At 300 bit/s it is 3 s, so the change would cut the link before its own reply
   had left. The delay should derive from the device, like the floor: at least the reply's
   airtime with a margin.
6. **The SBD floor protects a path the shipped faces close.** The 2-session floor exists for an
   operator configuring over SBD, but both SBD router configs deny all management on that face
   (`configs/zenohd-sat0-gateway.json5:87-127`). Today it only governs local operators.
7. **The `ops` principal can call every v1 read procedure** (§11.3): `describe`, `introspect`
   (about 50 KB), `sdu/lanes` and the config read. The generator carved only writes.

## 11. The far side: why it can arm a reach change, and neither confirm nor see the echo

The setup is `modem-driver/tests/fixtures/acl/zenohd-rf0-ops.json5`, generated by `zenctl acl gen`
from `enrollment-ops.toml`. That enrollment holds one principal, `user = "ops"`, `role =
"console"`, `writes = ["modem/config/*/*/set"]`, authenticated by `usrpwd` on the RF face (#153).
`auth_face.rs:441-503` runs it in a real router across a 220-byte medium.

**1. Arming crosses.**
- The ops policy (fixture l.64) repeats every face deny except `deny-rpc`. In its place it has
  `deny-rpc-legs` (l.45-47), which denies only `put`, `delete` and `declare_subscriber` on
  `**/@rpc/**`.
- A `query` on `v1/*/@rpc/modem/config/rf0/radio/set` therefore matches no deny in the ops policy.
- Under `default_permission: "allow"` (l.14), zenoh 1.10.1 returns Allow as soon as no deny
  matches (`zenoh/src/net/routing/interceptor/authorization.rs:604-614`).
- The `constrained-link` subject denies it, but across subjects any Allow wins
  (`access_control.rs:978-1018`).
- The `{token, apply_at, deadline}` reply crosses back the same way (`reply` is not in
  `deny-rpc-legs`).
- The grant cannot tell reach groups from hot ones, because the class is not in the key.
- `auth_face.rs:456-469` asserts the `set` crosses and is applied.

**2. Confirm, cancel and extend do not cross.**
- `no-remote-actions-ops` (l.53-55) denies `query` on `config/*/confirm`, `cancel`, `extend`,
  `persist` and `action/*`.
- Under a permissive default, every declared write the principal was not granted must be denied
  by name, and deny is evaluated first.
- The enrollment granted `set` only, so the generator carved out the rest.
- `auth_face.rs:491-502` asserts the confirm gets nothing back.

**3. The echo does not cross.**
- `deny-state-modem` (l.21-23) denies `v1/*/state/modem/**` in every message class, liveliness
  included, and it is in the ops policy too.
- That subtree contains the echo `state/modem/config/rf0`.
- `deny-events-modem` (l.17-19) hides the `config_change` events the same way.
- Unintended side effect: the v1 config **read procedure** (`@rpc/modem/config/rf0`) is a
  `query` the ops policy does not deny, so it crosses.
  - So do `describe`, `introspect` and `sdu/lanes`. The generator carves only the declared
    writes, and allow rules are inert under `allow`: the `writes-ops` rule at l.49-51 is never
    consulted (`authorization.rs:613-614`).
  - auth_face.rs does not probe reads. This is a reading of the fixture against zenoh's source,
    not a measurement.
  - The read-back can therefore be polled with a GET, but the echo cannot be watched.

**The result.** From the far side, a reach change is a guaranteed rollback:
1. Arm it.
2. Watch the link drop at `apply_at`.
3. Retune the far end.
4. Find nothing to confirm with: the confirm is denied, and the deadline undoes the change.

That is airOS's test mode without the "keep" button. It is safe, which is the point of the reach
class, but it is not configuration.

## 12. The zk2 grants that fix it

Under zk2 each act is its own `@op` key, and the device is the service. The ACL generator (#616)
reads contracts, deployment and the role file, so it can grant exactly:
- `set`, `confirm`, `cancel` and `extend` on **rf0's** `config.v1`;
- a GET of rf0's `config.v1` view and status;
- **not** `persist` (a deployment choice, shown withheld);
- nothing of `sat0`.

Two facts decide the shape:
- Under zenoh-modem's node-global `default_permission: "allow"`, an allow rule is never consulted
  (`authorization.rs:613-614`). A **grant is an absence of deny** in the principal's own policy.
- Any subject's Allow wins across subjects (`access_control.rs:983-995`). So the principal's
  policy repeats every face deny, and denies every declared call it does not grant: the
  complement, enumerated from the contracts and the deployment's services (README gap 13).

The face rules themselves are in [`link.md`](link.md) §3; below is what the `ops` principal adds.

```json5
// zenohd-rf0 (zk2), the `ops` principal on the RF face. Host h-3fa9c2d41b7e, device services rf0, sat0.
// Merged with link.md §3's face block: same `default_permission: "allow"`, same face subject.
rules: [
  // Of the @op plane, the legs no call uses (the face denies **/@op/** whole; this policy may not).
  { id: "ops-deny-op-legs", permission: "deny", flows: ["egress", "ingress"],
    messages: ["put", "delete", "declare_subscriber", "declare_queryable"],
    key_exprs: ["**/@op/**"] },
  // Every declared call ops is NOT granted: the complement, by service and interface.
  { id: "ops-deny-calls", permission: "deny", flows: ["egress", "ingress"],
    messages: ["query", "reply"],
    key_exprs: [
      "zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/persist",    // allowed to keep, not to survive a reboot
      "zk2/h-3fa9c2d41b7e/rf0/modem.v3/@op/**",          // lanes, actions
      "zk2/h-3fa9c2d41b7e/sat0/*/@op/**",                // every other device service, whole
    ] },
  // No subscription and no refresh of any device state crosses, the config view included.
  { id: "ops-deny-state-push", permission: "deny", flows: ["egress", "ingress"],
    messages: ["put", "delete", "declare_subscriber", "declare_queryable"],
    key_exprs: ["zk2/*/*/modem.v3/state/**",
                "zk2/h-3fa9c2d41b7e/rf0/*/state/**", "zk2/h-3fa9c2d41b7e/sat0/*/state/**"] },
  // GET only rf0's config.v1 view and status: deny the GET of everything else the face denies.
  { id: "ops-deny-state-get", permission: "deny", flows: ["egress", "ingress"],
    messages: ["query", "reply"],
    key_exprs: ["zk2/*/*/modem.v3/state/**",
                "zk2/h-3fa9c2d41b7e/rf0/health.v1/state/**",
                "zk2/h-3fa9c2d41b7e/sat0/*/state/**"] },
],
subjects: [
  { id: "radio", link_protocols: ["unixsock-stream"] },
  { id: "ops", usernames: ["ops"], link_protocols: ["unixsock-stream"] },
],
policies: [
  { id: "face", subjects: ["radio"], rules: [ /* link.md §3: face-deny-* */ ] },
  { id: "ops", subjects: ["ops"],
    rules: ["face-deny-zk", "face-deny-explicit", "face-deny-admin",
            "face-deny-modem-streams", "face-deny-modem-streams-detail", "face-deny-companion-streams",
            "ops-deny-op-legs", "ops-deny-calls", "ops-deny-state-push", "ops-deny-state-get"] },
],
```

**What that grants.**
- `query`/`reply` on `zk2/h-3fa9c2d41b7e/rf0/config.v1/@op/{set/*, confirm, cancel, extend}`.
- `query`/`reply` on `…/rf0/config.v1/state/{view, status}`.

**What it does not grant.**
- `persist`.
- Any operation or state of `sat0`.
- Any `@zk` traffic, `@stream` or admin space.
- Any refresh of a document, any event.

**Narrower variants.**
- *Hot groups only:* add `…/rf0/config.v1/@op/set/radio` to `ops-deny-calls`. The group names come
  from the device's schema, which the deployment knows.
- *Persist too:* drop the first line of `ops-deny-calls`.

**Wildcard queries.** A wildcard query such as `…/rf0/config.v1/@op/*` still crosses: deny works by
inclusion. Every operation's server then refuses it as non-concrete (O2), and the denied keys'
replies stop at the router. As in v1 (#113), O2 is the guarantee and the ACL is defense in depth
(U14).

**The far-side reach change under these grants.** rf0 is the SX1262, changing `radio/frequency_hz`,
at 2,400 bit/s; one 220 B SDU takes 0.73 s.

| Step | What crosses | Air |
|---|---|---|
| 1 | `query …/rf0/config.v1/@op/set/radio`: 46 B key, `ConfigChange` with `confirm_s = 120`, `expected_revision`, `idempotency_key`, O7 `{actor, request_id}` | 1 SDU |
| 2 | reply `{token, apply_at, deadline}`, 114 B | 1 SDU |
| 3 | apply at `apply_at`; the link drops; the far end retunes itself, locally | — |
| 4 | link re-established: re-declaration of what the far side subscribes to, the four link streams = 236 B of keys ([`link.md`](link.md) §6) | 0.79 s |
| 5 | GET `…/rf0/config.v1/state/status` (proposed, about 319 B), or `state/view` (about 1,286 B) | 2 SDUs, or 6 |
| 6 | `query …/rf0/config.v1/@op/confirm` `{token}`; its reply is the read-back (RFC v1.47) | 1 + 6 SDUs |
| — | `persist`: refused at the router; a local operator persists | — |

The window must cover steps 2 to 6, and the floor guarantees at least
max(20 × airtime(max SDU), 10 s). The confirm's 1,286 B reply is the largest single cost, which is
another argument for gap 9: let `confirm`, `cancel` and `extend` answer the status where the view
would not fit.

**What the zk2 binding still cannot do.**
- **Authentication.** `usrpwd` authenticates the far node at establishment only. Anyone on the
  channel can inject into a live transport afterwards (zenoh-modem I6, `architecture.md:1236-1241`).
- **Telling the lanes apart.** The face subject cannot tell rf0's lane from sat0's (README gap 10).
  These grants assume one radio per router.
