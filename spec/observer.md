# zk2 observer specification

| | |
|---|---|
| **Text version** | 0.10 ([`observer-CHANGELOG.md`](observer-CHANGELOG.md)) |
| **Status** | draft |
| **Written against** | core 0.68 ([`core.md`](core.md)) |
| **Evidence** | the core's: [`conformance/`](conformance/), [`scenarios/`](scenarios/), and the profiles' under [`profiles/`](profiles/) |

This text states what a **reader** of a zk2 bus does, and what it may
conclude from what it read. It states how a reader reads presence, state
and the admin space, when a read is complete, what a window over a stream
or an event shows, and what a tool tells a person. [`core.md`](core.md)
states what a participant puts on the wire and how an owner serves; this
text states what is read off it. It is normative.

Most of it is the reader's discipline that the core held, moved here
with no rule changed. That is core §8.1's "Reading presence", core §2.7's
readings and conclusions, core §4.2's seed and admin-space checks, and
the tool rules that sat beside owners' rules elsewhere. Appendix A lists
each block and its old home. The rest is
[`docs/zk2/tooling-guide.md`](../docs/zk2/tooling-guide.md)'s judgement
shape, its three states and those of its observer obligations that cite
evidence, promoted to normative text (§1, §2, §8). The guide keeps what
did not move: how the reference tools spell these rules and test
themselves, and the obligations that stayed for want of evidence.

---

## 0. Conventions

**Who it binds.** Every rule names the role it binds, as the core's do
(core §0). This text is normative for the **tool** role. It is
normative too for any other participant where it reads presence, state
or the admin space and concludes from what it read:
- a consumer waiting on presence (core R5) or seeding a state;
- a caller attributing silence (core O5);
- an owner that follows another service's presence, as a `desired.v1`
  target or a `redundancy.v1` candidate does.

A **reader** is any of them, reading. Each rule moved here binds exactly
whom it bound in the core.

**Its classes.** Conformance is claimed by role (core §12). This text's
rules for a tool belong to the observer class, with the core's. Its rules
for a reader belong to the class of the role the reader plays.
[`RULES.md`](RULES.md) lists both texts' rules stated with a keyword, by
role. This text states many without one, in §1, §2, §5 and §8. A class
holds those too.

**Keywords, roles and evidence** are the core's (core §0). `[F: path]` is
a fixture under [`conformance/`](conformance/), and `[Sc: file §n]` a
scenario under [`scenarios/`](scenarios/), as in the core. A profile's
fixture or scenario is named by its path under `profiles/`. Each MUST
cites what checks it.

**References.**
- A bare section number, `§3.4`, is this text's. `core §8.1` is the
  core's, and a profile's is written with its name, `redundancy.v1 §2.5`.
- A list after `core` is the core's throughout: `core §4.2, §5.1` names
  two of its sections. A semicolon ends it, so `(core R7; §7.1)` names
  the core's R7 and this text's §7.1.
- The rule names S1–S7, O1–O7 and R1–R7 are the core's (core §4.2, §5.1,
  §3.2), and so is P3, one writer per key (core §6).
- A rule's history, the amendment that made it and the finding that
  caused it, is in [`observer-CHANGELOG.md`](observer-CHANGELOG.md) and the
  core's [`CHANGELOG.md`](CHANGELOG.md). Appendix A lists each block that
  came from the core.
- Zenoh's behaviour that a rule rests on is core Appendix B's.

**What it does not state.** What a participant puts on the wire, an
owner's rules, the grants a deployment compiles (core §11) and the facts
of zenoh 1.10.1 (core Appendix B) stay in the core. Where a rule here
needs one of them, it cites it.

---

## 1. The judgement shape

Every verdict a tool presents fits one shape. That includes a badge, a
table cell, a report field and an exit code. A verdict reaches a person
through three media, and a tool keeps its claims straight in all three:
- a **table cell**, the aligned rendering a person scans;
- a **report field**, the serialized shape a script branches on;
- a **note**, a statement *about* a report: coverage, silence, bounds,
  caveats.

```text
Established(yes) | Established(no)
Unestablished(NotAsked) | Unestablished(Unobservable { reason })
```

- **Established:** the question was put to the bus, and the bus answered.
  Both poles are answers, and each claims evidence.
- **NotAsked:** nobody put the question. No contract was retrieved, no
  window ran, the flag was not passed. The tool has learned nothing, and
  renders no answer, never the negative one.
  - A profile can also spell not this profile's as NotAsked (below).
    There the question was put, and its premise does not hold. What
    NotAsked's bullet says above (nobody put the question, the tool has
    learned nothing, and it renders no answer) does not hold of that
    spelling. Asking again does not cure it.
- **Unobservable:** the question was put, and the observation could not be
  had. The session did not open, the bundle did not verify, the bytes did
  not decode. The `reason` is part of the verdict: it names what stood in
  the way.

`[F: budget/population.json, budget/rate.json]` `[Sc: presence.md §6]`

- **The two Unestablished kinds stay apart in every medium.** They call
  for different actions: NotAsked is cured by asking, Unobservable by
  fixing the named impediment. `[F: budget/population.json, budget/rate.json]`
- **Not asked is not answered no.** A tool distinguishes a question it has
  not put from one answered negatively, in each of the three media.
  `[F: budget/population.json]` `[Sc: profiles/desired/scenarios.md §9]`
- **Not this profile's.** A profile's question can carry a premise of its
  own. Where the premise does not hold, the profile answers **not this
  profile's** (its §5, [`profiles/README.md`](profiles/README.md)). The
  subject is then, for example, a role that carries none of its policy
  (`arbitration.v1`), an interface it does not govern (`redundancy.v1`) or
  a resource of a kind it does not read (`timing.v1`).
  That answer is Unestablished, and never Established(no), since the
  subject is not one the question is about. A profile spells it one of
  two ways, and its fixtures say which:
  - as a token of its own, `not_this_profiles`, kept apart from NotAsked
    and from Unobservable in every medium: `timing.v1`, `arbitration.v1`
    and `redundancy.v1`;
  - as NotAsked with a reason that names the failed premise:
    `freshness.v1` (`kind`, `deleted`), `telemetry.v1` (a reason
    `not_this_profiles`) and `desired.v1` (`not_a_target`).
    **That NotAsked keeps its reason in every medium.** A table cell, a
    report field and a note each say not this profile's, or name the
    reason, and never show a bare not asked. A bare not asked reads as
    cured by asking, and here asking again changes nothing.
    `[F: profiles/freshness/conformance/judgements.json, profiles/telemetry/conformance/samples.json, profiles/desired/conformance/conclusions.json]`

  Either way it is never folded into Unobservable, nor into no.
  `[F: profiles/timing/conformance/conclusions.json, profiles/arbitration/conformance/conclusions.json, profiles/redundancy/conformance/conclusions.json, profiles/freshness/conformance/judgements.json, profiles/telemetry/conformance/samples.json, profiles/desired/conformance/conclusions.json]`
- **An empty scope is Unobservable.** When nothing in scope was observed
  (no token, no reply, no sample), the judgement is unobservable, never
  clean. A monitor pointed at the wrong namespace must not be green.
  `[F: budget/population.json, budget/rate.json]`

How a tool spells each state in each medium, the polarity of a
vocabulary and the exit codes a judging tool projects the shape onto are
guidance ([`tooling-guide.md`](../docs/zk2/tooling-guide.md) §1, §3).

