# `freshness.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. The reference runtime runs §1
to §8 (`zenkey/tests/profile_freshness_v2.rs`, one test per section,
named after it). §2's step 6 has a test of its own,
`s2_a_candidates_held_put_clears_its_bit`. So have §8's steps 3 and 4,
`s8_an_owner_heartbeat_across_a_face`.

**Conventions.**
- **Section numbers** in parentheses (§2.4) are v2.md's rules; core
  sections say "core", and `link.v1`'s say so.
- **The bus.** R1 is a router with timestamping on, as a router's default
  is. The owner, the subscribers S and the GET reader G are its clients,
  unless a section says otherwise. The owner's session runs with
  timestamping off (core §8.2). Timeouts are 1 s unless stated.
- **The contract.** Every section uses `beacon.v2`, and §8 adds two of
  its own. The reference keeps it as
  `zenkey/tests/contracts/beacon.v2.toml`:

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
  v2.md §2.6 to the answer's entry, the delta at 500 ms, on the
  deployment's word: every session runs on one host.
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
  (§2.10). H2 hears heartbeats while the guard holds. Each one's time is
  the ahead time, the seconds all ones and the millisecond code 1022
  (core §7.5), never the unknown time. Each has its map, `1 0 1`,
  vouches for `status`, and
  holds `status`'s digest over the second put's stamp. S2 judges `status`
  fresh 4 s on, past its horizon. G's answer is a snapshot of `status`
  alone, its entry vouched for at the snapshot's time. That time is from
  the clock ahead, ahead of G's clock beyond the delta. G judges `status`
  unobservable (v1 §2.6's band).
- **Step 5.** A silent heartbeat: S judges `status` stale more than 2 s
  after the last heartbeat. It never reports the owner down.
- **Step 6.** Before the second put, its heartbeats vouch for `status`,
  their time the ahead time: no write of it was held. The second put is
  refused, since it would carry a Zenoh timestamp. From it on, each
  heartbeat H3 hears vouches for nothing, its time the ahead time, and
  `status`'s digest is still over the first put's stamp (§2.10). After
  the release the time is an instant again, and the bit stays clear. The third put goes on, and a heartbeat
  after it vouches for `status`, its digest over that put's stamp.

## §3 A GET reader's age from the confirmation (§2.4, §2.6)

**Steps.**
1. The owner puts `status` = `up`. S and H listen, and 3 s later G GETs
   `status`.
2. The owner clears `status`'s bit. 3.5 s later, G GETs `status`.
3. G GETs `state/peer/**` after §1's steps 1 and 2: `peer/a` vouched for,
   `peer/b` not.

**Expected.**
- **The put** reaches S stamped by R1's clock, if at all: the owner set
  no Zenoh timestamp (core §4.3's table).
- **Step 1.** G's answer is a snapshot of `status` alone, with no Zenoh
  timestamp. Its entry has confirmation 1: vouched for at the snapshot's
  time, within jitter of G's clock. G judges `status` fresh, 3 s after
  its put.
- **Step 2.** The entry has confirmation 2. Its conf is the last
  confirmation before the bit cleared: to the millisecond, the time H
  read in the last heartbeat that vouched for `status` (core §4.3). G
  judges `status` stale: its age is above ttl + δ.
- **Step 3.** One snapshot of `peer/{peer}`: `a` with confirmation 1, and
  `b` with confirmation 2 and its last confirmation.

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
2. 1 s later, S subscribes to `state/peer/*`, then GETs `state/peer/**`,
   a collection read, and seeds its view with the answer.
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
- **The re-read** replaces the view: the new instance's snapshot is
  complete and omits `peer/b`, which leaves the view by core §4.2's
  omission rule. The next heartbeat confirms `peer/a`. No heartbeat confirms `peer/b` again, and S
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

## §8 An owner heartbeat (§2.4, §2.5, §2.9; link.v2 §2.9)

**Setup.** The owner is `lab/w2`. It implements `beacon.v2` as the
conventions say, and two contracts of one collection each, W2's shape:

```toml
[interface]
name  = "dial"
major = 1
minor = 0
uses  = ["freshness.v2"]

[resources.config]                         # one member
kind        = "state"
type        = { raw = "text/plain" }
annotations = { "freshness.ttl_s" = 2 }
```

```toml
[interface]
name  = "pulse"
major = 1
minor = 0
uses  = ["freshness.v2"]

[resources.status]                         # one member
kind        = "state"
type        = { raw = "text/plain" }
annotations = { "freshness.ttl_s" = 2 }
```

- **The catalog.** The owner is given a catalog of three entries:
  `beacon.v2` at index 0, `dial.v1` at 1 and `pulse.v1` at 2. Every
  reader below holds it.
- **The owner heartbeats.** The owner covers the three interfaces by
  owner heartbeats. Its keys are
  `zk2/lab/w2/freshness.v2/stream/heartbeats/host` and
  `zk2/lab/w2/freshness.v2/stream/heartbeats/link`.
- **The classes.** `beacon.v2`'s `status` is `link`. Its `peer/{peer}`
  and `route`, `dial.v1`'s `config` and `pulse.v1`'s `status` are `host`.
  So each key's shortest horizon is 2 s, and its owner heartbeats are at
  most 1 s apart.
- **H** subscribes to both owner heartbeat keys and to
  `zk2/lab/w2/freshness.v2/stream/heartbeat/*`. It holds the three
  revisions.
- **S** subscribes to every collection of the three interfaces and to
  both owner heartbeat keys. Its session is a client of R2, a router
  linked to R1 by a link the test can cut, as in §4.
- **P** is bound to `pulse.v1` alone. It holds that revision and the
  catalog, and no other contract. It subscribes to `pulse.v1`'s `status`
  and to the `host` key.

**Steps.**
1. The owner puts `beacon.v2`'s `status` = `up` and `peer/a`, `peer/b`
   and `peer/c` = `1`, `dial.v1`'s `config` = `x` and `pulse.v1`'s
   `status` = `ok`. Then it puts nothing for 5 s. H, S and P listen.
2. The link is cut. The owner puts `peer/a` = `2`, which S never hears.
   2.5 s later the link heals, and S listens 6 s.
3. A `link.v2` face policy names one face, `rf`, of R1, toward the far
   principal F: `expose = "link"`, `interval_s = 2`, and a `[transport]`
   its linter accepts. The deployment lists `lab/w2` implementing the
   three interfaces and `freshness.v2`. A generator derives the plan,
   and R1 runs it. F is a client of R1 over the face's link protocol. It
   holds the catalog and `beacon.v2`. It subscribes to both owner
   heartbeat keys, and listens 5 s.
4. Step 3 again under §7's `link.v1` policy, with `lab/w2` listed.

**Expected.**
- **The descriptor** lists `beacon.v2`, `dial.v1`, `pulse.v1` and
  `freshness.v2`, and states the catalog's id.
- **One form** (step 1). H hears no put on any `heartbeat/<iface>` key.
- **The `host` key.** H hears an owner heartbeat at least every 1 s plus
  jitter, from the owner's tokens on. Each is form 6 with CH's tag 0, and
  their seqs are consecutive within one run.
  - It holds three groups, of indices 0, 1 and 2, each with its
    revision's tag.
  - `beacon.v2`'s group has the map `1 0 0`. Its `peer/{peer}` section
    holds 3 live members, all vouched for, its digest over their stamps.
    `status` is another class's, and `route` is not exposed.
  - `dial.v1`'s and `pulse.v1`'s groups each vouch for their one member.
  - Its payload is 34 bytes on a 16-bit run, and 36 on a 32-bit one:
    223 bits of body.
- **The `link` key.** H hears an owner heartbeat at least every 1 s plus
  jitter, its seqs its own. It holds one group, `beacon.v2`'s, with the
  map `0 0 1`: `status`, vouched for. Its payload is 19 bytes on a
  16-bit run.
- **Confirmed.** S hears each put once. 5 s after the puts it judges
  every member of the three interfaces fresh, past each horizon.
- **One interface's reader.** P judges `pulse.v1`'s `status` fresh 5 s
  after its put. It read its group behind two groups whose revisions it
  does not hold.
- **A lost owner heartbeat** (step 2). The first owner heartbeat S hears
  on each key after the heal follows a gap in that key's seqs. S reads
  it as one lost for every group: its reading names all four `host`
  collections it follows as having lost a delivery, and `status` on the
  `link` key. The next owner heartbeat on each key arrives in sequence,
  and ends that.
- **A divergence in one group.** S's view holds `peer/a`'s old stamp. So
  `beacon.v2`'s group diverges on `peer/{peer}`, on the `host` key.
  - `dial.v1`'s and `pulse.v1`'s groups confirm their members in the
    same owner heartbeats, and so does `status` on the `link` key.
  - After two in a row, S re-reads `peer/{peer}` once. The next `host`
    owner heartbeat confirms its three members.
- **The plan** (step 3). Its stream deny of `freshness.v2` holds
  `zk2/*/*/freshness.v2/stream/heartbeats/host/**`, and the heartbeat
  keys of `dial.v1` and `pulse.v1`. It holds nothing of
  `heartbeats/link`. Its liveness evidence for `lab/w2` lists
  `freshness.v2/stream/heartbeats/link`.
- **The `host` class stays home.** F hears no put on the `host` key in
  5 s. It hears an owner heartbeat on the `link` key at least every 1 s
  plus jitter, never downsampled. Each holds `beacon.v2`'s group alone,
  with the map `0 0 1`: no bit of `peer/{peer}` crosses.
- **Liveness.** F judges the `link` key fresh at the end of the 5 s, by
  its horizon of 2 s. So it judges `lab/w2` live across the face.
- **Under `link.v1`** (step 4). The plan's stream deny of `freshness.v2`
  holds `zk2/*/*/freshness.v2/stream/heartbeats/**`. F hears no owner
  heartbeat on either key.
