# Constrained scenarios (core §1.6, R7, §8.1, §8.2, §8.4, §8.5, §12)

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

## §6 A far router in a south region (§8.5, U23)

**Setup.** As §1, but the ground keeps a router of its own, with
`region_name = "ground"`. The vehicle router's `gateway.south` lists its
clients and peers, and the region `ground`. The `@zk` deny is on the face.

**Expected.**
- The vehicle's 200 tokens put about 0.5 KB on the link, and none of their
  key strings.
- A ground client's subscriber and state GET on the vehicle's data work.
- Without the south region (router to router), the same deny leaves every
  key string on the link.

*Spike S3, U23 addendum: 506 B and 0 keys, against 11,330 B and 201.*

## §7 Keys across the face (§8.5)

**Setup.** One router, and an owner that is its client. The owner writes
a stream `…/stream/level`, a templated stream's member `…/stream/axis/roll`
and a state `…/state/mode`. Each step's far client is a client of the
router through a TCP proxy. The proxy counts what crosses toward it.

**Steps.**
1. The far client subscribes to `…/stream/level`, its exact key.
2. It subscribes to the template's selector, `…/stream/axis/*`.
3. It subscribes to `zk2/**`.
4. It subscribes to `…/state/mode`, its exact key, then GETs it.

**Expected.**
1. Each put rides the scope its subscription declared. It carries none
   of its key's text.
2. Each put rides the scope, and carries `/roll`.
3. Each put carries every chunk of its key after `zk2` as text. It is
   larger than step 1's by that text and a byte of length.
4. The reply carries its whole key as text.

*The reference: 45–46 B a put in step 1, 52 B in step 2, and 82 B in step
3, its key's 35 B after `zk2` and a byte of length more.*

## §8 A reply across the face (§8.1)

**Setup.** One router. An owner, a client of it, serves an operation.
Its contract recommends `timeout_ms = 1000`. Its reply comes back 1.5 s
after each call: a reply queued behind a radio's traffic. A caller is a
client of the router. Its face states `reply_timeout_ms = 3000` (`link.v2`
§2.1).

**Steps.**
1. The caller calls the operation, setting no timeout of its own.
2. It calls again across the face, setting none either.

**Expected.**
1. No reply comes back. The router answers `Timeout` at the contract's
   1 s, and drops the reply that follows.
2. The value comes back after 1.5 s: the caller waited at least the
   face's 3 s.

## §9 A declaration lost across the face (§8.5, observer §7.5)

**Setup.** One router. An owner, a client of it, puts a stream every 100
ms. Its `freshness.ttl_s` is 1. A far client attaches through a TCP proxy.
On request, the proxy drops the next batch the far client sends, or the
next that holds the stream's key, and forwards the rest. That is a frame
lost below zenoh, its transport left up. The face's medium repairs no
lost frame (`arq = false`) unless a step says so. In steps 1 to 3 the far
client first subscribes to the stream, hears it and undeclares, so its
runtime has declared the key's prefix toward the router.

**Steps.**
1. The proxy drops the batch declaring a consumer's subscription to the
   stream: its `DeclareSubscriber`.
2. The same for a tool's watch of it.
3. **Control:** step 1 across a face whose medium repairs lost frames
   (`arq = true`).
4. On a far client that never subscribed, the proxy drops the batch
   holding the stream's key: its `DeclareKeyExpr`, and the
   `DeclareSubscriber` beside it.

**Expected.**
1. Nothing is delivered for two horizons, 2 s. The subscription is then
   declared again, once, and the stream is delivered from then on.
2. The same for the tool's watch.
3. Nothing is delivered for three horizons: the subscription is never
   declared again, and stays silent.
4. Nothing is delivered for three horizons. The subscription is declared
   again on the scope the router never learned, and stays silent (core
   Appendix B).

*The reference: `zenkey/tests/constrained.rs` (§8, §9 steps 1, 3 and 4)
and `zenkey-fleet/tests/redeclare.rs` (step 2).*

## §10 A first-hop device (§8.1, §8.2, §8.4, §8.5; observer §3.11, §7.1)

**Setup.**
- One router. A device owner at `lab/probe-a` is its client through a
  TCP proxy. The proxy counts each declaration that crosses toward the
  router, by kind and key.
- The owner implements `probe.v1` (`link.v2`'s test contract) and
  `health.v1`, so `freshness.v2` too. It puts a heartbeat covering each
  of the first two (`freshness.v2` §2.4).
- It is configured for a first hop: its three interfaces in its
  tokenless set, and no bundle held.
- The router's block is a `link.v2` device face's that lists the owner,
  under `deny` (`link.v2` §2.5, §2.6).
- On the near side, three clients of the router: a holder of the three
  bundles, a tool, and a consumer bound to `lab/probe-a`'s `probe.v1`
  that waits on presence (R5).

**Steps.**
1. The device starts.
2. The proxy closes its connection. The device's session opens a new
   transport.
3. The device starts a new run (§4.3), so its descriptor changes.
4. **Control:** the router's block keeps `link.v1` §2.6's whole `zk`, and
   the device starts again.
5. **Control:** under step 1's block, the device starts at its defaults,
   with no tokenless set and its bundles held.
6. A device configured for a first hop that also implements
   `zs.catalog.v1`, whose contract uses `redundancy.v1`, is started.

**Expected.**
1. Toward the router the proxy counts one `DeclareToken`, the instance
   token's. Under `@zk` it counts one `DeclareQueryable`, the
   descriptor's. No interface token crosses, and no contract queryable.
   - The tool lists the device and its three interfaces from its
     descriptor, each `"token": false`. It reports no missing interface
     token.
   - The consumer's wait ends once the instance token and the descriptor
     are in.
   - The tool retrieves `probe.v1`'s bundle from the near holder.
2. While the link is closed, the tool reports the device's link closed,
   never the device down.
   - Its liveness is its heartbeat's: confirmed until the horizon
     passes, then unobservable, never down.
   - At the new transport the proxy counts step 1's declarations again,
     once each, and nothing more under `@zk`.
3. The tool shows the new run, from the descriptor put it hears.
4. The tool lists no instance at `lab/probe-a`. The device's presence is
   unobservable across the face, never absent. The consumer still
   receives the device's puts and heartbeats, and judges its liveness
   from them (R7).
5. At each transport the proxy counts the instance token, three interface
   tokens and three contract queryables. The tool sees the instance token
   alone: the face denied the rest once it had crossed (Appendix B).
6. The device does not start: step 2 of §8.2 refuses it, and no instance
   token crosses.
