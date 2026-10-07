# S1 — grammar basics (#597)

Written by `spike s1`; the latest run (`unix_s` 1791409239), zenoh 1.10.1.

| Group | Case | Expected | Observed | Pass |
|---|---|---|---|---|
| roundtrip | 24 contracts: build, parse, resolve, unslug (values incl. ETH0, a/b, 10.0.0.1, über, -) | 0 failures | 0 failures of 2226 keys; 0 shadowed by a more literal template (D1) | yes |
| guard | zk2/<system>/** over 2030 plain and 196 verbatim/control keys | 0 violations | 0 violations | yes |
| guard | zk2/** over the same keys | 0 violations | 0 violations | yes |
| guard | live: subscriber zk2/g/** over puts on stream, state, events, @stream, @state | zk2/g/svc/i.v1/events/x/01j9zk6q6x5m2f4a8c0d3e7b9h zk2/g/svc/i.v1/state/x zk2/g/svc/i.v1/stream/x | zk2/g/svc/i.v1/events/x/01j9zk6q6x5m2f4a8c0d3e7b9h zk2/g/svc/i.v1/state/x zk2/g/svc/i.v1/stream/x | yes |
| guard | live: GET zk2/g/** reaches state, never @op | zk2/g/svc/i.v1/state/x | zk2/g/svc/i.v1/state/x | yes |
| guard | live: liveliness GET zk2/g/** sees no @zk token | none | none | yes |
| guard | live: liveliness GET zk2/g/*/@zk/** names it | zk2/g/svc/@zk/instance/0123456789abcdef | zk2/g/svc/@zk/instance/0123456789abcdef | yes |
| namespace | stream: same-namespace subscriber sees the key stripped | zk2/ns/svc/i.v1/stream/x | zk2/ns/svc/i.v1/stream/x | yes |
| namespace | stream: un-namespaced subscriber sees the key prefixed | dep1/zk2/ns/svc/i.v1/stream/x | dep1/zk2/ns/svc/i.v1/stream/x | yes |
| namespace | stream: un-namespaced dep1/zk2/** (guard under a prefix) | dep1/zk2/ns/svc/i.v1/stream/x | dep1/zk2/ns/svc/i.v1/stream/x | yes |
| namespace | stream: another namespace sees nothing | none | none | yes |
| namespace | @stream: same-namespace subscriber sees the key stripped | zk2/ns/svc/i.v1/@stream/x | zk2/ns/svc/i.v1/@stream/x | yes |
| namespace | @stream: un-namespaced subscriber sees the key prefixed | dep1/zk2/ns/svc/i.v1/@stream/x | dep1/zk2/ns/svc/i.v1/@stream/x | yes |
| namespace | @stream: un-namespaced dep1/zk2/** (guard under a prefix) | none | none | yes |
| namespace | @stream: another namespace sees nothing | none | none | yes |
| namespace | state: same-namespace subscriber sees the key stripped | zk2/ns/svc/i.v1/state/x | zk2/ns/svc/i.v1/state/x | yes |
| namespace | state: un-namespaced subscriber sees the key prefixed | dep1/zk2/ns/svc/i.v1/state/x | dep1/zk2/ns/svc/i.v1/state/x | yes |
| namespace | state: un-namespaced dep1/zk2/** (guard under a prefix) | dep1/zk2/ns/svc/i.v1/state/x | dep1/zk2/ns/svc/i.v1/state/x | yes |
| namespace | state: another namespace sees nothing | none | none | yes |
| namespace | @state: same-namespace subscriber sees the key stripped | zk2/ns/svc/i.v1/@state/x | zk2/ns/svc/i.v1/@state/x | yes |
| namespace | @state: un-namespaced subscriber sees the key prefixed | dep1/zk2/ns/svc/i.v1/@state/x | dep1/zk2/ns/svc/i.v1/@state/x | yes |
| namespace | @state: un-namespaced dep1/zk2/** (guard under a prefix) | none | none | yes |
| namespace | @state: another namespace sees nothing | none | none | yes |
| namespace | @op: same-namespace call, reply key stripped | zk2/ns/svc/i.v1/@op/f | zk2/ns/svc/i.v1/@op/f | yes |
| namespace | @op: un-namespaced call on the prefixed key | dep1/zk2/ns/svc/i.v1/@op/f | dep1/zk2/ns/svc/i.v1/@op/f | yes |
| namespace | @op: another namespace gets no reply | none | none | yes |
| namespace | @zk: same-namespace liveliness GET zk2/*/*/@zk/instance/* | zk2/ns/svc/@zk/instance/0123456789abcdef | zk2/ns/svc/@zk/instance/0123456789abcdef | yes |
| namespace | @zk: un-namespaced liveliness GET dep1/zk2/*/*/@zk/instance/* | dep1/zk2/ns/svc/@zk/instance/0123456789abcdef | dep1/zk2/ns/svc/@zk/instance/0123456789abcdef | yes |
| namespace | @zk: another namespace sees no token | none | none | yes |
| adv | stream, no namespace: (a) history from a publisher already present (initial query, no key parsing) | 5 | 5 | yes |
| adv | stream, no namespace: @adv tokens seen, and refused by the zk2 parser | 1 token, refused | 1 token, refused | yes |
| adv | stream, no namespace: (b) late-joining publisher, detected by parsing its @adv token | 5 | 5 | yes |
| adv | state, no namespace: (a) history from a publisher already present (initial query, no key parsing) | 5 | 5 | yes |
| adv | state, no namespace: @adv tokens seen, and refused by the zk2 parser | 1 token, refused | 1 token, refused | yes |
| adv | state, no namespace: (b) late-joining publisher, detected by parsing its @adv token | 5 | 5 | yes |
| adv | @stream, no namespace: (a) history from a publisher already present (initial query, no key parsing) | 5 | 5 | yes |
| adv | @stream, no namespace: @adv tokens seen, and refused by the zk2 parser | 1 token, refused | 1 token, refused | yes |
| adv | @stream, no namespace: (b) late-joining publisher, detected by parsing its @adv token | 0 | 0 | yes |
| adv | stream, namespace dep1: (a) history from a publisher already present (initial query, no key parsing) | 5 | 5 | yes |
| adv | stream, namespace dep1: @adv tokens seen, and refused by the zk2 parser | 1 token, refused | 1 token, refused | yes |
| adv | stream, namespace dep1: (b) late-joining publisher, detected by parsing its @adv token | 5 | 5 | yes |
| adv | state, namespace dep1: (a) history from a publisher already present (initial query, no key parsing) | 5 | 5 | yes |
| adv | state, namespace dep1: @adv tokens seen, and refused by the zk2 parser | 1 token, refused | 1 token, refused | yes |
| adv | state, namespace dep1: (b) late-joining publisher, detected by parsing its @adv token | 5 | 5 | yes |
| adv | @stream, namespace dep1: (a) history from a publisher already present (initial query, no key parsing) | 5 | 5 | yes |
| adv | @stream, namespace dep1: @adv tokens seen, and refused by the zk2 parser | 1 token, refused | 1 token, refused | yes |
| adv | @stream, namespace dep1: (b) late-joining publisher, detected by parsing its @adv token | 0 | 0 | yes |
| wildcard-put | stream: put on zk2/w/svc/i.v1/stream/* reaches a concrete subscriber, which sees the wildcard key | zk2/w/svc/i.v1/stream/* | zk2/w/svc/i.v1/stream/* | yes |
| wildcard-put | @stream: put on zk2/w/svc/i.v1/@stream/* reaches a concrete subscriber, which sees the wildcard key | zk2/w/svc/i.v1/@stream/* | zk2/w/svc/i.v1/@stream/* | yes |
| wildcard-put | R6 filter cost: is_wild() on a 66-byte concrete key, 10000000 checks | 0 wild | 0 wild; 11.15 ns/check | yes |
