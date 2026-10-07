# S13 — simulation and replay (#596)

Written by `spike s13`; the latest run (`unix_s` 1791410763), zenoh 1.10.1. Code changes needed between cases: **none** (one detector binary; bindings and the clock are configuration).

| Case | Value | Pass |
|---|---|---|
| rebind input -> vehicle-01/cam-front (real.bindings.toml): frames from it / other sources / info state | 20 / 0 / 1 | yes |
| rebind input -> vehicle-01/replay-cam (replay-cam.bindings.toml): frames from it / other sources / info state | 20 / 0 / 1 | yes |
| rebind input -> sim-1/cam-front (sim.bindings.toml): frames from it / other sources / info state | 20 / 0 / 1 | yes |
| v1 zenctl used for record and replay | zenctl 0.11.0 (0.13.0-4-ge86be67) | yes |
| zenctl record zk2/vehicle-01/cam-front/camera.v1/@stream/image --for 3 | ok, 8122 bytes | yes |
| v1 zenctl replay --base replay: frames reaching a detector in namespace `replay` (expected 0: --base re-prefixes v1 keys only) | replay exit Some(0); frames received 0; keys [] | yes |
| namespace-aware replay (capture, then republish through a session in namespace `replay`): detector in `replay`, bound to the original address | 30 captured; 30 received; keys as the detector sees them ["zk2/vehicle-01/cam-front/camera.v1/@stream/image"] | yes |
| clock.v1 at speed 1: commander silent, 100 ms deadline on the bound clock | miss 101 ms of wall time after the last command (expected about 100, within a 10 ms clock tick) | yes |
| clock.v1 at speed 0: commander silent, 100 ms deadline on the bound clock | no miss in 3 s of wall time (clock paused) | yes |
| clock.v1 at speed 2: commander silent, 100 ms deadline on the bound clock | miss 55 ms of wall time after the last command (expected about 50, within a 10 ms clock tick) | yes |
