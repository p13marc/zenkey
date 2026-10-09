# `freshness.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. The reference runtime runs §1
to §5 (`zenkey/tests/profile_freshness.rs`, one test per section, named
after it). §6 is a tool's, and the reference tool runs it
(`zenctl/tests/live_zk2.rs`, `check conform`).

**Conventions.**
- **Section numbers** in parentheses (§2.4) are v1.md's rules; core
  sections say "core".
- **The bus.** R1 is a router with timestamping on, as a router's default
  is. The owner, the subscriber S and the GET reader G are its clients.
  Timeouts are 1 s unless stated.
- **The contract.** Every section uses `beacon.v1`. The reference keeps
  it as `zenkey/tests/contracts/beacon.v1.toml`:

  ```toml
  [interface]
  name  = "beacon"
  major = 1
  minor = 0
  uses  = ["freshness.v1"]

  [resources.status]                         # re-put at least every 1 s
  kind        = "state"
  type        = { raw = "text/plain" }
  annotations = { "freshness.ttl_s" = 2 }

  [resources.intent]                         # never stale
  kind        = "state"
  type        = { raw = "text/plain" }
  annotations = { "freshness.ttl_s" = 0 }

  [resources.note]                           # no horizon
  kind = "state"
  type = { raw = "text/plain" }

  [resources.level]                          # a stream, ttl 1 s
  kind        = "stream"
  type        = { raw = "text/plain" }
  annotations = { "freshness.ttl_s" = 1 }
  ```
- **The owner** is `lab/beacon`, implementing `beacon.v1` with every
  resource exposed. Its keys are
  `zk2/lab/beacon/beacon.v1/state/{status,intent,note}` and
  `zk2/lab/beacon/beacon.v1/stream/level`.
- **S** subscribes to `zk2/lab/beacon/beacon.v1/state/*` and
  `zk2/lab/beacon/beacon.v1/stream/*`. It records each delivery's arrival
  on its monotonic clock, its kind, payload and stamp. "S judges" applies
  v1.md §2.5 at that instant.
- **G** GETs the owner's state keys as core S4 says, and "G judges"
  applies v1.md §2.6 to the reply, with the delta at 500 ms. G **measures**
  its clock (v1.md §2.6, ground 2) from S's deliveries: a put stamped by
  the owner's clock whose stamp was within 500 ms of G's clock at receipt.
- **Jitter.** A gap between deliveries is measured at S, which adds the
  bus's jitter to what the owner sent. The reference allows 200 ms on
  loopback.

## §1 The re-put cadence (§2.4)

**Steps.**
1. The owner puts `status` = `up` once, and puts nothing else on it for
   5 s.
2. The owner puts `status` = `down`, then nothing for 2 s.
3. G GETs `status`.
4. The owner puts `note` = `n` once, and S listens 3 s.
5. The owner deletes `status`, and S listens 3 s.
6. The owner puts `status` = `up` again, then closes its writer for
   `status`, keeping its service up. S listens 3 s, then G GETs `status`.
7. The owner puts `status` = `up` again from a new writer, then closes its
   service. S listens 3 s.

**Expected.**
1. S receives `up` again and again: at least 4 more times in the 5 s
   after the first, no two deliveries more than 1 s apart (ttl/2). Every
   delivery carries `up` and the same `Encoding` (`text/plain`). Every
   stamp's id is the owner's session's zid, and every stamp is above the
   one before.
2. Every delivery from then on carries `down`, and none carries `up`: the
   change started a new interval.
3. The reply carries `down`, and the stamp of the latest delivery S
   received before the GET, a re-put's, not the change's.
4. S receives `note` once: a member with no horizon is not re-put.
5. S receives the delete, and nothing more on `status`.
6. S receives the put, then at most one re-put already under way, then
   nothing more on `status` for the rest of the 3 s. G's reply carries
   `up` and the stamp of the last delivery: the owner still holds the
   value, and nobody confirms it.
7. As 6: no delivery on `status` once the service has closed.

## §2 A stopped refresher goes stale (§2.5, §2.6, §2.10, §2.11)

**Steps.**
1. The owner puts `status` = `up`. S receives re-puts, which G measures
   its clock from.
2. The owner closes its writer for `status`, and keeps its service up:
   its tokens held, its GET answered.
