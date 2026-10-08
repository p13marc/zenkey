# zk2 spike report

**Status: every spike has run (2026-10-08).** The spike measures r3's claims
before the core is specified (epic #585: "measure the network half before
specifying it"). Each section below is filled by its issue. The decisions
table is settled in #605, which produces r4.

Two draft upstream reports are waiting for the maintainer's go-ahead:
- [the storage manager](upstream/storage-manager-outdated-guard.md) (S5);
- [ACL denies on router links](upstream/acl-denied-declarations-cross.md) (S3).

Rules:
- **Numbers, never adjectives.** Every claim here cites a row of raw data.
- **A spike that overturns a working assumption reopens the issue that
  recorded it.**

## Environment

| | |
|---|---|
| zenoh | **1.10.1** from crates.io (the latest release, checked 2026-10-07), `unstable` feature |
| Routers | The harness's own binary (`spike router`): zenoh 1.10.1 sessions in router mode. A spike that needs plugins records the `zenohd` it built. |
| Host | AMD Ryzen 5 PRO 3600, 6 vCPU, Linux 6.12.107 (cloud VM); rustc 1.98.1; release builds |
| Transport | TCP on loopback unless a section says otherwise |
| Limits | RLIMIT_MEMLOCK 8 MiB (hard). It bounds zenoh SHM pools: see S9 |
| Code | Branch `zk2-spike`, `spike/` (never merged): `zk2rt`, a throwaway runtime over `zenkey-model`, and the `spike` harness |
| Raw data | `spike/results/<group>/` on `zk2-spike`, copied here to [`spike-results/`](spike-results/) by each docs PR |

## Decisions

Settled in #605. The leans are r3.3 §5's; the verdicts come from the
sections below.

| Item | Question | Lean (r3.3) | Spike | Verdict |
|---|---|---|---|---|
| Grammar | `zk2/` keys, kind tokens, namespaces, zenoh-ext | r3.3 §3.1 | S1 | **holds** (#597); one r3.2 §3.2 sentence corrected |
| U-A | P3 or P2 for control and commanding | **Decided: P3** | S10, S11, S12 | **confirmed** by S10, S11 and S12 |
| U-D | `@stream` as a kind token | Token | S3, S9 | **holds**: no measurable cost (S9); across a link, ambient selectors carried the samples and none of the frames (S3) |
| U-E | QoS defaults per pattern; `priority` in the core | As in §3.3 | S9, S11 | **defaults hold**; `real_time` + express lost 0.6–4 % at 1–5 kHz with no gain, so express stays opt-in (S9) |
| U-F | Decodability: two blessed kinds + others | As in §3.9 | S9 | **holds**: protobuf 126 ns, JSON 0.9 µs per message; raw over SHM for frames |
| U-G | Many-reply operations in the core | Yes (O6) | S6 | **holds**; consolidation `None` is necessary (`Latest` keeps 1 of 10) |
| U1 | State: producer + storage merged by timestamp | Merge | S5, S12 | **not met**: 10 of 42 wrong (4 from a storage-manager bug). Recommend an owner-authoritative GET, storage as `archive.v1`, and the upstream fix |
| U2 | Clock discipline | HLC MUST + hold writes past the last stored timestamp | S5, S12 | **catch-up confirmed**; add a bound on clocks *ahead* (S12) |
| U3 | Tombstone window | Core default, annotation override | S5 | 60 s default, with replication required for storages on state; ~100 B per tombstone |
| U4 | `complete` operation queryables | Keep | S6 | keep `complete`, but **O1 is not at-most-once across routers**: reword it |
| U5 | Token layout | Instance + interface tokens | S2 | **holds** (A = B per token); add a **presence budget**: discovery 1.2–1.9 s at 10k tokens, 46–49 s or never at 50k |
| U6 | Mandatory one-chunk `system` | Keep | S8 | on paper: holds ([`examples/zk2/shapes.md`](../../examples/zk2/shapes.md), #604) |
| U7 | Bundle stability | Build once, embed, retention rule | S7 | **holds**: protox = protoc byte for byte; normalized identity for `buf` |
| U10 | Large collections | No paging in the core | S5 | **holds** to 100k keys (248–390 ms) |
| U11 | Descriptor dynamics | Put + GET | S2 | **holds**: a re-mint per second over 10k tokens costs 0.7 KiB/s; make-before-break left no gap |
| U12 | Redundancy | Diagnose only | S6 | **diagnose** (the token check) **and delegate** to `redundancy.v1` |
| U13 | A constrained conformance level | Define it | S15, S3 | **define it**: wall-clock timestamps with catch-up, a literal prefix, gateway bundles for receive limits (S15); a link profile of about 1 KB zenoh batches at radio rates, `@stream` denied across the face, the far side attached as a client (S3) |
| R7, D15 | Bindings across a constrained face | Deny `@zk` on the face; bind statically | S3 | **restate the mechanism**: the far side attaches as a client (17 B per bring-up). On a router-to-router link a deny hides presence, but the denied declarations still cross (11.1 KB). [Draft upstream report](upstream/acl-denied-declarations-cross.md) |
| U14 | `default_permission: deny` as a MUST | SHOULD | S14 | **SHOULD** confirmed: deny gives P3 outright; under allow, D13 + R6 |
| U15 | The storage-manager position | With U1 | S5 | **require a fixed version** (draft upstream report) |
| U16, U19 | The `events` kind token | Yes (D3) | S5 | union replay works; the `retention` bound needs a time-series backend |
| U18 | Device-as-service at SNMP scale | Device-as-service | S2 | **holds at 5,000 devices** (15k tokens, 3.3 s); members (D9b) above that |
| U20 | `@state` for large populations | Yes (D3) | S2, S5 | **confirmed**: per-entity tokens break down between 15k and 50k (S2); 100k `@state` keys read in 248–390 ms (S5) |
| U21 | Allow rules under `default_permission: allow` | Not evaluated; compile to denies | S14 | **confirmed** on a live router |

## Harness (#591)

`just -f spike/justfile spike-smoke` runs two routers (A ← B), ten mock
services behind B (one client session each, every walkthrough and tcgui
contract), and one client on A. The run fails unless every step holds.
Raw data: [`spike-results/smoke/`](spike-results/smoke/).

One row, 2026-10-07 (`unix_s` 1791408734):

| Measure | Value |
|---|---|
| Instance tokens / interface tokens seen through two routers | 10 / 10 |
| Liveliness GET until every instance token is seen (services already up) | 0.3 ms |
| Descriptor (compact form, one interface) | 306 B |
| Contract fetch by hash (`BestMatching`, first attempt, bundle verified) | 0.60 ms; `camera.v1`, 1,761 B |
| State GET (`All` + `Latest`) on `camera.v1/state/info`: one value with a producer timestamp | 0.52 ms |
| Wildcard state GET, `zk2/smoke/*/*/state/**` | 24 replies |
| Call on `geo.v1/@op/elevation` (`BestMatching`, consolidation `None`) | 0.57 ms |
| RSS: router A, router B, the services process (10 sessions) | 13,756 KiB; 14,744 KiB; 18,744 KiB |
| Client allocations, presence through the call | 2,799 allocations; 640,567 B |

**What this shows.** The harness works end to end: keys built and parsed by
`zenkey-model`, bring-up in §3.10's order, contract retrieval by hash with
verification, S1 timestamps on state, `complete` operations.

**What it does not show.** These are not the S-group numbers. Presence was
read after every service was up, which is not cold discovery (that is S2).
Every exchange is on one loopback host, with no load.

## Paradigm spikes (EZ)

### S9 — typed-layer overhead (#592)

**Verdict:**
- **The typed path costs 126 ns per control message.** End-to-end latency on
  this host cannot resolve that against its noise.
- **Raw `@stream` frames over SHM are zero-copy end to end.**
- **The QoS defaults hold, and `express` must not become one.**
- **zenoh's SHM falls back silently under a default memlock limit.**

`spike s9 --repeat 3`: 26 cases, three interleaved repetitions, plus
micro-benchmarks. Raw data and the median table:
[`spike-results/s9/`](spike-results/s9/).

**Control messages** (a twist command through one router; p50 and p99 in
µs, median of three runs, with the p99 range):

| Case | p50 | p99 (range) | received |
|---|---|---|---|
| raw, 1 kHz, defaults | 212 | 3,911 (2,791–4,480) | 15,000/15,000 |
| protobuf, 1 kHz, defaults | 182 | 2,269 (1,030–4,757) | 15,000/15,000 |
| protobuf, 1 kHz, `real_time` + express | 203 | 4,950 (1,556–7,061) | **14,860/15,000** |
| raw, 5 kHz, defaults | 160 | 2,205 (449–3,697) | 60,000/60,000 |
| raw, 5 kHz, `real_time` + express | 172 | 3,315 (2,714–4,103) | **57,599/60,000** |
| protobuf, 1 kHz, defaults, on `@stream` | 198 | 1,950 (1,345–3,296) | 15,000/15,000 |
| protobuf, 1 kHz, defaults, under 4 MB @ 30 Hz | 203 | 2,331 (1,594–7,046) | 15,000/15,000 |
| protobuf, 1 kHz, `real_time` + express, under 4 MB @ 30 Hz | 200 | 2,390 (1,611–5,140) | 14,868/15,000 |

**Micro-benchmarks:**

| Measure | Value |
|---|---|
| Encode + decode, protobuf (prost) | 126 ns (29 B: proto3 omits the four zero fields; the raw path sends 64 B) |
| Encode + decode, JSON (serde_json) | 894 ns (99 B) |
| Encode + decode, CBOR (ciborium) | 1,046 ns (72 B) |
| A put, 64 B | 279 ns |
| A put with an explicit `new_timestamp()` (the state writer) | 273 ns |

**4 MB frames at 30 Hz** (p50 / p99 in µs, median of three runs):

| Path | p50 | p99 | Receive allocation per frame | Arrived contiguous |
|---|---|---|---|---|
| raw, router, no SHM | 4,989 | 19,425 | 8.40 MB | 0/150 |
| protobuf `bytes`, router, no SHM | 5,722 | 16,014 | 12.59 MB | 0/150 |
| raw, peer, no SHM | 2,824 | 16,689 | 8.40 MB | 0/150 |
| raw, peer, SHM with zenoh's defaults | 2,528 | 15,812 | 8.40 MB (**0/450 as SHM**) | 0/150 |
| raw, peer, implicit SHM with a 6 MiB pool | 716 | 2,884 | 0.08 MB (447/450 as SHM) | 149/150 |
| raw, peer, explicit SHM (written in place) | **273** | 2,027 | **0.02 MB** | 149/149 |
| flatbuffer, peer, explicit SHM | 786 | 3,347 | 0.02 MB | 150/150 |
| protobuf `bytes`, peer, explicit SHM | 1,236 | 3,377 | 4.21 MB | 150/150 |

**What the numbers say:**
- **The ≤ 5 % p99 budget cannot be measured here.** End-to-end p99 for small
  messages is 1.5–5 ms on this VM, and repetitions of the same case vary by a
  factor of up to 4. Raw and typed cases overlap entirely.
- **The deterministic cost can be measured.** prost encode + decode costs
  126 ns, under 0.1 % of the 150–400 µs p50. Allocations are the same on
  both paths: about 3 per received message and 4 per sent one.
- **The budget for r4 is therefore restated as a per-message cost:** ≤ 1 µs
  of CPU on the typed path for control-size messages. That is measurable on
  any host. The ≤ 5 % p99 target should be re-run on a quiet,
  CPU-isolated host.
- **Zero added copies holds for raw over SHM.** The producer writes into the
  SHM buffer and the consumer reads it in place: 0.02 MB allocated per frame,
  against 8.4 MB without SHM, where frames arrive fragmented and are
  reassembled. p50 is 273 µs against 2.8 ms.
- **A flatbuffer is zero-copy only on the read side.** The builder writes
  into its own buffer, which is then copied into SHM. Zero copies need the
  generated code to build in place, with a flatbuffers allocator over the
  SHM buffer. That is a codegen requirement (#611).
- **protobuf `bytes` pays one copy on each side,** so it is not a type for
  SHM frames.
- **zenoh-shm 1.10.1 `mlock`s every segment it creates or maps,** both pool
  and metadata (`shm/unix.rs:291`). The cost is the pool plus 1,280 KiB of
  metadata on the first allocation (`spike shm-probe`), on both sides.
- **The consequence on a host with RLIMIT_MEMLOCK = 8 MiB** (this one's hard
  limit, and a common default): zenoh's implicit 16 MiB pool cannot be
  created, and SHM **silently** falls back to TCP (0/450 as SHM).
- **A metadata allocation that cannot be locked panics.** It is an `unwrap`
  at `metadata/storage.rs:31`.
- **Deployment guidance:** set the memlock limit to at least the pool plus
  2 MiB, or size the pool to fit, and make tools report the fallback.
- **The router hop costs about 2 ms of p50 on a 4 MB frame** (5.0 ms against
  2.8 ms peer to peer).
- **The state writer's explicit timestamp is free:** 273 ns against 279 ns.

**Verdicts:**
- **U-D (`@stream` as a token):** it costs nothing measurable. p50 is 198 µs
  against 182 µs, and p99 is within noise. The token stays; S3 confirms that
  infrastructure can use it.
- **U-E (QoS defaults):**
  - **The §3.3 defaults hold.** `data` with no express delivered every
    sample at 100 Hz–5 kHz.
  - **`real_time` + express lost samples** (best_effort, drop): 0.6–1.0 % at
    1 kHz and 3.5–4.0 % at 5 kHz. It gave no p50 or p99 gain on loopback,
    including under 4 MB @ 30 Hz contention.
  - **`express` therefore stays opt-in,** and `twist_cmd.v1`'s
    `real_time` + express is re-examined in S3, where link contention could
    make priority matter.
- **U-F (decodability):** **holds.**
  - protobuf is the default for control: 126 ns.
  - JSON is affordable for documents and configuration, even at kHz rates
    (0.9 µs).
  - raw over SHM is the frame path.

**Not measured:** cross-host latency. The spike has a single VM.

### S10 — wildcard bindings (#593)

**Verdict: wildcard bindings hold.**
- Fan-in is correct under churn.
- The leave latency is known, and the lease bounds a network cut.
- The descriptor's binding record is enough to draw the graph.

`spike s10`: 23 cases, all passing. Raw data:
[`spike-results/s10/`](spike-results/s10/).

| Case | Result |
|---|---|
| R5 start-up wait | The first bound provider's token appeared 11 ms after its spawn, and its first sample 12 ms after |
| Fan-in 1 → 10 → 100 (`detections.v1` bound to `vehicle-01/*`) | 1/1, 10/10, 100/100 producers seen and delivering. Join to first sample, median: 1.2, 11.1, 89 ms (maximum 295 ms; bounded by the 10 Hz publish period with 90 producers starting at once) |
| Steady: 100 producers × 10 Hz × 5 s | 5,000 samples, **0 gaps, 0 duplicates** |
| A `nav.v2` producer on the same system | 0 samples delivered to the `detections.v1` binding |
| Leave: clean exit | 4–8 ms |
| Leave: `kill -9` | 3–7 ms |
| Leave: network cut (SIGSTOP; only the lease can tell) | 9,961–10,047 ms (zenoh's 10 s lease) |
| After SIGCONT | 5/5 back within 52 ms; the 100 long-lived producers all kept delivering |
| `*/tc` on 2, then 5 hosts | 2/2 and 5/5 systems answering, and the same number of interface tokens |
| Injection: 10 puts on `zk2/vehicle-01/*/detections.v1/stream/objects` | 10/10 discarded by the R6 filter |
| Graph from descriptors + interface tokens | 100 edges drawn, 100 producers delivering, **0 differ** |
| Descriptor with one wildcard binding | 222 B; 1 binding record found across all descriptors |
| Consumer RSS for 100 producers | +664 KiB |

**For r4:**
- **R5/R7:** a binding resolves at once and needs no presence. A consumer
  that waits on presence starts within one token propagation (11 ms here).
- **A network cut is detected at the lease, not before.** That is 10 s by
  default; deployments that need faster detection tune the lease.
- **R3 is sufficient.** A tool reading only descriptors and tokens drew
  exactly the data-flow graph.

### S11 — control arbitration (#594)

**Verdict: consumer-side arbitration meets a 50 Hz control loop's needs.
P3 is confirmed over the P2 fallback.**

`spike s11`: two actuators bind `twist_cmd.v1` as `cmd` to
`[safety, teleop, autopilot]`. The deadline and lifespan (100 ms each) come
from the contract's `timing.v1` annotations. Commanders run as separate
processes at the contract's 50 Hz, with its QoS. Raw data:
[`spike-results/s11/`](spike-results/s11/).

| Case | P3 (bindings) | P2 (`@in` sink) |
|---|---|---|
| Teleop takes over: first teleop send to the switch | 0.26 / 0.27 ms (l / r) | 0.33 / 0.34 ms |
| The two actuators switch within | 0.02 ms | 0.01 ms |
| Teleop silent (SIGSTOP): last command to autopilot resuming | 101.2 / 100.1 ms, by the deadline | 102.2 / 101.1 ms, by the deadline |
| Teleop crash (`kill -9`): signal to autopilot | 4.9 / 5.0 ms, **by liveliness** | 3.8 / 3.8 ms, by liveliness |
| All silent: last command to the dead-man stop | 101.5 / 100.4 ms | 101.5 / 100.4 ms |
| Safety preempts, from its spawn | 7.5 ms | 9.2 ms |
| False stops over the whole scenario | **0** | **0** |
| CPU of the process holding both actuators (2 ms evaluation tick each) | 7.0 % of one core | 7.7 % |
| Clock skew of −250 ms and +250 ms: a sender-clock lifespan check | rejects 100/100 and 100/100 of the skewed samples | — |
| The same skew, receive-clock deadline | Follows the skewed commander correctly | — |

**What the numbers say:**
- **Takeover costs one sample's latency.**
- **Silence is detected at the deadline plus the evaluation tick**
  (100–102 ms).
- **A crash is detected by liveliness long before the deadline** (4–5 ms).
- **The dead-man stop fires at the deadline, with no false stops.**
- **P2 times the same.** It loses on structure: a commander must address every
  actuator, and the sender's identity is *claimed* in the payload, where P3
  gets it from the key that only the commander may write (ownership, R6).

**`arbitration.v1` needs these rules:**
1. A binding is an ordered list of providers: the priority.
2. Each role has a deadline, from `timing.v1` on the contract, which a binding may tighten.
3. Arbitration is evaluated on every receive and on a timer of at most deadline/50 (2 ms here), against the **receiver's monotonic clock**.
4. A liveliness delete drops a provider at once.
5. No fresh provider produces the dead-man output, a defined safe value.
6. A source's identity comes from its key, never from the payload.

**`timing.v1` needs these rules:**
- `period` is the expected rate.
- `deadline` runs on the receiver's clock.
- `lifespan` checked against the **sender's** stamp needs synchronized
  clocks. At ±250 ms of skew it rejected every sample of a healthy
  commander. So it is either dropped or bounded by the HLC's maximum delta.

**U-A:** P3, confirmed by S10 and S11. S12 (#595), with #601's tombstone
cases, completes the evidence.

### S12 — store-and-forward (#595)

**Verdict: store-and-forward converges, with zero stale plans applied.**
- After the link heals, the vehicle converges in **1.4–1.6 s**: for one
  change, for ten changes, and for a delete.
- The two wrong answers are clock cases. U2's catch-up fixes the first;
  the second needs a bound on clocks that run *ahead*.

`spike s12`: `ground/fleet-mgr` owns `state plans/{vehicle}`, and
`vehicle-01/executor` binds `{vehicle} = self`. The vehicle's router
reaches the ground through a proxy, and cutting it takes the vehicle
offline. The executor applies a plan only if its timestamp is newer
(stale-command rejection). It re-reads on the fleet manager's presence and
every 2 s (R7), and reports its observed revision on its own state key.
There are two variants: storage on the ground, and storage on the vehicle.
Each was run with the stock and with the patched storage manager (see S5).
Raw data: [`spike-results/s12/`](spike-results/s12/),
[`spike-results/s12-patched/`](spike-results/s12-patched/).

| Phase | Result (both variants, stock and patched alike) |
|---|---|
| Offline: plan changed once / 10 times / deleted, then the link heals | Converged in 1,587 / 1,397 / 1,588 ms; the ground saw the observed revision 1.7–1.9 s after the heal |
| Back online, then the executor restarts | Converged in 11 ms and 0 ms |
| The fleet manager restarts 5 s behind, naive | **WRONG**: rev14 rejected as stale; never converged |
| The same, with U2's catch-up | Converged in 11 ms |
| A fleet manager 2 s ahead writes rev16, which a GET reads back at +2 s; 0.5 s later a right-clocked writer writes rev17 | **WRONG**: rev17 rejected as stale; the executor stays on rev16 until a write after the skew has passed (rev18: 11 ms) |
| Stale revisions applied over the whole run | **0** |

**What the numbers say:**
- **The owner's presence decides.** With the fleet manager present, its
  replies (and its tombstone) beat any storage, so the storage-manager bug
  of S5 never surfaced: stock and patched results agree.
- **A clock ahead is as harmful as a clock behind.** A router re-stamps a
  future-dated *put*, but not a GET *reply*. The same revision therefore
  carries two timestamps, and the future one makes the next right-clocked
  write look stale.

**For r4:**
- **U2** must bound skew both ways. A producer never stamps at or below
  the last stored timestamp (the catch-up), and never runs ahead of its
  router's HLC delta. The runtime can detect the second by comparing its own
  stamps with the router-stamped samples it receives.
- **U-A:** P3 holds for store-and-forward. The executor's binding
  `{vehicle} = self` is configuration.

### S13 — simulation and replay (#596)

**Verdict: switching the source needed zero code changes in every case.** One
detector binary was rebound by configuration alone. One case is blocked, and
the block is in the v1 tool: v1's `zenctl replay` cannot republish zk2 keys
into a namespace.

`spike s13`: 10 cases, all passing (the `zenctl replay` row records the
block). Raw data, including the binding files and the `.zrec`:
[`spike-results/s13/`](spike-results/s13/).

| Case | Result |
|---|---|
| `input` → `vehicle-01/cam-front` (`real.bindings.toml`) | 20 frames from it, 0 from any other source, its `info` state answered |
| `input` → `vehicle-01/replay-cam` | 20 / 0 / 1 |
| `input` → `sim-1/cam-front` | 20 / 0 / 1 |
| v1 `zenctl record zk2/vehicle-01/cam-front/camera.v1/@stream/image` (zenctl 0.11.0) | Works: the explicit stream is named, not `**` |
| v1 `zenctl replay --base replay` | **Blocked.** `--base` re-prefixes v1 keys only, so zk2 rows are republished verbatim on the live keys, and 0 frames reach namespace `replay`. A namespaced `--zenoh-config` is refused by design (RFC 09 §5). |
| A namespace-aware replay: capture, then republish through a session in namespace `replay` | 30 captured, 30 received by the detector in `replay`, bound to the *original* address |
| `clock.v1` at 1×: a commander silent, 100 ms deadline on the bound clock | Missed 101 ms of wall time after the last command |
| `clock.v1` paused | No miss in 3 s of wall time |
| `clock.v1` at 2× | Missed 55 ms of wall time after the last command (100 ms simulated) |

**For r4 and the tools:**
- **R1 holds.** A consumer's sources are its binding file.
- **Replay into a deployment namespace works** when the replayer's session
  carries the namespace. The zk2 `zenctl` (#612) needs `replay --namespace`.
  v1's refusal of namespaced sessions protects an explorer's view of the
  wire, and it does not apply to a replayer.
- **`clock.v1`** (this spike's sketch: simulated time in ns on a stream at
  100 Hz) works for deadlines. Its granularity is the clock's tick, so the
  profile must state the tick. Consumers then either extrapolate between
  ticks or accept tick granularity.

## Protocol spikes (EY, FA)

### S1 — grammar basics (#597)

**Verdict: the grammar holds.** It has no `@v1`-class trap, and S1 changes
no rule of the grammar. One sentence of r3.2 §3.2 is corrected for r4:
under `@stream`, zenoh-ext's initial history query works; what fails is
every path that parses the `@adv` token key. That is the real reason
E019 forbids `history` on explicit resources.

`just -f spike/justfile spike-s1`: 50 cases per run. Three runs, all
passing. Raw data: [`spike-results/s1/`](spike-results/s1/).

| Group | Case | Result |
|---|---|---|
| Round trips | All 24 example contracts. Every resource is built with slugged values (`ETH0`, `a/b`, `10.0.0.1`, `über`, `-`) and events get a ULID chunk. Plus instance, interface, member and contract keys. | **2,226 keys, 0 failures.** Every key parses and resolves to its own template with the same unslugged values. |
| Guard, offline | `zk2/<system>/**` and `zk2/**` over 2,030 plain and 196 verbatim or control keys | **0 violations** |
| Guard, live | A subscriber on `zk2/g/**`, with puts on `stream`, `state`, `events`, `@stream`, `@state` | It receives `stream`, `state` and `events` only |
| | A GET on `zk2/g/**` against a state queryable and an `@op` queryable | It reaches state, never `@op` |
| | A liveliness GET on `zk2/g/**`, then on `zk2/g/*/@zk/**` | No token, then the instance token |
| Namespace | A producer in namespace `dep1` × `stream`, `@stream`, `state`, `@state`, `@op`, `@zk` (16 cases) | **All hold:** the key is stripped for `dep1` consumers, prefixed (`dep1/zk2/…`) for un-namespaced ones, and invisible to namespace `dep2`. `dep1/zk2/**` sees no verbatim token, so the guard holds under a prefix too. |
| Advanced pub/sub (a) | History from a publisher already present (the subscriber's initial query on `<key>/@adv/**`) | **5/5** on `stream`, `state` and `@stream`, with and without a namespace |
| Advanced pub/sub (b) | A late-joining publisher, found by parsing its `@adv` liveliness token | **5/5** on `stream` and `state`; **0/5 on `@stream`**. Same with and without a namespace. |
| `@adv` tokens | `<key>/@adv/pub/<zid>/…`, one per advanced publisher | The zk2 parser refuses them. `**` never crosses `@adv`, so zk2 presence selectors do not see them. |
| Wildcard put | A put on `…/stream/*` and on `…/@stream/*` | Each reaches a concrete subscriber, which sees the **wildcard** key |
| R6 filter | `is_wild()` on a 66-byte concrete key, 10⁷ checks | 10.0–11.2 ns per sample |

**Why (b) fails under `@stream`.** zenoh-ext 1.10.1 parses advanced-publisher
tokens with the format `${remaining:**}/@adv/${entity:*}/${zid:*}/${eid:*}/${meta:**}`
(`advanced_cache.rs:40`). `**` cannot match a verbatim chunk, so the parse
fails for any key containing `@stream`. Two paths depend on that parse:
late-publisher detection (`advanced_subscriber.rs:1039`) and heartbeat
recovery (`advanced_subscriber.rs:1298`). The initial query never parses a
key, which is why (a) works.

**For r4:**
- §3.2's advanced pub/sub sentence becomes: "Under a verbatim chunk, history
  from publishers already present works, but late-publisher detection and
  heartbeat recovery do not. zk2 therefore allows history on plain `stream`
  and `state` only."
- R6 is confirmed necessary. A wildcard put reaches concrete subscribers
  and presents its wildcard key, and the filter costs about 10 ns per
  sample.
- Namespaces work with every kind token. The key fixtures of #607 need no
  change.

### S2 — liveliness scaling (#598)

**Verdict:**
- **The layout holds (U5): presence costs per token, not per layout.**
  Layouts A (r3's instance + interface tokens) and B (interface tokens under
  the instance token) measured the same at every scale.
- **Discovery is fast to 10k tokens and falls off a cliff between 15k and
  50k:**

  | Tokens | Discovery |
  |---|---|
  | 10k | 1.2–1.9 s |
  | 15k | 3.3 s |
  | 36k | 23.8 s |
  | 50k | 46–49 s, or never |

  At 50k, a liveliness GET returned almost nothing within 10 s.
- **r4 needs a presence budget.** The token count is what to spend:
  - r3's layout multiplies it by 1 + the number of interfaces;
  - ZenSight's shape spends 36k tokens;
  - device-as-service at 5,000 SNMP devices spends 15k (U18);
  - large populations belong in `@state` (U20, and S5's 100k keys).
- **zenoh#2678 bites at 996 tokens.** On a session that already watches
  presence, a liveliness GET with zenoh's default handler never completed.
  Tools use a callback.
- **Churn is cheap, and make-before-break leaves no gap (U11, D9a).** One
  re-mint per second over 10k tokens cost 0.7 KiB/s on the router link and
  0.7 % of a core. The re-minted service always had 1 or 2 live instance
  tokens, never 0. A descriptor put on each change added nothing, because
  nobody subscribed to it.

`spike s2`: token holders (child processes, up to 200 client sessions) on
router R2, an observer client on router R1, and R2 reaching R1 through the
byte-counting proxy (no shaping). Layouts:

| Layout | Tokens |
|---|---|
| A (r3 §3.10) | an instance token, plus one `@zk/alive/<iface>/…` token per interface (5 here) |
| B | the interface tokens under the instance token |
| C | instance tokens only |
| Members (D9b) | one parent service with member tokens |

Discovery is the time from the holders' `ready` to the observer's history
subscriber holding every token (limit 120 s). Raw data:
[`spike-results/s2/`](spike-results/s2/).

**Scale × layout:**

| Tokens | Layout | Discovery | R1 growth | R2→R1 bytes | Liveliness GET, fresh session | Liveliness GET, session already watching |
|---|---|---|---|---|---|---|
| 96 | A | < 50 ms | 440 KiB | 6 KiB | 6 ms | 24 ms |
| 96 | B | < 50 ms | 504 KiB | 6 KiB | 6 ms | 24 ms |
| 100 | C | < 50 ms | 320 KiB | 5 KiB | 4 ms | 24 ms |
| 996 | A | < 50 ms | 2,464 KiB | 72 KiB | 16 ms | **hung** (20 s) |
| 996 | B | < 50 ms | 2,176 KiB | 74 KiB | 16 ms | **hung** |
| 1,000 | C | < 50 ms | 2,676 KiB | 58 KiB | 22 ms | **hung** |
| 9,996 | A | 1.2 s | 22,956 KiB | 779 KiB | 679 ms | **hung** |
| 9,996 | B | 1.2 s | 21,024 KiB | 803 KiB | 682 ms | **hung** |
| 10,000 | C | 1.9 s | 23,600 KiB | 627 KiB | 1,550 ms | **hung** |
| 49,998 | A | **46.2 s** | 114,276 KiB | 4,065 KiB | 1 token in 10 s | 1 in 40 s |
| 49,998 | B | **48.6 s** | 103,716 KiB | 4,183 KiB | 0 in 10 s | 0 in 40 s |
| 50,000 | C | **never** (120 s) | 114,076 KiB | 3,716 KiB | 0 in 10 s | 1 in 40 s |

At 50k, a callback GET gathered 102 to 4,241 tokens in 15 s.

**Per token:**
- a router holds 2.1–2.4 KiB;
- the inter-router link carries 63–85 B per declaration;
- both are flat from 10k to 50k. Only the time grows faster.

**Shapes (the adopters):**

| Shape | Tokens | Discovery | R1 growth | R2→R1 |
|---|---|---|---|---|
| ZenSight: 1,000 hosts × 6 sensors × (1 instance + 5 interfaces) | 36,000 | **23.8 s** | 82,700 KiB | 2,901 KiB |
| U18 device-as-service: 5,000 SNMP devices × (1 instance + 2 interfaces) | 15,000 | 3.3 s | 36,176 KiB | 1,128 KiB |
| U18 member tokens: one poller with 5,000 device members (D9b) | 5,001 | 0.3 s | 10,476 KiB | 354 KiB |
| Member tokens: 512 containers × 20 hosts (D9b) | 10,260 | 1.2 s | 20,156 KiB | 686 KiB |

**Disruption:**

| Case | Observed |
|---|---|
| Router R2 killed under 9,996 tokens, then restarted | all 9,996 deleted at the observer 0.5 s after the kill; all back 2.5 s after the restart |
| 1,200 tokens' client loses R2 silently (blackhole) and reconnects to R1 | live count 1,200 → 0 at 10.0 s (the lease); back to 1,200 at 20.0 s; 1,200 deletes and 1,200 puts |

**Churn** (one service re-minted per second for 10 s over 9,996 tokens: new
tokens declared first, the old ones dropped after, as D9a says):

| Case | Router link | Routers' CPU | The service's live instance tokens |
|---|---|---|---|
| Re-mint only | 0.7 KiB/s | 0.7 % of a core | always 1 or 2 |
| Re-mint + a 300 B descriptor put on each change | 0.7 KiB/s | 0.7 % | always 1 or 2 |

**For r4:**
- **U5:** keep layout A. Add a **presence budget** to §3.10: about 10–15k
  tokens per presence domain keeps discovery within 2–4 s on this host. A
  deployment above it cuts the multiplier, and #605 picks how:
  - interface tokens only for the interfaces consumers discover by;
  - or instance tokens only, with the interfaces read from descriptors.

  Layout C at equal token counts is no faster: the count is the cost.
- **U18:** device-as-service holds at 5,000 devices (15k tokens, 3.3 s). It
  triples the token count against member tokens. Above about 5,000 devices
  per domain, members (D9b) are the shape.
- **U11:** put + GET holds. A descriptor put crosses only toward
  subscribers, and a re-mint costs about a dozen declarations.
- **U20:** confirmed from the presence side. Per-entity tokens break down
  between 15k and 50k, while S5 read 100k `@state` keys in 248–390 ms.
- **Liveliness is lease-bound.** A silent loss is noticed only after the
  10 s lease, and the client's move to another router took another 10 s. A
  router restart is noticed in 0.5 s.
- **Runtime rule (#2678):** a liveliness GET on a session that holds a
  liveliness subscriber uses a callback or an unbounded handler, never the
  default 256-slot FIFO.

### S3 — constrained links (#599)

**Verdict:**
- **`@stream` isolation holds across a link (U-D).** With the ground
  subscribed to `zk2/vehicle-01/**`, 4 s of 10 KB frames put 3.4 KB on the
  link: the 100 B stream samples, no frames. Frames crossed only when the
  ground named the `@stream` key.
- **At 64 kbit/s and above, discovery is cheap.** Presence for 50 services
  costs 11 KB. A descriptor takes about one RTT, and a tool's first view
  4.6 s.
- **At 2,400 bit/s, zenoh's defaults do not work at all.** A 64 KB batch
  outlasts the 10 s lease, so the link re-handshakes and resends in a loop:
  presence 0 of 100 tokens in 300 s.
  - **1 KB batches fix it:** link-speed presence in 38 s.
  - The batch size is part of a constrained face's configuration (U13).
- **R7 needs a different mechanism from the one r3.3 assumed.**
  - An ACL deny on `@zk` hides presence, but the denied declarations still
    cross a router-to-router link, key strings included. Bring-up cost
    11.1 KB, the same as with presence.
  - **A ground that is a zenoh *client* of the vehicle router** receives only
    what it shows interest in: a bring-up put 17 B on the link (11.1 KB router-to-router), a coverage gap 536 B (11.8 KB), and a statically bound state GET answered in 0.75 s.
- **A named `@stream` key starves a slow link.** Nothing is dropped: 4 s of
  frames took 29 s to drain at 64 kbit/s, and more than 600 s at 2,400 bit/s.
  Every request queued behind them.
- **Every coverage gap flaps every token**, and costs as many bytes as a
  bring-up.

**Setup.** `spike s3` builds a vehicle router with 50 mock `camera.v1`
services, a storage, and a holder of the 82 KB `zs.snmp.v1` bundle. It reaches
a ground router (the tools) through the harness's shaping proxy:
- the proxy behaves as a serial line: a one-way delay, a bandwidth cap per
  direction, bytes released in slices of about 50 ms, and a buffer of about
  0.5 s;
- windowed measures run until the link is quiet (64 B or less in 3 s).

**Not modelled:** loss, because `tc`/netem is not available here (over TCP,
loss would show as retransmission delay), and zenoh-modem's 220 B MTU.

The `@zk`-denied profiles carry an ACL on both routers' TCP face (the link
only; local clients use unix sockets) that denies every message on
`zk2/**/@zk/**` both ways. Each profile's router config is the
`link-<profile>.json` beside the data. The probes are `s3/probes.sh`.

Raw data: [`spike-results/s3/`](spike-results/s3/) (the profiles) and
[`spike-results/s3-probes/`](spike-results/s3-probes/) (the probes).

**A correction in this run.** The first version of the proxy released each
16 KB read whole after its link time, and buffered up to 256 KB. That made
two false findings:
- "at 2,400 bit/s presence never converges", for a mix of reasons;
- "a router queues `@stream` frames ahead of a tool's requests" at 64 kbit/s.

The proxy now emulates a line, and every profile was rerun.

**The profiles** (seconds / bytes vehicle→ground):

| Measure | 1 Mbit/s, 600 ms RTT | 64 kbit/s, 600 ms RTT | 2,400 bit/s, 200 ms RTT, zenoh defaults | 2,400 bit/s, 1 KB batches | the same, `@zk` denied (R7) | the same, ground as a client (R7) |
|---|---|---|---|---|---|---|
| Presence, first view (100 tokens) | 1.2 s / 11.3 KB | 2.4 s / 11.1 KB | **0 of 100 in 300 s** (85.5 KB) | 38.4 s / 11.3 KB | hidden: 0 tokens; bring-up still 37.7 s / 11.1 KB | hidden; bring-up **0.2 s / 17 B** |
| Presence replay after a reconnect | 2.7 s / 11.1 KB | 4.0 s / 11.1 KB | **0 of 100 in 300 s** | 39.9 s / 11.3 KB | — | — |
| One descriptor GET (312 B) | 0.61 s | 0.66 s | (the profile stopped) | 1.6 s | denied: 0 replies | denied: 0 replies |
| Bundle fetch, `camera.v1` (1,761 B) | 0.62 s | 0.85 s | | 6.7 s | local | local |
| Bundle fetch, `zs.snmp.v1` (82,391 B) | 1.3 s | 11.2 s | | 280 s (link speed) | local | local |
| State GET, answered by a storage on the vehicle | 0.61 s | 0.65 s | | **0 replies**, at once (note 1) | 1.4 s | 1.4 s |
| State GET, answered by the owner (static binding) | — | — | | — | 0.68 s | 0.75 s |
| 4 s of 10 KB frames + 100 B samples at 5 Hz, ground on `zk2/vehicle-01/**` | 3.4 KB | 3.4 KB | | 3.4 KB | 3.4 KB | 3.4 KB |
| The same, with the ground naming the `@stream` key | 204 KB in 7.2 s | 204 KB in **29.4 s** | | 179 KB, still draining at 604 s | the same | 179 KB, still draining at 604 s |
| A tool's first view: presence, 50 descriptors, 1 bundle, 1 state | 2.0 s / 22 KB | 4.6 s / 22 KB | | 165.7 s (note 2) | one state, statically bound: 91.5 s (note 2) | one state: 1.4 s (note 3) |
| 100 subscriber declarations toward the vehicle, zk2 / v1-shaped keys | 84 / 76 B each | 84 / 72 B | | 87 / 83 B | 94 / 86 B | 87 / 74 B |
| 3 coverage gaps of 2 s | 33.8 KB; 300 deletes + 300 puts | 34.1 KB; the same | | 34.5 KB in 128 s; the same | 11.8 KB per gap, 0 tokens seen | **536 B per gap**, 0 tokens seen |

**Notes:**
1. The storage started 0.8 s before the GET, and its queryable's declaration
   had not crossed yet. A GET with no route returns empty at once: silence,
   not absence, which is R5's point.
2. These views queued behind the previous step's frame backlog. Only 22 of
   the 50 descriptors arrived within the 120 s limit.
3. The proxy shapes each TCP connection separately. In the client profile
   every ground session has its own connection, so the tool did not queue
   behind the frame backlog. On one radio, all sessions share the line.

**Why the defaults fail at 2,400 bit/s** (probes, 100 tokens):

| Router config | First view (time; bytes; reconnects) | Replay after a cut |
|---|---|---|
| zenoh defaults (batch 65,535 B, lease 10 s) | 85.3 s; 24.3 KB; **4 reconnects** | 59.9 s; 16.8 KB; 2 beyond the healing reconnect |
| `batch_size` 1,024 | 24.1 s; 7.0 KB; none | 25.6 s; 7.0 KB |
| `lease` 60 s | 23.8 s; 6.9 KB; none | 25.3 s; 7.0 KB |
| both | 24.1 s; 7.0 KB; none | 25.5 s; 7.0 KB |

The defaults are also erratic. An earlier run of the same probe took 57 s and
175 s (2, and 8 beyond the healing one). The full profile above, with 50 services'
declarations (11 KB), never converged in 300 s. 7 KB at 300 B/s is 23 s:
tuned, the link runs at its own speed.

A batch must cross well within the lease, or the receiver drops the link
mid-batch. 1 KB batches keep the 10 s failure detection, which makes them the
right knob rather than a longer lease.

**What an ACL keeps off the link** (probes; 200 `@zk` tokens across a
router-to-router link, bytes vehicle→ground):

| Setup | Bytes | Denied key strings in the bytes | The ground sees the tokens |
|---|---|---|---|
| Router-to-router, no ACL | 11,811 | 201 of 201 | yes |
| Router-to-router, the deny on both routers | 10,530 | **201** | no |
| The deny on the vehicle router only | 11,097 | 201 | no |
| The deny on the vehicle router, egress only | 10,719 | 201 | no |
| The deny on the ground router only | 11,820 | 201 | no |
| 200 data queryables, no ACL / with a deny on their keys | 10,038 / 8,737 | 200 / **200** | — |
| **The ground as a client** of the vehicle router, no interest in `@zk` | **322** | 1 (its own GET's reply) | — |
| The ground as a client, holding a liveliness subscriber on `@zk` | 12,316 | 201 | yes |
| **The ground as a client, the deny on the vehicle router** | **285** | **0** | no |

The ACL controls what the far side *sees*, not what the link *carries*. On a
client link, declarations travel only toward interests, which is what makes
the deny effective.

**For r4:**
- **U-D:** confirmed on a link. `@stream` stays off ambient selectors.
- **R7 / D15, restated mechanism:**
  - Across a constrained face, the far side **attaches as a client** of the
    near router (interest-scoped declarations), with the `@zk` deny making it
    enforceable.
  - A router-to-router link carries every declaration, and in zenoh 1.10.1 a
    deny does not stop the key strings: [draft upstream
    report](upstream/acl-denied-declarations-cross.md), not filed.
- **U13, the constrained level, gains a link profile:**
  - batches of about 1 KB at radio rates;
  - `@stream` keys denied across the face unless `link.v1` downsamples them,
    because one named key starves the link for minutes;
  - presence never crosses: one gap costs a full re-declaration.
- **Declarations:** zk2 keys cost 4–12 B more per declaration than
  v1-shaped ones (84–94 B against 72–86 B), the difference in key length.

### S4 — contract serving (#600)

**Verdict: the retrieval rule holds, with one change for r4.** Accept the
first valid reply *as it arrives*. A client that waits for the GET to
complete pays the full timeout whenever a routed holder is slow or
unreachable. No bad hash was ever accepted.

`spike s4`: holders in four modes (ok, slow, corrupt, and small-only, a
constrained holder that cannot send more than 4 KB). The nearest holder sits on
the client's router, the others behind a second one. The per-attempt timeout
is 1 s. Raw data: [`spike-results/s4/`](spike-results/s4/).

| Case | Accepted | Replies (invalid) | First valid | GET complete |
|---|---|---|---|---|
| One holder (`camera.v1`, 1,761 B) | yes, attempt 1 | 1 | 1.0 ms | 1.1 ms |
| 200 equal holders, `BestMatching` | yes, attempt 1 | **1** | 5.1 ms | 5.2 ms |
| 200 equal holders, `All` (for comparison) | yes | 200 (352,200 B) | 6.9 ms | 6.9 ms |
| A slow nearest holder (3 s); 3 good behind router 2 | yes, attempt 1 | 2 | **1.3 ms** | 1,001.7 ms |
| A corrupt nearest holder | yes, attempt 1 | 2 (1) | 1.5 ms | 1.5 ms |
| An unreachable nearest holder (SIGSTOP after convergence) | yes, attempt 1 | 2 | 1.1 ms | 1,001 ms |
| Every holder corrupt (3) | **no**, after the retry; never accepted | 4 (4) | — | 11.7 ms |
| The holder three routers away | yes, attempt 1 | 1 | 1.4 ms | 1.4 ms |
| A constrained nearest holder (4 KB limit), `zs.sysinfo.v1` (83,688 B); a gateway holder behind router 2 | yes, attempt 1 | 1 | 6.4 ms | 8.7 ms |

**What the numbers say:**
- **"Exactly one reply" is a routing optimisation only.** Equal holders on
  one router gave 1 reply. Holders at different distances gave 2, because
  `BestMatching` reached the nearest *and* one behind the next router. S6
  shows the same per-router routing for operations.
- **The location-free key works for constrained holders.** A gateway or
  router-side holder serves what a pico participant cannot.

### S5 — state correctness (#601)

**Verdict:** merging producer and storage replies by timestamp (`All` +
`Latest`) gave **10 wrong answers in 42 cases** with the stock storage
manager 1.10.1. Four of them are a **storage-manager bug**, and a one-line
patch removes them. The six that remain are design-level, and each
points at a rule.

`spike s5`: the producer follows S1–S3 (it stamps every mutation, keeps a
tombstone window, and answers deletes with `reply_del`), or deliberately
breaks one rule. The storage router links zenoh-plugin-storage-manager 1.10.1
statically (memory volume, GC period 3 s) and reaches the main router through
a proxy that cuts the link. That is a real partition: SIGSTOP only buffers. The
cases ran without replication, and with two aligned replicas. Raw data:
[`spike-results/s5/`](spike-results/s5/) (stock) and
[`spike-results/s5-patched/`](spike-results/s5-patched/).

| Case | Without replication | With replication | Patched storage manager |
|---|---|---|---|
| Producer + storage; storage only | right, right | right, right | — |
| Stale storage (missed v2 behind a cut link), producer present | right | right | — |
| Stale storage, producer gone | **WRONG** (v1) | right (the replicas aligned) | WRONG without replication |
| Producer restarts 5 s behind, naive | **WRONG** (v1) | right, **by accident** (the bug let the older put win) | WRONG in both modes |
| The same, with U2's catch-up | right | right | right |
| Producer 2 s ahead | right; the subscriber's timestamp is −2,000 ms, the GET reply's +0 ms | same | — |
| Delete; the storage missed it; producer's tombstone in its window | right | right | — |
| The same, after the producer's 10 s window | **WRONG** (v2 resurrected) | right | WRONG without replication |
| Delete, then a late older put within 0.3 s, storage only | **WRONG** | **WRONG** | **right** |
| The same after the GC tick | **WRONG** | right | **right** |
| Wildcard delete, then a stale put (zenoh#2649) | **WRONG** (an empty value) | **WRONG** | **right** |
| Producer does not stamp (S1 broken), stale storage | **WRONG** (v1) | right | WRONG without replication |
| Events: replay GET of 1,000 occurrences from a union storage | right (4 ms) | — | — |
| Events: the same bounded by `_time=[now(-1.2)..]` | **WRONG** (1,000, expected 500: the memory backend ignores `_time`) | — | WRONG |

**The storage-manager bug** (draft upstream report:
[`upstream/storage-manager-outdated-guard.md`](upstream/storage-manager-outdated-guard.md)).
`guard_cache_if_latest` falls through when its cache holds a *newer* event
for the key, a delete for example. The older put then passes the
replication-log check (which lags) or the storage check (the entry is gone
after a delete), so an outdated put is accepted and the key resurrected.
Separately, the GC keeps the cache events *older* than the limit and drops
the recent ones. With both patched, every storage-caused wrong answer
disappears, including the wildcard-delete case (#2649 likely shares this
root).

**Scale** (producer + storage, wildcard GET):

| Keys | `Latest` | `None` | Storage only | Producer memory |
|---|---|---|---|---|
| 1,000 `state` | 1,000 in 5 ms | 2,000 in 6 ms | 1,000 in 5 ms | +328 KiB |
| 10,000 `state` | 10,000 in 40 ms | 20,000 in 36 ms | 10,000 in 21 ms | +1.7 MiB |
| 50,000 `@state` | 50,000 in 170 ms | 100,000 in 166 ms | 50,000 in 125 ms | +5.1 MiB (~104 B per entry) |
| 100,000 `@state` | 100,000 in 366 ms | 200,000 in 390 ms | 100,000 in 248 ms | — |

State puts must use reliable + block (r3 §3.3). A first run with the
default `put` (drop) lost 10,917 of 100,000 values on the way to the storage.

**Verdicts for #605:**
- **U1 (merge by timestamp).** The issue's bar, zero wrong answers in every
  scenario, is **not met**. The stock storage manager accounts for 4 of the
  10 wrong answers, and even patched, a storage answering alone can be
  stale. The recommendation for r4:
  - keep the merged GET, but make the **owner authoritative**: a storage is
    an *archive* (`archive.v1`), consulted explicitly or marked as such, and
    trusted alone only with replication on;
  - require a storage manager with the fix, which makes **U15** "require a
    fixed version";
  - until a fixed version ships, keep storages off `state/**` wherever
    deletes matter.
- **U2 (clock rule).**
  - **HLC MUST.**
  - **Catch-up on restart:** never stamp at or below the last stored
    timestamp of one's keys. It fixed the restart-behind case in every
    variant.
  - **A bound on clocks ahead,** within the router's HLC delta. Routers
    re-stamp future puts but not GET replies, and S12 shows the harm.
  - zenoh-pico participants cannot merge HLC timestamps, which is input for
    #617.
- **U3 (tombstone window).** The window protects only while it outlasts the
  storage's staleness. Without replication, staleness is unbounded. The
  recommended default is 60 s, with replication required for storages on
  state. The memory cost is about 100 B per tombstone, the same as a value.
- **U10 (large collections).** 100,000 keys come back in 248–390 ms on
  loopback, so no paging is needed at these sizes. The modelling advice
  stands for larger collections.
- **U16/U19 (`events`).** The union storage replays correctly (1,000 in
  4 ms). The `retention` bound cannot rely on `_time` with the memory
  backend: it needs a time-series backend, or a consumer-side filter.
- **U20 (`@state`):** confirmed at 50k and 100k.

### S6 — operations and ownership (#602)

**Verdict: O2, O6 and O7 hold. O1 does not.** A `complete` queryable plus a
`BestMatching` call is **not** at-most-once across routers:
- in a split-brain across two routers, **every call ran twice**;
- on a single router, every call ran once.

Exclusivity must therefore come from there being one serving instance,
which redundancy delegates and zk2 diagnoses. The token check found every
split-brain and flagged no standby.

`spike s6`: servers are child processes that report each execution.
Raw data, including the partition timeline:
[`spike-results/s6/`](spike-results/s6/).

| Group | Case | Result |
|---|---|---|
| Split-brain | 200 concrete calls, both instances on the client's router | A 200, B 0: **once each** |
| Split-brain | The second instance behind router 2 | A 200, B 200: **all 200 calls executed twice**; 400 replies |
| Split-brain | The token check (two alive instances of one service exposing one interface) | Found both cases |
| Replicated | 100 calls over 3 replicas (r1 on the client's router, r2 and r3 behind router 2) | r1 100, r2 100, r3 0: one execution per router per call |
| Replicated | The nearest replica is killed | 0 failed calls; first success 3.4 ms after the kill |
| Replicated | The nearest replica hangs | 10/10 answered by the far replica; first reply in 1.3 ms |
| Templated | A concrete call to h2's `interfaces/*/*/set` (a `complete` queryable over the template) | 1 reply; executions h1/h2/h3 = 0/1/0 |
| Fan-in | `zk2/*/tc/tc.netif.v1/@op/diagnostics`, `All`, over exact-key `complete` queryables | 3 replies, one execution per host |
| Fan-out (O2) | Non-concrete calls to a `fanout = "forbidden"` operation (one host; every host) | 0 values; 1 and 3 `fanout_forbidden` refusals; 0 executions |
| Many replies (O6) | 2 servers × 5 replies on one key | `None` 10, `Monotonic` 10, **`Latest` 1, `Auto` 1** |
| Partition | Server frozen (SIGSTOP) | Calls wait out the 1 s timeout until the 10 s lease (9 timeouts), then fail at once from 9.6 s |
| Partition | After SIGCONT | First success at 22 ms |
| Standby | Active + a standby holding its instance token only | 2 instances seen, **no finding**, 20/20 executions on the active |
| Metadata (O7) | `{actor, request_id}` as a request attachment | Seen by the server, echoed on the reply |

One of two runs showed a first post-SIGSTOP call failing at once. It did not
reproduce, and the committed timeline shows the lease pattern.

**Verdicts for #605:**
- **U4 (`complete` queryables, O1):** keep `complete`. It routes concrete
  calls, captures templated ones, and makes fan-in work. Reword O1: zk2
  promises at-most-once **only with a single serving instance**.
  `BestMatching` reaches one complete queryable per router, so a split-brain
  across routers executes every call on each side.
- **U12 (redundancy):** diagnose and delegate.
  - zk2 detects split-brain from tokens: two alive instances of one service
    exposing one interface.
  - `redundancy.v1` owns exclusivity. ZenSight's claim protocol, where a
    standby holds its instance token only, raised no finding and took no
    calls.
- **U-G (many replies):** in the core. O6's "consolidation `None`" is
  necessary: `Latest` and `Auto` keep 1 of 10.
- **Replicated serving** costs one execution per router per call, which is
  harmless only for idempotent operations, as `serving = "replicated"`
  already requires.

### S7 — bundle stability and classifier feasibility (#603)

**Verdict:**
- **U7 holds.** Bundles are built once, with bytes, plus the retention rule:
  - protox compiles byte-identically, and matches protoc 3.21.12 byte for byte;
  - a normalized comparison recognises `buf build`'s different bytes as the same schema.
- **The protobuf classifier is feasible on prost-reflect.** buf's WIRE_JSON
  rules re-implemented on it agree with `buf breaking` on 14 of 14 cases in
  both argument orders.
- **But `buf` in both orders cannot define FULL_TRANSITIVE:** it calls adding
  a field breaking. zk2 needs its own directional reader/writer semantics.
- **The zk2 JSON Schema subset is written down.** Its classifier agrees with
  `jsoncompat` on 13 of 15 cases. The other two are a deliberate choice:
  tolerant readers.
- **Python `rfc8785` verified every Rust bundle byte for byte.**

`spike s7` and `s7/verify_bundles.py`. Raw data and every input:
[`spike-results/s7/`](spike-results/s7/).

**Determinism** (the 8 walkthrough `.proto` sets):

| Comparison | Result |
|---|---|
| protox 0.9.1, twice | identical bytes, 8/8 |
| protoc 3.21.12 (`--include_imports`) | **identical bytes**, 8/8 |
| `buf build --as-file-descriptor-set` (1.73.0) | different bytes (it keeps source info: 784 B against 265 B); **identical after normalization** (source info dropped, default `json_name` dropped), 8/8 |

The retention rule's identity check is that normalized comparison. A
rebuild whose bytes differ but which normalizes the same keeps the old
bundle.

**Protobuf** (14 old→new cases):
- **buf's WIRE_JSON rules re-implemented on prost-reflect** (`FIELD_NO_DELETE_UNLESS_*`, `FIELD_SAME_NAME`, `FIELD_SAME_JSON_NAME`, `FIELD_WIRE_JSON_COMPATIBLE_TYPE`/`_CARDINALITY`, `FIELD_SAME_ONEOF`, `RESERVED_*_NO_DELETE`, `ENUM_VALUE_*`) **agree with `buf breaking` on all 14 cases, in both orders.**
- **`buf` in both orders is not "both directions".** Adding a field or an enum value is breaking in the swapped order, because the swap reads it as a deletion. Deleting a field is breaking in the forward order only.
- **The zk2 classifier proposed for #618 is directional.** A reader reads a writer's data; unknown fields are ignored and missing fields defaulted. A change breaks when the same field's wire or JSON encoding diverges: its type, JSON name, cardinality, oneof membership, or an enum value's name. It also **detects renumbering**, which is a deletion plus an addition and silently drops the data in both directions. A field deleted without reserving its number is a **warning**: reuse is caught against the full history.

| Case | `buf` new vs old / old vs new | zk2, WIRE_JSON | zk2, WIRE only |
|---|---|---|---|
| add a field | ok / breaking | compatible | compatible |
| delete a field (unreserved) | breaking / ok | compatible + warning | compatible |
| renumber a field | breaking / breaking | **breaking** (data dropped) | **breaking** |
| rename a field | breaking / breaking | breaking (JSON name) | compatible |
| int32 → int64; string → bytes | breaking / breaking | breaking | breaking |
| scalar → repeated; move into a oneof | breaking / breaking | breaking | breaking |
| add an enum value | ok / breaking | breaking (old JSON readers lack the name) | compatible |
| rename / delete an enum value | breaking / breaking; breaking / ok | breaking | compatible |
| `json_name` option added | breaking / breaking | breaking | compatible |
| documentation only | ok / ok | compatible | compatible |

**For r4:** WIRE_JSON or WIRE only? zk2 protobuf payloads are binary
on the wire (D5), and tools decode with the *writer's* bundle, fetched by
fingerprint. JSON-name changes then never break a running system; they only
relabel a tool's display. The recommendation is **WIRE** semantics with
renumber detection for compatibility, and JSON-name and enum-name changes as
**review**. That also makes adding an enum value compatible, as it is for
binary readers.

**The zk2 JSON Schema subset** (r3 §3.9, D11). Its keywords are `type`,
`properties`, `required`, `additionalProperties`, `items`, `enum`, `const`,
`minimum`/`maximum`/`exclusive*`, `minLength`/`maxLength`,
`minItems`/`maxItems`, local `$ref`, and `oneOf` (with a discriminator).
Annotations are ignored: `$schema`, `$defs`, `$comment`, `title`,
`description`, `default`, `examples`, `format`, `deprecated`, `readOnly`,
`writeOnly`. Everything else (`pattern`, `if`/`then`, `allOf`,
`patternProperties`, …) is refused, because containment is not decidable for it.
Rule: a reader accepts every instance its writer produces, with **readers
tolerating unknown properties and writers sending only what their schema
declares**, as in protobuf. Full compatibility needs both directions.

| Case | zk2 | `jsoncompat` 0.4.2 (serializer / deserializer) |
|---|---|---|
| add an optional property | compatible | ok / ok |
| add an optional property to a closed schema | compatible (tolerant readers) | **breaking** / ok |
| remove an optional property | compatible (writers send only what they declare) | **breaking** / ok |
| add a required property; optional → required | breaking | ok / breaking |
| required → optional | breaking | breaking / ok |
| integer → number; number → integer | breaking | breaking / ok; ok / breaking |
| add / remove an enum value | breaking | breaking / ok; ok / breaking |
| tighten `maximum`; loosen `maxLength` | breaking | ok / breaking; breaking / ok |
| add a `oneOf` branch | breaking | breaking / ok |
| add a `pattern` | refused (outside the subset) | ok / breaking |
| documentation only | compatible | ok / ok |

The oracle caught one real gap in a first version of the rules: a removed
property definition lifts its type constraint in an open schema. Resolving it
led to the tolerant-reader rule above. The two remaining disagreements are
that rule, and they are deliberate.

**Cross-language canonical bytes.** Python 3.13's `rfc8785` 0.1.4 and
`hashlib` verified **24 of 24** example bundles. Each file is the JCS of its
own value, its fingerprint matches its name, and every schema matches its
id. The same held for **11 of 11** seeded canonical fixtures.

**The compatibility matrix** is handed to #607 as
[`compat-matrix.draft.json`](spike-results/s7/compat-matrix.draft.json): 53
draft cases (14 protobuf, 15 JSON Schema, 24 contract-metadata rows from
r3 §3.11 that await #618).

### S8 — deployment shapes (#604)
Done on paper: [`examples/zk2/shapes.md`](../../examples/zk2/shapes.md).
Natural `system`/`service` names exist for every shape examined.

### S14 — ACL on a live router (#616)

**Verdict:**
- **`default_permission: deny` with P3's three grant shapes works:** 13 of 13
  checks.
- **U21 is confirmed:** under `allow`, allow rules are not evaluated, and
  every unauthorized action got through.
- **D13 works:** compiling each grant into denies of its complement blocks
  them all, except wildcard puts, which R6 stops at the consumer.
- **Two generator rules were found the hard way** (below).

`spike s14`: one router with usrpwd authentication, 9 principals (the
control walkthrough, the tcgui pilot, offline commanding), and the generated
router configs. Raw data, including each posture's config:
[`spike-results/s14/`](spike-results/s14/).

| Check | deny + allows | allow + the same allows (U21) | allow + complement denies (D13) |
|---|---|---|---|
| teleop puts on its own cmd; the thruster receives it | yes | yes | yes |
| teleop puts on autopilot's cmd key (impersonation) | **blocked** | gets through | blocked |
| teleop puts on `zk2/vehicle-01/*/twist_cmd.v1/stream/cmd` | blocked | gets through | **gets through** (a wildcard is in no deny; R6 drops it) |
| teleop subscribes to the thruster's status (not granted) | blocked | gets through | blocked |
| the frontend sees the vehicle's presence (not granted) | 0 tokens | 4 tokens | 0 tokens |
| the frontend's fan-in GET `zk2/*/tc/tc.netif.v1/state/**` | 2 replies | 2 | 2 |
| the frontend's wildcard call to a fan-out-forbidden operation | 2 `fanout_forbidden` refusals, 0 executions | same | same |
| backend h1 puts on h2's state | blocked | gets through | blocked |
| vehicle-01's executor reads vehicle-02's plan | blocked | gets through | blocked |
| contract fetch; a client without credentials | works; refused | same | same |

**Generator rules for `zenctl acl gen` (#612):**
1. **own** `sys/svc` spells out every verbatim chunk, because `**` never
   crosses one: `zk2/sys/svc/**`, `…/*/@stream/**`, `…/*/@state/**`,
   `…/*/@op/**`, `zk2/sys/svc/@zk/**`.
   - **Ingress:** put, delete, declare_queryable, reply, liveliness_token.
   - **Egress:** query and declare_subscriber (a publisher must learn of interest).
2. **consume** keys.
   - **Ingress:** declare_subscriber, query, declare_liveliness_subscriber, liveliness_query.
   - **Egress:** put, delete, reply, liveliness_token.
3. **call** operation keys: ingress query; egress reply.
4. **Found here:** egress to a provider is checked against the query's or
   subscription's *own* key expression, by inclusion. A consumer's wildcard
   selector (`zk2/*/tc/...`) is not included in `zk2/h1/tc/**`, so the fan-in
   GET got **0 replies** until **every consumer selector that intersects a
   provider was added to that provider's egress grant**.
5. **Found here:** a reply to a wildcard query is checked against the query's
   key, and that includes a `fanout_forbidden` refusal. The same selectors
   must be granted for the provider's ingress *reply*.
6. **Contract bundles:** anyone may hold or fetch them on `zk2/@zk/contract/**`;
   the hash is the check.

A liveliness GET returns the querying session's own token twice, its local
copy and the router's. Tools count distinct keys.

**For r4:**
- **U14:** a SHOULD, as r3.2 leaned. Default deny gives P3's guarantees
  outright.
- **Under `allow`:** grants must be compiled into denies of their complement
  (D13), and regenerated on every contract revision. Wildcard injection then
  rests on R6 and O2.
- mTLS certificate-CN subjects were not run; they differ from usrpwd only in
  how a subject is named.
- **Router-to-router faces (from S3):**
  - these checks ran on one router with client principals;
  - between routers, a deny hides the denied declarations, but their key
    strings still cross the link;
  - an ACL is therefore not a confidentiality boundary for key names.

### S15 — constrained devices (#617)

**Verdict: a zenoh-pico 1.10.1 participant can be a full zk2 owner.**
- Tokens, descriptor, stream, state with S1–S3 timestamps and `reply_del`,
  and `complete` operations all work.
- A literal prefix replaces the session namespace.
- The constrained level relaxes the clock rule and the namespace, and it
  needs no gateway for bundles of this size.

`spike s15`, with the participant `s15/zk2_pico.c` (C, client mode, linked
against `libzenohpico.a` 1.10.1 built with its defaults). Raw data and the
source: [`spike-results/s15/`](spike-results/s15/).

| Case | Observed |
|---|---|
| Bring-up (queryables, then the instance token, then the interface token) | ready in 1 ms |
| Its tokens, parsed by `zenkey-model` | instance and `alive health.v1`: both parse |
| Its descriptor, by GET on the instance key | answered |
| Its heartbeat stream, 2 s | 19 samples, all stamped by the router (pico stamps nothing by itself) |
| A state GET (`All` + `Latest`) | the value, with the pico's own timestamp (`z_timestamp_new`: wall clock + a per-session bump, the pico's id) |
| After a delete | a `reply_del` with a timestamp |
| A `complete` operation | 1 reply |
| Replies of 1 KB, 4 KB, 8 KB, 64 KB, 100 KB | **all delivered** (100,000 B in 1 ms) |
| Pico + a storage, unconsolidated | both replies carry the pico's timestamp (the storage keeps it) |
| A pico using the literal prefix `dep1/zk2`, read by a Rust session in namespace `dep1` | 1 reply, seen as `zk2/…` |

**Corrections to the r3 facts:**
- **`Z_FRAG_MAX_SIZE` = 4096 bounds what a pico *receives*, not what it
  sends.** A Linux pico served 100 KB replies, so a gateway holder (S4) is
  needed for pico *consumers* of large data, and for microcontroller builds
  with smaller batches.
- **The pico's timestamps are not HLC**, but they are monotonic within a
  session and merge cleanly with a storage.

**The proposed constrained conformance level, for #606:**
- **Clock (U2):** wall clock with a per-session bump, instead of HLC MUST. The
  catch-up rule still applies: read the last stored timestamp of one's keys
  before the first write.
- **Namespace:** a deployment prefix is literal in keys.
- **Descriptor:** small enough for one fragment.
- **Bundles:** served by a gateway when the device cannot (receive limits).
- **Constrained faces** (zenoh-modem's radios) are the other half of the
  level. Liveness there is link freshness (D15), and S3 measures the costs.
