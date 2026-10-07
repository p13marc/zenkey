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

**For an operator**, in order: [Install](#install) ·
[Production quickstart](#production-quickstart) ·
[Session posture](#session-posture) · [Exit codes](#exit-codes) ·
[Monitoring recipes](#monitoring-recipes) · [Write guards](#write-guards) ·
[Every command](#every-command). The rest of this page is the reasoning behind
the surface: what `--format` promises, where registry knowledge comes from, and
what the tool will not do.

`--base` (or `ZENCTL_BASE`) names the deployment base — the first chunk(s) of
every key on the wire. Applications set it as their session namespace and never
spell it; `zenctl` runs un-namespaced on purpose (RFC 09 §5), so it has to be
told about a *named* base. Left unset, it defaults to the **empty base** — the
base-less bus-root deployment whose keys start at `v1/`, the RFC v1.6 default —
so against a default-configured fleet `zenctl` works with no `--base` at all.
Don't know the base? `zenctl base list` discovers the bases actually in use.

**Selectors are wire keys.** `--base` is for discovery and for the selectors
zenctl composes itself; a selector you type is used exactly as typed. Under
`--base prod`, `zenctl echo 'v1/**'` listens to a keyspace nobody publishes on
— so zenctl says so on stderr (`hint: "v1/**" does not sit under base "prod" …
did you mean "prod/v1/**"?`) and carries on. Leave the selector out and the
verb watches `<base>/v1/**` for you.

## Install

Each release (bare tags, `X.Y.Z`) attaches a **linux x86_64** binary,
`zenctl-X.Y.Z-linux-x86_64`, beside `SHA256SUMS` and the source tarball. Every
other platform builds from source — and building from source is the install
everywhere else:

```bash
cargo install --git https://git.marcpardo.eu/marcpardo/zenkey zenctl --tag 0.12.0 --locked
```

**The v1 line.** This zenctl reads the v1 convention, which is frozen at RFC
v1.50 and maintained on the `v1` branch as 0.14.x patch releases. To build
the newest v1 patches from source, track the branch rather than a tag:

```bash
cargo install --git https://git.marcpardo.eu/marcpardo/zenkey zenctl --branch v1 --locked
```

`main` is the zk2 line (`docs/zk2/`).

`--locked` builds against the release's own `Cargo.lock`, the dependency set
that release was tested with; leave it out and cargo resolves fresh versions.
The toolchain is **Rust 1.98** (the workspace `rust-version`, pinned by
`rust-toolchain.toml`). From a checkout or the release tarball, `cargo build
--release -p zenctl --locked` does the same.

`zenctl --version` names the build as well as the crate:

```console
$ zenctl --version
zenctl 0.12.0 (0.14.0)
```

The first number is the crate's own version; the parenthesis is `git describe
--tags --always --dirty` of the tree it was built from — the release tag, or
`<tag>-<n>-g<commit>` between releases, `-dirty` for local edits. The release
tarball carries its description through `git archive`; a build with neither
git nor an archive behind it says `unknown`. Put that line in a bug report: it
is the one fact that tells two builds apart.

**Supported routers.** zenctl is built on zenoh **1.10** (the workspace pins
`zenoh = "1.10"` with the `unstable` feature; the lock file holds 1.10.0), and
its test suites — the live suite included — run that same zenoh on both ends
of every session. That makes **zenohd 1.10.x** the tested line. Other zenohd
versions are zenoh's own wire-compatibility question, and nothing here tests
them.

## Production quickstart

Three steps: a zenoh config that reaches the secured router, a named context
that remembers it, and a first question.

**1. The zenoh config.** zenctl takes zenoh's own JSON5 (`--zenoh-config`) as
its base layer, so TLS, mutual TLS, QUIC and usrpwd are all reachable without a
flag of zenctl's own. [`examples/prod.json5`](examples/prod.json5) is a
complete one — client mode, a `tls/` endpoint, a CA, a client certificate,
usrpwd, multicast off — and a test opens a session through that very file, so
it cannot drift from what zenctl parses:

```json5
{
  mode: "client",
  connect: { endpoints: ["tls/zenoh-router.example.net:7447"] },
  scouting: { multicast: { enabled: false } },
  transport: {
    link: {
      tls: {
        root_ca_certificate: "/etc/zenctl/prod/ca.pem",
        enable_mtls: true,
        connect_certificate: "/etc/zenctl/prod/zenctl.pem",
        connect_private_key: "/etc/zenctl/prod/zenctl-key.pem",
        verify_name_on_connect: true,
      },
    },
    auth: { usrpwd: { user: "zenctl-ops", password: "change-me" } },
  },
}
```

Keep it `0600` — it can hold a password. A file that sets a session
`namespace` is refused: an explorer that stripped keys would be lying about the
wire (RFC 09 §5). The router's side of the same certificates — its
`access_control` block, keyed on the client certificate's CN — is what
`zenctl acl gen` writes.

**2. A named context**, so no command line has to carry the base and the file:

```bash
zenctl context create prod --base prod --zenoh-config /etc/zenctl/prod.json5 --select
zenctl context show --format json | jq -r .base      # → prod
```

A context holds `--base`, `-c`/`-l` endpoints, `--registry` dirs, `--timeout`,
`--scouting` and `--zenoh-config`; `zenctl context list|select|rm|edit` manage
the file (`~/.config/zenkey-explorer/config.toml`, shared with zengui). Every
value resolves flag > environment (`ZENCTL_BASE`, `ZENCTL_CONTEXT`,
`ZENCTL_ZENOH_CONFIG`) > the context > the zenoh file, so `--context staging`
or `-c tls/other:7447` overrides it for one invocation.

**3. Ask.**

```bash
zenctl node list                                   # who is alive (liveliness roster)
zenctl doctor --registry path/to/registry          # does the fleet match what we ship?
zenctl echo --class state                          # current state traffic, decoded
```

## Session posture

What the session every verb opens is, and is not (RFC 09 §5):

* **A client.** No listener, no gossip, nothing the mesh can open a link to or
  route through. `-l`/`--listen` (or a context that listens, or a config file
  that states `mode`) is how you ask for a **peer**, and you should mean it.
* **No multicast scouting**, with `--zenoh-config` too, unless the file itself
  states `scouting.multicast.enabled` or you pass `--scouting`. A scouting
  explorer joins whatever mesh answers. `zenctl scout` is the one verb where
  multicast is on by default — it only listens for Hellos and opens no
  session.
* **No silent empty bus.** A router that does not answer fails the session,
  and an endpoint that does not parse (`-c 127.0.0.1:7447`, no `tcp/`) is
  refused by name: both exit **2** for every verb, `pub` and `retire`
  included. A verb holding `--registry` dirs still answers from them, and says
  so.
* **Un-namespaced**, so it sees the wire as it is — including traffic from
  outside the deployment, which is how a leak is spotted.

## Exit codes

One contract for the whole tool, written once in
[`src/exit.rs`](src/exit.rs):

| code | means | for example |
|---|---|---|
| **0** | asked, and the answer is clean | values came back; the assertion held; the act completed; a listing that found nothing is still an answer |
| **1** | asked, and the answer is a **finding** | an assertion did not hold; a reply was an error envelope; `--fail-on` tripped; `why` established a cause; an act (`pub`, `replay`, `gen`, `blob fetch`, `registry migrate`) failed |
| **2** | **no verdict**: the question could not be asked or proven | a usage error; input zenctl refuses (a bad expression, an unknown `--context`, an existing `-o` file); a session that never opened; silence under a fan-out; an observation too impaired (drops) to carry the claim |

Verdict verbs (`check *`, `why`) land **any** failure before the question was
put on 2, so a dead bus never reads as a pass or as a finding.

**Nagios and friends.** Their convention is 0 OK, 1 WARNING, 2 CRITICAL,
3 UNKNOWN, and zenctl's 2 is the UNKNOWN, not the CRITICAL. Map it:

```bash
#!/bin/sh
# check_zenkey_doctor — a Nagios/Icinga plugin over zenctl's exit contract.
out=$(zenctl doctor --context prod --registry /srv/registry --fail-on error 2>&1)
case $? in
  0) echo "OK - fleet matches its contracts"; exit 0 ;;
  1) echo "CRITICAL - $out" | head -n 5; exit 2 ;;
  *) echo "UNKNOWN - no verdict: $out" | head -n 5; exit 3 ;;
esac
```

## Monitoring recipes

```bash
# The fleet against what we ship: exit 1 on an error-severity finding.
zenctl doctor --registry /srv/registry --fail-on error
zenctl doctor --deep --sample 10 --for 30 --fail-on warning   # + freshness, coverage, live traffic

# One expectation, for CI or a cron job: at least one health sample in 60 s.
zenctl check expect 'prod/v1/*/state/sysinfo/health' --for 60 --at-least 1
zenctl check expect 'prod/v1/*/telemetry/**' --for 30 --rate-min 1 --valid-payload
zenctl check expect 'legacy/**' --for 60 --absent      # silence, asserted (2 if unprovable)

# A producer against its own registry, as a JUnit report CI can read.
zenctl check conform --producer sysinfo --registry /srv/registry --junit conform.xml

# Prometheus: a scrape target (loopback by default; another address needs --i-know)…
zenctl export --bind 127.0.0.1:9184 --validate --doctor-every 300
# …or one fold for the node_exporter textfile collector.
zenctl export --once --prom --for 10 > /var/lib/node_exporter/zenkey.prom.$$ \
  && mv /var/lib/node_exporter/zenkey.prom.$$ /var/lib/node_exporter/zenkey.prom

# Conditions, as ndjson transitions (ok / firing / unobservable); --count bounds a run.
zenctl watchdog --rule 'silent-for prod/v1/*/state/sysinfo/health 120' \
                --rule 'origin-down h-3fa9c2d41b7e' --rule dropped --count 12
zenctl doctor --transitions --every 60                 # check-id changes, not states

# Why is this key silent? Each rung established, not established, or not asked.
zenctl why 'prod/v1/h-3fa9c2d41b7e/state/sysinfo/health' --for 10
```

`watchdog` and `doctor --transitions` print **changes**: the first evaluation
states each baseline once, and an unchanged tick prints nothing. Read the
stream (`jq 'select(.to == "firing")'`) while it runs; a bounded run
(`--count N`) also ends on how its rules ended — 1 if one is firing, else 2
if one is unobservable, else 0. A drop under a completeness claim is
`unobservable`, never `ok`. For alerting
while nobody is watching a terminal, [`zenwatch`](../zenwatch/) is the daemon
over the same vocabulary, with sinks.

## Write guards

zenctl can publish, call and serve, so the verbs that write ask before they
reach further than one concrete thing:

| verb | refused (exit 2) | to mean it |
|---|---|---|
| `pub` | a wildcard key (a blast radius, not a publication) | not overridable — name the key |
| `pub --from ndjson`, `replay` | a put row on a wildcard key (refused and counted, exit 1) | not overridable |
| `retire` | a wildcard; a key that is not state-shaped | `--i-know` for the non-state key |
| `service call` | a `*` fan-out to a procedure nobody could establish as fan-out-safe | `--i-know`; a declared forbidden fan-out is never overridable |
| `config set` | a windowed (`--confirm`) change with no read-back, from a script | `--yes` |
| `replay` | a capture whose base differs from the target's, or empty onto empty | `--force-base` (always `--dry-run` first) |
| `serve` | a wildcard key expression; `--complete` | `--i-know` |
| `gen` | `--origin <host>` (publishing as a real host); `--fault`; more than 10 subjects | `--i-know` (faults also need an endpoint or base typed on the line); `--wide` |
| `export` | a non-loopback `--bind` | `--i-know` |
| `record`, `snapshot` | an `-o` file that already exists | `--overwrite` |
| `bench rpc` | a procedure not declared `idempotent` | `--i-know` |

Each refusal names its reason and comes before a session opens wherever the
command line alone decides it. [`CHANGELOG.md`](CHANGELOG.md) has the
before/after of every one.

## Every command

`zenctl --help` lists the tree; `zenctl <verb> --help` gives the long form, with
the RFC sections behind each behaviour. Grouped the way the tree is built
(nouns get verbs under them; acts and observations on traffic hang off the
root; judgements are exit-coded):

**Discover — what exists.**
`zenctl base list` (the deployment bases in use; needs no `--base`) ·
`zenctl node list|info` (the liveliness roster; one node's producers, versions,
capabilities) · `zenctl topic list|info` (subjects the registry declares; one
key refined against it) · `zenctl service list|info` (procedures) ·
`zenctl interface list|show` (payload types) · `zenctl schema show <producer>`
(the served `describe` shapes) · `zenctl admin routers|graph` (zenoh's admin
space: routers and peers with version and locators; the mesh, `--dot` for
Graphviz) · `zenctl scout` (raw scouting Hellos) · `zenctl key
includes|intersects|canon` (key-expression algebra, offline).

**Watch — live traffic.**
`zenctl get <selector>` (a fan-in GET, every reply attributed to its key;
`@/**` browses the admin space) · `zenctl echo` (subscribe and decode; `--seed`
pulls current state first) · `zenctl rate` (per-key rates, `--bytes` for
bandwidth) · `zenctl field --for 60` (per-field statistics: the stuck sensor,
the vanished field, the field the schema never declared) · `zenctl timeline
--for 10` (one merged ordering, a lane per origin, the clock stated; `--from`
a capture).

**Capture — keep what happened.**
`zenctl record -o bus.zrec --for 30` (a capture, drops recorded where they
fell) · `zenctl record -o incident.zrec --on 'silent-for prod/v1/** 30' --pre
30` (armed: written only when a rule fires, with the thirty seconds before it
and a state preamble) · `zenctl replay bus.zrec --dry-run` (replay is
publishing: preview first) · `zenctl snapshot -o fleet.zsnap` (the fleet's
current values, collected over a span) and `zenctl snapshot diff a.zsnap
b.zsnap` (`--normalize-origins` across two deployments).

**Act — write to the bus.**
`zenctl pub <key> <body>` (through a declared publisher, encoded against the
served schema; `--from ndjson` reads `echo`'s rows back) · `zenctl retire <key>`
(an authoritative tombstone) · `zenctl service call <origin> <producer>
<procedure>` (`--trace` subscribes first, then calls) · `zenctl config
get|set|confirm|cancel|extend|persist` (a producer's live configuration, typed
against its served schema, with confirmed changes driven to their end) ·
`zenctl serve <keyexpr> <reply>` (a mock queryable that logs every ask) ·
`zenctl gen --producer sysinfo` (registry-driven test traffic, every sample
marked synthetic) · `zenctl blob locate|fetch` (bulk content: who holds it,
and a verified fetch from one origin; `zenctl blob list` reads only the
registry).

**Judge — exit-coded.**
`zenctl check expect` (an expectation over a window) · `zenctl check cutover`
and `zenctl check probe` (the two halves of cutover acceptance: the old family
silent while the new one speaks; a consumer-shaped probe with concrete keys) ·
`zenctl check retired` (which `[[deprecated]]` subjects are actually gone) ·
`zenctl check conform` (a producer's registry as a conformance suite) ·
`zenctl check schema` (one payload against its schema) · `zenctl doctor` (the
fleet against the contracts it claims) · `zenctl why <key>` (why it is silent)
· `zenctl watchdog --rule …` (conditions, as transitions) · `zenctl export`
(the bus and its contract as Prometheus metrics, the observer's own blind spots
included).

**The registry — as a document.** See [below](#registry--the-registry-as-a-document):
`zenctl registry export|diff|lint|lock|consumers|impact|infer|migrate`.

**Router configuration — generated, then checked.**
`zenctl storage list` (configured storages, and which declared state they
cover) · `zenctl storage gen --deployment storages.toml --json5` (the
`plugins.storage_manager` block, lifespans derived from the registry's
`ttl_s`; `--check` compares a live router) · `zenctl acl gen --enrollment
enroll.toml --json5` (the `access_control` block, one principal per
certificate CN; `--check` likewise).

**The tool itself.**
`zenctl context …` (named connection contexts, above) · `zenctl cache
show|refresh|clear` (the slice cache behind completion) · `zenctl completions
<shell>` · `zenctl bench rpc` (procedure latency, per origin).

> **The command tree moved (#307).** `topic echo` → `echo`, `topic pub` →
> `pub`, `topic hz`/`topic bw` → `rate`, `expect`/`cutover`/`probe`/`registry
> retired`/`schema check` → `check …`, `blob probe` → `blob locate`; every
> observation window is `--for <SECS>`; `why` exits 1 on a finding, and a
> refused input exits 2 everywhere. No aliases, no shims — the old spellings
> are gone. [`CHANGELOG.md`](CHANGELOG.md) has the full old→new table and the
> exit-code contract.

## Two registry sources, kept visibly apart

| | Answers from | Works when the fleet is down | Tells you |
|---|---|---|---|
| **`--registry <dir>`** | local registry files — `*.toml`, or `*.kdl` (RFC 08 §5.1) | yes | what *should* exist (declared) |
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
  (`zenctl schema show sysinfo --type TelemetryPoint` for the served shape)
qos       sampled
cardinality  ~512 keys expected
```

**Declared is not observed.** A pattern with a trailing rest-variable
(`{device}/{path...}`) fixes a *shape*, not its members — proxy producers
register that way by design, because their metric tree belongs to the polled
device. `topic list` flags those `[open-ended]`; `echo` is what
enumerates them.

## A cheat sheet

```bash
zenctl base list -c tcp/127.0.0.1:7447  # discover deployment bases (needs no --base)
zenctl node list --base acme            # the liveliness roster (--verbose joins introspect)
zenctl node list --base acme --watch    # …re-rendered per liveliness event (no polling)
zenctl echo --base acme                 # subscribe + decode (defaults to <base>/v1/**)
zenctl topic list --base acme --watch --every 5  # topic/storage/base list poll+diff; +/- marks
zenctl rate --base acme --per-key       # per-key sample rates; --bytes for bandwidth
zenctl service call --base acme '*' sysinfo processes --param sort=cpu
zenctl service call --base acme h-3fa9 netring capture/trigger --body @trigger.json
zenctl get 'acme/v1/*/state/**'         # fan-in GET on any selector, replies attributed
zenctl get '@/**'                       # …including the zenoh admin space (was: admin get)
zenctl pub k '{"v":1}' --attachment meta        # attachments ship and render (#117)
zenctl retire acme/v1/h-3fa9…/state/sysinfo/health  # RFC 04 §1.2 tombstone, class-guarded
zenctl scout                            # raw Hellos: zid/whatami/locators (multicast ON here)
zenctl serve 'demo/mock/**' '{"ok":1}'  # mock queryable; logs every ask (who queries this key?)
zenctl key intersects 'v1/**' 'v1/h-1/@rpc/p/x'  # keyexpr algebra, no session; cites D2/D4 on a convention-shaped no
zenctl echo --format ndjson > f         # …and back: zenctl pub --from ndjson < f (one row shape, both directions)
zenctl record --base acme -o bus.zrec --for 10  # capture: same row shape + header + pacing + in-file drop ledger
zenctl replay bus.zrec --dry-run        # ALWAYS preview first — replay is publishing, and re-stamped old data wins LWW (RFC 09 §5.2)
zenctl get '@/**' --zenoh-config tls.json5       # your JSON5 as the base layer — TLS/QUIC/usrpwd reachable
zenctl admin graph --dot | dot -Tsvg > mesh.svg  # the mesh, labeled: heard-of nodes dashed, you bold
zenctl storage list --base acme         # declared state subjects vs storage coverage
zenctl blob list --base acme            # who declares which @blob tier (registry only)
zenctl blob locate 01jqz3demo0001       # who *holds* it, and at which content root
zenctl blob fetch 01jqz3demo0001 --origin h-3fa9 --root <hex> -o bundle.bin
zenctl doctor --base acme --registry path/to/registry
zenctl doctor --deep --sample 10 --fail-on error   # bounded deep sweep; 1 on errors, 2 if nothing was judged
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

Streaming verbs (`echo`, `serve`, `replay`, `gen`) emit tagged rows with
**no** envelope: their coverage is not known before the first row, and
`echo`'s rows are an *input* format that `zenctl pub --from ndjson` and
`.zrec` read back (RFC 09 §5.2), so nothing may precede them.

A field that is absent is a question nobody asked; it is never `null`
(RFC 09 §5.1 O4). In the table, that reads `—`, and an empty cell means the
question was asked and the answer was nothing.

**`pub` and `retire` put nothing on stdout, in any format.** Their
answer is "it went out", which is not a document — and the empty stdout is what
lets `zenctl echo --format ndjson | zenctl pub --from ndjson` compose. Everything
they say goes to stderr.

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
`retire` publishes an authoritative tombstone through a declared
publisher: state keys retire freely, anything else is the RFC 04 §1.2 (v1.12)
operator act and needs `--i-know`; wildcards are refused outright. `scout` is
the one verb where multicast is on by default — it only listens, and an empty
result names the boundary it heard.

`pub` and `service call` **encode** a JSON body against the producer's
served schema (request types come from the slice's procedure declaration),
and those encoded bytes are what goes on the wire, labelled with the declared
`Encoding`. Publishing to a subject that declares `application/protobuf`
therefore puts protobuf on the bus, not the JSON you typed; `echo`
decodes it back through the same descriptor set.

A body the schema cannot encode is refused before it touches the bus.

The three `blob` commands cost three very different things, and the surface
says which is which. `list` reads registry slices and touches no data plane:
it answers "who *declares* a tier", which is a capability claim and never
possession. `locate` fans two tiny GETs (`have`, `manifest`) across origins —
RFC 07 §2.5's sanctioned form — and reports every holder with its own concrete
key. `fetch` moves bytes from exactly one of them, at **data-low** priority
(§2.6), verifying each reply against the content root **before disk** (§2.1).

`--origin` is required and takes one concrete origin; `RemoteOrigin::parse`
rejects `*`, so a wildcard-origin bulk fetch has no spelling here. A tier-1
fetch also requires `--root <hex>` or an explicit `--allow-unpinned`: §2.1 says
a reference must carry the identity of the bytes it names, and an operator
typing an id by hand has no reference — so trust-on-first-use is a decision
made out loud, and the report says which one you made.

Two holders answering one id at two different roots is a **finding**, not a
tie-break. `locate` prints both and refuses to choose; the root you pin is what
the fetch will accept.
`--no-validate` drops the refusal (the body ships as typed, with a note);
`--raw` skips the schema lookup entirely and sends the bytes verbatim. A
producer serving no schema validates nothing — silence is not a verdict about
the type, and the tool says which of the three cases happened rather than
letting them look alike.

`pub` also prints a matching note ("a subscriber currently matches …")
— a routing fact about *this* publisher, never a fleet verdict.

`node list` is a liveliness query on `<base>/v1/*/state/*/alive` — RFC 04 §5's
"entire fleet-presence protocol, zero payload bytes". The token *key* is the
record.

`echo` walks wire key → subject → payload type → value with nothing
compiled in: the registry slices bind one payload type per subject (P5), and
the value renders generically (JSON, CBOR→JSON diagnostic, text, or hex —
tagged with the declared type name).

`schema show <producer>` dumps the served `describe` reply (RFC 08 §7) and
`interface show <Type> --schema` asks every producer that carries the type, so
two producers disagreeing about one name shows up as the drift it is. A
producer serving no `describe` says so — undescribed is not shapeless.

## `registry` — the registry as a document

```bash
zenctl registry export --as toml       # round-trips back through --registry (--as kdl too)
zenctl registry export --as jsonschema # bundled from the served describe sets
zenctl registry export --as asyncapi   # channels from subjects, ops from procedures
zenctl registry diff                   # local --registry dirs vs what the fleet serves
zenctl registry lint <dir>             # the consumer build's own RFC 08 §5 lints
zenctl registry lock <dir>             # write/update registry.lock; an incompatible edit is refused
zenctl registry consumers sysinfo/health  # who declares a reader of it, one row per session
zenctl registry impact sysinfo/health  # its readers, storage coverage, family, deprecation — one document
zenctl registry infer --out draft/ --for 60   # a draft registry from the wire, marked draft
zenctl registry migrate --to kdl registry --out registry-kdl  # TOML → KDL, all or nothing
```

A registry directory may be written in TOML or in KDL — the same document in
two spellings (RFC 08 §5.1) — and every `--registry <dir>` reads either.
`lock` keeps the compatibility pins (RFC 08 §3.1): additive evolution and
`[[deprecated]]` retirement regenerate cleanly, an incompatible change to an
existing path is refused, and `--force` overrides out loud. `consumers` and
`impact` are what to run *before* changing a subject: a declaration is not
proof of use, and an admin space that does not answer reads "not asked",
never "nobody". `infer`'s draft is refused by `zenkey-build` until a review
removes the marker; `registry lint --allow-drafts` checks it meanwhile.

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
zenctl bench rpc '*' sysinfo --calls 200 --concurrency 8
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

As a monitoring job, give it a threshold — findings are output, not verdicts,
until you do:

```
zenctl doctor --base acme -c tcp/router:7447 --fail-on error
```

0 is a fleet with no error finding, 1 is one with an error finding, and **2
is no verdict**: the session never opened, or it opened onto a bus where no
producer holds an `alive` token and no router answers — a wrong endpoint or a
wrong `--base` looks exactly like that, and it is never green (#510).
`watchdog --count N` follows the same contract on how its rules ended:
1 firing, 2 unobservable, 0 ok (#511).

## Things it will not do, on purpose

- **Silence is never a verdict.** RFC 05 §3.1: an empty reply set conflates an
  offline host, a mistyped origin, and a procedure that is not served. `service
  call` says so rather than guessing; `node list` is what attributes it.
- **Errors are never dressed up as success.** RFC 05 §3: a value reply always
  means success, a failure always rides `reply_err`. An error reply goes to
  stderr with its `error/...` name.
- **No namespace.** RFC 09 §5: debug tools run *without* the session namespace
  and spell full keys — "the honest view of what is on the wire".
- **No scouting, and not a peer**, unless you ask — see
  [Session posture](#session-posture). A scouting explorer joins whatever mesh
  it can find, which is how a throwaway session ends up talking to a
  production fleet.
- **Payload schemas are shown, not invented.** RFC 01 §5 keeps payload
  *definitions* with the owning applications, and this tool has no opinion
  about their contents. But since RFC 08 §7, a producer **serves** its shapes
  on `@rpc/<producer>/describe`, so `zenctl schema show <producer>` and
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
