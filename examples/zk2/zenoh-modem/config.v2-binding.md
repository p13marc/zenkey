# `config.v2`, bound to zenoh-modem (#985 G4)

[`config.v1-binding.md`](config.v1-binding.md) bound zenoh-modem's configuration plane to
`config.v1`, and found why the far side of a radio can **arm** a reach change but can **neither
confirm it nor see the echo**. Its three proposals were a union reply for `set` (README gap 8), a
compact status (gap 9) and a visible floor (gap 9). [`config.v2`](../../../spec/profiles/config/v2.md)
answers all three with one packed record. This file binds it, and keeps the v1 file as the record
it was.

**Verdict.** For the SX1262, the far side can now arm, watch and keep a reach change, and each
step is one radio frame:
- `set/radio` is 16 B, and its reply, the record with the change pending, 50 B;
- the record crosses as a push, 46 B after the apply, so no GET follows;
- `confirm` is 8 B, and its reply 36 B, where `config.v1`'s was the 1.3 KB view.

Three backends are ready at 0.1. Four hold a text parameter, which `config.v2` 0.1 cannot
declare, and stay on `config.v1` until a packed format carries text (§4).

| | |
|---|---|
| **The contract** | [`config/sx1262_config.v1.toml`](config/sx1262_config.v1.toml), `sha256:dc7c01069bffc9dc5a966c93cfe2444eca0391e71f051bc9bf6d2b2b54d4317b` |
| **Its types** | [`config/schemas/sx1262_config.json`](config/schemas/sx1262_config.json): the three group types, then the five derived ones |
| **Profile** | `config.v2` text 0.1, with `freshness.v2` and `link.v2` |
| **Catalog** | [`catalog.json`](catalog.json) `29bf7d47`, index 7, where `8c808dff` appended it |

## 1. Which service, which interface

Each device service whose backend has a configuration implements **its backend's config
contract**, beside `modem.v3`. The SX1262's is `sx1262_config.v1`. `config.v2` has no standard
contract (its §1): the groups, the classes and the bounds are what make one device's
configuration differ from another's.
- **Found by its profile.** A configurable service lists `config.v2` in its descriptor's
  `profiles`, through `uses` (config.v2 §2.1). `config.v1`'s selector
  `zk2/*/*/@zk/alive/config.v1/**` has no equivalent: a tool reads descriptors. Across a face,
  where no descriptor crosses, the deployment's word stands for it (core R7).
- **One or the other.** An instance implements `config.v1` or a config contract, never both. A
  device moves from one to the other with a new instance (config.v2 §1).
- **The interface id is `sx1262_config.v1`.** A name's segments are `[a-z][a-z0-9_]*` (core
  §1.2, E001), so the design's `sx1262-config`, with a hyphen, is none. The file is named after
  it (W107).

## 2. The keys

For `lora0`, the SX1262, on host `h-c2a95e41d7f0`:

| v1 (`config.v1`) | `config.v2` | Kind | Grant |
|---|---|---|---|
| `…/lora0/config.v1/state/view` | `zk2/h-c2a95e41d7f0/lora0/sx1262_config.v1/state/config` | state: the record | GET, and its push where exposed |
| `…/lora0/config.v1/state/status` (proposed) | the same record | — | — |
| `…/lora0/config.v1/@op/set/{group}`, up to 16 | `…/sx1262_config.v1/@op/set/duty`, `…/@op/set/radio` | `@op`, concrete | one key each |
| `…/@op/set/{group}` for `modulation` | none: a contract group has no `set` | — | — |
| `…/@op/confirm`, `cancel`, `extend` | `…/sx1262_config.v1/@op/confirm`, `cancel`, `extend` | `@op` | one key each |
| `…/@op/persist` | `…/sx1262_config.v1/@op/persist` (optional) | `@op` | its own key |
| `…/config.v1/events/change/<ulid>` | `…/sx1262_config.v1/events/change/<ulid>` | event, JSON | host only |

