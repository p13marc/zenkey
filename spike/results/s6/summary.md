# S6 — operations and ownership (#602)

Written by `spike s6`; the latest run (`unix_s` 1791411480), zenoh 1.10.1. Calls time out after 1 s.

| Group | Case | Value | Pass |
|---|---|---|---|
| split-brain | 200 concrete BestMatching calls, both on the client's router: executions A / B, duplicated calls, replies | 200 / 0 / 0 / 200 | yes |
| split-brain | both on the client's router: the token check finds | v/nav nav.v2: 2 instances | yes |
| split-brain | 200 concrete BestMatching calls, the second behind router 2: executions A / B, duplicated calls, replies | 200 / 200 / 200 / 400 | yes |
| split-brain | the second behind router 2: the token check finds | v/nav nav.v2: 2 instances | yes |
| replicated | 100 calls over 3 replicas: executions per replica | r1 (near) 100, r2 100, r3 0 | yes |
| replicated | the nearest replica crashes (kill -9): failed calls, then the first success at | 0 failed; first success 3.4 ms after the kill | yes |
| replicated | the nearest replica hangs: calls answered (by the far replica), median ms to the first reply | 10/10, 1.3 ms | yes |
| templated | a concrete call to h2's templated `interfaces/*/*/set`: replies, executions h1/h2/h3 | 1 (0 errors); [0, 1, 0] | yes |
| fan-in | fan-in `zk2/*/tc/tc.netif.v1/@op/diagnostics`, target All, over exact-key complete queryables: replies, executions | 3; [1, 1, 1] | yes |
| fan-out | a non-concrete call (one host, wildcard interface) to a fanout = forbidden operation: values, refusals, executions | 0 values; 1 refusals (1 at the servers), all `fanout_forbidden`: true; 0 executions | yes |
| fan-out | a non-concrete call (every host) to a fanout = forbidden operation: values, refusals, executions | 0 values; 3 refusals (3 at the servers), all `fanout_forbidden`: true; 0 executions | yes |
| many-replies | replies = many (2 servers x 5 replies on one key), target All: replies kept per consolidation | None 10, Monotonic 10, Latest 1, Auto 1 | yes |
| partition | server frozen (SIGSTOP): calls that waited out the 1 s timeout, calls that failed at once, and when (s) failing fast began | 9 timeouts, 16680 fast failures; fast from 9.6 s (the 10 s lease) | yes |
| partition | after SIGCONT: the first successful call at | 22 ms | yes |
| standby | active + standby (instance token only): instances seen, split-brain findings, executions of 20 calls by the active | 2; none; 20 | yes |
| metadata | call metadata as a request attachment: seen by the server, echoed on the reply | true, true | yes |
