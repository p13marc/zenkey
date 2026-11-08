# zenoh-modem's radio faces: the `link.v1` sketch, and the `link.v2` policies (#623, #616, #617, #988)

> **The faces zenoh-modem's routers run are [`link.v2`](../../../spec/profiles/link/v2.md)
> policies, in [`faces/`](faces/) (#988 L5).** One router per medium, as that profile requires
> (its §2.16): zenoh reads the lease, the keep-alives, the batch, `wait_before_close` and the
> timeouts once per router, for every link it holds. Each policy names the zenoh its router runs,
> `zm-1.10.2-r1`, and the catalog the ground holds, [`catalog.json`](catalog.json)'s `29bf7d47`
> (`modem.v3` 3.4 at index 10, #991, after `freshness.v2` at index 9, #1080, `modem.v3` 3.3 at
> index 8, #991, and `sx1262_config.v1` at index 7, #985 G4).
> [`faces/zenoh-modem.enrollment.toml`](faces/zenoh-modem.enrollment.toml) gives the routers'
> principals, and `link.v2`'s `conformance/v2/plan.json` holds each face's plan (its Appendix B).
>
> | Face policy | Node, service | bit/s | Lease / keep-alives, `wait_before_close` | Timeouts | Batch | What crosses, at each frame's `every_s` | `horizon_s` | The plan's warnings |
> |---|---|---:|---|---|---:|---|---:|---|
> | [`zenohd-rf0`](faces/zenohd-rf0.face.toml) | `h-3fa9c2d41b7e`, `rf0` | 2,400 | 240 s / 4, 240 s | 10 s | 222 | `frames/fast`, 60 s | 120 | `frame_unlisted` |
> | [`zenohd-lora0-sf7`](faces/zenohd-lora0-sf7.face.toml) | `h-c2a95e41d7f0`, `lora0` | 5,469 | 120 s / 4, 120 s | 10 s | 224 | `frames/fast`, 60 s | 120 | `frame_unlisted`, `keep_alive_not_suppressed` |
> | [`zenohd-lora0-sf9`](faces/zenohd-lora0-sf9.face.toml) | `lora0` | 1,758 | 480 s / 4, 480 s | 10 s | 117 | `frames/fast`, 120 s; `keyframe_s` 900 | 120 | `frame_unlisted`, `frame_over_batch` |
> | [`zenohd-lora0-sf12`](faces/zenohd-lora0-sf12.face.toml) | `lora0` | 293 | 3,600 s / 4, 3,600 s | 40 s, interests 40 s | 224 | `frames/slow`, 900 s; `keyframe_s` 3,600 | 1,800 | `frame_unlisted` |
> | [`zenohd-sat0`](faces/zenohd-sat0.face.toml) | `h-71c0e2a9d4b3`, `sat0` | 2,400 | 3,600 s / 2, 3,600 s | 1,000 s | 271 | `frames/paid`, 1,800 s | 3,600 | `frame_unlisted` |
>
> The three LoRa policies are alternatives for `lora0`'s one router, one per spreading factor. Every
> face runs `16bit` sequence numbers. RF and LoRa lose 5 % and repair nothing, so their linter also
> warns `no_arq`. SBD retries, so it has no loss and ARQ. What the plans say:
> - **What crosses.** A listed frame crosses whole, never downsampled (`link.v2` §2.7, `link.v1`
>   §2.8). Every frame a face leaves out is denied by name. So `stream:modem.v3` is the seven
>   reading subtrees (`air`, `if`, `layer`, `link`, `radio`, `sched`, `sdu`), 3.2's nine frames,
>   deprecated and `host` since 3.3, and the two of `frames/fast`, `slow` and `paid` the face does
>   not list. No plain stream crosses, so no face has a downsampling rule. `frames/fast`'s largest
>   frame, 216 B, is above SF9's 117 B batch alone (`frame_over_batch`): zenoh fragments its 207 B
>   full record every 15 minutes, and one lost fragment loses the frame, which the next full record
>   heals.
> - **`frame_unlisted`** names, on each router, the frames no face of its policy lets across. One
>   contract serves every medium, and each router knows only its own face. Its device's owner
>   leaves those frames unpublished (`config:device.frames.<name>`).
> - **The cadences** are the owner's configuration (#991's per-medium table), and each plan states
>   them as the deployment's word to the ground. The owner puts each listed frame at its `every_s`.
> - **Keep-alives.** A batch sent resets its end's keep-alive interval. On RF and SF9 the interval
>   is the frame's cadence, 60 s on RF, 120 s at SF9, 900 s at SF12 and 1,800 s on SBD, so data
>   replaces every keep-alive the uplink would send. For that, #991 moved SF9's lease from 240 s
>   to 480 s and SF12's from 1,080 s to 3,600 s, each four frame intervals: SF12's 270 s interval
>   had cost 288 keep-alives and 427 s of air a day, half its duty. SF7's 30 s is below its
>   frame's minute, so the uplink pays one keep-alive between two frames, 74 s of air a day
>   (`keep_alive_not_suppressed`).
> - **Keyframes** (#991's table). `frames/fast` writes a full record at least every 15: 15 minutes
>   on RF and SF7; SF9's face asks for one every 15 minutes, which its owner meets by one every
>   seventh frame. `frames/slow` writes one at least every 4, hourly at SF12. `frames/paid` is a
>   full record every frame.
> - **Liveness** (README gap 14). Each face's frame has a horizon its cadence keeps: 300 s for
>   `frames/fast`, 1,800 s for `frames/slow`, 3,600 s for `frames/paid`. Each plan lists it as
>   evidence, SBD's among them, and none draws `interval_over_horizon` or `horizon_under_face`.
> - **The counter's epoch** (README gap 1). Every counter that crosses is a frame's field, read
>   under [`telemetry.v2`](../../../spec/profiles/telemetry/v2.md) with the frame's run as its
>   epoch, in band (`link.v2` §2.13): a decrease within one run is no longer read as a reset.
> - **The fail-open hazard** (README gap 12) stays narrowed to `frames/`. A frame or a `host`
>   resource a later minor adds there crosses until the plan is derived again.
> - **Both ends run one transport.** `zenctl acl gen --face-policy … --json5` writes the router's
>   `transport` and `routing` blocks, and `--far-config` the ground end's: neither the lease nor
>   the keep-alives are negotiated. `zenctl check face --peer-config` compares the two ends.
>   zenoh-modem's shipped `configs/zenohd-*.json5` still run their own values, RF's 10 s lease
>   among them, which are zenoh-modem's to move.
>
> **Why `modem.v3` still lists `link.v1`.** A face policy is the deployment's, never on the bus
> (`link.v2` §3). A contract lists `link.v2` only to take its packed requirement (its §1 and
> §2.17), and one that lists `link.v1` crosses a `link.v2` face as `link.v1` says, warned
> `not_packed` for each unpacked resource that crosses. Every resource `modem.v3` exposes, 3.2's
> and 3.3's, is a packed frame, so no plan draws one. Revision 3.3 (#991) keeps `uses` at `link.v1`:
> whether `modem.v3` lists `link.v2`, which would make that profile draft, is the maintainer's
> call, and moving it later takes a revision of its own, a new fingerprint and a successor
> catalog. Until then a descriptor of a `modem.v3` instance lists `link.v1`, and so does the
> profiles index.

> **Superseded by [`link.v1`](../../../spec/profiles/link/v1.md), text 0.1
> (#769), then by `link.v2`'s policies above.** This sketch is kept as the record it was; the
> profile is the rule. `link.v1`'s Appendix B gives zenoh-modem's two `link.v1` face policies and
> the plans derived from them (`conformance/plan.json`'s first two cases). What changed:
> - **A defect: the events.** `face-deny-modem-streams` below denies
>   `zk2/*/*/modem.v3/stream/{oper_change,device_fault,identity_change,action_event}/**`,
>   the draft-0 keys of zenoh-modem's occurrences. Under draft 1 they are
>   events, keyed `…/modem.v3/events/<template>/<ulid>` (core §2.6), which
>   that rule does not cover: under this sketch's `allow`, they would cross
>   the RF face. The profile's plan denies every event that is not exposed,
>   here `zk2/*/*/*/events/**` (`link.v1` §2.9).
> - **The face file** gains the far principal, the attachment, the maximum
>   outage, the batch and the link's rate (§2.1).
> - **No `@op` deny and no link subject.** Calls and GETs cross by grant
>   (§2.9), and the face's denies go into the far principal's policy
>   (§2.4).
> - **Companions are denied by interface**, as every resource is: exposure
>   is the contract's (§2.7).
> - **A name beside a link resource** is denied as `…/<name>/**`, which
>   includes the key itself (§2.7).

> **Since `modem.v3` 3.2 (#983 T4, with #991), the readings cross in frames.** The revision lists
> [`telemetry.v2`](../../../spec/profiles/telemetry/v2.md) and adds nine frame resources (core
> §7.7), typed in [`schemas/modem-frames.json`](schemas/modem-frames.json) and exposed `link`. The
> four streams of §1 are `host` again: their readings cross in `frames/link`. §1's table, §3's
> block and its downsampling rules are 3.1's, kept as the record.
>
> | Frame resource | Carries | Largest frame, its payload (32-bit run; `link.v1` 0.16 §2.8) |
> |---|---|---:|
> | `frames/link` | the four link readings, 63 bits; `freshness.ttl_s = 150` | 22 B |
> | `frames/sdu`, `sched`, `air`, `layer`, `if`, `radio`, `bearer` | each group of readings (`bearer` is `link/`'s), counters at 32 bits, a full record at least every 15 | 22 to 109 B |
> | `frames/message` | all 73, counters at 24 bits: one paid SBD message | 213 B |
>
> Under `link.v1`'s policies, whose faces list no frames, every exposed frame crosses a face that
> exposes, and none crosses SBD's, which exposes none (`link.v1` Appendix B).
>
> **Since `modem.v3` 3.3 (#991), in one record, at zenoh-modem's ranges.** The nine above are
> deprecated and `host`: a frame's record has no extension marker, so their illustrative ranges
> could not change within the major (core §9.8), and eight frames a minute spent most of a LoRa
> face's duty on envelopes and times. Three frame resources carry all 73 readings in one record,
> `Readings` ([`schemas/modem-readings.json`](schemas/modem-readings.json)), each range taken from
> zenoh-modem's code or, where the code bounds nothing, a physical bound (README, "Frames"):
>
> | Frame resource | Horizon, full record | Listed by | Largest frame, its payload (32-bit run; `link.v1` 0.16 §2.8) |
> |---|---|---|---:|
> | `frames/fast` | 300 s, at least every 15; deadbands | RF, LoRa SF7 (60 s), SF9 (120 s) | 216 B: a delta of all 73 |
> | `frames/slow` | 1,800 s, at least every 4; deadbands | LoRa SF12 (900 s) | 216 B: a delta of all 73 |
> | `frames/paid` | 3,600 s, every frame | SBD (1,800 s) | 207 B: a full record |
>
> With every reading present and 24 of them moving, RF carries 177 KB a day, LoRa spends 376 s,
> 534 s and 785 s of its 864 s at SF7, SF9 and SF12, and SBD 96 messages a day, as idle.

> **Events, since `modem.v3` 3.4 (#991, after #980's C2).** Since core 0.103 an occurrence's key
> ends in its run and seq, and its payload is form 4, the event, which carries its time (core
> §2.6, §7.5). The four JSON events stay `host`: their records repeat both as `event_id` and
> `at_ms`, and removing a required property is breaking within the major (core §9.8,
> `required_removed`). Revision 3.4 adds three twins for a face, under `face/`, exposed `link`:
> `face/oper_change` (packed, 3 B), `face/identity_change` (packed, at most 16 B) and
> `face/device_fault` (JSON, its words free text). `action_event`'s `burst(600/h)` fits no radio
> face, and it has no twin.
> - **What crosses.** An exposed event crosses whole, every occurrence, never downsampled
>   (`link.v1` §2.9). Each plan denies the four JSON events by name, `events:modem.v3`.
> - **Which face carries which.** Each face lists its events, as it lists its frames (`link.v2`
>   0.12 §2.7, #1098), and its plan denies by name each exposed one it leaves out: rf0's face
>   lists the three, lora0's at SF7 or SF9 the two `rare` ones, `face/device_fault` denied, and
>   lora0's at SF12 and sat0's none, `events = []`, every event denied. Each device's owner
>   publishes what its face lists, `config:device.events.<name>`. `face/device_fault` crosses RF
>   alone, so RF's plan alone warns `not_packed` (`link.v2` §2.17).
> - **The cadence check** (#991's per-medium budgets; `zenkey-fleet/tests/acl_plans.rs`,
>   `zenoh_modem_events_fit_each_face_budget`), over what each face lists. The wire model of 3.3's figures (README, "What each
>   face spends"), for one occurrence alone in its SDU: a put on the scope the far reader's
>   template-prefix subscription declares, its suffix `/` and an 8-character chunk (a 16-bit run;
>   12 characters add 4 B), the QoS extension (reliable, `block`), form 4's 7 B and the record;
>   then the frame's 5 B and the lean stack's 16 B. Each at its rate's worst case: `rare` 24 a day,
>   `low` 1,440. `face/device_fault`'s record is taken at 221 B of JSON, a 16-character lane and
>   128 B of the medium's words.
>
> | Event, its SDU | RF: bytes a day | SF7: s of air | SF9 | SF12 | SBD: messages |
> |---|---:|---:|---:|---:|---:|
> | `face/oper_change`, 47 B | 1,128 | 2.2 | 7.4 | 79.0 (2 tiles) | 24 |
> | `face/identity_change`, 60 B | 1,440 | 2.7 | 8.9 | 90.8 | 24 |
> | `face/device_fault`, 220 + 68 B (115 B SDUs at SF9) | 414,720 | 679 | 2,423 | 22,496 | 1,440 |
> | 3.3's frames, all 73 readings | 177,000 | 376 | 534 | 785 | 96 |
> | **Carried** | the three: **594,288**, 2.3 % of 25.9 MB | two: **381**, 44 % of 864 | two: **550**, 64 % | none: **785**, 91 % | none: **96**, as idle |
>
> What each leaves out would not fit: `face/device_fault` takes LoRa past its duty at every
> spreading factor (1,060 s at SF7); at SF12 `face/oper_change` alone reaches 864.04 s, beside
> frames/slow's 785 s, and each event on SBD is a paid message of its own. With lora0's own
> readings, 434 s at SF12, the two `rare` events would fit there (604 s): a choice for lora0's
> owner, which the check does not take.

zenoh-modem carries stock Zenoh over radios where every byte is billed in airtime:
- 220 B MTU at 2,400 bit/s on RF;
- LoRa at a 1% duty cycle;
- Iridium SBD at about 96 messages a day.

Its management plane stays on the host bus. The router's radio face lets almost nothing of it
cross:
- **On RF**, four readings cross, downsampled to one a minute.
- **On SBD**, nothing does.

This file states how zk2 expresses that:
- the `link.v1` annotations in [`modem.v3.toml`](modem.v3.toml);
- the face declarations (deployment);
- the router rules a generator would emit from both (#616);
- why no `@zk` traffic is needed across the face;
- what the zk2 keys cost to declare, compared with v1's.

## 1. What the contract says: `link.exposure`

`link.v1` adds one per-resource annotation:

| Annotation | Values | Default | Meaning |
|---|---|---|---|
| `link.exposure` | `"host"` \| `"link"` | `"host"` | `link`: the resource is worth carrying across a constrained face, at the face's rate. `host`: never. |

The default is `host`, so a resource added by a later minor stays home unless its author says
otherwise. v1 spelled `exposure` on all 97 entries. zk2 spells it on the four that differ:

| Resource (`modem.v3/stream/…`) | Type | What it tells the far side |
|---|---|---|
| `sdu/dropped_total` | Counter | the link is **losing** |
| `sched/queue_depth` | Gauge | the link is **backing up** |
| `air/duty_cycle_available` | Gauge (0/1) | the medium is **waiting** for a duty-cycle or contact window… |
| `air/duty_cycle_wait_ms` | Gauge, ms | …and **for how long**: duty-cycle-limited, told apart from radio-dead |

These are exactly v1's `exposure = "link"` subjects (`modem.toml:96, 187, 627, 640`).

Two things are **not** contract facts, because the contract author cannot know them:
- which faces are constrained;
- how often a reading may cross a given face.

They are deployment facts (draft 0, `../README.md` "What stays out of the contract"), declared
next.

## 2. What the deployment says: the faces

A face file per router, which the generator reads beside the contracts, the deployment's services
and the role file:

```toml
# faces/zenohd-rf0.toml: the RF node (v1: configs/zenohd-rf0.json5, --link-interval 60)
system   = "h-3fa9c2d41b7e"                       # hostid.v1
services = ["rf0"]                                # the device services this router carries

[face.rf]
select     = { link_protocols = ["unixsock-stream"] }   # never `interfaces`: a unixsock lane has none
class      = "constrained"
expose     = "link"                               # resources with link.exposure = "link" may cross
interval_s = 60                                   # egress, one downsampling rule per (system, service, resource)
```

```toml
# faces/zenohd-sat0.toml: the SBD terminal and its ground gateway (v1: --link-interval none)
system   = "h-71c0e2a9d4b3"
services = ["sat0"]

[face.sbd]
select = { link_protocols = ["unixsock-stream"] }
class  = "constrained"
expose = "none"                                   # a per-message tariff has no affordable sample rate
```

Principals on a face, such as the `ops` operator of [`config.v1-binding.md`](config.v1-binding.md)
§12, come from the role file. A principal grant can expose resources the contracts never annotate,
such as `config.v1`'s view for an authenticated operator. A contract annotation cannot do that for
a standard profile's interface (README gap 20).

## 3. What the generator emits: the RF face

Four facts from zenoh-modem's own ACL work shape every rule. They come from zenoh-modem's
`architecture.md` §3.7 and the zenoh 1.10.1 sources cited there.
- **`default_permission` is node-global.** A deny on a permissive default is the face-scoped
  shape zenoh-modem ships. §4 has the alternative.
- **Within a subject, deny is evaluated first and returns** (`authorization.rs:604-612`). A
  carve-out for what crosses must be an **absence** from the deny list.
- **Deny works by inclusion** (`nodes_including`). A verbatim plane is denied by its widest
  spelling (`**/@op/**`), never by a narrower literal that a broader query would step over (#113).
- **`*` and `**` never match a verbatim chunk.** So `@zk`, `@op`, `@stream` and `@blob` each need
  a rule of their own, plus the admin space.

```json5
// zenohd-rf0 (zk2), generated from modem.v3 + config.v1 + health.v1, faces/zenohd-rf0.toml,
// and the role file. Host h-3fa9c2d41b7e, device service rf0. Every rule names both flows.
access_control: {
  enabled: true,
  default_permission: "allow",   // node-global: the face-scoped deny shape (§4 for the fail-closed one)
  rules: [
    // The control plane never crosses: presence, descriptors, contract bundles (r3.2 §3.10).
    { id: "face-deny-zk", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable",
                 "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"],
      key_exprs: ["**/@zk/**"] },                         // includes zk2/@zk/contract/**
    // No call crosses anonymously: every operation of every service.
    { id: "face-deny-op", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable"],
      key_exprs: ["**/@op/**"] },
    // Explicit-only streams and bulk never belong on a constrained link.
    { id: "face-deny-explicit", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable"],
      key_exprs: ["**/@stream/**", "**/@blob/**"] },
    // The admin space and zenoh-ext's sidecars: every rule above is zk2-shaped.
    { id: "face-deny-admin", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "declare_queryable", "reply",
                 "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"],
      key_exprs: ["@/**", "**/@adv/**"] },
    // No state of a device service crosses: modem.v3 by interface (any host), companions by service.
    { id: "face-deny-device-state", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable"],
      key_exprs: ["zk2/*/*/modem.v3/state/**", "zk2/h-3fa9c2d41b7e/rf0/*/state/**"] },
    // modem.v3 streams: whole subtrees holding no link resource (a new resource there stays home)…
    // (SUPERSEDED, see the note at the top: the four `…/stream/<occurrence>/**` keys below are
    //  draft 0's; draft 1 keys these events `…/modem.v3/events/<template>/<ulid>`, which this
    //  rule does not cover. link.v1 §2.9 denies them as `zk2/*/*/*/events/**`.)
    { id: "face-deny-modem-streams", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable"],
      key_exprs: ["zk2/*/*/modem.v3/stream/layer/**", "zk2/*/*/modem.v3/stream/radio/**",
                  "zk2/*/*/modem.v3/stream/if/**", "zk2/*/*/modem.v3/stream/link/**",
                  "zk2/*/*/modem.v3/stream/oper_change/**", "zk2/*/*/modem.v3/stream/device_fault/**",
                  "zk2/*/*/modem.v3/stream/identity_change/**", "zk2/*/*/modem.v3/stream/action_event/**"] },
    // …and by name inside the three mixed subtrees: the carve-out is the absence of four names.
    { id: "face-deny-modem-streams-detail", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable"],
      key_exprs: [
        "zk2/*/*/modem.v3/stream/sdu/tx_sdus_total", "zk2/*/*/modem.v3/stream/sdu/tx_bytes_total",
        "zk2/*/*/modem.v3/stream/sdu/rx_sdus_total", "zk2/*/*/modem.v3/stream/sdu/rx_bytes_total",
        "zk2/*/*/modem.v3/stream/sdu/tx_failed_total", "zk2/*/*/modem.v3/stream/sdu/rx_unroutable_total",
        "zk2/*/*/modem.v3/stream/sdu/rx_oversize_total", "zk2/*/*/modem.v3/stream/sdu/tx_oversize_total",
        "zk2/*/*/modem.v3/stream/sched/dropped_total", "zk2/*/*/modem.v3/stream/sched/stale_total",
        "zk2/*/*/modem.v3/stream/sched/peerless_total", "zk2/*/*/modem.v3/stream/sched/lane_backlog_bytes",
        "zk2/*/*/modem.v3/stream/sched/lane_backlog_drain_ms",
        "zk2/*/*/modem.v3/stream/air/tx_frames_total", "zk2/*/*/modem.v3/stream/air/rx_frames_total",
        "zk2/*/*/modem.v3/stream/air/cad_deferrals_total", "zk2/*/*/modem.v3/stream/air/cad_forced_total",
        "zk2/*/*/modem.v3/stream/air/airtime_ms_total", "zk2/*/*/modem.v3/stream/air/retransmits_total",
        "zk2/*/*/modem.v3/stream/air/bytes_wasted_total", "zk2/*/*/modem.v3/stream/air/rx_errors_total",
      ] },
    // The device service's companion interfaces have no link resource: their streams stay home whole.
    { id: "face-deny-companion-streams", permission: "deny", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable"],
      key_exprs: ["zk2/h-3fa9c2d41b7e/rf0/config.v1/stream/**", "zk2/h-3fa9c2d41b7e/rf0/health.v1/stream/**"] },
  ],
  subjects: [ { id: "radio", link_protocols: ["unixsock-stream"] } ],
  policies: [
    { id: "face", subjects: ["radio"],
      rules: ["face-deny-zk", "face-deny-op", "face-deny-explicit", "face-deny-admin",
              "face-deny-device-state", "face-deny-modem-streams", "face-deny-modem-streams-detail",
              "face-deny-companion-streams"] },
  ],
},
// The four link resources of rf0, capped at the face's interval. One rule per concrete key, none
// intersecting: zenoh gives each RULE one timer (downsampling.rs:208-251, 281-304), so a rule with
// a `*` in it would make every service and host it covers share one sample a minute (README gap 11).
downsampling: [
  { id: "rf-link", link_protocols: ["unixsock-stream"], flows: ["egress"], messages: ["put"],
    rules: [
      { key_expr: "zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/sdu/dropped_total",        freq: 0.016666666666666666 },
      { key_expr: "zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/sched/queue_depth",        freq: 0.016666666666666666 },
      { key_expr: "zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/air/duty_cycle_available", freq: 0.016666666666666666 },
      { key_expr: "zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream/air/duty_cycle_wait_ms",   freq: 0.016666666666666666 },
    ] },
],
```

**Compared with v1's block** (`configs/zenohd-rf0.json5:71-123`):
- **Plane denies.** `**/@rpc/**` becomes `**/@op/**`. `@zk` is new, and replaces the liveliness
  classes v1 denied under `state/modem/**`.
- **Telemetry.** 69 host readings denied by name become **8 subtree denies plus 21 names**. A
  subtree with no link resource is denied whole, so a minor that adds a resource under `layer/`,
  `if/`, `radio/` or `link/` stays home without regeneration.
- **Scope.** Rules are scoped by **interface** (`zk2/*/*/modem.v3/…`) rather than by producer
  (`v1/*/telemetry/modem/…`). A modem.v3 implementer on any host is covered, whatever its service
  is called.
- **Companions** (`config.v1`, `health.v1`) are standard interfaces that applications use too.
  They are scoped **by service**, from the deployment's `services` list. An application's own
  `config.v1` traffic across the radio is the application's business.
- **Downsampling** names the concrete host and service. v1's `v1/*/telemetry/modem/device/*/…`
  rules (`zenohd-rf0.json5:118-121`) give rf0 and any second SDU device on the node one shared
  slot per reading. That is a latent v1 bug on a two-device node.
- **Caveat.** The anonymous carve-out admits *any* host's four link streams, including ones routed
  through this node. Only this host's are downsampled here, because downsampling rules must not
  intersect and a wildcard rule would share the timer. On the single-uplink RF profile that is
  harmless. Each host downsamples its own at its own face.

**The SBD face.** Same block, with three differences:
- `face-deny-modem-streams` is the whole `zk2/*/*/modem.v3/stream/**`;
- there is no detail rule;
- there is no `downsampling` block.

That is v1's `deny-telemetry-modem`, the whole class (`zenohd-sat0-gateway.json5:99-102`). Nothing
of the management plane crosses.

**The fail-open hazard.** It remains in the three mixed subtrees. A resource that a later minor
adds under `sdu/`, `sched/` or `air/` crosses the radio until the face block is regenerated: the
carve-out is an absence, and the running ACL is not observable (r3 §3.13). This is README gap 12.

## 4. The fail-closed alternative

zenoh-modem's architecture states that a deny on a permissive default is "the only shape that
expresses 'deny here, allow elsewhere'" (zenoh-modem `architecture.md:286-296`). That holds for a
**wildcard** allow subject, which would match the lane too. It does not hold when the host face is
a different link protocol from the lanes, which is true of every shipped profile: the host bus is
on `tcp/127.0.0.1:7447`, the lanes are `unixsock-stream`, and the TCP lane kind is lab-only (§3.7
table).

Under `default_permission: "deny"`, zenoh consults allow rules (`authorization.rs:615-627`), so
this shape is expressible:

```json5
access_control: {
  enabled: true,
  default_permission: "deny",
  rules: [
    { id: "host-all", permission: "allow", flows: ["egress", "ingress"],
      messages: ["put", "delete", "declare_subscriber", "query", "reply", "declare_queryable",
                 "liveliness_token", "declare_liveliness_subscriber", "liveliness_query"],
      key_exprs: ["**", "**/@zk/**", "**/@op/**", "**/@stream/**", "**/@blob/**", "**/@adv/**", "@/**"] },
    { id: "radio-link", permission: "allow", flows: ["egress", "ingress"],
      messages: ["put", "declare_subscriber"],
      key_exprs: ["zk2/*/*/modem.v3/stream/sdu/dropped_total", "zk2/*/*/modem.v3/stream/sched/queue_depth",
                  "zk2/*/*/modem.v3/stream/air/duty_cycle_available", "zk2/*/*/modem.v3/stream/air/duty_cycle_wait_ms"] },
    // + the application traffic the deployment means to carry over the radio, named explicitly
  ],
  subjects: [ { id: "host", link_protocols: ["tcp"] }, { id: "radio", link_protocols: ["unixsock-stream"] } ],
  policies: [ { id: "host", subjects: ["host"], rules: ["host-all"] },
              { id: "radio", subjects: ["radio"], rules: ["radio-link"] } ],
},
```

**What it buys.**
- A resource a later minor adds never crosses: fail-closed.
- `allow` rules mean what they say (README gap 13).
- The radio's budget is an explicit list.

**What it costs.**
- Every verbatim chunk used on the host bus must be listed. A third party's unknown `@x` chunk
  is silently denied on the *host* bus.
- Every application topic meant for the radio must be named.
- A mistake silences the host bus. That at least fails loudly.

Both shapes belong in the generator. S14 (#616) should run this one on a live router before it
is recommended.

## 5. No `@zk` traffic across the face: what the far side needs, and where it gets it

r3.2 claims no `@zk` traffic is ever *required* across a constrained face (`architecture.md:117,
453-458`). For zenoh-modem the claim **holds**, with two caveats.

| The far side needs | Where it comes from, without `@zk` |
|---|---|
| the four keys | static binding (R7): the far side's configuration names `h-3fa9c2d41b7e/rf0` and the contract gives the resource paths. No presence. |
| their types | compiled in from `modem-contract` ([`contract-crate.md`](contract-crate.md)) or a pre-provisioned bundle. No `zk2/@zk/contract/…` fetch. |
| compatibility | FULL_TRANSITIVE inside `modem.v3`: a far side built against any minor reads any other. No fingerprint exchange. This matches zenoh-modem's "a field terminal and its ground gateway are upgraded months apart". |
| whether a reading is absent or dead | not answerable across the face. A device without `capability:duty_cycle` never publishes `air/duty_cycle_*`. v1 is the same: its `sensor` document was denied too. The far side's deployment knows its peer's device. |
| liveness | the freshness of the 1/min samples. **Caveat:** R7 says "state freshness (`health.v1`)" (`architecture.md:307`), but RF carries no state and SBD carries nothing (README gap 14). |
| a counter's continuity epoch | **not available.** The instance id lives in `@zk`. A far side that sees `sdu/dropped_total` go down cannot tell a reset from a fault (README gap 1). |
| configuration calls (`ops`) | static `@op` keys. Refusals carry their cause in the error envelope (`unavailable{cause}`), so no descriptor is needed ([`config.v1-binding.md`](config.v1-binding.md) §12). |

**What would cross if the face did not deny it.**
- The near router runs in peer mode and holds every token. It would propagate liveliness tokens
  to the far peer on interest, put descriptors, and declare contract queryables.
- For one device service implementing `modem.v3`, `config.v1` and `health.v1`, that is an
  instance token of 52 B and three interface tokens of 75 B: **277 B of `@zk` keys per
  reconnect, 7.4 s of air at 300 bit/s**. That is more than the four link keys together (6.3 s).
- `face-deny-zk` is therefore a rule a constrained face must never omit. It is not defense in
  depth.

## 6. Declaration cost: zk2 keys versus v1's

**The model** is zenoh-modem's own (`architecture.md:879-884`):
- A key declared across the face costs its full length in airtime.
- A flapping link re-pays every declaration on each reconnect.
- At 300 bit/s, 46 B is about 1.2 s.
- The 46 B v1 key is the device presence token `…/state/modem/device/rf0/alive`, and v1's longest
  key, `…/layer/compress/dictionary_mismatch_total`, is 85 B: the issue's "46–85 B". Over all 97
  v1 entries the range is 35–85 B.

**Assumptions:** host `h-3fa9c2d41b7e`, device `rf0`, a 26-character ULID. The figures give bytes
and the seconds of air at 300 bit/s.

| Resource | v1 key | zk2 key |
|---|---|---|
| `sdu/dropped_total` (crosses RF) | 62 B · 1.65 s | 56 B · 1.49 s |
| `sched/queue_depth` (crosses RF) | 62 B · 1.65 s | 56 B · 1.49 s |
| `air/duty_cycle_available` (crosses RF) | 69 B · 1.84 s | 63 B · 1.68 s |
| `air/duty_cycle_wait_ms` (crosses RF) | 67 B · 1.79 s | 61 B · 1.63 s |
| **the four, per reconnect** | **260 B · 6.93 s** (0.87 s at 2,400 bit/s) | **236 B · 6.29 s** (0.79 s at 2,400 bit/s) |
| shortest stream, `radio/snr` | 54 B · 1.44 s | 48 B · 1.28 s |
| longest stream, `layer/compress/dictionary_mismatch_total` | 85 B · 2.27 s | 79 B · 2.11 s |
| state `oper` | 45 B · 1.20 s | 42 B · 1.12 s |
| state `sdu/endpoint/peer-b` | 60 B · 1.60 s | 57 B · 1.52 s |
| occurrence `oper_change/<ulid>` | 80 B · 2.13 s | 77 B · 2.05 s |
| op `action/mailbox_check` | 60 B · 1.60 s | 56 B · 1.49 s |
| config `set`, group `radio` | 49 B · 1.31 s | 46 B · 1.23 s |
| config `confirm` | 47 B · 1.25 s | 44 B · 1.17 s |
| config echo | 40 B · 1.07 s | 43 B · 1.15 s |
| presence: host token / device token | 35 B · 0.93 s / 46 B · 1.23 s | — / instance token 52 B · 1.39 s |
| presence: interface token (one per interface) | — | 75 B · 2.00 s |
| contract retrieval (v1 `introspect`) | 39 B · 1.04 s | 90 B · 2.40 s |

**How to read it.**
- **Byte for byte**, zk2 keys are 3–6 B shorter for a device's resources.
  - `telemetry/modem/device/rf0/` (27 B) becomes `rf0/modem.v3/stream/` (20 B).
  - `state/modem/device/rf0/` (23 B) becomes `rf0/modem.v3/state/` (19 B).
  - `zk2/` costs one byte more than `v1/`.
  - Over all 73 telemetry keys that is 4,807 B against 4,369 B.
- **For the four keys that cross**, 9% less: 0.64 s of air saved per reconnect at 300 bit/s.
- **Longer in zk2:** the config echo (`config.v1/state/view` against `config/rf0`), presence, and
  contract retrieval. None of them crosses.
- **The decisive saving is structural, not in bytes.**
  - Only the four link keys ever need declaring across RF, and nothing across SBD.
  - Presence, descriptors and bundles stay home (§5).
  - Shortening interface names (`modem.v3` is 8 B) would buy another 1–2 B per key. That is not
    worth a less legible name.

**What the model leaves out, for S3 (#617) to measure.**
- Declaration framing.
- Whether the far side's declaration also maps the key for the replies and puts that flow back.
- Zenoh's scoped `WireExpr` (prefix id + suffix): a declared prefix such as
  `zk2/h-3fa9c2d41b7e/rf0/modem.v3/stream` could carry the four keys at about 20 B each after
  one 39 B declaration. Not verified.
