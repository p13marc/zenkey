# Presence scenarios (core §1.5, §3.3, §8)

## §1 Bring-up order and tokens (§8.1, §8.2)

**Setup.** An owner implementing `nav.v2` (one required state, one
operation), and a pure consumer.

**Steps.** A tool subscribes to `zk2/*/*/@zk/**` with history, and calls the
owner's operation the moment its interface token appears.

**Expected.**
- The call succeeds: alive ⇒ callable.
- A GET of the descriptor, and of the contract bundle by its key, made the
  moment the instance token appears, both answer.
- The owner holds an instance token and one `alive/nav.v2/…` token.
- The pure consumer holds an instance token only.
- An instance exposing nothing of an interface holds no interface token for
  it.

## §2 The descriptor (§3.3)

**Steps.**
1. GET the owner's instance key, with consolidation `None`.
2. The owner loses a capability that gates an optional resource.
3. An owner is started without one of its contract's required resources.
4. An owner is started with a required role its configuration binds to
   nothing (core §3.2).

**Expected.**
1. One reply, with `Encoding` `application/json`: a descriptor that
   validates against `descriptor.schema.json`. Its `profiles` is the union
   of its contracts' `uses`, and every role is listed, an unbound optional
   one with `"bindings": []`.
2. A new descriptor is put on the instance key. The gated resource's
   absence is implied by the missing capability, so it is not listed in
   `unavailable`. A GET returns the new descriptor.
3. The owner does not start: no instance token appears.
4. The owner does not start: no instance token appears.

*Measured on the reference owner, from zenoh-python 1.10.1 (#609): one
reply, `application/json`, no timestamp and no attachment; the
`walkthrough/thruster.v1` owner, whose required role `cmd` was unbound,
refused to start and declared no token.*

## §3 Epochs and re-minting (§1.5, §8.1)

**Steps.**
1. The owner's counters reset without a restart.
2. A member of a template with `epoch` loses continuity.
3. An owner whose contract has an `epoch` template starts, and declares no
   member.

**Expected.**
1. The owner declares a new instance token, interface tokens and descriptor,
   then undeclares the old ones. A watcher sees 1 or 2 live instance tokens
   throughout, never 0. Consumers treat the instance change as the counter
   discontinuity.
2. That member's token cycles. The others are untouched.
3. No member token: a member exists from the owner's first declaration of
   it (core §8.1).

*Spike S2: one re-mint per second over 10k tokens cost 0.7 KiB/s; the
re-minted service always had 1 or 2 live instance tokens.*

## §4 Reading presence at scale (§8.1)

**Setup.** 10,000 tokens.

**Steps.** A session that holds a liveliness subscriber issues a liveliness
GET, under each pairing of the GET's handler and the subscriber's.

**Expected.**
- With callbacks for both, the GET completes with every token. So it does
  with a GET handler drained as replies arrive, beside a callback
  subscriber.
- With a bounded subscriber handler that nobody drains, a callback GET ends
  at its timeout with a fraction of the tokens, silently, and a GET on a
  bounded handler hangs. This is why the rule binds the subscribers too
  (core §8.1).
- A tool that sees a GET end at its timeout reports the result as possibly
  incomplete.

*Spike S2 (Rust): hung at every measured size from 996 tokens
(zenoh#2678).*

*Measured with zenoh-python 1.10.1 (#609), 2,002 tokens, a 10 s timeout:
GET and subscriber both callbacks, complete in 0.34 s; a default GET
handler drained as replies arrive, or after 3 s, beside a callback
subscriber, complete; a default GET beside a default subscriber never
drained, 0 replies after 20 s; a callback GET beside that subscriber, 257
of 2,002, ended at the timeout. The default handler alone did not hang: the
undrained subscriber is what starves the GET. zenoh-python has no unbounded
handler.*

## §5 A tokenless set (§8.1, U22)

**Setup.** A deployment configures 100 services with `health.v1` in their
tokenless set; each also implements `nav.v2`.

**Expected.**
- Each instance holds an instance token and an `alive/nav.v2/…` token, and
  no `alive/health.v1/…` token: 200 tokens instead of 300.
- Each descriptor marks `health.v1` with `"token": false`.
- A tool lists the `health.v1` providers from instance tokens and
  descriptors, and finds all 100.