## 2. Silence is never a verdict

**The absence of a signal is evidence about nothing in particular** (core
O5). A tool that heard nothing has established nothing (not absence, not
health, not death) unless its procedure made the silence *attributable*.
That needs three things:
- a bounded window that was provably open;
- a named party expected to speak within it;
- an independent check that speaking was possible.

Unattributable silence is Unestablished, never Established(no).
`[Sc: operations.md §5]` `[Sc: presence.md §6]` `[F: budget/population.json]`

Its instances, each normative where it lives:
- **The reply set** (core O5). An empty reply set is not "no such
  operation". A caller attributes silence through presence: the service
  was present and silent, or absent. Envelopes in a fan-out carry no key,
  so they are reported unattributed (core §5.1).
- **Presence** (§3). A read that ended at its timeout, or at its
  session's interests timeout, is possibly incomplete (§3.3); a refused
  read is complete and empty (§3.4); a read through a session cut off is
  not complete (§3.5); absence is established only by a complete read
  (§3.6).
- **State** (core S4–S6; §4). Silence from the owner's GET is not "no
  value". An archive's answer is last-known, never current (core S6).
- **Contracts** (core §8.4, §9.6). A bundle that no holder served, or that
  failed verification, is "contract unavailable", not "no contract".
- **A window** (§5.2). What a window did not hear may be live, unless the
  window is complete.
- **Constrained faces** (core R7; §7.1). What does not cross a face is
  *unobservable* from the far side, never absent.

---

## 3. Reading presence

This is core §8.1's "Reading presence". The tokens, the tokenless set,
re-minting and the caller's timeouts stay in core §8.1.

### 3.1 Handlers

- **Reading presence.** A caller or tool's liveliness GET on a session that holds a
  liveliness subscriber MUST use a callback or an unbounded handler. With
  zenoh's default 256-slot handler, such a GET hung at every size measured
  (core Appendix B). `[Sc: presence.md §4]`
- **The subscribers bind too.** Every liveliness subscriber the caller or
  tool holds on that session MUST be callback-driven, or drained as its
  samples arrive. A bounded subscriber nobody drains starves even a
  callback GET, silently (core Appendix B). `[Sc: presence.md §4]`

### 3.2 A fault read from presence's shape

- **A fault read from presence shapes holds in two reads.** A
  tool that decides a fault from the shape of presence (two holders, §6.1;
  a token its descriptor does not list, or an exposed interface with no
  token) MUST see it in two reads a grace apart, the grace longer than
  the deployment's longest re-mint overlap. Start-up, re-mint and
  teardown pass through such shapes briefly, by design. §6.1 states the
  rule for split-brain, and it holds for every such shape.
  - **The transient half.** An owner within core §8.2's step 4, after
    its instance token and before its interface tokens, serves a
    descriptor that lists an interface whose token it has not yet
    declared (before its instance token, a tool finding instances by token
    sees no instance, and so no shape).
    Seen in one of two reads a grace apart, that shape is no fault.

  `[Sc: operations.md §8; profiles/hostid/scenarios.md §6; profiles/redundancy/scenarios.md §12]`

### 3.3 Possibly incomplete

- A tool SHOULD treat a liveliness GET that ended at its timeout, rather
  than at the routers' final reply, as possibly incomplete: silence is not
  a verdict (O5). In zenoh 1.10.1 the difference shows: a GET that
  reached its timeout ends with an error reply, `Timeout`, and one the
  routers finished ends with none (core Appendix B). A read that received any
  error reply is possibly incomplete. `[Sc: presence.md §6]`
- **A read that runs to its session's interests timeout is not
  complete.** In zenoh 1.10.1 a client's or a peer's runtime ends a
  liveliness GET its routers have not finished at
  `routing/interests/timeout` (10 s by default), with the final reply
  alone and no error reply, whatever the GET's own timeout (core
  Appendix B).
  - A reader MUST read as possibly incomplete a read whose time reached
    its session's interests timeout, as it reads a read that received an
    error reply.
  - A reader MUST read as not standing a round trip (§3.7) that ended at
    that timeout. The round trip was asked, so it was made, but it
    establishes nothing.
  - A reader whose reads may wait longer SHOULD run its session with an
    interests timeout above the longest it waits for one: its long
    reads can then be complete (core Appendix B).
    - A reader that asks each GET with its own timeout at or above the
      interests timeout (§3.7) bounds its own wait for the read apart
      from the GET's own timeout.
    - This rule measures that bound, never the GET's own timeout: such a
      reader meets it when the longest it waits is below the interests
      timeout.
    - For example, a reader whose session's interests timeout is 120 s,
      which asks each GET with 121 s and waits at most 30 s for a read,
      meets it. Its 30 s wait is what is measured, never its GETs' 121 s.
  - **Its time** is counted from before the GET is asked to its final
    reply, so it is never less than the time the runtime counted. The
    read itself carries no sign of the interests timeout, so the rule
    needs the session's value: a reader reads it from the session's
    configuration (core Appendix B).
  - A reader that cannot read its session's interests timeout SHOULD be
    the one that opens that session, with an interests timeout it sets,
    so that it knows the value.
  - A reader handed a session whose interests timeout it cannot read
    takes that timeout as zenoh's default, 10 s. The default is right
    only when the session's configuration states no interests timeout. A
    lower value ends a read sooner, and nothing in the read shows it. So
    such a reader says that its reads' completeness rests on the default.

  `[Sc: presence.md §10]`

### 3.4 A refused read

- **A refused read is complete, and empty.** A router whose access
  control refuses a liveliness GET answers it the way it answers a
  selector no token matches: a final reply, no token, no error reply
  (core Appendix B). A reader cannot tell a refused read from absence, so O5's
  attribution through presence is only as good as the reader's grants:
  - The Consume and Call grants (core §11.1) include liveliness reads on the
    `@zk` subtree of every service they name, so a principal that may
    consume from a service or call it may also see it alive.
  - A tool reports absence as what its reader could see. Where it cannot
    rule out a refusal, it SHOULD say so, as it says a read is possibly
    incomplete.

  `[Sc: presence.md §6]`

### 3.5 A session cut off

- **A read through a session cut off is not complete.** A
  session that reaches no other zenoh node — a client that has lost its
  router, a peer or a router alone — has every liveliness GET answered
  at once with the final reply alone, no token and no error reply,
  though nothing was asked of anyone (core Appendix B). Its other GETs, S4's
  included, are answered so too.
  - Such a read is **not complete, whatever its replies say.** A reader
    MUST NOT conclude absence from it, and MUST NOT take it as healing a
    loss (§5.2).
  - A reader SHOULD NOT make such a read. Before each read, and again
    when it ends, the reader checks that its session holds a transport to
    another node.
    - A read the first check stops is not made. The reader reports it so,
      with that reason, as it reports a read possibly incomplete. What it
      would have established is unobservable.
    - A read whose second check finds no link was cut off while it ran
      (below).
  - **What the check reads.** zenoh 1.10.1 closes a transport in
    three steps: it drops the transport's links, then closes its routing
    face, which answers every read in flight, then removes the transport
    from the session's list. A check of that list made the moment a read
    is answered at the cut can still find the closing transport. So a
    reader checks the session's **links**: in zenoh 1.10.1,
    `Session::info().links()`, unstable (core §0), never `transports()`,
    which can miss the cut (core Appendix B).
  - **A read cut off while it runs** is not complete
    either: one asked while a transport was held, and answered after the
    session's last transport closed, comes back at once with the final
    reply alone, as one through a session already cut off does. It was
    made, so it is **possibly incomplete** (above), with a reason that
    says the session was cut off during it: the tokens it returned were
    seen, and what it shows absent establishes nothing. A reader that
    finds no link when a read ends reads it so.
  - **A profile's premise.** Some profiles answer unobservable where a
    presence read was possibly incomplete or refused. A read not made
    gives the same answer: unobservable, with the profile's reason.
    - Beside the profile's reason, a tool reports the read's own: not
      made, because its session was cut off or because the read would be
      beside a liveliness subscriber.
    - The tool's report keeps a read not made apart from a read possibly
      incomplete and from one refused. The profile's answer does not
      change.

  `[Sc: presence.md §7]`

