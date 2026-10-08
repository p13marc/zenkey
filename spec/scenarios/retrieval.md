# Contract retrieval scenarios (core §8.4)

Common setup: holders of the `camera.v1` bundle. The nearest holder sits on
the client's router, and others sit behind a second router.

## §1 One holder; many holders

**Expected.** With one holder, its valid reply is accepted, with `Encoding`
`application/json`, which the caller does not depend on. With 200 equal
holders on one router and `BestMatching`, one reply. From a session that
holds the bundle itself, `BestMatching` gives two: its own holder's and the
nearest remote one's.

*Measured with zenoh-python 1.10.1 (#609), a client of the owner's router
holding a queryable of its own: two replies to `BestMatching`.*

## §2 A slow, an unreachable, a corrupt nearest holder

**Steps.** The nearest holder delays 3 s; is frozen; then returns corrupt
bytes. Three good holders sit behind the second router.

**Expected.** In every case, with consolidation `None`, a valid bundle is
accepted as soon as its reply arrives (about 1 ms), without waiting for the
GET to complete. The corrupt reply is refused. With zenoh's default
consolidation, a slow corrupt reply displaces the valid one, which is why
core §8.4 requires `None`.

*Spike S4: the first valid reply arrived in 1.1–1.5 ms; waiting for
completion cost 1,001 ms.*

*Measured with zenoh-python 1.10.1 (#609): a corrupt 12-byte reply after
2 s, a valid 4,800-byte one at once. Default consolidation, either target:
the corrupt reply alone, at 2.0 s. Consolidation `None`, either target: the
valid reply at 0.001 s, then the corrupt one.*

## §3 No valid holder; a constrained holder

**Steps.**
1. Every holder is corrupt.
2. The nearest participant is a constrained device that cannot receive or
   hold the 83 KB bundle, so it holds none. A gateway holder sits behind
   the second router.

**Expected.**
1. After the retry with `All`, the contract is reported unavailable. No
   unverified bundle is ever accepted.
2. The gateway's bundle is accepted.
