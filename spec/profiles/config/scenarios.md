# `config.v1` scenarios

The rules of [`v1.md`](v1.md) that a fixture cannot check, in the form of
the core's scenarios ([`../../scenarios/README.md`](../../scenarios/README.md)):
a setup, steps, and expected observations. §1 to §12 are the runtime's,
and run as tests of `zenkey`, one test per section, named after it. §13
and §14 are a deployment's and a tool's, and run as tests of the tool
that reads `config.v1`.

**Conventions.**
- **Section numbers** in parentheses (§2.9) are v1.md's rules; core
  sections say "core", and `freshness.v1`'s say so.
- **The bus.** R1 is a router with timestamping on. The owner, the caller
  C, the subscriber S and the tool T are its clients. Timeouts are 1 s,
  except a call's, which is 2 s, unless stated.
- **The contract** is the standard one, [`config.v1.toml`](config.v1.toml),
  with JSON payloads.
- **The owner** is `lab/rf0`, implementing `config.v1` with every resource
  exposed, a persisted layer, and the schema of
  [`conformance/lifecycle.json`](conformance/lifecycle.json)'s `rf`:
  - `duty` (hot, writable): `fraction_ppm`, an integer 0 to 1,000,000,
    startup 10000 from its file;
  - `radio` (reach, writable): `frequency_hz`, 150 MHz to 960 MHz, startup
    868100000 from its file;
  - `link` (reach, writable): `net_id`, 0 to 255, startup 1 by default;
  - `sim` (hot, writable): `pin`, text, sensitive, startup `"0000"`;
  - `queue` (hot, outside the allowlist): `tx_queue_len`.

  Its floor is 10 s, and it states `apply_at` 1 s after it receives a
  reach `set`. Its device is simulated: a test sets each act's result, and
  reads what the device holds.
- **Keys.** `V` is `zk2/lab/rf0/config.v1/state/view`, `St` its
  `state/status`, `Op(x)` its `@op/x`, and `Ev` its
  `events/change/*`.
- **S** subscribes to `zk2/lab/rf0/config.v1/**` and `zk2/lab/rf0/@zk/**`
  before the owner starts, and records each delivery's arrival, key, kind
  and payload. **C** calls as core O1 says, with the call metadata
  `{"actor": "ops", "request_id": …}` (core O7), and GETs as core S4 says.
