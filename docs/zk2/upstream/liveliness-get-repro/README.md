# lgr: a liveliness GET beside liveliness subscribers, zenoh only (zenkey #830)

This directory reproduces zenkey #830 without zenkey. A liveliness GET made from
a client session that also holds a liveliness subscriber can complete (a final
reply, no error reply, no timeout) while it misses tokens that are live. The
subscriber can have history or not.

- **It happens on zenoh 1.10.1 and on zenoh main** (git 766b455, 2026-10-22).
  The code on that path is the same in both.
- **Shapes C and D never miss.** C holds the subscribers on another session,
  and D holds none. That is 0 misses in 22,320 trials per version.
- **The mechanism is established.** The trace of a missing trial shows it, and
  so does an intervention: zenoh 1.10.1 with a one-condition patch
  (`fix.patch`) misses 0 of 6,480 trials in the cells where stock 1.10.1
  misses 1,052.

The upstream report draft is
[`../liveliness-get-misses-beside-liveliness-subscriber.md`](../liveliness-get-misses-beside-liveliness-subscriber.md).

**This copy is the evidence kept in zenkey** (#830, chunk PS), as
`docs/zk2/spike-results/` is the spike's. It holds the program, the
manifests, the patch, the scripts, the condensed trace excerpts and the
three results files, compressed. It leaves out what can be rebuilt or was
exploratory: the release builds (`target-*/`), the patched zenoh source
(`zenoh-1.10.1-fix/`), the raw traces, and `scratch/`. Each manifest is a
cargo project with its own `[workspace]`, which the zenkey workspace does
not list, so `cargo` at the repository root never builds it. Run every
command below from this directory.

## Layout

| Path | What it is |
|---|---|
| `src/main.rs` | `lgr`: the measuring program (`trial`, `drive`, `router`, `holder`, `summary`) |
| `src/minimal.rs` | `lgr-minimal`: the deterministic case in about 100 lines |
| `v1101/`, `vmain/`, `vfix/` | The same sources built against zenoh 1.10.1 (crates.io), zenoh main (git rev `766b4558337018e05c2db655254b8fd35ab22ab1`), and 1.10.1 with `fix.patch` |
| `zenoh-1.10.1-fix/` | Not committed: the crates.io source of zenoh 1.10.1 with `fix.patch` applied, used by `vfix/` through `[patch.crates-io]` (see Build) |
| `fix.patch` | The intervention: one condition in `hat/broker/interests.rs` |
| `target-1.10.1/`, `target-main/`, `target-fix/` | Not committed: the release builds |
| `run-matrix.sh` | The measurement matrix, in parts that each take under ten minutes |
| `summary.py` | The tables below, computed from the result lines |
| `results-1.10.1.txt.xz`, `results-main.txt.xz`, `results-fix.txt.xz` | Raw `result` lines (one per trial) and a `# drive` line per cell, xz-compressed. Each file ends with its own summary as `#` lines. `summary.py` reads them compressed; `xz -dk` gives the text a new run appends to. |
| `traces/` | The condensed excerpts of traced missing trials (`excerpt-*.txt`, made by `excerpt.sh` and `condense.py`); the raw traces are not committed |
| `scratch/` | Not committed: exploratory runs from earlier revisions of the program, which lacked some fields; not in the tables. Also `minimal-*.txt`: the one-token, one-subscriber cells. |

## Build

```bash
cd v1101 && RUSTC_WRAPPER= CARGO_BUILD_JOBS=3 cargo build --release --target-dir ../target-1.10.1
cd ../vmain && RUSTC_WRAPPER= CARGO_BUILD_JOBS=3 cargo build --release --target-dir ../target-main
cd ../vfix && RUSTC_WRAPPER= CARGO_BUILD_JOBS=3 cargo build --release --target-dir ../target-fix
```

Each build makes `lgr` and `lgr-minimal`, and each labels its result lines
`ver=1.10.1`, `ver=main` or `ver=1.10.1+fix` (from the package name). To make
`zenoh-1.10.1-fix/` again, copy `zenoh-1.10.1` from the cargo registry
(`$CARGO_HOME/registry/src/index.crates.io-*/zenoh-1.10.1`), make it writable,
and run `patch -p2 < ../fix.patch` inside it.

## The program

### `lgr trial`: one measurement in a fresh process

1. **The router.** With no `--connect`, the trial opens its own router on
   `tcp/127.0.0.1:0` in process (`mode=inproc`). With `--connect EP`, it uses a
   shared router process (`mode=shared`, set up by `drive --shared`).
2. **The sessions.** It opens the reader client session, a second client
   session for shape C, and a holder client session. On a shared router with
   `tokens-first`, the holder is instead the shared holder process.
3. **The tokens.** The holder declares zk2-shaped tokens for 3 services under
   `zk2/h1/`, 10 tokens in all (`--services N` changes the count):
   - for each service, `…/@zk/instance/<id>`;
   - for most services, two `…/@zk/alive/<iface>/<id>/<fp16>` tokens;
   - for every third service, one `alive` token and two
     `…/@zk/member/<iface>/<m>/<id>` tokens instead.
4. **The order** decides when the holder declares (`--order`):
   - `tokens-first` (default): the holder declares its tokens, then makes one
     narrow GET per instance from its own session, so every token is at the
     router before the reader subscribes.
   - `subs-first`: the reader subscribes first and makes a GET round trip
     through the router. Then the holder declares its tokens and makes the same
     narrow GETs. The subscribers learn the tokens as live declarations.
5. **The shape** decides where the liveliness subscribers are (`--shape`). There
   is one subscriber per selector: `zk2/*/*/@zk/instance/*`, `zk2/*/*/@zk/alive/**`
   and `zk2/*/*/@zk/member/**` (`--subs instance,alive,member`; `broad` stands
   for `zk2/*/*/@zk/**`). Every subscriber is a callback.
   - A: history subscribers on the reading session;
   - B: non-history subscribers on the reading session;
   - C: history subscribers on the second session;
   - D: none.
6. **The wait** between declaring the subscribers and the GET (`--wait`):
   - `0`, `10` or `100` ms;
   - `samples`: until the subscribers have seen every held token;
   - `narrow`: until a narrow GET per instance from the reader finds its
     instance, as zenkey's test did.
7. **The reads**, all from the reader's session:
   - the measured broad GET on `zk2/*/*/@zk/**`;
   - a second broad GET at once;
   - one narrow GET per service, `zk2/h1/svcN/@zk/instance/*`;
   - then a broad GET from a fresh session in the same process.

   Every GET has a 1 s timeout and an unbounded flume handler (zenoh#2678), so
   a full channel is never the cause.

### The result line

| Field | Meaning |
|---|---|
| `class` | `complete` (every held token), `MISSED` (complete but missing at least one), `incomplete` (an error reply, e.g. timeout). No trial in any file was incomplete or failed. |
| `first`, `second`, `fresh` | `c:<found>/<held>` for complete, or `i:` for an error reply |
| `missing`, `second_missing` | The tokens each GET missed, as `svcN:kind` |
| `narrow` | Narrow GETs that found their instance, as `<found>/<services>` |
| `seen_at_get` | Distinct keys the trial's subscribers held when the GET started. On a shared router with `subs-first`, this includes other trials' tokens. |
| `puts_at_get`, `puts_end` | Put samples the subscribers received by the GET and by the end. In process, more than `held` means a token reached them twice (see "Secondary effects"). |
| `declared_ms`, `waited_ms`, `first_ms` | The time to declare the subscribers, the time from the subscribers to the GET, and the first GET's duration |

### `lgr drive`: the load

`drive --trials N --jobs 12` runs N trials, 12 child processes at a time, and
appends every result line to `--out`. It ends a cell with one
`# drive ver=… <flags> trials= secs= loadavg_start= loadavg_end= counts={…}` line.

Its other flags:
- `--burn N` adds N spinning threads in the driver for the whole cell
  (`load=burn6`).
- `--shared` starts one router process and one holder process for every trial
  in the cell. A trial is then only a fresh reader process.
- `--trace-dir D` keeps a child's whole output for each miss. Set `RUST_LOG`
  for it, e.g. `RUST_LOG=zenoh::net::routing=debug,zenoh::api::session=trace`.

The host has 6 CPUs and 7 GB of RAM. In-process trials take about 0.5 s each:
that is the in-process router's open, which waits 500 ms after it starts
listening, before any measurement.

## Reproduce

```bash
# One cell: 720 fresh processes, 12 at a time (about 31 s); prints the # drive line.
target-1.10.1/release/lgr drive --jobs 12 --trials 720 --shape A --order subs-first --wait 0 --out /tmp/cell.txt
python3 -I summary.py /tmp/cell.txt

# The tables below, from the committed results.
python3 -I summary.py results-1.10.1.txt.xz results-main.txt.xz results-fix.txt.xz

# One trial, printed.
target-1.10.1/release/lgr trial --shape A --order tokens-first --wait 0

# The deterministic case: 20 of 20 first GETs are complete and empty on 1.10.1 and main.
target-1.10.1/release/lgr-minimal --rounds 20                    # history subscriber, same session
target-1.10.1/release/lgr-minimal --rounds 20 --no-history       # 0 of 20
target-1.10.1/release/lgr-minimal --rounds 20 --other-session    # 0 of 20
target-fix/release/lgr-minimal --rounds 20                       # 0 of 20 with fix.patch

# Traces of missing trials (tracing slows the process, which makes the race more likely).
RUST_LOG=zenoh::net::routing=debug,zenoh::api::session=trace \
  target-1.10.1/release/lgr drive --jobs 12 --trials 60 --shape B --order subs-first --wait 0 \
  --out traces/x/results.txt --trace-dir traces/x
traces/excerpt.sh traces/x/trial-*-<seq>.log

# The matrix, part by part (each part under ten minutes).
./run-matrix.sh 1.10.1 tf-ab   # then tf-cd sf shared burn delay big; and the same for main
./run-matrix.sh fix fix-check
```

## What was run

The run took place on 2026-10-24 (UTC), with a VM reboot in the middle. Every
part ran in the foreground, one at a time.

| Version | Parts | Trials |
|---|---|---|
| 1.10.1 | `tf-ab tf-cd sf shared burn big delay` | 61,920 |
| main | the same parts | 61,920 |
| 1.10.1+fix | `fix-check` | 6,480 |

- **main's `burn` part** was cut by the reboot inside its sixth cell (C
  subs-first samples burn6), after 648 of 720 trials. Those 648 lines stay in
  `results-main.txt.xz`. `burn-rest` added 72 trials to that cell and then ran
  the rest of the part.
- **`delay`** (the cells at 100 ms and the C/D cells under burn) ran after the
  reboot, for both versions.
- **The extra runs** are not in the tables: `scratch/minimal-*.txt`
  (`--services 1 --subs broad`, 120 trials per shape and version) and the
  traced runs in `traces/`. Neither is committed here, beyond the excerpts.

## Results

### Misses per cell (complete GETs that missed a held token / trials)

| router | load | order | wait | A 1.10.1 | A main | A 1.10.1+fix | B 1.10.1 | B main | B 1.10.1+fix | C 1.10.1 | C main | D 1.10.1 | D main |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| inproc | burn0 | tokens-first | 0 | **720/720** | **719/720** | 0/720 | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn0 | tokens-first | 10 | 0/720 | **7/720** | - | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn0 | tokens-first | 100 | 0/720 | **2/720** | - | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn0 | tokens-first | samples | **7/3600** | **18/3600** | - | - | - | - | 0/720 | 0/720 | - | - |
| inproc | burn0 | tokens-first | narrow | **1/720** | **7/720** | - | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn0 | subs-first | 0 | **134/720** | **107/720** | 0/720 | **128/720** | **118/720** | 0/720 | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn0 | subs-first | 100 | **3/720** | 0/720 | - | 0/720 | **1/720** | - | - | - | - | - |
| inproc | burn0 | subs-first | samples | **3/720** | **8/720** | - | **5/3600** | **24/3600** | - | 0/720 | 0/720 | - | - |
| inproc | burn0 | subs-first | narrow | **4/3600** | **7/3600** | - | 0/720 | **3/720** | - | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn6 | tokens-first | 100 | **16/720** | **16/720** | 0/720 | 0/720 | 0/720 | - | - | - | 0/720 | 0/720 |
| inproc | burn6 | tokens-first | samples | **16/720** | **10/720** | 0/720 | - | - | - | 0/720 | 0/720 | - | - |
| inproc | burn6 | subs-first | 0 | - | - | - | - | - | - | 0/720 | 0/720 | 0/720 | 0/720 |
| inproc | burn6 | subs-first | 100 | **5/720** | **4/720** | 0/720 | **3/720** | **2/720** | 0/720 | - | - | - | - |
| inproc | burn6 | subs-first | samples | **18/720** | **11/720** | 0/720 | **12/720** | **14/720** | 0/720 | 0/720 | 0/720 | - | - |
| inproc | burn6 | subs-first | narrow | - | - | - | - | - | - | - | - | 0/720 | 0/720 |
| shared | burn0 | tokens-first | 0 | **720/720** | **720/720** | - | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| shared | burn0 | tokens-first | 10 | 0/720 | **1/720** | - | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| shared | burn0 | tokens-first | 100 | 0/720 | 0/720 | - | 0/720 | 0/720 | - | 0/720 | 0/720 | 0/720 | 0/720 |
| shared | burn0 | tokens-first | samples | 0/720 | **1/720** | - | - | - | - | 0/720 | 0/720 | - | - |
| shared | burn0 | subs-first | 0 | **5/720** | **6/720** | - | **10/720** | **5/720** | - | 0/720 | 0/720 | 0/720 | 0/720 |
| shared | burn0 | subs-first | samples | **1/720** | **2/720** | - | **1/720** | 0/720 | - | 0/720 | 0/720 | - | - |
| shared | burn0 | subs-first | narrow | **1/720** | 0/720 | - | 0/720 | 0/720 | - | - | - | 0/720 | 0/720 |
| shared | burn6 | tokens-first | samples | **1/1440** | 0/1440 | - | - | - | - | - | - | - | - |
| shared | burn6 | subs-first | samples | **1/1440** | **1/1440** | - | 0/1440 | 0/1440 | - | - | - | - | - |
| all |  |  |  | 1656/23040 | 1647/23040 | 0/4320 | 159/16560 | 167/16560 | 0/2160 | 0/12240 | 0/12240 | 0/10080 | 0/10080 |

### The misses by family, and what recovers them

| zenoh | shape | family | load | misses | of which tokens-first wait 0 | GET empty | 2nd GET found all | union of both found all | narrow found all | fresh session found all |
|---|---|---|---|---|---|---|---|---|---|---|
| 1.10.1 | A | in-flight | burn0 | 1574 | 1440 | 1441 | 1574/1574 (100 %) | 1574/1574 (100 %) | 1572/1574 (100 %) | 1574/1574 (100 %) |
| 1.10.1 | A | lost-known | burn0 | 19 | 0 | 16 | 13/19 (68 %) | 13/19 (68 %) | 17/19 (89 %) | 19/19 (100 %) |
| 1.10.1 | A | lost-known | burn6 | 57 | 0 | 57 | 24/57 (42 %) | 24/57 (42 %) | 34/57 (60 %) | 57/57 (100 %) |
| 1.10.1 | A | ? | burn0 | 6 | 0 | 3 | 6/6 (100 %) | 6/6 (100 %) | 6/6 (100 %) | 6/6 (100 %) |
| 1.10.1 | B | in-flight | burn0 | 126 | 0 | 10 | 126/126 (100 %) | 126/126 (100 %) | 125/126 (99 %) | 126/126 (100 %) |
| 1.10.1 | B | lost-known | burn0 | 8 | 0 | 8 | 7/8 (88 %) | 7/8 (88 %) | 7/8 (88 %) | 8/8 (100 %) |
| 1.10.1 | B | lost-known | burn6 | 15 | 0 | 14 | 7/15 (47 %) | 7/15 (47 %) | 9/15 (60 %) | 15/15 (100 %) |
| 1.10.1 | B | ? | burn0 | 10 | 0 | 3 | 10/10 (100 %) | 10/10 (100 %) | 10/10 (100 %) | 10/10 (100 %) |
| main | A | in-flight | burn0 | 1541 | 1438 | 1437 | 1537/1541 (100 %) | 1537/1541 (100 %) | 1538/1541 (100 %) | 1541/1541 (100 %) |
| main | A | lost-known | burn0 | 58 | 0 | 56 | 31/58 (53 %) | 31/58 (53 %) | 41/58 (71 %) | 58/58 (100 %) |
| main | A | lost-known | burn6 | 42 | 0 | 41 | 19/42 (45 %) | 19/42 (45 %) | 32/42 (76 %) | 42/42 (100 %) |
| main | A | ? | burn0 | 6 | 0 | 2 | 6/6 (100 %) | 6/6 (100 %) | 6/6 (100 %) | 6/6 (100 %) |
| main | B | in-flight | burn0 | 115 | 0 | 19 | 114/115 (99 %) | 114/115 (99 %) | 114/115 (99 %) | 115/115 (100 %) |
| main | B | lost-known | burn0 | 31 | 0 | 31 | 19/31 (61 %) | 19/31 (61 %) | 23/31 (74 %) | 31/31 (100 %) |
| main | B | lost-known | burn6 | 16 | 0 | 16 | 8/16 (50 %) | 8/16 (50 %) | 12/16 (75 %) | 16/16 (100 %) |
| main | B | ? | burn0 | 5 | 0 | 1 | 5/5 (100 %) | 5/5 (100 %) | 5/5 (100 %) | 5/5 (100 %) |

**Family** says which of the two races in "The mechanism" caused a miss.
- **In-flight:** the GET found at least the tokens the subscribers already
  had, and missed tokens still on their way to them.
- **Lost-known:** the GET lost tokens the subscribers already had.
- **`?`:** the miss cannot be classified. On a shared router with
  `subs-first`, `seen_at_get` counts other trials' tokens too.

**Narrow** means the three instance tokens only.

### "Read twice": both broad GETs missed

| zenoh | router | load | order | wait | shape | trials | first GET missed | both GETs missed (union incomplete) |
|---|---|---|---|---|---|---|---|---|
| 1.10.1 | inproc | burn0 | subs-first | 0 | A | 720 | 134 | 0 |
| 1.10.1 | inproc | burn0 | subs-first | 0 | B | 720 | 128 | 0 |
| 1.10.1 | inproc | burn0 | subs-first | 100 | A | 720 | 3 | 1 |
| 1.10.1 | inproc | burn0 | subs-first | samples | A | 720 | 3 | 1 |
| 1.10.1 | inproc | burn0 | subs-first | samples | B | 3600 | 5 | 1 |
| 1.10.1 | inproc | burn0 | subs-first | narrow | A | 3600 | 4 | 1 |
| 1.10.1 | inproc | burn0 | tokens-first | 0 | A | 720 | 720 | 0 |
| 1.10.1 | inproc | burn0 | tokens-first | samples | A | 3600 | 7 | 3 |
| 1.10.1 | inproc | burn0 | tokens-first | narrow | A | 720 | 1 | 0 |
| 1.10.1 | inproc | burn6 | subs-first | 100 | A | 720 | 5 | 5 |
| 1.10.1 | inproc | burn6 | subs-first | 100 | B | 720 | 3 | 1 |
| 1.10.1 | inproc | burn6 | subs-first | samples | A | 720 | 18 | 10 |
| 1.10.1 | inproc | burn6 | subs-first | samples | B | 720 | 12 | 7 |
| 1.10.1 | inproc | burn6 | tokens-first | 100 | A | 720 | 16 | 8 |
| 1.10.1 | inproc | burn6 | tokens-first | samples | A | 720 | 16 | 9 |
| 1.10.1 | shared | burn0 | subs-first | 0 | A | 720 | 5 | 0 |
| 1.10.1 | shared | burn0 | subs-first | 0 | B | 720 | 10 | 0 |
| 1.10.1 | shared | burn0 | subs-first | samples | A | 720 | 1 | 0 |
| 1.10.1 | shared | burn0 | subs-first | samples | B | 720 | 1 | 0 |
| 1.10.1 | shared | burn0 | subs-first | narrow | A | 720 | 1 | 0 |
| 1.10.1 | shared | burn0 | tokens-first | 0 | A | 720 | 720 | 0 |
| 1.10.1 | shared | burn6 | subs-first | samples | A | 1440 | 1 | 0 |
| 1.10.1 | shared | burn6 | tokens-first | samples | A | 1440 | 1 | 1 |
| main | inproc | burn0 | subs-first | 0 | A | 720 | 107 | 4 |
| main | inproc | burn0 | subs-first | 0 | B | 720 | 118 | 3 |
| main | inproc | burn0 | subs-first | 100 | B | 720 | 1 | 0 |
| main | inproc | burn0 | subs-first | samples | A | 720 | 8 | 4 |
| main | inproc | burn0 | subs-first | samples | B | 3600 | 24 | 8 |
| main | inproc | burn0 | subs-first | narrow | A | 3600 | 7 | 1 |
| main | inproc | burn0 | subs-first | narrow | B | 720 | 3 | 2 |
| main | inproc | burn0 | tokens-first | 0 | A | 720 | 719 | 3 |
| main | inproc | burn0 | tokens-first | 10 | A | 720 | 7 | 5 |
| main | inproc | burn0 | tokens-first | 100 | A | 720 | 2 | 1 |
| main | inproc | burn0 | tokens-first | samples | A | 3600 | 18 | 9 |
| main | inproc | burn0 | tokens-first | narrow | A | 720 | 7 | 4 |
| main | inproc | burn6 | subs-first | 100 | A | 720 | 4 | 3 |
| main | inproc | burn6 | subs-first | 100 | B | 720 | 2 | 1 |
| main | inproc | burn6 | subs-first | samples | A | 720 | 11 | 5 |
| main | inproc | burn6 | subs-first | samples | B | 720 | 14 | 7 |
| main | inproc | burn6 | tokens-first | 100 | A | 720 | 16 | 10 |
| main | inproc | burn6 | tokens-first | samples | A | 720 | 10 | 5 |
| main | shared | burn0 | subs-first | 0 | A | 720 | 6 | 0 |
| main | shared | burn0 | subs-first | 0 | B | 720 | 5 | 0 |
| main | shared | burn0 | subs-first | samples | A | 720 | 2 | 0 |
| main | shared | burn0 | tokens-first | 0 | A | 720 | 720 | 0 |
| main | shared | burn0 | tokens-first | 10 | A | 720 | 1 | 0 |
| main | shared | burn0 | tokens-first | samples | A | 720 | 1 | 0 |
| main | shared | burn6 | subs-first | samples | A | 1440 | 1 | 0 |

### Delay dependence

- **At wait 0, the in-flight race dominates.**
  - Subs-first: A misses 134/720 (1.10.1) and 107/720 (main); B misses
    128/720 and 118/720.
  - A tokens-first: 720/720 and 719/720.
- **Every classified miss at 10 ms, 100 ms, `samples` or `narrow` is
  lost-known.** Waiting closes the in-flight window but not the other one.
  - Unloaded, lost-known misses run at about 0.1 to 1 % (A tokens-first
    samples: 7/3600 and 18/3600; B subs-first samples: 5/3600 and 24/3600;
    A subs-first samples: 3/720 and 8/720).
  - With six burner threads, they run at 1.4 to 2.5 % right after `samples`
    and at 100 ms tokens-first (A tokens-first 100 ms: 16/720 on both;
    A subs-first samples: 18/720 and 11/720; B subs-first samples: 12/720
    and 14/720), and at 0.3 to 0.7 % at 100 ms subs-first.
  - A longer wait does not make it go away. Under burn, A tokens-first misses
    as often at 100 ms as at `samples`.
- **B tokens-first never misses.** A non-history subscriber is not sent the
  tokens that existed before it subscribed.

### Recovery

- **An in-flight miss is recovered by the second broad GET** (3,351 of 3,356,
  both versions). Its tokens have arrived by then.
- **A lost-known miss usually loses every token.** 239 of 246 came back with
  no reply at all, complete.
- **The second GET recovers only 42 to 88 % of lost-known misses**, and less
  under load. The narrow GETs recover 60 to 89 %.
- **A fresh session always finds every token** (3,629 of 3,629).
- **"Read twice" leaves a residual.** A trial is still wrong when the union of
  both broad GETs is incomplete:
  - unloaded: 3/3600 (1.10.1) and 9/3600 (main), A tokens-first samples;
    1/3600 and 8/3600, B subs-first samples;
  - with burn: 8 to 10 of 720, A tokens-first 100 ms; 10/720 and 5/720,
    A subs-first samples.

## The mechanism

Topology: a client session (the reader) connects to a router. Inside the
reader's runtime, its session face sits in the south region, served by the
`broker` hat, and its link to the router in the north region, served by the
`client` hat. The paths are in zenoh 1.10.1's crate and are identical at main
766b455. The line numbers are 1.10.1's; at main, `face.rs` moved by 21 lines
and `hat/peer/interests.rs` by one.

**1. A liveliness GET is a `Current` token interest.**
- The session sends the interest and expects every match as a `DeclareToken`
  carrying its interest id, then a `DeclareFinal`. It does not answer from
  what it already knows (`api/session.rs:2934`, "the gateway resends current
  tokens on every query").
- A `DeclareToken` with the query's id becomes a reply (`api/session.rs:3222`).
  `DeclareFinal` ends the query (`api/session.rs:3330`).

**2. The reader's routing answers that interest in two parts**
(`dispatcher/interests.rs:271-332`, inside `Face::send_interest`,
`dispatcher/face.rs:545-552`):
- **The forward:** `hats[North].route_interest` forwards the interest to the
  router at once, under `ctrl_lock`, and records a pending current interest
  (`hat/client/interests.rs:128`).
- **The local answer:** `hats[South].send_current_tokens` builds a local answer
  from the tokens the client hat already knows (`other_token_matches`, line
  310). These are the tokens the router declared to this runtime because a
  liveliness subscriber asked for them, plus the session's own tokens.
- **When the local answer arrives:** these declarations are only collected
  under `ctrl_lock`. They reach the session after the lock is released, in
  `send_interest`'s replay loop on the calling thread (`face.rs:550-552`).

**3. The router's answer comes back through `propagate_current_token`.**
- The router answers with every matching token tagged with the forwarded id,
  then `DeclareFinal`.
- In the reader's runtime, each tagged token goes through
  `dispatcher/token.rs:75-88` → `hat/broker/interests.rs:376`
  `propagate_current_token`, which starts with:

  ```rust
  if self.face_hat(&dst).local_tokens.contains_key(&res) {
      tracing::debug!("Already propagated");
      return;
  }
  ```
- `local_tokens` holds the tokens already declared to the session face for a
  future interest, that is, for a liveliness subscriber, history or not.
- So for every token the session's subscribers already hold, the router's
  answer to the GET is dropped. The GET then depends entirely on the local
  answer from step 2.
- The router's `DeclareFinal` is still forwarded (`dispatcher/interests.rs`,
  `declare_final`) and ends the query.

**Two races follow.** In both, the GET is complete, so nothing tells the caller.

### Family 1: in flight (the GET found exactly what the subscribers had seen)

1. A token the router has already declared to the reader for its subscriber is
   still in flight when the GET is issued. It may sit in the router's
   transmission batch (`time_limit` 1 ms), or in the reader's receive path.
2. The local answer does not contain it.
3. That declaration travels ahead of the router's answer to the GET on the same
   link, so it reaches the session face first and enters `local_tokens`.
4. The router's answer for it is then "Already propagated" and dropped.

The GET returns precisely the tokens known when it was issued:
`first=c:N/10` with `seen_at_get=N` in 3,339 of the 3,356 in-flight misses.

- **Trace, subs-first:** `traces/excerpt-in-flight-B-subs-first-wait0.txt`. The
  B subscriber had 5 tokens, and the GET returned those 5. The other 5 plain
  declarations arrived 0.1 to 0.4 ms after the GET's local answer, and then
  all 10 of the router's answers were "Already propagated".
- **The A tokens-first wait 0 cell is this race at its extreme**, and
  deterministic. It is not an artifact of the trial.
  1. Declaring a history subscriber sends a `CurrentFuture` interest.
  2. The GET's `Current` interest follows it on the same link, microseconds
     later.
  3. The router answers both in order. The subscribers' current tokens always
     reach the reader before the GET's answer, and at the GET's local answer
     the reader knew none of them.
  4. So every token of the router's answer is dropped, and the GET completes
     empty: 720/720 (1.10.1) and 719/720 (main), in process and with a shared
     router alike.
  5. `lgr-minimal` shows it with one token and one subscriber, 20 of 20 on both
     versions.

  Trace: `traces/excerpt-in-flight-A-tokens-first-wait0.txt`.
- **Why B tokens-first never misses:** a non-history subscriber sends a
  `Future` interest. The router sends it no current tokens, so pre-existing
  tokens never enter `local_tokens`, and the router's answer to the GET is
  delivered.

### Family 2: lost-known (the router's DeclareFinal overtakes the local answer)

Every token is known, so the local answer holds all of them, and the router's
answer is all dropped (step 3). The local answer still has to reach the session
before the router's `DeclareFinal`. It travels on the calling thread after
`ctrl_lock` is released (step 2). The `DeclareFinal` travels on a receive
thread. If the calling thread is descheduled for longer than one router round
trip, the receive thread processes the router's answer and delivers the
`DeclareFinal` first. The session closes the query, and the local answer
arrives after it.

- **The result:** a complete GET with none of the tokens, or some of them when
  the final lands in the middle of the replay. The misses keep coming at
  every wait, and CPU load raises the rate.
- **Trace:** `traces/excerpt-lost-known-A-tokens-first-samples.txt`. The
  router's 10 answers are dropped, `recv DeclareFinal Some(9)` is delivered
  on `rx-0`, and the 10 local-answer `recv DeclareToken 0` follow on `main`.
- **Trace with a partial result:**
  `traces/excerpt-lost-known-B-subs-first-samples-burn6.txt`. 3 of the local
  answer's tokens were delivered before the final and 7 after.

### The intervention

`fix.patch` makes `propagate_current_token` skip a token that is already in
`local_tokens` only for future-mode interests, which are what `local_tokens`
deduplicates. A `Current` interest's tokens are always sent, tagged with its
id. This closes both races. The router's answer now reaches the GET in order,
ahead of the router's `DeclareFinal` on the same receive thread, whatever
`local_tokens` holds and whenever the local answer lands.

**Measured:** 0 misses in 6,480 trials (`results-fix.txt.xz`), in the nine cells
where stock 1.10.1 misses 1,052, and 0 of 120 in the minimal cell. A GET can
now receive a token twice, from the local answer and from the router. A GET from
the token's own session already does that on stock 1.10.1, so `lgr` and
`lgr-minimal` count distinct keys.

### Secondary effects (by code reading, partly measured)

- **A stray Put.** A local-answer declaration that arrives after its query's
  `DeclareFinal` is not dropped. It falls through to the subscriber path of the
  `DeclareToken` handler (`api/session.rs:3247`) under token id 0, which every
  `Current` reply uses.
  - The first such key is recorded in the session's `remote_tokens` under id 0,
    and history subscribers get one extra Put. That is the `puts_end=11` in
    shape A, and it is still there with `fix.patch` (134 of 4,320 A trials).
    The rest are ignored, because id 0 is taken.
  - When the token is undeclared, its real id is removed and id 0 stays.
  - **Suspected, not measured:** a history subscriber declared later on that
    session is handed the stale key from `remote_tokens`
    (`api/session.rs:2065`) as a live token, and nothing ever deletes it.
- **Own tokens.** The broker hat also declares a session's own tokens back to
  it when it has a matching subscriber (`hat/broker/token.rs:214`).
  **Suspected:** a session that holds its own tokens and a matching liveliness
  subscriber is exposed to family 2 for its own tokens too.
- **Peer mode.** The peer hat has the same guard
  (`hat/peer/interests.rs:614`). Not measured: every session here is a
  client.
- **zenoh#2678 would widen family 2.** That open PR moves `send_interest`'s
  replay loop onto a spawned blocking task. The local answer would then always
  race the router's `DeclareFinal`. With `fix.patch` the GET would no longer
  depend on it.

## Caveats

- **Trace excerpts:** the raw `traces/tf-samples/` logs (not committed) came
  from an earlier revision of the program, which lacked `puts_end`. The
  excerpts kept here come from this one. Tracing slows a process, and in the traced
  B wait-0 run some misses were lost-known instead of in-flight.
- **In-process trials:** the router, holder and reader share one process and
  one zenoh runtime, as zenkey's test suite does. Shared-router trials have a
  router process and a holder process. Both miss: in-flight at wait 0,
  lost-known otherwise.
- **The other agent:** during part of the run, another agent ran on the host
  without building anything. The `# drive` lines record the load average at
  each cell's start and end.
