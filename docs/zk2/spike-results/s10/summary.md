# S10 — wildcard bindings (#593)

Written by `spike s10`; the latest run (`unix_s` 1791410554), zenoh 1.10.1.

| Phase | Measure | Value | Expected | Pass |
|---|---|---|---|---|
| start-up | R5: wait for the first bound provider: token seen, then its first sample (ms after spawn) | 11 / 12 | both seen | yes |
| fan-in | 1 producers: tokens seen / producers delivering | 1 / 1 | 1 / 1 | yes |
| fan-in | 1 producers: join (token) to first sample, ms min/median/max | 1.2 / 1.2 / 1.2 | measured | yes |
| fan-in | 10 producers: tokens seen / producers delivering | 10 / 10 | 10 / 10 | yes |
| fan-in | 10 producers: join (token) to first sample, ms min/median/max | 0.2 / 11.1 / 24.1 | measured | yes |
| fan-in | 100 producers: tokens seen / producers delivering | 100 / 100 | 100 / 100 | yes |
| fan-in | 100 producers: join (token) to first sample, ms min/median/max | 0.2 / 89.0 / 295.2 | measured | yes |
| fan-in | consumer process RSS growth for 100 producers, KiB | 664 | measured | yes |
| steady | 100 producers x 10 Hz for 5 s: samples received | 5000 | about 5,000 | yes |
| steady | sequence gaps (missed samples) | 0 | 0 | yes |
| steady | duplicates | 0 | 0 | yes |
| wrong-interface | samples of nav.v2 on vehicle-01 delivered to the detections.v1 binding | 0 | 0 | yes |
| churn | clean exit (SIGINT): leave detected for 5/5; ms min/median/max | 4 / 6 / 8 | 5/5 detected | yes |
| churn | kill -9: leave detected for 5/5; ms min/median/max | 3 / 3 / 7 | 5/5 detected | yes |
| churn | network cut (SIGSTOP; only the lease can tell): leave detected for 5/5; ms min/median/max | 9961 / 10002 / 10047 | 5/5 detected | yes |
| churn | after SIGCONT: producers back within 20 s (ms) | 5/5 in 52 | measured | yes |
| churn | the 100 long-lived producers still delivering after the churn | 100 | 100 | yes |
| system-wildcard | */tc on 2 hosts: systems answering a state GET / interface tokens | 2 / 2 | 2 / 2 | yes |
| system-wildcard | */tc on 5 hosts: systems answering a state GET / interface tokens | 5 / 5 | 5 / 5 | yes |
| injection | 10 puts on zk2/vehicle-01/*/detections.v1/stream/objects: discarded by the R6 filter | 10 | 10 | yes |
| graph | edges drawn from descriptors + interface tokens = producers actually delivering | 100 drawn, 100 delivering, 0 differ | 0 differ | yes |
| graph | descriptor bytes with one wildcard binding (requires + bindings) | 222 | measured | yes |
| graph | binding records found in all descriptors | 1 | 1 | yes |