### 3.6 Absence

- **Absence is established only by a complete read.** An
  owner or a token is absent when a complete read (above) shows none,
  and only then.
  - **A withdrawal heard live is a reason to read again, never absence
    by itself.** A liveliness subscriber hears a token withdrawn when its
    holder undeclares it, when its lease runs out, and equally when the
    reader's own router restarts or its own link drops: then every
    token at once, and their owners may never have left (core Appendix B). A
    reader MUST NOT conclude absence from a withdrawal alone.
  - A reader that follows presence live SHOULD read again soon after a
    withdrawal that leaves one of these with nothing held:
    - an address, with none of its instance tokens;
    - an interface, with none of its tokens;
    - a member, with none of its epochs.

    One read serves a burst of such withdrawals. The reader SHOULD report
    what that read establishes: absent when the read is complete and
    shows no such token held, present when it shows one. Until then what
    was withdrawn is **unconfirmed**: an address unobservable, never down,
    and a withdrawn token not yet gone.
  - A make-before-break re-mint (core §8.1) leaves nothing with nothing
    held, so it owes no read: another instance holds throughout.
  - A read not made (above) leaves the withdrawal unconfirmed until the
    reader's session reaches a node again and a read is made.
  - **Acting on a withdrawal.** A reader MAY act on a withdrawal
    heard live where the act concludes no absence, and fails toward what
    the reader does without the token's holder: it reports nothing
    absent from it, and reads again as above. What the read then shows
    held undoes the act. [`arbitration.v1`](profiles/arbitration/v1.md)
    so stops following a provider at its withdrawal, ineligible and never
    absent. It re-admits the provider only when a token is heard declared
    or a read shows one held (its §2.7). A move of its watch session holds
    back what the move makes it hear until its round trip stands (its
    §2.8). `[Sc: profiles/arbitration/scenarios.md §3, §4, §7]`

  `[Sc: presence.md §8]`

### 3.7 The watch session

