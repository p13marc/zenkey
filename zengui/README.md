# zengui

A graphical bus explorer for Zenoh — the GUI sibling of [`zenctl`](../zenctl),
over the same engine ([`zenkey-fleet`](../zenkey-fleet)).

**Key-agnostic by construction.** zengui is a useful explorer on *any* Zenoh bus.
It groups, counts and renders whatever keys it observes, with no assumption that
they follow any convention. [zk2](../docs/zk2/README.md) is an *overlay*: a key
that resolves through the deployment's presence and contracts gains its
address, interface and resource, a contract-typed decode and a conformance
badge. A key that does not resolve is still a first-class node in the tree,
with counts, rates and structural payload rendering, and it says where
resolution stopped. "Does not resolve" is an answer, not an error.

zengui runs on zk2 since #614 ([CHANGELOG](CHANGELOG.md) has the old→new
tables). v1's zengui, which read the keyspace-v2 grammar
(`v1/<origin>/<class>/<producer>/…`) through registry slices, is on the `v1`
branch; on this one a `v1/…` key is a foreign key.

## Two sessions, one namespace

zengui opens **two sessions**, as zenctl and zenwatch do:

- the **raw** session, in **no** namespace, runs the data pump, the wire
  selectors and the admin space. It spells full wire keys, which is what lets
  it see traffic a namespaced application is blind to (RFC 09 §5);
- the **namespaced** session, **in** the deployment's namespace, reads
  presence, descriptors and contracts the way the deployment's consumers do,
  and asks every resolved question: an owner's state, health, why, a call.

The namespace resolves `--namespace` (alias `--base`) > the one picked last >
`ZENGUI_BASE` > the context's `base` > empty. The empty namespace is legal and
is the default: an explorer that cannot see and name the bus-root deployment
is blind to the common case. Namespaces in use are listed on connect
(`zenkey_fleet::namespace_listing`) and offered in the location bar's picker,
so `--namespace` is a shortcut, not a requirement. A switch reopens the
namespaced session, the presence monitor and the lens, and forgets what they
held.

**Contracts** come from `--contracts <PATH>` (repeatable: an authoring file, a
directory of them, or a `.history` root, loaded as `zenctl --contracts` loads
them) or from the Settings overlay, which remembers them in zengui's own
preferences, never in the shared context. They seed the contract store; every
other revision a descriptor names is retrieved by fingerprint from the
holders that serve it (spec §8.4). `--registry` is v1's, and is refused by
name.

## The lens and presence

**Presence** runs on a monitor of its own on the namespaced session, so the
pump's `Dropped(n)` cannot knock it to lost: each address up, down or
unobservable, since when, its instances and members. Unobservable is never
down, and "presence not read" is never an empty deployment.

**The lens** is what a raw key resolves through: the namespace, the catalog
presence and descriptors build, and the contracts in hand. Each key reads as
one of fourteen **identity rungs** — `resolved` to a resource of the
contract, or the rung where resolution stopped (`not in namespace`, `not
zk2`, `presence not read`, `no provider`, `no revision`, `contract not held`,
`contract unavailable`, `no resource`, `no member`, …) — held in a bounded
identity cache that resolves again whenever the lens moves. A resolved key's
payload is checked against its contract: **valid**, **invalid**,
**undecodable**, or **not checked** with the rung. The verdict cache's own
"not yet checked" is never a verdict.

## Usage

```bash
zengui -c tcp/127.0.0.1:7447                         # the bus-root deployment
zengui -c tcp/127.0.0.1:7447 --namespace spray       # a named deployment (--base is its alias)
zengui -c … --scope deployment                       # the namespace's ambient zk2/**
zengui -c … --scope streams                          # its ambient streams (`telemetry` still reads as this)
zengui -c … --scope explicit                         # its explicit @stream and @state, named
zengui -c … --selector 'demo/**'                     # your own key expressions
zengui -c … --contracts examples/zk2/.history        # contracts held offline (repeatable)
```

### A note on `**`

