# Operation scenarios (core §5, §6)

Common setup: three hosts `h1`, `h2`, `h3`, each with a service `tc`
exposing `@op/interfaces/{if}/set` (exclusive, fanout forbidden) and
`@op/diagnostics` (fanout allowed).

## §1 At most once, while one instance serves (O1)

**Steps.**
1. 200 concrete calls to `set`, with one serving instance, whose handler
   holds each query open for a while (500 ms, say) after replying.
2. The same, with a second instance of the same service on the same router.
3. The same, with that second instance on another router (a split-brain).

**Expected.**
1. 200 executions, and each call returns on its first reply, well before
   its server lets the query go. That is consolidation `None`: under
   `Latest`, the default on a concrete key, the call would wait for the
   query to complete.
2. 200 executions between the two instances, never 400. That is target
   `BestMatching`: one `complete` queryable per router. Under `All`, each
   call would run on both.
3. 400 executions: one per side. With consolidation `None`, the caller sees
   both replies of each call; under `Latest`, one per key would vanish.
   This is the documented limit of O1, and §8 below detects it.

**How a tester observes the query.** zenoh 1.10.1 does not give a
queryable the target or the consolidation of a query, so a capture at the
server cannot show them. Steps 1 and 2 show them by behaviour instead, and
that is the evidence this scenario asks for. A tester that can capture the
caller's outgoing queries (a proxy that decodes the protocol, say) MAY
check the two fields as well.

*Spike S6: A 200, B 0 on one router; 200 and 200 across routers.*

## §2 Fan-out (O2, O3)

**Setup.** Each `tc` also exposes `@op/interfaces/{if}/reset` (fanout
allowed, `replies = "one"`), with members `eth0` and `eth1`: on `h1` a
queryable per member, on `h2` one queryable over the template, and on `h3`
one queryable over the template that refuses every call with `busy`.

**Steps.**
1. Call `zk2/h1/tc/tc.v1/@op/interfaces/*/set`, then `zk2/*/tc/…`, then
   `zk2/*/tc/tc.v1/@op/interfaces/ETH0/set`, whose parameter chunk is not a
   canonical slug either.
2. Call `@op/diagnostics` on `zk2/*/tc/…` with `All` + `None`.
3. Call `zk2/*/tc/tc.v1/@op/interfaces/*/reset` with `All` + `None`.
4. Call `zk2/*/tc/tc.v1/@op/interfaces/ETH0/reset`, whose parameter chunk
   is not a canonical slug, with `All` + `None`.
5. **Many replies, over a template.** A service `h1/scan` exposes
   `@op/ports/{port}/scan` (`replies = "many"`, fanout allowed, raw request
   and response), served by one queryable over the template. Its handler
   names `p1`, sends two values, then tries to name `p2`. Call
   `zk2/h1/scan/scan.v1/@op/ports/*/scan` with `All` + `None`.
6. **Nothing sent.** The same operation (no `summary`), served over the
   template by a handler that names no member and sends nothing. Call it
   the same way.

**Expected.**
1. One, then three, then three `fanout_forbidden` refusals, and 0
   executions. The last is O2's, not `invalid_request`: O2 comes first
   (core §5.1, "The order of refusals").
2. One reply per host.
3. From `h1`, two value replies, on `…/interfaces/eth0/reset` and
   `…/interfaces/eth1/reset`. From `h2`, one value reply, on the key of the
   member its server named, which the call selected. One `busy` error,
   which the caller reports unattributed: a `reply_err` carries no key.
   Presence shows `h3` holding the token and sending no value, so `h3`
   refused or was silent, which the caller cannot tell apart.
4. Two `invalid_request` errors, from `h2` and `h3`, and no handler
   runs: the key names no member (core §5.1). `h1`'s queryables, on
   `eth0` and `eth1`, are not selected.
5. Two values, both on `…/ports/p1/scan`, and none on any other key:
   naming `p2` is refused to the handler. A template-wide server answers
   for one member per call, whatever `replies` is (core §5.1).
6. No value and no error: zero values, then completion. Sending nothing
   needs no member (core §5.1, "Answering").

## §3 Replies and errors (O3, §5.2)

**Steps.**
1. A call that succeeds.
2. One whose request does not decode as the request type.
3. One to `zk2/h1/tc/tc.v1/@op/interfaces/ETH0/set`, whose parameter chunk
   is not a canonical slug.
4. One to an optional operation that is unavailable because a capability
   is missing.
5. One whose handler returns without replying.
6. One refused with `app`: by an operation with no `error` type, by one
   with a JSON Schema `error` type, and by one with a raw `error` type.
7. **Beside a replica.** In a variant of `tc.v1` whose `set` is optional
   and whose `diagnostics` is replicated, a second instance of `h1/tc`
   exposes `diagnostics` only, and lists `set` `unavailable` (cause
   `config`); the first instance serves both. 200 concrete calls to `set`,
   both instances on one router.

