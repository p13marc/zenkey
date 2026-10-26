# asr: an admin-space GET freezes a zenoh router, zenoh only (zenkey #890)

This directory reproduces zenkey #890 without zenkey. A GET on a zenoh
1.10.1 router's declaration records, such as `@/*/router/subscriber/**`,
is answered while the router holds its routing tables' read lock, and
each reply takes that lock again. A declaration, an interest or a new
session from another client between two replies deadlocks the router for
good.

- **On zenoh 1.10.1 it happens every time the conditions meet.** 135 of
  135 trials froze the router when they asked a key whose handler replies
  under the lock, with at least one record to send, beside a client
  declaring: every declaration record, `linkstate/*`, the storages
  selector beside records ending in `**`, and `@/**`. 107 of them froze at
  the first GET the churn met. With the writer flat out, every trial froze
  within 38 ms of the churn's start; at 10 rounds a second beside 500
  records, at the first GET; beside one record, after 0.3 to 19.6 s.
- **None froze without a writer, or on a key answered without the lock
  held across a reply:** 0 of 28 trials (`@/*/router`, `@/*/*`, no churn,
  `route/successor/**` and the two storages selectors with nothing to
  send).
- **The mechanism is established.** Every frozen router's stacks hold a
  replier inside `resources_data` or `linkstate_data` waiting on a second
  read, and a writer waiting on the write lock; and zenoh 1.10.1 with
  `fix.patch` froze in 0 of 138 trials, every cell of the matrix but the
  paced ones (G).
- **zenoh's main is the same:** `zenoh/src/net/runtime/adminspace.rs` at
  `766b45583` (GitHub, 2026-10-26) is byte-identical to 1.10.1's, and
  `route_send_response` still takes the read lock first. Main was read,
  not run.

The upstream report draft is
[`../adminspace-reply-under-tables-read-lock.md`](../adminspace-reply-under-tables-read-lock.md).

**This copy is the evidence kept in zenkey**, as `liveliness-get-repro/`
is #830's. It holds the program, the manifests, the patch, the script,
the condensed stacks of one frozen trial per writer, and the two results
files. It leaves out what can be rebuilt: the release builds
(`target-*/`), the patched zenoh source (`zenoh-1.10.1-fix/`), the full
gdb dumps and `scratch/`. Each manifest is a cargo project with its own
`[workspace]`, which the zenkey workspace does not list, so `cargo` at the
repository root never builds it. Run every command below from this
directory.

## Layout

| Path | What it is |
|---|---|
| `src/main.rs` | `asr`: `asr router` (the router, a child process) and `asr trial` (one trial, one `result` line) |
| `v1101/`, `vfix/` | The same source built against zenoh 1.10.1 (crates.io), and 1.10.1 with `fix.patch` |
| `fix.patch` | The intervention: `resources_data`, `linkstate_data` and `route_successor`'s shortcut collect their replies under the read lock and send them after it is dropped |
| `run-matrix.sh` | The measurement matrix; one fresh router per trial |
| `summary.py` | The tables below, from the result lines |
| `results-1.10.1.txt`, `results-fix.txt` | Raw `result` lines, one per trial, and a `# cell` line per cell |
| `stacks/` | The condensed stacks (one line per thread that touches the tables) of one frozen trial per writer |

## Build

```bash
cd v1101 && CARGO_BUILD_JOBS=3 cargo build --release --target-dir ../target-1.10.1
cd ../vfix && CARGO_BUILD_JOBS=3 cargo build --release --target-dir ../target-fix
```

`vfix/` patches zenoh through `[patch.crates-io]` with the path
`../zenoh-1.10.1-fix`. To make it, copy `zenoh-1.10.1` from the cargo
registry (`$CARGO_HOME/registry/src/index.crates.io-*/zenoh-1.10.1`) to
`zenoh-1.10.1-fix/`, make it writable, and run `patch -p2 < ../fix.patch`
inside it. Each build labels its result lines `ver=1.10.1` or
`ver=1.10.1+fix` (from the package name).

## The program

`asr trial` starts `asr router`: a zenoh router with its admin space on
(`adminspace.enabled: true`, read permitted), listening on an ephemeral
loopback port, in a process of its own. It then opens client sessions of
it in its own process:

- **load** declares `--load N` each of subscribers, queryables,
  liveliness tokens, publishers and queriers, on `asr/load/<i>` (`--shape
  concrete`) or `asr/load/<i>/**` (`--shape wild`);
- **churn** declares and undeclares, in a loop, a subscriber, a
  liveliness subscriber with history (an interest) and a liveliness token
  (`--churn all`), or one of them (`sub`, `lsub`, `token`), or opens and
  closes a session (`connect`), or nothing (`none`); `--churn-every MS`
  pauses between rounds;
