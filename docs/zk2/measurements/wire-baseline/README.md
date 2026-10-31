# Today's zk2 wire across zenoh-modem's simulated RF, measured

Part of #989, phase 1 (2026-10-31). The report
[`constrained-overhead-review.md`](../../constrained-overhead-review.md) prices
the wire with a model (its Appendix A). This directory measures one workload of
it end to end: W1, modem.v3's 4 link readings at one a minute, and the idle
link, on RF, in both directions. Only RF is measured here; LoRa, SBD and W2 are
later phases.

## The path

```
zenctl gen (zk2 owner, zenoh 1.10.1, TCP client)
  -> zenohd A (the fork, peer)  --unixsock-stream lane-->  modem-driver A
  -> udp-medium ~> relay.py (5 % loss, 150 +/- 30 ms, in order) ~> udp-medium
  -> modem-driver B  --unixsock-stream lane-->  zenohd B (the fork, peer)
  -> 4 x zenctl echo --no-decode (TCP clients), one per link reading
```

- **Routers.** [`configs/zenohd-{a,b}-*.json5`](configs/) are copies of
  zenoh-modem's `configs/zenohd-rf0.json5` (d8c62df):
  - `mode: "peer"`, gossip and multicast scouting off;
  - lease 10,000 ms, keep_alive 4 and wait_before_close 10 s, as rf0 ships them;
  - `batch_size` equal to the driver's `zenoh_batch_size`: 222 bare, 206 under
    the lean stack;
  - **no lane-face `access_control` and no downsampling.** "Today" is what zk2's
    rules make travel (report §2.2), so the admin space and every zk2
    declaration cross.
- **Drivers.** [`configs/driver-{a,b}-*.toml`](configs/): one `modem-driver` per
  end, so each end's counters are its own direction.
  - The medium is `udp` (feature `lab-air`): `max_sdu = 220`, and `send` paced
    to `rate_bps = 2400`.
  - Driver A's endpoint `b` binds, and router A dials it. Driver B's endpoint
    `a` dials router B's listener.
  - The management plane listens on a unix socket of each driver's own
    (`[management] listen`), not on the data routers. Without the lane-face ACL,
    a management plane attached to a data router would cross the air whenever
    the far side subscribed to it, so the instrument would be in the
    measurement. `zenctl echo --no-decode` reads every reading there, once a
    second (`telemetry_interval_ms = 1000`).
- **The lean stack** (report §3.2), in the order an outgoing SDU meets it:
  - `compress`, with a 1 KB dictionary [`rf.zdict`](rf.zdict) that
    [`train-dictionary.sh`](train-dictionary.sh) trained on the bare run's own
    SDUs;
  - `aead`, `aes-128-ccm` with an 8-byte tag, sealed, one test key, counters
    created by `--init-layer-state`;
  - `overhead` with 2 bytes, standing in for `address`. `layer-address` refuses
    the udp medium at start ("unknown lane */be#0"), because udp-medium
    addresses peers itself. The stand-in is byte-exact (dst, src), and it does
    no filtering, which a point-to-point link does not need.

  That is 16 bytes per SDU when compress stores, as the model assumes.
- **The far side** is four `zenctl echo --no-decode` subscribers on router B,
  one per link reading. zenoh forwards only what a subscriber matches, so the
  owner's 69 other streams, its 7 states and its events stay on host A.
  `--no-decode` makes no presence read and fetches no bundle.
- **The owner** is `zenctl gen h-3fa9c2d41b7e/rf0 modem.v3 --rate 1/60`. It runs
  every resource of `examples/zk2/zenoh-modem/modem.v3.toml` at one a minute on
  host A, and its four link readings are put about 140 ms apart each tick.

### What it simulates, and what it does not

| Simulated | How |
|---|---|
| rf()'s 220-byte frame | udp-medium refuses a larger SDU (I4) |
| rf()'s 2,400 bit/s | each driver's `send` blocks for every datagram's airtime, the 1-byte lane header included |
| rf()'s 5 % loss | relay.py drops each datagram with p = 0.05, seeded, per direction |
| rf()'s 150 +/- 30 ms | relay.py delays each datagram, uniform jitter, never reordering a direction |
| one driver per end | per-direction counters, and the directional `aead` keys work |

| Not simulated | Consequence |
|---|---|
| half duplex and the channel mutex | each direction paces on its own, so both ends can send at once. At W1's load (3 % of the channel) it changes no byte count |
| an airtime counter | udp-medium declares no `capability:airtime`. Airtime here is SimMedium's rf() formula, 8 bits per SDU byte at 2,400 bit/s, which is what `air/airtime_ms_total` would count with no ARQ. It leaves out udp's lane byte |
| ARQ | rf() has none either. A lost SDU is lost: the counters' rx is about 95 % of the far end's tx |
| the address layer's filtering | not needed point to point. Only its 2 bytes are kept |

## Results

Two runs, each on one link: 10 minutes idle (no zk2 client on either router),
then W1 brought up, then 10 minutes of W1 starting 90 s after the owner, so the
window holds exactly 10 ticks. A->B is driver A's `sdu/tx_*` (the owner's side
toward the reader's), and B->A is driver B's. Per day is the window times 144.
The relay's capture agrees with the counters to the SDU, except for one
keep-alive at the edge of the lean idle window on A.

### Idle, per direction

| | SDUs / day | bytes / day | airtime / day |
|---|---:|---:|---:|
| bare, measured (both directions) | 34,559 | 34,559 | 115.2 s |
| lean, measured A->B (relay: 34,559 and 587,509) | 34,415 | 585,061 | 1,950.2 s |
| lean, measured B->A | 34,559 | 587,509 | 1,958.4 s (2.27 %) |
| **model**, lean (§6.2: 69,120 keep-alives a day, both directions) | 34,560 | 587,520 | 1,958.4 s |