A call to `…/@op/set/modulation` reaches no queryable, which is silence (core O5). The contract
says the group is `contract`, and a tool reads it there.

## 3. The contract

```toml
[interface]
name    = "sx1262_config"
major   = 1
minor   = 0
uses    = ["config.v2", "freshness.v2", "link.v2"]

[schemas]
packed     = ["schemas/sx1262_config.json"]
jsonschema = ["../../../../spec/profiles/config/config.v2.json"]   # the change event

[resources.config.annotations]
"freshness.ttl_s" = 300
"link.exposure"   = "link"
"config.groups"   = [
  { name = "duty",       class = "hot",      type = "packed:Duty",       units = { fraction_ppm = "ppm", period_s = "s" } },
  { name = "radio",      class = "reach",    type = "packed:Radio",      units = { frequency_hz = "Hz" } },
  { name = "modulation", class = "contract", type = "packed:Modulation" },
]
# set/duty, set/radio, confirm, cancel, extend and persist (optional): packed requests, the
# Record as every response, Error as every error; change, the JSON event.
```

**The groups,** from `sx1262-medium/src/lib.rs`'s `Configure::schema`, names, kinds, units and
descriptions as it gives them:

| Group (class) | Parameter | Bounds | Bits |
|---|---|---|---:|
| `duty` (hot) | `fraction_ppm` | 0 to 1,000,000 ppm | 20 |
| | `period_s` | 1 to 86,400 s | 17 |
| `radio` (reach) | `frequency_hz` | 150,000,000 to 960,000,000 Hz | 30 |
| `modulation` (contract) | `spreading_factor` | 7 to 9 | 2 |

The bounds are illustrative, and zenoh-modem chooses the real ones.
- **`period_s` has no upper bound** in the runtime schema (`ParamKind::integer(Some(1), None, …)`).
  config.v2's `int` needs one, and 86,400 is this contract's. `Configure::schema` must state the
  same bound, or the start-time check refuses (§13).
- **A bound is the wire.** A radio variant with a wider frequency range is a breaking revision
  (config.v2 §2.2), so the contract should state the backend's widest bounds. A device that refuses
  a value within them answers `device_refused`.
