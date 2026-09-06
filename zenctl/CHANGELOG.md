# zenctl changelog

`zenctl` ships as Forgejo release binaries, not on crates.io (0.1.x stays
there un-yanked). What that buys is the ability to fix a command tree instead
of carrying it — and what it costs is this file, which has to be complete
enough that a script written against the old spellings can be moved in one
sitting.

## Unreleased

**`export` — a metrics surface that exports its own blind spots** (#228).
A root wire verb: `zenctl export --bind 127.0.0.1:9184` serves
`/metrics` as Prometheus text (RFC 13 §3 *Exporter obligations*, v1.34),
two families deliberately apart. **Observer and contract metrics**:
`zenkey_observer_dropped_total`, `zenkey_observer_evicted_total{population=
keys|retained_bytes|retained_age|unwatched}` (four lines, never summed),
`zenkey_observer_coalesced_total` (between two scrapes only the newest
value per series is exposed; the rest are counted), `zenkey_observer_
unstamped_total`, `zenkey_qos_judged_total` and `zenkey_qos_mismatch_total
{producer,subject}`, `zenkey_payload_verdict_total{verdict=valid|invalid|
not_validated}` (three lines, never a ratio; without `--validate` everything
is `not_validated`), `zenkey_doctor_finding{check_id,severity,subject}` and
`zenkey_doctor_info{state=not_asked|ran}`, `zenkey_scope_info{selector,
excluded}` naming the planes a wildcard cannot reach, `zenkey_registry_info
{state=loaded|not_loaded}`, `zenkey_series_suppressed_total{reason}`,
`zenkey_unregistered_keys`. **Key metrics from the contract**:
`zenkey_subject_<producer>_<literal chunks>[_<unit>][_total]` — name and
unit from the registry's `unit` and `kind`, never sniffed; every `{var}` a
label by its declared name; the declared `cardinality` bounds the
population and the refusals are counted. A series that stopped keeps its
labels and `zenkey_series_state{state=evicted|origin_down|retired}` and
loses its value line — absence is named, never a flat line; `quiet` is
judged only for `state` subjects against `ttl_s`; every series carries
`zenkey_key_last_seen_timestamp_seconds` (constant between scrapes, so an
idle scrape is byte-identical) and `zenkey_series_drop_exposed_total`.
Flags: the `SelectorArgs` (default `<base>/v1/*/**`), `--bind`
(non-loopback needs `--i-know`; `--listen` stays the transport's), `--validate` (2 decodes per key per
second), `--doctor-every SECS` (off by default — it costs the control
plane), `--max-series N` (10 000), `--once` (observe `--for` seconds, fold
once, print the `export` report through `--format`; `--prom` prints the
exposition instead and is refused with `--format`). Refused up front: OTLP,
histograms and summaries, push gateways and remote write. The HTTP server
is hand-rolled (`cmd/export/http.rs`): `hyper` is not in zenctl's graph and
one route does not earn a framework; the switch point is a second endpoint.

Two new verbs under the `registry` noun and no moved spelling (#224).

**`registry consumers <target>` — who declares a reader of a subject.** A
join over the admin space: every declared subscriber and querier with its
verbatim keyexpr, related to the target by key algebra (`exact` <
`narrower` < `wider` < `intersects` < `total`), one row per declaring
session on the admin `sources` (reported-only when they name none), joined
to the origin its alive token attaches (#131) and the topology's `whatami`.
`<target>` is `<producer>/<subject-path>` resolved through the slices to
the family's wire selector, or a raw key/selector (anything with `*`/`@`,
or starting at the base or `v1/`). The honesty is the product: an admin
space that does not answer is `admin: not_available` with no rows — *not
asked*, never an empty set (RFC 13 §3 O4); a declaration is not proof of
use; a `**` declaration is `total` and flagged `total_wildcard` rather than
read as a consumer of this subject; the tool's own session appears and is
named `(this zenctl session)`; the answering admin spaces are counted. It
is not matching status (RFC 12 §9), and the render corpus greps the
family for "matching", "listening", "unmatched" and "no consumers".

**`registry impact <producer>/<path>` — the blast radius of changing one
declared subject.** The consumers above, plus the RFC 04 §2 storage
coverage row (made only when an admin space answered — an empty storage
list would read "uncovered"), the distinct sessions declaring a publisher
or queryable on the family, and the `[[deprecated]]` ledger entry. A path
that survives only in the ledger still resolves, class wildcarded.

Families `registry-consumers` and `registry-impact`; rows tagged
`consumer` and `coverage`; the admin discriminator rides flat in the
envelope (`admin`, `answered`, `nodes`).
**`timeline` — the fleet timeline, and deliberately no edges** (#216).
A new wire verb at the root. `zenctl timeline <SEL>… --for <SECS>
[--order arrival|hlc]` watches the selectors for the window and emits one
merged ordering of everything seen, partitioned into lanes per
origin/producer, with the clock stated per report and the stamper(s) per
lane. Every ndjson row carries `order_by`, so a line cut out of the
stream still says which axis its `pos` is on. Under `--order hlc` the
envelope carries the claim the axis can make (RFC 09 §5.1 O7): one
stamper is that node's happened-before; several are skewed wall clocks;
an empty axis claims nothing. Unstamped samples get their own lane on
the arrival axis and **cannot be placed on the HLC axis at all** — the
engine's `Placed<HlcAxis>` refuses them, and the report counts the
exclusion. Drops render as breaks at their arrival position and as a
total with no position on the HLC axis. The per-publisher
sequence-number lane is reported *unavailable* on this zenoh (no
`SourceInfo` reaches a subscriber), never empty. `--from <FILE>` reads a
`.zrec` through the same projection under the capture's stated base, and
the engine's identity test is what makes "the same window from the
file" a claim. No line is ever drawn between lanes: a merged ordering
shows when things were seen on which clock, never that one caused
another.

## 0.6.0 (2026-09-06) — the generators, and three rows that name a host

Two new verbs and no moved spelling: a script written against 0.5.1 runs
unchanged. Besides the two generators below: `interface show --schema`
names both hosts of a schema disagreement instead of recomputing one over
producer-keyed rows (#410); `call` and `check probe` print *stopped early*
under a partial page and a caveat when the cursor is null (RFC 05 §3.2,
#424); `topic info` gains a `kind` row (RFC 08 §2 v1.32, #422); `doctor`
carries `kind-mismatch` and, under `--deep`, `budget-exceeded` (#391);
`registry export --as toml` keeps a producer's `[budget]`.

**`acl gen` — RFC 09 §3's grant matrix, generated** (#392). A new `acl`
noun with one verb. `zenctl acl gen --enrollment <file.toml>` reads a small
TOML binding certificate CNs to roles (`host`, `catalog`, `console`,
`desired-author`, `watch`) and origins — given, or *computed* from a
machine-id with the RFC 06 §1 derivation, and refused when the two disagree
— and plans the router's `access_control` block: one rule per plane per
host because `**` never crosses `@rpc`/`@media`/`@blob` in inclusion
(fact 1), the catalog spelled on its own because `*` never covers it
(fact 2), rules *and* subjects *and* policies (fact 3), every consumer's
declarations allowed by name (fact 4), and — the fifth fact, from the
reference deployment — a shared egress-only `interest-prop` rule on every
publishing policy, without which a peer-mode publisher publishes to
nobody. With `--registry`, the planes narrow to what host producers declare
and `no-remote-actions` denies exactly the declared write procedures;
without one, the plan says so. `--json5` writes zenohd's block (field for
field zenoh 1.10, a comment per rule naming its row and its fact), a foreign
schema that conflicts with `--format` the way `--dot` does. A refused
principal exits 1.

`--check --against <router.json5>` diffs the plan against the block a
router's config file carries, read through zenoh's own loader — the running
block is not observable, because zenoh 1.10's admin space serves no GET
under `config/**` — and exits 0/1/2 through the one judgement projection;
the interest-propagation probe is *not asked*, and the report says why.
`--explain <principal> <key> <message>` answers per direction with the rules
that decided, inclusion by zenoh-keyexpr, exit 0. Zid-bound subjects need
`--allow-zid-subjects`, because zenoh's own config says a ZID is not
authenticated.
**`storage gen` plans the router's storages from the registry** (#393) — a
new verb under the `storage` noun, no flag or spelling elsewhere moved.
RFC 09 §2 specifies class-driven storages whose `garbage_collection.lifespan`
must be ≥ the longest `ttl_s` in the registry, and the registry knows that
number; nobody computed it. `zenctl storage gen --deployment <file.toml>`
takes a small TOML — the base, the volumes with their plugin (and, for
`redb`, the per-volume history mode of §2.1 v1.28), and per storage a class
(`state | telemetry | events | catalog | catalog-pdns`, or a base-relative
`selector`) and a volume — and derives the rest: the selector (`@catalog`
explicit, because `*` never matches it), the `strip_prefix` as the literal
leftmost run, and the lifespan as ceil(max covered `ttl_s` × `gc_margin`),
with the computation shown.

It refuses what the router would refuse — replication on an all-mode volume
(§2.2), a volume nobody declared, a class the registry declares nothing
under — and emits the rest of the plan around the refusal, naming it. It
warns, citing the clause, where a caveat applies: overlapping selectors
(§2), `complete = true` off the replicated latest storage (§2.2, emitted as
`false`), retention that is the database's and not zenoh's (§2.3, influx),
`redb`'s mandatory retention in all mode (§2.1), a seed on a volatile volume
(§2.1). Without a registry every lifespan is §2.3's default and says so
(`slices_optional`, #210) — not asked is not empty.

Four ways out. The plan report in the usual three renderings (families
`storage-plan`, rows `volume` / `storage` / `refusal`); `--json5`, the zenohd
`plugins.storage_manager` block with every derivation and warning as a
comment beside the storage it concerns — a foreign schema, so it is a flag
of its own and conflicts with a typed `--format`, like `--dot` and `--as`
(#243); `--check`, a verdict verb comparing the plan with the storages the
admin space reports (family `storage-check`: missing, extra, a differing
`key_expr` / `strip_prefix` / `volume`, a `gc.lifespan` below the computed
minimum; exit 0 as planned, 1 a difference, 2 no verdict — an empty admin
sweep included, since a peer-only mesh is not a router running the plan);
and `--explain <key>`, which planned storage takes a key and why (family
`storage-explain`). `gen` alone is an act: 0, or 2 when every storage was
refused.

Deliberately **not** in the RFC yet: 09 §2's notes will say that this verb
emits the block with `lifespan` derived, and that sentence needs a changelog
entry and a header bump of its own. The verb's `--help` carries it meanwhile.

## 0.5.1 (2026-08-29) — what the fleet does not agree about

No command moved and no flag changed. Three behaviour changes, all of them
this tool saying something it used to leave out.

**`registry diff` no longer prints `agree` about a comparison it made
against one of several answers** (#399). The diff is computed from one slice
per producer, so against a fleet mid-rollout it was computed from one
arbitrary host's — and the producer that happened to match your checkout
read as clean. A split producer now marks `✗` and details both hosts and
both versions, the envelope carries `self_disagreeing`, and `--format json`
gains a `collapsed` key.

The key is **absent**, not `[]`, when the served side did not come off the
bus: a diff computed from `--registry` files never asked how many origins
serve each producer, and a note says so. Scripts keying on `collapsed`
should treat absence as "not asked", never as "the fleet agrees".

**Every registry-aware verb says when its answer came from a pick** (#399).
A new stderr note, beside the existing bus-versus-checkout one and worded to
be unmistakable for it: that one says the fleet disagrees with your
checkout, this one says the fleet does not agree with itself. It appears on
any verb that loads slices from the bus.

**`watchdog` reports a write failure where it happens** (#397). It used to
keep the first `io::Error` and answer for it after the run, because the
engine's emit callback could not fail — so the run went on ticking against a
consumer that had gone, emitting into a local variable. A closed pipe still
exits 0 (`| head -1` is not a failure of the checks) and any other write
error still exits non-zero, but both now **stop the run** at the failed
write instead of at the tick bound, which is what Ctrl-C already did and
what `doctor --transitions` has always done. The summary line is not printed
in that case: the run did not finish, and a count of the ticks it happened
to reach is not the summary of anything.

**`doctor`'s `schema-drift` finding names the host** (#398), as
`producer@origin (hash)`. And it fires in a case it previously could not see
at all: one producer served by two hosts at two different schema hashes —
a half-rolled-out sensor, which is the likeliest schema disagreement there
is. The `check` id and the finding's `subject` are unchanged; the hosts ride
in `evidence`.

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
