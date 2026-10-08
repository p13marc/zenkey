# S15 — constrained devices (#617)

Written by `spike s15`; the latest run (`unix_s` 1791415443). A zenoh-pico 1.10.1 participant (`s15/zk2_pico.c`, client mode) against a zenoh 1.10.1 router.

| Case | Observed |
|---|---|
| the pico participant opens a client session and brings itself up | ready in 1 ms |
| its tokens, parsed by zenkey-model | alive health.v1, instance |
| its descriptor, by GET on the instance key | {"instance":"00065d49a9377f6d","interfaces":[{"iface":"health.v1"}],"meta":{"build":"zenoh-pico 1.10.1"},"service":"vehicle-01/pico"} |
| its heartbeat stream: samples in 2 s, how many carry a timestamp | 19 samples, 19 stamped (the router stamps puts; pico stamps nothing by itself) |
| a state GET (All + Latest) | value {"state":"ok"}; timestamp id 29bde7459accad2ee8f3656c4f4be402  |
| a call to the complete @op/reset | 1 replies |
| after @op/toggle deleted the state: the GET | a delete (reply_del) with a timestamp |
| a 1024 B reply from the pico | 1024 B received in 0 ms (0 errors) |
| a 4096 B reply from the pico | 4096 B received in 0 ms (0 errors) |
| a 8192 B reply from the pico | 8192 B received in 0 ms (0 errors) |
| a 65536 B reply from the pico | 65536 B received in 2 ms (0 errors) |
| a 100000 B reply from the pico | 100000 B received in 1 ms (0 errors) |
| pico producer + a storage, unconsolidated replies to a state GET | {"state":"ok"} ts 29bde7459accad2ee8f3656c4f4be402 \| {"state":"ok"} ts 29bde7459accad2ee8f3656c4f4be402 |
| a pico with the literal prefix dep1/zk2, read by a Rust session in namespace dep1 | 1 replies, key as seen: ["zk2/vehicle-01/pico2/health.v1/state/status"] |
