# Bandwidth-first wire redesign for zk2 on constrained links

**Date:** 2026-10-31  
**Status:** redesign proposal; non-normative. It **breaks wire compatibility by design**: zk2c becomes zk2's one wire, on every bus (decided 2026-10-31). No specification, contract, generated artifact or runtime was changed. The work it proposes is epic #975.  
**Repository:** zenkey `264ea30db2ef54d71d1500a40f6c0e8f45afb792`; zenoh-modem `95f884bce310ca4bf33f77cf9cf756b4df41a83e`  
**Scope:** zk2 core (descriptor, presence, keys, state reads, operations, events, archive), all eleven profiles, and how zk2 uses Zenoh 1.10 (stamps, Encoding, keep-alive, lease, declarations, batching).  
**Links:** the zenoh-modem media: RF packet radio, LoRa SX1262 and Iridium SBD. Zenoh stays the transport.

## 1. Executive summary

**Low bandwidth is the priority, and compatibility may break.** On that brief, zk2's wire spends most of a constrained link re-sending what has not changed. It spends the rest on per-message metadata that is larger than the information it carries.

The reference device (W2, §2.2) on zenoh-modem's 2,400 bit/s RF link sends **18.4 MB a day today, 71 % of the channel**. Under the compact wire proposed here, "zk2c", it measured **669 KB a day, 2.6 %: 25 times less** than today's wire measured the same way, 16.5 MB (§9). The model, corrected to that run, gives 674 KB.

| Element, W2 on RF (both directions) | Today, KB/day | zk2c measured, KB/day | zk2c model, KB/day | What changes |
|---|---:|---:|---:|---|
| Telemetry, 8 gauges + 4 counters every 10 s | 10,843 | 334 | 334 | One packed vector frame per 30 s, widths from declared ranges, on a declared key (§5.3) |
| config.v1 view and status re-puts | 6,003 | not measured | 0.3 | The schema moves to the contract; values are sent on change; the heartbeat confirms them |
| Keep-alives | 1,028 | 22 | 25 | One every 60 s, lease 240 s with keep_alive 4; data suppresses uplink keep-alives |
| health.v1 status re-puts | 252 | 0.2 | 0.2 | Sent on change; the heartbeat confirms it |
| alarms.v1 confirmations and transitions | 136 | 2.0 | 0.4 | Confirmations are the heartbeat's; transitions stay alarms.v1's JSON documents, where the model has 8-byte records |
| desired.v1 periodic re-reads | 91 | 84 | 64 | Today's GET every 300 s, its reply the whole document; a conditional GET by version would make it 27 |
| Reconnects (4 a day) | 11 | 12 | 12 | Instance token only, one key scope per kind; the far side's subscriptions again, one per resource |
| Events (15 a day) | 4 | 1.3 | 0.7 | A declared scope and the occurrence chunk instead of a ULID key |
| Heartbeat (new) | — | 238 | 238 | One put per interface with a collection: health.v1's and the config's every 28.5 s, alarms.v1's every 427.5 s, 16–18 B each |
| **Total** | **18,367** | **669** | **674** | **25× less measured, 27× modelled** |

The today column is Appendix A's model, corrected to today's wire as measured across zenoh-modem's simulated RF and LoRa ([`measurements/wire-baseline/`](measurements/wire-baseline/README.md)). The zk2c columns are the same baseline's zk2c runs, on RF at link.v2's transport, and Appendix A's model corrected to them (its section 10). Appendix A asserts each tolerance. The measured column gives each element from the bare run's decode, with the lean stack's 16 B an SDU added (32-bit runs, 695 KB in all); its total is the lean run's counters, with 16-bit runs.

The first zk2c model said 443 KB, 41 times less. It missed three things the reference's wire does. The heartbeat is one put per interface with a collection, never 18 B inside the telemetry frames: 52 KB modelled, 238 KB measured. Desired's re-read is today's GET, its reply the whole document: 27 KB, 64 KB. And a reconnect re-sends the far side's subscriptions, one per resource by its exact key: 3.5 KB, 12 KB.

Today's column is what the owner sends. Measured, 16.5 MB of it reached the air, with compress storing as the model assumes. Router A's `drop` congestion control discarded 19 % of the telemetry before the air. That is a loss today's wire inflicts, not a saving (§9).

**The six moves that give most of it:**

1. **Do not resend unchanged documents.** Today freshness re-puts whole documents every ttl/2: config.v1's 1,286-byte view, health status, alarm documents. Instead, one **heartbeat** confirms every member the owner still vouches for, one bit each (§4.4, §6.1).
2. **Reason in bits from the declared contract.** A field confined to 0–1000 takes 10 bits. Fields are packed contiguously, so a 10-bit value and a 6-bit value share one 16-bit word, and a message is padded once, at its end (§2.1, §4.2).
3. **Replace the per-put HLC stamp.** Zenoh's stamp costs 26 bytes per put: a 9-byte time, a length byte and a 16-byte session id. It is replaced by an in-band owner stamp of **40 bits**, a run and a sequence number. Time travels only where a profile's semantics need it (§4.3).
4. **Batch records into frames with implied time.** A frame carries one base time; periodic records take their time from `timing.v1`'s declared period. Every frame starts from absolute values (§4.4).
5. **Tune the lease per medium.** At Zenoh's defaults, idle keep-alives cost 69,120 messages a day in both directions [Z12]. zenoh-modem's tuned settings, and data that refreshes the lease, remove most of them (§6.2).
6. **Keep static context out of the wire.** Today's wire repeats series names, kinds, units, rule texts, parameter schemas, fingerprints and full keys. These move into the contract and a fingerprinted deployment catalog. On the wire they become small indices (§4.5).

**Per medium:**

- **RF, 2,400 bit/s, 220-byte frames.** zenoh-modem's 73 modem readings fit in **168 KB a day at one per minute**, 24 of them moving (§5.11). Measured across rf0's face with every reading moving every other minute, their frames took 207 KB. Today the 4 readings allowed across the face cost 334 KB a day at that rate, and the uplink's keep-alives 563 KB more. Measured, the readings took 281 KB, since zenoh batched some of them.
- **LoRa SX1262, 1 % duty cycle (864 s of air a day).**
  - No LoRa face ships today, so "today" is what the current wire would cost. On today's wire the 4 link readings at one per minute exceed the duty budget at every spreading factor. Measured, the uplink took 999 s a day at SF7, 2,494 s at SF9 and 14,032 s at SF12; the model gives 909, 3,100 and 21,527 s.
  - Keep-alives and reopens are part of it. At zenoh-modem's keep_alive of 2, every lost keep-alive closed the link, and each reopen re-sends the handshake and every declaration (§6.2).
  - zk2c fits the 4 link readings at SF9 in 473 s. At SF7, all 73 readings fit at one per minute: 288 s a day with 24 moving. Measured across lora0's face with every reading moving every other minute, the uplink took 474 s, 55 % of the budget, its keep-alives included.
  - At SF12, all 73 fit only as an hourly keyframe plus 15-minute deltas, at 88 % of the budget.
- **Iridium SBD, 270 bytes each way per message.** A message is the bill, so bits buy *records per message*. A full keyframe of all 73 readings fits in one message (230 bytes) if its counters wrap at 24 bits, a declared loss (§5.11). Each data message refreshes the lease, so it can replace a keep-alive the tuned link sends anyway (§6.7).

**What cannot shrink:**
- genuinely new text (summaries, free-text reasons, text parameters);
- about 4 bytes per Zenoh put, and per-frame framing (5 bytes of Zenoh frame and 16–29 bytes of zenoh-modem layers), except by batching;
- the handshake and declarations at each reconnect;
- downlink keep-alives;
- one heartbeat per declared horizon;
- the entropy of counters;
- retransmissions and authentication tags.

**Out of scope.** Implementation footprint. The Rust runtime is a Tokio/heap reference implementation, not a microcontroller one; a pico-based owner is a separate implementation of the same wire.

## 2. Method

### 2.1 From information to billing units

A field with lower bound L, upper bound U and resolution r has R = (U − L)/r + 1 codes, plus any reserved codes. A fixed-width field needs ⌈log₂ R⌉ bits.

| Step | Rule | Example |
|---|---|---|
| 1. Information | ⌈log₂ R⌉ bits per field, from the contract's declared range, resolution and reserved codes | 0–1000 → 10 bits; −40…85 °C at 0.1 → 11 bits, with 2 reserved codes still 11 |
| 2. Record | Fields packed MSB first in canonical order, **no padding between fields** | duty available 0–100.0 % at 0.1 (10) · signal bars 0–5 (3) · oper status of 4 values (2) · admin (1) = **16 bits exactly** |
| 3. Message | One padding, at the end, fewer than 8 bits; the generator reports it and the contract may claim it (an extension bit, a flag, finer resolution) | a 40-bit record is 5 bytes; a 41-bit one is 6, leaving 7 spare bits to claim |
| 4. Zenoh | ≈4 bytes per put on a declared key scope, more with a stamp, Encoding or attachment (§3.1) | |
| 5. Batch | Several puts share one Zenoh frame (5 bytes) and one medium SDU | |
| 6. Medium | zenoh-modem layers, then the medium's unit: airtime (RF, LoRa) or messages (SBD) (§3.2) | LoRa at SF12 bills in 51-byte tiles of 2.47 s each (0.62 s at SF10, 1.31 s at SF11) |
| 7. Day | Multiply by events per day, both directions, recovery included | |

**Spare bits within a record do not exist.** The user's example is a 10-bit field in a 16-bit word, with 6 bits left over. Under contiguous packing those 6 bits *are* the next field, so word boundaries never waste anything. The only spare bits are the final padding, fewer than 8 per message. Batching many records into one frame pays that padding once per frame, not once per record.

**Reason in bits, but count messages first.** Below the payload, each Zenoh put costs 4 bytes, and each SDU costs 5 bytes of frame plus 16–29 bytes of zenoh-modem layers. On SBD each message is a bill. A saving of 30 payload bits matters less than sending half as many messages.

### 2.2 Workloads

- **W1, zenoh-modem's own device.** An `rf0`, `lora0` or `sat0` service as a zk2 owner of `modem.v3` [`examples/zk2/zenoh-modem/modem.v3.toml`](../../examples/zk2/zenoh-modem/modem.v3.toml):
  - 73 telemetry streams (51 counters, 22 gauges);
  - 7 states, each with `freshness.ttl_s = 60`;
  - 4 events and 2 operations;
  - plus health.v1 and config.v1.
  
  Today only 4 readings are `link`-exposed, downsampled to one per minute on RF; nothing crosses SBD [Z49].
- **W2, a generic constrained device.** 8 gauges and 4 counters every 10 s; health.v1 with 4 status changes a day; alarms.v1 with 64 rules and 3 continuously firing, 10 transitions a day; config.v1 with 3 groups and 10 parameters, 5 changes a day; a desired.v1 target re-reading every 300 s across a face (the period is the deployment's; 300 s is ZenSight's); 15 events a day, a modem.v3 `oper_change` as the example; 4 reconnects a day. Four is what RF's keep-alives give at 5 % loss: three lost in a row close the link, 34,560 × 0.05³ a day on the idle downlink.

  "Today" assumes each element crosses the hop as zk2's rules make it travel, with an HLC-stamped owner session. Today's link.v1 faces deny much of this. That visibility, lost at the face today, is what the redesign buys back.
- **The sensor record.** Four readings: 21.5 °C (−40…85, 0.1), 43.5 % RH (0…100, 0.5), 1013.2 hPa (300…1100, 0.1), 3.70 V (2.00…4.20, 0.01). Packed, they take 11 + 8 + 13 + 8 = 40 bits.

| Sensor record as | Payload | Put today on a declared scope (HLC stamp, Encoding) | zk2c mutation (6-byte header + record) |
|---|---:|---:|---:|
| JSON, descriptive member names | 68 B | 99 B | 78 B |
| JSON, one-letter names | 38 B | 69 B | 48 B |
| CBOR array of scaled integers | 11 B | 42 B | 21 B |
| Byte-aligned fixed point | 6 B | 37 B | 16 B |
| **Bit-packed from range and resolution** | **5 B** | 36 B | **15 B** |

The table's lesson is the third column. Once a payload is packed, today's per-put stamp is five times the information. **Bits first must include the metadata.**

## 3. What the hop costs below the payload

### 3.1 Zenoh 1.10.1

Zenoh's minimum overhead is often quoted as 5 bytes [Z1]. That is the floor of the layout below, read in the cached 1.10.1 codec, protocol, transport and configuration crates [Z3]–[Z10]:

| Item | Bytes | Note |
|---|---:|---|
| PUSH header, key scope, PUT header, payload length | 4 + payload | A declared key's scope id is 1 byte. A key nobody declared toward the far side sends its text after the far subscriber's scope. A stream's key is one: a stream has no queryable, so no router declares it. W2's streams send 40–52 bytes of text |
| HLC timestamp on a put | **26** | NTP64 as a varint (9), id length (1), a 16-byte zid. Owner sessions stamp every put when timestamping is on; S1 requires it on every state put and delete |
| Encoding | 0 / 1 / 3 + 1 + n | Omitted for `zenoh/bytes`. One byte for a predefined id (JSON 5, CBOR 8, protobuf 13). A non-predefined media type costs id 0xFFFF plus the whole string, on every put |
| Attachment | 2 + n | e.g. telemetry's 18-byte JSON Epoch: 20 bytes |
| QoS extension | 2 | On the PUSH whenever priority is not `data`, congestion not `drop`, or express is set: alarms, every `block` state or event, modem.v3's `data_low` telemetry. On the frame only for a priority other than `data` |
| Declaration | 3 + body | DECLARE header and QoS, then the token, subscriber, queryable or key expression |
| Frame | 1 + SN | Zenoh masks its 32-bit sequence number to 28 bits, so it is at most a 4-byte varint [Z55]; at 16 bits, at most 2 |
| Reply | 27 + key + put | A RESPONSE names its key in text, then QoS and the responder's 20-byte id; the final adds 4 |
| Keep-alive | 1, in its own SDU | Sent when nothing else was written within lease / keep_alive; a batch resets the timer [Z7]. zenoh-modem measured one byte each [Z17] |
| Varints | LEB128-style | 7 bits per byte, at most 9 bytes for a u64 [Z5] |

The wire baseline's captures decode to these layouts, the 26-byte stamp included (§9).

zenoh-modem adds:
- the handshake is 6 SDUs and 192 bytes each way, and a batch must be at least 80 bytes [Z19]. On the baseline's path it was 83 bytes in 2 SDUs each way, then 131 bytes of each router's admin-space declarations;
- a flapping link pays every key's declaration again at each reconnect, about 1.2 s of air per key at 300 bit/s [Z52]; Zenoh coalesces the declaration messages in client/router mode [Z26], and a coverage gap costs about 5.3 paid messages on SBD [Z27];
- a lost `DeclareKeyExpr` is never repaired without link ARQ [Z20]. The baseline saw one silence a device's health for a whole transport;
- at keep_alive 2, a single lost keep-alive closes the link (§6.2).

### 3.2 zenoh-modem's layers and media

The driver adds no byte of its own: Zenoh's 2-byte stream length is stripped [Z19]. The optional layers apply to each SDU in the order compress → aead → fec → fragment → address:

