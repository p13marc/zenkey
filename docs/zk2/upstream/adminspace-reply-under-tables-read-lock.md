# Draft upstream report: the admin space replies under the routing tables' read lock, and a declaration in between freezes the node

**Status: not filed.** Drafted on 2026-10-26 for zenkey #890 (chunk WJ),
from a reproduction with zenoh alone,
[`adminspace-reply-repro/`](adminspace-reply-repro/). It is kept, as the
other drafts here are, as a record of behaviour zk2 designs around. Since
#890 the reference asks none of these keys unless its user does: zengui's
declared-entities sweep is opt-in (`--admin-declared`), and the storages
read names the storage manager's status subtree. Core 0.48 (§4.2,
Appendix B) records the behaviour. Since #894 (core 0.50, §11.1) the admin
read `zenctl acl gen` grants a tool admits none of them under `deny`.

---

**Title:** admin space: `resources_data`, `linkstate_data` and
`route_successor` reply while holding the routing tables' read lock, so a
declaration from another face between two replies deadlocks the node

**Version:**
- zenoh 1.10.1 (crates.io), router mode, the admin space enabled
  (`adminspace.enabled: true`, `permissions.read: true`), Linux, Rust
  1.98.1's std.
- Client sessions on TCP loopback.
- The code below reads the same on zenoh's main branch at `766b45583`
  (GitHub, 2026-10-26): `resources_data`, `linkstate_data`,
  `route_successor` and `route_send_response` are unchanged. Main was
  read, not run.

**What happens.** A GET on a node's declaration records, for example
`@/*/router/subscriber/**`, reaches `AdminSpace::send_request`, which runs
every handler whose key the selector intersects (`adminspace.rs`, lines
556–559). `route_query` has dropped the tables' lock before it calls the
admin face (`dispatcher/queries.rs`, lines 278–279, 341). The handlers for
`subscriber`, `publisher`, `queryable`, `querier` and `token` all run
`resources_data` (`adminspace.rs`, lines 808–829):

```rust
let rtables = zread!(tables.tables);          // line 813: read lock held
for res in f(&rtables) {
    ...
    query.reply(key, payload)...wait()          // line 822: reply under it
}
```

