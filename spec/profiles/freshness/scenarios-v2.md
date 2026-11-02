# `freshness.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. The reference runtime runs §1
to §7 (`zenkey/tests/profile_freshness_v2.rs`, one test per section,
named after it). §2's step 6 has a test of its own,
`s2_a_candidates_held_put_clears_its_bit`.

**Conventions.**
- **Section numbers** in parentheses (§2.4) are v2.md's rules; core
  sections say "core", and `link.v1`'s say so.
- **The bus.** R1 is a router with timestamping on, as a router's default
  is. The owner, the subscribers S and the GET reader G are its clients,
  unless a section says otherwise. The owner's session runs with
  timestamping off (core §8.2). Timeouts are 1 s unless stated.
- **The contract.** Every section uses `beacon.v2`. The reference keeps
  it as `zenkey/tests/contracts/beacon.v2.toml`:

  ```toml
  [interface]
  name  = "beacon"
  major = 2
  minor = 0
  uses  = ["freshness.v2", "link.v1"]

  [resources.status]                         # one member: a heartbeat every 0.95 s
  kind        = "state"
  type        = { raw = "text/plain" }
  annotations = { "freshness.ttl_s" = 2, "link.exposure" = "link" }

  [resources."peer/{peer}"]                  # templated, best_effort under loss
  kind        = "state"
  type        = { raw = "text/plain" }
  params      = { peer = "string" }
  cardinality = 8
  reliability = "best_effort"
  congestion  = "drop"
  annotations = { "freshness.ttl_s" = 4 }

  [resources.route]                          # behind a capability
  kind        = "state"
  type        = { raw = "text/plain" }
  optional    = true
  gate        = ["capability:routing"]
  annotations = { "freshness.ttl_s" = 4 }

  [resources.intent]                         # never stale: no collection
  kind        = "state"
  type        = { raw = "text/plain" }
  annotations = { "freshness.ttl_s" = 0 }

  [resources.note]                           # no horizon: no collection
  kind = "state"
  type = { raw = "text/plain" }

  [resources.level]                          # a stream, ttl 1 s
  kind        = "stream"
  type        = { raw = "text/plain" }
  annotations = { "freshness.ttl_s" = 1, "link.exposure" = "link" }
  ```

  Its collections, in map order, are `state/peer/{peer}`, `state/route`
  and `state/status`. The shortest horizon is 2 s, so the heartbeats are
  at most 1 s apart. `link.exposure` marks `status` and `level` to cross
  a face (`link.v1` §2.2). So the contract lists `link.v1`. Only §7 runs
  a face.
- **The owner** is `lab/beacon`, implementing `beacon.v2` with every
  resource exposed but `route`: it does not hold the capability
  `routing`. Its heartbeat key is
  `zk2/lab/beacon/freshness.v2/stream/heartbeat/beacon.v2`.
- **H** subscribes to the heartbeat key alone, and reads each heartbeat
  with `beacon.v2`'s collections (core §7.5).
- **S** subscribes to `state/status` or `state/peer/*` of `lab/beacon`, and
  to its heartbeat key. It records each delivery's arrival on its
  monotonic clock. "S judges" applies v2.md §2.5 at that instant.
- **G** GETs the owner's state keys as core S4 says. "G judges" applies
  v2.md §2.6 to the reply, the delta at 500 ms, on the deployment's word:
  every session runs on one host.
- **Jitter.** The reference allows 200 ms on loopback.

## §1 The cadence, and the bits (§2.4)

**Steps.**
1. The owner puts `status` = `up`, `peer/a` = `1` and `peer/b` = `1`,
   then nothing for 5 s. H and S listen.
2. The owner clears `peer/b`'s bit, and H listens 2 s.
3. The owner deletes `peer/a`, and H listens 2 s.
4. The owner closes its service, and H listens 2 s.

**Expected.**
- **The descriptor** lists `beacon.v2` and `freshness.v2`.
- **The cadence.** H hears a heartbeat at least every 1 s plus jitter,
  from the owner's tokens on, each `CH` of form 6 with `beacon.v2`'s tag,
  their seqs consecutive within one run.