- **The read that establishes absence goes on a session with no
  liveliness subscriber.** In zenoh 1.10.1, and on its main
  branch at `766b455`, a liveliness GET from a session that holds a
  liveliness subscriber can complete without live tokens the subscriber
  covers: the final reply and no error reply, the tokens missing. It
  happens with history or without, and by zenoh's code it lasts after the
  subscriber is undeclared (core Appendix B).
  - A reader SHOULD make every liveliness read it may conclude absence
    from (a seed, a re-read, a tool's presence read) on a session that
    has never held a liveliness subscriber of any kind. One undeclared
    still leaves its session exposed. The reader SHOULD hold its
    liveliness subscribers on another session of its own: a **watch
    session**.
  - zenoh orders one session's messages at its router, never two
    sessions'. A reader that seeds on one session and follows
    withdrawals on its watch session SHOULD make one round trip on the
    watch session after declaring its subscribers, and before the seed:
    a liveliness GET there whose answer it discards serves, since its
    final reply follows the declarations. Otherwise a token withdrawn
    between the seed's answer and the router's reading of the
    subscriber is never heard going.
  - **The round trip's GET need not match the subscribers' selectors.**
    In zenoh 1.10.1 a router handles one session's
    interests in turn: it answers the GET after it registered the
    subscribers declared before it and sent their history, whatever the
    GET matches. Through a chain of routers too: the watch session's
    router resolves its client's interest itself, and every withdrawal
    reaches the watch session from it. A GET of a key no token holds
    carries nothing: under a key `…/@zk/settle`, inside the presence
    subtree the subscribers read, so a grant that admits them admits it
    (core §11.1). `[Sc: presence.md §10]`
  - **The subscribers of a reader's watch session SHOULD have no history**
    when the reader seeds by a read on its other session. The round trip's
    final reply follows their history. So with history the round trip
    waits for the population, and without history it is one round trip
    (core Appendix B). The seed reads what is held, and a token declared
    before the subscribers and withdrawn after the round trip is still
    heard going: the router tells its withdrawal by its key.
    `[Sc: presence.md §10]`
  - **In a namespace, a watch session in no namespace, on the wire
    keys.** A router tells the withdrawal of a token declared before the
    subscribers, and withdrawn after the round trip, by the token's key
    (above). In zenoh 1.10.1 a namespaced session does not strip its
    namespace from that key. So its subscriber without history never
    hears the withdrawal (core Appendix B). A history there brings the
    population back into every round trip, long enough to reach the
    scenarios' 5 s wait and, in a debug build, to pass zenoh's 10 s
    interests timeout, where the round trip does not stand (above; core
    Appendix B). So:
    - A reader that follows presence in a namespace SHOULD hold its
      watch session in **no** namespace, and declare its subscribers,
      without history, on the **wire keys**: the namespace in front,
      `<ns>/zk2/…`, and its round trip's key so too,
      `<ns>/zk2/…/@zk/settle`. Its round trip is then one round trip at
      any population (core Appendix B), but for the first after each
      reopen of its transport, which waits for the population (below),
      and it hears the withdrawal on the wire key.
    - It strips the namespace from every key it hears. A key outside
      the namespace is not the deployment's: it is counted, never
      folded.
    - The wire keys are the keys a namespaced session sends: the grants
      core §11 compiles under the namespace (core §1.6) admit the subscribers and
      the round trip unchanged.
    - A reader that keeps a namespaced watch session with history
      instead SHOULD run its round trip's wait and its session's
      interests timeout above the history's time at its population.

    `[Sc: presence.md §10]`
  - **A round trip stands until the watch session's transport moves.** A
    transport that closes or opens re-declares the
    subscribers at a router that reads them again, unordered with the
    other session's reads; and a subscriber without history hears no
    withdrawal, at its link's drop, of a token it was never told of. A
    reader that follows presence live SHOULD make the round trip again
    before its next read, and read again at once, when its watch
    session's transport closes or opens, as after a withdrawal.
    - **A move unconfirms what the reader holds.** From the move until a complete read made after a round
      trip that stands, every token and every absence the reader
      followed through the watch session is **unconfirmed**: an address
      unobservable, never up or down on what was held before the move.
      While the transport is down its subscribers hear nothing, and a
      token withdrawn then is never told to them. A token declared then
      is told when the transport opens again, unordered with the
      reader's reads: in zenoh 1.10.1 the reopening tells a subscriber
      without history every token it matches (core Appendix B).
    - **A close's withdrawals come before its transport event.** In zenoh
      1.10.1 a watch session's
      subscribers are told the withdrawals of a close before the
      session's transport events listener hears the close, and the
      session holds no link by the time each is told (core Appendix B).
      A session with no link hears no withdrawal except those its own
      close tells. So a reader that holds back what a move makes it hear
      SHOULD begin the move at the first of two instants: its watch
      session's transport event, or a withdrawal its watch hears while
      the watch session holds no link. A reader that begins the move at
      the transport event alone applies the close's withdrawals as heard.
      A subscriber without history that was told no token hears no
      withdrawal at its first close (above).
      `[Sc: profiles/arbitration/scenarios.md §7]`
    - **An open is told before the subscribers' interests are declared
      again.** In zenoh 1.10.1 a session's transport events listener
      hears a transport open before the session's router face on it
      declares the subscribers' interests again. So a round trip asked at
      once at the open can find no face. It is then answered at once,
      complete and empty, before the reopen tells a token
      (core Appendix B): it stands and says nothing. A reader SHOULD ask the
      round trip after an open a margin after its event (the reference:
      100 ms), and SHOULD take what its watch heard before the round
      trip's reply as heard before it. `[Sc: profiles/arbitration/scenarios.md §7]`
    - **The round trip after a move waits for the population.** The
      reopening's telling comes first, and the round trip's final reply
      follows it, as a history's does (core Appendix B). So a watch
      session's round trip is one round trip at any population but for
      the first after each reopen of its transport, which waits for the
      reopening's telling; once that has been sent the next is one round
      trip again.
      - After a move, a reader SHOULD wait for the round trip longer
        than the reopening's telling takes at its population. It SHOULD
        run its interests timeout above that time too. That time is about
        what a session's first read of the population costs, every
        matching token sent once. A later read of it costs far less, and
        a read after a reopen is a later read (core Appendix B). So a
        reader that sizes this wait from its own later reads sizes it too
        short. Its seed, on a session that has not read the population
        before, is a session's first read, and the one it can time. The
        round trip after a reopen can run longer than the seed did, so a
        wait sized from a seed keeps a margin (core Appendix B).
      - A reader whose wait ends before the round trip's reply reads
        what it reads meanwhile as possibly incomplete, as any read made
        while no round trip stands (below). Before its next read, it takes
        up the round trip still in flight (below, "One round trip, and
        one read, at a time"). It asks another round trip only once that
        one has ended without standing.
      - **One round trip, and one read, at a time.** zenoh
        1.10.1 tells a router nothing of a liveliness GET whose caller
        stops waiting for it, by dropping it, by its timeout or by
        cancelling it, and the router answers it in full (core Appendix B).
        A reader SHOULD NOT ask a round trip while one it asked since
        its watch session's transport last moved is in flight. It SHOULD
        NOT ask a read while one it asked of the same scope is in flight.
        When the reader next needs a round trip, or a read of that scope,
        it takes up the one in flight. It does so whichever wait for that
        one ended: its caller's, or its own. A reader that asks again
        instead can ask read after read and fold none (core Appendix B).
        - **In flight is at the reader**: from the GET's
          ask until its final reply reaches the reader. A GET that its
          own timeout ends is over for the reader: zenoh hands the reader
          an error reply and the final reply, and nothing after them.
          Such a GET cannot be taken up. Its router answers it in full all
          the same (core Appendix B), and a GET asked on that session
          meanwhile waits behind it, for what is left of its router's
          answer (core Appendix B).
        - A reader MAY ask each GET with its own timeout at or above
          its session's interests timeout, where zenoh's runtime ends
          it, and bound its own wait apart, so that none ends before the
          reader can take it up. The reference does so for its
          round trip; its presence feed asks a read with the reader's
          timeout, and when one runs to it asks again after a back-off,
          1 s at first.
    - **No round trip through a watch session cut off.**
      Through a session cut off zenoh answers a liveliness GET at once,
      with the final reply alone (above), and a round trip is a
      liveliness GET. So a round trip through a watch session that holds
      no link to another node, or held none when it ended, does not
      stand, whatever its replies say. A reader checks the watch
      session's links before the round trip and when it ends, as it
      checks a read's. It asks none through a session cut off, and makes
      the round trip when the transport opens again.
    - **A reader whose watch session is cut off follows nothing**: its
      subscribers hear nothing. It SHOULD NOT make a read that it would
      follow from that watch session, whatever its reading session's
      links. It reports the read as not made, the reason its watch
      session's. What the read would establish is unobservable until the
      transport opens, a round trip stands and a read after it is
      complete.
    - **A read made while no round trip stands**, a seed or a re-read, is
      possibly incomplete: an address it shows is up,
      one it does not is unobservable, never down.

    `[Sc: presence.md §10]`
  - Ordinary GETs (S4's, a descriptor's, a bundle's) are not affected,
    and may go on either session in the namespace. A watch session in no
    namespace makes none of the deployment's: there its
    base-relative keys are the bus root's.

  `[Sc: presence.md §9]`

### 3.8 A read on a token's own session

- **A read on a token's own session answers that token itself.** An
  implementation note. In zenoh 1.10.1 a session's liveliness GET returns
  the tokens that session declared from the session itself, whatever its
  router answers, a router that refuses the read included (core
  Appendix B). So a read
  on a token's own session says nothing about the bus for that token. A
  reader that must see its own token as the bus holds it reads on a
  session that does not hold it: `redundancy.v1`'s candidate, whose
  check of its own claim is what tells a refused read from a lone
  candidate's (its §2.5). `[Sc: profiles/redundancy/scenarios.md §5]`

### 3.9 What a complete read covers

- **What a complete read covers.** A read is complete only for the tokens its routers' clients
  and its linked peers hold. In zenoh 1.10.1 a router propagates no
  token from one of its peers to another: the `auto` gateway, its
  default, makes a router's peers one region, whose members it expects
  to be linked to each other (`hat/peer/token.rs`), and
  `routing.router.peers_failover_brokering` is deprecated, read and
  ignored (core Appendix B).
  - A peer that other peers cannot link to reads none of their tokens,
    and its complete read establishes no absence of them.
  - A reader that concludes absence of a token a peer holds SHOULD be a
    client of a router, or a peer linked to that peer. `redundancy.v1`
    makes it a MUST of its candidates and of the tools that read them
    (its §2.5).

  `[Sc: profiles/redundancy/scenarios.md §5]`

### 3.10 What a tool can count

This is core §8.3's "What a tool can count"; the presence budget stays
there.

**What a tool can count**: liveliness tokens, the only declarations
a reader can list, and of those only the ones its selectors and grants
reach. Another application's tokens under its own verbatim chunks are out
of reach, and so are subscribers and queryables. A tool's count is a lower
bound of the domain's declarations, and it reports it as one.

---

## 4. Reading state

This is core §4.2's and §4.4's reading of state. S1–S7, a late joiner's order
and merge, a selector over several owners and "Observing S1" stay in core
§4.2; archives stay in core §4.4.

### 4.1 A seed

- **A tool's merge lasts.** A tool keeps core §4.2's merge for as long as
  it follows, across two timestamp ids too. A live sample whose stamp is
  older than what its key holds is not kept after the seed either. A tool
  that bounds what it holds counts what the bound forgot, and treats a
  forgotten key as holding nothing. A consumer orders by core §4.3's
  consumer rule instead (core §4.2). `[Sc: state.md §7]`
- **R6 and a tool's seed.** R6 binds a consumer: a consumer that
  seeds discards a reply or a sample whose key is not concrete, whichever
  path brought it. A tool's seed shows what arrived, as its subscription
  does. It keeps such a sample under the key it arrived on, merged with
  that key alone, never with the concrete keys it covers.
  `[Sc: state.md §11]`
- **What a seed cannot know.**
  - A GET that ran to its timeout seeds possibly incompletely. §3.3's
    reading of a liveliness GET applies to a state GET: in zenoh 1.10.1, a
    GET that reached its timeout ends with an error reply (core Appendix B).
  - An owner absent at the GET seeds nothing for its keys. That is O5's
    silence, never "no state", and the subscription shows its keys as
    they change from then on.
  - A seed is current state, the owner's answer (S6). Last-known state is
    still read from an archive, explicitly (S5).

### 4.2 S4's check against the admin space

A **tool** checks S4 against the routers' storage admin space. A consumer
cannot tell under `Latest` which replier answered.
- **The admin space is off by default** in zenoh 1.10.1 (core Appendix B). A
  deployment that wants S4 checked enables it, read-only, for the tools'
  principals (core §4.2). Without it, a tool reports the check
  unobservable, never clean.
- **What the check reads.** The reference reads two selectors:
  - `@/*/router`, the routers that answer;
  - `@/*/router/status/plugins/storage_manager/storages/**`, one key per
    storage a router's storage manager runs, its value the storage's
    configuration with its `key_expr`.

  **A storage** is an answer to the second selector whose key the selector
  includes, ending `…/storage_manager/storages/<name>`. Any
  other answer is not a storage.

  A storage whose `key_expr` intersects an owner's `state/**` or
  `@state/**` breaks S4. A router that answers the first selector and has
  no storage answer runs no storage. When no router answers the first, the
  check is unobservable.
- **Who answered.**
  - **The problem.** Any session can declare a queryable under
    `@/<zid>/router`, a real router's zid included. The key names the zid
    its declarer chose, and the document is whatever that session sends.
  - **Measured on zenoh 1.10.1.** With the admin space off, a client's
    answer on the router's own key is the only answer. A tool that read it
    as the router's would report S4 clean.
  - **The rule.** A tool counts an answer as a router's only when the
    reply's replier id is the zid the key names, and that zid is a
    **verified router**. Any other answer is **unverified**, and an
    unverified answer never contributes to a clean verdict.
  - **Verified routers, outward:**
    - the routers the tool's session is connected to, and the session
      itself, are verified;
    - so is every zid a verified router's own answer lists among its
      `sessions` with `whatami` `router`;
    - and so on, until no new router is verified.

    A client connects to one router at a time (core Appendix B), so without
    this rule a far router's honest answer could never count, and S4 could
    never be clean in a deployment with two or more routers. A session that
    is no router is listed as a `client` or `peer`, and never qualifies.
  - **The replier id** is unstable API in zenoh 1.10.1 (core Appendix B), which
    the core does not require (core §0). A tool that cannot read it holds every
    answer unverified.
  - **The operator's alternative.** An operator MAY tell a tool to trust
    every answer when the deployment's grants deny `@/**` queryables to
    every principal (core §11.1), which no tool can observe.

  `[Sc: security.md §3]`

### 4.3 What a tool does not ask of the admin space

- **What a tool does not ask.** A node with its admin space on, a router, a peer or a client,
  answers its declaration records, `@/<zid>/<whatami>/{subscriber,
  publisher,queryable,querier,token}/**`, while it holds its routing
  tables' read lock, and a declaration, an interest or a session from
  another face between two of those replies freezes it for good
  (core Appendix B). So does its `linkstate/*`, and its
  `route/successor/src/<zid>/dst/<zid>`.
  - A tool MUST NOT ask a selector that intersects one of these keys of
    any node unless its user asks for that read. `[Sc: state.md §12]`
  - A selector such as `@/*/router/**/storage_manager/storages/**`
    intersects them. A router records each queryable as
    `@/<zid>/router/queryable/<key expr>`, and the record of a queryable
    ending in `**`, such as an owner's `…/state/**`, answers it.
  - The two selectors above do not, and every zenoh 1.x storage manager
    answers a storage's GET under `status/plugins/`, never under
    `config/`.
  - Under `deny`, the admin read admits none of them (core §11.1). A
    router under a generated block refuses such a GET from a tool's
    credentials. So a tool whose user asks for that read reads none of
    these records through that router. It says that the admin space did
    not answer, never that nothing is declared.

### 4.4 Whose stamp

- **A tool's S1 check.** A tool attributes a state stamp
  to its owner by comparing the stamp's id with the owner's `meta.zid`
  (core §3.3; S1, core §4.2, "Observing S1"), by value (below). Without
  `meta.zid`, a stamp's clock is unattributable, never foreign.
  `[Sc: profiles/timing/scenarios.md §4, §5]`
  - **A foreign stamp is a finding** whatever else the tool read: an owner
    that is its own router stamps with that router's id, which is its own
    `meta.zid`.
  - **An owner's stamp is clean** only when the tool counted at least one
    router's own answer ("Who answered", §4.2), and `meta.zid` is none of
    the zids it knows to be routers. Those are the routers its session is
    connected to, the routers whose answers it counted, and every zid
    such an answer lists as a `router` session.
  - **A counted answer, not a connection.** The routers a session is
    connected to are verified routers, and verification starts from them,
    but a connection alone shows nothing about the owner: only a router's
    own answer lists the router sessions behind it.
  - **Otherwise S1 is unobservable for that owner,** never clean. Either
    `meta.zid` is a router's, so the owner is its own router, or the tool
    counted no router's answer (it has no admin read, the admin space is
    off, or it cannot read the replier id, core Appendix B), so the owner may be
    a router it cannot see.
- **Two zids compare by value.** A tool MUST compare two zids by
  value, never by their text: case and leading zeros carry no meaning.
  How zenoh 1.10.1 writes a zid, and how an owner writes `meta.zid`, stay
  in core §3.3, "How a zid compares". `[F: profiles/timing/conformance/lifespans.json]`

### 4.5 An archive's alignment

- **What a tool can see.** An archive publishes no alignment status
  at this version: not when it last aligned, from where, or how many
  attempts it made. A tool sees each key's `confirmed` and nothing more, so
  an archive that holds nothing reads as aligned, and a tool says so. An
  alignment status is a profile's to define, and none is written yet.

---

## 5. Windows and the population budget

This is core §2.4's and §2.7's tool half. The bound, a member, the
owner's rule and an event's rate stay in core §2.7, and the QoS in core
§2.4.

### 5.1 What a tool can conclude rests on QoS

- **What a tool can conclude rests on it.** Only a resource
  declared `reliable` with `block` has windows shown to have lost no
  delivery, so only its population and its rate can read clean from a
  window (§5.2). Any resource's window can still show a finding.

### 5.2 What a tool reads

- **A complete reading of a state** is an S4 GET to its owner (core §4.2) that
  ran to its final reply, with no error reply: a GET that reached its
  timeout ends with one (core Appendix B). Its members are the keys answered
  with a value. A `reply_del` is not a member.
- **A complete reading of a stream or an event** is a subscription window
  that hears every member live at its end. It is complete when all three
  hold:
  - it lasted at least one liveness span, an hour for a stream and the
    retention for an event, counted from when its subscription reached
    the owner (below);
  - it lost no delivery, as established below;
  - the owner was present, its instance token held, from the window's
    start until one lease after its end (below). An owner that
    joined, left or re-minted while the window listened is a new instance
    or a gap (core §1.5, §8.1), so the window is incomplete.

  A member is live for one span after its last publication, so such a
  window heard it. For a stream, a window counts the members heard within
  one hour; for an event, the members with occurrences within the
  retention. Two instants exactly one span apart are not within one span.
- **Lost no delivery is established, never assumed.** A reader can
  know that it lost a delivery, and never that it lost none: a sample a
  queue dropped leaves no trace at a subscriber (core Appendix B). A window
  lost no delivery when both hold:
  1. **Its reader lost none.** Over the window and the watch after it
     (below):
     - its own queue neither overflowed nor fell behind, and no delivery
       was kept out of its count by a cap;
     - **no transport of its session closed.** A router that gives
       up on the reader closes the reader's own transport, and what it
       held for the reader goes with it;
     - **its process did not stall longer than the bound**: the
       transport's `wait_before_close`, or its lease less one keep-alive
       interval when that is shorter. With zenoh's defaults these are 5 s
       and 7.5 s: a 10 s lease, with a keep-alive every quarter of it. A
       reader stalled no longer than the bound is never given up on: its
       router waits on its full queue for `wait_before_close`, and hears
       from it within its lease.
  2. **The path could not drop one silently.** The resource declares
     `reliable` with congestion `block` (core §2.4). A router's queue never
     drops such a sample: its push waits for room, and one it cannot queue
     within the transport's `wait_before_close` (5 s by default) closes
     the transport (core Appendix B). So a reliable path loses a delivery only
     by failing. A link that fails withdraws the owner's instance token at
     the reader, which the presence premise reads. A reader that stalls
     has its own transport closed, which rule 1 reads.

  Otherwise no window of the resource is lossless, whatever it heard, and
  a reading that needs one is unobservable. The reason names what failed:
  the reader's own loss, its stall, a miss detected, or the QoS. At
  `best_effort`, a sample dropped anywhere on the path leaves no trace; at
  `reliable` with `drop`, a full queue drops one and leaves none.
  `[F: budget/lossless.json]` `[Sc: budget.md §1]`
  - **Why the reader watches itself.** The owner's token does not show a
    stalled reader's loss. Measured, a reader stalled past
    `wait_before_close` saw the token withdrawn only when the token had
    reached it before it stalled. Its own transport closing withdraws
    only the tokens it had received, and a declaration its router had
    not yet sent it went with that router's queue. Its own transport's
    close showed in every run (core Appendix B).
  - **How a tool establishes it.** It watches its own receive path from
    before the window opens until the watch after it ends:
    - **its transports**, from zenoh's transport events
      (`Session::info().transport_events_listener()`, unstable in 1.10.1,
      core §0). A transport of its session that closed is a stall. This sees a
      router giving up on a reader whose receive threads were blocked
      while the rest of its process ran, which no ticker sees;
    - **a monotonic ticker** on a thread of its own. The greatest gap
      between two ticks, the gap still open when the tool reads it
      included, longer than the bound is a stall. This sees a process
      that did not run (stopped, starved, swapped out) on its first tick
      after, before its session has read its transport's close.

    A tool that cannot read transport events says so. It then rests that
    half on a premise of its own: nothing in its process blocks zenoh's
    receive threads, which its ticker cannot see (core Appendix B).
    `[F: budget/lossless.json]` `[Sc: budget.md §1]`
  - **Miss detection is evidence of a loss, never of none** (core §2.5). A
    subscription that detects misses and detected one lost a delivery.
    One that detected none established nothing: zenoh-ext takes a
    publisher's first sample heard as its start, so it reports neither
    the samples before that one nor a publisher none of whose samples
    arrived (core Appendix B). A member whose every sample was dropped is the
    one a population reading must rule out. A subscription detects misses
    only where the resource declares them (`history.miss_detection_ms`);
    elsewhere it has nothing to detect. `[Sc: budget.md §3]`
  - **What a deployment provides**, which no tool can see:
    - links that zenoh counts reliable, since a reliable sample crosses a
      UDP link best-effort when no reliable link is up;
    - routers in processes of their own, since a process that runs a
      router beside a subscriber whose callback blocks shares zenoh's
      receive threads with it, and has lost deliveries with no transport
      closed (core Appendix B).
- **The owner present until one lease after the end.** A link that
  closes withdraws the owner's instance token at the reader at once. One
  that goes silent withdraws it only when its lease runs out, and the
  deliveries put meanwhile are lost (core Appendix B). So a failure in the
  window's last lease is seen after the window ends. After the window's
  end, a tool watches the owner's instance tokens for the longer of one
  lease and the routers' `wait_before_close`, plus its reply timeout.
  `[Sc: budget.md §2]`
  - **The lease** is the deployment's, 10 s by default.
  - **`wait_before_close`** is the deployment's too, 5 s by default. A
    push that cannot queue within it closes the transport it pushes on
    (core Appendix B, "What a path drops"). A path that stays up while its
    queue is full keeps its lease. So its withdrawal can reach the reader
    up to `wait_before_close` after the failure. That is later than a
    silent link's when `wait_before_close` is longer than the lease.
    - **A bound reasoned, not measured.** This half rests on that
      behaviour of a push alone. `budget.md` §2 measures a silent link's
      withdrawal one lease after the failure. No run has shown one later
      than a lease (core Appendix B, "Not measured"). The bound is
      conservative. A longer watch can only read a window incomplete that
      a shorter one would read complete, never the reverse.
  - **The reply timeout** allows for the withdrawal's transit.
- **A window counts from its subscription reaching the owner.** A
  sample the owner put before its router knew of the subscription was
  never routed to it, and no reader can see that moment. A tool counts a
  window's span from its subscription's declaration plus its reply
  timeout. This is a convention, the time a tool already allows a message
  to cross the path, and not a bound zenoh gives.
- **Every other reading is incomplete.** A member it never heard may be
  live: a sample lost, a publisher matched late, and a member quiet while
  it listened look the same as no member.
  - A GET that ended at its timeout, or with an error reply.
  - A GET made through a session that reaches no other zenoh node, which
    zenoh answers at once, complete and empty, though nothing was asked of
    anyone (§3.5), or one whose session's last transport closed while it
    ran. It heals no loss.
  - A window shorter than one span, one not shown to have lost no
    delivery, or one over which the owner was not present throughout.
  - An archive's answer, which is last-known (S6).
- **A window shows no state population,** however long. It hears puts and
  deletes, and a delete it missed would count a member twice.
- **A refusal narrows even a complete reading** (§3.4). A tool
  reports its count as what its reader could see.
- **Spans.** A tool measures a rate's span between the occurrences'
  timestamps when they carry one clock id (a session with its HLC enabled
  stamps every put, core §4.1), and on its receive clock otherwise, where
  transit can shorten a span. A population's spans are on the tool's
  receive clock, which every member shares.

### 5.3 What a tool concludes

**What a tool concludes.** The question is "does this instance exceed its
budget here?", per templated resource, and the finding is the yes. A
lower bound that already exceeds the bound is a finding, so a finding
needs no complete reading, and a rate's needs no window of a whole
period.

| | Population | Rate (events) |
|---|---|---|
| **Finding** | more members than the bound, in any reading | `<n>` + 1 occurrences of one member less than the period apart, in any window |
| **Clean** | at most the bound, in a complete reading with at least one member: a state's complete GET, or a stream's or event's complete window | a window at least one period long that lost no delivery, the owner present throughout (both as above), an occurrence heard, none beyond the rate |
| **Unobservable** | at most the bound in an incomplete reading: a GET that did not run to its final reply; a window not shown to have lost no delivery, shorter than one span, or over which the owner was not present throughout; no member at all (O5); a reading that does not count the kind | a window not shown to have lost no delivery; shorter than one period; the owner not present throughout; no occurrence heard (O5) |
| **Not asked** | the no-ceiling bound; a template without parameters; an operation | a resource that declares no rate |

`[F: budget/]` `[Sc: budget.md §1–§3]`

---

## 6. Presence's shape: split-brain and the standby mark

This is core §6's tool half; ownership, serving and the standby stay
there.

### 6.1 Split-brain

Two instances of one service holding an interface token for the same
interface, for longer than a grace period, is a **finding**, except
where replicated serving explains it (below). A tool diagnoses it. The
runtime does not fence. For an interface whose contract uses
`redundancy.v1`, its holders step down as that profile's §2.9 says. That
is no fence either, since writes in flight still land (core §6).

- **A holder is an incumbent, whatever its mark.** The check
  counts the instances holding the token. One whose descriptor marks
  the interface standby is counted all the same, and its descriptor
  disagrees with its tokens besides: it holds a token for an interface
  its descriptor says it exposes nothing of (core §8.1).
- **Replicated serving.** Holders are not a finding when at most one
  of them exposes an exclusive resource of the interface: an interface
  whose holders expose only replicated operations is held by any number
  of instances. A tool decides this only for an interface whose
  contract declares a replicated operation, from the contract and the
  holders' descriptors (core §3.3's exposure). One that cannot read a
  holder's descriptor or the contract reports the holders as
  undecided, neither a finding nor clear (O5).
- **Longer than the grace period** is judged from two presence reads,
  `grace` apart: the service and interface held by two or more
  instances in both reads, not necessarily the same ones. A re-mint's
  overlap shows in one read at most, and a standby in neither. A read
  that ended at its timeout can miss a holder, never invent one (§3.3).
- The grace period is a tool setting. A tool's grace period SHOULD exceed
  the longest re-mint overlap the deployment allows (core §8.1).

`[Sc: operations.md §8]`

### 6.2 Reading the mark

- **Reading the mark.** A tool tells a standby from a twin by
  it. An instance listing the interface marked standby, and holding no
  token of it in a complete read (§3), is a standby: no finding of
  this check, nor of §3.2's fault shapes, since its listing claims
  nothing exposed. One listing it unmarked, outside its tokenless set,
  and holding no token of it in two complete reads a grace apart,
  claims an exposure no token covers (core §8.1). `redundancy.v1` §5 states
  what a tool concludes of each instance and of the address.
  `[Sc: profiles/redundancy/scenarios.md §12]`

---

## 7. Bindings, calls and payloads

### 7.1 The graph and liveness

This is core §3.2's tool half: the tool's halves of R3 and R7, and a
tool's `self.system`.

- **The graph** (core R3). The data-flow graph is read from descriptors,
  never inferred. `[Sc: bindings.md §3]`
- **Liveness across a face** (core R7). Where nothing crosses, liveness is
  *unobservable*, and a tool MUST say so rather than report the provider
  down. `[Sc: constrained.md §3; profiles/link/scenarios.md §6]`
- **A tool has no system of its own.** A tool that is not a service MUST
  refuse a `self.system` provider, as it refuses a parameter bound to
  `self.system` (R2). `[Sc: bindings.md §5]`

### 7.2 Judging O3 from outside

This is core §5.1's "Every call is answered (O3)", judged by a tool. A call
that reaches an owner's queryable gets a value or an envelope.

- A tool judging O3 from outside, as a conformance suite does, holds a
  silence as a finding only under grants that let it call: an
  access-control refusal is silent too (O5, core §11.3). It says so beside
  the finding.
  - **Where a tool learns its grants.** No tool can observe its
    grants (core §11.3). It learns that they let it call from its operator, or
    from the deployment's core §11.1 input when it holds one; a deployment that
    runs no access control lets every principal call.
  - **Told so,** the tool holds as the finding a silence from an owner
    whose tokens it reads. **Not told,** the silence is unobservable, and
    the tool names the premise it lacked: a silence is never a verdict on
    its own (O5).

`[Sc: operations.md §3, §5]`

### 7.3 Decoding and rendering

This is core §7.1's and §7.2's tool half; type identity and the sample's
`Encoding` stay there.

- **What a tool that shows or judges a payload decodes** (core §7.1).
  Such a tool MUST decode each payload by its declared type: `protobuf`
  (proto2 and proto3) and `jsonschema` (the zk2 subset of JSON Schema
  2020-12, core §7.3), and a `raw` type by its media type only. A
  reserved kind (`flatbuffer`, `ros2msg`, `cdr`), which a contract
  declares as `raw` with a media type, tools MAY decode through a plugin.
  `[Sc: types.md §2]`
- **Decode order** for a tool is: the sample's `Encoding`, then the
  contract's type, then sniffing.
- **Honest rendering.** A tool that shows a payload it cannot decode MUST
  render it as its declared type and size, or through a plugin. It MUST
  NOT show garbage, and MUST NOT drop the sample silently.
  `[Sc: types.md §2]`
  - **A type is named one way**, decoded or not: as a type reference
    is written (core §9.1). That is the message name for protobuf,
    `json:<name>` for a JSON Schema type, and the media type for a raw
    one. Which wire a JSON Schema type was read from, JSON or CBOR, is
    part of why it failed to decode, not of its name.

### 7.4 Shared memory

This is core §7.4's tool half; SHM's facts and the deployment's
`RLIMIT_MEMLOCK` stay there.

- A tool SHOULD check the limit at start: the `RLIMIT_MEMLOCK` that core
  §7.4 asks a deployment to set to at least the pool plus 2 MiB.
  - **What the check reports.** The pool is a
    publisher's, which a tool at its start does not know, so the check
    reports what the limit admits: the largest pool its soft limit
    covers, the limit less 2 MiB, or, below 2 MiB, that it admits none.
    An unlimited limit admits any pool, and a limit the tool cannot read
    leaves the check unobservable.

---

## 8. What a tool tells a person

These are the tooling guide's observer obligations O1, O5 and O7, each
with its evidence. The guide keeps the others, and how the reference tools
spell these.

### 8.1 A key that is not zk2 is a fact, not an error

The bus is shared (core §1.7): v1 keys, rmw_zenoh keys and other
applications coexist. A tool never hides or refuses a foreign key. It
shows what it can still establish: the chunks, the traffic, and a
structural rendering of the payload (§7.3's last rung). Then it states
what it could not. `[F: keys.json]` `[Sc: state.md §11]`

### 8.2 A wildcard scope is not total coverage

`*` and `**` never match a verbatim chunk. So an ambient selector
(`zk2/**`, `zk2/<system>/**`) never delivers:
- `@stream` or `@state` samples;
- `@op` queryables;
- `@adv` sidecars;
- control keys under `@zk`, contract bundles included (`zk2/@zk/contract/…`).

That guard is a gift, and a trap (core §1.3). A tool that offers a
wildcard scope does one of two things, in every medium it writes:
- it names, beside the scope, the verbatim kinds it watches;
- or it states that the verbatim kinds are excluded.

A report carries the names or the statement in a field. A stream of rows
with no place for a field (NDJSON) carries them on the tool's standard
error.
`[Sc: grammar.md §1]`

**Everything is three selectors.** Positions 1–5 of a zk2 key have fixed
arity (core §1.3), so these three reach every data key of every service
of a deployment, present or future, with no presence read:
- `<ns>/zk2/**`, the ambient kinds;
- `<ns>/zk2/*/*/*/@stream/**`, the explicit streams;
- `<ns>/zk2/*/*/*/@state/**`, the explicit state.

Together they are the deployment's whole data plane apart from its
operations (`@op`) and its control keys (`@zk`). An `@adv` subtree is
advanced publication's own (core §2.5), not data. A tool that offers
"everything" spells all three, and still names `@op`, `@zk` and `@adv`
as excluded. The explicit two are an opt-in, never a default: the guard
exists so that nobody pulls a camera's frames by accident.
`[Sc: grammar.md §1, §2]`

### 8.3 A tool that reports a timestamp names who stamped it

A state sample's stamp carries its clock's id (core §4.1, §4.3).
- **The owner's own stamp.** It is the owner's when S1 holds, and its id
  then matches the owner's session, which the descriptor's `meta.zid`
  names where present. Two zids compare by value, never by their text
  (§4.4).
- **A router's stamp.** A router stamps what arrives unstamped, and so
  does a reader's own session whose HLC is enabled (core §4.1).
- **No stamp.** A stream sample may carry none.

A tool says whose clock a time, an age or a latency is on.
`[Sc: state.md §1]` `[Sc: profiles/timing/scenarios.md §4, §5]`

---

## Appendix A. Where each rule came from

Core 0.58 moved the blocks below here, with no rule changed; core 0.57 is
their last text in the core, and each old home now holds a one-line
pointer, so a citation of the core section still leads here. The
tooling guide's rules that carry evidence were promoted with text 0.1.

| Here | From | What |
|---|---|---|
| §1 | tooling guide §1, §3 (O4) | the judgement shape, the three media, Not asked kept apart, an empty scope |
| §2 | tooling guide §2; core O5 | silence is never a verdict, and its instances |
| §3.1 | core §8.1, "Reading presence", "The subscribers bind too" | handlers on a session with a liveliness subscriber |
| §3.2 | core §8.1, "A fault read from presence shapes holds in two reads" | two reads a grace apart |
| §3.3 | core §8.1: a GET ended at its timeout; "A read that runs to its session's interests timeout is not complete" | possibly incomplete |
| §3.4 | core §8.1, "A refused read is complete, and empty" | a refused read |
| §3.5 | core §8.1, "A read through a session cut off is not complete" | a session cut off, the links check, a read cut off while it runs, a profile's premise |
| §3.6 | core §8.1, "Absence is established only by a complete read" | a withdrawal heard live, acting on one |
| §3.7 | core §8.1, "The read that establishes absence goes on a session with no liveliness subscriber" | the watch session, its round trip, a namespace's wire keys, a move of its transport |
| §3.8 | core §8.1, "A read on a token's own session answers that token itself" | |
| §3.9 | core §8.1, "What a complete read covers" | peers and routers |
| §3.10 | core §8.3, "What a tool can count" | a token count as a lower bound |
| §4.1 | core §4.2, "R6 and a tool's seed", "What a seed cannot know" | |
| §4.2 | core §4.2: a tool's S4 check, "What the check reads", "Who answered" | the admin space's tool half; its deployment half stays in core §4.2 |
| §4.3 | core §4.2, "What a tool does not ask" | the declaration records no tool asks unbidden |
| §4.4 | core §3.3's `meta` and "How a zid compares" (the tool's sentences); core §4.2, "A tool's S1 check" | |
| §4.5 | core §4.4, "What a tool can see" | an archive's alignment, as a tool sees it |
| §5.1 | core §2.4, "What a tool can conclude rests on it" | |
| §5.2 | core §2.7, "What a tool reads" | complete readings, lost no delivery, the reader's stall, the owner present, spans |
| §5.3 | core §2.7, "What a tool concludes" | the four states of a population and a rate |
| §6.1 | core §6: split-brain's holders, replicated serving, the grace period | |
| §6.2 | core §6, "Reading the mark" | |
| §7.1 | core §3.2: R3's and R7's tool halves, "A tool has no system of its own" | |
| §7.2 | core §5.1, "Every call is answered (O3)": a tool judging O3 from outside | |
| §7.3 | core §7.1's "Generic tools" column and its plugin; core §7.2, "Decode order" and "Honest rendering" | |
| §7.4 | core §7.4: a tool checks the memlock limit | |
| §8.1 | tooling guide O1 | |
| §8.2 | tooling guide O5, and its "Everything is three selectors" | |
| §8.3 | tooling guide O7 | |

## Appendix B. Evidence index

| Evidence | Covers |
|---|---|
| `conformance/budget/` (`population.json`, `rate.json`, `lossless.json`) | §1, §2, §5 |
| `conformance/keys.json` | §8.1 |
| `scenarios/presence.md` §4, §6–§10 | §1, §2, §3 |
| `scenarios/state.md` §1, §7, §11, §12 | §4.1 (§7 step 2: a tool's merge for as long as it follows), §4.3, §8.1, §8.3 |
| `scenarios/security.md` §3 | §4.2 |
| `scenarios/budget.md` §1–§3 | §5.2, §5.3 |
| `scenarios/operations.md` §3, §5, §8 | §2, §3.2 (§8 step 6), §6.1, §7.2 (§5 step 2) |
| `scenarios/bindings.md` §3, §5 | §7.1 |
| `scenarios/constrained.md` §3 | §7.1 |
| `scenarios/types.md` §2 | §7.3 |
| `scenarios/grammar.md` §1, §2 | §8.2 (§1 step 5) |
| `profiles/*/conformance/conclusions.json` (`arbitration`, `redundancy`, `timing`) | §1, not this profile's |
| `profiles/freshness/conformance/judgements.json`, `profiles/telemetry/conformance/samples.json`, `profiles/desired/conformance/conclusions.json` | §1, not this profile's spelled as not asked with a reason |
| `profiles/timing/conformance/lifespans.json` | §4.4, zids compared by value |
| `profiles/arbitration/scenarios.md` §3, §4, §7 | §3.6, §3.7 |
| `profiles/hostid/scenarios.md` §6 | §3.2 |
| `profiles/redundancy/scenarios.md` §5 | §3.8, §3.9 |
| `profiles/redundancy/scenarios.md` §12 | §3.2, §6.2 |
| `profiles/link/scenarios.md` §6 | §7.1, liveness across a face |
| `profiles/desired/scenarios.md` §9 | §1 |
| `profiles/timing/scenarios.md` §4, §5 | §4.4, §8.3 |
