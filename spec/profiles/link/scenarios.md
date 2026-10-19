# `link.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. They are a generator's and a
router's: the reference runs them as tests of the tool that compiles a
face policy into a router's configuration (`zenctl acl gen`), against live
routers, one test per section, named after it (#769, LB).

**Conventions.**
- **Section numbers** in parentheses (§2.7) are v1.md's rules; core
  sections say "core", and `freshness.v1`'s say so.
- **The bus.** N is the near router, zenoh 1.10.1, with timestamping on
  and `usrpwd` authentication. It listens on `tcp/127.0.0.1:<port>`, the
  near side, and on `unixsock-stream/<path>`, the face. Every session
  authenticates as a user of its own. Timeouts are 1 s unless stated.
- **The generator** compiles N's `access_control` block, under `deny`,
  from the enrollment below, the contract and the face policy, with the
  plan's denies in the far principal's policy (§2.4) and the plan's
  `downsampling` block beside it, and N's `gateway` block for a south
  region (§2.5). "The block" is what it wrote.
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