`*` and `**` never match a verbatim chunk — one beginning with `@` (core §1.3).
So the default raw scope cannot pull an explicit `@stream` (camera frames) or
an `@state` population into the echo pane — it is safe by key algebra rather
than by policy — but it equally cannot see `@op` or the `@zk` control keys.
`**` is therefore **not** "everything", and zengui never labels it as such:
the `explicit` scope names the verbatim kinds, and every selector says which
verbatim chunks it cannot see.

## What the window shows

**The Locator** is the key tree, with find-in-tree and virtualized rows. It
starts from the **contract skeleton**: every resource of every address
presence names, declared before a byte of it arrives, templates symbolic. A
declared `@op` row offers no watch, because a GET under `@op` is a call. Each
chunk carries its zk2 role (`zk2`, system, service, iface, kind, resource,
`@zk` control), and the tree **pivots** by system, by address, by interface
or by kind. A templated family under a watch wears a **budget badge** from
the core §2.7 judge: `within budget: n of b`, `exceeds budget: n of b`, or, mostly,
`budget unobservable (…)` with why — a window too short, lossy, the owner
absent, or state, which a window does not count.

**The Inspector** follows the selection, and every further subject is a pin:
a key, a subtree, or an address. It shows the key's identity (address,
interface, kind, resource, type, revision and the template's values), its
declared type, and **the value, read by what the key is**:

- a zk2 **state** key: S4's owner GET, `via owner GET (S4)`; no answer reads
  "silent", never "no value";
- **last-known**: behind one button per `archive.v1` provider presence shows,
  "last-known from <archive> — not current (S5, S6)";
- a **stream** with `history`: the owner's `@adv` cache; without history,
  nothing is read, and the pane says why;
- an **event**: replayed on a button press, bounded by its retention;
- a **foreign** or control key: a raw GET, labelled raw;
- under `@op`, on a symbolic path, or a zk2 key the lens stopped short of:
  nothing, and why.

Beside it: the declared QoS against the axes the newest sample rode, each
differing axis named with both sides; the history ring and its series;
**why** (zk2's ladder, on a key or an
address; `not_asked` is never "no"); **fields**, over a stated window and
bounds; and **consumers**, the R3 edges descriptors declare, never a matching
status. Stamps are attributed through the lens (O7).

**A metric** (`telemetry.v1`, `spec/profiles/telemetry/v1.md`) is read the
way the profile's reader reads one, through the engine's `TelemetryBook`
over the key's recorded history:

- the identity shows **METRIC KIND** and **UNIT** beside TYPE, each with
  where it came from: `counter · declared`, `dBm · from the Point`. A unit
  nothing declares and no Point carries reads `unknown (§2.5)`, never blank
  and never dimensionless. zenoh-modem's bare numbers carry theirs in their
  declarations alone, and read them;
- the Series section plots the profile's value: a Point's `value.value` or
  the bare value, a boolean as 1 or 0, and a histogram's mean,
  `sum / count`, the one number of a histogram in its member's unit (a gap
  while its count is 0). Text and bytes are not plotted. Each chart names
  its unit once, `counter [By]: 4096 now · …`;
- a counter gets a second chart, its **§2.11 rate**, in the counter's unit
  and `/s`, timed by the Points' timestamps, the owner's stamps or arrival,
  as the section says. A reset is a gap, with no rate across it and none
  restarted from zero;
- each **decrease within one epoch** is a finding, with how its epoch was
  established. zengui reads no presence for an epoch, because its presence
  and its samples arrive on two monitors, in no one order (§2.9), so an
  epoch here is in-band or assumed. Where every pair's was assumed, the
  section says that no decrease could have been found (§5);
- a sample that does not conform is not plotted, and the profile's reasons
  are listed in its words (`telemetry.v1 §2.6 tag_disagrees: …`). Echo
  names each drawn invalid row's first violation under its strips;
- the engine's EWMA stays beside them, named `message rate (samples/s)`;
- the Fields sparks carry the member's unit on its value path alone;
- a resource that is no metric says why its axis has no unit.

**The Activity dock** holds the session's parallel streams: echo (pause
without lying about the gap, filter by key expression and substring, drill
through to the Inspector, export zk2's NDJSON rows with each key's
`identity`), the publish log, doctor verdicts, replay transport, and traffic.

