# `link.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. They are a generator's and a
router's: the reference runs them as tests of the tool that compiles a
face policy into a router's configuration (`zenctl acl gen`), against live
routers, one test per section, named after it (#769, LB; §7 and §8 since
0.3, LC; §9 and §10 since 0.4, LD; §11 since 0.5, LE).

**Conventions.**
- **Section numbers** in parentheses (§2.7) are v1.md's rules; core
  sections say "core", and `freshness.v1`'s say so.
- **The bus.** N is the near router, zenoh 1.10.1, with timestamping on
  and `usrpwd` authentication. It listens on `tcp/127.0.0.1:<port>`, the
  near side, and on `unixsock-stream/<path>`, the face. Every session
  authenticates as a user of its own. Timeouts are 1 s unless stated.
- **The generator** compiles N's `access_control` block, under `deny`
  unless a section says `allow`, from the enrollment below, the contract
  and the face policy, with the plan's denies in the far principal's
  policy and every other principal's subject narrowed off the face (§2.4,
  0.3), the plan's `downsampling` block beside it, and N's `gateway` block
  for a south region (§2.5). "The block" is what it wrote.
- **The contract** is `radio.v1`:

  ```toml
  [interface]
  name  = "radio"
  major = 1
  minor = 0
  uses  = ["link.v1", "freshness.v1"]

  [resources."air/dropped_total"]               # crosses, downsampled
  kind        = "stream"
  type        = { raw = "text/plain" }
  annotations = { "link.exposure" = "link", "freshness.ttl_s" = 4 }

  [resources."air/frames_total"]                # stays home
  kind = "stream"
  type = { raw = "text/plain" }

  [resources.frames]                            # an @stream that crosses, downsampled
  kind        = "stream"
  explicit    = true
  type        = { raw = "application/octet-stream" }
  annotations = { "link.exposure" = "link" }

  [resources.raw]                               # an @stream that stays home
  kind     = "stream"
  explicit = true
  type     = { raw = "application/octet-stream" }

  [resources.oper]                              # a state that crosses whole
  kind        = "state"
  type        = { raw = "text/plain" }
  annotations = { "link.exposure" = "link", "freshness.ttl_s" = 4 }

  [resources."peers/{peer}"]                    # a state that stays home; a GET crosses by grant
  kind        = "state"
  type        = { raw = "text/plain" }
  params      = { peer = "string" }
  cardinality = 16

  [resources.fault]                             # an event that crosses whole
  kind        = "event"
  type        = { raw = "text/plain" }
  rate        = "burst(3600/h)"
  retention   = "1h"
  annotations = { "link.exposure" = "link" }

  [resources.change]                            # an event that stays home
  kind      = "event"
  type      = { raw = "text/plain" }
  rate      = "burst(3600/h)"
  retention = "1h"

  [resources.reset]                             # an operation, called by grant
  kind     = "operation"
  request  = { raw = "text/plain" }
  response = { raw = "text/plain" }
  ```
- **The owner** is `lab/radio-a` (user `radio-a`), on the near side,
  implementing `radio.v1` with every resource exposed, its tombstone window
  as each section says. §3 adds `lab/radio-b` (user `radio-b`), the same.
- **The face policy**, unless a section changes it:

  ```toml
  system   = "lab"
  services = ["radio-a"]

  [face.radio]
  far          = "ground"
  attach       = "client"
  select       = { link_protocols = ["unixsock-stream"] }
  class        = "constrained"
  expose       = "link"
  interval_s   = 1
  max_outage_s = 6
  batch_bytes  = 1000
  link_bps     = 1000000
  ```
- **The enrollment** (informative; core §11.1's input; the reference's is
  `examples/zk2/link/radio.enrollment.toml`): the services `lab/radio-a`
  and `lab/radio-b`; a tool that binds every stream, state and event of
  `radio.v1` on `lab/*`, and calls `reset` there; the principals `ground`,
  `ground-router` (§2) and `local`, each running it. Every grant a
  resource could need is there, so that what the face denies is the
  face's.
- **G** is the far session, user `ground`, a client of N over the face.
  **L** is the control, user `local`, a client of N on the near side.
  Each subscribes to the selectors its grants name, one per stream, state
  and event resource of `radio.v1`, `zk2/lab/*/radio.v1/<token>/<pattern>`
  (`…/stream/air/dropped_total`, `…/@stream/frames`, `…/state/peers/*`,
  `…/events/fault/*` and the rest), and holds a liveliness subscriber on
  `zk2/lab/*/@zk/**`, before the owner starts. Each records every
  delivery's arrival, key and kind. Under `deny`, zenoh admits a
  subscription or a GET only when a grant includes its key expression
  (core §11.3): a reader subscribed wider than its grants, to
  `zk2/lab/*/**`, would receive nothing at all, so the readers subscribe
  exactly to what they are granted (0.2, F-116).
