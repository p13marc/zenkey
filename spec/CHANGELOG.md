# zk2 spec changelog

Amendments to [`core.md`](core.md). Each entry records what changed, what
deliberately did not, and why.

## 0.47 — 2026-10-26: the period a black box is sampled at, and the first read a reader can time (round 71's F-248 and F-249)

zk2py round 71 (PR #889) read 0.46 cold and resolved F-245 to F-247. It
measured again beside 10,000 tokens: a session's first read took
3.13–3.91 s and a later read 0.47–0.50 s. The round trip after each of
three successive reopens took 3.43–4.54 s, and the next 1.6–1.9 ms. It
filed two findings, a minor point and a gap. Wording only; no fixture,
no verdict and no behaviour changes.

**Changed: a reader can time one first read, its seed** (§8.1; F-248,
minor).
- **The gap.** 0.46 said "a reader that times its own reads as it runs
  sees only later reads". A reader's seed, on a session that has not read
  the population before, is a session's first read, the cost the SHOULD
  names. zk2py's follower seeded in 3.1–3.9 s, and its next read took
  0.47–0.49 s. "Only" could lead a reader to drop the one measure it has.
- **Now.** A reader that sizes the wait from its own later reads sizes it
  too short. Its seed is the first read it can time. The round trip after
  a reopen ran to 4.54 s beside seeds of up to 3.91 s, so a wait sized
  from a seed keeps a margin. The SHOULD stands.

**Changed: a black box is run with its output's period below the cut**
(presence §10 Expected 10; F-249, gap).
- **The gap.** 0.46 judged a black box on "every address it followed
  unobservable from the cut". A tool's output can come once a period:
  `zenctl watchdog` prints a transition only when it evaluates, once
  every `--every`, 5 s by default. Beside 2,000 tokens, its link cut for
  1 s:
  - at `--every 0.5`, it printed unobservable 0.47–0.50 s after the cut,
    before the heal;
  - at 2 s, it printed unobservable at 1.91 s, after the heal;
  - at 5 s, it printed nothing after the cut, so its output said ok
    throughout, though it never read down.

  The step set no period, so one tool could pass, fail or be argued
  either way. 0.46's 0.41–0.49 s were measured at `--every 0.5`, which it
  did not say.
- **Now.** A tool run as a black box is run with its output's period below
  the cut; zk2py runs `zenctl watchdog` at `--every 0.5`. It is judged on
  what its output shows, from its first output after the cut until a
  complete read after the heal: every address it followed unobservable,
  never down, and its read complete within 30 s of the heal.

**Deliberately not changed.**
- **No new rule.** The SHOULD stands as 0.44 wrote it. Only how a reader
  sizes it, and how a black box is sampled in §10, are said.
- **zenctl's default `--every`** stays at 5 s. A watchdog that prints
  nothing during a 1 s cut never claimed the crowd was there. What it
  leaves out is a scenario's question, not a defect: its next evaluation
  reads what the cut left.
- **The profiles' "Written against" lines** stay at 0.40.

## 0.46 — 2026-10-26: what a black box is judged on, and which read the round trip after a reopen costs (round 70's F-245 to F-247)

zk2py round 70 (PR #887) read 0.45 cold and resolved F-241 to F-244. It
confirmed 0.45's facts by measurement. The first round trip after a
reopen waited 148–154 ms beside 2,000 tokens and 3.80 s beside 10,000,
and the next 0.6–1.7 ms. Asked once the telling was over, the first took
about 1–2 ms. A GET whose own timeout equals the interests timeout ends
at the interests timeout. It filed three findings: one gap and two minor
points. Wording only; no fixture, no verdict and no behaviour changes.

**Changed: a black box in §10 step 10 is judged on what its output
shows** (Expected 10; F-245, gap).
- **The gap.** 0.45 judged a tool run as a black box "on its read complete
  within 30 s of the heal alone". That dropped a verdict a black box does
  show: every address it followed is unobservable from the cut, never
  down, which §8.1 asks of every reader after a move. As written, a tool
  that read the crowd down at the cut, then ok again, passed. zk2py's
  runner never judged zenctl on the 30 s alone. The wording was its own
  proposal in round 69.
- **Now.** A black box is judged on what its output shows: every address
  it followed unobservable from the cut, never down, and its read complete
  within 30 s of the heal. The counts are not asked of it. zk2py measured
  zenctl unobservable 0.41–0.49 s after the cut.

**Changed: the cost is a session's first read** (§8.1; F-246, minor).
- **The gap.** The SHOULD sized the wait after a move as "about what a
  read of the population costs", and 0.45's F-243 figure was taken on a
  session that had read the population before. Measured by zk2py
  (zenoh-python 1.10.1, release, beside 10,000 tokens in `ns`):
  - a session's first read took 3.3–4.2 s, on 11 fresh sessions;
  - a later read took 0.52–0.58 s;
  - a read after a reopen was a later read, 0.55 s;
  - the round trip after a reopen took 3.7–4.9 s, about a first read.

  A reader that sized its wait from its own later reads would wait 7–9
  times too short. Behind a read its own timeout ended, the next round
  trip waited 266–280 ms on a session that had read before, and 2.6–3.4 s
  after the session's first read.
- **Now.** The SHOULD names a session's first read, with zk2py's figures,
  and the F-243 figure gives both cases. The rules are unchanged.

**Changed: which of zk2py's readers asks its GETs at the interests timeout**
(§8.1; F-247, minor). 0.45 said "zk2py does so for its round trips and
reads". Only its follower does. Its profile readers ask each GET with the
caller's wait as its own timeout, as their texts, written against 0.40,
let them. Now it reads "zk2py's follower".

**Deliberately not changed.**
- **No new rule.** The SHOULD and the MAY stand as 0.44 and 0.45 wrote
  them. Only what they cost and who follows them are corrected.
- **The reference's wait after a move** is `--timeout` or the monitor's
  5 s, whichever is larger. A round trip not back by then runs on and is
  taken up, so a short wait costs a possibly incomplete read, never a
  second GET.
- **The profiles' "Written against" lines** stay at 0.40.

## 0.45 — 2026-10-26: which round trip a reopen costs, and what taking up a GET means (round 69's F-241 to F-244)

zk2py round 69 (PR #885) read 0.44 cold, resolved F-238 to F-240, and
filed four findings: one measured contradiction, a minor wording point and
two gaps. It also ran round 68's watchdog run again against the fixed
reference: a debug `zenctl watchdog` beside 10,000 tokens read ok 25–27 s
after the heal, in 3 of 3 runs, with none of zenoh's `Unknown interest`
lines. Wording only; no fixture and no verdict changes.

**Changed: only the first round trip after a reopen waits for the
population** (§8.1; F-241, contradiction).
- **The gap.** 0.44 said the watch session's round trip is one round trip
  at any population "only until its transport first moves", and 0.42's
  bullet "until its transport moves, when it waits for the population".
  Read as written, every round trip after the first move waits for the
  population. Measured by zk2py (zenoh-python 1.10.1, release; round trips
  asked one after another, the link cut and healed): beside 10,000 tokens
  the first after the reopen took 3.7–4.2 s and the next three 0.8–1.7 ms;
  beside 2,000, 145–146 ms, then 0.3–0.8 ms. 0.44's own "Deliberately not
  changed" said it right: paid once at each reopen.
- **Now.** One round trip at any population but for the first after each
  reopen of its transport, which waits for the reopening's telling; once
  that has been sent, the next is one round trip again. 0.42's bullet says
  the same.

**Changed: a wait that ends first takes up the round trip in flight**
(§8.1; F-242, minor).
- **The gap.** The first SHOULD said a reader whose wait ends first "makes
  the round trip again", and the next bullet said not to ask one beside
  one in flight but to take it up. §10's Expected 10 read it the second
  way.
- **Now.** It reads possibly incomplete and, before its next read, takes
  up the round trip in flight, asking another only once that one has ended
  without standing. That is what both implementations do.

**Changed: what in flight means, and a GET its own timeout ends**
(§8.1, Appendix B; F-243, gap).
- **The gap.** "The next that needs one takes up the one in flight, its
  caller's wait or its own having ended" could read "its own" as the GET's
  own timeout. A GET that its own timeout ends cannot be taken up: zenoh
  hands it an error reply, `Timeout`, and its final reply, and nothing
  after them (`api/session.rs`, `liveliness_query`). Its router answers it
  in full all the same. Measured by zk2py beside 10,000 tokens: a GET asked
  on that session meanwhile took 258–266 ms, against 6–10 ms on a quiet
  session.
- **Now.** The wait that ends first is the caller's or the reader's own.
  In flight is at the reader: from the GET's ask until its final reply
  reaches the reader. A GET its own timeout ends is over for the reader
  and cannot be taken up, and the text gives its cost. A reader MAY ask
  each GET with its own timeout at or above its session's interests
  timeout and bound its own wait apart, so that none ends before it can
  take it up. zk2py does so for every GET of its follower. The reference
  does so for its round trip, and its presence feed asks a read with the
  reader's timeout and backs off, 1 s at first, after one that runs to it.
  Appendix B gains the fact.

**Changed: §10 step 10's counts are the reader's own** (F-244, gap).
- **The gap.** Expected 10's "asked one", "one round trip after the
  reopen" and "never a read beside one in flight" can only be counted at
  the reader's own sessions. zenoh tells a router nothing of a GET given up
  on, so no router or link can tell it from one waited for. The step's
  "giving up on the first three at once" needs an interface a tool's
  command line does not offer. Its "caller … may drop the call in flight"
  tests nothing for a reader whose reads do not run on its caller's call.
- **Now.** The four round trips go through the reader's own interface, and
  that half is not run for a tool run as a black box. A reader whose reads
  do not run on its caller's call has its own waits, each ended after
  20 ms, stand in for the dropped calls. Every count is the reader's own,
  as step 6's are. A tool run as a black box is judged only on its read
  being complete within 30 s of the heal, which zk2py's runner already did
  for zenctl.

**Deliberately not changed.**
- **No new MUST or SHOULD.** The SHOULD NOT reads as 0.44 meant it. The
  one new rule, a GET asked with its own timeout at or above the interests
  timeout, is a MAY: a GET its own timeout ends costs its router work and
  the next GET a wait, but not the reader's honesty. A read that runs to
  its own timeout is still possibly incomplete.
- **The reference's reads** keep the reader's timeout. A read that runs to
  it is retried after a back-off, so at most one more GET waits behind
  each, which is the cost F-243 measured.
- **The profiles' "Written against" lines** stay at 0.40.

## 0.44 — 2026-10-25: a watch session's reopen beside a population (#882, round 68's F-238 to F-240)

zk2py round 68 (PR #881) read 0.43 cold and filed three findings, and
measured what F-239 rests on: a watch session's transport that opens
again tells its subscribers without history every token they match, and
the round trip after it waits for them. Its run of a debug zenctl beside
10,000 tokens never read `instance-gone` ok again after a 3 s cut (#882).
The reference reproduced both and found why zenctl did not recover: its
loop dropped the presence re-read in flight four times a second, and
zenoh answers a GET its caller drops in full, so reads piled up at the
router and none was ever folded. Fixed in the reference (WF); measured in
`scenarios/presence.md` §10, step 10 new.

**Changed: what a move leaves, and what the round trip after it costs**
(§8.1; F-239).
- **The gap.** 0.42 gave the move rule's reason as "a token withdrawn
  then, or one declared then, of which a subscriber without history is
  never told", and called the watch session's round trip "one round trip
  at any population". Measured by zk2py (zenoh-python 1.10.1, release) and
  the reference (Rust, debug and release): when the transport opens again
  a subscriber without history is told every token it matches, one
  declared while the link was down included, and a token withdrawn then
  is never told. The round trip after the reopen ends after that telling:
  beside 10,000 tokens 3.2–4.7 s (zk2py) and 3.9–4.1 s (the reference,
  release), 24 s (debug), against 41–66 ms at the session's start; beside
  2,000, 134–151 ms (release). zenoh's client declares each interest of
  its session again in `CurrentFuture` mode at a reopen, whatever its
  mode (`hat/client/interests.rs`, `repropagate_interests`).
- **The rule stands; its reason is corrected.** While the transport is
  down nothing is heard: a token withdrawn then is never told, and one
  declared then is told at the reopen, unordered with the reader's reads.
  So what the reader held is unconfirmed from the move until a complete
  read after a round trip that stands, as 0.42 said.
- **New: the round trip after a move waits for the population.** "One
  round trip at any population" holds until the transport first moves. A
  reader SHOULD wait for the round trip after a move, and run its
  interests timeout, above that time at its population, about what a read
  of it costs.
- **New: one round trip, and one read, at a time.** zenoh 1.10.1 tells a
  router nothing of a liveliness GET whose caller drops it, times it out
  or cancels it (`api/session.rs`), so the router answers it in full, and
  after the interests timeout the client logs every late reply `Unknown
  interest`. A reader SHOULD NOT ask a round trip while one it asked since
  its watch session's transport last moved is in flight, nor a read while
  one of the same scope is: the next that needs one takes it up. This is
  #882: the reference's watchdog dropped its re-read at every wake of its
  loop, beside 10,000 tokens whose read takes 0.5–3 s; 450 reads were asked
  in 120 s and none folded, and after 120 s its stderr carried zenoh's
  `Didn't receive DeclareFinal … Timeout(120s)!` (139 in 300 s) and
  `Unknown interest` (700,070).
- **Measured, the fix** (`zenkey::presence::Barrier`: one round trip per
  transport state on a task of its own, shared by every waiter;
  `PresenceFeed` and zenwatch's ledger: a re-read's round trip and read on
  tasks that outlive the call): beside 10,000 tokens, complete again
  4.4–4.5 s after the watch session's reopen in a release build; the
  fleet's watchdog ok 5–6 s after the heal (release, 1 s ticks) and
  28–29 s (debug), where it never was. Step 10's reader, its call dropped
  every 20 ms beside 2,000 tokens: one round trip and one read after the
  reopen, complete 1.3–4.5 s after the heal, 24 of 24 runs; before, no
  read folded in 30 s.
- Appendix B gains both zenoh facts. An upstream report is drafted, not
  filed: `docs/zk2/upstream/liveliness-get-dropped-is-answered-in-full.md`.

**Changed: §10 step 6's stall, for a reader that makes no read while no
round trip stands, and the interests timeout's owner** (F-238).
- **The gap.** 0.43 ended the stall at the reader's first seed. §8.1 does
  not oblige a reader to seed while no round trip stands, and one that
  carries the cut-off caution over to a round trip that ran out makes no
  seed until one stands, which under the stall never happens. And the
  step gave the 300 ms interests timeout to the link; it is the watch
  session's.
- **Now.** The stall ends at the reader's first seed, or, for a reader
  that makes no read while no round trip stands, when its first round
  trip has run out under the 300 ms interests timeout; the timeout is the
  watch session's. Expected 6 is unchanged. Both implementations seed
  after a round trip that ran out, so neither run moves.

**Changed: a round trip ended at the interests timeout reads as not
standing** (§8.1; F-240).
- **The gap.** The MUST read a round trip that ended at the interests
  timeout "as not made". Elsewhere in §8.1 and §10 a round trip or a read
  "made" is one asked, and "not made" one never asked (Expected 9: "makes
  no round trip, asking no liveliness GET"); Expected 5 calls the same
  round trip "not standing".
- **Now.** "As not standing": it was asked, and it establishes nothing.
  §8.1's other use of the phrase for it, a history's round trip past the
  interests timeout, says so too. Every other "made" and "not made" in
  §8.1 and §10 already meant asked and never asked, and no doc comment or
  message of the reference quoted the phrase; the reference's `settle`
  and `Barrier` read such a round trip as not established.

**Also, outside the text** (round 68): zenctl worded a read ended at the
interests timeout as one that "ran to its timeout … raise --timeout to
ask again". Its reports that keep only `complete` now name both bounds,
the interests timeout by its key, and a view that holds the read says
which (`zenkey::presence::AT_INTERESTS_TIMEOUT`).

**Deliberately not changed.**
- **The move rule.** A move still unconfirms what the reader holds at
  once, before any read (0.42, F-229): the reopen's telling is unordered
  with the reader's reads, and a withdrawal while the link was down is
  never told, so a token told at the reopen is evidence it is held, never
  evidence of what went.
- **No new MUST.** The two new rules are SHOULDs: a reader that waits less
  for its round trip still reads honestly, possibly incomplete, and one
  that abandons a read costs its routers work and itself its progress,
  not its honesty.
- **A watch session in no namespace, without history** (0.42, F-233)
  stands: a history pays the population at every round trip, where this
  pays it once at each reopen.
- **Step 6's counts** stay the readers' own; the reference still makes
  four round trips in all there.
- **The profiles' "Written against" lines** stay at 0.40.

## 0.43 — 2026-10-25: two of §10's steps worded for any reader (round 67's F-236 and F-237)

zk2py round 67 (PR #879) read 0.42 cold, resolved F-227 to F-235, and
filed two minor findings, both on `scenarios/presence.md` §10's wording.
No rule changes.

**Changed: step 6's stall, by its event alone** (F-236).
- **The gap.** Step 6 ends the stall "until the reader's first seed has
  been read", then states what that takes, "two round trips have run out,
  the start's and the seed's". That is the reference's count: its reader
  makes a round trip at its start and another before its seed. zk2py's
  makes one before each read, so its start's round trip is its seed's,
  and when its first seed was read one had run out (302 ms; the stall
  304–308 ms). A runner ending the stall at two would end it, for such a
  reader, after its retry's seed, not its first.
- **Now.** The stall ends at the event; by then every round trip the
  reader asked before that seed has run out, how many being the reader's
  own (two for the reference, one for zk2py). Expected 6 is unchanged:
  it never counted them. The reference's measurement (four in all, two
  before the first seed) stands as a measurement.

**Changed: step 9's round trip waits 2 s and is cut 150 ms in** (F-237).
- **The gap.** The Setup's round trip waits 5 s, step 9 stated no wait,
  and Expected 9 says "well before its 2 s". The verdict holds under any
  wait (both implementations' came back at the cut, 151–152 ms in), but a
  runner could not tell which the step meant.
- **Now.** Step 9 says it: a 2 s wait, the link cut 150 ms into it, as
  the reference and zk2py both ran it.

**Deliberately not changed.**
- **§8.1** and every rule: both findings are a step's wording, and no
  verdict differed between the implementations.
- **The profiles' "Written against" lines** stay at 0.40.

## 0.42 — 2026-10-25: a watch session on the wire keys, and round 66's findings (#875, WD)

zk2py round 66 (PR #876) read 0.41 cold and filed F-227 to F-235, all on
§8.1, §9 and §10. F-233 is #875's own subject: in a namespace, 0.41 offered
a watch session with history and one in no namespace on the wire keys as
equals, and the first brings the population back into every round trip.
The reference's tools now take the second (`zenkey_fleet::WatchSession`,
`open_watch`), measured below. Every finding is answered here, each by a
change or a reason it stands.

**Changed: in a namespace, a watch session in no namespace on the wire
keys** (§8.1; F-233).
- **The gap.** 0.41's "keeps history, or watches the wire keys" did not
  weigh the two. With history the round trip waits for the population:
  23 s beside 10,000 tokens (the reference, debug), 3.2–3.9 s (zk2py,
  release), past the scenarios' 5 s wait and zenoh's 10 s interests
  timeout, where 0.41's own MUST makes it not made.
- **Measured** by the reference: beside 2,000 tokens in `ns`, a watch
  session in no namespace, its subscriber without history on
  `ns/zk2/*/*/@zk/instance/*`, made its round trip on
  `ns/zk2/*/*/@zk/settle` in 1.2 ms and heard a crowd token withdrawn
  (§10 step 8, new); beside 10,000 in `ns`, through the tools' wiring, in
  44 ms (`zenkey-fleet/tests/watch_scale.rs`). In `ns` with history: 634 ms
  beside 2,000 (step 4).
- **The rule.** A reader that follows presence in a namespace SHOULD hold
  its watch session in no namespace, its subscribers without history and
  its round trip on the wire keys, the namespace in front; it strips the
  namespace from what it hears, and counts a key outside it, never folding
  it. One that keeps history in the namespace SHOULD run its round trip's
  wait and its interests timeout above the history's time. The grants are
  unchanged: the wire keys are what a namespaced session sends, and what
  §11 compiles under the namespace (a test over the walkthrough's plan,
  `zenkey-fleet/tests/acl_plans.rs`). A watch session in no namespace makes
  no GET of the deployment's keys: there they would be the bus root's.

**Changed: no round trip through a watch session cut off** (§8.1; F-228).
- **The gap.** A round trip is a liveliness GET whose answer is
  discarded, and the cut-off rule was written for reads: through a session
  cut off zenoh answers it at once, final reply alone, which read by the
  text as written stands. zk2py measured it in 0.2–3 ms, 10 of 10.
- **Measured** by the reference (§10 step 9, new): through a watch session
  cut off no round trip is asked and none stands; one cut while it waits
  comes back at the cut, 152 ms into a 2 s wait, zenoh's own with the final
  reply alone and no error reply, the reference's not standing. The
  reference already checked the links before and after (`zenkey::presence::settle`).
- **The rule.** A round trip through a watch session that holds no link,
  or held none when it ended, does not stand, whatever its replies say; a
  reader checks the links before and after, asks none through a session
  cut off, and makes it when the transport opens.
- **And with it** (F-227's first point): a reader whose watch session is
  cut off follows nothing, so it SHOULD NOT make a read it would follow
  from it, whatever its reading session's links; it reports the read not
  made, its reason the watch session's. The reference did so already
  (`PresenceFeed`, the token ledger).

**Changed: a move unconfirms what the reader holds, and "a seed" is every
read** (§8.1; F-229).
- **The gap.** Through a cut, the owner's token was never told to a
  subscriber without history, so nothing was heard withdrawn, and the
  re-read was not made, which removes nothing: a reader doing only what
  0.41 said read the owner up throughout the cut (zk2py: every sample over
  1.5 s), where §10 Expected 6 said unobservable. An address read down
  before the move is no longer followed either.
- **What the reference did.** Its feed owed a re-read at once at the move
  and held what it held meanwhile: a cut was unobservable once the re-read
  found the watch session cut off, a reconnection after one barrier and
  one read. It now unconfirms at the move itself: `LivePresence::moved`
  (`PresenceBasis::Moved`, a `moved` transition source), the alarm
  reader's loss (`AlarmLoss { stalled: true }`), the token ledger's
  `TokenBasis::Moved`; telemetry already folded the move as a loss.
- **The rule.** From the move until a complete read made after a round
  trip that stands, every token and absence the reader followed through
  the watch session is unconfirmed: an address unobservable, never up or
  down on what was held before. And "a seed made while no round trip
  stands" is now "a read, a seed or a re-read". §10 Expected 6 says both;
  the reference's step 6 checks the owner unconfirmed by the move itself.

**Changed: an interests timeout the reader cannot read, and where a read's
time starts** (§8.1; F-232).
- **The gap.** The read carries no sign of the interests timeout, so the
  MUST needs the session's value, which zenoh-python 1.10.1 cannot read
  back from an open session; and the text did not say where the time is
  counted from.
- **The rule.** The time is counted from before the GET is asked to its
  final reply. A reader that cannot read the value SHOULD open the session
  itself with one it sets; a session it is handed and cannot read, it
  takes at zenoh's 10 s, which holds only when the session states none,
  and it says its reads' completeness rests on that. The reference reads
  the value from `Session::config()`, and counts from before the GET is
  asked (`zenkey::presence`, `get`).

**Changed: §10's setup and steps** (`scenarios/presence.md` §10; F-227,
F-230, F-231, F-234).
- **F-227.** The reader under test's watch session reaches R1 through the
  link and its reading session directly, as the reference runs it; the
  stall lasts from before its subscribers until its first seed is read,
  two round trips run out under the 300 ms interests timeout (603 ms for
  the reference). With the watch session alone on the link, "a read not
  made" at the cut holds by the new rule above.
- **F-230.** Expected 4: "by then zenoh has delivered all 2,000 tokens to
  the subscriber", a binding that runs each callback on a thread of its
  own possibly still handing them over.
- **F-231.** The crowd is in `ns`, and step 5 reads the steps' own system,
  `zk2/s10/*/@zk/instance/*`, never the crowd's: a read beside the
  population may run past 300 ms for its size alone. Step 4's watch
  sessions are in `ns` with the crowd; step 7 subscribes to `s10`'s tokens.
- **F-234.** Step 7: a fresh watch session for each subscriber, and a fresh
  token each time. One session holding a history subscriber is told the
  token and delivers its withdrawal to every subscriber it holds (zk2py,
  5 of 5); the reference ran one session per subscriber already.

**Changed: §9's miss is a race, nearly always lost** (`scenarios/presence.md`
§9, Appendix B; F-235).
- **The contradiction.** §9 Expected 2 and Appendix B said the GET at once
  beside a history subscriber misses "every time"; zenoh-python 1.10.1
  held the token in 2 of 60 trials beside the reference's owner and 5 of
  60 beside zk2py's.
- **Now.** Step 2 runs ten times on fresh sessions, and Expected 2 asks
  the miss in one run at least, nearly always: the session's own answer
  can win the race, and a GET that holds the token proves nothing safe.
  Appendix B gives both counts. The reference's step 2 missed 10 of 10,
  and asserts one miss at least.

**Deliberately not changed.**
- **The namespace rule stays a SHOULD**, beside the history option: a
  reader at a small population, or one whose grants or deployment keep it
  in the namespace, may keep history and pay for it, by the new SHOULD on
  its waits.
- **No key is reserved and no grant shape changes** (§1.1, §11.1): the
  wire keys are a namespaced session's own on the wire.
- **The runtime's owner-side watches** (an archive's owners, `desired.v1`'s
  target, `redundancy.v1`'s candidate) are unchanged: each watches one
  address's handful of tokens, so their history costs a few tokens, and
  they run on a session the application hands in.
- **"Complete" keeps its conditions** (0.37 to 0.41). A read made while
  the watch session is cut off is not made, a reason beside the reading
  session's own; the move's unconfirming is a reader's state, not a
  read's.
- **The profiles' "Written against" lines stay at 0.40**: none names the
  watch session's namespace or a move.

## 0.41 — 2026-10-25: a watch session's round trip at a population (#851, WB)

#849 (PU) found that a reader's round trip on its watch session, a
liveliness GET of the subscribers' own selector, outlasted the fleet's 5 s
cap beside 10,000 tokens, so the seed went without it: the race 0.37's
round trip exists to close. Measuring it (zenoh 1.10.1, the reference, a
debug build) showed that the cost is not the GET's own tokens. Its final
reply follows the subscribers' history, which took 23 s beside 10,000
tokens; the "about 10 s" was zenoh's own interests timeout ending the GET
with no error reply. Three facts went into §8.1 and Appendix B, each run by
the new [`scenarios/presence.md`](scenarios/presence.md) §10.

**Changed: the round trip's GET need not match the subscribers'
selectors** (§8.1, Appendix B).
- **Measured.** A GET of `zk2/*/*/@zk/settle`, a key no token holds, made
  at once after a subscriber on a link held stalled 150 ms, was still
  waiting when the stall ended, and the withdrawal after it was heard in
  10 of 10 runs; without it, missed in 3 of 3. The same through R2, its
  client's interest resolved there: 10 of 10, and 3 of 3. Under load, no
  stall, 8 at once beside 300 tokens: 0 of 160 missed with it, 13 of 160
  without. zenoh's code agrees (`dispatcher/interests.rs`: a session's
  interests are handled in turn, the subscriber's registered and its
  history sent before the GET's final reply; `hat/router/interests.rs`: a
  router resolves a client's interest itself).
- **The key** is `…/@zk/settle` under the subscribers' presence subtree,
  so a grant that admits them admits it; one access control refuses is
  still ordered (5 of 5: zenoh answers it at ingress, in turn).

**Changed: the watch session's subscribers SHOULD have no history, except
in a namespace** (§8.1, Appendix B).
- **Measured.** Beside 2,000 tokens the round trip after a subscriber
  without history took 1.4 ms and carried nothing; with history, 615 ms,
  every token heard by its end. Beside 10,000: 6 ms, and 23 s. A token
  declared before a subscriber without history, withdrawn after the round
  trip, is heard: the router tells the withdrawal by key
  (`hat/broker/token.rs`).
- **The namespace.** A session in a namespace does not strip it from that
  key (`routing/namespace.rs`), so its subscriber without history never
  hears the withdrawal; with history it does, and so does a session in no
  namespace on the wire key (§10 step 7). A watch session in a namespace
  keeps history, or the reader watches from no namespace.
- **A round trip stands until the watch session's transport moves**: a
  reconnection re-declares the subscribers unordered with the other
  session's reads, and a subscriber without history hears no withdrawal
  at a link's drop of a token it was never told of. A reader makes the
  round trip again and reads again when the transport closes or opens,
  and reads a seed made while none stands as possibly incomplete (§10
  step 6).

**Changed: a read that runs to its session's interests timeout is not
complete** (§8.1, Appendix B).
- **Measured.** zenoh 1.10.1's runtime ends a liveliness GET its routers
  have not finished at `routing/interests/timeout` (10 s by default),
  with the final reply alone and no error reply. A read with a 110 s
  timeout beside 10,000 tokens came back at 10 s holding 6,121 of them;
  under a 120 s interests timeout the same read was whole, in 22 s. A
  stalled read under a 300 ms interests timeout ended at 300 ms (§10 step
  5).
- **The rule.** A reader MUST read a read whose time reached its
  session's interests timeout as possibly incomplete, and a round trip
  ended there as not made; one whose GETs may wait longer SHOULD raise
  it. The reference's tools run with 120 s.
- `docs/zk2/upstream/liveliness-get-ends-at-interests-timeout.md` drafts
  the upstream report, not filed.

**Deliberately not changed.**
- **The round trip stays a SHOULD**, with what serves it unchanged in
  substance: "a liveliness GET there whose answer it discards" already
  allowed any key. The new bullet says one that matches no token serves,
  and what that costs.
- **"Complete" keeps its other conditions** (0.37, 0.38, 0.40): how the
  read ended, its session never having held a liveliness subscriber, its
  links held. The interests timeout is one more way a read ends without
  its routers' final reply, read as the timeout's error reply is read.
- **No key is reserved** in §1.1: `…/@zk/settle` is not a token form, and
  nothing is declared or answered on it. A reader may ask any key no token
  holds.
- **Appendix B's "a GET the routers finished ends with no error reply"
  stays**: true of every GET that ended before the interests timeout,
  which the new fact bounds.
- **The profiles' "Written against" lines stay at 0.40**: none names the
  round trip's key or a watch's history. `desired.v1` and `redundancy.v1`
  cite the round trip as core §8.1's, which it still is.

## 0.40 — 2026-10-25: what `redundancy.v1`'s runtime measured (#841, RB)

The second implementation read `redundancy.v1` 0.1 and core 0.39 cold
(zk2py round 56, PR #855) and filed F-199 to F-209, all accepted on #841.
Two of them are facts about zenoh that the profile's election rests on, and
the maintainer asked that the reference measure each in Rust before the
core states it (#841's answers). Both reproduced. They are §8.1's, as
reading presence is, and change no rule a reader already follows: the
first is an implementation note, the second says what a complete read
covers. No fixture case changed, and none is added: each is a fact about a
bus.

**Changed: a read on a token's own session answers that token itself**
(§8.1, Appendix B; F-204).
- **The gap.** `redundancy.v1` 0.1 let a candidate read the claim set on
  the session holding its claim. zk2py measured that a session's
  liveliness read returns the session's own tokens even through a router
  that refuses the read, so the candidate's check that the read holds its
  own claim, the one thing that tells a refused read from a lone
  candidate's, never sees a refusal there. §5 step 3 of its scenarios
  failed that way, the instance serving through the refusing router.
- **Measured** by the reference on zenoh 1.10.1: through a router whose
  access control denies `liveliness_query` on ingress, the holder's read
  was complete and held its token, and another session's the same read
  complete and empty, five times of five
  (`zenkey/tests/presence.rs`,
  `a_refused_read_on_a_tokens_own_session_still_returns_it`).
- **The note.** A read on a token's own session says nothing about the
  bus for that token; a reader that must see its own token as the bus holds
  it reads on a session that does not hold it. `redundancy.v1` 0.2 makes
  that a MUST of its candidates (its §2.5 step 1).

**Changed: what a complete read covers** (§8.1, Appendix B; F-206).
- **The gap.** A complete read was defined by how it ended and by its
  session, never by what it can reach. zk2py measured two peers behind one
  router, neither listening: one's read of the other's token was complete
  and empty every time for 15 s, while a client of the router read it. A
  candidate that is such a peer reads a vacancy beside a live incumbent.
- **Measured** by the reference on zenoh 1.10.1, against its source: a
  router's `auto` gateway puts its peers in one region, whose members it
  expects to be linked to each other, and propagates no token from one of
  them to another (`hat/peer/token.rs`, `propagate_token`). The answer's
  `routing.router.peers_failover_brokering` is deprecated in 1.10.1, read
  and ignored (`zenoh-config`), so its default changes nothing. Two peers,
  neither listening, with gossip on and with it off: the reader's read was
  complete and empty in six reads over 3 s, while a client of the same
  router read the token. Both listening, and gossiping through a router
  that gossips (zenoh's default): they linked, and the token was read
  (`a_peers_read_through_a_router_misses_an_unlinked_peers_token`).
- **The rule.** A read is complete only for the tokens its routers'
  clients and its linked peers hold. A peer that other peers cannot link
  to reads none of theirs, and its complete read establishes no absence of
  them. A reader that concludes absence of a peer's token SHOULD be a
  client of a router, or a peer linked to that peer; `redundancy.v1` 0.2
  makes it a MUST of its candidates and the tools that read them.
- **Appendix C** lists `profiles/redundancy/scenarios.md` §5, whose peers
  link (step 1) and whose reads go on a session apart from the claim
  (step 3).

**Deliberately not changed.**
- **"Complete" keeps its definition** (0.37, 0.38): how a read ended, its
  session never having held a liveliness subscriber, its links held before
  and after. The new bullet says what such a read covers; it adds no check
  a reader can make of its own read, since nothing on the session says
  which peers it is linked to.
- **No MUST for the core's readers.** Which sessions a reader runs, and
  whether a deployment's peers link, are a deployment's, and the profile
  that elects by presence states its own MUST. The SHOULD is for every
  other reader.
- **F-201** (a crash seen at once when its kernel closes the links) needs
  no core change: Appendix B already says both a link that closes and one
  that goes silent. `redundancy.v1` 0.2 cites it.
- **F-202** (a put's and a token's order at a subscriber) needs none
  either: §8.2's order is observed by a GET at the token, as
  `presence.md` §1 already observes it, and `redundancy.v1`'s scenarios
  now say so.

## 0.39 — 2026-10-24: what `redundancy.v1` needed of the core (#841)

`redundancy.v1`'s first text (#841, RA) is the profile the core has left
exclusivity to since 0.1: "election and fencing belong to
`redundancy.v1`" (§6), and O1's at most once holds only while one
instance serves (spike S6). The core already had a standby, without a
mark: §6 said it "exposes no exclusive resource, and declares no
interface token for an interface it exposes nothing of", and O3 that it
declares no operation queryable. Its descriptor listed every interface
the instance implements, exposure is compact (§3.3), and so a standby's
listing claimed the interface's resources exposed: the shape §8.1 calls
"an exposed interface with no token", which the reference doctor's
`token-missing` reports, and the standby that `hostid.v1` §2.12 cannot
tell from a twin. No fixture case changed; six are added, five
descriptors and one contract. The number follows 0.38, which answers the
second implementation's round 54 on 0.37's presence reads; 0.39 changes
none of 0.37's or 0.38's reading rules.

**Changed: the standby mark (§3.3, §6).**
- **The rule.** An interface entry of the descriptor MAY carry
  `"standby": true`: the instance implements the interface, lists it, and
  serves none of it now. Absent is serving, and `false` written out the
  same, as a role's `optional` is (0.10). §6 defines the standby by it:
  it exposes none of the interface's resources, declares none of its
  operation queryables, `unavailable` ones included, and holds none of
  its interface or member tokens; it holds its instance token, serves its
  descriptor, and still holds the interface's bundle, which is not one of
  its resources. `[F: descriptors/ok-standby, ok-standby-false]`
- **Its `unavailable` and `cardinality`** state what the instance will
  expose once it serves, checked as any entry's (D005–D007), and are never
  read as exposure: a standby entry exposes nothing, whatever else it
  says. A runtime then flips one member to take over, and a tool sees a
  standby's readiness (a capability it lacks) before a takeover.
- **Only as a profile says.** The core defines what a standby is, holds
  and exposes, and no election: when an instance stands by, and when it
  serves, is `redundancy.v1`'s for an interface whose contract uses it.
  The mark is the core's own member, not a fifth extension point (§10).
- **The type.** A `standby` that is not a boolean is not the record's
  shape: D000. `[F: descriptors/d000-standby-type]`

**Changed: a standby holds no interface token and exposes nothing (§8.1,
§8.2).** §8.1's interface token says a standby holds none, nor a member
token. §8.2 says a standby exposes none of the interface's resources,
required ones included, which step 2 does not refuse, and that it
validates at its start what it will expose once it serves, so that a
takeover cannot fail step 2. `[Sc: profiles/redundancy/scenarios.md §1]`

**Changed: serving later, and stopping (§8.2).** An instance that starts
to serve an interface while it runs, a standby taking over, does steps 1
to 4 for that interface in their order: its resources, then its
descriptor put again without the mark, then the interface's token. One
that stops serving undoes them in the reverse order: the tokens first,
then the resources, then the descriptor put marked. Alive ⇒ callable holds
at every step, as at a start. `[Sc: profiles/redundancy/scenarios.md §1,
§7]`

**Changed: an interface with a standby is never tokenless (§3.3 D012,
§8.1, §8.2).**
- **The gap.** The incumbent of an interface is the instance holding its
  token (`redundancy.v1` §2.3), and a tokenless interface has none, so an
  election over one has nothing to elect by, and the split-brain check
  (§6) does not cover it.
- **D012**: an entry marked both `"standby": true` and `"token": false`.
  It reads `standby` and `token` alone, as D011 reads `iface` and `token`
  (cascade 7, new): a malformed `contract` does not suppress it, and an
  `archive.v1` entry so marked gives D011 and D012.
  `[F: descriptors/d012-standby-tokenless, d012-bad-fingerprint]` The old
  cascade 7 ("not checked") is now 8, and §3.3's one reference to it moved.
- **The tokenless set** never names an interface whose contract uses
  `redundancy.v1` (§8.1), as it never names `archive.v1`, and §8.2 step 2
  refuses an owner whose set names one it implements.
  `[Sc: profiles/redundancy/scenarios.md §11]`

**Changed: the split-brain check reads the mark (§6).**
- **A holder is an incumbent, whatever its mark.** The check counts the
  instances holding the token, as before; one whose descriptor marks the
  interface standby is counted, and its descriptor disagrees with its
  tokens besides (§8.1's "a token for an interface it exposes nothing
  of").
- **Reading the mark.** An instance listing the interface marked, holding
  no token of it in a complete read, is a standby: no finding of the
  check, nor of §8.1's shapes, since its listing claims nothing exposed.
  One listing it unmarked, outside its tokenless set, holding no token of
  it in two complete reads a grace apart, claims an exposure no token
  covers. `redundancy.v1` §5 states the questions a tool answers of an
  address and of one instance. `[Sc: profiles/redundancy/scenarios.md
  §12]` `operations.md` §8's step 3 says its standby is marked.
- **Not fencing.** The runtime still fences nothing, but for an interface
  whose contract uses `redundancy.v1`, whose holders step down as its
  §2.9 says; writes already in flight still land, so that is no fence
  either.

**Changed: `redundancy.v1`'s vocabulary published (Appendix D, §10).**
- **The rule.** `redundancy.v1` §4 publishes the interim row's one key
  unchanged, `election` (`claim`, the default, or `external`), which a
  contract writes once on `[defaults]` (its §2.2). Appendix D's published
  table gains `redundancy`, and the interim row is gone. W105 reads it as
  it reads any other (0.21): a contract writing `redundancy.election`
  draws nothing, and one writing another `redundancy.<key>`, a fencing
  epoch among them, draws W105. `[F: contracts/w105-redundancy-epoch]`
- **§10** names `redundancy.v1` among the profiles the core names, for
  exclusivity, and its vocabulary beside the published ones (point 2).
  Appendix C lists the profile's scenarios that observe core rules: the
  mark, a standby's tokens and exposure, the order of a takeover and a
  step-down, the tokenless refusal, a call finding no standby, and a tool
  reading the mark.

**Deliberately not changed.**
- **No election in the core, and no role beyond the mark.** Standing by
  and serving are the two states a tool reads; an election, a preferred
  instance or a priority would be a profile's, and `redundancy.v1` has
  none of the last two at 1.0.
- **The descriptor's format** stays `zk2-descriptor/0.1`. The member is
  optional, absent meaning serving, as 0.10's `optional` was added: an
  instance that never stands by writes the record it wrote before, and a
  checker built before 0.39 reads a marked entry as D000, an unknown
  member, which is the checker behind, not the record changed.
- **No D-check of a governed interface marked tokenless,** nor of a mark
  on an interface no profile governs. Each needs the contract's `uses`,
  the descriptor fixtures are checked against one contract, `nav.v2`,
  which uses no profile, and a checker reads `profiles` for its form only
  (cascade 8). The owner's start refuses the first (§8.2); a tool given
  the contract reports both (`redundancy.v1` §2.14, its
  `descriptors.json`).
- **O1** keeps its words: at most once only while one instance serves. A
  split still runs a call on each side, which `redundancy.v1` §2.10
  states, and a standby is reached by no call because it declares no
  queryable (O3), which is unchanged.
- **The split-brain check's reads and grace** (§6, 0.10) are unchanged.
  The profile's heal (its §2.9) takes them as its own rule.
- **§8.1's reading rules,** 0.37's and 0.38's, are unchanged and are not
  restated: the profile's election reads presence and follows it by them
  (its §2.5, §2.7, §2.8).
- **`hostid.v1` §2.12** is the profile's to amend, and is: its text 0.6
  counts no recognised standby, by `redundancy.v1` §2.15 (its CHANGELOG).
  The core's §6 names no profile's finding.

## 0.38 — 2026-10-24: the second implementation's findings against 0.37 (#849)

The spec-only implementation read 0.37 cold (zk2py round 54, PR #848):
2665 of 2665 fixture checks, and `scenarios/presence.md` §7–§9 run from
their text. It filed seven findings, F-185 to F-191. Each is answered
here, except F-187's desired.v1 half, which desired.v1's text 0.3 answers
(#840). No fixture case changed, and none is added: each rule is a
scenario, since each is a fact about a bus.

**Changed: a read cut off while it runs** (§8.1, §2.7; F-185).
`[Sc: presence.md §7]`
- **The gap.** 0.37's check came before the read only. A read asked while
  a transport was held, and in flight when the session's last transport
  closed, came back at the cut, complete and empty (measured 4 of 4 by the
  second implementation). The check before had passed it.
- **The rule.** A reader checks again when the read ends. A read whose
  session holds no transport then was made, so it is possibly incomplete,
  never a read not made. Its reason says the session was cut off during it.
  The tokens it returned were seen, and what it shows absent establishes
  nothing. §2.7 lists it among the incomplete readings.
- **The reference.** `zenkey::presence::liveliness_read` checks the
  session again after the read, and returns the read possibly incomplete,
  `cut_off_during` set, with the tokens it got.
- **What the check reads** (found building the reference). zenoh 1.10.1
  drops a closing transport's links, then answers the reads in flight,
  then removes the transport from `Session::info().transports()`. A check
  of `transports()` made as the read is answered still found the transport
  in 4 of 10 runs of §7 step 5; a check of `Session::info().links()` found
  none in 20. §8.1 now names the links, for the check before a read as well
  as after it.

**Changed: a read not made in a profile's premise** (§8.1; F-191). Every
profile names a presence read present, absent, or possibly incomplete or
refused. None names a read not made, so a tool had only "possibly
incomplete" to give one, which 0.37's CHANGELOG seemed to forbid. The
rule: a read not made is read as a profile's possibly-incomplete premise
reads it, unobservable. The profile's reason stands, and the tool reports
the read's own reason beside it. 0.37's "never folded into either" is
about the read's kind, which the tool keeps apart. It is not about a
profile's answer. No profile text changes. The reference's readers of
alarms, health, config and telemetry each read a read not made as their
possibly incomplete premise, and keep its reason in the report beside the
profile's: alarms' now in its scope answer's words too, and config's
beside the GET's when both failed, where it was dropped.

**Changed: "has never held"** (§8.1; F-190). 0.37 said a read goes on a
session that "holds no liveliness subscriber". One undeclared leaves its
session exposed, as the sentence before said, so the rule now reads "has
never held". This is how the reference marks a session: for its life.
`scenarios/presence.md` §9 says so too, and the reference's refusals name
a session that has held one.

**Changed: the scenarios observe the reader** (`scenarios/presence.md`
§4, §8, §9; F-186, F-188, F-189).
- **§4** expected a GET beside a liveliness subscriber to hold every token,
  which Appendix B measures failing (F-186). It is now the watch session's
  round trip: the handler rule, a GET that completes, and its answer
  discarded.
- **§8** observed zenoh only, so a reader that concluded absence from the
  withdrawal passed (F-188). Each Expected now asks the reader under test:
  - while the link is cut, the address unobservable and the re-read not
    made;
  - up on the heal;
  - down only from a complete re-read;
  - one read for a burst;
  - none for a re-mint.
- **§9** observed the test's own GETs (F-189). Now:
  - the reader under test makes no liveliness read on its watch session
    except the round trip;
  - a read asked there is refused;
  - a token withdrawn as soon as the seed is answered is heard going in
    every run.

**Changed while building the reference** (`scenarios/presence.md` §4, §9).
- **§4's population.** 10,000 tokens became "at least 1,200", past the 996
  from which spike S2 measured the hang. The handler rule needs no more,
  and at 10,000 the reference's own run, in a debug build, took 24 s to
  set up. Its GETs wait 5 s, stated, where the scenarios' 1 s would have
  applied. Its pairings with a bounded subscriber nobody drains are left
  to zenoh's measurements: the reference declares every liveliness
  subscriber with a callback, and those pairings wedge zenoh's threads in
  process.
- **§9 step 5 stalls the watch session's link** while a fresh reader
  starts. Without the stall, a reader that skips the round trip missed the
  withdrawal in 0 of 200 runs on one router in process: the race the step
  names is too narrow to catch it, so "in some runs" was a guess. With
  the link stalled, the declarations reach the router late, and such a
  reader misses in every run, while a reader whose round trip waits for
  the link hears the token go in every run. The Setup gains the link.
- **What the check reads**, §8.1 above.

**Unchanged.** desired.v1 §2.5's "token gone" (F-187) is desired's text
0.3. alarms.v1 §2.7 gets the same wording as a fix, and its text stays 0.8
(its CHANGELOG). The four profiles whose presence premises are complete
reads (alarms §2.14, health §2.11, config §5, desired §2.17) keep their
words, as do telemetry's and alarms' reading of a withdrawal as a gap.

## 0.37 — 2026-10-24: what a complete presence read is (#839, #830)

§8.1 said when a liveliness read is possibly incomplete (it ended at its
timeout, or drew an error reply) and that a refused read is complete and
empty. It said nothing of three reads that come back complete, empty and
wrong. `alarms.v1` found two of them, and left them to the core on purpose
(its CHANGELOG, 0.4's and 0.6's "Deliberately not stated"); #830 measured
the third. No fixture case changed, and none is added: each rule is a
scenario, since each is a fact about a bus.

**Changed: a read through a session cut off is not complete** (§8.1, §2.7,
Appendix B). `[Sc: presence.md §7]`
- **The gap.** A client that has lost its router has every read answered
  at once, the final reply alone, though nothing was asked of anyone:
  every owner read absent, and a GET in the same state healed a loss
  nobody answered. Only `alarms.v1`'s reference reader guarded against it
  (#800, AD).
- **The rule.** A read through a session that reaches no other zenoh node
  is not complete, whatever its replies say. A reader MUST NOT conclude
  absence from it, nor take it as healing a loss, and SHOULD NOT make it,
  saying why it did not; what it would have established is unobservable.
  §2.7 lists such a GET among the readings that are incomplete.
- **The reference.** `zenkey::presence::liveliness_read` makes no read
  through a session with no transport (`Error::CutOff`), and every fleet
  reader reports it not made: `zenkey_fleet::bus::presence::cut_off`,
  where `bus::alarms` had it, and a presence projection whose read was not
  made is unobservable (`PresenceBasis::CutOff`).

**Changed: absence is established only by a complete read** (§8.1).
`[Sc: presence.md §8]`
- **The gap.** A reader following tokens live hears every one withdrawn
  at once when its router restarts. `alarms.v1`'s reference book read that
  as absent at once; its notifier took absence from its next complete
  read instead; zenwatch's presence rules read it down. Which a live
  withdrawal is was for the core to say, for every profile's reader.
- **The rule.** A withdrawal heard live is a reason to read again, never
  absence by itself (MUST NOT). A reader that follows presence live
  SHOULD read again soon after a withdrawal that leaves something with
  nothing held, one read for a burst, and report what that read
  establishes; until then the withdrawn address is unobservable, a
  withdrawn token unconfirmed. A make-before-break re-mint owes no read.
- **What it costs.** A real departure is said one read later: the owner's
  lease runs out (or it undeclares), the token is withdrawn, the read
  shows none, the address is down. The reference reads 0.1 s after the
  first withdrawal of a burst, so a crash pages 0.1 s and one read's
  round trip later than it did, and a router's restart pages nobody.

**Changed: the read that establishes absence goes on a session with no
liveliness subscriber** (§8.1, Appendix B). `[Sc: presence.md §9]`
- **The gap** (#830). On zenoh 1.10.1, and on its main branch at
  `766b455`, a liveliness GET from a session holding a liveliness
  subscriber came back complete without live tokens the subscriber
  covered: 1,656 of 23,040 trials with history subscribers, 159 of
  16,560 without, 0 of 12,240 with the subscribers on another session,
  and every time (720 of 720) when the GET followed a history subscriber
  at once. Appendix B records the measurement, the mechanism (the router
  drops its answer for every token already declared to the session's face
  for a subscriber) and the draft upstream report, which is not filed.
- **The rule.** A reader SHOULD make every liveliness read it may conclude
  absence from on a session holding no liveliness subscriber of any kind,
  and hold its subscribers on a session of its own; and SHOULD make one
  round trip on that session after declaring them, before a seed on the
  other, since zenoh orders no two sessions' messages. Ordinary GETs are
  unaffected.
- **The reference.** Every liveliness subscriber of `zenkey` and
  `zenkey-fleet` is declared through `zenkey::presence::declare_subscriber`,
  which marks its session for its life, and a liveliness read on a marked
  session is refused (`Error::ReadBesideSubscriber`). `zenkey::presence::settle`
  is the round trip. zenctl, zenwatch and zengui open a watch session for
  their live readers.

**Deliberately not changed.**
- **The handler rule** (§8.1's first point, zenoh#2678) stands: a
  liveliness GET on a session holding a liveliness subscriber MUST use a
  callback or an unbounded handler. Since 0.37 the one such GET a reader
  makes is the round trip, whose answer it discards; its handler still
  must not hang it.
- **"Read twice" is not the rule.** A second read at once recovered most
  misses and left up to 10 of 720 trials wrong under load; a settle wait
  before the read did not remove the overtaking race. A session with no
  subscriber missed nothing in 22,320 trials per version.
- **A refused read** (0.8) stays complete and empty: access control
  answers it, and a tool still says what its reader could see. A read
  through a cut-off session differs: nothing was asked.
- **Possibly incomplete** reads (a timeout, an error reply) keep their
  meaning. A read not made is a third kind, never folded into either.
- **The three presence states** a tool reports stay up, down and
  unobservable: an address heard withdrawn is unobservable, as one behind
  a loss is, with its own reason.
- **No fixture.** Each rule is about what a bus answers, which a
  scenario observes and a static case cannot; `conformance/` is unchanged,
  and nothing was blessed.
- **Profiles keep their words.** `alarms.v1` §2.14 step 1's absent is
  already "a complete read without its instance token"; its text stays 0.8
  (its CHANGELOG points here). `telemetry.v1` reads a withdrawal as a gap
  in presence between two samples, which it is: no absence is concluded.
  `desired.v1` reads its author's absence so from its text 0.3 (F-187).

**Indexed after the merge** (#840, DC; zk2py round 53's F-183). Appendix
C's `desired.v1` row lists `scenarios.md` §10 beside §1, §3, §6 and §7:
S6's other branch, an owner silent within the GET's timeout across a cut,
then its own answer to the seed when its instance token appears again.
An index entry, not a rule: the version stays 0.37, and no rule, fixture
or scenario of the core changes with it.

## 0.36 — 2026-10-24: what `desired.v1` needed of the core (#840)

`desired.v1`'s first text (#840, DA) states the profile three adopters
had each defined on its own (`config.v1` §2.18, the walkthrough's
`mission_plan.v1`, ZenSight's `zs.desired.v1`), and publishes the interim
`desired` row of Appendix D. No fixture case changed; one is added.

**Changed: `desired.v1`'s vocabulary published, without `target`
(Appendix D, §10).**
- **The gap.** Appendix D's interim tables gave `desired` two keys,
  `target_param` and `target`, from architecture D19's list, and nothing
  said what either meant. `target_param` is what the walkthrough and
  ZenSight write; `target` no contract under `examples/zk2/` or `spec/`
  wrote, and R2's `self.system` and `self.service` (0.20) already say
  which identity fills each target parameter.
- **The rule.** `desired.v1` §4 publishes `target_param` alone. Appendix
  D's published table gains `desired`, and the interim row is gone. W105
  reads the published table as it reads any other (0.21), so a contract
  that writes `desired.target` draws W105, and `desired.target_param`
  draws none. `[F: contracts/w105-desired-target]`
- **What the value means** is the profile's (§10): its §2.2 reads it, one
  template parameter or two, the first bound to `self.system` and the
  second to `self.service`.
- **§10** names `desired.v1` among the profiles the core names, for
  store-and-forward (§4.4), its standard schema beside `telemetry.v1`'s
  (point 1), and its vocabulary beside the published ones (point 2).
  Appendix C lists the profile's scenarios that observe core rules: a late
  joiner's seed (§4.2), a window covering an outage (S3), a new instance's
  re-puts (§4.3), last-known (S5, S6).

**Changed: a pointer (§4.4).** "Store-and-forward (`desired.v1`) uses
both" cites `desired.v1` §2.9.

**Deliberately not changed.**
- **Instance identity (§1.5): no zid is reused.** Considered on review
  and not adopted: letting an owner restarting from its record keep its
  session's zid, so that `desired.v1`'s author could answer its targets' seeds under
  the stamps its record held, and a tool's S1 check (§4.2) would read them
  as its own. It was dropped on review: the instance id is a continuity
  epoch, a new session takes a fresh zid, and zenoh may refuse or confuse
  a zid reused within the old session's lease. `desired.v1` solves the
  restart itself: its author puts each document it keeps again from its
  new instance, as `alarms.v1` §2.9's owner does, and a target adopts,
  without an act, a version whose document equals the one it last acted
  on (`desired.v1` §2.7, §2.11). The core gains nothing for it.
- **The order of versions.** `desired.v1` §2.6 is §4.3's consumer rule:
  by time within one id, the first value under another id accepted. Two
  ids within one seed stay §4.3's residual risk, as 0.26 left them; the
  profile names the case and adds no rule of the core's.
- **S1–S7.** A document is state, and its version the owner's stamp
  (S1, S2): nothing new is needed to carry it. That a version with the
  same document is adopted without an act is the profile's rule for what a
  target does, not a change to what a re-put is (0.21: a mutation like any
  other). The author's tombstone window covering the outage to its
  targets is S3's rule, extended by the profile to targets, as `link.v1`
  §2.11 extends it to every state behind a face.
- **A late joiner's seed** (§4.2) is the target's reading, unchanged, and
  silence is still no verdict (O5): a target reverts only on a delete or
  a `reply_del`.
- **No core lint reads `target_param`'s value.** A declaration
  `desired.v1` §2.2 reads as invalid (a stream, a name that is no
  parameter, a parameter left out) still loads, as `freshness.ttl_s` and
  `telemetry.*` values do (0.21, 0.35). A linter SHOULD warn.
- **§8.1 and Appendix B** are untouched: a target watches its author's
  instance token, which the tokenless set never removes.

## 0.35 — 2026-10-21: what `telemetry.v1` needed of the core (#819)

`telemetry.v1`'s first text (#819, TA) states what a metric is, moves
ZenSight's `telemetry.v1.json` under `profiles/telemetry/` as the
profile's standard schema, and publishes the interim `telemetry` row of
Appendix D. No fixture case changed, and none is added: the profile's
fixtures are its own.

**Changed: a profile's first point is a standard contract or a standard
schema (§10).**
- **The gap.** §10 gave a profile four points, and none for a type that
  contracts share. `telemetry.v1.Point` is such a type: four ZenSight
  contracts list one file so that their Points are one type (core §7.1),
  and a tool reads a metric's kind from it. The file lived in
  `examples/zk2/zensight/`, an adopter's directory, which no rule tied to
  the profile.
- **The rule.** Point 1 is "a standard contract or a standard schema". A
  standard schema is a schema artifact that contracts list, as any file
  (§9.4). Its types are the profile's when the artifact's id (§7.1, §9.4:
  the hash of its JCS bytes) is one the profile publishes, so a contract
  listing a copy, byte for byte or JCS-equal, carries the profile's
  types, and a type of the same name in another artifact does not. The
  first is `telemetry.v1`'s (its §3).
- **Why by id, and not by name.** A type's identity is already its kind,
  its name and its artifact (§7.1), so nothing new is needed to tell the
  profile's Point from another of the same name: the profile publishes
  the ids, and a reader compares. Every revision's id stays the
  profile's, so a reader of an old bundle meets an old id and still reads
  a Point (`telemetry.v1` §2.3).

**Changed: `telemetry.v1`'s vocabulary published (Appendix D, §10).**
- **The rule.** `telemetry.v1` §4 publishes `kind`, `unit`, `buckets` and
  `semantic`, the interim row's four keys. Appendix D's published table
  gains `telemetry`, and the interim row is gone. W105 reads the published
  table as it reads the interim one (0.21), so no contract's codes change.
- **What the values mean** is the profile's (§10): its §2.2 reads each
  value, and its §2.4 says which types carry which kind.

**Changed: a pointer (§1.5).** A counter that crosses a face carries its
epoch in-band "as `telemetry.v1`'s counter does" (its §2.9), where 0.29
said "as a profile's counter type does", and the reset it reads is cited
to `telemetry.v1` §2.10 and §2.14 beside `link.v1` §2.13. §10's list of
the profiles the core names gains `telemetry.v1`, for a counter's epoch.

**Deliberately not changed.**
- **The four points stay four.** A standard schema widens point 1: like
  a standard contract, it is a file a contract names, and it adds no slot
  to the descriptor, no key and no kind.
- **No core lint reads an annotation's value.** `telemetry.unit = "°C"`
  is still E027 for its non-ASCII text (`[F: contracts/e027-ascii]`), and
  `telemetry.buckets = [0.5, 1e21, -0.0]` on a raw stream still loads
  (`[F: contracts/ok-float-integral]`), although `telemetry.v1` reads that
  declaration as invalid (its §2.2). A linter SHOULD warn, as for
  `freshness.ttl_s` (0.21).
- **W105 behaves as before.** It reads `telemetry`'s published keys, which
  are the interim row's, and warns of any other `telemetry.<key>`.
- **No profile-type reference form.** ZenSight's G12 proposed
  `"telemetry.v1:Point"`. A contract lists the standard schema as a file
  and names its types as any other (§7.1): `json:telemetry.v1.Point`.
- **No standard schema's history.** Unlike a standard contract, a
  standard schema has no `.history` of its own (§9.7): its ids are
  published by its profile's text, and the contracts that list it are
  published in their own histories.

## 0.34 — 2026-10-20: what `alarms.v1` needed of the core (#800)

`alarms.v1`'s first text (#800, AA) states the profile whose standard
contract ZenSight's mapping wrote, moves the contract under
`profiles/alarms/`, and decides what the interim `alarms` row of Appendix
D was for. No fixture case changed; one is added.

**Changed: `alarms.v1`'s vocabulary published, and empty (Appendix D,
§10).**
- **The gap.** Appendix D's interim tables gave `alarms` one key,
  `severity_default`, from architecture D19's list, and nothing said what
  it defaulted. Every alarm's document carries its `severity`: the
  published revision 1.0 requires it (`alarms.v1` §2.4), so a default has
  nothing to fill. No contract under `examples/zk2/` or `spec/` wrote the
  key.
- **The rule.** `alarms.v1` §4 publishes its vocabulary, with no key.
  Appendix D's published table gains `alarms` with none, and the interim
  row is gone. W105 reads an empty published table as it reads any other
  (0.21), so a contract that writes `alarms.<key>` draws W105.
  `[F: contracts/w105-alarms-vocabulary]`
- **Why publish it empty, rather than drop the row.** A profile that
  Appendix D does not list has no table, and none of its keys draws W105
  (§10). Dropped, the row would turn today's warning on
  `alarms.severity_default` into silence; published empty, it keeps
  warning of a key that means nothing.
- **§10** says a profile may publish an empty vocabulary, and names
  `alarms.v1`'s standard contract beside `health.v1`'s and `config.v1`'s.

**Changed: a pointer (§8.1).** The tokenless set names `alarms.v1` beside
`health.v1`, as interfaces every service of a framework set implements
(`alarms.v1` §2.13).

**Deliberately not changed.**
- **The key recipe is not an extension point.** It is fixed by the
  profile (`alarms.v1` §2.2), byte for byte ZenSight's, and is not a
  per-application parameter: a parameter would need a slot for it in the
  descriptor or the contract, which §10's four points do not give, and
  the one known implementer has one recipe.
- **S4, and a restart.** An owner that restarts cannot GET its own alarms
  back: no storage answers on its keys (S4), and its previous process,
  which answered them, is gone. `alarms.v1` §2.9 takes the previous
  instance's alarms from the two sources §4.3's catch-up already names,
  the owner's own record and an archive on its own side, so no core rule
  is needed.
- **Silence.** `alarms.v1` §2.15 establishes "no alarm fires" only from a
  complete GET that drew at least one member, or a complete, empty one
  with a lossless window longer than the horizon (§2.7's readings, with
  the profile's re-put): an empty reply set alone stays no verdict (O5).

## 0.33 — 2026-10-20: one kind of name per principal (#788)

The reference generator found it while finishing `link.v1` (LE, F-121),
and the maintainer decided it on #788. No fixture case changed, and none is
added.

**Changed: an input binding one principal by both a user name and a CN
(§11.1, the input).**
- **The gap.** §11.1 binds each principal "to a user name or a certificate
  CN", and said nothing of an input naming both. zenoh 1.10.1 matches one
  value of each property a subject names, so a subject naming a user name
  and a CN matches only a session presenting both (measured over mTLS,
  `link.v1` scenarios §11). Under `allow`, that principal's session
  presenting one of them matched no subject and was allowed everything: a
  principal enrolled with both was weaker than one enrolled with either.
  The reference wrote such a subject everywhere but on a router with a
  constrained face, where `link.v1` §2.4 (0.5) already refused it.
- **The rule.** A generator MUST refuse the input. The reference refuses
  the whole enrollment before any plan (`zenctl acl gen`, exit 2), naming
  every such principal: a plan printed around a refused principal would
  leave it unrestricted under `allow`.
- **Both implementations already do this.** zk2py's generator binds a
  principal by its user name alone and has no CN subject. The reference
  refuses since #788.

**Deliberately not changed.**
- **No subject per kind.** Writing a user-name subject and a CN subject
  into one policy, so either name gets the grants, would make one
  principal two credentials. Nothing asked for it, and §11.1 says "or".
- **`link.v1` §2.4.** Its refusal on a router with a face stands, now a
  case of this rule. Its text is unchanged.
- **The other refusals.** A principal bound by a zid, or one running a
  service the input does not declare, is still the generator's to refuse
  per principal (the reference: a refused row, exit 1).

## 0.32 — 2026-10-19: a principal's complement, per message and flow (#769)

The spec-only implementation (zk2py, round 35) read 0.31 cold and filed
F-123, an ambiguity in the bullet 0.31 added. No fixture case changed,
and none is added.

**Changed: the MUST of "A principal's own keys", per message and flow
(§11.2).**
- **The gap.** 0.31 required a generator to deny each principal "the keys
  of the services it does not run that its grants do not name". A grant
  names a key for some messages and flows only, the rows of the table of
  §11.2. Read per key, a key that a principal's Consume grant names, such
  as another service's stream, would need no deny of that principal's
  ingress `put` on it, and the principal could write a key it may only
  read. Only `security.md` §2 ("every unauthorized action is blocked")
  decided it, and `link.v1`'s scenarios assumed the reading per message
  and flow.
- **The rule.** The deny is taken on each message and flow, over the keys
  of the services the principal does not run that its grants do not name
  for that message and flow. zk2py's wording.
- **Both implementations already do this.**
  - **The reference** denies ingress `put`, `delete`, `declare_queryable`,
    `reply` and `liveliness_token` over every other service's keys,
    whatever the principal may read (`deny-write`). It denies the reads
    (`deny-read`) and the deliveries (`deny-receive`) over the surfaces of
    other services that its read grants do not name.
  - **zk2py** compiles every message and flow apart.

**Deliberately not changed.**
- **The MAY** of 0.31, a principal's own services' keys left out of its
  complement, stands as it was.
- **The grouping of messages.** A generator may write one deny for several
  messages that share a key set, as the reference does. The rule is what
  is denied, not how the rules are cut.

## 0.31 — 2026-10-19: whether a principal's own keys are in its complement (#769)

The spec-only implementation (zk2py, round 34) read `link.v1` 0.5 and
filed F-122 against its scenario §11, expected 7. The cause is here:
§11.2 compiled a grant under `allow` into "denies of its complement" over
the deployment's keys, and never said whether the keys of the services the
principal runs are in it. `link.v1` text 0.6 answers the scenario beside
this amendment. No fixture case changed, and none is added.

**Changed: a principal's own keys (§11.2, the complement's key set).**
- **The two readings.**
  - **The reference** takes a principal's complement over the other
    services' keys. A principal's own keys are written by it alone (P3),
    and its policy denies it none of them.
  - **zk2py** takes it per message and flow, over every key the principal
    holds no grant for, its own services' keys included. Own's rows name
    no `put` toward the holder, so `radio-a`'s policy denies `radio-a`
    the deliveries of its own stream.
- **Where they differ.** For the principal's own sessions, which the
  stricter reading keeps to its Own grant's rows, as `deny` does. And for
  a subject two principals' policies list, which holds both policies'
  rules: `link.v1`'s §11 step 7, a face subject in `ground`'s policy and
  `radio-a`'s, where X received §1's share under the reference and
  nothing under zk2py. No grant shape of §11.1 writes such a subject, and
  `link.v1` §2.15 reports one on a face.
- **The rule.** A generator MUST deny, in each principal's policy, the
  keys of the services it does not run that its grants do not name. It
  MAY leave the keys of the services it runs out of its complement, and a
  generator that denies them too conforms. The reference leaves them out.
  `[Sc: link.v1 scenarios §11]`

**Deliberately not changed.**
- **The `deny` posture.** Under `deny` a principal gets its grants' rows
  and nothing else, its own keys included; that is unchanged, and stays
  RECOMMENDED.
- **The reference.** Denying a principal its own keys per message and
  flow would lengthen every principal's policy, and protect nothing P3
  does not: no other principal writes them, and §11.1's grants for
  reading them are each consumer's own.
- **One reading for every generator.** The difference shows only for a
  subject that two principals' policies list, which §11.1's shapes never
  write. Requiring one reading would make a conforming generator wrong
  for a block no generator writes.
- **`security.md` §2** keeps its steps; its first expectation now names
  this exception beside the wildcard put.

## 0.30 — 2026-10-18: what a south region's declaration grant covers (#769)

Compiling `link.v1` into live routers (#769, LB: the reference generator,
`zenctl acl gen --face-policy`, and the spec-only implementation, zk2py
round 30) ran §8.5's south region under `deny` with grants narrower than
the owners' queryables, which the measurement of 0.14 had not. zk2py filed
F-117 against `link.v1` §2.5, which restates §8.5; the core's sentence had
the same gap. `link.v1` text 0.2 answers it beside this amendment. No
fixture case changed, and none is added.

**Changed: the declaration grant, spelled out (§8.5).**
- **The gap.** §8.5 required the far side's principal to be granted
  `declare_queryable` on egress "for what its side may query". Read as the
  far side's GET and call selectors, that grant does not send the owner's
  state queryables: they are declared over the interface's `state/**` and
  `@state/**` (S2, the table of §2.1), and zenoh checks a declaration by
  inclusion against the queryable's own key expression (§11.3). A selector
  over one resource does not include `state/**`.
- **Measured.** zk2py, on zenoh 1.10.1: granted on its bindings' selectors
  only, a far client's call was answered and its state GET was not;
  granted on the owners' `state/**` and `@state/**` as well, both were;
  with no grant, neither. The reference measured the same in `link.v1`'s
  scenario §2.
- **The rule.** The grant covers every queryable the far side may query,
  as its owner declares it: a state GET's state queryables (S2), a call's
  operation queryable on its concrete key or over its template (O1, so
  `@op/set/*` for a call of `set/duty`), an archive's `@state` forms
  (§4.4). The MUST is unchanged; what it covers is now stated.
  `[Sc: link.v1 scenarios §2]`

**Changed: the reference generator compiles face policies (§8.5).** The
`@stream` bullet said the reference denied `@stream` wholesale "at this
version"; it now compiles `link.v1`'s plan, the downsampling of an exposed
`@stream` included (`zenctl acl gen --face-policy`, #769 LB).

**Deliberately not changed.**
- **A grant, not a widening of the far side's reads.** A declaration
  carries a key expression and no value. What crosses in the replies is
  still checked against Consume and Call (§11.1, §11.2), so naming an
  owner's whole `state/**` in the declaration grant shows the far router
  that the queryable exists, and gives its sessions nothing to read.
- **No narrower owner queryables.** An owner could declare one queryable
  per resource, so that a selector-wide grant would include it; S2's one
  queryable over `state/**` stays, since the grant can name it and the
  runtime's GET over a whole interface needs it.
- **Clients unchanged.** A far side attached as a client sends every query
  to its router and needs no declaration (§8.5): no grant is added there.
- **`constrained.md` §6** keeps its words. Its state GET is answered with
  the grant this amendment spells out; the scenario named no grant.
- **Subscriptions.** The same inclusion admits a subscription or a GET
  only when a grant includes its key expression (§11.3, F-116). §11.3
  already says it; only `link.v1`'s scenarios had subscribed wider.

## 0.29 — 2026-10-18: what `link.v1` needed of the core (#769)

`link.v1`'s first text (#769, LA) publishes the face policy that §8.5
names, and the one annotation a contract keeps, `link.exposure`. The
maintainer decided on #769 that the downsampling rate is the face
policy's, so the interim `link.downsample_ms` of Appendix D goes. Writing
the text against §8.5 found two more places where the core said less, or
other, than the profile needs. No fixture case changed; one is added.

**Changed: `link.v1`'s vocabulary published (Appendix D, §10).**
- **The rule.** Appendix D's published table gains `link`, with the one
  key `exposure`, published by `link.v1` §4. The interim `link` row, which
  listed `exposure` and `downsample_ms`, is gone. W105 reads the published
  table as it reads `freshness.v1`'s (0.21), so a contract that writes
  `link.downsample_ms` now draws W105.
  `[F: contracts/w105-link-downsample]`
- **Why the key goes.** A rate is a fact of a face, not of a resource: two
  faces of one network carry one resource at their own rates, and a
  contract's author knows neither. The face policy states it,
  `interval_s` (`link.v1` §2.8). No contract under `examples/zk2/` wrote
  the key, and none of the core's fixtures did.
- **§10** names `link.v1`'s as the second published vocabulary, and links
  the profile where the core names it for face configuration.

**Changed: a crossing counter's epoch (§1.5).**
- **The gap.** §1.5 said that a counter crossing a face where neither the
  new session's timestamps nor presence are visible "carries its epoch in
  its value: a profile's counter type does", with no keyword. `link.v1`
  §2.13 makes it a SHOULD, as #769 decided (architecture D9c, zenoh-modem's
  gap 1), and allows the attachment, which crosses with the sample as the
  value does.
- **The rule.** Such a counter SHOULD carry its epoch in-band, in its value
  or its attachment. Without one, a reader across the face reads a decrease
  as a reset.

**Changed: a batch's example made exact (§8.5).**
- **The gap.** §8.5 said that at 2,400 bit/s with the 10 s lease "1 KB
  batches qualify" for its third of the lease. A third of 10 s is 3.33 s,
  which carries 1,000 B at 2,400 bit/s. The spike's batches were 1,024 B,
  3.41 s each: they held the link, and they are a little over the bound.
  `link.v1` §2.10 lints the bound in integers, so the example had to agree
  with the arithmetic.
- **The rule** is unchanged, a SHOULD. The example now says up to 1,000 B,
  and why the spike's 1,024 B do not contradict it.

**Changed: pointers.** §8.5's face configuration and its `@stream`
exception, S3's maximum outage, and §12's Faces row point to `link.v1`.
Appendix C lists its scenarios beside the core's.

**Deliberately not changed.**
- **S3 stays about archived state.** `link.v1` §2.11 floors the window of
  every state behind a face, archived or not, since a far late joiner
  seeds by a GET after each outage (§4.2). That binds a deployment that
  uses the profile. The core keeps its MUST where the archive's evidence
  put it, and points to the profile.
- **R7 and §8.5's MUSTs.** `link.v1` restates what crosses and where its
  denies go; the core's rules, the attachment shapes, the `@zk` deny, the
  third of the lease and the south region's `declare_queryable`, are
  unchanged. The batch bound stays a SHOULD, not architecture r4's "MUST
  cross well within", as #769 decided.
- **No face subject, no `@op` deny in the core.** `link.v1` §2.4 puts the
  face's denies in every policy that can match a session on the face, and
  §2.9 lets requests cross by grant. Both follow from §11 as it stands
  (any allow wins across subjects, F-87; Call and Consume), so neither
  needs a core rule.
- **The reference generator** still denies `@stream` wholesale, and §8.5
  says so: compiling a face policy is #769's next chunk.
- **No core fixture for exposure.** What `link.exposure` means is the
  profile's (`conformance/exposure.json` there); the core checks only
  that its vocabulary is published (W105).

## 0.28 — 2026-10-18: what `config.v1` needed of the core (#768)

`config.v1`'s first text (#768, CA) found four places where the core said
less than the profile needed, and settled each inside the profile, waiting
for the core. Its runtime (CB) met the same four. The spec-only
implementation's cold read (zk2py round 28) filed F-107 to F-113 against
the profile alone, which `config.v1` 0.2 answers. This amendment decides
the four. One is a definition the core lacked, one lets an owner say what
it already had to say, one states the means of a MUST the core already
had, and one keeps the rule as it was, with its reason. No fixture case
changed (three are added, below), and no reading of an earlier version is
overturned.

**Changed: the error codes defined (§5.2).**
- **The gap.** §5.2 listed eight codes and defined some of them only in
  passing, elsewhere: `not_found` beside a key that names no member,
  `internal` beside a handler that ends without replying. `forbidden` was
  defined nowhere. `config.v1` §2.13 refuses with it, and CA had to guess
  what it meant: "an act refused by rule rather than by the request's form
  or the device".
- **The rule.** A list after the table says what each code means to a
  caller that reads no message. `forbidden` is a rule of the owner's own
  that goes on refusing the act while what it rests on holds. It is never
  access control's, which is silent (O5), and never a configuration or a
  capability that leaves the act out, which is `unavailable`.
- **Why that line.** A caller decides what to do from the code alone.
  `unavailable` says another deployment, or a later configuration, could
  serve it. `busy` says the same call could succeed later. `forbidden` says
  neither will help while the rule's premise holds: for `config.v1`, a new
  last change that set no sensitive value.

**Changed: `unavailable` from a handler, for one member (§5.1).**
- **The gap.** O3 gave `unavailable` and its cause to an optional operation
  an instance does not expose, answered before any handler runs.
  `config.v1`'s startup allowlist refuses `set/{group}` group by group:
  the operation is exposed, and one member of its template is left out
  by the owner's startup configuration. CA answered it `unavailable`,
  cause `config`, from the handler, which the core neither allowed nor
  forbade.
- **The rule.** A handler MAY refuse a call to one member of an exposed
  operation with `unavailable` and its cause, `config` or `capability`,
  after the refusals the core makes before any handler and the request's
  decode. The descriptor does not list it, since §3.3's `unavailable` names
  resources, not members. A profile that refuses so says it in its own
  state, as `config.v1`'s `writable` does, so a tool sees it before it
  calls.
- **Why not another code.** `forbidden` would tell the caller that nothing
  will help, while a startup configuration that names the group does.
  `not_found` would deny a member the view lists.

**Changed: the rate across a restart (§2.7).**
- **The gap.** 0.24 made a rate per member, and said that a re-mint does
  not reset it. A restarted owner is a re-mint (§1.5), and it can count its
  earlier instances' occurrences only from a record it keeps: nothing on
  the bus gives them back, since a union storage is optional (§2.6).
- **The rule.** The MUST stands, and its means is stated: a restarted
  owner counts its earlier instances' occurrences within the period from a
  record of their instants that it keeps across the restart. An owner that
  keeps none cannot keep the rate across a restart, and a tool that
  measures from keys reports the break like any other.
- **`config.v1`** (text 0.2) states it for its change event, and the
  reference carries the record through its application
  (`zenkey::config_v1::Startup::events`).

**Deliberately not changed.**
- **The rate is not relaxed to "since the instance started".** The rate
  bounds the keys a storage holds, cardinality × rate × retention (§2.2).
  An owner in a crash loop that reset its count at each start would
  multiply that bound by its restarts, which is the case the bound exists
  for. 0.24 already rejected a rate per instance as unmeasurable, since a
  key names no instance.
- **A detail stays on `app` alone (§5.2).** `config.v1` wants `busy` to name
  the pending change's token, and CA put the token in the message, for a
  person, and in the view and the status, for a program. The core keeps it
  so:
  - a tool decodes an envelope without the contract, and refuses a detail
    on any code but `app` (§5.2, "Decoding"), so that a detail always has
    a declared type to read it by;
  - a detail per code would need a type per code in every contract that
    declares an `error` type, while the owner's state is already typed and
    already GET-able.

  §5.2 now says this, beside the codes.
- **O3's own wording.** The table's O3 still speaks of an operation not
  exposed. Refusing one member from a handler is an owner's option beside
  it, stated under "Answering", not a new duty of every owner.
- **The decoder.** A tool's decoding of an envelope is unchanged. No case
  of `errors/cases.json` changed; three are added, since no case decoded
  `forbidden` at all: `json-forbidden` decodes, `refuse-cause-on-forbidden`
  is refused (`cause`), and `refuse-detail-on-busy`, `config.v1`'s token
  sent as a detail, is refused (`detail`). The reference passed all three
  as written. 45 cases.

## 0.27 — 2026-10-11: a reader that stalls, a tool as a role, and the merge's open cases (#753, #748, #752)

Three issues, one of them a premise a measurement falsified.

- **#753.** 0.25 rested "lost no delivery" on a stalled reader seeing the
  owner's token withdrawn. `zenkey/tests/budget.rs` §1, unchanged, failed
  that premise about one run in four: six runs on 2026-10-11 lost 661–792
  reliable, `block` samples, and two of them saw no withdrawal.
- **#748.** §11.1 called a tool "a principal with no address of its
  own", while the reference enrolls principals that run services and a
  tool. Its input named "the services a tool inspects", which the
  reference enrollment had no way to say.
- **#752.** The spec-only implementation read 0.26 cold (PR #751) and
  found three cases the late joiner's merge left open (F-106): an
  unstamped sample, equal times under two ids, and R6 for a tool's seed.

Reference behaviour is the rule, unless it is a bug. One was: the
generator refused a wildcard descriptor read under `deny` (below).

**Changed: rule 1 of "lost no delivery" narrowed (§2.7, #753).**
- **The mechanism, measured.** The probe recorded what 0.25's test did not:
  whether the token had reached the reader when it stalled, its own
  session's transport events, and a ticker on a thread of its own.
  - In 27 runs (a probe's 18, and 9 of the test as it now reports), the
    reader's own transport closed every time.
  - It saw the owner's token withdrawn in each of the 23 runs where the
    token had reached it before it stalled, and never in the 4 where it
    had not. A transport that closes withdraws only the tokens the reader
    received through it. The reader then read everything its router had
    sent before the close, and the token was not among it: it went with
    the router's queue toward the reader. The reader saw the token only
    after it reconnected, as a new token.
  - A ticker on a thread of the reader's process saw no gap above 107 ms:
    only zenoh's receive thread was held.
  - A reader whose whole process was stopped (SIGSTOP) for 7 s instead,
    in 13 runs: its ticker saw a 7.0–7.1 s gap on its first tick after,
    its session read its transport's close (0.35–0.51 s later in the 4
    runs that timed it), and it saw the token withdrawn.
  - A run with more than one withdrawal had another transport close
    beside the reader's: the owner's own push had waited
    `wait_before_close` too, and its session closed its transport to its
    router.
- **The rule.** "Its reader lost none" also requires, over the window and
  the watch after it:
  - **no transport of its session closed**. A router that gives up on the
    reader closes the reader's own transport, and what it held for the
    reader goes with it;
  - **its process did not stall longer than the bound**: the transport's
    `wait_before_close`, or its lease less one keep-alive interval when
    that is shorter (5 s and 7.5 s with zenoh's defaults). A shorter stall
    is never given up on: the router waits on a full queue for
    `wait_before_close`, and hears from the reader within its lease.
- **How a tool establishes it.** It watches both, from before the window
  until its watch of the owner ends:
  - its transports, from zenoh's transport events. These saw every
    measured stall, a stuck callback's and a stopped process's alike;
  - a monotonic ticker on a thread of its own. It sees a stopped process
    first, and does not see a blocked receive thread.

  #753 asked for a ticker, and for transport events if a measurement
  supported them. The measurement supports both, each for what the other
  cannot see, so both are the rule.
- **A tool without transport events.** They are unstable in zenoh 1.10.1
  (§0). Such a tool says so, and rests that half on a premise of its own:
  nothing in its process blocks zenoh's receive threads. That premise is
  the tool's to know, as routers in processes of their own are the
  deployment's.
  - Rejected: reading no window lossless without them. A tool's own
    callbacks are its own code, and the core already leaves one such
    premise to the side that can know it.
- **The reason** is `reader_stalled`, chosen second: after `reader_lost`,
  the reader's counted loss, and before every reason about the path.
  `lossless.json` states the order.

**Changed: Appendix B corrected.**
- "Each time the reader saw the owner's instance token withdrawn" is
  replaced by the measured rate. It adds the transport events, the case
  where only a token received is withdrawn, and the stopped process.
- It spells out the timestamp order (below).

**Changed: §11.1, the grant shapes per principal (#748).**
- **"Tool" is a role a principal plays.** A principal holds the union of
  the grants of what it runs. One that runs services may run a tool too,
  and holds the tool's grants beside its Own, the admin read included.
  This is safe because no shape writes beyond its writer's own prefix:
  - Own stays scoped to the principal's services;
  - Consume, Call, presence and the admin read write nothing;
  - the `@/**` queryable deny stays in every policy.

  The reference already compiled it: `ground`, and since FL3 `zenwatch`.
- **Inspection without a binding**, option (a) of #748. A doctor reads
  `zk2/*/*/@zk/**`, and access control works by inclusion, so a principal
  granted presence only through its bindings is refused the whole read,
  complete and empty.
  - A tool's `inspects` is presence alone: the `…/@zk/**` subtree of each
    service named, exact or wildcard, descriptors included, and no
    Consume.
  - The reference enrollment's `[[tool]]` takes
    `inspects = ["<system>/<service>", …]`.
  - The walkthrough's `ops` inspects `*/*`, for its doctor and
    `zenctl service list`.
  - Rejected: (b), presence through bindings and calls only. It would
    leave a doctor without a grant short of binding every interface it
    reads.

**Changed: a generator bug fixed (§11.2, #748).**
- **The bug.** Judging inspection on a live router found it: a tool's
  wildcard descriptor read (`zk2/*/tc/@zk/instance/*`) got nothing under
  `deny`, while its liveliness read answered.
  - A liveliness read is answered by the routers. The descriptor's GET and
    subscription, which have ridden the presence grant since 0.14, travel
    to the provider.
  - The generator left presence selectors out of the provider's egress
    grant ("presence reads never travel toward the provider"), which was
    true before 0.14.
- **The fix.** A presence selector that intersects a provider's `@zk/**`,
  and that its Own does not include, joins its egress grant as
  `fan-in-presence:<address>`.
  - It does not join the provider's ingress `reply`. No refusal answers a
    descriptor read, and a reply grant on the selector would let the
    provider answer under another provider's descriptor key.
  - §11.2 says so. Every pinned plan with a wildcard binding or inspection
    gains the rule.

**Changed: the merge's open cases (§4.2, #752).**
- **The order, spelled out.** Stamps compare as `Latest` compares replies:
  by time, then by id. The id is compared as zenoh compares it, its
  128-bit value written little-endian and the bytes compared from the
  first. So the least significant byte that differs decides: at one time,
  id `100` (hex) is below id `2`.
  - The spec-only implementation ordered by the id as an integer, which
    disagrees with zenoh's order whenever the low bytes and the high
    bytes disagree. It follows the spelled-out order.
  - The reference's order is zenoh's `Timestamp`; a fleet test checks the
    model's against it for every pair of a set of ids.
- **An unstamped sample** ranks below every stamped one, as `Latest` ranks
  an unstamped reply lowest. It replaces only another unstamped sample:
  two have no order but their arrival, and the later wins, a reply arriving
  after a live sample included.
  - That was the reference's merge already, and zk2py's guess.
- **R6 and a tool's seed.** R6 binds a consumer, whose seed discards a
  key that is not concrete on either path. A tool's seed keeps it, under
  the key it arrived on, never merged with the keys it covers.
  - That is the reference's behaviour, and zk2py's. 0.26 found it, and
    left it out of the text.

**Changed: evidence.**
- **`conformance/budget/lossless.json`.** Each case gains `reader_stall`
  (`gap_ms`, `bound_ms`, `transport_closed`), and seven cases pin
  `reader_stalled`, its order and the bound's edge (a gap of exactly the
  bound is no stall). A new `stall_bounds` array pins the bound from
  `wait_before_close`, the lease and the keep-alive count. No expected
  value of an existing case changes.
- **New `conformance/seed/merge.json`.** It holds pairs of stamps with
  their order, and one key's arrivals with which are kept.
  - 0.26 had no fixture, since the order was only zenoh's. Now that the
    order is spelled out and the unstamped rule added, a pure function
    (`zenkey_model::seed`) decides it, and the fleet's merge calls it.
- **`scenarios/budget.md` §1.**
  - Step 3's expectation is corrected.
  - Step 4 stops the reader's process.
  - "A tool's window over a stalled reader": a run whose own transport
    closes mid-window reads its population and rate unobservable.
- **`zenkey/tests/budget.rs`.** It reports the withdrawals, the transport
  closes and the ticker's gap. It asserts only the loss in step 3, and the
  loss and the ticker's gap in step 4.
- **New `scenarios/state.md` §11**: unstamped samples, equal times, and a
  key that is not concrete, in `zenkey-fleet/tests/seeding.rs` (`s11_…`).
- **`scenarios/security.md`.** §1 adds inspection and one principal with
  both roles. §2's generator check adds the presence fan-in.
- **Appendix C and E** name the new files and inputs.

**The reference.**
- **Stall detection.** `zenkey-fleet`'s `check conform` runs `bus::stall`:
  the session's transport events and a ticker, over a window's watch.
  `zenctl check conform` takes `--wait-before-close` (5 s).
  `zenctl/tests/live_zk2.rs` closes the tool's own transport mid-window
  and reads `budget-window` unobservable.
- **The enrollment.** `zenkey_fleet::model::acl` compiles `inspects`.
  `acl_live.rs` judges an inspecting principal on a router under `deny`,
  and the presence fan-in by removing it.

**Found while writing it.**
- **§8.1 does not forbid a tool's callback from blocking.** It asks a
  liveliness subscriber beside a GET to be callback-driven or drained,
  which is another rule. The premise of a tool without transport events
  is stated in §2.7 instead.
- **The bound uses the reader's lease and keep-alive**, the ones the
  router expires it by. A tool takes the deployment's `--lease` for both
  the watch and the bound, and zenoh's default keep-alive.
- **Two tokens' worth of noise.** A presence selector included in a wider
  one of the same provider's egress grant is listed beside it
  (`zk2/vehicle-01/*/@zk/**` beside `zk2/*/*/@zk/**`). That is harmless,
  as it already was for data fan-in.

**Deliberately not changed.**
- **The presence premise.** A link that fails still withdraws the
  owner's token at a reader that has it, and the owner present until one
  lease after the window still reads it. A stalled reader is the case it
  could not cover, and rule 1 now does.
- **Routers in processes of their own** stay a deployment premise. A
  reader sharing its process with routers lost deliveries with no
  transport closed, which neither signal sees.
- **No new state rule.** S1–S7, and the order of the subscription and
  the GET, stand. A foreign key is still outside S4.
- **The constrained face.** A far principal's presence never crosses
  (§8.5), so `inspects` adds nothing there. Its plans gain only the
  cite text and the vehicle's providers' presence fan-in, which the face's
  `@zk` deny still stops.

## 0.26 — 2026-10-10: a late joiner's seed (#614)

zengui's tree seeds a watched subtree with what is already there, then
follows it (#614, FL4). Its seed was v1's (RFC 04 §3.2): subscribe, then a
GET answered by router storages, merged by stamp. zk2 forbids half of
that: S4 addresses a state GET to the owner with `All` + `Latest`, and
forbids a storage on an owner's state. §4.2 stated S4, and nothing about a
reader that wants state and then its changes: the order of the
subscription and the GET, how the two merge, and what a seed cannot know.
The maintainer decided the rule (decision 7 of the plan for #614's GUI
half), and this amendment writes it.

**Changed: a rule stated (§4.2, "A late joiner's seed").**
- **The order.** A late joiner MUST subscribe first, then make S4's GET.
  In the other order, a mutation the owner makes after it answers and
  before the subscription reaches it is lost, and a lost delete leaves a
  deleted key standing. v1 wrote the same order (RFC 04 §3.2); zk2 had
  not.
- **The merge.** Per key, by the owner's stamp, whichever path brought
  each one: a newer live put or delete replaces a reply, and a newer reply
  replaces a live sample. A `reply_del` is a deletion with its stamp. Two
  samples with equal stamps are one mutation, kept once. Stamps compare as
  `Latest` compares replies (§4.1), so within one timestamp id by time
  (§4.3).
- **A selector over several owners.** One GET on a watched selector is
  addressed to the owner's keys when each key it matches has one owner
  (P3): `Latest` then consolidates each key over that owner's answers.
  Foreign keys are outside S4, and a storage may answer them.
  - Rejected: one GET per owner's resolved state key. It reads S4 more
    literally, but it needs the contracts before the first GET, and it
    cannot seed a foreign key.
- **What a seed cannot know.** A GET that ran to its timeout seeds
  possibly incompletely: §8.1's reading of a liveliness GET, applied to a
  state GET. An owner absent at the GET seeds nothing, which is O5's
  silence, never "no state". A seed is current state, and last-known state
  is still an archive's.

**Changed: evidence.**
- **New `scenarios/state.md` §10.** A late joiner subscribes, then GETs.
  The owner holds the GET: it puts a key its answer will hold, makes its
  answer, puts a newer value of a key the answer holds and a key the answer
  lacks, then answers. Expected: the stale reply loses to the newer put, the put the
  answer lacks is kept, the `reply_del` reaches the merge as a deletion, a
  put both paths carry is kept once, and the seed is complete. A control
  in the other order loses the put. The reference runs it in
  `zenkey-fleet/tests/seeding.rs` (`s10_a_late_joiners_seed`), through the
  seeded subscription zengui's tree uses.
- **No fixture.** No pure function decides the merge apart from Zenoh's
  own timestamp order, which a fixture would only restate. The scenario is
  the evidence, as it is for S1–S4.
- **Appendix C**'s `scenarios/state.md` row names §10.

**Changed: citations (editorial).** #614's FL3 made zenwatch a zk2
service, and `zenwatch/src/publish.rs` on `main` is now its zk2
self-publication. Three texts cited the v1 file by path and line. Each now
names the `v1` branch, and the zk2 home where the sentence is about one:
- `hostid.v1` Appendix B, zenwatch's v1 salt (text 0.5);
- `health.v1` Appendix B, zenwatch's health document, and `zenwatch.v1`
  (`zenwatch/contracts/zenwatch.v1.toml`) as the interface of its own the
  row foresaw (text 0.4);
- `examples/zk2/zensight/README.md`'s consumer table, whose other zenwatch
  citations (`rules.rs`, `engine.rs`, `discipline/inhibit.rs`), rewritten
  or removed by FL2, now name the `v1` branch too.

Each profile's `CHANGELOG.md` has its entry.

**Found while writing it.**
- **Two ids within one seed.** §4.3 orders an owner's values by time
  within one timestamp id, and has a consumer accept the first value under
  a new id. A seed that sees two ids for one key, an owner re-minting
  inside the seed's window, cannot tell from the stamps which is newer.
  Comparing as `Latest` does is what Zenoh's consolidation does at the GET
  anyway. It is named as §4.3's residual risk, and not resolved.
- **`Latest` delivers at completion** (§4.1): the replies of S4's GET
  arrive together when the GET ends. The live samples of the window are
  merged first nearly always, and a reply loses to every newer one: the
  case the merge exists for.
- **R6 and a tool's seed.** R6 has a consumer discard a reply on a key
  that is not concrete. A tool's seeded view of the raw bus shows what
  arrived, as its live subscription does, and the reference does not
  discard there. The rule binds a consumer; whether a tool counts such a
  reply apart is the tool's.

**Deliberately not changed.**
- **S4, S5, S6.** S4's GET is the seed's GET, and a storage on an owner's
  state is still forbidden. Last-known state is still read from an
  archive, explicitly, and never presented as current: a seed never turns
  to an archive.
- **The archive's role (§4.4).** An archive aligns from the owner's
  collection to record and serve last-known state. That is not a late
  joiner's seed, and its rules are unchanged.
- **§2.5's history.** A history query reaches the caches of the advanced
  publishers present, streams included. It is not S4's GET, and this
  amendment does not make it one. The reference tool keeps it beside the
  state GET, for resources that declare `history`, merged the same way.
- **Events.** An occurrence is replayed from union storages, bounded by
  its retention (§2.6). A seed of state does not reach it.
- **`freshness.v1`'s "zk2 has no seed: a late joiner GETs the owner (core
  S4)"** (its appendix, on seeding by re-publishing). That is this
  amendment's GET, and v1's seed attribute is still not ported. No text
  version moves.
- **S1–S7 keep their numbers.** The rule is a paragraph beside S4–S6, as
  0.21's re-put is.

## 0.25 — 2026-10-10: "lost no delivery" established, and the owner keeping its budget (#740, #741)

The spec-only implementation read 0.24 cold (PR #739) and found one
ambiguity, F-105. §2.7 lets a window be a complete reading of a stream's
or an event's population, and gives a rate its clean pole, only when the
window "lost no delivery". A subscriber can know that it lost a delivery,
never that it lost none: a best-effort or `drop` sample a queue dropped
leaves no trace. `freshness.v1` §2.5 states the condition as knowledge,
and §2.7 as fact. Read as a fact, no window is ever complete. Read as
knowledge, a window over a path that drops silently reads clean over a
population above its bound, since a member whose samples were all dropped
reads as no member.

The maintainer decided the resolution: "lost no delivery" is established
from what the reader can see. Its premises were measured before they were
written (`scenarios/budget.md` §1–§3, Appendix B), and the measurements
narrowed it in three places.

**Changed: rules stated (§2.7).**
- **Lost no delivery, established.** Both must hold:
  - **the reader lost none:** no overflow or lag in its own queue, and
    nothing kept out of its count by a cap;
  - **the path could not drop one silently:** the resource declares
    `reliable` with congestion `block`.

  Otherwise no window of the resource is lossless, whatever it heard. The
  reason names the reader's loss, a detected miss, or the QoS.
  - **Why `reliable` and `block`.** zenoh 1.10.1 counts such a sample
    non-droppable. A full queue makes its push wait, and one it cannot
    queue within `wait_before_close` (5 s by default) closes the
    transport. Measured through two routers in processes of their own: a
    reader taking 2 ms per 16 KiB sample heard 1,000 of 1,000 in order.
    The same reader stuck for 7 s lost 691–794, and saw the owner's
    instance token withdrawn each time. Best effort with `drop` lost
    695–832, and nothing it received said so.
  - **Why a failure is seen.** A transport that closes withdraws the
    tokens learned through it, and the presence premise reads the
    owner's. Measured: at once when a link closes (0.8–2.0 ms), between
    routers and on the reader's own link alike.
- **The owner present until one lease after the window's end (narrowed).**
  The decided resolution rested on "owner present throughout" catching a
  failed path. A link that goes silent is seen only when its lease runs
  out: 986–1,037 ms at a 1 s lease (and 9,961–10,047 ms at the default
  10 s, spike S10). The samples put meanwhile are lost. A failure in the
  window's last lease would be seen after the window ended, so the tool
  watches for a lease more, plus its reply timeout for the withdrawal's
  transit.
- **Miss detection is evidence of a loss, never of none (narrowed).** The
  decided resolution let a subscription that detects misses, and detected
  none, establish the window. zenoh-ext cannot carry that.
  - It takes a publisher's first sample heard as its start
    (`advanced_subscriber.rs`, `handle_sample`). So it reports neither the
    samples before that one nor a publisher none of whose samples
    arrived. Measured: five samples never heard, then none reported.
  - The reference writer gives each member its own publisher, so a member
    whose every sample was dropped is exactly what it cannot see, and
    exactly what a population reading must rule out.
  - Its periodic heartbeat, which would show a trailing loss, is
    published with `drop`.

  What it can show is kept: a detected miss is a loss. `lossless.json`
  keeps the inputs (declared, used, misses), so that a later amendment
  with a measured rule for publisher detection has a format to extend.
  Events, the only resources with a rate, cannot declare `history` (E019),
  so the rate never had this branch.
- **A window counts from its subscription reaching the owner.** 0.24's
  "a publisher matched late" was named as a cause of incompleteness and
  ruled out by no premise. A sample put before the owner's router knew of
  the subscription is never routed to it. No reader can see that moment,
  so a tool counts the span from its declaration plus its reply timeout:
  a convention, stated as one.
- **A rate's clean pole reads the same rule,** the owner's presence
  included. 0.24 asked a rate window to have lost nothing, and nothing of
  the owner. Losslessness now rests on presence catching a failed path,
  and a rate window over a failure is no more lossless than a population
  window.
- **`history.miss_detection_ms`** (§2.5) said nothing of what it
  declares. It declares that the owner numbers its samples and heartbeats
  every that many milliseconds, which is what the reference writer does.
- **Keeping the bound** (§2.7). An owner keeps it by refusing the write
  that would make a new member live while its live members are at the
  bound: a put of a new state key, a publication on a stream member not
  live, an occurrence of an event member not live. A put to a live key
  adds no member, a delete frees one, and a refused write publishes
  nothing and is never silent. An occurrence beyond the rate is refused
  the same way. 0.24 left this to runtimes; the spec-only owner refused
  already, and the reference runtime now does (#741).

**Changed: wording.**
- **§2.4** says that only a `reliable`, `block` resource has windows a
  tool can read clean from.
- **§2.7's table**: clean and unobservable cells name the established
  rule and the owner's presence for the rate.

**Changed: evidence.**
- **New `conformance/budget/lossless.json`:** the declared QoS, the
  reader's loss and miss detection → lossless or not, with a reason in a
  stated order: `reader_lost`, `misses_detected`, `best_effort`,
  `congestion_drop`, else `reliable_block`.
- **`rate.json`:** its reading's `complete` is renamed `lossless`, the
  same name and meaning as `population.json`'s. Each case gains
  `present`, and five cases pin `owner_absent`, ordered after
  `window_too_short` and before `empty`, as `population.json` orders it.
  No expected value of an existing case changes.
- **`population.json`:** its description says what `listened_s`,
  `lossless` and `present` now mean. No case changes.
- **New `scenarios/budget.md`:** §1–§3 the measurements, §4–§7 an owner
  keeping its bound and its rate. Appendix B lists the measured facts,
  and Appendix C and E index the new files.

**Found while writing it.**
- **A router that shares its process.** The sessions of one process share
  zenoh's receive threads, two by default. With two routers and a reader
  whose callback blocks in one process, 3 of 6 runs lost 674–970 of 1,000
  reliable, `block` samples. The transport closes stalled, and no token
  was withdrawn while the runs lasted. §2.7 names routers in processes of
  their own as a premise a deployment provides, and no tool can see it.
- **A UDP link.** zenoh sends a reliable sample over any link when no
  reliable link is up, so over a UDP link alone it travels best-effort
  (source). That is the other premise named.
- **A rate window across a re-mint** is now unobservable, although a
  re-mint does not reset a rate (0.24). Kept: the old instance's token
  goes, and nothing tells a re-mint's make-before-break from a failure
  that took it.

**Deliberately not changed.**
- **`freshness.v1` §2.5.** Its rule 3, "it knows it lost deliveries", is
  the reader's-loss half of §2.7's rule, and it is about another verdict.
  Fresh rests on a delivery heard, never on none lost. Stale is reported
  "as this reader observed" (§2.11), and §2.4's half interval absorbs one
  lost put. No text version moves.
- **Interceptors.** A router configured to downsample a key drops its
  samples by design, and like an access-control deny it narrows what any
  reader there can see (§8.1's narrowing, 0.8).
- **No lint.** A templated stream declared best-effort is its author's
  choice; §2.7 says only what a tool can then conclude.
- **Best effort is not made complete by length.** A longer window of a
  best-effort stream drops as silently as a short one, so its population
  and its rate stay unobservable from a window. Its finding, more members
  than the bound, stays available from any window.

## 0.24 — 2026-10-10: the population budget (#735)

FJ (#612) left the tools' budget checks dark as profile-backed, beside
kinds, alerts and configuration. The maintainer decided on 2026-10-09 that
the budget is core: `cardinality` bounds a template's population in one
instance (§2.2), and an event's `rate` and `retention` are core fields
(§2.6). Lighting it up showed that the core named the bound and never said
what it counts, when a member is live, what `rate` is a rate of, or what a
tool may conclude from a reading.

**Changed: rules stated (new §2.7).**
- **The bound.** The descriptor's lowered `cardinality` when it states one
  from 1 to the contract's, else the contract's. A value outside that
  range is D007's and does not apply. 4294967295 is no ceiling from either
  source, as §2.2 already said of the contract's.
  - **Across instances, the greatest.** A key names no instance, so a
    resource's members are the service's, and a re-mint's overlap shows
    two instances holding one population. A finding then never rests on a
    bound one of them did not state.
- **Members, and when one is live.** A member is one value of the
  template's parameters: a key, or an event's key without its ULID. An
  owner MUST NOT hold more live members of a templated stream, state or
  event than its bound. Live is:
  - **state:** a key with a value; a deleted key is not live;
  - **stream:** published within the last hour. A stream has no delete,
    so a reader cannot see a member stop. One that forgets a member after
    an hour of silence holds at most the bound. "Since the instance
    started" was rejected: an owner whose entities churn would exceed any
    bound in a long run, and could keep it only by minting instances it
    does not need;
  - **event:** an occurrence within the retention, as §2.2's "cardinality
    × rate × retention" already implied.
- **`rate` is per member,** within any period of its own: at most one
  occurrence of a member in any hour (`rare`), any minute (`low`), or
  `<n>` in any hour (`burst(<n>/h)`). `<n>` + 1 occurrences of one member
  less than the period apart break it.
  - §2.2's key population, cardinality × rate × retention, holds only per
    member: per template it would be rate × retention.
  - An owner keeps it by counting its own occurrences per member, and a
    tool measures it from keys alone. Per instance it could not be
    measured, since a key names no instance, and a re-mint does not reset
    it.
- **What a tool reads.** A state resource's S4 GET that ran to its final
  reply is complete. So is a stream's or an event's subscription window
  that lasted at least one liveness span (an hour, or the retention), lost
  no delivery, and over which the owner was present, its instance token
  held, from start to end.
  - **Why a window can be complete.** A member is live for one span after
    its last publication, so such a window heard every member live at its
    end. These are the premises `rate`'s clean pole already rests on (a
    lossless window of a whole period), plus presence. An owner that
    joined, left or re-minted mid-window is a new instance or a gap. A
    rule that refused every window would leave a stream's population a
    question no reading could ever answer no to.
  - Nothing else is complete: a GET that ended at its timeout, a shorter
    or lossy window or one with the owner absent (a member it never heard
    may be live), an archive.
  - A window shows no state population, however long, since a missed
    delete would count a member twice. A refusal narrows even a complete
    reading (§8.1, 0.8). A rate's spans are measured between stamps of
    one clock where the occurrences carry them, and on the receive clock
    otherwise.
- **What a tool concludes,** to "does this instance exceed its budget
  here?":
  - the finding: more members than the bound in any reading, or `<n>` + 1
    occurrences of one member less than the period apart in any window. A
    lower bound that exceeds is already the finding;
  - clean: at most the bound after a complete reading with a member (a
    complete GET, or a complete window), or, for a rate, a window of a
    whole period that lost nothing and heard an occurrence, none beyond
    the rate;
  - unobservable: within from an incomplete reading (a timed-out GET; a
    window short of one span, lossy, or with the owner absent), no member
    at all (O5), a rate window shorter than one period, deliveries lost;
  - not asked: no ceiling, a template without parameters, an operation, a
    resource without a rate.
- **Evidence:** new fixtures `conformance/budget/` (`bounds.json`,
  `population.json`, `rate.json`), written by hand from the text and
  checked by `zenkey-model/tests/conformance.rs`; Appendix C and E list
  them.

**Changed: wording.**
- **§2.2** points at §2.7 for what the bound counts. **§2.6** says the
  rate is per member, within any period that long. **§3.3**'s
  `cardinality` is the bound the owner keeps and a tool judges against.

**Found while writing it.**
- **An operation's `cardinality`.** E013 requires one on a templated
  operation, and nothing said what it bounds. It is the population of
  values the owner serves calls for. No reading of the owner counts it,
  since the values are the callers', so it is not judged at this version.
- **"At most 1/h"** could be read per calendar hour, as an average, or
  within any hour. It is within any hour, the one reading a tool can
  check from instants alone, and the one an owner keeps by counting.
- **An empty complete reading** could read as within any bound. An empty
  reply set is never a verdict (O5), and a refusal returns one too, so it
  is unobservable.

**Deliberately not changed.**
- **The presence budget (§8.3)** counts tokens in a presence domain, and
  is judged against a deployment's number, not a contract's. §2.7 counts
  members, and says the two are different.
- **D007.** Its range check is unchanged. §2.7 only says that a value it
  reports does not apply.
- **No new lint.** Nothing a contract could state is newly wrong: the
  bound, the rate and the retention keep their syntax and their codes
  (E013, E015, E026, E028).
- **No runtime rule.** The owner's MUST binds every runtime, and the
  reference runtime does not yet refuse a member beyond its bound: that
  is runtime work, not wire.
- **Profiles.** None was needed: `cardinality`, `rate` and `retention`
  are core fields, so the budget is core, and the tools' judgement of it
  no longer waits for #613.

## 0.23 — 2026-10-10: a clock ahead reported by a stream, and the first standard contract a profile publishes (#721, PD)

`health.v1` (text 0.1, draft) is the third profile written, and the first
with a standard contract (core §10, point 1). The contract was written
with the walkthrough, and moves under the profile unchanged. Writing the
text showed one place where the core asked for what its own rule forbids,
and three it left unsaid.

**Changed: rules stated.**
- **Ahead (§4.3): the report is not state.** 0.21 said an owner ahead MUST
  stop writing state and SHOULD report it, "`health.v1` is the standard
  way". `health.v1`'s status is state, so the owner could not. The core
  now says the report is a stream sample: a `health.v1` owner publishes a
  `faults` sample with code `clock_ahead`, and its status goes stale
  (`profiles/health/v1.md` §2.5). The core keeps its SHOULD, and the
  profile adds its MUST.
  - Evidence: `profiles/health/scenarios.md` §4, and `scenarios/state.md`
    §7's step 3, reworded to match.
- **A session's own stamps (§4.1, Appendix B).** A session with its HLC
  enabled, as every serving session's is (§4.3), stamps every put it is
  not given a timestamp for, a stream sample included
  (`Session::resolve_put`, zenoh 1.10.1). So an owner ahead puts its
  `clock_ahead` fault future-dated, and a router re-stamps it, or drops it
  under `timestamping.drop_future_timestamp`: §4.3 says so, since that
  setting loses the report. The same fact bounds §4.2's S1 control: the
  control's put comes from a session whose HLC is not enabled, as a
  client's is not by default, or it arrives with that session's own stamp.
- **One revision, two roots (§9.7).** A contract that moves keeps its
  revisions in the old root, which is append-only, and is published in the
  new one, copied byte for byte. `spec/profiles/.history` is the
  conventional root of the profiles' standard contracts. `health.v1`'s
  revision 1.0 (`sha256:e9dbcdcb…`) is in both, and `zk2 contract
  fingerprint` gives it before and after the move.

**Changed: wording.**
- **§8.2, "State values"** keeps its SHOULD, and says that a profile may
  make it a MUST for its own resources, as `health.v1` does for its status
  (§2.3). Evidence: `profiles/health/scenarios.md` §1.
- **§10** links `health.v1` among the profiles a rule names, and says where
  a standard contract lives: the profile's directory, published in
  `profiles/.history/`, the first being `health.v1`'s. An instance lists
  its `uses` in `profiles` and the interface in `interfaces`, as for any
  contract.
- **R7 and §8.5** point at `health.v1` §2.8: a status that crosses is
  liveness evidence, and health is unobservable where the face closes it.
- **§8.1**, the tokenless set, links `health.v1` §2.7. **Appendix C**
  indexes the two health scenarios the core now cites.

**Deliberately not changed.**
- **§4.3's SHOULD.** The core does not require any report, and still names
  no profile as a dependency: `health.v1` is the standard way, stated in
  the core's own terms (a stream sample).
- **§8.2's SHOULD.** Every other state value keeps it. An owner that holds
  no value at start has nothing to put, and `health.v1`'s status is the one
  every service has.
- **`profiles` in the descriptor (§3.3).** A profile that only defines a
  contract is neither a `uses` entry nor derivation-only, so an instance
  does not list `health.v1` there: its `interfaces` carry it, and a reader
  asks them (`health.v1` §2.7). The union stays as 0.19 defined it.
- **No W105 for `health.*`.** `health.v1` publishes no vocabulary, and
  Appendix D has no interim `health` table, so none of its keys is outside
  one (§10).
- **The tokenless set (§8.1, U22)** is unchanged. `health.v1` recommends
  itself for it near the budget, which §8.1 already says.
- **The archive (§4.4).** A health status is state like any other: an
  archive records it, and its answer is last-known.

## 0.22 — 2026-10-09: one major of a profile per contract (#726)

The Python implementation's live round against 0.20 (PR #725, F-98) built
a contract whose `uses` listed `views.v2` and `views.v10`, to test 0.20's
order. Both implementations accepted it. Yet nothing said which major a
key `views.<key>` belongs to, or which vocabulary W105 reads.

**Changed: a rule stated.**
- **One major per contract (§10 point 2; E002).** An annotation key names
  its profile without a major, so a contract uses one major of a profile.
  Two entries naming one profile at two majors are E002, reported once per
  profile, beside its malformed-entry case. New fixture:
  `contracts/e002-two-majors`. The reference lint now reports it, and no
  contract in `examples/zk2/` is affected.
- **A descriptor may still list both.** Two contracts of one instance can
  each use a different major, so the union in `profiles` (§3.3) can hold
  `views.v2` and `views.v10`. 0.20's order exists for that case.

**Deliberately not changed.**
- **Keys stay major-free.** A key spelled `views.v2.<key>` would make two
  majors expressible, but it changes every fingerprint that holds an
  annotation, for a case no contract has. A profile that needs both majors
  in one contract can publish a new major whose vocabulary holds both.
- **No new code.** E002 already covered `uses`' entries, and D010 is the
  descriptor's analogue: malformed, or listed twice.

hostid.v1 0.3 (#726, F-99) is its own changelog's: where an ephemeral
start is logged.

## 0.21 — 2026-10-09: the first published vocabulary, and a re-put (#720, PC)

`freshness.v1` (text 0.1, draft) is the second profile written, and the
first with an annotation vocabulary: `freshness.ttl_s`, the staleness
horizon of D16, which about 20 contracts of `examples/zk2/` already
declare. The maintainer decided on 2026-10-09 to write it before
`health.v1`, which uses it. Writing it showed what the core had to say
first.

**Changed: rules stated.**
- **A published vocabulary (§10 point 2, §9.2 W105, Appendix D).**
  Appendix D's `freshness` row moves to a table of published vocabularies,
  with the profile's text as its source: `profiles/freshness/v1.md` §4.
  W105 reads a profile's published table where it has one, and its interim
  table otherwise. The keys are unchanged (`ttl_s`), so no contract's codes
  change. §10 says that a published vocabulary replaces its interim table.
- **A re-put is a mutation (§4.2, after S7).** A put that repeats a
  member's current value, as an owner refreshing it makes, carries a
  timestamp the owner set (S1), minted above every stamp it issued (§4.3),
  and the owner's GET answers carry that stamp from then on (S2): a reply
  represents the latest put, not the latest change. Without it, a runtime
  could answer with the stamp of the last change, and a GET reader aging
  the reply (`freshness.v1` §2.6) would find a refreshed value stale.
  - Evidence: `profiles/freshness/scenarios.md` §1, step 3.
- **Ahead (§4.3).** An owner that stops writing state stops its re-puts
  too, and its members go stale, by design.
  - Evidence: `profiles/freshness/scenarios.md` §2, step 5.

**Changed: wording.**
- **§2.4** points at `freshness.v1` §2.4 for the `ttl_s/2` re-put it
  cites, and **R7** at its §2.9 for how a consumer measures freshness
  across a face. §10 names `freshness.v1` among the profiles a rule names.
  Appendix C indexes the two profile scenarios the core now cites.

**Deliberately not changed.**
- **No lint for an annotation's value.** A `freshness.ttl_s` that is not
  a horizon (`1.5`, `"60"`, a negative number) is the profile's to refuse
  (`freshness.v1` §2.1), which reads the resource's freshness as
  unobservable. The core's fixtures that carry odd `freshness.ttl_s` values
  (`e000-integer-range`, `e020-datetime`, `e028-*`, `ok-float-integral`)
  test the core's own lints, and keep their codes.
- **S2's wording.** "The mutation it represents" already covers a re-put,
  once a re-put is a mutation: the paragraph after the table says so,
  rather than a new rule.
- **R7's rule.** It still judges liveness from the freshness of what
  crosses, and says unobservable where nothing does. The profile states the
  measurement (a subscriber's receive clock), and adds streams to what can
  carry it.
- **The tombstone window (S3)** stays independent of any horizon. v1 tied
  tombstone visibility to `ttl_s`; zk2's owner answers its own deletes.
- **The HLC delta (§4.1).** The profile names it as the bound a GET
  reader's clock is trusted to, and the core's value stays 500 ms by
  default.
- **The core still depends on no profile.** Each new mention names
  `freshness.v1` as the standard way to meet a rule the core states in its
  own terms, as §2.4 and R7 already did by name.

## 0.20 — 2026-10-09: a provider on the service's own system, the order of `profiles`, and a derived address (#719, PB)

`hostid.v1`'s runtime landed (PB). A system the runtime mints is not
known before the service starts, so a binding cannot spell a provider on
the service's own host by name. The maintainer decided on 2026-10-09 that
`self.system/<service>` is a provider spelling, resolved at start like the
rest of the configuration. Implementing it, and the descriptor's
`profiles` beside it, showed three more things the core left unsaid.

**Changed: rules stated.**
- **`self.system` providers (§3.2, R1).** A provider whose system position
  is `self.system` names the service's own system, literal or derived:
  `self.system/<service>` and `self.system/*`. A runtime resolves it once,
  at start, and it holds across a re-mint. A tool, which is not a service,
  refuses one, as it refuses a parameter bound to `self.system` (R2).
  - New scenario `bindings.md` §5: a provider and a wildcard on the own
    system, delivered from that system alone, listed resolved, and refused
    to a tool.
- **Listed resolved (R3, §3.3 `requires`).** The descriptor lists such a
  provider as `<system>/<service>`, so a tool draws an edge to an address
  it can find. A parameter bound to `self.system` is still listed as
  configured.
- **The order of `profiles` (§3.3).** 0.19 said "sorted" without an order.
  It is now §9.5's order for `uses`: the name as a string, then the major
  as a number. A sort of the whole strings differs at a major of 10 and
  above (`views.v10` before `views.v2`), and where one name is another's
  prefix followed by a dot (`a.b.v1` before `a.v1`). The reference sorted
  strings, and now sorts as §9.5 does, with a unit test of both cases.
- **A derived address (§8.2).** The address, and every key built from it,
  is fixed before step 1 (a pure consumer's before its instance token). A
  system a profile derives is derived then, before the session opens where
  the runtime can, and a derivation that fails refuses the start as step 2
  does. The `self.system` providers resolve at the same time.
- **No implicit identity (§1.5).** A `self.system` binding is spelled in
  the configuration, so it is not the implicit identity §1.5 forbids.

**Found while implementing.**
- **`self.system` is a plain chunk** (§1.2 allows dots). A system could be
  named `self.system`, and a provider of that name would read as the
  keyword. R1 now says that at a provider's system position it always
  means the service's own system, a system of that name is bound only by
  `*`, and a deployment SHOULD NOT name a system `self.system` or
  `self.service`. R2 already read a parameter's value that way.
- **So D009 cannot see an unresolved provider.** A descriptor listing
  `self.system/teleop` passes D009, as a binding to a system of that name.
  Cascade 7 now says so; `bindings.md` §5 checks the runtime instead.

**Deliberately not changed.**
- **No new D code, and D009 unchanged.** A checker cannot tell an
  unresolved provider from a system named `self.system`, and the order of
  `profiles` is not checked: a tool reads the list as a set.
- **No `self.service` provider,** and no `self.system` at the service
  position: each is the literal chunk it spells. Binding a service to its
  own address has no use the core knows of.
- **No reserved system names in §1.2.** Refusing `self.system` as a name
  would change the lexical rules and `keys.json`. A SHOULD NOT is enough
  for a name nobody needs.
- **The core still depends on no profile.** §8.2 names `hostid.v1` as
  today's derived system, as §3.3 and §10 name it as today's
  derivation-only profile; the derivation, its inputs and its failures are
  the profile's (`hostid.v1` text 0.2).
- **`descriptor.schema.json`** and the descriptor fixtures. A resolved
  provider is an ordinary binding.

## 0.19 — 2026-10-09: profiles that only derive, and where profiles live (#719)

`hostid.v1` is the first profile to be written (#613's first tier,
decided on 2026-10-09). It defines no contract, no annotation and no kind:
it says how a service derives its system name from the machine id. The
core had no place for such a profile:
- §3.3 said `profiles` is the union of the contracts' `uses`, so an
  instance could not declare a profile that no contract uses;
- §10 said a profile contributes through "exactly four points", without
  saying it may use fewer;
- nothing said where a profile's text lives.

**Changed: rules stated.**
- **Derivation-only profiles (§10).** A profile contributes through the
  four points and through no other, and may use any of them. One that uses
  point 4 alone is derivation-only. It adds nothing of its own to the bus:
  what it derives goes where the core already puts it, and an instance
  that follows it lists it in the descriptor's `profiles`.
- **`profiles` (§3.3).** It lists the union of the contracts' `uses` and
  the derivation-only profiles the instance follows. Today that is
  `hostid.v1`, when the instance's system is minted.
  - New fixture `descriptors/ok-derivation-profile`: a descriptor listing
    `hostid.v1`, which no given contract uses, reports no code.
  - Cascade 7 now says why the check cannot verify the union: it cannot see
    which profiles an instance follows.
- **Where profiles live (§10).** Each profile has a directory under
  [`profiles/`](profiles/), with its text, its own changelog, fixtures and
  scenarios. [`profiles/README.md`](profiles/README.md) is the index, the
  template and the process. The first is `hostid.v1`, text 0.1, draft.
- **Wording that follows.** §1.5 points at `hostid.v1`'s text.
  `presence.md` §2's expectation of `profiles` follows §3.3. Appendix C
  indexes the profiles' evidence.

**Deliberately not changed.**
- **No new D code.** A checker still reads `profiles` for its form alone
  (D010). Whether a system is minted is the profile's question, and
  `hostid.v1` §5 answers it from the listing.
- **No registry of derivation-only profiles.** §3.3 names `hostid.v1` as
  today's, and the core depends on no profile. So no lint refuses a
  derivation-only profile in a contract's `uses`: the core cannot tell
  which profiles derive. `hostid.v1` advises against it (§2.8), and its §5
  reads such a listing as unobservable.
- **`descriptor.schema.json`.** Its wording, "the profiles this instance
  follows", already covers both halves of the union.
- **`meta` stays informative.** `hostid.v1` recommends `meta.host`, and
  `meta.zid` of consumers as well as owners. Both are the core's own
  members, and no checker reads them. A profile has no point in `meta`.
- **How a binding names a provider on its own system**
  (`self.system/<service>`, decided with `hostid.v1`) is not in this
  amendment. It comes with `hostid.v1`'s runtime (#719).

## 0.18 — 2026-10-09: two words 0.17 left loose (#716)

The Python implementation read 0.17 cold (PR #715). Both runners passed,
and 0.17's three rules held. Two words in them did not.

**Changed: wording only.**
- **D011's place among the cascades (§3.3; F-92).** Cascade 6, now 7,
  still listed `token` as deliberately unchecked, and no cascade said
  whether a malformed `contract` (cascade 3) suppresses D011. A new cascade
  6 says D011 reads `iface` and `token` only, so it is reported whatever the
  entry's `contract` says, and with or without that contract given. The
  reference already did this. The new fixture `d011-bad-fingerprint` expects
  D003 and D011 together.
- **"Verified at least one router" (§4.2, "A tool's S1 check"; F-93).** 0.13
  calls the routers a session is connected to "verified routers", so the
  phrase could be met by a connection alone. Read that way, a client tool
  with the admin space off would call an owner's own stamp clean. 0.17's
  own list of causes (no admin read, admin space off, no replier id) shows
  the intent: a router's own answer, counted. The text now says so. Both
  implementations already read it that way.

**Deliberately not changed.**
- **"Verified router" keeps its 0.13 meaning.** Verification still starts
  from the routers a session is connected to. Only S1's premise is
  restated, as an answer counted.

## 0.17 — 2026-10-09: what a tool needs that it cannot read off the bus (#713)

The Python implementation read 0.16 cold (PR #712, F-89 to F-91). Its rules
held, but three of them left an implementation guessing about something no
tool can observe: whether an owner is its own router, what its grants are,
and where a tokenless archive is refused. Each gap becomes a rule. In two
places the reference changes, because it was the implementation that guessed
wrong.

**Changed: rules stated.**
- **S1 without a verified router (§4.2, "A tool's S1 check"; F-89).** A
  foreign stamp is a finding whatever else the tool read: an owner that is
  its own router stamps with its own `meta.zid`. An owner's own stamp is
  clean only when the tool verified at least one router, and `meta.zid` is
  none of the zids it knows to be routers. Those are the routers its session
  is connected to, the routers it verified, and every zid a verified router
  lists as a `router` session. Otherwise S1 is unobservable for that owner.
  - The reference doctor used to judge by `meta.zid` alone when it had not
    read the admin space, and read it only for other checks.
    `state-stamp-foreign` now reads it, and `check conform`'s `state-stamp`
    does too, so that a conforming service can still pass. Both apply the
    rule through one function, with a unit test for each pole and a live
    one beside a router whose admin space is on.
  - Python's guess differed in one place. It held a foreign stamp
    unobservable when no router was verified. That stamp is a finding,
    because the owner's own router could not have made it.
- **Where a tool learns its grants (§5.1 O3; F-90).** No tool can observe
  its grants (§11.3). It learns that they let it call from its operator, or
  from the deployment's §11.1 input. A deployment without access control
  lets everyone call. Told so, a silence from a present owner is the
  finding. Not told, the silence is unobservable, because O5 forbids taking
  an empty reply set as a verdict.
  - The reference `check conform` had held such a silence as a finding,
    with a caveat, which is the reading O5 rules out.
  - It now takes `--calls-granted`, the operator's word, after the operator
    alternative of §4.2 (`--trust-admin-space`). Its live test passes the
    flag, and checks that without it the silence is unobservable.
- **Where a tokenless archive is refused (§4.4, §8.2 step 2; F-91).** Step 2
  refuses an owner whose tokenless set names `archive.v1`, whether or not
  it implements the interface, because the set is the deployment's
  configuration. A descriptor that marks `archive.v1` `"token": false` is the
  new D011.
  - The reference moved its refusal from `Archive::start` into the runtime's
    step 2, so every owner is checked.
  - `presence.md` §2 gains step 6, watched through R1 against a control.
  - New descriptor fixtures: `d011-tokenless-archive` and `ok-archive`.

**What this costs.** zenoh leaves a router's admin space off by default. A
tool that cannot read it now reports S1 unobservable, where it used to read
clean. That is the honest outcome, and the same one the doctor gives for a
storage it cannot see. A deployment that wants S1 judged from outside turns
its routers' admin space on, and grants its tools the admin read (§11.1).

**Deliberately not changed.**
- **No operator word that owners are not routers.** The S1 premise could
  have had a flag like `--calls-granted`. It is not needed, because the
  admin space answers the question when it is on.
- **The descriptor does not state its session's mode.** `meta` stays
  informative and unchecked (§3.3). A mode no tool could verify would not
  replace the routers' own word.

## 0.16 — 2026-10-09: what the tools' last verbs could not decide (#708)

FK1 built zenctl's `why`, `check conform`, `storage gen` and `admin graph`
for zk2 (#702–#705, PR #709). It found seven places where the spec left a
tool guessing. The reference's behaviour becomes the rule, with two small
reference changes. Each rule has a test.

**Changed: rules stated.**
- **Retention is not a storage's to enforce (§2.6, Appendix B).** zenoh
  1.10.1's storage manager garbage-collects tombstones, not values, so a
  union storage never prunes an occurrence by its retention. The retention
  is the bound a consumer applies on replay. `storage gen` warns
  `retention_not_enforced`.
- **An archive is never tokenless (§4.4).** Consumers and tools find
  archives by their `archive.v1` token for last-known reads (S6), so a
  tokenless archive would be invisible. The reference runtime now refuses
  to start one; there is a test.
- **O3 judged from outside (§5.1).** A tool counts a silence against "every
  call is answered" only under grants that let it call, because an
  access-control refusal is silent too. It says so beside the finding.
  `check conform` already did.
- **A tool's S1 check (§4.2).** A tool that reads the admin space compares
  the owner's `meta.zid` with the verified routers' zids, by value. When
  they match, the owner is its own router, and S1 is unobservable for it.
  The reference doctor's `state-stamp-foreign` now does this; there is a
  unit test.
- **Appendix B** records that §4.2's admin-space verification rests on the
  unstable `Reply::replier_id`. A release that removed it would leave every
  admin answer unverified, and the checks unobservable, never clean.

**Deliberately not changed.**
- **`meta.zid` stays a SHOULD (§3.3).** Both implementations write it. An
  owner without it is reported unattributable everywhere a tool needs it,
  which the tools already say, and never wrongly judged.
- **No union-storage grant is generated (§11.1).** 0.14 states the grant,
  and 0.15's input has no storages yet. The generator waits for a
  deployment that runs one off-router.

## 0.15 — 2026-10-09: what §11 needs to be built from (#695)

The Python implementation built §11's access control from the spec alone
(PR #694): a generator for both postures, run live on zenoh-python
routers. **§11 was not enough on its own.** It had to invent or measure
the input, the message pairs, a tool's admin read and the complement's
key set. It also found two errors, one of them mine in 0.12. These are
F-82..F-88. The reference generator (FJ7) is the rule where it was right,
and it is extended where it lacked a grant.

**Changed: rules stated, so that §11 can be built from.**
- **The input (F-82, §11.1)** is stated abstractly:
  - principals, each bound to a user or a CN;
  - services, with the contracts they implement;
  - bindings, calls and inspected services;
  - whether a tool reads the admin space;
  - archives, and the namespace.

  The format is a generator's own. The reference's enrollment is
  informative.
- **Messages and flows (F-83, §11.2)** are a table per grant. Both
  implementations measured the same pairs. A liveliness read needs its
  egress `liveliness_token` too.
- **The complement's key set (F-85, §11.2)** is the deployment's own keys:
  declared resources, each service's `@zk/**`, and the contract keys.
  Undeclared keys stay open under `allow`.
- **"Every principal" is compiled into each policy (F-87, §11.2).** In
  zenoh 1.10.1, measured, a catch-all subject makes the per-user subjects
  lose their denies. The reference already compiled the `@/**` deny into
  each policy.

**Changed: a grant added, and the reference generator extended.**
- **The admin read (F-84, §11.1 Tool).** Under `deny`, no grant reached the
  admin space, so a tool could not check S4.
  - A tool that checks S4 or runs a doctor now holds `query` on
    `@/*/router` and `@/*/router/**`, and their `reply`, never namespaced.
  - The reference enrollment's `[[tool]]` takes `admin = true`. The
    generator emits `admin-read-in` and `admin-read-out` (grant
    `admin_read`).
  - The walkthrough enrollment's `ops` tool uses it, and the pinned plans
    are re-blessed.

**Changed: two errors fixed.**
- **What a storage is (F-86, §4.2, against 0.11).** A router's admin space
  records each queryable as `@/<zid>/router/queryable/<key expr>`. The
  record of a `**` queryable, such as an owner's `…/state/**`, answers
  S4's storages selector. So 0.11's "nothing under the second selector"
  never held with an owner present.
  - A storage is now an answer whose key the selector includes, ending
    `…/storage_manager/storages/<name>`.
  - The reference's parser already read it that way. The text was wrong.
- **`security.md §3` step 3 (F-88, against 0.12).** It expected R1 to
  refuse `S` under each posture. Under `allow`, a session that is no
  principal matches no subject and gets everything (§11.3), so its spoof
  is answered, and only the replier check keeps S4 from clean. The step
  now runs `S` as a principal and as a non-principal, with the right
  expectation for each.

**Deliberately not changed.**
- **No input format is made normative.** Deployments describe themselves
  in many ways. The spec states what a generator must know, not how it is
  written.

## 0.14 — 2026-10-09: what access control measured (#689)

zk2's `acl gen` (#612, FJ7, PR #692) was built against spike S14's
principals and run live on zenoh 1.10.1. It measured four facts §11 had
wrong or left out, and found nine gaps. The generator's behaviour becomes
the reference.

**Measured, now stated.**
- **Value replies and error replies are checked differently (§11.2).** A
  value reply is checked against its own key, and an error reply, which has
  none, against the query's. S14's general claim holds only for refusals.
  So a provider's value replies pass through Own, and the consumer
  selectors granted for its ingress `reply` exist for its refusals.
- **Under `allow`, an ungranted wildcard GET gets nothing back (§11.3).**
  Each value reply is denied by its own key.
- **Under `allow`, a wildcard call is in no deny (§11.3).** A `fanout =
  "allowed"` operation executes on every provider for a principal never
  granted it. Such an operation that must not run for everyone MUST be
  `fanout = "forbidden"` under `allow`, or the deployment runs `deny`.
- **A far router in a south region needs declarations sent toward it
  (§8.5).** Its queries are never routed without an egress
  `declare_queryable` grant toward it. The generator's `face-declarations`
  rule is this grant.

**Changed: the grant shapes (§11.1).**
- **Own** now names `reply` on ingress, and its egress: queries and
  subscriber declarations on its own keys. Both came from S14, unstated
  until now.
- **Presence reads the descriptor too.** It now includes the descriptor's
  GET and subscription (§3.3), which share the `@zk` subtree.
  - Without them, a consumer could not resolve a token's `fp16` (§8.4),
    and a tool could not draw the graph.
  - The generator's presence rules gain `query` and `declare_subscriber`
    on ingress, and `reply` and `put` on egress.
  - FJ7's live test now reads a descriptor under the grant, and finds it
    silent without.
- **A Tool shape**, for a principal with no address: Consume, Call and
  presence on what it reads, calls and inspects.
- **Archives:** Own, Consume on their records and on peers' archive forms
  of them, and presence on owners and peers, as the generator emits.
- **History:** a consumer's advanced subscriber MUST NOT declare a
  subscriber-detection token, whose key would sit under the provider's
  prefix. The reference consumer declares no advanced subscriber.
- **A union storage** (§2.6) is the one cross-principal serving grant, and
  a deployment names it explicitly. It is not generated at this version.
- **The namespace:** grant keys carry it, and the admin space never does.

**Changed: other rules stated.**
- **§11.2:** an R2-narrowed grant has no complement by inclusion, so under
  `allow` its unnamed members stay readable. The generator warns
  (`complement_partial`).
- **§11.3:** a session matching no subject gets `default_permission`, so a
  deployment under `allow` MUST refuse unauthenticated sessions at the
  link.
- **§8.5:**
  - access control is per hop, and the near router sees a far router as
    one principal carrying the union of its side's grants;
  - `link.v1`'s exceptions to the `@stream` deny are its own (#613), and
    the generator denies `@stream` wholesale.

**Deliberately not changed.**
- **No union-storage generation.** The shape is stated; emitting it waits
  for a deployment that runs one.
- **`link.v1` downsampling** is not modelled in the generator.

## 0.13 — 2026-10-09: a far router is verified through the routers that list it (#687)

The Python implementation's round against 0.12 (PR #686) found F-81.
0.12 verified an admin answer only when it came from a router the tool's
session is connected to, and a client connects to one router at a time
(Appendix B). So with two linked routers, a client tool got the far
router's honest answer and could not count it, and S4 could never be clean
in a deployment of two or more routers.

**Changed: rules stated.**
- **Verified routers, outward (§4.2, "Who answered").**
  - The routers a tool's session is connected to, and the session itself,
    are verified.
  - So is every zid a verified router's own answer lists among its
    `sessions` with `whatami` `router`, and so on outward.
  - An answer counts when its replier id is its key's zid and that zid is
    a verified router.
  - **Measured on zenoh 1.10.1** (`admin_spoof.rs`): R1's document lists a
    linked R2 as `router` and every client as `client`, the spoofer
    included, and each answer carries its sender's own replier id. Appendix
    B states the document's shape.
- **`security.md §3` step 4** runs the far router and a self-consistent
  spoof together.

**Changed: the reference, here.**
- **The doctor** verifies routers outward through those session lists, and
  says why each unverified answer is unverified:
  - no replier id;
  - a replier other than its key's router;
  - its router's own answer unverified;
  - no verified router lists it.

  A live test runs two linked routers with a client tool: both routers are
  verified and S4 is clean, and a spoofer on its own key is not trusted.

**Deliberately not changed.**
- **A tool need not connect as a peer to every router.** A peer tool
  connected to each router verifies them all directly, as zk2py measured,
  but a deployment's tools are usually clients. The outward rule gives the
  same answer through one router.
- **Unmeasured:** whether a storage manager's own admin replies carry the
  router's replier id. An in-process router cannot load the plugin. If
  they did not, their storages would read unverified, which withholds a
  clean verdict and never invents one.

## 0.12 — 2026-10-09: who may answer the admin space (#684)

The Python implementation's round against 0.11 (PR #683) found F-80:
**any session can answer the admin space**. A plain client declaring a
queryable on `@/<zid>/router` turned S4's check from unobservable to
clean: the false clean 0.10 forbids. The reference's doctor had the same
hole. It is fixed here.

**Measured, beyond the finding.** zk2py's first defence was to accept an
answer only for a router its session is connected to, judged by the zid
in the key. That is not enough. A spoofer chooses the key, so it can
answer on the real router's own key. The reference measured this on zenoh
1.10.1 (`zenkey/tests/admin_spoof.rs`):
- with the admin space off, the spoof on the router's own key is the
  only answer;
- the reply's replier id names the spoofer, not the router.

The replier id is what tells them apart. The key and the document never
do.

**Changed: rules stated.**
- **Who answered (§4.2).** A tool counts an admin answer as a router's
  only when the reply's replier id is the zid its key names, and that zid
  is a router its session is connected to, or the session itself.
  - Any other answer is unverified, and never contributes to a clean
    verdict.
  - The replier id is unstable API (Appendix B, now stating what it
    names). A tool that cannot read it holds every answer unverified.
  - An operator MAY tell a tool to trust every answer when the grants deny
    `@/**` queryables to every principal, which no tool can observe.
- **No principal declares queryables under `@/**` (§11.1).** The routers
  serve the admin space themselves. A generator allows it to none under
  `deny`, and denies it to every principal under `allow`. §11.3 states the
  fact.
- **`security.md §3`** runs the spoof with the admin space off and on, and
  then under generated grants. That last step waits for FJ7's generator.

**Changed: the reference was wrong, and is fixed with this amendment.**
- **The doctor (`zenkey-fleet`)** read every admin answer as a router's.
  - Now it records each reply's replier id (`AdminEntry::replier`) and
    verifies every answer as above. It lists unverified ones as unjudged,
    naming the key and who answered, so `storage-on-state` and
    `router-version-skew` are never clean beside one.
  - `DoctorSpec::trust_admin`, which is zenctl's `doctor
    --trust-admin-space`, is the operator's alternative.
  - The live storage test played a storage manager from a raw session,
    which is the spoof, so it now runs trusted.
  - A new live test runs the spoof with the admin space off and on.

**Deliberately not changed.**
- **The core still requires no unstable API.** The replier id is how a
  tool verifies, and a tool that cannot read it loses the clean verdict,
  not its correctness.

## 0.11 — 2026-10-09: how a zid compares, and what S4's check reads (#681)

The Python implementation's round against 0.10 (PR #680) found three gaps,
F-77 to F-79, and one editorial slip in 0.10. One gap was a real bug in
the reference's doctor, and is fixed here.

**Changed: the reference was wrong, and is fixed with this amendment.**
- **A zid compares by value (F-78, §3.3, Appendix B).** zenoh 1.10.1 writes
  a zid as lowercase hex without leading zeros: the owner example's
  `meta.zid` was 31 digits, where `descriptors/ok-full` shows 16. The
  doctor's `state-stamp-foreign` compared the two texts, so a `meta.zid`
  spelled with a leading zero, or in capitals, would have called an owner's
  own stamp foreign.
  - **The rule:** a tool MUST compare two zids by value, never as text. An
    owner SHOULD write `meta.zid` as zenoh writes it.
  - **The fix:** the doctor now compares by value. A unit test pins it.
  - **What did not need fixing:** zk2py already compared numerically. The
    tooling guide's O7 now says so too.

**Changed: rules stated.**
- **What S4's check reads (F-79, §4.2).** 0.10 sent a tool to "the
  routers' storage admin space" without naming its keys. The reference
  reads two selectors:
  - `@/*/router`, for the routers that answer;
  - `@/*/router/**/storage_manager/storages/**`, one key per storage, its
    value the storage's configuration with its `key_expr`.

  A storage whose `key_expr` intersects an owner's state breaks S4. A
  router that answers the first selector and has nothing under the second
  runs no storage. When no router answers, the check is unobservable.
  zk2py read the router document's `plugins`, which agrees for a router
  with no plugin.
- **A role's `optional` against its contract (F-77, §3.3).** For a role a
  contract declares, a tool takes the need from the contract. A
  disagreeing `optional` is listed under "Not checked, deliberately", and
  `optional: false` written out is the same as absent.
  `descriptors/ok-optional-unchecked` pins both, with no code. This is what
  the reference and zk2py both did.

**Editorial.** In §9.8, 0.10's bullet on revision order was inserted inside
a paragraph and swallowed its second half ("Each rule below is a
transition…"). The paragraph is whole again, and the bullet follows it.

## 0.10 — 2026-10-09: what a doctor can and cannot decide (#677)

Writing zk2's `doctor` (#612, FJ6, PR #678) found eight places where a
check was undecidable, or rested on something the spec called
informative. The doctor's behaviour becomes the rule, as the reference's
has since 0.5. Two items are runtime fixes, made here.

**Changed: two wire additions.**
- **`requires[].optional` (§3.3).** A role a component's manifest declares
  (`declared_by: null`) had no way to say whether it is required, so a
  tool could not grade an unsatisfied one.
  - The entry now carries `optional: true` for a role the instance works
    without, and nothing otherwise: absent is required.
  - For a contract's role, it repeats the contract.
  - The reference runtime fills it from the role. It writes it only when
    `true`, so a descriptor without optional roles is byte-identical to
    0.9's. An older strict reader refuses an entry that carries it (D000).
  - `descriptors/ok-optional-role` pins it, and `descriptor.schema.json`
    is regenerated.
- **`meta.zid` (§3.3).** S1's attribution, and the tooling guide's O7,
  compare a state stamp's id with the owner's zid. The only place a tool
  learns that zid is the descriptor's `meta`, which 0.9 called
  informative.
  - An owner SHOULD now state `meta.zid`, as the reference always has.
  - The rest of `meta` stays unchecked.
  - Without it, a stamp's clock is unattributable, never foreign.

**Changed: rules stated.**
- **S4 needs the admin space (§4.2, Appendix B).** A tool checks S4 in the
  routers' storage admin space, which zenoh 1.10.1 disables by default. A
  deployment that wants the check enables it, read-only, for its tools.
  Without it, the check is unobservable, never clean.
- **A fault read from presence shapes holds in two reads (§8.1).** §6's
  grace rule for split-brain now covers every shape: a token a descriptor
  does not list, or an exposed interface with no token. Start-up, re-mint
  and teardown pass through those shapes briefly, by design. The doctor
  already required two reads.
- **What a tool can count (§8.3).** A reader lists only liveliness tokens,
  and only those its selectors and grants reach. Its count is a lower bound
  of the budget's declarations, and it says so.
- **Revisions carry no order on the bus (§9.8).** A bundle keeps no
  `minor`. A tool MAY order two revisions by the `minor` their descriptors
  state, when the two differ. Otherwise it classifies both ways: clean only
  when both directions are compatible, a finding when neither is, and
  undecided when they disagree. `minor` stays unchecked (§3.3).
- **What a tool can see of an archive (§4.4).** No alignment status is
  published at this version: a tool sees each key's `confirmed`, so an
  empty archive reads as aligned, and the tool says so. A status is
  profile work (#613).

**Changed: the runtime, here.**
- **`zenkey::shm::memlock()`** returns `Limited`, `Unlimited` or
  `Unknown`. `memlock_limit()` folded the last two into `None`, so the
  doctor read an unreadable limit as unlimited. The doctor's
  `shm-memlock-low` is now unobservable when the limit is unknown.
- **The doctor reads `optional`** for a manifest role. A required one
  unbound is an error, and an optional one is info, where 0.9's doctor
  could only warn.

**Deliberately not changed.**
- **The descriptor's `format` stays `zk2-descriptor/0.1`.** The addition
  is optional and written only when `true`. The core is pre-1.0, and every
  implementation follows each amendment.
- **No alignment status is invented for `archive.v1` here.** That belongs
  to the profile's own spec (#613), not to the core.

## 0.9 — 2026-10-08: the order of an owner's refusals, and a scenario 0.8 got wrong (#670)

The Python implementation's round against 0.8 (PR #669) found three things,
and one reference bug. They are resolved as before: **the reference's
behaviour becomes the rule**, unless it is a bug. Here the bug was in the
reference's interop example, and is fixed with it. Each rule is pinned by a
scenario step that the reference runs as a test.

**Changed: rules stated.**
- **The order of refusals (F-74, §5.1, "Answering").** Before any handler
  runs, an owner refuses a call for the first of these that applies:
  1. a key expression that is not concrete, on an operation that forbids
     fan-out (`fanout_forbidden`, O2);
  2. an operation not exposed now (`unavailable`, O3);
  3. a key that names no member (`invalid_request`).

  O2 and §5.1 both claimed a wildcard call to a fan-out-forbidden template
  whose parameter chunk is not canonical. The reference checks O2 first,
  and so did zk2py. `operations.md §2` step 1 now makes that call.
- **Sending nothing needs no member (F-76, §5.1, "Answering").** Only a
  reply needs a key. A server over a template whose handler names no
  member and sends nothing ends the call as zero values, then completion,
  when no `summary` is declared. When one is declared, it is answered
  `internal`, as any `many` handler without its summary is. This is the
  reference's behaviour and zk2py's guess. `operations.md §2` step 6 pins
  it.

**Changed: a scenario 0.8 got wrong.**
- **presence.md §6 step 3 (F-75).** It read `zk2/**`. By §1.3's guard, that
  selects no control token, so its "no token" could never fail, timeout or
  not. It now reads `zk2/*/*/@zk/instance/*` with an owner present: the open
  read holds the token, and the held read is empty with `Timeout`. The
  reference test it mirrors had the same flaw, and is fixed the same way.
  zk2py already read both.

**Changed: the reference was wrong, and is fixed with this amendment.**
- **The interop owner example exposed templated operations it did not
  serve.** `zenkey/examples/owner.rs` declared queryables only for
  operations without template parameters. Calls to the templated ones it
  exposed were silent, which breaks O1 and §8.2's "exposed" (alive ⇒
  callable). These were zk2py's 2 XFAILs in `py-live`.
  - **The fix:** it now serves every operation, a templated one over the
    whole template (§5.1). A call whose key binds the parameter is echoed
    on that member's key. An echo to a fan-out that leaves the parameter
    unbound names no member, and is refused `internal`, as §5.1 says.
  - **The test:** `owner_example.rs` checks both.

**Deliberately not changed.**
- **No new error code** for a fan-out that leaves a parameter unbound on a
  server that cannot name a member. `internal` stays, because the server is
  the one that could not answer, and §5.1 already says so.

## 0.8 — 2026-10-08: a refused presence read, and what implementing 0.7 found (#664)

Two sources, 10 items:
- **The runtime catching up with 0.7 (#660, FH2)** found that a presence
  read refused by access control looks complete and empty, plus six
  smaller gaps.
- **The Python implementation's round against 0.7 (#663)** found three:
  F-71 to F-73.

Each is resolved as in 0.5–0.7: **the reference's behaviour becomes the
rule**, unless it is a bug. One was a bug, in `zenkey-model`, and is fixed
here (the naming of an undecodable type, below).

A fixture pins each static rule: 3 new compatibility cases. No existing
expectation changed. The live rules land in scenarios:
- `presence.md §6`, new;
- `operations.md §1`, rewritten, and `§2` steps 4–5;
- `state.md §1` step 3;
- `types.md §2`;
- `security.md §1`.

The reference runtime gains a test for each live rule it had not yet
shown.

**Changed: rules stated, by section.**
- **A refused read is complete, and empty (§8.1, §11.1, §11.3, O5,
  Appendix B).** This is the important one.
  - **The behaviour.** A router whose access control refuses a liveliness
    GET answers it the way it answers a selector no token matches: a
    final reply, no token, and no error reply. Measured on zenoh 1.10.1 with
    a `liveliness_query` deny on the query's ingress at the router. The same
    rule on `egress` alone refused nothing.
  - **Why it matters.** O5 has a tool attribute silence through presence. A
    tool that is refused both a call and the presence read reports the
    service absent. That is a wrong verdict, and it is indistinguishable
    from a right one.
  - **The rule.** The Consume and Call grants now include liveliness reads
    on the `@zk` subtree of every service they name. A tool reports absence
    as what its reader could see, and says so where it cannot rule out a
    refusal.
- **A read that timed out shows it (§8.1, Appendix B).** In zenoh 1.10.1, a
  liveliness GET that reaches its timeout ends with the error reply
  `Timeout`, and one the routers finished ends with none. A read with any
  error reply is possibly incomplete. The reference reads completeness
  this way since #660, instead of guessing from elapsed time.
- **`["null"]` is the null schema (F-71, §7.3).** A `type` is read as a set
  of names everywhere in the subset, so `"null"` and `["null"]` are one
  null schema, and a nullable form may spell its null branch either way.
  This is the reference's behaviour. `nullable-null-as-list` pins it.
- **"As written" is by text (F-72, §9.8).** Inside `oneOf`, `anyOf` and
  `prefixItems`, a `$ref` back to a target already being followed is
  compared by its `$ref` value and siblings, annotations dropped, not by the
  target it resolves to.
  - **The consequence.** Renaming a recursive definition reached from inside
    one of those keywords is review, although the type is the same.
  - **Pinned by** `anyof-recursive-renamed`. Its control,
    `anyof-recursive-described`, shows that recursion ends and that an
    annotation is no change.
- **The tick is not observable from outside (F-73, `state.md §1`).** A tester
  can neither arrange two puts within one clock reading nor tell from the
  stamps that they fell in one.
  - **The scenario** now asks only that v4's stamp exceed v3's.
  - **The tick itself** is checked with a clock the implementation controls.
    The reference does it in §7's catch-up: its clock reads behind the
    record, and the stamp is the record plus one NTP64 unit.
- **Observing target and consolidation (`operations.md §1`).** zenoh 1.10.1
  does not give a queryable a query's target or consolidation, so a capture
  at the server cannot show them. The scenario shows them by behaviour, and
  that is the evidence it asks for:
  - a call returns on its first reply while its server holds the query
    open (`None`);
  - a second instance on the same router runs none of the calls the first
    runs (`BestMatching`).

  A capture of the caller's outgoing queries MAY add to that evidence.
- **One member per call, whatever `replies` is (§5.1).** The text said it
  for `replies = "one"`. A template-wide server answers for one member per
  call with `"many"` too: every value goes on that member's key, and naming
  a second member is refused to the handler (`operations.md §2` step 5).
- **A non-canonical chunk names no member (§5.1).** A server over a
  template refuses `invalid_request`, before any handler runs, a call whose
  concrete parameter chunk is not a canonical slug (§1.4). This holds for a
  fan-out as for a concrete call (`operations.md §2` step 4, `§3` step 3).
- **From a token to a fingerprint (§8.4).** An interface token's `fp16` is
  not enough to retrieve by. A tool reads the full fingerprint from the
  instance's descriptor, and retrieves by that. There is no retrieval by
  prefix.

**Changed: the reference was wrong, and is fixed in `zenkey-model`.**
- **An undecodable type is named as a decoded one is (§7.2, `types.md §2`).**
  `decode` named a JSON Schema type that failed to decode with its wire
  (`json:Status (cbor)`), while a tool names a decoded one `json:Status`.
  `decode::declared` is now the one spelling, the type reference as written
  (§9.1). The wire moves into the reason (`as cbor: …`). The fleet's
  renderer takes the function rather than keeping a copy.

**Recorded: the fix belongs to other work.**
- **The grant generator (#612, FJ7, `acl gen`)** MUST emit the new
  liveliness reads of Consume and Call. `security.md §1`'s new step is
  unmeasured until it does.
- **The Python implementation** follows 0.8 in its next round. Its live
  harness still marks the Rust owner example's two 0.7 deviations
  (F-65, F-68) as known. Both now pass since #660, which was measured:
  129 passed, 0 failed, 2 XPASS.

**Deliberately not changed.**
- **No probe that tells a refusal from absence.** None was measured. A
  reader's own tokens show nothing about what it may read elsewhere, since
  access control is per key expression and per hop. The fix is in the
  grants.
- **No retrieval by prefix.** The descriptor already carries the full
  fingerprint. A GET on a wildcard over contract keys would reach every
  holder of every revision. And a 64-bit prefix is a weaker check than the
  hash §9.6 verifies.
- **A recursive `$ref` is not compared by its target.** That needs a
  comparison of two reference graphs, which the classifier does nowhere
  else. A rename costs one review, never a wrong compatible.
- **S7's tick stays.** Only the scenario's claim to observe it from outside
  is withdrawn.

## 0.7 — 2026-10-08: the live findings, the operations runtime's decisions, and the codegen's gaps (#609, #621, #611)

Three sources, 22 items:
- **The Python implementation's live half (#609):** a minimal owner, state
  reads and calls, interop with the Rust owner both ways. It found 7 places
  where the spec was silent or ambiguous: F-64 to F-70.
- **The operations runtime (#621),** the first implementation of §5 and
  §6, made 13 decisions where they were silent: O-1 to O-13.
- **The codegen (#611)** found 2 gaps at the spec's level: C-1 and C-2.

Each is resolved as 0.5's and 0.6's were:
- **the reference's behaviour becomes the rule**, stated where a reader
  looks for it; or,
- **where that behaviour was a bug**, the corrected rule is stated. The
  classifier is `zenkey-model`'s, and is fixed here. The runtime
  (`zenkey/`) and the codegen (`zenkey-build/`) are not changed by this
  amendment: their fixes are listed below, for the work that owns them.

Resolving C-1 found one more bug, beyond the 22: the classifier never
looked through a `$ref` inside `oneOf`, `anyOf` or `prefixItems`, so a
breaking change could classify as compatible. It is fixed, and listed as
X-1.

A fixture pins every static rule a fixture can check: 8 compatibility
cases and 4 error envelopes. No existing expectation changed. The live
rules land in their scenarios: `state.md §1`, `presence.md §1–§2`, and
`operations.md §1–§8`.

**Changed: the reference was wrong, and is fixed in `zenkey-model`.**
- **A nullable is one type (C-1, §7.3, §9.8).** schemars 1 writes
  `Option<T>` as `{"type": [T, "null"]}` for a scalar, and as
  `{"anyOf": [S, {"type": "null"}]}` for a referenced type; other
  generators write the second for every optional value. The classifier read
  them as two types: a move from one to the other was `type_changed`,
  breaking, although nothing a reader or writer does changes.
  - **The rule:** the second spelling, with nothing else beside the `anyOf`
    that carries meaning, is the *nullable form*. Its *reading* is S, its
    `$ref`s followed, with `null` added to its `type` and, where it has
    one, to its `enum`. It exists when that S is an object with a `type`
    and without `const`, `oneOf` or `anyOf`, the keywords that constrain a
    null too. A form with a reading is compared as its reading.
  - **The one exception:** a form against an `anyOf` that is not one is
    compared as written, both sides, as any `anyOf` changed is. So
    `anyof-add-branch`, whose new revision adds a `null` branch to a
    one-branch `anyOf`, stays review.
  - **Why this rule.** Of the three the decision weighed, it is the only
    one that loses nothing:
    - *Keeping two types, and calling a move between them review* still
      asks a human to accept a generator upgrade, and leaves every change
      inside S undecided.
    - *Reading the `type` list as the `anyOf`* would make a change inside
      `["integer", "null"]`, breaking today under the `type` rule, review.
    - *The reading* moves the comparison into the decided rules: a change
      inside S is classified as one outside a nullable is, through S's
      `$ref`s. The conditions are those under which `null` added to the
      `type` (and the `enum`) means exactly "S, or null".
  - **What it changes.** On the new cases, 0.6's classifier said breaking
    for `nullable-type-to-anyof`, `nullable-anyof-to-type` and
    `nullable-enum-inlined` (now compatible), review for
    `nullable-inner-retyped` (now breaking), and compatible for
    `nullable-ref-target-changed` (now breaking).
- **Undecided keywords compare through their `$ref`s (X-1, §9.8).** 0.5
  wrote "a `$ref` there is not followed", while §7.3 says any change inside
  `oneOf`, `anyOf` or `prefixItems` is review. A definition reached only
  from inside one was compared nowhere, so a change to it, a field retyped
  included, classified as compatible. Measured on 0.6's classifier: an
  `Option<Struct>`'s struct with a field retyped, and a `oneOf` variant's
  payload retyped (`oneof-ref-target-changed`), both compatible. Inside an
  undecided keyword, a `$ref` is now compared by its target, siblings
  added: a change there is review, and inlining a definition is no change.
  A `$ref` back to a target already being followed is compared as written,
  which ends a recursive type, and a dangling one is `schema_unreadable`,
  as outside.

**Recorded: the runtime and the codegen are wrong, and the fix is theirs.**
Each item states the rule the fix must meet; none is made here.
- **A raw `error` type's detail (F-65).** The `Raw` codec's detail is
  bytes, which a JSON envelope refuses, so every `app` with a raw detail
  goes out as `internal`. §5.2 now says the detail is base64 text.
- **`app` without a detail (F-65).** `OpError` has no constructor for one,
  and the owner example sends `{}`, or empty protobuf bytes, as the detail
  of operations that declare no `error` type, which §5.2 now forbids.
- **The first state value (F-68).** The owner example puts its state value
  after `start()`, so after its tokens. §8.2 now asks for it before.
- **Replicas and `unavailable` (O-10, O-12).** The runtime declares an
  `unavailable` queryable over every optional operation an active instance
  does not expose. An instance serving only replicated operations then
  intercepts calls to an exclusive operation another instance serves. §5.1
  now forbids it.
- **The split-brain check (O-12).** `ownership::split_brain` compares
  tokens only, so it reports replicas. §6 now exempts them, decided from
  the contract and the holders' descriptors.
- **A typed handler over a template (C-2, O-1).** A typed handler cannot
  name the member a fan-out reply answers for: `Call::member` takes
  `&mut self`, the member it names is not shared between a call's clones,
  and `CallInfo` lends only `&Call`. So every fan-out over a templated
  operation served by `serve_one` or `serve_many` is answered `internal`.
- **The codegen's schema check (C-1).** `zenkey_build::check_schema`
  compares the two spellings as written, so a type whose schemars output
  spells an `Option` one way disagrees with a committed schema that spells
  it the other. It should read a nullable form as its reading, as §7.3 now
  says.

**Changed: rules stated, by section.**
- **The descriptor (F-67, §3.3, §8.1, §8.2).** The first descriptor is
  put, when its queryable is declared in step 3, so before any token; a
  re-mint puts the new instance's the same way.
- **State (§4.2, §4.3).**
  - *Observing S1 (F-69).* An owner that is its own router stamps with the
    router's zid, so the check proves nothing. The tester runs the owner
    as a client of a router with timestamping on, against a control: an
    unstamped put through that router arrives with the router's zid.
  - *The tick (F-66)* is the smallest step the timestamp type takes: one
    NTP64 unit, 2^−32 s, in the reference. Any larger step keeps S7, such
    as zenoh-python's 1 ns.
- **Calling (§5.1).**
  - *A concrete call (F-64, O-9)* MUST set `BestMatching` and `None`.
    `Latest` would hold the reply until the query completes, and keep one
    reply per key, so a split-brain's second execution would vanish.
  - *Timeouts (O-8):* the caller's, else the operation's `timeout_ms`,
    else 10 s, zenoh's default. `timeout_ms` had no stated meaning; it is
    the contract's recommendation.
  - *Retries (O-7)* are opt-in, and follow silence only. An envelope is an
    answer, `busy` included: calling again after it is a new call.
- **Answering (§5.1).**
  - *Every call is answered (O-5):* a handler that ends without replying,
    or without its declared summary, is answered `internal`.
  - *A key that names no member (O-4)* is `invalid_request`: the key is
    malformed. `not_found` is for a well-formed member the owner lacks.
  - *The request (O-13).* One that does not decode as the request type is
    `invalid_request`. The decode is the check the core requires: an owner
    need not evaluate the JSON Schema, and a caller does not depend on it.
  - *The active instance (O-10)* exposes at least one of the interface's
    resources. Replicas: one that serves only replicated operations MUST
    NOT answer `unavailable` on an exclusive operation, and replicas SHOULD
    expose the same replicated operations.
- **Fan-out (§5.1).**
  - *Over a template (O-1, C-2).* A server over the whole template learns
    only what the key expression binds, names the member each reply
    answers for, and replies on its key, which the call must select. A
    caller relies on a reply's key, and not on one reply per member.
  - *Attribution (O-2).* A refusal cannot be attributed in zenoh 1.10.1: a
    `reply_err` carries no key, and the replier's id is unstable. Who sent
    no value is read from presence, refused and silent alike.
  - *Possibly partial (O-3).* A replier, the replies on one key, is
    complete only with exactly one summary. With none it was cut off; with
    two, several instances answered on one key and cannot be told apart.
- **Call metadata (O-6, §5.1).** A JSON object whose `actor` and
  `request_id`, each optional, are strings where present; other members
  ignored; anything else, a `null` member included, is no metadata. A call
  is never refused for it.
- **`app` (F-65, §5.2).** Any operation may refuse with `app`. With no
  `error` type there is no detail; with one, the detail is optional, and a
  raw type's is base64 text in its JSON envelope. A detail that does not
  fit is never sent: the reference sends `internal`.
- **Serving (§6).**
  - *Replicated serving (O-12).* Each replica holds the interface's token,
    so the token check exempts holders of which at most one exposes an
    exclusive resource, decided from the contract and the descriptors.
    Where a tool cannot read them, the holders are undecided.
  - *The grace period (O-11)* is judged from two presence reads, `grace`
    apart: two or more holders in both, not necessarily the same.
- **Start-up (§8.2).**
  - *Exposed (F-70)* is what the instance serves, as its descriptor says:
    an operation by its queryable, a state by its state queryables and
    publisher, a stream by its publisher, an event by its puts. A
    template with no member yet is exposed by the template. Step 2 refuses
    an optional resource neither exposed nor absent as the descriptor
    says, since a descriptor's exposure is compact. The order of steps 1
    and 2 is free.
  - *State values (F-68).* An owner that holds a state value at start
    SHOULD put it before the tokens, so a GET made on presence finds it.
- **Appendix B** gains the reply error's shape, the default query timeout,
  and NTP64's unit.

**Where the Python implementation's guess and the stated rule differ** (it
was right to guess; these are now decided):
- *Exposed (F-70):* zk2py counts a state only once its value exists, and
  refuses a contract with a required templated resource. The rule counts a
  state without its value, and a template without members. zk2py may still
  refuse to start what it does not serve.
- *New static rules:* the nullable reading (C-1), and `$ref`s followed
  inside undecided keywords (X-1), change the classifier zk2py follows.

zk2py's guesses on F-64 (`None` for every call), F-65 (`app` without a
detail), F-66 (1 ns), F-67 (one stamped put after step 3, before the
tokens), F-68 (state values before the tokens) and F-69 (an owner behind a
router) are the stated rules.

**Deliberately not changed:**
- **No existing expectation.** Every fixture of 0.6 keeps its expected
  value; the classifier's fixes change behaviour only on inputs no fixture
  had. `anyof-add-branch` keeps review through the exception above.
- **The envelope gains no member to attribute a refusal (O-2).** A key or
  an id inside it would change `error.proto`, `error.schema.json`, every
  decoder and the fixtures, for an attribution a replier could claim
  falsely. Refusals stay unattributed until zenoh has a stable way.
- **No JSON Schema validation is required at run time (O-13).** Validators
  differ (§7.3 refused `pattern` for that reason); the decode is the check.
- **`oneOf: [S, {"type": "null"}]` has no reading.** It is not what the
  generators write, and a `oneOf` branch added is the one undecided change
  measured: it stays compared as written. A nullable inside an undecided
  keyword is compared as written too, like everything there.
- **No timeout value of the core's own (O-8).** 10 s is zenoh's default,
  named as such; the scenarios' 1 s still binds conformance runs.
- **No fan-out retries are specified (O-7).** The reference makes none,
  and the core neither requires nor forbids one beyond O4.
- **A split-brain "undecided" is not a new finding class.** It is what a
  tool says when it cannot decide, as silence is never a verdict.
- **The live rules have no fixture.** The first descriptor's put, the
  state value's place, S1's observation, the fan-out rules and the
  replicas' exemption are network behaviour: their evidence is the
  scenarios.
- **The owner example (#610)** is still its own router. `state.md §1`
  needs it as a client of R1, like `presence.md §2` step 4 (0.6): the
  runtime's change to make.

| Id | Resolution |
|---|---|
| F-64 | Rule stated (§5.1 O1: a concrete call MUST set `BestMatching` and `None`); scenario `operations.md §1` |
| F-65 | Rule stated (§5.2: `app` for any operation, no detail without an `error` type, a raw detail as base64 text); runtime fix recorded (the `Raw` codec's detail, a detail-less `app`, the owner example); fixtures `errors/json-app-no-detail`, `json-app-raw-detail`, `pb-app-no-detail`, `pb-app-empty-detail`; scenario `operations.md §3` |
| F-66 | Rule stated (§4.3: a tick is one NTP64 unit, any larger step allowed; Appendix B); scenario `state.md §1` step 3 |
| F-67 | Rule stated (§3.3, §8.1, §8.2 step 3: the first descriptor is put, before any token); scenario `presence.md §1` |
| F-68 | SHOULD added (§8.2: a state value held at start is put before the tokens); owner example's fix recorded; scenario `presence.md §1` |
| F-69 | Rule stated (§4.2 "Observing S1": an owner behind a router, with a control); scenario `state.md §1` rewritten |
| F-70 | Rule stated (§8.2: "exposed" defined; step 2 refuses an optional resource neither exposed nor absent); scenarios `presence.md §1`, `§2` step 5 |
| O-1 | Rule stated (§5.1 "Over a template": a template-wide server names the member, on a key the call selected) |
| O-2 | Rule stated (§5.1 "Attribution": refusals unattributed, presence for the rest; Appendix B); scenario `operations.md §2` |
| O-3 | Rule stated (§5.1 "Possibly partial": exactly one summary, two instances on one key indistinguishable); scenario `operations.md §6` |
| O-4 | Rule stated (§5.1: a key that names no member is `invalid_request`); scenario `operations.md §3` step 3 |
| O-5 | Rule stated (§5.1, O3: every call answered, `internal` for a handler that does not reply); scenario `operations.md §3` step 5 |
| O-6 | Rule stated (§5.1 "Call metadata"); scenario `operations.md §7` |
| O-7 | Rule stated (§5.1 "Retries": opt-in, after silence only, `busy` an answer); scenario `operations.md §4` |
| O-8 | Rule stated (§5.1 "The timeout": the caller's, `timeout_ms`, 10 s; Appendix B) |
| O-9 | As F-64 |
| O-10 | Rule stated (§5.1 "The active instance"); MUST NOT added beside replicas; runtime fix recorded (`Ops::start`); scenario `operations.md §3` step 7 |
| O-11 | Rule stated (§6: two presence reads `grace` apart); scenario `operations.md §8` |
| O-12 | Rule stated (§6: holders of which at most one exposes an exclusive resource are no finding, decided from the contract and descriptors); runtime fix recorded (`ownership::split_brain`); scenario `operations.md §8` steps 4–5 |
| O-13 | Rule stated (§5.1 "The request": the decode is the check); scenario `operations.md §3` step 2 |
| C-1 | Rust fixed (the classifier's nullable reading); rule stated (§7.3, §9.8); codegen fix recorded (`check_schema`); fixtures `compat/payload/jsonschema/nullable-type-to-anyof`, `nullable-anyof-to-type`, `nullable-inner-retyped`, `nullable-null-dropped`, `nullable-ref-target-changed`, `nullable-enum-inlined`, `nullable-enum-null-refused` |
| C-2 | Rule stated (§5.1 "Over a template": what a server and a caller rely on); runtime fix recorded (a typed handler names the member); scenario `operations.md §2` |
| X-1 | Found resolving C-1. Rust fixed (`$ref`s followed inside undecided keywords); rule stated (§9.8); fixture `compat/payload/jsonschema/oneof-ref-target-changed` |

## 0.6 — 2026-10-08: the findings against 0.5, and the archive's gaps (#609, #620)

The Python implementation (#609) was rewritten from 0.5, and found 8 more
places where the spec was silent, ambiguous or said two things: F-56 to
F-63. The runtime's state chunk (#620), the first implementation of §4.4,
found 4 gaps of its own: G-1 to G-4. Each is resolved as 0.5's were:
- **the reference's behaviour becomes the rule**, stated where a reader
  looks for it; or,
- **where that behaviour was a bug**, the Rust is fixed, and the corrected
  rule is stated.

A fixture pins every static rule a fixture can check: 4 contract cases,
1 bundle, 6 descriptors, 3 error envelopes and 2 compatibility cases. No
existing expectation changed. The live rules land in their scenarios.

**Changed: the reference was wrong, and is fixed.**
- **One id, one name (F-62, the contradiction).** Two listed JSON Schema
  files whose JCS bytes are equal share an id. The canonical form listed
  that id once, under the later file's name, while a bundle `$ref` names an
  artifact by stem (§9.4). So a contract could lint clean and build a
  bundle whose `$ref` resolved to nothing: `a.json` and `b.json` identical,
  `c.json` referring to `a.json#/$defs/X`, gave a bundle with the stems
  `b` and `c` only. The Rust builder did exactly this.
  - **The rule:** a later listed file with an earlier one's id is E024,
    once, and is not loaded, as a later file with a taken stem already
    was. Ids are compared, not file bytes: reordered members or other
    whitespace are the same document. In a contract with no E024 and no
    E032, every bundle `$ref` resolves (§9.4).
  - **Why this rule.** It is the smallest of the three the finding named
    that removes the contradiction:
    - *Keeping the first name* fixes a `$ref` to the first file and breaks
      one to the later file, so a contract could still lint clean and
      build a dangling `$ref`.
    - *Listing the id under every name* changes the canonical form's
      shape, makes §9.7's retention identity ambiguous (it replaces each id
      by its artifact's one name), and needs new mechanism in every
      builder and classifier.
    - *E024* reuses a code that already means "two listed files collide on
      what a bundle keys by", and its cascade. It changes no other
      contract's canonical form or fingerprint, and no fixture, example or
      test contract has two such files: every contract under
      `examples/zk2/`, `spec/conformance/`, `zenkey/tests/contracts/` and
      `impl/python/interop/` was checked. A contract that wanted both names
      lists the file once, and refers to it by that name.
  - **The reader's half.** The classifier resolved a `$ref` whose file
    part named no artifact in the referencing document instead, silently.
    Such a `$ref`, or one whose pointer resolves to nothing, is now
    `schema_unreadable` (review), like a descriptor set that does not
    decode (§9.8).
- **CBOR integers (F-58).** "An integer outside 64 bits" was read as
  outside the signed range: 2^63 to 2^64−1 were `decode`, although the
  reference's own encoder writes a `u64` detail that way, and JSON
  decoding holds it. An integer now decodes from −2^63 to 2^64−1, and
  only CBOR's −2^64 to −2^63−1 is `decode` (§5.2).
- **A peer archive's pattern (G-3).** Aligning from an owner-side archive
  read the peer's keys through the recorded selector's archive form. A
  wildcard there also matches a slugged verbatim chunk, which the selector
  itself never does, so the read could bring in explicit state the archive
  was never configured to record. A reply now counts only when the
  selector selects its decoded origin (§4.4).

**Changed: rules stated, by section.**
- **No ceiling (G-1, §2.2, §4.4).** §4.4 said `archive.v1` "declares no
  ceiling", while E013 requires a `cardinality` on a templated resource;
  the runtime wrote 4294967295 and called it a placeholder. That value is
  now the convention: a template whose population no contract can fix
  declares 2^32−1, which reads "no ceiling", and a tool never budgets with
  it. The authoring format does not change. The runtime's comment cites
  the rule.
- **Retention (F-56, §2.6, E026).** The seconds are an unsigned 64-bit
  integer: above 2^64−1 is E026, and 2^63 s, which a signed reading would
  refuse, reaches the canonical form and is E028 there.
- **An unbound required role, observed (F-61, §3.2).** The finding's
  runner watched an owner that was its own router, so the refusal was
  judged on silence. A tester watches through a router that outlives the
  owner, against a control run that does show the token;
  `presence.md §2` says how.
- **The descriptor (§3.3).**
  - *Repeats (F-57).* A repeated capability or profile is one D008 or D010
    for the whole list, however many values repeat; a malformed value
    counts at each occurrence as well, so `["A", "A"]` gives three.
  - *Integers (F-63).* `format` is a bound in a descriptor too, and there
    `uint64` is 0 to 2^64−1: a `cardinality` of 2^64 is D000, and 2^64−1
    above the contract's bound is D007.
- **Archives (§4.4).**
  - *The pattern (G-3).* An origin selector's archive form drops `zk2`,
    keeps `*` and `**`, and slugs every other chunk; a `$*` chunk has no
    form.
  - *Reachable* is judged by the owner's instance token, and a peer
    archive's token appearing triggers alignment too. Peers are read in
    turn, the first with a reply that counts wins.
  - *Retrying (G-2), a SHOULD.* An owner's token can arrive before the
    route to its state queryable, and a read then returns empty. An
    archive repeats an alignment that left keys unconfirmed, a bounded
    number of times. The reference makes 5 attempts. `state.md §5` step 2
    now refuses every read, retries included.
  - *Confirmation (G-4).* A value from the owner is confirmed. One from a
    peer keeps the peer's flag, unconfirmed when the attachment is absent
    or unreadable. A peer's `reply_del` is positive evidence. A reply at
    the held timestamp can confirm a key, never unconfirm it.
- **The error envelope (F-58, §5.2):** the CBOR integer range above.
- **When the wait for presence starts (F-59, §8.1).** From the later of the
  tool's session connecting and the owner's launch. Where the owner is the
  tool's router, that is the connection.
- **Bundles (F-62, §9.4, §9.6).** A bundle whose `$ref` names no artifact,
  such as 0.5's rule built for two identical files, still verifies:
  verification checks hashes, not content.
- **Compatibility (F-60, §9.8).** `oneof_branch_added` needs a `oneOf` on
  both sides. Adding the keyword where there was none, or removing it, is
  `undecided_changed` (review): an absent `oneOf` is no constraint, not
  zero branches.

**Where the Python implementation's guess and the stated rule differ** (it
was right to guess; these are now decided): repeats are one D008 or D010
for the list, not one per repeated value (F-57); a `oneOf` added where
there was none is review, not breaking (F-60); two listed files with one
id are E024, where zk2py followed 0.5 and built the dangling `$ref`
(F-62); and a refusal is watched through a router of the runner's own,
with a control (F-61). zk2py's guesses on F-56, F-58, F-59 and F-63 are
the stated rules.

**Deliberately not changed:**
- **No existing expectation.** Every fixture of 0.5 keeps its expected
  value; the fixes change behaviour only on inputs no fixture had.
- **The verifier does not resolve `$ref`s.** A bundle 0.5 built for two
  identical files stays verifiable, so a history holding one still passes
  §9.7's check. The lint keeps new ones from being built, and the
  classifier reads an old one's `$ref` as unreadable.
- **`descriptor.schema.json` gets no `maximum` for `uint64`.** 2^64−1 has
  no exact double, so a validator that reads numbers as doubles could not
  hold the bound; `format` is the bound, as §3.3 says. (0.5 added `maximum`
  to `contract.schema.json`'s `uint32` fields, which every reader holds.)
- **JSON envelope numbers** get no range rule. Only CBOR, whose integer
  type has a range of its own, needed one.
- **Removing a `oneOf`** loosens what a writer may send, as a branch added
  does, but it was not measured, so it stays review: the asymmetry
  paragraph leaves unmeasured changes to a human.
- **The authoring format** (G-1): the convention is a value, not a new
  spelling. `archive.v1` stays a profile (#613); the runtime's minimal
  contract keeps its value and its fingerprint.
- **No timeout values in the core** (F-59). The starting instant binds the
  scenarios, like their 1 s.
- **The retry's figures** (G-2). The core asks for a bound, and states the
  reference's 5 attempts as informative.
- **The owner example (#610)** is still its own router. A runner following
  `presence.md §2` step 4 needs it as a client of R1, which is the
  runtime's change to make, not the spec's.

| Id | Resolution |
|---|---|
| F-56 | Rule stated (§2.6, E026: unsigned); fixtures `e026-retention-range`, `e028-retention` |
| F-57 | Rule stated (§3.3: once for the list, per malformed occurrence); fixtures `d008-two-repeated-values`, `d008-malformed-twice`, `d010-two-repeated-values`, `d010-malformed-twice` |
| F-58 | Rust fixed (CBOR integers −2^63 to 2^64−1); rule stated (§5.2); 3 `errors/` cases |
| F-59 | Rule stated (§8.1: the later of the connection and the launch) |
| F-60 | Rule stated (§9.8: review); fixtures `oneof-keyword-added`, `oneof-keyword-removed` |
| F-61 | Rule stated (§3.2: watch through a router that stays up, with a control); scenario `presence.md §2` rewritten |
| F-62 | Rust fixed (E024 for a taken id; the classifier's dangling `$ref` is `schema_unreadable`); rules stated (§9.4, §9.6, §9.8); fixtures `contracts/e024-identical`, `bundles/ref-names-no-artifact` |
| F-63 | Rule stated (§3.3: `uint64` is 0 to 2^64−1, D000 outside); fixtures `d000-cardinality-range`, `d007-cardinality-max` |
| G-1 | Rule stated (§2.2: 2^32−1 is "no ceiling"; §4.4); fixture `ok-cardinality-no-ceiling`; `archive.rs`'s comment cites it |
| G-2 | SHOULD added (§4.4: bounded retry); scenario `state.md §5` |
| G-3 | Rust fixed (a peer's reply counts only for a selected origin); rule stated (§4.4: the archive form) |
| G-4 | Rule stated (§4.4: a peer's `confirmed` kept, its `reply_del` positive evidence); scenario `state.md §5` step 3 |

## 0.5 — 2026-10-08: the second implementation's findings (#609, #607)

The Python implementation (#609) was written from `spec/` alone, and passes
every fixture. It recorded 55 places where the spec was silent, ambiguous
or said two things: F-01 to F-45 from its static half, and F-46 to F-55
from its live half, which runs the reference runtime (#610) as a black box.
Each is resolved here, in one of two ways:
- **the reference's behaviour becomes the rule**, stated where a reader
  looks for it; or,
- **where that behaviour was a bug**, `zenkey-model` is fixed, and the
  corrected rule is stated.

A fixture pins every static rule a fixture can check: 21 contract cases, 4
descriptors, 1 set, 3 histories, 6 bundles, 11 error envelopes, 17
compatibility cases, 2 keys and 1 template case. No existing expectation
changed. The live rules land in their scenarios, with what was measured.

**Changed: the live half (F-46 to F-55, §3.2, §3.3, §8.1, §8.4).** The
runtime already behaved this way, except where noted; the spec now says so.
- **Retrieval needs consolidation `None` (F-50), a MUST.** Measured: with
  zenoh's default, a holder's corrupt reply after 2 s was delivered alone,
  and the valid reply, sent at once, never arrived. A caller following
  §8.4's steps would have reported the contract unavailable. The runtime
  already set `None`; §8.4 now requires it on both attempts.
- **The presence rule binds the subscribers (F-47).** Measured: a bounded
  liveliness subscriber nobody drains starves even a callback GET on the
  same session, which ended at its timeout with 257 of 2,002 tokens,
  silently. Every liveliness subscriber on such a session MUST be
  callback-driven or drained, and a GET that ends at its timeout SHOULD be
  read as possibly incomplete. `presence.md §4` said the default GET handler
  alone hangs; zenoh-python showed it does not, so the scenario now states
  what was measured, beside spike S2's Rust result.
- **The replies (F-46, F-48).** A descriptor and a bundle are each answered
  with one reply, `application/json`; the descriptor's has no timestamp and
  no attachment. A bundle reader MUST NOT depend on the encoding: the hash
  is the check. A descriptor GET uses consolidation `None` too.
- **Timeouts (F-49)** are the caller's choices, named in §8.1; the
  scenarios' 1 s is the conformance default.
- **A holder on the caller's own session (F-51)** answers `BestMatching`
  too, so a caller can get two replies; it assumes nothing about the count.
- **An unbound required role (F-52):** the owner MUST NOT start (R1, §8.2
  step 2), as the runtime already refused. It refuses a missing
  configuration, not a missing provider, so R5 and R7 stand.
- **`profiles` (F-53)** is the union of the implemented contracts' `uses`.
  The descriptor checker does not check it.
- **Members (F-54)** exist from the owner's first declaration of one: no
  member, no member token.
- **An unbound optional role (F-55)** is listed with `"bindings": []`, so
  the graph keeps every edge a contract declares.

**Changed: the reference was wrong, and is fixed.**
- **Floats (F-19).** `nan` and `inf` in an annotation became `null` without a
  word, so a contract changed meaning and linted clean. A float that JCS
  writes as an integer beyond ±(2^53−1), such as `1e16`, linted clean and
  built a bundle no implementation verifies (step 6). §9.5 now defines the
  **canonical domain**: an integer within ±(2^53−1); a float finite and,
  when integral and below 10^21, within the same range. Outside it is E028,
  in the canonical form and in a JSON Schema artifact.
- **TOML integers (F-16).** The reference reader accepted 2^63 to 2^64−1,
  which TOML 1.0 does not have, and reported E028 where the value reached
  the canonical form. An integer outside the 64-bit signed range is now
  E000 (§9.1), as in a 1.0 reader.
- **W103 beside E023 (F-23).** W103 fired when the type did not resolve,
  with a message calling it protobuf or raw. It is now judged on resolved
  types only (cascade 6).
- **E035 under E036 (F-38).** E035 checked the *last* declaration of a
  duplicated interface, an accident of a map insert. It checks the first,
  and E036 falls on each later one.
- **D010 (F-06).** A profile listed twice was silent, unlike a capability
  (D008) or an interface (D003). It is D010.
- **Schema positions (F-09, F-10).** `$ref`s were collected anywhere in a
  JSON Schema document, data included, so `"const": {"$ref": "x"}` was an
  E032. §7.3 defines schema positions once; E037, E032 and the classifier
  read only them.
- **What the classifier could not see (F-32, F-33).** Three changes were
  invisible, so a breaking change could pass as compatible:
  - keywords beside a `$ref` were dropped; they are now added to the
    target and compared;
  - `true` → `false` in a property compared as two empty schemas; any
    change to or from a boolean schema is now review;
  - a `required` name with no property was never compared; adding or
    removing one is now breaking.

  Annotations were also stripped by key at any depth inside `oneOf`, so a
  property named `title` there was invisible; they are now stripped at
  schema positions only.
- **The error envelope (F-08).** A CBOR byte string in a `detail` decoded
  as an array of numbers; it is now base64 text, the JSON form of bytes
  (§7.2). Bytes after the CBOR item were ignored; they are `decode`.
- **Bundle entries (F-25).** A schema or extra entry could carry members no
  hash covers, which contradicted "the whole bundle is verified". An entry
  now holds only its members (`shape`). A JSON document holding a number
  outside the canonical domain matches no id, by rule rather than by
  whatever a JCS library writes for it.
- **`contract.schema.json` (F-17)** carries `maximum` on its `uint32`
  fields, so a standard validator bounds them.

**Changed: rules stated, by section.**
- **Grammar (§1, §2.2).** A key is parsed lexically, so a resource chunk is
  any plain chunk, `x-eth0` included (F-02); a ULID chunk's first character
  has no bound (F-01). Parameter types play no part in matching (F-03). A
  tie in rank, which E021 makes impossible in a contract, leaves the first
  listed template the winner.
- **The descriptor (§3.3).** A D-code table, with severity and counting
  (F-04), and the cascades and scope that only fixtures showed (F-05): an
  invalid `iface` is skipped and no longer counts for `declared_by`; a
  malformed fingerprint stops the entry before D004; an interface none of
  the given contracts declares is checked for syntax only. What the checker
  deliberately does not check is listed: `cause` against gates, R3's
  completeness, `declared_by` against the contract's `[requires]`, `params`
  values, `minor` (F-06). A lowered bound of 0 is D007 (F-06). The example
  held `imu` yet gave "no IMU" as a cause; it now shows a held capability
  with a `config` cause (F-07).
- **The error envelope (§5.2).** The decoding edges (F-08): exact encoding
  strings, member types, CBOR tags, `undefined` and indefinite lengths, a
  protobuf wire-type mismatch, a missing protobuf `message`, an empty
  protobuf `cause`.
- **JSON Schema (§7.3, §9.4).** A refused keyword's content is not walked,
  and `definitions` is refused (F-09). A `$ref`'s scheme is a `:` in its
  file part; its fragment is a JSON Pointer used as written, never
  percent-decoded, and `#anchor` is not one (F-10). In a bundle, a file part
  names an artifact by stem (F-11). A duplicate member is E029 (F-12). E024
  falls once per later file with a taken stem, which is not loaded, so
  `json:stem#Name` looks only in the first (F-13).
- **Protobuf (§9.4).** The well-known types compile from protoc 3.21.12's
  `include/` sources, which fix their ids; a listed file's own import of
  them resolves through the import roots first (F-14). A listed file is
  named by the first import root containing it, E029 under none; a nested
  message is a message, an enum is E023; listed files shadow the well-known
  types; every field carries its `json_name`, as protoc writes it (F-15).
- **Authoring and lints (§9.1–§9.3).** TOML datetimes of all four kinds, at
  any depth (F-18). E020 counts a key's value and its name apart, so one key
  can give two, and requirement annotations are checked (F-20). A profile
  without an interim table raises no W105 (F-21). E018 and E033 judge
  resolved values (F-22). A retention may have leading zeros, a rate may
  not; `gate = []` is no gate; `history = false` on an event is E019
  (F-23). "The first" of E021 and E022 is the template that sorts first,
  since TOML gives a table's keys no order; codes do not depend on it, and
  E021 does not take a resource out of W101 (F-39). The 1.1 list is closed,
  and "MAY accept later TOML" now reads "MAY be a parser of later TOML,
  provided it reports these as E000" (F-41). A set check runs only when
  every member loads (F-38). Integers are bounded by the schema's `format`,
  `uint64` to TOML's 2^63−1 (F-17).
- **Bundles (§9.6).** Base64 is RFC 4648 §4, padded, strict (F-24). A
  non-object entry or one without `kind` is `schema_kind`; an extra's
  `media_type` is informative; one id listed twice is one artifact (F-25).
  A `views.document` value is one id string; where a builder finds the
  documents is `views.v1`'s (#613), and a bundle file always writes all
  three members (F-26).
- **History and retention (§9.7).** The check's order: entries in bytewise
  order, stray root entries `directory` and not looked into, then per file
  `io`, the bundle tag, `interface`, `jcs`, the last two both reported
  (F-27). Retention identity is defined apart from the classifier's
  classes, with protoc's default `json_name` (F-28). A history root is
  configured, and a contract's history found by interface id; the examples
  share `examples/zk2/.history` (F-45).
- **Compatibility (§9.8).** "Both directions" means the writer and reader
  roles, never a swap (F-29). Resources pair by kind and template, so a
  toggled `explicit` pairs and a changed kind does not: the
  `kind_changed`/`token_changed` row, which could never fire, now reads
  `resource_removed` (F-30, F-40). An artifact no type reaches is not
  compared (F-31). `oneOf`/`anyOf`/`prefixItems` compare as written, in
  order, annotations dropped; "a branch added" is more branches; the
  asymmetry is deliberate (F-33). The renumber bullet no longer calls a
  renumber a deletion (F-34). Protobuf: messages are compared by structure
  from the named type, oneofs by name, enums by number and closed when the
  candidate's file is proto2; defaults, options other than `json_name`, and
  reserved names are not compared; warnings are names, deduplicated (F-35).
  Explicit presence is protobuf's definition, and a oneof move reports
  `oneof_changed` alone (F-43). `compat/README.md` says where the wrapper
  sits and when a case is `invalid`; only a candidate can be (F-44). F-36
  was resolved by 0.3, and needs nothing more.
- **Documents (F-37, F-42).** Fixture documents cite spec sections instead
  of Rust symbols and design documents. 0.3's case count is corrected (23
  new, not 11), and `compat/expect.json` no longer says the runner only
  checks that inputs load.

**Where the Python implementation's guess and the stated rule differ** (it
was right to guess; these are now decided): CBOR bytes read as base64, not
`bytes_hex` (F-08); a refused keyword's content is not walked (F-09); a
pointer is not percent-decoded (F-10); one key can give two E020 (F-20);
`gate = []` is no gate, not E016 (F-23); a non-object schema entry is
`schema_kind`, unknown entry members are `shape`, an extra's `media_type`
must be a string (F-25); a `views.document` list references nothing
(F-26); `interface` and `jcs` are both reported for one file (F-27); `$ref`
siblings are compared rather than flagged review (F-32); `oneOf` branches
compare in order, so a reordering is review (F-33); a proto2 default change
is compatible, and an enum is closed by the candidate's syntax only (F-35);
template order, not document order (F-39); a oneof move reports one rule
(F-43).

**Deliberately not changed:**
- **No existing expectation.** Every fixture of 0.4 keeps its expected
  value; the fixes changed behaviour only on inputs no fixture had.
- **R3 in the descriptor checker.** That a descriptor lists every
  requirement its contracts declare stays a scenario rule
  (`bindings.md §3`): the checker sees the contracts it is given, not the
  deployment's.
- **Extras.** The core still defines no source for extra documents. That is
  `views.v1`'s (#613), and the core says so instead of guessing.
- **Rule names stay informative.** `expect.json` pins classes and warning
  names. The reference keeps `kind_changed` and `token_changed` as
  unreachable branches; the table no longer lists them.
- **buf.** The protobuf cases added here were not measured with buf;
  `compat/README.md` says so.
- **Protobuf portability.** Canonical bytes stay portable only across
  encoders that match protoc 3.21.12. Naming that release's well-known
  sources pins their ids, and adds no new mechanism.
- **The live rules have no fixture.** They are network behaviour: their
  evidence is the scenarios, with zenoh-python's measurements beside the
  spike's Rust ones. Spike S2's hang stays recorded as measured.
- **No timeout values in the core.** Naming one would bind every deployment
  to a loopback figure; the 1 s of the scenarios binds conformance runs
  only.
- **The descriptor GET's target** is left to the caller: one instance's
  queryable answers it, under `BestMatching` or `All` alike.
- **The runtime (#610)** needed no change: it already set consolidation
  `None`, answered with `application/json`, refused an unbound required
  role, listed unbound roles, and holds no liveliness subscriber.

| Id | Resolution |
|---|---|
| F-01 | Rule stated (§1.2): no first-character bound; fixture `keys.json` |
| F-02 | Rule stated (§1.1): parsing is lexical, a resource chunk is any plain chunk; fixture `keys.json` |
| F-03 | Rule stated (§2.2): types play no part in matching; tie rule pinned in `templates.json` |
| F-04 | Rule stated: §3.3's D-code table |
| F-05 | Rule stated: §3.3's cascades and scope; `ok-unknown-revision` renamed `ok-unknown-interface` |
| F-06 | Rust fixed (D010 twice); rest stated, with what is not checked; fixtures `d007-zero`, `d010-twice`, `ok-unchecked` |
| F-07 | Example fixed (§3.3) |
| F-08 | Rust fixed (CBOR bytes, trailing bytes); edges stated (§5.2); 11 `errors/` cases |
| F-09 | Rust fixed (one schema-position walker); defined (§7.3); fixture `e037-nested` |
| F-10 | Rule stated (§9.4); fixture `e032-fragment` |
| F-11 | Rule stated (§9.4: by stem in a bundle); fixture `compat/contract/cross-file-ref-retyped` |
| F-12 | Rule stated (E029); fixture `e029-duplicate-member` |
| F-13 | Rule stated (E024 counting, first file wins); fixture `e024-stem` |
| F-14 | Rule stated (§9.4: protoc 3.21.12's `include/` sources) |
| F-15 | Rule stated (§9.4); fixtures `ok-protobuf-nested`, `e023-enum`, `e029-outside-include` |
| F-16 | Rust fixed (E000 beyond 64 bits); fixture `e000-integer-range` |
| F-17 | Rust fixed (`maximum` in the schema); `format` stated as a bound (§9.1); fixture `d000-minor-float` |
| F-18 | Rule stated (§9.1: four kinds, any depth); fixture `e020-twice` |
| F-19 | Rust fixed (E028 for `nan`/`inf` and integral floats); canonical domain stated (§9.5); fixtures `e028-float`, `e028-non-finite`, `e028-schema-float`, `ok-float-integral` |
| F-20 | Rule stated (E020 counting, requirements); fixtures `e020-twice`, `e020-requirement` |
| F-21 | Rule stated (§10, W105); fixture `ok-profile-without-vocabulary` |
| F-22 | Rule stated (resolved values); fixture `ok-resolved-operation` |
| F-23 | Rust fixed (W103); rest stated; fixtures `e023-encoding`, `ok-gate-empty`, `e019-history-false`, `ok-retention-leading-zero` |
| F-24 | Rule stated (§9.6); fixture `base64-unpadded` |
| F-25 | Rust fixed (entry members, unportable numbers); rest stated; fixtures `schema-entry-member`, `schema-entry-not-object`, `schema-data-big-integer`, `extra-entry-member` |
| F-26 | Left to a profile (`views.v1`, #613), with what a core implementation does meanwhile; value shape and `"extras": {}` stated; fixtures `no-extras` (bundle and history) |
| F-27 | Rule stated (§9.7's order); fixtures `wrong-directory-not-jcs`, `no-extras`, `stray-entries` |
| F-28 | Rule stated (§9.7 identity, default `json_name`) |
| F-29 | Wording fixed (§9.8: roles, not swaps) |
| F-30 | Superseded by F-40 |
| F-31 | Rule stated (unreferenced artifact); fixture `compat/contract/unreferenced-artifact-changed` |
| F-32 | Rust fixed (`$ref` siblings, booleans, required names); fixtures `ref-sibling-changed`, `boolean-property`, `required-without-property` |
| F-33 | Rule stated, Rust fixed (strip at schema positions); fixtures `oneof-reordered`, `oneof-annotation-only`, `oneof-property-named-title`, `anyof-add-branch` |
| F-34 | Wording fixed (§9.8 renumber bullet) |
| F-35 | Rules stated (§9.8); fixtures `move-between-oneofs`, `proto2-default-changed`, `packed-option`, `message-renamed`, `enum-value-renumbered`, `nested-type-added` |
| F-36 | Resolved by 0.3; nothing more |
| F-37 | Pointers replaced by spec sections (`conformance/README.md`, every `description`) |
| F-38 | Rust fixed (E035 against the first); set loading stated; fixture `sets/e036-first-declaration` |
| F-39 | Rule stated (template order); fixture `e021-shape-types` |
| F-40 | Wording fixed (§9.8 pairing); fixture `compat/contract/kind-changed` |
| F-41 | Wording fixed (§9.1: closed list, "a parser of later TOML") |
| F-42 | Fixed (0.3's count; `compat/expect.json`'s description) |
| F-43 | Rule stated (§9.8 presence); fixture `proto2-to-proto3` |
| F-44 | Rule stated (`compat/README.md`: wrapper location, `invalid`) |
| F-45 | Rule stated (§9.7 history root); `examples/zk2/README.md` states the examples' requirement |
| F-46 | Rule stated (§3.3 "The GET": one reply, `application/json`, no timestamp, consolidation `None`); scenario `presence.md §2` |
| F-47 | MUST extended to the session's liveliness subscribers, SHOULD on a timed-out GET (§8.1); scenario `presence.md §4` corrected to the measurement |
| F-48 | Rule stated (§8.4: one reply, `application/json`; a caller MUST NOT depend on it); scenario `retrieval.md §1` |
| F-49 | Rule stated (§8.1: timeouts are the caller's; 1 s in the scenarios) |
| F-50 | MUST added (§8.4: consolidation `None`, both attempts); scenario `retrieval.md §2` with the measurement |
| F-51 | Rule stated (§8.4 step 1: a caller's own holder answers too); scenario `retrieval.md §1` |
| F-52 | MUST stated (§3.2: an unbound required role, no start); scenario `presence.md §2` step 4 |
| F-53 | Rule stated (§3.3 `profiles`); not checked by the descriptor checker; scenario `presence.md §2` |
| F-54 | Rule stated (§8.1 members); scenario `presence.md §3` step 3 |
| F-55 | Rule stated (§3.2, §3.3 `requires`); scenario `bindings.md §3` |

## 0.4 — 2026-10-08: TOML 1.0, enforced (#607)

**Changed:** §9.1's "a contract MUST NOT need TOML 1.1" gains its lint.
Syntax that only TOML 1.1 has is E000, which stops the load:
- a newline, a comment or a trailing comma inside an inline table;
- the `\e` and `\xHH` escapes;
- a time without seconds.

The reference reader (the `toml` crate, which reads 1.1) now finds these
on `toml_parser`'s event stream. Before, a 1.1-only contract loaded in
Rust and failed in a 1.0 reader such as Python's `tomllib`. Five fixtures
(`contracts/e000-toml11-*`) cover one construct each, and `tomllib`
refuses all five. §9.2's E000 row now reads "not TOML 1.0".

**Deliberately not changed:**
- A reader MAY still accept later TOML. The rule binds what a contract
  needs, not what a reader parses.
- Bare keys outside ASCII are not on the list: TOML 1.1.0 did not adopt
  them, and both readers refuse them already.
- In `e000-toml11-time`, the time stops the load as E000 before E020 is
  ever reached. E020 would have refused it anyway, as an annotation
  datetime.

## 0.3 — 2026-10-08: the classifier's rule set (#618)

**Changed:** §9.8 states every rule the classifier applies, each with its
name, in six tables: interface, resources, delivery, operations, roles and
types.
- **Rows that were missing,** chiefly the reverse directions:
  - required → optional, breaking;
  - `idempotent` false → true, review;
  - `fanout` forbidden → allowed, `replies` many → one, a role removed: all
    compatible;
  - `congestion` either way, review;
  - an optional resource added, compatible; a required one, breaking;
  - `deprecated` added, compatible.
- **Protobuf rules added:**
  - a reserved number reused, breaking;
  - explicit presence toggled, review;
  - proto2 `required`: added, deleted or toggled, all breaking.
- **JSON Schema rules added:**
  - `const` changed, and a required property removed: breaking;
  - `additionalProperties` or `items` gaining or losing a schema: review;
  - toggling them between absent, `true` and `false`, and reordering
    `enum` values: compatible.
- **`compat/` is evaluated.** 70 cases, 23 of them new since 0.2: 12
  contract, 5 JSON Schema and 6 protobuf. (This entry said "11 of them
  new" until 0.5 corrected it.) §0's caveat on `[F: compat/]` is removed.
- **`compat/README.md`** gives the case layout and the departures from
  `buf breaking --use WIRE`, measured in both orders.

**Deliberately not changed:**
- No existing case changed its class.
- Rule names stay informative: `expect.json` pins classes and warning
  names, so a second implementation is free to name its findings.

## 0.2 — 2026-10-08: U23 settled

**Changed:** §8.5's attachment rule gains a second shape.
- **The shape:** a far router in a `gateway.south` region of the near
  router, with the `@zk` deny. The near router keeps its own clients and
  peers south.
- **Measured** (spike S3, U23 addendum): 506 B and no denied key string for
  200 tokens, against 11.3 KB and all 201 router to router. Data crosses.
- **Also updated:** §12's faces row, constrained.md §6, and §13's U23 row.

**Deliberately not changed:**
- The client attachment stays a valid shape.
- A router-to-router link with a deny is still not recommended: the deny
  hides the declarations, but they cross anyway.

## 0.1 — accepted 2026-10-08 (#606)

The first accepted version, after two independent review passes (PR #639).
The second pass reproduced every non-protobuf canonical fixture byte for
byte from §9 alone.

**Decided at acceptance, and folded into 0.1:**
- **U22:** a deployment-configured tokenless set. Interfaces every service
  implements carry no interface token, and the descriptor records it with
  `"token": false` (§8.1, §3.3).
  - *Deliberately not done:* a contract-level flag, which would have
    changed every canonical form and fingerprint.

**Recorded as open:**
- **U23,** to be measured (`gateway.south`) before `link.v1`: settled in 0.2.
- **A TOML 1.1 lint:** done in 0.4.
- **Extras in the reference bundle builder.**
- **`compat/`,** evaluated once the classifier (#618) lands: done in 0.3.
