# S9 — the typed layer's cost (#592)

Written from `s9.csv` (3 interleaved repetitions, zenoh 1.10.1). Latencies in µs: the median across repetitions, with the p99 range. `recv` is received/sent over all repetitions; `shm` counts samples that arrived as SHM; allocations are per message.

| Case | p50 | p99 (min–max) | p99.9 | recv/sent | recv allocs | send allocs | shm |
|---|---|---|---|---|---|---|---|
| ctrl CtrlRaw 100 Hz Default | 403 | 3172 (2807–4028) | 4957 | 1500/1500 | 3.1 | 4.1 | 0 |
| ctrl CtrlRaw 100 Hz RealTime | 419 | 3138 (1638–3325) | 5585 | 1500/1500 | 3.1 | 4.1 | 0 |
| ctrl CtrlProto 100 Hz Default | 393 | 2893 (1772–3059) | 4863 | 1500/1500 | 3.1 | 4.1 | 0 |
| ctrl CtrlProto 100 Hz RealTime | 348 | 1865 (1682–3627) | 4695 | 1500/1500 | 3.1 | 4.1 | 0 |
| ctrl CtrlRaw 1000 Hz Default | 212 | 3911 (2791–4480) | 6980 | 15000/15000 | 2.9 | 4.0 | 0 |
| ctrl CtrlRaw 1000 Hz RealTime | 217 | 3475 (2832–4310) | 8444 | 14912/15000 | 3.0 | 4.0 | 0 |
| ctrl CtrlProto 1000 Hz Default | 182 | 2269 (1030–4757) | 5513 | 15000/15000 | 3.0 | 4.0 | 0 |
| ctrl CtrlProto 1000 Hz RealTime | 203 | 4950 (1556–7061) | 10295 | 14860/15000 | 3.0 | 4.0 | 0 |
| ctrl CtrlRaw 5000 Hz Default | 160 | 2205 (449–3697) | 5833 | 60000/60000 | 2.8 | 3.9 | 0 |
| ctrl CtrlRaw 5000 Hz RealTime | 172 | 3315 (2714–4103) | 5845 | 57599/60000 | 3.0 | 4.0 | 0 |
| ctrl CtrlProto 5000 Hz Default | 153 | 2130 (2119–5190) | 7117 | 60000/60000 | 2.8 | 3.9 | 0 |
| ctrl CtrlProto 5000 Hz RealTime | 155 | 1504 (1452–5106) | 3342 | 57879/60000 | 3.0 | 4.0 | 0 |
| ctrl CtrlProto 1000 Hz Default on @stream | 198 | 1950 (1345–3296) | 4208 | 15000/15000 | 3.0 | 4.0 | 0 |
| ctrl CtrlProto 1000 Hz Default under 4 MB@30 Hz | 203 | 2331 (1594–7046) | 5698 | 15000/15000 | 2.9 | 9.0 | 0 |
| ctrl CtrlProto 1000 Hz RealTime under 4 MB@30 Hz | 200 | 2390 (1611–5140) | 5236 | 14868/15000 | 3.0 | 9.0 | 0 |
| frame FrameRaw Router shm Off | 4989 | 19425 (15501–19810) | 21311 | 450/450 | 351.3 | 167.6 | 0 |
| frame FrameProto Router shm Off | 5722 | 16014 (14254–56091) | 23689 | 450/450 | 352.3 | 169.8 | 0 |
| frame FrameFb Router shm Off | 5320 | 14852 (10863–26998) | 15649 | 450/450 | 351.3 | 167.8 | 0 |
| frame FrameRaw Peer shm Off | 2824 | 16689 (10143–17494) | 20160 | 450/450 | 355.6 | 168.6 | 0 |
| frame FrameProto Peer shm Off | 3057 | 11573 (7498–30354) | 26512 | 450/450 | 356.6 | 169.9 | 0 |
| frame FrameFb Peer shm Off | 3182 | 10268 (8088–26712) | 13703 | 450/450 | 355.6 | 168.5 | 0 |
| frame FrameRaw Peer shm ImplicitDefault | 2528 | 15812 (11789–20421) | 15829 | 450/450 | 356.0 | 168.9 | 0 |
| frame FrameRaw Peer shm Implicit6MiB | 716 | 2884 (1917–4888) | 5660 | 450/450 | 12.5 | 9.5 | 447 |
| frame FrameRaw Peer shm Explicit6MiB | 273 | 2027 (1864–2945) | 2279 | 448/450 | 10.2 | 6.3 | 448 |
| frame FrameProto Peer shm Explicit6MiB | 1236 | 3377 (2735–5050) | 4252 | 450/450 | 11.2 | 8.3 | 450 |
| frame FrameFb Peer shm Explicit6MiB | 786 | 3347 (2575–3953) | 5181 | 450/450 | 10.1 | 6.3 | 450 |

## Micro-benchmarks (`micro.csv`)

| Measure | Value |
|---|---|
| encoded bytes: protobuf | 29.0 |
| encoded bytes: json | 99.0 |
| encoded bytes: cbor | 72.0 |
| ns encode+decode: protobuf (prost) | 126.1 |
| ns encode+decode: json (serde_json) | 894.1 |
| ns encode+decode: cbor (ciborium) | 1046.2 |
| ns twist() itself (stamp + build) | 43.1 |
| ns put, 64 B, no subscriber | 278.7 |
| ns put + new_timestamp() (the state writer), 64 B | 272.5 |
