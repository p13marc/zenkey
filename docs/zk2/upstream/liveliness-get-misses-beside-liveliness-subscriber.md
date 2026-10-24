# Draft upstream report: a liveliness GET misses the tokens its own session's liveliness subscriber holds

**Status: not filed.** Drafted on 2026-10-24 for zenkey #830 (chunk PS),
from a zenoh-only reproduction at zenoh 1.10.1 and zenoh main, and kept, as
the other drafts here are, as a record of behaviour zk2 designs around:
since core 0.37 (§8.1, Appendix B; #839) a reader makes the read that
establishes absence on a session that holds no liveliness subscriber of any
kind. #830 found the miss beside history subscribers; the reproduction
showed that a subscriber without history causes it too, so this report, and
its file name, speak of any liveliness subscriber.

The program is `lgr`, a zenoh-only reproduction committed beside this file,
in [`liveliness-get-repro/`](liveliness-get-repro/). Its `README.md` has the
commands, the full tables, and condensed traces of missing trials. The patch
below is its `fix.patch`.

---

**Title:** liveliness: a GET from a session that also holds a liveliness
subscriber completes without the tokens that subscriber already holds

**Version:**
- zenoh 1.10.1 (crates.io), and zenoh main at
  `766b4558337018e05c2db655254b8fd35ab22ab1` (2026-10-22). The code on the
  path below is identical in both.
- Client sessions, each connected to one router over TCP loopback, with the
  `unstable` feature.
- Every GET uses an unbounded handler.

**What happens.** `session.liveliness().get(sel)` returns a final reply with no
error reply and no timeout. It is missing live tokens matching `sel`, and
often every one of them. The cause is a liveliness subscriber on the same
session whose key expression covers those tokens, history or not.

*The deterministic case.*

```rust
// router on tcp/127.0.0.1:<port>; `holder` and `reader` are clients of it.
let _t = holder.liveliness().declare_token("demo/alive/1").await?;
// (a GET from `holder` confirms the token is at the router)
let _s = reader.liveliness().declare_subscriber("demo/**").history(true)
    .callback(|_| {}).await?;
let rx = reader.liveliness().get("demo/**").with(flume::unbounded()).await?;
// rx yields no reply and closes: complete, empty. The subscriber does receive the token.
// A second GET right after finds it.
```

Results, with a fresh reader in each of 20 rounds:
- **Stock 1.10.1 and main:** 20 of 20 first GETs are complete and empty.
- **Without `.history(true)`, or with the subscriber on another session:**
  0 of 20.
- **With the patch below:** 0 of 20.

*The general case.* A router, a client holding 10 tokens (zk2's shapes), and
a reader client with three liveliness subscribers covering them. The reader
makes a broad GET over all of them. Each trial is a fresh process, run 12 at
a time on 6 CPUs. Every cell is complete GETs that missed a token, over
trials:

| Where the liveliness subscribers are | Order | Wait before the GET | 1.10.1 | main |
|---|---|---|---|---|
| reading session, `history(true)` | tokens, then subscribers | none | **720/720** | **719/720** |
| reading session, `history(true)` | tokens, then subscribers | until the subscribers have every token | **7/3600** | **18/3600** |
| the same, 6 threads of CPU load | tokens, then subscribers | 100 ms | **16/720** | **16/720** |
| reading session, `history(true)` | subscribers, then tokens | none | **134/720** | **107/720** |
| reading session, `history(false)` | subscribers, then tokens | none | **128/720** | **118/720** |
| reading session, `history(false)` | subscribers, then tokens | until the subscribers have every token | **5/3600** | **24/3600** |
| the same, 6 threads of CPU load | subscribers, then tokens | until the subscribers have every token | **12/720** | **14/720** |
| reading session, `history(false)` | tokens, then subscribers | any, with or without load | 0/5760 | 0/5760 |
| another session of the same process, `history(true)` | any | any | 0/12240 | 0/12240 |
| none | any | any | 0/10080 | 0/10080 |

In all, 61,920 trials per version.
- **Two misses in a row are common.** A second GET at once misses again in
  about half of the misses that waiting does not prevent: 42 to 88 % of them
  are recovered, less under load.
- **A fresh session in the same process finds every token**, every time.

**Expected.** A liveliness GET that completes without an error reply returns
every live matching token, whatever liveliness subscribers the same session
holds.

**What works:**
- Holding the liveliness subscribers on another session, and making liveliness
  GETs from a session with no liveliness subscriber: 0 misses in 22,320 trials
  per version.
- Waiting after declaring the subscribers removes the deterministic case. It
  does not remove the other race (below).

**Cause** (1.10.1's line numbers; at main only `dispatcher/face.rs` moved, by 21
lines, and `hat/peer/interests.rs` by one). In the client's own runtime, the
session face is in the south region (`broker` hat) and the router link in the
north region (`client` hat).

1. **The GET is a `Current` token interest**, answered only by declarations
   carrying its id. `Session::liveliness_query` does not answer from known
   tokens (`api/session.rs:2934`: "the gateway resends current tokens on every
   query"). A `DeclareToken` with the query's id is a reply, and
   `DeclareFinal` ends the query (`api/session.rs:3222`, `:3330`).
2. **`Face::send_interest`** (`dispatcher/face.rs:545`, `dispatcher/interests.rs:271-332`)
   does three things:
   - forwards the interest north at once (`client` hat `route_interest`);
   - builds a *local answer* from the tokens the client runtime already knows
     (`send_current_tokens` with `other_token_matches`), which are the tokens
     the router declared to it for a liveliness subscriber;
   - delivers that local answer on the calling thread, after `ctrl_lock` is
     released (`face.rs:550-552`).
3. **The router answers with every matching token, then `DeclareFinal`.** In
   the client, each of those tokens goes through `propagate_current_token`
   (`dispatcher/token.rs:88`), which starts with:

   ```rust
   // hat/broker/interests.rs:388
   if self.face_hat(&dst).local_tokens.contains_key(&res) {
       tracing::debug!("Already propagated");
       return;
   }
   ```

   `local_tokens` holds what was already declared to the session face for a
   future interest, that is, for a liveliness subscriber. So the router's
   answer to the GET is dropped for every token a subscriber of that session
   holds, and the GET depends entirely on the local answer.

Two races follow, and a trace of a missing trial shows each.
- **In flight.** A token already sent to the client for the subscriber, but not
  yet processed when the GET is issued, is missing from the local answer.
  Being ahead on the link, it then fills `local_tokens` before the router's
  answer arrives, so that answer is dropped too. The GET returns exactly the
  tokens the subscribers held when it was issued.
  - `declare_subscriber(..).history(true)` followed at once by `get` hits this
    every time. The subscriber's `CurrentFuture` interest is answered first, on
    the same link.
  - A non-history subscriber hits it only for tokens declared after it
    subscribed. A `Future` interest is not sent the existing tokens.
- **Overtaken final.** The local answer holds every token, but it is delivered
  on the calling thread after the lock is released. The router's `DeclareFinal`
  is delivered on a receive thread. When the caller is descheduled for longer
  than a router round trip, the final arrives first and the GET completes with
  none of the tokens, or some of them. This race persists at any wait, and
  CPU load raises the rate.

**Patch** (`fix.patch`): skip a token already in `local_tokens` only for a
future-mode interest, which is what `local_tokens` deduplicates. A `Current`
interest's tokens are always sent, tagged with its id. They then reach the GET
ahead of the router's `DeclareFinal`, on the same receive thread.

```diff
-        if self.face_hat(&dst).local_tokens.contains_key(&res) {
+        if interest.mode.is_future() && self.face_hat(&dst).local_tokens.contains_key(&res) {
             tracing::debug!("Already propagated");
             return;
         }
```

**Measured effect.**
- **Patched 1.10.1:** 0 misses in 6,480 trials, over the nine cells where
  stock 1.10.1 misses 1,052, and 0 of 120 in the deterministic case. A GET may
  now receive a token twice, from the local answer and from the router. A GET
  from the token's own session already does on stock 1.10.1.
- `hat/peer/interests.rs:614` has the same guard. Not measured: every session
  here is a client.

**Two more things on this path:**
- **Late declarations reach the subscriber path, patched or not.**
  - A local-answer declaration that arrives after its query's `DeclareFinal`
    falls through to the subscriber path of the `DeclareToken` handler
    (`api/session.rs:3247`). It goes in under token id 0, which every
    `Current` reply uses.
  - Measured: one extra Put to history subscribers, in 134 of 4,320 patched
    trials of the history shape.
  - Suspected, not measured: the session keeps that key in `remote_tokens`
    under id 0 after the token is undeclared under its real id. A history
    subscriber declared later on that session would then be handed it as
    live (`api/session.rs:2065`). The fix: drop a `DeclareToken` whose
    interest id names a finished query, or deliver a `Current` interest's
    local answer before its final.
- **Undeclaring the subscriber does not end it** (suspected, from the code;
  not measured). `unregister_interest` (`hat/broker/interests.rs:464`)
  removes the interest and leaves `local_tokens` as it is; an entry leaves
  only when its token is undeclared, or when the face closes (`mod.rs:188`).
  So a session whose liveliness subscriber has come and gone would still
  drop the router's answer for every token that subscriber was sent.
- **#2678 would widen the overtaken-final race.** That open PR moves
  `send_interest`'s replay onto a spawned blocking task. Without the patch,
  the local answer would then race the router's `DeclareFinal` on every GET
  from a session with a subscriber.

**Repro:** `lgr`, kept with this report. It builds the same sources against
1.10.1, main and the patched 1.10.1, and runs:
- `lgr drive --jobs 12 --trials 720 --shape A|B|C|D --order tokens-first|subs-first --wait 0|10|100|samples|narrow [--burn 6] [--shared]`
  for one cell;
- `run-matrix.sh` for the whole matrix;
- `lgr-minimal --rounds 20 [--no-history] [--other-session]` for the
  deterministic case.

Its `traces/excerpt-*.txt` show both races, condensed from
`RUST_LOG=zenoh::net::routing=debug,zenoh::api::session=trace`.