- **The map.** Each body's map is `1 0 1`: `route` is not exposed.
- **The bits** (step 1). `status`'s section vouches for it, its digest the
  digest of `status`'s stamp. `peer/{peer}`'s holds 2 live members, both
  vouched for, its digest over both stamps.
- **No re-put.** S hears each put once, and nothing else on `status` or
  `peer/*` in 5 s. S judges `status` fresh 5 s after its put, more than
  its horizon: the heartbeats confirmed it.
- **A cleared bit** (step 2). The next heartbeats vouch for rank 0,
  `peer/a`, and not for `peer/b`, still live.
- **A delete** (step 3). The next heartbeats hold 1 live member, `peer/b`,
  the digest its stamp's alone.
- **The stop** (step 4). H hears no heartbeat after the owner's tokens
  went.

## §2 A vouch withdrawn goes stale (§2.4, §2.5, §2.10)

**Steps.**
1. The owner puts `status` = `up`. S listens 3 s.
2. The owner clears `status`'s bit. S listens 4 s.
3. The owner sets it again. S listens 2 s.
4. A second owner, `lab/ahead`, its clock 2 s ahead of R1's, watches a key
   that a client of R1 puts, stamped by R1. It puts `status` = `up`. Once
   its guard holds, it puts `status` = `still up`. S2, a subscriber of
   `lab/ahead`'s `status` and heartbeat, listens 4 s, and H2 reads its
   heartbeats. Then G GETs `lab/ahead`'s `status`.
5. The first owner closes its service. S listens 3 s.
6. A candidate, `lab/candidate`, serves `beacon.v2` at minor 1. Its
   `uses` adds `redundancy.v1`, and `status` is its only resource. The
   reference keeps it as `zenkey/tests/contracts/beacon-governed.v2.toml`.
   Its session stamps every write (core §8.2). Its clock is 2 s ahead of
   R1's. It watches the key of step 4, and puts `status` = `up`. Once
   its guard holds, it puts `status` = `still up`, and H3 reads its
   heartbeats 2.5 s. Then its clock is right again, and once its guard
   releases, H3 reads 2.5 s more. Then it puts `status` = `still up`
   again.

**Expected.**
- **Step 1.** S judges `status` fresh at 3 s, past its horizon.
- **Step 2.** S judges `status` fresh just after the bit is cleared, never
  stale at once. It judges it stale more than 2 s after the last
  heartbeat that vouched for it.
