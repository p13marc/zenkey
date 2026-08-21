# zenctl

A bus explorer for the keyspace-v2 convention — `busctl` / `d-feet` / `ros2` for
any conformant Zenoh fleet.

RFC 08 §6 specifies this tool into existence. Every producer MUST serve
`@rpc/<producer>/introspect`, returning the registry slice it was *compiled
against*; the point of that requirement is that "generic explorer tooling — the
`busctl`/`d-feet` equivalent — **needs no compiled-in registry**". `zenctl` is
that tooling: nothing application-specific is compiled in.

```bash
zenctl node list --base acme -c tcp/127.0.0.1:7447
```

`--base` (or `ZENCTL_BASE`) names the deployment base — the first chunk(s) of
every key on the wire. Applications set it as their session namespace and never
spell it; `zenctl` runs un-namespaced on purpose (RFC 09 §5), so it has to be
told about a *named* base. Left unset, it defaults to the **empty base** — the
base-less bus-root deployment whose keys start at `v1/`, the RFC v1.6 default —
so against a default-configured fleet `zenctl` works with no `--base` at all.
Don't know the base? `zenctl base list` discovers the bases actually in use.

## Two registry sources, kept visibly apart

| | Answers from | Works when the fleet is down | Tells you |
|---|---|---|---|
| **`--registry <dir>`** | local `registry/*.toml` files | yes | what *should* exist (declared) |
| **the bus** (default) | each producer's served introspect slice | no | what *does* exist (served) |

The gap between those two is where drift lives, and `doctor` is the command
that reports it.

```bash
zenctl topic list --base acme [--producer sysinfo] [--class telemetry] [--type TelemetryPoint]
zenctl topic list --base acme --deprecated   # + each slice's [[deprecated]] ledger rows
zenctl topic info --base acme acme/v1/h-3fa9c2d41b7e/state/sysinfo/health
zenctl service list --base acme [--producer sysinfo]
zenctl interface list --base acme
zenctl interface show --base acme TelemetryPoint
# any of the above, offline:  --registry path/to/registry
```

`topic info` runs the registry's **parse** direction (RFC 08 §1) — the thing
that replaced positional `split('/')` re-parsing. Variables come back *named*:

```
$ zenctl topic info --base acme acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/root/usage_percent
key       acme/v1/h-3fa9c2d41b7e/telemetry/sysinfo/disk/root/usage_percent
origin    h-3fa9c2d41b7e
producer  sysinfo
class     telemetry
subject   disk/{mount}/usage_percent
variables
  mount = root
payload   TelemetryPoint
  (`zenctl schema sysinfo --type TelemetryPoint` for the served shape)
qos       sampled
cardinality  ~512 keys expected
```

**Declared is not observed.** A pattern with a trailing rest-variable
(`{device}/{path...}`) fixes a *shape*, not its members — proxy producers
register that way by design, because their metric tree belongs to the polled
device. `topic list` flags those `[open-ended]`; `topic echo` is what
enumerates them.

## On-bus commands