- **ask** GETs `--selector` in a loop: target All, consolidation None,
  3 s timeout;
- **hb-pub** puts a heartbeat every 10 ms through the router, and
  **hb-sub** hears it.

Every session is open, and one GET of the selector has been answered,
before the churn and the asks start. The router is frozen when no
heartbeat has arrived for `--stall` seconds; the trial then takes the
router's stacks with `gdb -p` (`--stacks DIR`) and names the replying
thread's handler and the writer's frame. A trial that runs `--for`
seconds without a stall is clean. `t_ms` is the last heartbeat, from the
start of the churn and the asks: at or below 0, the first GET they met
froze the router.

```bash
./target-1.10.1/release/asr trial --selector '@/*/router/subscriber/**' --load 500 --stacks scratch/stacks
./run-matrix.sh 1.10.1 >> results-1.10.1.txt
./run-matrix.sh fix >> results-fix.txt
python3 -I summary.py results-1.10.1.txt results-fix.txt
```

## Results

A cell is one selector, load, shape, writer and pause; `hung` counts its
trials, `first GET's replies` is what the one GET before the churn was
answered with, `froze at` the median last heartbeat from the start of the
churn and the asks (at or below 0: the first GET they met), and `replier`
and `writer` the frames gdb found waiting. The lines the first build
wrote carry no `every_ms` (the pause came with cell G) and read as 0. Run
2026-10-26 on a 6-CPU VM (Debian 13, Linux 6.12), release builds, beside
the zenkey test suites.

