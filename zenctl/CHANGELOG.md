# zenctl changelog

`zenctl` ships as Forgejo release binaries, not on crates.io (0.1.x stays
there un-yanked). What that buys is the ability to fix a command tree instead
of carrying it — and what it costs is this file, which has to be complete
enough that a script written against the old spellings can be moved in one
sitting.

## 0.5.0 (2026-08-25) — the command tree, the flags, and the exit contract (#307)

**This is a breaking change with no alias layer and no deprecation shims.**
The old spellings are gone, not warned about. That is deliberate: an alias
layer is a second tree to keep honest, and the whole point of the change was
to stop having two of anything.

### Why

Three vocabularies had drifted, each in a way you could only see by reading
the whole surface at once:

* **Depth meant nothing.** `topic echo` subscribed to live traffic while
  `topic list` read a registry document; `schema <producer>` was a noun that
  was also a verb, and `zenctl schema` with no argument exited 1 where every
  other missing argument exits 2. Meanwhile `expect`, `cutover` and `probe`
  sat at the root beside `get` and `scout`, sharing an exit contract with each
  other and nothing else.
* **One concept had four spellings.** A passive observation window was
  `--window` on `cutover` and `hz`, `--within` on `expect`, `--duration` on
  `record`, and `--listen-for` on `doctor`, `why` and `registry retired` — and
  three of those were `u64` while three were `f64`.
* **Exit 2 meant two things.** The corpus pinned both doctrines side by side:
  `session-config-error.trycmd` argued *your input is a 1*,
  `key-algebra.trycmd` argued *an invalid expression is a 2*. No script could
  hold both.

The contract is now written once, in prose, in `zenctl/src/exit.rs`, and every
verb cites it.

### The tree

| was | is |
| --- | --- |
| `topic echo` | `echo` |
| `topic pub` | `pub` |
| `topic retire` | `retire` |
| `topic hz` | `rate` |
| `topic bw` | `rate --bytes` |
| `schema <PRODUCER>` | `schema show <PRODUCER>` |
| `schema check` | `check schema` |
| `expect` | `check expect` |
| `cutover` | `check cutover` |
| `probe` | `check probe` |
| `registry retired` | `check retired` |
| `blob probe` | `blob locate` |

`topic` keeps `list` and `info` — the two things that read a registry
document. `get`, `field`, `record`, `replay`, `serve`, `gen`, `scout`,
`doctor`, `why` and `watchdog` are unmoved. `bench rpc`, `key`, `node`,
`base`, `service`, `interface`, `storage`, `admin`, `registry`, `context`,
`cache` and `completions` are unmoved.

`rate` is a merge, not a rename: `--bytes` selects the byte view and
`--per-key`/`--loss`/`--latency` work as they did on `hz`.

### The flags

One spelling per concept; every duration is `f64` seconds.

| was | is | on |
| --- | --- | --- |
| `--window <SECS>` | `--for <SECS>` | `check cutover`, `rate` |
| `--within <SECS>` | `--for <SECS>` | `check expect` |
| `--duration <SECS>` | `--for <SECS>` | `record` |
| `--listen-for <SECS>` | `--for <SECS>` | `doctor`, `why`, `check retired` |
| `--budget [<SECS>]` | `--budget` + `--for <SECS>` | `topic list` |
| `--watch [<SECS>]` | `--watch` + `--every <SECS>` | `topic list`, `base list`, `storage list` |
| `--tick <SECS>` | `--every <SECS>` | `watchdog` |
| `--ticks <N>` | `--count <N>` | `watchdog` |
| `--runs <N>` | `--count <N>` | `doctor` |
| `--watch` | `--transitions` | `doctor` |
| `--interval <SECS>` | `--every <SECS>` | `pub` |
| `--repeat <N>` (default 0) | `--times <N>` (default 1) | `pub` |
| `--count <N>` | `--at-least <N>` | `check expect` |
| `--count <N>` | `--calls <N>` | `bench rpc` |
| `--from <ORIGIN>` | `--origin <ORIGIN>` | `blob fetch` |
| `--i-know` (width guard) | `--wide` | `gen` |

The rules those follow, so the next flag has somewhere to go:

* **`--for`** is every passive observation window.
* **`--timeout`** is reply-wait and nothing else.
* **`--duration`** bounds *generated output*, so only `gen` has it.
* **`--watch`** is a bare bool; **`--every`** is the one period.
* **`--count`** is a stop bound. An assertion is `--at-least`, an experiment
  size is `--calls`, a repeat count is `--times`.
* **`--from`** names an input source, never an origin.
* **`--i-know`** is one guard per verb. `gen` had two on one flag —
  acknowledging a wide run also armed the fault injector — and now spells the
  second one `--wide`.

`pub --repeat 0` and `--repeat 1` used to be the same number. `--times`
counts: 0 publishes nothing, 1 publishes once.

New: **`echo`, `rate`, `record`, `field`, `check expect` and `why` all take
the same selector shape** — a positional selector *or* `--origin`/`--class`/
`--producer` composed server-side, never both. `check expect`, `field` and
`why` used to force a hand-written selector.

New: **`field --fail-on <SEVERITY>`**, the opt-in `doctor` already had. The
asymmetry was the bug; the default is still exit 0.

### The exit contract

| code | meaning |
| --- | --- |
| **0** | asked, and the answer is clean — values, assertion met, pass, valid, healthy, act completed |
| **1** | asked, and the answer is a **finding** — not met, the old family still speaks, an invalid payload, an error reply, rows refused or malformed, `--fail-on` tripped, an act that failed |
| **2** | **no verdict** — the question could not be asked, or could not be proven |

What moved:

* **`why` flipped polarity.** It exited 0 on "a cause was found" and 1 on "the
  fleet looks healthy", which pages you for a healthy fleet. It now exits 1 on
  a finding and 0 when nothing is wrong. `zenkey_fleet::WhyVerdict::to_judgement`
  carries the inversion and `judgement_exit_code` projects it, so zenctl and
  zengui cannot drift on what a `why` verdict means.
* **Refused input is 2, everywhere.** An unresolvable `--context`, a `--qos`
  outside RFC 04 §3's five, a class outside RFC 04 §1's three, an unknown
  `--fault` kind, a `$*` selector (RFC 03 §2), a `--zenoh-config` this tool
  refuses, a `--registry` dir that will not read, a window given as zero
  seconds, an inert flag for the target you named, `gen`'s two guards: all
  exited **1** before, and all exit **2** now. Clap already exits 2 for every
  mis-shaped command line and cannot be argued with, so everything else joined
  it rather than competing with it.
* **`bench rpc` exits 1 when any measured reply was an error envelope.** It
  used to exit 0 with a latency distribution over failures.
* **`registry diff` with no `--registry`, and `registry export` with nothing
  to export, exit 2.** Neither is a finding: one question cannot be put, the
  other found silence.

Unchanged: acts (`pub`, `retire`, `replay`, `gen`, `blob fetch`) keep 1 for
their own failures — "unprovable" has no meaning for an act — and the verdict
verbs keep the pre-run guard that turns *every* setup failure into 2 rather
than a claimed verdict.

### Also

* `doctor`'s summary said what `registry diff` says. It now says what doctor
  does: check the fleet against the contracts it claims.
* `zenctl/tests/cmd/` is regrouped to match: `help-<noun>` per family,
  `help-wire` for the root verbs, `help-check` for the assertions.