**Expected.**
1. A value reply on the concrete key.
2. A `reply_err` with `code = invalid_request`.
3. `invalid_request`, not `not_found`: the key names no member.
4. A `reply_err` with `code = unavailable` and `cause = capability`.
5. A `reply_err` with `code = internal`, never silence.
6. `app` with no detail; `app` with the error value's JSON as its detail,
   or none; `app` with the raw error's bytes as its detail, never base64,
   or none. A detail its type cannot encode goes out as `internal`
   instead.
7. 200 executions on the first instance, and no `unavailable`: the replica
   declares no queryable on `set`.

Each error is core §7.5's error form, under `zenoh/bytes` (core §5.2).
§9 reads its bytes.

## §4 Retries (O4)

**Steps.** With retries configured, call an idempotent operation whose
server is frozen; then one whose server answers `busy`; then a
non-idempotent operation whose server is frozen. Then the idempotent one
again, with no retries configured.

**Expected.** The first is called again after each timeout, up to the
configured number of attempts, and ends silent. The second is called once
and returns `busy`: an error is an answer. The third is called once. The
last is called once: retrying is opt-in.

## §5 Silence (O5)

**Steps.**
1. Call an operation that the access control refuses, then one whose
   server is frozen.
2. **O3 judged from outside** (observer.md §7.2). A tool
   judging O3 from outside, as a conformance suite does, calls the frozen
   server's operation twice: once told that its grants let it call it,
   which is true of that call (it reached the server in step 1), and once
   not told.

**Expected.**
1. Both return no reply. A tool reports "no answer", and attributes it
   through presence. It never reports "no such operation".
2. Told, the tool holds the silence of an owner whose tokens it reads as
   the O3 finding, and says beside it that an access-control refusal is
   silent too. Not told, it reports the silence unobservable, never the
   finding, and names the premise it lacked: grants that let it call,
   which no tool can observe (core §11.3).

## §6 Many replies (O6)

**Steps.**
1. Two servers each reply 5 values and one summary to one call.
2. A server sends 5 values, and its handler returns before its summary.
3. A replicated operation's two replicas sit behind two routers, and each
   replies 5 values and its summary to one concrete call.

**Expected.**
1. With consolidation `None`, 10 values and 2 summaries. With `Latest` or
   `Auto`, values are lost: the reason `None` is required.
2. The 5 values, then an `internal` error. The caller reports the
   replier as possibly partial.
3. 10 values and 2 summaries on one key. The caller reports that replier
   as possibly partial too: two instances on one key cannot be told
   apart.

*Spike S6: `None` 10, `Latest` 1, `Auto` 1.*

## §7 Call metadata (O7)

**Steps.** A request carries the attachment `{actor, request_id}`; then
`{"actor": "a", "extra": 1}`; then `{"actor": 3}`, `[]` and bytes that are
not JSON.

**Expected.** The server sees both members of the first, and MAY record
them. The second is metadata with an actor and no request id. The rest are
no metadata, and each call is served all the same. Nothing authenticates
any of them.

## §8 Split-brain diagnosis, and a shape seen in one read (core §6; observer.md §3.2, §6.1)

**Steps.**
1. Two instances of one service hold an interface token for one interface
   for longer than the grace period.
2. One instance re-mints (core §8.1).
3. A standby holds its instance token only, and its descriptor marks
   the interface `"standby": true` (core §3.3; the profile's scenarios,
   `profiles/redundancy/scenarios.md` §12, run the mark's other shapes).
4. Two instances of one service expose an interface whose only resources
   are replicated operations.
5. One instance exposes an interface's exclusive resources and its
   replicated operation; a second exposes the replicated operation only,
   and lists the exclusive optional resources `unavailable`.
6. **An interface token a second after the descriptor** (observer.md §3.2).
   An instance of `tc` on `h4` serves its descriptor, which lists `tc.v1`
   outside its tokenless set with its resources exposed, and holds its
   instance token (core §8.2, step 4). It declares its `tc.v1` token one
   second later (the rest of step 4), as an owner that pauses between its
   two tokens would (observer.md §3.2). The tool reads presence three times,
   `grace` apart, its grace longer than that second: the first read within
   it, the next two after it.

**Expected.** A finding for 1. No finding for 2, whose overlap is shorter
than the grace period. No finding for 3. No finding for 4 or 5, which the
tool decides from the contract and the holders' descriptors: at most one
holder exposes an exclusive resource. No finding for 6: the interface its
descriptor lists, exposed and holding no token, shows in the first read
alone, so neither pair of reads a grace apart holds it. The runtime
fences nothing.

The tool reads presence twice, `grace` apart; a holder seen in one read
only is not part of a finding.

*Spike S6: the token check found every split-brain and flagged no standby.
A check that compares tokens only would report steps 4 and 5 both.*

## §9 A call's forms (core §5.1, §5.2, §7.5)

**Setup.** `h1/tc` serves `set` (§1) with JSON Schema types, a JSON
Schema `error` type and `encoding = "json"`. It serves three variants of
`set` too: one declared `encoding = "cbor"`, one with protobuf types,
and one with raw types (`application/octet-stream`). `tc.v1` also
declares `@op/ping`, `idempotent = true`.

`h1/tc` exposes a parameterless state too, `status`, which step 4 puts.
`tc.v1` may declare it, or another interface `h1/tc` implements: an
instance's as-of mark counts every state and event seq it mints (core
§5.1).

