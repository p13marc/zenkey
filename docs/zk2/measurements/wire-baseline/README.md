# Today's zk2 wire across zenoh-modem's simulated media, measured

Part of #989 (epic #975). The report
[`constrained-overhead-review.md`](../../constrained-overhead-review.md) prices
today's wire with a model, its Appendix A. This directory measures it end to
end, in both directions, on zenoh-modem's media as a lab simulates them:

- **phase 1** (2026-10-31): W1, modem.v3's 4 link readings at one a minute, and
  the idle link, on RF;
- **phase 2** (2026-10-31): W2, the generic device, on RF; W1 and the idle link
  on LoRa at SF7, SF9 and SF12, and SF7 idle at keep_alive 4;
- **zk2c** (2026-11-08): W1 with all 73 modem readings across rf0's face, W2
  and the idle link, on RF at link.v2's transport; W1-73 at LoRa SF7; and the
  honesty checks across the face ([below](#zk2c-on-rf-and-w1-73-at-lora-sf7-989s-zk2c-half));
- **zk2c on LoRa SF9 and SF12** (2026-11-21): W1-73 and the idle link across
  lora0's SF9 and SF12 faces, each at its face's cadence
  ([below](#w1-73-at-lora-sf9-and-sf12));
- **W2 with owner heartbeats** (2026-11-21, #1110): W2 zk2c on RF again, its
  owner confirming its three interfaces by one owner heartbeat
  ([below](#w2-with-owner-heartbeats-1110));
- **W2 with desired.v2 and alarms.v2** (2026-11-27, #986's J5, #984's S4):
  W2 zk2c on RF again, its desired target on desired.v2, by its period and by
  the watch, and its alarms on alarms.v2
  ([below](#w2-with-desiredv2-and-alarmsv2-986s-j5-984s-s4));
- **the batch's bound** (2026-11-28, #1136): puts of chosen lengths across
  rf0's face, bare, at 16-bit and 32-bit sequence numbers, to find where
  zenoh starts to fragment ([below](#the-batchs-bound-1136)).

SBD is not measured. Every figure is a day, extrapolated from a window of 10
to 45 minutes, or of one and two hours at SF9 and SF12 under zk2c, and every
one is per direction: A->B is the owner's router toward the reader's, B->A
the way back.

## The path

```
zk2 owner (zenoh 1.10.1, TCP client)
  -> zenohd A (the fork, peer)  --unixsock-stream lane-->  modem-driver A
  -> udp-medium ~> relay.py (loss, latency, in order) ~> udp-medium
  -> modem-driver B  --unixsock-stream lane-->  zenohd B (the fork, peer)
  -> zk2 readers (TCP clients)
```

- **Routers.** [`configs/zenohd.json5.in`](configs/zenohd.json5.in), rendered
  per run by [`render.py`](render.py), is zenoh-modem's
  `configs/zenohd-rf0.json5` (d8c62df) with:
  - `mode: "peer"`, gossip and multicast scouting off;
  - each medium's lease and keep-alive (the table below), `wait_before_close`
    at least the lease;
  - `batch_size` equal to the driver's `zenoh_batch_size`;
  - **no lane-face `access_control` and no downsampling.** "Today" is what
    zk2's rules make travel (report §2.2), so the admin space and every zk2
    declaration cross.
- **Drivers.** [`configs/driver.toml.in`](configs/driver.toml.in): one
  `modem-driver` per end, so each end's counters are its own direction.
  - The medium is `udp` (feature `lab-air`), with the medium's frame as
    `max_sdu` and `send` paced to its bit rate.
  - Driver A's endpoint `b` binds, and router A dials it. Driver B's endpoint
    `a` dials router B's listener.
  - The management plane listens on a unix socket of each driver's own, never
    on a data router, so none of it crosses the air. `zenctl echo --no-decode`
    records every reading there, once a second.
- **The relay** ([`relay.py`](relay.py)) stands in for the lab's netem, which
  needs root: a seeded loss per datagram, a one-way latency with jitter, each
  direction in order. It records every datagram (`air.jsonl`), takes a `cut`,
  a `restore` and a new loss on a control port, and never queues for
  bandwidth: each driver paces itself.
- **The lean stack** (report §3.2), in the order an outgoing SDU meets it:
  - `compress` with a trained 1 KB dictionary
    ([`train-dictionary.sh`](train-dictionary.sh)): [`rf.zdict`](rf.zdict),
    from phase 1's bare W1 run, for W1 and the idle link;
    [`w2.zdict`](w2.zdict), from a bare W2 run's SDUs, for W2;
  - `aead`, `aes-128-ccm` with an 8-byte tag, sealed, one test key, counters
    created by `--init-layer-state`;
  - at SF12, `fragment` (SCHC No-ACK, mtu 222, 51-byte tiles), below Zenoh's
    80-byte floor, as zenoh-modem's deploy/README.md says;
  - `overhead` with 2 bytes, standing in for `address`. `layer-address`
    refuses the udp medium at start ("unknown lane */be#0"), because
    udp-medium addresses peers itself. The stand-in is byte-exact (dst, src)
    and does no filtering, which a point-to-point link does not need.

  That is 16 bytes an SDU when compress stores, as the model assumes, and 6
  more on a one-tile fragment packet.

### The media

| Medium | Frame | Paced at | Loss | Latency | Lease / keep_alive | Source |
|---|---:|---:|---:|---:|---|---|
| RF | 220 B | 2,400 bit/s | 5 % | 150 ± 30 ms | 10 s / 4 | fake-medium `rf()`; `zenohd-rf0.json5` |
| LoRa SF7 | 222 B | 5,469 bit/s | 5 % | 10 ± 5 ms | 60 s / 2 | `lora(7)`, EU868; deploy/README.md |
| LoRa SF7, ka 4 | 222 B | 5,469 bit/s | 5 % | 10 ± 5 ms | 60 s / 4 | to test the flapping at 2 |
| LoRa SF9 | 115 B | 1,758 bit/s | 5 % | 10 ± 5 ms | 60 s / 2 | `lora(9)`, EU868; deploy/README.md |
| LoRa SF12 | 51-byte tiles | 293 bit/s | 5 % | 10 ± 5 ms | 540 s / 2, open 40 s | `lora(12)`, EU868; deploy/README.md |

LoRa's rate is the nominal bit rate, SF × 125 kHz / 2^SF × 4/5. Airtime in
every table is each SDU's, by the medium's formula, as fake-medium's
SimMedium charges a frame with no ARQ: on RF, 8 bits a byte at 2,400 bit/s;
on LoRa, the Semtech time on air (125 kHz, CR 4/5, an 8-symbol preamble,
explicit header, CRC on, low-data-rate optimisation at SF11–12). The duty
used is each direction's airtime against EU868's 1 %, 864 s a day: each end
is a transmitter of its own.

### What it simulates, and what it does not

| Simulated | How |
|---|---|
| each medium's frame | udp-medium refuses a larger SDU |
| each medium's bit rate | each driver's `send` blocks for every datagram's time at that rate, the 1-byte lane header included |
| 5 % loss | relay.py drops each datagram with p = 0.05, seeded, per direction |
| each medium's latency | relay.py delays each datagram, uniform jitter, never reordering a direction |
| one driver per end | per-direction counters, and the directional `aead` keys work |
| a coverage gap | relay.py's `cut` drops every datagram both ways until `restore` |

| Not simulated | Consequence |
|---|---|
| half duplex and the channel mutex | each direction paces on its own, so both ends can send at once. At W1's load it changes no byte count; at W2's, a half-duplex RF would have less room than measured |
| EU868's duty cycle | nothing enforces it. A direction over 100 % here would wait, queue and drop on a real radio, so the timing and the drops beyond 100 % are not a real LoRa's |
| LoRa's airtime, in time | the drivers pace at the nominal bit rate, which is faster than LoRa's time on air for short frames: a 51-byte SF12 tile paces in 1.42 s, and its time on air is 2.47 s. A real link is slower; the airtime columns charge the real figure |
| ARQ | fake-medium's media have none either. A lost SDU is lost: about 95 % of a direction's SDUs reach the far end |
| an airtime counter | udp-medium declares no `capability:airtime`, so the airtime is computed per SDU from the relay's capture, as `air/airtime_ms_total` would count it with no ARQ. It leaves out udp's lane byte |
| the address layer's filtering | not needed point to point. Only its 2 bytes are kept |
| independent loss between runs | the relay's seed is 0 in every run, so runs that make the same datagrams in the same order lose the same ones |

## Results, a day, per medium and direction

Every figure is a day: the window's count times a day over the window's length.
SDUs and bytes are each sender's `sdu/tx_*` counters, airtime the relay's capture
charged by the medium's formula.

### RF: the idle link and W1 (phase 1)

Two runs, `bare-1` and `lean-1`, each on one link: 10 minutes idle (no zk2
client on either router), then W1 brought up, then 10 minutes of W1 starting
90 s after the owner, so the window holds exactly 10 ticks.

- **The owner** is `zenctl gen h-3fa9c2d41b7e/rf0 modem.v3 --rate 1/60`. It
  runs every resource of `examples/zk2/zenoh-modem/modem.v3.toml` at one a
  minute on host A, and its four link readings are put about 140 ms apart
  each tick.
- **The far side** is four `zenctl echo --no-decode` subscribers on router B,
  one per link reading. zenoh forwards only what a subscriber matches, so the
  owner's 69 other streams, its 7 states and its events stay on host A.
  `--no-decode` makes no presence read and fetches no bundle.

The relay's capture agrees with the counters to the SDU, but for one
keep-alive at the edge of the lean idle window on A.

#### Idle, a day

| | SDUs | Bytes | Airtime |
|---|---:|---:|---:|
| bare, measured, each direction | 34,559 | 34,559 | 115.2 s |
| lean, measured A->B | 34,415 | 585,061 | 1,958.4 s |
| lean, measured B->A | 34,559 | 587,509 | 1,958.4 s (2.27 %) |
| **model**, lean, each direction (§6.2: 69,120 keep-alives a day, both) | 34,560 | 587,520 | 1,958.4 s |

Every idle SDU is a keep-alive: 1 byte bare, every 2.50 s each way; 17 bytes
lean (compress's stored marker, aead's 13, the overhead's 2).

#### W1, a day

| | SDUs | Bytes | Airtime |
|---|---:|---:|---:|
| **A->B, lean, measured** | **37,583** | **843,537** | **2,811.8 s (3.25 %)** |
| of which data (31 SDUs a window) | 4,464 | 280,507 | 935.0 s |
| of which keep-alives (230 a window) | 33,119 | 563,030 | 1,876.8 s |
| A->B, lean, **model** | 38,880 | 931,680 | 3,105.6 s |
| of which data (Appendix A's W1: 4 readings a minute, each in its own SDU) | 5,760 | 368,640 | 1,228.8 s |
| of which keep-alives (23 a minute, as the report reasons for W2) | 33,120 | 563,040 | 1,876.8 s |
| **B->A, lean, measured** (keep-alives only) | **34,559** | **587,509** | **1,958.4 s** |
| B->A, lean, model | 34,560 | 587,520 | 1,958.4 s |
| A->B, bare, measured | 36,719 | 259,482 | 864.9 s |
| of which data (25 SDUs a window) | 3,600 | 226,362 | 754.5 s |
| A->B, bare, model (data 276,480 B) | 38,880 | 309,600 | 1,032.0 s |
| B->A, bare, measured | 34,559 | 34,559 | 115.2 s |

W1 under the lean stack costs **1.43 MB a day on the air, both directions**
(4,770 s of a half-duplex channel, 5.5 %), against the model's 1.52 MB. Of
it, **1.15 MB are keep-alives** and 0.28 MB are W1's readings. A->B is 9.5 %
under the model in bytes, and its data 24 % under. All 40 readings of each
window reached the far side. The 5 % loss took 6 of 261 SDUs A->B, none of
them data.

Once per link, outside the windows:
- **the link open** is 214 bytes each way bare, in 3 SDUs: a handshake of 83
  bytes each way, then each router's admin-space declarations (131 bytes,
  `@/<zid>/…`), which rf0's ACL would deny;
- **W1's setup** is about 0.5 KB A->B: gen's operation, state and bundle
  queryables, the instance and interface tokens. B->A it is 0.3 KB, the four
  subscriptions in four SDUs (bare 316 B, lean 219 B).

### RF: W2 (phase 2)

Two runs, `w2-rf-bare-3` and `w2-rf-lean-1` (the lean stack with
`w2.zdict`), side by side on two paths
([`measure-w2.sh`](measure-w2.sh), [`w2-drive.py`](w2-drive.py)):

- **The device**, on router A, is the owner example with every interface W2
  asks for: `owner --health --alarms --config --desired-target fleet/desired
  --desired-watch --desired-reread-ms 300000 h-3fa9c2d41b7e/rf0
  contracts/gendev.v1.toml`. [`gendev.v1`](contracts/gendev.v1.toml) has 8
  gauges and 4 counters (revision 1.0: telemetry.v1, the built-in stream
  QoS), which w2-drive.py puts every 10 s, and `oper_change` events. The
  device raises 3 alarms at start; health.v1 and config.v1 are the example's
  own.
- **The far side**, on router B, is one `zenctl echo --no-decode` subscriber
  on `zk2/**`, which reads every element the device puts, and the desired
  author (`owner --desired-author fleet/desired`), which puts the device's
  document before the device starts.
- **The timeline**: the link opens and the device sets up with no loss
  (`LOSS_START=0`); the relay takes 5 % 60 s before the steady window
  (`LOSS_AT_STEADY=0.05`); 120 s to settle; the steady window, 2,100 s; then
  the device's acts (two health changes, two events, an alarm raised and
  resolved), a reconnect (the relay cut for 15 s) and two `zenctl config
  set` from router B.

**The link failed in both runs, 1,876 s and 1,878 s into the steady window,
and never came back** (the findings below). The figures are the first
1,860 s of each steady window, before the failure (`analyze.py
--link-failed-at-s … --steady-end-s 1860`). The acts, the reconnect and the
config changes ran on a link that carried nothing, so these runs did not
measure them. Each act's cost below comes from the two earlier smoke runs,
whose setup was lossy.

The tables are [`summary.py`](summary.py)'s. Bare is decoded per element
([`zdecode.py`](zdecode.py)); "lean, derived" adds the lean stack's 16 bytes
to each of those SDUs, as the model does; the model is Appendix A's W2,
each element under its sender ([`model.py`](model.py)).

#### W2 A->B, a day

| Element | Bare, steady (1,860 s) | Bare, SDUs | Lean, derived | Model (lean) |
|---|---:|---:|---:|---:|
| telemetry | 7,864.5 KB | 62,627 | 8,866.6 KB | 5,166.7 KB |
| config | 5,754.9 KB | 30,194 | 6,238.0 KB | 5,766.1 KB |
| health | 199.3 KB | 2,724 | 242.9 KB | 239.5 KB |
| alarms | 102.4 KB | 557 | 111.3 KB | 121.4 KB |
| desired (the re-read's request) | 4.7 KB | 238 | 8.6 KB | 31.1 KB |
| events | not in the window | | | 4.4 KB |
| keep-alive | 22.9 KB | 22,947 | 390.1 KB | 440.6 KB |
| reconnect | not measured | | | 11.6 KB |
| **total** | **13,948.8 KB** | 119,288 | **15,857.4 KB** | **11,781.4 KB** |

Measured under the lean stack (driver A's counters): **128,160 SDUs,
9,841.4 KB and 32,805 s of air a day (38 % of the channel)**. Bare: 46,496 s
(54 %). The model: 39,271 s (45 %).

#### W2 B->A, a day

| Element | Bare, steady (1,860 s) | Bare, SDUs | Lean, derived | Model (lean) |
|---|---:|---:|---:|---:|
| desired (the author's reply) | 47.1 KB | 325 | 52.3 KB | 74.3 KB |
| keep-alive | 34.4 KB | 34,421 | 585.2 KB | 587.5 KB |
| reconnect | not measured | | | 1.6 KB |
| **total** | **81.5 KB** | 34,746 | **637.4 KB** | **663.4 KB** |

Measured under the lean stack (driver B's counters): **34,653 SDUs,
621.8 KB and 2,073 s of air a day (2.4 %)**. Bare: 272 s. The model:
2,211 s.

Both directions, under the lean stack, W2 measured **10.46 MB a day, 40 % of
RF's 25.9 MB**, against the model's 12.44 MB (48 %). The measured figure is
lower only because compress worked. `w2.zdict` was trained on this
workload's own SDUs (`results/w2-rf-bare-2/air.jsonl`), and 2,190 of the
2,759 A->B SDUs compressed, saving 153 KB in 31 minutes, 7.1 MB a day.
Without it, the lean stack derived from the bare run is 16.49 MB, 64 %.
And this is with router A dropping a fifth of the telemetry before the air
(the findings).

#### Each act's cost, from the smokes

Decoded from `w2-smoke-1` and `w2-smoke-2` (`results/w2-smoke-*`), bare,
A->B, each act's own messages:

| Act | Measured (bare) | Lean (+16 B an SDU) | Model (lean) |
|---|---:|---:|---:|
| health status change | 58–71 B, one SDU or a share | 74–87 B | 79 B a put |
| event (`oper_change`) | 116 B, one SDU (smoke-2; smoke-1's two never reached the air, not investigated) | 132 B | 291 B, with a larger document |
| alarm raised | 182 B, one SDU | 198 B | 197 B a transition |
| alarm resolved (put, then delete) | 247 B, two SDUs | 279 B | 197 B a transition |
| config change (`zenctl config set`) | not measured: its requests are 142 B B->A and the view it reads 1.6 KB A->B, then the link failed (smoke-2) | | |
| reconnect (a 15-s cut) | 2,613 B in 23 SDUs A->B, 1,426 B in 15 B->A (smoke-1), and the link did not come back | | 2,904 B and 397 B |

At W2's daily counts (4 health changes, 15 events, 5 raised, 5 resolved),
the acts are about 5 KB a day: noise beside the periodic elements.

### LoRa: the idle link and W1 (phase 2)

Four runs under the lean stack with `rf.zdict`, the fragment layer at SF12,
on the same path as RF's: W1 is phase 1's (gen at one a minute, four
subscribers on router B). The link opens and W1 sets up with no loss
(`LOSSLESS_SETUP=1`, `LOSS_START=0`), and each window runs at the medium's
5 %.

| Run | Windows |
|---|---|
| `sf7-lean-2` | 10 minutes idle, then 10 minutes of W1 |
| `sf9-lean-2` | 10 minutes idle, then 10 minutes of W1 |
| `sf12-lean-2` | 45 minutes idle, then 30 minutes of W1 starting 270 s after the owner |
| `sf7ka4-lean-1` | 10 minutes idle at keep_alive 4 |

The model is Appendix A's sections 8 and 9: each W1 reading in its own SDU,
one keep-alive a minute on W1's uplink where lease / keep_alive is 30 s, and
one per lease / keep_alive on an idle direction. "Other" SDUs are neither
keep-alives nor W1's data: the links opened again and their declarations.

#### LoRa, a day, per window and direction

| Run | Window | Direction | SDUs | Bytes | Airtime | Duty used | Keep-alives lost; links reopened (s into the window) | Model: SDUs, airtime |
|---|---|---|---:|---:|---:|---:|---|---|
| sf7-lean-2 | idle (600 s) | A->B | 3,744 | 117.4 KB | 270.1 s | 31 % | lost +451.8; reopened +180.5, +541.9 | 2,880 keep-alives, 148.2 s |
| sf7-lean-2 | idle (600 s) | B->A | 3,456 | 108.3 KB | 250.1 s | 29 % | lost +150.4 | 2,880 keep-alives, 148.2 s |
| sf7-lean-2 | w1 (600 s) | A->B | 9,504 | 508.5 KB | 999.2 s | 116 % | lost none; reopened +445.1, +566.5 | 7,200 (5,760 data), 753.9 s |
| | | of which keep-alive | 1,440 | 24.5 KB | 74.1 s | 9 % | | |
| | | of which data | 5,472 | 334.5 KB | 639.1 s | 74 % | | |
| | | of which other | 2,592 | 149.5 KB | 286.0 s | 33 % | | |
| sf7-lean-2 | w1 (600 s) | B->A | 4,320 | 165.5 KB | 356.5 s | 41 % | lost +415.0, +536.4 | 2,880 keep-alives, 148.2 s |
| sf7ka4-lean-1 | idle (600 s) | A->B | 5,760 | 97.9 KB | 296.4 s | 34 % | lost +270.4, +345.4; reopened never | 5,760 keep-alives, 296.4 s |
| sf7ka4-lean-1 | idle (600 s) | B->A | 5,760 | 97.9 KB | 296.4 s | 34 % | lost +60.4 | 5,760 keep-alives, 296.4 s |
| sf9-lean-2 | idle (600 s) | A->B | 4,032 | 134.9 KB | 986.2 s | 114 % | lost +383.3; reopened +141.3, +473.4 | 2,880 keep-alives, 474.8 s |
| sf9-lean-2 | idle (600 s) | B->A | 3,888 | 129.5 KB | 944.7 s | 109 % | lost +111.1 | 2,880 keep-alives, 474.8 s |
| sf9-lean-2 | w1 (600 s) | A->B | 7,632 | 381.6 KB | 2,493.9 s | 289 % | lost none; reopened +235.3 | 7,200 (5,760 data), 2,484.6 s |
| | | of which keep-alive | 1,296 | 22.0 KB | 213.7 s | 25 % | | |
| | | of which data | 5,040 | 294.5 KB | 1,860.1 s | 215 % | | |
| | | of which other | 1,296 | 65.1 KB | 420.1 s | 49 % | | |
| sf9-lean-2 | w1 (600 s) | B->A | 4,032 | 123.8 KB | 930.1 s | 108 % | lost +205.1 | 2,880 keep-alives, 474.8 s |
| sf12-lean-2 | idle (2700 s) | A->B | 896 | 32.9 KB | 1,737.5 s | 201 % | lost +1247.7; reopened +698.8, +2058.2 | 320 keep-alives, 474.5 s |
| sf12-lean-2 | idle (2700 s) | B->A | 832 | 27.8 KB | 1,527.2 s | 177 % | lost +427.4 | 320 keep-alives, 474.5 s |
| sf12-lean-2 | w1 (1800 s) | A->B | 6,960 | 263.9 KB | 14,031.8 s | 1,624 % | lost none; reopened never | 5,760 data, 22,743.6 s |
| | | of which data | 6,768 | 257.5 KB | 13,668.5 s | 1,582 % | | |
| | | of which other | 192 | 6.5 KB | 363.3 s | 42 % | | |
| sf12-lean-2 | w1 (1800 s) | B->A | 288 | 6.6 KB | 427.0 s | 49 % | lost none | 320 keep-alives, 474.5 s |

At SF12 the counters count tiles, the medium's frames: an SDU of the model
is two of them. The model's W1 at SF12 sends no keep-alive uplink, since the
owner speaks every minute and the keep-alive is due every 270 s.

## Findings

Each with its evidence: the run, and the counter, capture or log line it
rests on.

### W1 on RF (phase 1)

1. **Keep-alives match the model exactly, and are the bill.** One per 2.5 s
   on an idle direction, 23 a minute on W1's uplink (`results/lean-1`: 240
   and 230 a window). Under the lean stack they are 1.15 MB of W1's 1.43 MB a
   day, twice its data bytes, which is §6.2's point.
2. **Batching.** The model sends each W1 reading in its own SDU. zenoh batched
   the readings that reached the link while the paced driver was still
   sending: 2.5 SDUs a tick bare and 3.1 lean, against the model's 4
   (`results/{bare,lean}-1`: 25 and 31 data SDUs a window of 10 ticks). It
   depends on the owner's timing: gen's four puts come about 140 ms apart,
   about one 42-byte SDU's airtime.
3. **The put** is 35 bytes for a 2-byte value, decoded from the bare capture
   (`results/bare-1/air.jsonl`): push 1, key scope 1, QoS 2, put header 1,
   HLC 26, Encoding 1, length 1, payload 2. The model's put is 40 bytes: it
   gives the declared key's scope 2 bytes, where the measured one is 1, and
   assumes a 6-byte value, where gen's synthetic values were 1 to 3 bytes. The
   HLC's 26 bytes are exactly the model's.
4. **The frame** is 7 bytes (header 1, sequence number 4, QoS 2), where the
   model has 8 with a 5-byte sequence number. Each of the four transports seen
   began below 2^28, so its sequence number is a 4-byte varint.
5. **Compression.** 9 of the 31 data SDUs compressed: each carried two
   readings, whose second HLC repeats the first's 16-byte zid. They saved 153
   bytes in 10 minutes. Single readings and every keep-alive were stored, at
   the one byte the model counts. The dictionary saw only 2.3 KB of W1's own
   SDUs (49 of them); zstd asks for ten times the dictionary's size.

### W2 on RF (phase 2)

1. **W2 today fills RF's uplink.** A->B, bare, it is 13.9 MB and 54 % of the
   channel a day, 16.5 MB both directions under the lean stack without
   compression (64 %), and 10.5 MB (40 %) with a dictionary trained on its
   own traffic. The model says 12.4 MB (48 %). Telemetry and config are 98 %
   of it, as the model says. Under the lean stack, derived, telemetry is 72 %
   over the model's (8.9 MB against 5.2) though only 81 % of it reached the
   air, and config 8 % over (6.2 MB against 5.8).
2. **Every telemetry put carries its key's text.** Each one is the far
   subscriber's `zk2` scope plus a 47-byte suffix,
   `/h-3fa9c2d41b7e/rf0/gendev.v1/stream/temperature`: 85 bytes for a
   5-character gauge, 113 for a counter with its Epoch attachment, against
   the model's 37 and 59 with a 2-byte declared scope (decoded from
   `w2-rf-bare-3`'s capture). The puts that carried a short suffix were
   those under a key expression router A had declared toward B with a
   queryable: `…/health.v1/state` with `/status`, `…/alarms.v1/state` with
   `/alarms/<key>`. A stream has no queryable, and router A declared nothing
   under it.
3. **Router A drops a fifth of the telemetry before the air.** Of the 2,232
   puts the device made in the 1,860-s window, 1,819 reached the air (81 %).
   The rest were dropped at router A by the streams' `drop` congestion
   control, the last of each tick's twelve the most: temperature 185 of 186,
   resets_total 98 of 186. The 5 % loss then left the far side 77 % of
   them. The uplink is 54 % busy on average, but each tick's 1.1 KB burst
   meets the config view's 1.6 KB (8 SDUs, 5.3 s of air) every 28.6 s, and a
   2,400-bit/s link drains 300 bytes a second.
4. **Config's re-puts are 8.6 % larger than the model said, and a view
   rarely crosses whole.** Each 28.6 s the device re-puts its view (1,592 B
   bare, 8 SDUs) and its status (314 B bare, 2 SDUs). With the lean stack's
   16 B an SDU that is 2,066 B, where the model had 1,902 B before its
   correction. That model gave the puts no key text, where they carried their
   key's text all run, since they were put before router A declared their
   scope. It also took config.v1's 1,286-B view and 330-B status, where the
   example's are 1,476 and 226 B. The
   corrected Appendix A gives exactly 2,066 B for these payloads, and 1,980 B
   for config.v1's. A view crosses only when all 8 of its SDUs do: 38 of 65
   did (58 %), where 0.95^8 is 66 %.
5. **Once its transport closed under W2's load, the link never came back.**
   In each of the four W2 runs where it closed, no new transport carried
   data again before the run ended:

   | Run | What closed the first transport | Router A's closes after | Data at B after |
   |---|---|---:|---|
   | w2-rf-bare-3 | 3 B->A keep-alives lost in a row (+1,869.6, +1,872.1, +1,874.6 s): 10.0 s of silence, the whole lease | 37 in 683 s | none |
   | w2-rf-lean-1 | 3 B->A keep-alives lost in a row too, the last at +1,878.4 s (the relay's seed is 0 in every run) | 37 in 681 s | none |
   | w2-smoke-1 | the relay's 15-s cut | 2 in the 40 s left | none |
   | w2-smoke-2 | nothing: the transport in place, 17.8 s after a `zenctl config set` | 7 in the 107 s left | none |

   Each new transport's handshake completed and router A sent its
   declarations, then router A logged "Unable to push non droppable network
   message to <router B>. Closing transport!" and "Initialisation of the
   transport … was cancelled before its RX task was started", 11 to 16 s
   after the open, and opened another. A non-droppable push waits at most
   `wait_before_close`, 10 s here. The radio was not full while it waited:
   the first four lives in `w2-rf-bare-3` carried 0.6–1.2 KB A->B in 11–16
   s (2–4 s of air), and smoke-2's 17.8 s 1.9 KB (6.3 s). Driver A dropped
   nothing and failed nothing (`sdu/dropped_total`, `sdu/tx_failed_total`
   stayed 0), and its scheduler held 0 to 3 batches when smoke-2's push
   stalled. After the first close in `w2-rf-bare-3`, its `sched/queue_depth`
   rose to 97 batches and `sched/stale_total` to 109. **The cause is not
   established.** These runs show where it is not (the air, the driver's
   drops), not where it is. Appendix A prices 4 reconnects a day at 13 KB;
   on this path one is a link lost for good.

   At W2's 5 % loss, three keep-alives in a row are lost with p = 1.25 ×
   10^-4. On the downlink's 34,560 a day, that is about 4 lease expiries a
   day from B->A alone, if three always expire it, as they did here.
6. **The desired target never re-reads its document on RF.** Its GETs time
   out after 1 s (`TargetStartup::timeout`, "1 s by default"). Router A
   forwarded each and logged "Didn't receive final reply … Timeout(1s)", and
   the author's reply came back to "Route reply: Query not found!": 3.7–5.2
   s after the GET in the steady window bare, 3.3–3.6 s lean, and 5.4–7.4 s
   for the 2 GETs at the start. All 8 GETs of each run timed out (2 at the
   start, then one every 300 s; one lean reply never came). The device's
   report said `"source": "baseline"` for the whole run, and it never
   adopted the document its author put. The reply still crossed every time: 47 KB a day B->A, for
   nothing. In `w2-smoke-1`, lighter and shorter, one GET succeeded at about
   5 minutes.
7. **`zenctl config set` with its default `--timeout` (5 s) draws silence on
   RF.** Both sets in `w2-smoke-2` exited 2 with `"silence"`, presence
   `present` and then `no_token_visible`. w2-drive.py gives it 30 s, but in
   the costing runs the link had already failed when the sets ran (both
   exited 2, `no_token_visible`), so no set at 30 s was measured.
8. **Not measured:** the acts, the reconnect and the config changes in the
   costing runs (finding 5); W2 across a face (`--across-face`, skipped by
   decision); W2 on LoRa and SBD.

### A declaration lost in the setup is never repaired

Four bare W2 runs were made with the medium's 5 % loss from the first datagram
on: `w2-smoke-1` and `w2-smoke-2` (60-s steady windows) and the stopped
attempts `w2-rf-bare-1` and `w2-rf-bare-2` (12 and 10 minutes, no acts).
[`setup-loss.py`](setup-loss.py) reads them
([`results/setup-loss.json`](results/setup-loss.json)). Each lost 33 to 70
SDUs, and 3 of them carried declarations. Two kinds of loss followed, each
for the whole life of the transport:

- **A put whose key scope was never declared is dropped by the far router,
  silently.** Router A sends health's and alarms' puts on the
  `…/<iface>/state` key expression it declared toward B with the state's
  queryable, plus `/status` or `/alarms/<key>`. When that declaration's SDU
  is lost, router B drops every later put on it and logs "Route data with
  unknown scope", one line per put:

  | Run | Declaration lost (s after the link opened) | Never received at B | Delivered by the relay | B's "unknown scope" lines |
  |---|---|---|---:|---:|
  | w2-smoke-1 | `…/alarms.v1/state` and `…/health.v1/state`, A->B, +21.7 s | alarms, health | 6 + 14 | 20 |
  | w2-rf-bare-1 | `…/health.v1/state`, A->B, +22.5 s | health | 25 | 25 |
  | w2-rf-bare-2 | `…/alarms.v1/state`, A->B, +20.8 s | alarms | 6 | 6 |
  | w2-smoke-2 | `…/worker.v1/state` (the desired report), A->B, +22.5 s | nothing | — | 0 |

  In `w2-rf-bare-1`, 25 health puts over 12 minutes crossed the air and none
  reached the reader; router B's line came 0.1–0.2 s after each. The
  device's alarms or health simply stop for the host, with nothing on the
  owner's side to say so. In `w2-smoke-2` the lost `worker.v1/state`
  declaration cost nothing measurable: the report's two puts went at +20.9
  and +22.1 s, before it, with their key's text. Telemetry lost nothing to
  this in any run, since its puts carry their key's text too (the W2
  findings).
- **A queryable whose declaration was lost is never asked.** In three runs
  the B->A SDU declaring the author's `zk2/fleet/desired/tasking.v1/state/**`
  queryable was lost (+13.2 s). In those runs not one of the device's
  desired GETs crossed A->B: router A knew no queryable to route them to. In
  `w2-smoke-1`, which kept that SDU, 3 crossed. The desired target re-reads
  nothing, and nothing on the air says so.

The same SDU was lost at +13.2 s in three runs because the relay's seed is 0
in every run, so these four runs are not four independent samples. This is
[Z20]'s "a lost `DeclareKeyExpr` is never repaired without link ARQ",
observed for a key expression and for a queryable. It is why the costing runs
open the link and make the setup with no loss, and take the medium's
loss for the windows (`LOSS_START=0` with `LOSS_AT_STEADY=0.05` for W2,
`LOSSLESS_SETUP=1` for W1): every declaration crosses, and the figures are a
link's that came up whole. The control, `w2-rf-bare-3`, set up with no
loss: every resource reached B, all 8 of the device's desired GETs crossed
A->B, and router B logged no unknown scope. The 16 SDUs with declarations it
lost all came after its link failed (W2's finding 5).

### LoRa (phase 2)

1. **At keep_alive 2, every lost keep-alive closes the link.** zenoh-modem's
   deploy/README.md gives LoRa two keep-alives a lease: every 30 s in a 60-s
   lease, every 270 s in SF12's 540-s one. A single loss then leaves the
   receiver a whole lease of silence, and it closes the transport:

   | Run | Window | Keep-alive lost (s into the window) | Link opened again | After |
   |---|---|---|---|---:|
   | sf7-lean-2 | idle | B->A +150.4 | +181.5 | 31 s |
   | sf7-lean-2 | idle | A->B +451.8 | +541.9 | 90 s |
   | sf7-lean-2 | W1 | B->A +415.0 | +446.1 | 31 s |
   | sf7-lean-2 | W1 | B->A +536.4 | +567.5 | 31 s |
   | sf9-lean-2 | idle | B->A +111.1 | +142.3 | 31 s |
   | sf9-lean-2 | idle | A->B +383.3 | +473.4 | 90 s |
   | sf9-lean-2 | W1 | B->A +205.1 | +271.3 | 66 s |
   | sf12-lean-2 | idle | B->A +427.4 | +700.8 | 273 s |
   | sf12-lean-2 | idle | A->B +1,247.7 | +2,099.2 | 852 s |

   Every keep-alive lost at keep_alive 2 closed the link, and nothing else
   closed it. A lost B->A keep-alive: router A's lease runs out one
   keep-alive period later, and it opens a new transport at once. Its first
   InitSyn draws router B's keep-alive on the old one ("Received an invalid
   message in response to an InitSyn … KeepAlive"), and the retry 1 s later
   opens the link. A lost A->B keep-alive: router B closes at its lease, and
   router A, which then hears nothing, closes only at its own, a lease
   later. In `sf9-lean-2`'s W1, a handshake SDU was lost too, and the open
   took five InitSyns over 36 s. At SF12 the same A->B loss left router B
   closed at +1,517.7 s and router A unaware until its own lease ran out at
   +2,056.9 s; the open then timed out once (40 s), and the link was back at
   +2,099.2 s, after about 580 s with no link at either end.

   At keep_alive 4 (`sf7ka4-lean-1`), 3 keep-alives were lost (B->A +60.4 s,
   A->B +270.4 and +345.4 s) and the link stayed up: a loss leaves 30 s of a
   60-s lease.
2. **A reopen costs far more air than the keep-alives it saves.** It is a
   handshake, both routers' admin-space declarations, and every zk2
   declaration again: at SF7, 3–4 SDUs and 220–250 B each way on the idle
   link, and with W1's owner up 9 SDUs and 0.5 KB A->B, 5 SDUs and 0.4 KB
   B->A. Idle at keep_alive 2, SF7 paid 250–270 s of air a day each way, of
   which 117–137 s were reopens, where the model says 148 s with no reopen.
   Keep_alive 4 doubles the keep-alives and costs 296 s, the model's figure
   for it exactly, 10–19 % more than keep_alive 2 with its reopens, and
   the link stays up. SF9 paid 945–986 s a day each way idle, more than
   EU868's 864 s, against the model's 475 s. And a reopen made at the
   medium's loss can lose a declaration again: in `sf7-lean-2`'s W1, the
   first A->B SDU after the reopen at +445.1 s was lost at +446.9 s, and
   router B logged "Unknown wire expr" 0.2 s later.
3. **W1 does not fit EU868's duty on today's wire at SF7 or slower.** At SF7
   W1's uplink took 999 s of air a day, 116 % of the 864 s, against the
   model's 754 s (87 %): its data 639 s (the model 680 s), its keep-alives
   74 s (the model's exactly), and the reopens 286 s. At SF9 it took 2,494 s
   (289 %), the model 2,485 s. At SF12 it took 14,032 s, 16 times the duty,
   against the model's 22,744 s: zenoh batched the readings while the tiles
   paced (141 tiles for 30 ticks, 4.7 a tick, where the model sends 4 SDUs of
   2 tiles), and a lost tile lost its whole SDU, so the far side got 102 of
   W1's 120 readings (85 %).
4. **Keep-alives and data match the model where the link holds.** W1's
   uplink keep-alives at SF7 were 1,440 a day, the model's exact figure, and
   the readings went nearly one to an SDU, as the model assumes (38 data
   SDUs for 40 readings): at 5,469 bit/s a 58-byte SDU takes 86 ms, so
   gen's puts 140 ms apart rarely meet. SF7 at keep_alive 4 matched
   the model to the SDU and the second (5,760 keep-alives and 296.4 s each
   way). The gaps from the model are the reopens.
5. **At SF12 a reopen costs more than the duty allows.** Its two reopens in
   45 idle minutes took 20 tiles A->B and 18 B->A, about 21 s and 18 s of
   air each. At the window's rate that is 1,358 s and 1,148 s a day, which
   takes each direction to 177–201 % of EU868's duty idle, where the
   model's keep-alives alone are 475 s (55 %). The keep-alives themselves
   matched it, one tile of 23 bytes every 270 s (8 in each direction in the
   window, fewer than 10 because the link was down).

## zk2c on RF, and W1-73 at LoRa SF7 (#989's zk2c half)

Run 2026-11-08, from zenkey at 2fa3e6ab (#1098's branch, with modem.v3 3.4
and link.v2 0.12). The same path, drivers and fork as phases 1 and 2, with
three changes: zk2c's owners, link.v2's transport, and, for W1-73, rf0's face.
The face policies were #1098's, which list each face's events: the rendered
access control in `results/zk2c-*/configs/` denies the unlisted ones by name.
Before #1098 a face lists none, and every exposed `face/` event crosses it,
`face/device_fault` across lora0's SF7 face among them. The SF9 and SF12
runs came later, 2026-11-21, from main at link.v2 0.17: their section says
what moved. W2 with owner heartbeats came the same day, from #1110's H3:
its section says what differs.

- **The owners are zenkey's reference, not a device's.**
  - modem.v3's owner is **`zenctl gen`**, zenkey's generator, under
    zenoh-modem's catalog (29bf7d47). zenoh-modem's own owner, modem-mgmt, is
    not ported yet (zenoh-modem#214), so no figure here is modem-mgmt's.
    gen's synthetic records hold every reading at random on even minutes and
    none on odd ones, so its frames are full records of all 73 readings or
    of none. The model's "24 of 73 moving" is computed, not measured
    (Appendix A section 10).
  - W2's device is the owner example (`--health --alarms --config-v2
    --desired-target`), with `zenctl gen` beside it at the same address for
    gendev.v1 1.1's `frames/sample` (`--i-know`). The owner example drives no
    contract's frame writer but its scenarios'.
- **link.v2's transport** on both routers, as rf0's policy states it: lease
  240 s, keep_alive 4, `wait_before_close` 240 s, 16-bit sequence numbers,
  batch 222 (render.py's `rf-zk2c`; `sf7-zk2c` is lora0's SF7 face, lease
  120 s).
- **rf0's face (W1-73 and the honesty checks).** Router A runs what `zenctl
  acl gen --face-policy zenohd-rf0.face.toml --json5` writes, in `router`
  mode, with three settings the generator does not write:
  - `timestamping: { enabled: { router: false } }` (core §8.2's SHOULD: a
    router stamps every unstamped put it routes otherwise);
  - usrpwd credentials, since the plan's subjects are user names: gen
    presents `rf-node`, router B `ground`;
  - the far side opens the lane, the reverse of phases 1 and 2. zenoh's
    usrpwd authenticates the opener to the acceptor, so router A can only
    bind router B to `ground` if router B opens.

  Router B runs `--far-config`'s transport as a peer. The far side reads with
  `zenctl echo --across-face h-3fa9c2d41b7e/rf0 --face-policy … --face rf`,
  whose scope is the plan: frames/fast and the three `face/` events.
- **W2 has no face.** Its standard contracts (health.v1, alarms.v1, rfcfg.v1)
  expose nothing `link`, so a face would carry none of them. Its routers run
  link.v2's transport as peers, as phase 2's did. The far side reads as core
  §8.5 asks across a constrained link: one `zenctl echo --across-face` per
  interface, each resource by its exact key, never `zk2/**`.
- **Loss as phase 2 had it.** Each link opens and its owners set up with no
  loss, and each window runs at the medium's 5 % (`LOSS_START=0` with
  `LOSSLESS_SETUP=1` or `LOSS_AT_STEADY=0.05`). The relay's seed is 0 in
  every run, so runs see the same loss sequence.
- The harness ran on cores 4–5.

| Run | Path, stack | Windows | Owner's run |
|---|---|---|---|
| `zk2c-w1-bare-1` | RF, rf0's face, no layers | idle 1,200 s, W1-73 1,800 s | 32-bit |
| `zk2c-w1-lean-1` | RF, rf0's face, lean (`rf.zdict`) | idle 600 s, W1-73 1,800 s | 16-bit (`--run-file`) |
| `zk2c-sf7-lean-1` | LoRa SF7, lora0's face, lean (`rf.zdict`) | idle 600 s, W1-73 1,800 s | 16-bit |
| `zk2c-sf9-lean-1` (2026-11-21) | LoRa SF9, lora0's SF9 face, lean (`rf.zdict`) | idle 1,920 s, W1-73 3,600 s, a frame every 120 s | 16-bit |
| `zk2c-sf12-lean-1` (2026-11-21) | LoRa SF12, lora0's SF12 face, lean (`rf.zdict`), 51-B tiles | idle 3,600 s, W1-73 7,200 s, a frame every 900 s | 16-bit |
| `zk2c-w2-bare-1` | RF, no face, no layers | steady 2,100 s, the acts, a 245-s cut, two config sets | 32-bit |
| `zk2c-w2-lean-1` | RF, no face, lean (`w2.zdict`) | as `-bare-1` | 16-bit |
| `zk2c-honesty-1`, `zk2c-honesty-reset-1` | RF, rf0's face, no layers | 75 % loss for 240 s, a 392-s cut, a restart | 32-bit |
| `zk2c-w2-bare-2` (2026-11-21, #1110) | RF, no face, no layers; the device sends owner heartbeats | as `zk2c-w2-bare-1` | 32-bit |
| `zk2c-w2-lean-2` (2026-11-21, #1110) | RF, no face, lean (`w2.zdict`); the device sends owner heartbeats | as `-bare-1` | 16-bit |
| `zk2c-w2-owner-honesty-1` (2026-11-21, #1110) | RF, no face, no layers; owner heartbeats, and the host's `watch` and `health` | steady 1,200 s, the acts, a 245-s cut | 32-bit |
| `zk2c-w2-v2-bare-1` (2026-11-27, #986, #984) | RF, no face, no layers; desired.v2 by its 300-s period, alarms.v2 | as `zk2c-w2-bare-1`, the v2 acts and the host's probes among the acts | 16-bit |
| `zk2c-w2-v2-watch-bare-1` (2026-11-27) | the same, desired.v2 by the watch, tasking.v2 at a 600-s horizon | the same | 16-bit |
| `zk2c-w2-v2-lean-1` (2026-11-27) | RF, no face, lean (`w2.zdict`); desired.v2 by its period, alarms.v2 | the same | 16-bit |

### The idle link at link.v2's transport

Each direction, a day: **1,440 keep-alives, one a minute, the model's figure
exactly**: 1.4 KB bare, 24.5 KB and 81.6 s of air under the lean stack
(`zk2c-w1-bare-1`'s 1,200 s and `zk2c-w1-lean-1`'s 600 s). Phase 1's lease of
10 s sent 34,560 a day each way, 24 times as many. One keep-alive was lost
each way in 1,200 s and the link held: at keep_alive 4 a close takes three
lost in a row.

### W1-73 across rf0's face

What crosses is the plan's and nothing else: frames/fast once a minute and
`face/device_fault` once a minute, gen's rate at that event's ceiling,
`low`. The two `rare` events fell outside the windows. gen also writes the 73
JSON streams, the 9 deprecated frames, `frames/slow` and `frames/paid`, every
state and the four JSON events, and none of it reached the air. No heartbeat
crosses either: modem.v3's collections are `host`, and the plan denies
`freshness.v2`'s stream on the face. The frames are the liveness evidence.

| A day, A->B | Bare (`-bare-1`) | Lean (`-lean-1`, the counters) | Model (lean) |
|---|---:|---:|---:|
| frames/fast, gen's (a full frame of every reading, then one of none) | 176.4 KB, 1,440 SDUs | 207.3 KB, 2,160 SDUs | 211.0 KB on the same frames (Appendix A §10) |
| `face/device_fault`, one a minute | 172.9 KB, 1,440 SDUs | 162.9 KB, 1,440 SDUs | not modelled |
| uplink keep-alives | none | 336, 5.7 KB | 0 to 1,440, at most 24.5 KB (link.v2 §2.16) |
| **total** | **349.3 KB, 1,164 s of air (1.3 %)** | **377.2 KB, 1,257 s (1.5 %)** | |
| W1-73 as the report prices it: a full frame every 15 minutes, a delta each minute with 24 readings moving | | | **168 KB**, corrected (Appendix A §8 said 200 KB); 168 to 192 KB with the uplink's keep-alives |

B->A, a day: the 1,440 keep-alives, 24.5 KB lean, the model's figure.

The uplink's keep-alives are a range in the model since link.v2 0.17. rf0's
frame goes every 60 s and its keep-alive interval is 60 s, so a keep-alive
falls due with each frame: none to one a gap (§2.16). The first model said
none. Both runs are asserted inside the range, in Appendix A §10 and in
`model.py` against each `analysis.json`: none bare, and 336 a day lean, 7
gaps of the window's 30.

- **The frame's anatomy is the model's, byte for byte.** A full record of all
  73 readings is 199 B (core §7.7: the time, `full`, then a presence bit and
  the code per field, 1,469 bits of codes at zenoh-modem's ranges), a record
  of none 15 B. With the envelope that is 205 B and 21 B on a 16-bit run, 207
  B and 23 B on gen's 32-bit one in the bare run. The put adds 5 B (no QoS
  extension: frames/fast is priority `data`) and the face's frame 3 B (16-bit
  sequence numbers): SDUs of 215 B and 30 B, exactly as decoded.
- **Under the lean stack a full frame takes two SDUs.** Its 210-B put does
  not fit the 201 B a 220-B SDU leaves after the layers and the frame, so
  zenoh fragments it: 220 B and 29 B. Compression stored 43 of the 82 SDUs
  and saved 579 B in 30 minutes, 7 % of the uplink's bytes, nearly all of it
  on the events' JSON.
- **The model's 200 KB was the model's miss, not the wire's.** Appendix A §8
  priced illustrative widths (254-B keyframe, 98-B delta) on a `data_low`
  frame with the QoS extension and a 5-byte frame. At zenoh-modem's ranges,
  on the face's 3-byte frame with a 16-bit run, a full frame is 205 B and a
  delta with 16 counters and 8 gauges moving is 84 B: **168 KB a day** under
  the lean stack, 0.65 % of RF. zenoh-modem's README says 177 KB for the same
  case.
- **An event at its ceiling costs as much as the frames.**
  `face/device_fault` at `low` is 1,440 a day of 97–98 B of form 4 (its JSON
  words) in 120-B SDUs bare: 173 KB a day, 163 KB lean, where `rf.zdict`
  compressed it. link.md prices rf0's three events at 417 KB a day at their
  worst; a device that faults once an hour sends 3 KB.

### W2 zk2c

Both directions, a day, as the report's §2.2 makes W2: the steady window's
periodic elements, its events at W2's 15 (gen put 60 an hour, so the window
gives each one's cost), the acts at their daily counts and 4 reconnects.
"Lean, derived" is the bare run (`zk2c-w2-bare-1`, decoded, 32-bit runs) plus
16 B an SDU; the model is Appendix A §6's zk2c column, each element under its
sender ([`model.py --zk2c`](model.py)).

| Element, a day | A->B bare | A->B lean, derived | B->A lean, derived | Model (lean) | Lean / model |
|---|---:|---:|---:|---:|---:|
| telemetry: frames/sample, three 10-s records a frame every 30 s | 288.0 KB | 334.1 KB | | 334.1 KB | 1.00 |
| heartbeat: health.v1's and rfcfg.v1's every 28.5 s, alarms.v1's every 427.5 s | 154.4 KB | 238.0 KB | | 51.8 KB | 4.59 |
| desired: a GET every 300 s, the author's reply | 19.4 KB | 26.0 KB | 58.3 KB | 27.4 KB | 3.08 |
| health, 4 changes | 0.1 KB | 0.2 KB | | 0.2 KB | 1.36 |
| alarms, 5 raised and 5 resolved | 1.7 KB | 2.0 KB | | 0.4 KB | 4.60 |
| config, 5 changes | not measured | | 0.6 KB (the requests) | 0.3 KB | |
| events, 15 | 1.1 KB | 1.3 KB | | 0.7 KB | 1.97 |
| keep-alives | | | 22.4 KB | 24.5 KB | 0.91 |
| reconnects, 4 | 4.9 KB | 5.6 KB | 6.6 KB | 3.5 KB | 3.45 |
| **total** | **469.6 KB** | **607.1 KB** | **87.9 KB** | **443 KB** | **1.57** |

Measured under the lean stack with 16-bit runs (`zk2c-w2-lean-1`, the
counters): **668.8 KB a day both directions as W2 makes it, 2.6 % of RF**
(steady window 688.9 KB A->B and 71.5 KB B->A, 2,296 s and 238 s of air,
with gen's 1,399 events; less those and plus the acts and reconnects as
above). The corrected model, Appendix A §10, says 674 KB.

- **zk2c cuts W2 by 25 times on RF, not 41.** Today's W2 crossed 16.49 MB a
  day under the lean stack, derived (phase 2), and the model says 18.37 MB:
  25 and 27 times the 0.67 MB measured. Against phase 2's lean run with a
  dictionary trained on its own traffic, 10.46 MB, it is 16 times. The
  channel goes from 64 % to 2.6 %. The first model's 443 KB (1.7 %, 41 times)
  missed by 1.5 times, in three elements: the heartbeats, the desired
  re-reads and the reconnects.
- **The heartbeat is one put per interface, not 18 B inside a frame.**
  freshness.v2 sends a heartbeat per interface with a collection, each on its
  own key (§2.4). W2's device has three: health.v1's and rfcfg.v1's every
  28.5 s (16 B each on a 16-bit run) and alarms.v1's every 427.5 s. Appendix
  A §6 folded one 18-B section into every third telemetry frame. Measured:
  238 KB a day, 4.6 times the model's 52 KB, between every pair of
  heartbeats in one SDU (199 KB) and each in its own (257 KB). The miss is the
  model's: it priced a design freshness.v2 did not take. #1110 then built
  one heartbeat per owner and class, as an option, and W2 was measured
  again with it ([below](#w2-with-owner-heartbeats-1110)).
- **The desired re-read is today's GET.** Each 300 s the target asks with a
  12-B request and the author answers its whole document, 166 B, then the
  final: 64 KB a day, where §6's conditional GET answering an unchanged slice
  in 3 B gave 27 KB. The target's 1-s default still loses some: 4 of the
  device's 12 GETs drew "Didn't receive final reply … Timeout(1s)", the two
  at its start among them, and their replies crossed for nothing. The first
  re-read, 300 s in, brought the document, and the device adopted it (its
  report changed), where none of phase 2's did (its finding 6).
- **Telemetry is the model's exactly.** Each 30-s frame of three records was
  93 B on gen's 32-bit run in a 100-B SDU bare, where the model's 5-byte frame
  and 16-bit run give 100 B too. With a 16-bit run (the lean run) it is 91 B,
  in 98 B on the 3-byte frame of 16-bit sequence numbers.
- **Alarms stay JSON.** An alarm raised took 154 B of air, a resolve 194 B
  in two SDUs (a put, then the delete), against §6's 8-B transition: alarms.v1
  has no packed record. Re-puts are gone (the heartbeat confirms), so at 10
  transitions a day it is 2 KB.
- **After a 245-s cut the link opened again, and then A->B carried
  nothing.** The reopen's handshake and declarations crossed: 1.2 KB A->B
  (presence and the device's declarations) and 1.4 KB B->A (the six
  readers' and the author's subscriptions again), against §6's 0.8 KB and
  0.1 KB. After them nothing crossed A->B, no frame, heartbeat or
  keep-alive, until the lease closed the transport. This is phase 2's W2
  finding 5 at zk2c's load, and #1109 reproduced it at 0 % loss: router A's
  routing waits in a push begun while it re-declares toward the new face,
  and driver A sends nothing more. The reconnect element is costed from the
  reopen alone.
- **The two config changes drew silence.** After the reconnect, both
  `zenctl config set` on rfcfg.v1 (config.v2) exited 2 with `"silence"`,
  presence `no_token_visible`, after their 30 s: router A, which sent
  nothing back, never answered the tool's liveliness read (#1109). Phase 2's
  sets drew silence too (W2 finding 7). The config element is not
  measured.
- **Compression does nothing for zk2c.** The lean run's compress layer
  stored 221 of 245 SDUs A->B and saved 69 B in 35 minutes, and stored
  every keep-alive: zk2c's records leave it nothing, as the report says
  (§3.3). W1-73's 7 % came from `face/device_fault`'s JSON.

### W1-73 at LoRa SF7

The same W1-73 across lora0's SF7 face, lean (`zk2c-sf7-lean-1`): lease
120 s, keep_alive 4 (a keep-alive every 30 s), 16-bit sequence numbers;
frames/fast every minute, the two `rare` events listed and none in the
window, `face/device_fault` denied.

| A day | Measured | Duty (864 s) | Model |
|---|---:|---:|---:|
| idle, each way | 2,880 keep-alives, 148.2 s | 17 % | 148.2 s |
| W1-73 uplink, gen's frames | 2,160 SDUs, 363.0 s | 42 % | 365.5 s (Appendix A §10) |
| W1-73 uplink keep-alives | 2,160, 111.1 s | 13 % | 1,440 to 2,880, 74.1 s to 148.2 s (link.v2 §2.16) |
| **W1-73 uplink** | **474.2 s** | **55 %** | 439.6 s to 513.7 s |
| W1-73 downlink | 2,880 keep-alives, 148.2 s | 17 % | 148.2 s |

- **W1-73 fits EU868's duty at SF7 under zk2c.** All 73 readings a minute
  took 55 % of the uplink's 864 s, where today's 4 link readings took 116 %
  (`sf7-lean-2`). With 24 readings moving, the corrected frames take 288 s,
  and the uplink 362 s to 436 s with its keep-alives (42–50 %), 399 s at the
  share this run sent.
- **The link never reopened.** Five keep-alives were lost in 40 minutes,
  never two in a row; at keep_alive 2 each would have closed it (phase 2's
  LoRa finding 1).
- **Frames spared one uplink keep-alive in four.** Frames a minute apart
  should leave one keep-alive between two (link.v2 §2.16, "Data spares the
  keep-alives"). The uplink kept its 30-s cadence (keep-alives at +0.13 s,
  +60.13 s, +90.13 s with frames at +30.4 s and +90.2 s), 37 s of air a day
  more than one a gap, and inside the one to two a gap that §2.16 states
  since 0.17. On RF, at a 60-s interval, the frames spared every one.
  - **Why (#1108).** zenoh 1.10.1's transmit task, and the fork's, counts
    each keep-alive interval from the last batch it sent or its last
    keep-alive. Nothing received counts. Between frames a minute apart at a
    30-s interval, the first keep-alive is certain, and the second falls
    due when the next frame does. It goes first when that frame is later on
    its schedule than the one before, which happened in half the gaps here.
  - **RF's 60-s interval** is the same race with one keep-alive a gap: none
    went first bare, 336 a day under the lean stack.
  - link.v2 0.17 §2.16 states it, and its scenarios' §5 runs it in process.

### W1-73 at LoRa SF9 and SF12

Run 2026-11-21, from zenkey at 5265cc52 (main: link.v2 0.17, modem.v3 3.4,
catalog 29bf7d47), with the same drivers, fork and path. The same W1-73
across lora0's SF9 face and its SF12 face, lean, each at its face's cadence
and transport:

| | SF9 (`zk2c-sf9-lean-1`) | SF12 (`zk2c-sf12-lean-1`) |
|---|---|---|
| The frame the face carries | `frames/fast` every 120 s | `frames/slow` every 900 s |
| Lease / keep_alive | 480 s / 4: a keep-alive every 120 s | 3,600 s / 4: every 900 s; open, accept and interests 40 s |
| Medium | 115-B frames, paced at 1,758 bit/s | 51-B tiles of a 222-B fragment layer, paced at 293 bit/s |
| zenoh's batch, the driver's under the lean stack | 101 B | 210 B |
| Events the face lists | the two `rare` ones | none |
| Windows | idle 1,920 s, 16 intervals; W1-73 3,600 s, 30 frames | idle 3,600 s, 4 intervals; W1-73 7,200 s, 8 frames |

The model is [`model.py --zk2c`](model.py): Appendix A §10's frames, and
the uplink's keep-alives beside them as a range, fewest to most, by link.v2
§2.16.

| A day, SF9 | Measured | Duty (864 s) | Model |
|---|---:|---:|---:|
| idle, each way | 720 keep-alives, 12.2 KB, 118.7 s | 14 % | 720, 118.7 s |
| W1-73 uplink, gen's frames | 1,440 SDUs, 112.3 KB, 642.9 s | 74 % | 642.9 s |
| W1-73 uplink keep-alives | 624, 10.6 KB, 102.9 s | 12 % | 0 to 720, 0 s to 118.7 s |
| **W1-73 uplink** | **2,064 SDUs, 122.9 KB, 745.8 s** | **86 %** | 642.9 s to 761.6 s |
| W1-73 downlink | 720 keep-alives, 12.2 KB, 118.7 s | 14 % | 720, 118.7 s |

| A day, SF12 | Measured | Duty (864 s) | Model |
|---|---:|---:|---:|
| idle, each way | 96 keep-alives, 2.2 KB, 142.3 s | 16 % | 96, 142.3 s |
| W1-73 uplink, gen's frames | 336 tiles, 15.3 KB, 757.7 s | 88 % | 757.7 s |
| W1-73 uplink keep-alives | 72, 1.7 KB, 106.8 s | 12 % | 0 to 96, 0 s to 142.3 s |
| **W1-73 uplink** | **408 tiles, 16.9 KB, 864.5 s** | **100 %** | 757.7 s to 900.1 s |
| W1-73 downlink | 96 keep-alives, 2.2 KB, 142.3 s | 16 % | 96, 142.3 s |

Each tolerance is asserted, in Appendix A §10 and in `model.py` against
each run's `analysis.json`: gen's frames' air within 1 % (met to 0.1 s), the
uplink's keep-alives inside §2.16's range, the uplink's air inside the
range's ends, and an idle direction's keep-alives exact.

- **The frames are the model's.**
  - At SF9 a full record of all 73 readings, 205 B in a 210-B put, crossed
    in three SDUs of 115, 115 and 38 B, and a record of none, 21 B, in one
    of 44 B: the model's, byte for byte.
  - At SF12 the driver leaves zenoh a 210-B batch under the lean stack,
    where the face policy states 224 B (finding 10). So zenoh cut a full
    record in two fragments. They crossed as four 51-B tiles and one of 32
    to 34 B, then a 31-B tile. A record of none took one 50-B tile. The
    model's tiles are within 2 B of these, on the same step of LoRa's time
    on air.
  - The far reader got payloads of 205 B and 21 B, every record valid, its
    73 fields all known or all absent. The sizes leave no room for a
    router's stamp: no put carried one.
  - Compression stored every uplink SDU at SF9, and saved 32 B in two hours
    at SF12.
- **The cadence is the face's, the frames are gen's.** gen wrote a full
  record on every other tick, every 240 s at SF9 and every 1,800 s at SF12.
  The faces state a full record every 900 s and 3,600 s (`keyframe_s`),
  with deltas between. The corrected frames of that case, 24 readings
  moving, are computed and not measured: 502 s a day at SF9 and 762 s at
  SF12 (Appendix A §10). The mapping's notes price them at 534 s and 785 s
  (`examples/zk2/zenoh-modem/README.md`, "All 73").
- **A keep-alive went before most frames** (link.v2 §2.16). Both faces run
  a keep-alive interval equal to the frame's cadence, so each keep-alive
  falls due when the next frame does: none to one a gap.
  - At SF9, 26 of the window's 30 gaps held one: all 15 before a full
    record, and 11 of the 15 before a record of none.
  - At SF12, 6 of 8: the four before a record of none, and two of the four
    before a full record.
  - gen's frames left 120.003 s and 900.015 s apart, 3 ms and 15 ms more
    than the interval. A frame that is later on its schedule than the one
    before lets the keep-alive go first, which is §2.16's race.
  - The rule gives the range and not the share. The relay sees an SDU after
    its pacing, so it shows which went first in a gap, not by how little.
    One keep-alive more or fewer in SF12's window moves its day by 12
    keep-alives and 17.8 s.
- **SF9 fits its duty.** The uplink took 86 %, and at the most keep-alives
  it takes 762 s, 88 %. With the corrected frames it is 502 s to 621 s,
  58 % to 72 %. Today's 4 link readings took 289 % (`sf9-lean-2`).
- **SF12 takes the whole duty.** The uplink took 864.5 s of the 864 s, and
  36.02 s in every hour of the window against EU868's 36 s.
  - gen's frames leave room for 71 keep-alives a day, and the run sent 72.
  - With the corrected frames, 762 s, the room is 68. At the run's 72 the
    uplink takes 869 s, and at the most, 96, it takes 904 s.
  - The mapping's notes price this face by §2.16 at 785 s to 927 s, none
    to 96 keep-alives beside 785 s of frames (`link.md`, since zk2py round
    110's answers). The run's 72 put it at 892 s on that scale.
  - So at its stated cadence and lease this face does not carry all 73
    readings inside EU868's duty, unless its frames spare most keep-alives,
    and here they spared one in four. An interval above the cadence by more
    than the frames' lateness spares them all (§2.16): the frames alone are
    88 %. That is zenoh-modem's lease or cadence to move.
- **The 100 % is no ceiling.** Nothing in this path enforces a duty cycle.
  - The drivers' readings hold no `air/duty_cycle_wait_ms` and no airtime
    counter: on the udp medium `air/rx_errors_total` is their one `air/`
    reading.
  - `sched/queue_depth` never passed 2, a frame's SDUs waiting on the
    pacing. `sched/dropped_total`, `sdu/dropped_total` and
    `sdu/tx_failed_total` stayed 0, and every frame left on its tick.
  - So the figure is 757.7 s of frames and 72 keep-alives of 1.48 s, a
    coincidence of this window. A real SX1262 at its limit would hold
    frames back, which this run does not show.
- **The idle link is the model's exactly:** 720 keep-alives a day each way
  at SF9 and 96 at SF12, 14 % and 16 % of each end's duty. At a keep_alive
  of 2, phase 2's idle link took 109–114 % and 177–201 % with its reopens.
- **No link opened again.** In the tables, "lost +t" is a keep-alive the
  relay dropped t seconds into the window, and "reopened never" says no
  InitSyn crossed in it. The far side opens, so that column is B->A's.
  - SF9 lost three keep-alives in 92 minutes, and SF12 one in three hours,
    never two in a row. At keep_alive 4 a close takes three in a row.
  - The one lost on SF9's uplink was followed by its frame 0.5 s later.
    None changed a byte count.
- **Lost frames were named.** At 5 % loss a full record crosses whole in
  86 % of cases at SF9, three SDUs, and in 74 % at SF12, six tiles.
  - SF9 lost 2 of its 15 full records, one SDU of each.
  - SF12 lost 2 of its 4, one tile of each. One failed the fragment
    layer's check, and driver B raised a device fault on its own management
    plane and cleared it a second later. The other lacked zenoh's second
    fragment.
  - The far reader read each next frame as `follows: gap, lost: 1`. Every
    record of none crossed.
- **What moved since the SF7 run.**
  - **Timestamping is the generator's** (link.v2 0.13 §2.18). Both routers
    run the `timestamping` block `zenctl acl gen` wrote, which `render.py
    --far` checks, where the RF and SF7 runs set router A's by hand
    (finding 3).
  - **The far end's block names its user** (link.v2 0.15 §2.5, §2.18).
    Router B opens the lane as the user `--far-config` wrote, `ground`,
    with the test's password and dictionary filled in (finding 4).
  - **The faces list their events** (link.v2 0.12 §2.7). SF9's two `rare`
    events crossed once each as gen started, before the window, and
    `face/device_fault` is denied by name. SF12's face lists none: its plan
    denies every event, and its reader's scope is `frames/slow` alone.
  - **Each plan warns `keep_alive_not_suppressed`** (link.v2 0.17), which
    these runs measure.
  - **gen ticks at the face's `every_s`** (`PERIOD_S`), where it ticked
    every 60 s.
- **W2 on LoRa SF7 was not run.** `zk2c-w2-lean-1`'s steady window, each of
  its 245 uplink SDUs charged SF7's time on air, is 1,273 s a day A->B,
  147 % of the 864 s, before any act or reconnect. W2 as RF carries it does
  not fit SF7's duty, and nothing here would have held it back.

### W2 with owner heartbeats (#1110)

Run 2026-11-21, from zenkey at 1546edb3 (#1110's H3: core 0.109,
freshness.v2 0.10, link.v2 0.21), with the same drivers, fork and path. W2
zk2c again, as `zk2c-w2-bare-1` and `zk2c-w2-lean-1` ran it: the same medium
(`rf-zk2c`), stacks, windows and loss, no face. One thing differs. The device
confirms its three interfaces by **one owner heartbeat** per class of
exposure (freshness.v2 §2.4, core §7.5), where it sent a heartbeat per
interface: `owner --catalog FILE --owner-heartbeat`, an option that is off
unless given.

- **The catalog** lists the device's three revisions that have a
  collection, health.v1, alarms.v1 and rfcfg.v1 (`zk2 catalog new`, catalog
  72b9cc38, epoch 0: `results/zk2c-w2-lean-2/catalog.json`). A group names
  its interface by its index there.
- **Every far reader holds it** (`zenctl echo --across-face … --catalog`).
  - With it, a reader's scope also holds the class key of each collection
    in hand. health.v1's, alarms.v1's and rfcfg.v1's readers each add
    `…/freshness.v2/stream/heartbeats/host` to the scope they had. worker.v1
    has no collection, and the frame and event readers name their keys, so
    those three add nothing.
  - Without it a reader declares no class key: its scope is what
    `zk2c-w2-lean-1`'s readers had, each interface's resources and its
    `…/heartbeat/<iface>` key. Then nothing asks router A for an owner
    heartbeat, and none crosses.
  - Checked on the path, by hand (`zk2c-w2-owner-nocatalog-check`, bare,
    3 minutes at no loss, "Reproduce"): with health.v1's reader alone and
    no catalog, the status put crossed and no put on a freshness.v2 key
    did in 105 s, three periods and more. A second reader then started
    with the catalog: router B declared the class key 0.3 s later, and
    the next three owner heartbeats crossed. The first reader heard none
    of them.
  - Router B declares the class key toward router A once for the three
    readers: a 65-B key expression and a 6-B subscriber, at each link open
    (`zk2c-w2-bare-2`, decoded).
- **W2's collections are all `host`,** so the owner heartbeat goes on one
  key, `…/heartbeats/host`, every 28.5 s, with three groups. No put went on
  a `…/heartbeat/<iface>` key in any run.
- **gen is unchanged.** gendev.v1 has no collection, and its frames are
  their own evidence.

| Run | Stack, runs | Windows |
|---|---|---|
| `zk2c-w2-lean-2` | lean (`w2.zdict`), 16-bit | steady 2,100 s, the acts, a 245-s cut, two config sets |
| `zk2c-w2-bare-2` | no layers, 32-bit | the same, beside it on a second path |
| `zk2c-w2-owner-honesty-1` | no layers, 32-bit | steady 1,200 s, the acts, a 245-s cut; the host's `watch` and `health` |

#### The heartbeat element, A->B, a day

| | Puts in the window (lost on the air) | Payload | Put | SDU | Alone in its SDU | SDUs a day | Bytes a day | Air a day |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `zk2c-w2-lean-1`: three heartbeats, lean, 16-bit runs | 153 | 16, 16, 17 B | 20, 20, 21 B | 39, 40 B; 58, 59 B for two | 107 of 153 | 5,349 | 226.8 KB | 756.1 s |
| **`zk2c-w2-lean-2`: one owner heartbeat, lean, a 16-bit run** | 74 (6) | 35 B | sealed | 58 B | 74 of 74 | **3,045** | **176.6 KB** | **588.6 s** |
| the layout (core §7.5): a put every 28.5 s, alone | 73.7 | 35 B | 39 B | 58 B | | 3,032 | 175.8 KB | 586.1 s |
| `zk2c-w2-bare-1`: three heartbeats, no layers, 32-bit runs | | 18, 18, 19 B | | | | 5,225 | 154.4 KB | 514.6 s |
| the same, lean derived (+16 B an SDU) | | | | | | 5,225 | 238.0 KB | 793.2 s |
| `zk2c-w2-bare-2`: one owner heartbeat, no layers, a 32-bit run | 74 (5) | 37 B | 41 B | 44 B | 74 of 74 | 3,045 | 134.0 KB | 446.5 s |
| the same, lean derived (+16 B an SDU) | | | | | | 3,045 | 182.7 KB | 608.9 s |

- **One owner heartbeat cost 176.6 KB a day where three heartbeats cost
  226.8 KB:** 50.2 KB and 22 % less, in 43 % fewer SDUs and 167 s less air.
  On the basis the 238 KB was stated on, the bare run plus 16 B an SDU with
  32-bit runs, it is 182.7 KB.
- **The layout's prediction held:** 175.8 KB a day, and the run is 0.4 %
  over it. The window held 74 puts where the day's rate gives 73.7.
  `model.py` and Appendix A §10 assert the element within one put of the
  window, 1.4 %, and fit no constant to the run.
- **The sizes are the layout's, byte for byte.**
  - The body is 226 bits: the time (42), the count of groups (4), and per
    group its index (10), its tag (4), its size (8) and its sections. The
    far reader decoded sections of 34, 46 and 34 bits: health.v1's status
    and rfcfg.v1's record, each a map bit, a digest and a vouched bit, and
    alarms.v1's three firing alarms as a bitmap.
  - That is 29 B, and a 35-B payload with the envelope's 6 B on a 16-bit
    run, 37 B with a 32-bit one. Every put the far side received was that
    size, 68 of 68 lean and 69 of 69 bare.
  - Decoded (`-bare-2`): a 41-B put in a 44-B SDU, 74 of 74. Under the lean
    stack (`-lean-2`): a 58-B SDU, 74 of 74.
- **An SDU is 23 B over its payload, not 19.** The put adds 4 B (the push's
  header, the scope its far reader declared, the put's header and the
  payload's length), the frame 3 B, the lean stack 16 B. link.v2's Appendix
  B names the 16 and the 3 and leaves the put's 4 out, though its sizes
  include them (zk2py's F-565). A put is 5 B over a payload of 128 B or
  more, whose length takes two bytes: W1-73's 205-B frame was.
- **No put shared an SDU.** The owner sends one put a period, and nothing
  else left within its SDU's air time. In `zk2c-w2-lean-1` health.v1's and
  rfcfg.v1's heartbeats left together, and 46 of the 153 puts went two to
  an SDU.
- **The lean run's SDUs are sealed, so its element is placed, not decoded.**
  `analyze.py` puts each heartbeat the far side received in the SDU the
  relay passed just before it, and each one lost in the SDU dropped at its
  slot of the cadence: all 6 were placed. The same placing on `-bare-2`
  gives its decode exactly, 134.0 KB. `zk2c-w2-lean-1`'s capture was not
  kept, so its element is read from its SDU sizes: one heartbeat in 39 B or
  40 B, two in 58 B or 59 B.

#### W2's day

The uplink's steady window, a day, by the counters:

| | SDUs | Bytes | Air | Of RF |
|---|---:|---:|---:|---:|
| `zk2c-w2-lean-1`, three heartbeats | 10,080 | 688.9 KB | 2,296 s | 2.66 % |
| `zk2c-w2-lean-2`, one owner heartbeat | 7,776 | 638.7 KB | 2,129 s | 2.46 % |

The two windows differ by their heartbeat SDUs and one byte: 245 SDUs and
16,744 B against 189 and 15,524 B. Every other SDU has a twin in the other
run: 7 of 31 B, 35 of 77 B, 70 of 114 B, one of 70 B, one of 106 B, and one
of 163 B that is 164 B in the second.

Both directions, a day as W2 makes it (the steady window's counters, gen's
events cut to W2's 15, the acts at their daily counts and 4 reconnects from
each run's bare twin):

| | Steady A->B | Steady B->A | Events cut to 15 | Acts | 4 reconnects | **Total** | Of RF | Against today's 16.49 MB |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `zk2c-w2-lean-1` | 688.9 KB | 71.5 KB | −106.6 KB | 2.8 KB | 12.2 KB | **668.8 KB** | 2.58 % | 25 times less |
| `zk2c-w2-lean-2` | 638.7 KB | 63.7 KB | −103.4 KB | 2.2 KB | 8.9 KB | **610.1 KB** | 2.35 % | 27 times less |
| the model, corrected (Appendix A §10) | | | | | | 674 KB, 612 KB | 2.60 %, 2.36 % | 27 and 30 times less than its 18.37 MB |

- **W2 measured 610 KB a day with one owner heartbeat, 669 KB with
  three:** 59 KB and 8.8 % less. The model with the layout's owner
  heartbeat says 612 KB, and Appendix A asserts the run within 1 % of it.
- **Of the 59 KB, 50 KB is the heartbeat's.** The rest is not the
  option's:
  - one desired reply fewer crossed B->A in the window, 7.8 KB a day (a
    request was lost on the air);
  - the reopen stalled sooner, 3.3 KB (below);
  - no config request crossed, 0.6 KB;
  - and one more of gen's events was lost on the air and still counted,
    3.2 KB the other way. The events are cut by what the far side
    received, 34 and 33, where each window sent 35.
- **The heartbeat is still the second element of W2,** 29 % of the day
  after the frames' 54 %. Its horizon is its bill: health.v1's and
  config.v2's 60 s put a 58-B SDU on the air every 28.5 s.
- **Compression still does nothing.** The lean run's compress layer stored
  187 of 189 uplink SDUs and saved 68 B in 35 minutes.
- **The reopen stalled again, in all three runs.** After the 245-s cut the
  link opened 9.0 s after the restore (3.4 s in the honesty run), and
  declarations crossed both ways for 5 s. Then nothing crossed either way
  to the end of the run, 230 s later: no frame, no heartbeat, no
  keep-alive. Both `zenctl config set` exited 2 with "Unable to connect to
  any of [tcp/127.0.0.1:…]": router B accepted no new session. In
  `zk2c-w2-bare-1` the sets opened their session and drew silence. This is
  finding 7 again, and its cause is not established here (#1109). The
  reconnect is costed from the reopen alone: 1,046 B in 9.4 SDUs A->B and
  908 B in 8 SDUs B->A, bare, where `-bare-1` crossed 1,214 B and 1,443 B.
  The config element is not measured.

#### What the far readers read

`zk2c-w2-owner-honesty-1` is W2 with owner heartbeats once more, bare, with
a 1,200-s window, and two consumers of health.v1's status on router B from
the device's ready line: `zenctl watch … health.v1 state/status
--across-face`, one with the catalog and one with none. `zenctl health
--across-face crosses` reads the status 60 s before the cut and inside it
([`results/zk2c-w2-owner-honesty.json`](results/zk2c-w2-owner-honesty.json),
by `honesty.py`). The costing runs have no watch: one re-reads its
collection by a GET of its own when it diverges.

The device put 61 owner heartbeats. The relay dropped 4 at the medium's 5 %
(seqs 18, 20, 42, 48) and 7 inside the cut.

1. **Loss is named.** The watch with the catalog printed a `confirmation`
   row for each owner heartbeat from seq 10 to seq 53 that crossed, 40
   rows: group 0, health.v1's index in the catalog, with
   `state/status` confirmed. The four rows after a dropped put, seqs 19,
   21, 43 and 49, say `lost: true`, and no other row does.
2. **No divergence arose, so none was read.** Both status changes of the
   acts crossed before the next heartbeat, and no row says `diverged`.
   This run does not show a divergence read under the owner form.
3. **Fresh while heartbeats cross.** 60 s before the cut, `zenctl health
   --across-face crosses --catalog` exited 0: `healthy`, "its status is OK
   and confirmed, and no check is worse", both readings.
4. **Never fresh in the cut, and never "stale" either.** The last owner
   heartbeat crossed 28 s before the cut. 100 s into it, past the 60-s
   horizon, the three probes each exited 2:
   - `zenctl health`: `unobservable`, `no_descriptor`;
   - the doctor's `health-stale`: `unobservable`, no descriptor read;
   - `zenctl health --across-face crosses`: `unobservable`, `silent`.

   The watch printed nothing from the cut to the end. So no tool read the
   status as current, and none said it was stale: a GET cannot cross a
   cut, and `watch` has no row for a horizon that passed (finding 5). The
   costing runs' two probes said the same.
5. **A reader with no catalog is never confirmed.** The second watch got
   the same three samples, the status put at the start and the two
   changes, and no `confirmation` row in 34 minutes, while 50 owner
   heartbeats reached its router. Its member is never fresh by a
   heartbeat, which is what the option's help says.
6. **The reader with the catalog confirmed nothing for its first 4.8
   minutes.** Ten owner heartbeats crossed, seqs 0 to 9, before its first
   row, 290 s after the device was ready.
   - An owner heartbeat is read through the catalog its owner's
     descriptor names (core §8.6), so the watch's session asked for it: a
     GET on `…/@zk/instance/*`, 51 B.
   - The reply is the device's descriptor and gen's, 1,649 B in 10 SDUs,
     and took 5.6 to 6.1 s to cross at 2,400 bit/s. The read waits 5 s.
   - Five reads in a row timed out ("Timeout(5s)" in `watch-catalog.err`),
     each retried on the second heartbeat after it, 47 to 57 s later. In
     each, gen's descriptor crossed first and the device's ended 5.6 s
     after the request. In the sixth the device's crossed first, 4.8 s
     after it, and the first row followed. `--timeout 30` on the watch did
     not move the 5 s.
   - So five replies, 8.2 KB, crossed A->B for nothing, and a member its
     owner vouched for every 28.5 s read unconfirmed for ten periods. This
     is the reference's reader on this link, not the layout: finding 12.

### W2 with desired.v2 and alarms.v2 (#986's J5, #984's S4)

Run 2026-11-27, from zenkey at 0cd0f6ba (`git describe` 0.14.0-2438-g0cd0f6ba:
main at a438f8b2 with `zenctl desired --across-face`), release builds,
zenoh 1.10.1, with the same fork, drivers and path. W2 zk2c again, as
`zk2c-w2-bare-1` and `zk2c-w2-lean-1` ran it (`rf-zk2c`, no face, the same
windows and loss), with the device's desired target and its alarms on their
v2 profiles (`measure-w2.sh`'s `V2=1`), every owner on a 16-bit run:

- **The device** is `owner --health --alarms-v2 --config-v2 --desired-target
  fleet/desired --desired-v2 --desired-watch --desired-reread-ms 300000
  --desired-timeout-ms 15000 --run-file … h-3fa9c2d41b7e/rf0
  contracts/w2rules.v1.toml`.
  - Its alarms are written by
    [`contracts/w2rules.v1.toml`](contracts/w2rules.v1.toml)'s table
    (alarms.v2 §4): W2's four rules, `hot` with its reading as its argument,
    none with a label, so each record is the profile's usual 9 B.
  - Each read of its slice waits 15 s, rf0's `reply_timeout_ms`, where the
    1-s default lost 4 of 12 GETs in `zk2c-w2-bare-1` (desired.v2 §2.5).
- **The author** is `owner --desired-author --desired-v2 --run-file …`, on
  router B. In `zk2c-w2-v2-watch-bare-1` both take `--desired-ttl 600`,
  "tasking.v2, watched": the author sends a heartbeat toward the target, and
  the target reads when its section moves, in place of its period (§2.5).
- **The far readers** read alarms.v2's standard contract and worker.v2, each
  resource by its exact key, as before.
- **The acts** are W2's two health changes, then `door_open` raised, `hot`
  refreshed (a new reading at one severity), `door_open` resolved, a change of
  the device's document, and the host's probes across the link, each a
  `zenctl` session on router B with the deployment's word for presence:
  - `zenctl desired h-3fa9c2d41b7e/rf0 --across-face --role task --author
    fleet/desired --contracts worker.v2.toml --contracts tasking.v2.toml`;
  - a late joiner, `zenctl alarms 'zk2/h-3fa9c2d41b7e/rf0/alarms.v2/state/alarms/*'
    --across-face granted --clocks-synced --contracts alarms.v2.toml --contracts
    w2rules.v1.toml`, with three alarms firing, then again once the three are
    resolved.

  Then the cut and the two config sets, as before.

Every figure below is the bare runs' decode, message by message, on 16-bit
runs, with the lean stack's 16 B an SDU added for "lean", the basis the designs
priced (#986's memo §1.3 and §5, #984's §1.2 and §4, computed through Appendix
A). Appendix A §10 asserts each.

#### desired.v2

| desired.v2 | Measured, bare | Lean | The design |
|---|---|---:|---:|
| a conditional re-read's request, A->B | a 21-B request (the held stamp, 6 B, in its payload), in a 24-B SDU | 40 B | 21 B, 40 B |
| its answer, unchanged, B->A | a 94-B reply, the held stamp alone, and a 4-B final, in one 101-B SDU | 117 B | 94 + 6 B, 119 B |
| a re-read | | **157 B** | 159 B |
| re-reads every 300 s, a day | 7 in the window, 6 answered unchanged; the relay dropped the 7th's answer, and its GET ended at 15 s, no verdict | **45.2 KB** | 45.8 KB |
| the watch: the author's heartbeat, B->A, a 600-s horizon | a 17-B payload, a 21-B put, a 24-B SDU, every 285 s: 7 in the window | **11.5 KB** a day, 12.1 KB at the rate | 17 B, 40 B, 12.1 KB |
| the watch's reads | none while the section stayed put; one at the first heartbeat after the target's start, which expects none then; none after the change of document | 157 B a read | at most one a heartbeat |
| the report, converged | an 18-B payload (a 12-B record), a 24-B put | 43 B alone in its SDU | 18 B, 43 B |
| the report at the baseline | a 9-B payload (a 3-B record) | | 9 B |
| a change of document | the author's put, B->A, a 59-B SDU; the target's report and its `running`, A->B, 27 B and 54 B bare | 75 B down; 43 B and 70 B up | the report's 43 B |

- **The design held, to 2 bytes a re-read.** Of the unchanged reply's 94 B,
  61 are its key and 20 its replier's id, as the design said. Its final is
  4 B and rides the reply's SDU, where the design took the 6-B final of
  desired.v1's re-read in `zk2c-w2-bare-1`. So a re-read is 157 B, not 159,
  and the period 45.2 KB a day, 1.3 % under the design and 29 % under
  desired.v1's GET, 63.9 KB by Appendix A's measured anatomy.
- **The watch is its author's heartbeat.** The window held 7 of the 7.4 a
  285-s period gives, within one put, as for #1110's owner heartbeat. The
  target read once, at its first heartbeat: it starts expecting no section
  (§2.5's rule, step 6), and the read was answered unchanged. From then on no
  heartbeat moved its section, and nothing was read.
- **A change of document costs the watch nothing.** The target admitted the
  author's put heard live and moved the digest it expects itself, the MAY of
  §2.5: the next heartbeat's section was the one it expected, and no read
  followed.
- **Not measured: a lost heartbeat.** None of the 7 was lost. Each lost one
  leaves the next out of sequence, so the target reads once (§2.5, step 3): at
  RF's 5 %, 15 of 303 heartbeats a day, 2.4 KB more, 14.5 KB with the 12.1.
  The design priced no loss.
- **The wait held.** Every read waited 15 s. Each answer that crossed did
  within the second, and none crossed after its GET had ended: the one GET
  that ended at its timeout had lost its answer on the air, and the next
  period's read was answered (core O5).

#### alarms.v2

| alarms.v2 | Measured, bare | Lean | The design |
|---|---|---:|---:|
| a raise, a refresh, a resolve | a 15-B payload (a 9-B record), a 24-B put, a 29-B SDU on the 5-byte frame of `interactive_high`, 9 of 9 decoded alike | **45 B** | 15 B, 24 B, 45 B |
| W2's 5 raised and 5 resolved, a day | | **0.45 KB** | 0.45 KB |
| the alarms' heartbeat, 3 firing | a 17-B payload in a 24-B SDU, every 427.5 s: 5 in the window | 40 B, **8.2 KB** a day, 8.1 KB at the rate | 17 B, 40 B, 8.1 KB |
| a late joiner's GET, 3 firing and the transient's resolved record kept | a 20-B request; the alarms' snapshot, a 160-B reply in a 163-B SDU; the words' snapshot, holding none, an 89-B reply and the 4-B final in a 96-B SDU | 36 B up, **179 + 112 B** down | 163 B in one SDU, for 3 firing |
| the same, all four resolved | the alarms' snapshot a 178-B reply | 197 + 112 B down | |

- **Every transition is the design's, byte for byte.** The 5-byte frame and
  the 2-byte suffix `/<slot>` it inferred from alarms.v1's totals are what
  the decode shows. A resolve is one put: no delete followed.
- **The alarms' snapshot is the design's too.** 138 B for three firing, 22 B
  for each resolved record kept and 16 B for each firing one: 160 B with the
  transient's kept, 178 B with all four resolved.
- **The design did not price the words' snapshot.** A reader GETs
  `…/alarms.v2/state/**`, as it must to read an alarm by words, and the owner
  answers each collection by a snapshot (core §4.2). The words' collection,
  empty since every alarm is a table's, still answers one: 89 B, most of it its
  key's text and the replier's id, in a second SDU. A late joiner's read is
  36 B up and 291 B down under the lean stack, where the design said 163 B.
  Nothing in the profile's rules moves: §2.15's read stands, and a complete
  snapshot with no entry is the owner's word.
- **Readers.** The first late joiner was told "established", three firing,
  exit 1, its owner known whole from one complete snapshot; the second "no
  alarm at or above the severity fires", exit 0 (alarms.v2 §2.15). Without
  `--clocks-synced` the firing ones read unobservable, their age untrusted
  (the smoke run's first probe).

#### What the host's tools read across the link

- **`zenctl desired --across-face`** read the target converged on the
  changed document's version, `0000/1`, exit 0, in both runs, with no
  presence read: a 16-B SDU up, the report's snapshot in a 117-B SDU down and
  its final in a 7-B one, 188 B lean. The author's slice was read on router
  B, and crossed nothing.
- **`zenctl alarms --across-face granted`**: above.

#### W2's day

| Run | Steady A->B | Steady B->A | W2's day, lean | Against `zk2c-w2-lean-1` |
|---|---:|---:|---:|---:|
| `zk2c-w2-v2-bare-1`, derived (+16 B an SDU) | 647.2 KB | 54.7 KB | **606.3 KB** | |
| `zk2c-w2-v2-watch-bare-1`, derived | 638.1 KB | 37.3 KB | **579.2 KB** | |
| `zk2c-w2-v2-lean-1`, the counters | 672.7 KB | 56.3 KB | **633.3 KB**, 2.44 % of RF | 35.5 KB less than its 668.8 KB |

The day is W2's as before: the steady window's counters or decode, gen's
events cut to W2's 15, the acts at their daily counts (W2 counts no refresh,
no change of document and no probe), and 4 reconnects.

- **W2 measured 633.3 KB a day under the lean stack with desired.v2's
  conditional read and alarms.v2**, 2.44 % of RF, 35.5 KB and 5.3 % less
  than `zk2c-w2-lean-1`'s 668.8 KB. The uplink's steady window fell 16.2 KB
  (688.9 to 672.7), its downlink's 15.2 KB (71.5 to 56.3): the author's
  whole replies became unchanged ones, and no reply crossed for a GET that
  had timed out. Compression stored 210 of the uplink's 236 SDUs in 35
  minutes and saved nothing.
- **The designs said 19.5 KB less** (#986's §5: 18 KB for the conditional
  read; #984's §4: 1.5 KB for alarms.v2). They priced today's re-read by
  Appendix A's model, 63.9 KB a day. `zk2c-w2-lean-1`'s cost more: the
  start's GETs, and the replies of the 4 GETs that timed out at 1 s, which
  crossed for nothing (W2 zk2c, above). Every read here waited 15 s, and no
  answer crossed after its read had ended, as #986's §5 said.
- **The watch would take 27 KB more off the day**: the two bare runs,
  derived, differ by 27.1 KB, the period's 45.2 KB of re-reads against the
  watch's 11.5 KB of heartbeats and its one read after the start. It was not
  run under the lean stack.
- **The heartbeats are the day's second element still**, 222.0 KB placed
  under the lean stack, where `zk2c-w2-lean-1`'s were 226.8 KB: health.v1's
  and rfcfg.v1's pair into SDUs differently from run to run, and alarms.v2's
  is alarms.v1's size. Owner heartbeats (#1110) were not turned on here.

### The honesty checks, across the face

Each through the tools a ground operator would run across rf0's face (`zenctl
echo`, `watch` and `telemetry` with `--across-face --face-policy … --face
rf`, and `doctor`), on W1-73's path ([`honesty.sh`](honesty.sh); what each
tool said is [`results/zk2c-honesty.json`](results/zk2c-honesty.json), read
by [`honesty.py`](honesty.py)).

1. **Loss is detected.** At 75 % loss for 240 s, the far frame reader and
   `watch` both read seq 6 as `follows: gap, lost: 2`, then seq 9 as `gap,
   lost: 2`; after the cut, seq 22 as `gap, lost: 7`. Every lost frame was
   named, by both.
2. **No false "nothing fires", on a frame series.** `zenctl telemetry` over a
   600-s window holding the loss read each counter's pair across the gap as
   one rate over 240 s (the counters are absolute and the run is one, so the
   rate holds), and each record of no reading as `no_sample: absent`, never
   0. Inside the cut it said `unobservable`: "no sample … arrived in 60.0s:
   silence is never a verdict".
3. **Stale is never fresh, but nothing says "unconfirmed".** The link was cut
   for 392 s, past frames/fast's 300-s horizon and the 240-s lease. No tool
   read the readings as current: `telemetry` was `unobservable`, the doctor
   exited 2 (no zk2 token is visible across a face, so it judged nothing), and
   `watch` printed nothing. None said the horizon had passed: `watch` reports
   no lost heartbeat and the doctor has no `state-unconfirmed` yet (#990's V5
   and V6). W2's `zenctl health` and the doctor's `health-stale`, run 100 s
   into a 245-s cut, with the device's tokens still held and health.v1's
   heartbeat silent past its 60-s horizon, both said `unobservable`: the
   descriptor's GET could not cross, so whether it implements health.v1
   could not be told.
4. **A reset is detected.** gen stopped and a new gen started (a new instance
   and run). The reader's next frame `follows: run`, a run it did not hold;
   `zenctl telemetry` over a window holding both
   runs read the counter pair across them `continuity: reset`, `no_rate:
   reset` (40, then 0), and the pairs after it a rate again
   (`zk2c-honesty-reset-1`).

The link came back after the 392-s cut: the lease expired on both ends, and
router B opened a new transport 3.9 s after the restore. Its reopen cost
544 B B->A (the handshake with usrpwd, then the reader's subscriptions) and
108 B A->B. Nothing re-declared by hand was needed.

### Findings (zk2c)

1. **The headline holds in order and misses in factor.** zk2c takes W2 from
   64 % of RF's channel to 2.6 %, and the idle link from 34,560 keep-alives a
   day each way to 1,440. W2 measured 669 KB a day, 25 times less than
   today's wire derived the same way, where the report's first model said
   443 KB and 41 times (W2's findings above). W1-73 fits RF in 1.5 % with gen's frames and
   LoRa SF7's duty in 55 %. At each face's own cadence it took 86 % of SF9's
   duty and the whole of SF12's (finding 9).
2. **Appendix A is corrected where the model missed** (§10): W1-73's frames
   at zenoh-modem's ranges (168 KB a day, not 200 KB, and at LoRa 288 s at
   SF7, 916 s at SF9, 762 s at SF12's hourly keyframe), W2's heartbeats per
   interface, its desired GET unconditional and a reconnect's subscriptions
   (674 KB, not 443 KB). Each correction asserts the run it rests on; the
   measured anatomy (frame, envelope, put, SDU) is asserted byte for byte.
   The report's text quotes the corrected and measured figures. Since the
   SF9 and SF12 runs, a LoRa uplink's keep-alives beside its frames are a
   range, fewest to most, by link.v2 §2.16, where the model had one a gap
   at SF7 and none at SF9 and SF12.
3. **`zenctl acl gen` does not write a face router's timestamping.** A
   router stamps every unstamped put it routes unless told not to (core
   §4.1), and core §8.2 says a deployment SHOULD turn it off on a router that
   forwards onto a constrained face. The generated block (link.v2 §2.18) has
   no `timestamping` key, so a router run from it alone stamps every frame
   (seen in a loopback check: every put through a default router carried
   the router's 26-B HLC). These runs set it by hand. Since link.v2 0.13
   (#1106) the generator writes it, and the SF9 and SF12 runs took its
   blocks as written.
4. **A face's subjects by user name need the far side to open the lane.**
   zenoh's usrpwd authenticates an opener to its acceptor only, so router A
   binds router B to `ground` only when router B opens; with phase 1's
   direction the handshake was refused ("Recv InitSyn. Expected extension").
   And credentials configured on router B make it ask them of every opener,
   so the ground's own tools present one too (`local`). Neither is in the
   face policy, link.v2 or the generated far block. Since link.v2 0.15
   (#1107) §2.5 says which end opens, and the far block names its user.
5. **Across a face no tool says "unconfirmed" yet.** The honesty checks
   hold (none read stale readings as current, every loss and reset was
   named), but `watch` and the doctor only fall silent or unobservable past a
   horizon (#990's V5 and V6 are not on this branch).
6. **At LoRa SF7, frames a minute apart spared one uplink keep-alive in
   four**, where link.v2 §2.16 had them spare every other one; on RF at a
   60-s interval they spared all bare. zenoh counts each interval from the
   last batch sent, so where the cadence is a whole multiple of the
   interval the last keep-alive races the next frame (#1108, link.v2
   0.17). At SF9 and SF12, whose interval is the cadence, a keep-alive went
   before 26 frames of 30 and 6 of 8.
7. **After W2's reconnect, A->B carried nothing but the reopen's
   declarations** until the lease closed the transport, as in phase 2
   (#1109 reproduced it at 0 % loss); the rf0-face honesty run's link came
   back after its cut.
8. **Not measured:** W2's config changes (they drew silence, and with
   owner heartbeats router B took no session), the `rare` modem.v3 events
   (none in a window), W2 across a face, W2 on LoRa (its RF traffic re-aired
   at SF7 is 147 % of the duty), SBD, and any face with zenoh-modem's own
   owner: its keyframes and deltas are computed. Under the owner form: an
   owner heartbeat across a face (W2's collections are `host`, so none
   would cross one), a divergence, and a "stale" from any tool.
9. **lora0's SF12 face takes the whole duty with all 73 readings.** A frame
   every 900 s at a 900-s keep-alive interval sent 72 keep-alives a day
   beside the frames: 864.5 s of the 864 s with gen's frames, and 869 s
   with the corrected ones, whose room is 68. Nothing here enforces the
   duty, so no frame waited. The SF9 face fits, in 86 % as measured and
   88 % at the most. The idle link is exact at both, 14 % and 16 % each
   way, and no link opened again.
10. **A face policy states the bare driver's batch, and the lean stack
    leaves zenoh less.** This is the policy against the driver, not the
    reference's plan against its rule.
    - **The policy.** lora0's SF12 policy states `batch_bytes = 224`: the
      fragment layer's 222-B SDU and zenoh's 2-B length. `acl gen` wrote
      `batch_size: 224` into both blocks (`configs/face-router.json5`,
      `face-far.json5`).
    - **The driver.** Under the lean stack, compress and aead sit above
      the fragment layer and take 14 B. Driver A logged `layer="fragment"
      tx_max_sdu=222`, `layer="aead" tx_max_sdu=209`, `layer="compress"
      tx_max_sdu=208` and `lanes up … max_batch_size=210`. Both routers ran
      `batch_size: 210` (`configs/zenohd-a.json5`, `driver-a.toml`'s
      `zenoh_batch_size`).
    - **What it cost.** A full record, 205 B in a 213-B frame, fits 224 and
      not 210, so zenoh cut each in two fragments. In SF12's window 18 SDUs
      entered the stack (compress stored 14 and compressed 4): 6
      keep-alives, 4 records of none, and 2 for each of the 4 full records.
      Each full record left as five tiles, then a 31-B tile of its own. One
      of the two lost full records lacked that 31-B tile alone.
    - **The plan is right by its rule.** It warns no `frame_over_batch` at
      SF12. link.v1 §2.8 compares the largest frame's payload, 216 B, with
      `batch_bytes`, 224, and the policy cannot know the driver's layers. A
      policy that stated the 210 its driver logs would draw the warning.
    - **The other faces.** SF7 states 224 and ran 208, RF 222 and 206 under
      the lean stack, and a full record took two SDUs at both. SF9 states
      117 and ran 101, and its plan warns already.
    - **Not measured:** a 216-B frame at a 224-B batch. gen wrote none. By
      the anatomy measured here, a put's 5 B and the frame's 3 B, it would
      be a 224-B message for the 222 B that batch carries: the rule counted
      the payload alone. "The batch's bound" (#1136) measured the bound
      since, and `link.v1` 0.22 counts what crosses.
11. **One owner heartbeat took W2's heartbeats from 227 KB a day to
    177 KB** (#1110), on 16-bit runs under the lean stack: 3,045 SDUs of
    58 B where three heartbeats took 5,349 SDUs. The layout said 176 KB,
    and its 35-B payload, 39-B put and 58-B SDU were measured byte for
    byte. W2's day went from 669 KB to 610 KB, 2.35 % of RF and 27 times
    less than today's wire. The option is off unless an owner is given it
    with a catalog, and its readers need that catalog too.
12. **The reference's reader waits 5 s for the descriptor that names its
    owner's catalog, and on RF that is too short.** A consumer reads an
    owner heartbeat's groups through the catalog the owner's descriptor
    names, so `zenctl watch --across-face --catalog` asked for the
    descriptors of the device's address when the first owner heartbeat
    arrived.
    - **Measured** (`zk2c-w2-owner-honesty-1`): the reply is 1,649 B, the
      device's descriptor and gen's, and takes 5.6 to 6.1 s on the air.
      Five reads of six timed out, 8.2 KB of uplink, and the status read
      unconfirmed for 4.8 minutes while ten owner heartbeats crossed.
    - **What decided it** was which descriptor crossed first. The
      device's alone is 1,156 B, 3.9 s of air.
    - **`--timeout` does not reach it,** and W2 has no face policy whose
      reply timeout would.
    - **Not measured:** the same reader across a face, where `@zk` is
      denied and no descriptor crosses. The runtime's rule there is the
      deployment's word, the catalog it was given. Whether a read that
      draws nothing is told from one that timed out was not run here.
13. **Across a link with no face, each far reader that wants its
    confirmations needs the catalog.** With none it declares no class key
    and hears no owner heartbeat. `zk2c-w2-owner-honesty-1`'s second watch
    got every status put and no confirmation in 34 minutes. An owner that
    turns the option on leaves such a reader's members unconfirmed, which
    is why it is opt-in.
14. **desired.v2 and alarms.v2 cost what their designs computed, but for
    one snapshot** (#986's J5, #984's S4; "W2 with desired.v2 and
    alarms.v2").
    - A conditional re-read is 157 B under the lean stack, 2 B under the
      design: its final is 4 B in the reply's SDU, where the design took
      6 B. At a 300-s period, 45.2 KB a day, where desired.v1's GET was
      63.9 KB; every read waited rf0's 15-s reply timeout, and no answer
      crossed after its read had ended.
    - The watch at a 600-s horizon is its author's heartbeat, 11.5 KB a
      day as the window held it and 12.1 KB at its rate, the design's. The target read once, after its start, and
      neither an unmoved section nor its own change of document drew a
      read. A lost heartbeat would draw one, which the design did not
      price: 2.4 KB a day at 5 %.
    - An alarm's transition is one 45-B SDU, byte for byte the design's,
      its 5-byte frame and its 2-byte slot suffix as inferred. W2's ten are
      0.45 KB a day, where alarms.v1's were 2.0 KB.
    - A late joiner's read draws the words' snapshot beside the alarms',
      though it holds nothing: 112 B in a second SDU that the design did
      not price. The alarms' snapshot itself is the design's to the byte.
    - W2's day under the lean stack fell to 633.3 KB, 2.44 % of RF, 35.5 KB
      under `zk2c-w2-lean-1`, where the designs said 19.5 KB: they priced
      today's re-read by Appendix A's model, and that run's timed-out GETs'
      replies crossed besides.
    - Neither moves a rule: the figures are costs of rules as written, and
      `zenctl desired --across-face` and `zenctl alarms --across-face
      granted` read the device across the link as their texts say.

## The batch's bound (#1136)

Run 2026-11-28 with #1136's harness on main at 206145ca (link.v1 0.21 and
link.v2 0.29), and the binaries of the W2 v2 runs: `zenctl` from 0cd0f6ba,
the fork's zenohd at `zm-1.10.2-r1` and `modem-driver` at d8c62df, on
cores 4–5. Finding 10
left a question: how much longer than its payload is the message zenoh
sends, and where does it start to fragment? [`bound.sh`](bound.sh) asks it
on rf0's face with no layers and no loss, the plan of
`zenohd-rf0.face.toml` on router A and its 222 B batch on
`unixsock-stream`:
- **The far side** is one `zenctl echo` on router B of `frames/fast`'s key
  alone, which it declares. So router A sends each put by that key's id, as
  a device's frame crosses.
- **The near side** is `zenctl replay` as `rf-node` of captures written by
  the script: one put every 8 s, or 4 s, each of a chosen length and QoS
  axes, so that each is alone in its batch. Replay declares one publisher
  per key, with its first row's QoS, so each run of one QoS is a capture
  of its own.
- **The routers' ids** are chosen, so that each direction's initial
  sequence number is known before the run (`render.py --ids`, wire.sh's
  `ZIDS`). zenoh 1.10.1 derives it from the two ids (`compute_sn`), and
  masks it to 14 bits at `16bit` and 28 at `32bit` (`common/seq_num.rs`).
  [`bound.py`](bound.py) computes it the same way and checks it against
  the OpenAck router A sent.
- **`rf-zk2c-32`** is `rf-zk2c` at 32-bit sequence numbers, rf0's policy
  with `sequence = "32bit"`, which is zenoh's default and what a link.v1
  face runs.

`bound.py` decodes every A->B SDU and finds each put by its payload, the
byte of its index repeated: the batch it took, or its fragments.

| Run | Sequence | Router A's first sequence number | Puts |
|---|---|---:|---|
| `bound-16bit-1` | `16bit` | 16,370 | 17: around the bound at each QoS, and four of 100 B to run past 16,383 |
| `bound-32bit-1` | `32bit` | 268,435,425 | 28: around the bound at each QoS, and 19 of 100 B to run past 268,435,455 |

The largest payload that crossed in one batch, by QoS and sequence number;
one byte more fragmented in every row:

| QoS | 16-bit, a number of 2 bytes | 16-bit, a number below 128 | 32-bit, a number of 4 bytes |
|---|---:|---:|---:|
| `data/drop/reliable` (`frames/fast`'s) | 212 B: 10 B beside it | 213 B: 9 | 210 B: 12 |
| `data/block/reliable` | 210 B: 12 | 211 B fit; 212 B not sent | 208 B: 14 |
| `data/drop/reliable+express` | | | 208 B: 14 |
| `data_low/drop/reliable` | 208 B: 14 | | 206 B: 16 |

- **What a batch holds beside the payload**, byte for byte as decoded:
  - the put: the push's header, the scope, the put's header, and the
    payload's length, 2 bytes from 128 B, so 5 bytes; 4 below 128 B;
  - the push's QoS extension, 2 bytes, at `block`, at express or at
    `data_low`;
  - the frame's header, 1 byte, and the sequence number; at `data_low` the
    frame's QoS extension, 2 bytes more;
  - the batch's 2-byte length, which the driver strips: the SDU is 2 bytes
    shorter than the batch.
- **The sequence numbers wrapped at their mask.** At `16bit` they ran from
  16,370 to 16,383 and then 0, so a number never took more than 2 bytes. At
  `32bit` they ran from 268,435,425 to 268,435,455 and then 0, never more
  than 4. A number below 128 took 1 byte: a 213 B put fragmented at 16,372
  and fit at 3.
- **Each fragmented message** spent one sequence number beyond its
  fragments: 16,372 and 16,373, then the next message at 16,375.
- **Every channel starts at the same number.** `data_low`'s first put took
  16,370, as `data`'s had, since each priority has its own sequence.
- **The 2-byte length is a streamed link's.** zenoh 1.10.1 writes it on
  `tcp`, `tls`, `quic`, `unixpipe`, `unixsock-stream`, `vsock`, and `udp`
  where the endpoint asks for a reliable link; not on `serial`, `ws` or the
  fork's `unixsock-dgram` (each link's `is_streamed`). Read in the source,
  not measured.

`link.v1` 0.22 §2.8 takes these as a put's **allowance**, and its
Appendix B holds the table; `link.v2` 0.30 §2.16 reads it at the
transport's `sequence`. The 2026-11-27 reading of #1136 had a 16-bit number
take 3 bytes from 16,384: the mask keeps it below that.

The first attempts (not kept): an unpaced run, its `t` given in seconds
where a capture's is in microseconds, so replay put every row at once and
zenoh dropped the `drop` puts that found the queue full; and a run whose
far reader died when `/srv/dev` filled, so the far side undeclared its
key midway. Their decoded batches agree with the runs kept, as far as they
went.

## Release candidate 0.23.0 (2026-11-29)

Run 2026-11-28, for zenkey 0.23.0's release candidate (#975, #989), on main at 1dbce36f
(`git describe` 0.14.0-2625-g1dbce36f). Since 0.22.0 (2026-11-08) main has #1110's owner
heartbeat, desired.v2's, alarms.v2's and health.v2's readers, zenoh 1.10.2, #1176's hold and
horizons, redundancy.v2, #980's compact descriptor and #1136's batch allowance. This section
runs again what set the baselines above, on zenoh-modem's shape, and soaks it.

- **Binaries.** `zenctl`, the owner example and `zk2` from main at 1dbce36f, release builds, on
  zenoh 1.10.2. The fork's zenohd at `zm-1.10.2-r1` (2454b10c) and `modem-driver` at d8c62df,
  zenoh-modem's head, a debug build with every feature: the binaries the W2 v2 runs used.
- **The harness** is this directory at main, run from a frozen copy, every process on cores
  4–5, up to eight paths at once. The host was short of memory throughout: other lanes' test
  suites held it, and up to 0.9 GB was swapped out.
- **The far side's blocks are the generator's** (`FAR=1`, wire.sh's default since the SF9
  runs), where the RF and SF7 runs of 2026-11-08 were rendered with `FAR=0`. Both routers'
  `timestamping` is `{ router: false, peer: false }`, and router A's plan grants each node's
  `freshness.v2/stream/heartbeats/host` beside its heartbeats (#1110). Neither changes what
  crosses: no owner on the face sends an owner heartbeat, and no put carried a router's stamp.

| Run | Runs again | Path | Windows |
|---|---|---|---|
| `rc-023-w1-lean-1` | `zk2c-w1-lean-1` | RF, rf0's face, lean (`rf.zdict`), 16-bit | idle 600 s, W1-73 1,800 s |
| `rc-023-w2-lean-1` | `zk2c-w2-lean-2` | RF, no face, lean (`w2.zdict`), owner heartbeats, 16-bit | steady 2,100 s, the acts, a 245-s cut, two config sets |
| `rc-023-w2-bare-1` | `zk2c-w2-bare-2` | the same, no layers, 32-bit | the same, 12 minutes after it on a second path |
| `rc-023-sf7-lean-1` | `zk2c-sf7-lean-1` | LoRa SF7, lora0's face, lean | idle 600 s, W1-73 1,800 s |
| `rc-023-sf9-lean-1` | `zk2c-sf9-lean-1` | LoRa SF9, lora0's SF9 face, lean, a frame every 120 s | idle 1,920 s, W1-73 3,600 s |
| `rc-023-honesty-1` | `zk2c-honesty-1`, `-reset-1` | RF, rf0's face, no layers | the three steps in one run; the cut 452 s |
| `rc-023-soak-1` | | W2's path, lean: W2's v2 device and modem.v3's gen at rf0's address, the host's tools | 2 h, 12 reopens |
| `rc-023-soak-face-2` | | W1-73's path, lean: gen as `rf-node`, the frame reader and `telemetry` | 2 h, 12 reopens |

### Each figure against its baseline

A day, as each baseline section states it:

| Figure | Baseline | Main | Change |
|---|---:|---:|---:|
| RF, the idle link, each way | 1,440 keep-alives, 24.5 KB | 1,440, 24.5 KB | 0 |
| **W1-73 across rf0's face, frames/fast, lean** | **207.3 KB** | **210.1 KB** | **+1.4 %** |
| the same, `face/device_fault` | 162.9 KB | 157.4 KB | −3.4 % |
| the same, the uplink's keep-alives | 336, 5.7 KB | 144, 2.4 KB | inside 0 to 1,440 |
| the same, A->B in all | 377.2 KB, 1,257 s (1.46 %) | 372.8 KB, 1,243 s (1.44 %) | −1.2 % |
| **W2 with owner heartbeats, W2's day, lean** | **610.1 KB** (2.35 %) | **640.7 KB** (2.47 %) | **+5.0 %** |
| the same, the owner heartbeat, lean, placed | 3,045 SDUs, 176.6 KB, 588.6 s | 3,045, 176.6 KB, 588.6 s | 0 |
| the same, bare, decoded | 134.0 KB | 134.0 KB | 0 |
| the same, the steady window A->B, lean | 638.7 KB | 641.1 KB | +0.4 % |
| the same, the steady window B->A, lean | 63.7 KB | 81.7 KB | +28 % |
| LoRa SF7, the idle link, each way | 2,880 keep-alives, 148.2 s | 2,880, 148.2 s | 0 |
| **SF7, W1-73's uplink** | **474.2 s (55 %)** | **481.8 s (56 %)** | **+1.6 %** |
| SF7, gen's frames | 2,160 SDUs, 363.0 s | 2,160, 363.3 s | +0.1 % |
| SF7, the uplink's keep-alives | 2,160, 111.1 s | 2,304, 118.6 s | inside 1,440 to 2,880 |
| LoRa SF9, the idle link, each way | 720 keep-alives, 12.2 KB, 118.7 s | the same | 0 |
| **SF9, W1-73's uplink** | **2,064 SDUs, 122.9 KB, 745.8 s (86 %)** | **2,160, 124.6 KB, 761.6 s (88 %)** | **+2.1 %** |
| SF9, gen's frames | 1,440 SDUs, 112.3 KB, 642.9 s | 1,440, 112.3 KB, 642.9 s | 0 |
| SF9, the uplink's keep-alives | 624, 102.9 s | 720, 118.7 s | inside 0 to 720 |

With these runs in place of the baselines, `model.py --zk2c` holds every tolerance: gen's frames'
air within 1 % at SF7 and SF9, each uplink's keep-alives inside link.v2 §2.16's range, the idle
links exact, and the owner heartbeat's layout byte for byte and within 1.4 %. The one assertion
that fails is section 10's record of the 2026-11-08 run itself, its 336 keep-alives.

What moved, and why:

- **W1-73's frames are the baseline's frames.** The far reader got 205-B and 21-B payloads, and
  `face/device_fault` 97 B and 98 B, as before. In the window, 15 full records left in two SDUs,
  and 15 records of none. One record of none left in one 135-B SDU with a `face/device_fault`,
  where both had SDUs of their own before. So the README's split, the data less the events'
  113-B and 114-B SDUs, counts that SDU as a frame: frames/fast +1.4 %, the event −3.4 %, and
  the two together −0.7 %.
- **The uplink's keep-alives fell to 3 of the window's 30 gaps,** from 7. This is the race of
  link.v2 §2.16. At SF7, whose interval is half the cadence, the second keep-alive of a gap went
  before its frame in 18 gaps of 30 (15 before). At SF9 a keep-alive went first in all 30 gaps
  (26 before). SF9's uplink is therefore 2.1 % over its baseline, at the top of its range:
  761.6 s, the model's most. Its frames are byte for byte the baseline's.
- **W2's owner heartbeat is the baseline's, to the byte.** 74 puts in the window, 35-B payloads in
  58-B SDUs lean, 37 B, 41 B and 44 B bare. W2's day rose 30.6 KB, for three reasons:
  - **The target reads its author's descriptor** (81dcbd44, desired.v2 §2.18, which applies to a
    target of either major). It reads `fleet/desired/@zk/instance/*` (a 43-B to 46-B request)
    twice at its start and once at its first re-read, 300 s later, inside the steady window. Each
    answer is a 419-B reply and a 7-B final. The window's one read is a one-off, which W2's day,
    scaling the window, counts 41 times: 19.7 KB bare, 22.4 KB lean (`presence` in the decode).
    §2.18 has the target read again at each appearance of the author's instance token, so a
    reconnect whose link carries data would draw one more.
  - **Two more of gen's events were lost on the air** (31 received where 33 were), and the day
    cuts the events by what the far side received: 6.3 KB, as the baseline notes of `-lean-2`.
  - **The reconnects cost 3.2 KB more, and the sets' requests crossed.** More of the reopen's
    declarations crossed B->A before the stall, the author's presence among them (258 B, where
    `zk2c-w2-bare-2` crossed 46 B), and the sets' requests crossed, 0.6 KB, where in `-bare-2`
    router B took no session. The two sets drew silence again: the reopen stalled, as finding
    7 says.

  Leaving the window's descriptor read out, W2's day is 618 KB, 1.3 % over the baseline, and
  the rest is the events' cut.
- **Every descriptor read of the target timed out.** It waits the target's read timeout, 1 s by
  default, and the 426-B answer takes 1.6 s at 2,400 bit/s. So the target takes the
  deployment's word for its author's revision, as §2.18 says it then does, and three reads,
  1.3 KB down, cross for nothing at each start and its first re-read. With the face's 15-s
  reply timeout, as the V2 runs give it, each read would land (rc-023's finding 5).

### The honesty checks

`rc-023-honesty-1` ran `honesty.sh`'s three steps in one run, where the baseline needed a
second run for the reset ([`results/rc-023-honesty.json`](results/rc-023-honesty.json)). Every
check said what the baseline's said:

1. **Loss is detected.** At 75 % loss, the far reader and `watch` both read seq 6 and seq 9 as
   `gap, lost: 2`. After the cut, which ran 452 s here, they read seq 23 as `gap, lost: 8`.
2. **No false "nothing fires".** `telemetry` over the loss: `not_established`, 102 pairs, the
   counter's pair across the gap one rate over 240 s, as before.
3. **Stale is never fresh.** Inside the cut, `watch` ended its 60-s window with a summary row,
   `received: 0`, `heartbeats: 0`, and exit 2. `telemetry` said `unobservable`, "no sample …
   arrived in 60.0s: silence is never a verdict". The doctor said `unobservable`: no zk2 token
   is visible to it across the face. The baseline's `watch` probe was refused for `--format
   json`; this one ran.
4. **A reset is detected.** The new gen's first frame `follows: run`, and `telemetry` over both
   runs read the counter's pair across them as `reset` (40, then 0), and the next pairs as rates.

The link came back after the 452-s cut, router B opening a new transport.

### The soak

Two soaks, each 2 h 10 min from the end of its setup at the medium's 5 %, the link reopened
every 10 minutes by one restart in turn: driver A, router A, driver B, router B, three of
each ([`soak.py`](soak.py), [`soak-analyze.py`](soak-analyze.py)). A restart stops the process's
`timeout`, which wire.sh started for the run, and starts the same command again. After each
reopen the host's tools ran once from router B, each across the face; every minute each
process's memory was read, its RssAnon and VmSwap together, since the host's memory pressure
shrank every RSS.

**`rc-023-soak-1`, W2's path**
([`results/rc-023-soak-1/analysis.json`](results/rc-023-soak-1/analysis.json)). The owners
are the zenoh-modem mapping's, as these runs have them: W2's v2 device (the owner example:
health.v1, alarms.v2 with three alarms firing, config.v2 on rfcfg.v1, a desired.v2 target)
and modem.v3's `zenctl gen` under zenoh-modem's catalog, both at rf0's address, the author on
router B. The probes are `config set` and `config get` with rf0's face policy,
`health --across-face crosses`, `alarms --across-face granted`, `desired --across-face` and
`telemetry` over frames/fast, at a fixed 90 s after each restart.

- **Before the first reopen every tool answered.** `config set`: the record, exit 0. `config
  get`: exit 0. `telemetry`: 146 samples of 73 members, 51 established pairs, none decreased,
  exit 0. `health`: healthy, its status fresh, exit 0. `alarms`: established, an alarm at
  warning fires, exit 1. `desired`: converged, exit 0.
- **After the first reopen, driver A's restart, the link never carried data again.** For 1 h
  50 min and the 11 reopens after it, of every kind, no far reader got a sample: no frame, no
  heartbeat, no event, nothing from the device.
  - Each transport opened, crossed its declarations both ways, and then A->B carried nothing,
    not even a keep-alive.
  - A lease later, a router closed it with "Unable to push non droppable network message …
    Closing transport!" (18 times on router B, 24 on router A), and the next transport did the
    same, one every 245 s to 260 s.
- **Every probe after it said so, and none said more.** All 72 exited 2. In 55, router B, waiting
  out a stalled transport, took no session ("Unable to connect to any of
  [tcp/127.0.0.1:18002]"). The other 17 drew silence:
  - `config set`: `silent`, "a set that drew nothing started nothing a tool can name";
  - `health`: `silent`;
  - `alarms`: `owner_unobservable`;
  - `desired`: `nothing_crossed`;
  - `telemetry`: no sample in 260 s, "silence is never a verdict".
- **This is zenoh-modem#210's shape, not #1109's.** The link itself never came back: its
  transports opened and stalled at the lane, as #1109 found below zenkey's wire. A `config
  set` drew silence because nothing crossed at all, and no tool can do better than say so.
  - The rf0-face soak below came back from the same restarts.
  - W2's own reconnect in `rc-023-w2-*-1` did as the soak did: a transport at the restore and
    another a lease later, each stalled after its declarations, as in `zk2c-w2-*`.
- **The far side opening does not save W2's device.** The soak smoke `rc-023-soak-smoke-2` ran
  the same owners with the far side opening the lane and no plan (`FAR_OPENS=1`). Its first
  transport stalled 14 s after it opened, at no loss, with 7 SDUs of declarations crossed, and
  no reopen of the four recovered it.

**`rc-023-soak-face-2`, W1-73's path across rf0's plan**
([`results/rc-023-soak-face-2/analysis.json`](results/rc-023-soak-face-2/analysis.json)): gen as
`rf-node`, the far frame reader and `telemetry` as the ground's tools, and nothing of the
device's, which the plan would deny. The probes wait for a frame to cross after each restart,
up to 420 s.

- **The link came back from 11 of the 12 reopens, gen's frames with it,** in this many seconds
  to the first frame:

  | Restart | 1st | 2nd | 3rd |
  |---|---:|---:|---:|
  | driver A | 242 | 282 | 302 |
  | router A | 242 | 457 | 242 |
  | driver B | 62 | 78 | 122 |
  | router B | never, to the next reopen 681 s later | 62 | 62 |

  - A restart on the near side, driver A's or router A's, left router B holding its old
    transport until its 240-s lease ran out; router B then opened anew.
  - A restart on the far side opened a new transport at once, and the next frame crossed.
  - After router A's second restart, its OpenAck was lost on the air, and router B's
    InitSyns, every 12 s to 14 s, drew no answer for 200 s.
  - After router B's first restart, one SDU of router B's declarations was lost, and the far
    reader's re-declaration did not repair it (finding 2).
- **`telemetry` answered all but one round the link was back in:** `not_established`, 146 samples of
  73 members, 51 established pairs, exit 0, 11 times. It exited 2 twice. After the 3rd reopen a
  full record was lost in its window, so no pair formed (`unobservable`). After the 4th no
  sample arrived ("silence is never a verdict").

**Memory** (`rc-023-soak-1`, each process's RssAnon and VmSwap, KB):

| Process | First | Last | Largest | KB an hour, first half, second half | Grows on (`soak-analyze.py`) |
|---|---:|---:|---:|---:|---|
| the device (owner) | 3,564 | 4,176 | 4,180 | 421, 166 | no |
| the author (owner) | 1,708 | 2,184 | 2,184 | 245, 196 | no |
| gen (`zenctl gen`) | 8,436 | 9,392 | 9,392 | 673, 235 | no |
| the frame reader (`zenctl echo`) | 5,164 | 5,776 | 5,776 | 405, 129 | no |
| health.v1's reader | 3,352 | 3,888 | 3,888 | 408, 123 | no |
| alarms.v2's reader | 3,308 | 3,892 | 3,892 | 342, 77 | no |
| rfcfg.v1's reader | 3,352 | 3,884 | 3,884 | 296, 176 | no |
| worker.v2's reader | 3,300 | 3,836 | 3,836 | 276, 130 | no |
| driver A's management-plane reader | 31,852 | 40,788 | 43,356 | 1,286, 2,175 | no |
| driver B's management-plane reader | 32,384 | 40,368 | 43,312 | 2,296, 2,834 | no |
| the relay | 11,692 | 11,700 | 11,708 | 1, 2 | no |
| driver A, the 5th to 9th reopen | 4,660 | 5,156 | 5,156 | 1,480, 0 | no |
| router B, the 4th to 8th reopen | 1,680 | 3,996 | 3,996 | 4,325, 2,904 | yes |

- **No zenkey process grew without levelling off.** The owners and `gen` grew by 0.5 MB to 1 MB
  in two hours, each more slowly in the second half than the first. The far readers grew by
  0.6 MB each, more slowly still. The management-plane readers, each a `zenctl echo` of about
  100 foreign keys a second, reached 38–43 MB within 30 minutes and swung between the two
  after it; their slopes are that swing.
- **The routers and drivers lived 40 minutes each**, restarted in turn, so the soak tells their
  memory over that alone: the drivers levelled at 5 MB. The router B that lived from the 4th
  reopen to the 8th grew from 1.7 MB to 4.0 MB, opening a stalled transport every 250 s.
- **In the face soak** gen grew from 8.6 MB to 9.0 MB and the frame reader from 5.3 MB to
  5.7 MB (their thirds' medians), each more slowly in the second half. Driver B's
  management-plane reader stepped from about 40 MB to 47–49 MB after the driver's second
  restart, and held there for the last 40 minutes.

### Findings (rc-023)

1. **The wire is the baselines' wire.** Every figure is within 2 % of its baseline or explained:
   SF9's uplink 2.1 % over, at the top of link.v2 §2.16's range, its frames byte for byte; W2's
   day 5.0 % over, by desired.v2 §2.18's descriptor read that fell in the window, and the
   events' cut. With these runs, `model.py --zk2c` holds every tolerance. The four honesty
   checks hold.
2. **A zenkey bug: a far reader's re-declaration repairs nothing while a second reader of the
   key shares its router** (core §8.5, `zenkey::redeclare`, #988 L4). The repair declares the
   reader's subscriber again on its own session, and its router declares toward the far router
   only when the first subscriber of a key arrives and undeclares only when the last leaves.
   With a second subscriber there, the router keeps its declaration, which the far router never
   got, and nothing crosses.
   - **`rc-023-redeclare-alone-1`** (bare, decoded, [`redeclare.sh`](redeclare.sh)): R1's four
     DeclareSubscribers were lost in an 8.5-s cut, the key expressions already declared. At
     601.3 s, two of frames/fast's horizons, R1's undeclaration and declaration crossed B->A,
     and its first frame came 4.6 s later.
   - **`rc-023-redeclare-shared-1`**: the same, and R2, the same reader, started 15 s after the
     restore. R2's declaration did not cross: router B held R1's. Neither R1's re-declaration
     at about 600 s nor R2's at about 625 s crossed. Neither reader got a frame in the 735 s
     to the end. Only keep-alives crossed, both ways, and router A sent no frame, since it had
     no subscriber.
   - **In the soak.** After the 4th reopen of `rc-023-soak-face-2`, router B's restart, one
     86-B B->A SDU of declarations was lost. No frame crossed for the 681 s to the next
     reopen. The frame reader's re-declaration fell due at 600 s, while the round's
     `telemetry` held frames/fast at router B, and no B->A declaration crossed then.
   - **What it costs.** A ground with two readers of a key behind one router, a frame reader
     beside zengui, zenwatch or a probe, never repairs that key while the transport stays up.
     Each reader's silence is honest, but it is never ended.
   - Not fixed here (the brief's rule). The evidence is
     `results/rc-023-redeclare-{alone,shared}-1/`: the decoded declarations and what each
     reader heard, and the bare captures. Fixed since by core 0.121's repair subscription
     (#1185), below.
3. **On W2's path no reopen brought the link back, which is zenoh-modem#210's, not zenkey's.**
   After driver A's first restart, W2's device and gen behind router A, no far reader got a
   sample for 1 h 50 min, through 11 more reopens of all four kinds. Every probe exited 2,
   none with a false verdict. The far side opening the lane stalled the first transport too.
4. **Across rf0's plan the link came back from each restart within a lease or so**, the one
   miss the reader's lost declaration (finding 2). Each restart of the near side's processes
   left router B holding its old transport for a lease: 242 s to 302 s to the first frame, where a
   far-side restart took 62 s to 122 s. Once a lost OpenAck held the link 200 s longer. This is
   the lease and zenoh's handshake, not zenkey's.
5. **desired.v2 §2.18's descriptor read times out on RF at a target's default read timeout.**
   W2's target read its author's descriptor three times at its start and its first
   re-read. Each 426-B answer took 1.6 s against the 1-s wait, so every read failed, and the
   target took the deployment's word, as §2.18 says it then does. That is 1.3 KB down and
   132 B up for nothing at each start, and §2.18 reads again at each appearance of the
   author's token. A target given the face's `reply_timeout_ms`, as the V2 runs give it, would
   read it.
6. **An event's subscription is never declared again.** It has no horizon
   (`zenkey::redeclare`: "a subscription to what has none is declared once"). In
   `rc-023-redeclare-alone-1` the repaired reader got frames again and never another
   `face/device_fault`, which gen put once a minute. A face that lists events loses them for
   as long as the transport stays up once their declaration is lost. Fixed since by core
   0.121 (#1185): an event's subscription is declared again and repaired beside the reader's
   others, below.
7. **Memory.** No zenkey process grew without levelling off in two hours: the owners and gen
   by 0.5 MB to 1 MB, slowing; the far readers by 0.6 MB. The management-plane readers held
   38–43 MB, but for driver B's in the face soak, which stepped to 47–49 MB at the driver's
   second restart and held there. Two hours do not show where the owners' slow growth ends.

### The repair (#1185, core 0.121)

Finding 2 was fixed before 0.23.0 shipped. Core 0.121 §8.5 adds a **repair subscription**:
a subscription still silent two horizons after it was declared again subscribes to its key
with `$*` appended to its literal prefix's last chunk (`…/frames/fast$*`), and keeps it. No
reader's own subscription declares that key expression, so router B declares it toward router
A, and router A routes the key's samples to router B, for every reader behind it.

**The check, before the rule** (`rc-023-repair-check-1`, bare, main at 1dbce36f's binaries).
`redeclare.sh`'s shared case, then two more sessions on router B, by hand:
- **Neither reader's re-declaration crossed**, at 600 s and 624 s, and nothing crossed B->A
  from 0.7 s to 796 s: the shared case is not repaired today.
- **At 796 s a zenoh-python session** ([`rawsub.py`](rawsub.py); zenctl refuses `$*` in a
  selector) subscribed to `…/frames/fast$*`. Router B sent a `DeclareKeyExpr` of
  `…/stream/frames` (57 B) and the `DeclareSubscriber` of `/fast$*` (14 B), and nothing else.
  Router A's next frame, at 845.8 s, reached R1, R2 and the session.
- **At 875 s a fourth** subscribed to `…/face/device_fault$*/*`: a `DeclareKeyExpr` of
  `…/events/face` (55 B) and the `DeclareSubscriber` (24 B). The next two events reached R1
  and R2.

**The re-soak** (`rc-023-redeclare-shared-2`, bare, zenctl with the repair, `WATCH_S=1380`).
`redeclare.sh`'s shared case, R1's four `DeclareSubscriber`s lost in the cut as before:

| s after the cut | What crossed, or what each reader did |
|---:|---|
| 0.6–0.7 | B->A: R1's four `DeclareSubscriber`s, lost |
| 600.5, 624 | R1, then R2, declares its four subscriptions again, the events' beside frames/fast's; nothing crosses |
| 1,202.2–1,202.6 | B->A: R1's four repairs, `…/stream/frames` and `…/events/face` as key expressions (52 B and 55 B) and four `DeclareSubscriber`s (14 B to 32 B), 205 B in all |
| 1,205.5, 1,206.2 | A->B: the next event and frame, each reaching R1 and R2 |
| 1,206 to 1,404 | four frames and four events A->B, each reader hearing each once; R2 declares no repair, hearing before its own fell due |

- **Repaired at 1,206 s**, two of frames/fast's horizons after the re-declaration that
  changed nothing. `rc-023-redeclare-shared-1` was never repaired in 735 s.
- **What it costs**: 205 B B->A once, the repairs kept while R1 reads. Nothing else crossed
  either way but keep-alives.
- **Events too** (finding 6): R1's three event subscriptions were declared again and repaired
  beside frames/fast's, and both readers got every `face/device_fault` after.
- Each reader's re-declarations and repairs are in its log (`READER_LOG`), in the run's
  `analysis.json` under `reader_acts`.

## Reproduce

`WORK` defaults to `/srv/dev/targets/zenkey-989`. It holds the binaries and
everything a run writes: rendered configs, sockets, aead state and logs. No
root, no network namespace: TCP on 127.0.0.1 ports `BASE`+1 to `BASE`+14 and
unix sockets under `WORK/run/<INST>`. Every process runs on cores 0–1 under
`timeout`, its PID in `WORK/run/<INST>/pids`, and `INST=<inst> ./wire.sh
down` stops exactly those. Four paths can run at once, each with its own
`INST` and `BASE`. Run each from a frozen copy of this directory, so that
editing the checkout cannot change a script a run is still reading.

```bash
WORK=/srv/dev/targets/zenkey-989
# zenctl and owner (zenkey) and modem-driver (zenoh-modem, built with every
# feature) in $WORK/bin; the fork's zenohd, built outside zenoh-modem's tree:
CARGO_BUILD_JOBS=2 taskset -c 0-1 cargo +1.98 install --locked \
  --git https://git.marcpardo.eu/marcpardo/zenoh --tag zm-1.10.2-r1 \
  --root $WORK/zenohd --target-dir $WORK/zenohd-build zenohd
cd docs/zk2/measurements/wire-baseline

# Phase 1: RF, idle and W1 (about 23 minutes each)
timeout 2h taskset -c 0-1 ./measure.sh rf bare bare-1
./train-dictionary.sh $WORK/logs/bare-1                    # writes rf.zdict
timeout 2h taskset -c 0-1 ./measure.sh rf lean lean-1

# Phase 2: W2 on RF (about 45 minutes); w2.zdict from a bare W2 run
INST=rf BASE=17400 LOSS_START=0 LOSS_AT_STEADY=0.05 \
  timeout 3h taskset -c 0-1 ./measure-w2.sh rf bare w2-rf-bare-3
./train-dictionary.sh $WORK/logs/w2-rf-bare-2 w2.zdict
INST=smoke BASE=17600 DICT=$PWD/w2.zdict LOSS_START=0 LOSS_AT_STEADY=0.05 \
  timeout 3h taskset -c 0-1 ./measure-w2.sh rf lean w2-rf-lean-1

# Phase 2: LoRa, lean (rf.zdict), the setup lossless
export LOSS_START=0 LOSSLESS_SETUP=1
INST=lora BASE=17500 timeout 2h taskset -c 0-1 ./measure.sh sf7 lean sf7-lean-2
INST=lora BASE=17500 SETTLE_S=40 timeout 2h taskset -c 0-1 ./measure.sh sf9 lean sf9-lean-2
INST=lora BASE=17500 W1_S=0 timeout 1h taskset -c 0-1 ./measure.sh sf7ka4 lean sf7ka4-lean-1
INST=lora12 BASE=17700 SETTLE_S=120 IDLE_S=2700 W1_LEAD_S=270 W1_S=1800 \
  timeout 3h taskset -c 0-1 ./measure.sh sf12 lean sf12-lean-2

# Analysis
python3 -I analyze.py $WORK/logs/<run> --json results/<run>/analysis.json
python3 -I analyze.py $WORK/logs/w2-rf-bare-3 --json results/w2-rf-bare-3/analysis.json \
  --link-failed-at-s 1875.9 --steady-end-s 1860         # w2-rf-lean-1: 1878.5
python3 -I setup-loss.py $WORK/logs/{w2-smoke-1,w2-smoke-2,w2-rf-bare-1,w2-rf-bare-2,w2-rf-bare-3} \
  --json results/setup-loss.json
python3 -I model.py > results/model.json                # Appendix A, per direction
python3 -I summary.py                                    # the tables above
```

zk2c (2026-11-08, and 2026-11-21 for SF9 and SF12): the routers at link.v2's
transport (`rf-zk2c`, `sf7-zk2c`, `sf9-zk2c`, `sf12-zk2c`), a face's plan on
router A where `FACE_POLICY` names one, each process on cores 4–5, each run
from a frozen copy whose `REPO` is the checkout (gendev.v1's schemas are
named relative to it, hence `CONTRACTS`):

```bash
R=$PWD/../../../..; F=$R/examples/zk2/zenoh-modem/faces
export WORK CORES=4-5 REPO=$R LOSS_START=0
# Idle and W1-73 across rf0's face (about 55 minutes), bare, then lean with 16-bit runs
INST=rf BASE=17400 LOSSLESS_SETUP=1 W1=73 FACE_POLICY=$F/zenohd-rf0.face.toml IDLE_S=1200 W1_S=1800 \
  timeout 2h taskset -c 4-5 ./measure.sh rf-zk2c bare zk2c-w1-bare-1
INST=rf BASE=17400 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-rf0.face.toml W1_S=1800 \
  timeout 2h taskset -c 4-5 ./measure.sh rf-zk2c lean zk2c-w1-lean-1
# W1-73 across lora0's SF7 face, lean
INST=lora BASE=17600 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-lora0-sf7.face.toml \
  DEVICE=h-c2a95e41d7f0/lora0 FACE=lora PRINCIPAL=lora-node W1_S=1800 \
  timeout 2h taskset -c 4-5 ./measure.sh sf7-zk2c lean zk2c-sf7-lean-1
# W1-73 across lora0's SF9 and SF12 faces, lean (2026-11-21; about 1.6 h and 3.4 h): gen
# ticks at the face's every_s (PERIOD_S), the windows hold 16 and 30, 4 and 8 of them
INST=lora9 BASE=17600 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-lora0-sf9.face.toml \
  DEVICE=h-c2a95e41d7f0/lora0 FACE=lora PRINCIPAL=lora-node \
  PERIOD_S=120 SETTLE_S=40 IDLE_S=1920 W1_LEAD_S=180 W1_S=3600 \
  timeout 3h taskset -c 4-5 ./measure.sh sf9-zk2c lean zk2c-sf9-lean-1
INST=lora12 BASE=17700 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-lora0-sf12.face.toml \
  DEVICE=h-c2a95e41d7f0/lora0 FACE=lora PRINCIPAL=lora-node LIFE=5h \
  PERIOD_S=900 SETTLE_S=120 IDLE_S=3600 W1_LEAD_S=1350 W1_S=7200 \
  timeout 5h taskset -c 4-5 ./measure.sh sf12-zk2c lean zk2c-sf12-lean-1
# W2 zk2c (about 50 minutes), bare, then lean with 16-bit runs and w2.zdict
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 CONTRACTS=$PWD/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c bare zk2c-w2-bare-1
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 RUN_FILE=1 DICT=$PWD/w2.zdict CONTRACTS=$PWD/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c lean zk2c-w2-lean-1
# The honesty checks (about 40 minutes), and the reset alone with a window across both runs
INST=honesty BASE=17600 LOSS_START=0.05 FACE_POLICY=$F/zenohd-rf0.face.toml \
  timeout 2h taskset -c 4-5 ./honesty.sh zk2c-honesty-1
INST=honesty BASE=17600 LOSS_START=0.05 FACE_POLICY=$F/zenohd-rf0.face.toml STEPS=reset \
  timeout 1h taskset -c 4-5 ./honesty.sh zk2c-honesty-reset-1
# W2 zk2c with owner heartbeats (#1110, 2026-11-21; about 50 minutes each, side by side on two
# paths): OWNER_HEARTBEAT=1 and nothing else changed. It needs zenkey-model's zk2 in $WORK/bin.
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 RUN_FILE=1 OWNER_HEARTBEAT=1 DICT=$PWD/w2.zdict \
  CONTRACTS=$PWD/contracts timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c lean zk2c-w2-lean-2
INST=w2b BASE=17700 LOSS_AT_STEADY=0.05 ZK2C=1 OWNER_HEARTBEAT=1 CONTRACTS=$PWD/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c bare zk2c-w2-bare-2
# Its honesty run (about 35 minutes): the host's watch and health beside it
INST=hb BASE=17600 LOSS_AT_STEADY=0.05 ZK2C=1 OWNER_HEARTBEAT=1 WATCH=1 STEADY_S=1200 \
  CONTRACTS=$PWD/contracts timeout 2h taskset -c 4-5 ./measure-w2.sh rf-zk2c bare zk2c-w2-owner-honesty-1
# W2 with desired.v2 and alarms.v2 (#986's J5, #984's S4; 2026-11-27, about 55 minutes each, one
# at a time): V2=1 and nothing else changed; DESIRED_TTL=600 for the watch
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 RUN_FILE=1 V2=1 CONTRACTS=$PWD/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c bare zk2c-w2-v2-bare-1
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 RUN_FILE=1 V2=1 DESIRED_TTL=600 CONTRACTS=$PWD/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c bare zk2c-w2-v2-watch-bare-1
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 RUN_FILE=1 V2=1 DICT=$PWD/w2.zdict CONTRACTS=$PWD/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c lean zk2c-w2-v2-lean-1

# The batch's bound (#1136, 2026-11-28; about 5 and 7 minutes): bare, no loss, the routers'
# ids fixing A->B's first sequence number; bound.sh builds rf0's 32-bit policy itself
P16="data/drop/reliable:211 data/drop/reliable:212 data/drop/reliable:213 data/drop/reliable:214"
P16="$P16 data/block/reliable:209 data/block/reliable:210 data/block/reliable:211"
P16="$P16 $(printf 'data/drop/reliable:100 %.0s' 1 2 3 4)data/drop/reliable:213 data/drop/reliable:214"
P16="$P16 data/block/reliable:211 data_low/drop/reliable:207 data_low/drop/reliable:208 data_low/drop/reliable:209"
INST=bound BASE=17800 ZIDS=a1136003a4d,b1136b PUTS="$P16" \
  timeout 30m taskset -c 4-5 ./bound.sh rf-zk2c bound-16bit-1
P32="data/drop/reliable:209 data/drop/reliable:210 data/drop/reliable:211 data/block/reliable:208"
P32="$P32 data/block/reliable:209 data/drop/reliable+express:208 data/drop/reliable+express:209"
P32="$P32 $(printf 'data/drop/reliable:100 %.0s' $(seq 19))data_low/drop/reliable:206 data_low/drop/reliable:207"
INST=bound BASE=17800 ZIDS=a1136011e8d34,b1136b GAP_S=4 PUTS="$P32" \
  timeout 30m taskset -c 4-5 ./bound.sh rf-zk2c-32 bound-32bit-1

# Analysis
python3 -I analyze.py $WORK/logs/<run> --json results/<run>/analysis.json
python3 -I bound.py $WORK/logs/bound-16bit-1 --json results/bound-16bit-1/analysis.json   # and -32bit-1
python3 -I honesty.py $WORK/logs/{zk2c-honesty-1,zk2c-honesty-reset-1,zk2c-w2-bare-1} \
  --json results/zk2c-honesty.json
python3 -I honesty.py $WORK/logs/zk2c-w2-owner-honesty-1 --json results/zk2c-w2-owner-honesty.json
python3 -I model.py --zk2c > results/model-zk2c.json   # Appendix A's zk2c column, per direction
python3 -I summary.py                                   # the zk2c tables
```

Release candidate 0.23.0 (2026-11-28, main at 1dbce36f): the baselines' commands under new
names, side by side on cores 4–5, and the soaks (about 2.2 h each), each from a frozen copy:

```bash
INST=rf BASE=17400 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-rf0.face.toml W1_S=1800 \
  timeout 2h taskset -c 4-5 ./measure.sh rf-zk2c lean rc-023-w1-lean-1
INST=w2 BASE=17500 LOSS_AT_STEADY=0.05 ZK2C=1 RUN_FILE=1 OWNER_HEARTBEAT=1 DICT=$PWD/w2.zdict \
  CONTRACTS=$R/docs/zk2/measurements/wire-baseline/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c lean rc-023-w2-lean-1
INST=w2b BASE=17700 LOSS_AT_STEADY=0.05 ZK2C=1 OWNER_HEARTBEAT=1 \
  CONTRACTS=$R/docs/zk2/measurements/wire-baseline/contracts \
  timeout 3h taskset -c 4-5 ./measure-w2.sh rf-zk2c bare rc-023-w2-bare-1
INST=lora BASE=17800 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-lora0-sf7.face.toml \
  DEVICE=h-c2a95e41d7f0/lora0 FACE=lora PRINCIPAL=lora-node W1_S=1800 \
  timeout 2h taskset -c 4-5 ./measure.sh sf7-zk2c lean rc-023-sf7-lean-1
INST=lora9 BASE=17600 LOSSLESS_SETUP=1 W1=73 RUN_FILE=1 FACE_POLICY=$F/zenohd-lora0-sf9.face.toml \
  DEVICE=h-c2a95e41d7f0/lora0 FACE=lora PRINCIPAL=lora-node \
  PERIOD_S=120 SETTLE_S=40 IDLE_S=1920 W1_LEAD_S=180 W1_S=3600 \
  timeout 3h taskset -c 4-5 ./measure.sh sf9-zk2c lean rc-023-sf9-lean-1
INST=honesty BASE=17900 LOSS_START=0.05 FACE_POLICY=$F/zenohd-rf0.face.toml \
  timeout 2h taskset -c 4-5 ./honesty.sh rc-023-honesty-1
# The soaks: W2's v2 device and gen on W2's path; gen alone across rf0's plan
INST=soak BASE=18000 LIFE=3h timeout 170m taskset -c 4-5 python3 -I -u ./soak.py \
  --harness $PWD --repo $R --logs $WORK/logs/rc-023-soak-1
INST=soakf BASE=18200 LIFE=3h FACE_POLICY=$F/zenohd-rf0.face.toml timeout 170m taskset -c 4-5 \
  python3 -I -u ./soak.py --face --harness $PWD --repo $R --logs $WORK/logs/rc-023-soak-face-2
# W2's v2 device and gen with the far side opening the lane (20 minutes, 4 reopens)
INST=soaksmoke2 BASE=18300 LIFE=1h FAR_OPENS=1 timeout 55m taskset -c 4-5 python3 -I -u ./soak.py \
  --harness $PWD --repo $R --logs $WORK/logs/rc-023-soak-smoke-2 \
  --cycles 4 --every-s 240 --probe-after-s 40 --telemetry-s 70 --settle-s 40
# A far reader's re-declaration, alone and beside a second reader (about 15 minutes each)
INST=rdA BASE=18400 CASE=alone FACE_POLICY=$F/zenohd-rf0.face.toml \
  timeout 30m taskset -c 4-5 ./redeclare.sh rc-023-redeclare-alone-1
INST=rdB BASE=18600 CASE=shared FACE_POLICY=$F/zenohd-rf0.face.toml \
  timeout 30m taskset -c 4-5 ./redeclare.sh rc-023-redeclare-shared-1
# #1185's check (main at 1dbce36f's binaries): the shared case, its window 1,000 s, and two
# zenoh-python sessions on router B past both readers' re-declarations (the venv
# impl/python/bootstrap.py makes), with the tools' credentials
INST=v1185 BASE=19400 CASE=shared WATCH_S=1000 FACE_POLICY=$F/zenohd-rf0.face.toml \
  timeout 40m taskset -c 4-5 ./redeclare.sh rc-023-repair-check-1 &
P=$R/target/py-venv/bin/python; A=$WORK/cfg/auth/local.json5; K=zk2/h-3fa9c2d41b7e/rf0/modem.v3
sleep 920; timeout 10m taskset -c 4-5 $P -I rawsub.py $A tcp/127.0.0.1:19402 "$K/stream/frames/fast\$*" \
  $WORK/logs/rc-023-repair-check-1/r3p.jsonl 260 &
sleep 80; timeout 10m taskset -c 4-5 $P -I rawsub.py $A tcp/127.0.0.1:19402 "$K/events/face/device_fault\$*/*" \
  $WORK/logs/rc-023-repair-check-1/r4p.jsonl 200
# The re-soak (#1185's branch, about 26 minutes): R1 repairs at 1,200 s
INST=rd2 BASE=19600 CASE=shared WATCH_S=1380 FACE_POLICY=$F/zenohd-rf0.face.toml \
  timeout 45m taskset -c 4-5 ./redeclare.sh rc-023-redeclare-shared-2

# Analysis: analyze.py as above for the five measured runs, then
python3 -I honesty.py $WORK/logs/rc-023-honesty-1 --json results/rc-023-honesty.json
python3 -I soak-analyze.py $WORK/logs/rc-023-soak-face-2 --json results/rc-023-soak-face-2/analysis.json
python3 -I soak-analyze.py $WORK/logs/rc-023-soak-1 --rss $WORK/logs/rc-023-soak-1/rss-swap.jsonl \
  --json results/rc-023-soak-1/analysis.json
```

`rc-023-soak-1` ran an earlier `soak.py`: its probes ran at a fixed 90 s after each restart,
`telemetry` third, and it sampled VmRSS alone. Its RssAnon and VmSwap were read beside it from
`/proc`, once a minute from its 7th minute, into `rss-swap.jsonl`. `rc-023-soak-face-1`, the
face soak with the probes at 90 s, was stopped after two reopens, since `telemetry` there ran
inside the lease the link takes to come back. The soak smoke of `rc-023-soak-smoke-2` ran the
same earlier `soak.py`. `results/rc-023-redeclare-*/analysis.json` holds each declaration the
bare capture decodes and what each reader heard, by a few lines of Python over
`zdecode.decode`'s listing. `rc-023-repair-check-1` was run from a copy of `redeclare.sh`'s
shared case whose two more readers were zenctl, which refused `$*` at its start
(`analysis.json`'s `notes`); the two zenoh-python sessions were then started by hand, at 796 s
and 875 s, as above.

The check that no owner heartbeat crosses to readers with no catalog was
made by hand, and keeps no record in `results/`: `INST=nocat BASE=17800
./wire.sh up rf-zk2c bare`, health.v1's `zenctl echo --across-face` reader
with no `--catalog` on router B, the device (`owner --health --alarms
--config-v2 --catalog FILE --owner-heartbeat`) on router A, the same reader
with `--catalog` 110 s later, and `zdecode.py` over the capture.

`model.py --zk2c` reads `results/zk2c-sf{7,9,12}-lean-1/analysis.json` and
fails where a LoRa row misses its tolerance, so run it after `analyze.py`.
It reads `results/zk2c-w2-{lean,bare}-2/analysis.json` the same way for W2's
owner heartbeat.
The SF9 and SF12 runs take the generator's blocks as they are: `wire.sh`
passes `acl gen --far-config`'s file to `render.py --far`, which checks
both `timestamping` blocks and reads router B's user from it.

That is now `wire.sh`'s default wherever `FACE_POLICY` is set. So the RF and
SF7 commands above, run again, take the generator's blocks too, where the
recorded runs (2026-11-08) set router A's `timestamping` by hand and gave
router B no user of the generator's. `FAR=0` renders them as recorded.

The first `honesty.sh` run's watch probe inside the cut asked `--format json`
of a stream and was refused; the script asks ndjson since, and the whole-run
`watch` it rests on printed nothing in the cut. Its reset step had a window
that held only the new run, so `zk2c-honesty-reset-1` ran the reset alone.

Phase 1's bare run was made from copies of these scripts, before they moved
into this directory, and both phase 1 runs from configs that `render.py`'s
`rf` medium now renders the same, but for the per-path directories. The
lossy-setup runs (`w2-smoke-1`, `w2-smoke-2`, `w2-rf-bare-1`,
`w2-rf-bare-2`) ran with the medium's loss from the start, before
`LOSS_START=0` existed; `w2-rf-bare-1` and `-2` were stopped attempts.

## Files

| | |
|---|---|
| `configs/` | the driver and router templates, which `render.py` fills per medium, stack and path |
| `render.py` | each medium's numbers (`MEDIA`), the layer stacks, the ports and directories of a path; `--ids` the routers' zenoh ids (#1136); `--far-opens` the far side opening the lane with no face (rc-023) |
| `wire.sh` | brings a path up (`up <medium> <stack>`), lists it (`status`), stops it (`down`), and tells the relay to `cut`, `restore` or take a `loss` |
| `measure.sh` | one W1 run: the idle window, W1's setup, the W1 window, `marks.json`; `PERIOD_S` is gen's tick, a face's `every_s` |
| `measure-w2.sh`, `w2-drive.py` | one W2 run: the device and the author through the owner example, the steady window, the acts, the reconnect, the config changes; `OWNER_HEARTBEAT=1` has the device send owner heartbeats under a catalog the far readers hold, and `WATCH=1` adds the host's `watch` and `health` (#1110); `V2=1` puts the desired target and the alarms on desired.v2 and alarms.v2, with a refresh, a change of document and the host's probes among the acts, and `DESIRED_TTL` the watch (#986's J5, #984's S4) |
| `contracts/w2rules.v1.toml` | the device's alarms.v2 table, `alarms.rules`, W2's four rules (lint clean; #984's S4) |
| `contracts/gendev.v1.toml`, `gendev.json`, `gendev-frames.json` | W2's device: 8 gauges, 4 counters, `oper_change` (lint clean); since revision 1.1, `frames/sample`, the twelve readings as one packed record, three a frame, for #989's zk2c run (telemetry.v2, #983 T4) |
| `relay.py` | the userspace netem between the two drivers, and a JSON line per datagram |
| `stamp.py` | stamps each `zenctl echo` row with the wall clock |
| `zdecode.py` | decodes a bare run's SDUs as the fork's codec writes them, and gives each zk2 element its bytes; each put's payload, its own bytes and its SDU's; each message's own bytes and its SDU's in a listing; desired.v2's and alarms.v2's interfaces among the elements |
| `analyze.py` | the drivers' counters and the relay's capture, per direction and window; W2's elements, acts and reconnect; zk2c's frame and event sizes, and the transports opened; on LoRa under zk2c, when each keep-alive and each frame left; freshness.v2's heartbeats per key: their puts, sizes, SDUs and the element a day, decoded bare and placed under the lean stack; a bare window's desired, alarms and heartbeat messages one by one, and each act's messages with their sizes |
| `honesty.sh`, `honesty.py` | #989's honesty checks across rf0's face (zk2c), and what each tool said in them; for #1110's honesty run, each owner heartbeat put, what `watch` printed with the catalog and with none, and the descriptor reads that crossed |
| `setup-loss.py` | what a declaration lost in the setup cost a bare W2 run |
| `bound.sh`, `bound.py` | #1136's bound: puts of chosen lengths and QoS across rf0's face, bare, the routers' ids fixing the first sequence number; each put's batch or fragments, decoded, and the largest payload that fit at each QoS and sequence width |
| `soak.py`, `soak-analyze.py` | rc-023's soak: the zenoh-modem mapping's owners and the host's tools for hours, the link reopened every 10 minutes by a driver's or a router's restart, the probes once the link is back, each process's memory once a minute; `--face` W1-73's path across rf0's plan. Each reopen, what crossed after it, each probe's verdict, and each process's RssAnon and VmSwap over its life |
| `redeclare.sh` | rc-023: a far reader's DeclareSubscriber lost in a cut across rf0's face, and whether its re-declaration (core §8.5) repairs it, alone (`CASE=alone`) and beside a second reader (`CASE=shared`); `WATCH_S` the window after the restore, `READER_LOG` the readers' log filter (#1185) |
| `rawsub.py` | #1185's live check: a plain zenoh-python subscriber on one key expression, a client of one router, each sample a JSON line; zenctl refuses `$*` in a selector |
| `model.py` | runs the report's Appendix A and states its figures per direction; `--zk2c` its zk2c column, each LoRa face's uplink a range by link.v2 §2.16, asserted against its run, and W2's owner heartbeat by its layout, asserted against `zk2c-w2-lean-2` and `-bare-2` |
| `summary.py` | the README's W2, LoRa and zk2c tables, from `results/`, and the heartbeat element with three heartbeats and with one owner heartbeat; the v2 runs' messages, elements and acts |
| `train-dictionary.sh`, `rf.zdict`, `w2.zdict` | the compress layer's dictionaries: `rf.zdict` from `bare-1` (W1), `w2.zdict` from `w2-rf-bare-2` (W2) |
| `results/` | each run's `analysis.json`, `marks.json`, relay summary and rendered `configs/`; for the bound runs (`bound-16bit-1`, `bound-32bit-1`) the capture `air.jsonl` too (a face's generated blocks among them, router B's test password redacted); the dictionaries' inputs (`bare-1/air.jsonl`, `w2-rf-bare-2/air.jsonl`); `setup-loss.json`; `model.json`, and `model-zk2c.json` and `zk2c-honesty.json` for the zk2c runs; `zk2c-w2-owner-honesty.json` and each owner-heartbeat run's `catalog.json` for #1110's; each v2 run's probes, `probe_desired-*.json` and `probe_alarms-*.json`, as `zenctl` printed them, and the watch run's `tasking.v2.toml` (#986's J5, #984's S4); rc-023's runs, `rc-023-honesty.json`, the soaks' `analysis.json`, and the re-declaration runs' bare captures (`rc-023-redeclare-*/air.jsonl`, and #1185's `rc-023-repair-check-1/air.jsonl`), with each reader's re-declarations and repairs in `rc-023-redeclare-shared-2/analysis.json` |

Each relay summary's `loss` is the loss it started with. Under `LOSS_START=0`
that is 0, and the windows' loss was set later on the control port.

Versions:
- zk2 owner and reader: `zenctl` and the owner example from zenkey at
  bedb9802 (main, `git describe` 0.14.0-1465-gbedb9802), zenoh 1.10.1;
- routers: zenoh's fork at tag `zm-1.10.2-r1` (2454b10c), rustc 1.98.1;
- drivers: zenoh-modem's `modem-driver` at d8c62df, built 2026-10-29 with
  every feature, run unchanged;
- zk2c (2026-11-08): `zenctl` and the owner example from zenkey at 2fa3e6ab
  (`git describe` 0.14.0-2019-g2fa3e6ab, #1098's branch), release builds,
  zenoh 1.10.1; the fork at `zm-1.10.2-r1` and `modem-driver` at d8c62df,
  rebuilt the same way;
- zk2c on LoRa SF9 and SF12 (2026-11-21): `zenctl`, the runs' owner (`gen`),
  reader and generator, from zenkey at 5265cc52 (main, `git describe` 0.14.0-2096-g5265cc52,
  built from this branch's harness commit, which changes this directory
  alone), release builds, zenoh 1.10.1; the fork at `zm-1.10.2-r1`;
  `modem-driver` at d8c62df, a debug build with every feature. The runs
  shared cores 4–5 while zk2py's suites ran on the host;
- W2 with owner heartbeats (2026-11-21): `zenctl`, the owner example and
  `zk2` from zenkey at 1546edb3 (#1110's H3, `git describe`
  0.14.0-2213-g1546edb3; this branch's harness commit changes this
  directory alone), release builds, zenoh 1.10.1; the fork at
  `zm-1.10.2-r1` and the same `modem-driver`. The lean and the bare run
  ran side by side on two paths on cores 4–5. A short smoke run of the
  harness shared the cores for the first 6 minutes of their steady
  windows, and the honesty run from then to its end. H3's two later fixes
  (20addfce, an owner heartbeat beside a governed interface's gate;
  60e3de1b, an owner on freshness.v2's first revision) are not in these
  binaries. W2's device has no governed interface and implements
  freshness.v2's revision 1.1 (9e07704d), so neither changes what it puts; that is read from their diffs, not
  measured;
- W2 with desired.v2 and alarms.v2 (2026-11-27): `zenctl` and the owner
  example from zenkey at 0cd0f6ba (`git describe` 0.14.0-2438-g0cd0f6ba),
  release builds, zenoh 1.10.1; the fork at `zm-1.10.2-r1`, rebuilt the same
  way (2454b10c), and `modem-driver` at d8c62df, the binary of 2026-10-29.
  The three runs ran one after the other on cores 4–5, after a 10-minute
  smoke run of the harness (`zk2c-w2-v2-smoke-1`, not kept), whose first
  late joiner without `--clocks-synced` read every firing alarm
  unobservable, its age untrusted.
- release candidate 0.23.0 (2026-11-28): `zenctl`, the owner example and
  `zk2` from zenkey at 1dbce36f (main, `git describe`
  0.14.0-2625-g1dbce36f), release builds, zenoh 1.10.2; the fork at
  `zm-1.10.2-r1` (2454b10c) and `modem-driver` at d8c62df, the W2 v2 runs'
  binaries (sha256 b277f046… and dcac30ff…). Up to eight paths ran at once on
  cores 4–5, after a soak smoke on W2's path (`rc-023-soak-smoke-1`, not
  kept), while other lanes' suites held most of the host's memory.
