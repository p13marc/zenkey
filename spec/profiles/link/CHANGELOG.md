# `link.v1` changelog

Versions of the text of [`v1.md`](v1.md). Each entry records what changed,
what deliberately did not, and why. A breaking change is a new major, a new
file, never an entry here ([`../README.md`](../README.md)).

## 0.8 — 2026-10-21: the counter is `telemetry.v1`'s (#819, TA)

`telemetry.v1` text 0.1 defines the counter type and its epoch that §2.13
waited for. §2.13 now cites it.

**Changed: §2.13's citations, no rule.**
- **A counter** is `telemetry.v1`'s: a member whose declared kind, or
  whose Point's tag, is `counter` (its §2.5, §2.7), where 0.7 said "a
  resource whose `telemetry.kind` is `counter` (core Appendix D's interim
  `telemetry` row), until `telemetry.v1` defines its counter type".
- **Its epoch** rides in the Point's `epoch` or an attachment typed
  `telemetry.v1.Epoch` (its §2.9), and a decrease without one reads as
  that profile's reset (its §2.10). The SHOULD's citation adds its §2.9
  and §2.14.

**Deliberately not changed.**
- **The rules.** The SHOULD to carry the epoch, and a decrease read as a
  reset where none crosses, stand as 0.7 wrote them; `telemetry.v1` §2.14
  states the same from the counter's side.
- **Everything else:** the plan, the lints, the fixtures, the scenarios,
  §5's row, and the core the text is written against (0.32), which the
  citation does not need. zenoh-modem's `sdu/dropped_total` still crosses
  with no epoch (Appendix B): adding the attachment is its contract's
  revision, review under core §9.8.

## Fixtures after 0.7 — 2026-10-19: two readings §2.1 already decides (#769, LI)

No text change. zk2py (round 37) read two of §2.1's lints by its rule
"Each key is checked alone. A key whose value is wrong is reported once,
and checks that need it are skipped", where the reference read them
otherwise, and no fixture told the two apart:
- **`batch_over_lease`** needs `batch_bytes`, `link_bps` and `lease_ms`
  (or its default) alone. It is checked whatever else the face holds, so
  a face with a wrong `class` still has it. The reference computed it
  only for a face with no error. A wrong `lease_ms` is reported once,
  and the lint is skipped.
- **`shared_protocol`** needs both faces' link protocols. A list naming
  one zenoh does not is `unknown_protocol`, a wrong value, and the lint
  is skipped for it. The reference read any well-formed list.

The reference is fixed to the text, and three `lints.json` cases pin it
(129 in all); zk2py passed all three as it stood.

## 0.7 — 2026-10-19: a block not enabled, and two faces of one far principal (#769, LH)

The spec-only implementation (zk2py, round 36) read 0.6 cold, passed
every fixture (1414) and all eleven scenario sections (79 checks), and
filed two findings, both gaps, each measured: F-124, a block whose access
control is not enabled, which no check read; and F-125, two faces of one
policy naming one far principal, which nothing described. It also named
two zenoh defaults the text left unstated. The reference confirms each
live and answers them in the text, the model, the generator's warnings,
`zenctl check face`, the fixtures and two new scenario sections. Written
against core 0.32, unchanged (below).