| zenoh | selector | load | shape | churn | pause (ms) | hung | first GET's replies | froze at (ms, median) | replier | writer |
|---|---|---|---|---|---|---|---|---|---|---|
| 1.10.1 | `@/*/router/subscriber/**` | 0 | concrete | all | 0 | 10 of 10 | 1 | 0 | subscribers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 1 | concrete | all | 0 | 10 of 10 | 2 | -1 | subscribers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 10 | concrete | all | 0 | 10 of 10 | 11 | -2 | subscribers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 100 | concrete | all | 0 | 10 of 10 | 101 | -4 | subscribers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | all | 0 | 10 of 10 | 501 | -6 | subscribers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/publisher/**` | 500 | concrete | all | 0 | 5 of 5 | 500 | -1 | publishers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/queryable/**` | 500 | concrete | all | 0 | 5 of 5 | 500 | -4 | queryables_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/querier/**` | 500 | concrete | all | 0 | 5 of 5 | 500 | 6 | queriers_data | Face::send_declare, Face::send_interest |
| 1.10.1 | `@/*/router/token/**` | 500 | concrete | all | 0 | 5 of 5 | 500 | 0 | tokens_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | sub | 0 | 5 of 5 | 501 | 1 | subscribers_data | Face::send_declare, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | lsub | 0 | 5 of 5 | 501 | -2 | subscribers_data | Face::send_declare, Face::send_interest |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | token | 0 | 5 of 5 | 501 | -3 | subscribers_data | Face::send_declare, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | connect | 0 | 5 of 5 | 501 | -3 | subscribers_data | Gateway::new_transport_unicast |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | none | 0 | 0 of 5 | 501 | — | — | — |
| 1.10.1 | `@/*/router` | 500 | concrete | all | 0 | 0 of 5 | 1 | — | — | — |
| 1.10.1 | `@/*/*` | 500 | concrete | all | 0 | 0 of 5 | 1 | — | — | — |
| 1.10.1 | `@/*/router/linkstate/*` | 500 | concrete | all | 0 | 5 of 5 | 2 | -1 | linkstate_data | Face::send_interest, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/route/successor/**` | 500 | concrete | all | 0 | 0 of 3 | 0 | — | — | — |
| 1.10.1 | `@/*/router/**/storage_manager/storages/**` | 500 | wild | all | 0 | 10 of 10 | 1000 | -6 | queryables_data, subscribers_data | Face::send_declare, Face::send_interest, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/**/storage_manager/storages/**` | 500 | concrete | all | 0 | 0 of 5 | 0 | — | — | — |
| 1.10.1 | `@/*/router/status/plugins/storage_manager/storages/**` | 500 | wild | all | 0 | 0 of 5 | 0 | — | — | — |
| 1.10.1 | `@/**` | 500 | concrete | all | 0 | 5 of 5 | 2507 | -3 | linkstate_data, publishers_data, queryables_data, subscribers_data, tokens_data | Face::send_declare, Face::send_interest, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | sub | 1 | 5 of 5 | 501 | -4 | subscribers_data | Face::send_declare, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | sub | 10 | 5 of 5 | 501 | -2 | subscribers_data | Face::send_declare, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 500 | concrete | sub | 100 | 5 of 5 | 501 | -5 | subscribers_data | Face::send_declare, Face::with_mapped_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 0 | concrete | sub | 100 | 5 of 5 | 1 | 7681 | subscribers_data | Face::send_declare, Face::with_mapped_expr, Face::with_mapped_nullable_expr |
| 1.10.1 | `@/*/router/subscriber/**` | 5000 | concrete | sub | 100 | 5 of 5 | 5001 | 95 | subscribers_data | Face::send_declare, Face::with_mapped_nullable_expr |
| 1.10.1+fix | `@/*/router/subscriber/**` | 0 | concrete | all | 0 | 0 of 10 | 1 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 1 | concrete | all | 0 | 0 of 10 | 2 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 10 | concrete | all | 0 | 0 of 10 | 11 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 100 | concrete | all | 0 | 0 of 10 | 101 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 500 | concrete | all | 0 | 0 of 10 | 501 | — | — | — |
| 1.10.1+fix | `@/*/router/publisher/**` | 500 | concrete | all | 0 | 0 of 5 | 500 | — | — | — |
| 1.10.1+fix | `@/*/router/queryable/**` | 500 | concrete | all | 0 | 0 of 5 | 500 | — | — | — |
| 1.10.1+fix | `@/*/router/querier/**` | 500 | concrete | all | 0 | 0 of 5 | 500 | — | — | — |
| 1.10.1+fix | `@/*/router/token/**` | 500 | concrete | all | 0 | 0 of 5 | 500 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 500 | concrete | sub | 0 | 0 of 5 | 501 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 500 | concrete | lsub | 0 | 0 of 5 | 501 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 500 | concrete | token | 0 | 0 of 5 | 501 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 500 | concrete | connect | 0 | 0 of 5 | 501 | — | — | — |
| 1.10.1+fix | `@/*/router/subscriber/**` | 500 | concrete | none | 0 | 0 of 5 | 501 | — | — | — |
| 1.10.1+fix | `@/*/router` | 500 | concrete | all | 0 | 0 of 5 | 1 | — | — | — |
| 1.10.1+fix | `@/*/*` | 500 | concrete | all | 0 | 0 of 5 | 1 | — | — | — |
| 1.10.1+fix | `@/*/router/linkstate/*` | 500 | concrete | all | 0 | 0 of 5 | 2 | — | — | — |
| 1.10.1+fix | `@/*/router/route/successor/**` | 500 | concrete | all | 0 | 0 of 3 | 0 | — | — | — |
| 1.10.1+fix | `@/*/router/**/storage_manager/storages/**` | 500 | wild | all | 0 | 0 of 10 | 1000 | — | — | — |
| 1.10.1+fix | `@/*/router/**/storage_manager/storages/**` | 500 | concrete | all | 0 | 0 of 5 | 0 | — | — | — |
| 1.10.1+fix | `@/*/router/status/plugins/storage_manager/storages/**` | 500 | wild | all | 0 | 0 of 5 | 0 | — | — | — |
| 1.10.1+fix | `@/**` | 500 | concrete | all | 0 | 0 of 5 | 2507 | — | — | — |

`route/successor/**` on a lone router has no successor to send, so its
handler holds the read lock and replies nothing; the `src/<zid>/dst/<zid>`
shortcut, which replies under it, needs a routed mesh, which this
reproduction does not build. The storages selectors with nothing to send
(no record ending in `**`, or the status subtree with no storage manager)
take no second read.

## The stacks

`stacks/` keeps the condensed stacks of six frozen trials, one line per
thread that waits on the tables (the full gdb dumps are not committed).
Every frozen trial shows the same pair:

- the **replier**, an rx thread running the admin-space handler for the
  GET: `subscribers_data` (`adminspace.rs:832`) → `resources_data`
  (`:822`, the reply) → `route_send_response` (`queries.rs:557`) →
  `RwLock::read_contended`, or `linkstate_data` (`:873`) → the same;
- the **writer**: the other rx thread in `Face::send_declare` →
  `declare_subscriber` → `with_mapped_expr` (`face.rs:434`), in
  `Face::send_declare` → `undeclare_token` → `with_mapped_nullable_expr`
  (`face.rs:531`), or in `Face::send_interest` (`face.rs:549`); or
  `acc-0` accepting a session: `init_new_transport_unicast` →
  `Gateway::new_transport_unicast` (`gateway.rs:271`); each in
  `RwLock::write_contended`;
- `net-0`, `QueryCleanup`'s final reply: `route_send_response` →
  `RwLock::read_contended`.

With both rx threads blocked the router reads nothing off any link, so
the heartbeat's puts never reach `route_data`.