`h1/tc` implements `seed.v1` beside it, the contract of
`conformance/contracts/ok-packed-operation.toml`:
- `calibrate`: its request, response and error types are packed;
- `scan`: `replies = "many"`, with a packed response and summary;
- `reset`: a packed request, a JSON Schema response, and no `error`
  type.

The test captures each query's payload and `Encoding` as the server
receives it. It captures each reply and reply error as the caller
receives it.

**Steps.**
1. **Each kind of type.** The caller calls `set` and each variant twice:
   once accepted, and once refused `app` with a detail.
2. **Packed types.** The caller calls `calibrate`, then calls it again,
   refused `app` with a `Change` detail. It calls `scan`, whose handler
   sends two values and its summary. It calls `reset`, refused `busy`.
3. **The wrong tag.** A session of the test's own sends `calibrate` a
   `Sensor` record under CH `0x40`, tag 0.
4. **The as-of mark.** `h1/tc` puts `status` twice, and the caller calls
   `set`. Then `h1/tc` puts once more. A second instance then replaces
   it, and the caller calls `set` before that instance's first put.
5. **Errors that are no owner's.** The caller has two attempts
   configured. A session of the test's own serves `ping` in place of its
   server, and answers every attempt of a call with one reply error:
   - the first call's, the 7 bytes `Timeout` under `zenoh/bytes`, as a
     router's (core Appendix B);
   - the second's, `600001000007a8`, `busy` with a cause, under
     `zenoh/bytes`;
   - the third's, `600001000007a0`, a `busy` error's bytes, under
     `zenoh/string`;
   - the fourth's, `600001000007a0` under `zenoh/bytes`.
6. **A timeout behind a router.** `ping`'s server, behind a router,
   freezes, and the caller calls it.
7. **An envelope.** A session of the test's own answers `set` with the
   JSON error envelope that an earlier version of the core had an owner
   write, `{"code":"busy","message":""}`, under `application/json`. Then
   it answers `ping` the same way, and the caller has two attempts
   configured.
8. **A request under another `Encoding`.** `h1/tc` serves `set` again. A
   session of the test's own sends it the bytes of step 1's request form,
   `0x40` then the record, under `application/json`.
9. **The order of refusals under another `Encoding`.** The same session
   sends those bytes under `application/json` to `…/@op/interfaces/*/set`,
   a wildcard call to an operation that forbids fan-out.

**Expected.**
1. Every query arrives under `zenoh/bytes`, as `0x40` then the request's
   record: its JSON, its CBOR, its protobuf message or its raw bytes.
   - Every value arrives under `zenoh/bytes`: CH `0x40`, or `0x50` with
     a 32-bit run, then the mark and the record.
   - Every error arrives under `zenoh/bytes`, CH `0x60` or `0x70`. Its
     detail is the error value in its type's own bytes: JSON, CBOR, the
     protobuf message or the raw bytes, never base64.
   - No sample carries a schema suffix.
   - A tool shows each error by its code, its cause and its message. It
     shows the detail decoded by the `error` type it holds.
2. Each CH carries its type's tag (`conformance/envelope/calls.json`):
   - `calibrate`'s request `0x46`, `Sensor`'s tag 6. Its value `0x4e`,
     `Status`'s tag 14, and its error `0x62`, `Change`'s tag 2, before a
     `Change` record.
   - `scan`'s request `0x40`, its values `0x46`, and its summary `0x4e`
     with the attachment `summary`.
   - `reset`'s request `0x4e`, and its `busy` error `0x60`: it declares
     no `error` type.
   - With a 32-bit run, a value's and an error's CH is `0x10` more.
3. `invalid_request`, and no handler runs: the record's tag is not
   `Sensor`'s (core §7.6, `tag`).
4. The reply's run is the descriptor's `run`. Its seq is the second
   put's, and the third put's seq follows it: a reply mints no seq. The
   second instance's reply carries its own descriptor's run, and seq 0.
5. The first three calls report no error. Each reply error is the
   transport's: `sample_kind`, `cause`, then another `Encoding`.
   - The caller calls `ping` twice for each, then reports silence.
   - A tool shows each as the transport's, with its `Encoding` and its
     bytes, never as `busy`.
   - The fourth is `busy`, an owner's error, and the caller calls once.
6. Silence. The caller's own `Timeout` comes under `zenoh/string`, or the
   router's under `zenoh/bytes`. Neither is an owner's error, so the
   caller calls twice.
7. Each reply error is the transport's: another `Encoding` (core §5.2).
   - The caller reports silence. It calls `set` once, since `set` is not
     idempotent, and `ping` twice.
   - A tool shows each as the transport's, with its `Encoding` and its
     bytes, never as `busy`.
8. `invalid_request`, and no handler runs. Under another `Encoding` a
   query carries no request form, whatever its bytes (core §5.1, "The
   request").
9. `fanout_forbidden`, and no handler runs: the key's refusals come
   before the request is read, whatever the query's `Encoding` (core
   §5.1, "The order of refusals").