3. S judges `status` 1 s after its last delivery, then 3 s after it. G
   GETs `status` and judges the reply 0.2 s after the close, then 3.5 s
   after it.
4. A tool reads the owner's presence (core §8.1) at the end of step 3.
5. **Clock ahead.** A second owner, `lab/ahead`, implementing `beacon.v1`,
   runs its clock 2 s ahead of R1, and watches a router-stamped heartbeat
   key as its clock reference (core §4.3). It puts `status` = `up` before
   any heartbeat. Then the heartbeat is put until the owner reports itself
   ahead. S listens to `lab/ahead` for 3 s from then.

**Expected.**
1. Both judge `status` fresh.
3. S: fresh at 1 s; stale at 3 s, its last delivery more than 2 s old.
   G: fresh at 0.2 s, the reply at most 1.2 s old against its measured
   clock, within ttl − δ (1.5 s); stale at 3.5 s, above ttl + δ (2.5 s).
4. The owner's instance and interface tokens are present. The tool reports
   the owner present and `status` stale, both, neither folded into the
   other.
5. Once the owner reports itself ahead, S receives nothing more from it on
   `status`: the guard holds its re-puts, and S judges `status` stale 2 s
   after its last delivery. The owner's tokens stay present.

## §3 Never stale (§2.3)

**Steps.**
1. The owner puts `intent` = `i` once. S listens 3 s, longer than any
   horizon of the contract.
2. G GETs `intent` 3 s after the put, and judges the reply with no
   measurement and no word from the deployment: its clock is not trusted.

**Expected.**
1. S receives `intent` once, and no re-put. S judges it fresh at 3 s:
   never stale.
2. G judges it fresh, though it cannot age the reply: ttl 0 needs no
   clock.

## §4 A skewed GET reader (§2.5, §2.6)

**Setup.** The owner's clock runs 5 s behind G's. A runner cannot set its
own clock, so it sets the owner's (the reference's `simulate_offset`):
what §2.6 bounds is the disagreement between the two.

**Steps.**
1. The owner puts `status` = `up`, and goes on re-putting it. S receives
   the re-puts, and G tries to measure its clock from them.
2. G GETs `status` and judges the reply.
3. G judges the same reply again, told by the deployment that its clocks
   agree within the delta (v1.md §2.6, ground 1), which here they do not.

**Expected.**
1. S judges `status` fresh: it ages on its receive clock, which the skew
   does not touch. G's measurement fails: each stamp is about 5 s behind
   G's clock at receipt.
2. G judges the reply **unobservable**, its clock untrusted: never fresh,
   and never stale, although the stamp reads about 5 s old, beyond
   ttl + δ.
3. G judges it stale. The word was the deployment's, and so is the error.

## §5 The stream case (§2.2, §2.4, §2.5)

**Steps.**
1. The owner publishes `level` every 200 ms for 2 s, then stops, keeping
   its writer and its service.
2. S judges `level` 0.5 s after the last sample, then 1.5 s after it.

**Expected.**
1. S receives a sample every 200 ms, within the 0.5 s (ttl/2) bound, and
   nothing after the owner stops: the runtime publishes no stream sample
   on the owner's behalf.
2. Fresh at 0.5 s; stale at 1.5 s.

## §6 A tool's verdict on a resource (§2.7, §5)

**Setup.** A tool reads `lab/beacon` over a 4 s window: it subscribes to
every exposed stream and state resource first, then GETs every state
resource, and measures its clock from what it receives. It judges every
member at the end of the window, and reports one verdict per resource
(v1.md §5, the second question).

**Steps.**
1. The owner holds `status` (re-put), `intent` and `note`, and publishes
   `level` every 200 ms. The tool reads.
2. The tool reads again. Once its subscription on `level` is declared (the
   owner's publisher sees a match), and 0.5 s more have passed, the owner
   closes its writer for `status` and stops `level`, keeping its service.

**Expected.**
1. `status` fresh; `intent` fresh (never stale); `note` not asked (no
   horizon); `level` fresh.
2. `status` stale: the subscription's last delivery is more than 2 s old
   at the end of the window, and the GET's reply is either stale or within
   δ of the horizon, against a clock measured on the re-puts heard before
   the stop (v1.md §2.7). `level` stale: its last sample is more than 1 s
   old. `intent` fresh; `note` not asked. The run's verdict is the finding.
