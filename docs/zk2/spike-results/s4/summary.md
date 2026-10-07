# S4 — contract retrieval by hash (#600)

Written by `spike s4`; the latest run (`unix_s` 1791411230), zenoh 1.10.1. Per-attempt timeout 1 s. `accept` is the time to the first valid reply; `complete` is when the GET completed (all routed holders answered or timed out).

| Case | Accepted | Attempts | Replies (invalid) | Bytes | accept ms | complete ms | Pass |
|---|---|---|---|---|---|---|---|
| one holder (camera.v1, 1.7 KB) | true | 1 | 1 (0) | 1761 | 1.0 | 1.1 | yes |
| 200 equal holders, BestMatching | true | 1 | 1 (0) | 1761 | 5.1 | 5.2 | yes |
| 200 equal holders, target All (for comparison) | true | 1 | 200 (0) | 352200 | 6.9 | 6.9 | yes |
| a slow nearest holder (3 s); 3 good behind router 2 | true | 1 | 2 (0) | 1761 | 1.3 | 1001.7 | yes |
| a corrupt nearest holder; 3 good behind router 2 | true | 1 | 2 (1) | 3522 | 1.5 | 1.5 | yes |
| an unreachable nearest holder (SIGSTOP after convergence; warm fetch 0.9 ms); 3 good behind router 2 | true | 1 | 2 (0) | 1761 | 1.1 | 1001.0 | yes |
| every holder corrupt (3) | false | 2 | 4 (4) | 7044 | 11.7 | 11.7 | yes |
| the holder three routers away | true | 1 | 1 (0) | 1761 | 1.4 | 1.4 | yes |
| a constrained nearest holder (4 KB limit) for zs.sysinfo.v1 (83688 B); a gateway holder behind router 2 | true | 1 | 1 (0) | 83688 | 6.4 | 8.7 | yes |