- **"The owner's traffic"**: for 5 s, the owner publishes
  `air/dropped_total` and `air/frames_total` at 20 Hz, `frames` and `raw`
  at 20 Hz with 1 KB payloads, puts `oper` 10 times, one every half
  second, each a new value, puts `peers/x`, and publishes `fault` and
  `change` 5 times each, all within one second.

## §1 A face compiled and enforced, the far side a client (§2.4–§2.9)

**Steps.**
1. The generator compiles the block, and N starts with it. The owner
   starts.
2. The owner's traffic.
3. G GETs `zk2/lab/radio-a/radio.v1/state/peers/x` as core S4 says, and
   calls `zk2/lab/radio-a/radio.v1/@op/reset` as core O1 says. L does the
   same.
4. A tool checks the block (§2.15).

**Expected.**
1. The block holds the plan: its far principal's policy carries `zk`,
   `adv`, `admin` and the exposure denies, and the downsampling block has
   one rule per crossing stream of `lab/radio-a` at 1 Hz, as `plan.json`'s
   third case derives them, with this policy's attachment and interval.
   Every subject but `ground`'s is selected by the link protocols other
   than `unixsock-stream` (§2.4, 0.3).
2. L receives every sample, put and occurrence, about 100 of each stream.
   G receives:
   - `air/dropped_total` 4 to 6 times, and `frames` 4 to 6 times: one per
     second, the first at once (§2.8);
   - nothing of `air/frames_total`, nor of `raw` (§2.7);
   - all 10 puts of `oper`: a state crosses whole (§2.7);
   - all 5 occurrences of `fault`, though they came within one interval,
     and nothing of `change` (§2.9).
3. G's GET is answered with `peers/x`'s value, and its call with the
   operation's reply: requests cross by grant (§2.9). L's are answered
   too.
4. L sees the owner's instance and interface tokens. G sees none, and no
   descriptor reaches it (§2.6).
5. The tool reports no finding.

## §2 A far router in a south region (§2.4, §2.5, §2.9)

**Setup.** As §1, with `attach = "south-region"`, `region = "ground"`
and `far = "ground-router"`. The far router F (user `ground-router` on N)
names its region `ground`, and attaches to N over the face. G is a client
of F, not of N. The enrollment's principal `ground` becomes
`ground-router`, with the same bindings and calls.

The owner serves its state as core S2 says, one queryable over
`…/radio.v1/state/**`, and `reset` over its own key (core §5). The block
grants the far principal `declare_queryable` on egress over both, as the
owner declares them (§2.5, core §8.5, 0.30).

**Steps.**
1. As §1, steps 1 to 3.
2. **Control:** the generator compiles the block without the far
   principal's `declare_queryable` egress grant (core §8.5), and G repeats
   its GET and its call.
3. **Control:** the grant covers G's own GET and call selectors only,
   `…/state/peers/*` and `…/@op/reset`, and not the owner's
   `…/radio.v1/state/**` (0.2, F-117). G repeats its GET and its call.
4. **Control:** the policy says `attach = "router"`.

**Expected.**
1. The block's `gateway` is the plan's: N's own peers and clients south,
   then the region `ground`. G receives what §1's G receives, sees no token
   and no descriptor, and its GET and its call are answered.
2. Without the grant, G's GET and its call get no reply: F learns of no
   queryable to route them to (core §8.5).
3. G's call is answered and its GET is not: a declaration is checked by
   inclusion against the queryable's own key expression, which the GET's
   selector does not include (core §11.3).
4. The generator refuses the policy (`router_attach`), and writes no
   block.

## §3 One rule per key, and a service not listed (§2.8)

**Setup.** As §1, with both owners, `lab/radio-a` and `lab/radio-b`.

**Steps.**
1. With `services = ["radio-a", "radio-b"]`, both owners publish
   `air/dropped_total` at 20 Hz for 5 s.
2. **Control:** N runs a hand-written block, the generated one with its
   downsampling rules replaced by one,
   `zk2/lab/*/radio.v1/stream/air/dropped_total` at 1 Hz. Both owners
   publish as in step 1. A tool checks that block.