**Changed: `block_disabled`, a check of §2.15's own (F-124; §2.15, §5).**
- **The finding.** zenoh enforces none of a block's rules when its
  `access_control.enabled` is false, or absent, zenoh's default. zk2py
  measured the generator's `deny` block both ways: G and X each got
  `local`'s share, and each one's five puts reached L, while its tool
  reported nothing. §2.15, `lints.json`'s block and §5 never named
  `enabled`, and §5's "whether the router runs the block read" is about
  another case: here the file itself says the block is off. The
  reference's `check face` reported it only through its comparison with
  the plan (`check_acl`'s `disabled`), whose shape the text leaves to the
  generator, so a second implementation need not report it at all.
- **What the reference measured** (scenarios §12, steps 1 and 2): with
  `enabled: false`, and with the line taken out, G and X each received 100
  of `air/frames_total` and of `raw`, all 10 puts of `oper`, all 5
  occurrences of `fault` and of `change`, the owner's two tokens and its
  descriptor, and had their GET and call answered; 5 or 6 of
  `air/dropped_total` and 5 of `frames` each, since the downsampling block
  is not the access control's; all five of each one's puts reached L.
- **The rule.** `block_disabled` (error, `at` `enabled`): a block whose
  `enabled` is false, or absent from the router's configuration. A tool
  reading a configuration reads an absent `enabled` as zenoh does, false.
  It is reported beside the block's other findings, which read the block
  as written, since the block may be wrong too. §5 gains its row.
- **The fixture format.** A `lints.json` block gains an optional
  `enabled`: `false` is the finding, `true` none, and absent means it was
  not read, so every earlier case stands as it was.
- **`zenctl check face`** reports `block_disabled` from §2.15's checks,
  read through zenoh's loader, beside the comparison's `disabled`, which
  stays: it is the generator's. The generator writes `enabled: true`, so
  `every_face_block_passes_its_own_checks` passes in both postures
  unchanged.

**Changed: a far principal of two faces carries both plans' denies, and a
warning says so (F-125; §2.1, §2.4, §5).**
- **The finding.** §2.4 puts every deny of a face's plan into the far
  principal's policy; a zenoh rule cannot be limited to a link protocol;
  and the far principal's subject is narrowed off no face of its own. So
  each of two faces naming one `far` carries the union of both plans'
  denies. zk2py measured it under `deny`, with a second face `sat` to
  `ground` over TLS, exposing none: G over the radio face received
  nothing pushed, its GET and call still answered, and nothing reported
  it. No lint, refusal or rule covered it, so a second implementation had
  to choose between refusing the policy, accepting the union, and
  splitting the far principal's subject per face. zk2py's generator, and
  the reference's (`far_of`, subject id to faces), wrote the union.
- **What the reference measured** (scenarios §13): G over the radio face
  received 0 of `air/dropped_total`, of `frames`, of `oper` and of
  `fault`, where §1's G receives 4 to 6, 4 to 6, 10 and 5; its GET and call
  were answered; it saw no token and no descriptor, and none of its puts
  reached L.
  With `ground-router` as `sat`'s far principal, G received 5, 5, 10 and 5.
  Neither block has a finding: §2.15 reads no plan.
- **The rule.**
  - §2.4: a far principal of two or more faces carries every one of their
    plans' denies on each of them, the stricter, failing closed. A
    deployment that wants each face to carry its own plan names a far
    principal per face.
  - §2.1: `far_of_two_faces`, a warning on the policy itself, one per name
    two or more faces give as `far`, with no face, `at` the name. It reads
    every face whose `far` is a non-empty string, whatever else the policy
    holds. A generator derives each face's plan as before, writes the
    union, and reports the warning with its plans: it refuses nothing,
    since nothing leaks.
  - §5 gains its row; Appendix B notes zenoh-modem's RF and SBD faces, on
    one router with one ground principal.

**Changed: what zenoh 1.10.1 does with what a block omits (§2.15).** Each
measured live (scenarios §12), and stated in "What zenoh 1.10.1 does with
a block", with a tool's reading beside them:
- access control not enabled, or absent, enforces nothing (above);
- a block without `default_permission` is `deny`: the generator's `deny`
  block without it gave G §1's share, and X nothing at all (step 3);
- a rule without `flows` applies to both flows: with `flows` taken out of
  the console's Consume rule for the samples it receives and out of every
  face deny, G received §1's share and nothing the face denies, and of its
  five puts on `air/dropped_total` all reached L, of its five on
  `air/frames_total` none; under the generator's block, none of either
  (step 4).

**Fixtures.** `lints.json`:
- the description: the block's optional `enabled`, `block_disabled`'s
  `at`, and `far_of_two_faces`'s face and `at`;
- cases 30, 31 and 60, two faces each naming `ground`, now also expect
  `far_of_two_faces`; their policies are unchanged. Case 31 was the one
  two-face policy with no finding, and case 119 is one now;
- thirteen new cases, 113 to 125:
  - F-124, 113 to 117: case 62's block with `enabled` false, then true;
    case 63's with it false, beside `zk_not_denied`; cases 80 and 83, a
    block with its subjects under `deny` and under `allow`, with it false;
  - F-125, 118 to 125: the scenarios' policy with `sat` to `ground`, and
    to `ground-router` (no finding); three faces, one name twice; four
    faces, two names twice each; a shared link protocol beside it; an
    unknown link protocol beside it (each key checked alone); an empty
    `far`, compared with none; and case 53's deployment behind two faces
    of one far principal, the plan's warnings beside the linter's.

  `plan.json` is unchanged: its two-face cases name distinct far
  principals, and the warning is the linter's, not the plan's.

**Deliberately not changed.**
- **Splitting the far principal's subject per face.** A subject per face's
  link protocols, each in a policy with that face's denies, would let each
  face carry its own plan. It would change §2.4's "and no further"
  narrowing, and which policy a face subject rides under `allow`. A later
  version may.
- **Refusing two faces of one far principal.** Nothing leaks: the union
  fails closed, and the warning names it.
- **`default_permission` and `flows` in `lints.json`.** A block's
  `default_permission` stays with its subjects, all three or none, and a
  deny's `flows` stays required: a fixture states what it means, and a
  tool reading a router's configuration reads what is absent as zenoh
  does.
- **§5's "Unobservable: whether the router runs the block read".** It
  stays: a file that says the block is off is `block_disabled`, and
  whether the router runs that file is still not on the bus.
- **The core** (0.32). The text is right without it: `block_disabled` is
  a check of the block this text reads, and zenoh's defaults are facts of
  that block, measured here. Core §11 compiles grants into rules and never
  names `enabled`; whether it should say a generator writes access control
  enabled is a core question no finding has asked. The reference's
  generator writes `enabled: true`, a `default_permission` and every
  rule's `flows`.

## 0.6 — 2026-10-19: what a subject in two policies receives (#769, LF)

The spec-only implementation (zk2py, round 34) read 0.5 cold, passed
every fixture (1414) and all eleven scenario sections, §11 over mTLS
among them, and filed one finding, F-122, an ambiguity. Written against
core 0.31, which answers it. No rule, code or fixture of this text
changed.

**Changed: scenarios §11, expected 7 (F-122; core §11.2).**
- **The finding.** In step 7, under `allow`, the face's subject is listed
  in `ground`'s policy and `radio-a`'s. The text expected X to receive
  §1's G's share. zk2py's X received nothing at all: its generator
  compiles `radio-a`'s complement over every message and flow `radio-a`
  holds no grant for, its own stream's `put` toward it among them, and the
  face's subject holds `radio-a`'s denies. The reference leaves a
  principal's own keys out of its complement, so `radio-a`'s policy
  denies none of `radio-a`'s traffic. Neither leaks: X gets less, or the
  same, and no put reaches L.
- **The rule.** Core 0.31 says a generator MAY leave the keys of the
  services a principal runs out of its own complement, and one that
  denies them too conforms. Expected 7 now gives X's share exactly under
  each: §1's G's share under the reference's complement, and nothing at
  all under the stricter one, since §1's G's share is `radio-a`'s traffic
  alone. G's share does not depend on it, since G also matches `ground`'s
  own subject.

**Deliberately not changed.**
- **The checks.** §2.15 reads which policies list a subject, never their
  rules (0.4). The block of step 7 is reported (`face_subject_misplaced`,
  `near_subject_on_face`, `zk_not_denied`, `narrowed_too_far`) whichever
  complement wrote it.
- **The fixtures.** No `lints.json` case carries a rule, and no
  `plan.json` case a principal's complement.
- **One complement for both implementations.** Choosing one would make
  either generator non-conforming for a block that only a misplaced
  subject can show, and that §2.15 already refuses.

## 0.5 — 2026-10-19: one kind of name per subject (#769, LE)

The spec-only implementation (zk2py, round 33) read 0.4 cold, passed
every fixture (1404) and all ten scenario sections, and filed one
finding, F-121: §2.15's `narrowed_too_far` credited a name to a subject
that names it beside a name of the other kind, or as the other kind,
which zenoh matches only for a session presenting both. The reference
confirms both blocks live, finds the same hole in the far principal's
subject, where it leaks, and answers them in the text, the model, the
generator, the fixtures and a new scenario section, run live. Round 33's
mutation test left five mutants alive; four are pinned now. Written
against core 0.30, unchanged (below).

**Changed: a subject naming both kinds binds no one alone (F-121; §2.4,
§2.15, §5).**
- **The finding**, two hand edits of the generator's `allow` block for
  §7's deployment, with `local` narrowed off `tcp` too:
  - (i) a subject naming `ground` as its user name and `local` as its CN,
    in `ground`'s policy;
  - (ii) a subject naming `local` as a CN, off the face, in `local`'s
    policy.

  In each, `local`'s session over TCP, presenting its user name alone,
  matches no subject, and its 5 puts reach the near side. 0.4 reported
  nothing.
- **What the reference measured** (scenarios §11):
  - (i) and (ii): 5 of 5 each, as zk2py measured. With neither subject,
    the same 5, and 0.4's `narrowed_too_far` at `local`.
  - Over mTLS, `local`'s subject naming `local` as its user name and as
    its CN. A session presenting the CN `local` matched it (0 of 5 puts
    reached the near side); one presenting the CN `other`, or over TCP
    with no certificate, matched nothing (5 of 5). That is the subject the
    generator wrote for a principal enrolled with a `user` and a `cn`
    both, and 0.4 passed it.
  - The same hole in the far principal's subject. 0.4 read a subject
    naming `ground` as its user name and `radio-a` as its CN as
    `ground`'s, so `radio-a`'s policy, which listed it, became one of the
    far principal's policies, and a face subject in it and in `ground`'s
    passed. Under `deny`, every session over the face then wrote
    `radio-a`'s keys, 5 of 5 for G and for X, under `radio-a`'s Own grant,
    which no face deny includes.
  - What 0.4 said of it: only `zk_not_denied`, by the accident of
    `radio-a`'s policy holding no `@zk` deny; and nothing at all once that
    policy also lists the face's `zk` rule, while the writes still cross.
  - With `local` in place of `radio-a`, or under `allow`, nothing more
    crossed: `local`'s grants are `ground`'s here, and a subject two
    policies list holds both's rules, deny first.
- **The rule.**
  - *The far principal's subject* names the far principal alone in one
    list and nothing in the other. A subject naming both kinds is *bound
    to another principal*: on the face it is `near_subject_on_face`, and
    it gives the far principal no policy, so a face subject riding its
    policy is `face_subject_misplaced`.
  - `narrowed_too_far` reads names by kind: each user name a subject
    names, each CN, and a face's `far` that no subject names, of a kind
    the block cannot tell. A subject naming no interface and no zid
    answers for a user name when it names it among its user names and no
    CN, or names neither kind; for a CN, the same with the lists swapped.
    One finding per name, whichever kinds fail.
  - §2.4: one principal per subject holds within one list. Across the
    two, one kind of name per subject: a generator writes each subject
    with one list, and refuses a principal given with both on a router
    with a face.
  - The denies' reading is unchanged, and wider than the far principal's
    subject on purpose: a subject naming the far principal beside a name
    of the other kind can match its session, if the session presents
    both. The reference's `matches_far` reads it so.
- **zenoh facts**, in §2.15, each measured: a subject naming both kinds
  matches only a session presenting one of each; a subject two policies
  list holds both's rules, deny first.
- **The generator** (the reference's `zenctl acl gen`), on a router with a
  face:
  - it refuses a principal enrolled with a `user` and a `cn` both, whose
    subject would name both kinds: a generator's refusal, with no code
    (§2.15);
  - it places a face's far principal by its user name or CN alone. It also
    matched the enrollment's `id`, which wrote a far principal's subject
    that 0.4's checks already refused (`near_subject_on_face` at it).

  Neither shape is in a pinned enrollment, so no pinned block changed, and
  `every_face_block_passes_its_own_checks` passes in both postures
  unchanged.
- **Fixtures.** `lints.json`'s case 86 now expects `near_subject_on_face`
  at `ground` and at `mixed`, beside `ops`: both name both kinds. Ten new
  cases, 103 to 112:
  - F-121's (i) and (ii);
  - the far principal's name beside `radio-a` as a CN, in `radio-a`'s
    policy, and case 86's `mixed` in `local`'s, each with a face subject
    riding that policy;
  - one name as both kinds, and a subject naming a user name and a CN
    that answers for neither;
  - round 33's surviving mutants: a face subject in the far principal's
    policies and one more; the far principal's subject and the face's
    both in no policy; `narrowed_too_far` answered by a subject bound to
    no principal; and not answered by one naming an interface or a zid.
- **Scenarios §11** runs each block live, with what it leaks and what a
  tool reports, 0.4's reading among it. `zenctl check face`'s help and its
  findings' text name the new reading.

**Deliberately not changed.**
- **The core** (0.30). Core §11.1 binds a principal "to a user name or a
  certificate CN", and no core grant shape writes a subject naming both;
  §11.3 already says a session matching no subject gets
  `default_permission`. That zenoh matches a subject of both kinds only
  for a session presenting both is a zenoh fact this text measures for
  the blocks it checks.
- **The generator without a face policy** still writes a principal
  enrolled with a `user` and a `cn` as one subject naming both. That is
  core §11's question, not this text's: under `deny` its session must
  present both, and under `allow` one presenting either alone gets
  `default_permission`, as any session matching no subject does (core
  §11.3).
- **A far whose kind the block cannot tell.** A face's `far` is a name,
  and a block shows its kind only through the lists that name it. A name
  some subject binds as a user name is not also checked as a CN: the
  enrollment decides which it is.
- **`at` stays the name.** One `narrowed_too_far` per name, whichever of
  its kinds fail.
- **The checks read a name's session as presenting that name alone.** A
  session presenting a CN may present a user name too, which another
  subject matches; a block cannot tell what else a session presents. 0.4
  read interfaces and zids the same way.
- **An empty list** stays read as absent, and no fixture carries one,
  since zenoh refuses one at load (0.4). Round 33's fifth surviving
  mutant, an empty list read as empty, stays unpinned.

## 0.4 — 2026-10-19: what the subject checks let through (#769, LD)

The spec-only implementation (zk2py, round 32) read 0.3 cold, passed
every fixture and all eight scenario sections, and filed one finding,
F-120: §2.15's two subject checks passed three blocks that §2.4 forbids,
each measured on a router. The reference confirms all three live, finds
two more of the same kind, and answers them in the text, the model, the
fixtures and two new scenario sections, run live. Written against core
0.30, unchanged (below).

**Changed: §2.15 reads which policies list a subject, and reports every
block that breaks §2.4 from what the block shows (F-120; §2.4, §2.15,
§5).**
- **The finding**, three hand edits of 0.3's block for §7's deployment:
  - (a) under `deny`, `local`'s subject bound to `ground` and `local`,
    listed in `local`'s policy: `local`'s credentials over the face got
    `local`'s grants there. `near_subject_on_face` exempted the subject,
    since one of its names was the far principal's;
  - (b) under `allow`, the face's own subject naming `tcp` beside
    `unixsock-stream`: `radio-a`'s credentials over TCP read
    `lab/radio-b`'s `air/dropped_total`. `face_subject_missing` was
    satisfied, since the subject named the face's protocol;
  - (c) under `allow`, `ground`'s subject narrowed to `unixsock-stream`:
    `ground`'s credentials over TCP matched no subject, and their puts on
    an owner's key reached the near side. Neither check looked at the far
    principal's subject.
- **What the reference measured** (scenarios §9, §10):
  - (a): X received 100 of `air/frames_total` and of `raw`, all 5 `change`
    occurrences and both tokens; G kept §1's share, as zk2py measured. The
    reason: zenoh 1.10.1 splits a subject into one combination per value
    of each property, and equal combinations, across subjects, are one
    subject. The `ground` part of the combined subject is `ground`'s own
    subject, whose face denies are evaluated first. With `ground`'s
    subject narrowed to its face (§2.4's MAY), the two combinations
    differ, and G gets `local`'s share too (§9, step 2). "Any allow wins
    across subjects" holds across distinct combinations;
  - (b): all 20 samples, against none under 0.3's block; under `deny`,
    such a subject in `ground`'s policy does the same;
  - (c): all 5 puts. A near principal narrowed off one more link protocol
    does the same: `radio-b`'s credentials over TCP, 5 of 5;
  - two more blocks 0.3 passes: a face subject in a near principal's
    policy, under either posture, gives G and X `local`'s share over the
    face; a face subject that no policy lists, under `allow`, gives them
    everything, their writes to an owner's key included, since zenoh gives
    such a subject `default_permission`.
- **The rule.** §2.15's checks report every block that breaks §2.4, from
  what the block shows: its posture, its subjects, which subjects each
  policy lists, and the face policy. Terms: a subject *on the face*; *the
  far principal's*, naming it alone as its user names or as its CNs; *a
  face subject*, on the face and naming no user name and no CN, whatever
  its id; *the far principal's policies*, those listing one of its
  subjects on the face.
  - `near_subject_on_face`, widened: a subject on the face bound to
    another principal, whatever else it names, the far principal's name
    among them. (a)
  - `face_subject_beyond_face`, new, `at` the subject's id: a face subject
    whose link protocols are absent or name one outside the face's. (b)
  - `face_subject_misplaced`, new, `at` the subject's id: a face subject
    not listed in exactly the far principal's policies, at least one. The
    two new blocks, and §7's step 5, which 0.3 left to the comparison.
  - `narrowed_too_far`, new, with no face, `at` the user name or CN: under
    `allow`, a name the block binds, or a face's `far`, and a link protocol
    outside the faces it is kept off that no subject selects for it. (c),
    a near principal narrowed the same way, and a far principal the block
    gives no subject at all, which a generator refuses to write.
  - `face_subject_missing` is kept, reading the defined face subject that
    names no interface and no zid.
- **The denies a face's block holds**, stated: for each subject that can
  match the far principal's session on the face, the deny rules of every
  policy listing it, each subject's checked alone. The reference has read
  them so since LB, and 0.3 never said it. That reading reports
  `zk_not_denied` for (a) and for the two new blocks, whose stray subject's
  policies hold no `@zk` deny, but only by that accident: a policy that
  denied `@zk` would have passed, and a reading of the far principal's
  policy alone passes them all.
- **§2.4** says it in the generator's terms: one principal per subject;
  narrowed no further than the faces it is kept off; the face's subject
  selected by the face's link protocols alone and listed in exactly the
  far principal's policies; and a face subject a block carries under
  `deny` held to the same rules.
- **zenoh facts**, in §2.15, each measured: the split into combinations;
  a subject that no policy lists; an empty list refused when the
  configuration loads.
- **Fixtures.** `lints.json`'s block carries `policies`, `[{subjects}]`,
  with `default_permission` and `subjects`: all three or none. 0.3's six
  cases with subjects gain `policies`. Two of them change besides: in the
  case of a face subject for one of two link protocols, `local` is now
  selected by every link protocol outside the face, where `tcp` alone
  would be `narrowed_too_far`; the catch-all case puts its subject in a
  policy of its own, and gains `face_subject_beyond_face` and
  `face_subject_misplaced`. Eighteen new cases, two of them the far
  principal's subject in no policy and the far principal with no subject
  at all (review of PR #786).
- **Scenarios §9 and §10** run each block live, with what it leaks and
  what a tool reports; §7's expected 6 now reports step 5.
- **The guesses round 32 listed, settled.**
  - The face subject's id, and every other, is the generator's: an id
    joins a subject to its policies and is where a finding points, and no
    check depends on what it says.
  - An empty list is refused by zenoh 1.10.1 when the configuration loads
    (measured): a generator never writes one, no fixture carries one, and
    a check reads one as absent.
  - A face whose far principal has no subject, and a principal left with
    no link protocol, are the generator's refusals, with no code here.
  - The comparison with the plan has the generator's shape.

**Deliberately not changed.**
- **The core** (0.30). §11.3 already says a session that matches no
  subject gets `default_permission`; that a subject no policy lists gets
  it too, and that equal combinations merge, are zenoh facts this text
  measures for the blocks it checks. No core grant shape writes a subject
  bound to two principals or a subject in no policy (§11.1: one subject
  per principal, each in its policy), so the core's rules do not turn on
  either.
- **Rules' permissions are not read.** The checks read which policies
  list a subject, not what their rules say, so a face subject in a policy
  of its own that only denies, under `deny`, is reported though it grants
  nothing (§10, step 6). Reading permissions would make the check exact
  for that case, at the cost of every rule in the fixture and a longer
  rule for a second implementation to agree on. A face subject is the far
  principal's, or a finding.
- **A subject bound to no principal and off the face**, one selecting near
  link protocols alone, is core §11's (§11.2, F-87), not this text's: no
  session over the face matches it.
- **The denies' reading leaves out** a subject bound to no principal that
  names an interface or a zid, as the reference has since LB. The new
  checks treat it as a face subject, and report it unless it rides
  exactly the far principal's policies, where its denies are the far
  principal's.
- **The plan's shape** (§2.3): no plan member lists the block's subjects.
- **The generator.** It passes every new check in both postures, the
  walkthrough's face in both attachments and the radio's, unchanged: no
  pinned block was blessed.

## 0.3 — 2026-10-19: a near credential on the face, and two revisions of one interface (#769, LC)

The spec-only implementation (zk2py, round 31) read 0.2 cold, passed
every fixture and all six scenario sections, and filed two findings,
F-118 and F-119. The reference answers both in the text, the model, the
generator, the fixtures and two new scenario sections, run live. Written
against core 0.30, unchanged (below).

**Changed: every other credential is kept off the face (F-118; §0, §2.4,
§2.14, §2.15, §5, Appendices A and B).**
- **The finding.** 0.2's §2.4 put the plan's denies into "each subject
  that can match a session on the face", then set the near principals
  aside because they "connect elsewhere": a fact of the deployment that
  the generator cannot see and the router does not enforce. zenoh ties no
  credential to a link, so a subject bound by a user name alone matches
  that principal's session over the face too. zk2py measured `local`'s
  credentials over the face receiving 100 of `air/frames_total` and of
  `raw` and all 5 `change` occurrences, and seeing the owner's tokens,
  while `ground` on the same face received none of them. The reference
  measured the same (scenarios §7, step 2).
- **The rule.** A session over a face that is not the far principal's
  MUST get nothing across the face that the plan denies, and never more
  than the far principal gets there. Who holds a near principal's
  credentials is not a guard: a generator MUST make this hold in the block
  it writes. "The near side's principals connect elsewhere" is gone.
- **The mechanism, measured in both postures** (scenarios §7):
  - every subject bound to a principal is selected by the link protocols
    of §2.1's closed list outside every face whose far principal it is
    not; a principal left with none is refused;
  - under `deny`, that is all: a near credential over the face matches no
    subject, and gets nothing at all, not even what the far principal
    gets;
  - under `allow`, a session that matches no subject is allowed
    everything, a narrowed credential over the face among them (measured,
    its writes to an owner's key included), so each face has a subject of
    its own, selected by its link protocols alone and listed in the far
    principal's policy. A near credential over the face then gets what the
    far principal gets there, its writes to an owner's key denied as the
    far principal's are.
- **What the measurement overturned.** The suggestion was a face subject
  under `allow` carrying the plan's denies alone, in a policy of its own.
  It failed live: under `allow`, a subject is allowed whatever its own
  denies do not include, and any allow wins across subjects, so it undid
  the far principal's own denies for every session over the face, and
  `ground`'s put on `lab/radio-a`'s key reached the near side (step 5).
  0.1's objection to a link subject (F-87's mechanism) was that implicit
  allow, which a deny-only policy still has under `allow`. Listed in the
  far principal's policy, the subject undoes nothing.
- **0.2's MAY**, narrowing the far principal's subject to its face, now
  holds under `deny` only: under `allow`, its session elsewhere would
  match no subject, and be allowed everything.
- **Two checks** (§2.15, errors), given the block's subjects and posture:
  `near_subject_on_face`, at the subject's id, and `face_subject_missing`,
  at the link protocol. They are link.v1's, decided by the block and the
  policy alone, as `zk_not_denied` is. Whether the face's subject rides
  the far principal's policy is the generator's comparison: §2.15 cannot
  tell which of a policy's denies are the far principal's own.
- **§2.14** says a face's link protocol is the face's alone: a near
  principal connecting over it is on the face.
- **Fixtures.** `lints.json`'s block may carry `default_permission` and
  `subjects` (zenoh's `AclConfigSubjects`), together or neither; absent,
  the subjects are not checked. Six new cases.

**Changed: two revisions of one interface (F-119; §0, §2.2, §2.3, §2.7,
§5).**
- **The finding.** An annotation changed between two minors is a review,
  not a breaking change (core §9.8), so one major's minors may differ in a
  resource's exposure, and two services of one deployment may run either
  (core §3.3). The plan's exposure denies are per interface id, the same
  keys for every service, while its downsampling rules and liveness
  evidence are per service, and the text named no revision to derive
  from. §2.7's "What a later minor adds" covered a resource added, not an
  exposure changed. The reference generator (LB) already read several
  revisions fail-closed, silently.
- **The rule.** The plan reads every revision of a listed interface it is
  given together. A resource crosses only where every revision that
  declares it reads `link`, failing closed as §2.2 does on a value that is
  no exposure. A revision that does not declare a resource says nothing of
  it, so a resource a later minor adds is read from the minors that
  declare it. Liveness evidence follows the same reading: a horizon counts
  only where every revision that declares the resource gives one above 0,
  and the least of them is the one the interval is held against (§2.12).
- **The warning**: `exposure_disagrees`, at the resource's name, on a face
  that exposes, so an operator sees why an exposed resource does not
  cross.
- **The fixture format**, kept minimal: an interface in `interfaces`
  (`plan.json`, `lints.json`) is one revision's resources, as before, or
  `{revisions: [resources, ...]}`; two files in `contracts` with one
  interface id are two revisions of it. `plan.json` gains one case (two
  revisions that disagree on a stream, a resource the later one adds, two
  horizons, and a face that exposes none, which says nothing of it);
  `lints.json` two. No 0.2 case changed.
- **Scenarios §8**: two owners running two minors that disagree on
  `air/frames_total`; it crosses for neither. Derived from the newer minor
  alone, it crosses for both, the older minor's owner's `host` stream with
  it.

**Deliberately not changed.**
- **The core** (0.30). §11.3 already says a session that matches no
  subject gets `default_permission`, everything under `allow`, and §11.2
  says why a subject that matches every session undoes per-user denies.
  Selecting a principal's subject by link protocol is a generator's use
  of a subject property the core's grant shapes (§11.1) leave to it, and
  only a constrained face needs it. §8.5's gateway, "the only session on
  its side that talks across the face", is a deployment's shape, which
  §2.4 now enforces rather than assumes.
- **The plan's shape** (§2.3): the narrowed subjects and the face's own
  are the block's, written from the policy's faces and the enrollment's
  principals, which no plan member carries; a plan member listing them was
  considered and left out, as 0.2 left out the declarations a south region
  needs.
- **A carve-out per service, by its minor.** The deployment (core §11.1's
  input) names the interfaces a service implements, not the minor it runs,
  and a service upgrades without its router's block being written again,
  while a router reads keys, never descriptors. Failing closed over every
  revision given holds whichever minor each service runs.
- **A warning for two horizons**: the least is read, and
  `interval_over_horizon` reports what it costs.
- **Exposure per principal**: still deferred (§2.14).

## 0.2 — 2026-10-18: what compiling it into live routers found (#769, LB)

The reference generator (`zenctl acl gen --face-policy`, LB) and the
spec-only implementation (zk2py, round 30) each compiled the profile into
zenoh 1.10.1 routers and ran the six scenario sections. zk2py passed every
fixture (119) and filed two findings, F-116 and F-117; the reference
confirmed F-117 in its own §2. Written against core 0.30, which corrects
§8.5 for F-117. No fixture case changed.

**Changed: readers subscribe to what they are granted (F-116;
scenarios.md Conventions, §5).**
- **The finding.** G and L subscribed to `zk2/lab/*/**`, and §5's G
  subscribed to and GOT `…/radio.v1/state/**`, under a `deny` block
  compiled from per-binding grants. zenoh refuses a subscription or a GET
  that no grant includes (core §11.3): zk2py measured a subscriber on
  `a/**` with allows on `a/b` and `a/c` receiving nothing, while a GET of
  `a/b` is answered. As written, the scenarios' readers received nothing.
- **The fix.** Each reader subscribes to, and GETs, the selectors its
  grants name, one per resource, and the Conventions say why. The
  enrollment is the reference's (`examples/zk2/link/radio.enrollment.toml`):
  one tool bound on `lab/*`, run by `ground`, `ground-router` and `local`.
- **The text** implied nothing wider: no rule of v1.md lets a reader
  subscribe beyond its grants, and §2.7's crossing is about an owner's
  pushes. It is unchanged.
- **No fixture:** a subscription's admission is a router's, which only a
  scenario runs.

**Changed: what the south region's declaration grant covers (F-117;
§2.5, scenarios.md §2).**
- **The finding.** §2.5, restating core §8.5, required the far principal
  to be granted `declare_queryable` on egress "for what its side may
  query". Granted on its bindings' selectors only, zk2py's G had its call
  answered and its state GET not; granted on the owners' `state/**` and
  `@state/**` as well, both; with no grant, neither. The reference
  measured the same in its live §2.
- **Why.** An owner's state queryables are declared over its interface's
  `state/**` and `@state/**` (core S2), and zenoh checks a declaration by
  inclusion against that key expression (core §11.3). A selector over one
  resource, `…/state/peers/*`, does not include `…/state/**`, so the
  declaration never reaches the far router, which then routes no GET.
- **The rule.** The grant covers every queryable the far side may query,
  **as its owner declares it**: the state queryables for a state GET, an
  operation's queryable over its template's pattern for a call (core §5:
  `@op/set/*` for a call of `set/duty`, the same inclusion), and an
  archive's over its `@state` forms. It is still a grant, not a deny: a
  declaration carries a key expression and no value, and what crosses in
  the replies is still the Consume and Call grants'.
- **Scenarios §2** say how the owner serves, and add the control F-117
  measured: the grant over G's own selectors answers the call and not the
  GET.
- **No fixture:** the grant depends on the enrollment's bindings and calls
  (core §11.1's input), which no fixture of this profile carries; the
  plan's shape is unchanged. The reference pins it in its generator's
  fixtures (`walkthrough.face-south-deny.json5`) and runs it in §2.

**Changed: the owner's traffic fits its 5 s (scenarios.md Conventions).**
0.1 had `oper` put 10 times "one a second apart" within 5 s, which
takes 9 s. It is one every half second, as the reference runs it.

**Changed: pointers.** §2.15 names `zenctl check face` beside `acl gen
--check`; Appendix B names the walkthrough's face policy and the radio
example (`examples/zk2/link/`).

**Deliberately not changed.**
- **The plan's shape** (§2.3) and every fixture: neither finding is the
  plan's. A plan member listing the declarations a south region needs was
  considered and left out: they follow from grants, not from the policy
  and the contracts, and the plan would carry an enrollment's facts.
- **The far principal's withheld grants.** The reference withholds the far
  principal's presence, history and admin-space grants, which the plan's
  `zk`, `adv` and `admin` denies would cut anyway (§2.4: deny is evaluated
  first within a policy). That is a generator's tidiness, with the same
  effect, and no rule here asks for it.
- **The core's constrained scenarios** (`constrained.md` §6) keep their
  words: a ground client's state GET "works" there with the declaration
  grant core 0.30 now spells out.

## 0.1 — 2026-10-18: the first text, draft (#769, LA)

`link.v1` is a profile of #613. Its record was zenoh-modem's sketch
(`examples/zk2/zenoh-modem/link.md`, #623), with the README's gaps 1, 10
to 14 and 20, architecture D14, D15, D9c and E3, spike S3, and core §8.5,
which names `link.v1` as the standard face configuration and needs only
two facts of it: that a face is constrained, and its maximum outage. The
maintainer decided on #769 that the face policy is a deployment file, that
the rate lives in it, that the contract keeps only `link.exposure`, that
exposure per principal is not in 0.1, that a crossing counter SHOULD carry
its epoch, and that core §8.5's "SHOULD … a third of the lease" wins over
architecture r4's "MUST cross well within". It is written against core
0.29, which publishes its vocabulary in place of Appendix D's interim row
and drops `link.downsample_ms`.

**Stated.**
- **The face policy** (§2.1): a TOML 1.0 file per router, its fields one
  by one (`system`, `services`, and per face `far`, `attach`, `region`,
  `select.link_protocols`, `class`, `expose`, `interval_s`,
  `max_outage_s`, `batch_bytes`, `link_bps`, `lease_ms`), and the findings
  a linter reports, each with a code, a severity, a face and a key.
- **`link.exposure`** (§2.2, §4): `host` or `link`, `host` when absent,
  any other value read as `host`; ignored on an operation.
- **The plan** (§2.3): the denies, the downsampling block, the
  attachment, the window floors and the liveness evidence of each face,
  in an order a second implementation reproduces.
- **Where the denies go** (§2.4): into every policy that can match a
  session on the face, since any allow wins across subjects; no subject
  of the face's own.
- **The attachment** (§2.5), router to router refused; **the `@zk` deny**,
  never omitted, beside `@adv` and the admin space (§2.6).
- **What crosses** (§2.7): the pushes of crossing resources; every other
  push denied, wholesale per kind token when nothing of it crosses, else
  per interface, carved into subtrees and names, template by template
  below a literal beside a parameter, with an overlap reported where
  zenoh cannot carve.
- **Downsampling** (§2.8): one rule per concrete key of a listed service,
  egress, `put`, 1/`interval_s` Hz; streams only; a templated exposed
  stream denied; every service whose exposed streams reach the face
  listed.
- **Events and requests** (§2.9): an exposed event whole, never
  downsampled; requests by grant, never by exposure.
- **The batch against the lease** (§2.10, a SHOULD), **the outage as every
  listed owner of state's window floor** (§2.11), **liveness from the
  plan's evidence** (§2.12), **counters' epochs** (§2.13, a SHOULD), what
  a deployment must not expect (§2.14), and the checks of a router's block
  (§2.15).
- **Evidence:** `conformance/exposure.json` (31 cases),
  `conformance/plan.json` (11 cases, two of them zenoh-modem's faces from
  its own contracts) and `conformance/lints.json` (77 cases), and six
  scenario sections for the generator and live routers (LB).

**How the fixtures' expectations were decided.** Each was written by hand
from this text before the model existed, and the model (`zenkey-model`'s
`link`) was then written to the text. It agreed with every expectation on
its first run.

**Decided here, where the record was silent.**
- **`far` in the policy.** The record's face file named no principal.
  §2.4 puts the denies in the far principal's policy, as the reference
  generator does for `--far`, so the policy names it.
- **No subject of the face's own** (§2.4). The sketch, and v1's RFC 09
  §4, selected the face by a link-protocol subject and repeated its denies
  in each operator's own. Under `allow`, such a subject's allows undo the
  far principal's denies (core §11.2, F-87), and core §11.3 already makes
  every session on a face a principal's.
- **Requests by grant, not exposure** (§2.9). The sketch denied every
  operation on the face and carved an operator's calls out of its own
  policy. Exposure is what crosses unasked; a request is asked once, and
  §11's Call and Consume grants say who may ask. This is also what
  `config.v1` §2.19 expects: its status GET crosses, its subscription not.
- **Only streams are downsampled** (§2.8). A downsampled state can leave
  the far side holding a superseded value as current, and a downsampled
  event loses occurrences no reader can tell from silence.
- **By interface, not by service** (§2.7). The sketch denied its
  companions' resources by service. Exposure is the contract's, for every
  service that implements it.
- **Wholesale per kind token** when nothing of that token crosses (§2.7),
  which fails closed for interfaces the policy does not know.
- **A rest parameter's pattern is `*/**`** (§0), so that a deny of
  `x/{p...}` leaves the key `x`, as the template does.
- **The window floor for every listed owner of state** (§2.11), not only
  for archived state as core S3 says: a far late joiner seeds by a GET
  after each outage, and a delete the window lost leaves its key standing.
- **The interval against the horizon** (§2.12): a crossing stream is fresh
  across the face only when `interval_s` ≤ ttl/2.

**Deliberately not stated.**
- **No exposure per principal** (the sketch's gap 20): deferred, as
  decided. Requests by grant cover the operator's calls and GETs.
- **No standard contract.** Nothing of the profile is on the bus.
- **No face subject, no `@op` deny, no `@media` or `@blob` deny**: §2.4,
  §2.9, and Appendix A.
- **No comparison of a block with the whole plan** in the model: it is a
  generator's check of its own output (`zenctl acl gen --check`, LB).
- **No counter type.** It is `telemetry.v1`'s (§2.13).
- **zenoh-modem's contract is unchanged.** Its four link streams declare
  no horizon, so the plan lists no liveness evidence for its RF face
  (Appendix B); its sketch is marked superseded and kept as the record.
