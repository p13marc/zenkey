# Budget scenarios (core §2.7)

§1–§3 are the measurements "lost no delivery" rests on (§2.7, Appendix B),
run as the spike ran its own: sessions on loopback, routers linked R2 → R1,
each number recorded. §4–§7 are an owner keeping its bound and its rate,
which a black-box runner drives through the reference owner
(`zenkey/examples/owner.rs`: `--cardinality`, and the `put`, `delete` and
`fill` commands).

The contract of §2 and §4–§7, `budget.v1`:

```toml
[interface]
name = "budget"
major = 1

[resources."items/{item}"]
kind = "state"
type = { raw = "text/plain" }
params = { item = "string" }
cardinality = 3

[resources."load/{cpu}"]
kind = "stream"
type = { raw = "text/plain" }
params = { cpu = "string" }
cardinality = 3
reliability = "reliable"
congestion = "block"

[resources."alarms/{src}"]
kind = "event"
type = { raw = "text/plain" }
params = { src = "string" }
cardinality = 2
rate = "burst(3/h)"
retention = "2s"
```

## §1 A reliable, blocking path loses nothing to a slow reader

**Setup.** R2 → R1, **each a process of its own**. An owner, a client of R2,
holds an instance token. A consumer, a client of R1, subscribes to the
owner's key and takes 2 ms over each sample in its callback. Beside it, it
holds a liveliness subscriber on the owner's instance tokens, watches its
own session's transport events, and runs a ticker on a thread of its own.

**Steps.** The owner puts 1,000 numbered samples of 16 KiB as fast as its
publisher lets it:
1. reliable, `block`;
2. best effort, `drop`;
3. reliable, `block`, the consumer's callback stuck for 7 s on the 100th;
4. reliable, `block`, the consumer in a process of its own, and that
   process stopped (SIGSTOP) for 7 s at the 100th, then continued. The
   owner's token reaches the consumer before the samples start.

**Expected.**
1. All 1,000 arrive, in order, and the owner's token is never withdrawn.
   The owner's put waits for the path.
2. Some never arrive, and nothing the consumer receives says so.
3. The consumer's router gives up on it after `wait_before_close` (5 s) and
   closes its transport. Samples are lost. The consumer's session sees its
   own transport close. It sees the owner's token withdrawn only if the
   token had reached it before it stalled. Its ticker sees no gap: only
   zenoh's receive thread was held.
4. Samples are lost. The consumer's ticker sees a gap of at least 7 s on
   its first tick after the stop. Its session sees its transport close,
   and it sees the owner's token withdrawn.

*Measured 2026-10-10: 1,000 of 1,000; 695–832 lost; 691–794 lost.
Measured 2026-10-11:*
- *step 3: 433–786 lost, the consumer's transport closed in 27 of 27
  runs, the owner's token withdrawn in each of the 23 where it had
  reached the consumer and in none of the 4 where it had not, the
  ticker's worst gap under 107 ms;*
- *step 4: 421–765 lost in 13 runs, the ticker's gap 7.0–7.1 s, the
  transport's close read 0.35–0.51 s after it (4 runs timed), the token
  withdrawn each time.*

*Six earlier runs of step 3 saw no withdrawal in 2.*

The reference (`zenkey/tests/budget.rs`) asserts only the loss in step 3,
and the loss and the ticker's gap in step 4, and reports the rest for the
record.

**Why each router has a process.** The sessions of one process share
zenoh's receive threads, two by default. With both routers and the slow
consumer in one process, a router's push waiting on a full `block` queue
holds one of them, and the consumer's own receive task starves. 3 of 6 such
runs lost 674–970 of 1,000 reliable, `block` samples, the routers' transport
closes stalled, and no token was withdrawn while the run lasted. That is a
measurement of the process, not of the path, and §2.7 names it as a
premise a deployment provides.

**A tool's window over a stalled reader** (core §2.7). The case
§2.7's "its reader lost none" (observer §5.2) exists for, read by a tool.
- *Setup.* An owner of an event of two sources at most, `reliable` with
  `block` and a 2 s retention, publishing both every 200 ms, its instance
  token held throughout. A tool reads its population over a 4 s window,
  1 s reply timeout and 1 s lease, through a proxy it connects by.
- *Steps.* Once the tool's subscription has reached the owner, the proxy
  closes the tool's connection; the tool reconnects.
