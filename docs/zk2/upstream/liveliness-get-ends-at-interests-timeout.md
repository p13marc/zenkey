# Draft upstream report: a liveliness GET ends "complete" at the interests timeout, with tokens missing

**Status: not filed.** Drafted on 2026-10-25 for zenkey #851 (chunk WB),
from the reference's measurements at zenoh 1.10.1, and kept, as the other
drafts here are, as a record of behaviour zk2 designs around: since core
0.41 (§8.1, Appendix B) a reader reads a liveliness GET whose time reached
its session's `routing/interests/timeout` as possibly incomplete, and zk2's
tools run their sessions with a 120 s interests timeout.

The measurements are `zenkey/tests/presence_scale.rs`'s ignored
`wb_a_barrier_beside_ten_thousand_tokens_measured` (10,000 tokens) and
`zenkey/tests/presence.rs`'s `s10_a_round_trip_at_a_population_and_the_interests_timeout`
step 5 (a stalled link), the second run on every CI build
([`spec/scenarios/presence.md`](../../../spec/scenarios/presence.md) §10).

---

**Title:** liveliness: a GET with a timeout longer than
`routing/interests/timeout` ends at the interests timeout with no error
reply, as if the routers had finished it

**Version:**
- zenoh 1.10.1 (crates.io), with the `unstable` feature.
- Client sessions, each connected to one in-process router over TCP
  loopback. A debug build, where a router answering 10,000 tokens is slow
  enough to show it; a stalled link shows it at any build and any size.
- Every GET uses an unbounded handler (`flume::unbounded()`).

**What happens.** `session.liveliness().get(sel).timeout(t)` with `t` longer
than the session's `routing/interests/timeout` (10 s by default) ends at
the interests timeout when its router has not finished it. The handler
closes after the replies received so far and **no error reply**: no
`Timeout`, nothing to tell the caller that the routers never sent their
final reply. A GET whose own timeout ends it gets `Timeout`
(`zenoh/string`), so a caller that reads "no error reply" as "the routers
finished it" takes a partial answer for a whole one.

*Beside 10,000 tokens.* A router; four clients holding 10,000 liveliness
tokens between them (`zk2/wb/svc<i>/@zk/instance/<16 hex>`); a fresh client
reads `zk2/*/*/@zk/instance/*` with a 110 s timeout:

| Reader's `routing/interests/timeout` | Ended after | Error replies | Tokens returned |
|---|---|---|---|
| 120,000 ms | 22.3 s | none | 10,000 |
| 10,000 ms (default) | 10.0 s | **none** | **6,121** |

*A stalled link.* A client whose `routing/interests/timeout` is 300 ms
reaches its router through a TCP proxy that stops forwarding both ways and
closes nothing. A liveliness GET with a 2 s timeout ends at 300.5 ms with no
reply at all and no error reply.

*A liveliness subscriber's history.* With the default interests timeout, a
client that declares a history liveliness subscriber over the same 10,000
tokens and then a liveliness GET of a key no token holds: the GET ends at
10.0 s with no error reply, and the subscriber had received **none** of
its history 20 s after it was declared. Under a 120 s interests timeout the
history arrived whole, its last token at 23.0 s, and the GET ended 1 ms
after it.

**Expected.** A liveliness GET that its routers have not finished when the
session gives up on them ends with an error reply, as one that reaches its
own timeout does, so that "no error reply" keeps meaning "the routers sent
their final reply". Equally, the GET's own timeout could bound the wait for
its routers' final reply, rather than the session-wide interests timeout.

**What works:**
- A GET whose timeout is shorter than the interests timeout ends with
  `Timeout` when the routers are late.
- Raising `routing/interests/timeout` above every GET's timeout: the
  10,000-token read was then whole.
- Reading the elapsed time: a GET that ended at or after the session's
  interests timeout was ended by the session, whatever its replies say
  (what zk2 does since core 0.41).

**Where it ends** (1.10.1's line numbers):
- A client's `hat/client/interests.rs` `route_interest` (line 88) forwards
  the `Current` interest to its router and parks it in the face's
  `pending_current_interests`, spawning
  `CurrentInterestCleanup::spawn_interest_clean_up_task`
  (`dispatcher/interests.rs`, line 131) with the tables'
  `interests_timeout` (`routing.interests.timeout`, `DEFAULT_CONFIG.json5`
  line 247). A peer's `hat/peer/interests.rs` `route_interest` (line 100)
  does the same.
- At the timeout, `CurrentInterestCleanup::execute` (line 157) logs "Didn't
  receive DeclareFinal for interest …" at `warn`, and calls
  `finalize_pending_interest` (line 90), which sends a `DeclareFinal` to the
  interest's source: the session's own face.
- The session (`api/session.rs`, line 3321) takes that `DeclareFinal` as
  any other: it removes the liveliness query, which drops its callback, and
  the handler closes with what it received. The GET's own timeout task
  then finds no query to answer with `Timeout`.
- The router's later declarations for the interest find no pending interest
  at the client and are dropped, which is where a history subscriber's
  history went in the case above.

**Not identified:**
- Whether a `CurrentFuture` interest (a history subscriber) parked the same
  way loses its history for good, or only until the router's declarations
  reach the client: the subscriber above had none 20 s after it was
  declared, and future declarations reached it. The trace was not taken.
- Why the router took 22 s to answer 10,000 tokens to one client in a debug
  build (about 2 ms a token, against 31 ms for 300). It is what makes the
  default interests timeout bite on a large population; the build's cost
  was not profiled.
- Whether a release build reaches the interests timeout at 10,000 tokens.
  The stalled link shows the behaviour at any build.

**Repro:**

```rust
// router on tcp/127.0.0.1:<port>; `proxy` forwards to it and can be stalled.
let mut c = zenoh::Config::default();
c.insert_json5("mode", "\"client\"")?;
c.insert_json5("connect/endpoints", &format!("[\"{proxy}\"]"))?;
c.insert_json5("routing/interests/timeout", "300")?;
let reader = zenoh::open(c).await?;
proxy.stall(); // stop forwarding both ways, close nothing
let rx = reader.liveliness().get("demo/**")
    .timeout(Duration::from_secs(2))
    .with(flume::unbounded()).await?;
let started = Instant::now();
let mut errors = 0;
while let Ok(r) = rx.recv_async().await {
    if r.result().is_err() { errors += 1; }
}
// started.elapsed() ≈ 300 ms, errors == 0: no `Timeout`, though the router
// never answered.
```