Every idle SDU is a keep-alive: 1 byte bare, every 2.50 s each way; 17 bytes
lean (compress's stored marker, aead's 13, the overhead's 2). The model holds
to the keep-alive.

### W1, per direction

| | SDUs / day | bytes / day | airtime / day |
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
(4,770 s of a half-duplex channel, 5.5 %), against the model's 1.52 MB. Of it,
**1.15 MB are keep-alives** and 0.28 MB are W1's readings. A->B is 9.5 % under
the model in bytes, and its data 24 % under. All 40 readings of each window
reached the far side. The 5 % loss took 6 of 261 SDUs A->B, none of them data.

Once per link, outside the windows:
- **The link open** is 214 bytes each way bare, in 3 SDUs: a handshake of 83
  bytes each way, then each router's admin-space declarations (131 bytes,
  `@/<zid>/…`), which rf0's ACL would deny.
- **W1's setup** is about 0.5 KB A->B: gen's operation, state and bundle
  queryables, the instance and interface tokens. B->A it is 0.3 KB, the four
  subscriptions in four SDUs (bare 316 B, lean 219 B).

### Where the model misses

1. **Batching.** The model sends each W1 reading in its own SDU. zenoh batched
   the readings that reached the link while the paced driver was still sending:
   2.5 SDUs a tick bare and 3.1 lean, against the model's 4. This depends on the
   owner's timing: gen's four puts come about 140 ms apart, about one 42-byte
   SDU's airtime.
2. **The put** is 35 bytes for a 2-byte value, decoded from the bare capture:
   push 1, key scope 1, QoS 2, put header 1, HLC 26, Encoding 1, length 1,
   payload 2. The model's put is 40 bytes:
   - it gives the declared key's scope id 2 bytes, where the measured one is 1;
   - it assumes a 6-byte value, where gen's synthetic values were 1 to 3 bytes.

   The HLC's 26 bytes are exactly the model's.
3. **The frame** is 7 bytes (header 1, sequence number 4, QoS 2), where the model
   has 8 with a 5-byte sequence number. Each of the four transports seen began
   below 2^28, so its sequence number is a 4-byte varint.
4. **Compression.** 9 of the 31 data SDUs compressed: each carried two readings,
   whose second HLC repeats the first's 16-byte zid. They saved 153 bytes in
   10 minutes. Single readings and every keep-alive were stored, at the one byte
   the model counts. The dictionary saw only 2.3 KB of W1's own SDUs (49 of
   them); zstd asks for ten times the dictionary's size.
5. **Keep-alives** match the model exactly: one per 2.5 s on an idle direction,
   23 a minute on W1's uplink. With the lean stack they are twice W1's data
   bytes, which is §6.2's point.

## Reproduce

`WORK` defaults to `/srv/dev/targets/zenkey-989`. It holds the binaries and
everything a run writes: rendered configs, sockets, aead state and logs. No
root, no network namespace: TCP on 127.0.0.1 ports 17401–17413 and unix sockets
under `WORK/run`. Every process runs on cores 0–1 under `timeout` and is
stopped by its own PID.

```bash
WORK=/srv/dev/targets/zenkey-989
# zenctl (zenkey main) and modem-driver (zenoh-modem, built with every feature) in $WORK/bin;
# the fork's zenohd, built outside zenoh-modem's tree:
CARGO_BUILD_JOBS=2 taskset -c 0-1 cargo +1.98 install --locked \
  --git https://git.marcpardo.eu/marcpardo/zenoh --tag zm-1.10.2-r1 \
  --root $WORK/zenohd --target-dir $WORK/zenohd-build zenohd
cd docs/zk2/measurements/wire-baseline
timeout 2h taskset -c 0-1 ./measure.sh bare bare-1        # about 23 minutes
./train-dictionary.sh $WORK/logs/bare-1                   # writes rf.zdict
timeout 2h taskset -c 0-1 ./measure.sh lean lean-1
python3 -I analyze.py $WORK/logs/lean-1                    # per direction, per window, per day
python3 -I model.py                                        # Appendix A's figures, per direction
```

The bare run was made from copies of these scripts, before they moved into
this directory. They differed only in rendering the configs from `WORK`.

## Files

| | |
|---|---|
| `configs/` | the two drivers and two routers, bare and lean, with `@WORK@` for the work directory |
| `wire.sh` | brings the path up (`up bare|lean`), lists it (`status`) and stops it (`down`) |
| `measure.sh` | one run: the idle window, W1's setup, the W1 window, `marks.json` |
| `relay.py` | the userspace netem between the two drivers, and a JSON line per datagram |
| `stamp.py` | stamps each `zenctl echo` row with the wall clock |
| `analyze.py` | the drivers' counters and the relay's capture, per direction and window |
| `model.py` | runs the report's Appendix A and states its W1 and idle figures per direction |
| `train-dictionary.sh`, `rf.zdict` | the compress layer's dictionary and how it was trained |
| `results/` | each run's `analysis.json`, `marks.json` and relay summary; the bare run's capture (`air.jsonl`, the dictionary's input); `model.json` |

Versions:
- zk2 owner and reader: `zenctl` from zenkey main at bd1914f2, zenoh 1.10.1;
- routers: zenoh's fork at tag `zm-1.10.2-r1` (2454b10c), rustc 1.98.1;
- drivers: zenoh-modem's `modem-driver` at d8c62df, built 2026-10-29 with every
  feature, run unchanged.
