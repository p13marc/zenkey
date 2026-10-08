# Presence scenarios (core §1.5, §3.3, §8)

## §1 Bring-up order and tokens (§8.1, §8.2)

**Setup.** An owner implementing `nav.v2` (one required state, one
operation, and a required templated state with no member yet), and a pure
consumer. The owner holds its state's value at start.

**Steps.** A tool subscribes to `zk2/*/*/@zk/**` with history, and calls the
owner's operation the moment its interface token appears. At that moment it
also GETs the owner's state, with `All` + `Latest`.

**Expected.**
- The call succeeds: alive ⇒ callable.
- A GET of the descriptor, and of the contract bundle by its key, made the
  moment the instance token appears, both answer.
- A data subscriber to `zk2/*/*/@zk/instance/*`, up before the owner
  starts, receives the owner's first descriptor, put on its instance key
  (core §3.3, §8.2 step 3), without a GET.
- The state GET returns the value the owner started with: it was put
  before the tokens (core §8.2, "State values").
- The owner starts although the templated state has no member: it is
  exposed by its template, its descriptor does not list it, and no member
  token appears (core §8.1, §8.2).
- The owner holds an instance token and one `alive/nav.v2/…` token.
- The pure consumer holds an instance token only.
- An instance exposing nothing of an interface holds no interface token for
  it.

*Measured (#609): zk2py's owner puts its descriptor once, stamped, after
step 3 and before the tokens, as the reference does, and puts its state
values before the tokens. The reference owner example put its state value
after starting (F-68), and was read only after presence and the descriptor
GET, so that runner did not test the moment the token appears.*

## §2 The descriptor (§3.3)

**Steps.**
1. GET the owner's instance key, with consolidation `None`.
2. The owner loses a capability that gates an optional resource.
3. An owner is started without one of its contract's required resources.
4. An owner is started with a required role its configuration binds to
   nothing (core §3.2).
5. An owner is started with an optional resource it neither exposes nor
   lists `unavailable`, its gate's capability held (core §8.2 step 2).

**Watching a refusal (steps 3 to 5).** A refusal shows only as a token
that never appears, and silence is not a verdict (core O5), so the watch
needs a router that outlives the owner and a control:
- the owner and the tool are clients of R1, which stays up whatever the
  owner does; never a router the owner's own process runs, which is gone
  when the owner refuses;
- the tool subscribes to the liveliness selector
  `zk2/<system>/<service>/@zk/**` through R1 before the owner is launched,
  and watches until the owner exits or the wait of core §8.1 ends;
- **the control:** the same owner, with the resource exposed (or, in
  step 5, listed `unavailable`) or the role bound, launched the same way,
  shows its instance token to that subscriber within the wait.

**Expected.**
1. One reply, with `Encoding` `application/json`: a descriptor that
   validates against `descriptor.schema.json`. Its `profiles` is the union
   of its contracts' `uses`, and every role is listed, an unbound optional
   one with `"bindings": []`.
2. A new descriptor is put on the instance key. The gated resource's
   absence is implied by the missing capability, so it is not listed in
   `unavailable`. A GET returns the new descriptor.
3. The owner does not start. The tool's subscriber receives no token of
   the service while it watches, and a liveliness GET of the same selector
   through R1 afterwards returns none, where the control showed one.
4. As 3.
5. As 3: its descriptor would have claimed the resource, whose exposure is
   compact (core §3.3).

*Measured on the reference owner, from zenoh-python 1.10.1 (#609): one
reply, `application/json`, no timestamp and no attachment; the
`walkthrough/thruster.v1` owner, whose required role `cmd` was unbound,
refused to start and declared no token. That owner was its own router, so
the client completed no presence GET before it exited: the check rested on
silence, which is why steps 3 and 4 now watch through R1 (#609, F-61). The
reference's own test of step 3 watches through a separate router.*

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

## §6 Possibly incomplete, and refused (§8.1)

**Setup.** One router R1 whose access control, under `allow`, denies
`liveliness_query` on the ingress flow for `zk2/*/*/@zk/alive/**`. An owner
and a tool are clients of R1. A second tool reaches R1 through a link
whose router-to-client direction the test can stall.

**Steps.**
1. The owner starts. Wait until R1's own session reads its interface
   token: R1 has no face of its own, so no rule applies to it.
2. The tool reads `zk2/*/*/@zk/instance/*`, then `zk2/*/*/@zk/alive/**`.
3. The second tool reads `zk2/**` with the link flowing, then again with
   R1's replies held back past the read's timeout.

**Expected.**
1. R1 reads the token.
2. The instance read holds the owner's instance token, and no error reply.
   The alive read is answered: a final reply, no token and no error reply,
   the same as a selector no token matches. A tool cannot tell it from
   absence (core §8.1, "A refused read").
3. The first read ends with no error reply. The second ends with the error
   reply `Timeout` and no token, and the tool reports it as possibly
   incomplete, never as absence.

*Measured (0.8) on zenoh 1.10.1, with the reference runtime: as above.
The same deny on the `egress` flow alone refused nothing: the alive read
held the token.*
