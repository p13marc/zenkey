# Contract retrieval scenarios (core §8.4)

Common setup: holders of the `camera.v1` bundle. The nearest holder sits on
the client's router, and others sit behind a second router.

## §1 One holder; many holders

**Expected.** With one holder, its valid reply is accepted. With 200 equal
holders on one router and `BestMatching`, one reply.

## §2 A slow, an unreachable, a corrupt nearest holder

**Steps.** The nearest holder delays 3 s; is frozen; then returns corrupt
bytes. Three good holders sit behind the second router.

**Expected.** In every case a valid bundle is accepted as soon as its reply
arrives (about 1 ms), without waiting for the GET to complete. The corrupt
reply is refused.

*Spike S4: the first valid reply arrived in 1.1–1.5 ms; waiting for
completion cost 1,001 ms.*

## §3 No valid holder; a constrained holder

**Steps.**
1. Every holder is corrupt.
2. The nearest holder can send only 4 KB, and the bundle is 83 KB. A gateway
   holder sits behind the second router.

**Expected.**
1. After the retry with `All`, the contract is reported unavailable. No
   unverified bundle is ever accepted.
2. The gateway's bundle is accepted.