| Layer | Bytes | Source |
|---|---|---|
| compress | 1 (stored) or 3 + zstd body, 1 KB pre-shared dictionary | [Z25], [Z13] |
| aead | header 1 + counter 4 + tag 4/8/16 = 9–21 | [Z15] |
| fec | 5, plus r repair SDUs per k: the shipped k = 4, r = 1 adds 25 % SDUs, measured at 1.37 frames per SDU and about 70 % more air per byte | [Z23], [Z50] |
| fragment (SCHC No-ACK) | 2 per tile, 6 on the last | [Z22] |
| address | 2 | [Z24] |

This report's **lean stack** is compress + aead with an 8-byte tag + address = **16 bytes per SDU**. The full stack, with fec and a 16-byte tag, is 29 bytes plus the repair frames.

| Medium | Frame | Bill | Loss |
|---|---|---|---|
| RF | 220 B at 2,400 bit/s (733 ms); 25.9 MB/day, half duplex | airtime | 5 %; link ARQ required, 3 retries being enough up to 20 % loss [Z20] |
| LoRa SX1262, EU868 | 222 B at SF7–8, 115 at SF9, 51-byte fragment tiles at SF10–12 [Z21]; Semtech's time-on-air formula [Z47] | airtime, within 1 % duty: 36 s an hour, 864 s a day | 5 % |
| Iridium SBD (9602/9603) | 340 B up, 270 down; Zenoh negotiates 270 each way, so 20.6 % of each uplink slot is wasted [Z18] | **per message**: every `SBDIX`, retries and polls included [Z51][Z12] | 10 % per attempt, reliable |

All media figures are zenoh-modem's simulator [Z14] and lab, not flights: "no byte has reached an antenna" [Z11]. Iridium's commercial plans are commonly described as billing bytes with a per-message minimum. That could not be verified at a primary source, so this report follows zenoh-modem's per-message model and notes where byte billing would change a conclusion.

**Consequences for the design:**
- Per-SDU cost is 21–34 bytes before any payload, so **records must share SDUs**.
- On LoRa, airtime grows in symbol blocks and, at SF10–12, in 51-byte tiles: 0.62 s at SF10, 1.31 s at SF11, 2.47 s at SF12 (zenoh-modem quotes about 2.6 s).
- On SBD, the unit is the message, so the goal is fewer messages carrying more records.

### 3.3 zenoh-modem's compression is not a substitute

With a 1 KB trained dictionary, zenoh-modem's compress layer saves 74 % of a telemetry workload and 72 % of 200-byte sensor JSON documents; without one, 6–26 % [Z13]. zstd's own documentation explains why: a small input has no past to learn from, so it compresses well only with a dictionary [Z48].

zk2c does better by construction. The contract is the dictionary, and it is exact: the sensor record above is 38 bytes of terse JSON and 5 bytes packed (87 %), before counting the 26-byte stamp the dictionary cannot remove. Packed records leave a compressor nothing to find. The layer then sends its 1-byte "stored" form; zenoh-modem's idle-link measurement shows compression costing 22 % when nothing compresses [Z13]. **Turn the compress layer off for zk2c traffic**, or keep it only for residual free text.

## 4. The compact substrate, zk2c

**One wire on every bus.** zk2c is not a constrained mode. Its core mechanics (owner stamp, snapshot, heartbeat, compact descriptor, error envelope, catalog) and every profile's records are zk2's wire everywhere, so an implementation has one code path and a host reader decodes a radio device's traffic the way it decodes its neighbour's. Application payload types gain a `packed` kind beside JSON Schema and protobuf. Both stay allowed on a host bus, and a lint requires `packed` for a resource exposed on a constrained face.

### 4.1 Prior art it borrows

| Technique | Precedent | Used for |
|---|---|---|
| Minimal-width constrained integers, no padding between fields | ASN.1 unaligned PER [Z28][Z29] (3GPP RRC, RANAP, NBAP); bitproto [Z31] | every field (§4.2) |
| Scale, offset, range per signal | CAN DBC [Z32] | fixed point |
| Reserved "error" and "not available" codes | SAE J1939 practice [Z33] | missing ≠ zero |
| Shared static context, residues in bits, padding once | SCHC, RFC 8724 [Z30] | the contract as context (§4.5) |
| Numeric identifiers instead of names, delta-coded map keys | YANG-CBOR SIDs, RFC 9254 [Z35]; Sparkplug aliases with rebirth [Z37]; SenML CBOR labels [Z34] | catalog indices, slots |
| Base time plus relative times | SenML `bt` [Z34] | frames (§4.4) |
| Delta-of-delta timestamps; zigzag, per-block width | Gorilla [Z38]; Sprintz, under 1 KB of memory [Z39] | implied time; residues |
| Decimals as integers | ALP [Z40] beats XOR float codecs (Gorilla, Chimp, Elf [Z41]) | declared resolution instead of floats |
| Rice codes with an escape | FLAC, RFC 9639 [Z42] | skewed values |
| Send-on-delta, deadband | Miśkowicz [Z43]; OPC UA `DataChangeFilter` [Z44]; dual prediction [Z45][Z46] | declared deadband (§5.11) |
| A compact periodic advertisement | VRRPv3: an 8-byte header with an 8-bit priority and a 12-bit interval in centiseconds [Z36] | the heartbeat (§4.4) |

### 4.2 Field types

| Type | Wire | Declared in the contract |
|---|---|---|
| Bounded uint `[L, U]` | offset from L in ⌈log₂(U − L + 1 + k)⌉ bits; the k reserved codes sit at the top | `minimum`, `maximum`, reserved codes (not available, error, out of range) |
| Fixed point | a bounded uint | scale, offset, resolution; the error bound r/2 is part of the declaration |
| Enum, bool | ⌈log₂(n + escape)⌉ bits; 1 bit | values in order; an escape when the enum is open |
| Bitmap, presence bitmap | 1 bit per member or per optional field | member order from the canonical form |
| Counter | absolute, at a declared width | width; reset identity is the owner's run (§4.3). A width below 64 bits wraps, a declared loss: a late joiner cannot recover the total, and more than one wrap between two readings goes unseen |
| Time | 32-bit seconds since the deployment epoch, optionally plus 10 bits of milliseconds; offsets inside a frame | resolution, epoch |
| Unbounded value, length | LEB128; Rice(k) with an escape for skewed residues | `"unbounded"` spelled out, as draft 2 does for cardinality |
| Text | a dictionary index; the all-ones index is an escape, then LEB128 length and UTF-8 | the dictionary (rule texts, reasons, codes) |
| Extension | one trailing bit, as UPER's extension marker | whether the type may grow |

`multipleOf` is refused by zk2's JSON Schema subset ([core §7.3](../../spec/core.md)), and 0.1 has no exact binary form anyway, the problem ALP's authors describe [Z40]. Resolution is therefore an explicit declaration on an integer, not a property of a float.

**Evolution.** Widening a range is a breaking change in zk2c, because the range is the wire. The compatibility classifier ([core §9.8](../../spec/core.md)) judges per encoding: appending an optional field behind the extension bit is compatible, and moving a bound is breaking.

### 4.3 The owner stamp: run and sequence instead of the HLC

| Guarantee today | What gives it in zk2c | Bits |
|---|---|---|
| S1/S2: order per key, the owner's own stamp | one owner-wide monotone sequence; a new run starts before it wraps | 24 |
| S7: a new instance's values accepted | the run: a 16-bit persistent boot counter, or 32 bits random where nothing persists or several candidates run (redundancy.v1) | 16 or 32 |
| core §4.3's residual risk: a late value from an old epoch read as new | closed for a persistent counter run, which orders runs; a 32-bit random run, or a candidate-and-counter run under redundancy.v1, keeps the residual risk, and a counter run must widen before it wraps. A device that finds no counter (a factory reset, a flash wipe) cannot tell a first boot from a lost counter, and a counter restarted at zero would order its runs below the old ones, so its new values would read as an old epoch's. While it holds no counter it takes a 32-bit random run, which keeps the residual risk | 0 |
| Clock ahead stops writes (S7, spike S12) | order no longer depends on the clock; a clock ahead marks time unknown (reserved value) and writes go on | 0 |
| Telling a previous instance's put apart (alarms.v1, desired.v1, redundancy.v1 fencing) | run ↔ 64-bit nonce in the birth record and the compact descriptor | 0 per message |
| Freshness aging, archive display | the confirmation time in the heartbeat and in snapshots | 32 |
| timing.v1 lifespan | a per-sample time, only on resources that declare a lifespan | 32 + 10 |
| Loss detection (new) | each collection's high-water sequence in the heartbeat, the full 24 bits | 24 |

**Each mutation carries run16 · seq24 = 40 bits, against 208 for the HLC.** Owners run with Zenoh timestamping off, on every bus. So does every router that would stamp traffic before it crosses the face; a router stamping after the hop costs the hop nothing.

Two consequences:
- **S4 reads use consolidation `None` and merge in-band.** With P3 each key has one owner, so `Latest` added nothing.
- **Zenoh's storage manager cannot order zk2c keys by timestamp.** Union event storages and archives must order by the in-band stamp (§7, case 7).

### 4.4 Message forms

The Encoding is always `zenoh/bytes`, so it costs 0 bytes. The key's contract names the type. A header byte `CH` holds the form (3 bits), the run width (1 bit) and the low 4 bits of the layout's hash. The tag, an exact-length check and the trailing extension bit together refuse a payload decoded against the wrong revision.

