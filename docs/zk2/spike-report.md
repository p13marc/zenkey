# zk2 spike report

**Status: in progress.** The spike measures r3's claims before the core is
specified (epic #585: "measure the network half before specifying it"). Each
section below is filled by its issue. The decisions table is settled in #605,
which produces r4.

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
| Code | Branch `zk2-spike`, `spike/` (never merged): `zk2rt`, a throwaway runtime over `zenkey-model`, and the `spike` harness |
| Raw data | `spike/results/<group>/` on `zk2-spike`, copied here to [`spike-results/`](spike-results/) by each docs PR |

## Decisions

Settled in #605. The leans are r3.3 §5's; the verdicts come from the
sections below.

| Item | Question | Lean (r3.3) | Spike | Verdict |
|---|---|---|---|---|
| U-A | P3 or P2 for control and commanding | **Decided: P3** | S11, S12 | pending (confirmation) |
| U-D | `@stream` as a kind token | Token | S3, S9 | pending |
| U-E | QoS defaults per pattern; `priority` in the core | As in §3.3 | S9, S11 | pending |
| U-F | Decodability: two blessed kinds + others | As in §3.9 | S9 | pending |
| U-G | Many-reply operations in the core | Yes (O6) | S6 | pending |
| U1 | State: producer + storage merged by timestamp | Merge | S5 | pending |
| U2 | Clock discipline | HLC MUST + hold writes past the last stored timestamp | S5 | pending |
| U3 | Tombstone window | Core default, annotation override | S5 | pending |
| U4 | `complete` operation queryables | Keep | S6 | pending |
| U5 | Token layout | Instance + interface tokens | S2 | pending |
| U6 | Mandatory one-chunk `system` | Keep | S8 | on paper: holds ([`examples/zk2/shapes.md`](../../examples/zk2/shapes.md), #604) |
| U7 | Bundle stability | Build once, embed, retention rule | S7 | pending |
| U10 | Large collections | No paging in the core | S5 | pending |
| U11 | Descriptor dynamics | Put + GET | S2 | pending |
| U12 | Redundancy | Diagnose only | S6 | pending |
| U13 | A constrained conformance level | Define it | S15 | pending |
| U14 | `default_permission: deny` as a MUST | SHOULD | S14 | pending |
| U15 | The storage-manager position | With U1 | S5 | pending |
| U16, U19 | The `events` kind token | Yes (D3) | S5 | pending |
| U18 | Device-as-service at SNMP scale | Device-as-service | S2 | pending |
| U20 | `@state` for large populations | Yes (D3) | S2, S5 | pending |
| U21 | Allow rules under `default_permission: allow` | Not evaluated; compile to denies | S14 | pending |

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
*Pending.* Matrix: 1 kHz control messages; 4 MB frames at 30 Hz, with and
without SHM; typed handles against raw zenoh. Measures: latency, p99 jitter,
throughput, CPU, allocations per sample. Decides U-D, U-E, U-F.

### S10 — wildcard bindings (#593)
*Pending.* 1 → 100 producers bound by `vehicle-01/*`, joining and leaving
through presence. Measures: fan-in correctness, join latency, memory.
Decides U-A, R1, R5.

### S11 — control arbitration (#594)
*Pending.* Teleop, autopilot and safety on priority + deadline; a commander
crash; the dead-man stop. Measures: switch-over latency, missed-deadline
detection, false stops. Decides U-A, U-E.

### S12 — store-and-forward (#595)
*Pending.* `desired.v1` across a disconnect and a router restart, with
storage on the ground and on the vehicle, and clock skew. Measures:
convergence time, stale-command rejection, wrong answers. Decides U-A, U1,
U2.

### S13 — simulation and replay (#596)
*Pending.* Rebinding sources to a simulator and to a replay with no code
change; simulated time through `clock.v1`. Measures: pass/fail, and
timing-profile behaviour under simulated time.

## Protocol spikes (EY, FA)

### S1 — grammar basics (#597)
*Pending.* Round trips; namespaces × verbatim chunks; an advanced publisher
on `zk2/…`; wildcard puts and the consumer-side filter; the key-algebra
guard.

### S2 — liveliness scaling (#598)
*Pending.* 100 / 1k / 10k / 50k tokens; layouts; 2+ routers; the ZenSight
shape. Measures: RSS, CPU, bytes, discovery latency, convergence, churn.
Decides U5, U11, U18, U20.

### S3 — constrained links (#599)
*Pending.* netem through tcgui, plus zenoh-modem's link classes. Measures:
presence replay, descriptor and bundle fetch, time to first view.

### S4 — contract serving (#600)
*Pending.* One, many, slow, unreachable and corrupt holders.

### S5 — state correctness (#601)
*Pending.* Producer, storage, both, or stale; a restart with the clock
behind; `reply_del`; wildcard GET at 1k and 10k; events storage. Decides U1,
U2, U3, U10, U15, U16, U19, U20.

### S6 — operations and ownership (#602)
*Pending.* Two owners; replicated serving; failover; partition; replies =
many. Decides U4, U12, U-G.

### S7 — bundle stability and classifier feasibility (#603)
*Pending.* protox/protoc/buf determinism; the retention rule; WIRE_JSON on
prost-reflect against `buf`; the JSON Schema subset; Python `rfc8785`
against Rust canonical bytes. Decides U7. A starting point: `zenkey-model`'s
conformance fixtures already pin JCS canonical bytes for JSON Schema and
raw contracts, and state that protobuf artifacts are protox's encoding
(`spec/conformance/README.md`).

### S8 — deployment shapes (#604)
Done on paper: [`examples/zk2/shapes.md`](../../examples/zk2/shapes.md).
Natural `system`/`service` names exist for every shape examined.

### S14 — ACL on a live router (#616)
*Pending.* The pilot and walkthrough rule sets under
`default_permission: deny`; wildcard-put injection; fan-out refusal; allow
rules under `allow`. Decides U14, U21.

### S15 — constrained devices (#617)
*Pending.* A zenoh-pico participant as an owner against a router. Decides
U13.
