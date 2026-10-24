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
6. An owner that does not implement `archive.v1` is started with
   `archive.v1` in its tokenless set (core §4.4, §8.2 step 2, 0.17).

**Watching a refusal (steps 3 to 6).** A refusal shows only as a token
that never appears, and silence is not a verdict (core O5), so the watch
needs a router that outlives the owner and a control:
- the owner and the tool are clients of R1, which stays up whatever the
  owner does; never a router the owner's own process runs, which is gone
  when the owner refuses;
- the tool subscribes to the liveliness selector
  `zk2/<system>/<service>/@zk/**` through R1 before the owner is launched,
  and watches until the owner exits or the wait of core §8.1 ends;
- **the control:** the same owner, with the resource exposed (or, in
  step 5, listed `unavailable`), the role bound, or (in step 6) the
  tokenless set empty, launched the same way, shows its instance token to
  that subscriber within the wait.

**Expected.**
1. One reply, with `Encoding` `application/json`: a descriptor that
   validates against `descriptor.schema.json`. Its `profiles` is the union
   of its contracts' `uses`, with `hostid.v1` added when its system is
   minted (core §3.3, 0.19). Every role is listed, an unbound optional
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
6. As 3: the tokenless set is the deployment's configuration, and naming
   the archive in it is an error whatever the owner implements.

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