| Form | Layout | Size |
|---|---|---|
| Mutation | CH8 · run16 · seq24 · record | 6 B + record |
| Delete | Zenoh DEL; stamp in a 5-byte attachment | ≈11 B |
| Frame | CH8 · run16 · seq24 (first record; +1 each) · t32 · records. Periodic records are dated base + i × `timing.period_ms`, flagged scheduled within a declared jitter; off-schedule records carry a Rice-coded offset. **Every frame starts from absolute values.** | 10 B + records |
| Heartbeat | CH8 · run16 · seq24 · t32 · resource map 8 · for each collection with a horizon, its confirmed members (a bitmap or a slot list, whichever is shorter, behind a 1-bit selector) and its 24-bit high-water seq | 10 B + bits on its own. Inside a telemetry frame, whose header it shares, **18 B extra** for W2 (three single members, 3 firing alarms as a list, 4 collections) |
| Birth | CH8 · run16 · mode2 · rsv6 · nonce64 · t32 · catalog32 | 20 B, once per run |
| Snapshot (replaces S4's per-key replies) | CH8 · run16 · asof24 · t32 · live count · tombstone count · live entries (id · mseq24 · record · binding) · tombstones (id · dseq24) · more1 | one reply per collection |

**The heartbeat is the central new element.** One heartbeat, every half of the shortest declared horizon, replaces:
- freshness.v1's re-puts;
- alarms.v1's 450-second confirmations;
- health.v1's 30-second status re-puts;
- config.v1's 30-second view and status re-puts;
- the periodic re-reads a far reader makes to learn that nothing changed.

A cleared bit means "no longer vouched for", so a stale value is never presented as fresh. A moved high-water seq tells the reader it missed a mutation and must read the snapshot.

### 4.5 Identifiers and static context

| Today on the wire | zk2c |
|---|---|
| 16-hex instance id in tokens, descriptor, config tokens, epochs | the run; the 64-bit nonce only in the birth record and the descriptor |
| zid inside every stamp (16 B) | off the wire |
| 71-character fingerprint in the descriptor; `fp16` in interface tokens | a 10-bit index into a **fingerprinted deployment catalog** (its 32-bit id in the birth record) |
| 26-character ULID per event key, never declarable | 8 base32 characters of run16 ‖ seq24, as the suffix of one declared `events/<template>` scope |
| 16-hex FNV alarm key | an 8-bit slot, covering the profile's bound of 256, bound to its rule and labels by a binding record that every snapshot repeats |
| Full text keys of 48–80 B, a stream's sent whole on every put | unchanged as text, for routing, ACLs and inspection, but declared once per (service, interface, kind) per session; each put adds a 1–3-character suffix (§5.3) |
| Series names, kinds, units, bounds, parameter schemas, rule texts, reasons | the contract, compiled by zenkey-build into the layout |

**The catalog** holds the (interface, fingerprint) pairs and the dictionaries a deployment shares. It is provisioned with the firmware, or by holders on the near side, as bundles are today ([core §8.4](../../spec/core.md)). A constrained owner never fetches a bundle.

**How a reader decodes:**
1. It resolves the interface from the key text.
2. It takes the fingerprint from the compact descriptor on the near side, or from the deployment's word across a face.
3. It loads the generated layout and checks the tag and the length.
4. It decodes, takes times from the frame or heartbeat, and maps the run to an instance through the birth record.

zenctl and zengui render the result as JSON, so readability lives in the tools, not on the radio. Recordings (`.zrec`) and archives keep the compact bytes with the catalog id, so an offline reader decodes them the same way.

## 5. Per-element redesign

Each element is given as today's wire, what must be kept, the compact layout, the cost after, and the risk. Unless a W1 or W2 figure is cited, bit widths are the layout's and byte counts are rounded up.

### 5.1 Descriptor

- **Today.** JSON of about 680 bytes plus an HLC stamp, put whole at start and on every change (re-mint, standby take-over or step-down) [core §3](../../spec/core.md). It SHOULD stay at or under 1 KB, and at the constrained level it MUST fit one fragment (4 KB is zenoh-pico's default).
- **Keep.** Exposure, standby and token marks, unavailable resources, bindings, `meta.zid`'s role (now the nonce), D-checks.
- **zk2c.** CH8 · run16 · nonce64 · catalog32 · interface count 4 · minted 1 · reserved 3, then per interface: catalog index 10 · minor 6 · token 1 · standby 1 · unavailable-present 1, then the unavailable list where present. Then capabilities 16 · binding set 12 · unbound 4. With four interfaces and no unavailable resource that is 236 bits, **30 B**. The D-codes are checked after decoding.
- **Risk.** An unknown catalog id must be refused, never guessed.

### 5.2 Presence and tokens

- **Today.**
  - Instance token ≈55-byte key; interface token ≈76-byte key; member tokens for epoch templates.
  - Computed from key lengths: 277 bytes per device per reconnect. The constrained scenarios record 100 tokens taking about 7 KB and 24 s at 2,400 bit/s ([link.v1](../../spec/profiles/link/v1.md); [scenarios/constrained](../../spec/scenarios/constrained.md)).
  - Backhaul faces already deny `@zk` ([core §8.5](../../spec/core.md)).
- **Keep.** Instance alive implies callable; standby and twin detection on the same side of a face.
- **zk2c.** On the device-to-router hop, the device holds **only its instance token**. Every interface behind the hop is tokenless (core's tokenless set), and liveness across faces is the heartbeat, as §8.5 already makes freshness the evidence.
- **Risk.** Interface-token selectors miss first-hop devices; tools read the compact descriptor instead. Governed interfaces (redundancy.v1) and archives stay on the near side of the hop.

### 5.3 Keys and declarations

- **Today.**
  - A router declares a key expression toward the far side only for a queryable, a subscriber or a token. A publisher gets none.
  - So a stream's put carries its key as text: 77–85 bytes for a W2 gauge, against 36 on a declared scope (baseline, W2 finding 2).
  - W2's device declares 21 key expressions with their queryables, tokens and subscription, about 1.6 KB, paid again at each reconnect.
- **zk2c.** One declared scope per (service, interface, kind); each put names its member by a 1–3-character suffix. Templated members, which link.v1 cannot carry across a face today, travel inside one concrete frame key.
  - Today's routers declare no key expression under a stream. A compact key for a stream's frame has to come from #980's catalog or #988's frame resources.
- **After.** W2's reconnect cost falls from 11.1 to 3.5 KB a day.
- **Risk.** A lost `DeclareKeyExpr` is never repaired [Z20]. Link ARQ is required, as zenoh-modem already concludes. The baseline saw a lost one drop 25 of 25 health puts at the far router, silently.

### 5.4 State reads and tombstones

- **Today.** S4 is a GET answered with one reply per live key, each with its stamp, plus a `reply_del` for every delete within the tombstone window. That window is at least the face's maximum outage, and each tombstone costs about 100 bytes ([core §4](../../spec/core.md); link.v1).
- **Keep.** S2/S3 completeness; an exact seed boundary; deletes reported.
- **zk2c.** One **snapshot** per collection on a literal-prefix key. For alarms with 3 live entries and 2 tombstones it is 63 bytes, against about 1,010 today. Each live entry repeats its binding (rule and labels), so a late joiner can name every slot. `asof` gives the seed boundary, and an empty snapshot is the owner's verdict, not silence. Large collections page with `more`.

### 5.5 Operations and the error envelope

- **Today.**
  - Calls name their key in text.
  - Replies are the declared types (config.v1 returns the whole view, or its status when the caller asks `answer = "status"`).
  - Errors are JSON `{code, message, …}` with string codes, 60–90 bytes ([core §5](../../spec/core.md)).
- **Keep.** O1–O7; the envelope's closed code list; `unavailable` with its cause.
- **zk2c.**
  - Requests and replies in their packed layouts on a declared querier scope.
  - The error envelope is a zk2c form: CH8 (the error form and the layout tag), then code3 · cause2 · detail1 · message1 · extension1 and a message index of 8 bits, **3 bytes**.
  - The Encoding alone cannot tell it from Zenoh's own errors. A router's query timeout arrives as `zenoh/bytes` with the payload `Timeout` [Z53], and a querier's local timeout as `zenoh/string`. The form, the tag and the exact length can; and a reply whose replier is not the called owner is the transport's.
- **After.** A call round trip costs about 55 bytes. Zenoh's reply envelope is most of it: QoS and the responder's 20-byte id.
- **Risk.** Free-text messages need the dictionary escape.

### 5.6 Events

- **Today.** One put per occurrence on `…/events/<template>/<ulid>`. The ULID is 26 characters and the key cannot be declared, so W2's `oper_change` key is 77 bytes of text. With a JSON payload and a stamp, an event is about 280 bytes with framing, two SDUs on RF. The baseline's smaller event was 116 bytes.
- **zk2c.** A declared scope plus an 8-character suffix (run16 ‖ seq24), and a record of CH8 · t32 · fields: about 50 bytes per event with framing. Replay filters by the time in the payload.

### 5.7 Archive

- **Today.** Every GET reply carries a JSON attachment `{iface, contract, type, confirmed}`, about 150–200 bytes, static but for one bit ([core §4.4](../../spec/core.md)).
- **zk2c.** catalog10 · type7 · confirmed1 · extension1 · reserved5 = **3 bytes**. Alignment reads the owner's snapshot.
- **Keep.** Last-known never shown as current; no resurrection (a put older than a held delete is refused, now by run and seq); drop only on positive evidence.

### 5.8 freshness.v1

- **Today.** Every member with a horizon is re-put unchanged, with a fresh stamp, at intervals of at most ttl/2; a stream must send a new sample at least as often ([freshness.v1](../../spec/profiles/freshness/v1.md)).
- **zk2c.** **Confirmation is not mutation.** The heartbeat confirms by one bit per member, and GET and snapshot replies carry the last confirmation time.
  - Cost: 18 bytes every 30 s at a 60-s horizon, 52 KB/day for all of W2's horizon state. A 300-s horizon cuts that to 10.4 KB/day.
  - Measured (§9), freshness.v2 sends one heartbeat per interface with a collection, each its own put: W2's three, health.v1's and the config's every 28.5 s and alarms.v1's every 427.5 s, cost 238 KB a day on RF.
  - Horizons on the hop should be chosen in seconds of budget, not left at 60.
- **Risk.** An owner-wide heartbeat reveals every collection's liveness to anyone allowed to read it, wider than per-interface ACLs. Offer per-interface heartbeats, at about 11 extra bytes each.

### 5.9 hostid.v1

- **Today.** The system chunk is `h-` plus 12 hex characters, in every key; the machine id never leaves the host ([hostid.v1](../../spec/profiles/hostid/v1.md)).
- **zk2c.** Unchanged in key text, since it is declared once per scope. Collision and clone findings compare the 64-bit nonce instead of the zid.

### 5.10 link.v1

- **Today.**
  - One downsampling rule per concrete stream key; templated streams cannot cross.
  - State and events cross whole.
  - A tombstone costs about 100 bytes.
  - zenkey's face file for zenoh-modem uses `interval_s = 60` for the four link streams, and zenoh-modem's router downsamples them to one a minute ([link.v1](../../spec/profiles/link/v1.md); [Z49]).
- **zk2c.** The face policy gains:
  - the catalog id;
  - the heartbeat horizon;
  - per-medium lease, keep-alive and sequence-number width (§6.2);
  - a list of **frame resources**.

  Rather than a router downsampling member by member, the owner publishes one frame resource per face cadence, such as `frames/1m`, declared in the contract. Downsampling a delta frame would corrupt what follows. Templated members cross inside frames, and tombstones cost about 4 bytes inside snapshots.

### 5.11 telemetry.v1

- **Today.** A Point (65 bytes for a gauge) repeats its kind tag, unit and labels, and a 27-byte epoch for counters. Bare values are 2–7 bytes but still pay the 26-byte stamp and, for counters, a 20-byte Epoch attachment. To a far reader on `zk2/**`, each put also carries its key's text, 40–52 bytes for W2's streams (§5.3). Histograms repeat their bounds and `count`, which is Σ counts ([telemetry.v1](../../spec/profiles/telemetry/v1.md)).
- **Keep.**
  - Kind and unit from the declaration.
  - Reset detection: the epoch is now the run, plus a 4-bit generation for templates that declare `epoch`.
  - Decrease judged only within an epoch.
  - Measurement time distinct from publication time.
  - Absent is not zero.
- **zk2c.** A **vector frame** per group of series: frame header 80 bits, then each member at its declared width.
  - W2: 8 gauges × 11 bits + 4 counters × 32 bits = 216 bits per sample.
  - Three samples per 30-s frame cost 334 KB a day on RF; one frame per 10 s costs 536 KB.
  - Histogram bounds move to the contract and `count` is dropped.
- **W1, zenoh-modem's 73 readings.** At zenoh-modem's declared ranges (modem.v3's `Readings`: 51 counters at 20–40 bits, each for its worst rate, and 22 gauges at 1–27 bits, 1,469 bits of codes, every field optional), measured byte for byte (§9):
  - **Keyframe**: every member, absolute. 205 bytes on a 16-bit run, its put 210 bytes, so two SDUs under the lean stack. A counter pair further apart than its width's window reads `wrap_or_decrease`, never a wrong rate.
  - **Delta frame**: a 73-bit changed bitmap and the *absolute* values of the members that changed. Assuming 16 counters and 8 gauges change per minute, 84 bytes. A lost delta frame leaves the omitted members unknown, never wrong; the next keyframe heals them.
  - The first model's illustrative widths gave 254 bytes and 98 bytes, on a `data_low` frame.

  | W1 case | RF | LoRa SF7 | LoRa SF9 | LoRa SF12 |
  |---|---|---|---|---|
  | 4 link readings at 1/min, today | 334 KB/day | 650 s/day (75 % of duty) | 2,129 s ✗ | 20,856 s ✗ |
  | The same, today's uplink with its keep-alives and reopens: model (measured) | 897 KB/day (844) | 909 s ✗ (999) | 3,100 s ✗ (2,494) | 21,527 s ✗ (14,032) |
  | 4 link readings at 1/min, zk2c | 71 KB/day | 140 s | 473 s | 4,978 s ✗ (fits every 6 min) |
  | All 73 at 1/min, today | 6.52 MB/day | ✗ | ✗ | ✗ |
  | All 73, zk2c: deltas each minute, keyframe every 15 min | **168 KB/day** | 288 s | 916 s ✗ (502 s every 2 min) | hourly keyframe + 15-min deltas: 762 s |
  | All 73, zk2c, measured: a full frame of every reading, then one of none | 207 KB/day | 363 s (474 s with the uplink's keep-alives) | — | — |

  ✗ marks a case that exceeds the 864 s of air the 1 % duty cycle allows in a day. modem.v3's streams are `data_low`, so every put and frame carries the 2-byte QoS extension. Its frame resource `frames/fast` is `data`, which carries none, on a face's 3-byte frame of 16-bit sequence numbers. The first model's all-73 row was 200 KB, 325 s, 1,043 s (556 s) and 833 s. No LoRa face ships today, so the LoRa "today" rows are what the current wire would cost.
  - The readings' rows send each reading in its own SDU, the upper bound. zenoh batched some of them on the paced links. The measured readings fell between that bound and all four in one SDU, on every medium (§9).
  - The uplink row adds the keep-alives and, at LoRa's keep_alive of 2, the reopens. Each keep-alive lost on the downlink closes the link (§6.2).
  - zk2c's rows are the readings alone, but the measured one's SF7 figure in brackets.
- **Deadband.** A declared δ and maximum interval let the owner omit members that moved less than δ. The reader then knows the value to ±δ until the interval runs out, and nothing after. That is a declared loss, allowed by the brief. It is forbidden on timed and arbitrated streams (§5.17, §5.18).
- **Risk.** One key per group means ACLs apply per group; split groups by audience.

### 5.12 alarms.v1

- **Today.**
  - Each alarm is a whole JSON document (107–138 bytes) under a 16-hex key, express, reliable.
  - It is put at every transition, and again every 450 s while firing (427.5 s in the runtime).
  - Resolve is a put and then a delete ([alarms.v1](../../spec/profiles/alarms/v1.md)).
  - W2: 136 KB a day.
- **Keep.**
  - Late-joiner self-contained state.
  - A stable identity per alarm.
  - Acknowledgement identity that moves only at transitions.
  - Stale is neither firing nor resolved.
  - "Nothing fires" can be established.
  - Refuse at the bound; never evict.
- **zk2c.**
  - **Transition record** (as a mutation): slot 8 · state 1 · severity 2 · extension 1 · t32 · argument 20 = **64 bits**. The 8-bit slot covers the profile's bound of 256 alarms. The summary is the rule's template rendered with the argument.
  - **Confirmation**: one heartbeat bit per firing slot, set only while the owner still evaluates it.
  - **Binding record**: slot · rule index · label values, sent when an identity first takes a slot and repeated in every snapshot, so a late joiner can name each slot.
  - The resolved record is terminal: no delete follows.
  - Acknowledgement identity is t32 plus seq.
  - "Nothing fires" is established by one snapshot, with no 900-s window.
- **After.** About 0.4 KB a day for W2.
- **Risk.** The transition time drops to 1-s resolution, declared, with a +1 bump to stay strictly increasing. Summaries outside the templates need the escape.

### 5.13 health.v1

- **Today.** A protobuf status of about 21 bytes plus the 26-byte stamp, re-put every 30 s (28.5 s); 64 checks at most; ordered puts so that the status is never better than the worst check ([health.v1](../../spec/profiles/health/v1.md)). W2: 252 KB a day.
- **zk2c.**
  - **Status**: level 2 · reason index 6 · since t32 = **5 bytes**, sent only on change.
  - **Check**: level 2 · detail 6 · at 32.
  - **Fault**: code 6 · level 2 · argument 16 · at 32 = 7 bytes.
  - **Confirmation**: one heartbeat bit.
- **Keep.** Put order by sequence number; unknown and undecodable read as no level, never OK; `clock_ahead` carried in every heartbeat.
- **After.** About 0.2 KB a day plus the heartbeat's share.

### 5.14 config.v1

- **Today.**
  - The view (about 1,286 bytes) and the status (about 330 bytes) are JSON. The view re-sends the whole parameter schema (names, kinds, descriptions, bounds) and RFC 3339 times of 24 characters.
  - Both are re-put every 30 s. Replies return the whole view, or the status (at most 400 bytes in the scenarios) when the caller asks `answer = "status"` ([config.v1](../../spec/profiles/config/v1.md)).
  - W2: 6.0 MB a day, the largest item after telemetry.
  - The baseline's view, the owner example's with five groups, was 1,476 bytes in 8 SDUs. It crosses only when all 8 do: 0.95⁸ = 66 % of the time at 5 % loss, and 38 of 65 crossed. Zenoh resends no lost fragment, so the next re-put, 28.5 s later, is the only retry.
- **Keep.**
  - No lock-out: a reach change applies under a window and is undone at the deadline.
  - One pending change at a time.
  - Tokens never reused.
  - `(instance, revision)` names the running values.
  - Sensitive values never on the wire.
  - Exactly one change event per change.
- **zk2c.**
  - The schema moves to the contract.
  - One record replaces view and status: revision 16 · floor 16 · pending 1 · applied 1 · desired 2 · last 1 · reserved 3. Then the optional pending, last-change and desired sections, then per parameter: source 2 · startup-differs 1 · value at its declared width.
  - The status is its projection by construction.
  - A pending reply is run 16 · token 16 · apply_at 32 · deadline 32 = 12 bytes; the scenario cap today is 160.
  - The config token is the run and a 16-bit counter. Carrying the run keeps tokens unique across instances, as the profile requires: no instance mints a token another minted before. The run changes before its counter wraps.
- **After.** About 0.3 KB a day for W2. The record fits one SDU, so a loss costs one record, not a view.
- **Risk.** A schema change is now a contract revision, which it already was semantically.

### 5.15 desired.v1

- **Today.**
  - The Report is JSON. It repeats the full slice key and two text stamps (`"<NTP64>/<zid>"`, usually equal), plus the outcome and refusal strings.
  - Every author re-put (freshness, restart) costs one report per target.
  - Across a face, a target SHOULD re-read periodically. The period is the deployment's; ZenSight's is 300 s, which W2 assumes ([desired.v1](../../spec/profiles/desired/v1.md)). W2: 91 KB a day.
  - On RF that is paid for nothing. In the baseline every re-read timed out at the target's 1-s default, and the author's replies still crossed, 3.3–7.4 s later. The target never adopted its document.
- **Keep.**
  - Each version acted on once.
  - Never an older version within one id.
  - Adoption without an act.
  - Revert only on a delete, never on silence.
  - Refusal whole.
  - Last-known flagged.
- **zk2c.**
  - The version is the author's (run, seq).
  - **Report**: role 4 · source 2 · outcome 3 · last-known 1 · reserved 6, then in_force 40 · observed 40 · revision 16 · refusal code 3 · pointer index 8 · detail index 8 · reserved 5 = **17 bytes**.
  - The re-read becomes a **conditional GET** carrying the held version; an unchanged slice answers in about 35 bytes, 20 of them the reply's responder id.
- **After.** 27 KB a day for W2. It falls to nearly zero where the author's heartbeat crosses the face, since the target then reads only when the author's high-water seq moves. Measured (§9), the reference's re-read is still today's GET, its reply the whole document: 64 KB a day.

### 5.16 redundancy.v1

- **Today.** Claims are tokens; the standby mark lives in the descriptor, which is re-put whole at take-over and step-down. Elections read `@zk/**` plus lower ids' descriptors, with a retry and grace of 2 s. Takeover re-puts every held state value, and the HLC is mandatory ([redundancy.v1](../../spec/profiles/redundancy/v1.md)).
- **zk2c.**
  - The standby flag is 1 bit in the compact descriptor.
  - Runs must be unique per candidate: 32 bits random, or a 4-bit candidate index and a 12-bit counter.
  - Consumers fence by the incumbent's run, not its zid.
  - A takeover publishes one snapshot instead of re-putting every value.
- **Risk.** Elections need presence, so candidates stay on one side of a constrained face, as today.

### 5.17 timing.v1

- **Today.** `period_ms`, `deadline_ms` and `lifespan_ms` live in the contract. A lifespan forces a stamp on every sample, so 26 bytes per sample ([timing.v1](../../spec/profiles/timing/v1.md)).
- **zk2c.**
  - The period dates frame records, which is the implied time of §4.4.
  - A lifespan resource carries t32 · ms10 per sample, or a 10-bit millisecond offset inside a frame.
  - Deadline and period stay judged on the receiver's clock.
- **Forbidden.** Deadband and multi-sample frames on a resource with a deadline, unless the deadline is declared as the maximum interval.

### 5.18 arbitration.v1

- **Today.** No wire of its own. Each provider is a stream with a deadline, the source is read from the key, and the spike's commanders ran at 50 Hz ([arbitration.v1](../../spec/profiles/arbitration/v1.md)).
- **zk2c.** Each sample is its own mutation, with no frames and no deadband. `not_incumbent` reads the run. At 50 Hz the mutation header is the dominant term after the payload: cost a command loop over a constrained hop as packed payload + 6 bytes of header (8 with a 32-bit run) + 4 bytes of put per sample, plus framing.

## 6. Cross-cutting decisions

### 6.1 One heartbeat per owner and horizon

The heartbeat replaces five separate periodic re-sends (§4.4). Its period is half the shortest horizon the owner declares, so **horizons become budget parameters**. At 18 bytes, a 60-s horizon costs 52 KB a day and a 300-s horizon 10.4 KB a day. Express it per interface where ACLs differ.

freshness.v2 took the per-interface form: one heartbeat per interface with a collection, each its own put on its own key. Measured on RF (§9), W2's three cost 238 KB a day, between every pair in one SDU and each in its own, where one per owner at 18 B would cost 52 KB.

### 6.2 Lease and keep-alive per medium

zenoh-modem's measurements already show that keep-alive, not payload, is the idle bill [Z12][Z16]. Data and heartbeats refresh the lease, so an owner publishing more often than the keep-alive interval sends none uplink.

| Medium | Setting | Keep-alives/day, both directions |
|---|---|---:|
| Zenoh defaults (lease 10 s, keep_alive 4) | every 2.5 s when idle | 69,120 |
| RF, proposed | lease 240 s, keep_alive 4: every 60 s | 2,880 (uplink: 0 while data flows at least once a minute) |
| LoRa SF7 (zenoh-modem's guidance) | lease 60 s, keep_alive 2 | 5,760 |
| LoRa SF12 (zenoh-modem's guidance) | lease 540 s, keep_alive 2 | 640 |
| SBD (zenoh-modem's tuned) | lease 3,600 s, keep_alive 2 | 96 |

Notes:
- `keep_alive = 1` flaps [Z12]. At 5 % loss, 2 flaps too: one lost keep-alive leaves a lease of silence and closes the link. On LoRa, 9 of 9 did (baseline, LoRa finding 1).
- A reopen costs more air than the keep-alives it saves. Idle at SF7, keep_alive 2 costs 281 s a day each way with its 288 reopens; 250–270 s were measured. Keep_alive 4 costs 296.4 s, its keep-alives' figure to the tenth, measured and modeled, and reopens less than once a day.
- At the same lease, 4 doubles the bill [Z12]. At twice the lease it keeps the count, and the link closes only after three losses in a row. Hence RF's proposed 240 s.
- `wait_before_close` must be at least the lease [Z12].
- A longer lease detects a dead link later, which the heartbeat's horizon must exceed.
- At SF12 a keep-alive every 30 s costs 178 s of air an hour with the lean stack, five times the 36-s budget. zenoh-modem measured 124 s without aead [Z16].

### 6.3 Presence, descriptor and bundles on the hop

- Only the instance token and the compact descriptor cross the first hop.
- Bundles and the catalog are provisioned, never fetched over the radio.
- Backhaul faces keep denying `@zk`, `@adv` and the admin space, as link.v1 does.

### 6.4 Express only for urgent acts

An express message is flushed at once with whatever its batch already holds, so it never waits for later messages to join it [Z9][Z54]. The contract marks which acts are urgent: alarm transitions, config control replies, commands. Heartbeats, telemetry and confirmations go in ordinary batches, so they share SDUs.

### 6.5 Sequence-number width

Zenoh masks its default 32-bit sequence number to 28 bits, so it is at most a 4-byte varint per frame [Z55], as the baseline measured. At a 16-bit resolution it is at most 2 bytes. On RF at one SDU per 10 s that saves about 17 KB a day; it must stay wide enough for the link's reordering window.

### 6.6 ACL granularity

Frames, snapshots and heartbeats aggregate members under fewer keys. A principal allowed one member of a frame sees the frame. Group members by audience in the contract, and keep per-interface heartbeats where interfaces have different readers.

### 6.7 SBD economics

On SBD the message is the unit:
- Tuned, the link sends about 96 keep-alive messages a day of roughly 1 byte each in 270-byte slots [Z12].
- A data message refreshes the lease, so an owner that sends a frame instead of letting a keep-alive fall due spends no extra message.
- The 48 uplink messages a day that are already paid carry 12,960 bytes.
- One keyframe of all 73 modem readings with 24-bit counters (230 bytes with frame, QoS and aead) fits one message; with 32-bit counters it is 281 bytes and needs two.
- Under a byte-billed plan, the same design minimizes bytes; bit packing then matters directly, not only through records per message.

## 7. Where compression would break a guarantee

1. **Lifespan streams with a truncated time.** A time window shorter than the maximum outage plus transit lets an old sample read as live (timing.v1).
2. **Deltas or XOR across frames.** A lost frame corrupts every later value and breaks counter continuity. zk2c sends absolute values in every frame; delta frames carry absolute values of changed members only.
3. **Random runs of 16 bits or fewer.** A collision continues an old epoch: values look older and resets read as findings. Use a persistent counter or 32 random bits, unique per candidate under redundancy.v1.
4. **A heartbeat without per-member bits.** It would turn stale values fresh.
5. **Deadband, downsampling or multi-sample frames on timed or arbitrated streams.** Deadlines miss, and providers become ineligible.
6. **A truncated sequence number, or a partial snapshot read as complete.** Both give false "nothing fires" and "not held" verdicts. The `more` bit and the full 24-bit high-water seq exist for this.
7. **Removing the HLC without replacing what read it.** `Latest` consolidation and Zenoh's storage manager no longer order zk2c keys. Storages must sit where they order by the in-band stamp.
8. **Decoding with the wrong catalog or revision.** It corrupts values silently unless the layout tag and the exact length are checked.
9. **Aggregate keys and owner-wide heartbeats.** They widen what an ACL exposes (§6.6).
10. **Text without a dictionary or escape.** The human information is lost.

## 8. Priorities and prototypes

| Priority | Work | Evidence of done |
|---|---|---|
| **P0** | Run W1 and W2 today and as zk2c on zenoh-modem's fake RF, LoRa and SBD media. Today on RF and LoRa, and zk2c on RF and LoRa SF7, are done (§9) | Measured bytes, SDUs, airtime and billed messages per day in both directions, against Appendix A's model |
| **P0** | Apply the per-medium lease and keep-alive settings of §6.2 | Keep-alive counts per day match the table; no flapping |
| **P1** | A zk2c layout generator in zenkey-build: widths from bounds, scale/offset, reserved codes, extension bit, layout tag; Rust and Python codecs with shared vectors | Cross-language test vectors; refusal of a wrong revision |
| **P1** | Heartbeat and snapshot in the runtime; freshness as confirmation | Stale never fresh; loss detected by the high-water seq; seed boundary exact |
| **P1** | The run/seq owner stamp; timestamping off on the hop | S1–S7 scenarios pass with in-band order; clock-ahead no longer blocks |
| **P2** | Profile texts v2: telemetry frames and deadband, alarms transitions and slots, config record, desired report and conditional GET, health record, descriptor | Each profile's guarantees in §5 kept, checked by its scenarios |
| **P2** | link.v1 v2: frame resources, catalog, heartbeat horizon, per-medium transport settings | `check face` covers the new fields |

## 9. Measurement plan

**Today's half is done, and zk2c's on RF.** The wire baseline, [`measurements/wire-baseline/`](measurements/wire-baseline/README.md), measured today's wire end to end, in both directions, under the lean stack:
- W1 and the idle link on RF, and on LoRa at SF7, SF9 and SF12;
- W2 on RF.

It then measured zk2c the same way (its "zk2c on RF, and W1-73 at LoRa SF7"), with zenkey's reference as the owners, at link.v2's transport:
- the idle link: 1,440 keep-alives a day each way, the model's figure exactly;
- W1 with all 73 modem readings, across rf0's face and across lora0's SF7 face, `zenctl gen` as modem.v3's owner;
- W2 on RF: **669 KB a day, 2.6 % of the channel, 25 times less than today's 16.5 MB**;
- the four guarantees, through the tools across rf0's face.

Appendix A is corrected to it:
- a declared key scope is 1 byte, and a frame 5 bytes, or 7 on a priority other than `data`;
- a stream's put carries its key as text, which doubles W2's telemetry;
- a GET's reply carries its key, QoS and a 20-byte responder id;
- W1's four link readings are 2-byte values, as gen's measured ones were: a queue depth, a 0/1 flag, a wait in ms that is 0 while the medium is free, and a drop count. All 73 readings keep 6 bytes;
- a reconnect re-sends the handshake, the admin space and every declaration;
- on LoRa at keep_alive 2, each lost keep-alive is a reconnect.

Where the model meets the measurements, Appendix A asserts the tolerance:
- **W2 on RF.** Telemetry, config, health and the downlink's keep-alives are within 4 % each. Telemetry is compared at the 81.5 % of its puts that reached the air. Alarms are within 22 %, since the model's document is larger, and the uplink's keep-alives within 13 %. Both directions together are within 1 % of the 16.5 MB that crossed.
- **W1.** Keep-alives are exact wherever the link held. The model sends each reading in its own SDU, since batching depends on when the owner puts; gen put them 140 ms apart. The measured readings fall between that bound and all four in one SDU, on every medium. On RF the measured batching, 3.1 SDUs a tick, and compression account for all but 1 %.
- **LoRa's reopens.** Each window's count is within one of the model's rate. Each reopen's air is within 45 % of the model's: below it where the trained dictionary compressed the declarations, above it where an open was retried.

- **zk2c.** Appendix A's section 10 asserts the measured anatomy byte for byte: the face's 3-byte frame, the envelope, the put, each SDU. W2's telemetry is the model's exactly. W2 as a whole is within 1 % of the lean run, once the model takes each heartbeat as its own put, the desired re-read as today's GET and a reconnect's subscriptions as the far side's. W1-73's frames are within 2 % on RF and 1 % of air at SF7. The first zk2c model said 443 KB for W2 and 200 KB for W1-73.

The baseline README's model column is Appendix A before these corrections. Its LoRa W1 figures add the uplink's keep-alives to the readings: 754 s at SF7 and 2,485 s at SF9. This report then quoted the readings alone, 680 s and 2,247 s. A duty budget pays for the uplink's whole air, so the README's were the right ones. The corrected Appendix A counts the reopens too.

**What the model cannot price:**
- **Router A's queue.** Its `drop` congestion control discarded 19 % of W2's telemetry before the air, where each tick's burst met the config view's 1.6 KB. The model has no queue. It prices what the owner sends, and the drop is a gap of today's wire.
- **A link that does not come back.** On RF, once a transport under W2's load closed, no new one carried data again, in 4 runs of 4. The cause is not established. A reconnect in the model is a link that comes back.
- **A declaration lost while the link comes up.** It is never repaired [Z20]. The far router then drops every put on that scope, or no GET reaches the queryable. The costing runs set up with no loss.
- **Replies after their timeout.** Every desired re-read on RF timed out at the target's 1-s default, and its reply still crossed, 3.3–7.4 s later. Under zk2c's lighter load, 4 of 12 still did. The model counts bytes that buy nothing there.
- **A value of several SDUs.** It crosses only when every SDU does, since zenoh resends no fragment.

**What remains:**
- SBD, which needs an in-process harness in zenoh-modem;
- W2 on LoRa, and across a face;
- zk2c at LoRa SF9 and SF12, and with zenoh-modem's own owner, modem-mgmt, once it is ported;
- a "stale" or "unconfirmed" from the tools across a face, where today they fall silent or unobservable once a horizon passes (#990).

For each remaining run:
- Measure both directions: bytes, SDUs, airtime and billed messages per day.
- Separate information bits, transmitted payload bits and billing units.
- Include opening, steady state, a lost frame, a coverage gap, a restart (new run) and a late joiner (snapshot).
- Record the layer stack, lease, keep-alive, sequence-number width, batch size and duty budget.
- For every compact form, record whether the profile's guarantees held: stale never fresh, loss detected, no false "nothing fires", reset detected.

## 10. Decisions requested

1. **Adopt zk2c as zk2's one wire, on every bus.** Core mechanics and every profile's records are packed from declared ranges and resolutions, carried as `zenoh/bytes` with a layout tag. Application payloads may stay JSON or protobuf on a host bus, and a lint requires `packed` on constrained faces.
2. **Replace the per-put HLC with the run/seq owner stamp,** carrying time only where a profile's semantics need it.
3. **Make confirmation a heartbeat, not a re-put,** and treat horizons as budget parameters.
4. **Move static context into the contract and a fingerprinted catalog,** provisioned, never fetched over the radio.
5. **Set lease and keep-alive per medium** in link.v1 v2.
6. **Allow declared bounded loss** (resolution, deadband with a maximum interval) except on timed and arbitrated streams.

## Appendix A. Reproduce every figure

Run with Python 3 (`python3 -I`); it needs no third-party package and asserts the figures the text quotes. Today's wire is the model corrected to the wire baseline ([`measurements/wire-baseline/`](measurements/wire-baseline/README.md)). Each correction cites the finding and the run it rests on. The measured figures are constants in the code, and each tolerance is an assertion against them. The rest of the Zenoh model follows the 1.10.1 codec layouts read for this review, the layer sizes follow zenoh-modem's READMEs, and the airtime follows the Semtech formula with zenoh-modem's simulator parameters. Gauge widths for W1 and the per-minute change counts are illustrative assumptions, marked in the code. The baseline's `model.py` runs this script; its committed `results/model.json` is from before the correction. Section 10 checks the zk2c column against the baseline's zk2c runs (its "zk2c on RF, and W1-73 at LoRa SF7") and corrects it where the model, not the wire, missed: W1's 73 readings at zenoh-modem's ranges, and W2's heartbeats, desired re-reads and reconnects. The text quotes the corrected figures and the measured ones; section 6's zk2c rows stay the first model, and `model.py --zk2c` writes `results/model-zk2c.json` from them.

```python
"""Reproduce every figure of the bandwidth-first redesign report. Standard library only.

Today's wire is the model corrected to measurements/wire-baseline/README.md (#989), cited as
"baseline" with its section, finding and run. zk2c stays a model.
"""
import json
import math

DAY = 86_400


# 1. Bits from declared range and resolution -------------------------------------------
def width(lo, hi, step=1, reserved=0):
    """Fixed width of a bounded field: ceil(log2 of its number of codes)."""
    return math.ceil(math.log2(round((hi - lo) / step) + 1 + reserved))


def B(bits):
    return math.ceil(bits / 8)


assert width(0, 1000) == 10
assert width(-40, 85, 0.1) == 11 and width(-40, 85, 0.1, reserved=2) == 11
# A 16-bit word shared by four fields: duty available 0..100.0 % at 0.1, signal bars 0..5,
# oper status (up, down, notPresent, unknown) and admin (up, down).
word = [width(0, 100, 0.1), width(0, 5), width(0, 3), width(0, 1)]
assert word == [10, 3, 2, 1] and sum(word) == 16
assert math.ceil(64 * math.log2(3)) == 102            # mixed radix: 64 three-level severities


# 2. Zenoh 1.10.1 message model, as the baseline decoded it ------------------------------
def vle(x):
    """Bytes of Zenoh's LEB128-style varint (9 at most)."""
    return min(9, 1 + (max(x, 1).bit_length() - 1) // 7)


NTP64_2026 = (1_793_000_000 << 32)            # seconds in the upper 32 bits
HLC = vle(NTP64_2026) + vle(16) + 16           # time, id length, 16-byte zid
assert HLC == 26                               # exact: baseline, RF W1 finding 3
QOS = 2                                        # QoS extension when priority, congestion or
                                               # express is not the default
SCOPE = 1        # a declared key's scope id. The model had 2. Baseline, RF W1 finding 3 (bare-1)


def put(payload, scope=SCOPE, suffix=0, hlc=False, enc=0, attach=0, qos=False):
    """PUSH header, key scope, optional suffix, QoS extension, PUT header, stamp, Encoding,
    attachment, payload length, payload."""
    n = 1 + scope + (1 + suffix if suffix else 0) + (QOS if qos else 0) + 1
    n += HLC if hlc else 0
    n += enc
    n += (1 + vle(attach) + attach) if attach else 0
    return n + vle(payload) + payload


assert put(2, hlc=True, enc=1, qos=True) == 35        # W1's measured put (RF W1 finding 3)
# A frame is a header and a sequence number. Zenoh 1.10.1 masks its default 32-bit resolution
# to 28 bits, so the number is at most a 4-byte varint [Z55]; the model gave it 5. Baseline, RF W1
# finding 4 (bare-1). A frame adds the QoS extension only on a priority other than `data`:
# health's and config's frames were 5 B, alarms' and modem.v3's 7 B (bare-1, w2-rf-bare-2).
FRAME = 1 + 4
FIRST_FRAG = 1       # the first fragment of a message carries a 1-byte extension
                     # (w2-rf-bare-2: 42 of its 210 fragment headers were 6 B, one a message)
KEEPALIVE = 1        # one transport message, one SDU


def keyexpr(key):
    """A key expression's declaration: DECLARE header and QoS (3), D_KEYEXPR and its id (2),
    then the whole key as text (bare-1, w2-rf-bare-2)."""
    return 3 + 2 + 1 + vle(len(key)) + len(key)


# A queryable, token or subscriber on a declared expression, and a queryable on its "/**".
QUERYABLE, TOKEN, SUBSCRIBER, SUBTREE = 8, 6, 6, 10
assert keyexpr("zk2/h-3fa9c2d41b7e/rf0/health.v1/state") == 45
# A link opens with a handshake of 83 B each way in 2 SDUs (InitSyn and OpenSyn; InitAck and
# OpenAck). Then each router declares its admin space: 124 B of messages, one SDU each way.
# Baseline, RF idle and W1 (bare-1).
HANDSHAKE = {"ab": [27, 56], "ba": [77, 6]}
ADMIN = 124
# A GET and its reply, as the desired target's re-reads crossed (w2-rf-bare-3). The request
# rides a declared scope: header, id, scope, QoS, target, timeout, then the query with its
# consolidation. The reply names its key: a scope, then the text where nothing was declared.
# It carries QoS and the responder's id (20 B). RESPONSE_FINAL closes the GET.
RESPONDER = 1 + 1 + 18
FINAL = 1 + 1 + QOS


def request(scope=SCOPE, timeout_ms=1000):
    return 1 + 1 + scope + QOS + 2 + 1 + vle(timeout_ms) + 2


def reply(payload, key=0, hlc=True, enc=1):
    wire = 1 + (vle(key) + key if key else 0)
    return 1 + 1 + wire + QOS + RESPONDER + 1 + 1 + (HLC if hlc else 0) + enc + vle(payload) + payload


def reply_del(key):
    return 1 + 1 + 1 + vle(key) + key + QOS + RESPONDER + 1 + 1 + HLC


TASKS_KEY = len("zk2/fleet/desired/tasking.v1/state/tasks/h-3fa9c2d41b7e/rf0")
assert (request(), reply(44, TASKS_KEY), FINAL) == (12, 159, 4)     # w2-rf-bare-3, exact


# 3. zenoh-modem layers and media ---------------------------------------------------------
# Lean stack: compress (1, its "stored" marker), aead with an 8-byte CCM tag (1 + 4 + 8),
# address (2). Full stack: fec (5) and a 16-byte tag, plus one repair SDU per 4.
LEAN = 1 + (1 + 4 + 8) + 2
FULL = 1 + (1 + 4 + 16) + 5 + 2
assert (LEAN, FULL) == (16, 29)


def sdu_list(messages, mtu, layers=LEAN, qos=False):
    """Each SDU's size when messages are packed into batches of (mtu - layers). Zenoh
    fragments a message that does not fit; each fragment carries the frame's header. qos: a
    priority other than `data`, whose frame carries the QoS extension."""
    frame = FRAME + (QOS if qos else 0)
    cap = mtu - layers - frame
    out, used = [], None
    for m in messages:
        if m > cap:
            rest = m + FIRST_FRAG
            while rest > 0:
                out.append(min(cap, rest) + frame + layers)
                rest -= cap
            used = None
            continue
        if used is None or used + m > cap:
            out.append(frame + layers)
            used = 0
        out[-1] += m
        used += m
    return out


def sdus(messages, mtu, layers=LEAN, qos=False):
    """(number of SDUs, bytes on the medium)."""
    s = sdu_list(messages, mtu, layers, qos)
    return len(s), sum(s)


RF_BPS, RF_MTU = 2400, 220
RF_DAY_BYTES = RF_BPS * DAY // 8
assert RF_DAY_BYTES == 25_920_000


def lora_toa_s(pl, sf, bw=125_000, cr=1, preamble=8, crc=1, ih=0):
    """Semtech SX127x/SX126x time on air, zenoh-modem's simulator parameters."""
    de = 1 if sf >= 11 else 0
    tsym = 2**sf / bw
    steps = math.ceil((8 * pl - 4 * sf + 28 + 16 * crc - 20 * ih) / (4 * (sf - 2 * de)))
    return (preamble + 4.25 + 8 + max(steps * (cr + 4), 0)) * tsym


assert round(lora_toa_s(0, 12) * 1000) == 664                     # SF12 empty frame
assert [round(lora_toa_s(51, sf), 2) for sf in (10, 11, 12)] == [0.62, 1.31, 2.47]
LORA_DUTY_S = 36 * 24                                              # 1 % of every hour
assert LORA_DUTY_S == 864


def lora_air_s(sdu_bytes, sf):
    """Airtime of one SDU: whole frames up to SF9, fragment tiles of 51 B at SF10-12."""
    if sf <= 9:
        return lora_toa_s(sdu_bytes, sf)
    tile_body = 51 - 2                                    # rule + fcn per tile
    tiles = math.ceil((sdu_bytes + 4) / tile_body)        # + 4-byte RCS on the last
    last = sdu_bytes + 4 - (tiles - 1) * tile_body + 2
    return (tiles - 1) * lora_toa_s(51, sf) + lora_toa_s(last, sf)


SBD_SLOT = 270                                            # what crosses each way
LOSS = 0.05                                               # RF's and LoRa's, per SDU


def reopens_a_day(keepalives_a_day, keep_alive, loss=LOSS):
    """A transport closes after a lease of silence. On a direction that only keep-alives
    refresh, that is keep_alive - 1 of them lost in a row. Baseline, LoRa finding 1: at
    keep_alive 2, 9 of 9 lost keep-alives closed the link; at 4, none of 3 did (sf7ka4-lean-1);
    on RF at 4, three in a row closed it (RF W2 finding 5)."""
    return keepalives_a_day * loss ** (keep_alive - 1)


# 4. Today's payloads and keys -----------------------------------------------------------
def jlen(o):
    return len(json.dumps(o, separators=(",", ":")).encode())


T = 1_793_000_000_000
EPOCH_ATT = jlen("0123456789abcdef")
HEALTH_STATUS = 2 + (2 + len("serving")) + (1 + vle(1_793_000_000 * 10**9))
ALARM_REFRESH = jlen({"timestamp": T, "rule": "hot", "severity": "warning", "state": "firing",
                      "summary": "Temperature high", "observed_at_ms": T + 1000})
assert (EPOCH_ATT, HEALTH_STATUS, ALARM_REFRESH) == (18, 21, 138)
CONFIG_VIEW, CONFIG_STATUS = 1286, 330                    # config.v1's own figures
DESIRED_KEY = len("zk2/fleet/desired/zs.desired.v1/state/desired/h-3fa9c2d41b7e/sysinfo/thresholds")
EVENT_KEY = len("zk2/h-3fa9c2d41b7e/rf0/modem.v3/events/oper_change/01j9x3k7q2m4n5p6r7s8t9v0wx")
OPER_CHANGE = jlen({"device": "rf0", "event_id": "01j9x3k7q2m4n5p6r7s8t9v0wx", "at_ms": T,
                    "status": "down", "reason": "link lost", "occurrences": 1})
ALARM_KEY = len("zk2/h-3fa9c2d41b7e/rf0/alarms.v1/state/alarms/0123456789abcdef")
assert (DESIRED_KEY, EVENT_KEY, ALARM_KEY) == (79, 77, 62)
# Which key a put names. A far reader on `zk2/**` declares only `zk2`. A stream has no
# queryable, so router A declares nothing under it, and each put carries the rest of its key
# as text (baseline, RF W2 finding 2, w2-rf-bare-3). Health's and alarms' puts ride the
# `.../state` expression router A declares with their queryable. Config's view and status were
# put before that declaration, and carried their key's text all run (w2-rf-bare-2 and -3).
DEV = "/h-3fa9c2d41b7e/rf0"
GAUGES = ("temperature", "humidity", "pressure", "battery", "rssi", "snr", "cpu_load", "mem_free")
COUNTERS = ("tx_frames_total", "rx_frames_total", "errors_total", "resets_total")
STREAM = [len(f"{DEV}/gendev.v1/stream/{name}") for name in GAUGES + COUNTERS]
CONFIG_KEYS = (len(f"{DEV}/config.v1/state/view"), len(f"{DEV}/config.v1/state/status"))
HEALTH_SUFFIX, ALARM_SUFFIX = len("/status"), len("/alarms/0123456789abcdef")
assert (min(STREAM), max(STREAM), CONFIG_KEYS, HEALTH_SUFFIX, ALARM_SUFFIX) == (40, 52, (40, 42), 7, 24)

# 5. Compact layouts (bits), as the report's tables state them ----------------------------
MUT_HDR = 8 + 16 + 24                     # CH, run, seq
FRAME_HDR = 8 + 16 + 24 + 32              # CH, run, seq, t (s since the deployment epoch)
# Heartbeat section inside a telemetry frame: resource map; one bit each for health status,
# config record and desired report; the firing-alarm set as a list (selector, count, 3 slots);
# a 24-bit high-water seq for 4 collections.
HB_EXTRA = 8 + 3 + (1 + 8 + 3 * 8) + 4 * 24
HEALTH_REC = 2 + 6 + 32                   # level, reason index, since
FAULT_REC = 6 + 2 + 16 + 32               # code index, level, argument, at
ALARM_TRANSITION = 8 + 1 + 2 + 1 + 32 + 20    # slot, state, severity, ext, t, argument
BINDING = 8 + 8                               # rule index, label value index
SNAPSHOT_ALARMS = (8 + 16 + 24 + 32) + 8 + 8 \
    + 3 * (8 + 24 + ALARM_TRANSITION + BINDING) + 2 * (8 + 24) + 1
DESIRED_REPORT = (4 + 2 + 3 + 1 + 6) + 40 + 40 + 16 + (3 + 8 + 8 + 5)
CONFIG_HEAD = 16 + 16 + 1 + 1 + 2 + 1 + 3     # revision, floor, P, A, D, L, reserved
CONFIG_PARAM = 2 + 1                          # source, startup-differs, then the value
PENDING_REPLY = 16 + 16 + 32 + 32             # run, token, apply_at, deadline
ERROR_ENV = 8 + (3 + 2 + 1 + 1 + 1) + 8       # CH (error form), code, cause, detail?, msg?, ext, msg index
ARCHIVE_ATT = 10 + 7 + 1 + 1 + 5
DESCRIPTOR = (8 + 16 + 64 + 32 + 4 + 1 + 3) + 4 * (10 + 6 + 1 + 1 + 1) + (16 + 12 + 4)
assert (MUT_HDR, FRAME_HDR, HB_EXTRA) == (48, 80, 140)
assert (HEALTH_REC, FAULT_REC, ALARM_TRANSITION, SNAPSHOT_ALARMS) == (40, 56, 64, 497)
assert (DESIRED_REPORT, PENDING_REPLY, ERROR_ENV, ARCHIVE_ATT, DESCRIPTOR) == (136, 96, 24, 24, 236)
assert [B(x) for x in (HB_EXTRA, SNAPSHOT_ALARMS, DESIRED_REPORT, PENDING_REPLY, ERROR_ENV,
                       ARCHIVE_ATT, DESCRIPTOR)] == [18, 63, 17, 12, 3, 3, 30]

# Today's S4 answer for 3 live alarms and 2 tombstones: a reply per key (its text, a stamp,
# the document), a reply_del per tombstone, then the final.
assert 3 * reply(ALARM_REFRESH, ALARM_KEY) + 2 * reply_del(ALARM_KEY) + FINAL == 1007
# A zk2c operation round trip on a declared querier scope: the request and its 4-byte record,
# the reply's 6-byte record with no stamp, the final. Zenoh's reply envelope is most of it.
assert (request() + 1 + 4) + reply(6, hlc=False, enc=0) + FINAL == 55

# 6. W2: the generic device on RF, bytes per day on the hop (both directions) -------------
rows, ab, ba = {}, {}, {}        # rows: (today, zk2c, variant); ab, ba: today by sender
cycles = DAY // 10
# Telemetry: 8 gauges + 4 counters every 10 s, stream defaults (no QoS extension), each put
# with its key's text. A gauge's put is 77-85 B and a counter's 108-111 B, exactly as the
# w2-rf-bare-2 and -3 captures decode; the README's 113 B counts the 5-byte frame a counter opens.
g_puts = [put(5, suffix=s, hlc=True, enc=1) for s in STREAM[:8]]
c_puts = [put(7, suffix=s, hlc=True, enc=1, attach=EPOCH_ATT) for s in STREAM[8:]]
assert (min(g_puts), max(g_puts), min(c_puts), max(c_puts)) == (77, 85, 108, 111)
frame10 = put(B(FRAME_HDR + 8 * 11 + 4 * 32))
frame30 = put(B(FRAME_HDR + 3 * (8 * 11 + 4 * 32)))
ab["telemetry"] = sdus(g_puts + c_puts, RF_MTU)[1] * cycles
rows["telemetry"] = (ab["telemetry"], sdus([frame30], RF_MTU)[1] * (DAY // 30),
                     sdus([frame10], RF_MTU)[1] * cycles)
# Health status re-put every 28.5 s (state: reliable, block, priority data), own SDU; 4 changes
# a day after.
hp = DAY / 28.5
ab["health"] = sdus([put(HEALTH_STATUS, suffix=HEALTH_SUFFIX, hlc=True, enc=1, qos=True)],
                    RF_MTU)[1] * hp
rows["health"] = (ab["health"], 4 * sdus([put(B(MUT_HDR + HEALTH_REC), qos=True)], RF_MTU)[1],
                  None)
# Alarms: 3 firing, each confirmed every 427.5 s (express, block, interactive_high); 10
# transitions a day.
ap = 3 * DAY / 427.5
ab["alarms"] = sdus([put(ALARM_REFRESH, suffix=ALARM_SUFFIX, hlc=True, enc=1, qos=True)],
                    RF_MTU, qos=True)[1] * (ap + 10)
rows["alarms"] = (ab["alarms"],
                  10 * sdus([put(B(MUT_HDR + ALARM_TRANSITION), qos=True)], RF_MTU, qos=True)[1],
                  None)


def config_reput(view, status):
    """One re-put of the view and the status, each with its key's text, in fragments."""
    return sum(sdus([put(n, suffix=k, hlc=True, enc=1, qos=True)], RF_MTU)[1]
               for n, k in ((view, CONFIG_KEYS[0]), (status, CONFIG_KEYS[1])))


# Config: view and status re-put every 28.5 s; 5 changes a day after.
ab["config"] = config_reput(CONFIG_VIEW, CONFIG_STATUS) * hp
rows["config"] = (ab["config"],
                  5 * sdus([put(B(MUT_HDR + CONFIG_HEAD + 10 * (CONFIG_PARAM + 12)), qos=True)],
                           RF_MTU, qos=True)[1],
                  None)
# Desired target: a GET every 300 s (W2's assumption) on the expression its subscription to the
# slice declared; the reply names the key in text, with a stamp and a 100-byte JSON document.
# zk2c: a conditional GET carrying the held version; an unchanged slice answers a 3-byte record.
req, rep = request(), reply(100, DESIRED_KEY) + FINAL
creq, crep = request() + 1 + 5, reply(3, hlc=False, enc=0) + FINAL
ab["desired"], ba["desired"] = sdus([req], RF_MTU)[1] * 288, sdus([rep], RF_MTU)[1] * 288
rows["desired"] = (ab["desired"] + ba["desired"],
                   (sdus([creq], RF_MTU)[1] + sdus([crep], RF_MTU)[1]) * 288, None)
# Events: 15 a day on ULID keys (undeclarable; reliable, block), their text after `zk2`,
# versus a declared scope + 8-character suffix.
ev_today = put(OPER_CHANGE, suffix=EVENT_KEY - len("zk2"), hlc=True, enc=1, qos=True)
ev_compact = put(B(8 + 32 + 4 * 8), suffix=8, qos=True)
ab["events"] = 15 * sdus([ev_today], RF_MTU)[1]
rows["events"] = (ab["events"], 15 * sdus([ev_compact], RF_MTU)[1], None)
# Keep-alives: lease 10 s / keep_alive 4 -> one per idle 2.5 s each way; the uplink is busy
# every 10 s, so 3 per 10 s; the downlink is idle. zk2c: one per idle 60 s (lease 240 s with
# keep_alive 4, §6.2); the uplink, busy every 30 s, sends none.
ab["keep-alive"] = 3 * cycles * (KEEPALIVE + LEAN)
ba["keep-alive"] = (DAY / 2.5) * (KEEPALIVE + LEAN)
rows["keep-alive"] = (ab["keep-alive"] + ba["keep-alive"], (DAY // 60) * (KEEPALIVE + LEAN), None)
# Reconnects: 4 a day, the idle downlink's 34,560 keep-alives at 5 % loss, three in a row
# lost (RF W2 finding 5). Each re-sends the handshake, the admin space and every declaration.
# A->B, the device's (w2-rf-bare-3): a key expression and its queryable per state and
# operation, the instance with its token, a bundle queryable and an interface token per
# interface, and the subscription to its desired slice. B->A, the reader's `zk2/**` and the
# desired author's. zk2c: the handshake and its own declarations, the admin space denied (§6.3).
INST, FP = "0123456789abcdef", "0" * 64
IFACES = ("gendev.v1", "health.v1", "config.v1", "alarms.v1", "worker.v1")


def presence(base, ifaces):
    return ([keyexpr(f"zk2{base}/@zk/instance/{INST}") + QUERYABLE + TOKEN]
            + [keyexpr(f"zk2/@zk/contract/{i}/{FP}") + QUERYABLE for i in ifaces]
            + [keyexpr(f"zk2{base}/@zk/alive/{i}/{INST}/{FP[:16]}") + TOKEN for i in ifaces])


DEVICE = (presence(DEV, IFACES)
          + [keyexpr(f"zk2{DEV}/{i}/state") + SUBTREE for i in IFACES[1:]]
          + [keyexpr(f"zk2{DEV}/config.v1/@op/{op}") + QUERYABLE
             for op in ("set", "confirm", "cancel", "extend", "persist")]
          + [keyexpr(f"zk2/fleet/desired/tasking.v1/state/tasks{DEV}") + SUBSCRIBER])
FAR = ([keyexpr("zk2") + SUBTREE, keyexpr("zk2/fleet/desired/tasking.v1/state") + SUBTREE]
       + presence("/fleet/desired", ("tasking.v1",)))


def reopen(direction, declarations, mtu=RF_MTU, layers=LEAN, admin=True):
    """One reopen's SDUs in one direction: the handshake, the admin space, the declarations."""
    return ([x + layers for x in HANDSHAKE[direction]]
            + (sdu_list([ADMIN], mtu, layers, qos=True) if admin else [])
            + sdu_list(declarations, mtu, layers, qos=True))


RECONNECTS = round(reopens_a_day(DAY / 2.5, 4))
assert RECONNECTS == 4
ab["reconnect"] = RECONNECTS * sum(reopen("ab", DEVICE))
ba["reconnect"] = RECONNECTS * sum(reopen("ba", FAR))
zk2c_decl = [3 + 4 + 55, 6 * (3 + 4 + 60), 3 * 15, put(25)]   # the instance token, a scope per
                                                              # kind, three queryables, the birth
rows["reconnect"] = (ab["reconnect"] + ba["reconnect"],
                     RECONNECTS * (sum(reopen("ab", zk2c_decl, admin=False))
                                   + sum(reopen("ba", [], admin=False))), None)
# Heartbeat (zk2c only): 18 B inside every third telemetry frame at a 60-s horizon, or one per
# 150 s at a declared 300-s horizon.
rows["heartbeat"] = (0, B(HB_EXTRA) * (DAY // 30), B(HB_EXTRA) * (DAY // 150))

today = sum(r[0] for r in rows.values())
after = sum(r[1] for r in rows.values())
for k, (a, b, c) in rows.items():
    print(f"{k:11s} today {a / 1e3:9.1f} KB/day   zk2c {b / 1e3:7.1f} KB/day"
          + (f"   (variant {c / 1e3:.1f} KB)" if c else ""))
print(f"W2 total: today {today / 1e6:.2f} MB/day = {100 * today / RF_DAY_BYTES:.1f} % of RF; "
      f"zk2c {after / 1e3:.0f} KB/day = {100 * after / RF_DAY_BYTES:.2f} %; {today / after:.1f}x less")
assert (round(today / 1e3), round(after / 1e3), round(today / after)) == (18_367, 443, 41)

# The model against the baseline (README, "RF: W2"): a day, under the lean stack, compress
# storing, each element under its sender. "Lean, derived" is the bare run plus 16 B an SDU.
W2_LEAN = {"telemetry": 8_866.6e3, "config": 6_238.0e3, "health": 242.9e3, "alarms": 111.3e3,
           "keep-alive": 390.1e3, "desired": 8.6e3}                          # A->B
W2_LEAN_BA = {"desired": 52.3e3, "keep-alive": 585.2e3}
W2_AIRED = 1_819 / 2_232     # telemetry puts that reached the air: router A dropped the rest
                             # through the streams' `drop` congestion control (RF W2 finding 3)


def within(model, measured, tol):
    return abs(model / measured - 1) <= tol


# What the owner sends is the model; what crossed is 81.5 % of the telemetry: within 1 %.
assert within(ab["telemetry"] * W2_AIRED, W2_LEAN["telemetry"], 0.01)
# Config: the measured view (1,476 B) and status (226 B) give the measured 2,066 B a re-put
# exactly. W2 keeps config.v1's three-group radio, 4 % under the owner example's five groups.
assert config_reput(1476, 226) == 1592 + 314 + 10 * LEAN
assert within(ab["config"], W2_LEAN["config"], 0.04)
assert within(ab["health"], W2_LEAN["health"], 0.04)
# Alarms: the model's 138-B document is larger than the example's 110-132 B, and its window
# held 12 confirmations where a day's rate gives 13.
assert within(ab["alarms"], W2_LEAN["alarms"], 0.22)
# The uplink is busier than one burst per 10 s, so it sends 11 % fewer keep-alives.
assert within(ab["keep-alive"], W2_LEAN["keep-alive"], 0.13)
assert within(ba["keep-alive"], W2_LEAN_BA["keep-alive"], 0.01)
# The desired GET with the run's own key and 44-B document: within the window's 6 GETs a 300 s.
assert within(288 * (sdus([req], RF_MTU)[1] + sdus([reply(44, TASKS_KEY) + FINAL], RF_MTU)[1]),
              W2_LEAN["desired"] + W2_LEAN_BA["desired"], 0.03)
# Both directions, less the telemetry router A dropped and the acts the window did not hold:
# within 1 % of the 16,494.8 KB that crossed (lean, derived).
crossed = today - ab["telemetry"] * (1 - W2_AIRED) - ab["events"] - ab["reconnect"] - ba["reconnect"]
assert within(crossed, 15_857.4e3 + 637.4e3, 0.01)
print(f"W2 by sender: A->B {sum(ab.values()) / 1e3:,.0f} KB, B->A {sum(ba.values()) / 1e3:,.0f} KB; "
      f"crossed {crossed / 1e6:.2f} MB (measured 16.49); a view crosses whole with p = "
      f"{0.95 ** 8:.2f} (measured 38 of 65)")
assert round(0.95 ** 8, 2) == 0.66

# 7. The sensor record (40 bits) in several encodings -------------------------------------
def cbor_uint(x):
    return 1 if x < 24 else 2 if x < 2**8 else 3 if x < 2**16 else 5


t, h, p, v = 21.5, 43.5, 1013.2, 3.7
scaled = [round(t * 10), round(h * 2), round(p * 10), round(v * 100)]
record = [jlen({"temperature": t, "humidity": h, "pressure": p, "battery": v}),
          jlen({"t": t, "h": h, "p": p, "b": v}),
          1 + sum(map(cbor_uint, scaled)), 2 + 1 + 2 + 1, B(11 + 8 + 13 + 8)]
assert record == [68, 38, 11, 6, 5]
assert [put(n, hlc=True, enc=1) for n in record] == [99, 69, 42, 37, 36]
assert [put(n + B(MUT_HDR)) for n in record] == [78, 48, 21, 16, 15]

# 8. W1: zenoh-modem's device telemetry --------------------------------------------------
COUNTERS_W1, GAUGES_W1 = 51, 22               # modem.v3.toml's telemetry.kind counts
gauge_bits = [10, 20, 20, 4, 16, 16, 3, 20, 16, 24, 24, 24, 24, 24]     # 14 integer gauges
measure_bits = [9, 7, 7, 9, 14, 10, 7, 7]                               # 8 real-valued
assert len(gauge_bits) + len(measure_bits) == GAUGES_W1
G = sum(gauge_bits) + sum(measure_bits)
key32 = FRAME_HDR + COUNTERS_W1 * 32 + G
key24 = FRAME_HDR + COUNTERS_W1 * 24 + G
delta = FRAME_HDR + 73 + 16 * 32 + 8 * 14    # changed bitmap, 16 counters, 8 gauges changed
assert (G, B(key32), B(key24), B(delta)) == (315, 254, 203, 98)
# modem.v3 streams are priority data_low: QoS extension on each put and each frame. The far
# readers subscribe to each reading's key, so each put rides the scope they declared. The four
# link readings are a queue depth, a 0/1 flag, a wait in ms (0 while the medium is free) and a
# drop count: 2-byte values, as gen's measured ones were (RF W1 finding 3). All 73 keep 6 bytes,
# since byte and frame counters reach six digits.
link_today = put(2, hlc=True, enc=1, qos=True)
reading_today = put(6, hlc=True, enc=1, qos=True)
link_zk2c = put(B(FRAME_HDR + 32 + 10 + 16 + 16), qos=True)
lt = 4 * 1440 * sdus([link_today], RF_MTU, qos=True)[1]
lt_batched = 1440 * sdus([link_today] * 4, RF_MTU, qos=True)[1]
lc = 1440 * sdus([link_zk2c], RF_MTU, qos=True)[1]
at = 73 * 1440 * sdus([reading_today], RF_MTU, qos=True)[1]
ac = (96 * sdus([put(B(key32), qos=True)], RF_MTU, qos=True)[1]
      + 1344 * sdus([put(B(delta), qos=True)], RF_MTU, qos=True)[1])
print(f"W1 RF: 4 readings/min today {lt / 1e3:.0f} KB/day ({lt_batched / 1e3:.0f} batched), "
      f"zk2c {lc / 1e3:.0f} KB/day; all 73/min today {at / 1e6:.2f} MB/day, zk2c {ac / 1e3:.0f} KB/day")
# The model sends each reading in its own SDU. zenoh batched them as they met the paced link,
# 3.1 SDUs a tick, and compress saved 153 B in 10 minutes (RF W1 findings 2 and 5, lean-1).
# Unbatched is the upper bound and all four in one SDU the lower; with the measured batching
# and saving the model is within 1 % of the 280,507 B measured. Keep-alives are exact.
W1_RF_DATA, W1_RF_KA = 280_507, 563_030
assert lt_batched < W1_RF_DATA < lt
assert within(1440 * (4 * link_today + 3.1 * (FRAME + QOS + LEAN)) - 153 * 144, W1_RF_DATA, 0.01)
assert within(1440 * 23 * (KEEPALIVE + LEAN), W1_RF_KA, 0.001)
assert within(lt + 1440 * 23 * (KEEPALIVE + LEAN), 843_537, 0.07)      # the uplink, measured


def air(put_bytes, sf, n=1):
    """Airtime of one SDU of n puts on a data_low frame."""
    return lora_air_s(n * put_bytes + FRAME + QOS + LEAN, sf)


for sf in (7, 9, 12):
    k, d = air(put(B(key32), qos=True), sf), air(put(B(delta), qos=True), sf)
    print(f"SF{sf}: 4 readings/min today {4 * 1440 * air(link_today, sf):.0f} s/day "
          f"({1440 * air(link_today, sf, 4):.0f} batched), zk2c {1440 * air(link_zk2c, sf):.0f} s/day; "
          f"all 73 every minute {96 * k + 1344 * d:.0f} s, every 2 min {96 * k + 624 * d:.0f} s; "
          f"hourly keyframe + 15-min deltas {24 * k + 72 * d:.0f} s (of 864)")
assert 1440 / 6 * air(link_zk2c, 12) < LORA_DUTY_S < 1440 / 5 * air(link_zk2c, 12)


# W1 on LoRa at zenoh-modem's timers (SF, lease, keep_alive): deploy/README.md's, and SF7 at
# keep_alive 4. The uplink carries the readings, a keep-alive between two of them where
# lease / keep_alive is under 60 s, and the reopens: at keep_alive 2, each keep-alive lost on
# the idle downlink (the readings refresh the uplink). An idle link reopens on either
# direction's loss. A reopen re-sends the handshake, the admin space and, with W1's owner up,
# its declarations (bare-1). A->B, gen's: the instance, bundle and interface token, two
# operations and the state; B->A, the four subscriptions.
LORA = {"sf7": (7, 60, 2), "sf9": (9, 60, 2), "sf12": (12, 540, 2), "sf7ka4": (7, 60, 4)}
W1_DEVICE = (presence(DEV, ("modem.v3",))
             + [keyexpr(f"zk2{DEV}/modem.v3/@op/action") + QUERYABLE + 3,
                keyexpr(f"zk2{DEV}/modem.v3/@op/sdu/lanes") + QUERYABLE,
                keyexpr(f"zk2{DEV}/modem.v3/state") + SUBTREE])
W1_FAR = [keyexpr(f"zk2{DEV}/modem.v3/stream/{k}") + SUBSCRIBER for k in
          ("sched/queue_depth", "air/duty_cycle_wait_ms", "air/duty_cycle_available",
           "sdu/dropped_total")]
assert sum(W1_FAR) == 316 - 4 * (FRAME + QOS)                  # bare-1: 316 B in four SDUs
lora = {}
for name, (sf, lease, ka) in LORA.items():
    mtu = 115 if sf == 9 else 222
    ka_day = DAY * ka / lease                                  # an idle direction's keep-alives
    ka_air = lora_air_s(KEEPALIVE + LEAN, sf)
    w1_ka = 1440 * (math.ceil(60 / (lease / ka)) - 1)
    reopen_air = {(d, w): sum(lora_air_s(x, sf) for x in reopen(d, decl, mtu))
                  for d, w, decl in (("ab", "idle", []), ("ba", "idle", []),
                                     ("ab", "w1", W1_DEVICE), ("ba", "w1", W1_FAR))}
    rate = {"idle": 2 * reopens_a_day(ka_day, ka), "w1": reopens_a_day(ka_day, ka)}
    data = (1440 * air(link_today, sf, 4), 4 * 1440 * air(link_today, sf))   # batched, not
    lora[name] = {"keepalive": ka_day * ka_air, "w1_keepalive": w1_ka * ka_air, "data": data,
                  "rate": rate, "reopen_air": reopen_air,
                  "idle": [ka_day * ka_air + rate["idle"] * reopen_air[(d, "idle")] for d in ("ab", "ba")],
                  "w1": [data[1] + w1_ka * ka_air + rate["w1"] * reopen_air[("ab", "w1")],
                         ka_day * ka_air + rate["w1"] * reopen_air[("ba", "w1")]]}
for name in ("sf7", "sf9", "sf12"):
    m = lora[name]
    print(f"{name}: W1 uplink {m['w1'][0]:,.0f} s/day (readings {m['data'][1]:,.0f} s, "
          f"{m['data'][0]:,.0f} batched; keep-alives {m['w1_keepalive']:.0f} s; {m['rate']['w1']:.0f} "
          f"reopens), downlink {m['w1'][1]:,.0f} s; idle {m['idle'][0]:,.0f} s each way "
          f"({m['rate']['idle']:.0f} reopens) (of 864)")
print(f"sf7ka4: idle {lora['sf7ka4']['keepalive']:.1f} s each way")
assert [round(lora[n]["w1"][0]) for n in ("sf7", "sf9", "sf12")] == [909, 3100, 21527]
assert round(lora["sf7"]["idle"][0]) == 281
# The model against the baseline's lean runs (README, "LoRa, a day"; results/*/analysis.json
# for the other SDUs per direction): each window's length, its reopens, and per direction
# its airtime a day, its readings' and the air of what was neither keep-alive nor reading.
LORA_RUNS = {("sf7", "idle"): (600, 2, (270.1, 250.1), None, (136.7, 116.7)),
             ("sf9", "idle"): (600, 2, (986.2, 944.7), None, (558.8, 517.4)),
             ("sf12", "idle"): (2700, 2, (1737.5, 1527.2), None, (1357.9, 1147.7)),
             ("sf7ka4", "idle"): (600, 0, (296.4, 296.4), None, None),
             ("sf7", "w1"): (600, 2, (999.2, 356.5), 639.1, (286.0, 208.3)),
             ("sf9", "w1"): (600, 1, (2493.9, 930.1), 1860.1, (420.1, 479.1)),
             ("sf12", "w1"): (1800, 0, (14031.8, 427.0), 13668.5, None)}
for (name, win), (secs, count, total, data, other) in LORA_RUNS.items():
    m = lora[name]
    # Reopens: each window's count is within one of the rate's (0 to 2 events a window).
    assert abs(count - m["rate"][win] * secs / DAY) <= 1
    if data:      # the readings: between all four in one SDU and each in its own
        assert m["data"][0] < data < m["data"][1]
    if other:     # a reopen's air: below the model's where rf.zdict compressed the
                  # declarations, above it where an open was retried
        per = [other[i] / (count * DAY / secs) for i in (0, 1)]
        assert all(within(per[i], m["reopen_air"][(d, win)], 0.45) for i, d in enumerate(("ab", "ba")))
    print(f"  {name} {win}: model {m[win][0]:,.0f} / {m[win][1]:,.0f} s, measured "
          f"{total[0]:,} / {total[1]:,} s")
# Where the link held, keep-alives match exactly: SF7 at keep_alive 4, 296.4 s each way, and
# W1's uplink at SF7, 1,440 a day and 74.1 s.
assert round(lora["sf7ka4"]["keepalive"], 1) == 296.4 and round(lora["sf7"]["w1_keepalive"], 1) == 74.1
# The names measurements/wire-baseline/model.py reads, for its per-direction figures.
hs = sum(sum(reopen(d, [])) for d in ("ab", "ba"))
tok, kexp, qbl, sub = sum(DEVICE[:11]), sum(DEVICE[11:15]), sum(DEVICE[15:]), sum(FAR)

# 9. Keep-alives on each medium, both directions -----------------------------------------
for name, lease, ka in (("defaults", 10, 4), ("RF proposed", 240, 4), ("LoRa SF7", 60, 2),
                        ("LoRa SF12", 540, 2), ("SBD", 3600, 2)):
    print(f"{name}: {2 * ka * DAY / lease:.0f} keep-alives/day")
assert 2 * 4 * DAY / 10 == 69_120 and round(2 * 2 * DAY / 3600) == 96
print("SF12 keep-alive every 30 s:", round(120 * lora_air_s(KEEPALIVE + LEAN, 12)), "s/h of 36")
sbd_key24 = put(B(key24), qos=True) + FRAME + QOS + (1 + 4 + 8)
sbd_key32 = put(B(key32), qos=True) + FRAME + QOS + (1 + 4 + 8)
print("SBD: keyframe SDU", sbd_key24, "B (24-bit counters),", sbd_key32, "B (32-bit), of", SBD_SLOT)
assert sbd_key24 <= SBD_SLOT < sbd_key32 and 48 * SBD_SLOT == 12_960

# 10. zk2c against the zk2c runs ----------------------------------------------------------
# Baseline, "zk2c on RF, and W1-73 at LoRa SF7": W1-73 across rf0's face with zenctl gen as
# modem.v3's owner, W2 with the owner example and gen at link.v2's transport, bare and decoded.
# zk2c's wire is the reference's: no HLC on a put, each on a scope its far reader declared by
# its exact key, a reliable or block one with the QoS extension. The face's 16-bit sequence
# numbers make a frame 3 B (the model's FRAME is 5), and an envelope (CH, run, seq) is 6 B
# with a 16-bit run, 8 B with the 32-bit one gen and the owner example draw without a run file.
FRAME16 = 1 + 2
ENV16, ENV32 = 1 + 2 + 3, 1 + 4 + 3


def sdu_list16(messages, layers=LEAN, mtu=RF_MTU):
    """sdu_list() on a link.v2 face: its 3-byte frame, no QoS extension on the frame."""
    cap = mtu - layers - FRAME16
    out, used = [], None
    for m in messages:
        if m > cap:
            rest = m + FIRST_FRAG
            while rest > 0:
                out.append(min(cap, rest) + FRAME16 + layers)
                rest -= cap
            used = None
            continue
        if used is None or used + m > cap:
            out.append(FRAME16 + layers)
            used = 0
        out[-1] += m
        used += m
    return out


def sdus16(messages, layers=LEAN, mtu=RF_MTU):
    s = sdu_list16(messages, layers, mtu)
    return len(s), sum(s)


# W1-73. frames/fast's record (modem.v3 3.3, schemas/modem-readings.json) holds the 73
# readings at zenoh-modem's ranges, each optional: 51 counters and 22 gauges, 1,469 bits of
# codes. A frame's body (core §7.7) is the time (42 bits), `full` (1), then per field a
# presence bit and its code; a delta record puts one bit per field before those that follow.
# Section 8's illustrative widths gave 254 B and 98 B.
READING_COUNTERS = [20, 24, 20, 24] + [20] * 31 + [24, 20, 20, 24, 20, 24, 20, 40, 40] + [32] * 7
READING_GAUGES = [10, 20, 16, 4, 1, 16, 3, 24, 16, 9, 7, 7, 9, 14, 27, 27, 27, 27, 20, 7, 7, 7]
assert (len(READING_COUNTERS), sum(READING_COUNTERS)) == (51, 1164)
assert (len(READING_GAUGES), sum(READING_GAUGES)) == (22, 305)
HEAD = 42 + 1 + 73                    # the time, `full`, and the full record's presence bits
full_body = B(HEAD + sum(READING_COUNTERS) + sum(READING_GAUGES))
none_body = B(HEAD)
assert (full_body, none_body) == (199, 15)
# gen writes every field at random on even ticks and none on odd ones, so its frames are
# full records of every reading or of none: 207 B and 23 B of payload with its 32-bit run,
# in SDUs of 215 B and 30 B (zk2c-w1-bare-1, exact), a day 720 of each.
assert [put(ENV32 + b) + FRAME16 for b in (full_body, none_body)] == [215, 30]
W1_73_FRAMES = 176_397                        # bare, a day (zk2c-w1-bare-1, 1,800 s)
assert within(720 * (215 + 30), W1_73_FRAMES, 0.001)
# The face's events at gen's rates, its contract's ceilings: face/device_fault (`low`, one
# a minute) is 97 B of form 4 in a 120-B SDU, its key's 12-character occurrence chunk and a
# reliable, block event's QoS extension; 172,941 B a day. The two `rare` ones never fell in
# the window.
assert put(97, suffix=13, qos=True) + FRAME16 == 120
assert within(1440 * 120.1, 172_941, 0.001)
# Corrected: a full record every 15 and a delta each minute with 16 counters and 8 gauges
# moving at the layout's mean widths, a data put on the face (no QoS extension), a 16-bit
# run, the lean stack. A full frame's put (210 B) takes two SDUs under the lean stack.
c_bar = sum(READING_COUNTERS) / len(READING_COUNTERS)
g_bar = sum(READING_GAUGES) / len(READING_GAUGES)
key16 = ENV16 + full_body
delta16 = ENV16 + B(HEAD + 16 * (1 + c_bar) + 8 * (1 + g_bar))     # 73 delta bits, then each
assert (key16, delta16) == (205, 84)
w1_73 = 96 * sdus16([put(key16)])[1] + 1344 * sdus16([put(delta16)])[1]
print(f"W1-73 zk2c on RF, corrected: {w1_73 / 1e3:.0f} KB/day (section 8: {ac / 1e3:.0f} KB); "
      f"keyframe {key16} B, delta {delta16} B")
assert round(w1_73 / 1e3) == 168
# Under the lean stack (zk2c-w1-lean-1, gen's 16-bit run): gen's frames are the model's within
# 2 % (compress stored most of them, and saved 579 B in 30 minutes), and an idle direction's
# keep-alives exactly 1,440 a day, one a minute, where today's lease of 10 s sent 34,560.
gen_lean = 720 * sdus16([put(ENV16 + full_body)])[1] + 720 * sdus16([put(ENV16 + none_body)])[1]
assert within(gen_lean, 207_256, 0.02)
assert round(DAY * 4 / 240) * (KEEPALIVE + LEAN) == 24_480                # measured 24,479 B
# At LoRa SF7 across lora0's face (zk2c-sf7-lean-1, lease 120 s, keep_alive 4): gen's frames
# took 363.0 s of air a day, the model's 365.5 s within 1 % (zenoh's batch the driver's 206 B,
# a full frame in two SDUs). The uplink sent 2,160 keep-alives a day, not the 1,440 of one
# between two frames: frames a minute apart spared one keep-alive in four, a cost of the wire.
def face_air(message, sf):
    """One message's air across a LoRa face under the lean stack, on the 3-byte frame: zenoh
    fragments it at what a 222-B frame (115 B at SF9) leaves, and SF10-12 tile the SDUs."""
    return sum(lora_air_s(x, sf) for x in sdu_list16([message], mtu=115 if sf == 9 else 222))


sf7_gen = 720 * sum(face_air(put(ENV16 + b), 7) for b in (full_body, none_body))
assert within(sf7_gen, 363.0, 0.01)
# Corrected, as section 8's cases: every minute, every 2 minutes, an hourly keyframe with
# 15-minute deltas. Section 8 gave 325 s, 1,043 s (556 s) and 833 s.
fk = {sf: face_air(put(key16), sf) for sf in (7, 9, 12)}
fd = {sf: face_air(put(delta16), sf) for sf in (7, 9, 12)}
w1_73_air = {sf: (96 * fk[sf] + 1344 * fd[sf], 96 * fk[sf] + 624 * fd[sf],
                  24 * fk[sf] + 72 * fd[sf]) for sf in (7, 9, 12)}
print("W1-73 zk2c on LoRa, corrected: SF7 {:.0f} s every minute; SF9 {:.0f} s ({:.0f} s every 2 "
      "min); SF12 hourly keyframe + 15-min deltas {:.0f} s (of 864)".format(
          w1_73_air[7][0], w1_73_air[9][0], w1_73_air[9][1], w1_73_air[12][2]))
assert [round(x) for x in (w1_73_air[7][0], w1_73_air[9][0], w1_73_air[9][1], w1_73_air[12][2])] \
    == [288, 916, 502, 762]

# W2. Telemetry is the model's exactly: frames/sample's three records crossed in 93 B (91 B
# and a 32-bit run) and a 100-B SDU, where the model's 5-byte frame and 16-bit run give 100 B
# too; on the face's 3-byte frame with a 16-bit run it is 98 B.
W2Z = {"telemetry": 288_000, "heartbeat": 154_368, "keep-alive": 1_317}   # bare, a day
W2Z_SDUS = {"heartbeat": 5_225}                         # (zk2c-w2-bare-1, 2,100 s, decoded)
assert put(B(FRAME_HDR + 3 * (8 * 11 + 4 * 32)) + 2) + FRAME16 == 100
assert within(cycles / 3 * 100, W2Z["telemetry"], 0.001)
# The heartbeat is one put per interface with a collection (freshness.v2 §2.4), on its own
# key, never inside a telemetry frame as section 6 has it: health.v1's and rfcfg.v1's every
# 28.5 s, 18 B each, alarms.v1's every 427.5 s, 19 B. Health's and config's leave together
# and often share an SDU: the day lies between every pair in one SDU and each alone.
hb_pairs = DAY / 28.5
hb_low = hb_pairs * sdus16([put(18), put(18)])[1] + DAY / 427.5 * sdus16([put(19)])[1]
hb_high = 2 * hb_pairs * sdus16([put(18)])[1] + DAY / 427.5 * sdus16([put(19)])[1]
hb_lean = W2Z["heartbeat"] + LEAN * W2Z_SDUS["heartbeat"]
assert hb_low < hb_lean < hb_high
# The desired re-read every 300 s is today's GET, not section 6's conditional one: a 12-B
# request, and the author's 166-B reply and 6-B final in one SDU or two (measured 175 B in
# 1.7 SDUs a re-read), 63.9 KB a day where section 6 had 27.4 KB. The target's own report,
# put once in the window (366 B), is not in it.
des = 288 * (sdus16([12])[1] + sdus16([166, 6])[1])
assert within(des, 288 * (15 + LEAN) + 288 * (175 + 1.71 * LEAN), 0.06)
# The downlink's keep-alives: one a minute, less where a reply refreshed the lease.
assert within(rows["keep-alive"][1], W2Z["keep-alive"] + LEAN * 1_317, 0.10)
# A reconnect re-sends the device's presence and declarations, and the far side's
# subscriptions, one per resource by its exact key (core §8.5): 1,214 B in 11.4 SDUs A->B and
# 1,443 B in 13 SDUs B->A, bare, where section 6 priced the far side's as none.
rec = RECONNECTS * (1_214 + 1_443 + LEAN * (11.4 + 13))
modelled = rows["heartbeat"][1] + rows["desired"][1] + rows["reconnect"][1]
w2_zk2c = after - modelled + hb_lean + des + rec
print(f"W2 zk2c on RF, corrected: {w2_zk2c / 1e3:.0f} KB/day ({after / 1e3:.0f} KB modelled) = "
      f"{100 * w2_zk2c / RF_DAY_BYTES:.1f} % of RF, {today / w2_zk2c:.1f}x less than today: "
      f"heartbeats {hb_lean / 1e3:.0f} KB, desired {des / 1e3:.0f} KB, "
      f"reconnects {rec / 1e3:.0f} KB")
assert (round(w2_zk2c / 1e3), round(today / w2_zk2c)) == (674, 27)
# Measured under the lean stack with 16-bit runs (zk2c-w2-lean-1), as W2 makes it: the steady
# window's counters, 688,896 B A->B and 71,465 B B->A a day; gen's 1,399 events cut to W2's 15
# (77 B each); the acts at their daily counts, 2,186 B and 618 B; and 4 reconnects, 5,586 B and
# 6,604 B (zk2c-w2-bare-1, decoded). Compression stored 221 of 245 SDUs and saved 69 B.
W2Z_LEAN = (688_896 - 1_399 * 77 + 15 * 77 + 2_186 + 5_586) + (71_465 + 618 + 6_604)
assert within(w2_zk2c, W2Z_LEAN, 0.01)
print("all assertions hold")
```

## Appendix B. Sources and evidence boundaries

### Zenoh 1.10.1, read in the locally cached crates

- [Z1: Zenoh homepage][Z1] — the "minimal wire overhead of 5 bytes" claim, a minimum and not this report's accounting.
- [Z3: PUT message layout][Z3] — the stamp, Encoding and attachment fields of a put.
- [Z4: timestamp codec][Z4] — an NTP64 varint, then the id's length and bytes.
- [Z5: integer codec][Z5] — LEB128-style varints, at most nine bytes.
- [Z6: Encoding API][Z6] — the predefined ids and the custom id 0xFFFF with the media type as its schema.
- [Z7: keep-alive write loop][Z7] — the keep-alive timer reset by every batch written.
- [Z8: token declaration layout][Z8] — the token id, key scope and key suffix.
- [Z9: publisher builder][Z9] — key expressions declared by publishers, and express messages not batched.
- [Z10: configuration defaults][Z10] — lease 10,000 ms, keep_alive 4.
- [Z53: query timeout reply][Z53] — a router's timeout as `zenoh/bytes` with the payload `Timeout`.
- [Z54: transmission pipeline][Z54] — an express message flushes its batch at once.
- [Z55: sequence-number masks][Z55] — a 32-bit resolution masked to 28 bits, at most 4 bytes when encoded.

### zenoh-modem at `95f884b`

zenoh-modem's figures are from its simulator and netns lab; it states that no byte has reached an antenna. The repository was read at `origin/main` with `git show`.

- [Z11: README][Z11]
- [Z12: SBD verdict][Z12] — 69,120 keep-alives a day at the defaults, 96 tuned.
- [Z13: compression measurements][Z13] — 74 % on telemetry and 72 % on sensor JSON with a 1 KB dictionary.
- [Z14: simulator media][Z14]
- [Z15: aead layer][Z15]
- [Z16: deployment guide][Z16] — LoRa lease guidance; 124 s of air an hour at SF12.
- [Z17: idle traffic][Z17] — keep-alives of one byte each.
- [Z18: SBD hardware][Z18] — 340/270-byte slots, 20.6 % wasted.
- [Z19: handshake and MTU][Z19] — 192 bytes, an 80-byte floor.
- [Z20: RF ARQ requirement][Z20] — 3 retries; a lost `DeclareKeyExpr` is never repaired.
- [Z21: LoRa below Zenoh's floor][Z21] — fragment tiles at SF10–12.
- [Z22: fragment layer][Z22], [Z23: fec layer][Z23], [Z24: address layer][Z24], [Z25: compress layer][Z25]
- [Z26: bring-up cost][Z26]
- [Z27: contact windows][Z27] — 5.3 paid messages per coverage gap.
- [Z49: interfaces][Z49] — the four `link`-exposed readings and their downsampling.
- [Z50: FEC measurements][Z50] — 1.37 frames per SDU at RS 4+1.
- [Z51: SBD AT protocol][Z51] — every `SBDIX` a billed message.
- [Z52: architecture][Z52] — declarations re-paid at each reconnect, about 1.2 s of air per key at 300 bit/s.

### Prior art, consulted 2026-10-31

- [Z28: ITU-T X.691 (07/2002)][Z28] — a superseded edition, consulted for the rule's substance.
- [Z29: OSS, PER quick reference][Z29] — unaligned PER pads no field, and is used in 3GPP.
- [Z30: RFC 8724, SCHC][Z30]
- [Z31: bitproto][Z31] — fixed-width bit fields with no padding; a 2-byte size header since 0.4.
- [Z32: dbc-rs `Signal`][Z32] — documentation of a DBC implementation, not Vector's specification.
- [Z33: Scania FMS CAN specification][Z33] — a vendor reproduction of J1939's reserved codes; SAE's text was not consulted.
- [Z34: RFC 8428, SenML][Z34] — base time and CBOR labels.
- [Z35: RFC 9254, YANG-CBOR][Z35] — SIDs and delta-coded keys.
- [Z36: RFC 5798, VRRPv3][Z36] — the 8-byte header, 8-bit priority and 12-bit centisecond interval.
- [Z37: Sparkplug payload specification][Z37] — the evolving `3.x` branch.
- [Z38: Gorilla, PVLDB 8(12), 2015][Z38]
- [Z39: Sprintz, 2018][Z39]
- [Z40: ALP, SIGMOD 2024][Z40]
- [Z41: Elf, PVLDB 16(7), 2023][Z41]
- [Z42: RFC 9639, FLAC][Z42]
- [Z43: Miśkowicz, send-on-delta, 2006][Z43]
- [Z44: OPC UA Part 4 §7.22][Z44]
- [Z45: prediction-based data reduction survey, 2016][Z45]
- [Z46: dual prediction schemes, 2015][Z46] — "almost 98 %" fewer transmissions; abstract only.
- [Z47: Semtech SX1276 datasheet][Z47] — the time-on-air formula, checked through a mirrored copy.
- [Z48: zstd README][Z48] — small data and dictionaries.

Iridium SBD billing could not be verified at a primary source: reseller pages refused automated reads. The report uses zenoh-modem's per-message model and says where byte billing would change a conclusion.

[Z1]: https://zenoh.io/
[Z3]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/commons/zenoh-protocol/src/zenoh/put.rs
[Z4]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/commons/zenoh-codec/src/core/timestamp.rs
[Z5]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/commons/zenoh-codec/src/core/zint.rs
[Z6]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/zenoh/src/api/encoding.rs
[Z7]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/io/zenoh-transport/src/unicast/universal/link.rs
[Z8]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/commons/zenoh-protocol/src/network/declare.rs
[Z9]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/zenoh/src/api/builders/publisher.rs
[Z10]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/commons/zenoh-config/src/defaults.rs
[Z11]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/README.md
[Z12]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/sbd-verdict.md
[Z13]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/compression.md
[Z14]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/fake-medium/src/sim.rs
[Z15]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/layer-aead/README.md
[Z16]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/deploy/README.md
[Z17]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/idle-traffic.md
[Z18]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/sbd-medium/docs/hardware.md
[Z19]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/m1-handshake-mtu.md
[Z20]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/rf-arq-requirement.md
[Z21]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/sub-floor-lora.md
[Z22]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/layer-fragment/README.md
[Z23]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/layer-fec/README.md
[Z24]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/layer-address/README.md
[Z25]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/layer-compress/README.md
[Z26]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/m2-bringup-cost.md
[Z27]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/contact-windows.md
[Z28]: https://www.itu.int/ITU-T/studygroups/com17/languages/X.691-0207.pdf
[Z29]: https://www.oss.com/asn1/resources/asn1-made-simple/asn1-quick-reference/packed-encoding-rules.html
[Z30]: https://www.rfc-editor.org/rfc/rfc8724.html
[Z31]: https://bitproto.readthedocs.io/
[Z32]: https://docs.rs/dbc-rs/0.2.0/dbc_rs/struct.Signal.html
[Z33]: https://bodybuilder.scania.com/content/dam/bodybuilder/local-page/thailand/CAN_Communication_Specification_for_FMS.PDF
[Z34]: https://www.rfc-editor.org/rfc/rfc8428.html
[Z35]: https://www.rfc-editor.org/rfc/rfc9254.html
[Z36]: https://www.rfc-editor.org/rfc/rfc5798.html
[Z37]: https://github.com/eclipse-sparkplug/sparkplug/blob/3.x/specification/src/main/asciidoc/chapters/Sparkplug_6_Payloads.adoc
[Z38]: https://www.vldb.org/pvldb/vol8/p1816-teller.pdf
[Z39]: https://arxiv.org/abs/1808.02515
[Z40]: https://ir.cwi.nl/pub/33334
[Z41]: https://www.vldb.org/pvldb/vol16/p1763-li.pdf
[Z42]: https://www.rfc-editor.org/rfc/rfc9639.html
[Z43]: https://www.mdpi.com/1424-8220/6/1/49
[Z44]: https://reference.opcfoundation.org/Core/Part4/v105/docs/7.22
[Z45]: https://arxiv.org/abs/1607.03443
[Z46]: https://arxiv.org/abs/1509.08778
[Z47]: https://www.semtech.com/products/wireless-rf/lora-connect/sx1276
[Z48]: https://github.com/facebook/zstd
[Z49]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/interfaces.md
[Z50]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/measurements/fec.md
[Z51]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/sbd-medium/docs/at-protocol.md
[Z52]: https://git.marcpardo.eu/marcpardo/zenoh-modem/src/commit/95f884bce310ca4bf33f77cf9cf756b4df41a83e/docs/architecture.md
[Z53]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/zenoh/src/net/routing/dispatcher/queries.rs
[Z54]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/io/zenoh-transport/src/common/pipeline.rs
[Z55]: https://github.com/eclipse-zenoh/zenoh/blob/1.10.1/io/zenoh-transport/src/common/seq_num.rs

### Validation performed

- **Read for this report:**
  - the zk2 specifications and standard contracts cited;
  - `examples/zk2/zenoh-modem/`;
  - zenoh-modem's documentation, measurements and layer READMEs at `95f884b`;
  - the cached Zenoh 1.10.1 codec, protocol, transport and configuration crates.
- **Measured** (#989, PRs #997 and #998): today's wire on zenoh-modem's simulated RF and LoRa, end to end in both directions, in [`measurements/wire-baseline/`](measurements/wire-baseline/README.md). For this correction:
  - its README and each run's `results/*/analysis.json` were read;
  - its captures were decoded with its `zdecode.py`: `bare-1` and `w2-rf-bare-2`, committed, and `w2-rf-bare-3`, from the run's own directory and not committed. The GET and its reply come from that last one;
  - the sequence-number mask was read in the cached zenoh-transport 1.10.1 [Z55].
- **Computed:** every figure, with Appendix A, whose assertions pass.
- **Not run:** no radio test or runtime benchmark. The zk2c column is a model, and §8's P0 is the measurement that confirms or corrects it. SBD is not measured.