3. With `services = ["radio-a"]`, both owners publish as in step 1.

**Expected.**
1. The block has a rule for each owner's key. G receives 4 to 6 samples
   from each owner: each rule has its own timer.
2. G receives 4 to 6 samples in all, from the two owners together: one
   timer for every key the rule covers. The tool reports
   `wildcard_downsampling`.
3. G receives 4 to 6 samples from `lab/radio-a`, and about 100 from
   `lab/radio-b`: an exposed stream of a service the policy does not list
   is in no rule, and crosses at its owner's rate. The deployment's MUST
   (§2.8) is what keeps this from happening.

## §4 An event that must not cross (§2.7, §2.9)

zenoh-modem's sketch denied its events under their draft-0 keys
(`…/stream/oper_change/**`), which draft 1's `…/events/<template>/<ulid>`
no longer match. This section shows the plan's keys hold, and the
sketch's would not.

**Steps.**
1. The owner publishes `change` 5 times and `fault` 5 times, all within
   one second.
2. **Control:** N runs the generated block with its `events:radio.v1`
   deny replaced by the sketch's spelling,
   `zk2/*/*/radio.v1/stream/change/**`. The owner publishes as in step 1.
3. The policy says `expose = "none"`, with no `interval_s`. The owner
   publishes as in step 1.

**Expected.**
1. G receives the 5 occurrences of `fault`, each on its own
   `…/events/fault/<ulid>` key, none dropped by the 1 s interval, and none
   of `change`. L receives all 10.
2. G receives the 5 occurrences of `change` too: the sketch's deny covers
   no event key.
3. G receives no occurrence at all. The plan denies `zk2/*/*/*/events/**`.

## §5 A state window raised to the outage (§2.11)

**Setup.** As §1. G is a late joiner (core §4.2): it keeps the members it
merged across an outage, as a consumer's cache does, and seeds again when
the face is back. The reference closes G's session for the outage and
opens a new one, keeping G's merged members.

**Steps.**
1. The owner, configured with a tombstone window of 6 s, the plan's floor,
   puts `oper` and `peers/x`. G seeds: it subscribes to its granted
   selectors over `lab/radio-a`'s state, `…/radio.v1/state/oper` and
   `…/radio.v1/state/peers/*`, then GETs each as core S4 says (0.2: never
   `state/**`, which no grant includes, F-116). The face denies the
   subscription to `peers/*`, which does not cross; the GET crosses.
2. The face goes down for 4 s. Meanwhile the owner deletes `oper` and
   `peers/x`.
3. The face comes back, and G seeds again.
4. **Control:** steps 1 to 3 again, the owner's window configured at 2 s.
   A linter checks the policy with that window (`windows`).

**Expected.**
1. G holds both members.
2. G receives nothing.
3. G's GET is answered with a `reply_del` for each, with the deletion's
   stamp (core S3), and G's merge removes both.
4. G's GET answers nothing for the two members, and its merge leaves both
   standing: a lost delete (core §4.2). The linter reports
   `window_under_outage` for `lab/radio-a`.

## §6 Liveness across the face (§2.12)

**Steps.**
1. The owner publishes `air/dropped_total` at 20 Hz, and keeps `oper`
   current, re-put every 2 s (`freshness.v1` §2.4), for 10 s. A tool on
   G's side judges `lab/radio-a`'s liveness across the face at 5 s
   (`freshness.v1` §2.9, §5), over the plan's evidence.
2. The owner closes its service. The tool judges again 6 s later.
3. The policy says `expose = "none"`, with no `interval_s`. The owner
   publishes as in step 1, and the tool judges at 5 s.

**Expected.**
1. The plan's evidence is `radio.v1/state/oper` and
   `radio.v1/stream/air/dropped_total`. Both are fresh at G, so the tool
   reports the service live.
2. Both are stale. The tool reports liveness unobservable, never down
   (core R7).
3. The plan lists no evidence, and G receives nothing of the owner. The
   tool reports liveness unobservable from the start, whatever the owner
   publishes.

## §7 A near principal's credentials on the face (§2.4)

zenoh ties no credential to a link: a subject bound by a user name alone
matches that principal's session over the face too (0.3, F-118). This
section shows that 0.2's block let a near credential across the face
carrying its own grants, and that 0.3's keeps it to nothing under `deny`
and to the far principal's share under `allow`.

**Setup.** As §1. X is a client of N over the face with `local`'s
credentials. Each of G, X and L subscribes as the Conventions say, to the
console's selectors under either posture.

