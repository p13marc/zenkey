# zk2 spec changelog

Amendments to [`core.md`](core.md). Each entry records what changed, what
deliberately did not, and why.

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
