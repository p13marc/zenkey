# zengui

A graphical bus explorer for Zenoh — the GUI sibling of [`zenctl`](../zenctl),
over the same engine ([`zenkey-fleet`](../zenkey-fleet)).

**Key-agnostic by construction.** zengui is a useful explorer on *any* Zenoh bus.
It groups, counts and renders whatever keys it observes, with no assumption that
they follow any convention. The [keyspace-v2 convention](../rfcs) is an
*enrichment overlay*: when a key parses as `v1/<origin>/<class>/<producer>/<subject…>`
it gains origin/class/producer facets, a registry-resolved type name, and a
registration badge. When it does not, it is still a first-class node in the tree
with counts, rates and structural payload rendering — "does not parse" is an
answer, not an error.

zengui runs **two sessions** since #614 (FL6a), as zenctl and zenwatch do: the
data pump watches on a session in **no** namespace (RFC 09 §5), spelling full
wire keys — which is what lets it see traffic a namespaced application is blind
to — and presence, descriptors and contracts are read on a session **in** the
deployment's namespace. Every zk2 key is checked through the contract its
provider's descriptor names: valid, invalid, undecodable, or not checked with
the rung where resolution stopped. The empty namespace is legal and is the
default — an explorer that cannot see and name the bus-root deployment is blind
to the common case. (The port is in progress: the tree, the nodes pane and the
v1 panes still read v1 until the chunks after FL6a move them.)

## Usage

```bash
zengui -c tcp/127.0.0.1:7447                         # the bus-root deployment
zengui -c tcp/127.0.0.1:7447 --namespace spray       # a named deployment (--base is its alias)
zengui -c … --scope deployment                       # the namespace's ambient zk2/**
zengui -c … --scope explicit                         # its explicit @stream and @state, named
zengui -c … --selector 'demo/**'                     # your own key expressions
zengui -c … --contracts examples/zk2/.history        # contracts held offline (repeatable)
```

Namespaces in use are listed on connect (`zenkey_fleet::namespace_listing`)
and offered in a picker; `--namespace` is a shortcut, not a requirement.
`--registry` is v1's, and is refused by name in favour of `--contracts`.

### A note on `**`

`*` and `**` never match a verbatim chunk — one beginning with `@` (core §1.3).
So the default raw scope cannot pull an explicit `@stream` (camera frames) or
an `@state` population into the echo pane — it is safe by key algebra rather
than by policy — but it equally cannot see `@op` or the `@zk` control keys.
`**` is therefore **not** "everything", and zengui never labels it as such:
the `explicit` scope names the verbatim kinds, and every selector says which
verbatim chunks it cannot see.

## Status

The §6.4 checklist is delivered: key tree (with pivots, find-in-tree and
virtualized rendering), live echo, the call and publish panes (#60 — typed
targets, per-origin outcomes with the alive-but-silent origins named, request
forms scaffolded from the served schema; and a publish form that encodes for
the wire through the engine's ladder, with a QoS picker that *is* the closed
enum, a repeat/interval stream carrying the #38 matching badge, and a bounded
send log that reports what it dropped), the node dashboard (#61 — liveliness
roster, suspect-on-retraction, lazy `node_info` detail), and the
schema-decoded payload inspector with the selection detail pane. A doctor
panel (#71) renders the same typed findings as `zenctl doctor --format json`,
run on demand with run-over-run deltas. An **admin & storage panel** (#70)
sweeps the Zenoh admin space on demand and renders the coverage table RFC 09
§2's late-joiner concern calls for: declared state families against the
storages that would seed them, with "not judged" kept firmly distinct from
"uncovered".

Phase 2 of the epic is complete: a **connect pane** (#67) selects and edits the
named contexts `zenctl` shares, and spends three lines on what RFC 09 §0.1
actually says about scouting; **echo v2** (#72) pauses without lying about the
gap, filters by key expression *and* substring, drills through to the inspector
and exports the CLI's ndjson rows; **preferences** (#73) persist theme, zoom,
geometry and scope, degrading loudly rather than silently when the file will not
parse; and a **command palette** (#75) — Ctrl+P for actions, Ctrl+K for observed
keys, `?` for the shortcut map, which is rendered from the same table that
dispatches it.

**Removed in #614 (FL11): the Config tool (#481), the blob browser (#68) and
the media viewer (#69).** They spoke v1's planes — RFC 05 §5.1's configuration
calls, RFC 07 §2's `@blob` and RFC 07 §1's `@media` — and zk2 has no profile
for any of them yet: `config.v1`, `blob.v1` and `media.v1` would come from
#613, and each surface comes back with its profile, not before. There are no
stub controls in the meantime, because a control that does nothing is a
promise the window cannot keep.

No codec logic lives here. The publish form hands its body to
`zenkey_fleet_v1::prepare_publish` and renders what comes back — encoded, sent as
typed, or sent raw — because a payload that shipped unencoded must never look
like one that did not.

## Try it

```bash
just gui-demo
```

That is the whole thing: it builds, starts a traffic generator, and opens the
GUI against it. No router and nothing else to install — `spray` listens and
zengui connects straight to it. Ctrl-C stops both.

`spray` (`examples/spray.rs`, #614 FL5) is a zk2 deployment in namespace
`spray` — nav, camera, perception, teleop, thrusters, a planner and a tc
backend, real services serving their contracts from `examples/zk2/`, and a
service implementing `health.v1` alone — beside the traffic no zk2 service
would send: an interface token with no instance token, a second answerer on
an owner's state, garbage on an owned key, an unowned key, a key in another
namespace, and foreign keys. `just gui-demo` opens zengui with `--namespace
spray`, so since FL6a each of those keys is checked through its contract and
badged with what the check found — valid, invalid, undecodable, or not checked
with the rung where resolution stopped. The tree still reads v1 while #614
ports it pane by pane, so the zk2 keys are unlabelled rather than mislabelled
rows. `just smoke-spray` reads the same deployment through zenctl's `service
list` and `doctor`.

What else to look at:

- `just gui-demo-bounded` trips both bounds immediately, so the status strip
  reads `N keys (+M retired — bound reached)` **and**
  `identities: N cached (+M retired — cache bound reached)`. Two
  bounds over two different populations, so two sentences (#107).
- The **admin** tool (the Workbench dock, or the palette's "go to admin
  pane"). Against the demo's peer-only bus every section
  should say *why* it is empty — "a peer-only mesh, or the admin space is
  disabled" — and the coverage table should read "coverage not judged"
  without a registry, never "uncovered". Point it at a router with storages
  and the rows must match `zenctl storage list`.
- `just gui-demo-no-contracts` holds nothing offline: the status strip counts
  every revision as retrieved from the spray's own holders (spec §8.4).

The v1 demo traffic (`spray_v1`) was deleted in #614 (FL11), with the
Config tool, the blob browser and the media viewer it was kept for.

The spray exists because no conforming service can emit non-conforming
traffic — that is the point of one — so it is the only way to exercise
the key-agnostic path for real.

## Development

```bash
cargo test -p zengui        # unit, registry-overlay, pane-rendering and spray tests
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
