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

**Setup.** More tokens than zenoh#2678's threshold: at least 1,200, past
the 996 from which spike S2 measured the hang (0.38; 10,000 before, which
the hang does not need). A reader's watch session holds a liveliness
subscriber to every one of them (core §8.1, "The read that establishes
absence"). Each GET waits 5 s.

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

*Measured (0.38) with the reference runtime and fleet, 1,200 tokens, a
debug build (`zenkey-fleet/tests/liveliness_handler.rs`'s
`s4_reading_presence_at_scale`): the round trip on the runtime's handler,
drained as replies arrive, beside the monitor's callback subscriber with
history, ended at the final reply (0.36 s with the subscriber's
declaration); a callback GET beside it, in 0.05 s, with no error reply.
The reference declares every liveliness subscriber with a callback, and
runs neither pairing with a bounded subscriber nobody drains: they wedge
zenoh's own threads in process. At 10,000 tokens, in the same build, the
setup alone took 24 s, and the round trip up to 10 s.*

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

*Measured (0.38) with the reference runtime (`zenkey/tests/presence.rs`'s
`s7_a_read_through_a_session_cut_off`): step 5's read was answered at the
cut, no token and no error reply, and read possibly incomplete, cut off
during it, in 20 of 20 runs, with the check after it reading the session's
links. Reading `Session::info().transports()` instead, it came back
complete and empty in 4 of 10: the closing transport was still listed
(core §8.1, "What the check reads"). The fleet's reader, `PresenceFeed`,
read such a re-read as a seed possibly incomplete, cut off while it ran,
and kept its owner up, in 10 of 10 runs (`zenkey-fleet/tests/presence_scenarios.rs`'s
`s7_a_read_cut_off_while_it_runs`).*

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

*Measured (0.38) with that reader, in `zenkey-fleet/tests/presence_scenarios.rs`'s
`s8_a_withdrawal_heard_live` (zenoh's half is `zenkey/tests/presence.rs`'s
`s8_a_withdrawal_heard_live`): as above, every step, in 10 of 10 runs. While
cut, the address was unobservable and withdrawn, and the re-read not made;
up on the heal; down from the read after the clean stop, unobservable on
its withdrawal before. The three stopped at once cost one read, the three
downs one fold of it; the re-mint cost none, and moved no status.*

## §9 A read beside a liveliness subscriber (§8.1, 0.37; Appendix B)

**Setup.** One router R1. An owner and a tool are clients of R1; the tool
opens a second session, its watch session. In steps 4 and 5 the watch
session reaches R1 through a link the test can stall (0.38).

**Steps.**
1. The owner starts. Wait until the tool reads its instance token.
2. The watch session declares a liveliness subscriber with history to
   `zk2/*/*/@zk/instance/*`, and at once makes a liveliness GET of
   `zk2/*/*/@zk/**`. Ten times, a fresh watch session each time (0.42,
   F-235). History is what shows the hazard nearly every time; a reader's
   own watch session declares its subscribers without history (core §8.1,
   0.41; §10).
3. The tool, whose session holds no liveliness subscriber, makes the same
   GET.
4. (0.38, F-189) A reader under test, as §8's, starts following presence:
   it declares its subscribers on its watch session, without history
   (0.41), then seeds through the tool's session. Observe the liveliness GETs each of its sessions
   makes. Then ask it for a liveliness read on its watch session.
5. Repeat, many times: a fresh reader starts as in step 4, its watch
   session's link stalled from before the reader declares its subscribers
   until 150 ms later, longer than a seed's answer and the owner's
   withdrawal take on the test's bus (0.38). The owner's instance token is
   withdrawn as soon as the reader's seed is answered.

**Expected.**
1. –
2. Each GET ends with the final reply and no error reply, and nearly
   always without the owner's instance token (Appendix B: the same
   session's subscriber covers it), in one run of the ten at least. The
   session's own answer can win the race, in a few runs in a hundred, and
   a GET that holds the token proves nothing safe: the hazard is a
   complete GET that can miss a live token (0.42, F-235). The subscriber
   does hear it.
3. The tool's GET holds the owner's instance token. A reader makes the
   read that establishes absence on a session like the tool's (core §8.1,
   "The read that establishes absence"): the reference refuses a liveliness
   read on a session that has held a liveliness subscriber it declared
   (0.38, F-190), and reports it not made.
