# `link.v2` scenarios

The rules of [`v2.md`](v2.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. The reference runs them as
tests of the tool that compiles a face policy into a router's
configuration (`zenctl acl gen`), against live routers, one test per
section, named after it. [`scenarios.md`](scenarios.md)'s sections are
`link.v1`'s, and hold for a link.v2 policy too.

**Conventions.**
- **Section numbers** in parentheses (§2.16) are v2.md's rules. Core
  sections say "core", and `link.v1`'s say so.
- **The bus.** N is the near router, zenoh 1.10.1, with `usrpwd`
  authentication, as `link.v1`'s scenarios run it. It listens on
  `tcp/127.0.0.1:<port>`, the near side, and on
  `unixsock-stream/<path>`, the face.
- **The contract and the enrollment** are `link.v1`'s scenarios'
  (`radio.v1`, and the reference's
  `examples/zk2/link/radio.enrollment.toml`).
- **The face policy**, unless a section changes it:

  ```toml
  profile  = "link.v2"
  system   = "lab"
  services = ["radio-a"]
  binding  = "zenoh-1.10.1"

  [transport]
  lease_ms             = 2000
  keep_alive           = 4
  wait_before_close_ms = 2000
  sequence             = "16bit"
  batch_bytes          = 1000

  [face.radio]
  far              = "ground"
  attach           = "client"
  select           = { link_protocols = ["unixsock-stream"] }
  class            = "constrained"
  expose           = "link"
  interval_s       = 1
  max_outage_s     = 6
  link_bps         = 1000000
  loss_ppm         = 0
  arq              = true
  reply_timeout_ms = 1000
  ```

  Its keep-alive interval is 0.5 s. A linter reports no finding of it.
- **F** is the far end: a client session, user `ground`, that attaches
  to N over the face.
- **R** is a relay between F and N's face, which F connects to. It
  forwards both directions, and counts each direction's keep-alives: a
  batch holding one `KeepAlive` message, the 1-byte batch after the
  stream link's 2-byte length. On request, R **holds** both directions
  for a time, a silence, and delivers what it held when the time is up.
  It reads both sockets throughout, so it sees an end close its socket
  at once.

## §1 The router runs the block (§2.16, §2.18, §2.19)

**Steps.**
1. The generator compiles N's block from the policy, the enrollment and
   the contract, and N starts with it. The generator writes the far
   end's block (§2.18), and F starts with it, through R.
2. A tool checks N's configuration against the policy, with F's as the
   far end's (§2.19).
3. Nothing is published for 4 s, two leases. R counts the keep-alives
   each way.
4. R holds both directions for 1 s, half the lease, then delivers what it
   held. Nothing is published for 1 s more.
5. R holds both directions for 4 s, twice the lease.
6. **Control:** the tool checks N's configuration with its
   `wait_before_close` edited to 1,000,000 µs, half the lease.

**Expected.**
1. N's configuration states the policy's eight keys: a lease of 2,000 ms,
   4 keep-alives, `16bit`, a batch of 1,000 B, a `wait_before_close` of
   2,000,000 µs, and 10,000 ms for each timeout. F's states the same.
2. The tool reports no finding.
3. R counts 7 to 9 keep-alives each way: 4 s × 4 / 2 s, §2.16's idle
   bill. That is 2 × 86,400 × 4 / 2, or 345,600 a day, both directions
   together.
4. The transport stays: neither end closes its socket, and R counts
   keep-alives each way in the second after the silence.
5. Each end closes the transport between 1.5 s and 2.5 s after the
   silence began. An end expires its receive side a lease after the last
   thing it received, which came at most one keep-alive interval before
   the silence.
6. The tool reports `wait_under_lease` and `transport_differs`, both at
   `transport.wait_before_close_ms`.

## §2 Frames per face (§2.7, §2.8; `link.v1` §2.9)

**Setup.**
- **The contract** is `probe.v1`
  ([`conformance/contracts/probe.v1.toml`](conformance/contracts/probe.v1.toml)),
  which lists `link.v2`. It exposes two frame resources, `frames/link`
  and `frames/detail`, and a collection, `status`. It also exposes a
  timed stream, `setpoint`, and a plain stream, `level`.
- **The deployment.** `lab/probe-a` implements `probe.v1`, as the user
  `probe-a`. The console, run by `ground` and `local`, binds every
  stream, state and event of `probe.v1` from `lab/*`.
- **The face policy** is the Conventions', its `services` set to
  `["probe-a"]`. Its face lists one frame:

  ```toml
  frames = { "probe.v1/frames/link" = { every_s = 1 } }
  ```

**Steps.**
1. The generator compiles N's block from the policy, the deployment and
   the contract, and N starts with it.
2. G, a client session as `ground`, attaches over the face. L, as
   `local`, attaches on the near side. Each subscribes to what the
   console is granted.
3. The owner, a session as `probe-a`, puts at 20 Hz for 5 s on
   `frames/link`, `frames/detail`, `setpoint` and `level`. It also puts
   its heartbeat covering `probe.v1`, at 2 Hz.

**Expected.**
1. The plan lets `frames/link`, `status`, `level` and the heartbeat
   cross. It denies `frames/detail` and `setpoint` by name.
2. Its one downsampling rule is `level`'s. It warns `timed_downsampled`
   at `setpoint` and `frame_unlisted` at `frames/detail`. It also warns
   `keep_alive_not_suppressed`: a frame a second is above the 0.5 s
   keep-alive interval.
3. L receives every put.
4. G receives every put of `frames/link` and of the heartbeat. Neither
   is downsampled.
5. G receives 4 to 6 puts of `level`, one a second.
6. G receives no put of `frames/detail` or `setpoint`.

## §3 The near router stamps nothing (§2.18, §2.19; core §8.2)

**Setup.** N, the owner and F as the Conventions run them. The owner is
`lab/radio-a`, a client session of N on the near side as `radio-a`, its
timestamping off (core §8.2). F attaches to N over the face directly.

**Steps.**
1. The generator compiles N's block from the policy, the enrollment and
   the contract, and N starts with it.
2. F subscribes to `zk2/lab/radio-a/radio.v1/state/oper`, a state the
   face lets cross whole. The owner puts `oper` until F receives it.
3. A tool checks N's configuration (§2.19).
4. **Control:** N starts again from its block with the `timestamping`
   object removed, and steps 2 and 3 run against it.

**Expected.**
1. N's configuration states timestamping off for a router and a peer.
2. F's sample of `oper` carries no timestamp.
3. The tool reports no finding.
4. In the control, F's sample carries a timestamp, N's own, its id N's
   zid. The tool reports `face_timestamping` at `timestamping`.

## §4 The far end opens a face bound by a user name (§2.5, §2.18)

**Setup.** N and the owner as §3 runs them, under `deny`. The face is a
`south-region` face, `region = "ground"`, whose far principal is
`ground-router`, a user name of the enrollment, which N's dictionary
holds. F is a far router naming its region `ground`, listening on
`tcp/127.0.0.1:<port>` and on a unix socket of its own. G is a client
session of F, as `ground`.

**Steps.**
1. The generator compiles N's block and F's block. N starts with its
   block. F starts with its block, `ground-router`'s password and a
   `dictionary_file` that holds `ground` and `router`, and connects to N
   over the face.
2. G subscribes to `zk2/lab/radio-a/radio.v1/state/oper`, a state the
   face lets cross. The owner puts `oper` until G receives it.
3. A session that presents no user opens a link to F.
4. **Control:** the face states `opens = "near"`, and the generator
   compiles the policy again.
5. **Control:** N starts again from step 1's block, presenting
   `router`, and connects to F's unix socket. F starts from step 1's
   configuration and connects to nothing. Step 2 runs again, for the
   silence.

**Expected.**
1. F's block names `ground-router` as its usrpwd user, its password and
   dictionary placeholders.
2. G receives `oper`: N bound F's link to `ground-router`.
3. F refuses it at its handshake.
4. The generator refuses the policy: `near_opens_user` at `opens`.
5. F lets N in, and holds a transport to N. G receives nothing: N bound
   the link it opened to no subject, and under `deny` grants it nothing.