```bash
zenctl base list -c tcp/127.0.0.1:7447  # discover deployment bases (needs no --base)
zenctl node list --base acme            # the liveliness roster (--verbose joins introspect)
zenctl node list --base acme --watch    # …re-rendered per liveliness event (no polling)
zenctl topic echo --base acme           # subscribe + decode (defaults to <base>/v1/**)
zenctl topic list --base acme --watch 5 # topic/storage/base list poll+diff; +/- marks
zenctl topic hz --base acme             # per-key sample rates; topic bw for bytes
zenctl service call --base acme '*' sysinfo processes --param sort=cpu
zenctl service call --base acme h-3fa9 netring capture/trigger --body @trigger.json
zenctl get 'acme/v1/*/state/**'         # fan-in GET on any selector, replies attributed
zenctl get '@/**'                       # …including the zenoh admin space (was: admin get)
zenctl topic pub k '{"v":1}' --attachment meta   # attachments ship and render (#117)
zenctl topic retire acme/v1/h-3fa9…/state/sysinfo/health  # RFC 04 §1.2 tombstone, class-guarded
zenctl scout                            # raw Hellos: zid/whatami/locators (multicast ON here)
zenctl serve 'demo/mock/**' '{"ok":1}'  # mock queryable; logs every ask (who queries this key?)
zenctl key intersects 'v1/**' 'v1/h-1/@rpc/p/x'  # keyexpr algebra, no session; cites D2/D4 on a convention-shaped no
zenctl topic echo --format ndjson > f   # …and back: topic pub --from ndjson < f (one row shape, both directions)
zenctl record --base acme -o bus.zrec --duration 10  # capture: same row shape + header + pacing + in-file drop ledger
zenctl replay bus.zrec --dry-run        # ALWAYS preview first — replay is publishing, and re-stamped old data wins LWW (RFC 09 §5.2)
zenctl get '@/**' --zenoh-config tls.json5       # your JSON5 as the base layer — TLS/QUIC/usrpwd reachable
zenctl admin graph --dot | dot -Tsvg > mesh.svg  # the mesh, labeled: heard-of nodes dashed, you bold
zenctl storage list --base acme         # declared state subjects vs storage coverage
zenctl blob list --base acme            # who declares which @blob tier (registry only)
zenctl blob probe 01jqz3demo0001        # who *holds* it, and at which content root
zenctl blob fetch 01jqz3demo0001 --from h-3fa9 --root <hex> -o bundle.bin
zenctl doctor --base acme --registry path/to/registry
zenctl doctor --deep --sample 10 --fail-on error   # bounded deep sweep; exit 1 on errors
zenctl context create lab --base acme -c tcp/…   # named contexts; completions <shell>
zenctl context edit                     # the whole config file, in $EDITOR, validated
```

`--watch` re-renders on change (appeared rows mark `+` for one cycle,
disappeared rows linger one cycle marked `-`, and a row whose *value* changed
shows as both); `--watch --format ndjson` streams one envelope plus its rows
per cycle, tagged with a monotonic `tick`. `node list --watch` is event-driven
— a producer stopping shows within one liveliness event, not one poll interval.

## What `--format` promises

**`--format json` and `--format ndjson` are a stable contract. `--format table`
explicitly is not.** The table is for a person to read, and it changes when a
better rendering is found; anything a script depends on belongs in one of the
other two.

Both machine formats carry the same values, differently packaged:

* **`json`** — one document: the report's own fields, its `rows` array, and a
  `notes` array when the report has something to say about itself.
* **`ndjson`** — one object per line. The **envelope leads**, carrying the
  report-level facts and the notes; then one line per row, each tagged with the
  kind of row it is:

```console
$ zenctl storage list --base acme --format ndjson
{"report":"storage-list","notes":[…]}
{"row":"storage","name":"main","zid":"…"}
{"row":"coverage","producer":"sysinfo","path":"health","coverage":"covered"}
```

`jq -c 'select(.row)'` takes the rows, `select(.report)` the envelope. The
envelope leads rather than trails so that a stream cut short — `| head`, a
closed pipe — still carries what was asked and what the bounds cost, which is
exactly the claim a truncated stream needs (RFC 09 §5.1 O5/O6).

Streaming verbs (`topic echo`, `serve`, `replay`, `gen`) emit tagged rows with
**no** envelope: their coverage is not known before the first row, and
`topic echo`'s rows are an *input* format that `topic pub --from ndjson` and
`.zrec` read back (RFC 09 §5.2), so nothing may precede them.

A field that is absent is a question nobody asked; it is never `null`
(RFC 09 §5.1 O4). In the table, that reads `—`, and an empty cell means the
question was asked and the answer was nothing.