- **"The fixture's step"** names a step of `lifecycle.json` whose replies,
  events and state the scenario observes on the bus: the reply on the
  call, the events on `Ev`, the state as `St` (and `V`'s parameters).

## §1 A hot set, read back (§2.3, §2.9, §2.17)

**Steps.**
1. The owner starts. T, holding a liveliness subscriber on
   `zk2/lab/*/@zk/**` with history, GETs `V` and `St` the moment it sees
   the owner's instance token.
2. C calls `Op(set/duty)` with `{"values": {"fraction_ppm": 20000}}`.
3. C GETs `V` and `St`.
4. The owner puts nothing for 35 s.

**Expected.**
1. Both of T's GETs answer: revision 0, nothing pending, no last change,
   `confirm_floor_s` 10, every group with its `writable`, `pin` without a
   value. They were put before the tokens (§2.3).
2. The reply is `{applied: <view>}`: `fraction_ppm` 20000 at `source:
   runtime`, `startup` 10000, revision 1, `last_change` `{token, ["duty"]}`.
   S receives one event on `Ev`, `applied`, with that token and the edit
   10000 → 20000, `actor` `"ops"`, before or after `V` and `St`, which S
   receives once each with revision 1.
3. `V` and `St` answer revision 1, and agree (§2.19).
4. S receives `V` and `St` again within 30 s, unchanged (`freshness.v1`
   §2.4). S receives no delete of either.

## §2 A reach set, confirmed (§2.8, §2.9, §2.10)

**Steps.**
1. C calls `Op(set/radio)` with `{"values": {"frequency_hz": 869525000},
   "confirm_s": 120}`, and notes the reply's arrival.
2. C calls `Op(confirm)` with the token, before `apply_at`.
3. C waits until `St` shows the change applied, then calls `Op(confirm)`
   with the token.
4. C calls `Op(confirm)` with the token again.

**Expected.**
1. The reply is `{pending: {token, apply_at, deadline}}`, `apply_at` 1 s
   and `deadline` 120 s after the owner received the call. The reply
   arrives before the device holds the new frequency: the device's
   record of the apply is later than the reply's send (§2.9). S receives
   `St` with `pending.applied` false, then with it true and `frequency_hz`
   869525000.
2. `busy`: not yet applied (§2.9).
3. `{view: <view>}`: nothing pending, `last_change` the token. S receives
   one event, `confirmed`, with `actor` `"ops"` and the `request_id` of
   step 1's call, not step 3's (§2.17).
4. `not_found` (§2.10).

## §3 A reach set rolled back at its deadline (§2.11, §2.17)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 5: the window is the floor,
   10 s (§2.8).
2. C does nothing. 10.5 s after the call, C GETs `St`, and calls
   `Op(confirm)` with the token.
3. A second owner, `lab/rf1`, configured as `lab/rf0`, takes the same
   `set`, and its simulated device refuses the apply at `apply_at`.

**Expected.**
1. The reply's `deadline` is 10 s after the call.
2. Before the GET, S receives one event, `rolled-back`, and `St`'s
   `frequency_hz` is 868100000 again, at `source: file`, with revision 2;
   the device holds the startup frequency. The confirm is `not_found`.
   The device saw exactly one undo.
3. `lab/rf1` publishes one event, `rolled-back`, at `apply_at`, with no
   undo made: nothing was applied (§2.9, §2.11).

## §4 An undo that fails (§2.11)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 10. The test makes the
   device's undo fail.
2. 20 s after the call, C GETs `V`.

**Expected.**
1. At the deadline, S receives one event, `partial`.
2. `V` shows `frequency_hz` as the test's device reports it, or without a
   value when the device reports none; nothing pending. The device saw
   exactly one undo, and no second within the 10 s after it: never
   retried (§2.11).

## §5 A join, and a second writer (§2.7)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 120.
2. A second caller, C2, calls `Op(set/link)` with `confirm_s` 60.
3. C calls `Op(set/link)` with `{"values": {"net_id": 7}, "token": <the
   token>}`, then `Op(set/duty)` with `fraction_ppm` 20000 and the token.
4. C confirms once `St` shows the change applied.

**Expected.**
1. A pending reply.
2. `busy`, its message naming the pending token. C2 reads the token from
   `St`'s `pending` (§2.7).
3. A pending reply with the same token and `deadline`, its own
   `apply_at`; then `{applied: <view>}` with `pending.groups` `["radio",
   "link", "duty"]`.
4. One event, `confirmed`, with the three groups' edits, in that order.
   No event was published at either join (§2.17).

## §6 An extend (§2.10)

**Steps.**
1. C calls `Op(set/duty)` with `fraction_ppm` 20000 and `confirm_s` 30.
2. 20 s later, C calls `Op(extend)` with `confirm_s` 60 and `answer =
   "status"`.
3. C does nothing more.

**Expected.**
1. `{applied: <view>}` with the change pending, deadline 30 s after the
   call.
2. `{status: <status>}` with the deadline 60 s after the extend. No event.
3. The undo runs at the new deadline, not the first: S receives the
   `rolled-back` event 80 s after step 1, ± 0.5 s.

## §7 A restart during a pending change (§2.6, §2.12)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 120, and waits until `St`
   shows it applied.
2. The owner's process stops, and starts again.
3. C calls `Op(confirm)` with the token.

**Expected.**
1. A pending reply; the device holds 869525000.
2. S sees the old instance token withdrawn and a new one declared (core
   §8.1). The new instance's `V` and `St`, put before its tokens, show
   revision 0, nothing pending, no last change, and `frequency_hz`
   868100000 at `source: file`; the device holds the startup frequency.
   `St`'s `instance` is the new instance's id. No event is published for
   the old change (§2.12).
3. `not_found`. The token is not one the new instance minted, whatever
   it mints next (§2.6).

## §8 Persist, granted and not (§2.13, §2.21)

**Setup.** R1 runs access control with `default_permission: deny`. The
principal `ops` is granted `Op(set/*)`, `Op(confirm)`, `Op(cancel)`,
`Op(extend)` and the GET of `V` and `St`; the principal `admin` those and
`Op(persist)`. A second owner, `lab/rf2`, has no persisted layer.

**Steps.**
1. As `admin`, C calls `Op(set/duty)` with `fraction_ppm` 20000, then
   `Op(persist)` with the reply's `last_change.token`.
2. As `ops`, C calls `Op(set/duty)` with 30000, then `Op(persist)` with
   its token.
3. As `admin`, C calls `Op(persist)` on `lab/rf2`, after a `set` there,
   and GETs `lab/rf2`'s descriptor.
4. The owner restarts. C GETs `V`.

**Expected.**
1. Both answered; the persist's reply shows `fraction_ppm` at `source:
   overlay`. No event for the persist.
2. The `set` is answered; the `persist` draws nothing before its timeout:
   a refusal by access control is silent (core O5), and C reports it so,
   never as `not_found`.
3. `unavailable`, cause `config`; the descriptor lists `persist` in
   `unavailable` with cause `config` (core §3.3).
4. `fraction_ppm` is 20000 at `source: overlay`: step 1's change survived;
   step 2's did not (§2.12).

## §9 A group outside the allowlist (§2.16)

**Steps.**
1. C GETs `V`.
2. C calls `Op(set/queue)` with `{"values": {"tx_queue_len": 10}}`.
3. C calls `Op(set/queue)` with `{"values": {"nope": 1}}`.

**Expected.**
1. Every group but `queue` has `writable` true.
2. `unavailable`, cause `config`. Nothing changed; no event.
3. The same: the allowlist comes before the values (§2.5).

## §10 An idempotent retry (§2.14)

**Steps.**
1. C calls `Op(set/radio)` with `confirm_s` 60 and `idempotency_key`
   `"k1"`. The test drops the reply before it reaches C.
2. C calls again, the same request, 5 s later.
3. C calls with the same key and another `frequency_hz`.
4. C cancels the change, and 300 s after step 1's call, calls step 1's
   request again.

**Expected.**
1. The owner arms the change; C receives nothing.
2. The reply is step 1's, token and instants unchanged. The device saw
   one apply, and the owner published nothing new (§2.14).
3. `invalid_request`.
4. A new pending reply, with a new token: the key was forgotten at 300 s.

## §11 Sensitive values (§2.15)

**Steps.**
1. C calls `Op(set/sim)` with `{"values": {"pin": "1234"}}`, then with
   `{"values": {"pin": 1234}}`.
2. The test makes the device refuse a third `set` of `pin`, with words
   holding the PIN.
3. C GETs `V` and `St`, and calls `Op(persist)` with the first change's
   token.
4. The test reads the owner's log.

**Expected.**
1. The first is applied; its event's edit is `{"group": "sim",
   "parameter": "pin", "redacted": true}`. The second is
   `invalid_request`, its message naming `pin` and holding no `1234`.
2. `app`, `ConfigError{code: "device-refused"}`, with no `detail`.
3. `V` shows `pin` with `source: runtime` and no `value` or `startup`;
   `St` has no `sim`. The persist is `forbidden`.
4. No line of it holds `1234`; a logged request writes `"<redacted>"`.
   No delivery S recorded, and no reply C received, holds `1234`.

## §12 Desired state refused (§2.18)

**Setup.** A deployment binds `lab/rf0` to a desired document, put by an
author service `lab/author` as its own state.

**Steps.**
1. The author puts `{"revision": 1, "values": {"duty": {"fraction_ppm":
   20000}}}`.
2. The author puts `{"revision": 2, "values": {"duty": {"fraction_ppm":
   30000}, "radio": {"frequency_hz": 869525000}}}`.
3. The author puts `{"revision": 3, "values": {"sim": {"pin": "1234"}}}`.

**Expected.**
1. S receives one event, `applied`, and `St` with `fraction_ppm` 20000 and
   `desired` `{observed_revision: 1, outcome: "converged"}`.
2. No act and no event: `St`'s `desired` is `{observed_revision: 2,
   outcome: "refused", refusal: {code: "reach", group: "radio"}}`, and
   `fraction_ppm` is still 20000. The duty change, legal on its own, was
   not applied: a document is refused whole.
3. `refusal: {code: "sensitive", group: "sim", parameter: "pin"}`, and no
   act.

## §13 The compact status across a constrained face (§2.10, §2.19, §2.21)

**Setup.** `spec/scenarios/constrained.md` §1's shape: the owner behind
R1, and the caller C as a client of R1 across a constrained face, with the
`@zk` deny on it and a 1 KB batch. The face denies the subscription to
`zk2/lab/rf0/*/state/**` and lets its GET cross (`config.v1-binding.md`
§12). C binds statically: it reads no presence and no descriptor.

**Steps.**
1. C GETs `St`.
2. C calls `Op(set/radio)` with `confirm_s` 120.
3. Once `St` shows it applied, C calls `Op(confirm)` with `answer =
   "status"`.
4. The owner restarts. C GETs `St`.

**Expected.**
1. `St` answers, with `confirm_floor_s`: C sizes its window from it.
   No `@zk` delivery crosses the face.
2. The pending reply, at most 160 bytes of payload.
3. `{status: <status>}`, at most 400 bytes for this schema: the status,
   not the view.
4. `St`'s `instance` differs from step 1's: C learns of the restart
   without presence (§2.12).

## §14 A tool's answers (§5)

**Setup.** The tool T reads `lab/rf0` and a service `lab/plain` that does
not implement `config.v1`.

**Steps.**
1. T asks "is a change pending?" of both, with the owner idle.
2. C arms a reach change; T asks again of `lab/rf0`, and asks "did this
   change land?" with its token.
3. The change rolls back at its deadline; T asks "did it land?" and "was
   its undo clean?".
4. C sets `duty` without a window; T asks "will `fraction_ppm` survive a
   restart?".
5. The owner stops; T asks "is a change pending?" within 60 s, then
   after 70 s from the owner's last put.

**Expected.**
1. `lab/rf0`: no. `lab/plain`: not asked, its descriptor not listing
   `config.v1`.
2. Yes, with the token and the deadline; "did it land?": not yet.
3. No, `rolled-back`; and yes, clean.
4. No: `fraction_ppm` runs a value with `startup` 10000.
5. Within 60 s: not asked, the owner absent (presence's word). After it,
   with a reader that does not read presence: unobservable, the status
   stale; never no (§5, core O5).
