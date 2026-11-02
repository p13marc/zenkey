# Today's zk2 wire across zenoh-modem's simulated media, measured

Part of #989 (epic #975). The report
[`constrained-overhead-review.md`](../../constrained-overhead-review.md) prices
today's wire with a model, its Appendix A. This directory measures it end to
end, in both directions, on zenoh-modem's media as a lab simulates them:

- **phase 1** (2026-10-31): W1, modem.v3's 4 link readings at one a minute, and
  the idle link, on RF;
- **phase 2** (2026-10-31): W2, the generic device, on RF; W1 and the idle link
  on LoRa at SF7, SF9 and SF12, and SF7 idle at keep_alive 4.

SBD is not measured. Every figure is a day, extrapolated from a window of 10
to 45 minutes, and every one is per direction: A->B is the owner's router
toward the reader's, B->A the way back.

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
| `render.py` | each medium's numbers (`MEDIA`), the layer stacks, the ports and directories of a path |
| `wire.sh` | brings a path up (`up <medium> <stack>`), lists it (`status`), stops it (`down`), and tells the relay to `cut`, `restore` or take a `loss` |
| `measure.sh` | one W1 run: the idle window, W1's setup, the W1 window, `marks.json` |
| `measure-w2.sh`, `w2-drive.py` | one W2 run: the device and the author through the owner example, the steady window, the acts, the reconnect, the config changes |
| `contracts/gendev.v1.toml`, `gendev.json`, `gendev-frames.json` | W2's device: 8 gauges, 4 counters, `oper_change` (lint clean); since revision 1.1, `frames/sample`, the twelve readings as one packed record, three a frame, for #989's zk2c run (telemetry.v2, #983 T4) |
| `relay.py` | the userspace netem between the two drivers, and a JSON line per datagram |
| `stamp.py` | stamps each `zenctl echo` row with the wall clock |
| `zdecode.py` | decodes a bare run's SDUs as the fork's codec writes them, and gives each zk2 element its bytes |
| `analyze.py` | the drivers' counters and the relay's capture, per direction and window; W2's elements, acts and reconnect |
| `setup-loss.py` | what a declaration lost in the setup cost a bare W2 run |
| `model.py` | runs the report's Appendix A and states its figures per direction |
| `summary.py` | the README's W2 and LoRa tables, from `results/` |
| `train-dictionary.sh`, `rf.zdict`, `w2.zdict` | the compress layer's dictionaries: `rf.zdict` from `bare-1` (W1), `w2.zdict` from `w2-rf-bare-2` (W2) |
| `results/` | each run's `analysis.json`, `marks.json`, relay summary and rendered `configs/`; the dictionaries' inputs (`bare-1/air.jsonl`, `w2-rf-bare-2/air.jsonl`); `setup-loss.json`; `model.json` |

Each relay summary's `loss` is the loss it started with. Under `LOSS_START=0`
that is 0, and the windows' loss was set later on the control port.

Versions:
- zk2 owner and reader: `zenctl` and the owner example from zenkey at
  bedb9802 (main, `git describe` 0.14.0-1465-gbedb9802), zenoh 1.10.1;
- routers: zenoh's fork at tag `zm-1.10.2-r1` (2454b10c), rustc 1.98.1;
- drivers: zenoh-modem's `modem-driver` at d8c62df, built 2026-10-29 with
  every feature, run unchanged.