- **SF12 is outside `spreading_factor`'s bound.** The contract states 7 to 9 in 2 bits, and the
  mapping plans SF12 (§11, [`faces/zenohd-lora0-sf12.face.toml`](faces/zenohd-lora0-sf12.face.toml)).
  A radio started at SF12 runs a value its record has no code for, so its owner's start-time check
  refuses it (§13). Either the bound becomes 7 to 12, 3 bits, which moves the record's sizes and
  §15's bytes, or SF12 runs another revision. That is zenoh-modem's call (marcpardo/zenoh-modem#213),
  and this contract stays as it is until it is made.

**The derived types,** which the packed file holds after the group types:

| Type | Tag | Bits | Is |
|---|---:|---|---|
| `Record` | 9 | 109 to 392 | `state/config`, and every reply |
| `SetDuty` | 13 | 7 to 172 | `set/duty`'s request |
| `SetRadio` | 12 | 6 to 164 | `set/radio`'s request |
| `Control` | 8 | 49 to 65 | `confirm`, `cancel`, `extend` and `persist`'s request |
| `Error` | 13 | 4 | the `app` detail: `device_refused`, or the escape |

Each was written by `config.v2` §3's derivation, and its lint finds nothing: the resources
present, each type its derivation, `freshness.ttl_s` above 0 under `freshness.v2`. A contract
cannot reference another's types (README gap 18), so every config contract carries its own
`Control` and `Error`.

## 4. Groups and classes, per backend

| Backend | Groups (class): parameters, bits | At 0.1 |
|---|---|---|
| `sx1262-medium` | `duty` (hot): `fraction_ppm` 20, `period_s` 17; `radio` (reach): `frequency_hz` 30; `modulation` (contract): `spreading_factor` 2 | **yes**: this contract |
| `fake-medium` | `duty` (hot); its cell's `radio` (reach): `channel` 0 to 255, 8 | yes, not written |
| `udp-medium` | `air` (hot): `rate_bps` 1 to 10,000,000, 24 | yes, not written |
| `sbd-medium` | `poll` (hot): two intervals; `alerts` (hot): `ring_alerts`; `terminal` (contract): `port`, `imei`, `baud` | no: `port` and `imei` are text |
| `netdev-modem`, `dlep-modem` | `link` (hot): `mtu` 68 to 65,535, 16; `queue` (hot); `interface` (contract) | no: `interface` is text |
| `mm-modem` | netdev's, `bearer` (reach): `apn`; `sim` (hot): `pin`, sensitive | no: `apn` and `pin` are text |

The four that wait need text, which #979's B3 brings to packed records. v1's classes hold as
[`config.v1-binding.md`](config.v1-binding.md) §4 gives them. Two things change:
- **The class is in the contract.** v1 served it in the view, so codegen and the ACL generator
  could not read it. Now both can, and a tool knows `radio` is reach before it calls.
- **`writable` is a bit per group** in the record (config.v2 §2.16). No shipped config makes
  `radio` writable: an operator still names it in the device's `writable` first.

## 5. One change, step by step

The owner refuses in core §7.5's error form: one byte of CH, the run and seq, one error byte. A
refusal is 7 B, where v1's JSON envelope was 60 to 90 B. config.v2 §2.5's rows, in order:

| Row | Check | Refusal | Bytes |
|---|---|---|---:|
| — | the key is not concrete | `fanout_forbidden` | 7 |
| — | a request that does not parse, a value out of its bounds included | `invalid_request` | 7 |
| 1 | an act is in flight | `busy` | 7 |
| 2 | a retry within 300 s | the first answer, byte for byte | its own |
| 4 | the group is not in `writable` | `unavailable`, cause `config` | 7 |
| 5 | another change is pending, or its deadline has come | `busy` | 7 |
| 6 | a `token`, and nothing pending | `not_found` | 7 |
| 7 | a stale `expected_revision` | `invalid_request` | 7 |
| 9 | both `token` and `confirm_s` | `invalid_request` | 7 |
| 10 | `radio` with neither a window nor a token | `invalid_request` | 7 |
| 12 | the event's rate is spent | `busy` | 7 |
| 13 | joining `radio`, which could not apply before the deadline | `busy` | 7 |
| 14 | the device refuses | `app`, `Error{device_refused}`, its words in the message | 8 + its words |

v1's rows 3, 8 and 11 go: no queryable for an unknown group, no `set` for a contract group, and
the request's type holds each bound. `busy` names no token: the record carries it.

Then, by class, as v1 §5 says:
- **hot, no window.** Apply, put the record, reply with it: values at `runtime`, the revision
  moved by one, the change the last change. The event follows.
- **any class, with `confirm_s`.** The window is max(`confirm_s`, floor), rounded up to a whole
  second. `confirm` keeps, `cancel` undoes, `extend` moves the deadline, each answering the
  record.
- **reach.** Put the record and reply **before** the apply: the change pending, `radio`'s bit
  set, `apply_at` stated. Apply at `apply_at`, and put the record again. A `confirm` before then
  is `busy`, 7 B.
- **dry run.** The record as it would read, `would_be` set, nothing put.

## 6. The floor, in the record

v1's floors hold, per device, as [`config.v1-binding.md`](config.v1-binding.md) §6 gives them. The
SX1262's is max(20 × airtime(max SDU), 10 s). What changes is where a caller reads it:
`floor_s` is in the record, so the far side knows it before it calls, not from a reply's deadline.
At SF7, 222 B frames at 5,469 bit/s, it is the 10 s minimum. On §15's 2,400 bit/s line,
with 220 B frames, it is 14.7 s, stated as 15.

## 7. The echo is the record

`state/config` is one packed record, put on every change and at nothing else. The heartbeat
confirms it every 150 s, half its 300 s horizon (`freshness.v2` §2.4). It holds the running
values, the pending and the last change, the floor, the `writable` bits and the desired report.

| The record | Record | A put or a reply |
|---|---:|---:|
| at start: no change yet | 24 B | 30 B |
| idle, with a last change | 30 B | 36 B |
| a reach change pending, `apply_at` stated | 44 B | 50 B |
| that change applied, not yet confirmed | 40 B | 46 B |

The envelope is 6 B: CH, a 16-bit run and a 24-bit seq. The `config.v1` view was 1,286 B, six
220 B frames, and the proposed status 319 B, two. Every size above is one frame.
- **What no longer travels:** the schema, the service's address, the 16-hex instance id,
  RFC 3339 times, the startup values (one `differs` bit each), and join order.
- **It can cross as a push.** The contract exposes it `link`, which `config.v1`'s standard
  contract could not (README gap 20). It is `reliable` and `block`, a state's defaults, and it
  fits one batch.

## 8. Persistence, and the boot that checks it

As [`config.v1-binding.md`](config.v1-binding.md) §8 says: the three layers, the overlay, the
reconciling boot and provisional boot. Each parameter's `source` shows divergence, as before, and
its `differs` bit says whether the startup value is another. A tool reads the startup value
itself on the host, where it reads files.

A restart is still a rollback. The record's `instance`, the instance's first run, tells it apart
across a face, where presence does not cross (config.v2 §2.12). A late `confirm` with the old
token is `not_found`. A re-mint is a restart too: `Service::new_epoch`, which a device whose
counters reset takes (core §1.5), cancels a pending change, returns the device to its startup
values through `Device::restore_startup`, and restarts the record at revision 0, its instance
number the new run (#1064).

## 9. Desired state never carries a reach group

As [`config.v1-binding.md`](config.v1-binding.md) §9 says. One thing changes: the class is a
contract fact now, so a tool can check a desired document before sending it. The owner still
refuses it whole, and its report is the record's desired fields, the group named by its index.

## 10. v1 findings, made while mapping

Findings 1 to 5 of [`config.v1-binding.md`](config.v1-binding.md) §10 close:
1. **The wrong reply for reach.** Every operation answers the record. A reach `set`'s record
   holds the token, `apply_at` and the deadline.
2. **The undescribed `deadline`.** It is `pending_deadline`, a field of the record.
3. **`persist` replies `{}`.** It answers the record, as every operation does.
4. **No joining and no `last_change`.** Both are config.v2's, by reference to v1, and the
   record's `last_token`. modem-mgmt must implement them to conform.
5. **`APPLY_AFTER` is a fixed 1 s.** `apply_at` is derived from the device (§13). The pending
   reply is 50 B, 0.17 s of payload at 2,400 bit/s.

Findings 6 and 7 stand. The SBD floor still guards a path the faces close (§11). Which reads a
principal reaches is the ACL generator's, now by concrete key (§12).

## 11. What crosses, per medium

| Medium | What crosses |
|---|---|
| RF, LoRa | the record's push, or its GET by grant; `set/duty`, `set/radio`, `confirm`, `cancel` and `extend` by grant; `persist` withheld; the heartbeat covering `sx1262_config.v1` |
| SBD | nothing: the shipped faces deny all management (v1 finding 6), and `sbd-medium` stays on `config.v1` (§4) |
| IP (`mm-modem`, `netdev-modem`) | nothing: they run over IP and stay host-side |

- **The record crosses** where a face exposes `link`: it is a state exposed `link`, not a frame,
  so a face's `frames` list does not govern it (`link.v2` §2.7).
- **The heartbeat crosses with it** (`link.v1` §2.9): one put every 150 s, which keeps the record
  fresh at a far reader.
- **The JSON change event stays home.** `link.v2` §2.17 binds what a contract exposes, and the
  event is not exposed.
- **At SF9** a frame is 115 B. The pending reply's 50 B, its key's 55 B and about 27 B of Zenoh
  take two. At SF12 the fragment layer carries them in 51 B tiles.
- **At SF12** the face affords a 1,800 s horizon, and the record's 300 s is below it, so the
  plan warns `horizon_under_face` of its heartbeat (`link.v2` §2.12). A horizon for the slowest
  medium is zenoh-modem's to choose.

## 12. The grants, and the far-side reach change

A far principal is granted by concrete key: `query` and `reply` on
`…/lora0/sx1262_config.v1/@op/{set/duty, set/radio, confirm, cancel, extend}`, and on
`…/state/config` for its GET, beside the record's push. `persist` is denied by name: a change
may be kept, but surviving a reboot is a local operator's act. Every other device's config
interface is denied whole.
- **A grant per group** is one key (config.v2 §2.21). *Hot groups only* denies `…/@op/set/radio`
  too.
- **The ACL generator** reads the contract, so it can now tell `radio` is reach.
- **The enrollment.** #988 L5's `faces/zenoh-modem.enrollment.toml` lists `config.v1` on
  `lora0`. Once both land, `lora0` implements `sx1262_config.v1` there in its place. The plans
  are then generated with `--contracts examples/zk2/zenoh-modem/config` beside the others, and
  each face names the catalog that holds this contract.

**The far-side reach change.** `lora0` moves `radio/frequency_hz`, with `confirm_s = 120`, an
`expected_revision` and an `idempotency_key`. `config.v1`'s column is
[`config.v1-binding.md`](config.v1-binding.md) §12's, counted on a 220 B frame:

| Step | `config.v1` | `config.v2` |
|---|---|---|
| 1. `set/radio` | about 100 B, 1 frame | 16 B, 1 frame |
| 2. The reply, before the apply | 114 B, 1 frame | 50 B, 1 frame: the record, pending, `apply_at` 2 s on |
| 3. The apply, the far end retuning | the link drops and reopens | as before: the reopen is the transport's (#988) |
| 4. Read the result | a GET of the status, 2 frames, or the view, 6 to 8 | the record's push at the apply, 46 B, 1 frame, no GET |
| 5. `confirm`, and its reply | 1 frame, then the view, 8 frames, whole 58 % of the time | 8 B, then 36 B, 1 frame each |

Each payload above travels with its key's text, 55 B for `…/@op/set/radio`, and about 27 B of
Zenoh's framing.
- **A lost push.** The radio repairs no lost frame, so a push can be lost. The heartbeat's
  digest then diverges, and the far side GETs `state/config`: one key, a 46 B reply.
- **The window** must still cover steps 2 to 5. It is max(`confirm_s`, floor): 120 s here.
- **A lost reply.** A retry with the same idempotency key within 300 s gets the first answer,
  byte for byte, its as-of mark included (config.v2 §2.14).

## 13. The modem side

What modem-mgmt does, once it runs on zk2's runtime (zenkey's `config_v2`, #985 G2):
- **One config contract per backend,** written from its `Configure::schema`. The bounds are
  zenoh-modem's to choose.
- **A schema check at start.** The owner compares its backend's runtime schema with its
  contract, and refuses to start on any difference: a group, a class, a kind or a bound.
  Otherwise the record states bounds the device does not hold. Today `period_s`'s missing upper
  bound is such a difference (§3).
- **`Device` per backend.** It applies, undoes and persists, as `zenkey::config_v1::Device` does
  today. `apply_at` and the floor come from the device, as now.
- **Tokens and times are the runtime's.** A token is the run and a 16-bit count. The runtime
  takes random 32-bit runs, or a counter store whose each run starts its token count at random.
  Times are whole seconds from the catalog's epoch, 2026-01-01, rounded up.
- **Nothing on a timer.** The record is put on a change. The runtime's heartbeat confirms it.

## 14. The ground side

- **zenctl:** `zenctl config get|set|confirm|cancel|extend --contracts examples/zk2/zenoh-modem/config
  --catalog examples/zk2/zenoh-modem/catalog.json`, with #988's `--face-policy` for the reply
  timeout (#985 G3).
- **zengui:** the Config tool, its form drawn from the contract.
- **Reading:** the record by subscription where it is pushed, else a GET of one key. A restart
  shows as a new `instance`.
- **The catalog** dates `apply_at` and the deadline, so the ground holds the same file as the
  host. A reader without it refuses to date them (core §8.6).

## 15. The round trip, as a scenario

\#985's "Done when": zenoh-modem's radio group round-trips a reach change. In zenkey it is
[`zenkey/tests/zenoh_modem_config.rs`](../../../zenkey/tests/zenoh_modem_config.rs), on the G2
owner. It runs in process across
[`constrained.md`](../../../spec/scenarios/constrained.md) §1's face: a vehicle router, and a ground
session attached as its client through a proxy that counts what crosses, standing in for the
2,400 bit/s line, unshaped. The router's access control is the face: the ground is granted §12's
keys alone, so `@zk` and the change event stay on the vehicle's side. The owner serves
`sx1262_config.v1` over a simulated device.

**Setup.** The owner writes under the 16-bit run 0x5A17. Its token count starts at 0x9C41. The
catalog's epoch is 1,767,225,600, 2026-01-01. The steps run on 2026-11-02, and the replies'
`apply_at` and deadline codes are of that date. The startup values are from the file:
`fraction_ppm` 10,000, `period_s` 3,600, `frequency_hz` 868,100,000 and `spreading_factor` 7.
`writable` holds `duty` and `radio`. The floor is 15 s, and `apply_at` comes 2 s after a reach
`set`.

**Steps,** with the bytes lane P's config.v2 codec writes:

| # | At | The ground sends | The owner puts, then replies |
|---|---|---|---|
| 0 | start | — | the record, 30 B: revision 0 |
| 1 | 12:00:00Z on 2026-11-02 | `set/duty`, `fraction_ppm` 9,000: 5 B | put and reply 36 B each: revision 1, `last_token` 0x5A179C41; the event after |
| 2 | 12:01:00Z | `set/radio`, 869,525,000 Hz, `confirm_s` 120, `expected_revision` 1, key 0x3D2C1B0A: 16 B | put and reply 50 B each, at seq 3: `pending_token` 0x5A179C42, `apply_at` 12:01:02Z, deadline 12:03:00Z; no apply yet |
| 3 | 12:01:01Z | `confirm`: 8 B | `busy`, 7 B: not applied |
| 4 | 12:01:02Z | — | the apply, then the put, 46 B at seq 4: still pending, no `apply_at` |
| 5 | 12:01:32Z | `confirm`: 8 B | put and reply 36 B each, at seq 5: revision 2, `last_token` 0x5A179C42; the event `confirmed` after |

**It asserts:**
- every request is at most 16 B, and every reply at most 64 B;
- the reply of step 2 leaves before the apply, and carries the put's bytes and seq;
- the ground holds step 4's record from its subscription, with no GET;
- a retry of step 2 within 300 s gets step 2's 50 B again, byte for byte;
- the change event stays on the vehicle's side of the face.

**The bytes.** The request of step 2 is `4cd5c6291100774000000067a5836140`. Its reply is
`495a1700000300005a1700000001000e40001685e710a032584fd0192c2f480002d0bce2082fc08ca2a0e0fd5ab355882a00`.
Step 5's request is `4800005a179c4200`, and its reply
`495a1700000500005a1700000002000e080002d0bce21027c08ca2a0e0fbfab8c5222a00`.

In zenoh-modem, the same round trip waits for modem-mgmt on zk2's runtime. That is its
tracker's.

## 16. What it still cannot do

- **Authentication.** `usrpwd` authenticates the far node at establishment only (zenoh-modem I6),
  as before.
- **Telling two radios' lanes apart** (README gap 10). The grants assume one radio per router,
  which #988 L5's router per medium keeps.
- **Text.** `sbd-medium`, `netdev-modem`, `dlep-modem` and `mm-modem` wait for a packed format
  that carries text, and stay on `config.v1` meanwhile (§4).
- **A packed change event.** The event is JSON and stays on the host: it would lose its claimed
  attribution until packed text exists.