- **Step 3.** S judges `status` fresh again from the next heartbeat.
- **Step 4.** The second put goes on: it carries no Zenoh timestamp
  (§2.10). H2 hears heartbeats while the guard holds, the time unknown,
  all 42 bits set. Each has its map, `1 0 1`, vouches for `status`, and
  holds `status`'s digest over the second put's stamp. S2 judges `status`
  fresh 4 s on, past its horizon. G's reply carries the owner's stamp
  from the clock ahead, ahead of G's clock beyond the delta. G judges
  `status` unobservable (v1 §2.6's band).
- **Step 5.** A silent heartbeat: S judges `status` stale more than 2 s
  after the last heartbeat. It never reports the owner down.
- **Step 6.** Before the second put, its heartbeats vouch for `status`,
  the time unknown: no write of it was held. The second put is refused,
  since it would carry a Zenoh timestamp. From it on, each heartbeat H3
  hears vouches for nothing, its time unknown, and `status`'s digest is
  still over the first put's stamp (§2.10). After the release the time is
  known, and the bit stays clear. The third put goes on, and a heartbeat
  after it vouches for `status`, its digest over that put's stamp.

## §3 A GET reader's age from the confirmation stamp (§2.4, §2.6)

**Steps.**
1. The owner puts `status` = `up`. S and H listen, and 3 s later G GETs
   `status`.
2. The owner clears `status`'s bit. 3.5 s later, G GETs `status`.

**Expected.**
- **The put** reaches S stamped by R1's clock, if at all: the owner set
  no Zenoh timestamp (core §4.3's table).
- **Step 1.** G's reply carries a Zenoh timestamp of the owner's zid,
  later than the put and within 1 s plus jitter of G's clock. It is the
  last heartbeat's that vouched for `status`. To the millisecond, its
  instant is the time H read in that heartbeat (core §4.3). G judges
  `status` fresh, 3 s after its put.
- **Step 2.** The reply's stamp is the last confirmation before the bit
  cleared. G judges `status` stale: its age is above ttl + δ.

## §4 A lost change, detected and re-read (§2.5)

**Setup.** S's session is a client of R2, a router linked to R1 by a link
the test can cut. `peer/{peer}` is `best_effort`, and nothing repairs a
loss.

**Steps.**
1. The owner puts `peer/a` = `1` and `peer/b` = `1`. S subscribes to
   `state/peer/*`, and its heartbeats confirm both.
2. The link is cut. The owner puts `peer/a` = `2`, which S never hears.
   The link heals.
3. S listens 6 s.

**Expected.**
- **A lost heartbeat.** The first heartbeat S hears after the heal follows
  a gap in its key's seqs. S reads both members unobservable until a
  heartbeat arrives in sequence, never fresh by a heartbeat.
- **The divergence.** S's view holds `peer/a`'s old stamp, so its digest
  is not the owner's. No heartbeat confirms `peer/{peer}` until S re-reads.
- **The re-read.** After two heartbeats in a row diverge, S re-reads the
  collection once. The answer holds `peer/a`'s new stamp, and the next
  heartbeat confirms both members.

## §5 A late joiner by S4 (§2.5, core §4.2)

**Steps.**
1. The owner puts `peer/a` = `1` and `peer/b` = `1`, then nothing.
2. 1 s later, S subscribes to `state/peer/*`, then GETs it, and seeds its
   view with the answer.
3. At the same time S′ subscribes to `state/peer/*` and does not GET.
4. Both listen 5 s.

**Expected.**
- **S** confirms both members at its first heartbeat, with no re-read,
  and judges both fresh 6 s after their puts, beyond their horizon.
- **S′** holds no member, so its first heartbeats diverge. It re-reads
  once, and its next heartbeat confirms both members.

## §6 A restart, a new run: the leftovers leave the view (§2.5, core §4.3)

**Steps.**
1. The owner puts `peer/a` = `1` and `peer/b` = `1`. S subscribes, and its
   heartbeats confirm both.
2. The owner closes. A new instance of `lab/beacon` starts, under a new
   run, and puts only `peer/a` = `1`.
3. S listens 6 s.

**Expected.**
- **No gap.** The new instance's heartbeats are of another run: S reads
  none as following a lost heartbeat.
- **The leftover.** S's view holds `peer/b` from the old run, which the new
  owner does not hold, so its heartbeats diverge.
- **The re-read** replaces the view: `peer/b` leaves it, and the next
  heartbeat confirms `peer/a`. No heartbeat confirms `peer/b` again, and S
  judges it stale more than 4 s after its last confirmation.

## §7 Across a face: liveness, and no downsampling (§2.9; link.v1 §2.8, §2.12)

**Setup.** A face policy names one face, `rf`, of R1, toward the far
principal F, `expose = "link"` and `interval_s = 2`. `beacon.v2` marks
`status` and `level` `link.exposure = "link"` (the conventions). The
owner, F and the generator are all given it. The deployment lists
`lab/beacon` implementing `beacon.v2` and `freshness.v2`. R1 runs the
plan's downsampling block. F is a client of R1 over the face's link
protocol.

**Steps.**
1. A generator derives the face's plan.
2. The owner puts `status` = `up` once and publishes `level` every
   200 ms. F subscribes to `level` and to the heartbeat key, and listens
   5 s.

**Expected.**
- **The plan** derives one downsampling rule, `level`'s, and none for the
  heartbeat key. No deny reaches the heartbeat covering `beacon.v2`. Its
  liveness evidence for `lab/beacon` lists
  `freshness.v2/stream/heartbeat/beacon.v2`.
- **No downsampling.** F hears a heartbeat at least every 1 s plus
  jitter, and `level` at most once every 2 s.
- **Liveness.** F judges the heartbeat key fresh at the end of the 5 s,
  by its horizon of 2 s, so it judges `lab/beacon` live across the face.