**Setup.** 10,000 tokens. A reader's watch session holds a liveliness
subscriber to every one of them (core §8.1, "The read that establishes
absence").

**Steps.** The watch session makes its one liveliness GET, the round trip
core §8.1 asks of it before a seed, under each pairing of the GET's
handler and the subscriber's.

**Expected.**
- With callbacks for both, the GET completes: it ends at the routers' final
  reply, before its timeout. So it does with a GET handler drained as
  replies arrive, beside a callback subscriber. Its answer is discarded:
  a GET beside a liveliness subscriber can complete without live tokens
  (core §8.1, Appendix B), so this section counts no token (0.38, F-186).
- With a bounded subscriber handler that nobody drains, a callback GET ends
  at its timeout, silently, and a GET on a bounded handler hangs. This is
  why the rule binds the subscribers too (core §8.1).
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
3. The second tool reads `zk2/*/*/@zk/instance/*` with the link flowing,
   then again with R1's replies held back past the read's timeout. (0.8
   read `zk2/**` here, which by core §1.3's guard selects no control
   token, so the "no token" below could never fail: F-75, fixed in 0.9.)

**Expected.**
1. R1 reads the token.
2. The instance read holds the owner's instance token, and no error reply.
   The alive read is answered: a final reply, no token and no error reply,
   the same as a selector no token matches. A tool cannot tell it from
   absence (core §8.1, "A refused read").
3. The first read holds the owner's instance token, and ends with no error
   reply. The second ends with the error reply `Timeout` and no token, and
   the tool reports it as possibly incomplete, never as absence.

*Measured (0.8) on zenoh 1.10.1, with the reference runtime: as above.
The same deny on the `egress` flow alone refused nothing: the alive read
held the token.*

## §7 A read through a session cut off (§8.1, 0.37)

**Setup.** One router R1. An owner is a client of R1. A tool reaches R1
through a link the test can cut and heal, and retries its connection.

**Steps.**
1. The owner starts. Wait until the tool reads its instance token.
2. Cut the link. Wait until the tool's session holds no transport.
3. The tool makes a liveliness GET of `zk2/*/*/@zk/**`, and R1's own
   session reads the owner's instance token.
4. Heal the link, and the tool reads `zk2/*/*/@zk/**` again.
5. (0.38, F-185) Stall the link so that it delivers nothing, while the
   tool's session still holds its transport. The reader under test asks a
   presence read of `zk2/*/*/@zk/**`. While it is in flight, cut the link.

**Expected.**
1. The tool reads the token.
2. –
3. The GET is answered at once: a final reply, no token and no error reply,
   though nothing was asked of R1, and R1's read holds the owner's token: it
   never left. The reader treats the GET as not complete (core §8.1, "A read
   through a session cut off"): the reference makes no such read, and
   reports it not made, with its reason.
4. The tool's read holds the owner's instance token.
5. The GET is answered at the cut: a final reply, no token and no error
   reply. A check made before the read passed it, since a transport was
   held. The reader finds no transport when the read ends, and reports the
   read possibly incomplete, cut off during it, never the owner absent
   (core §8.1, "A read cut off while it runs").

*Measured (0.37) on zenoh 1.10.1, with the reference runtime: as above.
alarms.v1 0.4 first measured the cut-off read, and its reader made none.
Step 5 measured (0.38) with zenoh-python 1.10.1 by the second
implementation (#609, round 54): answered at the cut, 4 of 4.*

## §8 A withdrawal heard live (§8.1, 0.37)

**Setup.** As §7, with a second session of the tool's — its watch session
— on the same link, holding a liveliness subscriber with history to
`zk2/*/*/@zk/instance/*`.

A **reader under test** follows presence live as core §8.1 asks: its
subscribers on the watch session, its reads on the tool's session. Every
Expected below that names the reader observes the reader's own verdict, not
zenoh (0.38, F-188).

**Steps.**
1. The owner starts. Wait until the subscriber hears its instance token,
   and the tool reads it.
2. Cut the link. While it is cut, ask the reader for the owner's address.
   Then heal it, and ask again.
3. The owner stops, cleanly.
4. Three owners start, on three addresses. Wait until the reader reads each
   up. Stop all three at once.
5. One owner re-mints make-before-break (core §8.1, "Re-minting is
   make-before-break").

**Expected.**
1. –
2. On the cut, the subscriber hears the owner's instance token withdrawn,
   and R1's own session still reads it: the owner never left. While the
   link is cut, the reader reports the address unobservable, withdrawn,
   **never down**, and its re-read **not made** (cut off). On the heal,
   the subscriber hears it again, the tool's read holds it, and the reader
   reports the address up. A reader that concluded absence from the
   withdrawal would have read a live owner down (core §8.1, "Absence is
   established only by a complete read").
3. The subscriber hears the token withdrawn. The reader reports the address
   unobservable until its re-read, and down only once that read is
   complete and holds no instance token of the owner: absence, as a read
   establishes it.
4. The reader makes **one** read for the three withdrawals, and reports all
   three addresses down from it.
5. The reader makes **no** read, and the address stays up throughout.

*Measured (0.37) on zenoh 1.10.1, with the reference runtime: as above.
A reader whose router restarts sees the same, every token at once
(`zenkey-fleet`'s `a_reader_whose_link_is_cut_reads_nobody_down`). The
reference's reader under test is `zenkey-fleet`'s `PresenceFeed`.*

## §9 A read beside a liveliness subscriber (§8.1, 0.37; Appendix B)

**Setup.** One router R1. An owner and a tool are clients of R1; the tool
opens a second session, its watch session.

**Steps.**
1. The owner starts. Wait until the tool reads its instance token.
2. The watch session declares a liveliness subscriber with history to
   `zk2/*/*/@zk/instance/*`, and at once makes a liveliness GET of
   `zk2/*/*/@zk/**`.
3. The tool, whose session holds no liveliness subscriber, makes the same
   GET.
4. (0.38, F-189) A reader under test, as §8's, starts following presence:
   it declares its subscribers on its watch session, then seeds through
   the tool's session. Observe the liveliness GETs each of its sessions
   makes. Then ask it for a liveliness read on its watch session.
5. Repeat, many times: a fresh reader starts as in step 4, and the owner's
   instance token is withdrawn as soon as the reader's seed is answered.

**Expected.**
1. –
2. The GET ends with the final reply and no error reply, and without the
   owner's instance token (Appendix B: the same session's subscriber
   covers it). The subscriber does hear it.
3. The tool's GET holds the owner's instance token. A reader makes the
   read that establishes absence on a session like the tool's (core §8.1,
   "The read that establishes absence"): the reference refuses a liveliness
   read on a session that holds a liveliness subscriber it declared, and
   reports it not made.
4. The watch session makes one liveliness GET, the round trip, after its
   subscribers and before the seed, and no other. The seed and every
   re-read go through the tool's session. The read asked on the watch
   session is not made, beside a liveliness subscriber.
5. In every run, the reader hears the token withdrawn and, after its
   re-read, reports the owner down. A reader that skips the round trip can
   miss the withdrawal: the token went between the seed's answer and the
   router's reading of the subscriber. Such a reader goes on reading the
   owner up, in some runs.

*Measured (0.37, #830) on zenoh 1.10.1 and zenoh main at 766b455: step 2's
GET missed the token in 720 of 720 fresh processes, and in none with the
subscriber on another session
(`docs/zk2/upstream/liveliness-get-repro/`).*