Each reply goes through `Face::send_response` to `route_send_response`
(`dispatcher/queries.rs`, line 552), which takes the same lock again,
`zread!(tables_ref.tables)` at line 557. The lock is std's `RwLock<Tables>`.
On Linux a new reader does not get it while a writer waits
(`library/std/src/sys/sync/rwlock/futex.rs`, `is_read_lockable`: "writers
… have priority over readers"). So if any thread asks for the write lock
between the first read and a later one, the node deadlocks:
- the replying thread holds a read and waits for a second read, queued
  behind the writer;
- the writer waits for that first read to go.

Writers are everyday events: a client declaring or undeclaring a
subscriber or a token (`Face::send_declare`, `face.rs`, line 563), a
liveliness subscriber's interest (`Face::send_interest`, line 549), and a
session connecting (`Gateway::new_transport_unicast`, `gateway.rs`, line
271). After that, every thread that touches the tables blocks for good:
both rx threads (so the node reads nothing off any link), puts
(`route_data`), and `QueryCleanup`'s final replies. The GET that started
it ends at its own timeout, so the asker sees nothing wrong.

`linkstate_data` (lines 858–879) has the same shape: its read at line 860
is held across its replies at line 873. `route_successor` (lines 882–927)
does too for its `src/<zid>/dst/<zid>` shortcut (the reply at line 902,
under the read taken at 893); its full listing drops the lock first (line
915). The root document (`local_data`), `metrics`, `plugins` and
`status/plugins` take no tables lock across a reply.

*Measured* with `adminspace-reply-repro` (one router process; a client
holding 500 each of subscribers, queryables, liveliness tokens,
publishers and queriers; a client churning declarations; a client asking
one admin selector in a loop; a heartbeat put through the router every
10 ms; frozen = no heartbeat for 5 s, confirmed by `gdb`):

| Asked | Writer | Records | Froze |
|---|---|---|---|
| `@/*/router/subscriber/**` | a subscriber, a liveliness subscriber and a token declared and undeclared, flat out | 0, 1, 10, 100 or 500 (1 to 501 replies: the heartbeat's subscriber too) | 50 of 50, at the first GET or within 38 ms |
| `…/publisher/**`, `…/queryable/**`, `…/querier/**`, `…/token/**` | the same | 500 each | 20 of 20 |
| `…/subscriber/**` | a subscriber alone, a liveliness subscriber alone, a token alone, or a session opened and closed | 500 | 20 of 20 |
| `…/subscriber/**` | one subscriber declared and undeclared every 1, 10 or 100 ms | 500 | 15 of 15, at the first GET |
| `…/subscriber/**` | every 100 ms | 0 (1 reply) | 5 of 5, after 0.3 to 19.6 s (median 7.7 s) |
| `…/subscriber/**` | every 100 ms | 5,000 | 5 of 5, within 0.1 s: one at the first GET, four after a few |
| `@/*/router/linkstate/*` | flat out | (2 replies) | 5 of 5 |
| `@/*/router/**/storage_manager/storages/**` | flat out | 500 subscribers and queryables on `…/**` (1,000 replies) | 10 of 10 |
| the same | flat out | 500 on concrete keys (no reply) | 0 of 5 |
| `@/**` | flat out | 500 (2,507 replies) | 5 of 5 |
| `…/subscriber/**` | none | 500 | 0 of 5 |
| `@/*/router`, `@/*/*` | flat out | 500 | 0 of 10 |

Every frozen trial's stacks (135 of 135) show the same pair, with
`QueryCleanup` queued behind them:
- the replier, an rx thread: `subscribers_data` (or one of the other
  four) → `resources_data` (`adminspace.rs:822`), or `linkstate_data`
  (`:873`), → `route_send_response` (`queries.rs:557`) →
  `RwLock::read_contended`;
- the writer: the other rx thread, `Face::send_declare` /
  `Face::send_interest` → `with_mapped_expr` (or
  `with_mapped_nullable_expr`) → `RwLock::write_contended`,
  or `acc-0`: `init_new_transport_unicast` → `Gateway::new_transport_unicast`
  (`gateway.rs:271`) → `RwLock::write_contended`;
- `net-0`: `QueryCleanup` → `route_send_response` → `read_contended`.

We met it in an explorer's boot (`@/*/*/{subscriber,publisher,queryable,
querier,token}/**`) against an in-process router: 3 runs of 40 froze with
eight test processes running at once, none alone, and 4 of 40 when we
measured again (0 of 80 once the explorer stopped asking). A selector a
tool reads for another purpose reaches the same handlers whenever it
intersects them: `@/*/router/**/storage_manager/storages/**` intersects
`@/<zid>/router/{subscriber,queryable}/**`, is answered with the record
of every subscriber and queryable ending in `**`, and froze the router in
10 of 10 trials above.

**Expected.** An admin-space GET never deadlocks the node, whatever is
declared meanwhile.

**Proposed fix.** Collect the replies under the lock, and send them after
it is dropped, in `resources_data`, `linkstate_data` and
`route_successor`'s shortcut (`adminspace-reply-repro/fix.patch`, 120
lines):

```rust
let replies: Vec<(OwnedKeyExpr, ZBytes)> = {
    let rtables = zread!(tables.tables);
    f(&rtables)
        .into_iter()
        .filter_map(|res| { /* the key and payload, if the query intersects */ })
        .collect()
};
for (key, payload) in replies {
    query.reply(key, payload).encoding(Encoding::APPLICATION_JSON).wait()?;
}
```

With it, zenoh 1.10.1 froze in 0 of 138 trials, every cell above but the
paced ones, three to ten trials each. Beside 500 subscribers and the
churn, the GET loop finished 4,000 to 4,600 GETs in each 15 s trial,
where stock 1.10.1's first one ended at its timeout. The replies are the
same ones, sent a moment later; a GET still sees the tables as they stood
under the lock.

**What works without the fix.** Not asking those keys. A GET on a root
document (`@/*/router`, `@/*/*`) never froze a router here (0 of 10 beside
the churn): `local_data` takes the read lock once, in `get_links_info`,
and drops it before it replies. `metrics`, `plugins` and `status/plugins`
take no tables lock across a reply (read, not measured: the reproduction
runs no plugin). A storage listing can name the status subtree whole,
`@/*/router/status/plugins/storage_manager/storages/**`, which reaches
none of the locked handlers.

**Access control keeps a principal off them** (zenkey #894). zenoh's
access control admits a query only when an allowed key expression
includes the query's (`policy_decision_point`, `nodes_including`), and a
query it refuses on ingress is answered with its final response at once,
never routed (`primitives/demux.rs`, `handle_message`). So under
`default_permission: deny`, a principal granted no key that intersects the
locked handlers never reaches them, whatever it asks. Measured with
`adminspace-reply-repro/run-acl.sh`, beside the same 500 records and churn:
a principal granted `@/*/router` and `@/*/router/**` froze the router in
45 of 45 trials (every declaration record, `linkstate/*`, `@/*/router/**`,
the `**` storages selector); one granted `@/*/router`,
`@/*/router/status/plugins/storage_manager/storages/**` and `@/*/*`, in
0 of 70 (the same selectors and `@/**`, beside the churn and beside each
writer alone; 6.9 million GETs, each refused with no reply), while its
own reads of those three stayed answered. Under
`default_permission: allow` there is no such defence: a deny applies by
inclusion too, so `@/**` is in none narrower than itself.

**Not identified.** Whether a writer can also slip in on a peer's or a
client's admin space (their tables have the same lock; the reproduction
measured a router only), and whether any other caller of
`route_send_response` holds the tables' lock across it.

**Repro:** `adminspace-reply-repro/` (README.md there): build `v1101/`,
run `asr trial` for one trial or `run-matrix.sh 1.10.1` for the table
above; `vfix/` builds the same program against 1.10.1 with `fix.patch`.