**`--as` and `--dot` are neither, because they are somebody else's schema.**
`--format` selects among zenctl's own three renderings of a report; `registry
export --as toml|jsonschema|asyncapi` and `admin graph --dot` emit a foreign
document, and their stability is whatever the format's own specification says.
The two are mutually exclusive: typing both is a usage error naming both flags,
rather than a `--format` that is accepted and then ignored. An exported
`ZENCTL_FORMAT` is a preference, not a request, and does not conflict with
anything.

`get` speaks the fleet discipline on any selector — target All,
consolidation None, every reply attributed by its own key, RFC 05 §3 error
envelopes rendered as errors, and exit codes scripts can branch on (0 values,
1 an error reply, 2 silence — which still prints its non-verdict paragraph).
`topic retire` publishes an authoritative tombstone through a declared
publisher: state keys retire freely, anything else is the RFC 04 §1.2 (v1.12)
operator act and needs `--i-know`; wildcards are refused outright. `scout` is
the one verb where multicast is on by default — it only listens, and an empty
result names the boundary it heard.

`topic pub` and `service call` **encode** a JSON body against the producer's
served schema (request types come from the slice's procedure declaration),
and those encoded bytes are what goes on the wire, labelled with the declared
`Encoding`. Publishing to a subject that declares `application/protobuf`
therefore puts protobuf on the bus, not the JSON you typed; `topic echo`
decodes it back through the same descriptor set.

A body the schema cannot encode is refused before it touches the bus.

The three `blob` commands cost three very different things, and the surface
says which is which. `list` reads registry slices and touches no data plane:
it answers "who *declares* a tier", which is a capability claim and never
possession. `probe` fans two tiny GETs (`have`, `manifest`) across origins —
RFC 07 §2.5's sanctioned form — and reports every holder with its own concrete
key. `fetch` moves bytes from exactly one of them, at **data-low** priority
(§2.6), verifying each reply against the content root **before disk** (§2.1).

`--from` is required and takes one concrete origin; `RemoteOrigin::parse`
rejects `*`, so a wildcard-origin bulk fetch has no spelling here. A tier-1
fetch also requires `--root <hex>` or an explicit `--allow-unpinned`: §2.1 says
a reference must carry the identity of the bytes it names, and an operator
typing an id by hand has no reference — so trust-on-first-use is a decision
made out loud, and the report says which one you made.

Two holders answering one id at two different roots is a **finding**, not a
tie-break. `probe` prints both and refuses to choose; the root you pin is what
the fetch will accept.
`--no-validate` drops the refusal (the body ships as typed, with a note);
`--raw` skips the schema lookup entirely and sends the bytes verbatim. A
producer serving no schema validates nothing — silence is not a verdict about
the type, and the tool says which of the three cases happened rather than
letting them look alike.

`topic pub` also prints a matching note ("a subscriber currently matches …")
— a routing fact about *this* publisher, never a fleet verdict.

`node list` is a liveliness query on `<base>/v1/*/state/*/alive` — RFC 04 §5's
"entire fleet-presence protocol, zero payload bytes". The token *key* is the
record.

`topic echo` walks wire key → subject → payload type → value with nothing
compiled in: the registry slices bind one payload type per subject (P5), and
the value renders generically (JSON, CBOR→JSON diagnostic, text, or hex —
tagged with the declared type name).

`schema <producer>` dumps the served `describe` reply (RFC 08 §7) and
`interface show <Type> --schema` asks every producer that carries the type, so
two producers disagreeing about one name shows up as the drift it is. A
producer serving no `describe` says so — undescribed is not shapeless.

## `registry` — the registry as a document

```bash
zenctl registry export --as toml       # round-trips back through --registry
zenctl registry export --as jsonschema # bundled from the served describe sets
zenctl registry export --as asyncapi   # channels from subjects, ops from procedures
zenctl registry diff                   # local --registry dirs vs what the fleet serves
zenctl registry lint <dir>             # the consumer build's own RFC 08 §5 lints
```

`lint` runs `zenkey-build`'s lints, not a second copy of them — the diagnostic
is byte-for-byte what the application's `build.rs` would print, which is the
only version worth having. `diff` is the side-by-side that `doctor` turns into
judgements: a producer present on one side only is a fact with two very
different explanations, and the output says which.

## Completions

```bash
source <(zenctl completions bash)      # zsh, fish, elvish, powershell too
```

The script is **dynamic**: it calls back into `zenctl`, so producer, type,
procedure and key candidates come from the cached registry of the active
context. Completion never opens a session — a `<TAB>` cannot hang on a fleet
that is down — and with no cache it degrades to the static command tree.

Any command that loads slices fills the cache; `zenctl cache show|refresh|clear`
makes it visible, current, or gone. The names are from the last sighting, not
a live inventory. `--static` emits the old self-contained script.

## `bench rpc` — how fast, and *which origin* is slow

```bash
zenctl bench rpc '*' sysinfo --count 200 --concurrency 8
```

Latency is measured **per reply**, not per call: a fan-out GET finishes when
the slowest origin answers, so charging that duration to every responder would
report the fastest node's latency as the worst one's. Error replies and calls
that drew *no* reply are counted separately from the distribution — averaging a
non-answer into a latency figure is how a benchmark lies.

Only procedures the registry declares `idempotent = true` bench by default; a
benchmark repeats, and repeating a write into a live fleet is a different act
from measuring it. `--i-know` overrides. The convention's own reads
(`introspect`, `describe`) need no registry permission — RFC 08 §6/§7 define
them, so their idempotence is not an application's to declare.

## `doctor` — the one `ros2` has no answer for

`introspect` is served by the *running binary*, from the same source as its key
constants — so it cannot drift from behavior. RFC 08 §6:

> A disagreement between introspection and the checked-in TOML is a **finding,
> not an ambiguity**: the TOML says what *should* run, the introspection says
> what *does*.

`doctor` fans `introspect` across the fleet and diffs each reply against the
`--registry` TOMLs:

```
$ zenctl doctor --base acme --registry registry -c tcp/127.0.0.1:7447
✗ h-9706b31ddad3/sysinfo: registry 1.1 (we compiled 1.2)
✗ h-9706b31ddad3/sysinfo: does not serve telemetry thermal/{zone}/temp_celsius
2 finding(s).
```

Version skew, subjects a host serves that we cannot name, subjects we expect
that it does not publish, and hosts still serving a deprecated subject — in one
round trip, without SSH.

## Things it will not do, on purpose

- **Silence is never a verdict.** RFC 05 §3.1: an empty reply set conflates an
  offline host, a mistyped origin, and a procedure that is not served. `service
  call` says so rather than guessing; `node list` is what attributes it.
- **Errors are never dressed up as success.** RFC 05 §3: a value reply always
  means success, a failure always rides `reply_err`. An error reply goes to
  stderr with its `error/...` name.
- **No namespace.** RFC 09 §5: debug tools run *without* the session namespace
  and spell full keys — "the honest view of what is on the wire".
- **Scouting is off by default.** A scouting explorer joins whatever mesh it can
  find, which is how a throwaway session ends up talking to a production fleet.
  `--scouting` is opt-in, and you should mean it.
- **Payload schemas are shown, not invented.** RFC 01 §5 keeps payload
  *definitions* with the owning applications, and this tool has no opinion
  about their contents. But since RFC 08 §7, a producer **serves** its shapes
  on `@rpc/<producer>/describe`, so `zenctl schema <producer>` and
  `interface show --schema` print served data rather than sending you to
  `curl`. (This bullet used to say the opposite; it predated §7.) A producer
  serving no `describe` degrades honestly — "undescribed" is not "no shape".

## Fan-in discipline

Every fleet GET goes through one helper (`zenkey_fleet::fleet_get`) because RFC 05 §2.1's
requirements fail *silently* when forgotten:

- **target = All** — the default `BestMatching` short-circuits to a single
  queryable the moment any matching one is declared `complete`, which is "one
  storage config away from silently collapsing the fleet to one reply";
- **consolidation = None** — default consolidation keeps one reply per reply key;
- **attribution by the reply's own concrete key**, never by the key we asked on.

Note `*` in the origin position can never match a verbatim service origin
(design property D4), so `@catalog` is always asked for by name. That is the
grammar working, not an exception to it.

## License

Apache-2.0.