**Steps.**
1. The owner's traffic, under the block. G and X each GET
   `…/state/peers/x`, call `…/@op/reset`, read `zk2/lab/*/@zk/**` by
   liveliness and GET the owner's descriptor. Then, the owner silent, each
   puts five times on `zk2/lab/radio-a/radio.v1/stream/air/dropped_total`,
   a key only `lab/radio-a` may write (core §11.1), and L counts what
   reaches it.
2. **Control:** 0.2's block, the same with no subject narrowed. X as in
   step 1.
3. Under `allow`: as step 1.
4. **Control:** under `allow`, the subjects narrowed and no subject of the
   face's own. X as in step 1.
5. **Control:** under `allow`, the face's subject in a policy of its own,
   holding the plan's denies alone. G as in step 1.
6. A tool checks the blocks of steps 1 to 5 (§2.15).

**Expected.**
1. Every subject but `ground`'s is selected by `tcp`, `udp`, `tls`,
   `quic`, `serial`, `unixpipe`, `vsock` and `ws`. G receives what §1's G
   receives. X matches no subject: it receives nothing at all, sees no
   token and no descriptor, and gets no reply. Neither's puts reach L. L
   receives every sample, put and occurrence.
2. X matches `local`'s subject over the face and gets its grants there:
   about 100 of `air/frames_total` and of `raw`, all 5 occurrences of
   `change`, the owner's two tokens, and its GET and call answered. The
   face's downsampling still applies to it: it selects by link protocol.
3. The block has a subject `face:radio`, selected by `unixsock-stream`
   alone, which `ground`'s policy lists beside `ground`. G and X each
   receive what §1's G receives, see no token, and have their GET and call
   answered. Neither's puts reach L: `ground`'s deny of other services'
   writes holds for both.
4. X matches no subject and is allowed everything: as in step 2, and its
   five puts reach L.
5. G receives what §1's G receives, and its five puts reach L: the face
   subject's policy allows what its denies do not include, and any allow
   wins across subjects, so `ground`'s own deny of other services' writes
   is undone.
6. Steps 1 and 3: no finding. Step 2: `near_subject_on_face` for
   `ground-router`, `local`, `radio-a` and `radio-b`. Step 4:
   `face_subject_missing` at `unixsock-stream`. Step 5:
   `face_subject_misplaced` at the face's subject (0.4; 0.3 left it to the
   comparison with the plan, which names the policy too).

## §8 Two minors that disagree on a stream's exposure (§2.3, §2.7)

**Setup.** As §1, with both owners, `services = ["radio-a", "radio-b"]`.
The generator is given `radio.v1` minor 0, the Conventions' contract, and
minor 1, which differs from it only in `air/frames_total`'s annotations,
`{ "link.exposure" = "link" }`: an annotation changed, a review (core
§9.8). `lab/radio-a` runs minor 1 and `lab/radio-b` minor 0 (core §3.3);
N reads keys, never descriptors.

**Steps.**
1. The generator compiles the block from both revisions. Both owners
   publish `air/frames_total` and `air/dropped_total` at 20 Hz for 5 s.
2. **Control:** the block from minor 1 alone. Both owners publish as in
   step 1.

**Expected.**
1. The plan warns `exposure_disagrees` at
   `radio.v1/stream/air/frames_total`, and its denies and rules are minor
   0's: `air/frames_total` denied, and no rule for it. G receives nothing
   of `air/frames_total` from either owner, and 4 to 6 samples of
   `air/dropped_total` from each. L receives about 100 of
   `air/frames_total` from each.
2. The plan has a rule for each owner's `air/frames_total`, and no deny of
   it: G receives 4 to 6 from each owner, `lab/radio-b`'s among them,
   though the minor it runs keeps it home (F-119).

## §9 Who a subject binds (§2.4, §2.15)

A subject that names the far principal and a near one binds the near one
too, and a principal narrowed further than §2.4 narrows it matches no
subject where it is narrowed off (0.4, F-120). Each step is a hand edit
of the generator's block.

**Setup.** As §7, with G, X and L as §7 says. A session "over TCP" is a
client of N on the near side, with the credentials named.

**Steps.**
1. Under `deny`, `local`'s subject replaced by one bound to `ground` and
   `local` (`usernames = ["ground", "local"]`, no link protocol), listed
   in `local`'s policy. The owner's traffic; G and X as §7's step 1.