- *Expected.* The window is not lossless: its population and its rate
  are unobservable, the reason naming the tool's own transport closing,
  whatever it heard. The same window with the connection left alone is
  clean.

*The reference runs it as `zenctl check conform`
(`zenctl/tests/live_zk2.rs`,
`check_conform_reads_a_window_unobservable_when_its_own_transport_closes`).*

## §2 A failed path withdraws the owner's token

**Setup.** R2 → R1 through a link that can be closed, or stalled: its
forwarding stops and nothing closes, as a pulled cable or a radio out of
range. A consumer, a client of R1 through a second such link. Both routers
and the consumer run a 1 s lease. The owner, a client of R2, brings up
`budget.v1` and puts on `stream/load/c0` every 10 ms.

**Steps.** For each link in turn: close it, then heal it; stall it, then
heal it.

**Expected.**
- Closed: the consumer sees the owner's instance token withdrawn at once.
- Stalled: it sees it withdrawn one lease after the stall began, and the
  samples put meanwhile never arrive.
- The same between the routers and on the consumer's own link.

*Measured 2026-10-10: closed, 0.8–2.0 ms; stalled, 986–1,037 ms, with 88–93
samples lost each time. The spike measured the default 10 s lease: a frozen
peer was dropped after 9,961–10,047 ms (S10).*

## §3 Miss detection cannot see a publisher it never heard

**Setup.** R2 → R1, linked through a link that can be closed. An owner, a
client of R2, with an advanced publisher declaring sample-miss detection
(§2.5). A consumer, a client of R1.

**Steps.**
1. The owner puts five samples, and no subscriber is declared.
2. The consumer declares an advanced subscriber with a miss listener. The
   owner puts until the consumer hears one.
3. The link is closed, the owner puts three samples, and the link is
   healed. The owner puts until the consumer hears one again.

**Expected.**
1. —
2. The first sample heard is the sixth or later, and no miss is reported:
   zenoh-ext takes a publisher's first sample heard as its start.
3. A miss is reported: a gap after a sample heard is detected.

*Measured 2026-10-10: the first heard was the seventh, 0 misses reported;
after the cut, 17 (the three put while it was closed, and those put before
the subscription crossed again).*

## §4 An owner keeps its bound: a state

**Setup.** An owner of `budget.v1`, `state/items/{item}` bound 3, the
contract's.

**Steps.**
1. Put `a`, `b`, `c`.
2. Put `d`.
3. Put `a` again.
4. Delete `b`, then put `d`.
5. Another owner, its descriptor lowering the bound to 2: put `a`, `b`,
   then `c`.

**Expected.**
1. Accepted: three live members, at the bound.
2. Refused, nothing published. The refusal names the resource, the bound
   and its source, the contract's.
3. Accepted: a put to a live key adds no member.
4. Accepted: a delete frees a member.
5. `c` is refused, naming the bound 2 and its source, the descriptor's.

## §5 An owner keeps its bound: a stream

**Setup.** An owner of `budget.v1`, `stream/load/{cpu}` bound 3.

**Steps.**
1. Publish on `c0`, `c1`, `c2`.
2. Publish on `c3`.
3. Publish on `c0` again.

**Expected.**
1. Accepted.
2. Refused: three members published within the last hour are live.
3. Accepted: `c0` is live. A member is forgotten an hour after its last
   publication, and a publication then is a new member again.

## §6 An owner keeps its bound: an event

**Setup.** An owner of `budget.v1`, `events/alarms/{src}` bound 2, its
retention 2 s.

**Steps.**
1. An occurrence of `a`, then of `b`.
2. An occurrence of `c`.
3. Wait 2 s; an occurrence of `c`.

**Expected.**
1. Accepted.
2. Refused: `a` and `b` have occurrences within the retention.
3. Accepted: `a` and `b` are no longer live.

## §7 An owner keeps its rate

**Setup.** An owner of `budget.v1`, `events/alarms/{src}` at
`burst(3/h)`.

**Steps.**
1. Three occurrences of `a`.
2. A fourth occurrence of `a`.
3. An occurrence of `b`.

**Expected.**
1. Accepted.
2. Refused, nothing published. The refusal names the rate: a fourth
   occurrence of one member within an hour.
3. Accepted: the rate bounds each member.
