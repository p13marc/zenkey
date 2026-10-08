# Constrained scenarios (core §1.6, R7, §8.5, §12)

Common setup: a vehicle router with 50 services, and a link to the ground
shaped as a serial line at 2,400 bit/s with a 200 ms RTT.

## §1 Attachment (§8.5)

**Steps.**
1. Link the routers router to router. Deny `zk2/**/@zk/**` on the link at
   both ends.
2. Instead, attach one ground session as a client of the vehicle router,
   with the same deny.

**Expected.**
1. The ground sees no token, but the vehicle's denied key strings still
   cross the link (about 11 KB per bring-up).
2. About 17 B crosses per bring-up, and a coverage gap costs about 0.5 KB.

*Spike S3: 11.1 KB against 17 B; a gap 11.8 KB against 536 B.*

## §2 A literal prefix (§1.6)

**Setup.** A zenoh-pico owner with the literal prefix `dep1/zk2/…`.

**Expected.** A session in namespace `dep1` reads its keys as base-relative
`zk2/…`.

*Spike S15.*

## §3 Static binding and unobservable liveness (R7)

**Steps.** The ground binds a vehicle state statically. Nothing crosses the
face for an hour.

**Expected.**
- The binding resolves without presence, and the ground's GET is answered
  whenever the link is up.
- While nothing crosses, a tool reports the provider's liveness as
  *unobservable*, not as down.

## §4 Batches and the lease (§8.5)

**Steps.** Bring up 100 tokens over the link, with zenoh's default batch
size, then with `batch_size = 1024`.

**Expected.** With the default, the link reconnects before the batch
crosses. With 1 KB batches, presence arrives at link speed: about 7 KB in
24 s, with no reconnect.

*Spike S3: default 85 s with 4 reconnects in the probe, never in the full
profile; 1 KB 24 s.*

## §5 A one-fragment descriptor (§3.3, §12)

**Setup.** A zenoh-pico owner with the default `Z_FRAG_MAX_SIZE` of 4 KB.

**Expected.** Its descriptor is under 4 KB, and a GET of its instance key
returns it whole.

*Spike S15: the pico's descriptor was answered.*