2. **Control:** the same, `ground`'s subject narrowed to `unixsock-stream`
   (§2.4's MAY under `deny`). As step 1.
3. Under `allow`, `ground`'s subject narrowed to `unixsock-stream`. A
   session over TCP with `ground`'s credentials puts five times on
   `zk2/lab/radio-a/radio.v1/stream/air/dropped_total`, and L counts what
   reaches it.
4. Under `allow`, `radio-b`'s subject narrowed off `tcp` too. A session
   over TCP with `radio-b`'s credentials, as step 3.
5. **Control:** the generator's `allow` block. Steps 3 and 4's sessions,
   as step 3.
6. A configuration whose subject names an empty list, `link_protocols: []`
   or `usernames: []`, is loaded as a router's configuration is, beside
   the same subject without the list.
7. A tool checks the blocks of steps 1 to 4 (§2.15).

**Expected.**
1. X matches the subject bound to both, and gets `local`'s grants across
   the face: about 100 of `air/frames_total` and of `raw`, all 5
   occurrences of `change`, the owner's two tokens, and its GET and call
   answered; the face's downsampling still applies. G
   receives what §1's G receives: zenoh splits a subject into one
   combination per value, and equal combinations are one subject, so the
   subject's `ground` combination is `ground`'s own subject, whose deny is
   evaluated first. Neither's puts reach L.
2. The two `ground` combinations differ, G matches both, and any allow
   wins: G gets `local`'s share too, as X does. Neither's puts reach L.
3. The session matches no subject, and all five puts reach L.
4. The same: all five reach L.
5. None of either session's puts reaches L.
6. Each empty list is refused at load; the subject without it loads.
7. Steps 1 and 2: `near_subject_on_face` at `local`, and `zk_not_denied`,
   since that subject can match `ground`'s session and its policy holds no
   `@zk` deny. Step 3: `narrowed_too_far` at `ground`, step 4 at
   `radio-b`, each with no face. The generator's `allow` block with
   `ground`'s subject removed, which no generator writes (§2.4), whose
   session over TCP matches no subject as step 3's does:
   `narrowed_too_far` at `ground`, and `face_subject_misplaced` at the
   face's subject, since the far principal has no policy.

## §10 The face's own subject (§2.4, §2.15)

A face subject that selects more than the face's link protocols, or rides
another policy than the far principal's, or none, hands sessions what
§2.4 forbids (0.4, F-120).

**Setup.** As §9.

**Steps.**
1. Under `allow`, the face's subject selected by `tcp` beside
   `unixsock-stream`; under `deny`, a subject bound to nothing and selected
   by both, listed in `ground`'s policy. `lab/radio-b` publishes
   `air/dropped_total` 20 times; a session over TCP with `radio-a`'s
   credentials and L each subscribe to that key alone, a read none of
   `radio-a`'s grants names.
2. **Control:** the generator's `allow` and `deny` blocks. As step 1.
3. Under `deny`, a subject selected by `unixsock-stream` alone and bound to
   nothing, listed in `local`'s policy. The owner's traffic; G and X as
   §7's step 1.
4. Under `allow`, the face's subject moved from `ground`'s policy to
   `local`'s. As step 3.
5. Under `allow`, the face's subject in no policy. As step 3.
6. Under `deny`, the face's subject in a policy of its own, holding the
   plan's denies alone (§7's step 5, under `deny`). As step 3.
7. A tool checks the blocks of steps 1 and 3 to 6 (§2.15).

**Expected.**
1. Under either posture, the `radio-a` session receives all 20 samples,
   with `ground`'s grants through the subject. L receives all 20.
2. The `radio-a` session receives none, under either posture.
3. G and X each get `local`'s share there, as §9's X does: every session
   over the face matches the subject, and any allow wins. Neither's puts
   reach L.
4. The same as step 3.
5. G and X each get `local`'s share there, and all five of each one's puts
   reach L: zenoh gives a subject no policy lists `default_permission`,
   everything under `allow`.
6. G receives what §1's G receives; X receives nothing, sees no token and
   gets no reply; neither's puts reach L. A policy that only denies grants
   nothing under `deny`.
7. Step 1, in each posture: `face_subject_beyond_face` at the subject.
   Steps 3 to 5: `face_subject_misplaced` at the face's subject, and
   `zk_not_denied`, since it can match `ground`'s session and the policies
   listing it hold no `@zk` deny. Step 6: `face_subject_misplaced` alone,
   though the block grants nothing more: §2.15 reads which policies list a
   subject, not their rules.

## §11 A subject naming both kinds (§2.4, §2.15)