**The Workbench** is the tools:

- **Send.** *Publish* writes a **foreign** key only, as bytes — the text as
  typed, or hex — on the wire QoS axes you pick (priority, congestion,
  reliability, express), with the `Encoding` you name. A key a zk2 service
  owns is refused as you type it (P3), and so is a wildcard; a tombstone of
  any concrete key needs `--i-know`. A repeat stream carries the matching
  badge, and the send log is bounded and says what it dropped. *Call* is a
  contract call on `@op`: an interface whose revision declares the
  operation, an address from the providers presence lists, a request form
  built from the operation's type. A forbidden fan-out is refused before
  anything is sent (O2), and a value, an error envelope, a malformed reply
  and silence render apart (O5).
- **Nodes.** One card per address, from presence: up, down or unobservable,
  its instances, its descriptor's facts, a `hostid.v1` chip only when the
  descriptor's `profiles` lists it, and the watched-freshness join. Health
  (`health.v1`) is read once per selection, never polled, with a "clocks
  synced" toggle; without it an age reads "age unobservable".
- **Doctor.** zk2's nineteen checks, the same typed findings as `zenctl
  doctor --format json`, run on demand with run-over-run deltas: findings by
  severity, then every check's verdict (finding, clean, unobservable, not
  asked), then what was not judged and what was fixed. `deep` and "clocks
  synced" are toggles; "re-read presence and contracts" restarts the
  presence link and forgets the store's absences. An empty scope is no
  verdict.
- **Admin & storage.** Routers, storages and the topology, swept from the
  admin space on demand, and one S4 row per storage: `answers on owner state`
  is the finding, because a late joiner seeds from the owner and last-known
  state is an archive's (§4.4). "Not judged" is kept apart from clean.
- **Mesh.** The router graph, with attached instances beside their router.
- **Config.** One service's configuration through `config.v1`
  (`spec/profiles/config/v1.md`), for an address whose descriptor lists
  the profile — reached from its nodes card or its Inspector view
  ("configure"), or from the strip with the address selected. It reads
  what the owner serves: the view's groups by class (`hot`, `reach`,
  `contract`), each parameter's value, source, startup value where it
  differs, bounds and unit, whether the group is writable, the floor, the
  last change, the change events a reading heard and the desired report —
  and §5's answers in their three states, the engine's (`run_config`).
  A pending change shows its deadline as the owner's instant and a
  countdown read against this host's clock, labelled so; near the
  deadline the tool listens across it, so an undo is seen. A form per
  writable group is typed by the served schema and refused in the form by
  the owner's own validator before anything is sent; a reach change asks
  its consent, with the window it will ask — raised to the floor — shown
  first, and a dry run touches nothing. Confirm is offered only once the
  view shows the change applied; cancel, extend and persist beside it.
  Each answer renders by its union: applied or pending, view or status,
  the envelope with its `ConfigError`, or silence — a non-verdict. A
  sensitive value is typed into a masked field and never shown back, in
  the view, an answer or an event. Every act is refused during a replay.

**Replay** loads a `.zrec` (versions 1 to 3) or scrubs the retained window,
and resolves through an **offline lens**: no catalog, the offline contracts,
and the capture's namespace, because the live catalog describes another
world. The tree merges against the offline skeleton, verdicts are checked
offline, and a v1 or v2 row reads "QoS not recorded". While a replay runs,
publishing, calling, every Config act, recording, why, fields and
consumers are refused in every window. `record` writes `.zrec` version 3; the History section compares
against a loaded `.zsnap` version 2.

Beside them: a **connect pane** that selects and edits the named contexts
`zenctl` shares; **preferences** that persist theme, zoom, geometry, density,
scope and the named layouts, degrading loudly rather than silently when the
file will not parse; torn-off windows; and a **command palette** — Ctrl+P for
actions, Ctrl+K for observed keys, `?` for the shortcut map, which is
rendered from the same table that dispatches it.

**Removed in #614: the blob browser (#68) and the media viewer (#69).**
They spoke v1's planes — RFC 07 §2's `@blob` and RFC 07 §1's `@media` — and
zk2 has no profile for either yet: `blob.v1` and `media.v1` would come from
#613, and each surface comes back with its profile, not before. There are no
stub controls in the meantime, because a control that does nothing is a
promise the window cannot keep. v1's Config tool (#481), which spoke RFC 05
§5.1, went with them, and came back on `config.v1` (#768).

No codec logic lives here: every decode, check and encode is the engine's,
through the contract that declares the type.

## Try it

```bash
just gui-demo
```

That is the whole thing: it builds, starts a traffic generator, and opens the
GUI against it with `--namespace spray --contracts examples/zk2/.history`. No
router and nothing else to install — `spray` listens and zengui connects
straight to it. Ctrl-C stops both.

`spray` (`examples/spray.rs`) is a zk2 deployment in namespace `spray` —
nav, camera, perception, teleop, thrusters, a planner and a tc backend, real
services serving their contracts from `examples/zk2/`, a service
implementing `health.v1` alone, and `lab/meter`, whose metrics are
`telemetry.v1`'s (`zenkey/tests/contracts/meter.v1.toml`, `--meter`) —
beside the traffic no zk2 service would
send: an interface token with no instance token, a second answerer on an
owner's state, garbage on an owned key, an unowned key, a key in another
namespace, and foreign keys. Each of those reads what it is: the owners' keys
resolved and valid, the squatter's invalid, the garbage undecodable, the
unowned key at `no provider`, the other namespace's at `not in namespace`.
`just smoke-spray` reads the same deployment through zenctl's `service list`
and `doctor`.

What else to look at:

- The **Doctor** (the Diagnose layout, or the palette's "run doctor"): with
  `deep` on, it finds the spray's phantom token (`token-missing`) and its
  state squatter (`state-stamp-foreign`).
- **Nodes**: select `host-a/sysmon` for its `health.v1` reading. Its status
  cycles OK → DEGRADED → OK, so "read again" moves it.
- **Send → get / call**: `nav.v2`'s `set_origin` at `robot1/nav`.
- **A metric**: watch `lab/meter` and select one of its streams. `frames`
  is a Point counter in `1`, `bytes` a bare counter in `By` with its epoch
  in the attachment, `drops` a counter whose unit reads unknown, `latency`
  a histogram whose mean is plotted in `s`. Each owner's term ends after an
  hour as a new instance, which a counter reads as a reset.
- `just gui-demo-bounded` trips both bounds immediately, so the status strip
  reads `N keys (+M retired — bound reached)` **and**
  `identities: N cached (+M retired — cache bound reached)`. Two
  bounds over two different populations, so two sentences (#107).
- The **admin** tool. The spray's own session is a router with the admin
  space on, so the pane lists that router, and says why each empty section is
  empty. Point it at a router with storages and the rows must match `zenctl
  storage list`.
- `just gui-demo-no-contracts` holds nothing offline: the status strip counts
  every revision as retrieved from the spray's own holders (spec §8.4).

The spray exists because no conforming service can emit non-conforming
traffic — that is the point of one — so it is the only way to exercise
the key-agnostic path for real.

## Development

```bash
cargo test -p zengui        # unit, pane-rendering, scene and spray tests
just spray                  # the zk2 spray in one terminal…
just test-live              # …live-bus tests in another (or, with nothing running:
                            #  cargo test -p zengui --test live_bus -- --ignored)
just ci                     # everything CI runs
just shots                  # a PNG of every scene, both themes → target/shots/current
just shots-base main        # the same scenes drawn from main → target/shots/base
```

There is no display on the CI host, so the shots are how a visual change is
reviewed: `zengui/tests/shots.rs` renders the whole-window scenes in
`tests/common/scenes.rs` with the app's own renderer (wgpu on Vulkan —
lavapipe without a GPU; tiny-skia drops canvases and mesa's GL path draws only
the last one in a frame), and a PR that moves pixels
carries the before/after pair. The scenes' landmarks are checked by an
ordinary test, so a scene keeps meaning what its name says.

## Distribution

Never published to crates.io. Ships as a Forgejo release binary, or
`cargo install --git`, exactly as `zenctl` does.

## License

Apache-2.0.