4. The watch session makes one liveliness GET, the round trip, after its
   subscribers and before the seed, and no other. The seed and every
   re-read go through the tool's session. The read asked on the watch
   session is not made, beside a liveliness subscriber.
5. In every run, the reader hears the token withdrawn and, after its
   re-read, reports the owner down: its round trip waited for the link. A
   reader that skips the round trip seeds while its subscribers'
   declarations wait in the stalled link, and the token goes before its
   router reads them: it never hears the token go, and goes on reading the
   owner up, in every run. Without the stall the race is narrow, and a run
   tests little: on one router in process, such a reader missed the
   withdrawal in 0 of 200 runs (0.38).

*Measured (0.37, #830) on zenoh 1.10.1 and zenoh main at 766b455: step 2's
GET missed the token in 720 of 720 fresh processes, and in none with the
subscriber on another session
(`docs/zk2/upstream/liveliness-get-repro/`). On zenoh-python 1.10.1 (zk2py,
0.42, F-235) it held the token in 2 of 60 trials beside the reference's
owner and 5 of 60 beside zk2py's; the reference's step 2 missed it in 10 of
10 (`zenkey/tests/presence.rs`).*

*Measured (0.38) with the reference's reader, `zenkey-fleet`'s
`PresenceFeed` over a `Monitor`, in `zenkey-fleet/tests/presence_scenarios.rs`'s
`s9_a_read_beside_a_liveliness_subscriber` (steps 1 to 3 are
`zenkey/tests/presence.rs`'s `s9_a_read_beside_a_liveliness_subscriber`).
Step 4: the runtime counts every liveliness GET per session
(`zenkey::presence::liveliness_gets`); the watch session made one, the
round trip, before the seed and no other, the tool's session every read
the reader made, and the read asked on the watch session was refused, not
counted. Step 5: 10 fresh readers a run, each stalled 150 ms, in 10 of 10
runs: every one heard the token go and read the owner down after its
re-read, 3 s for the ten. The test's control, a reader that skips the
round trip, its link stalled until the withdrawal had reached R1, missed
the withdrawal in 3 of 3 starts in each of the 10 runs.*

## §10 A round trip at a population, and the interests timeout (§8.1, 0.41, 0.42, 0.43, 0.44, 0.45, 0.46, 0.47, 0.49; Appendix B)

**Setup.** Routers R1 and R2, linked R2 → R1. An owner, a tool and two
crowd sessions are clients of R1; the crowd sessions, in a namespace `ns`,
hold 2,000 instance tokens between them (0.42). The steps' own tokens are
under the system `s10`, in no namespace but in step 7. A second tool is a
client of R2. A watch session reaches its router through a link the test
can stall and cut where a step says so; the reader under test's reading
session (step 6) reaches R1 directly (0.42, F-227). The round trip is a
liveliness GET of `zk2/*/*/@zk/settle`, a key no token holds (§1.1: a
token's `@zk` chunk is followed by its kind and more), and waits 5 s; from
a session in no namespace watching `ns`, of `ns/zk2/*/*/@zk/settle`.

**Steps.**
1. The owner holds a token, and the tool reads it. A fresh watch session,
   a client of R1, declares a liveliness subscriber **without history** to
   `zk2/*/*/@zk/instance/*`, its link stalled from before the declaration
   until 150 ms later, and at once makes the round trip. Then the tool
   reads the token (the seed), the owner withdraws it at once, and the tool
   reads it gone. Repeat, ten times.
2. As step 1, without the round trip: the stall ends only after the tool
   reads the token gone; then a round trip says the router has read the
   subscriber. Three times.
3. Steps 1 and 2 with the watch session a client of R2, and the seed read
   by both tools, at R2 and at R1.
4. Beside the crowd, its link not stalled: a fresh watch session in `ns`
   declares a liveliness subscriber without history to
   `zk2/*/*/@zk/instance/*` and makes the round trip at once; then another
   in `ns`, its session's `routing/interests/timeout` 60 s, declares one
   with history and makes the round trip at once.
5. A session in no namespace whose `routing/interests/timeout` is 300 ms,
   a client of R1 through a link the test can stall, reads the steps' own
   instance tokens, `zk2/s10/*/@zk/instance/*`, none held by then (0.42,
   F-231: never the crowd's, whose read may run past 300 ms for its size
   alone), with a 2 s timeout and its link open. Then, its link stalled, it
   reads again with a 2 s timeout, and makes the round trip with a 2 s
   timeout. The reader under test reads each, and says whether its round
   trip stands.
6. (The reader under test, as §8's and §9's.) The reader follows presence,
   the owner's token held: its watch session, whose
   `routing/interests/timeout` is 300 ms (0.44, F-238), reaches R1 through
   the link, and its reading session reaches R1 directly. The watch
   session's link is stalled from before the reader declares its
   subscribers until the reader's first seed has been read, or, for a
   reader that makes no read while no round trip stands, until its first
   round trip has run out under the 300 ms interests timeout (0.44,
   F-238). By then every round trip the reader asked before that seed has
   run out under it (0.42, F-227; 0.43, F-236: how many is the reader's
   own, two for one that makes a round trip at its start and another
   before its seed, one for one whose start's round trip is its seed's).
   Then the stall ends. Then the watch session's link is cut, and
   healed.
7. As step 1 without the stall, the owner and the tool in `ns`, each case
   with a fresh token, subscribed to the steps' own instance tokens
   (`zk2/s10/*/@zk/instance/*`): a fresh watch session in `ns` declares a
   subscriber without history; then another fresh one in `ns` declares one
   with history; then a fresh watch session in no namespace declares one
   without history to the wire key `ns/zk2/s10/*/@zk/instance/*`. One
   subscriber a session (0.42, F-234): a session holding a subscriber with
   history is told the token, and delivers its withdrawal to every
   subscriber it holds.
8. (0.42, F-233.) Beside the crowd, in `ns`: a fresh watch session in no
   namespace declares a liveliness subscriber without history to the wire
   key `ns/zk2/*/*/@zk/instance/*`, and makes the round trip at once, on
   `ns/zk2/*/*/@zk/settle`. Then a crowd token is withdrawn.
9. (0.42, F-228.) A fresh watch session, a client of R1 through a link the
   test can stall and cut, declares a subscriber without history to
   `zk2/*/*/@zk/instance/*`, and its round trip stands. Its link is cut, and
   the reader under test asks a round trip through it. Then, the link
   healed and stalled, a round trip waiting 2 s is asked, and the link is
   cut 150 ms into it (0.43, F-237): once zenoh's own GET, once the reader
   under test's.
10. (0.44, F-239.) Beside the crowd, in `ns`: a fresh watch session in no
    namespace, a client of R1 through a link the test can cut, its
    `routing/interests/timeout` 60 s, declares a subscriber without history
    to `ns/zk2/*/*/@zk/instance/*` and makes the round trip at once, on
    `ns/zk2/*/*/@zk/settle`. Its link is cut. While it is cut, a crowd
    token is withdrawn and a new one declared in `ns`, and the tool in
    `ns` reads both moves. The link is healed. Once the watch session's
    transport has opened again, the reader under test asks its round trip
    four times in a row, giving up on the first three at once and waiting
    30 s for the fourth, through its own interface (0.45, F-244: a tool
    run as a black box offers none, and this half is not run for it).
    Then (the reader under test, as step 6's) a reader follows presence
    beside the crowd, its watch session and its reading session both
    through a link the test can cut, and its caller asks it for its next
    move from a loop that also wakes every 20 ms, which may drop the call
    in flight; a reader whose reads do not run on its caller's call has
    none to drop, and its own waits, each ended after 20 ms, stand in
    (0.45, F-244). Its link is cut for 1 s, and healed.

**Expected.**
1. The round trip is still waiting when the stall ends, and ends with the
   final reply, no error reply and no token. In every run the watch
   session hears the token withdrawn, though its subscriber has no history
   and the token was declared before it: its router tells the withdrawal
   by key.
2. In every run the watch session never hears the token go: its router
   read the subscriber after the token went.
3. As steps 1 and 2: R2 resolves its client's interest itself and tells
   it what R1 withdraws.
4. Without history, the round trip carries no token and ends within one
   round trip, under 1 s. With history, it ends only after the history: by
   then zenoh has delivered all 2,000 tokens to the subscriber (0.42,
   F-230: a binding that runs each callback on a thread of its own may
   still be handing them over).
5. With the link open, the read is complete. Stalled, zenoh ends the read
   and the round trip at 300 ms, each with the final reply alone and no
   error reply. The reader reads the read possibly incomplete, never
   complete, and its round trip not standing.
6. While no round trip stands, each of the reader's reads, a seed or a
   re-read, is possibly incomplete, saying its round trip is not
   established: an address it shows is up, one it does not is
   unobservable, never down (0.42, F-229). It reads again, making the
   round trip first, and once one stands a read is complete again. When
   the link is cut, every address the reader followed is unobservable at
   once, the owner's among them, from the move itself and before any read
   (0.42, F-229: its subscribers may not hear what moved). It reads again
   at once and makes no read and no round trip through its watch session
   cut off, though its reading session holds its link (a read not made,
   F-228): the owner unobservable, never down. After the heal it makes
   the round trip again before the read that finds the owner up.
7. In `ns`, the subscriber without history never hears the token go: its
   router tells the withdrawal by the token's key, and the namespaced
   session does not strip its namespace from that key (Appendix B). With
   history it hears it, and so does the subscriber in no namespace, on the
   wire key.
8. The round trip carries no token and ends within one round trip, under
   1 s, as step 4's did without history, where the watch session in `ns`
   with history waited for the population; and the subscriber hears the
   crowd token withdrawn, on its wire key, though the token was declared
   before it. A reader at a population in a namespace watches so (core
   §8.1, 0.42).
9. Through the watch session cut off, the reader makes no round trip,
   asking no liveliness GET, and none stands. The round trip cut while it
   waits comes back at the cut, well before its 2 s: zenoh's own with the
   final reply alone, no error reply and no token; the reader's does not
   stand.
10. The round trip at the start carries no token, ends within one round
    trip, under 1 s, and the subscriber is told none; the cut tells it no
    withdrawal, since it was told no token. When the transport opens
    again the subscriber is told every crowd token its selector matches,
    the one declared while the link was cut among them; the one withdrawn
    then is never told (Appendix B: the client declares its subscriber's
    interest again in `CurrentFuture` mode). The round trip after the
    reopen ends only after them, as step 4's with history did: by its end
    the subscriber has been told all of them. The reader's round trip
    stands, and it asked one: the three it gave up on ran on, and the
    fourth took it up (0.44). The reader under test: from the cut, every
    address it followed is unobservable; after the heal it makes the round
    trip again, and a read after it is complete within 30 s of the heal,
    the crowd's addresses up. It asks one round trip after the reopen, and
    never a read beside one in flight, however often its caller drops the
    call (0.44). Every count here is the reader's own, taken at its own
    sessions, as step 6's are (0.45, F-244): zenoh tells a router nothing
    of a GET given up on (Appendix B), so no router or link tells one
    given up on from one waited for. A tool run as a black box is run
    with its output's period at most half the cut (0.47, F-249: zk2py
    runs `zenctl watchdog` at `--every 0.5` beside the 1 s cut), so that
    it evaluates at least once while the link is cut. It is judged on what
    its output shows, from its first output after the cut, which must come
    while the link is cut, to its first output that shows a complete read
    after the heal. Every output before that one shows every address it
    followed unobservable, never down, and the complete read comes within
    30 s of the heal. A tool that prints nothing while the link is cut has
    not shown the move, and fails (0.49, F-252). The counts are not asked
    of it (0.46, F-245).

*Measured (0.41, #851) by the reference, in a debug build, in
`zenkey/tests/presence.rs`'s `s10_a_round_trip_at_a_population_and_the_interests_timeout`
(steps 1 to 5 and 7) and `zenkey-fleet/tests/presence_scenarios.rs`'s
`s10_a_round_trip_at_a_population_and_the_interests_timeout` (step 6).
Step 1: heard 10 of 10, each round trip still waiting when its stall
ended. Step 2: missed 3 of 3. Step 3: 10 of 10 and 3 of 3. Step 4: without
history the round trip took 1.4 ms beside the 2,000 tokens, carrying none;
with history 615 ms, every token heard by its end. Step 5: zenoh ended the
stalled read at 300.5 ms, with no error reply; the reference read it
possibly incomplete (`at_interests_timeout`) and its round trip not
established. Step 6: the first seed possibly incomplete, the absent
address unobservable; complete after the stall, one round trip later;
four round trips on the watch session in all (the start's and the first
seed's, which ran out, the retry's and the one after the heal). Step 7:
not heard in `ns` without history; heard with it, and from no namespace.
Under load (`a_round_trip_under_load_misses_no_withdrawal`, 8 at once
beside 300 tokens, no stall): 0 of 160 withdrawals missed with the round
trip, 13 of 160 without it.*

*Measured (0.42, #875) by the reference, the same tests, the crowd in
`ns`: step 4, 8.3 ms without history and 634 ms with, all 2,000 heard by
its end; step 6, the stall 603 ms until the first seed, the owner
unconfirmed by the move at the cut and no round trip asked there, four
round trips in all; step 8, 1.2 ms from no namespace on the wire keys,
carrying none, and the crowd token's withdrawal heard; step 9, no round
trip asked through the session cut off, and the two cut while they waited
back at the cut in 152 ms, zenoh's with no error reply and the reference's
not standing. Through the tools' wiring
(`zenkey-fleet/tests/watch_scale.rs`, a binary of its own): beside 10,000
instance tokens declared in `ns`, a watch session from
`zenkey_fleet::open_watch` made its round trip on `ns/zk2/*/*/@zk/settle`
in 44 ms and heard a crowd token withdrawn after it; a presence monitor
on another stood at its start (36 ms), its feed's seed complete. In `ns`
with history (#872, `a_watch_session_in_a_namespace_waits_for_its_history`)
the round trip beside 2,000 tokens took 0.98–1.02 s, and beside 10,000
about 23 s (below).*

*Measured (0.41, #851) with zenoh-only GETs beside 10,000 tokens, debug
build (`zenkey/tests/presence_scale.rs`'s ignored
`wb_a_barrier_beside_ten_thousand_tokens_measured`): the round trip after
a subscriber with history ended 23.0 s later, 1 ms after the history's
last token; without history, 6 ms. Under zenoh's default 10 s interests
timeout, the round trip after a history subscriber ended at 10.0 s with no
reply from its router, and a read with a 110 s timeout of every token came
back at 10.0 s, with no error reply, holding 6,121 of the 10,000. With a
120 s interests timeout the same read was whole, in 22.3 s. Through a
tool's session (`zenkey_fleet::open_reporting`, which runs with 120 s;
`zenkey-fleet/tests/liveliness_handler.rs`'s ignored
`wb_a_tool_sessions_read_of_ten_thousand_tokens_is_whole`) the read was
complete with all 10,000, and again in 2.6 s; a fresh session on zenoh's
10 s read 6,098 at 10.0 s, which the reference read as possibly
incomplete.*


*Measured (0.44, #882) by the reference, debug builds: step 6 unchanged,
the stall 606 ms, four round trips in all. Step 10 (`zenkey/tests/presence.rs`):
the round trip at the start 1.6–46 ms, carrying none; after the reopen
0.75–1.5 s, the subscriber told 1,999 crowd tokens by its end, the one
declared while the link was cut among them and the one withdrawn then
not; one round trip asked for the four calls (17 runs, 16 of them eight
at a time). Its reader under test
(`zenkey-fleet/tests/watch_reopen.rs`'s
`s10_step10_a_reader_after_a_reopen_beside_the_crowd`, a binary of its
own, both sessions through one relay, the call dropped every 20 ms):
complete again 1.3–4.5 s after the heal, the sessions back 0.3 ms to
2.0 s after it, one read and one round trip asked, in 24 of 24 runs eight
at a time. Before #882 the same reader folded no read in 30 s, asking
1,401 round trips in one run. Beside 10,000 tokens (that file's ignored
measurements): the round trip after the reopen took 3.9–4.1 s in a
release build and 24 s in a debug one, against 49–66 ms at the start;
the fleet's watchdog with `instance-gone crowd/*`, every session through
the relay, read ok again 5–6 s after the heal in a release build (1 s
ticks) and 28–29 s in a debug one, where before it never did within
60 s and 90 s; a debug `zenctl watchdog` beside a release router, 32 s,
where before it never did within 300 s, its stderr carrying 139
`Didn't receive DeclareFinal` lines and 700,070 `Unknown interest`.*
