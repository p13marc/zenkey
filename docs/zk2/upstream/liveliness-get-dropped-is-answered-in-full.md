# Draft upstream report: a liveliness GET its caller stops waiting for is still answered in full

**Status: not filed.** Drafted on 2026-10-25 for zenkey #882 (chunk WF),
from the reference's measurements at zenoh 1.10.1. It is kept, as the
other drafts here are, as a record of behaviour zk2 designs around. Since
#882 the reference never drops a liveliness GET it has asked:
`zenkey::presence::Barrier` runs its round trip on a task of its own that
every waiter shares, and `zenkey_fleet::PresenceFeed` and zenwatch's token
ledger run each re-read's round trip and read on tasks that outlive the
call that started them. Core 0.44 (§8.1, F-239) records the second
behaviour below, which zk2 relies on.

The measurements:
- `zenkey-fleet/tests/watch_reopen.rs`: its ignored
  `wf_zenctl_watchdog_after_a_reopen_beside_the_crowd_measured`, with a
  zenctl built before #882, and `wf_the_feed_after_a_reopen_beside_the_crowd_measured`
  with `WF_COMPETE=250` on the same code;
- the same file's `wf_the_reopen_tells_every_token_and_the_round_trip_waits_measured`,
  for the second behaviour.

---

**Title:** liveliness: a GET whose caller stops waiting is still answered
in full, and after `routing/interests/timeout` every reply to it logs
`Unknown interest`

**Version:**
- zenoh 1.10.1 (crates.io), with the `unstable` feature.
- One router and client sessions on TCP loopback.
- The code cited below reads the same on zenoh's main branch at
  173b1220c. That branch was read, not run.

**What happens.** A client session asks a liveliness GET, a `Current`
interest (`api/session.rs`, `liveliness_query`, line 2947). Its caller can
stop waiting for it in three ways: it drops the reply receiver, the GET's
own timeout fires, or its cancellation token is cancelled. None of them
tells the router:
- the timeout task removes the query from the session's state and calls
  the callback with a `Timeout` error (lines 2911–2931); nothing is sent;
- `cancel_liveliness_query` removes the query from the session's state and
  sends nothing either (lines 2959–2966). It also returns an error exactly
  when it found the query: `Some(_) => bail!("Unable to find liveliness
  query {qid}")`;
- a dropped receiver changes nothing at all.

So the router answers the GET in full, every matching token and the
`DeclareFinal`. The client's runtime keeps the GET's
`pending_current_interests` entry until that final arrives, or until
`routing/interests/timeout`, when `CurrentInterestCleanup` gives up on it
and logs `Didn't receive DeclareFinal for interest …: Timeout(…)!`
(`net/routing/dispatcher/interests.rs`, line 167). Every token reply that
arrives after that cleanup, and the final, finds no entry, and
`route_current_token` and `route_declare_final` log `Unknown interest` at
error level, once per reply (`net/routing/hat/client/interests.rs`, lines
239 and 277).

*Measured.* A reader asked a liveliness GET of 10,000 tokens and dropped
it 250 ms later, four times a second, as a `select!` loop does when
another branch wins (zenkey #882). Each GET takes 0.5 to 3 s to answer on
its own, so none was ever answered before it was dropped, and the
router's queue grew:

| | |
|---|---|
| GETs asked in 120 s, none read to its end | 450 |
| `Didn't receive DeclareFinal … Timeout(120s)!` in the 300 s after the first | 139 |
| `Unknown interest` in the same 300 s | 700,070 |

That is about 70 GETs answered in full, 10,000 tokens each, after the
runtime had given up on them. Beside 2,000 tokens, where each answer fits
inside 250 ms more often than not, the reader recovered.

**Expected.**
- A liveliness GET that its caller drops, that times out, or that is
  cancelled sends an `Interest` `Final` for it, as undeclaring a
  subscriber's interest does (`api/session.rs`, line 1666), and the
  router stops answering it.
- At the least, `cancel_liveliness_query` returns `Ok` when it removed the
  query, and the replies to an interest the runtime has finalized are
  dropped quietly, or logged once per interest.

**Workaround.** Never drop a liveliness GET you have asked: run it on a
task of its own, and have a later caller take up the one in flight rather
than ask another.

---

**A second behaviour, recorded rather than reported.** When a client's
transport opens again, `new_transport_unicast_face` re-sends its session's
interests (`hat/client/mod.rs`, line 157), and `repropagate_interests`
sends each as `CurrentFuture` (`hat/client/interests.rs`, line 71),
whatever mode it was declared in. A liveliness subscriber declared without
history is a `Future` interest (`api/session.rs`, lines 2102–2106), and is
told at the reopen every token its selector matches, those declared while
the link was down included; a token withdrawn while it was down is not
told. A liveliness GET asked after the reopen, on that session, ends only
once the router has sent all of them:

| Round trip of a key no token holds | 2,000 tokens | 10,000 tokens |
|---|---|---|
| at the session's start | 42–46 ms | 49–66 ms |
| after a cut and a heal, release build | 149–151 ms | 3.9–4.1 s |
| after a cut and a heal, debug build | 1.1 s | 24 s |

zk2 relies on the telling: it is how a reader without history learns what
was declared while its link was down. What it costs is the population,
again at every reopen. A note in the liveliness subscriber's documentation
that a transport's reopening tells it what it matches, whatever its
`history`, would let other applications rely on it too.