zenoh matches one value of each property a subject names, so a subject
naming user names and CNs matches only a session presenting one of each.
0.4 read such a subject as binding each name it lists, and as the far
principal's when one of its lists named the far principal alone (0.5,
F-121). Each step but step 4 is a hand edit of the generator's block.

**Setup.** As §9. For step 4, N also listens on
`tls/127.0.0.1:<port>`, on the near side, with mTLS: its certificate and
a client certificate per CN, `local` and `other`, are signed by one CA
minted for the run. A session over TLS presents its certificate's CN
beside its user name.

**Steps.**
1. F-121 (i). Under `allow`, `local`'s subject narrowed off `tcp` too,
   and a subject naming `ground` as its user name and `local` as its CN,
   with no link protocol, listed in `ground`'s policy. A session over TCP
   with `local`'s credentials puts five times on
   `zk2/lab/radio-a/radio.v1/stream/air/dropped_total`, and L counts what
   reaches it.
2. F-121 (ii). Under `allow`, `local`'s subject narrowed off `tcp` too,
   and a subject naming `local` as a CN, selected by the eight near link
   protocols, listed in `local`'s policy. As step 1.
3. **Control:** under `allow`, `local`'s subject narrowed off `tcp`, and
   nothing else; then the generator's `allow` block. As step 1.
4. Under `allow`, `local`'s subject naming `local` as its user name and as
   its CN, as a generator given both writes it. Over TLS, a session with
   `local`'s credentials presenting the CN `local`, then one presenting
   the CN `other`; then one over TCP with no certificate. Each as step 1.
   **Control:** the generator's `allow` block, the same three sessions.
5. Under `deny`, a subject naming `ground` as its user name and `radio-a`
   as its CN, listed in `radio-a`'s policy, and a subject selected by
   `unixsock-stream` alone, bound to nothing, listed in `ground`'s policy
   and `radio-a`'s. The owner's traffic; G and X as §7's step 1.
6. As step 5, with `local` in place of `radio-a`.
7. As step 5, under `allow`: the face's subject listed in `radio-a`'s
   policy too.
8. As step 5, with `radio-a`'s policy also listing the face's `zk` rule.
   G and X each put five times as in §7's step 1, and L counts what
   reaches it.
9. A tool checks the blocks of steps 1 to 8 (§2.15).

**Expected.**
1. The session matches neither `local`'s subject, which `tcp` no longer
   selects, nor the added one, which it presents no CN for: all five puts
   reach L.
2. The same: the added subject names `local` as a CN, and the session
   presents it as a user name. All five reach L.
3. All five reach L; under the generator's block, none does.
4. Over TLS with the CN `local`, the session matches `local`'s subject,
   and none of its puts reaches L. With the CN `other`, or over TCP with
   no certificate, it matches no subject: all five reach L. Under the
   generator's block, `local`'s subject names the user name alone, and
   none of the three sessions' puts reaches L.
5. G and X each match the face's subject, whose rules are `ground`'s and
   `radio-a`'s together, deny first: each receives what §1's G receives,
   and all five of each one's puts reach L, under `radio-a`'s Own grant,
   which no face deny includes.
6. G and X each receive what §1's G receives, and neither's puts reach L:
   `local`'s grants are `ground`'s here, and `ground`'s face denies hold
   over them.
7. G and X each receive what §1's G receives, and neither's puts reach L:
   under `allow` every rule is a deny, and a subject two policies list
   holds both's.
8. All five of each one's puts reach L: denying `@zk` in `radio-a`'s
   policy changes nothing of step 5's writes.
9. 0.4 reported nothing for steps 1, 2, 4 and 8, `narrowed_too_far` at
   `local` for step 3, and only `zk_not_denied` for steps 5 to 7, because
   the added subject's policy holds no `@zk` deny. 0.5 reports:
   - step 1: `narrowed_too_far` at `local`, and `near_subject_on_face`
     at the added subject, which is on the face and names both kinds;
   - steps 2 to 4: `narrowed_too_far` at `local`;
   - steps 5 and 6: `near_subject_on_face` at the added subject,
     `face_subject_misplaced` at the face's subject, whose far principal's
     policies are `ground`'s alone, and `zk_not_denied`;
   - step 7: the same, and `narrowed_too_far` at `radio-a`, a CN the added
     subject binds and no subject naming CNs alone answers; reported
     though nothing more crosses, since §2.15 reads which policies list a
     subject, not their rules;
   - step 8: `near_subject_on_face` and `face_subject_misplaced`, with no
     `zk_not_denied`.
